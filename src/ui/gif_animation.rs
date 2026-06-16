use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use windows_sys::Win32::Graphics::Gdi::HDC;
use windows_sys::Win32::Graphics::GdiPlus::{
    BitmapData, FrameDimensionTime, GdipBitmapLockBits, GdipBitmapUnlockBits, GdipCreateFromHDC,
    GdipDeleteGraphics, GdipDisposeImage, GdipDrawImageRectRectI, GdipGetImageHeight,
    GdipGetImageWidth, GdipGetPropertyItem, GdipGetPropertyItemSize, GdipImageGetFrameCount,
    GdipImageSelectActiveFrame, GdipLoadImageFromFile, GdipSetInterpolationMode, GdiplusShutdown,
    GdiplusStartup, GdiplusStartupInput, GpBitmap, GpGraphics, GpImage, ImageLockModeRead,
    InterpolationModeNearestNeighbor, PropertyItem, PropertyTagFrameDelay, Rect,
};

use crate::app::PetMood;
use crate::globals::{APP_STATE, PET_RENDERER};
use crate::settings::{UserSettings, load_user_settings, resolve_mood_gif};
use crate::util::wide;

const MOODS: &[PetMood] = &[
    PetMood::Idle,
    PetMood::Thinking,
    PetMood::Typing,
    PetMood::Building,
    PetMood::Search,
    PetMood::Happy,
    PetMood::Error,
    PetMood::Deny,
    PetMood::Shrug,
    PetMood::Sleeping,
    PetMood::Subagent,
    PetMood::Pomodoro,
    PetMood::Wave,
    PetMood::Stretch,
    PetMood::Fishing,
    PetMood::FishingReel,
    PetMood::FishingCaught,
    PetMood::FishingMissed,
];
const PIXEL_FORMAT_32BPP_ARGB: i32 = 0x0026_200A;

#[derive(Clone, Copy, Debug)]
pub(crate) struct GifVisibleBounds {
    pub(crate) source_width: u32,
    pub(crate) source_height: u32,
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

struct GifClip {
    image: *mut GpImage,
    path: PathBuf,
    delays_ms: Vec<u32>,
    cumulative_ms: Vec<u32>,
    total_ms: u32,
    width: u32,
    height: u32,
    visible_bounds: GifVisibleBounds,
    // Frame currently selected on the GDI+ image; selecting a frame decodes
    // it, so repeated draws of the same frame skip the call.
    active_frame: Option<usize>,
}

unsafe impl Send for GifClip {}

impl Drop for GifClip {
    fn drop(&mut self) {
        unsafe {
            if !self.image.is_null() {
                GdipDisposeImage(self.image);
            }
        }
    }
}

impl GifClip {
    unsafe fn load(path: &Path) -> Result<Self, String> {
        let mut image: *mut GpImage = std::ptr::null_mut();
        let path_wide = wide(&path.to_string_lossy());
        let status = GdipLoadImageFromFile(path_wide.as_ptr(), &mut image);
        if status != 0 || image.is_null() {
            return Err(format!(
                "GdipLoadImageFromFile({}) failed: status {status}",
                path.display()
            ));
        }

        let mut width = 0_u32;
        let mut height = 0_u32;
        GdipGetImageWidth(image, &mut width);
        GdipGetImageHeight(image, &mut height);

        let mut frame_count = 0_u32;
        let status = GdipImageGetFrameCount(image, &FrameDimensionTime, &mut frame_count);
        if status != 0 || frame_count == 0 {
            frame_count = 1;
        }

        let visible_bounds =
            scan_visible_bounds(image, frame_count, width, height).unwrap_or(GifVisibleBounds {
                source_width: width,
                source_height: height,
                x: 0,
                y: 0,
                width: width.max(1),
                height: height.max(1),
            });
        let delays_ms = load_frame_delays(image, frame_count as usize);
        let mut cumulative_ms = Vec::with_capacity(delays_ms.len());
        let mut acc: u32 = 0;
        for delay in &delays_ms {
            acc = acc.saturating_add(*delay);
            cumulative_ms.push(acc);
        }
        let total_ms = acc.max(1);

        Ok(Self {
            image,
            path: path.to_path_buf(),
            delays_ms,
            cumulative_ms,
            total_ms,
            width,
            height,
            visible_bounds,
            active_frame: None,
        })
    }

    /// Dispose and reopen the GDI+ image so its internal decoded-frame cache
    /// is released; frames re-decode lazily when the clip is drawn again.
    unsafe fn release_frame_cache(&mut self) {
        if !self.image.is_null() {
            GdipDisposeImage(self.image);
            self.image = std::ptr::null_mut();
        }
        self.active_frame = None;
        let mut image: *mut GpImage = std::ptr::null_mut();
        let path_wide = wide(&self.path.to_string_lossy());
        if GdipLoadImageFromFile(path_wide.as_ptr(), &mut image) == 0 && !image.is_null() {
            self.image = image;
        }
    }

    fn frame_at(&self, elapsed_ms: u32) -> usize {
        if self.delays_ms.len() <= 1 {
            return 0;
        }
        let modded = elapsed_ms % self.total_ms;
        for (idx, cum) in self.cumulative_ms.iter().enumerate() {
            if modded < *cum {
                return idx;
            }
        }
        self.delays_ms.len() - 1
    }
}

unsafe fn scan_visible_bounds(
    image: *mut GpImage,
    frame_count: u32,
    width: u32,
    height: u32,
) -> Option<GifVisibleBounds> {
    if image.is_null() || width == 0 || height == 0 {
        return None;
    }

    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0_u32;
    let mut max_y = 0_u32;
    let mut found = false;

    for frame_idx in 0..frame_count.max(1) {
        if GdipImageSelectActiveFrame(image, &FrameDimensionTime, frame_idx) != 0 {
            return None;
        }
        let frame_bounds = lock_and_scan_frame(image as *mut GpBitmap, width, height)?;
        let Some(frame_bounds) = frame_bounds else {
            continue;
        };
        min_x = min_x.min(frame_bounds.x);
        min_y = min_y.min(frame_bounds.y);
        max_x = max_x.max(frame_bounds.x + frame_bounds.width - 1);
        max_y = max_y.max(frame_bounds.y + frame_bounds.height - 1);
        found = true;
    }

    found.then_some(GifVisibleBounds {
        source_width: width,
        source_height: height,
        x: min_x,
        y: min_y,
        width: max_x - min_x + 1,
        height: max_y - min_y + 1,
    })
}

unsafe fn lock_and_scan_frame(
    bitmap: *mut GpBitmap,
    width: u32,
    height: u32,
) -> Option<Option<GifVisibleBounds>> {
    let rect = Rect {
        X: 0,
        Y: 0,
        Width: width as i32,
        Height: height as i32,
    };
    let mut data = BitmapData::default();
    if GdipBitmapLockBits(
        bitmap,
        &rect,
        ImageLockModeRead as u32,
        PIXEL_FORMAT_32BPP_ARGB,
        &mut data,
    ) != 0
        || data.Scan0.is_null()
    {
        return None;
    }

    let bounds = scan_locked_argb(&data, width, height);
    GdipBitmapUnlockBits(bitmap, &mut data);
    Some(bounds.map(|(x, y, w, h)| GifVisibleBounds {
        source_width: width,
        source_height: height,
        x,
        y,
        width: w,
        height: h,
    }))
}

unsafe fn scan_locked_argb(
    data: &BitmapData,
    width: u32,
    height: u32,
) -> Option<(u32, u32, u32, u32)> {
    let scan0 = data.Scan0 as *const u8;
    let stride = data.Stride;
    if scan0.is_null() || stride == 0 {
        return None;
    }

    let row_stride = stride.unsigned_abs() as usize;
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0_u32;
    let mut max_y = 0_u32;
    let mut found = false;

    for y in 0..height {
        let source_y = if stride > 0 { y } else { height - 1 - y };
        let row = scan0.add(source_y as usize * row_stride);
        for x in 0..width {
            let alpha = *row.add(x as usize * 4 + 3);
            if alpha == 0 {
                continue;
            }
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
            found = true;
        }
    }

    found.then_some((min_x, min_y, max_x - min_x + 1, max_y - min_y + 1))
}

unsafe fn load_frame_delays(image: *mut GpImage, frame_count: usize) -> Vec<u32> {
    if frame_count <= 1 {
        return vec![100];
    }
    let mut size = 0_u32;
    let status = GdipGetPropertyItemSize(image, PropertyTagFrameDelay, &mut size);
    if status != 0 || size == 0 {
        return vec![100; frame_count];
    }
    let mut buffer = vec![0u8; size as usize];
    let header = buffer.as_mut_ptr() as *mut PropertyItem;
    let status = GdipGetPropertyItem(image, PropertyTagFrameDelay, size, header);
    if status != 0 {
        return vec![100; frame_count];
    }
    let item = *header;
    let payload_len = item.length as usize;
    let header_size = std::mem::size_of::<PropertyItem>();
    let payload_start = header_size;
    let payload_end = payload_start + payload_len;
    if payload_end > buffer.len() {
        return vec![100; frame_count];
    }
    let bytes = &buffer[payload_start..payload_end];
    let mut delays = Vec::with_capacity(frame_count);
    for chunk in bytes.chunks_exact(4).take(frame_count) {
        let centiseconds = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        let ms = if centiseconds == 0 {
            100
        } else {
            centiseconds.saturating_mul(10).max(20)
        };
        delays.push(ms);
    }
    while delays.len() < frame_count {
        delays.push(100);
    }
    delays
}

pub(crate) struct GifAnimation {
    clips: HashMap<PetMood, GifClip>,
    current: PetMood,
    clip_started_at: Instant,
}

unsafe impl Send for GifAnimation {}

impl GifAnimation {
    unsafe fn load(settings: &UserSettings) -> Result<(Self, String), String> {
        let mut clips: HashMap<PetMood, GifClip> = HashMap::new();
        for mood in MOODS {
            // Each mood resolves independently: the user's folder if it has the
            // file, otherwise the bundled default. A mood with neither is fatal
            // (the bundled assets are always expected to be present).
            let path = resolve_mood_gif(settings, *mood)
                .ok_or_else(|| format!("missing GIF for {}", mood.key()))?;
            let clip = GifClip::load(&path)?;
            clips.insert(*mood, clip);
        }
        let summary = format!("loaded {} GIFs", clips.len());
        Ok((
            Self {
                clips,
                current: PetMood::Idle,
                clip_started_at: Instant::now(),
            },
            summary,
        ))
    }

    pub(crate) fn request_mood(&mut self, mood: PetMood) -> bool {
        if mood == self.current {
            return true;
        }
        // Release the outgoing clip's decoded frames; mood switches restart
        // the incoming clip at frame 0 anyway, so nothing visible is lost.
        if let Some(clip) = self.clips.get_mut(&self.current) {
            unsafe { clip.release_frame_cache() };
        }
        self.current = mood;
        self.clip_started_at = Instant::now();
        true
    }

    fn elapsed_ms(&self) -> u32 {
        Instant::now()
            .duration_since(self.clip_started_at)
            .as_millis()
            .min(u32::MAX as u128) as u32
    }

    /// Current (mood, frame index) without drawing; switches the active clip
    /// like `draw` so the answer matches what the next paint will show.
    pub(crate) fn frame_signature(&mut self, mood: PetMood) -> (PetMood, usize) {
        if mood != self.current {
            self.request_mood(mood);
        }
        let frame = self
            .clips
            .get(&self.current)
            .map(|clip| clip.frame_at(self.elapsed_ms()))
            .unwrap_or(0);
        (self.current, frame)
    }

    pub(crate) unsafe fn draw(
        &mut self,
        hdc: HDC,
        mood: PetMood,
        x: i32,
        y: i32,
        max_w: i32,
        max_h: i32,
    ) -> bool {
        if mood != self.current {
            self.request_mood(mood);
        }
        let mood = self.current;
        let elapsed_ms = self.elapsed_ms();
        let Some(clip) = self.clips.get_mut(&mood) else {
            return false;
        };

        let frame_idx = clip.frame_at(elapsed_ms);
        if clip.active_frame != Some(frame_idx) {
            let select_status =
                GdipImageSelectActiveFrame(clip.image, &FrameDimensionTime, frame_idx as u32);
            if select_status != 0 {
                return false;
            }
            clip.active_frame = Some(frame_idx);
        }

        let (draw_w, draw_h) = fit_into(clip.width, clip.height, max_w, max_h);
        let draw_x = x + (max_w - draw_w) / 2;
        let draw_y = y + (max_h - draw_h);

        let mut graphics: *mut GpGraphics = std::ptr::null_mut();
        if GdipCreateFromHDC(hdc, &mut graphics) != 0 || graphics.is_null() {
            return false;
        }
        GdipSetInterpolationMode(graphics, InterpolationModeNearestNeighbor);
        let status = GdipDrawImageRectRectI(
            graphics,
            clip.image,
            draw_x,
            draw_y,
            draw_w,
            draw_h,
            0,
            0,
            clip.width as i32,
            clip.height as i32,
            2, // UnitPixel
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
        );
        GdipDeleteGraphics(graphics);
        status == 0
    }

    fn visible_bounds(&self, mood: PetMood) -> Option<GifVisibleBounds> {
        self.clips.get(&mood).map(|clip| clip.visible_bounds)
    }

    fn clip_total_ms(&self, mood: PetMood) -> Option<u32> {
        self.clips.get(&mood).map(|clip| clip.total_ms)
    }
}

fn fit_into(src_w: u32, src_h: u32, max_w: i32, max_h: i32) -> (i32, i32) {
    if src_w == 0 || src_h == 0 || max_w <= 0 || max_h <= 0 {
        return (max_w.max(1), max_h.max(1));
    }
    let scale_w = max_w as f32 / src_w as f32;
    let scale_h = max_h as f32 / src_h as f32;
    let scale = scale_w.min(scale_h).max(0.01);
    let w = ((src_w as f32) * scale).round() as i32;
    let h = ((src_h as f32) * scale).round() as i32;
    (w.max(1), h.max(1))
}

pub(crate) struct AnimationStore {
    token: usize,
    animation: GifAnimation,
    source_summary: String,
    // Bumped on reload so frame signatures from the old clip set never
    // compare equal to signatures from the new one.
    generation: u64,
}

unsafe impl Send for AnimationStore {}

impl Drop for AnimationStore {
    fn drop(&mut self) {
        unsafe {
            if self.token != 0 {
                GdiplusShutdown(self.token);
            }
        }
    }
}

impl AnimationStore {
    unsafe fn load() -> Result<Self, String> {
        let mut token = 0_usize;
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            DebugEventCallback: 0,
            SuppressBackgroundThread: 0,
            SuppressExternalCodecs: 0,
        };
        let status = GdiplusStartup(&mut token, &input, std::ptr::null_mut());
        if status != 0 {
            return Err(format!("GDI+ startup failed: status {status}"));
        }
        let settings = load_user_settings();
        let (animation, source_summary) = GifAnimation::load(&settings)?;
        Ok(Self {
            token,
            animation,
            source_summary,
            generation: 0,
        })
    }

    unsafe fn reload(&mut self) -> Result<String, String> {
        let settings = load_user_settings();
        let (animation, source_summary) = GifAnimation::load(&settings)?;
        self.animation = animation;
        self.source_summary = source_summary.clone();
        self.generation += 1;
        Ok(source_summary)
    }

    pub(crate) fn request_mood(&mut self, mood: PetMood) -> bool {
        self.animation.request_mood(mood)
    }

    pub(crate) fn frame_signature(&mut self, mood: PetMood) -> (PetMood, usize, u64) {
        let (mood, frame) = self.animation.frame_signature(mood);
        (mood, frame, self.generation)
    }

    pub(crate) unsafe fn draw(
        &mut self,
        hdc: HDC,
        mood: PetMood,
        x: i32,
        y: i32,
        max_w: i32,
        max_h: i32,
    ) -> bool {
        self.animation.draw(hdc, mood, x, y, max_w, max_h)
    }

    pub(crate) fn visible_bounds(&self, mood: PetMood) -> Option<GifVisibleBounds> {
        self.animation.visible_bounds(mood)
    }

    pub(crate) fn clip_total_ms(&self, mood: PetMood) -> Option<u32> {
        self.animation.clip_total_ms(mood)
    }
}

pub(crate) fn init_animation_store() {
    let store = unsafe { AnimationStore::load() };
    match store {
        Ok(store) => {
            let _ = PET_RENDERER.set(Mutex::new(store));
        }
        Err(err) => {
            if let Some(state) = APP_STATE.get() {
                let mut state = state.lock().expect("state poisoned");
                state.last_error = err;
            }
        }
    }
}

pub(crate) fn reload_animation_store() -> Result<String, String> {
    let Some(store) = PET_RENDERER.get() else {
        return Err("pet renderer is not initialized".to_string());
    };
    unsafe { store.lock().expect("pet renderer poisoned").reload() }
}

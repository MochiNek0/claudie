use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use windows_sys::Win32::Graphics::Gdi::HDC;
use windows_sys::Win32::Graphics::GdiPlus::{
    FrameDimensionTime, GdipCreateFromHDC, GdipDeleteGraphics, GdipDisposeImage,
    GdipDrawImageRectRectI, GdipGetImageHeight, GdipGetImageWidth, GdipGetPropertyItem,
    GdipGetPropertyItemSize, GdipImageGetFrameCount, GdipImageSelectActiveFrame,
    GdipLoadImageFromFile, GdipSetInterpolationMode, GdiplusShutdown, GdiplusStartup,
    GdiplusStartupInput, GpGraphics, GpImage, InterpolationModeNearestNeighbor, PropertyItem,
    PropertyTagFrameDelay,
};

use crate::app::PetMood;
use crate::globals::{APP_STATE, PET_RENDERER};
use crate::settings::{UserSettings, configured_gif_dir, load_user_settings};
use crate::util::wide;

const MOODS: &[PetMood] = &[
    PetMood::Idle,
    PetMood::Thinking,
    PetMood::Typing,
    PetMood::Building,
    PetMood::Search,
    PetMood::Happy,
    PetMood::Error,
    PetMood::Sleeping,
    PetMood::Subagent,
    PetMood::Pomodoro,
    PetMood::Wave,
    PetMood::Stretch,
];

struct GifClip {
    image: *mut GpImage,
    delays_ms: Vec<u32>,
    cumulative_ms: Vec<u32>,
    total_ms: u32,
    width: u32,
    height: u32,
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
            delays_ms,
            cumulative_ms,
            total_ms,
            width,
            height,
        })
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
        let dir =
            configured_gif_dir(settings).ok_or_else(|| "assets/claudie not found".to_string())?;
        let mut clips: HashMap<PetMood, GifClip> = HashMap::new();
        let mut loaded_paths = Vec::new();
        for mood in MOODS {
            let name = settings.animation_value(*mood);
            let path = resolve_gif_path(&dir, name);
            if !path.exists() {
                return Err(format!(
                    "missing GIF for {}: {}",
                    mood.key(),
                    path.display()
                ));
            }
            let clip = GifClip::load(&path)?;
            loaded_paths.push(format!("{} -> {}", mood.key(), path.display()));
            clips.insert(*mood, clip);
        }
        let summary = format!("loaded {} GIFs from {}", clips.len(), dir.display());
        let _ = loaded_paths;
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
        self.current = mood;
        self.clip_started_at = Instant::now();
        true
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
        let Some(clip) = self.clips.get(&mood) else {
            return false;
        };

        let elapsed_ms = Instant::now()
            .duration_since(self.clip_started_at)
            .as_millis()
            .min(u32::MAX as u128) as u32;
        let frame_idx = clip.frame_at(elapsed_ms);
        let select_status =
            GdipImageSelectActiveFrame(clip.image, &FrameDimensionTime, frame_idx as u32);
        if select_status != 0 {
            return false;
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

fn resolve_gif_path(dir: &Path, name: &str) -> PathBuf {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return dir.join("idle.gif");
    }
    if trimmed.ends_with(".gif") || trimmed.ends_with(".GIF") {
        return dir.join(trimmed);
    }
    dir.join(format!("{trimmed}.gif"))
}

pub(crate) struct AnimationStore {
    token: usize,
    animation: GifAnimation,
    source_summary: String,
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
        })
    }

    unsafe fn reload(&mut self) -> Result<String, String> {
        let settings = load_user_settings();
        let (animation, source_summary) = GifAnimation::load(&settings)?;
        self.animation = animation;
        self.source_summary = source_summary.clone();
        Ok(source_summary)
    }

    pub(crate) fn request_mood(&mut self, mood: PetMood) -> bool {
        self.animation.request_mood(mood)
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

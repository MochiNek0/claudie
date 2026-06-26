pub(crate) const DEFAULT_PORT: u16 = 17387;
pub(crate) const DEFAULT_PROXY_PORT: u16 = 17388;
pub(crate) const PET_SCALE_MIN_PERCENT: u32 = 50;
pub(crate) const PET_SCALE_MAX_PERCENT: u32 = 150;
pub(crate) const POMODORO_MIN_MINUTES: u32 = 1;
pub(crate) const POMODORO_MAX_MINUTES: u32 = 240;
pub(crate) const TRANSPARENT_KEY: u32 = 0x00ff_00ff;
pub(crate) const PET_X: i32 = 0;
pub(crate) const PET_Y: i32 = 0;
pub(crate) const PET_W: i32 = 188;
pub(crate) const PET_H: i32 = 170;
pub(crate) const SESSION_BAR_HEIGHT: i32 = 24;
pub(crate) const SESSION_BAR_GAP: i32 = 2;
pub(crate) const SESSION_SWITCHER_MIN_WIDTH: i32 = 236;
pub(crate) const SESSION_SWITCHER_VERTICAL_PADDING: i32 = 3;
pub(crate) const SESSION_SWITCHER_MAX_VISIBLE_ITEMS: usize = 4;
pub(crate) const POMODORO_HUD_WIDTH: i32 = 82;
pub(crate) const POMODORO_HUD_HEIGHT: i32 = 28;
pub(crate) const FISHING_HUD_WIDTH: i32 = 184;
pub(crate) const FISHING_HUD_HEIGHT: i32 = 58;
pub(crate) const MENU_SETTINGS_ID: usize = 1000;
pub(crate) const MENU_EXIT_ID: usize = 1001;
pub(crate) const MENU_POMODORO_START_ID: usize = 1002;
pub(crate) const MENU_POMODORO_STOP_ID: usize = 1003;
pub(crate) const MENU_FISHING_START_ID: usize = 1004;
pub(crate) const MENU_FISHING_STOP_ID: usize = 1005;
pub(crate) const MENU_POMODORO_PAUSE_RESUME_ID: usize = 1006;
pub(crate) const MENU_POMODORO_SKIP_ID: usize = 1007;
pub(crate) const MENU_CHECK_UPDATE_ID: usize = 1008;
pub(crate) const MENU_LLM_PROFILE_BASE_ID: usize = 1100;
pub(crate) const MENU_LLM_PROFILE_MAX_ITEMS: usize = 24;
/// Base id for the "copy launch command" submenu, one entry per profile,
/// mirroring the profile submenu's capacity. Kept clear of the profile range
/// (1100..1124).
pub(crate) const MENU_LLM_COPY_CMD_BASE_ID: usize = 1130;

pub(crate) fn scaled_pet_size_for_percent(scale_percent: u32) -> (i32, i32) {
    let scale = scale_percent.clamp(PET_SCALE_MIN_PERCENT, PET_SCALE_MAX_PERCENT) as i32;
    (
        ((PET_W * scale + 50) / 100).max(1),
        ((PET_H * scale + 50) / 100).max(1),
    )
}

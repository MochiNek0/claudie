use std::time::Duration;

pub(crate) const DEFAULT_PORT: u16 = 17387;
pub(crate) const DEFAULT_PROXY_PORT: u16 = 17388;
pub(crate) const WINDOW_WIDTH: i32 = 460;
pub(crate) const WINDOW_HEIGHT: i32 = 280;
pub(crate) const PET_SCALE_MIN_PERCENT: u32 = 50;
pub(crate) const PET_SCALE_MAX_PERCENT: u32 = 150;
pub(crate) const POMODORO_MIN_MINUTES: u32 = 1;
pub(crate) const POMODORO_MAX_MINUTES: u32 = 240;
pub(crate) const PERMISSION_WAIT: Duration = Duration::from_secs(590);
pub(crate) const TRANSPARENT_KEY: u32 = 0x00ff_00ff;
pub(crate) const PET_X: i32 = 14;
pub(crate) const PET_Y: i32 = 95;
pub(crate) const PET_W: i32 = 188;
pub(crate) const PET_H: i32 = 170;
pub(crate) const PERMISSION_OVERLAY_WIDTH: i32 = 640;
pub(crate) const PERMISSION_OVERLAY_HEIGHT: i32 = 740;
pub(crate) const PERMISSION_BUBBLE_X: i32 = 129;
pub(crate) const PERMISSION_BUBBLE_Y: i32 = 228;
pub(crate) const PERMISSION_BUBBLE_W: i32 = 382;
pub(crate) const PERMISSION_BUBBLE_H: i32 = 256;
pub(crate) const PERMISSION_DETAIL_PANEL_H: i32 = 120;
pub(crate) const PERM_BUTTON_Y: i32 = PERMISSION_BUBBLE_Y + PERMISSION_BUBBLE_H - 44;
pub(crate) const PERM_BUTTON_ROW_X: i32 = PERMISSION_BUBBLE_X + (PERMISSION_BUBBLE_W - 262) / 2;
pub(crate) const ALLOW_BUTTON: (i32, i32, i32, i32) = (PERM_BUTTON_ROW_X, PERM_BUTTON_Y, 74, 32);
pub(crate) const ALWAYS_BUTTON: (i32, i32, i32, i32) =
    (PERM_BUTTON_ROW_X + 84, PERM_BUTTON_Y, 94, 32);
pub(crate) const DENY_BUTTON: (i32, i32, i32, i32) =
    (PERM_BUTTON_ROW_X + 188, PERM_BUTTON_Y, 74, 32);
pub(crate) const CHOICE_CARD_X: i32 = 14;
pub(crate) const CHOICE_CARD_Y: i32 = 14;
pub(crate) const CHOICE_CARD_W: i32 = 612;
pub(crate) const CHOICE_CARD_H: i32 = 706;
pub(crate) const CHOICE_OPTION_X: i32 = CHOICE_CARD_X + 22;
pub(crate) const CHOICE_OPTION_W: i32 = CHOICE_CARD_W - 44;
pub(crate) const CHOICE_OPTION_H: i32 = 28;
pub(crate) const CHOICE_SUBMIT_BUTTON: (i32, i32, i32, i32) = (
    CHOICE_CARD_X + CHOICE_CARD_W - 194,
    CHOICE_CARD_Y + CHOICE_CARD_H - 44,
    84,
    32,
);
pub(crate) const CHOICE_DENY_BUTTON: (i32, i32, i32, i32) = (
    CHOICE_CARD_X + CHOICE_CARD_W - 102,
    CHOICE_CARD_Y + CHOICE_CARD_H - 44,
    80,
    32,
);
pub(crate) const MENU_SETTINGS_ID: usize = 1000;
pub(crate) const MENU_EXIT_ID: usize = 1001;
pub(crate) const MENU_POMODORO_START_ID: usize = 1002;
pub(crate) const MENU_POMODORO_STOP_ID: usize = 1003;
pub(crate) const MENU_FISHING_START_ID: usize = 1004;
pub(crate) const MENU_FISHING_STOP_ID: usize = 1005;
pub(crate) const MENU_POMODORO_PAUSE_RESUME_ID: usize = 1006;
pub(crate) const MENU_POMODORO_SKIP_ID: usize = 1007;
pub(crate) const MENU_LLM_PROFILE_BASE_ID: usize = 1100;
pub(crate) const MENU_LLM_PROFILE_MAX_ITEMS: usize = 24;

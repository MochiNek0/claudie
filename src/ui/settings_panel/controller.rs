mod basic;
mod pomodoro;
mod profiles;

use std::time::Duration;

use slint::{SharedString, Timer, TimerMode};

use crate::app::AppState;
use crate::globals::APP_STATE;
use crate::settings::{
    LlmProfile, LlmProfileDb, UserSettings, load_llm_profile_db, load_user_settings,
};
use crate::ui::slint_views::SettingsWindow;

pub(super) struct SettingsController {
    weak: slint::Weak<SettingsWindow>,
    settings: UserSettings,
    llm_db: LlmProfileDb,
    profile_index: usize,
    _timer: Timer,
}

impl SettingsController {
    pub(super) fn new(weak: slint::Weak<SettingsWindow>) -> Self {
        let mut llm_db = load_llm_profile_db();
        llm_db.normalize();
        let profile_index = llm_db
            .profiles
            .iter()
            .position(|profile| profile.id == llm_db.active_profile_id)
            .unwrap_or(0);
        let timer = Timer::default();
        Self {
            weak,
            settings: load_user_settings(),
            llm_db,
            profile_index,
            _timer: timer,
        }
    }

    pub(super) fn load_into_ui(&mut self) {
        self.refresh_basic_fields();
        self.refresh_pomodoro_fields();
        self.refresh_profile_fields();
        self.refresh_pomodoro_tab();

        let weak = self.weak.clone();
        self._timer
            .start(TimerMode::Repeated, Duration::from_secs(1), move || {
                if let Some(ui) = weak.upgrade()
                    && ui.get_active_tab() == 1
                {
                    pomodoro::set_pomodoro_status(&ui);
                }
            });
    }

    fn ui(&self) -> Option<SettingsWindow> {
        self.weak.upgrade()
    }

    fn status(&self, message: &str) {
        if let Some(ui) = self.ui() {
            ui.set_status_message(shared(message));
        }
    }
}

pub(super) fn mutate_app_state(action: impl FnOnce(&mut AppState)) {
    if let Some(state) = APP_STATE.get() {
        action(&mut state.lock().expect("state poisoned"));
    }
}

pub(super) fn sync_app_settings(settings: &UserSettings, event: &str) {
    if let Some(state) = APP_STATE.get() {
        let mut state = state.lock().expect("state poisoned");
        state.settings = settings.clone();
        state.push_event("settings", event);
    }
}

pub(super) fn sync_app_llm_profiles(llm_db: &LlmProfileDb, event: &str) {
    if let Some(state) = APP_STATE.get() {
        let mut state = state.lock().expect("state poisoned");
        state.llm_profiles = llm_db.clone();
        state.push_event("settings", event);
    }
}

pub(super) fn profile_label_for_message(profile: &LlmProfile) -> String {
    let label = profile.display_label();
    if label.trim().is_empty() {
        profile.id.clone()
    } else {
        label
    }
}

pub(super) fn clamp_i32(value: i32, min: u32, max: u32) -> u32 {
    value.clamp(min as i32, max as i32) as u32
}

pub(super) fn snap_sleep_after(value: f32) -> u32 {
    let raw = clamp_i32(value.round() as i32, 15, 1800);
    let snapped = ((raw + 7) / 15) * 15;
    snapped.clamp(15, 1800)
}

pub(super) fn shared(value: &str) -> SharedString {
    SharedString::from(value)
}

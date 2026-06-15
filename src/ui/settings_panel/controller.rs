mod basic;
mod pomodoro;
mod profiles;
mod stats;

use std::time::Duration;

use slint::{SharedString, Timer, TimerMode};

use crate::app::{AppState, QuotaStats};
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
        self.refresh_stats_tab();

        let weak = self.weak.clone();
        self._timer
            .start(TimerMode::Repeated, Duration::from_secs(1), move || {
                if let Some(ui) = weak.upgrade()
                    && (ui.get_active_tab() == 1
                        || ui.get_active_tab() == 2
                        || ui.get_active_tab() == 3)
                {
                    if ui.get_active_tab() == 1 {
                        pomodoro::set_pomodoro_status(&ui);
                    } else if ui.get_active_tab() == 2 {
                        controller_refresh_profile_usage(&ui);
                    } else {
                        stats::set_stats_status(&ui);
                    }
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

pub(super) fn sync_app_settings(settings: &UserSettings) {
    if let Some(state) = APP_STATE.get() {
        let mut state = state.lock().expect("state poisoned");
        state.settings = settings.clone();
    }
}

pub(super) fn sync_app_llm_profiles(llm_db: &LlmProfileDb) {
    if let Some(state) = APP_STATE.get() {
        let mut state = state.lock().expect("state poisoned");
        state.llm_profiles = llm_db.clone();
    }
}

/// Only the official profile has live usage; other providers show defaults.
pub(super) fn usage_quota_for_profile(profile: &LlmProfile, quota: QuotaStats) -> QuotaStats {
    if profile.is_official() {
        quota
    } else {
        QuotaStats::default()
    }
}

pub(super) fn current_profile_usage_quota(profile: &LlmProfile) -> QuotaStats {
    let Some(state) = APP_STATE.get() else {
        return QuotaStats::default();
    };
    let quota = state.lock().expect("state poisoned").quota.clone();
    usage_quota_for_profile(profile, quota)
}

fn controller_refresh_profile_usage(ui: &SettingsWindow) {
    // The full profile form refresh lives in profiles.rs; this timer path keeps
    // the selected provider's live usage fresh without disturbing in-progress edits.
    let index = ui.get_selected_profile_index();
    if index < 0 {
        return;
    }
    let Some(state) = APP_STATE.get() else {
        return;
    };
    let state = state.lock().expect("state poisoned");
    let mut db = state.llm_profiles.clone();
    let quota = state.quota.clone();
    drop(state);
    db.normalize();
    let Some(profile) = db.profiles.get(index as usize) else {
        return;
    };
    let display_quota = usage_quota_for_profile(profile, quota);
    profiles::set_profile_usage_fields(ui, profile, &db.active_profile_id, &display_quota);
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

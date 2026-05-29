use crate::app::pomodoro::{PomodoroMode, PomodoroStatus, format_remaining};
use crate::config::{POMODORO_MAX_MINUTES, POMODORO_MIN_MINUTES};
use crate::globals::APP_STATE;
use crate::settings::save_user_settings;
use crate::ui::slint_views::SettingsWindow;

use super::{SettingsController, clamp_i32, shared, sync_app_settings};

impl SettingsController {
    pub(super) fn refresh_pomodoro_fields(&self) {
        let Some(ui) = self.ui() else {
            return;
        };
        ui.set_focus_minutes(self.settings.pomodoro.focus_minutes() as i32);
        ui.set_short_break_minutes(self.settings.pomodoro.short_break_minutes() as i32);
        ui.set_long_break_minutes(self.settings.pomodoro.long_break_minutes() as i32);
    }

    fn collect_pomodoro_fields(&mut self) {
        let Some(ui) = self.ui() else {
            return;
        };
        self.settings.pomodoro.focus_minutes = clamp_i32(
            ui.get_focus_minutes(),
            POMODORO_MIN_MINUTES,
            POMODORO_MAX_MINUTES,
        );
        self.settings.pomodoro.short_break_minutes = clamp_i32(
            ui.get_short_break_minutes(),
            POMODORO_MIN_MINUTES,
            POMODORO_MAX_MINUTES,
        );
        self.settings.pomodoro.long_break_minutes = clamp_i32(
            ui.get_long_break_minutes(),
            POMODORO_MIN_MINUTES,
            POMODORO_MAX_MINUTES,
        );
    }

    pub(in crate::ui::settings_panel) fn save_pomodoro_settings(&mut self) {
        self.collect_pomodoro_fields();
        if let Err(err) = save_user_settings(&self.settings) {
            self.status(&format!("Failed to save pomodoro settings: {err}"));
            return;
        }
        sync_app_settings(&self.settings);
        self.refresh_pomodoro_tab();
        self.status("Saved pomodoro settings.");
    }

    pub(in crate::ui::settings_panel) fn refresh_pomodoro_tab(&self) {
        if let Some(ui) = self.ui() {
            set_pomodoro_status(&ui);
        }
    }
}

pub(super) fn set_pomodoro_status(ui: &SettingsWindow) {
    let Some(state) = APP_STATE.get() else {
        ui.set_pomodoro_status(shared("Pomodoro data is not ready."));
        return;
    };
    let state = state.lock().expect("state poisoned");
    let mode = match state.pomodoro.mode {
        PomodoroMode::Focus => "Focus",
        PomodoroMode::ShortBreak => "Short break",
        PomodoroMode::LongBreak => "Long break",
    };
    let status = match state.pomodoro.status {
        PomodoroStatus::Stopped => "Stopped",
        PomodoroStatus::Running => "Running",
        PomodoroStatus::Paused => "Paused",
    };
    ui.set_pomodoro_status(shared(&format!(
        "{}    {}    {}\nCompleted focus sessions: {}",
        mode,
        status,
        format_remaining(state.pomodoro.remaining(&state.settings.pomodoro)),
        state.pomodoro.completed_focus_count
    )));
    ui.set_pause_resume_label(shared(if state.pomodoro.status == PomodoroStatus::Paused {
        "Resume"
    } else {
        "Pause"
    }));
}

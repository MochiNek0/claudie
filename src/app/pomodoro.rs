use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

use crate::config::{POMODORO_MAX_MINUTES, POMODORO_MIN_MINUTES};

const DEFAULT_FOCUS_MINUTES: u32 = 25;
const DEFAULT_SHORT_BREAK_MINUTES: u32 = 5;
const DEFAULT_LONG_BREAK_MINUTES: u32 = 15;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum PomodoroMode {
    Focus,
    ShortBreak,
    LongBreak,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum PomodoroStatus {
    Stopped,
    Running,
    Paused,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PomodoroSettings {
    pub(crate) focus_minutes: u32,
    pub(crate) short_break_minutes: u32,
    pub(crate) long_break_minutes: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct PomodoroState {
    pub(crate) mode: PomodoroMode,
    pub(crate) status: PomodoroStatus,
    pub(crate) started_at: Option<Instant>,
    pub(crate) paused_remaining: Duration,
    pub(crate) completed_focus_count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PomodoroTick {
    None,
    FocusComplete,
    BreakComplete,
}

impl Default for PomodoroSettings {
    fn default() -> Self {
        Self {
            focus_minutes: DEFAULT_FOCUS_MINUTES,
            short_break_minutes: DEFAULT_SHORT_BREAK_MINUTES,
            long_break_minutes: DEFAULT_LONG_BREAK_MINUTES,
        }
    }
}

impl PomodoroSettings {
    pub(crate) fn focus_minutes(&self) -> u32 {
        clamped_minutes(self.focus_minutes)
    }

    pub(crate) fn short_break_minutes(&self) -> u32 {
        clamped_minutes(self.short_break_minutes)
    }

    pub(crate) fn long_break_minutes(&self) -> u32 {
        clamped_minutes(self.long_break_minutes)
    }

    pub(crate) fn duration_for(&self, mode: PomodoroMode) -> Duration {
        match mode {
            PomodoroMode::Focus => minutes(self.focus_minutes()),
            PomodoroMode::ShortBreak => minutes(self.short_break_minutes()),
            PomodoroMode::LongBreak => minutes(self.long_break_minutes()),
        }
    }

    pub(crate) fn normalize(&mut self) -> bool {
        let focus = self.focus_minutes();
        let short_break = self.short_break_minutes();
        let long_break = self.long_break_minutes();
        let changed = self.focus_minutes != focus
            || self.short_break_minutes != short_break
            || self.long_break_minutes != long_break;
        self.focus_minutes = focus;
        self.short_break_minutes = short_break;
        self.long_break_minutes = long_break;
        changed
    }
}

impl Default for PomodoroState {
    fn default() -> Self {
        Self {
            mode: PomodoroMode::Focus,
            status: PomodoroStatus::Stopped,
            started_at: None,
            paused_remaining: PomodoroSettings::default().duration_for(PomodoroMode::Focus),
            completed_focus_count: 0,
        }
    }
}

impl PomodoroState {
    pub(crate) fn start_focus(&mut self, settings: &PomodoroSettings) {
        self.mode = PomodoroMode::Focus;
        self.status = PomodoroStatus::Running;
        self.started_at = Some(Instant::now());
        self.paused_remaining = settings.duration_for(PomodoroMode::Focus);
    }

    pub(crate) fn stop(&mut self, settings: &PomodoroSettings) {
        self.status = PomodoroStatus::Stopped;
        self.started_at = None;
        self.paused_remaining = settings.duration_for(self.mode);
    }

    pub(crate) fn pause(&mut self, settings: &PomodoroSettings) {
        if self.status == PomodoroStatus::Running {
            self.paused_remaining = self.remaining(settings);
            self.started_at = None;
            self.status = PomodoroStatus::Paused;
        }
    }

    pub(crate) fn resume(&mut self, settings: &PomodoroSettings) {
        if self.status == PomodoroStatus::Paused {
            let elapsed = settings
                .duration_for(self.mode)
                .saturating_sub(self.paused_remaining);
            self.started_at = Instant::now()
                .checked_sub(elapsed)
                .or_else(|| Some(Instant::now()));
            self.status = PomodoroStatus::Running;
        }
    }

    pub(crate) fn skip(&mut self, settings: &PomodoroSettings) -> PomodoroTick {
        if self.status == PomodoroStatus::Stopped {
            return PomodoroTick::None;
        }
        self.paused_remaining = Duration::ZERO;
        self.started_at = Some(Instant::now() - settings.duration_for(self.mode));
        self.status = PomodoroStatus::Running;
        self.tick(settings)
    }

    pub(crate) fn remaining(&self, settings: &PomodoroSettings) -> Duration {
        match (self.status, self.started_at) {
            (PomodoroStatus::Running, Some(started_at)) => settings
                .duration_for(self.mode)
                .saturating_sub(started_at.elapsed()),
            (PomodoroStatus::Paused, _) => self.paused_remaining,
            _ => settings.duration_for(self.mode),
        }
    }

    pub(crate) fn tick(&mut self, settings: &PomodoroSettings) -> PomodoroTick {
        if self.status != PomodoroStatus::Running || self.remaining(settings) > Duration::ZERO {
            return PomodoroTick::None;
        }

        match self.mode {
            PomodoroMode::Focus => {
                self.completed_focus_count = self.completed_focus_count.saturating_add(1);
                self.mode = if self.completed_focus_count % 4 == 0 {
                    PomodoroMode::LongBreak
                } else {
                    PomodoroMode::ShortBreak
                };
                self.started_at = Some(Instant::now());
                self.paused_remaining = settings.duration_for(self.mode);
                PomodoroTick::FocusComplete
            }
            PomodoroMode::ShortBreak | PomodoroMode::LongBreak => {
                self.mode = PomodoroMode::Focus;
                self.status = PomodoroStatus::Stopped;
                self.started_at = None;
                self.paused_remaining = settings.duration_for(PomodoroMode::Focus);
                PomodoroTick::BreakComplete
            }
        }
    }
}

pub(crate) fn format_remaining(duration: Duration) -> String {
    let secs = duration.as_secs();
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

fn clamped_minutes(value: u32) -> u32 {
    value.clamp(POMODORO_MIN_MINUTES, POMODORO_MAX_MINUTES)
}

fn minutes(value: u32) -> Duration {
    Duration::from_secs(value as u64 * 60)
}

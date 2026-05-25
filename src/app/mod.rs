pub(crate) mod pomodoro;

use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use self::pomodoro::{PomodoroState, PomodoroStatus, PomodoroTick};
use crate::config::MAX_EVENTS;
#[cfg(windows)]
use crate::globals::PET_RENDERER;
use crate::settings::{LlmProfileDb, UserSettings, load_llm_profile_db, load_user_settings};

const THINKING_MIN_VISIBLE: Duration = Duration::from_millis(2_500);
const WORK_MIN_VISIBLE: Duration = Duration::from_millis(3_000);
const HAPPY_MIN_VISIBLE: Duration = Duration::from_millis(2_000);
const SUBAGENT_MIN_VISIBLE: Duration = Duration::from_millis(2_500);

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) enum PetMood {
    Idle,
    Thinking,
    Typing,
    Building,
    Permission,
    Happy,
    Error,
    Sleeping,
    Subagent,
}

impl Default for PetMood {
    fn default() -> Self {
        Self::Idle
    }
}

impl PetMood {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Thinking => "thinking",
            Self::Typing => "typing",
            Self::Building => "building",
            Self::Permission => "permission",
            Self::Happy => "happy",
            Self::Error => "error",
            Self::Sleeping => "sleeping",
            Self::Subagent => "subagent",
        }
    }

    pub(crate) fn is_active_work(self) -> bool {
        matches!(
            self,
            Self::Thinking | Self::Typing | Self::Building | Self::Subagent
        )
    }

    pub(crate) fn priority(self) -> u8 {
        match self {
            Self::Permission => 100,
            Self::Error => 90,
            Self::Building | Self::Typing => 80,
            Self::Subagent => 65,
            Self::Thinking => 60,
            Self::Happy => 40,
            Self::Sleeping => 20,
            Self::Idle => 10,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum PermissionDecision {
    AllowOnce,
    AllowAlways,
    Deny,
    Ignore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChoiceKind {
    AskUserQuestion,
    ExitPlanMode,
}

#[derive(Clone, Debug)]
pub(crate) enum ChoiceDecision {
    Submit(Vec<Vec<usize>>),
    Deny,
    Ignore,
}

pub(crate) struct PermissionWaiter {
    pub(crate) decision: Mutex<Option<PermissionDecision>>,
    pub(crate) ready: Condvar,
}

pub(crate) struct ChoiceWaiter {
    pub(crate) decision: Mutex<Option<ChoiceDecision>>,
    pub(crate) ready: Condvar,
}

#[derive(Clone)]
pub(crate) struct PendingPermission {
    pub(crate) id: u64,
    pub(crate) session_id: String,
    pub(crate) tool_name: String,
    pub(crate) summary: String,
    pub(crate) cwd: String,
    pub(crate) suggestions: Vec<Value>,
    pub(crate) waiter: Arc<PermissionWaiter>,
}

#[derive(Clone)]
pub(crate) struct ChoiceOption {
    pub(crate) label: String,
    pub(crate) description: String,
}

#[derive(Clone)]
pub(crate) struct ChoiceQuestion {
    pub(crate) header: String,
    pub(crate) question: String,
    pub(crate) multi_select: bool,
    pub(crate) options: Vec<ChoiceOption>,
}

#[derive(Clone)]
pub(crate) struct PendingChoice {
    pub(crate) id: u64,
    pub(crate) session_id: String,
    pub(crate) kind: ChoiceKind,
    pub(crate) title: String,
    pub(crate) detail: String,
    pub(crate) questions: Vec<ChoiceQuestion>,
    pub(crate) selected: Vec<Vec<usize>>,
    pub(crate) tool_input: Value,
    pub(crate) waiter: Arc<ChoiceWaiter>,
}

#[allow(dead_code)]
#[derive(Clone)]
pub(crate) struct EventRecord {
    pub(crate) event: String,
    pub(crate) detail: String,
}

#[derive(Clone)]
pub(crate) struct SpeechBubble {
    pub(crate) expires_at: Instant,
    pub(crate) priority: u8,
}

#[derive(Clone, Default)]
pub(crate) struct QuotaStats {
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) cache_creation_tokens: u64,
    pub(crate) cache_read_tokens: u64,
    pub(crate) total_tokens: u64,
    pub(crate) observed_total_tokens: u64,
    pub(crate) provider: String,
    pub(crate) quota_remaining: String,
    pub(crate) quota_limit: String,
    pub(crate) quota_reset: String,
    pub(crate) last_model: String,
    pub(crate) rate_limits: String,
    pub(crate) transcript_path: String,
}

#[allow(dead_code)]
#[derive(Clone)]
pub(crate) struct SessionInfo {
    pub(crate) last_event: String,
    pub(crate) cwd: String,
    pub(crate) updated_at: Instant,
}

#[derive(Clone, Debug)]
pub(crate) struct ActiveTool {
    pub(crate) tool_name: String,
    pub(crate) mood: PetMood,
}

pub(crate) struct AppState {
    pub(crate) mood: PetMood,
    pub(crate) mood_started_at: Instant,
    pub(crate) resting_mood: PetMood,
    pub(crate) resting_interrupts_visual: bool,
    pub(crate) last_non_idle_mood: PetMood,
    pub(crate) last_activity: Instant,
    pub(crate) last_user_input_tick: Option<u32>,
    pub(crate) last_error: String,
    pub(crate) next_permission_id: u64,
    pub(crate) next_choice_id: u64,
    pub(crate) active_tools: usize,
    pub(crate) active_tool_moods: HashMap<PetMood, usize>,
    pub(crate) active_tool_keys: HashMap<String, ActiveTool>,
    pub(crate) active_tool_names: HashMap<String, VecDeque<String>>,
    pub(crate) active_subagents: usize,
    pub(crate) sessions: HashMap<String, SessionInfo>,
    pub(crate) events: VecDeque<EventRecord>,
    pub(crate) pending_permissions: VecDeque<PendingPermission>,
    pub(crate) pending_choices: VecDeque<PendingChoice>,
    pub(crate) quota: QuotaStats,
    pub(crate) settings: UserSettings,
    pub(crate) llm_profiles: LlmProfileDb,
    pub(crate) pomodoro: PomodoroState,
    pub(crate) speech: Option<SpeechBubble>,
}

impl AppState {
    pub(crate) fn new() -> Self {
        Self {
            mood: PetMood::Idle,
            mood_started_at: Instant::now(),
            resting_mood: PetMood::Idle,
            resting_interrupts_visual: false,
            last_non_idle_mood: PetMood::Idle,
            last_activity: Instant::now(),
            last_user_input_tick: None,
            last_error: String::new(),
            next_permission_id: 1,
            next_choice_id: 1,
            active_tools: 0,
            active_tool_moods: HashMap::new(),
            active_tool_keys: HashMap::new(),
            active_tool_names: HashMap::new(),
            active_subagents: 0,
            sessions: HashMap::new(),
            events: VecDeque::new(),
            pending_permissions: VecDeque::new(),
            pending_choices: VecDeque::new(),
            quota: QuotaStats::default(),
            settings: load_user_settings(),
            llm_profiles: load_llm_profile_db(),
            pomodoro: PomodoroState::default(),
            speech: None,
        }
    }

    pub(crate) fn push_event(&mut self, event: impl Into<String>, detail: impl Into<String>) {
        self.events.push_front(EventRecord {
            event: event.into(),
            detail: detail.into(),
        });
        while self.events.len() > MAX_EVENTS {
            self.events.pop_back();
        }
        self.last_activity = Instant::now();
    }

    pub(crate) fn set_mood(&mut self, mood: PetMood) {
        self.last_activity = Instant::now();
        self.resting_mood = mood;
        self.resting_interrupts_visual = matches!(mood, PetMood::Permission | PetMood::Error);
        if matches!(mood, PetMood::Permission | PetMood::Error) {
            self.force_visual_mood(mood);
            return;
        }
        self.refresh_visual_mood();
    }

    pub(crate) fn set_resting_mood(&mut self, mood: PetMood, interrupts_visual: bool) {
        self.last_activity = Instant::now();
        self.resting_mood = mood;
        self.resting_interrupts_visual = interrupts_visual;
        self.refresh_visual_mood();
    }

    fn force_visual_mood(&mut self, mood: PetMood) {
        if !matches!(mood, PetMood::Idle | PetMood::Sleeping) {
            self.last_non_idle_mood = mood;
        }
        if request_renderer_mood(mood) {
            self.mood = mood;
            self.mood_started_at = Instant::now();
        }
    }

    pub(crate) fn start_tool_activity(
        &mut self,
        mut key: String,
        tool_name: String,
        mood: PetMood,
    ) {
        if self.active_tool_keys.contains_key(&key) {
            let base_key = key.clone();
            let mut suffix = 2_u32;
            while self.active_tool_keys.contains_key(&key) {
                key = format!("{base_key}#{suffix}");
                suffix = suffix.saturating_add(1);
            }
        }
        self.active_tools = self.active_tools.saturating_add(1);
        if mood.is_active_work() {
            *self.active_tool_moods.entry(mood).or_insert(0) += 1;
        }
        let name_key = normalize_tool_name_key(&tool_name);
        self.active_tool_names
            .entry(name_key)
            .or_default()
            .push_back(key.clone());
        self.active_tool_keys
            .insert(key, ActiveTool { tool_name, mood });
        self.last_activity = Instant::now();
        self.refresh_visual_mood();
    }

    #[cfg(test)]
    pub(crate) fn start_tool_mood(&mut self, mood: PetMood) {
        let key = format!(
            "legacy:{}:{}",
            mood.key(),
            self.active_tools.saturating_add(1)
        );
        self.start_tool_activity(key, "tool".to_string(), mood);
    }

    pub(crate) fn finish_tool_mood(&mut self, mood: PetMood) {
        if let Some(key) = self
            .active_tool_keys
            .iter()
            .find(|(_, tool)| tool.mood == mood)
            .map(|(key, _)| key.clone())
        {
            if let Some(tool) = self.active_tool_keys.remove(&key) {
                self.remove_tool_name_key(&tool.tool_name, &key);
            }
        }
        self.active_tools = self.active_tools.saturating_sub(1);
        self.decrement_active_tool_mood(mood);
        self.refresh_visual_mood();
    }

    pub(crate) fn finish_tool_activity(
        &mut self,
        keys: &[String],
        tool_name: &str,
        fallback_mood: PetMood,
    ) -> PetMood {
        let key = keys
            .iter()
            .find(|key| self.active_tool_keys.contains_key(*key))
            .cloned()
            .or_else(|| self.pop_named_tool_key(tool_name));

        let Some(key) = key else {
            self.finish_tool_mood(fallback_mood);
            return fallback_mood;
        };

        let Some(tool) = self.active_tool_keys.remove(&key) else {
            self.finish_tool_mood(fallback_mood);
            return fallback_mood;
        };
        self.remove_tool_name_key(&tool.tool_name, &key);
        self.active_tools = self.active_tools.saturating_sub(1);
        self.decrement_active_tool_mood(tool.mood);
        self.last_activity = Instant::now();
        self.refresh_visual_mood();
        tool.mood
    }

    pub(crate) fn finish_all_tools(&mut self) {
        self.active_tools = 0;
        self.active_tool_moods.clear();
        self.active_tool_keys.clear();
        self.active_tool_names.clear();
        self.refresh_visual_mood();
    }

    pub(crate) fn clear_activity(&mut self) {
        self.finish_all_tools();
        self.active_subagents = 0;
        self.refresh_visual_mood();
    }

    pub(crate) fn activity_mood(&self) -> Option<PetMood> {
        let direct_tool_mood = self
            .active_tool_moods
            .iter()
            .filter(|(mood, count)| **count > 0 && !matches!(mood, PetMood::Subagent))
            .map(|(mood, _)| *mood)
            .max_by_key(|mood| mood.priority());
        if direct_tool_mood.is_some() {
            return direct_tool_mood;
        }
        let subagent_tools = self
            .active_tool_moods
            .get(&PetMood::Subagent)
            .copied()
            .unwrap_or(0);
        if self.active_subagents > 0 || subagent_tools > 0 {
            return Some(PetMood::Subagent);
        }
        None
    }

    pub(crate) fn start_subagent(&mut self) {
        self.active_subagents = self.active_subagents.saturating_add(1);
        self.last_activity = Instant::now();
        self.refresh_visual_mood();
    }

    pub(crate) fn finish_subagent(&mut self) {
        self.active_subagents = self.active_subagents.saturating_sub(1);
        self.last_activity = Instant::now();
        self.refresh_visual_mood();
    }

    pub(crate) fn refresh_visual_mood(&mut self) {
        self.refresh_visual_mood_at(Instant::now());
    }

    pub(crate) fn refresh_visual_mood_at(&mut self, now: Instant) {
        let target = self.projected_mood();
        if self.can_switch_visual_to(target, now) {
            self.transition_visual_mood(target, now);
        }
    }

    fn projected_mood(&self) -> PetMood {
        self.activity_mood().unwrap_or(self.resting_mood)
    }

    fn can_switch_visual_to(&self, target: PetMood, now: Instant) -> bool {
        if target == self.mood {
            return true;
        }
        if matches!(target, PetMood::Permission | PetMood::Error) {
            return true;
        }
        if self.resting_interrupts_visual && target == self.resting_mood {
            return true;
        }
        if target.priority() > self.mood.priority() {
            return true;
        }
        now.duration_since(self.mood_started_at) >= min_visible_for(self.mood)
    }

    fn transition_visual_mood(&mut self, mood: PetMood, now: Instant) {
        if !matches!(mood, PetMood::Idle | PetMood::Sleeping) {
            self.last_non_idle_mood = mood;
        }
        if request_renderer_mood(mood) {
            if self.mood != mood {
                self.mood_started_at = now;
            }
            self.mood = mood;
            if mood == self.resting_mood {
                self.resting_interrupts_visual = false;
            }
        }
    }

    fn pop_named_tool_key(&mut self, tool_name: &str) -> Option<String> {
        let name_key = normalize_tool_name_key(tool_name);
        let queue = self.active_tool_names.get_mut(&name_key)?;
        let key = queue.pop_front();
        if queue.is_empty() {
            self.active_tool_names.remove(&name_key);
        }
        key
    }

    fn remove_tool_name_key(&mut self, tool_name: &str, key: &str) {
        let name_key = normalize_tool_name_key(tool_name);
        let Some(queue) = self.active_tool_names.get_mut(&name_key) else {
            return;
        };
        queue.retain(|item| item != key);
        if queue.is_empty() {
            self.active_tool_names.remove(&name_key);
        }
    }

    fn decrement_active_tool_mood(&mut self, mood: PetMood) {
        if let Some(count) = self.active_tool_moods.get_mut(&mood) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.active_tool_moods.remove(&mood);
            }
            return;
        }

        if let Some(fallback) = self
            .active_tool_moods
            .iter()
            .find(|(_, count)| **count > 0)
            .map(|(mood, _)| *mood)
        {
            if let Some(count) = self.active_tool_moods.get_mut(&fallback) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    self.active_tool_moods.remove(&fallback);
                }
            }
        }
    }

    pub(crate) fn show_speech(
        &mut self,
        _title: impl Into<String>,
        _detail: impl Into<String>,
        duration: Duration,
        priority: u8,
    ) {
        if self
            .speech
            .as_ref()
            .is_some_and(|speech| speech.expires_at > Instant::now() && speech.priority > priority)
        {
            return;
        }
        self.speech = Some(SpeechBubble {
            expires_at: Instant::now() + duration,
            priority,
        });
    }

    pub(crate) fn start_pomodoro(&mut self) {
        self.pomodoro.start_focus(&self.settings.pomodoro);
        self.set_mood(PetMood::Thinking);
        self.show_speech(
            "Pomodoro started",
            format!(
                "{}:00 focus session",
                self.settings.pomodoro.focus_minutes()
            ),
            Duration::from_secs(4),
            5,
        );
    }

    pub(crate) fn stop_pomodoro(&mut self) {
        self.pomodoro.stop(&self.settings.pomodoro);
        self.show_speech(
            "Pomodoro stopped",
            "Timer cleared",
            Duration::from_secs(3),
            5,
        );
    }

    pub(crate) fn pause_pomodoro(&mut self) {
        if self.pomodoro.status != PomodoroStatus::Running {
            self.show_speech(
                "Pomodoro idle",
                "Start a focus session first",
                Duration::from_secs(3),
                4,
            );
            return;
        }
        self.pomodoro.pause(&self.settings.pomodoro);
        self.show_speech(
            "Pomodoro paused",
            "Focus timer is waiting",
            Duration::from_secs(3),
            5,
        );
    }

    pub(crate) fn resume_pomodoro(&mut self) {
        if self.pomodoro.status != PomodoroStatus::Paused {
            self.show_speech(
                "Pomodoro not paused",
                "Nothing to resume yet",
                Duration::from_secs(3),
                4,
            );
            return;
        }
        self.pomodoro.resume(&self.settings.pomodoro);
        self.show_speech(
            "Pomodoro resumed",
            "Back to focus",
            Duration::from_secs(3),
            5,
        );
    }

    pub(crate) fn skip_pomodoro(&mut self) {
        match self.pomodoro.skip(&self.settings.pomodoro) {
            PomodoroTick::FocusComplete => {
                self.set_mood(PetMood::Happy);
                self.show_speech("Focus skipped", "Moved to break", Duration::from_secs(5), 7);
            }
            PomodoroTick::BreakComplete => {
                self.set_mood(PetMood::Idle);
                self.show_speech(
                    "Break skipped",
                    "Ready for focus",
                    Duration::from_secs(4),
                    6,
                );
            }
            PomodoroTick::None => {
                self.show_speech(
                    "No pomodoro",
                    "Start a focus session first",
                    Duration::from_secs(3),
                    4,
                );
            }
        }
    }

    pub(crate) fn tick_pomodoro(&mut self) {
        match self.pomodoro.tick(&self.settings.pomodoro) {
            PomodoroTick::FocusComplete => {
                self.set_mood(PetMood::Happy);
                self.show_speech("Focus complete", "Break time", Duration::from_secs(6), 7);
            }
            PomodoroTick::BreakComplete => {
                self.set_mood(PetMood::Idle);
                self.show_speech(
                    "Break complete",
                    "Ready for the next focus session",
                    Duration::from_secs(5),
                    6,
                );
            }
            PomodoroTick::None => {}
        }
    }

    pub(crate) fn decay_mood(
        &mut self,
        user_idle_for: Option<Duration>,
        user_input_tick: Option<u32>,
    ) {
        if self
            .speech
            .as_ref()
            .is_some_and(|speech| speech.expires_at <= Instant::now())
        {
            self.speech = None;
        }

        let user_input_changed = user_input_tick.is_some_and(|tick| {
            let changed = self
                .last_user_input_tick
                .is_some_and(|previous| previous != tick);
            self.last_user_input_tick = Some(tick);
            changed
        });

        if matches!(self.mood, PetMood::Sleeping) && user_input_changed {
            self.set_resting_mood(PetMood::Idle, true);
            return;
        }

        let idle_for = self.last_activity.elapsed();
        let sleep_after = Duration::from_secs(self.settings.sleep_after_secs() as u64);
        if self.pending_permissions.is_empty()
            && self.pending_choices.is_empty()
            && user_idle_for.is_some_and(|idle| idle > sleep_after)
            && self.active_tools == 0
            && self.active_subagents == 0
            && !matches!(self.resting_mood, PetMood::Sleeping)
        {
            self.set_resting_mood(PetMood::Sleeping, false);
        } else if matches!(self.resting_mood, PetMood::Happy | PetMood::Error)
            && idle_for > Duration::from_secs(7)
        {
            self.set_resting_mood(PetMood::Idle, false);
        }
        self.refresh_visual_mood();
    }
}

fn min_visible_for(mood: PetMood) -> Duration {
    match mood {
        PetMood::Typing | PetMood::Building => WORK_MIN_VISIBLE,
        PetMood::Thinking => THINKING_MIN_VISIBLE,
        PetMood::Subagent => SUBAGENT_MIN_VISIBLE,
        PetMood::Happy => HAPPY_MIN_VISIBLE,
        _ => Duration::ZERO,
    }
}

fn normalize_tool_name_key(tool_name: &str) -> String {
    tool_name.trim().to_ascii_lowercase()
}

#[cfg(windows)]
fn request_renderer_mood(mood: PetMood) -> bool {
    match PET_RENDERER.get() {
        Some(renderer) => renderer
            .lock()
            .expect("pet renderer poisoned")
            .request_mood(mood),
        None => true,
    }
}

#[cfg(not(windows))]
fn request_renderer_mood(_mood: PetMood) -> bool {
    true
}

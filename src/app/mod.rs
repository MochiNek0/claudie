pub(crate) mod pomodoro;
pub(crate) mod stats;

use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use self::pomodoro::{PomodoroMode, PomodoroState, PomodoroStatus, PomodoroTick};
use self::stats::{DailyStatsDb, ToolStatsKind, load_daily_stats};
use crate::config::MAX_EVENTS;
#[cfg(windows)]
use crate::globals::PET_RENDERER;
use crate::settings::{LlmProfileDb, UserSettings, load_llm_profile_db, load_user_settings};

const THINKING_MIN_VISIBLE: Duration = Duration::from_millis(2_500);
const WORK_MIN_VISIBLE: Duration = Duration::from_millis(3_000);
const HAPPY_MIN_VISIBLE: Duration = Duration::from_millis(2_000);
const SUBAGENT_MIN_VISIBLE: Duration = Duration::from_millis(2_500);
const INTERACTION_MIN_VISIBLE: Duration = Duration::from_millis(1_800);

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) enum PetMood {
    Idle,
    Thinking,
    Typing,
    Building,
    Search,
    Happy,
    Error,
    Sleeping,
    Subagent,
    Pomodoro,
    Wave,
    Stretch,
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
            Self::Search => "search",
            Self::Happy => "happy",
            Self::Error => "error",
            Self::Sleeping => "sleeping",
            Self::Subagent => "subagent",
            Self::Pomodoro => "pomodoro",
            Self::Wave => "wave",
            Self::Stretch => "stretch",
        }
    }

    pub(crate) fn is_active_work(self) -> bool {
        matches!(
            self,
            Self::Thinking | Self::Typing | Self::Building | Self::Search | Self::Subagent
        )
    }

    pub(crate) fn priority(self) -> u8 {
        match self {
            Self::Error => 90,
            Self::Building | Self::Typing => 80,
            Self::Search => 70,
            Self::Subagent => 65,
            Self::Thinking => 60,
            Self::Pomodoro => 45,
            Self::Wave | Self::Stretch => 42,
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
    Submit {
        selected: Vec<Vec<usize>>,
        other_text: Vec<String>,
    },
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
    pub(crate) is_other: bool,
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
    pub(crate) other_text: Vec<String>,
    pub(crate) tool_input: Value,
    pub(crate) waiter: Arc<ChoiceWaiter>,
}

impl PendingChoice {
    pub(crate) fn is_submittable(&self) -> bool {
        self.questions.iter().enumerate().all(|(qi, question)| {
            let Some(selected) = self.selected.get(qi) else {
                return false;
            };
            if selected.is_empty() {
                return false;
            }
            let other_text = self
                .other_text
                .get(qi)
                .map(|text| text.trim())
                .unwrap_or("");
            selected.iter().all(|&oi| match question.options.get(oi) {
                Some(option) if option.is_other => !other_text.is_empty(),
                Some(_) => true,
                None => false,
            })
        })
    }
}

#[allow(dead_code)]
#[derive(Clone)]
pub(crate) struct EventRecord {
    pub(crate) event: String,
    pub(crate) detail: String,
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
    pub(crate) stats: DailyStatsDb,
    stats_last_total_tokens: u64,
    stats_last_input_tokens: u64,
    stats_last_output_tokens: u64,
    stats_last_cache_creation_tokens: u64,
    stats_last_cache_read_tokens: u64,
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
            stats: load_daily_stats(),
            stats_last_total_tokens: 0,
            stats_last_input_tokens: 0,
            stats_last_output_tokens: 0,
            stats_last_cache_creation_tokens: 0,
            stats_last_cache_read_tokens: 0,
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
        self.resting_interrupts_visual = matches!(mood, PetMood::Error);
        if matches!(mood, PetMood::Error) {
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
        if !matches!(mood, PetMood::Idle | PetMood::Sleeping | PetMood::Pomodoro) {
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
        if matches!(target, PetMood::Error) {
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
        if !matches!(mood, PetMood::Idle | PetMood::Sleeping | PetMood::Pomodoro) {
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

    pub(crate) fn start_pomodoro(&mut self) {
        self.pomodoro.start_focus(&self.settings.pomodoro);
        self.set_mood(PetMood::Pomodoro);
    }

    pub(crate) fn stop_pomodoro(&mut self) {
        self.pomodoro.stop(&self.settings.pomodoro);
    }

    pub(crate) fn pause_pomodoro(&mut self) {
        if self.pomodoro.status != PomodoroStatus::Running {
            return;
        }
        self.pomodoro.pause(&self.settings.pomodoro);
        self.set_mood(self.default_resting_mood());
    }

    pub(crate) fn resume_pomodoro(&mut self) {
        if self.pomodoro.status != PomodoroStatus::Paused {
            return;
        }
        self.pomodoro.resume(&self.settings.pomodoro);
        self.set_mood(self.default_resting_mood());
    }

    pub(crate) fn interact_with_pet(&mut self) {
        if !self.pending_permissions.is_empty() || !self.pending_choices.is_empty() {
            return;
        }
        let mood = if matches!(self.mood, PetMood::Sleeping)
            || matches!(self.resting_mood, PetMood::Sleeping)
        {
            PetMood::Stretch
        } else {
            PetMood::Wave
        };
        self.set_resting_mood(mood, true);
        self.push_event("pet", mood.key());
    }

    pub(crate) fn skip_pomodoro(&mut self) {
        match self.pomodoro.skip(&self.settings.pomodoro) {
            PomodoroTick::FocusComplete => {
                self.record_completed_focus();
                self.set_mood(PetMood::Happy);
            }
            PomodoroTick::BreakComplete => {
                self.set_mood(PetMood::Idle);
            }
            PomodoroTick::None => {}
        }
    }

    pub(crate) fn tick_pomodoro(&mut self) {
        match self.pomodoro.tick(&self.settings.pomodoro) {
            PomodoroTick::FocusComplete => {
                self.record_completed_focus();
                self.set_mood(PetMood::Happy);
            }
            PomodoroTick::BreakComplete => {
                self.set_mood(PetMood::Idle);
            }
            PomodoroTick::None => {}
        }
    }

    pub(crate) fn decay_mood(
        &mut self,
        user_idle_for: Option<Duration>,
        user_input_tick: Option<u32>,
    ) {
        if let Some(tick) = user_input_tick {
            self.last_user_input_tick = Some(tick);
        }

        let idle_for = self.last_activity.elapsed();
        let sleep_after = Duration::from_secs(self.settings.sleep_after_secs() as u64);
        if self.pending_permissions.is_empty()
            && self.pending_choices.is_empty()
            && user_idle_for.is_some_and(|idle| idle > sleep_after)
            && !self.has_active_work()
            && !matches!(self.resting_mood, PetMood::Sleeping)
        {
            self.set_resting_mood(PetMood::Sleeping, false);
        } else if matches!(self.resting_mood, PetMood::Wave | PetMood::Stretch)
            && idle_for > Duration::from_secs(3)
        {
            let target = if !self.pending_permissions.is_empty() || !self.pending_choices.is_empty()
            {
                PetMood::Thinking
            } else {
                self.default_resting_mood()
            };
            self.set_resting_mood(target, false);
        } else if matches!(self.resting_mood, PetMood::Happy | PetMood::Error)
            && idle_for > Duration::from_secs(7)
        {
            self.set_resting_mood(PetMood::Idle, false);
        }
        self.refresh_visual_mood();
    }

    fn has_active_work(&self) -> bool {
        self.activity_mood().is_some()
            || self.resting_mood.is_active_work()
            || self.is_focus_pomodoro_running()
    }

    fn default_resting_mood(&self) -> PetMood {
        if self.is_focus_pomodoro_running() {
            PetMood::Pomodoro
        } else {
            PetMood::Idle
        }
    }

    fn is_focus_pomodoro_running(&self) -> bool {
        self.pomodoro.status == PomodoroStatus::Running && self.pomodoro.mode == PomodoroMode::Focus
    }

    pub(crate) fn record_prompt_stats(&mut self) {
        self.stats.record(|day| {
            day.prompts = day.prompts.saturating_add(1);
        });
    }

    pub(crate) fn record_tool_stats(&mut self, kind: ToolStatsKind) {
        self.stats.record(|day| {
            day.tool_uses = day.tool_uses.saturating_add(1);
            match kind {
                ToolStatsKind::Write => day.write_tools = day.write_tools.saturating_add(1),
                ToolStatsKind::Bash => day.bash_tools = day.bash_tools.saturating_add(1),
                ToolStatsKind::Search => day.search_tools = day.search_tools.saturating_add(1),
                ToolStatsKind::Subagent => {
                    day.subagent_tools = day.subagent_tools.saturating_add(1);
                }
                ToolStatsKind::Other => {}
            }
        });
    }

    pub(crate) fn record_permission_stats(&mut self) {
        self.stats.record(|day| {
            day.permission_requests = day.permission_requests.saturating_add(1);
        });
    }

    pub(crate) fn record_choice_stats(&mut self) {
        self.stats.record(|day| {
            day.choice_requests = day.choice_requests.saturating_add(1);
        });
    }

    pub(crate) fn record_error_stats(&mut self) {
        self.stats.record(|day| {
            day.errors = day.errors.saturating_add(1);
        });
    }

    pub(crate) fn record_token_snapshot(&mut self) {
        let input_delta = self
            .quota
            .input_tokens
            .saturating_sub(self.stats_last_input_tokens);
        let output_delta = self
            .quota
            .output_tokens
            .saturating_sub(self.stats_last_output_tokens);
        let cache_creation_delta = self
            .quota
            .cache_creation_tokens
            .saturating_sub(self.stats_last_cache_creation_tokens);
        let cache_read_delta = self
            .quota
            .cache_read_tokens
            .saturating_sub(self.stats_last_cache_read_tokens);
        let category_delta = input_delta
            .saturating_add(output_delta)
            .saturating_add(cache_creation_delta)
            .saturating_add(cache_read_delta);
        let total = self
            .quota
            .observed_total_tokens
            .max(self.quota.total_tokens)
            .max(
                self.quota
                    .input_tokens
                    .saturating_add(self.quota.output_tokens)
                    .saturating_add(self.quota.cache_creation_tokens)
                    .saturating_add(self.quota.cache_read_tokens),
            );
        if category_delta > 0 {
            self.stats.record(|day| {
                day.input_tokens = day.input_tokens.saturating_add(input_delta);
                day.output_tokens = day.output_tokens.saturating_add(output_delta);
                day.cache_creation_tokens = day
                    .cache_creation_tokens
                    .saturating_add(cache_creation_delta);
                day.cache_read_tokens = day.cache_read_tokens.saturating_add(cache_read_delta);
                day.token_delta = day.token_delta.saturating_add(category_delta);
            });
        } else if total > self.stats_last_total_tokens {
            let total_delta = total - self.stats_last_total_tokens;
            self.stats.record(|day| {
                day.token_delta = day.token_delta.saturating_add(total_delta);
            });
        }
        self.stats_last_total_tokens = total;
        self.stats_last_input_tokens = self.quota.input_tokens;
        self.stats_last_output_tokens = self.quota.output_tokens;
        self.stats_last_cache_creation_tokens = self.quota.cache_creation_tokens;
        self.stats_last_cache_read_tokens = self.quota.cache_read_tokens;
    }

    fn record_completed_focus(&mut self) {
        self.stats.record(|day| {
            day.completed_focus = day.completed_focus.saturating_add(1);
        });
    }
}

fn min_visible_for(mood: PetMood) -> Duration {
    match mood {
        PetMood::Typing | PetMood::Building => WORK_MIN_VISIBLE,
        PetMood::Thinking | PetMood::Search => THINKING_MIN_VISIBLE,
        PetMood::Subagent => SUBAGENT_MIN_VISIBLE,
        PetMood::Pomodoro | PetMood::Wave | PetMood::Stretch => INTERACTION_MIN_VISIBLE,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sleep_ignores_passive_user_input_until_pet_interaction() {
        let mut state = AppState::new();
        state.settings.sleep_after_secs = 15;

        state.decay_mood(Some(Duration::from_secs(16)), Some(10));
        assert_eq!(state.resting_mood, PetMood::Sleeping);
        assert_eq!(state.mood, PetMood::Sleeping);

        state.decay_mood(Some(Duration::ZERO), Some(11));
        assert_eq!(state.resting_mood, PetMood::Sleeping);
        assert_eq!(state.mood, PetMood::Sleeping);

        state.interact_with_pet();
        assert_eq!(state.resting_mood, PetMood::Stretch);
        assert_eq!(state.mood, PetMood::Stretch);
    }

    #[test]
    fn active_thinking_prevents_sleep_when_user_is_idle() {
        let mut state = AppState::new();
        state.settings.sleep_after_secs = 15;

        state.set_resting_mood(PetMood::Thinking, true);
        state.decay_mood(Some(Duration::from_secs(16)), Some(10));

        assert_eq!(state.resting_mood, PetMood::Thinking);
        assert_eq!(state.mood, PetMood::Thinking);
    }

    #[test]
    fn active_tool_prevents_sleep_when_user_is_idle() {
        let mut state = AppState::new();
        state.settings.sleep_after_secs = 15;

        state.start_tool_mood(PetMood::Building);
        state.decay_mood(Some(Duration::from_secs(16)), Some(10));

        assert_eq!(state.activity_mood(), Some(PetMood::Building));
        assert_eq!(state.mood, PetMood::Building);
    }
}

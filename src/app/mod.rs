pub(crate) mod fishing;
pub(crate) mod pomodoro;
pub(crate) mod stats;

use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use self::fishing::{FishingPhase, FishingState, FishingTick};
use self::pomodoro::{PomodoroMode, PomodoroState, PomodoroStatus, PomodoroTick};
use self::stats::{DailyStatsDb, ToolStatsKind, load_daily_stats};
use crate::globals::PET_RENDERER;
use crate::settings::{LlmProfileDb, UserSettings, load_llm_profile_db, load_user_settings};

const THINKING_MIN_VISIBLE: Duration = Duration::from_millis(2_500);
const WORK_MIN_VISIBLE: Duration = Duration::from_millis(3_000);
const HAPPY_MIN_VISIBLE: Duration = Duration::from_millis(2_000);
const SUBAGENT_MIN_VISIBLE: Duration = Duration::from_millis(2_500);
const INTERACTION_MIN_VISIBLE: Duration = Duration::from_millis(1_800);
const STATS_FLUSH_INTERVAL: Duration = Duration::from_secs(3);
const RESTING_ACTIVITY_KEY: &str = "resting";
const SUBAGENT_ACTIVITY_KEY: &str = "subagent";
const POMODORO_ACTIVITY_KEY: &str = "pomodoro";
const INTERACTION_ACTIVITY_KEY: &str = "interaction";
const FISHING_ACTIVITY_KEY: &str = "fishing";

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
    Fishing,
    FishingReel,
    FishingCaught,
    FishingMissed,
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
            Self::Fishing => "fishing",
            Self::FishingReel => "fishing_reel",
            Self::FishingCaught => "fishing_caught",
            Self::FishingMissed => "fishing_missed",
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
            Self::Fishing | Self::FishingReel => 47,
            Self::FishingCaught | Self::FishingMissed => 46,
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
    pub(crate) interaction_sequence: u64,
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
    pub(crate) interaction_sequence: u64,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PendingInteractionKind {
    Permission,
    Choice,
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
    pub(crate) next_interaction_sequence: u64,
    pub(crate) next_permission_id: u64,
    pub(crate) next_choice_id: u64,
    pub(crate) active_tools: usize,
    pub(crate) active_tool_moods: HashMap<PetMood, usize>,
    pub(crate) active_tool_keys: HashMap<String, ActiveTool>,
    pub(crate) active_tool_names: HashMap<String, VecDeque<String>>,
    pub(crate) active_subagents: usize,
    pub(crate) activity_spans: HashMap<String, ActivitySpan>,
    pub(crate) pending_permissions: VecDeque<PendingPermission>,
    pub(crate) pending_choices: VecDeque<PendingChoice>,
    pub(crate) quota: QuotaStats,
    pub(crate) settings: UserSettings,
    pub(crate) llm_profiles: LlmProfileDb,
    pub(crate) pomodoro: PomodoroState,
    pub(crate) fishing: FishingState,
    pub(crate) stats: DailyStatsDb,
    stats_last_total_tokens: u64,
    stats_last_input_tokens: u64,
    stats_last_output_tokens: u64,
    stats_last_cache_creation_tokens: u64,
    stats_last_cache_read_tokens: u64,
    stats_last_flush: Instant,
}

impl AppState {
    pub(crate) fn new() -> Self {
        let now = Instant::now();
        let mut state = Self {
            mood: PetMood::Idle,
            mood_started_at: now,
            resting_mood: PetMood::Idle,
            resting_interrupts_visual: false,
            last_non_idle_mood: PetMood::Idle,
            last_activity: now,
            last_user_input_tick: None,
            last_error: String::new(),
            next_interaction_sequence: 1,
            next_permission_id: 1,
            next_choice_id: 1,
            active_tools: 0,
            active_tool_moods: HashMap::new(),
            active_tool_keys: HashMap::new(),
            active_tool_names: HashMap::new(),
            active_subagents: 0,
            activity_spans: HashMap::new(),
            pending_permissions: VecDeque::new(),
            pending_choices: VecDeque::new(),
            quota: QuotaStats::default(),
            settings: load_user_settings(),
            llm_profiles: load_llm_profile_db(),
            pomodoro: PomodoroState::default(),
            fishing: FishingState::default(),
            stats: load_daily_stats(),
            stats_last_total_tokens: 0,
            stats_last_input_tokens: 0,
            stats_last_output_tokens: 0,
            stats_last_cache_creation_tokens: 0,
            stats_last_cache_read_tokens: 0,
            stats_last_flush: now,
        };
        state.set_resting_activity_at(PetMood::Idle, false, now);
        state
    }

    pub(crate) fn set_mood(&mut self, mood: PetMood) {
        let now = Instant::now();
        self.last_activity = now;
        self.set_resting_activity_at(mood, matches!(mood, PetMood::Error), now);
        if matches!(mood, PetMood::Error) {
            self.force_visual_mood(mood);
            return;
        }
        self.refresh_visual_mood();
    }

    pub(crate) fn set_resting_mood(&mut self, mood: PetMood, interrupts_visual: bool) {
        let now = Instant::now();
        self.last_activity = now;
        self.set_resting_activity_at(mood, interrupts_visual, now);
        self.refresh_visual_mood();
    }

    fn force_visual_mood(&mut self, mood: PetMood) {
        if !matches!(
            mood,
            PetMood::Idle | PetMood::Sleeping | PetMood::Pomodoro | PetMood::Fishing
        ) {
            self.last_non_idle_mood = mood;
        }
        if request_renderer_mood(mood) {
            self.mood = mood;
            self.mood_started_at = Instant::now();
        }
    }

    fn set_resting_activity_at(&mut self, mood: PetMood, interrupts_visual: bool, now: Instant) {
        self.resting_mood = mood;
        self.resting_interrupts_visual = interrupts_visual;
        if !matches!(mood, PetMood::Wave | PetMood::Stretch) {
            self.activity_spans.remove(INTERACTION_ACTIVITY_KEY);
        }
        self.activity_spans.insert(
            RESTING_ACTIVITY_KEY.to_string(),
            ActivitySpan::at(
                RESTING_ACTIVITY_KEY,
                "",
                ActivityKind::Resting,
                mood,
                mood.priority(),
                interrupts_visual,
                now,
            ),
        );
    }

    pub(crate) fn begin_activity_span(&mut self, span: ActivitySpan) {
        self.last_activity = Instant::now();
        self.activity_spans.insert(span.key.clone(), span);
        self.refresh_visual_mood();
    }

    pub(crate) fn end_activity_span(&mut self, key: &str) -> Option<ActivitySpan> {
        let removed = self.activity_spans.remove(key);
        if removed.is_some() {
            self.last_activity = Instant::now();
            self.refresh_visual_mood();
        }
        removed
    }

    pub(crate) fn clear_activity_spans_by_kind(&mut self, kind: ActivityKind) {
        let before = self.activity_spans.len();
        self.activity_spans
            .retain(|_, span| span.kind == ActivityKind::Resting || span.kind != kind);
        if self.activity_spans.len() != before {
            self.last_activity = Instant::now();
            self.refresh_visual_mood();
        }
    }

    pub(crate) fn clear_session_activities(&mut self, session_id: &str) {
        let before = self.activity_spans.len();
        self.activity_spans
            .retain(|_, span| span.kind == ActivityKind::Resting || span.session_id != session_id);
        if self.activity_spans.len() != before {
            self.last_activity = Instant::now();
            self.refresh_visual_mood();
        }
    }

    pub(crate) fn start_permission_activity(&mut self, id: u64, session_id: &str, mood: PetMood) {
        self.begin_activity_span(ActivitySpan::new(
            permission_activity_key(id),
            session_id,
            ActivityKind::Permission,
            mood,
            true,
        ));
    }

    pub(crate) fn finish_permission_activity(&mut self, id: u64) {
        self.end_activity_span(&permission_activity_key(id));
    }

    pub(crate) fn start_choice_activity(&mut self, id: u64, session_id: &str, mood: PetMood) {
        self.begin_activity_span(ActivitySpan::new(
            choice_activity_key(id),
            session_id,
            ActivityKind::Choice,
            mood,
            false,
        ));
    }

    pub(crate) fn finish_choice_activity(&mut self, id: u64) {
        self.end_activity_span(&choice_activity_key(id));
    }

    pub(crate) fn current_pending_interaction(&self) -> Option<PendingInteractionKind> {
        match (
            self.pending_permissions.front(),
            self.pending_choices.front(),
        ) {
            (Some(permission), Some(choice)) => {
                if permission.interaction_sequence <= choice.interaction_sequence {
                    Some(PendingInteractionKind::Permission)
                } else {
                    Some(PendingInteractionKind::Choice)
                }
            }
            (Some(_), None) => Some(PendingInteractionKind::Permission),
            (None, Some(_)) => Some(PendingInteractionKind::Choice),
            (None, None) => None,
        }
    }

    pub(crate) fn current_pending_permission(&self) -> Option<&PendingPermission> {
        matches!(
            self.current_pending_interaction(),
            Some(PendingInteractionKind::Permission)
        )
        .then(|| self.pending_permissions.front())
        .flatten()
    }

    pub(crate) fn current_pending_choice(&self) -> Option<&PendingChoice> {
        matches!(
            self.current_pending_interaction(),
            Some(PendingInteractionKind::Choice)
        )
        .then(|| self.pending_choices.front())
        .flatten()
    }

    pub(crate) fn current_pending_choice_mut(&mut self) -> Option<&mut PendingChoice> {
        if !matches!(
            self.current_pending_interaction(),
            Some(PendingInteractionKind::Choice)
        ) {
            return None;
        }
        self.pending_choices.front_mut()
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
        self.begin_activity_span(ActivitySpan::new(
            tool_activity_key(&key),
            tool_session_from_key(&key),
            ActivityKind::Tool,
            mood,
            false,
        ));
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
                self.end_activity_span(&tool_activity_key(&key));
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
        self.end_activity_span(&tool_activity_key(&key));
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
        self.clear_activity_spans_by_kind(ActivityKind::Tool);
        self.refresh_visual_mood();
    }

    pub(crate) fn finish_session_tools(&mut self, session_id: &str) {
        let keys = self
            .active_tool_keys
            .keys()
            .filter(|key| tool_session_from_key(key) == session_id)
            .cloned()
            .collect::<Vec<_>>();
        if keys.is_empty() {
            return;
        }

        for key in keys {
            if let Some(tool) = self.active_tool_keys.remove(&key) {
                self.remove_tool_name_key(&tool.tool_name, &key);
                self.active_tools = self.active_tools.saturating_sub(1);
                self.decrement_active_tool_mood(tool.mood);
            }
        }
        self.clear_session_activities(session_id);
        self.last_activity = Instant::now();
        self.refresh_visual_mood();
    }

    pub(crate) fn clear_activity(&mut self) {
        self.finish_all_tools();
        self.active_subagents = 0;
        self.activity_spans.remove(SUBAGENT_ACTIVITY_KEY);
        self.refresh_visual_mood();
    }

    pub(crate) fn activity_mood(&self) -> Option<PetMood> {
        self.best_active_work_span()
            .map(|span| span.mood)
            .or_else(|| self.legacy_activity_mood())
    }

    pub(crate) fn start_subagent(&mut self) {
        self.active_subagents = self.active_subagents.saturating_add(1);
        self.sync_subagent_span();
        self.last_activity = Instant::now();
        self.refresh_visual_mood();
    }

    pub(crate) fn finish_subagent(&mut self) {
        self.active_subagents = self.active_subagents.saturating_sub(1);
        self.sync_subagent_span();
        self.last_activity = Instant::now();
        self.refresh_visual_mood();
    }

    pub(crate) fn refresh_visual_mood(&mut self) {
        self.refresh_visual_mood_at(Instant::now());
    }

    pub(crate) fn refresh_visual_mood_at(&mut self, now: Instant) {
        self.prune_expired_activity_spans(now);
        let target = self.projected_activity();
        if self.can_switch_visual_to(target, now) {
            self.transition_visual_mood(target.mood, now);
        }
    }

    fn projected_activity(&self) -> ActivityProjection {
        if let Some(span) = self.best_span(|_| true) {
            return ActivityProjection {
                mood: span.mood,
                interrupts_visual: span.interrupts_visual,
            };
        }
        let resting = self
            .activity_spans
            .get(RESTING_ACTIVITY_KEY)
            .filter(|span| span.kind == ActivityKind::Resting);
        ActivityProjection {
            mood: resting.map(|span| span.mood).unwrap_or(self.resting_mood),
            interrupts_visual: resting
                .map(|span| span.interrupts_visual)
                .unwrap_or(self.resting_interrupts_visual),
        }
    }

    fn can_switch_visual_to(&self, target: ActivityProjection, now: Instant) -> bool {
        if target.mood == self.mood {
            return true;
        }
        if matches!(target.mood, PetMood::Error) {
            return true;
        }
        if target.interrupts_visual {
            return true;
        }
        if target.mood.priority() > self.mood.priority() {
            return true;
        }
        now.duration_since(self.mood_started_at) >= self.current_mood_min_visible()
    }

    fn transition_visual_mood(&mut self, mood: PetMood, now: Instant) {
        if !matches!(
            mood,
            PetMood::Idle | PetMood::Sleeping | PetMood::Pomodoro | PetMood::Fishing
        ) {
            self.last_non_idle_mood = mood;
        }
        if request_renderer_mood(mood) {
            if self.mood != mood {
                self.mood_started_at = now;
            }
            self.mood = mood;
            if mood == self.resting_mood {
                self.resting_interrupts_visual = false;
                if let Some(resting) = self.activity_spans.get_mut(RESTING_ACTIVITY_KEY) {
                    resting.interrupts_visual = false;
                }
            }
        }
    }

    fn best_active_work_span(&self) -> Option<&ActivitySpan> {
        self.best_span(|span| {
            matches!(span.kind, ActivityKind::Tool | ActivityKind::Subagent)
                && span.mood.is_active_work()
        })
    }

    fn best_span(&self, accept: impl Fn(&ActivitySpan) -> bool) -> Option<&ActivitySpan> {
        self.activity_spans
            .values()
            .filter(|span| accept(span))
            .max_by(|a, b| {
                a.priority
                    .cmp(&b.priority)
                    .then_with(|| a.started_at.cmp(&b.started_at))
            })
    }

    fn legacy_activity_mood(&self) -> Option<PetMood> {
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

    fn current_mood_min_visible(&self) -> Duration {
        self.activity_spans
            .values()
            .filter(|span| span.mood == self.mood)
            .map(|span| span.min_visible)
            .max()
            .unwrap_or_else(|| min_visible_for(self.mood))
    }

    fn prune_expired_activity_spans(&mut self, now: Instant) {
        self.activity_spans
            .retain(|_, span| span.kind == ActivityKind::Resting || !span.is_expired_at(now));
    }

    fn sync_subagent_span(&mut self) {
        if self.active_subagents > 0 {
            self.activity_spans.insert(
                SUBAGENT_ACTIVITY_KEY.to_string(),
                ActivitySpan::new(
                    SUBAGENT_ACTIVITY_KEY,
                    "",
                    ActivityKind::Subagent,
                    PetMood::Subagent,
                    false,
                ),
            );
        } else {
            self.activity_spans.remove(SUBAGENT_ACTIVITY_KEY);
        }
    }

    fn sync_pomodoro_span(&mut self) {
        if self.is_focus_pomodoro_running() {
            self.activity_spans.insert(
                POMODORO_ACTIVITY_KEY.to_string(),
                ActivitySpan::new(
                    POMODORO_ACTIVITY_KEY,
                    "",
                    ActivityKind::Pomodoro,
                    PetMood::Pomodoro,
                    false,
                ),
            );
        } else {
            self.activity_spans.remove(POMODORO_ACTIVITY_KEY);
        }
    }

    fn sync_fishing_span(&mut self) {
        if self.fishing.is_active() {
            let mood = fishing_mood_for_phase(self.fishing.phase);
            self.activity_spans.insert(
                FISHING_ACTIVITY_KEY.to_string(),
                ActivitySpan::new(FISHING_ACTIVITY_KEY, "", ActivityKind::Fishing, mood, true)
                    .with_priority(PetMood::FishingReel.priority()),
            );
        } else {
            self.activity_spans.remove(FISHING_ACTIVITY_KEY);
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
        self.sync_pomodoro_span();
        self.set_mood(PetMood::Pomodoro);
    }

    pub(crate) fn stop_pomodoro(&mut self) {
        self.pomodoro.stop(&self.settings.pomodoro);
        self.sync_pomodoro_span();
        self.set_mood(self.default_resting_mood());
    }

    pub(crate) fn pause_pomodoro(&mut self) {
        if self.pomodoro.status != PomodoroStatus::Running {
            return;
        }
        self.pomodoro.pause(&self.settings.pomodoro);
        self.sync_pomodoro_span();
        self.set_mood(self.default_resting_mood());
    }

    pub(crate) fn resume_pomodoro(&mut self) {
        if self.pomodoro.status != PomodoroStatus::Paused {
            return;
        }
        self.pomodoro.resume(&self.settings.pomodoro);
        self.sync_pomodoro_span();
        self.set_mood(self.default_resting_mood());
    }

    pub(crate) fn start_fishing(&mut self) {
        if !self.pending_permissions.is_empty() || !self.pending_choices.is_empty() {
            return;
        }
        self.fishing.start();
        self.sync_fishing_span();
        self.set_mood(PetMood::Fishing);
    }

    pub(crate) fn stop_fishing(&mut self) {
        self.fishing.stop();
        self.sync_fishing_span();
        self.set_mood(self.default_resting_mood());
    }

    pub(crate) fn tick_fishing(&mut self) {
        let event = self.fishing.tick();
        match event {
            FishingTick::Bite => {
                self.sync_fishing_span();
                self.set_mood(PetMood::FishingReel);
            }
            FishingTick::Caught => {
                self.sync_fishing_span();
                self.set_mood(PetMood::FishingCaught);
            }
            FishingTick::Missed => {
                self.sync_fishing_span();
                self.set_mood(PetMood::FishingMissed);
            }
            FishingTick::Finished => {
                self.sync_fishing_span();
                self.set_mood(self.default_resting_mood());
            }
            FishingTick::None => {
                self.sync_fishing_span();
            }
        }
    }

    pub(crate) fn handle_fishing_input(&mut self) -> bool {
        let handled = self.fishing.input();
        if handled {
            self.last_activity = Instant::now();
            self.sync_fishing_span();
            self.refresh_visual_mood();
        }
        handled
    }

    pub(crate) fn interact_with_pet(&mut self) {
        if self.handle_fishing_input() {
            return;
        }
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
        self.begin_activity_span(
            ActivitySpan::new(
                INTERACTION_ACTIVITY_KEY,
                "",
                ActivityKind::Interaction,
                mood,
                true,
            )
            .with_priority(PetMood::Pomodoro.priority().saturating_add(1)),
        );
        self.set_resting_mood(mood, true);
    }

    pub(crate) fn skip_pomodoro(&mut self) {
        match self.pomodoro.skip(&self.settings.pomodoro) {
            PomodoroTick::FocusComplete => {
                self.record_completed_focus();
                self.sync_pomodoro_span();
                self.set_mood(PetMood::Happy);
            }
            PomodoroTick::BreakComplete => {
                self.sync_pomodoro_span();
                self.set_mood(PetMood::Idle);
            }
            PomodoroTick::None => {}
        }
    }

    pub(crate) fn tick_pomodoro(&mut self) {
        match self.pomodoro.tick(&self.settings.pomodoro) {
            PomodoroTick::FocusComplete => {
                self.record_completed_focus();
                self.sync_pomodoro_span();
                self.set_mood(PetMood::Happy);
            }
            PomodoroTick::BreakComplete => {
                self.sync_pomodoro_span();
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
            && !self.fishing.is_active()
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
            || self.fishing.is_active()
    }

    fn default_resting_mood(&self) -> PetMood {
        if self.fishing.is_active() {
            fishing_mood_for_phase(self.fishing.phase)
        } else if self.is_focus_pomodoro_running() {
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

    pub(crate) fn flush_stats_if_due(&mut self) -> Result<(), String> {
        if !self.stats.is_dirty() || self.stats_last_flush.elapsed() < STATS_FLUSH_INTERVAL {
            return Ok(());
        }
        self.flush_stats_now()
    }

    pub(crate) fn flush_stats_now(&mut self) -> Result<(), String> {
        let result = self.stats.flush();
        self.stats_last_flush = Instant::now();
        result
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) enum ActivityKind {
    Resting,
    Tool,
    Subagent,
    Permission,
    Choice,
    Pomodoro,
    Interaction,
    Fishing,
}

#[derive(Clone, Debug)]
pub(crate) struct ActivitySpan {
    // Source-level activity; rendering projects all live spans to one PetMood.
    pub(crate) key: String,
    pub(crate) session_id: String,
    pub(crate) kind: ActivityKind,
    pub(crate) mood: PetMood,
    pub(crate) priority: u8,
    pub(crate) started_at: Instant,
    pub(crate) min_visible: Duration,
    pub(crate) expires_at: Option<Instant>,
    pub(crate) interrupts_visual: bool,
}

impl ActivitySpan {
    pub(crate) fn new(
        key: impl Into<String>,
        session_id: impl Into<String>,
        kind: ActivityKind,
        mood: PetMood,
        interrupts_visual: bool,
    ) -> Self {
        Self::at(
            key,
            session_id,
            kind,
            mood,
            mood.priority(),
            interrupts_visual,
            Instant::now(),
        )
    }

    fn at(
        key: impl Into<String>,
        session_id: impl Into<String>,
        kind: ActivityKind,
        mood: PetMood,
        priority: u8,
        interrupts_visual: bool,
        now: Instant,
    ) -> Self {
        Self {
            key: key.into(),
            session_id: session_id.into(),
            kind,
            mood,
            priority,
            started_at: now,
            min_visible: min_visible_for(mood),
            expires_at: None,
            interrupts_visual,
        }
    }

    fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }

    fn is_expired_at(&self, now: Instant) -> bool {
        self.expires_at.is_some_and(|expires_at| now >= expires_at)
    }
}

#[derive(Clone, Copy)]
struct ActivityProjection {
    mood: PetMood,
    interrupts_visual: bool,
}

fn min_visible_for(mood: PetMood) -> Duration {
    match mood {
        PetMood::Typing | PetMood::Building => WORK_MIN_VISIBLE,
        PetMood::Thinking | PetMood::Search => THINKING_MIN_VISIBLE,
        PetMood::Subagent => SUBAGENT_MIN_VISIBLE,
        PetMood::Pomodoro
        | PetMood::Wave
        | PetMood::Stretch
        | PetMood::Fishing
        | PetMood::FishingReel
        | PetMood::FishingCaught
        | PetMood::FishingMissed => INTERACTION_MIN_VISIBLE,
        PetMood::Happy => HAPPY_MIN_VISIBLE,
        _ => Duration::ZERO,
    }
}

fn fishing_mood_for_phase(phase: FishingPhase) -> PetMood {
    match phase {
        FishingPhase::Inactive => PetMood::Idle,
        FishingPhase::Waiting => PetMood::Fishing,
        FishingPhase::Reeling => PetMood::FishingReel,
        FishingPhase::Caught => PetMood::FishingCaught,
        FishingPhase::Missed => PetMood::FishingMissed,
    }
}

fn normalize_tool_name_key(tool_name: &str) -> String {
    tool_name.trim().to_ascii_lowercase()
}

fn tool_activity_key(key: &str) -> String {
    format!("tool:{key}")
}

fn permission_activity_key(id: u64) -> String {
    format!("permission:{id}")
}

fn choice_activity_key(id: u64) -> String {
    format!("choice:{id}")
}

fn tool_session_from_key(key: &str) -> String {
    key.split(':').next().unwrap_or_default().to_string()
}

fn request_renderer_mood(mood: PetMood) -> bool {
    match PET_RENDERER.get() {
        Some(renderer) => renderer
            .lock()
            .expect("pet renderer poisoned")
            .request_mood(mood),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_spans_project_active_work_over_resting_state() {
        let mut state = AppState::new();

        state.set_resting_mood(PetMood::Happy, false);
        state.start_tool_mood(PetMood::Building);

        assert_eq!(state.activity_mood(), Some(PetMood::Building));
        assert_eq!(state.mood, PetMood::Building);
        assert!(
            state
                .activity_spans
                .values()
                .any(|span| span.kind == ActivityKind::Tool && span.mood == PetMood::Building)
        );
    }

    #[test]
    fn clearing_session_activities_removes_only_matching_spans() {
        let mut state = AppState::new();

        state.begin_activity_span(ActivitySpan::new(
            "fish:s1",
            "s1",
            ActivityKind::Fishing,
            PetMood::Happy,
            false,
        ));
        state.begin_activity_span(ActivitySpan::new(
            "choice:s2",
            "s2",
            ActivityKind::Choice,
            PetMood::Thinking,
            false,
        ));

        state.clear_session_activities("s1");

        assert!(!state.activity_spans.contains_key("fish:s1"));
        assert!(state.activity_spans.contains_key("choice:s2"));
        assert!(state.activity_spans.contains_key(RESTING_ACTIVITY_KEY));
    }

    #[test]
    fn finish_session_tools_preserves_other_sessions() {
        let mut state = AppState::new();
        state.start_tool_activity(
            "s1:id:write-1".to_string(),
            "Write".to_string(),
            PetMood::Typing,
        );
        state.start_tool_activity(
            "s2:id:bash-1".to_string(),
            "Bash".to_string(),
            PetMood::Building,
        );

        state.finish_session_tools("s1");

        assert_eq!(state.active_tools, 1);
        assert!(state.active_tool_keys.contains_key("s2:id:bash-1"));
        assert!(!state.active_tool_keys.contains_key("s1:id:write-1"));
        assert_eq!(state.activity_mood(), Some(PetMood::Building));
    }

    #[test]
    fn current_pending_interaction_uses_arrival_order_across_types() {
        let mut state = AppState::new();

        state.pending_permissions.push_back(PendingPermission {
            id: 1,
            interaction_sequence: 1,
            session_id: "s1".to_string(),
            tool_name: "Edit".to_string(),
            summary: String::new(),
            cwd: String::new(),
            suggestions: Vec::new(),
            waiter: Arc::new(PermissionWaiter {
                decision: Mutex::new(None),
                ready: Condvar::new(),
            }),
        });
        state.pending_choices.push_back(PendingChoice {
            id: 1,
            interaction_sequence: 2,
            session_id: "s1".to_string(),
            kind: ChoiceKind::AskUserQuestion,
            title: String::new(),
            detail: String::new(),
            questions: Vec::new(),
            selected: Vec::new(),
            other_text: Vec::new(),
            tool_input: Value::Null,
            waiter: Arc::new(ChoiceWaiter {
                decision: Mutex::new(None),
                ready: Condvar::new(),
            }),
        });

        assert_eq!(
            state.current_pending_interaction(),
            Some(PendingInteractionKind::Permission)
        );
        assert!(state.current_pending_permission().is_some());
        assert!(state.current_pending_choice().is_none());
    }

    #[test]
    fn resting_error_overrides_lower_priority_activity_span() {
        let mut state = AppState::new();

        state.start_pomodoro();
        assert_eq!(state.mood, PetMood::Pomodoro);

        state.set_resting_mood(PetMood::Error, true);

        assert_eq!(state.resting_mood, PetMood::Error);
        assert_eq!(state.mood, PetMood::Error);
    }

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

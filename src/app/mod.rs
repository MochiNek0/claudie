pub(crate) mod fishing;
pub(crate) mod pomodoro;
pub(crate) mod stats;

use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::path::Path;
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
const ENDED_SESSION_RETENTION: Duration = Duration::from_secs(30 * 60);
const MAX_ENDED_SESSIONS: usize = 50;
const STATS_FLUSH_INTERVAL: Duration = Duration::from_secs(3);
// How often the UI tick re-scans a working session's transcript tail for a
// terminal interrupt (ESC) marker. ESC fires no hook, so polling is the only
// signal that the turn ended.
const INTERRUPT_POLL_INTERVAL: Duration = Duration::from_millis(750);
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
    Deny,
    Shrug,
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
    /// Every mood variant, in declaration order. Keep in sync with the enum so
    /// exhaustive consumers (GIF filename map, settings GIF list) can be checked
    /// for completeness.
    pub(crate) const ALL: [PetMood; 18] = [
        Self::Idle,
        Self::Thinking,
        Self::Typing,
        Self::Building,
        Self::Search,
        Self::Happy,
        Self::Error,
        Self::Deny,
        Self::Shrug,
        Self::Sleeping,
        Self::Subagent,
        Self::Pomodoro,
        Self::Wave,
        Self::Stretch,
        Self::Fishing,
        Self::FishingReel,
        Self::FishingCaught,
        Self::FishingMissed,
    ];

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Thinking => "thinking",
            Self::Typing => "typing",
            Self::Building => "building",
            Self::Search => "search",
            Self::Happy => "happy",
            Self::Error => "error",
            Self::Deny => "deny",
            Self::Shrug => "shrug",
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
            Self::Error | Self::Deny => 90,
            Self::Building | Self::Typing => 80,
            Self::Search => 70,
            Self::Subagent => 65,
            Self::Thinking => 60,
            // Transient "oops, recovered" reaction: low enough that the next
            // tool's working mood immediately overrides it.
            Self::Shrug => 50,
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

// AskUserQuestion arrives as a blocking PermissionRequest hook; its parsed
// questions become a PendingChoice so the popup offers the real options, and
// submitting answers via a PermissionRequest `updatedInput` decision.
// ExitPlanMode stays on the plain permission popup (approve/deny maps 1:1),
// so the ExitPlanMode kind is currently only produced in tests.
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
    pub(crate) tool_use_id: String,
    // Fallback identity for matching a follow-up state event back to this
    // permission when Claude Code's PermissionRequest payload carried no
    // tool_use_id: a normalized fingerprint of the tool input.
    pub(crate) tool_input_fingerprint: String,
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

#[derive(Clone, Copy, Debug)]
struct PendingInteractionTarget {
    kind: PendingInteractionKind,
    id: u64,
    sequence: u64,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct OfficialUsageWindow {
    pub(crate) used_percentage: Option<u8>,
    pub(crate) reset_at_unix_ms: Option<u64>,
    pub(crate) reset_label: String,
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
    pub(crate) official_plan: String,
    pub(crate) official_five_hour: OfficialUsageWindow,
    pub(crate) official_seven_day: OfficialUsageWindow,
    pub(crate) official_usage_updated_at_unix_ms: Option<u64>,
    pub(crate) official_usage_error: String,
    pub(crate) transcript_path: String,
}

/// Latest GitHub release info captured by the version checker. Both fields empty
/// means no newer release was found (or the check has not run yet).
#[derive(Clone, Default)]
pub(crate) struct UpdateInfo {
    pub(crate) latest_version: String,
    pub(crate) release_url: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ActiveTool {
    pub(crate) tool_name: String,
    pub(crate) mood: PetMood,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClaudeSessionStatus {
    Idle,
    Streaming,
    Tool,
    WaitingPermission,
    WaitingChoice,
    Compacting,
    Error,
    Denied,
    Done,
    Ended,
}

impl ClaudeSessionStatus {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Streaming => "Streaming",
            Self::Tool => "Tool",
            Self::WaitingPermission => "Permission",
            Self::WaitingChoice => "Choice",
            Self::Compacting => "Compacting",
            Self::Error => "Error",
            Self::Denied => "Denied",
            Self::Done => "Done",
            Self::Ended => "Ended",
        }
    }

    fn mood(self) -> PetMood {
        match self {
            Self::Idle | Self::Done | Self::Ended => PetMood::Idle,
            Self::Streaming | Self::Compacting => PetMood::Thinking,
            Self::Tool => PetMood::Thinking,
            Self::WaitingPermission | Self::WaitingChoice => PetMood::Thinking,
            Self::Error => PetMood::Error,
            Self::Denied => PetMood::Deny,
        }
    }

    fn is_live(self) -> bool {
        !matches!(self, Self::Ended)
    }

    /// A session the user is actively working through: streaming, running a
    /// tool, compacting, or waiting on the user. Focus stays put while the
    /// focused session is busy; once it finishes its turn (or errors/ends),
    /// focus is free to hand off to a session waiting on a request.
    fn is_busy(self) -> bool {
        matches!(
            self,
            Self::Streaming
                | Self::Tool
                | Self::Compacting
                | Self::WaitingPermission
                | Self::WaitingChoice
        )
    }

    /// Actively producing output (as opposed to merely waiting on the user).
    /// Drives auto-follow: an idle focused session yields to whichever session
    /// is currently working. Waiting states are handled separately via the
    /// pending-request hand-off so their popup can surface.
    fn is_working(self) -> bool {
        matches!(self, Self::Streaming | Self::Tool | Self::Compacting)
    }

    fn visual_projection(self) -> Option<ActivityProjection> {
        let interrupts_visual = matches!(
            self,
            Self::Streaming
                | Self::WaitingPermission
                | Self::WaitingChoice
                | Self::Compacting
                | Self::Error
                | Self::Denied
        );
        interrupts_visual.then_some(ActivityProjection {
            mood: self.mood(),
            interrupts_visual,
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ClaudeSession {
    pub(crate) id: String,
    pub(crate) cwd: String,
    pub(crate) status: ClaudeSessionStatus,
    pub(crate) detail: String,
    pub(crate) order: u64,
    pub(crate) last_seen: Instant,
    waiting_interaction_sequence: Option<u64>,
    // Latest transcript path seen for this session and the file length at the
    // current turn's start. A terminal-interrupt marker appearing past the
    // baseline means the turn was ESC-cancelled (no hook fires for that).
    transcript_path: String,
    turn_baseline_len: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionSwitcherItem {
    pub(crate) id: String,
    pub(crate) display_name: String,
    pub(crate) status: ClaudeSessionStatus,
    pub(crate) detail: String,
    pub(crate) focused: bool,
    pub(crate) pending_count: usize,
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
    pub(crate) active_subagent_sessions: HashMap<String, usize>,
    pub(crate) activity_spans: HashMap<String, ActivitySpan>,
    pub(crate) sessions: HashMap<String, ClaudeSession>,
    pub(crate) focused_session_id: Option<String>,
    // Set when the user picks a session in the switcher: focus stays pinned to
    // it (auto-follow suppressed) until that session ends, then releases.
    focus_pinned: bool,
    next_session_order: u64,
    pub(crate) pending_permissions: VecDeque<PendingPermission>,
    pub(crate) pending_choices: VecDeque<PendingChoice>,
    pub(crate) quota: QuotaStats,
    pub(crate) update: UpdateInfo,
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
    last_interrupt_poll: Instant,
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
            active_subagent_sessions: HashMap::new(),
            activity_spans: HashMap::new(),
            sessions: HashMap::new(),
            focused_session_id: None,
            focus_pinned: false,
            next_session_order: 1,
            pending_permissions: VecDeque::new(),
            pending_choices: VecDeque::new(),
            quota: QuotaStats::default(),
            update: UpdateInfo::default(),
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
            last_interrupt_poll: now,
        };
        state.set_resting_activity_at(PetMood::Idle, false, now);
        state
    }

    pub(crate) fn set_mood(&mut self, mood: PetMood) {
        let now = Instant::now();
        self.last_activity = now;
        self.set_resting_activity_at(mood, matches!(mood, PetMood::Error | PetMood::Deny), now);
        if matches!(mood, PetMood::Error | PetMood::Deny) {
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

    pub(crate) fn clear_session_activities(&mut self, session_id: &str) {
        let before = self.activity_spans.len();
        self.activity_spans
            .retain(|_, span| span.kind == ActivityKind::Resting || span.session_id != session_id);
        if self.activity_spans.len() != before {
            self.last_activity = Instant::now();
            self.refresh_visual_mood();
        }
    }

    fn clear_session_activity_kind(&mut self, session_id: &str, kind: ActivityKind) -> bool {
        let before = self.activity_spans.len();
        self.activity_spans.retain(|_, span| {
            span.kind == ActivityKind::Resting || span.session_id != session_id || span.kind != kind
        });
        self.activity_spans.len() != before
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

    pub(crate) fn note_session_event(
        &mut self,
        session_id: &str,
        cwd: &str,
        event: &str,
        tool_name: &str,
        tool_mood: PetMood,
    ) {
        match session_status_for_event(event, tool_name, tool_mood) {
            Some((status, detail)) => {
                self.set_session_status(session_id, cwd, status, detail, None)
            }
            None => self.touch_session(session_id, cwd),
        }
        if matches!(event, "SessionStart" | "SessionResume") {
            self.acquire_focus_for_new_session(session_id);
        }
    }

    pub(crate) fn mark_session_waiting_permission(
        &mut self,
        session_id: &str,
        cwd: &str,
        tool_name: &str,
        interaction_sequence: u64,
    ) {
        let detail = if tool_name.trim().is_empty() {
            "Waiting for permission".to_string()
        } else {
            format!("{} wants access", tool_name.trim())
        };
        self.set_session_status(
            session_id,
            cwd,
            ClaudeSessionStatus::WaitingPermission,
            detail,
            Some(interaction_sequence),
        );
    }

    pub(crate) fn mark_session_waiting_choice(
        &mut self,
        session_id: &str,
        detail: &str,
        interaction_sequence: u64,
    ) {
        let detail = if detail.trim().is_empty() {
            "Waiting for input".to_string()
        } else {
            detail.trim().to_string()
        };
        self.set_session_status(
            session_id,
            "",
            ClaudeSessionStatus::WaitingChoice,
            detail,
            Some(interaction_sequence),
        );
    }

    pub(crate) fn mark_session_interaction_finished(
        &mut self,
        session_id: &str,
        interaction_sequence: u64,
        status: ClaudeSessionStatus,
        detail: impl Into<String>,
    ) {
        let session_id = normalize_session_id(session_id);
        let is_current_wait = self.sessions.get(&session_id).is_some_and(|session| {
            session.waiting_interaction_sequence == Some(interaction_sequence)
        });
        if !is_current_wait {
            return;
        }
        self.set_session_status(&session_id, "", status, detail.into(), None);
    }

    /// Record a session's transcript path and, at a turn boundary (or the first
    /// time we see the file), baseline its length so only markers written
    /// during the current turn count as a fresh interrupt.
    pub(crate) fn note_session_transcript(
        &mut self,
        session_id: &str,
        path: &str,
        len: u64,
        is_turn_start: bool,
    ) {
        let session_id = normalize_session_id(session_id);
        if let Some(session) = self.sessions.get_mut(&session_id) {
            let first_capture = session.transcript_path.is_empty();
            session.transcript_path = path.to_string();
            if first_capture || is_turn_start {
                session.turn_baseline_len = len;
            }
        }
    }

    /// Throttle gate for transcript interrupt polling; returns true at most once
    /// per `INTERRUPT_POLL_INTERVAL`.
    pub(crate) fn interrupt_poll_due(&mut self, now: Instant) -> bool {
        if now.saturating_duration_since(self.last_interrupt_poll) < INTERRUPT_POLL_INTERVAL {
            return false;
        }
        self.last_interrupt_poll = now;
        true
    }

    /// Snapshot of `(session_id, transcript_path, turn_baseline_len)` for every
    /// session we still believe is actively working and has a transcript to scan.
    pub(crate) fn working_sessions_for_interrupt_poll(&self) -> Vec<(String, String, u64)> {
        self.sessions
            .values()
            .filter(|session| session.status.is_working() && !session.transcript_path.is_empty())
            .map(|session| {
                (
                    session.id.clone(),
                    session.transcript_path.clone(),
                    session.turn_baseline_len,
                )
            })
            .collect()
    }

    /// Apply a detected terminal interrupt (ESC): release the turn's work, drop
    /// the now-stale "thinking" tint if it was on screen, advance the scan
    /// baseline so the same marker isn't re-detected, and mark the turn done.
    pub(crate) fn mark_session_interrupted(&mut self, session_id: &str, new_baseline: u64) {
        let session_id = normalize_session_id(session_id);
        let still_working = self
            .sessions
            .get(&session_id)
            .is_some_and(|session| session.status.is_working());
        if !still_working {
            return;
        }
        let was_focused = self.focused_session_id.as_deref() == Some(session_id.as_str());
        self.finish_session_work(&session_id);
        if was_focused {
            let resting = self.default_resting_mood();
            self.set_resting_mood(resting, true);
        }
        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.turn_baseline_len = new_baseline;
        }
        self.set_session_status(
            &session_id,
            "",
            ClaudeSessionStatus::Done,
            "Ready".to_string(),
            None,
        );
    }

    pub(crate) fn focus_session(&mut self, session_id: &str) {
        let session_id = normalize_session_id(session_id);
        if !self.sessions.contains_key(&session_id) {
            return;
        }
        self.focused_session_id = Some(session_id);
        // A deliberate switcher pick pins focus until that session ends.
        self.focus_pinned = true;
        self.refresh_visual_mood();
    }

    /// A freshly started/resumed session takes focus when the user is not
    /// actively working through another conversation and hasn't pinned one.
    /// This is the "opening a new window focuses it" behavior.
    fn acquire_focus_for_new_session(&mut self, session_id: &str) {
        let session_id = normalize_session_id(session_id);
        if self.focus_pinned || !self.session_is_live(&session_id) {
            return;
        }
        let focus_busy = self
            .focused_session_id
            .as_deref()
            .and_then(|id| self.sessions.get(id))
            .is_some_and(|session| session.status.is_busy());
        if focus_busy {
            return;
        }
        if self.focused_session_id.as_deref() != Some(session_id.as_str()) {
            self.focused_session_id = Some(session_id);
            self.refresh_visual_mood();
        }
    }

    pub(crate) fn session_switcher_items(&self) -> Vec<SessionSwitcherItem> {
        let focused = self.focused_session_id.as_deref();
        let mut sessions = self
            .sessions
            .values()
            .filter(|session| session.status.is_live())
            .collect::<Vec<_>>();
        sessions.sort_by(|a, b| {
            let a_focused = focused.is_some_and(|focused| focused == a.id);
            let b_focused = focused.is_some_and(|focused| focused == b.id);
            b_focused
                .cmp(&a_focused)
                .then_with(|| {
                    self.pending_count_for_session(&b.id)
                        .cmp(&self.pending_count_for_session(&a.id))
                })
                .then_with(|| b.last_seen.cmp(&a.last_seen))
                .then_with(|| a.order.cmp(&b.order))
        });
        sessions
            .into_iter()
            .map(|session| SessionSwitcherItem {
                id: session.id.clone(),
                display_name: session_display_name(session),
                status: session.status,
                detail: session.detail.clone(),
                focused: self
                    .focused_session_id
                    .as_deref()
                    .is_some_and(|focused| focused == session.id),
                pending_count: self.pending_count_for_session(&session.id),
            })
            .collect()
    }

    fn set_session_status(
        &mut self,
        session_id: &str,
        cwd: &str,
        status: ClaudeSessionStatus,
        detail: String,
        waiting_interaction_sequence: Option<u64>,
    ) {
        let session_id = normalize_session_id(session_id);
        let now = Instant::now();
        self.ensure_session_entry(&session_id, now);

        if let Some(session) = self.sessions.get_mut(&session_id) {
            if !cwd.trim().is_empty() {
                session.cwd = cwd.trim().to_string();
            }
            session.status = status;
            session.detail = detail;
            session.last_seen = now;
            session.waiting_interaction_sequence = waiting_interaction_sequence;
        }
        self.prune_stale_sessions(now);
        self.reconcile_focused_session(Some(&session_id));
        self.refresh_visual_mood();
    }

    fn touch_session(&mut self, session_id: &str, cwd: &str) {
        let session_id = normalize_session_id(session_id);
        let now = Instant::now();
        self.ensure_session_entry(&session_id, now);

        if let Some(session) = self.sessions.get_mut(&session_id) {
            if !cwd.trim().is_empty() {
                session.cwd = cwd.trim().to_string();
            }
            session.last_seen = now;
        }
        self.prune_stale_sessions(now);
        self.reconcile_focused_session(Some(&session_id));
        self.refresh_visual_mood();
    }

    fn ensure_session_entry(&mut self, session_id: &str, now: Instant) {
        if self.sessions.contains_key(session_id) {
            return;
        }
        let order = self.next_session_order;
        self.next_session_order = self.next_session_order.saturating_add(1);
        self.sessions.insert(
            session_id.to_string(),
            ClaudeSession {
                id: session_id.to_string(),
                cwd: String::new(),
                status: ClaudeSessionStatus::Idle,
                detail: String::new(),
                order,
                last_seen: now,
                waiting_interaction_sequence: None,
                transcript_path: String::new(),
                turn_baseline_len: 0,
            },
        );
    }

    fn prune_stale_sessions(&mut self, now: Instant) {
        let mut remove = self
            .sessions
            .iter()
            .filter(|(_, session)| {
                session.status == ClaudeSessionStatus::Ended
                    && now.saturating_duration_since(session.last_seen) >= ENDED_SESSION_RETENTION
            })
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();

        let ended_count = self
            .sessions
            .values()
            .filter(|session| session.status == ClaudeSessionStatus::Ended)
            .count();
        let remaining_ended = ended_count.saturating_sub(remove.len());
        if remaining_ended > MAX_ENDED_SESSIONS {
            let extra = remaining_ended - MAX_ENDED_SESSIONS;
            let mut ended = self
                .sessions
                .iter()
                .filter(|(id, session)| {
                    session.status == ClaudeSessionStatus::Ended
                        && !remove.iter().any(|remove_id| remove_id == *id)
                })
                .map(|(id, session)| (id.clone(), session.last_seen, session.order))
                .collect::<Vec<_>>();
            ended.sort_by_key(|(_, last_seen, order)| (*last_seen, *order));
            remove.extend(ended.into_iter().take(extra).map(|(id, _, _)| id));
        }

        for id in remove {
            self.sessions.remove(&id);
        }
    }

    fn reconcile_focused_session(&mut self, preferred: Option<&str>) {
        // A manual pin only lasts while its session is alive; once it ends,
        // release the pin so auto-follow resumes.
        if self.focus_pinned
            && !self
                .focused_session_id
                .as_deref()
                .is_some_and(|id| self.session_is_live(id))
        {
            self.focus_pinned = false;
        }

        // A busy focused session is never preempted — keep the user's
        // attention on the conversation they are actively working through.
        let focus_busy = self
            .focused_session_id
            .as_deref()
            .and_then(|id| self.sessions.get(id))
            .is_some_and(|session| session.status.is_busy());
        if focus_busy {
            return;
        }

        // Focus is pinned to a live (but idle) session: hold it there. A
        // blocking request elsewhere still needs the user, so a waiting session
        // releases the pin and takes focus so its popup can surface.
        if self.focus_pinned {
            if let Some(waiter) = self.oldest_waiting_session() {
                self.focused_session_id = Some(waiter);
                self.focus_pinned = false;
            }
            return;
        }

        // Focus is free and unpinned. Hand off to the longest-waiting request
        // so its popup pops as soon as the previous conversation releases focus.
        if let Some(waiter) = self.oldest_waiting_session() {
            self.focused_session_id = Some(waiter);
            return;
        }

        // Auto-follow: track whichever session is actively working now.
        if let Some(working) = self.most_recently_working_session() {
            self.focused_session_id = Some(working);
            return;
        }

        // Nobody is waiting or working. Keep the current focus if it is still
        // alive to avoid flicker; otherwise fall back to the just-active
        // session, then the oldest live session.
        if self
            .focused_session_id
            .as_deref()
            .is_some_and(|id| self.session_is_live(id))
        {
            return;
        }

        if let Some(preferred) = preferred {
            if self.session_is_live(preferred) {
                self.focused_session_id = Some(preferred.to_string());
                return;
            }
        }

        self.focused_session_id = self
            .sessions
            .values()
            .filter(|session| session.status.is_live())
            .min_by_key(|session| session.order)
            .map(|session| session.id.clone());
    }

    /// The live session most recently seen in an actively-working state.
    /// Drives auto-follow once the focused session goes idle.
    fn most_recently_working_session(&self) -> Option<String> {
        self.sessions
            .values()
            .filter(|session| session.status.is_working())
            .max_by_key(|session| (session.last_seen, session.order))
            .map(|session| session.id.clone())
    }

    /// The live session with the oldest outstanding permission/choice request
    /// (by interaction sequence). Drives focus hand-off so a finished session
    /// yields to whoever has been waiting longest.
    fn oldest_waiting_session(&self) -> Option<String> {
        let permissions = self
            .pending_permissions
            .iter()
            .map(|pending| (pending.interaction_sequence, pending.session_id.as_str()));
        let choices = self
            .pending_choices
            .iter()
            .map(|pending| (pending.interaction_sequence, pending.session_id.as_str()));
        permissions
            .chain(choices)
            .filter(|&(_, session_id)| self.session_is_live(session_id))
            .min_by_key(|&(sequence, _)| sequence)
            .map(|(_, session_id)| session_id.to_string())
    }

    fn session_is_live(&self, session_id: &str) -> bool {
        self.sessions
            .get(session_id)
            .is_some_and(|session| session.status.is_live())
    }

    fn pending_count_for_session(&self, session_id: &str) -> usize {
        self.pending_permissions
            .iter()
            .filter(|pending| pending.session_id == session_id)
            .count()
            + self
                .pending_choices
                .iter()
                .filter(|pending| pending.session_id == session_id)
                .count()
    }

    #[allow(dead_code)]
    pub(crate) fn current_pending_interaction(&self) -> Option<PendingInteractionKind> {
        self.current_pending_target().map(|target| target.kind)
    }

    pub(crate) fn current_pending_permission(&self) -> Option<&PendingPermission> {
        let target = self.current_pending_target()?;
        if target.kind != PendingInteractionKind::Permission {
            return None;
        }
        self.pending_permissions
            .iter()
            .find(|pending| pending.id == target.id)
    }

    pub(crate) fn current_pending_choice(&self) -> Option<&PendingChoice> {
        let target = self.current_pending_target()?;
        if target.kind != PendingInteractionKind::Choice {
            return None;
        }
        self.pending_choices
            .iter()
            .find(|pending| pending.id == target.id)
    }

    pub(crate) fn current_pending_choice_mut(&mut self) -> Option<&mut PendingChoice> {
        let target = self.current_pending_target()?;
        if target.kind != PendingInteractionKind::Choice {
            return None;
        }
        self.pending_choices
            .iter_mut()
            .find(|pending| pending.id == target.id)
    }

    fn current_pending_target(&self) -> Option<PendingInteractionTarget> {
        // Popups are strictly focus-gated: only the focused session's requests
        // surface. Other sessions' requests wait (shown as a switcher hint)
        // until focus hands off to them, at which point their popup pops.
        let focused = self.focused_session_id.as_deref()?;
        self.pending_target_for_session(Some(focused))
    }

    fn pending_target_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Option<PendingInteractionTarget> {
        let permission = self
            .pending_permissions
            .iter()
            .filter(|pending| session_id.map_or(true, |id| pending.session_id == id))
            .map(|pending| PendingInteractionTarget {
                kind: PendingInteractionKind::Permission,
                id: pending.id,
                sequence: pending.interaction_sequence,
            })
            .min_by_key(|target| target.sequence);

        let choice = self
            .pending_choices
            .iter()
            .filter(|pending| session_id.map_or(true, |id| pending.session_id == id))
            .map(|pending| PendingInteractionTarget {
                kind: PendingInteractionKind::Choice,
                id: pending.id,
                sequence: pending.interaction_sequence,
            })
            .min_by_key(|target| target.sequence);

        match (permission, choice) {
            (Some(permission), Some(choice)) => Some(if permission.sequence <= choice.sequence {
                permission
            } else {
                choice
            }),
            (Some(permission), None) => Some(permission),
            (None, Some(choice)) => Some(choice),
            (None, None) => None,
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

    #[cfg(test)]
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
        session_id: &str,
        tool_name: &str,
        fallback_mood: PetMood,
    ) -> PetMood {
        let key = keys
            .iter()
            .find(|key| self.active_tool_keys.contains_key(*key))
            .cloned()
            .or_else(|| self.pop_named_tool_key_for_session(session_id, tool_name));

        let Some(key) = key else {
            return fallback_mood;
        };

        let Some(tool) = self.active_tool_keys.remove(&key) else {
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

    pub(crate) fn finish_session_tools(&mut self, session_id: &str) {
        let keys = self
            .active_tool_keys
            .keys()
            .filter(|key| tool_session_from_key(key) == session_id)
            .cloned()
            .collect::<Vec<_>>();

        let removed_tools = keys.len();
        for key in keys {
            if let Some(tool) = self.active_tool_keys.remove(&key) {
                self.remove_tool_name_key(&tool.tool_name, &key);
                self.active_tools = self.active_tools.saturating_sub(1);
                self.decrement_active_tool_mood(tool.mood);
            }
        }
        let changed = self.clear_session_activity_kind(session_id, ActivityKind::Tool);
        if changed || removed_tools > 0 {
            self.last_activity = Instant::now();
            self.refresh_visual_mood();
        }
    }

    pub(crate) fn finish_session_work(&mut self, session_id: &str) {
        self.finish_session_tools(session_id);
        self.finish_session_subagents(session_id);
        self.clear_session_activities(session_id);
    }

    pub(crate) fn activity_mood(&self) -> Option<PetMood> {
        self.best_active_work_span()
            .map(|span| span.mood)
            .or_else(|| {
                if self.focused_session_id.is_some() {
                    None
                } else {
                    self.legacy_activity_mood()
                }
            })
    }

    #[cfg(test)]
    pub(crate) fn start_subagent(&mut self) {
        self.start_subagent_for_session("");
    }

    pub(crate) fn start_subagent_for_session(&mut self, session_id: &str) {
        let session_id = session_id.trim().to_string();
        self.active_subagents = self.active_subagents.saturating_add(1);
        *self.active_subagent_sessions.entry(session_id).or_insert(0) += 1;
        self.sync_subagent_spans();
        self.last_activity = Instant::now();
        self.refresh_visual_mood();
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn finish_subagent(&mut self) {
        self.finish_subagent_for_session("");
    }

    pub(crate) fn finish_subagent_for_session(&mut self, session_id: &str) {
        let session_id = session_id.trim();
        let decremented = self.decrement_subagent_session(session_id)
            || (!session_id.is_empty() && self.decrement_subagent_session(""));
        if decremented {
            self.active_subagents = self.active_subagents.saturating_sub(1);
        }
        self.sync_subagent_spans();
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
        if let Some(focused) = self.focused_session_id.as_deref() {
            if let Some(span) = self.best_span(|span| self.span_visible_for_focus(span, focused)) {
                return ActivityProjection {
                    mood: span.mood,
                    interrupts_visual: span.interrupts_visual,
                };
            }
            if let Some(session_projection) = self
                .sessions
                .get(focused)
                .and_then(|session| session.status.visual_projection())
            {
                if let Some(resting) = self.resting_span() {
                    if resting.mood.priority() > session_projection.mood.priority() {
                        return ActivityProjection {
                            mood: resting.mood,
                            interrupts_visual: resting.interrupts_visual,
                        };
                    }
                }
                return session_projection;
            }
            if let Some(resting) = self.resting_span() {
                return ActivityProjection {
                    mood: resting.mood,
                    interrupts_visual: resting.interrupts_visual,
                };
            }
        }

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
        if matches!(target.mood, PetMood::Error | PetMood::Deny) {
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
                && self
                    .focused_session_id
                    .as_deref()
                    .map_or(true, |focused| self.span_visible_for_focus(span, focused))
        })
    }

    fn span_visible_for_focus(&self, span: &ActivitySpan, focused: &str) -> bool {
        span.session_id == focused
            || matches!(
                span.kind,
                ActivityKind::Subagent
                    | ActivityKind::Pomodoro
                    | ActivityKind::Interaction
                    | ActivityKind::Fishing
            )
    }

    fn resting_span(&self) -> Option<&ActivitySpan> {
        self.activity_spans
            .get(RESTING_ACTIVITY_KEY)
            .filter(|span| span.kind == ActivityKind::Resting)
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

    fn sync_subagent_spans(&mut self) {
        self.activity_spans
            .retain(|_, span| span.kind != ActivityKind::Subagent);

        for (session_id, count) in self.active_subagent_sessions.iter() {
            if *count == 0 {
                continue;
            }
            self.activity_spans.insert(
                subagent_activity_key(session_id),
                ActivitySpan::new(
                    subagent_activity_key(session_id),
                    session_id.clone(),
                    ActivityKind::Subagent,
                    PetMood::Subagent,
                    false,
                ),
            );
        }
    }

    fn finish_session_subagents(&mut self, session_id: &str) {
        let session_id = session_id.trim();
        let Some(count) = self.active_subagent_sessions.remove(session_id) else {
            return;
        };
        self.active_subagents = self.active_subagents.saturating_sub(count);
        self.sync_subagent_spans();
        self.last_activity = Instant::now();
        self.refresh_visual_mood();
    }

    fn decrement_subagent_session(&mut self, session_id: &str) -> bool {
        let Some(count) = self.active_subagent_sessions.get_mut(session_id) else {
            return false;
        };
        *count = count.saturating_sub(1);
        if *count == 0 {
            self.active_subagent_sessions.remove(session_id);
        }
        true
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

    fn pop_named_tool_key_for_session(
        &mut self,
        session_id: &str,
        tool_name: &str,
    ) -> Option<String> {
        let name_key = normalize_tool_name_key(tool_name);
        let queue = self.active_tool_names.get_mut(&name_key)?;
        let position = queue
            .iter()
            .position(|key| tool_session_from_key(key) == session_id)?;
        let key = queue.remove(position);
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
            && idle_for > self.interaction_decay_after(self.resting_mood)
        {
            let target = if !self.pending_permissions.is_empty() || !self.pending_choices.is_empty()
            {
                PetMood::Thinking
            } else {
                self.default_resting_mood()
            };
            self.set_resting_mood(target, false);
        } else if matches!(
            self.resting_mood,
            PetMood::Happy | PetMood::Error | PetMood::Deny
        ) && idle_for > Duration::from_secs(7)
        {
            self.set_resting_mood(PetMood::Idle, false);
        } else if matches!(self.resting_mood, PetMood::Shrug) && idle_for > Duration::from_secs(3) {
            self.set_resting_mood(PetMood::Idle, false);
        }
        self.refresh_visual_mood();
    }

    /// How long a one-shot interaction mood (`Wave`/`Stretch`) stays before
    /// decaying. `Stretch` lingers long enough to finish one full loop of its
    /// GIF (its motion has a clear begin/end); `Wave` stays the brief 3s
    /// acknowledgement even though its clip loops much longer.
    fn interaction_decay_after(&self, mood: PetMood) -> Duration {
        let floor = Duration::from_secs(3);
        if mood != PetMood::Stretch {
            return floor;
        }
        match renderer_clip_total_ms(mood) {
            Some(total_ms) => floor.max(Duration::from_millis(total_ms as u64)),
            None => floor,
        }
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
        let model = self.quota.last_model.clone();
        if category_delta > 0 {
            self.stats.record(|day| {
                day.input_tokens = day.input_tokens.saturating_add(input_delta);
                day.output_tokens = day.output_tokens.saturating_add(output_delta);
                day.cache_creation_tokens = day
                    .cache_creation_tokens
                    .saturating_add(cache_creation_delta);
                day.cache_read_tokens = day.cache_read_tokens.saturating_add(cache_read_delta);
                day.token_delta = day.token_delta.saturating_add(category_delta);
                day.add_model_tokens(&model, category_delta);
            });
        } else if total > self.stats_last_total_tokens {
            let total_delta = total - self.stats_last_total_tokens;
            self.stats.record(|day| {
                day.token_delta = day.token_delta.saturating_add(total_delta);
                day.add_model_tokens(&model, total_delta);
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

fn normalize_session_id(session_id: &str) -> String {
    let trimmed = session_id.trim();
    if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed.to_string()
    }
}

fn session_display_name(session: &ClaudeSession) -> String {
    project_name_from_cwd(&session.cwd).unwrap_or_else(|| session.id.clone())
}

fn project_name_from_cwd(cwd: &str) -> Option<String> {
    let cwd = cwd.trim();
    if cwd.is_empty() {
        return None;
    }
    Path::new(cwd)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            cwd.trim_end_matches(['\\', '/'])
                .rsplit(['\\', '/'])
                .find(|part| !part.trim().is_empty())
                .map(|part| part.trim().to_string())
        })
}

fn session_status_for_event(
    event: &str,
    tool_name: &str,
    tool_mood: PetMood,
) -> Option<(ClaudeSessionStatus, String)> {
    match event {
        "SessionStart" => Some((ClaudeSessionStatus::Idle, "Session started".to_string())),
        "SessionResume" => Some((ClaudeSessionStatus::Idle, "Session resumed".to_string())),
        "UserPromptSubmit" => Some((ClaudeSessionStatus::Streaming, "Streaming".to_string())),
        "PreToolUse" | "WorktreeCreate" => {
            let tool = if tool_name.trim().is_empty() {
                "tool"
            } else {
                tool_name.trim()
            };
            Some((
                ClaudeSessionStatus::Tool,
                format!("{} {}", tool_mood_label(tool_mood), tool),
            ))
        }
        "PostToolUse" | "PostToolBatch" => {
            Some((ClaudeSessionStatus::Streaming, "Streaming".to_string()))
        }
        "PreCompact" => Some((ClaudeSessionStatus::Compacting, "Compacting".to_string())),
        "PostCompact" => Some((ClaudeSessionStatus::Done, "Compacted".to_string())),
        "PermissionDenied" => Some((ClaudeSessionStatus::Done, "Permission denied".to_string())),
        // A single tool failing is a recoverable hiccup (pet shrugs, turn
        // continues), so it must NOT set the sticky Error session status — that
        // projection would override the low-priority Shrug mood. Leave the prior
        // status untouched and let the next event update it. A turn-ending
        // failure (StopFailure) is a real error.
        "StopFailure" => Some((ClaudeSessionStatus::Error, "Hook failure".to_string())),
        "Stop" => Some((ClaudeSessionStatus::Done, "Ready".to_string())),
        "SessionEnd" => Some((ClaudeSessionStatus::Ended, "Session ended".to_string())),
        "Notification" => Some((ClaudeSessionStatus::Done, "Notification".to_string())),
        "Elicitation" => Some((
            ClaudeSessionStatus::WaitingChoice,
            "Waiting for input".to_string(),
        )),
        // Subagent/task lifecycle events (SubagentStart, SubagentStop,
        // TaskCreated, TaskCompleted) and unknown events may arrive after
        // Stop; they must not flip a finished session back to Streaming.
        _ => None,
    }
}

fn tool_mood_label(mood: PetMood) -> &'static str {
    match mood {
        PetMood::Typing => "Editing",
        PetMood::Building => "Running",
        PetMood::Search => "Reading",
        PetMood::Subagent => "Delegating",
        _ => "Using",
    }
}

fn normalize_tool_name_key(tool_name: &str) -> String {
    tool_name.trim().to_ascii_lowercase()
}

fn tool_activity_key(key: &str) -> String {
    format!("tool:{key}")
}

fn subagent_activity_key(session_id: &str) -> String {
    if session_id.is_empty() {
        SUBAGENT_ACTIVITY_KEY.to_string()
    } else {
        format!("{SUBAGENT_ACTIVITY_KEY}:{session_id}")
    }
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

/// Full play length of the GIF bound to `mood`, if the renderer is loaded.
/// Used to let one-shot interaction moods finish a full loop before decaying.
fn renderer_clip_total_ms(mood: PetMood) -> Option<u32> {
    PET_RENDERER
        .get()?
        .lock()
        .expect("pet renderer poisoned")
        .clip_total_ms(mood)
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
    fn finish_tool_activity_does_not_fall_back_to_other_sessions() {
        let mut state = AppState::new();
        state.start_tool_activity(
            "s1:id:bash-1".to_string(),
            "Bash".to_string(),
            PetMood::Building,
        );

        state.finish_tool_activity(&[], "s2", "Bash", PetMood::Building);

        assert_eq!(state.active_tools, 1);
        assert!(state.active_tool_keys.contains_key("s1:id:bash-1"));
        assert_eq!(state.activity_mood(), Some(PetMood::Building));
    }

    #[test]
    fn finish_session_work_clears_matching_subagents() {
        let mut state = AppState::new();
        state.start_subagent_for_session("s1");
        state.start_subagent_for_session("s2");

        state.finish_session_work("s1");

        assert_eq!(state.active_subagents, 1);
        assert_eq!(state.active_subagent_sessions.get("s2").copied(), Some(1));
        assert!(!state.active_subagent_sessions.contains_key("s1"));
        assert!(state.activity_spans.contains_key("subagent:s2"));
    }

    #[test]
    fn current_pending_interaction_uses_arrival_order_across_types() {
        let mut state = AppState::new();
        // Both requests belong to the focused session; popups are focus-gated,
        // so focus s1 to exercise the across-type arrival-order tie-break.
        state.note_session_event("s1", "", "SessionStart", "", PetMood::Thinking);

        state.pending_permissions.push_back(PendingPermission {
            id: 1,
            interaction_sequence: 1,
            session_id: "s1".to_string(),
            tool_name: "Edit".to_string(),
            tool_use_id: String::new(),
            tool_input_fingerprint: String::new(),
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
    fn focused_session_pending_takes_priority_over_older_global_pending() {
        let mut state = AppState::new();
        state.note_session_event("s1", "", "SessionStart", "", PetMood::Thinking);
        state.note_session_event("s2", "", "SessionStart", "", PetMood::Thinking);
        state.focus_session("s2");

        state.pending_permissions.push_back(PendingPermission {
            id: 1,
            interaction_sequence: 1,
            session_id: "s1".to_string(),
            tool_name: "Edit".to_string(),
            tool_use_id: String::new(),
            tool_input_fingerprint: String::new(),
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
            session_id: "s2".to_string(),
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
            Some(PendingInteractionKind::Choice)
        );
        assert!(state.current_pending_choice().is_some());
        assert!(state.current_pending_permission().is_none());
    }

    #[test]
    fn stale_interaction_finish_does_not_reopen_ended_session() {
        let mut state = AppState::new();
        state.mark_session_waiting_permission("s1", "", "Edit", 1);
        state.note_session_event("s1", "", "SessionEnd", "", PetMood::Thinking);

        state.mark_session_interaction_finished(
            "s1",
            1,
            ClaudeSessionStatus::Done,
            "Permission closed",
        );

        assert_eq!(
            state.sessions.get("s1").expect("session").status,
            ClaudeSessionStatus::Ended
        );
    }

    #[test]
    fn older_interaction_finish_does_not_mask_newer_wait() {
        let mut state = AppState::new();
        state.mark_session_waiting_permission("s1", "", "Edit", 1);
        state.mark_session_waiting_choice("s1", "Question", 2);

        state.mark_session_interaction_finished(
            "s1",
            1,
            ClaudeSessionStatus::Done,
            "Permission closed",
        );

        assert_eq!(
            state.sessions.get("s1").expect("session").status,
            ClaudeSessionStatus::WaitingChoice
        );
    }

    #[test]
    fn focused_session_activity_mood_ignores_other_session_tools() {
        let mut state = AppState::new();
        state.note_session_event("s1", "", "SessionStart", "", PetMood::Thinking);
        state.note_session_event("s2", "", "SessionStart", "", PetMood::Thinking);
        state.focus_session("s2");

        state.start_tool_activity(
            "s1:id:write-1".to_string(),
            "Write".to_string(),
            PetMood::Typing,
        );
        assert_eq!(state.activity_mood(), None);

        state.start_tool_activity(
            "s2:id:bash-1".to_string(),
            "Bash".to_string(),
            PetMood::Building,
        );
        assert_eq!(state.activity_mood(), Some(PetMood::Building));
    }

    #[test]
    fn focused_session_still_allows_global_subagent_visual() {
        let mut state = AppState::new();
        state.note_session_event("s1", "", "SessionStart", "", PetMood::Thinking);
        state.note_session_event("s2", "", "SessionStart", "", PetMood::Thinking);
        state.focus_session("s2");

        state.start_subagent();

        assert_eq!(state.activity_mood(), Some(PetMood::Subagent));
        assert_eq!(state.mood, PetMood::Subagent);
    }

    fn push_test_permission(state: &mut AppState, id: u64, sequence: u64, session: &str) {
        state.pending_permissions.push_back(PendingPermission {
            id,
            interaction_sequence: sequence,
            session_id: session.to_string(),
            tool_name: "Edit".to_string(),
            tool_use_id: String::new(),
            tool_input_fingerprint: String::new(),
            summary: String::new(),
            cwd: String::new(),
            suggestions: Vec::new(),
            waiter: Arc::new(PermissionWaiter {
                decision: Mutex::new(None),
                ready: Condvar::new(),
            }),
        });
    }

    #[test]
    fn busy_focus_is_not_preempted_by_other_session_request() {
        let mut state = AppState::new();
        state.note_session_event("a", "", "UserPromptSubmit", "", PetMood::Thinking);
        state.note_session_event("b", "", "SessionStart", "", PetMood::Thinking);
        assert_eq!(state.focused_session_id.as_deref(), Some("a"));

        // b raises a permission while a is still streaming.
        push_test_permission(&mut state, 1, 1, "b");
        state.mark_session_waiting_permission("b", "", "Edit", 1);

        // Focus stays on the busy session and b's popup is withheld.
        assert_eq!(state.focused_session_id.as_deref(), Some("a"));
        assert!(state.current_pending_permission().is_none());
    }

    #[test]
    fn finished_focus_releases_to_waiting_session() {
        let mut state = AppState::new();
        state.note_session_event("a", "", "UserPromptSubmit", "", PetMood::Thinking);
        state.note_session_event("b", "", "SessionStart", "", PetMood::Thinking);
        push_test_permission(&mut state, 1, 1, "b");
        state.mark_session_waiting_permission("b", "", "Edit", 1);
        assert_eq!(state.focused_session_id.as_deref(), Some("a"));

        // a finishes its turn -> focus hands off to the waiting session b and
        // its popup becomes the current target.
        state.note_session_event("a", "", "Stop", "", PetMood::Thinking);
        assert_eq!(state.focused_session_id.as_deref(), Some("b"));
        assert_eq!(state.current_pending_permission().map(|p| p.id), Some(1));
    }

    #[test]
    fn errored_focus_also_releases_to_waiting_session() {
        let mut state = AppState::new();
        state.note_session_event("a", "", "UserPromptSubmit", "", PetMood::Thinking);
        state.note_session_event("b", "", "SessionStart", "", PetMood::Thinking);
        push_test_permission(&mut state, 1, 1, "b");
        state.mark_session_waiting_permission("b", "", "Edit", 1);

        // An error ends a's turn (per design) -> focus releases to the waiter.
        state.note_session_event("a", "", "StopFailure", "", PetMood::Thinking);
        assert_eq!(state.focused_session_id.as_deref(), Some("b"));
    }

    #[test]
    fn idle_focus_keeps_when_no_session_is_waiting() {
        let mut state = AppState::new();
        state.note_session_event("a", "", "UserPromptSubmit", "", PetMood::Thinking);
        state.note_session_event("b", "", "SessionStart", "", PetMood::Thinking);
        state.note_session_event("a", "", "Stop", "", PetMood::Thinking);

        // a is done and nobody is waiting: focus stays on a, no flicker to b.
        assert_eq!(state.focused_session_id.as_deref(), Some("a"));
    }

    #[test]
    fn new_window_takes_focus_from_idle_session() {
        let mut state = AppState::new();
        state.note_session_event("a", "", "SessionStart", "", PetMood::Thinking);
        state.note_session_event("a", "", "Stop", "", PetMood::Thinking);
        assert_eq!(state.focused_session_id.as_deref(), Some("a"));

        // Opening a new window focuses it when the current focus is idle.
        state.note_session_event("b", "", "SessionStart", "", PetMood::Thinking);
        assert_eq!(state.focused_session_id.as_deref(), Some("b"));
    }

    #[test]
    fn auto_follow_tracks_working_session_when_focus_idle() {
        let mut state = AppState::new();
        state.note_session_event("a", "", "SessionStart", "", PetMood::Thinking);
        state.note_session_event("a", "", "Stop", "", PetMood::Thinking);
        state.note_session_event("b", "", "SessionStart", "", PetMood::Thinking);
        // b (just opened) is focused and idle; a resuming work pulls focus back.
        assert_eq!(state.focused_session_id.as_deref(), Some("b"));

        state.note_session_event("a", "", "UserPromptSubmit", "", PetMood::Thinking);
        assert_eq!(state.focused_session_id.as_deref(), Some("a"));
    }

    #[test]
    fn manual_pin_holds_against_active_work() {
        let mut state = AppState::new();
        state.note_session_event("a", "", "SessionStart", "", PetMood::Thinking);
        state.note_session_event("b", "", "SessionStart", "", PetMood::Thinking);
        state.focus_session("b");

        // a starts working, but the manual pin keeps the pet on b.
        state.note_session_event("a", "", "UserPromptSubmit", "", PetMood::Thinking);
        assert_eq!(state.focused_session_id.as_deref(), Some("b"));
    }

    #[test]
    fn manual_pin_releases_when_pinned_session_ends_then_auto_follows() {
        let mut state = AppState::new();
        state.note_session_event("a", "", "SessionStart", "", PetMood::Thinking);
        state.note_session_event("b", "", "SessionStart", "", PetMood::Thinking);
        state.focus_session("b");
        state.note_session_event("a", "", "UserPromptSubmit", "", PetMood::Thinking);
        assert_eq!(state.focused_session_id.as_deref(), Some("b"));

        // b's window closes: the pin releases and focus auto-follows working a.
        state.note_session_event("b", "", "SessionEnd", "", PetMood::Thinking);
        assert_eq!(state.focused_session_id.as_deref(), Some("a"));
    }

    #[test]
    fn pinned_session_yields_to_waiting_popup() {
        let mut state = AppState::new();
        state.note_session_event("a", "", "SessionStart", "", PetMood::Thinking);
        state.note_session_event("b", "", "SessionStart", "", PetMood::Thinking);
        state.focus_session("b");

        // A blocking request on a must surface even though b is pinned.
        push_test_permission(&mut state, 1, 1, "a");
        state.mark_session_waiting_permission("a", "", "Edit", 1);
        assert_eq!(state.focused_session_id.as_deref(), Some("a"));
        assert_eq!(state.current_pending_permission().map(|p| p.id), Some(1));
    }

    #[test]
    fn completed_focused_session_does_not_mask_resting_happy() {
        let mut state = AppState::new();
        state.note_session_event("s1", "", "SessionStart", "", PetMood::Thinking);
        state.note_session_event("s1", "", "Stop", "", PetMood::Thinking);
        state.set_resting_mood(PetMood::Happy, false);

        assert_eq!(state.resting_mood, PetMood::Happy);
        assert_eq!(state.mood, PetMood::Happy);
    }

    #[test]
    fn session_switcher_keeps_focused_and_recent_sessions_first() {
        let mut state = AppState::new();
        let base = Instant::now();
        for index in 1..=6 {
            let id = format!("s{index}");
            state.note_session_event(&id, "", "SessionStart", "", PetMood::Thinking);
            state.sessions.get_mut(&id).expect("session").last_seen =
                base + Duration::from_secs(index);
        }
        state.focus_session("s1");

        let items = state.session_switcher_items();
        let visible = items
            .iter()
            .take(5)
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(visible.first().copied(), Some("s1"));
        assert!(visible.contains(&"s6"));
    }

    #[test]
    fn session_switcher_uses_project_name_from_cwd() {
        let mut state = AppState::new();
        state.note_session_event(
            "session-abcdef",
            "C:\\Users\\me\\repo",
            "SessionStart",
            "",
            PetMood::Thinking,
        );

        let items = state.session_switcher_items();

        assert_eq!(items[0].display_name, "repo");
    }

    #[test]
    fn ended_sessions_prune_after_retention() {
        let mut state = AppState::new();
        state.note_session_event("old", "", "SessionStart", "", PetMood::Thinking);
        state.note_session_event("old", "", "SessionEnd", "", PetMood::Thinking);
        state.sessions.get_mut("old").expect("session").last_seen =
            Instant::now() - ENDED_SESSION_RETENTION - Duration::from_secs(1);

        state.note_session_event("live", "", "SessionStart", "", PetMood::Thinking);

        assert!(!state.sessions.contains_key("old"));
        assert!(state.sessions.contains_key("live"));
    }

    #[test]
    fn ended_session_pruning_keeps_recent_cap() {
        let mut state = AppState::new();
        let base = Instant::now() - Duration::from_secs(60);
        for index in 0..(MAX_ENDED_SESSIONS + 3) {
            let id = format!("ended-{index}");
            state.note_session_event(&id, "", "SessionEnd", "", PetMood::Thinking);
            state.sessions.get_mut(&id).expect("session").last_seen =
                base + Duration::from_secs(index as u64);
        }

        let ended_count = state
            .sessions
            .values()
            .filter(|session| session.status == ClaudeSessionStatus::Ended)
            .count();

        assert_eq!(ended_count, MAX_ENDED_SESSIONS);
        assert!(!state.sessions.contains_key("ended-0"));
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
    fn interrupt_releases_focused_session_work_and_resets_mood() {
        let mut state = AppState::new();
        state.note_session_event("s1", "", "UserPromptSubmit", "", PetMood::Thinking);
        state.start_tool_activity(
            "s1:id:bash-1".to_string(),
            "Bash".to_string(),
            PetMood::Building,
        );
        assert_eq!(state.mood, PetMood::Building);
        assert_eq!(state.focused_session_id.as_deref(), Some("s1"));

        // ESC interrupt detected at file length 4096.
        state.mark_session_interrupted("s1", 4096);

        assert_eq!(state.active_tools, 0);
        assert_eq!(
            state.sessions.get("s1").expect("session").status,
            ClaudeSessionStatus::Done
        );
        assert_eq!(
            state.sessions.get("s1").expect("session").turn_baseline_len,
            4096
        );
        assert!(!state.mood.is_active_work());
    }

    #[test]
    fn note_session_transcript_baselines_at_first_sight_and_turn_start() {
        let mut state = AppState::new();
        state.note_session_event("s1", "", "SessionStart", "", PetMood::Thinking);

        // First capture baselines at the current length (excludes old markers).
        state.note_session_transcript("s1", "t.jsonl", 100, false);
        assert_eq!(state.sessions.get("s1").unwrap().turn_baseline_len, 100);

        // A mid-turn update keeps the baseline.
        state.note_session_transcript("s1", "t.jsonl", 250, false);
        assert_eq!(state.sessions.get("s1").unwrap().turn_baseline_len, 100);

        // A new turn advances it.
        state.note_session_transcript("s1", "t.jsonl", 250, true);
        assert_eq!(state.sessions.get("s1").unwrap().turn_baseline_len, 250);
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

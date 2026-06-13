mod secrets;
pub(crate) mod storage;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::app::{PetMood, pomodoro::PomodoroSettings};
use crate::config::{DEFAULT_PROXY_PORT, PET_SCALE_MAX_PERCENT, PET_SCALE_MIN_PERCENT};
use storage::{
    json_without_bom, read_json, read_json_or_default, save_pretty_json, write_text_atomic,
};

const DEFAULT_GIF_DIR: &str = "assets/claudie";
const DEFAULT_SLEEP_AFTER_SECS: u32 = 75;
const SLEEP_AFTER_MIN_SECS: u32 = 15;
const SLEEP_AFTER_MAX_SECS: u32 = 1800;
pub(crate) const OFFICIAL_LLM_PROFILE_ID: &str = "official";
const OFFICIAL_LLM_PROFILE_NAME: &str = "Official";
const LEGACY_FISHING_ANIMATIONS: (&str, &str, &str, &str) =
    ("pomodoro", "building", "happy", "error");

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct UserSettings {
    pub(crate) pet_dir: String,
    pub(crate) gif_dir: String,
    pub(crate) animations: AnimationSettings,
    pub(crate) window_position: Option<WindowPosition>,
    #[serde(default = "default_show_session_switcher")]
    pub(crate) show_session_switcher: bool,
    #[serde(default = "default_pet_scale_percent")]
    pub(crate) pet_scale_percent: u32,
    #[serde(default = "default_sleep_after_secs")]
    pub(crate) sleep_after_secs: u32,
    pub(crate) pomodoro: PomodoroSettings,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub(crate) struct WindowPosition {
    pub(crate) x: i32,
    pub(crate) y: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct AnimationSettings {
    pub(crate) idle: String,
    pub(crate) thinking: String,
    pub(crate) typing: String,
    pub(crate) building: String,
    #[serde(alias = "permission")]
    pub(crate) search: String,
    pub(crate) happy: String,
    pub(crate) error: String,
    pub(crate) sleeping: String,
    pub(crate) subagent: String,
    pub(crate) pomodoro: String,
    pub(crate) wave: String,
    pub(crate) stretch: String,
    pub(crate) fishing: String,
    pub(crate) fishing_reel: String,
    pub(crate) fishing_caught: String,
    pub(crate) fishing_missed: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct LlmProfile {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) base_url: String,
    #[serde(
        default,
        serialize_with = "secrets::serialize_secret",
        deserialize_with = "secrets::deserialize_secret"
    )]
    pub(crate) auth_token: String,
    #[serde(
        default,
        serialize_with = "secrets::serialize_secret",
        deserialize_with = "secrets::deserialize_secret"
    )]
    pub(crate) api_key: String,
    pub(crate) model: String,
    pub(crate) opus_model: String,
    pub(crate) sonnet_model: String,
    pub(crate) haiku_model: String,
    pub(crate) openai_extra_body: String,
    pub(crate) extra_env: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct LlmProfileDb {
    pub(crate) profiles: Vec<LlmProfile>,
    pub(crate) active_profile_id: String,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            pet_dir: String::new(),
            gif_dir: DEFAULT_GIF_DIR.to_string(),
            animations: AnimationSettings::default(),
            window_position: None,
            show_session_switcher: true,
            pet_scale_percent: 80,
            sleep_after_secs: DEFAULT_SLEEP_AFTER_SECS,
            pomodoro: PomodoroSettings::default(),
        }
    }
}

impl Default for AnimationSettings {
    fn default() -> Self {
        Self {
            idle: "idle".to_string(),
            thinking: "thinking".to_string(),
            typing: "typing".to_string(),
            building: "building".to_string(),
            search: "search".to_string(),
            happy: "happy".to_string(),
            error: "error".to_string(),
            sleeping: "sleeping".to_string(),
            subagent: "subagent".to_string(),
            pomodoro: "pomodoro".to_string(),
            wave: "wave".to_string(),
            stretch: "stretch".to_string(),
            fishing: "fishing".to_string(),
            fishing_reel: "reel".to_string(),
            fishing_caught: "caught".to_string(),
            fishing_missed: "missed".to_string(),
        }
    }
}

impl Default for LlmProfileDb {
    fn default() -> Self {
        Self {
            profiles: vec![official_llm_profile()],
            active_profile_id: OFFICIAL_LLM_PROFILE_ID.to_string(),
        }
    }
}

impl UserSettings {
    pub(crate) fn pet_scale_percent(&self) -> u32 {
        self.pet_scale_percent
            .clamp(PET_SCALE_MIN_PERCENT, PET_SCALE_MAX_PERCENT)
    }

    pub(crate) fn sleep_after_secs(&self) -> u32 {
        self.sleep_after_secs
            .clamp(SLEEP_AFTER_MIN_SECS, SLEEP_AFTER_MAX_SECS)
    }

    pub(crate) fn animation_value(&self, mood: PetMood) -> &str {
        match mood {
            PetMood::Idle => &self.animations.idle,
            PetMood::Thinking => &self.animations.thinking,
            PetMood::Typing => &self.animations.typing,
            PetMood::Building => &self.animations.building,
            PetMood::Search => &self.animations.search,
            PetMood::Happy => &self.animations.happy,
            PetMood::Error => &self.animations.error,
            PetMood::Sleeping => &self.animations.sleeping,
            PetMood::Subagent => &self.animations.subagent,
            PetMood::Pomodoro => &self.animations.pomodoro,
            PetMood::Wave => &self.animations.wave,
            PetMood::Stretch => &self.animations.stretch,
            PetMood::Fishing => &self.animations.fishing,
            PetMood::FishingReel => &self.animations.fishing_reel,
            PetMood::FishingCaught => &self.animations.fishing_caught,
            PetMood::FishingMissed => &self.animations.fishing_missed,
        }
    }

    pub(crate) fn pet_asset_base_dir(&self) -> PathBuf {
        let trimmed = self.pet_dir.trim();
        if trimmed.is_empty() {
            default_bundled_pet_dir()
        } else {
            expand_home(trimmed)
        }
    }
}

fn default_pet_scale_percent() -> u32 {
    80
}

fn default_sleep_after_secs() -> u32 {
    DEFAULT_SLEEP_AFTER_SECS
}

fn default_show_session_switcher() -> bool {
    true
}

impl LlmProfileDb {
    pub(crate) fn official_profile_active(&self) -> bool {
        self.active_profile_id.trim() == OFFICIAL_LLM_PROFILE_ID
    }

    pub(crate) fn normalize(&mut self) {
        self.ensure_official_profile();
        self.profiles.retain(|profile| {
            !profile.id.trim().is_empty()
                || !profile.name.trim().is_empty()
                || !profile.model.trim().is_empty()
        });
        for profile in &mut self.profiles {
            if profile.id.trim().is_empty() {
                profile.id = default_profile_id(&profile.name);
            } else {
                profile.id = profile.id.trim().to_string();
            }
        }
        if self.active_profile_id.trim().is_empty() {
            if let Some(profile) = self.profiles.first() {
                self.active_profile_id = profile.id.clone();
            }
        }
    }

    fn ensure_official_profile(&mut self) {
        if let Some(profile) = self
            .profiles
            .iter_mut()
            .find(|profile| profile.id.trim() == OFFICIAL_LLM_PROFILE_ID)
        {
            if profile.name.trim().is_empty() {
                profile.name = OFFICIAL_LLM_PROFILE_NAME.to_string();
            }
        } else {
            self.profiles.insert(0, official_llm_profile());
        }
    }

    pub(crate) fn active_profile(&self) -> Option<&LlmProfile> {
        self.profiles
            .iter()
            .find(|profile| profile.id == self.active_profile_id)
            .or_else(|| self.profiles.first())
    }

    pub(crate) fn upsert_profile(&mut self, mut profile: LlmProfile) {
        if profile.id.trim().is_empty() {
            profile.id = default_profile_id(&profile.name);
        } else {
            profile.id = profile.id.trim().to_string();
        }
        if let Some(existing) = self
            .profiles
            .iter_mut()
            .find(|existing| existing.id == profile.id)
        {
            *existing = profile;
        } else {
            self.profiles.push(profile);
        }
    }

    pub(crate) fn remove_profile(&mut self, id: &str) -> Option<LlmProfile> {
        let id = id.trim();
        let index = self.profiles.iter().position(|profile| profile.id == id)?;
        let removed = self.profiles.remove(index);
        if self.active_profile_id == removed.id {
            self.active_profile_id = self
                .profiles
                .get(index)
                .or_else(|| self.profiles.last())
                .map(|profile| profile.id.clone())
                .unwrap_or_default();
        }
        Some(removed)
    }
}

impl LlmProfile {
    pub(crate) fn is_official(&self) -> bool {
        self.id.trim() == OFFICIAL_LLM_PROFILE_ID
    }

    pub(crate) fn display_label(&self) -> String {
        match (self.name.trim().is_empty(), self.model.trim().is_empty()) {
            (false, false) => format!("{} {}", self.name.trim(), self.model.trim()),
            (false, true) => self.name.trim().to_string(),
            (true, false) => format!("model {}", self.model.trim()),
            (true, true) => String::new(),
        }
    }

    pub(crate) fn is_openai_chat_proxy(&self) -> bool {
        let base_url = self.base_url.trim().to_ascii_lowercase();
        base_url.contains("/chat/completions")
            || self.extra_env.lines().any(|line| {
                let Some((key, value)) = line.split_once('=') else {
                    return false;
                };
                let key = key.trim();
                let value = value.trim().to_ascii_lowercase();
                (matches!(key, "CLAUDIE_API_FORMAT" | "CLAUDIE_LLM_FORMAT")
                    && matches!(
                        value.as_str(),
                        "openai" | "openai-chat" | "openai-chat-completions"
                    ))
                    || (key == "CLAUDIE_OPENAI_PROXY"
                        && matches!(value.as_str(), "1" | "true" | "yes"))
            })
    }

    pub(crate) fn openai_chat_completions_url(&self) -> String {
        let base_url = self.base_url.trim().trim_end_matches('/');
        if base_url.ends_with("/chat/completions") {
            base_url.to_string()
        } else if base_url.ends_with("/v1") {
            format!("{base_url}/chat/completions")
        } else {
            format!("{base_url}/v1/chat/completions")
        }
    }

    pub(crate) fn openai_upstream_api_key(&self) -> &str {
        let api_key = self.api_key.trim();
        if api_key.is_empty() {
            self.auth_token.trim()
        } else {
            api_key
        }
    }

    pub(crate) fn openai_extra_body_fields(&self) -> Result<Map<String, Value>, String> {
        parse_openai_extra_body(&self.openai_extra_body)
    }

    pub(crate) fn extra_env_value(&self, key: &str) -> Option<String> {
        parse_extra_env(&self.extra_env)
            .ok()?
            .into_iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
            .map(|(_, value)| value)
    }

    fn env_pairs(&self) -> Vec<(&'static str, String)> {
        let proxy_base_url = format!("http://127.0.0.1:{DEFAULT_PROXY_PORT}");
        let proxy_auth_token = self
            .auth_token
            .trim()
            .is_empty()
            .then(|| "claudie-openai-proxy".to_string())
            .unwrap_or_else(|| self.auth_token.trim().to_string());
        let fields = if self.is_openai_chat_proxy() {
            [
                ("ANTHROPIC_BASE_URL", proxy_base_url.as_str()),
                ("ANTHROPIC_AUTH_TOKEN", proxy_auth_token.as_str()),
                ("ANTHROPIC_API_KEY", ""),
                ("ANTHROPIC_MODEL", self.model.trim()),
                ("ANTHROPIC_DEFAULT_OPUS_MODEL", self.opus_model.trim()),
                ("ANTHROPIC_DEFAULT_SONNET_MODEL", self.sonnet_model.trim()),
                ("ANTHROPIC_DEFAULT_HAIKU_MODEL", self.haiku_model.trim()),
            ]
        } else {
            [
                ("ANTHROPIC_BASE_URL", self.base_url.trim()),
                ("ANTHROPIC_AUTH_TOKEN", self.auth_token.trim()),
                ("ANTHROPIC_API_KEY", self.api_key.trim()),
                ("ANTHROPIC_MODEL", self.model.trim()),
                ("ANTHROPIC_DEFAULT_OPUS_MODEL", self.opus_model.trim()),
                ("ANTHROPIC_DEFAULT_SONNET_MODEL", self.sonnet_model.trim()),
                ("ANTHROPIC_DEFAULT_HAIKU_MODEL", self.haiku_model.trim()),
            ]
        };
        fields
            .into_iter()
            .filter(|(_, value)| !value.is_empty())
            .map(|(key, value)| (key, value.to_string()))
            .collect()
    }
}

fn official_llm_profile() -> LlmProfile {
    LlmProfile {
        id: OFFICIAL_LLM_PROFILE_ID.to_string(),
        name: OFFICIAL_LLM_PROFILE_NAME.to_string(),
        ..LlmProfile::default()
    }
}

pub(crate) fn settings_path() -> PathBuf {
    claudie_home().join("settings.json")
}

pub(crate) fn llm_profiles_path() -> PathBuf {
    claudie_home().join("llm_profiles.json")
}

pub(crate) fn load_user_settings() -> UserSettings {
    let path = settings_path();
    let mut had_settings_file = true;
    let mut settings = match read_json(&path) {
        Ok(settings) => settings,
        Err(_) => {
            had_settings_file = false;
            UserSettings::default()
        }
    };

    if normalize_user_settings(&mut settings) && had_settings_file {
        let _ = save_user_settings(&settings);
    }
    settings
}

fn normalize_user_settings(settings: &mut UserSettings) -> bool {
    let mut changed = false;

    if settings.gif_dir.trim().is_empty() {
        settings.gif_dir = DEFAULT_GIF_DIR.to_string();
        changed = true;
    }

    let legacy_fishing_animations = legacy_fishing_animations(&settings.animations);

    let legacy_search_animation = settings
        .animations
        .search
        .trim()
        .eq_ignore_ascii_case("permission")
        || settings
            .animations
            .search
            .trim()
            .eq_ignore_ascii_case("permission.gif");
    if legacy_search_animation && settings.gif_dir.trim() == DEFAULT_GIF_DIR {
        settings.animations.search = "search".to_string();
        changed = true;
    }

    if legacy_fishing_animations && settings.gif_dir.trim() == DEFAULT_GIF_DIR {
        set_default_fishing_animations(&mut settings.animations);
        changed = true;
    }

    // If the persisted gif_dir doesn't actually contain the required GIFs
    // (e.g. left over from a previous build), reset it to the bundled default
    // so the panel surfaces the right path next time it loads.
    if !settings.gif_dir.trim().is_empty()
        && settings.gif_dir != DEFAULT_GIF_DIR
        && configured_gif_dir_strict(settings).is_none()
    {
        settings.gif_dir = DEFAULT_GIF_DIR.to_string();
        if legacy_search_animation {
            settings.animations.search = "search".to_string();
        }
        if legacy_fishing_animations {
            set_default_fishing_animations(&mut settings.animations);
        }
        changed = true;
    }

    if settings.pet_scale_percent != settings.pet_scale_percent() {
        settings.pet_scale_percent = settings.pet_scale_percent();
        changed = true;
    }

    if settings.sleep_after_secs != settings.sleep_after_secs() {
        settings.sleep_after_secs = settings.sleep_after_secs();
        changed = true;
    }

    if settings.pomodoro.normalize() {
        changed = true;
    }

    changed
}

fn legacy_fishing_animations(animations: &AnimationSettings) -> bool {
    animation_name_matches(&animations.fishing, LEGACY_FISHING_ANIMATIONS.0)
        && animation_name_matches(&animations.fishing_reel, LEGACY_FISHING_ANIMATIONS.1)
        && animation_name_matches(&animations.fishing_caught, LEGACY_FISHING_ANIMATIONS.2)
        && animation_name_matches(&animations.fishing_missed, LEGACY_FISHING_ANIMATIONS.3)
}

fn set_default_fishing_animations(animations: &mut AnimationSettings) {
    animations.fishing = "fishing".to_string();
    animations.fishing_reel = "reel".to_string();
    animations.fishing_caught = "caught".to_string();
    animations.fishing_missed = "missed".to_string();
}

fn animation_name_matches(value: &str, expected: &str) -> bool {
    let trimmed = value.trim();
    trimmed.eq_ignore_ascii_case(expected)
        || trimmed.eq_ignore_ascii_case(&format!("{expected}.gif"))
}

pub(crate) fn load_llm_profile_db() -> LlmProfileDb {
    let mut db: LlmProfileDb = read_json_or_default(&llm_profiles_path());
    db.normalize();
    db
}

pub(crate) fn save_llm_profile_db(db: &LlmProfileDb) -> Result<(), String> {
    save_pretty_json(&llm_profiles_path(), db)
}

pub(crate) fn save_user_settings(settings: &UserSettings) -> Result<(), String> {
    save_pretty_json(&settings_path(), settings)
}

pub(crate) fn ensure_claude_onboarding_complete() -> Result<(), String> {
    ensure_claude_onboarding_complete_at(&claude_onboarding_path())
}

pub(crate) fn apply_llm_profile_to_claude(profile: &LlmProfile) -> Result<(), String> {
    if profile.name.trim().is_empty() {
        return Err("Profile name is required".to_string());
    }

    let path = claude_settings_path();
    let mut settings = if path.exists() {
        let text = fs::read_to_string(&path).map_err(|err| err.to_string())?;
        serde_json::from_str::<Value>(json_without_bom(&text))
            .map_err(|err| format!("~/.claude/settings.json is not valid JSON: {err}"))?
    } else {
        Value::Object(serde_json::Map::new())
    };

    if !settings.is_object() {
        settings = Value::Object(serde_json::Map::new());
    }
    let root = settings.as_object_mut().expect("settings object");
    let env_value = root
        .entry("env".to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if !env_value.is_object() {
        *env_value = Value::Object(serde_json::Map::new());
    }
    let env = env_value.as_object_mut().expect("env object");

    for key in managed_llm_env_keys() {
        env.remove(*key);
    }
    for (key, value) in profile.env_pairs() {
        env.insert(key.to_string(), Value::String(value));
    }
    for (key, value) in parse_extra_env(&profile.extra_env)? {
        env.insert(key, Value::String(value));
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let backup_path = path.with_extension("json.claudie.bak");
    if path.exists() && !backup_path.exists() {
        let _ = fs::copy(&path, backup_path);
    }
    write_text_atomic(
        &path,
        &format!(
            "{}\n",
            serde_json::to_string_pretty(&settings).map_err(|err| err.to_string())?
        ),
    )
}

pub(crate) fn current_claude_llm_profile() -> Option<LlmProfile> {
    let text = fs::read_to_string(claude_settings_path()).ok()?;
    let value: Value = serde_json::from_str(json_without_bom(&text)).ok()?;
    let env = value.get("env").and_then(Value::as_object);
    let model = env
        .and_then(|env| env_string(env, "ANTHROPIC_MODEL"))
        .or_else(|| {
            value
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();
    let name = env
        .map(|env| provider_name_from_env(env, &model))
        .unwrap_or_else(|| provider_name_from_model(&model));
    Some(LlmProfile {
        id: default_profile_id(&name),
        name,
        base_url: env
            .and_then(|env| env_string(env, "ANTHROPIC_BASE_URL"))
            .unwrap_or_default(),
        auth_token: env
            .and_then(|env| env_string(env, "ANTHROPIC_AUTH_TOKEN"))
            .unwrap_or_default(),
        api_key: env
            .and_then(|env| env_string(env, "ANTHROPIC_API_KEY"))
            .unwrap_or_default(),
        model,
        opus_model: env
            .and_then(|env| env_string(env, "ANTHROPIC_DEFAULT_OPUS_MODEL"))
            .unwrap_or_default(),
        sonnet_model: env
            .and_then(|env| env_string(env, "ANTHROPIC_DEFAULT_SONNET_MODEL"))
            .unwrap_or_default(),
        haiku_model: env
            .and_then(|env| env_string(env, "ANTHROPIC_DEFAULT_HAIKU_MODEL"))
            .unwrap_or_default(),
        openai_extra_body: String::new(),
        extra_env: env.map(extra_env_from_claude).unwrap_or_default(),
    })
}

pub(crate) fn configured_gif_dir(settings: &UserSettings) -> Option<PathBuf> {
    let raw = settings.gif_dir.trim();
    let candidate = if raw.is_empty() { DEFAULT_GIF_DIR } else { raw };
    let configured = expand_home(candidate);

    let configured_dir = if configured.is_absolute() {
        configured.is_dir().then(|| configured.clone())
    } else {
        let direct = settings.pet_asset_base_dir().join(&configured);
        direct.is_dir().then_some(direct)
    };
    if let Some(dir) = configured_dir {
        if dir_has_required_gifs(&dir, settings) {
            return Some(dir);
        }
    }

    // Configured path is missing or stale (e.g. left over from an older
    // build); fall back to the bundled default so the pet still renders.
    let fallback = settings.pet_asset_base_dir().join(DEFAULT_GIF_DIR);
    if dir_has_required_gifs(&fallback, settings) {
        return Some(fallback);
    }

    None
}

fn configured_gif_dir_strict(settings: &UserSettings) -> Option<PathBuf> {
    let raw = settings.gif_dir.trim();
    if raw.is_empty() {
        return None;
    }
    let candidate = expand_home(raw);
    let dir = if candidate.is_absolute() {
        candidate.is_dir().then_some(candidate)?
    } else {
        let direct = settings.pet_asset_base_dir().join(&candidate);
        direct.is_dir().then_some(direct)?
    };
    dir_has_required_gifs(&dir, settings).then_some(dir)
}

fn dir_has_required_gifs(dir: &Path, settings: &UserSettings) -> bool {
    mood_rows().iter().all(|(mood, _)| {
        let name = settings.animation_value(*mood);
        let filename = if name.ends_with(".gif") || name.ends_with(".GIF") {
            name.to_string()
        } else {
            format!("{name}.gif")
        };
        dir.join(filename).is_file()
    })
}

pub(crate) fn mood_rows() -> &'static [(PetMood, &'static str)] {
    &[
        (PetMood::Idle, "Idle"),
        (PetMood::Thinking, "Thinking"),
        (PetMood::Typing, "Typing"),
        (PetMood::Building, "Building"),
        (PetMood::Search, "Search"),
        (PetMood::Happy, "Happy"),
        (PetMood::Error, "Error"),
        (PetMood::Sleeping, "Sleeping"),
        (PetMood::Subagent, "Subagent"),
        (PetMood::Pomodoro, "Pomodoro"),
        (PetMood::Wave, "Wave"),
        (PetMood::Stretch, "Stretch"),
        (PetMood::Fishing, "Fishing"),
        (PetMood::FishingReel, "Fishing reel"),
        (PetMood::FishingCaught, "Fishing caught"),
        (PetMood::FishingMissed, "Fishing missed"),
    ]
}

pub(crate) fn claudie_home() -> PathBuf {
    env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claudie")
}

pub(crate) fn default_profile_id(name: &str) -> String {
    profile_id_from_name(name)
}

fn claude_settings_path() -> PathBuf {
    home_dir().join(".claude").join("settings.json")
}

fn claude_onboarding_path() -> PathBuf {
    home_dir().join(".claude.json")
}

fn ensure_claude_onboarding_complete_at(path: &Path) -> Result<(), String> {
    const KEY: &str = "hasCompletedOnboarding";
    const FIELD: &str = "\"hasCompletedOnboarding\": true";

    if !path.exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        return write_text_atomic(path, &format!("{{\n  {FIELD}\n}}\n"));
    }

    let text = fs::read_to_string(path).map_err(|err| err.to_string())?;
    let text = json_without_bom(&text);
    let value = serde_json::from_str::<Value>(text)
        .map_err(|err| format!("~/.claude.json is not valid JSON: {err}"))?;
    let root = value
        .as_object()
        .ok_or_else(|| "~/.claude.json root must be a JSON object".to_string())?;

    if root.get(KEY).and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }

    let updated = if root.contains_key(KEY) {
        replace_root_field_value(text, KEY, "true").unwrap_or_else(|| {
            let mut value = value;
            if let Some(root) = value.as_object_mut() {
                root.insert(KEY.to_string(), Value::Bool(true));
            }
            format!(
                "{}\n",
                serde_json::to_string_pretty(&value).expect("valid JSON")
            )
        })
    } else {
        append_root_bool_field(text, FIELD)
    };

    write_text_atomic(path, &updated)
}

fn append_root_bool_field(text: &str, field: &str) -> String {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let closing = text
        .char_indices()
        .rev()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    let prefix = text[..closing].trim_end();
    let mut updated = String::with_capacity(text.len() + field.len() + 8);
    updated.push_str(prefix);
    if !prefix.ends_with('{') {
        updated.push(',');
    }
    updated.push_str(newline);
    updated.push_str("  ");
    updated.push_str(field);
    updated.push_str(newline);
    updated.push('}');
    updated.push_str(newline);
    updated
}

fn replace_root_field_value(text: &str, key: &str, value: &str) -> Option<String> {
    let (start, end) = root_field_value_span(text, key)?;
    let mut updated = String::with_capacity(text.len() + value.len());
    updated.push_str(&text[..start]);
    updated.push_str(value);
    updated.push_str(&text[end..]);
    Some(updated)
}

fn root_field_value_span(text: &str, key: &str) -> Option<(usize, usize)> {
    let mut index = text.find('{')? + 1;
    let mut depth = 1_u32;
    while index < text.len() {
        index = skip_ws(text, index);
        let ch = text[index..].chars().next()?;
        match ch {
            '}' => return None,
            ',' => {
                index += 1;
            }
            '"' if depth == 1 => {
                let (field, after_key) = parse_json_string_at(text, index)?;
                let colon = skip_ws(text, after_key);
                if text[colon..].chars().next()? != ':' {
                    return None;
                }
                let value_start = skip_ws(text, colon + 1);
                let value_end = json_value_end(text, value_start)?;
                if field == key {
                    return Some((value_start, value_end));
                }
                index = value_end;
            }
            _ => {
                if ch == '{' || ch == '[' {
                    depth += 1;
                }
                index += ch.len_utf8();
            }
        }
    }
    None
}

fn parse_json_string_at(text: &str, start: usize) -> Option<(String, usize)> {
    let mut escaped = false;
    for (offset, ch) in text[start + 1..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            let end = start + 1 + offset + 1;
            let parsed = serde_json::from_str::<String>(&text[start..end]).ok()?;
            return Some((parsed, end));
        }
    }
    None
}

fn json_value_end(text: &str, start: usize) -> Option<usize> {
    let mut index = start;
    let mut depth = 0_i32;
    let mut in_string = false;
    let mut escaped = false;
    while index < text.len() {
        let ch = text[index..].chars().next()?;
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            index += ch.len_utf8();
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' | '[' => depth += 1,
            '}' if depth == 0 => return Some(index),
            ']' | '}' => depth -= 1,
            ',' if depth == 0 => return Some(index),
            _ => {}
        }
        index += ch.len_utf8();
    }
    Some(text.len())
}

fn skip_ws(text: &str, mut index: usize) -> usize {
    while index < text.len() {
        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        if !ch.is_whitespace() {
            break;
        }
        index += ch.len_utf8();
    }
    index
}

fn parse_extra_env(value: &str) -> Result<Vec<(String, String)>, String> {
    let mut pairs = Vec::new();
    for (index, line) in value.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("Extra env line {} must be KEY=VALUE", index + 1));
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(format!("Extra env line {} has an empty key", index + 1));
        }
        pairs.push((key.to_string(), value.trim().to_string()));
    }
    Ok(pairs)
}

fn parse_openai_extra_body(value: &str) -> Result<Map<String, Value>, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(Map::new());
    }

    if trimmed.starts_with('{') {
        let value = serde_json::from_str::<Value>(trimmed)
            .map_err(|err| format!("OpenAI extra body must be valid JSON: {err}"))?;
        let Value::Object(map) = value else {
            return Err("OpenAI extra body must be a JSON object".to_string());
        };
        return validate_openai_extra_body(map);
    }

    let mut map = Map::new();
    for (index, line) in trimmed.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, raw_value)) = line.split_once('=').or_else(|| line.split_once(':')) else {
            return Err(format!(
                "OpenAI extra body line {} must be key=value or a JSON object",
                index + 1
            ));
        };
        let key = normalize_openai_extra_key(key.trim());
        if key.is_empty() {
            return Err(format!(
                "OpenAI extra body line {} has an empty key",
                index + 1
            ));
        }
        let raw_value = raw_value.trim();
        let parsed = serde_json::from_str::<Value>(raw_value)
            .unwrap_or_else(|_| Value::String(raw_value.to_string()));
        map.insert(key, parsed);
    }
    validate_openai_extra_body(map)
}

fn validate_openai_extra_body(map: Map<String, Value>) -> Result<Map<String, Value>, String> {
    for key in map.keys() {
        if matches!(key.as_str(), "messages" | "stream") {
            return Err(format!(
                "OpenAI extra body cannot override the managed `{key}` field"
            ));
        }
    }
    Ok(map)
}

fn normalize_openai_extra_key(key: &str) -> String {
    match key {
        "model_reasoning_effort" => "reasoning_effort".to_string(),
        _ => key.to_string(),
    }
}

fn managed_llm_env_keys() -> &'static [&'static str] {
    &[
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_MODEL",
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        "ANTHROPIC_SMALL_FAST_MODEL",
        "ANTHROPIC_CUSTOM_MODEL_OPTION",
        "ANTHROPIC_CUSTOM_MODEL_OPTION_NAME",
        "ANTHROPIC_CUSTOM_MODEL_OPTION_DESCRIPTION",
        "CLAUDE_CODE_USE_VERTEX",
        "ANTHROPIC_VERTEX_BASE_URL",
        "CLAUDE_CODE_USE_BEDROCK",
        "ANTHROPIC_BEDROCK_BASE_URL",
        "CLAUDE_CODE_USE_ANTHROPIC_AWS",
        "ANTHROPIC_AWS_BASE_URL",
    ]
}

fn extra_env_from_claude(env: &serde_json::Map<String, Value>) -> String {
    let mut lines = Vec::new();
    for key in managed_llm_env_keys() {
        if matches!(
            *key,
            "ANTHROPIC_BASE_URL"
                | "ANTHROPIC_AUTH_TOKEN"
                | "ANTHROPIC_API_KEY"
                | "ANTHROPIC_MODEL"
                | "ANTHROPIC_DEFAULT_OPUS_MODEL"
                | "ANTHROPIC_DEFAULT_SONNET_MODEL"
                | "ANTHROPIC_DEFAULT_HAIKU_MODEL"
        ) {
            continue;
        }
        if let Some(value) = env_string(env, key) {
            lines.push(format!("{key}={value}"));
        }
    }
    lines.join("\n")
}

fn provider_name_from_env(env: &serde_json::Map<String, Value>, model: &str) -> String {
    if let Some(base_url) = env_string(env, "ANTHROPIC_BASE_URL") {
        let without_scheme = base_url
            .strip_prefix("https://")
            .or_else(|| base_url.strip_prefix("http://"))
            .unwrap_or(&base_url);
        let host = without_scheme
            .split(['/', '?', '#'])
            .next()
            .unwrap_or(without_scheme)
            .trim();
        if !host.is_empty() && host != "api.anthropic.com" {
            return host.to_string();
        }
    }
    provider_name_from_model(model)
}

fn provider_name_from_model(model: &str) -> String {
    if !model.trim().is_empty() {
        "Claude Code".to_string()
    } else {
        "LLM Profile".to_string()
    }
}

fn env_string(env: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    env.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn profile_id_from_name(name: &str) -> String {
    let mut id = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            id.push(ch.to_ascii_lowercase());
        } else if !id.ends_with('-') {
            id.push('-');
        }
    }
    let id = id.trim_matches('-');
    if id.is_empty() {
        format!("profile-{}", timestamp_millis())
    } else {
        id.to_string()
    }
}

fn timestamp_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn expand_home(value: &str) -> PathBuf {
    if value == "~" {
        return home_dir();
    }
    if let Some(rest) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        return home_dir().join(rest);
    }
    Path::new(value).to_path_buf()
}

fn home_dir() -> PathBuf {
    env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn default_bundled_pet_dir() -> PathBuf {
    let mut candidates = Vec::new();
    if let Ok(current) = env::current_dir() {
        candidates.push(current.clone());
        candidates.push(current.join("assets"));
        candidates.push(current.join("assets").join("pet"));
    }
    if let Ok(exe) = env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.to_path_buf());
        candidates.push(dir.join("assets"));
        candidates.push(dir.join("assets").join("pet"));
        if let Some(project_dir) = dir.parent().and_then(Path::parent) {
            candidates.push(project_dir.to_path_buf());
            candidates.push(project_dir.join("assets"));
            candidates.push(project_dir.join("assets").join("pet"));
        }
    }
    candidates
        .into_iter()
        .find(|path| path.join(DEFAULT_GIF_DIR).is_dir())
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_llm_profiles_include_empty_official_profile() {
        let db = LlmProfileDb::default();
        let profile = db.active_profile().unwrap();

        assert_eq!(db.active_profile_id, OFFICIAL_LLM_PROFILE_ID);
        assert_eq!(profile.id, OFFICIAL_LLM_PROFILE_ID);
        assert_eq!(profile.name, OFFICIAL_LLM_PROFILE_NAME);
        assert!(profile.env_pairs().is_empty());
        assert!(profile.extra_env.is_empty());
    }

    #[test]
    fn normalize_adds_official_profile_to_existing_databases() {
        let mut db = LlmProfileDb {
            profiles: vec![LlmProfile {
                id: "custom".to_string(),
                name: "Custom".to_string(),
                model: "custom-model".to_string(),
                ..LlmProfile::default()
            }],
            active_profile_id: "custom".to_string(),
        };

        db.normalize();

        assert_eq!(db.active_profile_id, "custom");
        assert_eq!(
            db.profiles
                .iter()
                .map(|profile| profile.id.as_str())
                .collect::<Vec<_>>(),
            vec![OFFICIAL_LLM_PROFILE_ID, "custom"]
        );
    }

    #[test]
    fn legacy_permission_animation_becomes_search_for_default_assets() {
        let mut settings: UserSettings = serde_json::from_value(serde_json::json!({
            "gif_dir": DEFAULT_GIF_DIR,
            "animations": {
                "idle": "idle",
                "thinking": "thinking",
                "typing": "typing",
                "building": "building",
                "permission": "permission",
                "happy": "happy",
                "error": "error",
                "sleeping": "sleeping",
                "subagent": "subagent"
            }
        }))
        .unwrap();

        assert_eq!(settings.animations.search, "permission");
        assert!(normalize_user_settings(&mut settings));
        assert_eq!(settings.animations.search, "search");
    }

    #[test]
    fn default_fishing_animations_use_fishing_gifs() {
        let settings = AnimationSettings::default();

        assert_eq!(settings.fishing, "fishing");
        assert_eq!(settings.fishing_reel, "reel");
        assert_eq!(settings.fishing_caught, "caught");
        assert_eq!(settings.fishing_missed, "missed");
    }

    #[test]
    fn legacy_fishing_animations_migrate_for_default_assets() {
        let mut settings: UserSettings = serde_json::from_value(serde_json::json!({
            "gif_dir": DEFAULT_GIF_DIR,
            "animations": {
                "idle": "idle",
                "thinking": "thinking",
                "typing": "typing",
                "building": "building",
                "search": "search",
                "happy": "happy",
                "error": "error",
                "sleeping": "sleeping",
                "subagent": "subagent",
                "pomodoro": "pomodoro",
                "wave": "wave",
                "stretch": "stretch",
                "fishing": "pomodoro",
                "fishing_reel": "building",
                "fishing_caught": "happy",
                "fishing_missed": "error"
            }
        }))
        .unwrap();

        assert!(normalize_user_settings(&mut settings));
        assert_eq!(settings.animations.fishing, "fishing");
        assert_eq!(settings.animations.fishing_reel, "reel");
        assert_eq!(settings.animations.fishing_caught, "caught");
        assert_eq!(settings.animations.fishing_missed, "missed");
    }

    #[test]
    fn openai_proxy_profile_writes_auth_token_not_api_key_to_claude_env() {
        let profile = LlmProfile {
            base_url: "https://example.com/v1/chat/completions".to_string(),
            auth_token: "local-token".to_string(),
            api_key: "upstream-key".to_string(),
            model: "gpt-test".to_string(),
            ..LlmProfile::default()
        };
        let env = profile.env_pairs();

        assert_eq!(
            env.iter()
                .find(|(key, _)| *key == "ANTHROPIC_BASE_URL")
                .map(|(_, value)| value.as_str()),
            Some("http://127.0.0.1:17388")
        );
        assert_eq!(
            env.iter()
                .find(|(key, _)| *key == "ANTHROPIC_AUTH_TOKEN")
                .map(|(_, value)| value.as_str()),
            Some("local-token")
        );
        assert!(!env.iter().any(|(key, _)| *key == "ANTHROPIC_API_KEY"));
        assert!(!env.iter().any(|(_, value)| value == "upstream-key"));
    }

    #[test]
    fn openai_proxy_uses_auth_token_when_api_key_field_is_empty() {
        let profile = LlmProfile {
            auth_token: "upstream-token".to_string(),
            ..LlmProfile::default()
        };

        assert_eq!(profile.openai_upstream_api_key(), "upstream-token");
    }

    #[test]
    fn openai_extra_body_accepts_json_object() {
        let profile = LlmProfile {
            openai_extra_body: r#"{"reasoning_effort":"xhigh","parallel_tool_calls":false}"#
                .to_string(),
            ..LlmProfile::default()
        };

        let body = profile.openai_extra_body_fields().unwrap();
        assert_eq!(body["reasoning_effort"], "xhigh");
        assert_eq!(body["parallel_tool_calls"], false);
    }

    #[test]
    fn openai_extra_body_accepts_assignment_lines() {
        let profile = LlmProfile {
            openai_extra_body: "model_reasoning_effort = \"xhigh\"\nservice_tier = flex"
                .to_string(),
            ..LlmProfile::default()
        };

        let body = profile.openai_extra_body_fields().unwrap();
        assert_eq!(body["reasoning_effort"], "xhigh");
        assert_eq!(body["service_tier"], "flex");
    }

    #[test]
    fn removing_active_profile_selects_neighbor() {
        let mut db = LlmProfileDb {
            profiles: vec![
                LlmProfile {
                    id: "first".to_string(),
                    ..LlmProfile::default()
                },
                LlmProfile {
                    id: "second".to_string(),
                    ..LlmProfile::default()
                },
                LlmProfile {
                    id: "third".to_string(),
                    ..LlmProfile::default()
                },
            ],
            active_profile_id: "second".to_string(),
        };

        let removed = db.remove_profile("second").unwrap();

        assert_eq!(removed.id, "second");
        assert_eq!(db.active_profile_id, "third");
        assert_eq!(
            db.profiles
                .iter()
                .map(|profile| profile.id.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "third"]
        );
    }

    #[test]
    fn removing_last_active_profile_clears_active_id() {
        let mut db = LlmProfileDb {
            profiles: vec![LlmProfile {
                id: "only".to_string(),
                ..LlmProfile::default()
            }],
            active_profile_id: "only".to_string(),
        };

        db.remove_profile("only");

        assert!(db.profiles.is_empty());
        assert!(db.active_profile_id.is_empty());
    }

    #[test]
    fn onboarding_file_is_created_when_missing() {
        let path = test_claude_json_path("missing");

        ensure_claude_onboarding_complete_at(&path).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text, "{\n  \"hasCompletedOnboarding\": true\n}\n");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn onboarding_field_is_appended_to_existing_object() {
        let path = test_claude_json_path("append");
        fs::write(&path, "{\n  \"projects\": {}\n}\n").unwrap();

        ensure_claude_onboarding_complete_at(&path).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            "{\n  \"projects\": {},\n  \"hasCompletedOnboarding\": true\n}\n"
        );
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            value.get("hasCompletedOnboarding").and_then(Value::as_bool),
            Some(true)
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn onboarding_field_is_set_true_when_present() {
        let path = test_claude_json_path("present-false");
        fs::write(
            &path,
            "{\n  \"hasCompletedOnboarding\": false,\n  \"projects\": {}\n}\n",
        )
        .unwrap();

        ensure_claude_onboarding_complete_at(&path).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            "{\n  \"hasCompletedOnboarding\": true,\n  \"projects\": {}\n}\n"
        );
        let _ = fs::remove_file(path);
    }

    fn test_claude_json_path(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!(
            "claudie-settings-{name}-{}-{}",
            std::process::id(),
            timestamp_millis()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir.join(".claude.json")
    }
}

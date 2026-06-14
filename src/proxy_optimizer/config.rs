use crate::settings::LlmProfile;

use super::OPTIMIZER_VERSION;

pub(super) const DEFAULT_SUMMARY_THRESHOLD_TOKENS: usize = 24_000;
pub(super) const DEFAULT_KEEP_RECENT_MESSAGES: usize = 12;
pub(super) const DEFAULT_KEEP_RECENT_TOKENS: usize = 10_000;
pub(super) const DEFAULT_TOOL_RESULT_LIMIT_TOKENS: usize = 3_000;
pub(super) const DEFAULT_TEXT_LIMIT_TOKENS: usize = 6_000;
pub(super) const DEFAULT_LOCAL_SUMMARY_TOKENS: usize = 2_000;
pub(super) const DEFAULT_CACHE_MAX_BYTES: u64 = 10 * 1024 * 1024;
pub(super) const DEFAULT_CHUNK_SIZE_MESSAGES: usize = 8;
pub(super) const DEFAULT_CHUNK_CACHE_TTL_HOURS: u64 = 168;
pub(super) const DEFAULT_CHUNK_CACHE_MAX_ENTRIES: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProxyOptimizationConfig {
    pub(crate) enabled: bool,
    pub(crate) summary_threshold_tokens: usize,
    pub(crate) keep_recent_messages: usize,
    pub(crate) keep_recent_tokens: usize,
    pub(crate) tool_result_limit_tokens: usize,
    pub(crate) text_limit_tokens: usize,
    pub(crate) local_summary_tokens: usize,
    pub(crate) cache_max_bytes: u64,
    pub(crate) chunk_summary_enabled: bool,
    pub(crate) chunk_size_messages: usize,
    pub(crate) chunk_cache_ttl_hours: u64,
    pub(crate) chunk_cache_max_entries: usize,
}

impl ProxyOptimizationConfig {
    pub(crate) fn from_profile(profile: &LlmProfile) -> Self {
        let mut config = Self::default();
        if let Some(value) = profile.extra_env_value("CLAUDIE_PROXY_OPTIMIZE") {
            config.enabled = !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            );
        }
        config.summary_threshold_tokens = env_usize(
            profile,
            "CLAUDIE_PROXY_SUMMARY_THRESHOLD",
            config.summary_threshold_tokens,
        );
        config.keep_recent_messages = env_usize(
            profile,
            "CLAUDIE_PROXY_KEEP_RECENT_MESSAGES",
            config.keep_recent_messages,
        );
        config.keep_recent_tokens = env_usize(
            profile,
            "CLAUDIE_PROXY_KEEP_RECENT_TOKENS",
            config.keep_recent_tokens,
        );
        config.tool_result_limit_tokens = env_usize(
            profile,
            "CLAUDIE_PROXY_TOOL_RESULT_LIMIT",
            config.tool_result_limit_tokens,
        );
        config.text_limit_tokens = env_usize(
            profile,
            "CLAUDIE_PROXY_TEXT_LIMIT",
            config.text_limit_tokens,
        );
        config.local_summary_tokens = env_usize(
            profile,
            "CLAUDIE_PROXY_LOCAL_SUMMARY_TOKENS",
            config.local_summary_tokens,
        );
        config.cache_max_bytes = env_u64(
            profile,
            "CLAUDIE_PROXY_CACHE_MAX_MB",
            config.cache_max_bytes / (1024 * 1024),
        )
        .saturating_mul(1024 * 1024);
        config.chunk_summary_enabled = env_bool(
            profile,
            "CLAUDIE_PROXY_CHUNK_SUMMARY",
            config.chunk_summary_enabled,
        );
        config.chunk_size_messages = env_usize(
            profile,
            "CLAUDIE_PROXY_CHUNK_SIZE_MESSAGES",
            config.chunk_size_messages,
        );
        config.chunk_cache_ttl_hours = env_u64(
            profile,
            "CLAUDIE_PROXY_CHUNK_CACHE_TTL_HOURS",
            config.chunk_cache_ttl_hours,
        );
        config.chunk_cache_max_entries = env_usize(
            profile,
            "CLAUDIE_PROXY_CHUNK_CACHE_MAX_ENTRIES",
            config.chunk_cache_max_entries,
        );
        config
    }

    pub(super) fn signature(&self) -> String {
        format!(
            "{OPTIMIZER_VERSION}:{}:{}:{}:{}:{}:{}:{}:{}",
            self.summary_threshold_tokens,
            self.keep_recent_messages,
            self.keep_recent_tokens,
            self.tool_result_limit_tokens,
            self.text_limit_tokens,
            self.local_summary_tokens,
            self.chunk_summary_enabled,
            self.chunk_size_messages
        )
    }
}

impl Default for ProxyOptimizationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            summary_threshold_tokens: DEFAULT_SUMMARY_THRESHOLD_TOKENS,
            keep_recent_messages: DEFAULT_KEEP_RECENT_MESSAGES,
            keep_recent_tokens: DEFAULT_KEEP_RECENT_TOKENS,
            tool_result_limit_tokens: DEFAULT_TOOL_RESULT_LIMIT_TOKENS,
            text_limit_tokens: DEFAULT_TEXT_LIMIT_TOKENS,
            local_summary_tokens: DEFAULT_LOCAL_SUMMARY_TOKENS,
            cache_max_bytes: DEFAULT_CACHE_MAX_BYTES,
            chunk_summary_enabled: true,
            chunk_size_messages: DEFAULT_CHUNK_SIZE_MESSAGES,
            chunk_cache_ttl_hours: DEFAULT_CHUNK_CACHE_TTL_HOURS,
            chunk_cache_max_entries: DEFAULT_CHUNK_CACHE_MAX_ENTRIES,
        }
    }
}

fn env_parse_filtered<T: std::str::FromStr>(
    profile: &LlmProfile,
    key: &str,
    default: T,
    accept: impl Fn(&T) -> bool,
) -> T {
    profile
        .extra_env_value(key)
        .and_then(|value| value.trim().parse::<T>().ok())
        .filter(accept)
        .unwrap_or(default)
}

fn env_usize(profile: &LlmProfile, key: &str, default: usize) -> usize {
    env_parse_filtered(profile, key, default, |value: &usize| *value > 0)
}

fn env_u64(profile: &LlmProfile, key: &str, default: u64) -> u64 {
    env_parse_filtered(profile, key, default, |value: &u64| *value > 0)
}

fn env_bool(profile: &LlmProfile, key: &str, default: bool) -> bool {
    profile
        .extra_env_value(key)
        .map(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            )
        })
        .unwrap_or(default)
}

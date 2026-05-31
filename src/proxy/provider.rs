use crate::settings::LlmProfile;

pub(super) const OPENAI_PROXY_COMPAT_PROMPT: &str = "\
Tool-result messages are observations from your prior tool calls; continue \
the task across multiple tool calls without re-asking the user. Prefer parallel \
tool calls when actions are independent (e.g. reading several files, staging \
multiple paths in one git command). If an edit tool fails, re-read the relevant \
file section before retrying.";

pub(super) const VISION_PLACEHOLDER_TEXT: &str = "[image attached, model does not support vision]";
pub(super) const TOOL_RESULT_IMAGE_PLACEHOLDER: &str = "[see attached image(s) in next message]";

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum ImageForwarding {
    Always,
    Auto,
    Never,
}

fn image_forwarding_mode(profile: &LlmProfile) -> ImageForwarding {
    match profile
        .extra_env_value("CLAUDIE_PROXY_FORWARD_IMAGES")
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("always" | "1" | "true" | "yes" | "on") => ImageForwarding::Always,
        Some("never" | "0" | "false" | "no" | "off") => ImageForwarding::Never,
        _ => ImageForwarding::Auto,
    }
}

fn model_supports_vision(model: &str) -> bool {
    let m = model.trim().to_ascii_lowercase();
    // OpenAI family
    m.contains("gpt-4o")
        || m.contains("gpt-4.1")
        || m.starts_with("o3")
        || m.contains("-o3-")
        || m.contains("gpt-5")
        || m.contains("vision")
        // Chinese providers (DeepSeek/Qwen/Kimi/GLM) - OpenRouter routes them with the same names.
        || m.contains("qwen-vl")
        || m.contains("qwen2-vl")
        || m.contains("qwen2.5-vl")
        || m.contains("glm-4v")
        || m.contains("moonshot-v1-vision")
        || m.contains("kimi-latest")
        || m.contains("deepseek-vl")
}

pub(super) fn model_is_reasoning(model: &str) -> bool {
    let m = model.trim().to_ascii_lowercase();
    // OpenAI o-series and thinking variants. Match standalone (o1*/o3*/o4*) and
    // namespaced (openai/o3-mini, group/o1-pro) forms used by OpenRouter / aggregators.
    let has_o_prefix = |needle: &str| {
        m.starts_with(needle)
            || m.contains(&format!("/{needle}"))
            || m.contains(&format!("-{needle}"))
    };
    has_o_prefix("o1-")
        || has_o_prefix("o1")
        || has_o_prefix("o3-")
        || has_o_prefix("o3")
        || has_o_prefix("o4-")
        || has_o_prefix("o4")
        || m.contains("-thinking")
        || m.contains("reasoning")
        // DeepSeek / Qwen / GLM reasoning models
        || m.contains("deepseek-r1")
        || m.contains("deepseek-reasoner")
        || m.contains("qwq")
        || m.contains("qwen3-thinking")
        || m.contains("glm-zero")
        || m.contains("glm-z1")
}

/// Blocklist of models that reject `tools`/`tool_choice` in OpenAI-format requests.
/// Conservative: default true; only deny what is known to break.
pub(super) fn model_supports_tools(model: &str) -> bool {
    let m = model.trim().to_ascii_lowercase();
    !(m.contains("deepseek-reasoner")
        || m.contains("deepseek-r1")
        || m.contains("qwq")
        || m.contains("glm-zero")
        || m.contains("glm-z1"))
}

pub(super) fn images_enabled_for(profile: &LlmProfile, model: &str) -> bool {
    match image_forwarding_mode(profile) {
        ImageForwarding::Always => true,
        ImageForwarding::Never => false,
        ImageForwarding::Auto => model_supports_vision(model),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Provider {
    OpenAI,
    Azure,
    DeepSeek,
    Qwen,
    Kimi,
    Glm,
    OpenRouter,
    Generic,
}

impl Provider {
    pub(super) fn detect(profile: &LlmProfile) -> Self {
        let url = profile.openai_chat_completions_url().to_ascii_lowercase();
        let Some(rest) = url.split("://").nth(1) else {
            return Provider::Generic;
        };
        let host = rest
            .split('/')
            .next()
            .unwrap_or("")
            .split(':')
            .next()
            .unwrap_or("");
        if host == "api.openai.com" {
            Provider::OpenAI
        } else if host.ends_with(".openai.azure.com") || host.ends_with(".azure.com") {
            Provider::Azure
        } else if host == "api.deepseek.com" || host.ends_with(".deepseek.com") {
            Provider::DeepSeek
        } else if host == "dashscope.aliyuncs.com" || host == "dashscope-intl.aliyuncs.com" {
            Provider::Qwen
        } else if host == "api.moonshot.cn" || host == "api.moonshot.ai" {
            Provider::Kimi
        } else if host == "open.bigmodel.cn" || host.ends_with(".bigmodel.cn") {
            Provider::Glm
        } else if host == "openrouter.ai" || host.ends_with(".openrouter.ai") {
            Provider::OpenRouter
        } else {
            Provider::Generic
        }
    }

    /// Should the compat prompt be added to system by default?
    /// All recognized commercial APIs do prefix-caching, so we keep the prefix clean
    /// for them. Only the Generic catch-all gets the compat sentence by default.
    pub(super) fn compat_prompt_default_on(self) -> bool {
        matches!(self, Provider::Generic)
    }

    /// Does this provider accept OpenAI's `reasoning_effort` field?
    /// DeepSeek/Qwen/Kimi/GLM do not have an equivalent param; sending it causes
    /// "unknown field" rejections on stricter backends.
    pub(super) fn accepts_reasoning_effort(self) -> bool {
        matches!(
            self,
            Provider::OpenAI | Provider::Azure | Provider::OpenRouter
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vision_extensions_detected() {
        for model in [
            "qwen-vl-max",
            "qwen2.5-vl-72b-instruct",
            "glm-4v-plus",
            "moonshot-v1-vision-preview",
            "kimi-latest",
            "deepseek-vl-7b-chat",
            "openrouter/qwen/qwen2-vl-72b",
        ] {
            assert!(
                model_supports_vision(model),
                "{model} should be detected as vision"
            );
        }
        assert!(!model_supports_vision("qwen-turbo"));
        assert!(!model_supports_vision("glm-4-plus"));
        assert!(!model_supports_vision("moonshot-v1-8k"));
    }

    #[test]
    fn reasoning_models_detected_across_providers() {
        for model in [
            "deepseek-r1",
            "deepseek-reasoner",
            "qwq-32b-preview",
            "qwen3-thinking-32b",
            "glm-zero-preview",
            "glm-z1-air",
            "o3-mini",
            "o1-preview",
            "deepseek/deepseek-r1", // OpenRouter style
        ] {
            assert!(model_is_reasoning(model), "{model} should be reasoning");
        }
        assert!(!model_is_reasoning("gpt-4o"));
        assert!(!model_is_reasoning("deepseek-chat"));
    }

    #[test]
    fn provider_detect_recognizes_known_hosts() {
        let cases = [
            ("https://api.openai.com/v1", Provider::OpenAI),
            (
                "https://my-resource.openai.azure.com/openai/deployments/gpt4o/chat/completions",
                Provider::Azure,
            ),
            ("https://api.deepseek.com/v1", Provider::DeepSeek),
            (
                "https://dashscope.aliyuncs.com/compatible-mode/v1",
                Provider::Qwen,
            ),
            (
                "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
                Provider::Qwen,
            ),
            ("https://api.moonshot.cn/v1", Provider::Kimi),
            ("https://api.moonshot.ai/v1", Provider::Kimi),
            ("https://open.bigmodel.cn/api/paas/v4", Provider::Glm),
            ("https://openrouter.ai/api/v1", Provider::OpenRouter),
            ("http://my-oneapi.local:3000/v1", Provider::Generic),
        ];
        for (url, expected) in cases {
            let profile = LlmProfile {
                base_url: url.to_string(),
                ..LlmProfile::default()
            };
            assert_eq!(
                Provider::detect(&profile),
                expected,
                "wrong provider for {url}"
            );
        }
    }
}

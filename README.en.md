# claudie

[中文](README.md) | English

`claudie` is a Windows-only lightweight desktop pet for Claude Code, built with Rust and a native Win32/GDI+ window. At runtime it runs the desktop UI, a synchronous `std::net::TcpListener` hook server, and a local Anthropic Messages compatible proxy that forwards Claude Code requests to OpenAI Chat Completions style providers.

The project intentionally avoids Electron, WebView, async runtimes, and web frameworks. Pet assets use a lightweight GIF directory, with one GIF mapped to each mood.

## Inspiration

claudie is inspired by [rullerzhou-afk/clawd-on-desk](https://github.com/rullerzhou-afk/clawd-on-desk) and [farion1231/cc-switch](https://github.com/farion1231/cc-switch).

## Features

- **Hook-driven pet states**: receives Claude Code HTTP hooks and switches pet states.

  | Event | Behavior |
  |-------|----------|
  | `SessionStart` | Return to idle |
  | `UserPromptSubmit` | Thinking |
  | `PreToolUse` | Start tool activity; write tools -> typing, shell tools -> building, read/search tools -> search |
  | `PostToolUse` | Finish tool activity |
  | `PostToolBatch` | Batch complete, refresh quota snapshot |
  | `PostToolUseFailure` / `StopFailure` / `PermissionDenied` | Error state |
  | `PermissionRequest` | Wait for Allow / Always / Deny in the pet UI |
  | `SubagentStart` / `TaskCreated` | Subagent working |
  | `SubagentStop` / `TaskCompleted` | Subagent done |
  | `PreCompact` / `PostCompact` | Context compression starts / finishes |
  | `Notification` / `Elicitation` | Notification prompt |
  | `WorktreeCreate` | Creating worktree |
  | `Stop` | Task complete |
  | `SessionEnd` | Session ended, clear pending interactions |

- **Permission requests**: intercepts `PermissionRequest` hooks and shows Allow / Always / Deny controls in the pet window.
- **Choice cards**: supports interactive `PreToolUse` choices for `AskUserQuestion` and `ExitPlanMode`, with options plus Submit / Cancel.
- **Hotkeys**: `Ctrl+Shift+Y` allows a permission or submits a choice; `Ctrl+Shift+N` denies a permission or cancels a choice.
- **Pomodoro timer**: built-in Pomodoro with Start / Stop / Pause / Resume / Skip and notifications on phase completion.
- **Fishing minigame**: click on the pet to start fishing through a "waiting → reeling → caught/missed" sequence; maintain tension by keeping the mouse inside a moving target zone during the reeling phase.
- **Pet interaction**: short left-click plays `wave` / `stretch`; click-and-move still drags the window; focus sessions can use the `pomodoro` animation.
- **Idle sleep**: auto-sleeps after inactivity and wakes on new activity.
- **Window and asset settings**: pet scaling, window position memory, GIF directory, and mood-to-GIF mapping.
- **Settings panel**: Basic, Pomodoro, LLM Profiles, and Stats tabs using native Slint windows.
- **LLM Profiles**: save official or custom LLM profiles, write the active profile to Claude Code settings, and switch quickly from the right-click menu.
- **Session ledger**: records daily prompts, tool categories, permission/choice counts, errors, completed focus sessions, and token usage; Stats shows today and the last 7 days.
- **Official usage monitoring**: real-time Claude Code 5h/7d usage limits in the right-click menu and Settings panel, with subscription plan detection (Max/Pro/Team) and auto-refresh via OAuth polling.
- **Secrets encrypted storage**: sensitive credentials encrypted/decrypted transparently with Windows DPAPI.
- **OpenAI-compatible proxy**: converts Claude Code Anthropic Messages requests to OpenAI Chat Completions with tools, streaming, image forwarding, reasoning output, parallel tool calls, tool-history fallback, context compression, summaries, and capability caching.
- **Windows-only**: ships the desktop pet UI, hook/proxy services, Settings panel, and permission/choice interactions for Windows.

## Quick Start

Run in development:

```powershell
cargo run --release
```

Normal startup listens on hook URL `http://127.0.0.1:17387/hook` and local proxy `http://127.0.0.1:17388`, then ensures Claude Code hooks point at the selected claudie port. Exiting the UI removes claudie-managed hooks.

Useful commands:

```powershell
cargo run --release -- --help
cargo run --release -- --port 17387
cargo run --release -- --install-claude-hooks
cargo run --release -- --uninstall-claude-hooks
cargo run --release -- --print-claude-settings
cargo run --release -- --install-claude-hooks --quiet
```

`--install` and `--uninstall` are accepted as short aliases. `--quiet` suppresses system notification popups during hook install/uninstall.

## OpenAI-Compatible API Proxy

claudie listens on:

```text
http://127.0.0.1:17388
```

In Settings -> LLM Profiles, create or edit a profile:

- Set `Base URL` to an OpenAI-compatible endpoint, such as `https://example.com/v1/chat/completions` or `https://example.com/v1`.
- Set `API key` to the upstream service key. If it is empty, the proxy uses `Auth token` as the upstream key.
- Set `Model` to a model supported by that service.
- Optionally set `OpenAI body` to extra request body fields, using either a JSON object or one `key=value` / `key: value` entry per line; for example `{"reasoning_effort":"xhigh"}` or `model_reasoning_effort = "xhigh"`.
- Use `Extra env` for one `KEY=VALUE` proxy switch or Claude Code environment variable per line.

After clicking `Use`, if the profile is OpenAI Chat Completions format, claudie writes Claude Code's `ANTHROPIC_BASE_URL` to the local proxy address. The upstream URL and key stay in the claudie profile. A `Base URL` containing `/chat/completions` enables the proxy automatically. If you enter an upstream root URL instead, add this to `Extra env`:

```text
CLAUDIE_API_FORMAT=openai
```

The proxy implements `POST /v1/messages`, `POST /v1/messages/count_tokens`, and `GET /v1/models`. `OpenAI body` is merged into the upstream chat completions request, but it cannot override claudie-managed `messages` or `stream` fields.

Current proxy capabilities:

- Non-streaming and streaming OpenAI responses are converted back to Anthropic Messages / SSE events.
- Anthropic tool use / tool result is converted to OpenAI `tools`, `tool_calls`, and `tool` messages.
- When tools are present and the model supports tools, `parallel_tool_calls=true` is sent by default; set `{"parallel_tool_calls": false}` to disable it.
- DeepSeek R1, QwQ, GLM-Zero, and similar reasoning streams are mapped to Anthropic thinking blocks.
- OpenAI/Azure/OpenRouter reasoning models auto-map Anthropic `thinking.budget_tokens` to `reasoning_effort`, unless the user explicitly sets it in `OpenAI body`.
- Image content forwarding is supported. By default claudie detects vision support from the model name; force it with `CLAUDIE_PROXY_FORWARD_IMAGES=always` or disable it with `CLAUDIE_PROXY_FORWARD_IMAGES=never`.
- Recognized OpenAI/Azure/DeepSeek/Qwen/Kimi/GLM/OpenRouter upstreams keep the compat prompt off by default; generic OneAPI/NewAPI-style upstreams get it by default. Control it with `CLAUDIE_PROXY_COMPAT_PROMPT=0/1`.
- If an upstream rejects native tool history, the proxy retries with text transcript mode and caches the result under `proxy_cache/capabilities/`.

Context optimization is enabled by default. claudie compresses very long tool results and text; when estimated input exceeds the threshold, it keeps recent messages and summarizes older history in chunks. The cache stores summary text or capability probe results only, not API keys or full original request bodies.

You can tune it from a profile's `Extra env`:

```text
CLAUDIE_PROXY_OPTIMIZE=0
CLAUDIE_PROXY_SUMMARY_MODE=local
CLAUDIE_PROXY_SUMMARY_THRESHOLD=24000
CLAUDIE_PROXY_KEEP_RECENT_MESSAGES=12
CLAUDIE_PROXY_KEEP_RECENT_TOKENS=10000
CLAUDIE_PROXY_TOOL_RESULT_LIMIT=3000
CLAUDIE_PROXY_TEXT_LIMIT=6000
CLAUDIE_PROXY_MAX_OUTPUT_TOKENS=32000
CLAUDIE_PROXY_LOCAL_SUMMARY_TOKENS=2000
CLAUDIE_PROXY_CACHE_MAX_MB=10
CLAUDIE_PROXY_SUMMARY_CACHE_TTL_HOURS=168
CLAUDIE_PROXY_SUMMARY_CACHE_MAX_ENTRIES=200
CLAUDIE_PROXY_CHUNK_SUMMARY=1
CLAUDIE_PROXY_CHUNK_SIZE_MESSAGES=8
CLAUDIE_PROXY_CHUNK_CACHE_TTL_HOURS=168
CLAUDIE_PROXY_CHUNK_CACHE_MAX_ENTRIES=200
CLAUDIE_PROXY_CAPABILITY_CACHE_TTL_HOURS=720
CLAUDIE_PROXY_CAPABILITY_CACHE_MAX_ENTRIES=200
CLAUDIE_PROXY_COMPAT_PROMPT=0
CLAUDIE_PROXY_FORWARD_IMAGES=auto
```

The default `CLAUDIE_PROXY_SUMMARY_MODE=local` uses local extractive summaries and does not call the upstream model for summarization. Set `CLAUDIE_PROXY_SUMMARY_MODE=model` to use upstream model summaries. If the summary request fails, claudie still forwards the request with long-content compression applied. Set `CLAUDIE_PROXY_MAX_OUTPUT_TOKENS=0` to disable the output-token cap.

## Project Structure

```text
src/
  main.rs                  CLI, startup flow, hook/proxy initialization, Windows UI entrypoint
  config.rs                Ports, window dimensions, menu IDs, overlay geometry, constants
  globals.rs               Process-wide OnceLock handles
  notifier.rs              Win32 message-box notification wrapper
  util.rs                  Arg parsing, paths, text shortening, UTF-16 helpers
  time_util.rs             Timestamp parsing, duration formatting, percentage extraction
  official_usage.rs        Claude Code official usage OAuth polling thread
  usage_display.rs         Official usage formatting and UI display adapter
  app/                     AppState, moods, permissions/choices, fishing, Pomodoro, stats domain state
  hooks/                   Claude Code hook server, event semantics, quota extraction, settings merge
  proxy/                   Anthropic Messages -> OpenAI Chat Completions proxy
  proxy_optimizer/         Long-context compression, chunk summaries, proxy cache
  settings/                User settings, LLM profiles, Secrets, Claude env integration, JSON storage
  ui/                      Win32/GDI+ main window, Slint settings/prompt windows, rendering
```

Key files:

- `src/app/fishing.rs`: fishing minigame state machine (waiting/reeling/caught/missed).
- `src/hooks/events.rs`: hook semantics, permission waiting, choice responses, and stats recording.
- `src/hooks/claude_settings.rs`: hook settings install, uninstall, merge, and backup.
- `src/hooks/quota.rs`: token, model, provider, quota, rate-limit, and official usage window capture.
- `src/proxy/request_conv.rs` / `response_conv.rs` / `streaming.rs`: request, response, and streaming conversion.
- `src/proxy/provider.rs`: provider/model capability detection, image forwarding, reasoning, and compat prompt policy.
- `src/proxy/tool_history.rs` / `capability_cache.rs`: tool-history transcript fallback and capability cache.
- `src/proxy_optimizer/config.rs` / `compress.rs` / `summary.rs` / `cache.rs`: context optimization and cache.
- `src/settings/mod.rs`: profiles, OpenAI body, Extra env, Claude settings writes, and path normalization.
- `src/settings/secrets.rs`: Windows DPAPI encrypted storage and custom serde serialization.
- `src/official_usage.rs`: Claude Code OAuth usage API polling and credential management.
- `src/usage_display.rs`: usage percentage bars, reset countdown, subscription plan formatting.
- `src/time_util.rs`: RFC3339/epoch timestamp parsing and percentage value extraction.
- `src/ui/window/mod.rs`: main window lifecycle, hotkeys, right-click menu, dragging, and profile menu.
- `src/ui/window/render.rs`: HUD, pet drawing, permission overlay, and choice cards.
- `src/ui/slint_views.rs` and `src/ui/settings_panel/`: Settings / Prompt declarations and controllers.

Other directories:

- `assets/claudie/`: bundled pet GIF animations.
- `assets/icon.*`, `assets/claudie.manifest`: application icons and Windows manifest.
- `packaging/`: Windows packaging scripts.

## Local Data

- `%USERPROFILE%\.claudie\settings.json`: asset path, GIF mapping, scale, sleep timeout, window position, and Pomodoro settings.
- `%USERPROFILE%\.claudie\llm_profiles.json`: LLM profiles, active profile, upstream auth, OpenAI body, and Extra env.
- `%USERPROFILE%\.claudie\secrets.json`: Windows DPAPI-encrypted sensitive credentials (API keys, OAuth tokens).
- `%USERPROFILE%\.claudie\daily_stats.json`: daily prompt, tool, permission/choice, error, focus-session, and token counters; keeps up to 45 days.
- `%USERPROFILE%\.claudie\proxy_summaries.json`: legacy single-block summary cache.
- `%USERPROFILE%\.claudie\proxy_cache\`: proxy cache directory containing `summaries/`, `chunks/`, and `capabilities/`.
- `%USERPROFILE%\.claude\settings.json`: Claude Code hook settings and claudie-managed LLM env.
- `%USERPROFILE%\.claude\settings.json.claudie.bak`: one-time backup created before the first Claude settings modification.

## Pet Assets

Bundled assets live in:

```text
assets/claudie/
  idle.gif
  thinking.gif
  typing.gif
  building.gif
  search.gif
  happy.gif
  error.gif
  sleeping.gif
  subagent.gif
  pomodoro.gif
  wave.gif
  stretch.gif
```

The Settings panel can adjust the GIF directory and file name mapped to each mood. When replacing art assets, keep the file name mapping consistent.

## Packaging

The Windows installer template is in `packaging/windows/claudie.iss`:

```powershell
powershell -ExecutionPolicy Bypass -File packaging\windows\build-installer.ps1
```

The output is `dist\claudie-setup.exe`.

## Verification

Before submitting changes, run at least:

```powershell
cargo fmt
cargo check
```

Run this when touching hook settings, quota extraction, LLM profiles, proxy conversion/streaming, context optimization, stats, Pomodoro, or pure domain rules:

```powershell
cargo test
```

For UI, hook, permission, settings, or proxy behavior changes, also run manually:

```powershell
cargo run --release
```

Check window position restore, right-click menu and LLM Profile switching, the four Settings tabs, left-click interaction and dragging, GIF loading, `POST /hook` state updates, permission/choice cards, Stats charts, and when relevant the local proxy, streaming conversion, `OpenAI body`, and context optimization.

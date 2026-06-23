# AGENTS.md

Quick-start notes for AI coding agents working on `claudie`.

## Project Summary

`claudie` is a Windows-only lightweight Rust desktop pet for Claude Code. It runs a native Win32/GDI+ always-on-top pet window, listens for Claude Code HTTP hook events, switches GIF animations based on activity, and lets the user answer permission or choice requests from the pet UI.

The project intentionally avoids Electron, WebView, async runtimes, and web frameworks. Runtime code is mostly a Win32 UI thread, a small synchronous `std::net::TcpListener` hook server, and a local Anthropic Messages compatible proxy that forwards to OpenAI Chat Completions style providers.

Non-Windows builds are not supported.

## Common Commands

```powershell
cargo fmt
cargo check
cargo test
cargo run --release
cargo run --release -- --help
cargo run --release -- --port 17387
cargo run --release -- --install-claude-hooks
cargo run --release -- --uninstall-claude-hooks
cargo run --release -- --print-claude-settings
cargo run --release -- --install-claude-hooks --quiet
```

Aliases accepted by the binary:

```powershell
cargo run --release -- --install
cargo run --release -- --uninstall
```

Build the Windows installer:

```powershell
powershell -ExecutionPolicy Bypass -File packaging\windows\build-installer.ps1
```

## Directory Map

- `src/main.rs`: CLI flags, shared `AppState`, hook/proxy startup, official usage polling, automatic hook ensure/cleanup, and Windows UI entrypoint.
- `src/config.rs`: ports, window dimensions, menu IDs, scale limits, and UI constants.
- `src/globals.rs`: process-wide `OnceLock` handles for shared app state and pet renderer.
- `src/notifier.rs`: Win32 message-box notification wrapper.
- `src/util.rs`: argument parsing, text shortening, markdown block parsing, and UTF-16 conversion.
- `src/i18n.rs`: lightweight dependency-free bilingual (zh/en) string table with process-wide language selection and Slint I18n global binding.
- `src/time_util.rs`: RFC3339/epoch timestamp parsing, duration formatting, and percentage value extraction.
- `src/official_usage.rs`: Claude Code OAuth usage API polling thread and credential management.
- `src/usage_display.rs`: official usage percentage bars, reset countdown, subscription plan formatting, and UI display adapter.
- `src/app/`: core domain state.
- `src/app/mod.rs`: `AppState`, `PetMood`, multi-session tracking (`sessions`, `focused_session_id`, `focus_pinned`, `transcript_path`, `turn_baseline_len`), pending permissions/choices, quota snapshots, fishing, pomodoro, stats, mood decay, and interrupt polling.
- `src/app/fishing.rs`: fishing minigame state machine with waiting/reeling/caught/missed phases and tension zone tracking.
- `src/app/pomodoro.rs`: focus/short-break/long-break timer state and transitions.
- `src/app/stats.rs`: local daily ledger, local date bucketing, tool classification, token counters, and fishing stats.
- `src/hooks/`: Claude Code hook facade.
- `src/hooks/mod.rs`: public re-exports and helper entry points.
- `src/hooks/server.rs`: minimal synchronous HTTP server accepting `POST /hook`.
- `src/hooks/events.rs`: Claude hook event semantics, mood transitions, permission waiting, hook responses, stats call sites, and terminal ESC interrupt detection (`transcript_recently_interrupted`, `poll_session_interrupts`).
- `src/hooks/quota.rs`: token, model, provider, quota, rate-limit, and official usage window capture from payloads/transcripts.
- `src/hooks/claude_settings.rs`: hook settings generation, merge, install, uninstall, and one-time backup handling.
- `src/proxy/`: local Anthropic Messages to OpenAI Chat Completions compatibility proxy.
- `src/proxy/mod.rs`: proxy server, active profile lookup, Bearer-token request auth (`proxy_auth_authorized`), request optimization, upstream call/retry flow, upstream `Retry-After` forwarding, and `/v1/messages` routing.
- `src/proxy/http.rs`: minimal HTTP request/response helpers, including extra-header responses.
- `src/proxy/provider.rs`: provider/model capability detection, image forwarding policy, reasoning model detection, and compat prompt defaults.
- `src/proxy/request_conv.rs`: Anthropic request to OpenAI request conversion, tools, images, reasoning effort, and `OpenAI body` merge.
- `src/proxy/response_conv.rs`: OpenAI response to Anthropic response conversion.
- `src/proxy/streaming.rs`: OpenAI SSE streaming to Anthropic streaming event translation.
- `src/proxy/tool_history.rs`: fallback text transcript mode for providers that reject native tool history.
- `src/proxy/capability_cache.rs`: upstream tool-history capability cache.
- `src/proxy/upstream.rs`: OpenAI-compatible upstream transport, error mapping, and 429/529 `Retry-After` capture.
- `src/proxy_optimizer/`: long-context compression, chunk summaries, and on-disk cache.
- `src/proxy_optimizer/mod.rs`: `optimize_openai_request` orchestration and shared helpers.
- `src/proxy_optimizer/config.rs`: `ProxyOptimizationConfig`, env parsing, and cache-key signature inputs.
- `src/proxy_optimizer/compress.rs`: in-place head/tail message compression and long-text trimming.
- `src/proxy_optimizer/summary.rs`: local extractive summaries and chunk generation.
- `src/proxy_optimizer/cache.rs`: chunk summary cache files, cache pruning, and FNV-1a keys.
- `src/proxy_optimizer/tests.rs`: optimizer integration-style unit tests.
- `src/settings/`: user settings, LLM profile persistence, secrets encryption, Claude env merge, and JSON storage.
- `src/settings/mod.rs`: settings/profile structs, normalization, OpenAI profile detection, extra env/body parsing, Claude settings writes, and onboarding helper.
- `src/settings/secrets.rs`: Windows DPAPI encrypted sensitive credential storage, custom serde serialization, and transparent load/save.
- `src/settings/storage.rs`: BOM-tolerant reads and pretty JSON writes.
- `src/ui/`: raw Win32 windowing, Slint dialog views, GIF rendering, window icons, folder picker, and theme tokens.
- `src/ui/mod.rs`: UI module structure and type re-exports.
- `src/ui/gif_animation.rs`: GDI+ GIF loading, frame delay sampling, mood transitions, and drawing.
- `src/ui/theme.rs`: shared color and radius tokens for the GDI-drawn HUD windows.
- `src/ui/slint_views.rs`: Slint declarations for Settings and Prompt windows.
- `src/ui/window_icon.rs`: Slint/Winit/Win32 icon bridge for auxiliary windows.
- `src/ui/window/mod.rs`: main transparent pet window, hotkeys, context menu, dragging/clicking, profile menu, official usage display, position persistence, system tray icon (`Shell_NotifyIconW`), multi-session switcher auxiliary window, ESC interrupt polling (`poll_session_interrupts` call in `WM_TIMER`), fishing minigame click trigger, and HUD anchoring to stable nominal pet rect.
- `src/ui/window/render.rs`: render snapshot, HUD, pet drawing, session switcher row rendering, and fishing HUD overlay.
- `src/ui/window_position.rs`: monitor-aware window centering and bounds helpers used by auxiliary Slint windows (permission/choice popup, settings).
- `src/ui/folder_dialog.rs`: Vista+ `IFileOpenDialog` folder picker for the Settings panel GIF directory selector.
- `src/ui/prompt_popup.rs`: Slint permission/choice popup snapshots and callbacks.
- `src/ui/settings_panel/`: Settings window Slint bridge, tab controllers, and live timers.
- `src/ui/settings_panel/mod.rs`: settings panel top-level refcell bridge and timer thread lifecycle.
- `src/ui/settings_panel/controller.rs`: settings controller dispatching, official profile usage refresh, and shared helpers.
- `src/ui/settings_panel/controller/basic.rs`: basic settings tab controller (GIF folder picker, scale, sleep timeout, session switcher).
- `src/ui/settings_panel/controller/pomodoro.rs`: pomodoro timer settings tab controller.
- `src/ui/settings_panel/controller/stats.rs`: daily stats display tab controller.
- `src/ui/settings_panel/controller/profiles.rs`: LLM profile management and official usage display tab controller.
- `build.rs`: compile-time Slint frontend compilation and resource embedding.
- `assets/claudie/`: bundled GIF pet moods.
- `packaging/`: Windows installer helpers.

## Runtime Files

- `%USERPROFILE%\.claudie\settings.json`: custom GIF folder (empty = bundled), scale, sleep timeout, session switcher toggle, window position, and pomodoro settings.
- `%USERPROFILE%\.claudie\llm_profiles.json`: saved LLM profiles, active profile id, upstream auth fields, OpenAI body, and extra env.
- `%USERPROFILE%\.claudie\daily_stats.json`: daily counters for prompts, tools, permissions, choices, errors, completed focus sessions, token usage, and fishing attempts.
- `%USERPROFILE%\.claudie\secrets.json`: Windows DPAPI-encrypted sensitive credentials (API keys, OAuth tokens).
- `%USERPROFILE%\.claudie\proxy_cache\`: OpenAI proxy cache directory:
  - `chunks/`: chunk summary cache JSON files.
  - `capabilities/`: upstream tool-history compatibility cache.
- `%USERPROFILE%\.claude\settings.json`: Claude Code hook settings and claudie-managed LLM env values.
- `%USERPROFILE%\.claude\settings.json.claudie.bak`: one-time backup created before claudie first modifies Claude settings.

## Architecture Notes

**State & data flow.** `AppState` (behind `Arc<Mutex<AppState>>` or `APP_STATE`) is the central mutable model. Access via `app_state.lock().unwrap()`. The hook server is on `DEFAULT_PORT` (17387), the OpenAI proxy on `DEFAULT_PROXY_PORT` (17388). Exiting uninstalls claudie-managed hooks from `settings.json`.

**Permissions & popups.** Popups fire only from `PermissionRequest`, never `PreToolUse`. `ExitPlanMode` and `AskUserQuestion` also arrive through `PermissionRequest`. Denying a normal tool writes `continue=false` + `interrupt=true` (matches terminal "No"). Denying `ExitPlanMode`/`AskUserQuestion` sends a plain deny so Claude Code falls back to its own terminal flow. `AskUserQuestion` uses a custom choice UI; `ExitPlanMode` renders the plan as markdown. Parallel tool events MUST NOT blanket-clear pending interactions — only precise (session, tool_use_id) sweeps. The primary "answered in terminal" signal is the hook socket closing; transcript denial markers are a fallback only. While a session still has a queued permission/choice, an out-of-band event (e.g. `Notification`) MUST NOT downgrade its row to "ready": `note_session_event` keeps the waiting state, so interaction resolution (`clear_stale_interactions`/sweep) runs before status recording. `Notification` never changes session status.

**Moods.** Use `AppState::set_mood`, `set_resting_mood`, `start_tool_activity`, `finish_tool_activity`, `start_subagent`, `finish_subagent` or `decay_mood`. `PetMood::Deny` (turn-interrupted-by-deny) is distinct from `Error`; `PetMood::Shrug` (recoverable `PostToolUseFailure`) is low-priority (50) resting with ~3s decay and no `ClaudeSessionStatus`. Only `StopFailure` maps to `Error`. Tool classification: edit/write → `Typing`, shell → `Building`, read/search → `Search`.

**Proxy & profiles.** `src/proxy/` handles transport and format conversion; `src/proxy_optimizer/` handles context compression/cache; `src/settings/` handles profiles and env. Bearer token auth (`proxy_auth_token`), upstream 429/529 forward `Retry-After`, transient failures → 529. Compat prompt auto-disabled for known OpenAI/Azure/DeepSeek/Qwen/Kimi/GLM/OpenRouter hosts. `[1m]` suffix stripped before upstream. `Fetch models` hits `/v1/models` with Bearer auth. Official profile reuses OAuth token; never sent to third-party hosts.

**UI.** Raw Win32, unsafe — keep close to FFI boundaries. Settings/prompt windows use Slint (`src/ui/slint_views.rs`), software renderer only. State changes from callbacks/background threads must call `redraw_after`/`request_redraw`. Tray icon (`Shell_NotifyIconW`, id `1`) mirrors the context menu. Use `util::wide` for Win32 strings. Do not block the UI thread. HUD sub-windows (pomodoro, fishing, session switcher) anchor to the stable nominal pet box (`pet_nominal_width`/`pet_nominal_height`, `nominal_pet_screen_rect`), not the variable visible rect, so they don't shift when mood GIF bounds change.

**Sessions & persistence.** Multiple Claude Code sessions keyed by id in `sessions` map; `focused_session_id` drives mood/HUD. Focus behavior: new `SessionStart`/`SessionResume` auto-focuses if current is idle (`acquire_focus_for_new_session`); switcher click pins focus (`focus_pinned`) until that session ends; unpinned focus auto-follows the most recently working session when idle (`most_recently_working_session`); a waiting permission/choice always preempts. ESC interrupt is detected by polling transcript file tail for `[Request interrupted by user]` markers past a per-session turn baseline (`transcript_path`, `turn_baseline_len`, `mark_session_interrupted`); the marker lives in the `text` field of a content-block array, so detection matches `"text":"[request interrupted by user` (compacted) via `TERMINAL_DENIAL_MARKERS`, not a bare `content` string. A force-quit fires no `SessionEnd`, so `prune_inactive_sessions` (UI tick) drops a live-but-idle session silent past `STALE_SESSION_TIMEOUT` (10 min); actively-working or popup-blocked sessions are never pruned. An `Ended` session is only revived by `SessionStart`/`SessionResume`/`UserPromptSubmit`, not by trailing hooks. Session switcher row (default on) renders one row per session, hidden when only one is active. Settings merge into `settings.json` preserving unrelated fields. Secrets use Windows DPAPI (user-scoped). Official usage polled every 60s from Claude Code OAuth API. Stats via `src/app/stats.rs`; UI only displays counters.

**Modules.** Hook events → `src/hooks/events.rs`, not the HTTP parser. Keep focus: proxy transport/conversion in `src/proxy/`, cache in `src/proxy_optimizer/`, profiles/env in `src/settings/`.

## Maintenance Guidelines

- Avoid new dependencies unless they remove real complexity.
- Preserve unrelated user settings in `~/.claude/settings.json`; merge only claudie-managed hook/env fields.
- Reuse `src/settings/storage.rs` for new JSON state files.
- Keep pet resources lightweight: one GIF per mood in `assets/claudie/`.
- Prefer focused changes over broad refactors because Win32 regressions can be subtle.

## Verification Checklist

Always run: `cargo fmt && cargo check`. Run `cargo test` when touching domain logic (hooks, proxy, optimizer, settings, stats, pomodoro, fishing). For UI/hook/permission changes, run `cargo run --release` and verify the pet window, tray icon, session switcher, settings tabs, GIF loading, permission popups, fishing minigame, and profile switching work as expected.

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
- `src/time_util.rs`: RFC3339/epoch timestamp parsing, duration formatting, and percentage value extraction.
- `src/official_usage.rs`: Claude Code OAuth usage API polling thread and credential management.
- `src/usage_display.rs`: official usage percentage bars, reset countdown, subscription plan formatting, and UI display adapter.
- `src/app/`: core domain state.
- `src/app/mod.rs`: `AppState`, `PetMood`, multi-session tracking (`sessions`, `focused_session_id`, switcher ordering), pending permissions/choices, quota snapshots, fishing, pomodoro, stats, and mood decay.
- `src/app/fishing.rs`: fishing minigame state machine with waiting/reeling/caught/missed phases and tension zone tracking.
- `src/app/pomodoro.rs`: focus/short-break/long-break timer state and transitions.
- `src/app/stats.rs`: local daily ledger, local date bucketing, tool classification, token counters, and fishing stats.
- `src/hooks/`: Claude Code hook facade.
- `src/hooks/mod.rs`: public re-exports and helper entry points.
- `src/hooks/server.rs`: minimal synchronous HTTP server accepting `POST /hook`.
- `src/hooks/events.rs`: Claude hook event semantics, mood transitions, permission waiting, hook responses, and stats call sites.
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
- `src/ui/`: raw Win32 windowing, Slint dialog views, GIF rendering, window icons, and theme tokens.
- `src/ui/mod.rs`: UI module structure and type re-exports.
- `src/ui/gif_animation.rs`: GDI+ GIF loading, frame delay sampling, mood transitions, and drawing.
- `src/ui/theme.rs`: shared color and radius tokens for the GDI-drawn HUD windows.
- `src/ui/slint_views.rs`: Slint declarations for Settings and Prompt windows.
- `src/ui/window_icon.rs`: Slint/Winit/Win32 icon bridge for auxiliary windows.
- `src/ui/window/mod.rs`: main transparent pet window, hotkeys, context menu, dragging/clicking, profile menu, official usage display, position persistence, system tray icon (`Shell_NotifyIconW`), multi-session switcher auxiliary window, and fishing minigame click trigger.
- `src/ui/window/render.rs`: render snapshot, HUD, pet drawing, session switcher row rendering, and fishing HUD overlay.
- `src/ui/window_position.rs`: monitor-aware window centering and bounds helpers used by auxiliary Slint windows (permission/choice popup, settings).
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

- `AppState` is the central mutable model. Access it through `Arc<Mutex<AppState>>` or `APP_STATE`.
- Normal startup starts the hook server on `DEFAULT_PORT` (`17387`), starts the OpenAI proxy on `DEFAULT_PROXY_PORT` (`17388`), then ensures Claude Code hooks point at the selected hook port. Exiting the UI uninstalls claudie-managed hooks.
- Installed hook events are defined in `src/hooks/claude_settings.rs`; event semantics belong in `src/hooks/events.rs`. The event handler also tolerates common camelCase field variants and some compatibility event names.
- Mood transitions should go through `AppState::set_mood`, `set_resting_mood`, `start_tool_activity`, `finish_tool_activity`, `start_subagent`, `finish_subagent`, or `decay_mood` so timestamps and renderer priority stay correct. `PetMood::Deny` (and `ClaudeSessionStatus::Denied`) is a separate mood from `Error`, fired when a permission deny interrupts the turn — it has its own `deny.gif`, color, and switcher symbol.
- `PreToolUse` mood classification treats edit/write tools as `Typing`, shell tools as `Building`, read/search tools as `Search`, and `Task` based on task text.
- Popups are created only by the blocking `PermissionRequest` hook; `PreToolUse` is never intercepted. Claude Code keeps its own terminal prompt visible while the hook waits, so the claudie popup and the terminal options stay usable side by side, and auto/bypass permission modes (which never fire `PermissionRequest`) never trigger popups. `ExitPlanMode` and `AskUserQuestion` also arrive through this event.
- Permission requests are `PendingPermission` values and complete through `decide_current_permission`. Denying a normal tool writes `continue=false` + `stopReason` and `interrupt=true` — this matches a terminal "No" (interrupt the turn) rather than feeding a tool-error back to the model. Denying `ExitPlanMode`/`AskUserQuestion` sends a plain deny instead, so Claude Code falls back to its own terminal flow (keep planning / ask in terminal). Denial also calls `finish_session_work` so the pet does not stay in a working mood when the tool will not run.
- `AskUserQuestion` permission requests do not use the generic popup: `handle_question_request` parses `tool_input.questions` into a `PendingChoice` (appending an "Other..." free-text option per question) and the Slint questions UI renders selectable options. Submitting answers the hook with `behavior=allow` plus `updatedInput` carrying an `answers` map (question text → chosen labels, comma-joined), matching what Claude Code's own dialog sends; cancelling sends the plain fall-back-to-terminal deny. Unparseable question input falls through to the generic permission popup. `ExitPlanMode` stays on the permission popup and renders the `plan` field as markdown.
- Tool events (`PreToolUse`, `PostToolUse`) MUST NOT blanket-clear pending interactions — parallel tools completing must not accidentally close a popup belonging to a different tool call. Precise sweeps are allowed: `PostToolUse`/`PostToolUseFailure` resolves the pending permission with the same session and `tool_use_id`, plus any pending `ExitPlanMode` popups and question choices in that session (forward progress means they were answered in the terminal); `PreToolUse` of another tool resolves pending `ExitPlanMode` popups.
- `session_status_for_event` returns `Option`; late events such as `SubagentStop`/`TaskCompleted` for an already-ended session do not roll the session back into `Streaming`.
- The primary "answered in the terminal" signal is the hook socket closing — Claude Code aborts the pending `PermissionRequest` once the terminal dialog is resolved, and the wait loop probes the connection every 250ms. The transcript-denial detector in `src/hooks/events.rs` is a fallback and only matches Claude Code's own structured denial markers in transcript lines, so reading source code or logs that mention "permission/denied" does not falsely dismiss popups.
- Claude Code may run several sessions in parallel. `AppState` keeps a `sessions` map keyed by session id with status (`Streaming`, `WaitingPermission`, `WaitingChoice`, `Idle`, …), CWD, and last-activity timestamp; `focused_session_id` drives which session the pet mood and HUD reflect. The session switcher auxiliary window (toggle: `Settings -> Basic -> show_session_switcher`, default on) renders one row per session and lets the user scroll/click to switch focus. Hide it when only one session is active.
- Daily stats should be recorded via `AppState` methods and stored by `src/app/stats.rs`; UI code should display counters, not derive business stats.
- Keep the hook server small and synchronous. Put Claude-event behavior in `src/hooks/events.rs`, not in the HTTP parser.
- Keep OpenAI proxy transport and Anthropic/OpenAI conversion in `src/proxy/`; keep context optimization and cache logic in `src/proxy_optimizer/`; keep profile persistence and Claude env behavior in `src/settings/`.
- The proxy defaults `parallel_tool_calls=true` when tools are present and the model supports tools. Users can set `{"parallel_tool_calls": false}` in `OpenAI body` for older/smaller upstreams.
- The proxy authenticates incoming requests against a Bearer token derived from the active profile (`proxy_auth_token`); the OAuth token Claude Code sends via `ANTHROPIC_AUTH_TOKEN` is the expected value. Requests without a matching token get `401 Unauthorized`.
- Upstream errors map to Anthropic-style responses. 429/529 from the upstream forward the upstream `Retry-After` header back to Claude Code so its native retry/backoff kicks in; transient upstream unavailability is reported as HTTP 529 (not 503) to align with Anthropic's overload semantics.
- Recognized OpenAI/Azure/DeepSeek/Qwen/Kimi/GLM/OpenRouter hosts keep the compat prompt off by default; Generic providers get it unless `CLAUDIE_PROXY_COMPAT_PROMPT=0` is set.
- Vision/image forwarding is auto-detected by model name and can be forced with `CLAUDIE_PROXY_FORWARD_IMAGES=always` or disabled with `CLAUDIE_PROXY_FORWARD_IMAGES=never`.
- Each model field (`model` / `opus_model` / `sonnet_model` / `haiku_model`) has its own `*_1m` opt-in. When set, `apply_1m_suffix` appends `[1m]` to the id written to Claude Code; the proxy strips it via `strip_1m_suffix` before forwarding upstream. Loaders detect the suffix on saved profiles and restore the toggle.
- The Settings → LLM Profiles `Fetch models` button hits the active profile's upstream `/v1/models` (Bearer-only OpenAI shape, with `/anthropic` and `/v{N}` path normalization, and a fallback to the site root). Real `api.anthropic.com` uses `x-api-key`; the official profile reuses its OAuth token, and OAuth tokens are only sent to the official profile to avoid leaking to third parties. Empty results fall back to a grey placeholder dropdown rather than an empty popup.
- UI code uses raw Win32 handles and unsafe calls. Keep unsafe usage close to Win32 boundaries and prefer small helper functions for repeated patterns.
- A Win32 tray icon (`Shell_NotifyIconW`, id `1`, callback `WM_APP+1`) is installed on window creation and removed on destroy; the tray menu mirrors the right-click context menu.
- Settings and prompt windows use Slint declarations in `src/ui/slint_views.rs`; keep Rust callback/state logic in `src/ui/settings_panel/` and `src/ui/prompt_popup.rs`. Slint runs the software renderer only (no femtovg/OpenGL backend); when state changes from a callback or background thread, wrap it in `redraw_after`/`redraw_after_arg` or call `request_redraw` so the panel actually repaints.
- Use `util::wide` for strings passed to Win32 APIs.
- Do not block the UI thread with noticeable network or filesystem work.
- When settings change, update both persisted files and in-memory `AppState`.
- Secrets in `src/settings/secrets.rs` use Windows DPAPI (`CryptProtectData`/`CryptUnprotectData`) for encryption at rest. The `Secrets` struct wraps a `serde_json::Value` and serializes encrypted. Secrets are scoped to the current Windows user — they cannot be decrypted by another user or on another machine.
- Official usage polling runs in a dedicated thread spawned at startup. It polls the Claude Code OAuth API (`/api/v2/organizations/.../usage` and `/api/v2/billing/subscription`) every 60 seconds with a cached `token.json` from the Claude Code config directory. Results are stored in `AppState` and displayed in the Settings panel Usage tab and the right-click menu live quota bar.
- The fishing minigame in `src/app/fishing.rs` is a turn-based state machine: `Inactive → Waiting → Reeling → Caught | Missed`. Left-click on the pet starts the game. During the `Reeling` phase, the player must click when the tension bar is within the green zone. Each phase maps to its own GIF file by the fixed naming convention (`fishing.gif` / `reel.gif` / `caught.gif` / `missed.gif`), resolved per-mood from the user's GIF folder with a fallback to the bundled default. Fishing stats are recorded in `daily_stats.json` alongside tool usage stats.

## Maintenance Guidelines

- Avoid new dependencies unless they remove real complexity.
- Preserve unrelated user settings in `~/.claude/settings.json`; merge only claudie-managed hook/env fields.
- Reuse `src/settings/storage.rs` for new JSON state files.
- Keep pet resources lightweight: one GIF per mood in `assets/claudie/`.
- Prefer focused changes over broad refactors because Win32 regressions can be subtle.

## Verification Checklist

Run at least:

```powershell
cargo fmt
cargo check
```

Run `cargo test` when touching hook settings, quota extraction, LLM profile logic, proxy conversion/streaming, optimizer/cache logic, stats, pomodoro, or other pure domain rules.

For UI, hook, permission, settings, or proxy behavior changes, also run:

```powershell
cargo run --release
```

Manual checks worth doing for relevant changes:

- Pet window opens without a console in release builds and restores its last saved position.
- A tray icon appears in the system notification area; clicking/right-clicking it surfaces the same menu as the pet's right-click menu.
- When two or more Claude Code sessions are active, the session switcher row appears alongside the pet; scrolling/clicking changes the focused session and the pet mood/HUD follow.
- Right-click menu opens Settings, Pomodoro actions, LLM Profile submenu, and Exit.
- Settings tabs switch cleanly between Basic, Pomodoro, LLM Profiles, and Stats.
- GIF resources load from configured or bundled assets.
- Short left-click plays an interaction animation; click-and-move still drags the window.
- `POST /hook` updates mood/events and stats.
- Permission requests show Allow, Always, and Deny controls in claudie while the same options stay usable in the Claude Code terminal; answering on either side resolves both (the popup closes when the terminal answers). Deny interrupts the current turn for normal tools; for plan approval and AskUserQuestion it falls back to the terminal instead. A parallel tool finishing in the meantime does not dismiss the popup.
- `AskUserQuestion` (e.g. plan-mode questions) shows the questions with selectable option rows plus an "Other..." free-text answer; picking an option and submitting in claudie answers the question in Claude Code, while the terminal selector stays usable and answering there closes the popup. `ExitPlanMode` shows the plan as rendered markdown, not raw JSON.
- With auto-accept or bypass permissions enabled, claudie shows no popups at all.
- LLM Profiles can save/use/import/delete profiles and switch from the right-click menu.
- OpenAI-format profiles route Claude Code through `http://127.0.0.1:17388`, forward `OpenAI body`, stream correctly when requested, and compress/summarize long conversations without losing recent messages.
- Stats records daily counters and token usage in `%USERPROFILE%\.claudie\daily_stats.json`, and the Stats tab charts do not overflow.
- Right-click menu shows official Claude Code usage (5h/7d) with percentage bars; basic users see the upgrade link.
- Left-clicking the pet triggers the fishing minigame: waiting phase, click again to reel, keep tension in the green zone to catch.
- Secrets are encrypted with DPAPI and survive app restart; importing an API key stores it encrypted.

# AGENTS.md

Quick-start notes for AI coding agents working on `claudie`.

## Project Summary

`claudie` is a lightweight Rust desktop pet for Claude Code. On Windows it runs a native Win32/GDI+ always-on-top pet window, listens for Claude Code HTTP hook events, switches GIF animations based on activity, and lets the user answer permission or choice requests from the pet UI.

The project intentionally avoids Electron, WebView, async runtimes, and web frameworks. Runtime code is mostly a Win32 UI thread, a small synchronous `std::net::TcpListener` hook server, and a local Anthropic Messages compatible proxy that forwards to OpenAI Chat Completions style providers.

Windows has the full UI. macOS/Linux builds run the hook/proxy services and CLI hook management only; permission requests are denied immediately because there is no desktop interaction UI.

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

- `src/main.rs`: CLI flags, shared `AppState`, hook/proxy startup, automatic hook ensure/cleanup, and platform entrypoint.
- `src/config.rs`: ports, window dimensions, menu IDs, overlay geometry, scale limits, permission timeout, and UI constants.
- `src/globals.rs`: process-wide `OnceLock` handles for shared app state and pet renderer.
- `src/notifier.rs`: platform notification/message-box wrapper.
- `src/util.rs`: argument parsing, path helpers, text shortening, and UTF-16 conversion.
- `src/app/`: core domain state.
- `src/app/mod.rs`: `AppState`, `PetMood`, sessions, pending permissions/choices, quota snapshots, pomodoro, stats, and mood decay.
- `src/app/pomodoro.rs`: focus/short-break/long-break timer state and transitions.
- `src/app/stats.rs`: local daily ledger, local date bucketing, tool classification, and token counters.
- `src/hooks/`: Claude Code hook facade.
- `src/hooks/server.rs`: minimal synchronous HTTP server accepting `POST /hook`.
- `src/hooks/events.rs`: Claude hook event semantics, mood transitions, permission/choice waiting, hook responses, and stats call sites.
- `src/hooks/quota.rs`: token, model, provider, quota, and rate-limit extraction from payloads/transcripts.
- `src/hooks/claude_settings.rs`: hook settings generation, merge, install, uninstall, and one-time backup handling.
- `src/proxy/`: local Anthropic Messages to OpenAI Chat Completions compatibility proxy.
- `src/proxy/mod.rs`: proxy server, active profile lookup, optimization handoff, upstream call/retry flow, and `/v1/messages` routing.
- `src/proxy/http.rs`: minimal HTTP request/response helpers.
- `src/proxy/provider.rs`: provider/model capability detection, image forwarding policy, reasoning model detection, and compat prompt defaults.
- `src/proxy/request_conv.rs`: Anthropic request to OpenAI request conversion, tools, images, reasoning effort, and `OpenAI body` merge.
- `src/proxy/response_conv.rs`: OpenAI response to Anthropic response conversion.
- `src/proxy/streaming.rs`: OpenAI SSE streaming to Anthropic streaming event translation.
- `src/proxy/tool_history.rs`: fallback text transcript mode for providers that reject native tool history.
- `src/proxy/capability_cache.rs`: upstream tool-history capability cache.
- `src/proxy/upstream.rs`: OpenAI-compatible upstream transport and error mapping.
- `src/proxy_optimizer/`: long-context compression, chunk summaries, and on-disk cache.
- `src/proxy_optimizer/mod.rs`: `optimize_openai_request` orchestration and shared helpers.
- `src/proxy_optimizer/config.rs`: `ProxyOptimizationConfig`, `SummaryMode`, env parsing, and cache-key signature inputs.
- `src/proxy_optimizer/compress.rs`: in-place head/tail message compression and long-text trimming.
- `src/proxy_optimizer/summary.rs`: local extractive summaries, optional model summary request building, and chunk generation.
- `src/proxy_optimizer/cache.rs`: legacy summary cache, summary/chunk cache files, cache pruning, and FNV-1a keys.
- `src/proxy_optimizer/tests.rs`: optimizer integration-style unit tests.
- `src/settings/`: user settings, LLM profile persistence, Claude env merge, and JSON storage.
- `src/settings/mod.rs`: settings/profile structs, normalization, OpenAI profile detection, extra env/body parsing, Claude settings writes, and onboarding helper.
- `src/settings/storage.rs`: BOM-tolerant reads and pretty JSON writes.
- `src/ui/gif_animation.rs`: GDI+ GIF loading, frame delay sampling, mood transitions, and drawing.
- `src/ui/theme.rs`: shared colors, radii, and typography tokens for settings and overlays.
- `src/ui/window/mod.rs`: main transparent pet window, hotkeys, context menu, dragging/clicking, profile menu, and position persistence.
- `src/ui/window/render.rs`: render snapshot, HUD, pet drawing, permission overlay, and choice card drawing.
- `src/ui/slint_views.rs`: Slint declarations for Settings and Prompt windows.
- `src/ui/settings_panel/`: Slint Settings lifecycle, callback wiring, and tab controllers.
- `src/ui/prompt_popup.rs`: Slint permission/choice popup snapshots and callbacks.
- `src/ui/window_icon.rs`: Slint/Winit/Win32 icon bridge for auxiliary windows.
- `assets/claudie/`: bundled GIF pet moods.
- `packaging/`: Windows installer and Unix user-level install helpers.

## Runtime Files

- `%USERPROFILE%\.claudie\settings.json`: pet asset base directory, GIF directory, animation mapping, scale, sleep timeout, window position, and pomodoro settings.
- `%USERPROFILE%\.claudie\llm_profiles.json`: saved LLM profiles, active profile id, upstream auth fields, OpenAI body, and extra env.
- `%USERPROFILE%\.claudie\daily_stats.json`: daily counters for prompts, tools, permissions, choices, errors, completed focus sessions, and token usage.
- `%USERPROFILE%\.claudie\proxy_summaries.json`: legacy single-block summary cache.
- `%USERPROFILE%\.claudie\proxy_cache\`: OpenAI proxy cache directory:
  - `summaries/`: single-block summary cache JSON files.
  - `chunks/`: chunk summary cache JSON files.
  - `capabilities/`: upstream tool-history compatibility cache.
- `%USERPROFILE%\.claude\settings.json`: Claude Code hook settings and claudie-managed LLM env values.
- `%USERPROFILE%\.claude\settings.json.claudie.bak`: one-time backup created before claudie first modifies Claude settings.

## Architecture Notes

- `AppState` is the central mutable model. Access it through `Arc<Mutex<AppState>>` or `APP_STATE`.
- Normal startup starts the hook server on `DEFAULT_PORT` (`17387`), starts the OpenAI proxy on `DEFAULT_PROXY_PORT` (`17388`), then ensures Claude Code hooks point at the selected hook port. On Windows, exiting the UI uninstalls claudie-managed hooks.
- Installed hook events are defined in `src/hooks/claude_settings.rs`; event semantics belong in `src/hooks/events.rs`. The event handler also tolerates common camelCase field variants and some compatibility event names.
- Mood transitions should go through `AppState::set_mood`, `set_resting_mood`, `start_tool_activity`, `finish_tool_activity`, `start_subagent`, `finish_subagent`, or `decay_mood` so timestamps and renderer priority stay correct.
- `PreToolUse` mood classification treats edit/write tools as `Typing`, shell tools as `Building`, read/search tools as `Search`, and `Task` based on task text.
- Permission requests are `PendingPermission` values and complete through `decide_current_permission`. Choice requests are `PendingChoice` values and complete through `submit_current_choice` or `deny_current_choice`.
- Daily stats should be recorded via `AppState` methods and stored by `src/app/stats.rs`; UI code should display counters, not derive business stats.
- Keep the hook server small and synchronous. Put Claude-event behavior in `src/hooks/events.rs`, not in the HTTP parser.
- Keep OpenAI proxy transport and Anthropic/OpenAI conversion in `src/proxy/`; keep context optimization and cache logic in `src/proxy_optimizer/`; keep profile persistence and Claude env behavior in `src/settings/`.
- The proxy defaults `parallel_tool_calls=true` when tools are present and the model supports tools. Users can set `{"parallel_tool_calls": false}` in `OpenAI body` for older/smaller upstreams.
- Recognized OpenAI/Azure/DeepSeek/Qwen/Kimi/GLM/OpenRouter hosts keep the compat prompt off by default; Generic providers get it unless `CLAUDIE_PROXY_COMPAT_PROMPT=0` is set.
- Vision/image forwarding is auto-detected by model name and can be forced with `CLAUDIE_PROXY_FORWARD_IMAGES=always` or disabled with `CLAUDIE_PROXY_FORWARD_IMAGES=never`.
- UI code uses raw Win32 handles and unsafe calls. Keep unsafe usage close to Win32 boundaries and prefer small helper functions for repeated patterns.
- Settings and prompt windows use Slint declarations in `src/ui/slint_views.rs`; keep Rust callback/state logic in `src/ui/settings_panel/` and `src/ui/prompt_popup.rs`.
- Use `util::wide` for strings passed to Win32 APIs.
- Do not block the UI thread with noticeable network or filesystem work.
- When settings change, update both persisted files and in-memory `AppState`.

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
- Right-click menu opens Settings, Pomodoro actions, LLM Profile submenu, and Exit.
- Settings tabs switch cleanly between Basic, Pomodoro, LLM Profiles, and Stats.
- GIF resources load from configured or bundled assets.
- Short left-click plays an interaction animation; click-and-move still drags the window.
- `POST /hook` updates mood/events and stats.
- Permission requests show Allow, Always, and Deny controls on Windows.
- Choice requests show options plus Submit and Cancel controls.
- LLM Profiles can save/use/import/delete profiles and switch from the right-click menu.
- OpenAI-format profiles route Claude Code through `http://127.0.0.1:17388`, forward `OpenAI body`, stream correctly when requested, and compress/summarize long conversations without losing recent messages.
- Stats records daily counters and token usage in `%USERPROFILE%\.claudie\daily_stats.json`, and the Stats tab charts do not overflow.

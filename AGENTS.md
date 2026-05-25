# AGENTS.md

This file is the quick-start map for AI coding agents working on `claudie`.

## Project Summary

`claudie` is a Windows-first Rust desktop pet for Claude Code. It runs a small Win32/GDI+ window, listens for Claude Code HTTP hook events, switches GIF animations based on activity, and can answer Claude Code permission and choice requests from the pet UI.

The project intentionally avoids Electron, WebView, async runtimes, and web frameworks. Runtime code is mostly a Win32 UI thread, a small synchronous `std::net::TcpListener` hook server, and a local Anthropic Messages to OpenAI Chat Completions proxy.

## Common Commands

```powershell
cargo fmt
cargo check
cargo test
cargo run --release
cargo run --release -- --install-claude-hooks
cargo run --release -- --uninstall-claude-hooks
cargo run --release -- --print-claude-settings
```

Use `--port <number>` after the binary args to change the hook port:

```powershell
cargo run --release -- --port 17387
```

## Directory Map

- `src/main.rs`: CLI flags, app bootstrap, shared `AppState`, hook/profile startup, and platform entrypoint.
- `src/app/`: core application domain.
- `src/app/mod.rs`: `AppState`, `PetMood`, sessions, quota stats, pending permissions/choices, speech timing, pomodoro integration, and mood decay.
- `src/app/pomodoro.rs`: lightweight focus timer state and transitions.
- `src/hooks/mod.rs`: hook subsystem facade and public re-exports.
- `src/hooks/events.rs`: Claude Code event processing, mood transitions, permission/choice waiting, and hook responses.
- `src/hooks/quota.rs`: token, model, provider, and quota extraction from hook payloads and transcript files.
- `src/hooks/claude_settings.rs`: hook settings generation, Claude settings merge, hook installation, and hook uninstall.
- `src/hooks/server.rs`: minimal local HTTP server accepting `POST /hook`.
- `src/proxy.rs`: local Anthropic Messages to OpenAI Chat Completions proxy on `127.0.0.1:17388`.
- `src/proxy_optimizer.rs`: OpenAI proxy long-context compression, chunked older-history summarization, and local summary cache handling (both `proxy_cache/summaries/` and `proxy_cache/chunks/`).
- `src/settings/`: user settings, LLM profiles, Claude env integration, and config file persistence.
- `src/settings/mod.rs`: persisted user settings, window position, GIF resource mapping, LLM profile database, Claude env merge logic, OpenAI extra body parsing, and path normalization.
- `src/settings/storage.rs`: shared JSON persistence helpers for BOM-tolerant reads and pretty writes.
- `src/ui/gif_animation.rs`: GDI+ GIF loading, frame delay sampling, mood transitions, and drawing.
- `src/ui/theme.rs`: shared colors, radii, and typography tokens for the settings panel and overlay popups.
- `src/ui/window/mod.rs`: main transparent always-on-top pet window, hotkeys, context menu, clicks, scaling, and position persistence.
- `src/ui/window/render.rs`: main window render snapshot, HUD, pet drawing, permission/choice overlay detail, and drawing helpers.
- `src/ui/settings_panel/mod.rs`: Settings window lifecycle, tab switching, save/apply actions, and AppState synchronization.
- `src/ui/settings_panel/controls.rs`: Win32 control creation, fonts, text helpers, message boxes, and shared drawing helpers.
- `src/ui/settings_panel/paint.rs`: Settings panel background, tabs, field chrome, and control color handling.
- `src/config.rs`: constants for ports, dimensions, menu IDs, colors, and timing.
- `src/globals.rs`: process-wide `OnceLock` handles for shared app state and pet renderer.
- `src/notifier.rs`: simple platform notification/message-box wrapper.
- `src/util.rs`: small shared helpers for args, paths, text shortening, and UTF-16 conversion.
- `build.rs`: embeds `assets/icon.ico` into Windows builds when `rc.exe` or `llvm-rc.exe` is available.
- `assets/claudie/`: bundled lightweight GIF pet resources.
- `packaging/`: Windows/Unix packaging and install helpers.

## Runtime Files

- `%USERPROFILE%\.claudie\settings.json`: pet asset directory, GIF directory, animation mapping, scale, sleep timeout, window position, and pomodoro settings.
- `%USERPROFILE%\.claudie\llm_profiles.json`: saved LLM provider/profile definitions, including OpenAI proxy extra request body fields.
- `%USERPROFILE%\.claudie\proxy_summaries.json`: legacy (single-block) OpenAI proxy summary cache.
- `%USERPROFILE%\.claudie\proxy_cache\`: OpenAI proxy cache directory, containing:
  - `summaries/`: single-block summary cache JSON files.
  - `chunks/`: chunked summary cache JSON files, each independently cached.
- `%USERPROFILE%\.claude\settings.json`: Claude Code hook settings and managed LLM env values.
- `%USERPROFILE%\.claude\settings.json.claudie.bak`: one-time backup created before modifying Claude settings.

## Architecture Notes

- `AppState` is the central mutable model. Access it through `Arc<Mutex<AppState>>` or `APP_STATE`.
- Normal runtime startup launches the hook server, launches the OpenAI proxy, then ensures Claude Code hooks point at the selected port. On Windows, exiting the UI calls hook cleanup.
- Mood transitions should go through `AppState::set_mood`, `AppState::set_resting_mood`, `AppState::start_tool_activity`, `AppState::finish_tool_activity`, or `AppState::decay_mood` so activity timestamps and renderer priority stay correct.
- Permission requests are represented by `PendingPermission` and completed through `decide_current_permission`.
- Choice-style requests are represented by `PendingChoice`; completed through `submit_current_choice` or `deny_current_choice` in `src/hooks/events.rs`.
- The hook server should stay small and synchronous. Put Claude-event semantics in `src/hooks/events.rs`, not in the HTTP parser.
- The OpenAI proxy should remain a small local compatibility layer. Keep request/response format conversion in `src/proxy.rs`, keep context optimization and summary caching in `src/proxy_optimizer.rs`, and keep profile persistence/env behavior and OpenAI extra body validation in `src/settings/mod.rs`.
- Keep OpenAI `parallel_tool_calls` enabled by default when tools are present. Modern OpenAI-compatible models handle independent tool calls correctly, and batching (e.g. reading multiple files, staging multiple paths in one git command) matches how Claude Code expects to operate. Users can still set `{"parallel_tool_calls": false}` in `OpenAI body` for older/smaller models that misbehave.
- UI code uses raw Win32 handles and unsafe calls. Keep unsafe usage close to Win32 boundaries and prefer small helper functions for repeated patterns.
- Main pet window behavior belongs in `src/ui/window/mod.rs`; main pet drawing and permission/choice overlays belong in `src/ui/window/render.rs`.
- Shared visual tokens for Settings and overlay chrome belong in `src/ui/theme.rs`; keep color, radius, and font changes centralized there.
- Settings panel lifecycle, commands, and save logic belong in `src/ui/settings_panel/mod.rs`; controls and painting belong in sibling files.
- The Settings panel draws its own tab backgrounds and uses real Win32 controls on top; tab buttons should not use default push-button styling because that leaves the default outline on the wrong tab.
- Use `util::wide` for strings passed to Win32 APIs.
- Do not block the UI thread with network or filesystem work that could take noticeable time.
- When settings change, update both persisted files and in-memory `AppState` so runtime behavior reflects changes immediately.

## Maintenance Guidelines

- Keep feature ownership clear:
  - Claude hook behavior belongs in `src/hooks/`.
  - HTTP parsing belongs in `src/hooks/server.rs`; Claude-event semantics belong in `src/hooks/events.rs`.
  - Hook settings merge/install/uninstall belongs in `src/hooks/claude_settings.rs`.
  - Quota and token field compatibility logic belongs in `src/hooks/quota.rs`.
  - OpenAI proxy transport and Anthropic/OpenAI conversion belong in `src/proxy.rs`; context optimization, long-text compression, chunked summary caching belong in `src/proxy_optimizer.rs`.
  - LLM profile serialization, OpenAI extra body parsing, and Claude env merging belong in `src/settings/mod.rs`.
  - Main pet rendering and permission/choice overlays belong in `src/ui/window/render.rs`; main window events, menu commands, and position persistence belong in `src/ui/window/mod.rs`.
  - Shared visual tokens belong in `src/ui/theme.rs`.
  - Settings UI commands belong in `src/ui/settings_panel/mod.rs`; native controls belong in `src/ui/settings_panel/controls.rs`; panel chrome belongs in `src/ui/settings_panel/paint.rs`.
  - Persistent config and JSON file read/write mechanics belong in `src/settings/`.
  - Shared domain state belongs in `src/app/mod.rs`; pomodoro domain rules belong in sibling files under `src/app/`.
- Avoid introducing new dependencies unless they remove real complexity. This app is deliberately lightweight.
- Be careful with `~/.claude/settings.json`; preserve unrelated user settings and merge only the managed hook/env fields.
- Keep pet resources lightweight: prefer one GIF per mood in `assets/claudie/`.
- Prefer focused changes over broad refactors because Win32 regressions can be subtle.

## Verification Checklist

Run at least:

```powershell
cargo fmt
cargo check
```

Run `cargo test` when touching hook settings, quota extraction, LLM profile logic, proxy conversion, or pure domain rules.

For UI, hook, permission, settings, or proxy behavior changes, also run the app manually:

```powershell
cargo run --release
```

Then verify:

- The pet window opens without a console in release builds and restores its last saved position.
- Right-click menu opens Settings, Pomodoro actions, and Exit.
- Settings tabs switch cleanly between Basic, Pomodoro, and LLM Profiles without leaving the tab outline on Basic.
- GIF resources load from settings or bundled assets.
- `POST /hook` updates mood/events.
- Permission requests show Allow, Always, and Deny controls.
- Choice requests show selectable options plus Submit and Cancel controls.
- LLM Profiles can save/use/import profiles, OpenAI-format profiles route Claude Code through `http://127.0.0.1:17388`, OpenAI extra body fields are forwarded to upstream chat completions requests, and long proxy conversations are compressed or summarized without losing the newest messages.

# claudie

[中文](README.md) | English

`claudie` is a lightweight desktop pet for Claude Code. The Windows version is built with Rust and a native Win32/GDI+ window. At runtime it is mostly one UI thread, a synchronous `std::net::TcpListener` hook server, and a local LLM proxy.

The project intentionally avoids Electron, WebView, async runtimes, and web frameworks. Pet assets use a lightweight GIF animation directory, with one GIF file mapped to each mood.

## Inspiration

claudie is inspired by [rullerzhou-afk/clawd-on-desk](https://github.com/rullerzhou-afk/clawd-on-desk) and [farion1231/cc-switch](https://github.com/farion1231/cc-switch).

## Features

- **Hook-driven pet states**: Receives Claude Code HTTP hooks and switches pet states for the following events:

  | Event | Behavior |
  |-------|----------|
  | `SessionStart` / `SessionResume` | Return to idle |
  | `UserPromptSubmit` | Thinking |
  | `PreToolUse` | Start tool, such as Write -> typing, Bash -> building, Read/Grep/Glob -> search |
  | `PostToolUse` | Finish tool |
  | `PostToolBatch` | Batch complete, trigger quota snapshot |
  | `PostToolUseFailure` / `StopFailure` / `PermissionDenied` | Error state |
  | `SubagentStart` / `TaskCreated` | Subagent working |
  | `SubagentStop` / `TaskCompleted` | Subagent done |
  | `PreCompact` | Context compressing (thinking state) |
  | `PostCompact` | Compression done |
  | `Notification` / `Elicitation` | Notification prompt |
  | `WorktreeCreate` | Creating worktree (building state) |
  | `Stop` | Task complete |
  | `SessionEnd` | Session ended, clear all pending interactions |

- **Permission requests**: Intercepts `PermissionRequest` hooks and shows Allow, Always Allow, and Deny controls in the pet window.
- **Choice cards**: Supports interactive `PreToolUse` choices for `AskUserQuestion` and `ExitPlanMode`, showing selectable options with Submit / Cancel buttons.
- **Hotkeys**:
  - `Ctrl+Shift+Y`: Allow current permission request / Submit current choice
  - `Ctrl+Shift+N`: Deny current permission request / Cancel current choice
- **Pomodoro timer**: Built-in pomodoro with Start / Stop / Pause / Resume / Skip and notification on completion.
- **Idle sleep**: Pet auto-sleeps after inactivity, wakes on new activity.
- **Pet scaling**: Adjustable pet window size.
- **Window position memory**: Saves and restores window position across sessions.
- **Mood-to-GIF mapping**: Configurable GIF file for each mood state.
- **Settings panel**: Basic, Pomodoro, and LLM Profiles tabs with a unified native theme.
- **LLM Profiles**: Save LLM providers/profiles, writes the active profile into Claude Code settings, and configures extra request body fields for the OpenAI proxy.
- **OpenAI-compatible proxy**: Converts Claude Code Anthropic Messages requests to OpenAI Chat Completions API, with tool call format conversion, parallel tool control, context compression, history summarization, and capability caching.
- **Cross-platform**: Windows has the full desktop UI; macOS / Linux currently run only headless hook and proxy servers without the desktop interaction UI.

## Quick Start

Run in development:

```powershell
cargo run --release
```

Normal startup ensures Claude Code hooks point at the current claudie port; on Windows, exiting the UI removes claudie-managed hooks. The install/uninstall commands below are for manual management or packaging flows.

Install Claude Code hooks:

```powershell
cargo run --release -- --install-claude-hooks
```

Uninstall Claude Code hooks:

```powershell
cargo run --release -- --uninstall-claude-hooks
```

Print a Claude Code settings snippet for manual merging:

```powershell
cargo run --release -- --print-claude-settings
```

Use a custom hook port:

```powershell
cargo run --release -- --port 17387
```

Suppress system notification popups during install/uninstall:

```powershell
cargo run --release -- --install-claude-hooks --quiet
```

## OpenAI-Compatible API Proxy

claudie also listens on a local proxy address when it starts:

```text
http://127.0.0.1:17388
```

In Settings -> LLM Profiles, create or edit a profile:

- Set `Base URL` to an OpenAI-compatible chat completions endpoint, such as `https://example.com/v1/chat/completions`.
- Set `API key` to the upstream OpenAI-compatible service key.
- Set `Model` to a model supported by that service.
- Optionally set `OpenAI body` to extra request body fields, using either a JSON object or one `key=value` / `key: value` entry per line; for example `{"reasoning_effort":"xhigh"}` or `model_reasoning_effort = "xhigh"`.

After clicking `Use`, claudie writes Claude Code's `ANTHROPIC_BASE_URL` to the local proxy address above. The upstream request URL and key stay only in the claudie profile. A Base URL containing `/chat/completions` enables the proxy automatically. If you enter an upstream root URL instead, add this to `Extra env`:

```text
CLAUDIE_API_FORMAT=openai
```

The proxy currently implements `POST /v1/messages`, `POST /v1/messages/count_tokens`, and `GET /v1/models`, and it converts tool calls between Anthropic and OpenAI formats. `OpenAI body` is merged into the upstream chat completions request, but it cannot override claudie-managed `messages` or `stream` fields.

When tools are present, the proxy sends `parallel_tool_calls=true` upstream by default so the model can batch independent actions (read several files, stage multiple paths in one git command) the way native Claude Code expects, cutting down on serial round-trips and duplicated token overhead. For older or smaller upstream models that misbehave under concurrent tool calls, set `{"parallel_tool_calls": false}` explicitly in `OpenAI body` to switch back to sequential. The proxy also injects a short (~80 token) compat prompt telling the model to treat tool-result messages as observations from prior calls and to re-read before retrying after a failed edit; disable it with `CLAUDIE_PROXY_COMPAT_PROMPT=0` if you don't want it.

OpenAI proxy context optimization is enabled by default. claudie compresses very long tool results and text before forwarding requests, and when the estimated input grows beyond the default threshold it keeps recent messages and summarizes older history in chunks. Each chunk is independently summarized and cached under `%USERPROFILE%\.claudie\proxy_cache\` in `summaries/`, `chunks/`, and `capabilities/` directories. The cache stores summary text or upstream capability probe results only, not API keys or full original request bodies.

You can tune or disable this behavior from a profile's `Extra env`:

```text
CLAUDIE_PROXY_OPTIMIZE=0
CLAUDIE_PROXY_SUMMARY_MODE=local
CLAUDIE_PROXY_SUMMARY_THRESHOLD=24000
CLAUDIE_PROXY_KEEP_RECENT_MESSAGES=12
CLAUDIE_PROXY_KEEP_RECENT_TOKENS=10000
CLAUDIE_PROXY_TOOL_RESULT_LIMIT=3000
CLAUDIE_PROXY_TEXT_LIMIT=6000
CLAUDIE_PROXY_MAX_OUTPUT_TOKENS=4096
CLAUDIE_PROXY_LOCAL_SUMMARY_TOKENS=2000
CLAUDIE_PROXY_CACHE_MAX_MB=10
CLAUDIE_PROXY_SUMMARY_CACHE_TTL_HOURS=168
CLAUDIE_PROXY_SUMMARY_CACHE_MAX_ENTRIES=200
CLAUDIE_PROXY_CHUNK_SUMMARY=1
CLAUDIE_PROXY_CHUNK_SIZE_MESSAGES=8
CLAUDIE_PROXY_CHUNK_CACHE_TTL_HOURS=168
CLAUDIE_PROXY_CHUNK_CACHE_MAX_ENTRIES=200
CLAUDIE_PROXY_CAPABILITY_CACHE_TTL_HOURS=168
CLAUDIE_PROXY_CAPABILITY_CACHE_MAX_ENTRIES=200
```

By default summaries are generated locally with extractive compaction (`CLAUDIE_PROXY_SUMMARY_MODE=local`) so expensive models are not called just to summarize. Set `CLAUDIE_PROXY_SUMMARY_MODE=model` if you prefer an upstream model-generated summary. If that summary request fails, claudie still forwards the request with long-content compression applied. Set `CLAUDIE_PROXY_MAX_OUTPUT_TOKENS=0` to disable the completion budget cap.

Chunked summarization (enabled by default) partitions older messages into groups of `CLAUDIE_PROXY_CHUNK_SIZE_MESSAGES`, generates a digest per chunk, and appends them to the compressed result. Set `CLAUDIE_PROXY_CHUNK_SUMMARY=0` to disable chunking and fall back to a single monolithic summary. Both the `proxy_cache/` directory files and the legacy `proxy_summaries.json` can be safely deleted; claudie regenerates them on demand.

## Project Structure

```text
src/
  main.rs                  CLI args, app startup, hook/profile initialization, platform entrypoint
  config.rs                Constants for ports, dimensions, menu IDs, colors, and timing
  globals.rs               Process-wide OnceLock handles
  notifier.rs              Platform notification / message-box wrapper
  util.rs                  Arg parsing, path, text shortening, and UTF-16 helpers
  app/                     AppState, permission requests, choice requests, Pomodoro domain rules
    mod.rs                 AppState, PetMood, sessions, quota, pending interactions, mood decay
    pomodoro.rs            Lightweight Pomodoro state and transitions
  hooks/                   Claude Code hook server, event semantics, quota extraction, settings merge
    claude_settings.rs     Hook settings generation, installation, uninstall, and merge
    events.rs              Claude Code event handling, permission waiting, and choice responses
    quota.rs               Token, model, provider, and quota compatibility extraction
    server.rs              Minimal synchronous HTTP server accepting POST /hook
  proxy.rs                 Local Anthropic Messages -> OpenAI Chat Completions proxy
  proxy_optimizer.rs       OpenAI proxy long-context compression, chunked history summaries, and summary cache
  settings/                User settings, LLM profiles, Claude env integration, JSON storage helpers
    mod.rs                 Persisted settings, profile database, OpenAI body parsing, path normalization
    storage.rs             BOM-tolerant reads and pretty JSON writes
  ui/
    gif_animation.rs       GIF loading, frame delay reading, mood transitions, and GDI+ drawing
    theme.rs               Shared visual tokens for the Settings panel and permission/choice overlays
    window/                Main pet window
      mod.rs               Window lifecycle, hotkeys, menu, click handling, position persistence
      render.rs            HUD, pet, permission overlay, and choice card drawing
    slint_views.rs         Slint component declarations for Settings and Prompt windows
    settings_panel/        Slint Settings panel lifecycle, callbacks, and controller logic
      controller.rs        Shared SettingsController state and sync helpers
      controller/          Basic, Pomodoro, and LLM Profiles tab behavior
    prompt_popup.rs        Slint permission/choice popup snapshots and callbacks
    window_icon.rs         Slint/Winit/Win32 auxiliary window icon bridge
```

Other directories:

- `assets/claudie/`: bundled pet GIF animation assets.
- `assets/icon.*`, `assets/claudie.manifest`: application icons and Windows manifest.
- `packaging/`: Windows/Unix packaging and install scripts.

## Local Data

- `%USERPROFILE%\.claudie\settings.json`: pet asset path, GIF directory, animation mapping, scale, sleep timeout, window position, and Pomodoro settings.
- `%USERPROFILE%\.claudie\llm_profiles.json`: LLM provider/profile definitions, including OpenAI proxy extra request body fields.
- `%USERPROFILE%\.claudie\proxy_summaries.json`: legacy (single-block) OpenAI proxy summary cache.
- `%USERPROFILE%\.claudie\proxy_cache\`: OpenAI proxy cache directory, containing:
  - `summaries/`: single-block summary cache JSON files.
  - `chunks/`: chunked summary cache JSON files, each chunk independently cached.
  - `capabilities/`: upstream model tool-history compatibility cache.
- `%USERPROFILE%\.claude\settings.json`: Claude Code hook settings and claudie-managed LLM env values.
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
```

The Settings panel can adjust the GIF directory and the file name mapped to each mood. When replacing art assets, keep the file name mapping consistent.

## Maintenance Boundaries

- `AppState` is the central mutable model; long-lived state and domain rules should usually live under `src/app/`.
- Keep the hook server small and synchronous; HTTP parsing stays in `src/hooks/server.rs`, while Claude event semantics belong in `src/hooks/events.rs`.
- Keep quota field compatibility logic centralized in `src/hooks/quota.rs`.
- When editing Claude settings, merge only claudie-managed hook/env fields and preserve unrelated user configuration.
- Reuse `src/settings/storage.rs` when adding new JSON state files.
- Keep OpenAI proxy context optimization, long-text compression, chunked history summaries, and summary cache logic centralized in `src/proxy_optimizer.rs`.
- Do not do potentially slow network or filesystem work on the UI thread.
- Add main-window visual elements in `src/ui/window/render.rs`; add menus, hotkeys, and mouse interactions in `src/ui/window/mod.rs`.
- Keep shared visual tokens for the Settings panel and permission/choice overlays in `src/ui/theme.rs`.
- For Settings panel fields, keep Slint component declarations in `src/ui/slint_views.rs`, callback wiring in `src/ui/settings_panel/mod.rs`, and save/refresh behavior in `src/ui/settings_panel/controller/`.

## Packaging

The Windows installer template is in `packaging/windows/claudie.iss`:

```powershell
powershell -ExecutionPolicy Bypass -File packaging\windows\build-installer.ps1
```

## Verification

Before submitting changes, run at least:

```powershell
cargo fmt
cargo check
```

Run `cargo test` when touching hook settings, quota extraction, LLM profile logic, proxy conversion, or pure domain rules.

For UI, hook, permission, settings, or proxy behavior changes, also run manually:

```powershell
cargo run --release
```

Check that the pet window position is restored after exit, the right-click menu works, Basic/Pomodoro/LLM Profiles tabs render correctly, GIF assets load, `POST /hook` updates state, permission/choice cards work, and the local LLM proxy plus `OpenAI body` forwarding work when relevant.

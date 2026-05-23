# claudie

[中文](README.md) | English

`claudie` is a lightweight desktop pet for Claude Code. The Windows version is built with Rust and a native Win32/GDI+ window. At runtime it is mostly one UI thread, a synchronous `std::net::TcpListener` hook server, and a local LLM proxy.

The project intentionally avoids Electron, WebView, async runtimes, and web frameworks. Pet assets use a lightweight GIF animation directory, with one GIF file mapped to each mood.

## Inspiration

claudie is inspired by [rullerzhou-afk/clawd-on-desk](https://github.com/rullerzhou-afk/clawd-on-desk) and [farion1231/cc-switch](https://github.com/farion1231/cc-switch).

## Features

- Receives Claude Code HTTP hooks and switches pet state for events such as `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`, and `SubagentStart/Stop`.
- Handles `PermissionRequest` hooks and shows Allow, Always, and Deny controls in the pet window.
- Supports hotkeys: `Ctrl+Shift+Y` allows the current permission request, and `Ctrl+Shift+N` denies it.
- Supports interactive `PreToolUse` choice cards for `AskUserQuestion` and `ExitPlanMode`.
- Supports a Pomodoro timer, idle sleep state, pet scaling, window position memory, and mood-to-GIF mapping.
- Right-click menu includes Settings, Start/Stop/Pause/Resume/Skip Pomodoro, and Exit.
- Settings panel includes Basic, Pomodoro, and LLM Profiles tabs with a unified native theme.
- Saves LLM providers/profiles, writes the active profile into Claude Code settings, and configures extra request body fields for the OpenAI proxy.
- Includes a local OpenAI Chat Completions proxy that converts Claude Code Anthropic Messages requests to OpenAI-compatible APIs.
- Windows has the full desktop UI; macOS/Linux currently run only headless hook/proxy servers without the desktop interaction UI.

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

The proxy currently implements the Claude Code endpoints `POST /v1/messages`, `POST /v1/messages/count_tokens`, and `GET /v1/models`, and it converts tool calls between Anthropic and OpenAI formats. `OpenAI body` is merged into the upstream chat completions request, but it cannot override claudie-managed `messages` or `stream` fields.

## Project Structure

```text
src/
  main.rs                  CLI args, app startup, hook/profile initialization, platform entrypoint
  config.rs                Constants for ports, dimensions, menu IDs, colors, and timing
  globals.rs               Process-wide OnceLock handles
  notifier.rs              Platform notification / message-box wrapper
  util.rs                  Arg parsing, path, text shortening, and UTF-16 helpers
  app/                     AppState, permission requests, choice requests, Pomodoro domain rules
  hooks/                   Claude Code hook server, event semantics, quota extraction, settings merge
  proxy.rs                 Local Anthropic Messages -> OpenAI Chat Completions proxy
  settings/                User settings, LLM profiles, Claude env integration, JSON storage helpers
  ui/
    gif_animation.rs       GIF loading, frame delay reading, and GDI+ drawing
    theme.rs               Shared visual tokens for the Settings panel and permission/choice overlays
    window/                Main pet window
      mod.rs               Window lifecycle, hotkeys, menu, click handling, position persistence
      render.rs            HUD, pet, permission overlay, and choice card drawing
    settings_panel/        Native Settings panel
      mod.rs               Panel lifecycle, tab switching, save/refresh logic
      controls.rs          Win32 control creation, text, fonts, message box helpers
      paint.rs             Settings panel background, tabs, and field drawing
```

Other directories:

- `assets/claudie/`: bundled pet GIF animation assets.
- `assets/icon.*`, `assets/claudie.manifest`: application icons and Windows manifest.
- `packaging/`: Windows/Unix packaging and install scripts.

## Local Data

- `%USERPROFILE%\.claudie\settings.json`: pet asset path, GIF directory, animation mapping, scale, sleep timeout, window position, and Pomodoro settings.
- `%USERPROFILE%\.claudie\llm_profiles.json`: LLM provider/profile definitions, including OpenAI proxy extra request body fields.
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
  permission.gif
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
- Do not do potentially slow network or filesystem work on the UI thread.
- Add main-window visual elements in `src/ui/window/render.rs`; add menus, hotkeys, and mouse interactions in `src/ui/window/mod.rs`.
- Keep shared visual tokens for the Settings panel and permission/choice overlays in `src/ui/theme.rs`.
- For Settings panel fields, keep window messages and save behavior in `src/ui/settings_panel/mod.rs`, control helpers in `controls.rs`, and background/field decoration in `paint.rs`.

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

For UI, hook, permission, settings, or proxy behavior changes, also run manually:

```powershell
cargo run --release
```

Check that the pet window position is restored after exit, the right-click menu works, Basic/Pomodoro/LLM Profiles tabs render correctly, GIF assets load, `POST /hook` updates state, permission/choice cards work, and the local LLM proxy plus `OpenAI body` forwarding work when relevant.

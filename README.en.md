# claudie

[中文](README.md) | English

> A desktop pet that lives next to Claude Code — it animates with Claude's activity, answers permission/choice requests right in its window, and can route Claude Code through any OpenAI-compatible model.

`claudie` is a **Windows-only** lightweight desktop pet built in **Rust with a native Win32/GDI+** window. It intentionally avoids Electron, WebView, async runtimes, and web frameworks — small resident footprint, no browser engine.

Inspired by [rullerzhou-afk/clawd-on-desk](https://github.com/rullerzhou-afk/clawd-on-desk) and [farion1231/cc-switch](https://github.com/farion1231/cc-switch).

## Three parts

| Component | Role |
|-----------|------|
| **Desktop pet UI** | Switches GIF animations based on Claude Code activity and answers permission/choice requests right in the window. |
| **Hook server** | A synchronous `std::net::TcpListener` that receives Claude Code HTTP hook events (`127.0.0.1:17387/hook`). |
| **OpenAI-compatible proxy** | Converts Claude Code's Anthropic Messages requests to OpenAI Chat Completions style upstreams (`127.0.0.1:17388`). |

## Features

- **State follows activity** — receives hook events and switches the pet's state: thinking, typing, running commands, searching, subagents, errors, sleep… Full mapping in the table below.
- **Permission requests** — shows Allow / Always / Deny in the pet window. Deny writes back `continue=false` + `interrupt=true`, matching a terminal "No": it stops the current turn instead of feeding the rejection back as a retriable tool error. Answering on either side (pet or terminal) closes the popup on both.
- **Choice cards** — supports `AskUserQuestion` (with a free-text "Other…" option) and `ExitPlanMode` (plan rendered as Markdown), with options plus Submit / Cancel.
- **Multi-session switcher** — tracks each session's status and renders a switcher panel beside the pet; scroll to change the focused session, which drives the pet mood and HUD. Hidden automatically when only one session is active.
- **Hotkeys** — `Ctrl+Shift+Y` allows / submits; `Ctrl+Shift+N` denies / cancels.
- **Pomodoro** — built-in timer with Start / Stop / Pause / Resume / Skip and phase-completion notifications.
- **Fishing minigame** — click the pet for a "waiting → reeling → caught/missed" sequence; keep tension inside the moving target zone while reeling.
- **Official usage monitoring** — real-time 5h / 7d usage in the right-click menu and Settings panel via OAuth polling, with Max/Pro/Team plan detection and a reset countdown.
- **LLM Profiles** — save official or custom profiles, write the active one to Claude Code settings, switch from the right-click menu in one click.
- **OpenAI-compatible proxy** — tools, streaming, image forwarding, reasoning output, parallel tool calls, tool-history fallback, context compression, and summaries. See [below](#openai-compatible-proxy).
- **Local ledger** — daily prompts, tool categories, permission/choice counts, errors, completed focus sessions, and token usage; Stats shows today and the last 7 days, never leaving your machine.
- **Also** — system tray icon, pet scaling and window-position memory, short-click interaction animations, idle auto-sleep, and DPAPI-encrypted secrets (API keys / OAuth tokens).

<details>
<summary>Hook event → pet state mapping</summary>

| Event | Behavior |
|-------|----------|
| `SessionStart` | Return to idle |
| `UserPromptSubmit` | Thinking |
| `PreToolUse` | Start tool activity; write tools → typing, shell tools → building, read/search tools → search |
| `PostToolUse` | Finish tool activity |
| `PostToolBatch` | Batch complete, refresh quota snapshot |
| `PostToolUseFailure` / `StopFailure` | Error state |
| `PermissionDenied` | Denied state (separate mood) |
| `PermissionRequest` | Wait for Allow / Always / Deny in the pet UI |
| `SubagentStart` / `TaskCreated` | Subagent working |
| `SubagentStop` / `TaskCompleted` | Subagent done |
| `PreCompact` / `PostCompact` | Context compression starts / finishes |
| `Notification` / `Elicitation` | Notification prompt |
| `WorktreeCreate` | Creating worktree |
| `Stop` | Task complete |
| `SessionEnd` | Session ended, clear pending interactions |

</details>

## Quick Start

Requirements: Windows 10/11 + the [Rust toolchain](https://rustup.rs/) (Windows only; non-Windows does not build).

```powershell
cargo run --release
```

Normal startup listens on the hook (`:17387`) and proxy (`:17388`), then automatically points Claude Code hooks at the current port; exiting the UI removes claudie-managed hooks. No manual hook install is needed to get started.

Common commands (`--install` / `--uninstall` are short aliases; `--quiet` suppresses install notification popups):

```powershell
cargo run --release -- --help
cargo run --release -- --port 17387
cargo run --release -- --install-claude-hooks      # alias: --install
cargo run --release -- --uninstall-claude-hooks    # alias: --uninstall
cargo run --release -- --print-claude-settings
```

To build an installer for non-developers, see [Packaging](#packaging).

## OpenAI-Compatible Proxy

The proxy listens on `http://127.0.0.1:17388` and implements `POST /v1/messages`, `POST /v1/messages/count_tokens`, and `GET /v1/models`, letting Claude Code run on any OpenAI-compatible upstream (DeepSeek, Qwen, Kimi, GLM, OpenRouter, OneAPI/NewAPI, …).

Configure a profile in **Settings → LLM Profiles**:

| Field | Description |
|-------|-------------|
| `Base URL` | OpenAI-compatible endpoint, e.g. `https://example.com/v1/chat/completions` or `https://example.com/v1`. |
| `API key` | Upstream service key; if empty, the proxy uses `Auth token` as the upstream key. |
| `Model` | A model supported by that service. Each model row has a `1M` toggle: when on, claudie appends a `[1m]` suffix to the model id written to Claude Code, and the proxy strips it before forwarding upstream. The `Fetch models` button at the top of the Models section probes the upstream OpenAI `/v1/models` (handling `/anthropic` and `/v{N}` path variants) and populates a dropdown for each model field. |
| `OpenAI body` | Optional extra request fields — JSON object or one `key=value` / `key: value` per line, e.g. `{"reasoning_effort":"xhigh"}`. Merged into the upstream request, but cannot override claudie-managed `messages` / `stream`. |
| `Extra env` | One `KEY=VALUE` proxy switch or Claude Code env var per line. |

After clicking `Use`, if the profile is OpenAI format claudie points Claude Code's `ANTHROPIC_BASE_URL` at the local proxy (the upstream URL/key stay only in the claudie profile). A `Base URL` containing `/chat/completions` enables the proxy automatically; if you enter an upstream root URL, add `CLAUDIE_API_FORMAT=openai` to `Extra env`.

**Proxy capabilities:**

- Streaming and non-streaming OpenAI responses are converted back to Anthropic Messages / SSE events.
- Anthropic tool use / tool result ↔ OpenAI `tools` / `tool_calls` / `tool` messages.
- `parallel_tool_calls=true` by default when tools are present and supported (set `false` in `OpenAI body` to disable).
- DeepSeek R1, QwQ, GLM-Zero and similar reasoning streams map to Anthropic thinking blocks; OpenAI/Azure/OpenRouter reasoning models auto-derive `reasoning_effort` from `thinking.budget_tokens`.
- Image forwarding is auto-detected from the model name; force with `CLAUDIE_PROXY_FORWARD_IMAGES=always/never`.
- Recognized mainstream upstreams keep the compat prompt off by default; generic OneAPI/NewAPI-style upstreams get it on (`CLAUDIE_PROXY_COMPAT_PROMPT=0/1`).
- If an upstream rejects native tool history, the proxy falls back to text transcript mode and caches the capability probe.
- Upstream 429/529 `Retry-After` is forwarded to Claude Code to trigger native backoff; connection failures/timeouts and other transient errors uniformly return HTTP 529 (matching Anthropic's overload semantics).

**Context optimization** (on by default): compresses very long tool results and text; when input exceeds the threshold it keeps recent messages and summarizes older history in chunks using a local extractive summary (no upstream call). The cache stores only chunk summaries and capability probes — never API keys or original request bodies.

<details>
<summary><code>Extra env</code> tunables (defaults)</summary>

```text
CLAUDIE_PROXY_OPTIMIZE=1
CLAUDIE_PROXY_SUMMARY_THRESHOLD=24000
CLAUDIE_PROXY_KEEP_RECENT_MESSAGES=12
CLAUDIE_PROXY_KEEP_RECENT_TOKENS=10000
CLAUDIE_PROXY_TOOL_RESULT_LIMIT=3000
CLAUDIE_PROXY_TEXT_LIMIT=6000
CLAUDIE_PROXY_LOCAL_SUMMARY_TOKENS=2000
CLAUDIE_PROXY_CACHE_MAX_MB=10
CLAUDIE_PROXY_CHUNK_SUMMARY=1
CLAUDIE_PROXY_CHUNK_SIZE_MESSAGES=8
CLAUDIE_PROXY_CHUNK_CACHE_TTL_HOURS=168
CLAUDIE_PROXY_CHUNK_CACHE_MAX_ENTRIES=200
CLAUDIE_PROXY_CAPABILITY_CACHE_TTL_HOURS=720
CLAUDIE_PROXY_CAPABILITY_CACHE_MAX_ENTRIES=200
CLAUDIE_PROXY_COMPAT_PROMPT=0
CLAUDIE_PROXY_FORWARD_IMAGES=auto
```

</details>

## Stats Panel

Settings → Stats visualizes usage from the local `daily_stats.json` (up to 45 days); all data stays on your machine.

- **Top KPIs** — large number is today, `7d · N` below is the 7-day total. Covers Prompts (turns), Tokens (input + output + cache read/write), Cache hit (cache-read ÷ all context input — higher is cheaper), and Tool calls.
- **Activity** — 14-day prompt bar chart with today highlighted and idle days kept as empty bars; the caption gives prompts/day, the 14-day peak, and active days in the last 7.
- **Productivity highlights** (last 7 days) — active days, avg tokens per prompt, top tool category, and completed Pomodoro sessions.
- **Detail** (last 7 days) — Tool mix (Write/Bash/Search/Agent/Perm/Choice) and Tokens (Input/Output/Cache W/Cache R).

## Local Data

All under `%USERPROFILE%\.claudie\` (except the last two, under `.claude\`):

| File | Contents |
|------|----------|
| `settings.json` | asset path, GIF mapping, scale, sleep timeout, window position, Pomodoro settings |
| `llm_profiles.json` | LLM profiles, active profile, upstream auth, OpenAI body, Extra env |
| `secrets.json` | DPAPI-encrypted credentials (API keys, OAuth tokens), decryptable only by the current Windows user |
| `daily_stats.json` | daily counters (prompts, tools, permissions/choices, errors, focus, tokens), kept 45 days |
| `proxy_cache/` | proxy cache: `chunks/`, `capabilities/` |
| `.claude\settings.json` | Claude Code hook settings and claudie-managed LLM env |
| `.claude\settings.json.claudie.bak` | one-time backup created before the first modification |

## Pet Assets

Bundled GIFs live in `assets/claudie/`, one file per mood: `idle` `thinking` `typing` `building` `search` `happy` `error` `deny` `sleeping` `subagent` `pomodoro` `wave` `stretch` `fishing` `reel` `caught` `missed`. The Settings panel can adjust the GIF directory and the file name for each mood; keep the mapping consistent when replacing art.

## Packaging

The Windows installer template is in `packaging/windows/claudie.iss`; output is `dist\claudie-setup.exe`:

```powershell
powershell -ExecutionPolicy Bypass -File packaging\windows\build-installer.ps1
```

## Development

Before submitting, run at least `cargo fmt` and `cargo check`; run `cargo test` when touching hooks, quota, profiles, proxy conversion/streaming, the optimizer, stats, Pomodoro, or other pure domain logic; UI/hook/permission/proxy behavior changes also need a manual `cargo run --release`.

For the code map, key-file list, and full verification checklist, see [AGENTS.md](AGENTS.md).

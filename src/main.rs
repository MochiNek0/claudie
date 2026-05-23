#![allow(unsafe_op_in_unsafe_fn)]
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod config;
mod globals;
mod hooks;
mod notifier;
mod proxy;
mod settings;
#[cfg(windows)]
mod ui;
mod util;

use std::env;
use std::sync::{Arc, Mutex};

use app::{AppState, PetMood};
use config::DEFAULT_PORT;
use globals::APP_STATE;
use hooks::{
    ensure_claude_hooks, install_claude_hooks, settings_snippet, start_hook_server,
    uninstall_claude_hooks,
};
use notifier::notify_user;
use proxy::start_openai_proxy_server;
#[cfg(windows)]
use ui::{init_animation_store, run_window};
use util::parse_port;

fn main() {
    let args: Vec<String> = env::args().collect();
    let port = parse_port(&args).unwrap_or(DEFAULT_PORT);
    let quiet = args.iter().any(|arg| arg == "--quiet");

    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return;
    }

    if args.iter().any(|arg| arg == "--print-claude-settings") {
        println!("{}", settings_snippet(port));
        return;
    }

    if args
        .iter()
        .any(|arg| arg == "--install" || arg == "--install-claude-hooks")
    {
        match install_claude_hooks(port) {
            Ok(path) => {
                let message = format!(
                    "Installed Claude Code hooks in {}\n\nRun claudie normally, then use Claude Code.\nHook URL: http://127.0.0.1:{port}/hook",
                    path.display()
                );
                println!("{message}");
                if !quiet {
                    notify_user("claudie", &message, false);
                }
            }
            Err(err) => {
                eprintln!("Failed to install hooks: {err}");
                if !quiet {
                    notify_user("claudie install failed", &err, true);
                }
                std::process::exit(1);
            }
        }
        return;
    }

    if args
        .iter()
        .any(|arg| arg == "--uninstall" || arg == "--uninstall-claude-hooks")
    {
        match uninstall_claude_hooks() {
            Ok(Some(path)) => {
                let message = format!("Removed claudie Claude Code hooks from {}", path.display());
                println!("{message}");
                if !quiet {
                    notify_user("claudie", &message, false);
                }
            }
            Ok(None) => {
                let message = "Claude Code settings.json was not found; nothing to remove.";
                println!("{message}");
                if !quiet {
                    notify_user("claudie", message, false);
                }
            }
            Err(err) => {
                eprintln!("Failed to uninstall hooks: {err}");
                if !quiet {
                    notify_user("claudie uninstall failed", &err, true);
                }
                std::process::exit(1);
            }
        }
        return;
    }

    let state = Arc::new(Mutex::new(AppState::new()));
    let _ = APP_STATE.set(state.clone());

    run_app(state, port);
}

fn print_help() {
    println!("claudie");
    println!("  cargo run --release");
    println!("  claudie --install-claude-hooks [--port 17387] [--quiet]");
    println!("  claudie --uninstall-claude-hooks [--quiet]");
    println!("  claudie --print-claude-settings [--port 17387]");
}

#[cfg(windows)]
fn run_app(state: Arc<Mutex<AppState>>, port: u16) {
    init_animation_store();
    let hooks_installed = start_runtime_hooks(state.clone(), port);
    unsafe {
        run_window(port);
    }
    if hooks_installed {
        cleanup_runtime_hooks();
    }
}

#[cfg(not(windows))]
fn run_app(state: Arc<Mutex<AppState>>, port: u16) {
    if start_runtime_hooks(state, port) {
        println!("claudie hook server is running at http://127.0.0.1:{port}/hook");
        println!("Desktop pet UI is currently available on Windows; press Ctrl+C to stop.");
        loop {
            std::thread::park();
        }
    }
}

fn start_runtime_hooks(state: Arc<Mutex<AppState>>, port: u16) -> bool {
    if let Err(err) = start_hook_server(state.clone(), port) {
        record_app_error(&state, "server", err);
        return false;
    }

    if let Err(err) = start_openai_proxy_server(state.clone()) {
        record_app_error(&state, "proxy", err);
    }

    if let Err(err) = ensure_claude_hooks(state.clone(), port) {
        record_app_error(&state, "hooks", err);
        return false;
    }

    true
}

fn cleanup_runtime_hooks() {
    if let Err(err) = uninstall_claude_hooks() {
        eprintln!("Failed to uninstall hooks on exit: {err}");
    }
}

fn record_app_error(state: &Arc<Mutex<AppState>>, source: &str, err: String) {
    let mut state = state.lock().expect("state poisoned");
    state.last_error = err.clone();
    state.set_mood(PetMood::Error);
    state.push_event(source, err);
}

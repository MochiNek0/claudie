mod claude_settings;
mod events;
mod quota;
mod server;

pub(crate) use claude_settings::{
    ensure_claude_hooks, install_claude_hooks, settings_snippet, uninstall_claude_hooks,
};
pub(crate) use events::{
    decide_current_permission, deny_current_choice, process_hook, set_current_choice_other_text,
    submit_current_choice, toggle_current_choice_option,
};
pub(crate) use server::start_hook_server;

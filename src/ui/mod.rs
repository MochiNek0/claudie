pub(crate) mod gif_animation;
pub(crate) mod prompt_popup;
pub(crate) mod settings_panel;
pub(crate) mod slint_views;
pub(crate) mod theme;
pub(crate) mod window_icon;
pub(crate) mod window_position;

mod window;

pub(crate) use gif_animation::init_animation_store;
pub(crate) use window::run_window;

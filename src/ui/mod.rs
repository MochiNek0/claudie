pub(crate) mod gif_animation;
pub(crate) mod theme;

mod settings_panel;
mod window;

pub(crate) use gif_animation::init_animation_store;
pub(crate) use window::run_window;

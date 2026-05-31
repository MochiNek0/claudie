use std::sync::{Arc, Mutex, OnceLock};

use crate::app::AppState;
use crate::ui::gif_animation::AnimationStore;

pub(crate) static APP_STATE: OnceLock<Arc<Mutex<AppState>>> = OnceLock::new();
pub(crate) static PET_RENDERER: OnceLock<Mutex<AnimationStore>> = OnceLock::new();

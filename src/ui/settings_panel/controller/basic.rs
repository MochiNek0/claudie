use std::rc::Rc;

use slint::{ModelRc, VecModel};

use crate::config::{PET_SCALE_MAX_PERCENT, PET_SCALE_MIN_PERCENT};
use crate::settings::{AnimationSettings, UserSettings, save_user_settings};
use crate::ui::gif_animation::reload_animation_store;
use crate::ui::slint_views::AnimField;

use super::{SettingsController, clamp_i32, shared, snap_sleep_after, sync_app_settings};

// Display labels for the mood GIF grid; order must match `anim_values` and
// `animations_from_values` below.
const ANIM_LABELS: [&str; 16] = [
    "Idle", "Thinking", "Typing", "Building", "Search", "Happy", "Error", "Sleeping", "Subagent",
    "Pomodoro", "Wave", "Stretch", "Fishing", "Reel", "Caught", "Missed",
];

fn anim_values(animations: &AnimationSettings) -> Vec<String> {
    vec![
        animations.idle.clone(),
        animations.thinking.clone(),
        animations.typing.clone(),
        animations.building.clone(),
        animations.search.clone(),
        animations.happy.clone(),
        animations.error.clone(),
        animations.sleeping.clone(),
        animations.subagent.clone(),
        animations.pomodoro.clone(),
        animations.wave.clone(),
        animations.stretch.clone(),
        animations.fishing.clone(),
        animations.fishing_reel.clone(),
        animations.fishing_caught.clone(),
        animations.fishing_missed.clone(),
    ]
}

fn animations_from_values(values: &[String]) -> AnimationSettings {
    let at = |index: usize| values.get(index).cloned().unwrap_or_default();
    AnimationSettings {
        idle: at(0),
        thinking: at(1),
        typing: at(2),
        building: at(3),
        search: at(4),
        happy: at(5),
        error: at(6),
        sleeping: at(7),
        subagent: at(8),
        pomodoro: at(9),
        wave: at(10),
        stretch: at(11),
        fishing: at(12),
        fishing_reel: at(13),
        fishing_caught: at(14),
        fishing_missed: at(15),
    }
}

impl SettingsController {
    pub(super) fn refresh_basic_fields(&mut self) {
        let Some(ui) = self.ui() else {
            return;
        };
        ui.set_pet_scale(self.settings.pet_scale_percent() as f32);
        ui.set_sleep_after(self.settings.sleep_after_secs() as f32);
        ui.set_show_session_switcher(self.settings.show_session_switcher);
        ui.set_pet_dir(shared(&self.settings.pet_dir));
        ui.set_gif_dir(shared(&self.settings.gif_dir));
        self.anim_values = anim_values(&self.settings.animations);
        let rows: Vec<AnimField> = ANIM_LABELS
            .iter()
            .zip(self.anim_values.iter())
            .map(|(label, value)| AnimField {
                label: shared(label),
                value: shared(value),
            })
            .collect();
        ui.set_anim_fields(ModelRc::from(Rc::new(VecModel::from(rows))));
    }

    pub(in crate::ui::settings_panel) fn set_anim_value(&mut self, index: i32, value: &str) {
        if let Some(slot) = self.anim_values.get_mut(index as usize) {
            *slot = value.to_string();
        }
    }

    fn collect_basic_fields(&mut self) {
        let Some(ui) = self.ui() else {
            return;
        };
        self.settings.pet_scale_percent = clamp_i32(
            ui.get_pet_scale().round() as i32,
            PET_SCALE_MIN_PERCENT,
            PET_SCALE_MAX_PERCENT,
        );
        self.settings.sleep_after_secs = clamp_i32(ui.get_sleep_after().round() as i32, 15, 1800);
        self.settings.show_session_switcher = ui.get_show_session_switcher();
        self.settings.pet_dir = ui.get_pet_dir().to_string();
        self.settings.gif_dir = ui.get_gif_dir().to_string();
        self.settings.animations = animations_from_values(&self.anim_values);
    }

    pub(in crate::ui::settings_panel) fn update_pet_scale_live(&mut self, value: f32) {
        self.settings.pet_scale_percent = clamp_i32(
            value.round() as i32,
            PET_SCALE_MIN_PERCENT,
            PET_SCALE_MAX_PERCENT,
        );
        sync_app_settings(&self.settings);
    }

    pub(in crate::ui::settings_panel) fn update_sleep_after_live(&mut self, value: f32) {
        self.settings.sleep_after_secs = snap_sleep_after(value);
        if let Some(ui) = self.ui()
            && (ui.get_sleep_after().round() as u32) != self.settings.sleep_after_secs
        {
            ui.set_sleep_after(self.settings.sleep_after_secs as f32);
        }
        sync_app_settings(&self.settings);
    }

    pub(in crate::ui::settings_panel) fn save_basic_settings(&mut self) {
        self.collect_basic_fields();
        if let Err(err) = save_user_settings(&self.settings) {
            self.status(&format!("Failed to save basic settings: {err}"));
            return;
        }
        sync_app_settings(&self.settings);
        match reload_animation_store() {
            Ok(_) => self.status("Saved pet settings."),
            Err(err) => self.status(&format!("Saved, but failed to reload pet renderer: {err}")),
        }
    }

    pub(in crate::ui::settings_panel) fn reset_basic_fields(&mut self) {
        let defaults = UserSettings::default();
        self.settings.pet_scale_percent = defaults.pet_scale_percent();
        self.settings.sleep_after_secs = defaults.sleep_after_secs();
        self.settings.show_session_switcher = defaults.show_session_switcher;
        self.settings.pet_dir = defaults.pet_dir;
        self.settings.gif_dir = defaults.gif_dir;
        self.settings.animations = defaults.animations;
        self.refresh_basic_fields();
        sync_app_settings(&self.settings);
        self.status("Reset pet fields to defaults.");
    }
}

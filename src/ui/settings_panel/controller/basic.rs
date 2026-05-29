use crate::config::{PET_SCALE_MAX_PERCENT, PET_SCALE_MIN_PERCENT};
use crate::settings::{AnimationSettings, UserSettings, save_user_settings};
use crate::ui::gif_animation::reload_animation_store;

use super::{SettingsController, clamp_i32, shared, snap_sleep_after, sync_app_settings};

impl SettingsController {
    pub(super) fn refresh_basic_fields(&self) {
        let Some(ui) = self.ui() else {
            return;
        };
        ui.set_pet_scale(self.settings.pet_scale_percent() as f32);
        ui.set_sleep_after(self.settings.sleep_after_secs() as f32);
        ui.set_pet_dir(shared(&self.settings.pet_dir));
        ui.set_gif_dir(shared(&self.settings.gif_dir));
        ui.set_anim_idle(shared(&self.settings.animations.idle));
        ui.set_anim_thinking(shared(&self.settings.animations.thinking));
        ui.set_anim_typing(shared(&self.settings.animations.typing));
        ui.set_anim_building(shared(&self.settings.animations.building));
        ui.set_anim_search(shared(&self.settings.animations.search));
        ui.set_anim_happy(shared(&self.settings.animations.happy));
        ui.set_anim_error(shared(&self.settings.animations.error));
        ui.set_anim_sleeping(shared(&self.settings.animations.sleeping));
        ui.set_anim_subagent(shared(&self.settings.animations.subagent));
        ui.set_anim_pomodoro(shared(&self.settings.animations.pomodoro));
        ui.set_anim_wave(shared(&self.settings.animations.wave));
        ui.set_anim_stretch(shared(&self.settings.animations.stretch));
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
        self.settings.pet_dir = ui.get_pet_dir().to_string();
        self.settings.gif_dir = ui.get_gif_dir().to_string();
        self.settings.animations = AnimationSettings {
            idle: ui.get_anim_idle().to_string(),
            thinking: ui.get_anim_thinking().to_string(),
            typing: ui.get_anim_typing().to_string(),
            building: ui.get_anim_building().to_string(),
            search: ui.get_anim_search().to_string(),
            happy: ui.get_anim_happy().to_string(),
            error: ui.get_anim_error().to_string(),
            sleeping: ui.get_anim_sleeping().to_string(),
            subagent: ui.get_anim_subagent().to_string(),
            pomodoro: ui.get_anim_pomodoro().to_string(),
            wave: ui.get_anim_wave().to_string(),
            stretch: ui.get_anim_stretch().to_string(),
        };
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
        self.settings.pet_dir = defaults.pet_dir;
        self.settings.gif_dir = defaults.gif_dir;
        self.settings.animations = defaults.animations;
        self.refresh_basic_fields();
        sync_app_settings(&self.settings);
        self.status("Reset pet fields to defaults.");
    }
}

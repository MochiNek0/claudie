use crate::config::{PET_SCALE_MAX_PERCENT, PET_SCALE_MIN_PERCENT};
use crate::settings::{
    UserSettings, mood_gif_filename, mood_rows, save_user_settings, user_gif_dir,
};
use crate::ui::folder_dialog::pick_folder;
use crate::ui::gif_animation::reload_animation_store;
use crate::ui::window_icon::slint_window_hwnd;
use slint::ComponentHandle;

use super::{SettingsController, clamp_i32, shared, snap_sleep_after, sync_app_settings};

impl SettingsController {
    pub(super) fn refresh_basic_fields(&mut self) {
        let Some(ui) = self.ui() else {
            return;
        };
        ui.set_pet_scale(self.settings.pet_scale_percent() as f32);
        ui.set_sleep_after(self.settings.sleep_after_secs() as f32);
        ui.set_show_session_switcher(self.settings.show_session_switcher);
        ui.set_gif_dir(shared(&self.settings.gif_dir));
        ui.set_gif_dir_label(shared(&self.gif_dir_label()));
        ui.set_gif_status(shared(&self.gif_status()));
    }

    /// Display label for the chosen folder; bundled GIFs when none is set.
    fn gif_dir_label(&self) -> String {
        let trimmed = self.settings.gif_dir.trim();
        if trimmed.is_empty() {
            "Bundled GIFs".to_string()
        } else {
            trimmed.to_string()
        }
    }

    /// Naming-convention hint plus, when a custom folder is set, the list of
    /// moods that fall back to the bundled default because their file is
    /// missing from that folder.
    fn gif_status(&self) -> String {
        let convention: Vec<&str> = mood_rows()
            .iter()
            .map(|(mood, _)| mood_gif_filename(*mood))
            .collect();
        let hint = format!(
            "Name files: {}. Any missing file uses the bundled default.",
            convention.join(", ")
        );

        let Some(dir) = user_gif_dir(&self.settings) else {
            return format!("Using bundled GIFs. {hint}");
        };

        let missing: Vec<&str> = mood_rows()
            .iter()
            .filter(|(mood, _)| !dir.join(mood_gif_filename(*mood)).is_file())
            .map(|(_, label)| *label)
            .collect();
        if missing.is_empty() {
            format!("All 16 GIFs found in this folder. {hint}")
        } else {
            format!(
                "Using the bundled default for: {}. {hint}",
                missing.join(", ")
            )
        }
    }

    pub(in crate::ui::settings_panel) fn browse_gif_dir(&mut self) {
        let owner = self
            .ui()
            .map(|ui| slint_window_hwnd(ui.window()))
            .unwrap_or(std::ptr::null_mut());
        if let Some(path) = pick_folder("Choose a GIF folder for the pet", owner) {
            self.settings.gif_dir = path.to_string_lossy().to_string();
            self.refresh_basic_fields();
        }
    }

    pub(in crate::ui::settings_panel) fn clear_gif_dir(&mut self) {
        self.settings.gif_dir.clear();
        self.refresh_basic_fields();
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
        // gif_dir is driven by the Browse / Use default buttons, so it is
        // already current in self.settings; nothing to read back from the UI.
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
        self.settings.gif_dir = defaults.gif_dir;
        self.refresh_basic_fields();
        sync_app_settings(&self.settings);
        self.status("Reset pet fields to defaults.");
    }
}

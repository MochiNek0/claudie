use crate::config::{PET_SCALE_MAX_PERCENT, PET_SCALE_MIN_PERCENT};
use crate::i18n::Lang;
use crate::settings::{
    UserSettings, mood_gif_filename, mood_rows, save_user_settings, user_gif_dir,
};
use crate::ui::folder_dialog::pick_folder;
use crate::ui::gif_animation::reload_animation_store;
use crate::ui::window_icon::slint_window_hwnd;
use slint::{ComponentHandle, ModelRc, VecModel};

use super::{SettingsController, clamp_i32, shared, snap_sleep_after, sync_app_settings};

impl SettingsController {
    pub(super) fn refresh_basic_fields(&mut self) {
        let Some(ui) = self.ui() else {
            return;
        };
        ui.set_pet_scale(self.settings.pet_scale_percent() as f32);
        ui.set_sleep_after(self.settings.sleep_after_secs() as f32);
        ui.set_show_session_switcher(self.settings.show_session_switcher);
        ui.set_language_model(ModelRc::new(VecModel::from(vec![
            shared("English"),
            shared("中文"),
        ])));
        ui.set_language_index(match self.settings.language {
            Lang::En => 0,
            Lang::Zh => 1,
        });
        ui.set_gif_dir(shared(&self.settings.gif_dir));
        ui.set_gif_dir_label(shared(&self.gif_dir_label()));
        ui.set_gif_status(shared(&self.gif_status()));
    }

    /// Switch the UI language: persist it, sync the running app, push the new
    /// strings into the open window's `I18n` global, and refresh the form so
    /// the picker and the dynamic status text follow immediately.
    pub(in crate::ui::settings_panel) fn change_language(&mut self, index: i32) {
        let lang = if index == 1 { Lang::Zh } else { Lang::En };
        if lang == self.settings.language {
            return;
        }
        self.settings.language = lang;
        crate::i18n::set_current(lang);
        let _ = save_user_settings(&self.settings);
        sync_app_settings(&self.settings);
        if let Some(ui) = self.ui() {
            crate::ui::settings_panel::apply_settings_i18n(&ui);
        }
        self.refresh_basic_fields();
        self.refresh_pomodoro_tab();
        self.refresh_stats_tab();
    }

    /// Display label for the chosen folder; bundled GIFs when none is set.
    fn gif_dir_label(&self) -> String {
        let trimmed = self.settings.gif_dir.trim();
        if trimmed.is_empty() {
            crate::i18n::strings().gif_bundled.to_string()
        } else {
            trimmed.to_string()
        }
    }

    /// Naming-convention hint plus, when a custom folder is set, the list of
    /// moods that fall back to the bundled default because their file is
    /// missing from that folder.
    fn gif_status(&self) -> String {
        let s = crate::i18n::strings();
        let convention: Vec<&str> = mood_rows()
            .iter()
            .map(|(mood, _)| mood_gif_filename(*mood))
            .collect();
        let hint = s.gif_name_hint_fmt.replace("{}", &convention.join(", "));

        let Some(dir) = user_gif_dir(&self.settings) else {
            return s.gif_using_bundled_fmt.replace("{}", &hint);
        };

        let missing: Vec<&str> = mood_rows()
            .iter()
            .filter(|(mood, _)| !dir.join(mood_gif_filename(*mood)).is_file())
            .map(|(_, label)| *label)
            .collect();
        if missing.is_empty() {
            s.gif_all_found_fmt
                .replacen("{}", &mood_rows().len().to_string(), 1)
                .replacen("{}", &hint, 1)
        } else {
            s.gif_using_default_fmt
                .replacen("{}", &missing.join(", "), 1)
                .replacen("{}", &hint, 1)
        }
    }

    pub(in crate::ui::settings_panel) fn browse_gif_dir(&mut self) {
        let owner = self
            .ui()
            .map(|ui| slint_window_hwnd(ui.window()))
            .unwrap_or(std::ptr::null_mut());
        if let Some(path) = pick_folder(crate::i18n::strings().folder_dialog_title, owner) {
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
        let s = crate::i18n::strings();
        if let Err(err) = save_user_settings(&self.settings) {
            self.status(&s.status_save_basic_fail_fmt.replace("{}", &err));
            return;
        }
        sync_app_settings(&self.settings);
        match reload_animation_store() {
            Ok(_) => self.status(s.status_saved_pet),
            Err(err) => self.status(&s.status_saved_reload_fail_fmt.replace("{}", &err)),
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
        self.status(crate::i18n::strings().status_reset_basic);
    }
}

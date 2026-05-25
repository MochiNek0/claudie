use slint::{ModelRc, VecModel};

use crate::settings::{
    LlmProfile, apply_llm_profile_to_claude, current_claude_llm_profile, default_profile_id,
    ensure_claude_onboarding_complete, save_llm_profile_db,
};
use crate::ui::slint_views::SettingsWindow;

use super::{SettingsController, profile_label_for_message, shared, sync_app_llm_profiles};

impl SettingsController {
    pub(in crate::ui::settings_panel) fn previous_profile(&mut self) {
        if self.llm_db.profiles.is_empty() {
            return;
        }
        self.profile_index = if self.profile_index == 0 {
            self.llm_db.profiles.len() - 1
        } else {
            self.profile_index - 1
        };
        self.refresh_profile_fields();
    }

    pub(in crate::ui::settings_panel) fn select_profile(&mut self, index: i32) {
        if index < 0 || self.llm_db.profiles.is_empty() {
            return;
        }
        self.profile_index = (index as usize).min(self.llm_db.profiles.len() - 1);
        self.refresh_profile_fields();
    }

    pub(in crate::ui::settings_panel) fn next_profile(&mut self) {
        if self.llm_db.profiles.is_empty() {
            return;
        }
        self.profile_index = (self.profile_index + 1) % self.llm_db.profiles.len();
        self.refresh_profile_fields();
    }

    pub(in crate::ui::settings_panel) fn new_profile(&mut self) {
        self.profile_index = self.llm_db.profiles.len();
        if let Some(ui) = self.ui() {
            set_profile_fields(&ui, &LlmProfile::default());
            ui.set_profile_position(shared("New profile"));
            ui.set_status_message(shared("Editing a new profile."));
        }
    }

    pub(super) fn refresh_profile_fields(&self) {
        let Some(ui) = self.ui() else {
            return;
        };
        ui.set_profile_model(ModelRc::new(VecModel::from_iter(
            self.llm_db
                .profiles
                .iter()
                .map(|profile| shared(&profile.display_label()))
                .collect::<Vec<_>>(),
        )));
        ui.set_selected_profile_index(self.profile_index as i32);
        if let Some(profile) = self.llm_db.profiles.get(self.profile_index) {
            set_profile_fields(&ui, profile);
            ui.set_profile_position(shared(&format!(
                "{} of {}",
                self.profile_index + 1,
                self.llm_db.profiles.len()
            )));
        } else {
            set_profile_fields(&ui, &LlmProfile::default());
            ui.set_profile_position(shared("New profile"));
        }
    }

    fn current_profile_from_fields(&self) -> Option<LlmProfile> {
        let ui = self.ui()?;
        Some(LlmProfile {
            id: ui.get_profile_id().to_string(),
            name: ui.get_profile_name().to_string(),
            base_url: ui.get_base_url().to_string(),
            auth_token: ui.get_auth_token().to_string(),
            api_key: ui.get_api_key().to_string(),
            model: ui.get_model().to_string(),
            opus_model: ui.get_opus_model().to_string(),
            sonnet_model: ui.get_sonnet_model().to_string(),
            haiku_model: ui.get_haiku_model().to_string(),
            openai_extra_body: ui.get_openai_extra_body().to_string(),
            extra_env: ui.get_extra_env().to_string(),
        })
    }

    pub(in crate::ui::settings_panel) fn save_profile(&mut self, activate_profile: bool) {
        let Some(mut profile) = self.current_profile_from_fields() else {
            return;
        };
        if profile.id.trim().is_empty() {
            profile.id = default_profile_id(&profile.name);
        }
        self.llm_db.upsert_profile(profile.clone());
        if activate_profile {
            self.llm_db.active_profile_id = profile.id.clone();
        }
        self.profile_index = self
            .llm_db
            .profiles
            .iter()
            .position(|candidate| candidate.id == profile.id)
            .unwrap_or(0);
        if let Err(err) = save_llm_profile_db(&self.llm_db) {
            self.status(&format!("Failed to save profile: {err}"));
            return;
        }
        sync_app_llm_profiles(&self.llm_db, "saved LLM profile");
        if activate_profile {
            if let Err(err) = ensure_claude_onboarding_complete() {
                self.status(&format!(
                    "Saved profile, but Claude onboarding failed: {err}"
                ));
                return;
            }
            if let Err(err) = apply_llm_profile_to_claude(&profile) {
                self.status(&format!("Saved profile, but failed to apply it: {err}"));
                return;
            }
            self.status(&format!("Using {}.", profile_label_for_message(&profile)));
        } else {
            self.status(&format!("Saved {}.", profile_label_for_message(&profile)));
        }
        self.refresh_profile_fields();
    }

    pub(in crate::ui::settings_panel) fn import_current_profile(&mut self) {
        let Some(profile) = current_claude_llm_profile() else {
            self.status("No Claude Code LLM env values were found to import.");
            return;
        };
        self.llm_db.upsert_profile(profile.clone());
        self.llm_db.active_profile_id = profile.id.clone();
        self.profile_index = self
            .llm_db
            .profiles
            .iter()
            .position(|candidate| candidate.id == profile.id)
            .unwrap_or(0);
        if let Err(err) = save_llm_profile_db(&self.llm_db) {
            self.status(&format!("Failed to save imported profile: {err}"));
            return;
        }
        sync_app_llm_profiles(&self.llm_db, "imported current LLM profile");
        self.refresh_profile_fields();
        self.status(&format!(
            "Imported {}.",
            profile_label_for_message(&profile)
        ));
    }

    pub(in crate::ui::settings_panel) fn delete_profile(&mut self) {
        let Some(profile) = self.current_profile_from_fields() else {
            return;
        };
        if profile.id.trim() == "official" {
            self.status("The official profile cannot be deleted.");
            return;
        }
        let Some(removed) = self.llm_db.remove_profile(&profile.id) else {
            self.status("Profile was not found.");
            return;
        };
        self.profile_index = self
            .profile_index
            .min(self.llm_db.profiles.len().saturating_sub(1));
        if let Err(err) = save_llm_profile_db(&self.llm_db) {
            self.status(&format!("Failed to delete profile: {err}"));
            return;
        }
        sync_app_llm_profiles(&self.llm_db, "deleted LLM profile");
        self.refresh_profile_fields();
        self.status(&format!("Deleted {}.", profile_label_for_message(&removed)));
    }
}

fn set_profile_fields(ui: &SettingsWindow, profile: &LlmProfile) {
    ui.set_profile_id(shared(&profile.id));
    ui.set_profile_name(shared(&profile.name));
    ui.set_base_url(shared(&profile.base_url));
    ui.set_auth_token(shared(&profile.auth_token));
    ui.set_api_key(shared(&profile.api_key));
    ui.set_model(shared(&profile.model));
    ui.set_opus_model(shared(&profile.opus_model));
    ui.set_sonnet_model(shared(&profile.sonnet_model));
    ui.set_haiku_model(shared(&profile.haiku_model));
    ui.set_extra_env(shared(&profile.extra_env));
    ui.set_openai_extra_body(shared(&profile.openai_extra_body));
}

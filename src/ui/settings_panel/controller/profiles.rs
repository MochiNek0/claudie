use slint::{ComponentHandle, ModelRc, VecModel};

use crate::settings::{
    LlmProfile, apply_llm_profile_to_claude, current_claude_llm_profile, default_profile_id,
    ensure_claude_onboarding_complete, save_llm_profile_db,
};
use crate::ui::slint_views::SettingsWindow;
use crate::usage_display::provider_usage_display;

use super::{
    SettingsController, current_profile_usage_quota, profile_label_for_message, shared,
    sync_app_llm_profiles,
};

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
            // Deselect the picker so the live-usage timer does not repaint the
            // previously selected profile's usage onto the blank form.
            ui.set_selected_profile_index(-1);
            let s = crate::i18n::strings();
            set_profile_fields(&ui, &LlmProfile::default());
            set_empty_profile_usage_fields(&ui);
            ui.set_profile_position(shared(s.status_new_profile));
            ui.set_status_message(shared(s.status_editing_new));
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
            set_profile_usage_fields(
                &ui,
                profile,
                &self.llm_db.active_profile_id,
                &current_profile_usage_quota(profile),
            );
            ui.set_profile_position(shared(&format!(
                "{} of {}",
                self.profile_index + 1,
                self.llm_db.profiles.len()
            )));
        } else {
            set_profile_fields(&ui, &LlmProfile::default());
            set_empty_profile_usage_fields(&ui);
            ui.set_profile_position(shared(crate::i18n::strings().status_new_profile));
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
            model_1m: ui.get_model_1m(),
            opus_1m: ui.get_opus_1m(),
            sonnet_1m: ui.get_sonnet_1m(),
            haiku_1m: ui.get_haiku_1m(),
            hide_attribution: ui.get_hide_attribution(),
        })
    }

    pub(in crate::ui::settings_panel) fn fetch_models(&mut self) {
        let Some(profile) = self.current_profile_from_fields() else {
            return;
        };
        self.status(crate::i18n::strings().status_fetching_models);
        let weak = self.weak.clone();
        std::thread::spawn(move || {
            let result = crate::proxy::fetch_provider_models(&profile);
            let _ = weak.upgrade_in_event_loop(move |ui| {
                let s = crate::i18n::strings();
                match result {
                    Ok(models) => {
                        let count = models.len();
                        ui.set_available_models(ModelRc::new(VecModel::from_iter(
                            models.iter().map(|model| shared(model)).collect::<Vec<_>>(),
                        )));
                        ui.set_status_message(shared(
                            &s.status_fetched_fmt.replace("{}", &count.to_string()),
                        ));
                    }
                    Err(err) => {
                        ui.set_status_message(shared(
                            &s.status_fetch_fail_fmt.replace("{}", &err.to_string()),
                        ));
                    }
                }
                // Arrives outside any UI callback, so request a repaint
                // explicitly under the software renderer.
                ui.window().request_redraw();
            });
        });
    }

    pub(in crate::ui::settings_panel) fn toggle_env_tool_search(&mut self, enabled: bool) {
        self.set_env_flag("ENABLE_TOOL_SEARCH", "true", enabled);
    }

    pub(in crate::ui::settings_panel) fn toggle_env_no_autoupdate(&mut self, enabled: bool) {
        self.set_env_flag("DISABLE_AUTOUPDATER", "1", enabled);
    }

    pub(in crate::ui::settings_panel) fn toggle_env_max_thinking(&mut self, enabled: bool) {
        self.set_env_flag("CLAUDE_CODE_EFFORT_LEVEL", "max", enabled);
    }

    /// Add or remove a `KEY=VALUE` line in the Extra env text box, then resync
    /// the quick-toggle pills with the new text.
    fn set_env_flag(&self, key: &str, value: &str, enabled: bool) {
        let Some(ui) = self.ui() else {
            return;
        };
        let current = ui.get_extra_env().to_string();
        let updated = extra_env_with_flag(&current, key, enabled.then_some(value));
        ui.set_extra_env(shared(&updated));
        set_env_flag_checks(&ui, &updated);
    }

    /// Re-derive the quick-toggle pills after the user edits Extra env by hand.
    pub(in crate::ui::settings_panel) fn sync_env_flag_checks(&self, extra_env: &str) {
        if let Some(ui) = self.ui() {
            set_env_flag_checks(&ui, extra_env);
        }
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
        let s = crate::i18n::strings();
        if let Err(err) = save_llm_profile_db(&self.llm_db) {
            self.status(&s.status_save_profile_fail_fmt.replace("{}", &err));
            return;
        }
        sync_app_llm_profiles(&self.llm_db);
        if activate_profile {
            if let Err(err) = ensure_claude_onboarding_complete() {
                self.status(&s.status_onboard_fail_fmt.replace("{}", &err));
                return;
            }
            if let Err(err) = apply_llm_profile_to_claude(&profile) {
                self.status(&s.status_apply_fail_fmt.replace("{}", &err));
                return;
            }
            self.status(
                &s.status_using_fmt
                    .replace("{}", &profile_label_for_message(&profile)),
            );
        } else {
            self.status(
                &s.status_saved_fmt
                    .replace("{}", &profile_label_for_message(&profile)),
            );
        }
        self.refresh_profile_fields();
    }

    pub(in crate::ui::settings_panel) fn import_current_profile(&mut self) {
        let Some(profile) = current_claude_llm_profile() else {
            self.status(crate::i18n::strings().status_no_import);
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
            self.status(
                &crate::i18n::strings()
                    .status_import_save_fail_fmt
                    .replace("{}", &err),
            );
            return;
        }
        sync_app_llm_profiles(&self.llm_db);
        self.refresh_profile_fields();
        self.status(
            &crate::i18n::strings()
                .status_imported_fmt
                .replace("{}", &profile_label_for_message(&profile)),
        );
    }

    pub(in crate::ui::settings_panel) fn delete_profile(&mut self) {
        let Some(profile) = self.current_profile_from_fields() else {
            return;
        };
        let s = crate::i18n::strings();
        if profile.is_official() {
            self.status(s.status_official_no_delete);
            return;
        }
        let Some(removed) = self.llm_db.remove_profile(&profile.id) else {
            self.status(s.status_profile_not_found);
            return;
        };
        self.profile_index = self
            .profile_index
            .min(self.llm_db.profiles.len().saturating_sub(1));
        if let Err(err) = save_llm_profile_db(&self.llm_db) {
            self.status(&s.status_delete_fail_fmt.replace("{}", &err));
            return;
        }
        sync_app_llm_profiles(&self.llm_db);
        self.refresh_profile_fields();
        self.status(
            &s.status_deleted_fmt
                .replace("{}", &profile_label_for_message(&removed)),
        );
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
    ui.set_model_1m(profile.model_1m);
    ui.set_opus_1m(profile.opus_1m);
    ui.set_sonnet_1m(profile.sonnet_1m);
    ui.set_haiku_1m(profile.haiku_1m);
    ui.set_hide_attribution(profile.hide_attribution);
    set_env_flag_checks(ui, &profile.extra_env);
    // Drop any model list fetched for the previously shown profile.
    ui.set_available_models(ModelRc::new(VecModel::<slint::SharedString>::default()));
}

/// First value for `key` in a newline-delimited `KEY=VALUE` Extra env block,
/// matching the trimming that `parse_extra_env` applies on save.
fn extra_env_value(extra_env: &str, key: &str) -> Option<String> {
    extra_env.lines().find_map(|line| {
        let (candidate, value) = line.split_once('=')?;
        candidate
            .trim()
            .eq_ignore_ascii_case(key)
            .then(|| value.trim().to_string())
    })
}

/// Return `extra_env` with any existing line for `key` removed, then a fresh
/// `key=value` line appended when `value` is `Some`.
fn extra_env_with_flag(extra_env: &str, key: &str, value: Option<&str>) -> String {
    let mut lines: Vec<String> = extra_env
        .lines()
        .filter(|line| {
            let candidate = line.split('=').next().unwrap_or("").trim();
            !candidate.eq_ignore_ascii_case(key)
        })
        .map(|line| line.to_string())
        .collect();
    if let Some(value) = value {
        lines.push(format!("{key}={value}"));
    }
    lines.join("\n")
}

/// A pill is checked only when its exact enabled value is present, so deleting
/// or changing the line in Extra env unchecks it.
fn set_env_flag_checks(ui: &SettingsWindow, extra_env: &str) {
    let on = |key: &str, value: &str| extra_env_value(extra_env, key).as_deref() == Some(value);
    ui.set_env_tool_search(on("ENABLE_TOOL_SEARCH", "true"));
    ui.set_env_no_autoupdate(on("DISABLE_AUTOUPDATER", "1"));
    ui.set_env_max_thinking(on("CLAUDE_CODE_EFFORT_LEVEL", "max"));
}

pub(super) fn set_profile_usage_fields(
    ui: &SettingsWindow,
    profile: &LlmProfile,
    active_profile_id: &str,
    quota: &crate::app::QuotaStats,
) {
    let profile_name = profile.name.trim();
    let profile_name = if profile_name.is_empty() {
        profile.id.as_str()
    } else {
        profile_name
    };
    let usage = provider_usage_display(profile_name, &profile.id, active_profile_id, quota);
    ui.set_profile_usage_title(shared(&usage.title));
    ui.set_profile_usage_summary(shared(&usage.summary));
    ui.set_profile_usage_five_hour_value(shared(&usage.five_hour.value));
    ui.set_profile_usage_seven_day_value(shared(&usage.seven_day.value));
    ui.set_profile_usage_five_hour_reset(shared(&usage.five_hour.reset_caption()));
    ui.set_profile_usage_seven_day_reset(shared(&usage.seven_day.reset_caption()));
    ui.set_profile_usage_five_hour_bar(usage.five_hour.bar);
    ui.set_profile_usage_seven_day_bar(usage.seven_day.bar);
}

fn set_empty_profile_usage_fields(ui: &SettingsWindow) {
    let s = crate::i18n::strings();
    ui.set_profile_usage_title(shared(s.usage_provider_usage));
    ui.set_profile_usage_summary(shared(s.usage_save_or_select));
    ui.set_profile_usage_five_hour_value(shared("--"));
    ui.set_profile_usage_seven_day_value(shared("--"));
    ui.set_profile_usage_five_hour_reset(shared(""));
    ui.set_profile_usage_seven_day_reset(shared(""));
    ui.set_profile_usage_five_hour_bar(0.0);
    ui.set_profile_usage_seven_day_bar(0.0);
}

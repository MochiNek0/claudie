mod controller;

use std::cell::RefCell;
use std::rc::Rc;

use slint::ComponentHandle;
use windows_sys::Win32::Foundation::HWND;

use crate::app::pomodoro::PomodoroStatus;
use crate::ui::slint_views::{I18n, SettingsWindow};
use crate::ui::window_icon::{apply_slint_window_icons, schedule_settings_window_icon_refresh};
use crate::ui::window_position::center_window_on_screen;
use controller::{SettingsController, mutate_app_state};

thread_local! {
    static SETTINGS: RefCell<Option<SettingsWindow>> = const { RefCell::new(None) };
}

pub(crate) unsafe fn show_settings_panel(parent: HWND) {
    show_settings_panel_tab_for_parent(parent, 0);
}

fn show_settings_panel_tab_for_parent(parent: HWND, tab: i32) {
    SETTINGS.with(|slot| {
        // Always create a fresh window to avoid Slint rendering issues when
        // re-showing a previously hidden window (white screen bug).
        if let Some(old_window) = slot.borrow_mut().take() {
            let _ = old_window.hide();
        }

        crate::ui::ensure_embedded_fonts();
        let Ok(window) = SettingsWindow::new() else {
            return;
        };
        window.set_active_tab(tab);
        apply_settings_i18n(&window);
        let controller = Rc::new(RefCell::new(SettingsController::new(window.as_weak())));
        controller.borrow_mut().load_into_ui();
        wire_callbacks(&window, controller);
        center_window_on_screen(window.window(), parent, (880.0, 760.0));
        let _ = window.show();
        center_window_on_screen(window.window(), parent, (880.0, 760.0));
        apply_slint_window_icons(window.window());
        schedule_settings_window_icon_refresh(window.as_weak());
        *slot.borrow_mut() = Some(window);
    });
}

/// Request a repaint of the open settings window, if any. The software renderer
/// does not auto-repaint when another window (e.g. a permission popup) appears
/// over or beside it, so the panel must be told to redraw or it goes blank.
pub(crate) fn request_settings_redraw() {
    SETTINGS.with(|slot| {
        if let Some(window) = slot.borrow().as_ref() {
            window.window().request_redraw();
        }
    });
}

pub(crate) fn close_settings_panel() {
    SETTINGS.with(|slot| {
        if let Some(window) = slot.borrow_mut().take() {
            let _ = window.hide();
        }
    });
}

fn wire_callbacks(window: &SettingsWindow, controller: Rc<RefCell<SettingsController>>) {
    // The software renderer only repaints on an explicit request or a window
    // event, and Slint's "property changed -> auto redraw" path is unreliable
    // here. Every callback below mutates UI properties, so wrap each one to
    // request a redraw after it runs; otherwise edits (e.g. switching the LLM
    // profile) update state but never repaint the form.
    let weak = window.as_weak();
    window.on_previous_profile(redraw_after(&weak, {
        let controller = controller.clone();
        move || {
            controller.borrow_mut().previous_profile();
        }
    }));
    window.on_next_profile(redraw_after(&weak, {
        let controller = controller.clone();
        move || {
            controller.borrow_mut().next_profile();
        }
    }));
    window.on_pet_scale_changed(redraw_after_arg(&weak, {
        let controller = controller.clone();
        move |value| {
            controller.borrow_mut().update_pet_scale_live(value);
        }
    }));
    window.on_sleep_after_changed(redraw_after_arg(&weak, {
        let controller = controller.clone();
        move |value| {
            controller.borrow_mut().update_sleep_after_live(value);
        }
    }));
    window.on_select_profile(redraw_after_arg(&weak, {
        let controller = controller.clone();
        move |index| {
            controller.borrow_mut().select_profile(index);
        }
    }));
    window.on_new_profile(redraw_after(&weak, {
        let controller = controller.clone();
        move || {
            controller.borrow_mut().new_profile();
        }
    }));
    window.on_save_profile(redraw_after(&weak, {
        let controller = controller.clone();
        move || {
            controller.borrow_mut().save_profile(false);
        }
    }));
    window.on_use_profile(redraw_after(&weak, {
        let controller = controller.clone();
        move || {
            controller.borrow_mut().save_profile(true);
        }
    }));
    window.on_import_profile(redraw_after(&weak, {
        let controller = controller.clone();
        move || {
            controller.borrow_mut().import_current_profile();
        }
    }));
    window.on_delete_profile(redraw_after(&weak, {
        let controller = controller.clone();
        move || {
            controller.borrow_mut().delete_profile();
        }
    }));
    window.on_fetch_models(redraw_after(&weak, {
        let controller = controller.clone();
        move || {
            controller.borrow_mut().fetch_models();
        }
    }));
    window.on_copy_launch_command(redraw_after(&weak, {
        let controller = controller.clone();
        move || {
            controller.borrow_mut().copy_launch_command();
        }
    }));
    window.on_toggle_env_tool_search(redraw_after_arg(&weak, {
        let controller = controller.clone();
        move |enabled| {
            controller.borrow_mut().toggle_env_tool_search(enabled);
        }
    }));
    window.on_toggle_env_no_autoupdate(redraw_after_arg(&weak, {
        let controller = controller.clone();
        move |enabled| {
            controller.borrow_mut().toggle_env_no_autoupdate(enabled);
        }
    }));
    window.on_toggle_env_max_thinking(redraw_after_arg(&weak, {
        let controller = controller.clone();
        move |enabled| {
            controller.borrow_mut().toggle_env_max_thinking(enabled);
        }
    }));
    window.on_extra_env_edited(redraw_after_arg(&weak, {
        let controller = controller.clone();
        move |text: slint::SharedString| {
            controller.borrow().sync_env_flag_checks(&text);
        }
    }));
    window.on_browse_gif_dir(redraw_after(&weak, {
        let controller = controller.clone();
        move || {
            controller.borrow_mut().browse_gif_dir();
        }
    }));
    window.on_clear_gif_dir(redraw_after(&weak, {
        let controller = controller.clone();
        move || {
            controller.borrow_mut().clear_gif_dir();
        }
    }));
    window.on_save_basic(redraw_after(&weak, {
        let controller = controller.clone();
        move || {
            controller.borrow_mut().save_basic_settings();
        }
    }));
    window.on_reset_basic(redraw_after(&weak, {
        let controller = controller.clone();
        move || {
            controller.borrow_mut().reset_basic_fields();
        }
    }));
    window.on_language_changed(redraw_after_arg(&weak, {
        let controller = controller.clone();
        move |index| {
            controller.borrow_mut().change_language(index);
        }
    }));
    window.on_save_pomodoro(redraw_after(&weak, {
        let controller = controller.clone();
        move || {
            controller.borrow_mut().save_pomodoro_settings();
        }
    }));
    window.on_start_pomodoro(redraw_after(&weak, {
        let controller = controller.clone();
        move || {
            mutate_app_state(|state| state.start_pomodoro());
            controller.borrow_mut().refresh_pomodoro_tab();
        }
    }));
    window.on_pause_resume_pomodoro(redraw_after(&weak, {
        let controller = controller.clone();
        move || {
            mutate_app_state(|state| {
                if state.pomodoro.status == PomodoroStatus::Paused {
                    state.resume_pomodoro();
                } else {
                    state.pause_pomodoro();
                }
            });
            controller.borrow_mut().refresh_pomodoro_tab();
        }
    }));
    window.on_skip_pomodoro(redraw_after(&weak, {
        let controller = controller.clone();
        move || {
            mutate_app_state(|state| state.skip_pomodoro());
            controller.borrow_mut().refresh_pomodoro_tab();
        }
    }));
    window.on_stop_pomodoro(redraw_after(&weak, {
        let controller = controller.clone();
        move || {
            mutate_app_state(|state| state.stop_pomodoro());
            controller.borrow_mut().refresh_pomodoro_tab();
        }
    }));

    window.window().on_close_requested(|| {
        // Drop the stored window so the renderer context is released; a
        // hidden-but-alive window keeps its GPU buffers committed.
        close_settings_panel();
        slint::CloseRequestResponse::HideWindow
    });
}

/// Push the active language's text into the window's `I18n` global. Slint
/// cannot read a Rust global, so every localized markup binding is fed here at
/// window creation (and again after a live language switch).
pub(crate) fn apply_settings_i18n(window: &SettingsWindow) {
    let s = crate::i18n::strings();
    let g = window.global::<I18n>();
    g.set_settings_title(s.settings_title.into());
    g.set_settings_subtitle(s.settings_subtitle.into());
    g.set_tab_basic(s.tab_basic.into());
    g.set_tab_pomodoro(s.tab_pomodoro.into());
    g.set_tab_llm(s.tab_llm.into());
    g.set_tab_stats(s.tab_stats.into());
    g.set_btn_save(s.btn_save.into());
    g.set_btn_reset(s.btn_reset.into());

    g.set_basic_header(s.basic_header.into());
    g.set_basic_sub(s.basic_sub.into());
    g.set_basic_pet_size(s.basic_pet_size.into());
    g.set_basic_sleep_after(s.basic_sleep_after.into());
    g.set_basic_gif_folder(s.basic_gif_folder.into());
    g.set_btn_browse(s.btn_browse.into());
    g.set_btn_use_default(s.btn_use_default.into());
    g.set_basic_language(s.basic_language.into());
    g.set_basic_session_switcher(s.basic_session_switcher.into());
    g.set_basic_session_switcher_desc(s.basic_session_switcher_desc.into());

    g.set_pomo_header(s.pomo_header.into());
    g.set_pomo_sub(s.pomo_sub.into());
    g.set_pomo_current_cycle(s.pomo_current_cycle.into());
    g.set_pomo_tune(s.pomo_tune.into());
    g.set_pomo_durations(s.pomo_durations.into());
    g.set_pomo_focus(s.pomo_focus.into());
    g.set_pomo_focus_hint(s.pomo_focus_hint.into());
    g.set_pomo_short_break(s.pomo_short_break.into());
    g.set_pomo_short_hint(s.pomo_short_hint.into());
    g.set_pomo_long_break(s.pomo_long_break.into());
    g.set_pomo_long_hint(s.pomo_long_hint.into());
    g.set_pomo_start(s.pomo_start.into());
    g.set_pomo_skip(s.pomo_skip.into());
    g.set_pomo_stop(s.pomo_stop.into());

    g.set_llm_header(s.llm_header.into());
    g.set_llm_sub(s.llm_sub.into());
    g.set_llm_profile(s.llm_profile.into());
    g.set_btn_new(s.btn_new.into());
    g.set_btn_import_current(s.btn_import_current.into());
    g.set_btn_delete(s.btn_delete.into());
    g.set_field_profile_id(s.field_profile_id.into());
    g.set_field_name(s.field_name.into());
    g.set_field_base_url(s.field_base_url.into());
    g.set_field_api_key(s.field_api_key.into());
    g.set_field_auth_token(s.field_auth_token.into());
    g.set_llm_models(s.llm_models.into());
    g.set_llm_models_hint(s.llm_models_hint.into());
    g.set_btn_fetch_models(s.btn_fetch_models.into());
    g.set_btn_copy_launch_command(s.btn_copy_launch_command.into());
    g.set_field_default_model(s.field_default_model.into());
    g.set_field_opus(s.field_opus.into());
    g.set_field_sonnet(s.field_sonnet.into());
    g.set_field_haiku(s.field_haiku.into());
    g.set_llm_quick_switches(s.llm_quick_switches.into());
    g.set_env_tool_search(s.env_tool_search.into());
    g.set_env_no_autoupdate(s.env_no_autoupdate.into());
    g.set_env_max_thinking(s.env_max_thinking.into());
    g.set_env_hide_attribution(s.env_hide_attribution.into());
    g.set_llm_extra_env(s.llm_extra_env.into());
    g.set_llm_openai_body(s.llm_openai_body.into());
    g.set_btn_use(s.btn_use.into());

    g.set_stats_header(s.stats_header.into());
    g.set_stats_sub(s.stats_sub.into());
    g.set_stats_kpi_prompts(s.stats_kpi_prompts.into());
    g.set_stats_kpi_tokens(s.stats_kpi_tokens.into());
    g.set_stats_kpi_cache(s.stats_kpi_cache.into());
    g.set_stats_kpi_tools(s.stats_kpi_tools.into());
    g.set_stats_activity(s.stats_activity.into());
    g.set_stats_tool_mix(s.stats_tool_mix.into());
    g.set_stats_tokens_7d(s.stats_tokens_7d.into());
    g.set_stats_tokens_by_model(s.stats_tokens_by_model.into());
    g.set_stats_model_hint(s.stats_model_hint.into());
    g.set_stat_write(s.stat_write.into());
    g.set_stat_bash(s.stat_bash.into());
    g.set_stat_search(s.stat_search.into());
    g.set_stat_agent(s.stat_agent.into());
    g.set_stat_perm(s.stat_perm.into());
    g.set_stat_choice(s.stat_choice.into());
    g.set_stat_input(s.stat_input.into());
    g.set_stat_output(s.stat_output.into());
    g.set_stat_cache_w(s.stat_cache_w.into());
    g.set_stat_cache_r(s.stat_cache_r.into());
}

/// Wrap a zero-argument callback so the window repaints after it runs. See the
/// note in `wire_callbacks` for why this is needed with the software renderer.
fn redraw_after(
    weak: &slint::Weak<SettingsWindow>,
    action: impl Fn() + 'static,
) -> impl Fn() + 'static {
    let weak = weak.clone();
    move || {
        action();
        if let Some(ui) = weak.upgrade() {
            ui.window().request_redraw();
        }
    }
}

/// Same as [`redraw_after`] for callbacks that take a single argument.
fn redraw_after_arg<A>(
    weak: &slint::Weak<SettingsWindow>,
    action: impl Fn(A) + 'static,
) -> impl Fn(A) + 'static {
    let weak = weak.clone();
    move |arg| {
        action(arg);
        if let Some(ui) = weak.upgrade() {
            ui.window().request_redraw();
        }
    }
}

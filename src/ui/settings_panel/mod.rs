mod controller;

use std::cell::RefCell;
use std::rc::Rc;

use slint::ComponentHandle;
use windows_sys::Win32::Foundation::HWND;

use crate::app::pomodoro::PomodoroStatus;
use crate::ui::slint_views::SettingsWindow;
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

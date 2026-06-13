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
    window.on_previous_profile({
        let controller = controller.clone();
        move || {
            controller.borrow_mut().previous_profile();
        }
    });
    window.on_next_profile({
        let controller = controller.clone();
        move || {
            controller.borrow_mut().next_profile();
        }
    });
    window.on_pet_scale_changed({
        let controller = controller.clone();
        move |value| {
            controller.borrow_mut().update_pet_scale_live(value);
        }
    });
    window.on_sleep_after_changed({
        let controller = controller.clone();
        move |value| {
            controller.borrow_mut().update_sleep_after_live(value);
        }
    });
    window.on_select_profile({
        let controller = controller.clone();
        move |index| {
            controller.borrow_mut().select_profile(index);
        }
    });
    window.on_new_profile({
        let controller = controller.clone();
        move || {
            controller.borrow_mut().new_profile();
        }
    });
    window.on_save_profile({
        let controller = controller.clone();
        move || {
            controller.borrow_mut().save_profile(false);
        }
    });
    window.on_use_profile({
        let controller = controller.clone();
        move || {
            controller.borrow_mut().save_profile(true);
        }
    });
    window.on_import_profile({
        let controller = controller.clone();
        move || {
            controller.borrow_mut().import_current_profile();
        }
    });
    window.on_delete_profile({
        let controller = controller.clone();
        move || {
            controller.borrow_mut().delete_profile();
        }
    });
    window.on_anim_field_changed({
        let controller = controller.clone();
        move |index, text| {
            controller.borrow_mut().set_anim_value(index, text.as_str());
        }
    });
    window.on_save_basic({
        let controller = controller.clone();
        move || {
            controller.borrow_mut().save_basic_settings();
        }
    });
    window.on_reset_basic({
        let controller = controller.clone();
        move || {
            controller.borrow_mut().reset_basic_fields();
        }
    });
    window.on_save_pomodoro({
        let controller = controller.clone();
        move || {
            controller.borrow_mut().save_pomodoro_settings();
        }
    });
    window.on_start_pomodoro({
        let controller = controller.clone();
        move || {
            mutate_app_state(|state| state.start_pomodoro());
            controller.borrow_mut().refresh_pomodoro_tab();
        }
    });
    window.on_pause_resume_pomodoro({
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
    });
    window.on_skip_pomodoro({
        let controller = controller.clone();
        move || {
            mutate_app_state(|state| state.skip_pomodoro());
            controller.borrow_mut().refresh_pomodoro_tab();
        }
    });
    window.on_stop_pomodoro({
        let controller = controller.clone();
        move || {
            mutate_app_state(|state| state.stop_pomodoro());
            controller.borrow_mut().refresh_pomodoro_tab();
        }
    });

    window.window().on_close_requested(|| {
        // Drop the stored window so the renderer context is released; a
        // hidden-but-alive window keeps its GPU buffers committed.
        close_settings_panel();
        slint::CloseRequestResponse::HideWindow
    });
}

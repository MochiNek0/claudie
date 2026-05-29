mod controller;

use std::cell::RefCell;
use std::rc::Rc;

use slint::ComponentHandle;
use windows_sys::Win32::Foundation::HWND;

use crate::app::pomodoro::PomodoroStatus;
use crate::ui::slint_views::SettingsWindow;
use crate::ui::window_icon::{apply_slint_window_icons, schedule_settings_window_icon_refresh};
use controller::{SettingsController, mutate_app_state};

thread_local! {
    static SETTINGS: RefCell<Option<SettingsWindow>> = const { RefCell::new(None) };
}

pub(crate) unsafe fn show_settings_panel(_parent: HWND) {
    show_settings_panel_tab(0);
}

pub(crate) fn show_settings_panel_tab(tab: i32) {
    SETTINGS.with(|slot| {
        // Always create a fresh window to avoid Slint rendering issues when
        // re-showing a previously hidden window (white screen bug).
        if let Some(old_window) = slot.borrow_mut().take() {
            let _ = old_window.hide();
        }

        let Ok(window) = SettingsWindow::new() else {
            return;
        };
        window.set_active_tab(tab);
        let controller = Rc::new(RefCell::new(SettingsController::new(window.as_weak())));
        controller.borrow_mut().load_into_ui();
        wire_callbacks(&window, controller);
        let _ = window.show();
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
    let weak = window.as_weak();
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

    window.window().on_close_requested(move || {
        if weak.upgrade().is_some() {
            slint::CloseRequestResponse::HideWindow
        } else {
            slint::CloseRequestResponse::HideWindow
        }
    });
}

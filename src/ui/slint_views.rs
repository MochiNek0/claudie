slint::slint! {
    import { LineEdit, TextEdit } from "std-widgets.slint";

    component ActionButton inherits Rectangle {
        in property <string> text;
        in property <bool> enabled: true;
        callback clicked();

        border-radius: 8px;
        border-width: 1px;
        border-color: root.enabled ? #cfd8e6 : #e5e7eb;
        background: root.enabled ? #ffffff : #f3f4f6;

        states [
            hover when touch.has-hover && root.enabled : {
                background: #eef6ff;
                border-color: #0a84ff;
            }
            pressed when touch.pressed && root.enabled : {
                background: #dbeafe;
                border-color: #0060c6;
            }
        ]

        touch := TouchArea {
            width: 100%;
            height: 100%;
            enabled: root.enabled;
            mouse-cursor: pointer;
            clicked => {
                root.clicked();
            }
        }

        label := Text {
            width: 100%;
            height: 100%;
            text: root.text;
            horizontal-alignment: center;
            vertical-alignment: center;
            font-size: 13px;
            font-weight: 600;
            color: root.enabled ? #111827 : #9ca3af;
        }
    }

    component GhostButton inherits Rectangle {
        in property <string> text;
        in property <bool> enabled: true;
        callback clicked();

        border-radius: 7px;
        background: touch.pressed && root.enabled ? #dbeafe : (touch.has-hover && root.enabled ? #eef6ff : transparent);

        touch := TouchArea {
            width: 100%;
            height: 100%;
            enabled: root.enabled;
            mouse-cursor: pointer;
            clicked => {
                root.clicked();
            }
        }

        Text {
            width: 100%;
            height: 100%;
            text: root.text;
            horizontal-alignment: center;
            vertical-alignment: center;
            font-size: 15px;
            font-weight: 600;
            color: root.enabled ? #475569 : #9ca3af;
        }
    }

    component OptionRow inherits Rectangle {
        in property <string> text;
        callback clicked();

        border-radius: 6px;
        background: touch.pressed ? #dbeafe : (touch.has-hover ? #eef6ff : transparent);

        touch := TouchArea {
            width: 100%;
            height: 100%;
            mouse-cursor: pointer;
            clicked => {
                root.clicked();
            }
        }

        Text {
            x: 10px;
            width: root.width - 20px;
            height: 100%;
            text: root.text;
            overflow: elide;
            vertical-alignment: center;
            font-size: 13px;
            color: #111827;
        }
    }

    component MonoLineEdit inherits LineEdit {
        font-family: "Cascadia Mono, Consolas";
        font-size: 12px;
    }

    component MonoTextEdit inherits TextEdit {
        font-family: "Cascadia Mono, Consolas";
        font-size: 12px;
    }

    component PointerSlider inherits Rectangle {
        in property <float> minimum: 0;
        in property <float> maximum: 100;
        in property <float> step: 1;
        in-out property <float> value: minimum;
        callback changed(float);

        border-radius: 8px;
        background: transparent;

        property <float> span: max(1, root.maximum - root.minimum);
        property <float> pct: (root.value - root.minimum) / root.span;
        property <length> handle-size: 18px;
        property <length> handle-x: max(0px, min(root.width - root.handle-size, root.pct * (root.width - root.handle-size)));

        changed value => {
            root.changed(root.value);
        }

        function set-from-x(x: length) {
            root.value = max(root.minimum, min(root.maximum, Math.round((root.minimum + (x / root.width) * root.span) / root.step) * root.step));
        }

        Rectangle {
            x: 0px;
            y: (root.height - 8px) / 2;
            width: 100%;
            height: 8px;
            border-radius: 4px;
            background: #dbe4f0;
        }

        Rectangle {
            x: 0px;
            y: (root.height - 8px) / 2;
            width: root.handle-x + root.handle-size / 2;
            height: 8px;
            border-radius: 4px;
            background: #0a84ff;
        }

        Rectangle {
            x: root.handle-x;
            y: (root.height - root.handle-size) / 2;
            width: root.handle-size;
            height: root.handle-size;
            border-radius: root.handle-size / 2;
            background: touch.pressed ? #0060c6 : #ffffff;
            border-width: 2px;
            border-color: #0a84ff;
        }

        touch := TouchArea {
            width: 100%;
            height: 100%;
            mouse-cursor: pointer;
            pointer-event(event) => {
                if event.kind == PointerEventKind.down {
                    root.set-from-x(self.mouse-x);
                }
            }
            moved => {
                root.set-from-x(self.mouse-x);
            }
        }
    }

    component PointerSpinBox inherits Rectangle {
        in property <int> minimum: 1;
        in property <int> maximum: 240;
        in-out property <int> value: minimum;

        border-radius: 8px;
        border-width: 1px;
        border-color: #cfd8e6;
        background: #ffffff;

        TouchArea {
            width: 100%;
            height: 100%;
            mouse-cursor: pointer;
        }

        Text {
            x: 34px;
            y: 0px;
            width: root.width - 68px;
            height: 100%;
            text: root.value;
            horizontal-alignment: center;
            vertical-alignment: center;
            font-size: 13px;
            color: #111827;
        }
        GhostButton {
            x: 2px; y: 2px; width: 30px; height: root.height - 4px;
            text: "-";
            clicked => { root.value = max(root.minimum, root.value - 1); }
        }
        GhostButton {
            x: root.width - 32px; y: 2px; width: 30px; height: root.height - 4px;
            text: "+";
            clicked => { root.value = min(root.maximum, root.value + 1); }
        }
    }

    component PointerComboBox inherits Rectangle {
        in property <[string]> model;
        in-out property <int> current-index: 0;
        callback selected(int);

        border-radius: 8px;
        border-width: 1px;
        border-color: touch.has-hover ? #0a84ff : #cfd8e6;
        background: touch.pressed ? #dbeafe : (touch.has-hover ? #eef6ff : #ffffff);

        property <string> current-value: root.current-index >= 0 && root.current-index < root.model.length ? root.model[root.current-index] : "";

        touch := TouchArea {
            width: 100%;
            height: 100%;
            mouse-cursor: pointer;
            clicked => {
                popup.show();
            }
        }

        Text {
            x: 12px;
            y: 0px;
            width: root.width - 42px;
            height: 100%;
            text: root.current-value;
            overflow: elide;
            vertical-alignment: center;
            font-size: 13px;
            color: #111827;
        }
        Text {
            x: root.width - 30px;
            y: 0px;
            width: 18px;
            height: 100%;
            text: "⌄";
            horizontal-alignment: center;
            vertical-alignment: center;
            color: touch.has-hover ? #0a84ff : #64748b;
            font-size: 18px;
        }

        popup := PopupWindow {
            x: 0px;
            y: root.height + 4px;
            width: root.width;
            height: min(root.model.length, 7) * 32px + 8px;
            close-policy: close-on-click-outside;

            Rectangle {
                width: 100%;
                height: 100%;
                background: #ffffff;
                border-radius: 8px;
                border-width: 1px;
                border-color: #cfd8e6;
            }
            VerticalLayout {
                x: 4px;
                y: 4px;
                width: root.width - 8px;
                spacing: 2px;
                for item[index] in root.model : OptionRow {
                    height: 30px;
                    text: item;
                    clicked => {
                        root.current-index = index;
                        root.selected(index);
                        popup.close();
                    }
                }
            }
        }
    }

    export component SettingsWindow inherits Window {
        width: 880px;
        height: 700px;
        title: "claudie Settings";
        icon: @image-url("../../assets/icon.ico");
        background: #f4f7fc;

        in-out property <int> active_tab: 0;
        in-out property <float> pet_scale: 80;
        in-out property <float> sleep_after: 75;
        in-out property <string> pet_dir;
        in-out property <string> gif_dir;
        in-out property <string> anim_idle;
        in-out property <string> anim_thinking;
        in-out property <string> anim_typing;
        in-out property <string> anim_building;
        in-out property <string> anim_permission;
        in-out property <string> anim_happy;
        in-out property <string> anim_error;
        in-out property <string> anim_sleeping;
        in-out property <string> anim_subagent;

        in-out property <int> focus_minutes: 25;
        in-out property <int> short_break_minutes: 5;
        in-out property <int> long_break_minutes: 15;
        in property <string> pomodoro_status;
        in property <string> pause_resume_label: "Pause";

        in property <string> profile_position;
        in property <[string]> profile_model;
        in-out property <int> selected_profile_index: 0;
        in-out property <string> profile_id;
        in-out property <string> profile_name;
        in-out property <string> base_url;
        in-out property <string> auth_token;
        in-out property <string> api_key;
        in-out property <string> model;
        in-out property <string> opus_model;
        in-out property <string> sonnet_model;
        in-out property <string> haiku_model;
        in-out property <string> extra_env;
        in-out property <string> openai_extra_body;

        in property <string> status_message;

        callback pet_scale_changed(float);
        callback sleep_after_changed(float);
        callback select_profile(int);
        callback previous_profile();
        callback next_profile();
        callback new_profile();
        callback save_profile();
        callback use_profile();
        callback import_profile();
        callback delete_profile();
        callback save_basic();
        callback reset_basic();
        callback save_pomodoro();
        callback start_pomodoro();
        callback pause_resume_pomodoro();
        callback skip_pomodoro();
        callback stop_pomodoro();

        Rectangle {
            x: 16px;
            y: 16px;
            width: 848px;
            height: 668px;
            background: white;
            border-radius: 18px;
            border-width: 1px;
            border-color: #e4e8f0;
        }

        Text {
            x: 36px;
            y: 28px;
            text: "claudie Settings";
            font-size: 22px;
            font-weight: 600;
            color: #111827;
        }

        Text {
            x: 36px;
            y: 57px;
            width: 720px;
            text: "Keep the pet light, tune the local runtime, and manage Claude Code profiles.";
            font-size: 13px;
            color: #6b7280;
        }

        ActionButton { x: 36px; y: 84px; width: 104px; height: 34px; text: "Basic"; clicked => { root.active_tab = 0; } }
        ActionButton { x: 148px; y: 84px; width: 124px; height: 34px; text: "Pomodoro"; clicked => { root.active_tab = 1; } }
        ActionButton { x: 280px; y: 84px; width: 140px; height: 34px; text: "LLM Profiles"; clicked => { root.active_tab = 2; } }

        if active_tab == 0: Rectangle {
            x: 36px;
            y: 132px;
            width: 808px;
            height: 508px;
            background: transparent;

            Text { x: 12px; y: 0px; text: "Pet renderer"; font-size: 16px; font-weight: 600; color: #111827; }
            Text { x: 12px; y: 28px; width: 700px; text: "Tune the desktop pet size and map each mood to a GIF filename."; font-size: 13px; color: #6b7280; }

            Text { x: 12px; y: 72px; text: "Pet size"; color: #6b7280; font-size: 12px; }
            PointerSlider {
                x: 12px; y: 94px; width: 342px; height: 38px;
                minimum: 50; maximum: 150; step: 1;
                value <=> root.pet_scale;
                changed(value) => { root.pet_scale_changed(value); }
            }
            Text { x: 370px; y: 103px; width: 64px; text: Math.round(root.pet_scale) + "%"; color: #111827; font-size: 13px; }
            Text { x: 444px; y: 72px; text: "Sleep after"; color: #6b7280; font-size: 12px; }
            PointerSlider {
                x: 444px; y: 94px; width: 294px; height: 38px;
                minimum: 15; maximum: 1800; step: 15;
                value <=> root.sleep_after;
                changed(value) => { root.sleep_after_changed(value); }
            }
            Text { x: 752px; y: 103px; width: 70px; text: Math.round(root.sleep_after) + "s"; color: #111827; font-size: 13px; }

            Text { x: 12px; y: 150px; text: "Pet asset directory"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 12px; y: 172px; width: 382px; height: 34px; text <=> root.pet_dir; }
            Text { x: 412px; y: 150px; text: "GIF directory"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 412px; y: 172px; width: 382px; height: 34px; text <=> root.gif_dir; }

            Text { x: 12px; y: 228px; text: "Idle"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 12px; y: 248px; width: 182px; height: 32px; text <=> root.anim_idle; }
            Text { x: 212px; y: 228px; text: "Thinking"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 212px; y: 248px; width: 182px; height: 32px; text <=> root.anim_thinking; }
            Text { x: 412px; y: 228px; text: "Typing"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 412px; y: 248px; width: 182px; height: 32px; text <=> root.anim_typing; }
            Text { x: 612px; y: 228px; text: "Building"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 612px; y: 248px; width: 182px; height: 32px; text <=> root.anim_building; }

            Text { x: 12px; y: 296px; text: "Permission"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 12px; y: 316px; width: 182px; height: 32px; text <=> root.anim_permission; }
            Text { x: 212px; y: 296px; text: "Happy"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 212px; y: 316px; width: 182px; height: 32px; text <=> root.anim_happy; }
            Text { x: 412px; y: 296px; text: "Error"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 412px; y: 316px; width: 182px; height: 32px; text <=> root.anim_error; }
            Text { x: 612px; y: 296px; text: "Sleeping"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 612px; y: 316px; width: 182px; height: 32px; text <=> root.anim_sleeping; }

            Text { x: 12px; y: 364px; text: "Subagent"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 12px; y: 384px; width: 182px; height: 32px; text <=> root.anim_subagent; }

            ActionButton { x: 594px; y: 460px; width: 96px; height: 34px; text: "Save"; clicked => { root.save_basic(); } }
            ActionButton { x: 702px; y: 460px; width: 96px; height: 34px; text: "Reset"; clicked => { root.reset_basic(); } }
        }

        if active_tab == 1: Rectangle {
            x: 36px;
            y: 132px;
            width: 808px;
            height: 508px;
            background: transparent;

            Text { x: 12px; y: 0px; text: "Pomodoro"; font-size: 16px; font-weight: 600; color: #111827; }
            Text { x: 12px; y: 28px; width: 700px; text: "Set focus and break lengths, then control the active timer."; font-size: 13px; color: #6b7280; }

            Text { x: 12px; y: 82px; text: "Focus min"; color: #6b7280; font-size: 12px; }
            PointerSpinBox {
                x: 12px; y: 104px; width: 120px; height: 34px;
                minimum: 1; maximum: 240; value <=> root.focus_minutes;
            }
            Text { x: 150px; y: 82px; text: "Short break"; color: #6b7280; font-size: 12px; }
            PointerSpinBox {
                x: 150px; y: 104px; width: 120px; height: 34px;
                minimum: 1; maximum: 240; value <=> root.short_break_minutes;
            }
            Text { x: 288px; y: 82px; text: "Long break"; color: #6b7280; font-size: 12px; }
            PointerSpinBox {
                x: 288px; y: 104px; width: 120px; height: 34px;
                minimum: 1; maximum: 240; value <=> root.long_break_minutes;
            }
            ActionButton { x: 430px; y: 104px; width: 96px; height: 34px; text: "Save"; clicked => { root.save_pomodoro(); } }

            Rectangle { x: 12px; y: 178px; width: 520px; height: 96px; background: #f2f5fa; border-radius: 10px; border-width: 1px; border-color: #dae0ea; }
            Text { x: 32px; y: 198px; width: 480px; text: root.pomodoro_status; color: #111827; font-size: 14px; }

            ActionButton { x: 12px; y: 312px; width: 110px; height: 34px; text: "Start"; clicked => { root.start_pomodoro(); } }
            ActionButton { x: 132px; y: 312px; width: 110px; height: 34px; text: root.pause_resume_label; clicked => { root.pause_resume_pomodoro(); } }
            ActionButton { x: 252px; y: 312px; width: 110px; height: 34px; text: "Skip"; clicked => { root.skip_pomodoro(); } }
            ActionButton { x: 372px; y: 312px; width: 110px; height: 34px; text: "Stop"; clicked => { root.stop_pomodoro(); } }
        }

        if active_tab == 2: Rectangle {
            x: 36px;
            y: 132px;
            width: 808px;
            height: 508px;
            background: transparent;

            Text { x: 12px; y: 0px; text: "Provider profiles"; font-size: 16px; font-weight: 600; color: #111827; }
            Text { x: 12px; y: 28px; width: 700px; text: "Keep Claude Code provider settings tidy without leaving the pet."; font-size: 13px; color: #6b7280; }

            Text { x: 12px; y: 72px; text: "Profile"; color: #6b7280; font-size: 12px; }
            PointerComboBox {
                x: 12px; y: 94px; width: 340px; height: 34px;
                model: root.profile_model;
                current-index <=> root.selected_profile_index;
                selected(index) => { root.select_profile(index); }
            }
            Text { x: 364px; y: 101px; width: 116px; text: root.profile_position; color: #6b7280; font-size: 12px; }
            ActionButton { x: 500px; y: 94px; width: 72px; height: 32px; text: "New"; clicked => { root.new_profile(); } }
            ActionButton { x: 584px; y: 94px; width: 128px; height: 32px; text: "Import Current"; clicked => { root.import_profile(); } }
            ActionButton { x: 724px; y: 94px; width: 70px; height: 32px; text: "Delete"; clicked => { root.delete_profile(); } }

            Text { x: 12px; y: 144px; text: "Profile ID"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 12px; y: 164px; width: 382px; height: 34px; text <=> root.profile_id; }
            Text { x: 412px; y: 144px; text: "Name"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 412px; y: 164px; width: 382px; height: 34px; text <=> root.profile_name; }

            Text { x: 12px; y: 208px; text: "Model"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 12px; y: 228px; width: 382px; height: 34px; text <=> root.model; }
            Text { x: 412px; y: 208px; text: "Base URL"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 412px; y: 228px; width: 382px; height: 34px; text <=> root.base_url; }

            Text { x: 12px; y: 272px; text: "API key"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 12px; y: 292px; width: 382px; height: 34px; text <=> root.api_key; }
            Text { x: 412px; y: 272px; text: "Auth token (proxy)"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 412px; y: 292px; width: 382px; height: 34px; text <=> root.auth_token; }

            Text { x: 12px; y: 336px; text: "Opus"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 12px; y: 356px; width: 248px; height: 34px; text <=> root.opus_model; }
            Text { x: 280px; y: 336px; text: "Sonnet"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 280px; y: 356px; width: 248px; height: 34px; text <=> root.sonnet_model; }
            Text { x: 548px; y: 336px; text: "Haiku"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 548px; y: 356px; width: 246px; height: 34px; text <=> root.haiku_model; }

            Text { x: 12px; y: 400px; text: "Extra env"; color: #6b7280; font-size: 12px; }
            MonoTextEdit { x: 12px; y: 420px; width: 382px; height: 64px; text <=> root.extra_env; }
            Text { x: 412px; y: 400px; text: "OpenAI body"; color: #6b7280; font-size: 12px; }
            MonoTextEdit { x: 412px; y: 420px; width: 382px; height: 64px; text <=> root.openai_extra_body; }

            ActionButton { x: 594px; y: 500px; width: 96px; height: 34px; text: "Save"; clicked => { root.save_profile(); } }
            ActionButton { x: 702px; y: 500px; width: 96px; height: 34px; text: "Use"; clicked => { root.use_profile(); } }
        }

        Text {
            x: 36px;
            y: 652px;
            width: 808px;
            height: 20px;
            text: root.status_message;
            color: #6b7280;
            font-size: 12px;
        }
    }

    export component PromptWindow inherits Window {
        width: 560px;
        height: 520px;
        title: "claudie request";
        icon: @image-url("../../assets/icon.ico");
        background: #f4f7fc;

        in property <bool> is_choice: false;
        in property <string> title_text;
        in property <string> subtitle_text;
        in property <string> detail_text;
        in property <string> meta_text;
        in property <bool> submit_enabled: false;

        in property <bool> option0_visible: false;
        in property <bool> option1_visible: false;
        in property <bool> option2_visible: false;
        in property <bool> option3_visible: false;
        in property <bool> option4_visible: false;
        in property <bool> option5_visible: false;
        in property <bool> option6_visible: false;
        in property <bool> option7_visible: false;
        in property <string> option0_text;
        in property <string> option1_text;
        in property <string> option2_text;
        in property <string> option3_text;
        in property <string> option4_text;
        in property <string> option5_text;
        in property <string> option6_text;
        in property <string> option7_text;

        callback allow_once();
        callback allow_always();
        callback deny();
        callback submit_choice();
        callback cancel_choice();
        callback toggle_option0();
        callback toggle_option1();
        callback toggle_option2();
        callback toggle_option3();
        callback toggle_option4();
        callback toggle_option5();
        callback toggle_option6();
        callback toggle_option7();

        Rectangle {
            x: 14px;
            y: 14px;
            width: 532px;
            height: 492px;
            background: white;
            border-radius: 18px;
            border-width: 1px;
            border-color: #e4e8f0;
        }
        Text { x: 38px; y: 36px; width: 480px; text: root.title_text; font-size: 20px; font-weight: 600; color: #111827; }
        Text { x: 38px; y: 68px; width: 480px; text: root.subtitle_text; font-size: 13px; color: #6b7280; }
        Rectangle { x: 38px; y: 104px; width: 484px; height: 156px; background: #f2f5fa; border-radius: 10px; border-width: 1px; border-color: #dae0ea; }
        Text { x: 58px; y: 124px; width: 444px; height: 94px; text: root.detail_text; wrap: word-wrap; font-size: 13px; color: #111827; }
        Text { x: 58px; y: 222px; width: 444px; height: 24px; text: root.meta_text; font-size: 12px; color: #9ca3af; }

        if is_choice: Rectangle {
            x: 38px;
            y: 278px;
            width: 484px;
            height: 144px;
            background: transparent;
            if option0_visible: ActionButton { x: 0px; y: 0px; width: 232px; height: 30px; text: root.option0_text; clicked => { root.toggle_option0(); } }
            if option1_visible: ActionButton { x: 252px; y: 0px; width: 232px; height: 30px; text: root.option1_text; clicked => { root.toggle_option1(); } }
            if option2_visible: ActionButton { x: 0px; y: 36px; width: 232px; height: 30px; text: root.option2_text; clicked => { root.toggle_option2(); } }
            if option3_visible: ActionButton { x: 252px; y: 36px; width: 232px; height: 30px; text: root.option3_text; clicked => { root.toggle_option3(); } }
            if option4_visible: ActionButton { x: 0px; y: 72px; width: 232px; height: 30px; text: root.option4_text; clicked => { root.toggle_option4(); } }
            if option5_visible: ActionButton { x: 252px; y: 72px; width: 232px; height: 30px; text: root.option5_text; clicked => { root.toggle_option5(); } }
            if option6_visible: ActionButton { x: 0px; y: 108px; width: 232px; height: 30px; text: root.option6_text; clicked => { root.toggle_option6(); } }
            if option7_visible: ActionButton { x: 252px; y: 108px; width: 232px; height: 30px; text: root.option7_text; clicked => { root.toggle_option7(); } }
        }

        if !is_choice: Rectangle {
            x: 38px;
            y: 278px;
            width: 484px;
            height: 50px;
            background: transparent;
            Text { x: 0px; y: 0px; width: 484px; text: "Use Ctrl+Shift+Y for Allow and Ctrl+Shift+N for Deny."; font-size: 12px; color: #6b7280; }
        }

        if !is_choice: ActionButton { x: 210px; y: 448px; width: 88px; height: 34px; text: "Allow"; clicked => { root.allow_once(); } }
        if !is_choice: ActionButton { x: 308px; y: 448px; width: 100px; height: 34px; text: "Always"; clicked => { root.allow_always(); } }
        if !is_choice: ActionButton { x: 418px; y: 448px; width: 88px; height: 34px; text: "Deny"; clicked => { root.deny(); } }

        if is_choice: ActionButton { x: 308px; y: 448px; width: 100px; height: 34px; text: "Submit"; enabled: root.submit_enabled; clicked => { root.submit_choice(); } }
        if is_choice: ActionButton { x: 418px; y: 448px; width: 88px; height: 34px; text: "Cancel"; clicked => { root.cancel_choice(); } }
    }
}

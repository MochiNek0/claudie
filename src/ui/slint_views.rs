slint::slint! {
    import { LineEdit, TextEdit, ScrollView } from "std-widgets.slint";

    component ActionButton inherits Rectangle {
        in property <string> text;
        in property <bool> enabled: true;
        // Filled accent treatment, used for the selected settings tab.
        in property <bool> active: false;
        callback clicked();

        border-radius: 999px;
        border-width: 1px;
        border-color: root.active ? #0a84ff : (root.enabled ? #cfd8e6 : #e5e7eb);
        background: root.active ? #0a84ff : (root.enabled ? #ffffff : #f3f4f6);

        states [
            hover when touch.has-hover && root.enabled && !root.active : {
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
            x: 8px;
            y: 4px;
            width: root.width - 16px;
            height: root.height - 8px;
            text: root.text;
            horizontal-alignment: center;
            vertical-alignment: center;
            font-size: 13px;
            font-weight: 600;
            color: root.active ? #ffffff : (root.enabled ? #111827 : #9ca3af);
        }
    }

    component GhostButton inherits Rectangle {
        in property <string> text;
        in property <bool> enabled: true;
        callback clicked();

        border-radius: 999px;
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
            x: 8px;
            y: 4px;
            width: root.width - 16px;
            height: root.height - 8px;
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

        border-radius: 8px;
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
        font-family: "Inter, Segoe UI";
        font-size: 12px;
    }

    component MonoTextEdit inherits TextEdit {
        font-family: "Inter, Segoe UI";
        font-size: 12px;
    }

    component TogglePill inherits Rectangle {
        in-out property <bool> checked: false;
        callback toggled(bool);

        border-radius: 8px;
        border-width: 1px;
        border-color: root.checked ? #0a84ff : #cfd8e6;
        background: root.checked ? #0a84ff : #ffffff;

        TouchArea {
            width: 100%;
            height: 100%;
            mouse-cursor: pointer;
            clicked => {
                root.checked = !root.checked;
                root.toggled(root.checked);
            }
        }

        Rectangle {
            x: root.checked ? root.width - 22px : 2px;
            y: 2px;
            width: 20px;
            height: 20px;
            border-radius: 8px;
            background: root.checked ? #ffffff : #cfd8e6;
        }
    }

    component SettingsTabButton inherits Rectangle {
        in property <string> text;
        in property <image> icon_source;
        in property <bool> active: false;
        callback clicked();

        border-radius: 8px;
        border-width: 1px;
        border-color: root.active ? #0a84ff : transparent;
        background: root.active ? #0a84ff : (touch.has-hover ? #eef6ff : transparent);

        states [
            pressed when touch.pressed : {
                background: root.active ? #0060c6 : #dbeafe;
                border-color: root.active ? #0060c6 : #bfdbfe;
            }
        ]

        touch := TouchArea {
            width: 100%;
            height: 100%;
            mouse-cursor: pointer;
            clicked => {
                root.clicked();
            }
        }

        Image {
            x: 10px;
            y: (root.height - 16px) / 2;
            width: 16px;
            height: 16px;
            source: root.icon_source;
            image-fit: contain;
            colorize: root.active ? #ffffff : #64748b;
        }

        Text {
            x: 34px;
            y: 4px;
            width: root.width - 42px;
            height: root.height - 8px;
            text: root.text;
            overflow: elide;
            vertical-alignment: center;
            font-size: 13px;
            font-weight: 600;
            color: root.active ? #ffffff : #334155;
        }
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

    component PomodoroDurationTile inherits Rectangle {
        in property <string> title;
        in property <string> hint;
        in property <brush> accent;
        in-out property <int> value: 25;

        border-radius: 8px;
        border-width: 1px;
        border-color: #dae0ea;
        background: #ffffff;

        Rectangle {
            x: 0px;
            y: 0px;
            width: 100%;
            height: 4px;
            border-radius: 8px;
            background: root.accent;
        }

        Text {
            x: 16px;
            y: 16px;
            width: root.width - 32px;
            text: root.title;
            color: #111827;
            font-size: 14px;
            font-weight: 700;
            overflow: elide;
        }

        Text {
            x: 16px;
            y: 38px;
            width: root.width - 32px;
            text: root.hint;
            color: #64748b;
            font-size: 12px;
            overflow: elide;
        }

        PointerSpinBox {
            x: 16px;
            y: 70px;
            width: root.width - 58px;
            height: 32px;
            minimum: 1;
            maximum: 240;
            value <=> root.value;
        }

        Text {
            x: root.width - 36px;
            y: 76px;
            width: 28px;
            text: "min";
            color: #64748b;
            font-size: 12px;
            vertical-alignment: center;
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
            height: min(root.model.length, 8) * 32px + 8px;
            close-policy: close-on-click-outside;

            Rectangle {
                width: 100%;
                height: 100%;
                background: #ffffff;
                border-radius: 8px;
                border-width: 1px;
                border-color: #cfd8e6;
            }
            ScrollView {
                x: 4px;
                y: 4px;
                width: root.width - 8px;
                height: popup.height - 8px;

                VerticalLayout {
                    padding: 0px;
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
    }

    component StatBarRow inherits Rectangle {
        in property <string> label;
        in property <string> value;
        in property <float> bar: 0;
        in property <brush> accent: #0a84ff;

        background: transparent;

        Text {
            x: 0px;
            y: 0px;
            width: 62px;
            height: root.height;
            text: root.label;
            overflow: elide;
            vertical-alignment: center;
            color: #475569;
            font-size: 12px;
        }
        Rectangle {
            x: 70px;
            y: (root.height - 6px) / 2;
            width: root.width - 116px;
            height: 6px;
            border-radius: 3px;
            background: #dbe4f0;
        }
        Rectangle {
            x: 70px;
            y: (root.height - 6px) / 2;
            width: max(4px, (root.width - 116px) * root.bar / 100);
            height: 6px;
            border-radius: 3px;
            background: root.accent;
        }
        Text {
            x: root.width - 38px;
            y: 0px;
            width: 38px;
            height: root.height;
            text: root.value;
            horizontal-alignment: right;
            vertical-alignment: center;
            color: #111827;
            font-size: 12px;
        }
    }

    export component SettingsWindow inherits Window {
        min-width: 400px;
        preferred-width: 800px;
        max-width: 800px;
        min-height: 300px;
        preferred-height: 600px;
        max-height: 600px;
        title: "claudie Settings";
        icon: @image-url("../../assets/icon.ico");
        background: #f4f7fc;
        default-font-family: "Inter, Segoe UI";
        default-font-size: 13px;

        property <length> content_width: 584px;

        in-out property <int> active_tab: 0;
        in-out property <float> pet_scale: 80;
        in-out property <float> sleep_after: 75;
        in-out property <bool> show_session_switcher: true;
        in-out property <string> pet_dir;
        in-out property <string> gif_dir;
        in-out property <string> anim_idle;
        in-out property <string> anim_thinking;
        in-out property <string> anim_typing;
        in-out property <string> anim_building;
        in-out property <string> anim_search;
        in-out property <string> anim_happy;
        in-out property <string> anim_error;
        in-out property <string> anim_sleeping;
        in-out property <string> anim_subagent;
        in-out property <string> anim_pomodoro;
        in-out property <string> anim_wave;
        in-out property <string> anim_stretch;
        in-out property <string> anim_fishing;
        in-out property <string> anim_fishing_reel;
        in-out property <string> anim_fishing_caught;
        in-out property <string> anim_fishing_missed;

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
        in property <string> profile_usage_title;
        in property <string> profile_usage_summary;
        in property <string> profile_usage_five_hour_value;
        in property <string> profile_usage_seven_day_value;
        in property <float> profile_usage_five_hour_bar;
        in property <float> profile_usage_seven_day_bar;

        in property <string> stats_today_title;
        in property <string> stats_today_summary;
        in property <string> stats_recent_title;
        in property <string> stats_recent_summary;
        in property <string> stats_today_write_value;
        in property <string> stats_today_bash_value;
        in property <string> stats_today_search_value;
        in property <string> stats_today_subagent_value;
        in property <string> stats_today_permission_value;
        in property <string> stats_today_choice_value;
        in property <float> stats_today_write_bar;
        in property <float> stats_today_bash_bar;
        in property <float> stats_today_search_bar;
        in property <float> stats_today_subagent_bar;
        in property <float> stats_today_permission_bar;
        in property <float> stats_today_choice_bar;
        in property <string> stats_recent_write_value;
        in property <string> stats_recent_bash_value;
        in property <string> stats_recent_search_value;
        in property <string> stats_recent_subagent_value;
        in property <string> stats_recent_permission_value;
        in property <string> stats_recent_choice_value;
        in property <float> stats_recent_write_bar;
        in property <float> stats_recent_bash_bar;
        in property <float> stats_recent_search_bar;
        in property <float> stats_recent_subagent_bar;
        in property <float> stats_recent_permission_bar;
        in property <float> stats_recent_choice_bar;
        in property <string> stats_today_input_value;
        in property <string> stats_today_output_value;
        in property <string> stats_today_cache_write_value;
        in property <string> stats_today_cache_read_value;
        in property <float> stats_today_input_bar;
        in property <float> stats_today_output_bar;
        in property <float> stats_today_cache_write_bar;
        in property <float> stats_today_cache_read_bar;
        in property <string> stats_recent_input_value;
        in property <string> stats_recent_output_value;
        in property <string> stats_recent_cache_write_value;
        in property <string> stats_recent_cache_read_value;
        in property <float> stats_recent_input_bar;
        in property <float> stats_recent_output_bar;
        in property <float> stats_recent_cache_write_bar;
        in property <float> stats_recent_cache_read_bar;

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

        if false: Rectangle {
        // Frosted card: 16px window margin, 8pt-grid radius.
        Rectangle {
            x: 16px;
            y: 16px;
            width: 848px;
            height: 728px;
            background: white;
            border-radius: 16px;
            border-width: 1px;
            border-color: #e4e8f0;
        }

        // H1 (modular scale, base 13 * 1.25^3 ≈ 24).
        Text {
            x: 40px;
            y: 28px;
            text: "claudie Settings";
            font-size: 24px;
            font-weight: 600;
            color: #111827;
        }

        Text {
            x: 40px;
            y: 64px;
            width: 760px;
            text: "Keep the pet light, tune the local runtime, and manage Claude Code profiles.";
            font-size: 13px;
            color: #6b7280;
        }

        // Tab bar: 40px (8*5) buttons, 8px gutters, active tab filled.
        ActionButton { x: 40px; y: 104px; width: 104px; height: 40px; text: "Basic"; active: root.active_tab == 0; clicked => { root.active_tab = 0; } }
        ActionButton { x: 152px; y: 104px; width: 120px; height: 40px; text: "Pomodoro"; active: root.active_tab == 1; clicked => { root.active_tab = 1; } }
        ActionButton { x: 280px; y: 104px; width: 140px; height: 40px; text: "LLM Profiles"; active: root.active_tab == 2; clicked => { root.active_tab = 2; } }
        ActionButton { x: 428px; y: 104px; width: 96px; height: 40px; text: "Stats"; active: root.active_tab == 3; clicked => { root.active_tab = 3; } }

        // Content region: x 40, width 800 (8*100), 4-column field grid
        // (col 188 + 16 gutter), row pitch 64, fields 32 high.
        if active_tab == 0: Rectangle {
            x: 40px;
            y: 160px;
            width: 800px;
            height: 556px;
            background: transparent;

            Text { x: 0px; y: 0px; text: "Pet renderer"; font-size: 16px; font-weight: 600; color: #111827; }
            Text { x: 0px; y: 26px; width: 760px; text: "Tune the desktop pet size and map each mood to a GIF filename."; font-size: 13px; color: #6b7280; }

            Text { x: 0px; y: 64px; text: "Pet size"; color: #6b7280; font-size: 12px; }
            PointerSlider {
                x: 0px; y: 84px; width: 300px; height: 32px;
                minimum: 50; maximum: 150; step: 1;
                value <=> root.pet_scale;
                changed(value) => { root.pet_scale_changed(value); }
            }
            Text { x: 312px; y: 90px; width: 80px; text: Math.round(root.pet_scale) + "%"; color: #111827; font-size: 13px; }
            Text { x: 408px; y: 64px; text: "Sleep after"; color: #6b7280; font-size: 12px; }
            PointerSlider {
                x: 408px; y: 84px; width: 300px; height: 32px;
                minimum: 15; maximum: 1800; step: 15;
                value <=> root.sleep_after;
                changed(value) => { root.sleep_after_changed(value); }
            }
            Text { x: 720px; y: 90px; width: 80px; text: Math.round(root.sleep_after) + "s"; color: #111827; font-size: 13px; }

            Text { x: 0px; y: 128px; text: "Pet asset directory"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 0px; y: 148px; width: 392px; height: 32px; text <=> root.pet_dir; }
            Text { x: 408px; y: 128px; text: "GIF directory"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 408px; y: 148px; width: 392px; height: 32px; text <=> root.gif_dir; }

            Text { x: 0px; y: 192px; text: "Idle"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 0px; y: 212px; width: 188px; height: 32px; text <=> root.anim_idle; }
            Text { x: 204px; y: 192px; text: "Thinking"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 204px; y: 212px; width: 188px; height: 32px; text <=> root.anim_thinking; }
            Text { x: 408px; y: 192px; text: "Typing"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 408px; y: 212px; width: 188px; height: 32px; text <=> root.anim_typing; }
            Text { x: 612px; y: 192px; text: "Building"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 612px; y: 212px; width: 188px; height: 32px; text <=> root.anim_building; }

            Text { x: 0px; y: 256px; text: "Search"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 0px; y: 276px; width: 188px; height: 32px; text <=> root.anim_search; }
            Text { x: 204px; y: 256px; text: "Happy"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 204px; y: 276px; width: 188px; height: 32px; text <=> root.anim_happy; }
            Text { x: 408px; y: 256px; text: "Error"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 408px; y: 276px; width: 188px; height: 32px; text <=> root.anim_error; }
            Text { x: 612px; y: 256px; text: "Sleeping"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 612px; y: 276px; width: 188px; height: 32px; text <=> root.anim_sleeping; }

            Text { x: 0px; y: 320px; text: "Subagent"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 0px; y: 340px; width: 188px; height: 32px; text <=> root.anim_subagent; }
            Text { x: 204px; y: 320px; text: "Pomodoro"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 204px; y: 340px; width: 188px; height: 32px; text <=> root.anim_pomodoro; }
            Text { x: 408px; y: 320px; text: "Wave"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 408px; y: 340px; width: 188px; height: 32px; text <=> root.anim_wave; }
            Text { x: 612px; y: 320px; text: "Stretch"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 612px; y: 340px; width: 188px; height: 32px; text <=> root.anim_stretch; }

            Text { x: 0px; y: 384px; text: "Fishing"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 0px; y: 404px; width: 188px; height: 32px; text <=> root.anim_fishing; }
            Text { x: 204px; y: 384px; text: "Reel"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 204px; y: 404px; width: 188px; height: 32px; text <=> root.anim_fishing_reel; }
            Text { x: 408px; y: 384px; text: "Caught"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 408px; y: 404px; width: 188px; height: 32px; text <=> root.anim_fishing_caught; }
            Text { x: 612px; y: 384px; text: "Missed"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 612px; y: 404px; width: 188px; height: 32px; text <=> root.anim_fishing_missed; }

            Text { x: 0px; y: 456px; text: "Session switcher"; color: #6b7280; font-size: 12px; }
            Text { x: 0px; y: 478px; width: 420px; text: "Show the compact focus panel when more than one Claude Code session is active."; color: #111827; font-size: 13px; }
            TogglePill { x: 440px; y: 464px; width: 46px; height: 24px; checked <=> root.show_session_switcher; }

            ActionButton { x: 592px; y: 504px; width: 96px; height: 40px; text: "Save"; clicked => { root.save_basic(); } }
            ActionButton { x: 704px; y: 504px; width: 96px; height: 40px; text: "Reset"; clicked => { root.reset_basic(); } }
        }

        if active_tab == 1: Rectangle {
            x: 40px;
            y: 160px;
            width: 800px;
            height: 556px;
            background: transparent;

            Text { x: 0px; y: 0px; text: "Pomodoro"; font-size: 16px; font-weight: 600; color: #111827; }
            Text { x: 0px; y: 26px; width: 760px; text: "Set focus and break lengths, then control the active timer."; font-size: 13px; color: #6b7280; }

            Text { x: 0px; y: 72px; text: "Focus min"; color: #6b7280; font-size: 12px; }
            PointerSpinBox {
                x: 0px; y: 92px; width: 120px; height: 32px;
                minimum: 1; maximum: 240; value <=> root.focus_minutes;
            }
            Text { x: 136px; y: 72px; text: "Short break"; color: #6b7280; font-size: 12px; }
            PointerSpinBox {
                x: 136px; y: 92px; width: 120px; height: 32px;
                minimum: 1; maximum: 240; value <=> root.short_break_minutes;
            }
            Text { x: 272px; y: 72px; text: "Long break"; color: #6b7280; font-size: 12px; }
            PointerSpinBox {
                x: 272px; y: 92px; width: 120px; height: 32px;
                minimum: 1; maximum: 240; value <=> root.long_break_minutes;
            }
            ActionButton { x: 408px; y: 92px; width: 112px; height: 32px; text: "Save"; clicked => { root.save_pomodoro(); } }

            Rectangle { x: 0px; y: 176px; width: 520px; height: 96px; background: #f2f5fa; border-radius: 12px; border-width: 1px; border-color: #dae0ea; }
            Text { x: 24px; y: 200px; width: 472px; text: root.pomodoro_status; color: #111827; font-size: 14px; }

            ActionButton { x: 0px; y: 312px; width: 112px; height: 40px; text: "Start"; clicked => { root.start_pomodoro(); } }
            ActionButton { x: 128px; y: 312px; width: 112px; height: 40px; text: root.pause_resume_label; clicked => { root.pause_resume_pomodoro(); } }
            ActionButton { x: 256px; y: 312px; width: 112px; height: 40px; text: "Skip"; clicked => { root.skip_pomodoro(); } }
            ActionButton { x: 384px; y: 312px; width: 112px; height: 40px; text: "Stop"; clicked => { root.stop_pomodoro(); } }
        }

        if active_tab == 2: Rectangle {
            x: 40px;
            y: 160px;
            width: 800px;
            height: 556px;
            background: transparent;

            Text { x: 0px; y: 0px; text: "Provider profiles"; font-size: 16px; font-weight: 600; color: #111827; }
            Text { x: 0px; y: 26px; width: 760px; text: "Keep Claude Code provider settings tidy without leaving the pet."; font-size: 13px; color: #6b7280; }

            Text { x: 0px; y: 72px; text: "Profile"; color: #6b7280; font-size: 12px; }
            PointerComboBox {
                x: 0px; y: 92px; width: 340px; height: 32px;
                model: root.profile_model;
                current-index <=> root.selected_profile_index;
                selected(index) => { root.select_profile(index); }
            }
            Text { x: 352px; y: 98px; width: 110px; text: root.profile_position; color: #6b7280; font-size: 12px; }
            ActionButton { x: 472px; y: 92px; width: 72px; height: 32px; text: "New"; clicked => { root.new_profile(); } }
            ActionButton { x: 552px; y: 92px; width: 130px; height: 32px; text: "Import Current"; clicked => { root.import_profile(); } }
            ActionButton { x: 690px; y: 92px; width: 110px; height: 32px; text: "Delete"; clicked => { root.delete_profile(); } }

            Text { x: 0px; y: 136px; text: "Profile ID"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 0px; y: 156px; width: 392px; height: 32px; text <=> root.profile_id; }
            Text { x: 408px; y: 136px; text: "Name"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 408px; y: 156px; width: 392px; height: 32px; text <=> root.profile_name; }

            Text { x: 0px; y: 200px; text: "Model"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 0px; y: 220px; width: 392px; height: 32px; text <=> root.model; }
            Text { x: 408px; y: 200px; text: "Base URL"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 408px; y: 220px; width: 392px; height: 32px; text <=> root.base_url; }

            Text { x: 0px; y: 264px; text: "API key"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 0px; y: 284px; width: 392px; height: 32px; input-type: InputType.password; text <=> root.api_key; }
            Text { x: 408px; y: 264px; text: "Auth token (proxy)"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 408px; y: 284px; width: 392px; height: 32px; input-type: InputType.password; text <=> root.auth_token; }

            Text { x: 0px; y: 328px; text: "Opus"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 0px; y: 348px; width: 256px; height: 32px; text <=> root.opus_model; }
            Text { x: 272px; y: 328px; text: "Sonnet"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 272px; y: 348px; width: 256px; height: 32px; text <=> root.sonnet_model; }
            Text { x: 544px; y: 328px; text: "Haiku"; color: #6b7280; font-size: 12px; }
            MonoLineEdit { x: 544px; y: 348px; width: 256px; height: 32px; text <=> root.haiku_model; }

            Text { x: 0px; y: 392px; text: "Extra env"; color: #6b7280; font-size: 12px; }
            MonoTextEdit { x: 0px; y: 412px; width: 392px; height: 72px; text <=> root.extra_env; }
            Text { x: 408px; y: 392px; text: "OpenAI body"; color: #6b7280; font-size: 12px; }
            MonoTextEdit { x: 408px; y: 412px; width: 392px; height: 72px; text <=> root.openai_extra_body; }

            Rectangle { x: 0px; y: 496px; width: 560px; height: 48px; background: #f2f5fa; border-radius: 10px; border-width: 1px; border-color: #dae0ea; }
            Text { x: 14px; y: 506px; width: 340px; text: root.profile_usage_title; overflow: elide; color: #111827; font-size: 13px; font-weight: 600; }
            Text { x: 14px; y: 526px; width: 340px; text: root.profile_usage_summary; overflow: elide; color: #475569; font-size: 11px; }
            StatBarRow { x: 368px; y: 500px; width: 174px; height: 18px; label: "5h"; value: root.profile_usage_five_hour_value; bar: root.profile_usage_five_hour_bar; accent: root.profile_usage_five_hour_bar >= 90 ? #d64545 : (root.profile_usage_five_hour_bar >= 70 ? #d88a24 : #0a84ff); }
            StatBarRow { x: 368px; y: 522px; width: 174px; height: 18px; label: "7d"; value: root.profile_usage_seven_day_value; bar: root.profile_usage_seven_day_bar; accent: root.profile_usage_seven_day_bar >= 90 ? #d64545 : (root.profile_usage_seven_day_bar >= 70 ? #d88a24 : #7c5cc4); }

            ActionButton { x: 592px; y: 504px; width: 96px; height: 40px; text: "Save"; clicked => { root.save_profile(); } }
            ActionButton { x: 704px; y: 504px; width: 96px; height: 40px; text: "Use"; clicked => { root.use_profile(); } }
        }

        if active_tab == 3: Rectangle {
            x: 40px;
            y: 160px;
            width: 800px;
            height: 556px;
            background: transparent;

            Text { x: 0px; y: 0px; text: "Session ledger"; font-size: 16px; font-weight: 600; color: #111827; }
            Text { x: 0px; y: 26px; width: 760px; text: "A quiet local record of Claude Code activity observed by claudie."; font-size: 13px; color: #6b7280; }

            Rectangle { x: 0px; y: 72px; width: 392px; height: 240px; background: #f2f5fa; border-radius: 12px; border-width: 1px; border-color: #dae0ea; }
            Text { x: 24px; y: 92px; width: 344px; text: root.stats_today_title; color: #111827; font-size: 14px; font-weight: 600; }
            Text { x: 24px; y: 116px; width: 344px; height: 28px; text: root.stats_today_summary; overflow: elide; color: #111827; font-size: 12px; }
            StatBarRow { x: 24px; y: 152px; width: 344px; height: 20px; label: "Write"; value: root.stats_today_write_value; bar: root.stats_today_write_bar; accent: #2a9d8f; }
            StatBarRow { x: 24px; y: 176px; width: 344px; height: 20px; label: "Bash"; value: root.stats_today_bash_value; bar: root.stats_today_bash_bar; accent: #4577c3; }
            StatBarRow { x: 24px; y: 200px; width: 344px; height: 20px; label: "Search"; value: root.stats_today_search_value; bar: root.stats_today_search_bar; accent: #d88a24; }
            StatBarRow { x: 24px; y: 224px; width: 344px; height: 20px; label: "Agent"; value: root.stats_today_subagent_value; bar: root.stats_today_subagent_bar; accent: #7c5cc4; }
            StatBarRow { x: 24px; y: 248px; width: 344px; height: 20px; label: "Perm"; value: root.stats_today_permission_value; bar: root.stats_today_permission_bar; accent: #0a84ff; }
            StatBarRow { x: 24px; y: 272px; width: 344px; height: 20px; label: "Choice"; value: root.stats_today_choice_value; bar: root.stats_today_choice_bar; accent: #6b8f3f; }

            Rectangle { x: 408px; y: 72px; width: 392px; height: 240px; background: #f2f5fa; border-radius: 12px; border-width: 1px; border-color: #dae0ea; }
            Text { x: 432px; y: 92px; width: 344px; text: root.stats_recent_title; color: #111827; font-size: 14px; font-weight: 600; }
            Text { x: 432px; y: 116px; width: 344px; height: 28px; text: root.stats_recent_summary; overflow: elide; color: #111827; font-size: 12px; }
            StatBarRow { x: 432px; y: 152px; width: 344px; height: 20px; label: "Write"; value: root.stats_recent_write_value; bar: root.stats_recent_write_bar; accent: #2a9d8f; }
            StatBarRow { x: 432px; y: 176px; width: 344px; height: 20px; label: "Bash"; value: root.stats_recent_bash_value; bar: root.stats_recent_bash_bar; accent: #4577c3; }
            StatBarRow { x: 432px; y: 200px; width: 344px; height: 20px; label: "Search"; value: root.stats_recent_search_value; bar: root.stats_recent_search_bar; accent: #d88a24; }
            StatBarRow { x: 432px; y: 224px; width: 344px; height: 20px; label: "Agent"; value: root.stats_recent_subagent_value; bar: root.stats_recent_subagent_bar; accent: #7c5cc4; }
            StatBarRow { x: 432px; y: 248px; width: 344px; height: 20px; label: "Perm"; value: root.stats_recent_permission_value; bar: root.stats_recent_permission_bar; accent: #0a84ff; }
            StatBarRow { x: 432px; y: 272px; width: 344px; height: 20px; label: "Choice"; value: root.stats_recent_choice_value; bar: root.stats_recent_choice_bar; accent: #6b8f3f; }

            Rectangle { x: 0px; y: 328px; width: 392px; height: 160px; background: #ffffff; border-radius: 12px; border-width: 1px; border-color: #dae0ea; }
            Text { x: 24px; y: 348px; width: 344px; text: "Tokens today"; color: #111827; font-size: 14px; font-weight: 600; }
            StatBarRow { x: 24px; y: 378px; width: 344px; height: 18px; label: "Input"; value: root.stats_today_input_value; bar: root.stats_today_input_bar; accent: #2a9d8f; }
            StatBarRow { x: 24px; y: 400px; width: 344px; height: 18px; label: "Output"; value: root.stats_today_output_value; bar: root.stats_today_output_bar; accent: #4577c3; }
            StatBarRow { x: 24px; y: 422px; width: 344px; height: 18px; label: "Cache W"; value: root.stats_today_cache_write_value; bar: root.stats_today_cache_write_bar; accent: #d88a24; }
            StatBarRow { x: 24px; y: 444px; width: 344px; height: 18px; label: "Cache R"; value: root.stats_today_cache_read_value; bar: root.stats_today_cache_read_bar; accent: #7c5cc4; }

            Rectangle { x: 408px; y: 328px; width: 392px; height: 160px; background: #ffffff; border-radius: 12px; border-width: 1px; border-color: #dae0ea; }
            Text { x: 432px; y: 348px; width: 344px; text: "Tokens last 7 days"; color: #111827; font-size: 14px; font-weight: 600; }
            StatBarRow { x: 432px; y: 378px; width: 344px; height: 18px; label: "Input"; value: root.stats_recent_input_value; bar: root.stats_recent_input_bar; accent: #2a9d8f; }
            StatBarRow { x: 432px; y: 400px; width: 344px; height: 18px; label: "Output"; value: root.stats_recent_output_value; bar: root.stats_recent_output_bar; accent: #4577c3; }
            StatBarRow { x: 432px; y: 422px; width: 344px; height: 18px; label: "Cache W"; value: root.stats_recent_cache_write_value; bar: root.stats_recent_cache_write_bar; accent: #d88a24; }
            StatBarRow { x: 432px; y: 444px; width: 344px; height: 18px; label: "Cache R"; value: root.stats_recent_cache_read_value; bar: root.stats_recent_cache_read_bar; accent: #7c5cc4; }
        }

        Text {
            x: 40px;
            y: 724px;
            width: 800px;
            height: 20px;
            text: root.status_message;
            color: #6b7280;
            font-size: 12px;
        }
        }

        Rectangle { x: 0px; y: 0px; width: root.width; height: root.height; background: #f4f7fc; }
        Rectangle {
            x: 8px;
            y: 8px;
            width: root.width - 16px;
            height: root.height - 16px;
            background: #ffffff;
            border-radius: 8px;
            border-width: 1px;
            border-color: #e4e8f0;
        }
        TouchArea { x: 0px; y: 0px; width: root.width; height: root.height; }

        Rectangle {
            x: 16px;
            y: 16px;
            width: 144px;
            height: root.height - 32px;
            background: #f8fafc;
            border-radius: 8px;
            border-width: 1px;
            border-color: #e7edf5;
        }

        Text {
            x: 28px;
            y: 28px;
            width: 120px;
            text: "claudie";
            font-size: 20px;
            font-weight: 700;
            color: #111827;
        }
        Text {
            x: 28px;
            y: 54px;
            width: 120px;
            text: "Settings";
            font-size: 12px;
            font-weight: 600;
            color: #64748b;
        }

        SettingsTabButton { x: 24px; y: 84px; width: 128px; height: 36px; text: "Basic"; icon_source: @image-url("../../assets/lucide/sliders-horizontal.svg"); active: root.active_tab == 0; clicked => { root.active_tab = 0; } }
        SettingsTabButton { x: 24px; y: 128px; width: 128px; height: 36px; text: "Pomodoro"; icon_source: @image-url("../../assets/lucide/timer.svg"); active: root.active_tab == 1; clicked => { root.active_tab = 1; } }
        SettingsTabButton { x: 24px; y: 172px; width: 128px; height: 36px; text: "LLM Profiles"; icon_source: @image-url("../../assets/lucide/bot.svg"); active: root.active_tab == 2; clicked => { root.active_tab = 2; } }
        SettingsTabButton { x: 24px; y: 216px; width: 128px; height: 36px; text: "Stats"; icon_source: @image-url("../../assets/lucide/chart-no-axes-column.svg"); active: root.active_tab == 3; clicked => { root.active_tab = 3; } }

        Rectangle { x: 168px; y: 16px; width: 1px; height: root.height - 32px; background: #e7edf5; }

        ScrollView {
            x: 176px;
            y: 16px;
            width: root.width - 192px;
            height: root.height - 56px;
            viewport-width: root.content_width;
            viewport-height: root.active_tab == 0 ? 608px : (root.active_tab == 1 ? 456px : (root.active_tab == 2 ? 568px : 536px));

            if active_tab == 0: Rectangle {
                width: root.content_width;
                height: 608px;
                background: transparent;

                Text { x: 0px; y: 0px; text: "Pet renderer"; font-size: 17px; font-weight: 700; color: #111827; }
                Text { x: 0px; y: 28px; width: 576px; text: "Tune the desktop pet size and map each mood to a GIF filename."; font-size: 13px; color: #6b7280; }

                Text { x: 0px; y: 64px; text: "Pet size"; color: #6b7280; font-size: 12px; }
                PointerSlider {
                    x: 0px; y: 84px; width: 220px; height: 32px;
                    minimum: 50; maximum: 150; step: 1;
                    value <=> root.pet_scale;
                    changed(value) => { root.pet_scale_changed(value); }
                }
                Text { x: 232px; y: 90px; width: 52px; text: Math.round(root.pet_scale) + "%"; color: #111827; font-size: 13px; }
                Text { x: 300px; y: 64px; text: "Sleep after"; color: #6b7280; font-size: 12px; }
                PointerSlider {
                    x: 300px; y: 84px; width: 220px; height: 32px;
                    minimum: 15; maximum: 1800; step: 15;
                    value <=> root.sleep_after;
                    changed(value) => { root.sleep_after_changed(value); }
                }
                Text { x: 532px; y: 90px; width: 52px; text: Math.round(root.sleep_after) + "s"; color: #111827; font-size: 13px; }

                Text { x: 0px; y: 128px; text: "Pet asset directory"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 0px; y: 148px; width: 284px; height: 32px; text <=> root.pet_dir; }
                Text { x: 300px; y: 128px; text: "GIF directory"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 300px; y: 148px; width: 284px; height: 32px; text <=> root.gif_dir; }

                Text { x: 0px; y: 204px; text: "Idle"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 0px; y: 224px; width: 134px; height: 32px; text <=> root.anim_idle; }
                Text { x: 150px; y: 204px; text: "Thinking"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 150px; y: 224px; width: 134px; height: 32px; text <=> root.anim_thinking; }
                Text { x: 300px; y: 204px; text: "Typing"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 300px; y: 224px; width: 134px; height: 32px; text <=> root.anim_typing; }
                Text { x: 450px; y: 204px; text: "Building"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 450px; y: 224px; width: 134px; height: 32px; text <=> root.anim_building; }

                Text { x: 0px; y: 268px; text: "Search"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 0px; y: 288px; width: 134px; height: 32px; text <=> root.anim_search; }
                Text { x: 150px; y: 268px; text: "Happy"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 150px; y: 288px; width: 134px; height: 32px; text <=> root.anim_happy; }
                Text { x: 300px; y: 268px; text: "Error"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 300px; y: 288px; width: 134px; height: 32px; text <=> root.anim_error; }
                Text { x: 450px; y: 268px; text: "Sleeping"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 450px; y: 288px; width: 134px; height: 32px; text <=> root.anim_sleeping; }

                Text { x: 0px; y: 332px; text: "Subagent"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 0px; y: 352px; width: 134px; height: 32px; text <=> root.anim_subagent; }
                Text { x: 150px; y: 332px; text: "Pomodoro"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 150px; y: 352px; width: 134px; height: 32px; text <=> root.anim_pomodoro; }
                Text { x: 300px; y: 332px; text: "Wave"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 300px; y: 352px; width: 134px; height: 32px; text <=> root.anim_wave; }
                Text { x: 450px; y: 332px; text: "Stretch"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 450px; y: 352px; width: 134px; height: 32px; text <=> root.anim_stretch; }

                Text { x: 0px; y: 396px; text: "Fishing"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 0px; y: 416px; width: 134px; height: 32px; text <=> root.anim_fishing; }
                Text { x: 150px; y: 396px; text: "Reel"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 150px; y: 416px; width: 134px; height: 32px; text <=> root.anim_fishing_reel; }
                Text { x: 300px; y: 396px; text: "Caught"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 300px; y: 416px; width: 134px; height: 32px; text <=> root.anim_fishing_caught; }
                Text { x: 450px; y: 396px; text: "Missed"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 450px; y: 416px; width: 134px; height: 32px; text <=> root.anim_fishing_missed; }

                Text { x: 0px; y: 472px; text: "Session switcher"; color: #6b7280; font-size: 12px; }
                Text { x: 0px; y: 494px; width: 480px; text: "Show the compact focus panel when more than one Claude Code session is active."; color: #111827; font-size: 13px; }
                TogglePill { x: 538px; y: 482px; width: 46px; height: 24px; checked <=> root.show_session_switcher; }

                ActionButton { x: 408px; y: 556px; width: 80px; height: 32px; text: "Save"; clicked => { root.save_basic(); } }
                ActionButton { x: 504px; y: 556px; width: 80px; height: 32px; text: "Reset"; clicked => { root.reset_basic(); } }
            }

            if active_tab == 1: Rectangle {
                width: root.content_width;
                height: 456px;
                background: transparent;

                Text { x: 0px; y: 0px; text: "Pomodoro"; font-size: 17px; font-weight: 700; color: #111827; }
                Text { x: 0px; y: 28px; width: 576px; text: "Set focus and break lengths, then control the active timer."; font-size: 13px; color: #6b7280; }

                Rectangle {
                    x: 0px;
                    y: 64px;
                    width: 584px;
                    height: 128px;
                    background: #f2f7ff;
                    border-radius: 8px;
                    border-width: 1px;
                    border-color: #d8e7ff;
                }
                Rectangle { x: 0px; y: 64px; width: 5px; height: 128px; background: #0a84ff; border-radius: 8px; }
                Rectangle { x: 24px; y: 88px; width: 52px; height: 52px; background: #ffffff; border-radius: 8px; border-width: 1px; border-color: #d8e7ff; }
                Image { x: 38px; y: 102px; width: 24px; height: 24px; source: @image-url("../../assets/lucide/timer.svg"); image-fit: contain; colorize: #0a84ff; }
                Text { x: 96px; y: 86px; width: 160px; text: "Current cycle"; color: #64748b; font-size: 12px; font-weight: 700; }
                Text { x: 96px; y: 108px; width: 456px; height: 48px; text: root.pomodoro_status; wrap: word-wrap; color: #111827; font-size: 15px; font-weight: 700; }
                Text { x: 96px; y: 162px; width: 456px; text: "Tune the rhythm below, then use the controls without leaving this panel."; color: #64748b; font-size: 12px; overflow: elide; }

                Text { x: 0px; y: 216px; text: "Durations"; color: #111827; font-size: 14px; font-weight: 700; }
                ActionButton { x: 504px; y: 206px; width: 80px; height: 32px; text: "Save"; clicked => { root.save_pomodoro(); } }

                PomodoroDurationTile {
                    x: 0px; y: 248px; width: 184px; height: 116px;
                    title: "Focus";
                    hint: "Deep work";
                    accent: #0a84ff;
                    value <=> root.focus_minutes;
                }
                PomodoroDurationTile {
                    x: 200px; y: 248px; width: 184px; height: 116px;
                    title: "Short break";
                    hint: "Quick reset";
                    accent: #2a9d8f;
                    value <=> root.short_break_minutes;
                }
                PomodoroDurationTile {
                    x: 400px; y: 248px; width: 184px; height: 116px;
                    title: "Long break";
                    hint: "Full recharge";
                    accent: #7c5cc4;
                    value <=> root.long_break_minutes;
                }

                Rectangle { x: 0px; y: 392px; width: 584px; height: 48px; background: #f8fafc; border-radius: 8px; border-width: 1px; border-color: #e7edf5; }
                ActionButton { x: 16px; y: 400px; width: 112px; height: 32px; text: "Start"; active: true; clicked => { root.start_pomodoro(); } }
                ActionButton { x: 144px; y: 400px; width: 112px; height: 32px; text: root.pause_resume_label; clicked => { root.pause_resume_pomodoro(); } }
                ActionButton { x: 272px; y: 400px; width: 112px; height: 32px; text: "Skip"; clicked => { root.skip_pomodoro(); } }
                ActionButton { x: 456px; y: 400px; width: 112px; height: 32px; text: "Stop"; clicked => { root.stop_pomodoro(); } }
            }

            if active_tab == 2: Rectangle {
                width: root.content_width;
                height: 568px;
                background: transparent;

                Text { x: 0px; y: 0px; text: "Provider profiles"; font-size: 17px; font-weight: 700; color: #111827; }
                Text { x: 0px; y: 28px; width: 576px; text: "Keep Claude Code provider settings tidy without leaving the pet."; font-size: 13px; color: #6b7280; }

                Text { x: 0px; y: 72px; text: "Profile"; color: #6b7280; font-size: 12px; }
                PointerComboBox {
                    x: 0px; y: 92px; width: 244px; height: 32px;
                    model: root.profile_model;
                    current-index <=> root.selected_profile_index;
                    selected(index) => { root.select_profile(index); }
                }
                Text { x: 256px; y: 98px; width: 40px; text: root.profile_position; color: #6b7280; font-size: 12px; }
                ActionButton { x: 304px; y: 92px; width: 60px; height: 32px; text: "New"; clicked => { root.new_profile(); } }
                ActionButton { x: 372px; y: 92px; width: 124px; height: 32px; text: "Import Current"; clicked => { root.import_profile(); } }
                ActionButton { x: 504px; y: 92px; width: 80px; height: 32px; text: "Delete"; clicked => { root.delete_profile(); } }

                Text { x: 0px; y: 136px; text: "Profile ID"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 0px; y: 156px; width: 284px; height: 32px; text <=> root.profile_id; }
                Text { x: 300px; y: 136px; text: "Name"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 300px; y: 156px; width: 284px; height: 32px; text <=> root.profile_name; }

                Text { x: 0px; y: 200px; text: "Model"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 0px; y: 220px; width: 284px; height: 32px; text <=> root.model; }
                Text { x: 300px; y: 200px; text: "Base URL"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 300px; y: 220px; width: 284px; height: 32px; text <=> root.base_url; }

                Text { x: 0px; y: 264px; text: "API key"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 0px; y: 284px; width: 284px; height: 32px; input-type: InputType.password; text <=> root.api_key; }
                Text { x: 300px; y: 264px; text: "Auth token (proxy)"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 300px; y: 284px; width: 284px; height: 32px; input-type: InputType.password; text <=> root.auth_token; }

                Text { x: 0px; y: 328px; text: "Opus"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 0px; y: 348px; width: 184px; height: 32px; text <=> root.opus_model; }
                Text { x: 200px; y: 328px; text: "Sonnet"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 200px; y: 348px; width: 184px; height: 32px; text <=> root.sonnet_model; }
                Text { x: 400px; y: 328px; text: "Haiku"; color: #6b7280; font-size: 12px; }
                MonoLineEdit { x: 400px; y: 348px; width: 184px; height: 32px; text <=> root.haiku_model; }

                Text { x: 0px; y: 392px; text: "Extra env"; color: #6b7280; font-size: 12px; }
                MonoTextEdit { x: 0px; y: 412px; width: 284px; height: 72px; text <=> root.extra_env; }
                Text { x: 300px; y: 392px; text: "OpenAI body"; color: #6b7280; font-size: 12px; }
                MonoTextEdit { x: 300px; y: 412px; width: 284px; height: 72px; text <=> root.openai_extra_body; }

                Rectangle { x: 0px; y: 504px; width: 432px; height: 52px; background: #f2f5fa; border-radius: 8px; border-width: 1px; border-color: #dae0ea; }
                Text { x: 14px; y: 514px; width: 216px; text: root.profile_usage_title; overflow: elide; color: #111827; font-size: 13px; font-weight: 600; }
                Text { x: 14px; y: 534px; width: 216px; text: root.profile_usage_summary; overflow: elide; color: #475569; font-size: 11px; }
                StatBarRow { x: 232px; y: 508px; width: 184px; height: 18px; label: "5h"; value: root.profile_usage_five_hour_value; bar: root.profile_usage_five_hour_bar; accent: root.profile_usage_five_hour_bar >= 90 ? #d64545 : (root.profile_usage_five_hour_bar >= 70 ? #d88a24 : #0a84ff); }
                StatBarRow { x: 232px; y: 530px; width: 184px; height: 18px; label: "7d"; value: root.profile_usage_seven_day_value; bar: root.profile_usage_seven_day_bar; accent: root.profile_usage_seven_day_bar >= 90 ? #d64545 : (root.profile_usage_seven_day_bar >= 70 ? #d88a24 : #7c5cc4); }

                ActionButton { x: 448px; y: 514px; width: 64px; height: 32px; text: "Save"; clicked => { root.save_profile(); } }
                ActionButton { x: 520px; y: 514px; width: 64px; height: 32px; text: "Use"; clicked => { root.use_profile(); } }
            }

            if active_tab == 3: Rectangle {
                width: root.content_width;
                height: 536px;
                background: transparent;

                Text { x: 0px; y: 0px; text: "Session ledger"; font-size: 17px; font-weight: 700; color: #111827; }
                Text { x: 0px; y: 28px; width: 576px; text: "A quiet local record of Claude Code activity observed by claudie."; font-size: 13px; color: #6b7280; }

                Rectangle { x: 0px; y: 72px; width: 284px; height: 240px; background: #f2f5fa; border-radius: 8px; border-width: 1px; border-color: #dae0ea; }
                Text { x: 24px; y: 92px; width: 236px; text: root.stats_today_title; color: #111827; font-size: 14px; font-weight: 600; }
                Text { x: 24px; y: 116px; width: 236px; height: 28px; text: root.stats_today_summary; overflow: elide; color: #111827; font-size: 12px; }
                StatBarRow { x: 24px; y: 152px; width: 236px; height: 20px; label: "Write"; value: root.stats_today_write_value; bar: root.stats_today_write_bar; accent: #2a9d8f; }
                StatBarRow { x: 24px; y: 176px; width: 236px; height: 20px; label: "Bash"; value: root.stats_today_bash_value; bar: root.stats_today_bash_bar; accent: #4577c3; }
                StatBarRow { x: 24px; y: 200px; width: 236px; height: 20px; label: "Search"; value: root.stats_today_search_value; bar: root.stats_today_search_bar; accent: #d88a24; }
                StatBarRow { x: 24px; y: 224px; width: 236px; height: 20px; label: "Agent"; value: root.stats_today_subagent_value; bar: root.stats_today_subagent_bar; accent: #7c5cc4; }
                StatBarRow { x: 24px; y: 248px; width: 236px; height: 20px; label: "Perm"; value: root.stats_today_permission_value; bar: root.stats_today_permission_bar; accent: #0a84ff; }
                StatBarRow { x: 24px; y: 272px; width: 236px; height: 20px; label: "Choice"; value: root.stats_today_choice_value; bar: root.stats_today_choice_bar; accent: #6b8f3f; }

                Rectangle { x: 300px; y: 72px; width: 284px; height: 240px; background: #f2f5fa; border-radius: 8px; border-width: 1px; border-color: #dae0ea; }
                Text { x: 324px; y: 92px; width: 236px; text: root.stats_recent_title; color: #111827; font-size: 14px; font-weight: 600; }
                Text { x: 324px; y: 116px; width: 236px; height: 28px; text: root.stats_recent_summary; overflow: elide; color: #111827; font-size: 12px; }
                StatBarRow { x: 324px; y: 152px; width: 236px; height: 20px; label: "Write"; value: root.stats_recent_write_value; bar: root.stats_recent_write_bar; accent: #2a9d8f; }
                StatBarRow { x: 324px; y: 176px; width: 236px; height: 20px; label: "Bash"; value: root.stats_recent_bash_value; bar: root.stats_recent_bash_bar; accent: #4577c3; }
                StatBarRow { x: 324px; y: 200px; width: 236px; height: 20px; label: "Search"; value: root.stats_recent_search_value; bar: root.stats_recent_search_bar; accent: #d88a24; }
                StatBarRow { x: 324px; y: 224px; width: 236px; height: 20px; label: "Agent"; value: root.stats_recent_subagent_value; bar: root.stats_recent_subagent_bar; accent: #7c5cc4; }
                StatBarRow { x: 324px; y: 248px; width: 236px; height: 20px; label: "Perm"; value: root.stats_recent_permission_value; bar: root.stats_recent_permission_bar; accent: #0a84ff; }
                StatBarRow { x: 324px; y: 272px; width: 236px; height: 20px; label: "Choice"; value: root.stats_recent_choice_value; bar: root.stats_recent_choice_bar; accent: #6b8f3f; }

                Rectangle { x: 0px; y: 336px; width: 284px; height: 168px; background: #ffffff; border-radius: 8px; border-width: 1px; border-color: #dae0ea; }
                Text { x: 24px; y: 356px; width: 236px; text: "Tokens today"; color: #111827; font-size: 14px; font-weight: 600; }
                StatBarRow { x: 24px; y: 386px; width: 236px; height: 18px; label: "Input"; value: root.stats_today_input_value; bar: root.stats_today_input_bar; accent: #2a9d8f; }
                StatBarRow { x: 24px; y: 408px; width: 236px; height: 18px; label: "Output"; value: root.stats_today_output_value; bar: root.stats_today_output_bar; accent: #4577c3; }
                StatBarRow { x: 24px; y: 430px; width: 236px; height: 18px; label: "Cache W"; value: root.stats_today_cache_write_value; bar: root.stats_today_cache_write_bar; accent: #d88a24; }
                StatBarRow { x: 24px; y: 452px; width: 236px; height: 18px; label: "Cache R"; value: root.stats_today_cache_read_value; bar: root.stats_today_cache_read_bar; accent: #7c5cc4; }

                Rectangle { x: 300px; y: 336px; width: 284px; height: 168px; background: #ffffff; border-radius: 8px; border-width: 1px; border-color: #dae0ea; }
                Text { x: 324px; y: 356px; width: 236px; text: "Tokens last 7 days"; color: #111827; font-size: 14px; font-weight: 600; }
                StatBarRow { x: 324px; y: 386px; width: 236px; height: 18px; label: "Input"; value: root.stats_recent_input_value; bar: root.stats_recent_input_bar; accent: #2a9d8f; }
                StatBarRow { x: 324px; y: 408px; width: 236px; height: 18px; label: "Output"; value: root.stats_recent_output_value; bar: root.stats_recent_output_bar; accent: #4577c3; }
                StatBarRow { x: 324px; y: 430px; width: 236px; height: 18px; label: "Cache W"; value: root.stats_recent_cache_write_value; bar: root.stats_recent_cache_write_bar; accent: #d88a24; }
                StatBarRow { x: 324px; y: 452px; width: 236px; height: 18px; label: "Cache R"; value: root.stats_recent_cache_read_value; bar: root.stats_recent_cache_read_bar; accent: #7c5cc4; }
            }
        }

        Text {
            x: 176px;
            y: root.height - 32px;
            width: root.width - 192px;
            height: 20px;
            text: root.status_message;
            overflow: elide;
            color: #6b7280;
            font-size: 12px;
        }
    }

    struct ChoiceOptionData {
        question_index: int,
        option_index: int,
        label: string,
        description: string,
        selected: bool,
        is_other: bool,
        other_text: string,
        multi_select: bool,
        is_question_header: bool,
        // Wrapped line counts estimated in Rust; Slint Text does not
        // grow with word-wrap, so row heights derive from these.
        label_lines: int,
        desc_lines: int,
    }

    struct DiffLine {
        text: string,
        // 0 context, 1 added, 2 removed.
        tone: int,
        // Wrapped line count estimated in Rust (Slint Text does not grow).
        lines: int,
    }

    struct MarkdownBlockData {
        // 0 paragraph, 1-3 heading level, 4 bullet, 5 code, 6 quote, 7 diff.
        kind: int,
        text: string,
        indent: int,
        lines: int,
        // Populated only for diff blocks (kind 7).
        diff_lines: [DiffLine],
    }

    component MarkdownBlockRow inherits Rectangle {
        in property <MarkdownBlockData> data;

        property <bool> is_heading: data.kind >= 1 && data.kind <= 3;
        property <bool> is_code: data.kind == 5 || data.kind == 7;
        property <length> line_h: data.kind == 1 ? 25px
            : (data.kind == 2 ? 22px
            : (data.kind == 3 ? 20px
            : (self.is_code ? 17px : 19px)));

        height: data.lines * self.line_h
            + (self.is_code ? 16px : 0px)
            + (self.is_heading ? 6px : 0px);
        background: data.kind == 5 ? #e9eef5 : (data.kind == 7 ? #f6f8fa : transparent);
        border-radius: 8px;
        border-width: data.kind == 7 ? 1px : 0px;
        border-color: #dde3ec;
        clip: data.kind == 7;

        if data.kind == 7: VerticalLayout {
            x: 0px; y: 8px;
            width: parent.width;
            height: parent.height - 16px;
            padding: 0px;
            spacing: 0px;
            for line in data.diff_lines: Rectangle {
                height: line.lines * 17px;
                background: line.tone == 1 ? #e6ffec
                    : (line.tone == 2 ? #ffebe9 : transparent);
                Text {
                    x: 12px; y: 0px;
                    width: parent.width - 20px;
                    height: 100%;
                    text: line.text;
                    wrap: word-wrap;
                    vertical-alignment: top;
                    font-family: "Cascadia Mono, Consolas";
                    font-size: 12px;
                    color: line.tone == 1 ? #1a7f37
                        : (line.tone == 2 ? #cf222e : #57606a);
                }
            }
        }

        if data.kind == 5: Text {
            x: 10px; y: 8px;
            width: parent.width - 20px;
            height: parent.height - 16px;
            text: data.text;
            wrap: word-wrap;
            font-family: "Cascadia Mono, Consolas";
            font-size: 12px;
            color: #111827;
        }

        if data.kind != 5 && data.kind != 7: Text {
            x: data.indent * 14px;
            y: root.is_heading ? 6px : 0px;
            width: parent.width - data.indent * 14px;
            height: parent.height - (root.is_heading ? 6px : 0px);
            text: data.text;
            wrap: word-wrap;
            font-size: data.kind == 1 ? 17px
                : (data.kind == 2 ? 15px
                : (data.kind == 3 ? 14px : 13px));
            font-weight: root.is_heading ? 700 : 400;
            font-italic: data.kind == 6;
            color: data.kind == 6 ? #6b7280 : #111827;
        }
    }

    component ChoiceOptionRow inherits Rectangle {
        in property <ChoiceOptionData> data;
        callback toggle();
        callback other_text_changed(string);

        property <length> label_h: data.label_lines * 19px;
        property <length> desc_h: data.description == "" ? 0px : data.desc_lines * 17px + 4px;
        property <bool> show_other: data.selected && data.is_other;

        width: 100%;
        height: data.is_question_header
            ? data.desc_lines * 17px + 14px
            : 16px + self.label_h + self.desc_h + (self.show_other ? 38px : 0px);
        background: data.is_question_header
            ? transparent
            : (data.selected ? #eef6ff : #ffffff);
        border-radius: 8px;
        border-width: data.is_question_header ? 0px : 1px;
        border-color: data.selected ? #0a84ff : #dae0ea;

        if !data.is_question_header: TouchArea {
            width: 100%;
            height: 100%;
            mouse-cursor: pointer;
            clicked => { root.toggle(); }
        }

        if data.is_question_header: Text {
            x: 4px; y: 8px;
            width: parent.width - 8px;
            height: parent.height - 10px;
            text: data.description;
            font-size: 12px;
            font-weight: 600;
            color: #475569;
            wrap: word-wrap;
        }

        if !data.is_question_header: Text {
            x: 12px; y: 6px;
            width: 24px; height: 22px;
            text: data.multi_select
                ? (data.selected ? "☑" : "☐")
                : (data.selected ? "●" : "○");
            font-size: 18px;
            horizontal-alignment: center;
            vertical-alignment: center;
            color: data.selected ? #0a84ff : #6b7280;
        }

        if !data.is_question_header: Text {
            x: 40px; y: 8px;
            width: parent.width - 52px;
            height: root.label_h;
            text: data.label;
            font-size: 13px;
            font-weight: 600;
            color: #111827;
            wrap: word-wrap;
        }

        if !data.is_question_header && data.description != "": Text {
            x: 40px; y: 8px + root.label_h + 4px;
            width: parent.width - 52px;
            height: data.desc_lines * 17px;
            text: data.description;
            font-size: 12px;
            color: #6b7280;
            wrap: word-wrap;
        }

        if !data.is_question_header && root.show_other: LineEdit {
            x: 40px; y: 8px + root.label_h + root.desc_h + 4px;
            width: parent.width - 52px;
            height: 30px;
            text: data.other_text;
            placeholder-text: "Type your answer...";
            edited(text) => { root.other_text_changed(text); }
        }
    }

    export component PromptWindow inherits Window {
        // Width is fixed: the Rust side estimates wrapped line counts
        // against this width. Height is user-resizable.
        min-width: 640px;
        max-width: 640px;
        preferred-height: 640px;
        min-height: 496px;
        title: "claudie request";
        icon: @image-url("../../assets/icon.ico");
        background: #f4f7fc;

        in property <bool> is_choice: false;
        in property <string> title_text;
        in property <string> subtitle_text;
        in property <[MarkdownBlockData]> detail_blocks;
        // Plans get most of the space; question lists give it to options.
        in property <bool> detail_dominant: false;
        in property <string> meta_text;
        in property <bool> submit_enabled: false;
        in property <string> submit_hint;
        in property <[ChoiceOptionData]> options_model;

        callback allow_once();
        callback allow_always();
        callback deny();
        callback submit_choice();
        callback cancel_choice();
        callback toggle_option(int);
        callback set_other_text(int, string);

        // Card: 16px window margin; inner content keeps a 16px gutter so the
        // wrapping width (576) stays in sync with prompt_popup.rs constants.
        Rectangle {
            x: 16px;
            y: 16px;
            width: 608px;
            height: root.height - 32px;
            background: white;
            border-radius: 12px;
            border-width: 1px;
            border-color: #e4e8f0;
        }

        VerticalLayout {
            x: 32px;
            y: 32px;
            width: 576px;
            height: root.height - 64px;
            spacing: 16px;

            Text { height: 28px; text: root.title_text; font-size: 20px; font-weight: 600; color: #111827; overflow: elide; }
            Text { height: 18px; text: root.subtitle_text; font-size: 13px; color: #6b7280; overflow: elide; }

            // Detail vs. options split follows the golden ratio (~3:2): plans
            // hand the larger share to the detail panel, question lists invert it.
            Rectangle {
                vertical-stretch: root.detail_dominant ? 3 : 2;
                min-height: 120px;
                background: #f2f5fa;
                border-radius: 12px;
                border-width: 1px;
                border-color: #dae0ea;

                ScrollView {
                    x: 12px; y: 12px;
                    width: parent.width - 24px;
                    height: parent.height - 24px;
                    VerticalLayout {
                        padding: 0px;
                        spacing: 8px;
                        for block in root.detail_blocks: MarkdownBlockRow {
                            data: block;
                        }
                    }
                }
            }

            Text {
                height: 16px;
                text: root.meta_text;
                font-size: 12px;
                color: #9ca3af;
                overflow: elide;
            }

            if is_choice: ScrollView {
                vertical-stretch: root.detail_dominant ? 2 : 3;
                min-height: 120px;
                VerticalLayout {
                    padding: 4px;
                    spacing: 8px;
                    for opt[idx] in root.options_model: ChoiceOptionRow {
                        data: opt;
                        toggle => { root.toggle_option(idx); }
                        other_text_changed(t) => { root.set_other_text(idx, t); }
                    }
                }
            }

            if !is_choice: Text {
                height: 16px;
                text: "Use Ctrl+Shift+Y for Allow and Ctrl+Shift+N for Deny.";
                font-size: 12px;
                color: #6b7280;
            }

            if is_choice && !submit_enabled: Text {
                height: 16px;
                text: root.submit_hint;
                font-size: 12px;
                color: #d97706;
                horizontal-alignment: center;
                overflow: elide;
            }

            HorizontalLayout {
                height: 40px;
                spacing: 12px;
                alignment: end;

                if !is_choice: ActionButton { width: 96px; text: "Allow"; clicked => { root.allow_once(); } }
                if !is_choice: ActionButton { width: 104px; text: "Always"; clicked => { root.allow_always(); } }
                if !is_choice: ActionButton { width: 96px; text: "Deny"; clicked => { root.deny(); } }

                if is_choice: ActionButton { width: 104px; text: "Submit"; enabled: root.submit_enabled; clicked => { root.submit_choice(); } }
                if is_choice: ActionButton { width: 96px; text: "Cancel"; clicked => { root.cancel_choice(); } }
            }
        }
    }
}

slint::slint! {
    import { LineEdit, TextEdit, ScrollView } from "std-widgets.slint";

    // Shared design tokens for the Slint-rendered windows. The GDI HUD keeps
    // its own copies in src/ui/theme.rs; keep palette changes in sync.
    export global Theme {
        // Accent
        out property <color> accent: #0a84ff;
        out property <color> accent-bright: #3094ff;
        out property <color> accent-deep: #0060c6;
        out property <color> accent-soft: #eef6ff;
        out property <color> accent-softer: #dbeafe;
        out property <color> accent-tint: #f2f7ff;
        out property <color> accent-tint-border: #d8e7ff;
        // Ink
        out property <color> ink: #111827;
        out property <color> ink-secondary: #475569;
        out property <color> ink-muted: #64748b;
        out property <color> ink-faint: #6b7280;
        out property <color> ink-disabled: #9ca3af;
        // Surfaces
        out property <color> window-bg: #f4f7fc;
        out property <color> surface: #ffffff;
        out property <color> sunken: #f2f5fa;
        out property <color> sunken-alt: #f8fafc;
        out property <color> track: #dbe4f0;
        // Lines
        out property <color> border: #cfd8e6;
        out property <color> control-border: #9aa7ba;
        out property <color> card-border: #dae0ea;
        out property <color> hairline: #e7edf5;
        out property <color> outline: #e4e8f0;
        out property <color> disabled-bg: #f3f4f6;
        out property <color> disabled-border: #e5e7eb;
        // Status
        out property <color> danger: #d64545;
        out property <color> danger-deep: #b13a3a;
        out property <color> danger-border: #ecc4c4;
        out property <color> danger-soft: #fdf1f1;
        out property <color> danger-softer: #f8dddd;
        out property <color> warning: #d97706;
        // Charts
        out property <color> chart-teal: #2a9d8f;
        out property <color> chart-blue: #4577c3;
        out property <color> chart-amber: #d88a24;
        out property <color> chart-purple: #7c5cc4;
        out property <color> chart-olive: #6b8f3f;
        // Motion
        out property <duration> fast: 120ms;
    }

    component ActionButton inherits Rectangle {
        in property <string> text;
        in property <bool> enabled: true;
        // Filled accent treatment, used for the selected settings tab.
        in property <bool> active: false;
        // "secondary" (default) | "primary" | "danger".
        in property <string> kind: "secondary";
        callback clicked();

        property <bool> filled: root.active || root.kind == "primary";
        property <bool> danger: root.kind == "danger";

        // height/2 instead of an oversized constant: FemtoVG turns radii
        // larger than the rect into an ellipse instead of clamping.
        border-radius: self.height / 2;
        border-width: 1px;
        border-color: !root.enabled ? Theme.disabled-border
            : (root.filled ? Theme.accent
            : (root.danger ? Theme.danger-border : Theme.border));
        background: !root.enabled ? Theme.disabled-bg
            : (root.filled ? Theme.accent : Theme.surface);

        animate background, border-color { duration: Theme.fast; easing: ease-out; }

        states [
            pressed when touch.pressed && root.enabled : {
                background: root.filled ? Theme.accent-deep
                    : (root.danger ? Theme.danger-softer : Theme.accent-softer);
                border-color: root.danger ? Theme.danger-deep : Theme.accent-deep;
            }
            hover when touch.has-hover && root.enabled && root.filled : {
                background: Theme.accent-bright;
            }
            hover-outline when touch.has-hover && root.enabled && !root.filled : {
                background: root.danger ? Theme.danger-soft : Theme.accent-soft;
                border-color: root.danger ? Theme.danger : Theme.accent;
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
            color: !root.enabled ? Theme.ink-disabled
                : (root.filled ? #ffffff
                : (root.danger ? Theme.danger : Theme.ink));
        }
    }

    component GhostButton inherits Rectangle {
        in property <string> text;
        in property <bool> enabled: true;
        callback clicked();

        border-radius: self.height / 2;
        background: touch.pressed && root.enabled ? Theme.accent-softer : (touch.has-hover && root.enabled ? Theme.accent-soft : transparent);
        animate background { duration: Theme.fast; easing: ease-out; }

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
            color: root.enabled ? Theme.ink-secondary : Theme.ink-disabled;
        }
    }

    component OptionRow inherits Rectangle {
        in property <string> text;
        callback clicked();

        border-radius: 8px;
        background: touch.pressed ? Theme.accent-softer : (touch.has-hover ? Theme.accent-soft : transparent);
        animate background { duration: Theme.fast; easing: ease-out; }

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
            color: Theme.ink;
        }
    }

    component MonoLineEdit inherits LineEdit {
        font-family: "Maple Mono CN";
        font-size: 12px;
    }

    component MonoTextEdit inherits TextEdit {
        font-family: "Maple Mono CN";
        font-size: 12px;
    }

    // Caption label stacked over a single-line input. Keeps form rows on the
    // 52px pitch used across the settings tabs.
    component FieldGroup inherits Rectangle {
        in property <string> label;
        in-out property <string> value;
        in property <InputType> input-type: InputType.text;
        callback edited(string);

        height: 52px;
        background: transparent;

        Text {
            x: 0px;
            y: 0px;
            text: root.label;
            color: Theme.ink-faint;
            font-size: 12px;
        }
        MonoLineEdit {
            x: 0px;
            y: 20px;
            width: root.width;
            height: 32px;
            input-type: root.input-type;
            text <=> root.value;
            edited(text) => {
                root.edited(text);
            }
        }
    }

    component TogglePill inherits Rectangle {
        in-out property <bool> checked: false;
        callback toggled(bool);

        border-radius: self.height / 2;
        border-width: 1px;
        border-color: root.checked ? Theme.accent : Theme.border;
        background: root.checked ? Theme.accent : Theme.surface;
        animate background, border-color { duration: Theme.fast; easing: ease-out; }

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
            x: root.checked ? root.width - self.width - 2px : 2px;
            y: 2px;
            width: root.height - 4px;
            height: root.height - 4px;
            border-radius: self.height / 2;
            background: root.checked ? Theme.surface : Theme.border;
            animate x { duration: Theme.fast; easing: ease-out; }
            animate background { duration: Theme.fast; }
        }
    }

    component SettingsTabButton inherits Rectangle {
        in property <string> text;
        in property <image> icon_source;
        in property <bool> active: false;
        callback clicked();

        border-radius: 8px;
        border-width: 1px;
        border-color: root.active ? Theme.accent : transparent;
        background: root.active ? Theme.accent : (touch.has-hover ? Theme.accent-soft : transparent);
        animate background, border-color { duration: Theme.fast; easing: ease-out; }

        states [
            pressed when touch.pressed : {
                background: root.active ? Theme.accent-deep : Theme.accent-softer;
                border-color: root.active ? Theme.accent-deep : Theme.accent-softer;
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
            colorize: root.active ? #ffffff : Theme.ink-muted;
        }

        Text {
            x: 30px;
            y: 4px;
            width: root.width - 36px;
            height: root.height - 8px;
            text: root.text;
            overflow: elide;
            vertical-alignment: center;
            font-size: 12px;
            font-weight: 600;
            color: root.active ? #ffffff : Theme.ink-secondary;
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
            background: Theme.track;
        }

        Rectangle {
            x: 0px;
            y: (root.height - 8px) / 2;
            width: root.handle-x + root.handle-size / 2;
            height: 8px;
            border-radius: 4px;
            background: Theme.accent;
        }

        Rectangle {
            x: root.handle-x;
            y: (root.height - root.handle-size) / 2;
            width: root.handle-size;
            height: root.handle-size;
            border-radius: root.handle-size / 2;
            background: touch.pressed ? Theme.accent-deep : Theme.surface;
            border-width: 2px;
            border-color: Theme.accent;
            animate background { duration: Theme.fast; }
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
        border-color: Theme.border;
        background: Theme.surface;

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
            color: Theme.ink;
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
        border-color: Theme.card-border;
        background: Theme.surface;

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
            color: Theme.ink;
            font-size: 14px;
            font-weight: 700;
            overflow: elide;
        }

        Text {
            x: 16px;
            y: 38px;
            width: root.width - 32px;
            text: root.hint;
            color: Theme.ink-muted;
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
            color: Theme.ink-muted;
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
        border-color: touch.has-hover ? Theme.accent : Theme.border;
        background: touch.pressed ? Theme.accent-softer : (touch.has-hover ? Theme.accent-soft : Theme.surface);
        animate background, border-color { duration: Theme.fast; easing: ease-out; }

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
            color: Theme.ink;
        }
        // Drawn chevron: a font glyph here can render as tofu when the
        // fallback font lacks it.
        Path {
            x: root.width - 30px;
            y: (root.height - 14px) / 2;
            width: 14px;
            height: 14px;
            viewbox-width: 14;
            viewbox-height: 14;
            commands: "M 3 5.5 L 7 9.5 L 11 5.5";
            stroke: touch.has-hover ? Theme.accent : Theme.ink-muted;
            stroke-width: 1.5px;
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
                background: Theme.surface;
                border-radius: 8px;
                border-width: 1px;
                border-color: Theme.border;
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
        in property <brush> accent: Theme.accent;

        background: transparent;

        Text {
            x: 0px;
            y: 0px;
            width: 62px;
            height: root.height;
            text: root.label;
            overflow: elide;
            vertical-alignment: center;
            color: Theme.ink-secondary;
            font-size: 12px;
        }
        Rectangle {
            x: 70px;
            y: (root.height - 6px) / 2;
            width: root.width - 116px;
            height: 6px;
            border-radius: 3px;
            background: Theme.track;
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
            color: Theme.ink;
            font-size: 12px;
        }
    }

    struct AnimField {
        label: string,
        value: string,
    }

    struct StatTrendBar {
        day_label: string,
        height: float,
        today: bool,
    }

    struct StatHighlight {
        label: string,
        value: string,
    }

    // Compact headline metric card used by the Stats tab.
    component StatKpiCard inherits Rectangle {
        in property <string> label;
        in property <string> value;
        in property <string> sub;
        in property <brush> accent: Theme.accent;

        background: Theme.sunken;
        border-radius: 8px;
        border-width: 1px;
        border-color: Theme.card-border;

        Rectangle {
            x: parent.width - 18px;
            y: 14px;
            width: 6px;
            height: 6px;
            border-radius: 3px;
            background: root.accent;
        }
        Text {
            x: 14px;
            y: 12px;
            width: parent.width - 40px;
            text: root.label;
            overflow: elide;
            color: Theme.ink-faint;
            font-size: 11px;
        }
        Text {
            x: 14px;
            y: 28px;
            width: parent.width - 28px;
            text: root.value;
            overflow: elide;
            color: Theme.ink;
            font-size: 20px;
            font-weight: 700;
        }
        Text {
            x: 14px;
            y: 54px;
            width: parent.width - 28px;
            text: root.sub;
            overflow: elide;
            color: Theme.ink-muted;
            font-size: 11px;
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
        background: Theme.window-bg;
        // Bundled Maple Mono CN (registered in ensure_embedded_fonts) covers
        // Latin + CJK incl. fullwidth punctuation (【】。) from one monospace
        // family, so nothing depends on system fonts or renderer fallback.
        default-font-family: "Maple Mono CN";
        default-font-size: 13px;

        property <length> content_width: 584px;

        in-out property <int> active_tab: 0;
        in-out property <float> pet_scale: 80;
        in-out property <float> sleep_after: 75;
        in-out property <bool> show_session_switcher: true;
        in-out property <string> pet_dir;
        in-out property <string> gif_dir;
        // Mood GIF filenames; order is fixed in settings_panel/controller/basic.rs.
        in property <[AnimField]> anim_fields;
        callback anim_field_changed(int, string);

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
        in property <string> profile_usage_five_hour_reset;
        in property <string> profile_usage_seven_day_reset;
        in property <float> profile_usage_five_hour_bar;
        in property <float> profile_usage_seven_day_bar;

        in property <string> stats_kpi_prompts;
        in property <string> stats_kpi_prompts_sub;
        in property <string> stats_kpi_tokens;
        in property <string> stats_kpi_tokens_sub;
        in property <string> stats_kpi_cache;
        in property <string> stats_kpi_cache_sub;
        in property <string> stats_kpi_tools;
        in property <string> stats_kpi_tools_sub;
        in property <[StatTrendBar]> stats_trend;
        in property <string> stats_trend_caption;
        in property <[StatHighlight]> stats_highlights;
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

        Rectangle { x: 0px; y: 0px; width: root.width; height: root.height; background: Theme.window-bg; }
        Rectangle {
            x: 8px;
            y: 8px;
            width: root.width - 16px;
            height: root.height - 16px;
            background: Theme.surface;
            border-radius: 8px;
            border-width: 1px;
            border-color: Theme.outline;
        }
        TouchArea { x: 0px; y: 0px; width: root.width; height: root.height; }

        Rectangle {
            x: 16px;
            y: 16px;
            width: 144px;
            height: root.height - 32px;
            background: Theme.sunken-alt;
            border-radius: 8px;
            border-width: 1px;
            border-color: Theme.hairline;
        }

        Text {
            x: 28px;
            y: 28px;
            width: 120px;
            text: "claudie";
            font-size: 20px;
            font-weight: 700;
            color: Theme.ink;
        }
        Text {
            x: 28px;
            y: 54px;
            width: 120px;
            text: "Settings";
            font-size: 12px;
            font-weight: 600;
            color: Theme.ink-muted;
        }

        SettingsTabButton { x: 24px; y: 84px; width: 128px; height: 36px; text: "Basic"; icon_source: @image-url("../../assets/lucide/sliders-horizontal.svg"); active: root.active_tab == 0; clicked => { root.active_tab = 0; } }
        SettingsTabButton { x: 24px; y: 128px; width: 128px; height: 36px; text: "Pomodoro"; icon_source: @image-url("../../assets/lucide/timer.svg"); active: root.active_tab == 1; clicked => { root.active_tab = 1; } }
        SettingsTabButton { x: 24px; y: 172px; width: 128px; height: 36px; text: "LLM Profiles"; icon_source: @image-url("../../assets/lucide/bot.svg"); active: root.active_tab == 2; clicked => { root.active_tab = 2; } }
        SettingsTabButton { x: 24px; y: 216px; width: 128px; height: 36px; text: "Stats"; icon_source: @image-url("../../assets/lucide/chart-no-axes-column.svg"); active: root.active_tab == 3; clicked => { root.active_tab = 3; } }

        Rectangle { x: 168px; y: 16px; width: 1px; height: root.height - 32px; background: Theme.hairline; }

        ScrollView {
            x: 176px;
            y: 16px;
            width: root.width - 192px;
            height: root.height - 56px;
            viewport-width: root.content_width;
            viewport-height: root.active_tab == 0 ? 608px : (root.active_tab == 1 ? 456px : (root.active_tab == 2 ? 612px : 600px));

            if active_tab == 0: Rectangle {
                width: root.content_width;
                height: 608px;
                background: transparent;

                Text { x: 0px; y: 0px; text: "Pet renderer"; font-size: 17px; font-weight: 700; color: Theme.ink; }
                Text { x: 0px; y: 28px; width: 576px; text: "Tune the desktop pet size and map each mood to a GIF filename."; font-size: 13px; color: Theme.ink-faint; }

                Text { x: 0px; y: 64px; text: "Pet size"; color: Theme.ink-faint; font-size: 12px; }
                PointerSlider {
                    x: 0px; y: 84px; width: 220px; height: 32px;
                    minimum: 50; maximum: 150; step: 1;
                    value <=> root.pet_scale;
                    changed(value) => { root.pet_scale_changed(value); }
                }
                Text { x: 232px; y: 90px; width: 52px; text: Math.round(root.pet_scale) + "%"; color: Theme.ink; font-size: 13px; }
                Text { x: 300px; y: 64px; text: "Sleep after"; color: Theme.ink-faint; font-size: 12px; }
                PointerSlider {
                    x: 300px; y: 84px; width: 220px; height: 32px;
                    minimum: 15; maximum: 1800; step: 15;
                    value <=> root.sleep_after;
                    changed(value) => { root.sleep_after_changed(value); }
                }
                Text { x: 532px; y: 90px; width: 52px; text: Math.round(root.sleep_after) + "s"; color: Theme.ink; font-size: 13px; }

                FieldGroup { x: 0px; y: 128px; width: 284px; label: "Pet asset directory"; value <=> root.pet_dir; }
                FieldGroup { x: 300px; y: 128px; width: 284px; label: "GIF directory"; value <=> root.gif_dir; }

                for field[i] in root.anim_fields: FieldGroup {
                    x: Math.mod(i, 4) * 150px;
                    y: 204px + Math.floor(i / 4) * 64px;
                    width: 134px;
                    label: field.label;
                    value: field.value;
                    edited(text) => { root.anim_field_changed(i, text); }
                }

                Text { x: 0px; y: 472px; text: "Session switcher"; color: Theme.ink-faint; font-size: 12px; }
                Text { x: 0px; y: 494px; width: 504px; height: 48px; text: "Show the compact focus panel when more than one Claude Code session is active."; wrap: word-wrap; color: Theme.ink; font-size: 13px; }
                TogglePill { x: 538px; y: 482px; width: 46px; height: 24px; checked <=> root.show_session_switcher; }

                ActionButton { x: 408px; y: 556px; width: 80px; height: 32px; text: "Save"; kind: "primary"; clicked => { root.save_basic(); } }
                ActionButton { x: 504px; y: 556px; width: 80px; height: 32px; text: "Reset"; clicked => { root.reset_basic(); } }
            }

            if active_tab == 1: Rectangle {
                width: root.content_width;
                height: 456px;
                background: transparent;

                Text { x: 0px; y: 0px; text: "Pomodoro"; font-size: 17px; font-weight: 700; color: Theme.ink; }
                Text { x: 0px; y: 28px; width: 576px; text: "Set focus and break lengths, then control the active timer."; font-size: 13px; color: Theme.ink-faint; }

                Rectangle {
                    x: 0px;
                    y: 64px;
                    width: 584px;
                    height: 128px;
                    background: Theme.accent-tint;
                    border-radius: 8px;
                    border-width: 1px;
                    border-color: Theme.accent-tint-border;
                }
                Rectangle { x: 0px; y: 64px; width: 5px; height: 128px; background: Theme.accent; border-radius: 8px; }
                Rectangle { x: 24px; y: 88px; width: 52px; height: 52px; background: Theme.surface; border-radius: 8px; border-width: 1px; border-color: Theme.accent-tint-border; }
                Image { x: 38px; y: 102px; width: 24px; height: 24px; source: @image-url("../../assets/lucide/timer.svg"); image-fit: contain; colorize: Theme.accent; }
                Text { x: 96px; y: 86px; width: 160px; text: "Current cycle"; color: Theme.ink-muted; font-size: 12px; font-weight: 700; }
                Text { x: 96px; y: 108px; width: 456px; height: 48px; text: root.pomodoro_status; wrap: word-wrap; color: Theme.ink; font-size: 15px; font-weight: 700; }
                Text { x: 96px; y: 162px; width: 456px; text: "Tune the rhythm below, then use the controls without leaving this panel."; color: Theme.ink-muted; font-size: 12px; overflow: elide; }

                Text { x: 0px; y: 216px; text: "Durations"; color: Theme.ink; font-size: 14px; font-weight: 700; }
                ActionButton { x: 504px; y: 206px; width: 80px; height: 32px; text: "Save"; clicked => { root.save_pomodoro(); } }

                PomodoroDurationTile {
                    x: 0px; y: 248px; width: 184px; height: 116px;
                    title: "Focus";
                    hint: "Deep work";
                    accent: Theme.accent;
                    value <=> root.focus_minutes;
                }
                PomodoroDurationTile {
                    x: 200px; y: 248px; width: 184px; height: 116px;
                    title: "Short break";
                    hint: "Quick reset";
                    accent: Theme.chart-teal;
                    value <=> root.short_break_minutes;
                }
                PomodoroDurationTile {
                    x: 400px; y: 248px; width: 184px; height: 116px;
                    title: "Long break";
                    hint: "Full recharge";
                    accent: Theme.chart-purple;
                    value <=> root.long_break_minutes;
                }

                Rectangle { x: 0px; y: 392px; width: 584px; height: 48px; background: Theme.sunken-alt; border-radius: 8px; border-width: 1px; border-color: Theme.hairline; }
                ActionButton { x: 16px; y: 400px; width: 112px; height: 32px; text: "Start"; kind: "primary"; clicked => { root.start_pomodoro(); } }
                ActionButton { x: 144px; y: 400px; width: 112px; height: 32px; text: root.pause_resume_label; clicked => { root.pause_resume_pomodoro(); } }
                ActionButton { x: 272px; y: 400px; width: 112px; height: 32px; text: "Skip"; clicked => { root.skip_pomodoro(); } }
                ActionButton { x: 456px; y: 400px; width: 112px; height: 32px; text: "Stop"; kind: "danger"; clicked => { root.stop_pomodoro(); } }
            }

            if active_tab == 2: Rectangle {
                width: root.content_width;
                height: 612px;
                background: transparent;

                Text { x: 0px; y: 0px; text: "Provider profiles"; font-size: 17px; font-weight: 700; color: Theme.ink; }
                Text { x: 0px; y: 28px; width: 576px; text: "Keep Claude Code provider settings tidy without leaving the pet."; font-size: 13px; color: Theme.ink-faint; }

                Text { x: 0px; y: 72px; text: "Profile"; color: Theme.ink-faint; font-size: 12px; }
                PointerComboBox {
                    x: 0px; y: 92px; width: 200px; height: 32px;
                    model: root.profile_model;
                    current-index <=> root.selected_profile_index;
                    selected(index) => { root.select_profile(index); }
                }
                Text { x: 208px; y: 98px; width: 88px; text: root.profile_position; overflow: elide; color: Theme.ink-faint; font-size: 12px; }
                ActionButton { x: 304px; y: 92px; width: 60px; height: 32px; text: "New"; clicked => { root.new_profile(); } }
                ActionButton { x: 372px; y: 92px; width: 124px; height: 32px; text: "Import Current"; clicked => { root.import_profile(); } }
                ActionButton { x: 504px; y: 92px; width: 80px; height: 32px; text: "Delete"; kind: "danger"; clicked => { root.delete_profile(); } }

                FieldGroup { x: 0px; y: 136px; width: 284px; label: "Profile ID"; value <=> root.profile_id; }
                FieldGroup { x: 300px; y: 136px; width: 284px; label: "Name"; value <=> root.profile_name; }

                FieldGroup { x: 0px; y: 200px; width: 284px; label: "Model"; value <=> root.model; }
                FieldGroup { x: 300px; y: 200px; width: 284px; label: "Base URL"; value <=> root.base_url; }

                FieldGroup { x: 0px; y: 264px; width: 284px; label: "API key"; input-type: InputType.password; value <=> root.api_key; }
                FieldGroup { x: 300px; y: 264px; width: 284px; label: "Auth token (proxy)"; input-type: InputType.password; value <=> root.auth_token; }

                FieldGroup { x: 0px; y: 328px; width: 184px; label: "Opus"; value <=> root.opus_model; }
                FieldGroup { x: 200px; y: 328px; width: 184px; label: "Sonnet"; value <=> root.sonnet_model; }
                FieldGroup { x: 400px; y: 328px; width: 184px; label: "Haiku"; value <=> root.haiku_model; }

                Text { x: 0px; y: 392px; text: "Extra env"; color: Theme.ink-faint; font-size: 12px; }
                MonoTextEdit { x: 0px; y: 412px; width: 284px; height: 72px; text <=> root.extra_env; }
                Text { x: 300px; y: 392px; text: "OpenAI body"; color: Theme.ink-faint; font-size: 12px; }
                MonoTextEdit { x: 300px; y: 412px; width: 284px; height: 72px; text <=> root.openai_extra_body; }

                Rectangle { x: 0px; y: 504px; width: 432px; height: 96px; background: Theme.sunken; border-radius: 8px; border-width: 1px; border-color: Theme.card-border; }
                Text { x: 14px; y: 512px; width: 404px; text: root.profile_usage_title; overflow: elide; color: Theme.ink; font-size: 13px; font-weight: 600; }
                Text { x: 14px; y: 532px; width: 404px; text: root.profile_usage_summary; overflow: elide; color: Theme.ink-secondary; font-size: 11px; }
                StatBarRow { x: 14px; y: 554px; width: 240px; height: 18px; label: "5h"; value: root.profile_usage_five_hour_value; bar: root.profile_usage_five_hour_bar; accent: root.profile_usage_five_hour_bar >= 90 ? Theme.danger : (root.profile_usage_five_hour_bar >= 70 ? Theme.chart-amber : Theme.accent); }
                Text { x: 262px; y: 554px; width: 156px; height: 18px; text: root.profile_usage_five_hour_reset; overflow: elide; vertical-alignment: center; color: Theme.ink-secondary; font-size: 11px; }
                StatBarRow { x: 14px; y: 576px; width: 240px; height: 18px; label: "7d"; value: root.profile_usage_seven_day_value; bar: root.profile_usage_seven_day_bar; accent: root.profile_usage_seven_day_bar >= 90 ? Theme.danger : (root.profile_usage_seven_day_bar >= 70 ? Theme.chart-amber : Theme.chart-purple); }
                Text { x: 262px; y: 576px; width: 156px; height: 18px; text: root.profile_usage_seven_day_reset; overflow: elide; vertical-alignment: center; color: Theme.ink-secondary; font-size: 11px; }

                ActionButton { x: 448px; y: 514px; width: 64px; height: 32px; text: "Save"; clicked => { root.save_profile(); } }
                ActionButton { x: 520px; y: 514px; width: 64px; height: 32px; text: "Use"; kind: "primary"; clicked => { root.use_profile(); } }
            }

            if active_tab == 3: Rectangle {
                width: root.content_width;
                height: 600px;
                background: transparent;

                Text { x: 0px; y: 0px; text: "Session ledger"; font-size: 17px; font-weight: 700; color: Theme.ink; }
                Text { x: 0px; y: 28px; width: 576px; text: "A quiet local record of Claude Code activity observed by claudie."; font-size: 13px; color: Theme.ink-faint; }

                // Headline metrics — today, with a 7-day comparison beneath.
                StatKpiCard { x: 0px; y: 60px; width: 137px; height: 74px; label: "Prompts"; value: root.stats_kpi_prompts; sub: root.stats_kpi_prompts_sub; accent: Theme.accent; }
                StatKpiCard { x: 149px; y: 60px; width: 137px; height: 74px; label: "Tokens"; value: root.stats_kpi_tokens; sub: root.stats_kpi_tokens_sub; accent: Theme.chart-blue; }
                StatKpiCard { x: 298px; y: 60px; width: 137px; height: 74px; label: "Cache hit"; value: root.stats_kpi_cache; sub: root.stats_kpi_cache_sub; accent: Theme.chart-teal; }
                StatKpiCard { x: 447px; y: 60px; width: 137px; height: 74px; label: "Tool calls"; value: root.stats_kpi_tools; sub: root.stats_kpi_tools_sub; accent: Theme.chart-purple; }

                // Activity trend — prompts per day across the last 14 days.
                Rectangle { x: 0px; y: 148px; width: 584px; height: 154px; background: Theme.sunken; border-radius: 8px; border-width: 1px; border-color: Theme.card-border;
                    Text { x: 14px; y: 14px; text: "Activity"; color: Theme.ink; font-size: 14px; font-weight: 600; }
                    Text { x: 200px; y: 16px; width: 370px; text: root.stats_trend_caption; horizontal-alignment: right; overflow: elide; color: Theme.ink-muted; font-size: 11px; }
                    for bar[i] in root.stats_trend: Rectangle {
                        x: 14px + i * 39px;
                        y: 50px;
                        width: 39px;
                        height: 90px;
                        Rectangle {
                            x: (parent.width - 22px) / 2;
                            width: 22px;
                            height: bar.height <= 0 ? 0px : max(3px, 64px * bar.height / 100);
                            y: 64px - self.height;
                            border-radius: 3px;
                            background: bar.today ? Theme.accent : Theme.chart-blue;
                        }
                        Text {
                            x: 0px;
                            y: 70px;
                            width: parent.width;
                            text: bar.day_label;
                            horizontal-alignment: center;
                            color: bar.today ? Theme.accent : Theme.ink-faint;
                            font-size: 9px;
                        }
                    }
                }

                // Productivity highlights over the last 7 days.
                Rectangle { x: 0px; y: 314px; width: 584px; height: 64px; background: Theme.sunken; border-radius: 8px; border-width: 1px; border-color: Theme.card-border;
                    for h[i] in root.stats_highlights: Rectangle {
                        x: 14px + i * 139px;
                        y: 0px;
                        width: 139px;
                        height: 64px;
                        Text { x: 0px; y: 14px; width: parent.width - 14px; text: h.label; overflow: elide; color: Theme.ink-faint; font-size: 11px; }
                        Text { x: 0px; y: 32px; width: parent.width - 14px; text: h.value; overflow: elide; color: Theme.ink; font-size: 15px; font-weight: 600; }
                    }
                }

                // Detailed 7-day distribution.
                Rectangle { x: 0px; y: 390px; width: 284px; height: 200px; background: Theme.surface; border-radius: 8px; border-width: 1px; border-color: Theme.card-border; }
                Text { x: 24px; y: 410px; width: 236px; text: "Tool mix · 7 days"; color: Theme.ink; font-size: 14px; font-weight: 600; }
                StatBarRow { x: 24px; y: 440px; width: 236px; height: 20px; label: "Write"; value: root.stats_recent_write_value; bar: root.stats_recent_write_bar; accent: Theme.chart-teal; }
                StatBarRow { x: 24px; y: 464px; width: 236px; height: 20px; label: "Bash"; value: root.stats_recent_bash_value; bar: root.stats_recent_bash_bar; accent: Theme.chart-blue; }
                StatBarRow { x: 24px; y: 488px; width: 236px; height: 20px; label: "Search"; value: root.stats_recent_search_value; bar: root.stats_recent_search_bar; accent: Theme.chart-amber; }
                StatBarRow { x: 24px; y: 512px; width: 236px; height: 20px; label: "Agent"; value: root.stats_recent_subagent_value; bar: root.stats_recent_subagent_bar; accent: Theme.chart-purple; }
                StatBarRow { x: 24px; y: 536px; width: 236px; height: 20px; label: "Perm"; value: root.stats_recent_permission_value; bar: root.stats_recent_permission_bar; accent: Theme.accent; }
                StatBarRow { x: 24px; y: 560px; width: 236px; height: 20px; label: "Choice"; value: root.stats_recent_choice_value; bar: root.stats_recent_choice_bar; accent: Theme.chart-olive; }

                Rectangle { x: 300px; y: 390px; width: 284px; height: 200px; background: Theme.surface; border-radius: 8px; border-width: 1px; border-color: Theme.card-border; }
                Text { x: 324px; y: 410px; width: 236px; text: "Tokens · 7 days"; color: Theme.ink; font-size: 14px; font-weight: 600; }
                StatBarRow { x: 324px; y: 440px; width: 236px; height: 20px; label: "Input"; value: root.stats_recent_input_value; bar: root.stats_recent_input_bar; accent: Theme.chart-teal; }
                StatBarRow { x: 324px; y: 464px; width: 236px; height: 20px; label: "Output"; value: root.stats_recent_output_value; bar: root.stats_recent_output_bar; accent: Theme.chart-blue; }
                StatBarRow { x: 324px; y: 488px; width: 236px; height: 20px; label: "Cache W"; value: root.stats_recent_cache_write_value; bar: root.stats_recent_cache_write_bar; accent: Theme.chart-amber; }
                StatBarRow { x: 324px; y: 512px; width: 236px; height: 20px; label: "Cache R"; value: root.stats_recent_cache_read_value; bar: root.stats_recent_cache_read_bar; accent: Theme.chart-purple; }
            }
        }

        Text {
            x: 176px;
            y: root.height - 32px;
            width: root.width - 192px;
            height: 20px;
            text: root.status_message;
            overflow: elide;
            color: Theme.ink-faint;
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
                    // Maple Mono CN is a 2:1 monospace grid (half-width Latin
                    // / full-width CJK) that matches the mono line-count
                    // estimate, so code/diff rows neither clip nor show tofu.
                    font-family: "Maple Mono CN";
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
            font-family: "Maple Mono CN";
            font-size: 12px;
            color: Theme.ink;
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
            color: data.kind == 6 ? Theme.ink-faint : Theme.ink;
        }
    }

    component ChoiceOptionRow inherits Rectangle {
        in property <ChoiceOptionData> data;
        callback toggle();
        callback other_text_changed(string);

        property <length> label_h: data.label_lines * 19px;
        property <length> desc_h: data.description == "" ? 0px : data.desc_lines * 17px + 4px;
        property <bool> show_other: data.selected && data.is_other;
        property <bool> is_header: data.is_question_header;

        width: 100%;
        height: root.is_header
            ? data.desc_lines * 17px + 14px
            : 16px + self.label_h + self.desc_h + (self.show_other ? 38px : 0px);
        background: root.is_header
            ? transparent
            : (data.selected ? Theme.accent-soft
            : (touch.has-hover ? Theme.sunken-alt : Theme.surface));
        border-radius: 8px;
        border-width: root.is_header ? 0px : 1px;
        border-color: data.selected ? Theme.accent : Theme.card-border;
        clip: !root.is_header;
        animate background, border-color { duration: Theme.fast; easing: ease-out; }

        touch := TouchArea {
            width: 100%;
            height: 100%;
            enabled: !root.is_header;
            mouse-cursor: pointer;
            clicked => { root.toggle(); }
        }

        if !root.is_header && data.selected: Rectangle {
            x: 0px; y: 0px;
            width: 3px;
            height: 100%;
            background: Theme.accent;
        }

        if root.is_header: Text {
            x: 4px; y: 8px;
            width: parent.width - 8px;
            height: parent.height - 10px;
            text: data.description;
            font-size: 12px;
            font-weight: 600;
            color: Theme.ink-secondary;
            wrap: word-wrap;
        }

        // Drawn checkbox (multi-select questions).
        if !root.is_header && data.multi_select: Rectangle {
            x: 14px; y: 9px;
            width: 18px;
            height: 18px;
            border-radius: 5px;
            border-width: data.selected ? 0px : 1.5px;
            border-color: Theme.control-border;
            background: data.selected ? Theme.accent : Theme.surface;
            animate background { duration: Theme.fast; }

            if data.selected: Path {
                x: 0px; y: 0px;
                width: 18px;
                height: 18px;
                viewbox-width: 18;
                viewbox-height: 18;
                commands: "M 4.5 9.5 L 7.5 12.5 L 13.5 6.5";
                stroke: #ffffff;
                stroke-width: 2px;
            }
        }

        // Drawn radio (single-select questions): thick accent ring when on.
        if !root.is_header && !data.multi_select: Rectangle {
            x: 14px; y: 9px;
            width: 18px;
            height: 18px;
            border-radius: 9px;
            border-width: data.selected ? 5px : 1.5px;
            border-color: data.selected ? Theme.accent : Theme.control-border;
            background: Theme.surface;
            animate border-width, border-color { duration: Theme.fast; }
        }

        if !root.is_header: Text {
            x: 40px; y: 8px;
            width: parent.width - 52px;
            height: root.label_h;
            text: data.label;
            font-size: 13px;
            font-weight: 600;
            color: Theme.ink;
            wrap: word-wrap;
        }

        if !root.is_header && data.description != "": Text {
            x: 40px; y: 8px + root.label_h + 4px;
            width: parent.width - 52px;
            height: data.desc_lines * 17px;
            text: data.description;
            font-size: 12px;
            color: Theme.ink-faint;
            wrap: word-wrap;
        }

        if !root.is_header && root.show_other: LineEdit {
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
        background: Theme.window-bg;
        default-font-family: "Maple Mono CN";

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
            background: Theme.surface;
            border-radius: 12px;
            border-width: 1px;
            border-color: Theme.outline;
        }

        VerticalLayout {
            x: 32px;
            y: 32px;
            width: 576px;
            height: root.height - 64px;
            spacing: 14px;

            HorizontalLayout {
                height: 48px;
                spacing: 14px;

                Rectangle {
                    width: 48px;
                    border-radius: 12px;
                    background: Theme.accent-tint;
                    border-width: 1px;
                    border-color: Theme.accent-tint-border;

                    Image {
                        x: (parent.width - 24px) / 2;
                        y: (parent.height - 24px) / 2;
                        width: 24px;
                        height: 24px;
                        source: root.is_choice
                            ? @image-url("../../assets/lucide/list-checks.svg")
                            : @image-url("../../assets/lucide/shield-check.svg");
                        image-fit: contain;
                        colorize: Theme.accent;
                    }
                }

                VerticalLayout {
                    spacing: 2px;
                    alignment: center;
                    Text { text: root.title_text; font-size: 18px; font-weight: 700; color: Theme.ink; overflow: elide; }
                    Text { text: root.subtitle_text; font-size: 13px; color: Theme.ink-faint; overflow: elide; }
                }
            }

            Rectangle { height: 1px; background: Theme.hairline; }

            // Detail vs. options split follows the golden ratio (~3:2): plans
            // hand the larger share to the detail panel, question lists invert it.
            Rectangle {
                vertical-stretch: root.detail_dominant ? 3 : 2;
                min-height: 120px;
                background: Theme.sunken;
                border-radius: 12px;
                border-width: 1px;
                border-color: Theme.card-border;
                clip: true;

                ScrollView {
                    x: 12px; y: 12px;
                    width: parent.width - 24px;
                    height: parent.height - 24px - 26px;
                    VerticalLayout {
                        padding: 0px;
                        spacing: 8px;
                        for block in root.detail_blocks: MarkdownBlockRow {
                            data: block;
                        }
                    }
                }

                // Session/CWD strip lives inside the card instead of floating
                // between panels.
                Rectangle {
                    x: 0px;
                    y: parent.height - 27px;
                    width: parent.width;
                    height: 1px;
                    background: Theme.card-border;
                }
                Text {
                    x: 14px;
                    y: parent.height - 23px;
                    width: parent.width - 28px;
                    height: 18px;
                    text: root.meta_text;
                    font-size: 11px;
                    color: Theme.ink-disabled;
                    overflow: elide;
                }
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
                color: Theme.ink-faint;
            }

            if is_choice && !submit_enabled: Text {
                height: 16px;
                text: root.submit_hint;
                font-size: 12px;
                color: Theme.warning;
                horizontal-alignment: center;
                overflow: elide;
            }

            HorizontalLayout {
                height: 40px;
                spacing: 12px;

                if !is_choice: ActionButton { width: 96px; kind: "danger"; text: "Deny"; clicked => { root.deny(); } }
                if is_choice: ActionButton { width: 96px; kind: "danger"; text: "Cancel"; clicked => { root.cancel_choice(); } }

                Rectangle { horizontal-stretch: 1; }

                if !is_choice: ActionButton { width: 104px; text: "Always"; clicked => { root.allow_always(); } }
                if !is_choice: ActionButton { width: 96px; kind: "primary"; text: "Allow"; clicked => { root.allow_once(); } }

                if is_choice: ActionButton { width: 104px; kind: "primary"; text: "Submit"; enabled: root.submit_enabled; clicked => { root.submit_choice(); } }
            }
        }
    }
}

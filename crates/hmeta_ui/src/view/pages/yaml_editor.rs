use super::super::*;

pub(crate) fn yaml_editor_dialog(state: Signal<State>, current: &State) -> Element {
    let content_key = dialog_content_key(&[
        if current.yaml_editor_testing {
            "testing"
        } else {
            "idle-test"
        },
        if current.yaml_editor_saving {
            "saving"
        } else {
            "idle-save"
        },
        current.yaml_editor_error.as_deref().unwrap_or(""),
    ]);
    rsx! {
        FlatDialog {
            open: true,
            content_key: content_key,
            on_close: move |_| {
                let busy = {
                    let current = state.read();
                    current.yaml_editor_testing || current.yaml_editor_saving
                };
                if !busy {
                    dispatch(state, Action::SetYamlEditorOpen(false));
                }
            },
            YamlEditorDialogBody { state }
        }
    }
}

#[component]
fn YamlEditorDialogBody(state: Signal<State>) -> Element {
    let current = state.read().clone();
    let summary = summarize_yaml_edit(&current.yaml_editor_text, &current.yaml_editor_original);
    let busy = current.yaml_editor_testing || current.yaml_editor_saving;
    rsx! {
        DialogHeader {
            title: strings(current.locale).profiles_yaml_editor_title.to_owned(),
            description: Some(current.yaml_editor_profile_name.clone()),
        }
        row { height: 16.0 }
        column {
            width: "100%",
            text {
                content: format!("{} {} · {} {} · {}", summary.lines, strings(current.locale).profiles_yaml_lines_unit, summary.characters, strings(current.locale).profiles_yaml_chars_unit, if summary.changed { strings(current.locale).profiles_yaml_changed } else { strings(current.locale).profiles_yaml_unchanged }),
                font_size: 12.0,
                font_color: subtle(),
            }
            row { height: 8.0 }
            Textarea {
                value: Some(current.yaml_editor_text.clone()),
                placeholder: Some(strings(current.locale).profiles_yaml_content.to_owned()),
                height: Some(260.0),
                width: Some("100%".into()),
                disabled: busy,
                on_change: move |value| dispatch(state, Action::SetYamlEditorText(value)),
            }
            if let Some(error) = current.yaml_editor_error.clone() {
                text { content: error, margin_top: 8.0, font_size: 12.0, font_color: danger() }
            }
        }
        DialogFooter {
            row {
                width: "100%",
                FlatButton {
                    variant: FlatButtonVariant::Ghost,
                    size: ButtonSize::Sm,
                    disabled: Some(busy),
                    onclick: move |_| dispatch(state, Action::ResetYamlEditorText),
                    {arkit::icon("rotate-ccw", 14.0, text_color())}
                    text { content: strings(current.locale).profiles_yaml_reset, margin_left: 6.0, font_size: 12.0, font_weight: 600, font_color: text_color() }
                }
                row { layout_weight: 1.0 }
                FlatButton {
                    variant: FlatButtonVariant::Outline,
                    size: ButtonSize::Sm,
                    disabled: Some(busy),
                    onclick: move |_| dispatch(state, Action::TestYamlEditor),
                    if current.yaml_editor_testing {
                        Spinner { size: 14.0, color: Some(text_color()) }
                    } else {
                        {arkit::icon("check", 14.0, text_color())}
                    }
                    text { content: if current.yaml_editor_testing { strings(current.locale).profiles_yaml_testing } else { strings(current.locale).profiles_yaml_test }, margin_left: 6.0, font_size: 12.0, font_weight: 600, font_color: text_color() }
                }
                row { width: 8.0 }
                FlatButton {
                    variant: FlatButtonVariant::Primary,
                    size: ButtonSize::Sm,
                    disabled: Some(busy),
                    onclick: move |_| dispatch(state, Action::SaveYamlEditor),
                    if current.yaml_editor_saving {
                        Spinner { size: 14.0, color: Some(primary_text()) }
                    } else {
                        {arkit::icon("save", 14.0, primary_text())}
                    }
                    text { content: if current.yaml_editor_saving { strings(current.locale).profiles_yaml_saving } else { strings(current.locale).profiles_yaml_save }, margin_left: 6.0, font_size: 12.0, font_weight: 600, font_color: primary_text() }
                }
            }
        }
    }
}

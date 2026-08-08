use super::super::*;

pub(crate) fn logs_page(state: Signal<State>) -> Element {
    let mut log_query = use_signal(String::new);
    let mut log_filter = use_signal(|| LogLevelFilter::All);
    let mut history_open = use_signal(|| false);
    let mut selected_log = use_signal(|| None::<VirtualLogRow>);
    let mut delete_archive = use_signal(|| None::<String>);
    let current = state.read().clone();
    let locale = current.locale;
    let recording_enabled = current.log_recording.enabled;
    let recording_pending = current.log_recording_pending;
    let export_pending = current.log_archive_export_pending.clone();
    let delete_pending = current.log_archive_delete_pending.clone();
    let current_tab = translate_ui(locale, tr::page_tr_260());
    let history_tab = translate_ui(locale, tr::page_tr_261());
    let tab_options = vec![current_tab.clone(), history_tab.clone()];
    let selected_tab = if history_open() {
        history_tab.clone()
    } else {
        current_tab.clone()
    };
    let query_value = log_query();
    let normalized_query = normalize_log_query(&query_value);
    let filter_value = log_filter();
    let all_label = translate_ui(current.locale, tr::logs_level_all());
    let info_label = "Info".to_owned();
    let warn_label = "Warn".to_owned();
    let error_label = "Error".to_owned();
    let debug_label = "Debug".to_owned();
    let filter_options = vec![
        all_label.clone(),
        info_label.clone(),
        warn_label.clone(),
        error_label.clone(),
        debug_label.clone(),
    ];
    let selected_filter = match filter_value {
        LogLevelFilter::All => all_label.clone(),
        LogLevelFilter::Info => info_label.clone(),
        LogLevelFilter::Warning => warn_label.clone(),
        LogLevelFilter::Error => error_label.clone(),
        LogLevelFilter::Debug => debug_label.clone(),
    };
    let total_log_count = current.snapshot.logs.len();
    let logs = current
        .snapshot
        .logs
        .iter()
        .filter(|log| matches_log_filter_normalized(log, filter_value, &normalized_query))
        .rev()
        .cloned()
        .map(|log| {
            let color = match log.level.to_ascii_lowercase().as_str() {
                "error" => danger(),
                "warning" | "warn" => warning(),
                "info" => success(),
                _ => subtle(),
            };
            VirtualLogRow {
                meta: format!(
                    "{}  ·  {}",
                    log.level.to_uppercase(),
                    time_format::format_unix_seconds(&log.timestamp).unwrap_or(log.timestamp),
                ),
                preview: truncate_text(&log.message.replace(['\n', '\r'], " "), 150),
                message: log.message,
                color,
            }
        })
        .collect::<Vec<_>>();
    let empty = logs.is_empty();
    let shown_log_count = logs.len();
    let palette = VirtualLogPalette {
        surface: surface(),
        foreground: text_color(),
        muted_foreground: subtle(),
        border: line(),
        danger: danger(),
    };
    let selected_log_value = selected_log();
    let delete_archive_value = delete_archive();
    let archives_empty = current.log_recording.archives.is_empty();
    let archives = current
        .log_recording
        .archives
        .iter()
        .cloned()
        .map(|archive| {
            let updated_at = archive
                .updated_at
                .as_deref()
                .and_then(time_format::format_unix_seconds)
                .unwrap_or_else(|| archive.date.clone());
            let detail = format!("{} · {}", format_total(archive.bytes), updated_at);
            VirtualLogArchiveRow {
                exporting: export_pending.as_deref() == Some(archive.file_name.as_str()),
                deleting: delete_pending.as_deref() == Some(archive.file_name.as_str()),
                export_disabled: export_pending.is_some() || delete_pending.is_some(),
                delete_disabled: archive.active
                    || export_pending.is_some()
                    || delete_pending.is_some(),
                detail: if archive.active {
                    format!(
                        "{detail} · {}",
                        tr(
                            locale,
                            "正在写入，停止记录后可删除",
                            "Recording; stop before deleting"
                        )
                    )
                } else {
                    detail
                },
                file_name: archive.file_name,
            }
        })
        .collect::<Vec<_>>();
    let body = rsx! {
        column {
            width: "100%",
            height: "100%",
            row {
                width: "100%",
                height: 32.0,
                align_items: "center",
                text {
                    content: if recording_enabled {
                        translate_ui(locale, tr::page_tr_262())
                    } else {
                        translate_ui(locale, tr::page_tr_263())
                    },
                    font_size: 12.0,
                    font_weight: 600,
                    font_color: if recording_enabled { success() } else { subtle() },
                }
                row { layout_weight: 1.0 }
                text {
                    content: format!(
                        "{} {}",
                        current.log_recording.archives.len(),
                        translate_ui(locale, tr::page_tr_264())
                    ),
                    font_size: 11.0,
                    font_color: subtle(),
                }
            }
            row { height: 6.0 }
            row {
                width: "100%",
                justify_content: "center",
                FlatSegmented {
                    options: tab_options,
                    selected: selected_tab,
                    on_change: move |value: String| {
                        history_open.set(value == history_tab);
                    },
                }
            }
            row { height: 12.0 }
            if history_open() {
                row {
                    layout_weight: 1.0,
                    width: "100%",
                    if archives_empty {
                        {empty_state(
                            "history",
                            translate_ui(locale, tr::page_tr_265()),
                            translate_ui(locale, tr::page_tr_266()),
                        )}
                    } else {
                        VirtualLogArchiveList {
                            items: archives,
                            palette,
                            on_export: move |file_name: String| {
                                dispatch(state, Action::ExportLogArchive(file_name));
                            },
                            on_delete: move |file_name: String| {
                                delete_archive.set(Some(file_name));
                            },
                        }
                    }
                }
            } else {
                Input {
                    value: Some(query_value),
                    placeholder: Some(translate_ui(locale, tr::logs_search_placeholder())),
                    width: Some("100%".into()),
                    on_change: move |value| log_query.set(value),
                }
                row { height: 12.0 }
                row {
                    width: "100%",
                    justify_content: "center",
                    FlatSegmented {
                        options: filter_options,
                        selected: selected_filter,
                        on_change: move |value: String| {
                            let filter = if value == info_label {
                                LogLevelFilter::Info
                            } else if value == warn_label {
                                LogLevelFilter::Warning
                            } else if value == error_label {
                                LogLevelFilter::Error
                            } else if value == debug_label {
                                LogLevelFilter::Debug
                            } else {
                                LogLevelFilter::All
                            };
                            log_filter.set(filter);
                        },
                    }
                }
                row {
                    width: "100%",
                    height: 32.0,
                    align_items: "center",
                    text {
                        content: format!("{} / {} {}", shown_log_count, total_log_count, translate_ui(locale, tr::page_tr_267())),
                        font_size: 11.0,
                        font_color: subtle(),
                    }
                    row { layout_weight: 1.0 }
                    if !empty {
                        text { content: translate_ui(locale, tr::page_tr_268()), font_size: 11.0, font_color: subtle() }
                    }
                }
                row {
                    layout_weight: 1.0,
                    width: "100%",
                    if empty {
                        {empty_state(
                            "scroll-text",
                            translate_ui(locale, tr::logs_empty_title()),
                            if recording_enabled {
                                translate_ui(locale, tr::logs_empty_subtitle())
                            } else {
                                translate_ui(locale, tr::page_tr_269())
                            },
                        )}
                    } else {
                        VirtualLogList {
                            items: logs,
                            palette,
                            on_open: move |row: VirtualLogRow| selected_log.set(Some(row)),
                        }
                    }
                }
            }
        }
    };
    let action = rsx! {
        FlatButton {
            variant: FlatButtonVariant::Ghost,
            size: ButtonSize::Icon,
            disabled: Some(recording_pending),
            onclick: move |_| dispatch(state, Action::ToggleLogRecording),
            if recording_pending {
                Spinner { size: 17.0, color: Some(text_color()) }
            } else if recording_enabled {
                {arkit::icon("square", 17.0, danger())}
            } else {
                {arkit::icon("play", 17.0, success())}
            }
        }
    };
    let page = fixed_scaffold(state, Route::Logs {}, action, body);
    rsx! {
        {page}
        if let Some(log) = selected_log_value {
            {log_detail_dialog(locale, log, selected_log)}
        }
        if let Some(file_name) = delete_archive_value {
            {log_archive_delete_dialog(state, locale, file_name, delete_archive)}
        }
    }
}

fn log_archive_delete_dialog(
    state: Signal<State>,
    locale: UiLocale,
    file_name: String,
    mut selected: Signal<Option<String>>,
) -> Element {
    let delete_file_name = file_name.clone();
    rsx! {
        FlatDialog {
            open: true,
            on_close: move |_| selected.set(None),
            DialogHeader {
                title: translate_ui(locale, tr::page_tr_270()),
                description: Some(format!(
                    "{} · {}",
                    file_name,
                    translate_ui(locale, tr::page_tr_113())
                )),
            }
            row { height: 20.0 }
            DialogFooter {
                row {
                    width: "100%",
                    FlatButton {
                        variant: FlatButtonVariant::Outline,
                        onclick: move |_| selected.set(None),
                        text { content: translate_ui(locale, tr::page_tr_114()), font_size: 13.0, font_weight: 600, font_color: text_color() }
                    }
                    row { layout_weight: 1.0 }
                    FlatButton {
                        variant: FlatButtonVariant::Destructive,
                        onclick: move |_| {
                            selected.set(None);
                            dispatch(state, Action::DeleteLogArchive(delete_file_name.clone()));
                        },
                        text { content: translate_ui(locale, tr::page_tr_115()), font_size: 13.0, font_weight: 600, font_color: destructive_text() }
                    }
                }
            }
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct VirtualLogRow {
    meta: String,
    message: String,
    preview: String,
    color: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct VirtualLogPalette {
    surface: u32,
    foreground: u32,
    muted_foreground: u32,
    border: u32,
    danger: u32,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct VirtualLogArchiveRow {
    file_name: String,
    detail: String,
    exporting: bool,
    deleting: bool,
    export_disabled: bool,
    delete_disabled: bool,
}

#[component]
fn VirtualLogArchiveList(
    items: Vec<VirtualLogArchiveRow>,
    palette: VirtualLogPalette,
    on_export: EventHandler<String>,
    on_delete: EventHandler<String>,
) -> Element {
    let item_keys = items
        .iter()
        .map(|item| {
            let mut hasher = DefaultHasher::new();
            item.hash(&mut hasher);
            palette.hash(&mut hasher);
            hasher.finish()
        })
        .collect::<Vec<_>>();
    let render_items = items;
    let source = use_virtual_source_items_keyed(VirtualKind::List, item_keys, move |index| {
        let Some(item) = render_items.get(index as usize).cloned() else {
            return rsx! {};
        };
        rsx! {
            VirtualLogArchiveRowView { item, palette, on_export, on_delete }
        }
    });

    rsx! {
        list {
            virtual_source: source,
            width: "100%",
            height: "100%",
            list_cached_count: 12_i32,
        }
    }
}

#[component]
fn VirtualLogList(
    items: Vec<VirtualLogRow>,
    palette: VirtualLogPalette,
    on_open: EventHandler<VirtualLogRow>,
) -> Element {
    let item_keys = items
        .iter()
        .map(|item| {
            let mut hasher = DefaultHasher::new();
            item.hash(&mut hasher);
            palette.hash(&mut hasher);
            hasher.finish()
        })
        .collect::<Vec<_>>();
    let render_items = items;
    let source = use_virtual_source_items_keyed(VirtualKind::List, item_keys, move |index| {
        let Some(item) = render_items.get(index as usize).cloned() else {
            return rsx! {};
        };
        rsx! {
            VirtualLogRowView { item, palette, on_open }
        }
    });

    rsx! {
        list {
            virtual_source: source,
            width: "100%",
            height: "100%",
            list_cached_count: 18_i32,
        }
    }
}

#[component]
fn VirtualLogArchiveRowView(
    item: VirtualLogArchiveRow,
    palette: VirtualLogPalette,
    on_export: EventHandler<String>,
    on_delete: EventHandler<String>,
) -> Element {
    let export_color = if item.export_disabled && !item.exporting {
        palette.muted_foreground
    } else {
        palette.foreground
    };
    let export_file_name = item.file_name.clone();
    let delete_color = if item.delete_disabled && !item.deleting {
        palette.muted_foreground
    } else {
        palette.danger
    };
    let delete_file_name = item.file_name.clone();
    let accessibility_text = format!("{}，{}", item.file_name, item.detail);
    rsx! {
        row {
            width: "100%",
            height: 72.0,
            background_color: palette.surface,
            padding_top: 8.0,
            padding_right: 8.0,
            padding_bottom: 8.0,
            padding_left: 14.0,
            margin_bottom: 7.0,
            border_width: 1.0,
            border_color: palette.border,
            border_radius: 9.0,
            clip: true,
            align_items: "center",
            column {
                layout_weight: 1.0,
                align_items: "start",
                justify_content: "center",
                text {
                    width: "100%",
                    content: item.file_name,
                    font_size: 14.0,
                    font_weight: 600,
                    font_color: palette.foreground,
                    line_height: 20.0,
                    max_lines: 1,
                    text_overflow: "ellipsis",
                }
                text {
                    width: "100%",
                    content: item.detail,
                    padding_top: 4.0,
                    font_size: 11.0,
                    font_weight: 400,
                    font_color: palette.muted_foreground,
                    line_height: 16.0,
                    max_lines: 1,
                    text_overflow: "ellipsis",
                }
                text { content: accessibility_text, width: 0.0, height: 0.0, opacity: 0.0 }
            }
            VirtualLogArchiveAction {
                content: if item.exporting { "…".to_owned() } else { "↓".to_owned() },
                color: export_color,
                accessibility: if item.exporting { "exporting log".to_owned() } else { "export log".to_owned() },
                disabled: item.export_disabled,
                on_click: move |_| on_export.call(export_file_name.clone()),
            }
            VirtualLogArchiveAction {
                content: if item.deleting { "…".to_owned() } else { "×".to_owned() },
                color: delete_color,
                accessibility: if item.deleting {
                    "deleting log".to_owned()
                } else if item.delete_disabled {
                    "stop recording before deleting this log".to_owned()
                } else {
                    "delete log".to_owned()
                },
                disabled: item.delete_disabled,
                on_click: move |_| on_delete.call(delete_file_name.clone()),
            }
        }
    }
}

#[component]
fn VirtualLogArchiveAction(
    content: String,
    color: u32,
    accessibility: String,
    disabled: bool,
    on_click: EventHandler<()>,
) -> Element {
    let font_size = if content == "…" { 18.0 } else { 20.0 };
    rsx! {
        text {
            width: 40.0,
            height: 40.0,
            content,
            font_size,
            font_color: color,
            font_weight: 500,
            text_align: "center",
            line_height: 40.0,
            max_lines: 1,
            enabled: !disabled,
            opacity: if disabled { 0.55 } else { 1.0 },
            onclick: move |_| {
                if !disabled {
                    on_click.call(());
                }
            },
        }
        text { content: accessibility, width: 0.0, height: 0.0, opacity: 0.0 }
    }
}

#[component]
fn VirtualLogRowView(
    item: VirtualLogRow,
    palette: VirtualLogPalette,
    on_open: EventHandler<VirtualLogRow>,
) -> Element {
    let accessibility_text = format!("{}，{}", item.meta, item.message);
    let open_item = item.clone();
    rsx! {
        column {
            width: "100%",
            height: 76.0,
            background_color: palette.surface,
            padding_top: 9.0,
            padding_right: 11.0,
            padding_bottom: 9.0,
            padding_left: 11.0,
            margin_bottom: 7.0,
            border_width: 1.0,
            border_color: palette.border,
            border_radius: 9.0,
            clip: true,
            align_items: "start",
            onclick: move |_| on_open.call(open_item.clone()),
            text {
                width: "100%",
                content: item.meta,
                font_size: 10.0,
                font_weight: 500,
                font_color: item.color,
                line_height: 15.0,
                max_lines: 1,
                text_overflow: "ellipsis",
            }
            text {
                width: "100%",
                content: item.preview,
                padding_top: 4.0,
                font_size: 12.0,
                font_weight: 400,
                font_color: palette.foreground,
                line_height: 17.0,
                max_lines: 2,
                text_overflow: "ellipsis",
            }
            text { content: accessibility_text, width: 0.0, height: 0.0, opacity: 0.0 }
        }
    }
}

fn log_detail_dialog(
    locale: UiLocale,
    log: VirtualLogRow,
    mut selected: Signal<Option<VirtualLogRow>>,
) -> Element {
    let detail_height = match log.message.chars().count() {
        0..=160 => 120.0,
        161..=420 => 200.0,
        _ => 300.0,
    };
    rsx! {
        FlatDialog {
            open: true,
            on_close: move |_| selected.set(None),
            DialogHeader {
                title: translate_ui(locale, tr::page_tr_271()),
                description: Some(log.meta),
            }
            row { height: 14.0 }
            scroll {
                width: "100%",
                height: detail_height,
                alignment: "top-start",
                scroll_bar: "off",
                background_color: muted(),
                border_radius: 9.0,
                column {
                    width: "100%",
                    padding: 12.0,
                    align_items: "start",
                    justify_content: "start",
                    text {
                        content: log.message,
                        width: "100%",
                        font_size: 12.0,
                        line_height: 19.0,
                        font_color: text_color(),
                    }
                }
            }
        }
    }
}

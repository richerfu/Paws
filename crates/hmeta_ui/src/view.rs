use super::*;
use crate::manual_rule::{find_manual_rule_conflict, manual_rule_preview};
use crate::notification::{use_notification_center, NotificationHost};
use crate::platform_callbacks;
use arkit::ohos_arkui_binding::{
    common::node::ArkUINode, types::attribute::ArkUINodeAttributeType,
};
use arkit::prelude::*;
use arkit::router::{use_back_handler, use_navigator, use_route, AnimatedOutlet, Router};
use arkit::shadcn::components::{
    Badge, BadgeVariant, BottomNavigation, BottomNavigationItem, Button, ButtonSize, ButtonVariant,
    Card, CardContent, CardHeader, CardTitle, DialogFooter, DialogHeader, Field, FieldContent,
    FieldDescription, FieldOrientation, FieldTitle, Form, FormItem, Input, RadioGroup, Separator,
    Spinner, Switch, Textarea,
};
use arkit::shadcn::theme::{
    spacing, typography, use_theme, Theme, ThemeMode, ThemePreset, ThemeProvider,
};
use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

#[path = "view/pages/mod.rs"]
mod pages;
#[path = "view/route.rs"]
mod route;

use pages::{
    about_page, appearance_page, connections_page, manual_rule_dialog, requests_page,
    settings_page, tools_page,
};
use route::Route;

fn bg() -> u32 {
    use_theme().colors.background
}

fn surface() -> u32 {
    use_theme().colors.card
}

fn muted() -> u32 {
    use_theme().colors.muted
}

fn text_color() -> u32 {
    use_theme().colors.foreground
}

fn subtle() -> u32 {
    use_theme().colors.muted_foreground
}

fn line() -> u32 {
    use_theme().colors.border
}

fn primary_text() -> u32 {
    use_theme().colors.primary_foreground
}

fn destructive_text() -> u32 {
    use_theme().colors.destructive_foreground
}

fn success() -> u32 {
    match use_theme().mode {
        ThemeMode::Light => 0xFF16A34A,
        ThemeMode::Dark => 0xFF4ADE80,
    }
}

fn warning() -> u32 {
    match use_theme().mode {
        ThemeMode::Light => 0xFFD97706,
        ThemeMode::Dark => 0xFFFBBF24,
    }
}

fn danger() -> u32 {
    use_theme().colors.destructive
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum FlatButtonVariant {
    #[default]
    Outline,
    Primary,
    Destructive,
    Ghost,
    Link,
}

impl FlatButtonVariant {
    fn to_button_variant(self) -> ButtonVariant {
        match self {
            Self::Outline => ButtonVariant::Outline,
            Self::Primary => ButtonVariant::Default,
            Self::Destructive => ButtonVariant::Destructive,
            Self::Ghost => ButtonVariant::Ghost,
            Self::Link => ButtonVariant::Link,
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct FlatButtonProps {
    #[props(default)]
    variant: FlatButtonVariant,
    #[props(default)]
    size: ButtonSize,
    disabled: Option<bool>,
    width: Option<String>,
    onclick: Option<EventHandler<()>>,
    children: Element,
}

/// Flat mobile button: shadcn Button variants/sizes with elevation disabled.
#[component]
fn FlatButton(props: FlatButtonProps) -> Element {
    rsx! {
        Button {
            variant: props.variant.to_button_variant(),
            size: props.size,
            disabled: props.disabled,
            width: props.width,
            shadow: Some(false),
            onclick: props.onclick,
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct FlatSegmentedProps {
    options: Vec<String>,
    selected: String,
    on_change: EventHandler<String>,
}

/// A full-width shadcn segmented control without ToggleGroup's fixed shadow.
#[component]
fn FlatSegmented(props: FlatSegmentedProps) -> Element {
    let theme = use_theme();
    let count = props.options.len();
    let options = props
        .options
        .into_iter()
        .enumerate()
        .map(|(index, option)| {
            let active = option == props.selected;
            let next = option.clone();
            let on_change = props.on_change;
            rsx! {
                row {
                    key: "{option}",
                    layout_weight: 1.0,
                    if index > 0 {
                        row { width: 1.0, height: 40.0, background_color: theme.colors.border }
                    }
                    button {
                        button_type: "normal",
                        width: "100%",
                        height: 40.0,
                        background_color: if active { theme.colors.muted } else { theme.colors.background },
                        foreground_color: theme.colors.foreground,
                        border_width: 0.0,
                        border_radius: if count == 1 { theme.radii.md } else { 0.0 },
                        onclick: move |_| {
                            let next = next.clone();
                            arkit::queue_ui_loop(move || on_change.call(next));
                        },
                        text {
                            content: option,
                            font_size: typography::XS,
                            font_weight: if active { 600 } else { 500 },
                            font_color: if active { theme.colors.foreground } else { theme.colors.muted_foreground },
                        }
                    }
                }
            }
        })
        .collect::<Vec<_>>();

    rsx! {
        row {
            width: "100%",
            height: 42.0,
            border_width: 1.0,
            border_color: theme.colors.border,
            border_radius: theme.radii.md,
            background_color: theme.colors.background,
            clip: true,
            {options.into_iter()}
        }
    }
}

#[derive(Clone, PartialEq)]
struct FlatSelectOption {
    value: String,
    label: String,
    description: String,
}

#[derive(Props, Clone, PartialEq)]
struct FlatSelectProps {
    options: Vec<FlatSelectOption>,
    selected: String,
    on_change: EventHandler<String>,
}

/// Compact inline select used where ArkUI's native menu cannot preserve the
/// currently selected value across declarative rerenders.
#[component]
fn FlatSelect(props: FlatSelectProps) -> Element {
    let theme = use_theme();
    let mut open = use_signal(|| false);
    let selected = props
        .options
        .iter()
        .find(|option| option.value == props.selected)
        .or_else(|| props.options.first())
        .cloned();
    let selected_label = selected
        .as_ref()
        .map(|option| option.label.clone())
        .unwrap_or_default();
    let selected_description = selected
        .as_ref()
        .map(|option| option.description.clone())
        .unwrap_or_default();
    let options = props.options.clone();

    rsx! {
        column {
            width: "100%",
            button {
                button_type: "normal",
                width: "100%",
                height: 44.0,
                padding_left: spacing::MD,
                padding_right: spacing::MD,
                background_color: theme.colors.background,
                border_width: 1.0,
                border_color: theme.colors.input,
                border_radius: theme.radii.md,
                onclick: move |_| open.set(!open()),
                row {
                    width: "100%",
                    align_items: "center",
                    row {
                        layout_weight: 1.0,
                        clip: true,
                        text {
                            content: selected_label,
                            width: "100%",
                            font_size: typography::SM,
                            font_weight: 600,
                            font_color: theme.colors.foreground,
                            max_lines: 1,
                            text_overflow: "ellipsis",
                        }
                    }
                    {arkit::icon(if open() { "chevron-up" } else { "chevron-down" }, 16.0, theme.colors.muted_foreground)}
                }
            }
            if !selected_description.is_empty() {
                text {
                    content: selected_description,
                    margin_top: spacing::XXS,
                    margin_left: 2.0,
                    font_size: typography::XS,
                    line_height: 16.0,
                    font_color: theme.colors.muted_foreground,
                }
            }
            if open() {
                column {
                    width: "100%",
                    margin_top: spacing::XXS,
                    border_width: 1.0,
                    border_color: theme.colors.border,
                    border_radius: theme.radii.md,
                    background_color: theme.colors.popover,
                    clip: true,
                    for option in options {
                        {
                            let active = option.value == props.selected;
                            let value = option.value.clone();
                            let on_change = props.on_change;
                            rsx! {
                                button {
                                    key: "{option.value}",
                                    button_type: "normal",
                                    width: "100%",
                                    height: 48.0,
                                    padding_left: spacing::MD,
                                    padding_right: spacing::MD,
                                    background_color: if active { theme.colors.accent } else { theme.colors.popover },
                                    border_width: 0.0,
                                    border_radius: 0.0,
                                    onclick: move |_| {
                                        open.set(false);
                                        let value = value.clone();
                                        arkit::queue_ui_loop(move || on_change.call(value));
                                    },
                                    row {
                                        width: "100%",
                                        align_items: "center",
                                        column {
                                            layout_weight: 1.0,
                                            align_items: "start",
                                            text {
                                                content: option.label,
                                                font_size: typography::SM,
                                                font_weight: if active { 600 } else { 500 },
                                                font_color: theme.colors.foreground,
                                                max_lines: 1,
                                            }
                                            text {
                                                content: option.description,
                                                margin_top: 2.0,
                                                font_size: typography::XS,
                                                font_color: theme.colors.muted_foreground,
                                                max_lines: 1,
                                                text_overflow: "ellipsis",
                                            }
                                        }
                                        if active {
                                            {arkit::icon("check", 15.0, theme.colors.foreground)}
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct FlatDialogProps {
    open: bool,
    /// Bump when dialog body should refresh while `open` stays true
    /// (loading spinners, validation errors, controlled field values, etc.).
    /// Overlay content is snapshotted on show; without a key change the panel
    /// freezes and pending/loading UI never appears.
    #[props(default)]
    content_key: u64,
    on_close: EventHandler<()>,
    children: Element,
}

/// Arkit modal behavior and shadcn dialog composition with a strictly flat panel.
#[component]
fn FlatDialog(props: FlatDialogProps) -> Element {
    let theme = use_theme();
    let close = props.on_close;
    let panel_close = close;
    let panel = rsx! {
        stack {
            width: "100%",
            max_width_constraint: 512.0,
            alignment: "top-start",
            border_radius: theme.radii.lg,
            border_width: 1.0,
            border_color: theme.colors.border,
            background_color: theme.colors.background,
            clip: true,
            column {
                width: "100%",
                padding: spacing::XXL,
                {props.children}
            }
            row {
                width: "100%",
                justify_content: "end",
                padding_top: 14.0,
                padding_right: 14.0,
                hit_test_behavior: "transparent",
                button {
                    button_type: "normal",
                    width: 28.0,
                    height: 28.0,
                    padding: 0.0,
                    background_color: 0x00000000,
                    border_width: 0.0,
                    border_radius: theme.radii.sm,
                    clip: true,
                    focusable: false,
                    focus_on_touch: false,
                    alignment: "center",
                    onclick: move |_| panel_close.call(()),
                    {arkit::icon("x", 18.0, theme.colors.muted_foreground)}
                }
            }
        }
    };
    use_flat_dialog_overlay(props.open, props.content_key, panel, close);
    rsx! {}
}

fn use_flat_dialog_overlay(
    open: bool,
    content_key: u64,
    panel: Element,
    on_dismiss: EventHandler<()>,
) {
    let overlay = arkit::use_overlay();
    let last_open = use_hook(|| Rc::new(Cell::new(false)));
    let spec = arkit::hooks::ModalOverlaySpec {
        open,
        presentation: arkit::hooks::ModalPresentation::CenteredDialog,
        dismiss_on_backdrop: true,
        backdrop_color: 0x80000000,
        viewport_inset: 8.0,
    };

    let effect_overlay = overlay.clone();
    let effect_last_open = last_open.clone();
    // Re-publish whenever open flips *or* content_key changes while open so
    // loading/error/field updates reach the overlay-hosted panel.
    use_effect(use_reactive((&open, &content_key), move |(open, _key)| {
        if open {
            let panel = panel.clone();
            effect_overlay.show_modal_with_dismiss(
                spec,
                move || panel.clone(),
                move || on_dismiss.call(()),
            );
            effect_last_open.set(true);
        } else if effect_last_open.get() {
            effect_overlay.dismiss();
            effect_last_open.set(false);
        }
    }));

    let cleanup_overlay = overlay.clone();
    let cleanup_last_open = last_open.clone();
    use_drop(move || {
        if cleanup_last_open.get() {
            cleanup_overlay.dismiss();
        }
    });
}

fn dialog_content_key(parts: &[&str]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for part in parts {
        part.hash(&mut hasher);
    }
    hasher.finish()
}

#[component]
pub(crate) fn App() -> Element {
    let notifications = use_notification_center();
    let state = use_signal(|| State::new(notifications));
    let _state = use_context_provider(move || state);
    let theme = if state.read().theme_dark() {
        Theme::dark(ThemePreset::Zinc)
    } else {
        Theme::light(ThemePreset::Zinc)
    };
    let mut applied_color_mode = use_signal(|| None::<i32>);

    use_effect(move || {
        let color_mode = state.read().theme_preference().platform_color_mode();
        if *applied_color_mode.peek() != Some(color_mode) {
            let _ = platform_callbacks::set_color_mode(color_mode);
            applied_color_mode.set(Some(color_mode));
        }
    });

    use_effect(move || {
        run_command(
            state,
            Command::batch([
                Command::perform(bootstrap_active_profile(), Action::SnapshotLoaded),
                Command::perform(delayed_snapshot(), Action::TickSnapshot),
            ]),
        );
    });

    rsx! {
        ThemeProvider {
            theme,
            Router::<Route> {}
            NotificationHost { center: notifications }
        }
    }
}

#[component]
fn AppShell() -> Element {
    let state = use_context::<Signal<State>>();
    let current = state.read().clone();
    let route = use_route::<Route>();
    let navigator = use_navigator();
    let _back_handler = use_back_handler();
    let nav_items = Route::bottom_routes()
        .iter()
        .map(|route| BottomNavigationItem::new(route.title(current.locale), route.icon()))
        .collect::<Vec<_>>();

    rsx! {
        stack {
            width: "100%",
            height: "100%",
            background_color: bg(),
            alignment: "top-start",
            column {
                width: "100%",
                height: "100%",
                column {
                    layout_weight: 1.0,
                    width: "100%",
                    AnimatedOutlet::<Route> {}
                }
            if route.parent().is_none() {
                BottomNavigation {
                        items: nav_items,
                        selected: Some(route.bottom_index()),
                        on_select: move |index| {
                            if let Some(route) = Route::bottom_routes().get(index).cloned() {
                                navigator.replace(route);
                            }
                        }
                    }
                }
            }
            if matches!(route, Route::Dashboard {}) {
                {vpn_floating_action(state, &current)}
            }
            if current.yaml_editor_open {
                {yaml_editor_dialog(state, &current)}
            }
        }
    }
}

fn vpn_floating_action(state: Signal<State>, current: &State) -> Element {
    let theme = use_theme();
    let pending = current.vpn_command_pending;
    let starting = pending == Some(VpnCommandAction::Start)
        || matches!(current.snapshot.vpn_lifecycle, VpnLifecycle::Starting);
    let stopping = pending == Some(VpnCommandAction::Stop);
    let active = current.snapshot.vpn_running && !stopping;
    let disabled =
        pending.is_some() || matches!(current.snapshot.vpn_lifecycle, VpnLifecycle::Starting);
    let icon = if active { "square" } else { "power" };
    let background = if disabled {
        theme.colors.muted
    } else if active {
        theme.colors.destructive
    } else {
        theme.colors.primary
    };
    let foreground = if disabled {
        theme.colors.muted_foreground
    } else if active {
        theme.colors.destructive_foreground
    } else {
        theme.colors.primary_foreground
    };
    rsx! {
        column {
            width: "100%",
            height: "100%",
            padding_right: spacing::XXL,
            padding_bottom: 92.0,
            align_items: "end",
            justify_content: "end",
            hit_test_behavior: "transparent",
            button {
                button_type: "normal",
                width: 56.0,
                height: 56.0,
                background_color: background,
                border_width: 0.0,
                border_radius: theme.radii.full,
                enabled: !disabled,
                opacity: if disabled { 0.6 } else { 1.0 },
                onclick: move |_| dispatch(state, Action::StartStopVpn),
                row {
                    width: "100%",
                    height: "100%",
                    align_items: "center",
                    justify_content: "center",
                    if starting || stopping {
                        Spinner { size: 22.0, color: Some(foreground) }
                    } else {
                        {arkit::icon(icon, 22.0, foreground)}
                    }
                }
            }
        }
    }
}

fn dispatch(mut state: Signal<State>, action: Action) {
    let command = {
        let mut current = state.write();
        reduce(&mut current, action)
    };
    run_command(state, command);
}

fn run_command(state: Signal<State>, command: Command<Action>) {
    let runtime = arkit::tokio_handle();
    for future in command.into_futures() {
        let task = runtime.spawn(future);
        arkit::dioxus_core::spawn_forever(async move {
            if let Ok(action) = task.await {
                dispatch(state, action);
            }
        });
    }
}

fn scaffold(state: Signal<State>, page: Route, actions: Element, body: Element) -> Element {
    scaffold_layout(state, page, actions, body, true, false)
}

fn fixed_scaffold(state: Signal<State>, page: Route, actions: Element, body: Element) -> Element {
    scaffold_layout(state, page, actions, body, false, false)
}

fn fixed_scaffold_flush_bottom(
    state: Signal<State>,
    page: Route,
    actions: Element,
    body: Element,
) -> Element {
    scaffold_layout(state, page, actions, body, false, true)
}

fn scaffold_layout(
    state: Signal<State>,
    page: Route,
    actions: Element,
    body: Element,
    scrollable: bool,
    flush_fixed_bottom: bool,
) -> Element {
    let current = state.read().clone();
    let parent = page.parent();
    use_parent_back_handler(parent.clone());
    let scroll_key = format!("page-scroll-{page:?}");
    let navigator = use_navigator();
    let theme = use_theme();
    rsx! {
        column {
            layout_weight: 1.0,
            width: "100%",
            background_color: theme.colors.background,
            row {
                height: 56.0,
                width: "100%",
                padding_left: spacing::MD,
                padding_right: spacing::MD,
                align_items: "center",
                background_color: theme.colors.card,
                row {
                    align_items: "center",
                    if let Some(parent) = parent {
                        FlatButton {
                            variant: FlatButtonVariant::Ghost,
                            size: ButtonSize::Icon,
                            onclick: move |_| {
                                if navigator.can_go_back() {
                                    navigator.go_back();
                                } else {
                                    navigator.push(parent.clone());
                                }
                            },
                            {arkit::icon("arrow-left", 18.0, theme.colors.foreground)}
                        }
                        row { width: spacing::XXS }
                    }
                    text {
                        content: page.title(current.locale),
                        font_size: typography::XL,
                        line_height: 28.0,
                        font_weight: 600,
                        font_color: theme.colors.foreground,
                        text_letter_spacing: -0.3,
                    }
                }
                row { layout_weight: 1.0 }
                {actions}
            }
            Separator {}
            column {
                layout_weight: 1.0,
                width: "100%",
                if scrollable {
                    scroll {
                        key: "{scroll_key}",
                        width: "100%",
                        height: "100%",
                        alignment: "top-start",
                        background_color: theme.colors.background,
                        scroll_bar: "off",
                        column {
                            width: "100%",
                            padding: spacing::LG,
                            align_items: "start",
                            justify_content: "start",
                            {body}
                            row { height: spacing::MD }
                        }
                    }
                } else {
                    column {
                        layout_weight: 1.0,
                        width: "100%",
                        padding_top: spacing::LG,
                        padding_right: spacing::LG,
                        padding_bottom: if flush_fixed_bottom { 0.0 } else { spacing::LG },
                        padding_left: spacing::LG,
                        align_items: "start",
                        justify_content: "start",
                        {body}
                    }
                }
            }
        }
    }
}

/// Secondary pages always consume the platform back gesture and return to
/// their declared parent. This does not depend on router history, so a cold
/// restore or a bottom-tab replacement cannot accidentally close the app.
fn use_parent_back_handler(parent: Option<Route>) {
    let navigator = use_navigator();
    let scoped_handler = arkit::dioxus_hooks::use_callback(move |()| {
        let Some(parent) = parent.clone() else {
            return false;
        };
        if navigator.can_go_back() {
            navigator.go_back();
        } else {
            navigator.push(parent);
        }
        true
    });
    let handler: Rc<dyn Fn() -> bool> = Rc::new(move || scoped_handler.call(()));
    let registered_handler = handler.clone();
    let _registration =
        use_hook(|| Rc::new(arkit::register_back_press_handler(registered_handler)));
}

fn dashboard_page(state: Signal<State>) -> Element {
    let current = state.read().clone();
    let snapshot = current.snapshot;
    let s = strings(current.locale);
    let navigator = use_navigator();
    let quick_item_order = use_hook(|| Rc::new(RefCell::new(Vec::<(String, String)>::new())));
    let vpn_starting = current.vpn_command_pending == Some(VpnCommandAction::Start)
        || matches!(snapshot.vpn_lifecycle, VpnLifecycle::Starting);
    let vpn_stopping = current.vpn_command_pending == Some(VpnCommandAction::Stop);
    let transitioning = vpn_starting || vpn_stopping;
    let connected = snapshot.vpn_running && !transitioning;
    let status_label = if vpn_starting {
        tr(current.locale, "正在连接", "Connecting")
    } else if vpn_stopping {
        tr(current.locale, "正在断开", "Disconnecting")
    } else {
        match snapshot.vpn_lifecycle {
            VpnLifecycle::Stopped => s.dashboard_disconnected,
            VpnLifecycle::EngineLoaded => tr(current.locale, "配置已就绪", "Ready to connect"),
            VpnLifecycle::Starting => s.lifecycle_starting,
            VpnLifecycle::Connected => s.dashboard_connected,
            VpnLifecycle::ProtectFailed => s.lifecycle_protect_failed,
            VpnLifecycle::Failed => tr(current.locale, "VPN 启动失败", "VPN failed"),
        }
    };
    let profile = snapshot
        .profiles
        .iter()
        .find(|profile| snapshot.active_profile.as_deref() == Some(profile.id.as_str()))
        .map(|profile| profile.name.clone())
        .unwrap_or_else(|| s.dashboard_profile_empty.to_owned());
    let status_color = if transitioning {
        subtle()
    } else if matches!(
        snapshot.vpn_lifecycle,
        VpnLifecycle::Failed | VpnLifecycle::ProtectFailed
    ) {
        danger()
    } else if connected {
        success()
    } else {
        subtle()
    };
    let mut quick_items = flatten_proxy_groups(&snapshot.proxy_groups, "");
    if let Some((pending_group, pending_proxy)) = &current.proxy_selection_pending {
        if !pending_proxy.is_empty() {
            for item in &mut quick_items {
                item.selected = item.group == *pending_group && item.name == *pending_proxy;
            }
        }
    }
    let current_node = quick_items
        .iter()
        .find(|item| item.selected)
        .map(|item| item.name.clone())
        .unwrap_or_else(|| tr(current.locale, "未选择", "Unselected").to_owned());
    let quick_count = quick_items.len();
    let quick_group_count = quick_items
        .iter()
        .map(|item| item.group.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let proxy_group_context = match current.locale {
        UiLocale::ZhCn => format!("{quick_count} 个节点 · {quick_group_count} 个分组"),
        UiLocale::En => format!("{quick_count} nodes · {quick_group_count} groups"),
    };
    let quick_palette = VirtualProxyGridPalette {
        surface: surface(),
        selected_surface: muted(),
        foreground: text_color(),
        muted_foreground: subtle(),
        border: line(),
        success: success(),
    };
    // Preserve the first-seen order independently from selection. Selection
    // remains a row-local visual input, so the keyed adapter only reloads the
    // previous and next rows without moving the list or its scroll anchor.
    let quick_list_items =
        stabilize_proxy_items(&mut quick_item_order.borrow_mut(), quick_items.clone());
    let subscriptions_navigator = navigator.clone();
    let all_nodes_navigator = navigator.clone();
    let vpn_ip = if connected {
        snapshot
            .vpn_options
            .addresses
            .iter()
            .find(|address| !address.contains(':'))
            .or_else(|| snapshot.vpn_options.addresses.first())
            .cloned()
            .unwrap_or_else(|| tr(current.locale, "未分配", "Not assigned").to_owned())
    } else {
        tr(current.locale, "未分配", "Not assigned").to_owned()
    };
    let status_icon = if connected {
        "shield-check"
    } else if matches!(
        snapshot.vpn_lifecycle,
        VpnLifecycle::Failed | VpnLifecycle::ProtectFailed
    ) {
        "triangle-alert"
    } else {
        "power"
    };

    let body = rsx! {
        column {
            width: "100%",
            layout_weight: 1.0,
            column {
                width: "100%",
                row {
                    width: "100%",
                    height: 52.0,
                    align_items: "center",
                    row {
                        width: 42.0,
                        height: 42.0,
                        align_items: "center",
                        justify_content: "center",
                        background_color: muted(),
                        border_radius: 10.0,
                        if transitioning {
                            Spinner { size: 20.0, color: Some(status_color) }
                        } else {
                            {arkit::icon(status_icon, 20.0, status_color)}
                        }
                    }
                    column {
                        layout_weight: 1.0,
                        margin_left: 12.0,
                        align_items: "start",
                        text {
                            content: status_label,
                            font_size: 19.0,
                            line_height: 24.0,
                            font_weight: 700,
                            font_color: status_color,
                        }
                        text {
                            width: "100%",
                            content: profile,
                            margin_top: 1.0,
                            font_size: 11.0,
                            line_height: 16.0,
                            font_color: subtle(),
                            max_lines: 1,
                            text_overflow: "ellipsis",
                        }
                    }
                }
                row { height: 14.0 }
                {mode_picker(state, snapshot.mode, current.locale)}
                row { height: 14.0 }
                row {
                    width: "100%",
                    height: 40.0,
                    padding_left: 4.0,
                    padding_right: 4.0,
                    align_items: "center",
                    row {
                        layout_weight: 1.0,
                        align_items: "center",
                        clip: true,
                        {arkit::icon("git-branch", 15.0, text_color())}
                        column {
                            layout_weight: 1.0,
                            margin_left: 8.0,
                            align_items: "start",
                            text { content: tr(current.locale, "当前节点", "Current node"), font_size: 10.0, line_height: 14.0, font_color: subtle() }
                            text { width: "100%", content: current_node, font_size: 13.0, line_height: 18.0, font_weight: 650, font_color: text_color(), max_lines: 1, text_overflow: "ellipsis" }
                        }
                    }
                    Separator { vertical_height: Some(30.0) }
                    row {
                        layout_weight: 1.0,
                        padding_left: 18.0,
                        align_items: "center",
                        clip: true,
                        {arkit::icon("network", 15.0, text_color())}
                        column {
                            layout_weight: 1.0,
                            margin_left: 8.0,
                            align_items: "start",
                            text { content: "VPN IP", font_size: 10.0, line_height: 14.0, font_color: subtle() }
                            text { width: "100%", content: vpn_ip, font_size: 13.0, line_height: 18.0, font_weight: 650, font_color: text_color(), max_lines: 1, text_overflow: "ellipsis" }
                        }
                    }
                }
            }
            row { height: 18.0 }
            Separator {}
            row { height: 18.0 }
            row {
                width: "100%",
                align_items: "center",
                column {
                    layout_weight: 1.0,
                    align_items: "start",
                    text { content: tr(current.locale, "快速切换", "Quick switch"), font_size: 17.0, line_height: 22.0, font_weight: 700, font_color: text_color() }
                    text { content: proxy_group_context, margin_top: 1.0, font_size: 10.0, line_height: 14.0, font_color: subtle(), max_lines: 1 }
                }
                if quick_count > 0 {
                    Button {
                        variant: ButtonVariant::Ghost,
                        size: ButtonSize::Sm,
                        shadow: Some(false),
                        onclick: move |_| {
                            all_nodes_navigator.push(Route::Proxies {});
                        },
                        text { content: tr(current.locale, "搜索", "Search"), font_size: 12.0, font_weight: 600, font_color: text_color() }
                        {arkit::icon("chevron-right", 14.0, subtle())}
                    }
                }
            }
            row { height: 6.0 }
            if quick_count == 0 {
                column {
                    layout_weight: 1.0,
                    width: "100%",
                    align_items: "center",
                    justify_content: "center",
                    {arkit::icon("rss", 21.0, subtle())}
                    text { content: tr(current.locale, "尚未选择订阅", "No subscription selected"), margin_top: 9.0, font_size: 14.0, font_weight: 700, font_color: text_color() }
                    text { content: tr(current.locale, "添加并启用订阅后即可选择节点", "Add and activate a subscription to choose nodes"), margin_top: 3.0, font_size: 11.0, line_height: 16.0, font_color: subtle(), text_align: "center" }
                    row { height: 10.0 }
                    Button {
                        variant: ButtonVariant::Default,
                        size: ButtonSize::Sm,
                        shadow: Some(false),
                        onclick: move |_| {
                            subscriptions_navigator.push(Route::Profiles {});
                        },
                        {arkit::icon("plus", 14.0, primary_text())}
                        text { content: tr(current.locale, "添加订阅", "Add subscription"), margin_left: 6.0, font_size: 12.0, font_weight: 600, font_color: primary_text() }
                    }
                }
            } else {
                column {
                    layout_weight: 1.0,
                    width: "100%",
                    clip: true,
                    VirtualQuickProxyList {
                        key: "dashboard-quick-proxy-list",
                        items: quick_list_items,
                        locale: current.locale,
                        palette: quick_palette,
                        on_select: move |(group, proxy): (String, String)| {
                            dispatch(state, Action::SelectProxy { group, proxy });
                        },
                    }
                }
            }
        }
    };
    fixed_scaffold_flush_bottom(state, Route::Dashboard {}, rsx! {}, body)
}

fn mode_picker(state: Signal<State>, selected: RuntimeMode, locale: UiLocale) -> Element {
    let rule = tr(locale, "规则", "Rule").to_owned();
    let global = tr(locale, "全局", "Global").to_owned();
    let direct = tr(locale, "直连", "Direct").to_owned();
    let selected_label = match selected {
        RuntimeMode::Rule => rule.clone(),
        RuntimeMode::Global => global.clone(),
        RuntimeMode::Direct => direct.clone(),
    };
    rsx! {
        FlatSegmented {
            options: vec![rule, global.clone(), direct.clone()],
            selected: selected_label,
            on_change: move |value: String| {
                let mode = if value == global {
                    RuntimeMode::Global
                } else if value == direct {
                    RuntimeMode::Direct
                } else {
                    RuntimeMode::Rule
                };
                dispatch(state, Action::SetMode(mode));
            },
        }
    }
}

fn proxies_page(state: Signal<State>) -> Element {
    let mut query = use_signal(String::new);
    let mut layout_mode = use_signal(ProxyLayoutMode::default);
    let current = state.read().clone();
    let query_value = query();
    let current_layout = layout_mode();
    let mut items = flatten_proxy_groups(&current.snapshot.proxy_groups, &query_value);
    if let Some((pending_group, pending_proxy)) = &current.proxy_selection_pending {
        if !pending_proxy.is_empty() {
            for item in &mut items {
                item.selected = item.group == *pending_group && item.name == *pending_proxy;
            }
        }
    }
    let matching_group_count = items
        .iter()
        .map(|item| item.group.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let result_summary = match current.locale {
        UiLocale::ZhCn => format!("{} 个节点 · {} 个分组", items.len(), matching_group_count),
        UiLocale::En => format!("{} nodes · {} groups", items.len(), matching_group_count),
    };
    let palette = VirtualProxyGridPalette {
        surface: surface(),
        selected_surface: muted(),
        foreground: text_color(),
        muted_foreground: subtle(),
        border: line(),
        success: success(),
    };
    let empty = items.is_empty();
    let body = rsx! {
        column {
            width: "100%",
            layout_weight: 1.0,
            Input {
                value: Some(query_value),
                placeholder: Some(strings(current.locale).proxies_search_placeholder.to_owned()),
                width: Some("100%".into()),
                on_change: move |value| query.set(value),
            }
            row {
                width: "100%",
                height: 34.0,
                align_items: "center",
                text {
                    content: result_summary,
                    font_size: 11.0,
                    font_color: subtle(),
                }
            }
            if empty {
                column {
                    layout_weight: 1.0,
                    width: "100%",
                    justify_content: "center",
                    {empty_state("git-branch", strings(current.locale).proxies_empty_title, strings(current.locale).proxies_empty_subtitle)}
                }
            } else {
                column {
                    layout_weight: 1.0,
                    width: "100%",
                    if current_layout == ProxyLayoutMode::Grid {
                        VirtualProxyGrid {
                            items,
                            locale: current.locale,
                            palette,
                            selection_pending: current.proxy_selection_pending.is_some(),
                            on_select: move |(group, proxy): (String, String)| {
                                if proxy.is_empty() {
                                    dispatch(state, Action::UnfixProxy { group });
                                } else {
                                    dispatch(state, Action::SelectProxy { group, proxy });
                                }
                            },
                        }
                    } else {
                        VirtualProxyList {
                            items,
                            locale: current.locale,
                            palette,
                            selection_pending: current.proxy_selection_pending.is_some(),
                            on_select: move |(group, proxy): (String, String)| {
                                if proxy.is_empty() {
                                    dispatch(state, Action::UnfixProxy { group });
                                } else {
                                    dispatch(state, Action::SelectProxy { group, proxy });
                                }
                            },
                        }
                    }
                }
            }
        }
    };
    let proxy_delay_loading = current.proxy_delay_loading;
    let actions = rsx! {
        row {
            FlatButton {
                variant: FlatButtonVariant::Outline,
                size: ButtonSize::Icon,
                onclick: move |_| layout_mode.set(current_layout.toggled()),
                {arkit::icon(current_layout.toggle_icon(), 17.0, text_color())}
            }
            column { width: 8.0 }
            FlatButton {
                variant: FlatButtonVariant::Outline,
                size: ButtonSize::Icon,
                disabled: Some(proxy_delay_loading),
                onclick: move |_| {
                    if !proxy_delay_loading {
                        dispatch(state, Action::TestAllProxyDelays);
                    }
                },
                if proxy_delay_loading {
                    Spinner { size: 16.0, color: Some(text_color()) }
                } else {
                    {arkit::icon("gauge", 17.0, text_color())}
                }
            }
        }
    };
    fixed_scaffold(state, Route::Proxies {}, actions, body)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum ProxyLayoutMode {
    #[default]
    Grid,
    List,
    Compact,
}

impl ProxyLayoutMode {
    fn toggled(self) -> Self {
        match self {
            Self::Grid => Self::List,
            Self::List => Self::Grid,
            Self::Compact => Self::List,
        }
    }

    fn toggle_icon(self) -> &'static str {
        match self {
            Self::Grid => "list",
            Self::List => "layout-grid",
            Self::Compact => "list",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct VirtualProxyGridPalette {
    surface: u32,
    selected_surface: u32,
    foreground: u32,
    muted_foreground: u32,
    border: u32,
    success: u32,
}

#[derive(Clone)]
struct VirtualProxyGridRenderState {
    items: Vec<ProxyGridItem>,
    locale: UiLocale,
    palette: VirtualProxyGridPalette,
    layout: ProxyLayoutMode,
    selection_pending: bool,
    on_select: EventHandler<(String, String)>,
}

fn virtual_proxy_item_keys(
    items: &[ProxyGridItem],
    locale: UiLocale,
    palette: VirtualProxyGridPalette,
    layout: ProxyLayoutMode,
    selection_pending: bool,
) -> Vec<u64> {
    items
        .iter()
        .map(|item| {
            let mut hasher = DefaultHasher::new();
            item.hash(&mut hasher);
            locale.language_tag().hash(&mut hasher);
            palette.hash(&mut hasher);
            layout.hash(&mut hasher);
            // Pending text is rendered only on the selected card. Keeping the
            // flag item-local means a selection updates the previous and next
            // cards instead of rebuilding the entire visible collection.
            (selection_pending && item.selected).hash(&mut hasher);
            hasher.finish()
        })
        .collect()
}

#[component]
fn VirtualProxyGrid(
    items: Vec<ProxyGridItem>,
    locale: UiLocale,
    palette: VirtualProxyGridPalette,
    selection_pending: bool,
    on_select: EventHandler<(String, String)>,
) -> Element {
    let item_keys = virtual_proxy_item_keys(
        &items,
        locale,
        palette,
        ProxyLayoutMode::Grid,
        selection_pending,
    );
    let render_state = use_hook(|| {
        Rc::new(RefCell::new(VirtualProxyGridRenderState {
            items: items.clone(),
            locale,
            palette,
            layout: ProxyLayoutMode::Grid,
            selection_pending,
            on_select,
        }))
    });
    *render_state.borrow_mut() = VirtualProxyGridRenderState {
        items,
        locale,
        palette,
        layout: ProxyLayoutMode::Grid,
        selection_pending,
        on_select,
    };
    let render_state_for_adapter = render_state.clone();
    let handle = use_virtual_node_adapter_items_keyed(VirtualKind::Grid, item_keys, move |index| {
        let state = render_state_for_adapter.borrow();
        render_virtual_proxy_card(
            &state.items[index as usize],
            state.locale,
            state.palette,
            state.layout,
            state.selection_pending,
            render_state_for_adapter.clone(),
        )
    });
    let attach_handle = handle.clone();
    use_layout_frame_node(move |host_node, _frame| {
        let _ = attach_handle.attach(&host_node);
    });

    rsx! {
        grid {
            width: "100%",
            height: "100%",
            grid_column_template: "1fr 1fr",
            grid_column_gap: 10.0,
            grid_row_gap: 10.0,
            grid_cached_count: 12_i32,
        }
    }
}

#[component]
fn VirtualProxyList(
    items: Vec<ProxyGridItem>,
    locale: UiLocale,
    palette: VirtualProxyGridPalette,
    selection_pending: bool,
    on_select: EventHandler<(String, String)>,
) -> Element {
    let item_keys = virtual_proxy_item_keys(
        &items,
        locale,
        palette,
        ProxyLayoutMode::List,
        selection_pending,
    );
    let render_state = use_hook(|| {
        Rc::new(RefCell::new(VirtualProxyGridRenderState {
            items: items.clone(),
            locale,
            palette,
            layout: ProxyLayoutMode::List,
            selection_pending,
            on_select,
        }))
    });
    *render_state.borrow_mut() = VirtualProxyGridRenderState {
        items,
        locale,
        palette,
        layout: ProxyLayoutMode::List,
        selection_pending,
        on_select,
    };
    let render_state_for_adapter = render_state.clone();
    let handle = use_virtual_node_adapter_items_keyed(VirtualKind::List, item_keys, move |index| {
        let state = render_state_for_adapter.borrow();
        render_virtual_proxy_card(
            &state.items[index as usize],
            state.locale,
            state.palette,
            state.layout,
            state.selection_pending,
            render_state_for_adapter.clone(),
        )
    });
    let attach_handle = handle.clone();
    use_layout_frame_node(move |host_node, _frame| {
        let _ = attach_handle.attach(&host_node);
    });

    rsx! {
        list {
            width: "100%",
            height: "100%",
            list_cached_count: 16_i32,
        }
    }
}

#[component]
fn VirtualQuickProxyList(
    items: Vec<ProxyGridItem>,
    locale: UiLocale,
    palette: VirtualProxyGridPalette,
    on_select: EventHandler<(String, String)>,
) -> Element {
    let item_keys = virtual_quick_proxy_item_keys(&items, locale, palette);
    let render_state = use_hook(|| {
        Rc::new(RefCell::new(VirtualProxyGridRenderState {
            items: items.clone(),
            locale,
            palette,
            layout: ProxyLayoutMode::Compact,
            selection_pending: false,
            on_select,
        }))
    });
    *render_state.borrow_mut() = VirtualProxyGridRenderState {
        items,
        locale,
        palette,
        layout: ProxyLayoutMode::Compact,
        selection_pending: false,
        on_select,
    };
    let render_state_for_adapter = render_state.clone();
    let handle = use_virtual_node_adapter_items_keyed(VirtualKind::List, item_keys, move |index| {
        let state = render_state_for_adapter.borrow();
        render_virtual_proxy_card(
            &state.items[index as usize],
            state.locale,
            state.palette,
            state.layout,
            state.selection_pending,
            render_state_for_adapter.clone(),
        )
    });
    let attach_handle = handle.clone();
    use_layout_frame_node(move |host_node, _frame| {
        let _ = attach_handle.attach(&host_node);
    });

    rsx! {
        list {
            width: "100%",
            height: "100%",
            list_cached_count: 20_i32,
        }
    }
}

fn virtual_quick_proxy_item_keys(
    items: &[ProxyGridItem],
    locale: UiLocale,
    palette: VirtualProxyGridPalette,
) -> Vec<u64> {
    items
        .iter()
        .map(|item| {
            let mut hasher = DefaultHasher::new();
            item.group.hash(&mut hasher);
            item.group_type.hash(&mut hasher);
            item.name.hash(&mut hasher);
            item.proxy_type.hash(&mut hasher);
            item.delay_ms.hash(&mut hasher);
            item.selected.hash(&mut hasher);
            item.automatic.hash(&mut hasher);
            item.pinned.hash(&mut hasher);
            locale.language_tag().hash(&mut hasher);
            palette.hash(&mut hasher);
            hasher.finish()
        })
        .collect()
}

fn render_virtual_proxy_card(
    item: &ProxyGridItem,
    locale: UiLocale,
    palette: VirtualProxyGridPalette,
    layout: ProxyLayoutMode,
    selection_pending: bool,
    interaction_state: Rc<RefCell<VirtualProxyGridRenderState>>,
) -> arkit::ohos_arkui_binding::common::error::ArkUIResult<ArkUINode> {
    if layout == ProxyLayoutMode::Compact {
        return render_virtual_quick_proxy_row(
            item,
            locale,
            palette,
            selection_pending,
            interaction_state,
        );
    }
    let selected_label = tr(locale, "当前", "Current");
    let delay = item
        .delay_ms
        .map(|value| format!("{value} ms"))
        .unwrap_or_else(|| strings(locale).proxies_untested.to_owned());
    let (height, padding, margin) = match layout {
        ProxyLayoutMode::Grid => (92.0, 12.0, [0.0; 4]),
        ProxyLayoutMode::List => (82.0, 11.0, [0.0, 0.0, 8.0, 0.0]),
        ProxyLayoutMode::Compact => unreachable!("compact rows use their dedicated renderer"),
    };
    let emphasized = item.selected || item.pinned;
    let title = if item.selected {
        format!("✓ {}", item.name)
    } else if item.pinned {
        format!("◆ {}", item.name)
    } else {
        item.name.clone()
    };
    let metadata = format!("{} · {}", item.group, item.proxy_type.to_ascii_uppercase(),);
    let status = if item.selected && selection_pending {
        tr(locale, "切换中…", "Switching…").to_owned()
    } else if item.pinned {
        format!(
            "{} · {} · {delay}",
            tr(locale, "已固定，点击恢复自动", "Pinned, tap for auto"),
            item.group_type
        )
    } else if item.automatic && item.selected {
        format!(
            "{} · {selected_label} · {delay}",
            tr(locale, "自动", "Auto")
        )
    } else if item.selected {
        format!("{selected_label} · {} · {delay}", item.group_type)
    } else {
        format!("{} · {delay}", item.group_type)
    };

    let title_node = virtual_proxy_text(
        title,
        13.0,
        if emphasized { 6 } else { 4 },
        if emphasized {
            palette.success
        } else {
            palette.foreground
        },
        18.0,
    )?;
    let metadata_node = virtual_proxy_text(metadata, 10.0, 3, palette.muted_foreground, 15.0)?;
    let status_node = virtual_proxy_text(
        status,
        10.0,
        if emphasized { 5 } else { 3 },
        if item.delay_ms.is_some() || emphasized {
            palette.success
        } else {
            palette.muted_foreground
        },
        15.0,
    )?;

    let accessibility_text = format!("{}，{}，{}", item.name, item.group, delay);
    let node = NodeBuilder::new("column")?
        .percent_width(1.0)?
        .height(height)?
        .background_color(format!(
            "#{:08x}",
            if emphasized {
                palette.selected_surface
            } else {
                palette.surface
            }
        ))?
        .padding([padding; 4])?
        .margin(margin)?
        .attr(ArkUINodeAttributeType::BorderWidth, vec![1.0; 4])?
        .attr(ArkUINodeAttributeType::BorderColor, palette.border)?
        .attr(ArkUINodeAttributeType::BorderRadius, vec![10.0; 4])?
        .attr(ArkUINodeAttributeType::Clip, true)?
        .attr(ArkUINodeAttributeType::ColumnAlignItems, 0_i32)?
        .attr(ArkUINodeAttributeType::ColumnJustifyContent, 2_i32)?
        .attr(
            ArkUINodeAttributeType::AccessibilityText,
            accessibility_text,
        )?
        .child(title_node)?
        .child(metadata_node)?
        .child(status_node)?;

    let group = item.group.clone();
    let proxy = item.name.clone();
    let unfix = item.pinned && layout != ProxyLayoutMode::Compact;
    Ok(node
        .on_click(move || {
            // Virtual rows outlive an individual Dioxus render. Resolve the
            // current handler at click time instead of retaining a stale
            // listener from the render that originally created this node.
            let state = interaction_state.borrow();
            if state.selection_pending {
                return;
            }
            let proxy = if unfix { String::new() } else { proxy.clone() };
            state.on_select.call((group.clone(), proxy));
        })?
        .build())
}

fn render_virtual_quick_proxy_row(
    item: &ProxyGridItem,
    locale: UiLocale,
    palette: VirtualProxyGridPalette,
    _selection_pending: bool,
    interaction_state: Rc<RefCell<VirtualProxyGridRenderState>>,
) -> arkit::ohos_arkui_binding::common::error::ArkUIResult<ArkUINode> {
    let delay = item
        .delay_ms
        .map(|value| format!("{value} ms"))
        .unwrap_or_else(|| strings(locale).proxies_untested.to_owned());
    let detail = if item.pinned {
        format!(
            "{} · {} · {} · {delay}",
            item.group,
            tr(locale, "已固定", "Pinned"),
            item.proxy_type.to_ascii_uppercase()
        )
    } else {
        format!(
            "{} · {} · {delay}",
            item.group,
            item.proxy_type.to_ascii_uppercase()
        )
    };
    let title_node = virtual_proxy_text(
        item.name.clone(),
        13.0,
        if item.selected { 6 } else { 4 },
        if item.selected {
            palette.success
        } else {
            palette.foreground
        },
        18.0,
    )?;
    let detail_node = virtual_proxy_text(detail, 10.0, 3, palette.muted_foreground, 15.0)?;
    let accessibility_text = format!("{}，{}，{}", item.name, item.group, delay);
    let selection_marker = NodeBuilder::new("column")?
        .width(3.0)?
        .height(28.0)?
        .margin([0.0, 8.0, 0.0, 0.0])?
        .background_color(format!(
            "#{:08x}",
            if item.selected {
                palette.success
            } else {
                0x00000000
            }
        ))?
        .attr(ArkUINodeAttributeType::BorderRadius, vec![2.0; 4])?
        .build();
    let content = NodeBuilder::new("column")?
        .attr(ArkUINodeAttributeType::LayoutWeight, 1.0_f32)?
        .attr(ArkUINodeAttributeType::ColumnAlignItems, 0_i32)?
        .attr(ArkUINodeAttributeType::ColumnJustifyContent, 2_i32)?
        .child(title_node)?
        .child(detail_node)?
        .build();
    let node = NodeBuilder::new("row")?
        .percent_width(1.0)?
        .height(56.0)?
        .background_color(format!("#{:08x}", palette.surface))?
        .padding([7.0, 10.0, 7.0, 7.0])?
        .attr(
            ArkUINodeAttributeType::BorderWidth,
            vec![0.0, 0.0, 1.0, 0.0],
        )?
        .attr(ArkUINodeAttributeType::BorderColor, palette.border)?
        .attr(ArkUINodeAttributeType::Clip, true)?
        .attr(ArkUINodeAttributeType::RowAlignItems, 1_i32)?
        .attr(
            ArkUINodeAttributeType::AccessibilityText,
            accessibility_text,
        )?
        .child(selection_marker)?
        .child(content)?;

    let group = item.group.clone();
    let proxy = item.name.clone();
    Ok(node
        .on_click(move || {
            let state = interaction_state.borrow();
            if !state.selection_pending {
                state.on_select.call((group.clone(), proxy.clone()));
            }
        })?
        .build())
}

fn virtual_proxy_text(
    content: String,
    size: f32,
    weight: i32,
    color: u32,
    line_height: f32,
) -> arkit::ohos_arkui_binding::common::error::ArkUIResult<ArkUINode> {
    Ok(NodeBuilder::new("text")?
        .percent_width(1.0)?
        .font_size(size)?
        .font_color(format!("#{color:08x}"))?
        .text_content(content)?
        .attr(ArkUINodeAttributeType::FontWeight, weight)?
        .attr(ArkUINodeAttributeType::TextLineHeight, line_height)?
        .attr(ArkUINodeAttributeType::TextMaxLines, 1_i32)?
        .attr(ArkUINodeAttributeType::TextOverflow, 2_i32)?
        .build())
}

fn profiles_page(state: Signal<State>) -> Element {
    let mut query = use_signal(String::new);
    let mut import_open = use_signal(|| false);
    let mut import_url = use_signal(String::new);
    let mut import_name = use_signal(String::new);
    let mut import_submitted = use_signal(|| false);
    let mut action_profile_id = use_signal(|| None::<String>);
    let edit_profile_id = use_signal(|| None::<String>);
    let edit_name = use_signal(String::new);
    let edit_url = use_signal(String::new);
    let delete_profile_id = use_signal(|| None::<String>);
    let current = state.read().clone();

    use_effect(move || {
        let (succeeded, loading) = {
            let feedback = state.read();
            (
                feedback.profile_import_succeeded,
                feedback.profile_import_loading,
            )
        };
        if import_submitted() && succeeded {
            import_open.set(false);
            import_url.set(String::new());
            import_name.set(String::new());
            import_submitted.set(false);
            dispatch(state, Action::ResetProfileImportFeedback);
        } else if import_submitted() && !loading && !succeeded {
            // Failure, validation error, or cancelled file picker.
            import_submitted.set(false);
        }
    });

    let query_value = query();
    let profiles = current
        .snapshot
        .profiles
        .iter()
        .filter(|profile| matches_profile_query(profile, &query_value))
        .cloned()
        .map(|profile| {
            let activate_id = profile.id.clone();
            let menu_id = profile.id.clone();
            let source = profile
                .subscription_url
                .clone()
                .unwrap_or_else(|| profile.source.clone());
            let updated = profile
                .last_refresh_at
                .as_deref()
                .or(profile.updated_at.as_deref())
                .and_then(time_format::format_unix_nanos)
                .unwrap_or_else(|| tr(current.locale, "尚未更新", "Never updated").to_owned());
            let usage = profile.subscription_user_info.as_ref().and_then(|info| {
                info.total_bytes.map(|total| {
                    format!(
                        "{} / {}",
                        format_total(info.upload_bytes + info.download_bytes),
                        format_total(total)
                    )
                })
            });
            let active = profile.active;
            rsx! {
                row {
                    key: "{profile.id}",
                    width: "100%",
                    height: 108.0,
                    background_color: surface(),
                    border_width: 1.0,
                    border_color: line(),
                    border_radius: 10.0,
                    clip: true,
                    row {
                        layout_weight: 1.0,
                        button {
                            width: "100%",
                            height: 106.0,
                            padding_left: 16.0,
                            padding_right: 8.0,
                            background_color: surface(),
                            border_width: 0.0,
                            border_radius: 0.0,
                            onclick: move |_| {
                                if !active {
                                    dispatch(state, Action::ActivateProfile(activate_id.clone()));
                                }
                            },
                            row {
                                width: "100%",
                                align_items: "center",
                                column {
                                    width: 24.0,
                                    align_items: "start",
                                    {arkit::icon(if active { "circle-check" } else { "circle" }, 20.0, if active { success() } else { subtle() })}
                                }
                                column {
                                    layout_weight: 1.0,
                                    padding_top: 12.0,
                                    padding_bottom: 12.0,
                                    align_items: "start",
                                    text {
                                        content: truncate_text(&profile.name, 40),
                                        font_size: 15.0,
                                        font_weight: 700,
                                        font_color: text_color(),
                                        max_lines: 1,
                                    }
                                    text {
                                        content: truncate_text(&source.replace(['\n', '\r'], " "), 54),
                                        margin_top: 4.0,
                                        font_size: 11.0,
                                        line_height: 16.0,
                                        font_color: subtle(),
                                        max_lines: 1,
                                    }
                                    if let Some(error) = profile.last_refresh_error.clone() {
                                        text {
                                            width: "100%",
                                            content: compact(&error),
                                            margin_top: 6.0,
                                            font_size: 11.0,
                                            line_height: 16.0,
                                            font_color: danger(),
                                            max_lines: 1,
                                        }
                                    } else {
                                        row {
                                            width: "100%",
                                            margin_top: 6.0,
                                            align_items: "center",
                                            {arkit::icon("clock", 12.0, subtle())}
                                            text { content: updated, margin_left: 5.0, font_size: 11.0, font_color: subtle(), max_lines: 1 }
                                            if let Some(usage) = usage {
                                                row { layout_weight: 1.0 }
                                                text { content: usage, margin_left: 8.0, font_size: 11.0, font_color: subtle(), max_lines: 1 }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    column {
                        width: 48.0,
                        height: 106.0,
                        align_items: "center",
                        justify_content: "center",
                        button {
                            width: 40.0,
                            height: 40.0,
                            padding: 0.0,
                            background_color: 0x00000000,
                            border_width: 0.0,
                            border_radius: 8.0,
                            onclick: move |_| action_profile_id.set(Some(menu_id.clone())),
                            {arkit::icon("ellipsis-vertical", 18.0, subtle())}
                        }
                    }
                }
            }
        })
        .collect::<Vec<_>>();
    let has_profiles = !current.snapshot.profiles.is_empty();
    let empty = profiles.is_empty();
    let action_profile = action_profile_id().and_then(|id| {
        current
            .snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == id)
            .cloned()
    });
    let delete_profile = delete_profile_id().and_then(|id| {
        current
            .snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == id)
            .cloned()
    });
    let body = rsx! {
        column {
            width: "100%",
            if !has_profiles {
                column {
                    width: "100%",
                    height: 360.0,
                    align_items: "center",
                    justify_content: "center",
                    {arkit::icon("rss", 30.0, subtle())}
                    text { content: strings(current.locale).profiles_empty_title, margin_top: 16.0, font_size: 17.0, font_weight: 700, font_color: text_color() }
                    text { content: tr(current.locale, "添加订阅以获取代理节点与规则", "Add a subscription to get proxy nodes and rules"), margin_top: 6.0, font_size: 13.0, line_height: 19.0, font_color: subtle(), text_align: "center" }
                    row { height: 18.0 }
                    FlatButton {
                        variant: FlatButtonVariant::Primary,
                        onclick: move |_| {
                            dispatch(state, Action::ResetProfileImportFeedback);
                            import_open.set(true);
                        },
                        {arkit::icon("plus", 16.0, primary_text())}
                        text { content: tr(current.locale, "添加订阅", "Add subscription"), margin_left: 8.0, font_size: 14.0, font_weight: 600, font_color: primary_text() }
                    }
                }
            } else {
                Input {
                    value: Some(query_value),
                    placeholder: Some(strings(current.locale).profiles_search_placeholder.to_owned()),
                    width: Some("100%".into()),
                    on_change: move |value| query.set(value),
                }
                row { height: 12.0 }
                if empty {
                    {empty_state("search", strings(current.locale).profiles_no_match_title, strings(current.locale).profiles_no_match_subtitle)}
                } else {
                    {spaced(profiles)}
                }
            }
        }
    };
    let actions = rsx! {
        row {
            {icon_action("refresh-cw", Action::RefreshAllProfiles, state)}
            FlatButton {
                variant: FlatButtonVariant::Ghost,
                size: ButtonSize::Icon,
                onclick: move |_| {
                    dispatch(state, Action::ResetProfileImportFeedback);
                    import_open.set(true);
                },
                {arkit::icon("plus", 18.0, text_color())}
            }
        }
    };
    let page = scaffold(state, Route::Profiles {}, actions, body);
    rsx! {
        {page}
        {profile_import_dialog(
            state,
            &current,
            import_open(),
            import_open,
            import_url,
            import_name,
            import_submitted,
        )}
        {profile_action_dialog(
            state,
            current.locale,
            action_profile,
            action_profile_id,
            edit_profile_id,
            edit_name,
            edit_url,
            delete_profile_id,
        )}
        {profile_edit_dialog(
            state,
            current.locale,
            edit_profile_id,
            edit_name,
            edit_url,
        )}
        {profile_delete_dialog(
            state,
            current.locale,
            delete_profile,
            delete_profile_id,
        )}
    }
}

#[allow(clippy::too_many_arguments)]
fn profile_action_dialog(
    state: Signal<State>,
    locale: UiLocale,
    profile: Option<hmeta_model::ProfileSummary>,
    mut action_profile_id: Signal<Option<String>>,
    mut edit_profile_id: Signal<Option<String>>,
    mut edit_name: Signal<String>,
    mut edit_url: Signal<String>,
    mut delete_profile_id: Signal<Option<String>>,
) -> Element {
    let Some(profile) = profile else {
        return rsx! {};
    };
    let activate_id = profile.id.clone();
    let edit_id = profile.id.clone();
    let edit_profile_name = profile.name.clone();
    let edit_profile_url = profile.subscription_url.clone().unwrap_or_default();
    let yaml_id = profile.id.clone();
    let export_id = profile.id.clone();
    let refresh_id = profile.id.clone();
    let restore_id = profile.id.clone();
    let delete_id = profile.id.clone();
    rsx! {
        FlatDialog {
            open: true,
            on_close: move |_| action_profile_id.set(None),
            DialogHeader {
                title: truncate_text(&profile.name, 42),
                description: Some(tr(locale, "配置操作", "Profile actions").to_owned()),
            }
            row { height: 14.0 }
            column {
                width: "100%",
                border_width: 1.0,
                border_color: line(),
                border_radius: 9.0,
                clip: true,
                if !profile.active {
                    button {
                        width: "100%",
                        height: 48.0,
                        padding_left: 14.0,
                        padding_right: 14.0,
                        background_color: surface(),
                        border_width: 0.0,
                        border_radius: 0.0,
                        onclick: move |_| {
                            action_profile_id.set(None);
                            dispatch(state, Action::ActivateProfile(activate_id.clone()));
                        },
                        row {
                            width: "100%",
                            align_items: "center",
                            {arkit::icon("circle-check", 16.0, text_color())}
                            text { content: tr(locale, "设为当前配置", "Use this profile"), margin_left: 10.0, font_size: 13.0, font_weight: 600, font_color: text_color() }
                            row { layout_weight: 1.0 }
                        }
                    }
                    Separator {}
                }
                if profile.subscription_url.is_some() {
                    button {
                        width: "100%",
                        height: 48.0,
                        padding_left: 14.0,
                        padding_right: 14.0,
                        background_color: surface(),
                        border_width: 0.0,
                        border_radius: 0.0,
                        onclick: move |_| {
                            edit_profile_id.set(Some(edit_id.clone()));
                            edit_name.set(edit_profile_name.clone());
                            edit_url.set(edit_profile_url.clone());
                            action_profile_id.set(None);
                        },
                        row {
                            width: "100%",
                            align_items: "center",
                            {arkit::icon("file-pen-line", 16.0, text_color())}
                            text { content: tr(locale, "编辑订阅", "Edit subscription"), margin_left: 10.0, font_size: 13.0, font_weight: 600, font_color: text_color() }
                            row { layout_weight: 1.0 }
                        }
                    }
                    Separator {}
                }
                button {
                    width: "100%",
                    height: 48.0,
                    padding_left: 14.0,
                    padding_right: 14.0,
                    background_color: surface(),
                    border_width: 0.0,
                    border_radius: 0.0,
                    onclick: move |_| {
                        action_profile_id.set(None);
                        dispatch(state, Action::OpenYamlEditor(yaml_id.clone()));
                    },
                    row {
                        width: "100%",
                        align_items: "center",
                        {arkit::icon("file-pen-line", 16.0, text_color())}
                        text { content: tr(locale, "编辑 YAML", "Edit YAML"), margin_left: 10.0, font_size: 13.0, font_weight: 600, font_color: text_color() }
                        row { layout_weight: 1.0 }
                    }
                }
                Separator {}
                button {
                    width: "100%",
                    height: 48.0,
                    padding_left: 14.0,
                    padding_right: 14.0,
                    background_color: surface(),
                    border_width: 0.0,
                    border_radius: 0.0,
                    onclick: move |_| {
                        action_profile_id.set(None);
                        dispatch(state, Action::ExportProfile(export_id.clone()));
                    },
                    row {
                        width: "100%",
                        align_items: "center",
                        {arkit::icon("download", 16.0, text_color())}
                        text { content: tr(locale, "导出配置", "Export profile"), margin_left: 10.0, font_size: 13.0, font_weight: 600, font_color: text_color() }
                        row { layout_weight: 1.0 }
                    }
                }
                if profile.subscription_url.is_some() {
                    Separator {}
                    button {
                        width: "100%",
                        height: 48.0,
                        padding_left: 14.0,
                        padding_right: 14.0,
                        background_color: surface(),
                        border_width: 0.0,
                        border_radius: 0.0,
                        onclick: move |_| {
                            action_profile_id.set(None);
                            dispatch(state, Action::RefreshProfile(refresh_id.clone()));
                        },
                        row {
                            width: "100%",
                            align_items: "center",
                            {arkit::icon("refresh-cw", 16.0, text_color())}
                            text { content: tr(locale, "刷新订阅", "Refresh subscription"), margin_left: 10.0, font_size: 13.0, font_weight: 600, font_color: text_color() }
                            row { layout_weight: 1.0 }
                        }
                    }
                }
                if profile.has_backup {
                    Separator {}
                    button {
                        width: "100%",
                        height: 48.0,
                        padding_left: 14.0,
                        padding_right: 14.0,
                        background_color: surface(),
                        border_width: 0.0,
                        border_radius: 0.0,
                        onclick: move |_| {
                            action_profile_id.set(None);
                            dispatch(state, Action::RestoreProfileBackup(restore_id.clone()));
                        },
                        row {
                            width: "100%",
                            align_items: "center",
                            {arkit::icon("history", 16.0, text_color())}
                            text { content: tr(locale, "恢复上次备份", "Restore backup"), margin_left: 10.0, font_size: 13.0, font_weight: 600, font_color: text_color() }
                            row { layout_weight: 1.0 }
                        }
                    }
                }
                Separator {}
                button {
                    width: "100%",
                    height: 48.0,
                    padding_left: 14.0,
                    padding_right: 14.0,
                    background_color: surface(),
                    border_width: 0.0,
                    border_radius: 0.0,
                    onclick: move |_| {
                        delete_profile_id.set(Some(delete_id.clone()));
                        action_profile_id.set(None);
                    },
                    row {
                        width: "100%",
                        align_items: "center",
                        {arkit::icon("trash-2", 16.0, danger())}
                        text { content: tr(locale, "删除配置", "Delete profile"), margin_left: 10.0, font_size: 13.0, font_weight: 600, font_color: danger() }
                        row { layout_weight: 1.0 }
                    }
                }
            }
        }
    }
}

fn profile_edit_dialog(
    state: Signal<State>,
    locale: UiLocale,
    mut profile_id: Signal<Option<String>>,
    mut name: Signal<String>,
    mut url: Signal<String>,
) -> Element {
    let open = profile_id().is_some();
    rsx! {
        FlatDialog {
            open: open,
            on_close: move |_| profile_id.set(None),
            DialogHeader {
                title: tr(locale, "编辑订阅", "Edit subscription").to_owned(),
                description: Some(tr(locale, "修改名称或订阅地址，保存后可手动刷新。", "Change the name or URL, then refresh when needed.").to_owned()),
            }
            row { height: 18.0 }
            row {
                width: "100%",
                text { content: tr(locale, "名称", "Name"), font_size: 12.0, font_weight: 600, font_color: text_color() }
            }
            row { height: 6.0 }
            Input {
                value: Some(name()),
                placeholder: Some(tr(locale, "配置名称", "Profile name").to_owned()),
                width: Some("100%".into()),
                on_change: move |value| name.set(value),
            }
            row { height: 14.0 }
            row {
                width: "100%",
                text { content: tr(locale, "订阅地址", "Subscription URL"), font_size: 12.0, font_weight: 600, font_color: text_color() }
            }
            row { height: 6.0 }
            Input {
                value: Some(url()),
                placeholder: Some("https://".to_owned()),
                width: Some("100%".into()),
                on_change: move |value| url.set(value),
            }
            DialogFooter {
                FlatButton {
                    variant: FlatButtonVariant::Primary,
                    width: "100%",
                    onclick: move |_| {
                        if let Some(id) = profile_id() {
                            dispatch(state, Action::UpdateProfileSubscription {
                                profile_id: id,
                                name: name(),
                                subscription_url: url(),
                            });
                            profile_id.set(None);
                        }
                    },
                    text { content: tr(locale, "保存修改", "Save changes"), font_size: 13.0, font_weight: 600, font_color: primary_text() }
                }
            }
        }
    }
}

fn profile_delete_dialog(
    state: Signal<State>,
    locale: UiLocale,
    profile: Option<hmeta_model::ProfileSummary>,
    mut profile_id: Signal<Option<String>>,
) -> Element {
    let Some(profile) = profile else {
        return rsx! {};
    };
    let delete_id = profile.id.clone();
    rsx! {
        FlatDialog {
            open: true,
            on_close: move |_| profile_id.set(None),
            DialogHeader {
                title: tr(locale, "删除配置？", "Delete profile?").to_owned(),
                description: Some(format!("{} · {}", truncate_text(&profile.name, 38), tr(locale, "此操作无法撤销", "This cannot be undone"))),
            }
            row { height: 20.0 }
            DialogFooter {
                row {
                    width: "100%",
                    FlatButton {
                        variant: FlatButtonVariant::Outline,
                        onclick: move |_| profile_id.set(None),
                        text { content: tr(locale, "取消", "Cancel"), font_size: 13.0, font_weight: 600, font_color: text_color() }
                    }
                    row { layout_weight: 1.0 }
                    FlatButton {
                        variant: FlatButtonVariant::Destructive,
                        onclick: move |_| {
                            profile_id.set(None);
                            dispatch(state, Action::DeleteProfile(delete_id.clone()));
                        },
                        text { content: tr(locale, "删除", "Delete"), font_size: 13.0, font_weight: 600, font_color: destructive_text() }
                    }
                }
            }
        }
    }
}

fn traffic_page(state: Signal<State>) -> Element {
    let current = state.read().clone();
    let snapshot = current.snapshot;
    let navigator = use_navigator();
    let history = snapshot
        .traffic_history
        .iter()
        .map(|point| (point.download_speed, point.upload_speed))
        .collect::<Vec<_>>();
    let summary = summarize_traffic_history(&history);
    let samples = summary.map(|value| value.samples).unwrap_or(0);
    let peak_download = summary.map(|value| value.peak_download).unwrap_or(0);
    let peak_upload = summary.map(|value| value.peak_upload).unwrap_or(0);
    let active_profile = snapshot.profiles.iter().find(|profile| profile.active);
    let profile_upload = active_profile
        .map(|profile| profile.upload_bytes)
        .unwrap_or(0);
    let profile_download = active_profile
        .map(|profile| profile.download_bytes)
        .unwrap_or(0);
    let connected = snapshot.vpn_running;
    let connection_upload = snapshot
        .connections
        .iter()
        .map(|connection| connection.upload_bytes)
        .sum::<u64>();
    let connection_download = snapshot
        .connections
        .iter()
        .map(|connection| connection.download_bytes)
        .sum::<u64>();
    let active_connection_count = snapshot.connections.len();
    let connection_rows = snapshot.connections.iter().take(5).map(|connection| {
        rsx! {
            {info_row(
                truncate_text(&connection.host, 28),
                format!("↓ {} · ↑ {}", format_total(connection.download_bytes), format_total(connection.upload_bytes)),
            )}
        }
    }).collect::<Vec<_>>();
    let connections_navigator = navigator.clone();
    let dns_upstreams = if snapshot.dns.upstreams.is_empty() {
        "—".to_owned()
    } else {
        truncate_text(&snapshot.dns.upstreams.join(", "), 52)
    };
    let dns_fallbacks = if snapshot.dns.fallbacks.is_empty() {
        "—".to_owned()
    } else {
        truncate_text(&snapshot.dns.fallbacks.join(", "), 52)
    };
    let dns_tun_addresses = if snapshot.dns.tun_addresses.is_empty() {
        "—".to_owned()
    } else {
        snapshot.dns.tun_addresses.join(", ")
    };
    let recent_dns = snapshot.dns.recent_queries.iter().map(|query| {
        rsx! { {info_row(format!("{} {}", query.record_type, query.name), query.count.to_string())} }
    }).collect::<Vec<_>>();
    let diagnostic_pending = current.controller_diagnostic_pending.is_some();
    let memory_in_use = format_total(snapshot.controller_diagnostics.memory_in_use_bytes);
    let memory_limit = if snapshot.controller_diagnostics.memory_limit_bytes > 0 {
        format_total(snapshot.controller_diagnostics.memory_limit_bytes)
    } else {
        "—".to_owned()
    };
    let last_config_sync = snapshot
        .controller_diagnostics
        .last_config_sync_at
        .as_deref()
        .and_then(time_format::format_unix_seconds)
        .unwrap_or_else(|| tr(current.locale, "尚未同步", "Not synced yet").to_owned());
    let body = rsx! {
        column {
            width: "100%",
            align_items: "start",
            row {
                width: "100%",
                align_items: "center",
                text { content: if connected { tr(current.locale, "已连接", "Connected") } else { tr(current.locale, "未连接", "Disconnected") }, font_size: 14.0, font_weight: 650, font_color: if connected { success() } else { subtle() } }
                row { layout_weight: 1.0 }
                {pill(if connected { tr(current.locale, "VPN 运行中", "VPN running") } else { tr(current.locale, "VPN 已停止", "VPN stopped") }.to_owned(), if connected { success() } else { subtle() })}
            }
            row { height: 18.0 }
            text { content: tr(current.locale, "流量用量", "Data usage"), font_size: 17.0, font_weight: 700, font_color: text_color() }
            row { height: 8.0 }
            row {
                width: "100%",
                row {
                    layout_weight: 1.0,
                    {usage_summary_card(
                        tr(current.locale, "当前配置", "Active profile"),
                        profile_upload,
                        profile_download,
                    )}
                }
                row { width: 10.0 }
                row {
                    layout_weight: 1.0,
                    {usage_summary_card(
                        tr(current.locale, "本次会话", "This session"),
                        snapshot.traffic.upload_bytes,
                        snapshot.traffic.download_bytes,
                    )}
                }
            }
            row { height: 18.0 }
            text { content: tr(current.locale, "当前会话", "Current session"), font_size: 17.0, font_weight: 700, font_color: text_color() }
            row { height: 8.0 }
            {traffic_metrics(
                strings(current.locale).traffic_download,
                format_speed(snapshot.traffic.download_speed),
                strings(current.locale).traffic_upload,
                format_speed(snapshot.traffic.upload_speed),
            )}
            row { height: 18.0 }
            {card(
                tr(current.locale, "速率图表", "Speed chart"),
                Some(format!("{} {}", samples, strings(current.locale).traffic_sample_unit)),
                rsx! {
                    column {
                        width: "100%",
                        {info_row(strings(current.locale).traffic_peak_download, format_speed(peak_download))}
                        {info_row(strings(current.locale).traffic_peak_upload, format_speed(peak_upload))}
                        {speed_bars(&snapshot.traffic_history)}
                    }
                }
            )}
            row { height: 12.0 }
            {card(
                tr(current.locale, "当前连接", "Active connections"),
                Some(format!("{} {}", active_connection_count, tr(current.locale, "条", "active"))),
                rsx! {
                    column {
                        width: "100%",
                        {info_row(tr(current.locale, "连接下载", "Connection download"), format_total(connection_download))}
                        {info_row(tr(current.locale, "连接上传", "Connection upload"), format_total(connection_upload))}
                        if !connection_rows.is_empty() {
                            Separator {}
                            {connection_rows.into_iter()}
                        }
                        row { height: 6.0 }
                        FlatButton {
                            variant: FlatButtonVariant::Outline,
                            size: ButtonSize::Sm,
                            width: Some("100%".into()),
                            onclick: move |_| {
                                connections_navigator.push(Route::Connections { query: String::new() });
                            },
                            text { content: tr(current.locale, "查看全部连接", "View all connections"), font_size: 12.0, font_weight: 600, font_color: text_color() }
                            {arkit::icon("chevron-right", 14.0, subtle())}
                        }
                    }
                }
            )}
            row { height: 12.0 }
            {card(
                strings(current.locale).traffic_dns_title.to_owned(),
                Some(snapshot.dns.model.clone()),
                rsx! {
                    column {
                        width: "100%",
                        {info_row(tr(current.locale, "DNS 劫持", "DNS hijack"), if snapshot.dns.hijacking { tr(current.locale, "已启用", "Enabled") } else { tr(current.locale, "已关闭", "Disabled") })}
                        {info_row(tr(current.locale, "监听地址", "Listen"), snapshot.dns.listen.clone())}
                        {info_row(tr(current.locale, "TUN DNS", "TUN DNS"), dns_tun_addresses)}
                        {info_row(tr(current.locale, "上游 DNS", "Upstreams"), dns_upstreams)}
                        {info_row(tr(current.locale, "备用 DNS", "Fallbacks"), dns_fallbacks)}
                        {info_row(tr(current.locale, "域名策略", "Domain policies"), snapshot.dns.nameserver_policy.len().to_string())}
                        {info_row(strings(current.locale).traffic_dns_handled, snapshot.dns.handled_packets.to_string())}
                        {info_row(strings(current.locale).dns_cache_hits, snapshot.dns.cache_hits.to_string())}
                        {info_row(strings(current.locale).dns_cache_misses, snapshot.dns.cache_misses.to_string())}
                        if !recent_dns.is_empty() {
                            row { height: 8.0 }
                            {recent_dns.into_iter()}
                        }
                        row { height: 8.0 }
                        row {
                            width: "100%",
                            FlatButton {
                                variant: FlatButtonVariant::Outline,
                                size: ButtonSize::Sm,
                                disabled: Some(diagnostic_pending),
                                onclick: move |_| dispatch(state, Action::FlushDnsCache),
                                text { content: tr(current.locale, "清理 DNS 缓存", "Flush DNS cache"), font_size: 12.0, font_weight: 600, font_color: text_color() }
                            }
                            row { width: 8.0 }
                            FlatButton {
                                variant: FlatButtonVariant::Outline,
                                size: ButtonSize::Sm,
                                disabled: Some(diagnostic_pending),
                                onclick: move |_| dispatch(state, Action::FlushFakeIpCache),
                                text { content: tr(current.locale, "清理 Fake-IP", "Flush Fake-IP"), font_size: 12.0, font_weight: 600, font_color: text_color() }
                            }
                        }
                    }
                }
            )}
            row { height: 12.0 }
            {card(
                tr(current.locale, "Controller 诊断", "Controller diagnostics"),
                snapshot.controller_addr.clone(),
                rsx! {
                    column {
                        width: "100%",
                        {info_row(tr(current.locale, "内存占用", "Memory in use"), memory_in_use)}
                        {info_row(tr(current.locale, "系统内存上限", "OS memory limit"), memory_limit)}
                        {info_row(tr(current.locale, "配置同步次数", "Config sync count"), snapshot.controller_diagnostics.config_sync_count.to_string())}
                        {info_row(tr(current.locale, "最近配置同步", "Last config sync"), last_config_sync)}
                        if let Some(error) = snapshot.controller_diagnostics.last_config_sync_error.clone() {
                            text { content: compact(&error), margin_top: 6.0, font_size: 12.0, font_color: danger(), max_lines: 3 }
                        }
                    }
                }
            )}
        }
    };
    scaffold(state, Route::Traffic {}, rsx! {}, body)
}

fn resources_page(state: Signal<State>) -> Element {
    let mut query = use_signal(String::new);
    let mut geodata_detail = use_signal(|| None::<hmeta_model::GeodataFileSummary>);
    let mut provider_detail = use_signal(|| None::<String>);
    let current = state.read().clone();
    let query_value = query();
    let active_profile_name = current
        .snapshot
        .profiles
        .iter()
        .find(|profile| profile.active)
        .map(|profile| profile.name.clone())
        .unwrap_or_else(|| tr(current.locale, "未选择", "Unselected").to_owned());
    let enabled_rule_count = current
        .snapshot
        .rules
        .iter()
        .filter(|rule| rule.enabled)
        .count();
    let total_rule_count = current.snapshot.rules.len();
    let total_provider_count = current.snapshot.providers.len();
    let ready_geodata_count = current
        .snapshot
        .geodata
        .iter()
        .filter(|file| file.exists)
        .count();
    let total_geodata_count = current.snapshot.geodata.len();
    let mode_label = match current.snapshot.mode {
        RuntimeMode::Rule => tr(current.locale, "规则", "Rule"),
        RuntimeMode::Global => tr(current.locale, "全局", "Global"),
        RuntimeMode::Direct => tr(current.locale, "直连", "Direct"),
    };
    let providers = current.snapshot.providers.iter()
        .filter(|provider| matches_provider_query(provider, &query_value))
        .cloned()
        .map(|provider| {
            let refresh_provider_type = provider.provider_type.clone();
            let refresh_provider_name = provider.name.clone();
            let health_provider_name = provider.name.clone();
            let detail_provider_name = provider.name.clone();
            let member_count = provider.members.len();
            let alive_count = provider.members.iter().filter(|member| member.alive).count();
            let can_healthcheck = provider.provider_type == "proxy"
                && provider.health_check_enabled;
            let provider_status = if provider.last_refresh_error.is_some() {
                tr(current.locale, "刷新失败", "Refresh failed")
            } else if provider.vehicle_type.as_deref().is_some_and(|kind| kind.eq_ignore_ascii_case("inline")) {
                tr(current.locale, "内置已加载", "Inline loaded")
            } else if provider.cache_exists {
                tr(current.locale, "缓存已加载", "Cache loaded")
            } else {
                tr(current.locale, "等待缓存", "Cache pending")
            };
            rsx! {
                {card(
                    truncate_text(&provider.name, 38),
                    Some(format!("{} · {}", provider.provider_type, provider.vehicle_type.clone().unwrap_or_default())),
                    rsx! {
                        column {
                            width: "100%",
                            {info_row(tr(current.locale, "状态", "Status"), provider_status)}
                            {info_row(tr(current.locale, "缓存", "Cache"), if provider.cache_exists { format_total(provider.cache_bytes.unwrap_or(0)) } else { tr(current.locale, "无", "None").to_owned() })}
                            {info_row(tr(current.locale, "刷新间隔", "Interval"), provider.interval_seconds.map(|value| format!("{value}s")).unwrap_or_else(|| "-".to_owned()))}
                            if provider.provider_type == "proxy" {
                                {info_row(tr(current.locale, "成员健康", "Member health"), format!("{alive_count}/{member_count}"))}
                            }
                            if let Some(error) = provider.last_refresh_error.clone() {
                                text { content: compact(&error), margin_top: 6.0, font_size: 12.0, font_color: danger(), max_lines: 2 }
                            }
                            row { height: 4.0 }
                            row {
                                width: "100%",
                                justify_content: "end",
                                FlatButton {
                                    variant: FlatButtonVariant::Ghost,
                                    size: ButtonSize::Sm,
                                    onclick: move |_| provider_detail.set(Some(detail_provider_name.clone())),
                                    {arkit::icon("list", 14.0, text_color())}
                                    text { content: tr(current.locale, "详情", "Details"), margin_left: 6.0, font_size: 12.0, font_weight: 600, font_color: text_color() }
                                }
                                if can_healthcheck {
                                    FlatButton {
                                        variant: FlatButtonVariant::Ghost,
                                        size: ButtonSize::Sm,
                                        disabled: Some(current.controller_diagnostic_pending.is_some()),
                                        onclick: move |_| dispatch(state, Action::HealthcheckProxyProvider {
                                            provider_name: health_provider_name.clone(),
                                        }),
                                        {arkit::icon("heart-pulse", 14.0, text_color())}
                                        text { content: tr(current.locale, "检查", "Check"), margin_left: 6.0, font_size: 12.0, font_weight: 600, font_color: text_color() }
                                    }
                                }
                                FlatButton {
                                    variant: FlatButtonVariant::Ghost,
                                    size: ButtonSize::Sm,
                                    onclick: move |_| dispatch(state, Action::RefreshProvider {
                                        provider_type: refresh_provider_type.clone(),
                                        provider_name: refresh_provider_name.clone(),
                                    }),
                                    {arkit::icon("refresh-cw", 14.0, text_color())}
                                    text { content: tr(current.locale, "刷新", "Refresh"), margin_left: 6.0, font_size: 12.0, font_weight: 600, font_color: text_color() }
                                }
                            }
                        }
                    }
                )}
            }
        }).collect::<Vec<_>>();
    let rules = current
        .snapshot
        .rules
        .iter()
        .filter(|rule| matches_rule_query(rule, &query_value))
        .cloned()
        .map(|rule| rule_view(state, &current, rule))
        .collect::<Vec<_>>();
    let geodata = current.snapshot.geodata.iter()
        .filter(|file| matches_geodata_query(file, &query_value))
        .cloned()
        .enumerate()
        .map(|(index, file)| {
            let detail = file.clone();
            let status = if file.exists {
                tr(current.locale, "可用", "Available")
            } else {
                tr(current.locale, "缺失", "Missing")
            };
            let metadata = if file.exists {
                format!("{status} · {}", format_total(file.bytes.unwrap_or(0)))
            } else {
                status.to_owned()
            };
            rsx! {
                if index > 0 {
                    Separator {}
                }
                button {
                    width: "100%",
                    height: 68.0,
                    padding_left: 14.0,
                    padding_right: 12.0,
                    background_color: surface(),
                    border_width: 0.0,
                    border_radius: 0.0,
                    onclick: move |_| geodata_detail.set(Some(detail.clone())),
                    row {
                        width: "100%",
                        align_items: "center",
                        row {
                            width: 36.0,
                            height: 36.0,
                            align_items: "center",
                            justify_content: "center",
                            background_color: muted(),
                            border_radius: 9.0,
                            {arkit::icon("file-text", 17.0, if file.exists { success() } else { danger() })}
                        }
                        column {
                            layout_weight: 1.0,
                            margin_left: 11.0,
                            align_items: "start",
                            text { content: file.name, width: "100%", font_size: 13.0, font_weight: 650, font_color: text_color(), max_lines: 1 }
                            text { content: metadata, width: "100%", margin_top: 3.0, font_size: 11.0, font_color: if file.exists { success() } else { danger() }, max_lines: 1 }
                        }
                        {arkit::icon("chevron-right", 15.0, subtle())}
                    }
                }
            }
        }).collect::<Vec<_>>();
    let visible_geodata_count = geodata.len();
    let selected_geodata = geodata_detail();
    let selected_provider = provider_detail().and_then(|name| {
        current
            .snapshot
            .providers
            .iter()
            .find(|provider| provider.name == name)
            .cloned()
    });
    let body = rsx! {
        column {
            width: "100%",
            Input {
                value: Some(query_value),
                placeholder: Some(strings(current.locale).resources_search_placeholder.to_owned()),
                width: Some("100%".into()),
                on_change: move |value| query.set(value),
            }
            row { height: 12.0 }
            {card(
                tr(current.locale, "规则运行状态", "Rules runtime"),
                Some(active_profile_name),
                rsx! {
                    column {
                        width: "100%",
                        {info_row(tr(current.locale, "引擎配置", "Engine config"), if current.snapshot.engine_loaded { tr(current.locale, "已加载", "Loaded") } else { tr(current.locale, "未加载", "Not loaded") })}
                        {info_row(tr(current.locale, "当前模式", "Current mode"), mode_label)}
                        {info_row(tr(current.locale, "生效规则", "Effective rules"), format!("{enabled_rule_count}/{total_rule_count}"))}
                        {info_row("Provider", total_provider_count.to_string())}
                        {info_row("GeoData", format!("{ready_geodata_count}/{total_geodata_count}"))}
                    }
                }
            )}
            row { height: 12.0 }
            column {
                width: "100%",
                background_color: surface(),
                border_width: 1.0,
                border_color: line(),
                border_radius: 10.0,
                clip: true,
                row {
                    width: "100%",
                    height: 56.0,
                    padding_left: 14.0,
                    padding_right: 14.0,
                    align_items: "center",
                    text { content: "GeoData", font_size: 14.0, font_weight: 700, font_color: text_color() }
                    row { layout_weight: 1.0 }
                    text {
                        content: format!("{ready_geodata_count}/{total_geodata_count} {}", tr(current.locale, "可用", "ready")),
                        font_size: 11.0,
                        font_weight: 600,
                        font_color: if ready_geodata_count == total_geodata_count && total_geodata_count > 0 { success() } else { warning() },
                    }
                }
                Separator {}
                if visible_geodata_count == 0 {
                    row {
                        width: "100%",
                        height: 66.0,
                        padding_left: 14.0,
                        padding_right: 14.0,
                        align_items: "center",
                        text { content: tr(current.locale, "没有匹配的 GeoData 文件", "No matching GeoData files"), font_size: 12.0, font_color: subtle() }
                    }
                } else {
                    {geodata.into_iter()}
                }
            }
            row { height: 12.0 }
            {section_label(tr(current.locale, "Provider", "Providers"))}
            if providers.is_empty() {
                {empty_state("database", tr(current.locale, "当前订阅没有 Provider", "No providers in this profile"), tr(current.locale, "分享链接订阅通常只包含节点；Provider 需由 Clash YAML 显式声明", "Share-link subscriptions usually contain nodes only; providers must be declared by Clash YAML"))}
            } else {
                {spaced(providers)}
            }
            row { height: 14.0 }
            row {
                width: "100%",
                height: 34.0,
                margin_bottom: 8.0,
                align_items: "center",
                text { content: strings(current.locale).resources_rules_title, font_size: 15.0, font_weight: 750, font_color: text_color() }
                row { layout_weight: 1.0 }
                FlatButton {
                    variant: FlatButtonVariant::Ghost,
                    size: ButtonSize::Sm,
                    onclick: move |_| dispatch(state, Action::OpenManualRuleEditor {
                        connection_id: None,
                        domain: String::new(),
                        destination_ip: String::new(),
                    }),
                    {arkit::icon("plus", 14.0, text_color())}
                    text { content: tr(current.locale, "添加", "Add"), margin_left: 5.0, font_size: 12.0, font_weight: 650, font_color: text_color() }
                }
            }
            if rules.is_empty() {
                {empty_state("list-checks", tr(current.locale, "当前配置没有可编辑规则", "No editable rules"), tr(current.locale, "请确认已选择订阅并完成配置加载", "Select a profile and wait for configuration loading"))}
            } else {
                {compact_rule_list(rules)}
            }
        }
    };
    let actions = rsx! {
        row {
            {icon_action("file-up", Action::ImportRules, state)}
            {icon_action("refresh-cw", Action::RefreshAllProviders, state)}
        }
    };
    let page = scaffold(state, Route::Resources {}, actions, body);
    rsx! {
        {page}
        if let Some(file) = selected_geodata {
            {geodata_detail_dialog(current.locale, file, geodata_detail)}
        }
        if let Some(provider) = selected_provider {
            {provider_detail_dialog(
                state,
                current.locale,
                provider,
                provider_detail,
                current.controller_diagnostic_pending.is_some(),
            )}
        }
        if current.manual_rule_editor.is_some() {
            {manual_rule_dialog(state, &current)}
        }
    }
}

fn provider_detail_dialog(
    state: Signal<State>,
    locale: UiLocale,
    provider: hmeta_model::ProviderSummary,
    mut selected: Signal<Option<String>>,
    pending: bool,
) -> Element {
    let provider_name = provider.name.clone();
    let health_url = provider
        .health_check_url
        .clone()
        .unwrap_or_else(|| "https://www.gstatic.com/generate_204".to_owned());
    let expected_status = provider.expected_status.clone();
    let members = provider.members.into_iter().map(|member| {
        let check_provider = provider_name.clone();
        let check_proxy = member.name.clone();
        let check_url = health_url.clone();
        let check_expected = expected_status.clone();
        let status = if member.alive {
            tr(locale, "可用", "Alive")
        } else {
            tr(locale, "不可用", "Unavailable")
        };
        let delay = member
            .delay_ms
            .map(|delay| format!("{delay} ms"))
            .unwrap_or_else(|| tr(locale, "未测试", "Untested").to_owned());
        rsx! {
            row {
                width: "100%",
                height: 50.0,
                padding_left: 10.0,
                padding_right: 8.0,
                align_items: "center",
                column {
                    layout_weight: 1.0,
                    align_items: "start",
                    text { content: truncate_text(&member.name, 34), width: "100%", font_size: 12.0, font_weight: 650, font_color: text_color(), max_lines: 1 }
                    text { content: format!("{} · {} · {}", member.proxy_type, status, delay), margin_top: 3.0, width: "100%", font_size: 10.0, font_color: if member.alive { success() } else { danger() }, max_lines: 1 }
                }
                FlatButton {
                    variant: FlatButtonVariant::Ghost,
                    size: ButtonSize::Icon,
                    disabled: Some(pending),
                    onclick: move |_| dispatch(state, Action::HealthcheckProviderProxy {
                        provider_name: check_provider.clone(),
                        proxy_name: check_proxy.clone(),
                        url: check_url.clone(),
                        expected_status: check_expected.clone(),
                    }),
                    {arkit::icon("gauge", 15.0, text_color())}
                }
            }
            Separator {}
        }
    }).collect::<Vec<_>>();
    rsx! {
        FlatDialog {
            open: true,
            on_close: move |_| selected.set(None),
            DialogHeader {
                title: truncate_text(&provider_name, 42),
                description: Some(format!("{} {}", members.len(), tr(locale, "个成员", "members"))),
            }
            row { height: 12.0 }
            if members.is_empty() {
                text { content: tr(locale, "当前 Provider 没有可展示的成员", "No provider members available"), font_size: 12.0, font_color: subtle() }
            } else {
                column {
                    width: "100%",
                    border_width: 1.0,
                    border_color: line(),
                    border_radius: 9.0,
                    clip: true,
                    {members.into_iter()}
                }
            }
        }
    }
}

fn geodata_detail_dialog(
    locale: UiLocale,
    file: hmeta_model::GeodataFileSummary,
    mut selected: Signal<Option<hmeta_model::GeodataFileSummary>>,
) -> Element {
    let availability = if file.exists {
        tr(locale, "文件可用", "File available")
    } else {
        tr(locale, "文件缺失", "File missing")
    };
    let size = file
        .bytes
        .map(format_total)
        .unwrap_or_else(|| "-".to_owned());
    let updated_at = file
        .updated_at
        .as_deref()
        .and_then(time_format::format_unix_seconds)
        .or(file.updated_at.clone())
        .unwrap_or_else(|| "-".to_owned());
    rsx! {
        FlatDialog {
            open: true,
            on_close: move |_| selected.set(None),
            DialogHeader {
                title: file.name,
                description: Some(availability.to_owned()),
            }
            row { height: 16.0 }
            column {
                width: "100%",
                border_width: 1.0,
                border_color: line(),
                border_radius: 9.0,
                padding_left: 12.0,
                padding_right: 12.0,
                {info_row(tr(locale, "状态", "Status"), availability)}
                Separator {}
                {info_row(tr(locale, "文件大小", "File size"), size)}
                Separator {}
                {info_row(tr(locale, "更新时间", "Updated"), updated_at)}
            }
            row { height: 14.0 }
            text { content: tr(locale, "文件位置", "File location"), font_size: 11.0, font_weight: 650, font_color: subtle() }
            row { height: 6.0 }
            row {
                width: "100%",
                padding: 11.0,
                background_color: muted(),
                border_radius: 8.0,
                text {
                    content: file.path,
                    width: "100%",
                    font_size: 11.0,
                    line_height: 17.0,
                    font_color: text_color(),
                    max_lines: 5,
                }
            }
        }
    }
}

fn rule_view(state: Signal<State>, current: &State, rule: hmeta_model::RuleSummary) -> Element {
    let editable = rule.source != "profile-yaml";
    let rule_source = if editable {
        rule.source.clone()
    } else {
        tr(
            current.locale,
            "订阅配置 · 已载入运行时",
            "Profile YAML · loaded at runtime",
        )
        .to_owned()
    };
    let toggle_profile = rule.profile_id.clone();
    let toggle_id = rule.id.clone();
    let delete_profile = rule.profile_id.clone();
    let delete_id = rule.id.clone();
    let enabled = rule.enabled;
    let up = reordered_rule_ids(&current.snapshot.rules, &rule.profile_id, &rule.id, -1);
    let down = reordered_rule_ids(&current.snapshot.rules, &rule.profile_id, &rule.id, 1);
    let toggle_action = Action::SetRuleEnabled {
        profile_id: toggle_profile,
        rule_id: toggle_id,
        enabled: !enabled,
    };
    let delete_action = Action::DeleteRule {
        profile_id: delete_profile,
        rule_id: delete_id,
    };
    rsx! {
        column {
            width: "100%",
            height: 88.0,
            padding_top: 8.0,
            padding_right: 8.0,
            padding_bottom: 8.0,
            padding_left: 10.0,
            background_color: surface(),
            border_width: 1.0,
            border_color: line(),
            border_radius: 8.0,
            clip: true,
            row {
                width: "100%",
                height: 32.0,
                align_items: "center",
                text {
                    content: format!("#{}", rule.order + 1),
                    font_size: 11.0,
                    font_weight: 700,
                    font_color: if enabled { success() } else { subtle() },
                    max_lines: 1,
                }
                row {
                    layout_weight: 1.0,
                    margin_left: 7.0,
                    margin_right: 4.0,
                    text {
                        content: rule_source,
                        width: "100%",
                        font_size: 10.0,
                        font_color: subtle(),
                        max_lines: 1,
                    }
                }
                if editable {
                    {compact_rule_action(if enabled { "toggle-right" } else { "toggle-left" }, if enabled { success() } else { subtle() }, toggle_action, state)}
                    if let Some(ids) = up {
                        {compact_rule_action("arrow-up", subtle(), Action::ReorderRules { profile_id: rule.profile_id.clone(), ordered_rule_ids: ids }, state)}
                    }
                    if let Some(ids) = down {
                        {compact_rule_action("arrow-down", subtle(), Action::ReorderRules { profile_id: rule.profile_id.clone(), ordered_rule_ids: ids }, state)}
                    }
                    {compact_rule_action("trash-2", danger(), delete_action, state)}
                } else {
                    {pill(tr(current.locale, "运行中", "Effective").to_owned(), success())}
                }
            }
            text {
                content: truncate_text(&rule.line, 180),
                width: "100%",
                margin_top: 5.0,
                font_size: 11.0,
                line_height: 16.0,
                font_color: text_color(),
                max_lines: 2,
            }
        }
    }
}

fn compact_rule_action(
    icon: &'static str,
    color: u32,
    action: Action,
    state: Signal<State>,
) -> Element {
    rsx! {
        button {
            width: 32.0,
            height: 32.0,
            padding: 0.0,
            background_color: surface(),
            border_width: 0.0,
            border_radius: 7.0,
            onclick: move |_| dispatch(state, action.clone()),
            row {
                width: "100%",
                height: "100%",
                align_items: "center",
                justify_content: "center",
                {arkit::icon(icon, 15.0, color)}
            }
        }
    }
}

fn compact_rule_list(items: Vec<Element>) -> Element {
    let len = items.len();
    let nodes = items.into_iter().enumerate().map(|(index, item)| {
        rsx! {
            {item}
            if index + 1 < len { row { height: 6.0 } }
        }
    });
    rsx! { column { width: "100%", {nodes} } }
}

fn reordered_rule_ids(
    rules: &[hmeta_model::RuleSummary],
    profile_id: &str,
    rule_id: &str,
    delta: isize,
) -> Option<Vec<String>> {
    let mut ordered = rules
        .iter()
        .filter(|rule| rule.profile_id == profile_id && rule.source != "profile-yaml")
        .collect::<Vec<_>>();
    ordered.sort_by_key(|rule| rule.order);
    let index = ordered.iter().position(|rule| rule.id == rule_id)?;
    let target = index.checked_add_signed(delta)?;
    if target >= ordered.len() {
        return None;
    }
    ordered.swap(index, target);
    Some(ordered.into_iter().map(|rule| rule.id.clone()).collect())
}

fn logs_page(state: Signal<State>) -> Element {
    let mut log_query = use_signal(String::new);
    let mut log_filter = use_signal(|| LogLevelFilter::All);
    let mut selected_log = use_signal(|| None::<VirtualLogRow>);
    let current = state.read().clone();
    let query_value = log_query();
    let normalized_query = normalize_log_query(&query_value);
    let filter_value = log_filter();
    let all_label = strings(current.locale).logs_level_all.to_owned();
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
    };
    let selected_log_value = selected_log();
    let body = rsx! {
        column {
            width: "100%",
            height: "100%",
            Input {
                value: Some(query_value),
                placeholder: Some(strings(current.locale).logs_search_placeholder.to_owned()),
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
                    content: format!("{} / {} {}", shown_log_count, total_log_count, tr(current.locale, "条日志", "logs")),
                    font_size: 11.0,
                    font_color: subtle(),
                }
                row { layout_weight: 1.0 }
                if !empty {
                    text { content: tr(current.locale, "点击日志查看全文", "Tap a log for details"), font_size: 11.0, font_color: subtle() }
                }
            }
            row {
                layout_weight: 1.0,
                width: "100%",
                if empty {
                    {empty_state("scroll-text", strings(current.locale).logs_empty_title, strings(current.locale).logs_empty_subtitle)}
                } else {
                    VirtualLogList {
                        items: logs,
                        palette,
                        on_open: move |row: VirtualLogRow| selected_log.set(Some(row)),
                    }
                }
            }
        }
    };
    let page = fixed_scaffold(
        state,
        Route::Logs {},
        destructive_icon_action("trash-2", Action::ClearLogs, state),
        body,
    );
    rsx! {
        {page}
        if let Some(log) = selected_log_value {
            {log_detail_dialog(current.locale, log, selected_log)}
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
}

#[derive(Clone)]
struct VirtualLogRenderState {
    items: Vec<VirtualLogRow>,
    palette: VirtualLogPalette,
    on_open: EventHandler<VirtualLogRow>,
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
    let render_state = use_hook(|| {
        Rc::new(RefCell::new(VirtualLogRenderState {
            items: items.clone(),
            palette,
            on_open,
        }))
    });
    *render_state.borrow_mut() = VirtualLogRenderState {
        items,
        palette,
        on_open,
    };
    let render_state_for_adapter = render_state.clone();
    let handle = use_virtual_node_adapter_items_keyed(VirtualKind::List, item_keys, move |index| {
        let state = render_state_for_adapter.borrow();
        render_virtual_log_row(&state.items[index as usize], state.palette, state.on_open)
    });
    let attach_handle = handle.clone();
    use_layout_frame_node(move |host_node, _frame| {
        let _ = attach_handle.attach(&host_node);
    });

    rsx! {
        list {
            width: "100%",
            height: "100%",
            list_cached_count: 18_i32,
        }
    }
}

fn render_virtual_log_row(
    item: &VirtualLogRow,
    palette: VirtualLogPalette,
    on_open: EventHandler<VirtualLogRow>,
) -> arkit::ohos_arkui_binding::common::error::ArkUIResult<ArkUINode> {
    let meta = virtual_log_text(item.meta.clone(), 10.0, 5, item.color, 15.0, 1, 0.0)?;
    let message = virtual_log_text(
        item.preview.clone(),
        12.0,
        4,
        palette.foreground,
        17.0,
        2,
        4.0,
    )?;
    let node = NodeBuilder::new("column")?
        .percent_width(1.0)?
        .height(76.0)?
        .background_color(format!("#{:08x}", palette.surface))?
        .padding([9.0, 11.0, 9.0, 11.0])?
        .margin([0.0, 0.0, 7.0, 0.0])?
        .attr(ArkUINodeAttributeType::BorderWidth, vec![1.0; 4])?
        .attr(ArkUINodeAttributeType::BorderColor, palette.border)?
        .attr(ArkUINodeAttributeType::BorderRadius, vec![9.0; 4])?
        .attr(ArkUINodeAttributeType::Clip, true)?
        .attr(ArkUINodeAttributeType::ColumnAlignItems, 0_i32)?
        .attr(
            ArkUINodeAttributeType::AccessibilityText,
            format!("{}，{}", item.meta, item.message),
        )?
        .child(meta)?
        .child(message)?;
    let item = item.clone();
    Ok(node.on_click(move || on_open.call(item.clone()))?.build())
}

fn virtual_log_text(
    content: String,
    size: f32,
    weight: i32,
    color: u32,
    line_height: f32,
    max_lines: i32,
    padding_top: f32,
) -> arkit::ohos_arkui_binding::common::error::ArkUIResult<ArkUINode> {
    Ok(NodeBuilder::new("text")?
        .percent_width(1.0)?
        .font_size(size)?
        .font_color(format!("#{color:08x}"))?
        .text_content(content)?
        .padding([padding_top, 0.0, 0.0, 0.0])?
        .attr(ArkUINodeAttributeType::FontWeight, weight)?
        .attr(ArkUINodeAttributeType::TextLineHeight, line_height)?
        .attr(ArkUINodeAttributeType::TextMaxLines, max_lines)?
        .attr(ArkUINodeAttributeType::TextOverflow, 2_i32)?
        .build())
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
                title: tr(locale, "日志详情", "Log details").to_owned(),
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

fn profile_import_dialog(
    state: Signal<State>,
    current: &State,
    open: bool,
    mut open_signal: Signal<bool>,
    url: Signal<String>,
    name: Signal<String>,
    submitted: Signal<bool>,
) -> Element {
    let import_loading = current.profile_import_loading;
    let error_value = current.profile_import_error.clone().unwrap_or_default();
    // Refresh the overlay shell when pending/error flips so the live body is remounted
    // with the latest loading branch. Field edits are handled by the body component.
    let content_key = dialog_content_key(&[
        if import_loading { "loading" } else { "idle" },
        &error_value,
    ]);
    rsx! {
        FlatDialog {
            open: open,
            content_key: content_key,
            on_close: move |_| {
                if !state.read().profile_import_loading {
                    open_signal.set(false);
                    dispatch(state, Action::ResetProfileImportFeedback);
                }
            },
            ProfileImportDialogBody {
                state,
                url,
                name,
                submitted,
            }
        }
    }
}

/// Lives inside the overlay tree and re-reads `state` so Spinner/disabled can
/// update while the dialog stays open.
#[component]
fn ProfileImportDialogBody(
    state: Signal<State>,
    mut url: Signal<String>,
    mut name: Signal<String>,
    mut submitted: Signal<bool>,
) -> Element {
    let current = state.read().clone();
    let locale = current.locale;
    let import_loading = current.profile_import_loading;
    let loading_label = strings(locale).profiles_import_loading;
    let url_value = url();
    let name_value = name();
    rsx! {
        DialogHeader {
            title: strings(locale).profiles_import_network.to_owned(),
            description: Some(strings(locale).profiles_import_network_subtitle.to_owned()),
        }
        row { height: 20.0 }
        column {
            width: "100%",
            Input {
                value: Some(url_value),
                placeholder: Some(strings(locale).profiles_import_url_label.to_owned()),
                width: Some("100%".into()),
                disabled: import_loading,
                on_change: move |value| {
                    url.set(value);
                    dispatch(state, Action::ResetProfileImportFeedback);
                },
            }
            row { height: 12.0 }
            Input {
                value: Some(name_value),
                placeholder: Some(strings(locale).profiles_import_name_placeholder.to_owned()),
                width: Some("100%".into()),
                disabled: import_loading,
                on_change: move |value| {
                    name.set(value);
                    dispatch(state, Action::ResetProfileImportFeedback);
                },
            }
            row { height: 8.0 }
            FlatButton {
                variant: FlatButtonVariant::Ghost,
                size: ButtonSize::Sm,
                disabled: Some(import_loading),
                onclick: move |_| {
                    if !state.read().profile_import_loading {
                        submitted.set(true);
                        dispatch(state, Action::ImportLocalProfile);
                    }
                },
                if import_loading {
                    Spinner { size: 14.0, color: Some(text_color()) }
                } else {
                    {arkit::icon("file-up", 14.0, text_color())}
                }
                text {
                    content: if import_loading {
                        loading_label
                    } else {
                        tr(locale, "从本地文件导入", "Import from local file")
                    },
                    margin_left: 6.0,
                    font_size: 12.0,
                    font_weight: 600,
                    font_color: text_color(),
                }
            }
            if let Some(error) = current.profile_import_error.clone() {
                text { content: error, margin_top: 10.0, font_size: 12.0, line_height: 18.0, font_color: danger() }
            }
        }
        DialogFooter {
            FlatButton {
                variant: FlatButtonVariant::Primary,
                width: Some("100%".into()),
                disabled: Some(import_loading),
                onclick: move |_| {
                    if !state.read().profile_import_loading {
                        submitted.set(true);
                        dispatch(state, Action::ImportProfileFromUrl {
                            url: url(),
                            name: name(),
                        });
                    }
                },
                if import_loading {
                    Spinner { size: 16.0, color: Some(primary_text()) }
                } else {
                    {arkit::icon("download", 16.0, primary_text())}
                }
                text {
                    content: if import_loading {
                        loading_label
                    } else {
                        strings(locale).profiles_import_submit
                    },
                    margin_left: 8.0,
                    font_size: 14.0,
                    font_weight: 600,
                    font_color: primary_text(),
                }
            }
        }
    }
}

fn yaml_editor_dialog(state: Signal<State>, current: &State) -> Element {
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

fn card(title: impl Into<String>, subtitle: Option<String>, body: Element) -> Element {
    let title = title.into();
    rsx! {
        Card {
            shadow: Some(false),
            if let Some(subtitle) = subtitle {
                CardHeader {
                    title: title,
                    description: subtitle,
                }
            } else {
                row {
                    width: "100%",
                    padding_top: spacing::XXL,
                    padding_right: spacing::XXL,
                    padding_bottom: spacing::LG,
                    padding_left: spacing::XXL,
                    CardTitle { content: title }
                }
            }
            CardContent {
                {body}
            }
        }
    }
}

fn traffic_metrics(
    download_label: impl Into<String>,
    download_value: impl Into<String>,
    upload_label: impl Into<String>,
    upload_value: impl Into<String>,
) -> Element {
    let download_label = download_label.into();
    let download_value = download_value.into();
    let upload_label = upload_label.into();
    let upload_value = upload_value.into();
    let theme = use_theme();
    rsx! {
        Card {
            shadow: Some(false),
            row {
                width: "100%",
                height: 92.0,
                padding: spacing::LG,
                align_items: "center",
                row { layout_weight: 1.0, align_items: "center",
                    row {
                        width: 38.0,
                        height: 38.0,
                        align_items: "center",
                        justify_content: "center",
                        background_color: theme.colors.muted,
                        border_radius: theme.radii.lg,
                        {arkit::icon("arrow-down", 18.0, success())}
                    }
                    column {
                        margin_left: spacing::MD,
                        text { content: download_label, font_size: typography::XS, line_height: 18.0, font_color: theme.colors.muted_foreground }
                        text { content: download_value, margin_top: 2.0, font_size: typography::LG, line_height: 24.0, font_weight: 700, font_color: theme.colors.foreground }
                    }
                }
                row { width: 1.0, height: 48.0, margin_left: spacing::MD, margin_right: spacing::LG, background_color: theme.colors.border }
                row { layout_weight: 1.0, align_items: "center",
                    row {
                        width: 38.0,
                        height: 38.0,
                        align_items: "center",
                        justify_content: "center",
                        background_color: theme.colors.muted,
                        border_radius: theme.radii.lg,
                        {arkit::icon("arrow-up", 18.0, warning())}
                    }
                    column {
                        margin_left: spacing::MD,
                        text { content: upload_label, font_size: typography::XS, line_height: 18.0, font_color: theme.colors.muted_foreground }
                        text { content: upload_value, margin_top: 2.0, font_size: typography::LG, line_height: 24.0, font_weight: 700, font_color: theme.colors.foreground }
                    }
                }
            }
        }
    }
}

fn usage_summary_card(title: impl Into<String>, upload: u64, download: u64) -> Element {
    let title = title.into();
    let theme = use_theme();
    rsx! {
        Card {
            shadow: Some(false),
            column {
                width: "100%",
                height: 126.0,
                padding: spacing::MD,
                align_items: "start",
                text {
                    content: title,
                    font_size: typography::SM,
                    font_weight: 600,
                    font_color: theme.colors.foreground,
                    max_lines: 1,
                }
                row { height: spacing::MD }
                row {
                    width: "100%",
                    align_items: "center",
                    {arkit::icon("arrow-up", 14.0, warning())}
                    text {
                        content: format_total(upload),
                        margin_left: spacing::XS,
                        font_size: typography::SM,
                        font_weight: 600,
                        font_color: theme.colors.foreground,
                        max_lines: 1,
                    }
                }
                row { height: spacing::SM }
                row {
                    width: "100%",
                    align_items: "center",
                    {arkit::icon("arrow-down", 14.0, success())}
                    text {
                        content: format_total(download),
                        margin_left: spacing::XS,
                        font_size: typography::SM,
                        font_weight: 600,
                        font_color: theme.colors.foreground,
                        max_lines: 1,
                    }
                }
            }
        }
    }
}

fn info_row(label: impl Into<String>, value: impl Into<String>) -> Element {
    let label = label.into();
    let value = value.into();
    rsx! {
        row {
            width: "100%",
            height: 36.0,
            align_items: "center",
            text { content: label, font_size: 13.0, font_color: subtle() }
            row {
                layout_weight: 1.0,
                margin_left: 16.0,
                justify_content: "end",
                text {
                    width: "100%",
                    content: value,
                    font_size: 13.0,
                    line_height: 19.0,
                    font_weight: 600,
                    font_color: text_color(),
                    max_lines: 1,
                    text_align: "end",
                }
            }
        }
    }
}

fn pill(label: String, color: u32) -> Element {
    rsx! {
        Badge {
            content: label,
            variant: BadgeVariant::Secondary,
            icon_colors: Some((muted(), color)),
            pill: Some(true),
        }
    }
}

fn section_label(label: impl Into<String>) -> Element {
    let label = label.into();
    let theme = use_theme();
    rsx! {
        row {
            width: "100%",
            margin_bottom: spacing::SM,
            text {
                content: label,
                font_size: typography::MD,
                font_weight: 600,
                font_color: theme.colors.foreground,
            }
        }
    }
}

fn empty_state(
    icon: &'static str,
    title: impl Into<String>,
    subtitle: impl Into<String>,
) -> Element {
    let title = title.into();
    let subtitle = subtitle.into();
    let theme = use_theme();
    rsx! {
        Card {
            shadow: Some(false),
            column {
                width: "100%",
                height: 190.0,
                padding: spacing::XXL,
                align_items: "center",
                justify_content: "center",
                row {
                    width: 48.0,
                    height: 48.0,
                    align_items: "center",
                    justify_content: "center",
                    background_color: theme.colors.muted,
                    border_radius: theme.radii.xl,
                    {arkit::icon(icon, 22.0, theme.colors.muted_foreground)}
                }
                text {
                    content: title,
                    margin_top: spacing::MD,
                    font_size: typography::MD,
                    line_height: 22.0,
                    font_weight: 600,
                    font_color: theme.colors.foreground,
                }
                text {
                    content: subtitle,
                    margin_top: spacing::XXS,
                    font_size: typography::SM,
                    line_height: 20.0,
                    font_color: theme.colors.muted_foreground,
                    text_align: "center",
                }
            }
        }
    }
}

fn spaced(items: Vec<Element>) -> Element {
    let len = items.len();
    let nodes = items.into_iter().enumerate().map(|(index, item)| {
        rsx! {
            {item}
            if index + 1 < len { row { height: 10.0 } }
        }
    });
    rsx! { column { width: "100%", {nodes} } }
}

fn icon_action(icon: &'static str, action: Action, state: Signal<State>) -> Element {
    rsx! {
        FlatButton {
            variant: FlatButtonVariant::Ghost,
            size: ButtonSize::Icon,
            onclick: move |_| dispatch(state, action.clone()),
            {arkit::icon(icon, 17.0, text_color())}
        }
    }
}

fn destructive_icon_action(icon: &'static str, action: Action, state: Signal<State>) -> Element {
    rsx! {
        FlatButton {
            variant: FlatButtonVariant::Ghost,
            size: ButtonSize::Icon,
            onclick: move |_| dispatch(state, action.clone()),
            {arkit::icon(icon, 17.0, danger())}
        }
    }
}

fn speed_bars(history: &[TrafficHistoryPoint]) -> Element {
    let max = history
        .iter()
        .map(|point| point.download_speed.max(point.upload_speed))
        .max()
        .unwrap_or(1)
        .max(1);
    let bars = history.iter().rev().take(24).rev().enumerate().map(|(index, point)| {
        let ratio = point.download_speed.max(point.upload_speed) as f32 / max as f32;
        rsx! {
            column {
                key: "{index}",
                layout_weight: 1.0,
                height: 56.0,
                justify_content: "end",
                row {
                    width: "72%",
                    height: 3.0 + ratio * 49.0,
                    border_radius: 2.0,
                    background_color: if point.download_speed >= point.upload_speed { success() } else { warning() },
                }
            }
        }
    });
    rsx! {
        row {
            width: "100%",
            height: 62.0,
            margin_top: 10.0,
            padding: 4.0,
            align_items: "end",
            background_color: muted(),
            border_radius: 8.0,
            {bars}
        }
    }
}

fn tr(locale: UiLocale, zh: &'static str, en: &'static str) -> &'static str {
    match locale {
        UiLocale::ZhCn => zh,
        UiLocale::En => en,
    }
}

fn compact(value: &str) -> String {
    let value = value.replace(['\n', '\r'], " ");
    truncate_text(&value, 120)
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let prefix = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

fn middle_truncate_text(value: &str, max_chars: usize) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars || max_chars < 3 {
        return value.to_owned();
    }
    let visible = max_chars - 1;
    let prefix_len = (visible + 1) / 2;
    let suffix_len = visible / 2;
    format!(
        "{}…{}",
        chars[..prefix_len].iter().collect::<String>(),
        chars[chars.len() - suffix_len..].iter().collect::<String>()
    )
}

fn format_speed(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB/s", bytes as f64 / 1024.0 / 1024.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB/s", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B/s")
    }
}

fn format_total(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GB", bytes as f64 / 1024.0 / 1024.0 / 1024.0)
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

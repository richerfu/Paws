use super::*;
use crate::manual_rule::{find_manual_rule_conflict, manual_rule_preview};
use crate::notification::{use_notification_center, NotificationHost};
use crate::platform_callbacks;
use arkit::prelude::*;
use arkit::router::{
    use_back_handler, use_navigator, use_route, AnimatedOutlet, RouteProvider, Router,
};
use arkit::shadcn::components::{
    Badge, BadgeVariant, BottomNavigation, BottomNavigationItem, Button, ButtonSize, ButtonVariant,
    Card, CardContent, CardHeader, CardTitle, DialogFooter, DialogHeader, Field, FieldContent,
    FieldDescription, FieldLabel, FieldOrientation, FieldTitle, Form, FormItem, Input, RadioGroup,
    Select, Separator, Spinner, Switch, Textarea,
};
use arkit::shadcn::theme::{
    spacing, typography, use_theme, Theme, ThemeMode, ThemePreset, ThemeProvider,
};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

#[path = "view/pages/mod.rs"]
mod pages;
#[path = "view/route.rs"]
mod route;

use pages::{
    about_page, appearance_page, connections_page, dashboard_page, logs_page, manual_rule_dialog,
    profiles_page, proxies_page, requests_page, resources_page, settings_page,
    subscription_converter_page, tools_page, traffic_page, yaml_editor_dialog,
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

/// Full-width segmented control in the shadcn ToggleGroup style:
/// muted track, raised active segment, no outer border or divider lines.
#[component]
fn FlatSegmented(props: FlatSegmentedProps) -> Element {
    let theme = use_theme();
    let runtime = arkit::use_runtime_handle();
    let options = props
        .options
        .into_iter()
        .map(|option| {
            let active = option == props.selected;
            let next = option.clone();
            let on_change = props.on_change;
            let runtime = runtime.clone();
            rsx! {
                row {
                    key: "{option}",
                    layout_weight: 1.0,
                    height: "100%",
                    padding_left: 2.0,
                    padding_right: 2.0,
                    button {
                        button_type: "normal",
                        width: "100%",
                        height: 32.0,
                        padding: 0.0,
                        background_color: if active { theme.colors.background } else { 0x00000000 },
                        foreground_color: theme.colors.foreground,
                        border_width: if active { 1.0 } else { 0.0 },
                        border_color: if active { theme.colors.border } else { 0x00000000 },
                        border_radius: theme.radii.md,
                        onclick: move |_| {
                            let next = next.clone();
                            runtime.queue_ui(move || on_change.call(next));
                        },
                        text {
                            content: option,
                            font_size: typography::SM,
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
            height: 40.0,
            padding_left: spacing::XXS,
            padding_right: spacing::XXS,
            align_items: "center",
            border_width: 0.0,
            border_radius: theme.radii.lg,
            background_color: theme.colors.muted,
            clip: true,
            {options.into_iter()}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct FlatDialogProps {
    open: bool,
    /// Retained for callers that use it to identify changing dialog content.
    /// The portal is declarative, so its children now reconcile live without
    /// requiring a snapshot refresh.
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
    let _ = props.content_key;
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
    rsx! {
        ModalPortal {
            open: props.open,
            presentation: ModalPresentation::CenteredDialog,
            dismiss_on_backdrop: true,
            backdrop_color: 0x8000_0000_u32,
            viewport_inset: 8.0,
            on_dismiss: close,
            {panel}
        }
    }
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
    let runtime = arkit::use_runtime_handle();
    let initial_runtime = runtime.clone();
    let state = use_signal(move || State::new(notifications, initial_runtime));
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

    let mut bootstrapped = use_signal(|| false);
    use_effect(move || {
        // One-shot startup dispatch. dioxus 0.7 effects re-run reactively
        // whenever a signal they *read* changes; `run_command` reads `state`
        // and the dispatched bootstrap completion then writes it back, which
        // re-triggered the effect: an unbounded self-referential loop that
        // pegged the UI thread (thousands of reloads per second) and ANR'd
        // on device. The guard makes the effect a no-op after the first run.
        if *bootstrapped.peek() {
            return;
        }
        bootstrapped.set(true);
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
    // Runtime ownership is stable for the lifetime of this root. In
    // particular, do not subscribe the caller's reactive effect to the entire
    // application State merely to obtain its executor: doing so makes every
    // snapshot update rerun the bootstrap effect and multiply polling tasks.
    let runtime = state.peek().runtime.clone();
    let async_runtime = runtime.tokio();
    for future in command.into_futures() {
        let task = async_runtime.spawn(future);
        let ui_runtime = runtime.clone();
        arkit::dioxus_core::spawn_forever(async move {
            if let Ok(action) = task.await {
                ui_runtime.queue_ui(move || dispatch(state, action));
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
                    // RouteProvider records ArkUI's per-frame scroll deltas and
                    // restores the route's saved position when the page is
                    // mounted again after navigation back.
                    RouteProvider {
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
    let runtime = arkit::use_runtime_handle();
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
        use_hook(move || Rc::new(runtime.register_back_handler(registered_handler)));
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

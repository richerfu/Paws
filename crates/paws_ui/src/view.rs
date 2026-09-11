use super::*;
use crate::bridge;
use crate::locale::UiLocale;
use crate::manual_rule::{find_manual_rule_conflict, manual_rule_preview};
use crate::notification::{use_notification_center, NotificationHost};
use crate::reactive_signal::use_tokio_subscription;
use arkit::prelude::*;
use arkit::router::{
    use_back_handler, use_navigator, use_route, AnimatedOutlet, RouteProvider, Router,
};
use arkit::shadcn::components::{
    BottomNavigation, BottomNavigationItem, Button, ButtonSize, ButtonVariant, Card, DialogFooter,
    DialogHeader, Field, FieldContent, FieldDescription, FieldLabel, FieldOrientation, FieldTitle,
    Form, FormItem, Input, RadioGroup, Select, Separator, Spinner, Switch, Textarea,
};
use arkit::shadcn::theme::{
    spacing, typography, use_theme, Theme, ThemeMode, ThemePreset, ThemeProvider,
};
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::rc::Rc;

#[path = "view/pages/mod.rs"]
mod pages;
#[path = "view/route.rs"]
mod route;

use pages::{
    about_page, appearance_page, connections_page, dashboard_page, logs_page, privacy_page,
    profiles_page, proxies_page, requests_page, resources_page, settings_page,
    subscription_converter_page, tools_page, traffic_page, ManualRuleDialog,
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

/// Embedded virtual-list rows do not inherit the page's theme context. Callers
/// therefore provide the resolved color instead of mounting shadcn's
/// theme-aware `Spinner` in the detached row VirtualDom.
fn virtual_loading_indicator(size: f32, color: u32) -> Element {
    rsx! {
        loadingprogress {
            width: size,
            height: size,
            loading_progress_color: color,
            loading_progress_enable_loading: true,
            hit_test_behavior: "transparent",
        }
    }
}

/// Page-scoped query ownership. Read-only work is aborted when its route is
/// disposed; committed mutations are allowed to finish, but their UI callback
/// is discarded after disposal.
#[derive(Clone)]
pub(crate) struct PageTasks {
    runtime: arkit::RuntimeHandle,
    alive: Rc<Cell<bool>>,
    queries: Rc<RefCell<Vec<tokio::task::AbortHandle>>>,
}

fn use_page_tasks() -> PageTasks {
    let runtime = arkit::use_runtime_handle();
    let owner = use_hook(move || PageTasks {
        runtime,
        alive: Rc::new(Cell::new(true)),
        queries: Rc::new(RefCell::new(Vec::new())),
    });
    let disposed = owner.clone();
    use_drop(move || {
        disposed.alive.set(false);
        for task in disposed.queries.borrow_mut().drain(..) {
            task.abort();
        }
    });
    owner.clone()
}

/// Editor state is owned by the route that opened it. Virtual-list callbacks
/// and dialog portals receive it explicitly; disposing the route aborts its
/// lookup query and prevents durable mutation callbacks from writing into a
/// dead Signal.
fn use_local_rule_editors() -> LocalRuleEditors {
    let signal = use_signal(RuleEditorDrafts::default);
    let local = use_hook(move || LocalRuleEditors::new(signal));
    let disposed = local.clone();
    use_drop(move || disposed.dispose());
    local
}

impl PageTasks {
    pub(crate) fn is_alive(&self) -> bool {
        self.alive.get()
    }

    pub(crate) fn query<F, T, C>(&self, future: F, complete: C)
    where
        F: Future<Output = Result<T, String>> + Send + 'static,
        T: Send + 'static,
        C: FnOnce(Result<T, String>) + 'static,
    {
        let task = self.runtime.tokio().spawn(future);
        let mut queries = self.queries.borrow_mut();
        queries.retain(|task| !task.is_finished());
        queries.push(task.abort_handle());
        drop(queries);
        let runtime = self.runtime.clone();
        let alive = self.alive.clone();
        arkit::dioxus_core::spawn_forever(async move {
            let result = match task.await {
                Ok(result) => result,
                Err(error) if error.is_cancelled() => return,
                Err(error) => Err(format!("Background task failed: {error}")),
            };
            runtime.queue_ui(move || {
                if alive.get() {
                    complete(result);
                }
            });
        });
    }

    pub(crate) fn mutate<F, T, C>(&self, future: F, complete: C)
    where
        F: Future<Output = Result<T, String>> + Send + 'static,
        T: Send + 'static,
        C: FnOnce(Result<T, String>) + 'static,
    {
        let task = self.runtime.tokio().spawn(future);
        let runtime = self.runtime.clone();
        let alive = self.alive.clone();
        arkit::dioxus_core::spawn_forever(async move {
            let result = task
                .await
                .map_err(|error| format!("Background task failed: {error}"))
                .and_then(|result| result);
            runtime.queue_ui(move || {
                if alive.get() {
                    complete(result);
                }
            });
        });
    }
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
                        height: 30.0,
                        padding: 0.0,
                        background_color: if active { theme.colors.background } else { 0x00000000 },
                        foreground_color: theme.colors.foreground,
                        border_width: 1.0,
                        border_color: 0x00000000,
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
            height: 36.0,
            padding: 3.0,
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

#[allow(non_snake_case)]
pub(crate) fn App(initial_safe_area: bridge::InitialSafeArea) -> Element {
    use_context_provider(move || initial_safe_area);
    let notifications = use_notification_center();
    let _notifications = use_context_provider(move || notifications);
    let runtime = arkit::use_runtime_handle();
    let stores = use_ui_stores(notifications);
    let _stores = use_context_provider(move || stores);
    let operations = use_ui_operation_stores();
    let _operations = use_context_provider(move || operations);
    let services_runtime = runtime.clone();
    let services = use_hook(move || {
        UiServices::new(services_runtime.clone(), stores, operations, notifications)
    });
    let cleanup_services = services.clone();
    use_drop(move || cleanup_services.cancel_profile_import());
    let context_services = services.clone();
    let _services = use_context_provider(move || context_services);
    let preferences = stores.preferences.read().clone();
    // Provide arkit's component i18n context from the app locale so shadcn
    // components (Select, DatePicker, …) translate instead of falling back
    // to English. The catalog is only read by app-level `t!` messages; shadcn
    // components translate against their own catalog using the locale id.
    static COMPONENT_I18N_CATALOG: arkit::i18n::Catalog = arkit::i18n::Catalog {
        fallback: "en-US",
        locales: &[],
    };
    let i18n_context = arkit::use_i18n_provider(
        &COMPONENT_I18N_CATALOG,
        match preferences.locale {
            UiLocale::ZhCn => "zh-CN",
            UiLocale::En => "en-US",
        },
    );
    let color_mode_runtime = runtime.clone();
    use_effect(move || {
        let locale_id = match stores.preferences.read().locale {
            UiLocale::ZhCn => "zh-CN",
            UiLocale::En => "en-US",
        };
        if i18n_context.locale_id() != locale_id {
            i18n_context.set_locale_id(locale_id);
        }
    });
    let theme = if preferences.dark {
        Theme::dark(ThemePreset::Zinc)
    } else {
        Theme::light(ThemePreset::Zinc)
    };
    use_effect(move || {
        let current = stores.preferences.read().clone();
        let color_mode = current.theme.platform_color_mode();
        if current.applied_color_mode == Some(color_mode)
            || current.pending_color_mode == Some(color_mode)
            || current.color_mode_error.is_some()
        {
            return;
        }
        stores.set_preferences(PreferencesProjection {
            pending_color_mode: Some(color_mode),
            ..current
        });
        let runtime = color_mode_runtime.clone();
        arkit::dioxus_core::spawn_forever(async move {
            let result = bridge::set_color_mode(color_mode).await;
            runtime.queue_ui(move || {
                let current = stores.preferences.peek().clone();
                if current.theme.platform_color_mode() != color_mode {
                    if current.pending_color_mode == Some(color_mode) {
                        stores.set_preferences(PreferencesProjection {
                            pending_color_mode: None,
                            ..current
                        });
                    }
                    return;
                }
                match result {
                    Ok(()) => stores.set_preferences(PreferencesProjection {
                        applied_color_mode: Some(color_mode),
                        pending_color_mode: None,
                        color_mode_error: None,
                        ..current
                    }),
                    Err(error) => {
                        notifications.publish(format!(
                            "{}: {error}",
                            translate_ui(current.locale, tr::appearance_color_mode_failed())
                        ));
                        stores.set_preferences(PreferencesProjection {
                            pending_color_mode: None,
                            color_mode_error: Some(error),
                            ..current
                        });
                    }
                }
            });
        });
    });

    let bootstrap_tokio = runtime.tokio().clone();
    let _bootstrap = use_future(move || {
        // Bootstrap can cross a configuration commit boundary. Spawn it on
        // the application runtime and intentionally detach it if the UI root
        // is disposed; only this local completion observer is scope-owned.
        let task = bootstrap_tokio.spawn(bootstrap_active_profile());
        async move {
            let result = task
                .await
                .map_err(|error| format!("Background task failed: {error}"))
                .and_then(|result| result);
            match result {
                Ok(()) => stores.set_bootstrap_error(None),
                Err(error) => {
                    let previous = stores.session.peek().bootstrap_error.clone();
                    stores.set_bootstrap_error(Some(error.clone()));
                    if previous.as_deref() != Some(error.as_str()) {
                        notifications.publish(error);
                    }
                }
            }
        }
    });
    use_runtime_projection_provider(stores, notifications, runtime.tokio().clone());
    use_system_preferences_provider(stores, runtime.tokio().clone());

    rsx! {
        // The full Arkit tree remains edge-to-edge. AppShell applies the safe
        // area to business layout; root portals such as NotificationHost read
        // the same window metrics independently and therefore avoid only once.
        ThemeProvider {
            theme,
            Router::<Route> {}
            NotificationHost { center: notifications }
        }
    }
}

/// Messages cross the Tokio/UI boundary as owned data. Dioxus signals and the
/// non-Send runtime handle stay on the UI executor.
enum RuntimeProjectionUpdate {
    Config(Result<paws_core::ConfigProjection, String>),
    Telemetry(Result<paws_core::TelemetryProjection, String>),
    Status(Result<paws_core::RuntimeStatusProjection, String>),
    Resources(Result<paws_core::ResourceProjection, String>),
    ClearError(String),
}

/// A root-owned subscription replaces the UI's one-second full-snapshot
/// polling loop. Its Tokio task is aborted when this Dioxus scope disappears.
fn use_runtime_projection_provider(
    stores: UiStores,
    notifications: NotificationCenter,
    tokio: tokio::runtime::Handle,
) {
    let initial_revisions = stores.runtime_revisions();
    use_tokio_subscription(
        tokio,
        move |updates| {
            let mut applied = initial_revisions;
            async move {
                let core = paws_core::shared_core();
                let mut revisions = core.subscribe_runtime_revisions();
                let mut last_projection_error = None::<String>;
                loop {
                    let announced = *revisions.borrow_and_update();
                    let mut attempted = false;
                    let mut failed = false;

                    if announced.config_revision != applied.config_revision {
                        attempted = true;
                        let result = core
                            .config_projection()
                            .map_err(|error| error.to_string())
                            .and_then(|projection| {
                                (projection.revisions.config_revision
                                    >= announced.config_revision)
                                    .then_some(projection)
                                    .ok_or_else(|| {
                                        "configuration projection is older than its announced revision"
                                            .to_owned()
                                    })
                            });
                        match &result {
                            Ok(projection)
                                if projection.revisions.config_revision
                                    >= announced.config_revision =>
                            {
                                applied.config_revision = projection.revisions.config_revision;
                            }
                            Ok(_) => failed = true,
                            Err(_) => failed = true,
                        }
                        if let Err(error) = &result {
                            last_projection_error = Some(error.clone());
                        }
                        if updates
                            .send(RuntimeProjectionUpdate::Config(result))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    if announced.telemetry_revision != applied.telemetry_revision {
                        attempted = true;
                        let result = core
                            .telemetry_projection()
                            .map_err(|error| error.to_string())
                            .and_then(|projection| {
                                (projection.revisions.telemetry_revision
                                    >= announced.telemetry_revision)
                                    .then_some(projection)
                                    .ok_or_else(|| {
                                        "telemetry projection is older than its announced revision"
                                            .to_owned()
                                    })
                            });
                        match &result {
                            Ok(projection)
                                if projection.revisions.telemetry_revision
                                    >= announced.telemetry_revision =>
                            {
                                applied.telemetry_revision =
                                    projection.revisions.telemetry_revision;
                            }
                            Ok(_) => failed = true,
                            Err(_) => failed = true,
                        }
                        if let Err(error) = &result {
                            last_projection_error = Some(error.clone());
                        }
                        if updates
                            .send(RuntimeProjectionUpdate::Telemetry(result))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    if announced.status_revision != applied.status_revision {
                        attempted = true;
                        let result = core
                            .runtime_status_projection()
                            .map_err(|error| error.to_string())
                            .and_then(|projection| {
                                (projection.revisions.status_revision
                                    >= announced.status_revision)
                                    .then_some(projection)
                                    .ok_or_else(|| {
                                        "runtime status projection is older than its announced revision"
                                            .to_owned()
                                    })
                            });
                        match &result {
                            Ok(projection)
                                if projection.revisions.status_revision
                                    >= announced.status_revision =>
                            {
                                applied.status_revision = projection.revisions.status_revision;
                            }
                            Ok(_) => failed = true,
                            Err(_) => failed = true,
                        }
                        if let Err(error) = &result {
                            last_projection_error = Some(error.clone());
                        }
                        if updates
                            .send(RuntimeProjectionUpdate::Status(result))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    if announced.resource_revision != applied.resource_revision {
                        attempted = true;
                        let result = core
                            .resource_projection()
                            .map_err(|error| error.to_string())
                            .and_then(|projection| {
                                (projection.revisions.resource_revision
                                    >= announced.resource_revision)
                                    .then_some(projection)
                                    .ok_or_else(|| {
                                        "resource projection is older than its announced revision"
                                            .to_owned()
                                    })
                            });
                        match &result {
                            Ok(projection)
                                if projection.revisions.resource_revision
                                    >= announced.resource_revision =>
                            {
                                applied.resource_revision = projection.revisions.resource_revision;
                            }
                            Ok(_) => failed = true,
                            Err(_) => failed = true,
                        }
                        if let Err(error) = &result {
                            last_projection_error = Some(error.clone());
                        }
                        if updates
                            .send(RuntimeProjectionUpdate::Resources(result))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    if attempted && !failed {
                        if let Some(error) = last_projection_error.take() {
                            if updates
                                .send(RuntimeProjectionUpdate::ClearError(error))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                    if failed {
                        tokio::select! {
                            changed = revisions.changed() => {
                                if changed.is_err() { break; }
                            }
                            _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {}
                        }
                    } else if revisions.changed().await.is_err() {
                        break;
                    }
                }
            }
        },
        move |update| match update {
            RuntimeProjectionUpdate::Config(result) => apply_projection_result(
                result,
                stores,
                notifications,
                UiStores::apply_config_projection,
            ),
            RuntimeProjectionUpdate::Telemetry(result) => apply_projection_result(
                result,
                stores,
                notifications,
                UiStores::apply_telemetry_projection,
            ),
            RuntimeProjectionUpdate::Status(result) => apply_projection_result(
                result,
                stores,
                notifications,
                UiStores::apply_status_projection,
            ),
            RuntimeProjectionUpdate::Resources(result) => apply_projection_result(
                result,
                stores,
                notifications,
                UiStores::apply_resource_projection,
            ),
            RuntimeProjectionUpdate::ClearError(error) => {
                if stores.session.peek().runtime_error.as_deref() == Some(error.as_str()) {
                    stores.clear_runtime_error();
                }
            }
        },
    );
}

fn apply_projection_result<T>(
    result: Result<T, String>,
    stores: UiStores,
    notifications: NotificationCenter,
    apply: fn(UiStores, T) -> bool,
) {
    match result {
        Ok(projection) => {
            apply(stores, projection);
        }
        Err(error) => {
            let previous = stores.session.peek().runtime_error.clone();
            stores.set_runtime_error(error.clone());
            if previous.as_deref() != Some(error.as_str()) {
                notifications.publish(error);
            }
        }
    }
}

fn use_system_preferences_provider(stores: UiStores, tokio: tokio::runtime::Handle) {
    use_tokio_subscription(
        tokio,
        move |updates| async move {
            let mut preferences = crate::system_preferences::subscribe();
            loop {
                let system = preferences.borrow_and_update().clone();
                if updates.send(system).await.is_err() {
                    break;
                }
                if preferences.changed().await.is_err() {
                    break;
                }
            }
        },
        move |system| {
            let current = stores.preferences.peek().clone();
            stores.set_preferences(PreferencesProjection {
                locale: current.language.resolve(&system.locale),
                dark: current.theme.resolve_dark(system.color_mode),
                ..current
            });
        },
    );
}

#[component]
fn AppShell() -> Element {
    let initial_safe_area = use_context::<bridge::InitialSafeArea>().0;
    let window_metrics = arkit::use_window_metrics();
    let safe_area = if window_metrics.content_rect.is_empty() {
        initial_safe_area
    } else {
        window_metrics.safe_area
    };
    let _back_handler = use_back_handler();

    rsx! {
        stack {
            width: "100%",
            height: "100%",
            background_color: bg(),
            alignment: "top-start",
            padding_top: safe_area.top,
            padding_right: safe_area.right,
            padding_bottom: safe_area.bottom,
            padding_left: safe_area.left,
            column {
                width: "100%",
                height: "100%",
                column {
                    layout_weight: 1.0,
                    width: "100%",
                    AnimatedOutlet::<Route> {}
                }
                BottomBar {}
            }
            DashboardVpnFloatingAction {}
        }
    }
}

#[component]
fn BottomBar() -> Element {
    let locale = use_context::<UiStores>().preferences.read().locale;
    let route = use_route::<Route>();
    let navigator = use_navigator();
    if route.parent().is_some() {
        return rsx! {};
    }
    let nav_items = Route::bottom_routes()
        .iter()
        .map(|route| BottomNavigationItem::new(route.title(locale), route.icon()))
        .collect::<Vec<_>>();
    rsx! {
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

#[component]
fn DashboardVpnFloatingAction() -> Element {
    let route = use_route::<Route>();
    let session = use_context::<UiStores>().session.read().clone();
    if !matches!(route, Route::Dashboard {}) {
        return rsx! {};
    }
    rsx! { VpnFloatingAction { session } }
}

#[component]
fn VpnFloatingAction(session: SessionProjection) -> Element {
    let services = use_context::<UiServices>();
    let theme = use_theme();
    let operation = use_context::<UiOperationStores>().vpn.read().active.clone();
    let starting = operation.as_ref().map_or_else(
        || matches!(session.lifecycle, VpnLifecycle::Starting),
        |operation| {
            operation.phase != VpnOperationPhase::Unconfirmed
                && matches!(
                    operation.action,
                    VpnCommandAction::Start | VpnCommandAction::Restart
                )
        },
    );
    let stopping = operation.as_ref().is_some_and(|operation| {
        operation.phase != VpnOperationPhase::Unconfirmed
            && matches!(
                operation.action,
                VpnCommandAction::Stop | VpnCommandAction::OwnedStop
            )
    });
    let active = session.vpn_running && !stopping;
    let disabled = operation.is_some() || matches!(session.lifecycle, VpnLifecycle::Starting);
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
                onclick: move |_| services.toggle_vpn(),
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

fn scaffold(page: Route, actions: Element, body: Element) -> Element {
    scaffold_layout(page, actions, body, true, false)
}

fn fixed_scaffold(page: Route, actions: Element, body: Element) -> Element {
    scaffold_layout(page, actions, body, false, false)
}

fn fixed_scaffold_flush_bottom(page: Route, actions: Element, body: Element) -> Element {
    scaffold_layout(page, actions, body, false, true)
}

fn scaffold_layout(
    page: Route,
    actions: Element,
    body: Element,
    scrollable: bool,
    flush_fixed_bottom: bool,
) -> Element {
    let locale = use_context::<UiStores>().preferences.read().locale;
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
                padding_left: spacing::LG,
                padding_right: spacing::LG,
                align_items: "center",
                background_color: theme.colors.background,
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
                        content: page.title(locale),
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
    let theme = use_theme();
    rsx! {
        Card {
            shadow: Some(false),
            column {
                width: "100%",
                padding: spacing::LG,
                align_items: "start",
                text {
                    content: title,
                    font_size: typography::SM,
                    line_height: 20.0,
                    font_weight: 600,
                    font_color: theme.colors.card_foreground,
                    text_letter_spacing: -0.2,
                }
                if let Some(subtitle) = subtitle {
                    text {
                        content: subtitle,
                        margin_top: spacing::XXS,
                        font_size: typography::XS,
                        line_height: 18.0,
                        font_color: theme.colors.muted_foreground,
                    }
                }
                column {
                    width: "100%",
                    margin_top: spacing::MD,
                    {body}
                }
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
                        text { content: download_value, margin_top: 2.0, font_size: typography::LG, line_height: 24.0, font_weight: 600, font_color: theme.colors.foreground }
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
                        text { content: upload_value, margin_top: 2.0, font_size: typography::LG, line_height: 24.0, font_weight: 600, font_color: theme.colors.foreground }
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
    let theme = use_theme();
    rsx! {
        row {
            width: "100%",
            height: 32.0,
            align_items: "center",
            text {
                content: label,
                font_size: typography::XS,
                line_height: 18.0,
                font_color: theme.colors.muted_foreground,
            }
            row {
                layout_weight: 1.0,
                margin_left: spacing::MD,
                justify_content: "end",
                text {
                    width: "100%",
                    content: value,
                    font_size: typography::SM,
                    line_height: 20.0,
                    font_weight: 500,
                    font_color: theme.colors.foreground,
                    max_lines: 1,
                    text_align: "end",
                }
            }
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
                font_size: typography::SM,
                line_height: 20.0,
                font_weight: 500,
                font_color: theme.colors.muted_foreground,
            }
        }
    }
}

fn field_label(label: impl Into<String>) -> Element {
    let label = label.into();
    let theme = use_theme();
    rsx! {
        text {
            content: label,
            font_size: typography::XS,
            line_height: 16.0,
            font_weight: 500,
            font_color: theme.colors.muted_foreground,
        }
    }
}

fn status_chip(label: impl Into<String>, color: u32) -> Element {
    let label = label.into();
    let theme = use_theme();
    rsx! {
        row {
            height: 24.0,
            padding_left: spacing::SM,
            padding_right: spacing::SM,
            align_items: "center",
            background_color: theme.colors.muted,
            border_radius: theme.radii.full,
            row {
                width: 6.0,
                height: 6.0,
                border_radius: 3.0,
                background_color: color,
            }
            text {
                content: label,
                margin_left: spacing::XS,
                font_size: typography::XS,
                line_height: 16.0,
                font_weight: 500,
                font_color: color,
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
                    {arkit::icon(icon, 20.0, theme.colors.muted_foreground)}
                }
                text {
                    content: title,
                    margin_top: spacing::MD,
                    font_size: typography::SM,
                    line_height: 20.0,
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
            if index + 1 < len { row { height: spacing::MD } }
        }
    });
    rsx! { column { width: "100%", {nodes} } }
}

fn speed_bars(history: &[TrafficHistoryPoint]) -> Element {
    let theme = use_theme();
    let max = history
        .iter()
        .map(|point| point.download_speed.max(point.upload_speed))
        .max()
        .unwrap_or(0)
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
            margin_top: spacing::SM,
            padding: spacing::XXS,
            align_items: "end",
            background_color: muted(),
            border_radius: theme.radii.lg,
            {bars}
        }
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
    let prefix_len = visible.div_ceil(2);
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

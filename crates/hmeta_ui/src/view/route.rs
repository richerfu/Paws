use super::*;
use arkit::dioxus_core::VNode;
use arkit::router::dioxus_router;
use arkit::router::Routable;

#[derive(Routable, Clone, PartialEq, Debug)]
pub(super) enum Route {
    #[layout(AppShell)]
    #[route("/")]
    Dashboard {},
    #[route("/subscriptions/proxies")]
    Proxies {},
    #[route("/subscriptions")]
    Profiles {},
    #[route("/traffic")]
    Traffic {},
    #[route("/settings")]
    Tools {},
    #[route("/settings/appearance")]
    Appearance {},
    #[route("/settings/network")]
    Settings {},
    #[route("/settings/subscription-converter")]
    SubscriptionConverter {},
    #[route("/settings/requests")]
    Requests {},
    #[route("/settings/connections?:query")]
    Connections { query: String },
    #[route("/settings/resources")]
    Resources {},
    #[route("/settings/logs")]
    Logs {},
    #[route("/settings/about")]
    About {},
}

impl Route {
    pub(super) fn title(&self, locale: UiLocale) -> &'static str {
        let strings = strings(locale);
        match self {
            Self::Dashboard {} => strings.nav_dashboard,
            Self::Proxies {} => strings.nav_proxies,
            Self::Profiles {} => strings.nav_profiles,
            Self::Requests {} => strings.nav_requests,
            Self::Connections { .. } => strings.nav_connections,
            Self::Traffic {} => strings.nav_traffic,
            Self::Resources {} => strings.nav_resources,
            Self::Logs {} => strings.nav_logs,
            Self::Tools {} => strings.nav_tools,
            Self::Appearance {} => tr(locale, "界面设置", "Appearance"),
            Self::Settings {} => strings.nav_settings,
            Self::SubscriptionConverter {} => tr(locale, "订阅转化规则", "Subscription conversion"),
            Self::About {} => strings.nav_about,
        }
    }

    pub(super) fn icon(&self) -> &'static str {
        match self {
            Self::Dashboard {} => "compass",
            Self::Proxies {} => "git-branch",
            Self::Profiles {} => "rss",
            Self::Requests {} => "activity",
            Self::Connections { .. } => "unplug",
            Self::Traffic {} => "activity",
            Self::Resources {} => "database",
            Self::Logs {} => "scroll-text",
            Self::Tools {} => "settings",
            Self::Appearance {} => "palette",
            Self::Settings {} => "settings",
            Self::SubscriptionConverter {} => "refresh-cw",
            Self::About {} => "badge-info",
        }
    }

    pub(super) fn bottom_index(&self) -> usize {
        match self {
            Self::Dashboard {} => 0,
            Self::Profiles {} | Self::Proxies {} => 1,
            Self::Traffic {} => 2,
            Self::Tools {}
            | Self::Appearance {}
            | Self::Settings {}
            | Self::SubscriptionConverter {}
            | Self::Requests {}
            | Self::Connections { .. }
            | Self::Resources {}
            | Self::Logs {}
            | Self::About {} => 3,
        }
    }

    pub(super) fn parent(&self) -> Option<Self> {
        match self {
            Self::Proxies {} => Some(Self::Profiles {}),
            Self::Appearance {}
            | Self::Settings {}
            | Self::SubscriptionConverter {}
            | Self::Requests {}
            | Self::Connections { .. }
            | Self::Resources {}
            | Self::Logs {}
            | Self::About {} => Some(Self::Tools {}),
            _ => None,
        }
    }

    pub(super) fn bottom_routes() -> [Self; 4] {
        [
            Self::Dashboard {},
            Self::Profiles {},
            Self::Traffic {},
            Self::Tools {},
        ]
    }
}

fn state() -> Signal<State> {
    use_context::<Signal<State>>()
}

#[component]
fn Dashboard() -> Element {
    dashboard_page(state())
}

#[component]
fn Proxies() -> Element {
    proxies_page(state())
}

#[component]
fn Profiles() -> Element {
    profiles_page(state())
}

#[component]
fn Requests() -> Element {
    requests_page(state())
}

#[component]
fn Connections(query: String) -> Element {
    connections_page(state(), query)
}

#[component]
fn Traffic() -> Element {
    traffic_page(state())
}

#[component]
fn Resources() -> Element {
    resources_page(state())
}

#[component]
fn Logs() -> Element {
    logs_page(state())
}

#[component]
fn Tools() -> Element {
    tools_page(state())
}

#[component]
fn Settings() -> Element {
    settings_page(state())
}

#[component]
fn SubscriptionConverter() -> Element {
    subscription_converter_page(state())
}

#[component]
fn Appearance() -> Element {
    appearance_page(state())
}

#[component]
fn About() -> Element {
    about_page(state())
}

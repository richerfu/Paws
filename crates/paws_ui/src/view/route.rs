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
    #[route("/settings/about/privacy")]
    Privacy {},
}

impl Route {
    pub(super) fn title(&self, locale: UiLocale) -> String {
        match self {
            Self::Dashboard {} => translate_ui(locale, tr::nav_dashboard()),
            Self::Proxies {} => translate_ui(locale, tr::nav_proxies()),
            Self::Profiles {} => translate_ui(locale, tr::nav_profiles()),
            Self::Requests {} => translate_ui(locale, tr::nav_requests()),
            Self::Connections { .. } => translate_ui(locale, tr::nav_connections()),
            Self::Traffic {} => translate_ui(locale, tr::nav_traffic()),
            Self::Resources {} => translate_ui(locale, tr::nav_resources()),
            Self::Logs {} => translate_ui(locale, tr::nav_logs()),
            Self::Tools {} => translate_ui(locale, tr::nav_tools()),
            Self::Appearance {} => translate_ui(locale, tr::page_tr_001()),
            Self::Settings {} => translate_ui(locale, tr::nav_settings()),
            Self::SubscriptionConverter {} => translate_ui(locale, tr::page_tr_002()),
            Self::About {} => translate_ui(locale, tr::nav_about()),
            Self::Privacy {} => translate_ui(locale, tr::page_tr_003()),
        }
    }

    pub(super) fn icon(&self) -> &'static str {
        match self {
            Self::Dashboard {} => "house",
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
            Self::About {} => "paw-print",
            Self::Privacy {} => "shield-check",
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
            | Self::About {}
            | Self::Privacy {} => 3,
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
            Self::Privacy {} => Some(Self::About {}),
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

#[component]
fn Dashboard() -> Element {
    dashboard_page()
}

#[component]
fn Proxies() -> Element {
    proxies_page()
}

#[component]
fn Profiles() -> Element {
    profiles_page()
}

#[component]
fn Requests() -> Element {
    requests_page()
}

#[component]
fn Connections(query: String) -> Element {
    connections_page(query)
}

#[component]
fn Traffic() -> Element {
    traffic_page()
}

#[component]
fn Resources() -> Element {
    resources_page()
}

#[component]
fn Logs() -> Element {
    logs_page()
}

#[component]
fn Tools() -> Element {
    tools_page()
}

#[component]
fn Settings() -> Element {
    settings_page()
}

#[component]
fn SubscriptionConverter() -> Element {
    subscription_converter_page()
}

#[component]
fn Appearance() -> Element {
    appearance_page()
}

#[component]
fn About() -> Element {
    about_page()
}

#[component]
fn Privacy() -> Element {
    privacy_page()
}

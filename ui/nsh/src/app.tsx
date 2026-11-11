import { density } from "./theme";
import {
  announce,
  cleanup,
  computed,
  effect,
  fontScale,
  signal,
  Text,
  untracked,
  View,
  type JSX,
  setColorScheme,
  windowSize,
  keymap,
  platform,
  reduceMotion,
} from "nsh";
import { BackHandler } from "nsh/mobile";
import {
  Tab,
  Tabs as BaseTabs,
  createNavStack,
  NavPage,
  NavStack,
} from "nsh/base";
import { CoreGateway, type CoreClient } from "./client";
import {
  App,
  Action,
  Actions,
  Tabs,
  Loading,
  control,
  Alert,
  confirm,
  Icon,
  IconButton,
  palette,
  radius,
  spacing,
  translator,
  type AppContext,
  type Page,
  useLifetime,
  useApp,
  RouteOwner,
} from "./components";
import { feedbackStyle } from "./motion";
import { chrome, focusRing } from "./theme";
import {
  DesktopNavigation,
  desktopPrimary,
  desktopRootPages,
} from "./desktop-navigation";
import { Dashboard, Traffic } from "./pages/overview";
import { FloatingVpnPowerButton } from "./floating-vpn";
import { Profiles } from "./pages/profiles";
import { ProfileEditor } from "./pages/profile-editor";
import { Proxies } from "./pages/proxies";
import { Activity } from "./pages/activity";
import { Resources } from "./pages/resources";
import { Appearance, Settings } from "./pages/settings";
import { Logs } from "./pages/logs";
import {
  About,
  Backups,
  Diagnostics,
  Tools,
  Privacy,
  AdvancedDiagnostics,
} from "./pages/tools";
import { Converter } from "./pages/converter";
import { COPY } from "./product-copy";
import type { IconName } from "./icons";
import {
  NotificationCenter,
  Notifications,
  NotificationViewport,
} from "./notifications";
import { pageDomains } from "./page-data";
import { PageError } from "./page-error";
import { isChinese } from "./locale";
import type { DomainInput } from "./model";

const primary: { page: Page; zh: string; en: string; icon: IconName }[] = [
  { page: "dashboard", zh: "首页", en: "Home", icon: "house" },
  { page: "profiles", zh: "订阅", en: "Subscriptions", icon: "rss" },
  { page: "traffic", zh: "流量", en: "Traffic", icon: "activity" },
  { page: "tools", zh: "设置", en: "Settings", icon: "settings" },
];
const titles: Record<Page, [string, string]> = {
  dashboard: ["首页", "Home"],
  profiles: ["订阅", "Subscriptions"],
  proxies: ["代理节点", "Proxy nodes"],
  traffic: ["流量", "Traffic"],
  tools: ["设置", "Settings"],
  settings: ["网络设置", "Network settings"],
  appearance: [...COPY["page-tr-001"]],
  connections: ["连接", "Connections"],
  requests: ["请求", "Requests"],
  resources: ["资源", "Resources"],
  logs: ["日志", "Logs"],
  diagnostics: [...COPY["diagnostics-title"]],
  "advanced-diagnostics": ["高级诊断", "Advanced diagnostics"],
  backups: ["备份与恢复", "Backup & restore"],
  converter: [...COPY["page-tr-002"]],
  about: ["关于", "About"],
  privacy: [...COPY["page-tr-003"]],
  editor: ["编辑配置", "Edit profile"],
};
export function parentPage(page: Page): Page | undefined {
  if (primary.some((item) => item.page === page)) return undefined;
  return page === "proxies" || page === "editor"
    ? "profiles"
    : page === "privacy"
      ? "about"
      : "tools";
}
export function pageMeasure(page: Page, wide: boolean): number {
  return !wide
    ? 640
    : page === "dashboard"
      ? 960
      : page === "profiles" || page === "proxies"
        ? 1120
        : page === "traffic"
          ? 1040
          : ["requests", "connections", "resources", "logs"].includes(page)
            ? 1180
            : 840;
}

export function PawsApp(props: {
  client: CoreClient;
  embeddedGateway?: boolean;
}) {
  const notifications = new NotificationCenter();
  for (const [source, tone] of [
    [props.client.error, "danger"],
    [props.client.warning, "warning"],
    [props.client.notice, "info"],
  ] as const)
    effect(() => {
      const message = source();
      if (!message) return;
      untracked(() => {
        notifications.publish(message, tone);
        if (tone !== "info") announce(message);
      });
    });
  cleanup(() => notifications.dispose());
  const nav = createNavStack<{ name: Page; query?: string }>([
    { name: "dashboard" },
  ]);
  const alive = useLifetime(),
    importIntent = signal(0),
    resourceIntent: AppContext["resourceIntent"] = signal(undefined),
    proxyDelayLoading = signal(false);
  const leaveGuard = signal<
    { route: unknown; check: () => boolean | Promise<boolean> } | undefined
  >();
  let leaving = false;
  let importConsumed = 0,
    resourceConsumed = 0,
    resourceSequence = 0;
  const logRecording = signal(false),
    logRecordingIntent = signal(0);
  const route = () => nav.routes()[nav.routes().length - 1],
    page = () => route().name;
  const requestNavigation = (action: () => void) => {
    const guard = leaveGuard();
    if (!guard || guard.route !== route()) {
      action();
      return;
    }
    if (leaving) return;
    const current = route(),
      result = guard.check();
    if (typeof result === "boolean") {
      if (result) action();
      return;
    }
    leaving = true;
    void result
      .then((accepted) => {
        if (accepted && alive() && route() === current) action();
      })
      .finally(() => {
        leaving = false;
      });
  };
  const navigate = (next: Page, query?: string) =>
    requestNavigation(() => {
      if (
        (platform.desktop && desktopRootPages.includes(next)) ||
        primary.some((item) => item.page === next)
      )
        nav.setRoutes([{ name: next, query }]);
      else nav.push({ name: next, query });
    });
  const back = () =>
    requestNavigation(() => {
      if (nav.routes().length > 1) nav.pop();
      else {
        const parent = parentPage(page());
        if (parent) nav.setRoutes([{ name: parent }]);
      }
    });
  const t = translator(props.client);
  const refreshProfiles = async () => {
    if (
      props.client.pending() > 0 ||
      !props.client.domains.readyForCommand(["config", "profiles"])
    )
      return;
    const attempted = new Set(
      props.client
        .state()
        ?.profiles.filter((profile) => profile.subscriptionUrl)
        .map((profile) => profile.id),
    );
    const result = await props.client.run("profiles.refreshAll");
    if (
      !alive() ||
      result === undefined ||
      props.client.warning() ||
      !props.client.domains.readyForCommand(["config", "profiles"])
    )
      return;
    const failed = props.client
      .state()!
      .profiles.filter(
        (profile) => attempted.has(profile.id) && profile.lastRefreshError,
      ).length;
    props.client.notice.set(
      attempted.size
        ? t(
            `全部订阅刷新完成：${attempted.size - failed} 成功，${failed} 失败`,
            `Subscriptions refreshed: ${attempted.size - failed} succeeded, ${failed} failed`,
          )
        : t("全部订阅没有可刷新的订阅", "No subscriptions to refresh"),
    );
  };
  const ui: AppContext = {
    client: props.client,
    page,
    alive,
    route,
    navigate,
    back,
    query: () => route().query || "",
    importIntent,
    consumeImportIntent: (id) => {
      if (id <= importConsumed) return false;
      importConsumed = id;
      return true;
    },
    resourceIntent,
    consumeResourceIntent: (id) => {
      if (id <= resourceConsumed) return false;
      resourceConsumed = id;
      return true;
    },
    logRecording,
    logRecordingIntent,
    refreshProfiles,
    t,
    editProfile: (id) => navigate("editor", id),
    beforeLeave: (check) => {
      const guard = { route: route(), check };
      leaveGuard.set(guard);
      return () => {
        if (leaveGuard() === guard) leaveGuard.set(undefined);
      };
    },
  };
  const wide = () => platform.desktop || windowSize().width >= 840;
  const canBack = () =>
    nav.routes().length > 1 || (!platform.desktop && !!parentPage(page()));
  const group = () =>
    page() === "proxies" || page() === "editor"
      ? "profiles"
      : primary.some((tab) => tab.page === page())
        ? page()
        : "tools";
  const theme = computed(() => props.client.preferences().theme);
  effect(() => {
    const current = page();
    untracked(() => props.client.domains.setPage(pageDomains(current)));
  });
  effect(() => {
    setColorScheme(theme());
  });
  const stopHardwareBack = BackHandler.on("press", () => {
    if (!canBack()) return false;
    back();
    return true;
  });
  cleanup(() => {
    stopHardwareBack();
  });
  if (platform.desktop) {
    for (let index = 0; index < desktopPrimary.length; index++)
      cleanup(
        keymap.bind(`mod+${index + 1}`, () =>
          navigate(desktopPrimary[index].page),
        ),
      );
    cleanup(keymap.bind("mod+,", () => navigate("appearance")));
    effect(() => {
      if (page() === "editor") return;
      cleanup(
        keymap.bind("mod+r", () => {
          if (page() === "profiles") void refreshProfiles();
          else props.client.domains.retry(pageDomains(page()));
        }),
      );
    });
  }
  const Shell = () => {
    const content = (
      <>
        <If when={props.embeddedGateway !== false}>
          <CoreGateway client={props.client} />
        </If>
        <View style={{ flex: 1, flexDirection: wide() ? "row" : "column" }}>
          <If when={wide()}>
            <DesktopNavigation
              page={page()}
              navigate={navigate}
              t={ui.t}
              version={props.client.domains.data("summary")?.appVersion || ""}
            />
          </If>
          <View style={{ flex: 1, minWidth: 0 }}>
            <View
              style={{
                minHeight: Math.max(
                  chrome.header,
                  density.line(28) * Math.max(1, fontScale()) + spacing.lg,
                ),
                borderBottomWidth: 1,
                borderBottomColor: palette().border,
                alignItems: "center",
                justifyContent: "center",
              }}
            >
              <View
                style={{
                  width: "100%",
                  maxWidth: pageMeasure(page(), wide()),
                  paddingHorizontal: wide() ? spacing.xxl : spacing.lg,
                  flexDirection: "row",
                  alignItems: "center",
                  gap: spacing.xxs,
                }}
              >
                <If when={canBack()}>
                  <IconButton
                    id="nav-back"
                    icon="arrow-left"
                    label={ui.t("返回", "Back")}
                    onPress={back}
                  />
                </If>
                <Text
                  role="header"
                  style={{
                    flex: 1,
                    color: palette().text,
                    fontSize: density.font(20),
                    lineHeight: density.line(28),
                    fontWeight: "600",
                    letterSpacing: isChinese(props.client.preferences())
                      ? 0
                      : -0.3,
                  }}
                >
                  {platform.desktop &&
                  (page() === "connections" || page() === "requests")
                    ? ui.t("连接与请求", "Network activity")
                    : ui.t(...titles[page()])}
                </Text>
                <If when={page() === "profiles"}>
                  <IconButton
                    id="refresh-profiles"
                    icon="refresh-cw"
                    shortcut="mod+r"
                    label={ui.t("全部更新", "Refresh all")}
                    loading={props.client.isPending("profiles.refreshAll")}
                    disabled={
                      props.client.pending() > 0 ||
                      !props.client.domains.readyForCommand([
                        "config",
                        "profiles",
                      ])
                    }
                    onPress={() => void refreshProfiles()}
                  />
                  <IconButton
                    id="add-profile"
                    icon="plus"
                    label={ui.t("添加订阅", "Add subscription")}
                    onPress={() => importIntent.set(importIntent() + 1)}
                  />
                </If>
                <If when={page() === "proxies"}>
                  <IconButton
                    icon="zap"
                    label={ui.t("测试延迟", "Test latency")}
                    loading={proxyDelayLoading()}
                    disabled={
                      proxyDelayLoading() ||
                      !props.client.domains.fresh(["proxies"])
                    }
                    onPress={() => {
                      if (proxyDelayLoading()) return;
                      proxyDelayLoading.set(true);
                      void (async () => {
                        try {
                          for (const group of props.client.state()!
                            .proxyGroups) {
                            if (!alive()) return;
                            await props.client.run("proxies.testGroup", {
                              group: group.name,
                            });
                          }
                        } finally {
                          if (alive()) proxyDelayLoading.set(false);
                        }
                      })();
                    }}
                  />
                </If>
                <If when={page() === "requests" || page() === "connections"}>
                  <IconButton
                    icon="trash-2"
                    label={
                      page() === "requests"
                        ? ui.t("清空记录", "Clear history")
                        : ui.t("关闭全部", "Close all")
                    }
                    disabled={
                      props.client.pending() > 0 ||
                      !props.client.domains.fresh([
                        page() === "requests" ? "requests" : "connections",
                      ])
                    }
                    onPress={() => {
                      const history = page() === "requests";
                      void confirm(
                        ui,
                        history
                          ? ui.t("清空请求记录？", "Clear request history?")
                          : ui.t(
                              "关闭全部活动连接？",
                              "Close all active connections?",
                            ),
                        () =>
                          void props.client.run(
                            history ? "requests.clear" : "connections.closeAll",
                          ),
                        {
                          actionLabel: history
                            ? ui.t("清空记录", "Clear history")
                            : ui.t("关闭全部连接", "Close all connections"),
                          description: history
                            ? ui.t(
                                "清除本地保存的请求历史。",
                                "Remove the locally stored request history.",
                              )
                            : ui.t(
                                "当前 TCP 与 UDP 会话将断开。",
                                "Disconnect the current TCP and UDP sessions.",
                              ),
                        },
                      );
                    }}
                  />
                </If>
                <If when={page() === "logs"}>
                  <IconButton
                    id="log-recording"
                    icon={logRecording() ? "square" : "play"}
                    label={
                      logRecording()
                        ? ui.t("停止记录日志", "Stop recording logs")
                        : ui.t("开始记录日志", "Record logs")
                    }
                    disabled={
                      props.client.isPending("logs.setRecording") ||
                      props.client.isPending("logs.recordingStatus")
                    }
                    onPress={() =>
                      logRecordingIntent.set(logRecordingIntent() + 1)
                    }
                  />
                </If>
                <If when={page() === "converter"}>
                  <IconButton
                    icon="github"
                    label={ui.t(
                      "打开 sub-web 仓库",
                      "Open sub-web repository",
                    )}
                    onPress={() =>
                      void props.client.run("platform.openUrl", {
                        url: "https://github.com/CareyWang/sub-web",
                      })
                    }
                  />
                </If>
                <If when={page() === "resources"}>
                  <IconButton
                    id="lookup-rule"
                    icon="search"
                    label={ui.t("规则查询", "Rule lookup")}
                    disabled={
                      !props.client.domains.fresh(pageDomains("resources"))
                    }
                    onPress={() =>
                      resourceIntent.set({
                        id: ++resourceSequence,
                        kind: "lookup",
                      })
                    }
                  />
                  <IconButton
                    id="add-rule"
                    icon="plus"
                    label={ui.t("添加规则", "Add rule")}
                    disabled={
                      !props.client.domains.fresh(pageDomains("resources"))
                    }
                    onPress={() =>
                      resourceIntent.set({
                        id: ++resourceSequence,
                        kind: "add",
                      })
                    }
                  />
                </If>
              </View>
            </View>
            <If
              when={
                platform.desktop &&
                (page() === "connections" || page() === "requests")
              }
            >
              <View
                style={{
                  paddingHorizontal: spacing.xxl,
                  paddingTop: spacing.md,
                  alignSelf: "center",
                  width: "100%",
                  maxWidth: pageMeasure(page(), true),
                }}
              >
                <Tabs
                  idPrefix="activity-view"
                  value={page()}
                  options={[
                    {
                      value: "connections",
                      label: ui.t("活动连接", "Active connections"),
                    },
                    {
                      value: "requests",
                      label: ui.t("请求历史", "Request history"),
                    },
                  ]}
                  onChange={(value) => navigate(value as Page)}
                />
              </View>
            </If>
            <If when={props.client.vpnUnconfirmed()}>
              <View
                style={{
                  paddingHorizontal: spacing.lg,
                  paddingVertical: spacing.sm,
                }}
              >
                <Alert
                  title={ui.t("VPN 操作尚未确认", "VPN operation unconfirmed")}
                  text={ui.t(
                    "尚未收到原操作的最终回执。请确认原操作，或停止并重新同步；此时不能再次启动。",
                    "The original operation has no final receipt yet. Confirm it or stop and resynchronize before starting again.",
                  )}
                >
                  <Actions>
                    <Action
                      id="vpn-confirm"
                      secondary
                      small
                      label={ui.t("确认状态", "Confirm operation")}
                      loading={props.client.isPending("vpn.confirm")}
                      disabled={props.client.pending() > 0}
                      onPress={() => {
                        const current = props.client.vpnUnconfirmed();
                        if (current)
                          void props.client.run("vpn.confirm", {
                            requestId: current.requestId,
                          });
                      }}
                    />
                    <Action
                      id="vpn-recover"
                      secondary
                      small
                      label={ui.t("停止并重新同步", "Stop and resynchronize")}
                      loading={props.client.isPending("vpn.recover")}
                      disabled={props.client.pending() > 0}
                      onPress={() => {
                        const current = props.client.vpnUnconfirmed();
                        if (current)
                          void props.client.run("vpn.recover", {
                            requestId: current.requestId,
                          });
                      }}
                    />
                  </Actions>
                </Alert>
              </View>
            </If>
            <If when={!props.client.state()}>
              <View
                style={{
                  flex: 1,
                  justifyContent: "center",
                  alignItems: "center",
                }}
              >
                <Loading
                  text={ui.t("正在读取连接状态…", "Loading connection state…")}
                />
                <If when={props.client.domains.error(["config", "status"])}>
                  <PageError
                    id="critical-error"
                    text={props.client.domains.error(["config", "status"])}
                  />
                </If>
                <If when={props.client.domains.error(["config", "status"])}>
                  <Action
                    secondary
                    label={ui.t("重试", "Retry")}
                    onPress={() =>
                      props.client.domains.retry(["config", "status"])
                    }
                  />
                </If>
              </View>
            </If>
            <If when={props.client.state()}>
              <NavStack state={nav} style={{ flex: 1 }}>
                {(current) => {
                  const content = (
                    <View
                      style={{ flex: 1, backgroundColor: palette().background }}
                    >
                      <View
                        style={{
                          flex: 1,
                          width: "100%",
                          maxWidth:
                            current.name === "editor"
                              ? undefined
                              : pageMeasure(current.name, wide()),
                          alignSelf: "center",
                        }}
                      >
                        <RouteOwner.Provider value={current}>
                          <PageData domains={pageDomains(current.name)}>
                            <If when={current.name === "dashboard"}>
                              <Dashboard />
                            </If>
                            <If when={current.name === "profiles"}>
                              <Profiles />
                            </If>
                            <If when={current.name === "editor"}>
                              <ProfileEditor profileId={current.query || ""} />
                            </If>
                            <If when={current.name === "proxies"}>
                              <Proxies />
                            </If>
                            <If when={current.name === "traffic"}>
                              <Traffic />
                            </If>
                            <If when={current.name === "connections"}>
                              <Activity />
                            </If>
                            <If when={current.name === "requests"}>
                              <Activity history />
                            </If>
                            <If when={current.name === "resources"}>
                              <Resources />
                            </If>
                            <If when={current.name === "tools"}>
                              <Tools />
                            </If>
                            <If when={current.name === "settings"}>
                              <Settings />
                            </If>
                            <If when={current.name === "appearance"}>
                              <Appearance />
                            </If>
                            <If when={current.name === "logs"}>
                              <Logs />
                            </If>
                            <If when={current.name === "diagnostics"}>
                              <Diagnostics />
                            </If>
                            <If when={current.name === "advanced-diagnostics"}>
                              <AdvancedDiagnostics />
                            </If>
                            <If when={current.name === "backups"}>
                              <Backups />
                            </If>
                            <If when={current.name === "about"}>
                              <About />
                            </If>
                            <If when={current.name === "privacy"}>
                              <Privacy />
                            </If>
                            <If when={current.name === "converter"}>
                              <Converter />
                            </If>
                          </PageData>
                        </RouteOwner.Provider>
                      </View>
                    </View>
                  );
                  return (
                    <NavPage
                      style={{ backgroundColor: palette().background }}
                      headerVisible={false}
                      animation={
                        reduceMotion()
                          ? "none"
                          : platform.desktop
                            ? "fade"
                            : parentPage(current.name)
                              ? "default"
                              : "fade"
                      }
                      popOnBack={
                        current.name !== "editor" && current.name !== "settings"
                      }
                      onBackPress={() => {
                        if (
                          current.name === "editor" ||
                          current.name === "settings"
                        )
                          back();
                      }}
                    >
                      {content}
                    </NavPage>
                  );
                }}
              </NavStack>
            </If>
          </View>
        </View>
        <If when={!wide() && !parentPage(page())}>
          <BaseTabs
            testID="bottom-navigation"
            style={{
              minHeight: chrome.bottom,
              flexDirection: "row",
              paddingHorizontal: spacing.sm,
              paddingVertical: spacing.xxs,
              gap: spacing.xxs,
              backgroundColor: palette().background,
              borderTopWidth: 1,
              borderTopColor: palette().border,
            }}
            indicatorStyle={{
              top: spacing.xxs,
              bottom: spacing.xxs,
              borderRadius: radius.md,
              borderCurve: "continuous",
              backgroundColor: palette().soft,
            }}
          >
            <Each of={primary}>
              {(tab) => (
                <Tab
                  testID={`nav-${tab().page}`}
                  selected={group() === tab().page}
                  accessibilityLabel={ui.t(tab().zh, tab().en)}
                  onPress={() => navigate(tab().page)}
                  focusStyle={focusRing()}
                  activeStyle={{ backgroundColor: palette().soft }}
                  style={{
                    flex: 1,
                    minHeight: control.compact ? 44 : 52,
                    alignItems: "center",
                    justifyContent: "center",
                    gap: spacing.xxs,
                    borderRadius: radius.md,
                    borderCurve: "continuous",
                    ...feedbackStyle(),
                  }}
                >
                  <Icon
                    name={tab().icon}
                    size={20}
                    color={
                      group() === tab().page
                        ? palette().accent
                        : palette().muted
                    }
                  />
                  <Text
                    maxFontSizeMultiplier={1.2}
                    style={{
                      fontSize: density.font(12),
                      lineHeight: density.line(16),
                      color:
                        group() === tab().page
                          ? palette().text
                          : palette().muted,
                      fontWeight: group() === tab().page ? "600" : "500",
                    }}
                  >
                    {ui.t(tab().zh, tab().en)}
                  </Text>
                </Tab>
              )}
            </Each>
          </BaseTabs>
        </If>
        <FloatingVpnPowerButton
          visible={!wide() && page() === "dashboard" && !!props.client.state()}
        />
        <NotificationViewport
          dismissLabel={ui.t("关闭通知", "Dismiss notification")}
        />
      </>
    );
    return (
      <View
        style={{
          flex: 1,
          backgroundColor: palette().background,
          paddingTop: "env(safe-area-inset-top)",
          paddingBottom: "env(safe-area-inset-bottom)",
          paddingLeft: "env(safe-area-inset-left)",
          paddingRight: "env(safe-area-inset-right)",
        }}
      >
        {content}
      </View>
    );
  };
  return (
    <Notifications.Provider value={notifications}>
      <App.Provider value={ui}>
        <Shell />
      </App.Provider>
    </Notifications.Provider>
  );
}

function PageData(props: { domains: DomainInput[]; children?: JSX.Element }) {
  const ui = useApp();
  return (
    <>
      <If
        when={ui.client.domains.has(props.domains)}
        else={
          <View
            style={{
              flex: 1,
              alignItems: "center",
              justifyContent: "center",
              gap: density.space(12),
            }}
          >
            <Loading text={ui.t("正在读取页面数据…", "Loading page data…")} />
            <If when={ui.client.domains.error(props.domains)}>
              <PageError text={ui.client.domains.error(props.domains)} />
            </If>
            <If when={ui.client.domains.error(props.domains)}>
              <Action
                id="page-data-retry"
                secondary
                label={ui.t("重试", "Retry")}
                onPress={() => ui.client.domains.retry(props.domains)}
              />
            </If>
          </View>
        }
      >
        {props.children}
      </If>
      <If
        when={
          ui.client.domains.has(props.domains) &&
          ui.client.domains.error(props.domains)
        }
      >
        <View style={{ padding: density.space(12), gap: density.space(8) }}>
          <PageError text={ui.client.domains.error(props.domains)} />
          <Action
            id="page-data-retry"
            secondary
            small
            label={ui.t("重新读取", "Reload data")}
            onPress={() => ui.client.domains.retry(props.domains)}
          />
        </View>
      </If>
    </>
  );
}

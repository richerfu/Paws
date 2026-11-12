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
  safeArea,
  keyboardFrame,
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
  Badge,
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
  desktopGroupTitle,
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
import { workspaceLayout } from "./layout";
import { Workspaces } from "./workspaces";
import { ActivityDetail } from "./pages/activity";
import { ObjectMenu } from "./object-menu";

const primary: { page: Page; zh: string; en: string; icon: IconName }[] = [
  { page: "dashboard", zh: "首页", en: "Home", icon: "house" },
  { page: "profiles", zh: "订阅", en: "Subscriptions", icon: "rss" },
  { page: "traffic", zh: "流量", en: "Traffic", icon: "activity" },
  { page: "tools", zh: "设置", en: "Settings", icon: "settings" },
];
const titles: Record<Page, [string, string]> = {
  dashboard: ["首页", "Home"],
  profiles: ["订阅", "Subscriptions"],
  "profile-detail": ["订阅详情", "Subscription details"],
  proxies: ["代理节点", "Proxy nodes"],
  "proxy-group": ["分组节点", "Group members"],
  traffic: ["流量", "Traffic"],
  tools: ["设置", "Settings"],
  settings: ["网络设置", "Network settings"],
  appearance: [...COPY["page-tr-001"]],
  connections: ["连接与请求", "Connections & requests"],
  requests: ["连接与请求", "Connections & requests"],
  "activity-detail": ["连接详情", "Connection details"],
  resources: ["资源与规则", "Resources & rules"],
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
  if (page === "proxy-group") return "proxies";
  if (page === "activity-detail") return "connections";
  return page === "proxies" || page === "editor" || page === "profile-detail"
    ? "profiles"
    : page === "privacy"
      ? "about"
      : "tools";
}
export function pageMeasure(page: Page, wide: boolean): number {
  if (!wide) return 640;
  if (page === "dashboard") return 960;
  if (["profiles", "profile-detail", "proxies", "proxy-group"].includes(page))
    return 1120;
  if (["traffic", "converter", "diagnostics"].includes(page)) return 1120;
  if (page === "backups") return platform.desktop ? 1120 : 640;
  if (["requests", "connections", "resources", "logs"].includes(page))
    return 1180;
  return platform.desktop ? 720 : 640;
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
  type Route = { name: Page; query?: string };
  const workspaces = new Workspaces<Route>("dashboard", { name: "dashboard" });
  const nav = createNavStack<Route>(workspaces.routes());
  const sidebarExpanded = signal<boolean | undefined>();
  const layout = () => workspaceLayout(windowSize().width - safeArea().left - safeArea().right,
    platform.desktop, sidebarExpanded(), fontScale(), pageMeasure(page(), platform.desktop || windowSize().width >= 840));
  const overlays = signal(0);
  const alive = useLifetime(),
    importIntent = signal(0),
    resourceIntent: AppContext["resourceIntent"] = signal(undefined),
    proxyDelayLoading = signal(false);
  const leaveGuards = new Map<unknown, Set<() => boolean | Promise<boolean>>>();
  let leaving = false;
  let importConsumed = 0,
    resourceConsumed = 0,
    resourceSequence = 0;
  const logRecording = signal(false),
    logRecordingIntent = signal(0);
  const route = () => nav.routes()[nav.routes().length - 1],
    page = () => route().name;
  const requestNavigation = (action: () => void) => {
    if (overlays() > 0) return;
    const guards = [...(leaveGuards.get(route()) || [])];
    if (!guards.length) {
      action();
      return;
    }
    if (leaving) return;
    const current = route();
    leaving = true;
    void (async () => {
      for (const check of guards) if (!(await check())) return false;
      return true;
    })()
      .then((accepted) => {
        if (accepted && alive() && route() === current) action();
      })
      .catch((error) => { props.client.error.set(String(error)); })
      .finally(() => {
        leaving = false;
      });
  };
  const switchWorkspace = (next: Page) => {
    if (workspaces.root === next) return;
    requestNavigation(() => {
    nav.setRoutes(workspaces.switch(next, () => ({ name: next })));
    });
  };
  const navigate = (next: Page, query?: string) =>
    requestNavigation(() => {
      if (primary.some((item) => item.page === next) && !query)
        nav.setRoutes(workspaces.switch(next, () => ({ name: next })));
      else nav.setRoutes(workspaces.push({ name: next, query }));
    });
  const back = () =>
    requestNavigation(() => {
      if (workspaces.depth > 1) nav.setRoutes(workspaces.back());
      else {
        const parent = parentPage(page());
        if (parent) nav.setRoutes(workspaces.switch(parent, () => ({ name: parent })));
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
    switchActivity: (mode) => {
      if (mode === page()) return;
      requestNavigation(() => {
        if (workspaces.depth > 1)
          nav.setRoutes(workspaces.replace({ name: mode }));
        else
          nav.setRoutes(
            workspaces.switch(mode, () => ({ name: mode })),
          );
      });
    },
    layout,
    viewState: new Map(),
    overlays,
    beforeLeave: (check, owner = route()) => {
      const guards = leaveGuards.get(owner) || new Set();
      guards.add(check);
      leaveGuards.set(owner, guards);
      return () => {
        guards.delete(check);
        if (!guards.size) leaveGuards.delete(owner);
      };
    },
  };
  const wide = () => layout().sidebar;
  const depth = () => { route(); return workspaces.depth; };
  const canBack = () =>
    depth() > 1 || (!wide() && !!parentPage(page()));
  const group = () =>
    page() === "proxies" || page() === "editor" || page() === "profile-detail"
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
    if (overlays() > 0) return false;
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
          switchWorkspace(desktopPrimary[index].page),
        ),
      );
    cleanup(keymap.bind("mod+,", () => switchWorkspace("appearance")));
    cleanup(
      keymap.bind("mod+b", () => sidebarExpanded.set(!layout().expanded)),
    );
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
              navigate={switchWorkspace}
              expanded={layout().expanded}
              onToggle={() => sidebarExpanded.set(!layout().expanded)}
              t={ui.t}
              version={props.client.domains.data("summary")?.appVersion || ""}
              connected={
                !!(
                  props.client.state()?.vpnRunning &&
                  props.client.state()?.networkProtected &&
                  props.client.state()?.vpnLifecycle === "connected"
                )
              }
              statusLabel={(() => {
                const current = props.client.state();
                if (!current) return "";
                const connected =
                  current.vpnRunning &&
                  current.networkProtected &&
                  current.vpnLifecycle === "connected";
                const engine = current.activeEngine
                  ? current.activeEngine === "meow"
                    ? "Meow"
                    : current.activeEngine
                  : "";
                const stateText = connected
                  ? ui.t("已连接", "Connected")
                  : ui.t("未连接", "Disconnected");
                return engine ? `${engine} · ${stateText}` : stateText;
              })()}
            />
          </If>
          <View style={{ flex: 1, minWidth: 0 }}>
            <View
              style={{
                minHeight: Math.max(
                  wide() ? chrome.toolbar : chrome.header,
                  density.line(24) * Math.max(1, fontScale()) + spacing.md,
                ),
                borderBottomWidth: 1,
                borderBottomColor: palette().line,
                alignItems: "center",
                justifyContent: "center",
              }}
            >
              <View
                style={{
                  width: "100%",
                  maxWidth: wide() ? undefined : pageMeasure(page(), false),
                  paddingHorizontal: wide() ? layout().inset : spacing.lg,
                  flexDirection: "row",
                  alignItems: "center",
                  gap: 8,
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
                <If when={platform.desktop && !canBack()}>
                  <View
                    style={{
                      flexDirection: "row",
                      alignItems: "center",
                      gap: 4,
                      flexShrink: 0,
                    }}
                  >
                    <Text
                      style={{
                        fontSize: 13,
                        lineHeight: 18,
                        color: palette().muted,
                      }}
                    >
                      {ui.t(...desktopGroupTitle(page()))}
                    </Text>
                    <Icon
                      name="chevron-right"
                      size={13}
                      color={palette().faint}
                    />
                  </View>
                </If>
                <Text
                  role="header"
                  style={{
                    flex: 1,
                    minWidth: 0,
                    numberOfLines: 1,
                    color: palette().text,
                    fontSize: density.font(17),
                    lineHeight: density.line(24),
                    fontWeight: "600",
                    letterSpacing: isChinese(props.client.preferences())
                      ? 0
                      : -0.2,
                  }}
                >
                  {ui.t(...titles[page()])}
                </Text>
                <If when={wide() && page() === "dashboard"}>
                  <Action
                    small
                    secondary
                    icon="refresh-cw"
                    label={ui.t("刷新", "Refresh")}
                    disabled={!props.client.state()}
                    onPress={() =>
                      props.client.domains.retry(pageDomains("dashboard"))
                    }
                  />
                </If>
                <If when={page() === "profiles"}>
                  <If when={platform.desktop}>
                    <Action
                      id="refresh-profiles"
                      small
                      secondary
                      icon="refresh-cw"
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
                    <Action
                      id="add-profile"
                      small
                      primary
                      icon="plus"
                      label={ui.t("添加订阅…", "Add subscription…")}
                      onPress={() => importIntent.set(importIntent() + 1)}
                    />
                  </If>
                  <If when={!platform.desktop}>
                    <IconButton
                      id="add-profile"
                      icon="plus"
                      label={ui.t("添加订阅", "Add subscription")}
                      onPress={() => importIntent.set(importIntent() + 1)}
                    />
                    <ObjectMenu
                      id="profiles-more"
                      title={ui.t("订阅操作", "Subscription actions")}
                      commands={[
                        {
                          id: "refresh-profiles",
                          icon: "refresh-cw",
                          label: ui.t("全部更新", "Refresh all"),
                          disabled:
                            props.client.pending() > 0 ||
                            !props.client.domains.readyForCommand([
                              "config",
                              "profiles",
                            ]),
                          run: () => void refreshProfiles(),
                        },
                      ]}
                      row={(trigger) => trigger}
                    />
                  </If>
                </If>
                <If when={page() === "proxies" || page() === "proxy-group"}>
                  <Action
                    small
                    secondary
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
                            .proxyGroups.filter(
                              (group) =>
                                page() !== "proxy-group" ||
                                group.name === ui.query(),
                            )) {
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
                  {(() => {
                    const clearOrCloseAll = () => {
                      const history = page() === "requests";
                      const state = props.client.state()!;
                      const ids = (
                        history ? state.requestHistory : state.connections
                      ).map((row) => row.id);
                      const session = state.vpnSessionId;
                      void confirm(
                        ui,
                        history
                          ? ui.t("清空请求记录？", "Clear request history?")
                          : ui.t(
                              "关闭全部活动连接？",
                              "Close all active connections?",
                            ),
                        () => {
                          const now = props.client.state()!;
                          if (
                            session !== now.vpnSessionId ||
                            (history &&
                              ids.join("\n") !==
                                now.requestHistory
                                  .map((row) => row.id)
                                  .join("\n"))
                          ) {
                            props.client.warning.set(
                              ui.t(
                                "记录或会话已变化，请重新确认范围。",
                                "Records or session changed. Review the scope again.",
                              ),
                            );
                            return;
                          }
                          if (history)
                            void props.client.run("requests.clear");
                          else
                            void (async () => {
                              for (const id of ids) {
                                if (
                                  props.client.state()!.vpnSessionId !==
                                  session
                                )
                                  break;
                                if (
                                  props.client
                                    .state()!
                                    .connections.some((row) => row.id === id)
                                )
                                  await props.client.run("connections.close", {
                                    id,
                                  });
                              }
                            })();
                        },
                        {
                          actionLabel: history
                            ? ui.t("清空记录", "Clear history")
                            : ui.t("关闭全部连接", "Close all connections"),
                          description: history
                            ? ui.t(
                                `清除当前 ${ids.length} 条请求历史，不影响活动连接。`,
                                `Clear these ${ids.length} history records. Active connections are unaffected.`,
                              )
                            : ui.t(
                                `关闭当前 ${ids.length} 条连接；确认后新建的连接不受影响。`,
                                `Close these ${ids.length} connections. Connections opened after this confirmation are unaffected.`,
                              ),
                        },
                      );
                    };
                    const actionDisabled = () =>
                      props.client.pending() > 0 ||
                      !props.client.domains.fresh([
                        page() === "requests" ? "requests" : "connections",
                      ]);
                    return (
                      <Action
                        id="activity-clear-all"
                        small
                        secondary
                        icon={page() === "requests" ? "trash-2" : "x"}
                        label={
                          page() === "requests"
                            ? ui.t("清空…", "Clear…")
                            : ui.t("关闭全部…", "Close all…")
                        }
                        disabled={actionDisabled()}
                        onPress={clearOrCloseAll}
                      />
                    );
                  })()}
                </If>
                <If when={page() === "logs"}>
                  <If when={wide()}>
                    <Badge
                      tone={logRecording() ? "brand" : "neutral"}
                      label={
                        logRecording()
                          ? ui.t("正在监听", "Listening")
                          : ui.t("已暂停", "Paused")
                      }
                    />
                  </If>
                  <Action
                    id="log-recording"
                    small
                    secondary
                    icon={logRecording() ? "pause" : "play"}
                    label={
                      logRecording()
                        ? ui.t("暂停", "Pause")
                        : ui.t("继续", "Resume")
                    }
                    disabled={
                      props.client.isPending("logs.setRecording") ||
                      props.client.isPending("logs.recordingStatus")
                    }
                    onPress={() =>
                      logRecordingIntent.set(logRecordingIntent() + 1)
                    }
                  />
                  <Action
                    id="logs-clear"
                    small
                    secondary
                    label={ui.t("清空…", "Clear…")}
                    disabled={
                      !props.client.domains.fresh(["logs"]) ||
                      props.client.isPending("logs.clear") ||
                      (props.client.state()?.logs.length || 0) === 0
                    }
                    onPress={() => {
                      const state = props.client.state()!,
                        session = state.vpnSessionId;
                      const scope = JSON.stringify(state.logs);
                      void confirm(
                        ui,
                        ui.t("清空当前内存日志？", "Clear current in-memory logs?"),
                        () => {
                          if (
                            session !== props.client.state()!.vpnSessionId ||
                            scope !== JSON.stringify(props.client.state()!.logs)
                          ) {
                            props.client.warning.set(
                              ui.t(
                                "日志或会话已变化，请重新确认。",
                                "Logs or session changed. Review again.",
                              ),
                            );
                            return;
                          }
                          void props.client.run("logs.clear");
                        },
                        {
                          actionLabel: ui.t("清空当前日志", "Clear current logs"),
                          description: ui.t(
                            `清空确认时的全部 ${state.logs.length} 条内存日志，不受筛选影响；保留归档，继续记录状态不变。`,
                            `Clear all ${state.logs.length} in-memory entries, including filtered entries. Archives and recording state remain unchanged.`,
                          ),
                        },
                      );
                    }}
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
                  <If when={platform.desktop}>
                    <Action
                      id="lookup-rule"
                      small
                      secondary
                      icon="search"
                      label={ui.t("规则查询…", "Rule lookup…")}
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
                    <Action
                      id="add-rule"
                      small
                      primary
                      icon="plus"
                      label={ui.t("添加规则…", "Add rule…")}
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
                  <If when={!platform.desktop}>
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
                    <ObjectMenu
                      id="resources-more"
                      title={ui.t("资源操作", "Resource actions")}
                      commands={[
                        {
                          id: "lookup-rule",
                          icon: "search",
                          label: ui.t("规则查询", "Rule lookup"),
                          disabled: !props.client.domains.fresh(
                            pageDomains("resources"),
                          ),
                          run: () =>
                            resourceIntent.set({
                              id: ++resourceSequence,
                              kind: "lookup",
                            }),
                        },
                      ]}
                      row={(trigger) => trigger}
                    />
                  </If>
                </If>
              </View>
            </View>
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
                      stop
                      small
                      icon="stop"
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
                <If when={!props.client.domains.error(["config", "status"])}><Loading
                  text={ui.t("正在读取连接状态…", "Loading connection state…")}
                /></If>
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
                            <If when={current.name === "profile-detail"}>
                              <Profiles profileId={current.query} />
                            </If>
                            <If when={current.name === "editor"}>
                              <ProfileEditor profileId={current.query || ""} />
                            </If>
                            <If when={current.name === "proxies"}>
                              <Proxies />
                            </If>
                            <If when={current.name === "proxy-group"}>
                              <Proxies groupKey={current.query} />
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
                            <If when={current.name === "activity-detail"}>
                              <ActivityDetail identity={current.query || ""} />
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
                      title={ui.t(...titles[current.name])}
                      popOnBack={false}
                      onBackPress={() => {
                        if (current === route() && overlays() === 0 && canBack()) back();
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
        <If when={!wide() && depth() === 1 && !parentPage(page())}>
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
              borderTopColor: palette().line,
            }}
          >
            <Each of={primary}>
              {(tab) => (
                <Tab
                  testID={`nav-${tab().page}`}
                  selected={group() === tab().page}
                  accessibilityLabel={ui.t(tab().zh, tab().en)}
                  onPress={() => switchWorkspace(tab().page)}
                  focusStyle={focusRing()}
                  hoverStyle={{ backgroundColor: palette().hover }}
                  activeStyle={{ backgroundColor: palette().soft }}
                  style={{
                    flex: 1,
                    minHeight: 52,
                    alignItems: "center",
                    justifyContent: "center",
                    gap: 2,
                    borderRadius: radius.lg,
                    borderCurve: "continuous",
                    ...feedbackStyle(),
                  }}
                >
                  <View
                    style={{
                      width: 48,
                      height: 28,
                      borderRadius: 14,
                      borderCurve: "continuous",
                      alignItems: "center",
                      justifyContent: "center",
                    }}
                  >
                    <Icon
                      name={tab().icon}
                      size={18}
                      strokeWidth={group() === tab().page ? 2.2 : 1.8}
                      color={
                        group() === tab().page
                          ? palette().accent
                          : palette().muted
                      }
                    />
                  </View>
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
          visible={
            !wide() &&
            !layout().overview &&
            page() === "dashboard" &&
            !!props.client.state() &&
            overlays() === 0 &&
            keyboardFrame().height === 0
          }
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
  const ui = useApp(), owner = RouteOwner() || ui.route();
  const error = () => ui.route() === owner ? ui.client.domains.error(props.domains) : "";
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
            <If when={!error()}><Loading text={ui.t("正在读取页面数据…", "Loading page data…")} /></If>
            <If when={error()}>
              <PageError text={error()} />
            </If>
            <If when={error()}>
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
          error()
        }
      >
        <View style={{ padding: density.space(12), gap: density.space(8) }}>
          <PageError text={error()} />
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

import { cleanup, computed, effect, signal, Text, untracked, View, type JSX } from "nsh";
import { BackHandler } from "nsh/android";
import { setColorScheme, windowSize } from "nsh/device";
import { createNavigator, SafeAreaView, Screen, Stack } from "nsh/ui/mobile";
import { CoreGateway, type CoreClient } from "./client";
import {
  App,
  Action,
  Actions,
  Dialog,
  Empty,
  Alert,
  confirm,
  Hint,
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
import { Dashboard, Traffic, VpnPowerButton } from "./pages/overview";
import { Profiles, ProfileEditor } from "./pages/profiles";
import { Proxies } from "./pages/proxies";
import { Activity } from "./pages/activity";
import { Resources } from "./pages/resources";
import { Appearance, Settings } from "./pages/settings";
import { Logs } from "./pages/logs";
import { About, Backups, Diagnostics, Tools, Privacy, AdvancedDiagnostics } from "./pages/tools";
import { Converter } from "./pages/converter";
import { COPY } from "./product-copy";
import type { IconName } from "./icons";
import { NotificationCenter, Notifications, NotificationViewport } from "./notifications";
import { pageDomains } from "./page-data";
import { PageError } from "./page-error";
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

export function PawsApp(props: { client: CoreClient; embeddedGateway?: boolean }) {
  const notifications = new NotificationCenter();
  for (const source of [props.client.error, props.client.warning, props.client.notice])
    effect(() => {
      const message = source();
      if (message) untracked(() => notifications.publish(message));
    });
  cleanup(() => notifications.dispose());
  const nav = createNavigator<{ name: Page; query?: string }>({ name: "dashboard" });
  const alive = useLifetime(),
    importIntent = signal(0),
    resourceIntent: AppContext["resourceIntent"] = signal(undefined),
    editor = signal(""),
    proxyDelayLoading = signal(false);
  let importConsumed = 0,
    resourceConsumed = 0,
    resourceSequence = 0;
  const logRecording = signal(false),
    logRecordingIntent = signal(0);
  const confirmation: AppContext["confirmation"] = signal(undefined);
  const route = () => nav.routes()[nav.routes().length - 1],
    page = () => route().name;
  const navigate = (next: Page, query?: string) => {
    if (primary.some((item) => item.page === next)) nav.reset([{ name: next, query }]);
    else nav.push({ name: next, query });
  };
  const back = () => {
    if (nav.routes().length > 1) nav.pop();
    else {
      const parent = parentPage(page());
      if (parent) navigate(parent);
    }
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
    confirmation,
    t: translator(props.client),
    editProfile: (id) => editor.set(id),
  };
  const wide = () => windowSize().width >= 840;
  const group = () =>
    page() === "proxies"
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
  effect(() => {
    route();
    untracked(() => {
      confirmation()?.resolve(false);
      confirmation.set(undefined);
    });
  });
  const hardware = BackHandler.addEventListener("hardwareBackPress", () => {
    if (!parentPage(page()) && nav.routes().length < 2) return false;
    back();
    return true;
  });
  cleanup(() => {
    hardware.remove();
    confirmation()?.resolve(false);
  });
  const confirmAnswer = (answer: boolean) => {
    const current = confirmation();
    confirmation.set(undefined);
    current?.resolve(answer);
  };
  const refreshProfiles = async () => {
    if (props.client.pending() > 0 || !props.client.domains.fresh(["config", "profiles"])) return;
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
      !props.client.domains.fresh(["config", "profiles"])
    )
      return;
    const failed = props.client
      .state()!
      .profiles.filter((profile) => attempted.has(profile.id) && profile.lastRefreshError).length;
    props.client.notice.set(
      attempted.size
        ? ui.t(
            `全部订阅刷新完成：${attempted.size - failed} 成功，${failed} 失败`,
            `Subscriptions refreshed: ${attempted.size - failed} succeeded, ${failed} failed`,
          )
        : ui.t("全部订阅没有可刷新的订阅", "No subscriptions to refresh"),
    );
  };
  const Shell = () => {
    const content = (
      <>
        <If when={props.embeddedGateway !== false}>
          <CoreGateway client={props.client} />
        </If>
        <View style={{ flex: 1, flexDirection: wide() ? "row" : "column" }}>
          <If when={wide()}>
            <View
              testID="desktop-sidebar"
              style={{
                width: 248,
                padding: spacing.md,
                backgroundColor: palette().sidebar,
                borderRightWidth: 1,
                borderRightColor: palette().border,
              }}
            >
              <View
                style={{
                  height: 52,
                  paddingLeft: spacing.sm,
                  flexDirection: "row",
                  alignItems: "center",
                  gap: spacing.sm,
                }}
              >
                <View
                  style={{
                    width: 32,
                    height: 32,
                    alignItems: "center",
                    justifyContent: "center",
                    borderRadius: radius.lg,
                    backgroundColor: palette().accent,
                  }}
                >
                  <Icon name="paw-print" size={18} color={palette().primaryText} />
                </View>
                <Text style={{ fontSize: 18, fontWeight: "700", color: palette().text }}>Paws</Text>
              </View>
              <View style={{ height: 1, backgroundColor: palette().border }} />
              <Text
                style={{
                  marginTop: spacing.lg,
                  marginHorizontal: spacing.sm,
                  marginBottom: spacing.xs,
                  color: palette().muted,
                  fontSize: 12,
                  fontWeight: "600",
                  textAlign: "center",
                }}
              >
                {ui.t("功能", "Navigation")}
              </Text>
              <Each of={primary}>
                {(tab) => (
                  <View
                    class="hover:bg-[#f4f4f5] dark:hover:bg-[#27272a] focus-visible:ring-2 focus-visible:ring-[#71717a]"
                    testID={`nav-${tab().page}`}
                    accessibilityRole="button"
                    accessibilityLabel={ui.t(tab().zh, tab().en)}
                    accessibilityState={{ selected: group() === tab().page }}
                    onPress={() => navigate(tab().page)}
                    activeOpacity={0.65}
                    style={{
                      height: 40,
                      paddingHorizontal: spacing.md,
                      marginBottom: spacing.xxs,
                      borderRadius: radius.md,
                      flexDirection: "row",
                      alignItems: "center",
                      gap: spacing.sm,
                      backgroundColor: group() === tab().page ? palette().soft : "transparent",
                    }}
                  >
                    <View
                      style={{
                        width: 3,
                        height: 18,
                        borderRadius: radius.full,
                        backgroundColor: group() === tab().page ? palette().accent : "transparent",
                      }}
                    />
                    <Icon name={tab().icon} size={17} color={palette().text} />
                    <Text
                      style={{
                        color: palette().text,
                        fontSize: 14,
                        fontWeight: group() === tab().page ? "600" : "500",
                      }}
                    >
                      {ui.t(tab().zh, tab().en)}
                    </Text>
                  </View>
                )}
              </Each>
              <View style={{ flex: 1 }} />
              <Text
                style={{
                  margin: spacing.sm,
                  color: palette().muted,
                  fontSize: 12,
                  textAlign: "center",
                }}
              >
                Paws {props.client.domains.data("summary")?.appVersion || ""}
              </Text>
            </View>
          </If>
          <View style={{ flex: 1, minWidth: 0 }}>
            <View
              style={{
                height: wide() ? 64 : 56,
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
                <If when={parentPage(page())}>
                  <IconButton
                    id="nav-back"
                    icon="arrow-left"
                    label={ui.t("返回", "Back")}
                    onPress={back}
                  />
                </If>
                <Text
                  style={{
                    flex: 1,
                    color: palette().text,
                    fontSize: 20,
                    lineHeight: 28,
                    fontWeight: "600",
                    letterSpacing: -0.3,
                  }}
                >
                  {ui.t(...titles[page()])}
                </Text>
                <If when={page() === "profiles"}>
                  <IconButton
                    id="refresh-profiles"
                    icon="refresh-cw"
                    label={ui.t("全部更新", "Refresh all")}
                    loading={props.client.isPending("profiles.refreshAll")}
                    disabled={
                      props.client.pending() > 0 ||
                      !props.client.domains.fresh(["config", "profiles"])
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
                    disabled={proxyDelayLoading() || !props.client.domains.fresh(["proxies"])}
                    onPress={() => {
                      if (proxyDelayLoading()) return;
                      proxyDelayLoading.set(true);
                      void (async () => {
                        try {
                          for (const group of props.client.state()!.proxyGroups) {
                            if (!alive()) return;
                            await props.client.run("proxies.testGroup", { group: group.name });
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
                          : ui.t("关闭全部活动连接？", "Close all active connections?"),
                        () =>
                          void props.client.run(
                            history ? "requests.clear" : "connections.closeAll",
                          ),
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
                    disabled={props.client.pending() > 0}
                    onPress={() => logRecordingIntent.set(logRecordingIntent() + 1)}
                  />
                </If>
                <If when={page() === "converter"}>
                  <IconButton
                    icon="github"
                    label="sub-web"
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
                    disabled={!props.client.domains.fresh(pageDomains("resources"))}
                    onPress={() => resourceIntent.set({ id: ++resourceSequence, kind: "lookup" })}
                  />
                  <IconButton
                    id="add-rule"
                    icon="plus"
                    label={ui.t("添加规则", "Add rule")}
                    disabled={!props.client.domains.fresh(pageDomains("resources"))}
                    onPress={() => resourceIntent.set({ id: ++resourceSequence, kind: "add" })}
                  />
                </If>
              </View>
            </View>
            <If when={props.client.vpnUnconfirmed()}>
              <View style={{ paddingHorizontal: spacing.lg, paddingVertical: spacing.sm }}>
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
                          void props.client.run("vpn.confirm", { requestId: current.requestId });
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
                          void props.client.run("vpn.recover", { requestId: current.requestId });
                      }}
                    />
                  </Actions>
                </Alert>
              </View>
            </If>
            <If when={!props.client.state()}>
              <View style={{ flex: 1, justifyContent: "center", alignItems: "center" }}>
                <Empty text={ui.t("正在读取连接状态…", "Loading connection state…")} />
                <If when={props.client.domains.error(["config", "status"])}>
                  <PageError
                    id="critical-error"
                    text={props.client.domains.error(["config", "status"])}
                  />
                </If>
                <Action
                  secondary
                  label={ui.t("重试", "Retry")}
                  onPress={() => props.client.domains.retry(["config", "status"])}
                />
              </View>
            </If>
            <If when={props.client.state()}>
              <Stack navigator={nav} style={{ flex: 1 }}>
                {(current) => {
                  const content = (
                    <View
                      style={{
                        flex: 1,
                        width: "100%",
                        maxWidth: pageMeasure(current.name, wide()),
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
                  );
                  return (
                    <Screen headerShown={false} animation="none">
                      {content}
                    </Screen>
                  );
                }}
              </Stack>
            </If>
          </View>
        </View>
        <If when={!wide() && !parentPage(page())}>
          <View
            testID="bottom-navigation"
            accessibilityRole="tablist"
            style={{
              height: 64,
              flexDirection: "row",
              backgroundColor: palette().background,
              borderTopWidth: 1,
              borderTopColor: palette().border,
            }}
          >
            <Each of={primary}>
              {(tab) => (
                <View
                  class="hover:bg-[#f4f4f5] dark:hover:bg-[#27272a] focus-visible:ring-2 focus-visible:ring-[#71717a]"
                  testID={`nav-${tab().page}`}
                  accessibilityRole="tab"
                  accessibilityLabel={ui.t(tab().zh, tab().en)}
                  accessibilityState={{ selected: group() === tab().page }}
                  onPress={() => navigate(tab().page)}
                  style={{ flex: 1, alignItems: "center", justifyContent: "center", gap: 3 }}
                >
                  <Icon
                    name={tab().icon}
                    size={22}
                    color={group() === tab().page ? palette().accent : palette().muted}
                  />
                  <Text
                    style={{
                      fontSize: 12,
                      lineHeight: 14,
                      fontWeight: group() === tab().page ? "600" : "500",
                      color: group() === tab().page ? palette().accent : palette().muted,
                    }}
                  >
                    {ui.t(tab().zh, tab().en)}
                  </Text>
                </View>
              )}
            </Each>
          </View>
        </If>
        <If when={!wide() && page() === "dashboard" && props.client.state()}>
          <View pointerEvents="box-none" style={{ position: "absolute", bottom: 92, right: 24 }}>
            <VpnPowerButton size={56} />
          </View>
        </If>
        <If when={editor()}>
          <ProfileEditor profileId={editor()} onClose={() => editor.set("")} />
        </If>
        <If when={confirmation()}>
          <Dialog title={confirmation()!.title} onClose={() => confirmAnswer(false)}>
            <Actions>
              <Action
                secondary
                label={ui.t("取消", "Cancel")}
                onPress={() => confirmAnswer(false)}
              />
              <Action danger label={ui.t("确认", "Confirm")} onPress={() => confirmAnswer(true)} />
            </Actions>
          </Dialog>
        </If>
        <NotificationViewport />
      </>
    );
    return (
      <SafeAreaView style={{ flex: 1, backgroundColor: palette().background }}>
        {content}
      </SafeAreaView>
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
          <View style={{ flex: 1, alignItems: "center", justifyContent: "center", gap: 12 }}>
            <Empty text={ui.t("正在读取页面数据…", "Loading page data…")} />
            <If when={ui.client.domains.error(props.domains)}>
              <PageError text={ui.client.domains.error(props.domains)} />
            </If>
            <Action
              id="page-data-retry"
              secondary
              label={ui.t("重试", "Retry")}
              onPress={() => ui.client.domains.retry(props.domains)}
            />
          </View>
        }
      >
        {props.children}
      </If>
      <If when={ui.client.domains.has(props.domains) && ui.client.domains.error(props.domains)}>
        <View style={{ padding: 12, gap: 8 }}>
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

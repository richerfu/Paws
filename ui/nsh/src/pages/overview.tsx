import { density, typography } from "../theme";
import {
  computed,
  Scroll,
  Text,
  View,
  platform,
  type JSX,
  type ViewProps,
} from "nsh";
import {
  Action,
  Actions,
  Badge,
  Body,
  Card,
  confirm,
  control,
  Detail,
  Empty,
  Hint,
  Icon,
  IconTile,
  ListRow,
  ListSurface,
  palette,
  radius,
  SectionHeader,
  Separator,
  spacing,
  SpeedGrid,
  Spinner,
  Tabs,
  useApp,
  useLayout,
  useDomainInterests,
} from "../components";
import { bytes } from "../locale";
import { connectionState } from "../connection-state";
import { PageError } from "../page-error";
import { SummaryPolicyGroups } from "./proxies";
import { feedbackStyle, useGlyphTransition, usePressFeedback } from "../motion";
import { focusRing, surfaceFinish } from "../theme";

export function VpnPowerButton(
  props: { size: number } & Pick<
    ViewProps,
    "pan" | "onPanEnd" | "style" | "accessibilityHint"
  >,
) {
  const ui = useApp(),
    current = () => ui.client.state()!;
  const starting = () =>
    current().vpnLifecycle === "starting" ||
    ui.client.vpnOperation() === "start" ||
    ui.client.vpnOperation() === "restart";
  const stopping = () => ui.client.vpnOperation() === "stop";
  const stop = () => current().vpnRunning || starting() || stopping();
  const pressDisabled = () =>
    !!ui.client.vpnUnconfirmed() ||
    stopping() ||
    (!stop() && (!current().activeProfile || ui.client.pending() > 0));
  const press = usePressFeedback(pressDisabled, 0.94);
  const glyph = useGlyphTransition(() =>
    starting() || stopping() ? "busy" : stop() ? "stop" : "start",
  );
  const foreground = () =>
    stop() ? palette().onStop : palette().primaryText;
  const finish = () => (stop() ? surfaceFinish("stop") : surfaceFinish("accent"));
  return (
    <View
      testID="vpn-toggle"
      role="button"
      accessibilityLabel={
        stop() ? ui.t("停止 VPN", "Stop VPN") : ui.t("启动 VPN", "Start VPN")
      }
      accessibilityHint={props.accessibilityHint}
      accessibilityState={{ disabled: pressDisabled() }}
      pan={props.pan}
      onPanEnd={
        props.pan
          ? (event) => {
              press.up();
              props.onPanEnd?.(event);
            }
          : undefined
      }
      // A temporarily unavailable VPN action must not disable its native drag.
      disabled={!props.pan && pressDisabled()}
      onPress={() => {
        if (!pressDisabled())
          void ui.client.run(stop() ? "vpn.stop" : "vpn.start");
      }}
      onPressDown={press.down}
      onPressUp={press.up}
      activeStyle={
        pressDisabled()
          ? undefined
          : stop()
            ? surfaceFinish("stopPressed")
            : { opacity: 0.88 }
      }
      focusStyle={focusRing()}
      hoverStyle={
        pressDisabled()
          ? undefined
          : stop()
            ? surfaceFinish("stopHover")
            : { opacity: 0.94 }
      }
      style={[
        {
          // Keep the icon within the same input target as the native pan gesture.
          pointerEvents: "box-only",
          width: props.size,
          height: props.size,
          borderRadius: props.size / 2,
          alignItems: "center",
          justifyContent: "center",
          borderWidth: 1,
          backgroundColor: stop() ? palette().stopFill : palette().brandFill,
          borderColor: finish().borderColor,
          boxShadow: props.pan
            ? surfaceFinish("floating").boxShadow
            : finish().boxShadow,
          opacity: pressDisabled() && !starting() && !stopping() ? 0.5 : 1,
          transform: [{ scale: press.scale }],
          ...feedbackStyle(),
        },
        props.style,
      ]}
    >
      <View
        style={{
          opacity: glyph,
          transform: [{ scale: glyph }],
          alignItems: "center",
          justifyContent: "center",
        }}
      >
        <If
          when={starting() || stopping()}
          else={
            <Icon
              name={stop() ? "stop" : "power"}
              size={Math.round(props.size * 0.42)}
              color={foreground()}
            />
          }
        >
          <Spinner size={Math.round(props.size * 0.42)} color={foreground()} />
        </If>
      </View>
    </View>
  );
}

export function Dashboard() {
  const ui = useApp(),
    layout = useLayout(),
    current = () => ui.client.state()!;
  useDomainInterests(() => ["traffic"]);
  const home = () => ui.client.domains.data("summary")!;
  const summary = () => ({
    global: home().globalNodeCount,
    members: home().nodeCount,
    groups: home().groupCount,
  });
  const state = () => connectionState(current(), ui.client.vpnOperation());
  const dotColor = () =>
    state() === "protected"
      ? palette().success
      : state() === "failed"
        ? palette().danger
        : ["connecting", "stopping", "unprotected"].includes(state()) ||
            !!ui.client.vpnUnconfirmed()
          ? palette().warning
          : palette().faint;
  const label = () =>
    ui.client.vpnUnconfirmed()
      ? ui.t("操作待确认", "Operation unconfirmed")
      : state() === "connecting"
        ? ui.t("正在连接", "Connecting")
        : state() === "stopping"
          ? ui.t("正在断开", "Disconnecting")
          : state() === "failed"
            ? ui.t("连接失败", "Connection failed")
            : state() === "protected"
              ? ui.t("已连接", "Connected")
              : state() === "unprotected"
                ? ui.t("保护未确认", "Protection unconfirmed")
                : ui.t("未连接", "Disconnected");
  const toggle = () =>
    void ui.client.run(
      current().vpnRunning || state() === "connecting"
        ? "vpn.stop"
        : "vpn.start",
    );
  const unavailable = () =>
    !!ui.client.vpnUnconfirmed() ||
    ui.client.vpnOperation() === "stop" ||
    (!current().vpnRunning &&
      state() !== "connecting" &&
      (!current().activeProfile || ui.client.pending() > 0));
  const node = () =>
    current().mode === "direct"
      ? ui.t("直连", "Direct")
      : (current().mode === "global"
          ? home().globalSelectedNode
          : home().primarySelectedNode || home().latestRuleNode) ||
        ui.t("未选择", "Not selected");
  const nodeSubtitle = () => {
    if (current().mode === "direct")
      return ui.t("直连模式 · 不经过代理节点", "Direct mode · bypasses proxies");
    if (current().mode === "global")
      return ui.t("全局模式 · 所有连接使用同一节点", "Global mode");
    const primaryGroup = home().groups.find(
      (group) => group.name.toUpperCase() !== "GLOBAL" && group.selected,
    );
    return primaryGroup
      ? `${primaryGroup.name} · ${ui.t("规则分流", "Rule routing")}`
      : ui.t("规则分流 · 点按选择节点", "Rule routing · tap to select node");
  };
  const exit = () =>
    !current().vpnRunning
      ? ui.t("连接后显示", "Shown when connected")
      : current().exitLocation.ip
        ? [
            current().exitLocation.ip,
            current().exitLocation.countryCode
              ? current()
                  .exitLocation.countryCode.toUpperCase()
                  .replace(/[A-Z]/g, (letter) =>
                    String.fromCodePoint(127397 + letter.charCodeAt(0)),
                  )
              : "",
            current().exitLocation.country,
          ]
            .filter(Boolean)
            .join(" · ")
        : current().exitLocation.error
          ? ui.t("获取失败", "Lookup failed")
          : ui.t("正在获取…", "Fetching…");
  const columns = () => layout().overview;
  const sessionTotal = () =>
    current().traffic.uploadBytes + current().traffic.downloadBytes;
  const RateSection = () => (
    <View style={{ gap: spacing.sm }}>
      <SectionHeader
        title={ui.t("速率", "Speed")}
        meta={
          state() === "protected"
            ? `${ui.t("本次", "Session")} ${bytes(sessionTotal())}`
            : undefined
        }
      />
      <SpeedGrid
        connected={
          state() === "protected" && ui.client.domains.fresh(["traffic"])
        }
        upload={current().traffic.uploadSpeed}
        download={current().traffic.downloadSpeed}
        formatBytes={bytes}
      />
      <Hint
        text={
          state() === "protected"
            ? ui.t(
                "查看流量详情，可查看速率历史与活动连接。",
                "Open Traffic for rate history and active connections.",
              )
            : ui.t("连接后显示实时速率。", "Connect to view live speed.")
        }
      />
    </View>
  );
  return (
    <Scroll
      testID="dashboard-scroll"
      bounces={false}
      style={{ flex: 1 }}
      contentStyle={{
        paddingBottom: layout().sidebar
          ? layout().inset
          : Math.max(76, 64 + layout().inset * 2),
      }}
    >
      <View
        testID={platform.desktop ? "desktop-dashboard-workspace" : undefined}
        style={{
          flexDirection: columns() ? "row" : "column",
          gap: columns() ? layout().gap : spacing.section,
          paddingHorizontal: layout().inset,
          paddingTop: layout().inset,
        }}
      >
        <View
          testID="dashboard-status-panel"
          style={{
            width: columns() ? layout().connection : "100%",
            minWidth: 0,
            gap: spacing.section,
          }}
        >
          <View
            style={{
              padding: spacing.lg,
              gap: density.space(12),
              borderRadius: radius.xl,
              borderCurve: "continuous",
              backgroundColor: palette().panel,
              borderWidth: 1,
              borderColor: palette().border,
              ...surfaceFinish("card"),
            }}
          >
            <View
              testID="connection-row"
              style={{
                flexDirection: "row",
                alignItems: "center",
                gap: spacing.md,
                minWidth: 0,
              }}
            >
              <View
                style={{
                  width: 18,
                  height: 18,
                  borderRadius: 9,
                  backgroundColor:
                    state() === "protected"
                      ? palette().successFill
                      : "transparent",
                  alignItems: "center",
                  justifyContent: "center",
                }}
              >
                <View
                  style={{
                    width: 10,
                    height: 10,
                    borderRadius: 5,
                    backgroundColor: dotColor(),
                  }}
                />
              </View>
              <View style={{ flex: 1, minWidth: 0, gap: density.space(2) }}>
                <Text
                  testID="vpn-status"
                  style={{
                    fontSize: density.font(20),
                    lineHeight: density.line(26),
                    fontWeight: "600",
                    color:
                      state() === "failed" ? palette().danger : palette().text,
                  }}
                >
                  {label()}
                </Text>
                <Text
                  testID="home-profile-summary"
                  style={{
                    numberOfLines: 1,
                    minWidth: 0,
                    fontSize: density.font(13),
                    lineHeight: density.line(18),
                    color: palette().muted,
                  }}
                >
                  {home().activeProfileName ||
                    ui.t(
                      "连接前请选择配置",
                      "Choose a profile before connecting",
                    )}
                </Text>
              </View>
            </View>
            <Tabs
              idPrefix="mode"
              value={current().mode}
              options={[
                { value: "rule", label: ui.t("规则", "Rule") },
                { value: "global", label: ui.t("全局", "Global") },
                { value: "direct", label: ui.t("直连", "Direct") },
              ]}
              onChange={(mode) => {
                if (mode !== current().mode)
                  void ui.client.run(
                    "mode.set",
                    { mode },
                    ui.t(
                      `运行模式已切换为${mode === "rule" ? "规则" : mode === "global" ? "全局" : "直连"}`,
                      `Mode changed to ${mode}`,
                    ),
                  );
              }}
              disabled={ui.client.pending() > 0}
            />
            <View style={{ gap: spacing.sm }}>
              <If
                when={current().activeProfile}
                else={
                  <Action
                    id="home-add-profile"
                    primary
                    fullWidth
                    icon="plus"
                    label={ui.t("添加订阅…", "Add subscription…")}
                    onPress={() => {
                      ui.navigate("profiles");
                      ui.importIntent.set(ui.importIntent() + 1);
                    }}
                  />
                }
              >
                <Action
                  id="home-connection-action"
                  fullWidth
                  primary={
                    !current().vpnRunning &&
                    state() !== "connecting" &&
                    state() !== "stopping"
                  }
                  stop={
                    current().vpnRunning ||
                    state() === "connecting" ||
                    state() === "stopping"
                  }
                  icon={
                    state() === "connecting" || state() === "stopping"
                      ? undefined
                      : current().vpnRunning
                        ? "stop"
                        : "power"
                  }
                  label={
                    state() === "stopping"
                      ? ui.t("正在停止", "Stopping")
                      : current().vpnRunning || state() === "connecting"
                        ? ui.t("停止", "Stop")
                        : ui.t("连接", "Connect")
                  }
                  loading={
                    state() === "connecting" || state() === "stopping"
                  }
                  disabled={unavailable()}
                  onPress={toggle}
                />
              </If>
              <If when={state() === "failed"}>
                <Action
                  fullWidth
                  label={ui.t("打开诊断", "Open diagnostics")}
                  onPress={() => ui.navigate("diagnostics")}
                />
              </If>
              <If when={state() === "unprotected" && !ui.client.vpnUnconfirmed()}>
                <Action
                  fullWidth
                  label={ui.t("确认状态", "Check status")}
                  onPress={() => ui.client.domains.retry(["config", "status"])}
                />
              </If>
            </View>
          </View>
          <If when={current().networkProtectError}>
            <PageError
              id="home-protection-error"
              text={current().networkProtectError || ""}
            />
          </If>
          <View style={{ gap: spacing.sm }}>
            <SectionHeader title={ui.t("节点", "Node")} />
            <ListSurface>
              <View
                testID="all-proxies"
                role="button"
                accessibilityLabel={ui.t("代理节点", "Proxy nodes")}
                onPress={() => ui.navigate("proxies")}
                focusStyle={focusRing()}
                hoverStyle={{ backgroundColor: palette().hover }}
                activeStyle={{ backgroundColor: palette().soft }}
                style={{
                  minHeight: control.row,
                  paddingHorizontal: platform.desktop ? 14 : 16,
                  paddingVertical: platform.desktop ? 6 : 8,
                  flexDirection: "row",
                  alignItems: "center",
                  gap: platform.desktop ? 10 : spacing.md,
                  ...feedbackStyle(),
                }}
              >
                <View
                  style={{
                    pointerEvents: "none",
                    position: "absolute",
                    left: platform.desktop ? 14 : 16,
                    right: platform.desktop ? 14 : 16,
                    bottom: 0,
                    height: 1,
                    backgroundColor: palette().line,
                  }}
                />
                <IconTile icon="git-branch" />
                <View style={{ flex: 1, minWidth: 0, gap: density.space(2) }}>
                  <Text
                    testID="home-node-value"
                    accessibilityLabel={node()}
                    style={{
                      numberOfLines: 1,
                      fontSize: typography.sm,
                      lineHeight: density.line(20),
                      fontWeight: "600",
                      color: palette().text,
                    }}
                  >
                    {node()}
                  </Text>
                  <Text
                    style={{
                      numberOfLines: 1,
                      fontSize: density.font(13),
                      lineHeight: density.line(18),
                      color: palette().muted,
                    }}
                  >
                    {nodeSubtitle()}
                  </Text>
                </View>
                <Icon name="chevron-right" size={16} color={palette().faint} />
              </View>
              <ListRow
                icon="globe"
                title={ui.t("出口 IP", "Exit IP")}
                value={exit()}
                tabular
              />
            </ListSurface>
          </View>
          <RateSection />
        </View>
        <View
          testID="dashboard-groups-panel"
          style={{
            flex: columns() ? 1 : undefined,
            minWidth: 0,
            gap: spacing.section,
          }}
        >
          <View style={{ gap: spacing.sm }}>
            <SectionHeader
              title={ui.t("策略组", "Policy groups")}
              meta={
                home().groups.length > 0
                  ? `${summary().groups} ${ui.t("组", "groups")} · ${summary().global} ${ui.t("节点", "nodes")}`
                  : undefined
              }
            />
            <If
              when={home().groups.length > 0}
              else={
                <ListSurface>
                  <Empty
                    icon="rss"
                    title={ui.t("尚未选择订阅", "No subscription selected")}
                    text={ui.t(
                      "添加并启用订阅后即可选择节点",
                      "Add and activate a subscription to select nodes",
                    )}
                  >
                    <Action
                      icon="plus"
                      primary
                      label={ui.t("添加订阅…", "Add subscription…")}
                      onPress={() => {
                        ui.navigate("profiles");
                        ui.importIntent.set(ui.importIntent() + 1);
                      }}
                    />
                  </Empty>
                </ListSurface>
              }
            >
              <SummaryPolicyGroups />
            </If>
          </View>
        </View>
      </View>
    </Scroll>
  );
}

export function Traffic() {
  const ui = useApp(),
    layout = useLayout(),
    current = () => ui.client.state()!;
  const detail = () => ui.client.domains.data("traffic")!;
  const connections = () => detail().connectionSummary;
  const history = computed(() => current().trafficHistory.slice(-24));
  const peakDownload = computed(() =>
    Math.max(0, ...history().map((point) => point.downloadSpeed)),
  );
  const peakUpload = computed(() =>
    Math.max(0, ...history().map((point) => point.uploadSpeed)),
  );
  const peak = computed(() => Math.max(1, peakDownload(), peakUpload()));
  const state = () => connectionState(current(), ui.client.vpnOperation());
  const unavailable = () =>
    ui.t("当前内核未提供统计", "Statistics unavailable from this engine");
  const columns = () => layout().overview;
  const sessionTotal = () =>
    current().traffic.uploadBytes + current().traffic.downloadBytes;
  return (
    <Scroll bounces={false} style={{ flex: 1 }}>
      <Body>
        <View
          style={{
            flexDirection: "row",
            alignItems: "center",
            justifyContent: "space-between",
            gap: spacing.sm,
          }}
        >
          <Badge
            tone={
              state() === "protected"
                ? "success"
                : state() === "failed"
                  ? "danger"
                  : ["connecting", "stopping", "unprotected"].includes(state())
                    ? "warning"
                    : "neutral"
            }
            label={
              state() === "protected"
                ? ui.t("已连接", "Connected")
                : state() === "connecting"
                  ? ui.t("正在连接", "Connecting")
                  : state() === "stopping"
                    ? ui.t("正在断开", "Disconnecting")
                    : state() === "failed"
                      ? ui.t("连接失败", "Connection failed")
                      : state() === "unprotected"
                        ? ui.t("保护未确认", "Protection unconfirmed")
                        : ui.t("未连接", "Disconnected")
            }
          />
        </View>
        <View
          style={{
            flexDirection: columns() ? "row" : "column",
            gap: columns() ? layout().gap : spacing.section,
            alignItems: "stretch",
          }}
        >
          <View
            style={{
              flex: columns() ? 1 : undefined,
              minWidth: 0,
              gap: spacing.section,
            }}
          >
            <View style={{ gap: spacing.sm }}>
              <SectionHeader
                title={ui.t("本次会话", "This session")}
                meta={`${ui.t("已用", "Used")} ${bytes(sessionTotal())}`}
              />
              <SpeedGrid
                connected={state() === "protected"}
                upload={current().traffic.uploadSpeed}
                download={current().traffic.downloadSpeed}
                formatBytes={bytes}
              />
            </View>
            <View style={{ gap: spacing.sm }}>
              <SectionHeader
                title={ui.t("速率", "Speed")}
                meta={`${history().length} ${ui.t("个采样", "samples")}`}
              />
              <ListSurface style={{ padding: spacing.md, gap: spacing.sm }}>
                <View
                  testID="traffic-chart"
                  accessible
                  accessibilityLabel={ui.t(
                    "最近流量趋势",
                    "Recent traffic history",
                  )}
                  accessibilityValue={{
                    text: `${ui.t("峰值下载", "Peak download")} ${bytes(peakDownload())}/s, ${ui.t("峰值上传", "Peak upload")} ${bytes(peakUpload())}/s`,
                  }}
                  style={{
                    height: 108,
                    paddingHorizontal: density.space(6),
                    paddingTop: density.space(8),
                    borderBottomWidth: 1,
                    borderBottomColor: palette().line,
                    flexDirection: "row",
                    alignItems: "flex-end",
                    gap: density.space(3),
                  }}
                >
                  <Each of={history()} key="index">
                    {(point) => (
                      <View
                        style={{
                          flex: 1,
                          height: "100%",
                          flexDirection: "row",
                          gap: 1,
                          alignItems: "flex-end",
                        }}
                      >
                        <View
                          style={{
                            flex: 1,
                            borderTopLeftRadius: 2,
                            borderTopRightRadius: 2,
                            height:
                              point().downloadSpeed > 0
                                ? Math.max(
                                    2,
                                    (point().downloadSpeed / peak()) * 100,
                                  )
                                : 0,
                            backgroundColor: palette().chartDownload,
                          }}
                        />
                        <View
                          style={{
                            flex: 1,
                            borderTopLeftRadius: 2,
                            borderTopRightRadius: 2,
                            height:
                              point().uploadSpeed > 0
                                ? Math.max(
                                    2,
                                    (point().uploadSpeed / peak()) * 100,
                                  )
                                : 0,
                            backgroundColor: palette().chartUpload,
                          }}
                        />
                      </View>
                    )}
                  </Each>
                </View>
                <View
                  style={{
                    flexDirection: "row",
                    flexWrap: "wrap",
                    justifyContent: "space-between",
                    gap: spacing.md,
                  }}
                >
                  <Legend
                    color={palette().chartDownload}
                    label={`${ui.t("下载 峰值", "Download peak")} ${bytes(peakDownload())}/s`}
                  />
                  <Legend
                    color={palette().chartUpload}
                    label={`${ui.t("上传 峰值", "Upload peak")} ${bytes(peakUpload())}/s`}
                  />
                </View>
                <If when={!history().length}>
                  <Hint
                    text={ui.t("等待流量采样", "Waiting for traffic samples")}
                  />
                </If>
              </ListSurface>
            </View>
          </View>
          <View
            style={{
              flex: columns() ? 1 : undefined,
              minWidth: 0,
              gap: spacing.section,
            }}
          >
            <View style={{ gap: spacing.sm }}>
              <SectionHeader
                title={ui.t("连接与诊断", "Connections and diagnostics")}
                meta={
                  ui.client.domains.data("traffic")
                    ?.connectionSummaryAvailable === false
                    ? unavailable()
                    : `${connections().count} ${ui.t("个活跃", "active")}`
                }
              />
              <ListSurface>
                <ListRow
                  id="traffic-connections"
                  icon="list-filter"
                  title={ui.t("连接与请求", "Connections & requests")}
                  subtitle={
                    ui.client.domains.data("traffic")
                      ?.connectionSummaryAvailable === false
                      ? unavailable()
                      : `${ui.t("总下载", "Downloaded")} ${bytes(connections().downloadBytes)} · ${ui.t("总上传", "Uploaded")} ${bytes(connections().uploadBytes)}`
                  }
                  chevron
                  divider
                  onPress={() => ui.navigate("connections")}
                />
                <ListRow
                  id="traffic-dns"
                  icon="globe"
                  title={ui.t("DNS 与控制器诊断", "DNS and controller diagnostics")}
                  subtitle={
                    current().dns.statisticsAvailable
                      ? `${ui.t("缓存命中", "Cache hits")} ${current().dns.cacheHits} / ${current().dns.cacheHits + current().dns.cacheMisses}`
                      : unavailable()
                  }
                  chevron
                  onPress={() => ui.navigate("dns-diagnostics")}
                />
              </ListSurface>
            </View>
          </View>
        </View>
      </Body>
    </Scroll>
  );
}


export function DnsDiagnostics() {
  const ui = useApp(),
    current = () => ui.client.state()!;
  const unavailable = () =>
    ui.t("当前内核未提供统计", "Statistics unavailable from this engine");
  return (
    <Scroll bounces={false} style={{ flex: 1 }}>
      <Body>
        <View style={{ gap: spacing.section }}>
            <View style={{ gap: spacing.sm }}>
              <SectionHeader
                title={ui.t("DNS", "DNS")}
                meta={
                  current().dns.configurationError
                    ? unavailable()
                    : current().dns.model
                }
              />
              <ListSurface>
                <TrafficRow>
                  <Detail
                    tone={
                      (current().dns.hijacking ??
                        current().vpnOptions.dnsHijacking)
                        ? "success"
                        : "neutral"
                    }
                    label={ui.t("DNS 劫持", "DNS hijacking")}
                    value={
                      (current().dns.hijacking ??
                        current().vpnOptions.dnsHijacking)
                        ? ui.t("开启", "Enabled")
                        : ui.t("关闭", "Disabled")
                    }
                  />
                </TrafficRow>
                <TrafficRow>
                  <Detail
                    label={ui.t("监听", "Listener")}
                    value={
                      current().dns.configurationError
                        ? unavailable()
                        : current().dns.listen || "—"
                    }
                  />
                </TrafficRow>
                <TrafficRow>
                  <Detail
                    label="TUN"
                    value={
                      (
                        current().dns.tunAddresses ||
                        current().vpnOptions.addresses
                      ).join(", ") || "—"
                    }
                  />
                </TrafficRow>
                <TrafficRow>
                  <Detail
                    label={ui.t("上游", "Upstreams")}
                    value={
                      current().dns.configurationError
                        ? unavailable()
                        : current().dns.upstreams.join("\n") || "—"
                    }
                  />
                </TrafficRow>
                <TrafficRow>
                  <Detail
                    label="Fallback"
                    value={
                      current().dns.configurationError
                        ? unavailable()
                        : current().dns.fallbacks.join("\n") || "—"
                    }
                  />
                </TrafficRow>
                <TrafficRow>
                  <Detail
                    label={ui.t("分流策略", "Nameserver policies")}
                    value={
                      current().dns.configurationError
                        ? unavailable()
                        : String(
                            Object.keys(current().dns.nameserverPolicy || {})
                              .length,
                          )
                    }
                  />
                </TrafficRow>
                <TrafficRow>
                  <Detail
                    label={ui.t("已处理", "Handled")}
                    value={
                      current().dns.statisticsAvailable
                        ? String(current().dns.handledPackets)
                        : unavailable()
                    }
                  />
                </TrafficRow>
                <TrafficRow>
                  <Detail
                    label={ui.t("缓存命中", "Cache hits")}
                    value={
                      current().dns.statisticsAvailable
                        ? String(current().dns.cacheHits)
                        : unavailable()
                    }
                  />
                </TrafficRow>
                <TrafficRow
                  last={
                    !(
                      current().dns.statisticsAvailable &&
                      current().dns.recentQueries.length > 0
                    )
                  }
                >
                  <Detail
                    label={ui.t("缓存回源", "Cache misses")}
                    value={
                      current().dns.statisticsAvailable
                        ? String(current().dns.cacheMisses)
                        : unavailable()
                    }
                  />
                </TrafficRow>
                <If
                  when={
                    current().dns.statisticsAvailable &&
                    current().dns.recentQueries.length > 0
                  }
                >
                  <View
                    style={{
                      paddingHorizontal: spacing.md,
                      paddingTop: density.space(8),
                    }}
                  >
                    <Hint text={ui.t("最近查询", "Recent queries")} />
                  </View>
                  <Each
                    of={current().dns.recentQueries}
                    key={(query) => `${query.name}:${query.recordType}`}
                  >
                    {(query, index) => (
                      <TrafficRow
                        last={index() === current().dns.recentQueries.length - 1}
                      >
                        <Detail
                          label={query().name}
                          value={`${query().recordType} · ${query().count}`}
                        />
                      </TrafficRow>
                    )}
                  </Each>
                </If>
              </ListSurface>
              <Actions align="start">
                <Action
                  secondary
                  label={ui.t("清理 DNS 缓存", "Flush DNS cache")}
                  disabled={ui.client.pending() > 0}
                  onPress={() =>
                    void ui.client.run(
                      "dns.flush",
                      {},
                      ui.t("DNS 缓存已清空", "DNS cache flushed"),
                    )
                  }
                />
                <Action
                  secondary
                  label={ui.t("清理 Fake-IP…", "Flush Fake-IP cache…")}
                  disabled={ui.client.pending() > 0}
                  onPress={() =>
                    void confirm(
                      ui,
                      ui.t("清理 Fake-IP 缓存？", "Flush Fake-IP cache?"),
                      () =>
                        void ui.client.run(
                          "dns.fakeIpFlush",
                          {},
                          ui.t("Fake-IP 缓存已清空", "Fake-IP cache flushed"),
                        ),
                      {
                        actionLabel: ui.t("清理 Fake-IP", "Flush Fake-IP"),
                        description: ui.t(
                          "清理后现有 Fake-IP 映射将失效，活动连接可能需要重新解析域名。",
                          "Existing Fake-IP mappings will be cleared; active connections may re-resolve domains.",
                        ),
                      },
                    )
                  }
                />
              </Actions>
            </View>
            <View style={{ gap: spacing.sm }}>
              <SectionHeader
                title={ui.t("控制器", "Controller")}
                meta={
                  current().controllerAddr || ui.t("未启动", "Not started")
                }
              />
              <ListSurface>
                <TrafficRow>
                  <Detail
                    label={ui.t("内存使用", "Memory in use")}
                    value={
                      current().controllerDiagnostics.memoryAvailable === true
                        ? bytes(
                            current().controllerDiagnostics.memoryInUseBytes,
                          )
                        : unavailable()
                    }
                  />
                </TrafficRow>
                <TrafficRow>
                  <Detail
                    label={ui.t("内存统计范围", "Memory scope")}
                    value={
                      current().controllerDiagnostics.memoryAvailable !== true
                        ? unavailable()
                        : current().controllerDiagnostics.memoryScope ===
                            "processResident"
                          ? ui.t(
                              "VPN 进程驻留内存（含宿主运行时）",
                              "VPN process resident memory, including its host runtime",
                            )
                          : ui.t("内核运行时堆内存", "Engine runtime heap")
                    }
                  />
                </TrafficRow>
                <TrafficRow last>
                  <Detail
                    label={ui.t("内存上限", "Memory limit")}
                    value={
                      current().controllerDiagnostics.memoryAvailable !== true
                        ? unavailable()
                        : current().controllerDiagnostics
                              .memoryLimitAvailable === true
                          ? bytes(
                              current().controllerDiagnostics.memoryLimitBytes,
                            )
                          : ui.t("未提供", "Not provided")
                    }
                  />
                </TrafficRow>
              </ListSurface>
            </View>
        </View>
      </Body>
    </Scroll>
  );
}

function TrafficRow(props: { last?: boolean; children?: JSX.Element }) {
  return (
    <View
      style={{
        paddingHorizontal: spacing.md,
        paddingVertical: density.space(10),
        borderBottomWidth: props.last ? 0 : 1,
        borderBottomColor: palette().line,
      }}
    >
      {props.children}
    </View>
  );
}

function Legend(props: { color: string; label: string }) {
  return (
    <View
      style={{
        flexDirection: "row",
        alignItems: "center",
        gap: density.space(6),
      }}
    >
      <View
        style={{
          width: 8,
          height: 8,
          borderRadius: 4,
          backgroundColor: props.color,
        }}
      />
      <Text
        style={{
          fontSize: density.font(12),
          lineHeight: density.line(18),
          color: palette().muted,
          fontVariant: ["tabular-nums"],
        }}
      >
        {props.label}
      </Text>
    </View>
  );
}

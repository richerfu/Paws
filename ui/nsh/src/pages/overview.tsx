import { computed, Scroll, Text, View } from "nsh";
import { windowSize } from "nsh/device";
import {
  Action,
  Actions,
  Badge,
  Body,
  Card,
  Detail,
  Empty,
  Hint,
  Icon,
  palette,
  radius,
  Separator,
  Spinner,
  Tabs,
  useApp,
} from "../components";
import { bytes, timestamp } from "../locale";
import type { Mode } from "../model";
import { connectionState } from "../connection-state";
import { PageError } from "../page-error";
import { ProxyGroupList } from "./proxies";

export function VpnPowerButton(props: { size: number }) {
  const ui = useApp(),
    current = () => ui.client.state()!;
  const starting = () =>
    current().vpnLifecycle === "starting" ||
    ui.client.vpnOperation() === "start" ||
    ui.client.vpnOperation() === "restart";
  const stopping = () => ui.client.vpnOperation() === "stop";
  const stop = () => current().vpnRunning || starting();
  return (
    <View
      class="hover:bg-[#f4f4f5] dark:hover:bg-[#27272a] focus-visible:ring-2 focus-visible:ring-[#71717a]"
      testID="vpn-toggle"
      accessibilityRole="button"
      accessibilityLabel={stop() ? ui.t("停止 VPN", "Stop VPN") : ui.t("启动 VPN", "Start VPN")}
      accessibilityState={{ disabled: stopping() }}
      disabled={
        !!ui.client.vpnUnconfirmed() ||
        stopping() ||
        (!stop() && (!current().activeProfile || ui.client.pending() > 0))
      }
      onPress={() => void ui.client.run(stop() ? "vpn.stop" : "vpn.start")}
      activeOpacity={0.8}
      style={{
        width: props.size,
        height: props.size,
        borderRadius: props.size / 2,
        backgroundColor: current().vpnRunning ? palette().danger : palette().accent,
        alignItems: "center",
        justifyContent: "center",
        opacity: stopping() ? 0.6 : 1,
      }}
    >
      <If
        when={starting() || stopping()}
        else={
          <Icon
            name={current().vpnRunning ? "square" : "power"}
            size={props.size * 0.4}
            color={palette().primaryText}
          />
        }
      >
        <Spinner size={props.size * 0.4} color={palette().primaryText} />
      </If>
    </View>
  );
}

export function Dashboard() {
  const ui = useApp(),
    current = () => ui.client.state()!;
  const home = () => ui.client.domains.data("summary")!;
  const summary = () => ({
    global: home().globalNodeCount,
    members: home().nodeCount,
    groups: home().groupCount,
  });
  const state = () => connectionState(current(), ui.client.vpnOperation());
  const color = () =>
    state() === "protected"
      ? palette().success
      : state() === "failed"
        ? palette().danger
        : palette().muted;
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
    void ui.client.run(current().vpnRunning || state() === "connecting" ? "vpn.stop" : "vpn.start");
  const node = () =>
    current().mode === "direct"
      ? ui.t("直连", "Direct")
      : (current().mode === "global"
          ? home().globalSelectedNode
          : home().primarySelectedNode || home().latestRuleNode) || ui.t("未选择", "Not selected");
  const exit = () =>
    !current().vpnRunning
      ? ui.t("未连接", "Disconnected")
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
  return (
    <View
      style={{
        flex: 1,
        paddingHorizontal: windowSize().width >= 840 ? 24 : 16,
        paddingTop: windowSize().width >= 840 ? 24 : 16,
      }}
    >
      <View style={{ flexDirection: "row", alignItems: "center", gap: 16, height: 52 }}>
        <View
          class="hover:bg-[#f4f4f5] dark:hover:bg-[#27272a] focus-visible:ring-2 focus-visible:ring-[#71717a]"
          testID="connection-row"
          accessibilityRole="button"
          accessibilityLabel={label()}
          disabled={!!ui.client.vpnUnconfirmed() || ui.client.vpnOperation() === "stop"}
          onPress={toggle}
          style={{ flex: 1, height: 52, flexDirection: "row", alignItems: "center", gap: 12 }}
        >
          <View
            style={{
              width: 40,
              height: 40,
              borderRadius: radius.lg,
              backgroundColor: palette().soft,
              alignItems: "center",
              justifyContent: "center",
            }}
          >
            <Icon
              name={
                state() === "protected"
                  ? "shield-check"
                  : state() === "failed"
                    ? "triangle-alert"
                    : "power"
              }
              size={18}
              color={color()}
            />
          </View>
          <View style={{ flex: 1, minWidth: 0 }}>
            <Text
              testID="vpn-status"
              style={{ fontSize: 20, lineHeight: 24, fontWeight: "600", color: color() }}
            >
              {label()}
            </Text>
            <View style={{ flexDirection: "row", minWidth: 0, alignItems: "center", gap: 6 }}>
              <If when={home().activeProfileName}>
                <Badge label={ui.t("当前配置", "Current profile")} />
              </If>
              <Text
                numberOfLines={1}
                style={{
                  flex: 1,
                  minWidth: 0,
                  fontSize: 12,
                  lineHeight: 16,
                  color: palette().muted,
                }}
              >
                {home().activeProfileName ||
                  ui.t("连接前请选择配置", "Choose a profile before connecting")}
              </Text>
            </View>
          </View>
        </View>
        <If when={windowSize().width >= 840}>
          <VpnPowerButton size={44} />
        </If>
      </View>
      <View style={{ height: 14 }} />
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
      <View style={{ height: 14 }} />
      <View
        style={{
          borderWidth: 1,
          borderColor: palette().border,
          borderRadius: radius.lg,
          backgroundColor: palette().panel,
          paddingHorizontal: 12,
        }}
      >
        <ConnectionRow
          lines={2}
          id="home-node-value"
          icon="git-branch"
          label={ui.t("当前节点", "Current node")}
          value={node()}
        />
        <Separator />
        <ConnectionRow icon="network" label={ui.t("出口 IP", "Exit IP")} value={exit()} />
      </View>
      <If when={current().networkProtectError}>
        <PageError id="home-protection-error" text={current().networkProtectError || ""} />
      </If>
      <View style={{ height: 14 }} />
      <View style={{ flexDirection: "row", alignItems: "center", marginBottom: 6 }}>
        <View style={{ flex: 1 }}>
          <Text style={{ fontSize: 14, lineHeight: 20, fontWeight: "600", color: palette().text }}>
            {ui.t("全局节点与策略分组", "Global nodes and policy groups")}
          </Text>
          <Text style={{ fontSize: 12, lineHeight: 16, color: palette().muted }}>
            {summary().global} {ui.t("个全局节点", "global nodes")} · {summary().members}{" "}
            {ui.t("个节点", "nodes")} · {summary().groups} {ui.t("个分组", "groups")}
          </Text>
        </View>
        <If when={home().groups.length > 0}>
          <Action
            id="all-proxies"
            variant="ghost"
            small
            label={ui.t("全部", "All")}
            onPress={() => ui.navigate("proxies")}
          />
        </If>
      </View>
      <If
        when={home().groups.length > 0}
        else={
          <View style={{ flex: 1, alignItems: "center", paddingTop: 36 }}>
            <Empty
              icon="rss"
              title={ui.t("尚未选择订阅", "No subscription selected")}
              text={ui.t(
                "添加并启用订阅后即可选择节点",
                "Add and activate a subscription to select nodes",
              )}
            />
            <Action
              icon="plus"
              label={ui.t("添加订阅", "Add subscription")}
              onPress={() => {
                ui.navigate("profiles");
                ui.importIntent.set(ui.importIntent() + 1);
              }}
            />
          </View>
        }
      >
        <ProxyGroupList summary />
      </If>
    </View>
  );
}
function ConnectionRow(props: {
  icon: "git-branch" | "network";
  label: string;
  value: string;
  lines?: number;
  id?: string;
}) {
  return (
    <View
      style={{
        minHeight: 44,
        paddingVertical: 10,
        minWidth: 0,
        flexDirection: "row",
        alignItems: "flex-start",
        gap: 8,
      }}
    >
      <Icon name={props.icon} size={15} color={palette().muted} />
      <Text style={{ width: 68, color: palette().muted, fontSize: 12 }}>{props.label}</Text>
      <Text
        testID={props.id}
        accessibilityLabel={props.value}
        selectable
        numberOfLines={props.lines}
        style={{
          flex: 1,
          minWidth: 0,
          marginLeft: 12,
          color: palette().text,
          fontSize: 14,
          lineHeight: 20,
          fontWeight: "500",
        }}
      >
        {props.value}
      </Text>
    </View>
  );
}

export function Traffic() {
  const ui = useApp(),
    current = () => ui.client.state()!;
  const detail = () => ui.client.domains.data("traffic")!;
  const profile = () => detail().activeProfileUsage;
  const connections = () => detail().connectionSummary;
  const history = computed(() => current().trafficHistory.slice(-24));
  const peakDownload = computed(() =>
    Math.max(0, ...history().map((point) => point.downloadSpeed)),
  );
  const peakUpload = computed(() => Math.max(0, ...history().map((point) => point.uploadSpeed)));
  const peak = computed(() => Math.max(1, peakDownload(), peakUpload()));
  const state = () => connectionState(current(), ui.client.vpnOperation());
  const unavailable = () => ui.t("当前内核未提供统计", "Statistics unavailable from this engine");
  return (
    <Scroll bounces={false} style={{ flex: 1 }}>
      <Body>
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
        <SectionTitle label={ui.t("流量用量", "Traffic usage")} />
        <View style={{ flexDirection: "row", gap: 12 }}>
          <UsageCard
            title={ui.t("当前配置", "Current profile")}
            upload={profile()?.uploadBytes || 0}
            download={profile()?.downloadBytes || 0}
          />
          <UsageCard
            title={ui.t("本次会话", "This session")}
            upload={current().traffic.uploadBytes}
            download={current().traffic.downloadBytes}
          />
        </View>
        <SectionTitle label={ui.t("当前会话", "Current session")} />
        <View
          style={{
            minHeight: 92,
            padding: 16,
            borderWidth: 1,
            borderColor: palette().border,
            borderRadius: radius.lg,
            backgroundColor: palette().panel,
            flexDirection: windowSize().width < 480 ? "column" : "row",
            alignItems: "stretch",
            gap: 16,
          }}
        >
          <SpeedMetric download value={current().traffic.downloadSpeed} />
          <View
            style={{
              height: windowSize().width < 480 ? 1 : 48,
              width: windowSize().width < 480 ? "100%" : 1,
              backgroundColor: palette().border,
            }}
          />
          <SpeedMetric value={current().traffic.uploadSpeed} />
        </View>
        <Card
          title={ui.t("速率图表", "Speed chart")}
          subtitle={`${history().length} ${ui.t("个采样", "samples")}`}
        >
          <Detail label={ui.t("峰值下载", "Peak download")} value={`${bytes(peakDownload())}/s`} />
          <Detail label={ui.t("峰值上传", "Peak upload")} value={`${bytes(peakUpload())}/s`} />
          <View
            accessibilityLabel={ui.t("最近流量趋势", "Recent traffic history")}
            style={{
              height: 62,
              paddingHorizontal: 4,
              paddingTop: 7,
              paddingBottom: 3,
              borderRadius: radius.md,
              backgroundColor: palette().soft,
              flexDirection: "row",
              alignItems: "flex-end",
            }}
          >
            <Each of={history()} key="index">
              {(point) => (
                <View
                  style={{
                    flex: 1,
                    alignItems: "center",
                    justifyContent: "flex-end",
                    height: "100%",
                  }}
                >
                  <View
                    style={{
                      width: "72%",
                      borderRadius: 2,
                      height:
                        (Math.max(point().downloadSpeed, point().uploadSpeed) / peak()) * 49 + 3,
                      backgroundColor:
                        point().downloadSpeed >= point().uploadSpeed
                          ? palette().success
                          : palette().warning,
                    }}
                  />
                </View>
              )}
            </Each>
          </View>
        </Card>
        <Card
          title={ui.t("当前连接", "Current connections")}
          subtitle={
            ui.client.domains.data("traffic")?.connectionSummaryAvailable === false
              ? unavailable()
              : `${connections().count} ${ui.t("个连接", "connections")}`
          }
        >
          <Detail
            label={ui.t("总下载", "Total download")}
            value={
              ui.client.domains.data("traffic")?.connectionSummaryAvailable === false
                ? unavailable()
                : bytes(connections().downloadBytes)
            }
          />
          <Detail
            label={ui.t("总上传", "Total upload")}
            value={
              ui.client.domains.data("traffic")?.connectionSummaryAvailable === false
                ? unavailable()
                : bytes(connections().uploadBytes)
            }
          />
          <If
            when={
              ui.client.domains.data("traffic")?.connectionSummaryAvailable !== false &&
              connections().count === 0
            }
          >
            <Hint
              text={ui.t(
                "VPN 产生的 TCP 和 UDP 会话会显示在这里。",
                "TCP and UDP sessions from the VPN appear here.",
              )}
            />
          </If>
          <Each of={connections().recent} key={(row) => row.id}>
            {(row) => (
              <Detail
                stacked
                label={row().domain || row().host || row().destinationIp}
                value={`${bytes(row().downloadBytes)} ↓ · ${bytes(row().uploadBytes)} ↑`}
              />
            )}
          </Each>
          <Action
            id="traffic-connections"
            secondary
            label={ui.t("查看连接", "View connections")}
            onPress={() => ui.navigate("connections")}
          />
        </Card>
        <Card title={ui.t("DNS 诊断", "DNS diagnostics")} subtitle={current().dns.configurationError ? unavailable() : current().dns.model}>
          <Detail
            tone={
              (current().dns.hijacking ?? current().vpnOptions.dnsHijacking) ? "success" : "neutral"
            }
            label={ui.t("DNS 劫持", "DNS hijacking")}
            value={
              (current().dns.hijacking ?? current().vpnOptions.dnsHijacking)
                ? ui.t("开启", "Enabled")
                : ui.t("关闭", "Disabled")
            }
          />
          <Detail label={ui.t("监听", "Listener")} value={current().dns.configurationError ? unavailable() : current().dns.listen || "—"} />
          <Detail
            label="TUN"
            value={(current().dns.tunAddresses || current().vpnOptions.addresses).join(", ") || "—"}
          />
          <Detail
            label={ui.t("上游", "Upstreams")}
            value={current().dns.configurationError ? unavailable() : current().dns.upstreams.join("\n") || "—"}
          />
          <Detail label="Fallback" value={current().dns.configurationError ? unavailable() : current().dns.fallbacks.join("\n") || "—"} />
          <Detail
            label={ui.t("分流策略", "Nameserver policies")}
            value={current().dns.configurationError ? unavailable() : String(Object.keys(current().dns.nameserverPolicy || {}).length)}
          />
          <Detail
            label={ui.t("已处理", "Handled")}
            value={
              current().dns.statisticsAvailable
                ? String(current().dns.handledPackets)
                : unavailable()
            }
          />
          <Detail
            label={ui.t("缓存命中", "Cache hits")}
            value={
              current().dns.statisticsAvailable ? String(current().dns.cacheHits) : unavailable()
            }
          />
          <Detail
            label={ui.t("缓存回源", "Cache misses")}
            value={
              current().dns.statisticsAvailable ? String(current().dns.cacheMisses) : unavailable()
            }
          />
          <If when={current().dns.statisticsAvailable && current().dns.recentQueries.length > 0}>
            <Separator />
            <Hint text={ui.t("最近查询", "Recent queries")} />
            <Each
              of={current().dns.recentQueries}
              key={(query) => `${query.name}:${query.recordType}`}
            >
              {(query) => (
                <Detail label={query().name} value={`${query().recordType} · ${query().count}`} />
              )}
            </Each>
          </If>
          <Actions>
            <Action
              secondary
              label={ui.t("清理 DNS 缓存", "Flush DNS cache")}
              disabled={ui.client.pending() > 0}
              onPress={() =>
                void ui.client.run("dns.flush", {}, ui.t("DNS 缓存已清空", "DNS cache flushed"))
              }
            />
            <Action
              secondary
              label={ui.t("清理 Fake-IP", "Flush Fake-IP cache")}
              disabled={ui.client.pending() > 0}
              onPress={() =>
                void ui.client.run(
                  "dns.fakeIpFlush",
                  {},
                  ui.t("Fake-IP 缓存已清空", "Fake-IP cache flushed"),
                )
              }
            />
          </Actions>
        </Card>
        <Card
          title={ui.t("控制器诊断", "Controller diagnostics")}
          subtitle={current().controllerAddr || ui.t("未启动", "Not started")}
        >
          <Detail
            label={ui.t("内存使用", "Memory in use")}
            value={
              current().controllerDiagnostics.memoryAvailable === true
                ? bytes(current().controllerDiagnostics.memoryInUseBytes)
                : unavailable()
            }
          />
          <Detail
            label={ui.t("内存统计范围", "Memory scope")}
            value={
              current().controllerDiagnostics.memoryAvailable !== true
                ? unavailable()
                : current().controllerDiagnostics.memoryScope === "processResident"
                  ? ui.t(
                      "VPN 进程驻留内存（含宿主运行时）",
                      "VPN process resident memory, including its host runtime",
                    )
                  : ui.t("内核运行时堆内存", "Engine runtime heap")
            }
          />
          <Detail
            label={ui.t("内存上限", "Memory limit")}
            value={
              current().controllerDiagnostics.memoryAvailable !== true
                ? unavailable()
                : current().controllerDiagnostics.memoryLimitAvailable === true
                  ? bytes(current().controllerDiagnostics.memoryLimitBytes)
                  : ui.t("未提供", "Not provided")
            }
          />
          <Detail
            label={ui.t("配置同步次数", "Configuration syncs")}
            value={String(current().controllerDiagnostics.configSyncCount)}
          />
          <Detail
            label={ui.t("上次同步", "Last sync")}
            value={timestamp(current().controllerDiagnostics.lastConfigSyncAt)}
          />
          <If when={current().controllerDiagnostics.lastConfigSyncError}>
            <PageError id="traffic-config-sync-error" text={current().controllerDiagnostics.lastConfigSyncError || ""} />
          </If>
        </Card>
      </Body>
    </Scroll>
  );
}
function SectionTitle(props: { label: string }) {
  return (
    <Text style={{ fontSize: 14, fontWeight: "600", lineHeight: 20, color: palette().text }}>
      {props.label}
    </Text>
  );
}
function UsageCard(props: { title: string; upload: number; download: number }) {
  return (
    <View
      style={{
        flex: 1,
        minWidth: 0,
        height: 126,
        padding: 12,
        gap: 10,
        borderWidth: 1,
        borderColor: palette().border,
        borderRadius: radius.lg,
        backgroundColor: palette().panel,
      }}
    >
      <Text numberOfLines={1} style={{ fontSize: 14, fontWeight: "600", color: palette().text }}>
        {props.title}
      </Text>
      <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
        <Icon name="arrow-up" size={14} color={palette().warning} />
        <Text style={{ fontSize: 14, color: palette().text }}>{bytes(props.upload)}</Text>
      </View>
      <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
        <Icon name="arrow-down" size={14} color={palette().success} />
        <Text style={{ fontSize: 14, color: palette().text }}>{bytes(props.download)}</Text>
      </View>
    </View>
  );
}
function SpeedMetric(props: { download?: boolean; value: number }) {
  const ui = useApp();
  return (
    <View
      style={{
        flex: windowSize().width < 480 ? undefined : 1,
        minWidth: 0,
        flexDirection: "row",
        alignItems: "center",
        gap: 10,
      }}
    >
      <View
        style={{
          width: 38,
          height: 38,
          borderRadius: radius.md,
          backgroundColor: palette().soft,
          alignItems: "center",
          justifyContent: "center",
        }}
      >
        <Icon
          name={props.download ? "arrow-down" : "arrow-up"}
          size={18}
          color={props.download ? palette().success : palette().warning}
        />
      </View>
      <View style={{ flex: 1, minWidth: 0, gap: 4 }}>
        <Text style={{ color: palette().muted, fontSize: 12 }}>
          {props.download ? ui.t("下载", "Download") : ui.t("上传", "Upload")}
        </Text>
        <Text numberOfLines={1} style={{ color: palette().text, fontSize: 18, fontWeight: "600" }}>
          {bytes(props.value)}/s
        </Text>
      </View>
    </View>
  );
}

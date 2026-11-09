import { computed, signal, Text, View, Clipboard, windowSize, fontScale, List } from "nsh";
import {
  Action,
  Actions,
  Badge,
  control,
  Detail,
  Dialog,
  Empty,
  Field,
  IconButton,
  palette,
  radius,
  Tabs,
  useApp} from "../components";
import { ObjectMenu } from "../object-menu";
import { feedbackStyle } from "../motion";
import { focusRing } from "../theme";
import { bytes, timestamp } from "../locale";
import { matchesConnection } from "../query";
import { ManualRuleDialog, type RuleContext } from "../manual-rule";
import type { Connection } from "../model";

export function Activity(props: { history?: boolean }) {
  const ui = useApp(),
    query = signal(ui.query()),
    network = signal("all"),
    status = signal("all"),
    selected = signal<Connection | undefined>(),
    manual = signal<RuleContext | undefined>();
  const rows = computed(() =>
    (props.history ? ui.client.state()!.requestHistory : ui.client.state()!.connections).filter(
      (row) =>
        matchesConnection(row, query()) &&
        (network() === "all" || row.network.toLowerCase() === network()) &&
        (status() === "all" || (status() === "active" ? row.active : !row.active)),
    ),
  );
  const rule = (row: Connection) =>
    manual.set({
      domain: row.domain || undefined,
      destinationIp: row.destinationIp,
      connectionId: !props.history || row.active ? row.id : undefined,
    });
  return (
    <View
      style={{
        flex: 1,
        paddingHorizontal: windowSize().width >= 840 ? 24 : 16,
        paddingTop: windowSize().width >= 840 ? 24 : 16,
      }}
    >
      <View style={{ gap: 12, marginBottom: 12 }}>
        <Field
          label={ui.t("搜索连接", "Search connections")}
          placeholder={ui.t("输入域名、规则、代理或网络", "Search host, rule, proxy or network")}
          value={query()}
          onChange={(text) => query.set(text)}
        />
        <Tabs value={network()} options={[
          { value: "all", label: ui.t("全部网络", "All networks") },
          { value: "tcp", label: "TCP" }, { value: "udp", label: "UDP" },
        ]} onChange={(value) => network.set(value)} />
        <If when={props.history}>
          <Tabs
            value={status()}
            options={[
              { value: "all", label: ui.t("全部", "All") },
              { value: "active", label: ui.t("活跃", "Active") },
              { value: "ended", label: ui.t("已结束", "Ended") },
            ]}
            onChange={(value) => status.set(value)}
          />
        </If>
      </View>
      <List
        bounces={false}
        each={rows()}
        keyExtractor={(row) => row.id}
        estimatedItemSize={128}
        style={{ flex: 1, borderWidth: 1, borderColor: palette().border,
          borderRadius: radius.lg, backgroundColor: palette().panel, overflow: "hidden" }}
        contentStyle={{ paddingBottom: 8 }}
        empty={
          <Empty
            icon="activity"
            title={ui.t("暂无记录", "No records")}
            text={ui.t("连接活动出现后将在这里显示", "Connection activity will appear here")}
          />
        }
      >
        {(row) => <ObjectMenu id={`activity-menu-${row().id}`}
          title={row().domain || row().host || row().destinationIp}
          commands={[
            { id: `activity-details-${row().id}`, label: ui.t("连接详情…", "Connection details…"), icon: "info",
              run: () => selected.set(row()) },
            { label: ui.t("复制地址", "Copy address"), icon: "file-text",
              run: () => Clipboard.writeText(row().domain || row().host || row().destinationIp) },
            {
              id: `activity-close-${props.history ? "request" : "connection"}-${row().id}`,
              label: ui.t("关闭连接", "Close connection"), icon: "unplug" as const, danger: true,
              disabled: !!props.history && !row().active || ui.client.pending() > 0 || !ui.client.domains.fresh([props.history ? "requests" : "connections"]),
              run: () => { void ui.client.run("connections.close", { id: row().id }, ui.t("连接已断开", "Connection closed")); },
            },
          ]}
          row={(menu) => <View testID={`activity-row-${props.history ? "request" : "connection"}-${row().id}`}
            style={{ minHeight: Math.max(120, 56 * Math.max(1, fontScale()) + 48),
              paddingHorizontal: 12, paddingVertical: 8, gap: 4,
              borderBottomWidth: 1, borderBottomColor: palette().line }}>
            <View testID={`activity-header-${props.history ? "request" : "connection"}-${row().id}`}
              style={{ minHeight: control.icon, flexDirection: "row", alignItems: "center", gap: 8 }}>
              <View testID={`activity-title-${props.history ? "request" : "connection"}-${row().id}`}
                role="button" accessibilityLabel={row().domain || row().host || row().destinationIp}
                focusStyle={focusRing()} hoverStyle={{ backgroundColor: palette().soft }}
                onPress={() => props.history
                  ? ui.navigate("connections", row().domain || row().host || row().destinationIp)
                  : selected.set(row())}
                style={{ minHeight: control.icon, justifyContent: "center", flex: 1, minWidth: 0,
                  borderRadius: radius.sm, ...feedbackStyle() }}>
                <Text style={{ numberOfLines: 1, fontSize: 14, lineHeight: 20, fontWeight: "600", color: palette().text }}>
                  {row().domain || row().host || row().destinationIp}
                </Text>
              </View>
              <View testID={`activity-actions-${props.history ? "request" : "connection"}-${row().id}`}
                style={{ width: control.icon * 2 + 8, flexShrink: 0, flexDirection: "row", alignItems: "center", gap: 8 }}>
                <IconButton id={`rule-for-${row().id}`} icon="plus" label={ui.t("添加规则…", "Add rule…")}
                  onPress={() => rule(row())} />
                {menu}
              </View>
            </View>
            <View testID={`activity-meta-${props.history ? "request" : "connection"}-${row().id}`}
              style={{ minHeight: 22 * Math.max(1, fontScale()), flexDirection: "row", alignItems: "center", gap: 8 }}>
              <If when={props.history}>
                <Badge tone={row().active ? "success" : "neutral"} label={row().active ? ui.t("活跃", "Active") : ui.t("已结束", "Ended")} />
              </If>
              <Text style={{ numberOfLines: 1, flex: 1, minWidth: 0, fontSize: 12, lineHeight: 18, color: palette().muted }}>
                {row().network.toUpperCase()} · {row().proxy} · {row().rule}
              </Text>
            </View>
            <Text style={{ numberOfLines: 1, fontSize: 12, lineHeight: 18, color: palette().muted, fontVariant: ["tabular-nums"] }}>
              ↓ {bytes(row().downloadBytes)}  ↑ {bytes(row().uploadBytes)} · {timestamp(props.history ? row().updatedAt : row().startedAt)}
            </Text>
          </View>} />}
      </List>
      <If when={selected()}>
        <Dialog title={ui.t("连接详情", "Connection details")} onClose={() => selected.set(undefined)}
          footer={<Actions><Action label={ui.t("复制地址", "Copy address")}
            onPress={() => Clipboard.writeText(selected()!.domain || selected()!.host || selected()!.destinationIp)} /></Actions>}>
          <Detail stacked label={ui.t("目标", "Destination")} value={selected()!.domain || selected()!.host || selected()!.destinationIp} />
          <Detail label="IP" value={selected()!.destinationIp} />
          <Detail label={ui.t("网络", "Network")} value={selected()!.network.toUpperCase()} />
          <Detail stacked label={ui.t("代理链", "Proxy chain")} value={selected()!.chains?.join(" → ") || selected()!.proxy} />
          <Detail stacked label={ui.t("规则", "Rule")} value={selected()!.rule} />
          <Detail label={ui.t("下载", "Download")} value={bytes(selected()!.downloadBytes)} />
          <Detail label={ui.t("上传", "Upload")} value={bytes(selected()!.uploadBytes)} />
          <Detail label={ui.t("开始时间", "Started")} value={timestamp(selected()!.startedAt)} />
          <Detail label={ui.t("更新时间", "Updated")} value={timestamp(selected()!.updatedAt)} />
        </Dialog>
      </If>
      <If when={manual()}>
        <ManualRuleDialog context={manual()} onClose={() => manual.set(undefined)} />
      </If>
    </View>
  );
}

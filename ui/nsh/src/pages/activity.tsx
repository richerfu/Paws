import { computed, signal, Text, View } from "nsh";
import { Clipboard } from "nsh/clipboard";
import { windowSize } from "nsh/device";
import { List } from "nsh/list";
import {
  Action,
  Actions,
  Badge,
  confirm,
  Detail,
  Dialog,
  Empty,
  Field,
  IconButton,
  palette,
  radius,
  Select,
  Tabs,
  useApp,
} from "../components";
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
  const clear = () =>
    void confirm(
      ui,
      props.history
        ? ui.t("清空请求记录？", "Clear request history?")
        : ui.t("关闭全部活动连接？", "Close all active connections?"),
      () => void ui.client.run(props.history ? "requests.clear" : "connections.closeAll"),
    );
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
          label=""
          placeholder={ui.t("输入域名、规则、代理或网络", "Search host, rule, proxy or network")}
          value={query()}
          onChange={(text) => query.set(text)}
        />
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
        estimatedItemSize={112}
        style={{ flex: 1 }}
        contentContainerStyle={{ gap: 8, paddingBottom: 16 }}
        ListEmptyComponent={
          <Empty
            icon="activity"
            title={ui.t("暂无记录", "No records")}
            text={ui.t("连接活动出现后将在这里显示", "Connection activity will appear here")}
          />
        }
      >
        {(row) => (
          <View
            testID={`activity-row-${props.history ? "request" : "connection"}-${row().id}`}
            style={{
              height: 104,
              paddingHorizontal: 14,
              paddingVertical: 12,
              borderWidth: 1,
              borderColor: palette().border,
              borderRadius: radius.lg,
              backgroundColor: palette().panel,
              gap: 4,
            }}
          >
            <View
              testID={`activity-header-${props.history ? "request" : "connection"}-${row().id}`}
              style={{ height: 32, flexDirection: "row", alignItems: "center", gap: 8 }}
            >
              <View
                testID={`activity-title-${props.history ? "request" : "connection"}-${row().id}`}
                accessibilityRole={props.history ? "button" : undefined}
                accessibilityLabel={row().domain || row().host || row().destinationIp}
                onPress={
                  props.history
                    ? () =>
                        ui.navigate(
                          "connections",
                          row().domain || row().host || row().destinationIp,
                        )
                    : undefined
                }
                style={{ height: 32, justifyContent: "center", flex: 1, minWidth: 0 }}
              >
                <Text
                  numberOfLines={1}
                  style={{ fontSize: 14, lineHeight: 20, fontWeight: "600", color: palette().text }}
                >
                  {row().domain || row().host || row().destinationIp}
                </Text>
              </View>
              <View
                testID={`activity-actions-${props.history ? "request" : "connection"}-${row().id}`}
                style={{
                  width: 72,
                  height: 32,
                  flexShrink: 0,
                  flexDirection: "row",
                  alignItems: "center",
                  gap: 8,
                }}
              >
                <IconButton
                  id={`rule-for-${row().id}`}
                  small
                  icon="plus"
                  label={ui.t("添加规则", "Add rule")}
                  onPress={() => rule(row())}
                />
                <If
                  when={!props.history || row().active}
                  else={
                    <View
                      testID={`activity-close-slot-request-${row().id}`}
                      accessible={false}
                      pointerEvents="none"
                      style={{ width: 32, height: 32 }}
                    />
                  }
                >
                  <IconButton
                    id={`activity-close-${props.history ? "request" : "connection"}-${row().id}`}
                    small
                    icon="x"
                    label={ui.t("关闭连接", "Close connection")}
                    disabled={
                      ui.client.pending() > 0 ||
                      !ui.client.domains.fresh([props.history ? "requests" : "connections"])
                    }
                    onPress={() =>
                      void ui.client.run(
                        "connections.close",
                        { id: row().id },
                        ui.t("连接已断开", "Connection closed"),
                      )
                    }
                  />
                </If>
              </View>
            </View>
            <View
              testID={`activity-meta-${props.history ? "request" : "connection"}-${row().id}`}
              style={{ height: 22, flexDirection: "row", alignItems: "center", gap: 8 }}
            >
              <If when={props.history}>
                <View style={{ flexShrink: 0, justifyContent: "center" }}>
                  <Badge
                    tone={row().active ? "success" : "neutral"}
                    label={row().active ? ui.t("活跃", "Active") : ui.t("已结束", "Ended")}
                  />
                </View>
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
                {row().network.toUpperCase()} · {row().proxy} · {row().rule}
                {row().chains?.length ? " · " + row().chains!.join(" > ") : ""}
              </Text>
            </View>
            <Text
              numberOfLines={1}
              style={{ fontSize: 12, lineHeight: 16, color: palette().muted }}
            >
              ↓ {bytes(row().downloadBytes)} ↑ {bytes(row().uploadBytes)} ·{" "}
              {timestamp(props.history ? row().updatedAt : row().startedAt)}
            </Text>
          </View>
        )}
      </List>
      <If when={manual()}>
        <ManualRuleDialog context={manual()} onClose={() => manual.set(undefined)} />
      </If>
    </View>
  );
}

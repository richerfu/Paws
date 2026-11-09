import { Text, View, windowSize, type JSX } from "nsh";
import { Badge, IconButton, useApp } from "./components";
import type { Connection } from "./model";
import { palette, spacing, focusRing, radius } from "./theme";
import { bytes, timestamp } from "./locale";

export function ActivityTableHeader() {
  const ui = useApp();
  return (
    <View
      testID="activity-table-header"
      style={{
        minHeight: 36,
        flexDirection: "row",
        alignItems: "center",
        gap: spacing.md,
        paddingHorizontal: spacing.md,
        borderBottomWidth: 1,
        borderBottomColor: palette().border,
        backgroundColor: palette().soft,
      }}
    >
      <Text
        style={{ flex: 2, minWidth: 0, fontSize: 12, color: palette().muted }}
      >
        {ui.t("目标地址", "Destination")}
      </Text>
      <If when={windowSize().width >= 1000}>
        <Text style={{ width: 64, fontSize: 12, color: palette().muted }}>
          {ui.t("网络", "Network")}
        </Text>
        <Text
          style={{ flex: 1, minWidth: 0, fontSize: 12, color: palette().muted }}
        >
          {ui.t("代理", "Proxy")}
        </Text>
        <Text
          style={{ flex: 1, minWidth: 0, fontSize: 12, color: palette().muted }}
        >
          {ui.t("规则", "Rule")}
        </Text>
      </If>
      <Text style={{ width: 120, fontSize: 12, color: palette().muted }}>
        {ui.t("传输", "Transfer")}
      </Text>
      <Text style={{ width: 72, fontSize: 12, color: palette().muted }}>
        {ui.t("操作", "Actions")}
      </Text>
    </View>
  );
}

export function ActivityTableRow(props: {
  connection: Connection;
  history: boolean;
  menu: JSX.Element;
  onDetails: () => void;
  onRule: () => void;
}) {
  const ui = useApp();
  const host = () =>
    props.connection.domain ||
    props.connection.host ||
    props.connection.destinationIp;
  return (
    <View
      testID={`activity-table-row-${props.connection.id}`}
      style={{
        minHeight: 72,
        flexDirection: "row",
        alignItems: "center",
        gap: spacing.md,
        paddingHorizontal: spacing.md,
        paddingVertical: spacing.sm,
        borderBottomWidth: 1,
        borderBottomColor: palette().line,
      }}
    >
      <View
        role="button"
        testID={`activity-table-destination-${props.connection.id}`}
        accessibilityLabel={host()}
        onPress={props.onDetails}
        focusStyle={focusRing()}
        hoverStyle={{ backgroundColor: palette().soft }}
        style={{
          flex: 2,
          minWidth: 0,
          minHeight: 40,
          justifyContent: "center",
          gap: spacing.xxs,
          borderRadius: radius.sm,
        }}
      >
        <Text
          style={{
            numberOfLines: 1,
            fontSize: 14,
            lineHeight: 20,
            fontWeight: "600",
            color: palette().text,
          }}
        >
          {host()}
        </Text>
        <View
          style={{
            flexDirection: "row",
            alignItems: "center",
            gap: spacing.sm,
          }}
        >
          <If when={props.history}>
            <Badge
              tone={props.connection.active ? "success" : "neutral"}
              label={
                props.connection.active
                  ? ui.t("活跃", "Active")
                  : ui.t("已结束", "Ended")
              }
            />
          </If>
          <Text
            style={{
              flex: 1,
              numberOfLines: 1,
              fontSize: 12,
              lineHeight: 18,
              color: palette().muted,
            }}
          >
            {timestamp(
              props.history
                ? props.connection.updatedAt
                : props.connection.startedAt,
            )}
          </Text>
        </View>
        <If when={windowSize().width < 1000}>
          <Text
            style={{
              numberOfLines: 1,
              fontSize: 12,
              lineHeight: 18,
              color: palette().muted,
            }}
          >
            {props.connection.network.toUpperCase()} · {props.connection.proxy}{" "}
            · {props.connection.rule}
          </Text>
        </If>
      </View>
      <If when={windowSize().width >= 1000}>
        <Text style={{ width: 64, fontSize: 12, color: palette().muted }}>
          {props.connection.network.toUpperCase()}
        </Text>
        <Text
          accessibilityLabel={props.connection.proxy}
          style={{
            flex: 1,
            minWidth: 0,
            numberOfLines: 1,
            fontSize: 13,
            lineHeight: 20,
            color: palette().text,
          }}
        >
          {props.connection.proxy}
        </Text>
        <Text
          accessibilityLabel={props.connection.rule}
          style={{
            flex: 1,
            minWidth: 0,
            numberOfLines: 1,
            fontSize: 13,
            lineHeight: 20,
            color: palette().muted,
          }}
        >
          {props.connection.rule}
        </Text>
      </If>
      <View style={{ width: 120, gap: spacing.xxs }}>
        <Text
          style={{
            fontSize: 12,
            lineHeight: 18,
            fontVariant: ["tabular-nums"],
            color: palette().text,
          }}
        >
          ↓ {bytes(props.connection.downloadBytes)}
        </Text>
        <Text
          style={{
            fontSize: 12,
            lineHeight: 18,
            fontVariant: ["tabular-nums"],
            color: palette().muted,
          }}
        >
          ↑ {bytes(props.connection.uploadBytes)}
        </Text>
      </View>
      <View
        testID={`activity-table-actions-${props.connection.id}`}
        style={{
          width: 72,
          flexDirection: "row",
          alignItems: "center",
          gap: spacing.sm,
        }}
      >
        <IconButton
          id={`rule-for-${props.connection.id}`}
          icon="plus"
          label={ui.t("添加规则…", "Add rule…")}
          onPress={props.onRule}
        />
        {props.menu}
      </View>
    </View>
  );
}

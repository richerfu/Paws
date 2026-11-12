import { Text, View, fontScale, type JSX } from "nsh";
import { Badge, IconButton, useApp, useLayout } from "./components";
import type { Connection } from "./model";
import {
  palette,
  spacing,
  focusRing,
  radius,
  typography,
  control,
  density,
  type StatusTone,
} from "./theme";
import { bytes, timestamp } from "./locale";
import { feedbackStyle } from "./motion";

export function isBlockedConnection(connection: Connection): boolean {
  const haystack =
    `${connection.proxy || ""} ${connection.chains?.join(" ") || ""} ${connection.rule || ""}`.toUpperCase();
  return (
    haystack.includes("REJECT") ||
    haystack.includes("BLOCK") ||
    connection.proxy === "广告拦截"
  );
}

export function historyStatusTone(connection: Connection): StatusTone {
  if (connection.active) return "success";
  if (isBlockedConnection(connection)) return "warning";
  return "neutral";
}

export function historyStatusLabel(
  connection: Connection,
  t: (zh: string, en: string) => string,
): string {
  if (connection.active) return t("活跃", "Active");
  if (isBlockedConnection(connection)) return t("已拦截", "Blocked");
  return t("已结束", "Ended");
}

export function formatActivityTime(raw?: string): string {
  const formatted = timestamp(raw);
  if (formatted === "—") return "—";
  const parts = formatted.split(" ");
  return parts.length > 1 ? parts[parts.length - 1]! : formatted;
}

export function formatActivityElapsed(
  startedAt: string | undefined,
  t: (zh: string, en: string) => string,
): string {
  if (!startedAt) return "—";
  const numeric = Number(startedAt);
  const ms = Number.isFinite(numeric)
    ? numeric > 1e17
      ? numeric / 1e6
      : numeric > 1e14
        ? numeric / 1e3
        : numeric > 1e11
          ? numeric
          : numeric * 1000
    : Date.parse(startedAt);
  if (!Number.isFinite(ms)) return formatActivityTime(startedAt);
  const seconds = Math.floor((Date.now() - ms) / 1000);
  if (seconds < 0 || seconds > 86400 * 7) return formatActivityTime(startedAt);
  const compact =
    seconds < 60
      ? `${seconds}s`
      : seconds < 3600
        ? `${Math.floor(seconds / 60)}m`
        : `${Math.floor(seconds / 3600)}h`;
  return t(`持续 ${compact}`, compact);
}

export function formatRouteChain(connection: Connection): string {
  const chains = connection.chains?.filter(Boolean);
  if (chains && chains.length > 0) return chains.join(" → ");
  if (
    connection.rule &&
    connection.proxy &&
    connection.rule !== connection.proxy
  ) {
    return `${connection.rule} → ${connection.proxy}`;
  }
  return connection.proxy || connection.rule || "—";
}

export function ActivityTableHeader(props: {
  history?: boolean;
  usable?: number;
}) {
  const ui = useApp(),
    layout = useLayout();
  const available = () => props.usable ?? layout().usable;
  const details = () => available() >= 760 * Math.max(1, fontScale());
  const transfer = () => available() >= 520 * Math.max(1, fontScale());
  const actionsWidth = () => control.icon * 2 + 4;
  return (
    <View
      testID="activity-table-header"
      style={{
        minHeight: 32,
        flexDirection: "row",
        alignItems: "center",
        gap: spacing.md,
        paddingHorizontal: 14,
        borderBottomWidth: 1,
        borderBottomColor: palette().border,
        backgroundColor: palette().soft,
      }}
    >
      <Text
        style={{
          flex: 2,
          minWidth: 0,
          fontSize: typography.xs,
          lineHeight: density.line(16),
          fontWeight: "600",
          color: palette().muted,
        }}
      >
        {details()
          ? ui.t("目标地址", "Destination")
          : ui.t("目标与命中规则", "Destination & rule")}
      </Text>
      <If when={details()}>
        <Text
          style={{
            flex: 1.5,
            minWidth: 0,
            fontSize: typography.xs,
            lineHeight: density.line(16),
            fontWeight: "600",
            color: palette().muted,
          }}
        >
          {ui.t("规则 / 代理链", "Rule / Chain")}
        </Text>
      </If>
      <Text
        style={{
          width: props.history ? 84 : 72,
          fontSize: typography.xs,
          lineHeight: density.line(16),
          fontWeight: "600",
          color: palette().muted,
        }}
      >
        {props.history
          ? ui.t("状态", "Status")
          : ui.t("网络", "Network")}
      </Text>
      <If when={props.history && details()}>
        <Text
          style={{
            width: 84,
            fontSize: typography.xs,
            lineHeight: density.line(16),
            fontWeight: "600",
            color: palette().muted,
          }}
        >
          {ui.t("时间", "Time")}
        </Text>
      </If>
      <If when={transfer()}>
        <Text
          style={{
            width: props.history ? 96 : 112,
            fontSize: typography.xs,
            lineHeight: density.line(16),
            fontWeight: "600",
            textAlign: "right",
            color: palette().muted,
          }}
        >
          {ui.t("流量", "Traffic")}
        </Text>
      </If>
      <Text
        style={{
          width: actionsWidth(),
          fontSize: typography.xs,
          lineHeight: density.line(16),
          fontWeight: "600",
          textAlign: "right",
          color: palette().muted,
        }}
      >
        {ui.t("操作", "Actions")}
      </Text>
    </View>
  );
}

export function ActivityTableRow(props: {
  connection: Connection;
  history: boolean;
  selected?: boolean;
  usable?: number;
  menu: JSX.Element;
  onDetails: () => void;
  onRule: () => void;
}) {
  const ui = useApp(),
    layout = useLayout();
  const available = () => props.usable ?? layout().usable;
  const details = () => available() >= 760 * Math.max(1, fontScale());
  const transfer = () => available() >= 520 * Math.max(1, fontScale());
  const actionsWidth = () => control.icon * 2 + 4;
  const host = () =>
    props.connection.domain ||
    props.connection.host ||
    props.connection.destinationIp;
  const totalBytes = () =>
    bytes(props.connection.downloadBytes + props.connection.uploadBytes);
  return (
    <View
      testID={`activity-table-row-${props.connection.id}`}
      style={{
        minHeight: 40,
        flexDirection: "row",
        alignItems: "center",
        gap: spacing.md,
        paddingHorizontal: 14,
        paddingVertical: 6,
        borderBottomWidth: 1,
        borderBottomColor: palette().line,
        backgroundColor: props.selected ? palette().accentFill : "transparent",
      }}
    >
      <View
        role="button"
        testID={`activity-table-destination-${props.connection.id}`}
        accessibilityLabel={host()}
        onPress={props.onDetails}
        focusStyle={focusRing()}
        hoverStyle={{ backgroundColor: palette().hover }}
        activeStyle={{ backgroundColor: palette().soft }}
        style={{
          flex: 2,
          minWidth: 0,
          minHeight: 34,
          justifyContent: "center",
          gap: 2,
          borderRadius: radius.sm,
          borderCurve: "continuous",
          ...feedbackStyle(),
        }}
      >
        <Text
          style={{
            minWidth: 0,
            numberOfLines: 1,
            fontSize: density.font(13),
            lineHeight: density.line(20),
            fontWeight: props.selected ? "600" : "500",
            color: palette().text,
          }}
        >
          {host()}
        </Text>
        <If when={!details()}>
          <Text
            style={{
              numberOfLines: 1,
              fontSize: density.font(12.5),
              lineHeight: density.line(17),
              color: palette().muted,
            }}
          >
            {formatRouteChain(props.connection)}
            {!transfer() ? ` · ${totalBytes()}` : ""}
          </Text>
        </If>
      </View>
      <If when={details()}>
        <Text
          accessibilityLabel={formatRouteChain(props.connection)}
          style={{
            flex: 1.5,
            minWidth: 0,
            numberOfLines: 1,
            fontSize: typography.note,
            lineHeight: density.line(18),
            color: palette().muted,
          }}
        >
          {formatRouteChain(props.connection)}
        </Text>
      </If>
      <View
        style={{
          width: props.history ? 84 : 72,
          justifyContent: "center",
          alignItems: "flex-start",
        }}
      >
        <Badge
          tone={
            props.history ? historyStatusTone(props.connection) : "neutral"
          }
          label={
            props.history
              ? historyStatusLabel(props.connection, ui.t)
              : props.connection.network.toUpperCase()
          }
        />
      </View>
      <If when={props.history && details()}>
        <Text
          style={{
            width: 84,
            fontSize: typography.note,
            lineHeight: density.line(18),
            fontVariant: ["tabular-nums"],
            color: palette().muted,
          }}
        >
          {formatActivityTime(props.connection.startedAt)}
        </Text>
      </If>
      <If when={transfer()}>
        <View
          style={{
            width: props.history ? 96 : 112,
            alignItems: "flex-end",
            justifyContent: "center",
          }}
        >
          <Text
            style={{
              fontSize: density.font(13.5),
              lineHeight: density.line(18),
              fontWeight: "500",
              fontVariant: ["tabular-nums"],
              color: palette().text,
            }}
          >
            {totalBytes()}
          </Text>
        </View>
      </If>
      <View
        testID={`activity-table-actions-${props.connection.id}`}
        style={{
          width: actionsWidth(),
          flexDirection: "row",
          alignItems: "center",
          justifyContent: "flex-end",
          gap: 4,
        }}
      >
        <IconButton
          id={`rule-for-${props.connection.id}`}
          icon="plus"
          iconColor={palette().muted}
          label={ui.t("添加规则…", "Add rule…")}
          onPress={props.onRule}
        />
        {props.menu}
      </View>
    </View>
  );
}

import { density, typography } from "../theme";
import {
  computed,
  signal,
  Text,
  View,
  fontScale,
  List,
  platform,
  Scroll,
} from "nsh";
import { Button } from "nsh/base";
import {
  ActivityTableHeader,
  ActivityTableRow,
  formatActivityElapsed,
  formatActivityTime,
  formatRouteChain,
  historyStatusLabel,
  historyStatusTone,
  isBlockedConnection,
} from "../activity-table";
import {
  Action,
  Actions,
  Badge,
  Body,
  BottomSheet,
  CompactPicker,
  copyText,
  Detail,
  Dialog,
  Empty,
  Hint,
  Icon,
  IconButton,
  ListSurface,
  palette,
  radius,
  SearchField,
  SectionHeader,
  Separator,
  SheetActionItem,
  spacing,
  Tabs,
  useApp,
  useLayout,
  useViewState,
  Well,
} from "../components";
import { ObjectMenu } from "../object-menu";
import { animateDisclosure, feedbackStyle } from "../motion";
import { focusRing, surfaceFinish } from "../theme";
import { bytes, timestamp } from "../locale";
import { matchesConnection } from "../query";
import { ManualRuleDialog, type RuleContext } from "../manual-rule";
import type { Connection } from "../model";

function parseConnectionMs(raw?: string): number {
  if (!raw) return 0;
  const numeric = Number(raw);
  if (Number.isFinite(numeric)) {
    return numeric > 1e17
      ? numeric / 1e6
      : numeric > 1e14
        ? numeric / 1e3
        : numeric > 1e11
          ? numeric
          : numeric * 1000;
  }
  const parsed = Date.parse(raw);
  return Number.isFinite(parsed) ? parsed : 0;
}

export function Activity(props: { history?: boolean }) {
  const table = () => platform.desktop;
  const ui = useApp(),
    layout = useLayout(),
    saved = useViewState(
      `activity.${props.history ? "requests" : "connections"}`,
      () => ({
        query: signal(ui.query()),
        network: signal("all"),
        status: signal("all"),
        sort: signal<"traffic" | "time">("traffic"),
        selectedId: signal(""),
        captured: signal<Connection | undefined>(),
      }),
    ),
    query = saved.query,
    network = saved.network,
    status = saved.status,
    sort = saved.sort,
    selectedId = saved.selectedId,
    captured = saved.captured,
    manual = signal<RuleContext | undefined>();

  const sourceRows = () =>
    (props.history
      ? ui.client.state()?.requestHistory
      : ui.client.state()?.connections) || [];

  const rows = computed(() => {
    const matched = sourceRows().filter((row) => {
      if (!matchesConnection(row, query())) return false;
      if (network() !== "all" && row.network.toLowerCase() !== network())
        return false;
      if (!props.history || status() === "all") return true;
      if (status() === "active") return row.active;
      if (status() === "blocked") return isBlockedConnection(row);
      return !row.active && !isBlockedConnection(row);
    });
    if (props.history) return matched;
    const ordered = matched.slice();
    if (sort() === "time") {
      ordered.sort(
        (a, b) =>
          parseConnectionMs(b.startedAt) - parseConnectionMs(a.startedAt),
      );
    } else {
      ordered.sort(
        (a, b) =>
          b.downloadBytes +
          b.uploadBytes -
          (a.downloadBytes + a.uploadBytes),
      );
    }
    return ordered;
  });

  const filtered = () =>
    !!query() ||
    network() !== "all" ||
    (!!props.history && status() !== "all");

  const clearFilters = () => {
    query.set("");
    network.set("all");
    status.set("all");
  };

  const splitPad = () => !platform.desktop && layout().masterDetail;

  const liveSelected = () => {
    const id = selectedId();
    if (!id) return undefined;
    return sourceRows().find((row) => row.id === id);
  };

  const activeSelection = () => {
    if (selectedId()) return liveSelected() || captured();
    if (splitPad()) return rows()[0];
    return undefined;
  };

  const splitDesktop = () =>
    platform.desktop &&
    layout().masterDetail &&
    !!selectedId() &&
    !!activeSelection();

  const sheetSelection = () =>
    !platform.desktop && !splitPad() && selectedId()
      ? activeSelection()
      : undefined;

  const modalSelection = () =>
    platform.desktop && !layout().masterDetail && selectedId()
      ? activeSelection()
      : undefined;

  const tableUsable = () =>
    splitDesktop()
      ? Math.max(320, layout().usable - 330 - layout().gap)
      : layout().usable;

  const selectInPane = (row: Connection) => {
    if (platform.desktop && layout().masterDetail && !selectedId()) {
      animateDisclosure();
    }
    selectedId.set(row.id);
    captured.set(row);
  };

  const clearSelection = () => {
    if (platform.desktop && layout().masterDetail && selectedId()) {
      animateDisclosure();
    }
    selectedId.set("");
    captured.set(undefined);
  };

  const showDetails = (row: Connection) => {
    selectInPane(row);
  };

  const rule = (row: Connection) =>
    manual.set({
      domain: row.domain || undefined,
      destinationIp: row.destinationIp,
      connectionId: !props.history || row.active ? row.id : undefined,
    });

  const modeTabs = (inline?: boolean) => (
    <Tabs
      inline={inline}
      idPrefix="activity-view"
      value={props.history ? "requests" : "connections"}
      options={[
        {
          value: "connections",
          label: ui.t("活动连接", "Active"),
          count: ui.client.state()?.connections?.length,
        },
        {
          value: "requests",
          label: ui.t("请求历史", "History"),
          count: ui.client.state()?.requestHistory?.length,
        },
      ]}
      onChange={(value) =>
        ui.switchActivity?.(value as "connections" | "requests")
      }
    />
  );

  const filterBar = (desktopInline?: boolean) => (
    <View
      style={{
        flexDirection: "row",
        alignItems: "center",
        gap: 8,
      }}
    >
      <Tabs
        size="small"
        flex={desktopInline ? undefined : 1}
        inline={desktopInline ? true : undefined}
        idPrefix="activity-net"
        value={network()}
        options={[
          { value: "all", label: ui.t("全部", "All") },
          { value: "tcp", label: "TCP" },
          { value: "udp", label: "UDP" },
        ]}
        onChange={(value) => network.set(value)}
      />
      <If
        when={props.history}
        else={
          <CompactPicker
            id="activity-sort"
            label={ui.t("排序方式", "Sort by")}
            value={sort()}
            options={[
              { value: "traffic", label: ui.t("按流量", "By traffic") },
              { value: "time", label: ui.t("按时间", "By time") },
            ]}
            onChange={(value) => sort.set(value as "traffic" | "time")}
          />
        }
      >
        <CompactPicker
          id="activity-status"
          label={ui.t("状态筛选", "Status filter")}
          value={status()}
          options={[
            { value: "all", label: ui.t("全部状态", "All status") },
            { value: "active", label: ui.t("活跃", "Active") },
            { value: "ended", label: ui.t("已结束", "Ended") },
            { value: "blocked", label: ui.t("已拦截", "Blocked") },
          ]}
          onChange={(value) => status.set(value)}
        />
      </If>
    </View>
  );

  const countBar = () => (
    <View
      style={{
        minHeight: 20,
        flexDirection: "row",
        alignItems: "center",
        justifyContent: "space-between",
        gap: spacing.sm,
        paddingHorizontal: 2,
      }}
    >
      <Text
        style={{
          fontSize: typography.note,
          lineHeight: density.line(18),
          color: palette().muted,
          fontVariant: ["tabular-nums"],
        }}
      >
        {filtered()
          ? `${rows().length} ${ui.t("条匹配记录", "matching records")}`
          : props.history
            ? `${rows().length} ${ui.t("条请求记录", "requests")}`
            : `${rows().length} ${ui.t("条活跃连接", "active")}`}
      </Text>
      <If when={filtered()}>
        <Button
          testID="activity-clear-filters"
          accessibilityLabel={ui.t("清除筛选条件", "Clear filters")}
          focusStyle={focusRing()}
          hoverStyle={{ opacity: 0.8 }}
          activeStyle={{ opacity: 0.65 }}
          onPress={clearFilters}
          style={{
            minHeight: 24,
            paddingHorizontal: 6,
            borderRadius: radius.sm,
            justifyContent: "center",
          }}
        >
          <Text
            style={{
              fontSize: typography.note,
              lineHeight: density.line(18),
              fontWeight: "600",
              color: palette().accent,
            }}
          >
            {ui.t("清除筛选条件", "Clear filters")}
          </Text>
        </Button>
      </If>
    </View>
  );

  const listCard = () => (
    <List
      bounces={false}
      keyboardDismissMode="on-drag"
      keyboardShouldPersistTaps="handled"
      each={rows()}
      keyExtractor={(row) => row.id}
      estimatedItemSize={table() ? 48 : 58}
      style={{
        flex: 1,
        borderWidth: 1,
        borderColor: palette().border,
        borderRadius: table() ? 12 : radius.xl,
        borderCurve: "continuous",
        backgroundColor: palette().panel,
        overflow: "hidden",
        ...surfaceFinish("card"),
      }}
      contentStyle={{ paddingBottom: spacing.xxs }}
      header={
        table() ? (
          <ActivityTableHeader
            history={!!props.history}
            usable={tableUsable()}
          />
        ) : undefined
      }
      empty={
        <Empty
          icon={filtered() ? "search" : "activity"}
          title={
            filtered()
              ? ui.t("没有匹配记录", "No matches")
              : ui.t("暂无记录", "No records")
          }
          text={
            filtered()
              ? props.history
                ? ui.t(
                    "当前协议与状态筛选下没有匹配的请求历史。",
                    "No request history matches the current filters.",
                  )
                : ui.t(
                    "当前协议筛选下没有匹配的活动连接。",
                    "No active connections match the current filters.",
                  )
              : ui.t(
                  "连接活动出现后将在这里显示",
                  "Connection activity will appear here",
                )
          }
        >
          <If when={filtered()}>
            <Action
              small
              secondary
              label={ui.t("清除筛选条件", "Clear filters")}
              onPress={clearFilters}
            />
          </If>
        </Empty>
      }
    >
      {(row) => {
        const host = () =>
          row().domain || row().host || row().destinationIp;
        const mode = () => (props.history ? "request" : "connection");
        const isSelected = () =>
          (platform.desktop || splitPad()) &&
          activeSelection()?.id === row().id;
        return (
          <ObjectMenu
            id={`activity-menu-${row().id}`}
            title={host()}
            subtitle={`${row().network.toUpperCase()} · ${formatRouteChain(row())}`}
            commands={[
              {
                id: `activity-details-${row().id}`,
                label: ui.t("连接详情…", "Connection details…"),
                icon: "info",
                run: () => showDetails(row()),
              },
              {
                label: ui.t("复制目标地址", "Copy destination"),
                icon: "copy",
                run: () =>
                  copyText(
                    ui,
                    host(),
                    ui.t("已复制地址", "Address copied"),
                  ),
              },
              {
                label: ui.t("以此域名添加规则…", "Add rule…"),
                icon: "plus",
                run: () => rule(row()),
              },
              {
                id: `activity-close-${mode()}-${row().id}`,
                label: ui.t("关闭此连接", "Close connection"),
                icon: "unplug" as const,
                danger: true,
                disabled:
                  (!!props.history && !row().active) ||
                  ui.client.pending() > 0 ||
                  !ui.client.domains.fresh([
                    props.history ? "requests" : "connections",
                  ]),
                run: () => {
                  void ui.client.run(
                    "connections.close",
                    { id: row().id },
                    ui.t("连接已断开", "Connection closed"),
                  );
                },
              },
            ]}
            row={(menu) => (
              <If
                when={table()}
                else={
                  <View
                    testID={`activity-row-${mode()}-${row().id}`}
                    role="button"
                    accessibilityLabel={host()}
                    focusStyle={focusRing()}
                    hoverStyle={{
                      backgroundColor: isSelected()
                        ? palette().accentFill
                        : palette().hover,
                    }}
                    activeStyle={{
                      backgroundColor: isSelected()
                        ? palette().accentFill
                        : palette().soft,
                    }}
                    onPress={() => showDetails(row())}
                    style={{
                      minHeight: Math.max(
                        58,
                        density.line(38) * Math.max(1, fontScale()) + 20,
                      ),
                      paddingHorizontal: 14,
                      paddingVertical: 10,
                      flexDirection: "row",
                      alignItems: "center",
                      gap: 10,
                      borderBottomWidth: 1,
                      borderBottomColor: palette().line,
                      backgroundColor: isSelected()
                        ? palette().accentFill
                        : "transparent",
                      ...feedbackStyle(),
                    }}
                  >
                    <View
                      style={{
                        flex: 1,
                        minWidth: 0,
                        justifyContent: "center",
                        gap: 3,
                      }}
                    >
                      <View
                        testID={`activity-title-${mode()}-${row().id}`}
                        style={{
                          flexDirection: "row",
                          alignItems: "center",
                          gap: 6,
                          minWidth: 0,
                        }}
                      >
                        <Text
                          style={{
                            flexShrink: 1,
                            minWidth: 0,
                            numberOfLines: 1,
                            fontSize: typography.sm,
                            lineHeight: density.line(20),
                            fontWeight: "600",
                            color: palette().text,
                          }}
                        >
                          {host()}
                        </Text>
                        <Badge
                          tone={
                            props.history
                              ? historyStatusTone(row())
                              : "neutral"
                          }
                          label={
                            props.history
                              ? historyStatusLabel(row(), ui.t)
                              : row().network.toUpperCase()
                          }
                        />
                      </View>
                      <View
                        testID={`activity-meta-${mode()}-${row().id}`}
                        style={{
                          minHeight: 18 * Math.max(1, fontScale()),
                          flexDirection: "row",
                          alignItems: "center",
                          minWidth: 0,
                        }}
                      >
                        <Text
                          style={{
                            numberOfLines: 1,
                            flex: 1,
                            minWidth: 0,
                            fontSize: typography.note,
                            lineHeight: density.line(18),
                            color: palette().muted,
                          }}
                        >
                          {(() => {
                            const route = formatRouteChain(row());
                            if (props.history) {
                              const when = formatActivityTime(row().startedAt);
                              return when === "—"
                                ? `${row().network.toUpperCase()} · ${route}`
                                : `${row().network.toUpperCase()} · ${route} · ${when}`;
                            }
                            const elapsed = formatActivityElapsed(
                              row().startedAt,
                              ui.t,
                            );
                            return elapsed === "—"
                              ? route
                              : `${route} · ${elapsed}`;
                          })()}
                        </Text>
                      </View>
                    </View>
                    <View
                      style={{
                        flexDirection: "row",
                        alignItems: "center",
                        gap: 4,
                        flexShrink: 0,
                      }}
                    >
                      <Text
                        testID={`activity-bytes-${mode()}-${row().id}`}
                        style={{
                          numberOfLines: 1,
                          fontSize: typography.note,
                          lineHeight: density.line(18),
                          color: palette().muted,
                          fontVariant: ["tabular-nums"],
                        }}
                      >
                        {bytes(row().downloadBytes + row().uploadBytes)}
                      </Text>
                      <If when={!splitPad()}>
                        <Icon
                          name="chevron-right"
                          size={16}
                          color={palette().faint}
                        />
                      </If>
                    </View>
                  </View>
                }
              >
                <ActivityTableRow
                  connection={row()}
                  history={!!props.history}
                  selected={isSelected()}
                  usable={tableUsable()}
                  menu={menu}
                  onDetails={() => showDetails(row())}
                  onRule={() => rule(row())}
                />
              </If>
            )}
          />
        );
      }}
    </List>
  );

  return (
    <View
      style={{
        flex: 1,
        paddingHorizontal: layout().inset,
        paddingTop: layout().inset,
        paddingBottom: spacing.md,
        gap: spacing.md,
      }}
    >
      <If when={props.history && ui.client.domains.data("requests")?.sampled}>
        <Hint
          text={ui.t(
            "仅记录刷新时可见的连接，短连接可能不会出现在历史中。",
            "History contains connections visible when refreshed; short connections may be absent.",
          )}
        />
      </If>

      <If
        when={splitPad()}
        else={
          <>
            <If
              when={platform.desktop}
              else={
                <View style={{ gap: 10 }}>
                  {modeTabs(false)}
                  <SearchField
                    label={ui.t("搜索连接", "Search connections")}
                    placeholder={ui.t(
                      "搜索域名、IP、规则或节点",
                      "Search host, IP, rule or proxy",
                    )}
                    value={query()}
                    onChange={(text) => query.set(text)}
                  />
                  {filterBar(false)}
                  {countBar()}
                </View>
              }
            >
              <View style={{ gap: 8 }}>
                <View
                  style={{
                    flexDirection: "row",
                    flexWrap: "wrap",
                    alignItems: "center",
                    gap: 10,
                  }}
                >
                  {modeTabs(true)}
                  <View style={{ flex: 1, minWidth: 200 }}>
                    <SearchField
                      label={ui.t("搜索连接", "Search connections")}
                      placeholder={ui.t(
                        "搜索域名、IP、规则或节点",
                        "Search destination, IP, rule or proxy",
                      )}
                      value={query()}
                      onChange={(text) => query.set(text)}
                    />
                  </View>
                  {filterBar(true)}
                </View>
                {countBar()}
              </View>
            </If>
            <View
              style={{
                flex: 1,
                minHeight: 0,
                flexDirection: "row",
                gap: layout().gap,
              }}
            >
              <View style={{ flex: 1, minWidth: 0 }}>{listCard()}</View>
              <If when={splitDesktop()}>
                <View
                  testID="activity-inspector"
                  style={{
                    width: 330,
                    flexShrink: 0,
                    borderWidth: 1,
                    borderColor: palette().border,
                    borderRadius: radius.xl,
                    borderCurve: "continuous",
                    backgroundColor: palette().panel,
                    overflow: "hidden",
                    ...surfaceFinish("card"),
                  }}
                >
                  <Scroll
                    bounces={false}
                    style={{ flex: 1 }}
                    contentStyle={{ padding: spacing.lg, gap: spacing.md }}
                  >
                    <ConnectionInspector
                      connection={activeSelection()!}
                      live={!!liveSelected()}
                      history={!!props.history}
                      title={
                        props.history
                          ? ui.t("请求检查器", "Request inspector")
                          : ui.t("连接检查器", "Connection inspector")
                      }
                      onClose={clearSelection}
                      onRule={() => rule(activeSelection()!)}
                    />
                  </Scroll>
                </View>
              </If>
            </View>
          </>
        }
      >
        <View
          style={{
            flex: 1,
            minHeight: 0,
            flexDirection: "row",
            gap: layout().gap,
          }}
        >
          <View
            style={{
              width: Math.min(380, Math.max(320, layout().collection)),
              flexShrink: 0,
              gap: 10,
            }}
          >
            {modeTabs(false)}
            <SearchField
              label={ui.t("搜索连接", "Search connections")}
              placeholder={ui.t(
                "搜索域名、IP、规则或节点",
                "Search host, IP, rule or proxy",
              )}
              value={query()}
              onChange={(text) => query.set(text)}
            />
            {filterBar(false)}
            {countBar()}
            {listCard()}
          </View>
          <View style={{ flex: 1, minWidth: 0 }}>
            <If
              when={activeSelection()}
              else={
                <ListSurface style={{ flex: 1, justifyContent: "center" }}>
                  <Empty
                    icon="activity"
                    title={ui.t("未选择连接", "No connection selected")}
                    text={ui.t(
                      "在左侧选择一条连接或请求以查看详情。",
                      "Select a connection or request on the left to inspect.",
                    )}
                  />
                </ListSurface>
              }
            >
              <Scroll
                bounces={false}
                style={{ flex: 1 }}
                contentStyle={{ gap: spacing.md, paddingBottom: spacing.lg }}
              >
                <ConnectionInspector
                  connection={activeSelection()!}
                  live={
                    selectedId()
                      ? !!liveSelected()
                      : sourceRows().some(
                          (row) => row.id === activeSelection()!.id,
                        )
                  }
                  history={!!props.history}
                  title={
                    props.history
                      ? ui.t("请求详情", "Request details")
                      : ui.t("连接详情", "Connection details")
                  }
                  onRule={() => rule(activeSelection()!)}
                />
              </Scroll>
            </If>
          </View>
        </View>
      </If>

      <If when={sheetSelection()}>
        {(() => {
          const current = sheetSelection()!;
          const host =
            current.domain || current.host || current.destinationIp;
          const live = !!liveSelected();
          const mode = props.history ? "request" : "connection";
          const canClose = live && (!props.history || current.active);
          return (
            <BottomSheet
              title={host}
              subtitle={`${current.network.toUpperCase()} · ${
                props.history
                  ? historyStatusLabel(current, ui.t)
                  : live && current.active
                    ? ui.t("活跃", "Active")
                    : ui.t("已结束", "Ended")
              } · ${ui.t("建立于", "Started")} ${formatActivityTime(current.startedAt)}`}
              onClose={clearSelection}
            >
              <If when={!live}>
                <Hint
                  text={ui.t(
                    "此记录已离开当前列表，以下为打开时的详情。",
                    "This record left the list. Details below were captured when opened.",
                  )}
                />
              </If>
              <Well style={{ gap: 8 }}>
                <ConnectionDetails connection={current} />
              </Well>
              <ListSurface style={{ padding: 6, gap: 2 }}>
                <SheetActionItem
                  id={`copy-for-${current.id}`}
                  icon="copy"
                  label={ui.t("复制目标地址", "Copy destination")}
                  onPress={() => {
                    copyText(ui, host, ui.t("已复制地址", "Address copied"));
                    clearSelection();
                  }}
                />
                <SheetActionItem
                  id={`rule-for-${current.id}`}
                  icon="plus"
                  label={ui.t("以此域名添加规则…", "Add rule for this host…")}
                  onPress={() => {
                    clearSelection();
                    rule(current);
                  }}
                />
                <If when={canClose}>
                  <Separator />
                  <SheetActionItem
                    id={`activity-close-${mode}-${current.id}`}
                    icon="x"
                    danger
                    label={ui.t("关闭此连接", "Close connection")}
                    disabled={
                      ui.client.isPending("connections.close") ||
                      !ui.client.domains.fresh([
                        props.history ? "requests" : "connections",
                      ])
                    }
                    onPress={() => {
                      void ui.client.run(
                        "connections.close",
                        { id: current.id },
                        ui.t("连接已断开", "Connection closed"),
                      );
                      clearSelection();
                    }}
                  />
                </If>
              </ListSurface>
            </BottomSheet>
          );
        })()}
      </If>

      <If when={modalSelection()}>
        <Dialog
          width={440}
          title={
            props.history
              ? ui.t("请求详情", "Request details")
              : ui.t("连接详情", "Connection details")
          }
          onClose={clearSelection}
          footer={
            <Actions>
              <Action
                secondary
                label={ui.t("关闭", "Close")}
                onPress={clearSelection}
              />
              <Action
                secondary
                icon="plus"
                label={ui.t("添加规则…", "Add rule…")}
                onPress={() => {
                  const current = modalSelection()!;
                  clearSelection();
                  rule(current);
                }}
              />
              <Action
                icon="copy"
                label={ui.t("复制地址", "Copy address")}
                onPress={() =>
                  copyText(
                    ui,
                    modalSelection()!.domain ||
                      modalSelection()!.host ||
                      modalSelection()!.destinationIp,
                    ui.t("已复制地址", "Address copied"),
                  )
                }
              />
            </Actions>
          }
        >
          <Well style={{ gap: 8 }}>
            <ConnectionDetails connection={modalSelection()!} />
          </Well>
        </Dialog>
      </If>

      <If when={manual()}>
        <ManualRuleDialog
          context={manual()}
          onClose={() => manual.set(undefined)}
        />
      </If>
    </View>
  );
}

export function ActivityDetail(props: { identity: string }) {
  const ui = useApp(),
    manual = signal<RuleContext | undefined>();
  const history = props.identity.startsWith("requests:");
  const id = props.identity.slice(props.identity.indexOf(":") + 1);
  const live = () =>
    (history
      ? ui.client.state()!.requestHistory
      : ui.client.state()!.connections
    ).find((row) => row.id === id);
  const captured = ui.viewState?.get(`activity.detail.${props.identity}`) as
    | Connection
    | undefined;
  const connection = () => live() || captured;
  return (
    <Scroll bounces={false} style={{ flex: 1 }}>
      <Body>
        <If
          when={connection()}
          else={
            <ListSurface>
              <Empty
                text={ui.t(
                  "此记录已不可用",
                  "This record is no longer available",
                )}
              />
            </ListSurface>
          }
        >
          <ConnectionInspector
            connection={connection()!}
            live={!!live()}
            history={history}
            title={
              connection()!.domain ||
              connection()!.host ||
              connection()!.destinationIp
            }
            onRule={() =>
              manual.set({
                domain: connection()!.domain || undefined,
                destinationIp: connection()!.destinationIp,
                connectionId:
                  !history || connection()!.active
                    ? connection()!.id
                    : undefined,
              })
            }
          />
        </If>
        <If when={manual()}>
          <ManualRuleDialog
            context={manual()}
            onClose={() => manual.set(undefined)}
          />
        </If>
      </Body>
    </Scroll>
  );
}

function ConnectionInspector(props: {
  connection: Connection;
  live: boolean;
  history: boolean;
  title: string;
  onClose?: () => void;
  onRule: () => void;
}) {
  const ui = useApp();
  const host = () =>
    props.connection.domain ||
    props.connection.host ||
    props.connection.destinationIp;
  const canClose = () =>
    props.live && (!props.history || props.connection.active);
  const badgeTone = () =>
    props.history
      ? historyStatusTone(props.connection)
      : props.live && props.connection.active
        ? ("success" as const)
        : ("neutral" as const);
  const badgeStatus = () =>
    props.history
      ? historyStatusLabel(props.connection, ui.t)
      : props.live && props.connection.active
        ? ui.t("活跃", "Active")
        : ui.t("已结束", "Ended");
  return (
    <View style={{ gap: spacing.md }}>
      <SectionHeader
        title={props.title}
        right={
          <View style={{ flexDirection: "row", alignItems: "center", gap: 6 }}>
            <Badge
              tone={badgeTone()}
              label={`${props.connection.network.toUpperCase()} · ${badgeStatus()}`}
            />
            <If when={props.onClose}>
              <IconButton
                id="inspector-close"
                icon="x"
                small
                iconColor={palette().muted}
                label={ui.t("关闭检查器", "Close inspector")}
                onPress={() => props.onClose?.()}
              />
            </If>
          </View>
        }
      />
      <If when={!props.live}>
        <Hint
          text={ui.t(
            "此记录已离开当前列表，以下为打开时的详情。",
            "This record left the list. Details below were captured when opened.",
          )}
        />
      </If>
      <ListSurface style={{ padding: spacing.md, gap: spacing.sm }}>
        <ConnectionDetails connection={props.connection} />
      </ListSurface>
      <Actions align="start">
        <Action
          small
          secondary
          icon="copy"
          label={ui.t("复制详情", "Copy details")}
          onPress={() =>
            copyText(
              ui,
              JSON.stringify(props.connection, null, 2),
              ui.t("已复制详情", "Details copied"),
            )
          }
        />
        <Action
          small
          secondary
          icon="plus"
          label={
            props.history
              ? ui.t("以此域名添加规则…", "Add rule…")
              : ui.t("添加规则…", "Add rule…")
          }
          onPress={props.onRule}
        />
        <If when={canClose()}>
          <Action
            small
            danger
            icon="x"
            label={ui.t("关闭此连接", "Close this connection")}
            disabled={
              ui.client.isPending("connections.close") ||
              !ui.client.domains.fresh([
                props.history ? "requests" : "connections",
              ])
            }
            onPress={() =>
              void ui.client.run(
                "connections.close",
                { id: props.connection.id },
                ui.t("连接已断开", "Connection closed"),
              )
            }
          />
        </If>
      </Actions>
    </View>
  );
}

function ConnectionDetails(props: { connection: Connection }) {
  const ui = useApp();
  return (
    <>
      <Detail
        stacked
        label={ui.t("目标地址", "Destination")}
        value={
          props.connection.domain ||
          props.connection.host ||
          props.connection.destinationIp
        }
      />
      <Separator />
      <Detail
        label={ui.t("解析 IP", "Resolved IP")}
        value={props.connection.destinationIp || "—"}
      />
      <Separator />
      <Detail
        label={ui.t("网络协议", "Network")}
        value={props.connection.network.toUpperCase()}
      />
      <Separator />
      <Detail
        stacked
        label={ui.t("代理链路", "Proxy chain")}
        value={props.connection.chains?.join(" → ") || props.connection.proxy}
      />
      <Separator />
      <Detail
        stacked
        label={ui.t("命中规则", "Matched rule")}
        value={props.connection.rule}
      />
      <Separator />
      <Detail
        label={ui.t("流量统计", "Traffic")}
        value={`↓ ${bytes(props.connection.downloadBytes)} · ↑ ${bytes(props.connection.uploadBytes)}`}
      />
      <Separator />
      <Detail
        label={ui.t("建立时间", "Started")}
        value={timestamp(props.connection.startedAt)}
      />
      <Separator />
      <Detail
        label={ui.t("更新时间", "Updated")}
        value={timestamp(props.connection.updatedAt)}
      />
    </>
  );
}

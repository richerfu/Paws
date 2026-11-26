import { density, typography } from "../theme";
import {
  cleanup,
  computed,
  effect,
  fontScale,
  signal,
  Text,
  View,
  List,
  platform,
} from "nsh";
import {
  Action,
  Chevron,
  control,
  Empty,
  Hint,
  Icon,
  IconTile,
  ListSurface,
  palette,
  RadioMark,
  radius,
  SearchField,
  SectionHeader,
  spacing,
  Spinner,
  useApp,
  useDomainInterests,
  useLifetime,
  useLayout,
  useViewState,
  Actions,
  copyText,
} from "../components";
import { includes } from "../locale";
import { focusRing, statusColors, surfaceFinish } from "../theme";
import { PageError } from "../page-error";
import { animateDisclosure, feedbackStyle } from "../motion";
import type { IconName } from "../icons";
import type {
  CoreRequest,
  ProxyGroup,
  Proxy,
  ProxyGroupHeader,
} from "../model";
import {
  groupMembers,
  isManualSelector,
  selectableGroup,
  selectedMember,
  groupSelectionMode,
} from "../proxy-groups";

type Group = ProxyGroup | ProxyGroupHeader;
type Row = { key: string; group: Group; proxy?: Proxy; loading?: boolean };

function pendingGroupSelection(
  requests: CoreRequest[],
  group: string,
): CoreRequest | undefined {
  return requests.find(
    (request) =>
      (request.command === "proxies.select" ||
        request.command === "proxies.unfix") &&
      request.payload.group === group,
  );
}

function optimisticSelected(
  serverSelected: string | null | undefined,
  pending: CoreRequest | undefined,
): string | undefined {
  if (
    pending?.command === "proxies.select" &&
    typeof pending.payload.name === "string"
  )
    return pending.payload.name;
  return serverSelected || undefined;
}

function optimisticFixed(
  group: Group,
  pending: CoreRequest | undefined,
): string | null | undefined {
  if (!pending) return group.fixed;
  if (pending.command === "proxies.unfix") return "";
  if (
    pending.command === "proxies.select" &&
    !isManualSelector(group) &&
    typeof pending.payload.name === "string"
  )
    return pending.payload.name;
  return group.fixed;
}

function effectiveGroupState(
  group: Group,
  pending: CoreRequest | undefined,
): Pick<Group, "fixed" | "groupType"> {
  const fixed = optimisticFixed(group, pending);
  return fixed === group.fixed
    ? group
    : { groupType: group.groupType, fixed: fixed ?? undefined };
}

function isSelectionTarget(
  pending: CoreRequest | undefined,
  group: Group,
  proxyName: string,
): boolean {
  if (!pending) return false;
  return pending.command === "proxies.unfix"
    ? group.fixed === proxyName
    : pending.payload.name === proxyName;
}

function compareProxyGroups(a: Group, b: Group, mode: string): number {
  const aGlobal = a.name.toUpperCase() === "GLOBAL";
  const bGlobal = b.name.toUpperCase() === "GLOBAL";
  if (aGlobal !== bGlobal) {
    if (mode === "global") return aGlobal ? -1 : 1;
    return aGlobal ? 1 : -1;
  }
  return 0;
}

function groupIcon(group: Group): IconName {
  const upper = group.name.toUpperCase();
  if (upper === "GLOBAL") return "network";
  if (
    upper.includes("REJECT") ||
    upper.includes("ADBLOCK") ||
    group.name.includes("广告") ||
    group.name.includes("拦截")
  )
    return "shield-check";
  if (groupSelectionMode(group) === "automatic") return "zap";
  return "layers";
}

function SelectionMark(props: {
  selected: boolean;
  selectable: boolean;
  nestedGroup: boolean;
  busy: boolean;
}) {
  const showSpinner = signal(false);
  effect(() => {
    if (!props.busy) {
      showSpinner.set(false);
      return;
    }
    const timer = setTimeout(() => showSpinner.set(true), 180);
    cleanup(() => clearTimeout(timer));
  });
  return (
    <View
      style={{
        width: control.box,
        height: control.box,
        alignItems: "center",
        justifyContent: "center",
        flexShrink: 0,
      }}
    >
      <If
        when={showSpinner()}
        else={
          <If
            when={props.selectable && !props.nestedGroup}
            else={
              <Icon
                name={props.nestedGroup ? "layers" : "network"}
                size={platform.desktop ? 15 : 17}
                color={props.selected ? palette().accent : palette().muted}
              />
            }
          >
            <RadioMark selected={props.selected} />
          </If>
        }
      >
        <Spinner size={platform.desktop ? 15 : 17} color={palette().accent} />
      </If>
    </View>
  );
}

function groupModeLabel(
  group: Pick<Group, "fixed" | "groupType">,
  t: (zh: string, en: string) => string,
): string {
  const mode = groupSelectionMode(group);
  return mode === "automatic"
    ? t("自动选择", "Automatic")
    : mode === "pinned"
      ? t("已固定", "Pinned")
      : mode === "manual"
        ? t("手动选择", "Manual")
        : t("自动策略", "Policy");
}

function isTimeout(proxy: Proxy) {
  return proxy.delayMs === 0;
}

function latencyTone(proxy: Proxy) {
  return isTimeout(proxy)
    ? ("danger" as const)
    : proxy.delayMs == null
      ? ("neutral" as const)
      : proxy.delayMs < 150
        ? ("success" as const)
        : proxy.delayMs < 400
          ? ("warning" as const)
          : ("danger" as const);
}

/** Inline non-scrolling summary groups for Overview (no nested ScrollView). */
export function SummaryPolicyGroups() {
  const ui = useApp(),
    alive = useLifetime(),
    expanded = signal("");
  useDomainInterests(() =>
    expanded() ? [{ domain: "proxies", groupKey: expanded() }] : [],
  );
  const allGroups = computed(() => {
    const mode = ui.client.state()!.mode;
    return ui.client.domains
      .data("summary")!
      .groups.slice()
      .sort((a, b) => compareProxyGroups(a, b, mode));
  });
  const groupNameSet = computed(
    () => new Set(allGroups().map((group) => group.name)),
  );
  const interest = (group: Group) => ({
    domain: "proxies" as const,
    groupKey: group.name,
  });
  const openRoster = computed((): ProxyGroup | undefined => {
    const key = expanded();
    if (!key) return undefined;
    return ui.client.domains
      .data("proxies", key)
      ?.proxyGroups.find((row) => row.name === key);
  });
  const openMembers = computed(() => {
    const value = openRoster();
    return value ? groupMembers(allGroups(), value) : [];
  });
  const openMemberSet = computed(
    () => new Set(openMembers().map((proxy) => proxy.name)),
  );
  const roster = (group: Group): ProxyGroup | undefined =>
    group.name === expanded()
      ? openRoster()
      : ui.client.domains
          .data("proxies", group.name)
          ?.proxyGroups.find((row) => row.name === group.name);
  const members = (group: Group) => {
    if (group.name === expanded()) return openMembers();
    const value = roster(group);
    return value ? groupMembers(allGroups(), value) : [];
  };
  const selection = (group: string) =>
    pendingGroupSelection(ui.client.pendingRequests(), group);
  const selectingAny = () =>
    ui.client.isPending("proxies.select") ||
    ui.client.isPending("proxies.unfix");
  const openSelection = computed(() =>
    expanded() ? selection(expanded()) : undefined,
  );
  const openFresh = computed(() =>
    expanded()
      ? ui.client.domains.fresh([
          "config",
          { domain: "proxies", groupKey: expanded() },
        ])
      : false,
  );
  const selected = (group: Group) => {
    const value = roster(group),
      name = value && selectedMember(value);
    const hasMember =
      group.name === expanded()
        ? !!name && openMemberSet().has(name)
        : !!name && members(group).some((proxy) => proxy.name === name);
    const serverSelected = hasMember
      ? name
      : ("effectiveSelectedNode" in group && group.effectiveSelectedNode) ||
          group.selected ||
          undefined;
    return optimisticSelected(
      serverSelected,
      group.name === expanded() ? openSelection() : selection(group.name),
    );
  };
  const select = async (group: Group, proxy: Proxy) => {
    if (
      selection(group.name) ||
      !selectableGroup(group) ||
      !ui.client.domains.fresh(["config", interest(group)])
    )
      return;
    const pinned = group.fixed === proxy.name;
    await ui.client.run(
      pinned ? "proxies.unfix" : "proxies.select",
      pinned ? { group: group.name } : { group: group.name, name: proxy.name },
    );
    if (!alive()) return;
  };
  return (
    <ListSurface id="summary-groups-list">
      {/* Slot-qualify GLOBAL so mode switching creates a fresh top/bottom row instead
          of reparenting a scroll-culled HarmonyOS node (whose native opacity stayed 0). */}
      <Each
        of={allGroups()}
        key={(group) =>
          group.name.toUpperCase() === "GLOBAL"
            ? `${ui.client.state()!.mode === "global" ? "top" : "bottom"}:${group.name}`
            : group.name
        }
      >
        {(group, index) => {
          const isOpen = () => expanded() === group().name;
          return (
            <View>
              <View
                testID={`summary-group-${group().name}`}
                role="button"
                accessibilityLabel={group().name}
                accessibilityState={{ expanded: isOpen() }}
                onPress={() => {
                  animateDisclosure();
                  expanded.set(isOpen() ? "" : group().name);
                }}
                focusVisibleStyle={focusRing(0)}
                hoverStyle={{ backgroundColor: palette().hover }}
                activeStyle={{ backgroundColor: palette().soft }}
                style={{
                  minHeight: Math.max(
                    52,
                    density.line(20) * Math.max(1, fontScale()) + spacing.lg,
                  ),
                  paddingHorizontal: 14,
                  paddingVertical: density.space(10),
                  gap: spacing.md,
                  borderBottomWidth:
                    isOpen() || index() < allGroups().length - 1 ? 1 : 0,
                  borderBottomColor: palette().line,
                  backgroundColor: isOpen() ? palette().inset : "transparent",
                  flexDirection: "row",
                  alignItems: "center",
                  ...feedbackStyle(),
                }}
              >
                <IconTile icon={groupIcon(group())} selected={isOpen()} />
                <View style={{ flex: 1, minWidth: 0, gap: density.space(2) }}>
                  <Text
                    style={{
                      numberOfLines: 1,
                      fontSize: typography.sm,
                      lineHeight: density.line(20),
                      fontWeight: "600",
                      color: palette().text,
                    }}
                  >
                    {group().name.toUpperCase() === "GLOBAL"
                      ? ui.t("全局节点", "Global nodes")
                      : group().name}
                  </Text>
                  <Text
                    style={{
                      numberOfLines: 1,
                      fontSize: density.font(13),
                      lineHeight: density.line(18),
                      color: palette().muted,
                    }}
                  >
                    {groupModeLabel(
                      effectiveGroupState(group(), selection(group().name)),
                      ui.t,
                    )}{" "}
                    ·{" "}
                    {selected(group()) ||
                      ui.t("未选择节点", "No node selected")}
                  </Text>
                </View>
                <Text
                  style={{
                    fontSize: density.font(13),
                    lineHeight: density.line(18),
                    color: palette().muted,
                    fontVariant: ["tabular-nums"],
                  }}
                >
                  {group().memberCount}
                </Text>
                <Chevron expanded={isOpen()} />
              </View>
              <If when={isOpen()}>
                <If
                  when={ui.client.domains.has([interest(group())])}
                  else={
                    <View
                      style={{
                        minHeight: 48,
                        padding: 14,
                        borderBottomWidth:
                          index() < allGroups().length - 1 ? 1 : 0,
                        borderBottomColor: palette().line,
                        gap: spacing.sm,
                      }}
                    >
                      <If
                        when={ui.client.domains.error([interest(group())])}
                        else={
                          <View
                            style={{
                              flexDirection: "row",
                              alignItems: "center",
                              gap: spacing.sm,
                            }}
                          >
                            <Spinner size={16} />
                            <Hint
                              text={ui.t(
                                "正在读取分组节点…",
                                "Loading group members…",
                              )}
                            />
                          </View>
                        }
                      >
                        <PageError
                          id={`group-error-${group().name}`}
                          text={ui.client.domains.error([interest(group())])}
                        />
                        <Action
                          small
                          secondary
                          label={ui.t("重试", "Retry")}
                          onPress={() =>
                            ui.client.domains.retry([interest(group())])
                          }
                        />
                      </If>
                    </View>
                  }
                >
                  <Each of={members(group())} key={(proxy) => proxy.name}>
                    {(proxy, memberIndex) => {
                      const groupSelected = computed(() => selected(group()));
                      const active = () => groupSelected() === proxy().name;
                      const nested = () => groupNameSet().has(proxy().name);
                      const pendingReq = () =>
                        group().name === expanded()
                          ? openSelection()
                          : selection(group().name);
                      const pinned = () =>
                        optimisticFixed(group(), pendingReq()) === proxy().name;
                      const rowDisabled = () =>
                        !selectableGroup(group()) ||
                        !ui.client.domains.has(["config", interest(group())]) ||
                        !!ui.client.domains.error([
                          "config",
                          interest(group()),
                        ]) ||
                        (!(group().name === expanded()
                          ? openFresh()
                          : ui.client.domains.fresh([
                              "config",
                              interest(group()),
                            ])) &&
                          !selectingAny());
                      return (
                        <View
                          testID={`summary-select-proxy-${proxy().name}`}
                          role="button"
                          accessibilityLabel={
                            pinned()
                              ? ui.t("恢复自动选择", "Use automatic selection")
                              : proxy().name
                          }
                          accessibilityState={{
                            disabled: rowDisabled(),
                            selected: active(),
                          }}
                          disabled={rowDisabled()}
                          onPress={() => void select(group(), proxy())}
                          focusVisibleStyle={focusRing(0)}
                          hoverStyle={
                            active() || rowDisabled()
                              ? undefined
                              : { backgroundColor: palette().hover }
                          }
                          activeStyle={
                            rowDisabled()
                              ? undefined
                              : {
                                  backgroundColor: active()
                                    ? palette().accentFill
                                    : palette().soft,
                                }
                          }
                          style={{
                            minHeight: Math.max(
                              50,
                              density.line(20) * Math.max(1, fontScale()) +
                                spacing.md,
                            ),
                            paddingLeft: spacing.xl,
                            paddingRight: 14,
                            paddingVertical: density.space(9),
                            borderBottomWidth:
                              memberIndex() < members(group()).length - 1 ||
                              index() < allGroups().length - 1
                                ? 1
                                : 0,
                            borderBottomColor: palette().line,
                            backgroundColor: active()
                              ? palette().accentFill
                              : palette().panel,
                            flexDirection: "row",
                            alignItems: "center",
                            gap: spacing.md,
                            ...feedbackStyle(),
                          }}
                        >
                          <SelectionMark
                            selected={active()}
                            selectable={selectableGroup(group())}
                            nestedGroup={nested()}
                            busy={isSelectionTarget(
                              pendingReq(),
                              group(),
                              proxy().name,
                            )}
                          />
                          <View style={{ flex: 1, minWidth: 0 }}>
                            <Text
                              style={{
                                numberOfLines: 1,
                                fontSize: typography.sm,
                                lineHeight: density.line(20),
                                fontWeight: active() ? "600" : "500",
                                color: palette().text,
                              }}
                            >
                              {proxy().name}
                            </Text>
                            <Text
                              style={{
                                numberOfLines: 1,
                                fontSize: density.font(13),
                                lineHeight: density.line(18),
                                color: palette().muted,
                              }}
                            >
                              {nested()
                                ? ui.t("策略分组", "Policy group")
                                : proxy().proxyType.toUpperCase()}
                              {pinned()
                                ? ui.t(" · 已固定", " · Pinned")
                                : active()
                                  ? ui.t(" · 当前", " · Current")
                                  : ""}
                            </Text>
                          </View>
                          <Text
                            style={{
                              fontSize: density.font(13),
                              lineHeight: density.line(18),
                              fontWeight: "600",
                              fontVariant: ["tabular-nums"],
                              color:
                                proxy().delayMs == null && !isTimeout(proxy())
                                  ? palette().faint
                                  : statusColors(latencyTone(proxy())).text,
                            }}
                          >
                            {isTimeout(proxy())
                              ? ui.t("超时", "Timeout")
                              : proxy().delayMs == null
                                ? "—"
                                : `${proxy().delayMs} ms`}
                          </Text>
                        </View>
                      );
                    }}
                  </Each>
                </If>
              </If>
            </View>
          );
        }}
      </Each>
    </ListSurface>
  );
}

export function ProxyGroupList(props: {
  query?: string;
  section?: boolean;
  summary?: boolean;
  groupKey?: string;
}) {
  const ui = useApp(),
    expanded = signal("");
  const alive = useLifetime();
  const groups = (): Group[] =>
    props.summary
      ? ui.client.domains.data("summary")!.groups
      : ui.client.state()!.proxyGroups;
  const interest = (group: Group) =>
    props.summary
      ? { domain: "proxies" as const, groupKey: group.name }
      : ("proxies" as const);
  const roster = (group: Group): ProxyGroup | undefined =>
    props.summary
      ? ui.client.domains
          .data("proxies", group.name)
          ?.proxyGroups.find((row) => row.name === group.name)
      : (group as ProxyGroup);
  const groupNameSet = computed(
    () => new Set(groups().map((group) => group.name)),
  );
  const membersByGroup = computed(() => {
    const all = groups();
    const map = new Map<string, Proxy[]>();
    for (const group of all) {
      if (props.groupKey && group.name !== props.groupKey) continue;
      const value = roster(group);
      map.set(group.name, value ? groupMembers(all, value) : []);
    }
    return map;
  });
  const members = (group: Group) =>
    membersByGroup().get(group.name) ??
    (() => {
      const value = roster(group);
      return value ? groupMembers(groups(), value) : [];
    })();
  const selectedByGroup = computed(() => {
    const map = new Map<string, string | undefined>();
    for (const group of groups()) {
      if (props.groupKey && group.name !== props.groupKey) continue;
      const value = roster(group),
        name = value && selectedMember(value),
        list = membersByGroup().get(group.name) ?? [];
      map.set(
        group.name,
        name && list.some((proxy) => proxy.name === name) ? name : undefined,
      );
    }
    return map;
  });
  const memberCount = (group: Group) =>
    "memberCount" in group ? group.memberCount : members(group).length;
  const fresh = (group: Group) =>
    ui.client.domains.fresh(["config", interest(group)]);
  useDomainInterests(() =>
    props.summary && expanded()
      ? [{ domain: "proxies", groupKey: expanded() }]
      : [],
  );
  const rows = computed<Row[]>(() => {
    const mode = ui.client.state()!.mode;
    return groups()
      .slice()
      .filter((group) => !props.groupKey || group.name === props.groupKey)
      .sort((a, b) => compareProxyGroups(a, b, mode))
      .flatMap((group) => {
        const query = props.query || "";
        const filtered = members(group).filter((proxy) =>
          includes(
            query,
            group.name,
            proxy.name,
            proxy.proxyType,
            proxy.delayMs == null ? "" : String(proxy.delayMs),
            proxy.selected ? "selected 当前 已选" : "",
          ),
        );
        if (
          !props.groupKey &&
          query &&
          !filtered.length &&
          !includes(query, group.name, group.groupType)
        )
          return [];
        const result: Row[] = props.groupKey
          ? []
          : [{ key: `group:${group.name}`, group }];
        if (props.groupKey || expanded() === group.name || !!query) {
          if (props.summary && !ui.client.domains.has([interest(group)]))
            result.push({ key: `loading:${group.name}`, group, loading: true });
          else
            for (const proxy of filtered)
              result.push({
                key: `member:${group.name}:${proxy.name}`,
                group,
                proxy,
              });
        }
        return result;
      });
  });
  const selection = (group: string) =>
    pendingGroupSelection(ui.client.pendingRequests(), group);
  const selectingAny = () =>
    ui.client.isPending("proxies.select") ||
    ui.client.isPending("proxies.unfix");
  const selected = (group: Group) => {
    const serverSelected = selectedByGroup().has(group.name)
      ? selectedByGroup().get(group.name)
      : (() => {
          const value = roster(group),
            name = value && selectedMember(value);
          return name && members(group).some((proxy) => proxy.name === name)
            ? name
            : undefined;
        })();
    return optimisticSelected(serverSelected, selection(group.name));
  };
  const headerSelected = (group: Group) =>
    "effectiveSelectedNode" in group
      ? optimisticSelected(
          group.effectiveSelectedNode || group.selected,
          selection(group.name),
        )
      : selected(group);
  const select = async (row: Row) => {
    if (
      selection(row.group.name) ||
      !selectableGroup(row.group) ||
      !fresh(row.group)
    )
      return;
    const pinned = row.group.fixed === row.proxy?.name;
    await ui.client.run(
      pinned ? "proxies.unfix" : "proxies.select",
      pinned
        ? { group: row.group.name }
        : { group: row.group.name, name: row.proxy!.name },
    );
    if (!alive()) return;
  };
  const disabled = (group: Group) =>
    !selectableGroup(group) ||
    !ui.client.domains.has(["config", interest(group)]) ||
    !!ui.client.domains.error(["config", interest(group)]) ||
    (!fresh(group) && !selectingAny());
  const isSelected = (row: Row) => selected(row.group) === row.proxy?.name;
  const isPinned = (row: Row) =>
    optimisticFixed(row.group, selection(row.group.name)) === row.proxy?.name;
  const isGroup = (proxy: Proxy) => groupNameSet().has(proxy.name);
  const isOpen = (group: Group) =>
    props.groupKey === group.name || !!props.query || expanded() === group.name;
  const groupHeight = () =>
    Math.max(
      56,
      density.line(36) * Math.max(1, fontScale()) + spacing.lg,
    );
  const nodeHeight = () =>
    Math.max(
      48,
      density.line(36) * Math.max(1, fontScale()) + spacing.sm,
    );
  const hasHeader = computed(() => platform.desktop && !!props.groupKey);
  return (
    <List
      testID={props.summary ? "summary-groups-list" : "proxy-groups-list"}
      bounces={false}
      keyboardDismissMode="on-drag"
      keyboardShouldPersistTaps="handled"
      each={rows()}
      keyExtractor={(row) => row.key}
      estimatedItemSize={props.groupKey ? nodeHeight() : groupHeight()}
      style={{
        flex: 1,
        minHeight: 0,
        borderWidth: 1,
        borderColor: palette().border,
        borderRadius: radius.xl,
        borderCurve: "continuous",
        backgroundColor: palette().panel,
        overflow: "hidden",
        ...surfaceFinish("card"),
      }}
      contentStyle={{ paddingBottom: density.space(4) }}
      header={
        hasHeader() ? (
          <View
            style={{
              minHeight: 30,
              paddingHorizontal: spacing.md,
              borderBottomWidth: 1,
              borderBottomColor: palette().line,
              backgroundColor: palette().soft,
              flexDirection: "row",
              alignItems: "center",
              gap: spacing.md,
            }}
          >
            <Text
              style={{
                flex: 1,
                fontSize: density.font(11),
                fontWeight: "600",
                color: palette().muted,
              }}
            >
              {ui.t("节点", "Node")}
            </Text>
            <Text
              style={{
                width: 96,
                fontSize: density.font(11),
                fontWeight: "600",
                color: palette().muted,
              }}
            >
              {ui.t("协议", "Protocol")}
            </Text>
            <Text
              style={{
                width: 88,
                textAlign: "right",
                fontSize: density.font(11),
                fontWeight: "600",
                color: palette().muted,
              }}
            >
              {ui.t("延迟", "Latency")}
            </Text>
          </View>
        ) : undefined
      }
      empty={
        <Empty
          icon="layers"
          title={ui.t("暂无代理节点", "No proxy nodes")}
          text={ui.t(
            "添加并启用订阅后即可选择节点",
            "Add and activate a subscription to select nodes",
          )}
        />
      }
    >
      {(item) => (
        <If
          when={item().loading}
          else={
            <If
              when={!item().proxy}
              else={
                <View
                  testID={`${props.summary ? "summary-" : ""}select-proxy-${item().proxy!.name}`}
                  role="button"
                  accessibilityLabel={
                    isPinned(item())
                      ? ui.t("恢复自动选择", "Use automatic selection")
                      : item().proxy!.name
                  }
                  accessibilityState={{
                    disabled: disabled(item().group),
                    selected: isSelected(item()),
                  }}
                  disabled={disabled(item().group)}
                  onPress={() => void select(item())}
                  focusVisibleStyle={focusRing(0)}
                  hoverStyle={
                    isSelected(item()) || disabled(item().group)
                      ? undefined
                      : { backgroundColor: palette().hover }
                  }
                  activeStyle={
                    disabled(item().group)
                      ? undefined
                      : {
                          backgroundColor: isSelected(item())
                            ? palette().accentFill
                            : palette().soft,
                        }
                  }
                  style={{
                    height: nodeHeight(),
                    paddingLeft: props.groupKey
                      ? spacing.md
                      : density.space(28),
                    paddingRight: spacing.md,
                    borderBottomWidth: 1,
                    borderBottomColor: palette().line,
                    backgroundColor: isSelected(item())
                      ? palette().accentFill
                      : palette().panel,
                    flexDirection: "row",
                    alignItems: "center",
                    gap: spacing.md,
                    ...feedbackStyle(),
                  }}
                >
                  <SelectionMark
                    selected={isSelected(item())}
                    selectable={selectableGroup(item().group)}
                    nestedGroup={isGroup(item().proxy!)}
                    busy={isSelectionTarget(
                      selection(item().group.name),
                      item().group,
                      item().proxy!.name,
                    )}
                  />
                  <View style={{ flex: 1, minWidth: 0, gap: density.space(2) }}>
                    <Text
                      style={{
                        numberOfLines: 1,
                        fontSize: typography.sm,
                        lineHeight: density.line(20),
                        fontWeight: isSelected(item()) ? "600" : "500",
                        color: palette().text,
                      }}
                    >
                      {item().proxy!.name}
                    </Text>
                    <If when={!(platform.desktop && !!props.groupKey)}>
                      <Text
                        style={{
                          numberOfLines: 1,
                          fontSize: density.font(12),
                          lineHeight: density.line(16),
                          color: palette().muted,
                        }}
                      >
                        {isGroup(item().proxy!)
                          ? ui.t("策略分组", "Policy group")
                          : item().proxy!.proxyType.toUpperCase()}
                        {isPinned(item())
                          ? ui.t(" · 已固定", " · Pinned")
                          : isSelected(item())
                            ? ui.t(" · 当前", " · Current")
                            : ""}
                      </Text>
                    </If>
                  </View>
                  <If when={platform.desktop && !!props.groupKey}>
                    <Text
                      style={{
                        width: 96,
                        numberOfLines: 1,
                        fontSize: density.font(12),
                        lineHeight: density.line(16),
                        color: palette().muted,
                      }}
                    >
                      {isGroup(item().proxy!)
                        ? ui.t("策略分组", "Policy group")
                        : item().proxy!.proxyType.toUpperCase()}
                      {isPinned(item())
                        ? ui.t(" · 固定", " · Pin")
                        : ""}
                    </Text>
                  </If>
                  <View
                    style={{
                      width: platform.desktop && !!props.groupKey ? 88 : undefined,
                      alignItems: "flex-end",
                      gap: density.space(2),
                    }}
                  >
                    <Text
                      style={{
                        fontSize: density.font(12),
                        lineHeight: density.line(16),
                        fontWeight: "600",
                        fontVariant: ["tabular-nums"],
                        color:
                          item().proxy!.delayMs == null &&
                          !isTimeout(item().proxy!)
                            ? palette().faint
                            : statusColors(latencyTone(item().proxy!)).text,
                      }}
                    >
                      {isTimeout(item().proxy!)
                        ? ui.t("超时", "Timeout")
                        : item().proxy!.delayMs == null
                          ? ui.t("未测试", "Not tested")
                          : `${item().proxy!.delayMs} ms`}
                    </Text>
                    <If when={!isManualSelector(item().group)}>
                      <Text
                        style={{
                          height: density.line(14),
                          fontSize: density.font(11),
                          lineHeight: density.line(14),
                          color: palette().muted,
                        }}
                      >
                        {isPinned(item())
                          ? ui.t("恢复自动", "Restore auto")
                          : !selectableGroup(item().group)
                            ? ui.t("自动", "Auto")
                            : ""}
                      </Text>
                    </If>
                  </View>
                </View>
              }
            >
              <View
                testID={`${props.summary ? "summary-" : ""}group-${item().group.name}`}
                role="button"
                accessibilityLabel={item().group.name}
                accessibilityState={{
                  expanded: isOpen(item().group),
                  disabled: !!props.query || !!props.groupKey,
                }}
                disabled={!!props.query || !!props.groupKey}
                onPress={() => {
                  if (!props.groupKey)
                    expanded.set(
                      expanded() === item().group.name ? "" : item().group.name,
                    );
                }}
                focusVisibleStyle={focusRing(0)}
                hoverStyle={{ backgroundColor: palette().hover }}
                activeStyle={{ backgroundColor: palette().soft }}
                style={{
                  height: groupHeight(),
                  paddingHorizontal: spacing.md,
                  gap: spacing.md,
                  borderBottomWidth: 1,
                  borderBottomColor: palette().line,
                  backgroundColor: isOpen(item().group)
                    ? palette().inset
                    : palette().panel,
                  flexDirection: "row",
                  alignItems: "center",
                  ...feedbackStyle(),
                }}
              >
                <Icon
                  name={
                    item().group.name.toUpperCase() === "GLOBAL"
                      ? "network"
                      : "layers"
                  }
                  size={17}
                  color={
                    isOpen(item().group) ? palette().accent : palette().muted
                  }
                />
                <View style={{ flex: 1, minWidth: 0, gap: density.space(2) }}>
                  <Text
                    style={{
                      numberOfLines: 1,
                      fontSize: typography.sm,
                      lineHeight: density.line(20),
                      fontWeight: "600",
                      color: palette().text,
                    }}
                  >
                    {item().group.name.toUpperCase() === "GLOBAL"
                      ? ui.t("全局节点", "Global nodes")
                      : item().group.name}
                  </Text>
                  <Text
                    style={{
                      numberOfLines: 1,
                      fontSize: density.font(12),
                      lineHeight: density.line(16),
                      color: palette().muted,
                    }}
                  >
                    {headerSelected(item().group) ||
                      ui.t("未选择节点", "No node selected")}
                  </Text>
                </View>
                <View style={{ alignItems: "flex-end", gap: density.space(2) }}>
                  <Text
                    style={{
                      fontSize: density.font(11),
                      lineHeight: density.line(16),
                      color: palette().muted,
                    }}
                  >
                    {groupModeLabel(
                      effectiveGroupState(
                        item().group,
                        selection(item().group.name),
                      ),
                      ui.t,
                    )}
                  </Text>
                  <Text
                    style={{
                      fontSize: density.font(12),
                      lineHeight: density.line(16),
                      color: palette().muted,
                      fontVariant: ["tabular-nums"],
                    }}
                  >
                    {memberCount(item().group)} {ui.t("个节点", "nodes")}
                  </Text>
                </View>
                <If when={!props.groupKey}>
                  <Chevron expanded={isOpen(item().group)} />
                </If>
              </View>
            </If>
          }
        >
          <View
            style={{
              minHeight: 56,
              padding: density.space(12),
              gap: density.space(8),
            }}
          >
            <If
              when={ui.client.domains.error([interest(item().group)])}
              else={
                <View
                  style={{
                    flexDirection: "row",
                    alignItems: "center",
                    gap: density.space(8),
                  }}
                >
                  <Spinner size={16} />
                  <Hint
                    text={ui.t("正在读取分组节点…", "Loading group members…")}
                  />
                </View>
              }
            >
              <PageError
                id={`group-error-${item().group.name}`}
                text={ui.client.domains.error([interest(item().group)])}
              />
              <Action
                small
                secondary
                label={ui.t("重试", "Retry")}
                onPress={() =>
                  ui.client.domains.retry([interest(item().group)])
                }
              />
            </If>
          </View>
        </If>
      )}
    </List>
  );
}

export function Proxies(props: { groupKey?: string } = {}) {
  const ui = useApp(),
    layout = useLayout(),
    alive = useLifetime();
  const saved = useViewState("proxies.collection", () => ({
    query: signal(""),
    active: signal(""),
    offset: 0,
  }));
  if (props.groupKey) saved.active.set(props.groupKey);
  const query = saved.query,
    activeGroup = saved.active,
    testingGroup = signal("");
  const groups = computed(() => {
    const state = ui.client.state()!;
    return state.proxyGroups
      .slice()
      .sort((a, b) => compareProxyGroups(a, b, state.mode));
  });
  // An explicit selection that disappears is never silently replaced.
  const selectedGroup = computed(() =>
    activeGroup()
      ? groups().find((group) => group.name === activeGroup())
      : layout().masterDetail
        ? groups()[0]
        : undefined,
  );
  const filtered = computed(() => {
    const q = query();
    const all = groups();
    if (!q) return all;
    return all.filter(
      (group) =>
        includes(q, group.name, group.groupType, selectedMember(group)) ||
        groupMembers(all, group).some((proxy) =>
          includes(q, proxy.name, proxy.proxyType),
        ),
    );
  });
  const choose = (group: ProxyGroup) => {
    activeGroup.set(group.name);
    if (!props.groupKey && !layout().masterDetail)
      ui.navigate("proxy-group", group.name);
  };
  const testSelectedGroup = async () => {
    const target = selectedGroup();
    if (!target || testingGroup() || !ui.client.domains.fresh(["proxies"]))
      return;
    testingGroup.set(target.name);
    try {
      await ui.client.run("proxies.testGroup", { group: target.name });
    } finally {
      if (alive()) testingGroup.set("");
    }
  };
  return (
    <View
      style={{
        flex: 1,
        minHeight: 0,
        padding: layout().inset,
        gap: spacing.md,
      }}
    >
      <SearchField
        id="proxies-search"
        value={query()}
        onChange={(value) => query.set(value)}
        placeholder={ui.t("搜索节点或策略组", "Search nodes or groups")}
      />
      <If when={!!query()}>
        <Actions align="start">
          <Hint
            text={`${filtered().length} ${ui.t("个匹配分组", "matching groups")}`}
          />
          <Action
            small
            label={ui.t("清除筛选", "Clear filters")}
            onPress={() => query.set("")}
          />
        </Actions>
      </If>
      <View
        testID={platform.desktop ? "desktop-proxy-workspace" : "proxy-workspace"}
        style={{
          flex: 1,
          minHeight: 0,
          flexDirection: "row",
          gap: layout().masterDetail ? layout().gap : 0,
        }}
      >
        <List
          testID="desktop-proxy-groups"
          each={filtered()}
          keyExtractor={(group) => group.name}
          estimatedItemSize={control.row + spacing.md}
          bounces={false}
          contentOffset={{ y: saved.offset }}
          onScroll={(event) => {
            saved.offset = event.offset.y;
          }}
          style={{
            display:
              props.groupKey && !layout().masterDetail ? "none" : "flex",
            width: layout().masterDetail ? layout().collection : undefined,
            flex: layout().masterDetail ? undefined : 1,
            minWidth: 0,
            borderWidth: 1,
            borderColor: layout().masterDetail ? palette().line : palette().border,
            borderRadius: radius.xl,
            borderCurve: "continuous",
            backgroundColor: layout().masterDetail
              ? palette().inset
              : palette().panel,
            overflow: "hidden",
            ...(layout().masterDetail ? surfaceFinish("inset") : surfaceFinish("card")),
          }}
          contentStyle={{
            padding: layout().masterDetail ? spacing.xs : 0,
          }}
          empty={
            <Empty
              icon="layers"
              text={
                query()
                  ? ui.t(
                      "没有匹配的分组。清除筛选后查看全部。",
                      "No matching groups. Clear filters to see all.",
                    )
                  : ui.t(
                      "添加并启用订阅后即可选择节点",
                      "Add and activate a subscription to select nodes",
                    )
              }
            />
          }
        >
          {(group, index) => {
            const active = () => group().name === selectedGroup()?.name;
            return (
              <View
                testID={`${layout().masterDetail ? "desktop-" : ""}group-${group().name}`}
                role="button"
                accessibilityLabel={group().name}
                accessibilityState={{ selected: active() }}
                onPress={() => choose(group())}
                focusVisibleStyle={focusRing(0)}
                hoverStyle={
                  layout().masterDetail && active()
                    ? undefined
                    : { backgroundColor: palette().hover }
                }
                activeStyle={{ backgroundColor: palette().soft }}
                style={{
                  minHeight: control.row + spacing.sm,
                  paddingHorizontal: spacing.md,
                  paddingVertical: density.space(10),
                  flexDirection: "row",
                  alignItems: "center",
                  gap: spacing.md,
                  borderRadius: layout().masterDetail ? radius.md : 0,
                  borderCurve: "continuous",
                  borderWidth: layout().masterDetail ? 1 : 0,
                  borderColor:
                    layout().masterDetail && active()
                      ? palette().border
                      : "transparent",
                  borderBottomWidth:
                    !layout().masterDetail && index() < filtered().length - 1
                      ? 1
                      : layout().masterDetail
                        ? 1
                        : 0,
                  borderBottomColor: !layout().masterDetail
                    ? palette().line
                    : active()
                      ? palette().border
                      : "transparent",
                  backgroundColor:
                    layout().masterDetail && active()
                      ? palette().panel
                      : "transparent",
                  ...(layout().masterDetail && active()
                    ? surfaceFinish("card")
                    : {}),
                  ...feedbackStyle(),
                }}
              >
                <Icon
                  name={
                    group().name.toUpperCase() === "GLOBAL"
                      ? "network"
                      : "layers"
                  }
                  size={17}
                  color={
                    layout().masterDetail && active()
                      ? palette().accent
                      : palette().muted
                  }
                />
                <View style={{ flex: 1, minWidth: 0, gap: density.space(2) }}>
                  <Text
                    style={{
                      fontSize: typography.sm,
                      lineHeight: density.line(20),
                      fontWeight: active() ? "600" : "500",
                      color: palette().text,
                      numberOfLines: 1,
                    }}
                  >
                    {group().name.toUpperCase() === "GLOBAL"
                      ? ui.t("全局节点", "Global nodes")
                      : group().name}
                  </Text>
                  <Text
                    style={{
                      fontSize: density.font(12),
                      lineHeight: density.line(16),
                      color: palette().muted,
                      numberOfLines: 1,
                    }}
                  >
                    {groupModeLabel(
                      effectiveGroupState(
                        group(),
                        pendingGroupSelection(
                          ui.client.pendingRequests(),
                          group().name,
                        ),
                      ),
                      ui.t,
                    )}{" "}
                    ·{" "}
                    {optimisticSelected(
                      selectedMember(group()),
                      pendingGroupSelection(
                        ui.client.pendingRequests(),
                        group().name,
                      ),
                    ) || ui.t("未选择", "Not selected")}
                  </Text>
                </View>
                <Text
                  style={{
                    fontSize: density.font(12),
                    lineHeight: density.line(16),
                    color: palette().muted,
                    fontVariant: ["tabular-nums"],
                  }}
                >
                  {groupMembers(groups(), group()).length}
                </Text>
                <If when={!layout().masterDetail}>
                  <Icon
                    name="chevron-right"
                    size={16}
                    color={palette().faint}
                  />
                </If>
              </View>
            );
          }}
        </List>
        <View
          testID="proxy-members-panel"
          style={{
            display: props.groupKey || layout().masterDetail ? "flex" : "none",
            flex: 1,
            minWidth: 0,
            flexDirection: "column",
            gap: spacing.sm,
          }}
        >
          <If
            when={selectedGroup()}
            else={
              <Empty
                text={
                  activeGroup()
                    ? ui.t(
                        "此分组已移除，请返回选择其他分组。",
                        "This group was removed. Return to choose another.",
                      )
                    : ui.t("选择分组查看节点", "Select a group to view nodes")
                }
              />
            }
          >
            <SectionHeader
              title={`${selectedGroup()!.name} · ${groupModeLabel(
                effectiveGroupState(
                  selectedGroup()!,
                  pendingGroupSelection(
                    ui.client.pendingRequests(),
                    selectedGroup()!.name,
                  ),
                ),
                ui.t,
              )}`}
              meta={`${groupMembers(groups(), selectedGroup()!).length} ${ui.t("个节点", "nodes")}`}
            />
            <Actions align="start">
              <Action
                small
                icon="zap"
                loading={!!testingGroup()}
                disabled={
                  !ui.client.domains.has(["proxies"]) ||
                  !!ui.client.domains.error(["proxies"]) ||
                  (!ui.client.domains.fresh(["proxies"]) &&
                    !ui.client.isPending("proxies.select") &&
                    !ui.client.isPending("proxies.unfix"))
                }
                label={ui.t("测速当前分组", "Test group")}
                onPress={() => void testSelectedGroup()}
              />
              <Action
                small
                label={ui.t("复制分组与节点", "Copy group and node")}
                onPress={() =>
                  copyText(
                    ui,
                    `${selectedGroup()?.name}\n${selectedGroup() ? selectedMember(selectedGroup()!) || "" : ""}`,
                    ui.t("已复制", "Copied"),
                  )
                }
              />
            </Actions>
            <If when={!!testingGroup()}>
              <Hint
                text={ui.t(
                  "正在测速，保留上次结果。",
                  "Testing; previous results remain visible.",
                )}
              />
            </If>
            <If
              when={
                !!query() &&
                selectedGroup() &&
                !filtered().some(
                  (group) => group.name === selectedGroup()!.name,
                )
              }
            >
              <Hint
                text={ui.t(
                  "当前分组不在筛选结果中，选择保持不变。",
                  "The selected group is outside the filter. Selection is unchanged.",
                )}
              />
            </If>
            <ProxyGroupList query={query()} groupKey={selectedGroup()?.name} />
          </If>
        </View>
      </View>
    </View>
  );
}

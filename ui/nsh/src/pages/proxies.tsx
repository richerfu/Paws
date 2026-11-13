import { density, typography } from "../theme";
import {
  computed,
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
import { animateDisclosure, feedbackStyle, follow } from "../motion";
import type { IconName } from "../icons";
import type { ProxyGroup, Proxy, ProxyGroupHeader } from "../model";
import {
  groupMembers,
  selectableGroup,
  selectedMember,
  groupSelectionMode,
} from "../proxy-groups";

type Group = ProxyGroup | ProxyGroupHeader;
type Row = { key: string; group: Group; proxy?: Proxy; loading?: boolean };

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
  const ring = () => 22;
  const dot = () => 8;
  const dotScale = follow(() => (props.selected ? 1 : 0));
  return (
    <If
      when={props.busy}
      else={
        <If
          when={props.selectable && !props.nestedGroup}
          else={
            <Icon
              name={props.nestedGroup ? "layers" : "network"}
              size={17}
              color={props.selected ? palette().accent : palette().muted}
            />
          }
        >
          <View
            style={{
              width: ring(),
              height: ring(),
              borderRadius: ring() / 2,
              borderWidth: 1,
              borderColor: props.selected
                ? surfaceFinish("accent").borderColor
                : palette().fieldBorder,
              boxShadow: props.selected
                ? surfaceFinish("accent").boxShadow
                : undefined,
              backgroundColor: props.selected
                ? palette().brandFill
                : palette().field,
              alignItems: "center",
              justifyContent: "center",
            }}
          >
            <View
              style={{
                width: dot(),
                height: dot(),
                borderRadius: dot() / 2,
                backgroundColor: palette().primaryText,
                opacity: dotScale,
                transform: [{ scale: dotScale }],
              }}
            />
          </View>
        </If>
      }
    >
      <Spinner size={17} color={palette().accent} />
    </If>
  );
}

function groupModeLabel(
  group: Group,
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
  const allGroups = computed(() =>
    ui.client.domains
      .data("summary")!
      .groups.slice()
      .sort((a, b) =>
        a.name.toUpperCase() === "GLOBAL"
          ? -1
          : b.name.toUpperCase() === "GLOBAL"
            ? 1
            : a.name.toLowerCase().localeCompare(b.name.toLowerCase()),
      ),
  );
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
    ui.client
      .pendingRequests()
      .find(
        (request) =>
          ["proxies.select", "proxies.unfix"].includes(request.command) &&
          request.payload.group === group,
      );
  const openSelection = computed(() =>
    expanded() ? selection(expanded()) : undefined,
  );
  const openFresh = computed(() =>
    expanded()
      ? ui.client.domains.fresh([{ domain: "proxies", groupKey: expanded() }])
      : false,
  );
  const selected = (group: Group) => {
    const value = roster(group),
      name = value && selectedMember(value);
    const hasMember =
      group.name === expanded()
        ? !!name && openMemberSet().has(name)
        : !!name && members(group).some((proxy) => proxy.name === name);
    return hasMember
      ? name
      : ("effectiveSelectedNode" in group && group.effectiveSelectedNode) ||
          group.selected ||
          undefined;
  };
  const select = async (group: Group, proxy: Proxy) => {
    if (
      selection(group.name) ||
      !selectableGroup(group) ||
      !ui.client.domains.fresh([interest(group)])
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
      <Each of={allGroups()} key={(group) => group.name}>
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
                focusStyle={focusRing()}
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
                    {groupModeLabel(group(), ui.t)} ·{" "}
                    {group().effectiveSelectedNode ||
                      group().selected ||
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
                      const rowDisabled = () =>
                        !!pendingReq() ||
                        !selectableGroup(group()) ||
                        !(group().name === expanded()
                          ? openFresh()
                          : ui.client.domains.fresh([interest(group())]));
                      return (
                        <View
                          testID={`summary-select-proxy-${proxy().name}`}
                          role="button"
                          accessibilityLabel={
                            group().fixed === proxy().name
                              ? ui.t("恢复自动选择", "Use automatic selection")
                              : proxy().name
                          }
                          accessibilityState={{
                            disabled: rowDisabled(),
                            selected: active(),
                          }}
                          disabled={rowDisabled()}
                          onPress={() => void select(group(), proxy())}
                          focusStyle={focusRing()}
                          hoverStyle={
                            rowDisabled()
                              ? undefined
                              : { backgroundColor: palette().hover }
                          }
                          activeStyle={
                            rowDisabled()
                              ? undefined
                              : { backgroundColor: palette().soft }
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
                            busy={
                              !!pendingReq() &&
                              (pendingReq()!.payload.name === proxy().name ||
                                group().fixed === proxy().name)
                            }
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
                              {group().fixed === proxy().name
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
  const feedback = signal<Record<string, string>>({});
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
  const fresh = (group: Group) => ui.client.domains.fresh([interest(group)]);
  useDomainInterests(() =>
    props.summary && expanded()
      ? [{ domain: "proxies", groupKey: expanded() }]
      : [],
  );
  const rows = computed<Row[]>(() =>
    groups()
      .slice()
      .filter((group) => !props.groupKey || group.name === props.groupKey)
      .sort((a, b) =>
        a.name.toUpperCase() === "GLOBAL"
          ? -1
          : b.name.toUpperCase() === "GLOBAL"
            ? 1
            : a.name.toLowerCase().localeCompare(b.name.toLowerCase()),
      )
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
      }),
  );
  const selection = (group: string) =>
    ui.client
      .pendingRequests()
      .find(
        (request) =>
          ["proxies.select", "proxies.unfix"].includes(request.command) &&
          request.payload.group === group,
      );
  const selected = (group: Group) =>
    selectedByGroup().has(group.name)
      ? selectedByGroup().get(group.name)
      : (() => {
          const value = roster(group),
            name = value && selectedMember(value);
          return name && members(group).some((proxy) => proxy.name === name)
            ? name
            : undefined;
        })();
  const headerSelected = (group: Group) =>
    "effectiveSelectedNode" in group
      ? group.effectiveSelectedNode || group.selected
      : selected(group);
  const select = async (row: Row) => {
    if (
      selection(row.group.name) ||
      !selectableGroup(row.group) ||
      !fresh(row.group)
    )
      return;
    const pinned = row.group.fixed === row.proxy?.name;
    feedback.set({
      ...feedback(),
      [row.group.name]: ui.t("正在提交选择", "Applying selection"),
    });
    const result = await ui.client.run(
      pinned ? "proxies.unfix" : "proxies.select",
      pinned
        ? { group: row.group.name }
        : { group: row.group.name, name: row.proxy!.name },
    );
    if (!alive()) return;
    feedback.set({
      ...feedback(),
      [row.group.name]:
        result === undefined
          ? ui.client.error() ||
            ui.t(
              "选择失败，保留先前节点。",
              "Selection failed; the previous node is retained.",
            )
          : ui.client.warning() || ui.t("选择已生效", "Selection applied"),
    });
  };
  const disabled = (group: Group) =>
    !!selection(group.name) || !selectableGroup(group) || !fresh(group);
  const isSelected = (row: Row) => selected(row.group) === row.proxy?.name;
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
        props.section ||
        (platform.desktop && !!props.groupKey) ||
        (props.groupKey && feedback()[props.groupKey]) ? (
          <View>
            <If when={platform.desktop && !!props.groupKey}>
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
            </If>
            <If when={props.groupKey && feedback()[props.groupKey]}>
              <View
                style={{
                  padding: density.space(10),
                  borderBottomWidth: 1,
                  borderBottomColor: palette().line,
                }}
              >
                <Hint text={feedback()[props.groupKey!]} />
              </View>
            </If>
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
                    item().group.fixed === item().proxy!.name
                      ? ui.t("恢复自动选择", "Use automatic selection")
                      : item().proxy!.name
                  }
                  accessibilityState={{
                    disabled: disabled(item().group),
                    selected: isSelected(item()),
                  }}
                  disabled={disabled(item().group)}
                  onPress={() => void select(item())}
                  focusStyle={focusRing()}
                  hoverStyle={{ backgroundColor: palette().hover }}
                  activeStyle={{
                    backgroundColor: isSelected(item())
                      ? palette().accentFill
                      : palette().soft,
                  }}
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
                    busy={
                      !!selection(item().group.name) &&
                      (selection(item().group.name)!.payload.name ===
                        item().proxy!.name ||
                        item().group.fixed === item().proxy!.name)
                    }
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
                        {item().group.fixed === item().proxy!.name
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
                      {item().group.fixed === item().proxy!.name
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
                    <If
                      when={
                        item().group.fixed === item().proxy!.name ||
                        !selectableGroup(item().group)
                      }
                    >
                      <Text
                        style={{
                          fontSize: density.font(11),
                          lineHeight: density.line(14),
                          color: palette().muted,
                        }}
                      >
                        {item().group.fixed === item().proxy!.name
                          ? ui.t("恢复自动", "Restore auto")
                          : ui.t("自动", "Auto")}
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
                focusStyle={focusRing()}
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
                    {selection(item().group.name)
                      ? ui.t("提交中", "Applying")
                      : groupModeLabel(item().group, ui.t)}
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
  const groups = () => ui.client.state()!.proxyGroups;
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
                focusStyle={focusRing()}
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
                    {group().name}
                  </Text>
                  <Text
                    style={{
                      fontSize: density.font(12),
                      lineHeight: density.line(16),
                      color: palette().muted,
                      numberOfLines: 1,
                    }}
                  >
                    {groupModeLabel(group(), ui.t)} ·{" "}
                    {selectedMember(group()) || ui.t("未选择", "Not selected")}
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
              title={`${selectedGroup()!.name} · ${groupModeLabel(selectedGroup()!, ui.t)}`}
              meta={`${groupMembers(groups(), selectedGroup()!).length} ${ui.t("个节点", "nodes")}`}
            />
            <Actions align="start">
              <Action
                small
                icon="zap"
                loading={!!testingGroup()}
                disabled={!ui.client.domains.fresh(["proxies"])}
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
            <Hint
              text={
                testingGroup()
                  ? ui.t(
                      "正在测速，保留上次结果。",
                      "Testing; previous results remain visible.",
                    )
                  : ui.t(
                      "延迟为上次结果，测量时间未知；可重新测速。",
                      "Latency is a previous result with no timestamp. Test again to update.",
                    )
              }
            />
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

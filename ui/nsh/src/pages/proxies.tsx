import { computed, fontScale, signal, Text, View, windowSize, List } from "nsh";
import {
  Action,
  Chevron,
  control,
  Empty,
  Field,
  Hint,
  Icon,
  palette,
  radius,
  spacing,
  Spinner,
  useApp,
  useDomainInterests,
} from "../components";
import { includes } from "../locale";
import { focusRing, statusColors } from "../theme";
import { PageError } from "../page-error";
import { animateDisclosure, feedbackStyle } from "../motion";
import type { ProxyGroup, Proxy, ProxyGroupHeader } from "../model";
import {
  groupMembers,
  selectableGroup,
  selectedMember,
  proxyGroupSummary,
  groupSelectionMode,
} from "../proxy-groups";

type Group = ProxyGroup | ProxyGroupHeader;
type Row = { key: string; group: Group; proxy?: Proxy; loading?: boolean };
export function ProxyGroupList(props: { query?: string; section?: boolean; summary?: boolean }) {
  const ui = useApp(),
    expanded = signal("");
  const groups = (): Group[] =>
    props.summary ? ui.client.domains.data("summary")!.groups : ui.client.state()!.proxyGroups;
  const interest = (group: Group) =>
    props.summary ? { domain: "proxies" as const, groupKey: group.name } : ("proxies" as const);
  const roster = (group: Group): ProxyGroup | undefined =>
    props.summary
      ? ui.client.domains
          .data("proxies", group.name)
          ?.proxyGroups.find((row) => row.name === group.name)
      : (group as ProxyGroup);
  const members = (group: Group) => {
    const value = roster(group);
    return value ? groupMembers(groups(), value) : [];
  };
  const memberCount = (group: Group) =>
    "memberCount" in group ? group.memberCount : members(group).length;
  const fresh = (group: Group) => ui.client.domains.fresh([interest(group)]);
  useDomainInterests(() =>
    props.summary && expanded() ? [{ domain: "proxies", groupKey: expanded() }] : [],
  );
  const rows = computed<Row[]>(() =>
    groups()
      .slice()
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
        if (query && !filtered.length && !includes(query, group.name, group.groupType)) return [];
        const result: Row[] = [{ key: `group:${group.name}`, group }];
        if (expanded() === group.name || !!query) {
          if (props.summary && !ui.client.domains.has([interest(group)]))
            result.push({ key: `loading:${group.name}`, group, loading: true });
          else
            for (const proxy of filtered)
              result.push({ key: `member:${group.name}:${proxy.name}`, group, proxy });
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
  const selected = (group: Group) => {
    const value = roster(group),
      name = value && selectedMember(value);
    return name && members(group).some((proxy) => proxy.name === name) ? name : undefined;
  };
  const headerSelected = (group: Group) =>
    "effectiveSelectedNode" in group
      ? group.effectiveSelectedNode || group.selected
      : selected(group);
  const select = (row: Row) => {
    if (selection(row.group.name) || !selectableGroup(row.group) || !fresh(row.group)) return;
    const pinned = row.group.fixed === row.proxy?.name;
    void ui.client.run(
      pinned ? "proxies.unfix" : "proxies.select",
      pinned ? { group: row.group.name } : { group: row.group.name, name: row.proxy!.name },
    );
  };
  const modeLabel = (group: Group) => {
    const mode = groupSelectionMode(group);
    return mode === "automatic" ? ui.t("自动选择", "Automatic")
      : mode === "pinned" ? ui.t("已固定", "Pinned")
      : mode === "manual" ? ui.t("手动选择", "Manual")
      : ui.t("自动策略", "Policy");
  };
  const disabled = (group: Group) =>
    !!selection(group.name) || !selectableGroup(group) || !fresh(group);
  const isSelected = (row: Row) => selected(row.group) === row.proxy?.name;
  const isGroup = (proxy: Proxy) => groups().some((group) => group.name === proxy.name);
  const latencyTone = (proxy: Proxy) => proxy.delayMs == null ? "neutral"
    : proxy.delayMs <= 200 ? "success" : proxy.delayMs <= 500 ? "warning" : "danger";
  const isOpen = (group: Group) => !!props.query || expanded() === group.name;
  const groupHeight = () => Math.max(control.height >= 44 ? 72 : 64, 36 * Math.max(1, fontScale()) + 24);
  const nodeHeight = () => Math.max(control.height >= 44 ? 64 : 56, 36 * Math.max(1, fontScale()) + 16);
  return (
    <List
      testID={props.summary ? "summary-groups-list" : "proxy-groups-list"}
      bounces={false}
      each={rows()}
      keyExtractor={(row) => row.key}
      estimatedItemSize={groupHeight()}
      style={{
        flex: 1, minHeight: 0, borderWidth: 1, borderColor: palette().border,
        borderRadius: radius.lg, backgroundColor: palette().panel, overflow: "hidden",
      }}
      contentStyle={{ paddingBottom: 8 }}
      header={props.section ? (
        <View style={{ padding: 12, borderBottomWidth: 1, borderBottomColor: palette().border }}>
          <Text style={{ fontSize: 12, lineHeight: 18, color: palette().muted }}>
            {ui.t("展开分组选择节点，点击已固定的节点恢复自动。", "Expand a group to choose a node. Press a pinned node to restore automatic selection.")}
          </Text>
        </View>
      ) : undefined}
      empty={
        <Empty icon="git-branch" title={ui.t("暂无代理节点", "No proxy nodes")}
          text={ui.t("添加并启用订阅后即可选择节点", "Add and activate a subscription to select nodes")} />
      }
    >
      {(item) => (
        <If when={item().loading} else={
          <If when={!item().proxy} else={
            <View
              testID={`${props.summary ? "summary-" : ""}select-proxy-${item().proxy!.name}`}
              role="button"
              accessibilityLabel={item().group.fixed === item().proxy!.name
                ? ui.t("恢复自动选择", "Use automatic selection") : item().proxy!.name}
              accessibilityState={{ disabled: disabled(item().group), selected: isSelected(item()) }}
              disabled={disabled(item().group)}
              onPress={() => select(item())}
              focusStyle={focusRing()}
              hoverStyle={{ backgroundColor: palette().soft }}
              activeStyle={{ opacity: 0.65 }}
              style={{
                height: nodeHeight(), paddingLeft: 20, paddingRight: 12,
                borderBottomWidth: 1, borderBottomColor: palette().border,
                backgroundColor: isSelected(item()) ? palette().accentFill : palette().panel,
                flexDirection: "row", alignItems: "center", gap: 10, ...feedbackStyle(),
              }}
            >
              <If when={selection(item().group.name)
                  && (selection(item().group.name)!.payload.name === item().proxy!.name
                    || item().group.fixed === item().proxy!.name)} else={
                <Icon name={isSelected(item()) ? "circle-check" : isGroup(item().proxy!) ? "folder-tree" : "circle"}
                  size={17} color={isSelected(item()) ? palette().accent : palette().muted} />
              }>
                <Spinner size={17} color={palette().accent} />
              </If>
              <View style={{ flex: 1, minWidth: 0, gap: 3 }}>
                <Text style={{ numberOfLines: 1, fontSize: 14, lineHeight: 20,
                  fontWeight: isSelected(item()) ? "600" : "500", color: palette().text }}>
                  {item().proxy!.name}
                </Text>
                <Text style={{ numberOfLines: 1, fontSize: 12, lineHeight: 16, color: palette().muted }}>
                  {isGroup(item().proxy!) ? ui.t("策略分组", "Policy group") : item().proxy!.proxyType.toUpperCase()}
                  {item().group.fixed === item().proxy!.name ? ui.t(" · 已固定", " · Pinned")
                    : isSelected(item()) ? ui.t(" · 当前使用", " · In use") : ""}
                </Text>
              </View>
              <View style={{ alignItems: "flex-end", gap: 3 }}>
                <View style={{ minWidth: 52, minHeight: 24, paddingVertical: 4, paddingHorizontal: 7, borderRadius: 6,
                  backgroundColor: statusColors(latencyTone(item().proxy!)).background,
                  alignItems: "center", justifyContent: "center" }}>
                  <Text style={{ fontSize: 12, lineHeight: 16, fontWeight: "500",
                    fontVariant: ["tabular-nums"], color: statusColors(latencyTone(item().proxy!)).text }}>
                    {item().proxy!.delayMs == null ? ui.t("未测试", "Not tested") : `${item().proxy!.delayMs} ms`}
                  </Text>
                </View>
                <If when={item().group.fixed === item().proxy!.name || !selectableGroup(item().group)}>
                  <Text style={{ fontSize: 11, lineHeight: 14, color: palette().muted }}>
                    {item().group.fixed === item().proxy!.name ? ui.t("恢复自动", "Restore auto") : ui.t("自动", "Auto")}
                  </Text>
                </If>
              </View>
            </View>
          }>
            <View
              testID={`${props.summary ? "summary-" : ""}group-${item().group.name}`}
              role="button"
              accessibilityLabel={item().group.name}
              accessibilityState={{ expanded: isOpen(item().group), disabled: !!props.query }}
              disabled={!!props.query}
              onPress={() => {
                animateDisclosure();
                expanded.set(expanded() === item().group.name ? "" : item().group.name);
              }}
              focusStyle={focusRing()}
              hoverStyle={{ backgroundColor: palette().soft }}
              activeStyle={{ opacity: 0.65 }}
              style={{
                height: groupHeight(), paddingHorizontal: spacing.md, gap: 10,
                borderBottomWidth: 1, borderBottomColor: palette().border,
                backgroundColor: isOpen(item().group) ? palette().raised : palette().panel,
                flexDirection: "row", alignItems: "center", ...feedbackStyle(),
              }}
            >
              <View style={{ width: 28, height: 28, borderRadius: radius.sm,
                backgroundColor: palette().soft, alignItems: "center", justifyContent: "center" }}>
                <Icon name={item().group.name.toUpperCase() === "GLOBAL" ? "network" : "git-branch"}
                  size={15} color={isOpen(item().group) ? palette().accent : palette().muted} />
              </View>
              <View style={{ flex: 1, minWidth: 0, gap: 4 }}>
                <Text style={{ numberOfLines: 1, fontSize: 14, lineHeight: 20,
                  fontWeight: "600", color: palette().text }}>
                  {item().group.name.toUpperCase() === "GLOBAL" ? ui.t("全局节点", "Global nodes") : item().group.name}
                </Text>
                <Text style={{ numberOfLines: 1, fontSize: 12, lineHeight: 16,
                  color: headerSelected(item().group) ? palette().text : palette().muted }}>
                  {headerSelected(item().group) || ui.t("未选择节点", "No node selected")}
                </Text>
              </View>
              <View style={{ alignItems: "flex-end", gap: 4 }}>
                <Text style={{ fontSize: 11, lineHeight: 16, color: palette().muted }}>
                  {modeLabel(item().group)}
                </Text>
                <Text style={{ fontSize: 12, lineHeight: 16, color: palette().muted,
                  fontVariant: ["tabular-nums"] }}>
                  {memberCount(item().group)} {ui.t("个节点", "nodes")}
                </Text>
              </View>
              <Chevron expanded={isOpen(item().group)} />
            </View>
          </If>
        }>
          <View style={{ minHeight: 56, padding: 12, gap: 8 }}>
            <If when={ui.client.domains.error([interest(item().group)])} else={
              <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
                <Spinner size={16} />
                <Hint text={ui.t("正在读取分组节点…", "Loading group members…")} />
              </View>
            }>
              <PageError id={`group-error-${item().group.name}`} text={ui.client.domains.error([interest(item().group)])} />
              <Action small secondary label={ui.t("重试", "Retry")}
                onPress={() => ui.client.domains.retry([interest(item().group)])} />
            </If>
          </View>
        </If>
      )}
    </List>
  );
}

export function Proxies() {
  const ui = useApp(),
    query = signal("");
  const groups = () => ui.client.state()!.proxyGroups;
  const summary = () =>
    proxyGroupSummary(
      groups().filter(
        (group) =>
          includes(query(), group.name, group.groupType, selectedMember(group)) ||
          groupMembers(groups(), group).some((proxy) =>
            includes(
              query(),
              proxy.name,
              proxy.proxyType,
              proxy.delayMs == null ? "" : String(proxy.delayMs),
            ),
          ),
      ),
    );
  return (
    <View
      style={{
        flex: 1,
        paddingHorizontal: windowSize().width >= 840 ? 24 : 16,
        paddingTop: windowSize().width >= 840 ? 24 : 16,
      }}
    >
      <Field
        label={ui.t("搜索节点", "Search nodes")}
        value={query()}
        onChange={(value) => query.set(value)}
        placeholder={ui.t("输入节点名、类型、延迟或当前", "Search node, type, latency or current")}
      />
      <View style={{ height: 34, justifyContent: "center" }}>
        <Hint
          text={`${summary().global} ${ui.t("个全局节点", "global nodes")} · ${summary().groups} ${ui.t("个策略组", "policy groups")}`}
        />
      </View>
      <ProxyGroupList query={query()} section />
    </View>
  );
}

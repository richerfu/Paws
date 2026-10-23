import { computed, signal, Text, View } from "nsh";
import { windowSize } from "nsh/device";
import { List } from "nsh/list";
import {
  Action,
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
import { PageError } from "../page-error";
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
  return (
    <List
      bounces={false}
      each={rows()}
      keyExtractor={(row) => row.key}
      estimatedItemSize={74}
      style={{ flex: 1 }}
      contentContainerStyle={{ paddingBottom: 16 }}
      ListHeaderComponent={
        props.section ? (
          <View style={{ paddingTop: 8, paddingBottom: 8, gap: 2 }}>
            <Hint text={ui.t("订阅节点与策略组", "Subscription nodes and policy groups")} />
            <Text style={{ fontSize: 12, color: palette().muted }}>
              {ui.t(
                "展开分组后可切换节点；自动组支持固定或恢复自动。",
                "Expand a group to switch nodes, pin or restore automatic selection.",
              )}
            </Text>
          </View>
        ) : undefined
      }
      ListEmptyComponent={
        <Empty
          icon="git-branch"
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
                  class="hover:bg-[#f4f4f5] dark:hover:bg-[#27272a] focus-visible:ring-2 focus-visible:ring-[#71717a]"
                  testID={`${props.summary ? "summary-" : ""}select-proxy-${item().proxy!.name}`}
                  accessibilityRole="button"
                  accessibilityLabel={
                    item().group.fixed === item().proxy!.name
                      ? ui.t("恢复自动选择", "Use automatic selection")
                      : item().proxy!.name
                  }
                  accessibilityState={{
                    disabled:
                      !!selection(item().group.name) ||
                      !selectableGroup(item().group) ||
                      !fresh(item().group),
                  }}
                  disabled={
                    !!selection(item().group.name) ||
                    !selectableGroup(item().group) ||
                    !fresh(item().group)
                  }
                  onPress={() => select(item())}
                  activeOpacity={0.65}
                  style={{
                    height: 66,
                    paddingLeft: 22,
                    paddingRight: 12,
                    marginBottom: 4,
                    borderWidth: 1,
                    borderColor: palette().border,
                    borderRadius: radius.lg,
                    backgroundColor:
                      selected(item().group) === item().proxy!.name
                        ? palette().soft
                        : palette().panel,
                    flexDirection: "row",
                    alignItems: "center",
                    gap: 7,
                  }}
                >
                  <If
                    when={
                      selection(item().group.name) &&
                      (selection(item().group.name)!.payload.name === item().proxy!.name ||
                        item().group.fixed === item().proxy!.name)
                    }
                    else={
                      <Icon
                        name={
                          selected(item().group) === item().proxy!.name
                            ? "circle-check"
                            : groups().some((group) => group.name === item().proxy!.name)
                              ? "folder-tree"
                              : "circle"
                        }
                        size={16}
                        color={
                          selected(item().group) === item().proxy!.name
                            ? palette().success
                            : palette().muted
                        }
                      />
                    }
                  >
                    <Spinner size={16} color={palette().success} />
                  </If>
                  <View style={{ flex: 1, gap: 2 }}>
                    <Text
                      numberOfLines={1}
                      style={{
                        fontSize: 14,
                        lineHeight: 20,
                        fontWeight: "500",
                        color:
                          selected(item().group) === item().proxy?.name
                            ? palette().success
                            : palette().text,
                      }}
                    >
                      {item().proxy!.name}
                    </Text>
                    <Text
                      numberOfLines={1}
                      style={{ fontSize: 12, lineHeight: 16, color: palette().muted }}
                    >{`${item().proxy!.proxyType.toUpperCase()}${groups().some((group) => group.name === item().proxy!.name) ? ui.t(" · 分组", " · Group") : item().group.fixed === item().proxy!.name ? ui.t(" · 固定", " · Pinned") : ""}`}</Text>
                  </View>
                  <Text
                    style={{
                      fontSize: 12,
                      color: item().proxy?.delayMs != null ? palette().success : palette().muted,
                    }}
                  >
                    {!selectableGroup(item().group)
                      ? ui.t("自动", "Auto")
                      : item().group.fixed === item().proxy!.name
                        ? ui.t("恢复自动", "Restore auto")
                        : item().proxy!.delayMs == null
                          ? ui.t("未测试", "Not tested")
                          : `${item().proxy!.delayMs} ms`}
                  </Text>
                </View>
              }
            >
              <View
                class="hover:bg-[#f4f4f5] dark:hover:bg-[#27272a] focus-visible:ring-2 focus-visible:ring-[#71717a]"
                testID={`${props.summary ? "summary-" : ""}group-${item().group.name}`}
                accessibilityRole="button"
                accessibilityLabel={item().group.name}
                accessibilityState={{ expanded: expanded() === item().group.name }}
                onPress={() =>
                  expanded.set(expanded() === item().group.name ? "" : item().group.name)
                }
                activeOpacity={0.65}
                style={{
                  height: 86,
                  paddingHorizontal: spacing.md,
                  marginBottom: spacing.sm,
                  borderWidth: 1,
                  borderColor: palette().border,
                  borderRadius: radius.lg,
                  backgroundColor: palette().panel,
                  flexDirection: "row",
                  alignItems: "center",
                  gap: 10,
                }}
              >
                <View
                  style={{
                    width: 34,
                    height: 34,
                    borderRadius: radius.md,
                    backgroundColor: palette().soft,
                    alignItems: "center",
                    justifyContent: "center",
                  }}
                >
                  <Icon name="git-branch" size={17} color={palette().text} />
                </View>
                <View style={{ flex: 1 }}>
                  <Text
                    numberOfLines={1}
                    style={{
                      fontSize: 14,
                      lineHeight: 20,
                      fontWeight: "600",
                      color: palette().text,
                    }}
                  >
                    {item().group.name.toUpperCase() === "GLOBAL"
                      ? ui.t("全局节点", "Global nodes")
                      : item().group.name}
                  </Text>
                  <Text
                    numberOfLines={1}
                    style={{ fontSize: 12, lineHeight: 16, color: palette().muted }}
                  >
                    {item().group.groupType} ·{" "}
                    {groupSelectionMode(item().group) === "automatic"
                      ? ui.t("自动", "Auto")
                      : groupSelectionMode(item().group) === "pinned"
                        ? ui.t("已固定", "Pinned")
                        : groupSelectionMode(item().group) === "manual"
                          ? ui.t("手动选择", "Manual selection")
                          : ui.t("自动策略", "Automatic policy")}{" "}
                    · {memberCount(item().group)} {ui.t("个节点", "members")}
                  </Text>
                  <Text
                    numberOfLines={1}
                    style={{
                      fontSize: 12,
                      lineHeight: 16,
                      fontWeight: "600",
                      color: palette().success,
                    }}
                  >
                    {ui.t("当前", "Current")} ·{" "}
                    {headerSelected(item().group) || ui.t("未选择", "Not selected")}
                  </Text>
                </View>
                <Icon
                  name={expanded() === item().group.name ? "chevron-up" : "chevron-down"}
                  size={16}
                  color={palette().muted}
                />
              </View>
            </If>
          }
        >
          <View style={{ minHeight: 56, padding: 12, gap: 8 }}>
            <If
              when={ui.client.domains.error([interest(item().group)])}
              else={
                <>
                  <Spinner size={16} />
                  <Hint text={ui.t("正在读取分组节点…", "Loading group members…")} />
                </>
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
                onPress={() => ui.client.domains.retry([interest(item().group)])}
              />
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
        label=""
        value={query()}
        onChange={(value) => query.set(value)}
        placeholder={ui.t("输入节点名、类型、延迟或当前", "Search node, type, latency or current")}
      />
      <View style={{ height: 34, justifyContent: "center" }}>
        <Hint
          text={`${summary().global} ${ui.t("个全局节点", "global nodes")} · ${summary().groups} ${ui.t("个策略组", "policy groups")} · ${summary().members} ${ui.t("个节点", "members")}`}
        />
      </View>
      <ProxyGroupList query={query()} section />
    </View>
  );
}

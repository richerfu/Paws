import type { ProxyGroup } from "./model";

export function selectedMember(
  group: Pick<ProxyGroup, "selected"> & Partial<Pick<ProxyGroup, "proxies">>,
): string | undefined {
  return group.selected || group.proxies?.find((proxy) => proxy.selected)?.name;
}
export function effectiveGroupLeaf(groups: ProxyGroup[], root: string): string | undefined {
  let group = groups.find((group) => group.name === root);
  const visited = new Set<string>();
  while (group) {
    if (visited.has(group.name)) return undefined;
    visited.add(group.name);
    const selected = selectedMember(group);
    if (!selected) return undefined;
    const next = groups.find((group) => group.name === selected);
    if (!next) return selected;
    group = next;
  }
  return undefined;
}
export function primarySelectedGroupLeaf(groups: ProxyGroup[]): string | undefined {
  const visible = groups.filter((group) => group.name.toUpperCase() !== "GLOBAL");
  const primary =
    visible.find((group) => group.name.toLowerCase() === "proxy" && selectedMember(group)) ||
    visible.find((group) => selectedMember(group));
  return primary ? effectiveGroupLeaf(groups, primary.name) : undefined;
}
export function groupMembers(groups: Pick<ProxyGroup, "name">[], group: ProxyGroup) {
  if (group.name.toUpperCase() !== "GLOBAL") return group.proxies;
  const nodes = group.proxies.filter(
    (proxy) =>
      !groups.some((group) => group.name === proxy.name) &&
      !["direct", "reject", "rejectdrop"].includes(
        proxy.proxyType.toLowerCase().replace(/[^a-z0-9]/g, ""),
      ),
  );
  return nodes.length ? nodes : group.proxies.filter((proxy) => proxy.name === "DIRECT");
}
export function selectableGroup(group: Pick<ProxyGroup, "groupType">): boolean {
  return ["selector", "urltest", "fallback"].includes(
    group.groupType.toLowerCase().replace(/[^a-z0-9]/g, ""),
  );
}

export function proxyGroupSummary(groups: ProxyGroup[]) {
  const global = groups.find((group) => group.name.toUpperCase() === "GLOBAL");
  const policies = groups.filter((group) => group.name.toUpperCase() !== "GLOBAL");
  return {
    global: global ? groupMembers(groups, global).length : 0,
    groups: policies.length,
    members: policies.reduce((sum, group) => sum + group.proxies.length, 0),
  };
}

export function groupSelectionMode(
  group: Pick<ProxyGroup, "fixed" | "groupType">,
): "automatic" | "pinned" | "manual" | "policy" {
  return group.fixed === ""
    ? "automatic"
    : group.fixed
      ? "pinned"
      : selectableGroup(group)
        ? "manual"
        : "policy";
}

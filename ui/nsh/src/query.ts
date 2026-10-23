import type { Connection, Profile, Provider, Rule } from "./model";
import { includes } from "./locale";

const normalized = (value: string) => value.trim().toLowerCase();
const is = (value: string, ...aliases: string[]) => aliases.includes(value);

export function matchesProfile(profile: Profile, query: string): boolean {
  const q = normalized(query);
  return (
    includes(
      q,
      profile.name,
      profile.source,
      profile.rawYamlPath,
      profile.runtimeYamlPath,
      profile.subscriptionUrl,
      profile.updatedAt,
      profile.lastRefreshAt,
      profile.lastRefreshError,
      profile.subscriptionMetadata?.title,
      profile.subscriptionMetadata?.webPageUrl,
      profile.subscriptionMetadata?.supportUrl,
    ) ||
    (profile.active && is(q, "active", "使用中", "当前")) ||
    (!!profile.subscriptionUrl && is(q, "remote", "subscription", "订阅", "网络")) ||
    (!profile.subscriptionUrl && is(q, "local", "本地", "yaml")) ||
    (profile.refreshDue && is(q, "due", "待刷新", "可刷新")) ||
    (!!profile.lastRefreshError && is(q, "failed", "失败", "错误")) ||
    (profile.hasBackup && is(q, "backup", "备份", "可回滚"))
  );
}
export function matchesConnection(connection: Connection, query: string): boolean {
  return includes(
    query,
    connection.host,
    connection.domain,
    connection.destinationIp,
    connection.network,
    connection.rule,
    connection.rulePayload,
    connection.proxy,
    connection.startedAt,
    connection.updatedAt,
    ...(connection.chains || []),
  );
}
export function matchesRule(rule: Rule, query: string): boolean {
  const q = normalized(query);
  return (
    includes(q, rule.line, rule.source, String(rule.order)) ||
    (rule.enabled ? is(q, "enabled", "启用", "on") : is(q, "disabled", "停用", "关闭", "off"))
  );
}
export function matchesProvider(provider: Provider, query: string): boolean {
  const q = normalized(query);
  return (
    includes(
      q,
      provider.name,
      provider.providerType,
      provider.url,
      provider.path,
      provider.vehicleType,
      provider.intervalSeconds == null ? undefined : String(provider.intervalSeconds),
      provider.filter,
      provider.excludeFilter,
      provider.behavior,
      provider.format,
      provider.healthCheckUrl,
      provider.lastRefreshError,
    ) ||
    (provider.cacheExists && is(q, "cached", "已缓存", "缓存")) ||
    (!!provider.staleCacheAvailable && is(q, "stale", "旧缓存", "失败缓存")) ||
    (!!provider.healthCheckEnabled && is(q, "health", "health-check", "健康检查"))
  );
}
export function matchesGeodata(
  file: { name: string; path: string; exists: boolean },
  query: string,
): boolean {
  const q = normalized(query);
  return (
    includes(q, file.name, file.path) ||
    (file.exists ? is(q, "available", "可用", "exists") : is(q, "missing", "缺失"))
  );
}
export function matchesLog(
  log: { level: string; message: string; timestamp: string },
  query: string,
  level: string,
): boolean {
  const normalizedLevel = log.level.toLowerCase() === "warning" ? "warn" : log.level.toLowerCase();
  return (
    (level === "all" || normalizedLevel === level) &&
    includes(query, log.level, log.message, log.timestamp)
  );
}

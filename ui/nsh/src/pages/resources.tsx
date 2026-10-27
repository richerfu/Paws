import {
  computed,
  effect,
  signal,
  Text,
  untracked,
  View,
  windowSize,
  Clipboard,
  Scroll,
  List,
} from "nsh";
import {
  Action,
  Actions,
  Badge,
  Card,
  Code,
  confirm,
  Detail,
  Dialog,
  Empty,
  Field,
  Hint,
  Icon,
  IconButton,
  palette,
  radius,
  Tabs,
  useApp,
  useLifetime,
} from "../components";
import { bytes, includes, timestamp } from "../locale";
import { matchesGeodata, matchesProvider, matchesRule } from "../query";
import { ManualRuleDialog, type RuleContext } from "../manual-rule";
import type { UiState, Rule, Provider } from "../model";
import { PageError } from "../page-error";

type Lookup = {
  query: string;
  inputKind: "domain" | "ip";
  resolvedIp?: string;
  resolutionAttempted: boolean;
  matched: boolean;
  target: string;
  ruleLine?: string;
};
export function Resources() {
  const ui = useApp(),
    query = signal(""),
    lookup = signal(""),
    lookupOpen = signal(false),
    lookupResult = signal<Lookup | undefined>(),
    lookupBusy = signal(false);
  const alive = useLifetime(),
    manual = signal<RuleContext | undefined>(),
    providerName = signal<{ name: string; providerType: string } | undefined>(),
    geodata = signal<UiState["geodata"][number] | undefined>();
  const selectedRule = signal<Rule | undefined>(),
    runtimeRulesOpen = signal(false);
  const runtimeRules = () => ui.client.domains.data("rules")?.runtimeRules;
  let lookupGeneration = 0;
  const closeLookup = () => {
    lookupGeneration++;
    lookupOpen.set(false);
    lookupBusy.set(false);
  };
  effect(() => {
    const intent = ui.resourceIntent();
    if (intent)
      untracked(() => {
        if (!ui.consumeResourceIntent(intent.id)) return;
        if (intent.kind === "lookup") {
          lookupResult.set(undefined);
          lookupOpen.set(true);
        } else manual.set({});
      });
  });
  const current = () => ui.client.state()!,
    busy = () =>
      ui.client.pending() > 0 ||
      !ui.client.domains.fresh(["config", "rules", "providers", "geodata"]);
  const rules = computed(() => current().rules.filter((rule) => matchesRule(rule, query()))),
    providers = computed(() =>
      current().providers.filter((provider) => matchesProvider(provider, query())),
    );
  const selectedProvider = computed(() =>
    current().providers.find((provider) => provider.name === providerName()?.name && provider.providerType === providerName()?.providerType),
  );
  const selectedProviderRuntime = () => ui.client.domains.data("providers")?.runtimeAvailability?.find(item => item.name === selectedProvider()?.name && item.providerType === selectedProvider()?.providerType);
  const moveRule = (id: string, distance: number) => {
    const ids = current().rules.map((rule) => rule.id),
      from = ids.indexOf(id),
      to = from + distance;
    if (from < 0 || to < 0 || to >= ids.length) return;
    [ids[from], ids[to]] = [ids[to], ids[from]];
    void ui.client.run(
      "rules.reorder",
      { ids, expectedRevision: current().configRevision },
      ui.t("规则设置已保存", "Rule settings saved"),
    );
  };
  const ruleLookup = async () => {
    if (lookupBusy()) return;
    const ticket = ++lookupGeneration;
    lookupBusy.set(true);
    const result = await ui.client.run<Lookup>("rules.lookup", { query: lookup() });
    if (alive() && lookupOpen() && lookupGeneration === ticket) {
      lookupBusy.set(false);
      if (result !== undefined) lookupResult.set(result);
    }
  };
  const addFromLookup = () => {
    const result = lookupResult();
    if (!result) return;
    closeLookup();
    manual.set(
      result.inputKind === "domain"
        ? { domain: result.query, destinationIp: result.resolvedIp }
        : { destinationIp: result.query },
    );
  };
  const importRules = async () => {
    const revision = current().configRevision;
    const file = await ui.client.run<{ name: string; content: string } | null>(
      "platform.importFile",
    );
    if (alive() && file)
      await ui.client.run(
        "rules.import",
        { content: file.content, expectedRevision: revision },
        ui.t("规则已导入", "Rules imported"),
      );
  };
  type Row = {
    key: string;
    kind: string;
    rule?: Rule;
    provider?: Provider;
    file?: UiState["geodata"][number];
  };
  const rows = computed<Row[]>(() => [
    { key: "summary", kind: "summary" },
    { key: "geo-header", kind: "geo-header" },
    ...current()
      .geodata.filter((file) => matchesGeodata(file, query()))
      .map((file) => ({ key: `geo:${file.path}`, kind: "file", file })),
    { key: "provider-header", kind: "provider-header" },
    ...providers().map((provider) => ({
      key: `provider:${provider.providerType}:${provider.name}`,
      kind: "provider",
      provider,
    })),
    ...(providers().length ? [] : [{ key: "providers-empty", kind: "providers-empty" }]),
    { key: "rules-header", kind: "rules-header" },
    ...rules().map((rule) => ({ key: `rule:${rule.id}`, kind: "rule", rule })),
    ...(rules().length ? [] : [{ key: "rules-empty", kind: "rules-empty" }]),
  ]);
  return (
    <View
      style={{
        flex: 1,
        paddingHorizontal: windowSize().width >= 840 ? 24 : 16,
        paddingTop: windowSize().width >= 840 ? 24 : 16,
      }}
    >
      <View style={{ marginBottom: 12 }}>
        <Field
          label=""
          placeholder={ui.t(
            "输入资源名、规则、路径、类型或状态",
            "Search resource, rule, path, type or status",
          )}
          value={query()}
          onChange={(text) => query.set(text)}
        />
      </View>
      <List
        testID="resources-list"
        bounces={false}
        each={rows()}
        keyExtractor={(row) => row.key}
        estimatedItemSize={112}
        style={{ flex: 1 }}
        contentStyle={{ gap: 8, paddingBottom: 16 }}
      >
        {(row) => (
          <>
            <If when={row().kind === "summary"}>
              <Card
                title={ui.t("规则运行状态", "Rule runtime status")}
                subtitle={
                  ui.client.domains.data("summary")!.activeProfileName ||
                  ui.t("未选择", "Not selected")
                }
              >
                <Detail
                  tone={current().engineLoaded ? "success" : "neutral"}
                  label={ui.t("引擎配置", "Engine configuration")}
                  value={
                    current().engineLoaded ? ui.t("已加载", "Loaded") : ui.t("未加载", "Not loaded")
                  }
                />
                <Detail
                  tone="neutral"
                  label={ui.t("当前模式", "Current mode")}
                  value={
                    current().mode === "rule"
                      ? ui.t("规则", "Rule")
                      : current().mode === "global"
                        ? ui.t("全局", "Global")
                        : ui.t("直连", "Direct")
                  }
                />
                <Detail
                  label={ui.t("配置规则", "Profile rules")}
                  value={`${current().rules.filter((rule) => rule.enabled).length}/${current().rules.length}`}
                />
                <If when={runtimeRules() != null}>
                  <Detail
                    label={ui.t("内核运行规则", "Engine runtime rules")}
                    value={String(runtimeRules()!.length)}
                  />
                  <Action
                    id="runtime-rule-details"
                    secondary
                    small
                    label={ui.t("查看内核规则", "View engine rules")}
                    onPress={() => runtimeRulesOpen.set(true)}
                  />
                </If>
                <Detail label="Provider" value={String(current().providers.length)} />
                <Detail
                  label="GeoData"
                  value={`${current().geodata.filter((file) => file.exists).length}/${current().geodata.length}`}
                />
              </Card>
            </If>
            <If when={row().kind === "geo-header"}>
              <View
                style={{
                  height: 64,
                  marginTop: 12,
                  paddingHorizontal: 16,
                  flexDirection: "row",
                  alignItems: "center",
                  borderWidth: 1,
                  borderColor: palette().border,
                  borderRadius: radius.lg,
                }}
              >
                <Text style={{ flex: 1, fontSize: 14, fontWeight: "600", color: palette().text }}>
                  GeoData
                </Text>
                <Hint
                  text={`${current().geodata.filter((file) => file.exists).length}/${current().geodata.length} ${ui.t("可用", "available")}`}
                />
              </View>
            </If>
            <If when={row().kind === "file"}>
              <View
                role="button"
                accessibilityLabel={row().file!.name}
                onPress={() => geodata.set(row().file)}
                style={{
                  height: 76,
                  paddingLeft: 14,
                  paddingRight: 12,
                  flexDirection: "row",
                  alignItems: "center",
                  gap: 11,
                  borderWidth: 1,
                  borderColor: palette().border,
                  borderRadius: radius.lg,
                  backgroundColor: palette().panel,
                }}
              >
                <View
                  style={{
                    width: 36,
                    height: 36,
                    borderRadius: radius.lg,
                    backgroundColor: palette().soft,
                    alignItems: "center",
                    justifyContent: "center",
                  }}
                >
                  <Icon
                    name="file-text"
                    size={17}
                    color={row().file!.exists ? palette().success : palette().danger}
                  />
                </View>
                <View style={{ flex: 1, minWidth: 0, gap: 3 }}>
                  <Text
                    numberOfLines={1}
                    style={{ fontSize: 14, fontWeight: "600", color: palette().text }}
                  >
                    {row().file!.name}
                  </Text>
                  <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
                    <Badge
                      tone={row().file!.exists ? "success" : "danger"}
                      label={
                        row().file!.exists ? ui.t("可用", "Available") : ui.t("缺失", "Missing")
                      }
                    />
                    <Hint text={bytes(row().file!.bytes)} />
                  </View>
                </View>
                <Icon name="chevron-right" size={15} color={palette().muted} />
              </View>
            </If>
            <If when={row().kind === "provider-header"}>
              <Text
                style={{ fontSize: 14, fontWeight: "600", color: palette().text, marginTop: 12 }}
              >
                Provider
              </Text>
            </If>
            <If when={row().kind === "provider"}>
              <ProviderCard
                provider={row().provider!}
                onDetails={() => providerName.set({ name: row().provider!.name, providerType: row().provider!.providerType })}
              />
            </If>
            <If when={row().kind === "providers-empty"}>
              <Empty
                icon="database"
                title={ui.t("当前订阅没有 Provider", "This subscription has no providers")}
                text={ui.t(
                  "分享链接订阅通常只包含节点；Provider 需由 Clash YAML 显式声明",
                  "Link subscriptions usually contain nodes only; providers must be declared in Clash YAML.",
                )}
              />
            </If>
            <If when={row().kind === "rules-header"}>
              <View
                style={{
                  height: 56,
                  marginTop: 12,
                  flexDirection: "row",
                  alignItems: "center",
                  gap: 8,
                }}
              >
                <Text style={{ flex: 1, fontSize: 14, fontWeight: "600", color: palette().text }}>
                  {ui.t("规则", "Rules")}
                </Text>
                <Action
                  secondary
                  small
                  label={ui.t("导入规则", "Import rules")}
                  disabled={busy() || !current().activeProfile}
                  onPress={() => void importRules()}
                />
                <Action
                  secondary
                  small
                  icon="plus"
                  label={ui.t("添加", "Add")}
                  disabled={!current().activeProfile}
                  onPress={() => manual.set({})}
                />
              </View>
            </If>
            <If when={row().kind === "rule"}>
              <RuleRow
                rule={row().rule!}
                onMove={moveRule}
                onDetails={() => selectedRule.set(row().rule)}
              />
            </If>
            <If when={row().kind === "rules-empty"}>
              <Empty
                icon="database"
                text={ui.t("当前配置没有可编辑规则", "This profile has no editable rules")}
              />
            </If>
          </>
        )}
      </List>
      <If when={runtimeRulesOpen()}>
        <Dialog
          title={ui.t("内核运行规则", "Engine runtime rules")}
          onClose={() => runtimeRulesOpen.set(false)}
        >
          <View style={{ height: Math.min(360, windowSize().height * 0.4) }}>
            <List
              bounces={false}
              each={runtimeRules() || []}
              keyExtractor={(_, index) => String(index)}
              estimatedItemSize={68}
              style={{ flex: 1 }}
            >
              {(rule) => (
                <View style={{ minHeight: 64, paddingVertical: 8, flexDirection: "row", alignItems: "center", gap: 8 }}>
                  <View style={{ flex: 1, minWidth: 0 }}><Code
                    lines={2}
                    text={[rule().ruleType, rule().payload, rule().target]
                      .filter(Boolean)
                      .join(",")}
                  /></View><Action variant="ghost" small label={ui.t("复制", "Copy")} onPress={() => Clipboard.writeText([rule().ruleType, rule().payload, rule().target].filter(Boolean).join(","))} />
                </View>
              )}
            </List>
          </View>
          <Hint
            text={ui.t(
              "此列表来自当前运行内核；配置规则可在资源列表中编辑。",
              "This list comes from the running engine. Edit profile rules in the resource list.",
            )}
          />
        </Dialog>
      </If>
      <If when={selectedRule()}>
        <Dialog
          title={ui.t("规则详情", "Rule details")}
          onClose={() => selectedRule.set(undefined)}
        >
          <Detail
            label={ui.t("状态", "Status")}
            value={selectedRule()!.enabled ? ui.t("启用", "Enabled") : ui.t("停用", "Disabled")}
            tone={selectedRule()!.enabled ? "success" : "neutral"}
          />
          <Scroll
            bounces={false}
            style={{ height: Math.min(280, Math.max(120, windowSize().height * 0.3)) }}
          >
            <Code text={selectedRule()!.line} />
          </Scroll>
          <Actions>
            <Action
              secondary
              label={ui.t("关闭", "Close")}
              onPress={() => selectedRule.set(undefined)}
            />
            <Action
              label={ui.t("复制完整规则", "Copy full rule")}
              onPress={() => Clipboard.writeText(selectedRule()!.line)}
            />
          </Actions>
        </Dialog>
      </If>
      <If when={manual()}>
        <ManualRuleDialog context={manual()} onClose={() => manual.set(undefined)} />
      </If>
      <If when={lookupOpen()}>
        <Dialog
          title={ui.t("规则查询", "Rule lookup")}
          description={ui.t(
            "用共享规则编译器预览当前配置的域名或 IP 匹配，不代表运行内核的实际连接判定。",
            "Preview the current profile with the shared rule compiler; this is not an observed routing decision from the running engine.",
          )}
          onClose={closeLookup}
        >
          <Field
            id="rule-lookup-query"
            label=""
            value={lookup()}
            placeholder="example.com / 203.0.113.1"
            disabled={lookupBusy()}
            onChange={(text) => {
              lookupGeneration++;
              lookup.set(text);
              lookupResult.set(undefined);
            }}
          />
          <If when={current().mode !== "rule"}>
            <Hint
              text={ui.t(
                "当前不是规则模式，查询结果不会改变实际连接路径。",
                "The current mode is not Rule. Lookup does not change the actual route.",
              )}
            />
          </If>
          <If when={lookupResult()}>
            <View
              style={{
                padding: 12,
                gap: 8,
                borderWidth: 1,
                borderColor: lookupResult()!.matched ? palette().success : palette().border,
                backgroundColor: palette().soft,
                borderRadius: radius.lg,
              }}
            >
              <Hint
                text={
                  lookupResult()!.matched
                    ? ui.t("匹配到规则", "Rule matched")
                    : ui.t("未匹配规则", "No rule matched")
                }
              />
              <Code text={lookupResult()!.ruleLine || "—"} />
              <Detail label={ui.t("输入类型", "Input type")} value={lookupResult()!.inputKind} />
              <If when={lookupResult()!.resolutionAttempted}>
                <Detail
                  label={ui.t("解析 IP", "Resolved IP")}
                  value={lookupResult()!.resolvedIp || ui.t("未能解析", "Unresolved")}
                />
              </If>
              <Detail label={ui.t("目标", "Target")} value={lookupResult()!.target} />
              <Action
                secondary
                label={ui.t("添加针对此地址的规则", "Add a rule for this address")}
                onPress={addFromLookup}
              />
            </View>
          </If>
          <If when={ui.client.error()}>
            <PageError id="rule-lookup-error" text={ui.client.error()} />
          </If>
          <Action
            loading={lookupBusy()}
            label={lookupBusy() ? ui.t("正在查询…", "Looking up…") : ui.t("查询", "Look up")}
            disabled={lookupBusy() || !lookup().trim() || !current().activeProfile}
            onPress={() => void ruleLookup()}
          />
        </Dialog>
      </If>
      <If when={selectedProvider()}>
        <Dialog
          title={selectedProvider()!.name}
          description={
            selectedProvider()!.providerType + " · " + (selectedProvider()!.vehicleType || "")
          }
          onClose={() => providerName.set(undefined)}
        >
          <If when={selectedProviderRuntime()?.available === false}><PageError id={`provider-detail-runtime-${selectedProvider()!.providerType}-${selectedProvider()!.name}`} danger={false} text={selectedProviderRuntime()!.reason || ui.t("当前内核资源不可用", "The engine resource is unavailable")} /></If>
          <If when={selectedProvider()!.providerType === "proxy" && selectedProviderRuntime()?.available !== false}>
            <View style={{ height: 320 }}>
              <List
                bounces={false}
                each={selectedProvider()!.members}
                keyExtractor={(member) => member.name}
                estimatedItemSize={52}
                style={{ flex: 1 }}
              >
                {(member) => (
                  <View
                    style={{
                      minHeight: 50,
                      paddingLeft: 10,
                      paddingRight: 8,
                      flexDirection: "row",
                      alignItems: "center",
                      gap: 8,
                    }}
                  >
                    <View style={{ flex: 1, minWidth: 0 }}>
                      <Text
                        numberOfLines={1}
                        style={{ fontSize: 12, fontWeight: "600", color: palette().text }}
                      >
                        {member().name}
                      </Text>
                      <View style={{ flexDirection: "row", alignItems: "center", gap: 4 }}>
                        <Badge
                          tone={member().alive == null ? "neutral" : member().alive ? "success" : "danger"}
                          label={member().alive == null ? ui.t("未测试", "Not tested") : member().alive ? ui.t("可用", "Available") : ui.t("不可用", "Unavailable")}
                        />
                        <If when={member().alive === true}>
                          <Hint text={`${member().delayMs ?? "—"} ms`} />
                        </If>
                      </View>
                    </View>
                    <IconButton
                      id={`test-provider-${selectedProvider()!.name}-${member().name}`}
                      icon="zap"
                      label={ui.t("测速", "Test latency")}
                      loading={ui.client
                        .pendingRequests()
                        .some(
                          (request) =>
                            request.command === "providers.testProxy" &&
                            request.payload.provider === selectedProvider()!.name &&
                            request.payload.name === member().name,
                        )}
                      disabled={busy()}
                      onPress={() => {
                        if (!busy())
                          void ui.client.run("providers.testProxy", {
                            provider: selectedProvider()!.name,
                            name: member().name,
                          });
                      }}
                    />
                  </View>
                )}
              </List>
            </View>
          </If>
        </Dialog>
      </If>
      <If when={geodata()}>
        <Dialog
          title={geodata()!.name}
          description={
            geodata()!.exists
              ? ui.t("文件可用", "File available")
              : ui.t("文件缺失", "File missing")
          }
          onClose={() => geodata.set(undefined)}
        >
          <Detail
            tone={geodata()!.exists ? "success" : "danger"}
            label={ui.t("状态", "Status")}
            value={geodata()!.exists ? ui.t("已安装", "Installed") : ui.t("缺失", "Missing")}
          />
          <Detail label={ui.t("大小", "Size")} value={bytes(geodata()!.bytes)} />
          <Detail label={ui.t("更新时间", "Updated")} value={timestamp(geodata()!.updatedAt)} />
          <Hint text={ui.t("文件路径", "File path")} />
          <Code text={geodata()!.path} />
        </Dialog>
      </If>
    </View>
  );
}

export function renderObject(value: unknown, depth = 0): string {
  if (value == null) return "—";
  if (typeof value !== "object") return String(value);
  if (Array.isArray(value)) return value.map((item) => renderObject(item, depth + 1)).join("\n");
  return Object.entries(value)
    .map(([key, item]) => `${"  ".repeat(depth)}${key}: ${renderObject(item, depth + 1)}`)
    .join("\n");
}

function RuleRow(props: {
  rule: Rule;
  onMove: (id: string, delta: number) => void;
  onDetails: () => void;
}) {
  const ui = useApp(),
    current = () => ui.client.state()!,
    rule = () => props.rule,
    busy = () =>
      ui.client.pending() > 0 ||
      !ui.client.domains.fresh(["config", "rules", "providers", "geodata"]);
  return (
    <View
      style={{
        height: 104,
        paddingHorizontal: 10,
        paddingVertical: 8,
        gap: 4,
        borderWidth: 1,
        borderColor: palette().border,
        borderRadius: radius.lg,
        backgroundColor: palette().panel,
      }}
    >
      <View style={{ height: 32, flexDirection: "row", alignItems: "center", gap: 7 }}>
        <Text
          style={{
            color: rule().enabled ? palette().success : palette().muted,
            fontSize: 12,
            fontWeight: "600",
          }}
        >
          #{rule().order + 1}
        </Text>
        <Text
          numberOfLines={1}
          style={{ flex: 1, minWidth: 0, fontSize: 12, color: palette().muted }}
        >
          {rule().source === "profile-yaml"
            ? ui.t("订阅 YAML", "Subscription YAML")
            : rule().source}
        </Text>
        <If when={rule().source !== "profile-yaml"}>
          <IconButton
            icon="chevron-up"
            label={ui.t("上移", "Move up")}
            small
            disabled={busy() || current().rules[0]?.id === rule().id}
            onPress={() => props.onMove(rule().id, -1)}
          />
          <IconButton
            id={`rule-down-${rule().id}`}
            icon="chevron-down"
            label={ui.t("下移", "Move down")}
            small
            disabled={busy() || current().rules[current().rules.length - 1]?.id === rule().id}
            onPress={() => props.onMove(rule().id, 1)}
          />
          <Action
            variant="ghost"
            small
            label={rule().enabled ? ui.t("停用", "Disable") : ui.t("启用", "Enable")}
            disabled={busy()}
            onPress={() =>
              void ui.client.run(
                "rules.enabled",
                {
                  id: rule().id,
                  enabled: !rule().enabled,
                  expectedRevision: current().configRevision,
                },
                ui.t("规则设置已保存", "Rule settings saved"),
              )
            }
          />
          <IconButton
            icon="trash-2"
            label={ui.t("删除", "Delete")}
            small
            disabled={busy()}
            onPress={() => {
              const id = rule().id,
                expectedRevision = current().configRevision;
              void confirm(
                ui,
                ui.t("删除此规则？", "Delete this rule?"),
                () =>
                  void ui.client.run(
                    "rules.delete",
                    { id, expectedRevision },
                    ui.t("规则设置已保存", "Rule settings saved"),
                  ),
              );
            }}
          />
        </If>
      </View>
      <View
        testID={`rule-details-${rule().id}`}
        role="button"
        accessibilityLabel={ui.t("查看完整规则", "View complete rule")}
        onPress={props.onDetails}
        style={{ minWidth: 0 }}
      >
        <Code text={rule().line} lines={2} />
      </View>
    </View>
  );
}
function ProviderCard(props: { provider: Provider; onDetails: () => void }) {
  const ui = useApp(),
    provider = () => props.provider,
    busy = () =>
      ui.client.pending() > 0 ||
      !ui.client.domains.fresh(["config", "rules", "providers", "geodata"]);
  const runtime = () => ui.client.domains.data("providers")?.runtimeAvailability?.find(item => item.name === provider().name && item.providerType === provider().providerType);
  const refreshing = () =>
    ui.client
      .pendingRequests()
      .some(
        (request) =>
          request.command === "providers.refresh" &&
          request.payload.name === provider().name &&
          request.payload.providerType === provider().providerType,
      );
  const checking = () =>
    provider().providerType === "proxy" &&
    ui.client
      .pendingRequests()
      .some(
        (request) =>
          request.command === "providers.healthcheck" && request.payload.name === provider().name,
      );
  const refresh = () => {
    if (!busy())
      void ui.client.run(
        "providers.refresh",
        { name: provider().name, providerType: provider().providerType },
        ui.t(`资源 ${provider().name} 已刷新`, `Resource ${provider().name} refreshed`),
      );
  };
  const check = () => {
    if (!busy()) void ui.client.run("providers.healthcheck", { name: provider().name });
  };
  return (
    <Card
      title={provider().name}
      subtitle={provider().providerType + " · " + (provider().vehicleType || "")}
    >
      <If when={runtime()}><Detail tone={runtime()!.available ? "success" : "warning"} label={ui.t("内核资源", "Engine resource")} value={runtime()!.available ? ui.t("已接入", "Available") : ui.t("当前不可用", "Unavailable")} /><If when={runtime()!.reason}><PageError id={`provider-runtime-${provider().providerType}-${provider().name}`} danger={false} text={runtime()!.reason!} /></If><If when={runtime()!.ruleCount != null}><Detail label={ui.t("运行规则数", "Runtime rule count")} value={String(runtime()!.ruleCount)} /></If><If when={runtime()!.updatedAt}><Detail label={ui.t("内核更新时间", "Engine update time")} value={timestamp(runtime()!.updatedAt || undefined)} /></If></If>
      <Detail
        tone={
          refreshing()
            ? "warning"
            : provider().lastRefreshError
              ? "danger"
              : provider().cacheExists
                ? "success"
                : "warning"
        }
        label={ui.t("状态", "Status")}
        value={
          refreshing()
            ? ui.t("正在刷新…", "Refreshing…")
            : provider().lastRefreshError
              ? ui.t("刷新失败", "Refresh failed")
              : provider().cacheExists
                ? ui.t("缓存已加载", "Cache loaded")
                : ui.t("等待缓存", "Waiting for cache")
        }
      />
      <Detail
        label={ui.t("缓存", "Cache")}
        value={provider().cacheExists ? bytes(provider().cacheBytes) : ui.t("无", "None")}
      />
      <If when={provider().path}>
        <Detail stacked label={ui.t("路径", "Path")} value={provider().path!} />
      </If>
      <If when={provider().url}>
        <Detail stacked label="URL" value={provider().url!} />
      </If>
      <If when={provider().intervalSeconds}>
        <Detail
          label={ui.t("刷新间隔", "Refresh interval")}
          value={`${provider().intervalSeconds} s`}
        />
      </If>
      <If when={provider().filter}>
        <Detail label={ui.t("过滤", "Filter")} value={provider().filter!} />
      </If>
      <If when={provider().excludeFilter}>
        <Detail label={ui.t("排除过滤", "Exclude filter")} value={provider().excludeFilter!} />
      </If>
      <If when={provider().behavior}>
        <Detail label={ui.t("行为", "Behavior")} value={provider().behavior!} />
      </If>
      <If when={provider().format}>
        <Detail label={ui.t("格式", "Format")} value={provider().format!} />
      </If>
      <If when={provider().providerType === "proxy"}>
        <Detail
          label={ui.t("成员健康", "Member health")}
          value={`${ui.t("可用", "Available")} ${provider().members.filter((member) => member.alive === true).length} · ${ui.t("不可用", "Unavailable")} ${provider().members.filter((member) => member.alive === false).length} · ${ui.t("未测试", "Not tested")} ${provider().members.filter((member) => member.alive == null).length}`}
        />
      </If>
      <If when={provider().lastRefreshError}>
        <PageError id={`provider-error-${provider().providerType}-${provider().name}`} text={provider().lastRefreshError!} />
      </If>
      <Actions>
        <If when={provider().providerType === "proxy"}>
          <Action secondary small label={ui.t("详情", "Details")} onPress={props.onDetails} />
          <Action
            id={`healthcheck-provider-${provider().name}`}
            secondary
            small
            loading={checking()}
            label={ui.t("检查", "Check")}
            disabled={busy()}
            onPress={check}
          />
        </If>
        <Action
          id={`refresh-provider-${provider().providerType}-${provider().name}`}
          secondary
          small
          icon="refresh-cw"
          loading={refreshing()}
          label={ui.t("刷新", "Refresh")}
          disabled={busy()}
          onPress={refresh}
        />
      </Actions>
    </Card>
  );
}

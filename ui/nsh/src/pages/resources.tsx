import { density } from "../theme";
import {
  computed,
  fontScale,
  effect,
  signal,
  Text,
  untracked,
  View,
  windowSize,
  Scroll,
  List,
} from "nsh";
import {
  Action,
  Actions,
  Badge,
  Card,
  Code,
  control,
  confirm,
  copyText,
  Detail,
  Dialog,
  Empty,
  Field,
  Hint,
  Icon,
  IconButton,
  IconTile,
  SearchField,
  Spinner,
  palette,
  radius,
  useApp,
  useLifetime,
  useDraftGuard,
  RouteOwner,
} from "../components";
import { bytes, timestamp } from "../locale";
import { matchesGeodata, matchesProvider, matchesRule } from "../query";
import { ManualRuleDialog, type RuleContext } from "../manual-rule";
import type { UiState, Rule, Provider } from "../model";
import { ObjectMenu } from "../object-menu";
import { feedbackStyle } from "../motion";
import { focusRing, surfaceFinish } from "../theme";
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
  const orderDraft = signal<{ ids: string[]; revision: number }>();
  const orderConflict = () => !!orderDraft() && orderDraft()!.revision !== ui.client.state()!.configRevision;
  useDraftGuard(() => !!orderDraft(), () => orderDraft.set(undefined), () => ui.client.isPending("rules.reorder"));
  const owner = RouteOwner() || ui.route();
  let lookupGeneration = 0;
  let afterLookup: RuleContext | undefined;
  const closeLookup = () => {
    lookupGeneration++;
    lookupOpen.set(false);
    lookupBusy.set(false);
  };
  effect(() => {
    const intent = ui.resourceIntent();
    if (ui.route() !== owner) return;
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
  const rules = computed(() =>
      (orderDraft() ? orderDraft()!.ids.flatMap((id, order) => {
        const rule = current().rules.find((rule) => rule.id === id);
        return rule ? [{ ...rule, order }] : [];
      }) : current().rules).filter((rule) => matchesRule(rule, query())),
    ),
    providers = computed(() =>
      current().providers.filter((provider) =>
        matchesProvider(provider, query()),
      ),
    );
  const selectedProvider = computed(() =>
    current().providers.find(
      (provider) =>
        provider.name === providerName()?.name &&
        provider.providerType === providerName()?.providerType,
    ),
  );
  const selectedProviderRuntime = () =>
    ui.client.domains
      .data("providers")
      ?.runtimeAvailability?.find(
        (item) =>
          item.name === selectedProvider()?.name &&
          item.providerType === selectedProvider()?.providerType,
      );
  const moveRule = (id: string, distance: number) => {
    if (orderConflict() || busy()) return;
    const ids = orderDraft()?.ids.slice() || current().rules.map((rule) => rule.id),
      from = ids.indexOf(id),
      to = from + distance;
    if (from < 0 || to < 0 || to >= ids.length) return;
    [ids[from], ids[to]] = [ids[to], ids[from]];
    orderDraft.set({ ids, revision: orderDraft()?.revision ?? current().configRevision });
  };
  const saveOrder = async () => {
    const submitted = orderDraft();
    if (!submitted || orderConflict() || busy()) return;
    const result = await ui.client.run(
      "rules.reorder",
      { ids: submitted.ids, expectedRevision: submitted.revision },
      ui.t("规则设置已保存", "Rule settings saved"),
    );
    if (alive() && result !== undefined && orderDraft() === submitted) orderDraft.set(undefined);
  };
  const ruleLookup = async () => {
    if (lookupBusy() || !lookup().trim() || !current().activeProfile) return;
    const ticket = ++lookupGeneration;
    lookupBusy.set(true);
    const result = await ui.client.run<Lookup>("rules.lookup", {
      query: lookup(),
    });
    if (alive() && lookupOpen() && lookupGeneration === ticket) {
      lookupBusy.set(false);
      if (result !== undefined) lookupResult.set(result);
    }
  };
  const addFromLookup = () => {
    const result = lookupResult();
    if (!result) return;
    closeLookup();
    afterLookup = (
      result.inputKind === "domain"
        ? { domain: result.query, destinationIp: result.resolvedIp }
        : { destinationIp: result.query }
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
    ...(providers().length
      ? []
      : [{ key: "providers-empty", kind: "providers-empty" }]),
    { key: "rules-header", kind: "rules-header" },
    ...rules().map((rule) => ({ key: `rule:${rule.id}`, kind: "rule", rule })),
    ...(rules().length ? [] : [{ key: "rules-empty", kind: "rules-empty" }]),
  ]);
  return (
    <View
      style={{
        flex: 1,
        paddingHorizontal:
          windowSize().width >= 840 ? density.space(24) : density.space(16),
        paddingTop:
          windowSize().width >= 840 ? density.space(24) : density.space(16),
      }}
    >
      <View style={{ marginBottom: density.space(12) }}>
        <SearchField
          label={ui.t("搜索资源", "Search resources")}
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
        keyboardDismissMode="on-drag"
        keyboardShouldPersistTaps="handled"
        each={rows()}
        keyExtractor={(row) => row.key}
        estimatedItemSize={112}
        style={{ flex: 1 }}
        contentStyle={{
          gap: density.space(8),
          paddingBottom: density.space(16),
        }}
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
                    current().engineLoaded
                      ? ui.t("已加载", "Loaded")
                      : ui.t("未加载", "Not loaded")
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
                <Detail
                  label="Provider"
                  value={String(current().providers.length)}
                />
                <Detail
                  label="GeoData"
                  value={`${current().geodata.filter((file) => file.exists).length}/${current().geodata.length}`}
                />
              </Card>
            </If>
            <If when={row().kind === "geo-header"}>
              <View
                style={{
                  minHeight: 40,
                  marginTop: density.space(12),
                  paddingHorizontal: density.space(4),
                  flexDirection: "row",
                  alignItems: "center",
                }}
              >
                <Text
                  role="header"
                  style={{
                    flex: 1,
                    fontSize: density.font(14),
                    fontWeight: "600",
                    color: palette().text,
                  }}
                >
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
                focusStyle={focusRing()}
                hoverStyle={{ backgroundColor: palette().hover }}
                activeStyle={{ backgroundColor: palette().soft }}
                onPress={() => geodata.set(row().file)}
                style={{
                  minHeight: Math.max(
                    68,
                    density.line(38) * Math.max(1, fontScale()) +
                      density.space(24),
                  ),
                  paddingLeft: density.space(14),
                  paddingRight: density.space(12),
                  flexDirection: "row",
                  alignItems: "center",
                  gap: density.space(11),
                  ...feedbackStyle(),
                  borderWidth: 1,
                  borderColor: palette().border,
                  borderRadius: radius.xl,
                  borderCurve: "continuous",
                  backgroundColor: palette().panel,
                  ...surfaceFinish("card"),
                }}
              >
                <IconTile
                  icon="file-text"
                  color={
                    row().file!.exists ? palette().success : palette().danger
                  }
                />
                <View style={{ flex: 1, minWidth: 0, gap: density.space(3) }}>
                  <Text
                    style={{
                      numberOfLines: 1,
                      fontSize: density.font(14),
                      fontWeight: "600",
                      color: palette().text,
                    }}
                  >
                    {row().file!.name}
                  </Text>
                  <View
                    style={{
                      flexDirection: "row",
                      alignItems: "center",
                      gap: density.space(8),
                    }}
                  >
                    <Badge
                      tone={row().file!.exists ? "success" : "danger"}
                      label={
                        row().file!.exists
                          ? ui.t("可用", "Available")
                          : ui.t("缺失", "Missing")
                      }
                    />
                    <Hint text={bytes(row().file!.bytes)} />
                  </View>
                </View>
                <Icon name="chevron-right" size={15} color={palette().muted} />
              </View>
            </If>
            <If when={row().kind === "provider-header"}>
              <View
                style={{
                  minHeight: 40,
                  marginTop: density.space(12),
                  paddingHorizontal: density.space(4),
                  flexDirection: "row",
                  alignItems: "center",
                }}
              >
                <Text
                  role="header"
                  style={{
                    flex: 1,
                    fontSize: density.font(14),
                    fontWeight: "600",
                    color: palette().text,
                  }}
                >
                  Provider
                </Text>
                <Hint text={String(providers().length)} />
              </View>
            </If>
            <If when={row().kind === "provider"}>
              <ProviderCard
                provider={row().provider!}
                onDetails={() =>
                  providerName.set({
                    name: row().provider!.name,
                    providerType: row().provider!.providerType,
                  })
                }
              />
            </If>
            <If when={row().kind === "providers-empty"}>
              <Empty
                icon="database"
                title={ui.t(
                  "当前订阅没有 Provider",
                  "This subscription has no providers",
                )}
                text={ui.t(
                  "分享链接订阅通常只包含节点；Provider 需由 Clash YAML 显式声明",
                  "Link subscriptions usually contain nodes only; providers must be declared in Clash YAML.",
                )}
              />
            </If>
            <If when={row().kind === "rules-header"}>
              <View
                style={{
                  minHeight: control.height + 16,
                  marginTop: density.space(12),
                  paddingHorizontal: density.space(4),
                  flexWrap: "wrap",
                  flexDirection: "row",
                  alignItems: "center",
                  gap: density.space(8),
                }}
              >
                <Text
                  role="header"
                  style={{
                    flex: 1,
                    fontSize: density.font(14),
                    fontWeight: "600",
                    color: palette().text,
                  }}
                >
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
                text={ui.t(
                  "当前配置没有可编辑规则",
                  "This profile has no editable rules",
                )}
              />
            </If>
          </>
        )}
      </List>
      <If when={orderDraft()}>
        <View style={{ paddingVertical: density.space(12), gap: density.space(8), borderTopWidth: 1, borderTopColor: palette().border }}>
          <Hint danger={orderConflict()} text={orderConflict() ? ui.t("配置已变化，无法提交旧顺序。放弃预览后重新排列。", "Configuration changed. Discard this preview and reorder the current rules.") : ui.t("规则顺序预览，尚未保存。", "Rule order preview; not saved yet.")} />
          <Actions><Action id="discard-rule-order" label={ui.t("放弃排序", "Discard order")} onPress={() => orderDraft.set(undefined)} /><Action id="save-rule-order" primary label={ui.t("保存顺序", "Save order")} loading={ui.client.isPending("rules.reorder")} disabled={orderConflict() || busy()} onPress={() => void saveOrder()} /></Actions>
        </View>
      </If>
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
                <View
                  style={{
                    minHeight: 64,
                    paddingVertical: density.space(8),
                    flexDirection: "row",
                    alignItems: "center",
                    gap: density.space(8),
                  }}
                >
                  <View style={{ flex: 1, minWidth: 0 }}>
                    <Code
                      lines={2}
                      text={[rule().ruleType, rule().payload, rule().target]
                        .filter(Boolean)
                        .join(",")}
                    />
                  </View>
                  <Action
                    variant="ghost"
                    small
                    label={ui.t("复制", "Copy")}
                    onPress={() =>
                      copyText(
                        ui,
                        [rule().ruleType, rule().payload, rule().target]
                          .filter(Boolean)
                          .join(","),
                        ui.t("已复制规则", "Rule copied"),
                      )
                    }
                  />
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
          footer={
            <Actions>
              <Action
                secondary
                label={ui.t("关闭", "Close")}
                onPress={() => selectedRule.set(undefined)}
              />
              <Action
                label={ui.t("复制完整规则", "Copy full rule")}
                onPress={() =>
                  copyText(
                    ui,
                    selectedRule()!.line,
                    ui.t("已复制规则", "Rule copied"),
                  )
                }
              />
            </Actions>
          }
        >
          <Detail
            label={ui.t("状态", "Status")}
            value={
              selectedRule()!.enabled
                ? ui.t("启用", "Enabled")
                : ui.t("停用", "Disabled")
            }
            tone={selectedRule()!.enabled ? "success" : "neutral"}
          />
          <Scroll
            bounces={false}
            style={{
              height: Math.min(280, Math.max(120, windowSize().height * 0.3)),
            }}
          >
            <Code text={selectedRule()!.line} />
          </Scroll>
        </Dialog>
      </If>
      <If when={manual()}>
        <ManualRuleDialog
          context={manual()}
          onClose={() => manual.set(undefined)}
        />
      </If>
      <If when={lookupOpen()}>
        <Dialog
          title={ui.t("规则查询", "Rule lookup")}
          description={ui.t(
            "用共享规则编译器预览当前配置的域名或 IP 匹配，不代表运行内核的实际连接判定。",
            "Preview the current profile with the shared rule compiler; this is not an observed routing decision from the running engine.",
          )}
          onClose={closeLookup}
          afterClose={() => {
            if (alive() && afterLookup) manual.set(afterLookup);
            afterLookup = undefined;
          }}
          footer={
            <Action
              loading={lookupBusy()}
              primary
              label={ui.t("查询", "Look up")}
              disabled={
                lookupBusy() || !lookup().trim() || !current().activeProfile
              }
              onPress={() => void ruleLookup()}
            />
          }
        >
          <Field
            id="rule-lookup-query"
            label={ui.t("域名或 IP", "Domain or IP")}
            clearable
            returnKeyType="search"
            onSubmit={() => void ruleLookup()}
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
                padding: density.space(12),
                gap: density.space(8),
                borderWidth: 1,
                borderColor: lookupResult()!.matched
                  ? palette().success
                  : palette().border,
                backgroundColor: palette().soft,
                borderRadius: radius.lg,
                borderCurve: "continuous",
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
              <Detail
                label={ui.t("输入类型", "Input type")}
                value={lookupResult()!.inputKind}
              />
              <If when={lookupResult()!.resolutionAttempted}>
                <Detail
                  label={ui.t("解析 IP", "Resolved IP")}
                  value={
                    lookupResult()!.resolvedIp || ui.t("未能解析", "Unresolved")
                  }
                />
              </If>
              <Detail
                label={ui.t("目标", "Target")}
                value={lookupResult()!.target}
              />
              <Action
                secondary
                label={ui.t(
                  "添加针对此地址的规则",
                  "Add a rule for this address",
                )}
                onPress={addFromLookup}
              />
            </View>
          </If>
          <If when={ui.client.error()}>
            <PageError id="rule-lookup-error" text={ui.client.error()} />
          </If>
        </Dialog>
      </If>
      <If when={selectedProvider()}>
        <Dialog
          title={selectedProvider()!.name}
          description={
            selectedProvider()!.providerType +
            " · " +
            (selectedProvider()!.vehicleType || "")
          }
          onClose={() => providerName.set(undefined)}
        >
          <ProviderDetails provider={selectedProvider()!} />
          <If when={selectedProviderRuntime()?.available === false}>
            <PageError
              id={`provider-detail-runtime-${selectedProvider()!.providerType}-${selectedProvider()!.name}`}
              danger={false}
              text={
                selectedProviderRuntime()!.reason ||
                ui.t("当前内核资源不可用", "The engine resource is unavailable")
              }
            />
          </If>
          <If
            when={
              selectedProvider()!.providerType === "proxy" &&
              selectedProviderRuntime()?.available !== false
            }
          >
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
                      paddingLeft: density.space(10),
                      paddingRight: density.space(8),
                      flexDirection: "row",
                      alignItems: "center",
                      gap: density.space(8),
                    }}
                  >
                    <View style={{ flex: 1, minWidth: 0 }}>
                      <Text
                        style={{
                          numberOfLines: 1,
                          fontSize: density.font(12),
                          fontWeight: "600",
                          color: palette().text,
                        }}
                      >
                        {member().name}
                      </Text>
                      <View
                        style={{
                          flexDirection: "row",
                          alignItems: "center",
                          gap: density.space(4),
                        }}
                      >
                        <Badge
                          tone={
                            member().alive == null
                              ? "neutral"
                              : member().alive
                                ? "success"
                                : "danger"
                          }
                          label={
                            member().alive == null
                              ? ui.t("未测试", "Not tested")
                              : member().alive
                                ? ui.t("可用", "Available")
                                : ui.t("不可用", "Unavailable")
                          }
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
                            request.payload.provider ===
                              selectedProvider()!.name &&
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
            value={
              geodata()!.exists
                ? ui.t("已安装", "Installed")
                : ui.t("缺失", "Missing")
            }
          />
          <Detail
            label={ui.t("大小", "Size")}
            value={bytes(geodata()!.bytes)}
          />
          <Detail
            label={ui.t("更新时间", "Updated")}
            value={timestamp(geodata()!.updatedAt)}
          />
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
  if (Array.isArray(value))
    return value.map((item) => renderObject(item, depth + 1)).join("\n");
  return Object.entries(value)
    .map(
      ([key, item]) =>
        `${"  ".repeat(depth)}${key}: ${renderObject(item, depth + 1)}`,
    )
    .join("\n");
}

function RuleRow(props: {
  rule: Rule;
  onMove: (id: string, delta: number) => void;
  onDetails: () => void;
}) {
  const ui = useApp(),
    current = () => ui.client.state()!,
    rule = () => props.rule;
  const busy = () =>
    ui.client.pending() > 0 ||
    !ui.client.domains.fresh(["config", "rules", "providers", "geodata"]);
  const readonly = () => rule().source === "profile-yaml";
  return (
    <ObjectMenu
      id={`rule-menu-${rule().id}`}
      title={ui.t(`规则 ${rule().order + 1}`, `Rule ${rule().order + 1}`)}
      commands={[
        {
          label: ui.t("规则详情…", "Rule details…"),
          icon: "info",
          run: props.onDetails,
        },
        {
          label: ui.t("复制完整规则", "Copy full rule"),
          icon: "file-text",
          run: () =>
            copyText(ui, rule().line, ui.t("已复制规则", "Rule copied")),
        },
        {
          id: `rule-up-${rule().id}`,
          label: ui.t("上移", "Move up"),
          icon: "arrow-up",
          disabled:
            readonly() || busy() || rule().order === 0,
          run: () => props.onMove(rule().id, -1),
        },
        {
          id: `rule-down-${rule().id}`,
          label: ui.t("下移", "Move down"),
          icon: "arrow-down",
          disabled:
            readonly() ||
            busy() ||
            rule().order === current().rules.length - 1,
          run: () => props.onMove(rule().id, 1),
        },
        {
          id: `rule-enabled-${rule().id}`,
          label: rule().enabled
            ? ui.t("停用", "Disable")
            : ui.t("启用", "Enable"),
          icon: "circle-check",
          disabled: readonly() || busy(),
          run: () => {
            void ui.client.run(
              "rules.enabled",
              {
                id: rule().id,
                enabled: !rule().enabled,
                expectedRevision: current().configRevision,
              },
              ui.t("规则设置已保存", "Rule settings saved"),
            );
          },
        },
        {
          label: ui.t("删除…", "Delete…"),
          icon: "trash-2",
          danger: true,
          disabled: readonly() || busy(),
          run: () => {
            const id = rule().id,
              expectedRevision = current().configRevision;
            void confirm(
              ui,
              ui.t("删除此规则？", "Delete this rule?"),
              () => {
                void ui.client.run(
                  "rules.delete",
                  { id, expectedRevision },
                  ui.t("规则设置已保存", "Rule settings saved"),
                );
              },
              {
                actionLabel: ui.t("删除规则", "Delete rule"),
                description: rule().line,
              },
            );
          },
        },
      ]}
      row={(menu) => (
        <View
          style={{
            minHeight: Math.max(
              68,
              density.line(38) * Math.max(1, fontScale()) + density.space(24),
            ),
            padding: density.space(12),
            gap: density.space(8),
            borderWidth: 1,
            borderColor: palette().border,
            borderRadius: radius.xl,
            borderCurve: "continuous",
            backgroundColor: palette().panel,
            opacity: rule().enabled ? 1 : 0.65,
            flexDirection: "row",
            alignItems: "center",
            ...surfaceFinish("card"),
          }}
        >
          <Text
            style={{
              width: 28,
              fontVariant: ["tabular-nums"],
              fontSize: density.font(12),
              color: rule().enabled ? palette().text : palette().muted,
            }}
          >
            {rule().order + 1}
          </Text>
          <View
            testID={`rule-details-${rule().id}`}
            role="button"
            accessibilityLabel={ui.t("查看完整规则", "View complete rule")}
            onPress={props.onDetails}
            focusStyle={focusRing()}
            hoverStyle={{ backgroundColor: palette().hover }}
            activeStyle={{ backgroundColor: palette().soft }}
            style={{
              flex: 1,
              minWidth: 0,
              minHeight: control.height,
              justifyContent: "center",
              gap: density.space(4),
              borderRadius: radius.sm,
              borderCurve: "continuous",
            }}
          >
            <Code text={rule().line} lines={2} />
            <Text
              style={{
                fontSize: density.font(12),
                lineHeight: density.line(18),
                color: palette().muted,
              }}
            >
              {rule().enabled
                ? ui.t("已启用", "Enabled")
                : ui.t("已停用", "Disabled")}{" "}
              ·{" "}
              {readonly()
                ? ui.t("订阅 YAML", "Subscription YAML")
                : rule().source}
            </Text>
          </View>
          {menu}
        </View>
      )}
    />
  );
}
function ProviderCard(props: { provider: Provider; onDetails: () => void }) {
  const ui = useApp(),
    provider = () => props.provider;
  const busy = () =>
    ui.client.pending() > 0 ||
    !ui.client.domains.fresh(["config", "rules", "providers", "geodata"]);
  const refreshing = () =>
    ui.client
      .pendingRequests()
      .some(
        (r) =>
          r.command === "providers.refresh" &&
          r.payload.name === provider().name &&
          r.payload.providerType === provider().providerType,
      );
  const checking = () =>
    ui.client
      .pendingRequests()
      .some(
        (r) =>
          r.command === "providers.healthcheck" &&
          r.payload.name === provider().name,
      );
  return (
    <ObjectMenu
      id={`provider-menu-${provider().providerType}-${provider().name}`}
      title={provider().name}
      commands={[
        {
          id: `provider-details-${provider().name}`,
          label: ui.t("资源详情…", "Resource details…"),
          icon: "info",
          run: props.onDetails,
        },
        {
          id: `refresh-provider-${provider().providerType}-${provider().name}`,
          label: ui.t("刷新资源", "Refresh resource"),
          icon: "refresh-cw",
          disabled: busy(),
          run: () => {
            void ui.client.run(
              "providers.refresh",
              { name: provider().name, providerType: provider().providerType },
              ui.t(
                `资源 ${provider().name} 已刷新`,
                `Resource ${provider().name} refreshed`,
              ),
            );
          },
        },
        {
          id: `healthcheck-provider-${provider().name}`,
          label: ui.t("检查节点健康", "Check node health"),
          icon: "zap",
          disabled: busy() || provider().providerType !== "proxy",
          run: () => {
            void ui.client.run("providers.healthcheck", {
              name: provider().name,
            });
          },
        },
      ]}
      row={(menu) => (
        <View
          testID={`provider-row-${provider().providerType}-${provider().name}`}
          accessibilityState={{ busy: refreshing() || checking() }}
          style={{
            minHeight: Math.max(
              68,
              density.line(38) * Math.max(1, fontScale()) + density.space(24),
            ),
            padding: density.space(12),
            gap: density.space(12),
            flexDirection: "row",
            alignItems: "center",
            borderWidth: 1,
            borderColor: palette().border,
            borderRadius: radius.xl,
            borderCurve: "continuous",
            backgroundColor: palette().panel,
            ...surfaceFinish("card"),
          }}
        >
          <If
            when={refreshing() || checking()}
            else={<IconTile icon="layers" color={palette().muted} />}
          >
            <Spinner size={18} color={palette().muted} />
          </If>
          <View
            role="button"
            onPress={props.onDetails}
            focusStyle={focusRing()}
            hoverStyle={{ backgroundColor: palette().hover }}
            activeStyle={{ backgroundColor: palette().soft }}
            accessibilityLabel={provider().name}
            style={{
              flex: 1,
              minWidth: 0,
              minHeight: control.height,
              justifyContent: "center",
              gap: density.space(4),
              borderRadius: radius.sm,
              borderCurve: "continuous",
            }}
          >
            <Text
              style={{
                fontSize: density.font(14),
                lineHeight: density.line(20),
                numberOfLines: 1,
                fontWeight: "600",
                color: palette().text,
              }}
            >
              {provider().name}
            </Text>
            <Text
              style={{
                fontSize: density.font(12),
                lineHeight: density.line(18),
                numberOfLines: 1,
                color: provider().lastRefreshError
                  ? palette().danger
                  : palette().muted,
              }}
            >
              {provider().providerType} · {provider().vehicleType || "—"} ·{" "}
              {refreshing()
                ? ui.t("正在刷新", "Refreshing")
                : checking()
                  ? ui.t("正在检查", "Checking")
                  : provider().lastRefreshError
                    ? ui.t("刷新失败", "Refresh failed")
                    : provider().cacheExists
                      ? bytes(provider().cacheBytes)
                      : ui.t("等待缓存", "Waiting for cache")}
            </Text>
          </View>
          {menu}
        </View>
      )}
    />
  );
}
function ProviderDetails(props: { provider: Provider }) {
  const ui = useApp(),
    provider = () => props.provider;
  const runtime = () =>
    ui.client.domains
      .data("providers")
      ?.runtimeAvailability?.find(
        (item) =>
          item.name === provider().name &&
          item.providerType === provider().providerType,
      );
  const refreshing = () =>
    ui.client
      .pendingRequests()
      .some(
        (r) =>
          r.command === "providers.refresh" &&
          r.payload.name === provider().name &&
          r.payload.providerType === provider().providerType,
      );
  return (
    <View style={{ gap: density.space(12) }}>
      <If when={runtime()}>
        <Detail
          tone={runtime()!.available ? "success" : "warning"}
          label={ui.t("内核资源", "Engine resource")}
          value={
            runtime()!.available
              ? ui.t("已接入", "Available")
              : ui.t("当前不可用", "Unavailable")
          }
        />
        <If when={runtime()!.reason}>
          <PageError
            id={`provider-runtime-${provider().providerType}-${provider().name}`}
            danger={false}
            text={runtime()!.reason!}
          />
        </If>
        <If when={runtime()!.ruleCount != null}>
          <Detail
            label={ui.t("运行规则数", "Runtime rule count")}
            value={String(runtime()!.ruleCount)}
          />
        </If>
        <If when={runtime()!.updatedAt}>
          <Detail
            label={ui.t("内核更新时间", "Engine update time")}
            value={timestamp(runtime()!.updatedAt || undefined)}
          />
        </If>
      </If>
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
        value={
          provider().cacheExists
            ? bytes(provider().cacheBytes)
            : ui.t("无", "None")
        }
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
        <Detail
          label={ui.t("排除过滤", "Exclude filter")}
          value={provider().excludeFilter!}
        />
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
        <PageError
          id={`provider-error-${provider().providerType}-${provider().name}`}
          text={provider().lastRefreshError!}
        />
      </If>
    </View>
  );
}

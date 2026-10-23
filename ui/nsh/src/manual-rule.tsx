import { computed, signal, View } from "nsh";
import {
  Action,
  Code,
  Dialog,
  Field,
  Hint,
  palette,
  radius,
  Select,
  Tabs,
  Toggle,
  useApp,
  useLifetime,
  useDomainInterests,
} from "./components";
import { PageError } from "./page-error";

export type RuleContext = { domain?: string; destinationIp?: string; connectionId?: string };
export function rulePreview(kind: string, input: string, target: string): string {
  const normalized =
    kind === "domain-suffix"
      ? input
          .trim()
          .replace(/^\.+|\.+$/g, "")
          .toLowerCase()
      : kind === "domain"
        ? input.trim().replace(/\.+$/g, "").toLowerCase()
        : input.trim().toLowerCase();
  const value =
    kind === "ip-cidr" && normalized && !normalized.includes("/")
      ? normalized + (normalized.includes(":") ? "/128" : "/32")
      : normalized;
  return `${kind === "domain-suffix" ? "DOMAIN-SUFFIX" : kind === "domain" ? "DOMAIN" : normalized.includes(":") ? "IP-CIDR6" : "IP-CIDR"},${value},${target.trim().toUpperCase() === "DIRECT" ? "DIRECT" : target.trim()}`;
}
export function ManualRuleDialog(props: { context?: RuleContext; onClose: () => void }) {
  const ui = useApp(),
    alive = useLifetime(),
    baseline = ui.client.state()!.configRevision;
  useDomainInterests(() => ["rules", "summary"]);
  const kind = signal(
      props.context?.domain ? "domain" : props.context?.destinationIp ? "ip-cidr" : "domain",
    ),
    value = signal(props.context?.domain || props.context?.destinationIp || ""),
    target = signal("DIRECT"),
    disconnect = signal(true);
  const preview = computed(() => rulePreview(kind(), value(), target()));
  const conflict = computed(() => {
    if (!ui.client.domains.has(["rules"])) return undefined;
    const selector = preview().split(",").slice(0, 2).join(",");
    return ui.client.state()!.rules.find(
      (rule) =>
        rule.line
          .split(",")
          .slice(0, 2)
          .map((part) => part.trim())
          .join(",")
          .toLowerCase() === selector.toLowerCase(),
    );
  });
  const busy = signal(false);
  const save = async () => {
    busy.set(true);
    const result = await ui.client.run<{
      connectionCloseRequested: boolean;
      connectionCloseError?: string | null;
    }>("rules.add", {
      matchKind: kind(),
      value: value().trim(),
      target: target(),
      expectedRevision: baseline,
      ...(props.context?.connectionId
        ? { connectionId: props.context.connectionId, disconnectAfterSave: disconnect() }
        : {}),
    });
    if (!alive()) return;
    busy.set(false);
    if (result !== undefined) {
      if (result?.connectionCloseError)
        ui.client.warning.set(
          [
            ui.client.warning(),
            ui.t(
              "规则已保存，但关联连接未能断开：",
              "Rule saved, but the associated connection could not be closed: ",
            ) + result.connectionCloseError,
          ]
            .filter(Boolean)
            .join("\n"),
        );
      else if (ui.client.domains.fresh(["config", "rules", "summary"]))
        ui.client.notice.set(ui.t("规则已保存", "Rule saved"));
      props.onClose();
    }
  };
  return (
    <Dialog
      title={ui.t("添加规则", "Add rule")}
      description={ui.t(
        "为此域名或 IP 添加优先匹配的规则。",
        "Add a priority rule for this domain or IP.",
      )}
      onClose={props.onClose}
    >
      <If
        when={ui.client.domains.has(["rules", "summary"])}
        else={
          <>
            <Hint text={ui.t("正在读取规则与策略组…", "Loading rules and policy groups…")} />
            <If when={ui.client.domains.error(["rules", "summary"])}>
              <PageError
                id="manual-rule-read-error"
                text={ui.client.domains.error(["rules", "summary"])}
              />
              <Action
                secondary
                label={ui.t("重试", "Retry")}
                onPress={() => ui.client.domains.retry(["rules", "summary"])}
              />
            </If>
          </>
        }
      >
        <Tabs
          value={kind()}
          options={[
            { value: "domain", label: ui.t("精确域名", "Exact domain") },
            { value: "domain-suffix", label: ui.t("域名后缀", "Domain suffix") },
            { value: "ip-cidr", label: "IP/CIDR" },
          ]}
          onChange={(next) => {
            kind.set(next);
            if (next === "ip-cidr" && props.context?.destinationIp)
              value.set(props.context.destinationIp);
            else if (next !== "ip-cidr" && props.context?.domain) value.set(props.context.domain);
          }}
          disabled={busy()}
        />
        <Field
          id="manual-rule-value"
          label={ui.t("匹配内容", "Match value")}
          value={value()}
          onChange={(next) => value.set(next)}
          disabled={busy()}
          placeholder={kind() === "ip-cidr" ? "203.0.113.1/32" : "example.com"}
        />
        <Select
          label={ui.t("目标策略", "Target")}
          value={target()}
          options={[
            { value: "DIRECT", label: "DIRECT" },
            ...ui.client.domains
              .data("summary")!
              .groups.filter((group) => group.name.toUpperCase() !== "GLOBAL")
              .map((group) => ({ value: group.name, label: group.name })),
          ]}
          onChange={(next) => target.set(next)}
          disabled={busy()}
        />
        <View
          style={{
            padding: 8,
            gap: 3,
            borderWidth: 1,
            borderColor: palette().border,
            borderRadius: radius.lg,
            backgroundColor: palette().soft,
          }}
        >
          <Hint text={ui.t("规则预览", "Rule preview")} />
          <Code text={preview()} />
        </View>
        <If when={conflict()}>
          <Hint
            text={
              conflict()!.line.split(",")[2]?.trim().toUpperCase() === target().toUpperCase()
                ? ui.t(
                    "已有相同匹配和目标，将保留或重新启用现有规则。",
                    "An equivalent rule already exists; it will be retained or re-enabled.",
                  )
                : ui.t(
                    "已有相同匹配规则，保存会更新该规则的目标。",
                    "A rule already matches this value. Saving updates its target.",
                  ) +
                  " " +
                  conflict()!.line
            }
          />
        </If>
        <If when={ui.client.state()!.mode !== "rule"}>
          <Hint
            text={ui.t(
              "当前不是规则模式，保存后切换到规则模式才会按此规则路由。",
              "Switch to Rule mode for this rule to affect routing.",
            )}
          />
        </If>
        <If when={props.context?.connectionId}>
          <Toggle
            label={ui.t("保存后断开此连接", "Disconnect this connection after saving")}
            value={disconnect()}
            onChange={(next) => disconnect.set(next)}
            disabled={busy()}
          />
        </If>
        <If when={baseline !== ui.client.state()!.configRevision}>
          <Hint
            danger
            text={ui.t(
              "配置已变更，请关闭并重新打开规则编辑器。",
              "Configuration changed. Close and reopen the rule editor.",
            )}
          />
        </If>
        <If when={ui.client.error()}>
          <PageError id="manual-rule-save-error" text={ui.client.error()} />
        </If>
        <Action
          id="save-manual-rule"
          label={busy() ? ui.t("正在保存…", "Saving…") : ui.t("保存规则", "Save rule")}
          disabled={
            busy() ||
            !ui.client.domains.fresh(["config", "rules", "summary"]) ||
            !value().trim() ||
            !ui.client.state()!.activeProfile ||
            baseline !== ui.client.state()!.configRevision
          }
          onPress={() => void save()}
        />
      </If>
    </Dialog>
  );
}

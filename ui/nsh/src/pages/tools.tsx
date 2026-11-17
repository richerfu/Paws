import { density, surfaceFinish, typography } from "../theme";
import { platform, Scroll, signal, Text, View } from "nsh";
import {
  Action,
  Actions,
  Body,
  Card,
  confirm,
  copyText,
  Detail,
  Field,
  ExternalLink,
  Hint,
  Icon,
  ListRow,
  ListSurface,
  palette,
  radius,
  SectionHeader,
  Select,
  Separator,
  spacing,
  Toggle,
  type Page,
  useApp,
  useLayout,
  useLifetime,
  useDraftGuard,
} from "../components";

import { ABOUT_LINKS } from "../about";
import { COPY } from "../product-copy";
import { PageError } from "../page-error";
import type { IconName } from "../icons";
import { connectionState } from "../connection-state";

type Tool = {
  page: Page;
  zh: string;
  en: string;
  descriptionZh?: string;
  descriptionEn?: string;
  icon: IconName;
};
const sections: { zh: string; en: string; rows: Tool[] }[] = [
  {
    zh: "偏好",
    en: "Preferences",
    rows: [
      {
        page: "settings",
        zh: "网络设置",
        en: "Network settings",
        descriptionZh: "VPN、DNS、端口",
        descriptionEn: "VPN, DNS and ports",
        icon: "sliders-horizontal",
      },
      {
        page: "appearance",
        zh: COPY["page-tr-001"][0],
        en: COPY["page-tr-001"][1],
        descriptionZh: "语言与主题",
        descriptionEn: "Language and theme",
        icon: "palette",
      },
    ],
  },
  {
    zh: "网络",
    en: "Network",
    rows: [
      {
        page: "connections",
        zh: "连接与请求",
        en: "Connections & requests",
        descriptionZh: "活动连接与请求历史",
        descriptionEn: "Active connections and request history",
        icon: "list-filter",
      },
      {
        page: "resources",
        zh: "资源与规则",
        en: "Resources & rules",
        descriptionZh: "分流规则与外部规则集",
        descriptionEn: "Routing rules and external rule sets",
        icon: "layers",
      },
      {
        page: "logs",
        zh: "日志",
        en: "Logs",
        descriptionZh: "实时日志与按天归档",
        descriptionEn: "Live logs and daily archives",
        icon: "file-text",
      },
    ],
  },
  {
    zh: "工具",
    en: "Tools",
    rows: [
      {
        page: "diagnostics",
        zh: COPY["diagnostics-title"][0],
        en: COPY["diagnostics-title"][1],
        descriptionZh: "连接状态与探测",
        descriptionEn: "Connection status and probes",
        icon: "shield-check",
      },
      {
        page: "backups",
        zh: "备份与恢复",
        en: "Backup & restore",
        icon: "download",
      },
      {
        page: "converter",
        zh: COPY["page-tr-002"][0],
        en: COPY["page-tr-002"][1],
        icon: "refresh-cw",
      },
      {
        page: "about",
        zh: "关于",
        en: "About",
        icon: "info",
      },
    ],
  },
];
export function Tools() {
  const ui = useApp();
  return (
    <Scroll
      bounces={false}
      keyboardDismissMode="on-drag"
      keyboardShouldPersistTaps="handled"
      style={{ flex: 1 }}
    >
      <Body>
        <Each of={sections}>
          {(section) => (
            <View style={{ gap: spacing.sm }}>
              <SectionHeader title={ui.t(section().zh, section().en)} />
              <ListSurface>
                <Each of={section().rows}>
                  {(tool, index) => (
                    <ListRow
                      id={`tool-${tool().page}`}
                      icon={tool().icon}
                      title={ui.t(tool().zh, tool().en)}
                      subtitle={
                        tool().descriptionZh
                          ? ui.t(tool().descriptionZh!, tool().descriptionEn!)
                          : undefined
                      }
                      divider={index() + 1 < section().rows.length}
                      chevron
                      onPress={() => ui.navigate(tool().page)}
                    />
                  )}
                </Each>
              </ListSurface>
            </View>
          )}
        </Each>
      </Body>
    </Scroll>
  );
}

type Health = {
  session: string;
  engine: string;
  dns: string;
  nodeTcp: string;
  https: string;
  elapsedMs: number;
};
type Watch = { enabled: boolean; group: string; url: string };
export function Diagnostics() {
  const ui = useApp(),
    layout = useLayout(),
    current = () => ui.client.state()!,
    alive = useLifetime();
  const prefilled = ui.viewState?.get("diagnostics.prefillUrl");
  if (typeof prefilled === "string") {
    ui.viewState?.delete("diagnostics.prefillUrl");
  }
  const group = signal(
      ui.client.domains.data("summary")!.groups[0]?.name || "",
    ),
    url = signal(
      typeof prefilled === "string" && prefilled
        ? prefilled
        : "https://www.gstatic.com/generate_204",
    ),
    watch = signal<Watch>({ enabled: false, group: "", url: "" }),
    health = signal<Health | null>(null),
    result = signal("");
  const resultSession = signal(""), resultTime = signal("");
  const busy = () => ["diagnostics.probe", "diagnostics.watchSet", "diagnostics.watchGet"].some((command) => ui.client.isPending(command));
  let changed = typeof prefilled === "string" && prefilled.length > 0;
  const loadWatch = async () => {
    const value = await ui.client.run<Watch>("diagnostics.watchGet");
    if (alive() && value) {
      watch.set(value);
      if (!changed) {
        if (value.group) group.set(value.group);
        if (value.url) url.set(value.url);
      }
    }
  };
  const loadHealth = async (notify = false) => {
    if (ui.client.isPending("diagnostics.healthGet")) return;
    const value = await ui.client.run<Health | null>(
      "diagnostics.healthGet",
      {},
      notify ? ui.t("检测状态已刷新", "Checks refreshed") : "",
    );
    if (alive() && value !== undefined) health.set(value);
  };
  void loadWatch();
  void loadHealth();
  const options = () =>
    ui.client.domains
      .data("summary")!
      .groups.map((group) => ({ value: group.name, label: group.name }));
  const selector = () =>
    ui.client.domains
      .data("summary")!
      .groups.some(
        (entry) =>
          entry.name === group() &&
          entry.groupType.toLowerCase() === "selector",
      );
  const validUrl = () => /^https:\/\/[^/?#@\s]+(?:\/[^?#\s]*)?$/.test(url());
  const saveWatch = async (enabled: boolean) => {
    const value = await ui.client.run<Watch>(
      "diagnostics.watchSet",
      enabled
        ? { enabled, group: group(), url: url() }
        : { enabled: false, group: "", url: "" },
      ui.t("自动恢复策略已保存", "Recovery policy saved"),
    );
    if (alive() && value) watch.set(value);
  };
  const probe = async () => {
    if (busy()) return;
    const submittedGroup = group(), submittedUrl = url(), session = current().vpnSessionId;
    const value = await ui.client.run<{ delay: number; sessionId: string }>(
      "diagnostics.probe",
      {
        group: submittedGroup,
        url: submittedUrl,
      },
    );
    if (alive() && value && session === current().vpnSessionId) {
      result.set(`${value.delay} ms · ${submittedGroup} · ${submittedUrl}`);
      resultSession.set(value.sessionId);
      resultTime.set(new Date().toISOString());
    }
  };
  const checkTone = (state: string) =>
    state === "passed"
      ? ("success" as const)
      : state === "failed"
        ? ("danger" as const)
        : state === "notApplicable"
          ? ("neutral" as const)
          : ("warning" as const);
  const checkValue = (state: string) =>
    state === "passed"
      ? ui.t("通过", "Passed")
      : state === "failed"
        ? ui.t("失败", "Failed")
        : state === "notApplicable"
          ? ui.t("不适用", "Not applicable")
          : ui.t("未测试", "Not tested");
  const currentHealth = () =>
    current().vpnRunning &&
    health()?.session === current().vpnSessionId &&
    health()?.engine === current().activeEngine
      ? health()
      : null;
  const recoveryCard = (
    <Card
      title={ui.t("自动恢复", "Automatic recovery")}
      subtitle={ui.t(
        "持续检测目标服务，在符合策略时尝试恢复连接。",
        "Watch the target service and recover according to the saved policy.",
      )}
    >
      <Toggle
        label={ui.t("启用自动恢复", "Enable recovery")}
        description={ui.t(
          "仅支持手动 Selector 策略组。",
          "Available for manual Selector groups.",
        )}
        value={watch().enabled}
        disabled={
          busy() || (!watch().enabled && (!selector() || !validUrl()))
        }
        onChange={(enabled) => void saveWatch(enabled)}
      />
      <If when={watch().enabled}>
        <Hint text={`${watch().group} · ${watch().url}`} />
        <Action
          secondary
          label={ui.t("保存当前探测目标", "Save current target")}
          disabled={
            busy() ||
            !selector() ||
            !validUrl() ||
            (watch().group === group() && watch().url === url())
          }
          onPress={() => void saveWatch(true)}
        />
      </If>
      <If when={!selector()}>
        <Hint
          text={ui.t(
            "请选择手动 Selector 分组以启用自动恢复。",
            "Select a manual Selector group to enable recovery.",
          )}
        />
      </If>
    </Card>
  );
  return (
    <Scroll
      bounces={false}
      keyboardDismissMode="on-drag"
      keyboardShouldPersistTaps="handled"
      style={{ flex: 1 }}
    >
      <Body>
        <Actions>
          <Action
            id="diagnostics-advanced"
            secondary
            small
            icon="stethoscope"
            label={ui.t("高级诊断", "Advanced diagnostics")}
            onPress={() => ui.navigate("advanced-diagnostics")}
          />
        </Actions>
        <View
          style={{
            flexDirection: layout().overview ? "row" : "column",
            gap: layout().overview ? layout().gap : spacing.section,
            alignItems: "flex-start",
          }}
        >
          <View
            style={{
              flex: layout().overview ? 1 : undefined,
              width: layout().overview ? undefined : "100%",
              minWidth: 0,
              gap: spacing.section,
            }}
          >
            <Card
              title={ui.t("连接状态", "Connection status")}
            >
              <Detail
                tone={
                  current().vpnRunning
                    ? current().networkProtected && !current().networkProtectError
                      ? "success"
                      : "warning"
                    : "neutral"
                }
                label="VPN"
                value={
                  connectionState(current(), ui.client.vpnOperation()) === "protected"
                    ? ui.t("已连接", "Connected")
                    : current().vpnRunning
                      ? ui.t("保护未确认", "Protection unconfirmed")
                      : ui.t("未连接", "Disconnected")
                }
              />
              <If when={current().vpnRunning || current().vpnLifecycle === "starting"}>
                <Action
                  id="diagnostics-stop"
                  stop
                  icon="stop"
                  label={ui.t("停止连接", "Stop connection")}
                  loading={ui.client.vpnOperation() === "stop"}
                  onPress={() => void ui.client.run("vpn.stop")}
                />
              </If>
              <Detail
                tone="neutral"
                label={ui.t("运行内核", "Active engine")}
                value={current().activeEngine || "—"}
              />
              <Detail
                label="DNS"
                value={
                  current().dns.statisticsAvailable
                    ? `${current().dns.handledPackets} / ${current().dns.cacheHits} / ${current().dns.cacheMisses}`
                    : ui.t(
                        "当前内核未提供统计",
                        "Statistics unavailable from the engine",
                      )
                }
              />
              <Separator />
              <Detail
                tone={checkTone(current().engineLoaded ? "passed" : "notTested")}
                label={ui.t("配置载入", "Configuration")}
                value={checkValue(current().engineLoaded ? "passed" : "notTested")}
              />
              <Detail
                tone={checkTone(
                  current().networkProtectError
                    ? "failed"
                    : current().networkProtected
                      ? "passed"
                      : "notTested",
                )}
                label={ui.t("出站保护", "Outbound protection")}
                value={checkValue(
                  current().networkProtectError
                    ? "failed"
                    : current().networkProtected
                      ? "passed"
                      : "notTested",
                )}
              />
              <Detail
                tone={checkTone(currentHealth()?.dns || "notTested")}
                label="DNS"
                value={checkValue(currentHealth()?.dns || "notTested")}
              />
              <Detail
                tone={checkTone(currentHealth()?.nodeTcp || "notTested")}
                label={ui.t("节点 TCP", "Node TCP")}
                value={checkValue(currentHealth()?.nodeTcp || "notTested")}
              />
              <Detail
                tone={checkTone(currentHealth()?.https || "notTested")}
                label="HTTPS"
                value={checkValue(currentHealth()?.https || "notTested")}
              />
              <Action
                id="refresh-checks"
                secondary
                icon="refresh-cw"
                loading={ui.client.isPending("diagnostics.healthGet")}
                label={ui.t("刷新检测状态", "Refresh checks")}
                disabled={busy()}
                onPress={() => void loadHealth(true)}
              />
              <If when={current().networkProtectError}>
                <PageError
                  id="diagnostics-protection-error"
                  text={current().networkProtectError || ""}
                />
              </If>
            </Card>
            <If when={layout().overview}>{recoveryCard}</If>
          </View>
          <View
            style={{
              flex: layout().overview ? 1 : undefined,
              width: layout().overview ? undefined : "100%",
              minWidth: 0,
              gap: spacing.section,
            }}
          >
            <Card
              title={ui.t("目标服务探测", "Target service probe")}
              subtitle={ui.t(
                "通过指定代理组发送 HTTPS 探测，不改变分组选择。",
                "Probe an HTTPS target through a group without changing its selection.",
              )}
            >
              <Select
                label={ui.t("策略组", "Proxy group")}
                value={group()}
                options={options()}
                disabled={busy()}
                onChange={(value) => {
                  changed = true;
                  group.set(value);
                }}
              />
              <Select
                label={ui.t("预设目标", "Target preset")}
                value={
                  [
                    "https://www.gstatic.com/generate_204",
                    "https://cp.cloudflare.com/generate_204",
                    "https://github.com/robots.txt",
                  ].includes(url())
                    ? url()
                    : "custom"
                }
                options={[
                  {
                    value: "https://www.gstatic.com/generate_204",
                    label: "Gstatic 204",
                  },
                  {
                    value: "https://cp.cloudflare.com/generate_204",
                    label: "Cloudflare",
                  },
                  { value: "https://github.com/robots.txt", label: "GitHub" },
                  { value: "custom", label: ui.t("自定义", "Custom") },
                ]}
                disabled={busy()}
                onChange={(value) => {
                  if (value !== "custom") {
                    changed = true;
                    url.set(value);
                  }
                }}
              />
              <Field
                label={ui.t("探测地址", "Probe URL")}
                value={url()}
                keyboardType="url"
                disabled={busy()}
                onChange={(value) => {
                  changed = true;
                  url.set(value);
                }}
              />
              <If when={!validUrl()}>
                <Hint
                  danger
                  text={ui.t(
                    "请输入不含凭据、查询参数或片段的 HTTPS 地址。",
                    "Enter an HTTPS URL without credentials, query or fragment.",
                  )}
                />
              </If>
              <Action
                primary
                id="diagnostics-probe"
                loading={ui.client.isPending("diagnostics.probe")}
                label={
                  current().vpnRunning
                    ? ui.t("开始探测", "Run probe")
                    : ui.t("请先连接 VPN", "Connect VPN first")
                }
                disabled={
                  busy() || !current().vpnRunning || !group() || !validUrl()
                }
                onPress={() => void probe()}
              />
              <If when={result()}>
                <Detail
                  tone={
                    resultSession() === current().vpnSessionId &&
                    current().vpnRunning
                      ? "success"
                      : "neutral"
                  }
                  label={ui.t("最近探测", "Last probe")}
                  value={`${result()}\n${resultTime()}\n${resultSession() === current().vpnSessionId && current().vpnRunning ? ui.t("当前会话", "Current session") : ui.t("历史会话结果", "Previous session result")}`}
                />
              </If>
            </Card>
            <If when={!layout().overview}>{recoveryCard}</If>
          </View>
        </View>
      </Body>
    </Scroll>
  );
}

export function AdvancedDiagnostics() {
  const ui = useApp(),
    alive = useLifetime(),
    group = signal(ui.client.domains.data("summary")!.groups[0]?.name || ""),
    domains = signal(""),
    report = signal("");
  const busy = () => ui.client.pending() > 0;
  useDraftGuard(() => !!domains().trim(), () => domains.set(""), () => ui.client.isPending("diagnostics.serviceDns"));
  const generate = async () => {
    if (busy()) return;
    const value = await ui.client.run<string>("diagnostics.report");
    if (alive() && value !== undefined) report.set(value);
  };
  return (
    <Scroll
      bounces={false}
      keyboardDismissMode="on-drag"
      keyboardShouldPersistTaps="handled"
      style={{ flex: 1 }}
    >
      <Body>
        <Card
          title={ui.t("域名 DoH 策略", "Domain DoH policy")}
          subtitle={ui.t(
            "通过所选策略组的 Cloudflare DoH 解析指定域名及其子域名。",
            "Resolve domains and subdomains with Cloudflare DoH through the selected group.",
          )}
        >
          <Select
            label={ui.t("策略组", "Proxy group")}
            value={group()}
            options={ui.client.domains.data("summary")!.groups.map((entry) => ({
              value: entry.name,
              label: entry.name,
            }))}
            onChange={(value) => group.set(value)}
            disabled={busy()}
          />
          <Field
            label={ui.t("域名列表", "Domains")}
            value={domains()}
            placeholder="example.com, example.net"
            disabled={busy()}
            onChange={(value) => domains.set(value)}
          />
          <Hint
            text={ui.t(
              "用逗号分隔多个域名。保留其他 DNS 设置，并重新加载当前 VPN 配置。",
              "Separate domains with commas. Other DNS settings are preserved; the active VPN is reloaded.",
            )}
          />
          <Action
            primary
            loading={ui.client.isPending("diagnostics.serviceDns")}
            label={ui.t("应用 DoH 策略", "Apply DoH policy")}
            disabled={busy() || !group() || !domains().trim()}
            onPress={() => {
              const submitted = domains();
              void ui.client.run(
                "diagnostics.serviceDns",
                {
                  domains: domains(),
                  group: group(),
                  expectedRevision: ui.client.state()!.configRevision,
                },
                ui.t("域名 DoH 策略已应用", "Domain DoH policy applied"),
              ).then((value) => { if (alive() && value !== undefined && domains() === submitted) domains.set(""); });
            }}
          />
        </Card>
        <Card
          title={ui.t("脱敏诊断报告", "Redacted diagnostic report")}
          subtitle={ui.t(
            "导出前请检查报告内容。报告不包含订阅密钥。",
            "Review the report before exporting. Subscription secrets are excluded.",
          )}
        >
          <Action
            id="generate-report"
            secondary
            loading={ui.client.isPending("diagnostics.report")}
            label={ui.t("生成报告", "Generate report")}
            disabled={busy()}
            onPress={() => void generate()}
          />
          <If when={report()}>
            <Field
              label={ui.t("报告预览", "Report preview")}
              value={report()}
              onChange={() => {}}
              multiline
              readonly
            />
            <Actions>
              <Action
                secondary
                icon="file-text"
                label={ui.t("复制报告", "Copy report")}
                onPress={() =>
                  copyText(
                    ui,
                    report(),
                    ui.t("已复制诊断报告", "Diagnostic report copied"),
                  )
                }
              />
              <Action
                secondary
                icon="download"
                label={ui.t("导出报告", "Export report")}
                disabled={busy()}
                onPress={() =>
                  void ui.client.run("platform.export", {
                    name: "Paws-diagnostics.json",
                    content: report(),
                  })
                }
              />
            </Actions>
          </If>
        </Card>
      </Body>
    </Scroll>
  );
}

export function Backups() {
  const ui = useApp(),
    layout = useLayout(),
    url = signal(""),
    username = signal(""),
    password = signal("");
  useDraftGuard(() => !!(url() || username() || password()), () => { url.set(""); username.set(""); password.set(""); }, () => ui.client.isPending("backups.webdavRestore") || ui.client.isPending("backups.webdavUpload"));
  const validUrl = () => /^https:\/\/[^/?#@\s]+(?:\/[^?#\s]*)?$/.test(url());
  const alive = useLifetime();
  const exportBackup = async () => {
    const content = await ui.client.run<string>("backups.export");
    if (alive() && content !== undefined)
      await ui.client.run("platform.export", {
        name: "Paws-backup.json",
        content,
      });
  };
  const restore = async () => {
    const file = await ui.client.run<{ name: string; content: string }>(
      "platform.importFile",
    );
    if (alive() && file)
      void confirm(
        ui,
        ui.t(
          `从“${file.name}”恢复配置？`,
          `Restore profiles from “${file.name}”?`,
        ),
        () =>
          void ui.client.run(
            "backups.restore",
            { content: file.content },
            ui.t("备份已恢复", "Backup restored"),
          ),
        {
          actionLabel: ui.t("恢复配置副本", "Restore profile copies"),
          destructive: false,
          description: ui.t("来源：本地文件。备份格式未记录生成时间。将新建配置副本，保留现有订阅和当前连接。", "Source: local file. The backup format has no creation timestamp. Creates profile copies and keeps existing subscriptions and the current connection."),
        },
      );
  };
  const webdav = (command: string) =>
    void ui.client.run(
      command,
      { url: url(), username: username(), password: password() },
      ui.t("操作已完成", "Operation completed"),
    );
  const twoColumns = () => platform.desktop && layout().overview;
  return (
    <Scroll
      bounces={false}
      keyboardDismissMode="on-drag"
      keyboardShouldPersistTaps="handled"
      style={{ flex: 1 }}
    >
      <Body>
        <View
          style={{
            flexDirection: twoColumns() ? "row" : "column",
            gap: twoColumns() ? layout().gap : spacing.section,
            alignItems: "flex-start",
          }}
        >
          <View
            style={{
              flex: twoColumns() ? 1 : undefined,
              width: twoColumns() ? undefined : "100%",
              minWidth: 0,
            }}
          >
            <Card
              title={ui.t("本地备份", "Local backup")}
              subtitle={ui.t(
                "导出完整配置，或从备份文件恢复配置副本。",
                "Export the full configuration, or restore profile copies from a backup file.",
              )}
            >
              <Hint text={ui.t("导出配置", "Export configuration")} />
              <Hint
                text={ui.t(
                  "备份包含订阅地址、覆写与节点密钥，请妥善保管。",
                  "Backups include subscription URLs, overrides and node secrets. Store them securely.",
                )}
              />
              <Action
                secondary
                icon="download"
                loading={ui.client.isPending("backups.export")}
                label={ui.t("导出备份文件…", "Export backup file…")}
                disabled={ui.client.pending() > 0}
                onPress={() => void exportBackup()}
              />
              <Separator />
              <Hint text={ui.t("从文件恢复", "Restore from file")} />
              <Hint
                text={ui.t(
                  "恢复时新建配置副本，保留当前配置与连接状态。",
                  "Restoring creates profile copies and preserves the active profile and connection.",
                )}
              />
              <Action
                secondary
                icon="file-up"
                loading={ui.client.isPending("backups.restore")}
                label={ui.t("选择备份文件…", "Choose a backup file…")}
                disabled={ui.client.pending() > 0}
                onPress={() => void restore()}
              />
            </Card>
          </View>
          <View
            style={{
              flex: twoColumns() ? 1 : undefined,
              width: twoColumns() ? undefined : "100%",
              minWidth: 0,
            }}
          >
            <Card
              title={ui.t("WebDAV 备份", "WebDAV backup")}
              subtitle={ui.t(
                "账号与密码仅保存在当前页面内存中。点击上传或下载时才会连接服务器。",
                "Credentials remain in this page’s memory. The server is contacted only when uploading or downloading.",
              )}
            >
              <Field
                label={ui.t("备份文件地址", "Backup file URL")}
                value={url()}
                keyboardType="url"
                onChange={(text) => url.set(text)}
                placeholder="https://example.com/Paws-backup.json"
              />
              <Field
                label={ui.t("用户名", "Username")}
                value={username()}
                onChange={(text) => username.set(text)}
              />
              <Hint
                text={ui.t(
                  "填写完整的 HTTPS 文件地址。上传会创建新的备份，下载会恢复配置副本。",
                  "Enter the complete HTTPS file URL. Upload creates a new backup; download restores profile copies.",
                )}
              />
              <If when={url() && !validUrl()}>
                <Hint
                  danger
                  text={ui.t(
                    "请输入不含凭据、查询参数或片段的 HTTPS 地址。",
                    "Enter an HTTPS URL without credentials, query or fragment.",
                  )}
                />
              </If>
              <Field
                label={ui.t("密码或应用专用密码", "Password or app password")}
                value={password()}
                onChange={(text) => password.set(text)}
                password
              />
              <Actions>
                <Action
                  primary
                  loading={ui.client.isPending("backups.webdavUpload")}
                  label={ui.t("上传备份", "Upload backup")}
                  disabled={!validUrl() || ui.client.pending() > 0}
                  onPress={() => webdav("backups.webdavUpload")}
                />
                <Action
                  secondary
                  loading={ui.client.isPending("backups.webdavRestore")}
                  label={ui.t("下载并恢复…", "Download and restore…")}
                  disabled={!validUrl() || ui.client.pending() > 0}
                  onPress={() =>
                    void confirm(
                      ui,
                      ui.t(
                        "下载远程备份并恢复？",
                        "Download and restore the remote backup?",
                      ),
                      () => webdav("backups.webdavRestore"),
                      {
                        actionLabel: ui.t("下载并恢复", "Download and restore"),
                        description: `${url()}\n${ui.t("将新建配置副本，保留现有订阅与连接。服务器未在恢复前提供备份时间或内容预览。", "Creates profile copies and keeps existing subscriptions and connections. The server provides no timestamp or content preview before restore.")}`,
                        destructive: false,
                      },
                    )
                  }
                />
              </Actions>
            </Card>
          </View>
        </View>
      </Body>
    </Scroll>
  );
}

export function About() {
  const ui = useApp(),
    about = () => ui.client.state()!.about;
  const renderComponent = (
    name: string,
    version: string | undefined,
    license: string,
    index: number,
  ) => (
    <View
      testID={`about-component-${index}`}
      style={{ minWidth: 0, gap: density.space(8) }}
    >
      <Text
        testID={`about-name-${index}`}
        style={{
          minWidth: 0,
          fontSize: typography.sm,
          lineHeight: density.line(20),
          fontWeight: "500",
          color: palette().text,
        }}
      >
        {name}
      </Text>
      <If when={version !== undefined}>
        <Text
          testID={`about-version-${index}`}
          style={{
            userSelect: "text",
            minWidth: 0,
            maxWidth: "100%",
            fontSize: density.font(13),
            lineHeight: density.line(20),
            color: palette().muted,
          }}
        >
          {version || ui.t("未知版本", "Unknown version")}
        </Text>
      </If>
      <View testID={`about-actions-${index}`}>
        <Actions>
          <ExternalLink
            id={`about-source-${index}`}
            small
            label={ui.t("源代码", "Source code")}
            href={ABOUT_LINKS[index].url}
          />
          <ExternalLink
            id={`about-license-${index}`}
            small
            label={license}
            href={ABOUT_LINKS[index + 1].url}
          />
        </Actions>
      </View>
    </View>
  );
  return (
    <Scroll testID="about-scroll" bounces={false} style={{ flex: 1 }}>
      <Body>
        <View
          style={{
            alignItems: "center",
            gap: density.space(8),
            paddingTop: density.space(8),
            paddingBottom: density.space(16),
          }}
        >
          <View
            style={{
              width: 56,
              height: 56,
              borderWidth: 1,
              borderRadius: radius.xxl,
              borderCurve: "continuous",
              alignItems: "center",
              justifyContent: "center",
              ...surfaceFinish("raised"),
            }}
          >
            <Icon name="paw-print" size={26} color={palette().accent} />
          </View>
          <Text
            style={{
              fontSize: density.font(20),
              fontWeight: "600",
              color: palette().text,
            }}
          >
            Paws
          </Text>
          <Hint
            text={ui.t(
              "HarmonyOS 原生多内核代理客户端",
              "A native multi-engine proxy client for HarmonyOS",
            )}
          />
        </View>
        <View testID="about-version-section">
          <Card title={ui.t("版本", "Version")}>
            <Detail
              stacked
              label={ui.t("应用版本", "App version")}
              value={about().appVersion}
            />
            <Actions align="start">
              <Action
                secondary
                small
                icon="copy"
                label={ui.t("复制版本信息", "Copy version info")}
                onPress={() =>
                  copyText(
                    ui,
                    [
                      `Paws ${about().appVersion}`,
                      `Core ${about().coreVersion}`,
                      `UI ${about().uiRuntime} ${about().uiRuntimeVersion}`,
                      `Rust ${about().rustVersion}`,
                    ].join("\n"),
                    ui.t("版本信息已复制", "Version info copied"),
                  )
                }
              />
            </Actions>
          </Card>
        </View>
        <View testID="about-engine-section">
          <Card
            title={ui.t("代理内核", "Proxy engines")}
          >
            {renderComponent("Mihomo", about().mihomoVersion, "GPL-3.0", 0)}
            <Separator />
            {renderComponent(
              "sing-box",
              about().singBoxVersion,
              "GPL-3.0-or-later",
              2,
            )}
            <Separator />
            {renderComponent("meow-rs", about().meowRsVersion, "MIT", 4)}
          </Card>
        </View>
        <View testID="about-ack-section">
          <Card
            title={ui.t("开源致谢", "Open source acknowledgements")}
            subtitle={ui.t(
              "感谢以下开源项目。各组件遵循各自的开源许可证。",
              "Thanks to these projects. Each component is distributed under its own license.",
            )}
          >
            {renderComponent(
              "Native Shell",
              undefined,
              ui.t("许可证", "License"),
              6,
            )}
            <Separator />
            <View
              testID="about-component-flclash"
              style={{ minWidth: 0, gap: density.space(8) }}
            >
              <Text
                style={{
                  fontSize: typography.sm,
                  lineHeight: density.line(20),
                  fontWeight: "500",
                  color: palette().text,
                }}
              >
                FlClash
              </Text>
              <Hint
                text={ui.t(
                  "Mihomo 的 HarmonyOS 桥接源自 FlClash，使用 GPL-3.0 许可证。",
                  "The HarmonyOS bridge for Mihomo originates from FlClash and is licensed under GPL-3.0.",
                )}
              />
              <Actions>
                <ExternalLink
                  id="about-source-8"
                  small
                  label={ui.t("源代码", "Source code")}
                  href={ABOUT_LINKS[8].url}
                />
                <ExternalLink
                  id="about-adaptations"
                  small
                  label={ui.t(
                    "第三方组件与适配说明",
                    "Third-party components and adaptations",
                  )}
                  href={ABOUT_LINKS[9].url}
                />
              </Actions>
            </View>
          </Card>
        </View>
        <View testID="about-privacy-section">
          <Card
            title={ui.t("隐私与数据", "Privacy and data")}
            subtitle={ui.t(
              "隐私策略、出口 IP 查询和第三方服务说明",
              "Privacy policy, exit IP lookup and third-party services",
            )}
          >
            <Action
              id="about-privacy"
              secondary
              icon="shield-check"
              label={ui.t("隐私说明", "Privacy notice")}
              onPress={() => ui.navigate("privacy")}
            />
          </Card>
        </View>
      </Body>
    </Scroll>
  );
}
export function Privacy() {
  const ui = useApp(),
    about = () => ui.client.state()!.about;
  return (
    <Scroll
      bounces={false}
      keyboardDismissMode="on-drag"
      keyboardShouldPersistTaps="handled"
      style={{ flex: 1 }}
    >
      <Body>
        <Card title={ui.t("隐私与数据处理", "Privacy & data handling")}>
          <Each of={about().privacySummary}>
            {(line) => (
              <View
                style={{
                  flexDirection: "row",
                  alignItems: "flex-start",
                  gap: density.space(8),
                }}
              >
                <Text
                  style={{
                    userSelect: "text",
                    flex: 1,
                    fontSize: density.font(14),
                    lineHeight: density.line(24),
                    color: palette().text,
                  }}
                >
                  {line()}
                </Text>
              </View>
            )}
          </Each>
        </Card>
        <Card
          title={ui.t("出口 IP 服务", "Exit IP services")}
          subtitle={ui.t(
            "连接后使用以下服务查询出口 IP，点击查看服务文档。",
            "These services determine the exit IP after connecting. Open their documentation for details.",
          )}
        >
          <Each of={about().exitIpServices}>
            {(service) => (
              <ExternalLink
                label={service().name}
                href={service().documentationUrl}
              />
            )}
          </Each>
        </Card>
      </Body>
    </Scroll>
  );
}

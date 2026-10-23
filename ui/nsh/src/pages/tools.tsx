import { Scroll, signal, Text, View } from "nsh";
import { Clipboard } from "nsh/clipboard";
import {
  Action,
  Actions,
  Badge,
  Body,
  Card,
  Code,
  confirm,
  Detail,
  Field,
  Hint,
  Icon,
  palette,
  radius,
  Select,
  Separator,
  Toggle,
  type Page,
  useApp,
  useLifetime,
} from "../components";
import { bytes } from "../locale";
import { renderObject } from "./resources";
import { ABOUT_LINKS } from "../about";
import { COPY } from "../product-copy";
import { PageError } from "../page-error";
import type { IconName } from "../icons";

type Tool = {
  page: Page;
  zh: string;
  en: string;
  descriptionZh: string;
  descriptionEn: string;
  icon: IconName;
};
const sections: { zh: string; en: string; rows: Tool[] }[] = [
  {
    zh: "界面",
    en: "Interface",
    rows: [
      {
        page: "appearance",
        zh: COPY["page-tr-001"][0],
        en: COPY["page-tr-001"][1],
        descriptionZh: "语言、浅色与深色主题",
        descriptionEn: "Language, light and dark themes",
        icon: "palette",
      },
    ],
  },
  {
    zh: "网络",
    en: "Network",
    rows: [
      {
        page: "settings",
        zh: "网络设置",
        en: "Network settings",
        descriptionZh: "VPN、DNS 与网络栈",
        descriptionEn: "VPN, DNS and network stack",
        icon: "settings",
      },
    ],
  },
  {
    zh: "工具",
    en: "Tools",
    rows: [
      {
        page: "converter",
        zh: COPY["page-tr-002"][0],
        en: COPY["page-tr-002"][1],
        descriptionZh: COPY["hard-zh-023"][0],
        descriptionEn: COPY["hard-zh-023"][1],
        icon: "refresh-cw",
      },
      {
        page: "backups",
        zh: "备份与恢复",
        en: "Backup & restore",
        descriptionZh: COPY["backup-subtitle"][0],
        descriptionEn: COPY["backup-subtitle"][1],
        icon: "archive",
      },
    ],
  },
  {
    zh: "诊断与管理",
    en: "Diagnostics & management",
    rows: [
      {
        page: "diagnostics",
        zh: COPY["diagnostics-title"][0],
        en: COPY["diagnostics-title"][1],
        descriptionZh: "连接状态、HTTPS 探测与自动恢复",
        descriptionEn: "Connection status, HTTPS probes and recovery",
        icon: "shield-check",
      },
      {
        page: "advanced-diagnostics",
        zh: "高级诊断",
        en: "Advanced diagnostics",
        descriptionZh: "域名 DoH 策略、脱敏报告与内核验收",
        descriptionEn: "Domain DoH policy, redacted reports and qualification",
        icon: "stethoscope",
      },
      {
        page: "requests",
        zh: "请求",
        en: "Requests",
        descriptionZh: "检查最近的请求与规则命中",
        descriptionEn: "Recent requests and matching rules",
        icon: "activity",
      },
      {
        page: "connections",
        zh: "连接",
        en: "Connections",
        descriptionZh: "查看或断开活动连接",
        descriptionEn: "Inspect or close active connections",
        icon: "unplug",
      },
      {
        page: "resources",
        zh: "资源",
        en: "Resources",
        descriptionZh: "规则、Provider 与 GeoData",
        descriptionEn: "Rules, providers and geodata",
        icon: "database",
      },
      {
        page: "logs",
        zh: "日志",
        en: "Logs",
        descriptionZh: COPY["page-tr-030"][0],
        descriptionEn: COPY["page-tr-030"][1],
        icon: "scroll-text",
      },
    ],
  },
  {
    zh: "关于",
    en: "About",
    rows: [
      {
        page: "about",
        zh: "关于",
        en: "About",
        descriptionZh: "开源信息、组件版本与隐私说明",
        descriptionEn: "Open source, component versions and privacy",
        icon: "paw-print",
      },
    ],
  },
];
export function Tools() {
  const ui = useApp();
  return (
    <Scroll bounces={false} style={{ flex: 1 }}>
      <Body>
        <Each of={sections}>
          {(section) => (
            <View style={{ gap: 8, marginBottom: 4 }}>
              <Text
                style={{ color: palette().muted, fontSize: 14, fontWeight: "600", marginLeft: 4 }}
              >
                {ui.t(section().zh, section().en)}
              </Text>
              <View
                style={{
                  borderWidth: 1,
                  borderColor: palette().border,
                  borderRadius: radius.lg,
                  overflow: "hidden",
                  boxShadow: "0px 1px 2px #0000000d",
                  backgroundColor: palette().panel,
                }}
              >
                <Each of={section().rows}>
                  {(tool, index) => (
                    <>
                      <View
                        class="hover:bg-[#f4f4f5] dark:hover:bg-[#27272a] focus-visible:ring-2 focus-visible:ring-[#71717a]"
                        testID={`tool-${tool().page}`}
                        accessibilityRole="button"
                        accessibilityLabel={ui.t(tool().zh, tool().en)}
                        onPress={() => ui.navigate(tool().page)}
                        activeOpacity={0.65}
                        style={{
                          minHeight: 64,
                          padding: 12,
                          flexDirection: "row",
                          alignItems: "center",
                          gap: 12,
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
                          <Icon name={tool().icon} size={17} color={palette().text} />
                        </View>
                        <View style={{ flex: 1, minWidth: 0, gap: 4 }}>
                          <Text
                            style={{
                              fontSize: 14,
                              lineHeight: 20,
                              fontWeight: "600",
                              color: palette().text,
                            }}
                          >
                            {ui.t(tool().zh, tool().en)}
                          </Text>
                          <Text
                            numberOfLines={1}
                            style={{ fontSize: 12, lineHeight: 18, color: palette().muted }}
                          >
                            {ui.t(tool().descriptionZh, tool().descriptionEn)}
                          </Text>
                        </View>
                        <Icon name="chevron-right" size={16} color={palette().muted} />
                      </View>
                      <If when={index() + 1 < section().rows.length}>
                        <Separator />
                      </If>
                    </>
                  )}
                </Each>
              </View>
            </View>
          )}
        </Each>
        <Text style={{ textAlign: "center", marginTop: 24, fontSize: 12, color: palette().muted }}>
          Paws {ui.client.domains.data("summary")?.appVersion || ""}
        </Text>
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
    current = () => ui.client.state()!,
    alive = useLifetime();
  const group = signal(ui.client.domains.data("summary")!.groups[0]?.name || ""),
    url = signal("https://www.gstatic.com/generate_204"),
    watch = signal<Watch>({ enabled: false, group: "", url: "" }),
    health = signal<Health | null>(null),
    result = signal("");
  const busy = () => ui.client.pending() > 0;
  let changed = false;
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
        (entry) => entry.name === group() && entry.groupType.toLowerCase() === "selector",
      );
  const validUrl = () => /^https:\/\/[^/?#@\s]+(?:\/[^?#\s]*)?$/.test(url());
  const saveWatch = async (enabled: boolean) => {
    const value = await ui.client.run<Watch>(
      "diagnostics.watchSet",
      enabled ? { enabled, group: group(), url: url() } : { enabled: false, group: "", url: "" },
      ui.t("自动恢复策略已保存", "Recovery policy saved"),
    );
    if (alive() && value) watch.set(value);
  };
  const probe = async () => {
    if (busy()) return;
    const value = await ui.client.run<{ delay: number; sessionId: string }>("diagnostics.probe", {
      group: group(),
      url: url(),
    });
    if (alive() && value) result.set(`${value.delay} ms · ${group()}`);
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
  return (
    <Scroll bounces={false} style={{ flex: 1 }}>
      <Body>
        <Card
          title={ui.t("连接状态", "Connection status")}
          subtitle={ui.t(
            "检测真实 VPN 会话，不以配置已载入代替网络连通。",
            "Checks belong to the actual VPN session; loaded configuration does not prove connectivity.",
          )}
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
              current().vpnRunning ? ui.t("已连接", "Connected") : ui.t("未连接", "Disconnected")
            }
          />
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
                : ui.t("当前内核未提供统计", "Statistics unavailable from the engine")
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
            <PageError id="diagnostics-protection-error" text={current().networkProtectError || ""} />
          </If>
        </Card>
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
              { value: "https://www.gstatic.com/generate_204", label: "Gstatic 204" },
              { value: "https://cp.cloudflare.com/generate_204", label: "Cloudflare" },
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
            id="diagnostics-probe"
            loading={ui.client.isPending("diagnostics.probe")}
            label={
              current().vpnRunning
                ? ui.t("开始探测", "Run probe")
                : ui.t("请先连接 VPN", "Connect VPN first")
            }
            disabled={busy() || !current().vpnRunning || !group() || !validUrl()}
            onPress={() => void probe()}
          />
          <If when={result()}>
            <Hint text={result()} />
          </If>
        </Card>
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
            disabled={busy() || (!watch().enabled && (!selector() || !validUrl()))}
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
  const generate = async () => {
    if (busy()) return;
    const value = await ui.client.run<string>("diagnostics.report");
    if (alive() && value !== undefined) report.set(value);
  };
  const qualification = async () => {
    const file = await ui.client.run<{ name: string; content: string } | null>(
      "platform.importFile",
    );
    if (alive() && file)
      await ui.client.run(
        "diagnostics.importQualification",
        { content: file.content },
        ui.t("验收证据已导入", "Qualification evidence imported"),
      );
  };
  return (
    <Scroll bounces={false} style={{ flex: 1 }}>
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
            options={ui.client.domains
              .data("summary")!
              .groups.map((entry) => ({ value: entry.name, label: entry.name }))}
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
            label={ui.t("应用 DoH 策略", "Apply DoH policy")}
            disabled={busy() || !group() || !domains().trim()}
            onPress={() =>
              void ui.client.run(
                "diagnostics.serviceDns",
                {
                  domains: domains(),
                  group: group(),
                  expectedRevision: ui.client.state()!.configRevision,
                },
                ui.t("域名 DoH 策略已应用", "Domain DoH policy applied"),
              )
            }
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
              disabled
            />
            <Action
              secondary
              label={ui.t("导出报告", "Export report")}
              disabled={busy()}
              onPress={() =>
                void ui.client.run("platform.export", {
                  name: "Paws-diagnostics.json",
                  content: report(),
                })
              }
            />
          </If>
        </Card>
        <Card
          title={ui.t("内核验收证据", "Engine qualification evidence")}
          subtitle={ui.t(
            "导入真实设备验收记录，供引擎选择策略校验。",
            "Import actual device qualification records for engine selection checks.",
          )}
        >
          <Action
            secondary
            label={ui.t("导入验收证据", "Import qualification evidence")}
            disabled={busy()}
            onPress={() => void qualification()}
          />
        </Card>
      </Body>
    </Scroll>
  );
}

export function Backups() {
  const ui = useApp(),
    url = signal(""),
    username = signal(""),
    password = signal("");
  const validUrl = () => /^https:\/\/[^/?#@\s]+(?:\/[^?#\s]*)?$/.test(url());
  const alive = useLifetime();
  const exportBackup = async () => {
    const content = await ui.client.run<string>("backups.export");
    if (alive() && content !== undefined)
      await ui.client.run("platform.export", { name: "Paws-backup.json", content });
  };
  const restore = async () => {
    const file = await ui.client.run<{ name: string; content: string }>("platform.importFile");
    if (alive() && file)
      void confirm(
        ui,
        ui.t("从所选文件恢复配置？", "Restore profiles from the selected backup?"),
        () =>
          void ui.client.run(
            "backups.restore",
            { content: file.content },
            ui.t("备份已恢复", "Backup restored"),
          ),
      );
  };
  const webdav = (command: string) =>
    void ui.client.run(
      command,
      { url: url(), username: username(), password: password() },
      ui.t("操作已完成", "Operation completed"),
    );
  return (
    <Scroll bounces={false} style={{ flex: 1 }}>
      <Body>
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
            label={ui.t("导出备份文件", "Export backup file")}
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
            label={ui.t("选择备份文件", "Choose a backup file")}
            disabled={ui.client.pending() > 0}
            onPress={() => void restore()}
          />
        </Card>
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
              label={ui.t("上传备份", "Upload backup")}
              disabled={!validUrl() || ui.client.pending() > 0}
              onPress={() => webdav("backups.webdavUpload")}
            />
            <Action
              secondary
              label={ui.t("下载并恢复", "Download and restore")}
              disabled={!validUrl() || ui.client.pending() > 0}
              onPress={() =>
                void confirm(
                  ui,
                  ui.t("下载远程备份并恢复？", "Download and restore the remote backup?"),
                  () => webdav("backups.webdavRestore"),
                )
              }
            />
          </Actions>
        </Card>
      </Body>
    </Scroll>
  );
}

export function About() {
  const ui = useApp(),
    about = () => ui.client.state()!.about;
  const renderComponent = (name: string, version: string, license: string, index: number) => (
    <View testID={`about-component-${index}`} style={{ minWidth: 0, gap: 8 }}>
      <Text
        testID={`about-name-${index}`}
        style={{
          minWidth: 0,
          fontSize: 14,
          lineHeight: 20,
          fontWeight: "500",
          color: palette().text,
        }}
      >
        {name}
      </Text>
      <Text
        testID={`about-version-${index}`}
        selectable
        style={{
          minWidth: 0,
          maxWidth: "100%",
          fontSize: 13,
          lineHeight: 20,
          color: palette().muted,
        }}
      >
        {version || ui.t("未知版本", "Unknown version")}
      </Text>
      <View testID={`about-actions-${index}`}>
        <Actions>
          <Action
            id={`about-source-${index}`}
            variant="link"
            small
            label={ui.t("源代码", "Source code")}
            onPress={() => void ui.client.run("platform.openUrl", { url: ABOUT_LINKS[index].url })}
          />
          <Action
            id={`about-license-${index}`}
            variant="link"
            small
            label={license}
            onPress={() =>
              void ui.client.run("platform.openUrl", { url: ABOUT_LINKS[index + 1].url })
            }
          />
        </Actions>
      </View>
    </View>
  );
  return (
    <Scroll testID="about-scroll" bounces={false} style={{ flex: 1 }}>
      <Body>
        <View style={{ alignItems: "center", gap: 8, paddingTop: 8, paddingBottom: 16 }}>
          <View
            style={{
              width: 56,
              height: 56,
              backgroundColor: palette().soft,
              borderRadius: radius.xxl,
              alignItems: "center",
              justifyContent: "center",
            }}
          >
            <Icon name="paw-print" size={26} color={palette().text} />
          </View>
          <Text style={{ fontSize: 20, fontWeight: "600", color: palette().text }}>Paws</Text>
          <Hint
            text={ui.t(
              "HarmonyOS 原生多内核代理客户端",
              "A native multi-engine proxy client for HarmonyOS",
            )}
          />
        </View>
        <View testID="about-version-section">
          <Card title={ui.t("版本", "Version")}>
            <Detail stacked label={ui.t("应用版本", "App version")} value={about().appVersion} />
            <Detail stacked label={ui.t("核心版本", "Core version")} value={about().coreVersion} />
            <Detail
              stacked
              label={ui.t("界面运行时", "UI runtime")}
              value={`${about().uiRuntime} ${about().uiRuntimeVersion}`}
            />
            <Detail label="Rust" value={about().rustVersion} />
          </Card>
        </View>
        <View testID="about-engine-section">
          <Card
            title={ui.t("代理内核", "Proxy engines")}
            subtitle={ui.t(
              "当前安装包内置的内核版本，包含 HarmonyOS 适配补丁。",
              "Engine versions bundled with this package, including HarmonyOS adaptations.",
            )}
          >
            {renderComponent("Mihomo", about().mihomoVersion, "GPL-3.0", 0)}
            <Separator />
            {renderComponent("sing-box", about().singBoxVersion, "GPL-3.0-or-later", 2)}
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
              about().uiRuntimeVersion,
              ui.t("许可证", "License"),
              6,
            )}
            <Separator />
            <View testID="about-component-flclash" style={{ minWidth: 0, gap: 8 }}>
              <Text
                style={{ fontSize: 14, lineHeight: 20, fontWeight: "500", color: palette().text }}
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
                <Action
                  id="about-source-8"
                  variant="link"
                  small
                  label={ui.t("源代码", "Source code")}
                  onPress={() =>
                    void ui.client.run("platform.openUrl", { url: ABOUT_LINKS[8].url })
                  }
                />
                <Action
                  id="about-adaptations"
                  variant="link"
                  small
                  label={ui.t("第三方组件与适配说明", "Third-party components and adaptations")}
                  onPress={() =>
                    void ui.client.run("platform.openUrl", { url: ABOUT_LINKS[9].url })
                  }
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
    <Scroll bounces={false} style={{ flex: 1 }}>
      <Body>
        <Card title={ui.t("隐私与数据处理", "Privacy & data handling")}>
          <Each of={about().privacySummary}>
            {(line) => (
              <View style={{ flexDirection: "row", alignItems: "flex-start", gap: 8 }}>
                <Icon name="shield-check" size={16} color={palette().success} />
                <Text style={{ flex: 1, fontSize: 14, lineHeight: 21, color: palette().text }}>
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
              <Action
                variant="link"
                label={service().name}
                onPress={() =>
                  void ui.client.run("platform.openUrl", { url: service().documentationUrl })
                }
              />
            )}
          </Each>
        </Card>
      </Body>
    </Scroll>
  );
}

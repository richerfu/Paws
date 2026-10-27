import { batch, effect, Scroll, signal, Text, untracked, View, Clipboard } from "nsh";
import {
  Action,
  Actions,
  Body,
  Card,
  Detail,
  Empty,
  Field,
  Hint,
  IconButton,
  RadioGroup,
  Select,
  palette,
  Separator,
  Toggle,
  useApp,
  useLifetime,
} from "../components";
import type { ControllerAccess, Engine, NetworkPorts, VpnOptions } from "../model";
import { DraftEdits, draftDecision } from "../draft";
import { PageError } from "../page-error";
import { statusColors } from "../theme";

const lines = (text: string) =>
  text
    .split(/[\n,]/)
    .map((value) => value.trim())
    .filter(Boolean);
const formatPolicy = (policy: Record<string, string[]> = {}) =>
  Object.entries(policy)
    .map(([name, values]) => `${name}=${values.join(",")}`)
    .join("\n");
const parsePolicy = (text: string) =>
  Object.fromEntries(
    text
      .split("\n")
      .filter((line) => line.trim())
      .map((line) => {
        const separator = line.indexOf("=");
        if (separator < 1)
          throw new Error("DNS policy requires matcher=server1,server2 on each line");
        return [line.slice(0, separator).trim(), lines(line.slice(separator + 1))];
      }),
  );

export function Settings() {
  const ui = useApp(),
    current = () => ui.client.state()!;
  const alive = useLifetime();
  const baseline = signal(current().configRevision),
    profile = signal(current().activeProfile);
  const vpn = signal<VpnOptions>({ ...current().vpnOptions }),
    ports = signal<NetworkPorts>({ ...current().networkPorts }),
    controller = signal<ControllerAccess>({ ...current().controllerAccess });
  const dnsServers = signal(current().vpnOptions.dnsServers.join("\n")),
    fallbacks = signal((current().vpnOptions.dnsFallbacks || []).join("\n")),
    policy = signal(formatPolicy(current().vpnOptions.dnsNameserverPolicy));
  type Section = "vpn" | "dns" | "network";
  const edits = { vpn: new DraftEdits(), dns: new DraftEdits(), network: new DraftEdits() };
  const dirty = signal(new Set<Section>()),
    conflict = signal(false),
    mtu = signal(String(current().vpnOptions.mtu));
  const mark = (section: Section) => {
    edits[section].edit();
    dirty.update((previous) => new Set([...previous, section]));
  };
  const mixedPort = signal(String(current().networkPorts.mixedPort)),
    controllerPort = signal(String(current().networkPorts.controllerPort));
  const reset = () =>
    batch(() => {
      const state = current();
      baseline.set(state.configRevision);
      profile.set(state.activeProfile);
      vpn.set({ ...state.vpnOptions });
      ports.set({ ...state.networkPorts });
      controller.set({ ...state.controllerAccess });
      dnsServers.set(state.vpnOptions.dnsServers.join("\n"));
      fallbacks.set((state.vpnOptions.dnsFallbacks || []).join("\n"));
      policy.set(formatPolicy(state.vpnOptions.dnsNameserverPolicy));
      mtu.set(String(state.vpnOptions.mtu));
      mixedPort.set(String(state.networkPorts.mixedPort));
      controllerPort.set(String(state.networkPorts.controllerPort));
      dirty.set(new Set());
      conflict.set(false);
    });
  effect(() => {
    const revision = current().configRevision;
    untracked(() => {
      const decision = draftDecision(baseline(), revision, dirty().size > 0);
      if (decision === "conflict") conflict.set(true);
      if (decision === "reload") reset();
    });
  });
  const changeVpn = (value: Partial<VpnOptions>) => {
    vpn.set({ ...vpn(), ...value });
    mark("vpn");
  };
  const changePorts = (value: Partial<NetworkPorts>) => {
    ports.set({ ...ports(), ...value });
    mark("network");
  };
  const changeController = (value: Partial<ControllerAccess>) => {
    controller.set({ ...controller(), ...value });
    mark("network");
  };
  const enginePreview = signal(""),
    previewReasons = signal(""),
    previewBusy = signal(false),
    previewFailed = signal(false);
  let previewGeneration = 0;
  effect(() => {
    const engine = vpn().engine,
      revision = current().configRevision,
      active = current().activeProfile;
    const generation = ++previewGeneration;
    untracked(() => {
      if (!active) {
        batch(() => {
          previewBusy.set(false);
          enginePreview.set("");
          previewReasons.set("");
          previewFailed.set(false);
        });
        return;
      }
      previewBusy.set(true);
      void ui.client
        .send<{ selected: string; reasons?: string[] }>("engines.decision", { options: vpn() })
        .then((value) => {
          if (
            alive() &&
            generation === previewGeneration &&
            vpn().engine === engine &&
            current().activeProfile === active &&
            current().configRevision === revision
          )
            batch(() => {
              enginePreview.set(value.selected);
              previewFailed.set(false);
              previewReasons.set(value.reasons?.join("\n") || "");
            });
        })
        .catch((error) => {
          if (
            alive() &&
            generation === previewGeneration &&
            vpn().engine === engine &&
            current().activeProfile === active &&
            current().configRevision === revision
          )
            batch(() => {
              enginePreview.set("");
              previewFailed.set(true);
              previewReasons.set(String(error));
            });
        })
        .finally(() => {
          if (alive() && generation === previewGeneration) previewBusy.set(false);
        });
    });
  });
  const normalizeEngine = (engine: string) => engine === "sing-box" ? "singbox" : engine;
  const previewFallback = () =>
    !previewBusy() && !previewFailed() && !!enginePreview() && vpn().engine !== "auto" &&
    normalizeEngine(enginePreview()) !== normalizeEngine(vpn().engine);
  const previewStatus = () =>
    previewBusy()
      ? ui.t("正在检查内核…", "Checking engine…")
      : previewFailed()
        ? ui.t("内核检查失败", "Engine check failed")
        : enginePreview()
          ? `${ui.t("下次连接：", "Next connection: ")}${enginePreview()}`
          : ui.t("尚未检查", "Not checked");
  const blocked = () =>
    !profile() || conflict() || ui.client.pending() > 0 || !ui.client.domains.fresh(["config"]);
  const save = async (section: "vpn" | "dns" | "network") => {
    const ticket = edits[section].ticket();
    try {
      let payload: Record<string, unknown> = { profileId: profile(), expectedRevision: baseline() };
      if (section === "vpn") payload = { ...payload, options: { ...vpn(), mtu: Number(mtu()) } };
      if (section === "dns")
        payload = {
          ...payload,
          servers: lines(dnsServers()),
          fallbacks: lines(fallbacks()),
          policy: parsePolicy(policy()),
        };
      if (section === "network")
        payload = {
          ...payload,
          ports: {
            ...ports(),
            mixedPort: Number(mixedPort()),
            controllerPort: Number(controllerPort()),
          },
          controller: controller(),
        };
      const result = await ui.client.run(
        `settings.${section}`,
        payload,
        ui.t("设置已保存", "Settings saved"),
      );
      // A successful write waits for the authoritative config query before its result.
      // Preserve other draft sections while advancing this draft's revision.
      if (alive() && result !== undefined && ui.client.domains.fresh(["config"])) {
        if (edits[section].unchangedSince(ticket))
          dirty.update((previous) => {
            const next = new Set(previous);
            next.delete(section);
            return next;
          });
        baseline.set(current().configRevision);
        conflict.set(false);
      }
    } catch (error) {
      ui.client.error.set(error instanceof Error ? error.message : String(error));
    }
  };
  return (
    <Scroll bounces={false} style={{ flex: 1 }}>
      <Body>
        <If
          when={profile()}
          else={
            <View
              style={{
                minHeight: 248,
                padding: 24,
                borderWidth: 1,
                borderStyle: "dashed",
                borderColor: palette().border,
                borderRadius: 8,
                alignItems: "center",
                justifyContent: "center",
              }}
            >
              <Empty
                icon="rss"
                title={
                  ui.client.domains.data("summary")!.profileCount
                    ? ui.t("选择一个配置", "Choose a profile")
                    : ui.t("还没有配置", "No profiles yet")
                }
                text={
                  ui.client.domains.data("summary")!.profileCount
                    ? ui.t(
                        "你已有配置。在订阅中启用一个后，即可调整它的网络设置。",
                        "Activate an existing profile in Subscriptions to adjust its network settings.",
                      )
                    : ui.t(
                        "导入订阅链接或本地配置文件，即可设置 VPN、DNS 和本地服务。",
                        "Import a subscription URL or local configuration file to configure VPN, DNS and local services.",
                      )
                }
              />
              <Action
                label={
                  ui.client.domains.data("summary")!.profileCount
                    ? ui.t("选择配置", "Choose a profile")
                    : ui.t("导入配置", "Import profile")
                }
                onPress={() => {
                  ui.navigate("profiles");
                  if (!ui.client.domains.data("summary")!.profileCount)
                    ui.importIntent.set(ui.importIntent() + 1);
                }}
              />
            </View>
          }
        >
          <If when={conflict()}>
            <Card title={ui.t("配置已变更", "Configuration changed")}>
              <Hint
                danger
                text={ui.t(
                  "保留了当前草稿。重新载入最新配置后再保存。",
                  "Your draft is preserved. Reload the latest configuration before saving.",
                )}
              />
              <Action
                secondary
                label={ui.t("丢弃草稿并重新载入", "Discard draft and reload")}
                onPress={reset}
              />
            </Card>
          </If>
          <Card
            title={ui.t("VPN 基础", "VPN basics")}
            subtitle={ui.t(
              "运行中的 VPN 会在保存后请求重连",
              "Saving requests a reconnect for the running VPN",
            )}
          >
            <Toggle
              label={ui.t("DNS 劫持", "DNS hijacking")}
              description={ui.t(
                "将应用的 DNS 请求交由 VPN 处理。",
                "Handle application DNS requests through the VPN.",
              )}
              value={vpn().dnsHijacking}
              onChange={(dnsHijacking) => changeVpn({ dnsHijacking })}
              disabled={ui.client.pending() > 0 || !ui.client.domains.fresh(["config"])}
            />
            <Separator />
            <Select
              id="vpn-engine"
              label={ui.t("VPN 内核", "VPN engine")}
              value={vpn().engine === "sing-box" ? "singbox" : vpn().engine}
              options={[
                { value: "auto", label: ui.t("自动选择", "Automatic") },
                { value: "mihomo", label: "mihomo" },
                { value: "singbox", label: "sing-box" },
                { value: "meow", label: "meow-rs" },
              ]}
              disabled={ui.client.pending() > 0 || !ui.client.domains.fresh(["config"])}
              loading={previewBusy()}
              labelAccessory={
                <View
                  testID="engine-preview-status"
                  style={{ height: 24, flexDirection: "row", alignItems: "center", gap: 4 }}
                >
                  <If when={!previewBusy() && (previewFailed() || previewFallback())}>
                    <Text
                      testID="engine-preview-warning"
                      numberOfLines={1}
                      style={{ maxWidth: 128, fontSize: 12, lineHeight: 20, color: statusColors("warning").text }}
                    >
                      {previewFailed()
                        ? ui.t("检查失败", "Check failed")
                        : `${ui.t("下次：", "Next: ")}${enginePreview()}`}
                    </Text>
                  </If>
                  <PageError
                    id="engine-preview-info"
                    danger={previewFailed()}
                    text={[previewStatus(), previewReasons()].filter(Boolean).join("\n")}
                    renderTrigger={(open) => (
                      <IconButton
                        id="engine-preview-details"
                        icon={previewFailed() || previewFallback() ? "triangle-alert" : "info"}
                        label={`${previewStatus()}，${ui.t("查看详情", "View details")}`}
                        size={24}
                        disabled={previewBusy()}
                        onPress={open}
                      />
                    )}
                  />
                </View>
              }
              onChange={(engine) => changeVpn({ engine: engine as Engine })}
            />
            <Select
              id="vpn-stack"
              label={ui.t("网络栈", "Network stack")}
              value={vpn().stack}
              options={[
                { value: "netstack-smoltcp", label: "smoltcp" },
                { value: "lwip", label: "lwIP" },
              ]}
              disabled={ui.client.pending() > 0 || !ui.client.domains.fresh(["config"])}
              onChange={(stack) => changeVpn({ stack })}
            />
            <Hint
              text={ui.t(
                "仅对 meow-rs 生效；mihomo 与 sing-box 使用内置系统或混合 TUN 栈。",
                "Applies to meow-rs only; mihomo and sing-box use built-in system or mixed TUN stacks.",
              )}
            />
            <Field
              id="vpn-mtu"
              label="MTU"
              value={mtu()}
              placeholder="1500"
              numeric
              disabled={blocked()}
              onChange={(value) => {
                if (value === mtu()) return;
                mtu.set(value);
                mark("vpn");
              }}
            />
            <Separator />
            <If when={vpn().systemProxy}>
              <Hint
                danger
                text={ui.t(
                  "当前平台不支持系统 HTTP 代理。",
                  "System HTTP proxy is unsupported on this platform.",
                )}
              />
              <Action
                secondary
                label={ui.t("移除不支持的系统代理设置", "Remove unsupported system proxy setting")}
                onPress={() => changeVpn({ systemProxy: false })}
              />
            </If>
            <If when={vpn().allowBypass}>
              <Hint
                danger
                text={ui.t(
                  "当前平台不支持绕过 VPN。",
                  "VPN bypass is unsupported on this platform.",
                )}
              />
              <Action
                secondary
                label={ui.t("移除不支持的绕过设置", "Remove unsupported bypass setting")}
                onPress={() => changeVpn({ allowBypass: false })}
              />
            </If>
            <Field
              label={ui.t("通过 VPN 的应用", "Applications through VPN")}
              value={(vpn().trustedApplications || []).join(", ")}
              placeholder="com.easy.hmos.abroad"
              onChange={(text) => changeVpn({ trustedApplications: lines(text) })}
            />
            <Field
              label={ui.t("绕过 VPN 的应用", "Applications bypassing VPN")}
              value={(vpn().blockedApplications || []).join(", ")}
              placeholder="com.huawei.hmos.browser"
              onChange={(text) => changeVpn({ blockedApplications: lines(text) })}
            />
            <Hint
              text={ui.t(
                "应用白名单和绕过名单只能设置一项；多个包名用逗号分隔。",
                "Set either an allow list or a bypass list. Separate package names with commas.",
              )}
            />
            <Actions>
              <Action
                secondary
                label={ui.t("选择出境易容器", "Select the Abroad Easy container")}
                disabled={ui.client.pending() > 0 || !ui.client.domains.fresh(["config"])}
                onPress={() =>
                  changeVpn({
                    trustedApplications: ["com.easy.hmos.abroad"],
                    blockedApplications: [],
                  })
                }
              />
              <Action
                secondary
                label={ui.t("清除应用限制", "Clear application restrictions")}
                disabled={
                  ui.client.pending() > 0 ||
                  !ui.client.domains.fresh(["config"]) ||
                  (!vpn().trustedApplications?.length && !vpn().blockedApplications?.length)
                }
                onPress={() => changeVpn({ trustedApplications: [], blockedApplications: [] })}
              />
            </Actions>
            <Action
              id="save-vpn"
              label={ui.t("保存 VPN 设置", "Save VPN settings")}
              disabled={
                blocked() ||
                !dirty().has("vpn") ||
                !Number.isInteger(Number(mtu())) ||
                Number(mtu()) < 1280 ||
                Number(mtu()) > 65535 ||
                !!(vpn().trustedApplications?.length && vpn().blockedApplications?.length)
              }
              onPress={() => void save("vpn")}
            />
          </Card>
          <Card
            title={ui.t("网络端口与控制器", "Network ports and controller")}
            subtitle={ui.t(
              "默认端口为 7890 和 9090；端口范围 1024–65535，且不能相同",
              "Defaults: 7890 and 9090. Ports must be distinct integers from 1024 to 65535.",
            )}
          >
            <Toggle
              label={ui.t("启用混合代理服务", "Enable mixed proxy service")}
              description={ui.t(
                "仅在 VPN 已连接时监听代理端口",
                "Listen on the proxy port only while VPN is connected",
              )}
              value={ports().mixedEnabled}
              onChange={(mixedEnabled) => changePorts({ mixedEnabled })}
            />
            <Field
              label={ui.t("混合代理端口", "Mixed proxy port")}
              value={mixedPort()}
              placeholder="7890"
              numeric
              onChange={(value) => {
                mixedPort.set(value);
                mark("network");
              }}
            />
            <Toggle
              label={ui.t("启用 Controller 服务", "Enable controller service")}
              description={ui.t(
                "仅在 VPN 已连接时监听控制端口",
                "Listen on the controller port only while VPN is connected",
              )}
              value={ports().controllerEnabled}
              onChange={(controllerEnabled) => changePorts({ controllerEnabled })}
            />
            <Field
              label={ui.t("控制器端口", "Controller port")}
              value={controllerPort()}
              placeholder="9090"
              numeric
              onChange={(value) => {
                controllerPort.set(value);
                mark("network");
              }}
            />
            <Toggle
              label={ui.t("允许局域网访问", "Allow LAN access")}
              value={controller().allowLan}
              onChange={(allowLan) => changeController({ allowLan })}
            />
            <Detail
              label={ui.t("应用内部地址", "Internal address")}
              value={`127.0.0.1:${controllerPort()}`}
            />
            <If when={controller().allowLan}>
              <Detail
                label={ui.t("局域网访问", "LAN access")}
                value={`${ui.t("使用设备局域网 IP", "Use the device LAN IP")}:${controllerPort()}`}
              />
              <Hint text="Authorization: Bearer <secret>" />
              <Detail
                label={ui.t("访问密钥", "Access secret")}
                value={controller().secret ? "••••••••" : "—"}
              />
              <Action
                secondary
                small
                label={ui.t("复制", "Copy")}
                disabled={!controller().secret}
                onPress={() => Clipboard.writeText(controller().secret || "")}
              />
            </If>
            <Action
              label={ui.t("保存网络设置", "Save network settings")}
              disabled={blocked() || !dirty().has("network")}
              onPress={() => void save("network")}
            />
          </Card>
          <Card
            title="DNS"
            subtitle={ui.t(
              "每行一个地址，策略格式为 matcher = upstream",
              "One address per line; policy format: matcher = upstream",
            )}
          >
            <Field
              label={ui.t("上游 DNS", "Upstream DNS")}
              value={dnsServers()}
              onChange={(value) => {
                dnsServers.set(value);
                mark("dns");
              }}
              multiline
              height={92}
            />
            <Field
              label={ui.t("备用 DNS", "Fallback DNS")}
              value={fallbacks()}
              onChange={(value) => {
                fallbacks.set(value);
                mark("dns");
              }}
              multiline
              height={76}
            />
            <Field
              label={ui.t("分流策略（匹配项=服务器）", "Policy, matcher=servers")}
              value={policy()}
              onChange={(value) => {
                policy.set(value);
                mark("dns");
              }}
              multiline
              height={104}
              placeholder="geosite:cn=223.5.5.5,119.29.29.29"
            />
            <Action
              label={ui.t("保存 DNS 设置", "Save DNS settings")}
              disabled={blocked() || !dirty().has("dns") || !dnsServers().trim()}
              onPress={() => void save("dns")}
            />
          </Card>
        </If>
      </Body>
    </Scroll>
  );
}

export function Appearance() {
  const ui = useApp(),
    prefs = () => ui.client.preferences();
  return (
    <Scroll bounces={false} style={{ flex: 1 }}>
      <Body>
        <Card
          title={ui.t("语言", "Language")}
          subtitle={ui.t(
            "选择界面语言；跟随系统会响应系统语言变化",
            "Choose the interface language or follow system settings.",
          )}
        >
          <RadioGroup
            label={ui.t("语言", "Language")}
            value={prefs().language}
            options={[
              { value: "system", label: ui.t("跟随系统", "System") },
              { value: "zh_cn", label: "简体中文" },
              { value: "en", label: "English" },
            ]}
            onChange={(language) => void ui.client.run("preferences.set", { language })}
          />
        </Card>
        <Card
          title={ui.t("主题", "Theme")}
          subtitle={ui.t(
            "切换浅色、深色或跟随系统；修改会立即生效",
            "Choose light, dark or follow the system appearance.",
          )}
        >
          <RadioGroup
            label={ui.t("主题", "Theme")}
            value={prefs().theme}
            options={[
              { value: "system", label: ui.t("跟随系统", "System") },
              { value: "light", label: ui.t("浅色", "Light") },
              { value: "dark", label: ui.t("深色", "Dark") },
            ]}
            onChange={(theme) => void ui.client.run("preferences.set", { theme })}
          />
        </Card>
      </Body>
    </Scroll>
  );
}

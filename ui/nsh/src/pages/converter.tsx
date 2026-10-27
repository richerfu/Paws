import { cleanup, Scroll, signal, View, Clipboard } from "nsh";
import {
  Action,
  Actions,
  Body,
  Card,
  Field,
  Hint,
  Select,
  Tabs,
  Toggle,
  useApp,
  useLifetime,
} from "../components";
import { ConverterDraftState, type ConverterDraft } from "../converter-draft";
import { CLIENT_TYPES, REMOTE_CONFIGS } from "../converter-options";
import { PageError } from "../page-error";

export function Converter() {
  const ui = useApp(),
    alive = useLifetime();
  const state = new ConverterDraftState(ui.client, {
    alive,
    saved: () => ui.t("转换设置已保存", "Converter settings saved"),
    changed: () =>
      ui.t(
        "解析期间转换设置已修改。已保留当前编辑，请重新解析。",
        "The settings changed while parsing. Your edits were preserved; parse the link again.",
      ),
  });
  const { draft, dirty, loading } = state;
  const longUrl = signal(""),
    shortUrl = signal(""),
    parseUrl = signal(""),
    uploadText = signal(""),
    version = signal(""),
    operation = signal("");
  let uploadEdits = 0;
  void state.load();
  cleanup(() => {
    if (draft() && dirty()) void ui.client.run("converter.saveDraft", { draft: draft() });
  });
  const change = (patch: Partial<ConverterDraft>) => state.change(patch);
  const busy = () => operation() !== "";
  const copy = (text: string, notice: string) => {
    Clipboard.writeText(text);
    ui.client.notice.set(notice);
  };
  const run = async (name: string, action: () => Promise<void>) => {
    if (busy()) return;
    operation.set(name);
    try {
      await action();
    } finally {
      if (alive()) operation.set("");
    }
  };
  const build = () =>
    run("build", async () => {
      const submitted = draft()!;
      const url = await ui.client.run<string>("converter.buildUrl", { draft: submitted });
      if (!alive() || url === undefined) return;
      if (draft() !== submitted) {
        ui.client.error.set(
          ui.t(
            "生成期间设置已修改，请重新生成。",
            "Settings changed during generation. Generate the link again.",
          ),
        );
        return;
      }
      longUrl.set(url);
      shortUrl.set("");
      await state.save();
      if (alive()) copy(url, ui.t("转换链接已生成并复制", "Conversion URL generated and copied"));
    });
  const shorten = () =>
    run("short", async () => {
      const submitted = longUrl();
      const url = await ui.client.run<string>("converter.shortUrl", {
        api: draft()!.short_url_api,
        longUrl: submitted,
      });
      if (alive() && longUrl() === submitted && url !== undefined) {
        shortUrl.set(url);
        copy(url, ui.t("短链接已生成并复制", "Short URL generated and copied"));
      }
    });
  const parse = () =>
    run("parse", async () => {
      const previous = draft();
      await state.parse(parseUrl());
      if (alive() && draft() !== previous) {
        longUrl.set("");
        shortUrl.set("");
        await state.save();
      }
    });
  const upload = () =>
    run("upload", async () => {
      const submitted = draft()!,
        generation = uploadEdits;
      const url = await ui.client.run<string>("converter.uploadConfig", {
        api: submitted.config_upload_api,
        content: uploadText(),
      });
      if (!alive() || url === undefined) return;
      if (draft() !== submitted || uploadEdits !== generation) {
        ui.client.error.set(
          ui.t(
            "上传期间内容已修改。已保留当前编辑。",
            "Content changed during upload. Your edits were preserved.",
          ),
        );
        return;
      }
      change({ remote_config: url });
      uploadText.set("");
      await state.save();
      if (alive())
        copy(
          url,
          ui.t("配置已上传，地址已填入并复制", "Configuration uploaded; URL filled and copied"),
        );
    });
  const checkVersion = () =>
    run("version", async () => {
      const backend = draft()!.backend;
      const value = await ui.client.run<string>("converter.backendVersion", { backend });
      if (alive() && draft()?.backend === backend && value !== undefined) version.set(value);
    });
  return (
    <Scroll bounces={false} style={{ flex: 1 }}>
      <Body>
        <If when={!draft()}>
          <Card title={ui.t("转换规则", "Conversion rules")}>
            <If
              when={ui.client.error()}
              else={<Hint text={ui.t("正在读取转换设置…", "Loading conversion settings…")} />}
            >
              <PageError id="converter-load-error" text={ui.client.error()} />
            </If>
            <Action
              secondary
              label={ui.t("重新读取", "Retry")}
              loading={loading()}
              disabled={loading()}
              onPress={() => void state.load()}
            />
          </Card>
        </If>
        <If when={draft()}>
          <>
            <Card
              title={ui.t("转换规则", "Conversion rules")}
              subtitle={ui.t(
                "兼容 sub-web 的订阅转换规则；生成链接后可导入 Clash。",
                "Subscription conversion compatible with sub-web. Generate a link to import into Clash.",
              )}
            >
              <Hint text={ui.t("模式", "Mode")} />
              <Tabs
                idPrefix="converter-mode"
                value={draft()!.advanced ? "advanced" : "basic"}
                options={[
                  { value: "basic", label: ui.t("基础模式", "Basic") },
                  { value: "advanced", label: ui.t("进阶模式", "Advanced") },
                ]}
                onChange={(value) => change({ advanced: value === "advanced" })}
              />
              <Field
                label={ui.t("订阅或节点链接 *", "Subscription or node links *")}
                value={draft()!.source_sub_url}
                placeholder={ui.t(
                  "每行一个订阅或节点链接",
                  "One subscription or node link per line",
                )}
                multiline
                height={108}
                disabled={busy()}
                onChange={(source_sub_url) => change({ source_sub_url })}
              />
              <Select
                label={ui.t("目标客户端 *", "Target client *")}
                value={draft()!.client_type}
                options={CLIENT_TYPES}
                disabled={busy()}
                onChange={(client_type) => change({ client_type })}
              />
            </Card>
            <If when={draft()!.advanced}>
              <>
                <Card
                  title={ui.t("后端与远程配置", "Backend and remote configuration")}
                  subtitle={ui.t(
                    "转换请求由目标服务处理，请只使用信任的后端。",
                    "Conversion requests are handled by the selected service. Use a trusted backend.",
                  )}
                >
                  <Field
                    label={ui.t("转换后端", "Conversion backend")}
                    value={draft()!.backend}
                    placeholder="http://127.0.0.1:25500/sub?"
                    disabled={busy()}
                    onChange={(backend) => {
                      version.set("");
                      change({ backend });
                    }}
                  />
                  <Actions>
                    <Hint text={version() || ui.t("尚未检测后端", "Backend not checked")} />
                    <Action
                      secondary
                      small
                      icon="network"
                      label={ui.t("检测", "Check")}
                      loading={operation() === "version"}
                      disabled={busy()}
                      onPress={() => void checkVersion()}
                    />
                  </Actions>
                  <Select
                    label={ui.t("远程配置预设", "Remote config preset")}
                    value={
                      REMOTE_CONFIGS.some((option) => option.value === draft()!.remote_config)
                        ? draft()!.remote_config
                        : ""
                    }
                    options={REMOTE_CONFIGS.map((option) => ({
                      ...option,
                      label: option.label || ui.t("自定义", "Custom"),
                    }))}
                    disabled={busy()}
                    onChange={(remote_config) => change({ remote_config })}
                  />
                  <Field
                    label={ui.t("远程配置地址", "Remote config URL")}
                    value={draft()!.remote_config}
                    placeholder={ui.t(
                      "可选择预设，也可直接输入 URL",
                      "Choose a preset or enter a URL",
                    )}
                    disabled={busy()}
                    onChange={(remote_config) => change({ remote_config })}
                  />
                  <Field
                    label="Include"
                    value={draft()!.include_remarks}
                    placeholder={ui.t(
                      "节点名必须包含，支持正则",
                      "Include node names matching a regex",
                    )}
                    disabled={busy()}
                    onChange={(include_remarks) => change({ include_remarks })}
                  />
                  <Field
                    label="Exclude"
                    value={draft()!.exclude_remarks}
                    placeholder={ui.t(
                      "排除匹配的节点名，支持正则",
                      "Exclude node names matching a regex",
                    )}
                    disabled={busy()}
                    onChange={(exclude_remarks) => change({ exclude_remarks })}
                  />
                  <Field
                    label="FileName"
                    value={draft()!.filename}
                    placeholder={ui.t("返回的订阅文件名", "Output subscription filename")}
                    disabled={busy()}
                    onChange={(filename) => change({ filename })}
                  />
                  <Field
                    label={ui.t("自定义参数", "Custom parameters")}
                    value={draft()!
                      .custom_params.map((item) => `${item.name}=${item.value}`)
                      .join("\n")}
                    placeholder="name=value"
                    multiline
                    height={88}
                    disabled={busy()}
                    onChange={(text) =>
                      change({
                        custom_params: text
                          .split("\n")
                          .filter((line) => line.includes("="))
                          .map((line) => ({
                            name: line.slice(0, line.indexOf("=")).trim(),
                            value: line.slice(line.indexOf("=") + 1).trim(),
                          })),
                      })
                    }
                  />
                </Card>
                <Card title={ui.t("输出与定制选项", "Output and customization")}>
                  <Each
                    of={[
                      [
                        "node_list",
                        "输出为 Node List",
                        "Output as Node List",
                        "只返回节点列表",
                        "Return nodes only",
                      ],
                      [
                        "emoji",
                        "Emoji",
                        "Emoji",
                        "为节点名添加地区 Emoji",
                        "Add region emoji to node names",
                      ],
                      ["scv", "跳过证书验证", "Skip certificate verification", "scv", "scv"],
                      ["udp", "启用 UDP", "Enable UDP", "udp", "udp"],
                      ["tfo", "TCP Fast Open", "TCP Fast Open", "tfo", "tfo"],
                      [
                        "append_type",
                        "附加节点类型",
                        "Append node type",
                        "append_type",
                        "append_type",
                      ],
                      ["sort", "排序节点", "Sort nodes", "sort", "sort"],
                      ["fdn", "过滤非法节点", "Filter invalid nodes", "fdn", "fdn"],
                      ["expand", "展开规则", "Expand rules", "expand", "expand"],
                      ["surge_doh", "Surge DoH", "Surge DoH", "surge.doh", "surge.doh"],
                      ["clash_doh", "Clash DoH", "Clash DoH", "clash.doh", "clash.doh"],
                      ["new_name", "Clash 新字段", "New Clash field names", "new_name", "new_name"],
                      [
                        "insert",
                        "插入默认节点",
                        "Insert default nodes",
                        "insert / insert_url",
                        "insert / insert_url",
                      ],
                    ]}
                  >
                    {(option) => (
                      <Toggle
                        label={ui.t(option()[1], option()[2])}
                        description={ui.t(option()[3], option()[4])}
                        disabled={busy()}
                        value={!!draft()![option()[0] as keyof ConverterDraft]}
                        onChange={(value) =>
                          change({
                            [option()[0]]: value,
                            ...(option()[0] === "udp" ? { need_udp: true } : {}),
                          })
                        }
                      />
                    )}
                  </Each>
                </Card>
              </>
            </If>
            <Card
              title={ui.t("生成结果", "Generated result")}
              subtitle={ui.t(
                "生成操作只构造转换地址，访问时会将订阅地址发送给转换服务。",
                "Generation builds a URL. Opening it sends the subscription URLs to the conversion service.",
              )}
            >
              <Action
                id="converter-build"
                icon="refresh-cw"
                label={ui.t("生成订阅链接", "Generate subscription URL")}
                loading={operation() === "build"}
                disabled={busy() || !draft()!.source_sub_url.trim()}
                onPress={() => void build()}
              />
              <Field
                label={ui.t("定制订阅", "Customized subscription")}
                value={longUrl()}
                placeholder={ui.t("生成后显示长链接", "Generated long URL")}
                multiline
                height={82}
                onChange={(value) => {
                  longUrl.set(value);
                  shortUrl.set("");
                }}
              />
              <Actions>
                <Action
                  secondary
                  small
                  icon="file-text"
                  label={ui.t("复制长链", "Copy long URL")}
                  disabled={!longUrl()}
                  onPress={() => copy(longUrl(), ui.t("长链接已复制", "Long URL copied"))}
                />
                <Action
                  secondary
                  small
                  icon="network"
                  label={ui.t("生成短链", "Generate short URL")}
                  loading={operation() === "short"}
                  disabled={busy() || !longUrl()}
                  onPress={() => void shorten()}
                />
              </Actions>
              <Field
                label={ui.t("订阅短链", "Short URL")}
                value={shortUrl()}
                placeholder={ui.t("生成后显示短链接", "Generated short URL")}
                onChange={() => {}}
                disabled
              />
              <If when={shortUrl()}>
                <Action
                  variant="link"
                  label={ui.t("复制短链接", "Copy short URL")}
                  onPress={() => copy(shortUrl(), ui.t("短链接已复制", "Short URL copied"))}
                />
              </If>
              <Action
                secondary
                icon="external-link"
                label={ui.t("一键导入 Clash", "Import into Clash")}
                disabled={busy() || !longUrl()}
                onPress={() =>
                  void ui.client.run("platform.openUrl", {
                    url: `clash://install-config?url=${encodeURIComponent(shortUrl() || longUrl())}`,
                  })
                }
              />
            </Card>
            <Card
              title={ui.t("从长链或短链解析", "Parse a long or short URL")}
              subtitle={ui.t(
                "解析已有转换地址，将规则载入当前表单。",
                "Load conversion settings from an existing URL.",
              )}
            >
              <Field
                label=""
                value={parseUrl()}
                placeholder="https://…/sub?target=clash&url=…"
                multiline
                height={82}
                disabled={busy()}
                onChange={(value) => parseUrl.set(value)}
              />
              <Action
                secondary
                icon="download"
                label={ui.t("解析并载入", "Parse and load")}
                loading={operation() === "parse"}
                disabled={busy() || !parseUrl().trim()}
                onPress={() => void parse()}
              />
            </Card>
            <Card
              title={ui.t("辅助服务", "Supporting services")}
              subtitle={ui.t(
                "配置与短链内容会发送至所填写的服务，请谨慎选择。",
                "Content is sent to the configured upload and short URL services. Choose them carefully.",
              )}
            >
              <Field
                label={ui.t("短链服务", "Short URL service")}
                value={draft()!.short_url_api}
                disabled={busy()}
                onChange={(short_url_api) => change({ short_url_api })}
              />
              <Field
                label={ui.t("配置上传服务", "Config upload service")}
                value={draft()!.config_upload_api}
                disabled={busy()}
                onChange={(config_upload_api) => change({ config_upload_api })}
              />
              <Field
                label={ui.t("远程配置内容", "Remote config content")}
                value={uploadText()}
                placeholder={ui.t(
                  "粘贴 subconverter INI 配置",
                  "Paste a subconverter INI configuration",
                )}
                multiline
                height={112}
                disabled={busy()}
                onChange={(value) => {
                  uploadEdits++;
                  uploadText.set(value);
                }}
              />
              <Action
                secondary
                icon="file-up"
                label={ui.t("上传并使用配置", "Upload and use configuration")}
                loading={operation() === "upload"}
                disabled={busy() || !uploadText().trim()}
                onPress={() => void upload()}
              />
            </Card>
          </>
        </If>
      </Body>
    </Scroll>
  );
}

import {
  cleanup,
  computed,
  effect,
  Scroll,
  signal,
  Text,
  untracked,
  View,
  Image,
  Clipboard,
  windowSize,
  List,
} from "nsh";
import {
  Action,
  Actions,
  Badge,
  Body,
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
  Separator,
  Spinner,
  Tabs,
  useApp,
  useLifetime,
} from "../components";
import { bytes, timestamp } from "../locale";
import { DraftEdits } from "../draft";
import { matchesProfile } from "../query";
import { PageError } from "../page-error";
import type { Profile } from "../model";

export function Profiles() {
  const ui = useApp(),
    query = signal(""),
    name = signal(""),
    input = signal(""),
    open = signal(false);
  const alive = useLifetime(),
    busy = signal(false),
    menu = signal<Profile | undefined>(),
    editing = signal<{ id: string; revision: number } | undefined>(),
    editName = signal(""),
    editUrl = signal("");
  const exporting = signal<Profile | undefined>(),
    qr = signal<{ source: string; pngBase64: string } | undefined>(),
    refreshError = signal("");
  const refreshing = (profile: Profile) =>
    ui.client
      .pendingRequests()
      .some(
        (request) =>
          (request.command === "profiles.refresh" && request.payload.profileId === profile.id) ||
          (request.command === "profiles.refreshAll" && !!profile.subscriptionUrl),
      );
  let intent: number | undefined,
    dialogGeneration = 0;
  const cancelImport = () => {
    const current = intent;
    intent = undefined;
    if (current !== undefined) void ui.client.run("profiles.cancelImport", { intentId: current });
    busy.set(false);
  };
  const closeImport = () => {
    dialogGeneration++;
    open.set(false);
    cancelImport();
  };
  cleanup(cancelImport);
  effect(() => {
    const id = ui.importIntent();
    untracked(() => {
      if (ui.consumeImportIntent(id)) open.set(true);
    });
  });
  const profiles = computed(() =>
    ui.client.state()!.profiles.filter((profile) => matchesProfile(profile, query())),
  );
  const profileAction = (
    command: "profiles.activate" | "profiles.refresh" | "profiles.restore" | "profiles.delete",
    profileId: string,
  ) => {
    if (
      command === "profiles.refresh" &&
      (ui.client.pending() > 0 || !ui.client.domains.fresh(["config", "profiles"]))
    )
      return;
    const name =
      ui.client.state()!.profiles.find((profile) => profile.id === profileId)?.name || profileId;
    const message =
      command === "profiles.activate"
        ? ui.t(`配置 ${name} 已启用`, `Profile ${name} activated`)
        : command === "profiles.refresh"
          ? ui.t(`订阅 ${name} 已刷新`, `Subscription ${name} refreshed`)
          : command === "profiles.delete"
            ? ui.t(`配置 ${name} 已删除`, `Profile ${name} deleted`)
            : ui.t(`配置 ${name} 已恢复备份`, `Profile ${name} backup restored`);
    return ui.client.run(command, { profileId }, message);
  };
  const performImport = async (
    command: "profiles.importUrl" | "profiles.importContent",
    payload: Record<string, unknown>,
  ) => {
    const token = ui.client.nextImportIntent();
    intent = token;
    busy.set(true);
    ui.client.error.set("");
    try {
      await ui.client.send(command, { intentId: token, ...payload });
      if (alive() && intent === token) {
        open.set(false);
        name.set("");
        input.set("");
        ui.client.notice.set(ui.t("订阅已导入并启用", "Subscription imported and activated"));
      }
    } catch (error) {
      if (alive() && intent === token) ui.client.error.set(String(error));
    } finally {
      if (alive() && intent === token) {
        intent = undefined;
        busy.set(false);
      }
    }
  };
  const importContent = () =>
    performImport("profiles.importUrl", { name: name().trim(), url: input().trim() });
  const pick = async () => {
    const generation = dialogGeneration;
    busy.set(true);
    const value = await ui.client.run<{ name: string; content: string } | null>(
      "platform.importFile",
    );
    if (!alive() || !open() || generation !== dialogGeneration) return;
    busy.set(false);
    if (value)
      await performImport("profiles.importContent", { name: value.name, content: value.content });
  };
  const scan = async () => {
    const generation = dialogGeneration;
    busy.set(true);
    const value = await ui.client.run<string | null>("platform.scan");
    if (!alive() || !open() || generation !== dialogGeneration) return;
    if (!value) {
      busy.set(false);
      return;
    }
    const parsed = await ui.client.run<{ url: string; name?: string }>("profiles.parseScan", {
      payload: value,
    });
    if (!alive() || !open() || generation !== dialogGeneration) return;
    busy.set(false);
    if (parsed)
      await performImport("profiles.importUrl", {
        url: parsed.url,
        name: parsed.name || name().trim(),
      });
  };
  const saveSubscription = async () => {
    const target = editing();
    if (!target) return;
    const result = await ui.client.run(
      "profiles.subscription",
      { profileId: target.id, name: editName(), url: editUrl(), expectedRevision: target.revision },
      ui.t("订阅信息已保存", "Subscription details saved"),
    );
    if (alive() && editing() === target && result !== undefined) editing.set(undefined);
  };
  const exportYaml = async () => {
    const target = exporting();
    if (!target) return;
    const content = await ui.client.run<string>("profiles.read", { profileId: target.id });
    if (alive() && exporting() === target && content !== undefined)
      await ui.client.run(
        "platform.export",
        { name: target.name + ".yaml", content },
        ui.t(`${target.name} 已导出`, `${target.name} exported`),
      );
  };
  const generateQr = async () => {
    const target = exporting();
    if (!target) return;
    const text =
      target.subscriptionUrl ||
      (await ui.client.run<string>("profiles.read", { profileId: target.id }));
    if (!alive() || exporting() !== target || !text) return;
    const image = await ui.client.run<{ source: string; pngBase64: string }>("platform.qr", {
      text,
    });
    if (alive() && exporting() === target && image) qr.set(image);
  };
  const openEdit = (profile: Profile) => {
    editing.set({ id: profile.id, revision: ui.client.state()!.configRevision });
    editName.set(profile.name);
    editUrl.set(profile.subscriptionUrl || "");
    menu.set(undefined);
  };
  return (
    <View style={{ flex: 1 }}>
      <List
        bounces={false}
        each={profiles()}
        keyExtractor={(profile) => profile.id}
        estimatedItemSize={120}
        style={{ flex: 1 }}
        contentStyle={{ padding: windowSize().width >= 840 ? 24 : 16, gap: 12 }}
        header={
          ui.client.state()!.profiles.length ? (
            <View style={{ marginBottom: 12 }}>
              <Field
                label=""
                placeholder={ui.t(
                  "搜索名称、地址、当前或备份",
                  "Search name, URL, active or backup",
                )}
                value={query()}
                onChange={(text) => query.set(text)}
              />
            </View>
          ) : undefined
        }
        empty={
          <View style={{ gap: 16, alignItems: "center" }}>
            <View
              style={{
                width: "100%",
                borderWidth: 1,
                borderColor: palette().border,
                borderRadius: radius.lg,
              }}
            >
              <Empty
                icon={query() ? "search" : "rss"}
                title={
                  query()
                    ? ui.t("没有匹配的配置", "No matching profiles")
                    : ui.t("暂无配置", "No profiles")
                }
                text={
                  query()
                    ? ui.t("尝试其他名称或订阅地址", "Try another name or subscription URL")
                    : ui.t(
                        "添加订阅以获取代理节点与规则",
                        "Add a subscription to get proxy nodes and rules",
                      )
                }
              />
            </View>
            <If when={!query()}>
              <Action
                icon="plus"
                label={ui.t("添加订阅", "Add subscription")}
                onPress={() => open.set(true)}
              />
            </If>
          </View>
        }
      >
        {(profile) => (
          <View
            style={{
              height: 108,
              flexDirection: "row",
              alignItems: "center",
              borderWidth: 1,
              borderColor: palette().border,
              borderRadius: radius.lg,
              backgroundColor: profile().active ? palette().soft : palette().panel,
            }}
          >
            <View
              class="hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring"
              testID={`activate-profile-${profile().id}`}
              role="button"
              accessibilityLabel={profile().name}
              disabled={ui.client.pending() > 0 || !ui.client.domains.fresh(["config", "profiles"])}
              onPress={() => {
                if (!profile().active) void profileAction("profiles.activate", profile().id);
              }}
              style={{
                flex: 1,
                height: 106,
                paddingLeft: 16,
                flexDirection: "row",
                alignItems: "center",
                gap: 12,
              }}
            >
              <If
                when={refreshing(profile())}
                else={
                  <Icon
                    name={profile().active ? "circle-check" : "rss"}
                    size={20}
                    color={profile().active ? palette().success : palette().muted}
                  />
                }
              >
                <Spinner size={20} />
              </If>
              <View style={{ flex: 1, minWidth: 0, gap: 4 }}>
                <View style={{ flexDirection: "row", minWidth: 0, alignItems: "center", gap: 6 }}>
                  <Text
                    numberOfLines={1}
                    style={{
                      flex: 1,
                      minWidth: 0,
                      fontSize: 14,
                      lineHeight: 20,
                      color: palette().text,
                      fontWeight: "600",
                    }}
                  >
                    {profile().name}
                  </Text>
                  <If when={profile().active}>
                    <Badge tone="success" label={ui.t("当前", "Current")} />
                  </If>
                </View>
                <Text
                  numberOfLines={1}
                  style={{ fontSize: 12, lineHeight: 16, color: palette().muted }}
                >
                  {profile().subscriptionUrl || profile().source}
                </Text>
                <View style={{ flexDirection: "row", minWidth: 0, alignItems: "center", gap: 4 }}>
                  <If when={refreshing(profile())}>
                    <Badge tone="warning" label={ui.t("正在刷新…", "Refreshing…")} />
                  </If>
                  <If when={!refreshing(profile()) && profile().lastRefreshError}>
                    <Badge tone="danger" label={ui.t("刷新失败", "Refresh failed")} />
                  </If>
                  <If when={!refreshing(profile()) && !profile().lastRefreshError}>
                    <Icon name="clock" size={12} color={palette().muted} />
                  </If>
                  <Text
                    numberOfLines={1}
                    style={{
                      flex: 1,
                      minWidth: 0,
                      fontSize: 12,
                      lineHeight: 16,
                      color:
                        !refreshing(profile()) && profile().lastRefreshError
                          ? palette().danger
                          : palette().muted,
                    }}
                  >
                    {refreshing(profile())
                      ? ""
                      : profile().lastRefreshError ||
                        (profile().lastRefreshAt || profile().updatedAt
                          ? timestamp(profile().lastRefreshAt || profile().updatedAt)
                          : ui.t("尚未更新", "Not updated yet"))}
                  </Text>
                  <If
                    when={
                      !refreshing(profile()) &&
                      !profile().lastRefreshError &&
                      profile().subscriptionUserInfo?.totalBytes
                    }
                  >
                    <Text numberOfLines={1} style={{ fontSize: 12, color: palette().muted }}>
                      {bytes(
                        (profile().subscriptionUserInfo?.uploadBytes || 0) +
                          (profile().subscriptionUserInfo?.downloadBytes || 0),
                      )}{" "}
                      / {bytes(profile().subscriptionUserInfo?.totalBytes)}
                    </Text>
                  </If>
                </View>
              </View>
            </View>
            <View style={{ width: 48, alignItems: "center" }}>
              <IconButton
                id={`profile-menu-${profile().id}`}
                icon="ellipsis-vertical"
                label={ui.t("配置操作", "Profile actions")}
                disabled={refreshing(profile())}
                onPress={() => menu.set(profile())}
              />
            </View>
          </View>
        )}
      </List>
      <If when={open()}>
        <Dialog
          title={ui.t("网络导入", "Network import")}
          description={ui.t(
            "填写订阅地址，导入后会保存到本地。",
            "Enter a subscription URL. The imported profile is saved locally.",
          )}
          onClose={closeImport}
        >
          <View style={{ gap: 12, marginTop: 4 }}>
            <Field
              id="profile-input"
              label=""
              value={input()}
              onChange={(text) => {
                input.set(text);
                ui.client.error.set("");
              }}
              placeholder={ui.t("配置 URL", "Profile URL")}
              disabled={busy()}
            />
            <Field
              label=""
              value={name()}
              onChange={(text) => {
                name.set(text);
                ui.client.error.set("");
              }}
              placeholder={ui.t("留空时自动使用文件名", "Leave blank to use the filename")}
              disabled={busy()}
            />
          </View>
          <View
            style={{
              flexDirection: "row",
              alignItems: "center",
              justifyContent: "space-between",
              marginTop: -8,
            }}
          >
            <Action
              variant="ghost"
              small
              icon="file-up"
              label={ui.t("从本地文件导入", "Import a local file")}
              loading={busy()}
              disabled={busy()}
              onPress={() => void pick()}
            />
            <Action
              variant="ghost"
              small
              icon="scan-qr-code"
              label={ui.t("扫码添加", "Scan to add")}
              disabled={busy()}
              onPress={() => void scan()}
            />
          </View>
          <If when={ui.client.error()}>
            <PageError id="profile-import-error" text={ui.client.error()} />
          </If>
          <View
            style={{ flexDirection: "row", alignItems: "center", justifyContent: "space-between" }}
          >
            <Action secondary label={ui.t("取消", "Cancel")} onPress={closeImport} />
            <Action
              id="submit-import"
              icon="download"
              loading={busy()}
              label={
                busy()
                  ? ui.t("导入中...", "Importing...")
                  : ui.t("导入网络配置", "Import network profile")
              }
              disabled={busy() || !input().trim()}
              onPress={() => void importContent()}
            />
          </View>
        </Dialog>
      </If>
      <If when={menu()}>
        <Dialog
          title={menu()!.name}
          description={ui.t("配置操作", "Profile actions")}
          onClose={() => menu.set(undefined)}
        >
          <If when={menu()!.lastRefreshError}>
            <Action
              id="profile-refresh-error"
              secondary
              small
              label={ui.t("查看刷新错误", "View refresh error")}
              onPress={() => {
                refreshError.set(menu()!.lastRefreshError!);
                menu.set(undefined);
              }}
            />
          </If>
          <View
            style={{
              borderWidth: 1,
              borderColor: palette().border,
              borderRadius: radius.lg,
              overflow: "hidden",
            }}
          >
            <If when={!menu()!.active}>
              <MenuRow
                disabled={!ui.client.domains.fresh(["config", "profiles"])}
                icon="circle-check"
                label={ui.t("设为当前配置", "Set as current profile")}
                onPress={() => {
                  const profileId = menu()!.id;
                  menu.set(undefined);
                  void profileAction("profiles.activate", profileId);
                }}
              />
            </If>
            <If when={menu()!.subscriptionUrl}>
              <MenuRow
                disabled={!ui.client.domains.fresh(["config", "profiles"])}
                id={`edit-subscription-${menu()!.id}`}
                icon="file-pen-line"
                label={ui.t("编辑订阅", "Edit subscription")}
                onPress={() => openEdit(menu()!)}
              />
            </If>
            <MenuRow
              disabled={!ui.client.domains.fresh(["config", "profiles"])}
              id={`edit-yaml-${menu()!.id}`}
              icon="file-pen-line"
              label={ui.t("编辑 YAML", "Edit YAML")}
              onPress={() => {
                const id = menu()!.id;
                menu.set(undefined);
                ui.editProfile(id);
              }}
            />
            <MenuRow
              disabled={!ui.client.domains.fresh(["config", "profiles"])}
              id={`profile-export-${menu()!.id}`}
              icon="download"
              label={ui.t("导出配置", "Export profile")}
              onPress={() => {
                exporting.set(menu());
                qr.set(undefined);
                menu.set(undefined);
              }}
            />
            <If when={menu()!.subscriptionUrl}>
              <MenuRow
                id={`refresh-profile-${menu()!.id}`}
                loading={refreshing(menu()!)}
                disabled={
                  ui.client.pending() > 0 || !ui.client.domains.fresh(["config", "profiles"])
                }
                icon="refresh-cw"
                label={ui.t("刷新订阅", "Refresh subscription")}
                onPress={() => {
                  const profileId = menu()!.id;
                  menu.set(undefined);
                  void profileAction("profiles.refresh", profileId);
                }}
              />
            </If>
            <If when={menu()!.hasBackup}>
              <MenuRow
                disabled={!ui.client.domains.fresh(["config", "profiles"])}
                icon="history"
                label={ui.t("恢复上次备份", "Restore previous backup")}
                onPress={() => {
                  const profileId = menu()!.id;
                  menu.set(undefined);
                  void confirm(
                    ui,
                    ui.t("恢复此配置的备份？", "Restore this profile's backup?"),
                    () => void profileAction("profiles.restore", profileId),
                  );
                }}
              />
            </If>
            <MenuRow
              disabled={!ui.client.domains.fresh(["config", "profiles"])}
              icon="trash-2"
              danger
              label={ui.t("删除配置", "Delete profile")}
              onPress={() => {
                const profileId = menu()!.id;
                menu.set(undefined);
                void confirm(
                  ui,
                  ui.t("删除此配置？", "Delete this profile?"),
                  () => void profileAction("profiles.delete", profileId),
                );
              }}
            />
          </View>
        </Dialog>
      </If>
      <If when={refreshError()}>
        <Dialog title={ui.t("刷新失败", "Refresh failed")} onClose={() => refreshError.set("")}>
          <Scroll
            bounces={false}
            style={{ height: Math.min(280, Math.max(120, windowSize().height * 0.3)) }}
          >
            <PageError id="profile-refresh-failure" text={refreshError()} />
          </Scroll>
          <Actions>
            <Action secondary label={ui.t("关闭", "Close")} onPress={() => refreshError.set("")} />
            <Action
              label={ui.t("复制错误信息", "Copy error")}
              onPress={() => Clipboard.writeText(refreshError())}
            />
          </Actions>
        </Dialog>
      </If>
      <If when={editing()}>
        <Dialog
          title={ui.t("编辑订阅", "Edit subscription")}
          description={ui.t(
            "修改名称或订阅地址，保存后可手动刷新。",
            "Change the name or subscription URL. Refresh manually after saving.",
          )}
          onClose={() => editing.set(undefined)}
        >
          <Field
            label={ui.t("名称", "Name")}
            value={editName()}
            placeholder={ui.t("配置名称", "Profile name")}
            onChange={(text) => editName.set(text)}
          />
          <Field
            label={ui.t("订阅地址", "Subscription URL")}
            value={editUrl()}
            placeholder="https://"
            onChange={(text) => editUrl.set(text)}
          />
          <If when={editing()!.revision !== ui.client.state()!.configRevision}>
            <Hint
              danger
              text={ui.t(
                "配置已变更，请关闭并重新打开此编辑器。",
                "Configuration changed. Close and reopen this editor.",
              )}
            />
          </If>
          <Actions>
            <Action
              secondary
              label={ui.t("取消", "Cancel")}
              onPress={() => editing.set(undefined)}
            />
            <Action
              id="save-subscription"
              label={ui.t("保存修改", "Save changes")}
              disabled={
                !editName().trim() ||
                ui.client.pending() > 0 ||
                !ui.client.domains.fresh(["config", "profiles"]) ||
                editing()!.revision !== ui.client.state()!.configRevision
              }
              onPress={() => void saveSubscription()}
            />
          </Actions>
        </Dialog>
      </If>
      <If when={exporting()}>
        <Dialog
          title={ui.t("导出配置", "Export profile")}
          description={exporting()!.name}
          onClose={() => {
            exporting.set(undefined);
            qr.set(undefined);
          }}
        >
          <If
            when={!qr()}
            else={
              <View style={{ gap: 16, alignItems: "center" }}>
                <Hint
                  text={
                    exporting()!.subscriptionUrl
                      ? ui.t("扫描此二维码导入订阅地址", "Scan to import the subscription URL")
                      : ui.t("扫描此二维码导入配置内容", "Scan to import the profile")
                  }
                />
                <Image src={qr()!.source} style={{ width: 260, height: 260 }} />
                <Actions>
                  <Action
                    secondary
                    label={ui.t("返回", "Back")}
                    onPress={() => qr.set(undefined)}
                  />
                  <Action
                    id="export-profile-qr"
                    label={ui.t("保存二维码图片", "Save QR image")}
                    onPress={() =>
                      void ui.client.run(
                        "platform.exportQr",
                        { name: exporting()!.name, pngBase64: qr()!.pngBase64 },
                        ui.t("二维码图片已保存", "QR image saved"),
                      )
                    }
                  />
                </Actions>
              </View>
            }
          >
            <View
              style={{
                borderWidth: 1,
                borderColor: palette().border,
                borderRadius: radius.lg,
                overflow: "hidden",
              }}
            >
              <MenuRow
                disabled={!ui.client.domains.fresh(["config", "profiles"])}
                icon="download"
                label={ui.t("导出 YAML 文件", "Export YAML file")}
                onPress={() => void exportYaml()}
              />
              <MenuRow
                disabled={!ui.client.domains.fresh(["config", "profiles"])}
                id={`profile-qr-${exporting()!.id}`}
                icon="qr-code"
                label={ui.t("导出为二维码", "Export as QR code")}
                onPress={() => void generateQr()}
              />
            </View>
          </If>
        </Dialog>
      </If>
    </View>
  );
}
function MenuRow(props: {
  icon: Parameters<typeof Icon>[0]["name"];
  label: string;
  onPress: () => void;
  danger?: boolean;
  id?: string;
  disabled?: boolean;
  loading?: boolean;
}) {
  const disabled = () => !!props.disabled || !!props.loading;
  return (
    <>
      <View
        class="hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring"
        testID={props.id}
        role="button"
        accessibilityLabel={props.label}
        accessibilityState={{ disabled: disabled(), busy: !!props.loading }}
        disabled={disabled()}
        onPress={() => {
          if (!disabled()) props.onPress();
        }}
        activeStyle={{ opacity: 0.65 }}
        style={{
          height: 48,
          paddingHorizontal: 14,
          flexDirection: "row",
          alignItems: "center",
          gap: 10,
          opacity: disabled() ? 0.5 : 1,
          backgroundColor: palette().panel,
        }}
      >
        <If
          when={props.loading}
          else={
            <Icon
              name={props.icon}
              size={16}
              color={props.danger ? palette().danger : palette().text}
            />
          }
        >
          <Spinner size={16} />
        </If>
        <Text
          style={{
            color: props.danger ? palette().danger : palette().text,
            fontSize: 14,
            fontWeight: "600",
          }}
        >
          {props.label}
        </Text>
      </View>
      <Separator />
    </>
  );
}

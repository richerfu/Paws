import { density } from "../theme";
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
  fontScale,
  platform,
} from "nsh";
import {
  Action,
  Actions,
  confirm,
  Dialog,
  Empty,
  Field,
  Hint,
  Icon,
  IconButton,
  palette,
  radius,
  control,
  spacing,
  Separator,
  Spinner,
  useApp,
  useLifetime,
} from "../components";
import { ContextMenu, Menu, MenuItem, MenuSeparator } from "nsh/base";
import { feedbackStyle } from "../motion";
import { focusRing, popupSurface } from "../theme";
import { bytes, timestamp } from "../locale";

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
          (request.command === "profiles.refresh" &&
            request.payload.profileId === profile.id) ||
          (request.command === "profiles.refreshAll" &&
            !!profile.subscriptionUrl),
      );
  let intent: number | undefined,
    dialogGeneration = 0;
  const cancelImport = () => {
    const current = intent;
    intent = undefined;
    if (current !== undefined)
      void ui.client.run("profiles.cancelImport", { intentId: current });
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
    ui.client
      .state()!
      .profiles.filter((profile) => matchesProfile(profile, query())),
  );
  const profileAction = (
    command:
      | "profiles.activate"
      | "profiles.refresh"
      | "profiles.restore"
      | "profiles.delete",
    profileId: string,
  ) => {
    if (
      command === "profiles.refresh" &&
      (ui.client.pending() > 0 ||
        !ui.client.domains.readyForCommand(["config", "profiles"]))
    )
      return;
    const name =
      ui.client.state()!.profiles.find((profile) => profile.id === profileId)
        ?.name || profileId;
    const message =
      command === "profiles.activate"
        ? ui.t(`配置 ${name} 已启用`, `Profile ${name} activated`)
        : command === "profiles.refresh"
          ? ui.t(`订阅 ${name} 已刷新`, `Subscription ${name} refreshed`)
          : command === "profiles.delete"
            ? ui.t(`配置 ${name} 已删除`, `Profile ${name} deleted`)
            : ui.t(
                `配置 ${name} 已恢复备份`,
                `Profile ${name} backup restored`,
              );
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
        ui.client.notice.set(
          ui.t("订阅已导入并启用", "Subscription imported and activated"),
        );
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
  const importContent = () => {
    if (busy() || !input().trim()) return;
    return performImport("profiles.importUrl", {
      name: name().trim(),
      url: input().trim(),
    });
  };
  const pick = async () => {
    const generation = dialogGeneration;
    busy.set(true);
    const value = await ui.client.run<{ name: string; content: string } | null>(
      "platform.importFile",
    );
    if (!alive() || !open() || generation !== dialogGeneration) return;
    busy.set(false);
    if (value)
      await performImport("profiles.importContent", {
        name: value.name,
        content: value.content,
      });
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
    const parsed = await ui.client.run<{ url: string; name?: string }>(
      "profiles.parseScan",
      {
        payload: value,
      },
    );
    if (!alive() || !open() || generation !== dialogGeneration) return;
    busy.set(false);
    if (parsed)
      await performImport("profiles.importUrl", {
        url: parsed.url,
        name: parsed.name || name().trim(),
      });
  };
  const canSaveSubscription = () =>
    !!editing() &&
    !!editName().trim() &&
    ui.client.pending() === 0 &&
    ui.client.domains.readyForCommand(["config", "profiles"]) &&
    editing()!.revision === ui.client.state()!.configRevision;
  const saveSubscription = async () => {
    const target = editing();
    if (!target || !canSaveSubscription()) return;
    const result = await ui.client.run(
      "profiles.subscription",
      {
        profileId: target.id,
        name: editName(),
        url: editUrl(),
        expectedRevision: target.revision,
      },
      ui.t("订阅信息已保存", "Subscription details saved"),
    );
    if (alive() && editing() === target && result !== undefined)
      editing.set(undefined);
  };
  const exportYaml = async () => {
    const target = exporting();
    if (!target) return;
    const content = await ui.client.run<string>("profiles.read", {
      profileId: target.id,
    });
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
    const image = await ui.client.run<{ source: string; pngBase64: string }>(
      "platform.qr",
      {
        text,
      },
    );
    if (alive() && exporting() === target && image) qr.set(image);
  };
  const openEdit = (profile: Profile) => {
    editing.set({
      id: profile.id,
      revision: ui.client.state()!.configRevision,
    });
    editName.set(profile.name);
    editUrl.set(profile.subscriptionUrl || "");
  };
  return (
    <View style={{ flex: 1 }}>
      <View
        style={{
          paddingHorizontal:
            windowSize().width >= 840 ? spacing.xxl : spacing.lg,
          paddingTop: spacing.lg,
          paddingBottom: spacing.md,
        }}
      >
        <Field
          label={ui.t("搜索订阅", "Search subscriptions")}
          placeholder={ui.t(
            "名称、地址、当前或备份",
            "Name, URL, current or backup",
          )}
          value={query()}
          onChange={(text) => query.set(text)}
        />
      </View>
      <List
        bounces={false}
        each={profiles()}
        keyExtractor={(profile) => profile.id}
        estimatedItemSize={120}
        style={{
          flex: 1,
          margin: windowSize().width >= 840 ? spacing.xxl : spacing.lg,
          marginTop: 0,
          borderWidth: 1,
          borderColor: palette().border,
          borderRadius: radius.xl,
          backgroundColor: palette().panel,
          overflow: "hidden",
        }}
        contentStyle={{ padding: 0 }}
        empty={
          <View style={{ gap: density.space(16), alignItems: "center" }}>
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
                    ? ui.t(
                        "尝试其他名称或订阅地址",
                        "Try another name or subscription URL",
                      )
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
          <ProfileRow
            profile={profile()}
            refreshing={refreshing(profile())}
            onAction={(command) => {
              void profileAction(command, profile().id);
            }}
            onEdit={() => openEdit(profile())}
            onExport={() => {
              exporting.set(profile());
              qr.set(undefined);
            }}
            onError={() => refreshError.set(profile().lastRefreshError || "")}
          />
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
          footer={
            <Actions>
              <Action
                secondary
                label={ui.t("取消", "Cancel")}
                onPress={closeImport}
              />
              <Action
                primary
                id="submit-import"
                icon="download"
                loading={busy()}
                label={ui.t("导入", "Import")}
                disabled={busy() || !input().trim()}
                onPress={() => void importContent()}
              />
            </Actions>
          }
        >
          <View style={{ gap: density.space(12), marginTop: density.space(4) }}>
            <Field
              onSubmit={() => void importContent()}
              id="profile-input"
              label={ui.t("订阅地址", "Subscription URL")}
              value={input()}
              onChange={(text) => {
                input.set(text);
                ui.client.error.set("");
              }}
              placeholder={ui.t("配置 URL", "Profile URL")}
              disabled={busy()}
            />
            <Field
              label={ui.t("名称（可选）", "Name (optional)")}
              value={name()}
              onChange={(text) => {
                name.set(text);
                ui.client.error.set("");
              }}
              placeholder={ui.t(
                "留空时自动使用文件名",
                "Leave blank to use the filename",
              )}
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
        </Dialog>
      </If>
      <If when={refreshError()}>
        <Dialog
          title={ui.t("刷新失败", "Refresh failed")}
          onClose={() => refreshError.set("")}
        >
          <Scroll
            bounces={false}
            style={{
              height: Math.min(280, Math.max(120, windowSize().height * 0.3)),
            }}
          >
            <PageError id="profile-refresh-failure" text={refreshError()} />
          </Scroll>
          <Actions>
            <Action
              secondary
              label={ui.t("关闭", "Close")}
              onPress={() => refreshError.set("")}
            />
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
          footer={
            <Actions>
              <Action
                secondary
                label={ui.t("取消", "Cancel")}
                onPress={() => editing.set(undefined)}
              />
              <Action
                primary
                id="save-subscription"
                label={ui.t("保存修改", "Save changes")}
                loading={ui.client.isPending("profiles.subscription")}
                disabled={!canSaveSubscription()}
                onPress={() => void saveSubscription()}
              />
            </Actions>
          }
        >
          <Field
            label={ui.t("名称", "Name")}
            onSubmit={() => void saveSubscription()}
            value={editName()}
            placeholder={ui.t("配置名称", "Profile name")}
            onChange={(text) => editName.set(text)}
          />
          <Field
            label={ui.t("订阅地址", "Subscription URL")}
            onSubmit={() => void saveSubscription()}
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
              <View style={{ gap: density.space(16), alignItems: "center" }}>
                <Hint
                  text={
                    exporting()!.subscriptionUrl
                      ? ui.t(
                          "扫描此二维码导入订阅地址",
                          "Scan to import the subscription URL",
                        )
                      : ui.t(
                          "扫描此二维码导入配置内容",
                          "Scan to import the profile",
                        )
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
              <ExportChoice
                disabled={
                  !ui.client.domains.readyForCommand(["config", "profiles"])
                }
                icon="download"
                label={ui.t("导出 YAML 文件", "Export YAML file")}
                onPress={() => void exportYaml()}
              />
              <ExportChoice
                disabled={
                  !ui.client.domains.readyForCommand(["config", "profiles"])
                }
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
type ProfileAction =
  | "profiles.activate"
  | "profiles.refresh"
  | "profiles.restore"
  | "profiles.delete";

/** One object and the same commands for a visible menu, right click, and a held finger. */
function ProfileRow(props: {
  profile: Profile;
  refreshing: boolean;
  onAction: (command: ProfileAction) => void;
  onEdit: () => void;
  onExport: () => void;
  onError: () => void;
}) {
  const ui = useApp(),
    alive = useLifetime(),
    route = ui.route();
  const open = signal(false),
    contextual = signal(false);
  let pending: { source: "menu" | "context"; action: () => void } | undefined;
  const ready = () => ui.client.domains.readyForCommand(["config", "profiles"]);
  const height = () =>
    Math.max(
      control.compact ? (windowSize().width >= 840 ? 76 : 88) : 88,
      density.line(52) * Math.max(1, fontScale()) + spacing.xl,
    );
  const afterClose = (source: "menu" | "context") => {
    if (pending?.source !== source) return;
    const action = pending.action;
    pending = undefined;
    if (alive() && ui.route() === route) action();
  };
  const choose = (source: "menu" | "context", action: () => void) => {
    pending = { source, action };
    open.set(false);
    contextual.set(false);
  };
  const updated = () =>
    props.refreshing
      ? ui.t("正在更新", "Updating")
      : props.profile.lastRefreshError
        ? ui.t("更新失败", "Update failed")
        : props.profile.lastRefreshAt || props.profile.updatedAt
          ? timestamp(props.profile.lastRefreshAt || props.profile.updatedAt)
          : ui.t("尚未更新", "Not updated");
  const usage = () =>
    props.profile.subscriptionUserInfo?.totalBytes
      ? `${bytes((props.profile.subscriptionUserInfo.uploadBytes || 0) + (props.profile.subscriptionUserInfo.downloadBytes || 0))} / ${bytes(props.profile.subscriptionUserInfo.totalBytes)}`
      : "";
  const Command = (item: {
    source: "menu" | "context";
    label: string;
    icon: Parameters<typeof Icon>[0]["name"];
    id?: string;
    danger?: boolean;
    disabled?: boolean;
    action: () => void;
  }) => (
    <MenuItem
      testID={item.id}
      accessibilityLabel={item.label}
      disabled={!ready() || !!item.disabled}
      onSelect={() => choose(item.source, item.action)}
      focusStyle={focusRing()}
      hoverStyle={{ backgroundColor: palette().soft }}
      activeStyle={{ backgroundColor: palette().soft }}
      style={{
        minHeight: control.row,
        paddingHorizontal: spacing.md,
        paddingVertical: spacing.sm,
        borderRadius: radius.sm,
        flexDirection: "row",
        alignItems: "center",
        gap: spacing.md,
        opacity: !ready() || item.disabled ? 0.4 : 1,
        ...feedbackStyle(),
      }}
    >
      <Icon
        name={item.icon}
        size={18}
        color={item.danger ? palette().danger : palette().muted}
      />
      <Text
        style={{
          flex: 1,
          minWidth: 0,
          fontSize: density.font(14),
          lineHeight: density.line(20),
          color: item.danger ? palette().danger : palette().text,
        }}
      >
        {item.label}
      </Text>
    </MenuItem>
  );
  const Commands = (value: { source: "menu" | "context" }) => (
    <View>
      <View
        style={{
          flexDirection: "row",
          alignItems: "center",
          paddingLeft: spacing.md,
          gap: spacing.sm,
        }}
      >
        <Text
          role="header"
          accessibilityLabel={props.profile.name}
          style={{
            flex: 1,
            minWidth: 0,
            numberOfLines: 2,
            fontSize: density.font(13),
            lineHeight: density.line(20),
            fontWeight: "500",
            color: palette().muted,
          }}
        >
          {props.profile.name}
        </Text>
        <IconButton
          id="profile-menu-close"
          icon="x"
          label={ui.t("关闭菜单", "Close menu")}
          onPress={() => {
            open.set(false);
            contextual.set(false);
          }}
        />
      </View>
      <If when={props.profile.lastRefreshError}>
        <Command
          source={value.source}
          id="profile-refresh-error"
          icon="triangle-alert"
          label={ui.t("更新错误…", "Update error…")}
          action={props.onError}
        />
        <MenuSeparator
          style={{
            height: 1,
            marginVertical: spacing.xxs,
            backgroundColor: palette().line,
          }}
        />
      </If>
      <If when={!props.profile.active}>
        <Command
          source={value.source}
          icon="circle-check"
          label={ui.t("设为当前订阅", "Use this subscription")}
          action={() => props.onAction("profiles.activate")}
        />
      </If>
      <If when={props.profile.subscriptionUrl}>
        <Command
          source={value.source}
          id={`edit-subscription-${props.profile.id}`}
          icon="file-pen-line"
          label={ui.t("编辑订阅…", "Edit subscription…")}
          action={props.onEdit}
        />
      </If>
      <Command
        source={value.source}
        id={`edit-yaml-${props.profile.id}`}
        icon="file-pen-line"
        label={ui.t("编辑 YAML…", "Edit YAML…")}
        action={() => ui.editProfile(props.profile.id)}
      />
      <Command
        source={value.source}
        id={`profile-export-${props.profile.id}`}
        icon="download"
        label={ui.t("导出…", "Export…")}
        action={props.onExport}
      />
      <If when={props.profile.subscriptionUrl}>
        <Command
          source={value.source}
          id={`refresh-profile-${props.profile.id}`}
          icon="refresh-cw"
          label={ui.t("更新订阅", "Update subscription")}
          disabled={ui.client.pending() > 0 || props.refreshing}
          action={() => props.onAction("profiles.refresh")}
        />
      </If>
      <If when={props.profile.hasBackup}>
        <Command
          source={value.source}
          icon="history"
          label={ui.t("恢复上次备份…", "Restore previous backup…")}
          action={() => {
            void confirm(
              ui,
              ui.t(
                `恢复“${props.profile.name}”的备份？`,
                `Restore “${props.profile.name}”?`,
              ),
              () => props.onAction("profiles.restore"),
              {
                actionLabel: ui.t("恢复备份", "Restore backup"),
                description: ui.t(
                  "当前配置会被上次备份替换。",
                  "This replaces the profile with its previous backup.",
                ),
              },
            );
          }}
        />
      </If>
      <MenuSeparator
        style={{
          height: 1,
          marginVertical: spacing.xxs,
          backgroundColor: palette().line,
        }}
      />
      <Command
        source={value.source}
        icon="trash-2"
        danger
        label={ui.t("删除…", "Delete…")}
        action={() => {
          void confirm(
            ui,
            ui.t(
              `删除“${props.profile.name}”？`,
              `Delete “${props.profile.name}”?`,
            ),
            () => props.onAction("profiles.delete"),
            { actionLabel: ui.t("删除", "Delete") },
          );
        }}
      />
    </View>
  );
  const trigger = (
    <View
      style={{
        height: height(),
        flexDirection: "row",
        alignItems: "center",
        borderBottomWidth: 1,
        borderBottomColor: palette().line,
        backgroundColor:
          props.profile.active || contextual()
            ? palette().soft
            : palette().panel,
      }}
    >
      <View
        testID={`activate-profile-${props.profile.id}`}
        role="button"
        accessibilityLabel={props.profile.name}
        accessibilityState={{
          selected: props.profile.active,
          disabled: ui.client.pending() > 0 || !ready(),
        }}
        disabled={ui.client.pending() > 0 || !ready()}
        focusStyle={focusRing()}
        hoverStyle={{ backgroundColor: palette().soft }}
        activeStyle={{ opacity: 0.8 }}
        onPress={() => {
          if (!props.profile.active) props.onAction("profiles.activate");
        }}
        style={{
          flex: 1,
          height: height() - 1,
          paddingHorizontal: spacing.lg,
          flexDirection: "row",
          alignItems: "center",
          gap: spacing.md,
          ...feedbackStyle(),
        }}
      >
        <If
          when={props.refreshing}
          else={
            <Icon
              name={props.profile.active ? "circle-check" : "rss"}
              size={20}
              color={props.profile.active ? palette().accent : palette().muted}
            />
          }
        >
          <Spinner size={20} />
        </If>
        <View style={{ flex: 1, minWidth: 0, gap: spacing.xxs }}>
          <View
            style={{
              flexDirection: "row",
              gap: spacing.sm,
              alignItems: "center",
            }}
          >
            <Text
              style={{
                flex: 1,
                minWidth: 0,
                numberOfLines: 1,
                fontSize: density.font(14),
                lineHeight: density.line(20),
                fontWeight: "600",
                color: palette().text,
              }}
            >
              {props.profile.name}
            </Text>
            <If when={props.profile.active}>
              <Text
                style={{
                  flexShrink: 0,
                  fontSize: density.font(12),
                  color: palette().muted,
                }}
              >
                {ui.t("当前", "Current")}
              </Text>
            </If>
          </View>
          <Text
            style={{
              numberOfLines: 1,
              fontSize: density.font(12),
              lineHeight: density.line(18),
              color: palette().muted,
            }}
          >
            {props.profile.subscriptionUrl || props.profile.source}
          </Text>
          <If when={windowSize().width < 840}>
            <Text
              style={{
                numberOfLines: 1,
                fontSize: density.font(12),
                lineHeight: density.line(18),
                color: props.profile.lastRefreshError
                  ? palette().danger
                  : palette().muted,
              }}
            >
              {updated()}
              {usage() ? ` · ${usage()}` : ""}
            </Text>
          </If>
        </View>
        <If when={windowSize().width >= 840}>
          <View
            style={{ width: 160, gap: spacing.xxs, alignItems: "flex-end" }}
          >
            <Text
              style={{
                fontSize: density.font(12),
                lineHeight: density.line(18),
                color: props.profile.lastRefreshError
                  ? palette().danger
                  : palette().muted,
              }}
            >
              {updated()}
            </Text>
            <Text
              style={{
                fontSize: density.font(12),
                lineHeight: density.line(18),
                color: palette().muted,
                fontVariant: ["tabular-nums"],
              }}
            >
              {usage()}
            </Text>
          </View>
        </If>
      </View>
      <Menu
        open={open()}
        onOpenChange={(event) => open.set(event.open && !props.refreshing)}
        onClose={() => afterClose("menu")}
        popupStyle={popupSurface()}
        align="end"
        offset={4}
        margin={8}
        popupStartingStyle={{ opacity: 0, transform: [{ translateY: -4 }] }}
        popupEndingStyle={{ opacity: 0 }}
        backdropStyle={{ backgroundColor: palette().scrim }}
        trigger={
          <View
            testID={`profile-menu-${props.profile.id}`}
            role="button"
            hitSlop={control.hitSlop(control.icon)}
            accessibilityLabel={ui.t("订阅操作", "Subscription actions")}
            accessibilityState={{ expanded: open() }}
            disabled={props.refreshing}
            focusStyle={focusRing()}
            hoverStyle={{ backgroundColor: palette().soft }}
            activeStyle={{ opacity: 0.8 }}
            style={{
              width: control.icon,
              height: control.icon,
              marginRight: spacing.sm,
              borderRadius: radius.md,
              alignItems: "center",
              justifyContent: "center",
              backgroundColor: open() ? palette().soft : "transparent",
            }}
          >
            <Icon name="ellipsis-vertical" size={18} color={palette().muted} />
          </View>
        }
      >
        <Commands source="menu" />
      </Menu>
    </View>
  );
  return (
    <ContextMenu
      disabled={props.refreshing}
      open={contextual()}
      onOpenChange={(event) => contextual.set(event.open)}
      onClose={() => afterClose("context")}
      popupStyle={popupSurface()}
      backdropStyle={{ backgroundColor: palette().scrim }}
      trigger={trigger}
    >
      <Commands source="context" />
    </ContextMenu>
  );
}

function ExportChoice(props: {
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
          minHeight: control.row,
          paddingVertical: spacing.sm,
          paddingHorizontal: spacing.md,
          flexDirection: "row",
          alignItems: "center",
          gap: density.space(10),
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
            fontSize: density.font(14),
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

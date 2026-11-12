import { density, typography } from "../theme";
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
  windowSize,
  List,
  fontScale,
  platform,
} from "nsh";
import {
  Action,
  Actions,
  Badge,
  BottomSheet,
  ChoiceTile,
  confirm,
  copyText,
  Dialog,
  Detail,
  Body,
  Empty,
  Field,
  Hint,
  Icon,
  IconButton,
  IconTile,
  ListSurface,
  MeterBar,
  OrRule,
  palette,
  radius,
  SearchField,
  SectionHeader,
  control,
  spacing,
  Separator,
  Spinner,
  useApp,
  useLifetime,
  useLayout,
  useViewState,
  useDraftGuard,
  useOverlay,
  RouteOwner,
} from "../components";
import { ContextMenu, Menu, MenuItem, MenuSeparator } from "nsh/base";
import { feedbackStyle } from "../motion";
import { focusRing, popupSurface, surfaceFinish } from "../theme";
import { bytes, timestamp } from "../locale";

import { matchesProfile } from "../query";
import { PageError } from "../page-error";
import type { Profile } from "../model";

export function Profiles(props: { profileId?: string } = {}) {
  const ui = useApp(),
    layout = useLayout(),
    name = signal(""),
    input = signal(""),
    open = signal(false);
  const collection = useViewState("profiles.collection", () => ({ query: signal(""), selected: signal(""), offset: 0 }));
  const query = collection.query;
  if (props.profileId) collection.selected.set(props.profileId);
  const selectedProfile = () => ui.client.state()!.profiles.find((profile) => profile.id === collection.selected());
  const owner = RouteOwner() || ui.route();
  const selectProfile = (profile: Profile) => {
    collection.selected.set(profile.id);
    if (!props.profileId) ui.navigate("profile-detail", profile.id);
  };
  const alive = useLifetime(),
    busy = signal(false),
    importOp = signal<"" | "url" | "file" | "scan">(""),
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
    importOp.set("");
  };
  const importGuard = useDraftGuard(() => open() && !!(name() || input()), () => { name.set(""); input.set(""); });
  const closeImport = async () => {
    if (!(await importGuard())) return;
    dialogGeneration++;
    open.set(false);
    cancelImport();
  };
  const initialEdit = signal({ name: "", url: "" });
  const guard = useDraftGuard(
    () => !!editing() && (editName() !== initialEdit().name || editUrl() !== initialEdit().url),
    () => editing.set(undefined),
    () => ui.client.isPending("profiles.subscription"),
  );
  const closeEdit = async () => { if (await guard()) editing.set(undefined); };
  cleanup(cancelImport);
  effect(() => {
    const id = ui.importIntent();
    if (ui.route() !== owner) return;
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
        importOp.set("");
      }
    }
  };
  const importContent = () => {
    if (busy() || !input().trim()) return;
    importOp.set("url");
    return performImport("profiles.importUrl", {
      name: name().trim(),
      url: input().trim(),
    });
  };
  const pick = async () => {
    const generation = dialogGeneration;
    busy.set(true);
    importOp.set("file");
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
    else importOp.set("");
  };
  const scan = async () => {
    const generation = dialogGeneration;
    busy.set(true);
    importOp.set("scan");
    const value = await ui.client.run<string | null>("platform.scan");
    if (!alive() || !open() || generation !== dialogGeneration) return;
    if (!value) {
      busy.set(false);
      importOp.set("");
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
    else importOp.set("");
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
    initialEdit.set({ name: profile.name, url: profile.subscriptionUrl || "" });
  };
  const row = (profile: Profile, detail = false) => <ProfileRow
    detail={detail}
    profile={profile} refreshing={refreshing(profile)}
    selected={collection.selected() === profile.id}
    compact={layout().masterDetail}
    onDetails={() => selectProfile(profile)}
    onAction={(command) => { void profileAction(command, profile.id); }}
    onEdit={() => openEdit(profile)}
    onExport={() => { exporting.set(profile); qr.set(undefined); }}
    onError={() => refreshError.set(profile.lastRefreshError || "")}
  />;
  return (
    <View style={{ flex: 1 }}>
      <View style={{ flex: 1, minHeight: 0, flexDirection: "row", gap: layout().masterDetail ? spacing.lg : 0 }}>
      <View testID="profiles-collection" style={{
        display: props.profileId && !layout().masterDetail ? "none" : "flex",
        flexDirection: "column",
        width: layout().masterDetail ? layout().collection : undefined,
        flex: layout().masterDetail ? undefined : 1, minWidth: 0,
      }}>
      <View
        style={{
          paddingHorizontal:
            windowSize().width >= 840 ? spacing.xxl : spacing.lg,
          paddingTop: spacing.lg,
          paddingBottom: spacing.md,
        }}
      >
        <SearchField
          id="profiles-search"
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
        testID="profiles-list"
        contentOffset={{ y: collection.offset }}
        onScroll={(event) => { if (ui.route() === owner) collection.offset = event.offset.y; }}
        bounces={false}
        keyboardDismissMode="on-drag"
        keyboardShouldPersistTaps="handled"
        refreshing={ui.client.isPending("profiles.refreshAll")}
        onRefresh={
          ui.refreshProfiles ? () => void ui.refreshProfiles!() : undefined
        }
        each={profiles()}
        keyExtractor={(profile) => profile.id}
        estimatedItemSize={120}
        style={{
          flex: 1,
          margin: spacing.lg,
          marginTop: 0,
          borderWidth: 1,
          borderColor: palette().border,
          borderRadius: radius.xl,
          borderCurve: "continuous",
          backgroundColor: palette().panel,
          overflow: "hidden",
          ...surfaceFinish("card"),
        }}
        contentStyle={{ padding: 0 }}
        empty={
          <View
            style={{
              padding: spacing.lg,
              gap: density.space(16),
              alignItems: "center",
            }}
          >
            <View style={{ width: "100%" }}>
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
                primary
                icon="plus"
                label={ui.t("添加订阅", "Add subscription")}
                onPress={() => open.set(true)}
              />
            </If>
            <If when={!!query()}>
              <Action label={ui.t("清除搜索", "Clear search")} onPress={() => query.set("")} />
            </If>
          </View>
        }
      >
        {(profile) => row(profile())}
      </List>
      </View>
      <View testID="profile-detail-panel" style={{ display: props.profileId || layout().masterDetail ? "flex" : "none", flex: 1, minWidth: 0, flexDirection: "column" }}>
        <Scroll bounces={false} style={{ flex: 1 }}>
          <Body>
            <If when={selectedProfile()} else={<Empty icon="rss" text={collection.selected() ? ui.t("此订阅已移除。返回列表选择其他订阅。", "This subscription was removed. Return to the list.") : ui.t("选择订阅查看详情", "Select a subscription to view details")} />}>
              <If when={selectedProfile() && !profiles().some((profile) => profile.id === collection.selected())}>
                <Hint text={ui.t("当前订阅不在筛选结果中。已保留详情。", "This subscription is outside the filter. Its details are retained.")} />
              </If>
              <ListSurface>
                {selectedProfile() ? row(selectedProfile()!, true) : undefined}
              </ListSurface>
              <View style={{ gap: spacing.sm }}>
                <SectionHeader title={ui.t("信息", "Details")} />
                <ListSurface>
                  <View style={{ paddingHorizontal: spacing.md, paddingVertical: density.space(10), borderBottomWidth: 1, borderBottomColor: palette().line }}>
                    <Detail stacked label={ui.t("来源", "Source")} value={selectedProfile()?.subscriptionUrl || selectedProfile()?.source || ""} />
                  </View>
                  <View style={{ paddingHorizontal: spacing.md, paddingVertical: density.space(10), borderBottomWidth: selectedProfile()?.subscriptionUserInfo ? 1 : 0, borderBottomColor: palette().line }}>
                    <Detail label={ui.t("更新时间", "Updated")} value={timestamp(selectedProfile()?.lastRefreshAt || selectedProfile()?.updatedAt)} />
                  </View>
                  <View style={{ paddingHorizontal: spacing.md, paddingVertical: density.space(10), borderBottomWidth: selectedProfile()?.subscriptionUserInfo ? 1 : 0, borderBottomColor: palette().line }}>
                    <Detail label={ui.t("本地备份", "Local backup")} value={selectedProfile()?.hasBackup ? ui.t("有上次备份", "Previous backup available") : ui.t("无备份", "No backup")} />
                  </View>
                  <If when={selectedProfile()?.subscriptionUserInfo}>
                    <View style={{ paddingHorizontal: spacing.md, paddingVertical: density.space(10), gap: spacing.sm }}>
                      <Detail
                        label={ui.t("已用流量", "Usage")}
                        value={
                          selectedProfile()?.subscriptionUserInfo?.totalBytes
                            ? `${bytes((selectedProfile()?.subscriptionUserInfo?.uploadBytes || 0) + (selectedProfile()?.subscriptionUserInfo?.downloadBytes || 0))} / ${bytes(selectedProfile()!.subscriptionUserInfo!.totalBytes!)}`
                            : bytes((selectedProfile()?.subscriptionUserInfo?.uploadBytes || 0) + (selectedProfile()?.subscriptionUserInfo?.downloadBytes || 0))
                        }
                      />
                      <If when={!!selectedProfile()?.subscriptionUserInfo?.totalBytes}>
                        <MeterBar
                          ratio={Math.min(
                            1,
                            ((selectedProfile()?.subscriptionUserInfo?.uploadBytes || 0) +
                              (selectedProfile()?.subscriptionUserInfo?.downloadBytes || 0)) /
                              (selectedProfile()?.subscriptionUserInfo?.totalBytes || 1),
                          )}
                        />
                      </If>
                    </View>
                  </If>
                </ListSurface>
              </View>
              <If when={selectedProfile()?.lastRefreshError}>
                <PageError id="profile-detail-error" text={selectedProfile()?.lastRefreshError || ""} />
              </If>
              <Actions align="start">
                <If when={selectedProfile() && !selectedProfile()!.active}>
                  <Action id="profile-activate" primary label={ui.t("启用此订阅", "Use this subscription")} disabled={ui.client.pending() > 0 || !ui.client.domains.readyForCommand(["config", "profiles"])} onPress={() => void profileAction("profiles.activate", collection.selected())} />
                </If>
                <If when={selectedProfile()?.subscriptionUrl}>
                  <Action icon="refresh-cw" label={selectedProfile()?.lastRefreshError ? ui.t("重试更新", "Retry update") : ui.t("更新订阅", "Update subscription")} loading={!!selectedProfile() && refreshing(selectedProfile()!)} disabled={ui.client.pending() > 0} onPress={() => void profileAction("profiles.refresh", collection.selected())} />
                </If>
                <If when={selectedProfile()?.active}>
                  <Action id="profile-proxies" label={ui.t("代理节点", "Proxy nodes")} onPress={() => ui.navigate("proxies")} />
                </If>
                <Action icon="file-pen-line" label={ui.t("编辑 YAML", "Edit YAML")} onPress={() => { if (selectedProfile()) ui.editProfile(selectedProfile()!.id); }} />
                <Action icon="download" label={ui.t("导出…", "Export…")} onPress={() => { if (selectedProfile()) { exporting.set(selectedProfile()); qr.set(undefined); } }} />
                <Action icon="copy" label={ui.t("复制完整来源", "Copy source")} onPress={() => copyText(ui, selectedProfile()?.subscriptionUrl || selectedProfile()?.source || "", ui.t("来源已复制", "Source copied"))} />
              </Actions>
            </If>
          </Body>
        </Scroll>
      </View>
      </View>
      <If when={open()}>
        {(() => {
          const importBody = (
            <View style={{ gap: 14 }}>
              <View style={{ flexDirection: "row", gap: 10 }}>
                <ChoiceTile
                  id="import-local-file"
                  icon="file-up"
                  title={ui.t("从本地导入", "Local file")}
                  subtitle={ui.t("选择 YAML 配置文件", "Pick a YAML profile")}
                  loading={importOp() === "file"}
                  disabled={busy()}
                  onPress={() => void pick()}
                />
                <ChoiceTile
                  id="import-scan-qr"
                  icon="scan-qr-code"
                  title={ui.t("扫码导入", "Scan QR")}
                  subtitle={ui.t("扫描订阅二维码", "Scan subscription QR")}
                  loading={importOp() === "scan"}
                  disabled={busy()}
                  onPress={() => void scan()}
                />
              </View>
              <OrRule
                label={ui.t("或粘贴订阅链接", "Or paste subscription URL")}
              />
              <View style={{ gap: density.space(12) }}>
                <Field
                  onSubmit={() => void importContent()}
                  id="profile-input"
                  keyboardType="url"
                  label={ui.t("订阅地址", "Subscription URL")}
                  value={input()}
                  onChange={(text) => {
                    input.set(text);
                    ui.client.error.set("");
                  }}
                  placeholder="https://example.com/sub?token=…"
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
                    "留空时自动使用配置名称",
                    "Leave blank to use profile name",
                  )}
                  disabled={busy()}
                />
              </View>
              <If when={ui.client.error()}>
                <PageError
                  id="profile-import-error"
                  text={ui.client.error()}
                />
              </If>
            </View>
          );
          const importTitle = ui.t("添加订阅", "Add subscription");
          const importSubtitle = ui.t(
            "从本地文件、二维码直接导入，或填写订阅地址",
            "Import from a local file or QR code, or paste a subscription URL",
          );
          return (
            <If
              when={layout().sidebar || platform.desktop}
              else={
                <BottomSheet
                  title={importTitle}
                  subtitle={importSubtitle}
                  onClose={closeImport}
                  footer={
                    <View style={{ flexDirection: "row", gap: 8 }}>
                      <Action
                        flex={1}
                        secondary
                        label={ui.t("取消", "Cancel")}
                        onPress={closeImport}
                      />
                      <Action
                        flex={1}
                        primary
                        id="submit-import"
                        icon="download"
                        loading={importOp() === "url"}
                        label={ui.t("导入订阅", "Import subscription")}
                        disabled={busy() || !input().trim()}
                        onPress={() => void importContent()}
                      />
                    </View>
                  }
                >
                  {importBody}
                </BottomSheet>
              }
            >
              <Dialog
                width={440}
                title={importTitle}
                subtitle={importSubtitle}
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
                      loading={importOp() === "url"}
                      label={ui.t("导入订阅", "Import subscription")}
                      disabled={busy() || !input().trim()}
                      onPress={() => void importContent()}
                    />
                  </Actions>
                }
              >
                {importBody}
              </Dialog>
            </If>
          );
        })()}
      </If>
      <If when={refreshError()}>
        <Dialog
          width={platform.desktop ? 400 : 460}
          title={ui.t("刷新失败", "Refresh failed")}
          onClose={() => refreshError.set("")}
          footer={
            <Actions>
              <Action
                secondary
                label={ui.t("关闭", "Close")}
                onPress={() => refreshError.set("")}
              />
              <Action
                label={ui.t("复制错误信息", "Copy error")}
                onPress={() =>
                  copyText(
                    ui,
                    refreshError(),
                    ui.t("错误信息已复制", "Error copied"),
                  )
                }
              />
            </Actions>
          }
        >
          <Scroll
            bounces={false}
            style={{
              height: Math.min(280, Math.max(120, windowSize().height * 0.3)),
            }}
          >
            <PageError id="profile-refresh-failure" text={refreshError()} />
          </Scroll>
        </Dialog>
      </If>
      <If when={editing()}>
        <Dialog
          width={platform.desktop ? 400 : 460}
          title={ui.t("编辑订阅", "Edit subscription")}
          description={ui.t(
            "修改名称或订阅地址，保存后可手动刷新。",
            "Change the name or subscription URL. Refresh manually after saving.",
          )}
          onClose={() => void closeEdit()}
          footer={
            <Actions>
              <Action
                secondary
                label={ui.t("取消", "Cancel")}
                onPress={() => void closeEdit()}
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
            keyboardType="url"
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
          width={platform.desktop ? 400 : 460}
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
                <View
                  style={{
                    padding: 12,
                    borderRadius: radius.lg,
                    borderCurve: "continuous",
                    backgroundColor: palette().qrCanvas,
                    borderWidth: 1,
                    borderColor: palette().border,
                  }}
                >
                  <Image
                    src={qr()!.source}
                    style={{
                      width: Math.max(140, Math.min(220, windowSize().width - 104)),
                      height: Math.max(140, Math.min(220, windowSize().width - 104)),
                    }}
                  />
                </View>
                <Actions>
                  <Action
                    secondary
                    label={ui.t("返回", "Back")}
                    onPress={() => qr.set(undefined)}
                  />
                  <Action
                    primary
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
                borderCurve: "continuous",
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
                last
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
  onDetails: () => void;
  selected?: boolean;
  compact?: boolean;
  detail?: boolean;
}) {
  const ui = useApp(),
    alive = useLifetime(),
    route = RouteOwner() || ui.route();
  const open = signal(false),
    contextual = signal(false);
  useOverlay(() => open() || contextual());
  let pending: { source: "menu" | "context"; action: () => void } | undefined;
  const ready = () => ui.client.domains.readyForCommand(["config", "profiles"]);
  const height = () =>
    Math.max(
      84,
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
        borderCurve: "continuous",
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
      <If when={!(platform.desktop && value.source === "context")}>
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
            tooltip={false}
            label={ui.t("关闭菜单", "Close menu")}
            onPress={() => {
              open.set(false);
              contextual.set(false);
            }}
          />
        </View>
      </If>
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
          id={`edit-subscription-${props.detail ? "detail-" : ""}${props.profile.id}`}
          icon="file-pen-line"
          label={ui.t("编辑订阅…", "Edit subscription…")}
          action={props.onEdit}
        />
      </If>
      <Command
        source={value.source}
        id={`edit-yaml-${props.detail ? "detail-" : ""}${props.profile.id}`}
        icon="file-pen-line"
        label={ui.t("编辑 YAML…", "Edit YAML…")}
        action={() => ui.editProfile(props.profile.id)}
      />
      <Command
        source={value.source}
        id={`profile-export-${props.detail ? "detail-" : ""}${props.profile.id}`}
        icon="download"
        label={ui.t("导出…", "Export…")}
        action={props.onExport}
      />
      <If when={props.profile.subscriptionUrl}>
        <Command
          source={value.source}
          id={`refresh-profile-${props.detail ? "detail-" : ""}${props.profile.id}`}
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
                actionLabel: ui.t("恢复并覆盖", "Restore and replace"),
                description: ui.t(
                  `仅替换“${props.profile.name}”。${props.profile.active ? "这是当前订阅，可能重新连接 VPN。" : "不影响当前连接。"}`,
                  `Replace only “${props.profile.name}”. ${props.profile.active ? "This is the active subscription; VPN may reconnect." : "The active connection is unaffected."}`,
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
            { actionLabel: ui.t("删除订阅", "Delete subscription"), description: props.profile.active ? ui.t("删除当前订阅会影响正在使用它的连接；不会删除其他订阅。", "Deleting the active subscription affects its connection. Other subscriptions are kept.") : ui.t("仅删除此订阅及其本地副本，其他订阅保持不变。", "Delete only this subscription and its local copies. Other subscriptions are kept.") },
          );
        }}
      />
    </View>
  );
  const trigger = (
    <View
      style={{
        minHeight: height(),
        flexDirection: "row",
        alignItems: "center",
        borderBottomWidth: props.detail ? 0 : 1,
        borderBottomColor: palette().line,
        backgroundColor:
          props.selected && props.compact && !props.detail
            ? palette().accentFill
            : contextual()
              ? palette().soft
              : palette().panel,
      }}
    >
      <View
        testID={`open-profile-${props.detail ? "detail-" : ""}${props.profile.id}`}
        role="button"
        accessibilityLabel={props.profile.name}
        accessibilityState={{
          selected: !!props.selected,
        }}
        focusStyle={focusRing()}
        hoverStyle={{
          backgroundColor:
            props.selected && props.compact && !props.detail
              ? palette().accentFill
              : palette().hover,
        }}
        activeStyle={{ backgroundColor: palette().soft }}
        onPress={props.onDetails}
        style={{
          flex: 1,
          minHeight: height() - 1,
          paddingHorizontal: spacing.lg,
          paddingVertical: density.space(10),
          flexDirection: "row",
          alignItems: "center",
          gap: spacing.md,
          ...feedbackStyle(),
        }}
      >
        <If
          when={props.refreshing}
          else={
            <IconTile
              icon={props.profile.active ? "circle-check" : "rss"}
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
                fontSize: typography.sm,
                lineHeight: density.line(20),
                fontWeight: "600",
                color: palette().text,
              }}
            >
              {props.profile.name}
            </Text>
            <If when={props.profile.active}>
              <Badge tone="brand" label={ui.t("当前", "Current")} />
            </If>
          </View>
          <If when={!(props.compact || windowSize().width < 840)}>
            <Text
              style={{
                numberOfLines: 1,
                textOverflow: "ellipsis-middle",
                fontSize: typography.note,
                lineHeight: density.line(platform.desktop ? 16 : 18),
                color: palette().muted,
              }}
            >
              {props.profile.subscriptionUrl || props.profile.source}
            </Text>
          </If>
          <If when={props.compact || windowSize().width < 840}>
            <Text
              style={{
                numberOfLines: 1,
                fontSize: typography.note,
                lineHeight: density.line(platform.desktop ? 16 : 18),
                fontVariant: ["tabular-nums"],
                color: props.profile.lastRefreshError
                  ? palette().danger
                  : palette().muted,
              }}
            >
              {updated()}
            </Text>
          </If>
          <If when={!!props.profile.subscriptionUserInfo?.totalBytes && !props.detail}>
            <View style={{ paddingTop: density.space(4) }}>
              <MeterBar
                ratio={Math.min(
                  1,
                  ((props.profile.subscriptionUserInfo?.uploadBytes || 0) +
                    (props.profile.subscriptionUserInfo?.downloadBytes || 0)) /
                    (props.profile.subscriptionUserInfo?.totalBytes || 1),
                )}
              />
            </View>
          </If>
          <If when={(props.compact || windowSize().width < 840) && !!usage()}>
            <Text
              style={{
                numberOfLines: 1,
                fontSize: typography.note,
                lineHeight: density.line(platform.desktop ? 16 : 18),
                fontVariant: ["tabular-nums"],
                color: palette().muted,
              }}
            >
              {usage()}
            </Text>
          </If>
        </View>
        <If when={!props.compact && windowSize().width >= 840}>
          <View
            style={{ width: 160, gap: spacing.xxs, alignItems: "flex-end" }}
          >
            <Text
              style={{
                fontSize: density.font(12),
                lineHeight: density.line(18),
                fontVariant: ["tabular-nums"],
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
        onOpenChange={(event) => open.set(event.open)}
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
            testID={`profile-menu-${props.detail ? "detail-" : ""}${props.profile.id}`}
            role="button"
            hitSlop={control.hitSlop(control.icon)}
            accessibilityLabel={ui.t("订阅操作", "Subscription actions")}
            accessibilityState={{ expanded: open() }}
            focusStyle={focusRing()}
            hoverStyle={{ backgroundColor: palette().soft }}
            activeStyle={{ backgroundColor: palette().soft }}
            style={{
              width: control.icon,
              height: control.icon,
              marginRight: spacing.sm,
              borderRadius: radius.md,
              borderCurve: "continuous",
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
  last?: boolean;
  id?: string;
  disabled?: boolean;
  loading?: boolean;
}) {
  const disabled = () => !!props.disabled || !!props.loading;
  return (
    <>
      <View
        testID={props.id}
        role="button"
        accessibilityLabel={props.label}
        accessibilityState={{ disabled: disabled(), busy: !!props.loading }}
        disabled={disabled()}
        onPress={() => {
          if (!disabled()) props.onPress();
        }}
        focusStyle={focusRing()}
        hoverStyle={disabled() ? undefined : { backgroundColor: palette().soft }}
        activeStyle={
          disabled() ? undefined : { backgroundColor: palette().soft }
        }
        style={{
          minHeight: control.row,
          paddingVertical: spacing.sm,
          paddingHorizontal: spacing.md,
          flexDirection: "row",
          alignItems: "center",
          gap: density.space(10),
          opacity: disabled() ? 0.5 : 1,
          backgroundColor: palette().panel,
          ...feedbackStyle(),
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
      <If when={!props.last}>
        <Separator />
      </If>
    </>
  );
}

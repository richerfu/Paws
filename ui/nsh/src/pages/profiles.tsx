import { density, typography } from "../theme";
import {
  batch,
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
  RouteOwner,
  ObjectMenu,
  type ObjectCommand,
} from "../components";
import { feedbackStyle } from "../motion";
import { focusRing, surfaceFinish } from "../theme";
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
    batch(() => {
      busy.set(false);
      importOp.set("");
    });
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
    batch(() => {
      busy.set(true);
      ui.client.error.set("");
    });
    try {
      await ui.client.send(command, { intentId: token, ...payload });
      if (alive() && intent === token) {
        batch(() => {
          open.set(false);
          name.set("");
          input.set("");
          ui.client.notice.set(
            ui.t("订阅已导入并启用", "Subscription imported and activated"),
          );
        });
      }
    } catch (error) {
      if (alive() && intent === token) ui.client.error.set(String(error));
    } finally {
      if (alive() && intent === token) {
        intent = undefined;
        batch(() => {
          busy.set(false);
          importOp.set("");
        });
      }
    }
  };
  const importContent = () => {
    if (busy() || !input().trim()) return;
    importOp.set("url");
    return performImport("profiles.importUrl", {
      name: name().trim() || undefined,
      url: input().trim(),
    });
  };
  const pick = async () => {
    const generation = dialogGeneration;
    batch(() => {
      busy.set(true);
      importOp.set("file");
    });
    const value = await ui.client.run<{ name: string; content: string } | null>(
      "platform.importFile",
    );
    if (!alive() || !open() || generation !== dialogGeneration) return;
    if (value) {
      busy.set(false);
      await performImport("profiles.importContent", {
        name: value.name,
        content: value.content,
      });
    } else {
      batch(() => {
        busy.set(false);
        importOp.set("");
      });
    }
  };
  const scan = async () => {
    const generation = dialogGeneration;
    batch(() => {
      busy.set(true);
      importOp.set("scan");
    });
    const value = await ui.client.run<string | null>("platform.scan");
    if (!alive() || !open() || generation !== dialogGeneration) return;
    if (!value) {
      batch(() => {
        busy.set(false);
        importOp.set("");
      });
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
        name: parsed.name || name().trim() || undefined,
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
    batch(() => {
      editing.set({
        id: profile.id,
        revision: ui.client.state()!.configRevision,
      });
      editName.set(profile.name);
      editUrl.set(profile.subscriptionUrl || "");
      initialEdit.set({
        name: profile.name,
        url: profile.subscriptionUrl || "",
      });
    });
  };
  const listInset = () =>
    layout().masterDetail
      ? layout().inset
      : windowSize().width >= 840
        ? spacing.xxl
        : spacing.lg;
  const engineMeta = () =>
    ui.client.state()?.vpnOptions.engine === "auto"
      ? ui.t("自动选择", "Auto")
      : ui.client.state()?.vpnOptions.engine === "mihomo"
        ? "Mihomo"
        : ui.client.state()?.vpnOptions.engine === "meow"
          ? "Meow"
          : "sing-box";
  // List retains keyed rows; keep its accessor alive instead of capturing the
  // record from the first render (including in commands that open an editor).
  const row = (profile: () => Profile, detail = false) => <ProfileRow
    detail={detail}
    profile={profile()} refreshing={refreshing(profile())}
    selected={collection.selected() === profile().id}
    compact={layout().masterDetail}
    onDetails={() => selectProfile(profile())}
    onAction={(command) => { void profileAction(command, profile().id); }}
    onEdit={() => openEdit(profile())}
    onExport={() =>
      batch(() => {
        exporting.set(profile());
        qr.set(undefined);
      })
    }
    onError={() => refreshError.set(profile().lastRefreshError || "")}
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
          paddingLeft: listInset(),
          paddingRight: layout().masterDetail ? 0 : listInset(),
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
        estimatedItemSize={!platform.desktop && windowSize().width < 840 ? 104 : 124}
        style={{ flex: 1 }}
        contentStyle={{
          paddingLeft: listInset(),
          paddingRight: layout().masterDetail ? 0 : listInset(),
          paddingBottom: spacing.section,
        }}
        footer={
          !layout().masterDetail && !query() && profiles().length > 0 ? (
            <View
              testID="profiles-quick-actions"
              style={{
                paddingTop: spacing.xs,
                gap: spacing.md,
              }}
            >
              <FeatureActionCard
                id="profiles-quick-import"
                icon="scan-qr-code"
                title={ui.t("扫码或粘贴链接导入", "Scan QR or paste subscription URL")}
                subtitle={ui.t(
                  "支持订阅地址、YAML 文件与二维码",
                  "Subscription URL, YAML file or QR code",
                )}
                onPress={() => open.set(true)}
              />
              <If when={!!ui.client.state()?.activeProfile}>
                <FeatureActionCard
                  id="profiles-quick-yaml"
                  icon="file-pen-line"
                  title={ui.t("编辑当前配置 YAML", "Edit active profile YAML")}
                  subtitle={ui.t(
                    "语法高亮、查找替换与保存前校验",
                    "Syntax highlight, search and validation",
                  )}
                  onPress={() => {
                    const active = ui.client.state()?.activeProfile;
                    if (active) ui.editProfile(active);
                  }}
                />
              </If>
            </View>
          ) : undefined
        }
        empty={
          <ListSurface style={{ ...surfaceFinish("card") }}>
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
          </ListSurface>
        }
      >
        {(profile) => row(profile)}
      </List>
      </View>
      <View testID="profile-detail-panel" style={{ display: props.profileId || layout().masterDetail ? "flex" : "none", flex: 1, minWidth: 0, flexDirection: "column" }}>
        <Scroll bounces={false} style={{ flex: 1 }}>
          <Body>
            <If when={selectedProfile()} else={<Empty icon="rss" text={collection.selected() ? ui.t("此订阅已移除。返回列表选择其他订阅。", "This subscription was removed. Return to the list.") : ui.t("选择订阅查看详情", "Select a subscription to view details")} />}>
              {(selected) => (
                <>
                  <If when={!profiles().some((profile) => profile.id === collection.selected())}>
                    <Hint text={ui.t("当前订阅不在筛选结果中。已保留详情。", "This subscription is outside the filter. Its details are retained.")} />
                  </If>
                  {row(selected, true)}
                  <View style={{ gap: spacing.sm }}>
                    <SectionHeader title={ui.t("信息", "Details")} />
                    <ListSurface style={{ ...surfaceFinish("card") }}>
                      <View style={{ paddingHorizontal: spacing.lg, paddingVertical: density.space(12), borderBottomWidth: 1, borderBottomColor: palette().line }}>
                        <View style={{ flexDirection: "row", alignItems: "flex-start", gap: spacing.sm }}>
                          <View style={{ flex: 1, minWidth: 0 }}>
                            <Detail stacked label={ui.t("来源", "Source")} value={selected().subscriptionUrl || selected().source || ""} />
                          </View>
                          <IconButton
                            id="profile-copy-source"
                            icon="copy"
                            label={ui.t("复制完整来源", "Copy source")}
                            onPress={() => copyText(ui, selected().subscriptionUrl || selected().source || "", ui.t("来源已复制", "Source copied"))}
                          />
                        </View>
                      </View>
                      <View style={{ paddingHorizontal: spacing.lg, paddingVertical: density.space(12), borderBottomWidth: 1, borderBottomColor: palette().line }}>
                        <Detail label={ui.t("更新时间", "Updated")} value={timestamp(selected().lastRefreshAt || selected().updatedAt)} />
                      </View>
                      <View style={{ paddingHorizontal: spacing.lg, paddingVertical: density.space(12), borderBottomWidth: selected().subscriptionUserInfo ? 1 : 0, borderBottomColor: palette().line }}>
                        <Detail label={ui.t("本地备份", "Local backup")} value={selected().hasBackup ? ui.t("有上次备份", "Previous backup available") : ui.t("无备份", "No backup")} />
                      </View>
                      <If when={!!selected().subscriptionUserInfo?.expireAt}>
                        <View style={{ paddingHorizontal: spacing.lg, paddingVertical: density.space(12), borderBottomWidth: 1, borderBottomColor: palette().line }}>
                          <Detail
                            label={ui.t("到期时间", "Expires")}
                            value={formatExpiry(selected().subscriptionUserInfo!.expireAt!, ui.t)}
                          />
                        </View>
                      </If>
                      <If when={selected().subscriptionUserInfo}>
                        <View style={{ paddingHorizontal: spacing.lg, paddingVertical: density.space(12), gap: spacing.xs }}>
                          <Detail
                            label={ui.t("已用流量", "Usage")}
                            value={
                              selected().subscriptionUserInfo?.totalBytes
                                ? `${bytes((selected().subscriptionUserInfo?.uploadBytes || 0) + (selected().subscriptionUserInfo?.downloadBytes || 0))} / ${bytes(selected().subscriptionUserInfo!.totalBytes!)}`
                                : bytes((selected().subscriptionUserInfo?.uploadBytes || 0) + (selected().subscriptionUserInfo?.downloadBytes || 0))
                            }
                          />
                          <If when={!!selected().subscriptionUserInfo?.totalBytes}>
                            <MeterBar
                              ratio={Math.min(
                                1,
                                ((selected().subscriptionUserInfo?.uploadBytes || 0) +
                                  (selected().subscriptionUserInfo?.downloadBytes || 0)) /
                                  (selected().subscriptionUserInfo?.totalBytes || 1),
                              )}
                            />
                          </If>
                        </View>
                      </If>
                    </ListSurface>
                  </View>
                  <If when={selected().lastRefreshError}>
                    <PageError id="profile-detail-error" text={selected().lastRefreshError || ""} />
                  </If>
                  <If
                    when={layout().masterDetail}
                    else={
                      <View style={{ gap: spacing.md }}>
                        <If when={!selected().active}>
                          <Action
                            id="profile-activate"
                            primary
                            fullWidth
                            icon="circle-check"
                            label={ui.t("启用此订阅", "Use this subscription")}
                            disabled={ui.client.pending() > 0 || !ui.client.domains.readyForCommand(["config", "profiles"])}
                            onPress={() => void profileAction("profiles.activate", collection.selected())}
                          />
                        </If>
                        <If when={selected().subscriptionUrl && selected().active}>
                          <Action
                            primary
                            fullWidth
                            icon="refresh-cw"
                            label={selected().lastRefreshError ? ui.t("重试更新", "Retry update") : ui.t("更新订阅", "Update subscription")}
                            loading={refreshing(selected())}
                            disabled={ui.client.pending() > 0}
                            onPress={() => void profileAction("profiles.refresh", collection.selected())}
                          />
                        </If>
                        <If when={selected().active}>
                          <FeatureActionCard
                            id="profile-proxies"
                            icon="git-branch"
                            title={ui.t("查看代理节点", "View proxy nodes")}
                            subtitle={ui.t("切换策略组节点与测试延迟", "Switch policy group nodes and test latency")}
                            onPress={() => ui.navigate("proxies")}
                          />
                        </If>
                        <FeatureActionCard
                          id="profile-edit-detail"
                          icon="link"
                          title={ui.t("编辑订阅…", "Edit subscription…")}
                          subtitle={ui.t("修改订阅名称与更新地址", "Edit profile name and subscription URL")}
                          onPress={() => openEdit(selected())}
                        />
                        <FeatureActionCard
                          id="profile-edit-yaml-detail"
                          icon="file-pen-line"
                          title={ui.t("编辑 YAML", "Edit YAML")}
                          subtitle={ui.t("语法高亮、查找替换与保存前校验", "Syntax highlight, search and validation")}
                          disabled={!ui.client.domains.readyForCommand(["config", "profiles"])}
                          onPress={() => ui.editProfile(selected().id)}
                        />
                        <FeatureActionCard
                          id="profile-export-detail"
                          icon="download"
                          title={ui.t("导出…", "Export…")}
                          subtitle={ui.t("导出 YAML 配置文件或生成二维码", "Export YAML file or generate QR code")}
                          disabled={!ui.client.domains.readyForCommand(["config", "profiles"])}
                          onPress={() =>
                            batch(() => {
                              exporting.set(selected());
                              qr.set(undefined);
                            })
                          }
                        />
                      </View>
                    }
                  >
                    <Actions align="start">
                      <If when={!selected().active}>
                        <Action id="profile-activate" primary label={ui.t("启用此订阅", "Use this subscription")} disabled={ui.client.pending() > 0 || !ui.client.domains.readyForCommand(["config", "profiles"])} onPress={() => void profileAction("profiles.activate", collection.selected())} />
                      </If>
                      <If when={selected().subscriptionUrl && selected().active}>
                        <Action primary icon="refresh-cw" label={selected().lastRefreshError ? ui.t("重试更新", "Retry update") : ui.t("更新订阅", "Update subscription")} loading={refreshing(selected())} disabled={ui.client.pending() > 0} onPress={() => void profileAction("profiles.refresh", collection.selected())} />
                      </If>
                      <If when={selected().active}>
                        <Action id="profile-proxies" label={ui.t("查看代理节点", "View proxy nodes")} onPress={() => ui.navigate("proxies")} />
                      </If>
                      <Action id="profile-edit-detail" label={ui.t("编辑订阅…", "Edit subscription…")} onPress={() => openEdit(selected())} />
                      <Action id="profile-edit-yaml-detail" icon="file-pen-line" label={ui.t("编辑 YAML", "Edit YAML")} disabled={!ui.client.domains.readyForCommand(["config", "profiles"])} onPress={() => ui.editProfile(selected().id)} />
                      <Action
                        id="profile-export-detail"
                        icon="download"
                        label={ui.t("导出…", "Export…")}
                        disabled={!ui.client.domains.readyForCommand(["config", "profiles"])}
                        onPress={() =>
                          batch(() => {
                            exporting.set(selected());
                            qr.set(undefined);
                          })
                        }
                      />
                    </Actions>
                  </If>
                  <View style={{ gap: spacing.sm }}>
                    <SectionHeader
                      title={ui.t("运行内核", "Engine")}
                      meta={engineMeta()}
                    />
                    <FeatureActionCard
                      id="profile-engine-settings"
                      icon="sliders-horizontal"
                      title={ui.t("三内核自由切换", "Multi-engine runtime")}
                      subtitle={ui.t(
                        "支持 Mihomo、sing-box 与 Meow，点击前往网络设置调整",
                        "Supports Mihomo, sing-box and Meow; open Network settings to configure",
                      )}
                      onPress={() => ui.navigate("settings")}
                    />
                  </View>
                </>
              )}
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
                  subtitle={ui.t("YAML 配置", "YAML file")}
                  loading={importOp() === "file"}
                  disabled={busy()}
                  onPress={() => void pick()}
                />
                <ChoiceTile
                  id="import-scan-qr"
                  icon="scan-qr-code"
                  title={ui.t("扫码导入", "Scan QR")}
                  subtitle={ui.t("订阅二维码", "Subscription QR")}
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
                  onChange={(text) =>
                    batch(() => {
                      input.set(text);
                      ui.client.error.set("");
                    })
                  }
                  placeholder="https://example.com/sub?token=…"
                  disabled={busy()}
                />
                <Field
                  label={ui.t("名称（可选）", "Name (optional)")}
                  value={name()}
                  onChange={(text) =>
                    batch(() => {
                      name.set(text);
                      ui.client.error.set("");
                    })
                  }
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
          return (
            <If
              when={
                layout().sidebar || platform.desktop || platform.idiom === "pad"
              }
              else={
                <BottomSheet
                  title={importTitle}
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
          onClose={() =>
            batch(() => {
              exporting.set(undefined);
              qr.set(undefined);
            })
          }
        >
          <If
            when={!qr()}
            else={
              <View style={{ gap: density.space(16), alignItems: "center" }}>
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

function remainingDaysLabel(
  expireAt: string | undefined,
  t: (zh: string, en: string) => string,
): string {
  if (!expireAt) return "";
  const numeric = Number(expireAt);
  const ms = Number.isFinite(numeric)
    ? numeric > 1e14
      ? numeric / 1e3
      : numeric > 1e11
        ? numeric
        : numeric * 1000
    : Date.parse(expireAt);
  if (!Number.isFinite(ms)) return "";
  const diffDays = Math.ceil((ms - Date.now()) / 86_400_000);
  if (diffDays <= 0) return t("已过期", "Expired");
  return t(`剩余 ${diffDays} 天`, `${diffDays}d left`);
}

function formatExpiry(
  expireAt: string,
  t: (zh: string, en: string) => string,
): string {
  const full = timestamp(expireAt);
  const dateOnly = full.length >= 10 ? full.slice(0, 10) : full;
  const remaining = remainingDaysLabel(expireAt, t);
  return remaining ? `${dateOnly} · ${remaining}` : dateOnly;
}

function FeatureActionCard(props: {
  id?: string;
  icon: Parameters<typeof Icon>[0]["name"];
  title: string;
  subtitle: string;
  onPress: () => void;
  disabled?: boolean;
}) {
  return (
    <View
      testID={props.id}
      role="button"
      accessibilityLabel={props.title}
      accessibilityState={{ disabled: !!props.disabled }}
      disabled={props.disabled}
      focusVisibleStyle={focusRing()}
      hoverStyle={
        props.disabled ? undefined : { backgroundColor: palette().hover }
      }
      activeStyle={
        props.disabled ? undefined : { backgroundColor: palette().soft }
      }
      onPress={() => {
        if (!props.disabled) props.onPress();
      }}
      style={{
        flexDirection: "row",
        alignItems: "center",
        gap: spacing.md,
        paddingHorizontal: spacing.lg,
        paddingVertical: density.space(14),
        borderRadius: radius.xl,
        borderCurve: "continuous",
        borderWidth: 1,
        borderColor: palette().border,
        backgroundColor: palette().panel,
        opacity: props.disabled ? 0.45 : 1,
        ...surfaceFinish("card"),
        ...feedbackStyle(),
      }}
    >
      <View
        style={{
          width: platform.desktop ? 36 : 44,
          height: platform.desktop ? 36 : 44,
          borderRadius: radius.lg,
          borderCurve: "continuous",
          borderWidth: 1,
          borderColor: palette().line,
          backgroundColor: palette().inset,
          alignItems: "center",
          justifyContent: "center",
          flexShrink: 0,
        }}
      >
        <Icon
          name={props.icon}
          size={platform.desktop ? 18 : 22}
          color={palette().text}
        />
      </View>
      <View style={{ flex: 1, minWidth: 0, gap: density.space(2) }}>
        <Text
          style={{
            numberOfLines: 1,
            fontSize: typography.sm,
            lineHeight: density.line(20),
            fontWeight: "600",
            color: palette().text,
          }}
        >
          {props.title}
        </Text>
        <Text
          style={{
            numberOfLines: 2,
            fontSize: typography.note,
            lineHeight: density.line(18),
            color: palette().muted,
          }}
        >
          {props.subtitle}
        </Text>
      </View>
      <Icon name="chevron-right" size={16} color={palette().faint} />
    </View>
  );
}

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
  const ui = useApp();
  const ready = () => ui.client.domains.readyForCommand(["config", "profiles"]);
  const highlighted = () =>
    !props.detail && (props.compact ? !!props.selected : props.profile.active);
  const updated = () => {
    const base = props.refreshing
      ? ui.t("正在更新", "Updating")
      : props.profile.lastRefreshError
        ? ui.t("更新失败", "Update failed")
        : props.profile.lastRefreshAt || props.profile.updatedAt
          ? timestamp(props.profile.lastRefreshAt || props.profile.updatedAt)
          : ui.t("尚未更新", "Not updated");
    if (props.refreshing || props.profile.lastRefreshError) return base;
    const remain = remainingDaysLabel(
      props.profile.subscriptionUserInfo?.expireAt,
      ui.t,
    );
    return remain ? `${base} · ${remain}` : base;
  };
  const usage = () =>
    props.profile.subscriptionUserInfo?.totalBytes
      ? `${bytes((props.profile.subscriptionUserInfo.uploadBytes || 0) + (props.profile.subscriptionUserInfo.downloadBytes || 0))} / ${bytes(props.profile.subscriptionUserInfo.totalBytes)}`
      : "";
  const idPrefix = () => (props.detail ? "detail-" : "");
  const commands = (): ObjectCommand[] => {
    const items: ObjectCommand[] = [];
    const locked = !ready();
    if (props.profile.lastRefreshError) {
      items.push({
        id: "profile-refresh-error",
        icon: "triangle-alert",
        label: ui.t("更新错误…", "Update error…"),
        disabled: locked,
        separatorAfter: true,
        run: props.onError,
      });
    }
    if (!props.profile.active) {
      items.push({
        icon: "circle-check",
        label: ui.t("设为当前订阅", "Use this subscription"),
        disabled: locked,
        run: () => props.onAction("profiles.activate"),
      });
    }
    if (props.profile.subscriptionUrl) {
      items.push({
        id: `edit-subscription-${idPrefix()}${props.profile.id}`,
        icon: "file-pen-line",
        label: ui.t("编辑订阅…", "Edit subscription…"),
        disabled: locked,
        run: props.onEdit,
      });
    }
    items.push(
      {
        id: `edit-yaml-${idPrefix()}${props.profile.id}`,
        icon: "file-pen-line",
        label: ui.t("编辑 YAML…", "Edit YAML…"),
        disabled: locked,
        run: () => ui.editProfile(props.profile.id),
      },
      {
        id: `profile-export-${idPrefix()}${props.profile.id}`,
        icon: "download",
        label: ui.t("导出…", "Export…"),
        disabled: locked,
        run: props.onExport,
      },
    );
    if (props.profile.subscriptionUrl) {
      items.push({
        id: `refresh-profile-${idPrefix()}${props.profile.id}`,
        icon: "refresh-cw",
        label: ui.t("更新订阅", "Update subscription"),
        disabled: locked || ui.client.pending() > 0 || props.refreshing,
        run: () => props.onAction("profiles.refresh"),
      });
    }
    if (props.profile.hasBackup) {
      items.push({
        icon: "history",
        label: ui.t("恢复上次备份…", "Restore previous backup…"),
        disabled: locked,
        run: () => {
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
        },
      });
    }
    items.push({
      icon: "trash-2",
      danger: true,
      label: ui.t("删除…", "Delete…"),
      disabled: locked,
      run: () => {
        void confirm(
          ui,
          ui.t(
            `删除“${props.profile.name}”？`,
            `Delete “${props.profile.name}”?`,
          ),
          () => props.onAction("profiles.delete"),
          {
            actionLabel: ui.t("删除订阅", "Delete subscription"),
            description: props.profile.active
              ? ui.t(
                  "删除当前订阅会影响正在使用它的连接；不会删除其他订阅。",
                  "Deleting the active subscription affects its connection. Other subscriptions are kept.",
                )
              : ui.t(
                  "仅删除此订阅及其本地副本，其他订阅保持不变。",
                  "Delete only this subscription and its local copies. Other subscriptions are kept.",
                ),
          },
        );
      },
    });
    return items;
  };
  const iconBoxSize = () => (platform.desktop ? 32 : 38);
  const bodyContent = () => (
    <>
      <View
        style={{
          width: iconBoxSize(),
          height: iconBoxSize(),
          borderRadius: radius.lg,
          borderCurve: "continuous",
          borderWidth: 1,
          borderColor: palette().line,
          backgroundColor: highlighted() ? palette().panel : palette().inset,
          alignItems: "center",
          justifyContent: "center",
          flexShrink: 0,
          alignSelf: "flex-start",
          marginTop: density.space(2),
        }}
      >
        <If
          when={props.refreshing}
          else={
            <Icon
              name={props.profile.active ? "check" : "rss"}
              size={platform.desktop ? 16 : 18}
              color={props.profile.active ? palette().accent : palette().muted}
            />
          }
        >
          <Spinner size={18} />
        </If>
      </View>
      <View style={{ flex: 1, minWidth: 0, gap: density.space(4) }}>
        <View
          style={{
            flexDirection: "row",
            gap: spacing.sm,
            alignItems: "center",
            minWidth: 0,
          }}
        >
          <Text
            style={{
              flexShrink: 1,
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
        <If
          when={
            !!props.profile.subscriptionUserInfo?.totalBytes && !props.detail
          }
        >
          <MeterBar
            ratio={Math.min(
              1,
              ((props.profile.subscriptionUserInfo?.uploadBytes || 0) +
                (props.profile.subscriptionUserInfo?.downloadBytes || 0)) /
                (props.profile.subscriptionUserInfo?.totalBytes || 1),
            )}
          />
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
    </>
  );
  return (
    <ObjectMenu
      id={`profile-menu-${idPrefix()}${props.profile.id}`}
      closeId="profile-menu-close"
      title={props.profile.name}
      subtitle={
        props.profile.lastRefreshError
          ? ui.t("更新失败", "Update failed")
          : undefined
      }
      triggerLabel={ui.t("订阅操作", "Subscription actions")}
      triggerIcon="ellipsis"
      triggerStyle={{ marginRight: spacing.sm }}
      commands={commands()}
      row={(menu, state) => (
        <View
          style={{
            marginBottom: props.detail ? 0 : spacing.md,
            flexDirection: "row",
            alignItems: "center",
            borderRadius: radius.xl,
            borderCurve: "continuous",
            borderWidth: 1,
            borderColor: palette().border,
            overflow: "hidden",
            backgroundColor: highlighted()
              ? palette().accentFill
              : state?.contextOpen()
                ? palette().soft
                : palette().panel,
            ...surfaceFinish("card"),
          }}
        >
          <If
            when={!props.detail}
            else={
              <View
                testID={`open-profile-${idPrefix()}${props.profile.id}`}
                style={{
                  flex: 1,
                  paddingHorizontal: spacing.lg,
                  paddingVertical: density.space(14),
                  flexDirection: "row",
                  alignItems: "center",
                  gap: spacing.md,
                }}
              >
                {bodyContent()}
              </View>
            }
          >
            <View
              testID={`open-profile-${idPrefix()}${props.profile.id}`}
              role="button"
              accessibilityLabel={props.profile.name}
              accessibilityState={{
                selected: !!props.selected,
              }}
              focusVisibleStyle={focusRing(0)}
              hoverStyle={{
                backgroundColor: highlighted()
                  ? palette().accentFill
                  : palette().hover,
              }}
              activeStyle={{ backgroundColor: palette().soft }}
              onPress={props.onDetails}
              style={{
                flex: 1,
                paddingHorizontal: spacing.lg,
                paddingVertical: density.space(14),
                flexDirection: "row",
                alignItems: "center",
                gap: spacing.md,
                ...feedbackStyle(),
              }}
            >
              {bodyContent()}
            </View>
          </If>
          {menu}
        </View>
      )}
    />
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
        focusVisibleStyle={focusRing(0)}
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

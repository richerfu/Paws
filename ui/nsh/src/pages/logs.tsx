import {
  computed,
  effect,
  fontScale,
  signal,
  Scroll,
  Text,
  untracked,
  View,
  Clipboard,
  windowSize,
  List,
} from "nsh";
import {
  Action,
  Actions,
  Badge,
  Code,
  confirm,
  Dialog,
  Empty,
  Field,
  Hint,
  palette,
  radius,
  Tabs,
  useApp,
  useLifetime} from "../components";
import { ObjectMenu } from "../object-menu";
import { feedbackStyle } from "../motion";
import { focusRing } from "../theme";
import { bytes, timestamp } from "../locale";
import type { LogRecording, UiState } from "../model";
import { matchesLog } from "../query";
import { PageError } from "../page-error";

type Log = UiState["logs"][number];
export function Logs() {
  const ui = useApp(),
    query = signal(""),
    level = signal("all"),
    history = signal(false),
    selected = signal<Log>();
  const alive = useLifetime(),
    recording = signal<LogRecording>({ enabled: false });
  const rows = computed(() => {
    const duplicates = new Map<string, number>();
    return ui.client.state()!.logs.map((row) => {
      const identity = `${row.timestamp}:${row.level}:${row.message}`;
      const count = duplicates.get(identity) || 0;
      duplicates.set(identity, count + 1);
      return { ...row, key: `${identity}:${count}` };
    }).filter((row) => matchesLog(row, query(), level())).reverse();
  });
  const apply = (data: LogRecording | undefined) => {
    if (alive() && data) {
      recording.set(data);
      ui.logRecording.set(data.enabled);
    }
  };
  const load = async () => apply(await ui.client.run<LogRecording>("logs.recordingStatus"));
  void load();
  let lastIntent = ui.logRecordingIntent();
  effect(() => {
    const intent = ui.logRecordingIntent();
    if (intent !== lastIntent) {
      lastIntent = intent;
      untracked(() => {
        void ui.client
          .run<LogRecording>("logs.setRecording", { enabled: !recording().enabled })
          .then((data) => {
            apply(data);
            if (alive() && data) {
              if (data.lastError) ui.client.error.set(data.lastError);
              else
                ui.client.notice.set(
                  data.enabled
                    ? ui.t("已开始记录日志", "Log recording started")
                    : ui.t("已停止记录日志", "Log recording stopped"),
                );
            }
          });
      });
    }
  });
  const recordingError = () => {
    const domain = ui.client.domains.data("logs");
    return domain && Object.prototype.hasOwnProperty.call(domain, "recordingError")
      ? domain.recordingError
      : recording().lastError;
  };
  const exportArchive = async (fileName: string) => {
    const text = await ui.client.run<string>("logs.readArchive", { fileName });
    if (alive() && text !== undefined)
      await ui.client.run(
        "platform.export",
        { name: fileName, content: text },
        ui.t(`日志已导出：${fileName}`, `Log exported: ${fileName}`),
      );
  };
  const levelTone = (value: string) =>
    ["error", "fatal"].includes(value.toLowerCase())
      ? ("danger" as const)
      : ["warn", "warning"].includes(value.toLowerCase())
        ? ("warning" as const)
        : ("neutral" as const);
  const meta = (row: Log) => `${row.level.toUpperCase()}  ·  ${timestamp(row.timestamp)}`;
  return (
    <View
      style={{
        flex: 1,
        paddingHorizontal: windowSize().width >= 840 ? 24 : 16,
        paddingTop: windowSize().width >= 840 ? 24 : 16,
      }}
    >
      <View style={{ minHeight: 32, flexDirection: "row", alignItems: "center", flexWrap: "wrap", gap: 8 }}>
        <Badge
          tone={recording().enabled ? "success" : "neutral"}
          label={
            recording().enabled
              ? ui.t("正在记录并按天保存", "Recording daily")
              : ui.t("日志记录已关闭", "Log recording off")
          }
        />
        <View style={{ flex: 1 }} />
        <Hint text={`${recording().archives?.length || 0} ${ui.t("个日志文件", "log files")}`} />
      </View>
      <If when={recordingError()}>
        <PageError id="logs-recording-error" text={recordingError() || ""} />
      </If>
      <View style={{ marginTop: 6, marginBottom: 12 }}>
        <Tabs
          idPrefix="logs-tab"
          value={history() ? "history" : "current"}
          options={[
            { value: "current", label: ui.t("当前日志", "Current logs") },
            { value: "history", label: ui.t("历史记录", "History") },
          ]}
          onChange={(tab) => {
            history.set(tab === "history");
            if (tab === "history") void load();
          }}
        />
      </View>
      <If
        when={history()}
        else={
          <>
            <Field
              label={ui.t("搜索日志", "Search logs")}
              placeholder={ui.t("输入关键词、等级或时间", "Search keyword, level or time")}
              value={query()}
              onChange={(value) => query.set(value)}
            />
            <View style={{ marginTop: 12 }}>
              <Tabs
                idPrefix="logs-level"
                value={level()}
                options={[
                  { value: "all", label: ui.t("全部", "All") },
                  { value: "info", label: "Info" },
                  { value: "warn", label: "Warn" },
                  { value: "error", label: "Error" },
                  { value: "debug", label: "Debug" },
                ]}
                onChange={(value) => level.set(value)}
              />
            </View>
            <View style={{ minHeight: 32, flexDirection: "row", alignItems: "center", flexWrap: "wrap", gap: 8 }}>
              <Hint
                text={`${rows().length} / ${ui.client.state()!.logs.length} ${ui.t("条日志", "logs")}`}
              />
              <View style={{ flex: 1 }} />
              <If when={rows().length > 0}>
                <Hint text={ui.t("点击查看完整内容", "Tap to view the full message")} />
              </If>
            </View>
            <List
              bounces={false}
              each={rows()}
              keyExtractor={(row) => row.key}
              estimatedItemSize={96}
              style={{ flex: 1, borderWidth: 1, borderColor: palette().border, borderRadius: radius.lg, overflow: "hidden", backgroundColor: palette().panel }}
              contentStyle={{ paddingBottom: 16 }}
              empty={
                <Empty
                  icon="scroll-text"
                  title={ui.t("暂无日志", "No logs")}
                  text={
                    query()
                      ? ui.t("调整筛选条件后继续查看。", "Adjust the filters to continue.")
                      : recording().enabled
                        ? ui.t("核心和 VPN 事件会显示在这里。", "Core and VPN events appear here.")
                        : ui.t(
                            "开启记录后，核心和 VPN 事件会显示在这里。",
                            "Start recording to see core and VPN events.",
                          )
                  }
                />
              }
            >
              {(row) => (
                <View
                  testID={`log-${row().timestamp}`}
                  role="button"
                  accessibilityLabel={meta(row())}
                  focusStyle={focusRing()} hoverStyle={{ backgroundColor: palette().soft }} activeStyle={{ opacity: 0.85 }}
                  onPress={() => selected.set(row())}
                  style={{
                    minHeight: Math.max(88, 54 * Math.max(1, fontScale()) + 20),
                    paddingHorizontal: 12,
                    paddingVertical: 8,
                    borderBottomWidth: 1,
                    borderBottomColor: palette().line,
                    ...feedbackStyle(),
                    backgroundColor: palette().panel,
                    gap: 4,
                  }}
                >
                  <View style={{ minWidth: 0, flexDirection: "row", alignItems: "center", gap: 8 }}>
                    <Badge tone={levelTone(row().level)} label={row().level.toUpperCase()} />
                    <Text
                      style={{
                        numberOfLines: 1,
                        flex: 1,
                        minWidth: 0,
                        fontSize: 12,
                        lineHeight: 16,
                        color: palette().muted,
                      }}
                    >
                      {timestamp(row().timestamp)}
                    </Text>
                  </View>
                  <Text
                    style={{ numberOfLines: 2, fontSize: 12, lineHeight: 17, color: palette().text }}
                  >
                    {row()
                      .message.replace(/[\n\r]/g, " ")
                      .slice(0, 150)}
                  </Text>
                </View>
              )}
            </List>
          </>
        }
      >
        <List
          bounces={false}
          each={recording().archives || []}
          keyExtractor={(row) => row.fileName}
          estimatedItemSize={80}
          style={{ flex: 1 }}
          contentStyle={{ paddingBottom: 16 }}
          empty={
            <Empty
              icon="history"
              title={ui.t("暂无历史日志", "No log archives")}
              text={ui.t(
                "开启记录后按日保存的日志会显示在这里。",
                "Daily log files appear here after recording starts.",
              )}
            />
          }
        >
          {(archive) => <ObjectMenu id={`archive-menu-${archive().fileName}`} title={archive().fileName}
            commands={[
              { label: ui.t("导出日志…", "Export log…"), icon: "download",
                disabled: ui.client.pending() > 0 || !ui.client.domains.fresh(["logs"]), run: () => { void exportArchive(archive().fileName); } },
              { label: ui.t("删除日志…", "Delete log…"), icon: "trash-2", danger: true,
                disabled: archive().active || ui.client.pending() > 0 || !ui.client.domains.fresh(["logs"]), run: () => {
                  const fileName = archive().fileName;
                  void confirm(ui, ui.t(`删除“${fileName}”？`, `Delete “${fileName}”?`), () => {
                    void ui.client.run<LogRecording>("logs.deleteArchive", { fileName }, ui.t(`日志已删除：${fileName}`, `Log deleted: ${fileName}`)).then(apply);
                  }, { actionLabel: ui.t("删除日志", "Delete log") });
                } },
            ]} row={(menu) => <View style={{ minHeight: Math.max(76, 38 * Math.max(1, fontScale()) + 24),
              paddingHorizontal: 12, paddingVertical: 8, gap: 12,
              flexDirection: "row", alignItems: "center", borderBottomWidth: 1,
              borderBottomColor: palette().line, backgroundColor: palette().panel }}>
              <View style={{ flex: 1, minWidth: 0, gap: 4 }}>
                <Text style={{ numberOfLines: 1, fontSize: 14, lineHeight: 20, fontWeight: "600", color: palette().text }}>{archive().fileName}</Text>
                <Text style={{ numberOfLines: 1, fontSize: 12, lineHeight: 18, color: palette().muted, fontVariant: ["tabular-nums"] }}>
                  {bytes(archive().bytes)} · {timestamp(archive().updatedAt) === "—" ? archive().date || "—" : timestamp(archive().updatedAt)}
                  {archive().active ? ui.t(" · 正在写入", " · Recording") : ""}
                </Text>
              </View>
              {menu}
            </View>} />}
        </List>
      </If>
      <If when={selected()}>
        <Dialog
          title={ui.t("日志详情", "Log details")}
          description={meta(selected()!)}
          onClose={() => selected.set(undefined)}
          footer={<Actions>
            <Action
              secondary
              label={ui.t("关闭", "Close")}
              onPress={() => selected.set(undefined)}
            />
            <Action
              label={ui.t("复制完整内容", "Copy full message")}
              onPress={() => Clipboard.writeText(selected()!.message)}
            />
          </Actions>}
        >
          <Badge tone={levelTone(selected()!.level)} label={selected()!.level.toUpperCase()} />
          <Scroll
            bounces={false}
            style={{ height: Math.min(320, Math.max(120, windowSize().height * 0.3)) }}
          >
            <Code text={selected()!.message} />
          </Scroll>

        </Dialog>
      </If>
    </View>
  );
}

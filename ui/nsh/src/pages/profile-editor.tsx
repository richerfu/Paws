import { cleanup, computed, Scroll, signal, Text, View } from "nsh";
import { Editor, type EditorRef, type EditorSearchResult } from "nsh/base";
import {
  Action, Actions, askConfirmation, Badge, Field, Hint, IconButton,
  palette, radius, Spinner, useApp, useLifetime,
} from "../components";
import { DraftEdits } from "../draft";
import { PageError } from "../page-error";
import { summarizeYaml } from "../yaml-summary";

/** A route owns this draft; NSH owns the code buffer, input and viewport. */
export function ProfileEditor(props: { profileId: string }) {
  const ui = useApp(), alive = useLifetime(), edits = new DraftEdits();
  const text = signal(""), original = signal(""), baseline = signal(0);
  const loaded = signal(false), operation = signal(""), error = signal("");
  const wrap = signal(true), finding = signal(false), query = signal("");
  const matches = signal<EditorSearchResult>({ count: 0, current: 0 });
  let editor!: EditorRef;
  let loadGeneration = 0;
  const busy = () => operation() !== "";
  const dirty = computed(() => loaded() && text() !== original());
  const conflict = () => loaded() && baseline() !== ui.client.state()!.configRevision;
  const statistics = computed(() => summarizeYaml(text()));
  const name = () => ui.client.state()!.profiles.find((p) => p.id === props.profileId)?.name || "";

  cleanup(ui.beforeLeave?.(() => {
    if (busy() && operation() !== "load") return false;
    return !dirty() || askConfirmation(ui, ui.t("放弃未保存的修改并离开？", "Discard unsaved changes and leave?"));
  }) || (() => {}));

  const load = async () => {
    if (busy()) return;
    const generation = ++loadGeneration, revision = ui.client.state()!.configRevision;
    operation.set("load");
    error.set("");
    const value = await ui.client.run<string>("profiles.read", { profileId: props.profileId });
    if (!alive() || generation !== loadGeneration) return;
    operation.set("");
    if (value === undefined) {
      error.set(ui.client.error() || ui.t("读取配置失败，请重试。", "Unable to read the profile. Retry."));
      return;
    }
    const normalized = value.replace(/\r\n?/g, "\n");
    text.set(normalized);
    original.set(normalized);
    baseline.set(revision);
    edits.edit();
    loaded.set(true);
  };
  const save = async () => {
    if (busy() || !loaded() || conflict() || !text().trim()) return;
    operation.set("save");
    error.set("");
    const content = text(), ticket = edits.ticket();
    const result = await ui.client.run("profiles.save", {
      profileId: props.profileId, content, expectedRevision: baseline(),
    }, ui.t("配置已保存", "Profile saved"));
    if (!alive()) return;
    operation.set("");
    if (result === undefined) {
      error.set(ui.client.error() || ui.t("保存失败，请重试。", "Unable to save the profile. Retry."));
      return;
    }
    if (edits.unchangedSince(ticket)) {
      original.set(content);
      baseline.set(ui.client.state()!.configRevision);
      // A committed save with a failed follow-up read stays visible, so its
      // warning cannot be mistaken for an unsaved/retryable mutation.
      if (!ui.client.warning()) ui.back();
    }
  };
  const validate = async () => {
    if (busy() || !loaded() || !text().trim()) return;
    operation.set("validate");
    error.set("");
    const result = await ui.client.run("profiles.validate", { content: text() },
      ui.t("配置校验通过", "Configuration is valid"));
    if (!alive()) return;
    operation.set("");
    if (result === undefined) error.set(ui.client.error());
  };
  const reload = async () => {
    if (busy()) return;
    if (dirty() && !(await askConfirmation(ui, ui.t("重新载入会丢弃当前编辑，继续？", "Discard edits and reload?")))) return;
    if (alive()) void load();
  };
  void load();

  return (
    <View testID="profile-editor-page" style={{ flex: 1, minHeight: 0, paddingBottom: "env(keyboard-inset-height)" }}>
      <View style={{ padding: 16, gap: 12 }}>
        <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
          <Text selectable numberOfLines={2} style={{ flex: 1, minWidth: 0, color: palette().text, fontSize: 16, fontWeight: "600" }}>{name()}</Text>
          <Badge label={dirty() ? ui.t("未保存", "Unsaved") : "YAML"} tone={dirty() ? "warning" : "neutral"} />
        </View>
        <View style={{ flexDirection: "row", flexWrap: "wrap", gap: 8, alignItems: "center" }}>
          <Actions>
            <IconButton id="editor-find" icon="search" label={ui.t("查找", "Find")} disabled={!loaded()} onPress={() => finding.set(!finding())} />
            <Action id="editor-wrap" secondary small selected={wrap()} label={ui.t("自动换行", "Wrap lines")} onPress={() => wrap.set(!wrap())} />
            <Action id="reset-profile" variant="ghost" small label={ui.t("还原", "Reset")} disabled={busy() || !dirty()} onPress={() => {
              text.set(original()); edits.edit(); error.set("");
            }} />
          </Actions>
          <View style={{ marginLeft: "auto" }}>
            <Actions>
              <Action id="validate-profile" secondary small icon="check" label={ui.t("校验", "Validate")} loading={operation() === "validate"} disabled={!loaded() || busy() || !text().trim()} onPress={() => void validate()} />
              <Action id="save-profile" small icon="save" label={ui.t("保存", "Save")} loading={operation() === "save"} disabled={!loaded() || busy() || conflict() || !text().trim()} onPress={() => void save()} />
            </Actions>
          </View>
        </View>
        <If when={finding()}>
          <Field id="editor-search" label={ui.t("查找配置内容", "Find in profile")} value={query()} onChange={(value) => query.set(value)} />
          <Actions>
            <Action secondary small id="editor-find-previous" label={ui.t("上一个", "Previous")} disabled={!query() || !matches().count} onPress={() => void editor.run("findPrevious")} />
            <Action secondary small id="editor-find-next" label={ui.t("下一个", "Next")} disabled={!query() || !matches().count} onPress={() => void editor.run("findNext")} />
            <Hint text={`${matches().count} ${ui.t("处匹配", "matches")}`} />
          </Actions>
        </If>
        <If when={conflict()}>
          <Hint danger text={ui.t("配置已在其他操作中变更，请重新载入后保存。", "Configuration changed elsewhere. Reload before saving.")} />
          <Action id="reload-profile" secondary small label={ui.t("重新载入", "Reload")} disabled={busy()} onPress={() => void reload()} />
        </If>
        <If when={error()}>
          <Scroll bounces={false} style={{ maxHeight: 104 }}>
            <PageError id={`profile-editor-error-${props.profileId}`} text={error()} />
          </Scroll>
        </If>
      </View>
      <If when={loaded()} else={
        <View style={{ flex: 1, alignItems: "center", justifyContent: "center", gap: 16 }}>
          <If when={operation() === "load"} else={<Action id="retry-profile-read" secondary label={ui.t("重新读取", "Retry reading")} onPress={() => void load()} />}>
            <Spinner size={24} />
            <Hint text={ui.t("正在读取配置…", "Reading profile…")} />
          </If>
        </View>
      }>
        <Editor
          ref={editor}
          testID="profile-yaml-editor"
          accessibilityLabel={ui.t("YAML 配置编辑器", "YAML profile editor")}
          value={text()}
          onChange={(value) => { text.set(value); edits.edit(); error.set(""); }}
          language="yaml"
          tabSize={2}
          softWrap={wrap()}
          readOnly={busy()}
          search={finding() && query() ? { query: query() } : undefined}
          onSearchChange={(result) => matches.set(result)}
          style={{ flex: 1, minHeight: 0, marginHorizontal: 16, borderRadius: radius.lg, borderWidth: 1, borderColor: palette().border, backgroundColor: palette().panel }}
        />
      </If>
      <View style={{ paddingHorizontal: 16, paddingVertical: 8 }}>
        <Hint text={`${statistics().lines} ${ui.t("行", "lines")} · ${statistics().characters} ${ui.t("字符", "characters")} · YAML`} />
      </View>
    </View>
  );
}

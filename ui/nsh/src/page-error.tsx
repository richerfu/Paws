import { computed, signal, Text, View, type JSX } from "nsh";
import { Clipboard } from "nsh/clipboard";
import { fontScale, windowSize } from "nsh/device";
import { List } from "nsh/list";
import { Action, Actions, Dialog, palette, useApp } from "./components";
import { statusColors } from "./theme";

const SUMMARY_SCALARS = 160;
const DETAIL_SCALARS = 48;
const DETAIL_LINE_HEIGHT = 20;
const DETAIL_ROW_HEIGHT = 40;
const DETAIL_VIEWPORT_HEIGHT = 320;
const HARD_BREAKS = new Set([10, 11, 12, 13, 0x85, 0x2028, 0x2029]);

function detailScale(scale: number): number {
  return Number.isFinite(scale) ? Math.max(1, scale) : 1;
}

/** Keep the exact source separately from each bounded display row. Hard line
 * separators belong to raw; no surrogate pair or CRLF is split. */
export function errorTextRows(source: string, scale = 1): { key: number; text: string; raw: string }[] {
  const rows: { key: number; text: string; raw: string }[] = [];
  const chunkScalars = Math.max(1, Math.floor(DETAIL_SCALARS / detailScale(scale)));
  let start = 0,
    offset = 0,
    scalars = 0;
  const push = (end: number, displayEnd: number) => {
    rows.push({
      key: rows.length,
      text: source.slice(start, displayEnd),
      raw: source.slice(start, end),
    });
    start = end;
    scalars = 0;
  };
  while (offset < source.length) {
    const value = source.charCodeAt(offset);
    if (HARD_BREAKS.has(value)) {
      const end = offset + (value === 13 && source.charCodeAt(offset + 1) === 10 ? 2 : 1);
      push(end, offset);
      offset = end;
      continue;
    }
    if (scalars === chunkScalars) push(offset, offset);
    offset += source.codePointAt(offset)! > 0xffff ? 2 : 1;
    scalars++;
  }
  if (start < source.length) push(source.length, source.length);
  else if (source.length) rows.push({ key: rows.length, text: "", raw: "" });
  return rows;
}

function preview(source: string): string {
  let end = 0,
    scalars = 0;
  for (const character of source) {
    end += character.length;
    if (++scalars === SUMMARY_SCALARS) break;
  }
  return source.slice(0, end) + (end < source.length ? "…" : "");
}

/** Domain errors can include native stacks. Only a bounded summary is inline;
 * the complete captured error is readable in a virtual list and copyable. */
export function PageError(props: {
  text: string;
  id?: string;
  danger?: boolean;
  renderTrigger?: (open: () => void) => JSX.Element;
}) {
  const ui = useApp(),
    detail = signal<string>(),
    rows = computed(() => errorTextRows(detail() || "", fontScale()));
  // List retains recycled cells, including their last Text. Keep both the
  // retained row count and each row's text small; a larger estimate alone
  // cannot bound a pool that has previously displayed short lines.
  const rowHeight = () => DETAIL_ROW_HEIGHT * detailScale(fontScale());
  const id = () => props.id || "page-error";
  const danger = () => props.danger !== false;
  return (
    <View
      testID={id()}
      style={{ width: props.renderTrigger ? undefined : "100%", minWidth: 0, gap: props.renderTrigger ? 0 : 8 }}
    >
      <If when={!props.renderTrigger}>
        <Text
          testID={`${id()}-summary`}
          numberOfLines={3}
          style={{
            width: "100%",
            minWidth: 0,
            fontSize: 13,
            lineHeight: 20,
            color: danger() ? statusColors("danger").text : palette().muted,
          }}
        >
          {preview(props.text)}
        </Text>
      </If>
      <If
        when={props.renderTrigger}
        else={
          <Actions>
            <Action
              id={`${id()}-details`}
              secondary
              small
              label={ui.t("查看详情", "View details")}
              onPress={() => detail.set(props.text)}
            />
          </Actions>
        }
      >
        {props.renderTrigger?.(() => detail.set(props.text))}
      </If>
      <If when={detail() !== undefined}>
        <Dialog
          title={danger() ? ui.t("错误详情", "Error details") : ui.t("详细信息", "Details")}
          onClose={() => detail.set(undefined)}
        >
          <View
            testID={`${id()}-list`}
            style={{ height: Math.max(20, Math.min(DETAIL_VIEWPORT_HEIGHT, windowSize().height * 0.4)) }}
          >
            <List
              bounces={false}
              each={rows()}
              keyExtractor={(row) => String(row.key)}
              estimatedItemSize={rowHeight()}
              style={{ flex: 1 }}
            >
              {(row) => (
                <Text
                  testID={`${id()}-chunk-${row().key}`}
                  selectable
                  accessibilityLabel={row().raw}
                  style={{
                    alignSelf: "flex-start",
                    maxWidth: "100%",
                    minWidth: 0,
                    minHeight: rowHeight(),
                    fontSize: 13,
                    lineHeight: DETAIL_LINE_HEIGHT,
                    color: palette().text,
                  }}
                >
                  {row().text}
                </Text>
              )}
            </List>
          </View>
          <Actions>
            <Action
              id={`${id()}-close`}
              secondary
              label={ui.t("关闭", "Close")}
              onPress={() => detail.set(undefined)}
            />
            <Action
              id={`${id()}-copy`}
              secondary
              label={danger()
                ? ui.t("复制完整错误", "Copy full error")
                : ui.t("复制完整信息", "Copy full information")}
              onPress={() => Clipboard.setString(detail() || "")}
            />
          </Actions>
        </Dialog>
      </If>
    </View>
  );
}

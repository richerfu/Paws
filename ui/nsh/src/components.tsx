import { density } from "./theme";
import {
  Clipboard,
  cleanup,
  computed,
  context,
  effect,
  keymap,
  platform,
  reduceMotion,
  Scroll,
  signal,
  Text,
  View,
  type JSX,
  type Signal,
  type ViewStyle,
  Animated,
  createAnimatedValue as createNativeValue,
  Easing,
  windowSize,
  fontScale,
} from "nsh";
import {
  Dialog as NativeDialog,
  Input,
  type InputProps,
  Button,
  Switch,
  Checkbox,
  Menu,
  MenuItem,
  Link,
  AlertDialog,
  Tab,
  Tabs as NativeTabs,
  Radio,
  RadioGroup as NativeRadioGroup,
  Tooltip,
} from "nsh/base";
import type { CoreClient } from "./client";
import type { DomainInput } from "./model";
import { isChinese } from "./locale";
import { workspaceLayout } from "./layout";
import {
  palette,
  control,
  radius,
  spacing,
  typography,
  statusColors,
  focusRing,
  surfaceFinish,
  popupSurface,
  type StatusTone,
} from "./theme";
import { Icon, type IconName } from "./icons";
import { NotificationViewport, useNotificationLayer } from "./notifications";
import {
  feedbackStyle,
  follow,
  motion,
  useGlyphTransition,
  usePopIn,
  usePressFeedback,
} from "./motion";
export { palette, control, radius, spacing, typography, Icon };
export type { StatusTone };

export type Page =
  | "dashboard"
  | "profiles"
  | "profile-detail"
  | "proxies"
  | "proxy-group"
  | "traffic"
  | "tools"
  | "settings"
  | "appearance"
  | "connections"
  | "requests"
  | "activity-detail"
  | "resources"
  | "logs"
  | "diagnostics"
  | "advanced-diagnostics"
  | "backups"
  | "converter"
  | "about"
  | "privacy"
  | "editor";
export type AppContext = {
  client: CoreClient;
  page: () => Page;
  route: () => unknown;
  alive: () => boolean;
  navigate: (page: Page, query?: string) => void;
  back: () => void;
  query: () => string;
  importIntent: Signal<number>;
  consumeImportIntent: (id: number) => boolean;
  resourceIntent: Signal<{ id: number; kind: "lookup" | "add" } | undefined>;
  consumeResourceIntent: (id: number) => boolean;
  logRecording: Signal<boolean>;
  logRecordingIntent: Signal<number>;
  refreshProfiles?: () => Promise<void>;
  editProfile: (id: string) => void;
  switchActivity?: (mode: "connections" | "requests") => void;
  beforeLeave?: (guard: () => boolean | Promise<boolean>, owner?: unknown) => () => void;
  layout?: () => ReturnType<typeof workspaceLayout>;
  viewState?: Map<string, unknown>;
  overlays?: Signal<number>;
  t: (zh: string, en: string) => string;
};
export const App = context<AppContext | undefined>(undefined);
export const RouteOwner = context<unknown>(undefined);
export function useApp(): AppContext {
  const value = App();
  if (!value) throw new Error("Paws UI context is missing");
  return value;
}
export function useLayout() {
  const ui = useApp();
  return () => ui.layout?.() || workspaceLayout(windowSize().width, platform.desktop, undefined, fontScale());
}
/** Session-only state for collection/detail pairs. No preference writes. */
export function useViewState<T>(key: string, create: () => T): T {
  const ui = useApp();
  if (!ui.viewState) return create();
  if (!ui.viewState.has(key)) ui.viewState.set(key, create());
  return ui.viewState.get(key) as T;
}
export function useOverlay(open: () => boolean) {
  const ui = App();
  effect(() => {
    if (!open() || !ui?.overlays) return;
    ui.overlays.update((count) => count + 1);
    cleanup(() => ui.overlays!.update((count) => Math.max(0, count - 1)));
  });
}
/** Register every draft with the current route, and reuse its discard choice
 * for modal dismissal. Auto-saved converter drafts use their own policy. */
export function useDraftGuard(dirty: () => boolean, discard: () => void, busy: () => boolean = () => false) {
  const ui = useApp();
  const check = async () => {
    if (busy()) return false;
    if (!dirty()) return true;
    const accepted = await askConfirmation(ui, ui.t("放弃未保存的修改？", "Discard unsaved changes?"), {
      actionLabel: ui.t("放弃修改", "Discard changes"),
      cancelLabel: ui.t("继续编辑", "Keep editing"),
    });
    if (accepted) discard();
    return accepted;
  };
  cleanup(ui.beforeLeave?.(check, RouteOwner() || ui.route()) || (() => {}));
  return check;
}
export function copyText(ui: AppContext, text: string, notice: string): void {
  Clipboard.writeText(text);
  ui.client.notice.set(notice);
}
export function useDomainInterests(inputs: () => DomainInput[]): void {
  const ui = useApp(),
    route = RouteOwner() || ui.route();
  effect(() => {
    if (ui.route() !== route) return;
    const required = inputs();
    if (required.length) cleanup(ui.client.domains.acquire(required));
  });
}
export function SectionHeader(props: {
  title: string;
  meta?: string;
  right?: JSX.Element;
}) {
  return (
    <View
      style={{
        flexDirection: "row",
        alignItems: "center",
        justifyContent: "space-between",
        gap: spacing.sm,
        paddingHorizontal: spacing.xxs,
      }}
    >
      <Text
        role="header"
        style={{
          flex: 1,
          minWidth: 0,
          color: palette().text,
          fontSize: typography.sm,
          lineHeight: density.line(20),
          fontWeight: "600",
        }}
      >
        {props.title}
      </Text>
      <If when={props.meta}>
        <Text
          style={{
            color: palette().muted,
            fontSize: typography.note,
            lineHeight: density.line(18),
            fontVariant: ["tabular-nums"],
          }}
        >
          {props.meta}
        </Text>
      </If>
      {props.right}
    </View>
  );
}

export function IconTile(props: {
  icon: IconName;
  selected?: boolean;
  color?: string;
  alignTop?: boolean;
}) {
  return (
    <View
      style={{
        width: 26,
        height: 26,
        borderRadius: 8,
        borderCurve: "continuous",
        borderWidth: 1,
        alignItems: "center",
        justifyContent: "center",
        flexShrink: 0,
        marginTop: props.alignTop ? 2 : 0,
        ...surfaceFinish("raised"),
      }}
    >
      <Icon
        name={props.icon}
        size={16}
        color={props.color || (props.selected ? palette().accent : palette().text)}
      />
    </View>
  );
}

export function ListSurface(props: {
  children?: JSX.Element;
  id?: string;
  style?: ViewStyle;
}) {
  return (
    <View
      testID={props.id}
      style={{
        minWidth: 0,
        borderRadius: radius.xl,
        borderCurve: "continuous",
        backgroundColor: palette().panel,
        borderWidth: 1,
        borderColor: palette().border,
        overflow: "hidden",
        ...surfaceFinish("card"),
        ...props.style,
      }}
    >
      {props.children}
    </View>
  );
}

export function ListRow(props: {
  id?: string;
  icon?: IconName;
  leading?: JSX.Element;
  title: string;
  badge?: JSX.Element;
  subtitle?: string;
  value?: string;
  valueTone?: StatusTone;
  tabular?: boolean;
  chevron?: boolean;
  trailing?: JSX.Element;
  onPress?: () => void;
  disabled?: boolean;
  selected?: boolean;
  divider?: boolean;
  alignTop?: boolean;
  children?: JSX.Element;
}) {
  const interactive = () => !!props.onPress;
  const content = (
    <>
      <If when={props.leading || props.icon}>
        <If
          when={props.leading}
          else={
            <IconTile
              icon={props.icon!}
              selected={props.selected}
              alignTop={props.alignTop}
            />
          }
        >
          {props.leading}
        </If>
      </If>
      <View style={{ flex: 1, minWidth: 0, gap: density.space(2) }}>
        <View
          style={{
            flexDirection: "row",
            alignItems: "center",
            gap: density.space(6),
            minWidth: 0,
          }}
        >
          <Text
            style={{
              flexShrink: 1,
              minWidth: 0,
              numberOfLines: 1,
              color: palette().text,
              fontSize: typography.sm,
              lineHeight: density.line(20),
              fontWeight: props.selected ? "600" : "500",
            }}
          >
            {props.title}
          </Text>
          {props.badge}
        </View>
        <If when={props.subtitle}>
          <Text
            style={{
              minWidth: 0,
              numberOfLines: 2,
              color: palette().muted,
              fontSize: typography.note,
              lineHeight: density.line(18),
            }}
          >
            {props.subtitle}
          </Text>
        </If>
        {props.children}
      </View>
      <If when={props.value !== undefined || props.trailing || props.chevron}>
        <View
          style={{
            flexDirection: "row",
            alignItems: "center",
            gap: density.space(6),
            flexShrink: 0,
            maxWidth: "52%",
          }}
        >
          <If when={props.value !== undefined}>
            <Text
              style={{
                minWidth: 0,
                flexShrink: 1,
                numberOfLines: 1,
                fontSize: typography.note,
                lineHeight: density.line(18),
                color: props.valueTone
                  ? statusColors(props.valueTone).text
                  : palette().muted,
                fontVariant: props.tabular ? ["tabular-nums"] : undefined,
              }}
            >
              {props.value}
            </Text>
          </If>
          {props.trailing}
          <If when={props.chevron}>
            <Icon name="chevron-right" size={16} color={palette().faint} />
          </If>
        </View>
      </If>
    </>
  );
  const rowStyle = () => ({
    minHeight: Math.max(
      52,
      density.line(20) * Math.max(1, fontScale()) + spacing.lg,
    ),
    paddingHorizontal: 14,
    paddingVertical: density.space(10),
    flexDirection: "row" as const,
    alignItems: (props.alignTop ? "flex-start" : "center") as
      | "flex-start"
      | "center",
    gap: spacing.md,
    borderBottomWidth: props.divider ? 1 : 0,
    borderBottomColor: palette().line,
    backgroundColor: props.selected ? palette().accentFill : "transparent",
    opacity: props.disabled ? 0.45 : 1,
    ...feedbackStyle(),
  });
  return (
    <If
      when={interactive()}
      else={
        <View testID={props.id} style={rowStyle()}>
          {content}
        </View>
      }
    >
      <View
        testID={props.id}
        role="button"
        accessibilityLabel={props.title}
        accessibilityState={{
          disabled: !!props.disabled,
          selected: !!props.selected,
        }}
        disabled={props.disabled}
        onPress={() => {
          if (!props.disabled) props.onPress?.();
        }}
        focusStyle={focusRing()}
        hoverStyle={
          props.disabled ? undefined : { backgroundColor: palette().hover }
        }
        activeStyle={
          props.disabled ? undefined : { backgroundColor: palette().soft }
        }
        style={rowStyle()}
      >
        {content}
      </View>
    </If>
  );
}

export function MeterBar(props: { ratio: number }) {
  const clamped = () =>
    Math.max(0, Math.min(100, Math.round((Number.isFinite(props.ratio) ? props.ratio : 0) * 100)));
  return (
    <View
      style={{
        height: 6,
        width: "100%",
        borderRadius: 3,
        borderWidth: 1,
        backgroundColor: palette().track,
        overflow: "hidden",
        marginTop: density.space(6),
        ...surfaceFinish("track"),
      }}
    >
      <View
        style={{
          height: "100%",
          width: `${clamped()}%`,
          borderRadius: 2,
          backgroundColor: palette().brandFill,
          transitionProperty: "width",
          transitionDuration: reduceMotion() ? 0 : 180,
          transitionTimingFunction: "ease-out",
          ...surfaceFinish("fill"),
        }}
      />
    </View>
  );
}

function splitRate(value: number | undefined, formatBytes: (n: number) => string) {
  if (!value || value <= 0) return { amount: "0", unit: "B/s" };
  const formatted = formatBytes(value);
  const space = formatted.lastIndexOf(" ");
  if (space <= 0) return { amount: formatted, unit: "/s" };
  return {
    amount: formatted.slice(0, space),
    unit: `${formatted.slice(space + 1)}/s`,
  };
}

export function SpeedGrid(props: {
  upload?: number;
  download?: number;
  connected: boolean;
  formatBytes: (n: number) => string;
}) {
  const ui = useApp();
  const stacked = () => windowSize().width < 340 || fontScale() > 1.35;
  const up = () =>
    splitRate(props.connected ? props.upload : 0, props.formatBytes);
  const down = () =>
    splitRate(props.connected ? props.download : 0, props.formatBytes);
  const Cell = (cell: {
    upload?: boolean;
    amount: string;
    unit: string;
    divider?: boolean;
  }) => (
    <View
      style={{
        flex: stacked() ? undefined : 1,
        minWidth: 0,
        paddingHorizontal: 14,
        paddingVertical: density.space(12),
        gap: density.space(4),
        borderLeftWidth: !stacked() && cell.divider ? 1 : 0,
        borderTopWidth: stacked() && cell.divider ? 1 : 0,
        borderColor: palette().line,
      }}
    >
      <View
        style={{
          flexDirection: "row",
          alignItems: "center",
          gap: density.space(4),
        }}
      >
        <Icon
          name={cell.upload ? "arrow-up" : "arrow-down"}
          size={14}
          color={cell.upload ? palette().chartUpload : palette().chartDownload}
        />
        <Text
          style={{
            fontSize: typography.note,
            lineHeight: density.line(18),
            color: palette().muted,
          }}
        >
          {cell.upload ? ui.t("上传", "Upload") : ui.t("下载", "Download")}
        </Text>
      </View>
      <View
        style={{
          flexDirection: "row",
          alignItems: "baseline",
          gap: density.space(4),
          minWidth: 0,
        }}
      >
        <Text
          style={{
            numberOfLines: 1,
            flexShrink: 1,
            fontSize: typography.xl,
            lineHeight: density.line(26),
            fontWeight: "600",
            color: palette().text,
            fontVariant: ["tabular-nums"],
          }}
        >
          {cell.amount}
        </Text>
        <Text
          style={{
            fontSize: typography.note,
            lineHeight: density.line(18),
            fontWeight: "500",
            color: palette().muted,
          }}
        >
          {cell.unit}
        </Text>
      </View>
    </View>
  );
  return (
    <View
      style={{
        minWidth: 0,
        flexDirection: stacked() ? "column" : "row",
        borderRadius: radius.xl,
        borderCurve: "continuous",
        backgroundColor: palette().panel,
        borderWidth: 1,
        borderColor: palette().border,
        overflow: "hidden",
        ...surfaceFinish("card"),
      }}
    >
      <Cell upload amount={up().amount} unit={up().unit} />
      <Cell divider amount={down().amount} unit={down().unit} />
    </View>
  );
}

export function Card(props: {
  title?: string;
  subtitle?: string;
  children?: JSX.Element;
  flush?: boolean;
  right?: JSX.Element;
}) {
  return (
    <View style={{ minWidth: 0, gap: spacing.sm }}>
      <If when={props.title || props.subtitle || props.right}>
        <View
          style={{
            flexDirection: "row",
            alignItems: "flex-end",
            gap: spacing.sm,
            paddingHorizontal: spacing.xxs,
          }}
        >
          <View style={{ flex: 1, minWidth: 0, gap: spacing.xxs }}>
            <If when={props.title}>
              <Text
                role="header"
                style={{
                  color: palette().text,
                  fontSize: typography.sm,
                  lineHeight: props.subtitle ? density.line(22) : density.line(20),
                  fontWeight: "600",
                }}
              >
                {props.title}
              </Text>
            </If>
            <If when={props.subtitle}>
              <Hint text={props.subtitle!} />
            </If>
          </View>
          {props.right}
        </View>
      </If>
      <View
        style={{
          minWidth: 0,
          padding: props.flush ? 0 : spacing.lg,
          gap: spacing.md,
          borderRadius: radius.xl,
          borderCurve: "continuous",
          backgroundColor: palette().panel,
          borderWidth: 1,
          borderColor: palette().border,
          overflow: props.flush ? "hidden" : undefined,
          ...surfaceFinish("card"),
        }}
      >
        {props.children}
      </View>
    </View>
  );
}

export function Action(props: {
  label: string;
  onPress: () => void;
  id?: string;
  secondary?: boolean;
  danger?: boolean;
  stop?: boolean;
  disabled?: boolean;
  selected?: boolean;
  primary?: boolean;
  variant?: "primary" | "stop" | "default" | "outline" | "secondary" | "ghost" | "link";
  small?: boolean;
  fullWidth?: boolean;
  flex?: number;
  icon?: IconName;
  loading?: boolean;
}) {
  const disabled = () => !!props.disabled || !!props.loading;
  const press = usePressFeedback(disabled);
  const glyph = useGlyphTransition(() => (props.loading ? "busy" : props.icon));
  const variant = () =>
    props.stop || props.variant === "stop"
      ? "stop"
      : props.danger
        ? "danger"
        : props.primary
          ? "primary"
          : props.variant || (props.secondary ? "outline" : "default");
  const foreground = () =>
    variant() === "stop"
      ? props.disabled && !props.loading
        ? palette().muted
        : palette().onStop
      : variant() === "danger"
        ? props.primary
          ? palette().dangerText
          : palette().danger
        : variant() === "primary"
          ? props.disabled && !props.loading
            ? palette().muted
            : palette().primaryText
          : variant() === "link"
            ? palette().accent
            : palette().text;
  const raised = () =>
    variant() === "default" ||
    variant() === "secondary" ||
    variant() === "outline";
  const solid = () =>
    variant() === "primary" ||
    variant() === "stop" ||
    (variant() === "danger" && !!props.primary);
  const restFinish = () => {
    if (variant() === "stop") {
      return props.disabled && !props.loading
        ? surfaceFinish("raised")
        : surfaceFinish("stop");
    }
    if (variant() === "primary") {
      return props.disabled && !props.loading
        ? surfaceFinish("raised")
        : surfaceFinish("accent");
    }
    if (variant() === "danger") {
      return props.primary
        ? {
            backgroundColor: palette().danger,
            borderColor: "transparent",
            boxShadow: surfaceFinish("accent").boxShadow,
          }
        : {
            backgroundColor: palette().dangerFill,
            borderColor: palette().danger,
            boxShadow: surfaceFinish("raised").boxShadow,
          };
    }
    if (raised()) {
      return props.selected ? surfaceFinish("on") : surfaceFinish("raised");
    }
    return {
      backgroundColor: props.selected ? palette().soft : "transparent",
      borderColor: "transparent",
    };
  };
  return (
    <Button
      testID={props.id}
      hitSlop={control.hitSlop(props.small ? control.small : control.height)}
      focusStyle={focusRing()}
      hoverStyle={
        disabled()
          ? undefined
          : variant() === "stop"
            ? surfaceFinish("stopHover")
            : solid()
              ? { opacity: 0.94 }
              : raised()
                ? props.selected
                  ? undefined
                  : surfaceFinish("hover")
                : { backgroundColor: palette().hover }
      }
      selected={props.selected}
      selectedStyle={
        raised()
          ? surfaceFinish("on")
          : { borderWidth: 1, borderColor: palette().accent }
      }
      accessibilityLabel={props.label}
      disabled={disabled()}
      onPressDown={press.down}
      onPressUp={press.up}
      activeStyle={
        disabled()
          ? undefined
          : variant() === "stop"
            ? surfaceFinish("stopPressed")
            : solid()
              ? { opacity: 0.88 }
              : raised()
                ? surfaceFinish("pressed")
                : { backgroundColor: palette().soft }
      }
      onPress={() => {
        if (!disabled()) props.onPress();
      }}
      style={{
        width: props.fullWidth ? "100%" : undefined,
        flex: props.flex,
        minWidth: 0,
        maxWidth: "100%",
        flexShrink: 1,
        minHeight: props.small ? control.small : control.height,
        paddingVertical: props.small ? spacing.xxs : density.space(6),
        paddingHorizontal: props.small ? 12 : 14,
        borderRadius: props.small ? 9 : radius.md,
        borderCurve: "continuous",
        flexDirection: "row",
        gap: props.small ? 6 : spacing.sm,
        alignItems: "center",
        justifyContent: "center",
        opacity: props.loading ? 1 : props.disabled ? 0.5 : 1,
        borderWidth: 1,
        ...restFinish(),
        ...feedbackStyle(),
        transform: [{ scale: press.scale }],
      }}
    >
      <If when={props.loading || props.icon}>
        <View style={{ opacity: glyph, transform: [{ scale: glyph }] }}>
          <If
            when={props.loading}
            else={
              <If when={props.icon}>
                <Icon name={props.icon!} size={16} color={foreground()} />
              </If>
            }
          >
            <Spinner size={16} color={foreground()} />
          </If>
        </View>
      </If>
      <Text
        style={{
          minWidth: 0,
          flexShrink: 1,
          lineHeight: density.line(20),
          fontSize: props.small ? density.font(14) : typography.sm,
          fontWeight: variant() === "primary" || variant() === "stop" ? "600" : "500",
          color: foreground(),
        }}
      >
        {props.label}
      </Text>
    </Button>
  );
}

export function IconButton(props: {
  icon: IconName;
  label: string;
  onPress: () => void;
  id?: string;
  disabled?: boolean;
  small?: boolean;
  size?: number;
  iconColor?: string;
  loading?: boolean;
  shortcut?: string;
  tooltip?: boolean;
}) {
  const disabled = () => !!props.disabled || !!props.loading;
  const press = usePressFeedback(disabled);
  const glyph = useGlyphTransition(() => (props.loading ? "busy" : props.icon));
  const size = () =>
    Math.max(
      control.icon,
      props.size || (props.small ? control.small : control.icon),
    );
  return (
    <Button
      testID={props.id}
      tooltip={
        platform.desktop && !disabled() && props.tooltip !== false ? (
          <Tooltip
            style={{
              maxWidth: 260,
              paddingHorizontal: spacing.sm,
              paddingVertical: density.space(4),
              borderRadius: radius.md,
              borderCurve: "continuous",
              backgroundColor: palette().popover,
              borderWidth: 1,
              borderColor: palette().border,
              flexDirection: "row",
              alignItems: "center",
              gap: spacing.xs,
            }}
          >
            <Text style={{ fontSize: density.font(12), color: palette().text }}>
              {props.label}
            </Text>
            <If when={props.shortcut}>
              <Text
                style={{
                  fontSize: density.font(11),
                  color: palette().muted,
                }}
              >
                {keymap.label(props.shortcut!)}
              </Text>
            </If>
          </Tooltip>
        ) : undefined
      }
      hitSlop={control.hitSlop(size())}
      focusStyle={focusRing()}
      hoverStyle={disabled() ? undefined : { backgroundColor: palette().soft }}
      accessibilityLabel={props.label}
      disabled={disabled()}
      onPressDown={press.down}
      onPressUp={press.up}
      onPress={() => {
        if (!disabled()) props.onPress();
      }}
      activeStyle={disabled() ? undefined : { backgroundColor: palette().soft }}
      style={{
        width: size(),
        height: size(),
        borderRadius: radius.md,
        borderCurve: "continuous",
        alignItems: "center",
        justifyContent: "center",
        backgroundColor: "transparent",
        opacity: props.loading ? 1 : props.disabled ? 0.4 : 1,
        transform: [{ scale: press.scale }],
        ...feedbackStyle(),
      }}
    >
      <View style={{ opacity: glyph, transform: [{ scale: glyph }] }}>
        <If
          when={props.loading}
          else={
            <Icon
              name={props.icon}
              size={18}
              color={props.iconColor || palette().text}
            />
          }
        >
          <Spinner size={18} />
        </If>
      </View>
    </Button>
  );
}

export function ExternalLink(props: {
  label: string;
  href: string;
  id?: string;
  small?: boolean;
}) {
  const ui = useApp();
  return (
    <Link
      testID={props.id}
      href={props.href}
      openWith="app"
      hitSlop={control.hitSlop(props.small ? control.small : control.height)}
      onOpen={(event) => {
        void ui.client.run("platform.openUrl", { url: event.href });
      }}
      accessibilityLabel={props.label}
      focusStyle={focusRing()}
      hoverStyle={{ backgroundColor: palette().soft }}
      activeStyle={{ backgroundColor: palette().soft }}
      style={{
        minHeight: props.small ? control.small : control.height,
        maxWidth: "100%",
        minWidth: 0,
        paddingVertical: density.space(6),
        paddingHorizontal: spacing.sm,
        flexDirection: "row",
        alignItems: "center",
        gap: spacing.sm,
        borderRadius: radius.md,
        borderCurve: "continuous",
        cursor: "pointer",
        ...feedbackStyle(),
      }}
    >
      <Text
        style={{
          flexShrink: 1,
          fontSize: props.small ? density.font(12) : density.font(14),
          lineHeight: density.line(20),
          color: palette().accent,
        }}
      >
        {props.label}
      </Text>
      <Icon name="external-link" size={14} color={palette().muted} />
    </Link>
  );
}

export function Separator() {
  return (
    <View
      style={{ height: 1, backgroundColor: palette().line, width: "100%" }}
    />
  );
}
export function Badge(props: {
  label: string;
  secondary?: boolean;
  tone?: StatusTone | "brand" | "info";
  id?: string;
}) {
  const colors = () =>
    props.tone === "brand"
      ? {
          text: palette().accent,
          background: palette().accentFill,
          border: palette().border,
        }
      : props.tone === "info"
        ? {
            text: palette().info,
            background: palette().infoFill,
            border: palette().border,
          }
        : !props.tone || props.tone === "neutral"
          ? {
              text: palette().text,
              background: palette().soft,
              border: palette().border,
            }
          : statusColors(props.tone);
  return (
    <View
      testID={props.id}
      accessible
      accessibilityLabel={props.label}
      style={{
        alignSelf: "flex-start",
        flexShrink: 1,
        minWidth: 0,
        maxWidth: "100%",
        minHeight: 20,
        borderRadius: radius.full,
        borderCurve: "continuous",
        paddingHorizontal: 8,
        paddingVertical: density.space(2),
        borderWidth: 0,
        borderColor: colors().border,
        backgroundColor: colors().background,
        justifyContent: "center",
      }}
    >
      <Text
        style={{
          minWidth: 0,
          fontSize: typography.xs,
          lineHeight: density.line(16),
          fontWeight: !props.tone || props.tone === "neutral" ? "500" : "600",
          fontVariant: ["tabular-nums"],
          color: colors().text,
        }}
      >
        {props.label}
      </Text>
    </View>
  );
}
export function Tabs(props: {
  options: { value: string; label: string; count?: number }[];
  value: string;
  onChange: (value: string) => void;
  idPrefix?: string;
  disabled?: boolean;
  inline?: boolean;
  flex?: number;
  size?: "regular" | "small";
}) {
  const inline = () =>
    props.flex !== undefined
      ? false
      : (props.inline ?? props.size === "small");
  const small = () => props.size === "small";
  return (
    <NativeTabs
      style={{
        flex: props.flex,
        minWidth: props.flex !== undefined ? 0 : undefined,
        alignSelf:
          props.flex !== undefined
            ? "auto"
            : inline()
              ? "flex-start"
              : "stretch",
        maxWidth: "100%",
        minHeight: small() ? control.compactHeight : 36,
        padding: 2,
        flexDirection: "row",
        backgroundColor: palette().inset,
        borderRadius: small() ? 9 : radius.md,
        borderCurve: "continuous",
        borderWidth: 1,
        borderColor: palette().border,
        ...surfaceFinish("track"),
      }}
      indicatorStyle={{
        top: 2,
        bottom: 2,
        borderRadius: small() ? 7 : 8,
        borderCurve: "continuous",
        borderWidth: 1,
        ...surfaceFinish("thumb"),
      }}
    >
      <Each of={props.options} key={(option) => option.value}>
        {(option) => {
          const active = () => props.value === option().value;
          return (
            <Tab
              focusStyle={focusRing()}
              hoverStyle={
                props.disabled ? undefined : { backgroundColor: palette().hover }
              }
              activeStyle={
                props.disabled ? undefined : { backgroundColor: palette().soft }
              }
              testID={
                props.idPrefix ? `${props.idPrefix}-${option().value}` : undefined
              }
              selected={active()}
              accessibilityLabel={option().label}
              disabled={props.disabled}
              onPress={() => props.onChange(option().value)}
              style={{
                flex: inline() ? undefined : 1,
                minWidth: 0,
                minHeight: small() ? 26 : 30,
                flexDirection: "row",
                alignItems: "center",
                justifyContent: "center",
                gap: 5,
                paddingHorizontal: small()
                  ? inline()
                    ? 11
                    : 6
                  : inline()
                    ? 12
                    : 10,
                borderRadius: small() ? 7 : 8,
                borderCurve: "continuous",
                backgroundColor: "transparent",
                opacity: props.disabled ? 0.5 : 1,
                ...feedbackStyle(),
              }}
            >
              <Text
                style={{
                  flexShrink: 1,
                  minWidth: 0,
                  numberOfLines: 1,
                  fontSize: small() ? typography.note : density.font(14),
                  lineHeight: density.line(small() ? 18 : 20),
                  fontWeight: active() ? "600" : "500",
                  color: active() ? palette().text : palette().muted,
                }}
              >
                {option().label}
              </Text>
              <If when={option().count !== undefined}>
                <Text
                  style={{
                    flexShrink: 0,
                    fontSize: typography.xs,
                    lineHeight: density.line(16),
                    fontWeight: "500",
                    fontVariant: ["tabular-nums"],
                    color: active() ? palette().muted : palette().faint,
                  }}
                >
                  {String(option().count)}
                </Text>
              </If>
            </Tab>
          );
        }}
      </Each>
    </NativeTabs>
  );
}

export function CompactPicker(props: {
  id?: string;
  label: string;
  value: string;
  options: { value: string; label: string }[];
  onChange: (value: string) => void;
  disabled?: boolean;
}) {
  const ui = App();
  const open = signal(false);
  useOverlay(open);
  const disabled = () => !!props.disabled;
  const press = usePressFeedback(disabled);
  const selectedLabel = () =>
    props.options.find((option) => option.value === props.value)?.label ||
    props.value;
  return (
    <Menu
      open={open()}
      onOpenChange={(event) => {
        if (!event.open || !disabled()) open.set(event.open);
      }}
      placement="bottom"
      align="end"
      offset={4}
      margin={8}
      popupStyle={popupSurface()}
      popupStartingStyle={{
        opacity: 0,
        transform: [{ scale: 0.96 }, { translateY: -4 }],
      }}
      popupEndingStyle={{ opacity: 0 }}
      backdropStyle={{ backgroundColor: palette().scrim }}
      trigger={
        <View
          testID={props.id}
          role="button"
          accessibilityLabel={props.label}
          accessibilityValue={{ text: selectedLabel() }}
          hitSlop={control.hitSlop(control.compactHeight)}
          accessibilityState={{
            expanded: open(),
            disabled: disabled(),
          }}
          disabled={disabled()}
          onPressDown={press.down}
          onPressUp={press.up}
          focusStyle={focusRing()}
          hoverStyle={disabled() ? undefined : surfaceFinish("hover")}
          activeStyle={disabled() ? undefined : surfaceFinish("pressed")}
          style={{
            minHeight: control.compactHeight,
            paddingLeft: 11,
            paddingRight: 9,
            borderRadius: 9,
            borderCurve: "continuous",
            borderWidth: 1,
            flexDirection: "row",
            alignItems: "center",
            gap: 6,
            opacity: disabled() ? 0.45 : 1,
            ...(open() ? surfaceFinish("on") : surfaceFinish("raised")),
            ...feedbackStyle(),
            transform: [{ scale: press.scale }],
          }}
        >
          <Text
            style={{
              fontSize: typography.note,
              lineHeight: density.line(18),
              fontWeight: "500",
              color: palette().text,
            }}
          >
            {selectedLabel()}
          </Text>
          <Chevron expanded={open()} size={14} />
        </View>
      }
    >
      <View style={{ minWidth: 180 }}>
        <Each of={props.options} key={(option) => option.value}>
          {(option) => (
            <MenuItem
              testID={
                props.id ? `${props.id}-option-${option().value}` : undefined
              }
              role="menuitemradio"
              disabled={props.disabled}
              checked={option().value === props.value}
              accessibilityLabel={option().label}
              onSelect={() => {
                if (option().value !== props.value)
                  props.onChange(option().value);
              }}
              focusStyle={focusRing()}
              hoverStyle={{ backgroundColor: palette().soft }}
              activeStyle={{ backgroundColor: palette().soft }}
              style={{
                minHeight: 40,
                paddingHorizontal: spacing.md,
                paddingVertical: spacing.xs,
                borderRadius: 8,
                borderCurve: "continuous",
                flexDirection: "row",
                alignItems: "center",
                gap: spacing.sm,
                backgroundColor:
                  option().value === props.value
                    ? palette().soft
                    : "transparent",
                ...feedbackStyle(),
              }}
            >
              <Text
                style={{
                  flex: 1,
                  minWidth: 0,
                  fontSize: density.font(14),
                  lineHeight: density.line(20),
                  fontWeight: option().value === props.value ? "600" : "500",
                  color: palette().text,
                }}
              >
                {option().label}
              </Text>
              <View
                style={{
                  width: 16,
                  opacity: option().value === props.value ? 1 : 0,
                }}
              >
                <Icon
                  name="check"
                  size={16}
                  color={ui ? palette().accent : palette().text}
                />
              </View>
            </MenuItem>
          )}
        </Each>
      </View>
    </Menu>
  );
}

export function ChoiceTile(props: {
  id?: string;
  icon: IconName;
  title: string;
  subtitle: string;
  loading?: boolean;
  disabled?: boolean;
  onPress: () => void;
}) {
  const disabled = () => !!props.disabled || !!props.loading;
  const press = usePressFeedback(disabled, 0.97);
  const glyph = useGlyphTransition(() => (props.loading ? "busy" : props.icon));
  return (
    <Button
      testID={props.id}
      accessibilityLabel={`${props.title}，${props.subtitle}`}
      disabled={disabled()}
      onPressDown={press.down}
      onPressUp={press.up}
      focusStyle={focusRing()}
      hoverStyle={disabled() ? undefined : surfaceFinish("hover")}
      activeStyle={disabled() ? undefined : surfaceFinish("pressed")}
      onPress={() => {
        if (!disabled()) props.onPress();
      }}
      style={{
        flex: 1,
        minWidth: 0,
        minHeight: 88,
        paddingVertical: 12,
        paddingHorizontal: 13,
        borderRadius: radius.lg,
        borderCurve: "continuous",
        borderWidth: 1,
        alignItems: "stretch",
        justifyContent: "space-between",
        gap: 10,
        opacity: props.loading ? 1 : props.disabled ? 0.45 : 1,
        ...surfaceFinish("raised"),
        ...feedbackStyle(),
        transform: [{ scale: press.scale }],
      }}
    >
      <View
        style={{
          flexDirection: "row",
          alignItems: "center",
          justifyContent: "space-between",
        }}
      >
        <View
          style={{
            width: 32,
            height: 32,
            borderRadius: 10,
            borderCurve: "continuous",
            borderWidth: 1,
            borderColor: palette().fieldBorder,
            backgroundColor: palette().field,
            alignItems: "center",
            justifyContent: "center",
          }}
        >
          <View style={{ opacity: glyph, transform: [{ scale: glyph }] }}>
            <If
              when={props.loading}
              else={<Icon name={props.icon} size={17} color={palette().text} />}
            >
              <Spinner size={17} color={palette().text} />
            </If>
          </View>
        </View>
        <Icon name="chevron-right" size={15} color={palette().faint} />
      </View>
      <View style={{ minWidth: 0, gap: 2 }}>
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
            fontSize: density.font(12.5),
            lineHeight: density.line(17),
            color: palette().muted,
          }}
        >
          {props.subtitle}
        </Text>
      </View>
    </Button>
  );
}

export function OrRule(props: { label: string }) {
  return (
    <View
      style={{
        flexDirection: "row",
        alignItems: "center",
        gap: 10,
        marginVertical: 2,
      }}
    >
      <View style={{ flex: 1, height: 1, backgroundColor: palette().line }} />
      <Text
        style={{
          fontSize: typography.xs,
          lineHeight: density.line(16),
          color: palette().faint,
        }}
      >
        {props.label}
      </Text>
      <View style={{ flex: 1, height: 1, backgroundColor: palette().line }} />
    </View>
  );
}

export function Well(props: { children?: JSX.Element; style?: ViewStyle }) {
  return (
    <View
      style={{
        minWidth: 0,
        padding: 12,
        gap: spacing.xs,
        borderRadius: radius.lg,
        borderCurve: "continuous",
        borderWidth: 1,
        borderColor: palette().line,
        backgroundColor: palette().inset,
        ...surfaceFinish("inset"),
        ...props.style,
      }}
    >
      {props.children}
    </View>
  );
}

export function FilterChips(props: {
  options: { value: string; label: string }[];
  value: string;
  onChange: (value: string) => void;
  idPrefix?: string;
  trailing?: JSX.Element;
}) {
  return (
    <View
      style={{
        flexDirection: "row",
        flexWrap: "wrap",
        alignItems: "center",
        gap: 6,
      }}
    >
      <Each of={props.options} key={(option) => option.value}>
        {(option) => {
          const active = () => props.value === option().value;
          return (
            <Button
              testID={
                props.idPrefix
                  ? `${props.idPrefix}-${option().value}`
                  : undefined
              }
              selected={active()}
              accessibilityLabel={option().label}
              focusStyle={focusRing()}
              hoverStyle={active() ? undefined : surfaceFinish("hover")}
              activeStyle={surfaceFinish("pressed")}
              onPress={() => props.onChange(option().value)}
              style={{
                minHeight: 30,
                paddingHorizontal: 11,
                borderRadius: 8,
                borderCurve: "continuous",
                borderWidth: 1,
                flexDirection: "row",
                alignItems: "center",
                justifyContent: "center",
                ...(active() ? surfaceFinish("on") : surfaceFinish("raised")),
                ...feedbackStyle(),
              }}
            >
              <Text
                style={{
                  fontSize: density.font(12),
                  lineHeight: density.line(16),
                  fontWeight: active() ? "600" : "500",
                  color: active() ? palette().text : palette().muted,
                }}
              >
                {option().label}
              </Text>
            </Button>
          );
        }}
      </Each>
      {props.trailing}
    </View>
  );
}

export function Actions(props: {
  children?: JSX.Element;
  align?: "start" | "end";
}) {
  return (
    <View
      style={{
        flexDirection: "row",
        flexWrap: "wrap",
        gap: spacing.sm,
        alignItems: "center",
        justifyContent: props.align === "start" ? "flex-start" : "flex-end",
      }}
    >
      {props.children}
    </View>
  );
}

export function Field(props: {
  label: string;
  accessibilityLabel?: string;
  value: string;
  onChange: (value: string) => void;
  onSubmit?: () => void;
  autoFocus?: boolean;
  placeholder?: string;
  multiline?: boolean;
  password?: boolean;
  numeric?: boolean;
  clearable?: boolean;
  leadingIcon?: IconName;
  keyboardType?: InputProps["keyboardType"];
  returnKeyType?: InputProps["returnKeyType"];
  onCompositionChange?: InputProps["onCompositionChange"];
  disabled?: boolean;
  readonly?: boolean;
  description?: string;
  error?: string;
  height?: number;
  id?: string;
}) {
  const focused = signal(false),
    passwordVisible = signal(false),
    ui = App();
  const showClear = () =>
    !!props.clearable &&
    !!props.value &&
    !props.password &&
    !props.disabled &&
    !props.readonly;
  return (
    <View style={{ gap: spacing.xs }}>
      <If when={props.label}>
        <Text
          style={{
            fontSize: typography.sm,
            fontWeight: "500",
            color: palette().text,
          }}
        >
          {props.label}
        </Text>
      </If>
      <View style={{ position: "relative" }}>
        <Input
          testID={props.id}
          accessibilityLabel={
            props.accessibilityLabel ||
            props.label ||
            props.placeholder ||
            ui?.t("文本输入", "Text input") ||
            "Text input"
          }
          value={props.value}
          onChange={(event) => props.onChange(event.value)}
          onCompositionChange={props.onCompositionChange}
          onSubmit={props.multiline ? undefined : props.onSubmit}
          autoFocus={props.autoFocus}
          placeholder={props.placeholder}
          multiline={props.multiline}
          masked={props.password && !passwordVisible()}
          disabled={props.disabled}
          readonly={props.readonly || props.disabled}
          autoCapitalize="none"
          autoCorrect={false}
          keyboardType={
            props.numeric ? "number-pad" : props.keyboardType || "default"
          }
          returnKeyType={props.returnKeyType}
          onFocus={() => focused.set(true)}
          onBlur={() => focused.set(false)}
          style={{
            width: "100%",
            height:
              props.height ||
              (props.multiline
                ? 120
                : Math.max(control.height, 20 * Math.max(1, fontScale()) + 16)),
            fontSize: typography.sm,
            lineHeight: density.line(20),
            color: palette().text,
            placeholderColor: palette().muted,
            paddingLeft: props.leadingIcon
              ? spacing.md + density.space(22)
              : spacing.md,
            paddingRight:
              props.password || showClear() ? control.icon + 4 : spacing.md,
            paddingVertical: props.multiline ? spacing.sm : spacing.xxs,
            opacity: props.disabled ? 0.5 : 1,
            backgroundColor: palette().field,
            borderWidth: 1,
            borderColor: props.error
              ? palette().danger
              : focused()
                ? palette().ring
                : palette().fieldBorder,
            borderRadius: radius.md,
            borderCurve: "continuous",
            ...feedbackStyle(),
          }}
        />
        <If when={props.leadingIcon}>
          <View
            style={{
              pointerEvents: "none",
              position: "absolute",
              left: spacing.md,
              top: 0,
              bottom: 0,
              justifyContent: "center",
            }}
          >
            <Icon name={props.leadingIcon!} size={15} color={palette().muted} />
          </View>
        </If>
        <If when={props.password}>
          <View
            style={{
              position: "absolute",
              right: 1,
              top: 0,
              bottom: 0,
              justifyContent: "center",
            }}
          >
            <IconButton
              icon={passwordVisible() ? "eye-off" : "eye"}
              size={32}
              tooltip={false}
              label={
                ui?.t("显示或隐藏密码", "Show or hide password") ||
                "Show or hide password"
              }
              disabled={props.disabled}
              onPress={() => passwordVisible.set(!passwordVisible())}
            />
          </View>
        </If>
        <If when={showClear()}>
          <View
            style={{
              position: "absolute",
              right: 1,
              top: 0,
              bottom: 0,
              justifyContent: "center",
            }}
          >
            <IconButton
              icon="x"
              size={32}
              tooltip={false}
              label={ui?.t("清除输入", "Clear input") || "Clear input"}
              onPress={() => props.onChange("")}
            />
          </View>
        </If>
      </View>
      <If when={props.error || props.description}>
        <Hint danger={!!props.error} text={props.error || props.description!} />
      </If>
    </View>
  );
}

export function SearchField(props: {
  label?: string;
  value: string;
  onChange: (value: string) => void;
  onSubmit?: () => void;
  autoFocus?: boolean;
  placeholder?: string;
  disabled?: boolean;
  id?: string;
}) {
  const composing = signal(false),
    draft = signal(props.value);
  effect(() => {
    const external = props.value;
    if (!composing()) draft.set(external);
  });
  return (
    <Field
      id={props.id}
      label=""
      accessibilityLabel={props.label || props.placeholder}
      value={draft()}
      onChange={(next) => {
        draft.set(next);
        if (!composing()) props.onChange(next);
      }}
      onCompositionChange={(event) => {
        composing.set(event.composing);
        draft.set(event.text);
        if (!event.composing) props.onChange(event.text);
      }}
      onSubmit={props.onSubmit}
      autoFocus={props.autoFocus}
      placeholder={props.placeholder}
      disabled={props.disabled}
      leadingIcon="search"
      clearable
      returnKeyType="search"
    />
  );
}

export function Toggle(props: {
  label: string;
  description?: string;
  value: boolean;
  onChange: (value: boolean) => void;
  disabled?: boolean;
  insetRow?: boolean;
  divider?: boolean;
}) {
  const position = follow(() => (props.value ? 1 : 0), motion.move);
  return (
    <Switch
      focusStyle={focusRing()}
      accessibilityLabel={props.label}
      checked={props.value}
      disabled={props.disabled}
      onChange={(event) => props.onChange(event.checked)}
      style={{
        flexDirection: "row-reverse",
        alignItems: "center",
        gap: spacing.md,
        minHeight: 48,
        paddingHorizontal: props.insetRow ? spacing.md : 0,
        paddingVertical: props.insetRow ? density.space(10) : 0,
        borderBottomWidth: props.divider ? 1 : 0,
        borderBottomColor: palette().line,
        opacity: props.disabled ? 0.45 : 1,
      }}
      trackStyle={{
        width: 42,
        height: 24,
        borderRadius: radius.full,
        padding: 2,
        borderWidth: 1,
        borderColor: props.value
          ? surfaceFinish("accent").borderColor
          : surfaceFinish("track").borderColor,
        boxShadow: props.value
          ? surfaceFinish("accent").boxShadow
          : surfaceFinish("track").boxShadow,
        backgroundColor: position.interpolate({
          inputRange: [0, 1],
          outputRange: [palette().track, palette().brandFill],
        }),
      }}
      thumbStyle={{
        width: 18,
        height: 18,
        borderRadius: radius.full,
        borderWidth: 1,
        ...(props.value ? surfaceFinish("knobOn") : surfaceFinish("knob")),
        transform: [
          {
            translateX: position.interpolate({
              inputRange: [0, 1],
              outputRange: [0, 18],
            }),
          },
        ],
      }}
    >
      <View style={{ flex: 1, minWidth: 0, gap: density.space(2) }}>
        <Text
          style={{
            color: palette().text,
            fontSize: typography.sm,
            lineHeight: density.line(20),
            fontWeight: "500",
          }}
        >
          {props.label}
        </Text>
        <If when={props.description}>
          <Hint text={props.description!} />
        </If>
      </View>
    </Switch>
  );
}

/** A draft choice committed by its form's Save action, unlike an immediate Switch. */
export function Check(props: {
  label: string;
  description?: string;
  value: boolean;
  onChange: (value: boolean) => void;
  disabled?: boolean;
  insetRow?: boolean;
  divider?: boolean;
}) {
  const selected = follow(() => (props.value ? 1 : 0));
  return (
    <Checkbox
      checked={props.value}
      disabled={props.disabled}
      accessibilityLabel={props.label}
      focusStyle={focusRing()}
      onChange={(event) => props.onChange(event.checked)}
      style={{
        flexDirection: "row",
        alignItems: "center",
        gap: spacing.md,
        minHeight: 46,
        paddingHorizontal: props.insetRow ? spacing.md : 0,
        paddingVertical: props.insetRow ? density.space(8) : 0,
        borderBottomWidth: props.divider ? 1 : 0,
        borderBottomColor: palette().line,
        opacity: props.disabled ? 0.45 : 1,
      }}
      indicatorStyle={{
        width: 22,
        height: 22,
        borderRadius: 7,
        borderCurve: "continuous",
        borderWidth: 1,
        borderColor: props.value
          ? surfaceFinish("accent").borderColor
          : palette().fieldBorder,
        boxShadow: props.value ? surfaceFinish("accent").boxShadow : undefined,
        backgroundColor: props.value ? palette().brandFill : palette().field,
        alignItems: "center",
        justifyContent: "center",
        ...feedbackStyle(),
      }}
      indicator={
        <View style={{ opacity: selected, transform: [{ scale: selected }] }}>
          <Icon
            name="check"
            size={14}
            strokeWidth={2.6}
            color={palette().primaryText}
          />
        </View>
      }
    >
      <View style={{ flex: 1, minWidth: 0, gap: spacing.xxs }}>
        <Text
          style={{
            color: palette().text,
            fontSize: typography.sm,
            lineHeight: density.line(20),
            fontWeight: "500",
          }}
        >
          {props.label}
        </Text>
        <If when={props.description}>
          <Hint text={props.description!} />
        </If>
      </View>
    </Checkbox>
  );
}

export function Chevron(props: { expanded: boolean; size?: number }) {
  const angle = follow(() => (props.expanded ? 180 : 0));
  return (
    <View style={{ pointerEvents: "none", transform: [{ rotate: angle }] }}>
      <Icon
        name="chevron-down"
        size={props.size || 16}
        color={palette().muted}
      />
    </View>
  );
}

export function Detail(props: {
  label: string;
  value: string;
  tone?: StatusTone;
  stacked?: boolean;
  id?: string;
}) {
  const stacked = () =>
    !!props.stacked || fontScale() > 1.3 || windowSize().width < 360;
  return (
    <View
      testID={props.id}
      style={{
        minWidth: 0,
        maxWidth: "100%",
        flexDirection: stacked() ? "column" : "row",
        gap: stacked() ? density.space(4) : density.space(12),
        minHeight: 30,
        alignItems: stacked() ? "stretch" : "flex-start",
      }}
    >
      <Text
        style={{
          minWidth: 0,
          flex: stacked() ? undefined : 1,
          fontSize: typography.note,
          lineHeight: density.line(20),
          color: palette().muted,
        }}
      >
        {props.label}
      </Text>
      <View
        style={{
          minWidth: 0,
          maxWidth: "100%",
          flex: stacked() ? undefined : 2,
          flexDirection: "row",
          justifyContent: stacked() ? "flex-start" : "flex-end",
          alignItems: "flex-start",
        }}
      >
        <Text
          style={{
            userSelect: "text",
            width: "100%",
            minWidth: 0,
            fontSize: density.font(14),
            lineHeight: density.line(20),
            fontWeight: "500",
            fontVariant: ["tabular-nums"],
            color: props.tone ? statusColors(props.tone).text : palette().text,
            textAlign: stacked() ? "left" : "right",
          }}
        >
          {props.value}
        </Text>
      </View>
    </View>
  );
}
export function Empty(props: {
  text?: string;
  title?: string;
  icon?: IconName;
  children?: JSX.Element;
}) {
  const ui = useApp();
  return (
    <View
      style={{
        padding: density.space(36),
        gap: spacing.sm,
        alignItems: "center",
      }}
    >
      <View
        style={{
          width: 44,
          height: 44,
          borderWidth: 1,
          borderRadius: 13,
          borderCurve: "continuous",
          alignItems: "center",
          justifyContent: "center",
          marginBottom: spacing.xxs,
          ...surfaceFinish("raised"),
        }}
      >
        <Icon
          name={props.icon || "database"}
          size={20}
          color={palette().muted}
        />
      </View>
      <If when={props.title}>
        <Text
          style={{
            color: palette().text,
            fontSize: typography.sm,
            fontWeight: "600",
          }}
        >
          {props.title}
        </Text>
      </If>
      <Text
        style={{
          color: palette().muted,
          fontSize: density.font(13),
          lineHeight: density.line(18),
          textAlign: "center",
          maxWidth: 280,
        }}
      >
        {props.text || ui.t("暂无数据", "No data yet")}
      </Text>
      <If when={props.children}>
        <View style={{ marginTop: spacing.xs }}>{props.children}</View>
      </If>
    </View>
  );
}
export function Body(props: { children?: JSX.Element }) {
  const layout = useLayout();
  return (
    <View
      style={{
        padding: layout().inset,
        gap: spacing.section,
        width: "100%",
      }}
    >
      {props.children}
    </View>
  );
}
export function FormFooter(props: { children?: JSX.Element }) {
  return <View style={{ padding: spacing.lg, gap: spacing.sm, borderTopWidth: 1, borderTopColor: palette().line, backgroundColor: palette().background }}>{props.children}</View>;
}

export function Loading(props: { text: string }) {
  return (
    <View
      accessible
      role="progressbar"
      accessibilityLabel={props.text}
      style={{
        flex: 1,
        minHeight: 160,
        padding: spacing.region,
        gap: spacing.md,
        alignItems: "center",
        justifyContent: "center",
      }}
    >
      <Spinner size={20} color={palette().muted} />
      <Hint text={props.text} />
    </View>
  );
}
export function Hint(props: { text: string; danger?: boolean }) {
  return (
    <Text
      style={{
        userSelect: "text",
        minWidth: 0,
        maxWidth: "100%",
        fontSize: density.font(13),
        lineHeight: density.line(20),
        color: props.danger ? statusColors("danger").text : palette().muted,
      }}
    >
      {props.text}
    </Text>
  );
}
export function Code(props: { text: string; lines?: number }) {
  return (
    <Text
      style={{
        userSelect: "text",
        numberOfLines: props.lines,
        minWidth: 0,
        maxWidth: "100%",
        fontSize: density.font(13.5),
        lineHeight: density.line(20),
        color: palette().code,
        fontFamily: "monospace",
      }}
    >
      {props.text}
    </Text>
  );
}

type ConfirmationOptions = {
  actionLabel: string;
  cancelLabel?: string;
  description?: string;
  destructive?: boolean;
};
export async function askConfirmation(
  ui: AppContext,
  title: string,
  options?: ConfirmationOptions,
): Promise<boolean> {
  const route = ui.route();
  ui.overlays?.update((count) => count + 1);
  try {
  const result = await AlertDialog.show({
    title,
    description: options?.description,
    closeOnEscape: true,
    actions: [
      { id: "cancel", label: options?.cancelLabel || ui.t("取消", "Cancel"), role: "cancel" },
      {
        id: "commit",
        label: options?.actionLabel || ui.t("继续", "Continue"),
        role: options?.destructive === false ? "default" : "destructive",
      },
    ],
  });
  return result.action === "commit" && ui.alive() && ui.route() === route;
  } finally { ui.overlays?.update((count) => Math.max(0, count - 1)); }
}
export async function confirm(
  ui: AppContext,
  title: string,
  action: () => void,
  options?: ConfirmationOptions,
): Promise<void> {
  if (await askConfirmation(ui, title, options)) action();
}

export function Dialog(props: {
  title: string;
  subtitle?: string;
  description?: string;
  onClose: () => void;
  afterClose?: () => void;
  children?: JSX.Element;
  footer?: JSX.Element;
  width?: number;
}) {
  const ui = App();
  const notificationLayer = useNotificationLayer();
  const enter = usePopIn();
  useOverlay(() => true);
  // Own Escape so an asynchronous draft decision can keep this dialog open.
  cleanup(keymap.bind("escape", () => props.onClose(), { when: "always" }));
  return (
    <NativeDialog
      open
      closeOnEscape={false}
      closeOnBackdropPress={false}
      closeOnSwipe={false}
      onCancel={props.onClose}
      onClose={props.afterClose}
      animation={reduceMotion() ? "none" : "fade"}
      style={{ flex: 1 }}
    >
      <View style={{ flex: 1, paddingBottom: "env(keyboard-inset-height)" }}>
        <View
          onPress={props.onClose}
          style={{
            position: "absolute",
            top: 0,
            right: 0,
            bottom: 0,
            left: 0,
            backgroundColor: palette().scrim,
          }}
        />
        <View
          style={{
            flex: 1,
            minHeight: 0,
            alignItems: "center",
            justifyContent: "center",
            paddingHorizontal: spacing.lg,
            paddingVertical: spacing.xxl,
          }}
        >
          <View
            accessibilityLabel={props.title}
            style={{
              width: "100%",
              maxWidth: props.width || 480,
              maxHeight: "100%",
              minHeight: 0,
              borderRadius: radius.xxl,
              borderCurve: "continuous",
              backgroundColor: palette().popover,
              borderWidth: 1,
              borderColor: palette().border,
              opacity: enter.opacity,
              transform: [{ scale: enter.scale }],
              ...surfaceFinish("overlay"),
            }}
          >
            <View
              style={{
                flexDirection: "row",
                alignItems: "flex-start",
                gap: spacing.sm,
                paddingLeft: spacing.lg,
                paddingRight: spacing.xs,
                paddingTop: 12,
                paddingBottom: 10,
                borderBottomWidth: 1,
                borderBottomColor: palette().line,
              }}
            >
              <View style={{ flex: 1, minWidth: 0, gap: 2, paddingTop: 4 }}>
                <Text
                  role="header"
                  accessibilityLabel={props.title}
                  style={{
                    minWidth: 0,
                    numberOfLines: 2,
                    fontSize: typography.md,
                    lineHeight: density.line(24),
                    fontWeight: "600",
                    color: palette().text,
                  }}
                >
                  {props.title}
                </Text>
                <If when={props.subtitle}>
                  <Text
                    style={{
                      minWidth: 0,
                      fontSize: typography.note,
                      lineHeight: density.line(18),
                      color: palette().muted,
                    }}
                  >
                    {props.subtitle}
                  </Text>
                </If>
              </View>
              <IconButton
                icon="x"
                tooltip={false}
                label={ui?.t("关闭对话框", "Close dialog") || "Close dialog"}
                id="dialog-close"
                onPress={props.onClose}
              />
            </View>
            <Scroll
              bounces={false}
              keyboardDismissMode="on-drag"
              keyboardShouldPersistTaps="handled"
              style={{ flexShrink: 1, minHeight: 0 }}
              contentStyle={{ padding: spacing.lg, gap: 14 }}
            >
              <If when={props.description}>
                <Hint text={props.description!} />
              </If>
              {props.children}
            </Scroll>
            <If when={props.footer}>
              <View
                style={{
                  padding: spacing.lg,
                  borderTopWidth: 1,
                  borderTopColor: palette().line,
                }}
              >
                {props.footer}
              </View>
            </If>
          </View>
        </View>
        <NotificationViewport
          owner={notificationLayer}
          modal
          dismissLabel={ui?.t("关闭通知", "Dismiss notification")}
        />
      </View>
    </NativeDialog>
  );
}

export function BottomSheet(props: {
  title: string;
  subtitle?: string;
  onClose: () => void;
  afterClose?: () => void;
  children?: JSX.Element;
  footer?: JSX.Element;
}) {
  const ui = App();
  const notificationLayer = useNotificationLayer();
  useOverlay(() => true);
  cleanup(keymap.bind("escape", () => props.onClose(), { when: "always" }));
  return (
    <NativeDialog
      open
      closeOnEscape={false}
      closeOnBackdropPress={false}
      closeOnSwipe={false}
      onCancel={props.onClose}
      onClose={props.afterClose}
      animation={reduceMotion() ? "none" : "slide"}
      backdropStyle={{ backgroundColor: palette().scrim }}
      style={{ flex: 1 }}
    >
      <View
        style={{
          flex: 1,
          justifyContent: "flex-end",
          paddingBottom: "env(keyboard-inset-height)",
        }}
      >
        <View
          onPress={props.onClose}
          style={{
            position: "absolute",
            top: 0,
            right: 0,
            bottom: 0,
            left: 0,
          }}
        />
        <View
          accessibilityLabel={props.title}
          style={{
            width: "100%",
            maxHeight: "88%",
            minHeight: 0,
            borderTopLeftRadius: 20,
            borderTopRightRadius: 20,
            borderCurve: "continuous",
            backgroundColor: palette().popover,
            borderTopWidth: 1,
            borderLeftWidth: 1,
            borderRightWidth: 1,
            borderColor: palette().border,
            ...surfaceFinish("overlay"),
          }}
        >
          <View
            style={{
              width: 36,
              height: 5,
              borderRadius: radius.full,
              backgroundColor: palette().border,
              alignSelf: "center",
              marginTop: 8,
              marginBottom: 4,
            }}
          />
          <View
            style={{
              flexDirection: "row",
              alignItems: "flex-start",
              gap: spacing.sm,
              paddingLeft: spacing.lg,
              paddingRight: spacing.xs,
              paddingBottom: 10,
              borderBottomWidth: 1,
              borderBottomColor: palette().line,
            }}
          >
            <View style={{ flex: 1, minWidth: 0, gap: 2, paddingTop: 4 }}>
              <Text
                role="header"
                accessibilityLabel={props.title}
                style={{
                  minWidth: 0,
                  numberOfLines: 2,
                  fontSize: typography.md,
                  lineHeight: density.line(24),
                  fontWeight: "600",
                  color: palette().text,
                }}
              >
                {props.title}
              </Text>
              <If when={props.subtitle}>
                <Text
                  style={{
                    minWidth: 0,
                    fontSize: typography.note,
                    lineHeight: density.line(18),
                    color: palette().muted,
                  }}
                >
                  {props.subtitle}
                </Text>
              </If>
            </View>
            <IconButton
              icon="x"
              tooltip={false}
              label={ui?.t("关闭面板", "Close sheet") || "Close sheet"}
              id="dialog-close"
              onPress={props.onClose}
            />
          </View>
          <Scroll
            bounces={false}
            keyboardDismissMode="on-drag"
            keyboardShouldPersistTaps="handled"
            style={{ flexShrink: 1, minHeight: 0 }}
            contentStyle={{ padding: spacing.lg, gap: 14 }}
          >
            {props.children}
          </Scroll>
          <If when={props.footer}>
            <View
              style={{
                paddingHorizontal: spacing.lg,
                paddingTop: 12,
                paddingBottom: spacing.lg,
                borderTopWidth: 1,
                borderTopColor: palette().line,
              }}
            >
              {props.footer}
            </View>
          </If>
        </View>
        <NotificationViewport
          owner={notificationLayer}
          modal
          dismissLabel={ui?.t("关闭通知", "Dismiss notification")}
        />
      </View>
    </NativeDialog>
  );
}

export function SheetActionItem(props: {
  id?: string;
  icon: IconName;
  label: string;
  danger?: boolean;
  disabled?: boolean;
  onPress: () => void;
}) {
  const color = () => (props.danger ? palette().danger : palette().text);
  return (
    <Button
      testID={props.id}
      accessibilityLabel={props.label}
      disabled={props.disabled}
      focusStyle={focusRing()}
      hoverStyle={props.disabled ? undefined : { backgroundColor: palette().hover }}
      activeStyle={props.disabled ? undefined : { backgroundColor: palette().soft }}
      onPress={() => {
        if (!props.disabled) props.onPress();
      }}
      style={{
        minHeight: 46,
        paddingHorizontal: 12,
        borderRadius: 9,
        borderCurve: "continuous",
        flexDirection: "row",
        alignItems: "center",
        gap: 10,
        backgroundColor: "transparent",
        opacity: props.disabled ? 0.45 : 1,
        ...feedbackStyle(),
      }}
    >
      <Icon
        name={props.icon}
        size={16}
        color={props.danger ? palette().danger : palette().muted}
      />
      <Text
        style={{
          flex: 1,
          minWidth: 0,
          fontSize: typography.sm,
          lineHeight: density.line(20),
          fontWeight: "500",
          color: color(),
        }}
      >
        {props.label}
      </Text>
    </Button>
  );
}

export function Select(props: {
  label: string;
  value: string;
  options: { value: string; label: string }[];
  onChange: (value: string) => void;
  disabled?: boolean;
  loading?: boolean;
  labelAccessory?: JSX.Element;
  id?: string;
}) {
  const ui = App();
  const open = signal(false);
  useOverlay(open);
  const disabled = () => !!props.disabled;
  const selectedLabel = () =>
    props.options.find((option) => option.value === props.value)?.label ||
    props.value;
  return (
    <View style={{ minWidth: 0, gap: spacing.xxs }}>
      <If when={props.label}>
        <View
          style={{
            flexDirection: "row",
            alignItems: "center",
            gap: spacing.sm,
          }}
        >
          <Text
            style={{
              flex: 1,
              minWidth: 0,
              fontSize: typography.sm,
              lineHeight: density.line(20),
              fontWeight: "500",
              color: palette().text,
            }}
          >
            {props.label}
          </Text>
          {props.labelAccessory}
        </View>
      </If>
      <Menu
        open={open()}
        onOpenChange={(event) => {
          if (!event.open || !disabled()) open.set(event.open);
        }}
        placement="bottom"
        align="start"
        offset={4}
        margin={8}
        popupStyle={popupSurface()}
        popupStartingStyle={{
          opacity: 0,
          transform: [{ scale: 0.96 }, { translateY: -4 }],
        }}
        popupEndingStyle={{ opacity: 0 }}
        backdropStyle={{ backgroundColor: palette().scrim }}
        trigger={
          <View
            testID={props.id}
            role="button"
            accessibilityLabel={
              props.label ||
              ui?.t("选择选项", "Choose an option") ||
              "Choose an option"
            }
            accessibilityValue={{ text: selectedLabel() }}
            hitSlop={control.hitSlop(control.height)}
            accessibilityState={{
              expanded: open(),
              disabled: disabled(),
              busy: !!props.loading,
            }}
            disabled={disabled()}
            focusStyle={focusRing()}
            hoverStyle={
              disabled() ? undefined : { backgroundColor: palette().soft }
            }
            activeStyle={
              disabled() ? undefined : { backgroundColor: palette().soft }
            }
            style={{
              minHeight: control.height,
              minWidth: 0,
              paddingHorizontal: spacing.md,
              paddingVertical: spacing.sm,
              borderWidth: 1,
              borderRadius: radius.md,
              borderCurve: "continuous",
              borderColor: open() ? palette().ring : palette().fieldBorder,
              backgroundColor: open() ? palette().soft : palette().field,
              flexDirection: "row",
              alignItems: "center",
              gap: spacing.sm,
              opacity: disabled() ? 0.4 : 1,
              ...feedbackStyle(),
            }}
          >
            <Text
              style={{
                flex: 1,
                minWidth: 0,
                fontSize: typography.sm,
                lineHeight: density.line(20),
                color: palette().text,
              }}
            >
              {selectedLabel()}
            </Text>
            <If when={props.loading} else={<Chevron expanded={open()} />}>
              <Spinner size={16} />
            </If>
          </View>
        }
      >
        <View testID="select-popover" style={{ minWidth: 0 }}>
          <View
            style={{
              flexDirection: "row",
              alignItems: "center",
              paddingLeft: spacing.md,
            }}
          >
            <Text
              style={{
                flex: 1,
                minWidth: 0,
                fontSize: typography.xs,
                lineHeight: density.line(18),
                color: palette().muted,
              }}
            >
              {props.label}
            </Text>
            <IconButton
              id="select-dismiss"
              icon="x"
              tooltip={false}
              label={ui?.t("关闭选项", "Close choices") || "Close choices"}
              onPress={() => open.set(false)}
            />
          </View>
          <Scroll
            bounces={false}
            keyboardShouldPersistTaps="handled"
            style={{
              maxHeight: Math.max(
                88,
                Math.min(480, windowSize().height * 0.65),
              ),
            }}
          >
            <Each of={props.options} key={(option) => option.value}>
              {(option) => (
                <MenuItem
                  testID={`select-option-${option().value}`}
                  role="menuitemradio"
                  disabled={props.disabled}
                  checked={option().value === props.value}
                  accessibilityLabel={option().label}
                  onSelect={() => {
                    if (option().value !== props.value)
                      props.onChange(option().value);
                  }}
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
                    gap: spacing.sm,
                    backgroundColor:
                      option().value === props.value
                        ? palette().soft
                        : "transparent",
                    ...feedbackStyle(),
                  }}
                >
                  <Text
                    style={{
                      flex: 1,
                      minWidth: 0,
                      fontSize: typography.sm,
                      lineHeight: density.line(20),
                      color: palette().text,
                    }}
                  >
                    {option().label}
                  </Text>
                  <View
                    style={{
                      width: 16,
                      opacity: option().value === props.value ? 1 : 0,
                    }}
                  >
                    <Icon name="check" size={16} color={palette().text} />
                  </View>
                </MenuItem>
              )}
            </Each>
          </Scroll>
        </View>
      </Menu>
    </View>
  );
}

export function RadioGroup(props: {
  label: string;
  value: string;
  options: { value: string; label: string }[];
  onChange: (value: string) => void;
  insetRow?: boolean;
}) {
  return (
    <NativeRadioGroup style={{ gap: props.insetRow ? 0 : spacing.xxs }}>
      <Each of={props.options} key={(option) => option.value}>
        {(option, index) => (
          <RadioChoice
            label={option().label}
            selected={props.value === option().value}
            insetRow={props.insetRow}
            divider={props.insetRow && index() < props.options.length - 1}
            onSelect={() => {
              if (props.value !== option().value)
                props.onChange(option().value);
            }}
          />
        )}
      </Each>
    </NativeRadioGroup>
  );
}
function RadioChoice(props: {
  label: string;
  selected: boolean;
  insetRow?: boolean;
  divider?: boolean;
  onSelect: () => void;
}) {
  const selected = follow(() => (props.selected ? 1 : 0));
  return (
    <Radio
      focusStyle={focusRing()}
      accessibilityLabel={props.label}
      checked={props.selected}
      onChange={props.onSelect}
      hoverStyle={{ backgroundColor: palette().hover }}
      activeStyle={{ backgroundColor: palette().soft }}
      style={{
        minHeight: 48,
        flexDirection: "row",
        alignItems: "center",
        gap: spacing.md,
        paddingVertical: density.space(10),
        paddingHorizontal: spacing.md,
        borderRadius: props.insetRow ? 0 : radius.md,
        borderCurve: props.insetRow ? undefined : "continuous",
        borderBottomWidth: props.divider ? 1 : 0,
        borderBottomColor: palette().line,
        backgroundColor: props.insetRow
          ? "transparent"
          : props.selected
            ? palette().soft
            : "transparent",
        ...feedbackStyle(),
      }}
    >
      <View
        style={{
          width: 22,
          height: 22,
          borderRadius: radius.full,
          borderWidth: 1,
          borderColor: props.selected
            ? surfaceFinish("accent").borderColor
            : palette().fieldBorder,
          boxShadow: props.selected
            ? surfaceFinish("accent").boxShadow
            : undefined,
          backgroundColor: props.selected ? palette().brandFill : palette().field,
          alignItems: "center",
          justifyContent: "center",
        }}
      >
        <View
          style={{
            width: 8,
            height: 8,
            borderRadius: radius.full,
            backgroundColor: palette().primaryText,
            opacity: selected,
            transform: [{ scale: selected }],
          }}
        />
      </View>
      <Text
        style={{
          flex: 1,
          minWidth: 0,
          fontSize: typography.sm,
          lineHeight: density.line(20),
          fontWeight: props.selected ? "600" : "500",
          color: palette().text,
        }}
      >
        {props.label}
      </Text>
    </Radio>
  );
}
export function useLifetime(): () => boolean {
  let alive = true;
  cleanup(() => {
    alive = false;
  });
  return () => alive;
}
export function translator(client: CoreClient) {
  // A theme/advanced preference change must not invalidate every translated
  // binding or recreate option objects in controls throughout the app.
  const chinese = computed(() => isChinese(client.preferences()));
  return (zh: string, en: string) => (chinese() ? zh : en);
}

export function Spinner(props: { size?: number; color?: string }) {
  const ui = App();
  const angle = createNativeValue(0);
  const animation = Animated.loop(
    Animated.timing(angle, {
      toValue: 360,
      duration: 900,
      easing: Easing.linear,
    }),
    { resetBeforeIteration: true },
  );
  effect(() => {
    if (reduceMotion()) {
      animation.stop();
      angle.set(0);
    } else {
      animation.start();
    }
  });
  cleanup(() => animation.stop());
  return (
    <View
      role="progressbar"
      accessibilityLabel={ui?.t("正在加载", "Loading") || "Loading"}
      style={{
        pointerEvents: "none",
        width: density.icon(props.size || 16),
        height: density.icon(props.size || 16),
        transform: [{ rotate: angle }],
      }}
    >
      <Icon
        name="loader-circle"
        size={props.size || 16}
        color={props.color || palette().text}
      />
    </View>
  );
}
export function Alert(props: {
  title?: string;
  text: string;
  tone?: "danger" | "warning" | "info";
  danger?: boolean;
  warning?: boolean;
  info?: boolean;
  children?: JSX.Element;
}) {
  const isDanger = () => props.danger || props.tone === "danger";
  const isWarning = () => props.warning || props.tone === "warning";
  const isInfo = () => props.info || props.tone === "info";
  const tone = (): StatusTone =>
    isDanger() ? "danger" : isWarning() ? "warning" : "neutral";
  return (
    <View
      role="alert"
      style={{
        borderWidth: 1,
        borderColor: isDanger()
          ? palette().danger
          : isWarning()
            ? palette().warning
            : isInfo()
              ? palette().info
              : palette().border,
        borderRadius: radius.lg,
        borderCurve: "continuous",
        paddingHorizontal: spacing.md,
        paddingVertical: density.space(11),
        flexDirection: "row",
        alignItems: "flex-start",
        gap: spacing.sm,
        backgroundColor: isDanger() || isWarning()
          ? statusColors(tone()).background
          : isInfo()
            ? palette().infoFill
            : palette().panel,
      }}
    >
      <Icon
        name={
          isDanger() || isWarning()
            ? "triangle-alert"
            : isInfo()
              ? "info"
              : "circle-check"
        }
        size={16}
        color={
          isDanger()
            ? palette().danger
            : isWarning()
              ? palette().warning
              : isInfo()
                ? palette().info
                : palette().text
        }
      />
      <View style={{ flex: 1, minWidth: 0, gap: density.space(4) }}>
        <If when={props.title}>
          <Text
            style={{
              fontSize: density.font(13),
              lineHeight: density.line(18),
              fontWeight: "600",
              color: props.danger
                ? palette().danger
                : props.warning
                  ? palette().warning
                  : props.info
                    ? palette().info
                    : palette().text,
            }}
          >
            {props.title}
          </Text>
        </If>
        <Text
          style={{
            fontSize: density.font(13),
            lineHeight: density.line(18),
            color: palette().text,
          }}
        >
          {props.text}
        </Text>
        {props.children}
      </View>
    </View>
  );
}

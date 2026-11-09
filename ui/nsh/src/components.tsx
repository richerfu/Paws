import {
  cleanup,
  computed,
  context,
  effect,
  Scroll,
  signal,
  Text,
  View,
  type JSX,
  type Signal,
  Animated,
  createAnimatedValue as createNativeValue,
  Easing,
  windowSize,
  fontScale} from "nsh";
import {
  Dialog as NativeDialog,
  Input,
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
} from "nsh/base";
import type { CoreClient } from "./client";
import type { DomainInput } from "./model";
import { isChinese } from "./locale";
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
import { feedbackStyle, follow, motion, useGlyphTransition, usePressFeedback } from "./motion";
export { palette, control, radius, spacing, typography, Icon };
export type { StatusTone };

export type Page =
  | "dashboard"
  | "profiles"
  | "proxies"
  | "traffic"
  | "tools"
  | "settings"
  | "appearance"
  | "connections"
  | "requests"
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
  editProfile: (id: string) => void;
  beforeLeave?: (guard: () => boolean | Promise<boolean>) => () => void;
  t: (zh: string, en: string) => string;
};
export const App = context<AppContext | undefined>(undefined);
export const RouteOwner = context<unknown>(undefined);
export function useApp(): AppContext {
  const value = App();
  if (!value) throw new Error("Paws UI context is missing");
  return value;
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
export function Card(props: { title?: string; subtitle?: string; children?: JSX.Element; flush?: boolean; right?: JSX.Element }) {
  return (
    <View style={{ minWidth: 0, gap: spacing.sm }}>
      <If when={props.title || props.subtitle || props.right}>
        <View style={{ flexDirection: "row", alignItems: "flex-end", gap: spacing.sm, paddingHorizontal: spacing.xxs }}>
          <View style={{ flex: 1, minWidth: 0, gap: spacing.xxs }}>
            <If when={props.title}>
              <Text role="header" style={{ color: palette().text, fontSize: 15, lineHeight: 22, fontWeight: "600" }}>
                {props.title}
              </Text>
            </If>
            <If when={props.subtitle}><Hint text={props.subtitle!} /></If>
          </View>
          {props.right}
        </View>
      </If>
      <View style={{ minWidth: 0, padding: props.flush ? 0 : spacing.lg, gap: spacing.md,
        borderRadius: radius.xl, borderCurve: "continuous", backgroundColor: palette().panel,
        borderWidth: 1, borderColor: palette().border }}>
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
  disabled?: boolean;
  selected?: boolean;
  primary?: boolean;
  variant?: "primary" | "default" | "outline" | "secondary" | "ghost" | "link";
  small?: boolean;
  icon?: IconName;
  loading?: boolean;
}) {
  const disabled = () => !!props.disabled || !!props.loading;
  const press = usePressFeedback(disabled);
  const glyph = useGlyphTransition(() => props.loading ? "busy" : props.icon);
  const variant = () =>
    props.danger ? "danger" : props.primary ? "primary" : props.variant || (props.secondary ? "outline" : "default");
  const foreground = () =>
    variant() === "danger"
      ? props.primary ? palette().dangerText : palette().danger
      : variant() === "primary"
        ? palette().primaryText
        : variant() === "link" ? palette().accent : palette().text;
  const background = () =>
    variant() === "danger"
      ? props.primary ? palette().danger : palette().dangerFill
      : variant() === "primary"
        ? palette().brandFill
        : variant() === "secondary" || variant() === "default" || props.selected
          ? palette().raised
          : variant() === "outline"
            ? palette().raised
            : "transparent";
  return (
    <Button
      testID={props.id}
      hitSlop={control.hitSlop(props.small ? control.small : control.height)}
      focusStyle={focusRing()}
      hoverStyle={disabled() ? undefined : { opacity: 0.9 }}
      selected={props.selected}
      selectedStyle={{ borderWidth: 1, borderColor: palette().accent }}
      accessibilityLabel={props.label}
      disabled={disabled()}
      onPressDown={press.down}
      onPressUp={press.up}
      activeStyle={disabled() ? undefined : { opacity: 0.86 }}
      onPress={() => {
        if (!disabled()) props.onPress();
      }}
      style={{
        minWidth: 0,
        maxWidth: "100%",
        flexShrink: 1,
        minHeight: props.small ? control.small : control.height,
        paddingVertical: control.compact ? 4 : props.small ? 4 : 6,
        paddingHorizontal: control.compact ? props.small ? 8 : 12 : props.small ? spacing.md : spacing.lg,
        borderRadius: radius.md,
        flexDirection: "row",
        gap: control.compact ? 6 : spacing.sm,
        alignItems: "center",
        justifyContent: "center",
        opacity: props.loading ? 1 : props.disabled ? 0.4 : 1,
        borderWidth: 1,
        borderColor: variant() === "ghost" || variant() === "link" || variant() === "primary" ? "transparent" : palette().border,
        backgroundColor: background(),
        ...(variant() === "primary" ? surfaceFinish("accent")
          : variant() === "outline" || variant() === "secondary" || variant() === "default" ? surfaceFinish("raised") : {}),
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
            <Icon name={props.icon!} size={control.compact ? 14 : 16} color={foreground()} />
          </If>
        }
      >
        <Spinner size={control.compact ? 14 : 16} color={foreground()} />
      </If>
        </View>
      </If>
      <Text
        style={{
          minWidth: 0,
          flexShrink: 1,
          lineHeight: control.compact ? 18 : 20,
          fontSize: props.small ? typography.xs : control.compact ? 13 : typography.sm,
          fontWeight: "500",
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
  loading?: boolean;
}) {
  const disabled = () => !!props.disabled || !!props.loading;
  const press = usePressFeedback(disabled);
  const glyph = useGlyphTransition(() => props.loading ? "busy" : props.icon);
  const size = () => Math.max(control.icon, props.size || (props.small ? control.small : control.icon));
  return (
    <Button
      testID={props.id}
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
      activeStyle={disabled() ? undefined : { opacity: 0.8 }}
      style={{
        width: size(),
        height: size(),
        borderRadius: radius.md,
        alignItems: "center",
        justifyContent: "center",
        backgroundColor: "transparent",
        opacity: props.loading ? 1 : props.disabled ? 0.4 : 1,
        transform: [{ scale: press.scale }],
        ...feedbackStyle(),
      }}
    >
      <View style={{ opacity: glyph, transform: [{ scale: glyph }] }}>
      <If when={props.loading} else={<Icon name={props.icon} size={control.compact ? 16 : 18} color={palette().text} />}>
        <Spinner size={control.compact ? 16 : 18} />
      </If>
      </View>
    </Button>
  );
}

export function ExternalLink(props: { label: string; href: string; id?: string; small?: boolean }) {
  const ui = useApp();
  return <Link testID={props.id} href={props.href} openWith="app"
    hitSlop={control.hitSlop(props.small ? control.small : control.height)}
    onOpen={(event) => { void ui.client.run("platform.openUrl", { url: event.href }); }}
    accessibilityLabel={props.label} focusStyle={focusRing()}
    hoverStyle={{ backgroundColor: palette().soft }} activeStyle={{ opacity: 0.8 }}
    style={{ minHeight: props.small ? control.small : control.height, maxWidth: "100%", minWidth: 0,
      paddingVertical: 6, paddingHorizontal: spacing.sm, flexDirection: "row", alignItems: "center",
      gap: spacing.sm, borderRadius: radius.md, cursor: "pointer", ...feedbackStyle() }}>
    <Text style={{ flexShrink: 1, fontSize: props.small ? 12 : 14, lineHeight: 20,
      color: palette().accent }}>{props.label}</Text>
    <Icon name="external-link" size={14} color={palette().muted} />
  </Link>;
}

export function Separator() {
  return <View style={{ height: 1, backgroundColor: palette().border, width: "100%" }} />;
}
export function Badge(props: {
  label: string;
  secondary?: boolean;
  tone?: StatusTone;
  id?: string;
}) {
  const colors = () => statusColors(props.tone);
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
        minHeight: 22,
        borderRadius: radius.full,
        paddingHorizontal: spacing.sm,
        paddingVertical: 2,
        borderWidth: 0,
        borderColor: colors().border,
        backgroundColor: colors().background,
      }}
    >
      <Text
        style={{
          minWidth: 0,
          fontSize: typography.xs,
          lineHeight: 16,
          fontWeight: "500",
          color: colors().text,
        }}
      >
        {props.label}
      </Text>
    </View>
  );
}
export function Tabs(props: {
  options: { value: string; label: string }[];
  value: string;
  onChange: (value: string) => void;
  idPrefix?: string;
  disabled?: boolean;
}) {
  return (
    <NativeTabs
      style={{
        minHeight: control.height + 4,
        padding: 2,
        flexDirection: "row",
        backgroundColor: palette().soft,
        borderRadius: radius.lg,
        ...surfaceFinish("track"),
      }}
      indicatorStyle={{ top: 2, bottom: 2, borderRadius: radius.md, borderWidth: 1,
        borderColor: palette().border, ...surfaceFinish("raised") }}
    >
      <Each of={props.options} key={(option) => option.value}>
        {(option) => (
          <Tab
            focusStyle={focusRing()}
            testID={props.idPrefix ? `${props.idPrefix}-${option().value}` : undefined}
            selected={props.value === option().value}
            accessibilityLabel={option().label}
            disabled={props.disabled}
            onPress={() => props.onChange(option().value)}
            style={{
              flex: 1,
              minHeight: control.height,
              alignItems: "center",
              justifyContent: "center",
              paddingHorizontal: spacing.sm,
              borderRadius: radius.md,
              backgroundColor: "transparent",
              opacity: props.disabled ? 0.5 : 1,
              ...feedbackStyle(),
            }}
          >
            <Text
              style={{
                fontSize: typography.sm,
                fontWeight: "500",
                color: props.value === option().value ? palette().text : palette().muted,
              }}
            >
              {option().label}
            </Text>
          </Tab>
        )}
      </Each>
    </NativeTabs>
  );
}

export function Actions(props: { children?: JSX.Element; align?: "start" | "end" }) {
  return (
    <View style={{ flexDirection: "row", flexWrap: "wrap", gap: spacing.sm, alignItems: "center",
      justifyContent: props.align === "start" ? "flex-start" : "flex-end" }}>
      {props.children}
    </View>
  );
}

export function Field(props: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  onSubmit?: () => void;
  autoFocus?: boolean;
  placeholder?: string;
  multiline?: boolean;
  password?: boolean;
  numeric?: boolean;
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
  return (
    <View style={{ gap: spacing.xs }}>
      <If when={props.label}>
        <Text style={{ fontSize: typography.sm, fontWeight: "500", color: palette().text }}>
          {props.label}
        </Text>
      </If>
      <View style={{ position: "relative" }}>
        <Input
          testID={props.id}
          accessibilityLabel={props.label || props.placeholder || "Text input"}
          value={props.value}
          onChange={(event) => props.onChange(event.value)}
          onSubmit={props.multiline ? undefined : props.onSubmit}
          autoFocus={props.autoFocus}
          placeholder={props.placeholder}
          multiline={props.multiline}
          masked={props.password && !passwordVisible()}
          disabled={props.disabled}
          readonly={props.readonly || props.disabled}
          autoCapitalize="none"
          autoCorrect={false}
          keyboardType={props.numeric ? "number-pad" : "default"}
          onFocus={() => focused.set(true)}
          onBlur={() => focused.set(false)}
          style={{
            width: "100%",
            height: props.height || (props.multiline ? 120 : Math.max(control.height, 20 * Math.max(1, fontScale()) + 16)),
            fontSize: typography.sm,
            lineHeight: 20,
            color: palette().text,
            placeholderColor: palette().muted,
            paddingLeft: spacing.md,
            paddingRight: props.password ? control.icon + 4 : spacing.md,
            paddingVertical: props.multiline ? spacing.sm : spacing.xxs,
            opacity: props.disabled ? 0.5 : 1,
            backgroundColor: palette().field,
            borderWidth: 1,
            borderColor: props.error ? palette().danger : focused() ? palette().ring : palette().fieldBorder,
            borderRadius: radius.md,
            ...surfaceFinish("field"),
            ...feedbackStyle(),
          }}
        />
        <If when={props.password}>
          <View style={{ position: "absolute", right: 1, top: 0, bottom: 0, justifyContent: "center" }}>
            <IconButton
              icon={passwordVisible() ? "eye-off" : "eye"}
              size={32}
              label={ui?.t("显示或隐藏密码", "Show or hide password") || "Show or hide password"}
              disabled={props.disabled}
              onPress={() => passwordVisible.set(!passwordVisible())}
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

export function Toggle(props: {
  label: string;
  description?: string;
  value: boolean;
  onChange: (value: boolean) => void;
  disabled?: boolean;
}) {
  const position = follow(() => props.value ? 1 : 0, motion.move);
  return (
    <Switch
      focusStyle={focusRing()}
      accessibilityLabel={props.label}
      checked={props.value}
      disabled={props.disabled}
      onChange={(event) => props.onChange(event.checked)}
      style={{ flexDirection: "row-reverse", alignItems: "center", gap: spacing.md,
        minHeight: control.row, opacity: props.disabled ? 0.4 : 1 }}
      trackStyle={{ width: 44, height: 26, borderRadius: radius.full,
        padding: 2, borderWidth: 1, borderColor: palette().border,
        backgroundColor: position.interpolate({ inputRange: [0, 1], outputRange: [palette().soft, palette().brandFill] }),
        ...surfaceFinish("track") }}
      thumbStyle={{ width: 20, height: 20, borderRadius: radius.full,
        backgroundColor: palette().panel, ...surfaceFinish("knob"),
        transform: [{ translateX: position.interpolate({ inputRange: [0, 1], outputRange: [0, 18] }) }] }}
    >
      <View style={{ flex: 1, gap: spacing.xs }}>
        <Text style={{ color: palette().text, fontSize: typography.sm, fontWeight: "500" }}>
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
  label: string; description?: string; value: boolean;
  onChange: (value: boolean) => void; disabled?: boolean;
}) {
  const selected = follow(() => props.value ? 1 : 0);
  return (
    <Checkbox checked={props.value} disabled={props.disabled}
      accessibilityLabel={props.label} focusStyle={focusRing()}
      onChange={(event) => props.onChange(event.checked)}
      style={{ flexDirection: "row", alignItems: "center", gap: spacing.md,
        minHeight: control.row, opacity: props.disabled ? 0.4 : 1 }}
      indicatorStyle={{ width: 20, height: 20, borderRadius: radius.sm,
        borderWidth: 1, borderColor: props.value ? palette().accent : palette().fieldBorder,
        backgroundColor: props.value ? palette().brandFill : palette().field,
        alignItems: "center", justifyContent: "center", ...feedbackStyle() }}
      indicator={<View style={{ opacity: selected, transform: [{ scale: selected }] }}>
        <Icon name="check" size={14} color={palette().primaryText} />
      </View>}
    >
      <View style={{ flex: 1, minWidth: 0, gap: spacing.xxs }}>
        <Text style={{ color: palette().text, fontSize: typography.sm, lineHeight: 20 }}>{props.label}</Text>
        <If when={props.description}><Hint text={props.description!} /></If>
      </View>
    </Checkbox>
  );
}

export function Chevron(props: { expanded: boolean; size?: number }) {
  const angle = follow(() => props.expanded ? 180 : 0);
  return <View style={{ pointerEvents: "none", transform: [{ rotate: angle }] }}>
    <Icon name="chevron-down" size={props.size || 16} color={palette().muted} />
  </View>;
}

export function Detail(props: {
  label: string;
  value: string;
  tone?: StatusTone;
  stacked?: boolean;
  id?: string;
}) {
  const stacked = () => !!props.stacked || fontScale() > 1.3 || windowSize().width < 360;
  return (
    <View
      testID={props.id}
      style={{
        minWidth: 0,
        maxWidth: "100%",
        flexDirection: stacked() ? "column" : "row",
        gap: stacked() ? 4 : 12,
        minHeight: 32,
        alignItems: stacked() ? "stretch" : "flex-start",
      }}
    >
      <Text
        style={{
          minWidth: 0,
          flex: stacked() ? undefined : 1,
          fontSize: 12,
          lineHeight: 20,
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
                fontSize: 14,
                lineHeight: 20,
                fontWeight: "500",
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
export function Empty(props: { text?: string; title?: string; icon?: IconName }) {
  const ui = useApp();
  return (
    <View style={{ padding: 36, gap: spacing.sm, alignItems: "center" }}>
      <View
        style={{
          width: 48,
          height: 48,
          backgroundColor: palette().soft,
          borderRadius: radius.xl,
          alignItems: "center",
          justifyContent: "center",
          marginBottom: spacing.xxs,
        }}
      >
        <Icon name={props.icon || "database"} size={20} color={palette().muted} />
      </View>
      <If when={props.title}>
        <Text style={{ color: palette().text, fontSize: typography.sm, fontWeight: "600" }}>
          {props.title}
        </Text>
      </If>
      <Text style={{ color: palette().muted, fontSize: 14, textAlign: "center" }}>
        {props.text || ui.t("暂无数据", "No data yet")}
      </Text>
    </View>
  );
}
export function Body(props: { children?: JSX.Element }) {
  return (
    <View
      style={{
        padding: windowSize().width >= 840 ? spacing.xxl : spacing.lg,
        gap: spacing.section,
        width: "100%",
      }}
    >
      {props.children}
    </View>
  );
}

export function Loading(props: { text: string }) {
  return <View accessible role="progressbar" accessibilityLabel={props.text}
    style={{ flex: 1, minHeight: 160, padding: spacing.region, gap: spacing.md,
      alignItems: "center", justifyContent: "center" }}>
    <Spinner size={20} color={palette().muted} />
    <Hint text={props.text} />
  </View>;
}
export function Hint(props: { text: string; danger?: boolean }) {
  return (
    <Text
      style={{
        userSelect: "text",
        minWidth: 0,
        maxWidth: "100%",
        fontSize: 13,
        lineHeight: 20,
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
        fontSize: 12,
        lineHeight: 19,
        color: palette().text,
        fontFamily: "monospace",
      }}
    >
      {props.text}
    </Text>
  );
}

type ConfirmationOptions = { actionLabel: string; description?: string; destructive?: boolean };
export async function askConfirmation(ui: AppContext, title: string, options?: ConfirmationOptions): Promise<boolean> {
  const route = ui.route();
  const result = await AlertDialog.show({ title, description: options?.description, closeOnEscape: true,
    actions: [
      { id: "cancel", label: ui.t("取消", "Cancel"), role: "cancel" },
      { id: "commit", label: options?.actionLabel || ui.t("继续", "Continue"),
        role: options?.destructive === false ? "default" : "destructive" },
    ],
  });
  return result.action === "commit" && ui.alive() && ui.route() === route;
}
export async function confirm(ui: AppContext, title: string, action: () => void, options?: ConfirmationOptions): Promise<void> {
  if (await askConfirmation(ui, title, options)) action();
}

export function Dialog(props: {
  title: string;
  description?: string;
  onClose: () => void;
  children?: JSX.Element;
  footer?: JSX.Element;
  width?: number;
}) {
  const notificationLayer = useNotificationLayer();
  return (
    <NativeDialog open closeOnEscape closeOnBackdropPress={false} closeOnSwipe={false}
      onCancel={props.onClose} animation="fade" style={{ flex: 1 }}>
      <View style={{ flex: 1, paddingBottom: "env(keyboard-inset-height)" }}>
        <View onPress={props.onClose} style={{ position: "absolute", top: 0, right: 0,
          bottom: 0, left: 0, backgroundColor: palette().scrim }} />
        <View style={{ flex: 1, minHeight: 0, alignItems: "center", justifyContent: "center",
          paddingHorizontal: spacing.lg, paddingVertical: spacing.xxl }}>
          <View accessibilityLabel={props.title} style={{ width: "100%", maxWidth: props.width || 480,
            maxHeight: "100%", minHeight: 0, borderRadius: radius.xxl, borderCurve: "continuous",
            backgroundColor: palette().popover, borderWidth: 1, borderColor: palette().border,
            ...surfaceFinish("overlay") }}>
            <View style={{ flexDirection: "row", alignItems: "center", gap: spacing.sm,
              paddingLeft: spacing.lg, paddingRight: spacing.xxs, paddingTop: spacing.xxs }}>
              <Text role="header" accessibilityLabel={props.title} style={{ flex: 1, minWidth: 0, numberOfLines: 2, fontSize: typography.lg,
                lineHeight: 24, fontWeight: "600", color: palette().text }}>{props.title}</Text>
              <IconButton icon="x" label="Close dialog" id="dialog-close" onPress={props.onClose} />
            </View>
            <Scroll bounces={false} style={{ flexShrink: 1, minHeight: 0 }}
              contentStyle={{ padding: spacing.lg, gap: spacing.lg }}>
              <If when={props.description}><Hint text={props.description!} /></If>
              {props.children}
            </Scroll>
            <If when={props.footer}>
              <View style={{ padding: spacing.lg, borderTopWidth: 1, borderTopColor: palette().line }}>
                {props.footer}
              </View>
            </If>
          </View>
        </View>
        <NotificationViewport owner={notificationLayer} modal />
      </View>
    </NativeDialog>
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
  const open = signal(false);
  const disabled = () => !!props.disabled;
  return (
    <View style={{ minWidth: 0, gap: spacing.xxs }}>
      <If when={props.label}>
        <View style={{ flexDirection: "row", alignItems: "center", gap: spacing.sm }}>
          <Text style={{ flex: 1, minWidth: 0, fontSize: typography.sm, lineHeight: 20,
            fontWeight: "500", color: palette().text }}>{props.label}</Text>
          {props.labelAccessory}
        </View>
      </If>
      <Menu open={open()} onOpenChange={(event) => { if (!event.open || !disabled()) open.set(event.open); }}
        placement="bottom" align="start" offset={4} margin={8}
        popupStyle={popupSurface()}
        popupStartingStyle={{ opacity: 0, transform: [{ translateY: -4 }] }}
        popupEndingStyle={{ opacity: 0 }}
        backdropStyle={{ backgroundColor: palette().scrim }}
        trigger={<View testID={props.id} role="button" accessibilityLabel={props.label || "Choose an option"}
          hitSlop={control.hitSlop(control.height)}
          accessibilityState={{ expanded: open(), disabled: disabled(), busy: !!props.loading }}
          disabled={disabled()} focusStyle={focusRing()}
          hoverStyle={disabled() ? undefined : { backgroundColor: palette().soft }}
          activeStyle={disabled() ? undefined : { opacity: 0.85 }}
          style={{ minHeight: control.height, minWidth: 0, paddingHorizontal: spacing.md,
            paddingVertical: spacing.sm, borderWidth: 1, borderRadius: radius.md,
            borderColor: open() ? palette().ring : palette().fieldBorder,
            backgroundColor: open() ? palette().soft : palette().field,
            flexDirection: "row", alignItems: "center", gap: spacing.sm,
            opacity: disabled() ? 0.4 : 1, ...feedbackStyle() }}>
          <Text style={{ flex: 1, minWidth: 0, fontSize: typography.sm, lineHeight: 20, color: palette().text }}>
            {props.options.find((option) => option.value === props.value)?.label || props.value}
          </Text>
          <If when={props.loading} else={<Chevron expanded={open()} />}><Spinner size={16} /></If>
        </View>}>
        <View testID="select-popover" style={{ minWidth: 0 }}>
          <View style={{ flexDirection: "row", alignItems: "center", paddingLeft: spacing.md }}>
            <Text style={{ flex: 1, minWidth: 0, fontSize: typography.xs, lineHeight: 18, color: palette().muted }}>
              {props.label}
            </Text>
            <IconButton id="select-dismiss" icon="x" label="Close choices" onPress={() => open.set(false)} />
          </View>
          <Scroll bounces={false} style={{ maxHeight: Math.max(88, Math.min(480, windowSize().height * 0.65)) }}>
            <Each of={props.options} key={(option) => option.value}>
              {(option) => <MenuItem testID={`select-option-${option().value}`} role="menuitemradio"
                disabled={props.disabled}
                checked={option().value === props.value} accessibilityLabel={option().label}
                onSelect={() => { if (option().value !== props.value) props.onChange(option().value); }}
                focusStyle={focusRing()} hoverStyle={{ backgroundColor: palette().soft }}
                activeStyle={{ backgroundColor: palette().soft }}
                style={{ minHeight: control.row, paddingHorizontal: spacing.md, paddingVertical: spacing.sm,
                  borderRadius: radius.sm, flexDirection: "row", alignItems: "center", gap: spacing.sm,
                  backgroundColor: option().value === props.value ? palette().soft : "transparent", ...feedbackStyle() }}>
                <Text style={{ flex: 1, minWidth: 0, fontSize: typography.sm, lineHeight: 20, color: palette().text }}>
                  {option().label}
                </Text>
                <View style={{ width: 16, opacity: option().value === props.value ? 1 : 0 }}>
                  <Icon name="check" size={16} color={palette().text} />
                </View>
              </MenuItem>}
            </Each>
          </Scroll>
        </View>
      </Menu>
    </View>
  );
}

export function RadioGroup(props: {
  label: string; value: string; options: { value: string; label: string }[]; onChange: (value: string) => void;
}) {
  return <NativeRadioGroup style={{ gap: spacing.xxs }}>
    <Each of={props.options} key={(option) => option.value}>
      {(option) => <RadioChoice label={option().label} selected={props.value === option().value}
        onSelect={() => { if (props.value !== option().value) props.onChange(option().value); }} />}
    </Each>
  </NativeRadioGroup>;
}
function RadioChoice(props: { label: string; selected: boolean; onSelect: () => void }) {
  const selected = follow(() => props.selected ? 1 : 0);
  return <Radio focusStyle={focusRing()} accessibilityLabel={props.label} checked={props.selected}
    onChange={props.onSelect} hoverStyle={{ backgroundColor: palette().soft }} activeStyle={{ opacity: 0.8 }}
    style={{ minHeight: control.row, flexDirection: "row", alignItems: "center", gap: spacing.md,
      paddingVertical: spacing.sm, paddingHorizontal: spacing.md, borderRadius: radius.md,
      backgroundColor: props.selected ? palette().soft : "transparent", ...feedbackStyle() }}>
    <View style={{ width: 20, height: 20, borderRadius: radius.full, borderWidth: 1,
      borderColor: props.selected ? palette().accent : palette().fieldBorder, alignItems: "center", justifyContent: "center" }}>
      <View style={{ width: 8, height: 8, borderRadius: radius.full, backgroundColor: palette().accent,
        opacity: selected, transform: [{ scale: selected }] }} />
    </View>
    <Text style={{ flex: 1, minWidth: 0, fontSize: typography.sm, lineHeight: 20, color: palette().text }}>{props.label}</Text>
  </Radio>;
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
  const angle = createNativeValue(0);
  const animation = Animated.loop(
    Animated.timing(angle, { toValue: 360, duration: 900, easing: Easing.linear }),
    { resetBeforeIteration: true },
  );
  animation.start();
  cleanup(() => animation.stop());
  return (
    <View
      role="progressbar"
      accessibilityLabel="Loading"
      style={{ pointerEvents: "none", width: props.size || 16, height: props.size || 16, transform: [{ rotate: angle }] }}
    >
      <Icon name="loader-circle" size={props.size || 16} color={props.color || palette().text} />
    </View>
  );
}
export function Alert(props: {
  title?: string;
  text: string;
  danger?: boolean;
  warning?: boolean;
  children?: JSX.Element;
}) {
  return (
    <View
      role="alert"
      style={{
        borderWidth: 1,
        borderColor: props.danger
          ? palette().danger
          : props.warning
            ? palette().warning
            : palette().border,
        borderRadius: radius.lg,
        padding: 16,
        flexDirection: "row",
        alignItems: "flex-start",
        gap: 12,
        backgroundColor: palette().background,
      }}
    >
      <Icon
        name={props.danger || props.warning ? "triangle-alert" : "circle-check"}
        size={16}
        color={props.danger ? palette().danger : props.warning ? palette().warning : palette().text}
      />
      <View style={{ flex: 1, gap: 4 }}>
        <If when={props.title}>
          <Text
            style={{
              fontSize: 14,
              fontWeight: "500",
              color: props.danger
                ? palette().danger
                : props.warning
                  ? palette().warning
                  : palette().text,
            }}
          >
            {props.title}
          </Text>
        </If>
        <Text
          style={{
            fontSize: 14,
            lineHeight: 20,
            color: props.danger
              ? palette().danger
              : props.warning
                ? palette().warning
                : palette().text,
          }}
        >
          {props.text}
        </Text>
        {props.children}
      </View>
    </View>
  );
}

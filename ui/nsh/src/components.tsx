import {
  cleanup,
  computed,
  context,
  effect,
  Input,
  Scroll,
  signal,
  Text,
  View,
  type JSX,
  type Signal,
  type YNode,
} from "nsh";
import { Animated, createNativeValue, Easing } from "nsh/animated";
import { windowSize } from "nsh/device";
import { keyboard, Modal, safeArea, useSafeAreaInsets } from "nsh/ui/mobile";
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
  type StatusTone,
} from "./theme";
import { Icon, type IconName } from "./icons";
import { NotificationViewport, useNotificationLayer } from "./notifications";
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
  confirmation: Signal<{ title: string; resolve: (accepted: boolean) => void } | undefined>;
  editProfile: (id: string) => void;
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
export function Card(props: { title?: string; subtitle?: string; children?: JSX.Element }) {
  return (
    <View
      style={{
        minWidth: 0,
        padding: spacing.lg,
        gap: spacing.md,
        borderRadius: radius.lg,
        backgroundColor: palette().panel,
        borderWidth: 1,
        borderColor: palette().border,
        boxShadow: "0px 1px 2px #0000000d",
      }}
    >
      <If when={props.title}>
        <Text
          style={{
            minWidth: 0,
            maxWidth: "100%",
            color: palette().text,
            fontSize: typography.md,
            lineHeight: 22,
            fontWeight: "600",
            letterSpacing: -0.35,
          }}
        >
          {props.title}
        </Text>
      </If>
      <If when={props.subtitle}>
        <Text
          style={{
            minWidth: 0,
            maxWidth: "100%",
            color: palette().muted,
            fontSize: typography.sm,
            lineHeight: 20,
          }}
        >
          {props.subtitle}
        </Text>
      </If>
      {props.children}
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
  variant?: "primary" | "outline" | "secondary" | "ghost" | "link";
  small?: boolean;
  icon?: IconName;
  loading?: boolean;
}) {
  const hovering = signal(false);
  const disabled = () => !!props.disabled || !!props.loading;
  const variant = () =>
    props.danger ? "danger" : props.variant || (props.secondary ? "outline" : "primary");
  const foreground = () =>
    variant() === "danger"
      ? palette().dangerText
      : variant() === "primary"
        ? palette().primaryText
        : palette().text;
  const background = () =>
    variant() === "danger"
      ? palette().danger
      : variant() === "primary"
        ? palette().accent
        : variant() === "secondary" || props.selected || hovering()
          ? palette().soft
          : variant() === "outline"
            ? palette().background
            : "transparent";
  return (
    <View
      testID={props.id}
      class="focus-visible:ring-2 focus-visible:ring-[#71717a]"
      accessibilityRole="button"
      accessibilityLabel={props.label}
      accessibilityState={{
        disabled: disabled(),
        busy: !!props.loading,
        selected: !!props.selected,
      }}
      disabled={disabled()}
      activeOpacity={0.65}
      onPress={() => {
        if (!disabled()) props.onPress();
      }}
      onHoverIn={() => hovering.set(true)}
      onHoverOut={() => hovering.set(false)}
      style={{
        minWidth: 0,
        maxWidth: "100%",
        flexShrink: 1,
        minHeight: props.small ? control.small : control.height,
        paddingVertical: props.small ? 4 : 6,
        paddingHorizontal: props.small ? spacing.md : spacing.lg,
        borderRadius: radius.md,
        flexDirection: "row",
        gap: spacing.sm,
        alignItems: "center",
        justifyContent: "center",
        opacity: disabled() ? 0.5 : 1,
        borderWidth: variant() === "outline" ? 1 : 0,
        borderColor: palette().border,
        backgroundColor: background(),
      }}
    >
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
      <Text
        style={{
          minWidth: 0,
          flexShrink: 1,
          lineHeight: 20,
          fontSize: props.small ? typography.xs : typography.sm,
          fontWeight: "500",
          color: foreground(),
        }}
      >
        {props.label}
      </Text>
    </View>
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
  const hover = signal(false);
  const disabled = () => !!props.disabled || !!props.loading;
  return (
    <View
      testID={props.id}
      class="focus-visible:ring-2 focus-visible:ring-[#71717a]"
      accessibilityRole="button"
      accessibilityLabel={props.label}
      accessibilityState={{ disabled: disabled(), busy: !!props.loading }}
      disabled={disabled()}
      onPress={() => {
        if (!disabled()) props.onPress();
      }}
      onHoverIn={() => hover.set(true)}
      onHoverOut={() => hover.set(false)}
      activeOpacity={0.65}
      style={{
        width: props.size || (props.small ? control.small : control.icon),
        height: props.size || (props.small ? control.small : control.icon),
        borderRadius: radius.md,
        alignItems: "center",
        justifyContent: "center",
        backgroundColor: hover() ? palette().soft : "transparent",
        opacity: disabled() ? 0.5 : 1,
      }}
    >
      <If when={props.loading} else={<Icon name={props.icon} size={18} color={palette().text} />}>
        <Spinner size={18} />
      </If>
    </View>
  );
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
        borderRadius: radius.md,
        paddingHorizontal: spacing.sm,
        paddingVertical: 2,
        borderWidth: 1,
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
    <View
      accessibilityRole="tablist"
      style={{
        height: 36,
        padding: 3,
        flexDirection: "row",
        backgroundColor: palette().soft,
        borderRadius: radius.lg,
      }}
    >
      <Each of={props.options} key={(option) => option.value}>
        {(option) => (
          <View
            class="focus-visible:ring-2 focus-visible:ring-[#71717a]"
            testID={props.idPrefix ? `${props.idPrefix}-${option().value}` : undefined}
            accessibilityRole="tab"
            accessibilityLabel={option().label}
            accessibilityState={{
              selected: props.value === option().value,
              disabled: !!props.disabled,
            }}
            disabled={props.disabled}
            onPress={() => props.onChange(option().value)}
            style={{
              flex: 1,
              height: 30,
              alignItems: "center",
              justifyContent: "center",
              borderRadius: radius.md,
              backgroundColor:
                props.value === option().value ? palette().background : "transparent",
              opacity: props.disabled ? 0.5 : 1,
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
          </View>
        )}
      </Each>
    </View>
  );
}

export function Actions(props: { children?: JSX.Element }) {
  return (
    <View style={{ flexDirection: "row", flexWrap: "wrap", gap: 8, alignItems: "center" }}>
      {props.children}
    </View>
  );
}

export function Field(props: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  multiline?: boolean;
  password?: boolean;
  numeric?: boolean;
  disabled?: boolean;
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
          accessibilityLabel={props.label}
          value={props.value}
          onChangeText={props.onChange}
          placeholder={props.placeholder}
          placeholderTextColor={palette().muted}
          multiline={props.multiline}
          secureTextEntry={props.password && !passwordVisible()}
          editable={!props.disabled}
          autoCapitalize="none"
          autoCorrect={false}
          keyboardType={props.numeric ? "number-pad" : "default"}
          onFocus={() => focused.set(true)}
          onBlur={() => focused.set(false)}
          style={{
            width: "100%",
            height: props.height || (props.multiline ? 120 : control.height),
            fontSize: typography.sm,
            lineHeight: 20,
            color: palette().text,
            paddingLeft: spacing.md,
            paddingRight: props.password ? 40 : spacing.md,
            paddingVertical: props.multiline ? spacing.sm : spacing.xxs,
            opacity: props.disabled ? 0.5 : 1,
            backgroundColor: palette().background,
            borderWidth: 1,
            borderColor: focused() ? palette().ring : palette().border,
            borderRadius: radius.md,
          }}
        />
        <If when={props.password}>
          <View style={{ position: "absolute", right: 2, top: 2 }}>
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
  return (
    <View style={{ flexDirection: "row", alignItems: "center", gap: spacing.md, minHeight: 40 }}>
      <View style={{ flex: 1, gap: spacing.xs }}>
        <Text style={{ color: palette().text, fontSize: typography.sm, fontWeight: "500" }}>
          {props.label}
        </Text>
        <If when={props.description}>
          <Hint text={props.description!} />
        </If>
      </View>
      <View
        class="focus-visible:ring-2 focus-visible:ring-[#71717a]"
        accessibilityRole="switch"
        accessibilityLabel={props.label}
        accessibilityState={{ checked: props.value, disabled: !!props.disabled }}
        disabled={props.disabled}
        onPress={() => !props.disabled && props.onChange(!props.value)}
        style={{
          width: 32,
          height: 18.4,
          borderRadius: radius.full,
          padding: 2,
          alignItems: props.value ? "flex-end" : "flex-start",
          justifyContent: "center",
          opacity: props.disabled ? 0.5 : 1,
          backgroundColor: props.value ? palette().accent : palette().border,
        }}
      >
        <View
          style={{
            width: 14.4,
            height: 14.4,
            borderRadius: radius.full,
            backgroundColor: palette().background,
          }}
        />
      </View>
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
  const stacked = () => !!props.stacked;
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
        <If
          when={props.tone}
          else={
            <Text
              selectable
              style={{
                width: "100%",
                minWidth: 0,
                fontSize: 14,
                lineHeight: 20,
                fontWeight: "500",
                color: palette().text,
                textAlign: stacked() ? "left" : "right",
              }}
            >
              {props.value}
            </Text>
          }
        >
          <Badge label={props.value} tone={props.tone} />
        </If>
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
        gap: spacing.md,
        width: "100%",
      }}
    >
      {props.children}
    </View>
  );
}
export function Hint(props: { text: string; danger?: boolean }) {
  return (
    <Text
      selectable
      style={{
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
      selectable
      numberOfLines={props.lines}
      style={{
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

export async function confirm(ui: AppContext, title: string, action: () => void): Promise<void> {
  const route = ui.route();
  ui.confirmation()?.resolve(false);
  const accepted = await new Promise<boolean>((resolve) => ui.confirmation.set({ title, resolve }));
  if (accepted && ui.alive() && ui.route() === route) action();
}

export function Dialog(props: {
  title: string;
  description?: string;
  onClose: () => void;
  children?: JSX.Element;
  width?: number;
}) {
  const notificationLayer = useNotificationLayer();
  const content = (
    <View style={{ flex: 1, alignItems: "center", justifyContent: "center", padding: spacing.sm }}>
      <View
        onPress={props.onClose}
        style={{
          position: "absolute",
          top: 0,
          right: 0,
          bottom: 0,
          left: 0,
          backgroundColor: "#00000080",
        }}
      />
      <View
        accessibilityLabel={props.title}
        style={{
          width: "100%",
          maxWidth: props.width || 512,
          maxHeight: "90%",
          padding: spacing.xxl,
          gap: spacing.lg,
          backgroundColor: palette().panel,
          borderColor: palette().border,
          borderWidth: 1,
          borderRadius: radius.lg,
          boxShadow: "0px 10px 15px #0000001a",
        }}
      >
        <View style={{ flexDirection: "row", gap: spacing.md, alignItems: "center" }}>
          <Text
            style={{ flex: 1, fontSize: typography.lg, fontWeight: "600", color: palette().text }}
          >
            {props.title}
          </Text>
          <View style={{ width: 34 }} />
        </View>
        <If when={props.description}>
          <Hint text={props.description!} />
        </If>
        {props.children}
        <View style={{ position: "absolute", top: 14, right: 14 }}>
          <IconButton
            icon="x"
            label="Close dialog"
            id="dialog-close"
            onPress={props.onClose}
            size={28}
          />
        </View>
      </View>
      <NotificationViewport owner={notificationLayer} modal />
    </View>
  );
  return (
    <Modal visible transparent onRequestClose={props.onClose} animationType="fade">
      {content}
    </Modal>
  );
}

export function selectPanelLayout(
  trigger: { pageX: number; pageY: number; width: number; height: number },
  viewport: { width: number; height: number },
  insets: { top: number; right: number; bottom: number; left: number },
  keyboardHeight: number,
  count: number,
  labelled: boolean,
) {
  const left = insets.left + 8,
    right = viewport.width - insets.right - 8,
    top = insets.top + 8;
  const bottom = Math.max(top, viewport.height - Math.max(insets.bottom, keyboardHeight) - 8);
  const width = Math.max(1, Math.min(trigger.width, right - left));
  const height = Math.max(
    1,
    Math.min(Math.min(192, count * 32) + (labelled ? 28 : 0) + 10, bottom - top),
  );
  const below = trigger.pageY + trigger.height + 4;
  return {
    x: Math.max(left, Math.min(trigger.pageX, right - width)),
    y: Math.max(
      top,
      Math.min(below + height <= bottom ? below : trigger.pageY - height - 4, bottom - height),
    ),
    width,
    height,
  };
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
  const open = signal(false),
    frame = signal({ x: 0, y: 0, width: 180, height: 36, pageX: 0, pageY: 0 });
  const alive = useLifetime(),
    insets = useSafeAreaInsets();
  let trigger!: YNode;
  const show = async () => {
    const measured = await trigger.measure();
    if (!alive() || !measured || props.disabled) return;
    frame.set(measured);
    open.set(true);
  };
  const panel = computed(() =>
    selectPanelLayout(
      frame(),
      windowSize(),
      {
        top: Math.max(insets().top, safeArea().top),
        bottom: Math.max(insets().bottom, safeArea().bottom),
        left: Math.max(insets().left, safeArea().left),
        right: Math.max(insets().right, safeArea().right),
      },
      keyboard().height,
      props.options.length,
      !!props.label,
    ),
  );
  const Options = () => (
    <View>
      <Each of={props.options} key={(option) => option.value}>
        {(option) => (
          <View
            class="focus-visible:ring-2 focus-visible:ring-[#71717a]"
            testID={`select-option-${option().value}`}
            accessibilityRole="radio"
            accessibilityLabel={option().label}
            accessibilityState={{ checked: option().value === props.value }}
            onPress={() => {
              const value = option().value;
              open.set(false);
              if (value !== props.value) props.onChange(value);
            }}
            style={{
              height: 32,
              paddingHorizontal: spacing.sm,
              borderRadius: radius.sm,
              flexDirection: "row",
              alignItems: "center",
              gap: spacing.sm,
              backgroundColor: option().value === props.value ? palette().soft : "transparent",
            }}
          >
            <Text
              numberOfLines={1}
              style={{ flex: 1, color: palette().text, fontSize: typography.sm }}
            >
              {option().label}
            </Text>
            <If when={option().value === props.value}>
              <Icon name="check" size={16} color={palette().text} />
            </If>
          </View>
        )}
      </Each>
    </View>
  );
  const Popover = () => {
    const notificationLayer = useNotificationLayer();
    const content = (
      <View style={{ flex: 1 }}>
        <View
          testID="select-dismiss"
          onPress={() => open.set(false)}
          style={{
            position: "absolute",
            top: 0,
            bottom: 0,
            left: 0,
            right: 0,
            backgroundColor: "#00000001",
          }}
        />
        <View
          testID="select-popover"
          style={{
            position: "absolute",
            left: panel().x,
            top: panel().y,
            width: panel().width,
            height: panel().height,
            padding: 4,
            borderWidth: 1,
            borderColor: palette().border,
            borderRadius: radius.md,
            backgroundColor: palette().panel,
            boxShadow: "0px 4px 12px #0000001a",
          }}
        >
          <If when={props.label}>
            <View style={{ height: 28, paddingHorizontal: 8, justifyContent: "center" }}>
              <Text numberOfLines={1} style={{ color: palette().muted, fontSize: 12 }}>
                {props.label}
              </Text>
            </View>
          </If>
          <If
            when={props.options.length * 32 + (props.label ? 28 : 0) + 10 > panel().height}
            else={<Options />}
          >
            <Scroll bounces={false} style={{ flex: 1 }}>
              <Options />
            </Scroll>
          </If>
        </View>
        <NotificationViewport owner={notificationLayer} modal />
      </View>
    );
    return (
      <Modal transparent visible onRequestClose={() => open.set(false)} animationType="none">
        {content}
      </Modal>
    );
  };
  return (
    <View style={{ gap: spacing.xs }}>
      <If when={props.label}>
        <View style={{ flexDirection: "row", alignItems: "center", gap: spacing.sm }}>
          <Text
            style={{ flex: 1, minWidth: 0, fontSize: typography.sm, fontWeight: "500", color: palette().text }}
          >
            {props.label}
          </Text>
          {props.labelAccessory}
        </View>
      </If>
      <View
        testID={props.id}
        class="focus-visible:ring-2 focus-visible:ring-[#71717a]"
        ref={trigger}
        accessibilityRole="button"
        accessibilityLabel={props.label}
        accessibilityState={{ expanded: open(), disabled: !!props.disabled, busy: !!props.loading }}
        disabled={props.disabled}
        onPress={() => void show()}
        style={{
          height: control.height,
          paddingHorizontal: spacing.md,
          borderWidth: 1,
          borderColor: palette().border,
          borderRadius: radius.md,
          backgroundColor: palette().background,
          flexDirection: "row",
          alignItems: "center",
          gap: spacing.sm,
          opacity: props.disabled ? 0.5 : 1,
        }}
      >
        <Text numberOfLines={1} style={{ flex: 1, fontSize: typography.sm, color: palette().text }}>
          {props.options.find((option) => option.value === props.value)?.label || props.value}
        </Text>
        <View style={{ width: 16, height: 16, alignItems: "center", justifyContent: "center" }}>
          <If when={props.loading} else={<Icon name="chevron-down" size={16} color={palette().muted} />}>
            <Spinner size={16} />
          </If>
        </View>
      </View>
      <If when={open()}>
        <Popover />
      </If>
    </View>
  );
}

export function RadioGroup(props: {
  label: string;
  value: string;
  options: { value: string; label: string }[];
  onChange: (value: string) => void;
}) {
  return (
    <View
      accessibilityRole="radiogroup"
      accessibilityLabel={props.label}
      style={{ gap: spacing.md }}
    >
      <Each of={props.options} key={(option) => option.value}>
        {(option) => (
          <View
            class="focus-visible:ring-2 focus-visible:ring-[#71717a]"
            accessibilityRole="radio"
            accessibilityLabel={option().label}
            accessibilityState={{ checked: props.value === option().value }}
            onPress={() => props.onChange(option().value)}
            style={{ flexDirection: "row", alignItems: "center", gap: spacing.sm, minHeight: 24 }}
          >
            <View
              style={{
                width: 16,
                height: 16,
                borderRadius: radius.full,
                borderWidth: 1,
                borderColor: palette().accent,
                alignItems: "center",
                justifyContent: "center",
              }}
            >
              <If when={props.value === option().value}>
                <View
                  style={{
                    width: 8,
                    height: 8,
                    borderRadius: radius.full,
                    backgroundColor: palette().accent,
                  }}
                />
              </If>
            </View>
            <Text style={{ fontSize: typography.sm, color: palette().text }}>{option().label}</Text>
          </View>
        )}
      </Each>
    </View>
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
  const angle = createNativeValue(0);
  const animation = Animated.loop(
    Animated.timing(angle, { toValue: 360, duration: 900, easing: Easing.linear }),
    { resetBeforeIteration: true },
  );
  animation.start();
  cleanup(() => animation.stop());
  return (
    <View
      accessibilityRole="progressbar"
      accessibilityLabel="Loading"
      pointerEvents="none"
      style={{ width: props.size || 16, height: props.size || 16, transform: [{ rotate: angle }] }}
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
      accessibilityRole="alert"
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

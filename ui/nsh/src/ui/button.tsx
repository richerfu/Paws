import { keymap, platform, Text, View, type JSX } from "nsh";
import { Button, Link, Tooltip } from "nsh/base";
import { Icon, type IconName } from "../icons";
import { useApp } from "./context";
import { feedbackStyle, useGlyphTransition, usePressFeedback } from "./motion";
import { Spinner } from "./surface";
import {
  control,
  density,
  focusRing,
  palette,
  radius,
  spacing,
  surfaceFinish,
} from "./tokens";

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
  variant?:
    | "primary"
    | "stop"
    | "default"
    | "outline"
    | "secondary"
    | "ghost"
    | "link";
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
      focusVisibleStyle={focusRing()}
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
        paddingVertical: platform.desktop
          ? 2
          : props.small
            ? spacing.xxs
            : density.space(6),
        paddingHorizontal: platform.desktop
          ? props.small
            ? 8
            : 12
          : props.small
            ? 12
            : 16,
        borderRadius: radius.md,
        borderCurve: "continuous",
        flexDirection: "row",
        gap: platform.desktop ? 6 : 8,
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
                <Icon
                  name={props.icon!}
                  size={
                    platform.desktop
                      ? props.small
                        ? 12
                        : 14
                      : props.small
                        ? 14
                        : 16
                  }
                  color={foreground()}
                />
              </If>
            }
          >
            <Spinner
              size={
                platform.desktop
                  ? props.small
                    ? 12
                    : 14
                  : props.small
                    ? 14
                    : 16
              }
              color={foreground()}
            />
          </If>
        </View>
      </If>
      <Text
        style={{
          minWidth: 0,
          flexShrink: 1,
          lineHeight: density.line(platform.desktop ? 18 : 20),
          fontSize: platform.desktop
            ? density.font(props.small ? 12 : 13)
            : density.font(props.small ? 14 : 15),
          fontWeight: "500",
          color: foreground(),
        }}
      >
        {props.label}
      </Text>
    </Button>
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
  const press = usePressFeedback(disabled, 0.95);
  const glyph = useGlyphTransition(() => (props.loading ? "busy" : props.icon));
  const size = () =>
    props.size ?? (props.small ? control.small : control.icon);
  const iconSize = () =>
    platform.desktop ? (props.small ? 13 : 15) : props.small ? 16 : 18;
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
              borderRadius: radius.sm,
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
                  fontVariant: ["tabular-nums"],
                }}
              >
                {keymap.label(props.shortcut!)}
              </Text>
            </If>
          </Tooltip>
        ) : undefined
      }
      hitSlop={control.hitSlop(size())}
      focusVisibleStyle={focusRing()}
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
              size={iconSize()}
              color={props.iconColor || palette().text}
            />
          }
        >
          <Spinner size={iconSize()} />
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
      focusVisibleStyle={focusRing()}
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

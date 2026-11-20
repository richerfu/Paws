import {
  Animated,
  cleanup,
  createAnimatedValue as createNativeValue,
  Easing,
  effect,
  fontScale,
  platform,
  reduceMotion,
  Text,
  View,
  windowSize,
  type JSX,
  type ViewStyle,
} from "nsh";
import { Icon, type IconName } from "../icons";
import { App, useApp, useLayout } from "./context";
import { feedbackStyle, follow } from "./motion";
import {
  control,
  density,
  focusRing,
  palette,
  radius,
  spacing,
  statusColors,
  surfaceFinish,
  typography,
  type StatusTone,
} from "./tokens";

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
          fontSize: typography.section,
          lineHeight: density.line(platform.desktop ? 18 : 20),
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
        borderRadius: platform.desktop ? 7 : 8,
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
        color={
          props.color || (props.selected ? palette().accent : palette().text)
        }
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
      <If when={props.divider}>
        <View
          style={{
            pointerEvents: "none",
            position: "absolute",
            left: platform.desktop ? 14 : 16,
            right: platform.desktop ? 14 : 16,
            bottom: 0,
            height: 1,
            backgroundColor: palette().line,
          }}
        />
      </If>
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
    minHeight: control.row,
    paddingHorizontal: platform.desktop ? 14 : 16,
    paddingVertical: control.rowPadding,
    position: "relative" as const,
    flexDirection: "row" as const,
    alignItems: (props.alignTop ? "flex-start" : "center") as
      | "flex-start"
      | "center",
    gap: platform.desktop ? 10 : spacing.md,
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
        focusVisibleStyle={focusRing(0)}
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
    Math.max(
      0,
      Math.min(
        100,
        Math.round((Number.isFinite(props.ratio) ? props.ratio : 0) * 100),
      ),
    );
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

function splitRate(
  value: number | undefined,
  formatBytes: (n: number) => string,
) {
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
                  fontSize: typography.section,
                  lineHeight: props.subtitle
                    ? density.line(22)
                    : density.line(platform.desktop ? 18 : 20),
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
        minHeight: 26,
        borderRadius: 13,
        borderCurve: "continuous",
        paddingHorizontal: 10,
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
          fontWeight: "500",
          fontVariant: ["tabular-nums"],
          color: colors().text,
        }}
      >
        {props.label}
      </Text>
    </View>
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
            fontSize: typography.sm,
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
  return (
    <View
      style={{
        paddingHorizontal: spacing.lg,
        paddingVertical: spacing.md,
        gap: spacing.sm,
        borderTopWidth: 1,
        borderTopColor: palette().line,
        backgroundColor: palette().background,
      }}
    >
      {props.children}
    </View>
  );
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
        backgroundColor:
          isDanger() || isWarning()
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
              color: isDanger()
                ? palette().danger
                : isWarning()
                  ? palette().warning
                  : isInfo()
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

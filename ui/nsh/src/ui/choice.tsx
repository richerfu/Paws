import { platform, Text, View } from "nsh";
import {
  Button,
  Checkbox,
  Radio,
  RadioGroup as NativeRadioGroup,
  Switch,
} from "nsh/base";
import { Icon, type IconName } from "../icons";
import {
  feedbackStyle,
  follow,
  motion,
  useGlyphTransition,
  usePressFeedback,
} from "./motion";
import { Hint, Spinner } from "./surface";
import {
  control,
  density,
  focusRing,
  palette,
  radius,
  spacing,
  surfaceFinish,
  typography,
} from "./tokens";

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
      focusVisibleStyle={focusRing(props.insetRow ? 0 : 1)}
      accessibilityLabel={props.label}
      checked={props.value}
      disabled={props.disabled}
      onChange={(event) => props.onChange(event.checked)}
      style={{
        flexDirection: "row-reverse",
        alignItems: "center",
        gap: spacing.md,
        minHeight: platform.desktop ? 40 : control.row,
        paddingHorizontal: props.insetRow ? spacing.md : 0,
        paddingVertical: platform.desktop
          ? props.insetRow
            ? density.space(10)
            : 0
          : control.rowPadding,
        borderBottomWidth: props.divider ? 1 : 0,
        borderBottomColor: palette().line,
        opacity: props.disabled ? 0.45 : 1,
      }}
      trackStyle={{
        width: platform.desktop ? 28 : 42,
        height: platform.desktop ? 16 : 24,
        borderRadius: radius.full,
        padding: 1,
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
        width: platform.desktop ? 12 : 20,
        height: platform.desktop ? 12 : 20,
        borderRadius: radius.full,
        borderWidth: 1,
        ...(props.value ? surfaceFinish("knobOn") : surfaceFinish("knob")),
        transform: [
          {
            translateX: position.interpolate({
              inputRange: [0, 1],
              outputRange: [0, platform.desktop ? 12 : 18],
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
      focusVisibleStyle={focusRing(props.insetRow ? 0 : 1)}
      onChange={(event) => props.onChange(event.checked)}
      style={{
        flexDirection: "row",
        alignItems: "center",
        gap: spacing.md,
        minHeight: platform.desktop ? 40 : control.row,
        paddingHorizontal: props.insetRow ? spacing.md : 0,
        paddingVertical: platform.desktop
          ? props.insetRow
            ? density.space(8)
            : 0
          : control.rowPadding,
        borderBottomWidth: props.divider ? 1 : 0,
        borderBottomColor: palette().line,
        opacity: props.disabled ? 0.45 : 1,
      }}
      indicatorStyle={{
        width: control.box,
        height: control.box,
        borderRadius: platform.desktop ? 5 : 7,
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
            size={platform.desktop ? 12 : 16}
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

/** Shared dual-tier radio indicator (16pt pointer / 22pt touch) reused by RadioGroup & proxy selection rows. */
export function RadioMark(props: { selected: boolean }) {
  const scale = follow(() => (props.selected ? 1 : 0));
  const dot = () => (platform.desktop ? 6 : 8);
  return (
    <View
      style={{
        width: control.box,
        height: control.box,
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
        flexShrink: 0,
      }}
    >
      <View
        style={{
          width: dot(),
          height: dot(),
          borderRadius: radius.full,
          backgroundColor: palette().primaryText,
          opacity: scale,
          transform: [{ scale }],
        }}
      />
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
  return (
    <Radio
      focusVisibleStyle={focusRing(props.insetRow ? 0 : 1)}
      accessibilityLabel={props.label}
      checked={props.selected}
      onChange={props.onSelect}
      hoverStyle={{ backgroundColor: palette().hover }}
      activeStyle={{ backgroundColor: palette().soft }}
      style={{
        minHeight: platform.desktop ? 40 : control.row,
        flexDirection: "row",
        alignItems: "center",
        gap: spacing.md,
        paddingVertical: platform.desktop
          ? density.space(8)
          : control.rowPadding,
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
      <RadioMark selected={props.selected} />
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
      focusVisibleStyle={focusRing()}
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
            borderRadius: radius.md,
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
              else={
                <Icon name={props.icon} size={17} color={palette().text} />
              }
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

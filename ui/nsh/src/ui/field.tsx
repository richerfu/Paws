import {
  effect,
  fontScale,
  platform,
  signal,
  Text,
  View,
} from "nsh";
import {
  Editor,
  Input,
  type EditorRef,
  type EditorSearchResult,
  type InputProps,
} from "nsh/base";
import { Icon, type IconName } from "../icons";
import { IconButton } from "./button";
import { App } from "./context";
import { feedbackStyle } from "./motion";
import { Hint } from "./surface";
import {
  control,
  density,
  palette,
  radius,
  spacing,
  surfaceFinish,
  typography,
} from "./tokens";

export { Editor, type EditorRef, type EditorSearchResult };

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
  const flat = () => !!props.disabled || !!props.readonly;
  const showClear = () =>
    !!props.clearable &&
    !!props.value &&
    !props.password &&
    !props.disabled &&
    !props.readonly;
  // Gallery fieldSize: 32pt / pad 10 / font 14 pointer; 46pt / pad 14 / font 15 touch.
  const inset = () => (platform.desktop ? 10 : 14);
  const embedded = () => control.field - 6;
  return (
    <View style={{ gap: spacing.xs }}>
      <If when={props.label}>
        <Text
          style={{
            fontSize: density.font(13),
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
                : Math.max(
                    control.field,
                    20 * Math.max(1, fontScale()) +
                      (platform.desktop ? 12 : 16),
                  )),
            fontSize: typography.sm,
            lineHeight: density.line(20),
            color: palette().text,
            placeholderColor: palette().muted,
            paddingLeft: props.leadingIcon
              ? inset() + density.space(22)
              : inset(),
            paddingRight:
              props.password || showClear() ? embedded() + 4 : inset(),
            paddingVertical: props.multiline
              ? platform.desktop
                ? 7
                : 13
              : 0,
            opacity: props.disabled ? 0.5 : 1,
            backgroundColor: palette().field,
            borderWidth: 1,
            borderColor: props.error
              ? palette().danger
              : focused()
                ? palette().ring
                : flat()
                  ? palette().line
                  : palette().fieldBorder,
            borderRadius: radius.md,
            borderCurve: "continuous",
            ...(flat() ? {} : surfaceFinish("field")),
            ...feedbackStyle(),
          }}
        />
        <If when={props.leadingIcon}>
          <View
            style={{
              pointerEvents: "none",
              position: "absolute",
              left: inset(),
              top: 0,
              bottom: 0,
              justifyContent: "center",
            }}
          >
            <Icon
              name={props.leadingIcon!}
              size={platform.desktop ? 14 : 15}
              color={palette().muted}
            />
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
              size={embedded()}
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
              size={embedded()}
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

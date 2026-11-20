import {
  platform,
  Scroll,
  signal,
  Text,
  View,
  windowSize,
  type JSX,
  type ViewStyle,
} from "nsh";
import {
  Button,
  ContextMenu,
  Menu,
  MenuItem,
  MenuSeparator,
  Popover,
} from "nsh/base";
import { Icon, type IconName } from "../icons";
import { IconButton } from "./button";
import {
  App,
  RouteOwner,
  useApp,
  useLifetime,
  useOverlay,
} from "./context";
import { feedbackStyle, usePressFeedback } from "./motion";
import { Chevron, Spinner } from "./surface";
import {
  control,
  density,
  focusRing,
  palette,
  popupSurface,
  radius,
  spacing,
  surfaceFinish,
  typography,
  useWidePopover,
} from "./tokens";

export type ObjectCommand = {
  label: string;
  icon: IconName;
  id?: string;
  disabled?: boolean;
  disabledReason?: string;
  danger?: boolean;
  /** Render a menu divider immediately after this command (e.g., error banner command). */
  separatorAfter?: boolean;
  run: () => void;
};

function menuRowMetrics(inPopover: boolean) {
  if (platform.desktop) {
    return {
      minHeight: control.menuRow,
      paddingHorizontal: 8,
      borderRadius: control.menuRadius,
      gap: 8,
      iconSize: 14,
      fontSize: density.font(13),
      lineHeight: density.line(18),
    };
  }
  if (inPopover) {
    return {
      minHeight: 36,
      paddingHorizontal: 10,
      borderRadius: 7,
      gap: 8,
      iconSize: 16,
      fontSize: density.font(14),
      lineHeight: density.line(20),
    };
  }
  return {
    minHeight: control.menuRow,
    paddingHorizontal: spacing.md,
    borderRadius: control.menuRadius,
    gap: 10,
    iconSize: 18,
    fontSize: density.font(16),
    lineHeight: density.line(22),
  };
}

/** One command set for the visible menu and native right click / long press.
 * - Phone (< 768pt touch): bottom sheet via `Menu` (46pt rows).
 * - Pad / HarmonyOS 2-in-1 wide (>= 768pt non-desktop): trigger-anchored `Popover` (36pt rows, transparent dismiss).
 * - PC (`platform.desktop`): trigger/pointer-anchored `Menu` / `ContextMenu` (26pt rows). */
export function ObjectMenu(props: {
  id: string;
  title: string;
  subtitle?: string;
  triggerLabel?: string;
  triggerIcon?: IconName;
  triggerStyle?: ViewStyle;
  closeId?: string;
  commands: ObjectCommand[];
  row: (trigger: JSX.Element, state?: { contextOpen: () => boolean }) => JSX.Element;
}) {
  const ui = useApp(),
    alive = useLifetime(),
    route = RouteOwner() || ui.route();
  const open = signal(false),
    contextOpen = signal(false);
  useOverlay(() => open() || contextOpen());
  let pending: { source: "menu" | "context"; run: () => void } | undefined;
  const close = () => {
    open.set(false);
    contextOpen.set(false);
  };
  const closed = (source: "menu" | "context") => {
    if (pending?.source !== source) return;
    const command = pending.run;
    pending = undefined;
    if (alive() && ui.route() === route) command();
  };
  const Commands = (value: { source: "menu" | "context"; inPopover: boolean }) => {
    const metrics = () => menuRowMetrics(value.inPopover);
    const separator = () =>
      value.inPopover ? (
        <View
          style={{
            height: 1,
            marginVertical: density.space(4),
            backgroundColor: palette().line,
          }}
        />
      ) : (
        <MenuSeparator
          style={{
            height: 1,
            marginVertical: density.space(4),
            backgroundColor: palette().line,
          }}
        />
      );
    return (
      <View>
        <If when={!(platform.desktop && value.source === "context")}>
          <View
            style={{
              flexDirection: "row",
              alignItems: "center",
              paddingLeft: value.inPopover ? 10 : spacing.md,
              paddingRight: value.inPopover ? 2 : 0,
              paddingVertical: density.space(4),
              borderBottomWidth: 1,
              borderBottomColor: palette().line,
              marginBottom: density.space(2),
            }}
          >
            <View style={{ flex: 1, minWidth: 0, gap: density.space(2) }}>
              <Text
                role="header"
                accessibilityLabel={props.title}
                style={{
                  minWidth: 0,
                  numberOfLines: 2,
                  fontSize: density.font(value.inPopover ? 12 : 13),
                  lineHeight: density.line(value.inPopover ? 16 : 18),
                  fontWeight: "600",
                  color: value.inPopover ? palette().muted : palette().text,
                }}
              >
                {props.title}
              </Text>
              <If when={props.subtitle}>
                <Text
                  style={{
                    minWidth: 0,
                    numberOfLines: 1,
                    fontSize: density.font(12),
                    lineHeight: density.line(16),
                    color: palette().muted,
                  }}
                >
                  {props.subtitle}
                </Text>
              </If>
            </View>
            <IconButton
              id={props.closeId || `${props.id}-close`}
              icon="x"
              size={value.inPopover ? 28 : undefined}
              small={value.inPopover}
              tooltip={false}
              label={ui.t("关闭菜单", "Close menu")}
              onPress={close}
            />
          </View>
        </If>
        <Scroll
          bounces={false}
          keyboardShouldPersistTaps="handled"
          style={{
            maxHeight: Math.max(88, Math.min(480, windowSize().height * 0.65)),
          }}
        >
          <Each
            of={props.commands}
            key={(command) => command.id || command.label}
          >
            {(command, index) => {
              const rowStyle = (): ViewStyle => ({
                minHeight: metrics().minHeight,
                paddingHorizontal: metrics().paddingHorizontal,
                paddingVertical: 0,
                borderRadius: metrics().borderRadius,
                borderCurve: "continuous",
                flexDirection: "row",
                alignItems: "center",
                gap: metrics().gap,
                opacity: command().disabled ? 0.45 : 1,
                ...feedbackStyle(),
              });
              const rowBody = () => (
                <>
                  <Icon
                    name={command().icon}
                    size={metrics().iconSize}
                    color={
                      command().danger ? palette().danger : palette().muted
                    }
                  />
                  <Text
                    style={{
                      flex: 1,
                      minWidth: 0,
                      fontSize: metrics().fontSize,
                      lineHeight: metrics().lineHeight,
                      fontWeight: "400",
                      color: command().danger
                        ? palette().danger
                        : palette().text,
                    }}
                  >
                    {command().label}
                  </Text>
                  <If when={command().disabled && !!command().disabledReason}>
                    <Text
                      style={{
                        fontSize: density.font(12),
                        lineHeight: density.line(16),
                        color: palette().muted,
                      }}
                    >
                      {command().disabledReason}
                    </Text>
                  </If>
                </>
              );
              const selectCommand = () => {
                if (command().disabled) return;
                pending = { source: value.source, run: command().run };
                close();
              };
              return (
                <>
                  <If
                    when={
                      !!command().danger &&
                      index() > 0 &&
                      !props.commands[index() - 1]?.danger &&
                      !props.commands[index() - 1]?.separatorAfter
                    }
                  >
                    {separator()}
                  </If>
                  <If
                    when={value.inPopover}
                    else={
                      <MenuItem
                        testID={command().id}
                        disabled={command().disabled}
                        accessibilityLabel={command().label}
                        onSelect={selectCommand}
                        focusVisibleStyle={focusRing(0)}
                        hoverStyle={{ backgroundColor: palette().hover }}
                        activeStyle={{ backgroundColor: palette().soft }}
                        style={rowStyle()}
                      >
                        {rowBody()}
                      </MenuItem>
                    }
                  >
                    <Button
                      testID={command().id}
                      role="menuitem"
                      disabled={command().disabled}
                      accessibilityLabel={command().label}
                      onPress={selectCommand}
                      focusVisibleStyle={focusRing(0)}
                      hoverStyle={{ backgroundColor: palette().hover }}
                      activeStyle={{ backgroundColor: palette().soft }}
                      style={rowStyle()}
                    >
                      {rowBody()}
                    </Button>
                  </If>
                  <If when={!!command().separatorAfter}>{separator()}</If>
                </>
              );
            }}
          </Each>
        </Scroll>
      </View>
    );
  };
  const triggerView = () => (
    <View
      testID={props.id}
      role="button"
      accessibilityLabel={
        props.triggerLabel ||
        ui.t(`${props.title}的操作`, `Actions for ${props.title}`)
      }
      hitSlop={control.hitSlop(control.icon)}
      accessibilityState={{ expanded: open() }}
      focusVisibleStyle={focusRing()}
      hoverStyle={{ backgroundColor: palette().soft }}
      activeStyle={{ backgroundColor: palette().soft }}
      style={{
        width: control.icon,
        height: control.icon,
        flexShrink: 0,
        borderRadius: radius.md,
        borderCurve: "continuous",
        alignItems: "center",
        justifyContent: "center",
        backgroundColor: open() ? palette().soft : "transparent",
        ...props.triggerStyle,
      }}
    >
      <Icon
        name={props.triggerIcon || "ellipsis"}
        size={platform.desktop ? 15 : 18}
        color={palette().muted}
      />
    </View>
  );
  const trigger = (
    <If
      when={useWidePopover()}
      else={
        <Menu
          open={open()}
          onOpenChange={(event) => open.set(event.open)}
          onClose={() => closed("menu")}
          popupStyle={popupSurface()}
          align="end"
          offset={4}
          margin={8}
          popupStartingStyle={{
            opacity: 0,
            transform: [{ scale: 0.96 }, { translateY: -4 }],
          }}
          popupEndingStyle={{ opacity: 0 }}
          backdropStyle={{ backgroundColor: palette().scrim }}
          trigger={triggerView()}
        >
          <Commands source="menu" inPopover={false} />
        </Menu>
      }
    >
      <Popover
        open={open()}
        onOpenChange={(event) => open.set(event.open)}
        onClose={() => closed("menu")}
        popupStyle={popupSurface("anchored", 280)}
        placement="bottom"
        align="end"
        offset={4}
        margin={8}
        popupStartingStyle={{
          opacity: 0,
          transform: [{ scale: 0.96 }, { translateY: -4 }],
        }}
        popupEndingStyle={{ opacity: 0 }}
        trigger={triggerView()}
      >
        <Commands source="menu" inPopover={true} />
      </Popover>
    </If>
  );
  return (
    <ContextMenu
      open={contextOpen()}
      onOpenChange={(event) => {
        if (event.open && useWidePopover()) {
          open.set(true);
          return;
        }
        contextOpen.set(event.open);
      }}
      onClose={() => closed("context")}
      popupStyle={popupSurface()}
      backdropStyle={{ backgroundColor: palette().scrim }}
      trigger={props.row(trigger, { contextOpen })}
    >
      <Commands source="context" inPopover={false} />
    </ContextMenu>
  );
}

/** Compact toolbar filter picker, strictly level with small Segmented (24pt pointer / 32pt touch). */
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
  const triggerView = () => (
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
      focusVisibleStyle={focusRing()}
      hoverStyle={disabled() ? undefined : surfaceFinish("hover")}
      activeStyle={disabled() ? undefined : surfaceFinish("pressed")}
      style={{
        minHeight: control.compactHeight,
        paddingLeft: platform.desktop ? 9 : 11,
        paddingRight: platform.desktop ? 7 : 9,
        borderRadius: platform.desktop ? 6 : 9,
        borderCurve: "continuous",
        borderWidth: 1,
        flexDirection: "row",
        alignItems: "center",
        gap: 5,
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
      <Chevron expanded={open()} size={platform.desktop ? 13 : 14} />
    </View>
  );
  const Options = (mode: { inPopover: boolean }) => {
    const metrics = () => menuRowMetrics(mode.inPopover);
    return (
      <View style={{ minWidth: 180 }}>
        <Each of={props.options} key={(option) => option.value}>
          {(option) => {
            const selected = () => option().value === props.value;
            const rowStyle = (): ViewStyle => ({
              minHeight: metrics().minHeight,
              paddingHorizontal: metrics().paddingHorizontal,
              paddingVertical: 0,
              borderRadius: metrics().borderRadius,
              borderCurve: "continuous",
              flexDirection: "row",
              alignItems: "center",
              gap: spacing.sm,
              backgroundColor: selected() ? palette().soft : "transparent",
              ...feedbackStyle(),
            });
            const body = () => (
              <>
                <Text
                  style={{
                    flex: 1,
                    minWidth: 0,
                    fontSize: metrics().fontSize,
                    lineHeight: metrics().lineHeight,
                    fontWeight: selected() ? "600" : "400",
                    color: palette().text,
                  }}
                >
                  {option().label}
                </Text>
                <View style={{ width: 16, opacity: selected() ? 1 : 0 }}>
                  <Icon
                    name="check"
                    size={platform.desktop ? 14 : 16}
                    color={ui ? palette().accent : palette().text}
                  />
                </View>
              </>
            );
            const choose = () => {
              if (props.disabled) return;
              if (option().value !== props.value)
                props.onChange(option().value);
              if (mode.inPopover) open.set(false);
            };
            return (
              <If
                when={mode.inPopover}
                else={
                  <MenuItem
                    testID={
                      props.id
                        ? `${props.id}-option-${option().value}`
                        : undefined
                    }
                    role="menuitemradio"
                    disabled={props.disabled}
                    checked={selected()}
                    accessibilityLabel={option().label}
                    onSelect={choose}
                    focusVisibleStyle={focusRing(0)}
                    hoverStyle={{ backgroundColor: palette().soft }}
                    activeStyle={{ backgroundColor: palette().soft }}
                    style={rowStyle()}
                  >
                    {body()}
                  </MenuItem>
                }
              >
                <Button
                  testID={
                    props.id
                      ? `${props.id}-option-${option().value}`
                      : undefined
                  }
                  role="menuitemradio"
                  disabled={props.disabled}
                  selected={selected()}
                  accessibilityLabel={option().label}
                  onPress={choose}
                  focusVisibleStyle={focusRing(0)}
                  hoverStyle={{ backgroundColor: palette().soft }}
                  activeStyle={{ backgroundColor: palette().soft }}
                  style={rowStyle()}
                >
                  {body()}
                </Button>
              </If>
            );
          }}
        </Each>
      </View>
    );
  };
  return (
    <If
      when={useWidePopover()}
      else={
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
          trigger={triggerView()}
        >
          <Options inPopover={false} />
        </Menu>
      }
    >
      <Popover
        open={open()}
        onOpenChange={(event) => {
          if (!event.open || !disabled()) open.set(event.open);
        }}
        placement="bottom"
        align="end"
        offset={4}
        margin={8}
        popupStyle={popupSurface("anchored", 240)}
        popupStartingStyle={{
          opacity: 0,
          transform: [{ scale: 0.96 }, { translateY: -4 }],
        }}
        popupEndingStyle={{ opacity: 0 }}
        trigger={triggerView()}
      >
        <Options inPopover={true} />
      </Popover>
    </If>
  );
}

/** Form dropdown selector: 32pt on pointer (level with Field), 44pt on touch. */
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
  const triggerView = () => (
    <View
      testID={props.id}
      role="button"
      accessibilityLabel={
        props.label ||
        ui?.t("选择选项", "Choose an option") ||
        "Choose an option"
      }
      accessibilityValue={{ text: selectedLabel() }}
      hitSlop={control.hitSlop(control.select)}
      accessibilityState={{
        expanded: open(),
        disabled: disabled(),
        busy: !!props.loading,
      }}
      disabled={disabled()}
      focusVisibleStyle={focusRing()}
      hoverStyle={disabled() ? undefined : { backgroundColor: palette().soft }}
      activeStyle={disabled() ? undefined : { backgroundColor: palette().soft }}
      style={{
        minHeight: control.select,
        minWidth: 0,
        paddingHorizontal: platform.desktop ? 10 : 14,
        paddingVertical: platform.desktop ? 0 : spacing.sm,
        borderWidth: 1,
        borderRadius: radius.md,
        borderCurve: "continuous",
        borderColor: open() ? palette().ring : palette().fieldBorder,
        backgroundColor: open() ? palette().soft : palette().field,
        flexDirection: "row",
        alignItems: "center",
        gap: spacing.sm,
        opacity: disabled() ? 0.45 : 1,
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
  );
  const SelectContent = (mode: { inPopover: boolean }) => {
    const metrics = () => menuRowMetrics(mode.inPopover);
    return (
      <View testID="select-popover" style={{ minWidth: 0 }}>
        <View
          style={{
            flexDirection: "row",
            alignItems: "center",
            paddingLeft: mode.inPopover ? 10 : spacing.md,
            paddingRight: mode.inPopover ? 2 : 0,
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
            size={mode.inPopover ? 28 : undefined}
            small={mode.inPopover}
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
            {(option) => {
              const selected = () => option().value === props.value;
              const rowStyle = (): ViewStyle => ({
                minHeight: metrics().minHeight,
                paddingHorizontal: metrics().paddingHorizontal,
                paddingVertical: 0,
                borderRadius: metrics().borderRadius,
                borderCurve: "continuous",
                flexDirection: "row",
                alignItems: "center",
                gap: spacing.sm,
                backgroundColor: selected() ? palette().soft : "transparent",
                ...feedbackStyle(),
              });
              const body = () => (
                <>
                  <Text
                    style={{
                      flex: 1,
                      minWidth: 0,
                      fontSize: metrics().fontSize,
                      lineHeight: metrics().lineHeight,
                      fontWeight: selected() ? "600" : "400",
                      color: palette().text,
                    }}
                  >
                    {option().label}
                  </Text>
                  <View style={{ width: 16, opacity: selected() ? 1 : 0 }}>
                    <Icon
                      name="check"
                      size={platform.desktop ? 14 : 16}
                      color={ui ? palette().accent : palette().text}
                    />
                  </View>
                </>
              );
              const choose = () => {
                if (props.disabled) return;
                if (option().value !== props.value)
                  props.onChange(option().value);
                if (mode.inPopover) open.set(false);
              };
              return (
                <If
                  when={mode.inPopover}
                  else={
                    <MenuItem
                      testID={`select-option-${option().value}`}
                      role="menuitemradio"
                      disabled={props.disabled}
                      checked={selected()}
                      accessibilityLabel={option().label}
                      onSelect={choose}
                      focusVisibleStyle={focusRing(0)}
                      hoverStyle={{ backgroundColor: palette().soft }}
                      activeStyle={{ backgroundColor: palette().soft }}
                      style={rowStyle()}
                    >
                      {body()}
                    </MenuItem>
                  }
                >
                  <Button
                    testID={`select-option-${option().value}`}
                    role="menuitemradio"
                    disabled={props.disabled}
                    selected={selected()}
                    accessibilityLabel={option().label}
                    onPress={choose}
                    focusVisibleStyle={focusRing(0)}
                    hoverStyle={{ backgroundColor: palette().soft }}
                    activeStyle={{ backgroundColor: palette().soft }}
                    style={rowStyle()}
                  >
                    {body()}
                  </Button>
                </If>
              );
            }}
          </Each>
        </Scroll>
      </View>
    );
  };
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
              fontSize: density.font(13),
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
      <If
        when={useWidePopover()}
        else={
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
            trigger={triggerView()}
          >
            <SelectContent inPopover={false} />
          </Menu>
        }
      >
        <Popover
          open={open()}
          onOpenChange={(event) => {
            if (!event.open || !disabled()) open.set(event.open);
          }}
          placement="bottom"
          align="start"
          offset={4}
          margin={8}
          popupStyle={popupSurface("anchored", 360)}
          popupStartingStyle={{
            opacity: 0,
            transform: [{ scale: 0.96 }, { translateY: -4 }],
          }}
          popupEndingStyle={{ opacity: 0 }}
          trigger={triggerView()}
        >
          <SelectContent inPopover={true} />
        </Popover>
      </If>
    </View>
  );
}

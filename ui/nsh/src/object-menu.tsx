import { density } from "./theme";
import {
  platform,
  Scroll,
  signal,
  Text,
  View,
  windowSize,
  type JSX,
} from "nsh";
import { ContextMenu, Menu, MenuItem, MenuSeparator } from "nsh/base";
import {
  control,
  Icon,
  IconButton,
  palette,
  radius,
  spacing,
  useApp,
  useLifetime,
  useOverlay,
  RouteOwner,
} from "./components";
import type { IconName } from "./icons";
import { feedbackStyle } from "./motion";
import { focusRing, popupSurface } from "./theme";

export type ObjectCommand = {
  label: string;
  icon: IconName;
  id?: string;
  disabled?: boolean;
  disabledReason?: string;
  danger?: boolean;
  run: () => void;
};

/** One command set for the visible menu and native right click / long press. */
export function ObjectMenu(props: {
  id: string;
  title: string;
  subtitle?: string;
  triggerLabel?: string;
  commands: ObjectCommand[];
  row: (trigger: JSX.Element) => JSX.Element;
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
  const Commands = (value: { source: "menu" | "context" }) => (
    <View>
      <If when={!(platform.desktop && value.source === "context")}>
        <View
          style={{
            flexDirection: "row",
            alignItems: "center",
            paddingLeft: spacing.md,
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
                fontSize: density.font(13),
                lineHeight: density.line(18),
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
                  numberOfLines: 1,
                  fontSize: density.font(11),
                  lineHeight: density.line(15),
                  color: palette().muted,
                }}
              >
                {props.subtitle}
              </Text>
            </If>
          </View>
          <IconButton
            id={`${props.id}-close`}
            icon="x"
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
          {(command, index) => (
            <>
              <If
                when={
                  !!command().danger &&
                  index() > 0 &&
                  !props.commands[index() - 1]?.danger
                }
              >
                <MenuSeparator
                  style={{
                    height: 1,
                    marginVertical: density.space(4),
                    backgroundColor: palette().line,
                  }}
                />
              </If>
              <MenuItem
                testID={command().id}
                disabled={command().disabled}
                accessibilityLabel={command().label}
                onSelect={() => {
                  pending = { source: value.source, run: command().run };
                  close();
                }}
                focusStyle={focusRing()}
                hoverStyle={{ backgroundColor: palette().hover }}
                activeStyle={{ backgroundColor: palette().soft }}
                style={{
                  minHeight: 42,
                  paddingHorizontal: spacing.md,
                  paddingVertical: spacing.xs,
                  borderRadius: 8,
                  borderCurve: "continuous",
                  flexDirection: "row",
                  alignItems: "center",
                  gap: 10,
                  opacity: command().disabled ? 0.45 : 1,
                  ...feedbackStyle(),
                }}
              >
                <Icon
                  name={command().icon}
                  size={16}
                  color={command().danger ? palette().danger : palette().muted}
                />
                <Text
                  style={{
                    flex: 1,
                    minWidth: 0,
                    fontSize: density.font(14),
                    lineHeight: density.line(20),
                    fontWeight: "500",
                    color: command().danger ? palette().danger : palette().text,
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
              </MenuItem>
            </>
          )}
        </Each>
      </Scroll>
    </View>
  );
  const trigger = (
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
      trigger={
        <View
          testID={props.id}
          role="button"
          accessibilityLabel={
            props.triggerLabel ||
            ui.t(`${props.title}的操作`, `Actions for ${props.title}`)
          }
          hitSlop={control.hitSlop(control.icon)}
          accessibilityState={{ expanded: open() }}
          focusStyle={focusRing()}
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
          }}
        >
          <Icon name="ellipsis" size={18} color={palette().muted} />
        </View>
      }
    >
      <Commands source="menu" />
    </Menu>
  );
  return (
    <ContextMenu
      open={contextOpen()}
      onOpenChange={(event) => contextOpen.set(event.open)}
      onClose={() => closed("context")}
      popupStyle={popupSurface()}
      backdropStyle={{ backgroundColor: palette().scrim }}
      trigger={props.row(trigger)}
    >
      <Commands source="context" />
    </ContextMenu>
  );
}

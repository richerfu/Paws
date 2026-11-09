import { Scroll, signal, Text, View, windowSize, type JSX } from "nsh";
import { ContextMenu, Menu, MenuItem } from "nsh/base";
import { control, Icon, IconButton, palette, radius, spacing, useApp, useLifetime } from "./components";
import type { IconName } from "./icons";
import { feedbackStyle } from "./motion";
import { focusRing, popupSurface } from "./theme";

export type ObjectCommand = {
  label: string;
  icon: IconName;
  id?: string;
  disabled?: boolean;
  danger?: boolean;
  run: () => void;
};

/** One command set for the visible menu and native right click / long press. */
export function ObjectMenu(props: {
  id: string;
  title: string;
  commands: ObjectCommand[];
  row: (trigger: JSX.Element) => JSX.Element;
}) {
  const ui = useApp(), alive = useLifetime(), route = ui.route();
  const open = signal(false), contextOpen = signal(false);
  let pending: { source: "menu" | "context"; run: () => void } | undefined;
  const close = () => { open.set(false); contextOpen.set(false); };
  const closed = (source: "menu" | "context") => {
    if (pending?.source !== source) return;
    const command = pending.run;
    pending = undefined;
    if (alive() && ui.route() === route) command();
  };
  const Commands = (value: { source: "menu" | "context" }) => <View>
    <View style={{ flexDirection: "row", alignItems: "center", paddingLeft: spacing.md }}>
      <Text role="header" accessibilityLabel={props.title} style={{ flex: 1, minWidth: 0, numberOfLines: 2, fontSize: 13, lineHeight: 20,
        color: palette().muted }}>{props.title}</Text>
      <IconButton id={`${props.id}-close`} icon="x" label={ui.t("关闭菜单", "Close menu")} onPress={close} />
    </View>
    <Scroll bounces={false} style={{ maxHeight: Math.max(88, Math.min(480, windowSize().height * 0.65)) }}>
      <Each of={props.commands} key={(command) => command.id || command.label}>
        {(command) => <MenuItem testID={command().id} disabled={command().disabled}
          accessibilityLabel={command().label} onSelect={() => {
            pending = { source: value.source, run: command().run }; close();
          }} focusStyle={focusRing()} hoverStyle={{ backgroundColor: palette().soft }}
          activeStyle={{ backgroundColor: palette().soft }}
          style={{ minHeight: control.row, paddingHorizontal: spacing.md, paddingVertical: spacing.sm,
            borderRadius: radius.sm, flexDirection: "row", alignItems: "center", gap: spacing.md,
            opacity: command().disabled ? 0.4 : 1, ...feedbackStyle() }}>
          <Icon name={command().icon} size={18} color={command().danger ? palette().danger : palette().muted} />
          <Text style={{ flex: 1, minWidth: 0, fontSize: 14, lineHeight: 20,
            color: command().danger ? palette().danger : palette().text }}>{command().label}</Text>
        </MenuItem>}
      </Each>
    </Scroll>
  </View>;
  const trigger = <Menu open={open()} onOpenChange={(event) => open.set(event.open)}
    onClose={() => closed("menu")} popupStyle={popupSurface()} align="end" offset={4} margin={8}
    popupStartingStyle={{ opacity: 0, transform: [{ translateY: -4 }] }} popupEndingStyle={{ opacity: 0 }}
    backdropStyle={{ backgroundColor: palette().scrim }}
    trigger={<View testID={props.id} role="button" accessibilityLabel={ui.t(`${props.title}的操作`, `Actions for ${props.title}`)}
      hitSlop={control.hitSlop(control.icon)}
      accessibilityState={{ expanded: open() }} focusStyle={focusRing()}
      hoverStyle={{ backgroundColor: palette().soft }} activeStyle={{ opacity: 0.8 }}
      style={{ width: control.icon, height: control.icon, flexShrink: 0, borderRadius: radius.md,
        alignItems: "center", justifyContent: "center", backgroundColor: open() ? palette().soft : "transparent" }}>
      <Icon name="ellipsis-vertical" size={control.compact ? 16 : 18} color={palette().muted} />
    </View>}><Commands source="menu" /></Menu>;
  return <ContextMenu open={contextOpen()} onOpenChange={(event) => contextOpen.set(event.open)}
    onClose={() => closed("context")} popupStyle={popupSurface()}
    backdropStyle={{ backgroundColor: palette().scrim }} trigger={props.row(trigger)}>
    <Commands source="context" />
  </ContextMenu>;
}

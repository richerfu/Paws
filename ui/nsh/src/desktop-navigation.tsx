import { signal, Text, View, Scroll, windowSize, keymap, platform } from "nsh";
import { Button, Tooltip } from "nsh/base";
import { Icon, type IconName } from "./icons";
import type { Page } from "./components";
import { palette, spacing, radius, control, density, focusRing } from "./theme";
import { feedbackStyle } from "./motion";

type Destination = { page: Page; zh: string; en: string; icon: IconName };
export const desktopPrimary: Destination[] = [
  { page: "dashboard", zh: "概览", en: "Overview", icon: "house" },
  { page: "profiles", zh: "订阅", en: "Subscriptions", icon: "rss" },
  { page: "proxies", zh: "代理节点", en: "Proxy nodes", icon: "git-branch" },
  { page: "traffic", zh: "流量", en: "Traffic", icon: "activity" },
];
const sections: { title: [string, string]; items: Destination[] }[] = [
  {
    title: ["网络", "Network"],
    items: [
      {
        page: "connections",
        zh: "连接与请求",
        en: "Connections",
        icon: "unplug",
      },
      {
        page: "resources",
        zh: "资源与规则",
        en: "Resources & rules",
        icon: "database",
      },
      { page: "logs", zh: "日志", en: "Logs", icon: "scroll-text" },
    ],
  },
  {
    title: ["工具", "Tools"],
    items: [
      {
        page: "converter",
        zh: "订阅转换",
        en: "Converter",
        icon: "refresh-cw",
      },
      {
        page: "backups",
        zh: "备份与恢复",
        en: "Backup & restore",
        icon: "archive",
      },
      {
        page: "diagnostics",
        zh: "诊断",
        en: "Diagnostics",
        icon: "shield-check",
      },
    ],
  },
  {
    title: ["偏好设置", "Preferences"],
    items: [
      {
        page: "settings",
        zh: "网络设置",
        en: "Network settings",
        icon: "settings",
      },
      { page: "appearance", zh: "外观", en: "Appearance", icon: "palette" },
    ],
  },
];
const about: Destination = {
  page: "about",
  zh: "关于 Paws",
  en: "About Paws",
  icon: "info",
};
export const desktopRootPages: Page[] = [
  ...desktopPrimary.map((item) => item.page),
  ...sections.flatMap((section) => section.items.map((item) => item.page)),
  "requests",
  "about",
];

export function DesktopNavigation(props: {
  page: Page;
  version: string;
  navigate: (page: Page) => void;
  t: (zh: string, en: string) => string;
}) {
  const preference = signal<boolean | undefined>();
  const expanded = () => preference() ?? windowSize().width >= 1100;
  const selected = (page: Page) => {
    const current =
      props.page === "requests"
        ? "connections"
        : props.page === "editor"
          ? "profiles"
          : props.page === "privacy"
            ? "about"
            : props.page === "advanced-diagnostics"
              ? "diagnostics"
              : props.page;
    return current === page;
  };
  const Item = (item: {
    destination: Destination;
    shortcut?: number;
    footer?: boolean;
  }) => (
    <Button
      tooltip={
        !expanded() ? (
          <Tooltip
            style={{
              maxWidth: 260,
              paddingHorizontal: spacing.md,
              paddingVertical: spacing.sm,
              borderRadius: radius.md,
              borderCurve: "continuous",
              backgroundColor: palette().popover,
              borderWidth: 1,
              borderColor: palette().border,
              flexDirection: "row",
              alignItems: "center",
              gap: spacing.sm,
            }}
          >
            <Text style={{ fontSize: density.font(13), color: palette().text }}>
              {props.t(item.destination.zh, item.destination.en)}
            </Text>
            <If when={platform.desktop && !!item.shortcut}>
              <Text
                style={{
                  fontSize: density.font(11),
                  color: palette().muted,
                }}
              >
                {keymap.label(`mod+${item.shortcut}`)}
              </Text>
            </If>
          </Tooltip>
        ) : undefined
      }
      testID={
        item.shortcut
          ? `nav-${item.destination.page}`
          : `desktop-nav-${item.destination.page}`
      }
      accessibilityLabel={props.t(item.destination.zh, item.destination.en)}
      selected={selected(item.destination.page)}
      onPress={() => props.navigate(item.destination.page)}
      focusStyle={focusRing()}
      hoverStyle={{ backgroundColor: palette().soft }}
      activeStyle={{ backgroundColor: palette().accentFill }}
      style={{
        minHeight: control.row,
        paddingHorizontal: expanded() ? spacing.md : 0,
        flexDirection: "row",
        alignItems: "center",
        justifyContent: expanded() ? "flex-start" : "center",
        gap: spacing.md,
        borderRadius: radius.md,
        borderCurve: "continuous",
        backgroundColor: selected(item.destination.page)
          ? palette().raised
          : "transparent",
        ...feedbackStyle(),
      }}
    >
      <Icon
        name={item.destination.icon}
        size={18}
        color={
          selected(item.destination.page) ? palette().accent : palette().muted
        }
      />
      <If when={expanded()}>
        <View style={{ flex: 1, minWidth: 0 }}>
          <Text
            style={{
              color: palette().text,
              fontSize: density.font(14),
              lineHeight: density.line(20),
              numberOfLines: 1,
              fontWeight: selected(item.destination.page) ? "600" : "500",
            }}
          >
            {props.t(item.destination.zh, item.destination.en)}
          </Text>
          <If when={item.footer && props.version}>
            <Text
              style={{
                color: palette().muted,
                fontSize: density.font(11),
                lineHeight: density.line(16),
              }}
            >
              {props.version}
            </Text>
          </If>
        </View>
        <If when={platform.desktop && !!item.shortcut}>
          <Text style={{ color: palette().muted, fontSize: density.font(11) }}>
            {keymap.label(`mod+${item.shortcut}`)}
          </Text>
        </If>
      </If>
    </Button>
  );
  return (
    <View
      testID="desktop-sidebar"
      style={{
        width: expanded() ? 232 : 64,
        backgroundColor: palette().sidebar,
        borderRightWidth: 1,
        borderRightColor: palette().border,
      }}
    >
      <Button
        testID="desktop-toggle-sidebar"
        tooltip={
          !expanded() ? (
            <Tooltip
              style={{
                maxWidth: 240,
                paddingHorizontal: spacing.md,
                paddingVertical: spacing.sm,
                borderRadius: radius.md,
                borderCurve: "continuous",
                backgroundColor: palette().popover,
                borderWidth: 1,
                borderColor: palette().border,
              }}
            >
              <Text
                style={{ fontSize: density.font(13), color: palette().text }}
              >
                {props.t("展开导航栏", "Expand sidebar")}
              </Text>
            </Tooltip>
          ) : undefined
        }
        accessibilityLabel={
          expanded()
            ? props.t("收起导航栏", "Collapse sidebar")
            : props.t("展开导航栏", "Expand sidebar")
        }
        onPress={() => preference.set(!expanded())}
        focusStyle={focusRing()}
        hoverStyle={{ backgroundColor: palette().soft }}
        activeStyle={{ backgroundColor: palette().soft }}
        style={{
          minHeight: 64,
          paddingHorizontal: expanded() ? 24 : 20,
          flexDirection: "row",
          alignItems: "center",
          gap: spacing.md,
          borderBottomWidth: 1,
          borderBottomColor: palette().border,
        }}
      >
        <Icon name="paw-print" size={24} color={palette().accent} />
        <If when={expanded()}>
          <Text
            style={{
              flex: 1,
              color: palette().text,
              fontSize: density.font(18),
              fontWeight: "700",
            }}
          >
            Paws
          </Text>
          <Icon name="arrow-left" size={16} color={palette().muted} />
        </If>
      </Button>
      <Scroll
        bounces={false}
        style={{ flex: 1 }}
        contentStyle={{ padding: spacing.sm, gap: spacing.lg }}
      >
        <View style={{ gap: spacing.xxs }}>
          <Each of={desktopPrimary}>
            {(destination, index) => (
              <Item destination={destination()} shortcut={index() + 1} />
            )}
          </Each>
        </View>
        <Each of={sections}>
          {(section) => (
            <View style={{ gap: spacing.xxs }}>
              <If
                when={expanded()}
                else={
                  <View
                    style={{
                      height: 1,
                      marginHorizontal: spacing.md,
                      backgroundColor: palette().line,
                    }}
                  />
                }
              >
                <Text
                  role="header"
                  style={{
                    paddingLeft: spacing.md + 18 + spacing.md,
                    paddingBottom: spacing.xxs,
                    fontSize: density.font(11),
                    lineHeight: density.line(16),
                    color: palette().muted,
                  }}
                >
                  {props.t(...section().title)}
                </Text>
              </If>
              <Each of={section().items}>
                {(destination) => <Item destination={destination()} />}
              </Each>
            </View>
          )}
        </Each>
      </Scroll>
      <View
        style={{
          padding: spacing.sm,
          borderTopWidth: 1,
          borderTopColor: palette().border,
        }}
      >
        <Item destination={about} footer />
      </View>
    </View>
  );
}

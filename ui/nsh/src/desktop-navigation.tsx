import { signal, Text, View, Scroll, windowSize, keymap, platform, fontScale } from "nsh";
import { Button, Tooltip } from "nsh/base";
import { Icon, type IconName } from "./icons";
import type { Page } from "./components";
import { palette, spacing, radius, control, density, focusRing, surfaceFinish } from "./theme";
import { feedbackStyle } from "./motion";

type Destination = { page: Page; zh: string; en: string; icon: IconName; keybinding?: string };
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
        en: "Connections & requests",
        icon: "list-filter",
      },
      {
        page: "resources",
        zh: "资源与规则",
        en: "Resources & rules",
        icon: "layers",
      },
      { page: "logs", zh: "日志", en: "Logs", icon: "file-text" },
    ],
  },
  {
    title: ["工具与偏好", "Tools & preferences"],
    items: [
      {
        page: "diagnostics",
        zh: "诊断",
        en: "Diagnostics",
        icon: "shield-check",
      },
      {
        page: "backups",
        zh: "备份与恢复",
        en: "Backup & restore",
        icon: "download",
      },
      {
        page: "converter",
        zh: "订阅转换",
        en: "Converter",
        icon: "refresh-cw",
      },
      {
        page: "settings",
        zh: "网络设置",
        en: "Network settings",
        icon: "sliders-horizontal",
      },
      {
        page: "appearance",
        zh: "外观",
        en: "Appearance",
        icon: "settings",
        keybinding: "mod+,",
      },
      {
        page: "about",
        zh: "关于",
        en: "About",
        icon: "info",
      },
    ],
  },
];
export function desktopGroupTitle(page: Page): [string, string] {
  const normalized =
    page === "requests" || page === "activity-detail"
      ? "connections"
      : page === "editor" || page === "profile-detail"
        ? "profiles"
        : page === "proxy-group"
          ? "proxies"
          : page === "privacy"
            ? "about"
            : page === "advanced-diagnostics"
              ? "diagnostics"
              : page;
  if (desktopPrimary.some((item) => item.page === normalized))
    return ["主要", "Main"];
  if (sections[0].items.some((item) => item.page === normalized))
    return sections[0].title;
  return sections[1].title;
}
export const desktopRootPages: Page[] = [
  ...desktopPrimary.map((item) => item.page),
  ...sections.flatMap((section) => section.items.map((item) => item.page)),
  "requests",
];

export function DesktopNavigation(props: {
  page: Page;
  version: string;
  statusLabel?: string;
  connected?: boolean;
  navigate: (page: Page) => void;
  t: (zh: string, en: string) => string;
  expanded: boolean;
  onToggle: () => void;
}) {
  const expanded = () => props.expanded;
  const selected = (page: Page) => {
    const current =
      props.page === "requests"
        ? "connections"
        : props.page === "editor" || props.page === "profile-detail"
          ? "profiles"
          : props.page === "proxy-group"
            ? "proxies"
            : props.page === "activity-detail"
              ? "connections"
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
      hoverStyle={
        selected(item.destination.page)
          ? undefined
          : { backgroundColor: palette().hover }
      }
      activeStyle={{ backgroundColor: palette().soft }}
      style={{
        minHeight: expanded()
          ? Math.max(36, density.line(20) * Math.max(1, fontScale()) + 12)
          : 42,
        paddingHorizontal: expanded() ? spacing.md : 0,
        flexDirection: "row",
        alignItems: "center",
        justifyContent: expanded() ? "flex-start" : "center",
        gap: 10,
        borderRadius: expanded() ? 9 : 10,
        borderCurve: "continuous",
        borderWidth: 1,
        borderColor: selected(item.destination.page)
          ? palette().border
          : "transparent",
        backgroundColor: selected(item.destination.page)
          ? palette().sidebarAccent
          : "transparent",
        ...(selected(item.destination.page) ? surfaceFinish("card") : {}),
        ...feedbackStyle(),
      }}
    >
      <Icon
        name={item.destination.icon}
        size={18}
        strokeWidth={selected(item.destination.page) ? 2.1 : 1.85}
        color={
          selected(item.destination.page)
            ? palette().text
            : palette().muted
        }
      />
      <If when={expanded()}>
        <View style={{ flex: 1, minWidth: 0 }}>
          <Text
            style={{
              color: selected(item.destination.page)
                ? palette().text
                : palette().muted,
              fontSize: density.font(14),
              lineHeight: density.line(20),
              numberOfLines: 1,
              fontWeight: selected(item.destination.page) ? "600" : "500",
            }}
          >
            {props.t(item.destination.zh, item.destination.en)}
          </Text>
        </View>
        <If
          when={
            platform.desktop &&
            (!!item.shortcut || !!item.destination.keybinding)
          }
        >
          <Text
            style={{
              color: palette().faint,
              fontSize: 11,
              fontVariant: ["tabular-nums"],
            }}
          >
            {keymap.label(
              item.shortcut
                ? `mod+${item.shortcut}`
                : item.destination.keybinding!,
            )}
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
        borderRightColor: palette().sidebarBorder,
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
        onPress={props.onToggle}
        focusStyle={focusRing()}
        hoverStyle={{ backgroundColor: palette().hover }}
        activeStyle={{ backgroundColor: palette().soft }}
        style={{
          minHeight: 54,
          paddingHorizontal: expanded() ? 14 : 0,
          flexDirection: "row",
          alignItems: "center",
          justifyContent: expanded() ? "flex-start" : "center",
          gap: 10,
          borderBottomWidth: 1,
          borderBottomColor: palette().sidebarBorder,
        }}
      >
        <View
          style={{
            width: 24,
            height: 24,
            borderRadius: 7,
            borderCurve: "continuous",
            borderWidth: 1,
            alignItems: "center",
            justifyContent: "center",
            ...surfaceFinish("accent"),
          }}
        >
          <Text
            style={{
              fontSize: 12,
              lineHeight: 15,
              fontWeight: "700",
              color: palette().primaryText,
            }}
          >
            P
          </Text>
        </View>
        <If when={expanded()}>
          <Text
            style={{
              flex: 1,
              color: palette().text,
              fontSize: 15,
              fontWeight: "600",
              letterSpacing: -0.2,
            }}
          >
            Paws
          </Text>
          <Icon name="panel-left" size={16} color={palette().muted} />
        </If>
      </Button>
      <Scroll
        bounces={false}
        style={{ flex: 1 }}
        contentStyle={{
          paddingHorizontal: 8,
          paddingVertical: 10,
          gap: platform.desktop ? 10 : spacing.md,
        }}
      >
        <View style={{ gap: 2 }}>
          <If when={expanded()}>
            <Text
              role="header"
              style={{
                paddingLeft: 9,
                paddingBottom: 3,
                fontSize: 12,
                lineHeight: 16,
                fontWeight: "600",
                color: palette().muted,
              }}
            >
              {props.t("主要", "Main")}
            </Text>
          </If>
          <Each of={desktopPrimary}>
            {(destination, index) => (
              <Item destination={destination()} shortcut={index() + 1} />
            )}
          </Each>
        </View>
        <Each of={sections}>
          {(section) => (
            <View style={{ gap: 2 }}>
              <If
                when={expanded()}
                else={
                  <View
                    style={{
                      height: 1,
                      marginHorizontal: 8,
                      marginVertical: 2,
                      backgroundColor: palette().sidebarBorder,
                    }}
                  />
                }
              >
                <Text
                  role="header"
                  style={{
                    paddingLeft: 9,
                    paddingTop: 4,
                    paddingBottom: 3,
                    fontSize: 12,
                    lineHeight: 16,
                    fontWeight: "600",
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
      <If when={expanded() && (props.statusLabel || props.version)}>
        <View
          style={{
            paddingHorizontal: 14,
            paddingVertical: 10,
            borderTopWidth: 1,
            borderTopColor: palette().sidebarBorder,
            flexDirection: "row",
            alignItems: "center",
            justifyContent: "space-between",
            gap: 8,
          }}
        >
          <View
            style={{
              flex: 1,
              minWidth: 0,
              flexDirection: "row",
              alignItems: "center",
              gap: 6,
            }}
          >
            <View
              style={{
                width: 7,
                height: 7,
                borderRadius: radius.full,
                backgroundColor: props.connected
                  ? palette().success
                  : palette().faint,
              }}
            />
            <Text
              style={{
                flexShrink: 1,
                minWidth: 0,
                numberOfLines: 1,
                fontSize: 12,
                lineHeight: 16,
                color: palette().muted,
              }}
            >
              {props.statusLabel || ""}
            </Text>
          </View>
          <If when={props.version}>
            <Text
              style={{
                flexShrink: 1,
                minWidth: 0,
                maxWidth: 88,
                numberOfLines: 1,
                fontSize: 12,
                lineHeight: 16,
                color: palette().muted,
                fontVariant: ["tabular-nums"],
              }}
            >
              {props.version.startsWith("v")
                ? props.version
                : `v${props.version}`}
            </Text>
          </If>
        </View>
      </If>
    </View>
  );
}

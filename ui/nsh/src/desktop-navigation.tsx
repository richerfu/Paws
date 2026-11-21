import { Text, View, Scroll, keymap, platform, fontScale } from "nsh";
import { titleBar } from "nsh/desktop";
import { Button, Tab, Tabs as BaseTabs, Tooltip } from "nsh/base";
import { Icon, type IconName } from "./icons";
import type { Page } from "./ui/context";
import {
  chrome,
  palette,
  spacing,
  radius,
  density,
  focusRing,
  surfaceFinish,
} from "./ui/tokens";
import { feedbackStyle } from "./ui/motion";

type Destination = {
  page: Page;
  zh: string;
  en: string;
  icon: IconName;
  keybinding: string;
  testID: string;
};

/** 4 core workspaces at the top of the sidebar (no redundant group header). */
export const desktopPrimary: Destination[] = [
  {
    page: "dashboard",
    zh: "概览",
    en: "Overview",
    icon: "layout-dashboard",
    keybinding: "mod+1",
    testID: "nav-dashboard",
  },
  {
    page: "profiles",
    zh: "订阅",
    en: "Subscriptions",
    icon: "rss",
    keybinding: "mod+2",
    testID: "nav-profiles",
  },
  {
    page: "proxies",
    zh: "节点",
    en: "Proxies",
    icon: "git-branch",
    keybinding: "mod+3",
    testID: "nav-proxies",
  },
  {
    page: "traffic",
    zh: "流量",
    en: "Traffic",
    icon: "activity",
    keybinding: "mod+4",
    testID: "nav-traffic",
  },
];

/** 4 network inspection & diagnostics workspaces under one concise group. */
export const desktopNetwork: Destination[] = [
  {
    page: "connections",
    zh: "连接",
    en: "Connections",
    icon: "list-filter",
    keybinding: "mod+5",
    testID: "desktop-nav-connections",
  },
  {
    page: "resources",
    zh: "规则",
    en: "Rules",
    icon: "layers",
    keybinding: "mod+6",
    testID: "desktop-nav-resources",
  },
  {
    page: "logs",
    zh: "日志",
    en: "Logs",
    icon: "file-text",
    keybinding: "mod+7",
    testID: "desktop-nav-logs",
  },
  {
    page: "diagnostics",
    zh: "诊断",
    en: "Diagnostics",
    icon: "shield-check",
    keybinding: "mod+8",
    testID: "desktop-nav-diagnostics",
  },
];

/** Single unified Settings destination pinned at the bottom (Network / Appearance / Backups / About). */
export const desktopSettings: Destination = {
  page: "settings",
  zh: "设置",
  en: "Settings",
  icon: "settings",
  keybinding: "mod+,",
  testID: "desktop-nav-settings",
};

export const desktopShortcuts: { key: string; page: Page }[] = [
  ...desktopPrimary,
  ...desktopNetwork,
  desktopSettings,
].map((item) => ({ key: item.keybinding, page: item.page }));

export type SettingsSection = "settings" | "appearance" | "backups" | "about";
export const SETTINGS_SECTIONS: SettingsSection[] = [
  "settings",
  "appearance",
  "backups",
  "about",
];
export function isSettingsSection(page: Page): page is SettingsSection {
  return (SETTINGS_SECTIONS as Page[]).includes(page);
}

function normalizeDesktopDestination(page: Page): Page {
  if (page === "requests" || page === "activity-detail") return "connections";
  if (page === "editor" || page === "profile-detail" || page === "converter")
    return "profiles";
  if (page === "proxy-group") return "proxies";
  if (page === "dns-diagnostics") return "traffic";
  if (page === "advanced-diagnostics") return "diagnostics";
  if (isSettingsSection(page) || page === "privacy") return "settings";
  return page;
}

/** Return the enclosing group breadcrumb only when a real secondary group adds context. */
export function desktopGroupTitle(page: Page): [string, string] | undefined {
  const normalized = normalizeDesktopDestination(page);
  if (desktopNetwork.some((item) => item.page === normalized))
    return ["网络", "Network"];
  return undefined;
}

export const desktopRootPages: Page[] = [
  ...desktopPrimary.map((item) => item.page),
  ...desktopNetwork.map((item) => item.page),
  desktopSettings.page,
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
  const selected = (page: Page) =>
    normalizeDesktopDestination(props.page) === page;

  const Item = (item: { destination: Destination }) => (
    <Tab
      tooltip={
        !expanded() ? (
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
              gap: spacing.sm,
            }}
          >
            <Text style={{ fontSize: density.font(12), color: palette().text }}>
              {props.t(item.destination.zh, item.destination.en)}
            </Text>
            <If when={platform.desktop}>
              <Text
                style={{
                  fontSize: density.font(11),
                  color: palette().muted,
                  fontVariant: ["tabular-nums"],
                }}
              >
                {keymap.label(item.destination.keybinding)}
              </Text>
            </If>
          </Tooltip>
        ) : undefined
      }
      testID={item.destination.testID}
      accessibilityLabel={props.t(item.destination.zh, item.destination.en)}
      selected={selected(item.destination.page)}
      onPress={() => props.navigate(item.destination.page)}
      focusVisibleStyle={focusRing()}
      hoverStyle={
        selected(item.destination.page)
          ? undefined
          : { backgroundColor: palette().hover }
      }
      activeStyle={{ backgroundColor: palette().soft }}
      style={{
        // Pointer: 28 pt rows and 36 pt rail tiles with 6 pt radius and subtle 1 pt selected edge
        // (matching NSH Gallery). Touch: 36 pt rows and 42 pt rail tiles.
        minHeight: platform.desktop
          ? expanded()
            ? Math.max(28, density.line(18) * Math.max(1, fontScale()) + 10)
            : 36
          : expanded()
            ? Math.max(36, density.line(20) * Math.max(1, fontScale()) + 12)
            : 42,
        paddingHorizontal: expanded() ? (platform.desktop ? 8 : spacing.md) : 0,
        flexDirection: "row",
        alignItems: "center",
        justifyContent: expanded() ? "flex-start" : "center",
        gap: platform.desktop ? 9 : 10,
        borderRadius: platform.desktop ? (expanded() ? 6 : 8) : expanded() ? 9 : 10,
        borderCurve: "continuous",
        borderWidth: 1,
        borderColor: selected(item.destination.page)
          ? palette().border
          : "transparent",
        backgroundColor: selected(item.destination.page)
          ? palette().sidebarAccent
          : "transparent",
        ...feedbackStyle(),
      }}
    >
      <Icon
        name={item.destination.icon}
        size={platform.desktop && expanded() ? 16 : 18}
        strokeWidth={selected(item.destination.page) ? 2.1 : 1.85}
        color={
          selected(item.destination.page) ? palette().text : palette().muted
        }
      />
      <If when={expanded()}>
        <View style={{ flex: 1, minWidth: 0 }}>
          <Text
            style={{
              color: selected(item.destination.page)
                ? palette().text
                : palette().muted,
              fontSize: density.font(platform.desktop ? 13 : 14),
              lineHeight: density.line(platform.desktop ? 18 : 20),
              numberOfLines: 1,
              fontWeight: selected(item.destination.page)
                ? platform.desktop
                  ? "500"
                  : "600"
                : platform.desktop
                  ? "400"
                  : "500",
            }}
          >
            {props.t(item.destination.zh, item.destination.en)}
          </Text>
        </View>
        <If when={platform.desktop}>
          <Text
            style={{
              color: palette().faint,
              fontSize: 11,
              fontVariant: ["tabular-nums"],
            }}
          >
            {keymap.label(item.destination.keybinding)}
          </Text>
        </If>
      </If>
    </Tab>
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
      <View
        desktop={platform.desktop ? { windowDrag: true } : undefined}
        style={{
          minHeight: platform.desktop
            ? Math.max(chrome.toolbar, titleBar().height)
            : chrome.toolbar + 4,
          paddingTop:
            platform.desktop && !expanded() && titleBar().left > 0
              ? titleBar().height
              : 0,
          paddingLeft:
            platform.desktop && expanded() && titleBar().left > 0
              ? titleBar().left + 8
              : 8,
          paddingRight: 8,
          justifyContent: "center",
        }}
      >
        <View desktop={platform.desktop ? { windowDrag: false } : undefined}>
          <Button
            testID="desktop-toggle-sidebar"
            tooltip={
              !expanded() ? (
                <Tooltip
                  style={{
                    maxWidth: 240,
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
                  <Text
                    style={{
                      fontSize: density.font(12),
                      color: palette().text,
                    }}
                  >
                    {props.t("展开导航栏", "Expand sidebar")}
                  </Text>
                  <If when={platform.desktop}>
                    <Text
                      style={{
                        fontSize: density.font(11),
                        color: palette().muted,
                        fontVariant: ["tabular-nums"],
                      }}
                    >
                      {keymap.label("mod+b")}
                    </Text>
                  </If>
                </Tooltip>
              ) : undefined
            }
            accessibilityLabel={
              expanded()
                ? props.t("收起导航栏", "Collapse sidebar")
                : props.t("展开导航栏", "Expand sidebar")
            }
            onPress={props.onToggle}
            focusVisibleStyle={focusRing()}
            hoverStyle={{ backgroundColor: palette().hover }}
            activeStyle={{ backgroundColor: palette().soft }}
            style={{
              minHeight: platform.desktop ? 34 : 40,
              paddingHorizontal: expanded() ? 8 : 0,
              flexDirection: "row",
              alignItems: "center",
              justifyContent: expanded() ? "flex-start" : "center",
              gap: 9,
              borderRadius: platform.desktop ? 6 : 9,
              borderCurve: "continuous",
              ...feedbackStyle(),
            }}
          >
            <If
              when={expanded()}
              else={
                <Icon
                  name="panel-left"
                  size={18}
                  strokeWidth={1.85}
                  color={palette().muted}
                />
              }
            >
              <View
                style={{
                  width: 26,
                  height: 26,
                  borderRadius: 8,
                  borderCurve: "continuous",
                  borderWidth: 1,
                  alignItems: "center",
                  justifyContent: "center",
                  ...surfaceFinish("accent"),
                }}
              >
                <Text
                  style={{
                    fontSize: 13,
                    lineHeight: 16,
                    fontWeight: "700",
                    color: palette().primaryText,
                  }}
                >
                  P
                </Text>
              </View>
              <Text
                style={{
                  flex: 1,
                  color: palette().text,
                  fontSize: 16,
                  fontWeight: "600",
                  letterSpacing: -0.16,
                }}
              >
                Paws
              </Text>
              <Icon name="panel-left" size={16} color={palette().muted} />
            </If>
          </Button>
        </View>
      </View>
      <Scroll
        bounces={false}
        style={{ flex: 1 }}
        contentStyle={{
          flexGrow: 1,
          paddingHorizontal: 8,
          paddingTop: 6,
          paddingBottom: 10,
        }}
      >
        <BaseTabs
          accessibilityLabel={props.t("导航", "Navigation")}
          style={{
            flexGrow: 1,
            gap: platform.desktop ? 10 : spacing.md,
          }}
        >
          <View style={{ gap: 2 }}>
            <Each of={desktopPrimary}>
              {(destination) => <Item destination={destination()} />}
            </Each>
          </View>
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
                {props.t("网络", "Network")}
              </Text>
            </If>
            <Each of={desktopNetwork}>
              {(destination) => <Item destination={destination()} />}
            </Each>
          </View>
          <View style={{ flex: 1, minHeight: 8 }} />
          <View style={{ gap: 6 }}>
            <View
              style={{
                height: 1,
                marginHorizontal: expanded() ? 4 : 8,
                backgroundColor: palette().sidebarBorder,
              }}
            />
            <Item destination={desktopSettings} />
          </View>
        </BaseTabs>
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

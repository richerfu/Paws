import {
  cleanup,
  keyboardFrame,
  keymap,
  platform,
  reduceMotion,
  safeArea,
  Scroll,
  Text,
  View,
  windowSize,
  type JSX,
} from "nsh";
import {
  Button,
  Dialog as NativeDialog,
  Sheet as BaseSheet,
} from "nsh/base";
import { Icon, type IconName } from "../icons";
import {
  NotificationViewport,
  useNotificationLayer,
} from "../notifications";
import { IconButton } from "./button";
import { App, useOverlay } from "./context";
import { feedbackStyle, usePopIn } from "./motion";
import { Hint } from "./surface";
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

export function Dialog(props: {
  title: string;
  subtitle?: string;
  description?: string;
  onClose: () => void;
  afterClose?: () => void;
  children?: JSX.Element;
  footer?: JSX.Element;
  width?: number;
}) {
  const ui = App();
  const notificationLayer = useNotificationLayer();
  const enter = usePopIn();
  useOverlay(() => true);
  let afterClosed = false;
  const runAfterClose = () => {
    if (afterClosed) return;
    afterClosed = true;
    props.afterClose?.();
  };
  // Own Escape so an asynchronous draft decision can keep this dialog open.
  cleanup(keymap.bind("escape", () => props.onClose(), { when: "always" }));
  cleanup(() => {
    if (!afterClosed && props.afterClose) {
      setTimeout(runAfterClose, 0);
    }
  });
  return (
    <NativeDialog
      open
      closeOnEscape={false}
      closeOnBackdropPress={false}
      closeOnSwipe={false}
      onCancel={props.onClose}
      onClose={props.afterClose ? runAfterClose : undefined}
      animation={reduceMotion() ? "none" : "fade"}
      style={{ flex: 1 }}
    >
      <View style={{ flex: 1, paddingBottom: "env(keyboard-inset-height)" }}>
        <View
          onPress={props.onClose}
          style={{
            position: "absolute",
            top: 0,
            right: 0,
            bottom: 0,
            left: 0,
            backgroundColor: palette().scrim,
          }}
        />
        <View
          style={{
            flex: 1,
            minHeight: 0,
            alignItems: "center",
            justifyContent: "center",
            paddingHorizontal: spacing.lg,
            paddingVertical: spacing.xxl,
          }}
        >
          <View
            accessibilityLabel={props.title}
            style={{
              width: "100%",
              maxWidth: props.width || 480,
              maxHeight: "100%",
              minHeight: 0,
              borderRadius: radius.xxl,
              borderCurve: "continuous",
              backgroundColor: palette().popover,
              borderWidth: 1,
              borderColor: palette().border,
              opacity: enter.opacity,
              transform: [{ scale: enter.scale }],
              ...surfaceFinish("overlay"),
            }}
          >
            <View
              style={{
                flexDirection: "row",
                alignItems: "flex-start",
                gap: spacing.sm,
                paddingLeft: spacing.lg,
                paddingRight: spacing.xs,
                paddingTop: 12,
                paddingBottom: 10,
                borderBottomWidth: 1,
                borderBottomColor: palette().line,
              }}
            >
              <View style={{ flex: 1, minWidth: 0, gap: 2, paddingTop: 4 }}>
                <Text
                  role="header"
                  accessibilityLabel={props.title}
                  style={{
                    minWidth: 0,
                    numberOfLines: 2,
                    fontSize: platform.desktop ? typography.md : typography.xl,
                    lineHeight: density.line(platform.desktop ? 24 : 26),
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
                      fontSize: typography.note,
                      lineHeight: density.line(18),
                      color: palette().muted,
                    }}
                  >
                    {props.subtitle}
                  </Text>
                </If>
              </View>
              <IconButton
                icon="x"
                tooltip={false}
                label={ui?.t("关闭对话框", "Close dialog") || "Close dialog"}
                id="dialog-close"
                onPress={props.onClose}
              />
            </View>
            <Scroll
              bounces={false}
              keyboardDismissMode="on-drag"
              keyboardShouldPersistTaps="handled"
              style={{ flexShrink: 1, minHeight: 0 }}
              contentStyle={{ padding: spacing.lg, gap: 14 }}
            >
              <If when={props.description}>
                <Hint text={props.description!} />
              </If>
              {props.children}
            </Scroll>
            <If when={props.footer}>
              <View
                style={{
                  padding: spacing.lg,
                  borderTopWidth: 1,
                  borderTopColor: palette().line,
                }}
              >
                {props.footer}
              </View>
            </If>
          </View>
        </View>
        <NotificationViewport
          owner={notificationLayer}
          modal
          dismissLabel={ui?.t("关闭通知", "Dismiss notification")}
        />
      </View>
    </NativeDialog>
  );
}

/** Responsive sheet: bottom slide-in `Sheet` with grabber on narrow touch (< 768pt),
 * automatically promoted to a centered `Dialog` on desktop and wide screens (>= 768pt). */
export function BottomSheet(props: {
  title: string;
  subtitle?: string;
  onClose: () => void;
  afterClose?: () => void;
  children?: JSX.Element;
  footer?: JSX.Element;
}) {
  const promoteToDialog = () => platform.desktop || windowSize().width >= 768;
  return (
    <If
      when={promoteToDialog()}
      else={<TouchBottomSheet {...props} />}
    >
      <Dialog
        width={480}
        title={props.title}
        subtitle={props.subtitle}
        onClose={props.onClose}
        afterClose={props.afterClose}
        footer={props.footer}
      >
        {props.children}
      </Dialog>
    </If>
  );
}

function TouchBottomSheet(props: {
  title: string;
  subtitle?: string;
  onClose: () => void;
  afterClose?: () => void;
  children?: JSX.Element;
  footer?: JSX.Element;
}) {
  const ui = App();
  useOverlay(() => true);
  let afterClosed = false;
  const runAfterClose = () => {
    if (afterClosed) return;
    afterClosed = true;
    props.afterClose?.();
  };
  cleanup(keymap.bind("escape", () => props.onClose(), { when: "always" }));
  cleanup(() => {
    if (!afterClosed && props.afterClose) {
      setTimeout(runAfterClose, 0);
    }
  });
  return (
    <BaseSheet
      open
      side="bottom"
      closeOnEscape={false}
      closeOnBackdropPress={false}
      closeOnSwipe={false}
      grabber
      grabberAccessibilityLabel={
        ui?.t("面板抓手", "Sheet handle") || "Sheet handle"
      }
      grabberStyle={{
        width: 36,
        height: 5,
        borderRadius: radius.full,
        backgroundColor: palette().border,
        alignSelf: "center",
        marginTop: 8,
        marginBottom: 4,
      }}
      accessibilityLabel={props.title}
      onCancel={props.onClose}
      onClose={props.afterClose ? runAfterClose : undefined}
      backdropStyle={{ backgroundColor: palette().scrim }}
      popupStyle={{
        width: "100%",
        maxHeight: Math.max(
          240,
          windowSize().height -
            safeArea().top -
            Math.max(0, keyboardFrame().height - safeArea().bottom) -
            24,
        ),
        minHeight: 0,
        borderTopLeftRadius: radius.sheet,
        borderTopRightRadius: radius.sheet,
        borderCurve: "continuous",
        backgroundColor: palette().popover,
        borderTopWidth: 1,
        borderLeftWidth: 1,
        borderRightWidth: 1,
        borderBottomWidth: 0,
        borderColor: palette().border,
        paddingBottom:
          safeArea().bottom > 0
            ? Math.max(8, Math.round(safeArea().bottom * 0.55))
            : 0,
        ...surfaceFinish("overlay"),
      }}
      footer={props.footer}
      footerStyle={
        props.footer
          ? {
              paddingHorizontal: spacing.lg,
              paddingTop: 12,
              paddingBottom: safeArea().bottom > 0 ? spacing.sm : spacing.lg,
              borderTopWidth: 1,
              borderTopColor: palette().line,
            }
          : undefined
      }
    >
      <View
        style={{
          flexDirection: "row",
          alignItems: "flex-start",
          gap: spacing.sm,
          paddingLeft: spacing.lg,
          paddingRight: spacing.xs,
          paddingBottom: 10,
          borderBottomWidth: 1,
          borderBottomColor: palette().line,
        }}
      >
        <View style={{ flex: 1, minWidth: 0, gap: 2, paddingTop: 4 }}>
          <Text
            role="header"
            accessibilityLabel={props.title}
            style={{
              minWidth: 0,
              numberOfLines: 2,
              fontSize: typography.md,
              lineHeight: density.line(24),
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
                fontSize: typography.note,
                lineHeight: density.line(18),
                color: palette().muted,
              }}
            >
              {props.subtitle}
            </Text>
          </If>
        </View>
        <IconButton
          icon="x"
          tooltip={false}
          label={ui?.t("关闭面板", "Close sheet") || "Close sheet"}
          id="dialog-close"
          onPress={props.onClose}
        />
      </View>
      <Scroll
        bounces={false}
        keyboardDismissMode="on-drag"
        keyboardShouldPersistTaps="handled"
        style={{ flexShrink: 1, minHeight: 0 }}
        contentStyle={{ padding: spacing.lg, gap: 14 }}
      >
        {props.children}
      </Scroll>
    </BaseSheet>
  );
}

/** Action item inside a BottomSheet, matching Gallery menuItemSize (46pt touch / 26pt pointer). */
export function SheetActionItem(props: {
  id?: string;
  icon: IconName;
  label: string;
  danger?: boolean;
  disabled?: boolean;
  onPress: () => void;
}) {
  const color = () => (props.danger ? palette().danger : palette().text);
  return (
    <Button
      testID={props.id}
      accessibilityLabel={props.label}
      disabled={props.disabled}
      focusVisibleStyle={focusRing(0)}
      hoverStyle={
        props.disabled ? undefined : { backgroundColor: palette().hover }
      }
      activeStyle={
        props.disabled ? undefined : { backgroundColor: palette().soft }
      }
      onPress={() => {
        if (!props.disabled) props.onPress();
      }}
      style={{
        minHeight: control.menuRow,
        paddingHorizontal: platform.desktop ? 8 : 12,
        paddingVertical: 0,
        borderRadius: control.menuRadius,
        borderCurve: "continuous",
        flexDirection: "row",
        alignItems: "center",
        gap: platform.desktop ? 8 : 10,
        backgroundColor: "transparent",
        opacity: props.disabled ? 0.45 : 1,
        ...feedbackStyle(),
      }}
    >
      <Icon
        name={props.icon}
        size={platform.desktop ? 14 : 18}
        color={props.danger ? palette().danger : palette().muted}
      />
      <Text
        style={{
          flex: 1,
          minWidth: 0,
          fontSize: density.font(platform.desktop ? 13 : 16),
          lineHeight: density.line(platform.desktop ? 18 : 22),
          fontWeight: "400",
          color: color(),
        }}
      >
        {props.label}
      </Text>
    </Button>
  );
}

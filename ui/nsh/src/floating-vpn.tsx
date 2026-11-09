import {
  cleanup,
  computed,
  createAnimatedValue,
  effect,
  reduceMotion,
  safeArea,
  Overlay,
  windowSize,
} from "nsh";
import { useApp } from "./components";
import { VpnPowerButton } from "./pages/overview";
import { chrome, control, spacing } from "./theme";

const clamp = (value: number, maximum: number) =>
  Math.max(0, Math.min(value, maximum));

/** Keep this owner mounted when changing tabs so the dock position survives. */
export function FloatingVpnPowerButton(props: { visible: boolean }) {
  const ui = useApp();
  const size = () => (control.compact ? 48 : 56);
  const area = computed(() => {
    const viewport = windowSize(),
      insets = safeArea();
    return {
      left: insets.left + spacing.xxl,
      top: insets.top + chrome.header + spacing.lg,
      width: Math.max(
        size(),
        viewport.width - insets.left - insets.right - spacing.xxl * 2,
      ),
      height: Math.max(
        size(),
        viewport.height -
          insets.top -
          insets.bottom -
          chrome.header -
          chrome.bottom -
          spacing.lg * 2,
      ),
    };
  });
  const maxX = () => area().width - size(),
    maxY = () => area().height - size();
  const x = createAnimatedValue(maxX()),
    y = createAnimatedValue(maxY());
  let right = true,
    vertical = 1;
  // Clamp the rendered position on the UI thread, including while the finger
  // moves past the safe area. JS only handles release and viewport changes.
  const translateX = computed(() =>
    x.interpolate({
      inputRange: [0, Math.max(1, maxX())],
      outputRange: [0, maxX()],
    }),
  );
  const translateY = computed(() =>
    y.interpolate({
      inputRange: [0, Math.max(1, maxY())],
      outputRange: [0, maxY()],
    }),
  );
  effect(() => {
    x.set(right ? maxX() : 0);
    y.set(vertical * maxY());
  });
  cleanup(() => {
    void x.stop();
    void y.stop();
  });
  return (
    <If when={props.visible}>
      <Overlay
        testID="vpn-floating-area"
        style={{
          position: "fixed",
          left: area().left,
          top: area().top,
          width: size(),
          height: size(),
          transform: [
            { translateX: translateX() },
            { translateY: translateY() },
          ],
        }}
      >
        <VpnPowerButton
          size={size()}
          accessibilityHint={ui.t(
            "点击切换 VPN，拖动后自动吸附屏幕边缘",
            "Tap to toggle VPN. Drag to dock at either edge.",
          )}
          pan={{ x, y }}
          onPanEnd={(event) => {
            const releaseX = clamp(
              event.value?.x ?? (right ? maxX() : 0) + event.translation.x,
              maxX(),
            );
            const releaseY = clamp(
              event.value?.y ?? vertical * maxY() + event.translation.y,
              maxY(),
            );
            right = releaseX >= maxX() / 2;
            vertical = maxY() ? releaseY / maxY() : vertical;
            // Normalize overshoot before the spring, so dragging out of bounds
            // never leaves an invisible part of the snap animation to play.
            x.set(releaseX);
            y.set(releaseY);
            const target = right ? maxX() : 0;
            if (reduceMotion()) x.set(target);
            else void x.spring(target, { stiffness: 320, damping: 32 });
          }}
        />
      </Overlay>
    </If>
  );
}

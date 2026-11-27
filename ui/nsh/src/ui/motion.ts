import {
  cleanup,
  createAnimatedValue,
  createMotion,
  Easing,
  effect,
  LayoutAnimation,
  reduceMotion,
  type AnimatedValue,
  type ViewStyle,
} from "nsh";

/**
 * NSH Gallery native control springs & motion tokens.
 * - Live pointer drags & telemetry counters never follow a spring.
 * - Every helper honors `reduceMotion()` by settling immediately.
 * - Sound Policy: Paws is silent by default; feedback relies on clear visual state
 *   changes, toasts, and screen-reader announcements (`announce()`).
 */
export const motion = {
  press: { stiffness: 520, damping: 34 },
  control: { stiffness: 1218, damping: 70 },
  move: { stiffness: 504, damping: 38 },
  feedback: 120,
  disclosure: 180,
} as const;

export function follow(
  target: () => number,
  spring: { stiffness: number; damping: number } = motion.control,
): AnimatedValue {
  const value = createAnimatedValue(target());
  let shown = target();
  effect(() => {
    const next = target();
    const reduced = reduceMotion();
    if (next === shown && !reduced) return;
    shown = next;
    if (reduced) value.set(next);
    else value.spring(next, spring);
  });
  cleanup(() => {
    void value.stop();
  });
  return value;
}

export function useGlyphTransition(
  key: () => string | undefined,
): AnimatedValue {
  const value = createAnimatedValue(1);
  let shown = key();
  effect(() => {
    const next = key();
    const reduced = reduceMotion();
    if (next === shown && !reduced) return;
    shown = next;
    if (reduced) value.set(1);
    else {
      value.set(0.65);
      value.spring(1, motion.control);
    }
  });
  cleanup(() => {
    void value.stop();
  });
  return value;
}

export function usePressFeedback(disabled: () => boolean, activeScale = 0.97) {
  const scale = createAnimatedValue(1);
  effect(() => {
    if (disabled() || reduceMotion()) scale.set(1);
  });
  cleanup(() => {
    void scale.stop();
  });
  return {
    scale,
    down: () => {
      if (!disabled() && !reduceMotion()) scale.spring(activeScale, motion.press);
    },
    up: () => {
      if (reduceMotion()) scale.set(1);
      else scale.spring(1, motion.press);
    },
  };
}

export function usePopIn() {
  const enter = createMotion({
    enter: {
      duration: 150,
      easing: Easing.bezier(0.16, 1, 0.3, 1),
      keyframes: { opacity: [0.01, 1], scale: [0.96, 1] },
    },
  });
  cleanup(() => enter.stop());
  return enter;
}

export function feedbackStyle(): ViewStyle {
  return {
    transitionProperty: "background-color, border-color, color, opacity",
    transitionDuration: reduceMotion() ? 0 : motion.feedback,
    transitionTimingFunction: "ease-out",
  };
}

/** Discrete user-triggered disclosure/reordering only; never called on resize or polling ticks. */
export function animateDisclosure(): void {
  if (!reduceMotion()) {
    LayoutAnimation.configureNext({
      duration: motion.disclosure,
      create: { type: "easeOut", property: "opacity" },
      update: { type: "easeInEaseOut" },
      delete: { type: "easeOut", property: "opacity" },
    });
  }
}

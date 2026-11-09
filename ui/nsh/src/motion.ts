import {
  cleanup,
  createAnimatedValue,
  effect,
  LayoutAnimation,
  reduceMotion,
  type AnimatedValue,
  type ViewStyle,
} from "nsh";

/** NSH Gallery's native control springs; live pointer positions never follow a spring. */
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

export function useGlyphTransition(key: () => string | undefined): AnimatedValue {
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

export function usePressFeedback(disabled: () => boolean) {
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
      if (!disabled() && !reduceMotion()) scale.spring(0.97, motion.press);
    },
    up: () => {
      if (reduceMotion()) scale.set(1);
      else scale.spring(1, motion.press);
    },
  };
}

export function feedbackStyle(): ViewStyle {
  return {
    transitionProperty: "background-color, border-color, color, opacity",
    transitionDuration: reduceMotion() ? 0 : motion.feedback,
    transitionTimingFunction: "ease-out",
  };
}

/** Discrete expansion and reordering only; telemetry never requests layout animation. */
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

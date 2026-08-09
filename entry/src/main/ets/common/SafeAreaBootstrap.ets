import window from '@ohos.window';

export interface SafeAreaInsets {
  top: number;
  right: number;
  bottom: number;
  left: number;
}

const VISUAL_AVOID_AREA_TYPES: window.AvoidAreaType[] = [
  window.AvoidAreaType.TYPE_SYSTEM,
  window.AvoidAreaType.TYPE_CUTOUT,
  window.AvoidAreaType.TYPE_NAVIGATION_INDICATOR,
];

let safeAreaInsets: SafeAreaInsets = {
  top: 0,
  right: 0,
  bottom: 0,
  left: 0,
};

/** Capture the visual safe area once, before the first page is loaded. */
export function initializeSafeArea(win: window.Window): void {
  let top = 0;
  let right = 0;
  let bottom = 0;
  let left = 0;

  for (const type of VISUAL_AVOID_AREA_TYPES) {
    try {
      const area = win.getWindowAvoidArea(type);
      if (!area.visible) {
        continue;
      }
      top = Math.max(top, area.topRect.height);
      right = Math.max(right, area.rightRect.width);
      bottom = Math.max(bottom, area.bottomRect.height);
      left = Math.max(left, area.leftRect.width);
    } catch (_) {
      // Unsupported avoid-area types must not block the first page load.
    }
  }

  safeAreaInsets = { top, right, bottom, left };
}

/** Read-only by convention: the page takes this snapshot during construction. */
export function getSafeAreaInsets(): SafeAreaInsets {
  return safeAreaInsets;
}

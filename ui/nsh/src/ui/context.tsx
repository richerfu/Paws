import {
  Clipboard,
  cleanup,
  computed,
  context,
  effect,
  fontScale,
  platform,
  windowSize,
  type Signal,
} from "nsh";
import { AlertDialog } from "nsh/base";
import type { CoreClient } from "../client";
import type { DomainInput } from "../model";
import { isChinese } from "../locale";
import { workspaceLayout, type WorkspaceLayout } from "../layout";

export type Page =
  | "dashboard"
  | "profiles"
  | "profile-detail"
  | "proxies"
  | "proxy-group"
  | "traffic"
  | "dns-diagnostics"
  | "tools"
  | "settings"
  | "appearance"
  | "connections"
  | "requests"
  | "activity-detail"
  | "resources"
  | "logs"
  | "diagnostics"
  | "advanced-diagnostics"
  | "backups"
  | "converter"
  | "about"
  | "privacy"
  | "editor";

export type AppContext = {
  client: CoreClient;
  page: () => Page;
  route: () => unknown;
  alive: () => boolean;
  navigate: (page: Page, query?: string) => void;
  back: () => void;
  query: () => string;
  importIntent: Signal<number>;
  consumeImportIntent: (id: number) => boolean;
  resourceIntent: Signal<{ id: number; kind: "lookup" | "add" } | undefined>;
  consumeResourceIntent: (id: number) => boolean;
  logRecording: Signal<boolean>;
  logRecordingIntent: Signal<number>;
  refreshProfiles?: () => Promise<void>;
  editProfile: (id: string) => void;
  switchActivity?: (mode: "connections" | "requests") => void;
  beforeLeave?: (
    guard: () => boolean | Promise<boolean>,
    owner?: unknown,
  ) => () => void;
  layout?: () => WorkspaceLayout;
  viewState?: Map<string, unknown>;
  overlays?: Signal<number>;
  t: (zh: string, en: string) => string;
};

export const App = context<AppContext | undefined>(undefined);
export const RouteOwner = context<unknown>(undefined);
export const RouteLayout = context<(() => WorkspaceLayout) | undefined>(
  undefined,
);

export function useApp(): AppContext {
  const value = App();
  if (!value) throw new Error("Paws UI context is missing");
  return value;
}

export function useLayout(): () => WorkspaceLayout {
  const scoped = RouteLayout();
  if (scoped) return scoped;
  const ui = useApp();
  return () =>
    ui.layout?.() ||
    workspaceLayout(
      windowSize().width,
      platform.desktop,
      undefined,
      fontScale(),
    );
}

/** Session-only state for collection/detail pairs. No preference writes. */
export function useViewState<T>(key: string, create: () => T): T {
  const ui = useApp();
  if (!ui.viewState) return create();
  if (!ui.viewState.has(key)) ui.viewState.set(key, create());
  return ui.viewState.get(key) as T;
}

export function useOverlay(open: () => boolean) {
  const ui = App();
  effect(() => {
    if (!open() || !ui?.overlays) return;
    ui.overlays.update((count) => count + 1);
    cleanup(() => ui.overlays!.update((count) => Math.max(0, count - 1)));
  });
}

export type ConfirmationOptions = {
  actionLabel: string;
  cancelLabel?: string;
  description?: string;
  destructive?: boolean;
};

export async function askConfirmation(
  ui: AppContext,
  title: string,
  options?: ConfirmationOptions,
): Promise<boolean> {
  const route = ui.route();
  ui.overlays?.update((count) => count + 1);
  try {
    const result = await AlertDialog.show({
      title,
      description: options?.description,
      closeOnEscape: true,
      actions: [
        {
          id: "cancel",
          label: options?.cancelLabel || ui.t("取消", "Cancel"),
          role: "cancel",
        },
        {
          id: "commit",
          label: options?.actionLabel || ui.t("继续", "Continue"),
          role: options?.destructive === false ? "default" : "destructive",
        },
      ],
    });
    return result.action === "commit" && ui.alive() && ui.route() === route;
  } finally {
    ui.overlays?.update((count) => Math.max(0, count - 1));
  }
}

export async function confirm(
  ui: AppContext,
  title: string,
  action: () => void,
  options?: ConfirmationOptions,
): Promise<void> {
  if (await askConfirmation(ui, title, options)) action();
}

/** Register every draft with the current route, and reuse its discard choice
 * for modal dismissal. Auto-saved converter drafts use their own policy. */
export function useDraftGuard(
  dirty: () => boolean,
  discard: () => void,
  busy: () => boolean = () => false,
) {
  const ui = useApp();
  const check = async () => {
    if (busy()) return false;
    if (!dirty()) return true;
    const accepted = await askConfirmation(
      ui,
      ui.t("放弃未保存的修改？", "Discard unsaved changes?"),
      {
        actionLabel: ui.t("放弃修改", "Discard changes"),
        cancelLabel: ui.t("继续编辑", "Keep editing"),
      },
    );
    if (accepted) discard();
    return accepted;
  };
  cleanup(ui.beforeLeave?.(check, RouteOwner() || ui.route()) || (() => {}));
  return check;
}

export function copyText(ui: AppContext, text: string, notice: string): void {
  Clipboard.writeText(text);
  ui.client.notice.set(notice);
}

export function useDomainInterests(inputs: () => DomainInput[]): void {
  const ui = useApp(),
    route = RouteOwner() || ui.route();
  effect(() => {
    if (ui.route() !== route) return;
    const required = inputs();
    if (required.length) cleanup(ui.client.domains.acquire(required));
  });
}

export function useLifetime(): () => boolean {
  let alive = true;
  cleanup(() => {
    alive = false;
  });
  return () => alive;
}

export function translator(client: CoreClient) {
  // A theme/advanced preference change must not invalidate every translated
  // binding or recreate option objects in controls throughout the app.
  const chinese = computed(() => isChinese(client.preferences()));
  return (zh: string, en: string) => (chinese() ? zh : en);
}

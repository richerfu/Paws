import {
  batch,
  cleanup,
  computed,
  context,
  effect,
  signal,
  Text,
  untracked,
  View,
  createAnimatedValue as createNativeValue,
  safeArea,
  safeAreaInsets as useSafeAreaInsets,
} from "nsh";
import { Icon } from "./icons";
import { palette, radius, surfaceFinish } from "./theme";

export type Notification = { id: number; message: string; created: number; leaving: boolean };

// Bounded feedback shared by pages and modal layers; each message expires
// independently, including entries hidden behind the three-card viewport.
export class NotificationCenter {
  readonly items = signal<Notification[]>([]);
  readonly expanded = signal(false);
  readonly layers = signal<number[]>([]);
  private nextId = 0;
  private nextLayer = 0;
  private readonly timers = new Map<number, number>();
  constructor(
    private readonly duration = 2200,
    private readonly exit = 260,
  ) {}
  publish(message: string): number {
    const id = ++this.nextId;
    const current = this.items();
    if (current.length === 32) {
      clearTimeout(this.timers.get(current[0].id));
      this.timers.delete(current[0].id);
    }
    this.items.set([...current.slice(-31), { id, message, created: Date.now(), leaving: false }]);
    this.timers.set(
      id,
      setTimeout(() => this.dismiss(id), this.duration),
    );
    return id;
  }
  dismiss(id: number): void {
    if (!this.items().some((item) => item.id === id && !item.leaving)) return;
    clearTimeout(this.timers.get(id));
    this.items.set(
      this.items().map((item) => (item.id === id ? { ...item, leaving: true } : item)),
    );
    this.timers.set(
      id,
      setTimeout(() => {
        this.timers.delete(id);
        batch(() => {
          this.items.set(this.items().filter((item) => item.id !== id));
          if (this.items().filter((item) => !item.leaving).length < 2) this.expanded.set(false);
        });
      }, this.exit),
    );
  }
  openLayer(): number {
    const id = ++this.nextLayer;
    this.layers.set([...this.layers(), id]);
    return id;
  }
  closeLayer(id: number): void {
    this.layers.set(this.layers().filter((layer) => layer !== id));
  }
  dispose(): void {
    for (const timer of this.timers.values()) clearTimeout(timer);
    this.timers.clear();
  }
}

export const Notifications = context<NotificationCenter | undefined>(undefined);
export function useNotificationLayer(): number {
  const center = Notifications();
  if (!center) return 0;
  const id = untracked(() => center.openLayer());
  cleanup(() => untracked(() => center.closeLayer(id)));
  return id;
}

export function NotificationViewport(props: { owner?: number; modal?: boolean }) {
  const center = Notifications();
  if (!center) return;
  const insets = useSafeAreaInsets();
  const active = () => (center.layers().at(-1) || 0) === (props.owner || 0);
  let painted = new Set<number>();
  const entries = computed(() => {
    const items = center.items().slice().reverse();
    const live = items.filter((item) => !item.leaving);
    const visible = center.expanded() ? live : live.slice(0, 3);
    // A queued card which was never visible must not flash into the deck just
    // because its independent timer expired behind the three-card cap.
    const leaving = items.filter((item) => item.leaving && painted.has(item.id));
    painted = new Set([...visible, ...leaving].map((item) => item.id));
    return [
      ...visible.map((item, index) => ({ item, index })),
      ...leaving.map((item) => ({ item, index: 0 })),
    ];
  });
  // Absolute children are positioned from the parent's padding box, so the
  // root safe-area padding does not inset this viewport for us.
  return (
    <If when={active() && entries().length > 0}>
      <View
        testID="sonner-viewport"
        pointerEvents="box-none"
        style={{
          position: "absolute",
          top: 16 + Math.max(insets().top, safeArea().top),
          left: 16 + Math.max(insets().left, safeArea().left),
          right: 16 + Math.max(insets().right, safeArea().right),
          alignItems: "center",
          zIndex: 10000,
        }}
      >
        <View
          pointerEvents="box-none"
          style={{
            width: "100%",
            maxWidth: 420,
            height: center.expanded() ? entries().length * 80 : 100,
          }}
        >
          <Each of={entries()} key={(entry) => entry.item.id}>
            {(entry) => <ToastCard center={center} item={entry().item} index={entry().index} />}
          </Each>
        </View>
      </View>
    </If>
  );
}

function ToastCard(props: { center: NotificationCenter; item: Notification; index: number }) {
  const x = createNativeValue(0),
    y = createNativeValue(0);
  const offset = computed(() => props.index * (props.center.expanded() ? 80 : 14));
  let previousOffset = offset();
  const stackY = createNativeValue(previousOffset);
  effect(() => {
    const next = offset();
    if (next !== previousOffset) {
      previousOffset = next;
      void stackY.timing(next, { duration: 400, easing: "easeOut" });
    }
  });
  const enter = Date.now() - props.item.created < 380;
  const opacity = createNativeValue(enter ? 0 : 1),
    entrance = createNativeValue(enter ? -80 : 0);
  if (enter) {
    void opacity.timing(1, { duration: 380, easing: "easeOut" });
    void entrance.timing(0, { duration: 380, easing: "easeOut" });
  }
  effect(() => {
    if (props.item.leaving) {
      void opacity.timing(0, { duration: 260, easing: "easeOut" });
      void entrance.timing(-80, { duration: 260, easing: "easeOut" });
    }
  });
  cleanup(() => {
    void x.stop();
    void y.stop();
    void stackY.stop();
    void opacity.stop();
    void entrance.stop();
  });
  const vertical = y.add(entrance).add(stackY);
  const front = () => props.index === 0 || props.center.expanded();
  return (
    <View
      testID={`toast-${props.item.id}`}
      role="alert"
      accessibilityLabel={props.item.message}
      pointerEvents={front() && !props.item.leaving ? "auto" : "none"}
      pan={{ x, y }}
      onPanEnd={(event) => {
        const dx = event.translationX,
          dy = event.translationY;
        const stackable = props.center.items().filter((item) => !item.leaving).length > 1;
        const horizontal = Math.abs(dx) >= 72 && Math.abs(dx) >= Math.abs(dy);
        if (
          !horizontal &&
          stackable &&
          ((!props.center.expanded() && dy >= 40) || (props.center.expanded() && dy <= -40))
        )
          props.center.expanded.set(!props.center.expanded());
        else if (Math.abs(dx) >= 72 || dy <= -56) props.center.dismiss(props.item.id);
        void x.spring(0);
        void y.spring(0);
      }}
      style={{
        position: "absolute",
        top: 0,
        left: `${props.center.expanded() ? 0 : props.index * 2.5}%`,
        width: `${props.center.expanded() ? 100 : 100 - props.index * 5}%`,
        minHeight: 52,
        height: front() ? undefined : 72,
        paddingVertical: 12,
        paddingLeft: 12,
        paddingRight: 8,
        borderWidth: 1,
        borderColor: palette().border,
        borderRadius: radius.xl,
        backgroundColor: palette().panel,
        ...surfaceFinish("card"),
        flexDirection: "row",
        alignItems: "center",
        zIndex: props.item.leaving ? 200 : 100 - props.index,
        opacity,
        transform: [{ translateX: x }, { translateY: vertical }],
      }}
    >
      <If when={front()}>
        <View style={{ width: 28, height: 32, justifyContent: "center" }}>
          <Icon name="info" size={16} color={palette().accent} />
        </View>
        <Text
          numberOfLines={2}
          style={{
            flex: 1,
            fontSize: 14,
            lineHeight: 20,
            fontWeight: "600",
            color: palette().text,
          }}
        >
          {props.item.message}
        </Text>
        <View
          testID={`dismiss-toast-${props.item.id}`}
          class="focus-visible:ring-2 focus-visible:ring-ring"
          role="button"
          accessibilityLabel="Dismiss notification"
          onPress={() => props.center.dismiss(props.item.id)}
          style={{
            width: 32,
            height: 32,
            alignItems: "center",
            justifyContent: "center",
            borderRadius: 6,
          }}
        >
          <Icon name="x" size={16} color={palette().muted} />
        </View>
      </If>
    </View>
  );
}

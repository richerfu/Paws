import { cleanup, effect, mount, Text, untracked } from "nsh";
import { NotificationCenter, Notifications, NotificationViewport } from "../src/notifications";

mount(() => {
  const center = new NotificationCenter();
  cleanup(() => center.dispose());
  for (let index = 0; index < 40; index++) center.publish(`Notification ${index}`);
  if (center.items().length !== 32 || center.items()[0].id !== 9)
    throw new Error("Notification queue did not preserve the newest 32 entries");
  const layer = center.openLayer();
  center.closeLayer(layer);
  const alive = center.items().map((item) => item.id);
  effect(() => {
    if (center.items().length === 0)
      untracked(() =>
        console.log("PASS: original Sonner queue is bounded and all entries expire independently"),
      );
  });
  if (new Set(alive).size !== 32) throw new Error("Notification identities were reused");
  return (
    <Notifications.Provider value={center}>
      <Text>Notification lifetime trace</Text>
      <NotificationViewport />
    </Notifications.Provider>
  );
});

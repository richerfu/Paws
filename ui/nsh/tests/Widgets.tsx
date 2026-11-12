import { cleanup, mount, signal, Text, View } from "nsh";
import { Dialog, Field, Select } from "../src/components";
import { NotificationCenter, Notifications, NotificationViewport } from "../src/notifications";

function Widgets() {
  const center = Notifications()!,
    value = signal("one"),
    text = signal(""),
    dialog = signal(false);
  return (
    <View style={{ flex: 1, padding: 24, gap: 16 }}>
      <Select
        label="Short options"
        value={value()}
        options={[
          { value: "one", label: "One" },
          { value: "two", label: "Two" },
        ]}
        onChange={(next) => value.set(next)}
      />
      <Select
        label="Long options"
        value={value()}
        options={Array.from({ length: 12 }, (_, index) => ({
          value: String(index),
          label: `Option ${index}`,
        }))}
        onChange={(next) => value.set(next)}
      />
      <Field
        id="widget-input"
        label="URL"
        value={text()}
        placeholder="https://example.com/profile"
        onChange={(next) => text.set(next)}
      />
      <View testID="notify-one" onPress={() => center.publish("Original Sonner feedback")}>
        <Text>Notify</Text>
      </View>
      <View
        testID="notify-many"
        onPress={() => {
          for (let index = 0; index < 4; index++) center.publish(`Notification ${index}`);
        }}
      >
        <Text>Queue four</Text>
      </View>
      <View testID="show-dialog" onPress={() => dialog.set(true)}>
        <Text>Open dialog</Text>
      </View>
      <If when={dialog()}>
        <Dialog title="Dialog" onClose={() => dialog.set(false)}>
          <View testID="notify-dialog" onPress={() => center.publish("Feedback above the dialog")}>
            <Text>Notify in dialog</Text>
          </View>
        </Dialog>
      </If>
      <NotificationViewport />
    </View>
  );
}

mount(() => {
  const center = new NotificationCenter();
  cleanup(() => center.dispose());
  return (
    <Notifications.Provider value={center}>
      <Widgets />
    </Notifications.Provider>
  );
});

use arkit::prelude::*;
use arkit::shadcn::components::{Sonner, SonnerPosition, SonnerToast, ToastVariant};
use std::collections::VecDeque;

const MAX_NOTIFICATIONS: usize = 32;
const DEFAULT_DURATION_MS: u64 = 2_200;

#[derive(Debug, Clone, PartialEq)]
struct Notification {
    id: u64,
    revision: u32,
    message: String,
    variant: ToastVariant,
    duration_ms: u64,
}

#[derive(Default)]
struct NotificationStore {
    next_id: u64,
    items: VecDeque<Notification>,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) struct NotificationCenter {
    store: Signal<NotificationStore>,
}

impl NotificationCenter {
    pub(crate) fn publish(mut self, message: String) {
        let mut store = self.store.write();
        store.next_id = store
            .next_id
            .checked_add(1)
            .expect("hmeta_ui: notification id space exhausted");
        let id = store.next_id;
        if store.items.len() == MAX_NOTIFICATIONS {
            store.items.pop_front();
        }
        store.items.push_back(Notification {
            id,
            revision: 0,
            message,
            variant: ToastVariant::Info,
            duration_ms: DEFAULT_DURATION_MS,
        });
    }

    fn dismiss(mut self, id: u64) {
        self.store.write().items.retain(|item| item.id != id);
    }

    fn items(self) -> Vec<Notification> {
        self.store.read().items.iter().cloned().collect()
    }
}

pub(crate) fn use_notification_center() -> NotificationCenter {
    NotificationCenter {
        store: use_signal(NotificationStore::default),
    }
}

#[component]
pub(crate) fn NotificationHost(center: NotificationCenter) -> Element {
    let toasts = center
        .items()
        .into_iter()
        .map(|item| {
            SonnerToast::new(item.id, item.message)
                .revision(item.revision)
                .variant(item.variant)
                .duration_ms(item.duration_ms)
        })
        .collect::<Vec<_>>();

    rsx! {
        Sonner {
            toasts,
            position: SonnerPosition::TopCenter,
            visible_toasts: 3,
            rich_colors: true,
            on_dismiss: move |id| center.dismiss(id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_limits_are_intentionally_bounded() {
        assert_eq!(MAX_NOTIFICATIONS, 32);
        assert_eq!(DEFAULT_DURATION_MS, 2_200);
    }
}

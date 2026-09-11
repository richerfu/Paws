use dioxus::prelude::{use_future, use_hook, ReadableExt, Signal, WritableExt};
use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;

struct AbortOnDrop(Option<tokio::task::AbortHandle>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
}

/// Own a subscription future in the current Dioxus scope. Replacing or
/// unmounting the provider drops the future immediately, including its watch
/// receiver and any other cancellation guards it owns.
#[cfg(test)]
pub(crate) fn use_scoped_subscription<F>(make_future: impl FnMut() -> F + 'static)
where
    F: Future<Output = ()> + 'static,
{
    let _subscription = use_future(make_future);
}

/// Run Tokio-dependent subscription work on the application's Tokio runtime,
/// while keeping cancellation owned by the current Dioxus scope. The Dioxus
/// executor only polls the `JoinHandle`, which does not require entering a
/// Tokio reactor on the UI thread.
pub(crate) fn use_tokio_subscription<T, F>(
    tokio: tokio::runtime::Handle,
    mut make_future: impl FnMut(tokio::sync::mpsc::Sender<T>) -> F + 'static,
    on_message: impl FnMut(T) + 'static,
) where
    T: Send + 'static,
    F: Future<Output = ()> + Send + 'static,
{
    let on_message = use_hook(move || Rc::new(RefCell::new(on_message)));
    let _subscription = use_future(move || {
        let on_message = on_message.clone();
        // Backpressure prevents a paused UI from accumulating whole telemetry
        // projections. Runtime watch channels retain the latest revision, so
        // skipped intermediate frames are recovered on the next iteration.
        let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
        let task = tokio.spawn(make_future(sender));
        // Construct the guard before returning the local future. Even if the
        // Dioxus executor never polls it, dropping the scope still aborts the
        // Tokio worker.
        let abort = AbortOnDrop(Some(task.abort_handle()));
        async move {
            let _abort = abort;
            while let Some(message) = receiver.recv().await {
                (on_message.borrow_mut())(message);
            }
            if let Err(error) = task.await {
                if !error.is_cancelled() {
                    eprintln!("root subscription task failed: {error}");
                }
            }
        }
    });
}

/// Publish only structural domain changes. An unchanged projection must not
/// schedule any Dioxus subscriber.
pub(crate) fn replace_if_changed<T: PartialEq + 'static>(signal: &mut Signal<T>, next: T) -> bool {
    if *signal.peek() == next {
        return false;
    }
    signal.set(next);
    true
}

pub(crate) fn update_if_changed<T: Clone + PartialEq + 'static>(
    mut signal: Signal<T>,
    update: impl FnOnce(&mut T),
) -> bool {
    let mut next = signal.peek().clone();
    update(&mut next);
    replace_if_changed(&mut signal, next)
}

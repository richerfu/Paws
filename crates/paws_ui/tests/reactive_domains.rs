use dioxus::dioxus_core::{NoOpMutations, VirtualDom};
use dioxus::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[path = "../src/reactive_signal.rs"]
mod reactive_signal;
use reactive_signal::{
    replace_if_changed, update_if_changed, use_scoped_subscription, use_tokio_subscription,
};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

#[derive(Clone)]
struct Handles {
    yaml: Rc<RefCell<Option<Signal<String>>>>,
}

impl PartialEq for Handles {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.yaml, &other.yaml)
    }
}

#[derive(Clone, PartialEq)]
struct Counters {
    yaml: Rc<Cell<usize>>,
    theme: Rc<Cell<usize>>,
    session: Rc<Cell<usize>>,
    profiles: Rc<Cell<usize>>,
}

#[derive(Clone, PartialEq, Props)]
struct HarnessProps {
    handles: Handles,
    counters: Counters,
}

fn harness(props: HarnessProps) -> Element {
    let yaml = use_signal(|| "initial".to_owned());
    let theme = use_signal(|| "system".to_owned());
    let session = use_signal(|| "connected".to_owned());
    let profiles = use_signal(|| "profile-a".to_owned());
    *props.handles.yaml.borrow_mut() = Some(yaml);
    rsx! {
        Subscriber { value: yaml, count: props.counters.yaml }
        Subscriber { value: theme, count: props.counters.theme }
        Subscriber { value: session, count: props.counters.session }
        Subscriber { value: profiles, count: props.counters.profiles }
    }
}

#[derive(Clone, PartialEq, Props)]
struct SubscriberProps {
    value: Signal<String>,
    count: Rc<Cell<usize>>,
}

#[allow(non_snake_case)]
fn Subscriber(props: SubscriberProps) -> Element {
    props.count.set(props.count.get() + 1);
    let _domain_value = props.value.read();
    rsx! {}
}

#[test]
fn yaml_updates_only_rerun_its_domain_and_same_value_is_quiet() {
    let handles = Handles {
        yaml: Rc::new(RefCell::new(None)),
    };
    let counters = Counters {
        yaml: Rc::new(Cell::new(0)),
        theme: Rc::new(Cell::new(0)),
        session: Rc::new(Cell::new(0)),
        profiles: Rc::new(Cell::new(0)),
    };
    let mut dom = VirtualDom::new_with_props(
        harness,
        HarnessProps {
            handles: handles.clone(),
            counters: counters.clone(),
        },
    );
    dom.rebuild_in_place();
    assert_eq!(
        (
            counters.yaml.get(),
            counters.theme.get(),
            counters.session.get(),
            counters.profiles.get(),
        ),
        (1, 1, 1, 1)
    );

    let mut yaml = handles.yaml.borrow().expect("yaml signal");
    dom.in_runtime(|| assert!(replace_if_changed(&mut yaml, "edited".to_owned())));
    dom.render_immediate(&mut NoOpMutations);
    assert_eq!(
        (
            counters.yaml.get(),
            counters.theme.get(),
            counters.session.get(),
            counters.profiles.get(),
        ),
        (2, 1, 1, 1)
    );

    dom.in_runtime(|| assert!(!update_if_changed(yaml, |value| *value = "edited".to_owned())));
    dom.render_immediate(&mut NoOpMutations);
    assert_eq!(
        (
            counters.yaml.get(),
            counters.theme.get(),
            counters.session.get(),
            counters.profiles.get(),
        ),
        (2, 1, 1, 1)
    );
}

#[derive(Clone)]
struct SubscriptionHarness {
    visible: Rc<RefCell<Option<Signal<bool>>>>,
    drops: Rc<Cell<usize>>,
}

impl PartialEq for SubscriptionHarness {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.visible, &other.visible) && Rc::ptr_eq(&self.drops, &other.drops)
    }
}

#[derive(Clone, PartialEq, Props)]
struct SubscriptionHarnessProps {
    harness: SubscriptionHarness,
}

fn subscription_harness(props: SubscriptionHarnessProps) -> Element {
    let visible = use_signal(|| true);
    *props.harness.visible.borrow_mut() = Some(visible);
    rsx! {
        if visible() {
            ScopedProvider { drops: props.harness.drops }
        }
    }
}

struct SubscriptionDropProbe(Rc<Cell<usize>>);

impl Drop for SubscriptionDropProbe {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

#[derive(Clone, PartialEq, Props)]
struct ScopedProviderProps {
    drops: Rc<Cell<usize>>,
}

#[allow(non_snake_case)]
fn ScopedProvider(props: ScopedProviderProps) -> Element {
    use_scoped_subscription(move || {
        let guard = SubscriptionDropProbe(props.drops.clone());
        async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        }
    });
    rsx! {}
}

#[test]
fn remounting_a_provider_cancels_each_previous_subscription() {
    let harness = SubscriptionHarness {
        visible: Rc::new(RefCell::new(None)),
        drops: Rc::new(Cell::new(0)),
    };
    let mut dom = VirtualDom::new_with_props(
        subscription_harness,
        SubscriptionHarnessProps {
            harness: harness.clone(),
        },
    );
    dom.rebuild_in_place();

    let mut visible = harness
        .visible
        .borrow()
        .expect("provider visibility signal");
    dom.in_runtime(|| visible.set(false));
    dom.render_immediate(&mut NoOpMutations);
    assert_eq!(harness.drops.get(), 1);

    dom.in_runtime(|| visible.set(true));
    dom.render_immediate(&mut NoOpMutations);
    dom.in_runtime(|| visible.set(false));
    dom.render_immediate(&mut NoOpMutations);
    assert_eq!(harness.drops.get(), 2);
}

#[derive(Clone)]
struct TokioRuntimeProp {
    id: usize,
    handle: tokio::runtime::Handle,
}

impl PartialEq for TokioRuntimeProp {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

struct ObservedPendingFuture {
    polled: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
}

impl Future for ObservedPendingFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        self.polled.fetch_add(1, Ordering::SeqCst);
        Poll::Pending
    }
}

impl Drop for ObservedPendingFuture {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Clone)]
struct TokioHarness {
    visible: Rc<RefCell<Option<Signal<bool>>>>,
    runtime: TokioRuntimeProp,
    polled: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
}

impl PartialEq for TokioHarness {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.visible, &other.visible)
    }
}

#[derive(Clone, PartialEq, Props)]
struct TokioHarnessProps {
    harness: TokioHarness,
}

fn tokio_harness(props: TokioHarnessProps) -> Element {
    let visible = use_signal(|| true);
    *props.harness.visible.borrow_mut() = Some(visible);
    rsx! {
        if visible() {
            TokioProvider { harness: props.harness }
        }
    }
}

#[derive(Clone, PartialEq, Props)]
struct TokioProviderProps {
    harness: TokioHarness,
}

#[allow(non_snake_case)]
fn TokioProvider(props: TokioProviderProps) -> Element {
    let polled = props.harness.polled.clone();
    let dropped = props.harness.dropped.clone();
    use_tokio_subscription(
        props.harness.runtime.handle.clone(),
        move |_updates: tokio::sync::mpsc::Sender<()>| ObservedPendingFuture {
            polled: polled.clone(),
            dropped: dropped.clone(),
        },
        |_| {},
    );
    rsx! {}
}

fn drive_tokio(runtime: &tokio::runtime::Runtime) {
    runtime.block_on(async {
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
    });
}

#[test]
fn tokio_provider_abort_guard_works_before_and_after_first_poll() {
    for poll_before_unmount in [false, true] {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Tokio runtime");
        let harness = TokioHarness {
            visible: Rc::new(RefCell::new(None)),
            runtime: TokioRuntimeProp {
                id: usize::from(poll_before_unmount),
                handle: runtime.handle().clone(),
            },
            polled: Arc::new(AtomicUsize::new(0)),
            dropped: Arc::new(AtomicUsize::new(0)),
        };
        let mut dom = VirtualDom::new_with_props(
            tokio_harness,
            TokioHarnessProps {
                harness: harness.clone(),
            },
        );
        dom.rebuild_in_place();
        if poll_before_unmount {
            drive_tokio(&runtime);
            assert!(harness.polled.load(Ordering::SeqCst) > 0);
        }

        let mut visible = harness.visible.borrow().expect("visibility signal");
        dom.in_runtime(|| visible.set(false));
        dom.render_immediate(&mut NoOpMutations);
        drive_tokio(&runtime);

        assert_eq!(harness.dropped.load(Ordering::SeqCst), 1);
        if !poll_before_unmount {
            assert_eq!(harness.polled.load(Ordering::SeqCst), 0);
        }
    }
}

#[derive(Clone)]
struct BackpressureHarness {
    runtime: TokioRuntimeProp,
    completed_sends: Arc<AtomicUsize>,
}

impl PartialEq for BackpressureHarness {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.completed_sends, &other.completed_sends)
    }
}

#[derive(Clone, PartialEq, Props)]
struct BackpressureProps {
    harness: BackpressureHarness,
}

fn backpressure_provider(props: BackpressureProps) -> Element {
    let completed = props.harness.completed_sends.clone();
    use_tokio_subscription(
        props.harness.runtime.handle.clone(),
        move |updates| {
            let completed = completed.clone();
            async move {
                for value in 0..10_u8 {
                    if updates.send(value).await.is_err() {
                        break;
                    }
                    completed.fetch_add(1, Ordering::SeqCst);
                }
            }
        },
        |_value| {},
    );
    rsx! {}
}

#[test]
fn paused_ui_applies_bounded_backpressure_to_projection_workers() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Tokio runtime");
    let completed_sends = Arc::new(AtomicUsize::new(0));
    let mut dom = VirtualDom::new_with_props(
        backpressure_provider,
        BackpressureProps {
            harness: BackpressureHarness {
                runtime: TokioRuntimeProp {
                    id: 2,
                    handle: runtime.handle().clone(),
                },
                completed_sends: completed_sends.clone(),
            },
        },
    );
    dom.rebuild_in_place();
    drive_tokio(&runtime);
    assert_eq!(completed_sends.load(Ordering::SeqCst), 4);
}

#[derive(Clone)]
struct DetachedSignalOwner {
    signal: Rc<RefCell<Option<Signal<bool>>>>,
}

impl PartialEq for DetachedSignalOwner {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.signal, &other.signal)
    }
}

#[derive(Clone, PartialEq, Props)]
struct DetachedSignalOwnerProps {
    owner: DetachedSignalOwner,
}

fn detached_signal_owner(props: DetachedSignalOwnerProps) -> Element {
    let pending = use_signal(|| false);
    *props.owner.signal.borrow_mut() = Some(pending);
    use_context_provider(|| "parent-only context");
    rsx! {}
}

#[derive(Clone, PartialEq, Props)]
struct DetachedRowProps {
    pending: Signal<bool>,
    renders: Rc<Cell<usize>>,
    observed: Rc<Cell<bool>>,
    parent_context_missing: Rc<Cell<bool>>,
}

fn detached_row(props: DetachedRowProps) -> Element {
    props.renders.set(props.renders.get() + 1);
    props.observed.set(*props.pending.read());
    props
        .parent_context_missing
        .set(try_consume_context::<&'static str>().is_none());
    rsx! {}
}

#[test]
fn detached_virtual_row_reads_explicit_signal_without_parent_context() {
    let owner = DetachedSignalOwner {
        signal: Rc::new(RefCell::new(None)),
    };
    let mut parent = VirtualDom::new_with_props(
        detached_signal_owner,
        DetachedSignalOwnerProps {
            owner: owner.clone(),
        },
    );
    parent.rebuild_in_place();
    let mut pending = owner.signal.borrow().expect("owned signal");
    let renders = Rc::new(Cell::new(0));
    let observed = Rc::new(Cell::new(false));
    let parent_context_missing = Rc::new(Cell::new(false));

    // ArkUI virtual rows are mounted as their own VirtualDom. They do not
    // inherit the page's context, so reactive dependencies must be props.
    let mut row = VirtualDom::new_with_props(
        detached_row,
        DetachedRowProps {
            pending,
            renders: renders.clone(),
            observed: observed.clone(),
            parent_context_missing: parent_context_missing.clone(),
        },
    );
    row.rebuild_in_place();
    assert_eq!(renders.get(), 1);
    assert!(!observed.get());
    assert!(parent_context_missing.get());

    parent.in_runtime(|| pending.set(true));
    row.render_immediate(&mut NoOpMutations);
    assert_eq!(renders.get(), 2);
    assert!(observed.get());
}

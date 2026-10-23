#![allow(dead_code, unsafe_op_in_unsafe_fn)]
extern crate self as napi_ohos;
use editor_state::State;
use nsh_core::{
    AutoCapitalize, EventKind, EventPayload, InputConfig, KeyboardType, Rect, ReturnKeyType,
    SubmitBehavior, TextAlign,
};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    sync::atomic::{AtomicU32, Ordering},
};
pub mod bindgen_prelude {
    use serde_json::Value;
    use std::{
        cell::RefCell,
        ffi::{CStr, c_char, c_void},
    };
    pub type Result<T> = std::result::Result<T, String>;
    pub mod sys {
        #![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]
        use super::*;
        pub type napi_env = ();
        pub type napi_value = usize;
        pub struct napi_property_descriptor {
            pub utf8name: *const c_char,
            pub name: *mut c_void,
            pub method: Option<()>,
            pub getter: Option<()>,
            pub setter: Option<()>,
            pub value: napi_value,
            pub attributes: u32,
            pub data: *mut c_void,
        }
        pub mod PropertyAttributes {
            pub const writable: u32 = 1;
            pub const enumerable: u32 = 2;
            pub const configurable: u32 = 4;
        }
    }
    thread_local! { static VALUES:RefCell<Vec<Value>>=const {RefCell::new(Vec::new())}; }
    fn put(v: Value) -> usize {
        VALUES.with(|a| {
            let mut a = a.borrow_mut();
            a.push(v);
            a.len() - 1
        })
    }
    pub fn value(id: usize) -> Value {
        VALUES.with(|a| a.borrow()[id].clone())
    }
    pub trait ToNapiValue: Sized {
        unsafe fn to_napi_value(env: (), value: Self) -> Result<usize>;
    }
    macro_rules! scalar {($($ty:ty),*)=>{$(impl ToNapiValue for $ty {unsafe fn to_napi_value(_:(),v:Self)->Result<usize>{Ok(put(Value::from(v)))}})*};}
    scalar!(f64, u32, bool, String);
    impl<T: ToNapiValue> ToNapiValue for Option<T> {
        unsafe fn to_napi_value(e: (), v: Self) -> Result<usize> {
            match v {
                Some(v) => unsafe { T::to_napi_value(e, v) },
                None => Ok(put(Value::Null)),
            }
        }
    }
    pub fn create_object_with_properties(
        _: (),
        props: &[sys::napi_property_descriptor],
    ) -> Result<usize> {
        let mut map = serde_json::Map::new();
        for p in props {
            // SAFETY: generated property names are static NUL-terminated string literals.
            let name = unsafe { CStr::from_ptr(p.utf8name) }
                .to_str()
                .unwrap()
                .to_owned();
            map.insert(name, value(p.value));
        }
        Ok(put(Value::Object(map)))
    }
    pub struct Object {
        id: usize,
    }
    impl Object {
        pub fn from_raw(_: (), id: usize) -> Self {
            Self { id }
        }
        pub fn set<T: ToNapiValue>(&mut self, key: &str, v: T) -> Result<()> {
            // SAFETY: this test ABI stores values in its own recording arena.
            let v = value(unsafe { T::to_napi_value((), v)? });
            VALUES.with(|a| {
                a.borrow_mut()[self.id]
                    .as_object_mut()
                    .unwrap()
                    .insert(key.to_owned(), v)
            });
            Ok(())
        }
    }
}
#[derive(Debug, PartialEq)]
enum Error {
    InvalidNode(u32),
    Unsupported(&'static str),
}
impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}
type Result<T> = std::result::Result<T, Error>;
fn callback(f: impl FnOnce() -> Result<()>) {
    f().unwrap();
}
fn argb(color: nsh_core::Color) -> u32 {
    color.0.rotate_right(8)
}
#[derive(Clone)]
struct ViewOptions {
    target_id: u32,
}
#[derive(Default)]
struct Host(Cell<bool>);
impl Host {
    fn has_ui_context(&self) -> bool {
        self.0.get()
    }
}
struct NativeAttachment {
    id: u32,
}
impl NativeAttachment {
    fn new(_: &()) -> Result<Self> {
        static IDS: AtomicU32 = AtomicU32::new(1);
        Ok(Self {
            id: IDS.fetch_add(1, Ordering::Relaxed),
        })
    }
    fn id(&self) -> u32 {
        self.id
    }
    fn invalidate(&self) -> Result<()> {
        Ok(())
    }
    fn with_child<T>(
        &self,
        f: impl FnOnce(&ohos_arkui_binding::common::node::ArkUINode) -> T,
    ) -> Result<Option<T>> {
        Ok(Some(f(&ohos_arkui_binding::common::node::ArkUINode)))
    }
}
// Native SDK effect port only. The real Input::with_child guard and promotion
// order are extracted unchanged from the selected platform source.
mod ohos_arkui_binding {
    pub(crate) mod common {
        pub(crate) mod node {
            use crate::*;
            thread_local! {
                pub(crate) static FAIL_BLUR: Cell<bool> = const {Cell::new(false)};
                pub(crate) static FOCUS_CALLS: RefCell<Vec<bool>> = const {RefCell::new(Vec::new())};
            }
            pub(crate) struct ArkUINode;
            impl ArkUINode {
                pub(crate) fn set_focus_status(&self, focus: bool) -> Result<()> {
                    FOCUS_CALLS.with(|calls| calls.borrow_mut().push(focus));
                    if !focus && FAIL_BLUR.with(Cell::get) {
                        return Err(Error::Unsupported("injected native blur failure"));
                    }
                    Ok(())
                }
            }
        }
    }
}
struct EditorRoute {
    id: u32,
}
impl EditorRoute {
    fn new(id: u32, _input: Weak<Input>) -> Self {
        Self { id }
    }
}
enum HitTestMode {
    Default,
}
impl From<HitTestMode> for i32 {
    fn from(_: HitTestMode) -> i32 {
        0
    }
}
mod app {
    pub(crate) mod runtime {
        pub(crate) enum Message {
            Arkui,
        }
        #[derive(Default)]
        pub(crate) struct Dispatcher(pub(crate) std::cell::Cell<usize>);
        impl Dispatcher {
            pub(crate) fn send(&self, _: Message) {
                self.0.set(self.0.get() + 1)
            }
        }
    }
}
mod render {
    pub(crate) mod mount {
        #[derive(Default)]
        pub(crate) struct UiContext {
            pub(crate) host: crate::Host,
            pub(crate) arkui: std::cell::RefCell<crate::Batch>,
            pub(crate) arkui_views: std::cell::RefCell<Vec<crate::NativeAttachment>>,
            pub(crate) dispatcher: std::rc::Weak<crate::app::runtime::Dispatcher>,
        }
    }
}
struct AccessibilityState {
    spec: RefCell<nsh_core::Accessibility>,
}
impl AccessibilityState {
    fn changed(&self, _: &Node) {}
}
#[derive(Default)]
struct Interaction {
    focused: Cell<bool>,
}
impl Interaction {
    fn focused(&self) -> bool {
        self.focused.get()
    }
    fn events(&self, _: &Rc<Node>, _: u64) -> Result<()> {
        Ok(())
    }
    fn mask(&self) -> u64 {
        0
    }
}
struct Node {
    native: (),
    accessibility_state: AccessibilityState,
    interaction: Interaction,
    context: render::mount::UiContext,
    input: RefCell<Option<Rc<Input>>>,
    wrapper: RefCell<Option<Wrapper>>,
    scroll: RefCell<Option<Scroll>>,
    web: RefCell<Option<Media>>,
    video: RefCell<Option<Media>>,
}
impl std::ops::Deref for Node {
    type Target = render::mount::UiContext;
    fn deref(&self) -> &Self::Target {
        &self.context
    }
}
impl Node {
    fn ensure_wrapper(&self) -> Result<()> {
        Ok(())
    }
    fn update_wrapper(&self) -> Result<()> {
        Ok(())
    }
    fn emit(&self, _: EventKind, _: EventPayload) {}
}
struct Wrapper;
impl Wrapper {
    fn invalidate_context(&self) {}
}
struct Scroll {
    overlay: RefCell<Option<Wrapper>>,
}
struct Media;
impl Media {
    fn context(&self, _: bool) -> Result<()> {
        Ok(())
    }
}
struct Mounter {
    root: Rc<Node>,
    slots: Vec<Option<Rc<Node>>>,
    applied: RefCell<Vec<Batch>>,
}
impl std::ops::Deref for Mounter {
    type Target = Node;
    fn deref(&self) -> &Self::Target {
        &self.root
    }
}
mod input {
    pub(crate) mod document {
        pub(crate) mod editor {
            use crate::*;
            thread_local! {pub(crate) static FAIL_NEW: Cell<bool> = const {Cell::new(false)};}
            pub(crate) struct Transfer {
                pub(crate) text: String,
                pub(crate) count: u32,
                pub(crate) anchor: usize,
                pub(crate) caret: usize,
                pub(crate) focused: bool,
            }
            /// Effect recorder only. Admission/target lifecycle are the actual
            /// extracted Input methods, not a second implementation of them.
            pub(crate) struct DocumentInput {
                pub(crate) generation: u32,
                pub(crate) text: RefCell<String>,
                pub(crate) count: Cell<u32>,
                pub(crate) selected: Cell<(u32, u32)>,
                pub(crate) content: Cell<Rect>,
                pub(crate) focused: Cell<bool>,
                pub(crate) closed: Cell<bool>,
                pub(crate) close_fails: Cell<bool>,
                pub(crate) calls: RefCell<Vec<&'static str>>,
            }
            impl DocumentInput {
                pub(crate) fn new(
                    _: Weak<Node>,
                    generation: u32,
                    text: &str,
                    count: u32,
                ) -> Result<Rc<Self>> {
                    if FAIL_NEW.with(Cell::get) {
                        return Err(Error::Unsupported("injected document create failure"));
                    }
                    Ok(Rc::new(Self {
                        generation,
                        text: RefCell::new(text.to_owned()),
                        count: Cell::new(count),
                        selected: Cell::new((0, 0)),
                        content: Cell::default(),
                        focused: Cell::new(false),
                        closed: Cell::new(false),
                        close_fails: Cell::new(false),
                        calls: RefCell::new(vec!["new"]),
                    }))
                }
                pub(crate) fn configure(&self, _: &InputConfig, content: Rect) -> Result<()> {
                    self.calls.borrow_mut().push("configure");
                    self.content.set(content);
                    Ok(())
                }
                pub(crate) fn controlled(&self, text: &str, count: u32) -> Result<()> {
                    self.calls.borrow_mut().push("controlled");
                    if count == self.count.get() {
                        self.text.replace(text.to_owned());
                    }
                    Ok(())
                }
                pub(crate) fn focus(&self, focused: bool) -> Result<()> {
                    self.calls.borrow_mut().push("focus");
                    self.focused.set(focused);
                    Ok(())
                }
                pub(crate) fn context(&self, ready: bool) -> Result<()> {
                    self.calls.borrow_mut().push(if ready {
                        "context-ready"
                    } else {
                        "context-lost"
                    });
                    if !ready {
                        self.focused.set(false);
                    }
                    Ok(())
                }
                pub(crate) fn select(&self, a: u32, b: u32) -> Result<()> {
                    self.calls.borrow_mut().push("select");
                    self.selected.set((a, b));
                    Ok(())
                }
                pub(crate) fn close(&self) -> Result<()> {
                    self.calls.borrow_mut().push("close");
                    if self.close_fails.get() {
                        return Err(Error::Unsupported("injected document close failure"));
                    }
                    self.closed.set(true);
                    Ok(())
                }
                pub(crate) fn transfer(&self) -> Result<Transfer> {
                    Ok(Transfer {
                        text: self.text.borrow().clone(),
                        count: self.count.get(),
                        anchor: self.selected.get().0 as usize,
                        caret: self.selected.get().1 as usize,
                        focused: self.focused.get(),
                    })
                }
            }
        }
    }
    pub(crate) mod secure {
        pub(crate) mod editor {
            use crate::*;
            pub(crate) struct SecureInput {
                pub(crate) calls: Cell<usize>,
            }
            impl SecureInput {
                pub(crate) fn new(_: Weak<Node>, _: u32, _: &str, _: u32) -> Result<Rc<Self>> {
                    Ok(Rc::new(Self {
                        calls: Cell::new(0),
                    }))
                }
                pub(crate) fn configure(
                    &self,
                    _: &InputConfig,
                    _: &str,
                    _: u32,
                    _: Rect,
                ) -> Result<()> {
                    self.calls.set(self.calls.get() + 1);
                    Ok(())
                }
                pub(crate) fn focus(&self, _: bool) -> Result<()> {
                    Ok(())
                }
                pub(crate) fn select(&self, _: u32, _: u32) -> Result<()> {
                    Ok(())
                }
                pub(crate) fn close(&self) -> Result<()> {
                    Ok(())
                }
            }
        }
    }
}
include!(concat!(env!("OUT_DIR"), "/production.rs"));

#[cfg(test)]
mod parity_tests;

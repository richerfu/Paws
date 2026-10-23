#!/usr/bin/env python3
"""Run the production Dialog owner and dismissal callback with native effects doubled.

Native close/dispose effects follow OpenHarmony's official custom_dialog_model.cpp:
https://github.com/openharmony/arkui_ace_engine/blob/master/frameworks/core/components_ng/pattern/dialog/custom_dialog/custom_dialog_model.cpp
The phone's private ArkUI implementation remains a separate runtime acceptance check.
The selected binding source is read-only; executables, test-port adaptations and
source-fenced proof are written to an independent host output directory.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('source', type=Path, help='Prepared NSH source root; read only')
parser.add_argument('--output-dir', type=Path, help='Host-only outputs; never the selected NSH tree')
args, test_args = parser.parse_known_args()
root = args.source.resolve() / 'paws-patches/ohos-arkui-binding'
output = args.output_dir.resolve() if args.output_dir else Path(tempfile.mkdtemp(prefix='paws-dialog-lifecycle-'))
if output == args.source.resolve() or args.source.resolve() in output.parents:
    parser.error('Host output must be outside the selected NSH source tree')
output.mkdir(parents=True, exist_ok=True)
def identity():
    return {path.relative_to(root).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
            for folder in ('src', 'tests') for path in sorted((root / folder).rglob('*.rs'))}
before = identity()
api = (root / 'src/api/dialog.rs').read_text()
start = api.index('unsafe extern "C" fn dialog_dismiss_callback')
callback = api[start:]

harness = r'''
#![allow(dead_code, non_camel_case_types, non_snake_case)]
extern crate self as log;
extern crate self as ohos_arkui_sys;
use std::{cell::{Cell,RefCell}, collections::HashMap, ffi::c_void, rc::Rc};
#[macro_export] macro_rules! error {($($arg:tt)*)=>{$crate::ERRORS.with(|e|e.borrow_mut().push(format!($($arg)*)))};}
thread_local! {static ERRORS:RefCell<Vec<String>>=RefCell::default();}
pub type ArkUI_NativeDialogHandle=*mut c_void;
#[derive(Clone,Copy)] pub struct Alignment;
#[derive(Clone,Copy)] pub enum DismissReason {BackPress}
impl From<u32> for DismissReason {fn from(_:u32)->Self {Self::BackPress}}
#[derive(Debug,PartialEq,Eq)] pub struct ArkUIError(&'static str);
pub type ArkUIResult<T>=Result<T,ArkUIError>;
mod arkui_input_binding {pub enum ArkUIErrorCode {ParamInvalid}}
impl ArkUIError {fn new(_:arkui_input_binding::ArkUIErrorCode,_:&str)->Self {Self("invalid")}}
// Native-node ABI port mirrors the current Rc admission rule; the separate
// upstream node_host suite below executes the actual production Node/registry.
thread_local! {static NODE_DISPOSALS:Cell<usize>=const{Cell::new(0)};}
pub struct ArkUINode {self_weak:std::rc::Weak<RefCell<Self>>,disposed:bool}
impl ArkUINode {
    fn new()->Self {Self{self_weak:Default::default(),disposed:false}}
    fn into_rc(self)->Rc<RefCell<Self>> {Rc::new_cyclic(|weak|{let mut node=self;node.self_weak=weak.clone();RefCell::new(node)})}
    fn bind_rc(node:&Rc<RefCell<Self>>) {if node.borrow().self_weak.strong_count()==0 {node.borrow_mut().self_weak=Rc::downgrade(node);}}
    fn raw(&self)->*mut c_void {if self.disposed {std::ptr::null_mut()}else{std::ptr::dangling_mut()}}
    fn is_owned(&self)->bool {!self.disposed}
    fn dispose(&mut self)->ArkUIResult<()> {
        if self.self_weak.strong_count()>1 {return Err(ArkUIError("retained"));}
        if self.disposed {return Err(ArkUIError("disposed"));}
        self.disposed=true;NODE_DISPOSALS.with(|count|count.set(count.get()+1));Ok(())
    }
}
struct Controller {has_dialog:bool,visible:bool,content:bool,generation:u64,user_data:*mut c_void}
#[derive(Default)] struct Native {
    next:usize,controllers:HashMap<usize,Controller>,closes:usize,disposals:usize,invalid_closes:usize,
    fail_show:bool,fail_close:bool,fail_dispose:bool,fail_remove:bool,fail_register:bool,
    order:Vec<&'static str>,
}
thread_local! {static NATIVE:RefCell<Native>=RefCell::default();}
struct Api;
impl Api {
    fn create(&self)->ArkUIResult<ArkUI_NativeDialogHandle> {NATIVE.with(|n|{let mut n=n.borrow_mut();n.next+=1;
        let id=n.next;n.controllers.insert(id,Controller{has_dialog:false,visible:false,content:false,generation:0,user_data:std::ptr::null_mut()});Ok(id as _)})}
    fn set_content(&self,h:ArkUI_NativeDialogHandle,_:*mut c_void)->ArkUIResult<()> {NATIVE.with(|n|{n.borrow_mut().controllers.get_mut(&(h as usize)).unwrap().content=true;});Ok(())}
    fn remove_content(&self,h:ArkUI_NativeDialogHandle)->ArkUIResult<()> {NATIVE.with(|n|{let mut n=n.borrow_mut();
        if n.fail_remove {return Err(ArkUIError("remove"));}n.order.push("remove");
        n.controllers.get_mut(&(h as usize)).unwrap().content=false;Ok(())})}
    fn register_dismiss(&self,h:ArkUI_NativeDialogHandle,data:*mut c_void)->ArkUIResult<()> {NATIVE.with(|n|{let mut n=n.borrow_mut();
        if n.fail_register {return Err(ArkUIError("register"));}n.controllers.get_mut(&(h as usize)).unwrap().user_data=data;Ok(())})}
    fn show(&self,h:ArkUI_NativeDialogHandle,_:bool)->ArkUIResult<()> {NATIVE.with(|n|{let mut n=n.borrow_mut();
        if n.fail_show {return Err(ArkUIError("show"));}let c=n.controllers.get_mut(&(h as usize)).ok_or(ArkUIError("null"))?;
        c.has_dialog=true;c.visible=true;c.generation+=1;Ok(())})}
    // Official CustomDialog::CloseDialog clears the controller's dialogHandle
    // synchronously; a second call with that controller returns ParamInvalid.
    fn close(&self,h:ArkUI_NativeDialogHandle)->ArkUIResult<()> {NATIVE.with(|n|{let mut n=n.borrow_mut();n.closes+=1;
        if n.fail_close {return Err(ArkUIError("close"));}
        if !n.controllers.get(&(h as usize)).is_some_and(|c|c.has_dialog) {n.invalid_closes+=1;return Err(ArkUIError("ParamInvalid"));}
        n.order.push("close");let c=n.controllers.get_mut(&(h as usize)).unwrap();c.has_dialog=false;c.visible=false;Ok(())})}
    // Controller disposal does not close its latest visible dialog. The owner
    // must not call this to conceal failure to close that native presentation.
    fn dispose(&self,h:ArkUI_NativeDialogHandle)->ArkUIResult<()> {NATIVE.with(|n|{let mut n=n.borrow_mut();
        if n.fail_dispose {return Err(ArkUIError("dispose"));}n.order.push("dispose");n.disposals+=1;
        n.controllers.remove(&(h as usize)).ok_or(ArkUIError("null"))?;Ok(())})}
    fn custom_style(&self,_:ArkUI_NativeDialogHandle,_:bool)->ArkUIResult<()> {Ok(())}
    fn custom_animation(&self,_:ArkUI_NativeDialogHandle,_:bool)->ArkUIResult<()> {Ok(())}
    fn set_modal_mode(&self,_:ArkUI_NativeDialogHandle,_:bool)->ArkUIResult<()> {Ok(())}
    fn set_auto_cancel(&self,_:ArkUI_NativeDialogHandle,_:bool)->ArkUIResult<()> {Ok(())}
    fn set_background_color(&self,_:ArkUI_NativeDialogHandle,_:u32)->ArkUIResult<()> {Ok(())}
    fn set_mask_color(&self,_:ArkUI_NativeDialogHandle,_:u32)->ArkUIResult<()> {Ok(())}
    fn set_content_alignment(&self,_:ArkUI_NativeDialogHandle,_:Alignment,_:f32,_:f32)->ArkUIResult<()> {Ok(())}
    fn set_corner_radius(&self,_:ArkUI_NativeDialogHandle,_:f32,_:f32,_:f32,_:f32)->ArkUIResult<()> {Ok(())}
}
struct ApiSlot;
impl ApiSlot {fn with<T>(&self,f:impl FnOnce(&Api)->T)->T {f(&Api)}}
mod api {pub(crate) static ARK_UI_NATIVE_DIALOG_API_1:crate::ApiSlot=crate::ApiSlot;}
#[path="__DIALOG_PATH__"] mod dialog;
use dialog::{Dialog,DialogDismissData,InnerDialogDismissData};
struct ArkUI_DialogDismissEvent {data:*mut c_void,blocked:Cell<bool>}
unsafe fn OH_ArkUI_DialogDismissEvent_GetUserData(e:*mut ArkUI_DialogDismissEvent)->*mut c_void {(*e).data}
unsafe fn OH_ArkUI_DialogDismissEvent_GetDismissReason(_: *mut ArkUI_DialogDismissEvent)->i32 {0}
unsafe fn OH_ArkUI_DialogDismissEvent_SetShouldBlockDismiss(e:*mut ArkUI_DialogDismissEvent,b:bool) {(*e).blocked.set(b);}
__DISMISS_CALLBACK_BODY__
fn reset() {NATIVE.with(|n|*n.borrow_mut()=Native::default());ERRORS.with(|e|e.borrow_mut().clear());}
fn node()->Rc<RefCell<ArkUINode>> {Rc::new(RefCell::new(ArkUINode::new()))}
fn system_dismiss(h:ArkUI_NativeDialogHandle)->bool {
    let (data,generation)=NATIVE.with(|n|{let n=n.borrow();let c=&n.controllers[&(h as usize)];(c.user_data,c.generation)});
    let mut event=ArkUI_DialogDismissEvent{data,blocked:Cell::new(false)};
    unsafe {dialog_dismiss_callback(&mut event)}
    if !event.blocked.get() {NATIVE.with(|n|{let mut n=n.borrow_mut();
        if let Some(c)=n.controllers.get_mut(&(h as usize)) {if c.generation==generation {c.visible=false;}}
    });}
    event.blocked.get()
}
fn assert_clean() {NATIVE.with(|n|assert_eq!(n.borrow().invalid_closes,0));ERRORS.with(|e|assert!(e.borrow().is_empty(),"{:?}",e.borrow()));}
#[test] fn explicit_modal_close_then_owner_drop_closes_native_once() {
    reset();let d=Dialog::new().unwrap();d.content_node(&node()).unwrap();d.show().unwrap();
    d.close().unwrap();drop(d);
    NATIVE.with(|n|{let n=n.borrow();assert_eq!(n.closes,1);assert_eq!(n.disposals,1);assert_eq!(n.order,vec!["close","remove","dispose"]);});assert_clean();
}
#[test] fn never_shown_and_failed_show_drop_do_not_close_a_missing_presentation() {
    reset();drop(Dialog::new().unwrap());let d=Dialog::new().unwrap();NATIVE.with(|n|n.borrow_mut().fail_show=true);
    assert_eq!(d.show(),Err(ArkUIError("show")));drop(d);NATIVE.with(|n|assert_eq!(n.borrow().closes,0));assert_clean();
}
#[test] fn visible_drop_closes_before_detaching_and_disposing() {
    reset();let d=Dialog::new().unwrap();d.content_node(&node()).unwrap();d.show_with_sub_window().unwrap();drop(d);
    NATIVE.with(|n|assert_eq!(n.borrow().order,vec!["close","remove","dispose"]));assert_clean();
}
#[test] fn repeated_show_is_rejected_but_close_and_reopen_are_supported() {
    reset();let mut d=Dialog::new().unwrap();d.show().unwrap();
    assert!(d.show().is_err());assert!(d.show_with_sub_window().is_err());
    NATIVE.with(|n|assert_eq!(n.borrow().controllers[&(d.raw as usize)].generation,1));
    d.close().unwrap();d.close().unwrap();d.show_with_sub_window().unwrap();d.close().unwrap();
    d.dispose().unwrap();assert!(d.show().is_err());d.close().unwrap();drop(d);
    NATIVE.with(|n|{let n=n.borrow();assert_eq!(n.closes,2);assert_eq!(n.disposals,1);});assert_clean();
}
#[test] fn close_failure_is_retryable_and_persistent_failure_quarantines_owners() {
    reset();let mut d=Dialog::new().unwrap();let child=node();d.content_node(&child).unwrap();d.show().unwrap();
    NATIVE.with(|n|n.borrow_mut().fail_close=true);assert_eq!(d.close(),Err(ArkUIError("close")));
    assert!(d.remove_content().is_err());assert!(d.dispose().is_err());
    assert!(child.borrow_mut().dispose().is_err());NATIVE.with(|n|assert!(n.borrow().order.is_empty()));
    NATIVE.with(|n|n.borrow_mut().fail_close=false);d.close().unwrap();drop(d);assert_clean();
    let d=Dialog::new().unwrap();d.content_node(&child).unwrap();d.show().unwrap();
    NATIVE.with(|n|n.borrow_mut().fail_close=true);let before=NATIVE.with(|n|n.borrow().disposals);drop(d);
    NATIVE.with(|n|assert_eq!(n.borrow().disposals,before));assert!(child.borrow_mut().dispose().is_err());
    ERRORS.with(|e|assert!(e.borrow().iter().any(|m|m.contains("Drop close failed"))));
}
#[test] fn blocked_and_allowed_system_dismissal_retain_native_ownership_until_close() {
    reset();let d=Dialog::new().unwrap();let child=node();d.content_node(&child).unwrap();d.show().unwrap();
    d.dismiss_handler(|_|true).unwrap();assert!(system_dismiss(d.raw));
    d.dismiss_handler(|_|false).unwrap();assert!(!system_dismiss(d.raw));assert!(child.borrow_mut().dispose().is_err());
    assert_eq!(Rc::strong_count(&d.inner_dismiss_data),1);d.close().unwrap();drop(d);assert_clean();
}
#[test] fn system_dismissal_without_a_user_handler_still_retires_the_controller_reference() {
    reset();let d=Dialog::new().unwrap();d.show().unwrap();assert!(!system_dismiss(d.raw));drop(d);
    NATIVE.with(|n|assert_eq!(n.borrow().closes,1));assert_clean();
}
#[test] fn legacy_dismiss_handler_keeps_its_borrowed_payload_alive() {
    fn allow(data:DialogDismissData)->Option<bool> {
        // SAFETY: the production callback retains the payload owner during this call.
        let seen=unsafe {&*data.data.unwrap().cast::<Rc<Cell<usize>>>()};seen.set(seen.get()+1);Some(false)
    }
    reset();let d=Dialog::new().unwrap();let seen=Rc::new(Cell::new(0));
    d.on_will_dismiss_with_data(seen.clone(),allow).unwrap();d.show().unwrap();
    assert_eq!(Rc::strong_count(&seen),2);assert!(!system_dismiss(d.raw));assert_eq!(seen.get(),1);
    d.close().unwrap();drop(d);assert_eq!(Rc::strong_count(&seen),1);assert_clean();
}
#[test] fn reentrant_dismiss_handler_reopen_is_not_marked_closing_by_the_old_event() {
    reset();let d=Rc::new(Dialog::new().unwrap());d.show().unwrap();let weak=Rc::downgrade(&d);
    d.dismiss_handler(move |_|{let d=weak.upgrade().unwrap();d.close().unwrap();d.show().unwrap();false}).unwrap();
    assert!(!system_dismiss(d.raw));NATIVE.with(|n|assert!(n.borrow().controllers[&(d.raw as usize)].visible));
    d.close().unwrap();drop(d);assert_clean();
}
#[test] fn disposal_requires_a_closed_presentation_and_repeated_disposal_is_harmless() {
    reset();let mut d=Dialog::new().unwrap();let child=node();d.content_node(&child).unwrap();d.show().unwrap();
    assert!(d.dispose().is_err());assert!(child.borrow_mut().dispose().is_err());d.close().unwrap();d.dispose().unwrap();d.dispose().unwrap();drop(d);
    NATIVE.with(|n|assert_eq!(n.borrow().disposals,1));assert_clean();
}
#[test] fn failed_content_retirement_or_native_disposal_retains_owners_for_retry() {
    reset();let mut d=Dialog::new().unwrap();let child=node();d.content_node(&child).unwrap();d.show().unwrap();d.close().unwrap();
    NATIVE.with(|n|n.borrow_mut().fail_remove=true);assert!(d.dispose().is_err());assert!(child.borrow_mut().dispose().is_err());
    NATIVE.with(|n|{let mut n=n.borrow_mut();n.fail_remove=false;n.fail_dispose=true;});assert!(d.dispose().is_err());
    NATIVE.with(|n|n.borrow_mut().fail_dispose=false);d.dispose().unwrap();drop(d);assert_clean();
}
#[test] fn failed_initial_callback_registration_disposes_its_new_controller() {
    reset();NATIVE.with(|n|n.borrow_mut().fail_register=true);assert!(Dialog::new().is_err());
    NATIVE.with(|n|{let n=n.borrow();assert_eq!(n.closes,0);assert_eq!(n.disposals,1);assert!(n.controllers.is_empty());});assert_clean();
}
#[test] fn shared_rc_owner_survives_failed_detach_and_releases_only_after_success() {
    reset();let d=Dialog::new().unwrap();let child=node();d.content_node(&child).unwrap();
    assert_eq!(Rc::strong_count(&child),2);assert!(child.borrow_mut().dispose().is_err());
    NATIVE.with(|n|n.borrow_mut().fail_remove=true);assert!(d.remove_content().is_err());
    assert_eq!(Rc::strong_count(&child),2);
    NATIVE.with(|n|n.borrow_mut().fail_remove=false);d.remove_content().unwrap();
    assert_eq!(Rc::strong_count(&child),1);child.borrow_mut().dispose().unwrap();drop(d);assert_clean();
}
#[test] fn transferred_node_disposes_after_detach_without_a_temporary_extra_rc() {
    reset();let before=NODE_DISPOSALS.with(Cell::get);let d=Dialog::new().unwrap();
    d.content(ArkUINode::new()).unwrap();d.show().unwrap();d.close().unwrap();
    d.remove_content().unwrap();assert_eq!(NODE_DISPOSALS.with(Cell::get),before+1);
    d.remove_content().unwrap();drop(d);assert_eq!(NODE_DISPOSALS.with(Cell::get),before+1);assert_clean();
}
'''.replace('__DIALOG_PATH__', str(root / 'src/dialog/mod.rs')).replace('__DISMISS_CALLBACK_BODY__', callback)

path = output / 'dialog-lifecycle.rs'
path.write_text(harness)
results = []
def suite(name, source, flags=()):
    binary = output / name
    compile_log = output / (name + '-compile.log')
    command = ['rustc', '--edition=2021', '--test', str(source), *flags, '-o', str(binary)]
    with compile_log.open('w') as log:
        compiled = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, timeout=90)
    result = {'suite': name, 'compileCommand': command, 'compileExit': compiled.returncode,
              'compileLog': str(compile_log)}
    if compiled.returncode == 0:
        run_log = output / (name + '.log')
        with run_log.open('w') as log:
            ran = subprocess.run([str(binary), '--nocapture', *test_args], stdout=log, stderr=subprocess.STDOUT, timeout=90)
        result.update(runExit=ran.returncode, log=str(run_log))
        match = re.search(r'test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored', run_log.read_text())
        if match:
            result.update(passed=int(match[2]), failed=int(match[3]), ignored=int(match[4]))
    results.append(result)
    print(json.dumps(result), flush=True)

suite('dialog-lifecycle', path)
for name, filename, flags in [('binding-host', 'host.rs', ()),
                               ('binding-host-api13', 'host.rs', ('--cfg', 'feature="api-13"')),
                               ('binding-node', 'node_host.rs', ()),
                               ('binding-root', 'root_host.rs', ())]:
    suite(name, root / 'tests' / filename, flags)

# The Paws overlay adds setMaskColor to the real Dialog. Upstream's older
# native-effect test port needs that method too, without changing frozen input.
source = root / 'tests/dialog_host.rs'
original = source.read_text()
adapted = original
if 'fn set_mask_color(' not in original:
    at = adapted.index('    fn set_background_color(')
    adapted = adapted[:at] + '''    fn set_mask_color(&self, _: ArkUI_NativeDialogHandle, value: u32) -> ArkUIResult<()> {
        MASK_VALUES.lock().unwrap().push(value); Ok(())
    }
''' + adapted[at:]
    adapted += '''\nstatic MASK_VALUES: Mutex<Vec<u32>> = Mutex::new(Vec::new());
#[test] fn mask_color_reaches_the_native_api_port() {
    let _serial=TEST_LOCK.lock().unwrap();MASK_VALUES.lock().unwrap().clear();
    let d=dialog::Dialog::new().unwrap();d.mask_color(0).unwrap();d.mask_color(0x12345678).unwrap();
    assert_eq!(&*MASK_VALUES.lock().unwrap(),&[0,0x12345678]);
}\n'''
adapted = adapted.replace('#[path = "../src/dialog/mod.rs"]', '#[path = ' + json.dumps(str(root / 'src/dialog/mod.rs')) + ']')
copy = output / 'binding-dialog-host.rs'
copy.write_text(adapted)
suite('binding-dialog', copy)
after = identity()
proof = {'source': str(root), 'sourceBefore': before, 'sourceAfter': after,
         'sourceStable': before == after, 'suites': results,
         'dialogHostPortAdaptation': {'originalSha256': hashlib.sha256(original.encode()).hexdigest(),
                                    'stagedSha256': hashlib.sha256(adapted.encode()).hexdigest(),
                                    'scope': 'absolute module path; recording set_mask_color ABI port and forwarding case if missing'},
         'limits': ['host native-effect tests; not phone ArkUI lifecycle acceptance']}
(output / 'summary.json').write_text(json.dumps(proof, indent=2) + '\n')
raise SystemExit(0 if proof['sourceStable'] and all(result.get('runExit') == 0 for result in results) else 1)

#!/usr/bin/env python3
"""Run the binding's production registration and receiver against native effects."""
from pathlib import Path
import subprocess
import sys
import tempfile

source = Path(sys.argv[1]).resolve() / 'paws-patches/ohos-arkui-binding/src/api/node'
node = (source / 'mod.rs').read_text()
receiver = (source / 'receiver.rs').read_text()


def function(text, name):
    start = text.rfind('\n', 0, text.index(f'fn {name}')) + 1
    opening = text.index('{', start)
    depth = 1
    end = opening + 1
    while depth:
        depth += (text[end] == '{') - (text[end] == '}')
        end += 1
    return text[start:end]


registry = node[node.index('pub(super) struct NodeCustomEventCallbackContext'):
                node.index('pub(crate) struct ArkUINativeNodeAPI1')]
methods = '\n'.join(function(node, name) for name in [
    'register_node_custom_event_callback', 'unregister_node_custom_event_callback',
    'clear_node_custom_event_callback', 'clear_node_custom_event_callbacks_for_node',
    'prepare_dispose',
])
dispatch = function(receiver, 'node_custom_event_receiver')
harness = r'''
#![allow(dead_code, non_camel_case_types)]
use std::{cell::{Cell, RefCell}, collections::{HashMap, HashSet}, ffi::c_void, rc::Rc};
mod ohos_arkui_sys {
    pub type ArkUI_NodeHandle = *mut std::ffi::c_void;
    pub type ArkUI_NodeCustomEventType = usize;
    pub type ArkUI_NodeCustomEvent = crate::NodeCustomEvent;
}
use ohos_arkui_sys::{ArkUI_NodeHandle, ArkUI_NodeCustomEvent};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum NodeCustomEventType { OnMeasure=1, OnLayout=2, OnDraw=4, OnForegroundDraw=8,
    OnOverlayDraw=16, OnDrawFront=32, OnDrawBehind=64 }
impl From<NodeCustomEventType> for usize { fn from(value:NodeCustomEventType)->Self {value as usize} }
#[derive(Debug, PartialEq, Eq, Clone, Copy)] enum Failure { Register, Add, Unregister, Remove }
type ArkUIResult<T> = Result<T, Failure>;
#[derive(Clone, Copy)] struct ArkUINode(usize);
impl ArkUINode {fn raw(&self)->ArkUI_NodeHandle {self.0 as ArkUI_NodeHandle}}
#[derive(Clone, Copy)] struct NodeCustomEvent {node:usize, kind:NodeCustomEventType}
impl NodeCustomEvent {
    unsafe fn from_raw(event:*mut Self)->Option<Self> {event.as_ref().copied()}
    unsafe fn node_handle(&self)->Option<ArkUINode> {Some(ArkUINode(self.node))}
    fn event_type(&self)->NodeCustomEventType {self.kind}
}
type Receiver=unsafe extern "C" fn(*mut ArkUI_NodeCustomEvent);
#[derive(Default)] struct Native {
    registered:HashSet<(usize,NodeCustomEventType)>, local:HashSet<usize>, global:Option<Receiver>,
    fail:Option<Failure>, adds:usize, removes:usize, global_registers:usize, external_calls:usize,
}
thread_local! {static NATIVE:RefCell<Native>=RefCell::new(Native::default());}
// ArkUI's HandleCustomEvent invokes its node set first, then its independent
// global receiver. No callback is invoked with this test native-state borrowed.
fn fire(node:usize,kind:NodeCustomEventType) {
    let enabled=NATIVE.with(|n|n.borrow().registered.contains(&(node,kind)));
    if !enabled {return;}
    let mut event=NodeCustomEvent{node,kind};
    if NATIVE.with(|n|n.borrow().local.contains(&node)) {unsafe {api::dispatch(&mut event)}}
    let global=NATIVE.with(|n|n.borrow().global);
    if let Some(global)=global {unsafe {global(&mut event)}}
}
unsafe extern "C" fn external_receiver(_event:*mut ArkUI_NodeCustomEvent) {
    NATIVE.with(|n|n.borrow_mut().external_calls+=1);
}
mod api {
    use super::*;
    REGISTRY
    #[derive(Clone,Copy)] pub(crate) struct ArkUINativeNodeAPI1;
    impl ArkUINativeNodeAPI1 {
        METHODS
        fn register_node_custom_event(&self,node:&ArkUINode,kind:NodeCustomEventType,
            _target:i32,_data:*mut c_void)->ArkUIResult<()> {
            NATIVE.with(|n|{let mut n=n.borrow_mut();
                if n.fail==Some(Failure::Register) {return Err(Failure::Register);}
                n.registered.insert((node.0,kind)); Ok(())})
        }
        fn unregister_node_custom_event(&self,node:&ArkUINode,kind:NodeCustomEventType)->ArkUIResult<()> {
            NATIVE.with(|n|{let mut n=n.borrow_mut();
                if n.fail==Some(Failure::Unregister) {return Err(Failure::Unregister);}
                n.registered.remove(&(node.0,kind)); Ok(())})
        }
        fn add_node_custom_event_receiver(&self,node:&ArkUINode)->ArkUIResult<()> {
            NATIVE.with(|n|{let mut n=n.borrow_mut(); n.adds+=1;
                if n.fail==Some(Failure::Add) {return Err(Failure::Add);}
                n.local.insert(node.0); Ok(())})
        }
        fn remove_node_custom_event_receiver(&self,node:&ArkUINode)->ArkUIResult<()> {
            NATIVE.with(|n|{let mut n=n.borrow_mut(); n.removes+=1;
                if n.fail==Some(Failure::Remove) {return Err(Failure::Remove);}
                n.local.remove(&node.0); Ok(())})
        }
        // The old production high-level method calls this stub, reproducing
        // native double dispatch. The fixed method must leave this slot alone.
        fn register_node_custom_event_receiver(&self)->ArkUIResult<()> {
            NATIVE.with(|n|{let mut n=n.borrow_mut(); n.global_registers+=1;
                n.global=Some(receiver::node_custom_event_receiver);}); Ok(())
        }
    }
    mod receiver {
        use super::*;
        DISPATCH
    }
    pub(crate) unsafe fn dispatch(event:*mut ArkUI_NodeCustomEvent) {
        receiver::node_custom_event_receiver(event);
    }
    pub(crate) fn reset() {NODE_CUSTOM_EVENT_CALLBACKS.with(|c|c.borrow_mut().clear());}
    pub(crate) fn callback_count(node:usize)->usize {
        NODE_CUSTOM_EVENT_CALLBACKS.with(|c|c.borrow().keys().filter(|key|key.0==node).count())
    }
}
use api::ArkUINativeNodeAPI1 as Api;
fn reset() {api::reset(); NATIVE.with(|n|*n.borrow_mut()=Native::default());}
#[test] fn nested_measure_is_not_multiplied_by_global_delivery() {
    reset(); let leaf=Rc::new(Cell::new(0));
    for node in 1..=6 {
        let leaf=leaf.clone();
        Api.register_node_custom_event_callback(&ArkUINode(node),NodeCustomEventType::OnMeasure,0,
            move |_|if node==6 {leaf.set(leaf.get()+1)} else {fire(node+1,NodeCustomEventType::OnMeasure)}).unwrap();
    }
    fire(1,NodeCustomEventType::OnMeasure);
    assert_eq!(leaf.get(),1,"six nested custom nodes must measure the leaf once, not 2^6 times");
    NATIVE.with(|n|assert_eq!(n.borrow().global_registers,0));
}
#[test] fn one_receiver_serves_all_custom_kinds_and_replacement() {
    reset(); let node=ArkUINode(1); let calls=Rc::new(Cell::new(0));
    let kinds=[NodeCustomEventType::OnMeasure,NodeCustomEventType::OnLayout,NodeCustomEventType::OnDraw,
        NodeCustomEventType::OnForegroundDraw,NodeCustomEventType::OnOverlayDraw,
        NodeCustomEventType::OnDrawFront,NodeCustomEventType::OnDrawBehind];
    for kind in kinds {let calls=calls.clone(); Api.register_node_custom_event_callback(&node,kind,0,
        move |_|calls.set(calls.get()+1)).unwrap();}
    for kind in kinds {fire(1,kind);}
    assert_eq!(calls.get(),7); NATIVE.with(|n|assert_eq!(n.borrow().local.len(),1));
    let replacement=Rc::new(Cell::new(0)); let seen=replacement.clone();
    Api.register_node_custom_event_callback(&node,NodeCustomEventType::OnMeasure,0,
        move |_|seen.set(seen.get()+1)).unwrap();
    Api.unregister_node_custom_event_callback(&node,NodeCustomEventType::OnLayout).unwrap();
    fire(1,NodeCustomEventType::OnLayout); fire(1,NodeCustomEventType::OnMeasure); fire(1,NodeCustomEventType::OnDraw);
    assert_eq!(calls.get(),8); assert_eq!(replacement.get(),1);
    assert_eq!(api::callback_count(1),6); NATIVE.with(|n|assert_eq!(n.borrow().local.len(),1));
}
#[test] fn registration_failure_preserves_previous_handlers_and_rolls_back_new_event() {
    reset(); let calls=Rc::new(Cell::new(0)); let seen=calls.clone();
    Api.register_node_custom_event_callback(&ArkUINode(1),NodeCustomEventType::OnMeasure,0,
        move |_|seen.set(seen.get()+1)).unwrap();
    NATIVE.with(|n|n.borrow_mut().fail=Some(Failure::Register));
    assert_eq!(Api.register_node_custom_event_callback(&ArkUINode(1),NodeCustomEventType::OnMeasure,0,
        |_|panic!("failed replacement must not replace its handler")),Err(Failure::Register));
    NATIVE.with(|n|n.borrow_mut().fail=None); fire(1,NodeCustomEventType::OnMeasure); assert_eq!(calls.get(),1);
    NATIVE.with(|n|n.borrow_mut().fail=Some(Failure::Add));
    assert_eq!(Api.register_node_custom_event_callback(&ArkUINode(2),NodeCustomEventType::OnMeasure,0,
        |_|panic!("failed registration must not dispatch")),Err(Failure::Add));
    assert_eq!(api::callback_count(2),0);
    NATIVE.with(|n|{let n=n.borrow(); assert!(!n.registered.contains(&(2,NodeCustomEventType::OnMeasure)));
        assert!(!n.local.contains(&2));});
    NATIVE.with(|n|n.borrow_mut().fail=None); fire(1,NodeCustomEventType::OnMeasure); assert_eq!(calls.get(),2);
}
#[test] fn failed_unregistration_keeps_its_callback() {
    reset(); let calls=Rc::new(Cell::new(0)); let seen=calls.clone();
    Api.register_node_custom_event_callback(&ArkUINode(1),NodeCustomEventType::OnDraw,0,
        move |_|seen.set(seen.get()+1)).unwrap();
    NATIVE.with(|n|n.borrow_mut().fail=Some(Failure::Unregister));
    assert_eq!(Api.unregister_node_custom_event_callback(&ArkUINode(1),NodeCustomEventType::OnDraw),Err(Failure::Unregister));
    fire(1,NodeCustomEventType::OnDraw); assert_eq!(calls.get(),1); assert_eq!(api::callback_count(1),1);
}
#[test] fn disposal_is_node_scoped_even_when_native_removal_fails() {
    reset(); NATIVE.with(|n|n.borrow_mut().global=Some(external_receiver));
    let held=Rc::new(()); let captured=held.clone();
    Api.register_node_custom_event_callback(&ArkUINode(1),NodeCustomEventType::OnMeasure,0,
        move |_|{let _=&captured; panic!("disposed callback must not run");}).unwrap();
    let calls=Rc::new(Cell::new(0)); let seen=calls.clone();
    Api.register_node_custom_event_callback(&ArkUINode(2),NodeCustomEventType::OnMeasure,0,
        move |_|seen.set(seen.get()+1)).unwrap();
    assert_eq!(Rc::strong_count(&held),2);
    NATIVE.with(|n|n.borrow_mut().fail=Some(Failure::Remove)); Api.prepare_dispose(&ArkUINode(1));
    assert_eq!(Rc::strong_count(&held),1); assert_eq!(api::callback_count(1),0);
    fire(1,NodeCustomEventType::OnMeasure); fire(2,NodeCustomEventType::OnMeasure); assert_eq!(calls.get(),1);
    NATIVE.with(|n|{let n=n.borrow(); assert_eq!(n.external_calls,2); assert_eq!(n.global_registers,0);});
    NATIVE.with(|n|n.borrow_mut().fail=None); Api.prepare_dispose(&ArkUINode(2));
    NATIVE.with(|n|assert!(!n.borrow().local.contains(&2)));
    fire(2,NodeCustomEventType::OnMeasure); assert_eq!(calls.get(),1);
}
#[test] fn receiver_allows_self_unregistration_and_releases_after_callback() {
    reset(); let held=Rc::new(()); let captured=held.clone(); let calls=Rc::new(Cell::new(0)); let seen=calls.clone();
    Api.register_node_custom_event_callback(&ArkUINode(1),NodeCustomEventType::OnDraw,0,move |_|{
        Api.unregister_node_custom_event_callback(&ArkUINode(1),NodeCustomEventType::OnDraw).unwrap();
        assert_eq!(Rc::strong_count(&captured),2); seen.set(seen.get()+1);
    }).unwrap();
    fire(1,NodeCustomEventType::OnDraw); fire(1,NodeCustomEventType::OnDraw);
    assert_eq!(calls.get(),1); assert_eq!(Rc::strong_count(&held),1);
}
'''.replace('REGISTRY', registry).replace('METHODS', methods).replace('DISPATCH', dispatch)

with tempfile.TemporaryDirectory(prefix='paws-custom-receiver-') as temp:
    path = Path(temp)
    (path / 'receiver.rs').write_text(harness)
    subprocess.run(['rustc', '--edition=2021', '--test', str(path / 'receiver.rs'),
                    '-o', str(path / 'receiver-tests')], check=True)
    subprocess.run([str(path / 'receiver-tests'), '--nocapture', *sys.argv[2:]], check=True)

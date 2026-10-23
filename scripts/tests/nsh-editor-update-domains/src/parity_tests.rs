use super::*;
use bindgen_prelude::{ToNapiValue, value};
fn fixture() -> (Rc<Node>, Rc<Input>) {
    let node = Rc::new(Node {
        native: (),
        accessibility_state: AccessibilityState {
            spec: RefCell::new(Default::default()),
        },
        interaction: Interaction::default(),
        context: render::mount::UiContext {
            host: Host(Cell::new(true)),
            ..Default::default()
        },
        input: RefCell::new(None),
        wrapper: RefCell::new(None),
        scroll: RefCell::new(None),
        web: RefCell::new(None),
        video: RefCell::new(None),
    });
    let input = Input::new(Rc::downgrade(&node)).unwrap();
    *node.input.borrow_mut() = Some(input.clone());
    (node, input)
}
fn batch(node: &Node) -> Batch {
    std::mem::take(&mut *node.arkui.borrow_mut())
}
fn wire(update: EditorUpdate) -> serde_json::Value {
    // SAFETY: the actual NAPI code generator calls only our recording value/object ABI port.
    value(unsafe { EditorUpdate::to_napi_value((), update).unwrap() })
}
fn value_update(id: u32, text: &str, count: u32) -> EditorUpdate {
    EditorUpdate {
        target_id: id,
        value: Some(EditorValue {
            text: text.into(),
            count,
        }),
        ..Default::default()
    }
}
#[test]
fn initial_target_and_context_replacement_seed_all_domains_and_nonzero_count() {
    let (n, i) = fixture();
    i.state.borrow_mut().changed("native\0😀界".into());
    i.content.set(Rect {
        x: 12.,
        y: 4.,
        width: 296.,
        height: 242.,
    });
    i.update();
    let old = i.target_id().unwrap();
    let first = batch(&n).editors.pop().unwrap();
    assert_eq!(first.value.as_ref().unwrap().count, 1);
    assert_eq!(first.value.as_ref().unwrap().text, "native\0😀界");
    assert!(first.settings.is_some());
    assert_eq!(first.frame.unwrap().width, 296.);
    i.context(false);
    i.state.borrow_mut().changed("newer native".into());
    i.context(true);
    let b = batch(&n);
    assert_ne!(i.target_id(), Some(old));
    assert_eq!(b.remove_editors, [old]);
    let next = b.editors.into_iter().next().unwrap();
    assert!(next.settings.is_some() && next.frame.is_some());
    assert_eq!(next.value.unwrap().count, 2);
}
#[test]
fn config_and_frame_updates_never_serialize_a_document_or_edit_count() {
    let (n, i) = fixture();
    i.value("YAML\0😀界\n".repeat(8000), 0).unwrap();
    let _ = batch(&n);
    i.update();
    let config = wire(batch(&n).editors.pop().unwrap());
    assert!(config.get("settings").is_some());
    assert!(config.get("frame").is_none());
    assert!(config.get("value").is_none());
    assert!(!config.to_string().contains("text"));
    assert!(!config.to_string().contains("count"));
    i.content(Rect {
        x: 0.,
        y: 0.,
        width: 300.,
        height: 250.,
    })
    .unwrap();
    let frame = wire(batch(&n).editors.pop().unwrap());
    assert_eq!(frame.as_object().unwrap().len(), 2);
    assert!(frame.get("value").is_none());
    assert!(frame.get("settings").is_none());
    i.content(Rect {
        x: 10.,
        y: 20.,
        width: 300.,
        height: 250.,
    })
    .unwrap();
    assert!(batch(&n).is_empty());
    println!(
        "metadata payload bytes={} viewport payload bytes={}",
        config.to_string().len(),
        frame.to_string().len()
    );
}
#[test]
fn same_or_stale_value_is_not_queued_but_real_unicode_replacement_is_complete() {
    let (n, i) = fixture();
    i.update();
    let _ = batch(&n);
    let text = "full\0😀界\n".repeat(8000);
    i.value(text.clone(), 0).unwrap();
    let b = batch(&n);
    assert_eq!(b.editors.len(), 1);
    let v = b.editors[0].value.as_ref().unwrap();
    assert_eq!(v.text, text);
    assert_eq!(v.count, 0);
    assert!(b.editors[0].settings.is_none() && b.editors[0].frame.is_none());
    i.value(text, 0).unwrap();
    i.state.borrow_mut().changed("new edit".into());
    i.value("stale".into(), 0).unwrap();
    assert!(batch(&n).is_empty());
}
#[test]
fn normal_merge_preserves_earlier_value_when_later_config_or_frame_arrives() {
    let (n, i) = fixture();
    i.update();
    let mut b = batch(&n);
    let initial = b.editors[0].clone();
    let id = initial.target_id;
    b.editor(value_update(id, "replacement", 0));
    b.editor(EditorUpdate {
        target_id: id,
        frame: Some(EditorFrame {
            width: 400.,
            height: 260.,
        }),
        ..Default::default()
    });
    b.editor(EditorUpdate {
        target_id: id,
        settings: initial.settings,
        ..Default::default()
    });
    assert_eq!(b.editors.len(), 1);
    assert_eq!(b.editors[0].value.as_ref().unwrap().text, "replacement");
    assert_eq!(b.editors[0].frame.as_ref().unwrap().width, 400.);
}
#[test]
fn failed_apply_fills_missing_domains_without_replacing_newer_value_or_frame() {
    let (n, i) = fixture();
    i.update();
    let failed = batch(&n);
    let id = failed.editors[0].target_id;
    let mut pending = Batch::default();
    pending.editor(value_update(id, "new pending", 0));
    pending.editor(EditorUpdate {
        target_id: id,
        frame: Some(EditorFrame {
            width: 500.,
            height: 300.,
        }),
        ..Default::default()
    });
    pending.retry_editors(failed.editors);
    assert!(pending.editors[0].settings.is_some());
    assert_eq!(
        pending.editors[0].value.as_ref().unwrap().text,
        "new pending"
    );
    assert_eq!(pending.editors[0].frame.as_ref().unwrap().width, 500.);
    let mut pending = Batch::default();
    pending.editor(EditorUpdate {
        target_id: id,
        frame: Some(EditorFrame {
            width: 7.,
            height: 8.,
        }),
        ..Default::default()
    });
    pending.retry_editors(vec![value_update(id, "uncommitted full value", 9)]);
    assert_eq!(pending.editors[0].value.as_ref().unwrap().count, 9);
}
#[test]
fn removal_during_failed_apply_cannot_resurrect_old_domains_or_commands() {
    let (n, i) = fixture();
    i.update();
    let failed = batch(&n);
    let id = failed.editors[0].target_id;
    let mut pending = Batch::default();
    pending.editor(value_update(id, "old pending", 0));
    pending.commands.push(EditorCommand {
        target_id: id,
        kind: "selection".into(),
        start: 0,
        end: 1,
    });
    pending.remove_editor(id);
    pending.retry_editors(failed.editors);
    pending.retry_editor_commands(vec![EditorCommand {
        target_id: id,
        kind: "focus".into(),
        start: 0,
        end: 0,
    }]);
    pending.editor(value_update(id, "late", 0));
    assert!(pending.editors.is_empty());
    assert!(pending.commands.is_empty());
    assert_eq!(pending.remove_editors, [id]);
}
#[test]
fn multiline_and_secure_mode_switches_do_not_reuse_an_old_initialization() {
    let (n, i) = fixture();
    i.value("current native\0😀".into(), 0).unwrap();
    i.state.borrow_mut().changed("edited".into());
    let _ = batch(&n);
    let old = i.target_id();
    let mut config = InputConfig::default();
    config.multiline = true;
    i.configure(config.clone()).unwrap();
    let first = batch(&n);
    assert_ne!(i.target_id(), old);
    assert!(first.editors[0].settings.as_ref().unwrap().multiline);
    assert_eq!(first.editors[0].value.as_ref().unwrap().count, 1);
    config.secure = true;
    i.configure(config.clone()).unwrap();
    assert!(i.is_secure());
    assert!(batch(&n).editors.is_empty());
    let calls = i.secure.borrow().as_ref().unwrap().calls.get();
    i.content(Rect {
        x: 0.,
        y: 0.,
        width: 200.,
        height: 100.,
    })
    .unwrap();
    assert!(i.secure.borrow().as_ref().unwrap().calls.get() > calls);
    config.secure = false;
    i.configure(config).unwrap();
    let resumed = batch(&n);
    assert!(!i.is_secure());
    assert_eq!(resumed.editors[0].value.as_ref().unwrap().text, "edited");
    assert_eq!(resumed.editors[0].value.as_ref().unwrap().count, 1);
    assert_eq!(resumed.editors[0].frame.as_ref().unwrap().width, 200.);
}
#[test]
fn closed_or_missing_context_updates_stay_local_until_a_new_live_target() {
    let (n, i) = fixture();
    n.host.0.set(false);
    i.value("offline\0😀".into(), 0).unwrap();
    i.content(Rect {
        x: 1.,
        y: 2.,
        width: 90.,
        height: 60.,
    })
    .unwrap();
    assert!(batch(&n).is_empty());
    n.host.0.set(true);
    i.context(true);
    let initial = batch(&n);
    assert_eq!(
        initial.editors[0].value.as_ref().unwrap().text,
        "offline\0😀"
    );
    i.close().unwrap();
    let _ = batch(&n);
    i.value("after close".into(), 0).unwrap();
    i.content(Rect::default()).unwrap();
    i.update();
    assert!(batch(&n).editors.is_empty());
}

#[test]
fn outer_context_clear_and_ready_rotation_block_retried_old_target_before_apply() {
    let (n, i) = fixture();
    i.state.borrow_mut().changed("nonzero latest\0😀".into());
    i.update();
    let failed = batch(&n);
    let old = failed.editors[0].target_id;
    let m = Mounter {
        root: n.clone(),
        slots: vec![Some(n.clone())],
        applied: RefCell::default(),
    };
    n.host.0.set(false);
    n.arkui
        .borrow_mut()
        .editor(value_update(old, "pending before context destruction", 1));
    n.arkui.borrow_mut().commands.push(EditorCommand {
        target_id: old,
        kind: "selection".into(),
        start: 0,
        end: 1,
    });
    m.native_context(false).unwrap();
    assert!(n.arkui.borrow().editors.is_empty());
    assert!(n.arkui.borrow().commands.is_empty());
    n.arkui.borrow_mut().retry_editors(failed.editors);
    n.arkui
        .borrow_mut()
        .retry_editor_commands(vec![EditorCommand {
            target_id: old,
            kind: "focus".into(),
            start: 0,
            end: 0,
        }]);
    m.flush_arkui().unwrap();
    assert!(
        m.applied.borrow().is_empty(),
        "actual flush guard rejects unavailable UI context"
    );
    n.host.0.set(true);
    m.native_context(true).unwrap();
    let applied = m.applied.borrow();
    assert_eq!(applied.len(), 1);
    let b = &applied[0];
    assert_eq!(b.remove_editors, [old]);
    assert_eq!(b.editors.len(), 1);
    assert_ne!(b.editors[0].target_id, old);
    assert!(b.editors[0].settings.is_some() && b.editors[0].frame.is_some());
    assert_eq!(
        b.editors[0].value.as_ref().unwrap().text,
        "nonzero latest\0😀"
    );
    assert_eq!(b.editors[0].value.as_ref().unwrap().count, 1);
    assert!(b.commands.is_empty());
}

#[cfg(has_document_input)]
mod document_input {
    use super::*;
    fn fixed() -> InputConfig {
        InputConfig {
            multiline: true,
            fixed_viewport: true,
            ..Default::default()
        }
    }
    fn no_full_sdk_value(node: &Node) {
        assert!(batch(node).editors.iter().all(|update| {
            update
                .value
                .as_ref()
                .is_none_or(|value| value.text.len() < 65_536)
        }));
    }

    #[test]
    fn large_document_requires_real_fixed_hint_and_uses_utf8_admission_units() {
        for (multiline, secure, proof, bytes, expected) in [
            (true, false, false, 65_536, false),
            (false, false, true, 65_536, false),
            (true, true, true, 65_536, false),
            (true, false, true, 65_535, false),
            (true, false, true, 65_536, true),
        ] {
            let (_node, input) = fixture();
            input.content.set(Rect {
                x: 0.,
                y: 0.,
                width: 296.,
                height: 242.,
            });
            input
                .configure(InputConfig {
                    multiline,
                    secure,
                    fixed_viewport: proof,
                    ..Default::default()
                })
                .unwrap();
            input.value("x".repeat(bytes), 0).unwrap();
            assert_eq!(input.document.borrow().is_some(), expected);
        }
        let (_node, input) = fixture();
        input.configure(fixed()).unwrap();
        input.value("中".repeat(21_846), 0).unwrap();
        assert!(
            input.document.borrow().is_some(),
            "64KiB admission is UTF8 bytes, not UTF16 count"
        );
    }

    #[test]
    fn accepted_early_large_value_never_crosses_the_old_sdk_full_setter() {
        let (node, input) = fixture();
        input.configure(fixed()).unwrap();
        input.state.borrow_mut().count = 17;
        let source = "x\n".repeat(40_000);
        input.value(source.clone(), 17).unwrap();
        let owner = input.document.borrow().as_ref().unwrap().clone();
        assert_eq!(owner.text.borrow().as_str(), source);
        assert_eq!(owner.count.get(), 17);
        assert_eq!(owner.content.get(), Rect::default());
        no_full_sdk_value(&node);
        input
            .content(Rect {
                x: 0.,
                y: 0.,
                width: 296.,
                height: 242.,
            })
            .unwrap();
        assert_eq!(owner.content.get().width, 296.);
        no_full_sdk_value(&node);
    }

    #[test]
    fn native_echo_style_and_frame_do_not_migrate_a_system_editor_or_its_history() {
        let (node, input) = fixture();
        input.configure(fixed()).unwrap();
        let _ = batch(&node);
        let source = "native\n".repeat(10_000);
        let count = input.state.borrow_mut().changed(source.clone()).unwrap();
        input.value(source.clone(), count).unwrap();
        input.update();
        input
            .content(Rect {
                x: 0.,
                y: 0.,
                width: 296.,
                height: 242.,
            })
            .unwrap();
        input.configure(fixed()).unwrap();
        assert!(input.document.borrow().is_none());
        input
            .value(source + "accepted programmatic change", count)
            .unwrap();
        assert!(input.document.borrow().is_some());
    }

    #[test]
    fn sticky_owner_survives_small_replacement_metadata_and_context_transitions() {
        let (node, input) = fixture();
        input.configure(fixed()).unwrap();
        input.value("x".repeat(70_000), 0).unwrap();
        let owner = input.document.borrow().as_ref().unwrap().clone();
        let target = input.target_id();
        let _ = batch(&node);
        input.value("small".into(), 0).unwrap();
        input.configure(fixed()).unwrap();
        input
            .content(Rect {
                x: 0.,
                y: 0.,
                width: 310.,
                height: 250.,
            })
            .unwrap();
        input.context(false);
        input.context(true);
        assert!(Rc::ptr_eq(
            input.document.borrow().as_ref().unwrap(),
            &owner
        ));
        assert_eq!(input.target_id(), target);
        assert_eq!(owner.text.borrow().as_str(), "small");
        assert!(batch(&node).editors.is_empty());
    }

    #[test]
    fn failed_document_creation_preserves_old_target_and_retries_same_value_without_sdk_fallback() {
        let (node, input) = fixture();
        input.configure(fixed()).unwrap();
        let old = input.target_id();
        let _ = batch(&node);
        let source = "x".repeat(70_000);
        input::document::editor::FAIL_NEW.with(|v| v.set(true));
        let result = input.value(source.clone(), 0);
        input::document::editor::FAIL_NEW.with(|v| v.set(false));
        assert!(result.is_err());
        assert_eq!(input.target_id(), old);
        assert!(input.document.borrow().is_none());
        no_full_sdk_value(&node);
        input.value(source.clone(), 0).unwrap();
        assert!(input.document.borrow().is_some());
        assert_ne!(input.target_id(), old);
        assert_eq!(
            input
                .document
                .borrow()
                .as_ref()
                .unwrap()
                .text
                .borrow()
                .as_str(),
            source
        );
        no_full_sdk_value(&node);
    }

    #[test]
    fn focused_promotion_blurs_the_live_sdk_child_before_replacement_and_retries_failure() {
        use ohos_arkui_binding::common::node::{FAIL_BLUR, FOCUS_CALLS};
        let (node, input) = fixture();
        input.configure(fixed()).unwrap();
        node.interaction.focused.set(true);
        let old = input.target_id();
        let _ = batch(&node);
        FOCUS_CALLS.with(|calls| calls.borrow_mut().clear());
        FAIL_BLUR.with(|failure| failure.set(true));
        let source = "x".repeat(70_000);
        let result = input.value(source.clone(), 0);
        FAIL_BLUR.with(|failure| failure.set(false));
        assert!(result.is_err());
        assert_eq!(input.target_id(), old);
        assert!(input.document.borrow().is_none());
        assert!(input.promotion_pending.get());
        let failed = batch(&node);
        assert!(failed.remove_editors.is_empty());
        assert!(
            failed.commands.is_empty(),
            "queued blur is not a synchronous native blur"
        );
        no_full_sdk_value(&node);
        input.value(source.clone(), 0).unwrap();
        assert_ne!(input.target_id(), old);
        assert_eq!(batch(&node).remove_editors, [old.unwrap()]);
        let owner = input.document.borrow().as_ref().unwrap().clone();
        assert_eq!(owner.text.borrow().as_str(), source);
        assert!(owner.focused.get());
        assert!(!input.promotion_pending.get());
        FOCUS_CALLS.with(|calls| assert_eq!(&*calls.borrow(), &[false, false]));
    }

    #[test]
    fn failed_document_close_retains_the_owned_backend_for_retry() {
        let (node, input) = fixture();
        input.configure(fixed()).unwrap();
        input.value("x".repeat(70_000), 0).unwrap();
        let owner = input.document.borrow().as_ref().unwrap().clone();
        let target = input.target_id();
        let _ = batch(&node);
        owner.close_fails.set(true);
        assert!(input.retire().is_err());
        assert!(Rc::ptr_eq(
            input.document.borrow().as_ref().unwrap(),
            &owner
        ));
        assert_eq!(input.target_id(), target);
        assert!(!owner.closed.get());
        owner.close_fails.set(false);
        input.retire().unwrap();
        assert!(input.document.borrow().is_none());
        assert!(owner.closed.get());
    }

    #[test]
    fn real_input_selection_and_focus_commands_route_to_custom_owner_without_sdk_commands() {
        let (node, input) = fixture();
        input.configure(fixed()).unwrap();
        input.native_selection.set((1, 4));
        input.value("a😀b\n".repeat(13_000), 0).unwrap();
        let owner = input.document.borrow().as_ref().unwrap().clone();
        assert_eq!(
            owner.selected.get(),
            (4, 4),
            "promotion uses the last observed native sorted selection end"
        );
        let _ = batch(&node);
        input.selection(2, 4).unwrap();
        input.focus(true).unwrap();
        assert_eq!(
            owner.selected.get(),
            (1, 4),
            "actual Input UTF16 clamp precedes the backend port"
        );
        assert!(owner.focused.get());
        assert!(batch(&node).commands.is_empty());
    }
}

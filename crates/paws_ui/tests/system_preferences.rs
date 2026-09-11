#[path = "../src/system_preferences.rs"]
mod system_preferences;

#[test]
fn platform_changes_notify_without_telemetry_and_survive_root_replacement() {
    use system_preferences::*;
    let mut receiver = subscribe();
    receiver.borrow_and_update();

    let locale = if current().locale == "en-US" {
        "zh-CN"
    } else {
        "en-US"
    };
    set_locale(locale.to_owned());
    assert!(receiver.has_changed().unwrap());
    assert_eq!(receiver.borrow_and_update().locale, locale);
    set_locale(locale.to_owned());
    assert!(
        !receiver.has_changed().unwrap(),
        "identical events must not invalidate subscribers"
    );

    let next_mode = if current().color_mode == 0 { 1 } else { 0 };
    set_color_mode(next_mode);
    assert!(receiver.has_changed().unwrap());
    assert_eq!(receiver.borrow_and_update().color_mode, next_mode);
    set_color_mode(next_mode);
    assert!(!receiver.has_changed().unwrap());

    drop(receiver);
    let replacement_root = subscribe();
    assert_eq!(*replacement_root.borrow(), current());
    assert_eq!(replacement_root.borrow().locale, locale);
    assert_eq!(replacement_root.borrow().color_mode, next_mode);
}

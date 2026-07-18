#[allow(dead_code)]
#[path = "../src/l10n.rs"]
mod l10n;
#[path = "../src/mode_feedback.rs"]
mod mode_feedback;

use hmeta_model::RuntimeMode;
use l10n::{strings, UiLocale};
use mode_feedback::{mode_changed_message, mode_label};

#[test]
fn mode_label_matches_ui_segments() {
    let zh = strings(UiLocale::ZhCn);
    let en = strings(UiLocale::En);
    assert_eq!(mode_label(RuntimeMode::Rule, zh), "规则");
    assert_eq!(mode_label(RuntimeMode::Global, zh), "全局");
    assert_eq!(mode_label(RuntimeMode::Direct, zh), "直连");
    assert_eq!(mode_label(RuntimeMode::Global, en), "Global");
}

#[test]
fn mode_changed_message_names_the_new_mode() {
    assert_eq!(
        mode_changed_message(RuntimeMode::Global, strings(UiLocale::ZhCn)),
        "运行模式已切换为全局"
    );
    assert_eq!(
        mode_changed_message(RuntimeMode::Global, strings(UiLocale::En)),
        "Mode changed to Global"
    );
}

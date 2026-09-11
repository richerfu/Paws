use super::super::*;
use crate::ui_preferences::{LanguagePreference, ThemePreference};

pub(crate) fn appearance_page() -> Element {
    let services = use_context::<UiServices>();
    let language_services = services.clone();
    let theme_services = services.clone();
    let retry_services = services;
    let current = use_context::<UiStores>().preferences.read().clone();
    let locale = current.locale;

    let system_language = translate_ui(locale, tr::page_tr_117());
    let simplified_chinese = translate_ui(locale, tr::hard_zh_031()).to_owned();
    let english = "English".to_owned();
    let selected_language = match current.language {
        LanguagePreference::System => system_language.clone(),
        LanguagePreference::ZhCn => simplified_chinese.clone(),
        LanguagePreference::En => english.clone(),
    };
    let language_system_option = system_language.clone();
    let language_chinese_option = simplified_chinese.clone();

    let system_theme = translate_ui(locale, tr::page_tr_117());
    let light_theme = translate_ui(locale, tr::page_tr_118());
    let dark_theme = translate_ui(locale, tr::page_tr_119());
    let selected_theme = match current.theme {
        ThemePreference::System => system_theme.clone(),
        ThemePreference::Light => light_theme.clone(),
        ThemePreference::Dark => dark_theme.clone(),
    };
    let theme_system_option = system_theme.clone();
    let theme_light_option = light_theme.clone();

    let body = rsx! {
        column {
            width: "100%",
            {card(
                translate_ui(locale, tr::page_tr_120()),
                Some(translate_ui(locale, tr::page_tr_121())),
                rsx! {
                    RadioGroup {
                        options: vec![system_language, simplified_chinese, english],
                        selected: Some(selected_language),
                        on_select: move |value: String| {
                            let preference = if value == language_system_option {
                                LanguagePreference::System
                            } else if value == language_chinese_option {
                                LanguagePreference::ZhCn
                            } else {
                                LanguagePreference::En
                            };
                            language_services.set_language(preference);
                        }
                    }
                }
            )}
            row { height: 12.0 }
            {card(
                translate_ui(locale, tr::page_tr_122()),
                Some(translate_ui(locale, tr::page_tr_123())),
                rsx! {
                    RadioGroup {
                        options: vec![system_theme, light_theme, dark_theme],
                        selected: Some(selected_theme),
                        on_select: move |value: String| {
                            let preference = if value == theme_system_option {
                                ThemePreference::System
                            } else if value == theme_light_option {
                                ThemePreference::Light
                            } else {
                                ThemePreference::Dark
                            };
                            theme_services.set_theme(preference);
                        }
                    }
                }
            )}
            if let Some(error) = current.preferences_error.clone() {
                row { height: 12.0 }
                {card(
                    translate_ui(locale, tr::appearance_preferences_unavailable()),
                    Some(error),
                    rsx! {
                        text {
                            content: translate_ui(locale, tr::appearance_preferences_unavailable_detail()),
                            font_size: typography::XS,
                            line_height: 18.0,
                            font_color: warning(),
                        }
                    }
                )}
            }
            if let Some(error) = current.color_mode_error.clone() {
                row { height: 12.0 }
                {card(
                    translate_ui(locale, tr::appearance_color_mode_failed()),
                    Some(format!("{}: {error}", translate_ui(locale, tr::appearance_color_mode_failed_detail()))),
                    rsx! {
                        FlatButton {
                            variant: FlatButtonVariant::Outline,
                            onclick: move |_| retry_services.retry_color_mode(),
                            {translate_ui(locale, tr::appearance_retry_color_mode())}
                        }
                    }
                )}
            }
        }
    };

    scaffold(Route::Appearance {}, rsx! {}, body)
}

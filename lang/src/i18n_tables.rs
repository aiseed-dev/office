//! 文言の対訳表の登録簿。**このファイルは ui/gen_lang.py が生成する。**
//! 手で書かない — 言語を足すときは gen_lang.py を回す。

/// 表の揃った言語(ja は鍵そのものなので載らない)
pub const LANGS: &[&str] = &["de", "en", "es", "fr", "id", "it", "ko", "pt", "ru", "tr", "vi", "zh", "zh-tw"];

pub fn table(lang: &str) -> Option<&'static [(&'static str, &'static str)]> {
    match lang {
        "de" => Some(crate::i18n_de::TABLE),
        "en" => Some(crate::i18n_en::TABLE),
        "es" => Some(crate::i18n_es::TABLE),
        "fr" => Some(crate::i18n_fr::TABLE),
        "id" => Some(crate::i18n_id::TABLE),
        "it" => Some(crate::i18n_it::TABLE),
        "ko" => Some(crate::i18n_ko::TABLE),
        "pt" => Some(crate::i18n_pt::TABLE),
        "ru" => Some(crate::i18n_ru::TABLE),
        "tr" => Some(crate::i18n_tr::TABLE),
        "vi" => Some(crate::i18n_vi::TABLE),
        "zh" => Some(crate::i18n_zh::TABLE),
        "zh-tw" => Some(crate::i18n_zh_tw::TABLE),
        _ => None,
    }
}

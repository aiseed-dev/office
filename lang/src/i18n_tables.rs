//! 文言の対訳表の登録簿。**このファイルは ui/gen_lang.py が生成する。**
//! 手で書かない — 言語を足すときは gen_lang.py を回す。

/// 表の揃った言語(ja は鍵そのものなので載らない)
pub const LANGS: &[&str] = &["en"];

pub fn table(lang: &str) -> Option<&'static [(&'static str, &'static str)]> {
    match lang {
        "en" => Some(crate::i18n_en::EN),
        _ => None,
    }
}

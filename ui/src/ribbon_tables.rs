//! リボンの表の登録簿。**このファイルは ui/gen_lang.py が生成する。**
//! 手で書かない — 言語を足すときは gen_lang.py を回す。

use super::ribbon::Tab;

pub fn tabs(lang: &str) -> Option<(&'static [Tab], &'static [Tab])> {
    match lang {
        "en" => Some((crate::ribbon_en::WRITER, crate::ribbon_en::CALC)),
        _ => None,
    }
}

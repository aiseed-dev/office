//! リボンの表の登録簿。**このファイルは ui/gen_lang.py が生成する。**
//! 手で書かない — 言語を足すときは gen_lang.py を回す。

use super::ribbon::Tab;

pub fn tabs(lang: &str) -> Option<(&'static [Tab], &'static [Tab])> {
    match lang {
        "de" => Some((crate::ribbon_de::WRITER, crate::ribbon_de::CALC)),
        "en" => Some((crate::ribbon_en::WRITER, crate::ribbon_en::CALC)),
        "es" => Some((crate::ribbon_es::WRITER, crate::ribbon_es::CALC)),
        "fr" => Some((crate::ribbon_fr::WRITER, crate::ribbon_fr::CALC)),
        "id" => Some((crate::ribbon_id::WRITER, crate::ribbon_id::CALC)),
        "it" => Some((crate::ribbon_it::WRITER, crate::ribbon_it::CALC)),
        "ko" => Some((crate::ribbon_ko::WRITER, crate::ribbon_ko::CALC)),
        "pt" => Some((crate::ribbon_pt::WRITER, crate::ribbon_pt::CALC)),
        "ru" => Some((crate::ribbon_ru::WRITER, crate::ribbon_ru::CALC)),
        "tr" => Some((crate::ribbon_tr::WRITER, crate::ribbon_tr::CALC)),
        "vi" => Some((crate::ribbon_vi::WRITER, crate::ribbon_vi::CALC)),
        "zh" => Some((crate::ribbon_zh::WRITER, crate::ribbon_zh::CALC)),
        "zh-tw" => Some((crate::ribbon_zh_tw::WRITER, crate::ribbon_zh_tw::CALC)),
        _ => None,
    }
}

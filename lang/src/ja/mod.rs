//! 日本語。**辞書では解けない側の言語。**
//!
//! 誤変換は「以外」「意外」のように**どちらも正しい語**として出てくるので、
//! 辞書引きでは何も見つからない。ふりがなも同じで、「後」の読みは
//! 辞書に書けない(前後で あと/のち/うし/ご と変わる)。
//!
//! 実測(青空文庫5作・ルビ14,994箇所): **読みが割れる語が561種**。

pub mod aozora;
pub mod furigana;
pub mod proof;

use crate::{Language, Target};

pub struct Japanese;

pub(crate) const PROOF: &str = "あなたは日本語の校正者です。渡された文章から、誤変換・表記ゆれ・\
送り仮名の誤り・重複表現・助詞の誤りだけを指摘します。\
文体の好みや言い換えの提案はしません。指摘が無ければ空の配列を返します。\
出力は JSON の配列のみ。各要素は {\"found\":\"本文中の該当文字列\",\
\"suggest\":\"直した文字列\",\"why\":\"誤変換|表記ゆれ|送り仮名|重複表現|助詞\"}。\
found は本文にそのまま現れる文字列にしてください。";

impl Language for Japanese {
    fn tag(&self) -> &'static str {
        "ja"
    }

    fn name(&self) -> &'static str {
        "日本語"
    }

    /// **足りない。** 誤変換は辞書に有る語になる
    fn dictionary_suffices(&self) -> bool {
        false
    }

    fn detect(&self, text: &str) -> bool {
        text.chars().any(|c| {
            let u = c as u32;
            (0x3040..=0x30FF).contains(&u) || (0x4E00..=0x9FFF).contains(&u)
        })
    }

    fn proof_prompt(&self) -> Option<&'static str> {
        Some(PROOF)
    }

    fn reading_prompt(&self) -> Option<&'static str> {
        Some(furigana::SYSTEM)
    }

    /// 漢字の連なりを拾う。**どれに振るかは決め打ちしない** — 候補を出して、選ぶのは人。
    fn reading_targets(&self, text: &str) -> Vec<Target> {
        let is_kanji = |c: char| {
            let u = c as u32;
            (0x4E00..=0x9FFF).contains(&u) || u == 0x3005
        };
        let mut out = Vec::new();
        let mut cur = String::new();
        let mut start = 0usize;
        for (i, ch) in text.chars().enumerate() {
            if is_kanji(ch) {
                if cur.is_empty() {
                    start = i;
                }
                cur.push(ch);
            } else if !cur.is_empty() {
                out.push(Target { base: std::mem::take(&mut cur), at: start });
            }
        }
        if !cur.is_empty() {
            out.push(Target { base: cur, at: start });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 仮名か漢字があれば日本語() {
        assert!(Japanese.detect("これは"));
        assert!(Japanese.detect("漢字"));
        assert!(Japanese.detect("Radeon で動く"));
        assert!(!Japanese.detect("This is English."));
    }

    #[test]
    fn 漢字の連なりを拾える() {
        let t = Japanese.reading_targets("その後、日本語を読む");
        let bases: Vec<&str> = t.iter().map(|x| x.base.as_str()).collect();
        assert_eq!(bases, vec!["後", "日本語", "読"], "{bases:?}");
        assert_eq!(t[0].at, 2, "位置がずれている");
    }

    #[test]
    fn 漢字が無ければ対象も無い() {
        assert!(Japanese.reading_targets("ひらがなだけ").is_empty());
    }
}

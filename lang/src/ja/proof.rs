//! 校正(レビュー > 校正)。
//!
//! 日本語の「スペルチェック」は綴りの話ではない。実際に起きるのは
//! **誤変換**(以外/意外)、**表記ゆれ**(問合せ/問い合わせ)、
//! **送り仮名**、**重複表現**。辞書式のスペルチェッカでは検出できない。
//! だから校正の中身にはモデルを使う。
//!
//! これは「AI機能」ではない。リボンに AI タブは作らない —
//! 校正という普通の機能の、中身の話である。
//!
//! 動いていなければ校正は使えないと出す。**黙って何も指摘しないのが一番悪い**
//! (利用者は「誤りが無い」と受け取ってしまう)。

use crate::model::{self, Endpoint};

/// 指摘1件。
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    /// 対象の文字列(本文中の該当箇所)
    pub found: String,
    /// 直し方の案
    pub suggest: String,
    /// なぜ(誤変換・表記ゆれ 等)
    pub why: String,
}


/// 本文を校正する。モデルに繋がらなければ Err(理由)。
pub fn proofread(ep: &Endpoint, text: &str) -> Result<Vec<Note>, String> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let reply = model::chat(ep, super::PROOF, text, 0.0)?;
    Ok(parse_notes(&reply.content, text))
}

/// モデルが返した JSON を指摘に直す。
///
/// **本文に実在しない `found` は捨てる。** モデルが作り話をしても、
/// 本文と突き合わせて残らない — 検査できないものを画面に出さないため。
pub fn parse_notes(content: &str, text: &str) -> Vec<Note> {
    let mut out = Vec::new();
    for obj in model::objects(content) {
        let (Some(f), Some(s)) = (model::field(obj, "found"), model::field(obj, "suggest")) else {
            continue;
        };
        if !f.is_empty() && text.contains(&f) && f != s {
            out.push(Note {
                found: f,
                suggest: s,
                why: model::field(obj, "why").unwrap_or_default(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 指摘を読める() {
        let text = "それ以外な結果でした。問合せは問い合わせ窓口へ。";
        let content = r#"[
          {"found":"以外","suggest":"意外","why":"誤変換"},
          {"found":"問合せ","suggest":"問い合わせ","why":"表記ゆれ"}
        ]"#;
        let n = parse_notes(content, text);
        assert_eq!(n.len(), 2);
        assert_eq!(n[0], Note { found: "以外".into(), suggest: "意外".into(), why: "誤変換".into() });
        assert_eq!(n[1].suggest, "問い合わせ");
    }

    #[test]
    fn 本文に無い指摘は捨てる() {
        let content = r#"[{"found":"存在しない語","suggest":"直し","why":"誤変換"}]"#;
        assert!(parse_notes(content, "実際の本文です。").is_empty(), "本文に無い指摘を通した");
    }

    #[test]
    fn 直しが同じものは指摘にしない() {
        let content = r#"[{"found":"日本","suggest":"日本","why":"誤変換"}]"#;
        assert!(parse_notes(content, "日本フネン").is_empty());
    }

    #[test]
    fn 指摘なしは空で返る() {
        assert!(parse_notes("[]", "正しい文章です。").is_empty());
    }

    #[test]
    fn 壊れた応答でも落ちない() {
        for c in ["", "{", "[{\"found\":", "ぐちゃぐちゃ", "null"] {
            let _ = parse_notes(c, "本文");
        }
    }

    #[test]
    fn 空の本文は問い合わせない() {
        // モデルが無くてもエラーにならない(そもそも聞きに行かない)
        let ep = Endpoint { port: 1, ..Default::default() };
        assert_eq!(proofread(&ep, "   ").unwrap().len(), 0);
    }

    #[test]
    fn 繋がらなければ理由を返す() {
        // 使えないときは「指摘なし」ではなく、使えないと言う
        let ep = Endpoint { port: 1, ..Default::default() };
        let r = proofread(&ep, "それ以外な結果でした。");
        assert!(r.is_err(), "繋がらないのに成功を返した(誤りが無いと誤解される)");
        assert!(r.unwrap_err().contains("繋がりません"));
    }
}

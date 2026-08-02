//! 校正の入口 — **1つの窓口で、言語ごとに正しい道具へ振り分ける。**
//!
//! hunspell が英語にしたことを日本語でやる、というのがこのソフトの存在理由。
//! だから校正は「うちのワープロの機能」ではなく、**単体で使える道具**として作る
//! (`office-spell` コマンド)。誰でも同梱できて、金を払わず、手元で動く。
//!
//! 振り分け:
//!
//! | 本文 | 誤りの正体 | 道具 | GPU |
//! |---|---|---|---|
//! | 英語 | 綴り誤りは**辞書に無い語**になる (`recieve`) | 辞書 | 要らない |
//! | 日本語 | 誤変換は**辞書に有る語**になる (以外/意外) | モデル | 要る |
//!
//! 混在した文書は**両方**を掛ける(日本語の文書に英単語が混ざるのは普通)。
//!
//! **一番やってはいけないのは、動かなかったときに黙って「指摘なし」と出すこと。**
//! 利用者は「誤りが無い」と受け取る。だから何を検査できて何を検査できなかったかを
//! 必ず持ち帰る(`Report::skipped`)。

use crate::ja::furigana::{self, Suggestion};
use crate::Target;
use crate::model::Endpoint;
use crate::ja::proof;
use crate::spell::{self, Dictionary};

/// 指摘の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// 英語の綴り誤り
    Spelling,
    /// 辞書に無いが、固有名詞か誤りか判定できなかった語
    UnknownWord,
    /// 誤変換 — 辞書に有る語なので辞書では捕まらない
    Conversion,
    /// 表記ゆれ
    Notation,
    /// 送り仮名
    Okurigana,
    /// 重複表現・助詞など
    Wording,
    /// ふりがな(誤りではなく、付ける候補)
    Reading,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Spelling => "綴り",
            Kind::UnknownWord => "未知語",
            Kind::Conversion => "誤変換",
            Kind::Notation => "表記ゆれ",
            Kind::Okurigana => "送り仮名",
            Kind::Wording => "言い回し",
            Kind::Reading => "ふりがな",
        }
    }

    fn from_why(why: &str) -> Kind {
        if why.contains("誤変換") {
            Kind::Conversion
        } else if why.contains("表記") {
            Kind::Notation
        } else if why.contains("送り") {
            Kind::Okurigana
        } else {
            Kind::Wording
        }
    }
}

/// どちらの道具が出したか。**辞書で出たものは GPU 無しで再現できる。**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Dictionary,
    Model,
}

/// 指摘1件。**`candidates` は順序が意味を持つ。断定はしない。**
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub kind: Kind,
    pub source: Source,
    /// 本文中の該当文字列
    pub found: String,
    /// 本文における文字位置(分からなければ None)
    pub at: Option<usize>,
    /// 直し方・読み方の候補(高い順)
    pub candidates: Vec<String>,
}

/// 検査の結果。**何を検査できなかったかを必ず持つ。**
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub findings: Vec<Finding>,
    /// 検査できなかったもの(理由つき)。空でなければ「指摘なし」と言ってはいけない
    pub skipped: Vec<String>,
}

impl Report {
    /// 「指摘はありません」と言ってよいか。
    ///
    /// **検査できなかった部分があるときは言ってはいけない。**
    pub fn may_say_clean(&self) -> bool {
        self.findings.is_empty() && self.skipped.is_empty()
    }

    /// 画面や端末に出す一行。
    pub fn summary(&self) -> String {
        match (self.findings.len(), self.skipped.len()) {
            (0, 0) => "指摘はありません".into(),
            (n, 0) => format!("{n} 件の指摘"),
            (0, _) => format!("検査できませんでした — {}", self.skipped.join(" / ")),
            (n, _) => format!("{n} 件の指摘(ただし {})", self.skipped.join(" / ")),
        }
    }
}

/// 校正の道具立て。辞書は起動時に1回だけ読む。
pub struct Checker {
    pub dict: Option<Dictionary>,
    pub dict_problem: Option<String>,
    pub endpoint: Endpoint,
    /// 一度判定した語は覚える。**同じ語を二度モデルに訊かない**
    seen: std::cell::RefCell<std::collections::BTreeMap<String, Verdict>>,
}

impl Default for Checker {
    fn default() -> Self {
        let (dict, dict_problem) = match Dictionary::load_default() {
            Ok(d) => (Some(d), None),
            Err(e) => (None, Some(e)),
        };
        Self { dict, dict_problem, endpoint: Endpoint::default(), seen: Default::default() }
    }
}

impl Checker {
    /// 本文を検査する。**言語で道具を振り分け、混在文には両方を掛ける。**
    pub fn check(&self, text: &str) -> Report {
        let mut r = Report::default();
        if text.trim().is_empty() {
            return r;
        }
        let lang = spell::lang_of(text);

        // --- 英語 ---
        //
        // **辞書は答えではなく絞り込み。** 辞書に無い語が誤りとは限らない
        // (`Bennet` `Bingley` `Radeon` は正しい)。実測では
        //   『高慢と偏見』 123,688語 → 辞書に無い語 312 (0.25%)
        //   技術文書        2,114語 → 辞書に無い語  74 (3.50%)
        // つまり**辞書が99.7%を落としてから、残りだけモデルに訊けばよい**。
        // 一番速い推論は、動かさない推論。
        match (&self.dict, &self.dict_problem) {
            (Some(d), _) => {
                let unknown = d.check(text);
                let verdicts = self.classify_unknown(text, &unknown, &mut r);
                for m in unknown {
                    match verdicts.get(m.word.as_str()) {
                        // 固有名詞と判定された。誤りではない
                        Some(Verdict::Name) => {}
                        Some(Verdict::Typo) => r.findings.push(Finding {
                            kind: Kind::Spelling,
                            source: Source::Model,
                            found: m.word,
                            at: Some(m.at),
                            candidates: m.suggestions,
                        }),
                        // 判定できなかった。**誤りと断定しない**
                        None => r.findings.push(Finding {
                            kind: Kind::UnknownWord,
                            source: Source::Dictionary,
                            found: m.word,
                            at: Some(m.at),
                            candidates: m.suggestions,
                        }),
                    }
                }
            }
            // 辞書が無いのに黙って通さない
            (None, Some(e)) => r.skipped.push(format!("英語の綴り({e})")),
            (None, None) => {}
        }

        // --- 日本語: モデル。辞書では原理的に捕まらない ---
        if lang == spell::Lang::Japanese {
            match proof::proofread(&self.endpoint, text) {
                Ok(notes) => {
                    for n in notes {
                        let at = char_index_of(text, &n.found);
                        r.findings.push(Finding {
                            kind: Kind::from_why(&n.why),
                            source: Source::Model,
                            found: n.found,
                            at,
                            candidates: vec![n.suggest],
                        });
                    }
                }
                Err(e) => r.skipped.push(format!("日本語の校正({e})")),
            }
        }

        r.findings.sort_by_key(|f| f.at.unwrap_or(usize::MAX));
        r
    }

    /// ふりがなの候補を出す。**誤りの指摘ではなく、付ける候補。**
    pub fn readings(&self, text: &str, targets: &[Target]) -> Result<Vec<Suggestion>, String> {
        furigana::candidates(&self.endpoint, text, targets).map(|(s, _)| s)
    }
}

/// 辞書に無い語の正体。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// 固有名詞・商標・地名など。誤りではない
    Name,
    /// 綴り誤り
    Typo,
}

const NAME_SYSTEM: &str = "英語の文章から、辞書に載っていない語を抜き出しました。\
それぞれが「固有名詞(人名・地名・会社名・製品名・商標・専門用語)」なのか\
「綴り誤り」なのかを、本文を読んで判定してください。\
迷ったら name にしてください(正しい語を誤りだと言う方が害が大きい)。\
出力は JSON の配列のみ。各要素は {\"word\":\"語\",\"verdict\":\"name\" または \"typo\"}。\
渡された語だけを判定し、語を増やさないでください。";

impl Checker {
    /// 辞書に無い語が、固有名詞か誤りかをモデルに訊く。
    ///
    /// **これは「AI機能」ではない。** 辞書が原理的に答えられない問いを、
    /// 答えられる道具に回しているだけ。
    /// モデルが居なければ空を返す — 呼び出し側は**誤りと断定しない**。
    fn classify_unknown(
        &self,
        text: &str,
        unknown: &[spell::Misspelling],
        report: &mut Report,
    ) -> std::collections::BTreeMap<String, Verdict> {
        use std::collections::BTreeMap;
        let mut out: BTreeMap<String, Verdict> = BTreeMap::new();
        if unknown.is_empty() {
            return out;
        }
        // 一度判定した語は覚えている(同じ文書で何度も訊かない)
        let mut ask: Vec<&str> = Vec::new();
        {
            let seen = self.seen.borrow();
            for m in unknown {
                if let Some(v) = seen.get(&m.word) {
                    out.insert(m.word.clone(), *v);
                } else if !ask.contains(&m.word.as_str()) {
                    ask.push(&m.word);
                }
            }
        }
        if ask.is_empty() {
            return out;
        }
        let user = format!("本文:\n{}\n\n判定する語: {}", snippet(text, 4000), ask.join(", "));
        match crate::model::chat(&self.endpoint, NAME_SYSTEM, &user, 0.0) {
            Ok(reply) => {
                let mut seen = self.seen.borrow_mut();
                for obj in crate::model::objects(&reply.content) {
                    let (Some(w), Some(v)) =
                        (crate::model::field(obj, "word"), crate::model::field(obj, "verdict"))
                    else {
                        continue;
                    };
                    // 訊いていない語は通さない
                    if !ask.contains(&w.as_str()) {
                        continue;
                    }
                    let verdict = match v.trim().to_lowercase().as_str() {
                        "name" => Verdict::Name,
                        "typo" => Verdict::Typo,
                        _ => continue,
                    };
                    seen.insert(w.clone(), verdict);
                    out.insert(w, verdict);
                }
            }
            Err(e) => {
                // 判定できないことを黙らない。呼び出し側は「未知語」として出す
                report.skipped.push(format!("固有名詞の判定({e})"));
            }
        }
        out
    }
}

/// 長すぎる本文は頭を渡す(判定には全文は要らない)。
fn snippet(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

/// 本文の中の文字位置(バイトではなく文字)。
fn char_index_of(text: &str, needle: &str) -> Option<usize> {
    let b = text.find(needle)?;
    Some(text[..b].chars().count())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker_with_dict() -> Checker {
        Checker {
            dict: Some(Dictionary::from_list("the\nreceive\ndocument\nis\nhere\n")),
            dict_problem: None,
            // 繋がらない宛先。モデル側は必ず失敗する
            endpoint: Endpoint { port: 1, ..Default::default() },
            seen: Default::default(),
        }
    }

    #[test]
    fn 辞書は絞り込みまでやる() {
        // 辞書に無い語を見つけるところまでは、モデル無しで動く
        let c = checker_with_dict();
        let r = c.check("the documnt is here");
        assert_eq!(r.findings.len(), 1, "{:?}", r.findings);
        assert_eq!(r.findings[0].found, "documnt");
        assert!(r.findings[0].candidates.contains(&"document".to_string()));
    }

    #[test]
    fn 判定できなければ綴り誤りと断定しない() {
        // 辞書に無い語が誤りとは限らない(Bennet・Radeon は正しい)。
        // モデルが居ないなら「未知語」までしか言わない
        let c = checker_with_dict();
        let r = c.check("the documnt is here");
        assert_eq!(r.findings[0].kind, Kind::UnknownWord, "誤りと断定してしまった");
        assert!(
            r.skipped.iter().any(|s| s.contains("固有名詞")),
            "判定できなかったことを黙っている: {:?}",
            r.skipped
        );
    }

    #[test]
    fn 固有名詞と判定されたら指摘しない() {
        let c = checker_with_dict();
        c.seen.borrow_mut().insert("Bennet".into(), Verdict::Name);
        c.seen.borrow_mut().insert("documnt".into(), Verdict::Typo);
        let r = c.check("Bennet wrote the documnt");
        let words: Vec<&str> = r.findings.iter().map(|f| f.found.as_str()).collect();
        assert!(!words.contains(&"Bennet"), "固有名詞を誤りにした: {words:?}");
        assert!(words.contains(&"documnt"), "本物の誤りを落とした: {words:?}");
        let t = r.findings.iter().find(|f| f.found == "documnt").unwrap();
        assert_eq!(t.kind, Kind::Spelling);
        assert_eq!(t.source, Source::Model, "判定はモデルの仕事");
    }

    #[test]
    fn 覚えた語は二度訊かない() {
        // 判定はモデルを動かす。同じ語で何度も動かさない
        let c = checker_with_dict();
        c.seen.borrow_mut().insert("Bennet".into(), Verdict::Name);
        let r = c.check("Bennet is here Bennet is here");
        assert!(r.findings.is_empty(), "{:?}", r.findings);
        assert!(
            r.skipped.is_empty(),
            "全部覚えていれば問い合わせは要らないはず: {:?}",
            r.skipped
        );
    }

    #[test]
    fn 日本語はモデルが要る() {
        let c = checker_with_dict();
        let r = c.check("それは以外な結果でした。");
        // 繋がらないので検査できない。**黙って「指摘なし」にしない**
        assert!(!r.skipped.is_empty(), "モデルが無いのに検査できたことにした");
        assert!(!r.may_say_clean(), "指摘なしと言ってはいけない場面で言えてしまう");
        assert!(r.summary().contains("検査できません"), "{}", r.summary());
    }

    #[test]
    fn 混在文は両方を掛ける() {
        let c = checker_with_dict();
        let r = c.check("Radeon の documnt を確認する");
        // 英語側は辞書で出る
        assert!(r.findings.iter().any(|f| f.found == "documnt"), "{:?}", r.findings);
        // 日本語側は検査できなかったと残る
        assert!(r.skipped.iter().any(|s| s.contains("日本語")), "{:?}", r.skipped);
    }

    #[test]
    fn 綺麗な英語なら指摘なしと言ってよい() {
        let c = checker_with_dict();
        let r = c.check("the document is here");
        assert!(r.may_say_clean());
        assert_eq!(r.summary(), "指摘はありません");
    }

    #[test]
    fn 辞書が無ければ黙らない() {
        let c = Checker {
            dict: None,
            dict_problem: Some("辞書が見つかりません".into()),
            endpoint: Endpoint { port: 1, ..Default::default() },
            seen: Default::default(),
        };
        let r = c.check("the documnt");
        assert!(!r.may_say_clean(), "辞書が無いのに「指摘なし」と言えてしまう");
        assert!(r.skipped.iter().any(|s| s.contains("英語")), "{:?}", r.skipped);
    }

    #[test]
    fn 空の本文は何も起こさない() {
        let c = checker_with_dict();
        let r = c.check("   \n  ");
        assert!(r.may_say_clean());
    }

    #[test]
    fn 指摘は本文の順に並ぶ() {
        let c = checker_with_dict();
        let r = c.check("documnt and anothr");
        let ats: Vec<usize> = r.findings.iter().filter_map(|f| f.at).collect();
        assert!(ats.windows(2).all(|w| w[0] <= w[1]), "順に並んでいない: {ats:?}");
    }

    #[test]
    fn 種類の振り分け() {
        assert_eq!(Kind::from_why("誤変換"), Kind::Conversion);
        assert_eq!(Kind::from_why("表記ゆれ"), Kind::Notation);
        assert_eq!(Kind::from_why("送り仮名"), Kind::Okurigana);
        assert_eq!(Kind::from_why("重複表現"), Kind::Wording);
        assert_eq!(Kind::from_why(""), Kind::Wording);
    }

    #[test]
    fn 文字位置は文字で数える() {
        // バイトで数えると日本語で位置がずれる
        assert_eq!(char_index_of("あいう以外です", "以外"), Some(3));
        assert_eq!(char_index_of("本文", "無い語"), None);
    }
}

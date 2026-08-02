//! ふりがな — **当てにいかない。候補を順に出す。**
//!
//! 日本語の読みは辞書では決まらない。実測(青空文庫5作・ルビ14,981箇所)で、
//! **読みが割れる親字の出現が36.9%**。「後」は あと/うし/のち/ご、
//! 「家」は うち/いえ/や/か。前後を読まないと決まらない。
//!
//! だからモデルの仕事は「正しい読みを当てる」ではなく
//! **「possible な読みを、文脈から見て高い順に並べる」**。
//! 1位を仮に振り、違えば次の候補へ送る — **IME の変換候補と同じ形**なので、
//! 日本語を打つ人は操作を覚え直さなくていい。
//!
//! この形は安全側にも効く。**黙って間違いを確定しない。**

use crate::model::{self, Endpoint};

pub use crate::Target;

/// 候補。**順序が意味を持つ** — 先頭が第1候補。
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub base: String,
    pub at: usize,
    pub readings: Vec<String>,
}

impl Suggestion {
    /// 正解が第何候補にあったか(1始まり)。無ければ None。
    pub fn rank_of(&self, answer: &str) -> Option<usize> {
        self.readings.iter().position(|r| r == answer).map(|i| i + 1)
    }
}

pub const SYSTEM: &str = "あなたは日本語のふりがなを付ける校正者です。\
渡された文中の指定された語について、その文脈で**ありうる読み**を、\
可能性の高い順に最大5つ挙げます。断定はしません。順序が答えです。\
読みは必ずひらがなだけで書きます(カタカナ・漢字・記号を混ぜない)。\
送り仮名は含めず、指定された語の部分の読みだけを書きます。\
出力は JSON の配列のみ。各要素は {\"n\":番号,\"base\":\"指定された語\",\
\"readings\":[\"よみ1\",\"よみ2\"]}。\
**同じ語が二度出てきたら、それぞれ別の番号で別々に答えてください**\
(「今日」は場所によって きょう / こんにち と読みが変わります)。説明文は書きません。";

/// ひらがなだけか。**モデルの作り話を落とすための検査。**
pub fn is_kana(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| {
            let u = c as u32;
            (0x3041..=0x3096).contains(&u) || u == 0x309D || u == 0x309E || u == 0x30FC
        })
}

/// 指定した語の読み候補を訊く(採点に使う道)。
///
/// `context` には**必ず前後を含めた本文**を渡すこと。
/// 語だけ渡すと、この仕事の36.9%は原理的に解けない。
pub fn candidates(
    ep: &Endpoint,
    context: &str,
    targets: &[Target],
) -> Result<(Vec<Suggestion>, model::Reply), String> {
    if targets.is_empty() {
        return Ok((Vec::new(), model::Reply::default()));
    }
    // **番号を振る。** 同じ語が二度出るとき、どちらの出現かを伝えられないと
    // 「今日=きょう/こんにち」のような場面で別々に答えられない
    let list = targets
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{}. {}", i + 1, t.base))
        .collect::<Vec<_>>()
        .join("\n");
    let user = format!("本文:\n{context}\n\n読みを知りたい語:\n{list}");
    let reply = model::chat(ep, SYSTEM, &user, 0.0)?;
    Ok((parse_suggestions(&reply.content, targets), reply))
}

/// モデルの返事を候補に直す。
///
/// **通さないもの**:
///   - 訊いていない語(作り話)
///   - ひらがな以外が混ざった読み
///   - 重複
pub fn parse_suggestions(content: &str, targets: &[Target]) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = Vec::new();
    for obj in model::objects(content) {
        let Some(base) = model::field(obj, "base") else { continue };
        // 番号があればそれで対応づける(同じ語の二度目を取り違えないため)。
        // 無ければ語で照合し、まだ埋まっていない位置へ割り当てる
        let by_n = number(obj)
            .and_then(|n| targets.get(n.checked_sub(1)?))
            .filter(|t| t.base == base && !out.iter().any(|s| s.at == t.at));
        let Some(t) = by_n.or_else(|| {
            targets.iter().find(|t| t.base == base && !out.iter().any(|s| s.at == t.at))
        }) else {
            continue;
        };
        let Some(i) = obj.find("\"readings\"") else { continue };
        let mut readings = Vec::new();
        for r in model::string_array(&obj[i..]) {
            if is_kana(&r) && !readings.contains(&r) {
                readings.push(r);
            }
        }
        if !readings.is_empty() {
            out.push(Suggestion { base, at: t.at, readings });
        }
    }
    out.sort_by_key(|s| s.at);
    out
}

/// pywashi(`washi`)のふりがな記法にして返す。
///
/// **候補を出しただけでは紙にならない。** washi は `{漢字|かんじ}` を
/// `<ruby>` に組んで PDF まで持っていける(縦書きでも効く)ので、
/// ここを出口にする。同じ語に違う読みを振れるのも washi 側で確認済み。
///
/// 第1候補を使う。**違えば人が次の候補に送る**という前提なので、
/// ここで悩まない。
pub fn to_washi(text: &str, suggestions: &[Suggestion]) -> String {
    let ch: Vec<char> = text.chars().collect();
    // 位置の大きい方から差し込む(前を書き換えると後ろの位置がずれるため)
    let mut items: Vec<&Suggestion> = suggestions
        .iter()
        .filter(|s| !s.readings.is_empty())
        // 記法を壊す文字を含む語には振らない
        .filter(|s| !s.base.contains(['{', '}', '|']))
        .collect();
    items.sort_by_key(|s| std::cmp::Reverse(s.at));

    let mut out = ch.clone();
    let mut last_start = usize::MAX;
    for s in items {
        let len = s.base.chars().count();
        let end = s.at + len;
        if end > out.len() || end > last_start {
            // 範囲外・重なりは飛ばす(二重にルビを振らない)
            continue;
        }
        let here: String = ch[s.at..end].iter().collect();
        if here != s.base {
            // 位置と語が合わない指摘は使わない
            continue;
        }
        let marked: Vec<char> = format!("{{{}|{}}}", s.base, s.readings[0]).chars().collect();
        out.splice(s.at..end, marked);
        last_start = s.at;
    }
    out.into_iter().collect()
}

/// `"n": 3` のような番号を読む(文字列ではないので `field` では取れない)。
fn number(obj: &str) -> Option<usize> {
    let i = obj.find("\"n\"")?;
    obj[i + 3..]
        .trim_start_matches(|c: char| c == ':' || c.is_whitespace())
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()
}

/// 文書のあいだ中おぼえておく選択。
///
/// **同じ語には同じ読みを使う**のが文書としての正しさ。
/// 一度人が選んだ読みは、以降その語の第1候補に繰り上げる。
#[derive(Debug, Default, Clone)]
pub struct Memory {
    chosen: std::collections::BTreeMap<String, String>,
}

impl Memory {
    pub fn remember(&mut self, base: &str, reading: &str) {
        self.chosen.insert(base.to_string(), reading.to_string());
    }

    pub fn chosen_for(&self, base: &str) -> Option<&str> {
        self.chosen.get(base).map(|s| s.as_str())
    }

    /// おぼえている読みを先頭へ繰り上げる。**候補から消しはしない** —
    /// 同じ語でも読みが変わる場合が日本語にはある(「家」うち/いえ)。
    pub fn apply(&self, s: &mut Suggestion) {
        let Some(want) = self.chosen.get(&s.base) else { return };
        if let Some(i) = s.readings.iter().position(|r| r == want) {
            let r = s.readings.remove(i);
            s.readings.insert(0, r);
        } else {
            s.readings.insert(0, want.clone());
        }
    }
}

/// top-N 命中率。**正解率ではない** — 当てられない語があっても成立する物差し。
#[derive(Debug, Default, Clone, Copy)]
pub struct Hits {
    pub asked: usize,
    pub answered: usize,
    /// ranks[n] = 第(n+1)候補までに正解が入っていた数
    pub ranks: [usize; 5],
}

impl Hits {
    pub fn add(&mut self, rank: Option<usize>) {
        self.asked += 1;
        if let Some(r) = rank {
            self.answered += 1;
            for n in (r - 1).min(4)..5 {
                self.ranks[n] += 1;
            }
        }
    }
    pub fn top(&self, n: usize) -> f64 {
        if self.asked == 0 {
            return 0.0;
        }
        self.ranks[n.clamp(1, 5) - 1] as f64 * 100.0 / self.asked as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(base: &str, at: usize) -> Target {
        Target { base: base.into(), at }
    }

    #[test]
    fn 候補を順序どおりに読める() {
        let ts = [t("後", 3)];
        let c = r#"[{"base":"後","readings":["のち","あと","うし","ご"]}]"#;
        let s = parse_suggestions(c, &ts);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].readings, vec!["のち", "あと", "うし", "ご"]);
        assert_eq!(s[0].at, 3, "位置が引き継がれていない");
    }

    #[test]
    fn 訊いていない語は捨てる() {
        // モデルが勝手に語を増やしても通さない
        let c = r#"[{"base":"猫","readings":["ねこ"]},{"base":"犬","readings":["いぬ"]}]"#;
        let s = parse_suggestions(c, &[t("猫", 0)]);
        assert_eq!(s.len(), 1, "訊いていない「犬」を通した: {s:?}");
    }

    #[test]
    fn ひらがな以外の読みは捨てる() {
        let c = r#"[{"base":"後","readings":["ノチ","のち","after","ご!"]}]"#;
        let s = parse_suggestions(c, &[t("後", 0)]);
        assert_eq!(s[0].readings, vec!["のち"], "仮名でない読みを通した: {:?}", s[0].readings);
    }

    #[test]
    fn 読みが空なら候補にしない() {
        let c = r#"[{"base":"後","readings":["ノチ","XYZ"]}]"#;
        assert!(parse_suggestions(c, &[t("後", 0)]).is_empty());
    }

    #[test]
    fn 同じ語が二箇所あれば別々の位置に付く() {
        let ts = [t("後", 5), t("後", 40)];
        let c = r#"[{"base":"後","readings":["のち"]},{"base":"後","readings":["あと"]}]"#;
        let s = parse_suggestions(c, &ts);
        assert_eq!(s.len(), 2, "{s:?}");
        assert_eq!(s[0].at, 5);
        assert_eq!(s[1].at, 40);
    }

    #[test]
    fn 番号で出現を取り違えない() {
        // 「今日」は場所によって きょう / こんにち。番号が無いと入れ替わる
        let ts = [t("今日", 0), t("今日", 6)];
        let c = r#"[{"n":2,"base":"今日","readings":["こんにち"]},
                    {"n":1,"base":"今日","readings":["きょう"]}]"#;
        let s = parse_suggestions(c, &ts);
        assert_eq!(s.len(), 2, "{s:?}");
        assert_eq!(s[0].at, 0);
        assert_eq!(s[0].readings[0], "きょう", "1番の答えが別の場所に付いた");
        assert_eq!(s[1].at, 6);
        assert_eq!(s[1].readings[0], "こんにち");
    }

    #[test]
    fn 番号が無くても壊れない() {
        // 古い形・番号を落としたモデルでも、語で照合して動く
        let ts = [t("後", 5)];
        let c = r#"[{"base":"後","readings":["のち"]}]"#;
        assert_eq!(parse_suggestions(c, &ts).len(), 1);
    }

    #[test]
    fn 番号が範囲外なら語で照合する() {
        let ts = [t("後", 5)];
        let c = r#"[{"n":99,"base":"後","readings":["のち"]}]"#;
        let s = parse_suggestions(c, &ts);
        assert_eq!(s.len(), 1, "範囲外の番号で落とした");
        assert_eq!(s[0].at, 5);
    }

    #[test]
    fn 重複した読みはまとめる() {
        let c = r#"[{"base":"家","readings":["うち","うち","いえ"]}]"#;
        let s = parse_suggestions(c, &[t("家", 0)]);
        assert_eq!(s[0].readings, vec!["うち", "いえ"]);
    }

    #[test]
    fn 壊れた応答でも落ちない() {
        for c in ["", "{", "[{\"base\":", "ぐちゃぐちゃ", "null", "[]"] {
            let _ = parse_suggestions(c, &[t("後", 0)]);
        }
    }

    #[test]
    fn 正解が第何候補かを出せる() {
        let s = Suggestion {
            base: "後".into(),
            at: 0,
            readings: vec!["のち".into(), "あと".into(), "うし".into()],
        };
        assert_eq!(s.rank_of("のち"), Some(1));
        assert_eq!(s.rank_of("うし"), Some(3));
        assert_eq!(s.rank_of("ご"), None, "無い読みを当たりにした");
    }

    fn sug(base: &str, at: usize, rs: &[&str]) -> Suggestion {
        Suggestion {
            base: base.into(),
            at,
            readings: rs.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn washiの記法にできる() {
        let t = "吾輩は猫である";
        let s = [sug("吾輩", 0, &["わがはい"]), sug("猫", 3, &["ねこ"])];
        assert_eq!(to_washi(t, &s), "{吾輩|わがはい}は{猫|ねこ}である");
    }

    #[test]
    fn 同じ語に違う読みを振れる() {
        // washi 側で別々の <ruby> になることは確認済み
        let t = "今日ではなく今日と読む";
        let s = [sug("今日", 0, &["きょう"]), sug("今日", 6, &["こんにち"])];
        assert_eq!(to_washi(t, &s), "{今日|きょう}ではなく{今日|こんにち}と読む");
    }

    #[test]
    fn 位置が合わない指摘は使わない() {
        // モデルがずれた位置を返しても本文を壊さない
        let t = "吾輩は猫である";
        let s = [sug("犬", 3, &["いぬ"])];
        assert_eq!(to_washi(t, &s), t, "本文を書き換えてしまった");
    }

    #[test]
    fn 重なった指摘は片方だけ振る() {
        let t = "日本語";
        let s = [sug("日本", 0, &["にほん"]), sug("本語", 1, &["ほんご"])];
        let got = to_washi(t, &s);
        assert_eq!(got.matches('{').count(), 1, "二重にルビを振った: {got}");
    }

    #[test]
    fn 記法を壊す語には振らない() {
        let t = "a{b}c";
        let s = [sug("{b}", 1, &["び"])];
        assert_eq!(to_washi(t, &s), t);
    }

    #[test]
    fn 範囲外は無視する() {
        let s = [sug("吾輩", 100, &["わがはい"])];
        assert_eq!(to_washi("短い", &s), "短い");
    }

    #[test]
    fn 候補が空なら振らない() {
        let s = [sug("後", 0, &[])];
        assert_eq!(to_washi("後で", &s), "後で");
    }

    #[test]
    fn 覚えた読みが第1候補に繰り上がる() {
        let mut m = Memory::default();
        m.remember("家", "いえ");
        let mut s = Suggestion {
            base: "家".into(),
            at: 0,
            readings: vec!["うち".into(), "いえ".into()],
        };
        m.apply(&mut s);
        assert_eq!(s.readings[0], "いえ");
        // 消さない — 日本語では同じ語でも読みが変わる
        assert!(s.readings.contains(&"うち".to_string()), "他の候補を消した: {:?}", s.readings);
    }

    #[test]
    fn 覚えた読みが候補に無くても先頭に入る() {
        let mut m = Memory::default();
        m.remember("後", "のち");
        let mut s = Suggestion { base: "後".into(), at: 0, readings: vec!["あと".into()] };
        m.apply(&mut s);
        assert_eq!(s.readings, vec!["のち", "あと"]);
    }

    #[test]
    fn top_n命中率を数えられる() {
        let mut h = Hits::default();
        h.add(Some(1)); // 第1候補で当たり
        h.add(Some(3)); // 第3候補で当たり
        h.add(None); // 候補に無い
        h.add(Some(2));
        assert_eq!(h.asked, 4);
        assert_eq!(h.answered, 3);
        assert!((h.top(1) - 25.0).abs() < 1e-9, "top1={}", h.top(1));
        assert!((h.top(3) - 75.0).abs() < 1e-9, "top3={}", h.top(3));
        assert!((h.top(5) - 75.0).abs() < 1e-9, "top5={}", h.top(5));
    }

    #[test]
    fn 訊いていなければ0で割らない() {
        assert_eq!(Hits::default().top(1), 0.0);
    }

    #[test]
    fn 仮名の判定() {
        assert!(is_kana("わがはい"));
        assert!(is_kana("こーひー"));
        assert!(!is_kana("ワガハイ"), "カタカナを通した");
        assert!(!is_kana("吾輩"));
        assert!(!is_kana(""));
        assert!(!is_kana("のち。"));
    }
}

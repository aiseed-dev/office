//! 青空文庫のテキストを読む。
//!
//! **なぜこれが要るか**: 入力者が人手で付けたルビが、そのまま**正解**になる。
//! ルビを剥いで本文だけをモデルに渡し、候補を出させ、元のルビと突き合わせれば
//! 命中率が機械で採点できる。作品IDを書くだけで**審査員が同じ入力で再現できる**。
//!
//! 17,835作品のうち **著作権フラグ「なし」は17,347**。
//! 残る928作品は著作権が存続しているので、コーパスを作る側で必ず除外する
//! (この判断はテキストからは分からない。目録CSVの `作品著作権フラグ` を見ること)。
//!
//! 記法(底本の凡例より):
//!   `《》` ルビ / `｜` ルビの親文字列の始まり / `［＃…］` 入力者注 / `※［＃…］` 外字

/// ルビ1つ。`at` は**剥がしたあとの本文**での文字位置(バイトではない)。
#[derive(Debug, Clone, PartialEq)]
pub struct Ruby {
    /// 親文字列(ルビが付く側)
    pub base: String,
    /// 入力者が付けた読み = 正解
    pub reading: String,
    /// 剥がしたあとの本文における開始文字位置
    pub at: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Work {
    pub title: String,
    pub author: String,
    /// ルビと注記を取り除いた本文
    pub text: String,
    pub ruby: Vec<Ruby>,
}

/// 文字の種別。ルビの親文字列は「同じ種別の連なり」で決まる。
#[derive(PartialEq, Clone, Copy, Debug)]
enum Kind {
    Kanji,
    Katakana,
    Hiragana,
    Latin,
    Other,
}

fn kind(c: char) -> Kind {
    let u = c as u32;
    match u {
        0x4E00..=0x9FFF | 0x3005 | 0x3006 | 0x3007 | 0xF900..=0xFAFF => Kind::Kanji,
        // ヶ・ヵ は漢字の連なりに混ざる(一ヶ月)
        0x30F5 | 0x30F6 => Kind::Kanji,
        // ゲタ(外字の置き換え)もルビの親になれる
        0x3013 => Kind::Kanji,
        0x30A1..=0x30FA | 0x30FC => Kind::Katakana,
        0x3041..=0x309F => Kind::Hiragana,
        0x0041..=0x005A | 0x0061..=0x007A | 0x0030..=0x0039 => Kind::Latin,
        0xFF21..=0xFF3A | 0xFF41..=0xFF5A | 0xFF10..=0xFF19 => Kind::Latin,
        _ => Kind::Other,
    }
}

/// 前付け(凡例)と後付け(底本)を落として本文だけにする。
fn body_of(raw: &str) -> (String, String, String) {
    let lines: Vec<&str> = raw.lines().collect();
    let title = lines.first().unwrap_or(&"").trim().to_string();
    let author = lines.get(1).unwrap_or(&"").trim().to_string();

    // 罫線(ハイフンの並び)2本に挟まれた凡例を飛ばす
    let rule: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.len() >= 20 && l.chars().all(|c| c == '-' || c == '‐' || c == '―'))
        .map(|(i, _)| i)
        .collect();
    let start = if rule.len() >= 2 { rule[1] + 1 } else { 2 };

    // 「底本：」以降は後付け
    let end = lines
        .iter()
        .enumerate()
        .skip(start)
        .find(|(_, l)| l.trim_start().starts_with("底本："))
        .map(|(i, _)| i)
        .unwrap_or(lines.len());

    let body = lines[start.min(lines.len())..end.min(lines.len())].join("\n");
    (title, author, body)
}

/// 字が無いことを示す印(ゲタ)。外字はここに置き換える。
pub const GETA: char = '〓';

/// `［＃…］` の入力者注を取り除く。
///
/// ただし **外字 `※［＃…］` は「文字が1つある」ことを表している**ので、
/// 消さずにゲタ `〓` を置く。消すと後続のルビが親文字を失い、
/// 手前の平仮名を巻き込む(実例: `※［＃「目＋爭」…］《みは》` が
/// 直前の「を」を親にしてしまった)。
fn strip_notes(s: &str) -> String {
    let ch: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < ch.len() {
        if ch[i] == '［' && ch.get(i + 1) == Some(&'＃') {
            // 対応する ］ まで飛ばす(入れ子は青空文庫では起きない)
            let mut j = i + 2;
            while j < ch.len() && ch[j] != '］' {
                j += 1;
            }
            // 外字は1文字ぶんの場所を残す
            if out.ends_with('※') {
                out.pop();
                out.push(GETA);
            }
            i = j + 1;
        } else {
            out.push(ch[i]);
            i += 1;
        }
    }
    out
}

/// ルビを剥がし、正解として取り出す。
pub fn parse(raw: &str) -> Work {
    let (title, author, body) = body_of(raw);
    let body = strip_notes(&body);

    let ch: Vec<char> = body.chars().collect();
    let mut out: Vec<char> = Vec::new();
    let mut ruby: Vec<Ruby> = Vec::new();
    // ｜ が示した親文字列の開始位置(out の中の位置)
    let mut mark: Option<usize> = None;
    let mut i = 0;

    while i < ch.len() {
        match ch[i] {
            '｜' => {
                mark = Some(out.len());
                i += 1;
            }
            '《' => {
                let mut j = i + 1;
                let mut reading = String::new();
                while j < ch.len() && ch[j] != '》' {
                    reading.push(ch[j]);
                    j += 1;
                }
                let start = match mark.take() {
                    Some(m) => m,
                    None => back_over_same_kind(&out),
                };
                let base: String = out[start..].iter().collect();
                if !base.is_empty() && !reading.is_empty() {
                    ruby.push(Ruby { base, reading, at: start });
                }
                i = j + 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }

    Work { title, author, text: out.into_iter().collect(), ruby }
}

/// 直前から、同じ種別の文字が続く限り遡る。`｜` が無いときの親文字列の決め方。
fn back_over_same_kind(out: &[char]) -> usize {
    let Some(&last) = out.last() else { return 0 };
    let k = kind(last);
    if k == Kind::Other {
        // 記号にはルビが付かない。親を取らない
        return out.len();
    }
    let mut s = out.len();
    while s > 0 && kind(out[s - 1]) == k {
        s -= 1;
    }
    s
}

impl Work {
    /// `at` のまわりの文脈を切り出す。**読みは前後を見ないと決まらない**ので、
    /// モデルには必ずこれを渡す。
    pub fn context(&self, at: usize, base_len: usize, width: usize) -> String {
        let ch: Vec<char> = self.text.chars().collect();
        let s = at.saturating_sub(width);
        let e = (at + base_len + width).min(ch.len());
        ch[s..e].iter().collect()
    }

    /// 同じ親文字列に何通りの読みが付いていたか。
    /// **2通り以上ある語が、辞書では決まらない語**。
    pub fn ambiguous(&self) -> Vec<(String, Vec<String>)> {
        let mut map: std::collections::BTreeMap<&str, Vec<&str>> = Default::default();
        for r in &self.ruby {
            let e = map.entry(&r.base).or_default();
            if !e.contains(&r.reading.as_str()) {
                e.push(&r.reading);
            }
        }
        map.into_iter()
            .filter(|(_, v)| v.len() > 1)
            .map(|(k, v)| (k.to_string(), v.into_iter().map(String::from).collect()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work(body: &str) -> Work {
        parse(&format!("表題\n著者\n\n{}\n{}\n本文開始\n{}\n底本：どこか\n",
            "-".repeat(30), "《》：ルビ", body))
    }

    #[test]
    fn ルビを剥がして正解が取れる() {
        let w = work("吾輩《わがはい》は猫である");
        assert!(w.text.ends_with("吾輩は猫である"), "本文: {:?}", w.text);
        let r = w.ruby.last().unwrap();
        assert_eq!(r.base, "吾輩");
        assert_eq!(r.reading, "わがはい");
    }

    #[test]
    fn 親文字列は同じ種別だけ遡る() {
        // 「一番」の手前の「は」は平仮名なので親に入らない
        let w = work("それは獰悪《どうあく》な");
        let r = w.ruby.last().unwrap();
        assert_eq!(r.base, "獰悪", "平仮名まで巻き込んでいる: {:?}", r.base);
    }

    #[test]
    fn 縦棒があれば種別をまたいで親になる() {
        let w = work("一番｜獰悪な奴《やつ》だ");
        let r = w.ruby.last().unwrap();
        assert_eq!(r.base, "獰悪な奴", "｜の指定を無視している: {:?}", r.base);
    }

    #[test]
    fn 位置が剥がしたあとの本文と合う() {
        let w = work("あの吾輩《わがはい》が");
        let r = w.ruby.last().unwrap();
        let ch: Vec<char> = w.text.chars().collect();
        let got: String = ch[r.at..r.at + r.base.chars().count()].iter().collect();
        assert_eq!(got, r.base, "at がずれている(本文: {:?})", w.text);
    }

    #[test]
    fn 入力者注は本文から消える() {
        let w = work("あばた［＃「あばた」に傍点］の顔");
        assert!(!w.text.contains('＃'), "注記が残っている: {:?}", w.text);
        assert!(w.text.contains("あばたの顔"), "{:?}", w.text);
    }

    #[test]
    fn 外字はゲタになって場所が残る() {
        let w = work("※［＃「言＋墟のつくり」、第4水準2-88-74］と言った");
        assert!(!w.text.contains('※'), "外字の※が残っている: {:?}", w.text);
        assert!(w.text.contains(GETA), "外字の場所が消えた: {:?}", w.text);
    }

    #[test]
    fn 外字に付いたルビが手前を巻き込まない() {
        // 実例(こころ): ※［＃「目＋爭」…］《みは》 が直前の「を」を親にしていた
        let w = work("目を※［＃「目＋爭」、第3水準1-88-85］《みは》った");
        let r = w.ruby.last().unwrap();
        assert_eq!(r.base, GETA.to_string(), "外字ではなく手前を親にした: {:?}", r.base);
        assert_eq!(r.reading, "みは");
    }

    #[test]
    fn 前付けと後付けが落ちる() {
        let w = work("本文だよ");
        assert!(!w.text.contains("《》：ルビ"), "凡例が残っている: {:?}", w.text);
        assert!(!w.text.contains("底本"), "後付けが残っている: {:?}", w.text);
        assert_eq!(w.title, "表題");
        assert_eq!(w.author, "著者");
    }

    #[test]
    fn 読みが割れる語を数えられる() {
        let w = work("後《のち》に。後《あと》で。後《うし》ろ。前《まえ》に。");
        let a = w.ambiguous();
        assert_eq!(a.len(), 1, "{a:?}");
        assert_eq!(a[0].0, "後");
        assert_eq!(a[0].1.len(), 3, "3通りの読みを拾えていない: {:?}", a[0].1);
    }

    #[test]
    fn 文脈を切り出せる() {
        let w = work("その後《のち》のこと");
        let r = w.ruby.last().unwrap();
        let c = w.context(r.at, r.base.chars().count(), 3);
        assert!(c.contains("後"), "{c:?}");
        assert!(c.contains("のこと") || c.contains("その"), "前後が入っていない: {c:?}");
    }

    #[test]
    fn 壊れた入力でも落ちない() {
        for s in ["", "《", "｜《》", "［＃", "表題のみ", "《読み》だけ"] {
            let _ = parse(s);
        }
    }
}

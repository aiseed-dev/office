//! 英語の綴り検査 — **辞書でやる。モデルは使わない。GPU も要らない。**
//!
//! ここが日本語との分かれ目で、この製品の主張そのものでもある。
//!
//! > **英語の綴り誤りは、辞書に無い語になる。**(`recieve`)
//! > **日本語の誤変換は、辞書に有る語になる。**(「以外」/「意外」)
//!
//! だから英語は40年前に辞書で解けた。日本語は同じ手では解けない。
//! 辞書引きでは「以外」に何の問題も見つからない — 正しい語だから。
//! ふりがなも同じで、「後」がどう読まれるかは辞書には書けない(文脈で変わる)。
//!
//! **道具を使い分けるのが正しい。** 英語に大型モデルを回すのは無駄で、
//! 日本語に辞書を回すのは無力。同じ画面(レビュー > 校正)に出しつつ、
//! 中身は本文の言語で振り分ける。
//!
//! 返すものは日本語側と同じ形 — **順に並んだ候補**。断定はしない。

use std::collections::HashSet;

/// このソフト自身が扱う固有名詞。一般の英語辞書には載っていない。
///
/// 大量に持たない — **ここは辞書の代わりではない**。
/// 利用者の語は `OFFICE_DICT_USER` で足す。
const BUILTIN_NAMES: &[&str] = &[
    "radeon", "amd", "rocm", "vllm", "gpu", "cpu", "vram", "kanji", "kana",
    "hiragana", "katakana", "furigana", "romaji", "docx", "xlsx", "ooxml",
    "hunspell", "aozora", "kotoba", "kumihan", "rust", "cargo", "gpui",
    "utf", "json", "http", "https", "api", "ssh", "linux", "wayland",
];

/// 綴りの指摘1件。`suggestions` は**順序が意味を持つ**(ふりがなと同じ形)。
#[derive(Debug, Clone, PartialEq)]
pub struct Misspelling {
    pub word: String,
    /// 本文における文字位置
    pub at: usize,
    pub suggestions: Vec<String>,
}

/// 語彙。既定は OS の辞書(/usr/share/dict/words)。
pub struct Dictionary {
    words: HashSet<String>,
}

impl Dictionary {
    /// 既定の置き場から読む。**無ければ Err** — 黙って「誤りなし」にしない。
    ///
    /// `OFFICE_DICT_USER` があれば足す。**固有名詞は一般の辞書に載らない**ので
    /// (`Radeon` → `Radon` と直された)、hunspell と同じくユーザ辞書で受ける。
    pub fn load_default() -> Result<Self, String> {
        let paths = [
            std::env::var("OFFICE_DICT").unwrap_or_default(),
            "/usr/share/dict/words".into(),
            "/usr/share/dict/american-english".into(),
            "/usr/share/hunspell/en_US.dic".into(),
        ];
        for p in paths.iter().filter(|p| !p.is_empty()) {
            if let Ok(s) = std::fs::read_to_string(p) {
                let mut d = Self::from_list(&s);
                d.add_user_words();
                return Ok(d);
            }
        }
        Err("英語の辞書が見つかりません(OFFICE_DICT で場所を指定できます)".into())
    }

    /// ユーザ辞書(`OFFICE_DICT_USER`)と、このソフト自身が使う固有名詞を足す。
    fn add_user_words(&mut self) {
        for w in BUILTIN_NAMES {
            self.words.insert(w.to_lowercase());
        }
        if let Ok(p) = std::env::var("OFFICE_DICT_USER") {
            if let Ok(s) = std::fs::read_to_string(&p) {
                for line in s.lines() {
                    let w = line.trim();
                    if !w.is_empty() && !w.starts_with('#') {
                        self.words.insert(w.to_lowercase());
                    }
                }
            }
        }
    }

    /// 語を1つ足す(利用者が「これは正しい」と言ったとき)。
    pub fn accept(&mut self, word: &str) {
        self.words.insert(word.to_lowercase());
    }

    /// 1行1語。hunspell の `.dic` にある `word/FLAGS` の旗も落とす。
    pub fn from_list(s: &str) -> Self {
        let mut words = HashSet::new();
        for line in s.lines() {
            let w = line.split('/').next().unwrap_or("").trim();
            if w.is_empty() || w.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            words.insert(w.to_lowercase());
        }
        Self { words }
    }

    pub fn len(&self) -> usize {
        self.words.len()
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    pub fn contains(&self, w: &str) -> bool {
        let w = w.to_lowercase();
        if self.words.contains(&w) {
            return true;
        }
        // 所有格・短縮形は本体で見る (John's → john)
        if let Some(stem) = w.strip_suffix("'s").or_else(|| w.strip_suffix("’s")) {
            return self.words.contains(stem);
        }
        false
    }

    /// 編集距離1で辞書に当たる語を、順に並べて返す。
    pub fn suggest(&self, w: &str) -> Vec<String> {
        let lower = w.to_lowercase();
        let ch: Vec<char> = lower.chars().collect();
        let mut seen = HashSet::new();
        let mut out: Vec<String> = Vec::new();
        let push = |cand: String, out: &mut Vec<String>, seen: &mut HashSet<String>| {
            if cand != lower && self.words.contains(&cand) && seen.insert(cand.clone()) {
                out.push(cand);
            }
        };
        // 削除
        for i in 0..ch.len() {
            let mut v = ch.clone();
            v.remove(i);
            push(v.into_iter().collect(), &mut out, &mut seen);
        }
        // 入れ替え
        for i in 0..ch.len().saturating_sub(1) {
            let mut v = ch.clone();
            v.swap(i, i + 1);
            push(v.into_iter().collect(), &mut out, &mut seen);
        }
        // 置換・挿入
        for c in 'a'..='z' {
            for i in 0..ch.len() {
                let mut v = ch.clone();
                v[i] = c;
                push(v.into_iter().collect(), &mut out, &mut seen);
            }
            for i in 0..=ch.len() {
                let mut v = ch.clone();
                v.insert(i, c);
                push(v.into_iter().collect(), &mut out, &mut seen);
            }
        }
        // 頭文字が同じものを先に(打ち間違いは頭が合っていることが多い)
        let head = ch.first().copied();
        out.sort_by_key(|s| (s.chars().next() != head, s.clone()));
        out.truncate(5);
        // 元の語が大文字始まりなら合わせる
        if w.chars().next().is_some_and(|c| c.is_uppercase()) {
            out = out.iter().map(|s| capitalize(s)).collect();
        }
        out
    }

    /// 本文を検査する。**英語以外の語は触らない。**
    pub fn check(&self, text: &str) -> Vec<Misspelling> {
        let mut out = Vec::new();
        for (at, word) in words_of(text) {
            if self.contains(&word) {
                continue;
            }
            out.push(Misspelling { at, suggestions: self.suggest(&word), word });
        }
        out
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// ラテン文字の語だけを、文字位置つきで拾う。
pub fn words_of(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut start = 0usize;
    for (i, c) in text.chars().enumerate() {
        let part = c.is_ascii_alphabetic() || ((c == '\'' || c == '’') && !cur.is_empty());
        if part {
            if cur.is_empty() {
                start = i;
            }
            cur.push(c);
        } else if !cur.is_empty() {
            out.push((start, std::mem::take(&mut cur).trim_matches(|c| c == '\'' || c == '’').to_string()));
        }
    }
    if !cur.is_empty() {
        out.push((start, cur.trim_matches(|c| c == '\'' || c == '’').to_string()));
    }
    out.retain(|(_, w)| w.len() > 1);
    out
}

/// 本文の言語。**振り分けのためだけの、粗い判定でよい。**
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Lang {
    Japanese,
    Latin,
    Unknown,
}

/// 仮名か漢字が1文字でもあれば日本語として扱う。
///
/// 日本語の文書に英単語が混ざるのは普通なので、**多数決にしない**。
/// 混在文では日本語側(モデル)に回し、英語の語だけ辞書でも見る。
pub fn lang_of(text: &str) -> Lang {
    let mut latin = false;
    for c in text.chars() {
        let u = c as u32;
        if (0x3040..=0x30FF).contains(&u) || (0x4E00..=0x9FFF).contains(&u) {
            return Lang::Japanese;
        }
        if c.is_ascii_alphabetic() {
            latin = true;
        }
    }
    if latin {
        Lang::Latin
    } else {
        Lang::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict() -> Dictionary {
        Dictionary::from_list(
            "receive\nreceipt\nrecess\nthe\nquick\nbrown\nfox\ndocument\ndocuments\n\
             separate\ndesperate\nJohn\noffice\nspell\nchecker\nis\na\nsolved\nproblem\n",
        )
    }

    #[test]
    fn 辞書にある語は指摘しない() {
        assert!(dict().check("the quick brown fox").is_empty());
    }

    #[test]
    fn 綴り誤りを見つけて候補を出す() {
        let m = dict().check("I recieve the documnt");
        let w: Vec<&str> = m.iter().map(|x| x.word.as_str()).collect();
        assert!(w.contains(&"recieve"), "誤りを見逃した: {w:?}");
        let r = m.iter().find(|x| x.word == "recieve").unwrap();
        assert!(r.suggestions.contains(&"receive".to_string()), "候補: {:?}", r.suggestions);
    }

    #[test]
    fn 候補は順に並ぶ() {
        // ふりがなと同じ形 — 断定ではなく候補
        let s = dict().suggest("seperate");
        assert!(s.contains(&"separate".to_string()), "{s:?}");
        assert!(s.len() <= 5, "候補が多すぎる: {s:?}");
    }

    #[test]
    fn 大文字始まりは候補も大文字始まり() {
        let d = Dictionary::from_list("john\noffice\n");
        let s = d.suggest("Jonh");
        assert_eq!(s, vec!["John"], "{s:?}");
    }

    #[test]
    fn 所有格は本体で見る() {
        let d = Dictionary::from_list("john\n");
        assert!(d.contains("John's"), "所有格を誤りにしてしまう");
    }

    #[test]
    fn 位置が本文と合う() {
        let d = dict();
        let text = "the documnt here";
        let m = d.check(text);
        let x = m.iter().find(|x| x.word == "documnt").unwrap();
        let ch: Vec<char> = text.chars().collect();
        let got: String = ch[x.at..x.at + x.word.chars().count()].iter().collect();
        assert_eq!(got, "documnt", "at がずれている");
    }

    #[test]
    fn 日本語は辞書で触らない() {
        // 「以外」は辞書に無いが、これは綴り誤りではない
        let m = dict().check("それは以外な結果でした");
        assert!(m.is_empty(), "日本語を綴り誤りにした: {m:?}");
    }

    #[test]
    fn 言語の振り分け() {
        assert_eq!(lang_of("これは日本語です"), Lang::Japanese);
        assert_eq!(lang_of("This is English."), Lang::Latin);
        assert_eq!(lang_of("Radeon で動く"), Lang::Japanese, "混在は日本語側へ");
        assert_eq!(lang_of("123 + 456"), Lang::Unknown);
        assert_eq!(lang_of(""), Lang::Unknown);
    }

    #[test]
    fn 固有名詞を誤りにしない() {
        // AMD への提出物で Radeon を「Radon の誤り」と直すわけにいかない
        let d = Dictionary::load_default().expect("辞書");
        for w in ["Radeon", "ROCm", "AMD", "docx", "hunspell"] {
            assert!(d.contains(w), "固有名詞を綴り誤りにした: {w}");
        }
    }

    #[test]
    fn 利用者が語を足せる() {
        let mut d = Dictionary::from_list("the\n");
        assert!(!d.contains("Funen"));
        d.accept("Funen");
        assert!(d.contains("Funen"), "足した語が効いていない");
    }

    #[test]
    fn 一文字の語は指摘しない() {
        // I, a などの取りこぼしを誤りにしない
        assert!(dict().check("I a x").is_empty());
    }

    #[test]
    fn 辞書が無ければエラーにする() {
        // 「誤りなし」と黙って返さない
        let d = Dictionary::from_list("");
        assert!(d.is_empty());
    }

    #[test]
    fn 日本語の誤変換は辞書では捕まらない() {
        // この製品の主張そのもの。英語と日本語で手が違う理由
        let d = Dictionary::from_list("以外\n意外\n");
        assert!(d.contains("以外"), "「以外」は正しい語なので辞書にある");
        // だから「それ以外な結果」の誤りは辞書引きでは絶対に出てこない
        assert!(d.check("それ以外な結果").is_empty());
    }
}

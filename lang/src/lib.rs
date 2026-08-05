//! `lang` — 言語ひとつぶんを差し込む層。**gpui を知らない。**
//!
//! ここが `ui` から分かれている理由は拡張点が欲しいからではない。
//! **画面が無くても回せる必要がある**から(コーパス17,000作品を窓越しには測れない)。
//!
//! # 各国語版はここを足すだけで作れる
//!
//! 綴りの誤りは、言語によって**正体が違う**。
//!
//! | | 誤りの正体 | 足りる道具 |
//! |---|---|---|
//! | 英語・独語・仏語… | 誤りは**辞書に無い語**になる (`recieve`) | **辞書**(hunspell 型) |
//! | 日本語・中国語… | 誤りは**辞書に有る語**になる (以外/意外) | **モデル** |
//!
//! 英語はこの違いのおかげで40年前に辞書で解け、hunspell という公共財になった。
//! 日本語は同じ手では解けないので、企業のものであり続けた。
//! **オープンウェイトのモデルは、その第2の型の言語にとっての hunspell になれる。**
//!
//! だから中核は言語に依らない形にし、言語ごとの差は [`Language`] に閉じる。
//! いま入っているのは [`ja::Japanese`] と [`latin::Latin`](英語ほかラテン文字圏)。
//! 中国語・韓国語を足すときも、足すのはこの実装1つだけ。
//!
//! リボンの言葉は Euro-Office のロケールから起こす(45言語ぶん現物がある)ので、
//! **画面の翻訳は言語追加の作業に含まれない。**

pub mod check;
pub mod i18n;
// gen_lang:begin(この間は ui/gen_lang.py が生成する — 手で書かない)
pub mod i18n_en;
pub mod i18n_tables;
// gen_lang:end
pub mod ja;
pub mod latin;
pub mod ai;
pub mod model;
pub mod spell;

/// 読みを注記したい箇所(ふりがな・拼音など)。
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub base: String,
    /// 本文における文字位置
    pub at: usize,
}

/// 言語ひとつぶん。**各国語版を作るとは、これを実装すること。**
pub trait Language: Send + Sync {
    /// BCP47 の言語タグ。Euro-Office のロケール名と揃える(`ja` `en` `zh` …)
    fn tag(&self) -> &'static str;

    /// 人が読む名前
    fn name(&self) -> &'static str;

    /// **綴りの検査に辞書で足りるか。**
    ///
    /// `true`  — 誤りが辞書に無い語になる言語。hunspell 型で解ける。GPU は要らない
    /// `false` — 誤りが辞書に有る語になる言語。モデルが要る
    fn dictionary_suffices(&self) -> bool;

    /// 本文がこの言語か(**振り分けのための粗い判定でよい**)
    fn detect(&self, text: &str) -> bool;

    /// 校正のためにモデルへ渡す指示。辞書で足りる言語は `None`
    fn proof_prompt(&self) -> Option<&'static str>;

    /// 読みの注記(日本語=ふりがな、中国語=拼音)を付ける言語か。
    /// 付けない言語は `None`
    fn reading_prompt(&self) -> Option<&'static str>;

    /// 読みを注記する対象を本文から拾う(日本語なら漢字の連なり)。
    /// 既定は「注記しない」
    fn reading_targets(&self, _text: &str) -> Vec<Target> {
        Vec::new()
    }
}

/// 使える言語。**足したらここに1行**。
pub fn all() -> Vec<Box<dyn Language>> {
    vec![Box::new(ja::Japanese), Box::new(latin::Latin)]
}

/// 本文からどの言語かを決める。
///
/// **辞書で足りない言語を先に見る。** 日本語の文書に英単語が混ざるのは普通で、
/// 逆は稀だから — 迷ったら重い方に倒す方が、見落としが減る。
pub fn detect(text: &str) -> Option<Box<dyn Language>> {
    let mut langs = all();
    langs.sort_by_key(|l| l.dictionary_suffices());
    langs.into_iter().find(|l| l.detect(text))
}

/// タグで引く(`JO_LANG=ja` のように指定されたとき)。
pub fn by_tag(tag: &str) -> Option<Box<dyn Language>> {
    all().into_iter().find(|l| l.tag() == tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 言語が揃っている() {
        let tags: Vec<&str> = all().iter().map(|l| l.tag()).collect();
        assert!(tags.contains(&"ja"), "{tags:?}");
        assert!(tags.contains(&"en"), "{tags:?}");
    }

    #[test]
    fn 辞書で足りるかが言語ごとに違う() {
        // これがこのソフトの存在理由そのもの
        assert!(
            !by_tag("ja").unwrap().dictionary_suffices(),
            "日本語が辞書で解けることになっている"
        );
        assert!(by_tag("en").unwrap().dictionary_suffices(), "英語にモデルを要求している");
    }

    #[test]
    fn 本文から言語を選べる() {
        assert_eq!(detect("これは日本語です").unwrap().tag(), "ja");
        assert_eq!(detect("This is English.").unwrap().tag(), "en");
    }

    #[test]
    fn 混在文は重い方へ倒す() {
        // 見落とすより、余分に見る方がまし
        assert_eq!(detect("Radeon で動く").unwrap().tag(), "ja");
    }

    #[test]
    fn 読みの注記は言語による() {
        assert!(by_tag("ja").unwrap().reading_prompt().is_some(), "日本語にふりがなが無い");
        assert!(by_tag("en").unwrap().reading_prompt().is_none(), "英語に読みの注記は要らない");
    }

    #[test]
    fn 知らないタグは無い() {
        assert!(by_tag("xx").is_none());
    }
}

//! ラテン文字圏(英語・独語・仏語…)。**辞書で解ける側の言語。**
//!
//! 綴りの誤りは `recieve` のように**辞書に無い語**として出てくるので、
//! 辞書引きで見つかる。だから英語は40年前に解け、hunspell という公共財になった。
//! この機械でも `hunspell-en-us` はシステムパッケージで、
//! 全デスクトップ環境が依存している。**誰も英語を綴るために金を払っていない。**
//!
//! ただし辞書は**答えではなく絞り込み**。辞書に無い語が誤りとは限らない
//! (`Bennet` `Radeon` は正しい)。実測:
//!
//! ```text
//! 『高慢と偏見』 123,688語 → 辞書に無い語 312種 (0.25%)
//! 技術文書        2,114語 → 辞書に無い語  74種 (3.50%)
//! ```
//!
//! 残りだけモデルに訊く(固有名詞か、誤りか)。**一番速い推論は、動かさない推論。**
//!
//! 読みの注記は無い — **英語では綴りが読み**だから。

use crate::Language;

pub struct Latin;

impl Language for Latin {
    fn tag(&self) -> &'static str {
        "en"
    }

    fn name(&self) -> &'static str {
        "English"
    }

    /// **足りる。** 誤りは辞書に無い語になる
    fn dictionary_suffices(&self) -> bool {
        true
    }

    fn detect(&self, text: &str) -> bool {
        text.chars().any(|c| c.is_ascii_alphabetic())
    }

    /// 辞書で足りるので、校正にモデルは要らない
    fn proof_prompt(&self) -> Option<&'static str> {
        None
    }

    /// 綴りが読みなので、注記するものが無い
    fn reading_prompt(&self) -> Option<&'static str> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ラテン文字があれば該当() {
        assert!(Latin.detect("This is English."));
        assert!(!Latin.detect("これは日本語"));
        assert!(!Latin.detect("123 456"));
    }

    #[test]
    fn 校正にモデルを使わない() {
        assert!(Latin.proof_prompt().is_none(), "英語にモデルを要求している");
    }

    #[test]
    fn 読みの注記は無い() {
        assert!(Latin.reading_targets("anything").is_empty());
    }
}

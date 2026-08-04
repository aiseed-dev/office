//! 設定の器 — `~/.config/office/settings.toml`(recent・sign.key の隣)。
//!
//! 優先順は **環境変数 > settings.toml > 既定**(現場の検証で一時的に
//! 差し替えたいときのため。SEKKEI「設定 — 器と言語」)。
//! writer と calc で1つのファイルを共有する。
//!
//! 読むのは素朴な `key = "value"` の行だけ(節 `[writer]` などは今は
//! 読み飛ばす)。依存を増やさない — この用途に TOML の全文法は要らない。
//!
//! ```toml
//! language = "en"   # リボンの言葉。ja(既定)か en
//! ```

use std::path::PathBuf;
use std::sync::OnceLock;

fn path() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".config/office/settings.toml")
}

/// settings.toml から素朴に1つの鍵を読む(`key = "value"` の行)
fn from_file(key: &str) -> Option<String> {
    let s = std::fs::read_to_string(path()).ok()?;
    for line in s.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let (k, v) = line.split_once('=')?;
        if k.trim() == key {
            return Some(v.trim().trim_matches('"').to_string());
        }
    }
    None
}

/// リボンの言葉。**文言が揃った言語だけ**を受ける(いまは ja と en —
/// できないものを、できるように見せない)。それ以外の指定は ja に落ちる。
/// 環境変数 OFFICE_LANG が一時上書きの口
pub fn language() -> &'static str {
    static LANG: OnceLock<String> = OnceLock::new();
    LANG.get_or_init(|| {
        let raw = std::env::var("OFFICE_LANG")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| from_file("language"))
            .unwrap_or_default();
        match raw.as_str() {
            "en" => "en".into(),
            _ => "ja".into(),
        }
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn 環境変数でenの表が選ばれる() {
        // OnceLock は1プロセス1回 — この試験プロセスで最初に language() を
        // 呼ぶのはここ(他の試験は表を直接見る)。旗を立ててから引く
        std::env::set_var("OFFICE_LANG", "en");
        assert_eq!(super::language(), "en");
        assert_eq!(crate::ribbon::writer_tabs()[1].name, "Home");
        assert_eq!(crate::ribbon::calc_tabs()[1].name, "Home");
    }

    #[test]
    fn 知らない言語はjaに落ちる() {
        // language() は一度きり(OnceLock)なので、判定の芯だけ検査する
        let pick = |raw: &str| match raw {
            "en" => "en",
            _ => "ja",
        };
        assert_eq!(pick("en"), "en");
        assert_eq!(pick("fr"), "ja", "文言の無い言語を名乗らない");
        assert_eq!(pick(""), "ja");
    }
}

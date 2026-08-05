//! 画面の文言の言語 — **日本語の文がそのまま鍵**。
//!
//! 設計(SEKKEI「設定 — 器と言語」段階③):
//! - 呼び出しは `t!("…")`(そのままの文)と `tf!("…{}…", 値)`(穴埋め)。
//!   ja では鍵をそのまま返すので、**日本語の挙動は1バイトも変わらない**
//! - en は [`i18n_en`](crate::i18n_en) の対訳表で引く。**表に無い文は
//!   ja のまま出る**(嘘の英語を作らない)— 表の完全性は
//!   `python3 ui/gen_i18n.py` が検査する(未訳があれば止まる)
//! - 穴埋めは実行時の簡易整形(format! は雛形がコンパイル時定数でないと
//!   使えないため)。対応する書式は `{}`・`{:.0}`・`{:?}` — このアプリの
//!   文言が実際に使う3種だけ(増やすときはここに足す)

use std::collections::HashMap;
use std::fmt::Display;
use std::path::PathBuf;
use std::sync::OnceLock;

fn settings_path() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".config/office/settings.toml")
}

/// settings.toml から素朴に1つの鍵を読む(`key = "value"` の行)
fn from_file(key: &str) -> Option<String> {
    let s = std::fs::read_to_string(settings_path()).ok()?;
    for line in s.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == key {
                return Some(v.trim().trim_matches('"').to_string());
            }
        }
    }
    None
}

/// 画面の言語。**文言が揃った言語だけ**を受ける(登録簿 i18n_tables が正)。
/// 優先順: 環境変数 OFFICE_LANG > settings.toml > 既定 ja
pub fn language() -> &'static str {
    static LANG: OnceLock<String> = OnceLock::new();
    LANG.get_or_init(|| {
        let raw = std::env::var("OFFICE_LANG")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| from_file("language"))
            .unwrap_or_default();
        if crate::i18n_tables::LANGS.contains(&raw.as_str()) {
            raw
        } else {
            "ja".into()
        }
    })
}

/// 選べる言語(ja + 表の揃った言語)。設定ページの巡回もこれを見る
pub fn languages() -> Vec<&'static str> {
    let mut v = vec!["ja"];
    v.extend_from_slice(crate::i18n_tables::LANGS);
    v
}

fn lang_map() -> Option<&'static HashMap<&'static str, &'static str>> {
    static MAP: OnceLock<Option<HashMap<&'static str, &'static str>>> = OnceLock::new();
    MAP.get_or_init(|| {
        crate::i18n_tables::table(language()).map(|t| t.iter().copied().collect())
    })
    .as_ref()
}

/// 文をいまの言語で。表に無い文は ja のまま(嘘の翻訳を作らない)
pub fn tr(ja: &'static str) -> &'static str {
    match lang_map() {
        Some(m) => m.get(ja).copied().unwrap_or(ja),
        None => ja,
    }
}

/// 穴埋めつきの文。雛形を tr で引いてから、実行時に埋める
pub fn trf(ja: &'static str, args: &[&dyn Display]) -> String {
    fill(tr(ja), args)
}

/// 簡易整形: `{}`・`{:.0}`・`{:?}` を左から順に埋める。
/// `{{`・`}}` は文字どおりの括弧
fn fill(template: &str, args: &[&dyn Display]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut it = template.chars().peekable();
    let mut n = 0usize;
    while let Some(c) = it.next() {
        match c {
            '{' if it.peek() == Some(&'{') => {
                it.next();
                out.push('{');
            }
            '}' if it.peek() == Some(&'}') => {
                it.next();
                out.push('}');
            }
            '{' => {
                let mut spec = String::new();
                for d in it.by_ref() {
                    if d == '}' {
                        break;
                    }
                    spec.push(d);
                }
                let s = args.get(n).map(|a| a.to_string()).unwrap_or_default();
                n += 1;
                match spec.as_str() {
                    ":.0" => match s.parse::<f64>() {
                        Ok(v) => out.push_str(&format!("{v:.0}")),
                        Err(_) => out.push_str(&s),
                    },
                    // {:?} は呼び側が表示済みの文字を渡す約束(そのまま)
                    _ => out.push_str(&s),
                }
            }
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 穴埋めが順に入る() {
        assert_eq!(fill("{} 列で {} 件", &[&"B", &3]), "B 列で 3 件");
        assert_eq!(fill("縮尺 {:.0}%", &[&99.6]), "縮尺 100%");
        assert_eq!(fill("{{リテラル}} と {}", &[&"値"]), "{リテラル} と 値");
    }

    #[test]
    fn 表に無い文はそのまま() {
        // ja では常に鍵のまま。en で表に無くても鍵のまま(黙って英語を作らない)
        assert_eq!(tr("この文は表に無い(試験用)"), "この文は表に無い(試験用)");
    }
}

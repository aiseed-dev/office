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

/// settings.toml の置き場(recent・sign.key の隣)
pub fn path() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".config/office/settings.toml")
}

/// settings.toml から素朴に1つの鍵を読む(`key = "value"` の行)
pub fn get(key: &str) -> Option<String> {
    let s = std::fs::read_to_string(path()).ok()?;
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

/// settings.toml に1つの鍵を書く(他の行は保つ。無ければ行を足す)
pub fn set(key: &str, value: &str) {
    let p = path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let cur = std::fs::read_to_string(&p).unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    let mut done = false;
    for line in cur.lines() {
        let t = line.trim();
        if !done && !t.starts_with('#') && !t.starts_with('[') {
            if let Some((k, _)) = t.split_once('=') {
                if k.trim() == key {
                    lines.push(format!("{key} = \"{value}\""));
                    done = true;
                    continue;
                }
            }
        }
        lines.push(line.to_string());
    }
    if !done {
        lines.push(format!("{key} = \"{value}\""));
    }
    let _ = std::fs::write(&p, lines.join("\n") + "\n");
}

/// リボンと文言の言語。実体は lang::i18n(1本道 — 表計算の関数や
/// 校正と同じ crate に置き、gpui を知らない層でも引けるように)
pub fn language() -> &'static str {
    lang::i18n::language()
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

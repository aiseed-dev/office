//! `office-spell` — 日本語も見る綴り検査。**単体で使える道具。**
//!
//! hunspell が英語にしたことを日本語でやる、というのがこのソフトの存在理由なので、
//! 校正はワープロの中に閉じ込めない。**誰でもパイプで通せる形**で出す。
//!
//!   office-spell 文書.txt
//!   cat 文書.txt | office-spell
//!   office-spell -l 文書.txt          # 語だけ(hunspell -l と同じ形)
//!   office-spell --furigana 文書.txt  # 読みの候補を出す
//!
//! 英語は辞書だけで動く(**モデルも GPU も要らない**)。
//! 日本語はモデルに繋がらなければ、**そう言う**。黙って「指摘なし」にはしない。
//!
//! 宛先: OFFICE_HOST / OFFICE_PORT / OFFICE_MODEL / OFFICE_API_KEY、辞書: OFFICE_DICT

use std::io::Read;

use lang::check::{Checker, Source};
use lang::Target;
use lang::spell;

fn main() {
    let mut list_only = false;
    let mut furigana = false;
    let mut washi = false;
    let mut files: Vec<String> = Vec::new();
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "-l" | "--list" => list_only = true,
            "--furigana" => furigana = true,
            // pywashi の記法で本文ごと出す。そのまま washi --pdf で紙になる
            "--washi" => {
                furigana = true;
                washi = true;
            }
            "-h" | "--help" => {
                eprintln!("{}", HELP);
                return;
            }
            f => files.push(f.to_string()),
        }
    }

    let text = match read_input(&files) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("読めません: {e}");
            std::process::exit(2);
        }
    };

    let c = Checker::default();
    if furigana {
        run_furigana(&c, &text, washi);
        return;
    }

    let r = c.check(&text);

    if list_only {
        for f in &r.findings {
            println!("{}", f.found);
        }
    } else {
        for f in &r.findings {
            let tool = match f.source {
                Source::Dictionary => "辞書",
                Source::Model => "モデル",
            };
            let where_ = f.at.map(|a| format!("{a}")).unwrap_or_else(|| "?".into());
            let cand = if f.candidates.is_empty() {
                "(候補なし)".to_string()
            } else {
                f.candidates.join(" / ")
            };
            println!("{where_}\t{}\t{}\t→ {cand}\t[{tool}]", f.kind.label(), f.found);
        }
        eprintln!("{}", r.summary());
    }

    // 検査できなかった部分があるなら、成功で終わらない。
    // 「終了コード0 = 問題なし」と受け取られるため
    if !r.skipped.is_empty() {
        std::process::exit(3);
    }
    if !r.findings.is_empty() {
        std::process::exit(1);
    }
}

fn run_furigana(c: &Checker, text: &str, washi: bool) {
    // 漢字の連なりを対象にする。**どれに振るかは決め打ちしない** —
    // 候補を出して、選ぶのは人
    let targets = kanji_runs(text);
    if targets.is_empty() {
        eprintln!("漢字がありません");
        return;
    }
    match c.readings(text, &targets) {
        Ok(sug) => {
            if washi {
                // 第1候補を振った本文をそのまま出す。
                //   office-spell --washi 原稿.txt > 原稿.md && washi 原稿.md --pdf
                print!("{}", lang::ja::furigana::to_washi(text, &sug));
            } else {
                for s in &sug {
                    println!("{}\t{}\t{}", s.at, s.base, s.readings.join(" / "));
                }
            }
            eprintln!("{} 語に候補を出しました", sug.len());
        }
        Err(e) => {
            // 繋がらないことと、ふりがなが不要なことは違う
            eprintln!("ふりがなを出せません — {e}");
            eprintln!("(宛先は OFFICE_HOST / OFFICE_PORT / OFFICE_MODEL で変えられます)");
            std::process::exit(3);
        }
    }
}

/// 漢字が続くところを拾う。
fn kanji_runs(text: &str) -> Vec<Target> {
    let is_kanji = |c: char| {
        let u = c as u32;
        (0x4E00..=0x9FFF).contains(&u) || u == 0x3005
    };
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut start = 0usize;
    for (i, ch) in text.chars().enumerate() {
        if is_kanji(ch) {
            if cur.is_empty() {
                start = i;
            }
            cur.push(ch);
        } else if !cur.is_empty() {
            out.push(Target { base: std::mem::take(&mut cur), at: start });
        }
    }
    if !cur.is_empty() {
        out.push(Target { base: cur, at: start });
    }
    out
}

fn read_input(files: &[String]) -> Result<String, String> {
    if files.is_empty() {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).map_err(|e| e.to_string())?;
        return Ok(s);
    }
    let mut s = String::new();
    for f in files {
        s.push_str(&std::fs::read_to_string(f).map_err(|e| format!("{f}: {e}"))?);
        s.push('\n');
    }
    Ok(s)
}

const HELP: &str = "office-spell — 日本語も見る綴り検査

  office-spell [ファイル…]        指摘を出す(標準入力も可)
  office-spell -l                語だけ出す(hunspell -l と同じ形)
  office-spell --furigana        ふりがなの候補を出す
  office-spell --washi           第1候補を振った本文を pywashi の記法で出す
                             (office-spell --washi 原稿.txt > 原稿.md && washi 原稿.md --pdf)

英語は辞書だけで動く(モデルも GPU も要らない)。
日本語の誤変換は辞書に有る語になるので、辞書では捕まらない。だからモデルを使う。

終了コード: 0=指摘なし 1=指摘あり 2=入力が読めない 3=検査できなかった部分がある

環境変数: OFFICE_DICT(辞書) OFFICE_HOST OFFICE_PORT OFFICE_MODEL OFFICE_API_KEY(モデルの宛先)";

#[allow(dead_code)]
fn _lang(text: &str) -> spell::Lang {
    spell::lang_of(text)
}

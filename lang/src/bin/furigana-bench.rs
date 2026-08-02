//! ふりがなの命中率と速度を、青空文庫で測る。**窓を開かずに回す。**
//!
//! 入力者が人手で付けたルビが正解。ルビを剥いだ本文だけをモデルに渡し、
//! 候補を出させ、元のルビが**第何候補に入っていたか**を数える。
//! 正解率ではなく **top-N 命中率**なので、当てられない語があっても成立する。
//!
//! 使い方:
//!   furigana-bench --dry corpus/*.txt        # モデル無し。コーパスの素性だけ見る
//!   furigana-bench --limit 200 corpus/*.txt  # モデルに訊いて採点する
//!
//! 宛先は環境変数(OFFICE_HOST / OFFICE_PORT / OFFICE_MODEL / OFFICE_API_KEY)。
//! Radeon Cloud の専用インスタンスは SSH 転送で 127.0.0.1 に見えるので、そのまま届く。

use lang::ja::aozora;
use lang::ja::furigana::{self, Hits};
use lang::Target;
use lang::model::Endpoint;

struct Args {
    dry: bool,
    limit: usize,
    width: usize,
    batch: usize,
    files: Vec<String>,
}

fn parse_args() -> Args {
    let mut a = Args { dry: false, limit: 100, width: 60, batch: 8, files: Vec::new() };
    let mut it = std::env::args().skip(1);
    while let Some(s) = it.next() {
        match s.as_str() {
            "--dry" => a.dry = true,
            "--limit" => a.limit = it.next().and_then(|v| v.parse().ok()).unwrap_or(a.limit),
            "--width" => a.width = it.next().and_then(|v| v.parse().ok()).unwrap_or(a.width),
            "--batch" => a.batch = it.next().and_then(|v| v.parse().ok()).unwrap_or(a.batch),
            "-h" | "--help" => {
                eprintln!("furigana-bench [--dry] [--limit N] [--width W] [--batch B] <file.txt>...");
                std::process::exit(0);
            }
            f => a.files.push(f.to_string()),
        }
    }
    a
}

fn main() {
    let args = parse_args();
    if args.files.is_empty() {
        eprintln!("入力がありません。青空文庫の .txt を UTF-8 にして渡してください。");
        eprintln!("  (著作権フラグ「あり」の928作品は、コーパスを作る側で除外すること)");
        std::process::exit(2);
    }

    let ep = Endpoint::default();
    if !args.dry {
        eprintln!("宛先: {}:{}{} model={}", ep.host, ep.port, ep.path, ep.model);
    }

    let mut hits = Hits::default();
    let (mut ms, mut ptok, mut ctok) = (0u128, 0u64, 0u64);
    let (mut works, mut all_ruby, mut all_chars, mut amb_kinds) = (0usize, 0usize, 0usize, 0usize);

    for path in &args.files {
        let Ok(raw) = std::fs::read_to_string(path) else {
            eprintln!("読めません: {path}");
            continue;
        };
        let w = aozora::parse(&raw);
        let chars = w.text.chars().count();
        let amb = w.ambiguous();
        works += 1;
        all_ruby += w.ruby.len();
        all_chars += chars;
        amb_kinds += amb.len();

        println!(
            "── {} / {}  {} 文字, ルビ {} 箇所, 読みが割れる語 {} 種",
            w.title,
            w.author,
            fmt(chars),
            fmt(w.ruby.len()),
            fmt(amb.len())
        );
        if args.dry {
            for (base, rs) in amb.iter().take(5) {
                println!("     {base} → {}", rs.join(" / "));
            }
            continue;
        }

        // 読みが割れる語を優先して訊く。**そこがこの仕事の本体**
        let hard: std::collections::BTreeSet<&str> =
            amb.iter().map(|(b, _)| b.as_str()).collect();
        // 外字(ゲタ)は字が分からないので訊いても意味がない。採点から外す
        let askable = |r: &&aozora::Ruby| !r.base.contains(aozora::GETA);
        let mut picks: Vec<&aozora::Ruby> = w
            .ruby
            .iter()
            .filter(|r| hard.contains(r.base.as_str()))
            .filter(askable)
            .collect();
        if picks.len() < args.limit {
            picks.extend(
                w.ruby.iter().filter(|r| !hard.contains(r.base.as_str())).filter(askable),
            );
        }
        picks.truncate(args.limit);
        picks.sort_by_key(|r| r.at);

        for chunk in picks.chunks(args.batch) {
            let first = chunk[0];
            let last = chunk[chunk.len() - 1];
            let span = last.at + last.base.chars().count() - first.at;
            let ctx = w.context(first.at, span, args.width);
            let targets: Vec<Target> =
                chunk.iter().map(|r| Target { base: r.base.clone(), at: r.at }).collect();

            match furigana::candidates(&ep, &ctx, &targets) {
                Ok((sug, reply)) => {
                    ms += reply.elapsed_ms;
                    ptok += reply.prompt_tokens;
                    ctok += reply.completion_tokens;
                    for r in chunk {
                        let rank = sug
                            .iter()
                            .find(|s| s.at == r.at)
                            .and_then(|s| s.rank_of(&r.reading));
                        hits.add(rank);
                    }
                }
                Err(e) => {
                    // 繋がらないのに「命中率0%」と出すと、モデルが悪いように読める
                    eprintln!("\nモデルに問い合わせできません: {e}");
                    eprintln!("採点を中止します(繋がらないことと、当たらないことは別)。");
                    std::process::exit(1);
                }
            }
            eprint!("\r  訊いた {} 件 …", hits.asked);
        }
        eprintln!();
    }

    println!();
    println!("=== コーパス ===");
    println!("  作品           : {}", fmt(works));
    println!("  本文           : {} 文字", fmt(all_chars));
    println!("  ルビ(正解)     : {} 箇所", fmt(all_ruby));
    println!("  読みが割れる語 : {} 種  ← 辞書では決まらない", fmt(amb_kinds));

    if args.dry {
        println!("\n(--dry。モデルには訊いていない)");
        return;
    }

    println!();
    println!("=== 命中率(正解が第何候補に入っていたか)===");
    println!("  訊いた       : {}", fmt(hits.asked));
    println!("  候補に有り   : {} ({:.1}%)", fmt(hits.answered), hits.top(5));
    for n in 1..=3 {
        println!("  top-{n}        : {:.1}%", hits.top(n));
    }

    println!();
    println!("=== 速度 ===");
    println!("  所要         : {:.1} 秒", ms as f64 / 1000.0);
    println!("  入力/出力    : {} / {} トークン", fmt(ptok as usize), fmt(ctok as usize));
    if ms > 0 {
        println!("  生成         : {:.1} tok/s", ctok as f64 * 1000.0 / ms as f64);
        // 「1冊が何分か」— tok/s より実務に近い数字
        if hits.asked > 0 && all_chars > 0 {
            let per = ms as f64 / hits.asked as f64;
            let est = per * all_ruby as f64 / 1000.0 / 60.0;
            println!("  この分量なら : 全ルビで約 {est:.1} 分", );
        }
    }
}

fn fmt(n: usize) -> String {
    let s = n.to_string();
    let b = s.as_bytes();
    let mut o = String::new();
    for (i, c) in b.iter().enumerate() {
        if i > 0 && (b.len() - i) % 3 == 0 {
            o.push(',');
        }
        o.push(*c as char);
    }
    o
}

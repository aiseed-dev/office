//! 実物の xlsx を読み、再計算し、書き戻す。
//!   cargo run -p sheet --example xl -- 入力.xlsx 出力.xlsx
use std::fs::File;
use std::io::BufWriter;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut a = std::env::args().skip(1);
    let (src, dst) = (a.next().expect("入力"), a.next().expect("出力"));
    let (mut book, rep) = sheet::xlsx::read(File::open(&src)?)?;
    println!("シート {} / セル {}", rep.sheets, rep.cells);
    if !rep.is_lossless() {
        println!("未対応:");
        for (n, c) in &rep.unsupported { println!("  {n} × {c}") }
    }
    for s in &mut book.sheets {
        let (r, c) = s.extent();
        let nf = s.cells.values().filter(|c| c.formula.is_some()).count();
        println!("  [{}] {}行×{}列 / 値{} / 式{}", s.name, r, c, s.cells.len(), nf);
        sheet::recalc(s);
        for (p, cell) in s.cells.iter().take(4) {
            println!("      {} = {}{}", p.a1(), cell.value.display(),
                cell.formula.as_ref().map(|f| format!("  (={f})")).unwrap_or_default());
        }
    }
    sheet::xlsx::write(&book, BufWriter::new(File::create(&dst)?))?;
    println!("書き出し: {dst}");
    Ok(())
}

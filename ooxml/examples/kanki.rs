//! K3 の実物検証: 実在の docx を読み、紙面に組み、docx として書き戻す。
//!
//!   cargo run -p ooxml --example kanki -- 入力.docx 出力.docx [紙面.pdf]
//!
//! 読めなかったものは必ず標準エラーに出す(黙って落とさない)。

use std::fs::File;
use std::io::BufWriter;

use kumihan::{layout, Frame, Metrics};


/// 本文のフォント。同梱せず、システムから探す
fn font_data() -> &'static [u8] {
    static FONT: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    FONT.get_or_init(|| {
        let (fam, _) = kumihan::font::for_document(None).expect("日本語フォントが要る");
        kumihan::font::load(fam).expect("読めない")
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut a = std::env::args().skip(1);
    let (src, dst) = match (a.next(), a.next()) {
        (Some(s), Some(d)) => (s, d),
        _ => {
            eprintln!("使い方: kanki 入力.docx 出力.docx [紙面.pdf]");
            std::process::exit(2);
        }
    };
    let pdf_out = a.next();

    // 読む
    let (doc, rep) = ooxml::read(File::open(&src)?)?;
    println!("読み込み: {} 段落 / {} ラン ({src})", rep.paragraphs, rep.runs);
    if rep.is_lossless() {
        println!("未対応の要素なし");
    } else {
        println!("未対応(この版では読み飛ばした):");
        for (name, n) in &rep.unsupported {
            println!("  {name} × {n}");
        }
    }
    let chars: usize = doc.paragraphs()
        .flat_map(|p| p.runs.iter()).map(|r| r.text.chars().count()).sum();
    let ntbl = doc.tables().count();
    let cells: usize = doc.tables().map(|t| t.rows.iter().map(|r| r.len()).sum::<usize>()).sum();
    println!("本文の文字数: {chars} / 表 {ntbl}個({cells}セル)");
    for p in doc.paragraphs().take(5) {
        let s: String = p.runs.iter().map(|r| r.text.as_str()).collect();
        if !s.trim().is_empty() {
            println!("  | {}", s.chars().take(48).collect::<String>());
        }
    }

    // 書き戻す
    ooxml::write(&doc, BufWriter::new(File::create(&dst)?))?;
    println!("書き出し: {dst}");

    // 紙面に組んで PDF にも出す(K0 の組版がそのまま効く)
    if let Some(pdf) = pdf_out {
        use printpdf::*;
        let m = Metrics::new(font_data()).map_err(|e| e.to_string())?;
        let sheet = layout(&doc, &m,
            &Frame { measure_mm: 170.0, line_height_mm: 6.4, y0_mm: 24.0 });
        let (p, page, layer) = PdfDocument::new("docx", Mm(210.0), Mm(297.0), "L1");
        let font = p.add_external_font(std::io::Cursor::new(font_data()))?;
        let l = p.get_page(page).get_layer(layer);
        for line in sheet.lines.iter().take(40) {
            l.use_text(line.text(), line.cells[0].size_pt,
                       Mm(20.0 + line.cells[0].x_mm), Mm(297.0 - line.y_mm), &font);
        }
        p.save(&mut BufWriter::new(File::create(&pdf)?))?;
        println!("紙面: {pdf}({}行)", sheet.lines.len());
    }
    Ok(())
}

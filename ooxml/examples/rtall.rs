//! 実物の書類を 開く→保存 して、内容と部品を突き合わせる
fn main() {
    let src = std::env::args().nth(1).unwrap();
    let dst = std::env::args().nth(2).unwrap();
    let bytes = std::fs::read(&src).unwrap();
    if src.ends_with(".docx") {
        let (doc, rep) = ooxml::read(std::io::Cursor::new(&bytes)).unwrap();
        let f = std::fs::File::create(&dst).unwrap();
        ooxml::write_with(&doc, Some(std::io::Cursor::new(&bytes)), std::io::BufWriter::new(f)).unwrap();
        let (back, _) = ooxml::read(std::io::Cursor::new(std::fs::read(&dst).unwrap())).unwrap();
        let a = doc.body_text();
        let b = back.body_text();
        println!("本文 {} 字 → {} 字 {} / 表 {} → {} / 注記 {}",
            a.chars().count(), b.chars().count(),
            if a == b { "一致" } else { "**不一致**" },
            doc.tables().count(), back.tables().count(),
            rep.unsupported.len());
    } else {
        let (book, rep) = sheet::xlsx::read(std::io::Cursor::new(&bytes)).unwrap();
        let f = std::fs::File::create(&dst).unwrap();
        sheet::xlsx::write_with(&book, Some(std::io::Cursor::new(&bytes)), std::io::BufWriter::new(f)).unwrap();
        let (back, _) = sheet::xlsx::read(std::io::Cursor::new(std::fs::read(&dst).unwrap())).unwrap();
        let n = |b: &sheet::Book| -> (usize, usize, usize) {
            (b.sheets.iter().map(|s| s.cells.len()).sum(),
             b.sheets.iter().map(|s| s.merges.len()).sum(),
             b.sheets.iter().map(|s| s.col_width.len()).sum())
        };
        let (c1, m1, w1) = n(&book);
        let (c2, m2, w2) = n(&back);
        println!("セル {c1}→{c2} {} / 結合 {m1}→{m2} / 列幅 {w1}→{w2} / 注記 {}",
            if (c1, m1, w1) == (c2, m2, w2) { "一致" } else { "**不一致**" },
            rep.unsupported.len());
    }
}

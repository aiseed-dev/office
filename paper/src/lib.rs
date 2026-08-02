//! 紙面を紙へ写す — 印刷と PDF 出力。
//!
//! **組版はやり直さない。** 画面に出しているのと同じ [`kumihan::Sheet`] を、
//! 座標そのままで PDF の面に置く。だから**画面と紙が必ず一致する**
//! (別々に組み直すと、そこで食い違いが生まれる)。
//!
//! engine 側に置かないのは、engine を PDF から独立させておくため。

use std::io::{BufWriter, Write};

use kumihan::{CharFormat, Sheet};
use printpdf::*;

/// 紙の大きさ(mm)。既定は A4 縦。
#[derive(Debug, Clone, Copy)]
pub struct Paper {
    pub width_mm: f32,
    pub height_mm: f32,
    /// 左の余白。紙面の x はここからの相対
    pub margin_mm: f32,
}

impl Default for Paper {
    fn default() -> Self {
        Paper { width_mm: 210.0, height_mm: 297.0, margin_mm: 20.0 }
    }
}

/// 紙面を PDF にする。
///
/// `font_data` は画面に使っているのと**同じフォントの実体**を渡すこと。
/// 別のものを渡すと字幅が変わり、画面と紙がずれる。
pub fn to_pdf<W: Write>(
    sheet: &Sheet,
    font_data: &[u8],
    paper: Paper,
    out: W,
) -> Result<(), String> {
    let (doc, page, layer) = PdfDocument::new(
        "office",
        Mm(paper.width_mm),
        Mm(paper.height_mm),
        "本文",
    );
    let font = doc
        .add_external_font(std::io::Cursor::new(font_data))
        .map_err(|e| e.to_string())?;
    let l = doc.get_page(page).get_layer(layer);

    for line in &sheet.lines {
        if line.cells.is_empty() {
            continue;
        }
        // 太字は同じ書体を少しずらして二度打つ(合成太字)。
        // 太字の実体を別に持っていないので、**持っていないものを持っている顔をしない**
        let bold = line.cells[0].fmt.bold;
        let text = line.text();
        let pt = line.cells[0].size_pt;
        let x = paper.margin_mm + line.cells[0].x_mm;
        // PDF の原点は左下。紙面の y は上からなので裏返す
        let y = paper.height_mm - line.y_mm;

        l.use_text(&text, pt, Mm(x), Mm(y), &font);
        if bold {
            l.use_text(&text, pt, Mm(x + 0.12), Mm(y), &font);
        }
        rule(&l, &line.cells[0].fmt, x, y, width_mm(line), pt);
    }

    doc.save(&mut BufWriter::new(out)).map_err(|e| e.to_string())
}

fn width_mm(line: &kumihan::Line) -> f32 {
    line.cells.iter().map(|c| c.w_mm).sum()
}

/// 下線と取り消し線。フォントが持っていないので線として引く。
fn rule(l: &PdfLayerReference, f: &CharFormat, x: f32, y: f32, w: f32, pt: f32) {
    let em = pt * 25.4 / 72.0;
    for (on, dy) in [(f.underline, -em * 0.18), (f.strike, em * 0.28)] {
        if !on {
            continue;
        }
        l.add_line(Line {
            points: vec![
                (Point::new(Mm(x), Mm(y + dy)), false),
                (Point::new(Mm(x + w), Mm(y + dy)), false),
            ],
            is_closed: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use kumihan::{font, layout, Align, Document, Frame, Metrics};

    use super::*;

    fn sheet(text: &str, align: Align) -> (Sheet, Vec<u8>) {
        let (fam, _) = font::for_document(None).unwrap();
        let data = font::load(fam).unwrap();
        let m = Metrics::new(&data).unwrap();
        let mut d = Document::plain(text, 10.5);
        d.apply_align(0..text.len(), align);
        let s = layout(&d, &m, &Frame { measure_mm: 170.0, line_height_mm: 6.4, y0_mm: 24.0 });
        (s, data)
    }

    fn pdf_of(text: &str, align: Align) -> Vec<u8> {
        let (s, data) = sheet(text, align);
        let mut buf = Vec::new();
        to_pdf(&s, &data, Paper::default(), &mut buf).unwrap();
        buf
    }

    #[test]
    fn pdfになる() {
        let b = pdf_of("日本語の書類を紙にする。", Align::Left);
        assert_eq!(&b[..5], b"%PDF-", "PDF になっていない");
        assert!(b.len() > 1000, "中身が薄すぎる: {} バイト", b.len());
    }

    #[test]
    fn 画面と同じ紙面から作る() {
        // 組み直さないので、行数は紙面のまま
        let (s, data) = sheet("一行目\n二行目\n三行目", Align::Left);
        assert_eq!(s.lines.len(), 3);
        let mut buf = Vec::new();
        to_pdf(&s, &data, Paper::default(), &mut buf).unwrap();
        assert_eq!(&buf[..5], b"%PDF-");
    }

    #[test]
    fn 中央揃えが紙にも効く() {
        // 揃えは紙面の x に入っているので、PDF 側で作り直さない
        let (left, _) = sheet("表題", Align::Left);
        let (center, _) = sheet("表題", Align::Center);
        assert!(
            center.lines[0].cells[0].x_mm > left.lines[0].cells[0].x_mm,
            "中央揃えが紙面に出ていない"
        );
    }

    #[test]
    fn 空の紙面でも落ちない() {
        let (fam, _) = font::for_document(None).unwrap();
        let data = font::load(fam).unwrap();
        let mut buf = Vec::new();
        to_pdf(&Sheet::default(), &data, Paper::default(), &mut buf).unwrap();
        assert_eq!(&buf[..5], b"%PDF-");
    }
}

//! 帳票(表計算)を紙へ写す。
//!
//! writer と同じ約束: **画面に見えているもの(値・書式・罫線)を写すだけ。**
//! 計算はやり直さない。
//!
//! まだやらないこと(黙らずに書いておく):
//!   - 塗りつぶしの色(罫線と文字だけ)
//!   - 列幅の指定(全列同じ幅)
//!   - 横に紙からはみ出す列は**次の紙に送らず、切れる**(status で伝えること)

use std::io::{BufWriter, Write};

use printpdf::*;
use sheet::model::{format_value, HAlign, Value};
use sheet::Sheet as Grid;

use crate::Paper;

const COL_MM: f32 = 26.0;
const ROW_MM: f32 = 7.0;

/// 1つの表を PDF にする。行が紙に収まらなければ次のページへ。
pub fn sheet_to_pdf<W: Write>(
    grid: &Grid,
    font_data: &[u8],
    paper: Paper,
    out: W,
) -> Result<(), String> {
    let (rows, cols) = grid.extent();
    let (doc, page, layer) = PdfDocument::new(
        &grid.name,
        Mm(paper.width_mm),
        Mm(paper.height_mm),
        "帳票",
    );
    let font = doc
        .add_external_font(std::io::Cursor::new(font_data))
        .map_err(|e| e.to_string())?;
    let mut l = doc.get_page(page).get_layer(layer);

    let rows_per_page =
        (((paper.height_mm - 2.0 * paper.margin_mm) / ROW_MM).floor() as u32).max(1);

    for r in 0..rows.max(1) {
        if r > 0 && r % rows_per_page == 0 {
            let (np, nl) = doc.add_page(
                Mm(paper.width_mm),
                Mm(paper.height_mm),
                format!("帳票 {}", r / rows_per_page + 1),
            );
            l = doc.get_page(np).get_layer(nl);
        }
        let y_top = paper.height_mm - paper.margin_mm - (r % rows_per_page) as f32 * ROW_MM;
        for c in 0..cols.max(1) {
            let p = sheet::Pos::new(r, c);
            let x = paper.margin_mm + c as f32 * COL_MM;
            let Some(cell) = grid.cells.get(&p) else { continue };

            // 罫線。引いてある辺だけ
            let b = cell.fmt.borders;
            for (on, (x1, y1, x2, y2)) in [
                (b.top, (x, y_top, x + COL_MM, y_top)),
                (b.bottom, (x, y_top - ROW_MM, x + COL_MM, y_top - ROW_MM)),
                (b.left, (x, y_top, x, y_top - ROW_MM)),
                (b.right, (x + COL_MM, y_top, x + COL_MM, y_top - ROW_MM)),
            ] {
                if on {
                    l.add_line(Line {
                        points: vec![
                            (Point::new(Mm(x1), Mm(y1)), false),
                            (Point::new(Mm(x2), Mm(y2)), false),
                        ],
                        is_closed: false,
                    });
                }
            }

            // 値。結合に呑まれた位置は左上にだけ出る(画面と同じ)
            if grid.covered_by_merge(p) {
                continue;
            }
            let shown = format_value(&cell.value, cell.fmt.number_format.as_deref());
            if shown.is_empty() {
                continue;
            }
            // 数は右、文字は左(指定があればそちら)
            let right = match cell.fmt.align {
                HAlign::Right => true,
                HAlign::Left | HAlign::Center => false,
                HAlign::General => matches!(cell.value, Value::Number(_)),
            };
            let pt = 9.5f32;
            let tx = if right {
                // だいたいの字幅で右に寄せる(全角 1em / 半角 0.55em)
                let w: f32 = shown
                    .chars()
                    .map(|ch| if ch.is_ascii() { 0.55 } else { 1.0 })
                    .sum::<f32>()
                    * pt
                    * 25.4
                    / 72.0;
                x + COL_MM - 1.5 - w
            } else {
                x + 1.5
            };
            let ty = y_top - ROW_MM + 2.0;
            l.use_text(&shown, pt, Mm(tx), Mm(ty), &font);
            if cell.fmt.bold {
                l.use_text(&shown, pt, Mm(tx + 0.1), Mm(ty), &font);
            }
        }
    }
    doc.save(&mut BufWriter::new(out)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use sheet::model::{Borders, Cell, CellFormat, Pos, Value};

    use super::*;

    fn grid() -> Grid {
        let mut s = Grid { name: "見積".into(), ..Default::default() };
        for (a1, v) in [("A1", "品名"), ("B1", "金額")] {
            s.set(Pos::parse(a1).unwrap(), Cell {
                formula: None,
                value: Value::Text(v.into()),
                fmt: CellFormat { borders: Borders::ALL, bold: true, ..Default::default() },
            });
        }
        s.set(Pos::parse("B2").unwrap(), Cell {
            formula: None,
            value: Value::Number(1200.0),
            fmt: CellFormat {
                borders: Borders::ALL,
                number_format: Some("#,##0".into()),
                ..Default::default()
            },
        });
        s
    }

    #[test]
    fn 帳票がpdfになる() {
        let (fam, _) = kumihan::font::for_document(None).unwrap();
        let data = kumihan::font::load(fam).unwrap();
        let mut buf = Vec::new();
        sheet_to_pdf(&grid(), &data, Paper::default(), &mut buf).unwrap();
        assert_eq!(&buf[..5], b"%PDF-");
        assert!(buf.len() > 1000);
    }

    #[test]
    fn 多い行は複数ページになる() {
        let (fam, _) = kumihan::font::for_document(None).unwrap();
        let data = kumihan::font::load(fam).unwrap();
        let mut s = Grid { name: "長い".into(), ..Default::default() };
        for r in 0..80 {
            s.set(Pos::new(r, 0), Cell {
                formula: None, value: Value::Number(r as f64), fmt: Default::default() });
        }
        let mut buf = Vec::new();
        sheet_to_pdf(&s, &data, Paper::default(), &mut buf).unwrap();
        let hay = String::from_utf8_lossy(&buf).to_string();
        let i = hay.find("/Count ").unwrap() + 7;
        let n: usize = hay[i..].chars().take_while(|c| c.is_ascii_digit())
            .collect::<String>().parse().unwrap();
        assert!(n >= 2, "80行が {n} ページ(下へはみ出している)");
    }

    #[test]
    fn 空の表でも落ちない() {
        let (fam, _) = kumihan::font::for_document(None).unwrap();
        let data = kumihan::font::load(fam).unwrap();
        let mut buf = Vec::new();
        sheet_to_pdf(&Grid { name: "空".into(), ..Default::default() },
                     &data, Paper::default(), &mut buf).unwrap();
        assert_eq!(&buf[..5], b"%PDF-");
    }
}

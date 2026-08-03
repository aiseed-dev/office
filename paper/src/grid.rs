//! 帳票(表計算)を紙へ写す。
//!
//! writer と同じ約束: **画面に見えているもの(値・書式・罫線・塗り・文字色)を
//! 写すだけ。** 計算はやり直さない。条件付き書式も画面と同じ規則で効く。
//!
//! まだやらないこと(黙らずに書いておく):
//!   - 横に紙からはみ出す列は**次の紙に送らず、切れる**。
//!     切れた列の数を返すので、呼ぶ側は画面に出すこと(黙って落とさない)

use std::io::{BufWriter, Write};

use printpdf::*;
use sheet::model::{format_value, HAlign, Value};
use sheet::Sheet as Grid;

use crate::Paper;

const COL_MM: f32 = 26.0;
const ROW_MM: f32 = 7.0;
/// xlsx の列幅1 ≒ 2.0mm(標準フォントの「0」1個ぶん)
const MM_PER_CHW: f32 = 2.0;

/// `RRGGBB` を 0..1 の RGB にする。読めなければ None(黙って黒にしない)。
fn hex_rgb(s: &str) -> Option<(f32, f32, f32)> {
    let g = |i: usize| {
        s.get(i * 2..i * 2 + 2)
            .and_then(|h| u8::from_str_radix(h, 16).ok())
            .map(|v| v as f32 / 255.0)
    };
    Some((g(0)?, g(1)?, g(2)?))
}

/// 1つの表を PDF にする。行が紙に収まらなければ次のページへ。
/// 返すのは**右にはみ出して切れた列の数**(0 なら全部紙に入っている)。
pub fn sheet_to_pdf<W: Write>(
    grid: &Grid,
    font_data: &[u8],
    paper: Paper,
    out: W,
) -> Result<u32, String> {
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

    // 列の幅と左端(文書の指定に従う)
    let col_mm: Vec<f32> = (0..cols.max(1))
        .map(|c| grid.col_width.get(&c).copied().or(grid.default_col_width)
            .map(|w| w * MM_PER_CHW).unwrap_or(COL_MM))
        .collect();
    let mut col_x = vec![0.0f32];
    for w in &col_mm {
        col_x.push(col_x.last().unwrap() + w);
    }
    // 右にはみ出して切れる列(右端が紙の使える幅を超えるもの)
    let usable_w = paper.width_mm - 2.0 * paper.margin_mm;
    let clipped = (0..cols)
        .filter(|c| col_x[*c as usize + 1] > usable_w + 0.1)
        .count() as u32;

    // 行の高さ(pt → mm)。指定のない行は既定
    let row_mm = |r: u32| -> f32 {
        grid.row_height.get(&r).map(|pt| pt * 25.4 / 72.0).unwrap_or(ROW_MM)
    };
    let usable = paper.height_mm - 2.0 * paper.margin_mm;

    let mut y_used = 0.0f32; // このページで使った高さ
    let mut page_no = 1u32;
    for r in 0..rows.max(1) {
        let rh = row_mm(r);
        if y_used + rh > usable && y_used > 0.0 {
            page_no += 1;
            y_used = 0.0;
            let (np, nl) = doc.add_page(
                Mm(paper.width_mm),
                Mm(paper.height_mm),
                format!("帳票 {page_no}"),
            );
            l = doc.get_page(np).get_layer(nl);
        }
        let y_top = paper.height_mm - paper.margin_mm - y_used;
        y_used += rh;
        for c in 0..cols.max(1) {
            let p = sheet::Pos::new(r, c);
            let x = paper.margin_mm + col_x[c as usize];
            let cw = col_mm[c as usize];
            let Some(cell) = grid.cells.get(&p) else { continue };

            // 塗りと文字色。条件付き書式は画面と同じ規則で上書きする
            let mut fill = cell.fmt.fill.clone();
            let mut ink = cell.fmt.color.clone();
            for rule in &grid.cond {
                if rule.hits(p, &cell.value) {
                    if let Some(f) = &rule.fill {
                        fill = Some(f.clone());
                    }
                    if let Some(c) = &rule.color {
                        ink = Some(c.clone());
                    }
                }
            }
            // 塗りは罫線より先に敷く(線を塗り潰さない)
            if let Some((cr, cg, cb)) = fill.as_deref().and_then(hex_rgb) {
                l.set_fill_color(Color::Rgb(Rgb::new(cr, cg, cb, None)));
                l.add_rect(Rect::new(
                    Mm(x), Mm(y_top - rh), Mm(x + cw), Mm(y_top),
                ));
                l.set_fill_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));
            }

            // 罫線。引いてある辺だけ
            let b = cell.fmt.borders;
            for (on, (x1, y1, x2, y2)) in [
                (b.top, (x, y_top, x + cw, y_top)),
                (b.bottom, (x, y_top - rh, x + cw, y_top - rh)),
                (b.left, (x, y_top, x, y_top - rh)),
                (b.right, (x + cw, y_top, x + cw, y_top - rh)),
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
                x + cw - 1.5 - w
            } else {
                x + 1.5
            };
            let ty = y_top - rh + 2.0;
            // 文字は塗り色で描かれる(PDF の作法)ので、色付きの字は前後で入れ替える
            let colored = ink.as_deref().and_then(hex_rgb);
            if let Some((cr, cg, cb)) = colored {
                l.set_fill_color(Color::Rgb(Rgb::new(cr, cg, cb, None)));
            }
            l.use_text(&shown, pt, Mm(tx), Mm(ty), &font);
            if cell.fmt.bold {
                l.use_text(&shown, pt, Mm(tx + 0.1), Mm(ty), &font);
            }
            if colored.is_some() {
                l.set_fill_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));
            }
        }
    }
    doc.save(&mut BufWriter::new(out)).map_err(|e| e.to_string())?;
    Ok(clipped)
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
    fn 塗りと文字色が紙に出る() {
        let (fam, _) = kumihan::font::for_document(None).unwrap();
        let data = kumihan::font::load(fam).unwrap();
        let mut s = grid();
        // 塗りが無ければ長方形(re)は1つも描かれない
        let mut plain = Vec::new();
        sheet_to_pdf(&s, &data, Paper::default(), &mut plain).unwrap();
        assert!(!String::from_utf8_lossy(&plain).contains(" re\n"), "塗りが無いのに長方形がある");
        s.set(Pos::parse("A2").unwrap(), Cell {
            formula: None,
            value: Value::Text("塗り".into()),
            fmt: CellFormat {
                fill: Some("FFF2CC".into()),
                color: Some("C00000".into()),
                ..Default::default()
            },
        });
        let mut buf = Vec::new();
        sheet_to_pdf(&s, &data, Paper::default(), &mut buf).unwrap();
        let hay = String::from_utf8_lossy(&buf).to_string();
        assert!(hay.contains(" re\n"), "塗りの長方形が無い");
        assert!(hay.contains(" rg\n"), "色の指定が無い");
    }

    #[test]
    fn 条件付き書式も紙に効く() {
        let (fam, _) = kumihan::font::for_document(None).unwrap();
        let data = kumihan::font::load(fam).unwrap();
        let mut s = grid(); // B2 = 1200(塗りの指定なし)
        s.cond.push(sheet::model::CondRule {
            range: (Pos::parse("B2").unwrap(), Pos::parse("B2").unwrap()),
            op: sheet::model::CondOp::Gt,
            value: 1000.0,
            color: None,
            fill: Some("E2EFDA".into()),
        });
        let mut buf = Vec::new();
        sheet_to_pdf(&s, &data, Paper::default(), &mut buf).unwrap();
        assert!(
            String::from_utf8_lossy(&buf).contains(" re\n"),
            "条件に合う値の塗りが紙に出ない"
        );
    }

    #[test]
    fn はみ出した列の数が返る() {
        let (fam, _) = kumihan::font::for_document(None).unwrap();
        let data = kumihan::font::load(fam).unwrap();
        let mut s = Grid { name: "広い".into(), ..Default::default() };
        // 40mm × 10列 = 400mm は A4 縦(使える幅 170mm)に入り切らない
        for c in 0..10 {
            s.set(Pos::new(0, c), Cell {
                formula: None, value: Value::Number(c as f64), fmt: Default::default() });
            s.col_width.insert(c, 20.0); // 20字 ≒ 40mm
        }
        let mut buf = Vec::new();
        let clipped = sheet_to_pdf(&s, &data, Paper::default(), &mut buf).unwrap();
        assert!(clipped > 0, "切れた列が報告されない");
        let mut buf = Vec::new();
        assert_eq!(sheet_to_pdf(&grid(), &data, Paper::default(), &mut buf).unwrap(), 0,
                   "入り切っているのに切れたと言った");
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

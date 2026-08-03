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

/// 印刷の指定(帳票が持っているもの)。Paper(紙の大きさ)とは別 —
/// こちらは「どこを・どんな余白で」。
#[derive(Debug, Clone, Default)]
pub struct PrintSetup {
    /// 印刷範囲(左上, 右下)。None なら使われている全域
    pub area: Option<(sheet::Pos, sheet::Pos)>,
    /// 余白 mm(左, 右, 上, 下)。None なら paper.margin_mm を四辺に
    pub margins_mm: Option<(f32, f32, f32, f32)>,
}

/// 1つの表を PDF にする。行が紙に収まらなければ次のページへ。
/// 返すのは**右にはみ出して切れた列の数**(0 なら全部紙に入っている)。
pub fn sheet_to_pdf<W: Write>(
    grid: &Grid,
    font_data: &[u8],
    paper: Paper,
    setup: &PrintSetup,
    out: W,
) -> Result<u32, String> {
    let (ext_rows, ext_cols) = grid.extent();
    // 印刷範囲があればそこだけ(行も列も)
    let (r0, r1, c0, c1) = match setup.area {
        Some((a, b)) => (a.row, b.row + 1, a.col, b.col + 1),
        None => (0, ext_rows, 0, ext_cols),
    };
    let (ml, mr, mt, mb) = setup
        .margins_mm
        .unwrap_or((paper.margin_mm, paper.margin_mm, paper.margin_mm, paper.margin_mm));
    // 拡大縮小印刷(pageSetup scale)。列幅・行高・文字を同じ倍で
    let scale = grid.print_scale.unwrap_or(100).clamp(10, 400) as f32 / 100.0;
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

    // 列の幅と左端(文書の指定に従う)。印刷範囲の左端が原点
    let ncols = (c1 - c0).max(1);
    let col_mm: Vec<f32> = (c0..c0 + ncols)
        .map(|c| grid.col_width.get(&c).copied().or(grid.default_col_width)
            .map(|w| w * MM_PER_CHW).unwrap_or(COL_MM) * scale)
        .collect();
    let mut col_x = vec![0.0f32];
    for w in &col_mm {
        col_x.push(col_x.last().unwrap() + w);
    }
    // 右にはみ出して切れる列(右端が紙の使える幅を超えるもの)
    let usable_w = paper.width_mm - ml - mr;
    let clipped = (0..ncols)
        .filter(|i| col_x[*i as usize + 1] > usable_w + 0.1)
        .count() as u32;

    // 行の高さ(pt → mm)。指定のない行は既定
    let row_mm = |r: u32| -> f32 {
        grid.row_height.get(&r).map(|pt| pt * 25.4 / 72.0).unwrap_or(ROW_MM) * scale
    };
    let usable = paper.height_mm - mt - mb;

    // 各ページの頭で繰り返すタイトル行(自分のいる範囲の外は繰り返さない)
    let title_rows: Vec<u32> = grid
        .print_title_rows
        .map(|(a, b)| (a..=b).filter(|r| *r < r1).collect())
        .unwrap_or_default();

    // 1行を紙に描く(セルの塗り・罫線・値、印刷の枠線・行番号)
    #[allow(clippy::too_many_arguments)]
    fn draw_row(
        grid: &Grid,
        l: &PdfLayerReference,
        font: &IndirectFontRef,
        r: u32,
        y_top: f32,
        rh: f32,
        ml: f32,
        c0: u32,
        ncols: u32,
        col_x: &[f32],
        col_mm: &[f32],
        scale: f32,
    ) {
        // 印刷の枠線(printOptions gridLines)。薄い灰で先に敷く
        if grid.print_gridlines {
            l.set_outline_color(Color::Rgb(Rgb::new(0.85, 0.87, 0.89, None)));
            let w_total = col_x[ncols as usize];
            for (x1, y1, x2, y2) in [
                (ml, y_top, ml + w_total, y_top),
                (ml, y_top - rh, ml + w_total, y_top - rh),
            ] {
                l.add_line(Line {
                    points: vec![
                        (Point::new(Mm(x1), Mm(y1)), false),
                        (Point::new(Mm(x2), Mm(y2)), false),
                    ],
                    is_closed: false,
                });
            }
            for i in 0..=ncols as usize {
                l.add_line(Line {
                    points: vec![
                        (Point::new(Mm(ml + col_x[i]), Mm(y_top)), false),
                        (Point::new(Mm(ml + col_x[i]), Mm(y_top - rh)), false),
                    ],
                    is_closed: false,
                });
            }
            l.set_outline_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));
        }
        // 行番号(printOptions headings)。左の余白に小さく
        if grid.print_headings {
            l.set_fill_color(Color::Rgb(Rgb::new(0.4, 0.44, 0.48, None)));
            l.use_text((r + 1).to_string(), 6.5, Mm(ml - 7.0), Mm(y_top - rh + 2.0), font);
            l.set_fill_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));
        }
        for c in c0..c0 + ncols {
            let p = sheet::Pos::new(r, c);
            let x = ml + col_x[(c - c0) as usize];
            let cw = col_mm[(c - c0) as usize];
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
                l.add_rect(Rect::new(Mm(x), Mm(y_top - rh), Mm(x + cw), Mm(y_top)));
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
            let pt = 9.5f32 * scale;
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
            l.use_text(&shown, pt, Mm(tx), Mm(ty), font);
            if cell.fmt.bold {
                l.use_text(&shown, pt, Mm(tx + 0.1), Mm(ty), font);
            }
            if colored.is_some() {
                l.set_fill_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));
            }
        }
    }

    // 列名の見出し(printOptions headings)。各ページの上の余白に
    let draw_col_heads = |l: &PdfLayerReference| {
        if !grid.print_headings {
            return;
        }
        l.set_fill_color(Color::Rgb(Rgb::new(0.4, 0.44, 0.48, None)));
        for c in c0..c0 + ncols {
            let x = ml + col_x[(c - c0) as usize] + col_mm[(c - c0) as usize] / 2.0 - 1.0;
            let name = sheet::Pos::new(0, c).a1();
            let name = name.trim_end_matches('1');
            l.use_text(name, 6.5, Mm(x), Mm(paper.height_mm - mt + 1.5), &font);
        }
        l.set_fill_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));
    };

    let mut y_used = 0.0f32; // このページで使った高さ
    let mut page_no = 1u32;
    draw_col_heads(&l);
    for r in r0..r1.max(r0 + 1) {
        let rh = row_mm(r);
        // 改ページ(rowBreaks: この行から新しい紙)か、紙が尽きたら次のページ
        let break_here = y_used > 0.0 && grid.row_breaks.contains(&r);
        if break_here || (y_used + rh > usable && y_used > 0.0) {
            page_no += 1;
            y_used = 0.0;
            let (np, nl) = doc.add_page(
                Mm(paper.width_mm),
                Mm(paper.height_mm),
                format!("帳票 {page_no}"),
            );
            l = doc.get_page(np).get_layer(nl);
            draw_col_heads(&l);
            // タイトル行を頭で繰り返す(いま描く行が自分自身なら繰り返さない)
            if !title_rows.contains(&r) {
                for tr in &title_rows {
                    let th = row_mm(*tr);
                    let y_top = paper.height_mm - mt - y_used;
                    draw_row(grid, &l, &font, *tr, y_top, th, ml, c0, ncols, &col_x, &col_mm, scale);
                    y_used += th;
                }
            }
        }
        let y_top = paper.height_mm - mt - y_used;
        y_used += rh;
        draw_row(grid, &l, &font, r, y_top, rh, ml, c0, ncols, &col_x, &col_mm, scale);
    }
    // 図形(挿した分も読んだ分も)。**輪郭だけ**を紙に出す(塗りはまだ —
    // printpdf の多角形塗りを持ち込むまで。黙って出したことにしない)
    {
        // セル→1ページ目基準のmm(改ページをまたぐ図形の紙送りはまだ)
        let cell_mm = |at: sheet::Pos| -> (f32, f32) {
            let x: f32 = (c0..at.col.min(c0 + ncols))
                .map(|c| col_mm[(c - c0) as usize])
                .sum();
            let y: f32 = (r0..at.row.min(r1)).map(row_mm).sum();
            (ml + x, paper.height_mm - mt - y)
        };
        let l1 = doc.get_page(page).get_layer(layer);
        for sp in grid.shapes.iter().chain(grid.shapes_new.iter()) {
            let (x, y_top) = cell_mm(sp.at);
            let mm = 25.4 / 96.0; // px → mm
            let (w, h) = (sp.width_px * mm * scale, sp.height_px * mm * scale);
            if let Some((cr, cg, cb)) = sp.line.as_deref().and_then(hex_rgb) {
                l1.set_outline_color(Color::Rgb(Rgb::new(cr, cg, cb, None)));
            }
            let pts: Vec<(f32, f32)> = match sp.kind.as_str() {
                "ellipse" => (0..=24)
                    .map(|i| {
                        let t = i as f32 / 24.0 * std::f32::consts::TAU;
                        (x + w / 2.0 + w / 2.0 * t.cos(), y_top - h / 2.0 + h / 2.0 * t.sin())
                    })
                    .collect(),
                "rightArrow" => {
                    let (ty, by, bx, my) =
                        (h * 0.25, h * 0.75, w - (w * 0.35).min(h), h / 2.0);
                    vec![
                        (x, y_top - ty),
                        (x + bx, y_top - ty),
                        (x + bx, y_top),
                        (x + w, y_top - my),
                        (x + bx, y_top - h),
                        (x + bx, y_top - by),
                        (x, y_top - by),
                    ]
                }
                "diamond" => vec![
                    (x + w / 2.0, y_top),
                    (x + w, y_top - h / 2.0),
                    (x + w / 2.0, y_top - h),
                    (x, y_top - h / 2.0),
                ],
                "line" => vec![(x, y_top), (x + w, y_top - h)],
                _ => vec![
                    (x, y_top),
                    (x + w, y_top),
                    (x + w, y_top - h),
                    (x, y_top - h),
                ],
            };
            let closed = sp.kind != "line";
            l1.add_line(Line {
                points: pts
                    .into_iter()
                    .map(|(px_, py_)| (Point::new(Mm(px_), Mm(py_)), false))
                    .collect(),
                is_closed: closed,
            });
            l1.set_outline_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));
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
        sheet_to_pdf(&grid(), &data, Paper::default(), &PrintSetup::default(), &mut buf).unwrap();
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
        sheet_to_pdf(&s, &data, Paper::default(), &PrintSetup::default(), &mut buf).unwrap();
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
        sheet_to_pdf(&s, &data, Paper::default(), &PrintSetup::default(), &mut plain).unwrap();
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
        sheet_to_pdf(&s, &data, Paper::default(), &PrintSetup::default(), &mut buf).unwrap();
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
        sheet_to_pdf(&s, &data, Paper::default(), &PrintSetup::default(), &mut buf).unwrap();
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
        let clipped = sheet_to_pdf(&s, &data, Paper::default(), &PrintSetup::default(), &mut buf).unwrap();
        assert!(clipped > 0, "切れた列が報告されない");
        let mut buf = Vec::new();
        assert_eq!(sheet_to_pdf(&grid(), &data, Paper::default(), &PrintSetup::default(), &mut buf).unwrap(), 0,
                   "入り切っているのに切れたと言った");
    }

    #[test]
    fn 空の表でも落ちない() {
        let (fam, _) = kumihan::font::for_document(None).unwrap();
        let data = kumihan::font::load(fam).unwrap();
        let mut buf = Vec::new();
        sheet_to_pdf(&Grid { name: "空".into(), ..Default::default() },
                     &data, Paper::default(), &PrintSetup::default(), &mut buf).unwrap();
        assert_eq!(&buf[..5], b"%PDF-");
    }
}

#[cfg(test)]
mod print_setup_tests {
    use sheet::model::{Cell, Pos, Value};

    use super::*;

    fn long_sheet() -> Grid {
        let mut s = Grid { name: "長い".into(), ..Default::default() };
        for r in 0..80 {
            s.set(Pos::new(r, 0), Cell {
                formula: None, value: Value::Number(r as f64), fmt: Default::default() });
        }
        s
    }

    fn pages(buf: &[u8]) -> usize {
        let hay = String::from_utf8_lossy(buf).to_string();
        let i = hay.find("/Count ").unwrap() + 7;
        hay[i..].chars().take_while(|c| c.is_ascii_digit())
            .collect::<String>().parse().unwrap()
    }

    #[test]
    fn 印刷範囲だけが紙に出る() {
        let (fam, _) = kumihan::font::for_document(None).unwrap();
        let data = kumihan::font::load(fam).unwrap();
        let s = long_sheet();
        // 全域は複数ページ、先頭5行の印刷範囲なら1ページ
        let mut all = Vec::new();
        sheet_to_pdf(&s, &data, Paper::default(), &PrintSetup::default(), &mut all).unwrap();
        assert!(pages(&all) >= 2);
        let setup = PrintSetup {
            area: Some((Pos::new(0, 0), Pos::new(4, 0))),
            margins_mm: None,
        };
        let mut part = Vec::new();
        sheet_to_pdf(&s, &data, Paper::default(), &setup, &mut part).unwrap();
        assert_eq!(pages(&part), 1, "印刷範囲が効いていない");
    }

    #[test]
    fn 余白が広いほど紙が増える() {
        let (fam, _) = kumihan::font::for_document(None).unwrap();
        let data = kumihan::font::load(fam).unwrap();
        let s = long_sheet();
        let mut narrow = Vec::new();
        sheet_to_pdf(&s, &data, Paper::default(),
            &PrintSetup { area: None, margins_mm: Some((10.0, 10.0, 10.0, 10.0)) },
            &mut narrow).unwrap();
        let mut wide = Vec::new();
        sheet_to_pdf(&s, &data, Paper::default(),
            &PrintSetup { area: None, margins_mm: Some((10.0, 10.0, 100.0, 100.0)) },
            &mut wide).unwrap();
        assert!(pages(&wide) > pages(&narrow), "余白が紙の枚数に効いていない");
    }
}

#[cfg(test)]
mod print_extras_tests {
    use sheet::model::{Cell, Pos, Value};

    use super::*;

    fn long_sheet() -> Grid {
        let mut s = Grid { name: "長い".into(), ..Default::default() };
        for r in 0..30 {
            s.set(Pos::new(r, 0), Cell {
                formula: None, value: Value::Number(r as f64), fmt: Default::default() });
        }
        s
    }

    fn pages(buf: &[u8]) -> usize {
        let hay = String::from_utf8_lossy(buf).to_string();
        let i = hay.find("/Count ").unwrap() + 7;
        hay[i..].chars().take_while(|c| c.is_ascii_digit())
            .collect::<String>().parse().unwrap()
    }

    #[test]
    fn 改ページで紙が割れる() {
        let (fam, _) = kumihan::font::for_document(None).unwrap();
        let data = kumihan::font::load(fam).unwrap();
        let mut s = long_sheet(); // 30行 = 既定では1ページに収まる
        let mut one = Vec::new();
        sheet_to_pdf(&s, &data, Paper::default(), &PrintSetup::default(), &mut one).unwrap();
        assert_eq!(pages(&one), 1);
        s.row_breaks = vec![10, 20];
        let mut broken = Vec::new();
        sheet_to_pdf(&s, &data, Paper::default(), &PrintSetup::default(), &mut broken).unwrap();
        assert_eq!(pages(&broken), 3, "改ページが効いていない");
    }

    #[test]
    fn 拡大縮小で入る行数が変わる() {
        let (fam, _) = kumihan::font::for_document(None).unwrap();
        let data = kumihan::font::load(fam).unwrap();
        let mut s = Grid { name: "s".into(), ..Default::default() };
        for r in 0..80 {
            s.set(Pos::new(r, 0), Cell {
                formula: None, value: Value::Number(r as f64), fmt: Default::default() });
        }
        let mut full = Vec::new();
        sheet_to_pdf(&s, &data, Paper::default(), &PrintSetup::default(), &mut full).unwrap();
        s.print_scale = Some(50);
        let mut half = Vec::new();
        sheet_to_pdf(&s, &data, Paper::default(), &PrintSetup::default(), &mut half).unwrap();
        assert!(pages(&half) < pages(&full), "縮小しても紙が減らない");
    }

    #[test]
    fn タイトル行は2ページ目にも出る() {
        let (fam, _) = kumihan::font::for_document(None).unwrap();
        let data = kumihan::font::load(fam).unwrap();
        let mut s = long_sheet();
        s.print_title_rows = Some((0, 0));
        s.row_breaks = vec![15];
        // 描画対象の行数で確かめる: タイトル繰り返しの分、テキスト描画が1つ増える
        let mut with_t = Vec::new();
        sheet_to_pdf(&s, &data, Paper::default(), &PrintSetup::default(), &mut with_t).unwrap();
        s.print_title_rows = None;
        let mut without = Vec::new();
        sheet_to_pdf(&s, &data, Paper::default(), &PrintSetup::default(), &mut without).unwrap();
        assert!(with_t.len() > without.len(), "タイトル行の繰り返しが出ていない");
    }
}

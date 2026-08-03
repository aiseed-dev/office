//! 紙面を紙へ写す — 印刷と PDF 出力。
//!
//! **組版はやり直さない。** 画面に出しているのと同じ [`kumihan::Sheet`] を、
//! 座標そのままで PDF の面に置く。だから**画面と紙が必ず一致する**
//! (別々に組み直すと、そこで食い違いが生まれる)。
//!
//! engine 側に置かないのは、engine を PDF から独立させておくため。

pub mod grid;

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
    to_pdf_with(sheet, font_data, paper, |_| Vec::new(), out)
}

/// 紙面を PDF にする(ページごとの飾りつき)。
///
/// `page_decor(k)` は k ページ目(1始まり)に置く行 — ヘッダー・フッター。
/// 行の y はページ上端からの mm、x は左余白からの mm
/// ([`kumihan::layout_hf`] がこの形で返す)。
/// ページ番号は組む側が字にして寄越すので、ここでは**置くだけ**。
pub fn to_pdf_with<W: Write, F: Fn(usize) -> Vec<kumihan::Line>>(
    sheet: &Sheet,
    font_data: &[u8],
    paper: Paper,
    page_decor: F,
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
    let mut l = doc.get_page(page).get_layer(layer);
    // ページごとの描き先を控えておく(罫線を後から同じ頁割りで引くため)
    let mut layers: Vec<PdfLayerReference> = vec![l.clone()];

    // **改ページはここでやる。** 紙面は1枚の長い巻物として来るので、
    // 紙の高さを超えたら次のページに移し、y を巻き戻す。
    // これをやらないと、長い文書が1ページ目の下へ黙ってはみ出す。
    let bottom = paper.height_mm - paper.margin_mm;
    let mut page_no = 0u32;
    let mut offset = 0.0f32; // このページの先頭が、巻物のどの高さか
    // 明示の改ページ(文書側の指定)。高さ超過とは別に、ここでも頁を割る
    let mut breaks = sheet.breaks.iter().copied().peekable();

    for line in &sheet.lines {
        if line.cells.is_empty() {
            continue;
        }
        let mut forced = false;
        while let Some(&b) = breaks.peek() {
            if line.y_mm >= b - 0.01 {
                breaks.next();
                forced = true;
            } else {
                break;
            }
        }
        let mut y_roll = line.y_mm - offset;
        if forced || y_roll > bottom {
            // 次のページへ。行の紙面上の高さは(余白ぶんを除いて)そのまま続ける
            page_no += 1;
            offset = line.y_mm - paper.margin_mm;
            y_roll = paper.margin_mm;
            let (np, nl) = doc.add_page(
                Mm(paper.width_mm),
                Mm(paper.height_mm),
                format!("本文 {}", page_no + 1),
            );
            l = doc.get_page(np).get_layer(nl);
            layers.push(l.clone());
        }
        // 太字は同じ書体を少しずらして二度打つ(合成太字)。
        // 太字の実体を別に持っていないので、**持っていないものを持っている顔をしない**
        let bold = line.cells[0].fmt.bold;
        let text = line.text();
        let pt = line.cells[0].size_pt;
        let x = paper.margin_mm + line.cells[0].x_mm;
        // PDF の原点は左下。紙面の y は上からなので裏返す
        let y = paper.height_mm - y_roll;

        l.use_text(&text, pt, Mm(x), Mm(y), &font);
        if bold {
            l.use_text(&text, pt, Mm(x + 0.12), Mm(y), &font);
        }
        rule(&l, &line.cells[0].fmt, x, y, width_mm(line), pt);
    }

    // 画像。行と同じ頁割りで置く
    {
        let usable = bottom - paper.margin_mm;
        let page_of = |y: f32| -> usize {
            let mut off = 0.0f32;
            let mut k = 0usize;
            while y - off > bottom {
                off = if k == 0 { bottom - paper.margin_mm } else { off + usable };
                k += 1;
            }
            k
        };
        for (bytes, [x, top, w_mm, h_mm]) in &sheet.images {
            let k = page_of(*top);
            if k >= layers.len() {
                continue;
            }
            let off = if k == 0 { 0.0 } else { (bottom - paper.margin_mm) + (k - 1) as f32 * usable };
            // 復号できない画像は飛ばして続ける(1枚のために紙全体を失敗させない)
            let Ok(im) = ::image::load_from_memory(bytes) else { continue };
            // 透過つき(RGBA)は printpdf 0.7 が正しく埋め込めないので RGB に落とす
            let im = ::image::DynamicImage::ImageRgb8(im.to_rgb8());
            let xobj = printpdf::ImageXObject::from_dynamic_image(&im);
            let pdf_im = printpdf::Image::from(xobj);
            // 目標の mm に合わせて拡縮(printpdf は px と dpi から実寸を出す)
            let (pw, ph) = (im.width() as f32, im.height() as f32);
            let dpi = 300.0;
            let natural_w = pw / dpi * 25.4;
            let natural_h = ph / dpi * 25.4;
            pdf_im.add_to_layer(layers[k].clone(), printpdf::ImageTransform {
                translate_x: Some(Mm(paper.margin_mm + x)),
                translate_y: Some(Mm(paper.height_mm - (top - off) - h_mm)),
                scale_x: Some(w_mm / natural_w),
                scale_y: Some(h_mm / natural_h),
                dpi: Some(dpi),
                ..Default::default()
            });
        }
    }

    // 表の罫線。行と同じ頁割りで引く(頁をまたぐ縦線は窓で切る)
    {
        let usable = bottom - paper.margin_mm;
        let page_of = |y: f32| -> usize {
            let mut off = 0.0f32;
            let mut k = 0usize;
            // 行の頁割りと同じ計算: 1頁目は y0 から、以降は margin から
            while y - off > bottom {
                off = if k == 0 { bottom - paper.margin_mm } else { off + usable };
                k += 1;
            }
            k
        };
        for r in &sheet.rules {
            let [x1, y1, x2, y2] = *r;
            let k = page_of(y1.min(y2));
            if k >= layers.len() {
                continue;
            }
            let off = if k == 0 { 0.0 } else { (bottom - paper.margin_mm) + (k - 1) as f32 * usable };
            let l = &layers[k];
            let (ry1, ry2) = (
                paper.height_mm - (y1 - off).clamp(paper.margin_mm, bottom),
                paper.height_mm - (y2 - off).clamp(paper.margin_mm, bottom),
            );
            l.add_line(Line {
                points: vec![
                    (Point::new(Mm(paper.margin_mm + x1), Mm(ry1)), false),
                    (Point::new(Mm(paper.margin_mm + x2), Mm(ry2)), false),
                ],
                is_closed: false,
            });
        }
    }
    // ページごとの飾り(ヘッダー・フッター)。y はページ上端からの絶対位置
    for (k, l) in layers.iter().enumerate() {
        for line in page_decor(k + 1) {
            if line.cells.is_empty() {
                continue;
            }
            let text = line.text();
            let pt = line.cells[0].size_pt;
            let x = paper.margin_mm + line.cells[0].x_mm;
            let y = paper.height_mm - line.y_mm;
            l.use_text(&text, pt, Mm(x), Mm(y), &font);
            if line.cells[0].fmt.bold {
                l.use_text(&text, pt, Mm(x + 0.12), Mm(y), &font);
            }
        }
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

#[cfg(test)]
mod page_tests {
    use kumihan::{font, layout, Document, Frame, Metrics};

    use super::*;

    fn pages(n_lines: usize) -> usize {
        let (fam, _) = font::for_document(None).unwrap();
        let data = font::load(fam).unwrap();
        let m = Metrics::new(&data).unwrap();
        let text = vec!["行"; n_lines].join("\n");
        let d = Document::plain(&text, 10.5);
        let s = layout(&d, &m, &Frame { measure_mm: 170.0, line_height_mm: 6.4, y0_mm: 24.0 });
        let mut buf = Vec::new();
        to_pdf(&s, &data, Paper::default(), &mut buf).unwrap();
        // ページ数は PDF の /Count に書かれている
        let hay = String::from_utf8_lossy(&buf).to_string();
        let i = hay.find("/Count ").expect("/Count が無い") + 7;
        hay[i..].chars().take_while(|c| c.is_ascii_digit())
            .collect::<String>().parse().unwrap()
    }

    #[test]
    fn 長い文書は複数ページになる() {
        // A4 で本文が入るのは 40行くらい。100行が1ページに収まっていたら
        // それは「下へ黙ってはみ出している」ということ
        assert_eq!(pages(10), 1, "10行で複数ページになった");
        let p100 = pages(100);
        assert!(p100 >= 3, "100行が {p100} ページにしかならない(はみ出している)");
    }

    #[test]
    fn ページ数が行数に見合う() {
        // A4(y0=24, 下余白20)に入るのは約40行。100行なら3ページ
        let n = pages(100);
        assert!((3..=4).contains(&n), "100行が {n} ページ(40行/頁の見当と合わない)");
        assert!(pages(300) >= 8, "300行が {} ページ", pages(300));
    }
}

#[cfg(test)]
mod hf_tests {
    use kumihan::{font, layout, layout_hf, Document, Frame, HeadFoot, Metrics, PageSetup,
                  PAGE_MARK};

    use super::*;

    #[test]
    fn ページ番号が各ページに載りページ数は変わらない() {
        let (fam, _) = font::for_document(None).unwrap();
        let data = font::load(fam).unwrap();
        let m = Metrics::new(&data).unwrap();
        let text = vec!["行"; 100].join("\n");
        let d = Document::plain(&text, 10.5);
        let s = layout(&d, &m, &Frame { measure_mm: 170.0, line_height_mm: 6.4, y0_mm: 24.0 });
        let pg = PageSetup::default();
        let hf = HeadFoot {
            paragraphs: Document::plain(&PAGE_MARK.to_string(), 10.5)
                .paragraphs().cloned().collect(),
            part: None,
        };
        let mut buf = Vec::new();
        to_pdf_with(&s, &data, Paper::default(),
            |k| layout_hf(&hf, &m, &pg, 6.4, k, true), &mut buf).unwrap();
        assert_eq!(&buf[..5], b"%PDF-");
        // ページ数は本文で決まる(飾りで増えない)
        let hay = String::from_utf8_lossy(&buf).to_string();
        let i = hay.find("/Count ").unwrap() + 7;
        let n: usize = hay[i..].chars().take_while(|c| c.is_ascii_digit())
            .collect::<String>().parse().unwrap();
        assert!((3..=4).contains(&n), "{n} ページ");
    }
}

#[cfg(test)]
mod break_tests {
    use kumihan::{font, layout, Block, Document, Frame, Metrics};

    use super::*;

    #[test]
    fn 改ページで頁が割れる() {
        let (fam, _) = font::for_document(None).unwrap();
        let data = font::load(fam).unwrap();
        let m = Metrics::new(&data).unwrap();
        let mut d = Document::plain("一頁目\n二頁目", 10.5);
        if let Block::Para(p) = &mut d.blocks[1] {
            p.page_break_before = true;
        }
        let s = layout(&d, &m, &Frame { measure_mm: 170.0, line_height_mm: 6.4, y0_mm: 24.0 });
        assert_eq!(s.breaks.len(), 1, "改ページが紙面に伝わっていない");
        let mut buf = Vec::new();
        to_pdf(&s, &data, Paper::default(), &mut buf).unwrap();
        let hay = String::from_utf8_lossy(&buf).to_string();
        let i = hay.find("/Count ").unwrap() + 7;
        let n: usize = hay[i..].chars().take_while(|c| c.is_ascii_digit())
            .collect::<String>().parse().unwrap();
        assert_eq!(n, 2, "2行の文書が改ページで2頁にならず {n} 頁");
    }

    #[test]
    fn 先頭の改ページは頁を増やさない() {
        // 1段落目に改ページが付いていても、空の1頁目を作らない
        let (fam, _) = font::for_document(None).unwrap();
        let data = font::load(fam).unwrap();
        let m = Metrics::new(&data).unwrap();
        let mut d = Document::plain("本文", 10.5);
        if let Block::Para(p) = &mut d.blocks[0] {
            p.page_break_before = true;
        }
        let s = layout(&d, &m, &Frame { measure_mm: 170.0, line_height_mm: 6.4, y0_mm: 24.0 });
        assert!(s.breaks.is_empty(), "先頭で頁を割った");
    }
}

#[cfg(test)]
mod image_tests {
    use kumihan::{font, layout, Document, Frame, InlineImage, Metrics};

    use super::*;

    /// 本物の PNG をその場で作る(手打ちのバイト列は CRC を壊しやすい)
    fn png() -> Vec<u8> {
        let img = ::image::RgbImage::from_pixel(4, 4, ::image::Rgb([200, 30, 30]));
        let mut buf = std::io::Cursor::new(Vec::new());
        ::image::DynamicImage::ImageRgb8(img)
            .write_to(&mut buf, ::image::ImageFormat::Png)
            .unwrap();
        buf.into_inner()
    }

    #[test]
    fn 画像が紙に埋まる() {
        let (fam, _) = font::for_document(None).unwrap();
        let data = font::load(fam).unwrap();
        let m = Metrics::new(&data).unwrap();
        let mut d = Document::plain("写真の前\n写真の後", 10.5);
        if let kumihan::Block::Para(p) = &mut d.blocks[0] {
            p.images.push(InlineImage {
                bytes: std::sync::Arc::new(png()),
                w_mm: 40.0,
                h_mm: 30.0,
            });
        }
        let s = layout(&d, &m, &Frame { measure_mm: 170.0, line_height_mm: 6.4, y0_mm: 24.0 });
        assert_eq!(s.images.len(), 1, "紙面に画像が無い");
        let mut buf = Vec::new();
        to_pdf(&s, &data, Paper::default(), &mut buf).unwrap();
        let hay = String::from_utf8_lossy(&buf);
        // 画像は XObject として埋まる
        assert!(hay.contains("/XObject"), "PDF に画像が埋まっていない");
    }

    #[test]
    fn 壊れた画像は飛ばして紙は出来る() {
        // 1枚のために紙全体を失敗させない
        let (fam, _) = font::for_document(None).unwrap();
        let data = font::load(fam).unwrap();
        let m = Metrics::new(&data).unwrap();
        let mut d = Document::plain("本文", 10.5);
        if let kumihan::Block::Para(p) = &mut d.blocks[0] {
            p.images.push(InlineImage {
                bytes: std::sync::Arc::new(b"not an image".to_vec()),
                w_mm: 40.0,
                h_mm: 30.0,
            });
        }
        let s = layout(&d, &m, &Frame { measure_mm: 170.0, line_height_mm: 6.4, y0_mm: 24.0 });
        let mut buf = Vec::new();
        to_pdf(&s, &data, Paper::default(), &mut buf).unwrap();
        assert_eq!(&buf[..5], b"%PDF-");
    }
}

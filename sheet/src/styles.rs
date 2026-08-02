//! xlsx の `styles.xml` — セルの見た目。
//!
//! xlsx はセルに書式を直接書かない。`<c r="A1" s="3">` の `s` が
//! `styles.xml` の `cellXfs` の何番目か、という**索引**になっている。
//! だから読むときは表を先に作り、書くときは使った組み合わせを集めて表にする。
//!
//! **日本の帳票は罫線で出来ている**ので、ここは飾りではない。
//! 罫線を落とすと、書類として通らないものが出来上がる。

use std::collections::BTreeMap;

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::model::{Borders, CellFormat, HAlign};

fn local(n: &[u8]) -> &[u8] {
    match n.iter().position(|c| *c == b':') {
        Some(i) => &n[i + 1..],
        None => n,
    }
}

fn attr(e: &quick_xml::events::BytesStart, k: &str) -> Option<String> {
    e.attributes().flatten().find(|a| local(a.key.as_ref()) == k.as_bytes())
        .and_then(|a| a.unescape_value().ok().map(|v| v.to_string()))
}

/// `styles.xml` を読んで、索引 → 書式 の表にする。
pub fn parse(xml: &str) -> Vec<CellFormat> {
    let mut fonts: Vec<(bool, bool, bool, Option<String>)> = Vec::new();
    let mut fills: Vec<Option<String>> = Vec::new();
    let mut borders: Vec<Borders> = Vec::new();
    let mut numfmts: BTreeMap<u32, String> = BTreeMap::new();
    let mut xfs: Vec<CellFormat> = Vec::new();

    // いま何の中にいるか。同じ名前の要素が section ごとに意味を変えるため
    let (mut in_fonts, mut in_fills, mut in_borders, mut in_cellxfs) = (false, false, false, false);
    let mut font = (false, false, false, None::<String>);
    let mut fill: Option<String> = None;
    let mut bd = Borders::default();
    let mut side: Option<Vec<u8>> = None;
    let mut xf: Option<(usize, usize, usize, u32)> = None;

    let mut r = Reader::from_str(xml);
    let mut buf = Vec::new();
    loop {
        let ev = r.read_event_into(&mut buf);
        let (e, empty) = match &ev {
            Ok(Event::Start(e)) => (e.clone(), false),
            Ok(Event::Empty(e)) => (e.clone(), true),
            Ok(Event::End(e)) => {
                match local(e.name().as_ref()) {
                    b"fonts" => in_fonts = false,
                    b"fills" => in_fills = false,
                    b"borders" => in_borders = false,
                    b"cellXfs" => in_cellxfs = false,
                    b"font" if in_fonts => fonts.push(std::mem::take(&mut font)),
                    b"fill" if in_fills => fills.push(fill.take()),
                    b"border" if in_borders => borders.push(std::mem::take(&mut bd)),
                    b"xf" if in_cellxfs => {
                        if let Some(x) = xf.take() {
                            xfs.push(resolve(x, &fonts, &fills, &borders, &numfmts, None));
                        }
                    }
                    _ => {}
                }
                buf.clear();
                continue;
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {
                buf.clear();
                continue;
            }
        };
        let n = local(e.name().as_ref()).to_vec();
        match n.as_slice() {
            b"fonts" => in_fonts = true,
            b"fills" => in_fills = true,
            b"borders" => in_borders = true,
            b"cellXfs" => in_cellxfs = true,
            b"numFmt" => {
                if let (Some(id), Some(code)) = (attr(&e, "numFmtId"), attr(&e, "formatCode")) {
                    if let Ok(i) = id.parse() {
                        numfmts.insert(i, code);
                    }
                }
            }
            b"font" if in_fonts => {
                font = (false, false, false, None);
                if empty {
                    fonts.push(std::mem::take(&mut font));
                }
            }
            b"b" if in_fonts => font.0 = on(&e),
            b"i" if in_fonts => font.1 = on(&e),
            b"u" if in_fonts => font.2 = true,
            b"color" if in_fonts => font.3 = rgb(&e),
            b"fill" if in_fills => {
                fill = None;
                if empty {
                    fills.push(fill.take());
                }
            }
            // 塗りは patternFill > fgColor に入る
            b"fgColor" if in_fills => fill = rgb(&e),
            b"border" if in_borders => {
                bd = Borders::default();
                if empty {
                    borders.push(std::mem::take(&mut bd));
                }
            }
            b"left" | b"right" | b"top" | b"bottom" if in_borders => {
                // style 属性が無い/none のときは引かれていない
                let drawn = attr(&e, "style").map_or(false, |s| s != "none");
                match n.as_slice() {
                    b"left" => bd.left = drawn,
                    b"right" => bd.right = drawn,
                    b"top" => bd.top = drawn,
                    _ => bd.bottom = drawn,
                }
                side = Some(n.clone());
            }
            b"xf" if in_cellxfs => {
                let g = |k: &str| attr(&e, k).and_then(|v| v.parse().ok()).unwrap_or(0);
                let x = (g("fontId"), g("fillId"), g("borderId"), g("numFmtId") as u32);
                if empty {
                    xfs.push(resolve(x, &fonts, &fills, &borders, &numfmts, None));
                } else {
                    xf = Some(x);
                }
            }
            b"alignment" if in_cellxfs => {
                if let Some(x) = xf.take() {
                    let a = attr(&e, "horizontal").map(|v| HAlign::from_xlsx(&v));
                    xfs.push(resolve(x, &fonts, &fills, &borders, &numfmts, a));
                }
            }
            _ => {}
        }
        let _ = &side;
        buf.clear();
    }
    xfs
}

fn on(e: &quick_xml::events::BytesStart) -> bool {
    !matches!(attr(e, "val").as_deref(), Some("0") | Some("false"))
}

/// `rgb="FFRRGGBB"` から `RRGGBB` を取る(先頭2桁は不透明度)。
fn rgb(e: &quick_xml::events::BytesStart) -> Option<String> {
    let v = attr(e, "rgb")?;
    let s = if v.len() == 8 { &v[2..] } else { &v[..] };
    (s.len() == 6).then(|| s.to_uppercase())
}

fn resolve(
    (fid, fillid, bid, nfid): (usize, usize, usize, u32),
    fonts: &[(bool, bool, bool, Option<String>)],
    fills: &[Option<String>],
    borders: &[Borders],
    numfmts: &BTreeMap<u32, String>,
    align: Option<HAlign>,
) -> CellFormat {
    let f = fonts.get(fid).cloned().unwrap_or_default();
    CellFormat {
        bold: f.0,
        italic: f.1,
        underline: f.2,
        color: f.3,
        fill: fills.get(fillid).cloned().flatten(),
        borders: borders.get(bid).copied().unwrap_or_default(),
        align: align.unwrap_or_default(),
        number_format: numfmts.get(&nfid).cloned().or_else(|| builtin(nfid)),
    }
}

/// xlsx が番号だけで持っている既定の表示形式(よく使うものだけ)。
fn builtin(id: u32) -> Option<String> {
    Some(match id {
        1 => "0", 2 => "0.00", 3 => "#,##0", 4 => "#,##0.00",
        9 => "0%", 10 => "0.00%",
        14 => "yyyy/mm/dd", 20 => "h:mm", 22 => "yyyy/mm/dd h:mm",
        _ => return None,
    }.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r##"<styleSheet>
      <numFmts count="1"><numFmt numFmtId="176" formatCode="#,##0&quot;円&quot;"/></numFmts>
      <fonts count="2">
        <font><sz val="11"/><name val="ＭＳ Ｐゴシック"/></font>
        <font><b/><color rgb="FFFF0000"/><name val="ＭＳ Ｐゴシック"/></font>
      </fonts>
      <fills count="2">
        <fill><patternFill patternType="none"/></fill>
        <fill><patternFill patternType="solid"><fgColor rgb="FFFFFF00"/></patternFill></fill>
      </fills>
      <borders count="2">
        <border><left/><right/><top/><bottom/></border>
        <border><left style="thin"/><right style="thin"/><top style="thin"/><bottom style="thin"/></border>
      </borders>
      <cellXfs count="4">
        <xf fontId="0" fillId="0" borderId="0" numFmtId="0"/>
        <xf fontId="1" fillId="0" borderId="1" numFmtId="0"/>
        <xf fontId="0" fillId="1" borderId="1" numFmtId="176"/>
        <xf fontId="0" fillId="0" borderId="0" numFmtId="0"><alignment horizontal="center"/></xf>
      </cellXfs>
    </styleSheet>"##;

    #[test]
    fn 素の書式は何も付かない() {
        let x = parse(SAMPLE);
        assert!(x[0].is_plain(), "{:?}", x[0]);
    }

    #[test]
    fn 罫線を読める() {
        // 日本の帳票の本体。落とすと書類として通らない
        let x = parse(SAMPLE);
        assert_eq!(x[1].borders, Borders::ALL, "四方の罫線が読めない: {:?}", x[1].borders);
        assert_eq!(x[0].borders, Borders::NONE, "無い罫線を引いた");
    }

    #[test]
    fn 太字と文字色を読める() {
        let x = parse(SAMPLE);
        assert!(x[1].bold);
        assert_eq!(x[1].color.as_deref(), Some("FF0000"), "先頭の不透明度を落とせていない");
    }

    #[test]
    fn 塗りつぶしを読める() {
        let x = parse(SAMPLE);
        assert_eq!(x[2].fill.as_deref(), Some("FFFF00"));
        assert_eq!(x[0].fill, None, "patternType=none を色にした");
    }

    #[test]
    fn 表示形式を読める() {
        let x = parse(SAMPLE);
        assert_eq!(x[2].number_format.as_deref(), Some("#,##0\"円\""));
    }

    #[test]
    fn 既定の表示形式は番号から引く() {
        let x = parse(r##"<styleSheet><cellXfs><xf numFmtId="3"/></cellXfs></styleSheet>"##);
        assert_eq!(x[0].number_format.as_deref(), Some("#,##0"));
    }

    #[test]
    fn 揃えを読める() {
        let x = parse(SAMPLE);
        assert_eq!(x[3].align, HAlign::Center);
    }

    #[test]
    fn 壊れた入力でも落ちない() {
        for s in ["", "<styleSheet/>", "<x", "ぐちゃぐちゃ"] {
            let _ = parse(s);
        }
    }
}

/// 使われている書式を集めて `styles.xml` にする。
///
/// xlsx はセルに書式を直接書けないので、**使った組み合わせを表にして索引を配る**。
/// 索引は `write` 側で `<c s="…">` に入れる。
pub fn build(used: &[CellFormat]) -> (String, BTreeMap<CellFormat, usize>) {
    // 素の書式は必ず 0 番。xlsx はそれを前提にしている道具が多い
    let mut order: Vec<CellFormat> = vec![CellFormat::default()];
    for f in used {
        if !order.contains(f) {
            order.push(f.clone());
        }
    }
    let mut fonts: Vec<(bool, bool, bool, Option<String>)> = vec![(false, false, false, None)];
    let mut fills: Vec<Option<String>> = vec![None, None]; // 0=none 1=gray125 は予約席
    let mut borders: Vec<Borders> = vec![Borders::NONE];
    let mut numfmts: Vec<String> = Vec::new();
    let mut xfs: Vec<(usize, usize, usize, usize, HAlign)> = Vec::new();

    for f in &order {
        let font = (f.bold, f.italic, f.underline, f.color.clone());
        let fi = idx(&mut fonts, font);
        let fl = match &f.fill {
            Some(c) => idx(&mut fills, Some(c.clone())),
            None => 0,
        };
        let bi = idx(&mut borders, f.borders);
        let ni = match &f.number_format {
            Some(c) => {
                let p = idx(&mut numfmts, c.clone());
                164 + p // 164 未満は xlsx の予約
            }
            None => 0,
        };
        xfs.push((fi, fl, bi, ni, f.align));
    }

    let mut s = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#,
    );
    if !numfmts.is_empty() {
        s.push_str(&format!("<numFmts count=\"{}\">", numfmts.len()));
        for (i, c) in numfmts.iter().enumerate() {
            s.push_str(&format!(
                "<numFmt numFmtId=\"{}\" formatCode=\"{}\"/>",
                164 + i,
                esc(c)
            ));
        }
        s.push_str("</numFmts>");
    }
    s.push_str(&format!("<fonts count=\"{}\">", fonts.len()));
    for (b, i, u, c) in &fonts {
        s.push_str("<font><sz val=\"11\"/>");
        if *b { s.push_str("<b/>") }
        if *i { s.push_str("<i/>") }
        if *u { s.push_str("<u/>") }
        if let Some(c) = c { s.push_str(&format!("<color rgb=\"FF{c}\"/>")) }
        s.push_str("</font>");
    }
    s.push_str("</fonts>");
    s.push_str(&format!("<fills count=\"{}\">", fills.len()));
    for (i, f) in fills.iter().enumerate() {
        match f {
            Some(c) => s.push_str(&format!(
                "<fill><patternFill patternType=\"solid\"><fgColor rgb=\"FF{c}\"/>\
                 <bgColor indexed=\"64\"/></patternFill></fill>"
            )),
            None if i == 1 => s.push_str("<fill><patternFill patternType=\"gray125\"/></fill>"),
            None => s.push_str("<fill><patternFill patternType=\"none\"/></fill>"),
        }
    }
    s.push_str("</fills>");
    s.push_str(&format!("<borders count=\"{}\">", borders.len()));
    for b in &borders {
        s.push_str("<border>");
        for (on, tag) in [(b.left, "left"), (b.right, "right"), (b.top, "top"), (b.bottom, "bottom")] {
            if on {
                s.push_str(&format!("<{tag} style=\"thin\"><color indexed=\"64\"/></{tag}>"));
            } else {
                s.push_str(&format!("<{tag}/>"));
            }
        }
        s.push_str("<diagonal/></border>");
    }
    s.push_str("</borders>");
    s.push_str("<cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>");
    s.push_str(&format!("<cellXfs count=\"{}\">", xfs.len()));
    for (fi, fl, bi, ni, al) in &xfs {
        // applyX を付けないと読み手が無視することがある
        s.push_str(&format!(
            "<xf numFmtId=\"{ni}\" fontId=\"{fi}\" fillId=\"{fl}\" borderId=\"{bi}\" xfId=\"0\"\
             applyNumberFormat=\"1\" applyFont=\"1\" applyFill=\"1\" applyBorder=\"1\""
        ));
        match al.as_xlsx() {
            Some(a) => s.push_str(&format!(" applyAlignment=\"1\"><alignment horizontal=\"{a}\"/></xf>")),
            None => s.push_str("/>"),
        }
    }
    s.push_str("</cellXfs>");
    s.push_str("<cellStyles count=\"1\"><cellStyle name=\"標準\" xfId=\"0\" builtinId=\"0\"/></cellStyles>");
    s.push_str("</styleSheet>");

    let map = order.into_iter().enumerate().map(|(i, f)| (f, i)).collect();
    (s, map)
}

fn idx<T: PartialEq>(v: &mut Vec<T>, x: T) -> usize {
    match v.iter().position(|y| *y == x) {
        Some(i) => i,
        None => {
            v.push(x);
            v.len() - 1
        }
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod build_tests {
    use super::*;

    fn ruled() -> CellFormat {
        CellFormat { borders: Borders::ALL, bold: true, ..Default::default() }
    }

    #[test]
    fn 素の書式は0番() {
        let (_, map) = build(&[ruled()]);
        assert_eq!(map[&CellFormat::default()], 0, "素の書式が0番でない");
    }

    #[test]
    fn 書いたものを読み戻せる() {
        // 罫線を落とすと帳票として通らない。往復で守る
        let f = ruled();
        let (xml, map) = build(&[f.clone()]);
        let back = parse(&xml);
        let i = map[&f];
        assert_eq!(back[i].borders, Borders::ALL, "罫線が消えた: {:?}", back[i]);
        assert!(back[i].bold, "太字が消えた");
    }

    #[test]
    fn 塗りと色と表示形式が往復する() {
        let f = CellFormat {
            fill: Some("FFFF00".into()),
            color: Some("FF0000".into()),
            number_format: Some("#,##0".into()),
            align: HAlign::Center,
            ..Default::default()
        };
        let (xml, map) = build(&[f.clone()]);
        let back = &parse(&xml)[map[&f]];
        assert_eq!(back.fill.as_deref(), Some("FFFF00"));
        assert_eq!(back.color.as_deref(), Some("FF0000"));
        assert_eq!(back.number_format.as_deref(), Some("#,##0"));
        assert_eq!(back.align, HAlign::Center);
    }

    #[test]
    fn 同じ書式は1つにまとまる() {
        let f = ruled();
        let (_, map) = build(&[f.clone(), f.clone(), f.clone()]);
        assert_eq!(map.len(), 2, "素の書式 + 1 のはず: {map:?}");
    }

    #[test]
    fn 一部だけの罫線も往復する() {
        // 表の下線だけ、という帳票は多い
        let f = CellFormat {
            borders: Borders { bottom: true, ..Borders::NONE },
            ..Default::default()
        };
        let (xml, map) = build(&[f.clone()]);
        let back = &parse(&xml)[map[&f]];
        assert!(back.borders.bottom, "下線が消えた");
        assert!(!back.borders.top, "無い罫線が増えた");
    }
}

//! xlsx(SpreadsheetML)の読み書き。
//! 読めないものは黙って落とさず `Report` に積む(ooxml と同じ作法)。
use std::io::{Cursor, Read, Seek, Write};

use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};

use crate::model::{Book, Cell, Pos, Sheet, Value};

#[derive(Debug, Default, Clone)]
pub struct Report {
    pub unsupported: Vec<(String, usize)>,
    pub sheets: usize,
    pub cells: usize,
}
impl Report {
    fn note(&mut self, n: &str) {
        match self.unsupported.iter_mut().find(|(x, _)| x == n) {
            Some(e) => e.1 += 1,
            None => self.unsupported.push((n.to_string(), 1)),
        }
    }
    pub fn is_lossless(&self) -> bool { self.unsupported.is_empty() }
}

fn local(n: &[u8]) -> &[u8] {
    match n.iter().position(|b| *b == b':') { Some(i) => &n[i + 1..], None => n }
}
fn attr(e: &BytesStart, want: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        (local(a.key.as_ref()) == want.as_bytes())
            .then(|| String::from_utf8_lossy(&a.value).to_string())
    })
}

/// sharedStrings.xml → 文字列表
///
/// 日本語の xlsx には**ふりがな**(`<rPh>`)が入る。その中にも `<t>` があるので、
/// 素直に全部の `<t>` を拾うと「提案見積書テイアンミツモリショ」になる。
/// 欧米の実装が落としがちな箇所。ふりがなは本文ではないので混ぜない。
fn parse_shared(xml: &str) -> (Vec<String>, usize) {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(false);
    let (mut out, mut cur) = (Vec::new(), String::new());
    let (mut in_t, mut in_si, mut in_rph) = (false, false, false);
    let mut ruby = 0usize;
    let mut buf = Vec::new();
    loop {
        match r.read_event_into(&mut buf) {
            Ok(Event::Eof) | Err(_) => break,
            Ok(Event::Start(e)) => match local(e.name().as_ref()) {
                b"si" => { in_si = true; cur.clear() }
                b"rPh" => { in_rph = true; ruby += 1 }
                b"t" if in_si && !in_rph => in_t = true,
                _ => {}
            },
            Ok(Event::Text(t)) if in_t => cur.push_str(&t.unescape().unwrap_or_default()),
            Ok(Event::End(e)) => match local(e.name().as_ref()) {
                b"t" => in_t = false,
                b"rPh" => in_rph = false,
                b"si" => { in_si = false; out.push(std::mem::take(&mut cur)) }
                _ => {}
            },
            _ => {}
        }
        buf.clear();
    }
    (out, ruby)
}

/// `<mergeCell ref="A1:B2"/>` を結合として持つ(読み飛ばすと保存で消える)。
fn merge(e: &quick_xml::events::BytesStart, sh: &mut Sheet) {
    if let Some(r) = attr(e, "ref") {
        if let Some((a, b)) = r.split_once(':') {
            if let (Some(a), Some(b)) = (Pos::parse(a), Pos::parse(b)) {
                sh.merges.push((a, b));
            }
        }
    }
}

/// `<row r="3" ht="27.5" customHeight="1">` — 指定のある行だけ持つ。
fn row_height(e: &quick_xml::events::BytesStart, sh: &mut Sheet) {
    let custom = attr(e, "customHeight").as_deref() == Some("1");
    if !custom {
        return;
    }
    if let (Some(r), Some(h)) = (
        attr(e, "r").and_then(|v| v.parse::<u32>().ok()),
        attr(e, "ht").and_then(|v| v.parse::<f32>().ok()),
    ) {
        if r >= 1 {
            sh.row_height.insert(r - 1, h);
        }
    }
}

/// `<col min="1" max="3" width="12.5"/>` — min..=max は1始まり。
///
/// 全列に近い指定(既定幅)は展開しない。1列ずつに割ると
/// 16,384 個の col になって保存が肥大する。
fn col_width(e: &quick_xml::events::BytesStart, sh: &mut Sheet) {
    let g = |k: &str| attr(e, k).and_then(|v| v.parse::<f32>().ok());
    if let (Some(min), Some(max), Some(w)) = (g("min"), g("max"), g("width")) {
        if max - min > 1000.0 {
            sh.default_col_width = Some(w);
            return;
        }
        for c in (min as u32)..=(max as u32) {
            if c >= 1 {
                sh.col_width.insert(c - 1, w);
            }
        }
    }
}

/// styles.xml の dxfs(条件付き書式の見た目)→ (文字色, 塗り) の列。
fn parse_dxfs(xml: &str) -> Vec<(Option<String>, Option<String>)> {
    let mut r = Reader::from_str(xml);
    let mut out = Vec::new();
    let mut buf = Vec::new();
    let (mut in_dxfs, mut in_dxf, mut in_font, mut in_fill) = (false, false, false, false);
    let mut cur: (Option<String>, Option<String>) = (None, None);
    loop {
        match r.read_event_into(&mut buf) {
            Ok(Event::Eof) | Err(_) => break,
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match local(e.name().as_ref()) {
                b"dxfs" => in_dxfs = true,
                b"dxf" if in_dxfs => {
                    in_dxf = true;
                    cur = (None, None);
                }
                b"font" if in_dxf => in_font = true,
                b"fill" if in_dxf => in_fill = true,
                b"color" if in_font => {
                    cur.0 = attr(&e, "rgb").map(|v| v.trim_start_matches("FF").to_string());
                }
                b"bgColor" if in_fill => {
                    cur.1 = attr(&e, "rgb").map(|v| v.trim_start_matches("FF").to_string());
                }
                _ => {}
            },
            Ok(Event::End(e)) => match local(e.name().as_ref()) {
                b"dxfs" => in_dxfs = false,
                b"dxf" => {
                    if in_dxf {
                        out.push(std::mem::take(&mut cur));
                    }
                    in_dxf = false;
                }
                b"font" => in_font = false,
                b"fill" => in_fill = false,
                _ => {}
            },
            _ => {}
        }
        buf.clear();
    }
    out
}

/// _rels/*.rels → (Id, Type, Target, 外部か)
fn parse_rels(xml: &str) -> Vec<(String, String, String, bool)> {
    let mut r = Reader::from_str(xml);
    let mut out = Vec::new();
    let mut buf = Vec::new();
    loop {
        match r.read_event_into(&mut buf) {
            Ok(Event::Eof) | Err(_) => break,
            Ok(Event::Start(e)) | Ok(Event::Empty(e))
                if local(e.name().as_ref()) == b"Relationship" =>
            {
                out.push((
                    attr(&e, "Id").unwrap_or_default(),
                    attr(&e, "Type").unwrap_or_default(),
                    attr(&e, "Target").unwrap_or_default(),
                    attr(&e, "TargetMode").as_deref() == Some("External"),
                ));
            }
            _ => {}
        }
        buf.clear();
    }
    out
}

/// xl/worksheets/ からの相対の的を zip の中の道に直す("../comments1.xml" → "xl/comments1.xml")。
/// drawing の錨の中身(画像か図形か)。
enum DrawKind {
    /// 画像(r:embed)
    Image(String),
    /// 図形(prstGeom の名前, 塗り, 線, 中の文字, 折れ線の点)
    Shape(
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Vec<(f32, f32)>,
    ),
}

/// drawing(xl/drawings/drawingN.xml)から、画像と図形の錨を拾う。
/// 返すのは (置き場所のセル, 幅EMU, 高さEMU, 中身)。
fn parse_drawing_anchors(xml: &str) -> Vec<(Pos, i64, i64, DrawKind)> {
    let mut r = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut out = Vec::new();
    let (mut col, mut row) = (None::<u32>, None::<u32>);
    let (mut cx, mut cy) = (None::<i64>, None::<i64>);
    let mut embed = None::<String>;
    let mut prst = None::<String>;
    // 図形の色: solidFill の1つ目が塗り、a:ln の中のものが線
    let (mut fill, mut line) = (None::<String>, None::<String>);
    // 図形の中の文字(a:t)と、custGeom の折れ線
    let mut text = String::new();
    let mut in_t = false;
    let mut pts: Vec<(f32, f32)> = Vec::new();
    let (mut path_w, mut path_h) = (1000.0f32, 1000.0f32);
    let mut has_custom = false;
    let mut in_from = false;
    let mut in_ln = false;
    let mut in_sp = false;
    let mut cur: Vec<u8> = Vec::new();
    loop {
        match r.read_event_into(&mut buf) {
            Ok(Event::Eof) | Err(_) => break,
            Ok(Event::Start(e)) => match local(e.name().as_ref()) {
                b"oneCellAnchor" | b"twoCellAnchor" | b"absoluteAnchor" => {
                    (col, row, cx, cy, embed, prst, fill, line) =
                        (None, None, None, None, None, None, None, None);
                    text.clear();
                    pts.clear();
                    has_custom = false;
                    (path_w, path_h) = (1000.0, 1000.0);
                    in_sp = false;
                    in_ln = false;
                }
                b"from" => in_from = true,
                t @ (b"col" | b"row") if in_from => cur = t.to_vec(),
                b"sp" => in_sp = true,
                b"ln" => in_ln = true,
                b"blip" => {
                    if embed.is_none() {
                        embed = attr(&e, "embed");
                    }
                }
                b"prstGeom" => {
                    if prst.is_none() {
                        prst = attr(&e, "prst");
                    }
                }
                b"custGeom" => has_custom = true,
                b"path" if has_custom => {
                    path_w = attr(&e, "w").and_then(|v| v.parse().ok()).unwrap_or(1000.0);
                    path_h = attr(&e, "h").and_then(|v| v.parse().ok()).unwrap_or(1000.0);
                }
                b"t" if in_sp => in_t = true,
                _ => cur.clear(),
            },
            Ok(Event::Empty(e)) => match local(e.name().as_ref()) {
                b"ext" => {
                    if cx.is_none() {
                        cx = attr(&e, "cx").and_then(|v| v.parse().ok());
                        cy = attr(&e, "cy").and_then(|v| v.parse().ok());
                    }
                }
                b"blip" => {
                    if embed.is_none() {
                        embed = attr(&e, "embed");
                    }
                }
                b"pt" if has_custom => {
                    let x = attr(&e, "x").and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
                    let y = attr(&e, "y").and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
                    pts.push((x / path_w.max(1.0), y / path_h.max(1.0)));
                }
                b"srgbClr" if in_sp => {
                    let v = attr(&e, "val");
                    if in_ln {
                        if line.is_none() {
                            line = v;
                        }
                    } else if fill.is_none() {
                        fill = v;
                    }
                }
                _ => {}
            },
            Ok(Event::Text(t)) if in_t => {
                text.push_str(&t.unescape().unwrap_or_default());
            }
            Ok(Event::Text(t)) if !cur.is_empty() => {
                let v: u32 = t.unescape().unwrap_or_default().trim().parse().unwrap_or(0);
                if cur == b"col" {
                    col = Some(v);
                } else {
                    row = Some(v);
                }
            }
            Ok(Event::End(e)) => match local(e.name().as_ref()) {
                b"from" => {
                    in_from = false;
                    cur.clear();
                }
                b"col" | b"row" => cur.clear(),
                b"ln" => in_ln = false,
                b"t" => in_t = false,
                b"oneCellAnchor" | b"twoCellAnchor" | b"absoluteAnchor" => {
                    let tx = (!text.is_empty()).then(|| text.clone());
                    let kind = match (embed.take(), prst.take(), has_custom) {
                        (Some(em), _, _) => Some(DrawKind::Image(em)),
                        (None, Some(pr), _) => {
                            Some(DrawKind::Shape(pr, fill.take(), line.take(), tx, Vec::new()))
                        }
                        (None, None, true) if !pts.is_empty() => Some(DrawKind::Shape(
                            "spark".into(),
                            fill.take(),
                            line.take(),
                            tx,
                            std::mem::take(&mut pts),
                        )),
                        _ => None,
                    };
                    if let (Some(c), Some(rr), Some(k)) = (col, row, kind) {
                        out.push((
                            Pos::new(rr, c),
                            cx.unwrap_or(300 * 9525),
                            cy.unwrap_or(200 * 9525),
                            k,
                        ));
                    }
                }
                _ => {}
            },
            _ => {}
        }
        buf.clear();
    }
    out
}

/// 挿した図形1枚の錨(oneCellAnchor の xdr:sp)。Excel でも図形として開ける。
fn shape_anchor_xml(sp: &crate::model::SheetShape, id: u32) -> String {
    let (cx, cy) = ((sp.width_px * 9525.0) as i64, (sp.height_px * 9525.0) as i64);
    let fill = match &sp.fill {
        Some(c) => format!("<a:solidFill><a:srgbClr val=\"{c}\"/></a:solidFill>"),
        None => "<a:noFill/>".to_string(),
    };
    let line = match &sp.line {
        Some(c) => format!(
            "<a:ln w=\"19050\"><a:solidFill><a:srgbClr val=\"{c}\"/></a:solidFill></a:ln>"
        ),
        None => String::new(),
    };
    // 形: 折れ線(spark)は custGeom、他は prstGeom
    let geom = if sp.kind == "spark" && !sp.points.is_empty() {
        let mut path = String::new();
        for (i, (x, y)) in sp.points.iter().enumerate() {
            let (px_, py_) = ((x * 10000.0) as i64, (y * 10000.0) as i64);
            if i == 0 {
                path.push_str(&format!(
                    "<a:moveTo><a:pt x=\"{px_}\" y=\"{py_}\"/></a:moveTo>"
                ));
            } else {
                path.push_str(&format!(
                    "<a:lnTo><a:pt x=\"{px_}\" y=\"{py_}\"/></a:lnTo>"
                ));
            }
        }
        format!(
            concat!(
                "<a:custGeom><a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/>",
                "<a:rect l=\"0\" t=\"0\" r=\"0\" b=\"0\"/>",
                "<a:pathLst><a:path w=\"10000\" h=\"10000\" fill=\"none\">{}</a:path></a:pathLst>",
                "</a:custGeom>"
            ),
            path
        )
    } else {
        format!("<a:prstGeom prst=\"{}\"><a:avLst/></a:prstGeom>", sp.kind)
    };
    // 中の文字(テキストボックス)
    let txt = match &sp.text {
        Some(t) => format!(
            concat!(
                "<xdr:txBody><a:bodyPr wrap=\"square\"/><a:lstStyle/>",
                "<a:p><a:r><a:rPr lang=\"ja-JP\" sz=\"1100\"/><a:t>{}</a:t></a:r></a:p>",
                "</xdr:txBody>"
            ),
            esc(t)
        ),
        None => String::new(),
    };
    format!(
        concat!(
            "<xdr:oneCellAnchor>",
            "<xdr:from><xdr:col>{col}</xdr:col><xdr:colOff>0</xdr:colOff>",
            "<xdr:row>{row}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>",
            "<xdr:ext cx=\"{cx}\" cy=\"{cy}\"/>",
            "<xdr:sp macro=\"\" textlink=\"\">",
            "<xdr:nvSpPr><xdr:cNvPr id=\"{id}\" name=\"図形 {id}\"/><xdr:cNvSpPr/></xdr:nvSpPr>",
            "<xdr:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm>",
            "{geom}{fill}{line}</xdr:spPr>{txt}",
            "</xdr:sp><xdr:clientData/></xdr:oneCellAnchor>"
        ),
        col = sp.at.col,
        row = sp.at.row,
        cx = cx,
        cy = cy,
        id = id,
        geom = geom,
        fill = fill,
        line = line,
        txt = txt
    )
}

/// 挿した画像1枚の錨(oneCellAnchor)。大きさは px → EMU(9525 EMU = 1px)。
fn image_anchor_xml(im: &crate::model::SheetImage, rid: &str, id: u32) -> String {
    let (cx, cy) = ((im.width_px * 9525.0) as i64, (im.height_px * 9525.0) as i64);
    format!(
        concat!(
            "<xdr:oneCellAnchor>",
            "<xdr:from><xdr:col>{col}</xdr:col><xdr:colOff>0</xdr:colOff>",
            "<xdr:row>{row}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>",
            "<xdr:ext cx=\"{cx}\" cy=\"{cy}\"/>",
            "<xdr:pic><xdr:nvPicPr><xdr:cNvPr id=\"{id}\" name=\"画像 {id}\"/><xdr:cNvPicPr/></xdr:nvPicPr>",
            "<xdr:blipFill><a:blip r:embed=\"{rid}\"/><a:stretch><a:fillRect/></a:stretch></xdr:blipFill>",
            "<xdr:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm>",
            "<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></xdr:spPr></xdr:pic>",
            "<xdr:clientData/></xdr:oneCellAnchor>"
        ),
        col = im.at.col,
        row = im.at.row,
        cx = cx,
        cy = cy,
        id = id,
        rid = rid
    )
}

/// `_xlnm.Print_Titles` の行の部($1:$4)を(シート番号, (先頭行, 末尾行))に解く。
/// 列の繰り返し($A:$B)や混在は None(原文のまま持ち越す)。
fn parse_print_titles(raw: &str) -> Option<(usize, (u32, u32))> {
    let sid = raw
        .split(SID_ATTR)
        .nth(1)
        .and_then(|r| r.split('"').next())
        .and_then(|v| v.parse::<usize>().ok())?;
    let body = raw.split('>').nth(1).and_then(|r| r.split('<').next())?;
    let range = body.rsplit('!').next()?.replace('$', "");
    let (a, b) = range.split_once(':')?;
    let (a, b) = (a.trim().parse::<u32>().ok()?, b.trim().parse::<u32>().ok()?);
    if a == 0 || b == 0 {
        return None;
    }
    Some((sid, (a - 1, b - 1)))
}

fn resolve_target(t: &str) -> String {
    if let Some(rest) = t.strip_prefix("../") {
        format!("xl/{rest}")
    } else if let Some(rest) = t.strip_prefix('/') {
        rest.to_string()
    } else {
        format!("xl/worksheets/{t}")
    }
}

/// commentsN.xml → (セル参照, 本文) の列
fn parse_comments(xml: &str) -> Vec<(Pos, String)> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(false);
    let mut out = Vec::new();
    let mut buf = Vec::new();
    let mut cur: Option<Pos> = None;
    let mut text = String::new();
    let mut in_t = false;
    loop {
        match r.read_event_into(&mut buf) {
            Ok(Event::Eof) | Err(_) => break,
            Ok(Event::Start(e)) => match local(e.name().as_ref()) {
                b"comment" => {
                    cur = attr(&e, "ref").and_then(|s| Pos::parse(&s));
                    text.clear();
                }
                b"t" if cur.is_some() => in_t = true,
                _ => {}
            },
            Ok(Event::Text(t)) if in_t => text.push_str(&t.unescape().unwrap_or_default()),
            Ok(Event::End(e)) => match local(e.name().as_ref()) {
                b"t" => in_t = false,
                b"comment" => {
                    if let Some(p) = cur.take() {
                        out.push((p, std::mem::take(&mut text)));
                    }
                }
                _ => {}
            },
            _ => {}
        }
        buf.clear();
    }
    out
}

fn parse_sheet(xml: &str, shared: &[String], styles: &[crate::model::CellFormat],
               name: &str, rep: &mut Report) -> Sheet {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(false);
    let mut sh = Sheet::new(name);
    let (mut pos, mut ty) = (None::<Pos>, String::new());
    let (mut in_v, mut in_f, mut in_is) = (false, false, false);
    let (mut v, mut f) = (String::new(), String::new());
    let mut style: Option<usize> = None;
    let mut buf = Vec::new();
    loop {
        match r.read_event_into(&mut buf) {
            Ok(Event::Eof) | Err(_) => break,
            Ok(Event::Start(e)) => match local(e.name().as_ref()) {
                b"row" => row_height(&e, &mut sh),
                b"c" => {
                    pos = attr(&e, "r").and_then(|s| Pos::parse(&s));
                    ty = attr(&e, "t").unwrap_or_default();
                    // s は styles.xml の cellXfs の索引。書式はそちらにある
                    style = attr(&e, "s").and_then(|s| s.parse::<usize>().ok());
                    v.clear(); f.clear();
                }
                b"v" => in_v = true,
                b"f" => in_f = true,
                b"is" => in_is = true,
                b"mergeCell" => merge(&e, &mut sh),
                b"col" => col_width(&e, &mut sh),
                _ => {}
            },
            Ok(Event::Empty(e)) => match local(e.name().as_ref()) {
                b"col" => col_width(&e, &mut sh),
                b"c" => {
                    // 値の無いセル(書式だけ)。持たない
                    pos = None;
                }
                b"mergeCell" => merge(&e, &mut sh),
                // 印刷の設定。読むだけ(保存は原文持ち越しが正)— PDF が従う
                b"pageSetup" => {
                    sh.landscape = attr(&e, "orientation").as_deref() == Some("landscape");
                    sh.paper_size = attr(&e, "paperSize").and_then(|v| v.parse().ok());
                    sh.print_scale = attr(&e, "scale").and_then(|v| v.parse().ok());
                }
                b"printOptions" => {
                    let on = |k: &str| {
                        matches!(attr(&e, k).as_deref(), Some("1") | Some("true"))
                    };
                    sh.print_gridlines = on("gridLines");
                    sh.print_headings = on("headings");
                }
                b"brk" => {
                    if let Some(id) = attr(&e, "id").and_then(|v| v.parse().ok()) {
                        sh.row_breaks.push(id);
                    }
                }
                b"pageMargins" => {
                    let g = |k: &str| {
                        attr(&e, k).and_then(|v| v.parse::<f32>().ok()).map(|inch| inch * 25.4)
                    };
                    if let (Some(l), Some(r), Some(t), Some(b)) =
                        (g("left"), g("right"), g("top"), g("bottom"))
                    {
                        sh.margins_mm = Some((l, r, t, b));
                    }
                }
                _ => {}
            },
            Ok(Event::Text(t)) if in_v || in_f || in_is => {
                let s = t.unescape().unwrap_or_default();
                if in_f { f.push_str(&s) } else { v.push_str(&s) }
            }
            Ok(Event::End(e)) => match local(e.name().as_ref()) {
                b"v" => in_v = false,
                b"f" => in_f = false,
                b"is" => in_is = false,
                b"c" => {
                    if let Some(p) = pos.take() {
                        let value = match ty.as_str() {
                            "s" => v.trim().parse::<usize>().ok()
                                .and_then(|i| shared.get(i).cloned())
                                .map(Value::Text).unwrap_or(Value::Empty),
                            "b" => Value::Bool(v.trim() == "1"),
                            "e" => Value::Error(v.trim().to_string()),
                            "str" | "inlineStr" => Value::Text(v.trim().to_string()),
                            _ => match v.trim().parse::<f64>() {
                                Ok(n) => Value::Number(n),
                                Err(_) if v.trim().is_empty() => Value::Empty,
                                Err(_) => Value::Text(v.trim().to_string()),
                            },
                        };
                        let fmt = style
                            .and_then(|i| styles.get(i).cloned())
                            .unwrap_or_default();
                        let cell = Cell {
                            formula: (!f.is_empty()).then(|| f.clone()),
                            value,
                            fmt,
                        };
                        // **罫線だけのセル**も帳票では意味を持つので落とさない
                        if cell.formula.is_some() || !cell.value.is_empty()
                            || !cell.fmt.is_plain() {
                            rep.cells += 1;
                            sh.set(p, cell);
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
        buf.clear();
    }
    sh
}

pub fn read<R: Read + Seek>(src: R) -> Result<(Book, Report), String> {
    let mut zip = zip::ZipArchive::new(src).map_err(|e| format!("zipを開けません: {e}"))?;
    let mut rep = Report::default();

    // 書式表を先に読む。セルの s= はこの索引
    let mut styles: Vec<crate::model::CellFormat> = Vec::new();
    let mut dxfs: Vec<(Option<String>, Option<String>)> = Vec::new();
    if let Ok(mut f) = zip.by_name("xl/styles.xml") {
        let mut s = String::new();
        let _ = f.read_to_string(&mut s);
        styles = crate::styles::parse(&s);
        dxfs = parse_dxfs(&s);
    }

    let shared = {
        let mut s = String::new();
        match zip.by_name("xl/sharedStrings.xml") {
            Ok(mut f) => {
                let _ = f.read_to_string(&mut s);
                let (v, ruby) = parse_shared(&s);
                if ruby > 0 {
                    // ふりがなは読み飛ばした(本文には混ぜない)。持ち越しは K4 の課題
                    for _ in 0..ruby { rep.note("ふりがな(rPh。本文には混ぜず、保存時に落ちる)") }
                }
                v
            }
            Err(_) => Vec::new(),
        }
    };
    // シート名(workbook.xml の並び順)と、名前の定義
    let mut names = Vec::new();
    // (名前, 中身) — 中身は 'Sheet1'!$A$1:$B$2 の形
    let mut defined: Vec<(String, String)> = Vec::new();
    // 理解できなかった definedName の原文(hidden 属性つき等)。捨てない
    let mut defined_raw: Vec<String> = Vec::new();
    if let Ok(mut f) = zip.by_name("xl/workbook.xml") {
        let mut s = String::new();
        let _ = f.read_to_string(&mut s);
        let mut r = Reader::from_str(&s);
        let mut buf = Vec::new();
        let mut in_defined: Option<(String, bool, usize)> = None; // (name, 属性が単純か, 原文の頭)
        let mut text = String::new();
        let mut last = r.buffer_position() as usize;
        loop {
            let ev = r.read_event_into(&mut buf);
            let start_pos = last;
            last = r.buffer_position() as usize;
            match ev {
                Ok(Event::Eof) | Err(_) => break,
                Ok(Event::Start(e)) | Ok(Event::Empty(e))
                    if local(e.name().as_ref()) == b"sheet" =>
                {
                    names.push(attr(&e, "name").unwrap_or_else(|| "Sheet".into()));
                }
                Ok(Event::Start(e)) if local(e.name().as_ref()) == b"definedName" => {
                    // name= 以外の属性(hidden 等)が付いていたら「単純ではない」
                    let simple = e.attributes().flatten().count() == 1;
                    in_defined = Some((
                        attr(&e, "name").unwrap_or_default(),
                        simple,
                        start_pos,
                    ));
                    text.clear();
                }
                Ok(Event::Text(t)) if in_defined.is_some() => {
                    text.push_str(&t.unescape().unwrap_or_default());
                }
                Ok(Event::End(e)) if local(e.name().as_ref()) == b"definedName" => {
                    if let Some((nm, simple, at)) = in_defined.take() {
                        if simple {
                            defined.push((nm, std::mem::take(&mut text)));
                        } else {
                            defined_raw.push(s[at..last].to_string());
                        }
                    }
                }
                _ => {}
            }
            buf.clear();
        }
    }
    let paths: Vec<String> = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
        .filter(|n| n.starts_with("xl/worksheets/sheet") && n.ends_with(".xml"))
        .collect();
    let mut paths = paths;
    paths.sort();

    let mut book = Book { sheets: Vec::new(), names_raw: defined_raw, scripts: Vec::new() };
    for (i, path) in paths.iter().enumerate() {
        let mut s = String::new();
        if let Ok(mut f) = zip.by_name(path) { let _ = f.read_to_string(&mut s); }
        let name = names.get(i).cloned().unwrap_or_else(|| format!("Sheet{}", i + 1));
        let mut sh = parse_sheet(&s, &shared, &styles, &name, &mut rep);
        // このシートの rels(ハイパーリンクの先・コメントの部品への道)
        let rels_path = {
            let base = path.rsplit_once('/').map(|(_, b)| b).unwrap_or(path);
            format!("xl/worksheets/_rels/{base}.rels")
        };
        let mut rels = Vec::new();
        if let Ok(mut f) = zip.by_name(&rels_path) {
            let mut rs = String::new();
            let _ = f.read_to_string(&mut rs);
            rels = parse_rels(&rs);
        }
        // 条件付き書式。cellIs(値との比較)だけ理解し、他は報告
        {
            let mut r = Reader::from_str(&s);
            let mut buf = Vec::new();
            let mut sqref: Option<(Pos, Pos)> = None;
            let mut rule: Option<(String, Option<usize>)> = None; // (operator, dxfId)
            let mut in_formula = false;
            let mut formula = String::new();
            loop {
                match r.read_event_into(&mut buf) {
                    Ok(Event::Eof) | Err(_) => break,
                    Ok(Event::Start(e)) | Ok(Event::Empty(e))
                        if local(e.name().as_ref()) == b"conditionalFormatting" =>
                    {
                        sqref = attr(&e, "sqref").and_then(|v| {
                            let v = v.split_whitespace().next()?.to_string();
                            match v.split_once(':') {
                                Some((a, b)) => Some((Pos::parse(a)?, Pos::parse(b)?)),
                                None => {
                                    let p = Pos::parse(&v)?;
                                    Some((p, p))
                                }
                            }
                        });
                    }
                    Ok(Event::Start(e)) if local(e.name().as_ref()) == b"cfRule" => {
                        if attr(&e, "type").as_deref() == Some("cellIs") {
                            rule = Some((
                                attr(&e, "operator").unwrap_or_default(),
                                attr(&e, "dxfId").and_then(|v| v.parse().ok()),
                            ));
                        } else {
                            rep.note("条件付き書式(cellIs 以外。保存で失われる)");
                            rule = None;
                        }
                        formula.clear();
                    }
                    Ok(Event::Start(e)) if local(e.name().as_ref()) == b"formula" => {
                        in_formula = true;
                    }
                    Ok(Event::Text(t)) if in_formula => {
                        formula.push_str(&t.unescape().unwrap_or_default());
                    }
                    Ok(Event::End(e)) => match local(e.name().as_ref()) {
                        b"formula" => in_formula = false,
                        b"cfRule" => {
                            if let (Some(range), Some((op_s, dxf))) = (sqref, rule.take()) {
                                match (
                                    crate::model::CondOp::from_xlsx(&op_s),
                                    formula.trim().parse::<f64>(),
                                ) {
                                    (Some(op), Ok(value)) => {
                                        let (color, fill) = dxf
                                            .and_then(|i| dxfs.get(i).cloned())
                                            .unwrap_or((None, None));
                                        sh.cond.push(crate::model::CondRule {
                                            range, op, value, color, fill,
                                        });
                                    }
                                    _ => rep.note(
                                        "条件付き書式(値との比較以外。保存で失われる)",
                                    ),
                                }
                            }
                        }
                        b"conditionalFormatting" => sqref = None,
                        _ => {}
                    },
                    _ => {}
                }
                buf.clear();
            }
        }
        // データの入力規則。list(候補から選ぶ)だけ理解し、他は報告
        {
            let mut r = Reader::from_str(&s);
            let mut buf = Vec::new();
            // (sqref の原文, list か)。formula1 は子要素なので End まで貯める
            let mut dv: Option<(String, bool)> = None;
            let mut in_f1 = false;
            let mut f1 = String::new();
            loop {
                match r.read_event_into(&mut buf) {
                    Ok(Event::Eof) | Err(_) => break,
                    Ok(Event::Start(e)) if local(e.name().as_ref()) == b"dataValidation" => {
                        let is_list = attr(&e, "type").as_deref() == Some("list");
                        if !is_list {
                            rep.note("入力規則(list 以外。保存で失われる)");
                        }
                        dv = attr(&e, "sqref").map(|sq| (sq, is_list));
                        f1.clear();
                    }
                    // 自己閉じは formula1 を持てない = list として成立しない
                    Ok(Event::Empty(e)) if local(e.name().as_ref()) == b"dataValidation" => {
                        rep.note("入力規則(候補が無い。保存で失われる)");
                    }
                    Ok(Event::Start(e)) if local(e.name().as_ref()) == b"formula1" => {
                        in_f1 = true;
                    }
                    Ok(Event::Text(t)) if in_f1 => {
                        f1.push_str(&t.unescape().unwrap_or_default());
                    }
                    Ok(Event::End(e)) => match local(e.name().as_ref()) {
                        b"formula1" => in_f1 = false,
                        b"dataValidation" => {
                            if let Some((sq, true)) = dv.take() {
                                // sqref は空白区切りで複数の範囲を持てる
                                for part in sq.split_whitespace() {
                                    let range = match part.split_once(':') {
                                        Some((a, b)) => Pos::parse(a).zip(Pos::parse(b)),
                                        None => Pos::parse(part).map(|p| (p, p)),
                                    };
                                    if let Some(range) = range {
                                        sh.validations.push(crate::model::Validation {
                                            range,
                                            formula: f1.trim().to_string(),
                                        });
                                    }
                                }
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
                buf.clear();
            }
        }
        // ハイパーリンク。r:id の付いた外部URLだけ理解し、文書内の場所は報告
        {
            let mut r = Reader::from_str(&s);
            let mut buf = Vec::new();
            loop {
                match r.read_event_into(&mut buf) {
                    Ok(Event::Eof) | Err(_) => break,
                    Ok(Event::Start(e)) | Ok(Event::Empty(e))
                        if local(e.name().as_ref()) == b"hyperlink" =>
                    {
                        let p = attr(&e, "ref").and_then(|v| Pos::parse(&v));
                        let rid = attr(&e, "id");
                        match (p, rid) {
                            (Some(p), Some(rid)) => {
                                if let Some((_, _, target, _)) = rels
                                    .iter()
                                    .find(|(id, ty, _, ext)| {
                                        *id == rid && ty.ends_with("/hyperlink") && *ext
                                    })
                                {
                                    sh.links.insert(p, target.clone());
                                }
                            }
                            _ => rep.note("ハイパーリンク(文書内の場所。保存で失われる)"),
                        }
                    }
                    _ => {}
                }
                buf.clear();
            }
        }
        // コメント(commentsN.xml)。rels の type で結ばれている
        if let Some((_, _, target, _)) =
            rels.iter().find(|(_, ty, _, _)| ty.ends_with("/comments"))
        {
            if let Ok(mut f) = zip.by_name(&resolve_target(target)) {
                let mut cs = String::new();
                let _ = f.read_to_string(&mut cs);
                for (p, t) in parse_comments(&cs) {
                    sh.comments.insert(p, t);
                }
            }
        }
        // 画像(drawing)。**表示のために**読む — 保存は原文の持ち越しが担うので、
        // ここで読んだ絵を書き直すことはしない(図形など理解しない部品を壊さない)
        if let Some((_, _, target, _)) =
            rels.iter().find(|(_, ty, _, _)| ty.ends_with("/drawing"))
        {
            let dpath = resolve_target(target);
            let mut dx = String::new();
            if let Ok(mut f) = zip.by_name(&dpath) {
                let _ = f.read_to_string(&mut dx);
            }
            let drels = {
                let (dir, base) = dpath.rsplit_once('/').unwrap_or(("xl/drawings", &dpath));
                format!("{dir}/_rels/{base}.rels")
            };
            let mut rx = String::new();
            if let Ok(mut f) = zip.by_name(&drels) {
                let _ = f.read_to_string(&mut rx);
            }
            let dmap = parse_rels(&rx);
            for (at, cx_emu, cy_emu, kind) in parse_drawing_anchors(&dx) {
                let (width_px, height_px) =
                    (cx_emu as f32 / 9525.0, cy_emu as f32 / 9525.0);
                match kind {
                    DrawKind::Image(embed) => {
                        let Some((_, _, t, _)) =
                            dmap.iter().find(|(id, _, _, _)| *id == embed)
                        else {
                            rep.note("画像(実体への参照が無い)");
                            continue;
                        };
                        let mpath = resolve_target(t);
                        let mut data = Vec::new();
                        if let Ok(mut f) = zip.by_name(&mpath) {
                            let _ = f.read_to_end(&mut data);
                        }
                        if data.is_empty() {
                            rep.note("画像(実体が見つからない)");
                            continue;
                        }
                        sh.images.push(crate::model::SheetImage {
                            at,
                            width_px,
                            height_px,
                            data,
                        });
                    }
                    DrawKind::Shape(prst, fill, line, text, points) => {
                        sh.shapes.push(crate::model::SheetShape {
                            at,
                            width_px,
                            height_px,
                            kind: prst,
                            fill,
                            line,
                            text,
                            points,
                        });
                    }
                }
            }
        }
        book.sheets.push(sh);
        rep.sheets += 1;
    }
    if book.sheets.is_empty() {
        return Err("worksheet がありません(xlsxではない可能性)".into());
    }
    // 名前の定義をシートへ配る。'Sheet1'!$A$1:$B$2 の形だけ理解し、
    // それ以外(複数範囲・行列全体・_xlnm 系)は原文のまま持ち越す
    for (nm, target) in defined {
        match split_defined(&target) {
            Some((sheet_name, r)) => {
                match book.sheets.iter_mut().find(|s| s.name == sheet_name) {
                    Some(sh) => sh.names.push((nm, r)),
                    None => book.names_raw.push(format!(
                        "<definedName name=\"{}\">{}</definedName>",
                        esc(&nm),
                        esc(&target)
                    )),
                }
            }
            None => book.names_raw.push(format!(
                "<definedName name=\"{}\">{}</definedName>",
                esc(&nm),
                esc(&target)
            )),
        }
    }
    // ブックに載せた Python(独自部品 xl/joPython.xml)。**読むだけで実行しない**
    {
        let mut sx = String::new();
        if let Ok(mut f) = zip.by_name("xl/joPython.xml") {
            let _ = f.read_to_string(&mut sx);
        }
        if !sx.is_empty() {
            let mut r = Reader::from_str(&sx);
            let mut buf = Vec::new();
            let mut name = None::<String>;
            let mut code = String::new();
            let mut in_s = false;
            loop {
                match r.read_event_into(&mut buf) {
                    Ok(Event::Eof) | Err(_) => break,
                    Ok(Event::Start(e)) if local(e.name().as_ref()) == b"script" => {
                        name = attr(&e, "name");
                        code.clear();
                        in_s = true;
                    }
                    Ok(Event::Text(t)) if in_s => {
                        code.push_str(&t.unescape().unwrap_or_default());
                    }
                    Ok(Event::End(e)) if local(e.name().as_ref()) == b"script" => {
                        if let Some(n) = name.take() {
                            book.scripts.push((n, code.clone()));
                        }
                        in_s = false;
                    }
                    _ => {}
                }
                buf.clear();
            }
        }
    }
    // 印刷範囲は編集の対象なのでモデルへ(他の definedName は原文のまま)。
    // 読めない形だけ原文に残す — 黙って捨てない
    let mut rest = Vec::new();
    for raw in std::mem::take(&mut book.names_raw) {
        if raw.contains("_xlnm.Print_Area") {
            if let Some((sid, areas)) = parse_print_area(&raw) {
                if let Some(sh) = book.sheets.get_mut(sid) {
                    sh.print_areas.extend(areas);
                    continue;
                }
            }
        }
        if raw.contains("_xlnm.Print_Titles") {
            // 行の部($1:$4)だけ読む。列の繰り返しはまだ(原文のまま残す)
            if let Some((sid, rows)) = parse_print_titles(&raw) {
                if let Some(sh) = book.sheets.get_mut(sid) {
                    sh.print_title_rows = Some(rows);
                    continue;
                }
            }
        }
        rest.push(raw);
    }
    book.names_raw = rest;
    Ok((book, rep))
}

/// `_xlnm.Print_Area` の definedName を(シート番号, 範囲の列)に解く。
/// `,` 区切りの複数の域も受ける。読めなければ None。
fn parse_print_area(raw: &str) -> Option<(usize, Vec<(Pos, Pos)>)> {
    let sid = raw
        .split(SID_ATTR)
        .nth(1)
        .and_then(|r| r.split('"').next())
        .and_then(|v| v.parse::<usize>().ok())?;
    let body = raw.split('>').nth(1).and_then(|r| r.split('<').next())?;
    let mut out = Vec::new();
    for part in body.split(',') {
        let range = part.rsplit('!').next().unwrap_or(part);
        let parsed = match range.split_once(':') {
            Some((x, y)) => Pos::parse(x).zip(Pos::parse(y)),
            None => Pos::parse(range).map(|p| (p, p)),
        };
        out.push(parsed?);
    }
    if out.is_empty() {
        return None;
    }
    Some((sid, out))
}

/// localSheetId 属性の頭(引用符の入れ子を避けるため定数で持つ)
const SID_ATTR: &str = "localSheetId=\"";

// ---------- 書く ----------

const CT: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>__SHEETS__<Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/></Types>"#;

const RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;

const NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const RNS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
     .replace('"', "&quot;")
}

/// definedName の中身を (シート名, "A1" か "A1:B2") に分ける。
/// 'Sheet 1'!$A$1 の引用も解く。理解できない形なら None(原文で持ち越す側)。
fn split_defined(target: &str) -> Option<(String, String)> {
    let (sheet, r) = target.split_once('!')?;
    let sheet = sheet.trim();
    let sheet = if let Some(q) = sheet.strip_prefix('\'') {
        q.strip_suffix('\'')?.replace("''", "'")
    } else {
        sheet.to_string()
    };
    let plain: String = r.chars().filter(|c| *c != '$').collect();
    // A1 か A1:B2 の形だけ。複数範囲(カンマ)や行・列全体は理解しない
    let ok = match plain.split_once(':') {
        Some((a, b)) => Pos::parse(a).is_some() && Pos::parse(b).is_some(),
        None => Pos::parse(&plain).is_some(),
    };
    ok.then_some((sheet, plain))
}

/// "A1" / "A1:B2" → "$A$1" / "$A$1:$B$2"
fn dollars(r: &str) -> String {
    let one = |s: &str| -> String {
        let split = s.find(|c: char| c.is_ascii_digit()).unwrap_or(s.len());
        let (c, n) = s.split_at(split);
        format!("${c}${n}")
    };
    match r.split_once(':') {
        Some((a, b)) => format!("{}:{}", one(a), one(b)),
        None => one(r),
    }
}

/// 原本の workbook.xml の definedNames を、こちらの塊に置き換える。
fn patch_defined_names(workbook: &str, block: &str) -> String {
    let mut s = workbook.to_string();
    if let Some(i) = s.find("<definedNames>") {
        if let Some(j) = s[i..].find("</definedNames>") {
            s.replace_range(i..i + j + "</definedNames>".len(), "");
        }
    } else if let Some(i) = s.find("<definedNames/>") {
        s.replace_range(i..i + "<definedNames/>".len(), "");
    }
    if block.is_empty() {
        return s;
    }
    // 位置は sheets の直後(スキーマの並び)
    match s.find("</sheets>") {
        Some(i) => {
            let at = i + "</sheets>".len();
            s.insert_str(at, block);
            s
        }
        None => s,
    }
}

/// A1 を絶対参照($A$1)にする。Print_Area は絶対参照で書くのが通り相場。
fn abs_a1(p: Pos) -> String {
    let a1 = p.a1();
    let split = a1.find(|c: char| c.is_ascii_digit()).unwrap_or(0);
    format!("${}${}", &a1[..split], &a1[split..])
}

/// 全シートの名前の定義 + 印刷範囲 + 理解しなかった原文を definedNames の塊にする。
fn defined_names_xml(book: &Book) -> String {
    let mut inner = String::new();
    for raw in &book.names_raw {
        inner.push_str(raw);
    }
    // タイトル行(モデルが正)
    for (i, sh) in book.sheets.iter().enumerate() {
        if let Some((a, b)) = sh.print_title_rows {
            inner.push_str(&format!(
                "<definedName name=\"_xlnm.Print_Titles\" localSheetId=\"{i}\">{}</definedName>",
                esc(&format!("'{}'!${}:${}", sh.name.replace('\'', "''"), a + 1, b + 1))
            ));
        }
    }
    // 印刷範囲(モデルが正)。シート名は常に引用符で包む(空白・記号に安全)
    for (i, sh) in book.sheets.iter().enumerate() {
        if sh.print_areas.is_empty() {
            continue;
        }
        let refs: Vec<String> = sh
            .print_areas
            .iter()
            .map(|(a, b)| {
                format!("'{}'!{}:{}", sh.name.replace('\'', "''"), abs_a1(*a), abs_a1(*b))
            })
            .collect();
        inner.push_str(&format!(
            "<definedName name=\"_xlnm.Print_Area\" localSheetId=\"{i}\">{}</definedName>",
            esc(&refs.join(","))
        ));
    }
    for s in &book.sheets {
        for (n, r) in &s.names {
            inner.push_str(&format!(
                "<definedName name=\"{}\">'{}'!{}</definedName>",
                esc(n),
                s.name.replace('\'', "''"),
                dollars(r)
            ));
        }
    }
    if inner.is_empty() {
        String::new()
    } else {
        format!("<definedNames>{inner}</definedNames>")
    }
}

/// 自己閉じ要素の属性を差し替える(無ければ足す)。他の属性は触らない。
fn set_attr(el: &str, name: &str, value: &str) -> String {
    let pat = format!("{name}=\"");
    if let Some(i) = el.find(&pat) {
        let vstart = i + pat.len();
        if let Some(vend) = el[vstart..].find('"') {
            let mut out = String::with_capacity(el.len() + value.len());
            out.push_str(&el[..vstart]);
            out.push_str(value);
            out.push_str(&el[vstart + vend..]);
            return out;
        }
    }
    el.replacen("/>", &format!(" {name}=\"{value}\"/>"), 1)
}

/// 印刷まわりの塊(pageMargins → pageSetup → drawing の順 = schema の順)。
/// 原文があれば**属性だけ差し替える**(拡大縮小など知らない属性を残す)。
/// 無ければモデルの値から最小の要素を作る。
fn print_extra_xml(orig: &str, sh: &Sheet) -> String {
    let take = |pat: &str| -> Option<String> {
        let i = orig.find(pat)?;
        let j = orig[i..].find("/>")? + i + 2;
        Some(orig[i..j].to_string())
    };
    let inch = |mm: f32| format!("{:.5}", mm / 25.4);
    // printOptions(枠線・見出しの印刷)。モデルの真偽を原文へ織り込む
    let popts = {
        let el = take("<printOptions").unwrap_or_else(|| "<printOptions/>".to_string());
        let el = set_attr(&el, "gridLines", if sh.print_gridlines { "1" } else { "0" });
        let el = set_attr(&el, "headings", if sh.print_headings { "1" } else { "0" });
        if !sh.print_gridlines && !sh.print_headings && !orig.contains("<printOptions") {
            None
        } else {
            Some(el)
        }
    };
    let margins = match (sh.margins_mm, take("<pageMargins")) {
        (Some((l, r, t, b)), Some(el)) => {
            let el = set_attr(&el, "left", &inch(l));
            let el = set_attr(&el, "right", &inch(r));
            let el = set_attr(&el, "top", &inch(t));
            Some(set_attr(&el, "bottom", &inch(b)))
        }
        (Some((l, r, t, b)), None) => Some(format!(
            "<pageMargins left=\"{}\" right=\"{}\" top=\"{}\" bottom=\"{}\" header=\"0.3\" footer=\"0.3\"/>",
            inch(l), inch(r), inch(t), inch(b)
        )),
        (None, el) => el,
    };
    let setup = {
        let orig_el = take("<pageSetup");
        if !sh.landscape && sh.paper_size.is_none() && sh.print_scale.is_none()
            && orig_el.is_none()
        {
            None
        } else {
            let el = orig_el.unwrap_or_else(|| "<pageSetup/>".to_string());
            let el = set_attr(
                &el,
                "orientation",
                if sh.landscape { "landscape" } else { "portrait" },
            );
            let el = match sh.paper_size {
                Some(c) => set_attr(&el, "paperSize", &c.to_string()),
                None => el,
            };
            Some(match sh.print_scale {
                Some(sc) => set_attr(&el, "scale", &sc.to_string()),
                None => el,
            })
        }
    };
    let mut out = String::new();
    if let Some(po) = popts {
        out.push_str(&po);
    }
    if let Some(m) = margins {
        out.push_str(&m);
    }
    if let Some(su) = setup {
        out.push_str(&su);
    }
    // 改ページ(モデルが正。原文の rowBreaks は読みでモデルへ入っている)
    if !sh.row_breaks.is_empty() {
        let mut sorted = sh.row_breaks.clone();
        sorted.sort_unstable();
        sorted.dedup();
        out.push_str(&format!(
            r#"<rowBreaks count="{}" manualBreakCount="{}">"#,
            sorted.len(),
            sorted.len()
        ));
        for r in sorted {
            out.push_str(&format!(r#"<brk id="{r}" max="16383" man="1"/>"#));
        }
        out.push_str("</rowBreaks>");
    }
    if let Some(d) = take("<drawing") {
        out.push_str(&d);
    }
    out
}

pub fn write<W: Write + Seek>(book: &Book, dst: W) -> Result<(), String> {
    write_with(book, None::<std::io::Cursor<Vec<u8>>>, dst)
}

/// 保存する。`original` に開いた元のファイルを渡すと、こちらが作り直す部品
/// (シート・共有文字列・書式)以外 — **図形・テーマ・印刷設定・文書情報** —
/// を原本から持ち越す。渡さないと消える。
///
/// calcChain.xml だけは意図して捨てる(位置が古いままだと Excel が
/// 誤った計算順で開くことがある。無ければ Excel が作り直す)。
pub fn write_with<R: Read + Seek, W: Write + Seek>(
    book: &Book,
    original: Option<R>,
    dst: W,
) -> Result<(), String> {
    // 原本の部品と、各シートの引き継ぎ要素(印刷まわり・図形)を先に読む
    let mut carried: Vec<(String, Vec<u8>)> = Vec::new();
    let mut sheet_extras: Vec<String> = Vec::new();
    // [Content_Types] とシートの rels は「そのまま」ではなく、
    // リンク・コメントのぶんを織り込んで作り直す
    let mut orig_ct: Option<String> = None;
    let mut orig_sheet_rels: Vec<Option<String>> = Vec::new();
    if let Some(src) = original {
        if let Ok(mut z) = zip::ZipArchive::new(src) {
            for i in 0..z.len() {
                let Ok(mut f) = z.by_index(i) else { continue };
                let name = f.name().to_string();
                let regenerated = name.starts_with("xl/worksheets/sheet")
                    && name.ends_with(".xml")
                    || name == "xl/sharedStrings.xml"
                    || name == "xl/styles.xml"
                    || name == "xl/calcChain.xml"
                    // コメントの部品はこちらが作り直す
                    || name.starts_with("xl/comments")
                    || name.starts_with("xl/drawings/vmlDrawing");
                let mut buf = Vec::new();
                if f.read_to_end(&mut buf).is_err() {
                    continue;
                }
                if name.starts_with("xl/worksheets/sheet") && name.ends_with(".xml") {
                    // シート本体は作り直すが、印刷まわりと図形の参照は引き継ぐ
                    let s = String::from_utf8_lossy(&buf);
                    let mut extra = String::new();
                    for pat in ["<printOptions", "<pageMargins", "<pageSetup", "<drawing"] {
                        if let Some(i) = s.find(pat) {
                            if let Some(j) = s[i..].find("/>") {
                                extra.push_str(&s[i..i + j + 2]);
                            }
                        }
                    }
                    let n: usize = name["xl/worksheets/sheet".len()..name.len() - 4]
                        .parse()
                        .unwrap_or(0);
                    while sheet_extras.len() < n {
                        sheet_extras.push(String::new());
                    }
                    if n >= 1 {
                        sheet_extras[n - 1] = extra;
                    }
                }
                if name == "xl/workbook.xml" {
                    // 名前の定義はこちらの帳簿(モデル+原文持ち越し)が正。
                    // 原本の definedNames を置き換えて持ち越す
                    let s = String::from_utf8_lossy(&buf).to_string();
                    let patched = patch_defined_names(&s, &defined_names_xml(book));
                    carried.push((name, patched.into_bytes()));
                    continue;
                }
                if name == "[Content_Types].xml" {
                    orig_ct = Some(String::from_utf8_lossy(&buf).to_string());
                    continue;
                }
                if let Some(n) = name
                    .strip_prefix("xl/worksheets/_rels/sheet")
                    .and_then(|r| r.strip_suffix(".xml.rels"))
                    .and_then(|n| n.parse::<usize>().ok())
                {
                    while orig_sheet_rels.len() < n {
                        orig_sheet_rels.push(None);
                    }
                    orig_sheet_rels[n - 1] = Some(String::from_utf8_lossy(&buf).to_string());
                    continue;
                }
                if !regenerated {
                    carried.push((name, buf));
                }
            }
        }
    }

    let mut zip = zip::ZipWriter::new(dst);
    let o: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // 共有文字列を集める
    let mut shared: Vec<String> = Vec::new();
    let mut idx = std::collections::HashMap::new();
    for sh in &book.sheets {
        for c in sh.cells.values() {
            if let Value::Text(t) = &c.value {
                if !idx.contains_key(t) {
                    idx.insert(t.clone(), shared.len());
                    shared.push(t.clone());
                }
            }
        }
    }

    // 使われている書式を集めて表にする。索引を <c s="…"> に配る
    let used: Vec<crate::model::CellFormat> = {
        let mut v = Vec::new();
        for sh in &book.sheets {
            for c in sh.cells.values() {
                if !c.fmt.is_plain() && !v.contains(&c.fmt) {
                    v.push(c.fmt.clone());
                }
            }
        }
        v
    };
    let (styles_xml, style_idx) = crate::styles::build(&used);
    // 条件付き書式の見た目(dxfs)。全シートの規則から集めて番号を振る
    let dxf_list: Vec<(Option<String>, Option<String>)> = {
        let mut v = Vec::new();
        for sh in &book.sheets {
            for r in &sh.cond {
                let pair = (r.color.clone(), r.fill.clone());
                if !v.contains(&pair) {
                    v.push(pair);
                }
            }
        }
        v
    };
    let styles_xml = if dxf_list.is_empty() {
        styles_xml
    } else {
        let mut dx = format!("<dxfs count=\"{}\">", dxf_list.len());
        for (color, fill) in &dxf_list {
            dx.push_str("<dxf>");
            if let Some(c) = color {
                dx.push_str(&format!("<font><color rgb=\"FF{c}\"/></font>"));
            }
            if let Some(f) = fill {
                dx.push_str(&format!(
                    "<fill><patternFill><bgColor rgb=\"FF{f}\"/></patternFill></fill>"
                ));
            }
            dx.push_str("</dxf>");
        }
        dx.push_str("</dxfs>");
        let mut s = styles_xml;
        if let Some(p) = s.rfind("</styleSheet>") {
            s.insert_str(p, &dx);
        }
        s
    };

    let overrides: String = (1..=book.sheets.len())
        .map(|i| format!(r#"<Override PartName="/xl/worksheets/sheet{i}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#))
        .collect();
    // このアプリで挿した画像(グラフ)の部品。原本に drawing のあるシートは
    // **その部品の中へ錨と rels を継ぎ足す**(drawing は1シート1部品の決まり)。
    // 無いシートは drawingC{N}.xml を新しく作る
    let mut media_out: Vec<(String, Vec<u8>)> = Vec::new();
    let mut fresh_parts: Vec<(String, String)> = Vec::new();
    // 連番は原本にある imageC の続きから(前の保存で足した分と衝突しない)
    let mut media_n = 0usize;
    for (name, _) in &carried {
        if let Some(rest) = name.strip_prefix("xl/media/imageC") {
            if let Some(num) = rest.split('.').next().and_then(|v| v.parse::<usize>().ok()) {
                media_n = media_n.max(num);
            }
        }
    }
    for (i, sh) in book.sheets.iter().enumerate() {
        if sh.images_new.is_empty() && sh.shapes_new.is_empty() {
            continue;
        }
        let mut anchors = String::new();
        let mut rels_add = String::new();
        for (k, spn) in sh.shapes_new.iter().enumerate() {
            anchors.push_str(&shape_anchor_xml(spn, (i as u32) * 100 + k as u32 + 50));
        }
        for (k, im) in sh.images_new.iter().enumerate() {
            media_n += 1;
            let ext = if im.data.starts_with(&[0xFF, 0xD8]) { "jpeg" } else { "png" };
            let _ = k;
            let rid = format!("rIdC{media_n}");
            media_out.push((format!("xl/media/imageC{media_n}.{ext}"), im.data.clone()));
            rels_add.push_str(&format!(
                r#"<Relationship Id="{rid}" Type="{RNS}/image" Target="../media/imageC{media_n}.{ext}"/>"#
            ));
            anchors.push_str(&image_anchor_xml(im, &rid, (i as u32) * 100 + k as u32 + 2));
        }
        let orig_target = orig_sheet_rels.get(i).cloned().flatten().and_then(|onr| {
            parse_rels(&onr)
                .into_iter()
                .find(|(_, ty, _, _)| ty.ends_with("/drawing"))
                .map(|(_, _, t, _)| resolve_target(&t))
        });
        match orig_target {
            Some(dpath) => {
                for (name, buf) in carried.iter_mut() {
                    if *name == dpath {
                        let mut xml = String::from_utf8_lossy(buf).to_string();
                        if let Some(p) = xml.rfind("</xdr:wsDr>") {
                            xml.insert_str(p, &anchors);
                            *buf = xml.into_bytes();
                        }
                    }
                }
                let drels = {
                    let (dir, base) = dpath.rsplit_once('/').unwrap_or(("xl/drawings", &dpath));
                    format!("{dir}/_rels/{base}.rels")
                };
                let mut found = false;
                for (name, buf) in carried.iter_mut() {
                    if *name == drels {
                        let mut xml = String::from_utf8_lossy(buf).to_string();
                        if let Some(p) = xml.rfind("</Relationships>") {
                            xml.insert_str(p, &rels_add);
                            *buf = xml.into_bytes();
                        }
                        found = true;
                    }
                }
                if !found {
                    fresh_parts.push((drels, format!(
                        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{rels_add}</Relationships>"
                    )));
                }
            }
            None => {
                fresh_parts.push((
                    format!("xl/drawings/drawingC{}.xml", i + 1),
                    format!(
                        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<xdr:wsDr xmlns:xdr=\"http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">{anchors}</xdr:wsDr>"
                    ),
                ));
                fresh_parts.push((
                    format!("xl/drawings/_rels/drawingC{}.xml.rels", i + 1),
                    format!(
                        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{rels_add}</Relationships>"
                    ),
                ));
            }
        }
    }

    // ブックに載せた Python はモデルが正(古い部品は写さない)
    carried.retain(|(name, _)| name != "xl/joPython.xml");
    let carry = !carried.is_empty();
    for (name, buf) in &carried {
        zip.start_file(name.as_str(), o).map_err(|e| e.to_string())?;
        zip.write_all(buf).map_err(|e| e.to_string())?;
    }
    for (name, buf) in &media_out {
        zip.start_file(name.as_str(), o).map_err(|e| e.to_string())?;
        zip.write_all(buf).map_err(|e| e.to_string())?;
    }
    let mut put = |name: &str, data: &str| -> Result<(), String> {
        zip.start_file(name, o).map_err(|e| e.to_string())?;
        zip.write_all(data.as_bytes()).map_err(|e| e.to_string())
    };
    // [Content_Types]。コメントの部品を持つときは、その宣言も要る
    {
        let mut ct = match &orig_ct {
            Some(s) => s.clone(),
            None => CT.replace("__SHEETS__", &overrides),
        };
        let has_comments = book.sheets.iter().any(|s| !s.comments.is_empty());
        let mut add = String::new();
        if has_comments && !ct.contains("Extension=\"vml\"") {
            add.push_str(r#"<Default Extension="vml" ContentType="application/vnd.openxmlformats-officedocument.vmlDrawing"/>"#);
        }
        for (i, sh) in book.sheets.iter().enumerate() {
            let part = format!("/xl/comments{}.xml", i + 1);
            if !sh.comments.is_empty() && !ct.contains(&part) {
                add.push_str(&format!(r#"<Override PartName="{part}" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml"/>"#));
            }
        }
        // 挿した画像の部品の宣言(絵の拡張子と、新しく作った drawing)
        if media_out.iter().any(|(n, _)| n.ends_with(".png")) && !ct.contains("Extension=\"png\"") {
            add.push_str(r#"<Default Extension="png" ContentType="image/png"/>"#);
        }
        if media_out.iter().any(|(n, _)| n.ends_with(".jpeg")) && !ct.contains("Extension=\"jpeg\"") {
            add.push_str(r#"<Default Extension="jpeg" ContentType="image/jpeg"/>"#);
        }
        for (name, _) in &fresh_parts {
            if name.starts_with("xl/drawings/drawingC") && name.ends_with(".xml") {
                add.push_str(&format!(
                    r#"<Override PartName="/{name}" ContentType="application/vnd.openxmlformats-officedocument.drawing+xml"/>"#
                ));
            }
        }
        if !add.is_empty() {
            if let Some(p) = ct.rfind("</Types>") {
                ct.insert_str(p, &add);
            }
        }
        put("[Content_Types].xml", &ct)?;
    }
    for (name, xml) in &fresh_parts {
        put(name, xml)?;
    }
    if !book.scripts.is_empty() {
        let mut sx = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<joPython>",
        );
        for (n, code) in &book.scripts {
            sx.push_str(&format!("<script name=\"{}\">{}</script>", esc(n), esc(code)));
        }
        sx.push_str("</joPython>");
        put("xl/joPython.xml", &sx)?;
    }
    if !carry {
        put("_rels/.rels", RELS)?;
    }

    let sheets_xml: String = book.sheets.iter().enumerate()
        .map(|(i, s)| format!(r#"<sheet name="{}" sheetId="{}" r:id="rId{}"/>"#,
                              esc(&s.name), i + 1, i + 1))
        .collect();
    if !carry {
    put("xl/workbook.xml", &format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="{NS}" xmlns:r="{RNS}"><sheets>{sheets_xml}</sheets>{}</workbook>"#,
        defined_names_xml(book)))?;

    let wrels: String = (1..=book.sheets.len())
        .map(|i| format!(r#"<Relationship Id="rId{i}" Type="{RNS}/worksheet" Target="worksheets/sheet{i}.xml"/>"#))
        .collect();
    put("xl/_rels/workbook.xml.rels", &format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{wrels}<Relationship Id="rIdSS" Type="{RNS}/sharedStrings" Target="sharedStrings.xml"/><Relationship Id="rIdST" Type="{RNS}/styles" Target="styles.xml"/></Relationships>"#))?;
    }

    put("xl/styles.xml", &styles_xml)?;

    let si: String = shared.iter()
        .map(|s| format!("<si><t xml:space=\"preserve\">{}</t></si>", esc(s)))
        .collect();
    put("xl/sharedStrings.xml", &format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<sst xmlns="{NS}" count="{}" uniqueCount="{}">{si}</sst>"#, shared.len(), shared.len()))?;

    for (i, sh) in book.sheets.iter().enumerate() {
        let mut w = Writer::new(Cursor::new(Vec::new()));
        let mut ws = BytesStart::new("worksheet");
        ws.push_attribute(("xmlns", NS));
        ws.push_attribute(("xmlns:r", RNS));
        w.write_event(Event::Start(ws)).unwrap();
        // 列幅。読んだものを返す(捨てると帳票の形が変わる)。
        // 同じ幅が並ぶ区間は1つの col にまとめる
        if !sh.col_width.is_empty() || sh.default_col_width.is_some() {
            w.write_event(Event::Start(BytesStart::new("cols"))).unwrap();
            if let Some(dw) = sh.default_col_width {
                let mut e = BytesStart::new("col");
                e.push_attribute(("min", "1"));
                e.push_attribute(("max", "16384"));
                e.push_attribute(("width", dw.to_string().as_str()));
                w.write_event(Event::Empty(e)).unwrap();
            }
            let mut it = sh.col_width.iter().peekable();
            while let Some((c0, wd)) = it.next() {
                let mut c1 = *c0;
                while let Some((cn, wn)) = it.peek() {
                    if **cn == c1 + 1 && (**wn - *wd).abs() < 1e-6 {
                        c1 = **cn;
                        it.next();
                    } else {
                        break;
                    }
                }
                let mut e = BytesStart::new("col");
                e.push_attribute(("min", (c0 + 1).to_string().as_str()));
                e.push_attribute(("max", (c1 + 1).to_string().as_str()));
                e.push_attribute(("width", wd.to_string().as_str()));
                e.push_attribute(("customWidth", "1"));
                w.write_event(Event::Empty(e)).unwrap();
            }
            w.write_event(Event::End(BytesEnd::new("cols"))).unwrap();
        }
        w.write_event(Event::Start(BytesStart::new("sheetData"))).unwrap();

        let mut rows: std::collections::BTreeMap<u32, Vec<(&Pos, &Cell)>> = Default::default();
        for (p, c) in &sh.cells { rows.entry(p.row).or_default().push((p, c)); }
        for (r, cells) in rows {
            let mut row = BytesStart::new("row");
            row.push_attribute(("r", (r + 1).to_string().as_str()));
            if let Some(h) = sh.row_height.get(&r) {
                row.push_attribute(("ht", h.to_string().as_str()));
                row.push_attribute(("customHeight", "1"));
            }
            w.write_event(Event::Start(row)).unwrap();
            for (p, c) in cells {
                let mut ce = BytesStart::new("c");
                ce.push_attribute(("r", p.a1().as_str()));
                let (ty, text) = match &c.value {
                    Value::Text(t) => ("s", idx[t].to_string()),
                    Value::Number(n) => ("", n.to_string()),
                    Value::Bool(b) => ("b", (*b as u8).to_string()),
                    Value::Error(e) => ("e", e.clone()),
                    Value::Empty => ("", String::new()),
                };
                if !ty.is_empty() { ce.push_attribute(("t", ty)); }
                // 書式は styles.xml 側にあり、ここは索引だけ
                if let Some(s) = style_idx.get(&c.fmt).filter(|i| **i > 0) {
                    ce.push_attribute(("s", s.to_string().as_str()));
                }
                w.write_event(Event::Start(ce)).unwrap();
                if let Some(f) = &c.formula {
                    w.write_event(Event::Start(BytesStart::new("f"))).unwrap();
                    w.write_event(Event::Text(BytesText::new(f))).unwrap();
                    w.write_event(Event::End(BytesEnd::new("f"))).unwrap();
                }
                if !text.is_empty() {
                    w.write_event(Event::Start(BytesStart::new("v"))).unwrap();
                    w.write_event(Event::Text(BytesText::new(&text))).unwrap();
                    w.write_event(Event::End(BytesEnd::new("v"))).unwrap();
                }
                w.write_event(Event::End(BytesEnd::new("c"))).unwrap();
            }
            w.write_event(Event::End(BytesEnd::new("row"))).unwrap();
        }
        w.write_event(Event::End(BytesEnd::new("sheetData"))).unwrap();
        // 結合を返す。読めたのに書かないと、開いて保存しただけで帳票が壊れる
        if !sh.merges.is_empty() {
            let mut mc = BytesStart::new("mergeCells");
            mc.push_attribute(("count", sh.merges.len().to_string().as_str()));
            w.write_event(Event::Start(mc)).unwrap();
            for (a, b) in &sh.merges {
                let mut m = BytesStart::new("mergeCell");
                m.push_attribute(("ref", format!("{}:{}", a.a1(), b.a1()).as_str()));
                w.write_event(Event::Empty(m)).unwrap();
            }
            w.write_event(Event::End(BytesEnd::new("mergeCells"))).unwrap();
        }
        w.write_event(Event::End(BytesEnd::new("worksheet"))).unwrap();
        let mut body = String::from_utf8(w.into_inner().into_inner()).unwrap();
        // 条件付き書式(schema では mergeCells の後・hyperlinks の前)
        if !sh.cond.is_empty() {
            let mut cf = String::new();
            for (n, r) in sh.cond.iter().enumerate() {
                let dxf = dxf_list
                    .iter()
                    .position(|p| *p == (r.color.clone(), r.fill.clone()))
                    .unwrap_or(0);
                let (a, b) = r.range;
                let sq = if a == b {
                    a.a1()
                } else {
                    format!("{}:{}", a.a1(), b.a1())
                };
                cf.push_str(&format!(
                    r#"<conditionalFormatting sqref="{sq}"><cfRule type="cellIs" dxfId="{dxf}" priority="{}" operator="{}"><formula>{}</formula></cfRule></conditionalFormatting>"#,
                    n + 1,
                    r.op.as_xlsx(),
                    r.value
                ));
            }
            if let Some(pos) = body.rfind("</worksheet>") {
                body.insert_str(pos, &cf);
            }
        }
        // データの入力規則(schema では conditionalFormatting の後・hyperlinks の前)
        if !sh.validations.is_empty() {
            let mut dv = format!(r#"<dataValidations count="{}">"#, sh.validations.len());
            for v in &sh.validations {
                let (a, b) = v.range;
                let sq = if a == b { a.a1() } else { format!("{}:{}", a.a1(), b.a1()) };
                // formula1 の中の & と < だけ字面を守る(" は本文では素のまま合法)
                let f = v.formula.replace('&', "&amp;").replace('<', "&lt;");
                dv.push_str(&format!(
                    r#"<dataValidation type="list" allowBlank="1" showInputMessage="1" showErrorMessage="1" sqref="{sq}"><formula1>{f}</formula1></dataValidation>"#,
                ));
            }
            dv.push_str("</dataValidations>");
            if let Some(pos) = body.rfind("</worksheet>") {
                body.insert_str(pos, &dv);
            }
        }
        // ハイパーリンク(schema では mergeCells の後・印刷まわりの前)
        if !sh.links.is_empty() {
            let mut hl = String::from("<hyperlinks>");
            for (n, (p, _)) in sh.links.iter().enumerate() {
                hl.push_str(&format!(r#"<hyperlink ref="{}" r:id="rIdHL{}"/>"#, p.a1(), n + 1));
            }
            hl.push_str("</hyperlinks>");
            if let Some(pos) = body.rfind("</worksheet>") {
                body.insert_str(pos, &hl);
            }
        }
        // 印刷まわり(原本の原文にモデルの向き・用紙・余白を織り込む)と
        // 図形の参照を、schema の位置(hyperlinks の後)へ
        {
            let orig = sheet_extras.get(i).map(|s| s.as_str()).unwrap_or("");
            let extra = print_extra_xml(orig, sh);
            if !extra.is_empty() {
                if let Some(pos) = body.rfind("</worksheet>") {
                    body.insert_str(pos, &extra);
                }
            }
        }
        // このアプリで挿した画像。原本に drawing が無ければ新しい部品への参照を足す
        // (原本に有るときは、その部品の中へ錨を継ぎ足す — 部品は1シート1つの決まり)
        if (!sh.images_new.is_empty() || !sh.shapes_new.is_empty())
            && !body.contains("<drawing ")
        {
            if let Some(pos) = body.rfind("</worksheet>") {
                body.insert_str(pos, r#"<drawing r:id="rIdDRW"/>"#);
            }
        }
        // コメントの図形(VML)への参照は一番後ろ
        if !sh.comments.is_empty() {
            if let Some(pos) = body.rfind("</worksheet>") {
                body.insert_str(pos, r#"<legacyDrawing r:id="rIdVML"/>"#);
            }
        }
        put(&format!("xl/worksheets/sheet{}.xml", i + 1),
            &format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n{body}"))?;

        // このシートの rels。原本のもの(図形など)は残し、
        // リンク・コメントのぶんはこちらが作り直す
        let orig = orig_sheet_rels.get(i).cloned().flatten();
        if !sh.links.is_empty() || !sh.comments.is_empty() || orig.is_some()
            || !sh.images_new.is_empty() || !sh.shapes_new.is_empty()
        {
            let mut inner = String::new();
            if let Some(o) = &orig {
                for (id, ty, target, ext) in parse_rels(o) {
                    if ty.ends_with("/hyperlink")
                        || ty.ends_with("/comments")
                        || ty.ends_with("/vmlDrawing")
                    {
                        continue;
                    }
                    inner.push_str(&format!(
                        r#"<Relationship Id="{}" Type="{}" Target="{}"{}/>"#,
                        esc(&id), esc(&ty), esc(&target),
                        if ext { r#" TargetMode="External""# } else { "" }
                    ));
                }
            }
            for (n, (_, url)) in sh.links.iter().enumerate() {
                inner.push_str(&format!(
                    r#"<Relationship Id="rIdHL{}" Type="{RNS}/hyperlink" Target="{}" TargetMode="External"/>"#,
                    n + 1, esc(url)
                ));
            }
            let had_drawing = orig.as_deref().is_some_and(|o| o.contains("/drawing\""));
            if (!sh.images_new.is_empty() || !sh.shapes_new.is_empty()) && !had_drawing {
                inner.push_str(&format!(
                    r#"<Relationship Id="rIdDRW" Type="{RNS}/drawing" Target="../drawings/drawingC{}.xml"/>"#,
                    i + 1
                ));
            }
            if !sh.comments.is_empty() {
                inner.push_str(&format!(
                    r#"<Relationship Id="rIdCM" Type="{RNS}/comments" Target="../comments{}.xml"/>"#,
                    i + 1
                ));
                inner.push_str(&format!(
                    r#"<Relationship Id="rIdVML" Type="{RNS}/vmlDrawing" Target="../drawings/vmlDrawing{}.vml"/>"#,
                    i + 1
                ));
            }
            put(&format!("xl/worksheets/_rels/sheet{}.xml.rels", i + 1), &format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{inner}</Relationships>"))?;
        }
        // コメントの本体と、Excel がコメントに使う最小の VML 図形
        if !sh.comments.is_empty() {
            let mut cl = String::new();
            for (p, t) in &sh.comments {
                cl.push_str(&format!(
                    r#"<comment ref="{}" authorId="0"><text><r><t xml:space="preserve">{}</t></r></text></comment>"#,
                    p.a1(), esc(t)
                ));
            }
            put(&format!("xl/comments{}.xml", i + 1), &format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<comments xmlns="{NS}"><authors><author></author></authors><commentList>{cl}</commentList></comments>"#))?;
            let mut shapes = String::new();
            for (n, (p, _)) in sh.comments.iter().enumerate() {
                shapes.push_str(&format!(
                    r##"<v:shape id="_x0000_s{}" type="#_x0000_t202" style="position:absolute;margin-left:80pt;margin-top:2pt;width:120pt;height:60pt;z-index:{};visibility:hidden" fillcolor="#ffffe1" o:insetmode="auto"><v:fill color2="#ffffe1"/><x:ClientData ObjectType="Note"><x:MoveWithCells/><x:SizeWithCells/><x:AutoFill>False</x:AutoFill><x:Row>{}</x:Row><x:Column>{}</x:Column></x:ClientData></v:shape>"##,
                    1025 + n, n + 1, p.row, p.col
                ));
            }
            put(&format!("xl/drawings/vmlDrawing{}.vml", i + 1), &format!(
                r#"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:x="urn:schemas-microsoft-com:office:excel"><o:shapelayout v:ext="edit"><o:idmap v:ext="edit" data="1"/></o:shapelayout><v:shapetype id="_x0000_t202" coordsize="21600,21600" o:spt="202" path="m,l,21600r21600,l21600,xe"><v:stroke joinstyle="miter"/><v:path gradientshapeok="t" o:connecttype="rect"/></v:shapetype>{shapes}</xml>"#))?;
        }
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod fmt_round {
    use crate::model::{Borders, Cell, CellFormat, HAlign, Pos, Value};
    use crate::{Book, Sheet};

    fn book(fmt: CellFormat) -> Book {
        let mut s = Sheet { name: "帳票".into(), ..Default::default() };
        s.set(Pos { row: 0, col: 0 }, Cell {
            formula: None, value: Value::Text("品名".into()), fmt: fmt.clone() });
        s.set(Pos { row: 0, col: 1 }, Cell {
            formula: None, value: Value::Number(1200.0), fmt });
        Book { sheets: vec![s], names_raw: Vec::new(), scripts: Vec::new() }
    }

    fn roundtrip(b: &Book) -> Book {
        let mut buf = Vec::new();
        crate::xlsx::write(b, std::io::Cursor::new(&mut buf)).unwrap();
        crate::xlsx::read(std::io::Cursor::new(&buf)).unwrap().0
    }

    #[test]
    fn 罫線が往復する() {
        // 日本の帳票の本体。落とすと書類として通らない
        let f = CellFormat { borders: Borders::ALL, ..Default::default() };
        let back = roundtrip(&book(f.clone()));
        let c = back.sheets[0].get(Pos { row: 0, col: 0 }).unwrap();
        assert_eq!(c.fmt.borders, Borders::ALL, "罫線が消えた: {:?}", c.fmt);
    }

    #[test]
    fn 太字と塗りと揃えが往復する() {
        let f = CellFormat {
            bold: true,
            fill: Some("FFFF00".into()),
            align: HAlign::Center,
            borders: Borders { bottom: true, ..Borders::NONE },
            ..Default::default()
        };
        let back = roundtrip(&book(f.clone()));
        let c = back.sheets[0].get(Pos { row: 0, col: 0 }).unwrap();
        assert_eq!(c.fmt, f, "書式が変わった");
    }

    #[test]
    fn 表示形式が往復する() {
        let f = CellFormat { number_format: Some("#,##0".into()), ..Default::default() };
        let back = roundtrip(&book(f.clone()));
        let c = back.sheets[0].get(Pos { row: 0, col: 1 }).unwrap();
        assert_eq!(c.fmt.number_format.as_deref(), Some("#,##0"));
        assert_eq!(c.value, Value::Number(1200.0), "値が壊れた");
    }

    #[test]
    fn 素の書式なら索引を付けない() {
        // 余計な索引を書かない(他の道具が読むときの雑音になる)
        let mut buf = Vec::new();
        crate::xlsx::write(&book(CellFormat::default()), std::io::Cursor::new(&mut buf)).unwrap();
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(&buf)).unwrap();
        let mut s = String::new();
        use std::io::Read;
        z.by_name("xl/worksheets/sheet1.xml").unwrap().read_to_string(&mut s).unwrap();
        assert!(!s.contains(" s=\""), "素の書式に索引を付けた");
    }

    #[test]
    fn 罫線だけのセルも残る() {
        // 値が無くても、罫線が引いてあれば帳票では意味を持つ
        let mut sh = Sheet { name: "枠".into(), ..Default::default() };
        sh.set(Pos { row: 2, col: 2 }, Cell {
            formula: None,
            value: Value::Empty,
            fmt: CellFormat { borders: Borders::ALL, ..Default::default() },
        });
        let back = roundtrip(&Book { sheets: vec![sh], names_raw: Vec::new(), scripts: Vec::new() });
        let c = back.sheets[0].get(Pos { row: 2, col: 2 });
        assert!(c.is_some(), "値の無い罫線セルが消えた");
        assert_eq!(c.unwrap().fmt.borders, Borders::ALL);
    }
}

#[cfg(test)]
mod merge_round {
    use crate::model::{Cell, Pos, Value};
    use crate::{Book, Sheet};

    fn roundtrip(b: &Book) -> Book {
        let mut buf = Vec::new();
        crate::xlsx::write(b, std::io::Cursor::new(&mut buf)).unwrap();
        crate::xlsx::read(std::io::Cursor::new(&buf)).unwrap().0
    }

    #[test]
    fn セル結合が往復する() {
        // 開いて保存しただけで帳票の枠組みが壊れてはいけない
        let mut s = Sheet { name: "帳票".into(), ..Default::default() };
        s.set(Pos::parse("A1").unwrap(), Cell {
            formula: None, value: Value::Text("見出し".into()), fmt: Default::default() });
        s.merges.push((Pos::parse("A1").unwrap(), Pos::parse("C1").unwrap()));
        s.merges.push((Pos::parse("A2").unwrap(), Pos::parse("A4").unwrap()));
        let back = roundtrip(&Book { sheets: vec![s], names_raw: Vec::new(), scripts: Vec::new() });
        assert_eq!(back.sheets[0].merges.len(), 2, "結合が消えた");
        assert_eq!(back.sheets[0].merges[0],
                   (Pos::parse("A1").unwrap(), Pos::parse("C1").unwrap()));
    }

    #[test]
    fn 行の出し入れで結合も動く() {
        let mut s = Sheet { name: "帳票".into(), ..Default::default() };
        s.merges.push((Pos::parse("A3").unwrap(), Pos::parse("C3").unwrap()));
        s.insert_row(1);
        assert_eq!(s.merges[0], (Pos::parse("A4").unwrap(), Pos::parse("C4").unwrap()),
                   "結合が置き去りになった");
        s.remove_row(1);
        assert_eq!(s.merges[0], (Pos::parse("A3").unwrap(), Pos::parse("C3").unwrap()));
    }

    #[test]
    fn 潰れた結合は消える() {
        // A1:A2 の縦結合で2行目を抜くと、1セルになる。1セルの結合は結合ではない
        let mut s = Sheet { name: "帳票".into(), ..Default::default() };
        s.merges.push((Pos::parse("A1").unwrap(), Pos::parse("A2").unwrap()));
        s.remove_row(1);
        assert!(s.merges.is_empty(), "1セルの結合が残った: {:?}", s.merges);
    }

    #[test]
    fn 呑まれた位置が分かる() {
        let mut s = Sheet { name: "帳票".into(), ..Default::default() };
        s.merges.push((Pos::parse("A1").unwrap(), Pos::parse("B2").unwrap()));
        assert!(!s.covered_by_merge(Pos::parse("A1").unwrap()), "左上まで呑んだ");
        assert!(s.covered_by_merge(Pos::parse("B2").unwrap()));
        assert!(!s.covered_by_merge(Pos::parse("C1").unwrap()));
    }
}

#[cfg(test)]
mod colwidth_round {
    use crate::model::{Cell, Pos, Value};
    use crate::{Book, Sheet};

    #[test]
    fn 列幅が往復する() {
        // 読み飛ばして保存すると帳票の形が変わる
        let mut s = Sheet { name: "帳票".into(), ..Default::default() };
        s.set(Pos::parse("A1").unwrap(), Cell {
            formula: None, value: Value::Text("品".into()), fmt: Default::default() });
        s.col_width.insert(0, 3.5);
        s.col_width.insert(2, 24.0);
        let mut buf = Vec::new();
        crate::xlsx::write(&Book { sheets: vec![s], names_raw: Vec::new(), scripts: Vec::new() }, std::io::Cursor::new(&mut buf)).unwrap();
        let back = crate::xlsx::read(std::io::Cursor::new(&buf)).unwrap().0;
        let cw = &back.sheets[0].col_width;
        assert_eq!(cw.get(&0), Some(&3.5), "列幅が消えた: {cw:?}");
        assert_eq!(cw.get(&2), Some(&24.0));
        assert_eq!(cw.get(&1), None, "指定していない列に幅が付いた");
    }

    #[test]
    fn 列の出し入れで幅も動く() {
        let mut s = Sheet { name: "帳票".into(), ..Default::default() };
        s.col_width.insert(1, 20.0);
        s.insert_col(0);
        assert_eq!(s.col_width.get(&2), Some(&20.0), "幅が置き去り: {:?}", s.col_width);
        s.remove_col(0);
        assert_eq!(s.col_width.get(&1), Some(&20.0));
    }

    #[test]
    fn 実物の様式の列幅を読める() {
        let p = "/mnt/sdb/home/dev/ドキュメント/機構/yoryou-yoshiki/実施要領様式7_提案見積書.xlsx";
        let Ok(f) = std::fs::File::open(p) else { return }; // 無い機械では飛ばす
        let (book, _) = crate::xlsx::read(f).unwrap();
        let n: usize = book.sheets.iter().map(|s| s.col_width.len()).sum();
        assert!(n > 0, "実物の列幅を1つも読めていない");
    }
}

#[cfg(test)]
mod rowheight_round {
    use crate::model::{Cell, Pos, Value};
    use crate::{Book, Sheet};

    #[test]
    fn 行の高さが往復する() {
        let mut s = Sheet { name: "帳票".into(), ..Default::default() };
        s.set(Pos::parse("A3").unwrap(), Cell {
            formula: None, value: Value::Text("高い行".into()), fmt: Default::default() });
        s.row_height.insert(2, 27.5);
        let mut buf = Vec::new();
        crate::xlsx::write(&Book { sheets: vec![s], names_raw: Vec::new(), scripts: Vec::new() }, std::io::Cursor::new(&mut buf)).unwrap();
        let back = crate::xlsx::read(std::io::Cursor::new(&buf)).unwrap().0;
        assert_eq!(back.sheets[0].row_height.get(&2), Some(&27.5), "行の高さが消えた");
    }

    #[test]
    fn 行の出し入れで高さも動く() {
        let mut s = Sheet { name: "帳票".into(), ..Default::default() };
        s.row_height.insert(3, 30.0);
        s.insert_row(0);
        assert_eq!(s.row_height.get(&4), Some(&30.0), "{:?}", s.row_height);
        s.remove_row(0);
        assert_eq!(s.row_height.get(&3), Some(&30.0));
    }
}

#[cfg(test)]
mod carry_tests {
    use crate::model::{Cell, Pos, Value};
    use crate::{Book, Sheet};
    use std::io::{Cursor, Read, Write};

    fn xlsx_with_parts() -> Vec<u8> {
        let mut book = Book::default();
        let mut s = Sheet { name: "帳票".into(), ..Default::default() };
        s.set(Pos::parse("A1").unwrap(), Cell::input("品名"));
        book.sheets.push(s);
        let mut base = Vec::new();
        crate::xlsx::write(&book, Cursor::new(&mut base)).unwrap();
        // 原本に「こちらが知らない部品」を足し、シートに印刷設定と図形を差す
        let mut z = zip::ZipArchive::new(Cursor::new(&base)).unwrap();
        let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let o: zip::write::FileOptions<'_, ()> = Default::default();
        for i in 0..z.len() {
            let mut f = z.by_index(i).unwrap();
            let name = f.name().to_string();
            let mut buf = Vec::new();
            f.read_to_end(&mut buf).unwrap();
            if name == "xl/worksheets/sheet1.xml" {
                let s = String::from_utf8(buf).unwrap().replace(
                    "</worksheet>",
                    r#"<pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/><pageSetup paperSize="9" orientation="landscape"/><drawing r:id="rId9"/></worksheet>"#,
                );
                buf = s.into_bytes();
            }
            out.start_file(name, o).unwrap();
            out.write_all(&buf).unwrap();
        }
        out.start_file("xl/theme/theme1.xml", o).unwrap();
        out.write_all(b"<theme/>").unwrap();
        out.start_file("xl/drawings/drawing1.xml", o).unwrap();
        out.write_all(b"<wsDr/>").unwrap();
        out.start_file("xl/printerSettings/printerSettings1.bin", o).unwrap();
        out.write_all(b"\x01\x02printer").unwrap();
        out.finish().unwrap().into_inner()
    }

    #[test]
    fn 開いて保存しても部品が残る() {
        let src = xlsx_with_parts();
        let (book, _) = crate::xlsx::read(Cursor::new(&src)).unwrap();
        let mut out = Vec::new();
        crate::xlsx::write_with(&book, Some(Cursor::new(&src)), Cursor::new(&mut out)).unwrap();
        let mut z = zip::ZipArchive::new(Cursor::new(&out)).unwrap();
        let names: Vec<String> =
            (0..z.len()).map(|i| z.by_index(i).unwrap().name().into()).collect();
        for want in ["xl/theme/theme1.xml", "xl/drawings/drawing1.xml",
                     "xl/printerSettings/printerSettings1.bin"] {
            assert!(names.iter().any(|n| n == want), "{want} が消えた: {names:?}");
        }
        // 印刷の向きと図形の参照がシートに戻っている
        let mut s = String::new();
        z.by_name("xl/worksheets/sheet1.xml").unwrap().read_to_string(&mut s).unwrap();
        assert!(s.contains("landscape"), "印刷の向きが消えた");
        assert!(s.contains("<drawing"), "図形の参照が消えた");
        // 値も生きている
        let (back, _) = crate::xlsx::read(Cursor::new(&out)).unwrap();
        assert_eq!(back.sheets[0].get(Pos::parse("A1").unwrap()).map(|c| c.value.display()),
                   Some("品名".into()));
    }

    #[test]
    fn 古い計算順は持ち越さない() {
        // calcChain が古いままだと Excel が誤った順で開くことがある
        let src = xlsx_with_parts();
        let mut with_chain = Vec::new();
        {
            let mut z = zip::ZipArchive::new(Cursor::new(&src)).unwrap();
            let mut out = zip::ZipWriter::new(Cursor::new(&mut with_chain));
            let o: zip::write::FileOptions<'_, ()> = Default::default();
            for i in 0..z.len() {
                let mut f = z.by_index(i).unwrap();
                let name = f.name().to_string();
                let mut buf = Vec::new();
                f.read_to_end(&mut buf).unwrap();
                out.start_file(name, o).unwrap();
                out.write_all(&buf).unwrap();
            }
            out.start_file("xl/calcChain.xml", o).unwrap();
            out.write_all(b"<calcChain/>").unwrap();
            out.finish().unwrap();
        }
        let (book, _) = crate::xlsx::read(Cursor::new(&with_chain)).unwrap();
        let mut out = Vec::new();
        crate::xlsx::write_with(&book, Some(Cursor::new(&with_chain)), Cursor::new(&mut out)).unwrap();
        let z = zip::ZipArchive::new(Cursor::new(&out)).unwrap();
        let names: Vec<String> = z.file_names().map(String::from).collect();
        assert!(!names.iter().any(|n| n == "xl/calcChain.xml"), "古い計算順を持ち越した");
    }
}

#[cfg(test)]
mod name_roundtrip_tests {
    use super::*;
    use crate::model::Cell;
    use crate::recalc;

    #[test]
    fn 名前の定義が往復して式で効く() {
        let mut b = Book::new();
        b.sheets[0].set(Pos::parse("A1").unwrap(), Cell::input("100"));
        b.sheets[0].set(Pos::parse("B1").unwrap(), Cell::input("=単価*2"));
        b.sheets[0].names.push(("単価".into(), "A1".into()));
        let mut buf = Cursor::new(Vec::new());
        write(&b, &mut buf).expect("書けない");
        buf.set_position(0);
        let (mut back, _) = read(buf).expect("読めない");
        assert_eq!(back.sheets[0].names, vec![("単価".to_string(), "A1".to_string())],
            "名前が往復しない");
        recalc(&mut back.sheets[0]);
        assert_eq!(back.sheets[0].value(Pos::parse("B1").unwrap()), Value::Number(200.0));
    }

    #[test]
    fn 実物のprint_areaを壊さない() {
        let src = "/mnt/sdb/home/dev/ドキュメント/機構/yoryou-yoshiki/実施要領様式7_提案見積書.xlsx";
        let Ok(bytes) = std::fs::read(src) else { return };
        let (book, _) = read(Cursor::new(&bytes)).expect("読めない");
        let mut out = Cursor::new(Vec::new());
        write_with(&book, Some(Cursor::new(&bytes)), &mut out).expect("書けない");
        out.set_position(0);
        let mut z = zip::ZipArchive::new(out).expect("zipでない");
        let mut s = String::new();
        use std::io::Read as _;
        z.by_name("xl/workbook.xml").expect("workbookが無い")
            .read_to_string(&mut s).unwrap();
        assert!(s.contains("_xlnm.Print_Area"),
            "印刷範囲(Print_Area)が保存で消えた");
    }
}

#[cfg(test)]
mod link_comment_tests {
    use super::*;
    use crate::model::Cell;

    fn roundtrip(b: &Book) -> Book {
        let mut buf = Cursor::new(Vec::new());
        write(b, &mut buf).expect("書けない");
        buf.set_position(0);
        read(buf).expect("読めない").0
    }

    #[test]
    fn ハイパーリンクが往復する() {
        let mut b = Book::new();
        let p = Pos::parse("B2").unwrap();
        b.sheets[0].set(p, Cell::input("会社サイト"));
        b.sheets[0].links.insert(p, "https://example.co.jp/".into());
        let back = roundtrip(&b);
        assert_eq!(back.sheets[0].links.get(&p).map(|s| s.as_str()),
            Some("https://example.co.jp/"), "リンクが往復しない");
    }

    #[test]
    fn コメントが往復する() {
        let mut b = Book::new();
        let p = Pos::parse("C3").unwrap();
        b.sheets[0].set(p, Cell::input("単価"));
        b.sheets[0].comments.insert(p, "去年の実績から仮置き。要確認".into());
        let back = roundtrip(&b);
        assert_eq!(back.sheets[0].comments.get(&p).map(|s| s.as_str()),
            Some("去年の実績から仮置き。要確認"), "コメントが往復しない");
    }

    #[test]
    fn 実物にコメントを足しても部品が揃う() {
        let src = "/mnt/sdb/home/dev/ドキュメント/機構/yoryou-yoshiki/実施要領様式7_提案見積書.xlsx";
        let Ok(bytes) = std::fs::read(src) else { return };
        let (mut book, _) = read(Cursor::new(&bytes)).expect("読めない");
        let p = Pos::parse("A30").unwrap();
        book.sheets[0].comments.insert(p, "ここに社名を書く".into());
        book.sheets[0].links.insert(p, "https://example.co.jp/".into());
        let mut out = Cursor::new(Vec::new());
        write_with(&book, Some(Cursor::new(&bytes)), &mut out).expect("書けない");
        out.set_position(0);
        // 読み直せて中身が残る
        let (back, _) = read(Cursor::new(out.get_ref().clone())).expect("読み直せない");
        assert_eq!(back.sheets[0].comments.get(&p).map(|s| s.as_str()),
            Some("ここに社名を書く"));
        assert!(back.sheets[0].links.contains_key(&p), "実物でリンクが消えた");
        // 部品の宣言も揃っている
        let mut z = zip::ZipArchive::new(out).unwrap();
        let mut ct = String::new();
        use std::io::Read as _;
        z.by_name("[Content_Types].xml").unwrap().read_to_string(&mut ct).unwrap();
        assert!(ct.contains("/xl/comments1.xml"), "コメントの宣言が無い");
        assert!(ct.contains("Extension=\"vml\""), "VML の宣言が無い");
    }
}

#[cfg(test)]
mod cond_tests {
    use super::*;
    use crate::model::{Cell, CondOp, CondRule};

    #[test]
    fn 条件付き書式が往復する() {
        let mut b = Book::new();
        b.sheets[0].set(Pos::parse("A1").unwrap(), Cell::input("-5"));
        b.sheets[0].cond.push(CondRule {
            range: (Pos::parse("A1").unwrap(), Pos::parse("A9").unwrap()),
            op: CondOp::Lt,
            value: 0.0,
            color: Some("C00000".into()),
            fill: None,
        });
        let mut buf = Cursor::new(Vec::new());
        write(&b, &mut buf).expect("書けない");
        buf.set_position(0);
        let (back, _) = read(buf).expect("読めない");
        let r = &back.sheets[0].cond;
        assert_eq!(r.len(), 1, "規則が往復しない");
        assert_eq!(r[0].op, CondOp::Lt);
        assert_eq!(r[0].value, 0.0);
        assert_eq!(r[0].color.as_deref(), Some("C00000"), "見た目(dxf)が往復しない");
        // 効き方
        assert!(r[0].hits(Pos::parse("A1").unwrap(), &Value::Number(-5.0)));
        assert!(!r[0].hits(Pos::parse("A1").unwrap(), &Value::Number(5.0)));
        assert!(!r[0].hits(Pos::parse("B1").unwrap(), &Value::Number(-5.0)), "範囲の外に効いた");
    }
}

#[cfg(test)]
mod validation_roundtrip_tests {
    use super::*;
    use crate::model::{Cell, Validation};

    #[test]
    fn 入力規則が往復する() {
        let mut b = Book::new();
        b.sheets[0].set(Pos::parse("D2").unwrap(), Cell::input("東京"));
        b.sheets[0].set(Pos::parse("D3").unwrap(), Cell::input("大阪"));
        b.sheets[0].validations.push(Validation {
            range: (Pos::parse("B2").unwrap(), Pos::parse("B10").unwrap()),
            formula: r#""甲,乙,丙""#.into(),
        });
        b.sheets[0].validations.push(Validation {
            range: (Pos::parse("C2").unwrap(), Pos::parse("C2").unwrap()),
            formula: "$D$2:$D$3".into(),
        });
        let mut buf = Cursor::new(Vec::new());
        write(&b, &mut buf).expect("書けない");
        buf.set_position(0);
        let (back, rep) = read(buf).expect("読めない");
        let v = &back.sheets[0].validations;
        assert_eq!(v.len(), 2, "規則が往復しない: {v:?}");
        assert_eq!(v[0].formula, r#""甲,乙,丙""#, "直書きの原文が変わった");
        assert_eq!(v[0].range, (Pos::parse("B2").unwrap(), Pos::parse("B10").unwrap()));
        assert_eq!(v[1].formula, "$D$2:$D$3", "範囲参照の原文が変わった");
        // 候補も引ける
        assert_eq!(v[0].options(&back.sheets[0]), vec!["甲", "乙", "丙"]);
        assert_eq!(v[1].options(&back.sheets[0]), vec!["東京", "大阪"]);
        assert!(rep.unsupported.is_empty(), "全部読めるのに報告が出た: {:?}", rep.unsupported);
    }

    #[test]
    fn list以外の規則は報告して落とす() {
        // 手書きの最小 xlsx を作るのは大掛かりなので、書いた xlsx の
        // dataValidation の type を書き換えて読み直す
        let mut b = Book::new();
        b.sheets[0].validations.push(Validation {
            range: (Pos::parse("A1").unwrap(), Pos::parse("A1").unwrap()),
            formula: r#""x""#.into(),
        });
        let mut buf = Cursor::new(Vec::new());
        write(&b, &mut buf).expect("書けない");
        // zip の中の sheet1.xml を直に書き換える
        let mut z = zip::ZipArchive::new(Cursor::new(buf.get_ref().clone())).unwrap();
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        use std::io::{Read as _, Write as _};
        for i in 0..z.len() {
            let mut f = z.by_index(i).unwrap();
            let name = f.name().to_string();
            let mut s = Vec::new();
            f.read_to_end(&mut s).unwrap();
            if name.ends_with("sheet1.xml") {
                let t = String::from_utf8(s).unwrap()
                    .replace(r#"type="list""#, r#"type="whole""#);
                s = t.into_bytes();
            }
            w.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(&s).unwrap();
        }
        let out = w.finish().unwrap();
        let (back, rep) = read(Cursor::new(out.into_inner())).expect("読めない");
        assert!(back.sheets[0].validations.is_empty(), "list 以外を黙って読んだ");
        assert!(
            rep.unsupported.iter().any(|(n, _)| n.contains("入力規則")),
            "落としたのに報告が無い: {:?}",
            rep.unsupported
        );
    }
}

#[cfg(test)]
mod page_setup_tests {
    use super::*;

    #[test]
    fn 印刷の設定が読める() {
        // 最小の xlsx を書き、sheet1.xml に pageSetup / pageMargins を差して読み直す
        let b = Book::new();
        let mut buf = Cursor::new(Vec::new());
        write(&b, &mut buf).expect("書けない");
        let mut z = zip::ZipArchive::new(Cursor::new(buf.get_ref().clone())).unwrap();
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        use std::io::{Read as _, Write as _};
        for i in 0..z.len() {
            let mut f = z.by_index(i).unwrap();
            let name = f.name().to_string();
            let mut s = Vec::new();
            f.read_to_end(&mut s).unwrap();
            if name.ends_with("sheet1.xml") {
                let t = String::from_utf8(s).unwrap().replace(
                    "</worksheet>",
                    r#"<pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/><pageSetup paperSize="8" orientation="landscape"/></worksheet>"#,
                );
                s = t.into_bytes();
            }
            w.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(&s).unwrap();
        }
        let out = w.finish().unwrap();
        let (back, _) = read(Cursor::new(out.into_inner())).expect("読めない");
        let sh = &back.sheets[0];
        assert!(sh.landscape, "横向きが読めない");
        assert_eq!(sh.paper_size, Some(8), "用紙コードが読めない");
        let (l, _, t, _) = sh.margins_mm.expect("余白が読めない");
        assert!((l - 17.78).abs() < 0.01, "0.7インチ = 17.78mm でない: {l}");
        assert!((t - 19.05).abs() < 0.01, "{t}");
    }
}

#[cfg(test)]
mod print_setup_roundtrip_tests {
    use super::*;

    #[test]
    fn 印刷設定と印刷範囲がモデル経由で往復する() {
        let mut b = Book::new();
        b.sheets[0].set(Pos::parse("A1").unwrap(), Cell::input("x"));
        b.sheets[0].landscape = true;
        b.sheets[0].paper_size = Some(12);
        b.sheets[0].margins_mm = Some((10.0, 10.0, 20.0, 20.0));
        b.sheets[0]
            .print_areas
            .push((Pos::parse("A1").unwrap(), Pos::parse("G30").unwrap()));
        let mut buf = Cursor::new(Vec::new());
        write(&b, &mut buf).expect("書けない");
        buf.set_position(0);
        let (back, _) = read(buf).expect("読めない");
        let sh = &back.sheets[0];
        assert!(sh.landscape, "向きが往復しない");
        assert_eq!(sh.paper_size, Some(12), "用紙が往復しない");
        let (l, _, t, _) = sh.margins_mm.expect("余白が往復しない");
        assert!((l - 10.0).abs() < 0.05, "{l}");
        assert!((t - 20.0).abs() < 0.05, "{t}");
        assert_eq!(
            sh.print_areas,
            vec![(Pos::parse("A1").unwrap(), Pos::parse("G30").unwrap())],
            "印刷範囲が往復しない"
        );
    }

    #[test]
    fn 原文の知らない属性を消さずに向きだけ変わる() {
        // 拡大縮小(scale)付きの原本を読み、向きだけ変えて保存する
        let b0 = Book::new();
        let mut buf = Cursor::new(Vec::new());
        write(&b0, &mut buf).expect("書けない");
        let mut z = zip::ZipArchive::new(Cursor::new(buf.get_ref().clone())).unwrap();
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        use std::io::Write as _;
        for i in 0..z.len() {
            let mut f = z.by_index(i).unwrap();
            let name = f.name().to_string();
            let mut s = Vec::new();
            f.read_to_end(&mut s).unwrap();
            if name.ends_with("sheet1.xml") {
                let t = String::from_utf8(s).unwrap().replace(
                    "</worksheet>",
                    r#"<pageSetup paperSize="9" scale="85" orientation="landscape"/></worksheet>"#,
                );
                s = t.into_bytes();
            }
            w.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(&s).unwrap();
        }
        let original = w.finish().unwrap().into_inner();
        let (mut book, _) = read(Cursor::new(original.clone())).expect("読めない");
        assert!(book.sheets[0].landscape, "原本の向きが読めていない");
        book.sheets[0].landscape = false; // 縦に変える
        let mut out = Cursor::new(Vec::new());
        write_with(&book, Some(Cursor::new(original)), &mut out).expect("書けない");
        let mut z = zip::ZipArchive::new(Cursor::new(out.into_inner())).unwrap();
        let mut s = String::new();
        z.by_name("xl/worksheets/sheet1.xml").unwrap().read_to_string(&mut s).unwrap();
        assert!(s.contains(r#"scale="85""#), "知らない属性(scale)が消えた");
        assert!(s.contains(r#"orientation="portrait""#), "変えた向きが書かれていない");
        assert!(!s.contains("landscape"), "古い向きが残った");
    }
}

#[cfg(test)]
mod image_roundtrip_tests {
    use super::*;
    use crate::model::SheetImage;

    fn png() -> Vec<u8> {
        // 実体は問わない(読みは復号しない)。PNG の魔法数だけ本物
        let mut d = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        d.extend_from_slice(&[0; 32]);
        d
    }

    #[test]
    fn 挿した画像が往復する() {
        let mut b = Book::new();
        b.sheets[0].set(Pos::parse("A1").unwrap(), Cell::input("x"));
        b.sheets[0].images_new.push(SheetImage {
            at: Pos::new(2, 3),
            width_px: 300.0,
            height_px: 200.0,
            data: png(),
        });
        let mut buf = Cursor::new(Vec::new());
        write(&b, &mut buf).expect("書けない");
        buf.set_position(0);
        let (back, _) = read(buf).expect("読めない");
        let ims = &back.sheets[0].images;
        assert_eq!(ims.len(), 1, "画像が往復しない");
        assert_eq!(ims[0].at, Pos::new(2, 3), "錨のセルが違う");
        assert!((ims[0].width_px - 300.0).abs() < 1.0, "幅が違う: {}", ims[0].width_px);
        assert_eq!(ims[0].data, png(), "実体が化けた");
        assert!(back.sheets[0].images_new.is_empty(), "読んだ画像が「挿した側」に入った");
    }

    #[test]
    fn 画像入りの原本に足しても両方残る() {
        // 1枚入りを作る → それを原本にもう1枚足して保存 → 2枚とも読める
        let mut b = Book::new();
        b.sheets[0].set(Pos::parse("A1").unwrap(), Cell::input("x"));
        b.sheets[0].images_new.push(SheetImage {
            at: Pos::new(0, 0),
            width_px: 100.0,
            height_px: 50.0,
            data: png(),
        });
        let mut buf1 = Cursor::new(Vec::new());
        write(&b, &mut buf1).expect("書けない");
        buf1.set_position(0);
        let (mut b2, _) = read(buf1.clone()).expect("読めない");
        assert_eq!(b2.sheets[0].images.len(), 1);
        b2.sheets[0].images_new.push(SheetImage {
            at: Pos::new(5, 5),
            width_px: 200.0,
            height_px: 100.0,
            data: png(),
        });
        let mut buf2 = Cursor::new(Vec::new());
        buf1.set_position(0);
        write_with(&b2, Some(buf1), &mut buf2).expect("書けない");
        buf2.set_position(0);
        let (b3, _) = read(buf2).expect("読めない");
        assert_eq!(b3.sheets[0].images.len(), 2, "継ぎ足しで枚数が合わない");
        assert!(
            b3.sheets[0].images.iter().any(|im| im.at == Pos::new(5, 5)),
            "足した方の錨が無い"
        );
    }
}

#[cfg(test)]
mod print_extras_roundtrip_tests {
    use super::*;

    #[test]
    fn 拡大縮小と改ページとタイトル行が往復する() {
        let mut b = Book::new();
        b.sheets[0].set(Pos::parse("A1").unwrap(), Cell::input("x"));
        b.sheets[0].print_scale = Some(80);
        b.sheets[0].row_breaks = vec![10, 30];
        b.sheets[0].print_gridlines = true;
        b.sheets[0].print_headings = true;
        b.sheets[0].print_title_rows = Some((0, 1));
        let mut buf = Cursor::new(Vec::new());
        write(&b, &mut buf).expect("書けない");
        buf.set_position(0);
        let (back, _) = read(buf).expect("読めない");
        let sh = &back.sheets[0];
        assert_eq!(sh.print_scale, Some(80), "scale が往復しない");
        assert_eq!(sh.row_breaks, vec![10, 30], "改ページが往復しない");
        assert!(sh.print_gridlines && sh.print_headings, "printOptions が往復しない");
        assert_eq!(sh.print_title_rows, Some((0, 1)), "タイトル行が往復しない");
    }
}

#[cfg(test)]
mod shape_roundtrip_tests {
    use super::*;
    use crate::model::SheetShape;

    #[test]
    fn 挿した図形が往復する() {
        let mut b = Book::new();
        b.sheets[0].set(Pos::parse("A1").unwrap(), Cell::input("x"));
        b.sheets[0].shapes_new.push(SheetShape {
            at: Pos::new(1, 2),
            width_px: 160.0,
            height_px: 100.0,
            kind: "rightArrow".into(),
            fill: Some("FFF2CC".into()),
            line: Some("1B6E3C".into()),
            ..Default::default()
        });
        let mut buf = Cursor::new(Vec::new());
        write(&b, &mut buf).expect("書けない");
        buf.set_position(0);
        let (back, _) = read(buf).expect("読めない");
        let sp = &back.sheets[0].shapes;
        assert_eq!(sp.len(), 1, "図形が往復しない");
        assert_eq!(sp[0].kind, "rightArrow");
        assert_eq!(sp[0].at, Pos::new(1, 2));
        assert_eq!(sp[0].fill.as_deref(), Some("FFF2CC"));
        assert_eq!(sp[0].line.as_deref(), Some("1B6E3C"), "線の色が塗りと混ざった");
        assert!((sp[0].width_px - 160.0).abs() < 1.0);
        assert!(back.sheets[0].shapes_new.is_empty());
    }
}

#[cfg(test)]
mod textbox_spark_roundtrip_tests {
    use super::*;
    use crate::model::SheetShape;

    #[test]
    fn 文字入りの図形と折れ線が往復する() {
        let mut b = Book::new();
        b.sheets[0].set(Pos::parse("A1").unwrap(), Cell::input("x"));
        b.sheets[0].shapes_new.push(SheetShape {
            at: Pos::new(0, 5),
            width_px: 200.0,
            height_px: 80.0,
            kind: "rect".into(),
            line: Some("7F7F7F".into()),
            text: Some("注意: 締切は8/10 <厳守>".into()),
            ..Default::default()
        });
        b.sheets[0].shapes_new.push(SheetShape {
            at: Pos::new(3, 5),
            width_px: 108.0,
            height_px: 24.0,
            kind: "spark".into(),
            line: Some("1B6E3C".into()),
            points: vec![(0.0, 1.0), (0.5, 0.0), (1.0, 0.6)],
            ..Default::default()
        });
        let mut buf = Cursor::new(Vec::new());
        write(&b, &mut buf).expect("書けない");
        buf.set_position(0);
        let (back, _) = read(buf).expect("読めない");
        let sp = &back.sheets[0].shapes;
        assert_eq!(sp.len(), 2, "図形が往復しない: {sp:?}");
        let tb = sp.iter().find(|s| s.kind == "rect").expect("文字箱が無い");
        assert_eq!(tb.text.as_deref(), Some("注意: 締切は8/10 <厳守>"), "文字が化けた");
        let sk = sp.iter().find(|s| s.kind == "spark").expect("折れ線が無い");
        assert_eq!(sk.points.len(), 3);
        assert!((sk.points[1].0 - 0.5).abs() < 0.01 && sk.points[1].1.abs() < 0.01);
    }
}

#[cfg(test)]
mod script_roundtrip_tests {
    use super::*;

    #[test]
    fn ブックに載せたpythonが往復する() {
        let mut b = Book::new();
        b.sheets[0].set(Pos::parse("A1").unwrap(), Cell::input("x"));
        b.scripts.push((
            "集計".into(),
            "s[\"B5\"] = \"合計\"\nprint(1 < 2 and \"OK\")".into(),
        ));
        let mut buf = Cursor::new(Vec::new());
        write(&b, &mut buf).expect("書けない");
        buf.set_position(0);
        let (back, _) = read(buf.clone()).expect("読めない");
        assert_eq!(back.scripts.len(), 1, "控えが往復しない");
        assert_eq!(back.scripts[0].0, "集計");
        assert!(back.scripts[0].1.contains("1 < 2"), "コードの逃がしが壊れた");
        // もう一往復(古い部品と二重にならない)
        let mut buf2 = Cursor::new(Vec::new());
        buf.set_position(0);
        write_with(&back, Some(buf), &mut buf2).expect("書けない");
        buf2.set_position(0);
        let (b3, _) = read(buf2).expect("読めない");
        assert_eq!(b3.scripts.len(), 1, "二往復で二重になった");
    }
}

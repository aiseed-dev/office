//! docx ⇄ kumihan の文書モデル。
//!
//! 方針(SEKKEI.md「互換は書式の境界で守る」):
//!   - エンジンは継がない。**書式(docx)だけを読み書きする**
//!   - **全部は実装しない。読めないものは読めないと言う** —
//!     解釈できなかった要素は捨てずに `Report` に積んで返す。
//!     黙って落とすのが一番悪い(利用者は失われたことに気づけない)
//!
//! v0 の範囲: 本文の段落・ラン・文字サイズ・改行、そして**表**(w:tbl)。
//! 表は日本の事務様式の本体なので、v0 から入れる(実物8件すべてに表があった)。
//! 画像・ヘッダ/フッタ・スタイル定義・セル結合は**未対応として報告する**。

use std::io::{Cursor, Read, Seek, Write};

use kumihan::{Align, CharFormat, ListKind, Block, Cellbox, Document, Paragraph, Run, Table};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};

/// 読み書きで落ちた・触れなかったものの記録。
/// 「読めたつもり」を防ぐための帳簿。
#[derive(Debug, Default, Clone)]
pub struct Report {
    /// 未対応で無視した要素(w:tbl など)と回数
    pub unsupported: Vec<(String, usize)>,
    /// 段落数・ラン数
    pub paragraphs: usize,
    pub runs: usize,
}

impl Report {
    fn note(&mut self, name: &str) {
        if let Some(e) = self.unsupported.iter_mut().find(|(n, _)| n == name) {
            e.1 += 1;
        } else {
            self.unsupported.push((name.to_string(), 1));
        }
    }
    pub fn is_lossless(&self) -> bool {
        self.unsupported.is_empty()
    }
}

const DEFAULT_PT: f32 = 10.5;

/// `<w:b/>` は付ける、`<w:b w:val="0"/>` は付けない。
/// 有無だけで見ると「太字を解除した文書」を太字にしてしまう。
fn on(e: &quick_xml::events::BytesStart) -> bool {
    !matches!(attr(e, "val").as_deref(), Some("0") | Some("false") | Some("none"))
}

fn local(name: &[u8]) -> &[u8] {
    match name.iter().position(|b| *b == b':') {
        Some(i) => &name[i + 1..],
        None => name,
    }
}

fn attr<'a>(e: &'a BytesStart, want: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        if local(a.key.as_ref()) == want.as_bytes() {
            Some(String::from_utf8_lossy(&a.value).to_string())
        } else {
            None
        }
    })
}

// ---------- 読む ----------

/// docx を読む。返るのは文書と、読めなかったものの帳簿。
pub fn read<R: Read + Seek>(src: R) -> Result<(Document, Report), String> {
    let mut zip = zip::ZipArchive::new(src).map_err(|e| format!("zipを開けません: {e}"))?;
    let mut xml = String::new();
    zip.by_name("word/document.xml")
        .map_err(|_| "word/document.xml がありません(docxではない可能性)".to_string())?
        .read_to_string(&mut xml)
        .map_err(|e| format!("document.xml を読めません: {e}"))?;
    Ok(parse_document_xml(&xml))
}

/// 組み立て中の表。表は入れ子になりうるのでスタックで持つ。
#[derive(Default)]
struct TblBuild {
    rows: Vec<Vec<Cellbox>>,
    row: Vec<Cellbox>,
    cell: Vec<Paragraph>,
    /// 列幅(mm)。w:gridCol から
    col_mm: Vec<f32>,
}

/// twip → mm(1twip = 1/20pt)
fn twip_mm(v: f32) -> f32 {
    v * 25.4 / (20.0 * 72.0)
}

/// 原文が使う接頭辞(`wp14:` など)の宣言を付けた `<w:r>` に包む。
/// 解決できない接頭辞があれば None(壊れた XML を書かないため)。
fn wrap_with_ns(raw: &str, decls: &std::collections::BTreeMap<String, String>) -> Option<String> {
    // 既定で分かっているもの(原本に宣言が無くても標準の URI)
    const KNOWN: &[(&str, &str)] = &[
        ("w", "http://schemas.openxmlformats.org/wordprocessingml/2006/main"),
        ("r", "http://schemas.openxmlformats.org/officeDocument/2006/relationships"),
        ("wp", "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"),
        ("a", "http://schemas.openxmlformats.org/drawingml/2006/main"),
        ("pic", "http://schemas.openxmlformats.org/drawingml/2006/picture"),
        ("v", "urn:schemas-microsoft-com:vml"),
        ("o", "urn:schemas-microsoft-com:office:office"),
        ("w10", "urn:schemas-microsoft-com:office:word"),
        ("mc", "http://schemas.openxmlformats.org/markup-compatibility/2006"),
        ("wp14", "http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing"),
        ("w14", "http://schemas.microsoft.com/office/word/2010/wordml"),
        ("w15", "http://schemas.microsoft.com/office/word/2012/wordml"),
        ("wps", "http://schemas.microsoft.com/office/word/2010/wordprocessingShape"),
        ("wpg", "http://schemas.microsoft.com/office/word/2010/wordprocessingGroup"),
        ("a14", "http://schemas.microsoft.com/office/drawing/2010/main"),
    ];
    // 原文に出てくる接頭辞を拾う(要素名と属性名)
    let mut prefixes: std::collections::BTreeSet<String> = Default::default();
    let bytes = raw.as_bytes();
    let is_name = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'-';
    let mut i = 0;
    while i < bytes.len() {
        let at_elem = bytes[i] == b'<';
        let at_attr = bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\n';
        if at_elem || at_attr {
            let mut j = i + 1;
            if at_elem && j < bytes.len() && bytes[j] == b'/' {
                j += 1;
            }
            let s = j;
            while j < bytes.len() && is_name(bytes[j]) {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b':' && j > s {
                // 属性なら後ろに = が要る(値の中の URL を接頭辞と間違えない)
                let mut k = j + 1;
                while k < bytes.len() && is_name(bytes[k]) {
                    k += 1;
                }
                let ok = at_elem || (k < bytes.len() && bytes[k] == b'=');
                if ok {
                    prefixes.insert(raw[s..j].to_string());
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    prefixes.remove("xmlns");
    let mut out = String::from("<w:r");
    for p in &prefixes {
        if p == "w" {
            continue; // root で宣言済み
        }
        let uri = decls
            .get(p)
            .map(|s| s.as_str())
            .or_else(|| KNOWN.iter().find(|(k, _)| k == p).map(|(_, v)| *v))?;
        out.push_str(&format!(" xmlns:{p}=\"{uri}\""));
    }
    out.push('>');
    out.push_str(raw);
    out.push_str("</w:r>");
    Some(out)
}

pub fn parse_document_xml(xml: &str) -> (Document, Report) {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(false);

    let mut doc = Document::default();
    let mut rep = Report::default();
    let mut stack: Vec<TblBuild> = Vec::new();

    let mut para: Option<Vec<Run>> = None;
    let mut size_pt = DEFAULT_PT;
    // **書体は文書の設定**。docx が w:rFonts で持っているものを捨てない
    let mut font: Option<String> = None;
    // 文字の書式(w:rPr)と段落の揃え(w:jc)。読んで捨てると開き直したとき消える
    let mut fmt = CharFormat::default();
    let mut align = Align::default();
    // 箇条書き・インデント・行間(w:numPr / w:ind / w:spacing)
    let mut list = ListKind::default();
    let mut indent = 0u8;
    let mut line_spacing = 1.0f32;
    let mut page_break_before = false;
    // 読めなかった要素の原文(画像など)。段落ごとに集めて持ち越す
    let mut anchors: Vec<String> = Vec::new();
    // 原本の root が宣言している名前空間。持ち越す原文の接頭辞をこれで包む
    let mut ns_decls: std::collections::BTreeMap<String, String> = Default::default();
    let mut in_ppr = false;
    let mut in_text = false;
    let mut in_rpr = false;
    let mut cur = String::new();
    const SKIP_KNOWN: &[&str] = &["drawing", "pict", "object", "sdt"];

    let mut buf = Vec::new();
    // 直前のイベントの終わり位置(原文の切り出しに使う)
    let mut last_pos = r.buffer_position() as usize;
    loop {
        let ev = r.read_event_into(&mut buf);
        let start_pos = last_pos;
        last_pos = r.buffer_position() as usize;
        match ev {
            Err(e) => { rep.note(&format!("XML解析エラー: {e}")); break }
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let n = local(e.name().as_ref()).to_vec();
                match n.as_slice() {
                    b"document" => {
                        // root の xmlns:* を控える(画像の原文の接頭辞に要る)
                        for a in e.attributes().flatten() {
                            let k = String::from_utf8_lossy(a.key.as_ref()).to_string();
                            if let Some(pfx) = k.strip_prefix("xmlns:") {
                                if let Ok(v) = a.unescape_value() {
                                    ns_decls.insert(pfx.to_string(), v.to_string());
                                }
                            }
                        }
                    }
                    b"tbl" => stack.push(TblBuild::default()),
                    b"gridCol" => if let Some(b) = stack.last_mut() {
                        if let Some(w) = attr(&e, "w").and_then(|v| v.parse::<f32>().ok()) {
                            b.col_mm.push(twip_mm(w));
                        }
                    },
                    b"tr" => if let Some(b) = stack.last_mut() { b.row.clear() },
                    b"tc" => if let Some(b) = stack.last_mut() { b.cell.clear() },
                    b"p" => { para = Some(Vec::new()); size_pt = DEFAULT_PT; font = None;
                              fmt = CharFormat::default(); align = Align::default();
                              list = ListKind::default(); indent = 0; line_spacing = 1.0;
                              page_break_before = false; }
                    b"rPr" => { in_rpr = true; fmt = CharFormat::default(); }
                    b"pPr" => in_ppr = true,
                    b"sz" if in_rpr => {
                        if let Some(v) = attr(&e, "val") {
                            if let Ok(h) = v.parse::<f32>() { size_pt = h / 2.0; }
                        }
                    }
                    // 日本語の書体は eastAsia に入る。ascii しか見ないと明朝が消える
                    b"rFonts" if in_rpr => {
                        font = attr(&e, "eastAsia")
                            .or_else(|| attr(&e, "ascii"))
                            .or_else(|| attr(&e, "hAnsi"))
                            .filter(|s| !s.is_empty());
                    }
                    // w:val="0"/"false" は「付けない」の意味なので、有無だけで判定しない
                    b"b" if in_rpr => fmt.bold = on(&e),
                    b"i" if in_rpr => fmt.italic = on(&e),
                    b"u" if in_rpr => fmt.underline = attr(&e, "val").as_deref() != Some("none"),
                    b"strike" if in_rpr => fmt.strike = on(&e),
                    b"color" if in_rpr => {
                        fmt.color = attr(&e, "val").filter(|v| !v.is_empty() && v != "auto");
                    }
                    // 箇条書きは numId で決まる。1 を中黒、2 を段落番号として扱う
                    // (numbering.xml を持たないので、往復できる最小の約束にしてある)
                    b"numId" if in_ppr => {
                        list = match attr(&e, "val").as_deref() {
                            Some("2") => ListKind::Number,
                            Some("0") | None => ListKind::None,
                            _ => ListKind::Bullet,
                        };
                    }
                    b"pageBreakBefore" if in_ppr => {
                        page_break_before = on(&e);
                    }
                    b"ind" if in_ppr => {
                        // twip。1段 = 全角2文字 = 10.5pt×2 ≒ 420twip
                        indent = attr(&e, "left")
                            .and_then(|v| v.parse::<f32>().ok())
                            .map(|v| (v / 420.0).round().clamp(0.0, 20.0) as u8)
                            .unwrap_or(0);
                    }
                    b"spacing" if in_ppr => {
                        // w:line は 240 = 1行
                        line_spacing = attr(&e, "line")
                            .and_then(|v| v.parse::<f32>().ok())
                            .map(|v| (v / 240.0).clamp(0.5, 5.0))
                            .unwrap_or(1.0);
                    }
                    b"jc" if in_ppr => {
                        if let Some(v) = attr(&e, "val") { align = Align::from_docx(&v); }
                    }
                    b"t" => { in_text = true; cur.clear(); }
                    b"vMerge" | b"gridSpan" => rep.note("セル結合(w:vMerge/w:gridSpan)"),
                    b"drawing" | b"pict" | b"object" => {
                        // **理解はしないが、捨てない。** 原文を丸ごと控えて保存で返す。
                        // 部分木は読み飛ばす — 中の a:t(図形の文字)を本文に
                        // 混ぜないため(以前はここから本文へ漏れていた)
                        let name = e.name().to_owned();
                        if r.read_to_end_into(name, &mut Vec::new()).is_ok() {
                            let end = r.buffer_position() as usize;
                            let raw = &xml[start_pos..end];
                            // 原文が使う接頭辞の宣言を、包む run に付ける。
                            // 付けないと保存した XML が「未宣言の接頭辞」で壊れる
                            match wrap_with_ns(raw, &ns_decls) {
                                Some(wrapped) => {
                                    anchors.push(wrapped);
                                    rep.note("画像・図形(保存では残る。表示は未対応)");
                                }
                                None => {
                                    // 出どころの分からない接頭辞。壊れたXMLを書くより
                                    // 落として報告する方がまし
                                    rep.note("画像・図形(接頭辞が解決できず、保存で失われる)");
                                }
                            }
                            last_pos = end;
                        }
                    }
                    b"sdt" => rep.note("w:sdt"),
                    other => {
                        let _ = other;
                    }
                }
            }
            Ok(Event::Empty(e)) => {
                let n = local(e.name().as_ref()).to_vec();
                match n.as_slice() {
                    // gridCol は空要素で来る
                    b"gridCol" => if let Some(b) = stack.last_mut() {
                        if let Some(w) = attr(&e, "w").and_then(|v| v.parse::<f32>().ok()) {
                            b.col_mm.push(twip_mm(w));
                        }
                    },
                    b"br" => if let Some(p) = para.as_mut() {
                        p.push(Run { text: "\n".into(), size_pt, font: font.clone(), fmt: fmt.clone() }) },
                    b"tab" => if let Some(p) = para.as_mut() {
                        p.push(Run { text: "\t".into(), size_pt, font: font.clone(), fmt: fmt.clone() }) },
                    b"sz" if in_rpr => {
                        if let Some(v) = attr(&e, "val") {
                            if let Ok(h) = v.parse::<f32>() { size_pt = h / 2.0; }
                        }
                    }
                    b"rFonts" if in_rpr => {
                        font = attr(&e, "eastAsia")
                            .or_else(|| attr(&e, "ascii"))
                            .or_else(|| attr(&e, "hAnsi"))
                            .filter(|s| !s.is_empty());
                    }
                    // w:val="0"/"false" は「付けない」の意味なので、有無だけで判定しない
                    b"b" if in_rpr => fmt.bold = on(&e),
                    b"i" if in_rpr => fmt.italic = on(&e),
                    b"u" if in_rpr => fmt.underline = attr(&e, "val").as_deref() != Some("none"),
                    b"strike" if in_rpr => fmt.strike = on(&e),
                    b"color" if in_rpr => {
                        fmt.color = attr(&e, "val").filter(|v| !v.is_empty() && v != "auto");
                    }
                    // 箇条書きは numId で決まる。1 を中黒、2 を段落番号として扱う
                    // (numbering.xml を持たないので、往復できる最小の約束にしてある)
                    b"numId" if in_ppr => {
                        list = match attr(&e, "val").as_deref() {
                            Some("2") => ListKind::Number,
                            Some("0") | None => ListKind::None,
                            _ => ListKind::Bullet,
                        };
                    }
                    b"pageBreakBefore" if in_ppr => {
                        page_break_before = on(&e);
                    }
                    b"ind" if in_ppr => {
                        // twip。1段 = 全角2文字 = 10.5pt×2 ≒ 420twip
                        indent = attr(&e, "left")
                            .and_then(|v| v.parse::<f32>().ok())
                            .map(|v| (v / 420.0).round().clamp(0.0, 20.0) as u8)
                            .unwrap_or(0);
                    }
                    b"spacing" if in_ppr => {
                        // w:line は 240 = 1行
                        line_spacing = attr(&e, "line")
                            .and_then(|v| v.parse::<f32>().ok())
                            .map(|v| (v / 240.0).clamp(0.5, 5.0))
                            .unwrap_or(1.0);
                    }
                    b"jc" if in_ppr => {
                        if let Some(v) = attr(&e, "val") { align = Align::from_docx(&v); }
                    }
                    b"vMerge" | b"gridSpan" => rep.note("セル結合(w:vMerge/w:gridSpan)"),
                    b"drawing" | b"pict" | b"object" =>
                        rep.note(&format!("w:{}", String::from_utf8_lossy(&n))),
                    _ => {}
                }
            }
            Ok(Event::Text(t)) if in_text => cur.push_str(&t.unescape().unwrap_or_default()),
            Ok(Event::End(e)) => {
                let n = local(e.name().as_ref()).to_vec();
                match n.as_slice() {
                    b"t" => {
                        in_text = false;
                        if !cur.is_empty() {
                            if let Some(p) = para.as_mut() {
                                p.push(Run { text: std::mem::take(&mut cur), size_pt, font: font.clone(), fmt: fmt.clone() });
                            }
                        }
                    }
                    b"rPr" => in_rpr = false,
                    b"pPr" => in_ppr = false,
                    b"p" => {
                        if let Some(runs) = para.take() {
                            rep.runs += runs.len();
                            rep.paragraphs += 1;
                            let p = Paragraph { align, anchors: std::mem::take(&mut anchors),
                                page_break_before, list, indent, line_spacing, runs: if runs.is_empty() {
                                vec![Run { text: String::new(), size_pt: DEFAULT_PT, font: None, fmt: Default::default() }]
                            } else { runs } };
                            // 表のセルの中なら、そのセルへ。外なら本文へ
                            match stack.last_mut() {
                                Some(b) => b.cell.push(p),
                                None => doc.push_para(p),
                            }
                        }
                    }
                    b"tc" => if let Some(b) = stack.last_mut() {
                        let paras = std::mem::take(&mut b.cell);
                        b.row.push(Cellbox { paragraphs: paras });
                    },
                    b"tr" => if let Some(b) = stack.last_mut() {
                        let row = std::mem::take(&mut b.row);
                        b.rows.push(row);
                    },
                    b"tbl" => {
                        if let Some(b) = stack.pop() {
                            let tb = Table { rows: b.rows, col_mm: b.col_mm };
                            if stack.is_empty() {
                                doc.blocks.push(Block::Table(tb));
                            } else {
                                // 入れ子の表は v0 では本文の流れに出す(報告つき)
                                rep.note("入れ子の表(親セルの外に出した)");
                                doc.blocks.push(Block::Table(tb));
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        buf.clear();
    }
    (doc, rep)
}

// ---------- 書く ----------

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;

const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

/// 文書を document.xml の本体にする。
pub fn write_document_xml(doc: &Document) -> String {
    use quick_xml::events::{BytesEnd, BytesStart as BS};
    let mut w = Writer::new(Cursor::new(Vec::new()));

    fn para(w: &mut Writer<Cursor<Vec<u8>>>, p: &Paragraph) {
        use quick_xml::events::{BytesEnd, BytesStart as BS, BytesText};
        w.write_event(Event::Start(BS::new("w:p"))).unwrap();
        // 段落の性質。既定のものは書かない — 余計な指定を増やさない
        let has_ppr = p.align != Align::Left
            || p.page_break_before
            || p.list != ListKind::None
            || p.indent > 0
            || (p.spacing() - 1.0).abs() > 0.001;
        if has_ppr {
            w.write_event(Event::Start(BS::new("w:pPr"))).unwrap();
            if p.page_break_before {
                w.write_event(Event::Empty(BS::new("w:pageBreakBefore"))).unwrap();
            }
            if p.list != ListKind::None {
                w.write_event(Event::Start(BS::new("w:numPr"))).unwrap();
                let mut lv = BS::new("w:ilvl");
                lv.push_attribute(("w:val", "0"));
                w.write_event(Event::Empty(lv)).unwrap();
                let mut id = BS::new("w:numId");
                id.push_attribute(("w:val", if p.list == ListKind::Number { "2" } else { "1" }));
                w.write_event(Event::Empty(id)).unwrap();
                w.write_event(Event::End(BytesEnd::new("w:numPr"))).unwrap();
            }
            if p.indent > 0 {
                let mut ind = BS::new("w:ind");
                ind.push_attribute(("w:left", (p.indent as u32 * 420).to_string().as_str()));
                w.write_event(Event::Empty(ind)).unwrap();
            }
            if (p.spacing() - 1.0).abs() > 0.001 {
                let mut sp = BS::new("w:spacing");
                sp.push_attribute(("w:line", ((p.spacing() * 240.0).round() as u32).to_string().as_str()));
                sp.push_attribute(("w:lineRule", "auto"));
                w.write_event(Event::Empty(sp)).unwrap();
            }
            if p.align != Align::Left {
                let mut jc = BS::new("w:jc");
                jc.push_attribute(("w:val", p.align.as_docx()));
                w.write_event(Event::Empty(jc)).unwrap();
            }
            w.write_event(Event::End(BytesEnd::new("w:pPr"))).unwrap();
        }
        // 読めなかった要素(画像など)を原文のまま返す。
        // 段落の並びの中の位置は失われ、末尾に戻る(正直な限界)
        for a in &p.anchors {
            // anchors は宣言付きの <w:r>…</w:r> を丸ごと持っている
            let _ = w.get_mut().write_all(a.as_bytes());
        }
        for run in &p.runs {
            if run.text.is_empty() { continue }
            w.write_event(Event::Start(BS::new("w:r"))).unwrap();
            w.write_event(Event::Start(BS::new("w:rPr"))).unwrap();
            // 書体は文書の設定なので、読んだものをそのまま返す。
            // 日本語は eastAsia が本体。ascii/hAnsi にも同じ名前を入れておかないと
            // Word が英数字だけ別の書体で出す
            if let Some(f) = &run.font {
                let mut rf = BS::new("w:rFonts");
                rf.push_attribute(("w:ascii", f.as_str()));
                rf.push_attribute(("w:hAnsi", f.as_str()));
                rf.push_attribute(("w:eastAsia", f.as_str()));
                w.write_event(Event::Empty(rf)).unwrap();
            }
            // 文字の書式。付いているものだけ書く
            if run.fmt.bold { w.write_event(Event::Empty(BS::new("w:b"))).unwrap() }
            if run.fmt.italic { w.write_event(Event::Empty(BS::new("w:i"))).unwrap() }
            if run.fmt.underline {
                let mut u = BS::new("w:u");
                u.push_attribute(("w:val", "single"));
                w.write_event(Event::Empty(u)).unwrap();
            }
            if run.fmt.strike { w.write_event(Event::Empty(BS::new("w:strike"))).unwrap() }
            if let Some(c) = &run.fmt.color {
                let mut col = BS::new("w:color");
                col.push_attribute(("w:val", c.as_str()));
                w.write_event(Event::Empty(col)).unwrap();
            }
            let mut sz = BS::new("w:sz");
            sz.push_attribute(("w:val",
                format!("{}", (run.size_pt * 2.0).round() as i32).as_str()));
            w.write_event(Event::Empty(sz)).unwrap();
            w.write_event(Event::End(BytesEnd::new("w:rPr"))).unwrap();
            for (i, seg) in run.text.split('\n').enumerate() {
                if i > 0 { w.write_event(Event::Empty(BS::new("w:br"))).unwrap(); }
                if seg.is_empty() { continue }
                let mut t = BS::new("w:t");
                t.push_attribute(("xml:space", "preserve"));
                w.write_event(Event::Start(t)).unwrap();
                w.write_event(Event::Text(BytesText::new(seg))).unwrap();
                w.write_event(Event::End(BytesEnd::new("w:t"))).unwrap();
            }
            w.write_event(Event::End(BytesEnd::new("w:r"))).unwrap();
        }
        w.write_event(Event::End(BytesEnd::new("w:p"))).unwrap();
    }

    let mut root = BS::new("w:document");
    root.push_attribute(("xmlns:w", W_NS));
    w.write_event(Event::Start(root)).unwrap();
    w.write_event(Event::Start(BS::new("w:body"))).unwrap();

    for b in &doc.blocks {
        match b {
            Block::Para(p) => para(&mut w, p),
            Block::Table(t) => {
                w.write_event(Event::Start(BS::new("w:tbl"))).unwrap();
                // 罫線(事務様式は罫線が見えないと様式にならない)
                w.write_event(Event::Start(BS::new("w:tblPr"))).unwrap();
                w.write_event(Event::Start(BS::new("w:tblBorders"))).unwrap();
                for side in ["top", "left", "bottom", "right", "insideH", "insideV"] {
                    let tag = format!("w:{side}");
                    let mut e = BS::new(tag.as_str());
                    e.push_attribute(("w:val", "single"));
                    e.push_attribute(("w:sz", "4"));
                    e.push_attribute(("w:color", "000000"));
                    w.write_event(Event::Empty(e)).unwrap();
                }
                w.write_event(Event::End(BytesEnd::new("w:tblBorders"))).unwrap();
                w.write_event(Event::End(BytesEnd::new("w:tblPr"))).unwrap();
                // 列幅を返す(読んだものを捨てると、保存で表の形が変わる)
                if !t.col_mm.is_empty() {
                    w.write_event(Event::Start(BS::new("w:tblGrid"))).unwrap();
                    for mm in &t.col_mm {
                        let mut g = BS::new("w:gridCol");
                        let tw = (mm * 20.0 * 72.0 / 25.4).round() as i64;
                        g.push_attribute(("w:w", tw.to_string().as_str()));
                        w.write_event(Event::Empty(g)).unwrap();
                    }
                    w.write_event(Event::End(BytesEnd::new("w:tblGrid"))).unwrap();
                }
                for row in &t.rows {
                    w.write_event(Event::Start(BS::new("w:tr"))).unwrap();
                    for cell in row {
                        w.write_event(Event::Start(BS::new("w:tc"))).unwrap();
                        if cell.paragraphs.is_empty() {
                            para(&mut w, &Paragraph { align: Default::default(), anchors: Vec::new(), page_break_before: false,
                    list: Default::default(), indent: 0, line_spacing: 1.0, runs: Vec::new() });
                        } else {
                            for p in &cell.paragraphs { para(&mut w, p) }
                        }
                        w.write_event(Event::End(BytesEnd::new("w:tc"))).unwrap();
                    }
                    w.write_event(Event::End(BytesEnd::new("w:tr"))).unwrap();
                }
                w.write_event(Event::End(BytesEnd::new("w:tbl"))).unwrap();
            }
        }
    }

    w.write_event(Event::End(BytesEnd::new("w:body"))).unwrap();
    w.write_event(Event::End(BytesEnd::new("w:document"))).unwrap();
    let body = String::from_utf8(w.into_inner().into_inner()).unwrap();
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n{body}")
}

/// docx として書き出す(最小の OPC パッケージ)。
pub fn write<W: Write + Seek>(doc: &Document, dst: W) -> Result<(), String> {
    write_with(doc, None::<std::io::Cursor<Vec<u8>>>, dst)
}

/// 保存する。`original` に**開いた元のファイル**を渡すと、
/// こちらが作り直す `word/document.xml` 以外の部品
/// (画像の実体・スタイル・ヘッダー・フッター・設定)を**そのまま持ち越す**。
///
/// 渡さないと部品ごと消える — 「開いて保存したらロゴが消えた」は
/// 書類の事故として重いので、アプリからは必ず元を渡すこと。
pub fn write_with<R: Read + Seek, W: Write + Seek>(
    doc: &Document,
    original: Option<R>,
    dst: W,
) -> Result<(), String> {
    let mut zip = zip::ZipWriter::new(dst);
    let opts: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut copied = false;
    if let Some(src) = original {
        if let Ok(mut z) = zip::ZipArchive::new(src) {
            for i in 0..z.len() {
                let mut f = match z.by_index(i) {
                    Ok(f) => f,
                    Err(_) => continue,
                };
                let name = f.name().to_string();
                // 本文だけがこちらの管轄。他の部品は原本のまま
                if name == "word/document.xml" {
                    continue;
                }
                let mut buf = Vec::new();
                if f.read_to_end(&mut buf).is_err() {
                    continue;
                }
                zip.start_file(name, opts).map_err(|e| e.to_string())?;
                zip.write_all(&buf).map_err(|e| e.to_string())?;
                copied = true;
            }
        }
    }
    if !copied {
        zip.start_file("[Content_Types].xml", opts).map_err(|e| e.to_string())?;
        zip.write_all(CONTENT_TYPES.as_bytes()).map_err(|e| e.to_string())?;
        zip.start_file("_rels/.rels", opts).map_err(|e| e.to_string())?;
        zip.write_all(ROOT_RELS.as_bytes()).map_err(|e| e.to_string())?;
    }
    zip.start_file("word/document.xml", opts).map_err(|e| e.to_string())?;
    zip.write_all(write_document_xml(doc).as_bytes()).map_err(|e| e.to_string())?;
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

// ---------- 検査 ----------

#[cfg(test)]
mod tests {
    use super::*;
    use kumihan::{Align, CharFormat, ListKind, Block, Cellbox, Document, Paragraph, Run, Table};

    fn para(s: &str) -> Paragraph {
        Paragraph { align: Default::default(), anchors: Vec::new(), page_break_before: false,
                    list: Default::default(), indent: 0, line_spacing: 1.0, runs: vec![Run { text: s.to_string(), size_pt: 10.5, font: None, fmt: Default::default() }] }
    }
    fn doc(parts: &[&str]) -> Document {
        Document { font: None, blocks: parts.iter().map(|s| Block::Para(para(s))).collect() }
    }
    fn round_trip(d: &Document) -> (Document, Report) {
        let mut buf = Cursor::new(Vec::new());
        write(d, &mut buf).expect("書けない");
        buf.set_position(0);
        read(buf).expect("読めない")
    }
    fn texts(d: &Document) -> Vec<String> {
        d.paragraphs().map(|p| p.runs.iter().map(|r| r.text.as_str()).collect()).collect()
    }

    #[test]
    fn 日本語の段落が往復する() {
        let d = doc(&[
            "日本フネン株式会社 設備利用申込",
            "事業者名: 〇〇工務店",
            "「防火ドアは、特定防火設備です。」",
        ]);
        let (back, rep) = round_trip(&d);
        assert_eq!(texts(&back), vec![
            "日本フネン株式会社 設備利用申込",
            "事業者名: 〇〇工務店",
            "「防火ドアは、特定防火設備です。」",
        ]);
        assert!(rep.is_lossless(), "未対応が出た: {:?}", rep.unsupported);
    }

    #[test]
    fn 文字サイズが保たれる() {
        let d = Document { font: None, blocks: vec![Block::Para(Paragraph { align: Default::default(), anchors: Vec::new(), page_break_before: false,
                    list: Default::default(), indent: 0, line_spacing: 1.0, runs: vec![
            Run { text: "大見出し".into(), size_pt: 16.0, font: None, fmt: Default::default() },
            Run { text: "本文".into(), size_pt: 10.5, font: None, fmt: Default::default() },
        ]})]};
        let (back, _) = round_trip(&d);
        let runs = &back.paragraphs().next().unwrap().runs;
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].size_pt, 16.0);
        assert_eq!(runs[1].size_pt, 10.5);
    }

    #[test]
    fn 空段落は空段落のまま残る() {
        let (back, _) = round_trip(&doc(&["一", "", "三"]));
        assert_eq!(texts(&back), vec!["一", "", "三"]);
    }

    #[test]
    fn 前後の空白が消えない() {
        let (back, _) = round_trip(&doc(&["氏名　　:  山田 太郎 "]));
        assert_eq!(texts(&back)[0], "氏名　　:  山田 太郎 ");
    }

    #[test]
    fn xmlの特殊文字が壊れない() {
        let (back, _) = round_trip(&doc(&["A&B <タグ> \"引用\" 'アポ'"]));
        assert_eq!(texts(&back)[0], "A&B <タグ> \"引用\" 'アポ'");
    }

    #[test]
    fn 段落内の改行が保たれる() {
        let d = Document { font: None, blocks: vec![Block::Para(Paragraph { align: Default::default(), anchors: Vec::new(), page_break_before: false,
                    list: Default::default(), indent: 0, line_spacing: 1.0, runs: vec![
            Run { text: "一行目\n二行目".into(), size_pt: 10.5, font: None, fmt: Default::default() }]})]};
        let (back, _) = round_trip(&d);
        assert_eq!(texts(&back)[0], "一行目\n二行目");
    }

    // ---- 表: 日本の事務様式の本体(実物8件すべてに w:tbl があった) ----

    fn cell(s: &str) -> Cellbox { Cellbox { paragraphs: vec![para(s)] } }

    #[test]
    fn 表が往復する() {
        let d = Document { font: None, blocks: vec![
            Block::Para(para("(様式3) 会社概要")),
            Block::Table(Table { col_mm: vec![], rows: vec![
                vec![cell("会　社　名"), cell("日本フネン株式会社")],
                vec![cell("所　在　地"), cell("徳島県吉野川市川島町三ツ島新田179-1")],
                vec![cell("資　本　金"), cell("3億1,400万円")],
            ]}),
            Block::Para(para("以上")),
        ]};
        let (back, rep) = round_trip(&d);
        let t: Vec<&Table> = back.tables().collect();
        assert_eq!(t.len(), 1, "表が1つ戻る");
        assert_eq!(t[0].rows.len(), 3);
        assert_eq!(t[0].rows[1].len(), 2);
        let v: String = t[0].rows[1][1].paragraphs[0].runs.iter()
            .map(|r| r.text.as_str()).collect();
        assert_eq!(v, "徳島県吉野川市川島町三ツ島新田179-1");
        assert_eq!(texts(&back), vec!["(様式3) 会社概要", "以上"], "本文の順序も保たれる");
        assert!(rep.is_lossless(), "未対応: {:?}", rep.unsupported);
    }

    #[test]
    fn 表と本文の順序が保たれる() {
        let d = Document { font: None, blocks: vec![
            Block::Para(para("前")),
            Block::Table(Table { col_mm: vec![], rows: vec![vec![cell("表1")]] }),
            Block::Para(para("中")),
            Block::Table(Table { col_mm: vec![], rows: vec![vec![cell("表2")]] }),
            Block::Para(para("後")),
        ]};
        let (back, _) = round_trip(&d);
        let kinds: Vec<&str> = back.blocks.iter().map(|b| match b {
            Block::Para(_) => "段落", Block::Table(_) => "表" }).collect();
        assert_eq!(kinds, vec!["段落", "表", "段落", "表", "段落"]);
    }

    #[test]
    fn 空セルも列として残る() {
        // 事務様式は「記入欄が空の表」が本体。空セルが消えると様式が壊れる
        let d = Document { font: None, blocks: vec![Block::Table(Table { col_mm: vec![], rows: vec![
            vec![cell("氏名"), Cellbox::default()],
            vec![cell("所属"), Cellbox::default()],
        ]})]};
        let (back, _) = round_trip(&d);
        let t: Vec<&Table> = back.tables().collect();
        assert_eq!(t[0].rows.len(), 2);
        assert_eq!(t[0].rows[0].len(), 2, "空セルが消えた");
        assert_eq!(t[0].rows[1].len(), 2, "空セルが消えた");
    }

    #[test]
    fn 読めない要素は黙って落とさず報告する() {
        let xml = r#"<?xml version="1.0"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office"><w:body>
<w:p><w:r><w:t>前</w:t></w:r></w:p>
<w:p><w:r><w:drawing/></w:r></w:p>
<w:tbl><w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr>
<w:p><w:r><w:t>結合セル</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
</w:body></w:document>"#;
        let (_, rep) = parse_document_xml(xml);
        assert!(!rep.is_lossless());
        assert!(rep.unsupported.iter().any(|(n, _)| n.contains("drawing")),
            "画像の未対応が報告されていない: {:?}", rep.unsupported);
        assert!(rep.unsupported.iter().any(|(n, _)| n.contains("セル結合")),
            "セル結合の未対応が報告されていない: {:?}", rep.unsupported);
    }
}

#[cfg(test)]
mod font_tests {
    use kumihan::{Align, CharFormat, ListKind, Block, Document, Paragraph, Run};

    #[test]
    fn 書体名が往復する() {
        // **フォントは文書の設定。** 読んで捨てると、開き直したとき別の字になる
        let doc = Document {
            font: None,
            blocks: vec![Block::Para(Paragraph {
                align: Default::default(),
                anchors: Vec::new(),
                page_break_before: false,
                    list: Default::default(),
                indent: 0,
                line_spacing: 1.0,
                runs: vec![Run {
                    text: "日本フネン".into(),
                    size_pt: 10.5,
                    font: Some("BIZ UDPゴシック".into()),
                    fmt: Default::default(),
                }],
            })],
        };
        let mut buf = Vec::new();
        crate::write(&doc, std::io::Cursor::new(&mut buf)).unwrap();
        let (back, _) = crate::read(std::io::Cursor::new(&buf)).unwrap();
        let run = back.paragraphs().next().unwrap().runs.first().unwrap();
        assert_eq!(run.font.as_deref(), Some("BIZ UDPゴシック"), "書体名が消えた");
    }

    #[test]
    fn 日本語の書体は_eastasia_から読む() {
        // ascii しか見ないと、日本語の明朝指定を落とす
        let xml = r#"<w:document xmlns:w="x"><w:body><w:p><w:r><w:rPr>
            <w:rFonts w:ascii="Century" w:eastAsia="ＭＳ 明朝"/></w:rPr>
            <w:t>本文</w:t></w:r></w:p></w:body></w:document>"#;
        let doc = crate::parse_document_xml(xml).0;
        let run = doc.paragraphs().next().unwrap().runs.first().unwrap();
        assert_eq!(run.font.as_deref(), Some("ＭＳ 明朝"), "eastAsia を見ていない");
    }
}

#[cfg(test)]
mod fmt_tests {
    use kumihan::{Align, Block, CharFormat, Document, Paragraph, Run};

    fn run(text: &str, fmt: CharFormat) -> Run {
        Run { text: text.into(), size_pt: 10.5, font: None, fmt }
    }

    fn roundtrip(doc: &Document) -> Document {
        let mut buf = Vec::new();
        crate::write(doc, std::io::Cursor::new(&mut buf)).unwrap();
        crate::read(std::io::Cursor::new(&buf)).unwrap().0
    }

    #[test]
    fn 太字と斜体と下線が往復する() {
        let f = CharFormat { bold: true, italic: true, underline: true, ..Default::default() };
        let d = Document {
            font: None,
            blocks: vec![Block::Para(Paragraph { align: Align::Left, anchors: Vec::new(), page_break_before: false,
                    list: Default::default(), indent: 0, line_spacing: 1.0, runs: vec![run("見出し", f.clone())] })],
        };
        let back = roundtrip(&d);
        assert_eq!(back.paragraphs().next().unwrap().runs[0].fmt, f, "書式が消えた");
    }

    #[test]
    fn 取り消し線と文字色が往復する() {
        let f = CharFormat { strike: true, color: Some("FF0000".into()), ..Default::default() };
        let d = Document {
            font: None,
            blocks: vec![Block::Para(Paragraph { align: Align::Left, anchors: Vec::new(), page_break_before: false,
                    list: Default::default(), indent: 0, line_spacing: 1.0, runs: vec![run("赤", f.clone())] })],
        };
        assert_eq!(roundtrip(&d).paragraphs().next().unwrap().runs[0].fmt, f);
    }

    #[test]
    fn 中央揃えが往復する() {
        for a in [Align::Center, Align::Right, Align::Justify, Align::Left] {
            let d = Document {
                font: None,
                blocks: vec![Block::Para(Paragraph {
                    align: a,
                    anchors: Vec::new(),
                    page_break_before: false,
                    list: Default::default(),
                    indent: 0,
                    line_spacing: 1.0,
                    runs: vec![run("表題", CharFormat::default())],
                })],
            };
            assert_eq!(roundtrip(&d).paragraphs().next().unwrap().align, a, "{a:?} が消えた");
        }
    }

    #[test]
    fn 解除された太字を太字にしない() {
        // <w:b w:val="0"/> は「太字ではない」。有無だけで見ると間違える
        let xml = r#"<w:document xmlns:w="x"><w:body><w:p><w:r><w:rPr>
            <w:b w:val="0"/><w:i/></w:rPr><w:t>本文</w:t></w:r></w:p></w:body></w:document>"#;
        let doc = crate::parse_document_xml(xml).0;
        let f = &doc.paragraphs().next().unwrap().runs[0].fmt;
        assert!(!f.bold, "w:val=0 の太字を太字にした");
        assert!(f.italic, "斜体を落とした");
    }

    #[test]
    fn 段落ごとに書式が混ざらない() {
        // 前の段落の太字が次に漏れないこと
        let xml = r#"<w:document xmlns:w="x"><w:body>
            <w:p><w:pPr><w:jc w:val="center"/></w:pPr>
                <w:r><w:rPr><w:b/></w:rPr><w:t>表題</w:t></w:r></w:p>
            <w:p><w:r><w:t>本文</w:t></w:r></w:p>
        </w:body></w:document>"#;
        let doc = crate::parse_document_xml(xml).0;
        let ps: Vec<_> = doc.paragraphs().collect();
        assert!(ps[0].runs[0].fmt.bold);
        assert_eq!(ps[0].align, Align::Center);
        assert!(!ps[1].runs[0].fmt.bold, "太字が次の段落へ漏れた");
        assert_eq!(ps[1].align, Align::Left, "揃えが次の段落へ漏れた");
    }
}

#[cfg(test)]
mod para_tests {
    use kumihan::{Align, Block, Document, ListKind, Paragraph, Run};

    fn para(list: ListKind, indent: u8, spacing: f32) -> Paragraph {
        Paragraph {
            align: Align::Left,
            anchors: Vec::new(),
            page_break_before: false,
            list,
            indent,
            line_spacing: spacing,
            runs: vec![Run {
                text: "項目".into(), size_pt: 10.5, font: None, fmt: Default::default(),
            }],
        }
    }

    fn roundtrip(p: Paragraph) -> Paragraph {
        let d = Document { font: None, blocks: vec![Block::Para(p)] };
        let mut buf = Vec::new();
        crate::write(&d, std::io::Cursor::new(&mut buf)).unwrap();
        crate::read(std::io::Cursor::new(&buf)).unwrap().0.paragraphs().next().unwrap().clone()
    }

    #[test]
    fn 箇条書きが往復する() {
        assert_eq!(roundtrip(para(ListKind::Bullet, 0, 1.0)).list, ListKind::Bullet);
        assert_eq!(roundtrip(para(ListKind::Number, 0, 1.0)).list, ListKind::Number);
        assert_eq!(roundtrip(para(ListKind::None, 0, 1.0)).list, ListKind::None);
    }

    #[test]
    fn インデントが往復する() {
        for n in [1u8, 3, 8] {
            assert_eq!(roundtrip(para(ListKind::None, n, 1.0)).indent, n, "{n}段が消えた");
        }
    }

    #[test]
    fn 行間が往復する() {
        for s in [1.5f32, 2.0] {
            let got = roundtrip(para(ListKind::None, 0, s)).spacing();
            assert!((got - s).abs() < 0.01, "{s} 倍が {got} になった");
        }
    }

    #[test]
    fn 既定の段落には余計な指定を書かない() {
        let d = Document { font: None, blocks: vec![Block::Para(para(ListKind::None, 0, 1.0))] };
        let mut buf = Vec::new();
        crate::write(&d, std::io::Cursor::new(&mut buf)).unwrap();
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(&buf)).unwrap();
        let mut s = String::new();
        use std::io::Read;
        z.by_name("word/document.xml").unwrap().read_to_string(&mut s).unwrap();
        assert!(!s.contains("w:pPr"), "何も指定していないのに pPr を書いた");
    }

    #[test]
    fn 行間が0でも壊れない() {
        // 0 や負が入っても本文が消えない
        let p = para(ListKind::None, 0, 0.0);
        assert_eq!(p.spacing(), 1.0);
        let p = para(ListKind::None, 0, -3.0);
        assert_eq!(p.spacing(), 1.0);
    }

    #[test]
    fn 箇条書きの印が出る() {
        assert_eq!(para(ListKind::Bullet, 0, 1.0).marker(0).as_deref(), Some("・"));
        assert_eq!(para(ListKind::Number, 0, 1.0).marker(0).as_deref(), Some("1. "));
        assert_eq!(para(ListKind::Number, 0, 1.0).marker(4).as_deref(), Some("5. "));
        assert_eq!(para(ListKind::None, 0, 1.0).marker(0), None);
    }
}

#[cfg(test)]
mod break_round {
    use kumihan::{Block, Document, Paragraph, Run};

    #[test]
    fn 改ページ指定が往復する() {
        let mut para = Paragraph::default();
        para.page_break_before = true;
        para.runs.push(Run {
            text: "二頁目".into(), size_pt: 10.5, font: None, fmt: Default::default() });
        let d = Document { font: None, blocks: vec![Block::Para(para)] };
        let mut buf = Vec::new();
        crate::write(&d, std::io::Cursor::new(&mut buf)).unwrap();
        let back = crate::read(std::io::Cursor::new(&buf)).unwrap().0;
        assert!(back.paragraphs().next().unwrap().page_break_before, "改ページが消えた");
    }
}

#[cfg(test)]
mod gridcol_round {
    #[test]
    fn 列幅が往復する() {
        // 読んだ幅を捨てると、保存で表の形が変わる
        let xml = r#"<w:document xmlns:w="x"><w:body><w:tbl>
            <w:tblGrid><w:gridCol w:w="2268"/><w:gridCol w:w="4536"/></w:tblGrid>
            <w:tr><w:tc><w:p><w:r><w:t>a</w:t></w:r></w:p></w:tc>
                  <w:tc><w:p><w:r><w:t>b</w:t></w:r></w:p></w:tc></w:tr>
        </w:tbl></w:body></w:document>"#;
        let doc = crate::parse_document_xml(xml).0;
        let t = doc.tables().next().expect("表が無い");
        assert_eq!(t.col_mm.len(), 2, "gridCol を読めていない");
        // 2268 twip = 40mm
        assert!((t.col_mm[0] - 40.0).abs() < 0.1, "{}", t.col_mm[0]);
        assert!((t.col_mm[1] - 80.0).abs() < 0.1);

        let mut buf = Vec::new();
        crate::write(&doc, std::io::Cursor::new(&mut buf)).unwrap();
        let back = crate::read(std::io::Cursor::new(&buf)).unwrap().0;
        let bt = back.tables().next().unwrap();
        assert!((bt.col_mm[0] - 40.0).abs() < 0.2, "列幅が保存で変わった");
    }
}

#[cfg(test)]
mod preserve_tests {
    use kumihan::Document;
    use std::io::{Cursor, Read, Write};

    /// スタイルと画像もどきの部品を持つ docx を作る
    fn docx_with_parts() -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let o: zip::write::FileOptions<'_, ()> = Default::default();
        let mut put = |n: &str, d: &[u8]| {
            zip.start_file(n, o).unwrap();
            zip.write_all(d).unwrap();
        };
        put("[Content_Types].xml", br#"<Types xmlns="ct"><Default Extension="xml" ContentType="application/xml"/></Types>"#);
        put("_rels/.rels", br#"<Relationships xmlns="r"/>"#);
        put("word/document.xml",
            br#"<w:document xmlns:w="x"><w:body><w:p><w:r><w:t>Logo included</w:t></w:r></w:p></w:body></w:document>"#);
        put("word/styles.xml", br#"<w:styles xmlns:w="x"/>"#);
        put("word/media/logo.png", b"\x89PNG-fake-bytes");
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn 開いて保存しても部品が残る() {
        // 「保存したらロゴが消えた」を防ぐ
        let src = docx_with_parts();
        let (doc, _) = crate::read(Cursor::new(&src)).unwrap();
        let mut out = Vec::new();
        crate::write_with(&doc, Some(Cursor::new(&src)), Cursor::new(&mut out)).unwrap();
        let mut z = zip::ZipArchive::new(Cursor::new(&out)).unwrap();
        let names: Vec<String> = (0..z.len()).map(|i| z.by_index(i).unwrap().name().into()).collect();
        assert!(names.iter().any(|n| n == "word/media/logo.png"), "画像の実体が消えた: {names:?}");
        assert!(names.iter().any(|n| n == "word/styles.xml"), "スタイルが消えた: {names:?}");
        // 画像の中身も同じ
        let mut buf = Vec::new();
        z.by_name("word/media/logo.png").unwrap().read_to_end(&mut buf).unwrap();
        assert_eq!(buf, b"\x89PNG-fake-bytes");
        // 本文はこちらが書いたもの
        let mut s = String::new();
        z.by_name("word/document.xml").unwrap().read_to_string(&mut s).unwrap();
        assert!(s.contains("Logo included"), "本文が消えた");
    }

    #[test]
    fn 本文は二重に入らない() {
        let src = docx_with_parts();
        let (doc, _) = crate::read(Cursor::new(&src)).unwrap();
        let mut out = Vec::new();
        crate::write_with(&doc, Some(Cursor::new(&src)), Cursor::new(&mut out)).unwrap();
        let z = zip::ZipArchive::new(Cursor::new(&out)).unwrap();
        let n = (0..z.len())
            .filter(|i| {
                let mut z2 = zip::ZipArchive::new(Cursor::new(&out)).unwrap();
                z2.by_index(*i).map(|f| f.name() == "word/document.xml").unwrap_or(false)
            })
            .count();
        assert_eq!(n, 1, "document.xml が {n} 個ある");
    }

    #[test]
    fn 元が無ければ最小の形で書ける() {
        let doc = Document::plain("新規", 10.5);
        let mut out = Vec::new();
        crate::write_with(&doc, None::<Cursor<Vec<u8>>>, Cursor::new(&mut out)).unwrap();
        assert!(crate::read(Cursor::new(&out)).is_ok());
    }
}

#[cfg(test)]
mod anchor_tests {
    #[test]
    fn 画像の原文が保存で返る() {
        // 理解はしないが、捨てない
        let xml = r#"<w:document xmlns:w="x"><w:body><w:p>
            <w:r><w:t>図は</w:t></w:r>
            <w:r><w:drawing><wp:inline><a:blip r:embed="rId5"/></wp:inline></w:drawing></w:r>
            <w:r><w:t>のとおり</w:t></w:r>
        </w:p></w:body></w:document>"#;
        let (doc, rep) = crate::parse_document_xml(xml);
        assert!(rep.unsupported.iter().any(|(n, _)| n.contains("画像")), "報告が無い");
        let p = doc.paragraphs().next().unwrap();
        assert_eq!(p.anchors.len(), 1, "原文を控えていない");
        assert!(p.anchors[0].contains("rId5"), "{}", p.anchors[0]);

        let out = crate::write_document_xml(&doc);
        assert!(out.contains("r:embed=\"rId5\""), "保存で画像の参照が消えた");
        assert!(out.contains("のとおり"), "本文が消えた");
    }

    #[test]
    fn 図形の中の文字が本文に漏れない() {
        // a:t の「飾り文字」が本文へ混ざっていた(t を接頭辞に関係なく拾っていた)
        let xml = r#"<w:document xmlns:w="x"><w:body><w:p>
            <w:r><w:t>本文</w:t></w:r>
            <w:r><w:drawing><a:t>飾り文字</a:t></w:drawing></w:r>
        </w:p></w:body></w:document>"#;
        let (doc, _) = crate::parse_document_xml(xml);
        assert_eq!(doc.body_text(), "本文", "図形の中の文字が本文へ漏れた: {:?}", doc.body_text());
    }

    #[test]
    fn 一文字打っても画像は消えない() {
        let xml = r#"<w:document xmlns:w="x"><w:body><w:p>
            <w:r><w:t>図</w:t></w:r>
            <w:r><w:drawing><a:blip r:embed="rId7"/></w:drawing></w:r>
        </w:p></w:body></w:document>"#;
        let (mut doc, _) = crate::parse_document_xml(xml);
        doc.set_body_text("図を直した", 10.5);
        let out = crate::write_document_xml(&doc);
        assert!(out.contains("rId7"), "編集しただけで画像が消えた");
    }
}

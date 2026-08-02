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
                _ => {}
            },
            Ok(Event::Empty(e)) => match local(e.name().as_ref()) {
                b"c" => {
                    // 値の無いセル(書式だけ)。持たない
                    pos = None;
                }
                b"mergeCell" => merge(&e, &mut sh),
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
    if let Ok(mut f) = zip.by_name("xl/styles.xml") {
        let mut s = String::new();
        let _ = f.read_to_string(&mut s);
        styles = crate::styles::parse(&s);
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
    // シート名(workbook.xml の並び順)
    let mut names = Vec::new();
    if let Ok(mut f) = zip.by_name("xl/workbook.xml") {
        let mut s = String::new();
        let _ = f.read_to_string(&mut s);
        let mut r = Reader::from_str(&s);
        let mut buf = Vec::new();
        loop {
            match r.read_event_into(&mut buf) {
                Ok(Event::Eof) | Err(_) => break,
                Ok(Event::Start(e)) | Ok(Event::Empty(e))
                    if local(e.name().as_ref()) == b"sheet" =>
                {
                    names.push(attr(&e, "name").unwrap_or_else(|| "Sheet".into()));
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

    let mut book = Book { sheets: Vec::new() };
    for (i, path) in paths.iter().enumerate() {
        let mut s = String::new();
        if let Ok(mut f) = zip.by_name(path) { let _ = f.read_to_string(&mut s); }
        let name = names.get(i).cloned().unwrap_or_else(|| format!("Sheet{}", i + 1));
        book.sheets.push(parse_sheet(&s, &shared, &styles, &name, &mut rep));
        rep.sheets += 1;
    }
    if book.sheets.is_empty() {
        return Err("worksheet がありません(xlsxではない可能性)".into());
    }
    Ok((book, rep))
}

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

pub fn write<W: Write + Seek>(book: &Book, dst: W) -> Result<(), String> {
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

    let overrides: String = (1..=book.sheets.len())
        .map(|i| format!(r#"<Override PartName="/xl/worksheets/sheet{i}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#))
        .collect();
    let mut put = |name: &str, data: &str| -> Result<(), String> {
        zip.start_file(name, o).map_err(|e| e.to_string())?;
        zip.write_all(data.as_bytes()).map_err(|e| e.to_string())
    };
    put("[Content_Types].xml", &CT.replace("__SHEETS__", &overrides))?;
    put("_rels/.rels", RELS)?;

    let sheets_xml: String = book.sheets.iter().enumerate()
        .map(|(i, s)| format!(r#"<sheet name="{}" sheetId="{}" r:id="rId{}"/>"#,
                              esc(&s.name), i + 1, i + 1))
        .collect();
    put("xl/workbook.xml", &format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="{NS}" xmlns:r="{RNS}"><sheets>{sheets_xml}</sheets></workbook>"#))?;

    let wrels: String = (1..=book.sheets.len())
        .map(|i| format!(r#"<Relationship Id="rId{i}" Type="{RNS}/worksheet" Target="worksheets/sheet{i}.xml"/>"#))
        .collect();
    put("xl/_rels/workbook.xml.rels", &format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{wrels}<Relationship Id="rIdSS" Type="{RNS}/sharedStrings" Target="sharedStrings.xml"/><Relationship Id="rIdST" Type="{RNS}/styles" Target="styles.xml"/></Relationships>"#))?;

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
        w.write_event(Event::Start(ws)).unwrap();
        w.write_event(Event::Start(BytesStart::new("sheetData"))).unwrap();

        let mut rows: std::collections::BTreeMap<u32, Vec<(&Pos, &Cell)>> = Default::default();
        for (p, c) in &sh.cells { rows.entry(p.row).or_default().push((p, c)); }
        for (r, cells) in rows {
            let mut row = BytesStart::new("row");
            row.push_attribute(("r", (r + 1).to_string().as_str()));
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
        let body = String::from_utf8(w.into_inner().into_inner()).unwrap();
        put(&format!("xl/worksheets/sheet{}.xml", i + 1),
            &format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n{body}"))?;
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
        Book { sheets: vec![s] }
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
        let back = roundtrip(&Book { sheets: vec![sh] });
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
        let back = roundtrip(&Book { sheets: vec![s] });
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

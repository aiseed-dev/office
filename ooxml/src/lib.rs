//! docx ⇄ kumihan の文書モデル。
//!
//! 方針(SEKKEI.md「互換は書式の境界で守る」):
//!   - エンジンは継がない。**書式(docx)だけを読み書きする**
//!   - **全部は実装しない。読めないものは読めないと言う** —
//!     解釈できなかった要素は捨てずに `Report` に積んで返す。
//!     黙って落とすのが一番悪い(利用者は失われたことに気づけない)
//!
//! v0 の範囲: 本文の段落・ラン・文字サイズ・改行、そして**表**(w:tbl、
//! セル結合 gridSpan/vMerge を含む)。
//! 表は日本の事務様式の本体なので、v0 から入れる(実物8件すべてに表があった)。
//! 画像・ヘッダ/フッタ・スタイル定義は**未対応として報告する**。

pub mod crypt;

use std::io::{Cursor, Read, Seek, Write};

use kumihan::{Align, Block, Cellbox, CharFormat, Comment, Document, ListKind, ParaStyle,
              Paragraph, RefField, Run, Stroke, Table, VMerge, PAGES_MARK, PAGE_MARK,
              TRK_DEL_E, TRK_DEL_S, TRK_INS_E, TRK_INS_S};
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

    // 関係ID → 部品名 を先に引いておく。
    // rels: <Relationship Id="rId5" Target="media/image1.png"/>。
    // 画像の実体のほか、ヘッダー・フッターの部品名もここから引く
    let mut rels = String::new();
    if let Ok(mut f) = zip.by_name("word/_rels/document.xml.rels") {
        let _ = f.read_to_string(&mut rels);
    }
    let mut targets: std::collections::BTreeMap<String, String> = Default::default();
    let mut at = 0usize;
    while let Some(i) = rels[at..].find("Id=\"") {
        let s = at + i + 4;
        let Some(e) = rels[s..].find('"') else { break };
        let id = rels[s..s + e].to_string();
        // 同じ Relationship の中の Target を探す(次の > まで)
        let tail = &rels[s + e..];
        if let Some(ti) = tail.find("Target=\"") {
            let ts = ti + 8;
            if let Some(te) = tail[ts..].find('"') {
                targets.insert(id, tail[ts..ts + te].to_string());
            }
        }
        at = s + e;
    }
    let mut media: std::collections::BTreeMap<String, std::sync::Arc<Vec<u8>>> =
        Default::default();
    for (id, target) in &targets {
        if target.starts_with("media/") {
            let path = format!("word/{target}");
            if let Ok(mut mf) = zip.by_name(&path) {
                let mut buf = Vec::new();
                if mf.read_to_end(&mut buf).is_ok() {
                    media.insert(id.clone(), std::sync::Arc::new(buf));
                }
            }
        }
    }
    // 文書の情報(docProps/core.xml)。作成者・タイトルなど
    let mut pxml = String::new();
    if let Ok(mut f) = zip.by_name("docProps/core.xml") {
        let _ = f.read_to_string(&mut pxml);
    }

    // 設定(settings.xml)。欧文ハイフネーションの旗を読む
    let mut sxml = String::new();
    if let Ok(mut f) = zip.by_name("word/settings.xml") {
        let _ = f.read_to_string(&mut sxml);
    }
    // コメント(comments.xml)。id → 本文。本文の参照より先に読む
    let mut cxml = String::new();
    if let Ok(mut f) = zip.by_name("word/comments.xml") {
        let _ = f.read_to_string(&mut cxml);
    }
    let cmap = parse_comments(&cxml);
    let (mut doc, mut rep) = parse_document_full(&xml, &media, &cmap);
    // このアプリのペン(joink)は原文控えから筆へ読み戻す
    extract_ink(&mut doc);
    if !pxml.is_empty() {
        let field = |tag: &str| -> String {
            let open = format!("<{tag}");
            let close = format!("</{tag}>");
            let Some(i) = pxml.find(&open) else { return String::new() };
            let Some(g) = pxml[i..].find('>') else { return String::new() };
            if pxml[i..i + g].ends_with('/') {
                return String::new(); // <tag/> = 空
            }
            let s0 = i + g + 1;
            let Some(e) = pxml[s0..].find(&close) else { return String::new() };
            unesc(&pxml[s0..s0 + e])
        };
        doc.props = kumihan::CoreProps {
            creator: field("dc:creator"),
            title: field("dc:title"),
            keywords: field("cp:keywords"),
            subject: field("dc:subject"),
            description: field("dc:description"),
        };
    }
    if let Some(i) = sxml.find("<w:autoHyphenation") {
        let head = &sxml[i..(i + 60).min(sxml.len())];
        doc.hyphenate = !(head.contains("w:val=\"0\"") || head.contains("w:val=\"false\""));
    }
    // 縦書き(sectPr の textDirection=tbRl)
    if doc.sect_raw.as_deref().is_some_and(|t| {
        t.contains("textDirection") && t.contains("tbRl")
    }) {
        doc.vertical = true;
    }
    // 文書の保護(readOnly 等)。enforcement が切られていれば保護ではない
    if let Some(i) = sxml.find("<w:documentProtection") {
        let head = &sxml[i..(i + 200).min(sxml.len())];
        let off = head.contains("w:enforcement=\"0\"")
            || head.contains("w:enforcement=\"false\"");
        if !off {
            if let Some(j) = head.find("w:edit=\"") {
                let s2 = j + 8;
                if let Some(e) = head[s2..].find('"') {
                    doc.protection = Some(head[s2..s2 + e].to_string());
                }
            }
        }
    }
    // ヘッダー・フッター(全ページ同じもの = type="default")を部品から読む。
    // 表など、まだ持てないものが入っていたら**編集の対象にしない** —
    // paragraphs を空のまま残せば、保存で原文の部品がそのまま生きる
    if let Some(sect) = doc.sect_raw.clone() {
        for (tag, footer) in [("headerReference", false), ("footerReference", true)] {
            let Some(rid) = hf_ref(&sect, tag) else { continue };
            let Some(target) = targets.get(&rid) else { continue };
            let part = format!("word/{}", target.trim_start_matches('/').trim_start_matches("word/"));
            let mut hxml = String::new();
            match zip.by_name(&part) {
                Ok(mut f) => { let _ = f.read_to_string(&mut hxml); }
                Err(_) => continue,
            }
            let (hdoc, hrep) = parse_document_with(&hxml, &media);
            let which = if footer { "フッター" } else { "ヘッダー" };
            let hf = if footer { &mut doc.footer } else { &mut doc.header };
            hf.part = Some(part);
            if hdoc.tables().next().is_some() {
                rep.note(&format!("{which}の表(編集できないが保存では残る)"));
            } else {
                hf.paragraphs = hdoc.paragraphs().cloned().collect();
                if hf.paragraphs.is_empty() {
                    // 段落の無い部品でも、空の段落を1つ置けば編集できる
                    hf.paragraphs.push(Paragraph::default());
                }
            }
            for (n, k) in hrep.unsupported {
                for _ in 0..k {
                    rep.note(&format!("{which}: {n}"));
                }
            }
        }
        // 透かし(ヘッダーの中の VML)。原文控えからモデルへ引き上げる
        // (保存はモデルから作り直すので、控えは外す — 二重になるため)
        for p in &mut doc.header.paragraphs {
            let mut i = 0;
            while i < p.anchors.len() {
                let a = &p.anchors[i];
                if a.contains("v:textpath") {
                    if let Some(j) = a.find("string=\"") {
                        let s0 = j + 8;
                        if let Some(e) = a[s0..].find('"') {
                            let raw = &a[s0..s0 + e];
                            doc.watermark = Some(raw
                                .replace("&quot;", "\"")
                                .replace("&lt;", "<")
                                .replace("&gt;", ">")
                                .replace("&amp;", "&"));
                            p.anchors.remove(i);
                            continue;
                        }
                    }
                }
                i += 1;
            }
        }
    }
    // 文書の既定書体(styles.xml の docDefaults)。読まないと
    // 明朝の書類がこちらの既定(ゴシック)で表示される
    let mut styles = String::new();
    if let Ok(mut f) = zip.by_name("word/styles.xml") {
        let _ = f.read_to_string(&mut styles);
    }
    if let Some(i) = styles.find("docDefaults") {
        let head = &styles[i..(i + 600).min(styles.len())];
        for key in ["w:eastAsia=\"", "w:ascii=\""] {
            if let Some(j) = head.find(key) {
                let s = j + key.len();
                if let Some(e) = head[s..].find('"') {
                    doc.font = Some(head[s..s + e].to_string());
                    break;
                }
            }
        }
    }
    Ok((doc, rep))
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

/// 原文から表示用の画像を引く。EMU(914400/inch)→ mm は ÷36000。
fn image_of(
    raw: &str,
    media: &std::collections::BTreeMap<String, std::sync::Arc<Vec<u8>>>,
) -> Option<kumihan::InlineImage> {
    let grab = |pat: &str| -> Option<String> {
        let i = raw.find(pat)? + pat.len();
        let end = raw[i..].find('"')? + i;
        Some(raw[i..end].to_string())
    };
    let rid = grab("r:embed=\"")?;
    let bytes = media.get(&rid)?.clone();
    // wp:extent cx/cy(EMU)。無ければ表示しない(大きさを勝手に決めない)
    let cx: f32 = grab("cx=\"")?.parse().ok()?;
    let cy: f32 = grab("cy=\"")?.parse().ok()?;
    Some(kumihan::InlineImage { bytes, w_mm: cx / 36000.0, h_mm: cy / 36000.0 })
}

/// sectPr から用紙の寸法を読む(twip → mm)。
fn parse_sect(raw: &str) -> kumihan::PageSetup {
    let g = |el: &str, at: &str| -> Option<f32> {
        let i = raw.find(el)?;
        let head = &raw[i..(i + 200).min(raw.len())];
        let k = format!("{at}=\"");
        let s = head.find(&k)? + k.len();
        let e = head[s..].find('"')? + s;
        head[s..e].parse::<f32>().ok().map(twip_mm)
    };
    let d = kumihan::PageSetup::default();
    // 段組み(w:cols w:num)。twip ではなく数なので g は使わない
    let cols = raw
        .find("<w:cols")
        .and_then(|i| {
            let head = &raw[i..(i + 200).min(raw.len())];
            let k = "w:num=\"";
            let s = head.find(k)? + k.len();
            let e = head[s..].find('"')? + s;
            head[s..e].parse::<u8>().ok()
        })
        .unwrap_or(1);
    kumihan::PageSetup {
        w_mm: g("<w:pgSz", "w:w").unwrap_or(d.w_mm),
        h_mm: g("<w:pgSz", "w:h").unwrap_or(d.h_mm),
        left_mm: g("<w:pgMar", "w:left").unwrap_or(d.left_mm),
        right_mm: g("<w:pgMar", "w:right").unwrap_or(d.right_mm),
        top_mm: g("<w:pgMar", "w:top").unwrap_or(d.top_mm),
        bottom_mm: g("<w:pgMar", "w:bottom").unwrap_or(d.bottom_mm),
        columns: cols.clamp(1, 8),
    }
}

/// sectPr の中から、全ページ同じヘッダー(フッター)の参照 r:id を引く。
/// `<w:headerReference w:type="default" r:id="rId8"/>`。type 無しは default 扱い。
fn hf_ref(sect: &str, tag: &str) -> Option<String> {
    let needle = format!("<w:{tag}");
    let mut at = 0usize;
    while let Some(i) = sect[at..].find(&needle) {
        let s = at + i;
        let e = sect[s..].find('>')? + s;
        let head = &sect[s..e];
        if !head.contains("w:type=") || head.contains("w:type=\"default\"") {
            if let Some(j) = head.find("r:id=\"") {
                let js = j + 6;
                if let Some(je) = head[js..].find('"') {
                    return Some(head[js..js + je].to_string());
                }
            }
        }
        at = e;
    }
    None
}

/// フィールドの命令を印(1字)へ。PAGE(いまのページの番号)と
/// NUMPAGES(総頁)だけを持つ。それ以外は None(報告して落とす)。
fn field_mark(instr: &str) -> Option<char> {
    match instr.split_whitespace().next() {
        Some("PAGE") => Some(PAGE_MARK),
        Some("NUMPAGES") => Some(PAGES_MARK),
        _ => None,
    }
}

/// 相互参照の命令(REF / PAGEREF しおり名)。
fn ref_instr(instr: &str) -> Option<RefField> {
    let mut it = instr.split_whitespace();
    let kind = it.next()?;
    let name = it.next()?.to_string();
    match kind {
        "REF" => Some(RefField { name, page: false }),
        "PAGEREF" => Some(RefField { name, page: true }),
        _ => None,
    }
}

/// 部分木の原文から、w:t の中の文字を繋いで返す(フィールドの見えている値)。
fn inner_texts(raw: &str) -> String {
    let mut out = String::new();
    let mut at = 0usize;
    while let Some(i) = raw[at..].find("<w:t") {
        let s = at + i;
        let Some(gt) = raw[s..].find('>') else { break };
        // <w:tab/> 等に引っ掛からない(<w:t か <w:t␣ だけ)
        let head = &raw[s..s + gt];
        if !(head == "<w:t" || head.starts_with("<w:t ")) {
            at = s + 4;
            continue;
        }
        let ts = s + gt + 1;
        let Some(te) = raw[ts..].find("</w:t>") else { break };
        out.push_str(
            &raw[ts..ts + te]
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&quot;", "\"")
                .replace("&amp;", "&"),
        );
        at = ts + te;
    }
    out
}

/// comments.xml を読む。id → コメント(書いた人・本文)。
fn parse_comments(xml: &str) -> std::collections::BTreeMap<String, Comment> {
    let mut out: std::collections::BTreeMap<String, Comment> = Default::default();
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut cur: Option<(String, Comment)> = None;
    let mut in_t = false;
    loop {
        match r.read_event_into(&mut buf) {
            Err(_) | Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => match local(e.name().as_ref()) {
                b"comment" => {
                    let id = attr(&e, "id").unwrap_or_default();
                    let author = attr(&e, "author").unwrap_or_default();
                    cur = Some((id, Comment { author, text: String::new() }));
                }
                b"p" => {
                    if let Some((_, c)) = cur.as_mut() {
                        if !c.text.is_empty() {
                            c.text.push('\n');
                        }
                    }
                }
                b"t" => in_t = true,
                _ => {}
            },
            Ok(Event::Text(t)) if in_t => {
                if let Some((_, c)) = cur.as_mut() {
                    c.text.push_str(&t.unescape().unwrap_or_default());
                }
            }
            Ok(Event::End(e)) => match local(e.name().as_ref()) {
                b"t" => in_t = false,
                b"comment" => {
                    if let Some((id, c)) = cur.take() {
                        out.insert(id, c);
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

/// XML の属性・本文に入れる文字を逃がす。
/// esc の逆(core.xml など小さな部品の中身を戻す)
fn unesc(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// comments.xml を作る。id は 1 から(document.xml 側の参照と同じ振り方)。
fn comments_xml(cmts: &[Comment]) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
    out.push_str(&format!("<w:comments xmlns:w=\"{W_NS}\">"));
    for (i, c) in cmts.iter().enumerate() {
        out.push_str(&format!(
            r#"<w:comment w:id="{}" w:author="{}">"#, i + 1, esc(&c.author)));
        for line in c.text.split('\n') {
            out.push_str("<w:p><w:r><w:t xml:space=\"preserve\">");
            out.push_str(&esc(line));
            out.push_str("</w:t></w:r></w:p>");
        }
        out.push_str("</w:comment>");
    }
    out.push_str("</w:comments>");
    out
}

/// 透かしの図形(VML)。Word が透かしに使う形(WordArt t136)に合わせる。
/// shapetype(字形の定義)ごと1つの w:pict に入れる。
fn watermark_vml(text: &str) -> String {
    format!(
        concat!(
            r#"<w:pict><v:shapetype id="_x0000_t136" coordsize="21600,21600" o:spt="136" adj="10800" path="m@7,l@8,m@5,21600l@6,21600e">"#,
            r#"<v:formulas><v:f eqn="sum #0 0 10800"/><v:f eqn="prod #0 2 1"/><v:f eqn="sum 21600 0 @1"/><v:f eqn="sum 0 0 @2"/><v:f eqn="sum 21600 0 @3"/><v:f eqn="if @0 @3 0"/><v:f eqn="if @0 21600 @1"/><v:f eqn="if @0 0 @2"/><v:f eqn="if @0 @4 21600"/><v:f eqn="mid @5 @6"/><v:f eqn="mid @8 @5"/><v:f eqn="mid @7 @8"/><v:f eqn="mid @6 @7"/><v:f eqn="sum @6 0 @5"/></v:formulas>"#,
            r#"<v:path textpathok="t" o:connecttype="custom" o:connectlocs="@9,0;@10,10800;@11,21600;@12,10800" o:connectangles="270,180,90,0"/>"#,
            r#"<v:textpath on="t" fitshape="t"/>"#,
            r##"<v:handles><v:h position="#0,bottommost" xrange="6629,14971"/></v:handles>"##,
            r#"<o:lock v:ext="edit" text="t" shapetype="t"/></v:shapetype>"#,
            r##"<v:shape id="jowatermark" type="#_x0000_t136" style="position:absolute;margin-left:0;margin-top:0;width:460pt;height:230pt;rotation:315;z-index:-251654144;mso-position-horizontal:center;mso-position-horizontal-relative:margin;mso-position-vertical:center;mso-position-vertical-relative:margin" o:allowincell="f" fillcolor="#d8d8d8" stroked="f">"##,
            r#"<v:fill opacity=".5"/><v:textpath style="font-family:&quot;游明朝&quot;" string="{}"/></v:shape></w:pict>"#
        ),
        esc(text)
    )
}

/// mm → EMU(914400/インチ)
fn emu(mm: f32) -> i64 {
    (mm * 36000.0).round() as i64
}

/// 手描きの1筆を、ページ固定の自由曲線(DrawingML)にする。
/// 名前 `joink…p{page}` が読み戻しの鍵(ページ番号も名前で持つ —
/// XML の座標はページの中の位置しか持てないため)。
/// アプリ(writer)が、そのページにある段落の anchors に差し込む。
pub fn ink_anchor_xml(st: &Stroke, id: usize) -> String {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (x, y) in &st.points {
        x0 = x0.min(*x);
        y0 = y0.min(*y);
        x1 = x1.max(*x);
        y1 = y1.max(*y);
    }
    let (w, h) = ((x1 - x0).max(0.5), (y1 - y0).max(0.5));
    let mut path = String::new();
    for (i, (x, y)) in st.points.iter().enumerate() {
        let tag = if i == 0 { "moveTo" } else { "lnTo" };
        path.push_str(&format!(
            r#"<a:{tag}><a:pt x="{}" y="{}"/></a:{tag}>"#,
            emu(x - x0),
            emu(y - y0)
        ));
    }
    // 蛍光ペンは太く・薄く・文字の下(behindDoc)。ペンは細く・濃く・上
    let (line_w, color, alpha, behind) = if st.highlighter {
        (emu(3.0), "FFE45C", r#"<a:alpha val="45000"/>"#, "1")
    } else {
        (emu(0.45), "1B3A52", "", "0")
    };
    format!(
        concat!(
            r#"<w:drawing><wp:anchor distT="0" distB="0" distL="0" distR="0" simplePos="0" "#,
            r#"relativeHeight="251658240" behindDoc="{behind}" locked="0" layoutInCell="1" allowOverlap="1">"#,
            r#"<wp:simplePos x="0" y="0"/>"#,
            r#"<wp:positionH relativeFrom="page"><wp:posOffset>{px}</wp:posOffset></wp:positionH>"#,
            r#"<wp:positionV relativeFrom="page"><wp:posOffset>{py}</wp:posOffset></wp:positionV>"#,
            r#"<wp:extent cx="{cx}" cy="{cy}"/><wp:wrapNone/>"#,
            r#"<wp:docPr id="{id}" name="joink{id}p{page}"/>"#,
            r#"<a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape">"#,
            r#"<wps:wsp><wps:cNvSpPr/><wps:spPr>"#,
            r#"<a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm>"#,
            r#"<a:custGeom><a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/>"#,
            r#"<a:rect l="0" t="0" r="{cx}" b="{cy}"/>"#,
            r#"<a:pathLst><a:path w="{cx}" h="{cy}">{path}</a:path></a:pathLst></a:custGeom>"#,
            r#"<a:noFill/>"#,
            r#"<a:ln w="{lw}" cap="rnd"><a:solidFill><a:srgbClr val="{color}">{alpha}</a:srgbClr></a:solidFill><a:round/></a:ln>"#,
            r#"</wps:spPr><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing>"#
        ),
        behind = behind,
        px = emu(x0),
        py = emu(y0),
        cx = emu(w),
        cy = emu(h),
        id = id,
        page = st.page,
        path = path,
        lw = line_w,
        color = color,
        alpha = alpha,
    )
}

/// [`ink_anchor_xml`] を、段落の控え(anchors)にそのまま置ける形
/// (名前空間の宣言つきの `<w:r>`)で返す。writer はこれを差し込む。
pub fn ink_anchor_run(st: &Stroke, id: usize) -> String {
    let inner = ink_anchor_xml(st, id);
    wrap_with_ns(&inner, &Default::default()).unwrap_or(inner)
}

/// 原文控え(anchors)の中の joink(手描きの線)を筆に読み戻す。
/// 読めたら控えから外す(保存はモデルから作り直す)。
fn extract_ink(doc: &mut Document) {
    let grab = |s: &str, pat: &str| -> Option<f32> {
        let i = s.find(pat)? + pat.len();
        let e = s[i..].find('<')? + i;
        s[i..e].parse::<f32>().ok()
    };
    let mut ink: Vec<Stroke> = Vec::new();
    for b in &mut doc.blocks {
        let Block::Para(p) = b else { continue };
        let mut i = 0;
        while i < p.anchors.len() {
            let a = &p.anchors[i];
            let Some(ni) = a.find("name=\"joink") else {
                i += 1;
                continue;
            };
            // 名前からページ番号(joink{id}p{page})
            let page = a[ni..]
                .find('p')
                .and_then(|pi| {
                    let s2 = &a[ni + pi + 1..];
                    let e = s2.find('"')?;
                    s2[..e].parse::<usize>().ok()
                })
                .unwrap_or(0);
            let x0 = grab(a, "<wp:positionH relativeFrom=\"page\"><wp:posOffset>")
                .map(|v| v / 36000.0);
            let y0 = grab(a, "<wp:positionV relativeFrom=\"page\"><wp:posOffset>")
                .map(|v| v / 36000.0);
            let (Some(x0), Some(y0)) = (x0, y0) else {
                i += 1;
                continue;
            };
            // a:pt を順に拾う
            let mut points: Vec<(f32, f32)> = Vec::new();
            let mut at = 0usize;
            while let Some(j) = a[at..].find("<a:pt x=\"") {
                let s2 = at + j + 9;
                let Some(xe) = a[s2..].find('"') else { break };
                let Some(yj) = a[s2 + xe..].find("y=\"") else { break };
                let ys = s2 + xe + yj + 3;
                let Some(ye) = a[ys..].find('"') else { break };
                if let (Ok(x), Ok(y)) =
                    (a[s2..s2 + xe].parse::<f32>(), a[ys..ys + ye].parse::<f32>())
                {
                    points.push((x0 + x / 36000.0, y0 + y / 36000.0));
                }
                at = ys + ye;
            }
            if points.is_empty() {
                i += 1;
                continue;
            }
            let highlighter = a.contains("<a:alpha");
            ink.push(Stroke { page, highlighter, points });
            p.anchors.remove(i);
        }
    }
    doc.ink.extend(ink);
}

/// `w:pStyle` の val を段落の役割へ。見出しと目次の行だけを見る
/// (それ以外のスタイルは今まで通り持たない)。
/// 見出しの style id は日本語版 Word が「1」、英語版が「Heading1」。
fn style_of(val: &str) -> ParaStyle {
    let v = val.to_ascii_lowercase().replace(' ', "");
    match v.as_str() {
        "1" | "heading1" | "見出し1" => ParaStyle::Heading(1),
        "2" | "heading2" | "見出し2" => ParaStyle::Heading(2),
        "3" | "heading3" | "見出し3" => ParaStyle::Heading(3),
        "tableoffigures" => ParaStyle::Tof,
        "toc1" => ParaStyle::Toc(1),
        "toc2" => ParaStyle::Toc(2),
        "toc3" => ParaStyle::Toc(3),
        _ => ParaStyle::Body,
    }
}

/// w:fldChar(複雑なフィールドの区切り)を1つ処理する。
/// begin〜end の間に instrText で命令が来る。separate〜end は
/// 「計算済みの見た目」なので持たない(開く側が計算し直す)。
#[allow(clippy::too_many_arguments)]
fn fldchar(
    kind: Option<&str>,
    in_field: &mut bool,
    field_hide: &mut bool,
    field_instr: &mut String,
    field_buf: &mut String,
    para: &mut Option<Vec<Run>>,
    rep: &mut Report,
    size_pt: f32,
    font: &Option<String>,
    fmt: &CharFormat,
) {
    match kind {
        Some("begin") => {
            *in_field = true;
            *field_hide = false;
            field_instr.clear();
            field_buf.clear();
        }
        Some("separate") => {
            if *in_field {
                *field_hide = true;
            }
        }
        Some("end") => {
            if *in_field {
                *in_field = false;
                *field_hide = false;
                if let Some(mark) = field_mark(field_instr) {
                    if let Some(p) = para.as_mut() {
                        p.push(Run {
                            text: mark.to_string(),
                            size_pt,
                            font: font.clone(),
                            fmt: fmt.clone(),
                        });
                    }
                } else if let Some(rf) = ref_instr(field_instr) {
                    // 相互参照。separate〜end の見えている値ごと run にする
                    if let Some(p) = para.as_mut() {
                        let mut f2 = fmt.clone();
                        f2.field = Some(rf);
                        p.push(Run {
                            text: std::mem::take(field_buf),
                            size_pt,
                            font: font.clone(),
                            fmt: f2,
                        });
                    }
                } else if !field_instr.trim().is_empty() {
                    rep.note(&format!("フィールド({})", field_instr.trim()));
                }
            }
        }
        _ => {}
    }
}

pub fn parse_document_xml(xml: &str) -> (Document, Report) {
    parse_document_with(xml, &Default::default())
}

/// media は 関係ID(rId5)→ 画像の実体。`read` が rels と media から作る。
pub fn parse_document_with(
    xml: &str,
    media: &std::collections::BTreeMap<String, std::sync::Arc<Vec<u8>>>,
) -> (Document, Report) {
    parse_document_full(xml, media, &Default::default())
}

/// cmts は comments.xml の中身(id → コメント)。参照をここで解決する。
fn parse_document_full(
    xml: &str,
    media: &std::collections::BTreeMap<String, std::sync::Arc<Vec<u8>>>,
    cmts: &std::collections::BTreeMap<String, Comment>,
) -> (Document, Report) {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(false);

    let mut doc = Document::default();
    let mut rep = Report::default();
    let mut stack: Vec<TblBuild> = Vec::new();

    let mut para: Option<Vec<Run>> = None;
    // いま読んでいるセルの結合(w:tcPr の gridSpan / vMerge)
    let mut cell_span = 0u8;
    let mut cell_vmerge = VMerge::None;
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
    // 段落の背景色(w:shd)と囲み枠(w:pBdr)
    let mut shade: Option<String> = None;
    let mut boxed = false;
    // 段落の役割(w:pStyle / w:outlineLvl)
    let mut pstyle = ParaStyle::Body;
    // リストの深さ(w:ilvl)。w:ind が無い文書ではこれがインデントになる
    let mut ilvl = 0u8;
    // この段落に付いたコメント(commentReference を解決したもの)
    let mut para_comments: Vec<Comment> = Vec::new();
    // この段落に付いたしおり(bookmarkStart の名前)
    let mut para_bookmarks: Vec<String> = Vec::new();
    // ドロップキャップ。Word は「枠の段落(頭の1字)+本文の段落」に割るので、
    // 枠の段落を控えて次の段落の頭に合流させる
    let mut dropcap = false;
    let mut pending_cap: Option<Vec<Run>> = None;
    // 読めなかった要素の原文(画像など)。段落ごとに集めて持ち越す
    let mut anchors: Vec<String> = Vec::new();
    // 表示できる画像(r:embed と wp:extent が読めたもの)
    let mut images: Vec<kumihan::InlineImage> = Vec::new();
    // 原本の root が宣言している名前空間。持ち越す原文の接頭辞をこれで包む
    let mut ns_decls: std::collections::BTreeMap<String, String> = Default::default();
    let mut in_ppr = false;
    let mut in_text = false;
    let mut in_rpr = false;
    let mut cur = String::new();
    // フィールド(w:fldChar / w:instrText)。PAGE は印、REF は参照の run になる
    let mut in_instr = false;
    let mut in_field = false;
    let mut field_hide = false;
    let mut field_instr = String::new();
    let mut field_buf = String::new();
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
            Ok(Event::Eof) => {
                // 枠の段落だけで終わった文書。段落として残す(黙って捨てない)
                if let Some(cap) = pending_cap.take() {
                    doc.push_para(Paragraph {
                        dropcap: true,
                        line_spacing: 1.0,
                        runs: cap,
                        ..Default::default()
                    });
                }
                break;
            }
            Ok(Event::Start(e)) => {
                let n = local(e.name().as_ref()).to_vec();
                match n.as_slice() {
                    // hdr / ftr はヘッダー・フッターの部品を読むときの root
                    b"document" | b"hdr" | b"ftr" => {
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
                    b"tc" => if let Some(b) = stack.last_mut() {
                        b.cell.clear();
                        cell_span = 0;
                        cell_vmerge = VMerge::None;
                    },
                    b"p" => { para = Some(Vec::new()); size_pt = DEFAULT_PT; font = None;
                              fmt = CharFormat::default(); align = Align::default();
                              list = ListKind::default(); indent = 0; line_spacing = 1.0;
                              page_break_before = false; shade = None; boxed = false;
                              pstyle = ParaStyle::Body; ilvl = 0;
                              para_comments.clear(); para_bookmarks.clear();
                              dropcap = false; }
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
                    b"vertAlign" if in_rpr => {
                        match attr(&e, "val").as_deref() {
                            Some("superscript") => fmt.superscript = true,
                            Some("subscript") => fmt.subscript = true,
                            _ => {}
                        }
                    }
                    b"highlight" if in_rpr => {
                        fmt.highlight = attr(&e, "val").filter(|v| v != "none");
                    }

                    // 箇条書きは numId で決まる。1 を中黒、2 を段落番号として扱う
                    // (numbering.xml を持たないので、往復できる最小の約束にしてある)
                    b"ilvl" if in_ppr => {
                        ilvl = attr(&e, "val").and_then(|v| v.parse().ok()).unwrap_or(0).min(8);
                    }
                    b"numId" if in_ppr => {
                        list = match attr(&e, "val").as_deref() {
                            Some("2") => ListKind::Number,
                            Some("0") | None => ListKind::None,
                            _ => ListKind::Bullet,
                        };
                    }
                    // 段落のスタイル。見出しと目次の行だけを持つ
                    b"pStyle" if in_ppr => {
                        if let Some(v) = attr(&e, "val") { pstyle = style_of(&v); }
                    }
                    // スタイル名で見出しと分からなくても、outlineLvl があれば見出し
                    b"outlineLvl" if in_ppr => {
                        if pstyle == ParaStyle::Body {
                            if let Some(n) = attr(&e, "val").and_then(|v| v.parse::<u8>().ok()) {
                                if n < 3 { pstyle = ParaStyle::Heading(n + 1); }
                            }
                        }
                    }
                    b"framePr" if in_ppr => {
                        dropcap = matches!(attr(&e, "dropCap").as_deref(),
                            Some("drop") | Some("margin"));
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
                    // 段落の背景色。fill が色(auto 以外)のときだけ
                    b"shd" if in_ppr => {
                        shade = attr(&e, "fill")
                            .filter(|v| !v.is_empty() && v != "auto");
                    }
                    // 段落の囲み枠。辺の別は持たない(あれば囲みとみなす)
                    b"pBdr" if in_ppr => boxed = true,
                    b"t" => { in_text = true; cur.clear(); }
                    // セル結合。横は列数、縦は restart/continue の区別で持つ
                    b"gridSpan" => if stack.last().is_some() {
                        cell_span = attr(&e, "val").and_then(|v| v.parse().ok()).unwrap_or(0);
                    },
                    b"vMerge" => if stack.last().is_some() {
                        cell_vmerge = match attr(&e, "val").as_deref() {
                            Some("restart") => VMerge::Start,
                            // val 無しは「続き」(docx の既定)
                            _ => VMerge::Continue,
                        };
                    },
                    b"sectPr" => {
                        // 節の設定。用紙・余白のほか、ヘッダーの参照も入っている。
                        // **理解はしないが捨てない**(捨てると保存で用紙設定と
                        // ヘッダーが消える)。寸法だけは読んで組版に使う
                        let name = e.name().to_owned();
                        if r.read_to_end_into(name, &mut Vec::new()).is_ok() {
                            let end = r.buffer_position() as usize;
                            let raw = &xml[start_pos..end];
                            doc.page = Some(parse_sect(raw));
                            doc.sect_raw = Some(raw.to_string());
                            last_pos = end;
                        }
                    }
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
                            // 表示用: 画像の実体と大きさが分かれば拾う
                            if let Some(im) = image_of(raw, media) {
                                images.push(im);
                            }
                            match wrap_with_ns(raw, &ns_decls) {
                                Some(wrapped) => {
                                    anchors.push(wrapped);
                                    rep.note("画像・図形(保存では残る)");
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
                    // 既にある変更履歴。挿入(w:ins)の中の字は本文として読み、
                    // 削除(w:del)の中(w:delText)は読まない = 確定後の姿。
                    // **保存すると履歴そのものは消える**ので、そう言う
                    b"ins" | b"del" => {
                        rep.note("変更履歴(表示は確定後の姿。保存で履歴は確定される)");
                    }
                    // ページの色(w:background。root 直下)
                    b"background" => {
                        doc.page_color = attr(&e, "color")
                            .filter(|v| !v.is_empty() && v != "auto");
                    }
                    // 単純なフィールド。PAGE は印として持ち、それ以外は報告する
                    b"fldSimple" => {
                        let instr = attr(&e, "instr").unwrap_or_default();
                        let name = e.name().to_owned();
                        if r.read_to_end_into(name, &mut Vec::new()).is_ok() {
                            last_pos = r.buffer_position() as usize;
                        }
                        if let Some(mark) = field_mark(&instr) {
                            if let Some(p) = para.as_mut() {
                                p.push(Run { text: mark.to_string(), size_pt,
                                             font: font.clone(), fmt: fmt.clone() });
                            }
                        } else if let Some(rf) = ref_instr(&instr) {
                            // 相互参照。見えている値ごと run にする
                            let raw = &xml[start_pos..last_pos];
                            if let Some(p) = para.as_mut() {
                                let mut f2 = fmt.clone();
                                f2.field = Some(rf);
                                p.push(Run { text: inner_texts(raw), size_pt,
                                             font: font.clone(), fmt: f2 });
                            }
                        } else {
                            rep.note(&format!("フィールド({})", instr.trim()));
                        }
                    }
                    // ルビ(w:ruby)。読みは w:rt、基底は w:rubyBase から。
                    // 基底の中の書式は落ちる(基底はふつう一様なので許す)
                    b"ruby" => {
                        let name = e.name().to_owned();
                        if r.read_to_end_into(name, &mut Vec::new()).is_ok() {
                            let end = r.buffer_position() as usize;
                            let raw = &xml[start_pos..end];
                            last_pos = end;
                            let slice = |tag: &str| -> &str {
                                let open = format!("<w:{tag}");
                                let close = format!("</w:{tag}>");
                                match (raw.find(&open), raw.find(&close)) {
                                    (Some(a), Some(b)) if b > a => &raw[a..b],
                                    _ => "",
                                }
                            };
                            let rt = inner_texts(slice("rt"));
                            let base_raw = slice("rubyBase").to_string();
                            let base = inner_texts(&base_raw);
                            // 基底の大きさは rubyBase の中の w:sz から
                            let pt = base_raw
                                .find("<w:sz ")
                                .and_then(|i| {
                                    base_raw[i..].find("w:val=\"").map(|j| i + j + 7)
                                })
                                .and_then(|s0| {
                                    base_raw[s0..].find('"').and_then(|e2| {
                                        base_raw[s0..s0 + e2].parse::<f32>().ok()
                                    })
                                })
                                .map(|h| h / 2.0)
                                .unwrap_or(size_pt);
                            if !base.is_empty() {
                                if let Some(p) = para.as_mut() {
                                    let mut f2 = fmt.clone();
                                    f2.ruby = (!rt.is_empty()).then_some(rt);
                                    p.push(Run {
                                        text: base,
                                        size_pt: pt,
                                        font: font.clone(),
                                        fmt: f2,
                                    });
                                }
                            }
                        }
                    }
                    b"instrText" => in_instr = true,
                    b"fldChar" => fldchar(attr(&e, "fldCharType").as_deref(),
                        &mut in_field, &mut field_hide, &mut field_instr, &mut field_buf,
                        &mut para, &mut rep, size_pt, &font, &fmt),
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
                    b"vertAlign" if in_rpr => {
                        match attr(&e, "val").as_deref() {
                            Some("superscript") => fmt.superscript = true,
                            Some("subscript") => fmt.subscript = true,
                            _ => {}
                        }
                    }
                    b"highlight" if in_rpr => {
                        fmt.highlight = attr(&e, "val").filter(|v| v != "none");
                    }

                    // 箇条書きは numId で決まる。1 を中黒、2 を段落番号として扱う
                    // (numbering.xml を持たないので、往復できる最小の約束にしてある)
                    b"ilvl" if in_ppr => {
                        ilvl = attr(&e, "val").and_then(|v| v.parse().ok()).unwrap_or(0).min(8);
                    }
                    b"numId" if in_ppr => {
                        list = match attr(&e, "val").as_deref() {
                            Some("2") => ListKind::Number,
                            Some("0") | None => ListKind::None,
                            _ => ListKind::Bullet,
                        };
                    }
                    // 段落のスタイル。見出しと目次の行だけを持つ
                    b"pStyle" if in_ppr => {
                        if let Some(v) = attr(&e, "val") { pstyle = style_of(&v); }
                    }
                    // スタイル名で見出しと分からなくても、outlineLvl があれば見出し
                    b"outlineLvl" if in_ppr => {
                        if pstyle == ParaStyle::Body {
                            if let Some(n) = attr(&e, "val").and_then(|v| v.parse::<u8>().ok()) {
                                if n < 3 { pstyle = ParaStyle::Heading(n + 1); }
                            }
                        }
                    }
                    b"framePr" if in_ppr => {
                        dropcap = matches!(attr(&e, "dropCap").as_deref(),
                            Some("drop") | Some("margin"));
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
                    // 段落の背景色。fill が色(auto 以外)のときだけ
                    b"shd" if in_ppr => {
                        shade = attr(&e, "fill")
                            .filter(|v| !v.is_empty() && v != "auto");
                    }
                    // 段落の囲み枠。辺の別は持たない(あれば囲みとみなす)
                    b"pBdr" if in_ppr => boxed = true,
                    // セル結合(空要素で来るのが普通の形)
                    b"gridSpan" => if stack.last().is_some() {
                        cell_span = attr(&e, "val").and_then(|v| v.parse().ok()).unwrap_or(0);
                    },
                    b"vMerge" => if stack.last().is_some() {
                        cell_vmerge = match attr(&e, "val").as_deref() {
                            Some("restart") => VMerge::Start,
                            _ => VMerge::Continue,
                        };
                    },
                    b"drawing" | b"pict" | b"object" =>
                        rep.note(&format!("w:{}", String::from_utf8_lossy(&n))),
                    // ページの色(空要素で来るのが普通の形)
                    b"background" => {
                        doc.page_color = attr(&e, "color")
                            .filter(|v| !v.is_empty() && v != "auto");
                    }
                    // しおり(bookmark)。名前を段落に持ち、保存で振り直す
                    // (範囲は段落単位 — コメントと同じ粒度)。
                    // 相互参照(REF)が指す名前はそのまま残る
                    b"bookmarkStart" => {
                        match (para.is_some(), attr(&e, "name")) {
                            (true, Some(name)) if !name.is_empty() => {
                                para_bookmarks.push(name);
                            }
                            (false, Some(_)) => {
                                rep.note("しおり(段落の外。保存で失われる)");
                            }
                            _ => {}
                        }
                    }
                    b"bookmarkEnd" => {} // 始まり側で持つ。終わりは保存で振り直す
                    // コメント。id が comments.xml で引けたら**段落のコメント**として
                    // 持つ(保存で作り直す)。引けなければ原文を控えて生かす
                    b"commentRangeStart" | b"commentRangeEnd" => {
                        let known = attr(&e, "id").is_some_and(|i| cmts.contains_key(&i));
                        if !known {
                            if para.is_some() {
                                let raw = &xml[start_pos..last_pos];
                                anchors.push(raw.trim().to_string());
                                rep.note("しおり・コメントの印(段落の頭に寄るが、保存で残る)");
                            }
                        }
                    }
                    b"commentReference" => {
                        match attr(&e, "id").and_then(|i| cmts.get(&i)) {
                            Some(c) => para_comments.push(c.clone()),
                            None => {
                                if para.is_some() {
                                    let raw = &xml[start_pos..last_pos];
                                    anchors.push(format!("<w:r>{}</w:r>", raw.trim()));
                                    rep.note("しおり・コメントの印(段落の頭に寄るが、保存で残る)");
                                }
                            }
                        }
                    }
                    // fldChar は空要素で来るのが普通の形
                    b"fldChar" => fldchar(attr(&e, "fldCharType").as_deref(),
                        &mut in_field, &mut field_hide, &mut field_instr, &mut field_buf,
                        &mut para, &mut rep, size_pt, &font, &fmt),
                    b"fldSimple" => {
                        // 空の fldSimple(中身なし)。持てる命令なら印だけ置く
                        let instr = attr(&e, "instr").unwrap_or_default();
                        if let Some(mark) = field_mark(&instr) {
                            if let Some(p) = para.as_mut() {
                                p.push(Run { text: mark.to_string(), size_pt,
                                             font: font.clone(), fmt: fmt.clone() });
                            }
                        } else {
                            rep.note(&format!("フィールド({})", instr.trim()));
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                if in_text {
                    cur.push_str(&t.unescape().unwrap_or_default());
                } else if in_instr {
                    field_instr.push_str(&t.unescape().unwrap_or_default());
                }
            }
            Ok(Event::End(e)) => {
                let n = local(e.name().as_ref()).to_vec();
                match n.as_slice() {
                    b"t" => {
                        in_text = false;
                        if field_hide {
                            // フィールドの計算済みの見た目。REF はここが「見えている値」
                            // になるので控える(PAGE 等では捨てられる)
                            field_buf.push_str(&std::mem::take(&mut cur));
                        } else if !cur.is_empty() {
                            if let Some(p) = para.as_mut() {
                                p.push(Run { text: std::mem::take(&mut cur), size_pt, font: font.clone(), fmt: fmt.clone() });
                            }
                        }
                    }
                    b"instrText" => in_instr = false,
                    b"rPr" => in_rpr = false,
                    b"pPr" => in_ppr = false,
                    b"p" => {
                        if let Some(runs) = para.take() {
                            rep.runs += runs.len();
                            rep.paragraphs += 1;
                            let mut p = Paragraph { align, anchors: std::mem::take(&mut anchors),
                                images: std::mem::take(&mut images),
                                comments: std::mem::take(&mut para_comments),
                                bookmarks: std::mem::take(&mut para_bookmarks),
                                page_break_before, list,
                                // 深さ: w:ind(直接指定)が無ければ w:ilvl から
                                indent: indent.max(ilvl),
                                line_spacing,
                                style: pstyle,
                                shade: shade.take(), boxed,
                                dropcap: false,
                                images_new: Vec::new(),
                                runs: if runs.is_empty() {
                                vec![Run { text: String::new(), size_pt: DEFAULT_PT, font: None, fmt: Default::default() }]
                            } else { runs } };
                            // ドロップキャップの枠の段落は、次の段落の頭に合流する
                            if dropcap && !p.runs.iter().all(|r| r.text.is_empty()) {
                                pending_cap = Some(p.runs);
                            } else {
                                if let Some(mut cap) = pending_cap.take() {
                                    // 頭の字の大きさは本文に合わせる(2.8倍は組むとき掛かる。
                                    // 読んだ大きさのまま持つと保存のたびに育つ)
                                    let body_pt = p.runs.first().map(|r| r.size_pt)
                                        .unwrap_or(DEFAULT_PT);
                                    for r in &mut cap {
                                        r.size_pt = body_pt;
                                    }
                                    cap.extend(p.runs);
                                    p.runs = cap;
                                    p.dropcap = true;
                                }
                                // 表のセルの中なら、そのセルへ。外なら本文へ
                                match stack.last_mut() {
                                    Some(b) => b.cell.push(p),
                                    None => doc.push_para(p),
                                }
                            }
                        }
                    }
                    b"tc" => if let Some(b) = stack.last_mut() {
                        // 枠の段落だけで終わったセルは、そのまま段落として置く
                        if let Some(cap) = pending_cap.take() {
                            b.cell.push(Paragraph {
                                dropcap: true,
                                line_spacing: 1.0,
                                runs: cap,
                                ..Default::default()
                            });
                        }
                        let paras = std::mem::take(&mut b.cell);
                        b.row.push(Cellbox {
                            paragraphs: paras,
                            col_span: cell_span,
                            v_merge: cell_vmerge,
                        });
                        cell_span = 0;
                        cell_vmerge = VMerge::None;
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

const RNS_DOC: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// 文書を document.xml の本体にする。
pub fn write_document_xml(doc: &Document) -> String {
    write_document_parts(doc).0
}

/// 本体と、このアプリで挿した画像(出て来る順)を返す。
pub fn write_document_parts(doc: &Document) -> (String, Vec<std::sync::Arc<Vec<u8>>>) {
    let (body, media, _) = write_document_full(doc);
    (body, media)
}

/// 本体・挿した画像・段落のコメント(参照した順)を返す。
/// 画像の番号(rIdJO1〜)とコメントの番号(1〜)はこの順で振られる。
fn write_para(w: &mut Writer<Cursor<Vec<u8>>>, p: &Paragraph,
        imgn: &mut usize, media: &mut Vec<std::sync::Arc<Vec<u8>>>,
        cmts: &mut Vec<Comment>, bmn: &mut usize,
        trkn: &mut usize, author: &str) {
        use quick_xml::events::{BytesEnd, BytesStart as BS, BytesText};
        // ドロップキャップは Word の作法どおり
        // 「枠の段落(頭の1字・大きめ)+本文の段落」に割って書く
        if p.dropcap {
            if let Some(ch) = p.runs.first().and_then(|r| r.text.chars().next()) {
                let r0 = p.runs.first().unwrap();
                let cap_pt = ((r0.size_pt * 2.8 * 2.0).round() as i32).to_string();
                let font_xml = r0.font.as_deref().map(|f| format!(
                    r#"<w:rFonts w:ascii="{f}" w:hAnsi="{f}" w:eastAsia="{f}"/>"#,
                    f = esc(f))).unwrap_or_default();
                let _ = w.get_mut().write_all(format!(
                    concat!(
                        r#"<w:p><w:pPr><w:framePr w:dropCap="drop" w:lines="2" "#,
                        r#"w:wrap="around" w:vAnchor="text" w:hAnchor="text"/></w:pPr>"#,
                        r#"<w:r><w:rPr>{font}<w:sz w:val="{sz}"/></w:rPr>"#,
                        r#"<w:t xml:space="preserve">{t}</w:t></w:r></w:p>"#
                    ),
                    font = font_xml, sz = cap_pt, t = esc(&ch.to_string())
                ).as_bytes());
                let mut rest = p.clone();
                rest.dropcap = false;
                if let Some(r) = rest.runs.first_mut() {
                    r.text = r.text[ch.len_utf8()..].to_string();
                }
                write_para(w, &rest, imgn, media, cmts, bmn, trkn, author);
                return;
            }
        }
        w.write_event(Event::Start(BS::new("w:p"))).unwrap();
        // 段落の性質。既定のものは書かない — 余計な指定を増やさない
        let has_ppr = p.align != Align::Left
            || p.page_break_before
            || p.list != ListKind::None
            || p.indent > 0
            || (p.spacing() - 1.0).abs() > 0.001
            || p.shade.is_some()
            || p.boxed
            || p.style != ParaStyle::Body;
        if has_ppr {
            w.write_event(Event::Start(BS::new("w:pPr"))).unwrap();
            // 段落のスタイル(pPr の先頭に置く — スキーマの並び)
            match p.style {
                ParaStyle::Heading(n) => {
                    let mut st = BS::new("w:pStyle");
                    st.push_attribute(("w:val", format!("Heading{n}").as_str()));
                    w.write_event(Event::Empty(st)).unwrap();
                }
                ParaStyle::Toc(n) => {
                    let mut st = BS::new("w:pStyle");
                    st.push_attribute(("w:val", format!("TOC{n}").as_str()));
                    w.write_event(Event::Empty(st)).unwrap();
                }
                ParaStyle::Tof => {
                    let mut st = BS::new("w:pStyle");
                    st.push_attribute(("w:val", "TableofFigures"));
                    w.write_event(Event::Empty(st)).unwrap();
                }
                ParaStyle::Body => {}
            }
            if p.page_break_before {
                w.write_event(Event::Empty(BS::new("w:pageBreakBefore"))).unwrap();
            }
            // 囲み枠(4辺とも同じ細線)
            if p.boxed {
                w.write_event(Event::Start(BS::new("w:pBdr"))).unwrap();
                for side in ["top", "left", "bottom", "right"] {
                    let tag = format!("w:{side}");
                    let mut b = BS::new(tag.as_str());
                    b.push_attribute(("w:val", "single"));
                    b.push_attribute(("w:sz", "4"));
                    b.push_attribute(("w:space", "4"));
                    b.push_attribute(("w:color", "000000"));
                    w.write_event(Event::Empty(b)).unwrap();
                }
                w.write_event(Event::End(BytesEnd::new("w:pBdr"))).unwrap();
            }
            // 背景色
            if let Some(c) = &p.shade {
                let mut sh = BS::new("w:shd");
                sh.push_attribute(("w:val", "clear"));
                sh.push_attribute(("w:color", "auto"));
                sh.push_attribute(("w:fill", c.as_str()));
                w.write_event(Event::Empty(sh)).unwrap();
            }
            if p.list != ListKind::None {
                w.write_event(Event::Start(BS::new("w:numPr"))).unwrap();
                let mut lv = BS::new("w:ilvl");
                lv.push_attribute(("w:val", p.indent.min(8).to_string().as_str()));
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
            // 見出しの階層。スタイル定義(styles.xml)が無い文書でも
            // 見出しとして扱われるように、outlineLvl も付けておく
            if let ParaStyle::Heading(n) = p.style {
                let mut ol = BS::new("w:outlineLvl");
                ol.push_attribute(("w:val", (n - 1).to_string().as_str()));
                w.write_event(Event::Empty(ol)).unwrap();
            }
            w.write_event(Event::End(BytesEnd::new("w:pPr"))).unwrap();
        }
        // しおり。範囲は段落まるごと。名前は読んだものを返す(REF が指す)
        let bm0 = *bmn;
        for name in &p.bookmarks {
            *bmn += 1;
            let _ = w.get_mut().write_all(format!(
                r#"<w:bookmarkStart w:id="{}" w:name="{}"/>"#, *bmn, esc(name)).as_bytes());
        }
        // 段落のコメント。範囲は段落まるごと(段落単位の粒度)。
        // 番号は文書を通しで振り、comments.xml 側と同じ順で並ぶ
        let cmt_ids: Vec<usize> = p.comments.iter()
            .map(|c| {
                cmts.push(c.clone());
                cmts.len()
            })
            .collect();
        for id in &cmt_ids {
            let _ = w.get_mut().write_all(
                format!(r#"<w:commentRangeStart w:id="{id}"/>"#).as_bytes());
        }
        // 読めなかった要素(画像など)を原文のまま返す。
        // 段落の並びの中の位置は失われ、末尾に戻る(正直な限界)
        for a in &p.anchors {
            // anchors は宣言付きの <w:r>…</w:r> を丸ごと持っている
            let _ = w.get_mut().write_all(a.as_bytes());
        }
        for run in &p.runs {
            if run.text.is_empty() { continue }
            // 相互参照はフィールドとして書く(見えている値をキャッシュに持つ)
            if let Some(rf) = &run.fmt.field {
                let instr = if rf.page {
                    format!(" PAGEREF {} \\h ", rf.name)
                } else {
                    format!(" REF {} \\h ", rf.name)
                };
                let b = if run.fmt.bold { "<w:b/>" } else { "" };
                let color = run.fmt.color.as_deref()
                    .map(|c| format!(r#"<w:color w:val="{c}"/>"#))
                    .unwrap_or_default();
                let _ = w.get_mut().write_all(format!(
                    concat!(
                        r#"<w:fldSimple w:instr="{instr}"><w:r><w:rPr>{b}{color}"#,
                        r#"<w:sz w:val="{sz}"/></w:rPr>"#,
                        r#"<w:t xml:space="preserve">{t}</w:t></w:r></w:fldSimple>"#
                    ),
                    instr = esc(&instr),
                    b = b,
                    color = color,
                    sz = (run.size_pt * 2.0).round() as i32,
                    t = esc(&run.text),
                ).as_bytes());
                continue;
            }
            // ページ番号・ページ数の印はフィールドとして書く
            // (印の前後で run を割る)。中の「1」は開く側が計算し直す種
            // 0=普通 1=挿入(w:ins) 2=削除(w:del)。変更履歴の印で切り替える
            let mut chunks: Vec<(u8, Option<char>, String)> =
                vec![(0, None, String::new())];
            let mut mode = 0u8;
            for ch in run.text.chars() {
                match ch {
                    TRK_INS_S => { mode = 1; chunks.push((mode, None, String::new())); }
                    TRK_DEL_S => { mode = 2; chunks.push((mode, None, String::new())); }
                    TRK_INS_E | TRK_DEL_E => {
                        mode = 0;
                        chunks.push((mode, None, String::new()));
                    }
                    PAGE_MARK | PAGES_MARK => chunks.push((mode, Some(ch), String::new())),
                    _ => chunks.last_mut().unwrap().2.push(ch),
                }
            }
            for (mode, mark, chunk) in chunks {
            if let Some(mk) = mark {
                let instr = if mk == PAGE_MARK { " PAGE " } else { " NUMPAGES " };
                let _ = w.get_mut().write_all(format!(
                    r#"<w:fldSimple w:instr="{instr}"><w:r><w:t>1</w:t></w:r></w:fldSimple>"#
                ).as_bytes());
            }
            if chunk.is_empty() { continue }
            // 変更履歴。挿入・削除は w:ins / w:del で包む(著者つき)
            if mode != 0 {
                *trkn += 1;
                let tag = if mode == 1 { "ins" } else { "del" };
                let _ = w.get_mut().write_all(format!(
                    r#"<w:{tag} w:id="{}" w:author="{}">"#, *trkn, esc(author)
                ).as_bytes());
            }
            let ttag = if mode == 2 { "w:delText" } else { "w:t" };
            // ルビ。基底の run を w:ruby(rt + rubyBase)で包む(Word の作法)
            let ruby_rt = if mode == 0 {
                run.fmt.ruby.as_deref().filter(|t| !t.is_empty())
            } else {
                None
            };
            if let Some(rt) = ruby_rt {
                let hps = run.size_pt.round() as i32; // 半分の大きさ(半ポイント)
                let base_sz = (run.size_pt * 2.0).round() as i32;
                let raise = (run.size_pt * 2.0 * 0.9).round() as i32;
                let _ = w.get_mut().write_all(format!(
                    concat!(
                        // w:ruby は run の中(ECMA-376 §17.3.3.25 — 親は w:r)。
                        // 段落直下に置くと LibreOffice が基底ごと落とす(実測)
                        r#"<w:r><w:ruby><w:rubyPr><w:rubyAlign w:val="center"/>"#,
                        r#"<w:hps w:val="{hps}"/><w:hpsRaise w:val="{raise}"/>"#,
                        r#"<w:hpsBaseText w:val="{base}"/><w:lid w:val="ja-JP"/>"#,
                        r#"</w:rubyPr><w:rt><w:r><w:rPr>"#,
                        r#"<w:rFonts w:hint="eastAsia"/><w:sz w:val="{hps}"/>"#,
                        r#"</w:rPr><w:t xml:space="preserve">{rt}</w:t></w:r></w:rt>"#,
                        r#"<w:rubyBase>"#
                    ),
                    hps = hps, raise = raise, base = base_sz, rt = esc(rt)
                ).as_bytes());
            }
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
            if run.fmt.superscript || run.fmt.subscript {
                let mut va = BS::new("w:vertAlign");
                va.push_attribute(("w:val",
                    if run.fmt.superscript { "superscript" } else { "subscript" }));
                w.write_event(Event::Empty(va)).unwrap();
            }
            if let Some(h) = &run.fmt.highlight {
                let mut hl = BS::new("w:highlight");
                hl.push_attribute(("w:val", h.as_str()));
                w.write_event(Event::Empty(hl)).unwrap();
            }
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
            for (i, seg) in chunk.split('\n').enumerate() {
                if i > 0 { w.write_event(Event::Empty(BS::new("w:br"))).unwrap(); }
                // タブは要素(w:tab)。w:t の中に生のタブを書くと Word が潰す
                for (j, part) in seg.split('\t').enumerate() {
                    if j > 0 { w.write_event(Event::Empty(BS::new("w:tab"))).unwrap(); }
                    if part.is_empty() { continue }
                    let mut t = BS::new(ttag);
                    t.push_attribute(("xml:space", "preserve"));
                    w.write_event(Event::Start(t)).unwrap();
                    w.write_event(Event::Text(BytesText::new(part))).unwrap();
                    w.write_event(Event::End(BytesEnd::new(ttag))).unwrap();
                }
            }
            w.write_event(Event::End(BytesEnd::new("w:r"))).unwrap();
            if ruby_rt.is_some() {
                let _ = w.get_mut().write_all(b"</w:rubyBase></w:ruby></w:r>");
            }
            if mode != 0 {
                let tag = if mode == 1 { "ins" } else { "del" };
                let _ = w.get_mut().write_all(format!("</w:{tag}>").as_bytes());
            }
            }
        }
        // このアプリで挿した画像。部品(media・rels)は write_with が同じ番号で作る
        for im in &p.images_new {
            *imgn += 1;
            let n = *imgn;
            let (cx, cy) = ((im.w_mm * 36000.0) as i64, (im.h_mm * 36000.0) as i64);
            let xml = format!(
                r#"<w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{cx}" cy="{cy}"/><wp:docPr id="{n}" name="図{n}"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="{n}" name="図{n}"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rIdJO{n}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#
            );
            let _ = w.get_mut().write_all(xml.as_bytes());
            media.push(im.bytes.clone());
        }
        for id in &cmt_ids {
            let _ = w.get_mut().write_all(format!(
                r#"<w:commentRangeEnd w:id="{id}"/><w:r><w:commentReference w:id="{id}"/></w:r>"#
            ).as_bytes());
        }
        for i in 0..p.bookmarks.len() {
            let _ = w.get_mut().write_all(
                format!(r#"<w:bookmarkEnd w:id="{}"/>"#, bm0 + i + 1).as_bytes());
        }
        w.write_event(Event::End(BytesEnd::new("w:p"))).unwrap();
}

/// ヘッダー・フッターの部品(headerN.xml / footerN.xml)の中身を作る。
fn hf_xml(hf: &kumihan::HeadFoot, footer: bool) -> String {
    use quick_xml::events::{BytesEnd, BytesStart as BS};
    let root_name = if footer { "w:ftr" } else { "w:hdr" };
    let mut w = Writer::new(Cursor::new(Vec::new()));
    let mut root = BS::new(root_name);
    root.push_attribute(("xmlns:w", W_NS));
    root.push_attribute(("xmlns:r", RNS_DOC));
    w.write_event(Event::Start(root)).unwrap();
    // 画像の挿入・コメントはヘッダーには無い(集め先はここでは増えない)
    let (mut imgn, mut media, mut cmts, mut bmn, mut trkn) =
        (0usize, Vec::new(), Vec::new(), 0usize, 0usize);
    for p in &hf.paragraphs {
        write_para(&mut w, p, &mut imgn, &mut media, &mut cmts, &mut bmn, &mut trkn, "");
    }
    w.write_event(Event::End(BytesEnd::new(root_name))).unwrap();
    let body = String::from_utf8(w.into_inner().into_inner()).unwrap();
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n{body}")
}

fn write_document_full(doc: &Document) -> (String, Vec<std::sync::Arc<Vec<u8>>>, Vec<Comment>) {
    use quick_xml::events::{BytesEnd, BytesStart as BS};
    let mut w = Writer::new(Cursor::new(Vec::new()));

    let mut root = BS::new("w:document");
    root.push_attribute(("xmlns:w", W_NS));
    root.push_attribute(("xmlns:r", RNS_DOC));
    root.push_attribute(("xmlns:wp",
        "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"));
    root.push_attribute(("xmlns:a", "http://schemas.openxmlformats.org/drawingml/2006/main"));
    root.push_attribute(("xmlns:pic",
        "http://schemas.openxmlformats.org/drawingml/2006/picture"));
    w.write_event(Event::Start(root)).unwrap();
    // ページの色。w:body の前に置く(スキーマの並び)。
    // 見せるには settings.xml の displayBackgroundShape も要る(write_with が足す)
    if let Some(c) = &doc.page_color {
        let mut bg = BS::new("w:background");
        bg.push_attribute(("w:color", c.as_str()));
        w.write_event(Event::Empty(bg)).unwrap();
    }
    w.write_event(Event::Start(BS::new("w:body"))).unwrap();

    let mut imgn = 0usize;
    let mut media: Vec<std::sync::Arc<Vec<u8>>> = Vec::new();
    let mut cmts: Vec<Comment> = Vec::new();
    let mut bmn = 0usize;
    let mut trkn = 0usize;
    let author = doc.track_author.clone().unwrap_or_default();
    for b in &doc.blocks {
        match b {
            Block::Para(p) => write_para(&mut w, p, &mut imgn, &mut media, &mut cmts, &mut bmn, &mut trkn, &author),
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
                        // セル結合を返す(読んだものを捨てると様式の枠が壊れる)
                        if cell.col_span > 1 || cell.v_merge != VMerge::None {
                            w.write_event(Event::Start(BS::new("w:tcPr"))).unwrap();
                            if cell.col_span > 1 {
                                let mut g = BS::new("w:gridSpan");
                                let v = cell.col_span.to_string();
                                g.push_attribute(("w:val", v.as_str()));
                                w.write_event(Event::Empty(g)).unwrap();
                            }
                            match cell.v_merge {
                                VMerge::Start => {
                                    let mut m = BS::new("w:vMerge");
                                    m.push_attribute(("w:val", "restart"));
                                    w.write_event(Event::Empty(m)).unwrap();
                                }
                                VMerge::Continue => {
                                    // val 無しが「続き」(docx の既定)
                                    w.write_event(Event::Empty(BS::new("w:vMerge"))).unwrap();
                                }
                                VMerge::None => {}
                            }
                            w.write_event(Event::End(BytesEnd::new("w:tcPr"))).unwrap();
                        }
                        if cell.paragraphs.is_empty() {
                            write_para(&mut w, &Paragraph { line_spacing: 1.0, ..Default::default() },
                                 &mut imgn, &mut media, &mut cmts, &mut bmn, &mut trkn, &author);
                        } else {
                            for p in &cell.paragraphs {
                                write_para(&mut w, p, &mut imgn, &mut media, &mut cmts, &mut bmn, &mut trkn, &author)
                            }
                        }
                        w.write_event(Event::End(BytesEnd::new("w:tc"))).unwrap();
                    }
                    w.write_event(Event::End(BytesEnd::new("w:tr"))).unwrap();
                }
                w.write_event(Event::End(BytesEnd::new("w:tbl"))).unwrap();
            }
        }
    }

    // 節の設定を原文のまま返す(用紙・余白・ヘッダーの参照)。
    // このアプリで足したヘッダー・フッターの参照が無ければ差し込む
    // (参照が無いと、部品を書いても表示されない)
    let mut sect = doc.sect_raw.clone();
    {
        let cur = sect.as_deref().unwrap_or("");
        let mut refs = String::new();
        // 透かしはヘッダーに入るので、透かしがあればヘッダーの参照も要る
        // (編集できないヘッダー(表入り)には差し込まないので、そこは除く)
        let need_hdr = !doc.header.paragraphs.is_empty()
            || (doc.watermark.as_deref().is_some_and(|t| !t.is_empty())
                && !(doc.header.paragraphs.is_empty() && doc.header.part.is_some()));
        if need_hdr && hf_ref(cur, "headerReference").is_none() {
            refs.push_str(r#"<w:headerReference w:type="default" r:id="rIdJOhdr"/>"#);
        }
        if !doc.footer.paragraphs.is_empty() && hf_ref(cur, "footerReference").is_none() {
            refs.push_str(r#"<w:footerReference w:type="default" r:id="rIdJOftr"/>"#);
        }
        if !refs.is_empty() {
            sect = Some(match sect {
                // 参照は sectPr の先頭に置く(スキーマの並び)
                Some(s) if s.contains("</w:sectPr>") => match s.find('>') {
                    Some(i) => format!("{}{}{}", &s[..i + 1], refs, &s[i + 1..]),
                    None => s,
                },
                // 無い・空(<w:sectPr/>)なら作る
                _ => format!("<w:sectPr>{refs}</w:sectPr>"),
            });
        }
    }
    // 縦書きの旗。sectPr の textDirection を旗に合わせて置き直す
    if doc.vertical {
        let mut s2 = sect.take().unwrap_or_else(|| "<w:sectPr></w:sectPr>".into());
        if !s2.contains("</w:sectPr>") {
            // <w:sectPr/> の形
            s2 = "<w:sectPr></w:sectPr>".into();
        }
        if !s2.contains("<w:textDirection") {
            if let Some(i) = s2.rfind("</w:sectPr>") {
                s2.insert_str(i, r#"<w:textDirection w:val="tbRl"/>"#);
            }
        }
        sect = Some(s2);
    } else if let Some(s2) = sect.as_mut() {
        if let Some(i) = s2.find("<w:textDirection") {
            if let Some(j) = s2[i..].find("/>") {
                s2.replace_range(i..i + j + 2, "");
            }
        }
    }
    if let Some(sect) = &sect {
        let _ = w.get_mut().write_all(sect.as_bytes());
    }
    w.write_event(Event::End(BytesEnd::new("w:body"))).unwrap();
    w.write_event(Event::End(BytesEnd::new("w:document"))).unwrap();
    let body = String::from_utf8(w.into_inner().into_inner()).unwrap();
    (format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n{body}"), media, cmts)
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
/// 画像の中身から拡張子と content type を見分ける。
fn image_kind(bytes: &[u8]) -> (&'static str, &'static str) {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        ("png", "image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        ("jpeg", "image/jpeg")
    } else {
        ("bin", "application/octet-stream")
    }
}

pub fn write_with<R: Read + Seek, W: Write + Seek>(
    doc: &Document,
    original: Option<R>,
    dst: W,
) -> Result<(), String> {
    let mut zip = zip::ZipWriter::new(dst);
    let opts: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let (body, new_media, cmts_out) = write_document_full(doc);
    // 今回こちらが作り直す部品の名前(これ以外の joimg は既存画像として持ち越す)
    let regen_media: Vec<String> = new_media.iter().enumerate()
        .map(|(i, m)| {
            let (ext, _) = image_kind(m);
            format!("word/media/joimg{}.{ext}", i + 1)
        })
        .collect();
    // ヘッダー・フッター。モデルに持っているもの(編集できたもの)だけ作り直す。
    // paragraphs が空 = 触っていない/持てなかった部品 → 原本のまま持ち越す。
    // 透かしはヘッダーの1段落目に VML として差し込む(Word の作法)
    let mut hdr_src = doc.header.clone();
    if let Some(text) = doc.watermark.as_deref().filter(|t| !t.is_empty()) {
        if hdr_src.paragraphs.is_empty() && hdr_src.part.is_some() {
            // 編集できないヘッダー(表入り)には差し込まない(壊すより見送る)
        } else {
            if hdr_src.paragraphs.is_empty() {
                hdr_src.paragraphs.push(Paragraph::default());
            }
            if let Some(wrapped) = wrap_with_ns(&watermark_vml(text), &Default::default()) {
                hdr_src.paragraphs[0].anchors.push(wrapped);
            }
        }
    }
    let hdr: Option<(String, String)> = (!hdr_src.paragraphs.is_empty()).then(|| (
        hdr_src.part.clone().unwrap_or_else(|| "word/johdr1.xml".to_string()),
        hf_xml(&hdr_src, false),
    ));
    let ftr: Option<(String, String)> = (!doc.footer.paragraphs.is_empty()).then(|| (
        doc.footer.part.clone().unwrap_or_else(|| "word/joftr1.xml".to_string()),
        hf_xml(&doc.footer, true),
    ));

    // [Content_Types] と本文の rels は、挿した画像のぶんを織り込んで作り直す
    let mut orig_ct: Option<String> = None;
    let mut orig_rels: Option<String> = None;
    let mut orig_settings: Option<String> = None;
    let mut orig_core: Option<String> = None;
    let mut orig_root_rels: Option<String> = None;
    let has_props = doc.props != Default::default();
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
                if name == "[Content_Types].xml" {
                    orig_ct = Some(String::from_utf8_lossy(&buf).to_string());
                    continue;
                }
                if name == "word/_rels/document.xml.rels" {
                    orig_rels = Some(String::from_utf8_lossy(&buf).to_string());
                    continue;
                }
                // 設定(ページの色・ハイフネーション)は織り込んで書き直す
                if name == "word/settings.xml" {
                    orig_settings = Some(String::from_utf8_lossy(&buf).to_string());
                    continue;
                }
                // 文書の情報。作成者などを織り込んで書き直す
                if name == "docProps/core.xml" {
                    orig_core = Some(String::from_utf8_lossy(&buf).to_string());
                    continue;
                }
                if name == "_rels/.rels" {
                    orig_root_rels = Some(String::from_utf8_lossy(&buf).to_string());
                    continue;
                }
                // コメントを持っているなら comments.xml はこちらが作り直す
                if name == "word/comments.xml" && !cmts_out.is_empty() {
                    continue;
                }
                // 今回作り直す画像の実体だけは持ち越さない(二重に持たない)。
                // 開き直した後の joimg は「既存の画像」なので、普通に持ち越す
                if regen_media.contains(&name) {
                    continue;
                }
                // 作り直すヘッダー・フッターの部品も持ち越さない(後で書く)
                if hdr.as_ref().is_some_and(|(n, _)| *n == name)
                    || ftr.as_ref().is_some_and(|(n, _)| *n == name)
                {
                    continue;
                }
                zip.start_file(name, opts).map_err(|e| e.to_string())?;
                zip.write_all(&buf).map_err(|e| e.to_string())?;
            }
        }
    }

    // [Content_Types]。挿した画像の拡張子とヘッダー・フッターの宣言が無ければ足す
    {
        let mut ct = orig_ct.unwrap_or_else(|| CONTENT_TYPES.to_string());
        let mut add = String::new();
        for m in &new_media {
            let (ext, ty) = image_kind(m);
            let decl = format!(r#"<Default Extension="{ext}" ContentType="{ty}"/>"#);
            if !ct.contains(&format!("Extension=\"{ext}\"")) && !add.contains(&decl) {
                add.push_str(&decl);
            }
        }
        for (name, ty) in [
            (hdr.as_ref().map(|(n, _)| n.as_str()),
             "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"),
            (ftr.as_ref().map(|(n, _)| n.as_str()),
             "application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"),
            ((doc.page_color.is_some() || doc.hyphenate || doc.protection.is_some())
                .then_some("word/settings.xml"),
             "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"),
            ((!cmts_out.is_empty()).then_some("word/comments.xml"),
             "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"),
            ((has_props || orig_core.is_some()).then_some("docProps/core.xml"),
             "application/vnd.openxmlformats-package.core-properties+xml"),
        ] {
            let Some(name) = name else { continue };
            if !ct.contains(&format!("PartName=\"/{name}\"")) {
                add.push_str(&format!(
                    r#"<Override PartName="/{name}" ContentType="{ty}"/>"#));
            }
        }
        if !add.is_empty() {
            if let Some(p) = ct.rfind("</Types>") {
                ct.insert_str(p, &add);
            }
        }
        zip.start_file("[Content_Types].xml", opts).map_err(|e| e.to_string())?;
        zip.write_all(ct.as_bytes()).map_err(|e| e.to_string())?;
    }
    {
        let mut rr = orig_root_rels.unwrap_or_else(|| ROOT_RELS.to_string());
        if (has_props || orig_core.is_some())
            && !rr.contains("Target=\"docProps/core.xml\"")
        {
            if let Some(i) = rr.rfind("</Relationships>") {
                rr.insert_str(i, concat!(
                    r#"<Relationship Id="rIdJOcore" "#,
                    r#"Type="http://schemas.openxmlformats.org/package/2006/"#,
                    r#"relationships/metadata/core-properties" "#,
                    r#"Target="docProps/core.xml"/>"#,
                ));
            }
        }
        zip.start_file("_rels/.rels", opts).map_err(|e| e.to_string())?;
        zip.write_all(rr.as_bytes()).map_err(|e| e.to_string())?;
    }

    // 文書の情報(docProps/core.xml)。原本の他の欄(日時など)は残し、
    // こちらが持つ5つの欄だけ置き直す。空の欄は消す
    if has_props || orig_core.is_some() {
        let mut cx = orig_core.unwrap_or_else(|| concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
            "<cp:coreProperties ",
            "xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" ",
            "xmlns:dc=\"http://purl.org/dc/elements/1.1/\" ",
            "xmlns:dcterms=\"http://purl.org/dc/terms/\" ",
            "xmlns:dcmitype=\"http://purl.org/dc/dcmitype/\" ",
            "xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">",
            "</cp:coreProperties>",
        ).to_string());
        for (tag, val) in [
            ("dc:creator", &doc.props.creator),
            ("dc:title", &doc.props.title),
            ("cp:keywords", &doc.props.keywords),
            ("dc:subject", &doc.props.subject),
            ("dc:description", &doc.props.description),
        ] {
            // 既存の欄を外す(<tag>…</tag> と <tag/> の両方)
            let open = format!("<{tag}");
            while let Some(i) = cx.find(&open) {
                let close = format!("</{tag}>");
                if let Some(j) = cx[i..].find(&close) {
                    cx.replace_range(i..i + j + close.len(), "");
                } else if let Some(j) = cx[i..].find("/>") {
                    cx.replace_range(i..i + j + 2, "");
                } else {
                    break;
                }
            }
            if !val.is_empty() {
                if let Some(i) = cx.rfind("</cp:coreProperties>") {
                    cx.insert_str(i, &format!("<{tag}>{}</{tag}>", esc(val)));
                }
            }
        }
        zip.start_file("docProps/core.xml", opts).map_err(|e| e.to_string())?;
        zip.write_all(cx.as_bytes()).map_err(|e| e.to_string())?;
    }

    // 本文の rels。原本の関係(既存の画像・ヘッダー等)は残し、
    // 挿した画像のぶん(rIdJO1〜)と、新しく作るヘッダー・フッターを足す
    if orig_rels.is_some() || !new_media.is_empty() || hdr.is_some() || ftr.is_some()
        || doc.page_color.is_some() || doc.hyphenate || doc.protection.is_some()
        || !cmts_out.is_empty()
    {
        let mut rels = orig_rels.unwrap_or_else(|| {
            concat!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
                "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"></Relationships>"
            ).to_string()
        });
        // 今回作り直す番号(rIdJO1〜n)だけ除いておく。残すと Id が重なる。
        // それより先の rIdJO は開き直した既存画像の参照なので触らない
        for n in 1..=new_media.len() {
            let needle = format!("<Relationship Id=\"rIdJO{n}\"");
            if let Some(i) = rels.find(&needle) {
                if let Some(j) = rels[i..].find("/>") {
                    rels.replace_range(i..i + j + 2, "");
                }
            }
        }
        let mut add = String::new();
        for (i, m) in new_media.iter().enumerate() {
            let n = i + 1;
            let (ext, _) = image_kind(m);
            add.push_str(&format!(
                r#"<Relationship Id="rIdJO{n}" Type="{RNS_DOC}/image" Target="media/joimg{n}.{ext}"/>"#
            ));
        }
        // コメント(comments.xml)への関係。無いときだけ足す
        if !cmts_out.is_empty() && !rels.contains("Target=\"comments.xml\"") {
            add.push_str(&format!(
                r#"<Relationship Id="rIdJOcm" Type="{RNS_DOC}/comments" Target="comments.xml"/>"#
            ));
        }
        // 設定(settings.xml)への関係。素の文書に設定を足したときだけ要る
        if (doc.page_color.is_some() || doc.hyphenate || doc.protection.is_some())
            && !rels.contains("Target=\"settings.xml\"")
        {
            add.push_str(&format!(
                r#"<Relationship Id="rIdJOset" Type="{RNS_DOC}/settings" Target="settings.xml"/>"#
            ));
        }
        // このアプリで作るヘッダー・フッター(johdr/joftr)の関係。
        // 原本由来の部品(header1.xml 等)は参照も rels も原本のまま
        for (rid, ty, part) in [
            hdr.as_ref().map(|(n, _)| ("rIdJOhdr", "header", n.as_str())),
            ftr.as_ref().map(|(n, _)| ("rIdJOftr", "footer", n.as_str())),
        ].into_iter().flatten() {
            let Some(target) = part.strip_prefix("word/") else { continue };
            if !target.starts_with("johdr") && !target.starts_with("joftr") {
                continue;
            }
            // 前の保存が残した同じ Id は除く(残すと Id が重なる)
            let needle = format!("<Relationship Id=\"{rid}\"");
            if let Some(i) = rels.find(&needle) {
                if let Some(j) = rels[i..].find("/>") {
                    rels.replace_range(i..i + j + 2, "");
                }
            }
            add.push_str(&format!(
                r#"<Relationship Id="{rid}" Type="{RNS_DOC}/{ty}" Target="{target}"/>"#
            ));
        }
        if let Some(p) = rels.rfind("</Relationships>") {
            rels.insert_str(p, &add);
        }
        zip.start_file("word/_rels/document.xml.rels", opts).map_err(|e| e.to_string())?;
        zip.write_all(rels.as_bytes()).map_err(|e| e.to_string())?;
    }
    // 画像の実体
    for (i, m) in new_media.iter().enumerate() {
        let n = i + 1;
        let (ext, _) = image_kind(m);
        zip.start_file(format!("word/media/joimg{n}.{ext}"), opts)
            .map_err(|e| e.to_string())?;
        zip.write_all(m).map_err(|e| e.to_string())?;
    }
    // ヘッダー・フッターの部品
    for (name, xml) in [&hdr, &ftr].into_iter().flatten() {
        zip.start_file(name.clone(), opts).map_err(|e| e.to_string())?;
        zip.write_all(xml.as_bytes()).map_err(|e| e.to_string())?;
    }
    // コメントの部品
    if !cmts_out.is_empty() {
        zip.start_file("word/comments.xml", opts).map_err(|e| e.to_string())?;
        zip.write_all(comments_xml(&cmts_out).as_bytes()).map_err(|e| e.to_string())?;
    }
    // 設定(settings.xml)。ページの色を見せる旗と、ハイフネーションの旗を
    // 織り込む。原本の settings は他の設定ごと生かす(丸ごと作り直さない)
    if doc.page_color.is_some() || doc.hyphenate || doc.protection.is_some()
        || orig_settings.is_some()
    {
        let mut st = orig_settings.unwrap_or_else(|| concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
            "<w:settings xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"></w:settings>"
        ).to_string());
        if doc.page_color.is_some() && !st.contains("displayBackgroundShape") {
            if let Some(i) = st.rfind("</w:settings>") {
                st.insert_str(i, "<w:displayBackgroundShape/>");
            }
        }
        if doc.hyphenate {
            if !st.contains("<w:autoHyphenation") {
                if let Some(i) = st.rfind("</w:settings>") {
                    st.insert_str(i, "<w:autoHyphenation/>");
                }
            }
        } else if let Some(i) = st.find("<w:autoHyphenation") {
            if let Some(j) = st[i..].find("/>") {
                st.replace_range(i..i + j + 2, "");
            }
        }
        // 文書の保護。旗に合わせて置き直す(古いものは外してから)
        if let Some(i) = st.find("<w:documentProtection") {
            if let Some(j) = st[i..].find("/>") {
                st.replace_range(i..i + j + 2, "");
            }
        }
        if let Some(edit) = &doc.protection {
            if let Some(i) = st.rfind("</w:settings>") {
                st.insert_str(i, &format!(
                    r#"<w:documentProtection w:edit="{}" w:enforcement="1"/>"#,
                    esc(edit)
                ));
            }
        }
        zip.start_file("word/settings.xml", opts).map_err(|e| e.to_string())?;
        zip.write_all(st.as_bytes()).map_err(|e| e.to_string())?;
    }

    zip.start_file("word/document.xml", opts).map_err(|e| e.to_string())?;
    zip.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

// ---------- 検査 ----------

#[cfg(test)]
mod tests {
    use super::*;
    use kumihan::{Align, CharFormat, ListKind, Block, Cellbox, Document, Paragraph, Run, Table};

    fn para(s: &str) -> Paragraph {
        Paragraph { align: Default::default(), style: Default::default(), comments: Vec::new(), bookmarks: Vec::new(), dropcap: false, anchors: Vec::new(),
                    images: Vec::new(), page_break_before: false,
                    list: Default::default(), indent: 0, line_spacing: 1.0, shade: None, boxed: false, images_new: Vec::new(), runs: vec![Run { text: s.to_string(), size_pt: 10.5, font: None, fmt: Default::default() }] }
    }
    fn doc(parts: &[&str]) -> Document {
        Document { font: None, page: None, sect_raw: None, header: Default::default(), footer: Default::default(), page_color: None, watermark: None, ink: Vec::new(), track_author: None, hyphenate: false, protection: None, props: Default::default(), vertical: false, blocks: parts.iter().map(|s| Block::Para(para(s))).collect() }
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
        let d = Document { font: None, page: None, sect_raw: None, header: Default::default(), footer: Default::default(), page_color: None, watermark: None, ink: Vec::new(), track_author: None, hyphenate: false, protection: None, props: Default::default(), vertical: false, blocks: vec![Block::Para(Paragraph { align: Default::default(), style: Default::default(), comments: Vec::new(), bookmarks: Vec::new(), dropcap: false, anchors: Vec::new(),
                    images: Vec::new(), page_break_before: false,
                    list: Default::default(), indent: 0, line_spacing: 1.0, shade: None, boxed: false, images_new: Vec::new(), runs: vec![
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
        let d = Document { font: None, page: None, sect_raw: None, header: Default::default(), footer: Default::default(), page_color: None, watermark: None, ink: Vec::new(), track_author: None, hyphenate: false, protection: None, props: Default::default(), vertical: false, blocks: vec![Block::Para(Paragraph { align: Default::default(), style: Default::default(), comments: Vec::new(), bookmarks: Vec::new(), dropcap: false, anchors: Vec::new(),
                    images: Vec::new(), page_break_before: false,
                    list: Default::default(), indent: 0, line_spacing: 1.0, shade: None, boxed: false, images_new: Vec::new(), runs: vec![
            Run { text: "一行目\n二行目".into(), size_pt: 10.5, font: None, fmt: Default::default() }]})]};
        let (back, _) = round_trip(&d);
        assert_eq!(texts(&back)[0], "一行目\n二行目");
    }

    // ---- 表: 日本の事務様式の本体(実物8件すべてに w:tbl があった) ----

    fn cell(s: &str) -> Cellbox {
        Cellbox { paragraphs: vec![para(s)], ..Default::default() }
    }

    #[test]
    fn 表が往復する() {
        let d = Document { font: None, page: None, sect_raw: None, header: Default::default(), footer: Default::default(), page_color: None, watermark: None, ink: Vec::new(), track_author: None, hyphenate: false, protection: None, props: Default::default(), vertical: false, blocks: vec![
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
        let d = Document { font: None, page: None, sect_raw: None, header: Default::default(), footer: Default::default(), page_color: None, watermark: None, ink: Vec::new(), track_author: None, hyphenate: false, protection: None, props: Default::default(), vertical: false, blocks: vec![
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
        let d = Document { font: None, page: None, sect_raw: None, header: Default::default(), footer: Default::default(), page_color: None, watermark: None, ink: Vec::new(), track_author: None, hyphenate: false, protection: None, props: Default::default(), vertical: false, blocks: vec![Block::Table(Table { col_mm: vec![], rows: vec![
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
        let (doc, rep) = parse_document_xml(xml);
        assert!(!rep.is_lossless());
        assert!(rep.unsupported.iter().any(|(n, _)| n.contains("drawing")),
            "画像の未対応が報告されていない: {:?}", rep.unsupported);
        // セル結合は読めるようになった。報告ではなくモデルに入る
        assert!(!rep.unsupported.iter().any(|(n, _)| n.contains("セル結合")),
            "読めるのに未対応と報告した: {:?}", rep.unsupported);
        let t: Vec<&Table> = doc.tables().collect();
        assert_eq!(t[0].rows[0][0].col_span, 2, "gridSpan が読めていない");
    }

    #[test]
    fn セル結合が往復する() {
        // 様式の見出しは結合で出来ている。往復で消えると枠がずれる
        let mut head = cell("会社概要");
        head.col_span = 2;
        let mut vstart = cell("所在地");
        vstart.v_merge = kumihan::VMerge::Start;
        let mut vcont = Cellbox::default();
        vcont.v_merge = kumihan::VMerge::Continue;
        let d = Document { font: None, page: None, sect_raw: None, header: Default::default(), footer: Default::default(), page_color: None, watermark: None, ink: Vec::new(), track_author: None, hyphenate: false, protection: None, props: Default::default(), vertical: false, blocks: vec![
            Block::Table(Table { col_mm: vec![], rows: vec![
                vec![head],
                vec![vstart, cell("本社")],
                vec![vcont, cell("工場")],
            ]}),
        ]};
        let (back, rep) = round_trip(&d);
        assert!(rep.is_lossless(), "未対応: {:?}", rep.unsupported);
        let t: Vec<&Table> = back.tables().collect();
        assert_eq!(t[0].rows[0][0].col_span, 2, "gridSpan が往復しない");
        assert_eq!(t[0].rows[1][0].v_merge, kumihan::VMerge::Start,
            "vMerge の始まりが往復しない");
        assert_eq!(t[0].rows[2][0].v_merge, kumihan::VMerge::Continue,
            "vMerge の続きが往復しない");
        assert_eq!(t[0].rows[1][1].v_merge, kumihan::VMerge::None,
            "結合していないセルに結合が付いた");
    }

    #[test]
    fn 実物の結合入りの様式が欠落なく読める() {
        // 様式3(会社概要)は gridSpan 15 + vMerge 3 の結合入り
        let src = "/mnt/sdb/home/dev/ドキュメント/機構/yoryou-yoshiki/実施要領様式3_会社概要.docx";
        let Ok(bytes) = std::fs::read(src) else { return };
        let (doc, rep) = crate::read(Cursor::new(bytes)).expect("読めない");
        assert!(!rep.unsupported.iter().any(|(n, _)| n.contains("セル結合")),
            "実物のセル結合が未対応のまま: {:?}", rep.unsupported);
        let spans: usize = doc.tables()
            .flat_map(|t| t.rows.iter())
            .flat_map(|r| r.iter())
            .filter(|c| c.col_span > 1)
            .count();
        assert!(spans > 0, "実物の gridSpan がモデルに入っていない");
        // 往復しても結合が残る
        let mut buf = Cursor::new(Vec::new());
        crate::write(&doc, &mut buf).expect("書けない");
        buf.set_position(0);
        let (back, _) = crate::read(buf).expect("読み直せない");
        let spans2: usize = back.tables()
            .flat_map(|t| t.rows.iter())
            .flat_map(|r| r.iter())
            .filter(|c| c.col_span > 1)
            .count();
        assert_eq!(spans, spans2, "保存で結合が消えた");
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
            page: None,
            sect_raw: None, header: Default::default(), footer: Default::default(), page_color: None, watermark: None, ink: Vec::new(), track_author: None, hyphenate: false, protection: None, props: Default::default(), vertical: false,
            blocks: vec![Block::Para(Paragraph { style: Default::default(), comments: Vec::new(), bookmarks: Vec::new(), dropcap: false,
                align: Default::default(),
                anchors: Vec::new(),
                    images: Vec::new(),
                page_break_before: false,
                    list: Default::default(),
                indent: 0,
                line_spacing: 1.0,
                shade: None, boxed: false, images_new: Vec::new(), runs: vec![Run {
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
            page: None,
            sect_raw: None, header: Default::default(), footer: Default::default(), page_color: None, watermark: None, ink: Vec::new(), track_author: None, hyphenate: false, protection: None, props: Default::default(), vertical: false,
            blocks: vec![Block::Para(Paragraph { align: Align::Left, style: Default::default(), comments: Vec::new(), bookmarks: Vec::new(), dropcap: false, anchors: Vec::new(),
                    images: Vec::new(), page_break_before: false,
                    list: Default::default(), indent: 0, line_spacing: 1.0, shade: None, boxed: false, images_new: Vec::new(), runs: vec![run("見出し", f.clone())] })],
        };
        let back = roundtrip(&d);
        assert_eq!(back.paragraphs().next().unwrap().runs[0].fmt, f, "書式が消えた");
    }

    #[test]
    fn 取り消し線と文字色が往復する() {
        let f = CharFormat { strike: true, color: Some("FF0000".into()), ..Default::default() };
        let d = Document {
            font: None,
            page: None,
            sect_raw: None, header: Default::default(), footer: Default::default(), page_color: None, watermark: None, ink: Vec::new(), track_author: None, hyphenate: false, protection: None, props: Default::default(), vertical: false,
            blocks: vec![Block::Para(Paragraph { align: Align::Left, style: Default::default(), comments: Vec::new(), bookmarks: Vec::new(), dropcap: false, anchors: Vec::new(),
                    images: Vec::new(), page_break_before: false,
                    list: Default::default(), indent: 0, line_spacing: 1.0, shade: None, boxed: false, images_new: Vec::new(), runs: vec![run("赤", f.clone())] })],
        };
        assert_eq!(roundtrip(&d).paragraphs().next().unwrap().runs[0].fmt, f);
    }

    #[test]
    fn 中央揃えが往復する() {
        for a in [Align::Center, Align::Right, Align::Justify, Align::Left] {
            let d = Document {
                font: None,
                page: None,
                sect_raw: None, header: Default::default(), footer: Default::default(), page_color: None, watermark: None, ink: Vec::new(), track_author: None, hyphenate: false, protection: None, props: Default::default(), vertical: false,
                blocks: vec![Block::Para(Paragraph { style: Default::default(), comments: Vec::new(), bookmarks: Vec::new(), dropcap: false,
                    align: a,
                    anchors: Vec::new(),
                    images: Vec::new(),
                    page_break_before: false,
                    list: Default::default(),
                    indent: 0,
                    line_spacing: 1.0,
                    shade: None, boxed: false, images_new: Vec::new(), runs: vec![run("表題", CharFormat::default())],
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
        Paragraph { style: Default::default(), comments: Vec::new(), bookmarks: Vec::new(), dropcap: false,
            align: Align::Left,
            anchors: Vec::new(),
                    images: Vec::new(),
            page_break_before: false,
            list,
            indent,
            line_spacing: spacing,
            shade: None, boxed: false, images_new: Vec::new(),
            runs: vec![Run {
                text: "項目".into(), size_pt: 10.5, font: None, fmt: Default::default(),
            }],
        }
    }

    fn roundtrip(p: Paragraph) -> Paragraph {
        let d = Document { font: None, page: None, sect_raw: None, header: Default::default(), footer: Default::default(), page_color: None, watermark: None, ink: Vec::new(), track_author: None, hyphenate: false, protection: None, props: Default::default(), vertical: false, blocks: vec![Block::Para(p)] };
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
        let d = Document { font: None, page: None, sect_raw: None, header: Default::default(), footer: Default::default(), page_color: None, watermark: None, ink: Vec::new(), track_author: None, hyphenate: false, protection: None, props: Default::default(), vertical: false, blocks: vec![Block::Para(para(ListKind::None, 0, 1.0))] };
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
        let d = Document { font: None, page: None, sect_raw: None, header: Default::default(), footer: Default::default(), page_color: None, watermark: None, ink: Vec::new(), track_author: None, hyphenate: false, protection: None, props: Default::default(), vertical: false, blocks: vec![Block::Para(para)] };
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

#[cfg(test)]
mod vertalign_tests {
    use kumihan::{Align, Block, CharFormat, Document, Paragraph, Run};

    fn doc_with(fmt: CharFormat) -> Document {
        Document {
            font: None,
            page: None,
            sect_raw: None, header: Default::default(), footer: Default::default(), page_color: None, watermark: None, ink: Vec::new(), track_author: None, hyphenate: false, protection: None, props: Default::default(), vertical: false,
            blocks: vec![Block::Para(Paragraph { style: Default::default(), comments: Vec::new(), bookmarks: Vec::new(), dropcap: false,
                align: Align::Left,
                runs: vec![Run { text: "x2".into(), size_pt: 10.5, font: None, fmt }],
                ..Default::default()
            })],
        }
    }

    #[test]
    fn 上付きと蛍光ペンが往復する() {
        let f = CharFormat {
            superscript: true,
            highlight: Some("yellow".into()),
            ..Default::default()
        };
        let mut buf = Vec::new();
        crate::write(&doc_with(f.clone()), std::io::Cursor::new(&mut buf)).unwrap();
        let (back, _) = crate::read(std::io::Cursor::new(&buf)).unwrap();
        let got = &back.paragraphs().next().unwrap().runs[0].fmt;
        assert!(got.superscript, "上付きが消えた");
        assert_eq!(got.highlight.as_deref(), Some("yellow"), "蛍光ペンが消えた");
    }

    #[test]
    fn 下付きも往復する() {
        let f = CharFormat { subscript: true, ..Default::default() };
        let mut buf = Vec::new();
        crate::write(&doc_with(f.clone()), std::io::Cursor::new(&mut buf)).unwrap();
        let (back, _) = crate::read(std::io::Cursor::new(&buf)).unwrap();
        assert!(back.paragraphs().next().unwrap().runs[0].fmt.subscript);
    }
}

#[cfg(test)]
mod sect_tests {
    #[test]
    fn 用紙と余白を読み保存で返す() {
        // sectPr を捨てると、保存で用紙設定とヘッダーの参照が消える
        let xml = r#"<w:document xmlns:w="x"><w:body>
            <w:p><w:r><w:t>本文</w:t></w:r></w:p>
            <w:sectPr><w:headerReference r:id="rId8"/>
              <w:pgSz w:w="16838" w:h="11906" w:orient="landscape"/>
              <w:pgMar w:top="1134" w:right="851" w:bottom="1134" w:left="851"/>
            </w:sectPr></w:body></w:document>"#;
        let (doc, _) = crate::parse_document_xml(xml);
        let pg = doc.page.expect("用紙を読めていない");
        // 16838 twip = 297mm(A4 横)
        assert!((pg.w_mm - 297.0).abs() < 0.5, "幅: {}", pg.w_mm);
        assert!((pg.h_mm - 210.0).abs() < 0.5, "高さ: {}", pg.h_mm);
        assert!((pg.left_mm - 15.0).abs() < 0.5, "左余白: {}", pg.left_mm);
        assert!((pg.top_mm - 20.0).abs() < 0.5, "上余白: {}", pg.top_mm);

        let out = crate::write_document_xml(&doc);
        assert!(out.contains("headerReference"), "ヘッダーの参照が消えた");
        assert!(out.contains("w:pgSz"), "用紙が消えた");
    }

    #[test]
    fn 段組みが読める() {
        let xml = r#"<w:document xmlns:w="x"><w:body><w:p/>
            <w:sectPr><w:pgSz w:w="11906" w:h="16838"/>
              <w:cols w:num="2" w:space="425"/>
            </w:sectPr></w:body></w:document>"#;
        let (doc, _) = crate::parse_document_xml(xml);
        assert_eq!(doc.page.expect("用紙が無い").columns, 2, "段数が読めない");
    }

    #[test]
    fn 用紙の無い文書は既定のまま() {
        let (doc, _) = crate::parse_document_xml(
            r#"<w:document xmlns:w="x"><w:body><w:p/></w:body></w:document>"#);
        assert!(doc.page.is_none());
        assert!(doc.sect_raw.is_none());
    }
}

#[cfg(test)]
mod shade_tests {
    use super::*;
    use kumihan::{Block, Document, Paragraph, Run};

    #[test]
    fn 段落の背景色と囲み枠が往復する() {
        let mut p = Paragraph { style: Default::default(), comments: Vec::new(), bookmarks: Vec::new(), dropcap: false,
            line_spacing: 1.0,
            runs: vec![Run { text: "注意".into(), size_pt: 10.5, font: None,
                             fmt: Default::default() }],
            ..Default::default()
        };
        p.shade = Some("FFF2CC".into());
        p.boxed = true;
        let d = Document { font: None, page: None, sect_raw: None, header: Default::default(), footer: Default::default(), page_color: None, watermark: None, ink: Vec::new(), track_author: None, hyphenate: false, protection: None, props: Default::default(), vertical: false,
                           blocks: vec![Block::Para(p)] };
        let mut buf = Cursor::new(Vec::new());
        write(&d, &mut buf).expect("書けない");
        buf.set_position(0);
        let (back, rep) = read(buf).expect("読めない");
        assert!(rep.is_lossless(), "未対応: {:?}", rep.unsupported);
        let p = back.paragraphs().next().unwrap();
        assert_eq!(p.shade.as_deref(), Some("FFF2CC"), "背景色が往復しない");
        assert!(p.boxed, "囲み枠が往復しない");
    }
}

#[cfg(test)]
mod bookmark_model_tests {
    use super::*;
    use kumihan::{Block, Document};

    #[test]
    fn しおりの名前が往復する() {
        let mut d = Document::plain("表紙\n会社の説明\n終わり", 10.5);
        if let Block::Para(p) = &mut d.blocks[1] {
            p.bookmarks.push("会社名".into());
        }
        let mut buf = Cursor::new(Vec::new());
        write(&d, &mut buf).expect("書けない");
        buf.set_position(0);
        let (back, rep) = read(buf).expect("読めない");
        assert!(rep.is_lossless(), "未対応: {:?}", rep.unsupported);
        let bs: Vec<usize> = back.paragraphs().map(|p| p.bookmarks.len()).collect();
        assert_eq!(bs, vec![0, 1, 0], "しおりの付き先がずれた");
        assert_eq!(back.paragraphs().nth(1).unwrap().bookmarks[0], "会社名");
    }
}

#[cfg(test)]
mod ref_field_round_tests {
    use super::*;
    use kumihan::{Document, RefField};

    #[test]
    fn 相互参照が往復する() {
        let mut d = Document::plain("仕様は3ページを見る", 10.5);
        let s0 = "仕様は".len();
        let e0 = "仕様は3ページ".len();
        d.apply_field(s0..e0, Some(RefField { name: "様式".into(), page: true }));
        let mut buf = Cursor::new(Vec::new());
        write(&d, &mut buf).expect("書けない");
        buf.set_position(0);
        let (back, rep) = read(buf).expect("読めない");
        assert!(rep.is_lossless(), "未対応: {:?}", rep.unsupported);
        assert_eq!(back.body_text(), "仕様は3ページを見る", "見えている値が変わった");
        let f: Vec<_> = back.paragraphs().flat_map(|p| p.runs.iter())
            .filter_map(|r| r.fmt.field.clone().map(|f| (f, r.text.clone())))
            .collect();
        assert_eq!(f.len(), 1, "参照が往復しない");
        assert_eq!(f[0].0, RefField { name: "様式".into(), page: true });
        assert_eq!(f[0].1, "3ページ");
        // XML の上では Word のフィールド
        let out = write_document_xml(&d);
        assert!(out.contains("PAGEREF 様式"), "PAGEREF が無い: {out}");
    }

    #[test]
    fn wordが書く複雑な形の参照も読める() {
        let xml = r#"<w:document xmlns:w="x"><w:body><w:p>
            <w:r><w:t>結果は</w:t></w:r>
            <w:r><w:fldChar w:fldCharType="begin"/></w:r>
            <w:r><w:instrText xml:space="preserve"> REF 結論 \h </w:instrText></w:r>
            <w:r><w:fldChar w:fldCharType="separate"/></w:r>
            <w:r><w:t>別紙のとおり</w:t></w:r>
            <w:r><w:fldChar w:fldCharType="end"/></w:r>
            <w:r><w:t>。</w:t></w:r>
        </w:p></w:body></w:document>"#;
        let (doc, _) = parse_document_xml(xml);
        assert_eq!(doc.body_text(), "結果は別紙のとおり。", "見えている値が繋がらない");
        let f: Vec<_> = doc.paragraphs().flat_map(|p| p.runs.iter())
            .filter(|r| r.fmt.field.is_some()).collect();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].text, "別紙のとおり");
        assert_eq!(f[0].fmt.field.as_ref().unwrap().name, "結論");
    }
}

#[cfg(test)]
mod partial_fmt_tests {
    use super::*;
    use kumihan::Document;

    #[test]
    fn 部分書式が往復する() {
        // 編集モデルが run 粒度になったので、段落の途中だけの太字が
        // docx に3つの run として入り、開き直しても残る
        let mut d = Document::plain("防火戸の仕様を確認", 10.5);
        let s0 = "防火戸の".len();
        let e0 = "防火戸の仕様".len();
        d.apply_char_format(s0..e0, |f| f.bold = true);
        d.apply_size(s0..e0, |_| 14.0);
        let mut buf = Cursor::new(Vec::new());
        write(&d, &mut buf).expect("書けない");
        buf.set_position(0);
        let (back, rep) = read(buf).expect("読めない");
        assert!(rep.is_lossless(), "未対応: {:?}", rep.unsupported);
        let runs: Vec<_> = back.paragraphs().next().unwrap().runs.iter()
            .map(|r| (r.text.clone(), r.fmt.bold, r.size_pt))
            .collect();
        assert_eq!(runs, vec![
            ("防火戸の".into(), false, 10.5),
            ("仕様".into(), true, 14.0),
            ("を確認".into(), false, 10.5),
        ], "部分書式が往復しない");
    }
}

#[cfg(test)]
mod vertical_round_tests {
    use super::*;
    use kumihan::Document;

    #[test]
    fn 縦書きの旗が往復し戻すと消える() {
        let mut d = Document::plain("縦の検査", 10.5);
        d.vertical = true;
        let mut buf = Vec::new();
        write(&d, Cursor::new(&mut buf)).unwrap();
        let (mut back, _) = read(Cursor::new(&buf)).unwrap();
        assert!(back.vertical, "縦書きが往復しない");
        back.vertical = false;
        let mut buf2 = Vec::new();
        write_with(&back, Some(Cursor::new(&buf)), Cursor::new(&mut buf2)).unwrap();
        let (b2, _) = read(Cursor::new(&buf2)).unwrap();
        assert!(!b2.vertical, "横に戻したのに縦のまま");
    }
}

#[cfg(test)]
mod ruby_round_tests {
    use super::*;
    use kumihan::Document;

    #[test]
    fn ルビが往復する() {
        let mut d = Document::plain("組版の話", 10.5);
        d.apply_char_format(0..6, |f| f.ruby = Some("くみはん".into()));
        let mut buf = Vec::new();
        write(&d, Cursor::new(&mut buf)).expect("書けない");
        let xml = {
            let mut z = zip::ZipArchive::new(Cursor::new(&buf)).unwrap();
            let mut s = String::new();
            use std::io::Read as _;
            z.by_name("word/document.xml").unwrap().read_to_string(&mut s).unwrap();
            s
        };
        assert!(xml.contains("<w:ruby>"), "w:ruby が無い");
        assert!(xml.contains("<w:rubyBase>"), "rubyBase が無い");
        let (back, _) = read(Cursor::new(&buf)).expect("読めない");
        assert_eq!(back.body_text(), "組版の話", "本文が変わった");
        let p = back.paragraphs().next().unwrap();
        let r = p.runs.iter().find(|r| r.text == "組版").expect("基底の run が無い");
        assert_eq!(r.fmt.ruby.as_deref(), Some("くみはん"), "読みが往復しない");
        assert!(
            p.runs.iter().any(|r| r.text.contains("の話") && r.fmt.ruby.is_none()),
            "ルビの無い字に読みが付いた"
        );
    }
}

#[cfg(test)]
mod props_round_tests {
    use super::*;
    use kumihan::Document;

    #[test]
    fn 文書の情報が往復し空にすると消える() {
        let mut d = Document::plain("本文", 10.5);
        d.props.creator = "山田 <太郎>".into();
        d.props.title = "検査の書".into();
        let mut first = Vec::new();
        write(&d, Cursor::new(&mut first)).expect("書けない");
        let (mut back, _) = read(Cursor::new(&first)).expect("読めない");
        assert_eq!(back.props.creator, "山田 <太郎>", "作成者が往復しない");
        assert_eq!(back.props.title, "検査の書");
        back.props.title = String::new();
        back.props.keywords = "様式,検査".into();
        let mut second = Vec::new();
        write_with(&back, Some(Cursor::new(&first)), Cursor::new(&mut second)).unwrap();
        let (b2, _) = read(Cursor::new(&second)).unwrap();
        assert_eq!(b2.props.title, "", "空にしたのに残っている");
        assert_eq!(b2.props.keywords, "様式,検査");
        assert_eq!(b2.props.creator, "山田 <太郎>", "触らない欄が消えた");
    }
}

#[cfg(test)]
mod protection_round_tests {
    use super::*;
    use kumihan::Document;

    #[test]
    fn 文書の保護が往復し解除で消える() {
        let mut d = Document::plain("大事な様式", 10.5);
        d.protection = Some("readOnly".into());
        let mut first = Vec::new();
        write(&d, Cursor::new(&mut first)).expect("書けない");
        let (mut back, _) = read(Cursor::new(&first)).expect("読めない");
        assert_eq!(back.protection.as_deref(), Some("readOnly"), "保護が往復しない");
        back.protection = None;
        let mut second = Vec::new();
        write_with(&back, Some(Cursor::new(&first)), Cursor::new(&mut second)).unwrap();
        let (back2, _) = read(Cursor::new(&second)).unwrap();
        assert_eq!(back2.protection, None, "解除したのに残っている");
    }
}

#[cfg(test)]
mod hyphenate_round_tests {
    use super::*;
    use kumihan::Document;

    #[test]
    fn ハイフネーションの旗が往復する() {
        let mut d = Document::plain("hyphenation flag", 10.5);
        d.hyphenate = true;
        let mut buf = Cursor::new(Vec::new());
        write(&d, &mut buf).expect("書けない");
        buf.set_position(0);
        let (mut back, _) = read(buf).expect("読めない");
        assert!(back.hyphenate, "旗が往復しない");
        // 切って保存すれば設定からも消える
        back.hyphenate = false;
        let mut buf2 = Cursor::new(Vec::new());
        write(&back, &mut buf2).expect("書けない");
        buf2.set_position(0);
        let (back2, _) = read(buf2).expect("読めない");
        assert!(!back2.hyphenate, "切ったのに残っている");
    }
}

#[cfg(test)]
mod dropcap_round_tests {
    use super::*;
    use kumihan::{Block, Document};

    #[test]
    fn ドロップキャップが往復する() {
        let mut d = Document::plain("春はあけぼの。やうやう白くなりゆく山際。\n次の段落", 10.5);
        if let Block::Para(p) = &mut d.blocks[0] {
            p.dropcap = true;
        }
        let mut buf = Cursor::new(Vec::new());
        write(&d, &mut buf).expect("書けない");
        buf.set_position(0);
        let (back, rep) = read(buf).expect("読めない");
        assert!(rep.is_lossless(), "未対応: {:?}", rep.unsupported);
        // 枠の段落と本文の段落は、読みで1つに合流する
        let ps: Vec<_> = back.paragraphs().collect();
        assert_eq!(ps.len(), 2, "段落の数が変わった");
        assert!(ps[0].dropcap, "ドロップキャップが消えた");
        let t: String = ps[0].runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(t, "春はあけぼの。やうやう白くなりゆく山際。", "本文が欠けた");
        // 頭の字の大きさは本文と同じに戻る(保存のたびに育たない)
        assert_eq!(ps[0].runs[0].size_pt, 10.5, "頭の字の大きさが育った");
        assert!(!ps[1].dropcap);
        // XML の上では Word の作法(framePr の枠の段落)になっている
        let out = write_document_xml(&d);
        assert!(out.contains(r#"w:dropCap="drop""#), "framePr が無い: {out}");
    }
}

#[cfg(test)]
mod track_write_tests {
    use super::*;
    use kumihan::{Document, TRK_DEL_E, TRK_DEL_S, TRK_INS_E, TRK_INS_S};

    #[test]
    fn 変更履歴の印がinsとdelになる() {
        let mut d = Document::plain(
            &format!("防火{TRK_DEL_S}戸{TRK_DEL_E}{TRK_INS_S}ドア{TRK_INS_E}の仕様"),
            10.5,
        );
        d.track_author = Some("検査".into());
        let out = write_document_xml(&d);
        assert!(out.contains(r#"<w:ins w:id="2" w:author="検査">"#), "w:ins が無い: {out}");
        assert!(out.contains("<w:delText"), "w:delText が無い: {out}");
        assert!(out.contains(r#"<w:del w:id="#), "w:del が無い: {out}");
        // 読み直すと「確定後の姿」(削除は消え、挿入は残る)
        let (back, rep) = parse_document_xml(&out);
        assert_eq!(back.body_text(), "防火ドアの仕様", "確定後の姿にならない");
        assert!(rep.unsupported.iter().any(|(n, _)| n.contains("変更履歴")),
            "履歴があると言っていない: {:?}", rep.unsupported);
    }
}

#[cfg(test)]
mod ink_tests {
    use super::*;
    use kumihan::{Block, Document, Stroke};

    #[test]
    fn ペンの筆が往復する() {
        let mut d = Document::plain("本文", 10.5);
        let st = Stroke {
            page: 2,
            highlighter: false,
            points: vec![(30.0, 40.0), (50.0, 60.5), (70.0, 55.0)],
        };
        let hl = Stroke {
            page: 0,
            highlighter: true,
            points: vec![(20.0, 25.0), (90.0, 25.0)],
        };
        // writer と同じ道: 図形を段落の控えに差し込んでから書く
        if let Block::Para(p) = &mut d.blocks[0] {
            p.anchors.push(ink_anchor_run(&st, 1));
            p.anchors.push(ink_anchor_run(&hl, 2));
        }
        let mut buf = Cursor::new(Vec::new());
        write(&d, &mut buf).expect("書けない");
        buf.set_position(0);
        let (back, _) = read(buf).expect("読めない");
        assert_eq!(back.ink.len(), 2, "筆の数が違う: {}", back.ink.len());
        let b0 = back.ink.iter().find(|s| !s.highlighter).expect("ペンが無い");
        assert_eq!(b0.page, 2, "ページ番号が消えた");
        assert_eq!(b0.points.len(), 3);
        for ((x, y), (wx, wy)) in b0.points.iter().zip(&st.points) {
            assert!((x - wx).abs() < 0.01 && (y - wy).abs() < 0.01,
                "座標がずれた: ({x},{y}) vs ({wx},{wy})");
        }
        assert!(back.ink.iter().any(|s| s.highlighter), "蛍光ペンの区別が消えた");
        // 控えには残っていない(二重にならない)
        assert!(back.paragraphs().next().unwrap().anchors.is_empty(),
            "筆が控えにも残っている");
    }
}

#[cfg(test)]
mod watermark_tests {
    use super::*;
    use kumihan::Document;

    #[test]
    fn 透かしが往復し二重にならない() {
        let mut d = Document::plain("本文", 10.5);
        d.watermark = Some("社外秘".into());
        let mut first = Vec::new();
        write(&d, Cursor::new(&mut first)).expect("書けない");
        let (back, _) = read(Cursor::new(&first)).expect("読めない");
        assert_eq!(back.watermark.as_deref(), Some("社外秘"), "透かしが往復しない");
        // もう一度保存しても、図形は1つのまま
        let mut second = Vec::new();
        write_with(&back, Some(Cursor::new(&first)), Cursor::new(&mut second)).unwrap();
        let mut z = zip::ZipArchive::new(Cursor::new(&second)).unwrap();
        let mut hx = String::new();
        z.by_name("word/johdr1.xml").unwrap().read_to_string(&mut hx).unwrap();
        assert_eq!(hx.matches("v:textpath").count(), 2,
            "図形が二重(shapetype+shape で2つが正: {})",
            hx.matches("v:textpath").count());
        let (back2, _) = read(Cursor::new(&second)).unwrap();
        assert_eq!(back2.watermark.as_deref(), Some("社外秘"));
    }

    #[test]
    fn 透かしを消すと図形も消える() {
        let mut d = Document::plain("本文", 10.5);
        d.watermark = Some("下書き".into());
        let mut first = Vec::new();
        write(&d, Cursor::new(&mut first)).unwrap();
        let (mut back, _) = read(Cursor::new(&first)).unwrap();
        back.watermark = None;
        let mut second = Vec::new();
        write_with(&back, Some(Cursor::new(&first)), Cursor::new(&mut second)).unwrap();
        let (back2, _) = read(Cursor::new(&second)).unwrap();
        assert_eq!(back2.watermark, None, "消したのに残っている");
    }
}

#[cfg(test)]
mod comment_tests {
    use super::*;
    use kumihan::{Block, Comment, Document};

    #[test]
    fn 段落のコメントが往復する() {
        let mut d = Document::plain("一\n二\n三", 10.5);
        if let Block::Para(p) = &mut d.blocks[1] {
            p.comments.push(Comment {
                author: "検査".into(),
                text: "ここは要確認。\n二行目の注記".into(),
            });
        }
        let mut buf = Cursor::new(Vec::new());
        write(&d, &mut buf).expect("書けない");
        buf.set_position(0);
        let (back, rep) = read(buf).expect("読めない");
        assert!(rep.is_lossless(), "未対応: {:?}", rep.unsupported);
        let cs: Vec<usize> = back.paragraphs().map(|p| p.comments.len()).collect();
        assert_eq!(cs, vec![0, 1, 0], "コメントの付き先がずれた");
        let c = &back.paragraphs().nth(1).unwrap().comments[0];
        assert_eq!(c.author, "検査", "書いた人が消えた");
        assert_eq!(c.text, "ここは要確認。\n二行目の注記", "本文が変わった");
    }

    #[test]
    fn 二度保存してもコメントは増えない() {
        let mut d = Document::plain("本文", 10.5);
        if let Block::Para(p) = &mut d.blocks[0] {
            p.comments.push(Comment { author: "私".into(), text: "注記".into() });
        }
        let mut first = Vec::new();
        write(&d, Cursor::new(&mut first)).unwrap();
        let (back, _) = read(Cursor::new(&first)).unwrap();
        let mut second = Vec::new();
        write_with(&back, Some(Cursor::new(&first)), Cursor::new(&mut second)).unwrap();
        let (back2, _) = read(Cursor::new(&second)).unwrap();
        assert_eq!(back2.paragraphs().next().unwrap().comments.len(), 1,
            "保存のたびにコメントが増える");
        let mut z = zip::ZipArchive::new(Cursor::new(&second)).unwrap();
        let mut rels = String::new();
        z.by_name("word/_rels/document.xml.rels").unwrap().read_to_string(&mut rels).unwrap();
        assert_eq!(rels.matches("comments.xml").count(), 1, "関係が二重: {rels}");
    }
}

#[cfg(test)]
mod page_color_tests {
    use super::*;
    use kumihan::Document;

    #[test]
    fn ページの色が往復し設定も付く() {
        let mut d = Document::plain("本文", 10.5);
        d.page_color = Some("E8F1F8".into());
        let mut buf = Cursor::new(Vec::new());
        write(&d, &mut buf).expect("書けない");
        buf.set_position(0);
        let (back, rep) = read(buf).expect("読めない");
        assert!(rep.is_lossless(), "未対応: {:?}", rep.unsupported);
        assert_eq!(back.page_color.as_deref(), Some("E8F1F8"), "色が往復しない");
        // 見せる設定(displayBackgroundShape)が settings に入っている
        let mut buf2 = Cursor::new(Vec::new());
        write(&back, &mut buf2).expect("書けない");
        buf2.set_position(0);
        let mut z = zip::ZipArchive::new(buf2).unwrap();
        let mut st = String::new();
        z.by_name("word/settings.xml").unwrap().read_to_string(&mut st).unwrap();
        assert_eq!(st.matches("displayBackgroundShape").count(), 1,
            "設定が無いか二重: {st}");
    }

    #[test]
    fn 原本の設定は他の項目ごと生きる() {
        // settings を丸ごと作り直すと、原本の設定(既定のタブ幅など)が消える
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let o: zip::write::FileOptions<'_, ()> = Default::default();
        let mut put = |n: &str, d: &[u8]| {
            zip.start_file(n, o).unwrap();
            zip.write_all(d).unwrap();
        };
        put("[Content_Types].xml", br#"<Types xmlns="ct"><Default Extension="xml" ContentType="application/xml"/></Types>"#);
        put("_rels/.rels", br#"<Relationships xmlns="r"/>"#);
        put("word/_rels/document.xml.rels", br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/></Relationships>"#);
        put("word/document.xml", r#"<w:document xmlns:w="x"><w:body><w:p><w:r><w:t>本文</w:t></w:r></w:p></w:body></w:document>"#.as_bytes());
        put("word/settings.xml", r#"<w:settings xmlns:w="x"><w:defaultTabStop w:val="840"/></w:settings>"#.as_bytes());
        let src = zip.finish().unwrap().into_inner();
        let (mut doc, _) = crate::read(Cursor::new(&src)).unwrap();
        doc.page_color = Some("FFF7DC".into());
        let mut out = Vec::new();
        crate::write_with(&doc, Some(Cursor::new(&src)), Cursor::new(&mut out)).unwrap();
        let mut z = zip::ZipArchive::new(Cursor::new(&out)).unwrap();
        let mut st = String::new();
        z.by_name("word/settings.xml").unwrap().read_to_string(&mut st).unwrap();
        assert!(st.contains("defaultTabStop"), "原本の設定が消えた: {st}");
        assert!(st.contains("displayBackgroundShape"), "見せる設定が付かない: {st}");
    }
}

#[cfg(test)]
mod bookmark_tests {
    use super::*;

    #[test]
    fn しおりとコメントの印は保存で残る() {
        // 実物の様式はしおりで記入欄を指すものがある。黙って捨てると
        // 相互参照・コメントの錨が壊れる
        let xml = r#"<w:document xmlns:w="x"><w:body><w:p>
            <w:bookmarkStart w:id="0" w:name="会社名"/>
            <w:r><w:t>日本フネン</w:t></w:r>
            <w:bookmarkEnd w:id="0"/>
            <w:commentRangeStart w:id="3"/>
            <w:r><w:t>要確認の箇所</w:t></w:r>
            <w:commentRangeEnd w:id="3"/>
            <w:r><w:commentReference w:id="3"/></w:r>
        </w:p></w:body></w:document>"#;
        let (doc, rep) = parse_document_xml(xml);
        assert!(rep.unsupported.iter().any(|(n, _)| n.contains("しおり・コメント")),
            "黙って扱った: {:?}", rep.unsupported);
        let out = write_document_xml(&doc);
        assert!(out.contains(r#"w:name="会社名""#), "しおりが消えた: {out}");
        assert!(out.contains("bookmarkEnd"), "しおりの終わりが消えた");
        assert!(out.contains("commentRangeStart"), "コメントの範囲が消えた");
        assert!(out.contains("commentReference"), "コメントの錨が消えた");
        assert!(out.contains("日本フネン") && out.contains("要確認の箇所"), "本文が消えた");
    }
}

#[cfg(test)]
mod list_level_tests {
    use super::*;
    use kumihan::{Block, Document, ListKind};

    #[test]
    fn リストの深さが往復する() {
        let mut d = Document::plain("親\n子", 10.5);
        for (i, ind) in [(0usize, 0u8), (1, 2)] {
            if let Block::Para(p) = &mut d.blocks[i] {
                p.list = ListKind::Bullet;
                p.indent = ind;
            }
        }
        let mut buf = Cursor::new(Vec::new());
        write(&d, &mut buf).expect("書けない");
        buf.set_position(0);
        let (back, _) = read(buf).expect("読めない");
        let inds: Vec<u8> = back.paragraphs().map(|p| p.indent).collect();
        assert_eq!(inds, vec![0, 2], "深さが往復しない");
    }

    #[test]
    fn indの無いリストはilvlが深さになる() {
        // Word のリストは w:ind を numbering.xml に置くので、段落側は ilvl だけのことが多い
        let xml = r#"<w:document xmlns:w="x"><w:body><w:p><w:pPr>
            <w:numPr><w:ilvl w:val="1"/><w:numId w:val="1"/></w:numPr>
            </w:pPr><w:r><w:t>子の項目</w:t></w:r></w:p></w:body></w:document>"#;
        let (doc, _) = parse_document_xml(xml);
        let p = doc.paragraphs().next().unwrap();
        assert_eq!(p.list, ListKind::Bullet);
        assert_eq!(p.indent, 1, "ilvl が深さに入らない");
    }

    #[test]
    fn タブが往復する() {
        // w:t の中に生のタブを書くと Word が潰す。要素(w:tab)で書く
        let d = Document::plain("項目\t値", 10.5);
        let mut buf = Cursor::new(Vec::new());
        write(&d, &mut buf).expect("書けない");
        buf.set_position(0);
        let (back, _) = read(buf).expect("読めない");
        assert_eq!(back.body_text(), "項目\t値", "タブが消えた");
        // XML の上でも w:tab 要素になっている
        let mut buf2 = Cursor::new(Vec::new());
        write(&back, &mut buf2).expect("書けない");
        buf2.set_position(0);
        let mut z = zip::ZipArchive::new(buf2).unwrap();
        let mut s = String::new();
        z.by_name("word/document.xml").unwrap().read_to_string(&mut s).unwrap();
        assert!(s.contains("<w:tab/>"), "タブが要素になっていない");
        assert!(!s.contains("項目\t"), "w:t に生のタブが残った");
    }
}

#[cfg(test)]
mod style_tests {
    use super::*;
    use kumihan::{Block, Document, ParaStyle};

    #[test]
    fn 見出しと目次の行が往復する() {
        let mut d = Document::plain("表題\n本文\n目次の行", 10.5);
        if let Block::Para(p) = &mut d.blocks[0] { p.style = ParaStyle::Heading(1); }
        if let Block::Para(p) = &mut d.blocks[2] { p.style = ParaStyle::Toc(2); }
        let mut buf = Cursor::new(Vec::new());
        write(&d, &mut buf).expect("書けない");
        buf.set_position(0);
        let (back, rep) = read(buf).expect("読めない");
        assert!(rep.is_lossless(), "未対応: {:?}", rep.unsupported);
        let ps: Vec<ParaStyle> = back.paragraphs().map(|p| p.style).collect();
        assert_eq!(ps, vec![ParaStyle::Heading(1), ParaStyle::Body, ParaStyle::Toc(2)],
            "段落の役割が往復しない");
    }

    #[test]
    fn 日本語版wordの見出しも読める() {
        // 日本語版 Word の見出し1は style id が「1」。outlineLvl だけでも見出し
        let xml = r#"<w:document xmlns:w="x"><w:body>
            <w:p><w:pPr><w:pStyle w:val="1"/></w:pPr><w:r><w:t>甲</w:t></w:r></w:p>
            <w:p><w:pPr><w:outlineLvl w:val="1"/></w:pPr><w:r><w:t>乙</w:t></w:r></w:p>
            <w:p><w:pPr><w:pStyle w:val="af0"/></w:pPr><w:r><w:t>丙</w:t></w:r></w:p>
        </w:body></w:document>"#;
        let (doc, _) = parse_document_xml(xml);
        let ps: Vec<ParaStyle> = doc.paragraphs().map(|p| p.style).collect();
        assert_eq!(ps, vec![ParaStyle::Heading(1), ParaStyle::Heading(2), ParaStyle::Body]);
    }
}

#[cfg(test)]
mod hf_tests {
    use super::*;
    use kumihan::{Align, Document, Paragraph, Run, PAGE_MARK};

    fn para(s: &str) -> Paragraph {
        Paragraph { style: Default::default(), comments: Vec::new(), bookmarks: Vec::new(), dropcap: false,
            line_spacing: 1.0,
            runs: vec![Run { text: s.into(), size_pt: 10.5, font: None,
                             fmt: Default::default() }],
            ..Default::default()
        }
    }

    #[test]
    fn ヘッダーとフッターが往復する() {
        let mut d = Document::plain("本文", 10.5);
        d.header.paragraphs = vec![para("社外秘")];
        let mut f = para(&format!("- {PAGE_MARK} -"));
        f.align = Align::Center;
        d.footer.paragraphs = vec![f];
        let mut buf = Cursor::new(Vec::new());
        write(&d, &mut buf).expect("書けない");
        buf.set_position(0);
        let (back, rep) = read(buf).expect("読めない");
        assert!(rep.is_lossless(), "未対応: {:?}", rep.unsupported);
        assert_eq!(kumihan::paras_text(&back.header.paragraphs), "社外秘",
            "ヘッダーが往復しない");
        let ftxt = kumihan::paras_text(&back.footer.paragraphs);
        assert!(ftxt.contains(PAGE_MARK), "ページ番号の印が消えた: {ftxt:?}");
        assert_eq!(back.footer.paragraphs[0].align, Align::Center,
            "フッターの揃えが消えた");
        assert_eq!(back.header.part.as_deref(), Some("word/johdr1.xml"));
        assert_eq!(texts_of(&back), vec!["本文"], "本文が変わった");
    }

    fn texts_of(d: &Document) -> Vec<String> {
        d.paragraphs()
            .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect())
            .collect()
    }

    #[test]
    fn 複雑なフィールドのページ番号も読める() {
        // Word は PAGE を fldChar(begin/instrText/separate/計算済み/end)で書く
        let xml = r#"<w:hdr xmlns:w="x"><w:p>
            <w:r><w:fldChar w:fldCharType="begin"/></w:r>
            <w:r><w:instrText xml:space="preserve"> PAGE </w:instrText></w:r>
            <w:r><w:fldChar w:fldCharType="separate"/></w:r>
            <w:r><w:t>7</w:t></w:r>
            <w:r><w:fldChar w:fldCharType="end"/></w:r>
        </w:p></w:hdr>"#;
        let (doc, _) = parse_document_xml(xml);
        let t = doc.body_text();
        assert!(!t.contains('7'), "計算済みの見た目(7)が本文へ漏れた: {t:?}");
        assert!(t.contains(PAGE_MARK), "PAGE が印にならない: {t:?}");
    }

    #[test]
    fn 持てないフィールドは報告して落とす() {
        let xml = r#"<w:document xmlns:w="x"><w:body><w:p>
            <w:fldSimple w:instr=" DATE "><w:r><w:t>2026/08/03</w:t></w:r></w:fldSimple>
        </w:p></w:body></w:document>"#;
        let (doc, rep) = parse_document_xml(xml);
        assert!(!doc.body_text().contains("2026"), "計算済みの見た目が漏れた");
        assert!(rep.unsupported.iter().any(|(n, _)| n.contains("フィールド")),
            "黙って落とした: {:?}", rep.unsupported);
    }

    #[test]
    fn ページ数の印が往復する() {
        use kumihan::PAGES_MARK;
        let mut d = Document::plain("本文", 10.5);
        d.footer.paragraphs = vec![para(&format!("{PAGE_MARK} / {PAGES_MARK}"))];
        let mut buf = Cursor::new(Vec::new());
        write(&d, &mut buf).expect("書けない");
        buf.set_position(0);
        let (back, rep) = read(buf).expect("読めない");
        assert!(rep.is_lossless(), "未対応: {:?}", rep.unsupported);
        let t = kumihan::paras_text(&back.footer.paragraphs);
        assert!(t.contains(PAGE_MARK) && t.contains(PAGES_MARK),
            "印が往復しない: {t:?}");
    }

    /// 原本に header1.xml を持つ最小の docx
    fn docx_with_header(header_xml: &[u8]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let o: zip::write::FileOptions<'_, ()> = Default::default();
        let mut put = |n: &str, d: &[u8]| {
            zip.start_file(n, o).unwrap();
            zip.write_all(d).unwrap();
        };
        put("[Content_Types].xml", br#"<Types xmlns="ct"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/></Types>"#);
        put("_rels/.rels", br#"<Relationships xmlns="r"/>"#);
        put("word/_rels/document.xml.rels", br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/></Relationships>"#);
        put("word/document.xml", r#"<w:document xmlns:w="x" xmlns:r="y"><w:body><w:p><w:r><w:t>本文</w:t></w:r></w:p><w:sectPr><w:headerReference w:type="default" r:id="rId9"/><w:pgSz w:w="11906" w:h="16838"/></w:sectPr></w:body></w:document>"#.as_bytes());
        put("word/header1.xml", header_xml);
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn 原本のヘッダー部品へ書き戻す() {
        let src = docx_with_header(
            r#"<w:hdr xmlns:w="x"><w:p><w:r><w:t>旧いヘッダー</w:t></w:r></w:p></w:hdr>"#.as_bytes());
        let (mut doc, _) = crate::read(Cursor::new(&src)).unwrap();
        assert_eq!(doc.header.part.as_deref(), Some("word/header1.xml"));
        assert_eq!(kumihan::paras_text(&doc.header.paragraphs), "旧いヘッダー");
        kumihan::set_paras_text(&mut doc.header.paragraphs, "新しいヘッダー", 10.5);
        let mut out = Vec::new();
        crate::write_with(&doc, Some(Cursor::new(&src)), Cursor::new(&mut out)).unwrap();
        let z = zip::ZipArchive::new(Cursor::new(&out)).unwrap();
        let names: Vec<String> = {
            let mut z2 = zip::ZipArchive::new(Cursor::new(&out)).unwrap();
            (0..z2.len()).map(|i| z2.by_index(i).unwrap().name().to_string()).collect()
        };
        drop(z);
        assert_eq!(names.iter().filter(|n| *n == "word/header1.xml").count(), 1,
            "部品が二重になった: {names:?}");
        let mut z = zip::ZipArchive::new(Cursor::new(&out)).unwrap();
        let mut s = String::new();
        z.by_name("word/header1.xml").unwrap().read_to_string(&mut s).unwrap();
        assert!(s.contains("新しいヘッダー"), "部品に書き戻っていない: {s}");
        let mut rels = String::new();
        z.by_name("word/_rels/document.xml.rels").unwrap().read_to_string(&mut rels).unwrap();
        assert!(!rels.contains("rIdJOhdr"), "原本の参照に重ねて足した: {rels}");
        let (back, _) = crate::read(Cursor::new(&out)).unwrap();
        assert_eq!(kumihan::paras_text(&back.header.paragraphs), "新しいヘッダー",
            "読み直すと消えている");
    }

    #[test]
    fn 表のあるヘッダーは触らず持ち越す() {
        // まだ持てないもの(表)が入った部品は編集の対象にせず、原文のまま生かす
        let orig = r#"<w:hdr xmlns:w="x"><w:tbl><w:tr><w:tc><w:p><w:r><w:t>枠</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:hdr>"#.as_bytes();
        let src = docx_with_header(orig);
        let (doc, rep) = crate::read(Cursor::new(&src)).unwrap();
        assert!(doc.header.paragraphs.is_empty(), "編集できない部品を編集の対象にした");
        assert!(rep.unsupported.iter().any(|(n, _)| n.contains("ヘッダーの表")),
            "黙って隠した: {:?}", rep.unsupported);
        let mut out = Vec::new();
        crate::write_with(&doc, Some(Cursor::new(&src)), Cursor::new(&mut out)).unwrap();
        let mut z = zip::ZipArchive::new(Cursor::new(&out)).unwrap();
        let mut s = String::new();
        z.by_name("word/header1.xml").unwrap().read_to_string(&mut s).unwrap();
        assert_eq!(s.as_bytes(), orig, "触っていない部品が変わった");
    }

    #[test]
    fn 二度保存しても参照と部品が二重にならない() {
        let mut d = Document::plain("本文", 10.5);
        d.footer.paragraphs = vec![para(&PAGE_MARK.to_string())];
        let mut first = Vec::new();
        crate::write(&d, Cursor::new(&mut first)).unwrap();
        // 開き直さず、同じモデルからもう一度保存(アプリの上書き保存と同じ形)
        let mut second = Vec::new();
        crate::write_with(&d, Some(Cursor::new(&first)), Cursor::new(&mut second)).unwrap();
        let mut z = zip::ZipArchive::new(Cursor::new(&second)).unwrap();
        let mut rels = String::new();
        z.by_name("word/_rels/document.xml.rels").unwrap().read_to_string(&mut rels).unwrap();
        assert_eq!(rels.matches("rIdJOftr").count(), 1, "関係が二重: {rels}");
        let (back, _) = crate::read(Cursor::new(&second)).unwrap();
        assert!(kumihan::paras_text(&back.footer.paragraphs).contains(PAGE_MARK));
    }
}

#[cfg(test)]
mod image_insert_tests {
    use super::*;
    use kumihan::{Block, Document, InlineImage, Paragraph, Run};

    fn png_bytes() -> Vec<u8> {
        // 中身は問わない(読み書きは実体を素通しする)。頭のPNG印だけ本物
        let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        v.extend_from_slice(&[1, 2, 3, 4, 5]);
        v
    }

    #[test]
    fn 挿した画像が部品ごと保存され読み直せる() {
        let mut p = Paragraph { style: Default::default(), comments: Vec::new(), bookmarks: Vec::new(), dropcap: false,
            line_spacing: 1.0,
            runs: vec![Run { text: "ロゴの下".into(), size_pt: 10.5, font: None,
                             fmt: Default::default() }],
            ..Default::default()
        };
        p.images_new.push(InlineImage {
            bytes: std::sync::Arc::new(png_bytes()),
            w_mm: 50.0,
            h_mm: 30.0,
        });
        let d = Document { font: None, page: None, sect_raw: None, header: Default::default(), footer: Default::default(), page_color: None, watermark: None, ink: Vec::new(), track_author: None, hyphenate: false, protection: None, props: Default::default(), vertical: false,
                           blocks: vec![Block::Para(p)] };
        let mut buf = Cursor::new(Vec::new());
        write(&d, &mut buf).expect("書けない");
        let first = buf.into_inner();

        let (back, _) = read(Cursor::new(first.clone())).expect("読めない");
        let bp = back.paragraphs().next().unwrap();
        assert_eq!(bp.images.len(), 1, "挿した画像が読み直せない");
        assert_eq!(*bp.images[0].bytes, png_bytes(), "画像の実体が変わった");
        assert!((bp.images[0].w_mm - 50.0).abs() < 0.5, "大きさが変わった");
        assert!(bp.images_new.is_empty(), "読み直しで二重の持ち場に入った");

        // アプリの保存と同じ形(原本を渡す)でもう一往復しても、壊れず残る
        let mut buf2 = Cursor::new(Vec::new());
        write_with(&back, Some(Cursor::new(&first)), &mut buf2).expect("書けない");
        buf2.set_position(0);
        let (back2, _) = read(buf2).expect("読み直せない");
        assert_eq!(back2.paragraphs().next().unwrap().images.len(), 1,
            "二度目の保存で画像が消えた");
    }
}

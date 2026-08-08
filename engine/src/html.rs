//! HTML(部分集合)→ 文書モデル。「writer の HTML — JS なしの閲覧と記入」の
//! 読み側(SEKKEI の同名の節)。対象は閉域の業務 Web — 公開 Web を何でも
//! 読む約束はしない。**JS は実行しない**(script は丸ごと読み飛ばす)。
//! 理解しない要素は notes に返す — 黙って落とさない。
//! p / li / tr / td の閉じ忘れは癒す(実物の Web によくある形)。

use crate::{
    Block, Cellbox, CharFormat, Document, ListKind, Paragraph, ParaStyle, Run, Table,
};

/// 記入欄(HTML の form)。writer がパネルで記入し、GET/POST で送る
#[derive(Debug, Clone, Default)]
pub struct Form {
    pub action: String,
    pub method: String, // "get" / "post"
    pub fields: Vec<Field>,
}

#[derive(Debug, Clone, Default)]
pub struct Field {
    pub name: String,
    pub value: String,
    pub hidden: bool,
    /// select の選択肢(空なら自由記入)
    pub options: Vec<String>,
}

/// HTML を文書モデルへ写す。返り値は (文書, 帳簿)。
pub fn parse(src: &str, size_pt: f32) -> (Document, Vec<String>) {
    let (d, n, _, _) = parse_full(src, size_pt);
    (d, n)
}

/// 記入欄(form)とリンクも返す版。リンクは (href, 見えている字)。
pub fn parse_full(
    src: &str,
    size_pt: f32,
) -> (Document, Vec<String>, Vec<Form>, Vec<(String, String)>) {
    let mut b = Builder::new(size_pt);
    let bytes = src.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            // コメントと DOCTYPE
            if src[i..].starts_with("<!--") {
                i = src[i..].find("-->").map(|j| i + j + 3).unwrap_or(bytes.len());
                continue;
            }
            if src[i..].starts_with("<!") {
                i = src[i..].find('>').map(|j| i + j + 1).unwrap_or(bytes.len());
                continue;
            }
            let end = match src[i..].find('>') {
                Some(j) => i + j,
                None => break, // 閉じない '<' — 残りは捨てる(壊れた HTML)
            };
            let tag_src = &src[i + 1..end];
            i = end + 1;
            let closing = tag_src.starts_with('/');
            let name: String = tag_src
                .trim_start_matches('/')
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase();
            if name.is_empty() {
                continue;
            }
            // script / style / head の中は読まない(title だけ拾う)
            if let Some(until) = &b.skip {
                if closing && name == *until {
                    b.skip = None;
                }
                continue;
            }
            if closing { b.close(&name) } else { b.open(&name, tag_src) }
        } else {
            let end = src[i..].find('<').map(|j| i + j).unwrap_or(bytes.len());
            b.text(&src[i..end]);
            i = end;
        }
    }
    b.finish()
}

struct Builder {
    size_pt: f32,
    doc: Document,
    runs: Vec<Run>,
    cur: String,
    bold: i32,
    italic: i32,
    under: i32,
    style: ParaStyle,
    list: ListKind,
    depth: u8,
    // 表。入れ子は初版では畳む(帳簿に言う)
    table: Option<Vec<Vec<Cellbox>>>,
    row: Vec<Cellbox>,
    cell: Option<Vec<Run>>,
    in_title: bool,
    /// HTML の ruby(基底, 読み, いま rt の中か)。うちのルビへ写す
    rt: Option<(String, String, bool)>,
    skip: Option<String>,
    notes: std::collections::BTreeMap<String, usize>,
    forms: Vec<Form>,
    links: Vec<(String, String)>,
    cur_link: Option<(String, String)>,
    cur_form: Option<Form>,
    /// select / textarea の中身を拾う先
    sel: Option<Field>,
    in_option: bool,
    ta: Option<Field>,
}

impl Builder {
    fn new(size_pt: f32) -> Builder {
        Builder {
            size_pt,
            doc: Document::plain("", size_pt),
            runs: Vec::new(),
            cur: String::new(),
            bold: 0,
            italic: 0,
            under: 0,
            style: ParaStyle::Body,
            list: ListKind::None,
            depth: 0,
            table: None,
            row: Vec::new(),
            cell: None,
            in_title: false,
            rt: None,
            skip: None,
            notes: Default::default(),
            forms: Vec::new(),
            links: Vec::new(),
            cur_link: None,
            cur_form: None,
            sel: None,
            in_option: false,
            ta: None,
        }
    }

    fn note(&mut self, what: &str) {
        *self.notes.entry(what.to_string()).or_insert(0) += 1;
    }

    fn fmt(&self) -> CharFormat {
        CharFormat {
            bold: self.bold > 0,
            italic: self.italic > 0,
            underline: self.under > 0,
            ..Default::default()
        }
    }

    /// 溜めた字を run に確定する(書式が変わる・段落が終わる前に呼ぶ)
    fn flush_text(&mut self) {
        if self.cur.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.cur);
        let run = Run {
            text,
            size_pt: self.size_pt,
            font: None,
            fmt: self.fmt(),
        };
        match &mut self.cell {
            Some(runs) => runs.push(run),
            None => self.runs.push(run),
        }
    }

    /// 段落を確定する(p の閉じ・次のブロック要素の頭)
    fn flush_para(&mut self) {
        self.flush_text();
        let runs = std::mem::take(&mut self.runs);
        if runs.iter().all(|r| r.text.trim().is_empty()) {
            return;
        }
        self.doc.blocks.push(Block::Para(Paragraph {
            style: self.style,
            list: self.list,
            indent: self.depth.saturating_sub(1),
            line_spacing: 1.0,
            runs,
            ..Default::default()
        }));
        self.style = ParaStyle::Body;
    }

    fn end_cell(&mut self) {
        self.flush_text();
        if let Some(runs) = self.cell.take() {
            self.row.push(Cellbox {
                paragraphs: vec![Paragraph {
                    line_spacing: 1.0,
                    runs,
                    ..Default::default()
                }],
                ..Default::default()
            });
        }
    }

    fn end_row(&mut self) {
        self.end_cell();
        if !self.row.is_empty() {
            if let Some(rows) = &mut self.table {
                rows.push(std::mem::take(&mut self.row));
            }
        }
    }

    fn open(&mut self, name: &str, tag: &str) {
        match name {
            "script" | "style" => self.skip = Some(name.to_string()),
            "title" => self.in_title = true,
            "p" | "div" | "section" | "article" | "header" | "footer" => self.flush_para(),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.flush_para();
                let n: u8 = name[1..].parse().unwrap_or(1);
                self.style = ParaStyle::Heading(n.min(4));
            }
            "br" => self.cur.push('\n'),
            "hr" => {
                self.flush_para();
                self.cur.push_str("――――――――");
                self.flush_para();
            }
            "b" | "strong" | "th" | "summary" => {
                self.flush_text();
                self.bold += 1;
                if name == "th" {
                    self.end_cell();
                    self.cell = Some(Vec::new());
                }
                if name == "summary" {
                    self.flush_para();
                }
            }
            "i" | "em" => {
                self.flush_text();
                self.italic += 1;
            }
            "s" | "strike" | "del" => self.flush_text(),
            "blockquote" | "figure" | "figcaption" => self.flush_para(),
            // HTML の ruby はうちのルビ(CharFormat.ruby)へ写す
            "ruby" => {
                self.flush_text();
                self.rt = Some((String::new(), String::new(), false));
            }
            "rt" => {
                if let Some(st) = &mut self.rt {
                    st.0 = std::mem::take(&mut self.cur);
                    st.2 = true;
                }
            }
            "u" | "a" => {
                self.flush_text();
                self.under += 1;
                if name == "a" {
                    if let Some(href) = attr_of(tag, "href") {
                        if !href.starts_with('#') && !href.is_empty() {
                            self.cur_link = Some((href, String::new()));
                        }
                    }
                }
            }
            "ul" | "ol" => {
                self.flush_para();
                self.depth = (self.depth + 1).min(8);
                self.list = if name == "ol" { ListKind::Number } else { ListKind::Bullet };
            }
            "li" => self.flush_para(),
            "table" => {
                self.flush_para();
                if self.table.is_some() {
                    // 入れ子の表は初版では畳む(外の表のセルの字になる)
                    self.note("入れ子の表(外の表に畳んだ)");
                } else {
                    self.table = Some(Vec::new());
                }
            }
            "tr" => self.end_row(),
            "td" => {
                self.end_cell();
                self.cell = Some(Vec::new());
            }
            "img" => self.note("画像(img。初版では出さない)"),
            // 記入(フォーム)。欄は下線の空欄として見せ、中身は Form に集める
            "form" => {
                self.cur_form = Some(Form {
                    action: attr_of(tag, "action").unwrap_or_default(),
                    method: attr_of(tag, "method")
                        .unwrap_or_else(|| "get".into())
                        .to_ascii_lowercase(),
                    fields: Vec::new(),
                });
            }
            "input" => {
                let ty = attr_of(tag, "type").unwrap_or_default();
                if ty == "submit" || ty == "button" {
                    return;
                }
                let f = Field {
                    name: attr_of(tag, "name").unwrap_or_default(),
                    value: attr_of(tag, "value").unwrap_or_default(),
                    hidden: ty == "hidden",
                    options: Vec::new(),
                };
                if !f.hidden {
                    self.flush_text();
                    self.under += 1;
                    self.cur.push_str(if f.value.is_empty() { "　　　　" } else { &f.value });
                    self.flush_text();
                    self.under -= 1;
                }
                if let Some(fm) = &mut self.cur_form {
                    if !f.name.is_empty() {
                        fm.fields.push(f);
                    }
                }
            }
            "select" => {
                self.sel = Some(Field {
                    name: attr_of(tag, "name").unwrap_or_default(),
                    ..Default::default()
                });
            }
            "option" => {
                if let Some(f) = &mut self.sel {
                    f.options.push(String::new());
                    self.in_option = true;
                }
            }
            "textarea" => {
                self.ta = Some(Field {
                    name: attr_of(tag, "name").unwrap_or_default(),
                    ..Default::default()
                });
            }
            "button" => {}
            "pre" | "code" | "span" | "small" | "label" | "details" | "tbody"
            | "thead" | "html" | "body" | "meta" | "link" | "head" | "option" => {}
            other => {
                let _ = tag;
                self.note(&format!("知らない要素({other})"));
            }
        }
    }

    fn close(&mut self, name: &str) {
        match name {
            "title" => self.in_title = false,
            "p" | "li" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            | "section" | "article" | "summary" | "blockquote" | "figure"
            | "figcaption" => self.flush_para(),
            "b" | "strong" => {
                self.flush_text();
                self.bold = (self.bold - 1).max(0);
            }
            "th" => {
                self.flush_text();
                self.bold = (self.bold - 1).max(0);
                self.end_cell();
            }
            "i" | "em" => {
                self.flush_text();
                self.italic = (self.italic - 1).max(0);
            }
            "u" | "a" => {
                self.flush_text();
                self.under = (self.under - 1).max(0);
                if name == "a" {
                    if let Some((href, text)) = self.cur_link.take() {
                        let t = if text.trim().is_empty() {
                            href.clone()
                        } else {
                            text.trim().chars().take(60).collect()
                        };
                        self.links.push((href, t));
                    }
                }
            }
            "ul" | "ol" => {
                self.flush_para();
                self.depth = self.depth.saturating_sub(1);
                if self.depth == 0 {
                    self.list = ListKind::None;
                }
            }
            "rt" => {
                if let Some(st) = &mut self.rt {
                    st.2 = false;
                }
            }
            "ruby" => {
                if let Some((base, rt, _)) = self.rt.take() {
                    let base = if base.is_empty() {
                        std::mem::take(&mut self.cur)
                    } else {
                        base
                    };
                    if !base.is_empty() {
                        let mut fmt = self.fmt();
                        fmt.ruby = (!rt.is_empty()).then_some(rt);
                        let run = Run {
                            text: base,
                            size_pt: self.size_pt,
                            font: None,
                            fmt,
                        };
                        match &mut self.cell {
                            Some(rs) => rs.push(run),
                            None => self.runs.push(run),
                        }
                    }
                }
            }
            "form" => {
                if let Some(fm) = self.cur_form.take() {
                    if !fm.fields.is_empty() {
                        self.forms.push(fm);
                    }
                }
            }
            "option" => self.in_option = false,
            "select" => {
                if let Some(mut f) = self.sel.take() {
                    self.in_option = false;
                    f.value = f.options.first().cloned().unwrap_or_default();
                    self.flush_text();
                    self.under += 1;
                    self.cur.push_str(if f.value.is_empty() { "　　　　" } else { &f.value });
                    self.flush_text();
                    self.under -= 1;
                    if let Some(fm) = &mut self.cur_form {
                        if !f.name.is_empty() {
                            fm.fields.push(f);
                        }
                    }
                }
            }
            "textarea" => {
                if let Some(f) = self.ta.take() {
                    self.flush_text();
                    self.under += 1;
                    self.cur.push_str(if f.value.is_empty() { "　　　　" } else { &f.value });
                    self.flush_text();
                    self.under -= 1;
                    if let Some(fm) = &mut self.cur_form {
                        if !f.name.is_empty() {
                            fm.fields.push(f);
                        }
                    }
                }
            }
            "td" => self.end_cell(),
            "tr" => self.end_row(),
            "table" => {
                self.end_row();
                if let Some(rows) = self.table.take() {
                    if !rows.is_empty() {
                        self.doc.blocks.push(Block::Table(Table {
                            col_mm: Vec::new(),
                            rows,
                        }));
                    }
                }
            }
            _ => {}
        }
    }

    fn text(&mut self, raw: &str) {
        if self.skip.is_some() {
            return;
        }
        let decoded = decode_entities(raw);
        if self.in_option {
            if let Some(f) = &mut self.sel {
                if let Some(o) = f.options.last_mut() {
                    o.push_str(decoded.trim());
                }
            }
            return;
        }
        if let Some(f) = &mut self.ta {
            f.value.push_str(&decoded);
            return;
        }
        if let Some(st) = &mut self.rt {
            if st.2 {
                st.1.push_str(decoded.trim());
                return;
            }
        }
        if let Some((_, t)) = &mut self.cur_link {
            t.push_str(&decoded);
        }
        if self.in_title {
            self.doc.props.title.push_str(decoded.trim());
            return;
        }
        // 空白は畳む(HTML の流儀)。段落の頭では入れない
        for ch in decoded.chars() {
            if ch.is_whitespace() && ch != '\n' || ch == '\n' {
                let target_empty = self.cur.is_empty()
                    && match &self.cell {
                        Some(rs) => rs.is_empty(),
                        None => self.runs.is_empty(),
                    };
                if !target_empty && !self.cur.ends_with(' ') {
                    self.cur.push(' ');
                }
            } else {
                self.cur.push(ch);
            }
        }
    }

    fn finish(mut self) -> (Document, Vec<String>, Vec<Form>, Vec<(String, String)>) {
        self.close("ruby");
        self.close("form");
        self.close("table");
        self.flush_para();
        // Document::plain("") の空段落が先頭に残っていたら外す
        if self.doc.blocks.len() > 1 {
            if let Some(Block::Para(p)) = self.doc.blocks.first() {
                if p.runs.iter().all(|r| r.text.is_empty()) {
                    self.doc.blocks.remove(0);
                }
            }
        }
        let notes = self
            .notes
            .into_iter()
            .map(|(k, n)| if n > 1 { format!("{k} × {n}") } else { k })
            .collect();
        (self.doc, notes, self.forms, self.links)
    }
}

/// 開始タグから属性を取り出す(部分集合。` key="値"` / ` key='値'` / ` key=値`)
fn attr_of(tag: &str, key: &str) -> Option<String> {
    let pat = format!(" {key}=");
    let i = tag.find(&pat)? + pat.len();
    let rest = &tag[i..];
    let (q, rest) = match rest.chars().next()? {
        '"' => ('"', &rest[1..]),
        '\'' => ('\'', &rest[1..]),
        _ => (' ', rest),
    };
    let end = rest.find(q).unwrap_or(rest.len());
    Some(decode_entities(rest[..end].trim_end_matches('>')))
}

/// 文字実体参照(最小限)と数値参照を戻す
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.char_indices();
    while let Some((i, ch)) = it.next() {
        if ch != '&' {
            out.push(ch);
            continue;
        }
        let rest = &s[i + 1..];
        let Some(semi) = rest.find(';').filter(|j| *j <= 10) else {
            out.push('&');
            continue;
        };
        let ent = &rest[..semi];
        let rep = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => ent
                .strip_prefix("#x")
                .or_else(|| ent.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match rep {
            Some(c) => {
                out.push(c);
                for _ in 0..semi + 1 {
                    it.next();
                }
            }
            None => out.push('&'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn フォームの欄が集まる() {
        let (_, _, forms, _) = parse_full(
            "<form action=\"/order\" method=\"post\">             <input type=\"text\" name=\"品名\" value=\"鉛筆\">             <select name=\"数\"><option>1</option><option>2</option></select>             <textarea name=\"備考\">急ぎ</textarea>             <input type=\"submit\" value=\"送る\"></form>",
            10.5,
        );
        assert_eq!(forms.len(), 1);
        let f = &forms[0];
        assert_eq!((f.action.as_str(), f.method.as_str()), ("/order", "post"));
        assert_eq!(f.fields.len(), 3);
        assert_eq!(f.fields[0].value, "鉛筆");
        assert_eq!(f.fields[1].options, vec!["1", "2"]);
        assert_eq!(f.fields[2].value, "急ぎ");
    }

    #[test]
    fn htmlのルビがうちのルビへ写る() {
        let (d, _) = parse("<p><ruby>組版<rt>くみはん</rt></ruby>の話</p>", 10.5);
        let p = d.paragraphs().next().unwrap();
        let r = p.runs.iter().find(|r| r.text == "組版").expect("基底が無い");
        assert_eq!(r.fmt.ruby.as_deref(), Some("くみはん"));
        assert!(p.runs.iter().any(|r| r.text.contains("の話") && r.fmt.ruby.is_none()));
    }

    #[test]
    fn 見出しと段落と書式が写る() {
        let (d, _) = parse(
            "<html><head><title>題</title></head><body>\
             <h1>見出し</h1><p>本文の<b>太字&amp;</b>続き</p></body></html>",
            10.5,
        );
        assert_eq!(d.props.title, "題");
        let ps: Vec<_> = d.paragraphs().collect();
        assert_eq!(ps[0].style, ParaStyle::Heading(1));
        assert_eq!(ps[0].runs[0].text, "見出し");
        let p = ps[1];
        assert_eq!(p.runs.iter().map(|r| r.text.as_str()).collect::<String>(),
                   "本文の太字&続き");
        assert!(p.runs.iter().any(|r| r.fmt.bold && r.text == "太字&"));
    }

    #[test]
    fn 閉じ忘れの表が癒えて同じ形になる() {
        let good = "<table><tr><th>a</th><th>b</th></tr>\
                    <tr><td>1</td><td>2</td></tr></table>";
        let lazy = "<table><tr><th>a<th>b<tr><td>1<td>2</table>";
        for src in [good, lazy] {
            let (d, _) = parse(src, 10.5);
            let t = d.tables().next().expect("表が無い");
            assert_eq!(t.rows.len(), 2, "行の数: {src}");
            assert_eq!(t.rows[0].len(), 2, "列の数: {src}");
            assert_eq!(t.rows[1][1].paragraphs[0].runs[0].text, "2");
        }
    }

    #[test]
    fn scriptは実行も表示もしないで帳簿に残らず消える() {
        let (d, _) = parse(
            "<p>前</p><script>alert('x')</script><p>後</p>",
            10.5,
        );
        assert_eq!(d.body_text(), "前\n後");
    }

    #[test]
    fn 箇条書きと帳簿() {
        let (d, notes) = parse(
            "<ul><li>一</li><li>二</li></ul><img src=x><blink>謎</blink>",
            10.5,
        );
        let ps: Vec<_> = d.paragraphs().collect();
        assert_eq!(ps[0].list, ListKind::Bullet);
        assert_eq!(ps[1].runs[0].text, "二");
        assert!(notes.iter().any(|n| n.contains("img")), "{notes:?}");
        assert!(notes.iter().any(|n| n.contains("blink")), "{notes:?}");
    }
}

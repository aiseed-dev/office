//! kumihan — 組版エンジンの核。Japanese-Office(仮称)の心臓。
//!
//! やること: 文書(段落の列)を、実フォントの字幅で行に組み、
//! 置かれた文字の座標(紙面)を返す。UIにもPDFにも依存しない。
//! 画面も紙も、この紙面を別の面に写すだけ — だから画面と印刷が一致する。
//!
//! v0 の範囲: 横組み。JIS X 4051 のうち
//!   - 行頭禁則(。、」などが行頭に来ない — 追い出しで直す)
//!   - 行末禁則(「(『 などが行末に残らない)
//!   - 欧文の語中で改行しない(語ごと次行へ)
//! 縦書き・ルビ・均等割付・ぶら下げは K4(モデルはそれを妨げない形にする)。

pub mod edit;
pub use edit::Editor;

use ttf_parser::Face;

// ---------- 文書モデル ----------

/// 文字の書式。**docx の `w:rPr` に対応する。**
///
/// 既定(全部 false・色なし)が素の本文。`Default` で作れば何も付かない。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CharFormat {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// 文字色。`RRGGBB`(docx の `w:color w:val` と同じ形)
    pub color: Option<String>,
}

impl CharFormat {
    pub fn is_plain(&self) -> bool {
        *self == CharFormat::default()
    }
}

#[derive(Debug, Clone)]
pub struct Run {
    pub text: String,
    pub size_pt: f32,
    /// 書体の名前。**フォントは文書の設定**であって、アプリの好みではない。
    /// docx の `w:rFonts`、xlsx の `<font><name>` に入っているもの。
    /// `None` は文書の既定に従う
    pub font: Option<String>,
    pub fmt: CharFormat,
}

/// 段落の揃え。docx の `w:jc`。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
    /// 両端揃え
    Justify,
}

impl Align {
    /// docx の `w:jc w:val` の値
    pub fn as_docx(self) -> &'static str {
        match self {
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
            Align::Justify => "both",
        }
    }
    pub fn from_docx(v: &str) -> Align {
        match v {
            "center" => Align::Center,
            "right" | "end" => Align::Right,
            "both" | "distribute" => Align::Justify,
            _ => Align::Left,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Paragraph {
    pub runs: Vec<Run>,
    pub align: Align,
}

/// 表の1セル。中は段落の列(セルの中にも段落がある)
#[derive(Debug, Clone, Default)]
pub struct Cellbox {
    pub paragraphs: Vec<Paragraph>,
}

/// 罫線の表。日本の事務様式の本体。
#[derive(Debug, Clone, Default)]
pub struct Table {
    pub rows: Vec<Vec<Cellbox>>,
}

/// 文書の中身は、段落か表。順序を保つ。
#[derive(Debug, Clone)]
pub enum Block {
    Para(Paragraph),
    Table(Table),
}

#[derive(Debug, Clone, Default)]
pub struct Document {
    /// 本文の流れ(段落と表が混ざる)
    pub blocks: Vec<Block>,
    /// 文書の既定の書体(docx の `w:docDefaults`)。
    /// 段落側が指定していなければこれを使う
    pub font: Option<String>,
}

impl Document {
    /// 段落だけを順に見る(組版は v0 では段落のみを組む)
    pub fn paragraphs(&self) -> impl Iterator<Item = &Paragraph> {
        self.blocks.iter().filter_map(|b| match b {
            Block::Para(p) => Some(p),
            Block::Table(_) => None,
        })
    }
    pub fn tables(&self) -> impl Iterator<Item = &Table> {
        self.blocks.iter().filter_map(|b| match b {
            Block::Table(t) => Some(t),
            Block::Para(_) => None,
        })
    }
    pub fn push_para(&mut self, p: Paragraph) {
        self.blocks.push(Block::Para(p));
    }

    /// 本文を編集用のプレーンテキストにする(表は含めない)。
    /// 段落の区切りは改行。編集はこの形で行い、保存時に段落へ戻す。
    pub fn body_text(&self) -> String {
        self.paragraphs()
            .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 編集後のテキストを段落に戻す。**表はそのままの位置に残す**
    /// (本文だけ差し替える。表を編集で失わせない)。
    /// 編集中の平文を本文へ戻す。
    ///
    /// **段落の書式を捨てない。** 以前はここで作り直していたので、
    /// 打鍵のたびに太字も揃えも消えていた。
    /// 同じ位置の段落からは、揃えと文字書式を引き継ぐ。
    pub fn set_body_text(&mut self, text: &str, size_pt: f32) {
        let tables: Vec<Block> = self.blocks.iter()
            .filter(|b| matches!(b, Block::Table(_))).cloned().collect();
        // 引き継ぐ元(段落だけを順に)
        let old: Vec<(Align, Option<String>, CharFormat, Option<f32>)> = self
            .paragraphs()
            .map(|p| {
                let r = p.runs.first();
                (p.align,
                 r.and_then(|r| r.font.clone()),
                 r.map(|r| r.fmt.clone()).unwrap_or_default(),
                 r.map(|r| r.size_pt))
            })
            .collect();
        self.blocks = text
            .split('\n')
            .enumerate()
            .map(|(i, s)| {
                let (align, font, fmt, old_pt) = old.get(i).cloned().unwrap_or_default();
                Block::Para(Paragraph {
                    align,
                    runs: vec![Run {
                        text: s.to_string(),
                        // 段落に付いていた大きさを守る。無ければ既定
                        size_pt: old_pt.unwrap_or(size_pt),
                        font,
                        fmt,
                    }],
                })
            })
            .collect();
        self.blocks.extend(tables);
    }

    /// 平文の中の位置(バイト)から、何番目の段落かを出す。
    fn para_range(&self, range: std::ops::Range<usize>) -> std::ops::Range<usize> {
        let mut at = 0usize;
        let (mut first, mut last) = (usize::MAX, 0usize);
        for (i, p) in self.paragraphs().enumerate() {
            let len: usize = p.runs.iter().map(|r| r.text.len()).sum();
            let end = at + len;
            // 空の選択(カーソルだけ)でも、その段落は対象にする
            if range.start <= end && range.end >= at {
                first = first.min(i);
                last = last.max(i);
            }
            at = end + 1; // 改行1つぶん
        }
        if first == usize::MAX { 0..0 } else { first..last + 1 }
    }

    /// 選択範囲にかかる段落の文字書式を変える。
    ///
    /// **段落まるごとに掛ける。** 編集中の本文は平文で持っているので、
    /// 段落の途中だけを太字にする仕組みがまだ無い。
    /// できないことをできるように見せないため、粒度をそろえてある。
    pub fn apply_char_format(
        &mut self,
        range: std::ops::Range<usize>,
        f: impl Fn(&mut CharFormat),
    ) {
        let target = self.para_range(range);
        for (i, b) in self.blocks.iter_mut().filter(|b| matches!(b, Block::Para(_))).enumerate() {
            if !target.contains(&i) {
                continue;
            }
            if let Block::Para(p) = b {
                for r in &mut p.runs {
                    f(&mut r.fmt);
                }
            }
        }
    }

    /// 選択範囲にかかる段落の文字の大きさを変える。
    ///
    /// 上限と下限を持つ — **際限なく大きく/小さくできると事故になる**
    /// (0pt にすると本文が消え、原因が分からなくなる)。
    pub fn apply_size(&mut self, range: std::ops::Range<usize>, f: impl Fn(f32) -> f32) {
        let target = self.para_range(range);
        for (i, b) in self.blocks.iter_mut().filter(|b| matches!(b, Block::Para(_))).enumerate() {
            if !target.contains(&i) {
                continue;
            }
            if let Block::Para(p) = b {
                for r in &mut p.runs {
                    r.size_pt = f(r.size_pt).clamp(4.0, 400.0);
                }
            }
        }
    }

    /// 選択範囲にかかる段落の書体を変える。
    pub fn apply_font(&mut self, range: std::ops::Range<usize>, name: Option<String>) {
        let target = self.para_range(range);
        for (i, b) in self.blocks.iter_mut().filter(|b| matches!(b, Block::Para(_))).enumerate() {
            if !target.contains(&i) {
                continue;
            }
            if let Block::Para(p) = b {
                for r in &mut p.runs {
                    r.font = name.clone();
                }
            }
        }
    }

    /// いま選択範囲の文字の大きさ。
    pub fn size_at(&self, range: std::ops::Range<usize>) -> Option<f32> {
        let target = self.para_range(range);
        self.paragraphs().nth(target.start).and_then(|p| p.runs.first()).map(|r| r.size_pt)
    }

    /// 選択範囲にかかる段落の揃えを変える。
    pub fn apply_align(&mut self, range: std::ops::Range<usize>, align: Align) {
        let target = self.para_range(range);
        for (i, b) in self.blocks.iter_mut().filter(|b| matches!(b, Block::Para(_))).enumerate() {
            if target.contains(&i) {
                if let Block::Para(p) = b {
                    p.align = align;
                }
            }
        }
    }

    /// いま選択範囲が太字か(ボタンを押した状態に見せるため)。
    pub fn char_format_at(&self, range: std::ops::Range<usize>) -> CharFormat {
        let target = self.para_range(range);
        self.paragraphs()
            .nth(target.start)
            .and_then(|p| p.runs.first())
            .map(|r| r.fmt.clone())
            .unwrap_or_default()
    }

    /// いま選択範囲の揃え。
    pub fn align_at(&self, range: std::ops::Range<usize>) -> Align {
        let target = self.para_range(range);
        self.paragraphs().nth(target.start).map(|p| p.align).unwrap_or_default()
    }
}

impl Document {
    pub fn plain(text: &str, size_pt: f32) -> Document {
        Document {
            font: None,
            blocks: text
                .split('\n')
                .map(|p| Block::Para(Paragraph {
                    align: Default::default(),
                    runs: vec![Run { text: p.to_string(), size_pt, font: None, fmt: Default::default() }] }))
                .collect(),
        }
    }
}

// ---------- 紙面(組んだ結果) ----------

/// 置かれた1文字。座標は紙の左上原点・mm。
#[derive(Debug, Clone)]
pub struct Cell {
    pub ch: char,
    pub x_mm: f32,
    pub w_mm: f32,
    pub size_pt: f32,
    /// この字の書式。**画面も紙も同じものを見る**ので、
    /// 太字や色が片方だけ出ることが起きない
    pub fmt: CharFormat,
}

#[derive(Debug, Clone)]
pub struct Line {
    pub cells: Vec<Cell>,
    pub y_mm: f32, // ベースライン
}

impl Line {
    pub fn text(&self) -> String {
        self.cells.iter().map(|c| c.ch).collect()
    }
    pub fn width_mm(&self) -> f32 {
        self.cells.iter().map(|c| c.w_mm).sum()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Sheet {
    pub lines: Vec<Line>,
}

// ---------- フォント(字幅の出どころ) ----------

pub mod font;

pub struct Metrics<'a> {
    face: Face<'a>,
    upem: f32,
}

const PT_TO_MM: f32 = 25.4 / 72.0;

impl<'a> Metrics<'a> {
    pub fn new(font_data: &'a [u8]) -> Result<Metrics<'a>, String> {
        let face = Face::parse(font_data, 0).map_err(|e| e.to_string())?;
        let upem = face.units_per_em() as f32;
        Ok(Metrics { face, upem })
    }

    /// 1文字の送り幅(mm)。フォントに無い文字は全角の半分で仮置きする。
    pub fn advance_mm(&self, ch: char, size_pt: f32) -> f32 {
        let adv = self
            .face
            .glyph_index(ch)
            .and_then(|g| self.face.glyph_hor_advance(g))
            .map(|a| a as f32 / self.upem)
            .unwrap_or(0.5);
        adv * size_pt * PT_TO_MM
    }
}

// ---------- 禁則(JIS X 4051 の主要部) ----------

/// 行頭に置けない(句読点・閉じ括弧・小書き仮名・長音など)
pub const GYOTO_KINSOKU: &str =
    "、。，．・：；？！ヽヾゝゞ々ー〜…‥ぁぃぅぇぉっゃゅょゎァィゥェォッャュョヮ\
     ）」』】〕〉》〙〗ゕゖㇷ゚%‰′″℃)]}>,.:;?!";

/// 行末に置けない(開き括弧など)
pub const GYOMATSU_KINSOKU: &str = "（「『【〔〈《〘〖([{<";

fn is_gyoto_kinsoku(c: char) -> bool {
    GYOTO_KINSOKU.contains(c)
}
fn is_gyomatsu_kinsoku(c: char) -> bool {
    GYOMATSU_KINSOKU.contains(c)
}

// ---------- 行組み ----------

/// 改行の単位。CJKは1字ずつ、欧文は語ごと(語中では折らない)。
#[derive(Debug)]
enum Tok {
    One(char, f32, f32, CharFormat),         // (字, 幅mm, サイズpt, 書式)
    Word(Vec<(char, f32)>, f32, CharFormat), // (字と幅の列, サイズpt, 書式)
    Space(f32, f32, CharFormat),
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

fn tokenize(p: &Paragraph, m: &Metrics) -> Vec<Tok> {
    let mut out = Vec::new();
    for run in &p.runs {
        let mut word: Vec<(char, f32)> = Vec::new();
        for ch in run.text.chars() {
            if is_word_char(ch) {
                word.push((ch, m.advance_mm(ch, run.size_pt)));
                continue;
            }
            if !word.is_empty() {
                out.push(Tok::Word(std::mem::take(&mut word), run.size_pt, run.fmt.clone()));
            }
            if ch == ' ' || ch == '\u{3000}' {
                out.push(Tok::Space(m.advance_mm(ch, run.size_pt), run.size_pt, run.fmt.clone()));
            } else {
                out.push(Tok::One(ch, m.advance_mm(ch, run.size_pt), run.size_pt, run.fmt.clone()));
            }
        }
        if !word.is_empty() {
            out.push(Tok::Word(word, run.size_pt, run.fmt.clone()));
        }
    }
    out
}

pub struct Frame {
    pub measure_mm: f32,   // 行長
    pub line_height_mm: f32,
    pub y0_mm: f32,        // 最初のベースライン
}

/// 段落の列を行に組む。
///
/// 禁則はその場で解決する(後処理にしない — 後から字を送ると送った先が
/// 行長を超えるため)。行を折る瞬間に:
///   1. 折る原因の字が行頭禁則なら、新しい行の頭が禁則でなくなるまで
///      前の行の末尾から字を引き取る(追い出し)
///   2. 前の行の末尾に行末禁則(開き括弧)が残っていれば、それも引き取る
/// 引き取った分だけ前の行は短くなる — 行長を超える方向には決して動かない。
pub fn layout(doc: &Document, m: &Metrics, frame: &Frame) -> Sheet {
    let mut sheet = Sheet::default();
    let mut y = frame.y0_mm;

    for para in doc.paragraphs() {
        let mut done: Vec<Vec<Cell>> = Vec::new();
        let mut cur: Vec<Cell> = Vec::new();
        let mut w_cur = 0.0f32;

        // 行を閉じ、禁則ぶんを引き取って次の行の頭(carry)を返す
        fn close(done: &mut Vec<Vec<Cell>>, cur: &mut Vec<Cell>, w_cur: &mut f32,
                 incoming_head: Option<char>) -> Vec<Cell> {
            let mut carry: Vec<Cell> = Vec::new();
            // 1) 折る原因の字が行頭禁則 → 頭が禁則でなくなるまで引き取る
            if incoming_head.map_or(false, is_gyoto_kinsoku) {
                while cur.len() > 1 {
                    let c = cur.pop().unwrap();
                    let head_ok = !is_gyoto_kinsoku(c.ch);
                    carry.insert(0, c);
                    if head_ok {
                        break;
                    }
                }
            }
            // 2) 行末に開き括弧を残さない
            while cur.len() > 1 && cur.last().map_or(false, |c| is_gyomatsu_kinsoku(c.ch)) {
                let c = cur.pop().unwrap();
                carry.insert(0, c);
            }
            done.push(std::mem::take(cur));
            *w_cur = carry.iter().map(|c| c.w_mm).sum();
            carry
        }

        for tok in tokenize(para, m) {
            let (cells, w): (Vec<Cell>, f32) = match &tok {
                Tok::One(ch, w, s, f) =>
                    (vec![Cell { ch: *ch, x_mm: 0.0, w_mm: *w, size_pt: *s, fmt: f.clone() }], *w),
                Tok::Word(cs, s, f) => (
                    cs.iter().map(|(c, w)| Cell { ch: *c, x_mm: 0.0, w_mm: *w, size_pt: *s, fmt: f.clone() })
                        .collect(),
                    cs.iter().map(|(_, w)| *w).sum()),
                Tok::Space(w, s, f) =>
                    (vec![Cell { ch: ' ', x_mm: 0.0, w_mm: *w, size_pt: *s, fmt: f.clone() }], *w),
            };

            if w_cur + w > frame.measure_mm && !cur.is_empty() {
                if let Tok::Space(..) = tok {
                    // 行末に空白は要らない。行を折るだけ
                    cur = close(&mut done, &mut cur, &mut w_cur, None);
                    continue;
                }
                let head = cells.first().map(|c| c.ch);
                cur = close(&mut done, &mut cur, &mut w_cur, head);
            }
            if cur.is_empty() {
                if let Tok::Space(..) = tok {
                    continue; // 行頭の空白は組まない
                }
            }
            w_cur += w;
            cur.extend(cells);
        }
        if !cur.is_empty() || done.is_empty() {
            done.push(cur);
        }

        // x座標を確定して紙面へ
        for cells in done {
            if cells.is_empty() {
                y += frame.line_height_mm;
                continue;
            }
            // 揃え。**行の幅と行長の差を、どこに置くか**の話でしかない
            let w: f32 = cells.iter().map(|c| c.w_mm).sum();
            let slack = (frame.measure_mm - w).max(0.0);
            let mut x = match para.align {
                Align::Left | Align::Justify => 0.0,
                Align::Center => slack / 2.0,
                Align::Right => slack,
            };
            let cells: Vec<Cell> = cells
                .into_iter()
                .map(|mut c| { c.x_mm = x; x += c.w_mm; c })
                .collect();
            sheet.lines.push(Line { cells, y_mm: y });
            y += frame.line_height_mm;
        }
    }
    sheet
}

// ---------- 検査 ----------

#[cfg(test)]
mod tests {
    use super::*;

    fn font() -> Vec<u8> {
        // **同梱しない。** システムのフォントを使う
        let (f, _) = crate::font::for_document(None).expect("日本語フォントが要る");
        crate::font::load(f).expect("読めない")
    }

    fn sheet_of(text: &str, measure: f32) -> Sheet {
        let data = font();
        let m = Metrics::new(&data).unwrap();
        let doc = Document::plain(text, 10.5);
        layout(&doc, &m, &Frame { measure_mm: measure, line_height_mm: 6.0, y0_mm: 20.0 })
    }

    const SAMPLE: &str = "日本の事務の実態は、文書ではなく様式です。その様式の定義をテキストにして、記入用の帳票・検証・データベースを全部そこから派生させます。「原本はテキスト。」と、私たちは Rust で書きます。";

    #[test]
    fn 行頭に句読点や閉じ括弧が来ない() {
        for measure in [30.0, 40.0, 55.0, 70.0, 90.0] {
            let s = sheet_of(SAMPLE, measure);
            for l in &s.lines {
                let c = l.cells[0].ch;
                assert!(!is_gyoto_kinsoku(c),
                    "行長{measure}mm で行頭が「{c}」: {}", l.text());
            }
        }
    }

    #[test]
    fn 行末に開き括弧が残らない() {
        for measure in [30.0, 40.0, 55.0, 70.0, 90.0] {
            let s = sheet_of(SAMPLE, measure);
            for l in &s.lines {
                let c = l.cells.last().unwrap().ch;
                assert!(!is_gyomatsu_kinsoku(c),
                    "行長{measure}mm で行末が「{c}」: {}", l.text());
            }
        }
    }

    #[test]
    fn 欧文の語は行の中で割れない() {
        for measure in [30.0, 40.0, 55.0, 70.0] {
            let s = sheet_of(SAMPLE, measure);
            let joined: Vec<String> = s.lines.iter().map(|l| l.text()).collect();
            // "Rust" がどこかの行に丸ごとある(行またぎで割れていない)
            assert!(joined.iter().any(|t| t.contains("Rust")),
                "行長{measure}mm で Rust が割れた: {joined:?}");
        }
    }

    #[test]
    fn 行長を大きく超えない() {
        // 追い出しで短くなるのは良い。超えるのは駄目(はみ出し)
        for measure in [40.0, 55.0, 70.0] {
            let s = sheet_of(SAMPLE, measure);
            for l in &s.lines {
                assert!(l.width_mm() <= measure + 0.1,
                    "行長{measure}mm を超過: {:.2}mm 「{}」", l.width_mm(), l.text());
            }
        }
    }

    #[test]
    fn 文字は一つも失われない() {
        let want: String = SAMPLE.chars().filter(|c| *c != ' ').collect();
        let s = sheet_of(SAMPLE, 55.0);
        let got: String = s.lines.iter().flat_map(|l| l.cells.iter())
            .map(|c| c.ch).filter(|c| *c != ' ').collect();
        assert_eq!(got, want);
    }

    #[test]
    fn 実フォントの字幅で組んでいる() {
        let data = font();
        let m = Metrics::new(&data).unwrap();
        let zen = m.advance_mm('あ', 10.5);
        let han = m.advance_mm('i', 10.5);
        assert!(zen > 3.0 && zen < 4.5, "全角の送りが不自然: {zen}mm");
        assert!(han < zen * 0.6, "半角が全角より十分細くない: {han}mm vs {zen}mm");
    }
}

#[cfg(test)]
mod format_tests {
    use super::*;

    fn doc(text: &str) -> Document {
        Document::plain(text, 10.5)
    }

    #[test]
    fn 打鍵しても書式が消えない() {
        // 以前は set_body_text が段落を作り直していたので、打つたびに太字が消えた
        let mut d = doc("表題\n本文");
        d.apply_char_format(0..2, |f| f.bold = true);
        d.apply_align(0..2, Align::Center);
        // 1文字打った、のつもり
        d.set_body_text("表題あ\n本文", 10.5);
        let p = d.paragraphs().next().unwrap();
        assert!(p.runs[0].fmt.bold, "太字が消えた");
        assert_eq!(p.align, Align::Center, "揃えが消えた");
    }

    #[test]
    fn 段落が増えても前の書式は残る() {
        let mut d = doc("表題");
        d.apply_char_format(0..2, |f| f.bold = true);
        d.set_body_text("表題\n新しい段落", 10.5);
        let ps: Vec<_> = d.paragraphs().collect();
        assert!(ps[0].runs[0].fmt.bold);
        assert!(!ps[1].runs[0].fmt.bold, "新しい段落まで太字になった");
    }

    #[test]
    fn 選択した段落だけに掛かる() {
        let mut d = doc("一行目\n二行目\n三行目");
        // 「二行目」は 4..7(一行目=9バイト+改行)
        let start = "一行目\n".len();
        d.apply_char_format(start..start + 3, |f| f.bold = true);
        let ps: Vec<_> = d.paragraphs().collect();
        assert!(!ps[0].runs[0].fmt.bold, "上の段落まで太字になった");
        assert!(ps[1].runs[0].fmt.bold, "選んだ段落が太字にならない");
        assert!(!ps[2].runs[0].fmt.bold, "下の段落まで太字になった");
    }

    #[test]
    fn 複数の段落にまたがる選択() {
        let mut d = doc("一行目\n二行目\n三行目");
        let end = "一行目\n二行目".len();
        d.apply_align(0..end, Align::Center);
        let ps: Vec<_> = d.paragraphs().collect();
        assert_eq!(ps[0].align, Align::Center);
        assert_eq!(ps[1].align, Align::Center);
        assert_eq!(ps[2].align, Align::Left, "選んでいない段落まで動いた");
    }

    #[test]
    fn 今の書式を読める() {
        // ボタンを押した状態に見せるために要る
        let mut d = doc("表題\n本文");
        d.apply_char_format(0..2, |f| f.bold = true);
        d.apply_align(0..2, Align::Right);
        assert!(d.char_format_at(0..2).bold);
        assert_eq!(d.align_at(0..2), Align::Right);
        let second = "表題\n".len();
        assert!(!d.char_format_at(second..second).bold, "別の段落の書式を返した");
    }

    #[test]
    fn 表は消えない() {
        let mut d = doc("本文");
        d.blocks.push(Block::Table(Table { rows: vec![vec![Cellbox::default()]] }));
        d.set_body_text("本文を直した", 10.5);
        assert_eq!(d.tables().count(), 1, "表が消えた");
    }
}

#[cfg(test)]
mod align_tests {
    use super::*;

    fn sheet(text: &str, a: Align) -> Sheet {
        let data = font::load(font::for_document(None).unwrap().0).unwrap();
        let m = Metrics::new(&data).unwrap();
        let mut d = Document::plain(text, 10.5);
        d.apply_align(0..text.len(), a);
        layout(&d, &m, &Frame { measure_mm: 100.0, line_height_mm: 6.0, y0_mm: 20.0 })
    }

    #[test]
    fn 中央揃えは左右の余りが等しい() {
        let s = sheet("表題", Align::Center);
        let line = &s.lines[0];
        let left = line.cells[0].x_mm;
        let right = 100.0 - (line.cells.last().unwrap().x_mm + line.cells.last().unwrap().w_mm);
        assert!((left - right).abs() < 0.01, "左 {left}mm / 右 {right}mm");
        assert!(left > 1.0, "中央に寄っていない");
    }

    #[test]
    fn 右揃えは行末が行長に届く() {
        let s = sheet("表題", Align::Right);
        let last = s.lines[0].cells.last().unwrap();
        assert!((last.x_mm + last.w_mm - 100.0).abs() < 0.01, "右端に着いていない");
    }

    #[test]
    fn 左揃えは0から始まる() {
        assert_eq!(sheet("表題", Align::Left).lines[0].cells[0].x_mm, 0.0);
    }

    #[test]
    fn 書式が字まで届く() {
        // 画面と紙が同じものを見るので、片方だけ太字になることが起きない
        let data = font::load(font::for_document(None).unwrap().0).unwrap();
        let m = Metrics::new(&data).unwrap();
        let mut d = Document::plain("太字", 10.5);
        d.apply_char_format(0..6, |f| f.bold = true);
        let s = layout(&d, &m, &Frame { measure_mm: 100.0, line_height_mm: 6.0, y0_mm: 20.0 });
        assert!(s.lines[0].cells.iter().all(|c| c.fmt.bold), "字に書式が届いていない");
    }
}

#[cfg(test)]
mod size_tests {
    use super::*;

    #[test]
    fn 打鍵しても大きさが戻らない() {
        let mut d = Document::plain("表題\n本文", 10.5);
        d.apply_size(0..2, |s| s + 6.0);
        d.set_body_text("表題あ\n本文", 10.5);
        assert_eq!(d.size_at(0..2), Some(16.5), "大きさが既定に戻った");
        let second = "表題あ\n".len();
        assert_eq!(d.size_at(second..second), Some(10.5), "他の段落まで変わった");
    }

    #[test]
    fn 際限なく大きくならない() {
        // 0pt にすると本文が消えて、原因が分からなくなる
        let mut d = Document::plain("本文", 10.5);
        for _ in 0..100 { d.apply_size(0..2, |s| s - 10.0) }
        assert!(d.size_at(0..2).unwrap() >= 4.0, "小さくしすぎた");
        for _ in 0..100 { d.apply_size(0..2, |s| s * 2.0) }
        assert!(d.size_at(0..2).unwrap() <= 400.0, "大きくしすぎた");
    }

    #[test]
    fn 書体を段落に掛けられる() {
        let mut d = Document::plain("表題\n本文", 10.5);
        d.apply_font(0..2, Some("BIZ UDPゴシック".into()));
        assert_eq!(d.paragraphs().next().unwrap().runs[0].font.as_deref(), Some("BIZ UDPゴシック"));
        assert_eq!(d.paragraphs().nth(1).unwrap().runs[0].font, None, "他の段落まで変わった");
    }
}

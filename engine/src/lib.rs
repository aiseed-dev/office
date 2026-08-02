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
    /// 上付き(x²)・下付き(H₂O)。docx の w:vertAlign
    pub superscript: bool,
    pub subscript: bool,
    /// 蛍光ペン。docx の w:highlight(yellow 等の名前で持つ)
    pub highlight: Option<String>,
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

/// 段落に入っている画像。表示のためのもの。
#[derive(Debug, Clone)]
pub struct InlineImage {
    /// 画像ファイルの中身(png/jpeg のまま)
    pub bytes: std::sync::Arc<Vec<u8>>,
    pub w_mm: f32,
    pub h_mm: f32,
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

/// 箇条書きの種類。docx の `w:numPr` に対応する。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ListKind {
    #[default]
    None,
    /// 中黒の箇条書き
    Bullet,
    /// 段落番号
    Number,
}

#[derive(Debug, Clone, Default)]
pub struct Paragraph {
    pub runs: Vec<Run>,
    /// 読めなかった要素(画像など)の原文。**理解はしないが、捨てない。**
    /// 保存でそのまま返す
    pub anchors: Vec<String>,
    /// 表示できる画像(anchors のうち、絵の実体と大きさが分かったもの)。
    /// 保存には使わない — 保存は anchors の原文が担う
    pub images: Vec<InlineImage>,
    pub align: Align,
    /// この段落の前で改ページする(docx の w:pageBreakBefore)
    pub page_break_before: bool,
    pub list: ListKind,
    /// 左のインデント段数。1段 = 全角2文字ぶん(日本の書類の慣習)
    pub indent: u8,
    /// 行間の倍率。1.0 が既定
    pub line_spacing: f32,
}

impl Paragraph {
    /// 行間の倍率。0 や負が入っていても壊れない値を返す。
    pub fn spacing(&self) -> f32 {
        if self.line_spacing <= 0.0 { 1.0 } else { self.line_spacing.clamp(0.5, 5.0) }
    }

    /// 箇条書きの頭に付く印。組版のときに本文の前へ置く。
    pub fn marker(&self, nth: usize) -> Option<String> {
        match self.list {
            ListKind::None => None,
            ListKind::Bullet => Some("・".into()),
            ListKind::Number => Some(format!("{}. ", nth + 1)),
        }
    }
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
    /// 列の幅(mm)。docx の `w:gridCol`。空なら等分
    pub col_mm: Vec<f32>,
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
        #[allow(clippy::type_complexity)]
        let old: Vec<(Align, Option<String>, CharFormat, Option<f32>, ListKind, u8, f32,
                      Vec<String>, bool, Vec<InlineImage>)> = self
            .paragraphs()
            .map(|p| {
                let r = p.runs.first();
                (p.align,
                 r.and_then(|r| r.font.clone()),
                 r.map(|r| r.fmt.clone()).unwrap_or_default(),
                 r.map(|r| r.size_pt),
                 p.list, p.indent, p.line_spacing, p.anchors.clone(),
                 p.page_break_before, p.images.clone())
            })
            .collect();
        self.blocks = text
            .split('\n')
            .enumerate()
            .map(|(i, s)| {
                // 段落の性質は同じ位置から引き継ぐ。**改ページも画像(anchors)も**
                // ここで持ち越さないと、1文字打つだけで消える
                let (align, font, fmt, old_pt, list, indent, ls, anchors, pbb, images) =
                    old.get(i).cloned().unwrap_or((Align::default(), None,
                        CharFormat::default(), None, ListKind::default(), 0, 1.0,
                        Vec::new(), false, Vec::new()));
                Block::Para(Paragraph {
                    align,
                    anchors,
                    images,
                    page_break_before: pbb,
                    list,
                    indent,
                    line_spacing: ls,
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

    /// 選択範囲にかかる段落の性質(箇条書き・インデント・行間)を変える。
    pub fn apply_para(&mut self, range: std::ops::Range<usize>, f: impl Fn(&mut Paragraph)) {
        let target = self.para_range(range);
        for (i, b) in self.blocks.iter_mut().filter(|b| matches!(b, Block::Para(_))).enumerate() {
            if target.contains(&i) {
                if let Block::Para(p) = b {
                    f(p);
                }
            }
        }
    }

    /// いま選択範囲の段落の性質。
    pub fn para_at(&self, range: std::ops::Range<usize>) -> Option<&Paragraph> {
        let target = self.para_range(range);
        self.paragraphs().nth(target.start)
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
                    anchors: Vec::new(),
                    images: Vec::new(),
                    page_break_before: false,
                    list: Default::default(),
                    indent: 0,
                    line_spacing: 1.0,
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
    /// 段落の頭からのバイト位置。**カーソルはこの値で本文と結ぶ**
    /// (行の文字数で数えると、折り返しや落とした空白でずれる)
    pub off: usize,
    /// この字の書式。**画面も紙も同じものを見る**ので、
    /// 太字や色が片方だけ出ることが起きない
    pub fmt: CharFormat,
}

#[derive(Debug, Clone)]
pub struct Line {
    pub cells: Vec<Cell>,
    pub y_mm: f32, // ベースライン
    /// 本文由来か。**表のセルの行は false** —
    /// カーソルや変換下線の位置合わせは本文の行だけを数える
    pub from_body: bool,
    /// この行の頭が、本文(段落を \n で繋いだもの)の何バイト目か。
    /// 表の行では**セルの文章**の中の位置
    pub byte0: usize,
    /// 表のセル由来なら (表の番号, 行, 列)
    pub cell: Option<(usize, usize, usize)>,
}

impl Line {
    /// この行が本文の何バイト目までを含むか(行末の改行は含まない)。
    ///
    /// 行の中の字は連続しているとは限らない(折り返しで空白が落ちる)ので、
    /// 最後の字の段落内位置から出す。
    pub fn byte_end(&self) -> usize {
        let base = self.cells.iter().map(|c| c.off).min().unwrap_or(0);
        self.cells
            .last()
            .map(|c| self.byte0 + (c.off + c.ch.len_utf8()) - base)
            .unwrap_or(self.byte0)
    }
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
    /// ここで新しいページを始める、という y(巻物の座標)。
    /// 紙に写す側([`paper`]相当)がこれを見て強制的に頁を割る
    pub breaks: Vec<f32>,
    /// 引く線(表の罫線)。[x1, y1, x2, y2] mm。
    /// 画面も紙も、これをそのまま引く
    pub rules: Vec<[f32; 4]>,
    /// 表のセルの当たり判定(クリックでセルを選ぶため)
    pub cell_boxes: Vec<CellBox>,
    /// 置いた画像(実体, [x, 上端y, 幅, 高さ] mm)。画面も紙もこれを見る
    pub images: Vec<(std::sync::Arc<Vec<u8>>, [f32; 4])>,
}

/// 表のセル1つぶんの場所。
#[derive(Debug, Clone, Copy)]
pub struct CellBox {
    pub table: usize,
    pub row: usize,
    pub col: usize,
    pub x_mm: f32,
    pub top_mm: f32,
    pub w_mm: f32,
    pub h_mm: f32,
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
    One(char, f32, f32, CharFormat, usize),         // (字, 幅mm, サイズpt, 書式, バイト位置)
    Word(Vec<(char, f32, usize)>, f32, CharFormat), // (字と幅と位置の列, サイズpt, 書式)
    Space(f32, f32, CharFormat, usize),
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

fn tokenize(p: &Paragraph, m: &Metrics) -> Vec<Tok> {
    let mut out = Vec::new();
    // 段落の頭からのバイト位置。run をまたいで通しで数える
    let mut off = 0usize;
    for run in &p.runs {
        let mut word: Vec<(char, f32, usize)> = Vec::new();
        for ch in run.text.chars() {
            if is_word_char(ch) {
                word.push((ch, m.advance_mm(ch, run.size_pt), off));
                off += ch.len_utf8();
                continue;
            }
            if !word.is_empty() {
                out.push(Tok::Word(std::mem::take(&mut word), run.size_pt, run.fmt.clone()));
            }
            if ch == ' ' || ch == '\u{3000}' {
                out.push(Tok::Space(m.advance_mm(ch, run.size_pt), run.size_pt, run.fmt.clone(), off));
            } else {
                out.push(Tok::One(ch, m.advance_mm(ch, run.size_pt), run.size_pt, run.fmt.clone(), off));
            }
            off += ch.len_utf8();
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
/// 段落を行長で折る。x はまだ置かない(呼ぶ側が揃え・字下げを決める)。
fn break_para(para: &Paragraph, m: &Metrics, measure: f32, marker: Option<&str>) -> Vec<Vec<Cell>> {
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

    // 箇条書きの印は本文の前に置く。**本文の一部にはしない**ので、
    // 編集中の文字位置とずれない(印は組版のときだけ現れる)
    if let Some(mk) = marker {
        let size = para.runs.first().map(|r| r.size_pt).unwrap_or(10.5);
        let fmt = para.runs.first().map(|r| r.fmt.clone()).unwrap_or_default();
        for ch in mk.chars() {
            let w = m.advance_mm(ch, size);
            // 印は本文の一部ではないので off は段落頭(0)のまま
            cur.push(Cell { ch, x_mm: 0.0, w_mm: w, size_pt: size, fmt: fmt.clone(), off: 0 });
            w_cur += w;
        }
    }
    for tok in tokenize(para, m) {
        let (cells, w): (Vec<Cell>, f32) = match &tok {
            Tok::One(ch, w, s, f, o) =>
                (vec![Cell { ch: *ch, x_mm: 0.0, w_mm: *w, size_pt: *s, fmt: f.clone(), off: *o }], *w),
            Tok::Word(cs, s, f) => (
                cs.iter().map(|(c, w, o)| Cell { ch: *c, x_mm: 0.0, w_mm: *w, size_pt: *s, fmt: f.clone(), off: *o })
                    .collect(),
                cs.iter().map(|(_, w, _)| *w).sum()),
            Tok::Space(w, s, f, o) =>
                (vec![Cell { ch: ' ', x_mm: 0.0, w_mm: *w, size_pt: *s, fmt: f.clone(), off: *o }], *w),
        };

        if w_cur + w > measure && !cur.is_empty() {
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
    done
}

/// セルの中の余白(mm)
const CELL_PAD: f32 = 1.4;

fn lh_of(para: &Paragraph, frame: &Frame) -> f32 {
    frame.line_height_mm * para.spacing()
}

pub fn layout(doc: &Document, m: &Metrics, frame: &Frame) -> Sheet {
    let mut sheet = Sheet::default();
    let mut y = frame.y0_mm;

    // 段落番号は「何番目の箇条書きか」で決まる。段落の位置ではない
    let mut nth = 0usize;
    // 本文(段落を \n で繋いだもの)における、いまの段落の頭のバイト位置
    let mut para_byte0 = 0usize;
    let mut table_no = 0usize;
    for block in &doc.blocks {
        match block {
            Block::Para(para) => {
                // 改ページ。紙に写すときにここで頁が割れる
                if para.page_break_before && !sheet.lines.is_empty() {
                    sheet.breaks.push(y);
                }
                // インデント1段 = 全角2文字ぶん(日本の書類の慣習)
                let em = para.runs.first().map(|r| r.size_pt).unwrap_or(10.5) * 25.4 / 72.0;
                let indent_mm = para.indent as f32 * em * 2.0;
                let measure = (frame.measure_mm - indent_mm).max(em);
                let marker = para.marker(nth);
                match para.list {
                    ListKind::None => nth = 0,
                    _ => nth += 1,
                }
                for cells in break_para(para, m, measure, marker.as_deref()) {
                    if cells.is_empty() {
                        // 空の段落も**行として持つ**。持たないと、後ろの行の
                        // バイト勘定が1つずつずれて、カーソルが本文とずれる
                        sheet.lines.push(Line {
                            cells: Vec::new(), y_mm: y, from_body: true,
                            byte0: para_byte0, cell: None });
                        y += frame.line_height_mm * para.spacing();
                        continue;
                    }
                    // 揃え。**行の幅と行長の差を、どこに置くか**の話でしかない
                    let w: f32 = cells.iter().map(|c| c.w_mm).sum();
                    let slack = (measure - w).max(0.0);
                    let mut x = indent_mm + match para.align {
                        Align::Left | Align::Justify => 0.0,
                        Align::Center => slack / 2.0,
                        Align::Right => slack,
                    };
                    let cells: Vec<Cell> = cells
                        .into_iter()
                        .map(|mut c| { c.x_mm = x; x += c.w_mm; c })
                        .collect();
                    // 行頭の字の段落内位置から、本文の絶対位置を出す。
                    // 箇条書きの印は off=0 で入っているので、最小値を取れば
                    // 1行目(印+本文頭)も続きの行も正しく出る
                    let byte0 = para_byte0
                        + cells.iter().map(|c| c.off).min().unwrap_or(0);
                    sheet.lines.push(Line { cells, y_mm: y, from_body: true, byte0, cell: None });
                    y += frame.line_height_mm * para.spacing();
                }
                // 画像は段落の下に置く。幅が行長を超えるなら比例で縮める
                for im in &para.images {
                    let scale = if im.w_mm > measure { measure / im.w_mm } else { 1.0 };
                    let (w, h) = (im.w_mm * scale, im.h_mm * scale);
                    sheet.images.push((im.bytes.clone(), [indent_mm, y - lh_of(para, frame) * 0.6, w, h]));
                    y += h + frame.line_height_mm * 0.4;
                }
                // 次の段落の頭 = この段落のバイト数 + 改行1つ
                let plen: usize = para.runs.iter().map(|r| r.text.len()).sum();
                para_byte0 += plen + 1;
            }
            Block::Table(table) => {
                y = layout_table(table, m, frame, y, &mut sheet, table_no);
                table_no += 1;
            }
        }
    }
    sheet
}

/// 表を組む。戻り値は表の下の、次のベースライン。
///
/// 列幅は等分(docx の gridCol はまだ読まない — 読めるようになったら差す)。
/// セルの中はそれぞれの幅で普通に折り返す。
fn layout_table(table: &Table, m: &Metrics, frame: &Frame, y_in: f32, sheet: &mut Sheet,
                table_no: usize) -> f32 {
    let ncols = table.rows.iter().map(|r| r.len()).max().unwrap_or(1).max(1);
    // 列幅。指定があればそれを使い、行長に収まらなければ**比例で縮める**
    // (右へ黙ってはみ出すより、比率を守って縮む方が様式の見た目が保たれる)
    let widths: Vec<f32> = if table.col_mm.len() == ncols
        && table.col_mm.iter().all(|w| *w > 0.5)
    {
        let total: f32 = table.col_mm.iter().sum();
        if total > frame.measure_mm {
            let k = frame.measure_mm / total;
            table.col_mm.iter().map(|w| w * k).collect()
        } else {
            table.col_mm.clone()
        }
    } else {
        vec![frame.measure_mm / ncols as f32; ncols]
    };
    // 列の左端(累積)
    let mut xs = vec![0.0f32];
    for w in &widths {
        xs.push(xs.last().unwrap() + w);
    }
    let table_w = *xs.last().unwrap();
    let lh = frame.line_height_mm;

    // 表の上端。直前のベースラインから少し空ける
    let table_top = y_in - lh * 0.55;
    let mut row_top = table_top;

    for (ri, row) in table.rows.iter().enumerate() {
        // 各セルを折って、行の高さを決める。
        // 行はセルの文章(段落を \n で繋いだもの)の中のバイト位置を持つ
        let mut cells_lines: Vec<Vec<(Vec<Cell>, usize)>> = Vec::new();
        let mut nlines = 1usize;
        for (ci, cell) in row.iter().enumerate() {
            let inner = (widths.get(ci).copied().unwrap_or(10.0) - 2.0 * CELL_PAD).max(2.0);
            let mut ls: Vec<(Vec<Cell>, usize)> = Vec::new();
            let mut para0 = 0usize;
            for para in &cell.paragraphs {
                for cs in break_para(para, m, inner, None) {
                    let b0 = para0 + cs.iter().map(|c| c.off).min().unwrap_or(0);
                    ls.push((cs, b0));
                }
                let plen: usize = para.runs.iter().map(|r| r.text.len()).sum();
                para0 += plen + 1;
            }
            nlines = nlines.max(ls.len());
            cells_lines.push(ls);
        }
        let row_h = nlines as f32 * lh + 2.0 * CELL_PAD;

        // 中身を置く(from_body=false。本文の位置合わせに入れない)
        for (ci, ls) in cells_lines.into_iter().enumerate() {
            let x0 = xs[ci] + CELL_PAD;
            let mut yy = row_top + CELL_PAD + lh * 0.8;
            let id = Some((table_no, ri, ci));
            for (cells, b0) in ls {
                let mut x = x0;
                let cells: Vec<Cell> = cells
                    .into_iter()
                    .map(|mut c| { c.x_mm = x; x += c.w_mm; c })
                    .collect();
                sheet.lines.push(Line { cells, y_mm: yy, from_body: false, byte0: b0, cell: id });
                yy += lh;
            }
            // クリックの当たり判定
            sheet.cell_boxes.push(CellBox {
                table: table_no,
                row: ri,
                col: ci,
                x_mm: xs[ci],
                top_mm: row_top,
                w_mm: widths.get(ci).copied().unwrap_or(10.0),
                h_mm: row_h,
            });
        }

        // 罫線: この行の上辺
        sheet.rules.push([0.0, row_top, table_w, row_top]);
        // 縦線(行ごとに引く。ページ割れで途切れても破綻しない)
        for x in &xs {
            sheet.rules.push([*x, row_top, *x, row_top + row_h]);
        }
        row_top += row_h;
    }
    // 一番下の線
    sheet.rules.push([0.0, row_top, table_w, row_top]);

    // 次のベースライン
    row_top + lh
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
        d.blocks.push(Block::Table(Table { col_mm: vec![], rows: vec![vec![Cellbox::default()]] }));
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

#[cfg(test)]
mod list_tests {
    use super::*;

    fn sheet(setup: impl Fn(&mut Document)) -> Sheet {
        let data = font::load(font::for_document(None).unwrap().0).unwrap();
        let m = Metrics::new(&data).unwrap();
        let mut d = Document::plain("一つ目\n二つ目\n三つ目", 10.5);
        setup(&mut d);
        layout(&d, &m, &Frame { measure_mm: 100.0, line_height_mm: 6.0, y0_mm: 20.0 })
    }

    fn text(s: &Sheet, i: usize) -> String {
        s.lines.get(i).map(|l| l.text()).unwrap_or_default()
    }

    #[test]
    fn 箇条書きの印が本文の前に出る() {
        let s = sheet(|d| {
            for b in &mut d.blocks {
                if let Block::Para(p) = b { p.list = ListKind::Bullet }
            }
        });
        assert!(text(&s, 0).starts_with('・'), "印が出ていない: {:?}", text(&s, 0));
    }

    #[test]
    fn 段落番号は連番になる() {
        let s = sheet(|d| {
            for b in &mut d.blocks {
                if let Block::Para(p) = b { p.list = ListKind::Number }
            }
        });
        assert!(text(&s, 0).starts_with("1."), "{:?}", text(&s, 0));
        assert!(text(&s, 1).starts_with("2."), "{:?}", text(&s, 1));
        assert!(text(&s, 2).starts_with("3."), "{:?}", text(&s, 2));
    }

    #[test]
    fn 印は本文を書き換えない() {
        // 編集中の文字位置とずれると、カーソルが合わなくなる
        let mut d = Document::plain("一つ目", 10.5);
        if let Block::Para(p) = &mut d.blocks[0] { p.list = ListKind::Bullet }
        assert_eq!(d.body_text(), "一つ目", "本文に印が混ざった");
    }

    #[test]
    fn インデントで右へ寄る() {
        let plain = sheet(|_| {});
        let ind = sheet(|d| {
            for b in &mut d.blocks {
                if let Block::Para(p) = b { p.indent = 2 }
            }
        });
        assert!(ind.lines[0].cells[0].x_mm > plain.lines[0].cells[0].x_mm + 5.0,
                "インデントが効いていない");
    }

    #[test]
    fn 行間で行が離れる() {
        let plain = sheet(|_| {});
        let wide = sheet(|d| {
            for b in &mut d.blocks {
                if let Block::Para(p) = b { p.line_spacing = 2.0 }
            }
        });
        let gap = |s: &Sheet| s.lines[1].y_mm - s.lines[0].y_mm;
        assert!((gap(&wide) - gap(&plain) * 2.0).abs() < 0.1,
                "行間が倍になっていない: {} → {}", gap(&plain), gap(&wide));
    }

    #[test]
    fn インデントすると行長が縮む() {
        // 右端がはみ出さないこと
        let data = font::load(font::for_document(None).unwrap().0).unwrap();
        let m = Metrics::new(&data).unwrap();
        let long = "あ".repeat(60);
        let mut d = Document::plain(&long, 10.5);
        if let Block::Para(p) = &mut d.blocks[0] { p.indent = 3 }
        let s = layout(&d, &m, &Frame { measure_mm: 100.0, line_height_mm: 6.0, y0_mm: 20.0 });
        for l in &s.lines {
            let right = l.cells.last().map(|c| c.x_mm + c.w_mm).unwrap_or(0.0);
            assert!(right <= 100.5, "行長を超えた: {right}mm");
        }
    }
}

#[cfg(test)]
mod table_layout_tests {
    use super::*;

    fn doc_with_table() -> Document {
        let cell = |s: &str| Cellbox {
            paragraphs: vec![Paragraph {
                runs: vec![Run {
                    text: s.into(), size_pt: 10.5, font: None, fmt: Default::default() }],
                ..Default::default()
            }],
        };
        let mut d = Document::plain("前の本文", 10.5);
        d.blocks.push(Block::Table(Table {
            col_mm: vec![],
            rows: vec![
                vec![cell("品名"), cell("金額")],
                vec![cell("防火戸"), cell("120,000")],
            ],
        }));
        d
    }

    fn sheet() -> Sheet {
        let data = font::load(font::for_document(None).unwrap().0).unwrap();
        let m = Metrics::new(&data).unwrap();
        layout(&doc_with_table(), &m,
               &Frame { measure_mm: 100.0, line_height_mm: 6.0, y0_mm: 20.0 })
    }

    #[test]
    fn 表の中身が紙面に出る() {
        let s = sheet();
        let all: String = s.lines.iter().map(|l| l.text()).collect();
        assert!(all.contains("品名"), "表のセルが描かれていない");
        assert!(all.contains("防火戸"));
    }

    #[test]
    fn 表の行は本文由来ではない() {
        // カーソルの位置合わせを壊さないための区別
        let s = sheet();
        let body: Vec<&Line> = s.lines.iter().filter(|l| l.from_body).collect();
        assert_eq!(body.len(), 1, "本文の行数が違う: {}", body.len());
        assert!(body[0].text().contains("前の本文"));
        assert!(s.lines.iter().any(|l| !l.from_body), "表の行が無い");
    }

    #[test]
    fn 罫線が引かれる() {
        let s = sheet();
        // 2行の表: 横線3本 + 縦線(3本×2行) = 9本
        assert_eq!(s.rules.len(), 9, "罫線の数が違う: {}", s.rules.len());
        // 横線は行長いっぱい
        let h: Vec<_> = s.rules.iter().filter(|r| r[1] == r[3]).collect();
        assert_eq!(h.len(), 3);
        assert!(h.iter().all(|r| (r[2] - r[0] - 100.0).abs() < 0.01));
    }

    #[test]
    fn セルの中で折り返す() {
        let cell = |s: &str| Cellbox {
            paragraphs: vec![Paragraph {
                runs: vec![Run {
                    text: s.into(), size_pt: 10.5, font: None, fmt: Default::default() }],
                ..Default::default()
            }],
        };
        let mut d = Document { font: None, blocks: vec![] };
        d.blocks.push(Block::Table(Table {
            col_mm: vec![],
            rows: vec![vec![cell(&"あ".repeat(30)), cell("短い")]],
        }));
        let data = font::load(font::for_document(None).unwrap().0).unwrap();
        let m = Metrics::new(&data).unwrap();
        let s = layout(&d, &m, &Frame { measure_mm: 100.0, line_height_mm: 6.0, y0_mm: 20.0 });
        // 50mm の列に 30文字(約110mm)は3行になる
        let cell_lines = s.lines.iter().filter(|l| !l.from_body).count();
        assert!(cell_lines >= 3, "セルの中で折り返していない: {cell_lines} 行");
        // 右のセルにはみ出さない
        for l in s.lines.iter().filter(|l| !l.from_body) {
            if l.text().starts_with('あ') {
                let right = l.cells.last().map(|c| c.x_mm + c.w_mm).unwrap_or(0.0);
                assert!(right <= 50.0 + 0.5, "隣のセルへはみ出した: {right}mm");
            }
        }
    }
}

#[cfg(test)]
mod gridcol_tests {
    use super::*;

    fn cell(s: &str) -> Cellbox {
        Cellbox {
            paragraphs: vec![Paragraph {
                runs: vec![Run {
                    text: s.into(), size_pt: 10.5, font: None, fmt: Default::default() }],
                ..Default::default()
            }],
        }
    }

    fn rules_of(col_mm: Vec<f32>) -> Vec<[f32; 4]> {
        let data = font::load(font::for_document(None).unwrap().0).unwrap();
        let m = Metrics::new(&data).unwrap();
        let d = Document {
            font: None,
            blocks: vec![Block::Table(Table {
                col_mm,
                rows: vec![vec![cell("項目"), cell("値")]],
            })],
        };
        layout(&d, &m, &Frame { measure_mm: 100.0, line_height_mm: 6.0, y0_mm: 20.0 }).rules
    }

    #[test]
    fn 列幅の指定が効く() {
        // 30mm + 70mm の2列。縦線が 0, 30, 100 に立つ
        let rules = rules_of(vec![30.0, 70.0]);
        let mut vx: Vec<f32> = rules.iter().filter(|r| r[0] == r[2]).map(|r| r[0]).collect();
        vx.sort_by(f32::total_cmp);
        vx.dedup_by(|a, b| (*a - *b).abs() < 0.01);
        assert_eq!(vx.len(), 3, "{vx:?}");
        assert!((vx[1] - 30.0).abs() < 0.01, "指定した列幅で立っていない: {vx:?}");
    }

    #[test]
    fn 行長を超える指定は比例で縮む() {
        // 120+80=200mm を 100mm に。比率 3:2 のまま 60/40 になる
        let rules = rules_of(vec![120.0, 80.0]);
        let mut vx: Vec<f32> = rules.iter().filter(|r| r[0] == r[2]).map(|r| r[0]).collect();
        vx.sort_by(f32::total_cmp);
        vx.dedup_by(|a, b| (*a - *b).abs() < 0.01);
        assert!((vx[1] - 60.0).abs() < 0.1, "比率が守られていない: {vx:?}");
        assert!((vx[2] - 100.0).abs() < 0.1, "右へはみ出した: {vx:?}");
    }
}

#[cfg(test)]
mod empty_line_tests {
    use super::*;

    #[test]
    fn 空の段落も行として持つ() {
        // 持たないと、後ろの行のバイト勘定がずれてカーソルが合わなくなる
        let data = font::load(font::for_document(None).unwrap().0).unwrap();
        let m = Metrics::new(&data).unwrap();
        let d = Document::plain("一行目\n\n三行目", 10.5);
        let s = layout(&d, &m, &Frame { measure_mm: 100.0, line_height_mm: 6.0, y0_mm: 20.0 });
        let body: Vec<&Line> = s.lines.iter().filter(|l| l.from_body).collect();
        assert_eq!(body.len(), 3, "空行が消えた: {} 行", body.len());
        assert!(body[1].cells.is_empty());
        // 3行目は2行ぶん下にある
        assert!((body[2].y_mm - body[0].y_mm - 12.0).abs() < 0.01);
    }
}

#[cfg(test)]
mod byte0_tests {
    use super::*;

    fn lines(text: &str, measure: f32) -> Vec<Line> {
        let data = font::load(font::for_document(None).unwrap().0).unwrap();
        let m = Metrics::new(&data).unwrap();
        let d = Document::plain(text, 10.5);
        layout(&d, &m, &Frame { measure_mm: measure, line_height_mm: 6.0, y0_mm: 20.0 })
            .lines
    }

    #[test]
    fn 折り返しても行のバイト位置が本文と合う() {
        // 「行の文字数 + 1」で数えると、折り返した行の数だけずれていた
        let text = "あ".repeat(40); // 100mm に入らないので折り返す
        let ls = lines(&text, 100.0);
        assert!(ls.len() >= 2, "折り返していない");
        for l in &ls {
            // byte0 の位置の字が、その行の先頭の字と一致する
            let head = text[l.byte0..].chars().next().unwrap();
            assert_eq!(head, l.cells[0].ch, "byte0 がずれている");
        }
        // 連結すると本文に戻る(空白落ちのない文)
        let total: usize = ls.iter().map(|l| l.byte_end() - l.byte0).sum();
        assert_eq!(total, text.len());
    }

    #[test]
    fn 空白が落ちてもずれない() {
        // 行末で捨てた空白のぶん、次の行の byte0 が進んでいること
        let text = format!("{} {}", "a".repeat(40), "b".repeat(40));
        let ls = lines(&text, 60.0);
        assert!(ls.len() >= 2);
        let l2 = &ls[1];
        let head = text[l2.byte0..].chars().next().unwrap();
        assert_eq!(head, l2.cells[0].ch, "落ちた空白の勘定が入っていない");
    }

    #[test]
    fn 段落をまたいでも合う() {
        let text = "一つ目\n二つ目の段落\n三";
        let ls = lines(text, 100.0);
        for l in &ls {
            if l.cells.is_empty() { continue }
            let head = text[l.byte0..].chars().next().unwrap();
            assert_eq!(head, l.cells[0].ch);
        }
    }

    #[test]
    fn 箇条書きの印はバイト位置に入らない() {
        let data = font::load(font::for_document(None).unwrap().0).unwrap();
        let m = Metrics::new(&data).unwrap();
        let mut d = Document::plain("項目", 10.5);
        if let Block::Para(p) = &mut d.blocks[0] { p.list = ListKind::Bullet }
        let s = layout(&d, &m, &Frame { measure_mm: 100.0, line_height_mm: 6.0, y0_mm: 20.0 });
        let l = &s.lines[0];
        assert_eq!(l.byte0, 0);
        // 印(・)ぶんが byte_end に乗っていない
        assert_eq!(l.byte_end(), "項目".len(), "印が本文のバイトに混ざった");
    }
}

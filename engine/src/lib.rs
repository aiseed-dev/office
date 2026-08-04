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
pub mod html;
pub use edit::Editor;

use ttf_parser::Face;

// ---------- 文書モデル ----------

/// 相互参照(docx の REF / PAGEREF フィールド)。**run 1つが1つの参照**で、
/// run の text は「いま見えている値」(更新で計算し直す)。
/// 編集で参照の中を割ったら、参照は普通の文字に降りる(予測できる形で壊す)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefField {
    /// 指す先(しおりの名前)
    pub name: String,
    /// true ならページ番号(PAGEREF)、false ならしおりの文字(REF)
    pub page: bool,
}

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
    /// 相互参照。ここ(書式)に持つのは、run の分割・結合・描画の
    /// 既存の道具立てがそのまま使えるため(field が違えば繋がらない)
    pub field: Option<RefField>,
    /// ルビ(ふりがな)。この run の字の上に半分の大きさで振る。
    /// field と同じ理由でここに持つ — run の切り貼りが面倒を見てくれる
    pub ruby: Option<String>,
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
    /// 均等割付(docx の distribute)。**最後の行も**行長いっぱいに
    /// 字間を配る — 両端揃えとの違いはそこ。見出しや表の項目名の作法
    Distribute,
}

impl Align {
    /// docx の `w:jc w:val` の値
    pub fn as_docx(self) -> &'static str {
        match self {
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
            Align::Justify => "both",
            Align::Distribute => "distribute",
        }
    }
    pub fn from_docx(v: &str) -> Align {
        match v {
            "center" => Align::Center,
            "right" | "end" => Align::Right,
            "both" => Align::Justify,
            "distribute" => Align::Distribute,
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

/// 段落の役割(docx の `w:pStyle` の最小対応)。
///
/// 見出しは目次の材料。目次の行は「目次の更新」で作り直すための印。
/// スタイル定義(styles.xml)は持たない — 見た目は直接書式で付ける。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ParaStyle {
    #[default]
    Body,
    /// 見出し(1〜3)。docx の Heading1〜3 / outlineLvl
    Heading(u8),
    /// 目次の行(1〜3)。docx の TOC1〜3(このアプリが目次を作った印)
    Toc(u8),
    /// 図表目次の行(docx の TableofFigures。「図表目次の更新」の印)
    Tof,
}

/// 段落に付くコメント(docx の comments.xml)。**段落単位**で持つ
/// (文中の範囲は持たない — 2026-08-03 発注者確認の粒度)。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Comment {
    pub author: String,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct Paragraph {
    pub runs: Vec<Run>,
    /// 段落の役割(見出し・目次の行)。既定は本文
    pub style: ParaStyle,
    /// この段落に付いたコメント
    pub comments: Vec<Comment>,
    /// この段落に付いたしおり(docx の bookmarkStart の名前)。
    /// 段落単位で持つ(範囲は段落まるごと — コメントと同じ粒度)
    pub bookmarks: Vec<String>,
    /// 読めなかった要素(画像など)の原文。**理解はしないが、捨てない。**
    /// 保存でそのまま返す
    pub anchors: Vec<String>,
    /// 表示できる画像(anchors のうち、絵の実体と大きさが分かったもの)。
    /// 保存には使わない — 保存は anchors の原文が担う
    pub images: Vec<InlineImage>,
    /// **このアプリで挿した**画像。保存でこちらが部品(media・rels)ごと書き出す。
    /// 読み込んだ画像(anchors 由来)とは持ち場が違う — 混ぜると保存で二重になる
    pub images_new: Vec<InlineImage>,
    pub align: Align,
    /// この段落の前で改ページする(docx の w:pageBreakBefore)
    pub page_break_before: bool,
    pub list: ListKind,
    /// 左のインデント段数。1段 = 全角2文字ぶん(日本の書類の慣習)
    pub indent: u8,
    /// 行間の倍率。1.0 が既定
    pub line_spacing: f32,
    /// 段落の背景色 `RRGGBB`(docx の w:shd)。見出しの帯に使われる
    pub shade: Option<String>,
    /// 段落を枠で囲む(docx の w:pBdr)。囲みの注意書きに使われる
    pub boxed: bool,
    /// ドロップキャップ(頭の1字を大きく)。docx では w:framePr の
    /// 「枠の段落+本文の段落」に割れるが、モデルでは1つの段落で持つ
    pub dropcap: bool,
}

impl Paragraph {
    /// 行間の倍率。0 や負が入っていても壊れない値を返す。
    pub fn spacing(&self) -> f32 {
        if self.line_spacing <= 0.0 { 1.0 } else { self.line_spacing.clamp(0.5, 5.0) }
    }

    /// 箇条書きの頭に付く印。組版のときに本文の前へ置く。
    /// **レベル(インデント)で印が変わる**(Word の複数レベルのリストの慣習)。
    pub fn marker(&self, nth: usize) -> Option<String> {
        match self.list {
            ListKind::None => None,
            ListKind::Bullet => Some(
                match self.indent % 3 {
                    0 => "・",
                    1 => "○",
                    _ => "■",
                }
                .into(),
            ),
            ListKind::Number => Some(match self.indent {
                0 => format!("{}. ", nth + 1),
                1 => format!("({}) ", nth + 1),
                _ => format!("{}) ", nth + 1),
            }),
        }
    }
}

/// 縦のセル結合(docx の w:vMerge)。
/// **日本の様式は結合で見出しを作る**ので、読めないと枠がずれて出る。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VMerge {
    #[default]
    None,
    /// 結合の始まり(w:vMerge w:val="restart")。下の Continue を呑み込む
    Start,
    /// 結合の続き。上のセルに呑まれており、中身は描かれない
    Continue,
}

/// 表の1セル。中は段落の列(セルの中にも段落がある)
#[derive(Debug, Clone, Default)]
pub struct Cellbox {
    pub paragraphs: Vec<Paragraph>,
    /// 横の結合(docx の w:gridSpan)。このセルが占める格子の列数。
    /// 0 と 1 はどちらも「結合なし」(既定の 0 を特別扱いしない)
    pub col_span: u8,
    /// 縦の結合
    pub v_merge: VMerge,
}

impl Cellbox {
    /// 占める格子の列数(最低1)。
    pub fn span(&self) -> usize {
        (self.col_span as usize).max(1)
    }
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

/// 用紙の設定(mm)。docx の `w:pgSz` / `w:pgMar` / `w:cols`。
#[derive(Debug, Clone, Copy)]
pub struct PageSetup {
    pub w_mm: f32,
    pub h_mm: f32,
    pub left_mm: f32,
    pub right_mm: f32,
    pub top_mm: f32,
    pub bottom_mm: f32,
    /// 段組みの段数(docx の w:cols w:num)。1 が普通の1段
    pub columns: u8,
}

impl Default for PageSetup {
    fn default() -> Self {
        // A4 縦・余白 20mm(日本の事務の慣行に近い値)
        PageSetup { w_mm: 210.0, h_mm: 297.0, left_mm: 20.0, right_mm: 20.0,
                    top_mm: 20.0, bottom_mm: 20.0, columns: 1 }
    }
}

/// 段の間(mm)。Word の既定(425twip ≒ 7.5mm)に合わせる
pub const COLUMN_GAP_MM: f32 = 7.5;

impl PageSetup {
    /// 行長(本文の幅)
    pub fn measure_mm(&self) -> f32 {
        (self.w_mm - self.left_mm - self.right_mm).max(10.0)
    }

    /// 段数(0 が入っていても壊れない)
    pub fn cols(&self) -> usize {
        (self.columns as usize).clamp(1, 8)
    }

    /// 1段の行長。段組みなら段の間を除いて割る
    pub fn column_measure_mm(&self) -> f32 {
        let n = self.cols() as f32;
        ((self.measure_mm() - COLUMN_GAP_MM * (n - 1.0)) / n).max(10.0)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Document {
    /// 本文の流れ(段落と表が混ざる)
    pub blocks: Vec<Block>,
    /// 文書の既定の書体(docx の `w:docDefaults`)。
    /// 段落側が指定していなければこれを使う
    pub font: Option<String>,
    /// 用紙の設定。無ければ既定(A4)
    pub page: Option<PageSetup>,
    /// 節の設定の原文(w:sectPr)。ヘッダーの参照などが入っているので、
    /// **理解はしないが捨てない**。保存でそのまま返す
    pub sect_raw: Option<String>,
    /// ヘッダー(docx の headerN.xml)。全ページ同じもの(type="default")だけを持つ
    pub header: HeadFoot,
    /// フッター(docx の footerN.xml)
    pub footer: HeadFoot,
    /// ページの色 `RRGGBB`(docx の w:background)。画面も紙も同じ色に塗る
    pub page_color: Option<String>,
    /// 透かし(斜めの薄い字)。docx ではヘッダーの中の VML の図形になる
    pub watermark: Option<String>,
    /// 手描きの線(描画タブのペン)。docx では自由曲線の図形になる
    pub ink: Vec<Stroke>,
    /// 変更履歴の書き手(w:ins / w:del の author)。
    /// 保存用の写しにだけ入る — 印([`TRK_INS_S`] 等)と対で使う
    pub track_author: Option<String>,
    /// 欧文のハイフネーション(docx の settings の autoHyphenation)。
    /// 日本語には掛からない(禁則で折る)。英語の語を音節で折って - を付ける
    pub hyphenate: bool,
    /// 文書の保護(docx の settings の documentProtection の w:edit)。
    /// Some("readOnly") なら読み取り専用。パスワード無しの保護は Word と
    /// 同じく「注意書き」— 解除の釦で誰でも外せる(そう見せる)
    pub protection: Option<String>,
    /// 文書の情報(docx の docProps/core.xml)。作成者・タイトルなど
    pub props: CoreProps,
    /// 縦書き(docx の sectPr の textDirection=tbRl)。
    /// 組みは fold_vertical が行を右からの列へ写す(K4)
    pub vertical: bool,
}

/// 文書の情報(core properties)。空の欄は書かない
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CoreProps {
    pub creator: String,
    pub title: String,
    pub keywords: String,
    pub subject: String,
    pub description: String,
}

/// 変更履歴の印。保存用の写しの本文に埋め、docx の w:ins / w:del になる。
/// PAGE の印と同じ作法(私用領域の字。画面には出さない)。
pub const TRK_INS_S: char = '\u{E010}';
/// 挿入の終わり
pub const TRK_INS_E: char = '\u{E011}';
/// 削除の始まり(中の字は w:delText — 消された字として残る)
pub const TRK_DEL_S: char = '\u{E012}';
/// 削除の終わり
pub const TRK_DEL_E: char = '\u{E013}';

/// 手描きの1筆。座標は**そのページの中**の mm(紙の左上が原点)。
/// ページに貼り付く(本文を編集しても動かない)— Word のページ固定の図形と同じ。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stroke {
    /// 何ページ目か(0始まり)
    pub page: usize,
    /// 蛍光ペンか(太く・薄く・文字の下に描く)
    pub highlighter: bool,
    /// 筆の通り道(mm)
    pub points: Vec<(f32, f32)>,
}

/// ヘッダー・フッター(1節ぶん)。
///
/// **paragraphs が空 = 持っていない(または編集できない)。**
/// 空のヘッダーを表すときは、空のランを持つ段落を1つ置く —
/// この区別で「触っていない部品を保存で書き潰さない」を守る。
#[derive(Debug, Clone, Default)]
pub struct HeadFoot {
    pub paragraphs: Vec<Paragraph>,
    /// 読み込んだ docx での部品名(`word/header1.xml` 等)。
    /// 保存で同じ部品へ書き戻す。None で paragraphs があれば新しい部品を作る
    pub part: Option<String>,
}

/// ページ番号の印(docx の PAGE フィールド)。ヘッダー・フッターの文中に
/// この1字で置き、組むとき([`layout_hf`])にそのページの番号の字になる。
/// 私用領域の字なので普通の本文と衝突しない。
pub const PAGE_MARK: char = '\u{E000}';

/// ページ数(総頁)の印(docx の NUMPAGES フィールド)。扱いは [`PAGE_MARK`] と同じ。
pub const PAGES_MARK: char = '\u{E001}';

/// 段落の列を編集用の平文にする(区切りは改行)。セル・ヘッダーの編集で使う。
pub fn paras_text(paras: &[Paragraph]) -> String {
    paras
        .iter()
        .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 平文を段落の列へ戻す。同じ位置の段落から書式を引き継ぐ
/// (本文の `set_body_text` と同じ規則 — 段落をまるごと写す)。
pub fn set_paras_text(paras: &mut Vec<Paragraph>, text: &str, size_pt: f32) {
    let old = std::mem::take(paras);
    *paras = text
        .split('\n')
        .enumerate()
        .map(|(i, s)| {
            let mut p = old.get(i).cloned().unwrap_or_default();
            let (pt, font, fmt) = p
                .runs
                .first()
                .map(|r| (r.size_pt, r.font.clone(), r.fmt.clone()))
                .unwrap_or((size_pt, None, CharFormat::default()));
            p.runs = vec![Run { text: s.to_string(), size_pt: pt, font, fmt }];
            p
        })
        .collect();
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

    /// 編集後のテキストを本文へ写す。
    ///
    /// **1回の編集は、連続した1箇所の置き換え**(打鍵・削除・貼り付け・
    /// IME の確定・undo の1手、すべてそう)。だから頭と尻尾の共通部分を
    /// 除けば、その1箇所が分かる。そこだけを run の構造に写すので、
    /// **run の境(部分書式)・段落の性質・表の位置が編集で流されない**
    /// (以前の「段落番号で写す」方式は、段落の増減で下の性質がずれ、
    /// 表が末尾へ動いていた)。
    pub fn set_body_text(&mut self, text: &str, size_pt: f32) {
        let old = self.body_text();
        if old == text {
            return;
        }
        let (ob, nb) = (old.as_bytes(), text.as_bytes());
        // 共通の頭(文字の境に合わせる)
        let mut pre = ob.iter().zip(nb).take_while(|(a, b)| a == b).count();
        while pre > 0 && !(old.is_char_boundary(pre) && text.is_char_boundary(pre)) {
            pre -= 1;
        }
        // 共通の尻尾(頭と重ならない範囲で)
        let max_suf = (ob.len() - pre).min(nb.len() - pre);
        let mut suf = ob
            .iter()
            .rev()
            .zip(nb.iter().rev())
            .take_while(|(a, b)| a == b)
            .count()
            .min(max_suf);
        while suf > 0
            && !(old.is_char_boundary(ob.len() - suf) && text.is_char_boundary(nb.len() - suf))
        {
            suf -= 1;
        }
        self.splice_text(pre, ob.len() - suf, &text[pre..nb.len() - suf], size_pt);
    }

    /// 本文の `start..end`(バイト。段落は \n で繋いだ物差し)を `insert` で
    /// 置き換える。run の境と段落の性質を保つ、編集モデルの心臓。
    pub fn splice_text(&mut self, start: usize, end: usize, insert: &str, size_pt: f32) {
        // 段落ごとの(blocks の位置, 本文での頭, 長さ)
        let mut paras: Vec<(usize, usize, usize)> = Vec::new();
        let mut at = 0usize;
        for (bi, b) in self.blocks.iter().enumerate() {
            if let Block::Para(p) = b {
                let len: usize = p.runs.iter().map(|r| r.text.len()).sum();
                paras.push((bi, at, len));
                at += len + 1;
            }
        }
        if paras.is_empty() {
            self.blocks.push(Block::Para(Paragraph {
                line_spacing: 1.0,
                runs: vec![Run { text: insert.to_string(), size_pt, font: None,
                                 fmt: CharFormat::default() }],
                ..Default::default()
            }));
            return;
        }
        // 置き換えの端が入る段落。段落の末尾(= \n の位置)も、その段落に数える
        let pi_of = |pos: usize| -> usize {
            paras
                .iter()
                .rposition(|(_, s, _)| *s <= pos)
                .unwrap_or(0)
        };
        let (ps, pe) = (pi_of(start), pi_of(end));
        let (bi_s, s0, _) = paras[ps];
        let (bi_e, e0, e_len) = paras[pe];
        let off_s = (start - s0).min(paras[ps].2);
        let off_e = (end - e0).min(e_len);

        let head_para = match &self.blocks[bi_s] {
            Block::Para(p) => p.clone(),
            _ => unreachable!(),
        };
        let tail_para = match &self.blocks[bi_e] {
            Block::Para(p) => p.clone(),
            _ => unreachable!(),
        };
        let (head_runs, _) = split_runs(&head_para.runs, off_s);
        let (_, tail_runs) = split_runs(&tail_para.runs, off_e);
        // 差し込む字の書式は、置き換えの直前の字のもの(無ければ直後、無ければ既定)
        let (ins_pt, ins_font, ins_fmt) = head_runs
            .iter()
            .rev()
            .find(|r| !r.text.is_empty())
            .or_else(|| head_para.runs.first())
            .map(|r| (r.size_pt, r.font.clone(), r.fmt.clone()))
            .unwrap_or((size_pt, None, CharFormat::default()));
        let ins_run = |t: &str| {
            let mut fmt = ins_fmt.clone();
            // 参照(フィールド)の直後に打った字は参照の一部ではない。
            // ルビも同じ(打った字に読みは付いてこない)
            fmt.field = None;
            fmt.ruby = None;
            Run { text: t.to_string(), size_pt: ins_pt, font: ins_font.clone(), fmt }
        };

        // 新しい段落の列を組み立てる
        let segs: Vec<&str> = insert.split('\n').collect();
        let mut out: Vec<Paragraph> = Vec::new();
        if segs.len() == 1 {
            // 段落の増減なし(または段落の合流)。head + 差し込み + tail が1つに
            let mut p = head_para.clone();
            p.runs = head_runs;
            p.runs.push(ins_run(segs[0]));
            p.runs.extend(tail_runs);
            // 合流したら、消えた側の性質は頭の段落に呑まれる
            out.push(p);
        } else {
            let mut first = head_para.clone();
            first.runs = head_runs;
            first.runs.push(ins_run(segs[0]));
            out.push(first);
            for seg in &segs[1..segs.len() - 1] {
                let mut mid = head_para.clone();
                mid.anchors = Vec::new();
                mid.images = Vec::new();
                mid.images_new = Vec::new();
                mid.comments = Vec::new();
                mid.bookmarks = Vec::new();
                mid.runs = vec![ins_run(seg)];
                out.push(mid);
            }
            // 最後の段落は、置き換えの尻の段落の性質を継ぐ(Enter で割った
            // 後ろ半分が、元の段落の性質のまま残る形)
            let mut last = tail_para.clone();
            last.runs = vec![ins_run(segs[segs.len() - 1])];
            last.runs.extend(tail_runs);
            if ps == pe {
                // 1つの段落を割ったときは、控え(画像など)は前半にだけ残す
                last.anchors = Vec::new();
                last.images = Vec::new();
                last.images_new = Vec::new();
                last.comments = Vec::new();
                last.bookmarks = Vec::new();
            }
            out.push(last);
        }
        for p in &mut out {
            normalize_runs(&mut p.runs, size_pt);
        }
        // 置き換えの範囲に挟まっていた表などは、後ろへ避けて残す(消さない)
        let kept: Vec<Block> = self.blocks[bi_s..=bi_e]
            .iter()
            .filter(|b| !matches!(b, Block::Para(_)))
            .cloned()
            .collect();
        let mut new_blocks: Vec<Block> = out.into_iter().map(Block::Para).collect();
        new_blocks.extend(kept);
        self.blocks.splice(bi_s..=bi_e, new_blocks);
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

    /// 選択範囲の run に手を入れる(部分書式の心臓)。
    /// 範囲の境で run を割り、中の run にだけ f を掛ける。
    /// 端が字の途中に掛かっていたら、その字はまるごと含める。
    fn apply_runs(&mut self, range: std::ops::Range<usize>, f: impl Fn(&mut Run)) {
        let body = self.body_text();
        let mut start = range.start.min(body.len());
        while start > 0 && !body.is_char_boundary(start) {
            start -= 1;
        }
        let mut end = range.end.min(body.len());
        while end < body.len() && !body.is_char_boundary(end) {
            end += 1;
        }
        let mut at = 0usize;
        for b in &mut self.blocks {
            let Block::Para(p) = b else { continue };
            let len: usize = p.runs.iter().map(|r| r.text.len()).sum();
            let (ps, pe) = (at, at + len);
            at = pe + 1;
            if pe < start || ps > end {
                continue;
            }
            let ls = start.saturating_sub(ps).min(len);
            let le = end.saturating_sub(ps).min(len);
            if ls >= le {
                continue;
            }
            let (left, rest) = split_runs(&p.runs, ls);
            let (mut mid, right) = split_runs(&rest, le - ls);
            for r in &mut mid {
                f(r);
            }
            let pt = p.runs.first().map(|r| r.size_pt).unwrap_or(10.5);
            p.runs = left;
            p.runs.extend(mid);
            p.runs.extend(right);
            normalize_runs(&mut p.runs, pt);
        }
    }

    /// 選択範囲の文字書式を変える。**選択の字にだけ掛かる**(run を割る)。
    /// 選択が空(カーソルだけ)のときは、その段落まるごと — 従来の作法。
    pub fn apply_char_format(
        &mut self,
        range: std::ops::Range<usize>,
        f: impl Fn(&mut CharFormat),
    ) {
        if range.start < range.end {
            self.apply_runs(range, |r| f(&mut r.fmt));
            return;
        }
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

    /// 選択範囲の文字の大きさを変える(選択の字にだけ。空なら段落まるごと)。
    ///
    /// 上限と下限を持つ — **際限なく大きく/小さくできると事故になる**
    /// (0pt にすると本文が消え、原因が分からなくなる)。
    pub fn apply_size(&mut self, range: std::ops::Range<usize>, f: impl Fn(f32) -> f32) {
        if range.start < range.end {
            self.apply_runs(range, |r| r.size_pt = f(r.size_pt).clamp(4.0, 400.0));
            return;
        }
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

    /// 選択範囲の書体を変える(選択の字にだけ。空なら段落まるごと)。
    pub fn apply_font(&mut self, range: std::ops::Range<usize>, name: Option<String>) {
        if range.start < range.end {
            self.apply_runs(range, |r| r.font = name.clone());
            return;
        }
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

    /// 範囲に相互参照を付ける(挿した値の run を参照にする)・外す。
    pub fn apply_field(&mut self, range: std::ops::Range<usize>, field: Option<RefField>) {
        self.apply_runs(range, |r| r.fmt.field = field.clone());
    }

    /// 本文の中の参照を数え、値を計算し直す。`value(名前, ページか)` が
    /// 新しい値を返す(計算できなければ None = 触らない)。
    /// 返り値は書き換えた数。**run の text を直に書き換える**ので、
    /// 編集中の平文を持つ側(writer)は呼んだ後に同期し直すこと。
    pub fn refresh_fields(
        &mut self,
        value: impl Fn(&str, bool) -> Option<String>,
    ) -> usize {
        let mut n = 0usize;
        for b in &mut self.blocks {
            let Block::Para(p) = b else { continue };
            for r in &mut p.runs {
                let Some(f) = &r.fmt.field else { continue };
                if let Some(v) = value(&f.name, f.page) {
                    if v != r.text {
                        r.text = v;
                        n += 1;
                    }
                }
            }
        }
        n
    }

    /// 選択範囲(の頭)の位置にある run。書式の表示・読み取りの元。
    fn run_at(&self, pos: usize) -> Option<&Run> {
        let mut at = 0usize;
        for p in self.paragraphs() {
            let len: usize = p.runs.iter().map(|r| r.text.len()).sum();
            if pos <= at + len {
                let off = pos - at;
                // カーソルの**直前の字**の run(打つとその書式になる、の慣習)。
                // 段落の頭では最初の run
                let mut racc = 0usize;
                for r in &p.runs {
                    let rend = racc + r.text.len();
                    if off <= rend && (off > racc || racc == 0) {
                        return Some(r);
                    }
                    racc = rend;
                }
                return p.runs.first();
            }
            at += len + 1;
        }
        None
    }

    /// いま選択範囲の文字の大きさ。
    pub fn size_at(&self, range: std::ops::Range<usize>) -> Option<f32> {
        self.run_at(range.start).map(|r| r.size_pt)
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
    /// カーソルの位置の run の書式を返す。
    pub fn char_format_at(&self, range: std::ops::Range<usize>) -> CharFormat {
        self.run_at(range.start).map(|r| r.fmt.clone()).unwrap_or_default()
    }

    /// いま選択範囲の揃え。
    pub fn align_at(&self, range: std::ops::Range<usize>) -> Align {
        let target = self.para_range(range);
        self.paragraphs().nth(target.start).map(|p| p.align).unwrap_or_default()
    }
}

/// run の列を byte で二つに割る(境に掛かる run はそこで切る)。
fn split_runs(runs: &[Run], byte: usize) -> (Vec<Run>, Vec<Run>) {
    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut at = 0usize;
    for r in runs {
        let end = at + r.text.len();
        if end <= byte {
            left.push(r.clone());
        } else if at >= byte {
            right.push(r.clone());
        } else {
            let cut = byte - at;
            let mut a = r.clone();
            a.text = r.text[..cut].to_string();
            let mut b = r.clone();
            b.text = r.text[cut..].to_string();
            // 参照(フィールド)の中を割った = 手で書き換えた。
            // 半分ずつが参照を名乗ると更新で二重になるので、普通の字に降ろす。
            // ルビも同じ(半分の基底に同じ読みが二重に付くのを防ぐ)
            a.fmt.field = None;
            b.fmt.field = None;
            a.fmt.ruby = None;
            b.fmt.ruby = None;
            left.push(a);
            right.push(b);
        }
        at = end;
    }
    (left, right)
}

/// 空の run を除き、同じ書式の隣り合う run を繋ぐ(編集で際限なく増やさない)。
/// 全部空なら、空の run を1つ残す(空の段落の形)。
fn normalize_runs(runs: &mut Vec<Run>, size_pt: f32) {
    let mut out: Vec<Run> = Vec::new();
    for r in runs.drain(..) {
        if r.text.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some(last)
                if last.size_pt == r.size_pt && last.font == r.font && last.fmt == r.fmt =>
            {
                last.text.push_str(&r.text);
            }
            _ => out.push(r),
        }
    }
    if out.is_empty() {
        out.push(Run { text: String::new(), size_pt, font: None,
                       fmt: CharFormat::default() });
    }
    *runs = out;
}

impl Document {
    pub fn plain(text: &str, size_pt: f32) -> Document {
        Document {
            font: None,
            page: None,
            sect_raw: None, header: Default::default(), footer: Default::default(), page_color: None, watermark: None, ink: Vec::new(), track_author: None, hyphenate: false, protection: None, props: Default::default(), vertical: false,
            blocks: text
                .split('\n')
                .map(|p| Block::Para(Paragraph {
                    line_spacing: 1.0,
                    shade: None, boxed: false, images_new: Vec::new(), runs: vec![Run { text: p.to_string(), size_pt, font: None, fmt: Default::default() }],
                    ..Default::default() }))
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
    /// この字の書体(run の指定。None は文書の既定)。
    /// 行の中で書体が混ざっても、描く側が連なりごとに切り替えられる
    pub font: Option<String>,
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
    /// 縦書きか。真なら vert_x が行ごとの「列の左肩の x(絶対 mm)」を持ち、
    /// Cell.x_mm は「上からの距離」の意味になる。描く側は1字ずつ置く
    pub vertical: bool,
    pub vert_x: Vec<f32>,
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

pub mod atomic;
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
    // (字, 幅mm, サイズpt, 書式, 書体, バイト位置)
    One(char, f32, f32, CharFormat, Option<String>, usize),
    // (字と幅と位置の列, サイズpt, 書式, 書体)
    Word(Vec<(char, f32, usize)>, f32, CharFormat, Option<String>),
    Space(f32, f32, CharFormat, Option<String>, usize),
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
                out.push(Tok::Word(std::mem::take(&mut word), run.size_pt, run.fmt.clone(),
                                   run.font.clone()));
            }
            if ch == ' ' || ch == '\u{3000}' {
                out.push(Tok::Space(m.advance_mm(ch, run.size_pt), run.size_pt,
                                    run.fmt.clone(), run.font.clone(), off));
            } else {
                out.push(Tok::One(ch, m.advance_mm(ch, run.size_pt), run.size_pt,
                                  run.fmt.clone(), run.font.clone(), off));
            }
            off += ch.len_utf8();
        }
        if !word.is_empty() {
            out.push(Tok::Word(word, run.size_pt, run.fmt.clone(), run.font.clone()));
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
fn break_para(para: &Paragraph, m: &Metrics, measure: f32, marker: Option<&str>,
              hyphenate: bool) -> Vec<Vec<Cell>> {
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
        let font = para.runs.first().and_then(|r| r.font.clone());
        for ch in mk.chars() {
            let w = m.advance_mm(ch, size);
            // 印は本文の一部ではないので off は段落頭(0)のまま
            cur.push(Cell { ch, x_mm: 0.0, w_mm: w, size_pt: size, fmt: fmt.clone(),
                            font: font.clone(), off: 0 });
            w_cur += w;
        }
    }
    for tok in tokenize(para, m) {
        let (cells, w): (Vec<Cell>, f32) = match &tok {
            Tok::One(ch, w, s, f, ft, o) =>
                (vec![Cell { ch: *ch, x_mm: 0.0, w_mm: *w, size_pt: *s, fmt: f.clone(),
                             font: ft.clone(), off: *o }], *w),
            Tok::Word(cs, s, f, ft) => (
                cs.iter().map(|(c, w, o)| Cell { ch: *c, x_mm: 0.0, w_mm: *w, size_pt: *s,
                                                 fmt: f.clone(), font: ft.clone(), off: *o })
                    .collect(),
                cs.iter().map(|(_, w, _)| *w).sum()),
            Tok::Space(w, s, f, ft, o) =>
                (vec![Cell { ch: ' ', x_mm: 0.0, w_mm: *w, size_pt: *s, fmt: f.clone(),
                             font: ft.clone(), off: *o }], *w),
        };

        if w_cur + w > measure && !cur.is_empty() {
            if let Tok::Space(..) = tok {
                // 行末に空白は要らない。行を折るだけ
                cur = close(&mut done, &mut cur, &mut w_cur, None);
                continue;
            }
            // 欧文の語は、設定が入っていれば音節で折って - を付ける
            if hyphenate {
                if let Tok::Word(cs, sz, f, ft) = &tok {
                    if let Some(k) = hyphen_split(cs, *sz, m, measure - w_cur) {
                        for (c, wch, o) in &cs[..k] {
                            cur.push(Cell { ch: *c, x_mm: 0.0, w_mm: *wch,
                                size_pt: *sz, fmt: f.clone(), font: ft.clone(), off: *o });
                            w_cur += *wch;
                        }
                        // ハイフンは本文の字ではない。バイト位置は直前の字に重ねる
                        // (欧文の字は1バイトなので、行末の勘定が壊れない)
                        let hw = m.advance_mm('-', *sz);
                        let off_h = cs[k - 1].2;
                        cur.push(Cell { ch: '-', x_mm: 0.0, w_mm: hw,
                            size_pt: *sz, fmt: f.clone(), font: ft.clone(), off: off_h });
                        w_cur += hw;
                        cur = close(&mut done, &mut cur, &mut w_cur, None);
                        for (c, wch, o) in &cs[k..] {
                            cur.push(Cell { ch: *c, x_mm: 0.0, w_mm: *wch,
                                size_pt: *sz, fmt: f.clone(), font: ft.clone(), off: *o });
                            w_cur += *wch;
                        }
                        continue;
                    }
                }
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

/// 欧文の語の分割点(Knuth-Liang のパターン。TeX と同じ方式)。
/// 前半の字数を返す。avail(残りの幅)に「前半 + '-'」が収まる最長の点を選ぶ。
fn hyphen_split(cs: &[(char, f32, usize)], size_pt: f32, m: &Metrics, avail: f32) -> Option<usize> {
    use hyphenation::{Hyphenator, Load};
    static DICT: std::sync::OnceLock<Option<hyphenation::Standard>> = std::sync::OnceLock::new();
    let dict = DICT
        .get_or_init(|| {
            hyphenation::Standard::from_embedded(hyphenation::Language::EnglishUS).ok()
        })
        .as_ref()?;
    let word: String = cs.iter().map(|(c, _, _)| *c).collect();
    // 数字入りの語(型番など)は折らない
    if word.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let hyphen_w = m.advance_mm('-', size_pt);
    let mut best = None;
    for b in dict.hyphenate(&word).breaks {
        // 語は ASCII なのでバイト位置 = 字数
        let w: f32 = cs[..b].iter().map(|(_, w, _)| *w).sum();
        if w + hyphen_w <= avail {
            best = Some(b);
        }
    }
    best
}

/// セルの中の余白(mm)
const CELL_PAD: f32 = 1.4;

fn lh_of(para: &Paragraph, frame: &Frame) -> f32 {
    frame.line_height_mm * para.spacing()
}

pub fn layout(doc: &Document, m: &Metrics, frame: &Frame) -> Sheet {
    let mut sheet = Sheet::default();
    let mut y = frame.y0_mm;

    // 段落番号は「何番目の箇条書きか」で決まる。段落の位置ではない。
    // レベル(インデント)ごとに数え、浅い番号が進んだら深い数えは振り出しへ
    let mut counters: Vec<usize> = Vec::new();
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
                let marker = match para.list {
                    ListKind::None => {
                        counters.clear();
                        None
                    }
                    _ => {
                        let l = para.indent as usize;
                        counters.truncate(l + 1);
                        while counters.len() <= l {
                            counters.push(0);
                        }
                        counters[l] += 1;
                        para.marker(counters[l] - 1)
                    }
                };
                // ドロップキャップ: 頭の1字を大きな1行として先に置き、
                // 残りは行長をその幅ぶん狭めて組む(Word は数行だけ回り込むが、
                // この版は段落まるごと狭める近似)
                let mut cap_shift = 0.0f32;
                let mut cap_len = 0usize;
                let mut owned_rest: Option<Paragraph> = None;
                if para.dropcap {
                    let first = para.runs.first();
                    if let Some(ch) = first.and_then(|r| r.text.chars().next()) {
                        let size0 = first.map(|r| r.size_pt).unwrap_or(10.5);
                        let cap_pt = size0 * 2.8;
                        let cap_w = m.advance_mm(ch, cap_pt) + 1.0;
                        cap_len = ch.len_utf8();
                        cap_shift = cap_w;
                        // 1行下のベースラインに置く = 2行に掛かる見た目
                        sheet.lines.push(Line {
                            cells: vec![Cell {
                                ch,
                                x_mm: indent_mm,
                                w_mm: cap_w,
                                size_pt: cap_pt,
                                fmt: first.map(|r| r.fmt.clone()).unwrap_or_default(),
                                font: first.and_then(|r| r.font.clone()),
                                off: 0,
                            }],
                            y_mm: y + frame.line_height_mm * para.spacing(),
                            from_body: true,
                            byte0: para_byte0,
                            cell: None,
                        });
                        let mut rest = para.clone();
                        if let Some(r0) = rest.runs.first_mut() {
                            r0.text = r0.text[cap_len..].to_string();
                        }
                        owned_rest = Some(rest);
                    }
                }
                let para_eff: &Paragraph = owned_rest.as_ref().unwrap_or(para);
                let measure = (measure - cap_shift).max(em);
                for mut cells in break_para(para_eff, m, measure, marker.as_deref(),
                                            doc.hyphenate) {
                    // 頭の1字を除いたぶん、バイト位置を戻す
                    if cap_len > 0 {
                        for c in &mut cells {
                            c.off += cap_len;
                        }
                    }
                    if cells.is_empty() {
                        // 空の段落も**行として持つ**。持たないと、後ろの行の
                        // バイト勘定が1つずつずれて、カーソルが本文とずれる
                        sheet.lines.push(Line {
                            cells: Vec::new(), y_mm: y, from_body: true,
                            byte0: para_byte0 + cap_len, cell: None });
                        y += frame.line_height_mm * para.spacing();
                        continue;
                    }
                    // 揃え。**行の幅と行長の差を、どこに置くか**の話でしかない
                    let w: f32 = cells.iter().map(|c| c.w_mm).sum();
                    let slack = (measure - w).max(0.0);
                    let mut x = indent_mm + cap_shift + match para.align {
                        Align::Left | Align::Justify | Align::Distribute => 0.0,
                        Align::Center => slack / 2.0,
                        Align::Right => slack,
                    };
                    // 均等割付: 差を字間に等しく配る(最後の行も配る)
                    let gap = if para.align == Align::Distribute && cells.len() >= 2 {
                        slack / (cells.len() - 1) as f32
                    } else {
                        0.0
                    };
                    let cells: Vec<Cell> = cells
                        .into_iter()
                        .map(|mut c| { c.x_mm = x; x += c.w_mm + gap; c })
                        .collect();
                    // ルビ。同じ読みの連なりの上に、半分の大きさの行を置く。
                    // 基底より狭ければ中付き(字間を等配)、広ければ中央から
                    // はみ出す(v1 — 基底を広げる詰めはまだしない)
                    {
                        let mut i = 0usize;
                        while i < cells.len() {
                            let Some(rt) = cells[i].fmt.ruby.clone() else {
                                i += 1;
                                continue;
                            };
                            let mut j = i + 1;
                            while j < cells.len()
                                && cells[j].fmt.ruby.as_deref() == Some(rt.as_str())
                            {
                                j += 1;
                            }
                            let x0 = cells[i].x_mm;
                            let x1 = cells[j - 1].x_mm + cells[j - 1].w_mm;
                            let pt = cells[i].size_pt / 2.0;
                            let rw: f32 =
                                rt.chars().map(|c| m.advance_mm(c, pt)).sum();
                            let n = rt.chars().count();
                            let gap = if n >= 1 && rw < (x1 - x0) {
                                ((x1 - x0) - rw) / (n as f32 + 1.0)
                            } else {
                                0.0
                            };
                            let mut rx = if gap > 0.0 {
                                x0 + gap
                            } else {
                                (x0 + x1 - rw) / 2.0
                            };
                            let mut rcells = Vec::new();
                            for ch in rt.chars() {
                                let w = m.advance_mm(ch, pt);
                                rcells.push(Cell {
                                    ch,
                                    x_mm: rx,
                                    w_mm: w,
                                    size_pt: pt,
                                    off: cells[i].off,
                                    fmt: CharFormat::default(),
                                    font: cells[i].font.clone(),
                                });
                                rx += w + gap;
                            }
                            sheet.lines.push(Line {
                                cells: rcells,
                                // 行送りの空き(黄金比の余白)の中、基底の頭の上
                                y_mm: y - frame.line_height_mm * 0.45,
                                from_body: false,
                                byte0: para_byte0 + cells[i].off,
                                cell: None,
                            });
                            i = j;
                        }
                    }
                    // 行頭の字の段落内位置から、本文の絶対位置を出す。
                    // 箇条書きの印は off=0 で入っているので、最小値を取れば
                    // 1行目(印+本文頭)も続きの行も正しく出る
                    let byte0 = para_byte0
                        + cells.iter().map(|c| c.off).min().unwrap_or(0);
                    sheet.lines.push(Line { cells, y_mm: y, from_body: true, byte0, cell: None });
                    y += frame.line_height_mm * para.spacing();
                }
                // 画像は段落の下に置く。幅が行長を超えるなら比例で縮める
                for im in para.images.iter().chain(para.images_new.iter()) {
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
                y = layout_table(table, m, frame, y, &mut sheet, table_no, doc.hyphenate);
                table_no += 1;
            }
        }
    }
    sheet
}

/// ヘッダー(footer=false)・フッター(footer=true)を**1ページぶん**組む。
///
/// [`PAGE_MARK`] はそのページの番号、[`PAGES_MARK`] は総頁の字に置き換わる。
/// **表示専用** — 置き換えでバイト位置が変わるので、行の byte0 は編集と結ばない
/// (ヘッダーの編集は紙面上ではなく板で行う)。
/// y はページ上端からの mm、x は左余白からの mm(本文の行と同じ物差し)。
pub fn layout_hf(
    hf: &HeadFoot,
    m: &Metrics,
    pg: &PageSetup,
    line_height_mm: f32,
    page_no: usize,
    total: usize,
    footer: bool,
) -> Vec<Line> {
    if hf.paragraphs.is_empty() {
        return Vec::new();
    }
    let num = page_no.to_string();
    let tot = total.to_string();
    let measure = pg.measure_mm();
    // ヘッダーは上余白の中を上から、フッターは下余白の頭から下へ
    let mut y = if footer {
        pg.h_mm - pg.bottom_mm + line_height_mm * 0.8
    } else {
        (pg.top_mm * 0.45).max(line_height_mm * 0.8)
    };
    let mut out = Vec::new();
    for para in &hf.paragraphs {
        let mut para = para.clone();
        for r in &mut para.runs {
            if r.text.contains(PAGE_MARK) {
                r.text = r.text.replace(PAGE_MARK, &num);
            }
            if r.text.contains(PAGES_MARK) {
                r.text = r.text.replace(PAGES_MARK, &tot);
            }
        }
        for cells in break_para(&para, m, measure, None, false) {
            let w: f32 = cells.iter().map(|c| c.w_mm).sum();
            let slack = (measure - w).max(0.0);
            let mut x = match para.align {
                Align::Left | Align::Justify | Align::Distribute => 0.0,
                Align::Center => slack / 2.0,
                Align::Right => slack,
            };
            let gap = if para.align == Align::Distribute && cells.len() >= 2 {
                slack / (cells.len() - 1) as f32
            } else {
                0.0
            };
            let cells: Vec<Cell> = cells
                .into_iter()
                .map(|mut c| {
                    c.x_mm = x;
                    x += c.w_mm + gap;
                    c
                })
                .collect();
            out.push(Line { cells, y_mm: y, from_body: false, byte0: 0, cell: None });
            y += line_height_mm;
        }
    }
    out
}

/// 段組み。**細い行長(column_measure_mm)で組んだ巻物**を、
/// ページごとに n 段へ折る。
///
/// 出てくる座標は「ページを縦に積み上げた物理座標」— y はページの並び、
/// x は段の位置へずらし済み。だから画面もクリックもキャレットも PDF も、
/// 座標を使う側は**何も変えずに**そのまま写せる。
/// ページの頭には breaks を置くので、紙に写す側はそこで頁を割る。
/// 縦書き(K4)。横に組んだ巻物を、右から左への列に写す。
/// 行 k → 右から k 本目の列。vert_x[i] が列の左肩の x(絶対 mm)、
/// Line.y_mm はその物理ページの本文の上端、Cell.x_mm は上からの距離。
/// ルビの行(基底より 0.45 行ぶん上)は基底の列の右肩に寄る。
/// 約物は縦用の字形へ置き換える(フォントの vert の近似)。
/// 初版の約束: 表・段組みとの併用はしない(呼ぶ側が避ける)。
/// 明示の改ページは列送りに畳まれる(頁の頭には来ない)
pub fn fold_vertical(sheet: &mut Sheet, pg: &PageSetup, y0_mm: f32, line_mm: f32) {
    sheet.vertical = true;
    let usable_w = (pg.w_mm - pg.left_mm - pg.right_mm).max(line_mm);
    let cpp = (usable_w / line_mm).floor().max(1.0) as usize; // 1頁の列数
    let right = pg.w_mm - pg.right_mm;
    sheet.breaks.clear();
    sheet.vert_x = Vec::with_capacity(sheet.lines.len());
    let mut max_page = 0usize;
    for line in &mut sheet.lines {
        let col_f = (line.y_mm - y0_mm) / line_mm;
        let col = col_f.round().max(0.0) as usize;
        let frac = col as f32 - col_f; // ルビなら正(基底の右へ)
        let (page, cip) = (col / cpp, col % cpp);
        max_page = max_page.max(page);
        sheet.vert_x.push(right - (cip as f32 + 1.0) * line_mm + frac * line_mm);
        line.y_mm = page as f32 * pg.h_mm + pg.top_mm;
        for c in &mut line.cells {
            c.ch = match c.ch {
                '、' => '︑',
                '。' => '︒',
                'ー' => '丨',
                '「' => '﹁',
                '」' => '﹂',
                '『' => '﹃',
                '』' => '﹄',
                '(' => '︵',
                ')' => '︶',
                '…' => '︙',
                other => other,
            };
        }
    }
    // ページの頭(物理座標)。紙に写す側がこの目印で頁を割る
    sheet.breaks = (1..=max_page).map(|k| k as f32 * pg.h_mm).collect();
}

pub fn fold_columns(sheet: &mut Sheet, pg: &PageSetup, y0_mm: f32) {
    let n = pg.cols();
    if n <= 1 {
        return;
    }
    let col_w = pg.column_measure_mm();
    let span = (pg.h_mm - pg.bottom_mm - y0_mm).max(10.0); // 1段に入る高さ
    // 第1走: 行を順に歩き、どの段(strip)に入るかを決める。
    // 巻物の y の区間 → 段、の対応も控える(罫線・画像を同じ折り方にするため)
    let mut strip = 0usize;
    let mut strip_y0 = y0_mm; // いまの段の頭(巻物の座標)
    let mut ranges: Vec<(f32, usize)> = vec![(strip_y0, 0)]; // (巻物のy起点, 段)
    let mut line_strip: Vec<usize> = Vec::with_capacity(sheet.lines.len());
    let mut breaks = std::mem::take(&mut sheet.breaks).into_iter().peekable();
    for line in &sheet.lines {
        // 明示の改ページ: 次のページの頭(= n の倍数の段)へ
        let mut forced = false;
        while let Some(&b) = breaks.peek() {
            if line.y_mm >= b - 0.01 {
                breaks.next();
                forced = true;
            } else {
                break;
            }
        }
        if forced {
            strip = (strip / n + 1) * n;
            strip_y0 = line.y_mm;
            ranges.push((strip_y0, strip));
        } else if line.y_mm - strip_y0 > span {
            strip += 1;
            strip_y0 = line.y_mm;
            ranges.push((strip_y0, strip));
        }
        line_strip.push(strip);
    }
    // 巻物の y → 段(罫線・画像・当たり判定に使う)
    let strip_of = |y: f32| -> usize {
        let mut s = 0usize;
        for (y0, k) in &ranges {
            if y >= *y0 - 0.01 {
                s = *k;
            }
        }
        s
    };
    // その段の起点(巻物の座標)。中身の無い段は最後の起点を使う
    let strip_start = |k: usize| -> f32 {
        ranges.iter().filter(|(_, s)| *s <= k).map(|(y0, _)| *y0).last().unwrap_or(y0_mm)
    };
    // 折る: y はページの積み上げへ、x は段の位置へ
    let place = |y: f32, k: usize| -> f32 {
        let page = k / n;
        page as f32 * pg.h_mm + y0_mm + (y - strip_start(k))
    };
    let dx = |k: usize| -> f32 { (k % n) as f32 * (col_w + COLUMN_GAP_MM) };
    for (line, k) in sheet.lines.iter_mut().zip(&line_strip) {
        line.y_mm = place(line.y_mm, *k);
        for c in &mut line.cells {
            c.x_mm += dx(*k);
        }
    }
    for r in &mut sheet.rules {
        let k = strip_of(r[1].min(r[3]));
        let (y1, y2) = (place(r[1], k), place(r[3], k));
        r[0] += dx(k);
        r[2] += dx(k);
        r[1] = y1;
        r[3] = y2;
    }
    for (_, rect) in &mut sheet.images {
        let k = strip_of(rect[1]);
        rect[1] = place(rect[1], k);
        rect[0] += dx(k);
    }
    for b in &mut sheet.cell_boxes {
        let k = strip_of(b.top_mm);
        b.top_mm = place(b.top_mm, k);
        b.x_mm += dx(k);
    }
    // 紙に写す側のために、2ページ目からの頭に改ページを置く
    let pages = line_strip.iter().map(|k| k / n + 1).max().unwrap_or(1);
    sheet.breaks = (1..pages).map(|p| p as f32 * pg.h_mm + y0_mm).collect();
}

/// 表を組む。戻り値は表の下の、次のベースライン。
///
/// セル結合を含めて組む:
/// - 横(gridSpan): セルが複数の格子を占め、幅はその合計
/// - 縦(vMerge): Continue のセルは描かず、Start のセルが行をまたいで延びる
///
/// 罫線は「格子」ではなく**結合後のセルの縁**に引く — 結合の中を
/// 線が横切ると、様式の枠が壊れて見える。
fn layout_table(table: &Table, m: &Metrics, frame: &Frame, y_in: f32, sheet: &mut Sheet,
                table_no: usize, hyphenate: bool) -> f32 {
    // 列数は「セルの数」ではなく「セルが占める格子の数」
    let ncols = table
        .rows
        .iter()
        .map(|r| r.iter().map(|c| c.span()).sum::<usize>())
        .max()
        .unwrap_or(1)
        .max(1);
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
    let lh = frame.line_height_mm;

    // 表の上端。直前のベースラインから少し空ける
    let table_top = y_in - lh * 0.55;

    // 第1走: 各セルを折り、格子の位置と行の高さを決める。
    // 行はセルの文章(段落を \n で繋いだもの)の中のバイト位置を持つ
    struct Laid {
        ci: usize,     // 行の中の何番目のセルか(編集はこの番号で結ぶ)
        gc: usize,     // 占める格子の左端
        span: usize,
        v: VMerge,
        lines: Vec<(Vec<Cell>, usize)>,
        x: f32,
        w: f32,
    }
    let mut rows_laid: Vec<Vec<Laid>> = Vec::new();
    let mut row_hs: Vec<f32> = Vec::new();
    for row in &table.rows {
        let mut gc = 0usize;
        let mut nlines = 1usize;
        let mut laid: Vec<Laid> = Vec::new();
        for (ci, cell) in row.iter().enumerate() {
            let span = cell.span().min(ncols.saturating_sub(gc)).max(1);
            let x = xs[gc.min(ncols)];
            let w = xs[(gc + span).min(ncols)] - x;
            let mut ls: Vec<(Vec<Cell>, usize)> = Vec::new();
            // 縦結合の続きは上のセルに呑まれている。中身は組まない
            if cell.v_merge != VMerge::Continue {
                let inner = (w - 2.0 * CELL_PAD).max(2.0);
                let mut para0 = 0usize;
                for para in &cell.paragraphs {
                    for cs in break_para(para, m, inner, None, hyphenate) {
                        let b0 = para0 + cs.iter().map(|c| c.off).min().unwrap_or(0);
                        ls.push((cs, b0));
                    }
                    let plen: usize = para.runs.iter().map(|r| r.text.len()).sum();
                    para0 += plen + 1;
                }
                nlines = nlines.max(ls.len());
            }
            laid.push(Laid { ci, gc, span, v: cell.v_merge, lines: ls, x, w });
            gc += span;
        }
        rows_laid.push(laid);
        row_hs.push(nlines as f32 * lh + 2.0 * CELL_PAD);
    }

    // 行の上端(累積)
    let mut tops = vec![table_top];
    for h in &row_hs {
        tops.push(tops.last().unwrap() + h);
    }
    let table_bottom = *tops.last().unwrap();

    // 格子の地図(第2走が中身を消費した後も結合の形を見られるように)
    let grid: Vec<Vec<(usize, usize, VMerge)>> = rows_laid
        .iter()
        .map(|r| r.iter().map(|l| (l.gc, l.span, l.v)).collect())
        .collect();
    // row 行で格子 g を覆うセル
    let cover = |row: usize, g: usize| -> Option<(usize, usize, VMerge)> {
        grid.get(row)?.iter().find(|(gc, span, _)| *gc <= g && g < gc + span).copied()
    };
    // (ri, gc) から始まる縦結合の高さ: 同じ格子位置で Continue が続く間
    let merged_h = |ri: usize, gc: usize| -> f32 {
        let mut h = row_hs[ri];
        for r in ri + 1..grid.len() {
            match cover(r, gc) {
                Some((g0, _, VMerge::Continue)) if g0 == gc => h += row_hs[r],
                _ => break,
            }
        }
        h
    };

    // 第2走: 中身と当たり判定(from_body=false。本文の位置合わせに入れない)
    for (ri, laid) in rows_laid.into_iter().enumerate() {
        let row_top = tops[ri];
        for l in laid {
            if l.v == VMerge::Continue {
                continue;
            }
            let h = if l.v == VMerge::Start { merged_h(ri, l.gc) } else { row_hs[ri] };
            let x0 = l.x + CELL_PAD;
            let mut yy = row_top + CELL_PAD + lh * 0.8;
            let id = Some((table_no, ri, l.ci));
            for (cells, b0) in l.lines {
                let mut x = x0;
                let cells: Vec<Cell> = cells
                    .into_iter()
                    .map(|mut c| { c.x_mm = x; x += c.w_mm; c })
                    .collect();
                sheet.lines.push(Line { cells, y_mm: yy, from_body: false, byte0: b0, cell: id });
                yy += lh;
            }
            // クリックの当たり判定(結合したセルは結合後の大きさで当てる)
            sheet.cell_boxes.push(CellBox {
                table: table_no,
                row: ri,
                col: l.ci,
                x_mm: l.x,
                top_mm: row_top,
                w_mm: l.w,
                h_mm: h,
            });
        }
    }

    // 罫線・横: 行の境ごとに、格子を歩いて「引ける区間」を繋いで引く。
    // 縦結合の中を横切る線は引かない
    for b in 0..=grid.len() {
        let y = tops[b];
        let mut g = 0usize;
        while g < ncols {
            // 境の下の行が Continue なら、この格子の上に線は引かない
            let blocked = b > 0
                && b < grid.len()
                && matches!(cover(b, g), Some((_, _, VMerge::Continue)));
            if blocked {
                g += 1;
                continue;
            }
            let start = g;
            while g < ncols {
                let blk = b > 0
                    && b < grid.len()
                    && matches!(cover(b, g), Some((_, _, VMerge::Continue)));
                if blk {
                    break;
                }
                g += 1;
            }
            sheet.rules.push([xs[start], y, xs[g], y]);
        }
    }
    // 罫線・縦: 行ごとに、結合後のセルの縁に引く(結合の中には引かない)
    for (ri, row) in grid.iter().enumerate() {
        let (top, bottom) = (tops[ri], tops[ri + 1]);
        let mut edges: Vec<f32> = Vec::new();
        for (gc, span, _) in row {
            edges.push(xs[*gc]);
            edges.push(xs[(gc + span).min(ncols)]);
        }
        if row.is_empty() {
            edges.push(xs[0]);
            edges.push(xs[ncols]);
        }
        edges.sort_by(|a, b| a.partial_cmp(b).unwrap());
        edges.dedup_by(|a, b| (*a - *b).abs() < 0.01);
        for x in edges {
            sheet.rules.push([x, top, x, bottom]);
        }
    }
    // 次のベースライン
    table_bottom + lh
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
    fn レベルで印と番号の形が変わる() {
        let mut p = Paragraph { list: ListKind::Bullet, ..Default::default() };
        assert_eq!(p.marker(0).as_deref(), Some("・"));
        p.indent = 1;
        assert_eq!(p.marker(0).as_deref(), Some("○"), "レベル2の印が変わらない");
        p.list = ListKind::Number;
        assert_eq!(p.marker(2).as_deref(), Some("(3) "), "レベル2の番号の形が違う");
    }

    #[test]
    fn 深い番号は浅い番号が進むと振り出しに戻る() {
        let data = font::load(font::for_document(None).unwrap().0).unwrap();
        let m = Metrics::new(&data).unwrap();
        let mut d = Document::plain("一\n一の一\n一の二\n二\n二の一", 10.5);
        for (i, ind) in [(0usize, 0u8), (1, 1), (2, 1), (3, 0), (4, 1)] {
            if let Block::Para(p) = &mut d.blocks[i] {
                p.list = ListKind::Number;
                p.indent = ind;
            }
        }
        let s = layout(&d, &m, &Frame { measure_mm: 100.0, line_height_mm: 6.0, y0_mm: 20.0 });
        let texts: Vec<String> = s.lines.iter().map(|l| l.text()).collect();
        assert!(texts[1].starts_with("(1) "), "{:?}", texts[1]);
        assert!(texts[2].starts_with("(2) "), "{:?}", texts[2]);
        assert!(texts[3].starts_with("2. "), "{:?}", texts[3]);
        assert!(texts[4].starts_with("(1) "), "深い数えが振り出しに戻らない: {:?}", texts[4]);
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
mod vertical_tests {
    use super::*;

    #[test]
    fn 縦書きは右の列から左へ進み字は上から下へ() {
        let data = font::load(font::for_document(None).unwrap().0).unwrap();
        let m = Metrics::new(&data).unwrap();
        let d = Document::plain("一行目の文。\n二行目。", 10.5);
        let pg = PageSetup::default();
        let y0 = pg.top_mm + 4.0;
        let measure = pg.h_mm - pg.top_mm - pg.bottom_mm - 8.0;
        let mut sheet = layout(&d, &m,
            &Frame { measure_mm: measure, line_height_mm: 6.0, y0_mm: y0 });
        fold_vertical(&mut sheet, &pg, y0, 6.0);
        assert!(sheet.vertical);
        assert_eq!(sheet.vert_x.len(), sheet.lines.len());
        // 1列目は右端の近く、2列目はその左
        let right = pg.w_mm - pg.right_mm;
        assert!((sheet.vert_x[0] - (right - 6.0)).abs() < 0.5,
            "1列目が右端に無い: {}", sheet.vert_x[0]);
        assert!(sheet.vert_x[1] < sheet.vert_x[0], "2列目が左に来ていない");
        // 字は上から下(Cell.x_mm が増える)
        let cs = &sheet.lines[0].cells;
        assert!(cs[0].x_mm < cs[1].x_mm, "字が上から下に並んでいない");
        // 約物が縦用に置き換わる
        assert!(cs.iter().any(|c| c.ch == '︒'), "句点が縦用でない");
    }
}

#[cfg(test)]
mod ruby_tests {
    use super::*;

    #[test]
    fn ルビの行が基底の上に半分の大きさで出る() {
        let data = font::load(font::for_document(None).unwrap().0).unwrap();
        let m = Metrics::new(&data).unwrap();
        let mut d = Document::plain("組版の話", 10.5);
        // 「組版」にだけルビを振る
        d.apply_char_format(0..6, |f| f.ruby = Some("くみはん".into()));
        let sheet = layout(&d, &m,
            &Frame { measure_mm: 100.0, line_height_mm: 6.0, y0_mm: 20.0 });
        let body: Vec<&Line> = sheet.lines.iter().filter(|l| l.from_body).collect();
        let ruby: Vec<&Line> = sheet.lines.iter().filter(|l| !l.from_body).collect();
        assert_eq!(body.len(), 1);
        assert_eq!(ruby.len(), 1, "ルビの行が無い");
        assert_eq!(ruby[0].text(), "くみはん");
        assert!(ruby[0].y_mm < body[0].y_mm, "ルビが基底より下にある");
        assert!((ruby[0].cells[0].size_pt - 5.25).abs() < 0.01, "半分の大きさでない");
        let bx0 = body[0].cells[0].x_mm;
        let bx1 = body[0].cells[1].x_mm + body[0].cells[1].w_mm;
        let rx0 = ruby[0].cells[0].x_mm;
        let rlast = ruby[0].cells.last().unwrap();
        let rx1 = rlast.x_mm + rlast.w_mm;
        let (bc, rc) = ((bx0 + bx1) / 2.0, (rx0 + rx1) / 2.0);
        assert!((bc - rc).abs() < 1.0, "ルビが基底の中央に来ていない: {bc} vs {rc}");
    }
}

#[cfg(test)]
mod distribute_tests {
    use super::*;

    #[test]
    fn 均等割付は最後の行も行長いっぱいに広がる() {
        let data = font::load(font::for_document(None).unwrap().0).unwrap();
        let m = Metrics::new(&data).unwrap();
        let mut d = Document::plain("氏名", 10.5);
        if let Some(Block::Para(p)) = d.blocks.first_mut() {
            p.align = Align::Distribute;
        }
        let sheet = layout(&d, &m,
            &Frame { measure_mm: 100.0, line_height_mm: 6.0, y0_mm: 20.0 });
        let line = &sheet.lines[0];
        let last = line.cells.last().unwrap();
        assert!(
            (last.x_mm + last.w_mm - 100.0).abs() < 0.5,
            "右端に届いていない: {}",
            last.x_mm + last.w_mm
        );
        assert!(line.cells[0].x_mm < 0.5, "左端から始まっていない");
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
            ..Default::default()
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
            ..Default::default()
        };
        let mut d = Document { font: None, page: None, sect_raw: None, header: Default::default(), footer: Default::default(), page_color: None, watermark: None, ink: Vec::new(), track_author: None, hyphenate: false, protection: None, props: Default::default(), vertical: false, blocks: vec![] };
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
mod merge_layout_tests {
    use super::*;

    fn cell(s: &str) -> Cellbox {
        Cellbox {
            paragraphs: vec![Paragraph {
                runs: vec![Run {
                    text: s.into(), size_pt: 10.5, font: None, fmt: Default::default() }],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn sheet_of(rows: Vec<Vec<Cellbox>>) -> Sheet {
        let data = font::load(font::for_document(None).unwrap().0).unwrap();
        let m = Metrics::new(&data).unwrap();
        let d = Document {
            font: None, page: None, sect_raw: None, header: Default::default(), footer: Default::default(), page_color: None, watermark: None, ink: Vec::new(), track_author: None, hyphenate: false, protection: None, props: Default::default(), vertical: false,
            blocks: vec![Block::Table(Table { col_mm: vec![], rows })],
        };
        layout(&d, &m, &Frame { measure_mm: 100.0, line_height_mm: 6.0, y0_mm: 20.0 })
    }

    #[test]
    fn 横の結合は列をまたぐ() {
        // 1行目: 見出しが2列ぶん。2行目: 普通の2列
        let mut head = cell("見出し");
        head.col_span = 2;
        let s = sheet_of(vec![
            vec![head],
            vec![cell("左"), cell("右")],
        ]);
        let b0 = s.cell_boxes.iter().find(|b| b.row == 0 && b.col == 0).unwrap();
        assert!((b0.w_mm - 100.0).abs() < 0.01, "結合したのに幅が広がらない: {}", b0.w_mm);
        // 結合の中(x=50)を縦線が横切らない(1行目の帯だけを見る)
        let mid_crosses = s.rules.iter().any(|r| {
            r[0] == r[2] && (r[0] - 50.0).abs() < 0.01 && r[1] < b0.top_mm + b0.h_mm - 0.1
                && r[3] > b0.top_mm + 0.1
        });
        assert!(!mid_crosses, "結合の中を縦線が横切った");
        // 2行目には x=50 の縦線がある
        let b1 = s.cell_boxes.iter().find(|b| b.row == 1 && b.col == 1).unwrap();
        assert!((b1.x_mm - 50.0).abs() < 0.01, "2行目の右セルの位置が違う: {}", b1.x_mm);
    }

    #[test]
    fn 縦の結合は行をまたぐ() {
        let mut start = cell("項目");
        start.v_merge = VMerge::Start;
        let mut cont = cell("");
        cont.v_merge = VMerge::Continue;
        let s = sheet_of(vec![
            vec![start, cell("1行目")],
            vec![cont, cell("2行目")],
        ]);
        // 呑まれたセルには当たり判定が無く、始まりのセルが2行ぶんに延びる
        assert!(s.cell_boxes.iter().all(|b| !(b.row == 1 && b.col == 0)),
            "呑まれたセルに当たり判定が残っている");
        let b0 = s.cell_boxes.iter().find(|b| b.row == 0 && b.col == 0).unwrap();
        let b1 = s.cell_boxes.iter().find(|b| b.row == 1 && b.col == 1).unwrap();
        let merged_bottom = b0.top_mm + b0.h_mm;
        let row1_bottom = b1.top_mm + b1.h_mm;
        assert!((merged_bottom - row1_bottom).abs() < 0.01,
            "結合が2行目の下端まで延びていない: {merged_bottom} vs {row1_bottom}");
        // 行の境の横線が、結合の中(左半分)を横切らない
        let boundary = b1.top_mm;
        for r in s.rules.iter().filter(|r| r[1] == r[3] && (r[1] - boundary).abs() < 0.01) {
            assert!(r[0] >= 50.0 - 0.01,
                "結合の中を横線が横切った: x {}..{}", r[0], r[2]);
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
            ..Default::default()
        }
    }

    fn rules_of(col_mm: Vec<f32>) -> Vec<[f32; 4]> {
        let data = font::load(font::for_document(None).unwrap().0).unwrap();
        let m = Metrics::new(&data).unwrap();
        let d = Document {
            font: None,
            page: None,
            sect_raw: None, header: Default::default(), footer: Default::default(), page_color: None, watermark: None, ink: Vec::new(), track_author: None, hyphenate: false, protection: None, props: Default::default(), vertical: false,
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

#[cfg(test)]
mod run_edit_tests {
    use super::*;

    fn bold_spans(d: &Document) -> Vec<(String, bool)> {
        d.paragraphs()
            .flat_map(|p| p.runs.iter())
            .map(|r| (r.text.clone(), r.fmt.bold))
            .collect()
    }

    #[test]
    fn 段落の途中だけ太字にできる() {
        let mut d = Document::plain("防火戸の仕様を確認", 10.5);
        let s = "防火戸の".len();
        let e = "防火戸の仕様".len();
        d.apply_char_format(s..e, |f| f.bold = true);
        assert_eq!(
            bold_spans(&d),
            vec![
                ("防火戸の".into(), false),
                ("仕様".into(), true),
                ("を確認".into(), false)
            ],
            "選択の字だけが太字になっていない"
        );
    }

    #[test]
    fn 部分書式が打鍵で流されない() {
        let mut d = Document::plain("防火戸の仕様を確認", 10.5);
        let s = "防火戸の".len();
        let e = "防火戸の仕様".len();
        d.apply_char_format(s..e, |f| f.bold = true);
        // 「仕様」の後ろに「書」を打った(1回の編集 = 1箇所の置き換え)
        d.set_body_text("防火戸の仕様書を確認", 10.5);
        assert_eq!(
            bold_spans(&d),
            vec![
                ("防火戸の".into(), false),
                ("仕様書".into(), true),
                ("を確認".into(), false)
            ],
            "太字の中に打った字が太字にならない・境が流された"
        );
        // 頭に打っても境は動かない
        d.set_body_text("この防火戸の仕様書を確認", 10.5);
        assert_eq!(bold_spans(&d)[1], ("仕様書".into(), true), "頭への挿入で境がずれた");
    }

    #[test]
    fn 選択の中だけ消しても境が残る() {
        let mut d = Document::plain("あいうえお", 10.5);
        d.apply_char_format(3..12, |f| f.bold = true); // いうえ
        d.set_body_text("あいえお", 10.5); // 「う」を消した
        assert_eq!(
            bold_spans(&d),
            vec![("あ".into(), false), ("いえ".into(), true), ("お".into(), false)],
            "削除で境が流された"
        );
    }

    #[test]
    fn 途中の段落でenterしても下の性質がずれない() {
        // 旧方式(段落番号で写す)の持病: 段落の増減で下の段落の性質がずれた
        let mut d = Document::plain("一\n二\n三", 10.5);
        let start = "一\n二".len();
        d.apply_align(start + 1..start + 1, Align::Center); // 「三」を中央に
        if let Block::Para(p) = &mut d.blocks[2] {
            p.shade = Some("FFF2CC".into());
        }
        // 「二」の後ろで Enter
        d.set_body_text("一\n二\n\n三", 10.5);
        let ps: Vec<&Paragraph> = d.paragraphs().collect();
        assert_eq!(ps.len(), 4);
        assert_eq!(ps[3].align, Align::Center, "下の段落の揃えがずれた");
        assert_eq!(ps[3].shade.as_deref(), Some("FFF2CC"), "下の段落の帯がずれた");
        // 割った両方が段落の性質を持つ(Word と同じ)。下へ「ずれる」のとは違う
        assert_eq!(ps[2].shade.as_deref(), Some("FFF2CC"));
        // undo(= 逆向きの1回の編集)でも戻る
        d.set_body_text("一\n二\n三", 10.5);
        let ps: Vec<&Paragraph> = d.paragraphs().collect();
        assert_eq!(ps[2].align, Align::Center, "undo で揃えが消えた");
    }

    #[test]
    fn 編集しても表の位置が動かない() {
        // 旧方式は打鍵のたびに表が末尾へ動いていた
        let mut d = Document::plain("前\n後", 10.5);
        d.blocks.insert(1, Block::Table(Table {
            col_mm: vec![],
            rows: vec![vec![Cellbox::default()]],
        }));
        d.set_body_text("前に足す\n後", 10.5);
        let kinds: Vec<&str> = d.blocks.iter().map(|b| match b {
            Block::Para(_) => "段落",
            Block::Table(_) => "表",
        }).collect();
        assert_eq!(kinds, vec!["段落", "表", "段落"], "表が動いた: {kinds:?}");
    }

    #[test]
    fn 段落の合流で頭の性質が残る() {
        let mut d = Document::plain("一\n二", 10.5);
        d.apply_align(0..0, Align::Center);
        // 「一」と「二」の間の改行を消した
        d.set_body_text("一二", 10.5);
        let ps: Vec<&Paragraph> = d.paragraphs().collect();
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].align, Align::Center, "合流で頭の性質が消えた");
        assert_eq!(ps[0].runs[0].text, "一二");
    }

    #[test]
    fn 大きさと書体も選択の字にだけ掛かる() {
        let mut d = Document::plain("見出しと本文", 10.5);
        d.apply_size(0.."見出し".len(), |_| 16.0);
        d.apply_font(0.."見出し".len(), Some("ゴシック".into()));
        let runs: Vec<&Run> = d.paragraphs().flat_map(|p| p.runs.iter()).collect();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].size_pt, 16.0);
        assert_eq!(runs[0].font.as_deref(), Some("ゴシック"));
        assert_eq!(runs[1].size_pt, 10.5, "選択の外まで変わった");
        assert_eq!(runs[1].font, None);
    }

    #[test]
    fn カーソル位置の書式が読める() {
        let mut d = Document::plain("あ太字い", 10.5);
        d.apply_char_format(3..9, |f| f.bold = true);
        assert!(!d.char_format_at(0..0).bold, "頭で太字と言った");
        assert!(d.char_format_at(9..9).bold, "太字の直後で太字と言わない");
        assert!(!d.char_format_at(12..12).bold, "太字の外で太字と言った");
    }
}

#[cfg(test)]
mod ref_field_tests {
    use super::*;

    fn field_doc() -> Document {
        let mut d = Document::plain("仕様は3ページを見る", 10.5);
        let s = "仕様は".len();
        let e = "仕様は3ページ".len();
        d.apply_field(s..e, Some(RefField { name: "様式".into(), page: false }));
        d
    }

    #[test]
    fn 参照は編集で流されず前後の打鍵で消えない() {
        let mut d = field_doc();
        // 参照の前に打つ
        d.set_body_text("この仕様は3ページを見る", 10.5);
        let f: Vec<_> = d.paragraphs().flat_map(|p| p.runs.iter())
            .filter(|r| r.fmt.field.is_some())
            .map(|r| r.text.clone())
            .collect();
        assert_eq!(f, vec!["3ページ"], "参照が前の打鍵で壊れた");
        // 参照の直後に打っても、参照は伸びない
        let e = "この仕様は3ページ".len();
        let mut t = d.body_text();
        t.insert_str(e, "目");
        d.set_body_text(&t, 10.5);
        let f: Vec<_> = d.paragraphs().flat_map(|p| p.runs.iter())
            .filter(|r| r.fmt.field.is_some())
            .map(|r| r.text.clone())
            .collect();
        assert_eq!(f, vec!["3ページ"], "打った字が参照に呑まれた");
    }

    #[test]
    fn 参照の中を触ると普通の字に降りる() {
        let mut d = field_doc();
        // 「3ページ」の中の「ペ」を消した
        d.set_body_text("仕様は3ージを見る", 10.5);
        assert!(d.paragraphs().flat_map(|p| p.runs.iter())
            .all(|r| r.fmt.field.is_none()),
            "壊れた参照が参照のまま残った");
        assert_eq!(d.body_text(), "仕様は3ージを見る", "本文まで変わった");
    }

    #[test]
    fn 参照の値を計算し直せる() {
        let mut d = field_doc();
        let n = d.refresh_fields(|name, page| {
            assert_eq!(name, "様式");
            assert!(!page);
            Some("5ページ".into())
        });
        assert_eq!(n, 1);
        assert_eq!(d.body_text(), "仕様は5ページを見る");
        // 同じ値ならもう数えない
        assert_eq!(d.refresh_fields(|_, _| Some("5ページ".into())), 0);
    }

    #[test]
    fn 参照ごと太字にしても参照は残る() {
        let mut d = field_doc();
        d.apply_char_format(0..d.body_text().len(), |f| f.bold = true);
        let r: Vec<_> = d.paragraphs().flat_map(|p| p.runs.iter())
            .filter(|r| r.fmt.field.is_some()).collect();
        assert_eq!(r.len(), 1, "太字で参照が消えた");
        assert!(r[0].fmt.bold);
    }
}

#[cfg(test)]
mod hyphen_tests {
    use super::*;

    #[test]
    fn 英語の語が音節で折れてハイフンが付く() {
        let data = font::load(font::for_document(None).unwrap().0).unwrap();
        let m = Metrics::new(&data).unwrap();
        let text = "The quick information hyphenation representation communication demonstration";
        let mut d = Document::plain(text, 10.5);
        d.hyphenate = true;
        let s = layout(&d, &m, &Frame { measure_mm: 45.0, line_height_mm: 6.0, y0_mm: 20.0 });
        let joined: Vec<String> = s.lines.iter().map(|l| l.text()).collect();
        assert!(joined.iter().any(|l| l.ends_with('-')),
            "どの行末にもハイフンが無い: {joined:?}");
        for l in &s.lines {
            assert!(l.width_mm() <= 45.1, "行長を超えた: {}", l.width_mm());
        }
        // ハイフンを除けば、文字は一つも失われない
        let got: String = s.lines.iter().flat_map(|l| l.cells.iter())
            .map(|c| c.ch).filter(|c| *c != '-' && *c != ' ').collect();
        let want: String = text.chars().filter(|c| *c != ' ').collect();
        assert_eq!(got, want, "ハイフネーションで字が消えた");
    }

    #[test]
    fn 切らなければ何も変わらない() {
        let data = font::load(font::for_document(None).unwrap().0).unwrap();
        let m = Metrics::new(&data).unwrap();
        let d = Document::plain("The quick information hyphenation", 10.5);
        let s = layout(&d, &m, &Frame { measure_mm: 45.0, line_height_mm: 6.0, y0_mm: 20.0 });
        assert!(s.lines.iter().all(|l| !l.text().ends_with('-')),
            "設定していないのに折った");
    }
}

#[cfg(test)]
mod dropcap_tests {
    use super::*;

    #[test]
    fn 頭の1字が大きく残りは狭く組まれる() {
        let data = font::load(font::for_document(None).unwrap().0).unwrap();
        let m = Metrics::new(&data).unwrap();
        let mut d = Document::plain(&format!("春{}", "はあけぼの。".repeat(8)), 10.5);
        if let Block::Para(p) = &mut d.blocks[0] {
            p.dropcap = true;
        }
        let s = layout(&d, &m, &Frame { measure_mm: 100.0, line_height_mm: 6.0, y0_mm: 20.0 });
        let cap = &s.lines[0];
        assert_eq!(cap.text(), "春");
        assert!(cap.cells[0].size_pt > 25.0, "頭の字が大きくない: {}", cap.cells[0].size_pt);
        // 本文は頭の字の右から始まり、バイト位置は「春」の後ろから
        let body = &s.lines[1];
        assert!(body.cells[0].x_mm > cap.cells[0].w_mm - 0.5, "本文が頭の字に重なる");
        assert_eq!(body.byte0, "春".len(), "バイト勘定がずれた");
        // 文字は一つも失われない
        let got: String = s.lines.iter().flat_map(|l| l.cells.iter()).map(|c| c.ch).collect();
        assert_eq!(got.chars().count(), format!("春{}", "はあけぼの。".repeat(8)).chars().count());
    }
}

#[cfg(test)]
mod column_tests {
    use super::*;

    fn folded(n_lines: usize, cols: u8) -> Sheet {
        let data = font::load(font::for_document(None).unwrap().0).unwrap();
        let m = Metrics::new(&data).unwrap();
        let text = (1..=n_lines).map(|i| format!("{i} 行目")).collect::<Vec<_>>().join("\n");
        let d = Document::plain(&text, 10.5);
        let pg = PageSetup { columns: cols, ..Default::default() };
        let mut s = layout(&d, &m, &Frame {
            measure_mm: pg.column_measure_mm(),
            line_height_mm: 6.4,
            y0_mm: 24.0,
        });
        fold_columns(&mut s, &pg, 24.0);
        s
    }

    #[test]
    fn 二段に折ると右の段へ続く() {
        // A4 の1段は約40行。60行なら 1ページ目の左40行 + 右20行
        let s = folded(60, 2);
        let pg = PageSetup { columns: 2, ..Default::default() };
        let left: Vec<&Line> = s.lines.iter()
            .filter(|l| l.cells[0].x_mm < pg.column_measure_mm()).collect();
        let right: Vec<&Line> = s.lines.iter()
            .filter(|l| l.cells[0].x_mm >= pg.column_measure_mm()).collect();
        assert!(!right.is_empty(), "右の段に何も行かない");
        assert!(left.len() > right.len(), "左から詰まっていない");
        // 右の段の頭は左の段の頭と同じ高さ(ページの頭)
        assert!((right[0].y_mm - s.lines[0].y_mm).abs() < 0.01,
            "右の段がページの頭から始まらない: {} vs {}", right[0].y_mm, s.lines[0].y_mm);
        // ページは1枚に収まる(60行 = 2段ぶん以内)
        assert!(s.breaks.is_empty(), "1ページに収まるのに頁が割れた");
    }

    #[test]
    fn 二段でも溢れれば次のページへ() {
        let s = folded(100, 2);
        assert!(!s.breaks.is_empty(), "2段×1ページを超えたのに頁が割れない");
        let pg = PageSetup::default();
        let last = s.lines.last().unwrap();
        assert!(last.y_mm > pg.h_mm, "2ページ目の座標が積み上がっていない");
        // どの行も段の中に収まる(x が紙からはみ出さない)
        for l in &s.lines {
            let right = l.cells.last().map(|c| c.x_mm + c.w_mm).unwrap_or(0.0);
            assert!(right <= pg.measure_mm() + 0.5, "段からはみ出した: {right}mm");
        }
    }

    #[test]
    fn 一段なら何も変わらない() {
        let a = folded(30, 1);
        let data = font::load(font::for_document(None).unwrap().0).unwrap();
        let m = Metrics::new(&data).unwrap();
        let text = (1..=30).map(|i| format!("{i} 行目")).collect::<Vec<_>>().join("\n");
        let d = Document::plain(&text, 10.5);
        let b = layout(&d, &m, &Frame {
            measure_mm: PageSetup::default().column_measure_mm(),
            line_height_mm: 6.4,
            y0_mm: 24.0,
        });
        assert_eq!(a.lines.len(), b.lines.len());
        for (x, y) in a.lines.iter().zip(&b.lines) {
            assert!((x.y_mm - y.y_mm).abs() < 0.001, "1段なのに動いた");
        }
    }
}

#[cfg(test)]
mod hf_layout_tests {
    use super::*;

    fn metrics() -> Vec<u8> {
        font::load(font::for_document(None).unwrap().0).unwrap()
    }

    fn hf(text: &str) -> HeadFoot {
        HeadFoot {
            paragraphs: Document::plain(text, 10.5).paragraphs().cloned().collect(),
            part: None,
        }
    }

    #[test]
    fn ページ番号の印が番号の字になる() {
        let data = metrics();
        let m = Metrics::new(&data).unwrap();
        let pg = PageSetup::default();
        let f = hf(&format!("- {PAGE_MARK} -"));
        assert_eq!(layout_hf(&f, &m, &pg, 6.4, 1, 9, true)[0].text(), "- 1 -");
        assert_eq!(layout_hf(&f, &m, &pg, 6.4, 12, 9, true)[0].text(), "- 12 -");
    }

    #[test]
    fn ヘッダーは上余白フッターは下余白に入る() {
        let data = metrics();
        let m = Metrics::new(&data).unwrap();
        let pg = PageSetup::default();
        let h = layout_hf(&hf("頭"), &m, &pg, 6.4, 1, 1, false);
        assert!(h[0].y_mm < pg.top_mm, "ヘッダーが本文域に食い込む: {}", h[0].y_mm);
        assert!(h[0].y_mm > 0.0);
        let f = layout_hf(&hf("足"), &m, &pg, 6.4, 1, 1, true);
        assert!(f[0].y_mm > pg.h_mm - pg.bottom_mm,
            "フッターが本文域に食い込む: {}", f[0].y_mm);
        assert!(f[0].y_mm < pg.h_mm, "紙の外に出た: {}", f[0].y_mm);
    }

    #[test]
    fn フッターの中央揃えが効く() {
        let data = metrics();
        let m = Metrics::new(&data).unwrap();
        let pg = PageSetup::default();
        let mut f = hf(&PAGE_MARK.to_string());
        f.paragraphs[0].align = Align::Center;
        let lines = layout_hf(&f, &m, &pg, 6.4, 1, 9, true);
        assert!(lines[0].cells[0].x_mm > pg.measure_mm() * 0.3,
            "中央に寄っていない: {}", lines[0].cells[0].x_mm);
    }

    #[test]
    fn ページ数の印が総頁の字になる() {
        let data = metrics();
        let m = Metrics::new(&data).unwrap();
        let pg = PageSetup::default();
        let f = hf(&format!("{PAGE_MARK} / {PAGES_MARK}"));
        assert_eq!(layout_hf(&f, &m, &pg, 6.4, 2, 7, true)[0].text(), "2 / 7");
    }

    #[test]
    fn 無ければ何も出ない() {
        let data = metrics();
        let m = Metrics::new(&data).unwrap();
        assert!(layout_hf(&HeadFoot::default(), &m, &PageSetup::default(), 6.4, 1, 1, false)
            .is_empty());
    }

    #[test]
    fn 平文との相互変換で書式が残る() {
        // 板での編集は paras_text / set_paras_text を通る
        let mut ps: Vec<Paragraph> =
            Document::plain("社外秘", 10.5).paragraphs().cloned().collect();
        ps[0].align = Align::Right;
        ps[0].runs[0].fmt.bold = true;
        set_paras_text(&mut ps, "社外秘・控", 10.5);
        assert_eq!(paras_text(&ps), "社外秘・控");
        assert_eq!(ps[0].align, Align::Right, "揃えが消えた");
        assert!(ps[0].runs[0].fmt.bold, "太字が消えた");
    }
}

#[cfg(test)]
mod shade_carry_tests {
    use super::*;

    #[test]
    fn 編集しても段落の帯と枠が残る() {
        // set_body_text は段落をまるごと写すので、新しい性質も自動で残る
        let mut d = Document::plain("見出し\n本文", 10.5);
        if let Block::Para(p) = &mut d.blocks[0] {
            p.shade = Some("DEEAF6".into());
            p.boxed = true;
        }
        d.set_body_text("見出しに追記\n本文", 10.5);
        let p = d.paragraphs().next().unwrap();
        assert_eq!(p.shade.as_deref(), Some("DEEAF6"), "1文字打つだけで帯が消えた");
        assert!(p.boxed, "枠が消えた");
    }
}

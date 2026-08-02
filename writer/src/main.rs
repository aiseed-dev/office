//! writer — docx互換のワープロ。calc とは**別のソフト**。
//!
//! 一つの巨大なスイートにしない。文書は writer、表計算は calc。
//! 共有するのは書式(docx/xlsx)と核(kumihan)、そして入力の結線(ui)だけ。
//!
//! **マクロは無い。** 文書の中に実行コードを置かないので、
//! 「開く=実行」という攻撃経路が最初から存在しない。
//!
//!   writer            空で開く
//!   writer 文書.docx  その文書を開く
//!
//! 打てる: 日本語(IME)・BackSpace/Delete・矢印・Shift+矢印で選択・Ctrl+A・
//!         Enter で改段落・Ctrl+Z/Ctrl+Shift+Z・Ctrl+S 保存・Ctrl+O 開く

use std::ops::Range;
use std::path::PathBuf;

use gpui::{
    div, prelude::*, px, rgb, size, App, Bounds, Context, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, SharedString, UTF16Selection, Window,
    WindowBounds, WindowOptions,
};
use gpui_platform::application;
use kumihan::{layout, Align, Document, Editor, Frame, ListKind, Metrics, Sheet as Page};
use ui::{handler, ribbon, HasEditor};

/// 本文のフォント。**同梱せず、システムから探す**
/// (埋め込むと実行ファイルがフォントを配ることになり、免許の表示義務も付く)。
///
/// 起動時に一度だけ読み、以後は借りて使う。
/// 見つからなければ**その場で止める** — 日本語が豆腐になった画面を
/// 「動いている」と見せない。
fn font_data() -> &'static [u8] {
    static FONT: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    FONT.get_or_init(|| {
        {
            // 文書が書体を指定していればそれを、無ければ機械にある日本語フォントを
            let (fam, _) = kumihan::font::for_document(None).unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            });
            kumihan::font::load(fam).unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            })
        }
    })
}

/// `RRGGBB` の1成分を 0.0〜1.0 で返す。読めない色は黒として扱う
fn hex(s: &str, i: usize) -> f32 {
    s.get(i * 2..i * 2 + 2)
        .and_then(|h| u8::from_str_radix(h, 16).ok())
        .map(|v| v as f32 / 255.0)
        .unwrap_or(0.0)
}

/// セルの文章(段落を \n で繋いだもの)。
fn cell_text(c: &kumihan::Cellbox) -> String {
    c.paragraphs
        .iter()
        .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// セルへ文章を戻す。段落ごとの書式は同じ位置から引き継ぐ(本文と同じ規則)。
fn set_cell_text(c: &mut kumihan::Cellbox, text: &str) {
    let old: Vec<kumihan::Paragraph> = c.paragraphs.clone();
    c.paragraphs = text
        .split('\n')
        .enumerate()
        .map(|(i, s)| {
            let mut p = old.get(i).cloned().unwrap_or_default();
            let (size, font, fmt) = p
                .runs
                .first()
                .map(|r| (r.size_pt, r.font.clone(), r.fmt.clone()))
                .unwrap_or((SIZE_PT, None, Default::default()));
            p.runs = vec![kumihan::Run { text: s.to_string(), size_pt: size, font, fmt }];
            p
        })
        .collect();
}

const PX_PER_MM: f32 = 96.0 / 25.4;
const MARGIN_MM: f32 = 20.0;
const MEASURE_MM: f32 = 210.0 - 2.0 * MARGIN_MM;
const SIZE_PT: f32 = 10.5;
const LINE_MM: f32 = 6.4;
const Y0_MM: f32 = 24.0;

/// いま編集しているもの。本文か、表のセルか。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    Body,
    Cell { table: usize, row: usize, col: usize },
}

struct Writer {
    focus: FocusHandle,
    doc: Document,
    ed: Editor,
    page: Page,
    path: Option<PathBuf>,
    status: SharedString,
    notes: Vec<SharedString>,
    dirty: bool,
    /// 選んでいるリボンのタブ
    tab: usize,
    /// 画面に使う書体名(文書の指定に従う)
    font_name: SharedString,
    /// 画面の倍率。**紙は変わらない** — 見る大きさだけの話
    zoom: f32,
    /// いま編集しているもの。**Editor は常にこの対象の文章を持つ**
    target: Target,
    /// 記号の一覧を出しているか
    symbols: bool,
    /// 置換の板。開いている間、打鍵は検索欄に入る
    find_open: bool,
    /// 0=検索語 1=置換後
    find_field: usize,
    find_ed: Editor,
    repl_ed: Editor,
    /// 校正の指摘(レビュー > 校正)。英語は辞書、日本語はモデル
    proof: Vec<ui::check::Finding>,
    proof_msg: SharedString,
    /// 辞書は起動時に1回だけ読む
    checker: ui::check::Checker,
}

impl HasEditor for Writer {
    fn editor(&mut self) -> &mut Editor {
        // 置換の板が開いている間、入力(IME含む)は検索欄へ入る。
        // 別の入力部品を作らず、同じ Editor と結線を使い回す
        if self.find_open {
            if self.find_field == 0 { &mut self.find_ed } else { &mut self.repl_ed }
        } else {
            &mut self.ed
        }
    }
    fn editor_ref(&self) -> &Editor {
        if self.find_open {
            if self.find_field == 0 { &self.find_ed } else { &self.repl_ed }
        } else {
            &self.ed
        }
    }
    fn on_edited(&mut self) {
        if self.find_open {
            // 検索欄への打鍵は文書を変えない
            return;
        }
        self.dirty = true;
        self.relayout();
    }
}

impl Writer {
    fn new(path: Option<PathBuf>, cx: &mut Context<Self>) -> Writer {
        let mut w = Writer {
            focus: cx.focus_handle(),
            doc: Document::default(),
            ed: Editor::new(""),
            page: Page::default(),
            path: None,
            status: "".into(),
            notes: Vec::new(),
            dirty: false,
            tab: 0,
            zoom: 1.0,
            target: Target::Body,
            symbols: false,
            find_open: false,
            find_field: 0,
            find_ed: Editor::new(""),
            repl_ed: Editor::new(""),
            font_name: kumihan::font::for_document(None)
                .map(|(f, _)| SharedString::from(f.name.clone()))
                .unwrap_or_else(|_| "sans-serif".into()),
            proof: Vec::new(),
            proof_msg: "".into(),
            checker: ui::check::Checker::default(),
        };
        match path {
            Some(p) => w.open(p),
            None => {
                w.set_doc(Document::plain(
                    "ここに打てます。日本語入力(IME)もそのまま使えます。\n\
                     Ctrl+S で docx として保存、Ctrl+O で開く。マクロはありません。",
                    SIZE_PT,
                ));
                w.dirty = false;
            }
        }
        w
    }

    fn set_doc(&mut self, doc: Document) {
        self.ed = Editor::new(&doc.body_text());
        self.doc = doc;
        self.relayout();
    }

    /// 編集中のテキストを文書に反映してから組み直す。
    /// いまの編集内容を、編集先(本文かセル)へ書き戻す。
    fn flush_target(&mut self) {
        match self.target {
            Target::Body => self.doc.set_body_text(self.ed.text(), SIZE_PT),
            Target::Cell { table, row, col } => {
                let text = self.ed.text().to_string();
                if let Some(kumihan::Block::Table(tb)) = self
                    .doc
                    .blocks
                    .iter_mut()
                    .filter(|b| matches!(b, kumihan::Block::Table(_)))
                    .nth(table)
                {
                    if let Some(cell) = tb.rows.get_mut(row).and_then(|r| r.get_mut(col)) {
                        set_cell_text(cell, &text);
                    }
                }
            }
        }
    }

    /// 編集先を切り替える。いまの内容を書き戻してから、次の文章を持つ。
    fn switch_target(&mut self, next: Target) {
        if self.target == next {
            return;
        }
        self.flush_target();
        self.target = next;
        let text = match next {
            Target::Body => self.doc.body_text(),
            Target::Cell { table, row, col } => self
                .doc
                .tables()
                .nth(table)
                .and_then(|t| t.rows.get(row))
                .and_then(|r| r.get(col))
                .map(cell_text)
                .unwrap_or_default(),
        };
        self.ed = Editor::new(&text);
        self.status = match next {
            Target::Body => "本文".into(),
            Target::Cell { row, col, .. } => {
                format!("表のセル({}行 {}列)を編集中", row + 1, col + 1).into()
            }
        };
    }

    fn relayout(&mut self) {
        self.flush_target();
        let m = Metrics::new(font_data()).expect("フォント");
        self.page = layout(
            &self.doc,
            &m,
            &Frame { measure_mm: MEASURE_MM, line_height_mm: LINE_MM, y0_mm: Y0_MM },
        );
    }

    fn open(&mut self, p: PathBuf) {
        self.target = Target::Body;
        match std::fs::File::open(&p)
            .map_err(|e| e.to_string())
            .and_then(ooxml::read)
        {
            Ok((doc, rep)) => {
                self.notes = rep
                    .unsupported
                    .iter()
                    .map(|(n, c)| SharedString::from(format!("{n} × {c}")))
                    .collect();
                self.status = format!(
                    "{} 段落 / 表 {} — {}",
                    rep.paragraphs,
                    doc.tables().count(),
                    p.file_name().unwrap_or_default().to_string_lossy()
                )
                .into();
                self.set_doc(doc);
                self.path = Some(p);
                self.dirty = false;
            }
            Err(e) => self.status = format!("開けません: {e}").into(),
        }
    }

    fn save(&mut self) {
        let p = match self.path.clone() {
            Some(p) => Some(p),
            None => rfd::FileDialog::new()
                .add_filter("Word文書", &["docx"])
                .save_file(),
        };
        let Some(p) = p else { return };
        self.flush_target();
        // 元のファイルの部品(画像・スタイル・ヘッダー等)を持ち越す。
        // 上書き保存では読み終えてから書く(同じファイルを同時に開かない)
        let original: Option<std::io::Cursor<Vec<u8>>> = self
            .path
            .as_ref()
            .and_then(|old| std::fs::read(old).ok())
            .map(std::io::Cursor::new);
        match std::fs::File::create(&p)
            .map_err(|e| e.to_string())
            .and_then(|f| ooxml::write_with(&self.doc, original, std::io::BufWriter::new(f)))
        {
            Ok(_) => {
                let caveat = if self.notes.is_empty() {
                    ""
                } else {
                    // 読めなかった要素は本文から消えている。黙って保存しない
                    "(読めなかった要素は本文に戻りません)"
                };
                self.status = format!(
                    "保存しました — {}{caveat}",
                    p.file_name().unwrap_or_default().to_string_lossy()
                )
                .into();
                self.path = Some(p);
                self.dirty = false;
            }
            Err(e) => self.status = format!("保存できません: {e}").into(),
        }
    }

    /// 文字位置 → 紙の上の座標(キャレットを出すため)
    fn caret_xy(&self) -> (f32, f32) {
        let cur = self.ed.cursor();
        // 行の頭のバイト位置(byte0)は組版が持っている。
        // 行の文字数で数え直すと、折り返しで落ちた空白や空行でずれる。
        // 折り返し・段落の境目では**後ろの行**に立てる(Enter の直後は次の行)
        let want = match self.target {
            Target::Body => None,
            Target::Cell { table, row, col } => Some((table, row, col)),
        };
        let mut hit: Option<(f32, f32)> = None;
        for line in self.page.lines.iter().filter(|l| match want {
            None => l.from_body,
            Some(id) => l.cell == Some(id),
        }) {
            if cur < line.byte0 {
                continue;
            }
            if cur > line.byte_end() + 1 {
                continue;
            }
            let within = cur.saturating_sub(line.byte0);
            let base = line.cells.iter().map(|c| c.off).min().unwrap_or(0);
            let x = line
                .cells
                .iter()
                .find(|c| c.off - base >= within)
                .map(|c| c.x_mm)
                .or_else(|| line.cells.last().map(|c| c.x_mm + c.w_mm))
                .unwrap_or(0.0);
            hit = Some((MARGIN_MM + x, line.y_mm));
        }
        hit.unwrap_or((
            MARGIN_MM,
            self.page.lines.last().map(|l| l.y_mm).unwrap_or(Y0_MM),
        ))
    }

    /// レビュー > 校正。**英語は辞書、日本語はモデル。**
    ///
    /// 英語の綴り誤りは辞書に無い語になるので辞書で捕まる(GPU も要らない)。
    /// 日本語の誤変換は辞書に有る語になるので、辞書では原理的に捕まらない。
    ///
    /// 検査できなかった部分があれば必ずそう出す — **黙って「指摘なし」にしない**
    /// (利用者は「誤りが無い」と受け取ってしまう)。
    fn run_proof(&mut self) {
        let r = self.checker.check(self.ed.text());
        self.proof_msg = r.summary().into();
        self.proof = r.findings;
    }

    /// 編集中のセルの段落へ書式を掛ける(セルは短いので丸ごと掛ける)。
    fn each_cell_para(&mut self, f: impl Fn(&mut kumihan::Paragraph)) {
        let Target::Cell { table, row, col } = self.target else { return };
        self.flush_target();
        if let Some(kumihan::Block::Table(tb)) = self
            .doc
            .blocks
            .iter_mut()
            .filter(|b| matches!(b, kumihan::Block::Table(_)))
            .nth(table)
        {
            if let Some(cell) = tb.rows.get_mut(row).and_then(|r| r.get_mut(col)) {
                for p in &mut cell.paragraphs {
                    f(p);
                }
            }
        }
    }

    /// 選択している段落の文字書式を入切する。
    ///
    /// **編集先が本文かセルかで掛け先が違う。** セル編集中に本文へ掛けると、
    /// set_body_text がセルの文章で本文を上書きしてしまう。
    fn toggle(&mut self, f: impl Fn(&mut kumihan::CharFormat) + Copy) {
        match self.target {
            Target::Body => {
                let sel = self.ed.selection();
                self.doc.set_body_text(self.ed.text(), SIZE_PT);
                self.doc.apply_char_format(sel, f);
            }
            Target::Cell { .. } => self.each_cell_para(|p| {
                for r in &mut p.runs {
                    f(&mut r.fmt);
                }
            }),
        }
        self.dirty = true;
        self.relayout_keep();
    }

    /// 選んでいる段落の性質を変える。
    fn para(&mut self, f: impl Fn(&mut kumihan::Paragraph) + Copy) {
        match self.target {
            Target::Body => {
                let sel = self.ed.selection();
                self.doc.set_body_text(self.ed.text(), SIZE_PT);
                self.doc.apply_para(sel, f);
            }
            Target::Cell { .. } => self.each_cell_para(f),
        }
        self.dirty = true;
        self.relayout_keep();
    }

    fn size(&mut self, f: impl Fn(f32) -> f32 + Copy) {
        match self.target {
            Target::Body => {
                let sel = self.ed.selection();
                self.doc.set_body_text(self.ed.text(), SIZE_PT);
                self.doc.apply_size(sel, f);
            }
            Target::Cell { .. } => self.each_cell_para(|p| {
                for r in &mut p.runs {
                    r.size_pt = f(r.size_pt).clamp(4.0, 400.0);
                }
            }),
        }
        self.dirty = true;
        self.relayout_keep();
    }

    /// PDF として保存。**画面に出しているのと同じ紙面を写す**ので、
    /// 画面と紙が食い違わない。
    fn save_pdf(&mut self) {
        let Some(p) = rfd::FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .set_file_name("文書.pdf")
            .save_file()
        else {
            return;
        };
        let r = std::fs::File::create(&p)
            .map_err(|e| e.to_string())
            .and_then(|f| {
                paper::to_pdf(
                    &self.page,
                    font_data(),
                    paper::Paper { margin_mm: MARGIN_MM, ..Default::default() },
                    std::io::BufWriter::new(f),
                )
            });
        self.status = match r {
            Ok(_) => format!("PDF にしました — {}", p.file_name().unwrap_or_default().to_string_lossy()).into(),
            Err(e) => format!("PDF にできません: {e}").into(),
        };
    }

    fn set_align(&mut self, a: Align) {
        match self.target {
            Target::Body => {
                let sel = self.ed.selection();
                self.doc.set_body_text(self.ed.text(), SIZE_PT);
                self.doc.apply_align(sel, a);
            }
            Target::Cell { .. } => self.each_cell_para(|p| p.align = a),
        }
        self.dirty = true;
        self.relayout_keep();
    }

    /// 書式を触ったあとの組み直し。**本文を戻さない**
    /// (戻すと今つけた書式が消える)。
    fn relayout_keep(&mut self) {
        let data = font_data();
        let m = Metrics::new(data).expect("フォント");
        self.page = layout(
            &self.doc,
            &m,
            &Frame { measure_mm: MEASURE_MM, line_height_mm: LINE_MM, y0_mm: Y0_MM },
        );
    }

    /// クリックした画素位置(編集領域からの相対)にカーソルを置く。
    fn click_at(&mut self, rel_x: f32, rel_y: f32, extend: bool) {
        let pxmm = PX_PER_MM * self.zoom;
        // 紙は編集領域の (28,14)px に置いてある
        let x_mm = (rel_x - 28.0) / pxmm - MARGIN_MM;
        let y_mm = (rel_y - 14.0) / pxmm;

        // 表のセルの中なら、そのセルの編集に切り替える
        let hit_box = self.page.cell_boxes.iter().find(|b| {
            x_mm >= b.x_mm && x_mm <= b.x_mm + b.w_mm
                && y_mm >= b.top_mm && y_mm <= b.top_mm + b.h_mm
        }).copied();
        if let Some(b) = hit_box {
            let id = Target::Cell { table: b.table, row: b.row, col: b.col };
            self.switch_target(id);
            // セルの中の行で位置を決める
            let mut hit = 0usize;
            for line in &self.page.lines {
                if line.cell != Some((b.table, b.row, b.col)) {
                    continue;
                }
                if line.y_mm - LINE_MM * 0.8 > y_mm {
                    continue;
                }
                hit = line.byte0;
                let base = line.cells.iter().map(|c| c.off).min().unwrap_or(0);
                let mut x = line.cells.first().map(|c| c.x_mm - MARGIN_MM).unwrap_or(0.0);
                for c in &line.cells {
                    if x_mm < x + c.w_mm / 2.0 {
                        break;
                    }
                    x += c.w_mm;
                    hit = line.byte0 + (c.off + c.ch.len_utf8()) - base;
                }
            }
            let hit = hit.min(self.ed.text().len());
            self.ed.move_to(hit, extend);
            return;
        }
        // 本文をクリックした。セルを編集していたら本文へ戻る
        self.switch_target(Target::Body);

        // 一番近いベースラインの本文行を選ぶ(クリックは字の少し上に落ちる)
        let target = y_mm + LINE_MM * 0.3;
        let mut best: Option<(f32, usize)> = None; // (距離, 本文行の通し番号)
        let mut nth = 0usize;
        for line in &self.page.lines {
            if !line.from_body {
                continue;
            }
            let d = (line.y_mm - target).abs();
            if best.map_or(true, |(bd, _)| d < bd) {
                best = Some((d, nth));
            }
            nth += 1;
        }
        let Some((_, want)) = best else { return };

        // 行が持つバイト位置から出す(文字数で数え直さない)
        let mut byte = 0usize;
        let mut nth = 0usize;
        for line in &self.page.lines {
            if !line.from_body {
                continue;
            }
            if nth == want {
                byte = line.byte0;
                let base = line.cells.iter().map(|c| c.off).min().unwrap_or(0);
                let mut x = line.cells.first().map(|c| c.x_mm).unwrap_or(0.0);
                for c in &line.cells {
                    if x_mm < x + c.w_mm / 2.0 {
                        break;
                    }
                    x += c.w_mm;
                    byte = line.byte0 + (c.off + c.ch.len_utf8()) - base;
                }
                break;
            }
            nth += 1;
        }
        let byte = byte.min(self.ed.text().len());
        self.ed.move_to(byte, extend);
    }

    /// 次の一致を選ぶ(カーソルの後ろから。末尾まで無ければ頭から一周)。
    fn find_next(&mut self) {
        let term = self.find_ed.text().to_string();
        if term.is_empty() {
            self.status = "検索語が空です".into();
            return;
        }
        let text = self.ed.text().to_string();
        let from = self.ed.selection().end;
        let hit = text[from..]
            .find(&term)
            .map(|i| from + i)
            .or_else(|| text.find(&term));
        match hit {
            Some(i) => {
                self.ed.move_to(i, false);
                self.ed.move_to(i + term.len(), true);
                self.status = "".into();
            }
            None => self.status = format!("「{term}」は見つかりません").into(),
        }
    }

    /// いま選ばれている一致を置き換えて、次へ。
    fn replace_current(&mut self) {
        let term = self.find_ed.text().to_string();
        let repl = self.repl_ed.text().to_string();
        if term.is_empty() {
            return;
        }
        let sel = self.ed.selection();
        let selected: String = self.ed.text()[sel.clone()].to_string();
        if selected == term {
            self.ed.insert(&repl);
            self.dirty = true;
            self.relayout();
        }
        self.find_next();
    }

    /// 全部置き換える。**何件変えたかを言う**(黙って書き換えない)。
    fn replace_all(&mut self) {
        let term = self.find_ed.text().to_string();
        let repl = self.repl_ed.text().to_string();
        if term.is_empty() {
            return;
        }
        let mut n = 0usize;
        loop {
            let text = self.ed.text().to_string();
            let Some(i) = text.find(&term) else { break };
            self.ed.move_to(i, false);
            self.ed.move_to(i + term.len(), true);
            self.ed.insert(&repl);
            n += 1;
            if n > 100_000 {
                break; // 置換後が検索語を含むと止まらなくなるのを防ぐ
            }
        }
        if n > 0 {
            self.dirty = true;
            self.relayout();
        }
        self.status = format!("{n} 件を置き換えました").into();
    }

    fn run_cmd(&mut self, id: &str) {
        match id {
            "open" => {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("Word文書", &["docx"]).pick_file() { self.open(p) }
            }
            "save" => self.save(),
            "undo" => { if self.ed.undo() { self.on_edited() } }
            "redo" => { if self.ed.redo() { self.on_edited() } }
            "selectall" => self.ed.select_all(),
            "spell" => self.run_proof(),
            // 文字書式 — 押すたびに入切する(Word と同じ挙動)
            "bold" => self.toggle(|f| f.bold = !f.bold),
            "italic" => self.toggle(|f| f.italic = !f.italic),
            "underline" => self.toggle(|f| f.underline = !f.underline),
            "strikeout" => self.toggle(|f| f.strike = !f.strike),
            // 段落の揃え
            "align-left" => self.set_align(Align::Left),
            "align-center" => self.set_align(Align::Center),
            "align-right" => self.set_align(Align::Right),
            "align-just" => self.set_align(Align::Justify),
            // 文字の大きさ
            "incfont" => self.size(|s| s + 1.0),
            "decfont" => self.size(|s| s - 1.0),
            // 印刷・PDF。**組み直さない** — 画面と同じ紙面をそのまま写す
            "pdf" => self.save_pdf(),
            // 文字色。押すたびに 赤 → 青 → 黒(解除)と回す。
            // 色を選ぶ小窓はまだ無いので、**無い機能を有るように見せず**
            // 使える範囲で回す形にしてある
            // 箇条書き・段落番号。押すたびに入切する
            "markers" => self.para(|p| {
                p.list = if p.list == ListKind::Bullet { ListKind::None } else { ListKind::Bullet }
            }),
            "numbering" => self.para(|p| {
                p.list = if p.list == ListKind::Number { ListKind::None } else { ListKind::Number }
            }),
            // インデント。0〜20段に留める
            "incoffset" => self.para(|p| p.indent = (p.indent + 1).min(20)),
            "decoffset" => self.para(|p| p.indent = p.indent.saturating_sub(1)),
            // 行間。1.0 → 1.5 → 2.0 → 1.0 と回す(小窓がまだ無いので)
            // この段落の前で改ページ(押すたびに入切)
            "pagebreak" => self.para(|p| p.page_break_before = !p.page_break_before),
            // 表の挿入。3×3 を末尾に(大きさを選ぶ小窓はまだ無い)。
            // セル編集が入っているので、挿した表はそのまま書ける
            "instable" => {
                let empty = || kumihan::Cellbox {
                    paragraphs: vec![kumihan::Paragraph {
                        runs: vec![kumihan::Run {
                            text: String::new(),
                            size_pt: SIZE_PT,
                            font: None,
                            fmt: Default::default(),
                        }],
                        ..Default::default()
                    }],
                };
                self.flush_target();
                self.doc.blocks.push(kumihan::Block::Table(kumihan::Table {
                    col_mm: vec![],
                    rows: (0..3).map(|_| (0..3).map(|_| empty()).collect()).collect(),
                }));
                self.dirty = true;
                self.relayout_keep();
                self.status = "3×3 の表を末尾に入れました(セルをクリックで編集)".into();
            }
            // 記号の一覧(押すと出る/消える)
            "inssymbol" => self.symbols = !self.symbols,
            // 置換の板。開いている間、打鍵は検索欄に入る
            "replace" => {
                self.find_open = !self.find_open;
                self.find_field = 0;
                if self.find_open {
                    self.switch_target(Target::Body);
                    self.status = "検索語を打って Enter で次へ".into();
                }
            }
            // 画面の倍率。50〜200%。紙は変わらない
            "zoom-in" => self.zoom = (self.zoom + 0.1).min(2.0),
            "zoom-out" => self.zoom = (self.zoom - 0.1).max(0.5),
            "linespace" => self.para(|p| {
                p.line_spacing = match p.spacing() {
                    s if s < 1.25 => 1.5,
                    s if s < 1.75 => 2.0,
                    _ => 1.0,
                }
            }),
            // 文字カウント。日本語は「単語数」に意味が無いので**文字数**を出す
            "wordcount" => {
                let text = self.ed.text();
                let all = text.chars().filter(|c| *c != '\n').count();
                let ink = text.chars().filter(|c| !c.is_whitespace()).count();
                let paras = text.split('\n').filter(|s| !s.trim().is_empty()).count();
                self.status = format!(
                    "文字数 {ink}(空白込み {all})/ 段落 {paras}").into();
            }
            "fontcolor" => self.toggle(|f| {
                f.color = match f.color.as_deref() {
                    None => Some("C00000".into()),
                    Some("C00000") => Some("1F4E79".into()),
                    _ => None,
                }
            }),
            _ => {}
        }
    }

    // ---- 割り当てられた操作 ----
    fn backspace(&mut self, _: &ui::Backspace, _: &mut Window, cx: &mut Context<Self>) {
        self.editor().backspace();
        self.on_edited();
        cx.notify();
    }
    fn delete(&mut self, _: &ui::Delete, _: &mut Window, cx: &mut Context<Self>) {
        self.editor().delete();
        self.on_edited();
        cx.notify();
    }
    fn left(&mut self, _: &ui::Left, _: &mut Window, cx: &mut Context<Self>) {
        self.editor().move_char(false, false);
        cx.notify();
    }
    fn right(&mut self, _: &ui::Right, _: &mut Window, cx: &mut Context<Self>) {
        self.editor().move_char(true, false);
        cx.notify();
    }
    fn select_left(&mut self, _: &ui::SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.editor().move_char(false, true);
        cx.notify();
    }
    fn select_right(&mut self, _: &ui::SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.editor().move_char(true, true);
        cx.notify();
    }
    fn select_all(&mut self, _: &ui::SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.editor().select_all();
        cx.notify();
    }
    fn home(&mut self, _: &ui::Home, _: &mut Window, cx: &mut Context<Self>) {
        self.editor().move_to(0, false);
        cx.notify();
    }
    fn end(&mut self, _: &ui::End, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.editor_ref().text().len();
        self.editor().move_to(n, false);
        cx.notify();
    }
    fn enter(&mut self, _: &ui::Enter, _: &mut Window, cx: &mut Context<Self>) {
        if self.find_open {
            self.find_next();
        } else {
            self.editor().insert("\n");
            self.on_edited();
        }
        cx.notify();
    }
    fn undo(&mut self, _: &ui::Undo, _: &mut Window, cx: &mut Context<Self>) {
        if self.ed.undo() {
            self.on_edited();
        }
        cx.notify();
    }
    fn redo(&mut self, _: &ui::Redo, _: &mut Window, cx: &mut Context<Self>) {
        if self.ed.redo() {
            self.on_edited();
        }
        cx.notify();
    }
    fn do_save(&mut self, _: &ui::Save, _: &mut Window, cx: &mut Context<Self>) {
        self.save();
        cx.notify();
    }
    fn do_open(&mut self, _: &ui::Open, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = rfd::FileDialog::new()
            .add_filter("Word文書", &["docx"])
            .pick_file()
        {
            self.open(p);
        }
        cx.notify();
    }
}

impl Focusable for Writer {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EntityInputHandler for Writer {
    fn text_for_range(
        &mut self,
        r: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _w: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        handler::text_for_range(self, r, actual)
    }
    fn selected_text_range(
        &mut self,
        _ignore: bool,
        _w: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: handler::selected_range_utf16(self), reversed: false })
    }
    fn marked_text_range(&self, _w: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
        handler::marked_range_utf16(self)
    }
    fn unmark_text(&mut self, _w: &mut Window, _cx: &mut Context<Self>) {
        handler::unmark(self);
    }
    fn replace_text_in_range(
        &mut self,
        r: Option<Range<usize>>,
        text: &str,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        handler::replace(self, r, text);
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        r: Option<Range<usize>>,
        text: &str,
        sel: Option<Range<usize>>,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        handler::replace_and_mark(self, r, text, sel);
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        _r: Range<usize>,
        bounds: Bounds<gpui::Pixels>,
        _w: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<gpui::Pixels>> {
        // IME の候補窓をキャレットの下に出す
        let (x, y) = self.caret_xy();
        Some(Bounds::new(
            gpui::point(
                bounds.origin.x + px(28.0 + x * PX_PER_MM),
                bounds.origin.y + px(y * PX_PER_MM),
            ),
            size(px(2.0), px(SIZE_PT * 96.0 / 72.0)),
        ))
    }
    fn character_index_for_point(
        &mut self,
        _p: gpui::Point<gpui::Pixels>,
        _w: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
    fn text_length_utf16(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> Option<usize> {
        Some(handler::text_len_utf16(self))
    }
}

impl Render for Writer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let me: Entity<Writer> = cx.entity();
        // 画面の倍率(紙のミリは変えず、画素への写像だけ変える)
        let pxmm = PX_PER_MM * self.zoom;
        let marked = self.ed.marked_range();
        let (cx_mm, cy_mm) = self.caret_xy();

        // ---- リボン(Euro-Office に名前と並びを合わせる) ----
        let (ready, all) = ribbon::progress(ribbon::WRITER);
        let mut tabs = div().flex().flex_row().items_end().gap_1()
            .px_3().pt_1p5().bg(rgb(0x165E83));
        for (i, tb) in ribbon::WRITER.iter().enumerate() {
            let on = i == self.tab;
            tabs = tabs.child(div()
                .id(SharedString::from(format!("tab{i}")))
                .px_3().py_1p5()
                .rounded_t_md()
                .bg(if on { rgb(0xFFFFFF) } else { rgb(0x165E83) })
                .text_color(if on { rgb(0x165E83) } else { rgb(0xCFE0EA) })
                .text_size(px(12.0))
                .font_weight(if on { gpui::FontWeight::BOLD } else { gpui::FontWeight::NORMAL })
                .cursor_pointer()
                .hover(|s| s.text_color(rgb(0xFFFFFF)))
                .child(tb.name)
                .on_click(cx.listener(move |this, _, _, cx| { this.tab = i; cx.notify() })));
        }
        tabs = tabs.child(div().flex_1())
            .child(div().pb_1p5().pr_1().text_size(px(10.5)).text_color(rgb(0x8FB8CC))
                   .child(SharedString::from(format!("writer — 実装済み {ready}/{all}"))));

        let mut cmds = div().flex().flex_row().flex_wrap().gap_1().items_center()
            .px_3().py_2().bg(gpui::white())
            .border_b_1().border_color(rgb(0xE1E6EA));
        for cmd in ribbon::WRITER[self.tab].cmds {
            if cmd.ready {
                let id = cmd.id;
                cmds = cmds.child(div()
                    .id(SharedString::from(cmd.id))
                    .px_3().py_1().rounded_md()
                    .border_1().border_color(rgb(0x165E83)).text_color(rgb(0x165E83))
                    .text_size(px(12.0)).cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF2F7)))
                    .flex().flex_row().items_center().gap_1()
                    .children(ui::icons::find(cmd.icon).map(|_| {
                        gpui::svg()
                            .path(SharedString::from(format!("icons/{}.svg", cmd.icon)))
                            .size(px(15.0))
                            .text_color(rgb(0x165E83))
                    }))
                    .child(cmd.label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.run_cmd(id); cx.notify()
                    })));
            } else {
                // 未実装。押せるように見せない
                cmds = cmds.child(div().px_3().py_1().rounded_md()
                    .border_1().border_color(rgb(0xEDEFF1))
                    .text_color(rgb(0xB6BDC4)).text_size(px(12.0))
                    .flex().flex_row().items_center().gap_1()
                    .children(ui::icons::find(cmd.icon).map(|_| {
                        gpui::svg()
                            .path(SharedString::from(format!("icons/{}.svg", cmd.icon)))
                            .size(px(15.0))
                            .text_color(rgb(0xB6BDC4))
                    }))
                    .child(cmd.label));
            }
        }
        cmds = cmds.child(div().flex_1())
            .child(div().text_size(px(11.0)).text_color(rgb(0x66707A))
                   .child(SharedString::from(format!("{}{}",
                       if self.dirty { "● " } else { "" }, self.status))));
        let bar = div().flex().flex_col().child(tabs).child(cmds);

        let mut paper = div().absolute().left(px(28.0)).top(px(14.0))
            .w(px(210.0 * pxmm)).h(px(297.0 * pxmm))
            .bg(gpui::white()).shadow_lg();

        // 表の罫線。紙面の座標をそのまま引く
        for r in &self.page.rules {
            let [x1, y1, x2, y2] = *r;
            let (x1, y1) = ((MARGIN_MM + x1) * pxmm, y1 * pxmm);
            let (x2, y2) = ((MARGIN_MM + x2) * pxmm, y2 * pxmm);
            paper = paper.child(div().absolute()
                .left(px(x1.min(x2))).top(px(y1.min(y2)))
                .w(px((x2 - x1).abs().max(1.0))).h(px((y2 - y1).abs().max(1.0)))
                .bg(rgb(0x444B52)));
        }

        // 未確定(変換中)の下線は、行が持つバイト位置(byte0)で結ぶ
        for line in &self.page.lines {
            if line.cells.is_empty() {
                continue;
            }
            let text = line.text();
            let pt = line.cells[0].size_pt;
            let sz = pt * 96.0 / 72.0 * self.zoom;
            let x0 = MARGIN_MM + line.cells[0].x_mm;
            let top = line.y_mm * pxmm - sz * 0.88;

            if let Some(m) = &marked {
                let mine = match self.target {
                    Target::Body => line.from_body,
                    Target::Cell { table, row, col } => line.cell == Some((table, row, col)),
                };
                if !mine {
                    // 編集していない行に変換下線は出さない
                } else {
                let (ls, le) = (line.byte0, line.byte_end());
                if m.start < le && m.end > ls {
                    let a = m.start.max(ls) - ls;
                    let b = m.end.min(le) - ls;
                    let base = line.cells.iter().map(|c| c.off).min().unwrap_or(0);
                    let w = |upto: usize| -> f32 {
                        line.cells.iter()
                            .take_while(|c| c.off - base < upto)
                            .map(|c| c.w_mm)
                            .sum()
                    };
                    paper = paper.child(div().absolute()
                        .left(px((x0 + w(a)) * pxmm))
                        .top(px(top + sz * 1.05))
                        .w(px((w(b) - w(a)).max(1.0) * pxmm))
                        .h(px(2.0)).bg(rgb(0x165E83)));
                }
                }
            }
            // 書式は字が持っている。行の頭の字のものを行に掛ける
            // (段落まるごとに掛ける粒度なので、行の中で混ざらない)
            let f = &line.cells[0].fmt;
            let mut d = div().absolute()
                .left(px(x0 * pxmm)).top(px(top))
                .text_size(px(sz))
                .font_family(self.font_name.clone())
                .whitespace_nowrap()
                .child(SharedString::from(text.clone()));
            if f.bold {
                d = d.font_weight(gpui::FontWeight::BOLD);
            }
            if f.italic {
                d = d.italic();
            }
            d = match &f.color {
                Some(c) => d.text_color(gpui::Rgba {
                    r: hex(c, 0), g: hex(c, 1), b: hex(c, 2), a: 1.0,
                }),
                None => d.text_color(rgb(0x1B1B1B)),
            };
            paper = paper.child(d);
            // 下線・取り消し線は自分で引く(gpui の text に無い)
            let w_mm: f32 = line.cells.iter().map(|c| c.w_mm).sum();
            for (on, dy) in [(f.underline, sz * 1.05), (f.strike, sz * 0.35)] {
                if on {
                    paper = paper.child(div().absolute()
                        .left(px(x0 * pxmm)).top(px(top + dy))
                        .w(px(w_mm * pxmm)).h(px(1.0))
                        .bg(rgb(0x1B1B1B)));
                }
            }

        }
        // キャレット
        paper = paper.child(div().absolute()
            .left(px(cx_mm * pxmm))
            .top(px(cy_mm * pxmm - SIZE_PT * 96.0 / 72.0 * self.zoom * 0.88))
            .w(px(1.5)).h(px(SIZE_PT * 96.0 / 72.0 * self.zoom * 1.15))
            .bg(rgb(0x165E83)));

        // 置換の板
        let find_panel = if !self.find_open {
            None
        } else {
            let field = |label: &str, ed: &Editor, active: bool| {
                // caret は | で見せる(専用の入力部品を作らない割り切り)
                let mut s = ed.text().to_string();
                let cur = ed.cursor().min(s.len());
                if active {
                    s.insert(cur, '|');
                }
                div().flex().flex_row().items_center().gap_2()
                    .child(div().w(px(64.0)).text_size(px(11.5))
                        .text_color(rgb(0x66707A)).child(SharedString::from(label.to_string())))
                    .child(div().flex_1().px_2().py_1().rounded_sm()
                        .border_1()
                        .border_color(if active { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                        .bg(gpui::white())
                        .text_size(px(12.5))
                        .whitespace_nowrap().overflow_hidden()
                        .child(SharedString::from(s)))
            };
            let btn = |id: &str, label: &str| {
                div().id(SharedString::from(id.to_string()))
                    .px_2p5().py_1().rounded_sm()
                    .border_1().border_color(rgb(0x1B6E3C)).text_color(rgb(0x1B6E3C))
                    .text_size(px(11.5)).cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF5EE)))
                    .child(SharedString::from(label.to_string()))
            };
            Some(div().absolute().left(px(16.0)).top(px(8.0)).w(px(430.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_2()
                .child(field("検索", &self.find_ed, self.find_field == 0)
                    .id("find-f").cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| { this.find_field = 0; cx.notify() })))
                .child(field("置換後", &self.repl_ed, self.find_field == 1)
                    .id("find-r").cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| { this.find_field = 1; cx.notify() })))
                .child(div().flex().flex_row().gap_2()
                    .child(btn("f-next", "次へ (Enter)")
                        .on_click(cx.listener(|this, _, _, cx| { this.find_next(); cx.notify() })))
                    .child(btn("f-one", "置換")
                        .on_click(cx.listener(|this, _, _, cx| { this.replace_current(); cx.notify() })))
                    .child(btn("f-all", "すべて置換")
                        .on_click(cx.listener(|this, _, _, cx| { this.replace_all(); cx.notify() })))
                    .child(div().flex_1())
                    .child(btn("f-close", "閉じる")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.find_open = false; cx.notify()
                        })))))
        };

        // 記号の一覧。事務の書類で使うものだけ(飾りの絵文字は入れない)
        let symbol_panel = if !self.symbols {
            None
        } else {
            const SYMS: &[&str] = &[
                "〒", "※", "→", "←", "↑", "↓", "℃", "±", "×", "÷",
                "①", "②", "③", "④", "⑤", "⑥", "⑦", "⑧", "⑨", "⑩",
                "㈱", "㈲", "№", "〆", "〜", "…", "・", "「", "」", "『",
                "』", "【", "】", "○", "●", "◎", "△", "▲", "□", "■",
            ];
            let mut d = div().absolute().right(px(16.0)).top(px(8.0)).w(px(340.0))
                .p_2().rounded_md().bg(gpui::white())
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_row().flex_wrap().gap_1();
            for s in SYMS {
                d = d.child(div()
                    .id(SharedString::from(format!("sym-{s}")))
                    .w(px(28.0)).h(px(28.0)).rounded_sm()
                    .flex().items_center().justify_center()
                    .text_size(px(15.0)).cursor_pointer()
                    .hover(|st| st.bg(rgb(0xEAF2F7)))
                    .child(SharedString::from(*s))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ed.insert(s);
                        this.on_edited();
                        cx.notify();
                    })));
            }
            Some(d)
        };

        // 校正の指摘
        let proof_panel = if self.proof.is_empty() && self.proof_msg.is_empty() {
            None
        } else {
            let mut d = div().absolute().right(px(16.0)).bottom(px(16.0)).w(px(300.0))
                .p_3().rounded_md().bg(gpui::white())
                .border_1().border_color(rgb(0xC6CDD3))
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                       .text_color(rgb(0x165E83))
                       .child(SharedString::from(format!("校正 — {}", self.proof_msg))));
            for n in &self.proof {
                // どちらの道具が出したかを隠さない。辞書の指摘は GPU 無しで再現できる
                let tool = match n.source {
                    ui::check::Source::Dictionary => "辞書",
                    ui::check::Source::Model => "モデル",
                };
                let cand = if n.candidates.is_empty() {
                    "候補なし".to_string()
                } else {
                    n.candidates.join(" / ")
                };
                d = d.child(div().mt_1p5().text_size(px(11.5))
                    .child(SharedString::from(
                        format!("{} → {}  ({}・{tool})", n.found, cand, n.kind.label()))));
            }
            Some(d)
        };

        let notes = if self.notes.is_empty() { None } else {
            let mut n = div().absolute().right(px(16.0)).top(px(14.0)).w(px(270.0))
                .p_3().rounded_md().bg(rgb(0xFFF6E6))
                .border_1().border_color(rgb(0xE8D5A8))
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                       .text_color(rgb(0x8A4B00)).child("この版で読み飛ばしたもの"));
            for x in &self.notes {
                n = n.child(div().text_size(px(11.0)).text_color(rgb(0x8A4B00))
                            .child(x.clone()));
            }
            Some(n)
        };

        div().size_full().flex().flex_col().bg(rgb(0x63686D))
            .key_context("jo_edit")
            .track_focus(&self.focus)
            .on_action(cx.listener(Writer::backspace))
            .on_action(cx.listener(Writer::delete))
            .on_action(cx.listener(Writer::left))
            .on_action(cx.listener(Writer::right))
            .on_action(cx.listener(Writer::select_left))
            .on_action(cx.listener(Writer::select_right))
            .on_action(cx.listener(Writer::select_all))
            .on_action(cx.listener(Writer::home))
            .on_action(cx.listener(Writer::end))
            .on_action(cx.listener(Writer::enter))
            .on_action(cx.listener(Writer::undo))
            .on_action(cx.listener(Writer::redo))
            .on_action(cx.listener(Writer::do_save))
            .on_action(cx.listener(Writer::do_open))
            .child(bar)
            .child(
                div().flex_1().relative()
                    .child(paper)
                    .children(notes)
                    .children(find_panel)
                    .children(symbol_panel)
                    .children(proof_panel)
                    .child(InputSink { view: me }),
            )
    }
}

/// 入力ハンドラは **paint のときに窓へ差す**(GPUI の作法)。
/// 何も描かない要素だが、これが無いと IME もキー入力も届かない。
struct InputSink {
    view: Entity<Writer>,
}

impl IntoElement for InputSink {
    type Element = Self;
    fn into_element(self) -> Self { self }
}

impl gpui::Element for InputSink {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> { None }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> { None }

    fn request_layout(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, ()) {
        let mut style = gpui::Style::default();
        style.size.width = gpui::relative(1.0).into();
        style.size.height = gpui::relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<gpui::Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {}

    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<gpui::Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.view.read(cx).focus.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );
        // クリックでカーソルを置く。編集領域の座標を知っているのはここだけ
        let view = self.view.clone();
        window.on_mouse_event(move |e: &gpui::MouseDownEvent, phase, _w, cx| {
            if phase != gpui::DispatchPhase::Bubble
                || e.button != gpui::MouseButton::Left
                || !bounds.contains(&e.position)
            {
                return;
            }
            let rel = e.position - bounds.origin;
            view.update(cx, |w, cx| {
                w.click_at(f32::from(rel.x), f32::from(rel.y), e.modifiers.shift);
                cx.notify();
            });
        });
    }
}

fn main() {
    let arg = std::env::args().nth(1).map(PathBuf::from);
    application().with_assets(ui::Icons).run(move |cx: &mut App| {
        cx.text_system()
            .add_fonts(vec![std::borrow::Cow::Borrowed(font_data())])
            .expect("フォント登録");
        cx.bind_keys(ui::bindings("jo_edit"));
        let bounds = Bounds::centered(None, size(px(900.0), px(1000.0)), cx);
        let arg2 = arg.clone();
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |window, cx| {
                let view = cx.new(|cx| Writer::new(arg2.clone(), cx));
                window.focus(&view.focus_handle(cx), cx);
                view
            },
        )
        .unwrap();
        cx.activate(true);
    });
}

#[cfg(test)]
mod cell_edit_tests {
    use super::*;

    fn doc_with_table() -> Document {
        let cell = |s: &str| kumihan::Cellbox {
            paragraphs: vec![kumihan::Paragraph {
                runs: vec![kumihan::Run {
                    text: s.into(), size_pt: SIZE_PT, font: None, fmt: Default::default() }],
                ..Default::default()
            }],
        };
        let mut d = Document::plain("本文", SIZE_PT);
        d.blocks.push(kumihan::Block::Table(kumihan::Table {
            col_mm: vec![],
            rows: vec![vec![cell("品名"), cell("金額")]],
        }));
        d
    }

    #[test]
    fn セルの文章を読み書きできる() {
        let d = doc_with_table();
        let t = d.tables().next().unwrap();
        assert_eq!(cell_text(&t.rows[0][0]), "品名");
        let mut c = t.rows[0][0].clone();
        set_cell_text(&mut c, "型式\n数量");
        assert_eq!(c.paragraphs.len(), 2, "段落に割れていない");
        assert_eq!(cell_text(&c), "型式\n数量");
    }

    #[test]
    fn セルの書式は書き戻しで残る() {
        let d = doc_with_table();
        let mut c = d.tables().next().unwrap().rows[0][0].clone();
        c.paragraphs[0].align = kumihan::Align::Center;
        c.paragraphs[0].runs[0].fmt.bold = true;
        set_cell_text(&mut c, "直した");
        assert_eq!(c.paragraphs[0].align, kumihan::Align::Center, "揃えが消えた");
        assert!(c.paragraphs[0].runs[0].fmt.bold, "太字が消えた");
    }
}

#[cfg(test)]
mod find_tests {
    use super::*;

    fn w(text: &str) -> (Editor, Editor, Editor) {
        (Editor::new(text), Editor::new(""), Editor::new(""))
    }

    // find_next/replace の中身はエディタ操作の列なので、
    // ここでは検索の規則(後ろから・一周する)だけを関数で確かめる
    fn next_hit(text: &str, term: &str, from: usize) -> Option<usize> {
        text[from..].find(term).map(|i| from + i).or_else(|| text.find(term))
    }

    #[test]
    fn カーソルの後ろから探す() {
        let t = "誤りを直す。誤りは残さない。";
        let first = next_hit(t, "誤り", 0).unwrap();
        let second = next_hit(t, "誤り", first + "誤り".len()).unwrap();
        assert!(second > first);
    }

    #[test]
    fn 末尾まで無ければ頭から一周() {
        let t = "誤りを直す。";
        // 「直」の後ろ(文字境界)から探す。実物の from はカーソル位置なので常に境界
        let from = "誤りを直".len();
        let hit = next_hit(t, "誤り", from);
        assert_eq!(hit, Some(0), "一周していない");
    }

    #[test]
    fn 無ければ無いと言える() {
        assert_eq!(next_hit("本文", "存在しない", 0), None);
        let _ = w("x");
    }
}

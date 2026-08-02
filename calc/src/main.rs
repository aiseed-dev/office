//! calc — xlsx互換の表計算。writer とは**別のソフト**。
//!
//! Office を一つのソフトにしない。文書は writer、表は calc。
//! 共有するのは書式(docx/xlsx)だけ。
//!
//! **マクロは無い。** 表の中に実行コードを置かない設計で、
//! 「開く=実行」という攻撃経路を最初から持たない。
//!
//!   calc            空で開く
//!   calc 表.xlsx    その表を開く

use std::ops::Range;
use std::path::PathBuf;

use gpui::{
    div, prelude::*, px, rgb, size, App, Bounds, Context, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, SharedString, UTF16Selection, Window,
    WindowBounds, WindowOptions,
};
use gpui_platform::application;
use kumihan::Editor;

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
use sheet::model::{Borders, CellFormat, HAlign};
use sheet::{recalc, Book, Cell, Pos, Value};
use ui::{handler, ribbon, HasEditor};

const ROW_H: f32 = 24.0;
/// `RRGGBB` を色にする。読めなければ黒
fn hex(s: &str) -> gpui::Rgba {
    let g = |i: usize| {
        s.get(i * 2..i * 2 + 2)
            .and_then(|h| u8::from_str_radix(h, 16).ok())
            .map(|v| v as f32 / 255.0)
            .unwrap_or(0.0)
    };
    gpui::Rgba { r: g(0), g: g(1), b: g(2), a: 1.0 }
}

const COL_W: f32 = 108.0;
const HEAD_W: f32 = 46.0;
const ROWS: u32 = 30;
const COLS: u32 = 9;

struct Calc {
    focus: FocusHandle,
    book: Book,
    active: usize,
    cursor: Pos,
    /// 数式バーの中身。IMEもここに来る(セルの入力は1本のテキスト編集)
    input: Editor,
    path: Option<PathBuf>,
    status: SharedString,
    notes: Vec<SharedString>,
    dirty: bool,
    /// 選んでいるリボンのタブ
    tab: usize,
}

impl HasEditor for Calc {
    fn editor(&mut self) -> &mut Editor { &mut self.input }
    fn editor_ref(&self) -> &Editor { &self.input }
    fn on_edited(&mut self) { self.dirty = true }
}

impl Calc {
    fn new(path: Option<PathBuf>, cx: &mut Context<Self>) -> Calc {
        let mut c = Calc {
            focus: cx.focus_handle(),
            book: Book::new(),
            active: 0,
            cursor: Pos::new(0, 0),
            input: Editor::new(""),
            path: None,
            status: "".into(),
            notes: Vec::new(),
            dirty: false,
            tab: 0,
        };
        if let Some(p) = path {
            c.open(p);
        } else {
            let s = &mut c.book.sheets[0];
            s.set(Pos::new(0, 0), Cell::input("品名"));
            s.set(Pos::new(0, 1), Cell::input("数量"));
            s.set(Pos::new(0, 2), Cell::input("単価"));
            s.set(Pos::new(0, 3), Cell::input("金額"));
            s.set(Pos::new(1, 0), Cell::input("ザボガードF F-02"));
            s.set(Pos::new(1, 1), Cell::input("4"));
            s.set(Pos::new(1, 2), Cell::input("125000"));
            s.set(Pos::new(1, 3), Cell::input("=B2*C2"));
            s.set(Pos::new(2, 0), Cell::input("エンブM"));
            s.set(Pos::new(2, 1), Cell::input("2"));
            s.set(Pos::new(2, 2), Cell::input("98000"));
            s.set(Pos::new(2, 3), Cell::input("=B3*C3"));
            s.set(Pos::new(3, 2), Cell::input("小計"));
            s.set(Pos::new(3, 3), Cell::input("=SUM(D2:D3)"));
            s.set(Pos::new(4, 2), Cell::input("消費税"));
            s.set(Pos::new(4, 3), Cell::input("=ROUND(D4*0.1,0)"));
            s.set(Pos::new(5, 2), Cell::input("合計"));
            s.set(Pos::new(5, 3), Cell::input("=D4+D5"));
            recalc(s);
            c.status = "セルを選んで打つ。Enter で確定して下へ、Ctrl+S で保存".into();
        }
        c.sync_input();
        c
    }

    fn sheet(&self) -> &sheet::Sheet {
        &self.book.sheets[self.active]
    }
    fn sheet_mut(&mut self) -> &mut sheet::Sheet {
        let a = self.active;
        &mut self.book.sheets[a]
    }

    fn sync_input(&mut self) {
        let s = self.sheet().get(self.cursor).map(|c| c.editable()).unwrap_or_default();
        self.input = Editor::new(&s);
    }

    /// 数式バーの内容をセルに入れて再計算する。
    fn commit(&mut self) {
        let (cur, text) = (self.cursor, self.input.text().to_string());
        self.sheet_mut().set(cur, Cell::input(&text));
        let s = self.sheet_mut();
        recalc(s);
        self.dirty = true;
    }

    /// カーソルを動かす(動かす前に編集中の内容を確定する)。
    fn move_cursor(&mut self, dr: i32, dc: i32) {
        self.commit();
        let r = (self.cursor.row as i32 + dr).max(0) as u32;
        let c = (self.cursor.col as i32 + dc).max(0) as u32;
        self.cursor = Pos::new(r.min(ROWS - 1), c.min(COLS - 1));
        self.sync_input();
    }

    fn open(&mut self, p: PathBuf) {
        match std::fs::File::open(&p)
            .map_err(|e| e.to_string())
            .and_then(sheet::xlsx::read)
        {
            Ok((mut book, rep)) => {
                for s in &mut book.sheets {
                    recalc(s);
                }
                self.notes = rep
                    .unsupported
                    .iter()
                    .map(|(n, c)| SharedString::from(format!("{n} × {c}")))
                    .collect();
                self.status = format!(
                    "{} シート / {} セル — {}",
                    rep.sheets,
                    rep.cells,
                    p.file_name().unwrap_or_default().to_string_lossy()
                )
                .into();
                self.book = book;
                self.active = 0;
                self.cursor = Pos::new(0, 0);
                self.path = Some(p);
                self.sync_input();
            }
            Err(e) => self.status = format!("開けません: {e}").into(),
        }
    }

    // ---- 割り当てられた操作 ----
    fn a_backspace(&mut self, _: &ui::Backspace, _: &mut Window, cx: &mut Context<Self>) {
        self.input.backspace(); self.dirty = true; cx.notify();
    }
    fn a_delete(&mut self, _: &ui::Delete, _: &mut Window, cx: &mut Context<Self>) {
        self.input.delete(); self.dirty = true; cx.notify();
    }
    fn a_left(&mut self, _: &ui::Left, _: &mut Window, cx: &mut Context<Self>) {
        // 編集中の文字があればテキスト内を、無ければセルを移動する
        if self.input.text().is_empty() { self.move_cursor(0, -1) }
        else { self.input.move_char(false, false) }
        cx.notify();
    }
    fn a_right(&mut self, _: &ui::Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.input.text().is_empty() { self.move_cursor(0, 1) }
        else { self.input.move_char(true, false) }
        cx.notify();
    }
    fn a_up(&mut self, _: &ui::Up, _: &mut Window, cx: &mut Context<Self>) {
        self.move_cursor(-1, 0); cx.notify();
    }
    fn a_down(&mut self, _: &ui::Down, _: &mut Window, cx: &mut Context<Self>) {
        self.move_cursor(1, 0); cx.notify();
    }
    fn a_tab(&mut self, _: &ui::Tab, _: &mut Window, cx: &mut Context<Self>) {
        self.move_cursor(0, 1); cx.notify();
    }
    fn a_enter(&mut self, _: &ui::Enter, _: &mut Window, cx: &mut Context<Self>) {
        self.move_cursor(1, 0); cx.notify();
    }
    fn a_select_all(&mut self, _: &ui::SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.input.select_all(); cx.notify();
    }
    fn a_undo(&mut self, _: &ui::Undo, _: &mut Window, cx: &mut Context<Self>) {
        self.input.undo(); cx.notify();
    }
    fn a_save(&mut self, _: &ui::Save, _: &mut Window, cx: &mut Context<Self>) {
        self.commit(); self.save(); cx.notify();
    }
    fn a_open(&mut self, _: &ui::Open, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = rfd::FileDialog::new()
            .add_filter("Excelブック", &["xlsx"]).pick_file() { self.open(p) }
        cx.notify();
    }

    /// リボンのコマンド。数式タブは選択セルに関数を入れる。
    /// 選んでいるセルの見た目を変える。
    ///
    /// **値の無いセルにも掛ける** — 罫線だけを引くのは帳票では普通の操作。
    fn fmt(&mut self, f: impl Fn(&mut CellFormat)) {
        self.commit();
        let p = self.cursor;
        let mut c = self.sheet().get(p).cloned().unwrap_or_default();
        f(&mut c.fmt);
        self.book.sheets[self.active].set(p, c);
        self.dirty = true;
        recalc(&mut self.book.sheets[self.active]);
    }

    /// 行・列を出し入れする。
    fn rowcol(&mut self, f: impl Fn(&mut sheet::Sheet, Pos)) {
        self.commit();
        let p = self.cursor;
        f(&mut self.book.sheets[self.active], p);
        self.dirty = true;
        recalc(&mut self.book.sheets[self.active]);
    }

    /// 小数点以下の桁を増減する。
    ///
    /// **0〜10 に留める。** 際限なく増やせると、桁だけの帳票が出来上がる。
    fn decimals(&mut self, d: i32) {
        self.fmt(move |f| {
            let now = f
                .number_format
                .as_deref()
                .and_then(|s| s.rsplit_once('.'))
                .map(|(_, dec)| dec.chars().take_while(|c| *c == '0').count() as i32)
                .unwrap_or(0);
            let n = (now + d).clamp(0, 10);
            let comma = f.number_format.as_deref().is_some_and(|s| s.contains(','));
            let head = if comma { "#,##0" } else { "0" };
            f.number_format = Some(if n == 0 {
                head.to_string()
            } else {
                format!("{head}.{}", "0".repeat(n as usize))
            });
        });
    }

    fn run_cmd(&mut self, id: &str) {
        match id {
            "open" => {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("Excelブック", &["xlsx"]).pick_file() { self.open(p) }
            }
            "save" => { self.commit(); self.save() }
            "undo" => { self.input.undo(); }
            "selectall" => self.input.select_all(),
            // 罫線 — **日本の帳票の本体**
            "borders" => self.fmt(|f| {
                f.borders = if f.borders.any() { Borders::NONE } else { Borders::ALL }
            }),
            "bold" => self.fmt(|f| f.bold = !f.bold),
            "italic" => self.fmt(|f| f.italic = !f.italic),
            "underline" => self.fmt(|f| f.underline = !f.underline),
            "align-left" => self.fmt(|f| f.align = HAlign::Left),
            "align-center" => self.fmt(|f| f.align = HAlign::Center),
            "align-right" => self.fmt(|f| f.align = HAlign::Right),
            // 表示形式
            "comma" => self.fmt(|f| f.number_format = Some("#,##0".into())),
            // 行・列の出し入れ
            "cell-ins" => self.rowcol(|s, p| s.insert_row(p.row)),
            "cell-del" => self.rowcol(|s, p| s.remove_row(p.row)),
            "insrow" => self.rowcol(|s, p| s.insert_row(p.row)),
            "inscol" => self.rowcol(|s, p| s.insert_col(p.col)),
            // 小数点以下の桁
            "digit-inc" => self.decimals(1),
            "digit-dec" => self.decimals(-1),
            // 書式のクリア。値は消さない
            "clear" => self.fmt(|f| *f = CellFormat::default()),
            // 塗りつぶし。黄 → 水色 → 解除(色を選ぶ小窓がまだ無い)
            "fillparag" => self.fmt(|f| {
                f.fill = match f.fill.as_deref() {
                    None => Some("FFF2CC".into()),
                    Some("FFF2CC") => Some("DEEAF6".into()),
                    _ => None,
                }
            }),
            "fontcolor" => self.fmt(|f| {
                f.color = match f.color.as_deref() {
                    None => Some("C00000".into()),
                    Some("C00000") => Some("1F4E79".into()),
                    _ => None,
                }
            }),
            // 並べ替えは**見出しを据え置き、行はまるごと動かす**
            "custom-sort" => {
                self.commit();
                let c = self.cursor.col;
                self.book.sheets[self.active].sort_by_column(c, true, true);
                self.dirty = true;
                recalc(&mut self.book.sheets[self.active]);
                self.status = format!("{} 列で並べ替えました", Pos::new(0, c).a1()
                    .trim_end_matches('1')).into();
            }
            "rem-duplicates" => {
                self.commit();
                let n = self.book.sheets[self.active].remove_duplicate_rows(true);
                self.dirty = true;
                recalc(&mut self.book.sheets[self.active]);
                // 何件消したかを黙らない
                self.status = format!("重複した {n} 行を削除しました").into();
            }
            "currency" => self.fmt(|f| f.number_format = Some("¥#,##0".into())),
            "percents" => self.fmt(|f| f.number_format = Some("0%".into())),
            f @ ("sum" | "average" | "count" | "max" | "min") => {
                // 上の連続した数値をまとめる(表計算の当たり前の動き)
                let name = f.to_uppercase();
                let (r, c) = (self.cursor.row, self.cursor.col);
                let mut top = r;
                while top > 0 && self.sheet().get(Pos::new(top - 1, c))
                    .map(|x| matches!(x.value, Value::Number(_)) || x.formula.is_some())
                    .unwrap_or(false) { top -= 1 }
                let text = if top < r {
                    format!("={name}({}:{})", Pos::new(top, c).a1(), Pos::new(r - 1, c).a1())
                } else {
                    format!("={name}()")
                };
                self.input = Editor::new(&text);
                self.commit();
                self.sync_input();
            }
            _ => {}
        }
    }

    fn save(&mut self) {
        let p = match self.path.clone() {
            Some(p) => Some(p),
            None => rfd::FileDialog::new()
                .add_filter("Excelブック", &["xlsx"])
                .save_file(),
        };
        let Some(p) = p else { return };
        match std::fs::File::create(&p)
            .map_err(|e| e.to_string())
            .and_then(|f| sheet::xlsx::write(&self.book, std::io::BufWriter::new(f)))
        {
            Ok(_) => {
                self.status = format!(
                    "保存しました — {}",
                    p.file_name().unwrap_or_default().to_string_lossy()
                )
                .into();
                self.path = Some(p);
                self.dirty = false;
            }
            Err(e) => self.status = format!("保存できません: {e}").into(),
        }
    }
}

impl Focusable for Calc {
    fn focus_handle(&self, _cx: &App) -> FocusHandle { self.focus.clone() }
}

impl EntityInputHandler for Calc {
    fn text_for_range(&mut self, r: Range<usize>, actual: &mut Option<Range<usize>>,
                      _w: &mut Window, _cx: &mut Context<Self>) -> Option<String> {
        handler::text_for_range(self, r, actual)
    }
    fn selected_text_range(&mut self, _i: bool, _w: &mut Window, _cx: &mut Context<Self>)
        -> Option<UTF16Selection> {
        Some(UTF16Selection { range: handler::selected_range_utf16(self), reversed: false })
    }
    fn marked_text_range(&self, _w: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
        handler::marked_range_utf16(self)
    }
    fn unmark_text(&mut self, _w: &mut Window, _cx: &mut Context<Self>) { handler::unmark(self) }
    fn replace_text_in_range(&mut self, r: Option<Range<usize>>, text: &str,
                             _w: &mut Window, cx: &mut Context<Self>) {
        handler::replace(self, r, text);
        cx.notify();
    }
    fn replace_and_mark_text_in_range(&mut self, r: Option<Range<usize>>, text: &str,
                                      sel: Option<Range<usize>>, _w: &mut Window,
                                      cx: &mut Context<Self>) {
        handler::replace_and_mark(self, r, text, sel);
        cx.notify();
    }
    fn bounds_for_range(&mut self, _r: Range<usize>, bounds: Bounds<gpui::Pixels>,
                        _w: &mut Window, _cx: &mut Context<Self>)
        -> Option<Bounds<gpui::Pixels>> {
        // IME の候補窓は選択中のセルの下に出す
        Some(Bounds::new(
            gpui::point(
                bounds.origin.x + px(HEAD_W + self.cursor.col as f32 * COL_W),
                bounds.origin.y + px((self.cursor.row + 2) as f32 * ROW_H),
            ),
            size(px(COL_W), px(ROW_H)),
        ))
    }
    fn character_index_for_point(&mut self, _p: gpui::Point<gpui::Pixels>,
                                 _w: &mut Window, _cx: &mut Context<Self>) -> Option<usize> {
        None
    }
    fn text_length_utf16(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> Option<usize> {
        Some(handler::text_len_utf16(self))
    }
}

/// 入力ハンドラは paint のときに窓へ差す(GPUI の作法)。
struct InputSink { view: Entity<Calc> }
impl IntoElement for InputSink { type Element = Self; fn into_element(self) -> Self { self } }
impl gpui::Element for InputSink {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<gpui::ElementId> { None }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> { None }
    fn request_layout(&mut self, _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>, window: &mut Window, cx: &mut App)
        -> (gpui::LayoutId, ()) {
        let mut s = gpui::Style::default();
        s.size.width = gpui::relative(1.0).into();
        s.size.height = gpui::relative(1.0).into();
        (window.request_layout(s, [], cx), ())
    }
    fn prepaint(&mut self, _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>, _: Bounds<gpui::Pixels>,
        _: &mut (), _: &mut Window, _: &mut App) {}
    fn paint(&mut self, _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>, bounds: Bounds<gpui::Pixels>,
        _: &mut (), _: &mut (), window: &mut Window, cx: &mut App) {
        let focus = self.view.read(cx).focus.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.view.clone()), cx);
    }
}

fn col_name(c: u32) -> String {
    Pos::new(0, c).a1().trim_end_matches('1').to_string()
}

impl Render for Calc {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // ---- リボン(Euro-Office に名前と並びを合わせる) ----
        let (ready, all) = ribbon::progress(ribbon::CALC);
        let mut tabs = div().flex().flex_row().items_end().gap_1()
            .px_3().pt_1p5().bg(rgb(0x1B6E3C));
        for (i, tb) in ribbon::CALC.iter().enumerate() {
            let on = i == self.tab;
            tabs = tabs.child(div()
                .id(SharedString::from(format!("tab{i}")))
                .px_3().py_1p5().rounded_t_md()
                .bg(if on { rgb(0xFFFFFF) } else { rgb(0x1B6E3C) })
                .text_color(if on { rgb(0x1B6E3C) } else { rgb(0xCFE6D8) })
                .text_size(px(12.0))
                .font_weight(if on { gpui::FontWeight::BOLD } else { gpui::FontWeight::NORMAL })
                .cursor_pointer().hover(|s| s.text_color(rgb(0xFFFFFF)))
                .child(tb.name)
                .on_click(cx.listener(move |this, _, _, cx| { this.tab = i; cx.notify() })));
        }
        tabs = tabs.child(div().flex_1())
            .child(div().pb_1p5().pr_1().text_size(px(10.5)).text_color(rgb(0x9CC9AF))
                   .child(SharedString::from(format!("calc — 実装済み {ready}/{all}"))));

        let mut cmds = div().flex().flex_row().flex_wrap().gap_1().items_center()
            .px_3().py_2().bg(gpui::white())
            .border_b_1().border_color(rgb(0xE1E6EA));
        for cmd in ribbon::CALC[self.tab].cmds {
            if cmd.ready {
                let id = cmd.id;
                cmds = cmds.child(div().id(SharedString::from(cmd.id))
                    .px_3().py_1().rounded_md()
                    .border_1().border_color(rgb(0x1B6E3C)).text_color(rgb(0x1B6E3C))
                    .text_size(px(12.0)).cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF5EE)))
                    .child(cmd.label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.run_cmd(id); cx.notify()
                    })));
            } else {
                cmds = cmds.child(div().px_3().py_1().rounded_md()
                    .border_1().border_color(rgb(0xEDEFF1))
                    .text_color(rgb(0xB6BDC4)).text_size(px(12.0))
                    .child(cmd.label));
            }
        }
        cmds = cmds.child(div().flex_1())
            .child(div().text_size(px(11.0)).text_color(rgb(0x66707A))
                   .child(SharedString::from(format!("{}{}",
                       if self.dirty { "● " } else { "" }, self.status))));
        let bar = div().flex().flex_col().child(tabs).child(cmds);

        // ---- 数式バー ----
        let formula_bar = div()
            .flex().flex_row().items_center().gap_2()
            .px_4().py_1p5().bg(rgb(0xFAFBFC))
            .border_b_1().border_color(rgb(0xE1E6EA))
            .child(div().w(px(56.0)).text_size(px(12.0))
                   .font_weight(gpui::FontWeight::BOLD).text_color(rgb(0x1B6E3C))
                   .child(SharedString::from(self.cursor.a1())))
            .child(div().flex_1().px_2().py_1().bg(gpui::white())
                   .border_1().border_color(rgb(0xC6CDD3)).rounded_sm()
                   .text_size(px(13.0)).font_family("Noto Sans JP")
                   .child(SharedString::from(if self.input.text().is_empty() {
                       " ".to_string() } else { self.input.text().to_string() })));

        // ---- 格子 ----
        let mut grid = div().flex().flex_col();
        // 列見出し
        let mut head = div().flex().flex_row()
            .child(div().w(px(HEAD_W)).h(px(ROW_H)).bg(rgb(0xEFF2F4))
                   .border_r_1().border_b_1().border_color(rgb(0xD5DBE0)));
        for c in 0..COLS {
            head = head.child(div().w(px(COL_W)).h(px(ROW_H))
                .bg(rgb(0xEFF2F4)).border_r_1().border_b_1()
                .border_color(rgb(0xD5DBE0))
                .flex().items_center().justify_center()
                .text_size(px(11.5)).text_color(rgb(0x66707A))
                .child(SharedString::from(col_name(c))));
        }
        grid = grid.child(head);

        for r in 0..ROWS {
            let mut row = div().flex().flex_row()
                .child(div().w(px(HEAD_W)).h(px(ROW_H))
                    .bg(rgb(0xEFF2F4)).border_r_1().border_b_1()
                    .border_color(rgb(0xD5DBE0))
                    .flex().items_center().justify_center()
                    .text_size(px(11.5)).text_color(rgb(0x66707A))
                    .child(SharedString::from((r + 1).to_string())));
            for c in 0..COLS {
                let p = Pos::new(r, c);
                let cell = self.sheet().get(p);
                let v = cell.map(|x| x.value.clone()).unwrap_or(Value::Empty);
                // 付けた表示形式は画面に出す。出ないなら飾りでしかない
                let shown = sheet::model::format_value(&v, cell.and_then(|x| x.fmt.number_format.as_deref()));
                let is_num = matches!(v, Value::Number(_));
                let is_err = matches!(v, Value::Error(_));
                let sel = p == self.cursor;
                let mut d = div()
                    .id(SharedString::from(p.a1()))
                    .w(px(COL_W)).h(px(ROW_H))
                    .border_r_1().border_b_1().border_color(rgb(0xE1E6EA))
                    .bg(if sel { rgb(0xEAF5EE) } else { rgb(0xFFFFFF) })
                    .flex().items_center()
                    .px_1p5()
                    .text_size(px(12.5)).font_family("Noto Sans JP")
                    .overflow_hidden().whitespace_nowrap()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.commit();          // 移る前に、いま打っていた内容を入れる
                        this.cursor = p;
                        this.sync_input();
                        cx.notify();
                    }));
                // 罫線・塗り・文字書式。**帳票の見た目はここで決まる**
                let f = cell.map(|x| x.fmt.clone()).unwrap_or_default();
                if let Some(c) = &f.fill {
                    d = d.bg(hex(c));
                }
                if f.bold {
                    d = d.font_weight(gpui::FontWeight::BOLD);
                }
                if f.italic {
                    d = d.italic();
                }
                if let Some(c) = &f.color {
                    d = d.text_color(hex(c));
                }
                // 引いてある辺だけ濃くする(引いていない辺は表の薄い線のまま)
                let ink = rgb(0x1B1B1B);
                if f.borders.top { d = d.border_t_1().border_color(ink) }
                if f.borders.bottom { d = d.border_b_1().border_color(ink) }
                if f.borders.left { d = d.border_l_1().border_color(ink) }
                if f.borders.right { d = d.border_r_1().border_color(ink) }
                if sel {
                    d = d.border_2().border_color(rgb(0x1B6E3C));
                }
                // 揃えの指定があればそちらが勝つ(既定は数=右・文字=左)
                match f.align {
                    HAlign::Left => d = d.justify_start(),
                    HAlign::Center => d = d.justify_center(),
                    HAlign::Right => d = d.justify_end(),
                    HAlign::General => {}
                }
                if is_num && f.align == HAlign::General {
                    d = d.justify_end();
                }
                d = d.text_color(if is_err { rgb(0xB3261E) } else { rgb(0x1B1B1B) });
                // 選択中のセルは、確定前の入力をその場に見せる
                let shown = if sel { self.input.text().to_string() } else { shown };
                row = row.child(d.child(SharedString::from(shown)));
            }
            grid = grid.child(row);
        }

        let notes = if self.notes.is_empty() { None } else {
            let mut n = div().px_4().py_2().bg(rgb(0xFFF6E6))
                .border_t_1().border_color(rgb(0xE8D5A8))
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                       .text_color(rgb(0x8A4B00)).child("この版で読み飛ばしたもの"));
            for x in &self.notes {
                n = n.child(div().text_size(px(11.0)).text_color(rgb(0x8A4B00))
                            .child(x.clone()));
            }
            Some(n)
        };

        let me: Entity<Calc> = cx.entity();
        div().size_full().flex().flex_col().bg(rgb(0xF3F5F7))
            .key_context("jo_edit")
            .track_focus(&self.focus)
            .on_action(cx.listener(Calc::a_backspace))
            .on_action(cx.listener(Calc::a_delete))
            .on_action(cx.listener(Calc::a_left))
            .on_action(cx.listener(Calc::a_right))
            .on_action(cx.listener(Calc::a_up))
            .on_action(cx.listener(Calc::a_down))
            .on_action(cx.listener(Calc::a_tab))
            .on_action(cx.listener(Calc::a_enter))
            .on_action(cx.listener(Calc::a_select_all))
            .on_action(cx.listener(Calc::a_undo))
            .on_action(cx.listener(Calc::a_save))
            .on_action(cx.listener(Calc::a_open))
            .child(bar)
            .child(formula_bar)
            .child(div().flex_1().overflow_hidden().relative()
                   .child(grid)
                   .child(InputSink { view: me }))
            .children(notes)
    }
}

fn main() {
    let arg = std::env::args().nth(1).map(PathBuf::from);
    application().run(move |cx: &mut App| {
        cx.text_system()
            .add_fonts(vec![std::borrow::Cow::Borrowed(font_data())])
            .expect("フォント登録");
        cx.bind_keys(ui::bindings("jo_edit"));
        let bounds = Bounds::centered(None, size(px(1060.0), px(820.0)), cx);
        let arg2 = arg.clone();
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |window, cx| {
                let view = cx.new(|cx| Calc::new(arg2.clone(), cx));
                window.focus(&view.focus_handle(cx), cx);
                view
            },
        )
        .unwrap();
        cx.activate(true);
    });
}

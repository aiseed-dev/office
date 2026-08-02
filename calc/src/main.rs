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
/// 下地に選択の緑を混ぜる。**塗りを置き換えない** — 選択中も帳票本来の色が
/// 透けて見える(選択を解かないと色が確かめられない、を避ける)。
fn tint(base: gpui::Rgba, k: f32) -> gpui::Rgba {
    let accent = (0x1B as f32 / 255.0, 0x6E as f32 / 255.0, 0x3C as f32 / 255.0);
    gpui::Rgba {
        r: base.r * (1.0 - k) + accent.0 * k,
        g: base.g * (1.0 - k) + accent.1 * k,
        b: base.b * (1.0 - k) + accent.2 * k,
        a: 1.0,
    }
}

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
/// xlsx の列幅1(=「0」1個ぶん)を何画素にするか。既定幅 8.43 ≒ 108px の比
const PX_PER_CHW: f32 = 108.0 / 8.43;
/// 描く行の並び。固定行は常に頭に、残りは窓から。
fn grid_rows(frozen: Option<Pos>, view: Pos, n: u32) -> Vec<u32> {
    let f = frozen.map(|p| p.row).unwrap_or(0);
    let mut out: Vec<u32> = (0..f.min(n)).collect();
    let start = view.row.max(f);
    while (out.len() as u32) < n {
        let next = start + out.len() as u32 - f.min(n);
        out.push(next);
    }
    out
}

fn grid_cols(frozen: Option<Pos>, view: Pos, n: u32) -> Vec<u32> {
    let f = frozen.map(|p| p.col).unwrap_or(0);
    let mut out: Vec<u32> = (0..f.min(n)).collect();
    let start = view.col.max(f);
    while (out.len() as u32) < n {
        let next = start + out.len() as u32 - f.min(n);
        out.push(next);
    }
    out
}

const HEAD_W: f32 = 46.0;
const ROWS: u32 = 30;
const COLS: u32 = 9;

/// 使われていないシート名(Sheet2, Sheet3, …)。
fn unique_sheet_name(book: &Book) -> String {
    let mut n = book.sheets.len() + 1;
    loop {
        let name = format!("Sheet{n}");
        if !book.sheets.iter().any(|s| s.name == name) {
            return name;
        }
        n += 1;
    }
}

/// 選んだ範囲を TSV(タブ区切り・行は改行)にする。
/// 式は `=` のまま持つ — 表計算どうしの受け渡しの通り相場。
fn range_tsv(s: &sheet::Sheet, a: Pos, b: Pos) -> String {
    (a.row..=b.row)
        .map(|r| {
            (a.col..=b.col)
                .map(|c| s.get(Pos::new(r, c)).map(|x| x.editable()).unwrap_or_default())
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// TSV を格子に戻す。他のアプリから来るもの(\r\n・末尾の改行)も受ける。
fn tsv_grid(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_suffix('\n').unwrap_or(text);
    text.split('\n')
        .map(|line| {
            line.trim_end_matches('\r')
                .split('\t')
                .map(|s| s.to_string())
                .collect()
        })
        .collect()
}

/// 格子を `at` から流し込む。返すのは置いたセルの数。
///
/// **書式は据え置く**(帳票の枠を壊さない — 範囲の Delete と同じ規則)。
/// `shift` があれば式の相対参照をずらす(このアプリの中でのコピー。
/// 外から来た TSV はずらさない — どこから切り取られたか知りようがない)。
fn paste_grid(
    s: &mut sheet::Sheet,
    at: Pos,
    grid: &[Vec<String>],
    shift: Option<(i64, i64)>,
) -> usize {
    let mut n = 0usize;
    for (dr, row) in grid.iter().enumerate() {
        for (dc, text) in row.iter().enumerate() {
            let p = Pos::new(at.row + dr as u32, at.col + dc as u32);
            let fmt = s.get(p).map(|c| c.fmt.clone()).unwrap_or_default();
            let text = match (shift, text.starts_with('=')) {
                (Some((r, c)), true) => sheet::model::offset_refs(text, r, c),
                _ => text.clone(),
            };
            let mut cell = Cell::input(&text);
            cell.fmt = fmt;
            s.set(p, cell);
            n += 1;
        }
    }
    n
}

struct Calc {
    focus: FocusHandle,
    book: Book,
    active: usize,
    cursor: Pos,
    /// 範囲選択の起点(Shift+矢印/クリックで伸ばす)。無ければ1セル
    anchor: Option<Pos>,
    /// ドラッグ選択の始点(マウスの左を押した位置。離すと終わる)
    drag: Option<Pos>,
    /// ホイールの端数(触板の細かい送りを捨てずに貯める)
    wheel: (f32, f32),
    /// 右クリックのメニュー(出ている場所。格子領域の px)
    menu_at: Option<(f32, f32)>,
    /// 開いている子メニュー(挿入▸ など)
    menu_sub: Option<&'static str>,
    /// 「ドロップダウンリストから選択」の一覧(候補, 出す場所)
    pick: Option<(Vec<String>, (f32, f32))>,
    /// 書式の小窓(セルをフォーマットする)。範囲を選び直しながら使える
    fmt_panel: Option<(f32, f32)>,
    /// 小さな入力の板(種類, 入力欄)。"name"=名前の定義。開いている間は打鍵がここへ
    prompt: Option<(&'static str, Editor)>,
    /// 数式を値の代わりに出す(数式の表示)
    show_formulas: bool,
    /// 画面の窓の左上(スクロール)。**表は画面より大きい**
    view: Pos,
    /// 固定する行数・列数(見出しを置き去りにしないため)。カーソル位置で決める
    frozen: Option<Pos>,
    /// 絞り込み(列, 値)。**見え方だけ** — 保存される中身は変わらない
    filter: Option<(u32, String)>,
    /// 表の操作(書式・フィル・行列・結合・並べ替え)を戻すための控え。
    /// 入力欄の undo とは別 — **戻せない操作は事故のとき逃げ道が無い**。
    /// **どのシートの控えかを一緒に持つ** — シートを切り替えた後の undo が
    /// 別のシートへ他所の中身を書き戻す事故を防ぐ
    undo_stack: Vec<(usize, sheet::Sheet)>,
    redo_stack: Vec<(usize, sheet::Sheet)>,
    /// シートごとのカーソル・窓・固定(切り替えても場所を失わない)
    sheet_ui: Vec<(Pos, Pos, Option<Pos>)>,
    /// コピーの控え(起点, そのとき書いた TSV)。貼り付け時に系のクリップボードと
    /// 突き合わせ、一致すればアプリ内コピーとして式の参照をずらす
    clip: Option<(Pos, String)>,
    /// グリッド線(表の薄い線)を出す
    gridlines: bool,
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
    // 小さな入力の板(名前の定義など)が開いている間は、打鍵(IME含む)はそこへ
    fn editor(&mut self) -> &mut Editor {
        match &mut self.prompt {
            Some((_, ed)) => ed,
            None => &mut self.input,
        }
    }
    fn editor_ref(&self) -> &Editor {
        match &self.prompt {
            Some((_, ed)) => ed,
            None => &self.input,
        }
    }
    fn on_edited(&mut self) {
        // 板への打鍵は文書を変えない
        if self.prompt.is_none() {
            self.dirty = true;
        }
    }
}

impl Calc {
    fn new(path: Option<PathBuf>, cx: &mut Context<Self>) -> Calc {
        let mut c = Calc {
            focus: cx.focus_handle(),
            book: Book::new(),
            active: 0,
            cursor: Pos::new(0, 0),
            anchor: None,
            drag: None,
            wheel: (0.0, 0.0),
            menu_at: None,
            menu_sub: None,
            pick: None,
            fmt_panel: None,
            prompt: None,
            show_formulas: false,
            view: Pos::new(0, 0),
            frozen: None,
            filter: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            sheet_ui: Vec::new(),
            clip: None,
            gridlines: true,
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
    /// いまの表を控える(次の操作を戻せるように)。やり直しの控えは捨てる。
    fn checkpoint(&mut self) {
        self.undo_stack.push((self.active, self.book.sheets[self.active].clone()));
        if self.undo_stack.len() > 100 {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    /// 控えたシートを見せる(別のシートの操作を戻したなら、そこへ移る —
    /// 見えない場所で表が変わるのは事故のもと)。
    fn show_sheet(&mut self, idx: usize) {
        if idx != self.active && idx < self.book.sheets.len() {
            self.remember_ui();
            self.active = idx;
            self.restore_ui();
            self.anchor = None;
            self.filter = None;
        }
    }

    fn undo_sheet(&mut self) {
        let Some((idx, prev)) = self.undo_stack.pop() else {
            self.status = "戻すものがありません".into();
            return;
        };
        self.redo_stack.push((idx, self.book.sheets[idx].clone()));
        self.book.sheets[idx] = prev;
        recalc(&mut self.book.sheets[idx]);
        self.show_sheet(idx);
        self.dirty = true;
        self.sync_input();
        self.status = "戻しました".into();
    }

    fn redo_sheet(&mut self) {
        let Some((idx, next)) = self.redo_stack.pop() else {
            self.status = "やり直すものがありません".into();
            return;
        };
        self.undo_stack.push((idx, self.book.sheets[idx].clone()));
        self.book.sheets[idx] = next;
        recalc(&mut self.book.sheets[idx]);
        self.show_sheet(idx);
        self.dirty = true;
        self.sync_input();
        self.status = "やり直しました".into();
    }

    /// いまのシートのカーソル・窓・固定を控える。
    fn remember_ui(&mut self) {
        while self.sheet_ui.len() < self.book.sheets.len() {
            self.sheet_ui.push((Pos::new(0, 0), Pos::new(0, 0), None));
        }
        self.sheet_ui[self.active] = (self.cursor, self.view, self.frozen);
    }

    fn restore_ui(&mut self) {
        let (c, v, f) = self
            .sheet_ui
            .get(self.active)
            .copied()
            .unwrap_or((Pos::new(0, 0), Pos::new(0, 0), None));
        self.cursor = c;
        self.view = v;
        self.frozen = f;
    }

    /// 画面に出ている行の並び(絞り込み中はその行だけ)。描画と当たり判定で共有する。
    fn visible_rows(&self) -> Vec<u32> {
        match &self.filter {
            Some((col, v)) => {
                self.matching_rows(*col, v).into_iter().take(ROWS as usize).collect()
            }
            None => grid_rows(self.frozen, self.view, ROWS),
        }
    }

    /// 格子の中の位置(px、格子領域の左上原点)からセルを逆算する。
    /// 見出しの帯の上なら None。
    fn cell_at(&self, x: f32, y: f32) -> Option<Pos> {
        if x < HEAD_W || y < ROW_H {
            return None;
        }
        let mut x0 = HEAD_W;
        let mut col = None;
        for c in grid_cols(self.frozen, self.view, COLS) {
            let w = self.col_px(c);
            if x < x0 + w {
                col = Some(c);
                break;
            }
            x0 += w;
        }
        let mut y0 = ROW_H;
        let mut row = None;
        for r in self.visible_rows() {
            let h = self.row_px(r);
            if y < y0 + h {
                row = Some(r);
                break;
            }
            y0 += h;
        }
        Some(Pos { row: row?, col: col? })
    }

    /// マウスの左を押した(格子領域の座標)。押したセルが選択の始まり。
    /// メニューが出ていたら閉じる(項目の上の押下は stop_propagation でここに来ない)。
    fn mouse_down_at(&mut self, x: f32, y: f32, shift: bool, ctrl: bool) {
        self.menu_at = None;
        self.pick = None;
        let Some(p) = self.cell_at(x, y) else { return };
        // Ctrl+クリックはリンクを開く(基幹網の外は既定のブラウザに任せる)
        if ctrl && !shift {
            if let Some(url) = self.sheet().links.get(&p).cloned() {
                let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
                self.status = format!("開きます: {url}").into();
                return;
            }
        }
        self.commit();
        if shift {
            // いまのセルから伸ばす
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
        } else {
            self.anchor = None;
            self.drag = Some(p);
        }
        self.cursor = p;
        self.sync_input();
    }

    /// 押したまま動いた。通り過ぎたセルまで選択を広げる。
    fn mouse_drag_at(&mut self, x: f32, y: f32) {
        let Some(start) = self.drag else { return };
        let Some(p) = self.cell_at(x, y) else { return };
        if self.cursor == p {
            return;
        }
        self.cursor = p;
        self.anchor = if p == start { None } else { Some(start) };
        if self.anchor.is_some() {
            let (a, b) = self.sel_rect();
            self.status = format!("{}:{}", a.a1(), b.a1()).into();
        }
        self.sync_input();
    }

    /// 離した。ドラッグ選択はここで確定する。
    fn mouse_up(&mut self) {
        if self.drag.take().is_some() && self.anchor.is_some() {
            let (a, b) = self.sel_rect();
            self.status = format!("{}:{}", a.a1(), b.a1()).into();
        }
    }

    /// 右クリック。選択の中ならその選択への操作、外ならそのセルへ移ってから
    /// メニューを出す(Excel の作法)。
    fn right_click_at(&mut self, x: f32, y: f32) {
        if let Some(p) = self.cell_at(x, y) {
            let (a, b) = self.sel_rect();
            let inside = self.anchor.is_some()
                && (a.row..=b.row).contains(&p.row)
                && (a.col..=b.col).contains(&p.col);
            if !inside && p != self.cursor {
                self.commit();
                self.anchor = None;
                self.cursor = p;
                self.sync_input();
            }
        }
        self.menu_at = Some((x, y));
        self.menu_sub = None;
    }

    /// いま表示されているセルの左上(格子領域の px)。画面の外なら None。
    fn cell_origin_px(&self, p: Pos) -> Option<(f32, f32)> {
        let mut x = HEAD_W;
        let mut cfound = false;
        for c in grid_cols(self.frozen, self.view, COLS) {
            if c == p.col {
                cfound = true;
                break;
            }
            x += self.col_px(c);
        }
        let mut y = ROW_H;
        let mut rfound = false;
        for r in self.visible_rows() {
            if r == p.row {
                rfound = true;
                break;
            }
            y += self.row_px(r);
        }
        (cfound && rfound).then_some((x, y))
    }

    /// 一覧から選んだ値をセルに入れる(書式は据え置き)。
    fn pick_value(&mut self, v: &str) {
        self.checkpoint();
        let p = self.cursor;
        let fmt = self.sheet().get(p).map(|c| c.fmt.clone()).unwrap_or_default();
        let mut cell = Cell::input(v);
        cell.fmt = fmt;
        self.book.sheets[self.active].set(p, cell);
        recalc(&mut self.book.sheets[self.active]);
        self.dirty = true;
        self.sync_input();
        self.status = format!("{} に入れました", p.a1()).into();
    }

    /// メニューの項目を実行する。
    fn menu_action(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let menu_was_at = self.menu_at.take();
        self.menu_sub = None;
        match id {
            "cut" => self.a_cut(&ui::Cut, window, cx),
            "copy" => self.a_copy(&ui::Copy, window, cx),
            "paste" => self.a_paste(&ui::Paste, window, cx),
            // 消去。Euro-Office の「消去 ▸」に対応する3段
            "clear-all" => {
                self.commit();
                self.checkpoint();
                let (a, b) = self.sel_rect();
                let mut n = 0usize;
                for r in a.row..=b.row {
                    for c in a.col..=b.col {
                        n += self.book.sheets[self.active]
                            .cells
                            .remove(&Pos::new(r, c))
                            .is_some() as usize;
                    }
                }
                recalc(&mut self.book.sheets[self.active]);
                self.dirty = true;
                self.sync_input();
                self.status = format!("{n} セルを消去しました(中身も書式も)").into();
            }
            "clear-text" => {
                self.checkpoint();
                let n = self.clear_range();
                self.status = format!("{n} セルの中身を消しました(書式は残る)").into();
            }
            "clear-fmt" => self.run_cmd("clear"),
            "insrow" => {
                self.rowcol(|s, p| s.insert_row(p.row));
                self.status = "行を挿しました(下の式の参照も直っています)".into();
            }
            "delrow" => {
                self.rowcol(|s, p| s.remove_row(p.row));
                self.status = "行を削除しました".into();
            }
            "inscol" => {
                self.rowcol(|s, p| s.insert_col(p.col));
                self.status = "列を挿しました".into();
            }
            "delcol" => {
                self.rowcol(|s, p| s.remove_col(p.col));
                self.status = "列を削除しました".into();
            }
            "sort-asc" | "sort-desc" => {
                self.commit();
                self.checkpoint();
                let c = self.cursor.col;
                self.book.sheets[self.active].sort_by_column(c, id == "sort-asc", true);
                self.dirty = true;
                recalc(&mut self.book.sheets[self.active]);
                self.status = format!(
                    "{} 列で{}に並べ替えました",
                    Pos::new(0, c).a1().trim_end_matches('1'),
                    if id == "sort-asc" { "昇順" } else { "降順" }
                )
                .into();
            }
            "filter-set" => self.run_cmd("setfilter"),
            "filter-clear" => self.run_cmd("clear-filter"),
            "reapply" => {
                if let Some((c, v)) = self.filter.clone() {
                    let n = self.matching_rows(c, &v).len();
                    self.status = format!(
                        "{}列を「{v}」で絞り込み直しました({n}行が一致)",
                        Pos::new(0, c).a1().trim_end_matches('1')
                    )
                    .into();
                }
            }
            // セル単位のシフト(挿入・削除)。結合をまたぐときは断られる
            "inscell-right" | "inscell-down" | "delcell-left" | "delcell-up" => {
                self.commit();
                self.checkpoint();
                let (a, b) = self.sel_rect();
                let r = match id {
                    "inscell-right" => self.book.sheets[self.active].insert_cells(a, b, true),
                    "inscell-down" => self.book.sheets[self.active].insert_cells(a, b, false),
                    "delcell-left" => self.book.sheets[self.active].delete_cells(a, b, true),
                    _ => self.book.sheets[self.active].delete_cells(a, b, false),
                };
                match r {
                    Ok(n) => {
                        recalc(&mut self.book.sheets[self.active]);
                        self.dirty = true;
                        self.anchor = None;
                        self.sync_input();
                        self.status = format!(
                            "{n} セルをシフトしました(動いたセルへの参照も直っています)"
                        )
                        .into();
                    }
                    Err(e) => {
                        // 何も変えていないので、積んだ控えは戻す
                        self.undo_stack.pop();
                        self.status = e.into();
                    }
                }
            }
            "cond-neg" => {
                self.commit();
                self.checkpoint();
                let range = self.sel_rect();
                self.book.sheets[self.active].cond.push(sheet::model::CondRule {
                    range,
                    op: sheet::model::CondOp::Lt,
                    value: 0.0,
                    color: Some("C00000".into()),
                    fill: None,
                });
                self.dirty = true;
                self.status = format!(
                    "{}:{} — 0未満を赤字にしました",
                    range.0.a1(), range.1.a1()
                ).into();
            }
            "cond-gt" => {
                self.commit();
                self.prompt = Some(("cond-gt", Editor::new("")));
            }
            "cond-lt" => {
                self.commit();
                self.prompt = Some(("cond-lt", Editor::new("")));
            }
            "cond-clear" => {
                self.commit();
                self.checkpoint();
                let (a, b) = self.sel_rect();
                let before = self.book.sheets[self.active].cond.len();
                self.book.sheets[self.active].cond.retain(|r| {
                    let (ra, rb) = r.range;
                    // 選んだ範囲と重なる規則を消す
                    !(ra.row <= b.row && rb.row >= a.row && ra.col <= b.col && rb.col >= a.col)
                });
                let n = before - self.book.sheets[self.active].cond.len();
                self.dirty = true;
                self.status = format!("{n} 本の条件を消しました").into();
            }
            "picklist" => self.open_pick_list(),
            "defname" => {
                self.commit();
                self.prompt = Some(("name", Editor::new("")));
            }
            "addcomment" => {
                self.commit();
                let cur = self.sheet().comments.get(&self.cursor).cloned().unwrap_or_default();
                self.prompt = Some(("comment", Editor::new(&cur)));
            }
            "hyperlink" => {
                self.commit();
                let cur = self.sheet().links.get(&self.cursor).cloned().unwrap_or_default();
                self.prompt = Some(("link", Editor::new(&cur)));
            }
            "fmtcells" => {
                // メニューの出ていた場所の近くに小窓を開く
                self.fmt_panel = Some(menu_was_at.unwrap_or((HEAD_W + 24.0, ROW_H + 24.0)));
            }
            "freeze" => self.run_cmd("freeze"),
            // 数値の書式・関数はリボンと同じ配線を通す
            "comma" | "currency" | "percents" | "digit-inc" | "digit-dec"
            | "sum" | "average" | "count" | "max" | "min" => self.run_cmd(id),
            _ => {}
        }
        cx.notify();
    }

    /// 子メニューの中身 (id, 名前, 押せるか)。
    /// **並びと名前は Euro-Office に合わせ、未実装は灰色**(リボンと同じ方針)。
    fn menu_sub_entries(&self, sub: &str) -> Vec<(&'static str, &'static str, bool)> {
        match sub {
            "ins" => vec![
                ("inscell-right", "セルを右にシフト", true),
                ("inscell-down", "セルを下にシフト", true),
                ("insrow", "行全体", true),
                ("inscol", "列全体", true),
            ],
            "del" => vec![
                ("delcell-left", "セルを左にシフト", true),
                ("delcell-up", "セルを上にシフト", true),
                ("delrow", "行全体", true),
                ("delcol", "列全体", true),
            ],
            "clr" => vec![
                ("clear-all", "すべて", true),
                ("clear-text", "テキスト(書式は残す)", true),
                ("clear-fmt", "書式(中身は残す)", true),
            ],
            "sort" => vec![
                ("sort-asc", "昇順", true),
                ("sort-desc", "降順", true),
            ],
            "filter" => vec![
                ("filter-set", "選択した値で絞り込む", self.filter.is_none()),
                ("filter-clear", "絞り込みを解く", self.filter.is_some()),
            ],
            "cond" => vec![
                ("cond-neg", "0未満を赤字にする", true),
                ("cond-gt", "値より大きいと薄緑の塗り…", true),
                ("cond-lt", "値より小さいと薄赤の塗り…", true),
                ("cond-clear", "この範囲の条件を消す", true),
            ],
            "numfmt" => vec![
                ("comma", "桁区切り(1,000)", true),
                ("currency", "通貨(¥)", true),
                ("percents", "パーセント(%)", true),
                ("digit-inc", "小数を増やす", true),
                ("digit-dec", "小数を減らす", true),
            ],
            "func" => vec![
                ("sum", "SUM(合計)", true),
                ("average", "AVERAGE(平均)", true),
                ("count", "COUNT(個数)", true),
                ("max", "MAX(最大)", true),
                ("min", "MIN(最小)", true),
            ],
            _ => vec![],
        }
    }

    fn a_context_menu(&mut self, _: &ui::ContextMenu, _: &mut Window, cx: &mut Context<Self>) {
        // キーボードから: カーソルのセルのそば(セルが画面の外なら左上)に出す
        let (x, y) = self
            .cell_origin_px(self.cursor)
            .map(|(x, y)| (x + 16.0, y + 16.0))
            .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
        self.menu_at = Some((x, y));
        self.menu_sub = None;
        cx.notify();
    }

    fn a_cancel(&mut self, _: &ui::Cancel, _: &mut Window, cx: &mut Context<Self>) {
        // 入力の板 → 一覧 → 子メニュー → 親メニュー → 書式の小窓、の順で閉じる
        if self.prompt.take().is_some()
            || self.pick.take().is_some()
            || self.menu_sub.take().is_some()
            || self.menu_at.take().is_some()
            || self.fmt_panel.take().is_some()
        {
            cx.notify();
        }
    }

    /// 入力の板を確定する(Enter)。
    fn finish_prompt(&mut self) {
        let Some((kind, ed)) = self.prompt.take() else { return };
        let text = ed.text().trim().to_string();
        match kind {
            "name" => {
                if text.is_empty() {
                    self.status = "名前を付けませんでした".into();
                    return;
                }
                let ok = text.chars().all(|c| c.is_alphanumeric() || c == '_')
                    && !text.chars().next().unwrap().is_ascii_digit()
                    && Pos::parse(&text).is_none();
                if !ok {
                    self.status = format!(
                        "「{text}」は名前にできません(文字と数字と _。セル参照の形は不可)"
                    )
                    .into();
                    return;
                }
                let (a, b) = self.sel_rect();
                let range = if self.anchor.is_some() {
                    format!("{}:{}", a.a1(), b.a1())
                } else {
                    a.a1()
                };
                let s = &mut self.book.sheets[self.active];
                s.names.retain(|(n, _)| *n != text);
                s.names.push((text.clone(), range.clone()));
                recalc(&mut self.book.sheets[self.active]);
                self.dirty = true;
                self.status = format!("名前「{text}」= {range}(式の中で使えます)").into();
            }
            "comment" => {
                let p = self.cursor;
                if text.is_empty() {
                    if self.book.sheets[self.active].comments.remove(&p).is_some() {
                        self.dirty = true;
                        self.status = format!("{} のコメントを消しました", p.a1()).into();
                    }
                } else {
                    self.book.sheets[self.active].comments.insert(p, text);
                    self.dirty = true;
                    self.status = format!("{} にコメントを付けました(保存で残ります)", p.a1()).into();
                }
            }
            "cond-gt" | "cond-lt" => {
                let Ok(value) = text.parse::<f64>() else {
                    self.status = format!("「{text}」は数として読めません").into();
                    return;
                };
                self.checkpoint();
                let range = self.sel_rect();
                let gt = kind == "cond-gt";
                self.book.sheets[self.active].cond.push(sheet::model::CondRule {
                    range,
                    op: if gt { sheet::model::CondOp::Gt } else { sheet::model::CondOp::Lt },
                    value,
                    color: None,
                    fill: Some(if gt { "E2EFDA".into() } else { "FCE4D6".into() }),
                });
                self.dirty = true;
                self.status = format!(
                    "{}:{} — {value} より{}を塗ります",
                    range.0.a1(), range.1.a1(),
                    if gt { "大きい値" } else { "小さい値" }
                ).into();
            }
            "link" => {
                let p = self.cursor;
                if text.is_empty() {
                    if self.book.sheets[self.active].links.remove(&p).is_some() {
                        self.dirty = true;
                        self.status = format!("{} のリンクを外しました", p.a1()).into();
                    }
                } else {
                    self.book.sheets[self.active].links.insert(p, text);
                    self.dirty = true;
                    self.status =
                        format!("{} にリンクを付けました(Ctrl+クリックで開く)", p.a1()).into();
                }
            }
            _ => {}
        }
    }

    /// 選んだ範囲の**外周だけ**に罫線(帳票の枠)。
    fn border_outline(&mut self) {
        self.commit();
        self.checkpoint();
        let (a, b) = self.sel_rect();
        for r in a.row..=b.row {
            for c in a.col..=b.col {
                let p = Pos::new(r, c);
                let mut cell = self.sheet().get(p).cloned().unwrap_or_default();
                if r == a.row { cell.fmt.borders.top = true }
                if r == b.row { cell.fmt.borders.bottom = true }
                if c == a.col { cell.fmt.borders.left = true }
                if c == b.col { cell.fmt.borders.right = true }
                self.book.sheets[self.active].set(p, cell);
            }
        }
        self.dirty = true;
        self.status = "外枠を引きました".into();
    }

    /// 書式の小窓の釦。
    fn fmt_panel_action(&mut self, id: &str) {
        match id {
            "close" => self.fmt_panel = None,
            "b-all" => {
                self.fmt(|f| f.borders = Borders::ALL);
                self.status = "格子の罫線を引きました".into();
            }
            "b-out" => self.border_outline(),
            "b-none" => {
                self.fmt(|f| f.borders = Borders::NONE);
                self.status = "罫線を消しました".into();
            }
            "numfmt-none" => {
                self.fmt(|f| f.number_format = None);
                self.status = "表示形式を戻しました".into();
            }
            id if id.starts_with("fill-") => {
                let v = id.trim_start_matches("fill-").to_string();
                if v == "none" {
                    self.fmt(|f| f.fill = None);
                } else {
                    self.fmt(move |f| f.fill = Some(v.clone()));
                }
            }
            id if id.starts_with("color-") => {
                let v = id.trim_start_matches("color-").to_string();
                if v == "none" {
                    self.fmt(|f| f.color = None);
                } else {
                    self.fmt(move |f| f.color = Some(v.clone()));
                }
            }
            other => self.run_cmd(other),
        }
    }

    /// 「ドロップダウンリストから選択」。同じ列に既にある値の一覧を出す
    /// (Excel の Alt+↓ と同じ発想。入力規則が無くても、列の値は候補になる)。
    fn open_pick_list(&mut self) {
        let col = self.cursor.col;
        let (rows, _) = self.sheet().extent();
        let mut vals: Vec<String> = Vec::new();
        for r in 0..rows {
            if r == self.cursor.row {
                continue;
            }
            if let Some(c) = self.sheet().get(Pos::new(r, col)) {
                // 式の結果ではなく「打つもの」を候補にする(文字の値だけ)
                if c.formula.is_none() {
                    let v = c.value.display();
                    if !v.is_empty() && !vals.contains(&v) {
                        vals.push(v);
                    }
                }
            }
        }
        if vals.is_empty() {
            self.status = "この列にはまだ値がありません".into();
            return;
        }
        vals.sort();
        vals.truncate(12);
        let at = self
            .cell_origin_px(self.cursor)
            .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
            .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
        self.pick = Some((vals, at));
    }

    /// シートを切り替える。いまの編集を確定し、場所はシートごとに覚えている。
    /// 絞り込みは解く(別のシートの列で絞ったままは意味を持たない)。
    fn switch_sheet(&mut self, i: usize) {
        if i >= self.book.sheets.len() || i == self.active {
            return;
        }
        self.commit();
        self.remember_ui();
        self.active = i;
        self.restore_ui();
        self.anchor = None;
        self.filter = None;
        self.sync_input();
        self.status = format!("シート「{}」", self.sheet().name).into();
    }

    /// シートを1枚足して、そこへ移る。
    fn add_sheet(&mut self) {
        let name = unique_sheet_name(&self.book);
        self.book.sheets.push(sheet::Sheet::new(&name));
        self.dirty = true;
        self.switch_sheet(self.book.sheets.len() - 1);
    }

    fn commit(&mut self) {
        let (cur, text) = (self.cursor, self.input.text().to_string());
        // 変わっていなければ何もしない(移動のたびに履歴が積まれるのを防ぐ)
        let now = self.sheet().get(cur).map(|c| c.editable()).unwrap_or_default();
        if now == text {
            return;
        }
        self.checkpoint();
        self.sheet_mut().set(cur, Cell::input(&text));
        let s = self.sheet_mut();
        recalc(s);
        self.dirty = true;
    }

    /// カーソルを動かす(動かす前に編集中の内容を確定する)。
    /// いま選んでいる長方形(左上, 右下)。
    /// 行の画面高。文書の指定(xlsx の ht、pt)に従う。既定 15pt = 24px
    fn row_px(&self, r: u32) -> f32 {
        self.sheet().row_height.get(&r).map(|pt| pt * 24.0 / 15.0).unwrap_or(ROW_H)
    }

    /// 列の画面幅。文書の指定(xlsx の width)に従う
    fn col_px(&self, c: u32) -> f32 {
        self.sheet()
            .col_width
            .get(&c)
            .copied()
            .or(self.sheet().default_col_width)
            .map(|w| w * PX_PER_CHW)
            .unwrap_or(COL_W)
    }

    /// 列の左端(見出しの右から)
    fn col_x(&self, c: u32) -> f32 {
        (0..c).map(|i| self.col_px(i)).sum()
    }

    fn sel_rect(&self) -> (Pos, Pos) {
        let a = self.anchor.unwrap_or(self.cursor);
        let c = self.cursor;
        (Pos::new(a.row.min(c.row), a.col.min(c.col)),
         Pos::new(a.row.max(c.row), a.col.max(c.col)))
    }

    /// Shift+矢印。起点を置いてから動く
    fn extend(&mut self, dr: i32, dc: i32) {
        if self.anchor.is_none() {
            self.anchor = Some(self.cursor);
        }
        self.commit();
        let r = (self.cursor.row as i32 + dr).max(0) as u32;
        let c = (self.cursor.col as i32 + dc).max(0) as u32;
        self.cursor = Pos::new(r.min(9999), c.min(255));
        self.follow();
        let (a, b) = self.sel_rect();
        self.status = format!("{}:{}", a.a1(), b.a1()).into();
        self.sync_input();
    }

    /// カーソルが見える位置まで窓を動かす。
    fn follow(&mut self) {
        if self.cursor.row < self.view.row {
            self.view.row = self.cursor.row;
        }
        if self.cursor.row >= self.view.row + ROWS {
            self.view.row = self.cursor.row + 1 - ROWS;
        }
        if self.cursor.col < self.view.col {
            self.view.col = self.cursor.col;
        }
        if self.cursor.col >= self.view.col + COLS {
            self.view.col = self.cursor.col + 1 - COLS;
        }
    }

    fn move_cursor(&mut self, dr: i32, dc: i32) {
        // 普通の移動は選択を解く
        self.anchor = None;
        self.commit();
        let r = (self.cursor.row as i32 + dr).max(0) as u32;
        let c = (self.cursor.col as i32 + dc).max(0) as u32;
        self.cursor = Pos::new(r.min(9999), c.min(255));
        self.follow();
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
                self.view = Pos::new(0, 0);
                self.anchor = None;
                self.frozen = None;
                self.filter = None;
                self.sheet_ui.clear();
                self.undo_stack.clear();
                self.redo_stack.clear();
                self.path = Some(p);
                self.sync_input();
            }
            Err(e) => self.status = format!("開けません: {e}").into(),
        }
    }

    // ---- 割り当てられた操作 ----
    fn a_backspace(&mut self, _: &ui::Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((_, ed)) = &mut self.prompt {
            ed.backspace();
        } else {
            self.input.backspace();
            self.dirty = true;
        }
        cx.notify();
    }
    /// 選んだ範囲の中身を消す(**書式は残す** — 帳票の枠を壊さない)。
    /// 控えを取ってから呼ぶこと。返すのは消したセルの数。
    fn clear_range(&mut self) -> usize {
        let (a, b) = self.sel_rect();
        let mut n = 0usize;
        for r in a.row..=b.row {
            for c in a.col..=b.col {
                let p = Pos::new(r, c);
                if let Some(cell) = self.sheet().get(p).cloned() {
                    self.book.sheets[self.active].set(p, Cell {
                        formula: None,
                        value: Value::Empty,
                        fmt: cell.fmt,
                    });
                    n += 1;
                }
            }
        }
        recalc(&mut self.book.sheets[self.active]);
        self.dirty = true;
        self.sync_input();
        n
    }

    fn a_delete(&mut self, _: &ui::Delete, _: &mut Window, cx: &mut Context<Self>) {
        if self.anchor.is_some() {
            // 範囲を選んでいるときの Delete は、その中身を消す(戻せる)
            self.checkpoint();
            let n = self.clear_range();
            self.status = format!("{n} セルの中身を消しました(書式は残る)").into();
        } else {
            self.input.delete();
            self.dirty = true;
        }
        cx.notify();
    }

    /// コピー。選んだ範囲(無ければいまのセル)を TSV で系のクリップボードへ。
    /// 他のアプリにはそのまま貼れる形で、アプリ内には起点を控えて式をずらせる形で。
    fn a_copy(&mut self, _: &ui::Copy, _: &mut Window, cx: &mut Context<Self>) {
        if self.input.has_selection() {
            // 数式バーの文字を選んでいるなら、その文字のコピー
            let sel = self.input.selection();
            if let Some(s) = self.input.text().get(sel) {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(s.to_string()));
                self.status = "コピーしました".into();
            }
            cx.notify();
            return;
        }
        let (a, b) = self.sel_rect();
        let tsv = range_tsv(self.sheet(), a, b);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(tsv.clone()));
        self.clip = Some((a, tsv));
        self.status = format!("{}:{} をコピーしました", a.a1(), b.a1()).into();
        cx.notify();
    }

    /// 切り取り = コピー + 中身を消す(書式は残る。1手で戻せる)。
    fn a_cut(&mut self, _: &ui::Cut, _: &mut Window, cx: &mut Context<Self>) {
        if self.input.has_selection() {
            let sel = self.input.selection();
            if let Some(s) = self.input.text().get(sel) {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(s.to_string()));
                self.input.insert("");
                self.dirty = true;
                self.status = "切り取りました".into();
            }
            cx.notify();
            return;
        }
        let (a, b) = self.sel_rect();
        let tsv = range_tsv(self.sheet(), a, b);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(tsv.clone()));
        // 切り取りの貼り付け先で式をずらさない(移動なので参照はそのまま)
        self.clip = None;
        self.checkpoint();
        let n = self.clear_range();
        self.status = format!("{n} セルを切り取りました").into();
        cx.notify();
    }

    /// 貼り付け。編集中なら文字として、そうでなければセルの格子として。
    fn a_paste(&mut self, _: &ui::Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) else {
            self.status = "貼り付けるものがありません".into();
            cx.notify();
            return;
        };
        if text.is_empty() {
            cx.notify();
            return;
        }
        if self.editing() {
            // 打ちかけの間は文字の貼り付け(書きかけの式に継ぎ足す使い方)
            self.input.insert(&text);
            self.dirty = true;
            cx.notify();
            return;
        }
        // アプリ内のコピーなら、式の相対参照を貼り付け先へずらす
        let shift = match &self.clip {
            Some((org, tsv)) if *tsv == text => Some((
                self.cursor.row as i64 - org.row as i64,
                self.cursor.col as i64 - org.col as i64,
            )),
            _ => None,
        };
        let grid = tsv_grid(&text);
        self.checkpoint();
        let at = self.cursor;
        let n = paste_grid(&mut self.book.sheets[self.active], at, &grid, shift);
        recalc(&mut self.book.sheets[self.active]);
        self.dirty = true;
        self.sync_input();
        self.status = format!("{n} セルを貼り付けました(書式は据え置き)").into();
        cx.notify();
    }
    /// 数式バーを打ちかけか(バーの中身がセルの保存内容から変わっているか)。
    /// バーには選んだセルの中身が常に写っているので、**空かどうかでは分からない**
    /// — 中身のあるセルで矢印が「見えない文字カーソル」に化け、
    /// セルから出られなくなる(踏んで直した)。
    fn editing(&self) -> bool {
        let saved = self.sheet().get(self.cursor).map(|c| c.editable()).unwrap_or_default();
        self.input.text() != saved
    }

    fn a_left(&mut self, _: &ui::Left, _: &mut Window, cx: &mut Context<Self>) {
        // 板 → 打ちかけの文字 → セル、の順で見る
        if let Some((_, ed)) = &mut self.prompt { ed.move_char(false, false) }
        else if self.editing() { self.input.move_char(false, false) }
        else { self.move_cursor(0, -1) }
        cx.notify();
    }
    fn a_right(&mut self, _: &ui::Right, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((_, ed)) = &mut self.prompt { ed.move_char(true, false) }
        else if self.editing() { self.input.move_char(true, false) }
        else { self.move_cursor(0, 1) }
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
        if self.prompt.is_some() {
            self.finish_prompt();
        } else {
            self.move_cursor(1, 0);
        }
        cx.notify();
    }
    fn a_select_left(&mut self, _: &ui::SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.editing() { self.input.move_char(false, true) }
        else { self.extend(0, -1) }
        cx.notify();
    }
    fn a_select_right(&mut self, _: &ui::SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        if self.editing() { self.input.move_char(true, true) }
        else { self.extend(0, 1) }
        cx.notify();
    }
    fn a_select_up(&mut self, _: &ui::SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.extend(-1, 0); cx.notify();
    }
    fn a_select_down(&mut self, _: &ui::SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.extend(1, 0); cx.notify();
    }
    fn a_select_all(&mut self, _: &ui::SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        if self.editing() {
            // 打ちかけの間は、バーの文字の全選択
            self.input.select_all();
        } else {
            // 使われている範囲の全選択(表計算の Ctrl+A)
            let (rows, cols) = self.sheet().extent();
            if rows == 0 {
                self.status = "空の表です".into();
            } else {
                self.commit();
                self.anchor = Some(Pos::new(0, 0));
                self.cursor = Pos::new(rows - 1, cols.saturating_sub(1));
                self.status = format!("A1:{} を選択しました", self.cursor.a1()).into();
                self.sync_input();
            }
        }
        cx.notify();
    }
    fn a_undo(&mut self, _: &ui::Undo, _: &mut Window, cx: &mut Context<Self>) {
        if !self.input.undo() {
            self.undo_sheet();
        }
        cx.notify();
    }
    fn a_redo(&mut self, _: &ui::Redo, _: &mut Window, cx: &mut Context<Self>) {
        if !self.input.redo() {
            self.redo_sheet();
        }
        cx.notify();
    }
    fn a_save(&mut self, _: &ui::Save, _: &mut Window, cx: &mut Context<Self>) {
        self.commit(); self.save(); cx.notify();
    }
    fn a_open(&mut self, _: &ui::Open, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = rfd::FileDialog::new()
            .add_filter("Excelブック", &["xlsx"]).pick_file() { self.open(p) }
        cx.notify();
    }

    /// 終了してよいか。書きかけがあれば保存するかを確かめる。
    /// 「はい」でも保存できなかった(ダイアログを閉じた等)なら終了しない —
    /// 書きかけを黙って捨てない。
    fn may_close(&mut self) -> bool {
        self.commit();
        if !self.dirty {
            return true;
        }
        match rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("calc")
            .set_description("保存していない変更があります。保存して終了しますか?")
            .set_buttons(rfd::MessageButtons::YesNoCancel)
            .show()
        {
            rfd::MessageDialogResult::Yes => {
                self.save();
                !self.dirty
            }
            rfd::MessageDialogResult::No => true,
            _ => false,
        }
    }

    fn a_quit(&mut self, _: &ui::Quit, _: &mut Window, cx: &mut Context<Self>) {
        if self.may_close() {
            cx.quit();
        }
    }

    /// リボンのコマンド。数式タブは選択セルに関数を入れる。
    /// 選んでいるセルの見た目を変える。
    ///
    /// **値の無いセルにも掛ける** — 罫線だけを引くのは帳票では普通の操作。
    fn fmt(&mut self, f: impl Fn(&mut CellFormat)) {
        self.commit();
        self.checkpoint();
        // 範囲選択があれば全部に掛ける。罫線も塗りも、帳票は範囲でやる仕事
        let (a, b) = self.sel_rect();
        for r in a.row..=b.row {
            for cidx in a.col..=b.col {
                let p = Pos::new(r, cidx);
                let mut c = self.sheet().get(p).cloned().unwrap_or_default();
                f(&mut c.fmt);
                self.book.sheets[self.active].set(p, c);
            }
        }
        self.dirty = true;
        recalc(&mut self.book.sheets[self.active]);
    }

    /// 選んだ範囲を結合する。**値は消さない** — 左上以外の値は隠れるだけで、
    /// 結合を解けば戻る(黙って捨てない)。
    fn merge_selection(&mut self) {
        self.checkpoint();
        let (a, b) = self.sel_rect();
        if a == b {
            self.status = "結合する範囲を Shift+矢印で選んでください".into();
            return;
        }
        let sh = &mut self.book.sheets[self.active];
        // 同じ範囲がもう結合されていたら解く(押すたびに入切)
        if let Some(i) = sh.merges.iter().position(|m| *m == (a, b)) {
            sh.merges.remove(i);
            self.status = format!("{}:{} の結合を解きました", a.a1(), b.a1()).into();
        } else {
            sh.merges.retain(|(x, y)| {
                // 重なる結合は先に外す(入れ子の結合は帳票を壊す)
                y.row < a.row || x.row > b.row || y.col < a.col || x.col > b.col
            });
            sh.merges.push((a, b));
            self.status = format!("{}:{} を結合しました", a.a1(), b.a1()).into();
        }
        self.dirty = true;
    }

    /// 行・列を出し入れする。
    fn rowcol(&mut self, f: impl Fn(&mut sheet::Sheet, Pos)) {
        self.commit();
        self.checkpoint();
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

    fn save_pdf(&mut self) {
        self.commit();
        let Some(p) = rfd::FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .set_file_name("帳票.pdf")
            .save_file()
        else {
            return;
        };
        let (fam, exact) = match kumihan::font::for_document(None) {
            Ok(x) => x,
            Err(e) => {
                self.status = format!("PDF にできません: {e}").into();
                return;
            }
        };
        let data = match kumihan::font::load(fam) {
            Ok(d) => d,
            Err(e) => {
                self.status = format!("PDF にできません: {e}").into();
                return;
            }
        };
        let r = kumihan::atomic::save(&p, |f| {
            paper::grid::sheet_to_pdf(
                &self.book.sheets[self.active],
                &data,
                paper::Paper::default(),
                std::io::BufWriter::new(f),
            )
        });
        self.status = match r {
            // 塗りの色と列幅はまだ紙に出ない。黙って出したことにしない
            Ok(_) => format!(
                "PDF にしました(塗りと列幅は未対応)— {}{}",
                p.file_name().unwrap_or_default().to_string_lossy(),
                if exact { "" } else { " ※代替フォント" }
            )
            .into(),
            Err(e) => format!("PDF にできません: {e}").into(),
        };
    }

    /// 絞り込みに一致する行(見出し行 0 は常に入れる)。
    fn matching_rows(&self, col: u32, v: &str) -> Vec<u32> {
        let (rows, _) = self.sheet().extent();
        let mut out = vec![0];
        for r in 1..rows {
            if self.sheet().get(Pos::new(r, col)).map(|c| c.value.display()).as_deref() == Some(v) {
                out.push(r);
            }
        }
        out
    }

    /// run_cmd が処理できる id。**リボンの ready はこの表の中に限る**
    /// (試験で突き合わせる。合っていない釦は「押せるのに何もしない」嘘になる)
    const HANDLED: &'static [&'static str] = &[
        "open", "save", "undo", "redo", "selectall", "pdf",
        "bold", "italic", "underline", "borders", "fillparag", "fontcolor",
        "align-left", "align-center", "align-right",
        "comma", "currency", "percents", "digit-inc", "digit-dec", "clear",
        "strikeout", "top", "middle", "bottom", "wrap", "incfont", "decfont",
        "cell-ins", "cell-del", "insrow", "inscol",
        "merge", "custom-sort", "rem-duplicates", "setfilter", "clear-filter",
        "fill-num", "freeze", "show-formulas", "show-gridlines",
        "fn-math", "fn-text", "fn-logical", "fn-recent",
        "sum", "average", "count", "max", "min",
    ];

    fn run_cmd(&mut self, id: &str) {
        match id {
            "open" => {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("Excelブック", &["xlsx"]).pick_file() { self.open(p) }
            }
            "save" => { self.commit(); self.save() }
            "undo" => {
                if !self.input.undo() {
                    self.undo_sheet();
                }
            }
            "redo" => {
                if !self.input.redo() {
                    self.redo_sheet();
                }
            }
            "selectall" => self.input.select_all(),
            // 罫線 — **日本の帳票の本体**
            "borders" => self.fmt(|f| {
                f.borders = if f.borders.any() { Borders::NONE } else { Borders::ALL }
            }),
            "bold" => self.fmt(|f| f.bold = !f.bold),
            "italic" => self.fmt(|f| f.italic = !f.italic),
            "underline" => self.fmt(|f| f.underline = !f.underline),
            "strikeout" => self.fmt(|f| f.strike = !f.strike),
            // 縦の揃えと折り返し
            "top" => self.fmt(|f| f.valign = sheet::model::VAlign::Top),
            "middle" => self.fmt(|f| f.valign = sheet::model::VAlign::Middle),
            "bottom" => self.fmt(|f| f.valign = sheet::model::VAlign::Bottom),
            "wrap" => self.fmt(|f| f.wrap = !f.wrap),
            // 文字の大きさ(4〜72pt)
            "incfont" => self.fmt(|f| {
                let pt = f.size_c.map(|c| c as f32 / 100.0).unwrap_or(11.0);
                f.size_c = Some((((pt + 1.0).min(72.0)) * 100.0) as u32);
            }),
            "decfont" => self.fmt(|f| {
                let pt = f.size_c.map(|c| c as f32 / 100.0).unwrap_or(11.0);
                f.size_c = Some((((pt - 1.0).max(4.0)) * 100.0) as u32);
            }),
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
            // フィル(下方向へコピー)。式は相対参照がずれ、$ は止まる。
            // 書式も一緒に写す(帳票の列は書式ごと揃える)
            "fill-num" => {
                let (a, b) = self.sel_rect();
                if a.row == b.row {
                    self.status = "Shift+↓ で埋める範囲を選んでください(先頭行を下へ写します)".into();
                } else {
                    self.commit();
                    self.checkpoint();
                    let sh = &mut self.book.sheets[self.active];
                    let mut n = 0usize;
                    for c in a.col..=b.col {
                        let Some(src) = sh.get(Pos::new(a.row, c)).cloned() else { continue };
                        for r in a.row + 1..=b.row {
                            let mut cell = src.clone();
                            if let Some(f) = &src.formula {
                                cell.formula =
                                    Some(sheet::model::offset_refs(f, (r - a.row) as i64, 0));
                            }
                            sh.set(Pos::new(r, c), cell);
                            n += 1;
                        }
                    }
                    recalc(&mut self.book.sheets[self.active]);
                    self.dirty = true;
                    self.status = format!("{n} セルを埋めました").into();
                }
            }
            // 塗りつぶし。黄 → 水色 → 解除(色を選ぶ小窓がまだ無い)
            "merge" => self.merge_selection(),
            // 表示。**値は変えない** — 見え方だけの話
            "show-formulas" => self.show_formulas = !self.show_formulas,
            // 帳票を PDF に。画面に見えているもの(値・書式・罫線)を写す
            "pdf" => self.save_pdf(),
            "show-gridlines" => self.gridlines = !self.gridlines,
            // ウィンドウ枠の固定。カーソルの上と左を留める。もう一度で解く
            // 選んだセルの値で絞る。もう一度で解く。**中身は変えない**
            "setfilter" => {
                let p = self.cursor;
                let v = self.sheet().get(p)
                    .map(|c| c.value.display())
                    .unwrap_or_default();
                if v.is_empty() {
                    self.status = "空のセルでは絞れません".into();
                } else {
                    let n = self.matching_rows(p.col, &v).len();
                    self.status = format!(
                        "{}列を「{v}」で絞り込み中({n}行が一致)。表示だけで中身は変わりません",
                        Pos::new(0, p.col).a1().trim_end_matches('1')
                    ).into();
                    self.filter = Some((p.col, v));
                }
            }
            "clear-filter" => {
                self.filter = None;
                self.status = "絞り込みを解きました".into();
            }
            "freeze" => {
                self.frozen = match self.frozen {
                    Some(_) => None,
                    None if self.cursor.row == 0 && self.cursor.col == 0 => {
                        self.status = "固定する位置にカーソルを置いてください(その上と左が留まります)".into();
                        None
                    }
                    None => {
                        self.status = format!(
                            "{}行 {}列を固定しました",
                            self.cursor.row, self.cursor.col
                        ).into();
                        Some(self.cursor)
                    }
                };
            }
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
                self.checkpoint();
                let c = self.cursor.col;
                self.book.sheets[self.active].sort_by_column(c, true, true);
                self.dirty = true;
                recalc(&mut self.book.sheets[self.active]);
                self.status = format!("{} 列で並べ替えました", Pos::new(0, c).a1()
                    .trim_end_matches('1')).into();
            }
            "rem-duplicates" => {
                self.commit();
                self.checkpoint();
                let n = self.book.sheets[self.active].remove_duplicate_rows(true);
                self.dirty = true;
                recalc(&mut self.book.sheets[self.active]);
                // 何件消したかを黙らない
                self.status = format!("重複した {n} 行を削除しました").into();
            }
            "currency" => self.fmt(|f| f.number_format = Some("¥#,##0".into())),
            "percents" => self.fmt(|f| f.number_format = Some("0%".into())),
            // 関数の一覧。**使える名前だけを出す** — 無いものを並べない
            f @ ("fn-math" | "fn-text" | "fn-logical" | "fn-recent") => {
                let names: &str = match f {
                    "fn-math" => "SUM AVERAGE ROUND ROUNDUP ROUNDDOWN INT ABS MOD POWER SQRT PRODUCT",
                    "fn-text" => "LEN LEFT RIGHT MID TRIM UPPER LOWER CONCATENATE",
                    "fn-logical" => "IF AND OR NOT TRUE FALSE ISBLANK ISERROR",
                    _ => "SUM AVERAGE COUNT MAX MIN IF SUMIF COUNTIF",
                };
                self.status = format!("使える関数: {names}").into();
            }
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
            other => {
                // ここに来たら結線漏れ。黙らず画面に出す
                self.status = format!("未配線のコマンド: {other}(不具合です)").into();
            }
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
        // 原本の部品(図形・テーマ・印刷設定)を持ち越す。読み終えてから書く
        let original: Option<std::io::Cursor<Vec<u8>>> = self
            .path
            .as_ref()
            .and_then(|old| std::fs::read(old).ok())
            .map(std::io::Cursor::new);
        match kumihan::atomic::save(&p, |f| {
            sheet::xlsx::write_with(&self.book, original, std::io::BufWriter::new(f))
        }) {
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
                bounds.origin.x
                    + px(HEAD_W + self.col_x(self.cursor.col) - self.col_x(self.view.col)),
                bounds.origin.y
                    + px(2.0 * ROW_H
                        + (self.view.row..self.cursor.row)
                            .map(|r| self.row_px(r))
                            .sum::<f32>()),
            ),
            size(px(self.col_px(self.cursor.col)), px(ROW_H)),
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
        // **格子の上に全面で重ねる。** 流れの中に置くと格子の右へ押し出され、
        // bounds が格子とずれてマウスが一切当たらなくなる(踏んで直した)
        s.position = gpui::Position::Absolute;
        s.inset.top = gpui::px(0.0).into();
        s.inset.left = gpui::px(0.0).into();
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
        // マウスは窓のレベルで受けて、座標からセルを逆算する(writer と同じ方式)。
        // セルごとのホバー判定に頼ると、ドラッグ中の移動を取り逃すことがある
        let view = self.view.clone();
        window.on_mouse_event(move |e: &gpui::MouseDownEvent, phase, _w, cx| {
            if phase != gpui::DispatchPhase::Bubble
                || e.button != gpui::MouseButton::Left
                || !bounds.contains(&e.position)
            {
                return;
            }
            let rel = e.position - bounds.origin;
            view.update(cx, |c, cx| {
                c.mouse_down_at(
                    f32::from(rel.x),
                    f32::from(rel.y),
                    e.modifiers.shift,
                    e.modifiers.control,
                );
                cx.notify();
            });
        });
        let view = self.view.clone();
        window.on_mouse_event(move |e: &gpui::MouseMoveEvent, phase, _w, cx| {
            // ドラッグ中は格子の外でも受ける(端で選択が止まらないように、
            // 位置は格子の中のセルに丸められる)
            if phase != gpui::DispatchPhase::Bubble
                || e.pressed_button != Some(gpui::MouseButton::Left)
            {
                return;
            }
            let rel = e.position - bounds.origin;
            view.update(cx, |c, cx| {
                if c.drag.is_some() {
                    c.mouse_drag_at(f32::from(rel.x), f32::from(rel.y));
                    cx.notify();
                }
            });
        });
        let view = self.view.clone();
        window.on_mouse_event(move |e: &gpui::MouseUpEvent, phase, _w, cx| {
            if phase != gpui::DispatchPhase::Bubble || e.button != gpui::MouseButton::Left {
                return;
            }
            view.update(cx, |c, cx| {
                c.mouse_up();
                cx.notify();
            });
        });
        // 右クリックでメニュー
        let view = self.view.clone();
        window.on_mouse_event(move |e: &gpui::MouseDownEvent, phase, _w, cx| {
            if phase != gpui::DispatchPhase::Bubble
                || e.button != gpui::MouseButton::Right
                || !bounds.contains(&e.position)
            {
                return;
            }
            let rel = e.position - bounds.origin;
            view.update(cx, |c, cx| {
                c.right_click_at(f32::from(rel.x), f32::from(rel.y));
                cx.notify();
            });
        });
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
        // 空いた帯は窓の取っ手(掴んで動かす・二度押しで最大化)。
        // GNOME の Wayland はサーバ側の飾りを付けてくれないので、自前で持つ
        tabs = tabs
            .child(div().flex_1().h(px(28.0)).id("drag")
                .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                    |_, e: &gpui::MouseDownEvent, window, _| {
                        if e.click_count >= 2 {
                            window.zoom_window();
                        } else {
                            window.start_window_move();
                        }
                    })))
            .child(div().pb_1p5().pr_2().text_size(px(10.5)).text_color(rgb(0x9CC9AF))
                   .child(SharedString::from(format!("calc — 実装済み {ready}/{all}"))));
        let winbtn = |id: &'static str, label: &'static str| {
            div().id(id).px_2p5().py_1().rounded_sm()
                .text_size(px(12.0)).text_color(rgb(0xCFE6D8))
                .cursor_pointer()
                .hover(move |s| if id == "close" { s.bg(rgb(0xC0392B)).text_color(rgb(0xFFFFFF)) }
                                else { s.bg(rgb(0x2E8B57)).text_color(rgb(0xFFFFFF)) })
                .child(label)
        };
        tabs = tabs
            .child(winbtn("min", "─").on_click(cx.listener(|_, _, window, _| {
                window.minimize_window();
            })))
            .child(winbtn("max", "▢").on_click(cx.listener(|_, _, window, _| {
                window.zoom_window();
            })))
            .child(winbtn("close", "✕").on_click(cx.listener(|this, _, _, cx| {
                if this.may_close() {
                    cx.quit();
                }
            })));

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
        for c in grid_cols(self.frozen, self.view, COLS) {
            head = head.child(div().w(px(self.col_px(c))).h(px(ROW_H))
                .bg(rgb(0xEFF2F4)).border_r_1().border_b_1()
                .border_color(rgb(0xD5DBE0))
                .flex().items_center().justify_center()
                .text_size(px(11.5)).text_color(rgb(0x66707A))
                .child(SharedString::from(col_name(c))));
        }
        grid = grid.child(head);

        // 当たり判定(cell_at)と同じ並びを使う — ずれるとクリックが別のセルに入る
        let visible: Vec<u32> = self.visible_rows();
        for r in visible {
            let rh = self.row_px(r);
            let mut row = div().flex().flex_row()
                .child(div().w(px(HEAD_W)).h(px(rh))
                    .bg(rgb(0xEFF2F4)).border_r_1().border_b_1()
                    .border_color(rgb(0xD5DBE0))
                    .flex().items_center().justify_center()
                    .text_size(px(11.5)).text_color(rgb(0x66707A))
                    .child(SharedString::from((r + 1).to_string())));
            for c in grid_cols(self.frozen, self.view, COLS) {
                let p = Pos::new(r, c);
                let cell = self.sheet().get(p);
                // 結合に呑まれた位置は空で描く(値は左上のセルにだけある)
                let v = if self.sheet().covered_by_merge(p) { Value::Empty }
                        else { cell.map(|x| x.value.clone()).unwrap_or(Value::Empty) };
                // 付けた表示形式は画面に出す。出ないなら飾りでしかない
                let shown = if self.show_formulas {
                    // 数式の表示。式が無いセルは値のまま
                    cell.and_then(|x| x.formula.clone())
                        .map(|f| format!("={f}"))
                        .unwrap_or_else(|| sheet::model::format_value(&v,
                            cell.and_then(|x| x.fmt.number_format.as_deref())))
                } else {
                    sheet::model::format_value(&v, cell.and_then(|x| x.fmt.number_format.as_deref()))
                };
                let is_num = matches!(v, Value::Number(_));
                let is_err = matches!(v, Value::Error(_));
                let sel = p == self.cursor;
                let (ra, rb) = self.sel_rect();
                let in_range = self.anchor.is_some()
                    && (ra.row..=rb.row).contains(&r) && (ra.col..=rb.col).contains(&c);
                let mut d = div()
                    .id(SharedString::from(p.a1()))
                    .w(px(self.col_px(c))).h(px(rh))
                    .border_r_1().border_b_1()
                    .border_color(if self.gridlines { rgb(0xE1E6EA) } else { rgb(0xFFFFFF) })
                    .bg(rgb(0xFFFFFF))
                    .flex().items_center()
                    .px_1p5()
                    .text_size(px(cell.and_then(|x| x.fmt.size_c)
                        .map(|c| c as f32 / 100.0 * 24.0 / 15.0 * 0.8)
                        .unwrap_or(12.5)))
                    .font_family("Noto Sans JP")
                    .overflow_hidden().whitespace_nowrap()
                    .cursor_pointer();
                // マウスの結線はセルではなく InputSink(窓レベル)にある。
                // セルの id は当たり判定ではなく描画の区別のためだけに残す
                // 罫線・塗り・文字書式。**帳票の見た目はここで決まる**
                let f = cell.map(|x| x.fmt.clone()).unwrap_or_default();
                let mut base = f.fill.as_deref().map(hex).unwrap_or(gpui::Rgba {
                    r: 1.0, g: 1.0, b: 1.0, a: 1.0,
                });
                // 条件付き書式。**付けた条件は画面に出す**(出ないなら飾り)
                let mut cond_color: Option<gpui::Rgba> = None;
                for rule in &self.sheet().cond {
                    if rule.hits(p, &v) {
                        if let Some(fill) = &rule.fill {
                            base = hex(fill);
                        }
                        if let Some(c) = &rule.color {
                            cond_color = Some(hex(c));
                        }
                    }
                }
                d = d.bg(base);
                // 範囲は下地に緑を**混ぜて**見せる(塗りは透けて残る)。
                // 色を抜くのは**起点のセル**(最初に選んだ方)— ドラッグで
                // 動くのは反対側の角なので、抜けが動き回らない(Excel の作法)
                let origin = self.anchor.unwrap_or(self.cursor);
                if in_range && p != origin {
                    d = d.bg(tint(base, 0.20));
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
                // セルの書体。無い書体は系統を保って代替(明朝→明朝)
                if let Some(name) = &f.font {
                    if let Ok((fam, _)) = kumihan::font::for_document(Some(name)) {
                        d = d.font_family(SharedString::from(fam.name.clone()));
                    }
                }
                // 引いてある辺だけ濃くする(引いていない辺は表の薄い線のまま)
                let ink = rgb(0x1B1B1B);
                if f.borders.top { d = d.border_t_1().border_color(ink) }
                if f.borders.bottom { d = d.border_b_1().border_color(ink) }
                if f.borders.left { d = d.border_l_1().border_color(ink) }
                if f.borders.right { d = d.border_r_1().border_color(ink) }
                // 太い枠は**選択の範囲の外周**に出す(Excel の作法)。
                // カーソルのセルに出すと、ドラッグ中は枠がマウスに付いて回る
                if self.anchor.is_some() {
                    if in_range {
                        let mut edge = false;
                        if r == ra.row { d = d.border_t_2(); edge = true }
                        if r == rb.row { d = d.border_b_2(); edge = true }
                        if c == ra.col { d = d.border_l_2(); edge = true }
                        if c == rb.col { d = d.border_r_2(); edge = true }
                        if edge {
                            d = d.border_color(rgb(0x1B6E3C));
                        }
                    }
                } else if sel {
                    d = d.border_2().border_color(rgb(0x1B6E3C));
                }
                // 縦の揃え(既定は下 = xlsx の既定)
                match f.valign {
                    sheet::model::VAlign::Top => d = d.items_start(),
                    sheet::model::VAlign::Middle => d = d.items_center(),
                    sheet::model::VAlign::Bottom => d = d.items_end(),
                }
                if f.wrap {
                    d = d.whitespace_normal().overflow_hidden();
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
                // 文字色の優先順: エラー > リンク > 条件 > セルの色 > 既定
                // (以前は最後に既定色で上書きしていて、セルの文字色が死んでいた)
                if is_err {
                    d = d.text_color(rgb(0xB3261E));
                } else if self.sheet().links.contains_key(&p) {
                    // リンクのあるセルは青(Ctrl+クリックで開く)
                    d = d.text_color(rgb(0x1F4E79));
                } else if let Some(c) = cond_color {
                    d = d.text_color(c);
                } else if f.color.is_none() {
                    d = d.text_color(rgb(0x1B1B1B));
                }
                // コメントのあるセルは右上に赤い角印
                if self.sheet().comments.contains_key(&p) {
                    d = d.relative().child(div().absolute()
                        .top(px(1.0)).right(px(1.0))
                        .w(px(6.0)).h(px(6.0)).rounded_sm().bg(rgb(0xC00000)));
                }
                // 選択中のセルは、確定前の入力をその場に見せる
                let shown = if sel { self.input.text().to_string() } else { shown };
                row = row.child(d.child(SharedString::from(shown)));
            }
            grid = grid.child(row);
        }

        // ---- シートの耳(Excel と同じく下に置く) ----
        let mut sheets_bar = div().flex().flex_row().items_center().gap_1()
            .px_3().py_1().bg(rgb(0xEFF2F4))
            .border_t_1().border_color(rgb(0xD5DBE0));
        for (i, s) in self.book.sheets.iter().enumerate() {
            let on = i == self.active;
            sheets_bar = sheets_bar.child(div()
                .id(SharedString::from(format!("sheet{i}")))
                .px_3().py_1().rounded_sm()
                .bg(if on { rgb(0xFFFFFF) } else { rgb(0xEFF2F4) })
                .border_1().border_color(if on { rgb(0x1B6E3C) } else { rgb(0xD5DBE0) })
                .text_size(px(11.5))
                .text_color(if on { rgb(0x1B6E3C) } else { rgb(0x66707A) })
                .font_weight(if on { gpui::FontWeight::BOLD } else { gpui::FontWeight::NORMAL })
                .cursor_pointer().hover(|s| s.bg(gpui::white()))
                .child(SharedString::from(s.name.clone()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.switch_sheet(i);
                    cx.notify()
                })));
        }
        sheets_bar = sheets_bar.child(div()
            .id("addsheet")
            .px_2().py_1().rounded_sm()
            .text_size(px(12.5)).text_color(rgb(0x1B6E3C))
            .cursor_pointer().hover(|s| s.bg(gpui::white()))
            .child("+")
            .on_click(cx.listener(|this, _, _, cx| {
                this.add_sheet();
                cx.notify()
            })));

        // ---- 右クリックのメニュー ----
        // **並びと名前は Euro-Office の右クリックメニューに合わせる**(リボンと
        // 同じ理由 — 乗り換える人が場所を覚え直さずに済む)。未実装は灰色。
        // AI・コメントなどの「入れないもの/まだ無いもの」も、場所だけは本家どおり。
        // InputSink より**後**に描く(bubble は後に登録した方が先に走るので、
        // 項目の stop_propagation が InputSink のセル選択より先に効く)
        let menu = self.menu_at.map(|(mx, my)| {
            // (id, 名前, 付記, 押せるか, 子メニューか)
            #[allow(clippy::type_complexity)]
            let entries: Vec<(&'static str, &'static str, &'static str, bool, bool)> = vec![
                ("cut", "切り取り", "Ctrl+X", true, false),
                ("copy", "コピー", "Ctrl+C", true, false),
                ("paste", "貼り付け", "Ctrl+V", true, false),
                ("", "", "", false, false),
                ("ins", "挿入", "", true, true),
                ("del", "削除", "", true, true),
                ("clr", "消去", "", true, true),
                ("", "", "", false, false),
                ("sort", "並べ替え", "", true, true),
                ("filter", "フィルター", "", true, true),
                ("reapply", "再適用", "", self.filter.is_some(), false),
                ("", "", "", false, false),
                ("addcomment", "コメントを追加", "", true, false),
                ("", "", "", false, false),
                ("fmtcells", "セルをフォーマットする", "", true, false),
                ("numfmt", "数値の書式", "", true, true),
                ("cond", "条件付き書式", "", true, true),
                ("picklist", "ドロップダウンリストから選択する", "", true, false),
                ("defname", "名前の定義", "", true, false),
                ("", "", "", false, false),
                ("func", "関数を挿入", "", true, true),
                ("hyperlink", "ハイパーリンク", "", true, false),
                ("", "", "", false, false),
                ("freeze", "枠の固定", "", true, false),
            ];
            // 画面の右・下で切れないように少し戻す
            const ITEM_H: f32 = 25.0;
            const SEP_H: f32 = 9.0;
            let h_est: f32 = entries.iter()
                .map(|e| if e.0.is_empty() && e.1.is_empty() { SEP_H } else { ITEM_H })
                .sum::<f32>() + 10.0;
            let grid_w = HEAD_W
                + grid_cols(self.frozen, self.view, COLS)
                    .iter()
                    .map(|c| self.col_px(*c))
                    .sum::<f32>();
            let grid_h = ROW_H + ROWS as f32 * ROW_H;
            let mx = mx.min((grid_w - 250.0).max(0.0));
            let my = my.min((grid_h - h_est).max(0.0));

            let mut m = div().absolute().left(px(mx)).top(px(my)).w(px(244.0))
                .p_1().rounded_md().bg(rgb(0xFFFFFF))
                .border_1().border_color(rgb(0xC6CDD3)).shadow_lg()
                // メニューの余白を押してもセルに抜けない
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation());
            // 開いている子メニューの縦位置(親項目の高さに合わせる)
            let mut sub_panel: Option<gpui::Div> = None;
            let mut y_acc = 4.0f32;
            for (i, (id, label, hint, ready, is_sub)) in entries.iter().enumerate() {
                let (id, label, hint, ready, is_sub) = (*id, *label, *hint, *ready, *is_sub);
                if id.is_empty() && label.is_empty() {
                    m = m.child(div().h(px(1.0)).my_1().bg(rgb(0xE1E6EA)));
                    y_acc += SEP_H;
                    continue;
                }
                let row_y = y_acc;
                y_acc += ITEM_H;
                if !ready {
                    // 未実装。押せるように見せない(場所だけ本家どおりに残す)
                    m = m.child(div()
                        .flex().flex_row().items_center().justify_between().gap_4()
                        .px_3().py_1()
                        .child(div().text_size(px(12.5)).text_color(rgb(0xB6BDC4))
                            .child(label))
                        .child(div().text_size(px(10.5)).text_color(rgb(0xD5DBE0))
                            .child(if is_sub { "▸" } else { hint })));
                    continue;
                }
                if is_sub {
                    let open = self.menu_sub == Some(id);
                    m = m.child(div()
                        .id(SharedString::from(format!("m{i}")))
                        .flex().flex_row().items_center().justify_between().gap_4()
                        .px_3().py_1().rounded_sm().cursor_pointer()
                        .bg(if open { rgb(0xEAF5EE) } else { rgb(0xFFFFFF) })
                        .hover(|s| s.bg(rgb(0xEAF5EE)))
                        .child(div().text_size(px(12.5)).text_color(rgb(0x1B1B1B))
                            .child(label))
                        .child(div().text_size(px(11.0)).text_color(rgb(0x66707A)).child("▸"))
                        // 触れたら開く(本家と同じ)。押しても開く
                        .on_mouse_move(cx.listener(move |this, _, _, cx| {
                            if this.menu_sub != Some(id) {
                                this.menu_sub = Some(id);
                                cx.notify();
                            }
                        }))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                            move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.menu_sub = Some(id);
                                cx.notify();
                            })));
                    if open {
                        // 子の板。親項目の右横に出す
                        let mut sp = div().absolute()
                            .left(px(mx + 244.0)).top(px(my + row_y))
                            .w(px(210.0)).p_1().rounded_md().bg(rgb(0xFFFFFF))
                            .border_1().border_color(rgb(0xC6CDD3)).shadow_lg()
                            .on_mouse_down(gpui::MouseButton::Left,
                                |_, _, cx| cx.stop_propagation());
                        for (j, (sid, slabel, sready)) in
                            self.menu_sub_entries(id).into_iter().enumerate()
                        {
                            if !sready {
                                sp = sp.child(div().px_3().py_1()
                                    .text_size(px(12.5)).text_color(rgb(0xB6BDC4))
                                    .child(slabel));
                                continue;
                            }
                            sp = sp.child(div()
                                .id(SharedString::from(format!("s{i}-{j}")))
                                .px_3().py_1().rounded_sm().cursor_pointer()
                                .hover(|s| s.bg(rgb(0xEAF5EE)))
                                .text_size(px(12.5)).text_color(rgb(0x1B1B1B))
                                .child(slabel)
                                .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                                    move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.menu_action(sid, window, cx);
                                    })));
                        }
                        sub_panel = Some(sp);
                    }
                    continue;
                }
                // 普通の項目
                m = m.child(div()
                    .id(SharedString::from(format!("m{i}")))
                    .flex().flex_row().items_center().justify_between().gap_4()
                    .px_3().py_1().rounded_sm().cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF5EE)))
                    .child(div().text_size(px(12.5)).text_color(rgb(0x1B1B1B))
                        .child(label))
                    .child(div().text_size(px(10.5)).text_color(rgb(0x9AA5AE)).child(hint))
                    // 実行できる普通の項目に触れたら、開いていた子は閉じる
                    .on_mouse_move(cx.listener(move |this, _, _, cx| {
                        if this.menu_sub.is_some() {
                            this.menu_sub = None;
                            cx.notify();
                        }
                    }))
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                        move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.menu_action(id, window, cx);
                        })));
            }
            div().absolute().left(px(0.0)).top(px(0.0)).size_full()
                .child(m)
                .children(sub_panel)
        });

        // ---- カーソルのセルの付記(コメント・リンク) ----
        let mut tip_lines: Vec<String> = Vec::new();
        if let Some(t) = self.sheet().comments.get(&self.cursor) {
            tip_lines.push(t.clone());
        }
        if let Some(u) = self.sheet().links.get(&self.cursor) {
            tip_lines.push(format!("リンク: {u}(Ctrl+クリックで開く)"));
        }
        let tip = if tip_lines.is_empty() {
            None
        } else {
            self.cell_origin_px(self.cursor).map(|(x, y)| {
                let mut t = div().absolute()
                    .left(px(x + self.col_px(self.cursor.col) + 6.0))
                    .top(px(y))
                    .max_w(px(280.0)).p_2().rounded_md()
                    .bg(rgb(0xFFF9DB)).border_1().border_color(rgb(0xE0C97F)).shadow_lg();
                for line in tip_lines {
                    t = t.child(div().text_size(px(11.5)).text_color(rgb(0x5C4A00))
                        .child(SharedString::from(line)));
                }
                t
            })
        };

        // ---- 入力の板(名前の定義など) ----
        let prompt_panel = self.prompt.as_ref().map(|(kind, ed)| {
            let (a, b) = self.sel_rect();
            let range = if self.anchor.is_some() {
                format!("{}:{}", a.a1(), b.a1())
            } else {
                a.a1()
            };
            let title = match *kind {
                "name" => format!("名前の定義 — {range} に名前を付ける"),
                "comment" => format!("コメント — {}(空にして Enter で消す)", self.cursor.a1()),
                "link" => format!("ハイパーリンク — {}(空にして Enter で外す)", self.cursor.a1()),
                "cond-gt" => format!("条件付き書式 — {range} で、いくつより大きい値を塗る?"),
                "cond-lt" => format!("条件付き書式 — {range} で、いくつより小さい値を塗る?"),
                _ => String::new(),
            };
            // キャレットは | で見せる(writer の検索欄と同じ割り切り)
            let mut text = ed.text().to_string();
            let cur = ed.cursor().min(text.len());
            text.insert(cur, '|');
            div().absolute().left(px(HEAD_W + 40.0)).top(px(ROW_H + 24.0)).w(px(380.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0x1B6E3C)).shadow_lg()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(div().text_size(px(12.0)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x1B6E3C)).child(SharedString::from(title)))
                .child(div().mt_1p5().px_2().py_1().bg(rgb(0xFFFFFF))
                    .border_1().border_color(rgb(0xC6CDD3)).rounded_sm()
                    .text_size(px(13.0)).font_family("Noto Sans JP")
                    .child(SharedString::from(text)))
                .child(div().mt_1().text_size(px(10.5)).text_color(rgb(0x66707A))
                    .child("Enter で決定 / Esc で取消。定義した名前は式の中で使えます(=単価*2)"))
        });

        // ---- 書式の小窓(セルをフォーマットする) ----
        // モーダルにしない: 範囲を選び直しながら続けて使える道具箱。
        // どの釦も既存の書式の道(fmt / run_cmd)を通り、1手ずつ戻せる
        let fmt_panel = self.fmt_panel.map(|(fx, fy)| {
            let fx = fx.min(560.0);
            let fy = fy.min(320.0);
            let btn = |id: &'static str, label: &'static str| {
                div().id(id).px_2().py_1().rounded_sm()
                    .border_1().border_color(rgb(0xC6CDD3))
                    .text_size(px(11.5)).text_color(rgb(0x1B1B1B))
                    .cursor_pointer().hover(|s| s.bg(rgb(0xEAF5EE)))
                    .child(label)
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                        move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.fmt_panel_action(id);
                            cx.notify();
                        }))
            };
            let swatch = |id: &'static str, color: Option<&'static str>| {
                let mut s = div().id(id).w(px(20.0)).h(px(20.0)).rounded_sm()
                    .border_1().border_color(rgb(0xC6CDD3))
                    .cursor_pointer();
                s = match color {
                    Some(c) => s.bg(hex(c)),
                    // 「なし」は斜線の代わりに白+薄字の×
                    None => s.bg(rgb(0xFFFFFF)).flex().items_center().justify_center()
                        .text_size(px(10.0)).text_color(rgb(0x9AA5AE)).child("×"),
                };
                s.on_mouse_down(gpui::MouseButton::Left, cx.listener(
                    move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.fmt_panel_action(id);
                        cx.notify();
                    }))
            };
            let title = |t: &'static str| div().text_size(px(10.5))
                .text_color(rgb(0x66707A)).mt_1p5().child(t);
            let row = || div().flex().flex_row().flex_wrap().gap_1().items_center();

            div().absolute().left(px(fx)).top(px(fy)).w(px(300.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3)).shadow_lg()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(div().flex().flex_row().items_center().justify_between()
                    .child(div().text_size(px(12.5)).font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(0x1B6E3C))
                        .child("セルの書式(選んでいる範囲に効く)"))
                    .child(div().id("fmtclose").px_2().rounded_sm().cursor_pointer()
                        .text_size(px(12.0)).text_color(rgb(0x66707A))
                        .hover(|s| s.bg(rgb(0xE1E6EA)))
                        .child("✕")
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                            move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.fmt_panel = None;
                                cx.notify();
                            }))))
                .child(title("罫線"))
                .child(row()
                    .child(btn("b-all", "格子"))
                    .child(btn("b-out", "外枠"))
                    .child(btn("b-none", "なし")))
                .child(title("塗り"))
                .child(row()
                    .child(swatch("fill-none", None))
                    .child(swatch("fill-FFF2CC", Some("FFF2CC")))
                    .child(swatch("fill-DEEAF6", Some("DEEAF6")))
                    .child(swatch("fill-E2EFDA", Some("E2EFDA")))
                    .child(swatch("fill-FCE4D6", Some("FCE4D6")))
                    .child(swatch("fill-D9D9D9", Some("D9D9D9"))))
                .child(title("文字の色"))
                .child(row()
                    .child(swatch("color-none", None))
                    .child(swatch("color-C00000", Some("C00000")))
                    .child(swatch("color-1F4E79", Some("1F4E79")))
                    .child(swatch("color-1B6E3C", Some("1B6E3C")))
                    .child(swatch("color-7F7F7F", Some("7F7F7F"))))
                .child(title("文字"))
                .child(row()
                    .child(btn("bold", "太字"))
                    .child(btn("italic", "斜体"))
                    .child(btn("underline", "下線"))
                    .child(btn("strikeout", "取り消し"))
                    .child(btn("incfont", "大きく"))
                    .child(btn("decfont", "小さく")))
                .child(title("揃え"))
                .child(row()
                    .child(btn("align-left", "左"))
                    .child(btn("align-center", "中央"))
                    .child(btn("align-right", "右"))
                    .child(btn("top", "上"))
                    .child(btn("middle", "中"))
                    .child(btn("bottom", "下"))
                    .child(btn("wrap", "折り返し")))
                .child(title("表示形式"))
                .child(row()
                    .child(btn("comma", "1,000"))
                    .child(btn("currency", "¥"))
                    .child(btn("percents", "%"))
                    .child(btn("digit-inc", ".0+"))
                    .child(btn("digit-dec", ".0−"))
                    .child(btn("numfmt-none", "なし")))
        });

        // ---- ドロップダウンリスト(同じ列の値の一覧) ----
        let pick_panel = self.pick.clone().map(|(vals, (vx, vy))| {
            let mut p = div().absolute().left(px(vx)).top(px(vy))
                .w(px(self.col_px(self.cursor.col).max(120.0)))
                .p_1().rounded_md().bg(rgb(0xFFFFFF))
                .border_1().border_color(rgb(0xC6CDD3)).shadow_lg()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation());
            for (i, v) in vals.into_iter().enumerate() {
                p = p.child(div()
                    .id(SharedString::from(format!("pk{i}")))
                    .px_2().py_1().rounded_sm().cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF5EE)))
                    .text_size(px(12.5)).text_color(rgb(0x1B1B1B))
                    .whitespace_nowrap().overflow_hidden()
                    .child(SharedString::from(v.clone()))
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                        move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.pick = None;
                            this.pick_value(&v);
                            cx.notify();
                        })));
            }
            p
        });

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
            .on_action(cx.listener(Calc::a_copy))
            .on_action(cx.listener(Calc::a_cut))
            .on_action(cx.listener(Calc::a_paste))
            .on_action(cx.listener(Calc::a_left))
            .on_action(cx.listener(Calc::a_right))
            .on_action(cx.listener(Calc::a_up))
            .on_action(cx.listener(Calc::a_down))
            .on_action(cx.listener(Calc::a_tab))
            .on_action(cx.listener(Calc::a_enter))
            .on_action(cx.listener(Calc::a_select_all))
            .on_action(cx.listener(Calc::a_redo))
            .on_action(cx.listener(Calc::a_select_left))
            .on_action(cx.listener(Calc::a_select_right))
            .on_action(cx.listener(Calc::a_select_up))
            .on_action(cx.listener(Calc::a_select_down))
            .on_action(cx.listener(Calc::a_undo))
            .on_action(cx.listener(Calc::a_save))
            .on_action(cx.listener(Calc::a_open))
            .on_action(cx.listener(Calc::a_quit))
            .on_action(cx.listener(Calc::a_context_menu))
            .on_action(cx.listener(Calc::a_cancel))
            .child(bar)
            .child(formula_bar)
            .child(div().flex_1().overflow_hidden().relative()
                   // ホイールで窓を動かす(下に回すと先の行が見える)
                   .on_scroll_wheel(cx.listener(|this, e: &gpui::ScrollWheelEvent, _, cx| {
                       let (dx, dy) = match e.delta {
                           gpui::ScrollDelta::Pixels(p) =>
                               (-f32::from(p.x) / COL_W, -f32::from(p.y) / ROW_H),
                           gpui::ScrollDelta::Lines(l) => (-l.x, -l.y * 3.0),
                       };
                       this.wheel.0 += dy;
                       this.wheel.1 += dx;
                       let dr = this.wheel.0.trunc() as i32;
                       let dc = this.wheel.1.trunc() as i32;
                       this.wheel.0 -= dr as f32;
                       this.wheel.1 -= dc as f32;
                       if dr != 0 || dc != 0 {
                           this.view.row = (this.view.row as i32 + dr).clamp(0, 9999) as u32;
                           this.view.col = (this.view.col as i32 + dc).clamp(0, 255) as u32;
                           cx.notify();
                       }
                   }))
                   .child(grid)
                   .child(InputSink { view: me })
                   .children(tip)
                   .children(fmt_panel)
                   .children(menu)
                   .children(pick_panel)
                   .children(prompt_panel))
            .child(sheets_bar)
            .children(notes)
    }
}

fn main() {
    let arg = std::env::args().nth(1).map(PathBuf::from);
    application().with_assets(ui::Icons).run(move |cx: &mut App| {
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
                // WM からの「閉じる」(Alt+F4 等)も同じ確認を通す。
                // 書きかけを黙って捨てない
                let v = view.clone();
                window.on_window_should_close(cx, move |_, cx| {
                    let ok = v.update(cx, |this, _| this.may_close());
                    if ok {
                        cx.quit();
                    }
                    ok
                });
                view
            },
        )
        .unwrap();
        cx.activate(true);
    });
}

#[cfg(test)]
mod freeze_tests {
    use super::*;

    #[test]
    fn 固定した行は窓が動いても頭に残る() {
        // 見出し行(0)を固定して、窓が10行目に居ても 0 行目が出る
        let rows = grid_rows(Some(Pos::new(1, 1)), Pos::new(10, 5), 5);
        assert_eq!(rows[0], 0, "固定した見出しが消えた: {rows:?}");
        assert_eq!(rows[1], 10, "続きが窓から始まっていない: {rows:?}");
        let cols = grid_cols(Some(Pos::new(1, 1)), Pos::new(10, 5), 4);
        assert_eq!(cols, vec![0, 5, 6, 7], "{cols:?}");
    }

    #[test]
    fn 固定なしなら窓のまま() {
        assert_eq!(grid_rows(None, Pos::new(3, 0), 4), vec![3, 4, 5, 6]);
    }

    #[test]
    fn 窓が固定の中に居ても重複しない() {
        // 窓が先頭にあるとき、固定行と窓の行が二重に出ない
        let rows = grid_rows(Some(Pos::new(2, 0)), Pos::new(0, 0), 5);
        let mut sorted = rows.clone();
        sorted.dedup();
        assert_eq!(rows.len(), sorted.len(), "行が二重に出た: {rows:?}");
    }
}

#[cfg(test)]
mod filter_tests {
    use super::*;

    #[test]
    fn 一致した行と見出しだけが残る() {
        let mut b = Book::default();
        b.sheets.push(sheet::Sheet { name: "表".into(), ..Default::default() });
        let s = &mut b.sheets[0];
        for (r, v) in [(0, "区分"), (1, "甲"), (2, "乙"), (3, "甲")] {
            s.set(Pos::new(r, 0), Cell::input(v));
        }
        // Calc を組み立てずに、絞り込みの規則だけ確かめる
        let matching = |col: u32, v: &str| -> Vec<u32> {
            let (rows, _) = s.extent();
            let mut out = vec![0];
            for r in 1..rows {
                if s.get(Pos::new(r, col)).map(|c| c.value.display()).as_deref() == Some(v) {
                    out.push(r);
                }
            }
            out
        };
        assert_eq!(matching(0, "甲"), vec![0, 1, 3], "見出し+一致行でない");
        assert_eq!(matching(0, "乙"), vec![0, 2]);
        assert_eq!(matching(0, "丙"), vec![0], "無い値は見出しだけ");
    }
}

#[cfg(test)]
mod sheet_name_tests {
    use super::*;

    #[test]
    fn 足すシートの名前がぶつからない() {
        let mut b = Book::new(); // Sheet1
        assert_eq!(unique_sheet_name(&b), "Sheet2");
        b.sheets.push(sheet::Sheet::new("Sheet2"));
        b.sheets.push(sheet::Sheet::new("Sheet3"));
        assert_eq!(unique_sheet_name(&b), "Sheet4");
        // 歯抜け(途中の名前が消えた等)でも重複しない
        b.sheets.remove(1);
        let n = unique_sheet_name(&b);
        assert!(!b.sheets.iter().any(|s| s.name == n), "重複した: {n}");
    }
}

#[cfg(test)]
mod clipboard_tests {
    use super::*;

    fn table() -> sheet::Sheet {
        let mut s = sheet::Sheet { name: "表".into(), ..Default::default() };
        s.set(Pos::new(0, 0), Cell::input("品名"));
        s.set(Pos::new(0, 1), Cell::input("金額"));
        s.set(Pos::new(1, 0), Cell::input("甲"));
        s.set(Pos::new(1, 1), Cell::input("=A2&\"円\""));
        s
    }

    #[test]
    fn コピーはtsvで式が残る() {
        let s = table();
        let tsv = range_tsv(&s, Pos::new(0, 0), Pos::new(1, 1));
        assert_eq!(tsv, "品名\t金額\n甲\t=A2&\"円\"", "TSV の形が違う: {tsv:?}");
    }

    #[test]
    fn 空セルは空欄として出る() {
        let s = table();
        let tsv = range_tsv(&s, Pos::new(0, 0), Pos::new(2, 1));
        assert!(tsv.ends_with("\n\t"), "空行の形が違う: {tsv:?}");
    }

    #[test]
    fn アプリ内の貼り付けは式がずれる() {
        let mut s = table();
        // B2 の式(=A2&"円")を B4 へ: 2行下 → =A4&"円"
        let grid = vec![vec!["=A2&\"円\"".to_string()]];
        paste_grid(&mut s, Pos::new(3, 1), &grid, Some((2, 0)));
        assert_eq!(
            s.get(Pos::new(3, 1)).and_then(|c| c.formula.clone()).as_deref(),
            Some("A4&\"円\""),
            "式の参照がずれていない"
        );
    }

    #[test]
    fn 外から来たtsvは式をずらさない() {
        let mut s = sheet::Sheet { name: "表".into(), ..Default::default() };
        let grid = tsv_grid("甲\t100\r\n乙\t=A1*2\n");
        let n = paste_grid(&mut s, Pos::new(0, 0), &grid, None);
        assert_eq!(n, 4);
        assert_eq!(s.value(Pos::new(0, 1)), Value::Number(100.0));
        assert_eq!(
            s.get(Pos::new(1, 1)).and_then(|c| c.formula.clone()).as_deref(),
            Some("A1*2"),
            "外来の式を勝手にずらした"
        );
    }

    #[test]
    fn 貼り付けても書式は据え置き() {
        // 帳票の枠(罫線)の上に値を貼っても枠が残る
        let mut s = sheet::Sheet { name: "枠".into(), ..Default::default() };
        s.set(Pos::new(0, 0), Cell {
            formula: None,
            value: Value::Empty,
            fmt: CellFormat { borders: Borders::ALL, ..Default::default() },
        });
        paste_grid(&mut s, Pos::new(0, 0), &[vec!["100".to_string()]], None);
        let c = s.get(Pos::new(0, 0)).unwrap();
        assert_eq!(c.value, Value::Number(100.0));
        assert_eq!(c.fmt.borders, Borders::ALL, "貼り付けで罫線が消えた");
    }

    #[test]
    fn 改行コードと末尾改行を受け流す() {
        assert_eq!(tsv_grid("a\tb\r\nc\td\r\n"),
                   vec![vec!["a".to_string(), "b".into()], vec!["c".into(), "d".into()]]);
        assert_eq!(tsv_grid("1"), vec![vec!["1".to_string()]]);
    }
}

#[cfg(test)]
mod wiring_tests {
    #[test]
    fn リボンのreadyは全部配線されている() {
        for tab in ui::ribbon::CALC {
            for cmd in tab.cmds {
                if cmd.ready {
                    assert!(
                        super::Calc::HANDLED.contains(&cmd.id),
                        "「{}」({}) は ready なのに run_cmd が知らない",
                        cmd.label, cmd.id
                    );
                }
            }
        }
    }
}

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

/// 境界の取っ手の当たり幅(縁から前後この px 以内で掴める)。
/// 見出しのクリックに他の意味は無いので、広めに取って掴みやすくする
const GRIP: f32 = 5.0;

/// `start` から `sizes` の幅で並ぶ区分のうち、`pos` がどの区分の
/// 右端(下端)±GRIP に掛かるかを返す。列見出し・行見出しの境界の当たり判定。
fn grip_hit(sizes: &[(u32, f32)], start: f32, pos: f32) -> Option<u32> {
    let mut edge = start;
    for (i, w) in sizes {
        edge += w;
        if (pos - edge).abs() <= GRIP {
            return Some(*i);
        }
    }
    None
}

/// `start` から `sizes` の幅で並ぶ区分のうち、`pos` がどの区分の中に
/// 入るかを返す。見出しのクリック(列・行の選択)の当たり判定。
fn index_at(sizes: &[(u32, f32)], start: f32, pos: f32) -> Option<u32> {
    let mut x = start;
    for (i, w) in sizes {
        if pos >= x && pos < x + w {
            return Some(*i);
        }
        x += w;
    }
    None
}

/// 見出しの境界を掴んだドラッグ(列幅・行高を変える)
struct SizeDrag {
    /// 列か(false なら行)
    col: bool,
    idx: u32,
    /// 掴んだ位置(px。列なら x、行なら y)
    grab: f32,
    /// 掴んだときの大きさ(px)
    base: f32,
    /// 動かしたか。**最初に動いた瞬間に undo の控えを取る** —
    /// 掴んだだけ(クリック)で redo の控えが消えるのを防ぐ
    moved: bool,
}

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

/// 行と列を入れ替える(転置)。歯抜けは空欄として埋める。
fn transpose<T: Clone + Default>(g: &[Vec<T>]) -> Vec<Vec<T>> {
    let rows = g.len();
    let cols = g.iter().map(|r| r.len()).max().unwrap_or(0);
    (0..cols)
        .map(|c| {
            (0..rows)
                .map(|r| g[r].get(c).cloned().unwrap_or_default())
                .collect()
        })
        .collect()
}

/// 控えたセルの**値だけ**を流し込む(式は計算結果の値になる)。書式は据え置き。
/// 控えの空セルは中身を消す(書式は残す)— 空も「値」のうち。
fn paste_values_cells(s: &mut sheet::Sheet, at: Pos, cells: &[Vec<Option<Cell>>]) -> usize {
    let mut n = 0usize;
    for (dr, row) in cells.iter().enumerate() {
        for (dc, src) in row.iter().enumerate() {
            let p = Pos::new(at.row + dr as u32, at.col + dc as u32);
            let fmt = s.get(p).map(|c| c.fmt.clone()).unwrap_or_default();
            let value = src.as_ref().map(|c| c.value.clone()).unwrap_or(Value::Empty);
            s.set(p, Cell { formula: None, value, fmt });
            n += 1;
        }
    }
    n
}

/// 外から来た TSV の**値だけ**を流し込む。`=` で始まる欄は式にせず文字として置く
/// (外の式は計算できない — 黙って別の意味にしない)。
fn paste_values_text(s: &mut sheet::Sheet, at: Pos, grid: &[Vec<String>]) -> usize {
    let mut n = 0usize;
    for (dr, row) in grid.iter().enumerate() {
        for (dc, text) in row.iter().enumerate() {
            let p = Pos::new(at.row + dr as u32, at.col + dc as u32);
            let fmt = s.get(p).map(|c| c.fmt.clone()).unwrap_or_default();
            let mut cell = if text.starts_with('=') {
                Cell { formula: None, value: Value::Text(text.clone()), fmt: Default::default() }
            } else {
                Cell::input(text)
            };
            cell.fmt = fmt;
            s.set(p, cell);
            n += 1;
        }
    }
    n
}

/// 控えたセルの**書式だけ**を写す(中身は残す)。帳票の枠の使い回し。
fn paste_formats(s: &mut sheet::Sheet, at: Pos, cells: &[Vec<Option<Cell>>]) -> usize {
    let mut n = 0usize;
    for (dr, row) in cells.iter().enumerate() {
        for (dc, src) in row.iter().enumerate() {
            let p = Pos::new(at.row + dr as u32, at.col + dc as u32);
            let fmt = src.as_ref().map(|c| c.fmt.clone()).unwrap_or_default();
            let mut cell = s.get(p).cloned().unwrap_or_default();
            cell.fmt = fmt;
            s.set(p, cell);
            n += 1;
        }
    }
    n
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
    /// 見出しの境界を掴んだドラッグ(列幅・行高)。セル選択の drag とは別
    size_drag: Option<SizeDrag>,
    /// 見出しを掴んだ選択ドラッグ(列か, 始まりの番号)。B→D と撫でて複数列
    head_drag: Option<(bool, u32)>,
    /// 画像の復号の控え(実体のアドレス → GPUI の画像)。
    /// 毎フレーム作り直すと復号と転送をやり直すことになる
    img_cache: std::cell::RefCell<std::collections::HashMap<usize, std::sync::Arc<gpui::Image>>>,
    /// 検索と置換の検索語(板を2枚続けて使う間の控え。次回の初期値にもなる)
    find_term: Option<String>,
    /// ゴールシークの途中の控え(目標セル, 目標値)
    goal: Option<(Pos, f64)>,
    /// PY のスピルの台帳(シート番号, 錨 → 行×列)。次の @計算 で前の面を消す
    py_spills: std::collections::HashMap<(usize, Pos), (u32, u32)>,
    /// トレースの光り(参照元=青緑 / 参照先=橙)。見え方だけ、保存されない
    trace: Vec<(Pos, bool)>,
    /// 自分が置いた排他ロック(閉じるとき・別のファイルを開くときに外す)
    my_lock: Option<PathBuf>,
    /// 先客の名乗り(このファイルは誰かが開いている)。上書き保存を止める
    locked_by: Option<String>,
    /// 選択中の図形(shapes_new の番号)。Esc/他クリックで解除、Del で削除
    shape_sel: Option<usize>,
    /// 図形のドラッグ(番号, 掴んだ格子px, 掴んだ時の錨の格子px, 大きさ変更か)
    shape_drag: Option<(usize, (f32, f32), (f32, f32), bool)>,
    /// ホイールの端数(触板の細かい送りを捨てずに貯める)
    wheel: (f32, f32),
    /// 右クリックのメニュー(出ている場所。格子領域の px)
    menu_at: Option<(f32, f32)>,
    /// 開いている子メニュー(挿入▸ など)
    menu_sub: Option<&'static str>,
    /// 「ドロップダウンリストから選択」などの一覧(候補, 出す場所)
    pick: Option<(Vec<String>, (f32, f32))>,
    /// pick の中身の意味: "value"=セルに入れる / "font"=書体 / "size"=文字の大きさ
    pick_kind: &'static str,
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
    /// 1手 = シートの控えの束。普通の操作は1枚、Python の実行のように
    /// 複数シートに触るものは全部まとめて1手(どれでも1手で戻せる)。
    /// **どのシートの控えかを一緒に持つ** — シートを切り替えた後の undo が
    /// 別のシートへ他所の中身を書き戻す事故を防ぐ
    undo_stack: Vec<Vec<(usize, sheet::Sheet)>>,
    redo_stack: Vec<Vec<(usize, sheet::Sheet)>>,
    /// シートごとのカーソル・窓・固定(切り替えても場所を失わない)
    sheet_ui: Vec<(Pos, Pos, Option<Pos>)>,
    /// コピーの控え(起点, そのとき書いた TSV)。貼り付け時に系のクリップボードと
    /// 突き合わせ、一致すればアプリ内コピーとして式の参照をずらす
    clip: Option<(Pos, String)>,
    /// コピーの控え(セルそのもの)。形式を選択して貼り付け(値だけ・書式だけ)に使う
    clip_cells: Option<Vec<Vec<Option<Cell>>>>,
    /// コピーした範囲(シート, 左上, 右下)。破線の枠で見せる。Esc で消える
    clip_range: Option<(usize, Pos, Pos)>,
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
            size_drag: None,
            head_drag: None,
            img_cache: Default::default(),
            find_term: None,
            goal: None,
            py_spills: Default::default(),
            trace: Vec::new(),
            my_lock: None,
            locked_by: None,
            shape_sel: None,
            shape_drag: None,
            wheel: (0.0, 0.0),
            menu_at: None,
            menu_sub: None,
            pick: None,
            pick_kind: "value",
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
            clip_cells: None,
            clip_range: None,
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
        self.undo_stack
            .push(vec![(self.active, self.book.sheets[self.active].clone())]);
        if self.undo_stack.len() > 100 {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    /// 全シートを1手として控える(Python の実行など、どこを変えるか
    /// 分からない操作の前に)。
    fn checkpoint_book(&mut self) {
        self.undo_stack.push(
            self.book
                .sheets
                .iter()
                .cloned()
                .enumerate()
                .collect(),
        );
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
        let Some(batch) = self.undo_stack.pop() else {
            self.status = "戻すものがありません".into();
            return;
        };
        let mut redo = Vec::new();
        let first = batch.first().map(|(i, _)| *i);
        for (idx, prev) in batch {
            if idx < self.book.sheets.len() {
                redo.push((idx, self.book.sheets[idx].clone()));
                self.book.sheets[idx] = prev;
                recalc(&mut self.book.sheets[idx]);
            }
        }
        self.redo_stack.push(redo);
        if let Some(i) = first {
            self.show_sheet(i);
        }
        self.dirty = true;
        self.sync_input();
        self.status = "戻しました".into();
    }

    fn redo_sheet(&mut self) {
        let Some(batch) = self.redo_stack.pop() else {
            self.status = "やり直すものがありません".into();
            return;
        };
        let mut undo = Vec::new();
        let first = batch.first().map(|(i, _)| *i);
        for (idx, next) in batch {
            if idx < self.book.sheets.len() {
                undo.push((idx, self.book.sheets[idx].clone()));
                self.book.sheets[idx] = next;
                recalc(&mut self.book.sheets[idx]);
            }
        }
        self.undo_stack.push(undo);
        if let Some(i) = first {
            self.show_sheet(i);
        }
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
        Some(Pos { row: self.row_at(y)?, col: self.col_at(x)? })
    }

    /// この x はどの列の上か(見出し・セルのどちらでも)。
    fn col_at(&self, x: f32) -> Option<u32> {
        let cols: Vec<(u32, f32)> = grid_cols(self.frozen, self.view, COLS)
            .into_iter()
            .map(|c| (c, self.col_px(c)))
            .collect();
        index_at(&cols, HEAD_W, x)
    }

    fn row_at(&self, y: f32) -> Option<u32> {
        let rows: Vec<(u32, f32)> = self
            .visible_rows()
            .into_iter()
            .map(|r| (r, self.row_px(r)))
            .collect();
        index_at(&rows, ROW_H, y)
    }

    /// 列をまるごと選ぶ(使われている高さまで)。`a` が起点、`b` が動く側。
    fn select_cols(&mut self, a: u32, b: u32) {
        let rows = self.sheet().extent().0.max(1);
        self.anchor = Some(Pos::new(rows - 1, a));
        self.cursor = Pos::new(0, b);
        self.sync_input();
        let (lo, hi) = (a.min(b), a.max(b));
        self.status = if lo == hi {
            format!("{}列を選択しました(1〜{rows}行)", col_name(lo)).into()
        } else {
            format!("{}〜{}列を選択しました(1〜{rows}行)", col_name(lo), col_name(hi)).into()
        };
    }

    /// 行をまるごと選ぶ(使われている幅まで)。
    fn select_rows(&mut self, a: u32, b: u32) {
        let cols = self.sheet().extent().1.max(1);
        self.anchor = Some(Pos::new(a, cols - 1));
        self.cursor = Pos::new(b, 0);
        self.sync_input();
        let (lo, hi) = (a.min(b), a.max(b));
        self.status = if lo == hi {
            format!("{}行を選択しました", lo + 1).into()
        } else {
            format!("{}〜{}行を選択しました", lo + 1, hi + 1).into()
        };
    }

    /// 見出しの帯の上の、列幅・行高の取っ手(境界 ±GRIP px)。Some((列か, 番号))。
    /// 描画・cell_at と同じ並び(固定・窓・絞り込み)を使う —
    /// ずれると別の境界を掴んでしまう。
    fn size_grip_at(&self, x: f32, y: f32) -> Option<(bool, u32)> {
        if y < ROW_H && x >= HEAD_W {
            let cols: Vec<(u32, f32)> = grid_cols(self.frozen, self.view, COLS)
                .into_iter()
                .map(|c| (c, self.col_px(c)))
                .collect();
            return grip_hit(&cols, HEAD_W, x).map(|c| (true, c));
        }
        if x < HEAD_W && y >= ROW_H {
            let rows: Vec<(u32, f32)> = self
                .visible_rows()
                .into_iter()
                .map(|r| (r, self.row_px(r)))
                .collect();
            return grip_hit(&rows, ROW_H, y).map(|r| (false, r));
        }
        None
    }

    /// 境界を掴んだまま動いた。列幅・行高をその場で変える(見ながら合わせる)。
    /// 最小幅で止める — ゼロにすると列が消えて掴み直せない。
    fn size_drag_at(&mut self, x: f32, y: f32) {
        if std::env::var_os("JO_MOUSE_LOG").is_some() {
            eprintln!("move x={x:.1} y={y:.1} size_drag={}", self.size_drag.is_some());
        }
        let Some(d) = &self.size_drag else { return };
        let (col, idx, grab, base, moved) = (d.col, d.idx, d.grab, d.base, d.moved);
        if !moved {
            self.checkpoint();
            if let Some(d) = &mut self.size_drag {
                d.moved = true;
            }
        }
        if col {
            let w = (base + x - grab).max(9.0) / PX_PER_CHW;
            let w = (w * 100.0).round() / 100.0;
            self.sheet_mut().col_width.insert(idx, w);
            self.status = format!(
                "{}列の幅: {w}({:.0}px)",
                col_name(idx),
                w * PX_PER_CHW
            )
            .into();
        } else {
            let pt = (base + y - grab).max(6.0) * 15.0 / 24.0;
            let pt = (pt * 100.0).round() / 100.0;
            self.sheet_mut().row_height.insert(idx, pt);
            self.status = format!(
                "{}行の高さ: {pt}pt({:.0}px)",
                idx + 1,
                pt * 24.0 / 15.0
            )
            .into();
        }
        self.dirty = true;
    }

    /// マウスの左を押した(格子領域の座標)。押したセルが選択の始まり。
    /// メニューが出ていたら閉じる(項目の上の押下は stop_propagation でここに来ない)。
    fn mouse_down_at(&mut self, x: f32, y: f32, shift: bool, ctrl: bool, clicks: usize) {
        self.menu_at = None;
        self.pick = None;
        // mouse-up を取り逃していても、新しい押下で必ず仕切り直す(自癒)
        self.size_drag = None;
        self.drag = None;
        self.head_drag = None;
        self.shape_drag = None;
        if std::env::var_os("JO_MOUSE_LOG").is_some() {
            eprintln!(
                "down x={x:.1} y={y:.1} clicks={clicks} grip={:?}",
                self.size_grip_at(x, y)
            );
        }
        // 浮いている図形が最優先(セルの上に描かれているので)
        if let Some((i, (sx, sy), corner)) = self.shape_at(x, y) {
            self.commit();
            self.checkpoint();
            self.shape_sel = Some(i);
            self.shape_drag = Some((i, (x, y), if corner { (sx, sy) } else { (sx, sy) }, corner));
            self.status = if corner {
                "右下を引いて大きさを変えます".into()
            } else {
                "図形を選びました(ドラッグで移動 / 右下で大きさ / Del で削除)".into()
            };
            return;
        }
        self.shape_sel = None;
        // 見出しの境界の取っ手が最優先(セルの当たり判定より先に見る)。
        // **ダブルクリックの自動調整は撤去した**(2026-08-03 発注者報告)。
        // 押し直し・掴み直しは 400ms 以内なら click_count が 2,3,… と数えられる
        // (Wayland の仕様)ので、クリック数で分岐するとやり直しのドラッグを
        // 自動調整が横取りする — ドラッグは常にドラッグでなければならない
        let _ = clicks;
        if let Some((is_col, idx)) = self.size_grip_at(x, y) {
            self.commit();
            if std::env::var_os("JO_MOUSE_LOG").is_some() {
                eprintln!("grip: col={is_col} idx={idx} x={x:.0} y={y:.0}");
            }
            self.size_drag = Some(SizeDrag {
                col: is_col,
                idx,
                grab: if is_col { x } else { y },
                base: if is_col { self.col_px(idx) } else { self.row_px(idx) },
                moved: false,
            });
            return;
        }
        // 見出しのクリック = 列・行の選択(Excel の作法)。撫でれば複数列・行
        if y < ROW_H && x >= HEAD_W {
            if let Some(c) = self.col_at(x) {
                if !self.commit() {
                    return;
                }
                if shift {
                    // いまの選択の起点の列から伸ばす
                    let a = self.anchor.map(|p| p.col).unwrap_or(self.cursor.col);
                    self.select_cols(a, c);
                } else {
                    self.select_cols(c, c);
                    self.head_drag = Some((true, c));
                }
            }
            return;
        }
        if x < HEAD_W && y >= ROW_H {
            if let Some(r) = self.row_at(y) {
                if !self.commit() {
                    return;
                }
                if shift {
                    let a = self.anchor.map(|p| p.row).unwrap_or(self.cursor.row);
                    self.select_rows(a, r);
                } else {
                    self.select_rows(r, r);
                    self.head_drag = Some((false, r));
                }
            }
            return;
        }
        // 左上の角 = 使われている範囲の全選択(Ctrl+A と同じ)
        if x < HEAD_W && y < ROW_H {
            if !self.commit() {
                return;
            }
            let (rows, cols) = self.sheet().extent();
            if rows > 0 {
                self.anchor = Some(Pos::new(0, 0));
                self.cursor = Pos::new(rows - 1, cols.saturating_sub(1));
                self.sync_input();
                self.status = format!("A1:{} を選択しました", self.cursor.a1()).into();
            }
            return;
        }
        let Some(p) = self.cell_at(x, y) else { return };
        // Ctrl+クリックはリンクを開く(基幹網の外は既定のブラウザに任せる)
        if ctrl && !shift {
            if let Some(url) = self.sheet().links.get(&p).cloned() {
                let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
                self.status = format!("開きます: {url}").into();
                return;
            }
        }
        if !self.commit() {
            // 入力規則で戻された。移動すると打った文字が黙って消えるので留まる
            return;
        }
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
        if let Some((is_col, start)) = self.head_drag {
            // 見出しから始めた選択は、どこを通っても列・行の選択のまま
            if is_col {
                if let Some(c) = self.col_at(x) {
                    if self.cursor.col != c {
                        self.select_cols(start, c);
                    }
                }
            } else if let Some(r) = self.row_at(y) {
                if self.cursor.row != r {
                    self.select_rows(start, r);
                }
            }
            return;
        }
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
        if std::env::var_os("JO_MOUSE_LOG").is_some() {
            eprintln!(
                "up size_drag={} moved={:?}",
                self.size_drag.is_some(),
                self.size_drag.as_ref().map(|d| d.moved)
            );
        }
        if self.size_drag.take().is_some() {
            // 幅・高さの確定。status は size_drag_at が出している
            return;
        }
        if self.head_drag.take().is_some() {
            return; // 列・行の選択の確定。status は select_* が出している
        }
        if let Some((_, _, _, moved)) = self.shape_drag.take() {
            // 動かしていない(選んだだけ)なら、積んだ控えは戻す
            let _ = moved;
            return;
        }
        if self.drag.take().is_some() && self.anchor.is_some() {
            let (a, b) = self.sel_rect();
            self.status = format!("{}:{}", a.a1(), b.a1()).into();
        }
    }

    /// 右クリック。選択の中ならその選択への操作、外ならそのセルへ移ってから
    /// メニューを出す(Excel の作法)。
    fn right_click_at(&mut self, x: f32, y: f32) {
        // 見出しの右クリック = その列・行を選んでからメニュー(Excel の作法)。
        // 既に選択の中なら選び直さない(複数列への操作を保つ)
        if y < ROW_H && x >= HEAD_W {
            if let Some(c) = self.col_at(x) {
                let (a, b) = self.sel_rect();
                if !(self.anchor.is_some() && (a.col..=b.col).contains(&c)) {
                    if !self.commit() {
                        return;
                    }
                    self.select_cols(c, c);
                }
                self.menu_at = Some((x, y));
                self.menu_sub = None;
            }
            return;
        }
        if x < HEAD_W && y >= ROW_H {
            if let Some(r) = self.row_at(y) {
                let (a, b) = self.sel_rect();
                if !(self.anchor.is_some() && (a.row..=b.row).contains(&r)) {
                    if !self.commit() {
                        return;
                    }
                    self.select_rows(r, r);
                }
                self.menu_at = Some((x, y));
                self.menu_sub = None;
            }
            return;
        }
        if let Some(p) = self.cell_at(x, y) {
            let (a, b) = self.sel_rect();
            let inside = self.anchor.is_some()
                && (a.row..=b.row).contains(&p.row)
                && (a.col..=b.col).contains(&p.col);
            if !inside && p != self.cursor {
                if !self.commit() {
                    // 入力規則で戻された。移動せずメニューも出さない
                    return;
                }
                self.anchor = None;
                self.cursor = p;
                self.sync_input();
            }
        }
        self.menu_at = Some((x, y));
        self.menu_sub = None;
    }

    /// 範囲の見えている部分の px 矩形 (x0, y0, x1, y1)。全部画面の外なら None。
    fn range_px(&self, a: Pos, b: Pos) -> Option<(f32, f32, f32, f32)> {
        let (mut x0, mut x1) = (None, None);
        let mut x = HEAD_W;
        for c in grid_cols(self.frozen, self.view, COLS) {
            let w = self.col_px(c);
            if c >= a.col && c <= b.col {
                if x0.is_none() {
                    x0 = Some(x);
                }
                x1 = Some(x + w);
            }
            x += w;
        }
        let (mut y0, mut y1) = (None, None);
        let mut y = ROW_H;
        for r in self.visible_rows() {
            let h = self.row_px(r);
            if r >= a.row && r <= b.row {
                if y0.is_none() {
                    y0 = Some(y);
                }
                y1 = Some(y + h);
            }
            y += h;
        }
        Some((x0?, y0?, x1?, y1?))
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

    /// 形式を選択して貼り付け。mode: values / formulas / formats / transpose
    fn paste_special(&mut self, mode: &str, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) else {
            self.status = "貼り付けるものがありません".into();
            return;
        };
        if text.is_empty() {
            return;
        }
        // アプリ内のコピーか(系のクリップボードと控えの突き合わせ)
        let internal = matches!(&self.clip, Some((_, t)) if *t == text);
        let at = self.cursor;
        let n = match mode {
            "values" => {
                self.commit();
                self.checkpoint();
                if internal {
                    let cells = self.clip_cells.clone().unwrap_or_default();
                    paste_values_cells(&mut self.book.sheets[self.active], at, &cells)
                } else {
                    let grid = tsv_grid(&text);
                    paste_values_text(&mut self.book.sheets[self.active], at, &grid)
                }
            }
            "formulas" => {
                // 式を**ずらさずそのまま**貼る(普通の貼り付けはずらす方)
                self.commit();
                self.checkpoint();
                let grid = tsv_grid(&text);
                paste_grid(&mut self.book.sheets[self.active], at, &grid, None)
            }
            "formats" => {
                if !internal {
                    self.status =
                        "書式は他のアプリからは持って来られません(このアプリでコピーした範囲だけ)"
                            .into();
                    return;
                }
                self.commit();
                self.checkpoint();
                let cells = self.clip_cells.clone().unwrap_or_default();
                paste_formats(&mut self.book.sheets[self.active], at, &cells)
            }
            "transpose" => {
                // 行と列を入れ替えて、値を貼る(式は計算結果の値になる —
                // 転置で参照を正しく回すのは別の話なので、黙って混ぜない)
                self.commit();
                self.checkpoint();
                if internal {
                    let cells = transpose(&self.clip_cells.clone().unwrap_or_default());
                    paste_values_cells(&mut self.book.sheets[self.active], at, &cells)
                } else {
                    let grid = transpose(&tsv_grid(&text));
                    paste_values_text(&mut self.book.sheets[self.active], at, &grid)
                }
            }
            _ => return,
        };
        recalc(&mut self.book.sheets[self.active]);
        self.dirty = true;
        self.sync_input();
        self.status = match mode {
            "values" => format!("{n} セルに値だけを貼りました(書式は据え置き)"),
            "formulas" => format!("{n} セルに式をそのまま貼りました(参照はずらしていません)"),
            "formats" => format!("{n} セルに書式だけを写しました(中身は残っています)"),
            _ => format!("{n} セルを転置して貼りました(式は値になっています)"),
        }
        .into();
    }

    fn a_paste_values(&mut self, _: &ui::PasteValues, _: &mut Window, cx: &mut Context<Self>) {
        self.paste_special("values", cx);
        cx.notify();
    }

    /// 一覧から選んだものを適用する(pick_kind で意味が変わる)。
    fn apply_pick(&mut self, v: &str) {
        match self.pick_kind {
            "font" => {
                let name = v.to_string();
                self.fmt(move |f| f.font = Some(name.clone()));
                self.status = format!("書体を「{v}」にしました").into();
            }
            "size" => {
                if let Ok(pt) = v.parse::<f32>() {
                    self.fmt(move |f| f.size_c = Some((pt * 100.0) as u32));
                    self.status = format!("文字の大きさを {v}pt にしました").into();
                }
            }
            "symbol" => {
                // 打ちかけの続きに差し込む(セルを置き換えない)
                self.input.insert(v);
                self.dirty = true;
                self.status = format!("「{v}」を差し込みました(Enter で確定)").into();
            }
            "shape" => {
                let kind = match v {
                    "角丸四角形" => "roundRect",
                    "楕円" => "ellipse",
                    "右矢印" => "rightArrow",
                    "ひし形" => "diamond",
                    "直線" => "line",
                    _ => "rect",
                };
                self.checkpoint();
                let at = self.cursor;
                self.sheet_mut().shapes_new.push(sheet::model::SheetShape {
                    at,
                    width_px: 160.0,
                    height_px: 100.0,
                    kind: kind.into(),
                    fill: None,
                    line: Some("1B6E3C".into()),
                    ..Default::default()
                });
                self.shape_sel = Some(self.sheet().shapes_new.len() - 1);
                self.dirty = true;
                self.status = format!(
                    "{v}を {} に置きました(ドラッグで移動 / 右下で大きさ / Del で削除)",
                    at.a1()
                )
                .into();
            }
            _ => self.pick_value(v),
        }
        self.pick_kind = "value";
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
            "ps-values" => self.paste_special("values", cx),
            "ps-formulas" => self.paste_special("formulas", cx),
            "ps-formats" => self.paste_special("formats", cx),
            "ps-transpose" => self.paste_special("transpose", cx),
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
            "clear-fmt" => self.run_cmd("clear", cx),
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
            "filter-set" => self.run_cmd("setfilter", cx),
            "filter-clear" => self.run_cmd("clear-filter", cx),
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
            "freeze" => self.run_cmd("freeze", cx),
            // 数値の書式・関数はリボンと同じ配線を通す
            "comma" | "currency" | "percents" | "digit-inc" | "digit-dec"
            | "sum" | "average" | "count" | "max" | "min" => self.run_cmd(id, cx),
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
            "pastesp" => vec![
                ("ps-values", "値だけ(Ctrl+Shift+V)", true),
                ("ps-formulas", "式をそのまま(ずらさない)", true),
                ("ps-formats", "書式だけ", self.clip_cells.is_some()),
                ("ps-transpose", "行と列を入れ替えて(値を)", true),
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
        // 入力の板 → 一覧 → 子メニュー → 親メニュー → 書式の小窓 → コピーの破線、
        // の順で閉じる
        if self.prompt.take().is_some()
            || self.pick.take().is_some()
            || self.menu_sub.take().is_some()
            || self.menu_at.take().is_some()
            || self.fmt_panel.take().is_some()
            || self.clip_range.take().is_some()
            || self.shape_sel.take().is_some()
        {
            cx.notify();
        } else if self.editing() {
            // 打ちかけを捨てて、セルの保存内容に戻す
            // (入力規則で堰き止められたときの逃げ道でもある)
            self.sync_input();
            self.status = "打ちかけを取り消しました".into();
            cx.notify();
        }
    }

    /// 入力の板を確定する(Enter)。
    fn finish_prompt(&mut self, cx: &mut Context<Self>) {
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
            "py" => {
                let t = text.trim().to_string();
                if t.is_empty() {
                    // 空 Enter = .py ファイルを選ぶ
                    self.run_python_file_dialog(cx);
                } else if t == "@計算" || t == "@calc" {
                    self.run_py_calc(cx);
                } else if t == "@" || t == "@list" {
                    let names: Vec<&str> =
                        self.book.scripts.iter().map(|(n, _)| n.as_str()).collect();
                    self.status = if names.is_empty() {
                        "ブックに載せた Python はありません(@save 名前 で載せる)".into()
                    } else {
                        format!("ブックの Python: {}(@名前 で実行)", names.join(" / ")).into()
                    };
                } else if let Some(name) = t.strip_prefix("@save ") {
                    let name = name.trim().to_string();
                    if name.is_empty() {
                        self.status = "@save 名前 の形で".into();
                    } else {
                        self.store_python_dialog(name, cx);
                    }
                } else if let Some(name) = t.strip_prefix("@del ") {
                    let name = name.trim();
                    let before = self.book.scripts.len();
                    self.book.scripts.retain(|(n, _)| n != name);
                    if self.book.scripts.len() < before {
                        self.dirty = true;
                        self.status = format!("「{name}」をブックから外しました").into();
                    } else {
                        self.status = format!("「{name}」はありません").into();
                    }
                } else if let Some(rest) = t.strip_prefix('@') {
                    // ブックに載ったコード = 出所が自分とは限らない。必ず檻の中。
                    // 網は既定で閉じる。「@名前 net」と**その場で打ったときだけ**開く
                    // (許可はブックに保存されない — 毎回が明示の同意)
                    let (name, net) = match rest.trim().strip_suffix(" net") {
                        Some(n) => (n.trim(), true),
                        None => (rest.trim(), false),
                    };
                    match self.book.scripts.iter().find(|(n, _)| n == name) {
                        Some((_, code)) => {
                            let code = code.clone();
                            if net {
                                self.status =
                                    "網あり檻で実行します(ファイルは守られたまま)".into();
                            }
                            self.run_python_inner(code, true, net, cx);
                        }
                        None => {
                            self.status =
                                format!("「{name}」はありません(@list で一覧)").into();
                        }
                    }
                } else {
                    self.run_python(t, cx);
                }
            }
            "shape-text" => {
                let Some(i) = self.shape_sel else { return };
                if self.sheet().shapes_new.len() <= i {
                    return;
                }
                self.checkpoint();
                self.sheet_mut().shapes_new[i].text =
                    (!text.is_empty()).then(|| text.clone());
                self.dirty = true;
                self.status = if text.is_empty() {
                    "文字を消しました".into()
                } else {
                    "図形に文字を入れました(保存で xlsx に入ります)".into()
                };
            }
            "split-delim" => {
                let delim = if text.is_empty() { ",".to_string() } else { text };
                let (a, b) = self.sel_rect();
                let col = a.col;
                let targets: Vec<(Pos, String)> = (a.row..=b.row)
                    .filter_map(|r| {
                        let p = Pos::new(r, col);
                        match self.sheet().get(p).map(|c| &c.value) {
                            Some(sheet::Value::Text(t)) if t.contains(&delim) => {
                                Some((p, t.clone()))
                            }
                            _ => None,
                        }
                    })
                    .collect();
                if targets.is_empty() {
                    self.status = format!("「{delim}」で割れるセルが選択にありません").into();
                    return;
                }
                self.checkpoint();
                let mut n = 0usize;
                for (p, t) in targets {
                    for (k, part) in t.split(&delim).enumerate() {
                        let q = Pos::new(p.row, p.col + k as u32);
                        let fmt = self.sheet().get(q).map(|c| c.fmt.clone()).unwrap_or_default();
                        let mut cell = if part.starts_with('=') {
                            Cell {
                                formula: None,
                                value: sheet::Value::Text(part.to_string()),
                                fmt: Default::default(),
                            }
                        } else {
                            Cell::input(part)
                        };
                        cell.fmt = fmt;
                        self.sheet_mut().set(q, cell);
                        n += 1;
                    }
                }
                recalc(&mut self.book.sheets[self.active]);
                self.dirty = true;
                self.sync_input();
                self.status =
                    format!("{n} 欄に割りました(右のセルは上書き。Ctrl+Z で戻せます)").into();
            }
            "goal-target" => {
                // 「D6=765600」の形
                let Some((cell_s, val_s)) = text.split_once('=') else {
                    self.status = "「セル=目標値」の形で(例: D6=800000)".into();
                    return;
                };
                let (Some(p), Ok(v)) = (Pos::parse(cell_s), val_s.trim().parse::<f64>()) else {
                    self.status = "読めません(例: D6=800000)".into();
                    return;
                };
                self.goal = Some((p, v));
                self.prompt = Some(("goal-var", Editor::new("")));
            }
            "goal-var" => {
                let Some((target, goal)) = self.goal.take() else { return };
                let Some(var) = Pos::parse(&text) else {
                    self.status = "変えるセルが読めません(例: B2)".into();
                    return;
                };
                self.goal_seek(target, goal, var);
            }
            "find" => {
                if text.is_empty() {
                    self.status = "探す言葉を入れてください".into();
                    return;
                }
                self.find_term = Some(text);
                self.prompt = Some(("replace-with", Editor::new("")));
            }
            "replace-with" => {
                let Some(find) = self.find_term.take() else { return };
                if text.is_empty() {
                    // 検索だけ
                    self.find_next(&find);
                    return;
                }
                // 全て置き換え(シート全体。式の中も)
                let targets: Vec<(Pos, String)> = self
                    .sheet()
                    .cells
                    .iter()
                    .filter(|(_, c)| c.editable().contains(&find))
                    .map(|(p, c)| (*p, c.editable()))
                    .collect();
                if targets.is_empty() {
                    self.status = format!("「{find}」は見つかりません").into();
                    self.find_term = Some(find);
                    return;
                }
                self.checkpoint();
                let mut n = 0usize;
                for (p, src) in targets {
                    n += src.matches(find.as_str()).count();
                    let dst = src.replace(find.as_str(), &text);
                    let fmt = self.sheet().get(p).map(|c| c.fmt.clone()).unwrap_or_default();
                    let mut cell = Cell::input(&dst);
                    cell.fmt = fmt;
                    self.sheet_mut().set(p, cell);
                }
                recalc(&mut self.book.sheets[self.active]);
                self.dirty = true;
                self.sync_input();
                self.find_term = Some(find.clone());
                self.status =
                    format!("「{find}」→「{text}」: {n} カ所を置き換えました(Ctrl+Z で戻せます)")
                        .into();
            }
            "validation" => {
                let (a, b) = self.sel_rect();
                let overlap = |v: &sheet::model::Validation| {
                    let (ra, rb) = v.range;
                    ra.row <= b.row && rb.row >= a.row && ra.col <= b.col && rb.col >= a.col
                };
                if text.is_empty() {
                    // 空で Enter = この範囲の規則を外す
                    let n = self.sheet().validations.iter().filter(|v| overlap(v)).count();
                    if n == 0 {
                        self.status = "この範囲に入力規則はありません".into();
                        return;
                    }
                    self.checkpoint();
                    self.book.sheets[self.active].validations.retain(|v| !overlap(v));
                    self.dirty = true;
                    self.status = format!("{n} 本の入力規則を外しました").into();
                    return;
                }
                // = 始まりは範囲の参照、それ以外は候補の直書き(, 区切り)
                let formula = match text.strip_prefix('=') {
                    Some(r) => r.trim().to_string(),
                    None => format!("\"{}\"", text.replace('"', "")),
                };
                let v = sheet::model::Validation { range: (a, b), formula };
                let opts = v.options(self.sheet());
                if opts.is_empty() {
                    // 読めない規則を作らない(できないものを、できるように見せない)
                    self.status =
                        "候補が読めません(例: 甲,乙,丙 または =D2:D5)".into();
                    return;
                }
                self.checkpoint();
                // 選択に重なる古い規則は入れ替える(重ね掛けは分かりにくい)
                self.book.sheets[self.active].validations.retain(|v| !overlap(v));
                self.book.sheets[self.active].validations.push(v);
                self.dirty = true;
                self.status = format!(
                    "{}:{} に入力規則を付けました(候補: {})",
                    a.a1(),
                    b.a1(),
                    opts.join(" / ")
                )
                .into();
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
    fn fmt_panel_action(&mut self, id: &str, cx: &mut Context<Self>) {
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
            other => self.run_cmd(other, cx),
        }
    }

    /// 「ドロップダウンリストから選択」。同じ列に既にある値の一覧を出す
    /// (Excel の Alt+↓ と同じ発想。入力規則が無くても、列の値は候補になる)。
    fn open_pick_list(&mut self) {
        // 入力規則があればその候補(規則に書かれた順のまま)。無ければ同じ列の値
        let from_rule = self
            .sheet()
            .validation_at(self.cursor)
            .map(|v| v.options(self.sheet()))
            .filter(|o| !o.is_empty());
        let mut vals: Vec<String> = from_rule.clone().unwrap_or_default();
        if vals.is_empty() {
            let col = self.cursor.col;
            let (rows, _) = self.sheet().extent();
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
        }
        let total = vals.len();
        vals.truncate(16);
        if total > 16 {
            // 切ったことを黙らない
            self.status = format!("候補 {total} 件のうち先頭 16 件を出しています").into();
        }
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
        if !self.commit() {
            return; // 入力規則で戻された。切り替えると打った文字が消える
        }
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

    /// 数式バーの内容をセルへ。**入力規則(list)に合わない値は入れない**
    /// (Excel と同じ)。false を返したら呼び側は移動しないこと —
    /// 打った文字が黙って消える。Esc でセルの保存内容に戻せる。
    fn commit(&mut self) -> bool {
        let (cur, text) = (self.cursor, self.input.text().to_string());
        // 変わっていなければ何もしない(移動のたびに履歴が積まれるのを防ぐ)
        let now = self.sheet().get(cur).map(|c| c.editable()).unwrap_or_default();
        if now == text {
            return true;
        }
        // 空にするのは常に許す(allowBlank の既定)。式は結果が変わり得るので通す
        if !text.trim().is_empty() && !text.starts_with('=') {
            if let Some(v) = self.sheet().validation_at(cur) {
                let opts = v.options(self.sheet());
                // 候補が解決できない規則(別のシートへの参照等)では堰き止めない
                if !opts.is_empty() && !opts.iter().any(|o| *o == text.trim()) {
                    self.status = format!(
                        "「{}」は入力規則に合いません(候補: {} / Esc で戻す)",
                        text.trim(),
                        opts.join(" / ")
                    )
                    .into();
                    return false;
                }
            }
        }
        self.checkpoint();
        // **書式は据え置く。** 打ち直しただけで罫線や塗りが消えるのは帳票の事故
        let fmt = self.sheet().get(cur).map(|c| c.fmt.clone()).unwrap_or_default();
        let mut cell = Cell::input(&text);
        cell.fmt = fmt;
        self.sheet_mut().set(cur, cell);
        let s = self.sheet_mut();
        recalc(s);
        self.dirty = true;
        // 中身を変えたらコピーの破線は消す(Excel と同じ)
        self.clip_range = None;
        true
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
        if !self.commit() {
            return;
        }
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
        if !self.commit() {
            return; // 入力規則で戻された(status に候補が出ている)
        }
        let r = (self.cursor.row as i32 + dr).max(0) as u32;
        let c = (self.cursor.col as i32 + dc).max(0) as u32;
        self.cursor = Pos::new(r.min(9999), c.min(255));
        self.follow();
        self.sync_input();
    }

    /// 自分のロックを外す(閉じる・別のファイルへ移るとき)。
    fn release_lock(&mut self) {
        if let Some(lp) = self.my_lock.take() {
            let _ = std::fs::remove_file(lp);
        }
    }

    /// このファイルのロックを見て、先客が居れば警告、居なければ自分が取る。
    fn acquire_lock(&mut self, p: &std::path::Path) {
        self.release_lock();
        match foreign_lock(p) {
            Some(who) => {
                self.locked_by = Some(who);
                // ロックは取らない(先客の邪魔をしない)
            }
            None => {
                self.locked_by = None;
                let lp = lock_path_for(p);
                // LibreOffice と同じ気持ちの中身(名乗りだけ。厳密な書式は要らない)
                if std::fs::write(&lp, format!("{},;", lock_identity())).is_ok() {
                    self.my_lock = Some(lp);
                }
            }
        }
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
                self.clip_range = None;
                self.acquire_lock(&p);
                if let Some(who) = self.locked_by.clone() {
                    self.status = format!(
                        "{} — **{who} が開いています**。上書き保存はできません(名前を付けて保存へ)",
                        self.status
                    )
                    .into();
                }
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
        if let Some(i) = self.shape_sel.take() {
            if self.sheet().shapes_new.len() > i {
                self.checkpoint();
                self.sheet_mut().shapes_new.remove(i);
                self.dirty = true;
                self.status = "図形を削除しました(Ctrl+Z で戻せます)".into();
            }
            cx.notify();
            return;
        }
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
        // セルそのものも控える(形式を選択して貼り付けの材料)
        self.clip_cells = Some(
            (a.row..=b.row)
                .map(|r| {
                    (a.col..=b.col)
                        .map(|c| self.sheet().get(Pos::new(r, c)).cloned())
                        .collect()
                })
                .collect(),
        );
        self.clip_range = Some((self.active, a, b));
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
        // 切り取りの貼り付け先で式をずらさない(移動なので参照はそのまま)。
        // 形式を選択して貼り付けも切り取りでは使えない(Excel と同じ)
        self.clip = None;
        self.clip_cells = None;
        self.clip_range = None;
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
    fn a_doc_home(&mut self, _: &ui::DocHome, _: &mut Window, cx: &mut Context<Self>) {
        // Ctrl+Home は A1 へ(表計算の作法)
        self.anchor = None;
        if !self.commit() {
            cx.notify();
            return;
        }
        self.cursor = Pos::new(0, 0);
        self.follow();
        self.sync_input();
        cx.notify();
    }
    fn a_doc_end(&mut self, _: &ui::DocEnd, _: &mut Window, cx: &mut Context<Self>) {
        // Ctrl+End は使われている範囲の右下へ
        self.anchor = None;
        if !self.commit() {
            cx.notify();
            return;
        }
        let (rows, cols) = self.sheet().extent();
        if rows > 0 {
            self.cursor = Pos::new(rows - 1, cols.saturating_sub(1));
        }
        self.follow();
        self.sync_input();
        cx.notify();
    }
    fn a_page_up(&mut self, _: &ui::PageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_cursor(-(ROWS as i32 - 1), 0);
        cx.notify();
    }
    fn a_page_down(&mut self, _: &ui::PageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_cursor(ROWS as i32 - 1, 0);
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
            self.finish_prompt(cx);
        } else if let Some(i) = self.shape_sel {
            // 図形を選んで Enter = 中の文字を書く(テキストボックス)
            let cur = self
                .sheet()
                .shapes_new
                .get(i)
                .and_then(|sp| sp.text.clone())
                .unwrap_or_default();
            self.prompt = Some(("shape-text", Editor::new(&cur)));
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
        self.save(false, cx); cx.notify();
    }
    fn a_open(&mut self, _: &ui::Open, _: &mut Window, cx: &mut Context<Self>) {
        self.open_dialog(cx); cx.notify();
    }

    /// 開くファイルを選ぶ。**ダイアログは別の糸** — rfd は同期で、
    /// 主の糸で開くと画面ごと固まる(終了確認と同じ作法)。
    fn open_dialog(&mut self, cx: &mut Context<Self>) {
        let ask = cx.background_executor().spawn(async {
            rfd::FileDialog::new().add_filter("Excelブック", &["xlsx"]).pick_file()
        });
        cx.spawn(async move |this, cx| {
            let r = ask.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(p) = r {
                    this.open(p);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 終了の要求。書きかけが無ければ即終了、あれば確認を**別の糸**で出す。
    /// 確認のダイアログで主の糸を塞がない — 塞ぐと画面ごと固まり、
    /// GNOME に「応答なし」と判定される(踏んで直した)。
    /// 「はい」でも保存できなかった(保存の窓を閉じた等)なら終了しない —
    /// 書きかけを黙って捨てない。
    fn request_quit(&mut self, cx: &mut Context<Self>) {
        self.commit();
        // 確認は**実ファイルの未保存変更**にだけ出す(発注者の指示 2026-08-03)。
        // 名前も付けていない試し打ちにまで確認を出すと、確認が煩さで
        // 押し流される — 本当に守るべき場面で効かなくなる
        if !self.dirty || self.path.is_none() {
            self.release_lock();
            cx.quit();
            return;
        }
        let ask = cx.background_executor().spawn(async move {
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title("calc")
                .set_description("保存していない変更があります。保存して終了しますか?")
                .set_buttons(rfd::MessageButtons::YesNoCancel)
                .show()
        });
        cx.spawn(async move |this, cx| {
            let r = ask.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    // 保存先が未定なら別の糸で選ばせ、済んだときだけ終了する
                    rfd::MessageDialogResult::Yes => this.save(true, cx),
                    rfd::MessageDialogResult::No => {
                        this.release_lock();
                        cx.quit();
                    }
                    _ => this.status = "終了をやめました".into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn a_quit(&mut self, _: &ui::Quit, _: &mut Window, cx: &mut Context<Self>) {
        self.request_quit(cx);
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

    /// PDF に書き出す。保存先の選択は**別の糸**(rfd は同期)。
    fn save_pdf(&mut self, cx: &mut Context<Self>) {
        self.commit();
        let ask = cx.background_executor().spawn(async {
            rfd::FileDialog::new()
                .add_filter("PDF", &["pdf"])
                .set_file_name("帳票.pdf")
                .save_file()
        });
        cx.spawn(async move |this, cx| {
            let r = ask.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(p) = r {
                    this.write_pdf(&p);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// この格子座標に**このアプリで挿した図形**があるか(上に描かれた順 = 後勝ち)。
    /// 返すのは (番号, 図形の左上px, 右下隅の掴みか)。
    fn shape_at(&self, x: f32, y: f32) -> Option<(usize, (f32, f32), bool)> {
        for (i, sp) in self.sheet().shapes_new.iter().enumerate().rev() {
            let Some((sx, sy)) = self.cell_origin_px(sp.at) else { continue };
            let (w, h) = (sp.width_px, sp.height_px);
            if x >= sx && x <= sx + w && y >= sy && y <= sy + h {
                let corner = x >= sx + w - 12.0 && y >= sy + h - 12.0;
                return Some((i, (sx, sy), corner));
            }
        }
        None
    }

    /// 図形のドラッグ(移動 or 右下の掴みで大きさ変更)。
    fn shape_drag_at(&mut self, x: f32, y: f32) {
        let Some((i, (gx, gy), (ox, oy), resize)) = self.shape_drag else { return };
        if self.sheet().shapes_new.len() <= i {
            return;
        }
        if resize {
            let sp = &mut self.sheet_mut().shapes_new[i];
            sp.width_px = (x - ox).max(16.0);
            sp.height_px = (y - oy).max(16.0);
            let (w, h) = (sp.width_px, sp.height_px);
            self.dirty = true;
            self.status = format!("大きさ: {w:.0}×{h:.0}px").into();
        } else {
            // 移動: 掴んだときのずれを保って、左上の来るセルに留め直す
            let (nx, ny) = (ox + x - gx, oy + y - gy);
            if let (Some(c), Some(r)) = (self.col_at(nx.max(HEAD_W)), self.row_at(ny.max(ROW_H))) {
                let at = Pos::new(r, c);
                if self.sheet().shapes_new[i].at != at {
                    self.sheet_mut().shapes_new[i].at = at;
                    self.dirty = true;
                    self.status = format!("図形を {} に留めました", at.a1()).into();
                }
            }
        }
    }

    /// 「次を検索」。いまのセルの次(行→列の順)から探し、末尾まで行ったら
    /// 頭に戻る。式の中の文字も探す(editable = 打った通りの姿)。
    fn find_next(&mut self, term: &str) {
        let hits: Vec<Pos> = self
            .sheet()
            .cells
            .iter()
            .filter(|(_, c)| c.editable().contains(term) || c.value.display().contains(term))
            .map(|(p, _)| *p)
            .collect();
        if hits.is_empty() {
            self.status = format!("「{term}」は見つかりません").into();
            return;
        }
        let cur = self.cursor;
        let next = hits.iter().find(|p| **p > cur).copied().unwrap_or(hits[0]);
        self.anchor = None;
        self.cursor = next;
        self.follow();
        self.sync_input();
        self.status = format!(
            "「{term}」: {}({} カ所)。もう一度「置き換え」で次へ",
            next.a1(),
            hits.len()
        )
        .into();
        // 次回の板の初期値に残す(続けて探すのが検索の常)
        self.find_term = Some(term.to_string());
    }

    /// 選んだ範囲を matplotlib で棒グラフにして、シートに浮かべる。
    /// 1列目が項目名、残りの列が系列(先頭行が文字なら系列名)。
    /// Python は別の糸で回す(主の糸を塞がない — ダイアログと同じ作法)。
    fn insert_chart(&mut self, a: Pos, b: Pos, cx: &mut Context<Self>) {
        let sh = self.sheet();
        // 先頭行が見出しか(項目列以外に文字があるか)
        let header = (a.col + 1..=b.col).any(|c| {
            matches!(
                sh.get(Pos::new(a.row, c)).map(|x| &x.value),
                Some(sheet::Value::Text(_))
            )
        });
        let r0 = if header { a.row + 1 } else { a.row };
        let labels: Vec<String> = (r0..=b.row)
            .map(|r| {
                let v = sh.get(Pos::new(r, a.col)).map(|x| x.value.display()).unwrap_or_default();
                if v.is_empty() { (r + 1).to_string() } else { v }
            })
            .collect();
        let mut series = Vec::new();
        for c in a.col + 1..=b.col {
            let name = if header {
                sh.get(Pos::new(a.row, c)).map(|x| x.value.display()).unwrap_or_default()
            } else {
                col_name(c)
            };
            let values: Vec<f64> = (r0..=b.row)
                .map(|r| sh.get(Pos::new(r, c)).map(|x| x.value.as_number()).unwrap_or(0.0))
                .collect();
            series.push((name, values));
        }
        if labels.is_empty() || series.is_empty() {
            self.status = "グラフにする範囲を選んでください(1列目が項目名、2列目からが数)".into();
            return;
        }
        // JSON は手で組む(依存を増やさない。文字列は最小の逃がし)
        let esc = |t: &str| t.replace('\\', "\\\\").replace('"', "\\\"");
        let dir = std::env::temp_dir().join(format!("jo-chart-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let out = dir.join("chart.png");
        let font = kumihan::font::for_document(None)
            .ok()
            .map(|(fam, _)| fam.path.to_string_lossy().to_string())
            .unwrap_or_default();
        let mut json = String::from("{\"labels\":[");
        json.push_str(&labels.iter().map(|l| format!("\"{}\"", esc(l))).collect::<Vec<_>>().join(","));
        json.push_str("],\"series\":[");
        json.push_str(
            &series
                .iter()
                .map(|(n, vs)| {
                    format!(
                        "{{\"name\":\"{}\",\"values\":[{}]}}",
                        esc(n),
                        vs.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")
                    )
                })
                .collect::<Vec<_>>()
                .join(","),
        );
        json.push_str(&format!(
            "],\"font\":\"{}\",\"out\":\"{}\"}}",
            esc(&font),
            esc(&out.to_string_lossy())
        ));
        let at = Pos::new(a.row, b.col + 1);
        self.status = "グラフを描いています…".into();
        let task = cx.background_executor().spawn(async move {
            let json_path = dir.join("chart.json");
            let py_path = dir.join("chart.py");
            std::fs::write(&json_path, json).map_err(|e| e.to_string())?;
            std::fs::write(&py_path, CHART_PY).map_err(|e| e.to_string())?;
            let o = std::process::Command::new(find_python())
                .arg(&py_path)
                .arg(&json_path)
                .output()
                .map_err(|e| format!("Python が起動できません: {e}"))?;
            if !o.status.success() {
                let err = String::from_utf8_lossy(&o.stderr);
                let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("原因不明");
                return Err(if err.contains("No module named") {
                    format!("matplotlib がありません({last})。conda か pip で入れてください")
                } else {
                    format!("グラフが描けません: {last}")
                });
            }
            std::fs::read(&out).map_err(|e| e.to_string())
        });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok(data) => {
                        let (w, h) = image_px(&data).unwrap_or((640, 400));
                        this.checkpoint();
                        this.sheet_mut().images_new.push(sheet::model::SheetImage {
                            at,
                            width_px: w as f32,
                            height_px: h as f32,
                            data,
                        });
                        this.dirty = true;
                        this.status = format!(
                            "グラフを {} に置きました(保存で xlsx に入ります)",
                            at.a1()
                        )
                        .into();
                    }
                    Err(e) => this.status = e.into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Python in Calc(発注者提案 2026-08-04)。**コードは文書に入れない** —
    /// マクロと違い「開く=実行」の経路が無い。いまの表を一時 xlsx に写し、
    /// office_sheet(pysheet)で b(ブック)と s(いまのシート)を束縛して
    /// 利用者のコードを回し、保存されたものを読み戻して**1手として**適用する。
    fn run_python(&mut self, user_code: String, cx: &mut Context<Self>) {
        // 自分で打った/選んだコード: 檻はかけるが網は許す(自分の道具が
        // Web から取り込むのは普通の仕事。守るのは機械のファイルの方)
        self.run_python_inner(user_code, false, true, cx);
    }

    /// sandbox=true は**必ず**bubblewrap の檻の中で回す(ブックに載っていた
    /// コード = 他人のファイル由来かもしれないもの)。檻: ネット遮断・
    /// 実ファイルは読み取り専用・ホームは不可視・書けるのは交換用の一時領域だけ。
    /// 檻が無い機械では載せたコードは**実行しない**(そう言う)。
    /// 自分で打った/選んだコードも、檻があれば檻で回す(深層防御)。
    fn run_python_inner(
        &mut self,
        user_code: String,
        sandbox: bool,
        allow_net: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.commit() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("jo-py-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let in_x = dir.join("in.xlsx");
        let out_x = dir.join("out.xlsx");
        // 実行は複製の上(失敗しても表は無傷)。原本の部品も持ち越して写す
        let original: Option<std::io::Cursor<Vec<u8>>> = self
            .path
            .as_ref()
            .and_then(|old| std::fs::read(old).ok())
            .map(std::io::Cursor::new);
        let w = std::fs::File::create(&in_x)
            .map_err(|e| e.to_string())
            .and_then(|f| {
                sheet::xlsx::write_with(&self.book, original, std::io::BufWriter::new(f))
            });
        if let Err(e) = w {
            self.status = format!("Python に渡せません: {e}").into();
            return;
        }
        // office_sheet.so は実行ファイルの隣(HIKITSUGI の配り方を参照)
        let so_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_default();
        let so_dir2 = so_dir.clone();
        let script = format!(
            concat!(
                "import sys\n",
                "sys.path.insert(0, {so_dir:?})\n",
                "import office_sheet\n",
                "b = office_sheet.Book.open({in_x:?})\n",
                "s = b[{active}]\n",
                "# ---- 利用者のコード ----\n",
                "{code}\n",
                "# ----\n",
                "b.save({out_x:?})\n"
            ),
            so_dir = so_dir.to_string_lossy(),
            in_x = in_x.to_string_lossy(),
            active = self.active,
            out_x = out_x.to_string_lossy(),
            code = user_code
        );
        self.status = "Python を実行しています…".into();
        let task = cx.background_executor().spawn(async move {
            let py_path = dir.join("run.py");
            std::fs::write(&py_path, script).map_err(|e| e.to_string())?;
            let py = find_python();
            let have_bwrap = std::path::Path::new("/usr/bin/bwrap").exists();
            if sandbox && !have_bwrap {
                return Err(
                    "檻(bubblewrap)がありません。ブックに載ったコードは檻の外では\
実行しません(apt install bubblewrap)"
                        .to_string(),
                );
            }
            let _ = allow_net;
            let mut cmd = if have_bwrap {
                // 檻: / は読み取り専用、ホームは空、書けるのは作業場だけ、ネット無し
                let venv = std::fs::canonicalize(".venv").unwrap_or_default();
                let mut c = std::process::Command::new("/usr/bin/bwrap");
                c.args(["--ro-bind", "/", "/", "--tmpfs", "/home", "--tmpfs", "/tmp"]);
                if venv.exists() {
                    c.arg("--ro-bind").arg(&venv).arg(&venv);
                }
                if so_dir2.exists() {
                    c.arg("--ro-bind").arg(&so_dir2).arg(&so_dir2);
                }
                c.arg("--bind").arg(&dir).arg(&dir);
                if !allow_net {
                    c.arg("--unshare-net");
                }
                c.args([
                    "--dev",
                    "/dev",
                    "--proc",
                    "/proc",
                    "--die-with-parent",
                    "--new-session",
                    "--setenv",
                    "HOME",
                    "/tmp",
                    "--",
                ]);
                c.arg(&py);
                c
            } else {
                std::process::Command::new(&py)
            };
            let o = cmd
                .arg(&py_path)
                .output()
                .map_err(|e| format!("Python が起動できません: {e}"))?;
            let out = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !o.status.success() {
                let err = String::from_utf8_lossy(&o.stderr);
                let last = err
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("原因不明")
                    .to_string();
                return Err(if err.contains("No module named 'office_sheet'") {
                    "office_sheet.so がありません(cargo build -p pysheet --release \
--features extension-module して、liboffice_sheet.so を office_sheet.so の名で \
calc の隣に置いてください)"
                        .to_string()
                } else {
                    last
                });
            }
            std::fs::read(&out_x)
                .map_err(|e| format!("結果が読めません: {e}"))
                .map(|bytes| (bytes, out))
        });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok((bytes, out)) => {
                        match sheet::xlsx::read(std::io::Cursor::new(bytes)) {
                            Ok((mut book, rep)) => {
                                for sh in &mut book.sheets {
                                    recalc(sh);
                                }
                                this.checkpoint_book();
                                this.book = book;
                                if this.active >= this.book.sheets.len() {
                                    this.active = 0;
                                }
                                this.sheet_ui.clear();
                                this.dirty = true;
                                this.sync_input();
                                this.notes = rep
                                    .unsupported
                                    .iter()
                                    .map(|(n, c)| SharedString::from(format!("{n} × {c}")))
                                    .collect();
                                this.status = if out.is_empty() {
                                    "Python を実行しました(Ctrl+Z で1手で戻せます)".into()
                                } else {
                                    let last =
                                        out.lines().last().unwrap_or_default().to_string();
                                    format!(
                                        "Python: {last}(出力{}行。変更は Ctrl+Z で戻せます)",
                                        out.lines().count()
                                    )
                                    .into()
                                };
                            }
                            Err(e) => {
                                this.status = format!("結果が読めません: {e}").into();
                            }
                        }
                    }
                    Err(e) => this.status = format!("Python: {e}").into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// =PY(…) のセルを全部、檻の中で計算する(@計算)。
    /// 関数の定義はブックの「関数」で始まる名前のスクリプトから読む。
    /// **これ以外の道で PY セルが計算されることはない**(開く=実行を持たない)。
    fn run_py_calc(&mut self, cx: &mut Context<Self>) {
        if !self.commit() {
            return;
        }
        let defs: String = self
            .book
            .scripts
            .iter()
            .filter(|(n, _)| n.starts_with("関数"))
            .map(|(_, c)| c.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let mut per_sheet: Vec<(usize, Vec<(String, String, Vec<sheet::calc::PyArg>)>)> =
            Vec::new();
        for (i, sh) in self.book.sheets.iter().enumerate() {
            let mut calls = Vec::new();
            for (p, c) in &sh.cells {
                let Some(f) = &c.formula else { continue };
                if !sheet::calc::is_py_formula(f) {
                    continue;
                }
                if let Some((name, args)) = sheet::calc::eval_py_call(sh, f) {
                    calls.push((p.a1(), name, args));
                }
            }
            if !calls.is_empty() {
                per_sheet.push((i, calls));
            }
        }
        if per_sheet.is_empty() {
            self.status = "=PY(\"関数名\", 引数…) のセルがありません".into();
            return;
        }
        if defs.trim().is_empty() {
            self.status =
                "関数の定義がありません(@save 関数 で def の入った .py をブックに載せる)"
                    .into();
            return;
        }
        let dir = std::env::temp_dir().join(format!("jo-udf-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut scripts = Vec::new();
        for (i, calls) in &per_sheet {
            let out = dir.join(format!("out{i}.txt"));
            scripts.push((
                *i,
                dir.join(format!("udf{i}.py")),
                out.clone(),
                build_udf_script(&defs, calls, &out),
            ));
        }
        self.status = "PY を計算しています…(檻の中)".into();
        let task = cx.background_executor().spawn(async move {
            if !std::path::Path::new("/usr/bin/bwrap").exists() {
                return Err(
                    "檻(bubblewrap)がありません。ブックの関数は檻の外では計算しません"
                        .to_string(),
                );
            }
            let py = find_python();
            let venv = std::fs::canonicalize(".venv").unwrap_or_default();
            let mut results = Vec::new();
            for (i, py_path, out_path, script) in scripts {
                std::fs::write(&py_path, script).map_err(|e| e.to_string())?;
                let mut c = std::process::Command::new("/usr/bin/bwrap");
                c.args(["--ro-bind", "/", "/", "--tmpfs", "/home", "--tmpfs", "/tmp"]);
                if venv.exists() {
                    c.arg("--ro-bind").arg(&venv).arg(&venv);
                }
                c.arg("--bind").arg(&dir).arg(&dir);
                c.args([
                    "--unshare-net",
                    "--dev",
                    "/dev",
                    "--proc",
                    "/proc",
                    "--die-with-parent",
                    "--new-session",
                    "--setenv",
                    "HOME",
                    "/tmp",
                    "--",
                ]);
                let o = c
                    .arg(&py)
                    .arg(&py_path)
                    .output()
                    .map_err(|e| format!("Python が起動できません: {e}"))?;
                if !o.status.success() {
                    let err = String::from_utf8_lossy(&o.stderr);
                    let last = err
                        .lines()
                        .rev()
                        .find(|l| !l.trim().is_empty())
                        .unwrap_or("原因不明");
                    return Err(format!("PY の計算に失敗: {last}"));
                }
                let raw = std::fs::read_to_string(&out_path).unwrap_or_default();
                results.push((i, raw));
            }
            Ok(results)
        });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok(outs) => {
                        this.checkpoint_book();
                        let (mut total, mut conflicts) = (0usize, 0usize);
                        for (i, raw) in outs {
                            let results = parse_udf_output(&raw);
                            let prev: std::collections::HashMap<Pos, (u32, u32)> = this
                                .py_spills
                                .iter()
                                .filter(|((si, _), _)| *si == i)
                                .map(|((_, p), d)| (*p, *d))
                                .collect();
                            let (spills, n, c) =
                                apply_py_results(&mut this.book.sheets[i], &results, &prev);
                            this.py_spills.retain(|(si, _), _| *si != i);
                            for (p, d) in spills {
                                this.py_spills.insert((i, p), d);
                            }
                            recalc(&mut this.book.sheets[i]);
                            total += n;
                            conflicts += c;
                        }
                        this.dirty = true;
                        this.sync_input();
                        this.status = if conflicts > 0 {
                            format!(
                                "PY: {total} セルを計算、{conflicts} 件は #SPILL!(展開先に他のデータ)。Ctrl+Z で戻せます"
                            )
                            .into()
                        } else {
                            format!("PY: {total} セルを計算しました(Ctrl+Z で戻せます)").into()
                        };
                    }
                    Err(e) => this.status = e.into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// .py を選んで**ブックに載せる**(実行はしない)。載せたコードは
    /// 保存で xlsx に入り、帳票と一緒に旅をする。実行は @名前 で、必ず檻の中。
    fn store_python_dialog(&mut self, name: String, cx: &mut Context<Self>) {
        let ask = cx.background_executor().spawn(async {
            rfd::FileDialog::new()
                .add_filter("Python", &["py"])
                .pick_file()
        });
        cx.spawn(async move |this, cx| {
            let r = ask.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(p) = r {
                    match std::fs::read_to_string(&p) {
                        Ok(code) => {
                            this.book.scripts.retain(|(n, _)| *n != name);
                            this.book.scripts.push((name.clone(), code));
                            this.dirty = true;
                            this.status = format!(
                                "「{name}」をブックに載せました(保存で xlsx に入る。@{name} で実行)"
                            )
                            .into();
                        }
                        Err(e) => this.status = format!("読めません: {e}").into(),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// .py ファイルを選んで回す(コードは利用者のファイルにある —
    /// 文書には決して入らない)。
    fn run_python_file_dialog(&mut self, cx: &mut Context<Self>) {
        let ask = cx.background_executor().spawn(async {
            rfd::FileDialog::new()
                .add_filter("Python", &["py"])
                .pick_file()
        });
        cx.spawn(async move |this, cx| {
            let r = ask.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(p) = r {
                    match std::fs::read_to_string(&p) {
                        Ok(code) => this.run_python(code, cx),
                        Err(e) => this.status = format!("読めません: {e}").into(),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// CSV/TSV を選んで、いまのセルから値として流し込む(裏方 Python)。
    /// 文字コード(CP932 含む)と区切りは Python 側で判定する。
    fn import_text_dialog(&mut self, cx: &mut Context<Self>) {
        let ask = cx.background_executor().spawn(async {
            let p = rfd::FileDialog::new()
                .add_filter("テキストのデータ", &["csv", "tsv", "txt"])
                .pick_file()?;
            let dir = std::env::temp_dir().join(format!("jo-csv-{}", std::process::id()));
            let _ = std::fs::create_dir_all(&dir);
            // csv.py という名前は標準ライブラリの csv を隠してしまう(踏んだ)
            let py_path = dir.join("jo_csv.py");
            if std::fs::write(&py_path, CSV_PY).is_err() {
                return Some(Err("一時ファイルが書けません".to_string()));
            }
            let o = std::process::Command::new(find_python())
                .arg(&py_path)
                .arg(&p)
                .output();
            Some(match o {
                Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).to_string()),
                Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
                Err(e) => Err(format!("Python が起動できません: {e}")),
            })
        });
        cx.spawn(async move |this, cx| {
            let r = ask.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    None => {}
                    Some(Ok(data)) => {
                        let grid: Vec<Vec<String>> = data
                            .split('\u{1e}')
                            .map(|row| row.split('\u{1f}').map(|f| f.to_string()).collect())
                            .collect();
                        let n_rows = grid.len();
                        this.checkpoint();
                        let at = this.cursor;
                        let n = paste_values_text(&mut this.book.sheets[this.active], at, &grid);
                        recalc(&mut this.book.sheets[this.active]);
                        this.dirty = true;
                        this.sync_input();
                        this.status = format!(
                            "{n_rows} 行 {n} 欄を {} から流し込みました(値として)",
                            at.a1()
                        )
                        .into();
                    }
                    Some(Err(e)) => this.status = format!("読み込めません: {e}").into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// ゴールシーク。変えるセルの値を割線法で探す(表の複製の上で試す)。
    fn goal_seek(&mut self, target: Pos, goal: f64, var: Pos) {
        let base = self.sheet().clone();
        if base.get(target).and_then(|c| c.formula.as_ref()).is_none() {
            self.status = format!("{} は式のセルではありません", target.a1()).into();
            return;
        }
        let found = solve_goal(&base, target, goal, var);
        match found {
            Some(x) => {
                let x = (x * 1e9).round() / 1e9;
                self.checkpoint();
                let fmt = self.sheet().get(var).map(|c| c.fmt.clone()).unwrap_or_default();
                let mut cell = Cell::input(&format!("{x}"));
                cell.fmt = fmt;
                self.sheet_mut().set(var, cell);
                recalc(&mut self.book.sheets[self.active]);
                self.dirty = true;
                self.sync_input();
                self.status = format!(
                    "{} = {x} で {} が {goal} になります(Ctrl+Z で戻せます)",
                    var.a1(),
                    target.a1()
                )
                .into();
            }
            None => {
                self.status = format!(
                    "見つかりません({} が {} に効いていないかもしれません)",
                    var.a1(),
                    target.a1()
                )
                .into();
            }
        }
    }

    /// 画像ファイルを選んで、いまのセルに浮かべる(選択は別の糸)。
    fn insert_image_dialog(&mut self, cx: &mut Context<Self>) {
        let ask = cx.background_executor().spawn(async {
            rfd::FileDialog::new()
                .add_filter("画像", &["png", "jpg", "jpeg"])
                .pick_file()
        });
        cx.spawn(async move |this, cx| {
            let r = ask.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(p) = r {
                    match std::fs::read(&p) {
                        Ok(data) => match image_px(&data) {
                            Some((w, h)) => {
                                this.checkpoint();
                                let at = this.cursor;
                                this.sheet_mut().images_new.push(sheet::model::SheetImage {
                                    at,
                                    width_px: w as f32,
                                    height_px: h as f32,
                                    data,
                                });
                                this.dirty = true;
                                this.status = format!(
                                    "画像を {} に置きました(保存で xlsx に入ります)",
                                    at.a1()
                                )
                                .into();
                            }
                            None => {
                                this.status =
                                    "この画像は読めません(PNG か JPEG を選んでください)".into();
                            }
                        },
                        Err(e) => this.status = format!("読めません: {e}").into(),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn write_pdf(&mut self, p: &std::path::Path) {
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
        // 帳票の印刷設定(pageSetup / pageMargins / Print_Area)に従う。
        // 効かせたものは status に言う(黙って既定で出さない)
        let sh = &self.book.sheets[self.active];
        let mut paper = paper::Paper::default();
        let mut desc: Vec<String> = Vec::new();
        if let Some(code) = sh.paper_size {
            match paper_mm(code) {
                Some((w, h, name)) => {
                    paper.width_mm = w;
                    paper.height_mm = h;
                    if code != 9 {
                        desc.push(name.into());
                    }
                }
                None => desc.push(format!("用紙コード{code}は未対応・A4で出します")),
            }
        }
        if sh.landscape {
            std::mem::swap(&mut paper.width_mm, &mut paper.height_mm);
            desc.push("横向き".into());
        }
        let areas = sh.print_areas.clone();
        let setup = paper::grid::PrintSetup {
            area: areas.first().copied(),
            margins_mm: sh.margins_mm,
        };
        if let Some((a, b)) = setup.area {
            desc.push(format!("印刷範囲 {}:{}", a.a1(), b.a1()));
        }
        if areas.len() > 1 {
            desc.push(format!("残り {} 域の印刷範囲はまだ出せません", areas.len() - 1));
        }
        let mut clipped = 0u32;
        let r = kumihan::atomic::save(p, |f| {
            paper::grid::sheet_to_pdf(
                &self.book.sheets[self.active],
                &data,
                paper,
                &setup,
                std::io::BufWriter::new(f),
            )
            .map(|n| clipped = n)
        });
        self.status = match r {
            // 紙に入り切らなかった列は黙らない
            Ok(_) => format!(
                "PDF にしました — {}{}{}{}",
                p.file_name().unwrap_or_default().to_string_lossy(),
                if desc.is_empty() {
                    String::new()
                } else {
                    format!("({})", desc.join("・"))
                },
                if exact { "" } else { " ※代替フォント" },
                if clipped > 0 {
                    format!("(右の {clipped} 列は紙に入り切らず切れています)")
                } else {
                    String::new()
                }
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
    #[allow(dead_code)] // wiring_tests(cfg(test))が使う
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
        "data-validation", "condformat", "defname",
        "pageorient", "pagesize", "pagemargins", "printarea",
        "inschart", "insimage", "inshyperlink", "replace",
        "changecase", "format", "cell-format", "fontname", "fontsize",
        "fn-datetime", "fn-lookup", "fn-financial", "fn-more",
        "scale", "pagebreak", "printtitles", "print-gridlines", "print-headings",
        "data-from-text", "text-column", "goal-seek", "data-external-links",
        "insshape", "instext", "inssparkline", "python", "addcomment",
        "trace-prec", "trace-dep", "remove-arrows", "insrecommend",
        "instable", "table-tpl", "inssymbol",
    ];

    fn run_cmd(&mut self, id: &str, cx: &mut Context<Self>) {
        match id {
            "open" => self.open_dialog(cx),
            "save" => self.save(false, cx),
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
            "pdf" => self.save_pdf(cx),
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
            // 印刷の設定。モデルに置き、保存で原文へ織り込み、PDF が従う。
            // どれもシートの控えで1手戻せる
            "pageorient" => {
                self.commit();
                self.checkpoint();
                let sh = self.sheet_mut();
                sh.landscape = !sh.landscape;
                let landscape = sh.landscape;
                self.dirty = true;
                self.status = format!(
                    "印刷の向き: {}(PDF と保存に効きます)",
                    if landscape { "横" } else { "縦" }
                )
                .into();
            }
            "pagesize" => {
                self.commit();
                self.checkpoint();
                // A4 → A3 → B4 → B5 → A5 → A4 の順で回す
                const CYCLE: [u32; 5] = [9, 8, 12, 13, 11];
                let sh = self.sheet_mut();
                let now = sh.paper_size.unwrap_or(9);
                let i = CYCLE.iter().position(|c| *c == now).unwrap_or(0);
                let next = CYCLE[(i + 1) % CYCLE.len()];
                sh.paper_size = Some(next);
                self.dirty = true;
                let name = paper_mm(next).map(|(_, _, n)| n).unwrap_or("A4");
                self.status = format!("用紙: {name}(B は JIS)").into();
            }
            "pagemargins" => {
                self.commit();
                self.checkpoint();
                // 既定(20mm)→ 狭い(10mm)→ 広い(30mm)→ 既定
                let sh = self.sheet_mut();
                let (next, label) = match sh.margins_mm {
                    None => (Some((10.0, 10.0, 10.0, 10.0)), "狭い(10mm)"),
                    Some((l, _, _, _)) if l < 15.0 => {
                        (Some((30.0, 30.0, 30.0, 30.0)), "広い(30mm)")
                    }
                    Some(_) => (None, "既定(20mm)"),
                };
                sh.margins_mm = next;
                self.dirty = true;
                self.status = format!("印刷の余白: {label}").into();
            }
            "printarea" => {
                self.commit();
                if self.anchor.is_some() {
                    self.checkpoint();
                    let range = self.sel_rect();
                    self.sheet_mut().print_areas = vec![range];
                    self.dirty = true;
                    self.status = format!(
                        "印刷範囲: {}:{}(もう一度押すと解除)",
                        range.0.a1(),
                        range.1.a1()
                    )
                    .into();
                } else if !self.sheet().print_areas.is_empty() {
                    self.checkpoint();
                    self.sheet_mut().print_areas.clear();
                    self.dirty = true;
                    self.status = "印刷範囲を解きました(全域を印刷します)".into();
                } else {
                    self.status =
                        "印刷範囲にする範囲を Shift+矢印かドラッグで選んでください".into();
                }
            }
            // 大文字小文字。選択の英字に小文字があれば大文字へ、無ければ小文字へ
            "changecase" => {
                self.commit();
                self.checkpoint();
                let (a, b) = self.sel_rect();
                let mut has_lower = false;
                for r in a.row..=b.row {
                    for c in a.col..=b.col {
                        if let Some(cell) = self.sheet().get(Pos::new(r, c)) {
                            if let sheet::Value::Text(t) = &cell.value {
                                if t.chars().any(|ch| ch.is_ascii_lowercase()) {
                                    has_lower = true;
                                }
                            }
                        }
                    }
                }
                let mut n = 0usize;
                for r in a.row..=b.row {
                    for c in a.col..=b.col {
                        let p = Pos::new(r, c);
                        let Some(cell) = self.sheet().get(p).cloned() else { continue };
                        let sheet::Value::Text(t) = &cell.value else { continue };
                        if !t.chars().any(|ch| ch.is_ascii_alphabetic()) {
                            continue;
                        }
                        let new_t = if has_lower { t.to_uppercase() } else { t.to_lowercase() };
                        if new_t != *t {
                            let mut cell = cell;
                            cell.value = sheet::Value::Text(new_t);
                            self.sheet_mut().set(p, cell);
                            n += 1;
                        }
                    }
                }
                if n == 0 {
                    self.undo_stack.pop();
                    self.status = "選択の中に英字がありません".into();
                } else {
                    self.dirty = true;
                    self.sync_input();
                    self.status = format!(
                        "{n} セルを{}にしました(もう一度で逆)",
                        if has_lower { "大文字" } else { "小文字" }
                    )
                    .into();
                }
            }
            // 数値の書式・セルのスタイル: 書式の小窓(道具箱)を開く
            "format" | "cell-format" => {
                let at = self
                    .cell_origin_px(self.cursor)
                    .map(|(x, y)| (x + 16.0, y + 16.0))
                    .unwrap_or((HEAD_W + 24.0, ROW_H + 24.0));
                self.fmt_panel = Some(at);
            }
            // 書体と大きさ: 一覧から選ぶ(日本語が組める書体だけ出す)
            "fontname" => {
                let vals: Vec<String> = kumihan::font::list()
                    .iter()
                    .filter(|f| f.japanese)
                    .map(|f| f.name.clone())
                    .collect();
                if vals.is_empty() {
                    self.status = "日本語の書体が見つかりません".into();
                } else {
                    let at = self
                        .cell_origin_px(self.cursor)
                        .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                        .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                    self.pick_kind = "font";
                    self.pick = Some((vals.into_iter().take(16).collect(), at));
                }
            }
            "fontsize" => {
                let at = self
                    .cell_origin_px(self.cursor)
                    .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                    .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                self.pick_kind = "size";
                self.pick = Some((
                    ["8", "9", "10", "11", "12", "14", "16", "18", "20", "24", "28", "36", "48"]
                        .iter()
                        .map(|v| v.to_string())
                        .collect(),
                    at,
                ));
            }
            // データタブ: Python 裏方と道具
            "data-from-text" => {
                self.commit();
                self.import_text_dialog(cx);
            }
            "python" => {
                self.commit();
                self.prompt = Some(("py", Editor::new("")));
            }
            // 参照のトレース。矢印の代わりに**セルを光らせる**(見え方だけ)
            "trace-prec" => {
                self.commit();
                let deps = self
                    .sheet()
                    .get(self.cursor)
                    .and_then(|c| c.formula.as_ref())
                    .map(|f| sheet::calc::deps(f))
                    .unwrap_or_default();
                if deps.is_empty() {
                    self.status = "このセルの式は他のセルを参照していません".into();
                } else {
                    self.status = format!(
                        "{} の参照元 {} セルを光らせました(トレース矢印の削除で消す)",
                        self.cursor.a1(),
                        deps.len()
                    )
                    .into();
                    self.trace = deps.into_iter().map(|p| (p, true)).collect();
                }
            }
            "trace-dep" => {
                self.commit();
                let me = self.cursor;
                let dependents: Vec<Pos> = self
                    .sheet()
                    .cells
                    .iter()
                    .filter(|(_, c)| {
                        c.formula
                            .as_ref()
                            .is_some_and(|f| sheet::calc::deps(f).contains(&me))
                    })
                    .map(|(p, _)| *p)
                    .collect();
                if dependents.is_empty() {
                    self.status = format!("{} を参照している式はありません", me.a1()).into();
                } else {
                    self.status = format!(
                        "{} の参照先 {} セルを光らせました(トレース矢印の削除で消す)",
                        me.a1(),
                        dependents.len()
                    )
                    .into();
                    self.trace = dependents.into_iter().map(|p| (p, false)).collect();
                }
            }
            "remove-arrows" => {
                self.trace.clear();
                self.status = "トレースを消しました".into();
            }
            // 推奨チャート = いまの一手(棒グラフ)をそのまま勧める
            "insrecommend" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status =
                        "グラフにする範囲を選んでください(1列目が項目名、2列目からが数)"
                            .into();
                } else {
                    let (a, b) = self.sel_rect();
                    self.insert_chart(a, b, cx);
                }
            }
            // 表の挿入 = 選択に表の書式(見出しの帯+縞々+外枠)を掛ける
            "instable" | "table-tpl" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status = "表にする範囲を選んでください".into();
                } else {
                    self.checkpoint();
                    let (a, b) = self.sel_rect();
                    for r in a.row..=b.row {
                        for c in a.col..=b.col {
                            let p = Pos::new(r, c);
                            let mut cell = self.sheet().get(p).cloned().unwrap_or_default();
                            if r == a.row {
                                cell.fmt.bold = true;
                                cell.fmt.fill = Some("D5E8DC".into());
                            } else if (r - a.row) % 2 == 0 {
                                cell.fmt.fill = Some("F1F6F3".into());
                            }
                            if r == a.row {
                                cell.fmt.borders.top = true;
                            }
                            if r == b.row {
                                cell.fmt.borders.bottom = true;
                            }
                            if c == a.col {
                                cell.fmt.borders.left = true;
                            }
                            if c == b.col {
                                cell.fmt.borders.right = true;
                            }
                            self.book.sheets[self.active].set(p, cell);
                        }
                    }
                    self.dirty = true;
                    self.status = format!(
                        "{}:{} に表の書式を掛けました(見出しの帯と縞々。Ctrl+Z で戻せます)",
                        a.a1(),
                        b.a1()
                    )
                    .into();
                }
            }
            // 記号を挿入: 一覧から選んで**数式バーへ**差し込む(セルは置き換えない)
            "inssymbol" => {
                let at = self
                    .cell_origin_px(self.cursor)
                    .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                    .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                self.pick_kind = "symbol";
                self.pick = Some((
                    ["〒", "℡", "№", "㈱", "〆", "※", "→", "←", "↑", "↓",
                     "○", "●", "◎", "△", "▲", "×", "☑", "☐", "✓", "①", "②", "③"]
                        .iter()
                        .map(|v| v.to_string())
                        .collect(),
                    at,
                ));
            }
            "addcomment" => {
                self.commit();
                let cur = self.sheet().comments.get(&self.cursor).cloned().unwrap_or_default();
                self.prompt = Some(("comment", Editor::new(&cur)));
            }
            "text-column" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status =
                        "割りたいセルを選んでください(選択した列の文字を右へ割ります)".into();
                } else {
                    self.prompt = Some(("split-delim", Editor::new("")));
                }
            }
            "goal-seek" => {
                self.commit();
                // 目標セルの初期値はいまのセル(式のセルの上で押すのが自然)
                let init = if self.sheet().get(self.cursor).and_then(|c| c.formula.as_ref()).is_some()
                {
                    format!("{}=", self.cursor.a1())
                } else {
                    String::new()
                };
                self.goal = None;
                self.prompt = Some(("goal-target", Editor::new(&init)));
            }
            "data-external-links" => {
                // 他のブックを**値として**取り込む(リンクは張らない —
                // リンク切れの帳票を作らない。SEKKEI の分業どおり)
                self.commit();
                let ask = cx.background_executor().spawn(async {
                    let p = rfd::FileDialog::new()
                        .add_filter("Excelブック", &["xlsx"])
                        .pick_file()?;
                    Some(
                        std::fs::File::open(&p)
                            .map_err(|e| e.to_string())
                            .and_then(sheet::xlsx::read)
                            .map(|(b, _)| (p, b)),
                    )
                });
                cx.spawn(async move |this, cx| {
                    let r = ask.await;
                    let _ = this.update(cx, |this, cx| {
                        match r {
                            None => {}
                            Some(Ok((p, mut other))) => {
                                this.checkpoint();
                                let mut n = 0usize;
                                for mut sh in other.sheets.drain(..) {
                                    recalc(&mut sh);
                                    // 式は計算結果の値に(他所の参照を持ち込まない)
                                    for c in sh.cells.values_mut() {
                                        c.formula = None;
                                    }
                                    sh.name = format!(
                                        "{}({})",
                                        sh.name,
                                        p.file_stem().unwrap_or_default().to_string_lossy()
                                    );
                                    while this.book.sheets.iter().any(|x| x.name == sh.name) {
                                        sh.name.push('+');
                                    }
                                    this.book.sheets.push(sh);
                                    n += 1;
                                }
                                this.dirty = true;
                                this.status = format!(
                                    "{n} シートを値として取り込みました(リンクは張りません)"
                                )
                                .into();
                            }
                            Some(Err(e)) => this.status = format!("取り込めません: {e}").into(),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            // 拡大縮小印刷: 100→90→80→70→50→100
            "scale" => {
                self.commit();
                self.checkpoint();
                let sh = self.sheet_mut();
                let next = match sh.print_scale.unwrap_or(100) {
                    100 => 90,
                    90 => 80,
                    80 => 70,
                    70 => 50,
                    _ => 100,
                };
                sh.print_scale = if next == 100 { None } else { Some(next) };
                self.dirty = true;
                self.status = format!("拡大縮小印刷: {next}%(PDF と保存に効きます)").into();
            }
            // 改ページ: いまの行から新しい紙を始める(もう一度で解除)
            "pagebreak" => {
                self.commit();
                self.checkpoint();
                let r = self.cursor.row;
                let sh = self.sheet_mut();
                if let Some(i) = sh.row_breaks.iter().position(|b| *b == r) {
                    sh.row_breaks.remove(i);
                    self.dirty = true;
                    self.status = format!("{} 行の改ページを外しました", r + 1).into();
                } else if r == 0 {
                    self.undo_stack.pop();
                    self.status = "1行目の前では改ページできません".into();
                } else {
                    sh.row_breaks.push(r);
                    self.dirty = true;
                    self.status =
                        format!("{} 行から新しい紙にします(もう一度で解除)", r + 1).into();
                }
            }
            // タイトルを印刷: 選んだ行を各ページの頭で繰り返す。選択なしで解除
            "printtitles" => {
                self.commit();
                if self.anchor.is_some() {
                    self.checkpoint();
                    let (a, b) = self.sel_rect();
                    self.sheet_mut().print_title_rows = Some((a.row, b.row));
                    self.dirty = true;
                    self.status = format!(
                        "{}〜{} 行を各ページの頭で繰り返します(選択なしで押すと解除)",
                        a.row + 1,
                        b.row + 1
                    )
                    .into();
                } else if self.sheet().print_title_rows.is_some() {
                    self.checkpoint();
                    self.sheet_mut().print_title_rows = None;
                    self.dirty = true;
                    self.status = "タイトル行を解除しました".into();
                } else {
                    self.status =
                        "繰り返す行を選んでから押してください(行の見出しをクリック)".into();
                }
            }
            "print-gridlines" => {
                self.commit();
                self.checkpoint();
                let sh = self.sheet_mut();
                sh.print_gridlines = !sh.print_gridlines;
                let on = sh.print_gridlines;
                self.dirty = true;
                self.status = format!(
                    "枠線の印刷: {}",
                    if on { "する(表の薄い線が紙に出ます)" } else { "しない" }
                )
                .into();
            }
            "print-headings" => {
                self.commit();
                self.checkpoint();
                let sh = self.sheet_mut();
                sh.print_headings = !sh.print_headings;
                let on = sh.print_headings;
                self.dirty = true;
                self.status = format!(
                    "見出しの印刷: {}",
                    if on { "する(行番号と列名が余白に出ます)" } else { "しない" }
                )
                .into();
            }
            // 検索と置換(ホーム > 置き換え)。板を2枚続けて使う
            "replace" => {
                self.commit();
                let init = self.find_term.clone().unwrap_or_default();
                self.prompt = Some(("find", Editor::new(&init)));
            }
            // グラフ(matplotlib)と画像。挿入タブ
            "inschart" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status =
                        "グラフにする範囲を選んでください(1列目が項目名、2列目からが数)"
                            .into();
                } else {
                    let (a, b) = self.sel_rect();
                    self.insert_chart(a, b, cx);
                }
            }
            "insimage" => {
                self.commit();
                self.insert_image_dialog(cx);
            }
            "instext" => {
                // テキストボックス = 枠の図形 + 文字。すぐ文字の板を開く
                self.checkpoint();
                let at = self.cursor;
                self.sheet_mut().shapes_new.push(sheet::model::SheetShape {
                    at,
                    width_px: 200.0,
                    height_px: 80.0,
                    kind: "rect".into(),
                    fill: None,
                    line: Some("7F7F7F".into()),
                    ..Default::default()
                });
                self.shape_sel = Some(self.sheet().shapes_new.len() - 1);
                self.dirty = true;
                self.prompt = Some(("shape-text", Editor::new("")));
            }
            "inssparkline" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status =
                        "折れ線にする数の範囲を選んでください(置き場所はいまのセル)".into();
                } else {
                    let (a, b) = self.sel_rect();
                    let mut vals: Vec<f64> = Vec::new();
                    for r in a.row..=b.row {
                        for c in a.col..=b.col {
                            if let Some(cell) = self.sheet().get(Pos::new(r, c)) {
                                if let sheet::Value::Number(n) = cell.value {
                                    vals.push(n);
                                }
                            }
                        }
                    }
                    if vals.len() < 2 {
                        self.status = "数が2つ以上要ります".into();
                    } else {
                        let (lo, hi) = vals
                            .iter()
                            .fold((f64::MAX, f64::MIN), |(l, h), v| (l.min(*v), h.max(*v)));
                        let span = (hi - lo).max(1e-9);
                        let n = vals.len();
                        let points: Vec<(f32, f32)> = vals
                            .iter()
                            .enumerate()
                            .map(|(i, v)| {
                                (
                                    i as f32 / (n - 1) as f32,
                                    (1.0 - ((v - lo) / span)) as f32,
                                )
                            })
                            .collect();
                        // 置き場所はいまのセル(選択の中なら右のセル)、大きさはそのセル
                        let at = if (a.row..=b.row).contains(&self.cursor.row)
                            && (a.col..=b.col).contains(&self.cursor.col)
                        {
                            Pos::new(a.row, b.col + 1)
                        } else {
                            self.cursor
                        };
                        self.checkpoint();
                        let (w, h) = (self.col_px(at.col) - 2.0, self.row_px(at.row) - 2.0);
                        self.sheet_mut().shapes_new.push(sheet::model::SheetShape {
                            at,
                            width_px: w,
                            height_px: h,
                            kind: "spark".into(),
                            fill: None,
                            line: Some("1B6E3C".into()),
                            points,
                            ..Default::default()
                        });
                        self.dirty = true;
                        self.status = format!(
                            "スパークラインを {} に置きました(その時の値で描く固定の線。\
データを変えたら作り直してください)",
                            at.a1()
                        )
                        .into();
                    }
                }
            }
            "insshape" => {
                let at = self
                    .cell_origin_px(self.cursor)
                    .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                    .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                self.pick_kind = "shape";
                self.pick = Some((
                    ["四角形", "角丸四角形", "楕円", "右矢印", "ひし形", "直線"]
                        .iter()
                        .map(|v| v.to_string())
                        .collect(),
                    at,
                ));
            }
            "inshyperlink" => {
                self.commit();
                let cur = self.sheet().links.get(&self.cursor).cloned().unwrap_or_default();
                self.prompt = Some(("link", Editor::new(&cur)));
            }
            // データの入力規則。選んだ範囲に候補を付ける(板で受ける)
            "data-validation" => {
                self.commit();
                // 既にある規則は編集の初期値に(直書きは中身、参照は = 付き)
                let cur = self
                    .sheet()
                    .validation_at(self.cursor)
                    .map(|v| v.formula.clone())
                    .unwrap_or_default();
                let init = if cur.is_empty() {
                    String::new()
                } else if let Some(inner) =
                    cur.strip_prefix('"').and_then(|s| s.strip_suffix('"'))
                {
                    inner.to_string()
                } else {
                    format!("={cur}")
                };
                self.prompt = Some(("validation", Editor::new(&init)));
            }
            // 条件付き書式。右クリックメニューと同じ一覧を開く(道は1本)
            "condformat" => {
                let (x, y) = self
                    .cell_origin_px(self.cursor)
                    .map(|(x, y)| (x + 16.0, y + 16.0))
                    .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                self.menu_at = Some((x, y));
                self.menu_sub = Some("cond");
            }
            // 名前の管理。右クリックの「名前の定義」と同じ板
            "defname" => {
                self.commit();
                self.prompt = Some(("name", Editor::new("")));
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
            f @ ("fn-math" | "fn-text" | "fn-logical" | "fn-recent" | "fn-datetime"
            | "fn-lookup" | "fn-financial" | "fn-more") => {
                let names: &str = match f {
                    "fn-math" => "SUM AVERAGE ROUND ROUNDUP ROUNDDOWN INT ABS MOD POWER SQRT PRODUCT",
                    "fn-text" => "LEN LEFT RIGHT MID TRIM UPPER LOWER CONCATENATE",
                    "fn-logical" => "IF AND OR NOT TRUE FALSE ISBLANK ISERROR IFERROR",
                    "fn-datetime" => "TODAY NOW DATE YEAR MONTH DAY WEEKDAY(値は通し番号)",
                    "fn-lookup" => "VLOOKUP HLOOKUP INDEX MATCH(照合は完全一致)",
                    "fn-financial" => "PMT PV FV NPER(RATE のような反復解はまだ)",
                    "fn-more" => "SUMIF COUNTIF COUNTA TRUNC IFERROR — 一覧は各族の釦で",
                    _ => "SUM AVERAGE COUNT MAX MIN IF SUMIF COUNTIF VLOOKUP TODAY",
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

    /// 保存。名前が無ければ選ばせる(**ダイアログは別の糸**)。
    /// `then_quit` なら保存が済んだときだけ終了する — 書きかけを黙って捨てない。
    fn save(&mut self, then_quit: bool, cx: &mut Context<Self>) {
        self.commit();
        if let Some(p) = self.path.clone() {
            if self.locked_by.is_some() {
                // 先客の作業を後勝ちで潰さない。別の名前でなら保存できる
                self.status = format!(
                    "{} が開いているため上書きしません。名前を付けて保存してください",
                    self.locked_by.as_deref().unwrap_or("誰か")
                )
                .into();
            } else {
                self.save_to(p);
                if then_quit && !self.dirty {
                    self.release_lock();
                    cx.quit();
                }
                return;
            }
        }
        let ask = cx.background_executor().spawn(async {
            rfd::FileDialog::new()
                .add_filter("Excelブック", &["xlsx"])
                .save_file()
        });
        cx.spawn(async move |this, cx| {
            let r = ask.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Some(p) => {
                        this.save_to(p);
                        if then_quit && !this.dirty {
                            this.release_lock();
                            cx.quit();
                        }
                    }
                    None => this.status = "保存をやめました(名前が決まっていません)".into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 決まった場所へ書く。成功すると dirty が消える。
    fn save_to(&mut self, p: PathBuf) {
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
                self.acquire_lock(&p);
                self.path = Some(p);
                self.dirty = false;
                // 挿した絵はもう原本(いま書いたファイル)にある。次の保存で
                // 二重に書かないよう「読んだ側」へ持ち場を移す
                for sh in &mut self.book.sheets {
                    let moved: Vec<_> = sh.images_new.drain(..).collect();
                    sh.images.extend(moved);
                    let moved: Vec<_> = sh.shapes_new.drain(..).collect();
                    sh.shapes.extend(moved);
                }
                self.shape_sel = None;
            }
            Err(e) => self.status = format!("保存できません: {e}").into(),
        }
    }
}

impl Drop for Calc {
    fn drop(&mut self) {
        // 置きっぱなしのロックは他の人の警告になってしまう。最後の保険
        self.release_lock();
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
                    e.click_count,
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
                if c.shape_drag.is_some() {
                    c.shape_drag_at(f32::from(rel.x), f32::from(rel.y));
                    cx.notify();
                } else if c.size_drag.is_some() {
                    c.size_drag_at(f32::from(rel.x), f32::from(rel.y));
                    cx.notify();
                } else if c.drag.is_some() || c.head_drag.is_some() {
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

/// PY の引数を Python の書き方(リテラル)にする。
fn py_literal(v: &sheet::Value) -> String {
    match v {
        sheet::Value::Number(n) => format!("{n}"),
        sheet::Value::Bool(b) => (if *b { "True" } else { "False" }).into(),
        sheet::Value::Empty => "None".into(),
        v => format!("{:?}", v.display()), // Rust の {:?} は Python でも読める逃がし
    }
}

/// @計算 の台本。「関数」スクリプトの def を読み、各 PY セルを評価して
/// 区切りの印(\x1c セル / \x1e 行 / \x1f 欄)で吐く。
fn build_udf_script(
    defs: &str,
    calls: &[(String, String, Vec<sheet::calc::PyArg>)], // (セルA1, 関数名, 引数)
    out_path: &std::path::Path,
) -> String {
    let mut body = String::new();
    for (cell, fname, args) in calls {
        let mut lit_args = Vec::new();
        for a in args {
            match a {
                sheet::calc::PyArg::One(v) => lit_args.push(py_literal(v)),
                sheet::calc::PyArg::Rect(cols, vs) => {
                    let cols = (*cols as usize).max(1);
                    let rows: Vec<String> = vs
                        .chunks(cols)
                        .map(|row| {
                            format!(
                                "[{}]",
                                row.iter().map(py_literal).collect::<Vec<_>>().join(",")
                            )
                        })
                        .collect();
                    lit_args.push(format!("[{}]", rows.join(",")));
                }
            }
        }
        body.push_str(&format!(
            "_jo_emit({cell:?}, {fname}({args}))\n",
            cell = cell,
            fname = fname,
            args = lit_args.join(", ")
        ));
    }
    format!(
        concat!(
            "# aiseed calc の PY(UDF)評価。関数の定義はブックの「関数」スクリプト\n",
            "{defs}\n",
            "_jo_out = []\n",
            "def _jo_emit(cell, r):\n",
            "    if not isinstance(r, (list, tuple)):\n",
            "        r = [[r]]\n",
            "    elif r and not isinstance(r[0], (list, tuple)):\n",
            "        r = [[v] for v in r]  # 1次元は縦に広げる\n",
            "    rows = ['\\x1f'.join('' if v is None else str(v) for v in row) for row in r]\n",
            "    _jo_out.append(cell + '\\x1e' + '\\x1e'.join(rows))\n",
            "{body}\n",
            "open({out:?}, 'w', encoding='utf-8').write('\\x1c'.join(_jo_out))\n"
        ),
        defs = defs,
        body = body,
        out = out_path.to_string_lossy()
    )
}

/// 台本の出力を (セル, 行×欄の文字) に戻す。
fn parse_udf_output(raw: &str) -> Vec<(Pos, Vec<Vec<String>>)> {
    raw.split('\u{1c}')
        .filter_map(|rec| {
            let mut it = rec.split('\u{1e}');
            let cell = Pos::parse(it.next()?)?;
            let rows: Vec<Vec<String>> = it
                .map(|r| r.split('\u{1f}').map(|v| v.to_string()).collect())
                .collect();
            (!rows.is_empty()).then_some((cell, rows))
        })
        .collect()
}

/// PY の結果をシートへ。錨のセルは**式を保ったまま**値を差し替え、
/// 2次元はスピル(右下へ展開)。他人のデータを潰しそうなら #SPILL! で止まる。
/// 返すのは (新しいスピルの台帳, 適用した数, 衝突した数)。
fn apply_py_results(
    sh: &mut sheet::Sheet,
    results: &[(Pos, Vec<Vec<String>>)],
    prev: &std::collections::HashMap<Pos, (u32, u32)>,
) -> (std::collections::HashMap<Pos, (u32, u32)>, usize, usize) {
    // 前回のスピル面(錨以外)をまず消す(小さくなったとき古い値を残さない)
    for (anchor, (rows, cols)) in prev {
        for dr in 0..*rows {
            for dc in 0..*cols {
                if dr == 0 && dc == 0 {
                    continue;
                }
                let p = Pos::new(anchor.row + dr, anchor.col + dc);
                if let Some(c) = sh.cells.get_mut(&p) {
                    if c.formula.is_none() {
                        c.value = sheet::Value::Empty;
                    }
                }
            }
        }
    }
    let mut spills = std::collections::HashMap::new();
    let (mut applied, mut conflicts) = (0usize, 0usize);
    for (anchor, rows) in results {
        let (nr, nc) = (rows.len() as u32, rows.iter().map(|r| r.len()).max().unwrap_or(1) as u32);
        // 衝突検査(錨以外に、中身か式のあるセルが居ないか)
        let mut blocked = false;
        for dr in 0..nr {
            for dc in 0..nc {
                if dr == 0 && dc == 0 {
                    continue;
                }
                let p = Pos::new(anchor.row + dr, anchor.col + dc);
                if let Some(c) = sh.cells.get(&p) {
                    let was_prev_spill = prev
                        .get(anchor)
                        .is_some_and(|(pr, pc)| dr < *pr && dc < *pc);
                    if c.formula.is_some() || (!c.value.is_empty() && !was_prev_spill) {
                        blocked = true;
                    }
                }
            }
        }
        let put = |sh: &mut sheet::Sheet, p: Pos, text: &str| {
            let fmt = sh.get(p).map(|c| c.fmt.clone()).unwrap_or_default();
            let formula = sh.get(p).and_then(|c| c.formula.clone());
            let value = if text.is_empty() {
                sheet::Value::Empty
            } else if let Ok(n) = text.parse::<f64>() {
                sheet::Value::Number(n)
            } else {
                sheet::Value::Text(text.to_string())
            };
            sh.set(p, sheet::Cell { formula, value, fmt });
        };
        if blocked {
            let fmt = sh.get(*anchor).map(|c| c.fmt.clone()).unwrap_or_default();
            let formula = sh.get(*anchor).and_then(|c| c.formula.clone());
            sh.set(
                *anchor,
                sheet::Cell {
                    formula,
                    value: sheet::Value::Error("#SPILL!".into()),
                    fmt,
                },
            );
            conflicts += 1;
            continue;
        }
        for (dr, row) in rows.iter().enumerate() {
            for (dc, text) in row.iter().enumerate() {
                put(sh, Pos::new(anchor.row + dr as u32, anchor.col + dc as u32), text);
            }
        }
        if nr > 1 || nc > 1 {
            spills.insert(*anchor, (nr, nc));
        }
        applied += 1;
    }
    (spills, applied, conflicts)
}

/// 排他ロックの置き場(LibreOffice と同じ `.~lock.<名前>#`)。
/// ファイルサーバーの共有フォルダで「同時に開いて後勝ちで潰す」を防ぐ。
fn lock_path_for(p: &std::path::Path) -> std::path::PathBuf {
    let name = p.file_name().unwrap_or_default().to_string_lossy();
    p.with_file_name(format!(".~lock.{name}#"))
}

/// 自分の名乗り(誰が開いているか)。user@host。
fn lock_identity() -> String {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "?".into());
    let host = std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "?".into());
    format!("{user}@{host}")
}

/// 先客のロックを読む(あれば名乗りを返す)。自分自身のロックは先客と見ない。
fn foreign_lock(p: &std::path::Path) -> Option<String> {
    let lp = lock_path_for(p);
    let raw = std::fs::read_to_string(lp).ok()?;
    let who = raw
        .split(',')
        .map(str::trim)
        .find(|t| !t.is_empty())
        .unwrap_or("誰か")
        .to_string();
    (who != lock_identity()).then_some(who)
}

/// ゴールシークの解探索(割線法)。表の複製の上で var を動かし、
/// target が goal になる値を探す。見つからなければ None。
fn solve_goal(base: &sheet::Sheet, target: Pos, goal: f64, var: Pos) -> Option<f64> {
    let probe = |x: f64| -> f64 {
        let mut s = base.clone();
        let fmt = s.get(var).map(|c| c.fmt.clone()).unwrap_or_default();
        let mut cell = Cell::input(&format!("{x}"));
        cell.fmt = fmt;
        s.set(var, cell);
        recalc(&mut s);
        s.value(target).as_number() - goal
    };
    let x0 = base.get(var).map(|c| c.value.as_number()).unwrap_or(0.0);
    let (mut a, mut b) = (x0, if x0 == 0.0 { 1.0 } else { x0 * 1.1 });
    let (mut fa, mut fb) = (probe(a), probe(b));
    let tol = 1e-7 * goal.abs().max(1.0);
    for _ in 0..200 {
        if fb.abs() < tol {
            return Some(b);
        }
        if (fb - fa).abs() < f64::EPSILON {
            return None;
        }
        let c = b - fb * (b - a) / (fb - fa);
        if !c.is_finite() {
            return None;
        }
        (a, fa) = (b, fb);
        (b, fb) = (c, probe(c));
    }
    None
}

/// 画像の寸法(px)。PNG は IHDR、JPEG は SOF から(writer と同じ読み方)。
fn image_px(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        let w = u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?);
        let h = u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?);
        return Some((w, h));
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        let mut i = 2usize;
        while i + 9 < bytes.len() {
            if bytes[i] != 0xFF {
                return None;
            }
            let marker = bytes[i + 1];
            if marker == 0xFF || (0xD0..=0xD9).contains(&marker) || marker == 0x01 {
                i += 2;
                continue;
            }
            let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
            if matches!(marker, 0xC0..=0xC3) {
                let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
                let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
                return Some((w, h));
            }
            i += 2 + len;
        }
        return None;
    }
    None
}

/// グラフ描きの Python を探す。JO_PYTHON → リポジトリの .venv → python3。
/// matplotlib が居るかは実行して分かる(居なければ status で言う)。
fn find_python() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("JO_PYTHON") {
        return p.into();
    }
    let venv = std::path::Path::new(".venv/bin/python");
    if venv.exists() {
        return venv.into();
    }
    "python3".into()
}

/// グラフの台本(matplotlib)。データは JSON で渡す。
/// 日本語は機械のフォントを matplotlib に登録して出す(豆腐にしない)。
const CHART_PY: &str = r#"
import json, sys
import matplotlib
matplotlib.use("Agg")
import numpy as np
from matplotlib import font_manager, pyplot as plt

spec = json.load(open(sys.argv[1], encoding="utf-8"))
if spec.get("font"):
    try:
        font_manager.fontManager.addfont(spec["font"])
        plt.rcParams["font.family"] = font_manager.FontProperties(
            fname=spec["font"]).get_name()
    except Exception:
        pass
labels = spec["labels"]
x = np.arange(len(labels))
series = spec["series"] or [{"name": "", "values": [0] * len(labels)}]
n = len(series)
w = 0.8 / n
fig, ax = plt.subplots(figsize=(6.4, 4.0))
for i, s in enumerate(series):
    ax.bar(x + (i - (n - 1) / 2) * w, s["values"], w, label=s["name"])
ax.set_xticks(x)
ax.set_xticklabels(labels)
if n > 1:
    ax.legend()
fig.tight_layout()
fig.savefig(spec["out"], dpi=100)
"#;

/// CSV/TSV 読みの台本。文字コード(UTF-8 → CP932 → Latin-1)と区切りを判定し、
/// 区切りに使えない印(US=\x1F, RS=\x1E)で吐く — タブや改行入りの欄でも壊れない。
const CSV_PY: &str = r#"
import csv, sys

path = sys.argv[1]
raw = open(path, "rb").read()
text = None
for enc in ("utf-8-sig", "cp932", "latin-1"):
    try:
        text = raw.decode(enc)
        break
    except UnicodeDecodeError:
        continue
if text is None:
    sys.exit("文字コードが判定できません")
try:
    dialect = csv.Sniffer().sniff(text[:4096], delimiters=",\t;")
except csv.Error:
    dialect = csv.excel_tab if "\t" in text[:4096] else csv.excel
rows = list(csv.reader(text.splitlines(), dialect))
out = "\x1e".join("\x1f".join(row) for row in rows)
sys.stdout.buffer.write(out.encode("utf-8"))
"#;

fn col_name(c: u32) -> String {
    Pos::new(0, c).a1().trim_end_matches('1').to_string()
}

/// xlsx の paperSize → mm と名前。**B は JIS**(ECMA-376 の表は ISO だが、
/// 日本の事務様式と日本語版の印刷ドライバの実情は JIS。ここは日本のソフト)。
fn paper_mm(code: u32) -> Option<(f32, f32, &'static str)> {
    Some(match code {
        8 => (297.0, 420.0, "A3"),
        9 => (210.0, 297.0, "A4"),
        11 => (148.0, 210.0, "A5"),
        12 => (257.0, 364.0, "B4"),
        13 => (182.0, 257.0, "B5"),
        _ => return None,
    })
}

impl Render for Calc {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::env::var_os("JO_SELFTEST").is_some() {
            // 実際に描画が走った証拠を残す(notify だけでは画面は変わらない —
            // これが止まってティックが続くなら、提示(present)の停止)
            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            eprintln!("render #{}", N.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        }
        // ---- リボン(Euro-Office に名前と並びを合わせる) ----
        // **タブの行そのものが窓の取っ手**(掴んで移動・二度押しで最大化)。
        // 空きの帯だけだとタブが多い窓で幅がゼロになり掴めない(writer で踏んだ)。
        // 釦の類いは stop_propagation で取っ手より先に効く
        let (ready, all) = ribbon::progress(ribbon::CALC);
        let mut tabs = div().id("titlebar").flex().flex_row().items_end().gap_1()
            .px_3().pt_1p5().bg(rgb(0x1B6E3C))
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                |_, e: &gpui::MouseDownEvent, window, _| {
                    if e.click_count >= 2 {
                        window.zoom_window();
                    } else {
                        window.start_window_move();
                    }
                }));
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
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| { this.tab = i; cx.notify() })));
        }
        tabs = tabs
            .child(div().flex_1().h(px(28.0)))
            .child(div().pb_1p5().pr_2().text_size(px(10.5)).text_color(rgb(0x9CC9AF))
                   .child(SharedString::from(format!("calc — 実装済み {ready}/{all}"))));
        let winbtn = |id: &'static str, label: &'static str| {
            div().id(id).px_2p5().py_1().rounded_sm()
                .text_size(px(12.0)).text_color(rgb(0xCFE6D8))
                .cursor_pointer()
                .hover(move |s| if id == "close" { s.bg(rgb(0xC0392B)).text_color(rgb(0xFFFFFF)) }
                                else { s.bg(rgb(0x2E8B57)).text_color(rgb(0xFFFFFF)) })
                .child(label)
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        tabs = tabs
            .child(winbtn("min", "─").on_click(cx.listener(|_, _, window, _| {
                window.minimize_window();
            })))
            .child(winbtn("max", "▢").on_click(cx.listener(|_, _, window, _| {
                window.zoom_window();
            })))
            .child(winbtn("close", "✕").on_click(cx.listener(|this, _, _, cx| {
                this.request_quit(cx);
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
                        this.run_cmd(id, cx); cx.notify()
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
        let (sel_a, sel_b) = self.sel_rect();
        let has_sel = self.anchor.is_some();
        for c in grid_cols(self.frozen, self.view, COLS) {
            // 選択に入っている列の見出しは色を変える(いまどこを選んでいるかの道標)
            let on = has_sel && (sel_a.col..=sel_b.col).contains(&c) || c == self.cursor.col;
            head = head.child(div().w(px(self.col_px(c))).h(px(ROW_H))
                .bg(if on { rgb(0xCFE6D8) } else { rgb(0xEFF2F4) })
                .border_r_1().border_b_1()
                .border_color(rgb(0xD5DBE0))
                .flex().items_center().justify_center()
                .text_size(px(11.5))
                .text_color(if on { rgb(0x1B6E3C) } else { rgb(0x66707A) })
                .child(SharedString::from(col_name(c)))
                // 右端の帯は幅を変える取っ手(カーソル形状の誘いだけ。
                // 当たり判定は InputSink の窓レベルで size_grip_at がやる)
                .relative().children((std::env::var_os("JO_NO_STRIPS").is_none()).then(|| {
                    div().absolute()
                        .top(px(0.0)).right(px(-GRIP)).w(px(GRIP * 2.0)).h_full()
                        .cursor_col_resize()
                })));
        }
        grid = grid.child(head);

        // 当たり判定(cell_at)と同じ並びを使う — ずれるとクリックが別のセルに入る
        let visible: Vec<u32> = self.visible_rows();
        for r in visible {
            let rh = self.row_px(r);
            let row_on = has_sel && (sel_a.row..=sel_b.row).contains(&r) || r == self.cursor.row;
            let mut row = div().flex().flex_row()
                .child(div().w(px(HEAD_W)).h(px(rh))
                    .bg(if row_on { rgb(0xCFE6D8) } else { rgb(0xEFF2F4) })
                    .border_r_1().border_b_1()
                    .border_color(rgb(0xD5DBE0))
                    .flex().items_center().justify_center()
                    .text_size(px(11.5))
                    .text_color(if row_on { rgb(0x1B6E3C) } else { rgb(0x66707A) })
                    .child(SharedString::from((r + 1).to_string()))
                    // 下端の帯は高さを変える取っ手(列見出しの右端と同じ仕掛け)
                    .relative().children((std::env::var_os("JO_NO_STRIPS").is_none()).then(|| {
                        div().absolute()
                            .left(px(0.0)).bottom(px(-GRIP)).w_full().h(px(GRIP * 2.0))
                            .cursor_row_resize()
                    })));
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
                // トレースの光り(参照元=青緑、参照先=橙)。塗りは透けたまま
                if let Some((_, prec)) = self.trace.iter().find(|(tp, _)| *tp == p) {
                    d = d.bg(if *prec {
                        gpui::Rgba { r: base.r * 0.55 + 0.10, g: base.g * 0.55 + 0.38, b: base.b * 0.55 + 0.38, a: 1.0 }
                    } else {
                        gpui::Rgba { r: base.r * 0.55 + 0.43, g: base.g * 0.55 + 0.30, b: base.b * 0.55 + 0.08, a: 1.0 }
                    });
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
                // 入力規則のあるセルを選ぶと右下に ▾
                // (右クリック → ドロップダウンリストから選択、の目印)
                if sel && self.sheet().validation_at(p).is_some() {
                    d = d.relative().child(div().absolute()
                        .bottom(px(-1.0)).right(px(1.0))
                        .text_size(px(8.5)).text_color(rgb(0x1B6E3C))
                        .child("▾"));
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
                // 本家(Euro-Office)に無いのが残念、との声で追加した唯一の独自項目
                ("pastesp", "形式を選択して貼り付け", "", true, true),
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

        // ---- 選択中の図形の枠と右下の掴み ----
        let shape_frame = self.shape_sel.and_then(|i| {
            let sp = self.sheet().shapes_new.get(i)?;
            let (x, y) = self.cell_origin_px(sp.at)?;
            Some(
                div()
                    .absolute()
                    .left(px(x - 2.0))
                    .top(px(y - 2.0))
                    .w(px(sp.width_px + 4.0))
                    .h(px(sp.height_px + 4.0))
                    .border_2()
                    .border_dashed()
                    .border_color(rgb(0x1B6E3C))
                    .child(
                        div()
                            .absolute()
                            .right(px(-1.0))
                            .bottom(px(-1.0))
                            .w(px(10.0))
                            .h(px(10.0))
                            .bg(rgb(0x1B6E3C))
                            .cursor_nwse_resize(),
                    ),
            )
        });

        // ---- コピーした範囲の破線(蟻の行進の静止版) ----
        // セルの罫線と混ざらないよう、重ね描きの1枚で囲む。マウスは受けない
        let ants = self.clip_range.and_then(|(si, a, b)| {
            if si != self.active {
                return None;
            }
            self.range_px(a, b).map(|(x0, y0, x1, y1)| {
                div().absolute()
                    .left(px(x0)).top(px(y0))
                    .w(px((x1 - x0).max(2.0))).h(px((y1 - y0).max(2.0)))
                    .border_2().border_dashed().border_color(rgb(0x1B6E3C))
            })
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
                "validation" => format!("入力規則 — {range} は候補から選ぶ(空にして Enter で解除)"),
                "find" => "検索と置換 — 探す言葉".to_string(),
                "split-delim" => format!("区切り位置 — {range} を何で割る?(空 Enter = カンマ)"),
                "shape-text" => "図形の文字(空にして Enter で消す)".to_string(),
                "py" => "Python — 一行のコード(空 Enter = .py ファイルを選ぶ)".to_string(),
                "goal-target" => "ゴールシーク — 目標(セル=値。例: D6=800000)".to_string(),
                "goal-var" => format!(
                    "{} をいくつにするか探します — 変えるセルは?(例: B2)",
                    self.goal.map(|(p, v)| format!("{}={v}", p.a1())).unwrap_or_default()
                ),
                "replace-with" => format!(
                    "「{}」を何に置き換える?",
                    self.find_term.as_deref().unwrap_or("")
                ),
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
                    .child(match *kind {
                        "name" => "Enter で決定 / Esc で取消。定義した名前は式の中で使えます(=単価*2)",
                        "validation" => "候補の直書き(甲,乙,丙)か、範囲の参照(=D2:D5)。Enter で決定 / Esc で取消",
                        "find" => "Enter で次へ / Esc で取消。式の中の文字も探します",
                        "split-delim" => "選択した列の文字を割って、右の列へ並べます(右は上書き)",
                        "shape-text" => "図形を選んで Enter でいつでも書き直せます",
                        "py" => "b=ブック s=シート / @計算 =PY(…)セルを評価 / @名前 実行 @名前 net @save @list @del",
                        "goal-target" | "goal-var" => "式のセルが目標の値になるよう、変えるセルの数を探します",
                        "replace-with" => "Enter で全て置き換え / **空のまま Enter = 検索だけ** / Esc で取消",
                        _ => "Enter で決定 / Esc で取消",
                    }))
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
                            this.fmt_panel_action(id, cx);
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
                        this.fmt_panel_action(id, cx);
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
                            this.apply_pick(&v);
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
            .on_action(cx.listener(Calc::a_paste_values))
            .on_action(cx.listener(Calc::a_left))
            .on_action(cx.listener(Calc::a_right))
            .on_action(cx.listener(Calc::a_up))
            .on_action(cx.listener(Calc::a_down))
            .on_action(cx.listener(Calc::a_page_up))
            .on_action(cx.listener(Calc::a_page_down))
            .on_action(cx.listener(Calc::a_doc_home))
            .on_action(cx.listener(Calc::a_doc_end))
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
                   .children({
                       // 浮かぶ画像(グラフ)。錨のセルが見えている間だけ描く。
                       // マウスは受けない(セルの操作を遮らない)
                       let mut layer: Vec<gpui::AnyElement> = Vec::new();
                       for im in self.sheet().images.iter().chain(self.sheet().images_new.iter()) {
                           let Some((x, y)) = self.cell_origin_px(im.at) else { continue };
                           let key = im.data.as_ptr() as usize;
                           let src = self
                               .img_cache
                               .borrow_mut()
                               .entry(key)
                               .or_insert_with(|| {
                                   let fmt = if im.data.starts_with(&[0xFF, 0xD8]) {
                                       gpui::ImageFormat::Jpeg
                                   } else {
                                       gpui::ImageFormat::Png
                                   };
                                   std::sync::Arc::new(gpui::Image::from_bytes(
                                       fmt,
                                       im.data.clone(),
                                   ))
                               })
                               .clone();
                           layer.push(
                               gpui::img(src)
                                   .absolute()
                                   .left(px(x))
                                   .top(px(y))
                                   .w(px(im.width_px))
                                   .h(px(im.height_px))
                                   .into_any_element(),
                           );
                       }
                       // 図形(SVG)。大きさを織り込んで作るので、伸ばしても鮮明
                       for (i, sp) in self
                           .sheet()
                           .shapes
                           .iter()
                           .chain(self.sheet().shapes_new.iter())
                           .enumerate()
                       {
                           let Some((x, y)) = self.cell_origin_px(sp.at) else { continue };
                           let svg = sp.to_svg();
                           let key = {
                               use std::hash::{Hash, Hasher};
                               let mut h = std::collections::hash_map::DefaultHasher::new();
                               svg.hash(&mut h);
                               h.finish() as usize
                           };
                           let src = self
                               .img_cache
                               .borrow_mut()
                               .entry(key)
                               .or_insert_with(|| {
                                   std::sync::Arc::new(gpui::Image::from_bytes(
                                       gpui::ImageFormat::Svg,
                                       svg.into_bytes(),
                                   ))
                               })
                               .clone();
                           layer.push(
                               gpui::img(src)
                                   .absolute()
                                   .left(px(x))
                                   .top(px(y))
                                   .w(px(sp.width_px))
                                   .h(px(sp.height_px))
                                   .into_any_element(),
                           );
                           if let Some(t) = &sp.text {
                               layer.push(
                                   div()
                                       .absolute()
                                       .left(px(x + 6.0))
                                       .top(px(y + 4.0))
                                       .w(px((sp.width_px - 12.0).max(8.0)))
                                       .h(px((sp.height_px - 8.0).max(8.0)))
                                       .overflow_hidden()
                                       .text_size(px(12.5))
                                       .font_family("Noto Sans JP")
                                       .text_color(rgb(0x1B1B1B))
                                       .whitespace_normal()
                                       .child(SharedString::from(t.clone()))
                                       .into_any_element(),
                               );
                           }
                           let _ = i;
                       }
                       // 控えが育ちすぎたら捨てる(undo のクローンで鍵が増えるため)
                       if self.img_cache.borrow().len() > 64 {
                           self.img_cache.borrow_mut().clear();
                       }
                       layer
                   })
                   .child(InputSink { view: me })
                   .children(shape_frame)
                   .children(ants)
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
                // 書きかけがあれば「まだ閉じない」と答え、確認は別の糸で出す
                let v = view.clone();
                window.on_window_should_close(cx, move |_, cx| {
                    let quit_now = v.update(cx, |this, cx| {
                        this.commit();
                        if this.dirty && this.path.is_some() {
                            this.request_quit(cx);
                            false
                        } else {
                            this.release_lock();
                            true
                        }
                    });
                    if quit_now {
                        cx.quit();
                    }
                    quit_now
                });
                if std::env::var_os("JO_SELFTEST").is_some() {
                    // 画面が実際に動くかの自己診断: B列の幅を1秒ごとに広げ狭めし、
                    // 15秒で自動終了する。**操作は要らない** — 見ているだけで、
                    // 「モデルは動くのに画面が止まる」疑いを切り分けられる
                    let v = view.clone();
                    cx.spawn(async move |cx| {
                        for i in 0..15u32 {
                            cx.background_executor()
                                .timer(std::time::Duration::from_millis(1000))
                                .await;
                            let _ = v.update(cx, |c, cx| {
                                let w = if i % 2 == 0 { 20.0 } else { 5.0 };
                                c.book.sheets[0].col_width.insert(1, w);
                                eprintln!("tick {}", i + 1);
                                c.status = format!(
                                    "自己診断 {}/15: B列の幅 {w}(勝手に動けば描画は健全)",
                                    i + 1
                                )
                                .into();
                                cx.notify();
                            });
                        }
                        let _ = cx.update(|cx| cx.quit());
                    })
                    .detach();
                }
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
mod size_grip_tests {
    use super::*;

    #[test]
    fn 境界の近くだけ掴める() {
        // 2列(48px, 108px)が HEAD_W から並ぶ
        let cols = [(0u32, 48.0f32), (1, 108.0)];
        let e1 = HEAD_W + 48.0; // 1本目の境界
        let e2 = e1 + 108.0; // 2本目
        assert_eq!(grip_hit(&cols, HEAD_W, e1), Some(0));
        assert_eq!(grip_hit(&cols, HEAD_W, e1 - GRIP), Some(0), "縁の手前±GRIPで掴めない");
        assert_eq!(grip_hit(&cols, HEAD_W, e1 + GRIP), Some(0));
        assert_eq!(grip_hit(&cols, HEAD_W, e2 - 1.0), Some(1), "2本目の境界が累積位置にない");
        assert_eq!(grip_hit(&cols, HEAD_W, e1 + GRIP + 1.0), None, "境界の外で掴めた");
        assert_eq!(grip_hit(&cols, HEAD_W, HEAD_W + 10.0), None, "列の中ほどで掴めた");
    }

    #[test]
    fn 幅の換算が往復する() {
        // 画面px → xlsxの字数 → 画面px が(丸め2桁でも)崩れない
        let px0 = 108.0f32;
        let w = ((px0 / PX_PER_CHW) * 100.0).round() / 100.0;
        assert!((w - 8.43).abs() < 0.01, "既定幅が 8.43 にならない: {w}");
        assert!((w * PX_PER_CHW - px0).abs() < 0.5, "幅の往復がずれる");
        // 行: 画面px → pt → 画面px。既定 24px = 15pt
        let pt = (24.0f32 * 15.0 / 24.0 * 100.0).round() / 100.0;
        assert_eq!(pt, 15.0);
        assert_eq!(pt * 24.0 / 15.0, 24.0);
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
    fn 値だけの貼り付けで式が値になる() {
        let mut s = table();
        recalc(&mut s);
        // B2(=A2&"円")を控えて、値だけを B4 へ
        let cells = vec![vec![s.get(Pos::new(1, 1)).cloned()]];
        paste_values_cells(&mut s, Pos::new(3, 1), &cells);
        let c = s.get(Pos::new(3, 1)).unwrap();
        assert!(c.formula.is_none(), "式が残っている");
        assert_eq!(c.value, Value::Text("甲円".into()), "計算結果の値になっていない");
    }

    #[test]
    fn 外来の式もどきは文字として貼る() {
        let mut s = sheet::Sheet { name: "表".into(), ..Default::default() };
        paste_values_text(&mut s, Pos::new(0, 0), &[vec!["=A1*2".to_string()]]);
        let c = s.get(Pos::new(0, 0)).unwrap();
        assert!(c.formula.is_none(), "外の式を黙って式にした");
        assert_eq!(c.value, Value::Text("=A1*2".into()));
    }

    #[test]
    fn 書式だけの貼り付けで中身は残る() {
        let mut s = sheet::Sheet { name: "枠".into(), ..Default::default() };
        s.set(Pos::new(0, 0), Cell::input("100"));
        let src = Some(Cell {
            formula: None,
            value: Value::Empty,
            fmt: CellFormat { borders: Borders::ALL, ..Default::default() },
        });
        paste_formats(&mut s, Pos::new(0, 0), &[vec![src]]);
        let c = s.get(Pos::new(0, 0)).unwrap();
        assert_eq!(c.value, Value::Number(100.0), "書式だけのはずが中身が消えた");
        assert_eq!(c.fmt.borders, Borders::ALL, "書式が写っていない");
    }

    #[test]
    fn 転置で行と列が入れ替わる() {
        let g = vec![
            vec!["a".to_string(), "b".into(), "c".into()],
            vec!["1".to_string(), "2".into()],
        ];
        let t = transpose(&g);
        assert_eq!(t.len(), 3, "列の数が行にならない");
        assert_eq!(t[0], vec!["a".to_string(), "1".into()]);
        assert_eq!(t[2], vec!["c".to_string(), "".into()], "歯抜けが埋まらない");
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

#[cfg(test)]
mod paper_tests {
    use super::*;

    #[test]
    fn 用紙コードはjisのbで引く() {
        assert_eq!(paper_mm(9), Some((210.0, 297.0, "A4")));
        assert_eq!(paper_mm(12), Some((257.0, 364.0, "B4")), "B4 は JIS の紙");
        assert_eq!(paper_mm(99), None, "知らないコードを黙って A4 にしない");
    }
}

#[cfg(test)]
mod index_at_tests {
    use super::*;

    #[test]
    fn 位置から列が引ける() {
        let cols = [(0u32, 108.0f32), (1, 54.0), (2, 108.0)];
        assert_eq!(index_at(&cols, HEAD_W, HEAD_W + 1.0), Some(0));
        assert_eq!(index_at(&cols, HEAD_W, HEAD_W + 107.9), Some(0));
        assert_eq!(index_at(&cols, HEAD_W, HEAD_W + 108.0), Some(1), "境界は次の区分");
        assert_eq!(index_at(&cols, HEAD_W, HEAD_W + 200.0), Some(2));
        assert_eq!(index_at(&cols, HEAD_W, HEAD_W + 400.0), None, "並びの外");
        assert_eq!(index_at(&cols, HEAD_W, 10.0), None, "start より手前");
    }
}

#[cfg(test)]
mod goal_seek_tests {
    use super::*;

    #[test]
    fn 合計を目標に数量が逆算できる() {
        // 見本の表: D2=B2*C2, D4=SUM, D6=D4+D5(消費税は固定にして単純化)
        let mut s = sheet::Sheet { name: "表".into(), ..Default::default() };
        s.set(Pos::parse("B2").unwrap(), Cell::input("4"));
        s.set(Pos::parse("C2").unwrap(), Cell::input("125000"));
        s.set(Pos::parse("D2").unwrap(), Cell::input("=B2*C2"));
        recalc(&mut s);
        // D2 を 800000 にする B2 は 6.4
        let x = solve_goal(&s, Pos::parse("D2").unwrap(), 800000.0, Pos::parse("B2").unwrap())
            .expect("見つからない");
        assert!((x - 6.4).abs() < 1e-6, "6.4 のはず: {x}");
        // 効かないセルでは正直に None
        assert!(
            solve_goal(&s, Pos::parse("D2").unwrap(), 800000.0, Pos::parse("F9").unwrap())
                .is_none(),
            "効かないセルで見つかったことにした"
        );
    }
}

#[cfg(test)]
mod lock_tests {
    use super::*;

    #[test]
    fn 先客のロックが見え_自分のは先客に数えない() {
        let dir = std::env::temp_dir().join(format!("jo-lock-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let book = dir.join("台帳.xlsx");
        std::fs::write(&book, b"x").unwrap();
        let lp = lock_path_for(&book);
        assert!(lp.file_name().unwrap().to_string_lossy().starts_with(".~lock.台帳"));
        // 誰も居ない
        assert!(foreign_lock(&book).is_none());
        // 先客
        std::fs::write(&lp, "yamada@jimusho,;").unwrap();
        assert_eq!(foreign_lock(&book).as_deref(), Some("yamada@jimusho"));
        // 自分のロックは先客ではない
        std::fs::write(&lp, format!("{},;", lock_identity())).unwrap();
        assert!(foreign_lock(&book).is_none(), "自分を先客と間違えた");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod udf_tests {
    use super::*;

    #[test]
    fn 台本の出力が解けてスピルが効く() {
        // 出力形式: セル \x1e 行 \x1e 行 … / 行の中は \x1f
        let raw = "B2\u{1e}10\u{1f}20\u{1e}30\u{1f}40\u{1c}D1\u{1e}こんにちは";
        let results = parse_udf_output(raw);
        assert_eq!(results.len(), 2);
        let mut sh = sheet::Sheet { name: "表".into(), ..Default::default() };
        let mut py = Cell::input("=PY(\"f\",A1)");
        py.value = sheet::Value::Error("#PY?".into());
        sh.set(Pos::parse("B2").unwrap(), py);
        let (spills, n, c) = apply_py_results(&mut sh, &results, &Default::default());
        assert_eq!((n, c), (2, 0));
        // 錨は式を保ったまま値が入る
        let b2 = sh.get(Pos::parse("B2").unwrap()).unwrap();
        assert!(b2.formula.is_some(), "式が消えた");
        assert_eq!(b2.value, sheet::Value::Number(10.0));
        // スピル面
        assert_eq!(sh.value(Pos::parse("C3").unwrap()), sheet::Value::Number(40.0));
        assert_eq!(spills.get(&Pos::parse("B2").unwrap()), Some(&(2, 2)));
        assert_eq!(sh.value(Pos::parse("D1").unwrap()), sheet::Value::Text("こんにちは".into()));
    }

    #[test]
    fn スピル先に他人のデータがあれば止まる() {
        let mut sh = sheet::Sheet { name: "表".into(), ..Default::default() };
        sh.set(Pos::parse("B2").unwrap(), Cell::input("=PY(\"f\")"));
        sh.set(Pos::parse("C3").unwrap(), Cell::input("大事なメモ"));
        let raw = "B2\u{1e}1\u{1f}2\u{1e}3\u{1f}4";
        let (spills, n, c) =
            apply_py_results(&mut sh, &parse_udf_output(raw), &Default::default());
        assert_eq!((n, c), (0, 1));
        assert_eq!(
            sh.value(Pos::parse("B2").unwrap()),
            sheet::Value::Error("#SPILL!".into())
        );
        assert_eq!(
            sh.value(Pos::parse("C3").unwrap()),
            sheet::Value::Text("大事なメモ".into()),
            "他人のデータを潰した"
        );
        assert!(spills.is_empty());
    }

    #[test]
    fn 縮んだスピルの残骸は消える() {
        let mut sh = sheet::Sheet { name: "表".into(), ..Default::default() };
        sh.set(Pos::parse("A1").unwrap(), Cell::input("=PY(\"f\")"));
        // 前回 1x3 で展開していた
        sh.set(Pos::parse("B1").unwrap(), Cell::input("古い"));
        sh.set(Pos::parse("C1").unwrap(), Cell::input("残骸"));
        let mut prev = std::collections::HashMap::new();
        prev.insert(Pos::parse("A1").unwrap(), (1u32, 3u32));
        // 今回はスカラー
        let raw = "A1\u{1e}9";
        let (_, n, c) = apply_py_results(&mut sh, &parse_udf_output(raw), &prev);
        assert_eq!((n, c), (1, 0));
        assert_eq!(sh.value(Pos::parse("A1").unwrap()), sheet::Value::Number(9.0));
        assert!(sh.value(Pos::parse("C1").unwrap()).is_empty(), "残骸が残った");
    }

    #[test]
    fn 台本が実際にpythonで回る() {
        // .venv が無い機械では黙って飛ぶ(HIKITSUGI の作法)。
        // cargo test の cwd は calc/ なので、リポジトリ直下の .venv も見る
        let py = ["../.venv/bin/python", ".venv/bin/python"]
            .iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.exists());
        let Some(py) = py else { return };
        let dir = std::env::temp_dir().join(format!("jo-udf-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let out = dir.join("out.txt");
        let defs = "def 倍(x):\n    return x * 2\ndef 表(r):\n    return [[v * 10 for v in row] for row in r]";
        let calls = vec![
            (
                "B1".to_string(),
                "倍".to_string(),
                vec![sheet::calc::PyArg::One(sheet::Value::Number(21.0))],
            ),
            (
                "D1".to_string(),
                "表".to_string(),
                vec![sheet::calc::PyArg::Rect(
                    2,
                    vec![
                        sheet::Value::Number(1.0),
                        sheet::Value::Number(2.0),
                        sheet::Value::Number(3.0),
                        sheet::Value::Number(4.0),
                    ],
                )],
            ),
        ];
        let script = build_udf_script(defs, &calls, &out);
        let py_path = dir.join("t.py");
        std::fs::write(&py_path, script).unwrap();
        let o = std::process::Command::new(&py).arg(&py_path).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let raw = std::fs::read_to_string(&out).unwrap();
        let results = parse_udf_output(&raw);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].1[0][0], "42", "倍(21) が違う: {raw:?}");
        assert_eq!(results[1].1[1][1], "40", "表の2x2が違う");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

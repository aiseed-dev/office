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
use sheet::{recalc, recalc_book, Book, Cell, Pos, Value};
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

mod funcs;

/// 「関数を挿入」の小窓(本家の FormulaDialog と同じ形 —
/// 検索 / 分類 / 一覧 / 引数と説明 / OK・キャンセル)。
/// 一覧・引数・説明は funcs.rs(本家の日本語から生成。使える関数だけ)
struct FnDlg {
    search: Editor,
    /// FN_GROUPS の添字(0 = すべて)
    group: usize,
    /// 絞り込み後の一覧の中の選択
    sel: usize,
}

/// 分類の耳。「すべて」+ funcs.rs の分類
const FN_GROUPS: &[&str] = &["すべて", "数学", "統計", "文字列", "論理", "日付", "検索", "財務", "情報"];

/// 「関数の引数」の画面(本家の第2段)。引数ごとの欄と説明、結果の下見
struct FnArgs {
    f: &'static funcs::FnInfo,
    /// (引数名, 省略可)
    names: Vec<(String, bool)>,
    eds: Vec<Editor>,
    focus: usize,
    /// 関数の結果(引数を打つたびに、表の複製で計算した下見)
    result: String,
    /// セルの掴みの起点。ドラッグすると「起点:いま」の範囲が欄に入る
    pick_from: Option<Pos>,
}

/// 引数の書き方「(数値1, [数値2], ...)」を(名前, 省略可)の列に解く
fn parse_fn_args(spec: &str) -> Vec<(String, bool)> {
    spec.trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && *s != "...")
        .map(|s| {
            let opt = s.starts_with('[');
            (s.trim_start_matches('[').trim_end_matches(']').to_string(), opt)
        })
        .collect()
}

/// 検索と分類で絞った一覧(名前順は funcs.rs の並びのまま)
fn fn_filtered(search: &str, group: usize) -> Vec<&'static funcs::FnInfo> {
    let q = search.trim().to_uppercase();
    funcs::FUNCS
        .iter()
        .filter(|f| group == 0 || f.group == FN_GROUPS[group])
        .filter(|f| q.is_empty() || f.name.contains(&q))
        .collect()
}

/// ソルバーの小窓(ONLYOFFICE の「ソルバーのパラメータ」と同じ形)。
/// 解法は単体法 LP だけ — 本家と同じで、非線形は正直に断る。
struct Solver {
    /// 目的のセル
    target: Editor,
    /// 0=最大 1=最小 2=値
    mode: u8,
    /// mode=2 の目標の値
    value: Editor,
    /// 変数セル(範囲・カンマ区切り)
    vars: Editor,
    /// 決めた制約(左辺セル/範囲, 記号, 右辺)
    cons: Vec<(String, &'static str, String)>,
    /// 追加・変更中の制約の入力
    con_l: Editor,
    con_op: usize,
    con_r: Editor,
    /// 制約のない変数を非負にする
    nonneg: bool,
    /// 打鍵の宛先: 0=目的 1=値 2=変数 3=制約左 4=制約右
    focus: u8,
    /// 一覧で選んだ制約(変更・削除の相手)
    sel: Option<usize>,
}

impl Solver {
    fn new(target: &str) -> Self {
        Solver {
            target: Editor::new(target),
            mode: 0,
            value: Editor::new(""),
            vars: Editor::new(""),
            cons: Vec::new(),
            con_l: Editor::new(""),
            con_op: 0,
            con_r: Editor::new(""),
            nonneg: true,
            focus: 2, // まず変数セルを聞く(目的は選択から入っている)
            sel: None,
        }
    }
    fn focused(&mut self) -> &mut Editor {
        match self.focus {
            0 => &mut self.target,
            1 => &mut self.value,
            2 => &mut self.vars,
            3 => &mut self.con_l,
            _ => &mut self.con_r,
        }
    }
    fn focused_ref(&self) -> &Editor {
        match self.focus {
            0 => &self.target,
            1 => &self.value,
            2 => &self.vars,
            3 => &self.con_l,
            _ => &self.con_r,
        }
    }
}

const SOLVER_OPS: [&str; 3] = ["<=", "=", ">="];

/// SmartArt の一覧。**分類・並び・名前は Euro-Office の現物**
/// (web-apps の define.js の並びと ja.json の訳)から取った。
/// 載せるのは**うちの図形(SVG 方式)で組めるものだけ** —
/// できないものを、できるように見せない。
const SMARTART: &[(&str, &[(&str, &str)])] = &[
    ("リスト", &[
        ("カード型リスト", "block-list"),
        ("縦方向リスト", "vbox-list"),
        ("ピラミッドのリスト", "pyramid-list"),
    ]),
    ("プロセス", &[
        ("基本ステップ", "basic-process"),
        ("プロセス", "chevron-process"),
        ("タイムライン", "timeline"),
    ]),
    ("循環", &[
        ("基本の循環", "basic-cycle"),
        ("ボックス循環", "block-cycle"),
    ]),
    ("階層", &[
        ("組織図", "org-chart"),
        ("階層", "hierarchy"),
    ]),
    ("関係", &[("基本ベン図", "venn")]),
    ("マトリックス", &[("基本マトリックス", "matrix")]),
    ("ピラミッド", &[("基本ピラミッド", "pyramid")]),
];

/// セル・範囲の列挙を読む(A1 / B2:B5 / $A$1。カンマ・読点・空白区切り)。
/// 範囲は左上→右下に展開する。読めない・大きすぎるときは None。
fn parse_cell_list(text: &str, cap: usize) -> Option<Vec<Pos>> {
    let mut out = Vec::new();
    // $ の絶対参照の印は捨て、小文字も受ける(Excel と同じく区別しない)
    for tok in split_fields(&text.replace('$', "").to_uppercase()) {
        if let Some((a, b)) = tok.split_once(':') {
            let (a, b) = (Pos::parse(a.trim())?, Pos::parse(b.trim())?);
            let (r0, r1) = (a.row.min(b.row), a.row.max(b.row));
            let (c0, c1) = (a.col.min(b.col), a.col.max(b.col));
            for r in r0..=r1 {
                for c in c0..=c1 {
                    out.push(Pos::new(r, c));
                    if out.len() > cap {
                        return None;
                    }
                }
            }
        } else {
            out.push(Pos::parse(tok.trim())?);
            if out.len() > cap {
                return None;
            }
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

/// ピボットの聞き取りの途中経過。板を3枚続けて使う間の控え
/// (行に並べる欄 → 列に広げる欄 → 値と集計、の順に聞く)。
struct PivotPend {
    a: Pos,
    b: Pos,
    headers: Vec<String>,
    rows_sel: Vec<String>,
    cols_sel: Vec<String>,
}

/// 見出しの列挙を割る(カンマ・読点・セミコロン・空白のどれでも。
/// ; も受けるのは日本語配列で : が ; に化けやすいため)。
fn split_fields(text: &str) -> Vec<String> {
    text.split(|c: char| matches!(c, ',' | '、' | ';' | '；') || c.is_whitespace())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

const PIVOT_AGGS: [&str; 5] = ["合計", "平均", "個数", "最大", "最小"];

/// 「金額 合計」を(見出し, 集計)に読む。集計を省けば合計。
fn parse_pivot_val(text: &str, headers: &[String]) -> Result<(String, &'static str), String> {
    let mut parts = split_fields(text);
    let agg = match parts.last().map(|s| s.as_str()) {
        Some(last) => match PIVOT_AGGS.iter().find(|a| **a == last) {
            Some(a) => {
                parts.pop();
                *a
            }
            None => "合計",
        },
        None => "合計",
    };
    let name = parts.join(" ");
    if name.is_empty() {
        return Err(ui::t!("値にする見出しを書いてください(例: 金額 合計)").into());
    }
    if !headers.iter().any(|h| *h == name) {
        return Err(ui::tf!("「{}」は見出しにありません", name));
    }
    Ok((name, agg))
}

/// ピボットの指図を JSON にする(手で組む — グラフと同じ割り切り)。
fn pivot_spec_json(headers: &[String], rows: &[Vec<String>], d: &sheet::model::PivotDef) -> String {
    let esc = |t: &str| t.replace('\\', "\\\\").replace('"', "\\\"");
    let strs = |xs: &[String]| {
        xs.iter().map(|x| format!("\"{}\"", esc(x))).collect::<Vec<_>>().join(",")
    };
    format!(
        "{{\"headers\":[{}],\"rows\":[{}],\"index\":[{}],\"columns\":[{}],\"value\":\"{}\",\"agg\":\"{}\",\"totals\":{},\"subtotals\":{},\"blank_rows\":{},\"compact\":{}}}",
        strs(headers),
        rows.iter().map(|r| format!("[{}]", strs(r))).collect::<Vec<_>>().join(","),
        strs(&d.rows_sel),
        strs(&d.cols_sel),
        esc(&d.value),
        esc(&d.agg),
        d.totals,
        d.subtotals,
        d.blank_rows,
        d.compact,
    )
}

/// ピボットの台本の答えを読む。各行の1欄目は種別
/// (h=見出し d=データ s=小計 b=空行 t=総計)、残りが欄。
fn parse_pivot_grid(raw: &str) -> (Vec<Vec<String>>, Vec<char>) {
    let mut grid = Vec::new();
    let mut kinds = Vec::new();
    for line in raw.split('\u{1e}') {
        let mut it = line.split('\u{1f}');
        let kind = it.next().and_then(|k| k.chars().next()).unwrap_or('d');
        grid.push(it.map(|v| v.to_string()).collect());
        kinds.push(kind);
    }
    (grid, kinds)
}

/// 表のデザインの「合計行」。選択の下の行に、数の列へ =SUM(…) を入れて
/// 太字+上罫線にする。1行目が見出し(文字)なら合計の範囲から外す。
/// 文字の列の先頭には「合計」の札。書いた欄の数を返す。
fn add_total_row(s: &mut sheet::Sheet, a: Pos, b: Pos) -> usize {
    let header = (a.col..=b.col).any(|c| {
        matches!(s.get(Pos::new(a.row, c)).map(|x| &x.value), Some(Value::Text(_)))
    });
    let from = if header && b.row > a.row { a.row + 1 } else { a.row };
    let total = b.row + 1;
    let mut n = 0usize;
    for c in a.col..=b.col {
        let numeric = (from..=b.row).any(|r| {
            matches!(s.get(Pos::new(r, c)).map(|x| &x.value), Some(Value::Number(_)))
        });
        let p = Pos::new(total, c);
        let fmt0 = s.get(p).map(|x| x.fmt.clone()).unwrap_or_default();
        let mut cell = if numeric {
            Cell::input(&format!(
                "=SUM({}:{})",
                Pos::new(from, c).a1(),
                Pos::new(b.row, c).a1()
            ))
        } else if c == a.col {
            Cell::input("合計")
        } else {
            s.get(p).cloned().unwrap_or_default()
        };
        cell.fmt = fmt0;
        cell.fmt.bold = true;
        cell.fmt.borders.top = true;
        s.set(p, cell);
        n += 1;
    }
    n
}

/// データタブの「小計」(Excel の集計)。基準の列の値が変わる区切りごとに
/// 「〜 小計」の行(=SUM)を挿し、明細にグループ化(深さ1)を掛け、最後に
/// 総計の行を足す。**小計・総計の行はグループ化しない** — 詳細を畳んでも
/// 合計は見えたまま残る(発注者指摘 2026-08-04)。挿した式は最終の座標で
/// 書き、既存の式は insert_row が直す。返り値は区切りの数。
fn apply_subtotals(s: &mut sheet::Sheet, a: Pos, b: Pos, by: u32, vals: &[u32]) -> usize {
    // 区切り = 基準の列で連続する同じ値の並び(Excel と同じく、並べ替えは
    // 済んでいる前提。飛び飛びなら区切りもその数だけできる)
    let mut runs: Vec<(u32, u32, String)> = Vec::new();
    for r in a.row + 1..=b.row {
        let v = s.get(Pos::new(r, by)).map(|c| c.value.display()).unwrap_or_default();
        match runs.last_mut() {
            Some((_, end, label)) if *label == v => *end = r,
            _ => runs.push((r, r, v)),
        }
    }
    if runs.is_empty() {
        return 0;
    }
    // 枠を下から挿す(上の位置が狂わない): 総計の枠 → 各区切りの小計の枠
    s.insert_row(b.row + 1);
    for (_, end, _) in runs.iter().rev() {
        s.insert_row(end + 1);
    }
    // 中身は最終の座標で書く: k 番目の区切りの小計行 = end+1+k、
    // その明細は k 行ぶん下がっている。総計 = b.row+1+区切りの数
    let style = |s: &mut sheet::Sheet, p: Pos, text: &str| {
        let fmt0 = s.get(p).map(|c| c.fmt.clone()).unwrap_or_default();
        let mut cell = Cell::input(text);
        cell.fmt = fmt0;
        cell.fmt.bold = true;
        cell.fmt.borders.top = true;
        s.set(p, cell);
    };
    let mut sub_rows = Vec::new();
    for (k, (start, end, label)) in runs.iter().enumerate() {
        let k = k as u32;
        let (det0, det1, srow) = (start + k, end + k, end + 1 + k);
        sub_rows.push(srow);
        style(s, Pos::new(srow, by), &ui::tf!("{} 小計", label));
        for c in vals {
            style(
                s,
                Pos::new(srow, *c),
                &format!("=SUM({}:{})", Pos::new(det0, *c).a1(), Pos::new(det1, *c).a1()),
            );
        }
        for r in det0..=det1 {
            s.row_outline.insert(r, 1);
        }
    }
    let trow = b.row + 1 + runs.len() as u32;
    style(s, Pos::new(trow, by), "総計");
    for c in vals {
        let refs: Vec<String> = sub_rows.iter().map(|r| Pos::new(*r, *c).a1()).collect();
        style(s, Pos::new(trow, *c), &format!("={}", refs.join("+")));
    }
    runs.len()
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
    /// ピボットの聞き取りの途中経過(元の範囲・見出し・決めた欄)
    pivot_pend: Option<PivotPend>,
    /// 小計の聞き取りの途中経過(同じ形の控えを使い回す)
    sub_pend: Option<PivotPend>,
    /// ソルバーの小窓(開いている間、打鍵は選んだ欄へ)
    solver: Option<Solver>,
    /// SmartArt の選択中の分類(2段の pick の1段目の答え)
    sa_cat: usize,
    /// スライサー(列, 選んだ値たち, 複数選択か)。**見え方だけ** —
    /// 絞り込みと同じで、保存される中身は変わらない
    slicer: Option<(u32, std::collections::BTreeSet<String>, bool)>,
    /// コメントを見せるか(共同編集タブで切替。隠しても付いたまま)
    show_comments: bool,
    /// 暗号化のパスワード(次の保存から効く。開いた暗号化ブックからも入る)
    encrypt_pw: Option<String>,
    /// 「開くために聞いている」パスワード待ちのファイル
    pw_pending: Option<PathBuf>,
    /// pick の一覧が指す実体(バージョン履歴・プラグインの表示名 → パス)
    pick_paths: Vec<(String, PathBuf)>,
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
    /// 窓の大きさ(px)。描画のたびに実測 — **見える範囲**の計算に使う。
    /// セルの大きさは設定どおり固定で、窓に合わせて伸縮させない
    view_w_px: f32,
    view_h_px: f32,
    /// このセルで**編集を始めた**(F2・ダブルクリック・打ち始め)。
    /// 立っていない間の最初の打鍵は、既存の中身を消して置き換える
    /// (Excel の作法)。セルを移ると降りる(sync_input)
    edit_armed: bool,
    /// 名前ボックスの打ちかけ(数式バーの左端)。番地・範囲・名前で飛び、
    /// 知らない名前なら**いまの選択に付ける**(Excel の名前ボックスと同じ)
    name_edit: Option<Editor>,
    /// 「関数を挿入」の小窓(検索・分類・一覧・説明)
    fn_dlg: Option<FnDlg>,
    /// 「関数の引数」の画面(次へ、で進む第2段)
    fn_args: Option<FnArgs>,
    /// 式の直入力中のセル掴み(起点, 入れた参照の文字の範囲)。
    /// クリックで参照がカーソルに入り、ドラッグで範囲(A1:C9)に伸びる
    ref_pick: Option<(Pos, std::ops::Range<usize>)>,
    /// 終了確認の板(未保存の変更があるときに出る。窓の中の中央)
    quit_ask: bool,
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
    /// ファイルの全面ページから「戻る」ときのタブ
    prev_tab: usize,
    /// 釦に乗っているときの名前(下のステータスバーに出す)
    hover_hint: Option<&'static str>,
    /// ファイルのページの右側(0=詳細情報 1=最近開いた)
    file_view: u8,
    /// 表示の倍率(表示タブのズーム。0.5〜2.0)
    zoom: f32,
    /// 数式バーを見せるか(表示タブ)
    show_formula_bar: bool,
    /// 行番号・列名の見出しを見せるか(表示タブ)
    show_headers: bool,
    /// 0 の値を見せるか(表示タブ。消しても値は 0 のまま)
    show_zeros: bool,
    /// 画面を暗くする(インターフェイステーマ)。**セルは白のまま** —
    /// 画面と紙の一致を守る(writer の「紙は白のまま」と同じ考え)
    dark: bool,
    /// 自動で再計算するか(数式タブの「計算方法」。手動のときは F9)
    auto_calc: bool,
    /// 見張り(ウォッチウィンドウ)。(シート番号, セル)
    watch: Vec<(usize, Pos)>,
    /// AI に頼み中(終わるまで次の頼みは断る)
    ai_busy: bool,
    /// 描画の道具(0=ペン 1=蛍光ペン 2=消しゴム)。writer と同じ形
    tool: Option<u8>,
    /// 描きかけの線(ドラッグ中)
    ink_cur: Option<Vec<(f32, f32)>>,
}

impl HasEditor for Calc {
    // 小さな入力の板(名前の定義など)・ソルバーの小窓が開いている間は、
    // 打鍵(IME含む)はそこへ
    fn editor(&mut self) -> &mut Editor {
        if let Some(ed) = &mut self.name_edit {
            return ed;
        }
        if let Some(a) = &mut self.fn_args {
            if !a.eds.is_empty() {
                let i = a.focus.min(a.eds.len() - 1);
                return &mut a.eds[i];
            }
        }
        if let Some(d) = &mut self.fn_dlg {
            return &mut d.search;
        }
        if let Some(sv) = &mut self.solver {
            return sv.focused();
        }
        match &mut self.prompt {
            Some((_, ed)) => ed,
            None => &mut self.input,
        }
    }
    fn editor_ref(&self) -> &Editor {
        if let Some(ed) = &self.name_edit {
            return ed;
        }
        if let Some(a) = &self.fn_args {
            if !a.eds.is_empty() {
                let i = a.focus.min(a.eds.len() - 1);
                return &a.eds[i];
            }
        }
        if let Some(d) = &self.fn_dlg {
            return &d.search;
        }
        if let Some(sv) = &self.solver {
            return sv.focused_ref();
        }
        match &self.prompt {
            Some((_, ed)) => ed,
            None => &self.input,
        }
    }
    fn on_edited(&mut self) {
        // 検索を打ち替えたら一覧の選択は先頭に戻す
        if let Some(d) = &mut self.fn_dlg {
            d.sel = 0;
        }
        // 引数を打ち替えたら結果の下見を計算し直す
        if self.fn_args.is_some() {
            self.fn_args_recalc();
        }
        // 板・小窓・名前ボックスへの打鍵は文書を変えない
        if self.prompt.is_none() && self.name_edit.is_none()
            && self.fn_dlg.is_none() && self.fn_args.is_none()
        {
            self.dirty = true;
            // 式の直入力の支援: 打ちかけの関数名の補完一覧と、引数のヒント
            self.formula_assist();
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
            pivot_pend: None,
            sub_pend: None,
            solver: None,
            sa_cat: 0,
            slicer: None,
            show_comments: true,
            pick_paths: Vec::new(),
            encrypt_pw: None,
            pw_pending: None,
            goal: None,
            py_spills: Default::default(),
            trace: Vec::new(),
            my_lock: None,
            locked_by: None,
            shape_sel: None,
            shape_drag: None,
            wheel: (0.0, 0.0),
            view_w_px: 0.0,
            view_h_px: 0.0,
            edit_armed: false,
            name_edit: None,
            fn_dlg: None,
            fn_args: None,
            ref_pick: None,
            quit_ask: false,
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
            tab: 1, // ファイルは全面ページになったので、開きはホーム
            prev_tab: 1,
            hover_hint: None,
            file_view: 0,
            zoom: 1.0,
            show_formula_bar: true,
            show_headers: true,
            show_zeros: true,
            dark: false,
            auto_calc: true,
            watch: Vec::new(),
            ai_busy: false,
            tool: None,
            ink_cur: None,
        };
        if let Some(p) = path {
            c.open(p);
        } else {
            // 新規は空白のブック(発注者 2026-08-06。見本を入れない —
            // 試験は自前で表を作り、触れる見本は sample/*.xlsx にある)
            c.status = ui::t!("セルを選んで打つ。Enter で確定して下へ、Ctrl+S で保存").into();
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
        self.edit_armed = false; // セルを移った=編集は仕切り直し
        if self.pick_kind == "fn-complete" {
            self.pick = None; // 補完の一覧も畳む
        }
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
            self.status = ui::t!("戻すものがありません").into();
            return;
        };
        let mut redo = Vec::new();
        let first = batch.first().map(|(i, _)| *i);
        for (idx, prev) in batch {
            if idx < self.book.sheets.len() {
                redo.push((idx, self.book.sheets[idx].clone()));
                self.book.sheets[idx] = prev;
                recalc_book(&mut self.book, idx);
            }
        }
        self.redo_stack.push(redo);
        if let Some(i) = first {
            self.show_sheet(i);
        }
        self.dirty = true;
        self.sync_input();
        self.status = ui::t!("戻しました").into();
    }

    fn redo_sheet(&mut self) {
        let Some(batch) = self.redo_stack.pop() else {
            self.status = ui::t!("やり直すものがありません").into();
            return;
        };
        let mut undo = Vec::new();
        let first = batch.first().map(|(i, _)| *i);
        for (idx, next) in batch {
            if idx < self.book.sheets.len() {
                undo.push((idx, self.book.sheets[idx].clone()));
                self.book.sheets[idx] = next;
                recalc_book(&mut self.book, idx);
            }
        }
        self.undo_stack.push(undo);
        if let Some(i) = first {
            self.show_sheet(i);
        }
        self.dirty = true;
        self.sync_input();
        self.status = ui::t!("やり直しました").into();
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

    /// 画面に出ている行の並び(絞り込み中はその行だけ。グループ化で畳んだ行は
    /// 飛ばす)。描画と当たり判定で共有する。
    /// スライサーで残る行か(選びが空なら全部残る)。1行目=見出しは常に残す。
    fn slicer_keeps(&self, r: u32) -> bool {
        let Some((col, sel, _)) = &self.slicer else { return true };
        if sel.is_empty() || r == 0 {
            return true;
        }
        let v = self
            .sheet()
            .get(Pos::new(r, *col))
            .map(|c| c.value.display())
            .unwrap_or_default();
        let v = if v.is_empty() { ui::t!("(空白)").to_string() } else { v };
        sel.contains(&v)
    }

    /// 窓に入る行数。**セルの大きさは固定**で、窓が大きいほど多くの行が
    /// 見える(発注者 2026-08-06)。まだ窓の大きさを知らない(描画前・試験)
    /// なら従来の既定。少し多めに数えても、はみ出しは器が刈る
    fn rows_fit(&self) -> u32 {
        self.rows_fit_in(self.view_h_px)
    }

    fn rows_fit_in(&self, budget: f32) -> u32 {
        if self.view_h_px <= 0.0 {
            return ROWS; // 描画前・試験は従来の既定
        }
        let (mut h, mut n, mut r) = (0.0f32, 0u32, self.view.row);
        while h < budget && n < 300 {
            h += self.row_px(r);
            r += 1;
            n += 1;
        }
        n.max(3)
    }

    /// 端の追従・ページ移動用: 額縁(リボン・数式バー・耳・状態行)を
    /// 差し引いた「確実に丸ごと見える」行数
    fn rows_snug(&self) -> u32 {
        self.rows_fit_in(self.view_h_px - 270.0)
    }

    /// 窓に入る列数(rows_fit と同じ役割)
    fn cols_fit(&self) -> u32 {
        self.cols_fit_in(self.view_w_px)
    }

    fn cols_fit_in(&self, budget: f32) -> u32 {
        if self.view_w_px <= 0.0 {
            return COLS;
        }
        let (mut w, mut n, mut c) = (0.0f32, 0u32, self.view.col);
        while w < budget && n < 120 {
            w += self.col_px(c);
            c += 1;
            n += 1;
        }
        n.max(2)
    }

    fn cols_snug(&self) -> u32 {
        self.cols_fit_in(self.view_w_px - HEAD_W - 24.0)
    }

    fn visible_rows(&self) -> Vec<u32> {
        let hidden = &self.sheet().row_hidden;
        let fit = self.rows_fit();
        match &self.filter {
            Some((col, v)) => self
                .matching_rows(*col, v)
                .into_iter()
                .filter(|r| !hidden.contains(r) && self.slicer_keeps(*r))
                .take(fit as usize)
                .collect(),
            None if self.slicer.as_ref().is_some_and(|(_, sel, _)| !sel.is_empty()) => {
                // スライサーで絞る: 見出し+選んだ値の行(絞り込みと同じ流儀)
                let (rows, _) = self.sheet().extent();
                (0..rows)
                    .filter(|r| !hidden.contains(r) && self.slicer_keeps(*r))
                    .take(fit as usize)
                    .collect()
            }
            None => {
                // 畳んだ行のぶん多めに見て、画面の行数まで詰める
                let extra = hidden.len() as u32;
                grid_rows(self.frozen, self.view, fit + extra)
                    .into_iter()
                    .filter(|r| !hidden.contains(r))
                    .take(fit as usize)
                    .collect()
            }
        }
    }

    /// 画面に出ている列の並び(畳んだ列は飛ばす)。visible_rows と同じ役割。
    fn visible_cols(&self) -> Vec<u32> {
        let hidden = &self.sheet().col_hidden;
        let extra = hidden.len() as u32;
        let fit = self.cols_fit();
        let mut v: Vec<u32> = grid_cols(self.frozen, self.view, fit + extra)
            .into_iter()
            .filter(|c| !hidden.contains(c))
            .take(fit as usize)
            .collect();
        if self.sheet().rtl {
            // 右から左のシートは列を逆順に並べる。**描画も当たり判定も
            // この一点を通る**ので、掴む場所と見える場所がずれない
            v.reverse();
        }
        v
    }

    /// 格子の中の位置(px、格子領域の左上原点)からセルを逆算する。
    /// 見出しの帯の上なら None。
    fn cell_at(&self, x: f32, y: f32) -> Option<Pos> {
        if x < self.head_w() || y < self.head_h() {
            return None;
        }
        Some(Pos { row: self.row_at(y)?, col: self.col_at(x)? })
    }

    /// この x はどの列の上か(見出し・セルのどちらでも)。
    fn col_at(&self, x: f32) -> Option<u32> {
        let cols: Vec<(u32, f32)> = self.visible_cols()
            .into_iter()
            .map(|c| (c, self.col_px(c)))
            .collect();
        index_at(&cols, self.head_w(), x)
    }

    fn row_at(&self, y: f32) -> Option<u32> {
        let rows: Vec<(u32, f32)> = self
            .visible_rows()
            .into_iter()
            .map(|r| (r, self.row_px(r)))
            .collect();
        index_at(&rows, self.head_h(), y)
    }

    /// 列をまるごと選ぶ(使われている高さまで)。`a` が起点、`b` が動く側。
    fn select_cols(&mut self, a: u32, b: u32) {
        let rows = self.sheet().extent().0.max(1);
        self.anchor = Some(Pos::new(rows - 1, a));
        self.cursor = Pos::new(0, b);
        self.sync_input();
        let (lo, hi) = (a.min(b), a.max(b));
        self.status = if lo == hi {
            ui::tf!("{}列を選択しました(1〜{}行)", col_name(lo), rows).into()
        } else {
            ui::tf!("{}〜{}列を選択しました(1〜{}行)", col_name(lo), col_name(hi), rows).into()
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
            ui::tf!("{}行を選択しました", lo + 1).into()
        } else {
            ui::tf!("{}〜{}行を選択しました", lo + 1, hi + 1).into()
        };
    }

    /// 見出しの帯の上の、列幅・行高の取っ手(境界 ±GRIP px)。Some((列か, 番号))。
    /// 描画・cell_at と同じ並び(固定・窓・絞り込み)を使う —
    /// ずれると別の境界を掴んでしまう。
    fn size_grip_at(&self, x: f32, y: f32) -> Option<(bool, u32)> {
        if !self.show_headers {
            return None; // 見出しが無ければ掴む縁も無い
        }
        if y < ROW_H && x >= HEAD_W {
            let cols: Vec<(u32, f32)> = self.visible_cols()
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
            self.status = ui::tf!("{}列の幅: {}({:.0}px)", col_name(idx), w, w * PX_PER_CHW)
            .into();
        } else {
            let pt = ((base + y - grab) / self.zoom).max(6.0) * 15.0 / 24.0;
            let pt = (pt * 100.0).round() / 100.0;
            self.sheet_mut().row_height.insert(idx, pt);
            self.status = ui::tf!("{}行の高さ: {}pt({:.0}px)", idx + 1, pt, pt * 24.0 / 15.0)
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
        // 描画の道具が出ていれば筆が最優先(セルは触らない)
        if let Some(t) = self.tool {
            if x >= self.head_w() && y >= self.head_h() {
                if t == 2 {
                    // 消しゴム: なぞった線を1筆消す
                    match self.ink_at(x, y) {
                        Some(i) => {
                            self.checkpoint();
                            self.sheet_mut().shapes_new.remove(i);
                            self.dirty = true;
                            self.status = ui::t!("1筆消しました(Ctrl+Z で戻せます)").into();
                        }
                        None => self.status = ui::t!("線の上をなぞってください").into(),
                    }
                } else {
                    self.ink_cur = Some(vec![(x, y)]);
                }
                return;
            }
        }
        // 浮いている図形が最優先(セルの上に描かれているので)
        if let Some((i, (sx, sy), corner)) = self.shape_at(x, y) {
            self.commit();
            self.checkpoint();
            self.shape_sel = Some(i);
            self.shape_drag = Some((i, (x, y), if corner { (sx, sy) } else { (sx, sy) }, corner));
            self.status = if corner {
                ui::t!("右下を引いて大きさを変えます").into()
            } else {
                ui::t!("図形を選びました(ドラッグで移動 / 右下で大きさ / Del で削除)").into()
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
                self.status = ui::tf!("A1:{} を選択しました", self.cursor.a1()).into();
            }
            return;
        }
        let Some(p) = self.cell_at(x, y) else { return };
        // 関数の引数の画面が開いている間は、セルのクリックで
        // **いまの欄に参照が入る**。そのままドラッグすると範囲(A1:C9)になる
        if self.fn_args.is_some() {
            let a1 = p.a1();
            if let Some(a) = &mut self.fn_args {
                if a.eds.is_empty() {
                    return;
                }
                let i = a.focus.min(a.eds.len() - 1);
                a.eds[i] = Editor::new(&a1);
                a.eds[i].move_to(a1.len(), false);
                a.pick_from = Some(p);
            }
            self.fn_args_recalc();
            return;
        }
        // 式の直入力中は、セルのクリックで**参照がカーソルに入る**(Excel の
        // 作法)。入るのは参照を待つ場所(= ( , 演算子の直後)のときだけ —
        // それ以外の場所でのクリックは、従来どおり確定して移動
        if (self.editing() || self.edit_armed) && self.input.text().starts_with('=') {
            let t = self.input.text().to_string();
            let cur = self.input.cursor().min(t.len());
            let prev = t[..cur].trim_end().chars().last();
            if matches!(
                prev,
                Some('=' | '(' | ',' | '+' | '-' | '*' | '/' | ':' | '^' | '&' | '<' | '>' | '%')
            ) {
                let a1 = p.a1();
                self.input.insert(&a1);
                let end = self.input.cursor();
                self.ref_pick = Some((p, end - a1.len()..end));
                return;
            }
        }
        // Ctrl+クリックはリンクを開く(基幹網の外は既定のブラウザに任せる)
        if ctrl && !shift {
            if let Some(url) = self.sheet().links.get(&p).cloned() {
                let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
                self.status = ui::tf!("開きます: {}", url).into();
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
        // ダブルクリックはその場で編集(次の打鍵が追記になる — Excel の作法)
        if clicks >= 2 {
            self.edit_armed = true;
            self.input.move_to(self.input.text().len(), false);
            self.status = ui::t!("編集: そのまま打つと続きに入ります(Esc で取消)").into();
        }
    }

    /// 押したまま動いた。通り過ぎたセルまで選択を広げる。
    fn mouse_drag_at(&mut self, x: f32, y: f32) {
        // 式の直入力のセル掴み: 入れた参照を「起点:いま」の範囲に置き換える
        if let Some((from, range)) = self.ref_pick.clone() {
            let Some(p) = self.cell_at(x, y) else { return };
            let (ra, rb) = (from.row.min(p.row), from.row.max(p.row));
            let (ca, cb) = (from.col.min(p.col), from.col.max(p.col));
            let text = if from == p {
                p.a1()
            } else {
                format!("{}:{}", Pos::new(ra, ca).a1(), Pos::new(rb, cb).a1())
            };
            let mut t = self.input.text().to_string();
            if range.end <= t.len() {
                t.replace_range(range.clone(), &text);
                self.input = Editor::new(&t);
                self.input.move_to(range.start + text.len(), false);
                self.ref_pick = Some((from, range.start..range.start + text.len()));
            }
            return;
        }
        // 関数の引数のセル掴み: なぞった範囲「起点:いま」を欄に入れる
        if self.fn_args.as_ref().is_some_and(|a| a.pick_from.is_some()) {
            let Some(p) = self.cell_at(x, y) else { return };
            if let Some(a) = &mut self.fn_args {
                let Some(from) = a.pick_from else { return };
                let i = a.focus.min(a.eds.len().saturating_sub(1));
                let (ra, rb) = (from.row.min(p.row), from.row.max(p.row));
                let (ca, cb) = (from.col.min(p.col), from.col.max(p.col));
                let text = if from == p {
                    p.a1()
                } else {
                    format!("{}:{}", Pos::new(ra, ca).a1(), Pos::new(rb, cb).a1())
                };
                a.eds[i] = Editor::new(&text);
                a.eds[i].move_to(text.len(), false);
            }
            self.fn_args_recalc();
            return;
        }
        if self.tool == Some(2) {
            // 消しゴムはなぞっている間ずっと効く
            if let Some(i) = self.ink_at(x, y) {
                self.checkpoint();
                self.sheet_mut().shapes_new.remove(i);
                self.dirty = true;
            }
            return;
        }
        if let Some(pts) = &mut self.ink_cur {
            // 近すぎる点は捨てる(点の数を抑える)
            let far = pts
                .last()
                .map(|(lx, ly)| (x - lx).abs() + (y - ly).abs() > 2.0)
                .unwrap_or(true);
            if far {
                pts.push((x, y));
            }
            return;
        }
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
        // 関数の引数・式の直入力のセル掴みは、離した所で終わり
        if let Some(a) = &mut self.fn_args {
            a.pick_from = None;
        }
        self.ref_pick = None;
        if let Some(pts) = self.ink_cur.take() {
            self.finish_ink(pts);
            return;
        }
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
        for c in self.visible_cols() {
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
        let mut x = self.head_w();
        let mut cfound = false;
        for c in self.visible_cols() {
            if c == p.col {
                cfound = true;
                break;
            }
            x += self.col_px(c);
        }
        let mut y = self.head_h();
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
            self.status = ui::t!("貼り付けるものがありません").into();
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
                        ui::t!("書式は他のアプリからは持って来られません(このアプリでコピーした範囲だけ)").into();
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
        recalc_book(&mut self.book, self.active);
        self.dirty = true;
        self.sync_input();
        self.status = match mode {
            "values" => ui::tf!("{} セルに値だけを貼りました(書式は据え置き)", n),
            "formulas" => ui::tf!("{} セルに式をそのまま貼りました(参照はずらしていません)", n),
            "formats" => ui::tf!("{} セルに書式だけを写しました(中身は残っています)", n),
            _ => ui::tf!("{} セルを転置して貼りました(式は値になっています)", n),
        }
        .into();
    }

    fn a_paste_values(&mut self, _: &ui::PasteValues, _: &mut Window, cx: &mut Context<Self>) {
        self.paste_special("values", cx);
        cx.notify();
    }

    /// 一覧から選んだものを適用する(pick_kind で意味が変わる)。
    fn apply_pick(&mut self, v: &str, cx: &mut Context<Self>) {
        match self.pick_kind {
            "font" => {
                let name = v.to_string();
                self.fmt(move |f| f.font = Some(name.clone()));
                self.status = ui::tf!("書体を「{}」にしました", v).into();
            }
            "size" => {
                if let Ok(pt) = v.parse::<f32>() {
                    self.fmt(move |f| f.size_c = Some((pt * 100.0) as u32));
                    self.status = ui::tf!("文字の大きさを {}pt にしました", v).into();
                }
            }
            "symbol" => {
                // 打ちかけの続きに差し込む(セルを置き換えない)
                self.input.insert(v);
                self.dirty = true;
                self.status = ui::tf!("「{}」を差し込みました(Enter で確定)", v).into();
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
                self.status = ui::tf!("{}を {} に置きました(ドラッグで移動 / 右下で大きさ / Del で削除)", v, at.a1())
                .into();
            }
            "sa-cat" => {
                if let Some(ci) = SMARTART.iter().position(|(n, _)| *n == v) {
                    self.sa_cat = ci;
                    let names: Vec<String> =
                        SMARTART[ci].1.iter().map(|(n, _)| n.to_string()).collect();
                    self.pick_kind = "sa-item";
                    self.pick = Some((names, (HEAD_W + 120.0, ROW_H + 20.0)));
                    self.status = ui::tf!("SmartArt > {}: 形を選ぶと図形の集まりとして入ります", v)
                    .into();
                    return; // pick_kind を "value" に戻さない(2段目へ)
                }
            }
            "sa-item" => {
                let hit = SMARTART
                    .get(self.sa_cat)
                    .and_then(|(_, items)| items.iter().find(|(n, _)| *n == v));
                if let Some((name, key)) = hit {
                    let (name, key) = (name.to_string(), key.to_string());
                    self.insert_smartart(&name, &key);
                }
            }
            "scheme" => {
                if let Some((_, cols)) = sheet::theme::SCHEMES.iter().find(|(n, _)| *n == v) {
                    self.checkpoint_book();
                    self.book.theme = cols.iter().map(|c| c.to_string()).collect();
                    // テーマ由来の色を持つセルを解き直す(配色に追従させる)
                    let theme = self.book.theme.clone();
                    let mut n = 0usize;
                    for sh in &mut self.book.sheets {
                        for cell in sh.cells.values_mut() {
                            if let Some((i, t)) = cell.fmt.color_theme {
                                cell.fmt.color =
                                    Some(sheet::theme::resolve(&theme, i, t as f32 / 1000.0));
                                n += 1;
                            }
                            if let Some((i, t)) = cell.fmt.fill_theme {
                                cell.fmt.fill =
                                    Some(sheet::theme::resolve(&theme, i, t as f32 / 1000.0));
                                n += 1;
                            }
                        }
                    }
                    self.dirty = true;
                    self.status = ui::tf!("配色を「{}」にしました({} 箇所の色が追従。テーマ色を使っていないセルは変わりません)", v, n)
                    .into();
                }
            }
            // 直入力の補完: 打ちかけの名前を選んだ関数に置き換えて ( まで入れる
            "fn-complete" => {
                let t = self.input.text().to_string();
                let cur = self.input.cursor().min(t.len());
                let tok_len: usize = t[..cur]
                    .chars()
                    .rev()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '.')
                    .map(|c| c.len_utf8())
                    .sum();
                let start = cur - tok_len;
                let mut t2 = t.clone();
                t2.replace_range(start..cur, &format!("{v}("));
                self.input = Editor::new(&t2);
                self.input.move_to(start + v.len() + 1, false);
                self.edit_armed = true;
                self.formula_assist();
            }
            "func-cat" => {
                let id = match v {
                    "統計" => "fn-math",
                    "数学" => "fn-math",
                    "財務" => "fn-financial",
                    "日付" => "fn-datetime",
                    "文字列" => "fn-text",
                    "論理" => "fn-logical",
                    _ => "fn-lookup",
                };
                self.run_cmd(id, cx);
            }
            "cell-style" => {
                if let Some((_, f)) = CELL_STYLES.iter().find(|(n, _)| *n == v) {
                    let f = *f;
                    self.fmt(move |c| f(c));
                    self.status = ui::tf!("セルのスタイル「{}」を掛けました", v).into();
                }
            }
            "unhide" => {
                if let Some((_, path)) = self.pick_paths.iter().find(|(n, _)| n == v).cloned() {
                    if let Ok(i) = path.to_string_lossy().parse::<usize>() {
                        if i < self.book.sheets.len() {
                            self.checkpoint_book();
                            self.book.sheets[i].hidden = false;
                            self.switch_sheet(i);
                            self.dirty = true;
                            self.status = ui::tf!("シート「{}」を表示に戻しました", v).into();
                        }
                    }
                }
                self.pick_paths.clear();
            }
            "history" | "plugin" => {
                let plugin = self.pick_kind == "plugin";
                let hit = self.pick_paths.iter().find(|(n, _)| n == v).cloned();
                if let Some((_, path)) = hit {
                    if plugin {
                        match std::fs::read_to_string(&path) {
                            Ok(code) => self.run_python(code, cx),
                            Err(e) => self.status = ui::tf!("読めません: {}", e).into(),
                        }
                    } else {
                        self.open_version(&path);
                    }
                }
                self.pick_paths.clear();
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
        recalc_book(&mut self.book, self.active);
        self.dirty = true;
        self.sync_input();
        self.status = ui::tf!("{} に入れました", p.a1()).into();
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
                recalc_book(&mut self.book, self.active);
                self.dirty = true;
                self.sync_input();
                self.status = ui::tf!("{} セルを消去しました(中身も書式も)", n).into();
            }
            "clear-text" => {
                self.checkpoint();
                let n = self.clear_range();
                self.status = ui::tf!("{} セルの中身を消しました(書式は残る)", n).into();
            }
            "clear-fmt" => self.run_cmd("clear", cx),
            "insrow" => {
                self.rowcol(|s, p| s.insert_row(p.row));
                self.status = ui::t!("行を挿しました(下の式の参照も直っています)").into();
            }
            "delrow" => {
                self.rowcol(|s, p| s.remove_row(p.row));
                self.status = ui::t!("行を削除しました").into();
            }
            "inscol" => {
                self.rowcol(|s, p| s.insert_col(p.col));
                self.status = ui::t!("列を挿しました").into();
            }
            "delcol" => {
                self.rowcol(|s, p| s.remove_col(p.col));
                self.status = ui::t!("列を削除しました").into();
            }
            "sort-asc" | "sort-desc" => {
                self.commit();
                self.checkpoint();
                let c = self.cursor.col;
                self.book.sheets[self.active].sort_by_column(c, id == "sort-asc", true);
                self.dirty = true;
                recalc_book(&mut self.book, self.active);
                self.status = ui::tf!("{} 列で{}に並べ替えました", Pos::new(0, c).a1().trim_end_matches('1'), if id == "sort-asc" { "昇順" } else { "降順" })
                .into();
            }
            "filter-set" => self.run_cmd("setfilter", cx),
            "filter-clear" => self.run_cmd("clear-filter", cx),
            "reapply" => {
                if let Some((c, v)) = self.filter.clone() {
                    let n = self.matching_rows(c, &v).len();
                    self.status = ui::tf!("{}列を「{}」で絞り込み直しました({}行が一致)", Pos::new(0, c).a1().trim_end_matches('1'), v, n)
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
                        recalc_book(&mut self.book, self.active);
                        self.dirty = true;
                        self.anchor = None;
                        self.sync_input();
                        self.status = ui::tf!("{} セルをシフトしました(動いたセルへの参照も直っています)", n)
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
                self.status = ui::tf!("{}:{} — 0未満を赤字にしました", range.0.a1(), range.1.a1()).into();
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
                self.status = ui::tf!("{} 本の条件を消しました", n).into();
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

    /// 名前ボックスの Enter。番地(B12)・範囲(A1:C9)・定義済みの名前なら
    /// そこへ飛ぶ。知らない名前なら**いまの選択に名前を付ける**(Excel と同じ)
    fn commit_name_box(&mut self) {
        let Some(ed) = self.name_edit.take() else { return };
        let t = ed.text().trim().to_string();
        if t.is_empty() {
            return;
        }
        let up = t.to_uppercase();
        let jump = |this: &mut Self, a: Pos, b: Option<Pos>| {
            this.commit();
            this.cursor = b.unwrap_or(a);
            this.anchor = b.is_some().then_some(a);
            this.sync_input();
            this.follow();
        };
        if let Some((a, b)) = up.split_once(':') {
            if let (Some(pa), Some(pb)) = (Pos::parse(a), Pos::parse(b)) {
                jump(self, pa, Some(pb));
                self.status = ui::tf!("{} を選びました", up).into();
                return;
            }
        }
        if let Some(p) = Pos::parse(&up) {
            jump(self, p, None);
            self.status = ui::tf!("{} へ移動しました", p.a1()).into();
            return;
        }
        // 定義済みの名前ならそこへ
        if let Some((_, r)) = self
            .sheet()
            .names
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(&t))
            .cloned()
        {
            let up = r.to_uppercase();
            if let Some((a, b)) = up.split_once(':') {
                if let (Some(pa), Some(pb)) = (Pos::parse(a), Pos::parse(b)) {
                    jump(self, pa, Some(pb));
                    self.status = ui::tf!("名前「{}」({})を選びました", t, up).into();
                    return;
                }
            }
            if let Some(p) = Pos::parse(&up) {
                jump(self, p, None);
                self.status = ui::tf!("名前「{}」({})へ移動しました", t, up).into();
                return;
            }
        }
        // 新しい名前 = いまの選択に付ける
        let range = if self.anchor.is_some() {
            let (a, b) = self.sel_rect();
            format!("{}:{}", a.a1(), b.a1())
        } else {
            self.cursor.a1()
        };
        self.checkpoint();
        self.sheet_mut().names.push((t.clone(), range.clone()));
        self.dirty = true;
        self.status = ui::tf!("名前「{}」を {} に付けました(名前ボックスで呼べます)", t, range).into();
    }

    /// 式の直入力の支援。=を打っている間だけ:
    /// - 打ちかけの関数名(2字以上)には**補完の一覧**(セルの下。押すと入る)
    /// - 開いた括弧の中では、**いま打っている引数のヒント**を状態帯に
    fn formula_assist(&mut self) {
        let t = self.input.text().to_string();
        if !t.starts_with('=') {
            if self.pick_kind == "fn-complete" {
                self.pick = None;
            }
            return;
        }
        let cur = self.input.cursor().min(t.len());
        // --- 補完: カーソルの直前の識別子(英字はじまり・2字以上) ---
        let token: String = {
            let rev: String = t[..cur]
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '.')
                .collect();
            rev.chars().rev().collect()
        };
        let mut showed = false;
        if token.len() >= 2 && token.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
            let up = token.to_uppercase();
            let cands: Vec<String> = funcs::FUNCS
                .iter()
                .filter(|f| f.name.starts_with(&up) && f.name != up)
                .map(|f| f.name.to_string())
                .take(12)
                .collect();
            if !cands.is_empty() {
                if let Some((x, y)) = self.cell_origin_px(self.cursor) {
                    let h = self.row_px(self.cursor.row);
                    self.pick_kind = "fn-complete";
                    self.pick = Some((cands, (x, y + h)));
                    showed = true;
                }
            }
        }
        if !showed && self.pick_kind == "fn-complete" {
            self.pick = None;
        }
        // --- 引数のヒント: いちばん内側の閉じていない関数と、何番目の引数か ---
        let mut stack: Vec<(String, usize)> = Vec::new();
        let mut in_str = false;
        let mut ident = String::new();
        for ch in t[..cur].chars() {
            match ch {
                '"' => in_str = !in_str,
                _ if in_str => {}
                '(' => {
                    stack.push((ident.to_uppercase(), 0));
                    ident.clear();
                }
                ')' => {
                    stack.pop();
                    ident.clear();
                }
                ',' => {
                    if let Some((_, n)) = stack.last_mut() {
                        *n += 1;
                    }
                    ident.clear();
                }
                c if c.is_ascii_alphanumeric() || c == '.' => ident.push(c),
                _ => ident.clear(),
            }
        }
        if let Some((name, argi)) = stack.last() {
            if let Some(f) = funcs::FUNCS.iter().find(|f| f.name == name) {
                let hint = f
                    .arg_desc
                    .get(*argi)
                    .or(f.arg_desc.last())
                    .copied()
                    .unwrap_or("");
                let names = parse_fn_args(f.args);
                let arg_name = names
                    .get(*argi)
                    .or(names.last())
                    .map(|(n, _)| n.clone())
                    .unwrap_or_default();
                self.status =
                    format!("{}{} — {}{}", f.name, f.args, arg_name, hint).into();
            }
        }
    }

    /// 「関数を挿入」の次へ = 選んだ関数の**引数の画面**へ進む(本家の第2段)
    fn fn_next(&mut self) {
        let Some(d) = self.fn_dlg.take() else { return };
        let list = fn_filtered(d.search.text(), d.group);
        let Some(f) = list.get(d.sel.min(list.len().saturating_sub(1))).copied() else {
            self.status = ui::t!("その条件の関数がありません").into();
            return;
        };
        let names = parse_fn_args(f.args);
        let eds = (0..names.len()).map(|_| Editor::new("")).collect();
        self.fn_args = Some(FnArgs {
            f,
            names,
            eds,
            focus: 0,
            result: String::new(),
            pick_from: None,
        });
        self.fn_args_recalc();
        self.status = ui::t!(
            "関数の引数: Tab で次の欄。セルをクリックすると参照が入ります。Enter で式に")
        .into();
    }

    /// 引数の画面の中身から式の文字を組む(埋めた欄まで)
    fn fn_args_formula(&self) -> Option<String> {
        let a = self.fn_args.as_ref()?;
        let vals: Vec<String> = a.eds.iter().map(|e| e.text().trim().to_string()).collect();
        let mut last = 0;
        for (i, v) in vals.iter().enumerate() {
            if !v.is_empty() {
                last = i + 1;
            }
        }
        Some(format!("{}({})", a.f.name, vals[..last].join(", ")))
    }

    /// 関数の結果の下見。**表の複製**の空きセルで計算する(ゴールシークと
    /// 同じ流儀 — 本物の表は触らない)
    fn fn_args_recalc(&mut self) {
        let Some(fstr) = self.fn_args_formula() else { return };
        let mut s = self.sheet().clone();
        let (rows, _) = s.extent();
        let p = Pos::new(rows + 2, 0);
        s.set(p, Cell::input(&format!("={fstr}")));
        recalc(&mut s);
        let out = s.get(p).map(|c| c.value.display()).unwrap_or_default();
        if let Some(a) = &mut self.fn_args {
            a.result = out;
        }
    }

    /// 引数の画面の OK。組んだ式をセルへ(編集中ならカーソルに差し込み)
    fn fn_args_ok(&mut self) {
        let Some(fstr) = self.fn_args_formula() else {
            self.fn_args = None;
            return;
        };
        self.fn_args = None;
        if self.editing() || self.edit_armed {
            self.input.insert(&fstr);
        } else {
            self.input = Editor::new(&format!("={fstr}"));
            let end = self.input.text().len();
            self.input.move_to(end, false);
        }
        self.edit_armed = true;
        self.status = ui::t!("式を入れました(Enter で確定 / Esc で取消)").into();
    }

    /// F2 = このセルを編集(次の打鍵が**追記**になる。Excel と同じ)
    fn a_edit_cell(&mut self, _: &ui::EditCell, _: &mut Window, cx: &mut Context<Self>) {
        if self.prompt.is_some() || self.solver.is_some() {
            return;
        }
        self.edit_armed = true;
        self.input.move_to(self.input.text().len(), false);
        self.status = ui::t!("編集: そのまま打つと続きに入ります(Esc で取消)").into();
        cx.notify();
    }

    fn a_cancel(&mut self, _: &ui::Cancel, _: &mut Window, cx: &mut Context<Self>) {
        if self.quit_ask {
            self.quit_ask = false;
            self.status = ui::t!("終了をやめました").into();
            cx.notify();
            return;
        }
        // 名前ボックス・関数の小窓は最優先で閉じる
        if self.name_edit.take().is_some()
            || self.fn_args.take().is_some()
            || self.fn_dlg.take().is_some()
        {
            cx.notify();
            return;
        }
        // 入力の板 → 一覧 → 子メニュー → 親メニュー → 書式の小窓 → コピーの破線、
        // の順で閉じる
        self.pivot_pend = None; // 聞き取り途中のピボット・小計は Esc でやめる
        self.sub_pend = None;
        self.pw_pending = None; // パスワード待ちも Esc でやめる(開かない)
        if self.tool.take().is_some() {
            self.ink_cur = None;
            self.status = ui::t!("セルの操作に戻りました").into();
        }
        if self.solver.take().is_some()
            || self.slicer.take().is_some()
            || self.prompt.take().is_some()
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
            self.status = ui::t!("打ちかけを取り消しました").into();
            cx.notify();
        } else if self.edit_armed {
            // F2 だけ押して何も打っていない — 編集をやめる
            self.edit_armed = false;
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
                    self.status = ui::t!("名前を付けませんでした").into();
                    return;
                }
                let ok = text.chars().all(|c| c.is_alphanumeric() || c == '_')
                    && !text.chars().next().unwrap().is_ascii_digit()
                    && Pos::parse(&text).is_none();
                if !ok {
                    self.status = ui::tf!("「{}」は名前にできません(文字と数字と _。セル参照の形は不可)", text)
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
                recalc_book(&mut self.book, self.active);
                self.dirty = true;
                self.status = ui::tf!("名前「{}」= {}(式の中で使えます)", text, range).into();
            }
            "comment" => {
                let p = self.cursor;
                if text.is_empty() {
                    if self.book.sheets[self.active].comments.remove(&p).is_some() {
                        self.dirty = true;
                        self.status = ui::tf!("{} のコメントを消しました", p.a1()).into();
                    }
                } else {
                    self.book.sheets[self.active].comments.insert(p, text);
                    self.dirty = true;
                    self.status = ui::tf!("{} にコメントを付けました(保存で残ります)", p.a1()).into();
                }
            }
            "cond-gt" | "cond-lt" => {
                let Ok(value) = text.parse::<f64>() else {
                    self.status = ui::tf!("「{}」は数として読めません", text).into();
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
                self.status = ui::tf!("{}:{} — {} より{}を塗ります", range.0.a1(), range.1.a1(), value, if gt { "大きい値" } else { "小さい値" }).into();
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
                        ui::t!("ブックに載せた Python はありません(@save 名前 で載せる)").into()
                    } else {
                        ui::tf!("ブックの Python: {}(@名前 で実行)", names.join(" / ")).into()
                    };
                } else if let Some(name) = t.strip_prefix("@save ") {
                    let name = name.trim().to_string();
                    if name.is_empty() {
                        self.status = ui::t!("@save 名前 の形で").into();
                    } else {
                        self.store_python_dialog(name, cx);
                    }
                } else if let Some(name) = t.strip_prefix("@del ") {
                    let name = name.trim();
                    let before = self.book.scripts.len();
                    self.book.scripts.retain(|(n, _)| n != name);
                    if self.book.scripts.len() < before {
                        self.dirty = true;
                        self.status = ui::tf!("「{}」をブックから外しました", name).into();
                    } else {
                        self.status = ui::tf!("「{}」はありません", name).into();
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
                                    ui::t!("網あり檻で実行します(ファイルは守られたまま)").into();
                            }
                            self.run_python_inner(code, true, net, cx);
                        }
                        None => {
                            self.status =
                                ui::tf!("「{}」はありません(@list で一覧)", name).into();
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
                    ui::t!("文字を消しました").into()
                } else {
                    ui::t!("図形に文字を入れました(保存で xlsx に入ります)").into()
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
                    self.status = ui::tf!("「{}」で割れるセルが選択にありません", delim).into();
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
                recalc_book(&mut self.book, self.active);
                self.dirty = true;
                self.sync_input();
                self.status =
                    ui::tf!("{} 欄に割りました(右のセルは上書き。Ctrl+Z で戻せます)", n).into();
            }
            "goal-target" => {
                // 「D6=765600」の形
                let Some((cell_s, val_s)) = text.split_once('=') else {
                    self.status = ui::t!("「セル=目標値」の形で(例: D6=800000)").into();
                    return;
                };
                let (Some(p), Ok(v)) = (Pos::parse(cell_s), val_s.trim().parse::<f64>()) else {
                    self.status = ui::t!("読めません(例: D6=800000)").into();
                    return;
                };
                self.goal = Some((p, v));
                self.prompt = Some(("goal-var", Editor::new("")));
            }
            "goal-var" => {
                let Some((target, goal)) = self.goal.take() else { return };
                let Some(var) = Pos::parse(&text) else {
                    self.status = ui::t!("変えるセルが読めません(例: B2)").into();
                    return;
                };
                self.goal_seek(target, goal, var);
            }
            // パスワードの板。開き待ちがあれば解いて開き、
            // 無ければ「次の保存から暗号化」を決める(空なら解除)
            "pw-open" => {
                let Some(p) = self.pw_pending.take() else { return };
                let bytes = match std::fs::read(&p) {
                    Ok(b) => b,
                    Err(e) => {
                        self.status = ui::tf!("開けません: {}", e).into();
                        return;
                    }
                };
                match ooxml::crypt::decrypt(&bytes, &text) {
                    Ok(plain) => {
                        self.open_plain(p.clone(), plain);
                        if self.path.as_deref() == Some(p.as_path()) {
                            self.encrypt_pw = Some(text);
                            self.status = ui::tf!("{}(保存も同じパスワードで暗号化します)", self.status)
                            .into();
                        }
                    }
                    Err(e) => {
                        // 板は開いたまま。打ち直せる
                        self.pw_pending = Some(p);
                        self.prompt = Some(("pw-open", Editor::new("")));
                        self.status = e.into();
                    }
                }
            }
            "pw-set" => {
                if text.is_empty() {
                    self.encrypt_pw = None;
                    self.status = ui::t!("暗号化しません(次の保存から普通の xlsx)").into();
                } else {
                    self.encrypt_pw = Some(text);
                    self.dirty = true;
                    self.status =
                        ui::t!("次の保存から、このパスワードで暗号化します(AES-128。Excel や LibreOffice でも開けます)").into();
                }
            }
            "equation" => {
                if text.is_empty() {
                    self.status = ui::t!("式が空です(何も置きませんでした)").into();
                } else {
                    self.insert_py_image(EQ_PY, "eq", text, cx);
                }
            }
            "textart" => {
                if text.is_empty() {
                    self.status = ui::t!("文字が空です(何も置きませんでした)").into();
                } else {
                    self.insert_py_image(TEXTART_PY, "textart", text, cx);
                }
            }
            // ブックの情報(保存で docProps/core.xml へ)
            "prop-creator" | "prop-title" | "prop-keywords" | "prop-subject"
            | "prop-desc" => {
                let f = match kind {
                    "prop-creator" => &mut self.book.props.creator,
                    "prop-title" => &mut self.book.props.title,
                    "prop-keywords" => &mut self.book.props.keywords,
                    "prop-subject" => &mut self.book.props.subject,
                    _ => &mut self.book.props.description,
                };
                *f = text;
                self.dirty = true;
                self.status =
                    ui::t!("ブックの情報を控えました(保存で xlsx に入ります)").into();
            }
            "table-resize" => {
                let p = self.cursor;
                let Some(i) = self.sheet().tables.iter().position(|t| t.contains(p)) else {
                    return;
                };
                let parse = |t: &str| -> Option<(Pos, Pos)> {
                    let (x, y) = t.split_once(':')?;
                    Some((Pos::parse(x.trim())?, Pos::parse(y.trim())?))
                };
                match parse(&text) {
                    None => {
                        self.status = ui::t!("範囲は A1:C9 の形で書いてください").into();
                        self.prompt = Some(("table-resize", Editor::new(&text)));
                    }
                    Some((a, b)) if b.row < a.row || b.col < a.col => {
                        self.status = ui::t!("左上と右下が逆です(A1:C9 の順で)").into();
                        self.prompt = Some(("table-resize", Editor::new(&text)));
                    }
                    Some((a, b)) => {
                        self.checkpoint();
                        {
                            let t = &mut self.book.sheets[self.active].tables[i];
                            t.a = a;
                            t.b = b;
                        }
                        self.dirty = true;
                        self.status = ui::tf!("表の範囲を {}:{} にしました(書式は掛け直しません — 表のデザインの釦でどうぞ)", a.a1(), b.a1())
                        .into();
                    }
                }
            }
            "ai-table" => {
                if text.is_empty() {
                    self.status = ui::t!("文章がありません(何もしていません)").into();
                } else {
                    self.ai_go(CalcAi::Table(text), cx);
                }
            }
            "ai-ask" => {
                if text.is_empty() {
                    self.status = ui::t!("用件がありません(何もしていません)").into();
                } else {
                    self.ai_go(CalcAi::Ask(text), cx);
                }
            }
            "chat" => {
                if text.is_empty() {
                    self.status = ui::t!("何も書き残しませんでした").into();
                } else if let Some(cp) = self.chat_path() {
                    let stamp = std::process::Command::new("date")
                        .arg("+%Y-%m-%d %H:%M")
                        .output()
                        .ok()
                        .filter(|o| o.status.success())
                        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                        .unwrap_or_default();
                    let line = format!("[{stamp}] {}: {text}\n", lock_identity());
                    use std::io::Write as _;
                    let r = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&cp)
                        .and_then(|mut f| f.write_all(line.as_bytes()));
                    self.status = match r {
                        Ok(_) => ui::tf!("書き残しました({})", cp.file_name().unwrap_or_default().to_string_lossy())
                        .into(),
                        Err(e) => ui::tf!("書けません: {}", e).into(),
                    };
                }
            }
            // 小計の聞き取り(区切りの見出し → 合計する見出し)
            "subtotal-by" => {
                let Some(mut pend) = self.sub_pend.take() else { return };
                let t = text.trim().to_string();
                if !pend.headers.iter().any(|h| *h == t) {
                    self.status =
                        ui::tf!("「{}」は見出しにありません: {}", t, pend.headers.join(" / "))
                            .into();
                    self.sub_pend = Some(pend);
                    self.prompt = Some(("subtotal-by", Editor::new(&text)));
                    return;
                }
                pend.rows_sel = vec![t];
                self.status =
                    ui::t!("合計する見出し(カンマ区切り可。空 Enter = 数の列全部)").into();
                self.sub_pend = Some(pend);
                self.prompt = Some(("subtotal-vals", Editor::new("")));
            }
            "subtotal-vals" => {
                let Some(pend) = self.sub_pend.take() else { return };
                let by_off =
                    pend.headers.iter().position(|h| *h == pend.rows_sel[0]).unwrap_or(0);
                let by = pend.a.col + by_off as u32;
                let sel = split_fields(&text);
                let mut vals: Vec<u32> = Vec::new();
                if sel.is_empty() {
                    // 数の列を自動で拾う(基準の列は除く)
                    let sh = self.sheet();
                    for i in 0..pend.headers.len() {
                        let c = pend.a.col + i as u32;
                        if c == by {
                            continue;
                        }
                        let numeric = (pend.a.row + 1..=pend.b.row).any(|r| {
                            matches!(
                                sh.get(Pos::new(r, c)).map(|x| &x.value),
                                Some(Value::Number(_))
                            )
                        });
                        if numeric {
                            vals.push(c);
                        }
                    }
                    if vals.is_empty() {
                        self.status =
                            ui::t!("数の列が見つかりません(合計する見出しを書いてください)").into();
                        self.sub_pend = Some(pend);
                        self.prompt = Some(("subtotal-vals", Editor::new("")));
                        return;
                    }
                } else {
                    for name in &sel {
                        match pend.headers.iter().position(|h| h == name) {
                            Some(i) => vals.push(pend.a.col + i as u32),
                            None => {
                                self.status =
                                    ui::tf!("「{}」は見出しにありません", name).into();
                                self.sub_pend = Some(pend);
                                self.prompt = Some(("subtotal-vals", Editor::new(&text)));
                                return;
                            }
                        }
                    }
                }
                self.checkpoint();
                let n = apply_subtotals(
                    &mut self.book.sheets[self.active],
                    pend.a,
                    pend.b,
                    by,
                    &vals,
                );
                recalc_book(&mut self.book, self.active);
                self.dirty = true;
                self.sync_input();
                self.status = ui::tf!("{} 区切りに小計と総計を入れ、明細をグループ化しました — 「詳細の非表示」で畳むと合計だけ残ります(Ctrl+Z で1手)", n)
                .into();
            }
            // ピボットの聞き取り(行 → 列 → 値と集計)。間違いは板を出し直して言う
            "pivot-rows" => {
                let Some(mut pend) = self.pivot_pend.take() else { return };
                let sel = split_fields(&text);
                if sel.is_empty() {
                    self.status =
                        ui::tf!("行に並べる見出しを1つは選んでください: {}", pend.headers.join(" / ")).into();
                    self.pivot_pend = Some(pend);
                    self.prompt = Some(("pivot-rows", Editor::new("")));
                    return;
                }
                if let Some(bad) = sel.iter().find(|s| !pend.headers.contains(s)) {
                    self.status = ui::tf!("「{}」は見出しにありません: {}", bad, pend.headers.join(" / ")).into();
                    self.pivot_pend = Some(pend);
                    self.prompt = Some(("pivot-rows", Editor::new(&text)));
                    return;
                }
                pend.rows_sel = sel;
                let rest: Vec<&String> =
                    pend.headers.iter().filter(|h| !pend.rows_sel.contains(h)).collect();
                self.status = ui::tf!("列に広げる見出し(空 Enter = なし): {}", rest.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" / ")).into();
                self.pivot_pend = Some(pend);
                self.prompt = Some(("pivot-cols", Editor::new("")));
            }
            "pivot-cols" => {
                let Some(mut pend) = self.pivot_pend.take() else { return };
                let sel = split_fields(&text);
                if let Some(bad) = sel.iter().find(|s| !pend.headers.contains(s)) {
                    self.status = ui::tf!("「{}」は見出しにありません: {}", bad, pend.headers.join(" / ")).into();
                    self.pivot_pend = Some(pend);
                    self.prompt = Some(("pivot-cols", Editor::new(&text)));
                    return;
                }
                pend.cols_sel = sel;
                self.status = ui::t!("値にする見出しと集計(例: 金額 合計。合計/平均/個数/最大/最小)").into();
                self.pivot_pend = Some(pend);
                self.prompt = Some(("pivot-val", Editor::new("")));
            }
            "pivot-val" => {
                let Some(pend) = self.pivot_pend.take() else { return };
                match parse_pivot_val(&text, &pend.headers) {
                    Ok((value, agg)) => self.insert_pivot(pend, value, agg, cx),
                    Err(e) => {
                        self.status = e.into();
                        self.pivot_pend = Some(pend);
                        self.prompt = Some(("pivot-val", Editor::new(&text)));
                    }
                }
            }
            "find" => {
                if text.is_empty() {
                    self.status = ui::t!("探す言葉を入れてください").into();
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
                    self.status = ui::tf!("「{}」は見つかりません", find).into();
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
                recalc_book(&mut self.book, self.active);
                self.dirty = true;
                self.sync_input();
                self.find_term = Some(find.clone());
                self.status =
                    ui::tf!("「{}」→「{}」: {} カ所を置き換えました(Ctrl+Z で戻せます)", find, text, n)
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
                        self.status = ui::t!("この範囲に入力規則はありません").into();
                        return;
                    }
                    self.checkpoint();
                    self.book.sheets[self.active].validations.retain(|v| !overlap(v));
                    self.dirty = true;
                    self.status = ui::tf!("{} 本の入力規則を外しました", n).into();
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
                        ui::t!("候補が読めません(例: 甲,乙,丙 または =D2:D5)").into();
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
        self.status = ui::t!("外枠を引きました").into();
    }

    /// 書式の小窓の釦。
    fn fmt_panel_action(&mut self, id: &str, cx: &mut Context<Self>) {
        match id {
            "close" => self.fmt_panel = None,
            "b-all" => {
                self.fmt(|f| f.borders = Borders::ALL);
                self.status = ui::t!("格子の罫線を引きました").into();
            }
            "b-out" => self.border_outline(),
            "b-none" => {
                self.fmt(|f| f.borders = Borders::NONE);
                self.status = ui::t!("罫線を消しました").into();
            }
            "numfmt-none" => {
                self.fmt(|f| f.number_format = None);
                self.status = ui::t!("表示形式を戻しました").into();
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
                self.status = ui::t!("この列にはまだ値がありません").into();
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
    /// 描いた1筆(格子の px の列)を図形(折れ線)にして置く。
    /// **既にある図形の仕組みに乗せる** — xlsx へは custGeom で入り、
    /// Excel でも線に見え、消しゴムも移動も Ctrl+Z も全部そのまま効く
    fn finish_ink(&mut self, pts: Vec<(f32, f32)>) {
        if pts.len() < 2 {
            return; // 点を打っただけ(線にならない)
        }
        let (mut x0, mut y0) = (f32::MAX, f32::MAX);
        let (mut x1, mut y1) = (f32::MIN, f32::MIN);
        for (x, y) in &pts {
            x0 = x0.min(*x);
            y0 = y0.min(*y);
            x1 = x1.max(*x);
            y1 = y1.max(*y);
        }
        let (w, h) = ((x1 - x0).max(4.0), (y1 - y0).max(4.0));
        // 錨は左上の点があるセル。そこからのずらしで位置を覚える
        let at = self.cell_at(x0, y0).unwrap_or(self.view);
        let (ox, oy) = self.cell_origin_px(at).unwrap_or((self.head_w(), self.head_h()));
        let marker = self.tool == Some(1);
        self.checkpoint();
        self.sheet_mut().shapes_new.push(sheet::model::SheetShape {
            at,
            dx_px: x0 - ox,
            dy_px: y0 - oy,
            width_px: w,
            height_px: h,
            kind: if marker { "marker".into() } else { "ink".into() },
            fill: None,
            line: Some(if marker { "FFD54A".into() } else { "1B1B1B".into() }),
            points: pts
                .iter()
                .map(|(x, y)| ((x - x0) / w, (y - y0) / h))
                .collect(),
            ..Default::default()
        });
        self.dirty = true;
        self.status = if marker {
            ui::t!("蛍光ペンで引きました(Ctrl+Z で戻せます)").into()
        } else {
            ui::t!("ペンで描きました(Ctrl+Z で戻せます)").into()
        };
    }

    /// この位置にある手描きの線(いちばん上のもの)。消しゴムが使う
    fn ink_at(&self, x: f32, y: f32) -> Option<usize> {
        let sh = self.sheet();
        for (i, sp) in sh.shapes_new.iter().enumerate().rev() {
            if !matches!(sp.kind.as_str(), "ink" | "marker" | "spark") {
                continue;
            }
            let Some((ox, oy)) = self.cell_origin_px(sp.at) else { continue };
            let (x0, y0) = (ox + sp.dx_px, oy + sp.dy_px);
            let near = if sp.kind == "marker" { 7.0 } else { 4.0 };
            let hit = sp.points.iter().any(|(px_, py_)| {
                let (cx, cy) = (x0 + px_ * sp.width_px, y0 + py_ * sp.height_px);
                (cx - x).abs() <= near && (cy - y).abs() <= near
            });
            if hit {
                return Some(i);
            }
        }
        None
    }

    /// 選択範囲(見た目の値)の TSV。AI に渡す形
    fn tsv_display(&self, a: Pos, b: Pos) -> String {
        let sh = self.sheet();
        (a.row..=b.row)
            .map(|r| {
                (a.col..=b.col)
                    .map(|c| sh.get(Pos::new(r, c)).map(|x| x.value.display()).unwrap_or_default())
                    .collect::<Vec<_>>()
                    .join("\t")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// AI に頼んで、返事を表に反映する。**別の糸で待つ**(画面は止めない)。
    /// 反映は必ず checkpoint してから = **Ctrl+Z の1手で戻る**。
    /// 宛先が使えなければ理由を言う(黙って空にしない)
    fn ai_go(&mut self, job: CalcAi, cx: &mut Context<Self>) {
        if self.sheet().protected {
            self.status =
                ui::t!("シートが保護されています(保護タブの「保護」で解除)").into();
            return;
        }
        if self.ai_busy {
            self.status = ui::t!("いま考えています(終わるまでお待ちください)").into();
            return;
        }
        let back = ui::ai::backend();
        if let Err(e) = ui::ai::ready(back) {
            self.status = format!("AI: {e}").into();
            return;
        }
        self.commit();
        // 渡す範囲: 選択があればそこ。要約だけは無選択なら使っている全域
        let sel = self.anchor.map(|_| self.sel_rect());
        let (a, b) = match (&job, sel) {
            (_, Some(r)) => r,
            (CalcAi::Summary, None) => {
                let (rows, cols) = self.sheet().extent();
                if rows == 0 || cols == 0 {
                    self.status = ui::t!("表がありません").into();
                    return;
                }
                (Pos::new(0, 0), Pos::new((rows - 1).min(199), cols - 1))
            }
            (CalcAi::Table(_) | CalcAi::Ask(_), None) => (self.cursor, self.cursor),
            _ => {
                self.status = ui::t!("範囲を選んでから押してください").into();
                return;
            }
        };
        if matches!(job, CalcAi::Furigana) && a.col != b.col {
            self.status =
                ui::t!("ふりがなは1列だけ選んでください(読みは右隣の列に入ります)").into();
            return;
        }
        let body = match &job {
            CalcAi::Table(_) => String::new(),
            CalcAi::Ask(_) if self.anchor.is_none() => String::new(),
            _ => self.tsv_display(a, b),
        };
        if body.trim().is_empty()
            && !matches!(job, CalcAi::Table(_) | CalcAi::Ask(_))
        {
            self.status = ui::t!("選んだ範囲が空です").into();
            return;
        }
        let (sys, ask) = job.prompt();
        let user = match &job {
            CalcAi::Table(q) => q.clone(),
            CalcAi::Ask(q) => {
                if body.trim().is_empty() {
                    q.clone()
                } else {
                    format!("{q}\n\n---\n{body}")
                }
            }
            _ => format!("{ask}\n\n---\n{body}"),
        };
        let sys = sys.to_string();
        let job2 = job.clone();
        self.ai_busy = true;
        self.status =
            format!("AI({})に{}を頼んでいます…", back.label(), job.label()).into();
        let task = cx
            .background_executor()
            .spawn(async move { ui::ai::ask(back, &sys, &user) });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |this, cx| {
                this.ai_busy = false;
                match r {
                    Ok(out) => this.ai_apply(job2, a, b, out),
                    Err(e) => this.status = format!("AI: {e}").into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 返事を表へ入れる。**1手で戻せる**(checkpoint してから)
    fn ai_apply(&mut self, job: CalcAi, a: Pos, b: Pos, out: String) {
        let out = out.trim().to_string();
        if out.is_empty() {
            self.status = ui::t!("AI: 答えが空でした(何もしていません)").into();
            return;
        }
        let grid = |t: &str| -> Vec<Vec<String>> {
            t.lines().map(|l| l.split('\t').map(str::to_string).collect()).collect()
        };
        match job {
            // 要約はカーソルのコメントへ(保存で xlsx に残る)
            CalcAi::Summary => {
                let p = self.cursor;
                self.checkpoint();
                self.book.sheets[self.active].comments.insert(p, out);
                self.dirty = true;
                self.status = format!(
                    "要約を {} のコメントに付けました(Ctrl+Z で戻せます)",
                    p.a1()
                )
                .into();
            }
            // 書き直し・翻訳: 同じ形の TSV を受け、**文字のセルだけ**置き換える
            CalcAi::Rewrite(_, _) | CalcAi::Translate => {
                let g = grid(&out);
                let rows = (b.row - a.row + 1) as usize;
                if g.len() != rows {
                    self.status = format!(
                        "AI: 行数が合いません({} 行の答え / {rows} 行の範囲)— 何もしていません",
                        g.len()
                    )
                    .into();
                    return;
                }
                self.checkpoint();
                let mut n = 0usize;
                for (ri, row) in g.iter().enumerate() {
                    for (ci, v) in row.iter().enumerate() {
                        let p = Pos::new(a.row + ri as u32, a.col + ci as u32);
                        if p.col > b.col {
                            break;
                        }
                        let is_text = matches!(
                            self.sheet().get(p).map(|x| &x.value),
                            Some(Value::Text(_))
                        );
                        if is_text && !v.trim().is_empty() {
                            let fmt = self
                                .sheet()
                                .get(p)
                                .map(|c| c.fmt.clone())
                                .unwrap_or_default();
                            let mut cell = Cell::input(v);
                            cell.fmt = fmt;
                            self.book.sheets[self.active].set(p, cell);
                            n += 1;
                        }
                    }
                }
                recalc_book(&mut self.book, self.active);
                self.dirty = true;
                self.sync_input();
                self.status = format!(
                    "{n} 個の文字のセルを直しました(数字と式は触っていません。Ctrl+Z で1手)"
                )
                .into();
            }
            // ふりがな: 右隣の列へ(空きでなければ断る — 黙って潰さない)
            CalcAi::Furigana => {
                let yomi: Vec<&str> = out.lines().collect();
                let rows = (b.row - a.row + 1) as usize;
                if yomi.len() != rows {
                    self.status = format!(
                        "AI: 行数が合いません({} 行の答え / {rows} 行の範囲)— 何もしていません",
                        yomi.len()
                    )
                    .into();
                    return;
                }
                let dst = a.col + 1;
                let used = (a.row..=b.row).any(|r| {
                    self.sheet()
                        .get(Pos::new(r, dst))
                        .map(|c| !c.value.display().is_empty() || c.formula.is_some())
                        .unwrap_or(false)
                });
                if used {
                    self.status =
                        ui::t!("右隣の列に中身があります(空けてから — 黙って上書きしません)").into();
                    return;
                }
                self.checkpoint();
                for (i, y) in yomi.iter().enumerate() {
                    if y.trim().is_empty() {
                        continue;
                    }
                    let p = Pos::new(a.row + i as u32, dst);
                    self.book.sheets[self.active].set(p, Cell::input(y.trim()));
                }
                self.dirty = true;
                self.status =
                    ui::t!("読みを右隣の列に入れました(Ctrl+Z で戻せます)").into();
            }
            // 続き: 選択の下の空き行へ(空きでなければ断る)
            CalcAi::Continue => {
                let g = grid(&out);
                let start = b.row + 1;
                let used = g.iter().enumerate().any(|(ri, row)| {
                    row.iter().enumerate().any(|(ci, _)| {
                        self.sheet()
                            .get(Pos::new(start + ri as u32, a.col + ci as u32))
                            .map(|c| {
                                !c.value.display().is_empty() || c.formula.is_some()
                            })
                            .unwrap_or(false)
                    })
                });
                if used {
                    self.status =
                        ui::t!("下の行に中身があります(空けてから — 黙って上書きしません)").into();
                    return;
                }
                self.checkpoint();
                let n = paste_values_text(
                    &mut self.book.sheets[self.active],
                    Pos::new(start, a.col),
                    &g,
                );
                recalc_book(&mut self.book, self.active);
                self.dirty = true;
                self.status = format!(
                    "続きを {} 行足しました({n} 欄。よく確かめてください — AI の当て推量です。Ctrl+Z で1手)",
                    g.len()
                )
                .into();
            }
            // 表にする: カーソルから流し込み(空きでなければ断る)
            CalcAi::Table(_) => {
                let g = grid(&out);
                let at = self.cursor;
                let used = g.iter().enumerate().any(|(ri, row)| {
                    row.iter().enumerate().any(|(ci, _)| {
                        self.sheet()
                            .get(Pos::new(at.row + ri as u32, at.col + ci as u32))
                            .map(|c| {
                                !c.value.display().is_empty() || c.formula.is_some()
                            })
                            .unwrap_or(false)
                    })
                });
                if used {
                    self.status =
                        ui::t!("ここには中身があります(空きへカーソルを置いてから)").into();
                    return;
                }
                self.checkpoint();
                let n = paste_values_text(&mut self.book.sheets[self.active], at, &g);
                recalc_book(&mut self.book, self.active);
                self.dirty = true;
                self.status = format!(
                    "表を {} に置きました({} 行 {n} 欄。Ctrl+Z で1手)",
                    at.a1(),
                    g.len()
                )
                .into();
            }
            // 頼む: = で始まる1行は式としてカーソルへ。他はコメントへ
            CalcAi::Ask(_) => {
                let p = self.cursor;
                if out.starts_with('=') && !out.contains('\n') {
                    self.checkpoint();
                    let fmt =
                        self.sheet().get(p).map(|c| c.fmt.clone()).unwrap_or_default();
                    let mut cell = Cell::input(&out);
                    cell.fmt = fmt;
                    self.book.sheets[self.active].set(p, cell);
                    recalc_book(&mut self.book, self.active);
                    self.dirty = true;
                    self.sync_input();
                    let shown = self
                        .sheet()
                        .get(p)
                        .map(|c| c.value.display())
                        .unwrap_or_default();
                    self.status = format!(
                        "{} に式を入れました(= {shown}。式は数式バーで確かめられます。Ctrl+Z で1手)",
                        p.a1()
                    )
                    .into();
                } else {
                    self.checkpoint();
                    self.book.sheets[self.active].comments.insert(p, out);
                    self.dirty = true;
                    self.status = format!(
                        "答えを {} のコメントに付けました(Ctrl+Z で戻せます)",
                        p.a1()
                    )
                    .into();
                }
            }
        }
    }

    /// いまの計算方法で再計算する(手動なら何もしない — 「計算」で回す)
    fn recalc_if_auto(&mut self) {
        if self.auto_calc {
            recalc_book(&mut self.book, self.active);
        }
    }

    fn commit(&mut self) -> bool {
        let (cur, text) = (self.cursor, self.input.text().to_string());
        // 変わっていなければ何もしない(移動のたびに履歴が積まれるのを防ぐ)
        let now = self.sheet().get(cur).map(|c| c.editable()).unwrap_or_default();
        if now == text {
            return true;
        }
        // シートの保護。打ちかけは捨てて元に戻す(黙って通さない)
        if self.sheet().protected {
            self.sync_input();
            self.status =
                ui::t!("シートが保護されています(保護タブの「保護」で解除)").into();
            return false;
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
        recalc_book(&mut self.book, self.active);
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
            * self.zoom
    }

    /// 見出しの幅・高さ(表示タブで消せる。当たり判定も同じ値を使う)
    fn head_w(&self) -> f32 {
        if self.show_headers { HEAD_W } else { 0.0 }
    }
    fn head_h(&self) -> f32 {
        if self.show_headers { ROW_H } else { 0.0 }
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
            * self.zoom
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
        let (nr, nc) = (self.rows_snug(), self.cols_snug());
        if self.cursor.row < self.view.row {
            self.view.row = self.cursor.row;
        }
        if self.cursor.row >= self.view.row + nr {
            self.view.row = self.cursor.row + 1 - nr;
        }
        if self.cursor.col < self.view.col {
            self.view.col = self.cursor.col;
        }
        if self.cursor.col >= self.view.col + nc {
            self.view.col = self.cursor.col + 1 - nc;
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
        let bytes = match std::fs::read(&p) {
            Ok(b) => b,
            Err(e) => {
                self.status = format!("開けません: {e}").into();
                return;
            }
        };
        if ooxml::crypt::is_encrypted(&bytes) {
            // 板でパスワードを聞き、Enter が続きをやる
            self.pw_pending = Some(p);
            self.prompt = Some(("pw-open", Editor::new("")));
            self.status =
                ui::t!("このブックは暗号化されています。パスワードを打って Enter").into();
            return;
        }
        self.open_plain(p, bytes);
    }

    /// 平文(zip)の xlsx を読み込む。open とパスワードの板の共通の続き。
    fn open_plain(&mut self, p: PathBuf, bytes: Vec<u8>) {
        // 前のブックのパスワードを引きずらない(暗号化して開いた時だけ
        // 板の続きが後から入れ直す)
        self.encrypt_pw = None;
        match sheet::xlsx::read(std::io::Cursor::new(bytes)) {
            Ok((mut book, rep)) => {
                sheet::recalc_all(&mut book);
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
                Self::note_recent(&p);
                self.path = Some(p);
                self.sync_input();
            }
            Err(e) => self.status = format!("開けません: {e}").into(),
        }
    }

    /// 上書きの前に、直前の中身を控えとして残す(最大9世代)。writer と
    /// 同じ作法: 同じフォルダの .jo-history/<ファイル名>/<日時>.xlsx。
    /// 名前は**その中身を保存した日時**(mtime)— いつの姿かが分かる。
    fn keep_version(&self, p: &std::path::Path) {
        let Some(name) = p.file_name().map(|n| n.to_string_lossy().to_string()) else {
            return;
        };
        let dir = p
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join(".jo-history")
            .join(&name);
        if std::fs::create_dir_all(&dir).is_err() {
            return; // 控えられなくても保存は止めない
        }
        let stamp = std::process::Command::new("date")
            .arg("-r")
            .arg(p)
            .arg("+%Y%m%d-%H%M%S")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|| "0".into());
        let _ = std::fs::copy(p, dir.join(format!("{stamp}.xlsx")));
        if let Ok(rd) = std::fs::read_dir(&dir) {
            let mut old: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
            old.sort();
            while old.len() > 9 {
                let _ = std::fs::remove_file(old.remove(0));
            }
        }
    }

    /// 控えの一覧(新しい順)。(表示名, パス)
    fn versions(&self) -> Vec<(String, PathBuf)> {
        let Some(p) = &self.path else { return Vec::new() };
        let Some(name) = p.file_name().map(|n| n.to_string_lossy().to_string()) else {
            return Vec::new();
        };
        let dir = p
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join(".jo-history")
            .join(&name);
        let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
        let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        v.sort();
        v.reverse();
        v.into_iter()
            .map(|q| {
                let stem = q
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                // 20260804-183012 → 2026-08-04 18:30
                let disp = if stem.len() >= 13 && stem.is_ascii() {
                    format!(
                        "{}-{}-{} {}:{}",
                        &stem[0..4], &stem[4..6], &stem[6..8], &stem[9..11], &stem[11..13]
                    )
                } else {
                    stem
                };
                let kb = std::fs::metadata(&q).map(|m| m.len() / 1024).unwrap_or(0);
                (format!("{disp}({kb} KB)"), q)
            })
            .collect()
    }

    /// 控えを開く。いまのファイルは動かさず、**名無しの複製**として読む
    /// (保存すると名前を聞く。元へ戻したいなら同じ名前で保存する —
    /// 黙って元のファイルを書き戻したりしない)。
    fn open_version(&mut self, q: &std::path::Path) {
        let raw = match std::fs::read(q) {
            Ok(b) => b,
            Err(e) => {
                self.status = format!("控えが読めません: {e}").into();
                return;
            }
        };
        let raw = if ooxml::crypt::is_encrypted(&raw) {
            match self.encrypt_pw.as_ref().map(|pw| ooxml::crypt::decrypt(&raw, pw)) {
                Some(Ok(b)) => b,
                _ => {
                    self.status =
                        ui::t!("控えは暗号化されています(いまのパスワードでは解けません)").into();
                    return;
                }
            }
        } else {
            raw
        };
        match sheet::xlsx::read(std::io::Cursor::new(raw)) {
            Ok((mut book, _rep)) => {
                sheet::recalc_all(&mut book);
                self.release_lock();
                self.locked_by = None;
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
                self.path = None;
                self.dirty = true;
                self.sync_input();
                self.status = ui::t!("控えを開きました(名無しの複製。保存で名前を聞きます。元へ戻すなら同じ名前で保存)").into();
            }
            Err(e) => self.status = format!("控えが読めません: {e}").into(),
        }
    }

    /// 原本の中身(暗号化されていれば解いた平文)。部品の持ち越しに使う
    fn original_plain(&self) -> Option<Vec<u8>> {
        let bytes = std::fs::read(self.path.as_ref()?).ok()?;
        if ooxml::crypt::is_encrypted(&bytes) {
            let pw = self.encrypt_pw.as_ref()?;
            ooxml::crypt::decrypt(&bytes, pw).ok()
        } else {
            Some(bytes)
        }
    }

    /// 選択の生きた値(Excel の下端と同じ 合計・平均・個数)。
    /// 2セル以上を選んでいて、数のセルがあるときだけ出す。
    fn sel_stats(&self) -> Option<String> {
        self.anchor?;
        let (a, b) = self.sel_rect();
        let cells = (b.row - a.row + 1) as u64 * (b.col - a.col + 1) as u64;
        // 全選択のような巨大な矩形は数えない(描画のたびに走るので)
        if cells < 2 || cells > 200_000 {
            return None;
        }
        let sh = self.sheet();
        let mut n = 0u64;
        let mut sum = 0.0f64;
        for r in a.row..=b.row {
            for c in a.col..=b.col {
                if let Some(Value::Number(v)) = sh.get(Pos::new(r, c)).map(|x| &x.value) {
                    n += 1;
                    sum += *v;
                }
            }
        }
        if n == 0 {
            return None;
        }
        let avg = (sum / n as f64 * 100.0).round() / 100.0;
        Some(format!(
            "合計 {} / 平均 {} / 個数 {n}",
            Value::Number(sum).display(),
            Value::Number(avg).display()
        ))
    }

    /// チャット(申し送り帳)の置き場。ブックの隣の 名前.xlsx.chat.txt
    fn chat_path(&self) -> Option<PathBuf> {
        self.path.as_ref().map(|p| {
            let mut os = p.as_os_str().to_owned();
            os.push(".chat.txt");
            PathBuf::from(os)
        })
    }

    /// 最近開いた・保存したブックの控え(writer と同じ作法)
    fn recent_file() -> PathBuf {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(".config/office/recent-calc.txt")
    }

    fn note_recent(p: &std::path::Path) {
        let rf = Self::recent_file();
        if let Some(dir) = rf.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut list: Vec<String> = std::fs::read_to_string(&rf)
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_default();
        let me = p.to_string_lossy().to_string();
        list.retain(|x| *x != me);
        list.insert(0, me);
        list.truncate(12);
        let _ = std::fs::write(&rf, list.join("\n"));
    }

    fn recent_list() -> Vec<PathBuf> {
        std::fs::read_to_string(Self::recent_file())
            .map(|s| s.lines().map(PathBuf::from).filter(|p| p.exists()).collect())
            .unwrap_or_default()
    }

    /// 新しいブック。未保存の変更があるときは作らない(黙って捨てない)。
    fn new_book(&mut self) -> bool {
        if self.dirty {
            self.status =
                ui::t!("未保存の変更があります。先に保存してください(Ctrl+S)").into();
            return false;
        }
        self.release_lock();
        self.locked_by = None;
        self.path = None;
        self.encrypt_pw = None;
        self.notes = Vec::new();
        self.book = Book::new();
        self.active = 0;
        self.cursor = Pos::new(0, 0);
        self.view = Pos::new(0, 0);
        self.anchor = None;
        self.frozen = None;
        self.filter = None;
        self.slicer = None;
        self.sheet_ui.clear();
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.dirty = false;
        self.sync_input();
        self.status = ui::t!("新しいブックです").into();
        true
    }

    /// 名前を付けて保存(いつでもダイアログ。別の糸 — rfd は同期)
    fn save_as(&mut self, cx: &mut Context<Self>) {
        let ask = cx.background_executor().spawn(async {
            rfd::FileDialog::new().add_filter("Excelブック", &["xlsx"]).save_file()
        });
        cx.spawn(async move |this, cx| {
            let r = ask.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(mut p) = r {
                    if p.extension().is_none() {
                        p.set_extension("xlsx");
                    }
                    this.save_to(p);
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ---- 割り当てられた操作 ----
    fn a_backspace(&mut self, _: &ui::Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(ed) = &mut self.name_edit {
            ed.backspace();
        } else if self.fn_args.is_some() {
            self.editor().backspace();
            self.fn_args_recalc();
        } else if let Some(d) = &mut self.fn_dlg {
            d.search.backspace();
            d.sel = 0;
        } else if let Some(sv) = &mut self.solver {
            sv.focused().backspace();
        } else if let Some((_, ed)) = &mut self.prompt {
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
        recalc_book(&mut self.book, self.active);
        self.dirty = true;
        self.sync_input();
        n
    }

    fn a_delete(&mut self, _: &ui::Delete, _: &mut Window, cx: &mut Context<Self>) {
        if self.sheet().protected {
            self.status =
                ui::t!("シートが保護されています(保護タブの「保護」で解除)").into();
            cx.notify();
            return;
        }
        if let Some(i) = self.shape_sel.take() {
            if self.sheet().shapes_new.len() > i {
                self.checkpoint();
                self.sheet_mut().shapes_new.remove(i);
                self.dirty = true;
                self.status = ui::t!("図形を削除しました(Ctrl+Z で戻せます)").into();
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
        self.copy_now(cx)
    }
    fn copy_now(&mut self, cx: &mut Context<Self>) {
        if self.input.has_selection() {
            // 数式バーの文字を選んでいるなら、その文字のコピー
            let sel = self.input.selection();
            if let Some(s) = self.input.text().get(sel) {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(s.to_string()));
                self.status = ui::t!("コピーしました").into();
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
        self.cut_now(cx)
    }
    fn cut_now(&mut self, cx: &mut Context<Self>) {
        if self.input.has_selection() {
            let sel = self.input.selection();
            if let Some(s) = self.input.text().get(sel) {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(s.to_string()));
                self.input.insert("");
                self.dirty = true;
                self.status = ui::t!("切り取りました").into();
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
        self.paste_now(cx)
    }
    fn paste_now(&mut self, cx: &mut Context<Self>) {
        if self.sheet().protected {
            self.status =
                ui::t!("シートが保護されています(保護タブの「保護」で解除)").into();
            cx.notify();
            return;
        }
        let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) else {
            self.status = ui::t!("貼り付けるものがありません").into();
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
        recalc_book(&mut self.book, self.active);
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
        // 小窓 → 板 → 打ちかけの文字 → セル、の順で見る
        if let Some(ed) = &mut self.name_edit { ed.move_char(false, false) }
        else if self.fn_args.is_some() { self.editor().move_char(false, false) }
        else if let Some(d) = &mut self.fn_dlg { d.search.move_char(false, false) }
        else if let Some(sv) = &mut self.solver { sv.focused().move_char(false, false) }
        else if let Some((_, ed)) = &mut self.prompt { ed.move_char(false, false) }
        else if self.editing() || self.edit_armed { self.input.move_char(false, false) }
        else { self.move_cursor(0, -1) }
        cx.notify();
    }
    fn a_right(&mut self, _: &ui::Right, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(ed) = &mut self.name_edit { ed.move_char(true, false) }
        else if self.fn_args.is_some() { self.editor().move_char(true, false) }
        else if let Some(d) = &mut self.fn_dlg { d.search.move_char(true, false) }
        else if let Some(sv) = &mut self.solver { sv.focused().move_char(true, false) }
        else if let Some((_, ed)) = &mut self.prompt { ed.move_char(true, false) }
        else if self.editing() || self.edit_armed { self.input.move_char(true, false) }
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
        self.move_cursor(-(self.rows_snug() as i32 - 1).max(1), 0);
        cx.notify();
    }
    fn a_page_down(&mut self, _: &ui::PageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_cursor((self.rows_snug() as i32 - 1).max(1), 0);
        cx.notify();
    }
    fn a_up(&mut self, _: &ui::Up, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(a) = &mut self.fn_args {
            a.focus = a.focus.saturating_sub(1);
        } else if let Some(d) = &mut self.fn_dlg {
            d.sel = d.sel.saturating_sub(1);
        } else {
            self.move_cursor(-1, 0);
        }
        cx.notify();
    }
    fn a_down(&mut self, _: &ui::Down, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(a) = &mut self.fn_args {
            a.focus = (a.focus + 1).min(a.eds.len().saturating_sub(1));
        } else if let Some(d) = &mut self.fn_dlg {
            let n = fn_filtered(d.search.text(), d.group).len();
            d.sel = (d.sel + 1).min(n.saturating_sub(1));
        } else {
            self.move_cursor(1, 0);
        }
        cx.notify();
    }
    fn a_tab(&mut self, _: &ui::Tab, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(a) = &mut self.fn_args {
            if !a.eds.is_empty() {
                a.focus = (a.focus + 1) % a.eds.len();
            }
        } else {
            self.move_cursor(0, 1);
        }
        cx.notify();
    }
    fn a_enter(&mut self, _: &ui::Enter, _: &mut Window, cx: &mut Context<Self>) {
        if self.quit_ask {
            // Enter = 保存して終了(いちばん安全な既定)
            self.quit_ask = false;
            self.save(true, cx);
            cx.notify();
            return;
        }
        if self.name_edit.is_some() {
            self.commit_name_box();
            cx.notify();
            return;
        }
        if self.fn_args.is_some() {
            self.fn_args_ok();
            cx.notify();
            return;
        }
        if self.fn_dlg.is_some() {
            self.fn_next();
            cx.notify();
            return;
        }
        if self.solver.is_some() {
            // 小窓の Enter では何も走らせない(解くのは「解を求める」の釦)
            cx.notify();
            return;
        }
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
        self.select_all_now();
        cx.notify();
    }
    /// 全選択の実体。Ctrl+A ともリボンの「すべて選択」とも同じ道を通す
    /// (リボンだけバーの文字選択、という別物にしない)
    fn select_all_now(&mut self) {
        if self.editing() {
            // 打ちかけの間は、バーの文字の全選択
            self.input.select_all();
        } else {
            // 使われている範囲の全選択(表計算の Ctrl+A)
            let (rows, cols) = self.sheet().extent();
            if rows == 0 {
                self.status = ui::t!("空の表です").into();
            } else {
                self.commit();
                self.anchor = Some(Pos::new(0, 0));
                self.cursor = Pos::new(rows - 1, cols.saturating_sub(1));
                self.status = format!("A1:{} を選択しました", self.cursor.a1()).into();
                self.sync_input();
            }
        }
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
        // 確認を出すのは**未保存の変更があるとき**。名前の無い新規でも、
        // 何か打ってあれば出す — 打った物を黙って捨てない(発注者 2026-08-06。
        // 2026-08-03 の「実ファイルに限る」を改訂: 新規が見本入りだった頃は
        // 「試し打ち」扱いでよかったが、空白の新規は実の仕事が始まる場所)。
        // 本当に空のままの新規は、従来どおり黙って閉じる(煩くしない)
        let empty_new = self.path.is_none()
            && self.book.sheets.iter().all(|s| s.cells.is_empty());
        if !self.dirty || empty_new {
            self.release_lock();
            cx.quit();
            return;
        }
        // 確認は**窓の中の板**で出す。rfd の OS ダイアログは親窓を持てず
        // **スクリーンの中央**に出て、窓から離れすぎる(発注者 2026-08-06)
        self.quit_ask = true;
        cx.notify();
    }

    fn a_quit(&mut self, _: &ui::Quit, _: &mut Window, cx: &mut Context<Self>) {
        self.request_quit(cx);
    }

    /// リボンのコマンド。数式タブは選択セルに関数を入れる。
    /// 選んでいるセルの見た目を変える。
    ///
    /// **値の無いセルにも掛ける** — 罫線だけを引くのは帳票では普通の操作。
    fn fmt(&mut self, f: impl Fn(&mut CellFormat)) {
        if self.sheet().protected {
            self.status =
                ui::t!("シートが保護されています(保護タブの「保護」で解除)").into();
            return;
        }
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
        recalc_book(&mut self.book, self.active);
    }

    /// 選んだ範囲を結合する。**値は消さない** — 左上以外の値は隠れるだけで、
    /// 結合を解けば戻る(黙って捨てない)。
    fn merge_selection(&mut self) {
        self.checkpoint();
        let (a, b) = self.sel_rect();
        if a == b {
            self.status = ui::t!("結合する範囲を Shift+矢印で選んでください").into();
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
        recalc_book(&mut self.book, self.active);
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
            let (sx, sy) = (sx + sp.dx_px, sy + sp.dy_px);
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
            // 移動: 掴んだときのずれを保って、左上の来るセルに留め直す。
            // セルからのはみ出しは px のずらしとして持つ(位置が飛ばない)
            let (nx, ny) = (ox + x - gx, oy + y - gy);
            if let (Some(c), Some(r)) = (self.col_at(nx.max(HEAD_W)), self.row_at(ny.max(ROW_H))) {
                let at = Pos::new(r, c);
                if let Some((cx0, cy0)) = self.cell_origin_px(at) {
                    let (dx, dy) = ((nx - cx0).max(0.0), (ny - cy0).max(0.0));
                    let sp = &mut self.sheet_mut().shapes_new[i];
                    if sp.at != at || (sp.dx_px - dx).abs() > 0.5 || (sp.dy_px - dy).abs() > 0.5 {
                        sp.at = at;
                        sp.dx_px = dx;
                        sp.dy_px = dy;
                        self.dirty = true;
                        self.status = format!("図形を {} に留めました", at.a1()).into();
                    }
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
            self.status = ui::t!("グラフにする範囲を選んでください(1列目が項目名、2列目からが数)").into();
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
        self.status = ui::t!("グラフを描いています…").into();
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

    /// ピボットテーブルの挿入(polars が裏方)。指図(PivotDef)を組んで
    /// 回すだけ — 置き直せるように指図はブックに控える(xl/joPivot.xml)。
    fn insert_pivot(
        &mut self,
        pend: PivotPend,
        value: String,
        agg: &'static str,
        cx: &mut Context<Self>,
    ) {
        let def = sheet::model::PivotDef {
            sheet: self.book.sheets[self.active].name.clone(),
            src: (pend.a, pend.b),
            rows_sel: pend.rows_sel,
            cols_sel: pend.cols_sel,
            value,
            agg: agg.to_string(),
            totals: false,
            subtotals: false,
            blank_rows: false,
            compact: false,
            dest: pend.a, // 仮 — 置くときに右の空きを探して決める
            size: (0, 0),
        };
        self.spawn_pivot(def, None, cx);
    }

    /// いまのシートで、この位置に置いてあるピボットの指図の番号。
    fn pivot_at(&self, p: Pos) -> Option<usize> {
        let name = &self.book.sheets[self.active].name;
        self.book.pivots.iter().position(|d| {
            d.sheet == *name
                && d.size.0 > 0
                && p.row >= d.dest.row
                && p.row < d.dest.row + d.size.0
                && p.col >= d.dest.col
                && p.col < d.dest.col + d.size.1
        })
    }

    /// 集計の面をセルに書く。種別で見た目を付ける(h=見出しの帯、
    /// s=小計 t=総計は太字、t は上罫線も)。
    fn place_pivot_grid(&mut self, si: usize, at: Pos, grid: &[Vec<String>], kinds: &[char]) {
        paste_values_text(&mut self.book.sheets[si], at, grid);
        let w = grid.iter().map(|r| r.len()).max().unwrap_or(1) as u32;
        for (i, k) in kinds.iter().enumerate() {
            if !matches!(k, 'h' | 's' | 't') {
                continue;
            }
            for c in 0..w {
                let p = Pos::new(at.row + i as u32, at.col + c);
                let mut cell = self.book.sheets[si].get(p).cloned().unwrap_or_default();
                cell.fmt.bold = true;
                if *k == 'h' {
                    cell.fmt.fill = Some("D5E8DC".into());
                }
                if *k == 't' {
                    cell.fmt.borders.top = true;
                }
                self.book.sheets[si].set(p, cell);
            }
        }
    }

    /// 指図どおりに polars を回して置く。replace=None は挿入(右の空きを探す)、
    /// Some(i) は i 番の指図の更新(同じ場所に置き直す)。
    fn spawn_pivot(
        &mut self,
        mut def: sheet::model::PivotDef,
        replace: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        let Some(si) = self.book.sheets.iter().position(|s| s.name == def.sheet) else {
            self.status = format!("シート「{}」がありません(ピボットの元の表)", def.sheet).into();
            return;
        };
        let (a, b) = def.src;
        let sh = &self.book.sheets[si];
        let headers: Vec<String> = (a.col..=b.col)
            .map(|c| {
                let v = sh.get(Pos::new(a.row, c)).map(|x| x.value.display()).unwrap_or_default();
                if v.is_empty() { col_name(c) } else { v }
            })
            .collect();
        let data: Vec<Vec<String>> = (a.row + 1..=b.row)
            .map(|r| {
                (a.col..=b.col)
                    .map(|c| sh.get(Pos::new(r, c)).map(|x| x.value.display()).unwrap_or_default())
                    .collect()
            })
            .collect();
        let json = pivot_spec_json(&headers, &data, &def);
        let dir = std::env::temp_dir().join(format!("jo-pivot-{}", std::process::id()));
        self.status = format!("{} の {} を集めています…", def.value, def.agg).into();
        let task = cx.background_executor().spawn(async move {
            let _ = std::fs::create_dir_all(&dir);
            let json_path = dir.join("pivot.json");
            let py_path = dir.join("pivot.py");
            std::fs::write(&json_path, json).map_err(|e| e.to_string())?;
            std::fs::write(&py_path, PIVOT_PY).map_err(|e| e.to_string())?;
            let o = std::process::Command::new(find_python())
                .arg(&py_path)
                .arg(&json_path)
                .output()
                .map_err(|e| format!("Python が起動できません: {e}"))?;
            if !o.status.success() {
                let err = String::from_utf8_lossy(&o.stderr);
                let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("原因不明");
                return Err(if err.contains("No module named") {
                    format!("polars がありません({last})。pip で入れてください")
                } else {
                    format!("集計できません: {last}")
                });
            }
            Ok(String::from_utf8_lossy(&o.stdout).to_string())
        });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok(raw) => {
                        let (grid, kinds) = parse_pivot_grid(&raw);
                        let h = grid.len() as u32;
                        let w = grid.iter().map(|r| r.len()).max().unwrap_or(1) as u32;
                        let used = |this: &Self, p: Pos| {
                            this.book.sheets[si]
                                .get(p)
                                .map(|cell| {
                                    !cell.value.display().is_empty() || cell.formula.is_some()
                                })
                                .unwrap_or(false)
                        };
                        match replace {
                            None => {
                                // 右の空きを探す(埋まっていたらさらに右へ。黙って上書きしない)
                                let mut dc = b.col + 2;
                                let mut tries = 0;
                                let free = loop {
                                    let occupied = (0..h).any(|r| {
                                        (0..w).any(|c| used(this, Pos::new(a.row + r, dc + c)))
                                    });
                                    if !occupied {
                                        break true;
                                    }
                                    dc += w + 1;
                                    tries += 1;
                                    if tries > 50 {
                                        break false;
                                    }
                                };
                                if !free {
                                    this.status =
                                        ui::t!("右に空きが見つかりません(場所を空けてから)").into();
                                } else {
                                    this.checkpoint_book();
                                    def.dest = Pos::new(a.row, dc);
                                    def.size = (h, w);
                                    let at = def.dest;
                                    this.place_pivot_grid(si, at, &grid, &kinds);
                                    recalc_book(&mut this.book, si);
                                    let (value, agg) = (def.value.clone(), def.agg.clone());
                                    this.book.pivots.push(def);
                                    this.dirty = true;
                                    this.sync_input();
                                    this.status = format!(
                                        "ピボット({value} の {agg})を {} に置きました — その時の値。元が変わったら「更新」(Ctrl+Z で戻せます)",
                                        at.a1()
                                    )
                                    .into();
                                }
                            }
                            Some(pi) => {
                                let Some(old) = this.book.pivots.get(pi).cloned() else {
                                    return;
                                };
                                let dest = old.dest;
                                let in_old = |p: Pos| {
                                    p.row >= dest.row
                                        && p.row < dest.row + old.size.0
                                        && p.col >= dest.col
                                        && p.col < dest.col + old.size.1
                                };
                                let occupied = (0..h).any(|r| {
                                    (0..w).any(|c| {
                                        let p = Pos::new(dest.row + r, dest.col + c);
                                        !in_old(p) && used(this, p)
                                    })
                                });
                                if occupied {
                                    this.status =
                                        ui::t!("広がった分の場所が塞がっています(右下を空けてから更新)").into();
                                } else {
                                    this.checkpoint_book();
                                    for r in 0..old.size.0 {
                                        for c in 0..old.size.1 {
                                            this.book.sheets[si]
                                                .cells
                                                .remove(&Pos::new(dest.row + r, dest.col + c));
                                        }
                                    }
                                    def.dest = dest;
                                    def.size = (h, w);
                                    this.place_pivot_grid(si, dest, &grid, &kinds);
                                    recalc_book(&mut this.book, si);
                                    this.book.pivots[pi] = def;
                                    this.dirty = true;
                                    this.sync_input();
                                    this.status = format!(
                                        "ピボットを更新しました({} — その時の値。Ctrl+Z で戻せます)",
                                        dest.a1()
                                    )
                                    .into();
                                }
                            }
                        }
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
        self.status = ui::t!("Python を実行しています…").into();
        let task = cx.background_executor().spawn(async move {
            let py_path = dir.join("run.py");
            std::fs::write(&py_path, script).map_err(|e| e.to_string())?;
            let py = find_python();
            let have_bwrap = std::path::Path::new("/usr/bin/bwrap").exists();
            if sandbox && !have_bwrap {
                return Err(
                    ui::t!("檻(bubblewrap)がありません。ブックに載ったコードは檻の外では\
実行しません(apt install bubblewrap)").to_string(),
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
                    ui::t!("office_sheet.so がありません(cargo build -p pysheet --release \
--features extension-module して、liboffice_sheet.so を office_sheet.so の名で \
calc の隣に置いてください)").to_string()
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
                                sheet::recalc_all(&mut book);
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
                                    ui::t!("Python を実行しました(Ctrl+Z で1手で戻せます)").into()
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
            self.status = ui::t!("=PY(\"関数名\", 引数…) のセルがありません").into();
            return;
        }
        if defs.trim().is_empty() {
            self.status =
                ui::t!("関数の定義がありません(@save 関数 で def の入った .py をブックに載せる)").into();
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
        self.status = ui::t!("PY を計算しています…(檻の中)").into();
        let task = cx.background_executor().spawn(async move {
            if !std::path::Path::new("/usr/bin/bwrap").exists() {
                return Err(
                    ui::t!("檻(bubblewrap)がありません。ブックの関数は檻の外では計算しません").to_string(),
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
                            recalc_book(&mut this.book, i);
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
                return Some(Err(ui::t!("一時ファイルが書けません").to_string()));
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
                        recalc_book(&mut this.book, this.active);
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

    /// 板の文字を Python の台本で絵にして、画像としてシートに浮かべる。
    /// writer の方式(図は Python で描いて画像で貼る)の自動化 —
    /// 方程式(EQ_PY)とテキストアート(TEXTART_PY)が同じ道を通る。
    fn insert_py_image(
        &mut self,
        script: &'static str,
        name: &'static str,
        tex: String,
        cx: &mut Context<Self>,
    ) {
        let esc = |t: &str| t.replace('\\', "\\\\").replace('"', "\\\"");
        let dir =
            std::env::temp_dir().join(format!("jo-{name}-{}", std::process::id()));
        let out = dir.join("eq.png");
        let font = kumihan::font::for_document(None)
            .ok()
            .map(|(fam, _)| fam.path.to_string_lossy().to_string())
            .unwrap_or_default();
        let json = format!(
            "{{\"tex\":\"{}\",\"font\":\"{}\",\"out\":\"{}\"}}",
            esc(&tex),
            esc(&font),
            esc(&out.to_string_lossy())
        );
        let at = self.cursor;
        self.status = ui::t!("清書しています…").into();
        let task = cx.background_executor().spawn(async move {
            let _ = std::fs::create_dir_all(&dir);
            let json_path = dir.join("eq.json");
            let py_path = dir.join("eq.py");
            std::fs::write(&json_path, json).map_err(|e| e.to_string())?;
            std::fs::write(&py_path, script).map_err(|e| e.to_string())?;
            let o = std::process::Command::new(find_python())
                .arg(&py_path)
                .arg(&json_path)
                .output()
                .map_err(|e| format!("Python が起動できません: {e}"))?;
            if !o.status.success() {
                let err = String::from_utf8_lossy(&o.stderr);
                let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("原因不明");
                return Err(if err.contains("No module named") {
                    format!("matplotlib がありません({last})")
                } else {
                    format!("式が読めません: {last}")
                });
            }
            std::fs::read(&out).map_err(|e| e.to_string())
        });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok(data) => {
                        let (w, h) = image_px(&data).unwrap_or((200, 60));
                        this.checkpoint();
                        // 200dpi で描いたので画面では半分の大きさに置く
                        this.sheet_mut().images_new.push(sheet::model::SheetImage {
                            at,
                            width_px: w as f32 / 2.0,
                            height_px: h as f32 / 2.0,
                            data,
                        });
                        this.dirty = true;
                        this.status = format!(
                            "{} を {} に置きました(画像。保存で xlsx に入ります。Ctrl+Z で1手)",
                            if name == "eq" { "方程式" } else { "テキストアート" },
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

    /// SmartArt を図形の集まりとして組む。1手 = checkpoint 一回(全部まとめて
    /// Ctrl+Z で戻る)。各図形は普通の図形なので、選んで動かす・Enter で
    /// 文字を書く・Del で消す、が全部効く。xlsx へは prstGeom の図形として
    /// 入る(Excel でも図形として見える。本物の SmartArt 部品ではない)。
    fn insert_smartart(&mut self, name: &str, key: &str) {
        self.checkpoint();
        let at = self.cursor;
        let (g, l) = (Some("D5E8DC".to_string()), Some("1B6E3C".to_string()));
        // (dx, dy, w, h, kind, 塗り?, 文字?)
        let mut parts: Vec<(f32, f32, f32, f32, &str, bool, bool)> = Vec::new();
        match key {
            "block-list" => {
                for i in 0..3 {
                    parts.push((i as f32 * 140.0, 0.0, 128.0, 72.0, "roundRect", true, true));
                }
            }
            "vbox-list" => {
                for i in 0..3 {
                    parts.push((0.0, i as f32 * 60.0, 240.0, 48.0, "rect", true, true));
                }
            }
            "pyramid-list" => {
                for i in 0..3 {
                    let w = 160.0 + i as f32 * 60.0;
                    parts.push(((280.0 - w) / 2.0, i as f32 * 58.0, w, 48.0, "roundRect", true, true));
                }
            }
            "basic-process" => {
                for i in 0..3 {
                    parts.push((i as f32 * 164.0, 0.0, 120.0, 56.0, "rect", true, true));
                    if i < 2 {
                        parts.push((i as f32 * 164.0 + 124.0, 16.0, 36.0, 24.0, "rightArrow", true, false));
                    }
                }
            }
            "chevron-process" => {
                for i in 0..3 {
                    parts.push((i as f32 * 140.0, 0.0, 150.0, 56.0, "rightArrow", true, true));
                }
            }
            "timeline" => {
                parts.push((0.0, 46.0, 420.0, 3.0, "rect", true, false)); // 軸
                for i in 0..3 {
                    parts.push((30.0 + i as f32 * 150.0, 38.0, 18.0, 18.0, "ellipse", true, false));
                    parts.push((6.0 + i as f32 * 150.0, 0.0, 100.0, 32.0, "rect", false, true));
                }
            }
            "basic-cycle" => {
                parts.push((110.0, 0.0, 110.0, 64.0, "ellipse", true, true));
                parts.push((0.0, 110.0, 110.0, 64.0, "ellipse", true, true));
                parts.push((220.0, 110.0, 110.0, 64.0, "ellipse", true, true));
            }
            "block-cycle" => {
                parts.push((105.0, 0.0, 120.0, 48.0, "rect", true, true));
                parts.push((220.0, 78.0, 120.0, 48.0, "rect", true, true));
                parts.push((105.0, 156.0, 120.0, 48.0, "rect", true, true));
                parts.push((0.0, 78.0, 120.0, 48.0, "rect", true, true));
            }
            "org-chart" | "hierarchy" => {
                let kids = if key == "org-chart" { 3 } else { 2 };
                let (w, gap) = (120.0, 40.0);
                let total = kids as f32 * w + (kids - 1) as f32 * gap;
                parts.push(((total - w) / 2.0, 0.0, w, 48.0, "rect", true, true));
                // 継ぎの線(細い棒): 親の下 → 横橋 → 子の上
                parts.push((total / 2.0 - 1.0, 48.0, 2.0, 22.0, "rect", true, false));
                parts.push((w / 2.0, 70.0, total - w, 2.0, "rect", true, false));
                for i in 0..kids {
                    let x = i as f32 * (w + gap);
                    parts.push((x + w / 2.0 - 1.0, 72.0, 2.0, 22.0, "rect", true, false));
                    parts.push((x, 94.0, w, 48.0, "rect", true, true));
                }
            }
            "venn" => {
                parts.push((0.0, 0.0, 150.0, 150.0, "ellipse", false, true));
                parts.push((90.0, 0.0, 150.0, 150.0, "ellipse", false, true));
                parts.push((45.0, 78.0, 150.0, 150.0, "ellipse", false, true));
            }
            "matrix" => {
                for r in 0..2 {
                    for c in 0..2 {
                        parts.push((c as f32 * 132.0, r as f32 * 62.0, 124.0, 54.0, "rect", true, true));
                    }
                }
            }
            "pyramid" => {
                for i in 0..3 {
                    let w = 120.0 + i as f32 * 90.0;
                    parts.push(((300.0 - w) / 2.0, i as f32 * 54.0, w, 48.0, "rect", true, true));
                }
            }
            _ => {}
        }
        let n = parts.len();
        for (dx, dy, w, h, kind, filled, texted) in parts {
            self.sheet_mut().shapes_new.push(sheet::model::SheetShape {
                at,
                dx_px: dx,
                dy_px: dy,
                width_px: w,
                height_px: h,
                kind: kind.into(),
                fill: if filled { g.clone() } else { None },
                line: l.clone(),
                text: if texted { Some(ui::t!("テキスト").into()) } else { None },
                ..Default::default()
            });
        }
        self.dirty = true;
        self.status = format!(
            "{name} を {} に置きました({n} 個の図形。図形を選んで Enter で文字、ドラッグで移動。全部まとめて Ctrl+Z で1手)",
            at.a1()
        )
        .into();
    }

    /// ソルバーを解く。係数は**表の複製の上で測る**(ゴールシークと同じ流儀):
    /// 変数を全部 0 → 単位ベクトル、で目的と制約左辺の一次係数を取り、
    /// 全部 1 の点で検算して**線形でなければ正直に断る**(単体法 LP は
    /// 線形の問題だけ — 本家 ONLYOFFICE の断り書きと同じ)。
    /// 解くのは scipy.optimize.linprog(highs)。
    fn solve_solver(&mut self, cx: &mut Context<Self>) {
        let Some(sv) = &self.solver else { return };
        // ---- 読み取りと検め ----
        let Some(target) = Pos::parse(&sv.target.text().replace('$', "").to_uppercase()) else {
            self.status = ui::t!("目的のセルが読めません(例: E6)").into();
            return;
        };
        let Some(vars) = parse_cell_list(&sv.vars.text(), 64) else {
            self.status = ui::t!("変数セルが読めません(例: B2:B4 や B2,C2。最大64個)").into();
            return;
        };
        let mode = sv.mode;
        let want = if mode == 2 {
            match sv.value.text().trim().parse::<f64>() {
                Ok(v) => v,
                Err(_) => {
                    self.status = ui::t!("「値」が数として読めません").into();
                    return;
                }
            }
        } else {
            0.0
        };
        // 制約: (セル, op, 右辺の数)。左辺は範囲なら1セルずつの行になる
        let mut rows: Vec<(Pos, usize, f64)> = Vec::new();
        for (l, op, r) in &sv.cons {
            let Some(cells) = parse_cell_list(l, 256) else {
                self.status = format!("制約の左辺が読めません: {l}").into();
                return;
            };
            let opi = SOLVER_OPS.iter().position(|o| o == op).unwrap_or(0);
            // 右辺: 数か、セルの今の値
            let rhs = match r.trim().parse::<f64>() {
                Ok(v) => v,
                Err(_) => match Pos::parse(&r.replace('$', "").to_uppercase()) {
                    Some(p) => self
                        .sheet()
                        .get(p)
                        .map(|c| c.value.as_number())
                        .unwrap_or(0.0),
                    None => {
                        self.status = format!("制約の右辺が読めません: {r}").into();
                        return;
                    }
                },
            };
            for c in cells {
                rows.push((c, opi, rhs));
            }
        }
        // ---- 係数の抽出(表の複製で測る)----
        let base = self.sheet().clone();
        let eval = |xs: &[f64]| -> (f64, Vec<f64>) {
            let mut s = base.clone();
            for (i, p) in vars.iter().enumerate() {
                s.set(*p, Cell::input(&format!("{}", xs[i])));
            }
            recalc(&mut s);
            let g = |p: Pos| s.get(p).map(|c| c.value.as_number()).unwrap_or(0.0);
            (g(target), rows.iter().map(|(p, _, _)| g(*p)).collect())
        };
        let n = vars.len();
        let zeros = vec![0.0; n];
        let (f0, c0) = eval(&zeros);
        let mut obj = vec![0.0; n];
        let mut a: Vec<Vec<f64>> = vec![vec![0.0; n]; rows.len()];
        for i in 0..n {
            let mut xs = zeros.clone();
            xs[i] = 1.0;
            let (fi, ci) = eval(&xs);
            obj[i] = fi - f0;
            for (k, v) in ci.iter().enumerate() {
                a[k][i] = v - c0[k];
            }
        }
        // 線形の検算(全部 1 の点)
        let ones = vec![1.0; n];
        let (f1, c1) = eval(&ones);
        let lin = |measured: f64, base: f64, coefs: &[f64]| -> bool {
            let predicted = base + coefs.iter().sum::<f64>();
            (measured - predicted).abs() <= 1e-6 * measured.abs().max(1.0)
        };
        let mut linear = lin(f1, f0, &obj);
        for k in 0..rows.len() {
            linear = linear && lin(c1[k], c0[k], &a[k]);
        }
        if !linear {
            self.status =
                ui::t!("線形ではありません — 単体法 LP は線形の問題だけを解きます(非線形は未対応。本家と同じ)").into();
            return;
        }
        // ---- LP に組む ----
        // 目的: 最大=係数を負に、最小=そのまま、値=目的0で f=want を等式に
        let mut aub: Vec<Vec<f64>> = Vec::new();
        let mut bub: Vec<f64> = Vec::new();
        let mut aeq: Vec<Vec<f64>> = Vec::new();
        let mut beq: Vec<f64> = Vec::new();
        for (k, (_, opi, rhs)) in rows.iter().enumerate() {
            let row = a[k].clone();
            let b = rhs - c0[k];
            match opi {
                0 => {
                    aub.push(row);
                    bub.push(b);
                }
                1 => {
                    aeq.push(row);
                    beq.push(b);
                }
                _ => {
                    aub.push(row.iter().map(|v| -v).collect());
                    bub.push(-b);
                }
            }
        }
        let c: Vec<f64> = match mode {
            0 => obj.iter().map(|v| -v).collect(),
            1 => obj.clone(),
            _ => {
                aeq.push(obj.clone());
                beq.push(want - f0);
                vec![0.0; n]
            }
        };
        // ---- JSON → scipy ----
        let arr = |v: &[f64]| {
            v.iter().map(|x| format!("{x}")).collect::<Vec<_>>().join(",")
        };
        let mat = |m: &[Vec<f64>]| {
            m.iter().map(|r| format!("[{}]", arr(r))).collect::<Vec<_>>().join(",")
        };
        let json = format!(
            "{{\"c\":[{}],\"aub\":[{}],\"bub\":[{}],\"aeq\":[{}],\"beq\":[{}],\"nonneg\":{}}}",
            arr(&c),
            mat(&aub),
            arr(&bub),
            mat(&aeq),
            arr(&beq),
            sv.nonneg
        );
        let dir = std::env::temp_dir().join(format!("jo-solver-{}", std::process::id()));
        self.status = ui::t!("解を探しています…(単体法 LP)").into();
        let task = cx.background_executor().spawn(async move {
            let _ = std::fs::create_dir_all(&dir);
            let json_path = dir.join("solver.json");
            let py_path = dir.join("solver.py");
            std::fs::write(&json_path, json).map_err(|e| e.to_string())?;
            std::fs::write(&py_path, SOLVER_PY).map_err(|e| e.to_string())?;
            let o = std::process::Command::new(find_python())
                .arg(&py_path)
                .arg(&json_path)
                .output()
                .map_err(|e| format!("Python が起動できません: {e}"))?;
            if !o.status.success() {
                let err = String::from_utf8_lossy(&o.stderr);
                let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("原因不明");
                return Err(if err.contains("No module named") {
                    format!("scipy がありません({last})。pip で入れてください")
                } else {
                    last.to_string()
                });
            }
            Ok(String::from_utf8_lossy(&o.stdout).to_string())
        });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok(out) => {
                        let xs: Vec<f64> = out
                            .split('\u{1f}')
                            .filter_map(|v| v.trim().parse().ok())
                            .collect();
                        if xs.len() != vars.len() {
                            this.status = format!("答えの形が違います: {out}").into();
                        } else {
                            this.checkpoint();
                            for (p, x) in vars.iter().zip(&xs) {
                                let x = (x * 1e9).round() / 1e9;
                                let fmt = this
                                    .sheet()
                                    .get(*p)
                                    .map(|c| c.fmt.clone())
                                    .unwrap_or_default();
                                let mut cell = Cell::input(&format!("{x}"));
                                cell.fmt = fmt;
                                this.book.sheets[this.active].set(*p, cell);
                            }
                            recalc_book(&mut this.book, this.active);
                            this.dirty = true;
                            this.sync_input();
                            this.solver = None;
                            let got = this
                                .sheet()
                                .get(target)
                                .map(|c| c.value.display())
                                .unwrap_or_default();
                            this.status = format!(
                                "解を求めました: {} = {got}(変数 {} 個を書き換え。Ctrl+Z で1手)",
                                target.a1(),
                                xs.len()
                            )
                            .into();
                        }
                    }
                    Err(e) => this.status = e.into(),
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
                recalc_book(&mut self.book, self.active);
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
                                    ui::t!("この画像は読めません(PNG か JPEG を選んでください)").into();
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
            desc.push(ui::t!("横向き").into());
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
        "copy", "cut", "paste",
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
        "instable", "table-tpl", "inssymbol", "pivot-insert",
        "pivot-refresh", "pivot-refresh-all", "pivot-select",
        "pivot-totals", "pivot-subtotals", "pivot-blank", "pivot-layout",
        "td-header", "td-total", "td-band-row", "td-band-col",
        "td-first", "td-last", "td-filter",
        "group", "ungroup", "hide-details", "show-details", "subtotal", "solver",
        "inssmartart", "insequation", "insslicer", "inscheckbox", "instextart",
        "coauth-mode", "co-delcomment", "co-showcomment", "co-chat",
        "co-history", "plug-macros", "plug-manage",
        "prot-doc", "prot-encrypt", "prot-sign",
        "zoom-in", "zoom-out", "formula-bar", "show-headings", "show-zeros",
        "subscript", "align-just", "text-orient", "calc-mode",
        "td-torange", "td-resize", "rtl-sheet", "direction",
        "colorschemas", "theme",
        "ai-where", "ai-summary", "ai-rewrite", "ai-polite", "ai-plain",
        "ai-translate", "ai-furigana", "ai-continue", "ai-table", "ai-ask",
        "insert-function", "cell-styles", "sheet-view", "watch",
        "pen", "highlighter", "eraser",
    ];

    /// シートの保護中でも通す操作(見るだけ・保存・保護の操作そのもの)
    const PROTECTED_OK: &'static [&'static str] = &[
        "open", "save", "pdf", "selectall", "undo", "redo",
        "freeze", "show-formulas", "show-gridlines",
        "setfilter", "clear-filter",
        "trace-prec", "trace-dep", "remove-arrows", "pivot-select",
        "coauth-mode", "co-showcomment", "co-chat", "co-history", "plug-manage",
        "prot-doc", "prot-encrypt", "prot-sign", "ai-where",
    ];

    fn run_cmd(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.sheet().protected && !Self::PROTECTED_OK.contains(&id) {
            self.status =
                ui::t!("シートが保護されています(保護タブの「保護」で解除)").into();
            cx.notify();
            return;
        }
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
            "selectall" => self.select_all_now(),
            "copy" => self.copy_now(cx),
            "cut" => self.cut_now(cx),
            "paste" => self.paste_now(cx),
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
                    self.status = ui::t!("Shift+↓ で埋める範囲を選んでください(先頭行を下へ写します)").into();
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
                    recalc_book(&mut self.book, self.active);
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
                    self.status = ui::t!("空のセルでは絞れません").into();
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
                self.status = ui::t!("絞り込みを解きました").into();
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
                    self.status = ui::t!("印刷範囲を解きました(全域を印刷します)").into();
                } else {
                    self.status =
                        ui::t!("印刷範囲にする範囲を Shift+矢印かドラッグで選んでください").into();
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
                    self.status = ui::t!("選択の中に英字がありません").into();
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
                    self.status = ui::t!("日本語の書体が見つかりません").into();
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
                    self.status = ui::t!("このセルの式は他のセルを参照していません").into();
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
                self.status = ui::t!("トレースを消しました").into();
            }
            // 推奨チャート = いまの一手(棒グラフ)をそのまま勧める
            "insrecommend" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status =
                        ui::t!("グラフにする範囲を選んでください(1列目が項目名、2列目からが数)").into();
                } else {
                    let (a, b) = self.sel_rect();
                    self.insert_chart(a, b, cx);
                }
            }
            // ピボットテーブル = polars が裏方。結果は「その時の値」で右に置く
            // (元が変わったら選び直してもう一度 — 開く=再計算の仕掛けは持たない)
            "pivot-insert" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status =
                        ui::t!("元の表を範囲で選んでください(1行目が見出し)").into();
                } else {
                    let (a, b) = self.sel_rect();
                    if b.row <= a.row {
                        self.status = ui::t!("見出しの下にデータの行が要ります").into();
                    } else {
                        let headers: Vec<String> = (a.col..=b.col)
                            .map(|c| {
                                let v = self
                                    .sheet()
                                    .get(Pos::new(a.row, c))
                                    .map(|x| x.value.display())
                                    .unwrap_or_default();
                                if v.is_empty() { col_name(c) } else { v }
                            })
                            .collect();
                        self.status = format!(
                            "行に並べる見出し(カンマ区切り可): {}",
                            headers.join(" / ")
                        ).into();
                        self.pivot_pend = Some(PivotPend {
                            a,
                            b,
                            headers,
                            rows_sel: Vec::new(),
                            cols_sel: Vec::new(),
                        });
                        self.prompt = Some(("pivot-rows", Editor::new("")));
                    }
                }
            }
            // シートの保護。パスワードは掛けない(掛けた振りもしない)—
            // Excel でも「保護されたシート」に見え、解除も同じ1手でできる
            "prot-doc" => {
                let name = self.sheet().name.clone();
                if self.sheet().protected {
                    self.sheet_mut().protected = false;
                    self.dirty = true;
                    self.status = format!(
                        "シート「{name}」の保護を外しました(編集できます。保存で xlsx にも残ります)"
                    )
                    .into();
                } else {
                    self.commit();
                    self.sheet_mut().protected = true;
                    self.dirty = true;
                    self.status = format!(
                        "シート「{name}」を保護しました(編集を堰き止めます。同じ釦で解除。パスワードは掛けません — 掛けた振りもしません)"
                    )
                    .into();
                }
            }
            // 暗号化。パスワードを決めると、保存で ECMA-376 Standard
            // (AES-128)の複合ファイルに包む。空 Enter で解除
            "prot-encrypt" => {
                self.pw_pending = None;
                self.prompt = Some(("pw-set", Editor::new("")));
                self.status = if self.encrypt_pw.is_some() {
                    ui::t!("暗号化は入っています。新しいパスワードを打って Enter(空のまま Enter で暗号化をやめる)").into()
                } else {
                    ui::t!("暗号化: パスワードを打って Enter(次の保存から効きます)").into()
                };
            }
            // デジタル署名。**隣の .sig への添え書き**(Ed25519)。
            // Excel の署名欄には出ない独自方式 — そう言って出す。
            // 有効なら報告だけ、無効・未署名なら(作り直して)署名する
            "prot-sign" => {
                use ed25519_dalek::{Signer as _, Verifier as _};
                let Some(p) = self.path.clone() else {
                    self.status =
                        ui::t!("まだファイルになっていません(先に保存してください)").into();
                    return;
                };
                if self.dirty {
                    self.status =
                        ui::t!("未保存の変更があります。保存してから署名してください").into();
                    return;
                }
                let bytes = match std::fs::read(&p) {
                    Ok(b) => b,
                    Err(e) => {
                        self.status = format!("読めません: {e}").into();
                        return;
                    }
                };
                let sp = sig_path_for(&p);
                // 既にある署名を検める
                if let Ok(txt) = std::fs::read_to_string(&sp) {
                    let field = |k: &str| -> Option<String> {
                        txt.lines()
                            .find(|l| l.starts_with(k))
                            .map(|l| l[k.len()..].trim().to_string())
                    };
                    let ok = (|| -> Option<(String, bool)> {
                        let signer = field("signer:")?;
                        let vk: [u8; 32] = unhex(&field("pubkey:")?)?.try_into().ok()?;
                        let sg: [u8; 64] = unhex(&field("sig:")?)?.try_into().ok()?;
                        let vk = ed25519_dalek::VerifyingKey::from_bytes(&vk).ok()?;
                        let sig = ed25519_dalek::Signature::from_bytes(&sg);
                        Some((signer, vk.verify(&bytes, &sig).is_ok()))
                    })();
                    if let Some((signer, true)) = ok {
                        self.status = format!(
                            "署名は有効です — {signer} が署名した時のままの中身です"
                        )
                        .into();
                        return;
                    }
                }
                // 無い・壊れている・中身が変わった → 署名し(直し)て添える
                match load_or_make_key() {
                    Ok(key) => {
                        let sig = key.sign(&bytes);
                        let txt = format!(
                            "office-sign v1\nsigner: {}\npubkey: {}\nsig: {}\n",
                            lock_identity(),
                            to_hex(key.verifying_key().as_bytes()),
                            to_hex(&sig.to_bytes())
                        );
                        match std::fs::write(&sp, txt) {
                            Ok(_) => {
                                self.status = format!(
                                    "署名しました — 隣の {} に添え書き(独自方式。Excel の署名欄には出ません。もう一度押すと検めます)",
                                    sp.file_name().unwrap_or_default().to_string_lossy()
                                )
                                .into();
                            }
                            Err(e) => {
                                self.status = format!("署名が置けません: {e}").into();
                            }
                        }
                    }
                    Err(e) => self.status = format!("署名できません: {e}").into(),
                }
            }
            // 共同編集モード。実体はファイルの錠(.~lock)による早い者勝ちの
            // 編集権。押すと錠の今を確かめ、先客が去っていれば取り直す
            "coauth-mode" => match self.path.clone() {
                None => {
                    self.status =
                        ui::t!("まだファイルになっていません(保存すると編集権=錠を取ります)").into();
                }
                Some(p) => {
                    if self.my_lock.is_some() {
                        self.status = format!(
                            "編集権はこちら({})にあります。同じブックは先に開いた人が書け、後の人は読むだけになります(錠は .~lock ファイル)",
                            lock_identity()
                        )
                        .into();
                    } else {
                        self.acquire_lock(&p);
                        self.status = match &self.locked_by {
                            Some(who) => format!(
                                "{who} が編集中です(読めますが上書き保存はできません。相手が閉じたら、またこの釦で確かめてください)"
                            )
                            .into(),
                            None => ui::t!("先客が居なくなっていたので、編集権を取り直しました").into(),
                        };
                    }
                }
            },
            "co-showcomment" => {
                self.show_comments = !self.show_comments;
                self.status = if self.show_comments {
                    ui::t!("コメントを表示します").into()
                } else {
                    ui::t!("コメントを隠しました(付いてはいます)").into()
                };
            }
            "co-delcomment" => {
                let p = self.cursor;
                if self.sheet().comments.contains_key(&p) {
                    self.checkpoint();
                    self.book.sheets[self.active].comments.remove(&p);
                    self.dirty = true;
                    self.status =
                        format!("{} のコメントを外しました(Ctrl+Z で戻せます)", p.a1())
                            .into();
                } else {
                    self.status = ui::t!("このセルにコメントはありません").into();
                }
            }
            // バージョン履歴。上書き保存のたびに .jo-history へ残る控えの一覧
            "co-history" => {
                if self.path.is_none() {
                    self.status =
                        ui::t!("まだファイルになっていません(保存すると、上書きのたびに控えが残ります)").into();
                } else {
                    let v = self.versions();
                    if v.is_empty() {
                        self.status =
                            ui::t!("控えはまだありません(上書き保存のたびに .jo-history へ残ります)").into();
                    } else {
                        let names: Vec<String> = v.iter().map(|(n, _)| n.clone()).collect();
                        self.pick_paths = v;
                        self.pick_kind = "history";
                        self.pick = Some((names, (HEAD_W + 60.0, ROW_H + 20.0)));
                        self.status =
                            ui::t!("バージョン履歴: 選ぶと控えを名無しの複製で開きます(いまの書きかけは要るなら先に保存)").into();
                    }
                }
            }
            // チャット。ブックの隣の申し送り帳(.chat.txt)へ名乗り付きで追記。
            // サーバーは無いので生放送ではない — ファイル越しの言伝
            "co-chat" => match self.chat_path() {
                None => {
                    self.status =
                        ui::t!("まだファイルになっていません(保存すると、隣に申し送り帳ができます)").into();
                }
                Some(cp) => {
                    let tail = std::fs::read_to_string(&cp)
                        .map(|t| {
                            t.lines()
                                .rev()
                                .take(3)
                                .map(|l| l.to_string())
                                .collect::<Vec<_>>()
                                .into_iter()
                                .rev()
                                .collect::<Vec<_>>()
                                .join(" / ")
                        })
                        .unwrap_or_default();
                    self.status = if tail.is_empty() {
                        ui::t!("まだ言伝はありません(打って Enter で書き残します)").into()
                    } else {
                        format!("言伝: {tail}").into()
                    };
                    self.prompt = Some(("chat", Editor::new("")));
                }
            },
            // マクロ = Python in Calc と同じ実体(檻の中で .py を回す)
            "plug-macros" => {
                self.commit();
                self.run_python_file_dialog(cx);
                self.status =
                    ui::t!("マクロ: .py を選ぶと檻の中の Python が回ります(b=ブック s=シート。実体は データ > Python と同じ)").into();
            }
            // プラグインの管理。置き場の .py を一覧し、同じ檻で実行
            "plug-manage" => {
                let dir = plugins_dir();
                let mut items: Vec<PathBuf> = std::fs::read_dir(&dir)
                    .ok()
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e == "py"))
                    .collect();
                items.sort();
                if items.is_empty() {
                    self.status = format!(
                        "プラグイン: {} に .py を置くと、ここに並びます",
                        dir.display()
                    )
                    .into();
                } else {
                    let v: Vec<(String, PathBuf)> = items
                        .into_iter()
                        .map(|q| {
                            (
                                q.file_name().unwrap_or_default().to_string_lossy().to_string(),
                                q,
                            )
                        })
                        .collect();
                    let names: Vec<String> = v.iter().map(|(n, _)| n.clone()).collect();
                    self.pick_paths = v;
                    self.pick_kind = "plugin";
                    self.pick = Some((names, (HEAD_W + 60.0, ROW_H + 20.0)));
                    self.status =
                        ui::t!("プラグイン: 選ぶと檻の中の Python で実行します(b=ブック s=シート)").into();
                }
            }
            // チェックボックス(セルの部品)。空のセルに FALSE を置くと
            // ☑/☐ で見え、空白キーで切り替わる(Excel では TRUE/FALSE の値)
            "inscheckbox" => {
                self.commit();
                let (a, b) = self.sel_rect();
                let mut empties = Vec::new();
                let mut bools = 0usize;
                let mut skipped = 0usize;
                for r in a.row..=b.row {
                    for c in a.col..=b.col {
                        let p = Pos::new(r, c);
                        match self.sheet().get(p).map(|x| &x.value) {
                            None | Some(Value::Empty) => empties.push(p),
                            Some(Value::Bool(_)) => bools += 1,
                            _ => skipped += 1,
                        }
                    }
                }
                if empties.is_empty() && bools == 0 {
                    self.status =
                        ui::t!("空のセルを選んでください(中身のあるセルは潰しません)").into();
                } else {
                    if !empties.is_empty() {
                        self.checkpoint();
                        for p in &empties {
                            let mut cell =
                                self.sheet().get(*p).cloned().unwrap_or_default();
                            cell.formula = None;
                            cell.value = Value::Bool(false);
                            self.book.sheets[self.active].set(*p, cell);
                        }
                        recalc_book(&mut self.book, self.active);
                        self.dirty = true;
                        self.sync_input();
                    }
                    let skip_note = if skipped > 0 {
                        format!("。中身のある {skipped} セルは触っていません")
                    } else {
                        String::new()
                    };
                    self.status = format!(
                        "チェックボックスを {} 個置きました(空白キーで切替。Excel では TRUE/FALSE で見えます{skip_note})",
                        empties.len()
                    )
                    .into();
                }
            }
            // スライサー。カーソルの列の一意な値を釦で並べ、押して絞る。
            // 絞り込みと同じく**見え方だけ**(保存される中身は変わらない)
            "insslicer" => {
                if self.slicer.take().is_none() {
                    self.commit();
                    let col = self.cursor.col;
                    let (rows, _) = self.sheet().extent();
                    if rows < 2 {
                        self.status =
                            ui::t!("スライサーにする列を選んでください(見出しの下にデータの行が要ります)").into();
                    } else {
                        self.slicer =
                            Some((col, std::collections::BTreeSet::new(), false));
                        self.status = format!(
                            "スライサー: {} 列の値を押して絞る(≡=複数選択 / ✕=解除。見え方だけで、中身は変わりません)",
                            col_name(col)
                        )
                        .into();
                    }
                }
            }
            // テキストアート。文字を板に打つと飾り文字を描いて画像で置く
            "instextart" => {
                self.commit();
                self.prompt = Some(("textart", Editor::new("")));
                self.status =
                    ui::t!("テキストアート: 文字を打つと、太字+縁取りの飾り文字を画像で置きます").into();
            }
            // 方程式(数式エディタ)。式を板に打つと mathtext が清書して画像で置く
            "insequation" => {
                self.commit();
                self.prompt = Some(("equation", Editor::new("")));
                self.status =
                    ui::t!("方程式: TeX の書き方で(例: \\frac{a}{b} や \\sum_{i=1}^n i^2)。Enter で清書").into();
            }
            // SmartArt。分類 → 形の2段の一覧(分類・並び・名前は本家)
            "inssmartart" => {
                self.commit();
                let names: Vec<String> =
                    SMARTART.iter().map(|(n, _)| n.to_string()).collect();
                self.pick_kind = "sa-cat";
                self.pick = Some((names, (HEAD_W + 60.0, ROW_H + 20.0)));
                self.status =
                    ui::t!("SmartArt: 分類 → 形の順に選ぶ(図形の集まりとして入ります)").into();
            }
            // ソルバー。ONLYOFFICE と同じ小窓を開く(解法も同じ単体法 LP)
            "solver" => {
                if self.solver.take().is_none() {
                    self.commit();
                    let init = if self.anchor.is_some() {
                        self.sel_rect().0.a1()
                    } else {
                        self.cursor.a1()
                    };
                    self.solver = Some(Solver::new(&init));
                    self.status =
                        ui::t!("ソルバー: 欄を押して打つ。目的・変数セル・制約を決めて「解を求める」").into();
                }
            }
            // 下付き(vertAlign subscript)。上付きは本家 calc にも無い
            "subscript" => {
                self.fmt(|f| f.subscript = !f.subscript);
                self.status = ui::t!("下付きを切り替えました").into();
            }
            // 両端揃え(セルの横揃え。折り返した行を左右に伸ばす)
            "align-just" => {
                self.fmt(|f| {
                    f.align = if f.align == sheet::model::HAlign::Justify {
                        sheet::model::HAlign::General
                    } else {
                        sheet::model::HAlign::Justify
                    };
                    f.wrap = true; // 揃えるには折り返しが要る
                });
                self.status = ui::t!("両端揃えにしました(折り返して全体を表示も入れます)").into();
            }
            // 文字の回転(縦書きのセル。90度ずつ回る)
            "text-orient" => {
                self.fmt(|f| {
                    f.rotation = match f.rotation {
                        None | Some(0) => Some(90),
                        Some(90) => Some(180),
                        Some(180) => Some(255), // 255 = 縦に積む(xlsx の作法)
                        _ => None,
                    };
                });
                let r = self.sheet().get(self.cursor).map(|c| c.fmt.rotation).unwrap_or(None);
                self.status = match r {
                    Some(90) => ui::t!("文字を 90 度回しました").into(),
                    Some(180) => ui::t!("文字を 180 度回しました").into(),
                    Some(255) => ui::t!("文字を縦に積みました").into(),
                    _ => ui::t!("文字の向きを戻しました").into(),
                };
            }
            // 計算方法(自動 ⇔ 手動)。手動のときは F9 で計算する
            "calc-mode" => {
                self.auto_calc = !self.auto_calc;
                self.status = if self.auto_calc {
                    ui::t!("計算方法: 自動(いつもすぐ計算します)").into()
                } else {
                    ui::t!("計算方法: 手動(F9 で計算します — 大きな表で待たされない)").into()
                };
            }
            // 関数の挿入 = 本家と同じ小窓(検索・分類・一覧・説明)。
            // 数式バーの fx と同じ実体
            "insert-function" => {
                self.fn_dlg = Some(FnDlg {
                    search: Editor::new(""),
                    group: 0,
                    sel: 0,
                });
                self.status =
                    ui::t!("関数を挿入: 打って絞り込み、↑↓で選んで Enter(Esc で取消)").into();
            }
            // セルのスタイル(既定の書式の組。押すと一覧から選ぶ)
            "cell-styles" => {
                self.pick_kind = "cell-style";
                self.pick = Some((
                    CELL_STYLES.iter().map(|(n, _)| n.to_string()).collect(),
                    (HEAD_W + 60.0, ROW_H + 20.0),
                ));
                self.status = ui::t!("セルのスタイル: 選ぶと選択に掛かります(Ctrl+Z で戻せます)").into();
            }
            // シートの表示(隠したシートを戻す/いまのシートを隠す)
            "sheet-view" => {
                let hidden: Vec<(usize, String)> = self
                    .book
                    .sheets
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.hidden)
                    .map(|(i, s)| (i, s.name.clone()))
                    .collect();
                if hidden.is_empty() {
                    // 隠すほう(最後の1枚は隠さない — 見えるシートがゼロになる)
                    if self.book.sheets.iter().filter(|s| !s.hidden).count() <= 1 {
                        self.status = ui::t!("最後の1枚は隠せません").into();
                    } else {
                        let n = self.sheet().name.clone();
                        self.checkpoint_book();
                        self.sheet_mut().hidden = true;
                        // 見えるシートへ移る
                        if let Some(i) = self.book.sheets.iter().position(|s| !s.hidden) {
                            self.switch_sheet(i);
                        }
                        self.dirty = true;
                        self.status = format!(
                            "シート「{n}」を隠しました(同じ釦で戻せます。保存で xlsx にも残ります)"
                        )
                        .into();
                    }
                } else {
                    self.pick_kind = "unhide";
                    self.pick_paths = hidden
                        .iter()
                        .map(|(i, n)| (n.clone(), PathBuf::from(i.to_string())))
                        .collect();
                    self.pick = Some((
                        hidden.into_iter().map(|(_, n)| n).collect(),
                        (HEAD_W + 60.0, ROW_H + 20.0),
                    ));
                    self.status = ui::t!("隠したシート: 選ぶと表示に戻します").into();
                }
            }
            // ウォッチウィンドウ(見張りの窓)。選んだセルを控えて下に見せる
            "watch" => {
                let (a, b) = self.sel_rect();
                let mut n = 0usize;
                for r in a.row..=b.row {
                    for c in a.col..=b.col {
                        let p = Pos::new(r, c);
                        if self.sheet().get(p).and_then(|x| x.formula.as_ref()).is_some()
                            || self.anchor.is_none()
                        {
                            if !self.watch.contains(&(self.active, p)) {
                                self.watch.push((self.active, p));
                                n += 1;
                            }
                        }
                    }
                }
                if n == 0 && !self.watch.is_empty() {
                    self.watch.clear();
                    self.status = ui::t!("見張りを空にしました").into();
                } else {
                    self.status = format!(
                        "{n} 個を見張ります(値は下の帯に出ます。もう一度押すと空に)"
                    )
                    .into();
                }
            }
            // 描画(ペン・蛍光ペン・消しゴム)。writer と同じ形の道具の入切
            "pen" | "highlighter" | "eraser" => {
                let t = match id {
                    "pen" => 0u8,
                    "highlighter" => 1,
                    _ => 2,
                };
                self.tool = if self.tool == Some(t) { None } else { Some(t) };
                self.ink_cur = None;
                self.status = match self.tool {
                    Some(0) => ui::t!("ペン: 表の上をドラッグで描く(もう一度押すか Esc で戻る)").into(),
                    Some(1) => ui::t!("蛍光ペン: ドラッグで引く(セルの上に薄く乗る)").into(),
                    Some(2) => ui::t!("消しゴム: 線をなぞると1筆ずつ消える").into(),
                    _ => ui::t!("セルの操作に戻りました").into(),
                };
            }
            // AI タブ。**モデルに任せる変換と生成の道具箱**(writer と同じ宛先)
            "ai-where" => {
                let next = ui::ai::backend().next();
                ui::ai::set_backend(next);
                self.status = match ui::ai::ready(next) {
                    Ok(_) => format!("AI の宛先: {}(覚えました)", next.label()).into(),
                    Err(e) => format!(
                        "AI の宛先: {} — ただし今は使えません: {e}",
                        next.label()
                    )
                    .into(),
                };
            }
            "ai-summary" => self.ai_go(CalcAi::Summary, cx),
            "ai-rewrite" => self.ai_go(
                CalcAi::Rewrite(
                    "あなたは表の中の文字を整える道具です。渡されたタブ区切りの表と\
                     同じ行数・同じ列数のタブ区切りだけを返します。文字は意味を\
                     変えずに読みやすく直し、数字と空欄はそのまま写します。",
                    "次の表の文字を、意味を変えずに読みやすく直してください。",
                ),
                cx,
            ),
            "ai-polite" => self.ai_go(
                CalcAi::Rewrite(
                    "あなたは表の中の文字を整える道具です。渡されたタブ区切りの表と\
                     同じ行数・同じ列数のタブ区切りだけを返します。文字は内容を\
                     変えずに丁寧な言い方(です・ます)へ直し、数字と空欄はそのまま\
                     写します。",
                    "次の表の文字を、内容を変えずに丁寧な言い方へ直してください。",
                ),
                cx,
            ),
            "ai-plain" => self.ai_go(
                CalcAi::Rewrite(
                    "あなたは表の中の文字をやさしくする道具です。渡されたタブ区切りの\
                     表と同じ行数・同じ列数のタブ区切りだけを返します。難しい言葉を\
                     やさしい言葉に置き換え、数字と空欄はそのまま写します。",
                    "次の表の文字を、内容を変えずにやさしい日本語へ直してください。",
                ),
                cx,
            ),
            "ai-translate" => self.ai_go(CalcAi::Translate, cx),
            "ai-furigana" => self.ai_go(CalcAi::Furigana, cx),
            "ai-continue" => self.ai_go(CalcAi::Continue, cx),
            "ai-table" => {
                self.commit();
                self.prompt = Some(("ai-table", Editor::new("")));
                self.status = format!(
                    "AI({})が表にします: 文章を打って(貼って)Enter",
                    ui::ai::backend().label()
                )
                .into();
            }
            "ai-ask" => {
                self.commit();
                self.prompt = Some(("ai-ask", Editor::new("")));
                self.status = format!(
                    "AI({})に頼む: 用件を打って Enter(選んだ範囲があれば一緒に渡します)",
                    ui::ai::backend().label()
                )
                .into();
            }
            // 配色の変更(テーマ色の組を入れ替える)。テーマ由来の色を
            // 使っているセルは、色がそのまま追従する
            "colorschemas" => {
                self.pick_kind = "scheme";
                self.pick = Some((
                    sheet::theme::SCHEMES.iter().map(|(n, _)| n.to_string()).collect(),
                    (HEAD_W + 60.0, ROW_H + 20.0),
                ));
                self.status = ui::t!("配色の変更: 選ぶとテーマ色が入れ替わります").into();
            }
            // インターフェイステーマ(画面の明暗)。**セルは白のまま**
            "theme" => {
                self.dark = !self.dark;
                self.status = if self.dark {
                    ui::t!("画面を暗くしました(セルは白のまま — 画面と紙の一致を守る)").into()
                } else {
                    ui::t!("画面を明るくしました").into()
                };
            }
            // 範囲に変換する(表オブジェクトを外す。**書式と式は残る**)
            "td-torange" => {
                self.commit();
                let p = self.cursor;
                match self.sheet().tables.iter().position(|t| t.contains(p)) {
                    None => {
                        self.status =
                            ui::t!("表の中にカーソルを置いてください(表のない範囲は「表の挿入」で表にできます)").into();
                    }
                    Some(i) => {
                        self.checkpoint();
                        let t = self.book.sheets[self.active].tables.remove(i);
                        self.dirty = true;
                        self.status = format!(
                            "表「{}」を普通の範囲に戻しました(帯や縞々の書式と式はそのまま残ります)",
                            t.name
                        )
                        .into();
                    }
                }
            }
            // テーブルのサイズ変更(範囲を変える)。板で新しい範囲を聞く
            "td-resize" => {
                self.commit();
                let p = self.cursor;
                match self.sheet().tables.iter().position(|t| t.contains(p)) {
                    None => self.status = ui::t!("表の中にカーソルを置いてください").into(),
                    Some(i) => {
                        let t = &self.sheet().tables[i];
                        let init = format!("{}:{}", t.a.a1(), t.b.a1());
                        self.status = format!("表「{}」の新しい範囲は?", t.name).into();
                        self.prompt = Some(("table-resize", Editor::new(&init)));
                    }
                }
            }
            // シートの方向(右から左へ)。**日本語も右から書くことがある**
            "rtl-sheet" => {
                let on = !self.sheet().rtl;
                self.sheet_mut().rtl = on;
                self.dirty = true;
                self.status = if on {
                    ui::t!("右から左へ並べます(右横書き。列は右から A B C…)").into()
                } else {
                    ui::t!("左から右へ戻しました").into()
                };
            }
            // 文字の向き(セルの中を右横書きに)。1字ずつ右から並べる
            "direction" => {
                self.fmt(|f| f.rtl_text = !f.rtl_text);
                self.status =
                    ui::t!("セルの中を右横書きにしました(1字ずつ右から。昔の看板の書き方)").into();
            }
            // 表示タブ(本家のデスクトップ版に合わせる)。どれも見え方だけ
            "zoom-in" => {
                self.zoom = (self.zoom + 0.1).min(2.0);
                self.status = format!("ズーム {}%", (self.zoom * 100.0).round() as i32).into();
            }
            "zoom-out" => {
                self.zoom = (self.zoom - 0.1).max(0.5);
                self.status = format!("ズーム {}%", (self.zoom * 100.0).round() as i32).into();
            }
            "formula-bar" => {
                self.show_formula_bar = !self.show_formula_bar;
                self.status = if self.show_formula_bar {
                    ui::t!("数式バーを表示します").into()
                } else {
                    ui::t!("数式バーを隠しました(表示タブで戻せます)").into()
                };
            }
            "show-headings" => {
                self.show_headers = !self.show_headers;
                self.status = if self.show_headers {
                    ui::t!("見出しを表示します").into()
                } else {
                    ui::t!("見出しを隠しました(列幅のドラッグ等は見出しと一緒に戻ります)").into()
                };
            }
            "show-zeros" => {
                self.show_zeros = !self.show_zeros;
                self.status = if self.show_zeros {
                    ui::t!("0 を表示します").into()
                } else {
                    ui::t!("0 を隠しました(見え方だけ — 値は 0 のまま)").into()
                };
            }
            // 小計(Excel の集計)。本家のデータタブに無い釦だが、グループ化を
            // 「畳むと合計が残る」形で使うために要る(発注者指摘 2026-08-04)
            "subtotal" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status = ui::t!("表を範囲で選んでください(1行目が見出し)").into();
                } else {
                    let (a, b) = self.sel_rect();
                    if b.row <= a.row {
                        self.status = ui::t!("見出しの下にデータの行が要ります").into();
                    } else {
                        let headers: Vec<String> = (a.col..=b.col)
                            .map(|c| {
                                let v = self
                                    .sheet()
                                    .get(Pos::new(a.row, c))
                                    .map(|x| x.value.display())
                                    .unwrap_or_default();
                                if v.is_empty() { col_name(c) } else { v }
                            })
                            .collect();
                        self.status = format!(
                            "何の区切りで集めるか(見出しを1つ): {}",
                            headers.join(" / ")
                        )
                        .into();
                        self.sub_pend = Some(PivotPend {
                            a,
                            b,
                            headers,
                            rows_sel: Vec::new(),
                            cols_sel: Vec::new(),
                        });
                        self.prompt = Some(("subtotal-by", Editor::new("")));
                    }
                }
            }
            // グループ化(アウトライン)。行か列かは選択の形で決める:
            // 見出しから列をまるごと選んでいれば列、それ以外は選択の行。
            // 深さは xlsx の outlineLevel と往復し、畳みも保存に残る
            "group" | "ungroup" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status =
                        ui::t!("まとめたい行(または列)を選んでください(見出しの番号を撫でる)").into();
                } else {
                    let (a, b) = self.sel_rect();
                    let (rows_ext, cols_ext) = self.sheet().extent();
                    let whole_rows = a.row == 0 && b.row + 1 >= rows_ext.max(1);
                    let on_cols = whole_rows && !(a.col == 0 && b.col + 1 >= cols_ext.max(1));
                    self.checkpoint();
                    let add = id == "group";
                    let sh = self.sheet_mut();
                    if on_cols {
                        for c in a.col..=b.col {
                            let l = sh.col_outline.get(&c).copied().unwrap_or(0);
                            let nl = if add { (l + 1).min(7) } else { l.saturating_sub(1) };
                            if nl == 0 {
                                sh.col_outline.remove(&c);
                                sh.col_hidden.remove(&c);
                            } else {
                                sh.col_outline.insert(c, nl);
                            }
                        }
                    } else {
                        for r in a.row..=b.row {
                            let l = sh.row_outline.get(&r).copied().unwrap_or(0);
                            let nl = if add { (l + 1).min(7) } else { l.saturating_sub(1) };
                            if nl == 0 {
                                sh.row_outline.remove(&r);
                                sh.row_hidden.remove(&r);
                            } else {
                                sh.row_outline.insert(r, nl);
                            }
                        }
                    }
                    self.dirty = true;
                    let what = if on_cols {
                        format!("{}〜{}列", col_name(a.col), col_name(b.col))
                    } else {
                        format!("{}〜{}行", a.row + 1, b.row + 1)
                    };
                    self.status = if add {
                        format!(
                            "{what}をグループ化しました(深さ+1。「詳細の非表示」で畳めます。Ctrl+Z で戻せます)"
                        )
                        .into()
                    } else {
                        format!("{what}のグループ化を1段解きました(Ctrl+Z で戻せます)").into()
                    };
                }
            }
            // 詳細の非表示=グループ化した行(列)を畳む / 詳細の表示=開く。
            // 対象は選択、無ければカーソルの行が属するグループのひとつながり
            "hide-details" | "show-details" => {
                self.commit();
                let hide = id == "hide-details";
                let (a, b) = self.sel_rect();
                let (rows_ext, cols_ext) = self.sheet().extent();
                let whole_rows =
                    self.anchor.is_some() && a.row == 0 && b.row + 1 >= rows_ext.max(1);
                let on_cols = whole_rows && !(a.col == 0 && b.col + 1 >= cols_ext.max(1));
                if on_cols {
                    let sh = self.sheet();
                    let targets: Vec<u32> = (a.col..=b.col)
                        .filter(|c| sh.col_outline.contains_key(c))
                        .collect();
                    if targets.is_empty() {
                        self.status =
                            ui::t!("選択にグループ化した列がありません(先にグループ化)").into();
                    } else {
                        self.checkpoint();
                        let sh = self.sheet_mut();
                        for c in &targets {
                            if hide {
                                sh.col_hidden.insert(*c);
                            } else {
                                sh.col_hidden.remove(c);
                            }
                        }
                        self.dirty = true;
                        self.status = format!(
                            "{} 列を{}(Ctrl+Z で戻せます)",
                            targets.len(),
                            if hide { "畳みました" } else { "開きました" }
                        )
                        .into();
                    }
                } else {
                    // 行: 選択、または カーソルの行が属するグループのひとつながり
                    let (r0, r1) = if self.anchor.is_some() {
                        (a.row, b.row)
                    } else {
                        let sh = self.sheet();
                        let at = self.cursor.row;
                        if !sh.row_outline.contains_key(&at) {
                            self.status = ui::t!("グループ化した行の上で押してください(先に データ > グループ化)").into();
                            cx.notify();
                            return;
                        }
                        let mut lo = at;
                        while lo > 0 && sh.row_outline.contains_key(&(lo - 1)) {
                            lo -= 1;
                        }
                        let mut hi = at;
                        while sh.row_outline.contains_key(&(hi + 1)) {
                            hi += 1;
                        }
                        (lo, hi)
                    };
                    let sh = self.sheet();
                    let targets: Vec<u32> =
                        (r0..=r1).filter(|r| sh.row_outline.contains_key(r)).collect();
                    if targets.is_empty() {
                        self.status =
                            ui::t!("選択にグループ化した行がありません(先に データ > グループ化)").into();
                    } else {
                        self.checkpoint();
                        let sh = self.sheet_mut();
                        for r in &targets {
                            if hide {
                                sh.row_hidden.insert(*r);
                            } else {
                                sh.row_hidden.remove(r);
                            }
                        }
                        self.dirty = true;
                        self.status = format!(
                            "{} 行を{}(Ctrl+Z で戻せます)",
                            targets.len(),
                            if hide { "畳みました" } else { "開きました" }
                        )
                        .into();
                    }
                }
            }
            // ピボットの手入れ: どれも「指図を直して置き直す」だけ。
            // 対象はカーソルの下のピボット(指図はブックに控えてある)
            "pivot-refresh" => {
                self.commit();
                match self.pivot_at(self.cursor) {
                    Some(i) => {
                        let d = self.book.pivots[i].clone();
                        self.spawn_pivot(d, Some(i), cx);
                    }
                    None => {
                        self.status =
                            ui::t!("更新したいピボットの上にカーソルを置いてください").into();
                    }
                }
            }
            "pivot-refresh-all" => {
                self.commit();
                let n = self.book.pivots.len();
                if n == 0 {
                    self.status = ui::t!("このブックにピボットはありません").into();
                } else {
                    for i in 0..n {
                        let d = self.book.pivots[i].clone();
                        self.spawn_pivot(d, Some(i), cx);
                    }
                    self.status = format!("{n} 件のピボットを更新しています…").into();
                }
            }
            "pivot-select" => {
                match self.pivot_at(self.cursor) {
                    Some(i) => {
                        let d = &self.book.pivots[i];
                        self.cursor = d.dest;
                        self.anchor = Some(Pos::new(
                            d.dest.row + d.size.0.saturating_sub(1),
                            d.dest.col + d.size.1.saturating_sub(1),
                        ));
                        self.sync_input();
                        self.status = ui::t!("ピボット全体を選びました").into();
                    }
                    None => {
                        self.status = ui::t!("ピボットの上にカーソルを置いてください").into();
                    }
                }
            }
            "pivot-totals" | "pivot-subtotals" | "pivot-blank" | "pivot-layout" => {
                self.commit();
                match self.pivot_at(self.cursor) {
                    None => {
                        self.status = ui::t!("ピボットの上にカーソルを置いてください").into();
                    }
                    Some(i) => {
                        let need_two = matches!(id, "pivot-subtotals" | "pivot-blank");
                        if need_two && self.book.pivots[i].rows_sel.len() < 2 {
                            self.status =
                                ui::t!("行の見出しが2つ以上のピボットで効きます(挿入で複数選ぶ)").into();
                        } else {
                            let d = &mut self.book.pivots[i];
                            let (name, on) = match id {
                                "pivot-totals" => {
                                    d.totals = !d.totals;
                                    ("総計", d.totals)
                                }
                                "pivot-subtotals" => {
                                    d.subtotals = !d.subtotals;
                                    ("小計", d.subtotals)
                                }
                                "pivot-blank" => {
                                    d.blank_rows = !d.blank_rows;
                                    ("空行", d.blank_rows)
                                }
                                _ => {
                                    d.compact = !d.compact;
                                    ("コンパクト形式", d.compact)
                                }
                            };
                            let d = self.book.pivots[i].clone();
                            self.dirty = true;
                            self.status = format!(
                                "{name}を{}にして置き直します…",
                                if on { "あり" } else { "なし" }
                            )
                            .into();
                            self.spawn_pivot(d, Some(i), cx);
                        }
                    }
                }
            }
            // 表のデザイン: 表オブジェクトは持たない。選択に**1手ずつ掛ける道具**
            // (掛けた書式・式が帳面に残るだけ。切り替え式に見せない。
            // まとめて掛けるなら挿入タブの「表の挿入」)
            "td-header" | "td-band-row" | "td-band-col" | "td-first" | "td-last" => {
                // 表の中なら、表オブジェクトの性質も一緒に更新する
                let pcur = self.cursor;
                if let Some(i) = self.sheet().tables.iter().position(|t| t.contains(pcur)) {
                    let t = &mut self.book.sheets[self.active].tables[i];
                    match id {
                        "td-header" => t.header = !t.header,
                        "td-band-row" => t.banded_rows = !t.banded_rows,
                        "td-band-col" => t.banded_cols = !t.banded_cols,
                        "td-first" => t.first_col = !t.first_col,
                        _ => t.last_col = !t.last_col,
                    }
                    self.dirty = true;
                }
                self.commit();
                if self.anchor.is_none() {
                    self.status = ui::t!("表の範囲を選んでください").into();
                } else {
                    self.checkpoint();
                    let (a, b) = self.sel_rect();
                    for r in a.row..=b.row {
                        for c in a.col..=b.col {
                            let p = Pos::new(r, c);
                            let mut cell = self.sheet().get(p).cloned().unwrap_or_default();
                            let touched = match id {
                                "td-header" if r == a.row => {
                                    cell.fmt.bold = true;
                                    cell.fmt.fill = Some("D5E8DC".into());
                                    cell.fmt.borders.top = true;
                                    true
                                }
                                "td-band-row" if r > a.row && (r - a.row) % 2 == 0 => {
                                    cell.fmt.fill = Some("F1F6F3".into());
                                    true
                                }
                                "td-band-col" if (c - a.col) % 2 == 1 => {
                                    cell.fmt.fill = Some("F1F6F3".into());
                                    true
                                }
                                "td-first" if c == a.col => {
                                    cell.fmt.bold = true;
                                    true
                                }
                                "td-last" if c == b.col => {
                                    cell.fmt.bold = true;
                                    true
                                }
                                _ => false,
                            };
                            if touched {
                                self.book.sheets[self.active].set(p, cell);
                            }
                        }
                    }
                    self.dirty = true;
                    let what = match id {
                        "td-header" => "1行目を見出しの帯に",
                        "td-band-row" => "1行おきの縞々に",
                        "td-band-col" => "1列おきの縞々に",
                        "td-first" => "最初の列を太字に",
                        _ => "最後の列を太字に",
                    };
                    self.status = format!(
                        "{}:{} を{}しました(Ctrl+Z で戻せます)",
                        a.a1(),
                        b.a1(),
                        what
                    )
                    .into();
                }
            }
            // 合計行 = 選択の下に =SUM(…) の行を足す(式なので元が変われば追従)
            "td-total" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status = ui::t!("合計したい表の範囲を選んでください").into();
                } else {
                    let (a, b) = self.sel_rect();
                    let below_used = (a.col..=b.col).any(|c| {
                        self.sheet()
                            .get(Pos::new(b.row + 1, c))
                            .map(|cell| {
                                !cell.value.display().is_empty() || cell.formula.is_some()
                            })
                            .unwrap_or(false)
                    });
                    if below_used {
                        self.status =
                            ui::t!("すぐ下の行に中身があります(空けてから — 黙って上書きしません)").into();
                    } else {
                        self.checkpoint();
                        add_total_row(&mut self.book.sheets[self.active], a, b);
                        recalc_book(&mut self.book, self.active);
                        self.dirty = true;
                        self.status = format!(
                            "{} 行目に合計(=SUM)を足しました。式なので元が変われば追従します(Ctrl+Z で戻せます)",
                            b.row + 2
                        )
                        .into();
                    }
                }
            }
            // フィルタのボタン = データタブの絞り込みと同じ実体
            "td-filter" => self.run_cmd("setfilter", cx),
            // 表の挿入 = 選択に表の書式(見出しの帯+縞々+外枠)を掛ける
            "instable" | "table-tpl" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status = ui::t!("表にする範囲を選んでください").into();
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
                    let n = self.book.sheets.iter().map(|s| s.tables.len()).sum::<usize>() + 1;
                    self.book.sheets[self.active].tables.push(sheet::model::TableDef {
                        name: format!("テーブル{n}"),
                        a,
                        b,
                        ..Default::default()
                    });
                    self.dirty = true;
                    self.status = format!(
                        "{}:{} を表にしました(見出しの帯と縞々。範囲に変換・サイズ変更もできます。Ctrl+Z で戻せます)",
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
                        ui::t!("割りたいセルを選んでください(選択した列の文字を右へ割ります)").into();
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
                                sheet::recalc_all(&mut other);
                                let mut n = 0usize;
                                for mut sh in other.sheets.drain(..) {
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
                    self.status = ui::t!("1行目の前では改ページできません").into();
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
                    self.status = ui::t!("タイトル行を解除しました").into();
                } else {
                    self.status =
                        ui::t!("繰り返す行を選んでから押してください(行の見出しをクリック)").into();
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
                        ui::t!("グラフにする範囲を選んでください(1列目が項目名、2列目からが数)").into();
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
                        ui::t!("折れ線にする数の範囲を選んでください(置き場所はいまのセル)").into();
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
                        self.status = ui::t!("数が2つ以上要ります").into();
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
                        self.status = ui::t!("固定する位置にカーソルを置いてください(その上と左が留まります)").into();
                        None
                    }
                    None => {
                        self.status = ui::tf!("{}行 {}列を固定しました", self.cursor.row, self.cursor.col).into();
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
                recalc_book(&mut self.book, self.active);
                self.status = ui::tf!("{} 列で並べ替えました", Pos::new(0, c).a1()
                    .trim_end_matches('1')).into();
            }
            "rem-duplicates" => {
                self.commit();
                self.checkpoint();
                let n = self.book.sheets[self.active].remove_duplicate_rows(true);
                self.dirty = true;
                recalc_book(&mut self.book, self.active);
                // 何件消したかを黙らない
                self.status = ui::tf!("重複した {} 行を削除しました", n).into();
            }
            "currency" => self.fmt(|f| f.number_format = Some("¥#,##0".into())),
            "percents" => self.fmt(|f| f.number_format = Some("0%".into())),
            // 関数の一覧。**使える名前だけを出す** — 無いものを並べない
            f @ ("fn-math" | "fn-text" | "fn-logical" | "fn-recent" | "fn-datetime"
            | "fn-lookup" | "fn-financial" | "fn-more") => {
                let names: &str = match f {
                    "fn-math" => "SUM AVERAGE ROUND ROUNDUP ROUNDDOWN INT ABS MOD POWER SQRT \
                                  PRODUCT SUMPRODUCT SUMSQ CEILING FLOOR MROUND EVEN ODD SIGN \
                                  FACT COMBIN PERMUT GCD LCM PI SIN COS TAN ASIN ACOS ATAN ATAN2 \
                                  SINH COSH TANH EXP LN LOG LOG10 DEGREES RADIANS RAND RANDBETWEEN \
                                  SEQUENCE(隣へあふれる。=SEQUENCE(3)+1 のような式も可)",
                    "fn-text" => "LEN LEFT RIGHT MID TRIM UPPER LOWER CONCATENATE CONCAT TEXT \
                                  SUBSTITUTE FIND SEARCH VALUE TEXTJOIN REPT CHAR CODE \
                                  UNICHAR UNICODE PROPER EXACT CLEAN FIXED YEN NUMBERVALUE \
                                  LENB LEFTB RIGHTB MIDB ASC JIS DATESTRING(和暦) \
                                  PHONETIC(ふりがな — 読んだ xlsx の rPh を引く)",
                    "fn-logical" => "IF IFS SWITCH AND OR NOT TRUE FALSE ISBLANK ISERROR IFERROR \
                                     IFNA ISNA ISERR ISLOGICAL ISNONTEXT ISNUMBER ISTEXT NA",
                    "fn-datetime" => "TODAY NOW DATE DATEVALUE YEAR MONTH DAY WEEKDAY \
                                      TIME HOUR MINUTE SECOND EDATE EOMONTH DATEDIF \
                                      WORKDAY NETWORKDAYS DAYS DAYS360 YEARFRAC \
                                      WEEKNUM ISOWEEKNUM(値は通し番号)",
                    "fn-lookup" => "VLOOKUP HLOOKUP XLOOKUP LOOKUP INDEX MATCH CHOOSE \
                                    ROW COLUMN ROWS COLUMNS OFFSET INDIRECT ADDRESS HYPERLINK \
                                    FILTER SORT UNIQUE TRANSPOSE(照合は完全一致。\
                                    FILTER 等は隣へあふれ、四則と組み合わせても効く)",
                    "fn-financial" => "PMT PV FV NPER NPV IRR RATE(IRR と RATE は挟み撃ちの反復解)",
                    "fn-more" => "SUMIF SUMIFS COUNTIF COUNTIFS AVERAGEIF AVERAGEIFS \
                                  MINIFS MAXIFS COUNTA COUNTBLANK TRUNC \
                                  RANK RANK.EQ RANK.AVG LARGE SMALL \
                                  MEDIAN MODE STDEV STDEVP VAR VARP PERCENTILE QUARTILE \
                                  CORREL SLOPE INTERCEPT FORECAST AVERAGEA MAXA MINA \
                                  SUBTOTAL QUOTIENT CEILING.MATH FLOOR.MATH \
                                  ISEVEN ISODD T N TYPE — 一覧は各族の釦で",
                    _ => "SUM AVERAGE COUNT MAX MIN IF SUMIF COUNTIF VLOOKUP TODAY",
                };
                self.status = ui::tf!("使える関数: {}", names).into();
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
                self.status = ui::tf!("未配線のコマンド: {}(不具合です)", other).into();
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
                self.status = ui::tf!("{} が開いているため上書きしません。名前を付けて保存してください", self.locked_by.as_deref().unwrap_or("誰か"))
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
                    None => this.status = ui::t!("保存をやめました(名前が決まっていません)").into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 決まった場所へ書く。成功すると dirty が消える。
    fn save_to(&mut self, p: PathBuf) {
        // 原本の部品(図形・テーマ・印刷設定)を持ち越す。読み終えてから書く。
        // 暗号化されていた原本は解いた平文を渡す
        let original: Option<std::io::Cursor<Vec<u8>>> =
            self.original_plain().map(std::io::Cursor::new);
        // 上書きの前に、直前の中身をバージョン履歴に控える
        if p.exists() {
            self.keep_version(&p);
        }
        let saved = if let Some(pw) = self.encrypt_pw.clone() {
            // 暗号化は zip 丸ごとが単位 — 一度メモリへ書いてから包む。
            // Agile 方式(AES-256。Excel 2013+ の既定)で書く — 本物と相互
            // 検証済み。読みは Standard(2007)も Agile も両方できる
            let mut plain = Vec::new();
            sheet::xlsx::write_with(&self.book, original, std::io::Cursor::new(&mut plain))
                .and_then(|_| ooxml::crypt::encrypt_agile(&plain, &pw))
                .and_then(|enc| {
                    kumihan::atomic::save(&p, |mut f| {
                        use std::io::Write as _;
                        f.write_all(&enc).map_err(|e| e.to_string())
                    })
                })
        } else {
            kumihan::atomic::save(&p, |f| {
                sheet::xlsx::write_with(&self.book, original, std::io::BufWriter::new(f))
            })
        };
        match saved {
            Ok(_) => {
                let enc_note = if self.encrypt_pw.is_some() { "(暗号化)" } else { "" };
                self.status = ui::tf!("保存しました — {}{}", p.file_name().unwrap_or_default().to_string_lossy(), enc_note)
                .into();
                self.acquire_lock(&p);
                Self::note_recent(&p);
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
            Err(e) => self.status = ui::tf!("保存できません: {}", e).into(),
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
        // 空白キーはチェックボックス(Bool のセル)の切替。打ちかけ・板・
        // 小窓が無いときだけ(文字としての空白を奪わない)
        if text == " " && self.prompt.is_none() && self.solver.is_none() && !self.editing() {
            if let Some(Value::Bool(b)) =
                self.sheet().get(self.cursor).map(|c| c.value.clone())
            {
                if self.sheet().protected {
                    self.status =
                        ui::t!("シートが保護されています(保護タブの「保護」で解除)").into();
                } else {
                    self.checkpoint();
                    let p = self.cursor;
                    let mut cell = self.sheet().get(p).cloned().unwrap_or_default();
                    cell.formula = None;
                    cell.value = Value::Bool(!b);
                    self.book.sheets[self.active].set(p, cell);
                    recalc_book(&mut self.book, self.active);
                    self.dirty = true;
                    self.sync_input();
                    self.status = ui::tf!("{} = {}(空白キーで切替)", p.a1(), if b { "☐" } else { "☑" })
                    .into();
                }
                cx.notify();
                return;
            }
        }
        // セルを選んで**打ち始めたら置き換え**(Excel の作法)。追記になるのは
        // 同じセルで編集を続けている間(edit_armed)だけ — F2・ダブルクリック・
        // 2打目以降。IME の変換途中(marked)は消さない
        if self.prompt.is_none() && self.solver.is_none()
            && self.name_edit.is_none() && self.fn_dlg.is_none()
            && self.fn_args.is_none()
            && !self.edit_armed && !self.editing()
            && handler::marked_range_utf16(self).is_none()
        {
            self.input = Editor::new("");
            self.edit_armed = true;
        }
        handler::replace(self, r, text);
        cx.notify();
    }
    fn replace_and_mark_text_in_range(&mut self, r: Option<Range<usize>>, text: &str,
                                      sel: Option<Range<usize>>, _w: &mut Window,
                                      cx: &mut Context<Self>) {
        // IME の1打目も同じ(変換中の下線ごと、空にしてから始める)
        if self.prompt.is_none() && self.solver.is_none()
            && self.name_edit.is_none() && self.fn_dlg.is_none()
            && self.fn_args.is_none()
            && !self.edit_armed && !self.editing()
            && handler::marked_range_utf16(self).is_none()
        {
            self.input = Editor::new("");
            self.edit_armed = true;
        }
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
                } else if c.drag.is_some()
                    || c.head_drag.is_some()
                    || c.ink_cur.is_some()
                    || c.tool == Some(2)
                    // 関数の引数・式の直入力のセル掴み(範囲をなぞる)も
                    // ここを通す — この表に入れ忘れると「押せるのに伸びない」
                    // (writer で踏んだ罠)
                    || c.fn_args.as_ref().is_some_and(|a| a.pick_from.is_some())
                    || c.ref_pick.is_some()
                {
                    // 筆と消しゴムもここを通る(描きかけ・なぞり)
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

/// 方程式の台本(matplotlib の mathtext)。式を清書して透過 PNG に描く。
const EQ_PY: &str = r#"
import json, sys
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib import font_manager

spec = json.load(open(sys.argv[1], encoding="utf-8"))
if spec.get("font"):
    try:
        font_manager.fontManager.addfont(spec["font"])
        plt.rcParams["font.family"] = font_manager.FontProperties(
            fname=spec["font"]).get_name()
    except Exception:
        pass
fig = plt.figure()
t = fig.text(0.05, 0.5, "$%s$" % spec["tex"], fontsize=20)
fig.canvas.draw()  # 式が読めなければここで止まる(黙って白紙にしない)
bbox = t.get_window_extent()
fig.set_size_inches(bbox.width / fig.dpi + 0.15, bbox.height / fig.dpi + 0.15)
plt.savefig(spec["out"], dpi=200, transparent=True)
"#;

/// テキストアートの台本(matplotlib)。太字+塗り+縁取りの飾り文字を
/// 透過 PNG に描く(色は calc の緑)。
const TEXTART_PY: &str = r##"
import json, sys
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import matplotlib.patheffects as pe
from matplotlib import font_manager

spec = json.load(open(sys.argv[1], encoding="utf-8"))
if spec.get("font"):
    try:
        font_manager.fontManager.addfont(spec["font"])
        plt.rcParams["font.family"] = font_manager.FontProperties(
            fname=spec["font"]).get_name()
    except Exception:
        pass
fig = plt.figure()
t = fig.text(0.05, 0.5, spec["tex"], fontsize=44, fontweight="bold",
             color="#1B6E3C",
             path_effects=[pe.withStroke(linewidth=6, foreground="#D5E8DC")])
fig.canvas.draw()
bbox = t.get_window_extent()
fig.set_size_inches(bbox.width / fig.dpi + 0.2, bbox.height / fig.dpi + 0.2)
plt.savefig(spec["out"], dpi=200, transparent=True)
"##;

/// ソルバーの台本(scipy)。指図は JSON、答えは \x1f 区切りの変数の値。
const SOLVER_PY: &str = r#"
import json, sys
from scipy.optimize import linprog

spec = json.load(open(sys.argv[1], encoding="utf-8"))
n = len(spec["c"])
lo = 0 if spec["nonneg"] else None
r = linprog(
    c=spec["c"],
    A_ub=spec["aub"] or None,
    b_ub=spec["bub"] or None,
    A_eq=spec["aeq"] or None,
    b_eq=spec["beq"] or None,
    bounds=[(lo, None)] * n,
    method="highs",
)
if not r.success:
    sys.exit("解がありません: " + str(r.message))
sys.stdout.write("\x1f".join("%.12g" % v for v in r.x))
"#;

/// ピボットの台本(polars)。指図は JSON、答えは CSV 取り込みと同じ
/// 区切りの印(\x1e 行 / \x1f 欄)で返す。
const PIVOT_PY: &str = r#"
import json, sys
import polars as pl

spec = json.load(open(sys.argv[1], encoding="utf-8"))
headers = spec["headers"]
data = {h: [row[i] for row in spec["rows"]] for i, h in enumerate(headers)}
df = pl.DataFrame(data)
val, agg = spec["value"], spec["agg"]
if agg != "個数":
    # 数にならないものは null(集計から外れる)
    df = df.with_columns(pl.col(val).cast(pl.Float64, strict=False))
idx, cols = spec["index"], spec["columns"]
FN = {"合計": "sum", "平均": "mean", "個数": "len", "最大": "max", "最小": "min"}

def agg_expr():
    return {"合計": pl.sum(val), "平均": pl.mean(val), "個数": pl.len().alias(val),
            "最大": pl.max(val), "最小": pl.min(val)}[agg]

def table(frame, index):
    if cols:
        return frame.pivot(cols, index=index, values=val,
                           aggregate_function=FN[agg], sort_columns=True).sort(index)
    return frame.group_by(index).agg(agg_expr()).sort(index)

def stub(frame, label, index):
    # index の1列目に札を立て、残りを空にした複製。ピボットに通すことで
    # 列名の並びを main と揃えたまま「1行に集めた」答えが得られる
    ex = [pl.lit(label).alias(index[0])] + [pl.lit("").alias(i) for i in index[1:]]
    return frame.with_columns(ex)

def row_total(frame, index):
    # 行ごとの総計(列に広げたぶんを全部まとめた値)。集計の種類を守る
    return {tuple(r[:-1]): r[-1]
            for r in frame.group_by(index).agg(agg_expr().alias("_t")).rows()}

main = table(df, idx)
tot_col = spec["totals"] and bool(cols)
tots = row_total(df, idx) if tot_col else {}

out = []  # (種別, 欄) 種別: d=データ s=小計 b=空行 t=総計

sub = None
if spec["subtotals"] and len(idx) >= 2:
    sub = {r[0]: list(r[1:]) for r in table(df, [idx[0]]).rows()}
    sub_tots = row_total(df, [idx[0]]) if tot_col else {}

# 1つ目の見出しで束ねながら吐く(小計・空行はその区切りごと)
groups = []
for r in main.rows():
    if groups and groups[-1][0] == r[0]:
        groups[-1][1].append(r)
    else:
        groups.append((r[0], [r]))

for g, rs in groups:
    prev = None
    for r in rs:
        cells = list(r)
        if spec["compact"] and prev is not None:
            # 繰り返しの見出しを空欄に(コンパクト形式)
            for i in range(len(idx)):
                if cells[i] == prev[i]:
                    cells[i] = ""
                else:
                    break
        if tot_col:
            cells.append(tots.get(tuple(r[:len(idx)])))
        out.append(("d", cells))
        prev = list(r)
    if sub is not None:
        cells = [f"{g} 小計"] + [""] * (len(idx) - 1) + sub[g]
        if tot_col:
            cells.append(sub_tots.get((g,)))
        out.append(("s", cells))
    if spec["blank_rows"] and len(idx) >= 2:
        out.append(("b", [""] * (len(main.columns) + (1 if tot_col else 0))))

if spec["totals"]:
    cells = list(table(stub(df, "総計", idx), idx).rows()[0])
    if tot_col:
        cells.append(df.select(agg_expr()).item())
    out.append(("t", cells))

def s(v):
    if v is None:
        return ""
    if isinstance(v, float):
        return "%g" % v
    return str(v)

head = list(main.columns) + (["総計"] if tot_col else [])
lines = ["h\x1f" + "\x1f".join(head)]
for kind, cells in out:
    lines.append(kind + "\x1f" + "\x1f".join(s(v) for v in cells))
sys.stdout.buffer.write("\x1e".join(lines).encode("utf-8"))
"#;

/// プラグイン(.py)の置き場。writer と同じ ~/.config/office/plugins
fn plugins_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/office/plugins")
}

/// 署名の鍵の置き場。writer と共通の ~/.config/office/sign.key
fn sign_key_path() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".config/office/sign.key")
}

/// 署名の鍵を読む。無ければ作る(/dev/urandom の種。0600 で置く)
fn load_or_make_key() -> Result<ed25519_dalek::SigningKey, String> {
    let kp = sign_key_path();
    if let Ok(bytes) = std::fs::read(&kp) {
        let seed: [u8; 32] = bytes
            .get(..32)
            .and_then(|b| b.try_into().ok())
            .ok_or("鍵ファイルが壊れています(~/.config/office/sign.key)")?;
        return Ok(ed25519_dalek::SigningKey::from_bytes(&seed));
    }
    let mut seed = [0u8; 32];
    use std::io::Read as _;
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut seed))
        .map_err(|e| ui::tf!("乱数が取れません: {}", e))?;
    if let Some(dir) = kp.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&kp)
        .and_then(|mut f| f.write_all(&seed))
        .map_err(|e| ui::tf!("鍵が置けません: {}", e))?;
    Ok(ed25519_dalek::SigningKey::from_bytes(&seed))
}

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok())
        .collect()
}

/// 署名の添え書きの置き場。ブックの隣の 名前.xlsx.sig
fn sig_path_for(p: &std::path::Path) -> PathBuf {
    let mut os = p.as_os_str().to_owned();
    os.push(".sig");
    PathBuf::from(os)
}

/// AI に頼む仕事(calc 流)。writer と同じ10釦だが、表計算なので
/// 渡すのは選択範囲の TSV、返してもらうのも TSV や式になる。
#[derive(Clone)]
enum CalcAi {
    /// 選択(無ければ使っている範囲)の表を要約 → カーソルのコメントへ
    Summary,
    /// 文字のセルを書き直して置き換える(整える・敬語・やさしく)
    Rewrite(&'static str, &'static str),
    /// 文字のセルを訳して置き換える
    Translate,
    /// 選択した1列の読みを右隣の列へ(名簿のフリガナ欄)
    Furigana,
    /// 選択のパターンから続きの行を作り、下の空きへ
    Continue,
    /// 文章から表を作り、カーソルから流し込む
    Table(String),
    /// 自由に頼む。= で始まる答えは式としてカーソルへ、他はコメントへ
    Ask(String),
}

impl CalcAi {
    /// モデルへの言いつけ(system)と、何を渡すか
    fn prompt(&self) -> (&'static str, &'static str) {
        match self {
            CalcAi::Summary => (
                "あなたは表を読む道具です。渡されたタブ区切りの表の要点を、                 2〜4文の日本語でまとめてください。前置き・後書きは書かず、                 要約の本文だけを返します。",
                "次の表を要約してください。",
            ),
            CalcAi::Rewrite(sys, ask) => (sys, ask),
            CalcAi::Translate => (
                "あなたは表の中の文字を訳す道具です。渡されたタブ区切りの表と                 同じ行数・同じ列数のタブ区切りだけを返します。文字は日本語なら                 英語へ、それ以外なら日本語へ訳し、数字と空欄はそのまま写します。                 説明は書きません。",
                "次の表の文字を訳してください。",
            ),
            CalcAi::Furigana => (
                "あなたは日本語の読みを返す道具です。渡された1行1語の並びに                 対して、同じ行数で、各行にその語の読みをカタカナだけで返します。                 説明・記号は書きません。読めない行は空行にします。",
                "次の各行の読みをカタカナで返してください。",
            ),
            CalcAi::Continue => (
                "あなたは表のパターンを読む道具です。渡されたタブ区切りの表の                 規則を読み取り、**続きの行を3行だけ**、同じ列数のタブ区切りで                 返します。元の行は返しません。説明は書きません。",
                "次の表の続きの行を作ってください。",
            ),
            CalcAi::Table(_) => (
                "あなたは文章を表に整える道具です。渡された文章から表を作り、                 タブ区切り(1行目は見出し)だけを返します。説明・前置き・                 罫線の記号は書きません。",
                "",
            ),
            CalcAi::Ask(_) => (
                "あなたは表計算を手伝う道具です。数式を頼まれたら = で始まる                 1つの数式だけを返します(使える関数: SUM AVERAGE COUNT COUNTA                  MIN MAX SUMIF COUNTIF ABS MOD POWER SQRT INT ROUND ROUNDUP TRUNC                  PRODUCT PMT PV FV NPER TODAY NOW DATE YEAR MONTH DAY WEEKDAY LEN                  LEFT RIGHT MID TRIM UPPER LOWER CONCATENATE IF AND OR NOT IFERROR                  ISBLANK ISERROR VLOOKUP HLOOKUP INDEX MATCH)。それ以外の頼みには                 答えの本文だけを返します。前置きは書きません。",
                "",
            ),
        }
    }

    fn label(&self) -> &'static str {
        match self {
            CalcAi::Summary => "要約",
            CalcAi::Rewrite(_, _) => "書き直し",
            CalcAi::Translate => "翻訳",
            CalcAi::Furigana => "ふりがな",
            CalcAi::Continue => "続き",
            CalcAi::Table(_) => "表",
            CalcAi::Ask(_) => "頼み",
        }
    }
}

/// セルのスタイル(本家の「セルのスタイル」。よく使う組だけ)。
/// 表オブジェクトは持たない方針どおり、掛けるのは普通の書式 —
/// どれも Ctrl+Z の1手で戻る
#[allow(clippy::type_complexity)]
const CELL_STYLES: &[(&str, fn(&mut CellFormat))] = &[
    ("標準", |f| *f = CellFormat::default()),
    ("見出し", |f| {
        f.bold = true;
        f.fill = Some("D5E8DC".into());
        f.borders.bottom = true;
    }),
    ("表題", |f| {
        f.bold = true;
        f.size_c = Some(1600);
        f.color = Some("1B6E3C".into());
    }),
    ("良い", |f| {
        f.fill = Some("C6EFCE".into());
        f.color = Some("006100".into());
    }),
    ("悪い", |f| {
        f.fill = Some("FFC7CE".into());
        f.color = Some("9C0006".into());
    }),
    ("どちらでもない", |f| {
        f.fill = Some("FFEB9C".into());
        f.color = Some("9C6500".into());
    }),
    ("メモ", |f| {
        f.fill = Some("FFFFCC".into());
        f.borders = Borders::ALL;
    }),
    ("計算", |f| {
        f.italic = true;
        f.fill = Some("F2F2F2".into());
        f.color = Some("7F7F7F".into());
    }),
    ("通貨", |f| f.number_format = Some("¥#,##0".into())),
    ("パーセント", |f| f.number_format = Some("0.0%".into())),
];

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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 窓の大きさを控える(見える行数・列数がこれに追従する)
        self.view_w_px = f32::from(window.viewport_size().width);
        self.view_h_px = f32::from(window.viewport_size().height);
        if std::env::var_os("JO_SELFTEST").is_some() {
            // 実際に描画が走った証拠を残す(notify だけでは画面は変わらない —
            // これが止まってティックが続くなら、提示(present)の停止)
            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            eprintln!("render #{}", N.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        }
        // ---- 画面の額縁(デスクトップ版の形。writer と同じ構成) ----
        // 1段目 = クイックアクセス+ブック名(この行が窓の取っ手)。
        // 表計算の色は緑(デスクトップ版の app 色分けと同じ)。
        // 2段目 = 白地のタブ+現在地の緑の下線。右端に 🔍。
        // 下端 = ステータスバー(シートの耳+状態の文言+選択の生きた値)
        let (ready, all) = ribbon::progress(ribbon::calc_tabs());
        // 画面の明暗(インターフェイステーマ)。**セルは白のまま** —
        // 暗くするのは周り(帯・タブ・釦・見出し・耳)だけ
        let dk = self.dark;
        let th_bar = if dk { rgb(0x14432A) } else { rgb(0x1B6E3C) };
        let th_band = if dk { rgb(0x1B1E21) } else { rgb(0xFFFFFF) };
        let th_fg = if dk { rgb(0xCFD6DC) } else { rgb(0x444B52) };
        let th_gray = if dk { rgb(0x565D64) } else { rgb(0xB6BDC4) };
        let th_hover = if dk { rgb(0x2C333A) } else { rgb(0xEAF5EE) };
        let th_line = if dk { rgb(0x33383D) } else { rgb(0xE1E6EA) };
        let th_head = if dk { rgb(0x22262A) } else { rgb(0xEFF2F4) };
        let qa = |id: &'static str, icon: &'static str| {
            div().id(id).px_2().py_1().rounded_sm().cursor_pointer()
                .hover(move |s| s.bg(rgb(0x2E8B57)))
                .child(gpui::svg()
                    .path(SharedString::from(format!("icons/{icon}.svg")))
                    .size(px(15.0)).text_color(rgb(0xE8F3EC)))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let title = self
            .path
            .as_ref()
            .and_then(|q| q.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| ui::t!("無題のブック").into());
        let winbtn = |id: &'static str, label: &'static str| {
            div().id(id).px_2p5().py_1().rounded_sm()
                .text_size(px(12.0)).text_color(rgb(0xCFE6D8))
                .cursor_pointer()
                .hover(move |s| if id == "close" { s.bg(rgb(0xC0392B)).text_color(rgb(0xFFFFFF)) }
                                else { s.bg(rgb(0x2E8B57)).text_color(rgb(0xFFFFFF)) })
                .child(label)
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let top = div().id("titlebar").flex().flex_row().items_center().gap_0p5()
            .px_2().py_0p5().bg(th_bar)
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                |_, e: &gpui::MouseDownEvent, window, _| {
                    if e.click_count >= 2 {
                        window.zoom_window();
                    } else {
                        window.start_window_move();
                    }
                }))
            .child(qa("qa-save", "save").on_click(cx.listener(|this, _, _, cx| {
                this.run_cmd("save", cx);
                cx.notify()
            })))
            .child(qa("qa-print", "print").on_click(cx.listener(|this, _, _, cx| {
                this.run_cmd("pdf", cx);
                cx.notify()
            })))
            .child(qa("qa-undo", "undo").on_click(cx.listener(|this, _, _, cx| {
                this.run_cmd("undo", cx);
                cx.notify()
            })))
            .child(qa("qa-redo", "redo").on_click(cx.listener(|this, _, _, cx| {
                this.run_cmd("redo", cx);
                cx.notify()
            })))
            .child(div().flex_1())
            .child(div().text_size(px(12.5)).text_color(rgb(0xFFFFFF))
                .whitespace_nowrap().overflow_hidden()
                .child(SharedString::from(format!(
                    "{}{title}",
                    if self.dirty { "*" } else { "" }
                ))))
            .child(div().flex_1())
            .child(div().pr_2().text_size(px(10.5)).text_color(rgb(0x9CC9AF))
                .child(SharedString::from(ui::tf!("calc — 実装済み {}/{}", ready, all))))
            .child(winbtn("min", "─").on_click(cx.listener(|_, _, window, _| {
                window.minimize_window();
            })))
            .child(winbtn("max", "▢").on_click(cx.listener(|_, _, window, _| {
                window.zoom_window();
            })))
            .child(winbtn("close", "✕").on_click(cx.listener(|this, _, _, cx| {
                this.request_quit(cx);
            })));

        let mut tabs = div().flex().flex_row().items_end().gap_1()
            .px_2().bg(th_band);
        for (i, tb) in ribbon::calc_tabs().iter().enumerate() {
            let on = i == self.tab;
            tabs = tabs.child(div()
                .id(SharedString::from(format!("tab{i}")))
                .px_2p5().pt_1p5()
                .text_size(px(12.0))
                .text_color(if on { rgb(0x2E8B57) } else { th_fg })
                .font_weight(if on { gpui::FontWeight::BOLD } else { gpui::FontWeight::NORMAL })
                .cursor_pointer()
                .hover(|s| s.text_color(rgb(0x1B6E3C)))
                .flex().flex_col().items_center().gap_1()
                .child(tb.name)
                // 現在地の緑の下線(デスクトップ版の形)
                .child(div().h(px(2.5)).w_full().rounded_sm()
                    .bg(if on { rgb(0x2E8B57) } else { th_band }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.tab != 0 {
                        this.prev_tab = this.tab;
                    }
                    this.tab = i;
                    cx.notify()
                })));
        }
        tabs = tabs.child(div().flex_1())
            .child(div().id("tab-find").px_2().pb_1().text_size(px(12.0))
                .text_color(rgb(0x555E66)).cursor_pointer()
                .hover(|s| s.text_color(rgb(0x1B6E3C)))
                .child("🔍")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.run_cmd("replace", cx);
                    cx.notify()
                })));

        // 釦の帯: 本家のデスクトップ版の一段の絵釦(writer の写し)。
        // 主要な釦は名札つきの大釦、他は絵だけ(乗ると名前が下のステータス
        // バーへ)。絵の無い釦は小さな文字の釦。ホームだけ2段(釦が多い)
        const BIG: &[(&str, &str)] = &[
            ("instable", "表"), ("insimage", "画像"), ("insshape", "図形"),
            ("inschart", "グラフ"), ("inssmartart", "SmartArt"),
            ("autosum", "オートSUM"), ("recent", "最近使った関数"),
            ("pagemargins", "余白"), ("pageorient", "向き"), ("pagesize", "サイズ"),
            ("printarea", "印刷範囲"),
            ("data-from-text", "テキストから"), ("custom-sort", "並べ替え"),
            ("setfilter", "フィルター"), ("python", "Python"),
            ("subtotal", "小計"), ("solver", "ソルバー"), ("group", "グループ化"),
            ("pivot-insert", "ピボットの挿入"),
            ("td-header", "ヘッダー行"), ("td-total", "合計行"),
            ("coauth-mode", "共同編集モード"), ("co-addcomment", "コメント"),
            ("co-chat", "チャット"), ("co-history", "バージョン履歴"),
            ("prot-encrypt", "暗号化"), ("prot-sign", "署名"), ("prot-doc", "保護"),
            ("freeze", "枠の固定"), ("pen", "ペン"), ("highlighter", "蛍光ペン"),
            ("eraser", "消しゴム"),
            ("plug-macros", "マクロ"), ("plug-manage", "プラグインの管理"),
        ];
        let th_cmd_border = th_line;
        let th_btn_hover = th_hover;
        let mut cmds = div().flex().flex_col().gap_0p5()
            .px_3().py_1().bg(th_band)
            .border_b_1().border_color(th_cmd_border);
        let items = ribbon::calc_tabs()[self.tab].cmds;
        // 1つの釦を組み立てる(名札つきの大釦 / 絵だけ / 文字の小釦)。
        // ホームの対の並びと、他タブの一段の並びの両方から使う
        let mk_btn = |cmd: &ribbon::Cmd, cx: &mut Context<Self>| -> gpui::AnyElement {
            let label = cmd.label;
            let icon = cmd.icon;
            let has_icon = ui::icons::find(icon).is_some();
            let big = BIG.iter().find(|(k, _)| *k == icon).map(|(_, s)| *s);
            // 名札の短い形は ja 向け — 他の言語では表の語を使う
            let big = if ui::settings::language() == "ja" {
                big
            } else {
                big.map(|_| cmd.label)
            };
            let hoverable = cx.listener(move |this: &mut Calc, on: &bool, _, cx| {
                if *on {
                    this.hover_hint = Some(label);
                } else if this.hover_hint == Some(label) {
                    this.hover_hint = None;
                }
                cx.notify()
            });
            let fg = if cmd.ready { th_fg } else { th_gray };
            if let Some(short) = big {
                // 名札つきの大釦(絵の下に短い名前 — 本家の言い方)
                let mut b = div().id(SharedString::from(format!("h-{icon}")))
                    .px_2().h(px(46.0)).rounded_sm()
                    .flex().flex_col().items_center().justify_center().gap_1()
                    .on_hover(hoverable)
                    .children(has_icon.then(|| {
                        gpui::svg()
                            .path(SharedString::from(format!("icons/{icon}.svg")))
                            .size(px(20.0)).text_color(fg)
                    }))
                    .child(div().text_size(px(10.5)).text_color(fg).child(short));
                if cmd.ready {
                    let cid = cmd.id;
                    b = b.cursor_pointer().hover(move |st| st.bg(th_btn_hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.run_cmd(cid, cx);
                            cx.notify()
                        }));
                }
                return b.into_any_element();
            }
            let mut b = div().id(SharedString::from(format!("h-{icon}")))
                .h(px(26.0)).rounded_sm()
                .flex().items_center().justify_center()
                .on_hover(hoverable);
            b = if has_icon { b.w(px(26.0)) } else { b.px_1p5() };
            b = b
                .children(has_icon.then(|| {
                    gpui::svg()
                        .path(SharedString::from(format!("icons/{icon}.svg")))
                        .size(px(18.0)).text_color(fg)
                }))
                .children((!has_icon).then(|| {
                    div().text_size(px(10.5)).text_color(fg).child(label)
                }));
            if cmd.ready {
                let cid = cmd.id;
                b = b.cursor_pointer().hover(move |st| st.bg(th_btn_hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.run_cmd(cid, cx);
                        cx.notify()
                    }));
            }
            b.into_any_element()
        };
        if ribbon::CALC[self.tab].name == "ホーム" {
            // 本家のホームは**単純な2行割りではない**(発注者 2026-08-06
            // スクショ)。組ごとに上の段と下の段が対になっている —
            // コピーの下に貼り付け、書体の下に B I U…、縦揃えの下に横揃え。
            // その対をそのまま書き、組の間に縦の区切り線を引く
            const HOME_PAIRS: &[(&[&str], &[&str])] = &[
                (&["copy", "cut"], &["paste"]),
                (&["fontname", "fontsize", "incfont", "decfont", "changecase"],
                 &["bold", "italic", "underline", "strikeout", "subscript",
                   "fontcolor", "fillparag", "borders"]),
                (&["top", "middle", "bottom", "wrap", "text-orient"],
                 &["align-left", "align-center", "align-right", "align-just",
                   "merge", "direction"]),
                (&["insert-function", "fill-num"], &["defname", "clear"]),
                (&["format", "currency", "percents"],
                 &["comma", "digit-dec", "digit-inc"]),
                (&["cell-ins", "cell-del", "cell-format"],
                 &["condformat", "table-tpl", "cell-styles"]),
                (&["replace", "selectall"], &["setfilter", "clear-filter"]),
            ];
            let mut used: std::collections::HashSet<&str> = Default::default();
            let mut band = div().flex().flex_row().items_center().gap_1();
            let mut first = true;
            for (topr, botr) in HOME_PAIRS {
                if topr.iter().chain(botr.iter())
                    .all(|id| !items.iter().any(|c| c.id == *id))
                {
                    continue; // 表に無い組は出さない(将来の並び替えでも落ちない)
                }
                if !first {
                    band = band.child(div().w(px(1.0)).h(px(46.0))
                        .bg(th_cmd_border).mx_1());
                }
                first = false;
                let mut col = div().flex().flex_col().gap_0p5();
                for ids in [*topr, *botr] {
                    let mut r = div().flex().flex_row().items_center()
                        .gap_0p5().h(px(26.0));
                    for id in ids {
                        if let Some(cmd) = items.iter().find(|c| c.id == *id) {
                            used.insert(cmd.id);
                            r = r.child(mk_btn(cmd, cx));
                        }
                    }
                    col = col.child(r);
                }
                band = band.child(col);
            }
            // 対の表に無い釦も**黙って落とさない** — 右端に半々で足す
            let rest: Vec<&ribbon::Cmd> =
                items.iter().filter(|c| !used.contains(c.id)).collect();
            if !rest.is_empty() {
                band = band.child(div().w(px(1.0)).h(px(46.0))
                    .bg(th_cmd_border).mx_1());
                let half = rest.len().div_ceil(2);
                let mut col = div().flex().flex_col().gap_0p5();
                for chunk in rest.chunks(half.max(1)) {
                    let mut r = div().flex().flex_row().items_center()
                        .gap_0p5().h(px(26.0));
                    for cmd in chunk {
                        r = r.child(mk_btn(cmd, cx));
                    }
                    col = col.child(r);
                }
                band = band.child(col);
            }
            cmds = cmds.child(band);
        } else {
            let mut row = div().flex().flex_row().items_center().gap_0p5();
            for cmd in items {
                row = row.child(mk_btn(cmd, cx));
            }
            cmds = cmds.child(row);
        }
        let bar = if self.tab == 0 {
            // ファイルの全面ページは釦の帯を持たない(本家の形)
            div().flex().flex_col().child(top).child(tabs)
        } else {
            div().flex().flex_col().child(top).child(tabs).child(cmds)
        };

        // ---- 数式バー ----
        // クリックで**編集モード**(発注者 2026-08-06)— 置き換えでなく、
        // 押した位置に文字カーソルを立てて続きを直せる。編集中はキャレットを見せる
        let in_edit = self.editing() || self.edit_armed;
        let bar_text = {
            let mut t = self.input.text().to_string();
            if in_edit {
                let cur = self.input.cursor().min(t.len());
                t.insert(cur, '|');
            }
            if t.is_empty() { " ".to_string() } else { t }
        };
        // 名前ボックス(左端): 押すと打てる。番地・範囲・名前で飛び、
        // 知らない名前ならいまの選択に付ける(Excel の名前ボックス)
        let name_box = if let Some(ed) = &self.name_edit {
            let mut t = ed.text().to_string();
            let cur = ed.cursor().min(t.len());
            t.insert(cur, '|');
            div().w(px(88.0)).px_1().py_0p5().bg(gpui::white())
                .border_1().border_color(rgb(0x1B6E3C)).rounded_sm()
                .text_size(px(12.0)).whitespace_nowrap().overflow_hidden()
                .child(SharedString::from(t))
        } else {
            div().w(px(88.0)).px_1().py_0p5()
                .border_1().border_color(rgb(0xC6CDD3)).rounded_sm()
                .text_size(px(12.0))
                .font_weight(gpui::FontWeight::BOLD).text_color(rgb(0x1B6E3C))
                .cursor_text()
                .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.name_edit = Some(Editor::new(""));
                    this.status = ui::t!(
                        "名前ボックス: 番地(B12)・範囲(A1:C9)・名前で移動。\
                         知らない名前は選択に付きます")
                    .into();
                    cx.notify();
                }))
                .child(SharedString::from(self.cursor.a1()))
        };
        let formula_bar = div()
            .flex().flex_row().items_center().gap_2()
            .px_4().py_1p5().bg(rgb(0xFAFBFC))
            .border_b_1().border_color(rgb(0xE1E6EA))
            .child(name_box)
            // fx = 関数を挿入(本家と同じ場所)。幅は固定 —
            // 数式編集のクリック位置の換算(下の 156px)が崩れないように
            .child(div().id("fx").w(px(28.0)).py_0p5().rounded_sm()
                   .flex().items_center().justify_center()
                   .text_size(px(13.0)).italic()
                   .font_weight(gpui::FontWeight::BOLD).text_color(rgb(0x1B6E3C))
                   .cursor_pointer().hover(|s| s.bg(rgb(0xE4EFE8)))
                   .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                       cx.stop_propagation();
                       this.fn_dlg = Some(FnDlg {
                           search: Editor::new(""),
                           group: 0,
                           sel: 0,
                       });
                       cx.notify();
                   }))
                   .child("fx"))
            .child(div().flex_1().px_2().py_1().bg(gpui::white())
                   .border_1().border_color(if in_edit { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                   .rounded_sm()
                   .text_size(px(13.0)).font_family("Noto Sans JP")
                   .cursor_text()
                   .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                       |this, e: &gpui::MouseDownEvent, _, cx| {
                           cx.stop_propagation();
                           // 押した位置へ文字カーソル(幅は 全角=1em・半角=0.5em の見積り)。
                           // 起点 = 左余白16 + 名前ボックス88 + 隙間8 + fx 28 + 隙間8 + 内余白8
                           let x = f32::from(e.position.x)
                               - (16.0 + 88.0 + 8.0 + 28.0 + 8.0 + 8.0);
                           let text = this.input.text().to_string();
                           let mut acc = 0.0;
                           let mut at = text.len();
                           for (i, ch) in text.char_indices() {
                               let w = if (ch as u32) < 0x2E80 { 6.8 } else { 13.0 };
                               if acc + w / 2.0 > x {
                                   at = i;
                                   break;
                               }
                               acc += w;
                           }
                           this.input.move_to(at, false);
                           this.edit_armed = true;
                           this.status =
                               ui::t!("数式バーで編集: Enter で確定 / Esc で取消").into();
                           cx.notify();
                       }))
                   .child(SharedString::from(bar_text)));

        // ---- 折り返しの無い文字の、隣の空セルへのはみ出し(Excel の流儀) ----
        // 折り返し・縮小・回転・右横書きでない文字のセルで、伸びる方向の
        // 隣が空(値も式も無い)なら、そのセルの上にも描く(発注者 2026-08-06)。
        // 描くのは格子の後の重ね描き(spill_texts)で、セル側は文字を出さない
        let vis_cols: Vec<u32> = self.visible_cols();
        let mut spill_from: std::collections::HashSet<Pos> = Default::default();
        let mut spill_texts: Vec<gpui::Div> = Vec::new();
        if !self.show_formulas {
            let mut y = ROW_H;
            for r in self.visible_rows() {
                let rh = self.row_px(r);
                let mut x = HEAD_W;
                for (ci, &c) in vis_cols.iter().enumerate() {
                    let w = self.col_px(c);
                    let p = Pos::new(r, c);
                    let x0 = x;
                    x += w;
                    if p == self.cursor {
                        continue; // 編集中の見た目は従来どおり
                    }
                    let Some(cl) = self.sheet().get(p) else { continue };
                    let Value::Text(t) = &cl.value else { continue };
                    if t.is_empty() {
                        continue;
                    }
                    let f = &cl.fmt;
                    if f.wrap || f.shrink || f.rtl_text
                        || f.rotation.is_some_and(|r| r != 0)
                    {
                        continue;
                    }
                    if self.sheet().covered_by_merge(p)
                        || self.sheet().merges.iter().any(|(a, _)| *a == p)
                    {
                        continue;
                    }
                    let to_left = match f.align {
                        HAlign::Right => true,
                        HAlign::Left | HAlign::General => false,
                        _ => continue, // 中央・両端揃えは流さない
                    };
                    let t1 = t.replace('\n', " ");
                    let size = self.zoom
                        * f.size_c
                            .map(|c| c as f32 / 100.0 * 24.0 / 15.0 * 0.8)
                            .unwrap_or(12.5);
                    let units: f32 = t1
                        .chars()
                        .map(|ch| if (ch as u32) < 0x2E80 { 1.0 } else { 2.0 })
                        .sum();
                    let need = units * size * 0.52 + 14.0;
                    if need <= w {
                        continue; // 収まっている
                    }
                    // 伸びる方向の空きセルぶんだけ許す
                    let (mut avail, mut left_ext, mut k) = (w, 0.0f32, ci);
                    loop {
                        if need <= avail {
                            break;
                        }
                        let nk = if to_left {
                            k.checked_sub(1)
                        } else {
                            (k + 1 < vis_cols.len()).then_some(k + 1)
                        };
                        let Some(nk) = nk else { break };
                        let nc = vis_cols[nk];
                        let np = Pos::new(r, nc);
                        let occupied = self
                            .sheet()
                            .get(np)
                            .is_some_and(|q| !q.value.is_empty() || q.formula.is_some())
                            || self.sheet().covered_by_merge(np)
                            || np == self.cursor;
                        if occupied {
                            break;
                        }
                        let nw = self.col_px(nc);
                        avail += nw;
                        if to_left {
                            left_ext += nw;
                        }
                        k = nk;
                    }
                    if avail <= w {
                        continue; // 隣が塞がっている — 今までどおり切る
                    }
                    spill_from.insert(p);
                    let wd = avail.min(need);
                    let lx = if to_left { x0 + w - wd } else { x0 };
                    let _ = left_ext;
                    let mut d = div().absolute()
                        .left(px(lx)).top(px(y))
                        .w(px(wd)).h(px(rh))
                        .px_1p5().flex()
                        .text_size(px(size))
                        .font_family("Noto Sans JP")
                        .whitespace_nowrap().overflow_hidden();
                    match f.valign {
                        sheet::model::VAlign::Top => d = d.items_start(),
                        sheet::model::VAlign::Middle => d = d.items_center(),
                        sheet::model::VAlign::Bottom => d = d.items_end(),
                    }
                    d = if to_left { d.justify_end() } else { d.justify_start() };
                    if f.bold {
                        d = d.font_weight(gpui::FontWeight::BOLD);
                    }
                    if f.italic {
                        d = d.italic();
                    }
                    d = if let Some(cv) = &f.color {
                        d.text_color(hex(cv))
                    } else {
                        d.text_color(rgb(0x1B1B1B))
                    };
                    if let Some(name) = &f.font {
                        if let Ok((fam, _)) = kumihan::font::for_document(Some(name)) {
                            d = d.font_family(SharedString::from(fam.name.clone()));
                        }
                    }
                    spill_texts.push(d.child(SharedString::from(t1)));
                }
                y += rh;
            }
        }

        // ---- 格子 ----
        let mut grid = div().flex().flex_col();
        // 列見出し
        // 見出しもセルも flex_none — **窓の大きさで伸縮させない**
        // (窓に合わせるのは見える範囲。セルの大きさは設定どおり固定)
        let mut head = div().flex().flex_row().flex_none()
            .child(div().flex_none().w(px(HEAD_W)).h(px(ROW_H)).bg(th_head)
                   .border_r_1().border_b_1().border_color(rgb(0xD5DBE0)));
        let (sel_a, sel_b) = self.sel_rect();
        let has_sel = self.anchor.is_some();
        for c in self.visible_cols() {
            // 選択に入っている列の見出しは色を変える(いまどこを選んでいるかの道標)
            let on = has_sel && (sel_a.col..=sel_b.col).contains(&c) || c == self.cursor.col;
            head = head.child(div().flex_none().w(px(self.col_px(c))).h(px(ROW_H))
                .bg(if on { rgb(0xCFE6D8) } else { th_head })
                .border_r_1().border_b_1()
                .border_color(rgb(0xD5DBE0))
                .flex().items_center().justify_center()
                .text_size(px(11.5))
                .text_color(if on { rgb(0x1B6E3C) } else if dk { rgb(0x9AA5AE) } else { rgb(0x66707A) })
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
            let mut row = div().flex().flex_row().flex_none()
                .child(div().flex_none().w(px(HEAD_W)).h(px(rh))
                    .bg(if row_on { rgb(0xCFE6D8) } else { th_head })
                    .border_r_1().border_b_1()
                    .border_color(rgb(0xD5DBE0))
                    .flex().items_center().justify_center()
                    .text_size(px(11.5))
                    .text_color(if row_on { rgb(0x1B6E3C) } else if dk { rgb(0x9AA5AE) } else { rgb(0x66707A) })
                    .child(SharedString::from((r + 1).to_string()))
                    // 下端の帯は高さを変える取っ手(列見出しの右端と同じ仕掛け)
                    .relative().children((std::env::var_os("JO_NO_STRIPS").is_none()).then(|| {
                        div().absolute()
                            .left(px(0.0)).bottom(px(-GRIP)).w_full().h(px(GRIP * 2.0))
                            .cursor_row_resize()
                    }))
                    // グループ化の +/-(アウトラインの縁)。直前で終わる
                    // かたまりの頭金の行に置く(Excel の「集計行が下」の形)
                    .children({
                        let sh = self.sheet();
                        r.checked_sub(1).and_then(|pr| {
                            let lv = *sh.row_outline.get(&pr).unwrap_or(&0);
                            // かたまりが r の直前で**終わっている**ときだけ
                            // (続きの行に印を出さない)
                            if lv == 0 || *sh.row_outline.get(&r).unwrap_or(&0) >= lv {
                                return None;
                            }
                            let mut start = pr;
                            while start > 0
                                && *sh.row_outline.get(&(start - 1)).unwrap_or(&0) >= lv
                            {
                                start -= 1;
                            }
                            let hidden = sh.row_hidden.contains(&pr);
                            Some(div()
                                .id(SharedString::from(format!("gut{r}")))
                                .absolute().left(px(1.0)).top(px((rh - 11.0) / 2.0))
                                .w(px(11.0)).h(px(11.0)).rounded_sm()
                                .border_1().border_color(rgb(0x8FA3AE))
                                .bg(gpui::white())
                                .flex().items_center().justify_center()
                                .text_size(px(9.0)).text_color(rgb(0x1B6E3C))
                                .cursor_pointer()
                                .hover(|s| s.bg(rgb(0xEAF5EE)))
                                .child(if hidden { "+" } else { "−" })
                                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation()
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.checkpoint();
                                    for i in start..=pr {
                                        if hidden {
                                            this.sheet_mut().row_hidden.remove(&i);
                                        } else {
                                            this.sheet_mut().row_hidden.insert(i);
                                        }
                                    }
                                    this.dirty = true;
                                    this.status = if hidden {
                                        ui::t!("詳細を表示しました(+/− でいつでも)").into()
                                    } else {
                                        ui::t!("詳細を畳みました(+ で開きます)").into()
                                    };
                                    cx.notify()
                                })))
                        })
                    }));
            for c in self.visible_cols() {
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
                // Bool のセルはチェックボックスとして見せる(☑/☐。
                // 空白キーで切替。Excel では TRUE/FALSE の値で見える)
                let shown = match v {
                    Value::Bool(b) if !self.show_formulas => {
                        if b { "☑".to_string() } else { "☐".to_string() }
                    }
                    _ => shown,
                };
                let shown = if !self.show_zeros && matches!(v, Value::Number(n) if n == 0.0) {
                    String::new()
                } else {
                    shown
                };
                let is_num = matches!(v, Value::Number(_));
                let is_err = matches!(v, Value::Error(_));
                let sel = p == self.cursor;
                let (ra, rb) = self.sel_rect();
                let in_range = self.anchor.is_some()
                    && (ra.row..=rb.row).contains(&r) && (ra.col..=rb.col).contains(&c);
                let mut d = div()
                    .id(SharedString::from(p.a1()))
                    .flex_none()
                    .w(px(self.col_px(c))).h(px(rh))
                    .border_r_1().border_b_1()
                    .border_color(if self.gridlines { rgb(0xE1E6EA) } else { rgb(0xFFFFFF) })
                    .bg(rgb(0xFFFFFF))
                    .flex().items_center()
                    .px_1p5()
                    .text_size(px(self.zoom * cell.and_then(|x| x.fmt.size_c)
                        .map(|c| c as f32 / 100.0 * 24.0 / 15.0 * 0.8)
                        .unwrap_or(12.5)))
                    .font_family("Noto Sans JP")
                    .overflow_hidden().whitespace_nowrap()
                    // セルの上は Excel と同じ十字(手のひらだと「押す物」に見える)
                    .cursor(gpui::CursorStyle::Crosshair);
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
                // 下付きは小さく下げて見せる(xlsx へは vertAlign で入る)
                if f.subscript {
                    d = d.text_size(px(self.zoom * 8.5)).pt_2();
                }
                // 縦積み(255)は1字ずつ縦に並べる — 日本の帳票の縦の見出し。
                // 90/180 度は GPUI に字の回転が無いので、いまは縦積みで見せる
                if f.rotation.is_some_and(|r| r != 0) {
                    d = d.flex().flex_col().items_center();
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
                // 引いてある辺だけ濃くする(引いていない辺は表の薄い線のまま)。
                // border_color は div の**全辺に1色**なので使わない —
                // 使うと、外枠の上辺だけのセルで右・下の灰色の格子線まで
                // 黒くなり、外枠が格子に化ける(発注者報告)。
                // 辺ごとに細い帯を重ねて描く
                let ink = rgb(0x1B1B1B);
                if f.borders.top || f.borders.bottom || f.borders.left || f.borders.right {
                    d = d.relative();
                    if f.borders.top {
                        d = d.child(div().absolute().left(px(0.0)).top(px(0.0))
                            .w_full().h(px(1.0)).bg(ink));
                    }
                    if f.borders.bottom {
                        d = d.child(div().absolute().left(px(0.0)).bottom(px(0.0))
                            .w_full().h(px(1.0)).bg(ink));
                    }
                    if f.borders.left {
                        d = d.child(div().absolute().left(px(0.0)).top(px(0.0))
                            .w(px(1.0)).h_full().bg(ink));
                    }
                    if f.borders.right {
                        d = d.child(div().absolute().right(px(0.0)).top(px(0.0))
                            .w(px(1.0)).h_full().bg(ink));
                    }
                }
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
                // 縮小して全体を表示(折り返しと併せない)— 幅に収まるまで
                // 文字を小さくする。見積りは全角=1em・半角=0.5em
                if f.shrink && !f.wrap {
                    let size = self.zoom
                        * f.size_c
                            .map(|c| c as f32 / 100.0 * 24.0 / 15.0 * 0.8)
                            .unwrap_or(12.5);
                    let units: f32 = shown
                        .chars()
                        .map(|ch| if (ch as u32) < 0x2E80 { 1.0 } else { 2.0 })
                        .sum();
                    let need = units * size * 0.52 + 14.0;
                    let cw = self.col_px(c);
                    if need > cw && units > 0.0 {
                        d = d.text_size(px((size * cw / need).max(6.0)));
                    }
                }
                // 揃えの指定があればそちらが勝つ(既定は数=右・文字=左)
                match f.align {
                    HAlign::Left => d = d.justify_start(),
                    HAlign::Center => d = d.justify_center(),
                    HAlign::Right => d = d.justify_end(),
                    HAlign::Justify => d = d.justify_between(),
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
                // コメントのあるセルは右上に赤い角印(表示を消していれば出さない)
                if self.show_comments && self.sheet().comments.contains_key(&p) {
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
                // はみ出しで描くセルは、ここでは文字を出さない(二重描き防止)。
                // 折り返しの無いセルは改行を畳んで1行にする(発注者 2026-08-06)
                let shown = if spill_from.contains(&p) {
                    String::new()
                } else if !f.wrap && shown.contains('\n') {
                    shown.replace('\n', " ")
                } else {
                    shown
                };
                if f.rotation.is_some_and(|r| r != 0) {
                    let mut stack = d;
                    for ch in shown.chars() {
                        stack = stack.child(SharedString::from(ch.to_string()));
                    }
                    row = row.child(stack);
                } else if f.rtl_text {
                    // 右横書き: 1字ずつ右から並べる(昔の看板の書き方)。
                    // ラテン文字の bidi は扱わない — 日本語の右横書きのため
                    let rev: String = shown.chars().rev().collect();
                    row = row.child(d.justify_end().child(SharedString::from(rev)));
                } else {
                    row = row.child(d.child(SharedString::from(shown)));
                }
            }
            grid = grid.child(row);
        }
        // はみ出しの文字は格子の後に重ねる = 隣のセルの白地に負けない
        if !spill_texts.is_empty() {
            grid = grid.relative();
            for sp in spill_texts {
                grid = grid.child(sp);
            }
        }

        // ---- シートの耳(Excel と同じく下に置く) ----
        let mut sheets_bar = div().flex().flex_row().items_center().gap_1()
            .px_3().py_1().bg(th_head)
            .border_t_1().border_color(rgb(0xD5DBE0));
        for (i, s) in self.book.sheets.iter().enumerate() {
            if s.hidden {
                continue; // 隠したシートは耳に出さない(表示タブで戻す)
            }
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
                .child(SharedString::from(format!(
                    "{}{}",
                    if s.protected { "🔒" } else { "" },
                    s.name
                )))
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
        // 描きかけの1筆(点の粒で見せる。離すと1本の線になる)
        let ink_preview: Vec<gpui::AnyElement> = self
            .ink_cur
            .as_ref()
            .map(|pts| {
                let marker = self.tool == Some(1);
                let (sz, col) = if marker {
                    (9.0, rgb(0xFFD54A))
                } else {
                    (2.5, rgb(0x1B1B1B))
                };
                pts.iter()
                    .map(|(x, y)| {
                        div()
                            .absolute()
                            .left(px(x - sz / 2.0))
                            .top(px(y - sz / 2.0))
                            .w(px(sz))
                            .h(px(sz))
                            .rounded_full()
                            .bg(col)
                            .into_any_element()
                    })
                    .collect()
            })
            .unwrap_or_default();

        // 見張り(ウォッチウィンドウ)。控えたセルの値を下に並べる
        let watch_bar = (!self.watch.is_empty()).then(|| {
            let mut w = div().flex().flex_row().flex_wrap().gap_3()
                .px_3().py_1().bg(rgb(0xF7F9FA))
                .border_t_1().border_color(rgb(0xD5DBE0))
                .text_size(px(11.0)).text_color(rgb(0x1B1B1B));
            w = w.child(div().font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(0x1B6E3C)).child(ui::t!("見張り")));
            for (si, p) in self.watch.iter().take(24) {
                let Some(sh) = self.book.sheets.get(*si) else { continue };
                let v = sh.get(*p).map(|c| c.value.display()).unwrap_or_default();
                w = w.child(div().flex().flex_row().gap_1()
                    .child(div().text_color(rgb(0x66707A))
                        .child(SharedString::from(format!("{}!{}", sh.name, p.a1()))))
                    .child(div().font_weight(gpui::FontWeight::BOLD)
                        .child(SharedString::from(v))));
            }
            w
        });

        // 下端はステータスバーを兼ねる(デスクトップ版の形):
        // 状態の文言と、選択の生きた値(合計・平均・個数)
        sheets_bar = sheets_bar
            .child(div().pl_3().text_size(px(11.0)).text_color(rgb(0x66707A))
                .whitespace_nowrap().overflow_hidden()
                .child(SharedString::from(match self.hover_hint {
                    // 釦に乗っている間はその名前(本家の作法)
                    Some(h) => h.to_string(),
                    None => format!(
                        "{}{}",
                        if self.dirty { "● " } else { "" },
                        self.status
                    ),
                })))
            .child(div().flex_1())
            .children(self.sel_stats().map(|s| {
                div().pr_2().text_size(px(11.0)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x1B6E3C)).whitespace_nowrap()
                    .child(SharedString::from(s))
            }));

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
                + self.visible_cols()
                    .iter()
                    .map(|c| self.col_px(*c))
                    .sum::<f32>();
            let grid_h = if self.view_h_px > 0.0 {
                self.view_h_px - 120.0
            } else {
                ROW_H + ROWS as f32 * ROW_H
            };
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
            let (x, y) = (x + sp.dx_px, y + sp.dy_px);
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

        // ---- 関数を挿入の小窓(本家の FormulaDialog の形) ----
        // 検索 / 分類 / 一覧(↑↓で選ぶ・ダブルクリックで入る)/ 引数と説明
        let fn_panel = self.fn_dlg.as_ref().map(|d| {
            let list = fn_filtered(d.search.text(), d.group);
            let sel = d.sel.min(list.len().saturating_sub(1));
            let mut search_t = d.search.text().to_string();
            let cur = d.search.cursor().min(search_t.len());
            search_t.insert(cur, '|');
            let mut chips = div().flex().flex_row().flex_wrap().gap_1();
            for (gi, g) in FN_GROUPS.iter().enumerate() {
                let on = gi == d.group;
                chips = chips.child(div()
                    .id(SharedString::from(format!("fng{gi}")))
                    .px_2().py_0p5().rounded_sm().text_size(px(11.5))
                    .border_1()
                    .border_color(if on { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                    .bg(if on { rgb(0xE4EFE8) } else { rgb(0xFFFFFF) })
                    .text_color(if on { rgb(0x1B6E3C) } else { rgb(0x66707A) })
                    .cursor_pointer()
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(d) = &mut this.fn_dlg {
                            d.group = gi;
                            d.sel = 0;
                        }
                        cx.notify();
                    }))
                    .child(SharedString::from(ui::tr(g))));
            }
            let start = sel.saturating_sub(5);
            let mut lst = div().flex().flex_col().h(px(252.0)).overflow_hidden()
                .border_1().border_color(rgb(0xC6CDD3)).rounded_sm().bg(rgb(0xFFFFFF));
            if list.is_empty() {
                lst = lst.child(div().px_2().py_1().text_size(px(12.5))
                    .text_color(rgb(0x66707A))
                    .child(ui::t!("その条件の関数がありません")));
            }
            for (i, f) in list.iter().enumerate().skip(start).take(11) {
                let on = i == sel;
                lst = lst.child(div()
                    .id(SharedString::from(format!("fnr{i}")))
                    .px_2().py_0p5().text_size(px(12.5)).flex_none()
                    .bg(if on { rgb(0x1B6E3C) } else { rgb(0xFFFFFF) })
                    .text_color(if on { rgb(0xFFFFFF) } else { rgb(0x1B1B1B) })
                    .cursor_pointer()
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                        move |this, e: &gpui::MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            if let Some(d) = &mut this.fn_dlg {
                                d.sel = i;
                            }
                            if e.click_count >= 2 {
                                this.fn_next();
                            }
                            cx.notify();
                        }))
                    .child(SharedString::from(f.name)));
            }
            let (syntax, desc) = list
                .get(sel)
                .map(|f| (format!("{}{}", f.name, f.args), f.desc.to_string()))
                .unwrap_or_default();
            let btn = |id: &'static str, label: String, primary: bool| {
                div().id(id).px_3().py_1().rounded_sm().text_size(px(12.5))
                    .border_1()
                    .border_color(if primary { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                    .bg(if primary { rgb(0x1B6E3C) } else { rgb(0xFFFFFF) })
                    .text_color(if primary { rgb(0xFFFFFF) } else { rgb(0x1B1B1B) })
                    .cursor_pointer()
                    .child(SharedString::from(label))
            };
            div().absolute().inset_0().flex().items_center().justify_center()
                .child(div().w(px(430.0)).p_3().rounded_md().bg(rgb(0xF7F9FA))
                    .border_1().border_color(rgb(0x1B6E3C)).shadow_lg()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .flex().flex_col().gap_1p5()
                    .child(div().flex().flex_row().items_center()
                        .child(div().text_size(px(13.0)).font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(0x1B6E3C)).child(ui::t!("関数を挿入")))
                        .child(div().flex_1())
                        .child(div().id("fn-x").px_2().cursor_pointer().text_size(px(13.0))
                            .text_color(rgb(0x66707A)).hover(|s| s.text_color(rgb(0xC0392B)))
                            .child("✕")
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.fn_dlg = None;
                                cx.notify();
                            }))))
                    .child(div().px_2().py_1().bg(rgb(0xFFFFFF))
                        .border_1().border_color(rgb(0xC6CDD3)).rounded_sm()
                        .text_size(px(12.5)).whitespace_nowrap().overflow_hidden()
                        .child(SharedString::from(if search_t == "|" {
                            format!("|{}", ui::t!("(打つと絞り込み)"))
                        } else {
                            search_t
                        })))
                    .child(chips)
                    .child(lst)
                    .child(div().text_size(px(12.5)).font_weight(gpui::FontWeight::BOLD)
                        .child(SharedString::from(syntax)))
                    .child(div().text_size(px(11.5)).text_color(rgb(0x4A545E))
                        .min_h(px(48.0))
                        .child(SharedString::from(desc)))
                    .child(div().flex().flex_row().gap_2().justify_center()
                        .child(btn("fn-next", ui::t!("次へ").to_string(), true)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.fn_next();
                                cx.notify();
                            })))
                        .child(btn("fn-cancel", ui::t!("キャンセル").to_string(), false)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.fn_dlg = None;
                                cx.notify();
                            })))))
        });

        // ---- 関数の引数の画面(本家の第2段) ----
        // 引数ごとの欄と説明、結果の下見。セルをクリックすると欄に参照が入る
        let fn_args_panel = self.fn_args.as_ref().map(|a| {
            let mut rows_el = div().flex().flex_col().gap_1();
            for (i, (name, opt)) in a.names.iter().enumerate() {
                let on = i == a.focus;
                let mut t = a.eds[i].text().to_string();
                if on {
                    let cur = a.eds[i].cursor().min(t.len());
                    t.insert(cur, '|');
                }
                rows_el = rows_el.child(div()
                    .id(SharedString::from(format!("fna{i}")))
                    .flex().flex_row().items_center().gap_2()
                    .cursor_text()
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(a) = &mut this.fn_args {
                            a.focus = i;
                        }
                        cx.notify();
                    }))
                    .child(div().w(px(110.0)).text_size(px(12.0))
                        .text_color(rgb(0x1B1B1B))
                        .child(SharedString::from(if *opt {
                            format!("{name}(省略可)")
                        } else {
                            name.clone()
                        })))
                    .child(div().flex_1().px_2().py_0p5().bg(rgb(0xFFFFFF))
                        .border_1()
                        .border_color(if on { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                        .rounded_sm().text_size(px(12.5))
                        .whitespace_nowrap().overflow_hidden()
                        .child(SharedString::from(if t.is_empty() { " ".into() } else { t }))));
            }
            // いまの欄の説明(本家の ad — 引数順。可変長は最後の1つが代表)
            let arg_hint = a
                .names
                .get(a.focus)
                .map(|(n, _)| {
                    let d = a.f.arg_desc.get(a.focus)
                        .or(a.f.arg_desc.last())
                        .copied()
                        .unwrap_or("");
                    format!("{n}: {d}")
                })
                .unwrap_or_default();
            let btn = |id: &'static str, label: String, primary: bool| {
                div().id(id).px_3().py_1().rounded_sm().text_size(px(12.5))
                    .border_1()
                    .border_color(if primary { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                    .bg(if primary { rgb(0x1B6E3C) } else { rgb(0xFFFFFF) })
                    .text_color(if primary { rgb(0xFFFFFF) } else { rgb(0x1B1B1B) })
                    .cursor_pointer()
                    .child(SharedString::from(label))
            };
            div().absolute().inset_0().flex().items_center().justify_center()
                .child(div().w(px(520.0)).p_3().rounded_md().bg(rgb(0xF7F9FA))
                    .border_1().border_color(rgb(0x1B6E3C)).shadow_lg()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .flex().flex_col().gap_1p5()
                    .child(div().flex().flex_row().items_center()
                        .child(div().text_size(px(13.0)).font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(0x1B6E3C)).child(ui::t!("関数の引数")))
                        .child(div().flex_1())
                        .child(div().id("fna-x").px_2().cursor_pointer().text_size(px(13.0))
                            .text_color(rgb(0x66707A)).hover(|s| s.text_color(rgb(0xC0392B)))
                            .child("✕")
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.fn_args = None;
                                cx.notify();
                            }))))
                    .child(div().text_size(px(12.5)).font_weight(gpui::FontWeight::BOLD)
                        .child(SharedString::from(format!("{}{}", a.f.name, a.f.args))))
                    .child(div().text_size(px(11.5)).text_color(rgb(0x4A545E))
                        .child(SharedString::from(a.f.desc)))
                    .child(rows_el)
                    .child(div().text_size(px(11.5)).text_color(rgb(0x4A545E))
                        .min_h(px(44.0)).px_2().py_1()
                        .bg(rgb(0xEFF2F4)).rounded_sm()
                        .child(SharedString::from(arg_hint)))
                    .child(div().text_size(px(12.0))
                        .child(SharedString::from(ui::tf!("関数の結果 = {}", a.result))))
                    .child(div().text_size(px(11.0)).text_color(rgb(0x66707A))
                        .child(ui::t!("セルをクリックすると、いまの欄に参照が入ります")))
                    .child(div().flex().flex_row().gap_2().justify_center()
                        .child(btn("fna-back", ui::t!("戻る").to_string(), false)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.fn_args = None;
                                this.fn_dlg = Some(FnDlg {
                                    search: Editor::new(""),
                                    group: 0,
                                    sel: 0,
                                });
                                cx.notify();
                            })))
                        .child(btn("fna-ok", ui::t!("OK").to_string(), true)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.fn_args_ok();
                                cx.notify();
                            })))
                        .child(btn("fna-cancel", ui::t!("キャンセル").to_string(), false)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.fn_args = None;
                                cx.notify();
                            })))))
        });

        // ---- 終了確認の板(窓の中の中央。rfd はスクリーン中央に出て遠い) ----
        let quit_panel = self.quit_ask.then(|| {
            let btn = |id: &'static str, label: String, primary: bool| {
                div().id(id).px_3().py_1().rounded_sm().text_size(px(12.5))
                    .border_1()
                    .border_color(if primary { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                    .bg(if primary { rgb(0x1B6E3C) } else { rgb(0xFFFFFF) })
                    .text_color(if primary { rgb(0xFFFFFF) } else { rgb(0x1B1B1B) })
                    .cursor_pointer()
                    .child(SharedString::from(label))
            };
            div().absolute().inset_0().flex().items_center().justify_center()
                .child(div().w(px(420.0)).p_3().rounded_md().bg(rgb(0xF7F9FA))
                    .border_1().border_color(rgb(0x1B6E3C)).shadow_lg()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .flex().flex_col().gap_2()
                    .child(div().text_size(px(13.0)).font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(0x1B6E3C))
                        .child(ui::t!("保存していない変更があります")))
                    .child(div().text_size(px(12.0))
                        .child(ui::t!("保存して終了しますか?(Enter = 保存して終了 / Esc = やめる)")))
                    .child(div().flex().flex_row().gap_2().justify_center()
                        .child(btn("q-save", ui::t!("保存して終了").to_string(), true)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.quit_ask = false;
                                this.save(true, cx);
                                cx.notify();
                            })))
                        .child(btn("q-drop", ui::t!("保存せず終了").to_string(), false)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.release_lock();
                                cx.quit();
                            })))
                        .child(btn("q-cancel", ui::t!("キャンセル").to_string(), false)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.quit_ask = false;
                                this.status = ui::t!("終了をやめました").into();
                                cx.notify();
                            })))))
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
        if self.show_comments {
            if let Some(t) = self.sheet().comments.get(&self.cursor) {
                tip_lines.push(t.clone());
            }
        }
        if let Some(u) = self.sheet().links.get(&self.cursor) {
            tip_lines.push(ui::tf!("リンク: {}(Ctrl+クリックで開く)", u));
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
                "name" => ui::tf!("名前の定義 — {} に名前を付ける", range),
                "comment" => ui::tf!("コメント — {}(空にして Enter で消す)", self.cursor.a1()),
                "link" => ui::tf!("ハイパーリンク — {}(空にして Enter で外す)", self.cursor.a1()),
                "cond-gt" => ui::tf!("条件付き書式 — {} で、いくつより大きい値を塗る?", range),
                "cond-lt" => ui::tf!("条件付き書式 — {} で、いくつより小さい値を塗る?", range),
                "validation" => ui::tf!("入力規則 — {} は候補から選ぶ(空にして Enter で解除)", range),
                "find" => ui::t!("検索と置換 — 探す言葉").to_string(),
                "split-delim" => ui::tf!("区切り位置 — {} を何で割る?(空 Enter = カンマ)", range),
                "shape-text" => ui::t!("図形の文字(空にして Enter で消す)").to_string(),
                "py" => ui::t!("Python — 一行のコード(空 Enter = .py ファイルを選ぶ)").to_string(),
                "goal-target" => ui::t!("ゴールシーク — 目標(セル=値。例: D6=800000)").to_string(),
                "goal-var" => ui::tf!("{} をいくつにするか探します — 変えるセルは?(例: B2)", self.goal.map(|(p, v)| format!("{}={v}", p.a1())).unwrap_or_default()),
                "replace-with" => ui::tf!("「{}」を何に置き換える?", self.find_term.as_deref().unwrap_or("")),
                "chat" => ui::t!("チャット — 言伝を書き残す(ブックの隣の .chat.txt)").to_string(),
                "equation" => ui::t!("方程式 — 式を打つ(TeX の書き方。清書して画像で置く)").to_string(),
                "ai-table" => ui::t!("AI — 表にする文章").to_string(),
                "ai-ask" => ui::t!("AI — 頼み(例: 合計の式を書いて)").to_string(),
                "table-resize" => ui::t!("テーブルのサイズ変更 — 新しい範囲(A1:C9)").to_string(),
                "prop-creator" => ui::t!("ブックの情報 — 作成者").to_string(),
                "prop-title" => ui::t!("ブックの情報 — タイトル").to_string(),
                "prop-keywords" => ui::t!("ブックの情報 — タグ").to_string(),
                "prop-subject" => ui::t!("ブックの情報 — 件名").to_string(),
                "prop-desc" => ui::t!("ブックの情報 — コメント").to_string(),
                "textart" => ui::t!("テキストアート — 飾り文字にする文字を打つ").to_string(),
                "pw-open" => ui::t!("暗号化されたブック — パスワード").to_string(),
                "pw-set" => ui::t!("暗号化 — パスワード(空にして Enter で暗号化をやめる)").to_string(),
                "subtotal-by" => ui::t!("小計 1/2 — 何の区切りで集めるか(見出しを1つ)").to_string(),
                "subtotal-vals" => ui::t!("小計 2/2 — 合計する見出し").to_string(),
                "pivot-rows" => ui::t!("ピボット 1/3 — 行に並べる見出し(カンマ区切り可)").to_string(),
                "pivot-cols" => ui::t!("ピボット 2/3 — 列に広げる見出し(空 Enter = なし)").to_string(),
                "pivot-val" => ui::t!("ピボット 3/3 — 値にする見出しと集計").to_string(),
                _ => String::new(),
            };
            // キャレットは | で見せる(writer の検索欄と同じ割り切り)。
            // パスワードは伏せ字
            let mut text = if matches!(*kind, "pw-open" | "pw-set") {
                "●".repeat(ed.text().chars().count())
            } else {
                ed.text().to_string()
            };
            let cur = ed.cursor().min(text.len());
            text.insert(cur, '|');
            // 板は表の中央に出す(発注者 2026-08-06「表示位置を見直す」)。
            // 外側の受け皿は聞き手を持たない = 後ろのセルの操作を遮らない
            div().absolute().inset_0().flex().items_center().justify_center()
                .child(div().w(px(380.0))
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
                        "chat" => "生放送ではありません — ファイル越しの言伝。最近の言伝は下の状態行に",
                        "equation" => "例: \\frac{a}{b} / \\sqrt{x^2+1} / \\sum_{i=1}^n i^2 / \\int_0^1 x\\,dx(計算はしません — セルの式とは別物)",
                        "textart" => "太字+縁取り(calc の緑)で描いて、画像としてシートに浮かべます",
                        "ai-table" => "答えのタブ区切りを、カーソルの位置の空きに流し込みます",
                        "ai-ask" => "= で始まる答えはカーソルに式として入ります。他はコメントに付きます",
                        "pw-open" => "間違えると開けません(板は残ります)。Esc で開くのをやめる",
                        "pw-set" => "次の保存から AES-128 で包みます。Excel や LibreOffice でも開けます",
                        "subtotal-by" => "使える見出しは下の状態行に出ています。並べ替えてから使うと区切りがまとまります",
                        "subtotal-vals" => "空のまま Enter = 数の列全部に入れます。畳んでも小計と総計は残ります",
                        "pivot-rows" | "pivot-cols" => "使える見出しは下の状態行に出ています。Enter で次へ / Esc で取消",
                        "pivot-val" => "例: 金額 合計。集計は 合計/平均/個数/最大/最小(省けば合計)",
                        _ => "Enter で決定 / Esc で取消",
                    })))
        });

        // ---- ソルバーの小窓(ONLYOFFICE の「ソルバーのパラメータ」の形) ----
        // モーダルにしない板たちと同じ作法。打鍵は focus の欄へ(HasEditor)
        let solver_panel = self.solver.as_ref().map(|sv| {
            let show = |ed: &Editor, on: bool| -> String {
                let mut t = ed.text().to_string();
                if on {
                    let cur = ed.cursor().min(t.len());
                    t.insert(cur, '|');
                }
                if t.is_empty() { t = " ".into() }
                t
            };
            let (focus, mode, nonneg, sel) = (sv.focus, sv.mode, sv.nonneg, sv.sel);
            let field = |id: &'static str, f: u8, text: String, cx: &mut Context<Self>| {
                div().id(id).flex_1().px_2().py_1().bg(rgb(0xFFFFFF))
                    .border_1().rounded_sm()
                    .border_color(if focus == f { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                    .text_size(px(12.5)).font_family("Noto Sans JP")
                    .whitespace_nowrap().overflow_hidden()
                    .child(SharedString::from(text))
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(sv) = &mut this.solver {
                            sv.focus = f;
                        }
                        cx.notify();
                    }))
            };
            let label = |t: &'static str| {
                div().mt_1p5().text_size(px(11.5)).text_color(rgb(0x444B52)).child(t)
            };
            let btn = |id: &'static str, t: &'static str, on: bool| {
                div().id(id).px_2p5().py_1().rounded_sm().border_1()
                    .border_color(if on { rgb(0xC6CDD3) } else { rgb(0xEDEFF1) })
                    .text_size(px(11.5))
                    .text_color(if on { rgb(0x1B1B1B) } else { rgb(0xB6BDC4) })
                    .when(on, |d| d.cursor_pointer().hover(|s| s.bg(rgb(0xEAF5EE))))
            };
            let radio = |id: &'static str, m: u8, t: &'static str, cx: &mut Context<Self>| {
                div().id(id).flex().flex_row().items_center().gap_1()
                    .cursor_pointer().text_size(px(12.0))
                    .child(if mode == m { "◉" } else { "○" })
                    .child(t)
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(sv) = &mut this.solver {
                            sv.mode = m;
                            if m == 2 {
                                sv.focus = 1;
                            }
                        }
                        cx.notify();
                    }))
            };
            // 制約の一覧
            let mut list = div().mt_1().p_1().h(px(96.0)).bg(rgb(0xFAFBFC))
                .border_1().border_color(rgb(0xC6CDD3)).rounded_sm()
                .flex().flex_col().overflow_hidden();
            if sv.cons.is_empty() {
                list = list.child(div().flex_1().flex().items_center().justify_center()
                    .text_size(px(11.5)).text_color(rgb(0xB6BDC4))
                    .child(ui::t!("まだ制約はありません。左辺・記号・右辺を打って「追加」")));
            } else {
                for (i, (l, op, r)) in sv.cons.iter().enumerate() {
                    let on = sel == Some(i);
                    list = list.child(div()
                        .id(SharedString::from(format!("con{i}")))
                        .px_2().py_0p5().rounded_sm().text_size(px(12.0))
                        .bg(if on { rgb(0xEAF5EE) } else { rgb(0xFAFBFC) })
                        .cursor_pointer()
                        .child(SharedString::from(format!("{l} {op} {r}")))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(sv) = &mut this.solver {
                                sv.sel = Some(i);
                                let (l, op, r) = sv.cons[i].clone();
                                sv.con_l = Editor::new(&l);
                                sv.con_op =
                                    SOLVER_OPS.iter().position(|o| *o == op).unwrap_or(0);
                                sv.con_r = Editor::new(&r);
                            }
                            cx.notify();
                        })));
                }
            }
            // ソルバーも表の中央(prompt の板と同じ作法)
            div().absolute().inset_0().flex().items_center().justify_center()
                .child(div().w(px(470.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0x1B6E3C)).shadow_lg()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .flex().flex_col().gap_1()
                .child(div().flex().flex_row().items_center()
                    .child(div().text_size(px(13.0)).font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(0x1B6E3C)).child(ui::t!("ソルバーのパラメータ")))
                    .child(div().flex_1())
                    .child(div().id("sv-x").px_2().cursor_pointer().text_size(px(13.0))
                        .text_color(rgb(0x66707A)).hover(|s| s.text_color(rgb(0xC0392B)))
                        .child("✕")
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.solver = None;
                            cx.notify();
                        }))))
                .child(label("目的を設定"))
                .child(div().flex().flex_row()
                    .child(field("sv-target", 0, show(&sv.target, focus == 0), cx)))
                .child(div().mt_1().flex().flex_row().items_center().gap_3()
                    .child(radio("sv-max", 0, "最大", cx))
                    .child(radio("sv-min", 1, "最小", cx))
                    .child(radio("sv-val", 2, "値:", cx))
                    .child(field("sv-value", 1, show(&sv.value, focus == 1), cx)))
                .child(label("変数セルを変更して"))
                .child(div().flex().flex_row()
                    .child(field("sv-vars", 2, show(&sv.vars, focus == 2), cx)))
                .child(label("制約条件付き(左辺セル / 記号 / 右辺の数かセル)"))
                .child(div().flex().flex_row().items_center().gap_1()
                    .child(field("sv-conl", 3, show(&sv.con_l, focus == 3), cx))
                    .child(div().id("sv-op").px_2().py_1().rounded_sm().border_1()
                        .border_color(rgb(0xC6CDD3)).text_size(px(12.0))
                        .cursor_pointer().hover(|s| s.bg(rgb(0xEAF5EE)))
                        .child(SOLVER_OPS[sv.con_op])
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(sv) = &mut this.solver {
                                sv.con_op = (sv.con_op + 1) % 3;
                            }
                            cx.notify();
                        })))
                    .child(field("sv-conr", 4, show(&sv.con_r, focus == 4), cx)))
                .child(div().mt_1().flex().flex_row().gap_1()
                    .child(btn("sv-add", "追加", true).child(ui::t!("追加"))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(sv) = &mut this.solver {
                                let (l, r) =
                                    (sv.con_l.text().trim().to_string(),
                                     sv.con_r.text().trim().to_string());
                                if l.is_empty() || r.is_empty() {
                                    this.status =
                                        ui::t!("制約の左辺と右辺を先に打ってください").into();
                                } else {
                                    sv.cons.push((l, SOLVER_OPS[sv.con_op], r));
                                    sv.con_l = Editor::new("");
                                    sv.con_r = Editor::new("");
                                    sv.sel = None;
                                }
                            }
                            cx.notify();
                        })))
                    .child(btn("sv-edit", "変更", sel.is_some()).child(ui::t!("変更"))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(sv) = &mut this.solver {
                                if let Some(i) = sv.sel {
                                    let (l, r) =
                                        (sv.con_l.text().trim().to_string(),
                                         sv.con_r.text().trim().to_string());
                                    if !l.is_empty() && !r.is_empty() && i < sv.cons.len() {
                                        sv.cons[i] = (l, SOLVER_OPS[sv.con_op], r);
                                    }
                                }
                            }
                            cx.notify();
                        })))
                    .child(div().flex_1())
                    .child(btn("sv-del", "削除", sel.is_some()).child(ui::t!("削除"))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(sv) = &mut this.solver {
                                if let Some(i) = sv.sel.take() {
                                    if i < sv.cons.len() {
                                        sv.cons.remove(i);
                                    }
                                }
                            }
                            cx.notify();
                        }))))
                .child(list)
                .child(div().id("sv-nonneg").mt_1().flex().flex_row().items_center().gap_1()
                    .cursor_pointer().text_size(px(12.0))
                    .child(if nonneg { "☑" } else { "☐" })
                    .child(ui::t!("制約のない変数を非負にする"))
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(sv) = &mut this.solver {
                            sv.nonneg = !sv.nonneg;
                        }
                        cx.notify();
                    })))
                .child(div().mt_1().flex().flex_row().items_center().gap_2()
                    .child(div().text_size(px(12.0)).font_weight(gpui::FontWeight::BOLD)
                        .child(ui::t!("解法の方法")))
                    .child(div().px_2().py_0p5().border_1().border_color(rgb(0xC6CDD3))
                        .rounded_sm().text_size(px(11.5)).child(ui::t!("単体法 LP"))))
                .child(div().text_size(px(10.5)).text_color(rgb(0x66707A))
                    .child(ui::t!("線形の問題を LP シンプレックスで解きます(裏方 scipy)。非線形はまだ解けません — そのときは断ります")))
                .child(div().mt_1p5().flex().flex_row().gap_1()
                    .child(btn("sv-reset", "すべてリセット", true).child(ui::t!("すべてリセット"))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            let init = this.cursor.a1();
                            this.solver = Some(Solver::new(&init));
                            cx.notify();
                        })))
                    .child(div().flex_1())
                    .child(div().id("sv-solve").px_3().py_1().rounded_sm()
                        .bg(rgb(0x1B6E3C)).text_color(rgb(0xFFFFFF))
                        .text_size(px(12.0)).cursor_pointer()
                        .hover(|s| s.bg(rgb(0x2E8B57)))
                        .child(ui::t!("解を求める"))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.solve_solver(cx);
                            cx.notify();
                        })))
                    .child(btn("sv-close", "閉じる", true).child(ui::t!("閉じる"))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.solver = None;
                            cx.notify();
                        })))))
        });

        // ---- ファイルの全面ページ(本家の File メニュー。タブ0で全面) ----
        let filepage = (self.tab == 0).then(|| {
            let item_bg = rgb(0xE2E6EA);
            let gray = rgb(0xB6BDC4);
            let fg = rgb(0x444B52);
            let dim = rgb(0x66707A);
            let mk = |id: &'static str, label: &'static str, ready: bool| {
                let d = div().id(id).px_4().py_1p5().text_size(px(13.0));
                if ready {
                    d.text_color(fg).cursor_pointer().hover(move |s| s.bg(item_bg))
                } else {
                    d.text_color(gray)
                }
                .child(label)
            };
            let sb = div().w(px(280.0)).bg(rgb(0xF1F3F5))
                .border_r_1().border_color(rgb(0xE1E6EA))
                .flex().flex_col().py_2()
                .child(mk("f-back", ui::t!("‹ 戻る"), true).on_click(cx.listener(|this, _, _, cx| {
                    this.tab = this.prev_tab;
                    cx.notify()
                })))
                .child(div().h(px(10.0)))
                .child(mk("f-new", ui::t!("新規作成"), true).on_click(cx.listener(|this, _, _, cx| {
                    if this.new_book() {
                        this.tab = this.prev_tab;
                    }
                    cx.notify()
                })))
                .child(mk("f-tpl", ui::t!("テンプレートから作成"), false))
                .child(mk("f-open", ui::t!("開く"), true).on_click(cx.listener(|this, _, _, cx| {
                    this.tab = this.prev_tab;
                    this.open_dialog(cx);
                    cx.notify()
                })))
                .child({
                    let d = mk("f-recent", ui::t!("最近開いた"), true).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.file_view = 1;
                            cx.notify()
                        }));
                    if self.file_view == 1 { d.bg(item_bg) } else { d }
                })
                .child(div().h(px(10.0)))
                .child(mk("f-save", ui::t!("保存"), true).on_click(cx.listener(|this, _, _, cx| {
                    this.save(false, cx);
                    cx.notify()
                })))
                .child(mk("f-saveas", ui::t!("名前を付けて保存"), true).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.save_as(cx);
                        cx.notify()
                    })))
                .child(mk("f-print", ui::t!("印刷"), true).on_click(cx.listener(|this, _, _, cx| {
                    this.run_cmd("pdf", cx);
                    cx.notify()
                })))
                .child(mk("f-protect", ui::t!("保護する"), true).on_click(cx.listener(
                    |this, _, _, cx| {
                        if let Some(i) =
                            ribbon::CALC.iter().position(|t| t.name == "保護")
                        {
                            this.prev_tab = i;
                            this.tab = i;
                        }
                        cx.notify()
                    })))
                .child(div().h(px(10.0)))
                .child({
                    let d = mk("f-info", ui::t!("詳細情報"), true).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.file_view = 0;
                            cx.notify()
                        }));
                    if self.file_view == 0 { d.bg(item_bg) } else { d }
                })
                .child(mk("f-place", ui::t!("ファイルの場所を開く"), true).on_click(cx.listener(
                    |this, _, _, cx| {
                        match this.path.as_ref().and_then(|p| p.parent()) {
                            Some(dir) => {
                                let _ = std::process::Command::new("xdg-open")
                                    .arg(dir)
                                    .spawn();
                            }
                            None => {
                                this.status = ui::t!("まだファイルになっていません").into();
                            }
                        }
                        cx.notify()
                    })))
                .child(div().h(px(10.0)))
                .child(mk("f-quit", ui::t!("終了"), true).on_click(cx.listener(|this, _, _, cx| {
                    this.request_quit(cx);
                    cx.notify()
                })))
                .child(div().flex_1())
                .child({
                    let d = mk("f-opts", ui::t!("詳細設定"), true).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.file_view = 2;
                            cx.notify()
                        }));
                    if self.file_view == 2 { d.bg(item_bg) } else { d }
                })
                .child(mk("f-help", ui::t!("ヘルプ"), false))
                .child(mk("f-req", ui::t!("機能のリクエスト"), false));
            let mut pane = div().flex_1().bg(gpui::white()).p_8()
                .flex().flex_col().gap_3().text_size(px(12.5)).text_color(fg);
            if self.file_view == 2 {
                // 詳細設定 — 器は ~/.config/office/settings.toml
                // (SEKKEI「設定 — 器と言語」。環境変数が一時上書きで優先)
                let lang_now = ui::settings::get("language").unwrap_or_else(|| "ja".into());
                let row = |label: &'static str, value: String| {
                    div().flex().flex_row().items_center().gap_2()
                        .child(div().w(px(200.0)).text_color(dim).child(label))
                        .child(div().child(SharedString::from(value)))
                };
                pane = pane
                    .child(div().text_size(px(16.0))
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(ui::t!("詳細設定")))
                    .child(div().text_color(dim).child(SharedString::from(
                        ui::tf!("置き場: {}", ui::settings::path().display()))))
                    .child(div().h(px(6.0)))
                    .child(div().flex().flex_row().items_center().gap_2()
                        .child(div().w(px(200.0)).text_color(dim)
                            .child(ui::t!("言語(リボンと文言)")))
                        .child(div().id("set-lang")
                            .px_3().py_1().rounded_sm().cursor_pointer()
                            .bg(item_bg)
                            .child(SharedString::from(match lang_now.as_str() {
                                "ja" => "日本語".to_string(),
                                other => other.to_string(),
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                let cur = ui::settings::get("language")
                                    .unwrap_or_else(|| "ja".into());
                                let all = ui::languages();
                                let i = all.iter().position(|l| **l == cur).unwrap_or(0);
                                let next = all[(i + 1) % all.len()];
                                ui::settings::set("language", next);
                                this.status = ui::t!("言語を控えました(次の起動から効きます。環境変数 OFFICE_LANG があればそちらが優先)").into();
                                cx.notify()
                            }))))
                    .child(div().h(px(10.0)))
                    .child(row(ui::t!("書体(OFFICE_FONT)"),
                        std::env::var("OFFICE_FONT")
                            .unwrap_or_else(|_| ui::t!("(文書に従う)").into())))
                    .child(row(ui::t!("校正の宛先"), {
                        let ep = ui::Endpoint::default();
                        format!("{}:{} / {}", ep.host, ep.port, ep.model)
                    }))
                    .child(row(ui::t!("Python の経路"),
                        std::env::var("JO_PYTHON")
                            .unwrap_or_else(|_| ui::t!("(自動: .venv → python3)").into())))
                    .child(row(ui::t!("名前(ロック・チャット・署名)"), lock_identity()));
            } else if self.file_view == 1 {
                pane = pane.child(div().text_size(px(16.0))
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(ui::t!("最近開いた")));
                let list = Self::recent_list();
                if list.is_empty() {
                    pane = pane.child(div().text_color(dim)
                        .child(ui::t!("(まだありません。開く・保存すると残ります)")));
                }
                for (i, q) in list.into_iter().enumerate() {
                    let name = q.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let dir = q.parent()
                        .map(|d| d.to_string_lossy().to_string())
                        .unwrap_or_default();
                    pane = pane.child(div()
                        .id(SharedString::from(format!("recent-{i}")))
                        .px_2().py_1().rounded_sm().cursor_pointer()
                        .hover(move |s| s.bg(item_bg))
                        .flex().flex_row().items_center().gap_2()
                        .child(div().text_size(px(13.0)).child(SharedString::from(name)))
                        .child(div().text_size(px(11.0)).text_color(dim)
                            .child(SharedString::from(dir)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.tab = this.prev_tab;
                            this.open(q.clone());
                            cx.notify()
                        })));
                }
            } else {
                // 統計(生きた値)とブックの情報(docProps/core.xml から)
                let sheets_n = self.book.sheets.len();
                let mut cells_n = 0usize;
                let mut formulas_n = 0usize;
                for sh in &self.book.sheets {
                    cells_n += sh.cells.len();
                    formulas_n +=
                        sh.cells.values().filter(|c| c.formula.is_some()).count();
                }
                let shapes_n: usize = self
                    .book
                    .sheets
                    .iter()
                    .map(|s| {
                        s.shapes.len() + s.shapes_new.len() + s.images.len()
                            + s.images_new.len()
                    })
                    .sum();
                pane = pane.child(div().text_size(px(16.0))
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(ui::t!("ブックの情報")))
                    .child(div().text_size(px(13.5))
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(ui::t!("統計")));
                for (k, v) in [
                    ("シート", sheets_n),
                    ("使っているセル", cells_n),
                    ("式のセル", formulas_n),
                    ("図形と画像", shapes_n),
                ] {
                    pane = pane.child(div().flex().flex_row()
                        .child(div().w(px(220.0)).text_color(dim).child(k))
                        .child(SharedString::from(format!("{v}"))));
                }
                pane = pane.child(div().h(px(6.0)))
                    .child(div().text_size(px(13.5))
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(ui::t!("プロパティ")));
                let pr = &self.book.props;
                for (k, v, kind) in [
                    ("作成者", pr.creator.clone(), "prop-creator"),
                    ("タイトル", pr.title.clone(), "prop-title"),
                    ("タグ", pr.keywords.clone(), "prop-keywords"),
                    ("件名", pr.subject.clone(), "prop-subject"),
                    ("コメント", pr.description.clone(), "prop-desc"),
                ] {
                    let empty = v.is_empty();
                    let init = v.clone();
                    pane = pane.child(div().flex().flex_row().items_center()
                        .child(div().w(px(220.0)).text_color(dim).child(k))
                        .child(div()
                            .id(SharedString::from(kind))
                            .w(px(320.0)).px_2().py_1().rounded_sm()
                            .border_1().border_color(rgb(0xE1E6EA))
                            .cursor_pointer()
                            .hover(move |s| s.bg(item_bg))
                            .whitespace_nowrap().overflow_hidden()
                            .text_color(if empty { gray } else { fg })
                            .child(SharedString::from(if empty {
                                ui::t!("テキストの追加").to_string()
                            } else {
                                v
                            }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.prompt = Some((kind, Editor::new(&init)));
                                cx.notify()
                            }))));
                }
                pane = pane.child(div().text_size(px(11.5)).text_color(dim)
                    .child(ui::t!("欄を押して打ち、Enter で控える(保存で xlsx の情報に入ります)")));
            }
            div().absolute().inset_0().bg(gpui::white())
                .flex().flex_row()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(sb)
                .child(pane)
        });

        // ---- スライサーの小窓(列の値の釦で絞る) ----
        let slicer_panel = self.slicer.as_ref().map(|(col, sel, multi)| {
            let col = *col;
            let multi = *multi;
            // 見出し(1行目)と、その下の一意な値。空欄は「(空白)」で最後に
            let head = self
                .sheet()
                .get(Pos::new(0, col))
                .map(|c| c.value.display())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| ui::tf!("列{}", col_name(col)));
            let (rows, _) = self.sheet().extent();
            let mut vals: std::collections::BTreeSet<String> = Default::default();
            let mut has_blank = false;
            for r in 1..rows {
                let v = self
                    .sheet()
                    .get(Pos::new(r, col))
                    .map(|c| c.value.display())
                    .unwrap_or_default();
                if v.is_empty() {
                    has_blank = true;
                } else {
                    vals.insert(v);
                }
            }
            let mut items: Vec<String> = vals.into_iter().take(64).collect();
            if has_blank {
                items.push(ui::t!("(空白)").to_string());
            }
            let mut p = div().absolute().right(px(24.0)).top(px(ROW_H + 16.0)).w(px(190.0))
                .p_2().rounded_md().bg(gpui::white())
                .border_1().border_color(rgb(0x1B6E3C)).shadow_lg()
                .flex().flex_col().gap_1()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(div().flex().flex_row().items_center()
                    .child(div().text_size(px(12.5)).font_weight(gpui::FontWeight::BOLD)
                        .whitespace_nowrap().overflow_hidden()
                        .child(SharedString::from(head)))
                    .child(div().flex_1())
                    // ≡ = 複数選択の入切(本家のスライサーと同じ並び)
                    .child(div().id("sl-multi").px_1p5().rounded_sm().cursor_pointer()
                        .text_size(px(12.5))
                        .bg(if multi { rgb(0xCFE6D8) } else { rgb(0xFFFFFF) })
                        .hover(|s| s.bg(rgb(0xEAF5EE)))
                        .child("≡")
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some((_, _, m)) = &mut this.slicer {
                                *m = !*m;
                                this.status = if *m {
                                    ui::t!("複数選択: 押した値を重ねて絞ります").into()
                                } else {
                                    ui::t!("単数選択: 押した値ひとつで絞ります").into()
                                };
                            }
                            cx.notify();
                        })))
                    // ✕ = 選びを解除(全部見せる)
                    .child(div().id("sl-clear").px_1p5().rounded_sm().cursor_pointer()
                        .text_size(px(12.5)).hover(|s| s.bg(rgb(0xEAF5EE)))
                        .child("✕")
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some((_, sel, _)) = &mut this.slicer {
                                sel.clear();
                            }
                            this.status = ui::t!("スライサーの絞りを解除しました").into();
                            cx.notify();
                        }))));
            for (i, v) in items.into_iter().enumerate() {
                let on = sel.contains(&v);
                p = p.child(div()
                    .id(SharedString::from(format!("sl{i}")))
                    .px_2().py_1().rounded_sm().border_1()
                    .border_color(rgb(0xC6CDD3))
                    .bg(if on { rgb(0xBBD9EA) } else { rgb(0xFFFFFF) })
                    .text_size(px(12.0)).cursor_pointer()
                    .whitespace_nowrap().overflow_hidden()
                    .hover(|s| s.bg(rgb(0xEAF5EE)))
                    .child(SharedString::from(v.clone()))
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some((_, sel, multi)) = &mut this.slicer {
                            if *multi {
                                if !sel.remove(&v) {
                                    sel.insert(v.clone());
                                }
                            } else if sel.len() == 1 && sel.contains(&v) {
                                sel.clear(); // 同じ釦をもう一度 = 解除
                            } else {
                                sel.clear();
                                sel.insert(v.clone());
                            }
                            this.status = if sel.is_empty() {
                                ui::t!("絞りなし(全部見えています)").into()
                            } else {
                                ui::tf!("絞り: {}(見え方だけ。中身は変わりません)", sel.iter().cloned().collect::<Vec<_>>().join(" / "))
                                .into()
                            };
                        }
                        cx.notify();
                    })));
            }
            p
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
                        .child(ui::t!("セルの書式(選んでいる範囲に効く)")))
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
                    .child(btn("b-all", ui::t!("格子")))
                    .child(btn("b-out", ui::t!("外枠")))
                    .child(btn("b-none", ui::t!("なし"))))
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
                    .child(btn("bold", ui::t!("太字")))
                    .child(btn("italic", ui::t!("斜体")))
                    .child(btn("underline", ui::t!("下線")))
                    .child(btn("strikeout", ui::t!("取り消し")))
                    .child(btn("incfont", ui::t!("大きく")))
                    .child(btn("decfont", ui::t!("小さく"))))
                .child(title("揃え"))
                .child(row()
                    .child(btn("align-left", ui::t!("左")))
                    .child(btn("align-center", ui::t!("中央")))
                    .child(btn("align-right", ui::t!("右")))
                    .child(btn("top", ui::t!("上")))
                    .child(btn("middle", ui::t!("中")))
                    .child(btn("bottom", ui::t!("下")))
                    .child(btn("wrap", ui::t!("折り返し"))))
                .child(title("表示形式"))
                .child(row()
                    .child(btn("comma", "1,000"))
                    .child(btn("currency", "¥"))
                    .child(btn("percents", "%"))
                    .child(btn("digit-inc", ".0+"))
                    .child(btn("digit-dec", ".0−"))
                    .child(btn("numfmt-none", ui::t!("なし"))))
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
                            this.apply_pick(&v, cx);
                            cx.notify();
                        })));
            }
            p
        });

        let notes = if self.notes.is_empty() { None } else {
            let mut n = div().px_4().py_2().bg(rgb(0xFFF6E6))
                .border_t_1().border_color(rgb(0xE8D5A8))
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                       .text_color(rgb(0x8A4B00)).child(ui::t!("この版で読み飛ばしたもの")));
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
            .on_action(cx.listener(Calc::a_edit_cell))
            .child(bar)
            .children((self.tab != 0 && self.show_formula_bar).then(|| formula_bar))
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
                   .children(ink_preview)
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
                           let (x, y) = (x + sp.dx_px, y + sp.dy_px);
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
                   .children(filepage)
                   .children(pick_panel)
                   .children(prompt_panel)
                   .children(solver_panel)
                   .children(fn_panel)
                   .children(fn_args_panel)
                   .children(quit_panel)
                   .children(slicer_panel))
            .children(watch_bar)
            .child(sheets_bar)
            .children(notes)
            // 窓の縁のつかみ(最後に描く = 最初にマウスを受ける)
            .children(ui::resize_edges(window))
    }
}

fn main() {
    let arg = std::env::args().nth(1).map(PathBuf::from);
    application().with_assets(ui::Icons).run(move |cx: &mut App| {
        cx.text_system()
            .add_fonts(vec![std::borrow::Cow::Borrowed(font_data())])
            .expect("フォント登録");
        cx.bind_keys(ui::bindings("jo_edit"));
        // 前に閉じたときの姿で開く。控えが無ければ既定の大きさで中央に
        let saved = ui::winstate::load("calc");
        let bounds = match saved {
            Some(st) => Bounds::new(gpui::point(px(st.x), px(st.y)), size(px(st.w), px(st.h))),
            None => Bounds::centered(None, size(px(1060.0), px(820.0)), cx),
        };
        let wb = if saved.is_some_and(|st| st.maximized) {
            WindowBounds::Maximized(bounds)
        } else {
            WindowBounds::Windowed(bounds)
        };
        let arg2 = arg.clone();
        cx.open_window(
            WindowOptions {
                window_bounds: Some(wb),
                ..Default::default()
            },
            move |window, cx| {
                let view = cx.new(|cx| Calc::new(arg2.clone(), cx));
                window.focus(&view.focus_handle(cx), cx);
                // 動かす・伸ばすたびに控える — 閉じる経路が何本あっても漏れない。
                // 全画面は控えない(次も全画面で開くと出口が分かりにくい)
                view.update(cx, |_, cx| {
                    cx.observe_window_bounds(window, |_, window, _| {
                        let wb = window.window_bounds();
                        if matches!(wb, WindowBounds::Fullscreen(_)) {
                            return;
                        }
                        let b = wb.get_bounds();
                        ui::winstate::save("calc", ui::winstate::WinState {
                            x: f32::from(b.origin.x),
                            y: f32::from(b.origin.y),
                            w: f32::from(b.size.width),
                            h: f32::from(b.size.height),
                            maximized: matches!(wb, WindowBounds::Maximized(_)),
                        });
                    })
                    .detach();
                });
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
                                c.status = ui::tf!("自己診断 {}/15: B列の幅 {}(勝手に動けば描画は健全)", i + 1, w)
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
mod table_design_tests {
    use super::*;

    #[test]
    fn 合計行は見出しを外して数の列だけ足す() {
        let mut s = sheet::Sheet { name: "表".into(), ..Default::default() };
        s.set(Pos::new(0, 0), Cell::input("品名"));
        s.set(Pos::new(0, 1), Cell::input("金額"));
        s.set(Pos::new(1, 0), Cell::input("甲"));
        s.set(Pos::new(1, 1), Cell::input("100"));
        s.set(Pos::new(2, 0), Cell::input("乙"));
        s.set(Pos::new(2, 1), Cell::input("50"));
        add_total_row(&mut s, Pos::new(0, 0), Pos::new(2, 1));
        recalc(&mut s);
        let label = s.get(Pos::new(3, 0)).unwrap();
        assert_eq!(label.value.display(), "合計", "文字の列の先頭は札");
        assert!(label.fmt.bold && label.fmt.borders.top, "合計行の書式が付かない");
        let sum = s.get(Pos::new(3, 1)).unwrap();
        assert_eq!(
            sum.formula.as_deref(),
            Some("SUM(B2:B3)"),
            "見出しが合計に混ざった: {:?}",
            sum.formula
        );
        assert_eq!(sum.value.display(), "150");
    }

    #[test]
    fn 見出しの無い表は全行を合計する() {
        let mut s = sheet::Sheet { name: "表".into(), ..Default::default() };
        for (r, v) in [(0, "10"), (1, "20")] {
            s.set(Pos::new(r, 0), Cell::input(v));
        }
        add_total_row(&mut s, Pos::new(0, 0), Pos::new(1, 0));
        recalc(&mut s);
        let sum = s.get(Pos::new(2, 0)).unwrap();
        assert_eq!(sum.formula.as_deref(), Some("SUM(A1:A2)"));
        assert_eq!(sum.value.display(), "30");
    }
}

#[cfg(test)]
mod subtotal_tests {
    use super::*;

    #[test]
    fn 小計と総計が入り明細だけ畳まれる() {
        let mut s = sheet::Sheet { name: "表".into(), ..Default::default() };
        for (r, row) in [
            ["部署", "月", "金額"],
            ["営業", "1月", "100"],
            ["営業", "1月", "50"],
            ["営業", "2月", "70"],
            ["総務", "1月", "30"],
        ]
        .iter()
        .enumerate()
        {
            for (c, v) in row.iter().enumerate() {
                s.set(Pos::new(r as u32, c as u32), Cell::input(v));
            }
        }
        let n = apply_subtotals(&mut s, Pos::new(0, 0), Pos::new(4, 2), 0, &[2]);
        recalc(&mut s);
        assert_eq!(n, 2, "区切りの数が違う");
        // 並び: 1見出し 2-4営業明細 5営業小計 6総務明細 7総務小計 8総計
        let d = |r: u32, c: u32| s.get(Pos::new(r, c)).map(|x| x.value.display()).unwrap_or_default();
        assert_eq!(d(4, 0), "営業 小計");
        assert_eq!(d(4, 2), "220", "営業の小計が違う");
        assert_eq!(
            s.get(Pos::new(4, 2)).and_then(|c| c.formula.clone()).as_deref(),
            Some("SUM(C2:C4)"),
            "小計が式でない"
        );
        assert_eq!(d(6, 0), "総務 小計");
        assert_eq!(d(6, 2), "30");
        assert_eq!(d(7, 0), "総計");
        assert_eq!(d(7, 2), "250", "総計が違う");
        // 明細だけグループ化(小計・総計はされない → 畳んでも残る)
        for r in [1, 2, 3, 5] {
            assert_eq!(s.row_outline.get(&r), Some(&1), "明細 {r} が畳めない");
        }
        for r in [0, 4, 6, 7] {
            assert!(!s.row_outline.contains_key(&r), "行 {r} まで畳まれてしまう");
        }
    }

    #[test]
    fn 行の挿抜でグループ化が付いてくる() {
        let mut s = sheet::Sheet { name: "表".into(), ..Default::default() };
        s.row_outline.insert(5, 1);
        s.row_hidden.insert(5);
        s.insert_row(2);
        assert_eq!(s.row_outline.get(&6), Some(&1), "挿入で深さが置き去り");
        assert!(s.row_hidden.contains(&6), "挿入で畳みが置き去り");
        s.remove_row(0);
        assert_eq!(s.row_outline.get(&5), Some(&1), "削除で深さが置き去り");
        assert!(s.row_hidden.contains(&5));
    }
}

#[cfg(test)]
mod solver_tests {
    use super::*;

    #[test]
    fn セルと範囲の列挙が読める() {
        let v = parse_cell_list("B2:B4", 64).unwrap();
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], Pos::new(1, 1));
        let v = parse_cell_list("$A$1, C3", 64).unwrap();
        assert_eq!(v, vec![Pos::new(0, 0), Pos::new(2, 2)]);
        assert!(parse_cell_list("ほげ", 64).is_none(), "読めないものは None");
        assert!(parse_cell_list("A1:Z99", 10).is_none(), "上限を超えたら None");
        assert!(parse_cell_list("", 64).is_none());
    }

    #[test]
    fn 台本が実際にscipyで回る() {
        // .venv が無い機械では黙って飛ぶ(HIKITSUGI の作法)
        let py = ["../.venv/bin/python", ".venv/bin/python"]
            .iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.exists());
        let Some(py) = py else { return };
        // max x+2y  s.t. x+y<=4, x<=2, x,y>=0 → x=0,y=4(目的8)
        let dir = std::env::temp_dir().join(format!("jo-solver-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let spec = "{\"c\":[-1,-2],\"aub\":[[1,1],[1,0]],\"bub\":[4,2],\"aeq\":[],\"beq\":[],\"nonneg\":true}";
        let json_path = dir.join("solver.json");
        let py_path = dir.join("solver.py");
        std::fs::write(&json_path, spec).unwrap();
        std::fs::write(&py_path, SOLVER_PY).unwrap();
        let o = std::process::Command::new(&py)
            .arg(&py_path)
            .arg(&json_path)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let out = String::from_utf8_lossy(&o.stdout).to_string();
        let xs: Vec<f64> =
            out.split('\u{1f}').filter_map(|v| v.trim().parse().ok()).collect();
        assert_eq!(xs.len(), 2, "答えの形が違う: {out:?}");
        assert!(xs[0].abs() < 1e-6 && (xs[1] - 4.0).abs() < 1e-6,
                "最適解が違う: {xs:?}");
    }
}

#[cfg(test)]
mod equation_tests {
    use super::*;

    #[test]
    fn 台本が実際にmathtextで清書する() {
        // .venv が無い機械では黙って飛ぶ(HIKITSUGI の作法)
        let py = ["../.venv/bin/python", ".venv/bin/python"]
            .iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.exists());
        let Some(py) = py else { return };
        let dir = std::env::temp_dir().join(format!("jo-eq-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let out = dir.join("eq.png");
        let spec = format!(
            "{{\"tex\":\"\\\\frac{{a}}{{b}}+\\\\sqrt{{x^2+1}}\",\"font\":\"\",\"out\":\"{}\"}}",
            out.to_string_lossy()
        );
        let json_path = dir.join("eq.json");
        let py_path = dir.join("eq.py");
        std::fs::write(&json_path, spec).unwrap();
        std::fs::write(&py_path, EQ_PY).unwrap();
        let o = std::process::Command::new(&py)
            .arg(&py_path)
            .arg(&json_path)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let data = std::fs::read(&out).unwrap();
        assert!(data.starts_with(&[0x89, b'P', b'N', b'G']), "PNG が出ていない");
        let (w, h) = image_px(&data).expect("大きさが読めない");
        assert!(w > 40 && h > 20, "清書が小さすぎる: {w}x{h}");
        // テキストアートも同じ道(飾り文字が PNG になる)
        let ta = format!(
            "{{\"tex\":\"見積書\",\"font\":\"\",\"out\":\"{}\"}}",
            out.to_string_lossy()
        );
        std::fs::write(&json_path, ta).unwrap();
        std::fs::write(&py_path, TEXTART_PY).unwrap();
        let o = std::process::Command::new(&py)
            .arg(&py_path)
            .arg(&json_path)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let data = std::fs::read(&out).unwrap();
        assert!(data.starts_with(&[0x89, b'P', b'N', b'G']), "テキストアートが PNG でない");
        // 読めない式は黙って白紙にせず、ちゃんと失敗する(台本を式のものに戻す)
        std::fs::write(&py_path, EQ_PY).unwrap();
        let bad = format!(
            "{{\"tex\":\"\\\\frac{{a\",\"font\":\"\",\"out\":\"{}\"}}",
            out.to_string_lossy()
        );
        std::fs::write(&json_path, bad).unwrap();
        let o = std::process::Command::new(&py)
            .arg(&py_path)
            .arg(&json_path)
            .output()
            .unwrap();
        assert!(!o.status.success(), "壊れた式が通ってしまった");
    }
}

#[cfg(test)]
mod pivot_tests {
    use super::*;

    #[test]
    fn 見出しの列挙はカンマでも読点でも空白でも() {
        assert_eq!(split_fields("部署, 月"), vec!["部署", "月"]);
        assert_eq!(split_fields("部署、月 区分"), vec!["部署", "月", "区分"]);
        assert!(split_fields("  ").is_empty());
    }

    #[test]
    fn 値と集計の読み取り() {
        let hs = vec!["部署".to_string(), "金額".to_string()];
        assert_eq!(
            parse_pivot_val("金額 合計", &hs).unwrap(),
            ("金額".to_string(), "合計")
        );
        assert_eq!(parse_pivot_val("金額", &hs).unwrap().1, "合計", "省けば合計");
        assert_eq!(parse_pivot_val("金額 平均", &hs).unwrap().1, "平均");
        assert!(parse_pivot_val("売上 合計", &hs).is_err(), "無い見出しは断る");
        assert!(parse_pivot_val("", &hs).is_err(), "空は断る");
    }

    fn def(rows: &[&str], cols: &[&str], value: &str, agg: &str) -> sheet::model::PivotDef {
        sheet::model::PivotDef {
            sheet: "S".into(),
            src: (Pos::new(0, 0), Pos::new(1, 1)),
            rows_sel: rows.iter().map(|s| s.to_string()).collect(),
            cols_sel: cols.iter().map(|s| s.to_string()).collect(),
            value: value.into(),
            agg: agg.into(),
            totals: false,
            subtotals: false,
            blank_rows: false,
            compact: false,
            dest: Pos::new(0, 0),
            size: (0, 0),
        }
    }

    #[test]
    fn 指図のjsonは逃がしが効く() {
        let json = pivot_spec_json(
            &["部\"署".to_string()],
            &[vec!["営\\業".to_string()]],
            &def(&["部\"署"], &[], "部\"署", "合計"),
        );
        assert!(json.contains("部\\\"署"), "二重引用符が逃げていない: {json}");
        assert!(json.contains("営\\\\業"), "バックスラッシュが逃げていない: {json}");
        assert!(json.contains("\"totals\":false"), "旗が無い: {json}");
    }

    fn run_py(spec: String) -> Option<(Vec<Vec<String>>, Vec<char>)> {
        // .venv が無い機械では黙って飛ぶ(HIKITSUGI の作法)
        let py = ["../.venv/bin/python", ".venv/bin/python"]
            .iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.exists())?;
        // 並走する試験と取り合わないよう、呼び出しごとに番号を振る
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("jo-pivot-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let json_path = dir.join(format!("pivot{n}.json"));
        let py_path = dir.join(format!("pivot{n}.py"));
        std::fs::write(&json_path, spec).unwrap();
        std::fs::write(&py_path, PIVOT_PY).unwrap();
        let o = std::process::Command::new(&py)
            .arg(&py_path)
            .arg(&json_path)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        Some(parse_pivot_grid(&String::from_utf8_lossy(&o.stdout)))
    }

    #[test]
    fn 台本が実際にpolarsで回る() {
        let headers: Vec<String> =
            ["部署", "月", "金額"].iter().map(|s| s.to_string()).collect();
        let rows: Vec<Vec<String>> = [
            ["営業", "1月", "100"],
            ["営業", "1月", "50"],
            ["総務", "1月", "30"],
            ["営業", "2月", "70"],
        ]
        .iter()
        .map(|r| r.iter().map(|s| s.to_string()).collect())
        .collect();
        // 部署×月の合計(クロス表)
        let spec = pivot_spec_json(&headers, &rows, &def(&["部署"], &["月"], "金額", "合計"));
        let Some((g, k)) = run_py(spec) else { return };
        assert_eq!(k[0], 'h');
        assert_eq!(g[0], vec!["部署", "1月", "2月"], "見出しの形が違う: {g:?}");
        assert_eq!(g[1], vec!["営業", "150", "70"]);
        // 無い組み合わせ: 合計は 0(空の合計)。平均などは null → 空欄になる
        assert_eq!(g[2], vec!["総務", "30", "0"]);
        // 部署ごとの個数(列に広げない)
        let spec = pivot_spec_json(&headers, &rows, &def(&["部署"], &[], "金額", "個数"));
        let Some((g, _)) = run_py(spec) else { return };
        assert_eq!(g[0], vec!["部署", "金額"]);
        assert_eq!(g[1], vec!["営業", "3"]);
        assert_eq!(g[2], vec!["総務", "1"]);
    }

    #[test]
    fn 総計と小計と空行が付く() {
        let headers: Vec<String> =
            ["部署", "係", "月", "金額"].iter().map(|s| s.to_string()).collect();
        let rows: Vec<Vec<String>> = [
            ["営業", "一", "1月", "100"],
            ["営業", "二", "1月", "50"],
            ["営業", "一", "2月", "70"],
            ["総務", "一", "1月", "30"],
        ]
        .iter()
        .map(|r| r.iter().map(|s| s.to_string()).collect())
        .collect();
        let mut d = def(&["部署", "係"], &["月"], "金額", "合計");
        d.totals = true;
        d.subtotals = true;
        d.blank_rows = true;
        let spec = pivot_spec_json(&headers, &rows, &d);
        let Some((g, k)) = run_py(spec) else { return };
        assert_eq!(g[0], vec!["部署", "係", "1月", "2月", "総計"], "見出し: {g:?}");
        assert_eq!(g[1], vec!["営業", "一", "100", "70", "170"]);
        assert_eq!(g[2], vec!["営業", "二", "50", "0", "50"]);
        assert_eq!(
            g[3],
            vec!["営業 小計", "", "150", "70", "220"],
            "小計が違う: {g:?}"
        );
        assert_eq!(k[3], 's', "小計の種別が違う");
        assert_eq!(k[4], 'b', "空行が無い");
        assert_eq!(g[6], vec!["総務 小計", "", "30", "0", "30"]);
        let last = g.last().unwrap();
        assert_eq!(last, &vec!["総計", "", "180", "70", "250"], "総計が違う: {g:?}");
        assert_eq!(*k.last().unwrap(), 't');
        // コンパクト形式: 繰り返しの見出しが空欄になる
        d.subtotals = false;
        d.blank_rows = false;
        d.totals = false;
        d.compact = true;
        let spec = pivot_spec_json(&headers, &rows, &d);
        let Some((g, _)) = run_py(spec) else { return };
        assert_eq!(g[2][0], "", "繰り返しの部署が空欄にならない: {g:?}");
        assert_eq!(g[2][1], "二");
    }
}

/// **メニューの釦を全部おして、落ちないか・繋がっているかを見る。**
/// writer の menu_run_tests と同じ作法 — リボンに ready で並ぶものは
/// ここで実際に run_cmd を通す(ダイアログを開くものだけは外す)。
/// GUI は起こさない — gpui の試験用の場で Calc を作って叩く
#[cfg(test)]
mod menu_run_tests {
    use super::*;

    /// ファイル選択の窓を開く釦。**試験では押さない** —
    /// rfd は実際に窓を出しに行くので、画面の無い試験では返ってこない
    /// (writer で踏んで確かめた轍。実機での確認に回す)
    const DIALOG: &[&str] = &[
        "open", "save", "pdf", "plug-macros", "insimage", "data-from-text",
        "data-external-links",
    ];

    /// 空の表だと何も起きない釦があるので、見本の小さな表を入れて選ぶ
    fn seed(this: &mut Calc) {
        if this.sheet().cells.is_empty() {
            for (a1, v) in [
                ("A1", "品名"), ("B1", "数量"), ("C1", "単価"),
                ("A2", "防火戸"), ("B2", "4"), ("C2", "125000"),
                ("A3", "点検口"), ("B3", "2"), ("C3", "8000"),
                ("D2", "=B2*C2"), ("D3", "=B3*C3"),
            ] {
                this.sheet_mut().set(Pos::parse(a1).unwrap(), Cell::input(v));
            }
            recalc(this.sheet_mut());
        }
        this.cursor = Pos::parse("A1").unwrap();
        this.anchor = Some(Pos::parse("D3").unwrap());
        // バーとセルを揃える(実機ではカーソル移動が必ず呼ぶ。ずれたままだと
        // 最初の commit() が A1 を空で潰し、種の表が崩れる)
        this.sync_input();
    }

    #[gpui::test]
    fn 全部の釦が落ちずに通る(cx: &mut gpui::TestAppContext) {
        // AI の宛先は覚える設定なので、試験で変えたら戻す
        let keep_ai = ui::ai::backend();
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        for tab in ui::ribbon::CALC {
            for cmd in tab.cmds {
                if !cmd.ready || DIALOG.contains(&cmd.id) {
                    continue;
                }
                let (id, label) = (cmd.id, cmd.label);
                c.update(cx, |this, cx| {
                    seed(this);
                    this.run_cmd(id, cx);
                    let st = this.status.to_string();
                    assert!(
                        !st.contains("未配線"),
                        "「{label}」({id}) が未配線: {st}"
                    );
                });
            }
        }
        ui::ai::set_backend(keep_ai);
    }

    /// リボンの「すべて選択」は**セル**に効く(バーの文字選択に化けない —
    /// Ctrl+A と同じ実体を通ることの検査。2026-08-05 に別実装のサボりを直した)
    #[gpui::test]
    fn 全選択はセルに効く(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            seed(this);
            this.anchor = None;
            this.sync_input(); // 実機ではカーソル移動が必ず呼ぶ
            this.run_cmd("selectall", cx);
            let (rows, cols) = this.sheet().extent();
            assert_eq!(this.anchor, Some(Pos::parse("A1").unwrap()), "起点が A1 でない");
            assert_eq!(
                this.cursor,
                Pos::new(rows - 1, cols - 1),
                "使われている範囲の端まで選べていない"
            );
        });
    }

    /// 押すと入切する釦は、2回押すと元に戻る(1手で戻せる家訓)
    #[gpui::test]
    fn 入切の釦は二度おすと戻る(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        let state = |this: &Calc, id: &str| -> bool {
            match id {
                "show-formulas" => this.show_formulas,
                "show-gridlines" => this.gridlines,
                "co-showcomment" => this.show_comments,
                "formula-bar" => this.show_formula_bar,
                "show-headings" => this.show_headers,
                "show-zeros" => this.show_zeros,
                "freeze" => this.frozen.is_some(),
                "rtl-sheet" => this.sheet().rtl,
                _ => unreachable!(),
            }
        };
        for id in [
            "show-formulas", "show-gridlines", "co-showcomment", "formula-bar",
            "show-headings", "show-zeros", "freeze", "rtl-sheet",
        ] {
            c.update(cx, |this, cx| {
                seed(this);
                // freeze は A1 では効かない仕様(固定する位置が要る)
                this.cursor = Pos::parse("B2").unwrap();
                this.anchor = None;
                let before = state(this, id);
                this.run_cmd(id, cx);
                assert_ne!(before, state(this, id), "「{id}」を押しても変わらない");
                this.run_cmd(id, cx);
                assert_eq!(before, state(this, id), "「{id}」が元に戻らない");
            });
        }
    }

    /// **見本のブックを開いた状態でも**全部の釦が通る。
    /// 空のブックと違い、式・結合・列幅・条件付き書式が入っているので
    /// 「前提があるときの道」も通る(sample/*.xlsx が検査の材料)。
    /// 見本は写しを開く — 署名やチャットが隣にファイルを添えるため、
    /// 追跡している見本の隣を汚さない
    #[gpui::test]
    fn 見本を開いても全部の釦が通る(cx: &mut gpui::TestAppContext) {
        let dir = std::path::Path::new("../sample");
        let dir = if dir.exists() {
            dir.to_path_buf()
        } else {
            std::path::Path::new("sample").to_path_buf()
        };
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return; // 見本が無い環境では黙って飛ばす(失敗にはしない)
        };
        let mut files: Vec<std::path::PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("xlsx"))
            .collect();
        files.sort();
        assert!(!files.is_empty(), "見本が無い: {}", dir.display());
        let work = std::env::temp_dir().join(format!("jo-menu-{}", std::process::id()));
        std::fs::create_dir_all(&work).unwrap();
        let keep_ai = ui::ai::backend();
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        for f in files {
            let copy = work.join(f.file_name().unwrap());
            std::fs::copy(&f, &copy).unwrap();
            c.update(cx, |this, _| this.open(copy.clone()));
            for tab in ui::ribbon::CALC {
                for cmd in tab.cmds {
                    if !cmd.ready || DIALOG.contains(&cmd.id) {
                        continue;
                    }
                    let (id, label) = (cmd.id, cmd.label);
                    let name = f.file_name().unwrap().to_string_lossy().to_string();
                    c.update(cx, |this, cx| {
                        this.run_cmd(id, cx);
                        let st = this.status.to_string();
                        assert!(
                            !st.contains("未配線"),
                            "{name} で「{label}」({id}) が未配線: {st}"
                        );
                    });
                }
            }
            c.update(cx, |this, _| this.release_lock());
        }
        ui::ai::set_backend(keep_ai);
        let _ = std::fs::remove_dir_all(&work);
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

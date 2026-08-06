//! main.rs からの純移動(2026-08-06 の分割)。挙動は変えない。

use crate::*;

/// 本文のフォント。**同梱せず、システムから探す**
/// (埋め込むと実行ファイルがフォントを配ることになり、免許の表示義務も付く)。
///
/// 起動時に一度だけ読み、以後は借りて使う。
/// 見つからなければ**その場で止める** — 日本語が豆腐になった画面を
/// 「動いている」と見せない。
pub(crate) fn font_data() -> &'static [u8] {
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

pub(crate) const ROW_H: f32 = 24.0;
/// `RRGGBB` を色にする。読めなければ黒
/// 下地に選択の緑を混ぜる。**塗りを置き換えない** — 選択中も帳票本来の色が
/// 透けて見える(選択を解かないと色が確かめられない、を避ける)。
pub(crate) fn tint(base: gpui::Rgba, k: f32) -> gpui::Rgba {
    let accent = (0x1B as f32 / 255.0, 0x6E as f32 / 255.0, 0x3C as f32 / 255.0);
    gpui::Rgba {
        r: base.r * (1.0 - k) + accent.0 * k,
        g: base.g * (1.0 - k) + accent.1 * k,
        b: base.b * (1.0 - k) + accent.2 * k,
        a: 1.0,
    }
}

pub(crate) fn hex(s: &str) -> gpui::Rgba {
    let g = |i: usize| {
        s.get(i * 2..i * 2 + 2)
            .and_then(|h| u8::from_str_radix(h, 16).ok())
            .map(|v| v as f32 / 255.0)
            .unwrap_or(0.0)
    };
    gpui::Rgba { r: g(0), g: g(1), b: g(2), a: 1.0 }
}

pub(crate) const COL_W: f32 = 108.0;
/// xlsx の列幅1(=「0」1個ぶん)を何画素にするか。既定幅 8.43 ≒ 108px の比
pub(crate) const PX_PER_CHW: f32 = 108.0 / 8.43;
/// 描く行の並び。固定行は常に頭に、残りは窓から。
pub(crate) fn grid_rows(frozen: Option<Pos>, view: Pos, n: u32) -> Vec<u32> {
    let f = frozen.map(|p| p.row).unwrap_or(0);
    let mut out: Vec<u32> = (0..f.min(n)).collect();
    let start = view.row.max(f);
    while (out.len() as u32) < n {
        let next = start + out.len() as u32 - f.min(n);
        out.push(next);
    }
    out
}

pub(crate) fn grid_cols(frozen: Option<Pos>, view: Pos, n: u32) -> Vec<u32> {
    let f = frozen.map(|p| p.col).unwrap_or(0);
    let mut out: Vec<u32> = (0..f.min(n)).collect();
    let start = view.col.max(f);
    while (out.len() as u32) < n {
        let next = start + out.len() as u32 - f.min(n);
        out.push(next);
    }
    out
}

pub(crate) const HEAD_W: f32 = 46.0;
pub(crate) const ROWS: u32 = 30;
pub(crate) const COLS: u32 = 9;

/// 境界の取っ手の当たり幅(縁から前後この px 以内で掴める)。
/// 見出しのクリックに他の意味は無いので、広めに取って掴みやすくする
pub(crate) const GRIP: f32 = 5.0;

/// `start` から `sizes` の幅で並ぶ区分のうち、`pos` がどの区分の
/// 右端(下端)±GRIP に掛かるかを返す。列見出し・行見出しの境界の当たり判定。
pub(crate) fn grip_hit(sizes: &[(u32, f32)], start: f32, pos: f32) -> Option<u32> {
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
pub(crate) fn index_at(sizes: &[(u32, f32)], start: f32, pos: f32) -> Option<u32> {
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
pub(crate) struct SizeDrag {
    /// 列か(false なら行)
    pub(crate) col: bool,
    pub(crate) idx: u32,
    /// 掴んだ位置(px。列なら x、行なら y)
    pub(crate) grab: f32,
    /// 掴んだときの大きさ(px)
    pub(crate) base: f32,
    /// 動かしたか。**最初に動いた瞬間に undo の控えを取る** —
    /// 掴んだだけ(クリック)で redo の控えが消えるのを防ぐ
    pub(crate) moved: bool,
}

/// 使われていないシート名(Sheet2, Sheet3, …)。
pub(crate) fn unique_sheet_name(book: &Book) -> String {
    let mut n = book.sheets.len() + 1;
    loop {
        let name = format!("Sheet{n}");
        if !book.sheets.iter().any(|s| s.name == name) {
            return name;
        }
        n += 1;
    }
}

/// 複製のシート名(Excel の流儀: 「名前 (2)」から空きを探す)
pub(crate) fn copy_sheet_name(book: &Book, base: &str) -> String {
    let mut n = 2;
    loop {
        let name = format!("{base} ({n})");
        if !book.sheets.iter().any(|s| s.name == name) {
            return name;
        }
        n += 1;
    }
}

/// 式の文字列の外側だけで、古いシート名の参照(`古!` と `'古'!`)を
/// 新しい名前に書き換える。変えたら Some(新しい式)。
/// 名前の頭が別の語の続きのとき(例: 「合計!」の中の「計!」)は書き換えない
pub(crate) fn rename_refs_in(f: &str, old: &str, new: &str) -> Option<String> {
    let needs_quote =
        |n: &str| !n.chars().all(|c| c.is_alphanumeric() || c == '_') || n.is_empty();
    let to = if needs_quote(new) { format!("'{new}'!") } else { format!("{new}!") };
    let bare = format!("{old}!");
    let quoted = format!("'{old}'!");
    let cs: Vec<char> = f.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut changed = false;
    let mut in_str = false;
    while i < cs.len() {
        let c = cs[i];
        if c == '"' {
            in_str = !in_str;
            out.push(c);
            i += 1;
            continue;
        }
        if !in_str {
            let rest: String = cs[i..].iter().collect();
            let prev_word = i > 0 && (cs[i - 1].is_alphanumeric() || cs[i - 1] == '_');
            if rest.starts_with(&quoted) {
                out.push_str(&to);
                i += quoted.chars().count();
                changed = true;
                continue;
            }
            if !prev_word && rest.starts_with(&bare) {
                out.push_str(&to);
                i += bare.chars().count();
                changed = true;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    changed.then_some(out)
}

/// 全シートの式と名前の定義の中のシート参照を、新しい名前へ書き換える。
/// 書き換えた式の数を返す(黙って直さない — 状態行で件数を言う)
pub(crate) fn rename_sheet_refs(book: &mut Book, old: &str, new: &str) -> usize {
    let mut n = 0;
    for s in book.sheets.iter_mut() {
        let hits: Vec<(Pos, String)> = s
            .cells
            .iter()
            .filter_map(|(p, c)| {
                c.formula
                    .as_ref()
                    .and_then(|f| rename_refs_in(f, old, new))
                    .map(|nf| (*p, nf))
            })
            .collect();
        for (p, nf) in hits {
            if let Some(c) = s.cells.get_mut(&p) {
                c.formula = Some(nf);
                n += 1;
            }
        }
        for (_, r) in s.names.iter_mut() {
            if let Some(nr) = rename_refs_in(r, old, new) {
                *r = nr;
            }
        }
    }
    n
}

/// 選んだ範囲を TSV(タブ区切り・行は改行)にする。
/// 式は `=` のまま持つ — 表計算どうしの受け渡しの通り相場。
pub(crate) fn range_tsv(s: &sheet::Sheet, a: Pos, b: Pos) -> String {
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
pub(crate) fn tsv_grid(text: &str) -> Vec<Vec<String>> {
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
pub(crate) fn transpose<T: Clone + Default>(g: &[Vec<T>]) -> Vec<Vec<T>> {
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
pub(crate) fn paste_values_cells(s: &mut sheet::Sheet, at: Pos, cells: &[Vec<Option<Cell>>]) -> usize {
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
pub(crate) fn paste_values_text(s: &mut sheet::Sheet, at: Pos, grid: &[Vec<String>]) -> usize {
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

/// 「関数を挿入」の小窓(本家の FormulaDialog と同じ形 —
/// 検索 / 分類 / 一覧 / 引数と説明 / OK・キャンセル)。
/// 一覧・引数・説明は funcs.rs(本家の日本語から生成。使える関数だけ)
pub(crate) struct FnDlg {
    pub(crate) search: Editor,
    /// FN_GROUPS の添字(0 = すべて)
    pub(crate) group: usize,
    /// 絞り込み後の一覧の中の選択
    pub(crate) sel: usize,
}

/// 分類の耳。「すべて」+ funcs.rs の分類
pub(crate) const FN_GROUPS: &[&str] = &["すべて", "数学", "統計", "文字列", "論理", "日付", "検索", "財務", "情報"];

/// 「関数の引数」の画面(本家の第2段)。引数ごとの欄と説明、結果の下見
pub(crate) struct FnArgs {
    pub(crate) f: &'static funcs::FnInfo,
    /// (引数名, 省略可)
    pub(crate) names: Vec<(String, bool)>,
    pub(crate) eds: Vec<Editor>,
    pub(crate) focus: usize,
    /// 関数の結果(引数を打つたびに、表の複製で計算した下見)
    pub(crate) result: String,
    /// セルの掴みの起点。ドラッグすると「起点:いま」の範囲が欄に入る
    pub(crate) pick_from: Option<Pos>,
}

/// 引数の書き方「(数値1, [数値2], ...)」を(名前, 省略可)の列に解く
pub(crate) fn parse_fn_args(spec: &str) -> Vec<(String, bool)> {
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
pub(crate) fn fn_filtered(search: &str, group: usize) -> Vec<&'static funcs::FnInfo> {
    let q = search.trim().to_uppercase();
    funcs::FUNCS
        .iter()
        .filter(|f| group == 0 || f.group == FN_GROUPS[group])
        .filter(|f| q.is_empty() || f.name.contains(&q))
        .collect()
}

/// ソルバーの小窓(ONLYOFFICE の「ソルバーのパラメータ」と同じ形)。
/// 解法は単体法 LP だけ — 本家と同じで、非線形は正直に断る。
pub(crate) struct Solver {
    /// 目的のセル
    pub(crate) target: Editor,
    /// 0=最大 1=最小 2=値
    pub(crate) mode: u8,
    /// mode=2 の目標の値
    pub(crate) value: Editor,
    /// 変数セル(範囲・カンマ区切り)
    pub(crate) vars: Editor,
    /// 決めた制約(左辺セル/範囲, 記号, 右辺)
    pub(crate) cons: Vec<(String, &'static str, String)>,
    /// 追加・変更中の制約の入力
    pub(crate) con_l: Editor,
    pub(crate) con_op: usize,
    pub(crate) con_r: Editor,
    /// 制約のない変数を非負にする
    pub(crate) nonneg: bool,
    /// 打鍵の宛先: 0=目的 1=値 2=変数 3=制約左 4=制約右
    pub(crate) focus: u8,
    /// 一覧で選んだ制約(変更・削除の相手)
    pub(crate) sel: Option<usize>,
}

impl Solver {
    pub(crate) fn new(target: &str) -> Self {
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
    pub(crate) fn focused(&mut self) -> &mut Editor {
        match self.focus {
            0 => &mut self.target,
            1 => &mut self.value,
            2 => &mut self.vars,
            3 => &mut self.con_l,
            _ => &mut self.con_r,
        }
    }
    pub(crate) fn focused_ref(&self) -> &Editor {
        match self.focus {
            0 => &self.target,
            1 => &self.value,
            2 => &self.vars,
            3 => &self.con_l,
            _ => &self.con_r,
        }
    }
}

pub(crate) const SOLVER_OPS: [&str; 3] = ["<=", "=", ">="];

/// 「データの入力規則」の板(本家の3タブのダイアログと同じ形 —
/// 設定 / メッセージを入力 / エラー警告、OK・キャンセル)。
pub(crate) struct DvDlg {
    /// 0=設定 1=メッセージを入力 2=エラー警告
    pub(crate) tab: u8,
    /// 許可: DV_KINDS の添字(0 すべての値 / 1 整数 / 2 小数 /
    /// 3 リスト / 4 文字列の長さ)。読めない種類(日付など)を開いたら 5=そのまま
    pub(crate) kind: usize,
    /// データ: DV_OPS の添字(整数/小数/文字数のときだけ)
    pub(crate) op: usize,
    /// 空白を無視(xlsx の allowBlank)
    pub(crate) allow_blank: bool,
    /// これらの変更を同じ設定の他のすべてのセルに適用する
    pub(crate) apply_same: bool,
    /// セルの ▾ を出さない(xlsx の showDropDown="1")
    pub(crate) hide_arrow: bool,
    /// エラー警告のスタイル: 0 停止 / 1 警告 / 2 情報
    pub(crate) err_style: usize,
    /// 欄: 0=最小(値・元の値) 1=最大 2=メッセージ題 3=メッセージ本文
    /// 4=エラー題 5=エラー本文
    pub(crate) eds: [Editor; 6],
    /// 打鍵の宛先(eds の添字)
    pub(crate) focus: usize,
    /// 開いているドロップダウン: 0 なし / 1 許可 / 2 データ / 3 スタイル
    pub(crate) menu: u8,
    /// 種類が読めない既存の規則(日付・時刻・カスタム)。OK でもそのまま保つ
    pub(crate) keep: Option<sheet::model::Validation>,
    /// 開いたときの既存の規則(「同じ設定の他のセル」の比較の相手)
    pub(crate) was: Option<sheet::model::Validation>,
}

impl DvDlg {
    pub(crate) fn focused(&mut self) -> &mut Editor {
        &mut self.eds[self.focus.min(5)]
    }
    pub(crate) fn focused_ref(&self) -> &Editor {
        &self.eds[self.focus.min(5)]
    }
}

/// 許可の一覧(判定できる種類だけ。日付・時刻・カスタムは保持のみ)
pub(crate) const DV_KINDS: [&str; 5] =
    ["すべての値", "整数", "小数", "リスト", "文字列の長さ"];
/// kind の添字 → xlsx の type
pub(crate) const DV_KIND_XLSX: [&str; 5] = ["", "whole", "decimal", "list", "textLength"];
/// データ(比較)の一覧。並びは xlsx の operator と対
pub(crate) const DV_OPS: [(&str, &str); 8] = [
    ("between", "次の値の間"),
    ("notBetween", "次の値の間以外"),
    ("equal", "次の値に等しい"),
    ("notEqual", "次の値に等しくない"),
    ("greaterThan", "次の値より大きい"),
    ("lessThan", "次の値より小さい"),
    ("greaterThanOrEqual", "次の値より大きいか等しい"),
    ("lessThanOrEqual", "次の値より小さいか等しい"),
];
/// エラー警告のスタイル(xlsx の errorStyle と対)
pub(crate) const DV_STYLES: [(&str, &str); 3] =
    [("stop", "停止"), ("warning", "警告"), ("information", "情報")];

/// SmartArt の一覧。**分類・並び・名前は Euro-Office の現物**
/// (web-apps の define.js の並びと ja.json の訳)から取った。
/// 載せるのは**うちの図形(SVG 方式)で組めるものだけ** —
/// できないものを、できるように見せない。
pub(crate) const SMARTART: &[(&str, &[(&str, &str)])] = &[
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
pub(crate) fn parse_cell_list(text: &str, cap: usize) -> Option<Vec<Pos>> {
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
pub(crate) struct PivotPend {
    pub(crate) a: Pos,
    pub(crate) b: Pos,
    pub(crate) headers: Vec<String>,
    pub(crate) rows_sel: Vec<String>,
    pub(crate) cols_sel: Vec<String>,
    /// 値にする見出し(集計の選択へ渡す控え)
    pub(crate) val_sel: String,
    /// 組み替えの相手(book.pivots の番号)。None = 新しく挿入
    pub(crate) replace: Option<usize>,
}

/// 見出しの列挙を割る(カンマ・読点・セミコロン・空白のどれでも。
/// ; も受けるのは日本語配列で : が ; に化けやすいため)。
pub(crate) fn split_fields(text: &str) -> Vec<String> {
    text.split(|c: char| matches!(c, ',' | '、' | ';' | '；') || c.is_whitespace())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

pub(crate) const PIVOT_AGGS: [&str; 5] = ["合計", "平均", "個数", "最大", "最小"];


/// ピボットの指図を JSON にする(手で組む — グラフと同じ割り切り)。
pub(crate) fn pivot_spec_json(headers: &[String], rows: &[Vec<String>], d: &sheet::model::PivotDef) -> String {
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
pub(crate) fn parse_pivot_grid(raw: &str) -> (Vec<Vec<String>>, Vec<char>) {
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
pub(crate) fn add_total_row(s: &mut sheet::Sheet, a: Pos, b: Pos) -> usize {
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
pub(crate) fn apply_subtotals(s: &mut sheet::Sheet, a: Pos, b: Pos, by: u32, vals: &[u32]) -> usize {
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
pub(crate) fn paste_formats(s: &mut sheet::Sheet, at: Pos, cells: &[Vec<Option<Cell>>]) -> usize {
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
pub(crate) fn paste_grid(
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

/// ゴールシークの解探索(割線法)。表の複製の上で var を動かし、
/// target が goal になる値を探す。見つからなければ None。
pub(crate) fn solve_goal(base: &sheet::Sheet, target: Pos, goal: f64, var: Pos) -> Option<f64> {
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
pub(crate) fn image_px(bytes: &[u8]) -> Option<(u32, u32)> {
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

/// 数値形式の一覧(本家のドロップダウン相当)。名前 → xlsx の書式コード。
/// None = 一般(書式なし)。会計・分数はまだ描けないので載せない(台帳に控え)
pub(crate) const NUMFMTS: &[(&str, Option<&str>)] = &[
    ("一般", None),
    ("数値 (1234.56)", Some("0.00")),
    ("桁区切り (1,234)", Some("#,##0")),
    ("通貨 (¥1,234)", Some("¥#,##0")),
    ("パーセント (12.34%)", Some("0.00%")),
    ("指数 (1.23E+04)", Some("0.00E+00")),
    ("短い日付 (2026/8/6)", Some("yyyy/m/d")),
    ("長い日付 (2026年8月6日)", Some("yyyy\"年\"m\"月\"d\"日\"")),
    ("時刻 (13:45:00)", Some("h:mm:ss")),
    ("テキスト (@)", Some("@")),
];

/// オートフィルタの控え。**隠す値**で持つ — 後から増えた値は隠れない
/// (Excel の「新しい項目は表示」に寄せた割り切り)。値は表示文字列。
/// 空セルは "" で持ち、画面では「(空白)」と見せる
pub(crate) struct AutoFilter {
    /// 張った範囲(左上=見出し行の左端, 右下)
    pub(crate) range: (Pos, Pos),
    /// 列(絶対の列番号)→ 隠す値。空の集合は持たない(=素通し)
    pub(crate) hide: std::collections::BTreeMap<u32, std::collections::BTreeSet<String>>,
}

/// 大文字小文字の変え方(本家の5択)。pick の項目名がそのまま鍵
pub(crate) const CASE_MODES: &[&str] = &[
    "文の先頭だけ大文字",
    "すべて小文字",
    "すべて大文字",
    "単語の先頭を大文字",
    "大文字と小文字を入れ替え",
];

/// 選んだ変え方で文字列を変換する(Unicode の upper/lower に従う)
pub(crate) fn change_case(t: &str, mode: &str) -> String {
    match mode {
        "すべて小文字" => t.to_lowercase(),
        "すべて大文字" => t.to_uppercase(),
        "文の先頭だけ大文字" => {
            let mut out = String::with_capacity(t.len());
            let mut done = false;
            for ch in t.to_lowercase().chars() {
                if !done && ch.is_alphabetic() {
                    out.extend(ch.to_uppercase());
                    done = true;
                } else {
                    out.push(ch);
                }
            }
            out
        }
        "単語の先頭を大文字" => {
            let mut out = String::with_capacity(t.len());
            let mut head = true;
            for ch in t.chars() {
                if ch.is_whitespace() {
                    head = true;
                    out.push(ch);
                } else if ch.is_alphabetic() {
                    if head {
                        out.extend(ch.to_uppercase());
                    } else {
                        out.extend(ch.to_lowercase());
                    }
                    head = false;
                } else {
                    // 数字や記号も語の中身(「3rd」の r は頭ではない)
                    out.push(ch);
                    head = false;
                }
            }
            out
        }
        "大文字と小文字を入れ替え" => t
            .chars()
            .flat_map(|ch| {
                if ch.is_uppercase() {
                    ch.to_lowercase().collect::<Vec<_>>()
                } else if ch.is_lowercase() {
                    ch.to_uppercase().collect()
                } else {
                    vec![ch]
                }
            })
            .collect(),
        _ => t.to_string(),
    }
}

/// セルのスタイル(本家の「セルのスタイル」。よく使う組だけ)。
/// 表オブジェクトは持たない方針どおり、掛けるのは普通の書式 —
/// どれも Ctrl+Z の1手で戻る
#[allow(clippy::type_complexity)]
/// フォントの色のパレット(本家の標準の色に寄せる。「自動」= 色なし)
pub(crate) const FONT_COLORS: &[(&str, Option<&str>)] = &[
    ("自動", None),
    ("黒", Some("1B1B1B")),
    ("赤", Some("C00000")),
    ("橙", Some("ED7D31")),
    ("黄", Some("FFC000")),
    ("緑", Some("70AD47")),
    ("青", Some("4472C4")),
    ("紺", Some("1F4E79")),
    ("紫", Some("7030A0")),
    ("灰", Some("7F7F7F")),
    ("白", Some("FFFFFF")),
];

/// 塗りつぶしのパレット(帳票で使う薄い色を先に)
pub(crate) const FILL_COLORS: &[(&str, Option<&str>)] = &[
    ("色なし", None),
    ("薄い黄", Some("FFF2CC")),
    ("薄い青", Some("DEEAF6")),
    ("薄い緑", Some("E2EFDA")),
    ("薄い橙", Some("FCE4D6")),
    ("薄い灰", Some("D9D9D9")),
    ("黄", Some("FFC000")),
    ("橙", Some("ED7D31")),
    ("緑", Some("70AD47")),
    ("青", Some("4472C4")),
    ("灰", Some("7F7F7F")),
];

pub(crate) const CELL_STYLES: &[(&str, fn(&mut CellFormat))] = &[
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

pub(crate) fn col_name(c: u32) -> String {
    Pos::new(0, c).a1().trim_end_matches('1').to_string()
}

/// xlsx の paperSize → mm と名前。**B は JIS**(ECMA-376 の表は ISO だが、
/// 日本の事務様式と日本語版の印刷ドライバの実情は JIS。ここは日本のソフト)。
pub(crate) fn paper_mm(code: u32) -> Option<(f32, f32, &'static str)> {
    Some(match code {
        8 => (297.0, 420.0, "A3"),
        9 => (210.0, 297.0, "A4"),
        11 => (148.0, 210.0, "A5"),
        12 => (257.0, 364.0, "B4"),
        13 => (182.0, 257.0, "B5"),
        _ => return None,
    })
}

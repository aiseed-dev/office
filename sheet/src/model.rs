//! 表計算のモデル — セル・シート・ブック。UI非依存。
//!
//! セルの中身は「入力されたもの」と「計算された値」を分けて持つ。
//! 式は入力の一種であり、値はその結果。xlsx も同じ持ち方をしている
//! (`<f>` が式、`<v>` が最後に計算された値)。

use std::collections::BTreeMap;

/// A1 形式のセル位置。0起点の (行, 列) で持つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pos {
    pub row: u32,
    pub col: u32,
}

impl Pos {
    pub fn new(row: u32, col: u32) -> Pos {
        Pos { row, col }
    }

    /// "B3" → Pos{row:2, col:1}。列は A..Z, AA.. の26進(1起点)。
    pub fn parse(s: &str) -> Option<Pos> {
        // 絶対参照の $ は位置に影響しないので先に全部落とす($C$5 も C5 と同じ)
        let s: String = s.trim().chars().filter(|c| *c != '$').collect();
        let split = s.find(|c: char| c.is_ascii_digit())?;
        let (col_s, row_s) = s.split_at(split);
        if col_s.is_empty() || !col_s.chars().all(|c| c.is_ascii_alphabetic()) {
            return None;
        }
        let mut col = 0u32;
        for c in col_s.chars() {
            col = col * 26 + (c.to_ascii_uppercase() as u32 - 'A' as u32 + 1);
        }
        let row: u32 = row_s.parse().ok()?;
        if row == 0 {
            return None;
        }
        Some(Pos { row: row - 1, col: col - 1 })
    }

    pub fn a1(&self) -> String {
        let mut n = self.col + 1;
        let mut s = String::new();
        while n > 0 {
            let r = ((n - 1) % 26) as u8;
            s.insert(0, (b'A' + r) as char);
            n = (n - 1) / 26;
        }
        format!("{s}{}", self.row + 1)
    }
}

/// セルの値(計算の結果)。
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Empty,
    Number(f64),
    Text(String),
    Bool(bool),
    /// #DIV/0! のようなエラー。文字列で持つ(表計算の作法)
    Error(String),
}

impl Value {
    pub fn as_number(&self) -> f64 {
        match self {
            Value::Number(n) => *n,
            Value::Bool(b) => *b as i32 as f64,
            // 表計算の慣習: 文字列は数値として0。ただし数字だけの文字列は読む
            Value::Text(s) => s.trim().parse().unwrap_or(0.0),
            _ => 0.0,
        }
    }
    pub fn display(&self) -> String {
        match self {
            Value::Empty => String::new(),
            Value::Number(n) => {
                if (n.fract()).abs() < 1e-10 && n.abs() < 1e15 {
                    format!("{}", *n as i64)
                } else {
                    format!("{n}")
                }
            }
            Value::Text(s) => s.clone(),
            Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.into(),
            Value::Error(e) => e.clone(),
        }
    }
    pub fn is_empty(&self) -> bool {
        matches!(self, Value::Empty)
    }
}

/// 罫線の引き方。**日本の帳票は罫線で出来ている**ので、ここは飾りではない。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Borders {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

impl Borders {
    pub const ALL: Borders = Borders { top: true, bottom: true, left: true, right: true };
    pub const NONE: Borders = Borders { top: false, bottom: false, left: false, right: false };

    pub fn any(self) -> bool {
        self.top || self.bottom || self.left || self.right
    }
}

/// セルの横の揃え。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum HAlign {
    /// 指定なし — **数は右、文字は左**という表計算の既定に従う
    #[default]
    General,
    Left,
    Center,
    Right,
}

impl HAlign {
    pub fn as_xlsx(self) -> Option<&'static str> {
        match self {
            HAlign::General => None,
            HAlign::Left => Some("left"),
            HAlign::Center => Some("center"),
            HAlign::Right => Some("right"),
        }
    }
    pub fn from_xlsx(v: &str) -> HAlign {
        match v {
            "left" => HAlign::Left,
            "center" | "centerContinuous" => HAlign::Center,
            "right" => HAlign::Right,
            _ => HAlign::General,
        }
    }
}

/// セルの縦の揃え。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum VAlign {
    Top,
    Middle,
    /// xlsx の既定は下揃え
    #[default]
    Bottom,
}

impl VAlign {
    pub fn as_xlsx(self) -> Option<&'static str> {
        match self {
            VAlign::Top => Some("top"),
            VAlign::Middle => Some("center"),
            VAlign::Bottom => None,
        }
    }
    pub fn from_xlsx(v: &str) -> VAlign {
        match v {
            "top" => VAlign::Top,
            "center" => VAlign::Middle,
            _ => VAlign::Bottom,
        }
    }
}

/// セルの書式。xlsx の `styles.xml`(xf / font / fill / border)に対応する。
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct CellFormat {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub borders: Borders,
    pub align: HAlign,
    /// 塗りつぶしの色 `RRGGBB`
    pub fill: Option<String>,
    /// 文字色 `RRGGBB`
    pub color: Option<String>,
    /// 書体の名前(xlsx の `<font><name>`)。文書の設定
    pub font: Option<String>,
    /// 文字の大きさ(pt×100 で持つ。f32 だと Ord が付かない)
    pub size_c: Option<u32>,
    pub strike: bool,
    pub valign: VAlign,
    /// 折り返して全体を表示
    pub wrap: bool,
    /// 表示形式(`#,##0` `0.00%` など)。xlsx の numFmt
    pub number_format: Option<String>,
}

impl CellFormat {
    pub fn is_plain(&self) -> bool {
        *self == CellFormat::default()
    }
}

/// 1つのセル。入力(式か値)と、計算後の値と、見た目。
#[derive(Debug, Clone, Default)]
pub struct Cell {
    /// 式("=" で始まる)。無ければ None
    pub formula: Option<String>,
    /// 計算後の値(式が無ければ入力そのもの)
    pub value: Value,
    /// 見た目。**罫線はここ**
    pub fmt: CellFormat,
}

impl Default for Value {
    fn default() -> Self {
        Value::Empty
    }
}

impl Cell {
    /// 利用者が入力した文字列を、式か値として解釈する。
    pub fn input(s: &str) -> Cell {
        let t = s.trim();
        if let Some(f) = t.strip_prefix('=') {
            return Cell { formula: Some(f.to_string()), value: Value::Empty, fmt: Default::default() };
        }
        if t.is_empty() {
            return Cell::default();
        }
        if let Ok(n) = t.parse::<f64>() {
            return Cell { formula: None, value: Value::Number(n), fmt: Default::default() };
        }
        match t.to_ascii_uppercase().as_str() {
            "TRUE" => Cell { formula: None, value: Value::Bool(true), fmt: Default::default() },
            "FALSE" => Cell { formula: None, value: Value::Bool(false), fmt: Default::default() },
            _ => Cell { formula: None, value: Value::Text(t.to_string()), fmt: Default::default() },
        }
    }

    /// 編集欄に出す文字列(式ならその式、値ならその表示)。
    pub fn editable(&self) -> String {
        match &self.formula {
            Some(f) => format!("={f}"),
            None => self.value.display(),
        }
    }
}

/// 1枚のシート。疎な表なので BTreeMap で持つ(空セルは持たない)。
#[derive(Debug, Clone, Default)]
pub struct Sheet {
    pub name: String,
    pub cells: BTreeMap<Pos, Cell>,
    /// セル結合(左上, 右下)。**日本の帳票は結合で見出しを作る**ので、
    /// 読み飛ばして保存すると枠組みが壊れる
    pub merges: Vec<(Pos, Pos)>,
    /// 列幅(xlsx の単位 = 標準フォントの「0」何個ぶん)。無い列は既定幅。
    /// これも読み飛ばして保存すると帳票の形が変わる
    pub col_width: BTreeMap<u32, f32>,
    /// 全列の既定幅。`<col min="1" max="16384">` を1列ずつ展開しない
    /// (展開すると保存が 16,384 個の col で肥大する)
    pub default_col_width: Option<f32>,
    /// 行の高さ(pt)。無い行は既定。列幅と同じ構図
    pub row_height: BTreeMap<u32, f32>,
    /// 名前の定義(名前, 参照 "A1" か "A1:B2")。式の中で名前が使える。
    /// workbook.xml の definedNames と往復する
    pub names: Vec<(String, String)>,
    /// セルのハイパーリンク(外部URL)。sheet.xml の hyperlinks と往復する
    pub links: BTreeMap<Pos, String>,
    /// セルのコメント。commentsN.xml と往復する
    pub comments: BTreeMap<Pos, String>,
    /// 条件付き書式(cellIs だけ)。xlsx の conditionalFormatting と往復する
    pub cond: Vec<CondRule>,
    /// データの入力規則(list だけ)。xlsx の dataValidations と往復する
    pub validations: Vec<Validation>,
    /// 印刷の向き(xlsx の pageSetup orientation="landscape")。
    /// **読むだけ** — 保存は原文持ち越しが正。PDF がこれに従う
    pub landscape: bool,
    /// 用紙(xlsx の pageSetup paperSize。9=A4, 8=A3, 11=A5, 12=B4, 13=B5)
    pub paper_size: Option<u32>,
    /// 印刷の余白 mm(左, 右, 上, 下)。xlsx の pageMargins(インチ)から換算
    pub margins_mm: Option<(f32, f32, f32, f32)>,
    /// 印刷範囲(definedName _xlnm.Print_Area)。編集の対象なのでモデルで持つ
    /// (xlsx との往復は読み書きが解く)。複数の域も持てる
    pub print_areas: Vec<(Pos, Pos)>,
    /// 拡大縮小印刷(pageSetup scale、%)。無ければ 100
    pub print_scale: Option<u32>,
    /// 改ページ(このモデルでは「新しい紙をここから始める行」0起点。
    /// xlsx の rowBreaks/brk@id と同じ数え方)
    pub row_breaks: Vec<u32>,
    /// 枠線・見出し(行番号と列名)も印刷する(printOptions)
    pub print_gridlines: bool,
    pub print_headings: bool,
    /// タイトル行(各ページの頭で繰り返す行の範囲。Print_Titles の行の部)
    pub print_title_rows: Option<(u32, u32)>,
    /// 読んだ xlsx の図形(**表示だけ**。保存は原文の持ち越しが担う)
    pub shapes: Vec<SheetShape>,
    /// **このアプリで挿した**図形。保存でこちらが DrawingML として書き出す
    pub shapes_new: Vec<SheetShape>,
    /// 読んだ xlsx の画像(**表示だけ**。保存は原文の drawing 持ち越しが担う —
    /// 図形など理解しない部品を壊さないため、読んだ絵はこちらで書き直さない)
    pub images: Vec<SheetImage>,
    /// **このアプリで挿した**画像(グラフもこれ)。保存でこちらが部品
    /// (drawing・rels・media)ごと書き出す。読んだ画像と持ち場を分ける —
    /// 混ぜると保存で二重になる(writer と同じ構図)
    pub images_new: Vec<SheetImage>,
}

/// シートに浮かぶ図形。**中身はベクタ**(発注者案 2026-08-04: SVG で作る —
/// 拡大縮小で崩れない)。画面へは to_svg が SVG を作り、xlsx へは DrawingML の
/// 図形(prstGeom)として書く — Excel でも図形として開ける。
#[derive(Debug, Clone, PartialEq)]
pub struct SheetShape {
    /// 左上を留めるセル
    pub at: Pos,
    pub width_px: f32,
    pub height_px: f32,
    /// 図形の種類(xlsx の prstGeom の名前):
    /// rect / roundRect / ellipse / rightArrow / diamond / line
    pub kind: String,
    /// 塗り RRGGBB(無ければ塗らない)
    pub fill: Option<String>,
    /// 線 RRGGBB(無ければ引かない)
    pub line: Option<String>,
}

impl SheetShape {
    /// 画面用の SVG。**大きさを width/height に織り込む**ので、
    /// 描画側がその都度ラスタ化すれば、どの大きさでも輪郭が鮮明に出る。
    pub fn to_svg(&self) -> String {
        let (w, h) = (self.width_px.max(4.0), self.height_px.max(4.0));
        let fill = self
            .fill
            .as_deref()
            .map(|c| format!("#{c}"))
            .unwrap_or_else(|| "none".into());
        let line = self
            .line
            .as_deref()
            .map(|c| format!("#{c}"))
            .unwrap_or_else(|| "none".into());
        let style = format!(r#"fill="{fill}" stroke="{line}" stroke-width="2""#);
        // 線の太さの半分だけ内側に(縁が切れないように)
        let (x0, y0, x1, y1) = (1.0, 1.0, w - 1.0, h - 1.0);
        let body = match self.kind.as_str() {
            "roundRect" => format!(
                r#"<rect x="{x0}" y="{y0}" width="{}" height="{}" rx="{r}" ry="{r}" {style}/>"#,
                x1 - x0,
                y1 - y0,
                r = ((x1 - x0).min(y1 - y0) * 0.15).max(4.0)
            ),
            "ellipse" => format!(
                r#"<ellipse cx="{}" cy="{}" rx="{}" ry="{}" {style}/>"#,
                w / 2.0,
                h / 2.0,
                (x1 - x0) / 2.0,
                (y1 - y0) / 2.0
            ),
            "rightArrow" => {
                // 胴と鏃。高さの半分が鏃(prstGeom の既定に寄せる)
                let neck = h * 0.25;
                let head = (w * 0.35).min(h);
                format!(
                    r#"<polygon points="{x0},{ty} {bx},{ty} {bx},{y0} {x1},{my} {bx},{y1} {bx},{by} {x0},{by}" {style}/>"#,
                    ty = y0 + neck,
                    by = y1 - neck,
                    bx = x1 - head,
                    my = h / 2.0
                )
            }
            "diamond" => format!(
                r#"<polygon points="{},{y0} {x1},{} {},{y1} {x0},{}" {style}/>"#,
                w / 2.0,
                h / 2.0,
                w / 2.0,
                h / 2.0
            ),
            "line" => format!(r#"<line x1="{x0}" y1="{y0}" x2="{x1}" y2="{y1}" {style}/>"#),
            _ => format!(
                r#"<rect x="{x0}" y="{y0}" width="{}" height="{}" {style}/>"#,
                x1 - x0,
                y1 - y0
            ),
        };
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">{body}</svg>"#
        )
    }
}

/// シートに浮かぶ画像。左上をセルに留める(xlsx の oneCellAnchor)。
#[derive(Debug, Clone)]
pub struct SheetImage {
    /// 左上を留めるセル
    pub at: Pos,
    /// 画面での大きさ(px)。xlsx の EMU とは 9525 EMU = 1px で換算
    pub width_px: f32,
    pub height_px: f32,
    /// 絵の実体(PNG / JPEG)
    pub data: Vec<u8>,
}

/// データの入力規則(list だけ)。「この範囲は、この候補から選ぶ」。
///
/// `formula` は xlsx の formula1 の**原文**で持つ — `"甲,乙,丙"`(引用符つきの
/// 直書き)か `$D$2:$D$5`(同じシートの範囲参照)。候補は使うときに解決する
/// (範囲参照の中身が変われば候補も変わる — 原文を持てば追従できる)。
#[derive(Debug, Clone, PartialEq)]
pub struct Validation {
    pub range: (Pos, Pos),
    pub formula: String,
}

impl Validation {
    pub fn contains(&self, p: Pos) -> bool {
        let (a, b) = self.range;
        (a.row..=b.row).contains(&p.row) && (a.col..=b.col).contains(&p.col)
    }

    /// 候補の一覧。直書きは `,` で割り、範囲参照はそのシートの値を集める。
    /// 解決できない参照(別のシート等)は空 — 空の候補は「制限なし」と扱うこと
    /// (読めない規則で入力を堰き止めない)。
    pub fn options(&self, sheet: &Sheet) -> Vec<String> {
        let f = self.formula.trim();
        if let Some(inner) = f.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
            return inner
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        // 範囲参照。$ は絶対参照の印なので剥がして読む
        let clean: String = f.chars().filter(|c| *c != '$').collect();
        let (a, b) = match clean.split_once(':') {
            Some((x, y)) => match (Pos::parse(x), Pos::parse(y)) {
                (Some(a), Some(b)) => (a, b),
                _ => return Vec::new(),
            },
            None => match Pos::parse(&clean) {
                Some(p) => (p, p),
                None => return Vec::new(),
            },
        };
        let mut out = Vec::new();
        for r in a.row..=b.row {
            for c in a.col..=b.col {
                if let Some(cell) = sheet.cells.get(&Pos::new(r, c)) {
                    let v = cell.value.display();
                    if !v.is_empty() && !out.contains(&v) {
                        out.push(v);
                    }
                }
            }
        }
        out
    }
}

/// 条件付き書式の1本。「範囲の値が◯◯なら、この見た目」。
#[derive(Debug, Clone, PartialEq)]
pub struct CondRule {
    pub range: (Pos, Pos),
    pub op: CondOp,
    pub value: f64,
    /// 文字色 RRGGBB
    pub color: Option<String>,
    /// 塗り RRGGBB
    pub fill: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CondOp {
    Gt,
    Lt,
    Eq,
}

impl CondOp {
    pub fn as_xlsx(self) -> &'static str {
        match self {
            CondOp::Gt => "greaterThan",
            CondOp::Lt => "lessThan",
            CondOp::Eq => "equal",
        }
    }
    pub fn from_xlsx(s: &str) -> Option<CondOp> {
        match s {
            "greaterThan" => Some(CondOp::Gt),
            "lessThan" => Some(CondOp::Lt),
            "equal" => Some(CondOp::Eq),
            _ => None,
        }
    }
}

impl CondRule {
    /// この位置のこの値に効くか。数の値だけを見る(文字は対象にしない)。
    pub fn hits(&self, p: Pos, v: &Value) -> bool {
        let (a, b) = self.range;
        if !((a.row..=b.row).contains(&p.row) && (a.col..=b.col).contains(&p.col)) {
            return false;
        }
        let Value::Number(n) = v else { return false };
        match self.op {
            CondOp::Gt => *n > self.value,
            CondOp::Lt => *n < self.value,
            CondOp::Eq => (*n - self.value).abs() < f64::EPSILON,
        }
    }
}

impl Sheet {
    pub fn new(name: &str) -> Sheet {
        Sheet { name: name.to_string(), ..Default::default() }
    }
    pub fn get(&self, p: Pos) -> Option<&Cell> {
        self.cells.get(&p)
    }
    pub fn value(&self, p: Pos) -> Value {
        self.cells.get(&p).map(|c| c.value.clone()).unwrap_or(Value::Empty)
    }
    /// セルを置く。
    ///
    /// **中身も書式も無いセルは持たない**(表が無駄に太る)。
    /// ただし**罫線だけのセルは残す** — 値が無くても、
    /// 枠が引いてあれば帳票では意味を持つ。
    pub fn set(&mut self, p: Pos, c: Cell) {
        if c.formula.is_none() && c.value.is_empty() && c.fmt.is_plain() {
            self.cells.remove(&p);
        } else {
            self.cells.insert(p, c);
        }
    }
    /// 使われている範囲(行数, 列数)。空なら (0,0)。
    pub fn extent(&self) -> (u32, u32) {
        self.cells.keys().fold((0, 0), |(r, c), p| (r.max(p.row + 1), c.max(p.col + 1)))
    }
}

#[derive(Debug, Clone, Default)]
pub struct Book {
    pub sheets: Vec<Sheet>,
    /// こちらが理解できなかった definedName の原文(Print_Area など)。
    /// **理解はしないが、捨てない。** 保存でそのまま返す
    pub names_raw: Vec<String>,
}

impl Book {
    pub fn new() -> Book {
        Book { sheets: vec![Sheet::new("Sheet1")], names_raw: Vec::new() }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a1形式を読み書きできる() {
        for (s, r, c) in [("A1", 0, 0), ("B3", 2, 1), ("Z1", 0, 25),
                          ("AA1", 0, 26), ("AB10", 9, 27), ("$C$5", 4, 2)] {
            let p = Pos::parse(s).unwrap_or_else(|| panic!("{s} を読めない"));
            assert_eq!((p.row, p.col), (r, c), "{s}");
        }
        for s in ["A1", "B3", "Z1", "AA1", "AB10"] {
            assert_eq!(Pos::parse(s).unwrap().a1(), s);
        }
        assert!(Pos::parse("A0").is_none(), "0行は無い");
        assert!(Pos::parse("1A").is_none());
    }

    #[test]
    fn 入力が式と値に分かれる() {
        assert_eq!(Cell::input("123").value, Value::Number(123.0));
        assert_eq!(Cell::input("1.5").value, Value::Number(1.5));
        assert_eq!(Cell::input("日本フネン").value, Value::Text("日本フネン".into()));
        assert_eq!(Cell::input("TRUE").value, Value::Bool(true));
        assert_eq!(Cell::input("=SUM(A1:A3)").formula.as_deref(), Some("SUM(A1:A3)"));
        assert!(Cell::input("  ").formula.is_none());
    }

    #[test]
    fn 編集欄には式が戻る() {
        let mut c = Cell::input("=A1+1");
        c.value = Value::Number(42.0);
        assert_eq!(c.editable(), "=A1+1", "計算後も編集欄には式を出す");
        assert_eq!(c.value.display(), "42");
    }

    #[test]
    fn 数値の表示が事務向けになる() {
        assert_eq!(Value::Number(1000.0).display(), "1000", "整数に .0 を付けない");
        assert_eq!(Value::Number(1.5).display(), "1.5");
        assert_eq!(Value::Empty.display(), "");
    }
}

impl Sheet {
    /// 行を1つ挿し込む。**下にあるものを1つずつ下げる。**
    ///
    /// **残ったセルの式の参照も直す。** 直さないと、行を挿しただけで
    /// 式が別のセルを指し、間違った答えを黙って出す。
    pub fn insert_row(&mut self, at: u32) {
        self.shift(|p| p.row >= at, 1, 0);
        self.fix_formulas(at, 1, true);
        self.shift_merges(at, 1, true);
        self.row_height = self
            .row_height
            .iter()
            .map(|(r, h)| (if *r >= at { r + 1 } else { *r }, *h))
            .collect();
    }

    /// 行を1つ抜く。
    pub fn remove_row(&mut self, at: u32) {
        self.cells.retain(|p, _| p.row != at);
        self.shift(|p| p.row > at, -1, 0);
        self.fix_formulas(at, -1, true);
        self.shift_merges(at, -1, true);
        self.row_height = self
            .row_height
            .iter()
            .filter(|(r, _)| **r != at)
            .map(|(r, h)| (if *r > at { r - 1 } else { *r }, *h))
            .collect();
    }

    pub fn insert_col(&mut self, at: u32) {
        self.shift(|p| p.col >= at, 0, 1);
        self.fix_formulas(at, 1, false);
        self.shift_merges(at, 1, false);
        // 列幅も一緒に動かす
        self.col_width = self
            .col_width
            .iter()
            .map(|(c, w)| (if *c >= at { c + 1 } else { *c }, *w))
            .collect();
    }

    pub fn remove_col(&mut self, at: u32) {
        self.cells.retain(|p, _| p.col != at);
        self.shift(|p| p.col > at, 0, -1);
        self.fix_formulas(at, -1, false);
        self.shift_merges(at, -1, false);
        self.col_width = self
            .col_width
            .iter()
            .filter(|(c, _)| **c != at)
            .map(|(c, w)| (if *c > at { c - 1 } else { *c }, *w))
            .collect();
    }

    /// 出し入れに合わせて、**残ったセルの式の参照も直す**。
    /// これをやらないと、行を挿しただけで式が別のセルを指す。
    fn fix_formulas(&mut self, at: u32, delta: i64, is_row: bool) {
        for c in self.cells.values_mut() {
            if let Some(f) = &c.formula {
                c.formula = Some(shift_refs(f, at, delta, is_row));
            }
        }
    }

    /// 行・列の出し入れに合わせて結合の範囲も動かす。
    ///
    /// 削除では**上端と下端で動きが違う**: 上端が消えた行なら次の行が
    /// 滑り込む(据え置き)、下端が消えた行なら1つ縮む。
    fn shift_merges(&mut self, at: u32, delta: i64, is_row: bool) {
        let top = |v: u32| -> u32 {
            if delta > 0 {
                if v >= at { v + 1 } else { v }
            } else if v > at {
                v - 1
            } else {
                v
            }
        };
        let bottom = |v: u32| -> u32 {
            if delta > 0 {
                if v >= at { v + 1 } else { v }
            } else if v >= at {
                v.saturating_sub(1)
            } else {
                v
            }
        };
        for (a, b) in self.merges.iter_mut() {
            if is_row {
                a.row = top(a.row);
                b.row = bottom(b.row);
            } else {
                a.col = top(a.col);
                b.col = bottom(b.col);
            }
        }
        // 1セルに潰れた・裏返った結合は結合ではない
        self.merges.retain(|(a, b)| a <= b && (a.row != b.row || a.col != b.col));
    }

    /// この位置に効く入力規則(最初に見つかったもの)。
    pub fn validation_at(&self, p: Pos) -> Option<&Validation> {
        self.validations.iter().find(|v| v.contains(p))
    }

    /// この位置は結合に呑まれているか(左上を除く)。
    pub fn covered_by_merge(&self, p: Pos) -> bool {
        self.merges.iter().any(|(a, b)| {
            p != *a && (a.row..=b.row).contains(&p.row) && (a.col..=b.col).contains(&p.col)
        })
    }

    fn shift(&mut self, pick: impl Fn(&Pos) -> bool, dr: i64, dc: i64) {
        let moved: Vec<(Pos, Cell)> = self
            .cells
            .iter()
            .filter(|(p, _)| pick(p))
            .map(|(p, c)| (*p, c.clone()))
            .collect();
        for (p, _) in &moved {
            self.cells.remove(p);
        }
        for (p, c) in moved {
            let row = (p.row as i64 + dr).max(0) as u32;
            let col = (p.col as i64 + dc).max(0) as u32;
            self.cells.insert(Pos { row, col }, c);
        }
    }
}

#[cfg(test)]
mod rowcol_tests {
    use super::*;

    fn sheet() -> Sheet {
        let mut s = Sheet { name: "表".into(), ..Default::default() };
        for r in 0..3 {
            s.set(Pos { row: r, col: 0 }, Cell {
                formula: None, value: Value::Number(r as f64), fmt: Default::default() });
        }
        s
    }

    fn at(s: &Sheet, r: u32) -> Option<f64> {
        match s.get(Pos { row: r, col: 0 }).map(|c| c.value.clone()) {
            Some(Value::Number(n)) => Some(n),
            _ => None,
        }
    }

    #[test]
    fn 行を挿すと下がる() {
        let mut s = sheet();
        s.insert_row(1);
        assert_eq!(at(&s, 0), Some(0.0));
        assert_eq!(at(&s, 1), None, "挿した行が空でない");
        assert_eq!(at(&s, 2), Some(1.0), "下がっていない");
        assert_eq!(at(&s, 3), Some(2.0));
    }

    #[test]
    fn 行を抜くと詰まる() {
        let mut s = sheet();
        s.remove_row(1);
        assert_eq!(at(&s, 0), Some(0.0));
        assert_eq!(at(&s, 1), Some(2.0), "詰まっていない");
        assert_eq!(at(&s, 2), None, "元の場所が残っている");
    }

    #[test]
    fn 列も同じように動く() {
        let mut s = Sheet { name: "表".into(), ..Default::default() };
        s.set(Pos { row: 0, col: 0 }, Cell {
            formula: None, value: Value::Text("左".into()), fmt: Default::default() });
        s.set(Pos { row: 0, col: 1 }, Cell {
            formula: None, value: Value::Text("右".into()), fmt: Default::default() });
        s.insert_col(1);
        assert!(s.get(Pos { row: 0, col: 1 }).is_none());
        assert_eq!(s.get(Pos { row: 0, col: 2 }).map(|c| c.value.clone()),
                   Some(Value::Text("右".into())));
        s.remove_col(0);
        assert_eq!(s.get(Pos { row: 0, col: 1 }).map(|c| c.value.clone()),
                   Some(Value::Text("右".into())));
    }

    #[test]
    fn 罫線も一緒に動く() {
        // 帳票の枠が置き去りになると書類が壊れる
        let mut s = Sheet { name: "枠".into(), ..Default::default() };
        s.set(Pos { row: 1, col: 0 }, Cell {
            formula: None, value: Value::Empty,
            fmt: CellFormat { borders: Borders::ALL, ..Default::default() } });
        s.insert_row(0);
        assert!(s.get(Pos { row: 1, col: 0 }).is_none(), "元の場所に残っている");
        assert_eq!(s.get(Pos { row: 2, col: 0 }).map(|c| c.fmt.borders), Some(Borders::ALL));
    }

    #[test]
    fn 空の表でも落ちない() {
        let mut s = Sheet { name: "空".into(), ..Default::default() };
        s.insert_row(0);
        s.remove_row(0);
        s.insert_col(0);
        s.remove_col(0);
        assert!(s.cells.is_empty());
    }
}

/// 表示形式を当てて、画面に出す文字列にする。
///
/// **付けた書式が画面に出ないなら、それは飾りでしかない。**
/// 対応するのは実務で使う分だけ — 桁区切り・小数・パーセント・通貨。
/// 日付は別の話(連番の解釈が要る)なのでここでは扱わない。
pub fn format_value(v: &Value, code: Option<&str>) -> String {
    let Value::Number(n) = v else { return v.display() };
    let Some(code) = code else { return v.display() };

    let percent = code.contains('%');
    let n = if percent { n * 100.0 } else { *n };
    let comma = code.contains(',');
    // 小数点以下の桁数は書式の `.000` から数える
    let dec = code
        .rsplit_once('.')
        .map(|(_, d)| d.chars().take_while(|c| *c == '0' || *c == '#').count())
        .unwrap_or(0);

    let s = format!("{:.*}", dec, n.abs());
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i.to_string(), format!(".{f}")),
        None => (s, String::new()),
    };
    let int = if comma { group(&int) } else { int };

    let mut out = String::new();
    if n < 0.0 {
        out.push('-');
    }
    // 通貨の記号は書式の先頭にそのまま書かれている
    for c in code.chars() {
        if c == '#' || c == '0' || c == ',' || c == '.' || c == '%' || c == '"' {
            break;
        }
        out.push(c);
    }
    out.push_str(&int);
    out.push_str(&frac);
    if percent {
        out.push('%');
    }
    out
}

/// 3桁ごとに区切る。
fn group(s: &str) -> String {
    let b = s.as_bytes();
    let mut o = String::new();
    for (i, c) in b.iter().enumerate() {
        if i > 0 && (b.len() - i) % 3 == 0 {
            o.push(',');
        }
        o.push(*c as char);
    }
    o
}

#[cfg(test)]
mod format_tests {
    use super::*;

    fn f(n: f64, code: &str) -> String {
        format_value(&Value::Number(n), Some(code))
    }

    #[test]
    fn 桁区切り() {
        assert_eq!(f(1234567.0, "#,##0"), "1,234,567");
        assert_eq!(f(0.0, "#,##0"), "0");
        assert_eq!(f(999.0, "#,##0"), "999");
    }

    #[test]
    fn 小数() {
        assert_eq!(f(3.14159, "0.00"), "3.14");
        assert_eq!(f(3.0, "0.00"), "3.00");
        assert_eq!(f(1234.5, "#,##0.0"), "1,234.5");
    }

    #[test]
    fn パーセント() {
        assert_eq!(f(0.25, "0%"), "25%");
        assert_eq!(f(0.1234, "0.00%"), "12.34%");
    }

    #[test]
    fn 通貨() {
        assert_eq!(f(1200.0, "¥#,##0"), "¥1,200");
    }

    #[test]
    fn 負の数() {
        assert_eq!(f(-1234.0, "#,##0"), "-1,234");
        assert_eq!(f(-0.5, "0%"), "-50%");
    }

    #[test]
    fn 書式が無ければそのまま() {
        assert_eq!(format_value(&Value::Number(1234.0), None), "1234");
    }

    #[test]
    fn 数でなければ触らない() {
        assert_eq!(format_value(&Value::Text("品名".into()), Some("#,##0")), "品名");
        assert_eq!(format_value(&Value::Error("#DIV/0!".into()), Some("0%")), "#DIV/0!");
    }
}

/// 式の中の A1 参照を、行・列の出し入れに合わせてずらす。
///
/// **これをやらないと、行を挿しただけで式が別のセルを指す。**
/// 「動かない」ではなく「**間違った答えを黙って出す**」側の欠陥なので、
/// 帳票では致命的になる。
///
/// 絶対参照(`$C$5`)の `$` は形として残す — 利用者が書いたものを勝手に消さない。
/// 参照先が消えたときは `#REF!` にする(黙って別のセルを指すより良い)。
pub fn shift_refs(formula: &str, at: u32, delta: i64, is_row: bool) -> String {
    let ch: Vec<char> = formula.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < ch.len() {
        // 文字列の中の A1 らしきものは触らない
        if ch[i] == '"' {
            out.push('"');
            i += 1;
            while i < ch.len() {
                out.push(ch[i]);
                if ch[i] == '"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        // 参照の形: [$]英字+[$]数字+
        let start = i;
        let mut j = i;
        let abs_col = j < ch.len() && ch[j] == '$';
        if abs_col {
            j += 1;
        }
        let letters = j;
        while j < ch.len() && ch[j].is_ascii_alphabetic() {
            j += 1;
        }
        if j == letters {
            out.push(ch[i]);
            i += 1;
            continue;
        }
        let abs_row = j < ch.len() && ch[j] == '$';
        if abs_row {
            j += 1;
        }
        let digits = j;
        while j < ch.len() && ch[j].is_ascii_digit() {
            j += 1;
        }
        if j == digits {
            // 英字だけ = 関数名。触らない
            out.extend(&ch[start..j]);
            i = j;
            continue;
        }
        let raw: String = ch[start..j].iter().collect();
        out.push_str(&shift_one(&raw, at, delta, is_row, abs_col, abs_row));
        i = j;
    }
    out
}

fn shift_one(raw: &str, at: u32, delta: i64, is_row: bool, abs_col: bool, abs_row: bool) -> String {
    let Some(p) = Pos::parse(raw) else { return raw.to_string() };
    let target = if is_row { p.row } else { p.col };
    // 挿した/抜いた場所より手前は動かない
    if target < at {
        return raw.to_string();
    }
    // 抜いた行そのものを指していたら、指す先が無い
    if delta < 0 && target == at {
        return "#REF!".to_string();
    }
    let moved = (target as i64 + delta).max(0) as u32;
    let np = if is_row { Pos { row: moved, col: p.col } } else { Pos { row: p.row, col: moved } };
    // $ の形を戻す
    let a1 = np.a1();
    let split = a1.find(|c: char| c.is_ascii_digit()).unwrap_or(a1.len());
    let (c, r) = a1.split_at(split);
    format!("{}{c}{}{r}", if abs_col { "$" } else { "" }, if abs_row { "$" } else { "" })
}

#[cfg(test)]
mod ref_tests {
    use super::*;

    #[test]
    fn 挿した行より下の参照が下がる() {
        assert_eq!(shift_refs("=A5+B6", 2, 1, true), "=A6+B7");
    }

    #[test]
    fn 挿した行より上は動かない() {
        assert_eq!(shift_refs("=A1+A2", 5, 1, true), "=A1+A2");
    }

    #[test]
    fn 抜いた行より下が詰まる() {
        assert_eq!(shift_refs("=A5", 2, -1, true), "=A4");
    }

    #[test]
    fn 抜いた行を指していたら_ref_になる() {
        // 黙って隣のセルを指すより、壊れたと言う方がよい
        assert_eq!(shift_refs("=A3+B1", 2, -1, true), "=#REF!+B1");
    }

    #[test]
    fn 絶対参照の形が残る() {
        // 利用者が書いた $ を勝手に消さない
        assert_eq!(shift_refs("=$A$5", 2, 1, true), "=$A$6");
        assert_eq!(shift_refs("=$A5", 2, 1, true), "=$A6");
    }

    #[test]
    fn 列の出し入れも効く() {
        assert_eq!(shift_refs("=C1+A1", 1, 1, false), "=D1+A1");
        assert_eq!(shift_refs("=C1", 1, -1, false), "=B1");
    }

    #[test]
    fn 関数名を参照と間違えない() {
        assert_eq!(shift_refs("=SUM(A5:A9)", 2, 1, true), "=SUM(A6:A10)");
        assert_eq!(shift_refs("=IF(A5>0,1,0)", 2, 1, true), "=IF(A6>0,1,0)");
    }

    #[test]
    fn 文字列の中は触らない() {
        assert_eq!(shift_refs(r#"="A5は合計"&A5"#, 2, 1, true), r#"="A5は合計"&A6"#);
    }

    #[test]
    fn 数だけの式は変わらない() {
        assert_eq!(shift_refs("=1+2*3", 0, 1, true), "=1+2*3");
    }
}

#[cfg(test)]
mod rowcol_formula_tests {
    use super::*;

    fn sheet() -> Sheet {
        let mut s = Sheet { name: "帳票".into(), ..Default::default() };
        for r in 0..3 {
            s.set(Pos { row: r, col: 0 }, Cell {
                formula: None, value: Value::Number((r + 1) as f64), fmt: Default::default() });
        }
        // A4 = SUM(A1:A3)
        s.set(Pos { row: 3, col: 0 }, Cell {
            formula: Some("=SUM(A1:A3)".into()), value: Value::Empty, fmt: Default::default() });
        s
    }

    fn f(s: &Sheet, r: u32) -> Option<String> {
        s.get(Pos { row: r, col: 0 }).and_then(|c| c.formula.clone())
    }

    #[test]
    fn 行を挿すと式の参照も伸びる() {
        // これを直さないと、行を挿した瞬間に合計が合わなくなる
        let mut s = sheet();
        s.insert_row(1);
        assert_eq!(f(&s, 4).as_deref(), Some("=SUM(A1:A4)"), "参照が伸びていない");
    }

    #[test]
    fn 行を抜くと式の参照も縮む() {
        let mut s = sheet();
        s.remove_row(1);
        assert_eq!(f(&s, 2).as_deref(), Some("=SUM(A1:A2)"), "参照が縮んでいない");
    }

    #[test]
    fn 参照先を抜いたら_ref_が出る() {
        let mut s = Sheet { name: "表".into(), ..Default::default() };
        s.set(Pos { row: 0, col: 0 }, Cell {
            formula: Some("=A3".into()), value: Value::Empty, fmt: Default::default() });
        s.remove_row(2);
        assert_eq!(f(&s, 0).as_deref(), Some("=#REF!"), "壊れたのに黙って別のセルを指した");
    }
}

#[cfg(test)]
mod col_formula_tests {
    use super::*;

    #[test]
    fn 列の出し入れでも式が直る() {
        let mut s = Sheet { name: "表".into(), ..Default::default() };
        s.set(Pos { row: 0, col: 3 }, Cell {
            formula: Some("=B1+C1".into()), value: Value::Empty, fmt: Default::default() });
        s.insert_col(1);
        assert_eq!(s.get(Pos { row: 0, col: 4 }).and_then(|c| c.formula.clone()).as_deref(),
                   Some("=C1+D1"), "列を挿しても参照が動いていない");
        s.remove_col(1);
        assert_eq!(s.get(Pos { row: 0, col: 3 }).and_then(|c| c.formula.clone()).as_deref(),
                   Some("=B1+C1"), "列を抜いても参照が戻っていない");
    }
}

impl Sheet {
    /// 指定した列で並べ替える。
    ///
    /// **見出し行は動かさない**(`header` が true のとき先頭行を据え置く)。
    /// 帳票の並べ替えで見出しが混ざるのは事故なので、既定で守る。
    ///
    /// **行はまるごと動かす。** 選んだ列だけ並べ替えると、
    /// 隣の列との対応が壊れて、静かに嘘の表ができる。
    pub fn sort_by_column(&mut self, col: u32, ascending: bool, header: bool) {
        let (rows, cols) = self.extent();
        if rows == 0 { return }
        let (last_row, last_col) = (rows - 1, cols.saturating_sub(1));
        let first = if header { 1 } else { 0 };
        if last_row < first {
            return;
        }
        // 行をまるごと取り出す
        let mut rows: Vec<(u32, Vec<(u32, Cell)>)> = Vec::new();
        for r in first..=last_row {
            let cells: Vec<(u32, Cell)> = (0..=last_col)
                .filter_map(|c| self.cells.get(&Pos { row: r, col: c }).map(|x| (c, x.clone())))
                .collect();
            rows.push((r, cells));
        }
        rows.sort_by(|a, b| {
            let key = |v: &Vec<(u32, Cell)>| {
                v.iter().find(|(c, _)| *c == col).map(|(_, x)| x.value.clone())
            };
            let (x, y) = (key(&a.1), key(&b.1));
            let o = cmp_value(&x, &y);
            if ascending { o } else { o.reverse() }
        });
        // 置き直す
        for r in first..=last_row {
            for c in 0..=last_col {
                self.cells.remove(&Pos { row: r, col: c });
            }
        }
        for (i, (_, cells)) in rows.into_iter().enumerate() {
            let r = first + i as u32;
            for (c, cell) in cells {
                self.cells.insert(Pos { row: r, col: c }, cell);
            }
        }
    }

    /// 中身が同じ行を落とす。**先に出てきた方を残す。**
    ///
    /// 返すのは落とした行数 — 何件消したかを黙らない。
    pub fn remove_duplicate_rows(&mut self, header: bool) -> usize {
        let (rows, cols) = self.extent();
        if rows == 0 { return 0 }
        let (last_row, last_col) = (rows - 1, cols.saturating_sub(1));
        let first = if header { 1 } else { 0 };
        let mut seen: Vec<Vec<String>> = Vec::new();
        let mut keep: Vec<Vec<(u32, Cell)>> = Vec::new();
        let mut dropped = 0usize;
        for r in first..=last_row {
            let cells: Vec<(u32, Cell)> = (0..=last_col)
                .filter_map(|c| self.cells.get(&Pos { row: r, col: c }).map(|x| (c, x.clone())))
                .collect();
            let key: Vec<String> = (0..=last_col)
                .map(|c| {
                    cells.iter().find(|(cc, _)| *cc == c)
                        .map(|(_, x)| x.value.display()).unwrap_or_default()
                })
                .collect();
            // 空の行は重複と見なさない(表の中の空行は区切りとして使われる)
            if key.iter().all(|s| s.is_empty()) {
                keep.push(cells);
                continue;
            }
            if seen.contains(&key) {
                dropped += 1;
                continue;
            }
            seen.push(key);
            keep.push(cells);
        }
        for r in first..=last_row {
            for c in 0..=last_col {
                self.cells.remove(&Pos { row: r, col: c });
            }
        }
        for (i, cells) in keep.into_iter().enumerate() {
            let r = first + i as u32;
            for (c, cell) in cells {
                self.cells.insert(Pos { row: r, col: c }, cell);
            }
        }
        dropped
    }
}

/// 並べ替えの比較。**数は数として、文字は文字として。空は最後。**
fn cmp_value(a: &Option<Value>, b: &Option<Value>) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let rank = |v: &Option<Value>| match v {
        None => 3,
        Some(Value::Empty) => 3,
        Some(Value::Number(_)) => 0,
        Some(Value::Bool(_)) => 1,
        Some(Value::Text(_)) => 2,
        Some(Value::Error(_)) => 4,
    };
    let (ra, rb) = (rank(a), rank(b));
    if ra != rb {
        return ra.cmp(&rb);
    }
    match (a, b) {
        (Some(Value::Number(x)), Some(Value::Number(y))) => {
            x.partial_cmp(y).unwrap_or(Ordering::Equal)
        }
        (Some(Value::Text(x)), Some(Value::Text(y))) => x.cmp(y),
        (Some(Value::Bool(x)), Some(Value::Bool(y))) => x.cmp(y),
        _ => Ordering::Equal,
    }
}

#[cfg(test)]
mod sort_tests {
    use super::*;

    fn table(rows: &[(&str, f64)], header: bool) -> Sheet {
        let mut s = Sheet { name: "表".into(), ..Default::default() };
        let mut r = 0u32;
        if header {
            s.set(Pos { row: 0, col: 0 }, Cell {
                formula: None, value: Value::Text("品名".into()), fmt: Default::default() });
            s.set(Pos { row: 0, col: 1 }, Cell {
                formula: None, value: Value::Text("金額".into()), fmt: Default::default() });
            r = 1;
        }
        for (name, n) in rows {
            s.set(Pos { row: r, col: 0 }, Cell {
                formula: None, value: Value::Text((*name).into()), fmt: Default::default() });
            s.set(Pos { row: r, col: 1 }, Cell {
                formula: None, value: Value::Number(*n), fmt: Default::default() });
            r += 1;
        }
        s
    }

    fn col0(s: &Sheet, r: u32) -> String {
        s.get(Pos { row: r, col: 0 }).map(|c| c.value.display()).unwrap_or_default()
    }

    #[test]
    fn 数で並べ替えられる() {
        let mut s = table(&[("丙", 300.0), ("甲", 100.0), ("乙", 200.0)], false);
        s.sort_by_column(1, true, false);
        assert_eq!(col0(&s, 0), "甲");
        assert_eq!(col0(&s, 2), "丙");
    }

    #[test]
    fn 見出しは動かない() {
        // 帳票の並べ替えで見出しが混ざるのは事故
        let mut s = table(&[("丙", 300.0), ("甲", 100.0)], true);
        s.sort_by_column(1, true, true);
        assert_eq!(col0(&s, 0), "品名", "見出しが並べ替えに巻き込まれた");
        assert_eq!(col0(&s, 1), "甲");
    }

    #[test]
    fn 行はまるごと動く() {
        // 選んだ列だけ動かすと、隣の列との対応が壊れて静かに嘘の表になる
        let mut s = table(&[("丙", 300.0), ("甲", 100.0)], false);
        s.sort_by_column(1, true, false);
        let amount = |r: u32| s.get(Pos { row: r, col: 1 }).map(|c| c.value.clone());
        assert_eq!(col0(&s, 0), "甲");
        assert_eq!(amount(0), Some(Value::Number(100.0)), "名前と金額の対応が壊れた");
    }

    #[test]
    fn 降順にもできる() {
        let mut s = table(&[("甲", 100.0), ("丙", 300.0)], false);
        s.sort_by_column(1, false, false);
        assert_eq!(col0(&s, 0), "丙");
    }

    #[test]
    fn 空は最後に来る() {
        let mut s = table(&[("甲", 100.0)], false);
        s.set(Pos { row: 1, col: 0 }, Cell {
            formula: None, value: Value::Text("空欄".into()), fmt: Default::default() });
        s.sort_by_column(1, true, false);
        assert_eq!(col0(&s, 0), "甲", "空が先に来た");
    }

    #[test]
    fn 重複した行を落とせる() {
        let mut s = table(&[("甲", 100.0), ("甲", 100.0), ("乙", 200.0)], false);
        let n = s.remove_duplicate_rows(false);
        assert_eq!(n, 1, "落とした件数が違う");
        assert_eq!(col0(&s, 0), "甲");
        assert_eq!(col0(&s, 1), "乙");
        assert_eq!(col0(&s, 2), "", "詰まっていない");
    }

    #[test]
    fn 見出しは重複と見なさない() {
        let mut s = table(&[("品名", 0.0)], true);
        assert_eq!(s.remove_duplicate_rows(true), 0);
        assert_eq!(col0(&s, 0), "品名");
    }

    #[test]
    fn 空の表でも落ちない() {
        let mut s = Sheet { name: "空".into(), ..Default::default() };
        s.sort_by_column(0, true, true);
        assert_eq!(s.remove_duplicate_rows(true), 0);
    }
}

/// 参照の引き直しの結果。
pub enum MapRef {
    /// そのまま
    Keep,
    /// 参照先が動いた(一緒に動かす)
    To(Pos),
    /// 参照先が消えた(#REF! にする — 黙って別のセルを指すより良い)
    Broken,
}

/// 式の中の A1 参照を、写像 `f` で引き直す。
/// 文字列の中・関数名は触らない。`$` の形は保つ。
pub fn map_refs(formula: &str, f: impl Fn(Pos) -> MapRef) -> String {
    let ch: Vec<char> = formula.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < ch.len() {
        if ch[i] == '"' {
            out.push('"');
            i += 1;
            while i < ch.len() {
                out.push(ch[i]);
                if ch[i] == '"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        let start = i;
        let mut j = i;
        let abs_col = j < ch.len() && ch[j] == '$';
        if abs_col {
            j += 1;
        }
        let letters = j;
        while j < ch.len() && ch[j].is_ascii_alphabetic() {
            j += 1;
        }
        if j == letters {
            out.push(ch[i]);
            i += 1;
            continue;
        }
        let abs_row = j < ch.len() && ch[j] == '$';
        if abs_row {
            j += 1;
        }
        let digits = j;
        while j < ch.len() && ch[j].is_ascii_digit() {
            j += 1;
        }
        if j == digits {
            out.extend(&ch[start..j]);
            i = j;
            continue;
        }
        let raw: String = ch[start..j].iter().collect();
        match Pos::parse(&raw) {
            Some(p) => match f(p) {
                MapRef::Keep => out.push_str(&raw),
                MapRef::Broken => out.push_str("#REF!"),
                MapRef::To(np) => {
                    let a1 = np.a1();
                    let split = a1.find(|c: char| c.is_ascii_digit()).unwrap_or(a1.len());
                    let (c, r) = a1.split_at(split);
                    out.push_str(&format!(
                        "{}{c}{}{r}",
                        if abs_col { "$" } else { "" },
                        if abs_row { "$" } else { "" }
                    ));
                }
            },
            None => out.push_str(&raw),
        }
        i = j;
    }
    out
}

impl Sheet {
    /// 全部の式の参照を写像で引き直す。
    fn remap_formulas(&mut self, f: impl Fn(Pos) -> MapRef) {
        for c in self.cells.values_mut() {
            if let Some(fla) = &c.formula {
                c.formula = Some(map_refs(fla, &f));
            }
        }
    }

    /// 結合が「動く帯」の境界をまたいでいないか。またぐなら断る(Excel と同じ)。
    fn merges_cross(&self, in_band: impl Fn(Pos) -> bool) -> bool {
        self.merges.iter().any(|(a, b)| {
            let corners = [
                Pos::new(a.row, a.col),
                Pos::new(a.row, b.col),
                Pos::new(b.row, a.col),
                Pos::new(b.row, b.col),
            ];
            let inside = corners.iter().filter(|p| in_band(**p)).count();
            inside != 0 && inside != corners.len()
        })
    }

    /// 部分的な挿入。選んだ範囲の大きさぶん、帯のセルを右(または下)へずらす。
    /// **動いたセルを指す参照も一緒に動く。** 結合が帯をまたぐときは断る。
    pub fn insert_cells(&mut self, a: Pos, b: Pos, right: bool) -> Result<usize, String> {
        let n = if right { b.col - a.col + 1 } else { b.row - a.row + 1 };
        let in_band = |p: Pos| {
            if right {
                (a.row..=b.row).contains(&p.row) && p.col >= a.col
            } else {
                (a.col..=b.col).contains(&p.col) && p.row >= a.row
            }
        };
        if self.merges_cross(&in_band) {
            return Err("結合されたセルが範囲をまたいでいるため、シフトできません".into());
        }
        let shift = |p: Pos| {
            if right { Pos::new(p.row, p.col + n) } else { Pos::new(p.row + n, p.col) }
        };
        // 式の参照を先に引き直す(セルを動かす前の位置で判定する)
        self.remap_formulas(|p| if in_band(p) { MapRef::To(shift(p)) } else { MapRef::Keep });
        // セルを動かす
        let moved: Vec<(Pos, Cell)> = self
            .cells
            .iter()
            .filter(|(p, _)| in_band(**p))
            .map(|(p, c)| (*p, c.clone()))
            .collect();
        let count = moved.len();
        for (p, _) in &moved {
            self.cells.remove(p);
        }
        for (p, c) in moved {
            self.cells.insert(shift(p), c);
        }
        // 帯の中の結合も一緒に
        for (m1, m2) in self.merges.iter_mut() {
            if in_band(*m1) {
                *m1 = shift(*m1);
                *m2 = shift(*m2);
            }
        }
        Ok(count)
    }

    /// 部分的な削除。選んだ範囲を消し、帯の先のセルを左(または上)へ詰める。
    /// **消えた範囲を指していた参照は #REF! になる。**
    pub fn delete_cells(&mut self, a: Pos, b: Pos, left: bool) -> Result<usize, String> {
        let n = if left { b.col - a.col + 1 } else { b.row - a.row + 1 };
        let in_range =
            |p: Pos| (a.row..=b.row).contains(&p.row) && (a.col..=b.col).contains(&p.col);
        let beyond = |p: Pos| {
            if left {
                (a.row..=b.row).contains(&p.row) && p.col > b.col
            } else {
                (a.col..=b.col).contains(&p.col) && p.row > b.row
            }
        };
        let in_band = |p: Pos| in_range(p) || beyond(p);
        if self.merges_cross(&in_band) {
            return Err("結合されたセルが範囲をまたいでいるため、シフトできません".into());
        }
        let shift_back = |p: Pos| {
            if left { Pos::new(p.row, p.col - n) } else { Pos::new(p.row - n, p.col) }
        };
        self.remap_formulas(|p| {
            if in_range(p) {
                MapRef::Broken
            } else if beyond(p) {
                MapRef::To(shift_back(p))
            } else {
                MapRef::Keep
            }
        });
        let removed = self.cells.iter().filter(|(p, _)| in_range(**p)).count();
        self.cells.retain(|p, _| !in_range(*p));
        let moved: Vec<(Pos, Cell)> = self
            .cells
            .iter()
            .filter(|(p, _)| beyond(**p))
            .map(|(p, c)| (*p, c.clone()))
            .collect();
        for (p, _) in &moved {
            self.cells.remove(p);
        }
        for (p, c) in moved {
            self.cells.insert(shift_back(p), c);
        }
        self.merges.retain(|(m1, _)| !in_range(*m1));
        for (m1, m2) in self.merges.iter_mut() {
            if beyond(*m1) {
                *m1 = shift_back(*m1);
                *m2 = shift_back(*m2);
            }
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod cellshift_tests {
    use super::*;

    fn s3() -> Sheet {
        let mut s = Sheet::new("表");
        s.set(Pos::parse("A1").unwrap(), Cell::input("1"));
        s.set(Pos::parse("A2").unwrap(), Cell::input("2"));
        s.set(Pos::parse("B1").unwrap(), Cell::input("=A2*10"));
        s
    }

    #[test]
    fn 下へシフトすると参照も付いて動く() {
        let mut s = s3();
        // A1 の場所に1セル挿入(A列だけ下へ)
        s.insert_cells(Pos::parse("A1").unwrap(), Pos::parse("A1").unwrap(), false).unwrap();
        assert!(s.get(Pos::parse("A1").unwrap()).is_none(), "挿した場所が空でない");
        assert_eq!(s.value(Pos::parse("A2").unwrap()), Value::Number(1.0));
        assert_eq!(s.value(Pos::parse("A3").unwrap()), Value::Number(2.0));
        // B1 は動かないが、指していた A2 は A3 へ動いた
        assert_eq!(
            s.get(Pos::parse("B1").unwrap()).and_then(|c| c.formula.clone()).as_deref(),
            Some("A3*10"),
            "動いたセルへの参照が付いて動いていない"
        );
    }

    #[test]
    fn 右へシフトは行の帯だけ動く() {
        let mut s = s3();
        s.insert_cells(Pos::parse("A1").unwrap(), Pos::parse("A1").unwrap(), true).unwrap();
        assert_eq!(s.value(Pos::parse("B1").unwrap()), Value::Number(1.0), "右へ動いていない");
        // 2行目は帯の外。動かない
        assert_eq!(s.value(Pos::parse("A2").unwrap()), Value::Number(2.0));
        // 元の B1 の式は C1 へ動き、A2 への参照はそのまま
        assert_eq!(
            s.get(Pos::parse("C1").unwrap()).and_then(|c| c.formula.clone()).as_deref(),
            Some("A2*10")
        );
    }

    #[test]
    fn 上へ詰めると消えた参照はrefになる() {
        let mut s = s3();
        // A1 を削除して上へ詰める → A2(=1)ではなく元A1が消え、A2の中身が A1 へ
        s.delete_cells(Pos::parse("A1").unwrap(), Pos::parse("A1").unwrap(), false).unwrap();
        assert_eq!(s.value(Pos::parse("A1").unwrap()), Value::Number(2.0), "詰まっていない");
        // B1 が指していた A2 は A1 へ動いた
        assert_eq!(
            s.get(Pos::parse("B1").unwrap()).and_then(|c| c.formula.clone()).as_deref(),
            Some("A1*10")
        );
        // こんどは参照先そのものを消す
        let mut s2 = s3();
        s2.delete_cells(Pos::parse("A2").unwrap(), Pos::parse("A2").unwrap(), false).unwrap();
        assert_eq!(
            s2.get(Pos::parse("B1").unwrap()).and_then(|c| c.formula.clone()).as_deref(),
            Some("#REF!*10"),
            "消えた参照が黙って別のセルを指した"
        );
    }

    #[test]
    fn 結合が帯をまたぐと断る() {
        let mut s = s3();
        s.merges.push((Pos::parse("A1").unwrap(), Pos::parse("B1").unwrap()));
        let r = s.insert_cells(Pos::parse("A1").unwrap(), Pos::parse("A1").unwrap(), false);
        assert!(r.is_err(), "結合をまたぐシフトを黙って通した");
    }
}

/// 式の中の相対参照を (dr, dc) だけずらす。**コピーの規則**。
///
/// 行の出し入れ(`shift_refs`)とは別物 — コピーでは位置に関係なく
/// **相対参照が全部ずれ、`$` の付いた側だけ止まる**。
/// 紙の外(負の位置)を指すことになったら `#REF!`。
pub fn offset_refs(formula: &str, dr: i64, dc: i64) -> String {
    let ch: Vec<char> = formula.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < ch.len() {
        if ch[i] == '"' {
            out.push('"');
            i += 1;
            while i < ch.len() {
                out.push(ch[i]);
                if ch[i] == '"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        let start = i;
        let mut j = i;
        let abs_col = j < ch.len() && ch[j] == '$';
        if abs_col {
            j += 1;
        }
        let letters = j;
        while j < ch.len() && ch[j].is_ascii_alphabetic() {
            j += 1;
        }
        if j == letters {
            out.push(ch[i]);
            i += 1;
            continue;
        }
        let abs_row = j < ch.len() && ch[j] == '$';
        if abs_row {
            j += 1;
        }
        let digits = j;
        while j < ch.len() && ch[j].is_ascii_digit() {
            j += 1;
        }
        if j == digits {
            out.extend(&ch[start..j]);
            i = j;
            continue;
        }
        let raw: String = ch[start..j].iter().collect();
        match Pos::parse(&raw) {
            Some(p) => {
                let nr = if abs_row { p.row as i64 } else { p.row as i64 + dr };
                let nc = if abs_col { p.col as i64 } else { p.col as i64 + dc };
                if nr < 0 || nc < 0 {
                    out.push_str("#REF!");
                } else {
                    let a1 = Pos { row: nr as u32, col: nc as u32 }.a1();
                    let split = a1.find(|c: char| c.is_ascii_digit()).unwrap_or(a1.len());
                    let (c, r) = a1.split_at(split);
                    out.push_str(&format!(
                        "{}{c}{}{r}",
                        if abs_col { "$" } else { "" },
                        if abs_row { "$" } else { "" }
                    ));
                }
            }
            None => out.push_str(&raw),
        }
        i = j;
    }
    out
}

#[cfg(test)]
mod offset_tests {
    use super::*;

    #[test]
    fn 相対参照は全部ずれる() {
        assert_eq!(offset_refs("=A1+B2", 1, 0), "=A2+B3");
        assert_eq!(offset_refs("=SUM(A1:A3)", 2, 0), "=SUM(A3:A5)");
    }

    #[test]
    fn 固定した側は止まる() {
        assert_eq!(offset_refs("=$A$1+A1", 1, 1), "=$A$1+B2");
        assert_eq!(offset_refs("=A$1", 3, 0), "=A$1", "行を固定したのに動いた");
        assert_eq!(offset_refs("=$A1", 0, 3), "=$A1", "列を固定したのに動いた");
    }

    #[test]
    fn 紙の外はrefになる() {
        assert_eq!(offset_refs("=A1", -1, 0), "=#REF!");
    }

    #[test]
    fn 文字列と関数名は触らない() {
        assert_eq!(offset_refs(r#"="A1"&A1"#, 1, 0), r#"="A1"&A2"#);
        assert_eq!(offset_refs("=SUM(A1)", 1, 0), "=SUM(A2)");
    }
}

#[cfg(test)]
mod validation_tests {
    use super::*;

    #[test]
    fn 直書きの候補が割れる() {
        let v = Validation {
            range: (Pos::new(1, 1), Pos::new(9, 1)),
            formula: r#""甲, 乙,丙""#.into(),
        };
        let s = Sheet::default();
        assert_eq!(v.options(&s), vec!["甲", "乙", "丙"], "空白ごと候補にした");
        assert!(v.contains(Pos::new(5, 1)));
        assert!(!v.contains(Pos::new(5, 2)));
    }

    #[test]
    fn 範囲参照の候補はシートの値から集まる() {
        let mut s = Sheet::default();
        for (r, t) in [(1, "東京"), (2, "大阪"), (3, "東京"), (4, "")] {
            s.set(Pos::new(r, 3), Cell::input(t));
        }
        let v = Validation {
            range: (Pos::new(0, 0), Pos::new(0, 0)),
            formula: "$D$2:$D$5".into(),
        };
        assert_eq!(v.options(&s), vec!["東京", "大阪"], "重複と空欄が候補に入った");
        // 解決できない参照は空(制限なしと扱う側の約束)
        let alien = Validation {
            range: (Pos::new(0, 0), Pos::new(0, 0)),
            formula: "Sheet2!$A$1:$A$3".into(),
        };
        assert!(alien.options(&s).is_empty());
    }

    #[test]
    fn 位置に効く規則が引ける() {
        let mut s = Sheet::default();
        s.validations.push(Validation {
            range: (Pos::new(1, 1), Pos::new(3, 1)),
            formula: r#""a,b""#.into(),
        });
        assert!(s.validation_at(Pos::new(2, 1)).is_some());
        assert!(s.validation_at(Pos::new(2, 2)).is_none());
    }
}


#[cfg(test)]
mod shape_tests {
    use super::*;

    #[test]
    fn 図形のsvgに大きさと色が入る() {
        let sh = SheetShape {
            at: Pos::new(0, 0),
            width_px: 200.0,
            height_px: 100.0,
            kind: "ellipse".into(),
            fill: Some("FFF2CC".into()),
            line: Some("1B6E3C".into()),
        };
        let svg = sh.to_svg();
        assert!(svg.contains(r#"width="200""#), "{svg}");
        assert!(svg.contains("#FFF2CC") && svg.contains("#1B6E3C"));
        assert!(svg.contains("<ellipse"));
        // 知らない種類は四角で描く(黙って消さない)
        let unknown = SheetShape { kind: "hexagon".into(), ..sh };
        assert!(unknown.to_svg().contains("<rect"));
    }
}

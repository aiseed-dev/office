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
}

impl Book {
    pub fn new() -> Book {
        Book { sheets: vec![Sheet::new("Sheet1")] }
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
    }

    /// 行を1つ抜く。
    pub fn remove_row(&mut self, at: u32) {
        self.cells.retain(|p, _| p.row != at);
        self.shift(|p| p.row > at, -1, 0);
        self.fix_formulas(at, -1, true);
    }

    pub fn insert_col(&mut self, at: u32) {
        self.shift(|p| p.col >= at, 0, 1);
        self.fix_formulas(at, 1, false);
    }

    pub fn remove_col(&mut self, at: u32) {
        self.cells.retain(|p, _| p.col != at);
        self.shift(|p| p.col > at, 0, -1);
        self.fix_formulas(at, -1, false);
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

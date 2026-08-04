//! 式の評価と再計算。
//!
//! 範囲は「Euro-Office ができている範囲」で十分という方針のうち、
//! **事務で実際に使うところ**に絞る: 四則・比較・括弧・セル参照・範囲・
//! よく使う関数(SUM/AVERAGE/COUNT/COUNTA/MIN/MAX/IF/ROUND/ABS/AND/OR/NOT/
//! CONCATENATE)。
//!
//! **マクロは実装しない。** これは機能不足ではなく設計判断で、
//! 「開く=実行」という攻撃経路を最初から持たないため(migration-kit DESIGN.md §5)。
//!
//! 循環参照は検出してエラーにする(黙って0を返さない)。

use std::collections::{HashMap, HashSet};

use crate::model::{format_value, Pos, Sheet, Value};

// ---------- 字句 ----------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ref(Pos),
    Range(Pos, Pos),
    Name(String),
    Op(char),
    Cmp(String),
    LParen,
    RParen,
    Comma,
}

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let b: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let c = b[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '"' {
            let mut s = String::new();
            i += 1;
            while i < b.len() && b[i] != '"' {
                s.push(b[i]);
                i += 1;
            }
            if i >= b.len() {
                return Err("文字列が閉じていません".into());
            }
            i += 1;
            out.push(Tok::Str(s));
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && i + 1 < b.len() && b[i + 1].is_ascii_digit()) {
            let st = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == '.') {
                i += 1;
            }
            let s: String = b[st..i].iter().collect();
            out.push(Tok::Num(s.parse().map_err(|_| format!("数値として読めません: {s}"))?));
            continue;
        }
        if c.is_ascii_alphabetic() || c == '$' || c == '_' {
            let st = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == '$' || b[i] == '_' || b[i] == '.') {
                i += 1;
            }
            let word: String = b[st..i].iter().collect();
            // A1:B3 の範囲
            if i < b.len() && b[i] == ':' {
                let st2 = i + 1;
                let mut j = st2;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == '$') {
                    j += 1;
                }
                let word2: String = b[st2..j].iter().collect();
                if let (Some(a), Some(z)) = (Pos::parse(&word), Pos::parse(&word2)) {
                    out.push(Tok::Range(a, z));
                    i = j;
                    continue;
                }
            }
            match Pos::parse(&word) {
                Some(p) => out.push(Tok::Ref(p)),
                None => out.push(Tok::Name(word.to_ascii_uppercase())),
            }
            continue;
        }
        // 比較演算子
        if "<>=".contains(c) {
            let two: String = b[i..(i + 2).min(b.len())].iter().collect();
            if ["<=", ">=", "<>"].contains(&two.as_str()) {
                out.push(Tok::Cmp(two));
                i += 2;
                continue;
            }
            out.push(Tok::Cmp(c.to_string()));
            i += 1;
            continue;
        }
        match c {
            '+' | '-' | '*' | '/' | '^' | '&' => out.push(Tok::Op(c)),
            '(' => out.push(Tok::LParen),
            ')' => out.push(Tok::RParen),
            ',' => out.push(Tok::Comma),
            _ => return Err(format!("読めない文字: {c}")),
        }
        i += 1;
    }
    Ok(out)
}

// ---------- 構文と評価(再帰下降) ----------

struct P<'a> {
    t: &'a [Tok],
    i: usize,
    sheet: &'a Sheet,
    resolved: &'a HashMap<Pos, Value>,
    /// いま計算しているセル。ROW()/COLUMN()(引数なし)が使う
    at: Pos,
}

impl<'a> P<'a> {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.t.get(self.i).cloned();
        self.i += 1;
        t
    }

    fn cell(&self, p: Pos) -> Value {
        self.resolved.get(&p).cloned().unwrap_or_else(|| self.sheet.value(p))
    }

    fn range_values(&self, a: Pos, z: Pos) -> Vec<Value> {
        let (r0, r1) = (a.row.min(z.row), a.row.max(z.row));
        let (c0, c1) = (a.col.min(z.col), a.col.max(z.col));
        let mut v = Vec::new();
        for r in r0..=r1 {
            for c in c0..=c1 {
                v.push(self.cell(Pos::new(r, c)));
            }
        }
        v
    }

    // 比較 < 加減 < 乗除 < 冪 < 単項 < 原子
    fn expr(&mut self) -> Result<Value, String> {
        let lhs = self.add()?;
        if let Some(Tok::Cmp(op)) = self.peek().cloned() {
            self.next();
            let rhs = self.add()?;
            let r = match (&lhs, &rhs) {
                (Value::Text(a), Value::Text(b)) => match op.as_str() {
                    "=" => a == b,
                    "<>" => a != b,
                    "<" => a < b,
                    ">" => a > b,
                    "<=" => a <= b,
                    ">=" => a >= b,
                    _ => return Err(format!("比較演算子が不正: {op}")),
                },
                _ => {
                    let (a, b) = (lhs.as_number(), rhs.as_number());
                    match op.as_str() {
                        "=" => (a - b).abs() < f64::EPSILON,
                        "<>" => (a - b).abs() >= f64::EPSILON,
                        "<" => a < b,
                        ">" => a > b,
                        "<=" => a <= b,
                        ">=" => a >= b,
                        _ => return Err(format!("比較演算子が不正: {op}")),
                    }
                }
            };
            return Ok(Value::Bool(r));
        }
        Ok(lhs)
    }

    fn add(&mut self) -> Result<Value, String> {
        let mut v = self.mul()?;
        while let Some(Tok::Op(o @ ('+' | '-' | '&'))) = self.peek().cloned() {
            self.next();
            let r = self.mul()?;
            // エラーは伝播する(表計算の作法)。ここで消すと循環参照が0になって隠れる
            if let Value::Error(_) = v { continue }
            if let Value::Error(_) = r { v = r; continue }
            v = match o {
                '+' => Value::Number(v.as_number() + r.as_number()),
                '-' => Value::Number(v.as_number() - r.as_number()),
                // & は文字列連結(表計算の作法)
                _ => Value::Text(format!("{}{}", v.display(), r.display())),
            };
        }
        Ok(v)
    }

    fn mul(&mut self) -> Result<Value, String> {
        let mut v = self.pow()?;
        while let Some(Tok::Op(o @ ('*' | '/'))) = self.peek().cloned() {
            self.next();
            let r = self.pow()?;
            if let Value::Error(_) = v { continue }
            if let Value::Error(_) = r { v = r; continue }
            if o == '/' && r.as_number() == 0.0 {
                return Ok(Value::Error("#DIV/0!".into()));
            }
            v = Value::Number(match o {
                '*' => v.as_number() * r.as_number(),
                _ => v.as_number() / r.as_number(),
            });
        }
        Ok(v)
    }

    fn pow(&mut self) -> Result<Value, String> {
        let v = self.unary()?;
        if let Some(Tok::Op('^')) = self.peek() {
            self.next();
            let r = self.pow()?;
            return Ok(Value::Number(v.as_number().powf(r.as_number())));
        }
        Ok(v)
    }

    fn unary(&mut self) -> Result<Value, String> {
        match self.peek().cloned() {
            Some(Tok::Op('-')) => {
                self.next();
                match self.unary()? {
                    e @ Value::Error(_) => Ok(e),
                    v => Ok(Value::Number(-v.as_number())),
                }
            }
            Some(Tok::Op('+')) => {
                self.next();
                self.unary()
            }
            _ => self.atom(),
        }
    }

    fn args(&mut self) -> Result<Vec<Arg>, String> {
        // 関数の引数。範囲は**形(列数)を残して**包む — VLOOKUP・INDEX は
        // 表の縦横が要る。平らな値しか要らない関数は flatten で崩す
        let mut out = Vec::new();
        if let Some(Tok::RParen) = self.peek() {
            self.next();
            return Ok(out);
        }
        loop {
            if let Some(Tok::Range(a, z)) = self.peek().cloned() {
                self.next();
                let cols = a.col.abs_diff(z.col) + 1;
                out.push(Arg::Rect(cols, self.range_values(a, z)));
            } else {
                out.push(Arg::One(self.expr()?));
            }
            match self.next() {
                Some(Tok::Comma) => continue,
                Some(Tok::RParen) => break,
                _ => return Err("引数の括弧が閉じていません".into()),
            }
        }
        Ok(out)
    }

    /// ROW / COLUMN / ROWS / COLUMNS — 参照の位置と大きさを答える。
    /// 値ではなく**参照そのもの**が要るので、args() で崩す前に読む。
    /// 引数なしの ROW()/COLUMN() は、いま計算しているセルの位置
    fn pos_fn(&mut self, name: &str) -> Result<Value, String> {
        let (a, z) = match self.peek().cloned() {
            Some(Tok::RParen) => (self.at, self.at),
            Some(Tok::Ref(p)) => {
                self.next();
                (p, p)
            }
            Some(Tok::Range(a, z)) => {
                self.next();
                (a, z)
            }
            _ => return Ok(Value::Error("#VALUE!".into())),
        };
        match self.next() {
            Some(Tok::RParen) => {}
            _ => return Err("引数の括弧が閉じていません".into()),
        }
        Ok(Value::Number(match name {
            "ROW" => (a.row.min(z.row) + 1) as f64,
            "COLUMN" => (a.col.min(z.col) + 1) as f64,
            "ROWS" => (a.row.abs_diff(z.row) + 1) as f64,
            _ => (a.col.abs_diff(z.col) + 1) as f64,
        }))
    }

    fn atom(&mut self) -> Result<Value, String> {
        match self.next() {
            Some(Tok::Num(n)) => Ok(Value::Number(n)),
            Some(Tok::Str(s)) => Ok(Value::Text(s)),
            Some(Tok::Ref(p)) => Ok(self.cell(p)),
            Some(Tok::Range(a, z)) => {
                // 単独の範囲は先頭セルの値(関数の外では範囲は使えない)
                Ok(self.range_values(a, z).into_iter().next().unwrap_or(Value::Empty))
            }
            Some(Tok::LParen) => {
                let v = self.expr()?;
                match self.next() {
                    Some(Tok::RParen) => Ok(v),
                    _ => Err("括弧が閉じていません".into()),
                }
            }
            Some(Tok::Name(name)) => {
                match self.peek() {
                    Some(Tok::LParen) => {
                        self.next();
                        // 参照の「位置」を答える関数は、値に崩す前に受ける
                        if matches!(name.as_str(), "ROW" | "COLUMN" | "ROWS" | "COLUMNS") {
                            return self.pos_fn(&name);
                        }
                        let args = self.args()?;
                        call(&name, args)
                    }
                    _ => match name.as_str() {
                        "TRUE" => Ok(Value::Bool(true)),
                        "FALSE" => Ok(Value::Bool(false)),
                        _ => Ok(Value::Error("#NAME?".into())),
                    },
                }
            }
            other => Err(format!("式が途中で終わっています: {other:?}")),
        }
    }
}

/// SUMIF / COUNTIF の条件合わせ。数は数として、文字は文字として比べる。
fn matches_cond(v: &Value, cond: &Value) -> bool {
    match cond {
        Value::Number(n) => (v.as_number() - n).abs() < f64::EPSILON,
        Value::Text(s) => {
            // ">100" のような書き方に応える
            let t = s.trim();
            for (op, f) in [
                (">=", (|a: f64, b: f64| a >= b) as fn(f64, f64) -> bool),
                ("<=", |a, b| a <= b),
                ("<>", |a, b| (a - b).abs() >= f64::EPSILON),
                (">", |a, b| a > b),
                ("<", |a, b| a < b),
                ("=", |a, b| (a - b).abs() < f64::EPSILON),
            ] {
                if let Some(rest) = t.strip_prefix(op) {
                    if let Ok(n) = rest.trim().parse::<f64>() {
                        return !v.is_empty() && f(v.as_number(), n);
                    }
                }
            }
            v.display() == *s
        }
        _ => false,
    }
}

/// 暦(y,m,d)→ 1970-01-01 からの日数(Howard Hinnant の civil_from_days の逆)。
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// 1970-01-01 からの日数 → 暦(y,m,d)。
pub(crate) fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Excel の日付の通し番号(1899-12-30 起点)と 1970 起点の橋。
pub(crate) const EXCEL_EPOCH_DAYS: i64 = 25569;

/// 暦の日付 → Excel の通し番号。DATE 関数と pysheet(datetime の受け口)が
/// **同じ規約を通るための一本道** — 別々に持つと必ずずれる。
pub fn date_serial(y: i64, m: i64, d: i64) -> i64 {
    days_from_civil(y, m, d) + EXCEL_EPOCH_DAYS
}

/// 通し番号 → 曜日(0=日曜)。通し番号 1(1900-01-01)は月曜。
pub(crate) fn weekday0(serial: i64) -> i64 {
    // 1970-01-01(木)起点に直して数える
    ((serial - EXCEL_EPOCH_DAYS).rem_euclid(7) + 4).rem_euclid(7)
}

/// いまの機械の暦での「今日」の通し番号と、時刻(日の割合)。
/// 時計は系の TZ 環境(日本なら JST)に従う — libc の localtime を使う
/// chrono に頼らず、TZ のずれは環境変数 JO_TZ_OFF_HOURS で補える(既定 +9)。
fn today_serial() -> (f64, f64) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let off_h: i64 = std::env::var("JO_TZ_OFF_HOURS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(9);
    let local = secs + off_h * 3600;
    let days = local.div_euclid(86400);
    let frac = local.rem_euclid(86400) as f64 / 86400.0;
    ((days + EXCEL_EPOCH_DAYS) as f64, frac)
}

/// 関数の引数。ほとんどの関数は平らな値で足りるが、表を引く関数
/// (VLOOKUP・INDEX 等)は範囲の**形**(列数)が要る。
#[derive(Debug, Clone)]
enum Arg {
    One(Value),
    /// (列数, 行優先の値)
    Rect(u32, Vec<Value>),
}

impl Arg {
    fn values(&self) -> &[Value] {
        match self {
            Arg::One(v) => std::slice::from_ref(v),
            Arg::Rect(_, vs) => vs,
        }
    }
    fn first(&self) -> Value {
        self.values().first().cloned().unwrap_or(Value::Empty)
    }
}

fn call(name: &str, args: Vec<Arg>) -> Result<Value, String> {
    // 表を引く関数は形が要るので、平らにする前に受ける
    match name {
        "VLOOKUP" | "HLOOKUP" => {
            let key = args.first().map(|g| g.first()).unwrap_or(Value::Empty);
            let Some(Arg::Rect(cols, vals)) = args.get(1) else {
                return Ok(Value::Error("#VALUE!".into()));
            };
            let idx = args.get(2).map(|g| g.first().as_number()).unwrap_or(0.0) as usize;
            let (cols, vals) = (*cols as usize, vals);
            if cols == 0 || idx == 0 {
                return Ok(Value::Error("#VALUE!".into()));
            }
            let rows = vals.len() / cols;
            let same = |v: &Value| -> bool {
                match (v, &key) {
                    (Value::Number(x), Value::Number(y)) => (x - y).abs() < 1e-9,
                    _ => v.display() == key.display(),
                }
            };
            let hit = if name == "VLOOKUP" {
                // 1列目を上から探し、その行の idx 列目
                (0..rows)
                    .find(|r| same(&vals[r * cols]))
                    .and_then(|r| vals.get(r * cols + (idx - 1)))
            } else {
                // 1行目を左から探し、その列の idx 行目
                (0..cols)
                    .find(|c| same(&vals[*c]))
                    .and_then(|c| vals.get((idx - 1) * cols + c))
            };
            return Ok(hit.cloned().unwrap_or(Value::Error("#N/A".into())));
        }
        "INDEX" => {
            let Some(Arg::Rect(cols, vals)) = args.first() else {
                return Ok(Value::Error("#VALUE!".into()));
            };
            let cols = *cols as usize;
            let r = args.get(1).map(|g| g.first().as_number()).unwrap_or(0.0) as usize;
            let c = args.get(2).map(|g| g.first().as_number()).unwrap_or(1.0) as usize;
            if r == 0 || c == 0 {
                return Ok(Value::Error("#VALUE!".into()));
            }
            return Ok(vals
                .get((r - 1) * cols + (c - 1))
                .cloned()
                .unwrap_or(Value::Error("#REF!".into())));
        }
        "MATCH" => {
            let key = args.first().map(|g| g.first()).unwrap_or(Value::Empty);
            let hay = args.get(1).map(|g| g.values()).unwrap_or(&[]);
            // 照合の型は 0(完全一致)だけを受ける(それ以外は正直に断る)
            if let Some(t) = args.get(2) {
                if t.first().as_number() != 0.0 {
                    return Ok(Value::Error("#VALUE!".into()));
                }
            }
            return Ok(hay
                .iter()
                .position(|v| v.display() == key.display())
                .map(|i| Value::Number((i + 1) as f64))
                .unwrap_or(Value::Error("#N/A".into())));
        }
        "XLOOKUP" => {
            // XLOOKUP(探す値, 探す範囲, 返す範囲, [見つからないとき]) — 完全一致
            let key = args.first().map(|g| g.first()).unwrap_or(Value::Empty);
            let hay = args.get(1).map(|g| g.values()).unwrap_or(&[]);
            let ret = args.get(2).map(|g| g.values()).unwrap_or(&[]);
            if hay.is_empty() || hay.len() != ret.len() {
                return Ok(Value::Error("#VALUE!".into()));
            }
            let same = |v: &Value| match (v, &key) {
                (Value::Number(x), Value::Number(y)) => (x - y).abs() < 1e-9,
                _ => v.display() == key.display(),
            };
            return Ok(match hay.iter().position(same) {
                Some(i) => ret[i].clone(),
                None => args.get(3).map(|g| g.first()).unwrap_or(Value::Error("#N/A".into())),
            });
        }
        "COUNTIFS" | "SUMIFS" | "AVERAGEIFS" | "MINIFS" | "MAXIFS" => {
            // SUMIFS(合計範囲, 条件範囲1, 条件1, …) / COUNTIFS(条件範囲1, 条件1, …)
            // 条件は**行ごとに全部**合ったものだけ数える(範囲は同じ大きさ)
            let (vals, pairs) = if name == "COUNTIFS" {
                (None, &args[..])
            } else {
                (args.first(), args.get(1..).unwrap_or(&[]))
            };
            if pairs.is_empty() || pairs.len() % 2 != 0 {
                return Ok(Value::Error("#VALUE!".into()));
            }
            let n = pairs[0].values().len();
            if pairs.chunks(2).any(|c| c[0].values().len() != n)
                || vals.map(|v| v.values().len() != n).unwrap_or(false)
            {
                return Ok(Value::Error("#VALUE!".into()));
            }
            let hit = |i: usize| {
                pairs.chunks(2).all(|c| matches_cond(&c[0].values()[i], &c[1].first()))
            };
            let picked: Vec<f64> = (0..n)
                .filter(|i| hit(*i))
                .map(|i| vals.map(|v| v.values()[i].as_number()).unwrap_or(0.0))
                .collect();
            return Ok(match name {
                "COUNTIFS" => Value::Number(picked.len() as f64),
                "SUMIFS" => Value::Number(picked.iter().sum()),
                "AVERAGEIFS" => {
                    if picked.is_empty() {
                        Value::Error("#DIV/0!".into())
                    } else {
                        Value::Number(picked.iter().sum::<f64>() / picked.len() as f64)
                    }
                }
                // Excel の約束: 1件も合わなければ 0
                "MINIFS" => Value::Number(picked.iter().cloned().reduce(f64::min).unwrap_or(0.0)),
                _ => Value::Number(picked.iter().cloned().reduce(f64::max).unwrap_or(0.0)),
            });
        }
        "AVERAGEIF" => {
            // AVERAGEIF(条件を見る範囲, 条件, [平均する範囲])
            let rng = args.first().map(|g| g.values()).unwrap_or(&[]);
            let cond = args.get(1).map(|g| g.first()).unwrap_or(Value::Empty);
            let avg = args.get(2).map(|g| g.values()).unwrap_or(rng);
            if avg.len() != rng.len() {
                return Ok(Value::Error("#VALUE!".into()));
            }
            let picked: Vec<f64> = (0..rng.len())
                .filter(|i| matches_cond(&rng[*i], &cond))
                .map(|i| avg[i].as_number())
                .collect();
            return Ok(if picked.is_empty() {
                Value::Error("#DIV/0!".into())
            } else {
                Value::Number(picked.iter().sum::<f64>() / picked.len() as f64)
            });
        }
        "SUMPRODUCT" => {
            // 同じ大きさの範囲を要素ごとに掛けて、全部足す
            let n = args.first().map(|g| g.values().len()).unwrap_or(0);
            if n == 0 || args.iter().any(|g| g.values().len() != n) {
                return Ok(Value::Error("#VALUE!".into()));
            }
            let mut total = 0.0;
            for i in 0..n {
                total += args.iter().map(|g| g.values()[i].as_number()).product::<f64>();
            }
            return Ok(Value::Number(total));
        }
        "LARGE" | "SMALL" => {
            // 大きい方(小さい方)から k 番目。数だけを見る
            let mut ns: Vec<f64> = args
                .first()
                .map(|g| {
                    g.values()
                        .iter()
                        .filter(|v| matches!(v, Value::Number(_)))
                        .map(|v| v.as_number())
                        .collect()
                })
                .unwrap_or_default();
            let k = args.get(1).map(|g| g.first().as_number()).unwrap_or(0.0) as usize;
            if k == 0 || k > ns.len() {
                return Ok(Value::Error("#NUM!".into()));
            }
            ns.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
            return Ok(Value::Number(if name == "LARGE" { ns[ns.len() - k] } else { ns[k - 1] }));
        }
        "RANK" => {
            // RANK(値, 範囲, [順序]) — 省略は大きい方が1位。同値は同順位
            let x = args.first().map(|g| g.first().as_number()).unwrap_or(0.0);
            let ns: Vec<f64> = args
                .get(1)
                .map(|g| {
                    g.values()
                        .iter()
                        .filter(|v| matches!(v, Value::Number(_)))
                        .map(|v| v.as_number())
                        .collect()
                })
                .unwrap_or_default();
            let asc = args.get(2).map(|g| g.first().as_number() != 0.0).unwrap_or(false);
            if !ns.iter().any(|v| (v - x).abs() < 1e-9) {
                return Ok(Value::Error("#N/A".into()));
            }
            let better =
                ns.iter().filter(|v| if asc { **v < x - 1e-9 } else { **v > x + 1e-9 }).count();
            return Ok(Value::Number((better + 1) as f64));
        }
        _ => {}
    }
    let a: Vec<Value> = args.iter().flat_map(|g| g.values().iter().cloned()).collect();
    // 引数にエラーがあればそれを返す(黙って0として数えない)。
    // ただしエラーを受けて働く関数(IFERROR・ISERROR・ISBLANK・IF)と、
    // 選ばなかった枝のエラーを踏んではいけない関数(IFS・SWITCH・CHOOSE)は素通しする
    if !matches!(name, "IFERROR" | "ISERROR" | "ISBLANK" | "IF" | "IFS" | "SWITCH" | "CHOOSE") {
        if let Some(e) = a.iter().find(|v| matches!(v, Value::Error(_))) {
            return Ok(e.clone());
        }
    }
    let nums = |a: &[Value]| -> Vec<f64> {
        a.iter().filter(|v| !v.is_empty()).map(|v| v.as_number()).collect()
    };
    Ok(match name {
        "SUM" => Value::Number(nums(&a).iter().sum()),
        "AVERAGE" => {
            let n = nums(&a);
            if n.is_empty() {
                Value::Error("#DIV/0!".into())
            } else {
                Value::Number(n.iter().sum::<f64>() / n.len() as f64)
            }
        }
        "COUNT" => Value::Number(
            a.iter().filter(|v| matches!(v, Value::Number(_))).count() as f64),
        "COUNTA" => Value::Number(a.iter().filter(|v| !v.is_empty()).count() as f64),
        "MIN" => nums(&a).into_iter().reduce(f64::min).map(Value::Number)
            .unwrap_or(Value::Number(0.0)),
        "MAX" => nums(&a).into_iter().reduce(f64::max).map(Value::Number)
            .unwrap_or(Value::Number(0.0)),
        // 事務でよく使うもの。無いと「関数が違う」で止まる
        "ROUNDDOWN" | "TRUNC" => {
            let n = nums(&a);
            let (v, d) = (n.first().copied().unwrap_or(0.0), n.get(1).copied().unwrap_or(0.0));
            let f = 10f64.powi(d as i32);
            Value::Number((v * f).trunc() / f)
        }
        "ROUNDUP" => {
            let n = nums(&a);
            let (v, d) = (n.first().copied().unwrap_or(0.0), n.get(1).copied().unwrap_or(0.0));
            let f = 10f64.powi(d as i32);
            // 0 から遠ざかる向きに上げる(負の数で符号が入れ替わらないように)
            Value::Number(if v < 0.0 { (v * f).floor() / f } else { (v * f).ceil() / f })
        }
        "SUMIF" => {
            // SUMIF(範囲, 条件) — 条件に合うものだけ足す
            let cond = a.last().cloned().unwrap_or(Value::Empty);
            let sum: f64 = a[..a.len().saturating_sub(1)]
                .iter()
                .filter(|v| matches_cond(v, &cond))
                .map(|v| v.as_number())
                .sum();
            Value::Number(sum)
        }
        "COUNTIF" => {
            let cond = a.last().cloned().unwrap_or(Value::Empty);
            let n = a[..a.len().saturating_sub(1)].iter().filter(|v| matches_cond(v, &cond)).count();
            Value::Number(n as f64)
        }
        "PRODUCT" => Value::Number(nums(&a).iter().product()),
        "MOD" => {
            let n = nums(&a);
            let (x, y) = (n.first().copied().unwrap_or(0.0), n.get(1).copied().unwrap_or(0.0));
            if y == 0.0 {
                // 0 で割った答えは無い。黙って 0 を返さない
                Value::Error("#DIV/0!".into())
            } else {
                Value::Number(x - y * (x / y).floor())
            }
        }
        "POWER" => {
            let n = nums(&a);
            Value::Number(n.first().copied().unwrap_or(0.0)
                .powf(n.get(1).copied().unwrap_or(0.0)))
        }
        "SQRT" => {
            let v = nums(&a).first().copied().unwrap_or(0.0);
            if v < 0.0 { Value::Error("#NUM!".into()) } else { Value::Number(v.sqrt()) }
        }
        "LEFT" | "RIGHT" | "MID" => {
            let s = a.first().map(|v| v.display()).unwrap_or_default();
            let ch: Vec<char> = s.chars().collect();
            let n = |i: usize| a.get(i).map(|v| v.as_number() as usize).unwrap_or(0);
            Value::Text(match name {
                "LEFT" => ch.iter().take(n(1).min(ch.len())).collect(),
                "RIGHT" => ch.iter().skip(ch.len().saturating_sub(n(1))).collect(),
                // MID は1始まり(表計算の約束)
                _ => ch.iter().skip(n(1).saturating_sub(1)).take(n(2)).collect(),
            })
        }
        "TRIM" => Value::Text(a.first().map(|v| v.display()).unwrap_or_default().trim().to_string()),
        "UPPER" => Value::Text(a.first().map(|v| v.display()).unwrap_or_default().to_uppercase()),
        "LOWER" => Value::Text(a.first().map(|v| v.display()).unwrap_or_default().to_lowercase()),
        "ISBLANK" => Value::Bool(a.first().map(|v| v.is_empty()).unwrap_or(true)),
        "ISERROR" => Value::Bool(matches!(a.first(), Some(Value::Error(_)))),
        "IFERROR" => {
            // 第1引数がエラーなら第2引数(無ければ空)に落とす
            match a.first() {
                Some(Value::Error(_)) => a.get(1).cloned().unwrap_or(Value::Empty),
                v => v.cloned().unwrap_or(Value::Empty),
            }
        }
        "ABS" => Value::Number(a.first().map(|v| v.as_number().abs()).unwrap_or(0.0)),
        "ROUND" => {
            let x = a.first().map(|v| v.as_number()).unwrap_or(0.0);
            let d = a.get(1).map(|v| v.as_number()).unwrap_or(0.0) as i32;
            let f = 10f64.powi(d);
            Value::Number((x * f).round() / f)
        }
        "INT" => Value::Number(a.first().map(|v| v.as_number().floor()).unwrap_or(0.0)),
        "IF" => {
            // 条件のエラーは伝える。選ばなかった側のエラーは踏まない
            // (引数は先に評価済みなので、値の段階で無視するのが遅延評価の代わり)
            if let Some(e @ Value::Error(_)) = a.first() {
                return Ok(e.clone());
            }
            let c = matches!(a.first(), Some(Value::Bool(true)))
                || a.first().map(|v| v.as_number() != 0.0).unwrap_or(false);
            if c {
                a.get(1).cloned().unwrap_or(Value::Bool(true))
            } else {
                a.get(2).cloned().unwrap_or(Value::Bool(false))
            }
        }
        "AND" => Value::Bool(a.iter().all(|v| v.as_number() != 0.0
            || matches!(v, Value::Bool(true)))),
        "OR" => Value::Bool(a.iter().any(|v| v.as_number() != 0.0
            || matches!(v, Value::Bool(true)))),
        "NOT" => Value::Bool(!(a.first().map(|v| v.as_number() != 0.0).unwrap_or(false))),
        "CONCATENATE" => Value::Text(a.iter().map(|v| v.display()).collect()),
        // ---- 日付と時刻(値は Excel の通し番号 1899-12-30 起点)----
        "TODAY" => Value::Number(today_serial().0),
        "NOW" => {
            let (d, f) = today_serial();
            Value::Number(d + f)
        }
        "DATE" => {
            let g = |i: usize| a.get(i).map(|v| v.as_number() as i64).unwrap_or(0);
            Value::Number(date_serial(g(0), g(1), g(2)) as f64)
        }
        "YEAR" | "MONTH" | "DAY" => {
            let serial = a.first().map(|v| v.as_number()).unwrap_or(0.0) as i64;
            let (y, m, d) = civil_from_days(serial - EXCEL_EPOCH_DAYS);
            Value::Number(match name {
                "YEAR" => y,
                "MONTH" => m,
                _ => d,
            } as f64)
        }
        "WEEKDAY" => {
            // Excel の既定(1=日曜)。通し番号 1(1900-01-01)は月曜
            let serial = a.first().map(|v| v.as_number()).unwrap_or(0.0) as i64;
            Value::Number(weekday0(serial) as f64 + 1.0)
        }
        "TIME" => {
            let g = |i: usize| a.get(i).map(|v| v.as_number()).unwrap_or(0.0);
            let secs = g(0) * 3600.0 + g(1) * 60.0 + g(2);
            Value::Number(secs.rem_euclid(86400.0) / 86400.0)
        }
        "HOUR" | "MINUTE" | "SECOND" => {
            let serial = a.first().map(|v| v.as_number()).unwrap_or(0.0);
            let total = (serial.rem_euclid(1.0) * 86400.0).round() as i64;
            Value::Number(match name {
                "HOUR" => total / 3600 % 24,
                "MINUTE" => total / 60 % 60,
                _ => total % 60,
            } as f64)
        }
        "DATEVALUE" => {
            // "2026/8/5"・"2026-8-5"・"2026年8月5日" を通し番号に
            let s = a.first().map(|v| v.display()).unwrap_or_default();
            let t = s.trim().replace(['年', '月'], "/");
            let t = t.trim_end_matches('日');
            let parts: Vec<i64> =
                t.split(['/', '-']).filter_map(|p| p.trim().parse().ok()).collect();
            match parts.as_slice() {
                [y, m, d] => Value::Number(date_serial(*y, *m, *d) as f64),
                _ => Value::Error("#VALUE!".into()),
            }
        }
        "EDATE" | "EOMONTH" => {
            // n ヶ月あと(前)。EDATE は同じ日(無ければ月末)、EOMONTH はその月末
            let serial = a.first().map(|v| v.as_number()).unwrap_or(0.0) as i64;
            let months = a.get(1).map(|v| v.as_number()).unwrap_or(0.0) as i64;
            let (y, m, d) = civil_from_days(serial - EXCEL_EPOCH_DAYS);
            let total = y * 12 + (m - 1) + months;
            let (ny, nm) = (total.div_euclid(12), total.rem_euclid(12) + 1);
            let month_end = date_serial(ny, nm + 1, 1) - 1; // 13月は翌年1月に正しく繰り上がる
            Value::Number(match name {
                "EOMONTH" => month_end,
                _ => date_serial(ny, nm, d).min(month_end),
            } as f64)
        }
        "DATEDIF" => {
            // DATEDIF(始, 終, 単位) — 単位は Y/M/D/YM/MD/YD
            let s = a.first().map(|v| v.as_number()).unwrap_or(0.0) as i64;
            let e = a.get(1).map(|v| v.as_number()).unwrap_or(0.0) as i64;
            let unit = a.get(2).map(|v| v.display().to_uppercase()).unwrap_or_default();
            if e < s {
                return Ok(Value::Error("#NUM!".into()));
            }
            let (sy, sm, sd) = civil_from_days(s - EXCEL_EPOCH_DAYS);
            let (ey, em, ed) = civil_from_days(e - EXCEL_EPOCH_DAYS);
            let borrow = (em, ed) < (sm, sd);
            let months = ey * 12 + em - (sy * 12 + sm) - i64::from(ed < sd);
            Value::Number(match unit.as_str() {
                "Y" => ey - sy - i64::from(borrow),
                "M" => months,
                "D" => e - s,
                "YM" => months.rem_euclid(12),
                "YD" => {
                    // 年を無視した日数: 始の年を終の直前まで進めて引く
                    let anchor = date_serial(ey - i64::from(borrow), sm, sd);
                    e - anchor
                }
                "MD" => {
                    // 月を無視した日数: 始の「日」を終の月(足りなければ前月)に置いて引く
                    let (ay, am) = if ed >= sd {
                        (ey, em)
                    } else if em == 1 {
                        (ey - 1, 12)
                    } else {
                        (ey, em - 1)
                    };
                    e - date_serial(ay, am, sd)
                }
                _ => return Ok(Value::Error("#VALUE!".into())),
            } as f64)
        }
        "WORKDAY" => {
            // WORKDAY(始, 日数, [休みの日…]) — 土日と休みを飛ばして数える
            let mut cur = a.first().map(|v| v.as_number()).unwrap_or(0.0) as i64;
            let days = a.get(1).map(|v| v.as_number()).unwrap_or(0.0) as i64;
            let holidays: HashSet<i64> =
                a.get(2..).unwrap_or(&[]).iter().map(|v| v.as_number() as i64).collect();
            if days.abs() > 1_000_000 {
                return Ok(Value::Error("#NUM!".into()));
            }
            let step = if days < 0 { -1 } else { 1 };
            let mut left = days.abs();
            while left > 0 {
                cur += step;
                let w = weekday0(cur);
                if w != 0 && w != 6 && !holidays.contains(&cur) {
                    left -= 1;
                }
            }
            Value::Number(cur as f64)
        }
        "NETWORKDAYS" => {
            // NETWORKDAYS(始, 終, [休みの日…]) — 両端を含む平日の数
            let s = a.first().map(|v| v.as_number()).unwrap_or(0.0) as i64;
            let e = a.get(1).map(|v| v.as_number()).unwrap_or(0.0) as i64;
            let holidays: HashSet<i64> =
                a.get(2..).unwrap_or(&[]).iter().map(|v| v.as_number() as i64).collect();
            let (lo, hi) = (s.min(e), s.max(e));
            if hi - lo > 10_000_000 {
                return Ok(Value::Error("#NUM!".into()));
            }
            let n = (lo..=hi)
                .filter(|d| {
                    let w = weekday0(*d);
                    w != 0 && w != 6 && !holidays.contains(d)
                })
                .count() as i64;
            Value::Number(if e < s { -n } else { n } as f64)
        }
        // ---- 財務(閉じた式で解けるものだけ。RATE のような反復解は持たない)----
        "PMT" | "PV" | "FV" | "NPER" => {
            let g = |i: usize| a.get(i).map(|v| v.as_number()).unwrap_or(0.0);
            let rate = g(0);
            match name {
                "PMT" => {
                    let (nper, pv, fv) = (g(1), g(2), g(3));
                    if nper == 0.0 {
                        Value::Error("#DIV/0!".into())
                    } else if rate == 0.0 {
                        Value::Number(-(pv + fv) / nper)
                    } else {
                        let k = (1.0 + rate).powf(nper);
                        Value::Number(-(pv * k + fv) * rate / (k - 1.0))
                    }
                }
                "PV" => {
                    let (nper, pmt, fv) = (g(1), g(2), g(3));
                    if rate == 0.0 {
                        Value::Number(-(pmt * nper + fv))
                    } else {
                        let k = (1.0 + rate).powf(nper);
                        Value::Number(-(pmt * (k - 1.0) / rate + fv) / k)
                    }
                }
                "FV" => {
                    let (nper, pmt, pv) = (g(1), g(2), g(3));
                    if rate == 0.0 {
                        Value::Number(-(pv + pmt * nper))
                    } else {
                        let k = (1.0 + rate).powf(nper);
                        Value::Number(-(pv * k + pmt * (k - 1.0) / rate))
                    }
                }
                _ => {
                    // NPER(rate, pmt, pv, [fv])
                    let (pmt, pv, fv) = (g(1), g(2), g(3));
                    if rate == 0.0 {
                        if pmt == 0.0 {
                            Value::Error("#DIV/0!".into())
                        } else {
                            Value::Number(-(pv + fv) / pmt)
                        }
                    } else {
                        let x = (pmt / rate - fv) / (pv + pmt / rate);
                        if x <= 0.0 {
                            Value::Error("#NUM!".into())
                        } else {
                            Value::Number(x.ln() / (1.0 + rate).ln())
                        }
                    }
                }
            }
        }
        "LEN" => Value::Number(a.first().map(|v| v.display().chars().count())
            .unwrap_or(0) as f64),
        // ---- 選ぶ関数(選ばなかった枝のエラーは踏まない — IF と同じ考え)----
        "IFS" => {
            // IFS(条件1, 値1, 条件2, 値2, …) — 最初に真になった対の値
            let mut out = Value::Error("#N/A".into());
            let mut i = 0;
            while let Some(c) = a.get(i) {
                if let Value::Error(_) = c {
                    out = c.clone();
                    break;
                }
                if c.as_number() != 0.0 {
                    out = a.get(i + 1).cloned().unwrap_or(Value::Empty);
                    break;
                }
                i += 2;
            }
            out
        }
        "SWITCH" => {
            // SWITCH(式, 候補1, 値1, …, [どれでもないとき])
            let key = a.first().cloned().unwrap_or(Value::Empty);
            if let Value::Error(_) = key {
                return Ok(key);
            }
            let rest = a.get(1..).unwrap_or(&[]);
            let mut out = if rest.len() % 2 == 1 {
                rest.last().cloned().unwrap_or(Value::Empty)
            } else {
                Value::Error("#N/A".into())
            };
            let mut i = 0;
            while i + 1 < rest.len() {
                if !matches!(rest[i], Value::Error(_)) && rest[i].display() == key.display() {
                    out = rest[i + 1].clone();
                    break;
                }
                i += 2;
            }
            out
        }
        "CHOOSE" => {
            // CHOOSE(番号, 値1, 値2, …) — 番号は1起点
            let idx = a.first().cloned().unwrap_or(Value::Empty);
            if let Value::Error(_) = idx {
                return Ok(idx);
            }
            let i = idx.as_number() as usize;
            if i == 0 || i >= a.len() {
                Value::Error("#VALUE!".into())
            } else {
                a[i].clone()
            }
        }
        // ---- 文字列 ----
        "TEXT" => {
            // TEXT(値, 表示形式) — セルの表示と同じ描き方で文字列にする
            let v = a.first().cloned().unwrap_or(Value::Empty);
            let code = a.get(1).map(|v| v.display()).unwrap_or_default();
            Value::Text(format_value(&v, Some(&code)))
        }
        "SUBSTITUTE" => {
            // SUBSTITUTE(文字列, 探す, 置く, [何個目だけ])
            let s = a.first().map(|v| v.display()).unwrap_or_default();
            let old = a.get(1).map(|v| v.display()).unwrap_or_default();
            let new = a.get(2).map(|v| v.display()).unwrap_or_default();
            if old.is_empty() {
                return Ok(Value::Text(s));
            }
            match a.get(3) {
                None => Value::Text(s.replace(&old, &new)),
                Some(nth) => {
                    let n = nth.as_number() as usize;
                    match s.match_indices(&old).nth(n.saturating_sub(1)) {
                        Some((i, _)) if n >= 1 => {
                            let mut t = s.clone();
                            t.replace_range(i..i + old.len(), &new);
                            Value::Text(t)
                        }
                        _ => Value::Text(s),
                    }
                }
            }
        }
        "FIND" | "SEARCH" => {
            // FIND(探す, 文字列, [開始]) — 1起点の文字番号。SEARCH は大文字小文字を見ない
            let (mut needle, mut hay) = (
                a.first().map(|v| v.display()).unwrap_or_default(),
                a.get(1).map(|v| v.display()).unwrap_or_default(),
            );
            if name == "SEARCH" {
                needle = needle.to_lowercase();
                hay = hay.to_lowercase();
            }
            let start = (a.get(2).map(|v| v.as_number()).unwrap_or(1.0) as usize).max(1);
            let ch: Vec<char> = hay.chars().collect();
            let from: String = ch.iter().skip(start - 1).collect();
            match from.find(&needle) {
                Some(b) => {
                    // バイト位置 → 文字番号(1起点、開始位置ぶんを足し戻す)
                    let chars_before = from[..b].chars().count();
                    Value::Number((start + chars_before) as f64)
                }
                None => Value::Error("#VALUE!".into()),
            }
        }
        "VALUE" => {
            // 「¥1,234」のような表示も数に戻す(記号と桁区切りを外して読む)
            let s = a.first().map(|v| v.display()).unwrap_or_default();
            let t: String =
                s.trim().chars().filter(|c| !matches!(c, ',' | '¥' | '\u{a0}' | ' ')).collect();
            match t.trim_end_matches('%').parse::<f64>() {
                Ok(n) if t.ends_with('%') => Value::Number(n / 100.0),
                Ok(n) => Value::Number(n),
                Err(_) => Value::Error("#VALUE!".into()),
            }
        }
        "TEXTJOIN" => {
            // TEXTJOIN(区切り, 空を飛ばすか, 値…)
            let delim = a.first().map(|v| v.display()).unwrap_or_default();
            let skip_empty = a.get(1).map(|v| v.as_number() != 0.0).unwrap_or(true);
            let parts: Vec<String> = a
                .get(2..)
                .unwrap_or(&[])
                .iter()
                .map(|v| v.display())
                .filter(|s| !(skip_empty && s.is_empty())) // 空文字も「空」と見る
                .collect();
            Value::Text(parts.join(&delim))
        }
        "REPT" => {
            let s = a.first().map(|v| v.display()).unwrap_or_default();
            let n = a.get(1).map(|v| v.as_number()).unwrap_or(0.0);
            if n < 0.0 || s.chars().count() as f64 * n > 32767.0 {
                Value::Error("#VALUE!".into())
            } else {
                Value::Text(s.repeat(n as usize))
            }
        }
        "CHAR" => {
            let n = a.first().map(|v| v.as_number()).unwrap_or(0.0) as u32;
            match char::from_u32(n) {
                Some(c) if n > 0 => Value::Text(c.to_string()),
                _ => Value::Error("#VALUE!".into()),
            }
        }
        "CODE" => match a.first().map(|v| v.display()).unwrap_or_default().chars().next() {
            Some(c) => Value::Number(c as u32 as f64),
            None => Value::Error("#VALUE!".into()),
        },
        "PY" => Value::Error("#PY単独".into()), // =PY(…) はセル単独でだけ使える
        _ => Value::Error("#NAME?".into()),
    })
}

/// PY セルの呼び出しを解く: (関数名, 引数)。引数は式をいま評価した値
/// (範囲は列数つきの2次元)。**Python は動かさない** — 材料を出すだけ。
pub enum PyArg {
    One(Value),
    /// (列数, 行優先の値)
    Rect(u32, Vec<Value>),
}

pub fn eval_py_call(sheet: &Sheet, formula: &str) -> Option<(String, Vec<PyArg>)> {
    if !is_py_formula(formula) {
        return None;
    }
    let expanded = expand_names(formula, &sheet.names);
    let toks = lex(&expanded).ok()?;
    // PY ( の中の引数を、通常の引数解析(範囲は形つき)で読む
    let resolved = HashMap::new();
    // PY セルの引数評価では ROW()/COLUMN() の「いまのセル」は分からない — 原点で代える
    let mut p = P { t: &toks, i: 0, sheet, resolved: &resolved, at: Pos::new(0, 0) };
    match (p.next(), p.next()) {
        (Some(Tok::Name(n)), Some(Tok::LParen)) if n == "PY" => {}
        _ => return None,
    }
    let args = p.args().ok()?;
    let mut it = args.into_iter();
    let name = match it.next()? {
        Arg::One(Value::Text(t)) => t,
        _ => return None, // 1つ目は関数名の文字でなければならない
    };
    let rest = it
        .map(|a| match a {
            Arg::One(v) => PyArg::One(v),
            Arg::Rect(c, vs) => PyArg::Rect(c, vs),
        })
        .collect();
    Some((name, rest))
}

// ---------- 再計算 ----------

/// 式が参照しているセルを集める(依存関係)。トレース(参照元の可視化)にも使う。
pub fn deps(formula: &str) -> Vec<Pos> {
    let mut out = Vec::new();
    if let Ok(toks) = lex(formula) {
        for t in toks {
            match t {
                Tok::Ref(p) => out.push(p),
                Tok::Range(a, z) => {
                    for r in a.row.min(z.row)..=a.row.max(z.row) {
                        for c in a.col.min(z.col)..=a.col.max(z.col) {
                            out.push(Pos::new(r, c));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// 式の中の「名前」を参照に置き換える(=単価*2 → =A1*2)。
/// 文字列の中は触らない。名前の前後が識別子の続きなら置き換えない。
/// 長い名前から先に試す(「単価」と「単価計」を取り違えない)。
fn expand_names(f: &str, names: &[(String, String)]) -> String {
    if names.is_empty() {
        return f.to_string();
    }
    let mut sorted: Vec<&(String, String)> = names.iter().collect();
    sorted.sort_by_key(|(n, _)| std::cmp::Reverse(n.chars().count()));
    let ch: Vec<char> = f.chars().collect();
    let ident = |c: char| c.is_alphanumeric() || c == '_';
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
        // 識別子の途中からは始めない
        let prev_ident = i > 0 && ident(ch[i - 1]);
        if !prev_ident {
            let mut hit = None;
            for (n, r) in &sorted {
                let nc: Vec<char> = n.chars().collect();
                if !nc.is_empty() && ch[i..].starts_with(&nc[..]) {
                    let after = ch.get(i + nc.len()).copied();
                    if !after.map(ident).unwrap_or(false) {
                        hit = Some((nc.len(), r.clone()));
                        break;
                    }
                }
            }
            if let Some((len, r)) = hit {
                out.push_str(&r);
                i += len;
                continue;
            }
        }
        out.push(ch[i]);
        i += 1;
    }
    out
}

/// シート全体を再計算する。循環参照は #CIRC! にする(黙って0にしない)。
/// この式は PY セルか(=PY("関数", …) が**単独で**立っている)。
/// PY は普通の再計算では**実行しない** — 「開く=実行」を持たないため。
/// 「PY(…)+1」のような複合式は PY セルではない(そちらは #PY単独 になる)。
pub fn is_py_formula(f: &str) -> bool {
    let Ok(toks) = lex(f) else { return false };
    let mut it = toks.iter();
    if !matches!(it.next(), Some(Tok::Name(n)) if n == "PY") {
        return false;
    }
    if !matches!(it.next(), Some(Tok::LParen)) {
        return false;
    }
    // 括弧の釣り合いが最後のトークンでちょうど閉じること
    let mut depth = 1i32;
    for (i, t) in it.enumerate() {
        match t {
            Tok::LParen => depth += 1,
            Tok::RParen => {
                depth -= 1;
                if depth == 0 {
                    return i + 3 == toks.len(); // これが末尾でなければ複合式
                }
            }
            _ => {}
        }
    }
    false
}

pub fn recalc(sheet: &mut Sheet) {
    // PY セルはここでは計算しない(最後に計算した値を保つ)。
    // まだ一度も計算していなければ「#PY?」の印を置く(空白で誤魔化さない)
    let py_cells: Vec<Pos> = sheet
        .cells
        .iter()
        .filter_map(|(p, c)| {
            c.formula.as_ref().filter(|f| is_py_formula(f)).map(|_| *p)
        })
        .collect();
    for p in &py_cells {
        if let Some(c) = sheet.cells.get_mut(p) {
            if c.value.is_empty() {
                c.value = Value::Error("#PY?".into());
            }
        }
    }
    let formulas: Vec<(Pos, String)> = sheet
        .cells
        .iter()
        .filter_map(|(p, c)| {
            c.formula
                .as_ref()
                .filter(|f| !is_py_formula(f))
                .map(|f| (*p, expand_names(f, &sheet.names)))
        })
        .collect();

    let mut resolved: HashMap<Pos, Value> = HashMap::new();
    let mut visiting: HashSet<Pos> = HashSet::new();

    fn eval_at(
        p: Pos,
        map: &HashMap<Pos, String>,
        sheet: &Sheet,
        resolved: &mut HashMap<Pos, Value>,
        visiting: &mut HashSet<Pos>,
    ) -> Value {
        if let Some(v) = resolved.get(&p) {
            return v.clone();
        }
        let Some(f) = map.get(&p) else {
            return sheet.value(p);
        };
        if !visiting.insert(p) {
            return Value::Error("#CIRC!".into());
        }
        // 先に依存を解く
        for d in deps(f) {
            if map.contains_key(&d) && !resolved.contains_key(&d) {
                let v = eval_at(d, map, sheet, resolved, visiting);
                resolved.insert(d, v);
            }
        }
        let v = match lex(f) {
            Ok(toks) => {
                let mut p2 = P { t: &toks, i: 0, sheet, resolved, at: p };
                match p2.expr() {
                    Ok(v) if p2.i == toks.len() => v,
                    Ok(_) => Value::Error("#ERROR!".into()),
                    Err(_) => Value::Error("#ERROR!".into()),
                }
            }
            Err(_) => Value::Error("#ERROR!".into()),
        };
        visiting.remove(&p);
        resolved.insert(p, v.clone());
        v
    }

    let map: HashMap<Pos, String> = formulas.iter().cloned().collect();
    for (p, _) in &formulas {
        let v = eval_at(*p, &map, sheet, &mut resolved, &mut visiting);
        resolved.insert(*p, v);
    }
    for (p, v) in resolved {
        if let Some(c) = sheet.cells.get_mut(&p) {
            if c.formula.is_some() {
                c.value = v;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Cell;

    fn s(pairs: &[(&str, &str)]) -> Sheet {
        let mut sh = Sheet::new("Sheet1");
        for (a1, input) in pairs {
            sh.set(Pos::parse(a1).unwrap(), Cell::input(input));
        }
        recalc(&mut sh);
        sh
    }
    fn v(sh: &Sheet, a1: &str) -> String {
        sh.value(Pos::parse(a1).unwrap()).display()
    }

    #[test]
    fn 四則と括弧() {
        let sh = s(&[("A1", "=1+2*3"), ("A2", "=(1+2)*3"), ("A3", "=10/4"),
                     ("A4", "=2^10"), ("A5", "=-3+1")]);
        assert_eq!(v(&sh, "A1"), "7");
        assert_eq!(v(&sh, "A2"), "9");
        assert_eq!(v(&sh, "A3"), "2.5");
        assert_eq!(v(&sh, "A4"), "1024");
        assert_eq!(v(&sh, "A5"), "-2");
    }

    #[test]
    fn セル参照と連鎖が解ける() {
        // 定義の順序が逆でも解ける(依存を先に解く)
        let sh = s(&[("C1", "=B1*2"), ("B1", "=A1+10"), ("A1", "5")]);
        assert_eq!(v(&sh, "B1"), "15");
        assert_eq!(v(&sh, "C1"), "30");
    }

    #[test]
    fn 範囲と関数() {
        let sh = s(&[("A1", "10"), ("A2", "20"), ("A3", "30"), ("A4", "文字"),
                     ("B1", "=SUM(A1:A3)"), ("B2", "=AVERAGE(A1:A3)"),
                     ("B3", "=COUNT(A1:A4)"), ("B4", "=COUNTA(A1:A4)"),
                     ("B5", "=MAX(A1:A3)"), ("B6", "=MIN(A1:A3)")]);
        assert_eq!(v(&sh, "B1"), "60");
        assert_eq!(v(&sh, "B2"), "20");
        assert_eq!(v(&sh, "B3"), "3", "COUNT は数値だけ数える");
        assert_eq!(v(&sh, "B4"), "4", "COUNTA は空でないものを数える");
        assert_eq!(v(&sh, "B5"), "30");
        assert_eq!(v(&sh, "B6"), "10");
    }

    #[test]
    fn 外した検索をiferrorで受けられる() {
        // 実測で出た形: 見つからない VLOOKUP を IFERROR・IF で受ける
        let sh = s(&[
            ("A2", "りんご"), ("B2", "100"),
            ("A3", "みかん"), ("B3", "80"),
            ("C1", "=IFERROR(VLOOKUP(\"zzz\",A2:B3,2),\"\")"),
            ("C2", "=IFERROR(VLOOKUP(\"みかん\",A2:B3,2),\"\")"),
            ("C3", "=IF(ISBLANK(G4),\"\",VLOOKUP(\"zzz\",A2:B3,2))"),
        ]);
        assert_eq!(v(&sh, "C1"), "", "外れたら第2引数に落ちる");
        assert_eq!(v(&sh, "C2"), "80", "当たればそのまま");
        assert_eq!(v(&sh, "C3"), "", "使わない側のエラーを踏まない");
    }

    #[test]
    fn 見積書の計算ができる() {
        // 事務で実際に使う形: 単価×数量、小計、消費税、合計
        let sh = s(&[
            ("A1", "ザボガードF F-02"), ("B1", "4"), ("C1", "125000"), ("D1", "=B1*C1"),
            ("A2", "エンブM"),          ("B2", "2"), ("C2", "98000"),  ("D2", "=B2*C2"),
            ("D3", "=SUM(D1:D2)"),
            ("D4", "=ROUND(D3*0.1,0)"),
            ("D5", "=D3+D4"),
        ]);
        assert_eq!(v(&sh, "D1"), "500000");
        assert_eq!(v(&sh, "D2"), "196000");
        assert_eq!(v(&sh, "D3"), "696000");
        assert_eq!(v(&sh, "D4"), "69600", "消費税");
        assert_eq!(v(&sh, "D5"), "765600", "税込合計");
    }

    #[test]
    fn 条件と文字列() {
        let sh = s(&[("A1", "12"), ("B1", "=IF(A1>10,\"超過\",\"適正\")"),
                     ("B2", "=IF(A1>100,\"超過\",\"適正\")"),
                     ("B3", "=\"H\"&A1&\"まで\""),
                     ("B4", "=CONCATENATE(\"合計\",A1,\"枚\")"),
                     ("B5", "=LEN(\"日本フネン\")")]);
        assert_eq!(v(&sh, "B1"), "超過");
        assert_eq!(v(&sh, "B2"), "適正");
        assert_eq!(v(&sh, "B3"), "H12まで");
        assert_eq!(v(&sh, "B4"), "合計12枚");
        assert_eq!(v(&sh, "B5"), "5", "日本語は文字数で数える");
    }

    #[test]
    fn ゼロ除算はエラーになる() {
        let sh = s(&[("A1", "0"), ("B1", "=10/A1")]);
        assert_eq!(v(&sh, "B1"), "#DIV/0!", "黙って0を返さない");
    }

    #[test]
    fn 循環参照は検出される() {
        let sh = s(&[("A1", "=B1+1"), ("B1", "=A1+1")]);
        assert!(v(&sh, "A1").contains("CIRC") || v(&sh, "B1").contains("CIRC"),
            "循環を検出していない: A1={} B1={}", v(&sh, "A1"), v(&sh, "B1"));
    }

    #[test]
    fn 知らない関数は名前エラー() {
        // XLOOKUP も実装済みになった(2026-08-05)ので、本当に無い名前で確かめる
        let sh = s(&[("A1", "=NAINAMAE(1,B1:C9,2)")]);
        assert_eq!(v(&sh, "A1"), "#NAME?", "できないものはできないと言う");
    }

    #[test]
    fn 壊れた式でも落ちない() {
        for f in ["=1+", "=(1+2", "=SUM(", "=@#$", "=A1+"] {
            let sh = s(&[("A1", "1"), ("Z9", f)]);
            let got = v(&sh, "Z9");
            assert!(got.starts_with('#'), "{f} → {got}(エラー値になっていない)");
        }
    }
}

#[cfg(test)]
mod more_fn_tests {
    use crate::model::{Cell, Pos, Sheet, Value};

    fn eval(formula: &str, data: &[(&str, f64)]) -> Value {
        let mut s = Sheet { name: "表".into(), ..Default::default() };
        for (a1, n) in data {
            s.set(Pos::parse(a1).unwrap(), Cell {
                formula: None, value: Value::Number(*n), fmt: Default::default() });
        }
        let out = Pos::parse("Z1").unwrap();
        // 式は = を外して持つ約束(Cell::input と同じ形にする)
        s.set(out, Cell::input(formula));
        crate::recalc(&mut s);
        s.get(out).unwrap().value.clone()
    }

    fn n(formula: &str) -> f64 {
        match eval(formula, &[]) {
            Value::Number(x) => x,
            v => panic!("数でない: {v:?}"),
        }
    }

    #[test]
    fn 切り捨てと切り上げ() {
        assert!((n("=ROUNDDOWN(3.567,2)") - 3.56).abs() < 1e-9);
        assert!((n("=ROUNDUP(3.501,1)") - 3.6).abs() < 1e-9);
        // 負の数で符号が入れ替わらない
        assert!((n("=ROUNDUP(-3.501,1)") + 3.6).abs() < 1e-9);
        assert!((n("=ROUNDDOWN(-3.567,2)") + 3.56).abs() < 1e-9);
    }

    #[test]
    fn 剰余は0で割れない() {
        // 黙って0を返すと、集計が静かに狂う
        assert_eq!(eval("=MOD(10,0)", &[]), Value::Error("#DIV/0!".into()));
        assert!((n("=MOD(10,3)") - 1.0).abs() < 1e-9);
    }

    #[test]
    fn 負の数の平方根はエラー() {
        assert_eq!(eval("=SQRT(-1)", &[]), Value::Error("#NUM!".into()));
        assert!((n("=SQRT(9)") - 3.0).abs() < 1e-9);
    }

    #[test]
    fn 条件つきの合計() {
        let d = [("A1", 100.0), ("A2", 200.0), ("A3", 50.0)];
        assert!((match eval("=SUMIF(A1:A3,\">80\")", &d) {
            Value::Number(x) => x, v => panic!("{v:?}") } - 300.0).abs() < 1e-9);
        assert!((match eval("=COUNTIF(A1:A3,\">80\")", &d) {
            Value::Number(x) => x, v => panic!("{v:?}") } - 2.0).abs() < 1e-9);
    }

    #[test]
    fn 文字を切り出せる() {
        // 日本語は文字数で数える(バイトではない)
        assert_eq!(eval("=LEFT(\"日本フネン\",2)", &[]), Value::Text("日本".into()));
        assert_eq!(eval("=RIGHT(\"日本フネン\",3)", &[]), Value::Text("フネン".into()));
        // MID は1始まり
        assert_eq!(eval("=MID(\"日本フネン\",3,2)", &[]), Value::Text("フネ".into()));
    }

    #[test]
    fn 空とエラーを見分けられる() {
        assert_eq!(eval("=ISBLANK(A9)", &[]), Value::Bool(true));
        assert_eq!(eval("=ISBLANK(A1)", &[("A1", 5.0)]), Value::Bool(false));
    }

    #[test]
    fn エラーを受けて働く関数() {
        // IFERROR は第1引数のエラーを捕まえて第2引数に落ちる
        // (以前は引数の先行エラー弾きで #N/A が素通りしていた)
        assert_eq!(eval("=IFERROR(MOD(1,0),\"×\")", &[]), Value::Text("×".into()));
        assert_eq!(eval("=IFERROR(A1,\"×\")", &[("A1", 5.0)]), Value::Number(5.0));
        // ISERROR も同じ弾きで壊れていた(エラーを見て TRUE を返せなかった)
        assert_eq!(eval("=ISERROR(MOD(1,0))", &[]), Value::Bool(true));
        assert_eq!(eval("=ISERROR(1)", &[]), Value::Bool(false));
        // IF は選ばなかった側のエラーを踏まない。条件のエラーは伝える
        assert_eq!(eval("=IF(1,\"可\",MOD(1,0))", &[]), Value::Text("可".into()));
        assert_eq!(eval("=IF(0,MOD(1,0),\"否\")", &[]), Value::Text("否".into()));
        assert_eq!(eval("=IF(MOD(1,0),1,2)", &[]), Value::Error("#DIV/0!".into()));
        // 選んだ側がエラーならそのまま伝える
        assert_eq!(eval("=IF(1,MOD(1,0),\"否\")", &[]), Value::Error("#DIV/0!".into()));
    }

    #[test]
    fn 積と累乗() {
        assert!((n("=PRODUCT(2,3,4)") - 24.0).abs() < 1e-9);
        assert!((n("=POWER(2,10)") - 1024.0).abs() < 1e-9);
    }

    #[test]
    fn 文字の整形() {
        assert_eq!(eval("=TRIM(\"  余白  \")", &[]), Value::Text("余白".into()));
        assert_eq!(eval("=UPPER(\"abc\")", &[]), Value::Text("ABC".into()));
    }
}

#[cfg(test)]
mod name_tests {
    use super::*;
    use crate::model::Cell;

    #[test]
    fn 名前が式で使える() {
        let mut s = Sheet::new("表");
        s.set(Pos::parse("A1").unwrap(), Cell::input("100"));
        s.set(Pos::parse("B1").unwrap(), Cell::input("=単価*2"));
        s.names.push(("単価".into(), "A1".into()));
        recalc(&mut s);
        assert_eq!(s.value(Pos::parse("B1").unwrap()), Value::Number(200.0),
            "名前が参照に展開されない");
    }

    #[test]
    fn 範囲の名前がsumで使える() {
        let mut s = Sheet::new("表");
        for (r, v) in [(0, "10"), (1, "20"), (2, "30")] {
            s.set(Pos::new(r, 0), Cell::input(v));
        }
        s.set(Pos::new(3, 0), Cell::input("=SUM(明細)"));
        s.names.push(("明細".into(), "A1:A3".into()));
        recalc(&mut s);
        assert_eq!(s.value(Pos::new(3, 0)), Value::Number(60.0));
    }

    #[test]
    fn 名前の途中一致では置き換えない() {
        assert_eq!(expand_names("単価計*2", &[("単価".into(), "A1".into())]),
            "単価計*2", "「単価計」の頭だけ置き換えた");
        assert_eq!(expand_names("\"単価\"&A1", &[("単価".into(), "B9".into())]),
            "\"単価\"&A1", "文字列の中を置き換えた");
        // 長い名前が勝つ
        assert_eq!(expand_names("単価計", &[
            ("単価".into(), "A1".into()), ("単価計".into(), "B1".into())]), "B1");
    }
}

#[cfg(test)]
mod fn_ext_tests {
    use super::*;
    use crate::model::Cell;

    fn sheet_with(cells: &[(&str, &str)]) -> Sheet {
        let mut s = Sheet { name: "表".into(), ..Default::default() };
        for (a1, v) in cells {
            s.set(Pos::parse(a1).unwrap(), Cell::input(v));
        }
        s
    }

    fn value_of(s: &mut Sheet, f: &str) -> Value {
        s.set(Pos::parse("Z99").unwrap(), Cell::input(f));
        recalc(s);
        s.value(Pos::parse("Z99").unwrap())
    }

    #[test]
    fn vlookupで表が引ける() {
        let mut s = sheet_with(&[
            ("A1", "甲"), ("B1", "100"),
            ("A2", "乙"), ("B2", "200"),
            ("A3", "丙"), ("B3", "300"),
        ]);
        assert_eq!(value_of(&mut s, "=VLOOKUP(\"乙\",A1:B3,2)"), Value::Number(200.0));
        assert_eq!(
            value_of(&mut s, "=VLOOKUP(\"丁\",A1:B3,2)"),
            Value::Error("#N/A".into()),
            "無い鍵は正直に #N/A"
        );
    }

    #[test]
    fn indexとmatchが組で使える() {
        let mut s = sheet_with(&[
            ("A1", "品"), ("B1", "数"),
            ("A2", "筆"), ("B2", "12"),
            ("A3", "紙"), ("B3", "34"),
        ]);
        assert_eq!(value_of(&mut s, "=MATCH(\"紙\",A1:A3,0)"), Value::Number(3.0));
        assert_eq!(value_of(&mut s, "=INDEX(A1:B3,3,2)"), Value::Number(34.0));
        assert_eq!(
            value_of(&mut s, "=INDEX(B1:B3,MATCH(\"筆\",A1:A3,0))"),
            Value::Number(12.0),
            "INDEX+MATCH の常套が動かない"
        );
    }

    #[test]
    fn 日付の通し番号が暦と往復する() {
        let mut s = sheet_with(&[]);
        // 2026-08-04 の通し番号(1899-12-30 起点)
        let serial = match value_of(&mut s, "=DATE(2026,8,4)") {
            Value::Number(n) => n,
            v => panic!("数でない: {v:?}"),
        };
        assert_eq!(value_of(&mut s, &format!("=YEAR({serial})")), Value::Number(2026.0));
        assert_eq!(value_of(&mut s, &format!("=MONTH({serial})")), Value::Number(8.0));
        assert_eq!(value_of(&mut s, &format!("=DAY({serial})")), Value::Number(4.0));
        // 2026-08-04 は火曜(Excel の既定: 日=1 → 火=3)
        assert_eq!(value_of(&mut s, &format!("=WEEKDAY({serial})")), Value::Number(3.0));
        // 既知の値: 1900-01-01 = 2
        assert_eq!(value_of(&mut s, "=DATE(1900,1,1)"), Value::Number(2.0));
    }

    #[test]
    fn 財務の式が教科書の値になる() {
        let mut s = sheet_with(&[]);
        // 年利12%を月利1%、60回、100万円借入 → 月々の返済(教科書値 -22244.45…)
        let pmt = match value_of(&mut s, "=PMT(0.01,60,1000000)") {
            Value::Number(n) => n,
            v => panic!("数でない: {v:?}"),
        };
        assert!((pmt + 22244.45).abs() < 0.5, "PMT が教科書とずれる: {pmt}");
        // 利率0なら単純割り
        assert_eq!(value_of(&mut s, "=PMT(0,10,1000)"), Value::Number(-100.0));
        // FV: 毎月1万円・月利0.5%・12回
        let fv = match value_of(&mut s, "=FV(0.005,12,-10000)") {
            Value::Number(n) => n,
            v => panic!("数でない: {v:?}"),
        };
        assert!((fv - 123355.62).abs() < 1.0, "FV がずれる: {fv}");
    }
}

/// 第1段の拡充(2026-08-05)— 日常と帳票を閉じる約37個。
#[cfg(test)]
mod dan1_tests {
    use super::*;
    use crate::model::Cell;

    fn sheet_with(cells: &[(&str, &str)]) -> Sheet {
        let mut s = Sheet { name: "表".into(), ..Default::default() };
        for (a1, v) in cells {
            s.set(Pos::parse(a1).unwrap(), Cell::input(v));
        }
        s
    }

    fn value_of(s: &mut Sheet, f: &str) -> Value {
        s.set(Pos::parse("Z99").unwrap(), Cell::input(f));
        recalc(s);
        s.value(Pos::parse("Z99").unwrap())
    }

    fn n(s: &mut Sheet, f: &str) -> f64 {
        match value_of(s, f) {
            Value::Number(x) => x,
            v => panic!("{f} が数でない: {v:?}"),
        }
    }

    fn t(s: &mut Sheet, f: &str) -> String {
        match value_of(s, f) {
            Value::Text(x) => x,
            v => panic!("{f} が文字でない: {v:?}"),
        }
    }

    #[test]
    fn 条件が複数の集計() {
        // 台帳: 品名・区分・金額
        let mut s = sheet_with(&[
            ("A1", "筆"), ("B1", "文具"), ("C1", "100"),
            ("A2", "紙"), ("B2", "文具"), ("C2", "200"),
            ("A3", "机"), ("B3", "家具"), ("C3", "900"),
            ("A4", "筆"), ("B4", "文具"), ("C4", "150"),
        ]);
        assert_eq!(n(&mut s, "=SUMIFS(C1:C4,B1:B4,\"文具\",A1:A4,\"筆\")"), 250.0);
        assert_eq!(n(&mut s, "=COUNTIFS(B1:B4,\"文具\",C1:C4,\">120\")"), 2.0);
        assert_eq!(n(&mut s, "=AVERAGEIF(B1:B4,\"文具\",C1:C4)"), 150.0);
        assert_eq!(n(&mut s, "=AVERAGEIFS(C1:C4,B1:B4,\"文具\")"), 150.0);
        assert_eq!(n(&mut s, "=MINIFS(C1:C4,B1:B4,\"文具\")"), 100.0);
        assert_eq!(n(&mut s, "=MAXIFS(C1:C4,B1:B4,\"文具\")"), 200.0);
        // 1件も合わない MINIFS は 0(Excel の約束)、AVERAGEIF は #DIV/0!
        assert_eq!(n(&mut s, "=MINIFS(C1:C4,B1:B4,\"食品\")"), 0.0);
        assert_eq!(
            value_of(&mut s, "=AVERAGEIF(B1:B4,\"食品\",C1:C4)"),
            Value::Error("#DIV/0!".into())
        );
    }

    #[test]
    fn sumproductで掛けて足す() {
        let mut s = sheet_with(&[
            ("A1", "4"), ("B1", "100"),
            ("A2", "2"), ("B2", "250"),
        ]);
        assert_eq!(n(&mut s, "=SUMPRODUCT(A1:A2,B1:B2)"), 900.0);
        assert_eq!(
            value_of(&mut s, "=SUMPRODUCT(A1:A2,B1:B1)"),
            Value::Error("#VALUE!".into()),
            "大きさ違いを黙って計算しない"
        );
    }

    #[test]
    fn ifsとswitchとchoose() {
        let mut s = sheet_with(&[("A1", "85")]);
        assert_eq!(
            t(&mut s, "=IFS(A1>=90,\"秀\",A1>=80,\"優\",TRUE,\"可\")"),
            "優"
        );
        assert_eq!(
            value_of(&mut s, "=IFS(A1>=90,\"秀\")"),
            Value::Error("#N/A".into()),
            "どれも真でないなら正直に #N/A"
        );
        // 選ばなかった枝のエラー(1/0)を踏まない
        assert_eq!(t(&mut s, "=IFS(TRUE,\"良\",TRUE,1/0)"), "良");
        assert_eq!(t(&mut s, "=SWITCH(2,1,\"甲\",2,\"乙\",\"他\")"), "乙");
        assert_eq!(t(&mut s, "=SWITCH(9,1,\"甲\",2,\"乙\",\"他\")"), "他");
        assert_eq!(t(&mut s, "=CHOOSE(2,\"松\",\"竹\",\"梅\")"), "竹");
        assert_eq!(
            value_of(&mut s, "=CHOOSE(9,\"松\",\"竹\")"),
            Value::Error("#VALUE!".into())
        );
    }

    #[test]
    fn xlookupは完全一致で引く() {
        let mut s = sheet_with(&[
            ("A1", "F-01"), ("B1", "防火戸"),
            ("A2", "F-02"), ("B2", "防火ダンパー"),
        ]);
        assert_eq!(t(&mut s, "=XLOOKUP(\"F-02\",A1:A2,B1:B2)"), "防火ダンパー");
        assert_eq!(
            value_of(&mut s, "=XLOOKUP(\"F-09\",A1:A2,B1:B2)"),
            Value::Error("#N/A".into())
        );
        assert_eq!(t(&mut s, "=XLOOKUP(\"F-09\",A1:A2,B1:B2,\"該当なし\")"), "該当なし");
    }

    #[test]
    fn 日付の計算が暦どおり() {
        let mut s = sheet_with(&[]);
        // 2026-08-05 から: 月末・翌月・月をまたぐ日の丸め
        assert_eq!(
            n(&mut s, "=EOMONTH(DATE(2026,8,5),0)"),
            n(&mut s, "=DATE(2026,8,31)")
        );
        assert_eq!(
            n(&mut s, "=EDATE(DATE(2026,8,5),1)"),
            n(&mut s, "=DATE(2026,9,5)")
        );
        // 1/31 の1ヶ月後は 2/28(在らぬ 2/31 を作らない)
        assert_eq!(
            n(&mut s, "=EDATE(DATE(2026,1,31),1)"),
            n(&mut s, "=DATE(2026,2,28)")
        );
        // 12月から年をまたぐ
        assert_eq!(
            n(&mut s, "=EOMONTH(DATE(2026,12,1),0)"),
            n(&mut s, "=DATE(2026,12,31)")
        );
        assert_eq!(n(&mut s, "=DATEDIF(DATE(2020,4,1),DATE(2026,8,5),\"Y\")"), 6.0);
        assert_eq!(n(&mut s, "=DATEDIF(DATE(2026,5,1),DATE(2026,8,5),\"M\")"), 3.0);
        assert_eq!(n(&mut s, "=DATEDIF(DATE(2026,8,1),DATE(2026,8,5),\"D\")"), 4.0);
        assert_eq!(
            n(&mut s, "=DATEVALUE(\"2026/8/5\")"),
            n(&mut s, "=DATE(2026,8,5)")
        );
        assert_eq!(
            n(&mut s, "=DATEVALUE(\"2026年8月5日\")"),
            n(&mut s, "=DATE(2026,8,5)")
        );
        // 時刻
        assert_eq!(n(&mut s, "=TIME(6,0,0)"), 0.25);
        assert_eq!(n(&mut s, "=HOUR(TIME(18,30,45))"), 18.0);
        assert_eq!(n(&mut s, "=MINUTE(TIME(18,30,45))"), 30.0);
        assert_eq!(n(&mut s, "=SECOND(TIME(18,30,45))"), 45.0);
    }

    #[test]
    fn 営業日の計算() {
        let mut s = sheet_with(&[]);
        // 2026-08-05 は水曜。3営業日後は月曜(8/10)
        assert_eq!(
            n(&mut s, "=WORKDAY(DATE(2026,8,5),3)"),
            n(&mut s, "=DATE(2026,8,10)")
        );
        // 休みを教えれば飛ばす(8/10 を祝日に)
        assert_eq!(
            n(&mut s, "=WORKDAY(DATE(2026,8,5),3,DATE(2026,8,10))"),
            n(&mut s, "=DATE(2026,8,11)")
        );
        // 8/3(月)〜8/9(日)の平日は5日
        assert_eq!(
            n(&mut s, "=NETWORKDAYS(DATE(2026,8,3),DATE(2026,8,9))"),
            5.0
        );
    }

    #[test]
    fn 文字列の道具() {
        let mut s = sheet_with(&[]);
        assert_eq!(t(&mut s, "=SUBSTITUTE(\"防火戸の戸\",\"戸\",\"扉\")"), "防火扉の扉");
        assert_eq!(t(&mut s, "=SUBSTITUTE(\"防火戸の戸\",\"戸\",\"扉\",2)"), "防火戸の扉");
        assert_eq!(n(&mut s, "=FIND(\"戸\",\"防火戸の戸\")"), 3.0);
        assert_eq!(n(&mut s, "=FIND(\"戸\",\"防火戸の戸\",4)"), 5.0);
        assert_eq!(
            value_of(&mut s, "=FIND(\"X\",\"防火戸\")"),
            Value::Error("#VALUE!".into())
        );
        assert_eq!(n(&mut s, "=SEARCH(\"abc\",\"xxABCxx\")"), 3.0, "SEARCH は大小を見ない");
        assert_eq!(n(&mut s, "=VALUE(\"¥1,234\")"), 1234.0);
        assert_eq!(n(&mut s, "=VALUE(\"25%\")"), 0.25);
        assert_eq!(t(&mut s, "=TEXTJOIN(\"、\",TRUE,\"松\",\"\",\"竹\")"), "松、竹");
        assert_eq!(t(&mut s, "=TEXTJOIN(\"-\",FALSE,\"a\",\"\",\"b\")"), "a--b");
        assert_eq!(t(&mut s, "=REPT(\"は\",3)"), "ははは");
        assert_eq!(t(&mut s, "=CHAR(65)"), "A");
        assert_eq!(n(&mut s, "=CODE(\"A\")"), 65.0);
    }

    #[test]
    fn textが表示形式で描く() {
        let mut s = sheet_with(&[]);
        assert_eq!(t(&mut s, "=TEXT(DATE(2026,8,5),\"yyyy/m/d\")"), "2026/8/5");
        assert_eq!(t(&mut s, "=TEXT(DATE(2026,8,5),\"yyyy年m月d日\")"), "2026年8月5日");
        // 2026-08-05 は水曜
        assert_eq!(t(&mut s, "=TEXT(DATE(2026,8,5),\"aaa\")"), "水");
        assert_eq!(t(&mut s, "=TEXT(DATE(2026,8,5),\"aaaa\")"), "水曜日");
        assert_eq!(t(&mut s, "=TEXT(TIME(9,5,0),\"h:mm\")"), "9:05");
        assert_eq!(t(&mut s, "=TEXT(1234567,\"#,##0\")"), "1,234,567", "数の形式も同じ道");
        assert_eq!(t(&mut s, "=TEXT(0.25,\"0%\")"), "25%");
    }

    #[test]
    fn 位置を答える関数() {
        let mut s = sheet_with(&[("B2", "9")]);
        // Z99 で計算しているので、引数なしは自分の位置
        assert_eq!(n(&mut s, "=ROW()"), 99.0);
        assert_eq!(n(&mut s, "=COLUMN()"), 26.0);
        assert_eq!(n(&mut s, "=ROW(B2)"), 2.0);
        assert_eq!(n(&mut s, "=COLUMN(B2)"), 2.0);
        assert_eq!(n(&mut s, "=ROWS(A1:B3)"), 3.0);
        assert_eq!(n(&mut s, "=COLUMNS(A1:B3)"), 2.0);
    }

    #[test]
    fn 順位と大きい順() {
        let mut s = sheet_with(&[
            ("A1", "70"), ("A2", "90"), ("A3", "80"), ("A4", "90"),
        ]);
        assert_eq!(n(&mut s, "=LARGE(A1:A4,1)"), 90.0);
        assert_eq!(n(&mut s, "=LARGE(A1:A4,3)"), 80.0);
        assert_eq!(n(&mut s, "=SMALL(A1:A4,1)"), 70.0);
        assert_eq!(n(&mut s, "=RANK(80,A1:A4)"), 3.0, "同値の90が2つで80は3位");
        assert_eq!(n(&mut s, "=RANK(90,A1:A4)"), 1.0, "同値は同順位");
        assert_eq!(n(&mut s, "=RANK(70,A1:A4,1)"), 1.0, "昇順なら最小が1位");
        assert_eq!(
            value_of(&mut s, "=LARGE(A1:A4,9)"),
            Value::Error("#NUM!".into())
        );
    }
}

#[cfg(test)]
mod py_cell_tests {
    use super::*;
    use crate::model::Cell;

    #[test]
    fn pyセルは再計算で実行されず値を保つ() {
        let mut s = Sheet { name: "表".into(), ..Default::default() };
        s.set(Pos::parse("A1").unwrap(), Cell::input("10"));
        let mut py = Cell::input("=PY(\"倍\",A1)");
        py.value = Value::Number(20.0); // 前に計算した値
        s.set(Pos::parse("B1").unwrap(), py);
        s.set(Pos::parse("C1").unwrap(), Cell::input("=B1+5"));
        recalc(&mut s);
        assert_eq!(s.value(Pos::parse("B1").unwrap()), Value::Number(20.0), "PY の値が流された");
        assert_eq!(s.value(Pos::parse("C1").unwrap()), Value::Number(25.0), "下流が古い値を見ない");
        // 一度も計算していない PY は #PY? の印
        s.set(Pos::parse("D1").unwrap(), Cell::input("=PY(\"倍\",A1)"));
        recalc(&mut s);
        assert_eq!(s.value(Pos::parse("D1").unwrap()), Value::Error("#PY?".into()));
        // 式の途中の PY は正直に断る
        s.set(Pos::parse("E1").unwrap(), Cell::input("=PY(\"倍\",A1)+1"));
        recalc(&mut s);
        assert_eq!(s.value(Pos::parse("E1").unwrap()), Value::Error("#PY単独".into()));
    }

    #[test]
    fn pyの呼び出しが材料に解ける() {
        let mut s = Sheet { name: "表".into(), ..Default::default() };
        s.set(Pos::parse("A1").unwrap(), Cell::input("1"));
        s.set(Pos::parse("A2").unwrap(), Cell::input("2"));
        s.set(Pos::parse("B1").unwrap(), Cell::input("3"));
        s.set(Pos::parse("B2").unwrap(), Cell::input("4"));
        recalc(&mut s);
        let (name, args) =
            eval_py_call(&s, "PY(\"集計\", A1:B2, 10, \"甲\")").expect("解けない");
        assert_eq!(name, "集計");
        assert_eq!(args.len(), 3);
        match &args[0] {
            PyArg::Rect(cols, vs) => {
                assert_eq!(*cols, 2);
                assert_eq!(vs.len(), 4, "2x2 のはず");
            }
            _ => panic!("範囲が形を失った"),
        }
        match (&args[1], &args[2]) {
            (PyArg::One(Value::Number(n)), PyArg::One(Value::Text(t))) => {
                assert_eq!(*n, 10.0);
                assert_eq!(t, "甲");
            }
            _ => panic!("引数の型が違う"),
        }
    }
}

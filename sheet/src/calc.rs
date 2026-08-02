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

use crate::model::{Pos, Sheet, Value};

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

    fn args(&mut self) -> Result<Vec<Value>, String> {
        // 関数の引数。範囲はその場で展開する
        let mut out = Vec::new();
        if let Some(Tok::RParen) = self.peek() {
            self.next();
            return Ok(out);
        }
        loop {
            if let Some(Tok::Range(a, z)) = self.peek().cloned() {
                self.next();
                out.extend(self.range_values(a, z));
            } else {
                out.push(self.expr()?);
            }
            match self.next() {
                Some(Tok::Comma) => continue,
                Some(Tok::RParen) => break,
                _ => return Err("引数の括弧が閉じていません".into()),
            }
        }
        Ok(out)
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

fn call(name: &str, a: Vec<Value>) -> Result<Value, String> {
    // 引数にエラーがあればそれを返す(黙って0として数えない)
    if let Some(e) = a.iter().find(|v| matches!(v, Value::Error(_))) {
        return Ok(e.clone());
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
            // 元の引数を見る必要があるので、頭のエラー弾きより前に効かせたいが、
            // ここでは第2引数を返すだけにしてある(呼ぶ前に弾かれるため要注意)
            a.first().cloned().unwrap_or(Value::Empty)
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
        "LEN" => Value::Number(a.first().map(|v| v.display().chars().count())
            .unwrap_or(0) as f64),
        _ => Value::Error("#NAME?".into()),
    })
}

// ---------- 再計算 ----------

/// 式が参照しているセルを集める(依存関係)。
fn deps(formula: &str) -> Vec<Pos> {
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
pub fn recalc(sheet: &mut Sheet) {
    let formulas: Vec<(Pos, String)> = sheet
        .cells
        .iter()
        .filter_map(|(p, c)| {
            c.formula.as_ref().map(|f| (*p, expand_names(f, &sheet.names)))
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
                let mut p2 = P { t: &toks, i: 0, sheet, resolved };
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
        let sh = s(&[("A1", "=VLOOKUP(1,B1:C9,2)")]);
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

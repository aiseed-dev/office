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

/// シート全体を再計算する。循環参照は #CIRC! にする(黙って0にしない)。
pub fn recalc(sheet: &mut Sheet) {
    let formulas: Vec<(Pos, String)> = sheet
        .cells
        .iter()
        .filter_map(|(p, c)| c.formula.as_ref().map(|f| (*p, f.clone())))
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

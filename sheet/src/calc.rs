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

use crate::model::{format_value, Cell, Pos, Sheet, Value};

// ---------- 字句 ----------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ref(Pos),
    Range(Pos, Pos),
    /// 別のシートの参照(シート名, 始点, 終点)。1セルなら始点=終点。
    /// 値は**その時の値を写す**(位置では持ち帰れない — RefAns::Rect と同じ理屈)
    Sheet(String, Pos, Pos),
    /// 串刺し集計 `Sheet1:Sheet3!A1`(始めのシート, 終わりのシート, 始点, 終点)。
    /// **ブックの並び順**で2枚の間にある全シートの同じ場所を集める
    Sheet3(String, String, Pos, Pos),
    /// 構造化参照(表の名前 — `[@列]` のときは None, 列の名前, この行だけか)。
    /// `Table1[金額]` = その列のデータ本体、`[@金額]` = いまの行の同じ列
    Table(Option<String>, String, bool),
    Name(String),
    Op(char),
    Cmp(String),
    LParen,
    RParen,
    Comma,
}

/// 構造化参照 `Table1[金額]` / `Table1[@金額]` / `[@金額]` / `[金額]` を読む。
/// 読めたら (表の名前, 列の名前, この行だけか, 次の位置)。
/// `[[#見出し],[列]]` のような入れ子の形は**受けない**(None を返して
/// 式のエラーにする — 黙って違う範囲を読むより正直)
#[allow(clippy::type_complexity)]
fn lex_table_ref(b: &[char], i: usize) -> Option<(Option<String>, String, bool, usize)> {
    let (tbl, open) = if b.get(i) == Some(&'[') {
        (None, i) // 表の名前を省いた形 — いまのセルが入っている表
    } else {
        let mut j = i;
        let mut s = String::new();
        while j < b.len() && (b[j].is_alphanumeric() || b[j] == '_' || b[j] == '.') {
            s.push(b[j]);
            j += 1;
        }
        if s.is_empty() || b.get(j) != Some(&'[') {
            return None;
        }
        (Some(s), j)
    };
    // 中身は `]` まで。入れ子の `[` があれば受けない
    let mut k = open + 1;
    let mut inner = String::new();
    while k < b.len() && b[k] != ']' {
        if b[k] == '[' {
            return None;
        }
        inner.push(b[k]);
        k += 1;
    }
    if k >= b.len() {
        return None; // 閉じていない
    }
    let this_row = inner.starts_with('@');
    let col = inner.trim_start_matches('@').trim().to_string();
    // `#見出し` などの特別な名前は受けない(上と同じ理由)
    if col.is_empty() || col.starts_with('#') {
        return None;
    }
    Some((tbl, col, this_row, k + 1))
}

/// `Sheet2!A1` / `売上!A1:B3` / `'4月 実績'!B2` を読む。
/// 読めたら (シート名, 始点, 終点, 次の位置)。**後ろに `!` があるときだけ**
/// 当たるので、既存の字句の読み方は変わらない(当たらなければ None で素通し)。
/// 和文のシート名(売上・4月)も通すため、名前の走査は Unicode の英数字で見る
#[allow(clippy::type_complexity)]
fn lex_sheet_ref(
    b: &[char],
    i: usize,
) -> Option<(String, Option<String>, Pos, Pos, usize)> {
    let (name, after) = if b.get(i) == Some(&'\'') {
        // 引用符つき = 空白や記号を含む名前('4月 実績'!B2)
        let mut j = i + 1;
        let mut s = String::new();
        while j < b.len() && b[j] != '\'' {
            s.push(b[j]);
            j += 1;
        }
        if j >= b.len() {
            return None; // 閉じていない — 既存の枝に任せる
        }
        (s, j + 1)
    } else {
        let mut j = i;
        let mut s = String::new();
        while j < b.len() && (b[j].is_alphanumeric() || b[j] == '_' || b[j] == '.') {
            s.push(b[j]);
            j += 1;
        }
        (s, j)
    };
    if name.is_empty() {
        return None;
    }
    // `Sheet1:Sheet3!A1` — 2枚目の名前(串刺し集計)
    let (name2, after) = if b.get(after) == Some(&':') {
        let mut j = after + 1;
        let mut s2 = String::new();
        if b.get(j) == Some(&'\'') {
            j += 1;
            while j < b.len() && b[j] != '\'' {
                s2.push(b[j]);
                j += 1;
            }
            if j >= b.len() {
                return None;
            }
            j += 1;
        } else {
            while j < b.len() && (b[j].is_alphanumeric() || b[j] == '_' || b[j] == '.') {
                s2.push(b[j]);
                j += 1;
            }
        }
        if s2.is_empty() {
            return None;
        }
        (Some(s2), j)
    } else {
        (None, after)
    };
    if b.get(after) != Some(&'!') {
        return None;
    }
    // `!` の後ろの A1 か A1:B3
    let one = |from: usize| -> (Option<Pos>, usize) {
        let mut j = from;
        while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == '$') {
            j += 1;
        }
        (Pos::parse(&b[from..j].iter().collect::<String>()), j)
    };
    let (a, j) = one(after + 1);
    let a = a?;
    if b.get(j) == Some(&':') {
        let (z, k) = one(j + 1);
        if let Some(z) = z {
            return Some((name, name2, a, z, k));
        }
    }
    Some((name, name2, a, a, j))
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
        // 別シートの参照は**数の枝より先**に見る(`4月!B2` のように
        // 数字で始まる名前があるため)。`!` が無ければ素通しする
        if let Some((name, name2, a, z, k)) = lex_sheet_ref(&b, i) {
            out.push(match name2 {
                Some(n2) => Tok::Sheet3(name, n2, a, z),
                None => Tok::Sheet(name, a, z),
            });
            i = k;
            continue;
        }
        // 構造化参照(表の列)。`[` が要るので、これも当たらなければ素通し
        if let Some((tbl, col, this_row, k)) = lex_table_ref(&b, i) {
            out.push(Tok::Table(tbl, col, this_row));
            i = k;
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
            // ATAN2 や LOG10 は「ATAN 列の 2 行目」とも読めてしまう —
            // **直後が ( なら関数名**(セル参照を関数のようには呼べない)
            let called = b.get(i).copied() == Some('(');
            match Pos::parse(&word) {
                Some(p) if !called => out.push(Tok::Ref(p)),
                _ => out.push(Tok::Name(word.to_ascii_uppercase())),
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
    /// 同じブックの他のシート(読み取りだけ)。INDIRECT("別の表!A1") が引く。
    /// recalc(1枚だけ)では空 — そのときの別シート参照は #REF!
    others: &'a [&'a Sheet],
    /// 自分がブックの何枚目か。**others は「自分より前」+「自分より後」の
    /// 並びなので、これが無いとブックの並び順を復元できない**(串刺し集計が使う)
    sheet_at: usize,
    /// 範囲を読むとき、**手で隠した行(row_hidden)を飛ばす**。
    /// SUBTOTAL/AGGREGATE の 101〜111 の間だけ立つ(Excel の約束)
    skip_hidden: std::cell::Cell<bool>,
    /// LET が束ねた名前(大文字で持つ)。**後ろが勝ち**= 入れ子や
    /// 同じ名前の付け直しで内側が外側を隠す
    lets: Vec<(String, Value)>,
}

/// 参照を計算する関数(OFFSET/INDIRECT)の答え。
/// 別のシートの中身は「位置」では持ち帰れないので、値の形も持つ
enum RefAns {
    /// このシートの範囲
    At(Pos, Pos),
    /// 別のシートから写した値(列数, 行優先)
    Rect(u32, Vec<Value>),
    /// 参照として成り立たない(エラー値をそのまま出す)
    Bad(Value),
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
        let skip = self.skip_hidden.get();
        let mut v = Vec::new();
        for r in r0..=r1 {
            // 101〜111 の SUBTOTAL/AGGREGATE のときだけ、隠した行は数に入れない
            if skip && self.sheet.row_hidden.contains(&r) {
                continue;
            }
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
        // 関数の引数は**配列数式として**読む — 範囲は形(列数)を保ち、
        // あふれる関数(FILTER 等)や要素ごとの演算(C1:C9>100、
        // SEQUENCE(3)*2)もそのまま並びとして渡る。
        // 1つの値に落ちるものは従来どおり1つの値
        let mut out = Vec::new();
        if let Some(Tok::RParen) = self.peek() {
            self.next();
            return Ok(out);
        }
        loop {
            let v = {
                let mut ap = AP { p: self };
                ap.expr()?
            };
            out.push(match v {
                AVal::One(x) => Arg::One(x),
                AVal::Arr(rows) => {
                    let w = rows.iter().map(|r| r.len()).max().unwrap_or(0).max(1);
                    let mut vals = Vec::new();
                    for row in &rows {
                        for c in 0..w {
                            vals.push(row.get(c).cloned().unwrap_or(Value::Empty));
                        }
                    }
                    Arg::Rect(w as u32, vals)
                }
            });
            match self.next() {
                Some(Tok::Comma) => continue,
                Some(Tok::RParen) => break,
                _ => return Err("引数の括弧が閉じていません".into()),
            }
        }
        Ok(out)
    }

    /// 別のシートの範囲を答える。直書きの `Sheet2!A1` と
    /// `INDIRECT("Sheet2!A1")` の両方がここを通る(道を1本にする)。
    /// 自分のシート名なら普通の範囲、知らない名前と1枚だけの計算では #REF!
    fn sheet_ans(&self, name: &str, a: Pos, z: Pos) -> RefAns {
        if name == self.sheet.name {
            return RefAns::At(a, z);
        }
        match self.others.iter().find(|s| s.name == name) {
            // 別のシートの値は**その時の値**を写す(位置では持ち帰れない)
            Some(other) => {
                let cols = a.col.abs_diff(z.col) + 1;
                let mut vals = Vec::new();
                for r in a.row.min(z.row)..=a.row.max(z.row) {
                    for c in a.col.min(z.col)..=a.col.max(z.col) {
                        vals.push(other.value(Pos::new(r, c)));
                    }
                }
                RefAns::Rect(cols, vals)
            }
            // 知らない名前・1枚だけの計算では #REF!(黙って自シートと読まない)
            None => RefAns::Bad(Value::Error("#REF!".into())),
        }
    }

    /// LET の中身。`(` の次から読み、閉じ括弧まで。
    /// 「名前 , 値」の組が続く限り束ね、組でなくなった所が本体の式
    fn let_body(&mut self) -> Result<Value, String> {
        loop {
            // 束縛の名前か? — **次が `,` のときだけ**名前として取る。
            // 本体が名前1つだけ(LET(x,1,x))なら次は `)` なので本体に回る
            let pair = matches!(
                (self.t.get(self.i), self.t.get(self.i + 1)),
                (Some(Tok::Name(_)), Some(Tok::Comma))
            );
            if pair {
                let Some(Tok::Name(n)) = self.t.get(self.i).cloned() else {
                    unreachable!("上で確かめた")
                };
                self.i += 2; // 名前と `,` を飛ばす
                let v = self.expr()?;
                self.lets.push((n, v));
                match self.next() {
                    Some(Tok::Comma) => continue,
                    // 名前と値で終わった = 本体の式が無い
                    _ => return Ok(Value::Error("#VALUE!".into())),
                }
            }
            let body = self.expr()?;
            return match self.next() {
                Some(Tok::RParen) => Ok(body),
                _ => Err("引数の括弧が閉じていません".into()),
            };
        }
    }

    /// 構造化参照を範囲に直す。表の名前(省いたらいまのセルが入っている表)と
    /// **見出しの字**で列を引く。見出し行の無い表からは引けない(None)
    fn table_range(&self, tbl: Option<&str>, col: &str, this_row: bool) -> Option<(Pos, Pos)> {
        let inside = |t: &crate::model::TableDef, p: Pos| {
            p.row >= t.a.row && p.row <= t.b.row && p.col >= t.a.col && p.col <= t.b.col
        };
        let t = match tbl {
            Some(n) => self.sheet.tables.iter().find(|t| t.name == n)?,
            None => self.sheet.tables.iter().find(|t| inside(t, self.at))?,
        };
        if !t.header {
            return None; // 見出しが無ければ列を名前で引けない
        }
        let c = (t.a.col..=t.b.col).find(|c| {
            self.sheet
                .get(Pos::new(t.a.row, *c))
                .map(|x| x.value.display())
                .unwrap_or_default()
                == col
        })?;
        if this_row {
            // いまの行の同じ列。表の外や見出しの行なら引けない
            if self.at.row <= t.a.row || self.at.row > t.b.row {
                return None;
            }
            return Some((Pos::new(self.at.row, c), Pos::new(self.at.row, c)));
        }
        // データ本体(見出しと合計行は外す)
        let r0 = t.a.row + 1;
        let r1 = if t.totals { t.b.row.checked_sub(1)? } else { t.b.row };
        (r0 <= r1).then_some((Pos::new(r0, c), Pos::new(r1, c)))
    }

    /// ブックの並び順のシート一覧(自分を含む)。串刺し集計が使う —
    /// others は「自分より前」+「自分より後」の並びなので、
    /// sheet_at の所へ自分を挿し戻せば元の順になる
    fn sheets_in_order(&self) -> Vec<&Sheet> {
        let k = self.sheet_at.min(self.others.len());
        let mut v: Vec<&Sheet> = Vec::with_capacity(self.others.len() + 1);
        v.extend(self.others[..k].iter().copied());
        v.push(self.sheet);
        v.extend(self.others[k..].iter().copied());
        v
    }

    /// 串刺し集計 `Sheet1:Sheet3!A1` — 並び順で2枚の間にある全シートの
    /// 同じ場所を集めて1つの並びにする。どちらかの名前が無ければ #REF!
    fn sheet3_ans(&self, from: &str, to: &str, a: Pos, z: Pos) -> RefAns {
        let all = self.sheets_in_order();
        let (Some(i), Some(j)) = (
            all.iter().position(|s| s.name == from),
            all.iter().position(|s| s.name == to),
        ) else {
            return RefAns::Bad(Value::Error("#REF!".into()));
        };
        let (i, j) = (i.min(j), i.max(j));
        let cols = a.col.abs_diff(z.col) + 1;
        let mut vals = Vec::new();
        for sh in &all[i..=j] {
            for r in a.row.min(z.row)..=a.row.max(z.row) {
                for c in a.col.min(z.col)..=a.col.max(z.col) {
                    let p = Pos::new(r, c);
                    // 自分のシートは計算途中の値(resolved)を見る
                    vals.push(if sh.name == self.sheet.name {
                        self.cell(p)
                    } else {
                        sh.value(p)
                    });
                }
            }
        }
        RefAns::Rect(cols, vals)
    }

    /// OFFSET / INDIRECT — **計算して決まる参照**。
    /// Ok(RefAns) / Err(構文エラー) の2層
    fn ref_call(&mut self, name: &str) -> Result<RefAns, String> {
        if name == "INDIRECT" {
            // INDIRECT(文字列) — "A1"・"A1:B2"・"別の表!A1" を受ける
            let v = self.expr()?;
            match self.next() {
                Some(Tok::RParen) => {}
                _ => return Err("引数の括弧が閉じていません".into()),
            }
            if let Value::Error(_) = v {
                return Ok(RefAns::Bad(v));
            }
            let s = v.display();
            let (sheet_name, rest) = match s.split_once('!') {
                Some((n, r)) => (Some(n.trim_matches('\'').to_string()), r.to_string()),
                None => (None, s),
            };
            let range = match rest.split_once(':') {
                Some((a, z)) => Pos::parse(a).zip(Pos::parse(z)),
                None => Pos::parse(&rest).map(|p| (p, p)),
            };
            let Some((a, z)) = range else {
                return Ok(RefAns::Bad(Value::Error("#REF!".into())));
            };
            return Ok(match sheet_name {
                None => RefAns::At(a, z),
                Some(n) => self.sheet_ans(&n, a, z),
            });
        }
        // OFFSET(基準, 行, 列, [高さ], [幅])
        let (a, z) = match self.next() {
            Some(Tok::Ref(p)) => (p, p),
            Some(Tok::Range(a, z)) => (a, z),
            _ => return Ok(RefAns::Bad(Value::Error("#VALUE!".into()))),
        };
        let mut vals = Vec::new();
        loop {
            match self.next() {
                Some(Tok::Comma) => vals.push(self.expr()?.as_number()),
                Some(Tok::RParen) => break,
                _ => return Err("引数の括弧が閉じていません".into()),
            }
        }
        if !(2..=4).contains(&vals.len()) {
            return Ok(RefAns::Bad(Value::Error("#VALUE!".into())));
        }
        let (r0, c0) = (a.row.min(z.row) as i64, a.col.min(z.col) as i64);
        let (dr, dc) = (vals[0] as i64, vals[1] as i64);
        let h = vals.get(2).map(|v| *v as i64).unwrap_or(i64::from(a.row.abs_diff(z.row)) + 1);
        let w = vals.get(3).map(|v| *v as i64).unwrap_or(i64::from(a.col.abs_diff(z.col)) + 1);
        let (nr, nc) = (r0 + dr, c0 + dc);
        // 表の外に出た参照は #REF!(Excel と同じ数え方の上限)
        if nr < 0 || nc < 0 || h < 1 || w < 1 || nr + h > 1_048_576 || nc + w > 16_384 {
            return Ok(RefAns::Bad(Value::Error("#REF!".into())));
        }
        Ok(RefAns::At(
            Pos::new(nr as u32, nc as u32),
            Pos::new((nr + h - 1) as u32, (nc + w - 1) as u32),
        ))
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
            Some(Tok::Sheet(name, a, z)) => Ok(match self.sheet_ans(&name, a, z) {
                RefAns::At(a, z) => {
                    self.range_values(a, z).into_iter().next().unwrap_or(Value::Empty)
                }
                RefAns::Rect(_, vals) => vals.into_iter().next().unwrap_or(Value::Empty),
                RefAns::Bad(v) => v,
            }),
            Some(Tok::Sheet3(from, to, a, z)) => {
                Ok(match self.sheet3_ans(&from, &to, a, z) {
                    RefAns::Rect(_, vals) => vals.into_iter().next().unwrap_or(Value::Empty),
                    RefAns::Bad(v) => v,
                    RefAns::At(a, z) => {
                        self.range_values(a, z).into_iter().next().unwrap_or(Value::Empty)
                    }
                })
            }
            Some(Tok::Table(tbl, col, this_row)) => {
                Ok(match self.table_range(tbl.as_deref(), &col, this_row) {
                    // 単独なら先頭の値([@列] は1セルなのでその値)
                    Some((a, z)) => {
                        self.range_values(a, z).into_iter().next().unwrap_or(Value::Empty)
                    }
                    None => Value::Error("#REF!".into()),
                })
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
                        // 計算して決まる参照。式の中では1セルの値として使う
                        if matches!(name.as_str(), "OFFSET" | "INDIRECT") {
                            return Ok(match self.ref_call(&name)? {
                                RefAns::At(a, z) if a == z => self.cell(a),
                                RefAns::Rect(_, vals) if vals.len() == 1 => {
                                    vals.into_iter().next().unwrap()
                                }
                                RefAns::Bad(v) => v,
                                _ => Value::Error("#VALUE!".into()),
                            });
                        }
                        // ふりがな。シートが持つ読み(xlsx の rPh)を引く。
                        // 読みが無ければセルの字そのもの(Excel と同じ約束)
                        if name == "PHONETIC" {
                            let (a, z) = match self.next() {
                                Some(Tok::Ref(p)) => (p, p),
                                Some(Tok::Range(a, z)) => (a, z),
                                _ => return Ok(Value::Error("#VALUE!".into())),
                            };
                            match self.next() {
                                Some(Tok::RParen) => {}
                                _ => return Err("引数の括弧が閉じていません".into()),
                            }
                            let mut out = String::new();
                            for r in a.row.min(z.row)..=a.row.max(z.row) {
                                for c in a.col.min(z.col)..=a.col.max(z.col) {
                                    let p = Pos::new(r, c);
                                    match self.sheet.phonetics.get(&p) {
                                        Some(ruby) => out.push_str(ruby),
                                        None => out.push_str(&self.cell(p).display()),
                                    }
                                }
                            }
                            return Ok(Value::Text(out));
                        }
                        // LET(名前, 値, [名前, 値]…, 式) — 名前を束ねてから
                        // 最後の式を計算する。値は**先に計算して**束ねるので、
                        // 後の束縛や本体から前の名前が見える(Excel と同じ)
                        if name == "LET" {
                            let depth = self.lets.len();
                            let r = self.let_body();
                            self.lets.truncate(depth); // 束縛は LET の中だけ
                            return r;
                        }
                        // SUBTOTAL/AGGREGATE の 101〜111 は「手で隠した行を
                        // 飛ばす」。番号は最初の引数なので、**読んでみてから**
                        // 100 超なら隠れ行を飛ばして読み直す(字句は残って
                        // いるので安い)。1〜11 は今までどおり全部数える
                        if matches!(name.as_str(), "SUBTOTAL" | "AGGREGATE") {
                            let save = self.i;
                            let args = self.args()?;
                            let n = args.first().map(|g| g.first().as_number()).unwrap_or(0.0);
                            if n > 100.0 && !self.sheet.row_hidden.is_empty() {
                                self.i = save;
                                self.skip_hidden.set(true);
                                let again = self.args();
                                self.skip_hidden.set(false);
                                return call(&name, again?);
                            }
                            return call(&name, args);
                        }
                        let args = self.args()?;
                        call(&name, args)
                    }
                    _ => match name.as_str() {
                        "TRUE" => Ok(Value::Bool(true)),
                        "FALSE" => Ok(Value::Bool(false)),
                        // LET が束ねた名前(後ろが勝ち = 内側が外側を隠す)
                        _ => match self.lets.iter().rev().find(|(n, _)| *n == name) {
                            Some((_, v)) => Ok(v.clone()),
                            None => Ok(Value::Error("#NAME?".into())),
                        },
                    },
                }
            }
            other => Err(format!("式が途中で終わっています: {other:?}")),
        }
    }
}

/// 配列数式の途中の値 — 1つの値か、2次元の並び。
enum AVal {
    One(Value),
    Arr(Vec<Vec<Value>>),
}

impl AVal {
    fn dims(&self) -> (usize, usize) {
        match self {
            AVal::One(_) => (1, 1),
            AVal::Arr(r) => (r.len(), r.iter().map(|x| x.len()).max().unwrap_or(0)),
        }
    }
    /// 要素を取る。1行・1列の側は引き伸ばす(Excel のブロードキャストと同じ)。
    /// 引き伸ばせない外側は #N/A(Excel の配列数式と同じ答え)
    fn at(&self, r: usize, c: usize) -> Value {
        match self {
            AVal::One(v) => v.clone(),
            AVal::Arr(rows) => {
                let (h, w) = self.dims();
                let rr = if h == 1 { 0 } else { r };
                let cc = if w == 1 { 0 } else { c };
                if rr >= h || cc >= w {
                    Value::Error("#N/A".into())
                } else {
                    rows[rr].get(cc).cloned().unwrap_or(Value::Empty)
                }
            }
        }
    }
}

/// 要素ごとの2項演算。両方が1つの値なら普通の計算、
/// 並びが混ざれば大きい方の形に広げて1要素ずつ
fn zip_aval(a: &AVal, b: &AVal, f: impl Fn(&Value, &Value) -> Value) -> AVal {
    if let (AVal::One(x), AVal::One(y)) = (a, b) {
        return AVal::One(f(x, y));
    }
    let (ha, wa) = a.dims();
    let (hb, wb) = b.dims();
    let (h, w) = (ha.max(hb), wa.max(wb));
    AVal::Arr(
        (0..h)
            .map(|r| {
                (0..w)
                    .map(|c| {
                        // エラーは伝播する(表計算の作法)
                        let x = a.at(r, c);
                        if let Value::Error(_) = x {
                            return x;
                        }
                        let y = b.at(r, c);
                        if let Value::Error(_) = y {
                            return y;
                        }
                        f(&x, &y)
                    })
                    .collect()
            })
            .collect(),
    )
}

/// 配列数式の評価器。文法は P と同じで、**演算が要素ごとに働く**。
/// =SEQUENCE(3)+1 のように、あふれる関数を四則・比較・& と組み合わせた
/// 式はこちらを通る(範囲もここでは並びとして扱う)
struct AP<'a, 'b> {
    p: &'b mut P<'a>,
}

impl AP<'_, '_> {
    fn expr(&mut self) -> Result<AVal, String> {
        let lhs = self.add()?;
        if let Some(Tok::Cmp(op)) = self.p.peek().cloned() {
            self.p.next();
            let rhs = self.add()?;
            return Ok(zip_aval(&lhs, &rhs, |x, y| Value::Bool(cmp_values(&op, x, y))));
        }
        Ok(lhs)
    }

    fn add(&mut self) -> Result<AVal, String> {
        let mut v = self.mul()?;
        while let Some(Tok::Op(o @ ('+' | '-' | '&'))) = self.p.peek().cloned() {
            self.p.next();
            let r = self.mul()?;
            v = zip_aval(&v, &r, |x, y| match o {
                '+' => Value::Number(x.as_number() + y.as_number()),
                '-' => Value::Number(x.as_number() - y.as_number()),
                _ => Value::Text(format!("{}{}", x.display(), y.display())),
            });
        }
        Ok(v)
    }

    fn mul(&mut self) -> Result<AVal, String> {
        let mut v = self.pow()?;
        while let Some(Tok::Op(o @ ('*' | '/'))) = self.p.peek().cloned() {
            self.p.next();
            let r = self.pow()?;
            v = zip_aval(&v, &r, |x, y| {
                if o == '/' && y.as_number() == 0.0 {
                    Value::Error("#DIV/0!".into())
                } else if o == '*' {
                    Value::Number(x.as_number() * y.as_number())
                } else {
                    Value::Number(x.as_number() / y.as_number())
                }
            });
        }
        Ok(v)
    }

    fn pow(&mut self) -> Result<AVal, String> {
        let v = self.unary()?;
        if let Some(Tok::Op('^')) = self.p.peek() {
            self.p.next();
            let r = self.pow()?;
            return Ok(zip_aval(&v, &r, |x, y| {
                Value::Number(x.as_number().powf(y.as_number()))
            }));
        }
        Ok(v)
    }

    fn unary(&mut self) -> Result<AVal, String> {
        match self.p.peek().cloned() {
            Some(Tok::Op('-')) => {
                self.p.next();
                let v = self.unary()?;
                Ok(zip_aval(&v, &AVal::One(Value::Number(0.0)), |x, _| match x {
                    e @ Value::Error(_) => e.clone(),
                    v => Value::Number(-v.as_number()),
                }))
            }
            Some(Tok::Op('+')) => {
                self.p.next();
                self.unary()
            }
            _ => self.atom(),
        }
    }

    fn atom(&mut self) -> Result<AVal, String> {
        match self.p.peek().cloned() {
            // 配列数式の中では、範囲は並びそのもの
            Some(Tok::Range(a, z)) => {
                self.p.next();
                let cols = (a.col.abs_diff(z.col) + 1) as usize;
                let vals = self.p.range_values(a, z);
                Ok(AVal::Arr(vals.chunks(cols.max(1)).map(|r| r.to_vec()).collect()))
            }
            // 串刺し集計も並びで渡す(=SUM(Sheet1:Sheet3!A1) が効く)
            Some(Tok::Sheet3(from, to, a, z)) => {
                self.p.next();
                Ok(match self.p.sheet3_ans(&from, &to, a, z) {
                    RefAns::Rect(cols, vals) => AVal::Arr(
                        vals.chunks((cols as usize).max(1)).map(|r| r.to_vec()).collect(),
                    ),
                    RefAns::Bad(v) => AVal::One(v),
                    RefAns::At(a, z) => {
                        let cols = (a.col.abs_diff(z.col) + 1) as usize;
                        let vals = self.p.range_values(a, z);
                        AVal::Arr(vals.chunks(cols.max(1)).map(|r| r.to_vec()).collect())
                    }
                })
            }
            // 構造化参照も並びで渡す(=SUM(Table1[金額]) が効く)
            Some(Tok::Table(tbl, col, this_row)) => {
                self.p.next();
                Ok(match self.p.table_range(tbl.as_deref(), &col, this_row) {
                    Some((a, z)) => {
                        let cols = (a.col.abs_diff(z.col) + 1) as usize;
                        let vals = self.p.range_values(a, z);
                        AVal::Arr(vals.chunks(cols.max(1)).map(|r| r.to_vec()).collect())
                    }
                    None => AVal::One(Value::Error("#REF!".into())),
                })
            }
            // 別シートの範囲も並びで渡す(=SUM(Sheet2!A1:A5) が効く)
            Some(Tok::Sheet(name, a, z)) => {
                self.p.next();
                Ok(match self.p.sheet_ans(&name, a, z) {
                    RefAns::At(a, z) => {
                        let cols = (a.col.abs_diff(z.col) + 1) as usize;
                        let vals = self.p.range_values(a, z);
                        AVal::Arr(vals.chunks(cols.max(1)).map(|r| r.to_vec()).collect())
                    }
                    RefAns::Rect(cols, vals) => AVal::Arr(
                        vals.chunks((cols as usize).max(1)).map(|r| r.to_vec()).collect(),
                    ),
                    RefAns::Bad(v) => AVal::One(v),
                })
            }
            Some(Tok::LParen) => {
                self.p.next();
                let v = self.expr()?;
                match self.p.next() {
                    Some(Tok::RParen) => Ok(v),
                    _ => Err("括弧が閉じていません".into()),
                }
            }
            Some(Tok::Name(n)) if self.p.t.get(self.p.i + 1) == Some(&Tok::LParen) => {
                if ARRAY_FNS.contains(&n.as_str()) {
                    // あふれる関数の呼び出し — 並びのまま持つ
                    self.p.next();
                    self.p.next();
                    let args = self.p.args()?;
                    Ok(match array_call(&n, args) {
                        Ok(rows) => AVal::Arr(rows),
                        Err(e) => AVal::One(e),
                    })
                } else if matches!(n.as_str(), "OFFSET" | "INDIRECT") {
                    self.p.next();
                    self.p.next();
                    Ok(match self.p.ref_call(&n)? {
                        RefAns::At(a, z) => {
                            let cols = (a.col.abs_diff(z.col) + 1) as usize;
                            let vals = self.p.range_values(a, z);
                            AVal::Arr(
                                vals.chunks(cols.max(1)).map(|r| r.to_vec()).collect())
                        }
                        RefAns::Rect(cols, vals) => AVal::Arr(
                            vals.chunks((cols as usize).max(1)).map(|r| r.to_vec()).collect()),
                        RefAns::Bad(v) => AVal::One(v),
                    })
                } else {
                    // 普通の関数は1つの値(集計は中で並びを受けている)
                    Ok(AVal::One(self.p.atom()?))
                }
            }
            _ => Ok(AVal::One(self.p.atom()?)),
        }
    }
}

/// 比較の中身。文字同士は文字として、それ以外は数として比べる
/// (式の比較と、範囲の要素ごとの比較が同じ規則を通る)
fn cmp_values(op: &str, lhs: &Value, rhs: &Value) -> bool {
    match (lhs, rhs) {
        (Value::Text(a), Value::Text(b)) => match op {
            "=" => a == b,
            "<>" => a != b,
            "<" => a < b,
            ">" => a > b,
            "<=" => a <= b,
            _ => a >= b,
        },
        _ => {
            let (a, b) = (lhs.as_number(), rhs.as_number());
            match op {
                "=" => a == b,
                "<>" => a != b,
                "<" => a < b,
                ">" => a > b,
                "<=" => a <= b,
                _ => a >= b,
            }
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

/// RAND 用の乱数(0.0 以上 1.0 未満)。暗号用ではない(表計算の RAND も同じ)。
/// 依存を増やさず xorshift64* を自前で持つ。種は最初の呼び出し時刻
fn rand01() -> f64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEED: AtomicU64 = AtomicU64::new(0);
    let mut x = SEED.load(Ordering::Relaxed);
    if x == 0 {
        x = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| u64::from(d.subsec_nanos()) ^ d.as_secs())
            .unwrap_or(0x9E37_79B9_7F4A_7C15)
            | 1;
    }
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    SEED.store(x, Ordering::Relaxed);
    (x >> 11) as f64 / (1u64 << 53) as f64
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

/// LENB 系の1文字の「バイト」数。全角=2、半角(ASCII と半角カナ)=1
/// (Excel の日本語ロケールと同じ数え方。実際の UTF-8 の長さではない)
fn jchar_width(c: char) -> usize {
    if c.is_ascii() || ('\u{FF61}'..='\u{FF9F}').contains(&c) {
        1
    } else {
        2
    }
}

/// 全角カタカナ ↔ 半角カナの対応表(並びを揃えてある)
const KANA_Z: &str = "ァィゥェォャュョッーアイウエオカキクケコサシスセソタチツテト\
                      ナニヌネノハヒフヘホマミムメモヤユヨラリルレロワヲン。「」、・";
const KANA_H: &str = "ｧｨｩｪｫｬｭｮｯｰｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜｦﾝ｡｢｣､･";
/// 濁点つき(→ 半角では2文字になる)
const DAKU_Z: &str = "ガギグゲゴザジズゼゾダヂヅデドバビブベボヴ";
const DAKU_H: &str = "ｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾊﾋﾌﾍﾎｳ";
const HANDAKU_Z: &str = "パピプペポ";
const HANDAKU_H: &str = "ﾊﾋﾌﾍﾎ";

/// ASC — 全角を半角へ(英数記号・空白・カタカナ)
fn asc_hankaku(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        // 全角英数記号(!〜~)は 0xFEE0 ずらすと半角
        if ('\u{FF01}'..='\u{FF5E}').contains(&c) {
            out.push(char::from_u32(c as u32 - 0xFEE0).unwrap_or(c));
        } else if c == '\u{3000}' {
            out.push(' ');
        } else if let Some(i) = DAKU_Z.chars().position(|z| z == c) {
            out.push(DAKU_H.chars().nth(i).unwrap());
            out.push('ﾞ');
        } else if let Some(i) = HANDAKU_Z.chars().position(|z| z == c) {
            out.push(HANDAKU_H.chars().nth(i).unwrap());
            out.push('ﾟ');
        } else if let Some(i) = KANA_Z.chars().position(|z| z == c) {
            out.push(KANA_H.chars().nth(i).unwrap());
        } else {
            out.push(c);
        }
    }
    out
}

/// JIS — 半角を全角へ(濁点は1文字に組む)
fn jis_zenkaku(s: &str) -> String {
    let ch: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < ch.len() {
        let c = ch[i];
        let next = ch.get(i + 1).copied();
        // 濁点・半濁点は前の字と組んで1文字へ
        if next == Some('ﾞ') {
            if let Some(k) = DAKU_H.chars().position(|h| h == c) {
                out.push(DAKU_Z.chars().nth(k).unwrap());
                i += 2;
                continue;
            }
        }
        if next == Some('ﾟ') {
            if let Some(k) = HANDAKU_H.chars().position(|h| h == c) {
                out.push(HANDAKU_Z.chars().nth(k).unwrap());
                i += 2;
                continue;
            }
        }
        if c.is_ascii_graphic() {
            out.push(char::from_u32(c as u32 + 0xFEE0).unwrap_or(c));
        } else if c == ' ' {
            out.push('\u{3000}');
        } else if let Some(k) = KANA_H.chars().position(|h| h == c) {
            out.push(KANA_Z.chars().nth(k).unwrap());
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

/// 元号。通し番号 → (名前, ローマ字の頭文字, 和暦の年)。明治より前は None。
/// DATESTRING と表示形式(g・e)が**同じ表**を使う
pub(crate) fn era_of(serial: i64) -> Option<(&'static str, &'static str, i64)> {
    let (y, _, _) = civil_from_days(serial - EXCEL_EPOCH_DAYS);
    let eras: [(i64, &'static str, &'static str, i64); 5] = [
        (date_serial(2019, 5, 1), "令和", "R", 2019),
        (date_serial(1989, 1, 8), "平成", "H", 1989),
        (date_serial(1926, 12, 25), "昭和", "S", 1926),
        (date_serial(1912, 7, 30), "大正", "T", 1912),
        (date_serial(1868, 10, 23), "明治", "M", 1868),
    ];
    for (start, name, initial, base) in eras {
        if serial >= start {
            return Some((name, initial, y - base + 1));
        }
    }
    None
}

/// 通し番号 → 和暦の文字(DATESTRING)。明治より前は西暦のまま
fn wareki(serial: i64) -> String {
    let (y, m, d) = civil_from_days(serial - EXCEL_EPOCH_DAYS);
    match era_of(serial) {
        Some((name, _, ey)) => format!("{name}{ey:02}年{m:02}月{d:02}日"),
        None => format!("{y}年{m:02}月{d:02}日"),
    }
}

/// 30/360(米国方式)の日数。DAYS360 と YEARFRAC が使う
fn days360(s: i64, e: i64) -> i64 {
    let (sy, sm, mut sd) = civil_from_days(s - EXCEL_EPOCH_DAYS);
    let (ey, em, mut ed) = civil_from_days(e - EXCEL_EPOCH_DAYS);
    if sd == 31 {
        sd = 30;
    }
    if ed == 31 && sd == 30 {
        ed = 30;
    }
    (ey - sy) * 360 + (em - sm) * 30 + (ed - sd)
}

/// 符号の変わる区間を粗く探して挟み撃ち(IRR・RATE の反復解)。
/// 見つからなければ None(#NUM! — 黙って0を返さない)
fn bisect(f: &dyn Fn(f64) -> f64, lo: f64, hi: f64) -> Option<f64> {
    let steps = 400;
    let mut px = lo;
    let mut py = f(lo);
    for i in 1..=steps {
        let x = lo + (hi - lo) * f64::from(i) / f64::from(steps);
        let y = f(x);
        if px.is_finite() && py.is_finite() && y.is_finite() && py * y <= 0.0 {
            let (mut a, mut b, mut fa) = (px, x, py);
            for _ in 0..200 {
                let m = (a + b) / 2.0;
                let fm = f(m);
                if fa * fm <= 0.0 {
                    b = m;
                } else {
                    a = m;
                    fa = fm;
                }
            }
            return Some((a + b) / 2.0);
        }
        px = x;
        py = y;
    }
    None
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
        "SUMIF" | "AVERAGEIF" => {
            // SUMIF(条件を見る範囲, 条件, [足す範囲])
            // AVERAGEIF(条件を見る範囲, 条件, [平均する範囲])
            // 3つ目を省いたら、条件を見た範囲そのものを足す(平均する)
            let rng = args.first().map(|g| g.values()).unwrap_or(&[]);
            let cond = args.get(1).map(|g| g.first()).unwrap_or(Value::Empty);
            let tgt = args.get(2).map(|g| g.values()).unwrap_or(rng);
            if tgt.len() != rng.len() {
                return Ok(Value::Error("#VALUE!".into()));
            }
            // 範囲にエラーがあればそれを返す(黙って0として数えない)
            if let Some(e) =
                rng.iter().chain(tgt).chain([&cond]).find(|v| matches!(v, Value::Error(_)))
            {
                return Ok(e.clone());
            }
            let picked: Vec<f64> = (0..rng.len())
                .filter(|i| matches_cond(&rng[*i], &cond))
                .map(|i| tgt[i].as_number())
                .collect();
            return Ok(if name == "SUMIF" {
                Value::Number(picked.iter().sum())
            } else if picked.is_empty() {
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
        "PERCENTILE" | "PERCENTILE.INC" | "QUARTILE" | "QUARTILE.INC" => {
            // 百分位(直線補間 — Excel の PERCENTILE.INC と同じ)
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
            let k = args.get(1).map(|g| g.first().as_number()).unwrap_or(f64::NAN);
            let k = if name.starts_with("QUARTILE") { k / 4.0 } else { k };
            if ns.is_empty() || !(0.0..=1.0).contains(&k) {
                return Ok(Value::Error("#NUM!".into()));
            }
            ns.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
            let pos = k * (ns.len() - 1) as f64;
            let (lo, frac) = (pos.floor() as usize, pos.fract());
            let hi = (lo + 1).min(ns.len() - 1);
            return Ok(Value::Number(ns[lo] + (ns[hi] - ns[lo]) * frac));
        }
        "CORREL" | "SLOPE" | "INTERCEPT" | "FORECAST" | "FORECAST.LINEAR" => {
            // 対で見る統計。両方が数の行だけを使う(Excel と同じ)
            let fc = name.starts_with("FORECAST");
            let (ys, xs) = if fc {
                (args.get(1), args.get(2))
            } else {
                (args.first(), args.get(1))
            };
            let (ys, xs) = (
                ys.map(|g| g.values()).unwrap_or(&[]),
                xs.map(|g| g.values()).unwrap_or(&[]),
            );
            if ys.len() != xs.len() {
                return Ok(Value::Error("#N/A".into()));
            }
            let pairs: Vec<(f64, f64)> = ys
                .iter()
                .zip(xs)
                .filter(|(y, x)| {
                    matches!(y, Value::Number(_)) && matches!(x, Value::Number(_))
                })
                .map(|(y, x)| (y.as_number(), x.as_number()))
                .collect();
            let n = pairs.len() as f64;
            if pairs.is_empty() {
                return Ok(Value::Error("#DIV/0!".into()));
            }
            let (my, mx) = (
                pairs.iter().map(|p| p.0).sum::<f64>() / n,
                pairs.iter().map(|p| p.1).sum::<f64>() / n,
            );
            let sxy: f64 = pairs.iter().map(|(y, x)| (x - mx) * (y - my)).sum();
            let sxx: f64 = pairs.iter().map(|(_, x)| (x - mx) * (x - mx)).sum();
            let syy: f64 = pairs.iter().map(|(y, _)| (y - my) * (y - my)).sum();
            return Ok(match name {
                "CORREL" => {
                    if sxx == 0.0 || syy == 0.0 {
                        Value::Error("#DIV/0!".into())
                    } else {
                        Value::Number(sxy / (sxx * syy).sqrt())
                    }
                }
                _ => {
                    if sxx == 0.0 {
                        Value::Error("#DIV/0!".into())
                    } else {
                        let slope = sxy / sxx;
                        match name {
                            "SLOPE" => Value::Number(slope),
                            "INTERCEPT" => Value::Number(my - slope * mx),
                            _ => {
                                let x = args.first().map(|g| g.first().as_number())
                                    .unwrap_or(0.0);
                                Value::Number(my - slope * mx + slope * x)
                            }
                        }
                    }
                }
            });
        }
        "RANK" | "RANK.EQ" | "RANK.AVG" => {
            // RANK(値, 範囲, [順序]) — 省略は大きい方が1位。
            // .EQ は同値同順位、.AVG は同値の順位の平均
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
            let ties = ns.iter().filter(|v| (*v - x).abs() < 1e-9).count();
            if ties == 0 {
                return Ok(Value::Error("#N/A".into()));
            }
            let better =
                ns.iter().filter(|v| if asc { **v < x - 1e-9 } else { **v > x + 1e-9 }).count();
            return Ok(Value::Number(if name == "RANK.AVG" {
                // 同値が k 個なら (r + r+1 + … + r+k-1) / k
                (better + 1) as f64 + (ties - 1) as f64 / 2.0
            } else {
                (better + 1) as f64
            }));
        }
        "IRR" => {
            // IRR(額の並び, [推定値]) — 符号の変わる区間を探して挟み撃ち(反復解)
            let vals: Vec<f64> = args
                .first()
                .map(|g| {
                    g.values()
                        .iter()
                        .filter(|v| matches!(v, Value::Number(_)))
                        .map(|v| v.as_number())
                        .collect()
                })
                .unwrap_or_default();
            let f = |r: f64| -> f64 {
                vals.iter().enumerate().map(|(i, v)| v / (1.0 + r).powi(i as i32)).sum()
            };
            return Ok(match bisect(&f, -0.9999, 10.0) {
                Some(r) => Value::Number(r),
                None => Value::Error("#NUM!".into()),
            });
        }
        "LOOKUP" => {
            // LOOKUP(値, 探す並び, [結果の並び]) — 昇順前提の古典。
            // 「値以下でいちばん大きいもの」を選ぶ(Excel と同じ)
            let key = args.first().map(|g| g.first()).unwrap_or(Value::Empty);
            let hay = args.get(1).map(|g| g.values()).unwrap_or(&[]);
            let ret = args.get(2).map(|g| g.values()).unwrap_or(hay);
            if hay.is_empty() || hay.len() != ret.len() {
                return Ok(Value::Error("#N/A".into()));
            }
            let mut hit: Option<usize> = None;
            for (i, v) in hay.iter().enumerate() {
                let le = match (v, &key) {
                    (Value::Number(a), Value::Number(b)) => a <= b,
                    (Value::Text(a), Value::Text(b)) => a.as_str() <= b.as_str(),
                    _ => false,
                };
                if le {
                    hit = Some(i);
                }
            }
            return Ok(hit
                .and_then(|i| ret.get(i).cloned())
                .unwrap_or(Value::Error("#N/A".into())));
        }
        "SUBTOTAL" | "AGGREGATE" => {
            // SUBTOTAL(集計番号, 範囲…) — オートフィルターと「小計」が
            // 自動で埋め込む、実物のファイル頻出の関数。
            // **101〜111(手で隠した行を飛ばす)は呼ぶ側で範囲から抜いてある**
            // (P::atom の二度読み。ここへ来る値は既に飛ばした後)。
            // 絞り込みで隠れた行は app 側の状態なので、まだ数に入る(在庫)
            let f = args.first().map(|g| g.first().as_number()).unwrap_or(0.0) as i64;
            let f = if f > 100 { f - 100 } else { f };
            // AGGREGATE は第2引数が「無視の指定」— 読み飛ばす
            let skip = if name == "AGGREGATE" { 2 } else { 1 };
            let ns: Vec<f64> = args
                .get(skip..)
                .unwrap_or(&[])
                .iter()
                .flat_map(|g| g.values())
                .filter(|v| matches!(v, Value::Number(_)))
                .map(|v| v.as_number())
                .collect();
            let cnt_a = args
                .get(skip..)
                .unwrap_or(&[])
                .iter()
                .flat_map(|g| g.values())
                .filter(|v| !v.is_empty())
                .count();
            let n = ns.len() as f64;
            let mean = if ns.is_empty() { 0.0 } else { ns.iter().sum::<f64>() / n };
            let ss: f64 = ns.iter().map(|x| (x - mean) * (x - mean)).sum();
            return Ok(match f {
                1 if !ns.is_empty() => Value::Number(mean),
                2 => Value::Number(n),
                3 => Value::Number(cnt_a as f64),
                4 => Value::Number(ns.iter().cloned().reduce(f64::max).unwrap_or(0.0)),
                5 => Value::Number(ns.iter().cloned().reduce(f64::min).unwrap_or(0.0)),
                6 => Value::Number(ns.iter().product()),
                7 if ns.len() >= 2 => Value::Number((ss / (n - 1.0)).sqrt()),
                8 if !ns.is_empty() => Value::Number((ss / n).sqrt()),
                9 => Value::Number(ns.iter().sum()),
                10 if ns.len() >= 2 => Value::Number(ss / (n - 1.0)),
                11 if !ns.is_empty() => Value::Number(ss / n),
                1 | 7 | 8 | 10 | 11 => Value::Error("#DIV/0!".into()),
                _ => Value::Error("#VALUE!".into()),
            });
        }
        _ => {}
    }
    let a: Vec<Value> = args.iter().flat_map(|g| g.values().iter().cloned()).collect();
    // 引数にエラーがあればそれを返す(黙って0として数えない)。
    // ただしエラーを受けて働く関数(IFERROR・ISERROR・ISBLANK・IF)と、
    // 選ばなかった枝のエラーを踏んではいけない関数(IFS・SWITCH・CHOOSE)は素通しする
    if !matches!(
        name,
        "IFERROR" | "ISERROR" | "ISBLANK" | "IF" | "IFS" | "SWITCH" | "CHOOSE"
            | "ISNUMBER" | "ISTEXT" // エラーは数でも文字でもない → FALSE と答える
            | "IFNA" | "ISNA" | "ISERR" | "ISLOGICAL" | "ISNONTEXT" | "TYPE" | "N" | "T"
    ) {
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
        "CONCATENATE" | "CONCAT" => Value::Text(a.iter().map(|v| v.display()).collect()),
        // 引数つきの TRUE()/FALSE() も本物の Excel ファイルには出てくる
        "TRUE" => Value::Bool(true),
        "FALSE" => Value::Bool(false),
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
        "TEXTBEFORE" | "TEXTAFTER" => {
            // TEXTBEFORE(文字, 区切り, [何番目], [見つからない時の値])
            // 何番目が負なら**後ろから**数える(Excel と同じ)。
            // 見つからなければ #N/A(4つ目を渡していればその値)
            let s = a.first().map(|v| v.display()).unwrap_or_default();
            let d = a.get(1).map(|v| v.display()).unwrap_or_default();
            let nth = a.get(2).map(|v| v.as_number() as i64).unwrap_or(1);
            let not_found = a.get(3).cloned();
            if d.is_empty() || nth == 0 {
                return Ok(Value::Error("#VALUE!".into()));
            }
            let hits: Vec<usize> = s.match_indices(d.as_str()).map(|(i, _)| i).collect();
            let idx = if nth > 0 {
                hits.get(nth as usize - 1).copied()
            } else {
                hits.iter().rev().nth((-nth) as usize - 1).copied()
            };
            match idx {
                Some(i) => Value::Text(if name == "TEXTBEFORE" {
                    s[..i].to_string()
                } else {
                    s[i + d.len()..].to_string()
                }),
                None => not_found.unwrap_or(Value::Error("#N/A".into())),
            }
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
        "CHAR" | "UNICHAR" => {
            let n = a.first().map(|v| v.as_number()).unwrap_or(0.0) as u32;
            match char::from_u32(n) {
                Some(c) if n > 0 => Value::Text(c.to_string()),
                _ => Value::Error("#VALUE!".into()),
            }
        }
        "CODE" | "UNICODE" => {
            match a.first().map(|v| v.display()).unwrap_or_default().chars().next() {
                Some(c) => Value::Number(c as u32 as f64),
                None => Value::Error("#VALUE!".into()),
            }
        }
        // ---- 統計(第2段 2026-08-05)----
        "MEDIAN" => {
            let mut ns = nums(&a);
            if ns.is_empty() {
                Value::Error("#NUM!".into())
            } else {
                ns.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
                let m = ns.len() / 2;
                Value::Number(if ns.len() % 2 == 1 { ns[m] } else { (ns[m - 1] + ns[m]) / 2.0 })
            }
        }
        "MODE" | "MODE.SNGL" => {
            // 最も多く現れる数。1度ずつしか現れないなら #N/A(Excel と同じ)
            let ns = nums(&a);
            let mut best: Option<(f64, usize)> = None;
            for (i, x) in ns.iter().enumerate() {
                let c = ns.iter().filter(|y| (*y - x).abs() < 1e-12).count();
                // 同数なら先に現れた方(Excel の MODE.SNGL の癖)
                let earlier = ns[..i].iter().any(|y| (y - x).abs() < 1e-12);
                if c > 1 && !earlier && best.map(|(_, bc)| c > bc).unwrap_or(true) {
                    best = Some((*x, c));
                }
            }
            best.map(|(x, _)| Value::Number(x)).unwrap_or(Value::Error("#N/A".into()))
        }
        "STDEV" | "STDEV.S" | "STDEVP" | "STDEV.P" | "VAR" | "VAR.S" | "VARP" | "VAR.P" => {
            let ns = nums(&a);
            let sample = matches!(name, "STDEV" | "STDEV.S" | "VAR" | "VAR.S");
            let need = if sample { 2 } else { 1 };
            if ns.len() < need {
                Value::Error("#DIV/0!".into())
            } else {
                let n = ns.len() as f64;
                let mean = ns.iter().sum::<f64>() / n;
                let ss: f64 = ns.iter().map(|x| (x - mean) * (x - mean)).sum();
                let var = ss / if sample { n - 1.0 } else { n };
                Value::Number(if name.starts_with("STDEV") { var.sqrt() } else { var })
            }
        }
        "COUNTBLANK" => Value::Number(a.iter().filter(|v| v.is_empty()).count() as f64),
        "SUMSQ" => Value::Number(nums(&a).iter().map(|x| x * x).sum()),
        // ---- 数学(第2段)----
        "FACT" => {
            let n = a.first().map(|v| v.as_number()).unwrap_or(0.0);
            if !(0.0..=170.0).contains(&n) {
                Value::Error("#NUM!".into())
            } else {
                Value::Number((1..=n as i64).map(|i| i as f64).product())
            }
        }
        "COMBIN" | "PERMUT" => {
            let n = a.first().map(|v| v.as_number()).unwrap_or(0.0).floor();
            let k = a.get(1).map(|v| v.as_number()).unwrap_or(0.0).floor();
            if n < 0.0 || k < 0.0 || k > n || n > 1e15 {
                Value::Error("#NUM!".into())
            } else {
                let mut r = 1.0f64;
                for i in 0..k as i64 {
                    r *= n - i as f64;
                    if name == "COMBIN" {
                        r /= (i + 1) as f64;
                    }
                    if !r.is_finite() {
                        return Ok(Value::Error("#NUM!".into()));
                    }
                }
                Value::Number(r.round())
            }
        }
        "GCD" | "LCM" => {
            let ns: Vec<i64> = nums(&a).iter().map(|x| x.abs().floor() as i64).collect();
            if ns.is_empty() {
                return Ok(Value::Error("#VALUE!".into()));
            }
            fn gcd(a: i64, b: i64) -> i64 {
                if b == 0 { a } else { gcd(b, a % b) }
            }
            let mut acc: i64 = if name == "GCD" { 0 } else { 1 };
            for x in ns {
                if name == "GCD" {
                    acc = gcd(acc, x);
                } else {
                    let g = gcd(acc, x);
                    if g == 0 {
                        acc = 0;
                        continue;
                    }
                    match (acc / g).checked_mul(x) {
                        Some(v) => acc = v,
                        None => return Ok(Value::Error("#NUM!".into())),
                    }
                }
            }
            Value::Number(acc as f64)
        }
        "PI" => Value::Number(std::f64::consts::PI),
        "SIN" | "COS" | "TAN" | "SINH" | "COSH" | "TANH" | "EXP" | "DEGREES" | "RADIANS" => {
            let x = a.first().map(|v| v.as_number()).unwrap_or(0.0);
            Value::Number(match name {
                "SIN" => x.sin(),
                "COS" => x.cos(),
                "TAN" => x.tan(),
                "SINH" => x.sinh(),
                "COSH" => x.cosh(),
                "TANH" => x.tanh(),
                "EXP" => x.exp(),
                "DEGREES" => x.to_degrees(),
                _ => x.to_radians(),
            })
        }
        "ASIN" | "ACOS" => {
            let x = a.first().map(|v| v.as_number()).unwrap_or(0.0);
            if !(-1.0..=1.0).contains(&x) {
                Value::Error("#NUM!".into())
            } else {
                Value::Number(if name == "ASIN" { x.asin() } else { x.acos() })
            }
        }
        "ATAN" => Value::Number(a.first().map(|v| v.as_number()).unwrap_or(0.0).atan()),
        "ATAN2" => {
            // Excel の約束: ATAN2(x, y)(数学の atan2(y, x) と引数が逆順)
            let x = a.first().map(|v| v.as_number()).unwrap_or(0.0);
            let y = a.get(1).map(|v| v.as_number()).unwrap_or(0.0);
            if x == 0.0 && y == 0.0 {
                Value::Error("#DIV/0!".into())
            } else {
                Value::Number(y.atan2(x))
            }
        }
        "LN" | "LOG10" => {
            let x = a.first().map(|v| v.as_number()).unwrap_or(0.0);
            if x <= 0.0 {
                Value::Error("#NUM!".into())
            } else {
                Value::Number(if name == "LN" { x.ln() } else { x.log10() })
            }
        }
        "LOG" => {
            // LOG(数, [底]) — 底の既定は 10
            let x = a.first().map(|v| v.as_number()).unwrap_or(0.0);
            let b = a.get(1).map(|v| v.as_number()).unwrap_or(10.0);
            if x <= 0.0 || b <= 0.0 || b == 1.0 {
                Value::Error("#NUM!".into())
            } else {
                Value::Number(x.log(b))
            }
        }
        "CEILING" | "FLOOR" => {
            // 基準値の倍数へ。符号が食い違う組は #NUM!(Excel の約束)
            let x = a.first().map(|v| v.as_number()).unwrap_or(0.0);
            let s = a.get(1).map(|v| v.as_number()).unwrap_or(1.0);
            if x > 0.0 && s < 0.0 {
                Value::Error("#NUM!".into())
            } else if s == 0.0 {
                if name == "CEILING" { Value::Number(0.0) } else { Value::Error("#DIV/0!".into()) }
            } else {
                let q = x / s;
                Value::Number(if name == "CEILING" { q.ceil() } else { q.floor() } * s)
            }
        }
        "MROUND" => {
            let x = a.first().map(|v| v.as_number()).unwrap_or(0.0);
            let m = a.get(1).map(|v| v.as_number()).unwrap_or(0.0);
            if m == 0.0 {
                Value::Number(0.0)
            } else if x.signum() * m.signum() < 0.0 {
                Value::Error("#NUM!".into())
            } else {
                Value::Number((x / m).round() * m)
            }
        }
        "EVEN" | "ODD" => {
            let x = a.first().map(|v| v.as_number()).unwrap_or(0.0);
            let sign = if x < 0.0 { -1.0 } else { 1.0 };
            let m = x.abs().ceil();
            let r = if name == "EVEN" {
                if m % 2.0 == 0.0 { m } else { m + 1.0 }
            } else if m % 2.0 == 1.0 {
                m
            } else {
                m + 1.0
            };
            Value::Number(sign * r)
        }
        "SIGN" => {
            let x = a.first().map(|v| v.as_number()).unwrap_or(0.0);
            Value::Number(if x == 0.0 { 0.0 } else { x.signum() })
        }
        "RAND" => Value::Number(rand01()),
        "RANDBETWEEN" => {
            let lo = a.first().map(|v| v.as_number()).unwrap_or(0.0).ceil();
            let hi = a.get(1).map(|v| v.as_number()).unwrap_or(0.0).floor();
            if hi < lo {
                Value::Error("#NUM!".into())
            } else {
                Value::Number(lo + (rand01() * (hi - lo + 1.0)).floor().min(hi - lo))
            }
        }
        // ---- 情報(第2段)----
        "ISNUMBER" => Value::Bool(matches!(a.first(), Some(Value::Number(_)))),
        "ISTEXT" => Value::Bool(matches!(a.first(), Some(Value::Text(_)))),
        "ISEVEN" | "ISODD" => {
            let x = a.first().map(|v| v.as_number()).unwrap_or(0.0).abs().floor() as i64;
            Value::Bool((x % 2 == 0) == (name == "ISEVEN"))
        }
        // ---- Excel 互換の穴埋め(第4段 2026-08-05)----
        "IFNA" => match a.first() {
            Some(Value::Error(e)) if e == "#N/A" => a.get(1).cloned().unwrap_or(Value::Empty),
            v => v.cloned().unwrap_or(Value::Empty),
        },
        "NA" => Value::Error("#N/A".into()),
        "ISNA" => Value::Bool(matches!(a.first(), Some(Value::Error(e)) if e == "#N/A")),
        "ISERR" => Value::Bool(matches!(a.first(), Some(Value::Error(e)) if e != "#N/A")),
        "ISLOGICAL" => Value::Bool(matches!(a.first(), Some(Value::Bool(_)))),
        "ISNONTEXT" => Value::Bool(!matches!(a.first(), Some(Value::Text(_)))),
        "T" => match a.first() {
            Some(Value::Text(s)) => Value::Text(s.clone()),
            Some(e @ Value::Error(_)) => e.clone(),
            _ => Value::Text(String::new()),
        },
        "N" => match a.first() {
            Some(Value::Number(n)) => Value::Number(*n),
            Some(Value::Bool(b)) => Value::Number(*b as i32 as f64),
            Some(e @ Value::Error(_)) => e.clone(),
            _ => Value::Number(0.0),
        },
        "TYPE" => Value::Number(match a.first() {
            Some(Value::Text(_)) => 2.0,
            Some(Value::Bool(_)) => 4.0,
            Some(Value::Error(_)) => 16.0,
            _ => 1.0,
        }),
        "PROPER" => {
            let s = a.first().map(|v| v.display()).unwrap_or_default();
            let mut out = String::new();
            let mut head = true;
            for c in s.chars() {
                if c.is_alphabetic() {
                    out.extend(if head { c.to_uppercase().collect::<Vec<_>>() }
                               else { c.to_lowercase().collect() });
                    head = false;
                } else {
                    out.push(c);
                    head = true;
                }
            }
            Value::Text(out)
        }
        "EXACT" => Value::Bool(
            a.first().map(|v| v.display()) == a.get(1).map(|v| v.display())),
        "CLEAN" => Value::Text(
            a.first().map(|v| v.display()).unwrap_or_default()
                .chars().filter(|c| !c.is_control()).collect()),
        "FIXED" | "YEN" | "DOLLAR" => {
            // 数を文字にする(桁区切りつき)。YEN は ¥、DOLLAR は $ を頭に
            let x = a.first().map(|v| v.as_number()).unwrap_or(0.0);
            let dec = a.get(1).map(|v| v.as_number()).unwrap_or(2.0) as i32;
            let no_comma = name == "FIXED"
                && a.get(2).map(|v| v.as_number() != 0.0).unwrap_or(false);
            let code = match (name, no_comma) {
                (_, true) => format!("0.{}", "0".repeat(dec.max(0) as usize)),
                ("YEN", _) => format!("¥#,##0{}", if dec > 0 {
                    format!(".{}", "0".repeat(dec as usize)) } else { String::new() }),
                ("DOLLAR", _) => format!("$#,##0.{}", "0".repeat(dec.max(0) as usize)),
                _ => format!("#,##0{}", if dec > 0 {
                    format!(".{}", "0".repeat(dec as usize)) } else { String::new() }),
            };
            // 桁数が負なら、その桁で丸める(FIXED(-2) は百の位)
            let x = if dec < 0 {
                let f = 10f64.powi(-dec);
                (x / f).round() * f
            } else {
                x
            };
            Value::Text(format_value(&Value::Number(x), Some(&code)))
        }
        "NUMBERVALUE" => {
            // NUMBERVALUE(文字列, [小数点], [桁区切り])
            let s = a.first().map(|v| v.display()).unwrap_or_default();
            let dec = a.get(1).map(|v| v.display()).unwrap_or_else(|| ".".into());
            let grp = a.get(2).map(|v| v.display()).unwrap_or_else(|| ",".into());
            let t: String = s
                .trim()
                .chars()
                .filter(|c| !grp.contains(*c) && !c.is_whitespace())
                .map(|c| if dec.contains(c) { '.' } else { c })
                .collect();
            match t.parse::<f64>() {
                Ok(n) => Value::Number(n),
                Err(_) => Value::Error("#VALUE!".into()),
            }
        }
        // バイト数の一族。**日本の古い帳票の定番** — 全角は2、半角(ASCII と
        // 半角カナ)は1と数える(Excel の日本語ロケールと同じ数え方)
        "LENB" => Value::Number(
            a.first().map(|v| v.display()).unwrap_or_default()
                .chars().map(jchar_width).sum::<usize>() as f64),
        "LEFTB" | "RIGHTB" | "MIDB" => {
            let s = a.first().map(|v| v.display()).unwrap_or_default();
            let ch: Vec<char> = s.chars().collect();
            let n = |i: usize| a.get(i).map(|v| v.as_number() as usize).unwrap_or(0);
            let take_bytes = |it: &mut dyn Iterator<Item = &char>, budget: usize| -> String {
                let mut used = 0;
                let mut out = String::new();
                for c in it {
                    let w = jchar_width(*c);
                    if used + w > budget {
                        break;
                    }
                    used += w;
                    out.push(*c);
                }
                out
            };
            Value::Text(match name {
                "LEFTB" => take_bytes(&mut ch.iter(), n(1)),
                "RIGHTB" => {
                    // 後ろから測って、前から出す
                    let total: usize = ch.iter().map(|c| jchar_width(*c)).sum();
                    let skip = total.saturating_sub(n(1));
                    let mut used = 0;
                    ch.iter()
                        .skip_while(|c| {
                            used += jchar_width(**c);
                            used <= skip
                        })
                        .collect()
                }
                _ => {
                    // MIDB(文字列, 始まり, 数)。始まりは1起点のバイト位置
                    let start = n(1).saturating_sub(1);
                    let mut used = 0;
                    let rest: Vec<&char> = ch
                        .iter()
                        .skip_while(|c| {
                            if used < start {
                                used += jchar_width(**c);
                                true
                            } else {
                                false
                            }
                        })
                        .collect();
                    take_bytes(&mut rest.into_iter(), n(2))
                }
            })
        }
        // 全角と半角(日本語一級の道具)
        "ASC" => Value::Text(asc_hankaku(&a.first().map(|v| v.display()).unwrap_or_default())),
        "JIS" | "DBCS" => {
            Value::Text(jis_zenkaku(&a.first().map(|v| v.display()).unwrap_or_default()))
        }
        "DATESTRING" => {
            // 通し番号 → 和暦の文字(令和08年08月05日)。明治より前は西暦で
            let serial = a.first().map(|v| v.as_number()).unwrap_or(0.0) as i64;
            Value::Text(wareki(serial))
        }
        "ADDRESS" => {
            // ADDRESS(行, 列, [形式]) — 1=絶対(既定) 4=相対
            let r = a.first().map(|v| v.as_number()).unwrap_or(0.0) as i64;
            let c = a.get(1).map(|v| v.as_number()).unwrap_or(0.0) as i64;
            let abs = a.get(2).map(|v| v.as_number()).unwrap_or(1.0) as i64;
            if r < 1 || c < 1 {
                Value::Error("#VALUE!".into())
            } else {
                let cell = Pos::new(r as u32 - 1, c as u32 - 1).a1();
                Value::Text(match abs {
                    4 => cell,
                    _ => {
                        let split = cell.find(|ch: char| ch.is_ascii_digit()).unwrap_or(0);
                        format!("${}${}", &cell[..split], &cell[split..])
                    }
                })
            }
        }
        "HYPERLINK" => {
            // 表示は文字だけ(飛ぶ仕掛けはセルのリンクの仕事)
            a.get(1).or(a.first()).cloned().unwrap_or(Value::Empty)
        }
        "QUOTIENT" => {
            let x = a.first().map(|v| v.as_number()).unwrap_or(0.0);
            let y = a.get(1).map(|v| v.as_number()).unwrap_or(0.0);
            if y == 0.0 {
                Value::Error("#DIV/0!".into())
            } else {
                Value::Number((x / y).trunc())
            }
        }
        "CEILING.MATH" | "FLOOR.MATH" => {
            // 新しい既定の丸め。基準は絶対値で見る(符号の縛りが無い)
            let x = a.first().map(|v| v.as_number()).unwrap_or(0.0);
            let s = a.get(1).map(|v| v.as_number().abs()).unwrap_or(1.0);
            if s == 0.0 {
                Value::Number(0.0)
            } else {
                let q = x / s;
                Value::Number(if name == "CEILING.MATH" { q.ceil() } else { q.floor() } * s)
            }
        }
        "AVERAGEA" | "MAXA" | "MINA" => {
            // A 付き: 文字は0、TRUE は1 と数える(Excel の約束)
            let ns: Vec<f64> = a
                .iter()
                .filter(|v| !v.is_empty())
                .map(|v| match v {
                    Value::Number(n) => *n,
                    Value::Bool(b) => *b as i32 as f64,
                    _ => 0.0,
                })
                .collect();
            if ns.is_empty() {
                if name == "AVERAGEA" {
                    Value::Error("#DIV/0!".into())
                } else {
                    Value::Number(0.0)
                }
            } else {
                Value::Number(match name {
                    "AVERAGEA" => ns.iter().sum::<f64>() / ns.len() as f64,
                    "MAXA" => ns.iter().cloned().reduce(f64::max).unwrap(),
                    _ => ns.iter().cloned().reduce(f64::min).unwrap(),
                })
            }
        }
        "DAYS" => {
            let end = a.first().map(|v| v.as_number()).unwrap_or(0.0);
            let start = a.get(1).map(|v| v.as_number()).unwrap_or(0.0);
            Value::Number(end - start)
        }
        "DAYS360" => {
            let s = a.first().map(|v| v.as_number()).unwrap_or(0.0) as i64;
            let e = a.get(1).map(|v| v.as_number()).unwrap_or(0.0) as i64;
            Value::Number(days360(s, e) as f64)
        }
        "YEARFRAC" => {
            // 基準 0=30/360(既定) 1=実日数/年平均 2=/360 3=/365 4=欧州30/360
            let s = a.first().map(|v| v.as_number()).unwrap_or(0.0) as i64;
            let e = a.get(1).map(|v| v.as_number()).unwrap_or(0.0) as i64;
            let basis = a.get(2).map(|v| v.as_number()).unwrap_or(0.0) as i64;
            let days = (e - s) as f64;
            Value::Number(match basis {
                1 => days / 365.25, // 実際の暦の平均(Excel の厳密式の近似)
                2 => days / 360.0,
                3 => days / 365.0,
                4 => days360(s, e) as f64 / 360.0,
                _ => days360(s, e) as f64 / 360.0,
            })
        }
        "WEEKNUM" => {
            // 年の何週目か(1=日曜始まり(既定)、2=月曜始まり)
            let serial = a.first().map(|v| v.as_number()).unwrap_or(0.0) as i64;
            let mon = a.get(1).map(|v| v.as_number()).unwrap_or(1.0) as i64 == 2;
            let (y, _, _) = civil_from_days(serial - EXCEL_EPOCH_DAYS);
            let jan1 = date_serial(y, 1, 1);
            let head = if mon { (weekday0(jan1) + 6) % 7 } else { weekday0(jan1) };
            Value::Number(((serial - jan1 + head) / 7 + 1) as f64)
        }
        "ISOWEEKNUM" => {
            // ISO 8601: 木曜を含む週がその年の第1週
            let serial = a.first().map(|v| v.as_number()).unwrap_or(0.0) as i64;
            // その週の木曜へ動かして年内通算で数える
            let dow = (weekday0(serial) + 6) % 7; // 0=月曜
            let thu = serial - dow + 3;
            let (y, _, _) = civil_from_days(thu - EXCEL_EPOCH_DAYS);
            Value::Number(((thu - date_serial(y, 1, 1)) / 7 + 1) as f64)
        }
        "NPV" => {
            // NPV(利率, 額…) — 1期目の終わりから割り引く(Excel と同じ)
            let r = a.first().map(|v| v.as_number()).unwrap_or(0.0);
            if r <= -1.0 {
                Value::Error("#NUM!".into())
            } else {
                Value::Number(
                    a.get(1..)
                        .unwrap_or(&[])
                        .iter()
                        .filter(|v| !v.is_empty())
                        .enumerate()
                        .map(|(i, v)| v.as_number() / (1.0 + r).powi(i as i32 + 1))
                        .sum(),
                )
            }
        }
        "RATE" => {
            // RATE(回数, 定額, 現在価値, [将来価値]) — 反復解
            let g = |i: usize| a.get(i).map(|v| v.as_number()).unwrap_or(0.0);
            let (nper, pmt, pv, fv) = (g(0), g(1), g(2), g(3));
            let f = |r: f64| -> f64 {
                if r.abs() < 1e-12 {
                    pv + pmt * nper + fv
                } else {
                    let k = (1.0 + r).powf(nper);
                    pv * k + pmt * (k - 1.0) / r + fv
                }
            };
            match bisect(&f, -0.9999, 10.0) {
                Some(r) => Value::Number(r),
                None => Value::Error("#NUM!".into()),
            }
        }
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
    let mut p = P { t: &toks, i: 0, sheet, resolved: &resolved, at: Pos::new(0, 0), others: &[], sheet_at: 0, skip_hidden: Default::default(), lets: Vec::new() };
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

/// 並びを返す関数(スピルする関数)。セル単独でも、四則・比較・& と
/// 組み合わせた配列数式でも使える。答えが2次元なら隣へあふれる
const ARRAY_FNS: &[&str] =
    &["FILTER", "SORT", "UNIQUE", "SEQUENCE", "TRANSPOSE", "TEXTSPLIT"];

pub fn recalc(sheet: &mut Sheet) {
    recalc_impl(sheet, &[], 0);
}

/// ブックの1枚を、**他のシートを見ながら**再計算する
/// (INDIRECT("別の表!A1") はこの道でだけ解ける)。
pub fn recalc_book(book: &mut crate::Book, target: usize) {
    if target >= book.sheets.len() {
        return;
    }
    let iter = book.calc_iter;
    let (left, rest) = book.sheets.split_at_mut(target);
    let (tgt, right) = rest.split_first_mut().expect("上で確かめた");
    let others: Vec<&Sheet> = left.iter().map(|s| &*s).chain(right.iter().map(|s| &*s)).collect();
    match iter {
        Some((count, delta)) => {
            // 反復計算: 循環は前回の値で埋めて、変化が delta 以下に
            // 落ち着くまで(上限 count 回)回す — Excel と同じ枠組み
            for _ in 0..count.max(1) {
                let (changed, maxd) = recalc_pass_iter(tgt, &others, target, true);
                if !changed || maxd <= delta {
                    break;
                }
            }
        }
        None => recalc_impl(tgt, &others, target),
    }
}

/// 全シートの再計算。別のシートへの間接参照があるときは、
/// 参照の先が新しくなるようもう1周する
pub fn recalc_all(book: &mut crate::Book) {
    // 直書きの `Sheet2!A1` も INDIRECT と同じく別シートを見るので、
    // `!` を含む式があれば2周する(参照の先が新しくなってから写すため)
    let cross = book.sheets.iter().any(|s| {
        s.cells.values().any(|c| {
            c.formula
                .as_ref()
                .map(|f| f.to_ascii_uppercase().contains("INDIRECT") || f.contains('!'))
                .unwrap_or(false)
        })
    });
    for _ in 0..if cross { 2 } else { 1 } {
        for i in 0..book.sheets.len() {
            recalc_book(book, i);
        }
    }
}

fn recalc_impl(sheet: &mut Sheet, others: &[&Sheet], at: usize) {
    // OFFSET/INDIRECT(計算で決まる参照)とスピルは、1回の走査では依存の順が
    // 読めないことがある — そのときだけ、値が動かなくなるまで回す(上限つき。
    // RAND/NOW 入りの式は毎回変わるので、比較からは外している)
    let dynamic = !sheet.spills.is_empty()
        || sheet.cells.values().any(|c| {
            c.formula
                .as_ref()
                .map(|f| {
                    let u = f.to_ascii_uppercase();
                    // 構造化参照(`[`)も deps では位置に解けない —
                    // 依存の順が読めないので、値が動かなくなるまで回す組に入れる
                    u.contains("OFFSET") || u.contains("INDIRECT") || u.contains('[')
                        || ARRAY_FNS.iter().any(|n| u.contains(n))
                })
                .unwrap_or(false)
        });
    if !dynamic {
        recalc_pass(sheet, others, at);
        return;
    }
    for _ in 0..5 {
        if !recalc_pass(sheet, others, at) {
            break;
        }
    }
}

/// 再計算の1周。値が動いたら true(まだ安定していないかもしれない)
fn recalc_pass(sheet: &mut Sheet, others: &[&Sheet], at: usize) -> bool {
    recalc_pass_iter(sheet, others, at, false).0
}

/// 再計算の1周(反復モードつき)。反復モードでは循環参照を #CIRC! に
/// せず**前回の値**で埋める。返りは (動いたか, 数の最大変化量)
fn recalc_pass_iter(
    sheet: &mut Sheet,
    others: &[&Sheet],
    at: usize,
    iter_mode: bool,
) -> (bool, f64) {
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
    // 式を集める。あふれる関数の入った式は「配列数式」として別扱い
    let mut formulas: Vec<(Pos, String)> = Vec::new();
    let mut arrays: Vec<(Pos, String)> = Vec::new();
    for (p, c) in &sheet.cells {
        let Some(f) = c.formula.as_ref().filter(|f| !is_py_formula(f)) else { continue };
        let f = expand_names(f, &sheet.names);
        if is_array_formula(&f) {
            arrays.push((*p, f));
        } else {
            formulas.push((*p, f));
        }
    }
    // RAND/NOW/TODAY 入りの式は毎回値が変わる — 安定の判定から外す
    let volatile: HashSet<Pos> = formulas
        .iter()
        .chain(arrays.iter())
        .filter(|(_, f)| {
            let u = f.to_ascii_uppercase();
            u.contains("RAND") || u.contains("NOW") || u.contains("TODAY")
        })
        .map(|(p, _)| *p)
        .collect();
    // 前回のスピルの影(起点以外)。**ここではまだ消さない** — 先に消すと
    // 通常の式が「消された直後」を読んで、値が縮んだまま安定してしまう。
    // 影の席は「置き直してよい席」として覚えるだけ。掃除は置き場所が
    // 決まったあと(この関数の後半)
    let mut freed: HashSet<Pos> = HashSet::new();
    for (o, (h, w)) in sheet.spills.iter() {
        for r in o.row..o.row + h {
            for c in o.col..o.col + w {
                let p = Pos::new(r, c);
                if p != *o {
                    freed.insert(p);
                }
            }
        }
    }
    let mut changed = false;

    let mut resolved: HashMap<Pos, Value> = HashMap::new();
    let mut visiting: HashSet<Pos> = HashSet::new();

    fn eval_at(
        p: Pos,
        map: &HashMap<Pos, String>,
        sheet: &Sheet,
        others: &[&Sheet],
        at: usize,
        resolved: &mut HashMap<Pos, Value>,
        visiting: &mut HashSet<Pos>,
        iter_mode: bool,
    ) -> Value {
        if let Some(v) = resolved.get(&p) {
            return v.clone();
        }
        let Some(f) = map.get(&p) else {
            return sheet.value(p);
        };
        if !visiting.insert(p) {
            if iter_mode {
                // 反復計算: 循環は**前回の値**で埋める。初回(空や #CIRC! の
                // 残骸)は 0 から始める — Excel と同じ起点
                return match sheet.value(p) {
                    Value::Number(n) => Value::Number(n),
                    _ => Value::Number(0.0),
                };
            }
            return Value::Error("#CIRC!".into());
        }
        // 先に依存を解く
        for d in deps(f) {
            if map.contains_key(&d) && !resolved.contains_key(&d) {
                let v = eval_at(d, map, sheet, others, at, resolved, visiting, iter_mode);
                resolved.insert(d, v);
            }
        }
        let v = match lex(f) {
            Ok(toks) => {
                let mut p2 = P { t: &toks, i: 0, sheet, resolved, at: p, others, sheet_at: at, skip_hidden: Default::default(), lets: Vec::new() };
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
        let v = eval_at(*p, &map, sheet, others, at, &mut resolved, &mut visiting, iter_mode);
        resolved.insert(*p, v);
    }
    let mut max_delta = 0.0f64;
    for (p, v) in resolved {
        if let Some(c) = sheet.cells.get_mut(&p) {
            if c.formula.is_some() {
                if c.value != v && !volatile.contains(&p) {
                    changed = true;
                    if let (Value::Number(a), Value::Number(b)) = (&c.value, &v) {
                        max_delta = max_delta.max((a - b).abs());
                    } else {
                        max_delta = f64::INFINITY; // 数でない変化は「まだ大きい」
                    }
                }
                c.value = v;
            }
        }
    }

    // 配列の式(スピル)。通常の式のあとに評価し、置き先をまず全部決めてから
    // (掃除 → 書き込み)の順で反映する
    let mut new_spills: std::collections::BTreeMap<Pos, (u32, u32)> = Default::default();
    let mut writes: Vec<(Pos, Value)> = Vec::new();
    let mut written: HashSet<Pos> = HashSet::new();
    for (origin, f) in &arrays {
        let put_origin = |sheet: &mut Sheet, v: Value, changed: &mut bool| {
            if let Some(c) = sheet.cells.get_mut(origin) {
                if c.value != v && !volatile.contains(origin) {
                    *changed = true;
                }
                c.value = v;
            }
        };
        let rows = match eval_array(sheet, others, at, f, *origin) {
            Err(e) => {
                put_origin(sheet, e, &mut changed);
                continue;
            }
            Ok(r) => r,
        };
        let h = rows.len() as u32;
        let w = rows.iter().map(|r| r.len()).max().unwrap_or(0) as u32;
        if h == 0 || w == 0 || h.saturating_mul(w) > 200_000 {
            put_origin(sheet, Value::Error("#NUM!".into()), &mut changed);
            continue;
        }
        // 1×1 の答えは普通の値として置く(=SUM(FILTER(…))+1 のような集計)
        if h == 1 && w == 1 {
            let v = rows[0].first().cloned().unwrap_or(Value::Empty);
            put_origin(sheet, v, &mut changed);
            continue;
        }
        // 席の検査: 既に中身のあるセルへは**あふれない**(黙って潰さない)。
        // 前回の自分たちの影(freed)は空席と見る。同じ周の別のスピルとも争わない
        let mut blocked = false;
        'seek: for r in 0..h {
            for c in 0..w {
                let p = Pos::new(origin.row + r, origin.col + c);
                if p == *origin {
                    continue;
                }
                if written.contains(&p) {
                    blocked = true;
                    break 'seek;
                }
                if let Some(cell) = sheet.cells.get(&p) {
                    if cell.formula.is_some()
                        || (!cell.value.is_empty() && !freed.contains(&p))
                    {
                        blocked = true;
                        break 'seek;
                    }
                }
            }
        }
        if blocked {
            put_origin(sheet, Value::Error("#SPILL!".into()), &mut changed);
            continue;
        }
        for (r, row) in rows.iter().enumerate() {
            for c in 0..w as usize {
                let p = Pos::new(origin.row + r as u32, origin.col + c as u32);
                let v = row.get(c).cloned().unwrap_or(Value::Empty);
                if p == *origin {
                    put_origin(sheet, v, &mut changed);
                } else {
                    written.insert(p);
                    writes.push((p, v));
                }
            }
        }
        new_spills.insert(*origin, (h, w));
    }
    // 掃除: 前回の影のうち、今回書かない席だけ空にする(書式は残す)
    for p in &freed {
        if written.contains(p) {
            continue;
        }
        if let Some(cell) = sheet.cells.get_mut(p) {
            if cell.formula.is_none() && !cell.value.is_empty() {
                changed = true;
                cell.value = Value::Empty;
            }
        }
        if sheet
            .cells
            .get(p)
            .map(|c| c.formula.is_none() && c.value.is_empty() && c.fmt == Default::default())
            .unwrap_or(false)
        {
            sheet.cells.remove(p);
        }
    }
    // 書き込み
    for (p, v) in writes {
        match sheet.cells.get_mut(&p) {
            Some(cell) => {
                if cell.value != v {
                    changed = true;
                }
                cell.value = v;
            }
            None => {
                if !v.is_empty() {
                    changed = true;
                }
                sheet.cells.insert(p, Cell { formula: None, value: v, fmt: Default::default() });
            }
        }
    }
    if sheet.spills != new_spills {
        changed = true;
        sheet.spills = new_spills;
    }
    (changed, max_delta)
}

/// 配列数式か — あふれる関数(FILTER 等)が式のどこかに入っているか。
/// 文字列の中の "FILTER" を拾わないよう、字句にしてから見る
fn is_array_formula(f: &str) -> bool {
    lex(f)
        .map(|toks| {
            toks.iter()
                .any(|t| matches!(t, Tok::Name(n) if ARRAY_FNS.contains(&n.as_str())))
        })
        .unwrap_or(false)
}

/// 配列数式を評価して、行ごとの値にする。
/// =SEQUENCE(3)+1 のように演算子と組み合わせた式も、要素ごとに計算される
fn eval_array(
    sheet: &Sheet,
    others: &[&Sheet],
    sheet_at: usize,
    f: &str,
    at: Pos,
) -> Result<Vec<Vec<Value>>, Value> {
    let err = |s: &str| Value::Error(s.into());
    let toks = lex(f).map_err(|_| err("#ERROR!"))?;
    let resolved = HashMap::new();
    let mut p = P { t: &toks, i: 0, sheet, resolved: &resolved, at, others, sheet_at, skip_hidden: Default::default(), lets: Vec::new() };
    let v = {
        let mut ap = AP { p: &mut p };
        ap.expr().map_err(|_| err("#ERROR!"))?
    };
    if p.i != toks.len() {
        return Err(err("#ERROR!"));
    }
    Ok(match v {
        AVal::One(x) => vec![vec![x]],
        AVal::Arr(rows) => rows,
    })
}

fn array_call(name: &str, args: Vec<Arg>) -> Result<Vec<Vec<Value>>, Value> {
    let err = |s: &str| Value::Error(s.into());
    // 範囲を行ごとに割る
    let rows_of = |a: &Arg| -> Vec<Vec<Value>> {
        match a {
            Arg::One(v) => vec![vec![v.clone()]],
            Arg::Rect(w, vals) => {
                let w = (*w).max(1) as usize;
                vals.chunks(w).map(|r| r.to_vec()).collect()
            }
        }
    };
    match name {
        "SEQUENCE" => {
            // SEQUENCE(行, [列], [始まり], [間隔])
            let g = |i: usize| args.get(i).map(|a| a.first().as_number());
            let rows = g(0).unwrap_or(f64::NAN);
            let cols = g(1).unwrap_or(1.0);
            let start = g(2).unwrap_or(1.0);
            let step = g(3).unwrap_or(1.0);
            if !(1.0..=200_000.0).contains(&rows)
                || !(1.0..=200_000.0).contains(&cols)
                || rows * cols > 200_000.0
            {
                return Err(err("#NUM!"));
            }
            let (rows, cols) = (rows as usize, cols as usize);
            Ok((0..rows)
                .map(|r| {
                    (0..cols)
                        .map(|c| Value::Number(start + step * (r * cols + c) as f64))
                        .collect()
                })
                .collect())
        }
        "UNIQUE" => {
            let rows = rows_of(args.first().ok_or(err("#VALUE!"))?);
            let mut seen = HashSet::new();
            let mut out = Vec::new();
            for row in rows {
                let key: String =
                    row.iter().map(|v| v.display()).collect::<Vec<_>>().join("\u{1}");
                if seen.insert(key) {
                    out.push(row);
                }
            }
            Ok(out)
        }
        "SORT" => {
            // SORT(範囲, [鍵の列], [順序 1/-1])
            let mut rows = rows_of(args.first().ok_or(err("#VALUE!"))?);
            let w = rows.first().map(|r| r.len()).unwrap_or(0);
            let idx = args.get(1).map(|a| a.first().as_number() as usize).unwrap_or(1);
            let desc = args.get(2).map(|a| a.first().as_number() < 0.0).unwrap_or(false);
            if idx == 0 || idx > w {
                return Err(err("#VALUE!"));
            }
            rows.sort_by(|x, y| {
                let (a, b) = (&x[idx - 1], &y[idx - 1]);
                let o = match (a, b) {
                    (Value::Number(p), Value::Number(q)) => {
                        p.partial_cmp(q).unwrap_or(std::cmp::Ordering::Equal)
                    }
                    _ => a.display().cmp(&b.display()),
                };
                if desc { o.reverse() } else { o }
            });
            Ok(rows)
        }
        "TEXTSPLIT" => {
            // TEXTSPLIT(文字, 列の区切り, [行の区切り], [空を飛ばす])
            // 区切りが空なら #VALUE!(黙って1個の塊を返さない)
            let s = args.first().ok_or(err("#VALUE!"))?.first().display();
            let col_d = args.get(1).map(|a| a.first().display()).unwrap_or_default();
            let row_d = args.get(2).map(|a| a.first().display()).unwrap_or_default();
            let skip_empty = args.get(3).is_some_and(|a| a.first().as_number() != 0.0);
            if col_d.is_empty() && row_d.is_empty() {
                return Err(err("#VALUE!"));
            }
            let lines: Vec<&str> = if row_d.is_empty() {
                vec![s.as_str()]
            } else {
                s.split(row_d.as_str()).collect()
            };
            let mut out: Vec<Vec<Value>> = Vec::new();
            for line in lines {
                let cells: Vec<&str> = if col_d.is_empty() {
                    vec![line]
                } else {
                    line.split(col_d.as_str()).collect()
                };
                let row: Vec<Value> = cells
                    .into_iter()
                    .filter(|c| !(skip_empty && c.is_empty()))
                    .map(|c| Value::Text(c.to_string()))
                    .collect();
                if !(skip_empty && row.is_empty()) {
                    out.push(row);
                }
            }
            if out.is_empty() {
                out.push(vec![Value::Text(String::new())]);
            }
            Ok(out)
        }
        "TRANSPOSE" => {
            let rows = rows_of(args.first().ok_or(err("#VALUE!"))?);
            let h = rows.len();
            let w = rows.iter().map(|r| r.len()).max().unwrap_or(0);
            Ok((0..w)
                .map(|c| {
                    (0..h)
                        .map(|r| rows[r].get(c).cloned().unwrap_or(Value::Empty))
                        .collect()
                })
                .collect())
        }
        "FILTER" => {
            // FILTER(範囲, 条件の範囲, [空のとき])
            let rows = rows_of(args.first().ok_or(err("#VALUE!"))?);
            let inc: Vec<Value> =
                args.get(1).map(|a| a.values().to_vec()).unwrap_or_default();
            if inc.len() != rows.len() {
                return Err(err("#VALUE!"));
            }
            let out: Vec<Vec<Value>> = rows
                .into_iter()
                .zip(&inc)
                .filter(|(_, i)| i.as_number() != 0.0)
                .map(|(r, _)| r)
                .collect();
            if out.is_empty() {
                // 1件も無いときは第3引数(無ければ #CALC! — Excel と同じ)
                match args.get(2) {
                    Some(d) => Ok(vec![vec![d.first()]]),
                    None => Err(err("#CALC!")),
                }
            } else {
                Ok(out)
            }
        }
        _ => Err(err("#NAME?")),
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
    fn 反復計算は循環を収束させる() {
        // A1 = A1/2 + 1 の不動点は 2。反復なしなら #CIRC!
        let mut b = crate::Book::new();
        b.sheets[0].set(Pos::parse("A1").unwrap(), Cell::input("=A1/2+1"));
        recalc_book(&mut b, 0);
        assert_eq!(
            b.sheets[0].value(Pos::parse("A1").unwrap()).display(),
            "#CIRC!",
            "反復なしで循環が通った"
        );
        b.calc_iter = Some((100, 1e-9));
        recalc_book(&mut b, 0);
        let got = b.sheets[0].value(Pos::parse("A1").unwrap()).as_number();
        assert!((got - 2.0).abs() < 1e-6, "不動点に収束しない: {got}");
        // 相互参照(A2=B2+1, B2=A2 は発散 — 上限で止まりエラーにならない)
        b.sheets[0].set(Pos::parse("A2").unwrap(), Cell::input("=B2+1"));
        b.sheets[0].set(Pos::parse("B2").unwrap(), Cell::input("=A2"));
        recalc_book(&mut b, 0);
        let a2 = b.sheets[0].value(Pos::parse("A2").unwrap());
        assert!(matches!(a2, Value::Number(_)), "上限で止まらずエラー: {a2:?}");
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
    fn 三つ引数のsumifは足す範囲を分けられる() {
        // =SUMIF(条件範囲, 条件, 合計範囲) — Excel で最も多い書き方。
        // 条件は B 列で見て、足すのは C 列
        let mut s = sheet_with(&[
            ("A1", "筆"), ("B1", "文具"), ("C1", "100"),
            ("A2", "紙"), ("B2", "文具"), ("C2", "200"),
            ("A3", "机"), ("B3", "家具"), ("C3", "900"),
        ]);
        assert_eq!(n(&mut s, "=SUMIF(B1:B3,\"文具\",C1:C3)"), 300.0);
        // 3つ目を省いたら、条件を見た範囲そのものを足す
        assert_eq!(n(&mut s, "=SUMIF(C1:C3,\">150\")"), 1100.0);
        // 1件も合わなければ 0(Excel の約束)
        assert_eq!(n(&mut s, "=SUMIF(B1:B3,\"食品\",C1:C3)"), 0.0);
        // 範囲の大きさが違えば黙って数を返さない
        assert_eq!(
            value_of(&mut s, "=SUMIF(B1:B3,\"文具\",C1:C2)"),
            Value::Error("#VALUE!".into()),
            "大きさ違いを黙って計算しない"
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

/// 第2段の拡充(2026-08-05)— 統計・数学で「表計算らしさ」を出す約45個。
#[cfg(test)]
mod dan2_tests {
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

    #[test]
    fn 成績処理の統計() {
        let mut s = sheet_with(&[
            ("A1", "70"), ("A2", "80"), ("A3", "80"), ("A4", "90"), ("A5", "100"),
        ]);
        assert_eq!(n(&mut s, "=MEDIAN(A1:A5)"), 80.0);
        assert_eq!(n(&mut s, "=MEDIAN(A1:A4)"), 80.0, "偶数個は真ん中2つの平均");
        assert_eq!(n(&mut s, "=MODE(A1:A5)"), 80.0);
        assert_eq!(
            value_of(&mut s, "=MODE(A1:A2)"),
            Value::Error("#N/A".into()),
            "重複が無ければ最頻値は無い"
        );
        // 母集団の分散: 平均84、偏差平方和 (196+16+16+36+256)=520 → 104
        assert!((n(&mut s, "=VARP(A1:A5)") - 104.0).abs() < 1e-9);
        assert!((n(&mut s, "=VAR(A1:A5)") - 130.0).abs() < 1e-9, "標本分散は n-1 で割る");
        assert!((n(&mut s, "=STDEVP(A1:A5)") - 104.0f64.sqrt()).abs() < 1e-9);
        assert!((n(&mut s, "=STDEV(A1:A5)") - 130.0f64.sqrt()).abs() < 1e-9);
        assert_eq!(
            value_of(&mut s, "=STDEV(A1)"),
            Value::Error("#DIV/0!".into()),
            "1個から標本標準偏差は出ない"
        );
        assert_eq!(n(&mut s, "=PERCENTILE(A1:A5,0.5)"), 80.0);
        assert_eq!(n(&mut s, "=PERCENTILE(A1:A5,0.25)"), 80.0);
        assert_eq!(n(&mut s, "=QUARTILE(A1:A5,0)"), 70.0, "第0四分位は最小");
        assert_eq!(n(&mut s, "=QUARTILE(A1:A5,4)"), 100.0, "第4四分位は最大");
        assert_eq!(n(&mut s, "=SUMSQ(3,4)"), 25.0);
    }

    #[test]
    fn 相関と回帰() {
        // y = 2x + 1 きっかり(相関1・傾き2・切片1)
        let mut s = sheet_with(&[
            ("A1", "1"), ("B1", "3"),
            ("A2", "2"), ("B2", "5"),
            ("A3", "3"), ("B3", "7"),
        ]);
        assert!((n(&mut s, "=CORREL(A1:A3,B1:B3)") - 1.0).abs() < 1e-12);
        assert!((n(&mut s, "=SLOPE(B1:B3,A1:A3)") - 2.0).abs() < 1e-12);
        assert!((n(&mut s, "=INTERCEPT(B1:B3,A1:A3)") - 1.0).abs() < 1e-12);
        assert!((n(&mut s, "=FORECAST(10,B1:B3,A1:A3)") - 21.0).abs() < 1e-12);
        assert_eq!(
            value_of(&mut s, "=CORREL(A1:A3,B1:B2)"),
            Value::Error("#N/A".into()),
            "大きさ違いを黙って計算しない"
        );
    }

    #[test]
    fn 組合せと整数論() {
        let mut s = sheet_with(&[]);
        assert_eq!(n(&mut s, "=FACT(5)"), 120.0);
        assert_eq!(n(&mut s, "=COMBIN(10,3)"), 120.0);
        assert_eq!(n(&mut s, "=PERMUT(10,3)"), 720.0);
        assert_eq!(n(&mut s, "=GCD(12,18,24)"), 6.0);
        assert_eq!(n(&mut s, "=LCM(4,6)"), 12.0);
        assert_eq!(value_of(&mut s, "=FACT(200)"), Value::Error("#NUM!".into()));
    }

    #[test]
    fn 三角と対数() {
        let mut s = sheet_with(&[]);
        assert!((n(&mut s, "=SIN(PI()/2)") - 1.0).abs() < 1e-12);
        assert!((n(&mut s, "=COS(0)") - 1.0).abs() < 1e-12);
        assert!((n(&mut s, "=TAN(PI()/4)") - 1.0).abs() < 1e-12);
        assert!((n(&mut s, "=DEGREES(PI())") - 180.0).abs() < 1e-12);
        assert!((n(&mut s, "=RADIANS(180)") - std::f64::consts::PI).abs() < 1e-12);
        assert!((n(&mut s, "=ASIN(1)") - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        // Excel の約束: ATAN2(x, y) で点 (1,1) は 45度
        assert!((n(&mut s, "=ATAN2(1,1)") - std::f64::consts::FRAC_PI_4).abs() < 1e-12);
        assert!((n(&mut s, "=EXP(1)") - std::f64::consts::E).abs() < 1e-12);
        assert!((n(&mut s, "=LN(EXP(2))") - 2.0).abs() < 1e-12);
        assert_eq!(n(&mut s, "=LOG10(1000)"), 3.0);
        assert_eq!(n(&mut s, "=LOG(8,2)"), 3.0);
        assert_eq!(n(&mut s, "=LOG(100)"), 2.0, "底の既定は10");
        assert_eq!(value_of(&mut s, "=LN(0)"), Value::Error("#NUM!".into()));
        assert_eq!(value_of(&mut s, "=ASIN(2)"), Value::Error("#NUM!".into()));
    }

    #[test]
    fn 丸めの一族() {
        let mut s = sheet_with(&[]);
        assert_eq!(n(&mut s, "=CEILING(6.1,2)"), 8.0);
        assert_eq!(n(&mut s, "=FLOOR(6.9,2)"), 6.0);
        assert_eq!(n(&mut s, "=CEILING(-2.5,-2)"), -4.0, "負の基準は0から遠ざかる");
        assert_eq!(n(&mut s, "=MROUND(7,3)"), 6.0);
        assert_eq!(n(&mut s, "=MROUND(8,3)"), 9.0);
        assert_eq!(n(&mut s, "=EVEN(3)"), 4.0);
        assert_eq!(n(&mut s, "=EVEN(-3)"), -4.0, "0から遠ざかる");
        assert_eq!(n(&mut s, "=ODD(2)"), 3.0);
        assert_eq!(n(&mut s, "=SIGN(-5)"), -1.0);
        assert_eq!(n(&mut s, "=SIGN(0)"), 0.0);
        assert_eq!(
            value_of(&mut s, "=CEILING(2.5,-2)"),
            Value::Error("#NUM!".into()),
            "符号違いを黙って丸めない"
        );
    }

    #[test]
    fn 乱数は範囲に収まる() {
        let mut s = sheet_with(&[]);
        for _ in 0..20 {
            let r = n(&mut s, "=RAND()");
            assert!((0.0..1.0).contains(&r), "RAND が範囲外: {r}");
            let d = n(&mut s, "=RANDBETWEEN(1,6)");
            assert!((1.0..=6.0).contains(&d) && d.fract() == 0.0, "さいころが変: {d}");
        }
        assert_eq!(value_of(&mut s, "=RANDBETWEEN(6,1)"), Value::Error("#NUM!".into()));
    }

    #[test]
    fn 情報関数() {
        let mut s = sheet_with(&[("A1", "9"), ("A2", "文字")]);
        assert_eq!(value_of(&mut s, "=ISNUMBER(A1)"), Value::Bool(true));
        assert_eq!(value_of(&mut s, "=ISNUMBER(A2)"), Value::Bool(false));
        assert_eq!(value_of(&mut s, "=ISNUMBER(1/0)"), Value::Bool(false), "エラーは数でない");
        assert_eq!(value_of(&mut s, "=ISTEXT(A2)"), Value::Bool(true));
        assert_eq!(value_of(&mut s, "=ISEVEN(4)"), Value::Bool(true));
        assert_eq!(value_of(&mut s, "=ISODD(4)"), Value::Bool(false));
        assert_eq!(n(&mut s, "=COUNTBLANK(A1:A5)"), 3.0);
    }
}

/// 第3段の拡充(2026-08-05)— 計算で決まる参照とスピル。
#[cfg(test)]
mod dan3_tests {
    use super::*;
    use crate::model::Cell;

    fn sheet_with(cells: &[(&str, &str)]) -> Sheet {
        let mut s = Sheet { name: "表".into(), ..Default::default() };
        for (a1, v) in cells {
            s.set(Pos::parse(a1).unwrap(), Cell::input(v));
        }
        s
    }

    fn v(s: &Sheet, a1: &str) -> Value {
        s.value(Pos::parse(a1).unwrap())
    }

    #[test]
    fn offsetは参照をずらす() {
        let mut s = sheet_with(&[
            ("A1", "10"), ("B1", "20"),
            ("A2", "30"), ("B2", "40"),
            ("A3", "50"),
            ("Z1", "=OFFSET(A1,1,1)"),
            ("Z2", "=SUM(OFFSET(A1,0,0,3,1))"),
            ("Z3", "=OFFSET(A1,-1,0)"),
            ("Z4", "=OFFSET(A1,0,0,2,2)"),
        ]);
        recalc(&mut s);
        assert_eq!(v(&s, "Z1"), Value::Number(40.0), "1行1列ずらして B2");
        assert_eq!(v(&s, "Z2"), Value::Number(90.0), "高さ3の範囲を SUM に渡す");
        assert_eq!(v(&s, "Z3"), Value::Error("#REF!".into()), "表の外は正直に #REF!");
        assert_eq!(v(&s, "Z4"), Value::Error("#VALUE!".into()),
            "複数セルを1セルの場所に置けない");
    }

    #[test]
    fn indirectは文字列の参照を解く() {
        let mut s = sheet_with(&[
            ("B2", "99"),
            ("C1", "2"),
            ("Z1", "=INDIRECT(\"B2\")"),
            ("Z2", "=INDIRECT(\"B\"&C1)"),
            ("Z3", "=SUM(INDIRECT(\"B1:B3\"))"),
            ("Z4", "=INDIRECT(\"別の表!A1\")"),
            ("Z5", "=INDIRECT(\"ほげ\")"),
        ]);
        recalc(&mut s);
        assert_eq!(v(&s, "Z1"), Value::Number(99.0));
        assert_eq!(v(&s, "Z2"), Value::Number(99.0), "組み立てた参照が解けない");
        assert_eq!(v(&s, "Z3"), Value::Number(99.0), "範囲の間接参照が関数に渡らない");
        assert_eq!(v(&s, "Z4"), Value::Error("#REF!".into()),
            "別のシートはまだ — 黙って自シートと読まない");
        assert_eq!(v(&s, "Z5"), Value::Error("#REF!".into()));
    }

    #[test]
    fn 間接参照の先が式でも追いつく() {
        // A1 は B1 を間接参照、B1 は C1 の式 — 依存が読めないので複数周で収束
        let mut s = sheet_with(&[
            ("A1", "=INDIRECT(\"B1\")"),
            ("B1", "=C1+1"),
            ("C1", "5"),
        ]);
        recalc(&mut s);
        assert_eq!(v(&s, "A1"), Value::Number(6.0), "1周目の古い値で止まっている");
    }

    #[test]
    fn sequenceがあふれて広がる() {
        let mut s = sheet_with(&[("A1", "=SEQUENCE(3,2)")]);
        recalc(&mut s);
        for (a1, n) in [("A1", 1.0), ("B1", 2.0), ("A2", 3.0), ("B2", 4.0),
                        ("A3", 5.0), ("B3", 6.0)] {
            assert_eq!(v(&s, a1), Value::Number(n), "{a1} が違う");
        }
        assert_eq!(s.spills.get(&Pos::parse("A1").unwrap()), Some(&(3, 2)));
        // 縮めたら残骸は消える
        s.set(Pos::parse("A1").unwrap(), Cell::input("=SEQUENCE(2,1)"));
        recalc(&mut s);
        assert_eq!(v(&s, "A2"), Value::Number(2.0));
        assert_eq!(v(&s, "A3"), Value::Empty, "縮めた後に残骸が残った");
        assert_eq!(v(&s, "B1"), Value::Empty);
        assert_eq!(s.spills.get(&Pos::parse("A1").unwrap()), Some(&(2, 1)));
    }

    #[test]
    fn 先客がいればあふれない() {
        let mut s = sheet_with(&[
            ("A1", "=SEQUENCE(3,1)"),
            ("A3", "既にある"),
        ]);
        recalc(&mut s);
        assert_eq!(v(&s, "A1"), Value::Error("#SPILL!".into()),
            "先客を黙って潰してはいけない");
        assert_eq!(v(&s, "A3"), Value::Text("既にある".into()), "先客が消えた");
        assert_eq!(v(&s, "A2"), Value::Empty, "途中まで書いてはいけない");
        // 先客がどけば次の再計算であふれる
        s.set(Pos::parse("A3").unwrap(), Cell::default());
        recalc(&mut s);
        assert_eq!(v(&s, "A1"), Value::Number(1.0));
        assert_eq!(v(&s, "A3"), Value::Number(3.0));
    }

    #[test]
    fn filterとsortとunique() {
        let mut s = sheet_with(&[
            ("A1", "筆"), ("B1", "100"), ("C1", "1"),
            ("A2", "紙"), ("B2", "300"), ("C2", "0"),
            ("A3", "机"), ("B3", "200"), ("C3", "1"),
            ("E1", "=FILTER(A1:B3,C1:C3)"),
            ("H1", "=SORT(A1:B3,2,-1)"),
            ("K1", "=UNIQUE(C1:C3)"),
            ("M1", "=FILTER(A1:B3,B1:B3>999,\"該当なし\")"),
        ]);
        recalc(&mut s);
        // FILTER: C=1 の行だけ
        assert_eq!(v(&s, "E1"), Value::Text("筆".into()));
        assert_eq!(v(&s, "F1"), Value::Number(100.0));
        assert_eq!(v(&s, "E2"), Value::Text("机".into()));
        // SORT: 金額の大きい順
        assert_eq!(v(&s, "H1"), Value::Text("紙".into()));
        assert_eq!(v(&s, "H2"), Value::Text("机".into()));
        assert_eq!(v(&s, "H3"), Value::Text("筆".into()));
        // UNIQUE: 1 と 0
        assert_eq!(v(&s, "K1"), Value::Number(1.0));
        assert_eq!(v(&s, "K2"), Value::Number(0.0));
        assert_eq!(s.spills.get(&Pos::parse("K1").unwrap()), Some(&(2, 1)));
        // 1件も無いときは第3引数
        assert_eq!(v(&s, "M1"), Value::Text("該当なし".into()));
    }

    #[test]
    fn spillの記録がxlsxを往復する() {
        let mut book = crate::Book::new();
        book.sheets[0] = sheet_with(&[("A1", "=SEQUENCE(3,1)"), ("C1", "=SUM(A1:A3)")]);
        book.sheets[0].name = "Sheet1".into();
        recalc(&mut book.sheets[0]);
        assert_eq!(v(&book.sheets[0], "C1"), Value::Number(6.0),
            "スピルの結果を普通の式が拾えない");
        let mut buf = std::io::Cursor::new(Vec::new());
        crate::xlsx::write(&book, &mut buf).unwrap();
        let (mut back, _) = crate::xlsx::read(std::io::Cursor::new(buf.into_inner())).unwrap();
        assert_eq!(back.sheets[0].spills.get(&Pos::parse("A1").unwrap()), Some(&(3, 1)),
            "スピルの記録が往復しない");
        // 開き直して再計算しても、自分の跡を先客と間違えない
        recalc(&mut back.sheets[0]);
        assert_eq!(v(&back.sheets[0], "A1"), Value::Number(1.0),
            "開き直しで偽の #SPILL! になった");
        assert_eq!(v(&back.sheets[0], "A3"), Value::Number(3.0));
    }

    #[test]
    fn 演算子と組み合わせた配列数式もあふれる() {
        // 2026-08-05 まで #配列単独 と断っていた形。要素ごとに計算して広がる
        let mut s = sheet_with(&[("A1", "=SEQUENCE(3,1)+1")]);
        recalc(&mut s);
        assert_eq!(v(&s, "A1"), Value::Number(2.0));
        assert_eq!(v(&s, "A2"), Value::Number(3.0));
        assert_eq!(v(&s, "A3"), Value::Number(4.0));
    }
}

/// 第4段の拡充(2026-08-05)— Excel で作った実物のファイルが読める穴埋め。
#[cfg(test)]
mod dan4_tests {
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
    fn subtotalはフィルターの定番() {
        let mut s = sheet_with(&[
            ("A1", "10"), ("A2", "20"), ("A3", "30"), ("A4", "文字"),
        ]);
        assert_eq!(n(&mut s, "=SUBTOTAL(9,A1:A4)"), 60.0, "9=SUM");
        assert_eq!(n(&mut s, "=SUBTOTAL(109,A1:A4)"), 60.0, "109 も SUM と同じに扱う");
        assert_eq!(n(&mut s, "=SUBTOTAL(1,A1:A3)"), 20.0, "1=AVERAGE");
        assert_eq!(n(&mut s, "=SUBTOTAL(2,A1:A4)"), 3.0, "2=COUNT(数だけ)");
        assert_eq!(n(&mut s, "=SUBTOTAL(3,A1:A4)"), 4.0, "3=COUNTA");
        assert_eq!(n(&mut s, "=SUBTOTAL(4,A1:A3)"), 30.0);
        assert_eq!(n(&mut s, "=SUBTOTAL(5,A1:A3)"), 10.0);
    }

    #[test]
    fn 新名と選択の関数() {
        let mut s = sheet_with(&[("A1", "70"), ("A2", "90"), ("A3", "90")]);
        assert_eq!(t(&mut s, "=CONCAT(\"防火\",\"戸\")"), "防火戸");
        assert_eq!(t(&mut s, "=IFNA(NA(),\"無し\")"), "無し");
        assert_eq!(value_of(&mut s, "=ISNA(NA())"), Value::Bool(true));
        assert_eq!(value_of(&mut s, "=ISNA(1/0)"), Value::Bool(false), "#DIV/0! は NA でない");
        assert_eq!(value_of(&mut s, "=ISERR(1/0)"), Value::Bool(true));
        assert_eq!(n(&mut s, "=RANK.EQ(90,A1:A3)"), 1.0);
        assert_eq!(n(&mut s, "=RANK.AVG(90,A1:A3)"), 1.5, "同値2つの順位の平均");
        assert_eq!(value_of(&mut s, "=TRUE()"), Value::Bool(true), "括弧つきの TRUE()");
        assert_eq!(t(&mut s, "=HYPERLINK(\"https://例\",\"表示名\")"), "表示名");
    }

    #[test]
    fn 新しい丸めと商() {
        let mut s = sheet_with(&[]);
        assert_eq!(n(&mut s, "=CEILING.MATH(6.1)"), 7.0, "基準の既定は1");
        assert_eq!(n(&mut s, "=CEILING.MATH(-6.1,2)"), -6.0, "負は0へ寄るのが既定");
        assert_eq!(n(&mut s, "=FLOOR.MATH(-6.1,2)"), -8.0);
        assert_eq!(n(&mut s, "=QUOTIENT(7,2)"), 3.0);
        assert_eq!(n(&mut s, "=QUOTIENT(-7,2)"), -3.0, "商は0へ切る");
        assert_eq!(value_of(&mut s, "=QUOTIENT(7,0)"), Value::Error("#DIV/0!".into()));
    }

    #[test]
    fn 古典のlookupとtranspose() {
        let mut s = sheet_with(&[
            ("A1", "10"), ("B1", "甲"),
            ("A2", "20"), ("B2", "乙"),
            ("A3", "30"), ("B3", "丙"),
            ("D1", "=LOOKUP(25,A1:A3,B1:B3)"),
            ("E1", "=TRANSPOSE(A1:B3)"),
        ]);
        recalc(&mut s);
        assert_eq!(s.value(Pos::parse("D1").unwrap()), Value::Text("乙".into()),
            "25以下でいちばん大きい 20 の行");
        // 3行2列 → 2行3列にあふれる
        assert_eq!(s.value(Pos::parse("E1").unwrap()), Value::Number(10.0));
        assert_eq!(s.value(Pos::parse("G1").unwrap()), Value::Number(30.0));
        assert_eq!(s.value(Pos::parse("E2").unwrap()), Value::Text("甲".into()));
        assert_eq!(s.value(Pos::parse("G2").unwrap()), Value::Text("丙".into()));
    }

    #[test]
    fn 日付の週と日数() {
        let mut s = sheet_with(&[]);
        assert_eq!(n(&mut s, "=DAYS(DATE(2026,8,5),DATE(2026,8,1))"), 4.0);
        assert_eq!(n(&mut s, "=DAYS360(DATE(2026,1,31),DATE(2026,3,1))"), 31.0,
            "30/360 の数え方");
        assert!((n(&mut s, "=YEARFRAC(DATE(2026,1,1),DATE(2026,7,1))") - 0.5).abs() < 1e-9);
        // 2026-01-01 は木曜 → 第1週。2026-08-05 は?(自前の暦で数える)
        assert_eq!(n(&mut s, "=WEEKNUM(DATE(2026,1,1))"), 1.0);
        assert_eq!(n(&mut s, "=ISOWEEKNUM(DATE(2026,1,1))"), 1.0,
            "木曜を含む週が ISO の第1週");
        assert_eq!(t(&mut s, "=ADDRESS(5,2)"), "$B$5");
        assert_eq!(t(&mut s, "=ADDRESS(5,2,4)"), "B5", "4=相対");
    }

    #[test]
    fn 財務の反復解() {
        let mut s = sheet_with(&[
            ("A1", "-1000"), ("A2", "500"), ("A3", "500"), ("A4", "500"),
        ]);
        // IRR: -1000 + 500/(1+r) + 500/(1+r)^2 + 500/(1+r)^3 = 0 → 約 23.4%
        let irr = n(&mut s, "=IRR(A1:A4)");
        assert!((irr - 0.2337).abs() < 0.001, "IRR がずれる: {irr}");
        // RATE: PMT(0.01,60,1000000) の逆算 → 月利1%
        let rate = n(&mut s, "=RATE(60,-22244.4477,1000000)");
        assert!((rate - 0.01).abs() < 1e-6, "RATE がずれる: {rate}");
        // NPV: 利率0なら単純合計
        assert!((n(&mut s, "=NPV(0,A2:A4)") - 1500.0).abs() < 1e-9);
    }

    #[test]
    fn 文字の道具の残り() {
        let mut s = sheet_with(&[]);
        assert_eq!(t(&mut s, "=PROPER(\"hello world\")"), "Hello World");
        assert_eq!(value_of(&mut s, "=EXACT(\"Abc\",\"abc\")"), Value::Bool(false));
        assert_eq!(value_of(&mut s, "=EXACT(\"戸\",\"戸\")"), Value::Bool(true));
        assert_eq!(t(&mut s, "=FIXED(1234.567,1)"), "1,234.6");
        assert_eq!(t(&mut s, "=YEN(1234567)"), "¥1,234,567.00");
        assert_eq!(t(&mut s, "=YEN(1234567,0)"), "¥1,234,567");
        assert_eq!(n(&mut s, "=NUMBERVALUE(\"1.234,56\",\",\",\".\")"), 1234.56,
            "欧州式の区切りも読める");
        assert_eq!(t(&mut s, "=T(\"文字\")"), "文字");
        assert_eq!(t(&mut s, "=T(123)"), "");
        assert_eq!(n(&mut s, "=N(TRUE)"), 1.0);
        assert_eq!(n(&mut s, "=TYPE(\"a\")"), 2.0);
        assert_eq!(n(&mut s, "=TYPE(1/0)"), 16.0, "エラーの型は16");
        assert_eq!(t(&mut s, "=UNICHAR(12354)"), "あ");
        assert_eq!(n(&mut s, "=UNICODE(\"あ\")"), 12354.0);
    }

    #[test]
    fn バイト数の一族は全角を2と数える() {
        let mut s = sheet_with(&[]);
        assert_eq!(n(&mut s, "=LENB(\"防火戸\")"), 6.0);
        assert_eq!(n(&mut s, "=LENB(\"abc\")"), 3.0);
        assert_eq!(n(&mut s, "=LENB(\"ｱｲｳ\")"), 3.0, "半角カナは1");
        assert_eq!(t(&mut s, "=LEFTB(\"防火戸\",4)"), "防火");
        assert_eq!(t(&mut s, "=LEFTB(\"防火戸\",3)"), "防", "半端な1バイトは取らない");
        assert_eq!(t(&mut s, "=RIGHTB(\"防火戸\",2)"), "戸");
        assert_eq!(t(&mut s, "=MIDB(\"防火戸\",3,2)"), "火");
    }

    #[test]
    fn 全角半角の変換() {
        let mut s = sheet_with(&[]);
        assert_eq!(t(&mut s, "=ASC(\"ＡＢＣ１２３\")"), "ABC123");
        assert_eq!(t(&mut s, "=ASC(\"カタカナ\")"), "ｶﾀｶﾅ");
        assert_eq!(t(&mut s, "=ASC(\"ガンダム\")"), "ｶﾞﾝﾀﾞﾑ", "濁点は2文字に割れる");
        assert_eq!(t(&mut s, "=JIS(\"ｶﾞﾝﾀﾞﾑ\")"), "ガンダム", "濁点が1文字に組み直る");
        assert_eq!(t(&mut s, "=JIS(\"abc 123\")"), "ａｂｃ　１２３");
        // 往復して戻る
        assert_eq!(t(&mut s, "=JIS(ASC(\"パピプペポ・ヴ\"))"), "パピプペポ・ヴ");
    }

    #[test]
    fn 和暦の文字() {
        let mut s = sheet_with(&[]);
        assert_eq!(t(&mut s, "=DATESTRING(DATE(2026,8,5))"), "令和08年08月05日");
        assert_eq!(t(&mut s, "=DATESTRING(DATE(1989,1,7))"), "昭和64年01月07日",
            "改元の前日は前の元号");
        assert_eq!(t(&mut s, "=DATESTRING(DATE(1989,1,8))"), "平成01年01月08日");
        assert_eq!(t(&mut s, "=DATESTRING(DATE(2019,5,1))"), "令和01年05月01日");
    }

    #[test]
    fn aつきの統計は文字を0と数える() {
        let mut s = sheet_with(&[("A1", "10"), ("A2", "文字"), ("A3", "20")]);
        assert_eq!(n(&mut s, "=AVERAGEA(A1:A3)"), 10.0, "(10+0+20)/3");
        assert_eq!(n(&mut s, "=MAXA(A1:A3)"), 20.0);
        assert_eq!(n(&mut s, "=MINA(A1:A3)"), 0.0, "文字の0が最小");
    }
}

/// 残件の掃討(2026-08-05)— 和暦の表示形式・配列の入れ子・
/// ふりがな・別のシートへの間接参照。
#[cfg(test)]
mod dan5_tests {
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

    fn t(s: &mut Sheet, f: &str) -> String {
        match value_of(s, f) {
            Value::Text(x) => x,
            v => panic!("{f} が文字でない: {v:?}"),
        }
    }

    #[test]
    fn 和暦の表示形式() {
        let mut s = sheet_with(&[]);
        assert_eq!(t(&mut s, "=TEXT(DATE(2026,8,5),\"ggge年m月d日\")"), "令和8年8月5日");
        assert_eq!(t(&mut s, "=TEXT(DATE(2026,8,5),\"gge\")"), "令8");
        assert_eq!(t(&mut s, "=TEXT(DATE(2026,8,5),\"ge\")"), "R8");
        assert_eq!(t(&mut s, "=TEXT(DATE(1989,1,7),\"ggge年\")"), "昭和64年");
        assert_eq!(t(&mut s, "=TEXT(DATE(2026,8,5),\"ggg ee\")"), "令和 08", "ee は0詰め");
    }

    #[test]
    fn 配列を式の中に混ぜられる() {
        let mut s = sheet_with(&[
            ("A1", "10"), ("B1", "1"),
            ("A2", "20"), ("B2", "0"),
            ("A3", "30"), ("B3", "1"),
        ]);
        assert_eq!(value_of(&mut s, "=SUM(FILTER(A1:A3,B1:B3))"), Value::Number(40.0),
            "SUM(FILTER(…)) の定番が通らない");
        assert_eq!(value_of(&mut s, "=COUNTA(UNIQUE(B1:B3))"), Value::Number(2.0));
        assert_eq!(value_of(&mut s, "=SUM(SEQUENCE(10))"), Value::Number(55.0));
        assert_eq!(value_of(&mut s, "=SUM(FILTER(A1:A3,B1:B3))+1"), Value::Number(41.0),
            "集計に食わせた残りの四則も通る");
    }

    #[test]
    fn ふりがなが読めて往復する() {
        let mut book = crate::Book::new();
        book.sheets[0] = sheet_with(&[("A1", "日本"), ("A2", "ふりがな無し")]);
        book.sheets[0].name = "Sheet1".into();
        book.sheets[0].phonetics.insert(Pos::parse("A1").unwrap(), "ニホン".into());
        // PHONETIC 関数: 読みがあれば読み、無ければ字そのもの
        let s = &mut book.sheets[0];
        assert_eq!(value_of(s, "=PHONETIC(A1)"), Value::Text("ニホン".into()));
        assert_eq!(value_of(s, "=PHONETIC(A2)"), Value::Text("ふりがな無し".into()));
        // xlsx を往復しても読みが残る(rPh — 欧米の実装が落とす宝)
        let mut buf = std::io::Cursor::new(Vec::new());
        crate::xlsx::write(&book, &mut buf).unwrap();
        let (back, _) = crate::xlsx::read(std::io::Cursor::new(buf.into_inner())).unwrap();
        assert_eq!(
            back.sheets[0].phonetics.get(&Pos::parse("A1").unwrap()),
            Some(&"ニホン".to_string()),
            "ふりがなが保存で落ちた"
        );
    }

    #[test]
    fn 別のシートへの間接参照() {
        let mut book = crate::Book::new();
        book.sheets[0] = sheet_with(&[("A1", "=INDIRECT(\"台帳!B2\")"),
            ("A2", "=SUM(INDIRECT(\"台帳!B1:B3\"))"),
            ("A3", "=INDIRECT(\"'台帳'!B2\")")]);
        book.sheets[0].name = "表紙".into();
        let mut daicho = sheet_with(&[("B1", "10"), ("B2", "20"), ("B3", "=B1+B2")]);
        daicho.name = "台帳".into();
        book.sheets.push(daicho);
        recalc_all(&mut book);
        let v = |a1: &str| book.sheets[0].value(Pos::parse(a1).unwrap());
        assert_eq!(v("A1"), Value::Number(20.0), "別のシートの1セルが引けない");
        assert_eq!(v("A2"), Value::Number(60.0), "別のシートの範囲が SUM に渡らない");
        assert_eq!(v("A3"), Value::Number(20.0), "'名前'! の引用が剥けない");
        // 1枚だけの再計算では正直に #REF!
        let mut alone = sheet_with(&[("A1", "=INDIRECT(\"台帳!B2\")")]);
        recalc(&mut alone);
        assert_eq!(alone.value(Pos::parse("A1").unwrap()), Value::Error("#REF!".into()));
    }

    #[test]
    fn 別のシートを間接参照する三つ引数のsumif() {
        // 実物の xlsx で出た形。条件範囲と合計範囲が別々に INDIRECT で来る
        let mut book = crate::Book::new();
        book.sheets[0] =
            sheet_with(&[("A1", "=SUMIF(INDIRECT(\"4月!A1:A3\"),\"りんご\",INDIRECT(\"4月!B1:B3\"))")]);
        book.sheets[0].name = "表紙".into();
        let mut april = sheet_with(&[
            ("A1", "りんご"), ("B1", "100"),
            ("A2", "みかん"), ("B2", "200"),
            ("A3", "りんご"), ("B3", "50"),
        ]);
        april.name = "4月".into();
        book.sheets.push(april);
        recalc_all(&mut book);
        assert_eq!(
            book.sheets[0].value(Pos::parse("A1").unwrap()),
            Value::Number(150.0),
            "3引数 SUMIF が黙って違う数を返している"
        );
    }
}

/// 配列数式と演算子の組み合わせ(2026-08-05)。
#[cfg(test)]
mod dan6_tests {
    use super::*;
    use crate::model::Cell;

    fn sheet_with(cells: &[(&str, &str)]) -> Sheet {
        let mut s = Sheet { name: "表".into(), ..Default::default() };
        for (a1, v) in cells {
            s.set(Pos::parse(a1).unwrap(), Cell::input(v));
        }
        s
    }

    fn v(s: &Sheet, a1: &str) -> Value {
        s.value(Pos::parse(a1).unwrap())
    }

    #[test]
    fn 要素ごとの四則と文字連結() {
        let mut s = sheet_with(&[
            ("A1", "10"), ("A2", "20"), ("A3", "30"),
            ("C1", "=SEQUENCE(3,1)*10+A1:A3"),
            ("E1", "=\"第\"&SEQUENCE(3,1)&\"回\""),
        ]);
        recalc(&mut s);
        assert_eq!(v(&s, "C1"), Value::Number(20.0), "10*1+10");
        assert_eq!(v(&s, "C2"), Value::Number(40.0));
        assert_eq!(v(&s, "C3"), Value::Number(60.0));
        assert_eq!(v(&s, "E1"), Value::Text("第1回".into()));
        assert_eq!(v(&s, "E3"), Value::Text("第3回".into()));
    }

    #[test]
    fn 比較と括弧() {
        let mut s = sheet_with(&[
            ("A1", "=SEQUENCE(3,1)>=2"),
            ("C1", "=(SEQUENCE(2,1)+1)*2"),
        ]);
        recalc(&mut s);
        assert_eq!(v(&s, "A1"), Value::Bool(false));
        assert_eq!(v(&s, "A2"), Value::Bool(true));
        assert_eq!(v(&s, "A3"), Value::Bool(true));
        assert_eq!(v(&s, "C1"), Value::Number(4.0));
        assert_eq!(v(&s, "C2"), Value::Number(6.0));
    }

    #[test]
    fn 形が合わない要素はエラーになる() {
        // {1;2;3} + {1;2} → 3行目は #N/A(Excel の配列数式と同じ)
        let mut s = sheet_with(&[("A1", "=SEQUENCE(3,1)+SEQUENCE(2,1)")]);
        recalc(&mut s);
        assert_eq!(v(&s, "A1"), Value::Number(2.0));
        assert_eq!(v(&s, "A2"), Value::Number(4.0));
        assert_eq!(v(&s, "A3"), Value::Error("#N/A".into()));
    }

    #[test]
    fn 引数の中でも要素ごとに計算できる() {
        let mut s = sheet_with(&[
            ("A1", "1"), ("A2", "2"), ("A3", "3"),
            ("C1", "=SUM(SEQUENCE(3,1)*2)"),
            ("C2", "=SUM(A1:A3*10)"),
            ("C3", "=SUMPRODUCT(A1:A3,A1:A3)"),
        ]);
        recalc(&mut s);
        assert_eq!(v(&s, "C1"), Value::Number(12.0), "SUM(SEQUENCE*2)");
        assert_eq!(v(&s, "C2"), Value::Number(60.0), "範囲の要素ごとの倍が SUM に渡らない");
        assert_eq!(v(&s, "C3"), Value::Number(14.0), "既存の SUMPRODUCT はそのまま");
    }

    #[test]
    fn 集計に落ちれば1つの値のまま() {
        let mut s = sheet_with(&[("A1", "=SUM(SEQUENCE(3,1))+1"), ("B1", "9")]);
        recalc(&mut s);
        assert_eq!(v(&s, "A1"), Value::Number(7.0));
        assert!(s.spills.is_empty(), "1つの値なのにスピルの記録が残った");
        assert_eq!(v(&s, "B1"), Value::Number(9.0), "隣に何も書いていない");
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

/// 直書きの別シート参照(2026-08-08。それまでは `!` を読めず #ERROR! だった)。
/// 他所の xlsx にはこの形の式が並の頻度で入っている — 乗り換えの壁だった所
#[cfg(test)]
mod cross_sheet_tests {
    use super::*;
    use crate::model::Cell;

    fn sheet_named(name: &str, cells: &[(&str, &str)]) -> Sheet {
        let mut s = Sheet { name: name.into(), ..Default::default() };
        for (a1, v) in cells {
            s.set(Pos::parse(a1).unwrap(), Cell::input(v));
        }
        s
    }

    /// 表紙 + 4月 + '5月 実績' の3枚。表紙の式を引数で差し替えて値を見る
    fn ans(formula: &str) -> Value {
        let mut book = crate::Book::new();
        book.sheets[0] = sheet_named("表紙", &[("A1", formula)]);
        book.sheets.push(sheet_named("4月", &[("B1", "100"), ("B2", "200"), ("B3", "文")]));
        book.sheets.push(sheet_named("5月 実績", &[("B2", "50")]));
        recalc_all(&mut book);
        book.sheets[0].value(Pos::parse("A1").unwrap())
    }

    #[test]
    fn 直書きの別シート参照が解ける() {
        // 1セル・和文のシート名(Excel が普通に書く形)
        assert_eq!(ans("=4月!B1"), Value::Number(100.0));
        // 範囲は関数の中で並びとして渡る
        assert_eq!(ans("=SUM(4月!B1:B2)"), Value::Number(300.0));
        assert_eq!(ans("=COUNTA(4月!B1:B3)"), Value::Number(3.0));
        // 式の中で他の値と混ぜられる
        assert_eq!(ans("=4月!B1*2+1"), Value::Number(201.0));
        // 引用符つき(空白を含む名前)
        assert_eq!(ans("='5月 実績'!B2"), Value::Number(50.0));
        // 自分のシート名は普通の参照として働く
        let mut book = crate::Book::new();
        book.sheets[0] = sheet_named("表紙", &[("A1", "=表紙!C3"), ("C3", "7")]);
        recalc_all(&mut book);
        assert_eq!(book.sheets[0].value(Pos::parse("A1").unwrap()), Value::Number(7.0));
    }

    #[test]
    fn 知らないシートと1枚だけの計算は参照エラー() {
        // 黙って自分のシートと読まない
        assert_eq!(ans("=無い月!B1"), Value::Error("#REF!".into()));
        // 1枚だけの再計算(others が空)でも #REF! — 嘘の値を出さない
        let mut only = sheet_named("表紙", &[("A1", "=4月!B1")]);
        recalc(&mut only);
        assert_eq!(only.value(Pos::parse("A1").unwrap()), Value::Error("#REF!".into()));
    }

    #[test]
    fn 既存の書き方を壊していない() {
        // INDIRECT の道は今までどおり
        assert_eq!(ans("=INDIRECT(\"4月!B1\")"), Value::Number(100.0));
        assert_eq!(ans("=SUM(INDIRECT(\"4月!B1:B2\"))"), Value::Number(300.0));
        // 同じシートの参照・範囲・関数名は `!` を足しても変わらない
        let mut s = sheet_named("表紙", &[
            ("A1", "10"), ("A2", "20"),
            ("B1", "=SUM(A1:A2)"), ("B2", "=A1<>A2"), ("B3", "=NOT(A1=A2)"),
        ]);
        recalc(&mut s);
        assert_eq!(s.value(Pos::parse("B1").unwrap()), Value::Number(30.0));
        assert_eq!(s.value(Pos::parse("B2").unwrap()), Value::Bool(true));
        assert_eq!(s.value(Pos::parse("B3").unwrap()), Value::Bool(true));
    }

    #[test]
    fn 別シートを見る式どうしが連鎖しても解ける() {
        // 4月!B1 → 集計!A1 → 表紙!A1 の2段(再計算の周回が足りるか)
        let mut book = crate::Book::new();
        book.sheets[0] = sheet_named("表紙", &[("A1", "=集計!A1+1")]);
        book.sheets.push(sheet_named("集計", &[("A1", "=4月!B1*2")]));
        book.sheets.push(sheet_named("4月", &[("B1", "100")]));
        recalc_all(&mut book);
        assert_eq!(book.sheets[0].value(Pos::parse("A1").unwrap()), Value::Number(201.0));
    }
}

/// SUBTOTAL/AGGREGATE の 101〜111(隠した行を飛ばす)。2026-08-08 実装 —
/// それまでは 1〜11 と同じに扱っていて、畳んだ台帳で黙って違う数が出ていた
#[cfg(test)]
mod subtotal_hidden_tests {
    use super::*;
    use crate::model::Cell;

    fn sheet4() -> Sheet {
        let mut s = Sheet { name: "表".into(), ..Default::default() };
        for (a1, v) in [("A1", "10"), ("A2", "20"), ("A3", "30"), ("A4", "40")] {
            s.set(Pos::parse(a1).unwrap(), Cell::input(v));
        }
        s
    }

    fn val(s: &mut Sheet, f: &str) -> Value {
        s.set(Pos::parse("C1").unwrap(), Cell::input(f));
        recalc(s);
        s.value(Pos::parse("C1").unwrap())
    }

    #[test]
    fn 隠した行は百番台だけが飛ばす() {
        let mut s = sheet4();
        // 2行目(A2=20)を畳む
        s.row_hidden.insert(1);
        // 9 = SUM(全部数える)/ 109 = SUM(隠した行を飛ばす)
        assert_eq!(val(&mut s, "=SUBTOTAL(9,A1:A4)"), Value::Number(100.0));
        assert_eq!(val(&mut s, "=SUBTOTAL(109,A1:A4)"), Value::Number(80.0), "隠した行を飛ばしていない");
        // 平均・個数・最大・最小も同じ規則
        assert_eq!(val(&mut s, "=SUBTOTAL(101,A1:A4)"), Value::Number(80.0 / 3.0));
        assert_eq!(val(&mut s, "=SUBTOTAL(102,A1:A4)"), Value::Number(3.0));
        assert_eq!(val(&mut s, "=SUBTOTAL(104,A1:A4)"), Value::Number(40.0));
        assert_eq!(val(&mut s, "=SUBTOTAL(105,A1:A4)"), Value::Number(10.0));
        // AGGREGATE も同じ(第2引数は無視の指定)
        assert_eq!(val(&mut s, "=AGGREGATE(109,0,A1:A4)"), Value::Number(80.0));
        assert_eq!(val(&mut s, "=AGGREGATE(9,0,A1:A4)"), Value::Number(100.0));
    }

    #[test]
    fn 隠した行が無ければ今までどおり() {
        let mut s = sheet4();
        assert_eq!(val(&mut s, "=SUBTOTAL(9,A1:A4)"), Value::Number(100.0));
        assert_eq!(val(&mut s, "=SUBTOTAL(109,A1:A4)"), Value::Number(100.0));
        // 隠れ行を飛ばすのは SUBTOTAL の中だけ — 普通の SUM は影響を受けない
        let mut s2 = sheet4();
        s2.row_hidden.insert(1);
        assert_eq!(val(&mut s2, "=SUM(A1:A4)"), Value::Number(100.0));
        assert_eq!(val(&mut s2, "=AVERAGE(A1:A4)"), Value::Number(25.0));
    }
}

/// 構造化参照(2026-08-08 実装。台帳 第3便 [中])。
/// 表オブジェクトの列を見出しの字で引く — Excel の `=SUM(Table1[金額])`
#[cfg(test)]
mod table_ref_tests {
    use super::*;
    use crate::model::{Cell, TableDef};

    /// A1:C4 の表(見出し + 3行)。名前は「売上表」
    fn with_table(totals: bool) -> Sheet {
        let mut s = Sheet { name: "表".into(), ..Default::default() };
        let rows = [
            ("A1", "品名"), ("B1", "数量"), ("C1", "金額"),
            ("A2", "筆"), ("B2", "2"), ("C2", "100"),
            ("A3", "紙"), ("B3", "3"), ("C3", "200"),
            ("A4", "机"), ("B4", "1"), ("C4", "900"),
        ];
        for (a1, v) in rows {
            s.set(Pos::parse(a1).unwrap(), Cell::input(v));
        }
        if totals {
            s.set(Pos::parse("A5").unwrap(), Cell::input("合計"));
            s.set(Pos::parse("C5").unwrap(), Cell::input("1200"));
        }
        s.tables.push(TableDef {
            name: "売上表".into(),
            a: Pos::parse("A1").unwrap(),
            b: Pos::parse(if totals { "C5" } else { "C4" }).unwrap(),
            header: true,
            totals,
            ..Default::default()
        });
        s
    }

    fn at(s: &mut Sheet, cell: &str, f: &str) -> Value {
        s.set(Pos::parse(cell).unwrap(), Cell::input(f));
        recalc(s);
        s.value(Pos::parse(cell).unwrap())
    }

    #[test]
    fn 表の列を見出しの字で引ける() {
        let mut s = with_table(false);
        assert_eq!(at(&mut s, "E1", "=SUM(売上表[金額])"), Value::Number(1200.0));
        assert_eq!(at(&mut s, "E2", "=AVERAGE(売上表[数量])"), Value::Number(2.0));
        assert_eq!(at(&mut s, "E3", "=COUNTA(売上表[品名])"), Value::Number(3.0));
        // 単独なら先頭の値
        assert_eq!(at(&mut s, "E4", "=売上表[金額]"), Value::Number(100.0));
        // 知らない列・知らない表は #REF!(黙って違う所を読まない)
        assert_eq!(at(&mut s, "E5", "=SUM(売上表[無い列])"), Value::Error("#REF!".into()));
        assert_eq!(at(&mut s, "E6", "=SUM(無い表[金額])"), Value::Error("#REF!".into()));
    }

    #[test]
    fn 合計行はデータ本体から外れる() {
        let mut s = with_table(true);
        // C5 の 1200 は合計行なので二重に数えない
        assert_eq!(at(&mut s, "E1", "=SUM(売上表[金額])"), Value::Number(1200.0));
    }

    #[test]
    fn この行の参照は同じ行の列を指す() {
        let mut s = with_table(false);
        // 表を D 列(税)まで広げて、その中で [@金額] を使う。
        // **名前を省いた形は表の中でだけ効く**(Excel と同じ)
        s.set(Pos::parse("D1").unwrap(), Cell::input("税"));
        s.tables[0].b = Pos::parse("D4").unwrap();
        assert_eq!(at(&mut s, "D3", "=[@金額]*2"), Value::Number(400.0));
        // 表の名前つきなら表の外の同じ行からも引ける
        assert_eq!(at(&mut s, "E3", "=売上表[@数量]"), Value::Number(3.0));
        // 見出しの行では引けない
        assert_eq!(at(&mut s, "E1", "=売上表[@金額]"), Value::Error("#REF!".into()));
        // 表の外で名前を省いたら引けない(どの表か決まらない)
        assert_eq!(at(&mut s, "G3", "=[@金額]"), Value::Error("#REF!".into()));
    }

    #[test]
    fn 表が無ければ今までどおりの読み方() {
        // 表オブジェクトが無いシートで [ が出たら式のエラー(黙って0にしない)
        let mut s = Sheet { name: "表".into(), ..Default::default() };
        s.set(Pos::parse("A1").unwrap(), Cell::input("=SUM(無い表[金額])"));
        recalc(&mut s);
        assert_eq!(s.value(Pos::parse("A1").unwrap()), Value::Error("#REF!".into()));
    }
}

/// LET(2026-08-08 実装。台帳 第3便 [中])。
/// 長い式の途中結果に名前を付けて、読みやすく・二度計算しない
#[cfg(test)]
mod let_tests {
    use super::*;
    use crate::model::Cell;

    fn v(f: &str) -> Value {
        let mut s = Sheet { name: "表".into(), ..Default::default() };
        for (a1, x) in [("A1", "10"), ("A2", "20"), ("A3", "30")] {
            s.set(Pos::parse(a1).unwrap(), Cell::input(x));
        }
        s.set(Pos::parse("E1").unwrap(), Cell::input(f));
        recalc(&mut s);
        s.value(Pos::parse("E1").unwrap())
    }

    #[test]
    fn 名前を束ねて式に使える() {
        assert_eq!(v("=LET(x,5,x*2)"), Value::Number(10.0));
        // 複数の束縛。後の束縛から前の名前が見える
        assert_eq!(v("=LET(x,5,y,x+1,x*y)"), Value::Number(30.0));
        // セルや関数の結果も束ねられる(二度計算しないのが本来の狙い)
        assert_eq!(v("=LET(s,SUM(A1:A3),s/3)"), Value::Number(20.0));
        // 本体が名前1つだけ(次が `)` なので束縛と取り違えない)
        assert_eq!(v("=LET(x,7,x)"), Value::Number(7.0));
        // 入れ子。内側の同じ名前が外側を隠す
        assert_eq!(v("=LET(x,1,LET(x,2,x))"), Value::Number(2.0));
        // 内側を抜けたら外側の名前に戻る
        assert_eq!(v("=LET(x,1,LET(y,2,y)+x)"), Value::Number(3.0));
    }

    #[test]
    fn 文字と論理値も束ねられる() {
        assert_eq!(v("=LET(t,\"あ\",t&\"い\")"), Value::Text("あい".into()));
        assert_eq!(v("=LET(b,A1>5,IF(b,\"大\",\"小\"))"), Value::Text("大".into()));
    }

    #[test]
    fn 形が違えば正直に断る() {
        // 名前と値だけで本体が無い
        assert_eq!(v("=LET(x,5)"), Value::Error("#VALUE!".into()));
        // LET の外へ名前は漏れない
        assert_eq!(v("=LET(x,5,x)+x"), Value::Error("#NAME?".into()));
        // 知らない名前は今までどおり #NAME?
        assert_eq!(v("=UNKNOWNNAME+1"), Value::Error("#NAME?".into()));
        // 和文の知らない名前は字句で落ちて #ERROR!(LET の前からこう。
        // どちらもエラーで黙って計算はしない — 印の揃え方は在庫)
        assert_eq!(v("=しらない名前+1"), Value::Error("#ERROR!".into()));
    }
}

/// TEXTSPLIT / TEXTBEFORE / TEXTAFTER(2026-08-08 実装。台帳 第3便 [中])
#[cfg(test)]
mod text_split_tests {
    use super::*;
    use crate::model::Cell;

    fn v(f: &str) -> Value {
        let mut s = Sheet { name: "表".into(), ..Default::default() };
        s.set(Pos::parse("E1").unwrap(), Cell::input(f));
        recalc(&mut s);
        s.value(Pos::parse("E1").unwrap())
    }

    #[test]
    fn 区切りの前と後ろを取れる() {
        assert_eq!(v("=TEXTBEFORE(\"甲-乙-丙\",\"-\")"), Value::Text("甲".into()));
        assert_eq!(v("=TEXTAFTER(\"甲-乙-丙\",\"-\")"), Value::Text("乙-丙".into()));
        // 何番目か(2つ目の区切り)
        assert_eq!(v("=TEXTBEFORE(\"甲-乙-丙\",\"-\",2)"), Value::Text("甲-乙".into()));
        assert_eq!(v("=TEXTAFTER(\"甲-乙-丙\",\"-\",2)"), Value::Text("丙".into()));
        // 負は後ろから
        assert_eq!(v("=TEXTAFTER(\"甲-乙-丙\",\"-\",-1)"), Value::Text("丙".into()));
        assert_eq!(v("=TEXTBEFORE(\"甲-乙-丙\",\"-\",-1)"), Value::Text("甲-乙".into()));
        // 見つからなければ #N/A、4つ目を渡せばその値
        assert_eq!(v("=TEXTBEFORE(\"甲乙\",\"-\")"), Value::Error("#N/A".into()));
        assert_eq!(v("=TEXTBEFORE(\"甲乙\",\"-\",1,\"無\")"), Value::Text("無".into()));
        // 区切りが空・0番目は #VALUE!(黙って全部を返さない)
        assert_eq!(v("=TEXTBEFORE(\"甲乙\",\"\")"), Value::Error("#VALUE!".into()));
        assert_eq!(v("=TEXTAFTER(\"甲-乙\",\"-\",0)"), Value::Error("#VALUE!".into()));
    }

    #[test]
    fn textsplitは横へ広がる() {
        let mut s = Sheet { name: "表".into(), ..Default::default() };
        s.set(Pos::parse("A1").unwrap(), Cell::input("=TEXTSPLIT(\"甲,乙,丙\",\",\")"));
        recalc(&mut s);
        assert_eq!(s.value(Pos::parse("A1").unwrap()), Value::Text("甲".into()));
        assert_eq!(s.value(Pos::parse("B1").unwrap()), Value::Text("乙".into()));
        assert_eq!(s.value(Pos::parse("C1").unwrap()), Value::Text("丙".into()));
    }

    #[test]
    fn 行の区切りで縦にも割れる() {
        let mut s = Sheet { name: "表".into(), ..Default::default() };
        s.set(
            Pos::parse("A1").unwrap(),
            Cell::input("=TEXTSPLIT(\"甲,乙;丙,丁\",\",\",\";\")"),
        );
        recalc(&mut s);
        assert_eq!(s.value(Pos::parse("A1").unwrap()), Value::Text("甲".into()));
        assert_eq!(s.value(Pos::parse("B1").unwrap()), Value::Text("乙".into()));
        assert_eq!(s.value(Pos::parse("A2").unwrap()), Value::Text("丙".into()));
        assert_eq!(s.value(Pos::parse("B2").unwrap()), Value::Text("丁".into()));
        // 区切りが両方とも空なら #VALUE!
        let mut s2 = Sheet { name: "表".into(), ..Default::default() };
        s2.set(Pos::parse("A1").unwrap(), Cell::input("=TEXTSPLIT(\"甲乙\",\"\")"));
        recalc(&mut s2);
        assert_eq!(s2.value(Pos::parse("A1").unwrap()), Value::Error("#VALUE!".into()));
    }
}

/// 串刺し集計(2026-08-08 実装。台帳 第3便 [中])。
/// `=SUM(4月:6月!B2)` — ブックの並び順で2枚の間の全シートを集める
#[cfg(test)]
mod sheet3_tests {
    use super::*;
    use crate::model::Cell;

    /// 表紙 / 4月 / 5月 / 6月 / 別 の5枚(この並び)
    fn book5(formula: &str) -> crate::Book {
        let mut b = crate::Book::new();
        let mut cover = Sheet { name: "表紙".into(), ..Default::default() };
        cover.set(Pos::parse("A1").unwrap(), Cell::input(formula));
        b.sheets[0] = cover;
        for (n, v) in [("4月", "10"), ("5月", "20"), ("6月", "30"), ("別", "999")] {
            let mut s = Sheet { name: n.into(), ..Default::default() };
            s.set(Pos::parse("B2").unwrap(), Cell::input(v));
            b.sheets.push(s);
        }
        b
    }

    fn ans(formula: &str) -> Value {
        let mut b = book5(formula);
        recalc_all(&mut b);
        b.sheets[0].value(Pos::parse("A1").unwrap())
    }

    #[test]
    fn 並び順で二枚の間を集める() {
        // 4月〜6月 = 10+20+30(「別」の 999 は入らない)
        assert_eq!(ans("=SUM(4月:6月!B2)"), Value::Number(60.0));
        assert_eq!(ans("=SUM(4月:5月!B2)"), Value::Number(30.0));
        // 逆順に書いても同じ(Excel と同じ)
        assert_eq!(ans("=SUM(6月:4月!B2)"), Value::Number(60.0));
        // 1枚だけを挟む形
        assert_eq!(ans("=SUM(5月:5月!B2)"), Value::Number(20.0));
        // 平均・個数も同じ並びで効く
        assert_eq!(ans("=AVERAGE(4月:6月!B2)"), Value::Number(20.0));
        assert_eq!(ans("=COUNT(4月:6月!B2)"), Value::Number(3.0));
    }

    #[test]
    fn 自分のシートを跨いでも並び順が崩れない() {
        // 表紙(1枚目)を含む範囲。自分の A1 は式なので B2 を見る
        let mut b = book5("=SUM(表紙:5月!B2)");
        b.sheets[0].set(Pos::parse("B2").unwrap(), Cell::input("5"));
        recalc_all(&mut b);
        // 表紙 5 + 4月 10 + 5月 20
        assert_eq!(b.sheets[0].value(Pos::parse("A1").unwrap()), Value::Number(35.0));
    }

    #[test]
    fn 知らない名前と範囲の形() {
        assert_eq!(ans("=SUM(4月:無い月!B2)"), Value::Error("#REF!".into()));
        // 範囲を跨ぐ形(B2:B3)も集められる
        let mut b = book5("=SUM(4月:6月!B2:B3)");
        for (i, v) in [("4月", "1"), ("5月", "2"), ("6月", "3")] {
            let k = b.sheets.iter().position(|s| s.name == i).unwrap();
            b.sheets[k].set(Pos::parse("B3").unwrap(), Cell::input(v));
        }
        recalc_all(&mut b);
        // B2 の 10+20+30 と B3 の 1+2+3
        assert_eq!(b.sheets[0].value(Pos::parse("A1").unwrap()), Value::Number(66.0));
    }
}

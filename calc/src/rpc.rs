//! Python(jocalc)からの遠隔操作の口。
//!
//! Jupyter の xlwings 流の使い勝手(Book / Range / .value / DataFrame)を
//! **動いている calc** に向ける(発注者 2026-08-08 — Qiita の記事の車線)。
//! ユニックスソケット `$XDG_RUNTIME_DIR/jo-office/calc.sock` に JSON を
//! 1行ずつ。**この機械の中だけ**(TCP は開かない — ネイティブファースト)。
//!
//! 糸の作法: ソケットの糸は状態に触らない。要求を溜め、GPUI の泵(ポンプ)が
//! 30ms ごとに主の糸で捌いて答えを返す(Editor 系と同じ「主で触る」を守る)。

use crate::*;
use std::io::{BufRead, Write};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

pub(crate) struct RpcReq {
    pub line: String,
    pub reply: Sender<String>,
}

pub(crate) type RpcQueue = Arc<Mutex<Vec<RpcReq>>>;

/// ソケットの置き場所。`$XDG_RUNTIME_DIR/jo-office/calc.sock`。
/// AF_UNIX の径路は 108 字までなので、長すぎるときは
/// `/tmp/jo-office-UID/calc.sock` へ落とす(Python 側も同じ規則)
pub(crate) fn sock_path() -> std::path::PathBuf {
    if let Some(base) = std::env::var_os("XDG_RUNTIME_DIR") {
        let p = std::path::PathBuf::from(&base).join("jo-office").join("calc.sock");
        if p.as_os_str().len() <= 90 {
            return p;
        }
    }
    let uid = std::fs::metadata("/proc/self")
        .map(|m| std::os::unix::fs::MetadataExt::uid(&m))
        .unwrap_or(0);
    std::env::temp_dir().join(format!("jo-office-{uid}")).join("calc.sock")
}

/// 口を開く。聞き取りの糸を立て、主の糸に泵を付ける。
pub(crate) fn start(view: gpui::Entity<Calc>, cx: &mut gpui::App) {
    let queue: RpcQueue = Arc::new(Mutex::new(Vec::new()));
    let path = sock_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::remove_file(&path); // 前回の残骸
    let listener = match std::os::unix::net::UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            // 口が開けなくてもアプリは動く(黙らず標準エラーにだけ言う)
            eprintln!("jocalc の口が開けません: {e}");
            return;
        }
    };
    let q = queue.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(conn) = conn else { continue };
            let q = q.clone();
            std::thread::spawn(move || {
                let mut w = match conn.try_clone() {
                    Ok(c) => c,
                    Err(_) => return,
                };
                let r = std::io::BufReader::new(conn);
                for line in r.lines() {
                    let Ok(line) = line else { break };
                    if line.trim().is_empty() {
                        continue;
                    }
                    let (tx, rx) = std::sync::mpsc::channel();
                    q.lock().unwrap().push(RpcReq { line, reply: tx });
                    // 主の糸が捌くのを待つ(泵は 30ms 刻み。5秒で諦める)
                    let resp = rx
                        .recv_timeout(std::time::Duration::from_secs(5))
                        .unwrap_or_else(|_| {
                            r#"{"err":"calc が応じません(忙しいか、閉じかけ)"}"#.into()
                        });
                    if w.write_all(resp.as_bytes()).is_err() {
                        break;
                    }
                    let _ = w.write_all(b"\n");
                }
            });
        }
    });
    // 泵: 30ms ごとに溜まった要求を主の糸で捌く
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(30))
                .await;
            let reqs: Vec<RpcReq> = std::mem::take(&mut *queue.lock().unwrap());
            if reqs.is_empty() {
                continue;
            }
            view.update(cx, |calc, cx| {
                for req in reqs {
                    let resp = handle(calc, &req.line, cx);
                    let _ = req.reply.send(resp);
                }
                cx.notify();
            });
        }
    })
    .detach();
}

// ---- JSON の小さな読み書き(依存を増やさない — ooxml/crypt と同じ流儀) ----

/// ごく浅い JSON の値。この口の語彙(文字列・数・真偽・null・2次元の並び)だけ
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum J {
    S(String),
    N(f64),
    B(bool),
    Null,
    A(Vec<J>),
}

pub(crate) fn jesc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

impl J {
    fn to_json(&self) -> String {
        match self {
            J::S(s) => format!("\"{}\"", jesc(s)),
            J::N(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    format!("{}", *n as i64)
                } else {
                    format!("{n}")
                }
            }
            J::B(b) => b.to_string(),
            J::Null => "null".into(),
            J::A(xs) => {
                format!("[{}]", xs.iter().map(|x| x.to_json()).collect::<Vec<_>>().join(","))
            }
        }
    }
}

/// 1行の JSON オブジェクトから鍵を引く(浅い読み。入れ子の object は読まない)
pub(crate) struct Jobj<'a> {
    src: &'a str,
}

impl<'a> Jobj<'a> {
    pub fn parse(src: &'a str) -> Option<Jobj<'a>> {
        let t = src.trim();
        if t.starts_with('{') && t.ends_with('}') {
            Some(Jobj { src: t })
        } else {
            None
        }
    }
    /// "key" の値の始まりの位置(値の1文字目)
    fn value_start(&self, key: &str) -> Option<usize> {
        let pat = format!("\"{}\"", jesc(key));
        let mut from = 0;
        loop {
            let i = self.src[from..].find(&pat)? + from;
            // 鍵の後ろに : が来るか(文字列の中の偶然の一致は雑に弾く)
            let after = &self.src[i + pat.len()..];
            let colon = after.find(|c: char| !c.is_whitespace())?;
            if after[colon..].starts_with(':') {
                let v = &after[colon + 1..];
                let off = v.find(|c: char| !c.is_whitespace())?;
                return Some(i + pat.len() + colon + 1 + off);
            }
            from = i + pat.len();
        }
    }
    pub fn str(&self, key: &str) -> Option<String> {
        let at = self.value_start(key)?;
        let v = &self.src[at..];
        if !v.starts_with('"') {
            return None;
        }
        let mut o = String::new();
        let mut it = v[1..].chars();
        while let Some(c) = it.next() {
            match c {
                '"' => return Some(o),
                '\\' => match it.next()? {
                    'n' => o.push('\n'),
                    'r' => o.push('\r'),
                    't' => o.push('\t'),
                    'u' => {
                        let h: String = (0..4).filter_map(|_| it.next()).collect();
                        if let Ok(cp) = u32::from_str_radix(&h, 16) {
                            if let Some(ch) = char::from_u32(cp) {
                                o.push(ch);
                            }
                        }
                    }
                    c => o.push(c),
                },
                c => o.push(c),
            }
        }
        None
    }
    /// 2次元の並び(値は 文字列/数/真偽/null)。"values": [[...],[...]]
    pub fn grid(&self, key: &str) -> Option<Vec<Vec<J>>> {
        let at = self.value_start(key)?;
        let v = &self.src[at..];
        if !v.starts_with('[') {
            return None;
        }
        let mut rows: Vec<Vec<J>> = Vec::new();
        let mut row: Vec<J> = Vec::new();
        let mut depth = 0usize;
        let mut i = 0usize;
        let b = v.as_bytes();
        while i < v.len() {
            let c = b[i] as char;
            match c {
                '[' => {
                    depth += 1;
                    if depth == 2 {
                        row = Vec::new();
                    }
                    i += 1;
                }
                ']' => {
                    if depth == 2 {
                        rows.push(std::mem::take(&mut row));
                    }
                    depth -= 1;
                    i += 1;
                    if depth == 0 {
                        return Some(rows);
                    }
                }
                '"' => {
                    // 文字列
                    let mut o = String::new();
                    i += 1;
                    while i < v.len() {
                        let c = b[i] as char;
                        if c == '\\' {
                            // 逃げ
                            let n = v[i + 1..].chars().next()?;
                            match n {
                                'n' => o.push('\n'),
                                'r' => o.push('\r'),
                                't' => o.push('\t'),
                                'u' => {
                                    let h = v.get(i + 2..i + 6)?;
                                    if let Ok(cp) = u32::from_str_radix(h, 16) {
                                        if let Some(ch) = char::from_u32(cp) {
                                            o.push(ch);
                                        }
                                    }
                                    i += 4;
                                }
                                c => o.push(c),
                            }
                            i += 1 + n.len_utf8();
                        } else if c == '"' {
                            i += 1;
                            break;
                        } else {
                            let ch = v[i..].chars().next()?;
                            o.push(ch);
                            i += ch.len_utf8();
                        }
                    }
                    if depth == 2 {
                        row.push(J::S(o));
                    }
                }
                ',' | ' ' | '\t' | '\n' | '\r' => i += 1,
                _ => {
                    // 数・真偽・null
                    let end = v[i..]
                        .find(|c: char| c == ',' || c == ']' || c.is_whitespace())
                        .map(|e| i + e)
                        .unwrap_or(v.len());
                    let tok = &v[i..end];
                    let j = match tok {
                        "true" => J::B(true),
                        "false" => J::B(false),
                        "null" => J::Null,
                        t => J::N(t.parse().ok()?),
                    };
                    if depth == 2 {
                        row.push(j);
                    }
                    i = end;
                }
            }
        }
        None
    }
}

fn err(msg: &str) -> String {
    format!("{{\"err\":\"{}\"}}", jesc(msg))
}

/// 1要求を捌く(主の糸)。答えは JSON 1行。
pub(crate) fn handle(calc: &mut Calc, line: &str, cx: &mut Context<Calc>) -> String {
    let Some(o) = Jobj::parse(line) else {
        return err("JSON が読めません");
    };
    let Some(cmd) = o.str("cmd") else {
        return err("cmd がありません");
    };
    match cmd.as_str() {
        "ping" => "{\"ok\":true,\"app\":\"calc\"}".into(),
        // ブックの情報(名前・道・シートの一覧・いまのシート)
        "book_info" => {
            let sheets = J::A(
                calc.book
                    .sheets
                    .iter()
                    .map(|s| J::S(s.name.clone()))
                    .collect(),
            );
            format!(
                "{{\"ok\":true,\"path\":{},\"sheets\":{},\"active\":{}}}",
                match &calc.path {
                    Some(p) => J::S(p.display().to_string()).to_json(),
                    None => "null".into(),
                },
                sheets.to_json(),
                calc.active
            )
        }
        // 新しいブック(未保存の変更があれば断る — 黙って捨てない)
        "new" => {
            calc.commit();
            if calc.dirty {
                return err("calc に未保存の変更があります(保存するか、捨ててから)");
            }
            if !calc.new_book() {
                return err(&format!("{}", calc.status));
            }
            cx.notify();
            "{\"ok\":true}".into()
        }
        // ファイルを開く(同じく、未保存があれば断る)
        "open" => {
            let Some(p) = o.str("path") else { return err("path がありません") };
            calc.commit();
            if calc.dirty {
                return err("calc に未保存の変更があります(保存するか、捨ててから)");
            }
            calc.open(std::path::PathBuf::from(&p));
            if calc.path.as_deref() != Some(std::path::Path::new(&p)) {
                return err(&format!("開けません: {}", calc.status));
            }
            cx.notify();
            "{\"ok\":true}".into()
        }
        // 保存(path 省略はいまの場所へ)
        "save" => {
            calc.commit();
            match o.str("path").map(std::path::PathBuf::from).or(calc.path.clone()) {
                Some(p) => {
                    calc.save_to(p);
                    "{\"ok\":true}".into()
                }
                None => err("保存先がありません(path を渡してください)"),
            }
        }
        // 範囲の値を読む(計算済みの値。数は数・文字は文字・空は null)
        "get" => {
            let (si, a, b) = match target(calc, &o) {
                Ok(t) => t,
                Err(e) => return e,
            };
            let sh = &calc.book.sheets[si];
            let mut rows: Vec<J> = Vec::new();
            for r in a.row..=b.row {
                let mut cols: Vec<J> = Vec::new();
                for c in a.col..=b.col {
                    cols.push(match sh.value(Pos::new(r, c)) {
                        sheet::Value::Number(n) => J::N(n),
                        sheet::Value::Bool(x) => J::B(x),
                        sheet::Value::Empty => J::Null,
                        v => J::S(v.display()),
                    });
                }
                rows.push(J::A(cols));
            }
            format!("{{\"ok\":true,\"values\":{}}}", J::A(rows).to_json())
        }
        // 式を読む(無ければ null)
        "get_formula" => {
            let (si, a, b) = match target(calc, &o) {
                Ok(t) => t,
                Err(e) => return e,
            };
            let sh = &calc.book.sheets[si];
            let mut rows: Vec<J> = Vec::new();
            for r in a.row..=b.row {
                let mut cols: Vec<J> = Vec::new();
                for c in a.col..=b.col {
                    cols.push(
                        match sh.get(Pos::new(r, c)).and_then(|x| x.formula.clone()) {
                            Some(f) => J::S(format!("={f}")),
                            None => J::Null,
                        },
                    );
                }
                rows.push(J::A(cols));
            }
            format!("{{\"ok\":true,\"formulas\":{}}}", J::A(rows).to_json())
        }
        // 範囲へ書く(origin の a1 から values の形ぶん)。
        // 文字列は Cell::input と同じ扱い(=から始まれば式)。1回=1手(Ctrl+Z)
        "set" => {
            let Some(a1) = o.str("a1") else { return err("a1 がありません") };
            let Some(origin) = Pos::parse(a1.split(':').next().unwrap_or(&a1)) else {
                return err("a1 が読めません");
            };
            let si = match sheet_index(calc, &o) {
                Ok(i) => i,
                Err(e) => return e,
            };
            let Some(grid) = o.grid("values") else { return err("values がありません") };
            if calc.book.sheets[si].protected {
                return err("シートが保護されています");
            }
            calc.commit();
            calc.checkpoint();
            let mut n = 0usize;
            for (dr, row) in grid.iter().enumerate() {
                for (dc, v) in row.iter().enumerate() {
                    let p = Pos::new(origin.row + dr as u32, origin.col + dc as u32);
                    let sh = &mut calc.book.sheets[si];
                    let fmt = sh.get(p).map(|c| c.fmt.clone()).unwrap_or_default();
                    let mut cell = match v {
                        J::Null => sheet::Cell::input(""),
                        J::N(x) => sheet::Cell::input(&J::N(*x).to_json()),
                        J::B(x) => {
                            sheet::Cell::input(if *x { "TRUE" } else { "FALSE" })
                        }
                        J::S(s) => sheet::Cell::input(s),
                        J::A(_) => return err("values の入れ子が深すぎます"),
                    };
                    cell.fmt = fmt; // 書式は据え置く(打ち直しと同じ作法)
                    sh.set(p, cell);
                    n += 1;
                }
            }
            recalc_book(&mut calc.book, si);
            calc.dirty = true;
            calc.sync_input();
            calc.status = ui::tf!("Python から {} セルを書き込みました", n).into();
            cx.notify();
            format!("{{\"ok\":true,\"cells\":{n}}}")
        }
        // 左上から地続きの表の大きさ(xlwings の expand='table')
        "expand" => {
            let Some(a1) = o.str("a1") else { return err("a1 がありません") };
            let Some(p) = Pos::parse(&a1) else { return err("a1 が読めません") };
            let si = match sheet_index(calc, &o) {
                Ok(i) => i,
                Err(e) => return e,
            };
            let sh = &calc.book.sheets[si];
            let filled =
                |r: u32, c: u32| !sh.value(Pos::new(r, c)).is_empty();
            let mut h = 0u32;
            while filled(p.row + h, p.col) {
                h += 1;
            }
            let mut w = 0u32;
            while filled(p.row, p.col + w) {
                w += 1;
            }
            format!("{{\"ok\":true,\"rows\":{},\"cols\":{}}}", h.max(1), w.max(1))
        }
        _ => err(&format!("知らない cmd: {cmd}")),
    }
}

/// 要求のシート(名前。省略はいまのシート)を添字に。
fn sheet_index(calc: &Calc, o: &Jobj) -> Result<usize, String> {
    match o.str("sheet") {
        None => Ok(calc.active),
        Some(name) => calc
            .book
            .sheets
            .iter()
            .position(|s| s.name == name)
            .ok_or_else(|| err(&format!("シート「{name}」がありません"))),
    }
}

/// 要求の範囲(a1 は "B2" か "B2:D4")を (シート, 左上, 右下) に。
fn target(calc: &Calc, o: &Jobj) -> Result<(usize, Pos, Pos), String> {
    let si = sheet_index(calc, o)?;
    let a1 = o.str("a1").ok_or_else(|| err("a1 がありません"))?;
    let mut it = a1.split(':');
    let a = it
        .next()
        .and_then(Pos::parse)
        .ok_or_else(|| err("a1 が読めません"))?;
    let b = match it.next() {
        Some(t) => Pos::parse(t).ok_or_else(|| err("a1 が読めません"))?,
        None => a,
    };
    Ok((si, a, b))
}

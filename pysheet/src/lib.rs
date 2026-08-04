//! office_sheet — `sheet` crate の Python 束縛。
//!
//! 分業の設計(SEKKEI.md): **データを作る・分析する仕事は Python、
//! 見ながら整える仕事は calc。** その橋がこれ。
//! polars で集計した結果を、**帳票の枠(罫線・結合・列幅・図形)を
//! 保ったまま**実物の様式 xlsx に差し込んで保存できる。
//! openpyxl との違いはそこ — 開いて保存しただけで様式が崩れない。
//!
//! マクロの置き換えでもある: 表の中に実行コードを埋める(VBA)代わりに、
//! 表の外のスクリプトが表を扱う。「開く=実行」という攻撃経路を持たない。
//!
//!     import office_sheet
//!     b = office_sheet.Book.open("様式7.xlsx")
//!     s = b["提案見積書"]
//!     s["A30"] = "日本フネン株式会社"
//!     s["C30"] = "=B30*100"        # 文字列は「打ったのと同じ」解釈
//!     b.save("out.xlsx")           # 罫線・結合・列幅・図形は元のまま

use std::sync::{Arc, Mutex, MutexGuard};

use pyo3::exceptions::{PyIOError, PyIndexError, PyKeyError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDate, PyDateTime, PyTime};

use sheet::calc::date_serial;
use sheet::model::format_value;
use sheet::{recalc_all, recalc_book, xlsx, Cell, Pos, Value};

/// ブックの中身。Book と Sheet が同じものを見るために1枚挟む。
struct Inner {
    book: sheet::Book,
    /// 開いた元のファイル。保存時に、こちらが作り直さない部品
    /// (図形・テーマ・印刷設定)を持ち越すために取っておく
    original: Option<Vec<u8>>,
    /// 読めなかったものの帳簿。黙って落とさない(ooxml と同じ作法)
    unsupported: Vec<(String, usize)>,
}

fn lock(inner: &Arc<Mutex<Inner>>) -> PyResult<MutexGuard<'_, Inner>> {
    inner
        .lock()
        .map_err(|_| PyValueError::new_err("別の操作が失敗した後で、ブックの状態が信用できない"))
}

fn parse_ref(s: &str) -> PyResult<Pos> {
    Pos::parse(s).ok_or_else(|| PyValueError::new_err(format!("セル参照として読めない: {s:?}")))
}

/// Python へ返す値。Empty は None(Option 側)で返す。
/// エラー値(#DIV/0! など)は文字列で返す — 表計算の作法どおり。
#[derive(IntoPyObject)]
enum Out {
    Num(f64),
    Text(String),
    Bool(bool),
}

fn to_out(v: &Value) -> Option<Out> {
    match v {
        Value::Empty => None,
        Value::Number(n) => Some(Out::Num(*n)),
        Value::Text(s) => Some(Out::Text(s.clone())),
        Value::Bool(b) => Some(Out::Bool(*b)),
        Value::Error(e) => Some(Out::Text(e.clone())),
    }
}

/// 数の入ったセル(書式は呼び側で付け直す)。
fn num_cell(n: f64) -> Cell {
    Cell { formula: None, value: Value::Number(n), fmt: Default::default() }
}

/// date/datetime の年月日 → Excel の通し番号(日の部分)。
/// abi3 では C の accessor が使えないので、属性(year/month/day)で読む
fn date_days(v: &Bound<'_, PyAny>) -> PyResult<i64> {
    let g = |n: &str| -> PyResult<i64> { v.getattr(n)?.extract() };
    Ok(date_serial(g("year")?, g("month")?, g("day")?))
}

/// time/datetime の時刻 → 日の割合(0.0〜)。
fn time_frac(v: &Bound<'_, PyAny>) -> PyResult<f64> {
    let g = |n: &str| -> PyResult<i64> { v.getattr(n)?.extract() };
    Ok((g("hour")? as f64 * 3600.0
        + g("minute")? as f64 * 60.0
        + g("second")? as f64
        + g("microsecond")? as f64 / 1e6)
        / 86400.0)
}

/// xlsx のブック。
#[pyclass(name = "Book")]
struct PyBook {
    inner: Arc<Mutex<Inner>>,
}

#[pymethods]
impl PyBook {
    /// 空のブック(Sheet1 が1枚)。
    #[new]
    fn new() -> PyBook {
        PyBook {
            inner: Arc::new(Mutex::new(Inner {
                book: sheet::Book::new(),
                original: None,
                unsupported: Vec::new(),
            })),
        }
    }

    /// xlsx を開く。式は開いた時点で再計算される。
    #[staticmethod]
    fn open(path: &str) -> PyResult<PyBook> {
        let bytes = std::fs::read(path)
            .map_err(|e| PyIOError::new_err(format!("{path}: 読めない: {e}")))?;
        let (mut book, rep) = xlsx::read(std::io::Cursor::new(&bytes))
            .map_err(|e| PyIOError::new_err(format!("{path}: xlsx として読めない: {e}")))?;
        recalc_all(&mut book);
        Ok(PyBook {
            inner: Arc::new(Mutex::new(Inner {
                book,
                original: Some(bytes),
                unsupported: rep.unsupported,
            })),
        })
    }

    /// 保存する。開いた元のファイルがあれば、こちらが作り直さない部品
    /// (図形・テーマ・印刷設定・文書情報)を原本から持ち越す。
    fn save(&self, path: &str) -> PyResult<()> {
        let mut g = lock(&self.inner)?;
        recalc_all(&mut g.book);
        let mut buf = std::io::Cursor::new(Vec::new());
        let r = match &g.original {
            Some(bytes) => xlsx::write_with(&g.book, Some(std::io::Cursor::new(bytes)), &mut buf),
            None => xlsx::write(&g.book, &mut buf),
        };
        r.map_err(|e| PyIOError::new_err(format!("{path}: 書けない: {e}")))?;
        std::fs::write(path, buf.into_inner())
            .map_err(|e| PyIOError::new_err(format!("{path}: 書けない: {e}")))
    }

    /// シートの名前の一覧。
    #[getter]
    fn sheet_names(&self) -> PyResult<Vec<String>> {
        Ok(lock(&self.inner)?.book.sheets.iter().map(|s| s.name.clone()).collect())
    }

    /// 読めなかったものの帳簿 [(名前, 件数)]。空なら取りこぼしなし。
    /// **黙って落とさない** — 開いた様式に読めないものがあれば、ここに出る。
    #[getter]
    fn unsupported(&self) -> PyResult<Vec<(String, usize)>> {
        Ok(lock(&self.inner)?.unsupported.clone())
    }

    /// 名前か番号(0起点)でシートを取る。`book["提案見積書"]` / `book[0]`。
    fn __getitem__(&self, key: &Bound<'_, PyAny>) -> PyResult<PySheet> {
        let g = lock(&self.inner)?;
        let idx = if let Ok(i) = key.extract::<usize>() {
            if i >= g.book.sheets.len() {
                return Err(PyIndexError::new_err(format!(
                    "シートは {} 枚しかない: {i}",
                    g.book.sheets.len()
                )));
            }
            i
        } else {
            let name = key.extract::<String>()?;
            g.book
                .sheets
                .iter()
                .position(|s| s.name == name)
                .ok_or_else(|| PyKeyError::new_err(format!("シートが無い: {name:?}")))?
        };
        Ok(PySheet { inner: Arc::clone(&self.inner), idx })
    }

    fn __len__(&self) -> PyResult<usize> {
        Ok(lock(&self.inner)?.book.sheets.len())
    }

    /// シートを1枚足す。同じ名前があればエラー。
    fn add_sheet(&self, name: &str) -> PyResult<PySheet> {
        let mut g = lock(&self.inner)?;
        if g.book.sheets.iter().any(|s| s.name == name) {
            return Err(PyValueError::new_err(format!("同じ名前のシートがある: {name:?}")));
        }
        g.book.sheets.push(sheet::Sheet::new(name));
        let idx = g.book.sheets.len() - 1;
        Ok(PySheet { inner: Arc::clone(&self.inner), idx })
    }

    /// 全シートを再計算する(セルを置いた時点でそのシートは再計算済み。
    /// 明示的にやり直したいとき用)。
    fn recalc(&self) -> PyResult<()> {
        let mut g = lock(&self.inner)?;
        recalc_all(&mut g.book);
        Ok(())
    }
}

/// 1枚のシート。セルは A1 形式で読み書きする。
#[pyclass(name = "Sheet")]
struct PySheet {
    inner: Arc<Mutex<Inner>>,
    idx: usize,
}

impl PySheet {
    fn with<T>(&self, f: impl FnOnce(&mut sheet::Sheet) -> PyResult<T>) -> PyResult<T> {
        let mut g = lock(&self.inner)?;
        let s = g
            .idx_sheet(self.idx)
            .ok_or_else(|| PyKeyError::new_err("このシートはもうブックに無い"))?;
        f(s)
    }

    /// 書き換えてから、**ブック全体の文脈で**このシートを再計算する
    /// (INDIRECT("別の表!A1") も解ける)
    fn with_calc<T>(&self, f: impl FnOnce(&mut sheet::Sheet) -> PyResult<T>) -> PyResult<T> {
        let mut g = lock(&self.inner)?;
        let s = g
            .idx_sheet(self.idx)
            .ok_or_else(|| PyKeyError::new_err("このシートはもうブックに無い"))?;
        let r = f(s)?;
        recalc_book(&mut g.book, self.idx);
        Ok(r)
    }
}

impl Inner {
    fn idx_sheet(&mut self, idx: usize) -> Option<&mut sheet::Sheet> {
        self.book.sheets.get_mut(idx)
    }
}

#[pymethods]
impl PySheet {
    #[getter]
    fn name(&self) -> PyResult<String> {
        self.with(|s| Ok(s.name.clone()))
    }

    /// 計算後の値。空なら None、エラー(#DIV/0! 等)は文字列。
    fn __getitem__(&self, key: &str) -> PyResult<Option<Out>> {
        let p = parse_ref(key)?;
        self.with(|s| Ok(to_out(&s.value(p))))
    }

    /// セルに置く。**書式(罫線・結合・表示形式)は据え置き** — それが存在理由。
    ///
    /// - 数・bool はそのまま値になる
    /// - `datetime.date` / `datetime.datetime` / `datetime.time` は Excel の
    ///   通し番号(1899-12-30 起点。DATE 関数と同じ一本道)になる。
    ///   セルに表示形式が無いときだけ日付の形式を付ける(数字の羅列で見せない)。
    ///   帳票の日付セルには元の表示形式が付いているので、それに従う
    /// - 文字列は「calc で打ったのと同じ」解釈: `"=SUM(A1:A3)"` は式、
    ///   `"123"` は数、それ以外は文字
    /// - None は中身を消す(罫線だけのセルは枠として残る)
    ///
    /// 置いたらこのシートは再計算される。
    fn __setitem__(&self, key: &str, value: Option<Bound<'_, PyAny>>) -> PyResult<()> {
        let p = parse_ref(key)?;
        // (セルの中身, セルに表示形式が無いときに付ける形式)
        let (cell, date_fmt): (Cell, Option<&str>) = match &value {
            None => (Cell::default(), None),
            // datetime は date の子なので、必ず datetime を先に見る
            Some(v) => {
                if v.cast::<PyDateTime>().is_ok() {
                    (num_cell(date_days(v)? as f64 + time_frac(v)?), Some("yyyy/m/d h:mm"))
                } else if v.cast::<PyDate>().is_ok() {
                    (num_cell(date_days(v)? as f64), Some("yyyy/m/d"))
                } else if v.cast::<PyTime>().is_ok() {
                    (num_cell(time_frac(v)?), Some("h:mm"))
                } else if let Ok(b) = v.extract::<bool>() {
                    // bool を数より先に見る(Python の bool は int の子)
                    (Cell { formula: None, value: Value::Bool(b), fmt: Default::default() }, None)
                } else if let Ok(n) = v.extract::<f64>() {
                    (num_cell(n), None)
                } else if let Ok(t) = v.extract::<String>() {
                    (Cell::input(&t), None)
                } else {
                    return Err(PyTypeError::new_err(format!(
                        "セルに置けるのは 数・bool・文字列・datetime/date/time・None。渡されたのは {}",
                        v.get_type().name().map(|n| n.to_string()).unwrap_or_default()
                    )));
                }
            }
        };
        self.with_calc(|s| {
            let mut fmt = s.get(p).map(|c| c.fmt.clone()).unwrap_or_default();
            if fmt.number_format.is_none() {
                if let Some(df) = date_fmt {
                    fmt.number_format = Some(df.into());
                }
            }
            let mut cell = cell;
            cell.fmt = fmt;
            s.set(p, cell);
            Ok(())
        })
    }

    /// セルの式("=SUM(A1:A3)" の形)。式が無ければ None。
    fn formula(&self, key: &str) -> PyResult<Option<String>> {
        let p = parse_ref(key)?;
        self.with(|s| Ok(s.get(p).and_then(|c| c.formula.as_ref().map(|f| format!("={f}")))))
    }

    /// 表示形式(#,##0 など)を当てた、画面に出るのと同じ文字列。
    fn display(&self, key: &str) -> PyResult<String> {
        let p = parse_ref(key)?;
        self.with(|s| {
            Ok(match s.get(p) {
                Some(c) => format_value(&c.value, c.fmt.number_format.as_deref()),
                None => String::new(),
            })
        })
    }

    /// 使われている範囲 (行数, 列数)。DataFrame の shape と同じ向き。
    #[getter]
    fn shape(&self) -> PyResult<(u32, u32)> {
        self.with(|s| Ok(s.extent()))
    }

    /// 使われている範囲の値を list[list](行ごと)で。
    /// `polars.DataFrame(s.values(), orient="row")` でそのまま表になる。
    fn values(&self) -> PyResult<Vec<Vec<Option<Out>>>> {
        self.with(|s| {
            let (rows, cols) = s.extent();
            Ok((0..rows)
                .map(|r| {
                    (0..cols).map(|c| to_out(&s.value(Pos { row: r, col: c }))).collect()
                })
                .collect())
        })
    }

    /// セル結合の一覧 [("A1", "B2"), …](左上, 右下)。帳票の枠組みが見える。
    #[getter]
    fn merges(&self) -> PyResult<Vec<(String, String)>> {
        self.with(|s| Ok(s.merges.iter().map(|(a, b)| (a.a1(), b.a1())).collect()))
    }

    /// 行を挿す。`at` は画面で見える行番号(1起点)。その行の位置に空行が入り、
    /// 下の行と**残った式の参照**が下がる(明細の行を増やす操作)。
    fn insert_row(&self, at: u32) -> PyResult<()> {
        self.with_calc(|s| {
            s.insert_row(row0(at)?);
            Ok(())
        })
    }

    /// 行を抜く(1起点)。抜いた行を指していた式は #REF! になる — 黙って
    /// 別のセルを指すより良い。
    fn remove_row(&self, at: u32) -> PyResult<()> {
        self.with_calc(|s| {
            s.remove_row(row0(at)?);
            Ok(())
        })
    }

    /// 列を挿す。`at` は列の文字("C" なら C 列の位置に空列が入る)。
    fn insert_col(&self, at: &str) -> PyResult<()> {
        let c = col0(at)?;
        self.with_calc(|s| {
            s.insert_col(c);
            Ok(())
        })
    }

    /// 列を抜く(列の文字で指す)。
    fn remove_col(&self, at: &str) -> PyResult<()> {
        let c = col0(at)?;
        self.with_calc(|s| {
            s.remove_col(c);
            Ok(())
        })
    }
}

/// 画面の行番号(1起点)→ 内部の行(0起点)。0行目は無い。
fn row0(at: u32) -> PyResult<u32> {
    at.checked_sub(1).ok_or_else(|| PyValueError::new_err("行番号は1から(0行は無い)"))
}

/// 列の文字("A"〜)→ 内部の列(0起点)。
fn col0(s: &str) -> PyResult<u32> {
    let p = Pos::parse(&format!("{}1", s.trim()))
        .ok_or_else(|| PyValueError::new_err(format!("列の文字として読めない: {s:?}")))?;
    Ok(p.col)
}

#[pymodule]
fn office_sheet(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyBook>()?;
    m.add_class::<PySheet>()?;
    Ok(())
}

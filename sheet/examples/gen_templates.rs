//! 台帳テンプレ集を作る(N個のブック路線の実証)。
//!
//!   cargo run -p sheet --example gen_templates
//!
//! 帳票+コード(檻つき)+運用を1つの xlsx に詰めて templates/ に吐く。
//! テンプレートは生成物 — 直すのはこのファイル。

use sheet::model::{Borders, Validation};
use sheet::{recalc, Book, Cell, Pos};

fn header(s: &mut sheet::Sheet, cols: &[(&str, f32)]) {
    for (i, (name, w)) in cols.iter().enumerate() {
        let mut c = Cell::input(name);
        c.fmt.bold = true;
        c.fmt.fill = Some("D5E8DC".into());
        c.fmt.borders = Borders::ALL;
        s.set(Pos::new(0, i as u32), c);
        s.col_width.insert(i as u32, *w);
    }
}

fn save(book: &Book, path: &str) {
    let f = std::fs::File::create(path).expect("開けない");
    sheet::xlsx::write(book, std::io::BufWriter::new(f)).expect("書けない");
    println!("書いた: {path}");
}

fn inquiries() -> Book {
    let mut b = Book::new();
    let s = &mut b.sheets[0];
    s.name = "台帳".into();
    header(s, &[
        ("受信", 18.0), ("名前", 14.0), ("連絡先", 22.0),
        ("用件", 44.0), ("状態", 10.0), ("対応メモ", 30.0),
    ]);
    // 状態は三択(入力規則)
    s.validations.push(Validation {
        range: (Pos::new(1, 4), Pos::new(500, 4)),
        formula: "\"未対応,対応中,完了\"".into(),
    });
    // 見本の1行
    for (c, v) in ["2026-08-04 09:12", "山田花子", "hanako@example.jp",
                   "見積書の再発行をお願いします", "未対応", ""].iter().enumerate() {
        s.set(Pos::new(1, c as u32), Cell::input(v));
    }
    // 集計の置き場(=PY は @計算 のときだけ動く)
    s.set(Pos::new(0, 7), Cell::input("状態の集計"));
    s.set(Pos::new(1, 7), Cell::input("=PY(\"状態集計\", E2:E501)"));
    s.print_title_rows = Some((0, 0));
    s.print_gridlines = true;
    recalc(s);
    b.scripts.push((
        "関数".into(),
        r#"def 状態集計(r):
    from collections import Counter
    c = Counter(v for row in r for v in row if v)
    return [[k, n] for k, n in sorted(c.items())] or [["(まだ無い)", 0]]
"#.into(),
    ));
    b.scripts.push((
        "取り込み".into(),
        r#"# フォームの受信箱(CSV を返す URL)から新着を台帳へ追記する。
# 実行は「@取り込み net」(網あり檻 — 許可はその場の操作だけ)。
# URL を自分のフォームのものに書き換えて使う。
URL = "http://127.0.0.1:8000/inbox.csv"

import urllib.request, csv, io
raw = urllib.request.urlopen(URL, timeout=5).read()
rows = list(csv.DictReader(io.StringIO(raw.decode("utf-8"))))
base = len([r for r in s.values() if any(v is not None and v != "" for v in r)])
for i, r in enumerate(rows):
    n = base + 1 + i
    s[f"A{n}"] = r.get("received", "")
    s[f"B{n}"] = r.get("name", "")
    s[f"C{n}"] = r.get("email", "")
    s[f"D{n}"] = r.get("body", "")
    s[f"E{n}"] = "未対応"
print(f"{len(rows)} 件を取り込みました")
"#.into(),
    ));
    b
}

fn inventory() -> Book {
    let mut b = Book::new();
    let s = &mut b.sheets[0];
    s.name = "在庫".into();
    header(s, &[
        ("品名", 20.0), ("型番", 14.0), ("在庫数", 9.0),
        ("発注点", 9.0), ("単価", 12.0), ("在庫金額", 14.0),
    ]);
    for (r, (name, code, qty, min, price)) in [
        ("ザボガードF", "F-02", "12", "10", "125000"),
        ("エンブM", "M-11", "3", "5", "98000"),
        ("防炎シート", "S-70", "40", "20", "5200"),
    ].iter().enumerate() {
        let r = r as u32 + 1;
        for (c, v) in [*name, *code, *qty, *min, *price].iter().enumerate() {
            s.set(Pos::new(r, c as u32), Cell::input(v));
        }
        s.set(Pos::new(r, 5), Cell::input(&format!("=C{0}*E{0}", r + 1)));
    }
    s.set(Pos::new(0, 7), Cell::input("要発注(在庫数<発注点)"));
    s.set(Pos::new(1, 7), Cell::input("=PY(\"要発注\", A2:D100)"));
    s.print_gridlines = true;
    recalc(s);
    b.scripts.push((
        "関数".into(),
        r#"def 要発注(r):
    out = [[row[0], row[2], row[3]] for row in r
           if row[0] and row[2] is not None and row[3] is not None and row[2] < row[3]]
    return out or [["(無し)", "", ""]]
"#.into(),
    ));
    b
}

fn timesheet() -> Book {
    let mut b = Book::new();
    let s = &mut b.sheets[0];
    s.name = "勤怠".into();
    header(s, &[
        ("日", 6.0), ("曜日", 7.0), ("出勤", 9.0), ("退勤", 9.0), ("実働h", 9.0),
    ]);
    for d in 1..=31u32 {
        let r = d;
        s.set(Pos::new(r, 0), Cell::input(&d.to_string()));
        // 曜日は日付関数で(1=日 … 7=土)
        s.set(Pos::new(r, 1), Cell::input(&format!("=WEEKDAY(DATE(2026,8,{d}))")));
        s.set(Pos::new(r, 4), Cell::input(&format!("=PY(\"実働\", C{0}, D{0})", r + 1)));
    }
    s.set(Pos::new(32, 3), Cell::input("合計"));
    s.set(Pos::new(32, 4), Cell::input("=SUM(E2:E32)"));
    s.set(Pos::new(1, 2), Cell::input("9:00"));
    s.set(Pos::new(1, 3), Cell::input("18:00"));
    s.print_title_rows = Some((0, 0));
    recalc(s);
    b.scripts.push((
        "関数".into(),
        r#"def 実働(a, b):
    # "9:00"〜"18:00" → 休憩1時間を引いた時間数。空なら空
    if not a or not b:
        return ""
    def m(t):
        h, mm = str(t).split(":")
        return int(h) * 60 + int(mm)
    return round((m(b) - m(a) - 60) / 60, 2)
"#.into(),
    ));
    b
}

fn main() {
    std::fs::create_dir_all("templates").expect("templates/");
    save(&inquiries(), "templates/問い合わせ台帳.xlsx");
    save(&inventory(), "templates/在庫台帳.xlsx");
    save(&timesheet(), "templates/勤怠表.xlsx");
    // 読み戻して壊れていないことを確かめる(黙って配らない)
    for p in ["templates/問い合わせ台帳.xlsx", "templates/在庫台帳.xlsx", "templates/勤怠表.xlsx"] {
        let f = std::fs::File::open(p).expect("開けない");
        let (back, rep) = sheet::xlsx::read(f).expect("読めない");
        assert!(!back.scripts.is_empty(), "{p}: 同梱コードが消えた");
        assert!(rep.unsupported.is_empty(), "{p}: 読めないもの {:?}", rep.unsupported);
        println!("検め OK: {p}({} スクリプト同梱)", back.scripts.len());
    }
}

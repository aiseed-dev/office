//! 公開できるサンプル帳票を作る。
//!
//!   cargo run -p sheet --example gen_samples
//!
//! templates/ が「ブック=業務アプリ」の見本なのに対し、sample/ は
//! 「開いて・直して・刷る」を試すための普通の帳票。中身はすべて架空。
//! サンプルは生成物 — 直すのはこのファイル。

use sheet::model::{Borders, HAlign, VAlign};
use sheet::{recalc, Book, Cell, Pos};

fn save(book: &Book, path: &str) {
    let f = std::fs::File::create(path).expect("開けない");
    sheet::xlsx::write(book, std::io::BufWriter::new(f)).expect("書けない");
    println!("書いた: {path}");
}

/// 見積書 — 結合・罫線・表示形式・式(SUM/ROUND)・印刷範囲の見本。
fn mitsumori() -> Book {
    let mut b = Book::new();
    let s = &mut b.sheets[0];
    s.name = "見積書".into();
    for (i, w) in [(0, 6.0), (1, 34.0), (2, 10.0), (3, 8.0), (4, 12.0), (5, 14.0)] {
        s.col_width.insert(i, w);
    }

    // 表題(A1:F1 結合・中央)
    let mut t = Cell::input("御 見 積 書");
    t.fmt.bold = true;
    t.fmt.size_c = Some(1800);
    t.fmt.align = HAlign::Center;
    t.fmt.valign = VAlign::Middle;
    s.set(Pos::new(0, 0), t);
    s.merges.push((Pos::new(0, 0), Pos::new(0, 5)));
    s.row_height.insert(0, 32.0);

    // 宛先と番号・日付(中身は架空)
    let mut atesaki = Cell::input("株式会社みほん商事 御中");
    atesaki.fmt.bold = true;
    atesaki.fmt.size_c = Some(1200);
    s.set(Pos::new(2, 0), atesaki);
    s.set(Pos::new(2, 4), Cell::input("見積番号"));
    s.set(Pos::new(2, 5), Cell::input("M-2026-001"));
    s.set(Pos::new(3, 4), Cell::input("発行日"));
    s.set(Pos::new(3, 5), Cell::input("2026-08-04"));
    s.set(Pos::new(4, 0), Cell::input("下記のとおりお見積り申し上げます。"));

    // 税込合計(明細から式で引く)
    let mut label = Cell::input("御見積金額(税込)");
    label.fmt.bold = true;
    s.set(Pos::new(5, 0), label);
    let mut total = Cell::input("=F18");
    total.fmt.bold = true;
    total.fmt.size_c = Some(1400);
    total.fmt.number_format = Some("¥#,##0".into());
    s.set(Pos::new(5, 2), total);

    // 発行元(架空)
    s.set(Pos::new(5, 4), Cell::input("例示工務店"));
    s.set(Pos::new(6, 4), Cell::input("見本県架空市例示町1-2-3"));
    s.set(Pos::new(7, 4), Cell::input("012-345-6789"));

    // 明細(10行目が見出し、11〜15行目が明細 — A1 で言えば)
    for (i, name) in ["No.", "品名", "数量", "単位", "単価", "金額"].iter().enumerate() {
        let mut c = Cell::input(name);
        c.fmt.bold = true;
        c.fmt.fill = Some("DCE6F1".into());
        c.fmt.borders = Borders::ALL;
        c.fmt.align = HAlign::Center;
        s.set(Pos::new(9, i as u32), c);
    }
    let rows: &[(&str, &str, &str, &str)] = &[
        ("外壁塗装工事", "1", "式", "450000"),
        ("足場の設置", "120", "㎡", "800"),
        ("高圧洗浄", "120", "㎡", "300"),
    ];
    for r in 0..5u32 {
        let row = 10 + r; // 0-based。A1 で言えば 11〜15 行目
        for col in 0..6u32 {
            let mut c = match (col, rows.get(r as usize)) {
                (0, Some(_)) => Cell::input(&format!("{}", r + 1)),
                (1, Some(d)) => Cell::input(d.0),
                (2, Some(d)) => Cell::input(d.1),
                (3, Some(d)) => Cell::input(d.2),
                (4, Some(d)) => Cell::input(d.3),
                (5, Some(_)) => Cell::input(&format!("=C{n}*E{n}", n = row + 1)),
                _ => Cell::input(""),
            };
            c.fmt.borders = Borders::ALL;
            if col >= 4 {
                c.fmt.number_format = Some("#,##0".into());
            }
            s.set(Pos::new(row, col), c);
        }
    }
    // 小計・消費税・合計(式)
    for (row, label, formula) in [
        (15u32, "小計", "=SUM(F11:F15)"),
        (16, "消費税(10%)", "=ROUND(F16*0.1,0)"),
        (17, "合計", "=F16+F17"),
    ] {
        let mut l = Cell::input(label);
        l.fmt.borders = Borders::ALL;
        l.fmt.align = HAlign::Center;
        s.set(Pos::new(row, 4), l);
        let mut f = Cell::input(formula);
        f.fmt.borders = Borders::ALL;
        f.fmt.number_format = Some("¥#,##0".into());
        if row == 17 {
            f.fmt.bold = true;
        }
        s.set(Pos::new(row, 5), f);
    }

    // 印刷は A4 縦・この範囲だけ
    s.paper_size = Some(9); // A4
    s.landscape = false;
    s.print_areas.push((Pos::new(0, 0), Pos::new(18, 5)));
    recalc(s);
    b
}

fn main() {
    std::fs::create_dir_all("sample").expect("sample/ が作れない");
    save(&mitsumori(), "sample/見積書.xlsx");
}

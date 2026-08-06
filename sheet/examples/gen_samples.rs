//! 公開できるサンプル帳票を作る。
//!
//!   cargo run -p sheet --example gen_samples
//!
//! templates/ が「ブック=業務アプリ」の見本なのに対し、sample/ は
//! 「開いて・直して・刷る」を試すための普通の帳票。中身はすべて架空。
//! サンプルは生成物 — 直すのはこのファイル。

use sheet::model::{Borders, CondOp, CondRule, HAlign, VAlign, Validation};
use sheet::{recalc, Book, Cell, Pos};

/// 品番マスタ(品番・品名・単価)。カタログ(gen_catalog.py の同梱データ)と
/// 同じ架空の36品目。サーバー(catalog_server.py)が正本で、
/// 注文書の @更新 net が写しをこれと入れ替える。
const MASTER: &[(&str, &str, u32)] = &[
    ("A-101", "ボールペン(黒)", 150), ("A-102", "ボールペン(赤)", 150),
    ("A-103", "ボールペン(青)", 150), ("A-104", "シャープペン", 220),
    ("A-105", "シャープ替芯", 120), ("A-106", "蛍光マーカー(黄)", 130),
    ("A-107", "蛍光マーカー(桃)", 130), ("A-108", "油性ペン(黒)", 160),
    ("A-109", "鉛筆HB", 480), ("A-110", "消しゴム", 90),
    ("B-201", "コピー用紙A4", 550), ("B-202", "コピー用紙B5", 520),
    ("B-203", "ノートA罫", 180), ("B-204", "レポート用紙A4", 250),
    ("B-205", "付箋 75×75mm", 210), ("B-206", "付箋 75×25mm", 260),
    ("B-207", "封筒 長形3号", 680), ("B-208", "クラフト封筒 角形2号", 750),
    ("C-301", "クリアファイルA4", 240), ("C-302", "パイプ式ファイルA4", 780),
    ("C-303", "個別フォルダA4", 620), ("C-304", "2穴バインダーA4", 450),
    ("C-305", "書類トレーA4", 520), ("C-306", "マグネットバー", 330),
    ("D-401", "電卓", 1480), ("D-402", "ホッチキス10号", 620),
    ("D-403", "ホッチキス針10号", 110), ("D-404", "2穴パンチ", 830),
    ("D-405", "テープカッター", 690), ("D-406", "はさみ", 420),
    ("D-407", "カッターL型", 380), ("D-408", "スティックのり", 140),
    ("E-501", "ガムテープ(布)", 280), ("E-502", "OPPテープ(透明)", 190),
    ("E-503", "緩衝材", 640), ("E-504", "宅配袋(大)", 520),
];

fn head(s: &mut sheet::Sheet, cols: &[(&str, f32)]) {
    for (i, (name, w)) in cols.iter().enumerate() {
        let mut c = Cell::input(name);
        c.fmt.bold = true;
        c.fmt.fill = Some("DCE6F1".into());
        c.fmt.borders = Borders::ALL;
        c.fmt.align = HAlign::Center;
        s.set(Pos::new(0, i as u32), c);
        s.col_width.insert(i as u32, *w);
    }
}

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

/// 現金出納帳 — 前行を引き継ぐ残高の式・条件付き書式・コメントの見本。
fn suitou() -> Book {
    let mut b = Book::new();
    let s = &mut b.sheets[0];
    s.name = "出納帳".into();
    head(s, &[
        ("日付", 10.0), ("摘要", 30.0), ("収入", 12.0), ("支出", 12.0), ("残高", 13.0),
    ]);
    let rows: &[(&str, &str, &str, &str)] = &[
        ("7/1", "前月繰越", "50000", ""),
        ("7/2", "事務用品(例示文具)", "", "3240"),
        ("7/5", "売上の入金(みほん商事)", "120000", ""),
        ("7/12", "ガソリン代", "", "6800"),
        ("7/20", "電気代", "", "9500"),
        ("7/25", "売上の入金(架空団地自治会)", "80000", ""),
        ("7/28", "外注費(例示塗装)", "", "225000"),
    ];
    for (i, d) in rows.iter().enumerate() {
        let row = 1 + i as u32; // A1 で言えば 2 行目から
        for (col, v) in [d.0, d.1, d.2, d.3].iter().enumerate() {
            let mut c = Cell::input(v);
            c.fmt.borders = Borders::ALL;
            if col >= 2 {
                c.fmt.number_format = Some("#,##0".into());
            }
            s.set(Pos::new(row, col as u32), c);
        }
        // 残高: 先頭行は繰越そのもの、以降は前行を引き継ぐ
        let f = if i == 0 {
            "=C2-D2".to_string()
        } else {
            format!("=E{}+C{n}-D{n}", row, n = row + 1)
        };
        let mut c = Cell::input(&f);
        c.fmt.borders = Borders::ALL;
        c.fmt.number_format = Some("¥#,##0".into());
        s.set(Pos::new(row, 4), c);
    }
    // 残高が 1 万円を割ったら塗って知らせる(条件付き書式)
    s.cond.push(CondRule {
        range: (Pos::new(1, 4), Pos::new(30, 4)),
        op: CondOp::Lt,
        value: 10000.0,
        color: None,
        fill: Some("FCE4D6".into()),
    });
    s.comments.insert(
        Pos::new(7, 4),
        "外注費の支払いで一時的に薄い。8/5 の入金で戻る見込み".into(),
    );
    s.print_title_rows = Some((0, 0));
    s.print_gridlines = true;
    recalc(s);
    b
}

/// 成績表 — AVERAGE・MAX・MIN・COUNTIF と条件付き書式(80点以上を塗る)の見本。
fn seiseki() -> Book {
    let mut b = Book::new();
    let s = &mut b.sheets[0];
    s.name = "成績表".into();
    head(s, &[
        ("名前", 14.0), ("国語", 9.0), ("数学", 9.0), ("英語", 9.0),
        ("合計", 10.0), ("平均", 10.0),
    ]);
    let rows: &[(&str, &str, &str, &str)] = &[
        ("相川一郎", "82", "65", "91"),
        ("井坂二葉", "74", "88", "70"),
        ("上野三郎", "58", "92", "63"),
        ("江本四月", "95", "71", "84"),
        ("大場五実", "67", "79", "88"),
    ];
    for (i, d) in rows.iter().enumerate() {
        let row = 1 + i as u32;
        let n = row + 1; // A1 の行番号
        for (col, v) in [
            d.0.to_string(), d.1.to_string(), d.2.to_string(), d.3.to_string(),
            format!("=SUM(B{n}:D{n})"), format!("=ROUND(AVERAGE(B{n}:D{n}),1)"),
        ].iter().enumerate() {
            let mut c = Cell::input(v);
            c.fmt.borders = Borders::ALL;
            s.set(Pos::new(row, col as u32), c);
        }
    }
    for (i, (label, tpl)) in [
        ("科目平均", "=ROUND(AVERAGE({c}2:{c}6),1)"),
        ("最高", "=MAX({c}2:{c}6)"),
        ("80点以上", "=COUNTIF({c}2:{c}6,\">=80\")"),
    ].iter().enumerate() {
        let row = 7 + i as u32;
        let mut l = Cell::input(label);
        l.fmt.bold = true;
        l.fmt.borders = Borders::ALL;
        s.set(Pos::new(row, 0), l);
        for col in 1..4u32 {
            let letter = char::from(b'A' + col as u8); // B・C・D
            let mut c = Cell::input(&tpl.replace("{c}", &letter.to_string()));
            c.fmt.borders = Borders::ALL;
            s.set(Pos::new(row, col), c);
        }
    }
    // 80 点以上を塗る(整数の点なので >79 = >=80)
    s.cond.push(CondRule {
        range: (Pos::new(1, 1), Pos::new(5, 3)),
        op: CondOp::Gt,
        value: 79.0,
        color: None,
        fill: Some("E2EFDA".into()),
    });
    recalc(s);
    b
}

/// 注文書 — カタログ(writer の カタログ.docx)の受け側。
/// 品番を打つと品名・単価が写し(H〜J 列)から引かれて金額まで計算される。
/// @更新 net で写しをサーバーの正本と入れ替え、@送信 net で注文を送る。
fn chumon() -> Book {
    let mut b = Book::new();
    let s = &mut b.sheets[0];
    s.name = "注文書".into();
    for (i, w) in [(0, 9.0), (1, 24.0), (2, 12.0), (3, 8.0), (4, 13.0),
                   (5, 3.0), (6, 3.0), (7, 9.0), (8, 22.0), (9, 9.0)] {
        s.col_width.insert(i, w);
    }

    let mut t = Cell::input("注 文 書");
    t.fmt.bold = true;
    t.fmt.size_c = Some(1800);
    t.fmt.align = HAlign::Center;
    t.fmt.valign = VAlign::Middle;
    s.set(Pos::new(0, 0), t);
    s.merges.push((Pos::new(0, 0), Pos::new(0, 4)));
    s.row_height.insert(0, 30.0);

    s.set(Pos::new(1, 0), Cell::input("例示文具株式会社 行(品番はカタログから)"));
    for (row, col, label) in [
        (2u32, 0u32, "社名"), (2, 2, "担当"), (3, 0, "電話"), (3, 2, "納品希望日"),
    ] {
        let mut l = Cell::input(label);
        l.fmt.bold = true;
        l.fmt.borders = Borders::ALL;
        l.fmt.fill = Some("F2F2F2".into());
        s.set(Pos::new(row, col), l);
        let mut e = Cell::input("");
        e.fmt.borders = Borders::ALL;
        s.set(Pos::new(row, col + 1), e);
    }

    // 明細(6行目が見出し、7〜16行目が注文行)
    for (i, name) in ["品番", "品名", "単価(税抜)", "数量", "金額"].iter().enumerate() {
        let mut c = Cell::input(name);
        c.fmt.bold = true;
        c.fmt.fill = Some("DCE6F1".into());
        c.fmt.borders = Borders::ALL;
        c.fmt.align = HAlign::Center;
        s.set(Pos::new(5, i as u32), c);
    }
    let filled: &[(&str, &str)] = &[("A-101", "10"), ("B-201", "5"), ("C-301", "3")];
    for r in 0..10u32 {
        let row = 6 + r;
        let n = row + 1; // A1 の行番号
        for col in 0..5u32 {
            // 式は全行に置き、品番が空の行は空欄に畳む(IF は選ばなかった側の
            // エラーを踏まない — 空行に #N/A は出ない)
            let mut c = match (filled.get(r as usize), col) {
                (Some(d), 0) => Cell::input(d.0),
                (_, 1) => Cell::input(&format!(
                    "=IF(ISBLANK($A{n}),\"\",VLOOKUP($A{n},$H$2:$J$40,2))")),
                (_, 2) => Cell::input(&format!(
                    "=IF(ISBLANK($A{n}),\"\",VLOOKUP($A{n},$H$2:$J$40,3))")),
                (Some(d), 3) => Cell::input(d.1),
                (_, 4) => Cell::input(&format!("=IF(ISBLANK($A{n}),\"\",C{n}*D{n})")),
                _ => Cell::input(""),
            };
            c.fmt.borders = Borders::ALL;
            if col == 2 || col == 4 {
                c.fmt.number_format = Some("#,##0".into());
            }
            s.set(Pos::new(row, col), c);
        }
    }
    for (row, label, formula) in [
        (16u32, "小計", "=SUM(E7:E16)"),
        (17, "消費税(10%)", "=ROUND(E17*0.1,0)"),
        (18, "合計(税込)", "=E17+E18"),
    ] {
        let mut l = Cell::input(label);
        l.fmt.borders = Borders::ALL;
        l.fmt.align = HAlign::Center;
        if row == 18 {
            l.fmt.bold = true;
        }
        s.set(Pos::new(row, 3), l);
        let mut f = Cell::input(formula);
        f.fmt.borders = Borders::ALL;
        f.fmt.number_format = Some("¥#,##0".into());
        if row == 18 {
            f.fmt.bold = true;
        }
        s.set(Pos::new(row, 4), f);
    }
    s.set(Pos::new(20, 0),
        Cell::input("品番と数量を入れるだけ(品名・単価・金額は式が引く)。"));
    s.set(Pos::new(21, 0),
        Cell::input("データ > Python: @更新 net でマスタを取り直し、@送信 net で注文を送る。"));

    // 品番マスタの写し(H〜J 列。印刷範囲の外)
    for (i, name) in ["品番", "品名", "単価"].iter().enumerate() {
        let mut c = Cell::input(name);
        c.fmt.bold = true;
        c.fmt.fill = Some("F2F2F2".into());
        s.set(Pos::new(0, 7 + i as u32), c);
    }
    for (i, (code, name, price)) in MASTER.iter().enumerate() {
        let row = 1 + i as u32;
        s.set(Pos::new(row, 7), Cell::input(code));
        s.set(Pos::new(row, 8), Cell::input(name));
        s.set(Pos::new(row, 9), Cell::input(&price.to_string()));
    }

    // 品番の入力規則: 候補は写しの品番列(範囲参照 — @更新 に追従する)
    s.validations.push(Validation::list(
        (Pos::new(6, 0), Pos::new(15, 0)),
        "$H$2:$H$40".into(),
    ));

    s.paper_size = Some(9); // A4
    s.print_areas.push((Pos::new(0, 0), Pos::new(21, 4)));
    recalc(s);

    b.scripts.push((
        "更新".into(),
        r#"# 品番マスタの写し(H〜J 列)をサーバーの正本と入れ替える。
# 実行は「@更新 net」(網あり檻 — 許可はその場の操作だけ)。
URL = "http://127.0.0.1:8765/catalog.csv"

import urllib.request, csv, io
raw = urllib.request.urlopen(URL, timeout=5).read()
rows = list(csv.reader(io.StringIO(raw.decode("utf-8"))))[1:]
for i, r in enumerate(rows):
    n = 2 + i
    s[f"H{n}"] = r[0]          # 品番
    s[f"I{n}"] = r[2]          # 品名
    s[f"J{n}"] = int(r[4])     # 単価
for n in range(2 + len(rows), 41):   # 減った分の残骸は消す
    s[f"H{n}"] = None; s[f"I{n}"] = None; s[f"J{n}"] = None
b.recalc()
print(f"品番マスタを {len(rows)} 品目に更新しました")
"#.into(),
    ));
    b.scripts.push((
        "送信".into(),
        r#"# 注文行(品番と数量の入った行)をサーバーへ送る。
# 実行は「@送信 net」(網あり檻 — 許可はその場の操作だけ)。
URL = "http://127.0.0.1:8765/order"

import urllib.request, json
lines = []
for n in range(7, 17):
    code, qty = s[f"A{n}"], s[f"D{n}"]
    if code and qty:
        lines.append({"品番": code, "数量": int(qty)})
if not lines:
    print("注文行がありません(品番と数量を入れてから)")
else:
    order = {"社名": s["B3"] or "(未記入)", "担当": s["D3"] or "", "明細": lines}
    req = urllib.request.Request(
        URL, json.dumps(order, ensure_ascii=False).encode("utf-8"),
        {"Content-Type": "application/json"})
    r = json.loads(urllib.request.urlopen(req, timeout=5).read().decode("utf-8"))
    print(f"送信しました(受付番号 {r['受付番号']}・明細 {len(lines)} 行)")
"#.into(),
    ));
    b
}

/// 受注台帳 — e-shop の販売管理側(SEKKEI「e-shop」の段②)。
/// 店(catalog_server)に溜まった注文を @取り込み net で引き取り、
/// 売上はピボット・小計・条件付き書式など calc の既存道具で見る。
fn juchu() -> Book {
    let mut b = Book::new();
    let s = &mut b.sheets[0];
    s.name = "受注台帳".into();
    head(s, &[
        ("受付", 7.0), ("社名", 20.0), ("品番", 9.0), ("品名", 22.0),
        ("数量", 8.0), ("単価", 10.0), ("金額", 12.0), ("発送済", 8.0),
    ]);
    // 集計と控え(J〜K 列。取込済の件数は @取り込み が使う)
    let mut l = Cell::input("売上合計");
    l.fmt.bold = true;
    s.set(Pos::new(0, 9), l);
    let mut f = Cell::input("=SUM(G2:G500)");
    f.fmt.number_format = Some("¥#,##0".into());
    f.fmt.bold = true;
    s.set(Pos::new(1, 9), f);
    let mut l = Cell::input("取込済");
    l.fmt.bold = true;
    s.set(Pos::new(0, 10), l);
    s.set(Pos::new(1, 10), Cell::input("0"));
    s.col_width.insert(9, 12.0);
    // 台帳は記録なので**空で出荷**する(見本の行を持つと、店の実際の
    // 受付番号と衝突して取りこぼす — 実測で踏んだ)。行は @取り込み が刻む
    s.print_title_rows = Some((0, 0));
    recalc(s);

    b.scripts.push((
        "取り込み".into(),
        r#"# 店(catalog_server)に溜まった注文を台帳へ追記する。
# 実行は「@取り込み net」(網あり檻 — 許可はその場の操作だけ)。
# 取込済の件数(K2)を控えているので、新しい注文だけが入る。
URL = "http://127.0.0.1:8765"

import urllib.request, json, csv, io
orders = json.loads(urllib.request.urlopen(URL + "/orders", timeout=5).read().decode("utf-8"))
raw = urllib.request.urlopen(URL + "/catalog.csv", timeout=5).read().decode("utf-8")
master = {r[0]: (r[2], int(r[4])) for r in list(csv.reader(io.StringIO(raw)))[1:]}
done = int(float(s["K2"] or 0))
new = orders[done:]
if not new:
    print(f"新しい注文はありません(累計 {len(orders)} 件)")
else:
    n = 2
    while s[f"A{n}"] not in (None, ""):
        n += 1
    lines = 0
    for i, o in enumerate(new, start=done + 1):
        for line in o.get("明細", []):
            code = str(line.get("品番", ""))
            name, price = master.get(code, ("(不明な品番)", 0))
            s[f"A{n}"] = i
            s[f"B{n}"] = o.get("社名", "")
            s[f"C{n}"] = code
            s[f"D{n}"] = name
            s[f"E{n}"] = int(line.get("数量", 0))
            s[f"F{n}"] = price
            s[f"G{n}"] = f"=E{n}*F{n}"
            s[f"H{n}"] = "FALSE"
            n += 1
            lines += 1
    s["K2"] = len(orders)
    b.recalc()
    print(f"{len(new)} 件({lines} 行)を取り込みました(累計 {len(orders)} 件)")
"#.into(),
    ));
    b
}

/// 売上台帳 — ピボットの試し場。月×区分×品名の36行が詰まっていて、
/// 挿入 > ピボットテーブルを挿入(行=区分、列=月、値=金額)がすぐ試せる。
/// ピボットの上にカーソルを置くと「ピボットテーブル」タブが現れ、
/// 結合・入力規則などの釦が灰色になる(文脈タブとロックの見本)
fn uriage() -> Book {
    let mut b = Book::new();
    let s = &mut b.sheets[0];
    s.name = "売上台帳".into();
    head(s, &[
        ("月", 7.0), ("区分", 12.0), ("品名", 22.0),
        ("数量", 8.0), ("単価", 10.0), ("金額", 12.0),
    ]);
    // 月×区分×品名(数量は決め打ち — 生成物は毎回同じ中身にする)
    const KUBUN: &[(&str, &[(&str, u32)])] = &[
        ("筆記具", &[("ボールペン(黒)", 150), ("シャープペン", 220), ("蛍光マーカー(黄)", 130)]),
        ("紙製品", &[("コピー用紙A4", 550), ("ノートA罫", 180), ("付箋 75×75mm", 210)]),
        ("ファイル", &[("クリアファイルA4", 240), ("パイプ式ファイルA4", 780), ("個別フォルダA4", 620)]),
        ("事務機器", &[("電卓", 1480), ("ホッチキス10号", 620), ("2穴パンチ", 830)]),
    ];
    const QTY: &[u32] = &[
        12, 30, 8, 20, 5, 16, 24, 10, 6, 3, 9, 15,
        18, 25, 12, 22, 8, 20, 30, 7, 10, 4, 11, 13,
        15, 40, 10, 28, 6, 18, 26, 9, 12, 5, 8, 17,
    ];
    let mut r = 1u32;
    for month in ["4月", "5月", "6月"] {
        for (kubun, items) in KUBUN {
            for (name, tanka) in *items {
                s.set(Pos::new(r, 0), Cell::input(month));
                s.set(Pos::new(r, 1), Cell::input(kubun));
                s.set(Pos::new(r, 2), Cell::input(name));
                s.set(Pos::new(r, 3), Cell::input(&QTY[(r - 1) as usize % QTY.len()].to_string()));
                let mut c = Cell::input(&tanka.to_string());
                c.fmt.number_format = Some("¥#,##0".into());
                s.set(Pos::new(r, 4), c);
                let mut c = Cell::input(&format!("=D{}*E{}", r + 1, r + 1));
                c.fmt.number_format = Some("¥#,##0".into());
                s.set(Pos::new(r, 5), c);
                r += 1;
            }
        }
    }
    s.print_title_rows = Some((0, 0));
    recalc(s);
    b
}

fn main() {
    std::fs::create_dir_all("sample").expect("sample/ が作れない");
    save(&mitsumori(), "sample/見積書.xlsx");
    save(&suitou(), "sample/出納帳.xlsx");
    save(&seiseki(), "sample/成績表.xlsx");
    save(&chumon(), "sample/注文書.xlsx");
    save(&juchu(), "sample/受注台帳.xlsx");
    save(&uriage(), "sample/売上台帳.xlsx");
}

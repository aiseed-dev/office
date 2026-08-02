//! calc の編集がxlsxを往復することの検査(UI抜き。GPUIから呼ばれるのと同じ道)。
use sheet::{recalc, xlsx, Book, Cell, Pos};

fn round(book: &Book) -> Book {
    let mut buf = std::io::Cursor::new(Vec::new());
    xlsx::write(book, &mut buf).expect("書けない");
    buf.set_position(0);
    let (mut b, _) = xlsx::read(buf).expect("読めない");
    for s in &mut b.sheets { recalc(s) }
    b
}

#[test]
fn 打った値と式が保存され再計算される() {
    let mut book = Book::new();
    let s = &mut book.sheets[0];
    s.set(Pos::parse("A1").unwrap(), Cell::input("ザボガードF F-02"));
    s.set(Pos::parse("B1").unwrap(), Cell::input("4"));
    s.set(Pos::parse("C1").unwrap(), Cell::input("125000"));
    s.set(Pos::parse("D1").unwrap(), Cell::input("=B1*C1"));
    s.set(Pos::parse("D2").unwrap(), Cell::input("=ROUND(D1*0.1,0)"));
    s.set(Pos::parse("D3").unwrap(), Cell::input("=D1+D2"));
    recalc(s);
    assert_eq!(s.value(Pos::parse("D3").unwrap()).display(), "550000");

    let back = round(&book);
    let b = &back.sheets[0];
    assert_eq!(b.value(Pos::parse("A1").unwrap()).display(), "ザボガードF F-02",
        "日本語が往復しない");
    assert_eq!(b.value(Pos::parse("D3").unwrap()).display(), "550000",
        "式が保存されず再計算できない");
    assert_eq!(b.get(Pos::parse("D3").unwrap()).unwrap().editable(), "=D1+D2",
        "編集欄に式が戻らない");
}

#[test]
fn 実物を開いて直して保存できる() {
    let src = "/mnt/sdb/home/dev/ドキュメント/機構/yoryou-yoshiki/実施要領様式7_提案見積書.xlsx";
    let Ok(bytes) = std::fs::read(src) else { return };
    let (mut book, _) = xlsx::read(std::io::Cursor::new(bytes)).expect("読めない");
    for s in &mut book.sheets { recalc(s) }
    let n = book.sheets[0].cells.len();

    let s = &mut book.sheets[0];
    s.set(Pos::parse("A30").unwrap(), Cell::input("日本フネン株式会社"));
    s.set(Pos::parse("B30").unwrap(), Cell::input("3"));
    s.set(Pos::parse("C30").unwrap(), Cell::input("=B30*100"));
    recalc(s);

    let back = round(&book);
    let b = &back.sheets[0];
    assert!(b.cells.len() >= n + 3, "打った内容が保存されていない");
    assert_eq!(b.value(Pos::parse("A30").unwrap()).display(), "日本フネン株式会社");
    assert_eq!(b.value(Pos::parse("C30").unwrap()).display(), "300", "式が効いていない");
    assert_eq!(b.value(Pos::parse("B1").unwrap()).display(), "（様式７）", "元の内容が壊れた");
}

#[test]
fn 空にするとセルが消える() {
    let mut book = Book::new();
    let s = &mut book.sheets[0];
    s.set(Pos::parse("A1").unwrap(), Cell::input("消す"));
    assert_eq!(s.cells.len(), 1);
    s.set(Pos::parse("A1").unwrap(), Cell::input(""));
    assert_eq!(s.cells.len(), 0, "空にしたセルが残る");
}

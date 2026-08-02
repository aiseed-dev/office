//! 編集の結線が「実際に使える」ことの検査。
//! 打つ → 保存 → 読み直す、を UI 抜きで通す(GPUI から呼ばれるのと同じ道)。

use kumihan::{Document, Editor};
use ui::{handler, HasEditor};

struct Doc {
    doc: Document,
    ed: Editor,
}
impl HasEditor for Doc {
    fn editor(&mut self) -> &mut Editor { &mut self.ed }
    fn editor_ref(&self) -> &Editor { &self.ed }
    fn on_edited(&mut self) {
        // writer と同じ: 編集のたびに本文を文書へ反映する
        self.doc.set_body_text(self.ed.text(), 10.5);
    }
}

fn open(bytes: Vec<u8>) -> Doc {
    let (doc, _) = ooxml::read(std::io::Cursor::new(bytes)).expect("読めない");
    let ed = Editor::new(&doc.body_text());
    Doc { doc, ed }
}
fn save(d: &Doc) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    ooxml::write(&d.doc, &mut buf).expect("書けない");
    buf.into_inner()
}

#[test]
fn 日本語を打って保存し読み直せる() {
    let mut d = Doc { doc: Document::default(), ed: Editor::new("") };
    // IME で「ぼうか」→「防火」を確定
    handler::replace_and_mark(&mut d, None, "ぼうか", None);
    handler::replace_and_mark(&mut d, None, "防火", None);
    handler::replace(&mut d, None, "防火");
    handler::replace(&mut d, None, "ドアの見積をお願いします。");
    assert_eq!(d.ed.text(), "防火ドアの見積をお願いします。");

    let back = open(save(&d));
    assert_eq!(back.doc.body_text(), "防火ドアの見積をお願いします。",
        "打った内容が docx を往復して残らない");
}

#[test]
fn 実物の様式を開いて追記し保存できる() {
    let src = "/mnt/sdb/home/dev/ドキュメント/機構/yoryou-yoshiki/実施要領様式1_参加表明.docx";
    let Ok(bytes) = std::fs::read(src) else { return };
    let mut d = open(bytes);
    assert!(d.doc.body_text().contains("参 加 表 明 書"), "元の本文が読めていない");

    let n = d.ed.text().len();
    d.ed.move_to(n, false);
    handler::replace(&mut d, None, "\n日本フネン株式会社");

    let after = open(save(&d)).doc.body_text();
    assert!(after.contains("参 加 表 明 書"), "元の本文が消えた");
    assert!(after.trim_end().ends_with("日本フネン株式会社"),
        "追記が残っていない: {:?}", after.chars().rev().take(30).collect::<String>());
}

#[test]
fn 編集しても表は失われない() {
    let src = "/mnt/sdb/home/dev/ドキュメント/機構/yoryou-yoshiki/実施要領様式3_会社概要.docx";
    let Ok(bytes) = std::fs::read(src) else { return };
    let mut d = open(bytes);
    let before = d.doc.tables().count();
    assert!(before > 0, "元の文書に表が無い(検査にならない)");

    handler::replace(&mut d, None, "追記");

    let back = open(save(&d));
    assert_eq!(back.doc.tables().count(), before, "本文を編集したら表が消えた");
    assert!(back.doc.body_text().contains("追記"));
}

#[test]
fn undoで打つ前に戻る() {
    let mut d = Doc { doc: Document::default(), ed: Editor::new("元の文") };
    let n = d.ed.text().len();
    d.ed.move_to(n, false);
    handler::replace_and_mark(&mut d, None, "ついき", None);
    handler::replace(&mut d, None, "追記");
    assert_eq!(d.ed.text(), "元の文追記");
    assert!(d.ed.undo());
    d.on_edited();
    assert_eq!(d.doc.body_text(), "元の文", "IMEの確定が1手で戻らない");
}

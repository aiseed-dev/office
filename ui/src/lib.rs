//! 入力の結線 — kumihan の編集モデル(Editor)と GPUI の入力系をつなぐ。
//!
//! writer も calc もここを共有する。**編集できることがこのソフトの存在理由**なので、
//! 入力の道は1本にして、両方のアプリで同じ挙動にする。
//!
//! IME(日本語入力)の要点:
//!   GPUI は UTF-16 の位置で来る。Editor はバイト位置で持つ。境界で必ず変換する。
//!   変換中(marked)は本文に見せるが、確定するまで undo の1手にしない。
//!   この規則は Editor 側に実装済みで、ここはその呼び分けをするだけ。

/// 日本語まわりの中身は `kotoba` にある(gpui を知らない層)。
/// ここから再輸出して、アプリ側の呼び出しは変えない。
pub use lang::ja::{furigana, proof};
pub use lang::{check, spell, Language, Target};
pub use lang::model::Endpoint;
pub mod icons;
pub mod ribbon;

use std::ops::Range;

use gpui::{actions, AssetSource, KeyBinding, SharedString};

/// リボンのアイコンを gpui に渡す(`svg().path("icons/bold.svg")` で引ける)。
/// フォントと違い、アイコンは**こちらの成果物の一部**なので埋め込んでよい
/// (Euro-Office 由来・AGPL。NOTICE.md に明記)。
pub struct Icons;

impl AssetSource for Icons {
    fn load(&self, path: &str) -> gpui::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        Ok(path
            .strip_prefix("icons/")
            .and_then(|n| n.strip_suffix(".svg"))
            .and_then(icons::find)
            .map(std::borrow::Cow::Borrowed))
    }

    fn list(&self, _path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(icons::ICONS.iter().map(|(k, _)| SharedString::from(format!("icons/{k}.svg"))).collect())
    }
}
use kumihan::Editor;

actions!(
    jo_edit,
    [
        Backspace, Delete, Left, Right, SelectLeft, SelectRight, SelectAll,
        SelectUp, SelectDown,
        Home, End, Enter, Undo, Redo, Save, Open, Up, Down, Tab,
        Copy, Cut, Paste, PasteValues, Quit, ContextMenu, Cancel,
        WordLeft, WordRight, SelectWordLeft, SelectWordRight, PageUp, PageDown,
        Find, DocHome, DocEnd,
    ]
);

/// 標準の割り当て。アプリの起動時に一度呼ぶ。
pub fn bindings(context: &'static str) -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("backspace", Backspace, Some(context)),
        KeyBinding::new("delete", Delete, Some(context)),
        KeyBinding::new("left", Left, Some(context)),
        KeyBinding::new("right", Right, Some(context)),
        KeyBinding::new("shift-left", SelectLeft, Some(context)),
        KeyBinding::new("shift-right", SelectRight, Some(context)),
        KeyBinding::new("ctrl-left", WordLeft, Some(context)),
        KeyBinding::new("ctrl-right", WordRight, Some(context)),
        KeyBinding::new("ctrl-shift-left", SelectWordLeft, Some(context)),
        KeyBinding::new("ctrl-shift-right", SelectWordRight, Some(context)),
        KeyBinding::new("pageup", PageUp, Some(context)),
        KeyBinding::new("pagedown", PageDown, Some(context)),
        KeyBinding::new("ctrl-f", Find, Some(context)),
        KeyBinding::new("ctrl-h", Find, Some(context)),
        KeyBinding::new("ctrl-home", DocHome, Some(context)),
        KeyBinding::new("ctrl-end", DocEnd, Some(context)),
        KeyBinding::new("shift-up", SelectUp, Some(context)),
        KeyBinding::new("shift-down", SelectDown, Some(context)),
        KeyBinding::new("ctrl-a", SelectAll, Some(context)),
        KeyBinding::new("home", Home, Some(context)),
        KeyBinding::new("end", End, Some(context)),
        KeyBinding::new("enter", Enter, Some(context)),
        KeyBinding::new("up", Up, Some(context)),
        KeyBinding::new("down", Down, Some(context)),
        KeyBinding::new("tab", Tab, Some(context)),
        KeyBinding::new("ctrl-z", Undo, Some(context)),
        KeyBinding::new("ctrl-shift-z", Redo, Some(context)),
        KeyBinding::new("ctrl-y", Redo, Some(context)),
        KeyBinding::new("ctrl-s", Save, Some(context)),
        KeyBinding::new("ctrl-o", Open, Some(context)),
        KeyBinding::new("ctrl-c", Copy, Some(context)),
        KeyBinding::new("ctrl-x", Cut, Some(context)),
        KeyBinding::new("ctrl-v", Paste, Some(context)),
        // 値だけの貼り付け(新しい Excel と同じ割り当て)
        KeyBinding::new("ctrl-shift-v", PasteValues, Some(context)),
        KeyBinding::new("ctrl-q", Quit, Some(context)),
        // メニューキー(アプリケーションキー)と Shift+F10 は右クリックと同じ
        KeyBinding::new("menu", ContextMenu, Some(context)),
        KeyBinding::new("shift-f10", ContextMenu, Some(context)),
        KeyBinding::new("escape", Cancel, Some(context)),
    ]
}

/// GPUI の EntityInputHandler が求める操作を、Editor の言葉に翻訳する。
///
/// アプリ側は「編集対象の Editor をくれ」とだけ実装すればよく、
/// UTF-16 との変換や marked の扱いはここで閉じる。
pub trait HasEditor {
    fn editor(&mut self) -> &mut Editor;
    fn editor_ref(&self) -> &Editor;
    /// 本文が変わったときに呼ばれる(組版のやり直し・再計算など)
    fn on_edited(&mut self) {}
}

/// EntityInputHandler の中身。アプリの impl から丸ごと委譲する。
pub mod handler {
    use super::*;

    pub fn text_for_range<T: HasEditor>(
        this: &mut T,
        range_utf16: Range<usize>,
        actual: &mut Option<Range<usize>>,
    ) -> Option<String> {
        let e = this.editor_ref();
        let r = e.byte_range(range_utf16);
        actual.replace(e.utf16_range(r.clone()));
        e.text().get(r).map(|s| s.to_string())
    }

    pub fn selected_range_utf16<T: HasEditor>(this: &T) -> Range<usize> {
        let e = this.editor_ref();
        e.utf16_range(e.selection())
    }

    pub fn marked_range_utf16<T: HasEditor>(this: &T) -> Option<Range<usize>> {
        let e = this.editor_ref();
        e.marked_range().map(|r| e.utf16_range(r))
    }

    pub fn unmark<T: HasEditor>(this: &mut T) {
        this.editor().clear_marked();
        this.on_edited();
    }

    /// 確定した文字が来た(通常の入力・IMEの確定・貼り付け)
    pub fn replace<T: HasEditor>(this: &mut T, range_utf16: Option<Range<usize>>, text: &str) {
        {
            let e = this.editor();
            if let Some(r) = range_utf16 {
                let b = e.byte_range(r);
                e.move_to(b.start, false);
                e.move_to(b.end, true);
            }
            // 変換中なら確定、そうでなければ普通の挿入。
            // どちらも undo の1手になる(Editor 側の規則)
            if e.marked_range().is_some() {
                e.commit_marked(text);
            } else {
                e.insert(text);
            }
        }
        this.on_edited();
    }

    /// 変換中の文字が来た(未確定)
    pub fn replace_and_mark<T: HasEditor>(
        this: &mut T,
        range_utf16: Option<Range<usize>>,
        text: &str,
        sel_utf16: Option<Range<usize>>,
    ) {
        {
            let e = this.editor();
            if let Some(r) = range_utf16 {
                let b = e.byte_range(r);
                e.move_to(b.start, false);
                e.move_to(b.end, true);
            }
            // 未確定の中での選択(変換対象の文節)はバイト位置に直す
            let sel = sel_utf16.map(|r| {
                let bytes = |u: usize| {
                    text.char_indices()
                        .scan(0usize, |acc, (b, c)| {
                            let cur = *acc;
                            *acc += c.len_utf16();
                            Some((cur, b))
                        })
                        .find(|(u16pos, _)| *u16pos >= u)
                        .map(|(_, b)| b)
                        .unwrap_or(text.len())
                };
                bytes(r.start)..bytes(r.end)
            });
            e.set_marked(text, sel);
        }
        this.on_edited();
    }

    pub fn text_len_utf16<T: HasEditor>(this: &T) -> usize {
        this.editor_ref().utf16_len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct App {
        ed: Editor,
        edits: usize,
    }
    impl HasEditor for App {
        fn editor(&mut self) -> &mut Editor { &mut self.ed }
        fn editor_ref(&self) -> &Editor { &self.ed }
        fn on_edited(&mut self) { self.edits += 1 }
    }

    fn app(s: &str) -> App {
        App { ed: Editor::new(s), edits: 0 }
    }

    #[test]
    fn 通常の入力が本文に入る() {
        let mut a = app("");
        handler::replace(&mut a, None, "日本フネン");
        assert_eq!(a.ed.text(), "日本フネン");
        assert_eq!(a.edits, 1, "組版のやり直しが呼ばれる");
    }

    #[test]
    fn ime_の一巡が通る() {
        let mut a = app("特定");
        // 「ぼうか」を打つ(未確定)
        handler::replace_and_mark(&mut a, None, "ぼうか", None);
        assert_eq!(a.ed.text(), "特定ぼうか");
        assert!(handler::marked_range_utf16(&a).is_some());
        // 変換して「防火」(まだ未確定)
        handler::replace_and_mark(&mut a, None, "防火", None);
        assert_eq!(a.ed.text(), "特定防火");
        // 確定
        handler::replace(&mut a, None, "防火");
        assert_eq!(a.ed.text(), "特定防火");
        assert!(handler::marked_range_utf16(&a).is_none());
        // undo は1手で変換前に戻る
        assert!(a.ed.undo());
        assert_eq!(a.ed.text(), "特定");
    }

    #[test]
    fn utf16の範囲指定で置き換わる() {
        let mut a = app("あいうえお");
        // UTF-16 で 1..3 =「いう」
        handler::replace(&mut a, Some(1..3), "XY");
        assert_eq!(a.ed.text(), "あXYえお");
    }

    #[test]
    fn 選択範囲がutf16で返る() {
        let mut a = app("あa亜");
        a.ed.select_all();
        // あ=1, a=1, 亜=1 → 3単位
        assert_eq!(handler::selected_range_utf16(&a), 0..3);
        assert_eq!(handler::text_len_utf16(&a), 3);
    }

    #[test]
    fn 変換の取り消しで跡が残らない() {
        let mut a = app("設備");
        handler::replace_and_mark(&mut a, None, "りよう", None);
        handler::unmark(&mut a);
        assert_eq!(a.ed.text(), "設備");
    }

    #[test]
    fn 文節の選択がバイト位置に直る() {
        let mut a = app("");
        // 「日本フネン」のうち UTF-16 で 0..2 =「日本」が変換対象
        handler::replace_and_mark(&mut a, None, "日本フネン", Some(0..2));
        assert_eq!(a.ed.selection(), 0.."日本".len(), "UTF-16→バイトの変換が違う");
    }
}

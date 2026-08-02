//! 編集モデル — カーソル・選択・undo/redo・IMEの未確定文字。UI非依存。
//!
//! 位置はすべて**バイト位置**(Rustの文字列と同じ物差し)で持つ。
//! GPUIのIME APIは UTF-16 の位置で来るので、境界で変換する(utf16.rs 相当は
//! ここに小さく持つ)。日本語は UTF-8 で3バイト・UTF-16 で1〜2単位なので、
//! ここを混同すると即座に文字化けする — だから型で分けず、関数名で分ける。
//!
//! IMEの要点(K2の難所):
//!   変換中の文字列は「まだ文書ではない」。marked(未確定)として本文に載せつつ、
//!   確定(commit)するまでは undo の単位にしない。変換をやめたら丸ごと消える。

use std::ops::Range;

#[derive(Debug, Clone, PartialEq)]
struct Snapshot {
    text: String,
    cursor: usize,
    anchor: usize,
}

/// 一つの編集可能なテキスト。
#[derive(Debug, Clone)]
pub struct Editor {
    text: String,
    /// 選択の可動端(キャレット)。バイト位置
    cursor: usize,
    /// 選択の固定端。cursor と同じなら選択なし
    anchor: usize,
    /// IMEの未確定範囲(バイト位置)。変換中だけ Some
    marked: Option<Range<usize>>,
    /// 変換が始まる前の状態。確定したときの undo はここへ戻る
    /// (未確定を消した後の状態に戻ってはいけない — 選択を置き換える変換で壊れる)
    pre_ime: Option<Snapshot>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

impl Default for Editor {
    fn default() -> Self {
        Editor::new("")
    }
}

impl Editor {
    pub fn new(text: &str) -> Editor {
        let n = text.len();
        Editor {
            text: text.to_string(),
            cursor: n,
            anchor: n,
            marked: None,
            pre_ime: None,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    pub fn marked_range(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }
    pub fn selection(&self) -> Range<usize> {
        let (a, b) = (self.anchor.min(self.cursor), self.anchor.max(self.cursor));
        a..b
    }
    pub fn has_selection(&self) -> bool {
        self.anchor != self.cursor
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { text: self.text.clone(), cursor: self.cursor, anchor: self.anchor }
    }

    /// undo の区切りを打つ。IMEの未確定中は打たない(確定して初めて1手)。
    fn checkpoint(&mut self) {
        let s = self.snapshot();
        if self.undo.last().map(|x| &x.text) != Some(&s.text) {
            self.undo.push(s);
            self.redo.clear();
        }
    }

    // ---------- 移動と選択 ----------

    /// 文字単位で移動。extend=true なら選択を伸ばす。
    pub fn move_char(&mut self, forward: bool, extend: bool) {
        let p = if forward {
            next_boundary(&self.text, self.cursor)
        } else {
            prev_boundary(&self.text, self.cursor)
        };
        self.cursor = p;
        if !extend {
            self.anchor = p;
        }
        self.marked = None;
    }

    pub fn move_to(&mut self, byte: usize, extend: bool) {
        let p = clamp_boundary(&self.text, byte);
        self.cursor = p;
        if !extend {
            self.anchor = p;
        }
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.cursor = self.text.len();
    }

    // ---------- 入力 ----------

    /// 選択を置き換えて文字列を入れる(通常の入力・貼り付け)。undo の1手。
    pub fn insert(&mut self, s: &str) {
        self.checkpoint();
        let r = self.replace_range();
        self.text.replace_range(r.clone(), s);
        self.cursor = r.start + s.len();
        self.anchor = self.cursor;
        self.marked = None;
    }

    /// 後退削除。選択があればそれを消す。
    pub fn backspace(&mut self) {
        if self.has_selection() {
            self.insert("");
            return;
        }
        if self.cursor == 0 {
            return;
        }
        self.checkpoint();
        let p = prev_boundary(&self.text, self.cursor);
        self.text.replace_range(p..self.cursor, "");
        self.cursor = p;
        self.anchor = p;
    }

    pub fn delete(&mut self) {
        if self.has_selection() {
            self.insert("");
            return;
        }
        if self.cursor >= self.text.len() {
            return;
        }
        self.checkpoint();
        let n = next_boundary(&self.text, self.cursor);
        self.text.replace_range(self.cursor..n, "");
    }

    // ---------- IME(未確定文字) ----------

    /// 変換中の文字列を置く。確定していないので undo の区切りは打たない。
    /// `sel` は未確定文字列の中での選択(変換対象の文節)をバイト位置で。
    pub fn set_marked(&mut self, s: &str, sel: Option<Range<usize>>) {
        // 変換の開始時点(まだ何も置いていない状態)を控える
        if self.marked.is_none() {
            self.pre_ime = Some(self.snapshot());
        }
        // 前回の未確定があればそれを、無ければ選択範囲を置き換える
        let r = match self.marked.clone() {
            Some(m) => m,
            None => self.replace_range(),
        };
        self.text.replace_range(r.clone(), s);
        if s.is_empty() {
            self.marked = None;
            self.cursor = r.start;
            self.anchor = r.start;
            return;
        }
        let start = r.start;
        self.marked = Some(start..start + s.len());
        let (a, b) = match sel {
            Some(x) => (start + x.start, start + x.end),
            None => (start + s.len(), start + s.len()),
        };
        self.anchor = clamp_boundary(&self.text, a);
        self.cursor = clamp_boundary(&self.text, b);
    }

    /// 変換を確定する。ここで初めて undo の1手になる。
    ///
    /// 戻り先は**変換が始まる前**の状態。未確定を消した後の状態を控えると、
    /// 選択を置き換える形で始まった変換(選択→変換→確定)で、
    /// undo が「選択していた元の文字列」ではなく空に戻ってしまう。
    pub fn commit_marked(&mut self, s: &str) {
        let before = self.pre_ime.take().unwrap_or_else(|| self.snapshot());
        let r = match self.marked.take() {
            Some(m) => m,
            None => self.replace_range(),
        };
        self.text.replace_range(r.clone(), s);
        self.cursor = r.start + s.len();
        self.anchor = self.cursor;
        if before.text != self.text {
            self.undo.push(before);
            self.redo.clear();
        }
    }

    /// 変換をやめる(未確定を捨てる)。
    pub fn clear_marked(&mut self) {
        self.pre_ime = None;
        if let Some(m) = self.marked.take() {
            self.text.replace_range(m.clone(), "");
            self.cursor = m.start;
            self.anchor = m.start;
        }
    }

    fn replace_range(&self) -> Range<usize> {
        if self.has_selection() {
            self.selection()
        } else {
            self.cursor..self.cursor
        }
    }

    // ---------- undo / redo ----------

    pub fn undo(&mut self) -> bool {
        // 未確定のまま undo されたら、まず変換を捨てる
        if self.marked.is_some() {
            self.clear_marked();
            return true;
        }
        match self.undo.pop() {
            Some(prev) => {
                self.redo.push(self.snapshot());
                self.text = prev.text;
                self.cursor = prev.cursor;
                self.anchor = prev.anchor;
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(next) => {
                self.undo.push(self.snapshot());
                self.text = next.text;
                self.cursor = next.cursor;
                self.anchor = next.anchor;
                true
            }
            None => false,
        }
    }

    // ---------- UTF-16 との境界(IME APIのため) ----------

    pub fn utf16_len(&self) -> usize {
        self.text.chars().map(char::len_utf16).sum()
    }

    /// UTF-16 位置 → バイト位置
    pub fn utf16_to_byte(&self, u: usize) -> usize {
        let mut acc = 0;
        for (b, c) in self.text.char_indices() {
            if acc >= u {
                return b;
            }
            acc += c.len_utf16();
        }
        self.text.len()
    }

    /// バイト位置 → UTF-16 位置
    pub fn byte_to_utf16(&self, b: usize) -> usize {
        let b = b.min(self.text.len());
        self.text[..b].chars().map(char::len_utf16).sum()
    }

    pub fn utf16_range(&self, r: Range<usize>) -> Range<usize> {
        self.byte_to_utf16(r.start)..self.byte_to_utf16(r.end)
    }
    pub fn byte_range(&self, r: Range<usize>) -> Range<usize> {
        self.utf16_to_byte(r.start)..self.utf16_to_byte(r.end)
    }
}

fn clamp_boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn next_boundary(s: &str, i: usize) -> usize {
    s[i..].chars().next().map_or(i, |c| i + c.len_utf8())
}

fn prev_boundary(s: &str, i: usize) -> usize {
    s[..i].chars().next_back().map_or(0, |c| i - c.len_utf8())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 日本語を入れて消せる() {
        let mut e = Editor::new("");
        e.insert("日本フネン");
        assert_eq!(e.text(), "日本フネン");
        assert_eq!(e.cursor(), "日本フネン".len());
        e.backspace();
        assert_eq!(e.text(), "日本フネ", "1文字=3バイトを丸ごと消す");
    }

    #[test]
    fn カーソルは文字境界で動く() {
        let mut e = Editor::new("あa亜");
        e.move_to(0, false);
        e.move_char(true, false);
        assert_eq!(e.cursor(), 3, "あ は3バイト");
        e.move_char(true, false);
        assert_eq!(e.cursor(), 4, "a は1バイト");
        e.move_char(false, false);
        assert_eq!(e.cursor(), 3);
    }

    #[test]
    fn 選択して置き換えられる() {
        let mut e = Editor::new("防火ドア");
        e.move_to(0, false);
        e.move_char(true, true);
        e.move_char(true, true);
        assert_eq!(e.selection(), 0.."防火".len());
        e.insert("玄関");
        assert_eq!(e.text(), "玄関ドア");
    }

    // ---- IME: K2 の難所 ----

    #[test]
    fn ime_変換中は未確定として本文に見えるが確定で一手になる() {
        let mut e = Editor::new("特定");
        // 「ぼうか」と打つ(未確定)
        e.set_marked("ぼうか", None);
        assert_eq!(e.text(), "特定ぼうか");
        assert_eq!(e.marked_range(), Some(6..6 + "ぼうか".len()));
        // 変換して「防火」に(まだ未確定)
        e.set_marked("防火", None);
        assert_eq!(e.text(), "特定防火", "未確定は置き換わる。積み重ならない");
        // 確定
        e.commit_marked("防火");
        assert_eq!(e.text(), "特定防火");
        assert_eq!(e.marked_range(), None);
        // undo は「防火の確定」を1手として戻す(かな入力の途中には戻らない)
        assert!(e.undo());
        assert_eq!(e.text(), "特定");
    }

    #[test]
    fn ime_変換をやめると未確定は丸ごと消える() {
        let mut e = Editor::new("設備");
        e.set_marked("りよう", None);
        assert_eq!(e.text(), "設備りよう");
        e.clear_marked();
        assert_eq!(e.text(), "設備", "変換をやめたら跡が残らない");
        assert_eq!(e.cursor(), "設備".len());
    }

    #[test]
    fn ime_変換中のundoは変換の取り消しになる() {
        let mut e = Editor::new("申込");
        e.set_marked("よう", None);
        assert!(e.undo(), "未確定があるときの undo は変換の取り消し");
        assert_eq!(e.text(), "申込");
        assert!(!e.undo(), "それ以上は戻らない");
    }

    #[test]
    fn ime_文節の選択が未確定の中で表される() {
        let mut e = Editor::new("");
        // 「にほんふねん」→ 変換候補「日本フネン」、うち「日本」が変換対象
        e.set_marked("日本フネン", Some(0.."日本".len()));
        assert_eq!(e.selection(), 0.."日本".len());
        assert_eq!(e.marked_range(), Some(0.."日本フネン".len()));
    }

    #[test]
    fn ime_選択を置き換える形で変換が始まる() {
        let mut e = Editor::new("旧製品");
        e.select_all();
        e.set_marked("しんせいひん", None);
        assert_eq!(e.text(), "しんせいひん", "選択は未確定に置き換わる");
        e.commit_marked("新製品");
        assert_eq!(e.text(), "新製品");
        assert!(e.undo());
        assert_eq!(e.text(), "旧製品", "1手で元に戻る");
    }

    #[test]
    fn undoとredoが往復する() {
        let mut e = Editor::new("");
        e.insert("一");
        e.insert("二");
        e.insert("三");
        assert_eq!(e.text(), "一二三");
        assert!(e.undo());
        assert!(e.undo());
        assert_eq!(e.text(), "一");
        assert!(e.redo());
        assert_eq!(e.text(), "一二");
        e.insert("四");
        assert!(!e.redo(), "新しい編集の後は redo が消える");
        assert_eq!(e.text(), "一二四");
    }

    #[test]
    fn utf16との往復が保たれる() {
        let e = Editor::new("あa𩸽い"); // 𩸽 は UTF-16 で2単位・UTF-8 で4バイト
        assert_eq!(e.utf16_len(), 1 + 1 + 2 + 1);
        for (b, _) in e.text().char_indices().chain([(e.text().len(), ' ')]) {
            let u = e.byte_to_utf16(b);
            assert_eq!(e.utf16_to_byte(u), b, "バイト{b} ⇄ UTF-16 {u} が往復しない");
        }
    }

    #[test]
    fn 空でも壊れない() {
        let mut e = Editor::new("");
        e.backspace();
        e.delete();
        e.move_char(false, false);
        e.move_char(true, false);
        assert_eq!(e.text(), "");
        assert!(!e.undo());
    }
}

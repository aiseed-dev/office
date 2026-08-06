//! calc — xlsx互換の表計算。writer とは**別のソフト**。
//!
//! Office を一つのソフトにしない。文書は writer、表は calc。
//! 共有するのは書式(docx/xlsx)だけ。
//!
//! **マクロは無い。** 表の中に実行コードを置かない設計で、
//! 「開く=実行」という攻撃経路を最初から持たない。
//!
//!   calc            空で開く
//!   calc 表.xlsx    その表を開く

pub(crate) use std::ops::Range;
pub(crate) use std::path::PathBuf;

pub(crate) use gpui::{
    div, prelude::*, px, rgb, size, App, Bounds, Context, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, SharedString, UTF16Selection, Window,
    WindowBounds, WindowOptions,
};
pub(crate) use gpui_platform::application;
pub(crate) use kumihan::Editor;

pub(crate) use sheet::model::{Borders, CellFormat, HAlign};
pub(crate) use sheet::{recalc, recalc_book, Book, Cell, Pos, Value};
pub(crate) use ui::{handler, ribbon, HasEditor};

mod funcs;
mod util;
pub(crate) use util::*;
mod py;
pub(crate) use py::*;
mod io;
pub(crate) use io::*;
mod picks;
#[cfg(test)]
mod tests;

struct Calc {
    focus: FocusHandle,
    book: Book,
    active: usize,
    cursor: Pos,
    /// 範囲選択の起点(Shift+矢印/クリックで伸ばす)。無ければ1セル
    anchor: Option<Pos>,
    /// ドラッグ選択の始点(マウスの左を押した位置。離すと終わる)
    drag: Option<Pos>,
    /// 見出しの境界を掴んだドラッグ(列幅・行高)。セル選択の drag とは別
    size_drag: Option<SizeDrag>,
    /// 見出しを掴んだ選択ドラッグ(列か, 始まりの番号)。B→D と撫でて複数列
    head_drag: Option<(bool, u32)>,
    /// 画像の復号の控え(実体のアドレス → GPUI の画像)。
    /// 毎フレーム作り直すと復号と転送をやり直すことになる
    img_cache: std::cell::RefCell<std::collections::HashMap<usize, std::sync::Arc<gpui::Image>>>,
    /// 検索と置換の検索語(板を2枚続けて使う間の控え。次回の初期値にもなる)
    find_term: Option<String>,
    /// ゴールシークの途中の控え(目標セル, 目標値)
    goal: Option<(Pos, f64)>,
    /// ピボットの聞き取りの途中経過(元の範囲・見出し・決めた欄)
    pivot_pend: Option<PivotPend>,
    /// 小計の聞き取りの途中経過(同じ形の控えを使い回す)
    sub_pend: Option<PivotPend>,
    /// ソルバーの小窓(開いている間、打鍵は選んだ欄へ)
    solver: Option<Solver>,
    /// SmartArt の選択中の分類(2段の pick の1段目の答え)
    sa_cat: usize,
    /// スライサー(列, 選んだ値たち, 複数選択か)。**見え方だけ** —
    /// 絞り込みと同じで、保存される中身は変わらない
    slicer: Option<(u32, std::collections::BTreeSet<String>, bool)>,
    /// コメントを見せるか(共同編集タブで切替。隠しても付いたまま)
    show_comments: bool,
    /// 暗号化のパスワード(次の保存から効く。開いた暗号化ブックからも入る)
    encrypt_pw: Option<String>,
    /// 「開くために聞いている」パスワード待ちのファイル
    pw_pending: Option<PathBuf>,
    /// pick の一覧が指す実体(バージョン履歴・プラグインの表示名 → パス)
    pick_paths: Vec<(String, PathBuf)>,
    /// PY のスピルの台帳(シート番号, 錨 → 行×列)。次の @計算 で前の面を消す
    py_spills: std::collections::HashMap<(usize, Pos), (u32, u32)>,
    /// トレースの光り(参照元=青緑 / 参照先=橙)。見え方だけ、保存されない
    trace: Vec<(Pos, bool)>,
    /// 自分が置いた排他ロック(閉じるとき・別のファイルを開くときに外す)
    my_lock: Option<PathBuf>,
    /// 先客の名乗り(このファイルは誰かが開いている)。上書き保存を止める
    locked_by: Option<String>,
    /// 選択中の図形(shapes_new の番号)。Esc/他クリックで解除、Del で削除
    shape_sel: Option<usize>,
    /// 図形のドラッグ(番号, 掴んだ格子px, 掴んだ時の錨の格子px, 大きさ変更か)
    shape_drag: Option<(usize, (f32, f32), (f32, f32), bool)>,
    /// ホイールの端数(触板の細かい送りを捨てずに貯める)
    wheel: (f32, f32),
    /// 窓の大きさ(px)。描画のたびに実測 — **見える範囲**の計算に使う。
    /// セルの大きさは設定どおり固定で、窓に合わせて伸縮させない
    view_w_px: f32,
    view_h_px: f32,
    /// このセルで**編集を始めた**(F2・ダブルクリック・打ち始め)。
    /// 立っていない間の最初の打鍵は、既存の中身を消して置き換える
    /// (Excel の作法)。セルを移ると降りる(sync_input)
    edit_armed: bool,
    /// 名前ボックスの打ちかけ(数式バーの左端)。番地・範囲・名前で飛び、
    /// 知らない名前なら**いまの選択に付ける**(Excel の名前ボックスと同じ)
    name_edit: Option<Editor>,
    /// 「関数を挿入」の小窓(検索・分類・一覧・説明)
    fn_dlg: Option<FnDlg>,
    /// 「関数の引数」の画面(次へ、で進む第2段)
    fn_args: Option<FnArgs>,
    /// 式の直入力中のセル掴み(起点, 入れた参照の文字の範囲)。
    /// クリックで参照がカーソルに入り、ドラッグで範囲(A1:C9)に伸びる
    ref_pick: Option<(Pos, std::ops::Range<usize>)>,
    /// 終了確認の板(未保存の変更があるときに出る。窓の中の中央)
    quit_ask: bool,
    /// 右クリックのメニュー(出ている場所。格子領域の px)
    menu_at: Option<(f32, f32)>,
    /// 開いている子メニュー(挿入▸ など)
    menu_sub: Option<&'static str>,
    /// 「ドロップダウンリストから選択」などの一覧(候補, 出す場所)
    pick: Option<(Vec<String>, (f32, f32))>,
    /// pick の中身の意味: "value"=セルに入れる / "font"=書体 / "size"=文字の大きさ
    pick_kind: &'static str,
    /// 耳(シートのタブ)のメニューが指しているシート(右クリックで開く)。
    /// 改名・色の2段目の板が閉じるまで持ち越す
    sheet_menu_at: Option<usize>,
    /// 書式の小窓(セルをフォーマットする)。範囲を選び直しながら使える
    fmt_panel: Option<(f32, f32)>,
    /// 小さな入力の板(種類, 入力欄)。"name"=名前の定義。開いている間は打鍵がここへ
    prompt: Option<(&'static str, Editor)>,
    /// 数式を値の代わりに出す(数式の表示)
    show_formulas: bool,
    /// 画面の窓の左上(スクロール)。**表は画面より大きい**
    view: Pos,
    /// 固定する行数・列数(見出しを置き去りにしないため)。カーソル位置で決める
    frozen: Option<Pos>,
    /// 絞り込み(列, 値)。**見え方だけ** — 保存される中身は変わらない
    filter: Option<(u32, String)>,
    /// 表の操作(書式・フィル・行列・結合・並べ替え)を戻すための控え。
    /// 入力欄の undo とは別 — **戻せない操作は事故のとき逃げ道が無い**。
    /// 1手 = シートの控えの束。普通の操作は1枚、Python の実行のように
    /// 複数シートに触るものは全部まとめて1手(どれでも1手で戻せる)。
    /// **どのシートの控えかを一緒に持つ** — シートを切り替えた後の undo が
    /// 別のシートへ他所の中身を書き戻す事故を防ぐ
    undo_stack: Vec<Vec<(usize, sheet::Sheet)>>,
    redo_stack: Vec<Vec<(usize, sheet::Sheet)>>,
    /// シートごとのカーソル・窓・固定(切り替えても場所を失わない)
    sheet_ui: Vec<(Pos, Pos, Option<Pos>)>,
    /// コピーの控え(起点, そのとき書いた TSV)。貼り付け時に系のクリップボードと
    /// 突き合わせ、一致すればアプリ内コピーとして式の参照をずらす
    clip: Option<(Pos, String)>,
    /// コピーの控え(セルそのもの)。形式を選択して貼り付け(値だけ・書式だけ)に使う
    clip_cells: Option<Vec<Vec<Option<Cell>>>>,
    /// コピーした範囲(シート, 左上, 右下)。破線の枠で見せる。Esc で消える
    clip_range: Option<(usize, Pos, Pos)>,
    /// グリッド線(表の薄い線)を出す
    gridlines: bool,
    /// 数式バーの中身。IMEもここに来る(セルの入力は1本のテキスト編集)
    input: Editor,
    path: Option<PathBuf>,
    status: SharedString,
    notes: Vec<SharedString>,
    dirty: bool,
    /// 選んでいるリボンのタブ
    tab: usize,
    /// ファイルの全面ページから「戻る」ときのタブ
    prev_tab: usize,
    /// 釦に乗っているときの名前(下のステータスバーに出す)
    hover_hint: Option<&'static str>,
    /// ファイルのページの右側(0=詳細情報 1=最近開いた)
    file_view: u8,
    /// 表示の倍率(表示タブのズーム。0.5〜2.0)
    zoom: f32,
    /// 数式バーを見せるか(表示タブ)
    show_formula_bar: bool,
    /// 行番号・列名の見出しを見せるか(表示タブ)
    show_headers: bool,
    /// 0 の値を見せるか(表示タブ。消しても値は 0 のまま)
    show_zeros: bool,
    /// 画面を暗くする(インターフェイステーマ)。**セルは白のまま** —
    /// 画面と紙の一致を守る(writer の「紙は白のまま」と同じ考え)
    dark: bool,
    /// 自動で再計算するか(数式タブの「計算方法」。手動のときは F9)
    auto_calc: bool,
    /// 見張り(ウォッチウィンドウ)。(シート番号, セル)
    watch: Vec<(usize, Pos)>,
    /// AI に頼み中(終わるまで次の頼みは断る)
    ai_busy: bool,
    /// 描画の道具(0=ペン 1=蛍光ペン 2=消しゴム)。writer と同じ形
    tool: Option<u8>,
    /// 描きかけの線(ドラッグ中)
    ink_cur: Option<Vec<(f32, f32)>>,
}

impl HasEditor for Calc {
    // 小さな入力の板(名前の定義など)・ソルバーの小窓が開いている間は、
    // 打鍵(IME含む)はそこへ
    fn editor(&mut self) -> &mut Editor {
        if let Some(ed) = &mut self.name_edit {
            return ed;
        }
        if let Some(a) = &mut self.fn_args {
            if !a.eds.is_empty() {
                let i = a.focus.min(a.eds.len() - 1);
                return &mut a.eds[i];
            }
        }
        if let Some(d) = &mut self.fn_dlg {
            return &mut d.search;
        }
        if let Some(sv) = &mut self.solver {
            return sv.focused();
        }
        match &mut self.prompt {
            Some((_, ed)) => ed,
            None => &mut self.input,
        }
    }
    fn editor_ref(&self) -> &Editor {
        if let Some(ed) = &self.name_edit {
            return ed;
        }
        if let Some(a) = &self.fn_args {
            if !a.eds.is_empty() {
                let i = a.focus.min(a.eds.len() - 1);
                return &a.eds[i];
            }
        }
        if let Some(d) = &self.fn_dlg {
            return &d.search;
        }
        if let Some(sv) = &self.solver {
            return sv.focused_ref();
        }
        match &self.prompt {
            Some((_, ed)) => ed,
            None => &self.input,
        }
    }
    fn on_edited(&mut self) {
        // 検索を打ち替えたら一覧の選択は先頭に戻す
        if let Some(d) = &mut self.fn_dlg {
            d.sel = 0;
        }
        // 引数を打ち替えたら結果の下見を計算し直す
        if self.fn_args.is_some() {
            self.fn_args_recalc();
        }
        // 板・小窓・名前ボックスへの打鍵は文書を変えない
        if self.prompt.is_none() && self.name_edit.is_none()
            && self.fn_dlg.is_none() && self.fn_args.is_none()
        {
            self.dirty = true;
            // 式の直入力の支援: 打ちかけの関数名の補完一覧と、引数のヒント
            self.formula_assist();
        }
    }
}

impl Calc {
    fn new(path: Option<PathBuf>, cx: &mut Context<Self>) -> Calc {
        let mut c = Calc {
            focus: cx.focus_handle(),
            book: Book::new(),
            active: 0,
            cursor: Pos::new(0, 0),
            anchor: None,
            drag: None,
            size_drag: None,
            head_drag: None,
            img_cache: Default::default(),
            find_term: None,
            pivot_pend: None,
            sub_pend: None,
            solver: None,
            sa_cat: 0,
            slicer: None,
            show_comments: true,
            pick_paths: Vec::new(),
            encrypt_pw: None,
            pw_pending: None,
            goal: None,
            py_spills: Default::default(),
            trace: Vec::new(),
            my_lock: None,
            locked_by: None,
            shape_sel: None,
            shape_drag: None,
            wheel: (0.0, 0.0),
            view_w_px: 0.0,
            view_h_px: 0.0,
            edit_armed: false,
            name_edit: None,
            fn_dlg: None,
            fn_args: None,
            ref_pick: None,
            quit_ask: false,
            menu_at: None,
            menu_sub: None,
            pick: None,
            pick_kind: "value",
            sheet_menu_at: None,
            fmt_panel: None,
            prompt: None,
            show_formulas: false,
            view: Pos::new(0, 0),
            frozen: None,
            filter: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            sheet_ui: Vec::new(),
            clip: None,
            clip_cells: None,
            clip_range: None,
            gridlines: true,
            input: Editor::new(""),
            path: None,
            status: "".into(),
            notes: Vec::new(),
            dirty: false,
            tab: 1, // ファイルは全面ページになったので、開きはホーム
            prev_tab: 1,
            hover_hint: None,
            file_view: 0,
            zoom: 1.0,
            show_formula_bar: true,
            show_headers: true,
            show_zeros: true,
            dark: false,
            auto_calc: true,
            watch: Vec::new(),
            ai_busy: false,
            tool: None,
            ink_cur: None,
        };
        if let Some(p) = path {
            c.open(p);
        } else {
            // 新規は空白のブック(発注者 2026-08-06。見本を入れない —
            // 試験は自前で表を作り、触れる見本は sample/*.xlsx にある)
            c.status = ui::t!("セルを選んで打つ。Enter で確定して下へ、Ctrl+S で保存").into();
        }
        c.sync_input();
        c
    }

    fn sheet(&self) -> &sheet::Sheet {
        &self.book.sheets[self.active]
    }
    fn sheet_mut(&mut self) -> &mut sheet::Sheet {
        let a = self.active;
        &mut self.book.sheets[a]
    }

    fn sync_input(&mut self) {
        let s = self.sheet().get(self.cursor).map(|c| c.editable()).unwrap_or_default();
        self.input = Editor::new(&s);
        self.edit_armed = false; // セルを移った=編集は仕切り直し
        if self.pick_kind == "fn-complete" {
            self.pick = None; // 補完の一覧も畳む
        }
    }

    /// 数式バーの内容をセルに入れて再計算する。
    /// いまの表を控える(次の操作を戻せるように)。やり直しの控えは捨てる。
    fn checkpoint(&mut self) {
        self.undo_stack
            .push(vec![(self.active, self.book.sheets[self.active].clone())]);
        if self.undo_stack.len() > 100 {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    /// 全シートを1手として控える(Python の実行など、どこを変えるか
    /// 分からない操作の前に)。
    fn checkpoint_book(&mut self) {
        self.undo_stack.push(
            self.book
                .sheets
                .iter()
                .cloned()
                .enumerate()
                .collect(),
        );
        if self.undo_stack.len() > 100 {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    /// 控えたシートを見せる(別のシートの操作を戻したなら、そこへ移る —
    /// 見えない場所で表が変わるのは事故のもと)。
    fn show_sheet(&mut self, idx: usize) {
        if idx != self.active && idx < self.book.sheets.len() {
            self.remember_ui();
            self.active = idx;
            self.restore_ui();
            self.anchor = None;
            self.filter = None;
        }
    }

    fn undo_sheet(&mut self) {
        let Some(batch) = self.undo_stack.pop() else {
            self.status = ui::t!("戻すものがありません").into();
            return;
        };
        let mut redo = Vec::new();
        let first = batch.first().map(|(i, _)| *i);
        for (idx, prev) in batch {
            if idx < self.book.sheets.len() {
                redo.push((idx, self.book.sheets[idx].clone()));
                self.book.sheets[idx] = prev;
                recalc_book(&mut self.book, idx);
            }
        }
        self.redo_stack.push(redo);
        if let Some(i) = first {
            self.show_sheet(i);
        }
        self.dirty = true;
        self.sync_input();
        self.status = ui::t!("戻しました").into();
    }

    fn redo_sheet(&mut self) {
        let Some(batch) = self.redo_stack.pop() else {
            self.status = ui::t!("やり直すものがありません").into();
            return;
        };
        let mut undo = Vec::new();
        let first = batch.first().map(|(i, _)| *i);
        for (idx, next) in batch {
            if idx < self.book.sheets.len() {
                undo.push((idx, self.book.sheets[idx].clone()));
                self.book.sheets[idx] = next;
                recalc_book(&mut self.book, idx);
            }
        }
        self.undo_stack.push(undo);
        if let Some(i) = first {
            self.show_sheet(i);
        }
        self.dirty = true;
        self.sync_input();
        self.status = ui::t!("やり直しました").into();
    }

    /// いまのシートのカーソル・窓・固定を控える。
    fn remember_ui(&mut self) {
        while self.sheet_ui.len() < self.book.sheets.len() {
            self.sheet_ui.push((Pos::new(0, 0), Pos::new(0, 0), None));
        }
        self.sheet_ui[self.active] = (self.cursor, self.view, self.frozen);
    }

    fn restore_ui(&mut self) {
        let (c, v, f) = self
            .sheet_ui
            .get(self.active)
            .copied()
            .unwrap_or((Pos::new(0, 0), Pos::new(0, 0), None));
        self.cursor = c;
        self.view = v;
        self.frozen = f;
    }

    /// 画面に出ている行の並び(絞り込み中はその行だけ。グループ化で畳んだ行は
    /// 飛ばす)。描画と当たり判定で共有する。
    /// スライサーで残る行か(選びが空なら全部残る)。1行目=見出しは常に残す。
    fn slicer_keeps(&self, r: u32) -> bool {
        let Some((col, sel, _)) = &self.slicer else { return true };
        if sel.is_empty() || r == 0 {
            return true;
        }
        let v = self
            .sheet()
            .get(Pos::new(r, *col))
            .map(|c| c.value.display())
            .unwrap_or_default();
        let v = if v.is_empty() { ui::t!("(空白)").to_string() } else { v };
        sel.contains(&v)
    }

    /// 窓に入る行数。**セルの大きさは固定**で、窓が大きいほど多くの行が
    /// 見える(発注者 2026-08-06)。まだ窓の大きさを知らない(描画前・試験)
    /// なら従来の既定。少し多めに数えても、はみ出しは器が刈る
    fn rows_fit(&self) -> u32 {
        self.rows_fit_in(self.view_h_px)
    }

    fn rows_fit_in(&self, budget: f32) -> u32 {
        if self.view_h_px <= 0.0 {
            return ROWS; // 描画前・試験は従来の既定
        }
        let (mut h, mut n, mut r) = (0.0f32, 0u32, self.view.row);
        while h < budget && n < 300 {
            h += self.row_px(r);
            r += 1;
            n += 1;
        }
        n.max(3)
    }

    /// 端の追従・ページ移動用: 額縁(リボン・数式バー・耳・状態行)を
    /// 差し引いた「確実に丸ごと見える」行数
    fn rows_snug(&self) -> u32 {
        self.rows_fit_in(self.view_h_px - 270.0)
    }

    /// 窓に入る列数(rows_fit と同じ役割)
    fn cols_fit(&self) -> u32 {
        self.cols_fit_in(self.view_w_px)
    }

    fn cols_fit_in(&self, budget: f32) -> u32 {
        if self.view_w_px <= 0.0 {
            return COLS;
        }
        let (mut w, mut n, mut c) = (0.0f32, 0u32, self.view.col);
        while w < budget && n < 120 {
            w += self.col_px(c);
            c += 1;
            n += 1;
        }
        n.max(2)
    }

    fn cols_snug(&self) -> u32 {
        self.cols_fit_in(self.view_w_px - HEAD_W - 24.0)
    }

    fn visible_rows(&self) -> Vec<u32> {
        let hidden = &self.sheet().row_hidden;
        let fit = self.rows_fit();
        match &self.filter {
            Some((col, v)) => self
                .matching_rows(*col, v)
                .into_iter()
                .filter(|r| !hidden.contains(r) && self.slicer_keeps(*r))
                .take(fit as usize)
                .collect(),
            None if self.slicer.as_ref().is_some_and(|(_, sel, _)| !sel.is_empty()) => {
                // スライサーで絞る: 見出し+選んだ値の行(絞り込みと同じ流儀)
                let (rows, _) = self.sheet().extent();
                (0..rows)
                    .filter(|r| !hidden.contains(r) && self.slicer_keeps(*r))
                    .take(fit as usize)
                    .collect()
            }
            None => {
                // 畳んだ行のぶん多めに見て、画面の行数まで詰める
                let extra = hidden.len() as u32;
                grid_rows(self.frozen, self.view, fit + extra)
                    .into_iter()
                    .filter(|r| !hidden.contains(r))
                    .take(fit as usize)
                    .collect()
            }
        }
    }

    /// 画面に出ている列の並び(畳んだ列は飛ばす)。visible_rows と同じ役割。
    fn visible_cols(&self) -> Vec<u32> {
        let hidden = &self.sheet().col_hidden;
        let extra = hidden.len() as u32;
        let fit = self.cols_fit();
        let mut v: Vec<u32> = grid_cols(self.frozen, self.view, fit + extra)
            .into_iter()
            .filter(|c| !hidden.contains(c))
            .take(fit as usize)
            .collect();
        if self.sheet().rtl {
            // 右から左のシートは列を逆順に並べる。**描画も当たり判定も
            // この一点を通る**ので、掴む場所と見える場所がずれない
            v.reverse();
        }
        v
    }

    /// 格子の中の位置(px、格子領域の左上原点)からセルを逆算する。
    /// 見出しの帯の上なら None。
    fn cell_at(&self, x: f32, y: f32) -> Option<Pos> {
        if x < self.head_w() || y < self.head_h() {
            return None;
        }
        Some(Pos { row: self.row_at(y)?, col: self.col_at(x)? })
    }

    /// この x はどの列の上か(見出し・セルのどちらでも)。
    fn col_at(&self, x: f32) -> Option<u32> {
        let cols: Vec<(u32, f32)> = self.visible_cols()
            .into_iter()
            .map(|c| (c, self.col_px(c)))
            .collect();
        index_at(&cols, self.head_w(), x)
    }

    fn row_at(&self, y: f32) -> Option<u32> {
        let rows: Vec<(u32, f32)> = self
            .visible_rows()
            .into_iter()
            .map(|r| (r, self.row_px(r)))
            .collect();
        index_at(&rows, self.head_h(), y)
    }

    /// 列をまるごと選ぶ(使われている高さまで)。`a` が起点、`b` が動く側。
    fn select_cols(&mut self, a: u32, b: u32) {
        let rows = self.sheet().extent().0.max(1);
        self.anchor = Some(Pos::new(rows - 1, a));
        self.cursor = Pos::new(0, b);
        self.sync_input();
        let (lo, hi) = (a.min(b), a.max(b));
        self.status = if lo == hi {
            ui::tf!("{}列を選択しました(1〜{}行)", col_name(lo), rows).into()
        } else {
            ui::tf!("{}〜{}列を選択しました(1〜{}行)", col_name(lo), col_name(hi), rows).into()
        };
    }

    /// 行をまるごと選ぶ(使われている幅まで)。
    fn select_rows(&mut self, a: u32, b: u32) {
        let cols = self.sheet().extent().1.max(1);
        self.anchor = Some(Pos::new(a, cols - 1));
        self.cursor = Pos::new(b, 0);
        self.sync_input();
        let (lo, hi) = (a.min(b), a.max(b));
        self.status = if lo == hi {
            ui::tf!("{}行を選択しました", lo + 1).into()
        } else {
            ui::tf!("{}〜{}行を選択しました", lo + 1, hi + 1).into()
        };
    }

    /// 見出しの帯の上の、列幅・行高の取っ手(境界 ±GRIP px)。Some((列か, 番号))。
    /// 描画・cell_at と同じ並び(固定・窓・絞り込み)を使う —
    /// ずれると別の境界を掴んでしまう。
    fn size_grip_at(&self, x: f32, y: f32) -> Option<(bool, u32)> {
        if !self.show_headers {
            return None; // 見出しが無ければ掴む縁も無い
        }
        if y < ROW_H && x >= HEAD_W {
            let cols: Vec<(u32, f32)> = self.visible_cols()
                .into_iter()
                .map(|c| (c, self.col_px(c)))
                .collect();
            return grip_hit(&cols, HEAD_W, x).map(|c| (true, c));
        }
        if x < HEAD_W && y >= ROW_H {
            let rows: Vec<(u32, f32)> = self
                .visible_rows()
                .into_iter()
                .map(|r| (r, self.row_px(r)))
                .collect();
            return grip_hit(&rows, ROW_H, y).map(|r| (false, r));
        }
        None
    }

    /// 境界を掴んだまま動いた。列幅・行高をその場で変える(見ながら合わせる)。
    /// 最小幅で止める — ゼロにすると列が消えて掴み直せない。
    fn size_drag_at(&mut self, x: f32, y: f32) {
        if std::env::var_os("JO_MOUSE_LOG").is_some() {
            eprintln!("move x={x:.1} y={y:.1} size_drag={}", self.size_drag.is_some());
        }
        let Some(d) = &self.size_drag else { return };
        let (col, idx, grab, base, moved) = (d.col, d.idx, d.grab, d.base, d.moved);
        if !moved {
            self.checkpoint();
            if let Some(d) = &mut self.size_drag {
                d.moved = true;
            }
        }
        if col {
            let w = (base + x - grab).max(9.0) / PX_PER_CHW;
            let w = (w * 100.0).round() / 100.0;
            self.sheet_mut().col_width.insert(idx, w);
            self.status = ui::tf!("{}列の幅: {}({:.0}px)", col_name(idx), w, w * PX_PER_CHW)
            .into();
        } else {
            let pt = ((base + y - grab) / self.zoom).max(6.0) * 15.0 / 24.0;
            let pt = (pt * 100.0).round() / 100.0;
            self.sheet_mut().row_height.insert(idx, pt);
            self.status = ui::tf!("{}行の高さ: {}pt({:.0}px)", idx + 1, pt, pt * 24.0 / 15.0)
            .into();
        }
        self.dirty = true;
    }

    /// マウスの左を押した(格子領域の座標)。押したセルが選択の始まり。
    /// メニューが出ていたら閉じる(項目の上の押下は stop_propagation でここに来ない)。
    fn mouse_down_at(&mut self, x: f32, y: f32, shift: bool, ctrl: bool, clicks: usize) {
        self.menu_at = None;
        self.pick = None;
        // mouse-up を取り逃していても、新しい押下で必ず仕切り直す(自癒)
        self.size_drag = None;
        self.drag = None;
        self.head_drag = None;
        self.shape_drag = None;
        if std::env::var_os("JO_MOUSE_LOG").is_some() {
            eprintln!(
                "down x={x:.1} y={y:.1} clicks={clicks} grip={:?}",
                self.size_grip_at(x, y)
            );
        }
        // 描画の道具が出ていれば筆が最優先(セルは触らない)
        if let Some(t) = self.tool {
            if x >= self.head_w() && y >= self.head_h() {
                if t == 2 {
                    // 消しゴム: なぞった線を1筆消す
                    match self.ink_at(x, y) {
                        Some(i) => {
                            self.checkpoint();
                            self.sheet_mut().shapes_new.remove(i);
                            self.dirty = true;
                            self.status = ui::t!("1筆消しました(Ctrl+Z で戻せます)").into();
                        }
                        None => self.status = ui::t!("線の上をなぞってください").into(),
                    }
                } else {
                    self.ink_cur = Some(vec![(x, y)]);
                }
                return;
            }
        }
        // 浮いている図形が最優先(セルの上に描かれているので)
        if let Some((i, (sx, sy), corner)) = self.shape_at(x, y) {
            self.commit();
            self.checkpoint();
            self.shape_sel = Some(i);
            self.shape_drag = Some((i, (x, y), if corner { (sx, sy) } else { (sx, sy) }, corner));
            self.status = if corner {
                ui::t!("右下を引いて大きさを変えます").into()
            } else {
                ui::t!("図形を選びました(ドラッグで移動 / 右下で大きさ / Del で削除)").into()
            };
            return;
        }
        self.shape_sel = None;
        // 見出しの境界の取っ手が最優先(セルの当たり判定より先に見る)。
        // **ダブルクリックの自動調整は撤去した**(2026-08-03 発注者報告)。
        // 押し直し・掴み直しは 400ms 以内なら click_count が 2,3,… と数えられる
        // (Wayland の仕様)ので、クリック数で分岐するとやり直しのドラッグを
        // 自動調整が横取りする — ドラッグは常にドラッグでなければならない
        let _ = clicks;
        if let Some((is_col, idx)) = self.size_grip_at(x, y) {
            self.commit();
            if std::env::var_os("JO_MOUSE_LOG").is_some() {
                eprintln!("grip: col={is_col} idx={idx} x={x:.0} y={y:.0}");
            }
            self.size_drag = Some(SizeDrag {
                col: is_col,
                idx,
                grab: if is_col { x } else { y },
                base: if is_col { self.col_px(idx) } else { self.row_px(idx) },
                moved: false,
            });
            return;
        }
        // 見出しのクリック = 列・行の選択(Excel の作法)。撫でれば複数列・行
        if y < ROW_H && x >= HEAD_W {
            if let Some(c) = self.col_at(x) {
                if !self.commit() {
                    return;
                }
                if shift {
                    // いまの選択の起点の列から伸ばす
                    let a = self.anchor.map(|p| p.col).unwrap_or(self.cursor.col);
                    self.select_cols(a, c);
                } else {
                    self.select_cols(c, c);
                    self.head_drag = Some((true, c));
                }
            }
            return;
        }
        if x < HEAD_W && y >= ROW_H {
            if let Some(r) = self.row_at(y) {
                if !self.commit() {
                    return;
                }
                if shift {
                    let a = self.anchor.map(|p| p.row).unwrap_or(self.cursor.row);
                    self.select_rows(a, r);
                } else {
                    self.select_rows(r, r);
                    self.head_drag = Some((false, r));
                }
            }
            return;
        }
        // 左上の角 = 使われている範囲の全選択(Ctrl+A と同じ)
        if x < HEAD_W && y < ROW_H {
            if !self.commit() {
                return;
            }
            let (rows, cols) = self.sheet().extent();
            if rows > 0 {
                self.anchor = Some(Pos::new(0, 0));
                self.cursor = Pos::new(rows - 1, cols.saturating_sub(1));
                self.sync_input();
                self.status = ui::tf!("A1:{} を選択しました", self.cursor.a1()).into();
            }
            return;
        }
        let Some(p) = self.cell_at(x, y) else { return };
        // 関数の引数の画面が開いている間は、セルのクリックで
        // **いまの欄に参照が入る**。そのままドラッグすると範囲(A1:C9)になる
        if self.fn_args.is_some() {
            let a1 = p.a1();
            if let Some(a) = &mut self.fn_args {
                if a.eds.is_empty() {
                    return;
                }
                let i = a.focus.min(a.eds.len() - 1);
                a.eds[i] = Editor::new(&a1);
                a.eds[i].move_to(a1.len(), false);
                a.pick_from = Some(p);
            }
            self.fn_args_recalc();
            return;
        }
        // 式の直入力中は、セルのクリックで**参照がカーソルに入る**(Excel の
        // 作法)。入るのは参照を待つ場所(= ( , 演算子の直後)のときだけ —
        // それ以外の場所でのクリックは、従来どおり確定して移動
        if (self.editing() || self.edit_armed) && self.input.text().starts_with('=') {
            let t = self.input.text().to_string();
            let cur = self.input.cursor().min(t.len());
            let prev = t[..cur].trim_end().chars().last();
            if matches!(
                prev,
                Some('=' | '(' | ',' | '+' | '-' | '*' | '/' | ':' | '^' | '&' | '<' | '>' | '%')
            ) {
                let a1 = p.a1();
                self.input.insert(&a1);
                let end = self.input.cursor();
                self.ref_pick = Some((p, end - a1.len()..end));
                return;
            }
        }
        // Ctrl+クリックはリンクを開く(基幹網の外は既定のブラウザに任せる)
        if ctrl && !shift {
            if let Some(url) = self.sheet().links.get(&p).cloned() {
                let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
                self.status = ui::tf!("開きます: {}", url).into();
                return;
            }
        }
        if !self.commit() {
            // 入力規則で戻された。移動すると打った文字が黙って消えるので留まる
            return;
        }
        if shift {
            // いまのセルから伸ばす
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
        } else {
            self.anchor = None;
            self.drag = Some(p);
        }
        self.cursor = p;
        self.sync_input();
        // ダブルクリックはその場で編集(次の打鍵が追記になる — Excel の作法)
        if clicks >= 2 {
            self.edit_armed = true;
            self.input.move_to(self.input.text().len(), false);
            self.status = ui::t!("編集: そのまま打つと続きに入ります(Esc で取消)").into();
        }
    }

    /// 押したまま動いた。通り過ぎたセルまで選択を広げる。
    fn mouse_drag_at(&mut self, x: f32, y: f32) {
        // 式の直入力のセル掴み: 入れた参照を「起点:いま」の範囲に置き換える
        if let Some((from, range)) = self.ref_pick.clone() {
            let Some(p) = self.cell_at(x, y) else { return };
            let (ra, rb) = (from.row.min(p.row), from.row.max(p.row));
            let (ca, cb) = (from.col.min(p.col), from.col.max(p.col));
            let text = if from == p {
                p.a1()
            } else {
                format!("{}:{}", Pos::new(ra, ca).a1(), Pos::new(rb, cb).a1())
            };
            let mut t = self.input.text().to_string();
            if range.end <= t.len() {
                t.replace_range(range.clone(), &text);
                self.input = Editor::new(&t);
                self.input.move_to(range.start + text.len(), false);
                self.ref_pick = Some((from, range.start..range.start + text.len()));
            }
            return;
        }
        // 関数の引数のセル掴み: なぞった範囲「起点:いま」を欄に入れる
        if self.fn_args.as_ref().is_some_and(|a| a.pick_from.is_some()) {
            let Some(p) = self.cell_at(x, y) else { return };
            if let Some(a) = &mut self.fn_args {
                let Some(from) = a.pick_from else { return };
                let i = a.focus.min(a.eds.len().saturating_sub(1));
                let (ra, rb) = (from.row.min(p.row), from.row.max(p.row));
                let (ca, cb) = (from.col.min(p.col), from.col.max(p.col));
                let text = if from == p {
                    p.a1()
                } else {
                    format!("{}:{}", Pos::new(ra, ca).a1(), Pos::new(rb, cb).a1())
                };
                a.eds[i] = Editor::new(&text);
                a.eds[i].move_to(text.len(), false);
            }
            self.fn_args_recalc();
            return;
        }
        if self.tool == Some(2) {
            // 消しゴムはなぞっている間ずっと効く
            if let Some(i) = self.ink_at(x, y) {
                self.checkpoint();
                self.sheet_mut().shapes_new.remove(i);
                self.dirty = true;
            }
            return;
        }
        if let Some(pts) = &mut self.ink_cur {
            // 近すぎる点は捨てる(点の数を抑える)
            let far = pts
                .last()
                .map(|(lx, ly)| (x - lx).abs() + (y - ly).abs() > 2.0)
                .unwrap_or(true);
            if far {
                pts.push((x, y));
            }
            return;
        }
        if let Some((is_col, start)) = self.head_drag {
            // 見出しから始めた選択は、どこを通っても列・行の選択のまま
            if is_col {
                if let Some(c) = self.col_at(x) {
                    if self.cursor.col != c {
                        self.select_cols(start, c);
                    }
                }
            } else if let Some(r) = self.row_at(y) {
                if self.cursor.row != r {
                    self.select_rows(start, r);
                }
            }
            return;
        }
        let Some(start) = self.drag else { return };
        let Some(p) = self.cell_at(x, y) else { return };
        if self.cursor == p {
            return;
        }
        self.cursor = p;
        self.anchor = if p == start { None } else { Some(start) };
        if self.anchor.is_some() {
            let (a, b) = self.sel_rect();
            self.status = format!("{}:{}", a.a1(), b.a1()).into();
        }
        self.sync_input();
    }

    /// 離した。ドラッグ選択はここで確定する。
    fn mouse_up(&mut self) {
        // 関数の引数・式の直入力のセル掴みは、離した所で終わり
        if let Some(a) = &mut self.fn_args {
            a.pick_from = None;
        }
        self.ref_pick = None;
        if let Some(pts) = self.ink_cur.take() {
            self.finish_ink(pts);
            return;
        }
        if std::env::var_os("JO_MOUSE_LOG").is_some() {
            eprintln!(
                "up size_drag={} moved={:?}",
                self.size_drag.is_some(),
                self.size_drag.as_ref().map(|d| d.moved)
            );
        }
        if self.size_drag.take().is_some() {
            // 幅・高さの確定。status は size_drag_at が出している
            return;
        }
        if self.head_drag.take().is_some() {
            return; // 列・行の選択の確定。status は select_* が出している
        }
        if let Some((_, _, _, moved)) = self.shape_drag.take() {
            // 動かしていない(選んだだけ)なら、積んだ控えは戻す
            let _ = moved;
            return;
        }
        if self.drag.take().is_some() && self.anchor.is_some() {
            let (a, b) = self.sel_rect();
            self.status = format!("{}:{}", a.a1(), b.a1()).into();
        }
    }

    /// 右クリック。選択の中ならその選択への操作、外ならそのセルへ移ってから
    /// メニューを出す(Excel の作法)。
    fn right_click_at(&mut self, x: f32, y: f32) {
        // 見出しの右クリック = その列・行を選んでからメニュー(Excel の作法)。
        // 既に選択の中なら選び直さない(複数列への操作を保つ)
        if y < ROW_H && x >= HEAD_W {
            if let Some(c) = self.col_at(x) {
                let (a, b) = self.sel_rect();
                if !(self.anchor.is_some() && (a.col..=b.col).contains(&c)) {
                    if !self.commit() {
                        return;
                    }
                    self.select_cols(c, c);
                }
                self.menu_at = Some((x, y));
                self.menu_sub = None;
            }
            return;
        }
        if x < HEAD_W && y >= ROW_H {
            if let Some(r) = self.row_at(y) {
                let (a, b) = self.sel_rect();
                if !(self.anchor.is_some() && (a.row..=b.row).contains(&r)) {
                    if !self.commit() {
                        return;
                    }
                    self.select_rows(r, r);
                }
                self.menu_at = Some((x, y));
                self.menu_sub = None;
            }
            return;
        }
        if let Some(p) = self.cell_at(x, y) {
            let (a, b) = self.sel_rect();
            let inside = self.anchor.is_some()
                && (a.row..=b.row).contains(&p.row)
                && (a.col..=b.col).contains(&p.col);
            if !inside && p != self.cursor {
                if !self.commit() {
                    // 入力規則で戻された。移動せずメニューも出さない
                    return;
                }
                self.anchor = None;
                self.cursor = p;
                self.sync_input();
            }
        }
        self.menu_at = Some((x, y));
        self.menu_sub = None;
    }

    /// 範囲の見えている部分の px 矩形 (x0, y0, x1, y1)。全部画面の外なら None。
    fn range_px(&self, a: Pos, b: Pos) -> Option<(f32, f32, f32, f32)> {
        let (mut x0, mut x1) = (None, None);
        let mut x = HEAD_W;
        for c in self.visible_cols() {
            let w = self.col_px(c);
            if c >= a.col && c <= b.col {
                if x0.is_none() {
                    x0 = Some(x);
                }
                x1 = Some(x + w);
            }
            x += w;
        }
        let (mut y0, mut y1) = (None, None);
        let mut y = ROW_H;
        for r in self.visible_rows() {
            let h = self.row_px(r);
            if r >= a.row && r <= b.row {
                if y0.is_none() {
                    y0 = Some(y);
                }
                y1 = Some(y + h);
            }
            y += h;
        }
        Some((x0?, y0?, x1?, y1?))
    }

    /// いま表示されているセルの左上(格子領域の px)。画面の外なら None。
    fn cell_origin_px(&self, p: Pos) -> Option<(f32, f32)> {
        let mut x = self.head_w();
        let mut cfound = false;
        for c in self.visible_cols() {
            if c == p.col {
                cfound = true;
                break;
            }
            x += self.col_px(c);
        }
        let mut y = self.head_h();
        let mut rfound = false;
        for r in self.visible_rows() {
            if r == p.row {
                rfound = true;
                break;
            }
            y += self.row_px(r);
        }
        (cfound && rfound).then_some((x, y))
    }

    /// 形式を選択して貼り付け。mode: values / formulas / formats / transpose
    fn paste_special(&mut self, mode: &str, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) else {
            self.status = ui::t!("貼り付けるものがありません").into();
            return;
        };
        if text.is_empty() {
            return;
        }
        // アプリ内のコピーか(系のクリップボードと控えの突き合わせ)
        let internal = matches!(&self.clip, Some((_, t)) if *t == text);
        let at = self.cursor;
        let n = match mode {
            "values" => {
                self.commit();
                self.checkpoint();
                if internal {
                    let cells = self.clip_cells.clone().unwrap_or_default();
                    paste_values_cells(&mut self.book.sheets[self.active], at, &cells)
                } else {
                    let grid = tsv_grid(&text);
                    paste_values_text(&mut self.book.sheets[self.active], at, &grid)
                }
            }
            "formulas" => {
                // 式を**ずらさずそのまま**貼る(普通の貼り付けはずらす方)
                self.commit();
                self.checkpoint();
                let grid = tsv_grid(&text);
                paste_grid(&mut self.book.sheets[self.active], at, &grid, None)
            }
            "formats" => {
                if !internal {
                    self.status =
                        ui::t!("書式は他のアプリからは持って来られません(このアプリでコピーした範囲だけ)").into();
                    return;
                }
                self.commit();
                self.checkpoint();
                let cells = self.clip_cells.clone().unwrap_or_default();
                paste_formats(&mut self.book.sheets[self.active], at, &cells)
            }
            "transpose" => {
                // 行と列を入れ替えて、値を貼る(式は計算結果の値になる —
                // 転置で参照を正しく回すのは別の話なので、黙って混ぜない)
                self.commit();
                self.checkpoint();
                if internal {
                    let cells = transpose(&self.clip_cells.clone().unwrap_or_default());
                    paste_values_cells(&mut self.book.sheets[self.active], at, &cells)
                } else {
                    let grid = transpose(&tsv_grid(&text));
                    paste_values_text(&mut self.book.sheets[self.active], at, &grid)
                }
            }
            _ => return,
        };
        recalc_book(&mut self.book, self.active);
        self.dirty = true;
        self.sync_input();
        self.status = match mode {
            "values" => ui::tf!("{} セルに値だけを貼りました(書式は据え置き)", n),
            "formulas" => ui::tf!("{} セルに式をそのまま貼りました(参照はずらしていません)", n),
            "formats" => ui::tf!("{} セルに書式だけを写しました(中身は残っています)", n),
            _ => ui::tf!("{} セルを転置して貼りました(式は値になっています)", n),
        }
        .into();
    }

    fn a_paste_values(&mut self, _: &ui::PasteValues, _: &mut Window, cx: &mut Context<Self>) {
        self.paste_special("values", cx);
        cx.notify();
    }

    /// メニューの項目を実行する。
    /// いまの列で並べ替え(右クリックとリボンの昇順/降順が同じ道)
    fn sort_active(&mut self, asc: bool) {
        self.commit();
        self.checkpoint();
        let c = self.cursor.col;
        self.book.sheets[self.active].sort_by_column(c, asc, true);
        self.dirty = true;
        recalc_book(&mut self.book, self.active);
        self.status = ui::tf!("{} 列で{}に並べ替えました", Pos::new(0, c).a1().trim_end_matches('1'), if asc { "昇順" } else { "降順" })
            .into();
    }

    /// 数式バーの内容をセルへ。**入力規則(list)に合わない値は入れない**
    /// (Excel と同じ)。false を返したら呼び側は移動しないこと —
    /// 打った文字が黙って消える。Esc でセルの保存内容に戻せる。
    /// 描いた1筆(格子の px の列)を図形(折れ線)にして置く。
    /// **既にある図形の仕組みに乗せる** — xlsx へは custGeom で入り、
    /// Excel でも線に見え、消しゴムも移動も Ctrl+Z も全部そのまま効く
    fn finish_ink(&mut self, pts: Vec<(f32, f32)>) {
        if pts.len() < 2 {
            return; // 点を打っただけ(線にならない)
        }
        let (mut x0, mut y0) = (f32::MAX, f32::MAX);
        let (mut x1, mut y1) = (f32::MIN, f32::MIN);
        for (x, y) in &pts {
            x0 = x0.min(*x);
            y0 = y0.min(*y);
            x1 = x1.max(*x);
            y1 = y1.max(*y);
        }
        let (w, h) = ((x1 - x0).max(4.0), (y1 - y0).max(4.0));
        // 錨は左上の点があるセル。そこからのずらしで位置を覚える
        let at = self.cell_at(x0, y0).unwrap_or(self.view);
        let (ox, oy) = self.cell_origin_px(at).unwrap_or((self.head_w(), self.head_h()));
        let marker = self.tool == Some(1);
        self.checkpoint();
        self.sheet_mut().shapes_new.push(sheet::model::SheetShape {
            at,
            dx_px: x0 - ox,
            dy_px: y0 - oy,
            width_px: w,
            height_px: h,
            kind: if marker { "marker".into() } else { "ink".into() },
            fill: None,
            line: Some(if marker { "FFD54A".into() } else { "1B1B1B".into() }),
            points: pts
                .iter()
                .map(|(x, y)| ((x - x0) / w, (y - y0) / h))
                .collect(),
            ..Default::default()
        });
        self.dirty = true;
        self.status = if marker {
            ui::t!("蛍光ペンで引きました(Ctrl+Z で戻せます)").into()
        } else {
            ui::t!("ペンで描きました(Ctrl+Z で戻せます)").into()
        };
    }

    /// この位置にある手描きの線(いちばん上のもの)。消しゴムが使う
    fn ink_at(&self, x: f32, y: f32) -> Option<usize> {
        let sh = self.sheet();
        for (i, sp) in sh.shapes_new.iter().enumerate().rev() {
            if !matches!(sp.kind.as_str(), "ink" | "marker" | "spark") {
                continue;
            }
            let Some((ox, oy)) = self.cell_origin_px(sp.at) else { continue };
            let (x0, y0) = (ox + sp.dx_px, oy + sp.dy_px);
            let near = if sp.kind == "marker" { 7.0 } else { 4.0 };
            let hit = sp.points.iter().any(|(px_, py_)| {
                let (cx, cy) = (x0 + px_ * sp.width_px, y0 + py_ * sp.height_px);
                (cx - x).abs() <= near && (cy - y).abs() <= near
            });
            if hit {
                return Some(i);
            }
        }
        None
    }

    /// 選択範囲(見た目の値)の TSV。AI に渡す形
    fn tsv_display(&self, a: Pos, b: Pos) -> String {
        let sh = self.sheet();
        (a.row..=b.row)
            .map(|r| {
                (a.col..=b.col)
                    .map(|c| sh.get(Pos::new(r, c)).map(|x| x.value.display()).unwrap_or_default())
                    .collect::<Vec<_>>()
                    .join("\t")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// AI に頼んで、返事を表に反映する。**別の糸で待つ**(画面は止めない)。
    /// 反映は必ず checkpoint してから = **Ctrl+Z の1手で戻る**。
    /// 宛先が使えなければ理由を言う(黙って空にしない)
    fn ai_go(&mut self, job: CalcAi, cx: &mut Context<Self>) {
        if self.sheet().protected {
            self.status =
                ui::t!("シートが保護されています(保護タブの「シートを保護する」で解除)").into();
            return;
        }
        if self.ai_busy {
            self.status = ui::t!("いま考えています(終わるまでお待ちください)").into();
            return;
        }
        let back = ui::ai::backend();
        if let Err(e) = ui::ai::ready(back) {
            self.status = format!("AI: {e}").into();
            return;
        }
        self.commit();
        // 渡す範囲: 選択があればそこ。要約だけは無選択なら使っている全域
        let sel = self.anchor.map(|_| self.sel_rect());
        let (a, b) = match (&job, sel) {
            (_, Some(r)) => r,
            (CalcAi::Summary, None) => {
                let (rows, cols) = self.sheet().extent();
                if rows == 0 || cols == 0 {
                    self.status = ui::t!("表がありません").into();
                    return;
                }
                (Pos::new(0, 0), Pos::new((rows - 1).min(199), cols - 1))
            }
            (CalcAi::Table(_) | CalcAi::Ask(_), None) => (self.cursor, self.cursor),
            _ => {
                self.status = ui::t!("範囲を選んでから押してください").into();
                return;
            }
        };
        if matches!(job, CalcAi::Furigana) && a.col != b.col {
            self.status =
                ui::t!("ふりがなは1列だけ選んでください(読みは右隣の列に入ります)").into();
            return;
        }
        let body = match &job {
            CalcAi::Table(_) => String::new(),
            CalcAi::Ask(_) if self.anchor.is_none() => String::new(),
            _ => self.tsv_display(a, b),
        };
        if body.trim().is_empty()
            && !matches!(job, CalcAi::Table(_) | CalcAi::Ask(_))
        {
            self.status = ui::t!("選んだ範囲が空です").into();
            return;
        }
        let (sys, ask) = job.prompt();
        let user = match &job {
            CalcAi::Table(q) => q.clone(),
            CalcAi::Ask(q) => {
                if body.trim().is_empty() {
                    q.clone()
                } else {
                    format!("{q}\n\n---\n{body}")
                }
            }
            _ => format!("{ask}\n\n---\n{body}"),
        };
        let sys = sys.to_string();
        let job2 = job.clone();
        self.ai_busy = true;
        self.status =
            format!("AI({})に{}を頼んでいます…", back.label(), job.label()).into();
        let task = cx
            .background_executor()
            .spawn(async move { ui::ai::ask(back, &sys, &user) });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |this, cx| {
                this.ai_busy = false;
                match r {
                    Ok(out) => this.ai_apply(job2, a, b, out),
                    Err(e) => this.status = format!("AI: {e}").into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 返事を表へ入れる。**1手で戻せる**(checkpoint してから)
    fn ai_apply(&mut self, job: CalcAi, a: Pos, b: Pos, out: String) {
        let out = out.trim().to_string();
        if out.is_empty() {
            self.status = ui::t!("AI: 答えが空でした(何もしていません)").into();
            return;
        }
        let grid = |t: &str| -> Vec<Vec<String>> {
            t.lines().map(|l| l.split('\t').map(str::to_string).collect()).collect()
        };
        match job {
            // 要約はカーソルのコメントへ(保存で xlsx に残る)
            CalcAi::Summary => {
                let p = self.cursor;
                self.checkpoint();
                self.book.sheets[self.active].comments.insert(p, out);
                self.dirty = true;
                self.status = format!(
                    "要約を {} のコメントに付けました(Ctrl+Z で戻せます)",
                    p.a1()
                )
                .into();
            }
            // 書き直し・翻訳: 同じ形の TSV を受け、**文字のセルだけ**置き換える
            CalcAi::Rewrite(_, _) | CalcAi::Translate => {
                let g = grid(&out);
                let rows = (b.row - a.row + 1) as usize;
                if g.len() != rows {
                    self.status = format!(
                        "AI: 行数が合いません({} 行の答え / {rows} 行の範囲)— 何もしていません",
                        g.len()
                    )
                    .into();
                    return;
                }
                self.checkpoint();
                let mut n = 0usize;
                for (ri, row) in g.iter().enumerate() {
                    for (ci, v) in row.iter().enumerate() {
                        let p = Pos::new(a.row + ri as u32, a.col + ci as u32);
                        if p.col > b.col {
                            break;
                        }
                        let is_text = matches!(
                            self.sheet().get(p).map(|x| &x.value),
                            Some(Value::Text(_))
                        );
                        if is_text && !v.trim().is_empty() {
                            let fmt = self
                                .sheet()
                                .get(p)
                                .map(|c| c.fmt.clone())
                                .unwrap_or_default();
                            let mut cell = Cell::input(v);
                            cell.fmt = fmt;
                            self.book.sheets[self.active].set(p, cell);
                            n += 1;
                        }
                    }
                }
                recalc_book(&mut self.book, self.active);
                self.dirty = true;
                self.sync_input();
                self.status = format!(
                    "{n} 個の文字のセルを直しました(数字と式は触っていません。Ctrl+Z で1手)"
                )
                .into();
            }
            // ふりがな: 右隣の列へ(空きでなければ断る — 黙って潰さない)
            CalcAi::Furigana => {
                let yomi: Vec<&str> = out.lines().collect();
                let rows = (b.row - a.row + 1) as usize;
                if yomi.len() != rows {
                    self.status = format!(
                        "AI: 行数が合いません({} 行の答え / {rows} 行の範囲)— 何もしていません",
                        yomi.len()
                    )
                    .into();
                    return;
                }
                let dst = a.col + 1;
                let used = (a.row..=b.row).any(|r| {
                    self.sheet()
                        .get(Pos::new(r, dst))
                        .map(|c| !c.value.display().is_empty() || c.formula.is_some())
                        .unwrap_or(false)
                });
                if used {
                    self.status =
                        ui::t!("右隣の列に中身があります(空けてから — 黙って上書きしません)").into();
                    return;
                }
                self.checkpoint();
                for (i, y) in yomi.iter().enumerate() {
                    if y.trim().is_empty() {
                        continue;
                    }
                    let p = Pos::new(a.row + i as u32, dst);
                    self.book.sheets[self.active].set(p, Cell::input(y.trim()));
                }
                self.dirty = true;
                self.status =
                    ui::t!("読みを右隣の列に入れました(Ctrl+Z で戻せます)").into();
            }
            // 続き: 選択の下の空き行へ(空きでなければ断る)
            CalcAi::Continue => {
                let g = grid(&out);
                let start = b.row + 1;
                let used = g.iter().enumerate().any(|(ri, row)| {
                    row.iter().enumerate().any(|(ci, _)| {
                        self.sheet()
                            .get(Pos::new(start + ri as u32, a.col + ci as u32))
                            .map(|c| {
                                !c.value.display().is_empty() || c.formula.is_some()
                            })
                            .unwrap_or(false)
                    })
                });
                if used {
                    self.status =
                        ui::t!("下の行に中身があります(空けてから — 黙って上書きしません)").into();
                    return;
                }
                self.checkpoint();
                let n = paste_values_text(
                    &mut self.book.sheets[self.active],
                    Pos::new(start, a.col),
                    &g,
                );
                recalc_book(&mut self.book, self.active);
                self.dirty = true;
                self.status = format!(
                    "続きを {} 行足しました({n} 欄。よく確かめてください — AI の当て推量です。Ctrl+Z で1手)",
                    g.len()
                )
                .into();
            }
            // 表にする: カーソルから流し込み(空きでなければ断る)
            CalcAi::Table(_) => {
                let g = grid(&out);
                let at = self.cursor;
                let used = g.iter().enumerate().any(|(ri, row)| {
                    row.iter().enumerate().any(|(ci, _)| {
                        self.sheet()
                            .get(Pos::new(at.row + ri as u32, at.col + ci as u32))
                            .map(|c| {
                                !c.value.display().is_empty() || c.formula.is_some()
                            })
                            .unwrap_or(false)
                    })
                });
                if used {
                    self.status =
                        ui::t!("ここには中身があります(空きへカーソルを置いてから)").into();
                    return;
                }
                self.checkpoint();
                let n = paste_values_text(&mut self.book.sheets[self.active], at, &g);
                recalc_book(&mut self.book, self.active);
                self.dirty = true;
                self.status = format!(
                    "表を {} に置きました({} 行 {n} 欄。Ctrl+Z で1手)",
                    at.a1(),
                    g.len()
                )
                .into();
            }
            // 頼む: = で始まる1行は式としてカーソルへ。他はコメントへ
            CalcAi::Ask(_) => {
                let p = self.cursor;
                if out.starts_with('=') && !out.contains('\n') {
                    self.checkpoint();
                    let fmt =
                        self.sheet().get(p).map(|c| c.fmt.clone()).unwrap_or_default();
                    let mut cell = Cell::input(&out);
                    cell.fmt = fmt;
                    self.book.sheets[self.active].set(p, cell);
                    recalc_book(&mut self.book, self.active);
                    self.dirty = true;
                    self.sync_input();
                    let shown = self
                        .sheet()
                        .get(p)
                        .map(|c| c.value.display())
                        .unwrap_or_default();
                    self.status = format!(
                        "{} に式を入れました(= {shown}。式は数式バーで確かめられます。Ctrl+Z で1手)",
                        p.a1()
                    )
                    .into();
                } else {
                    self.checkpoint();
                    self.book.sheets[self.active].comments.insert(p, out);
                    self.dirty = true;
                    self.status = format!(
                        "答えを {} のコメントに付けました(Ctrl+Z で戻せます)",
                        p.a1()
                    )
                    .into();
                }
            }
        }
    }

    /// いまの計算方法で再計算する(手動なら何もしない — 「計算」で回す)
    fn recalc_if_auto(&mut self) {
        if self.auto_calc {
            recalc_book(&mut self.book, self.active);
        }
    }

    fn commit(&mut self) -> bool {
        let (cur, text) = (self.cursor, self.input.text().to_string());
        // 変わっていなければ何もしない(移動のたびに履歴が積まれるのを防ぐ)
        let now = self.sheet().get(cur).map(|c| c.editable()).unwrap_or_default();
        if now == text {
            return true;
        }
        // シートの保護。打ちかけは捨てて元に戻す(黙って通さない)
        if self.sheet().protected {
            self.sync_input();
            self.status =
                ui::t!("シートが保護されています(保護タブの「シートを保護する」で解除)").into();
            return false;
        }
        // 空にするのは常に許す(allowBlank の既定)。式は結果が変わり得るので通す
        if !text.trim().is_empty() && !text.starts_with('=') {
            if let Some(v) = self.sheet().validation_at(cur) {
                let opts = v.options(self.sheet());
                // 候補が解決できない規則(別のシートへの参照等)では堰き止めない
                if !opts.is_empty() && !opts.iter().any(|o| *o == text.trim()) {
                    self.status = format!(
                        "「{}」は入力規則に合いません(候補: {} / Esc で戻す)",
                        text.trim(),
                        opts.join(" / ")
                    )
                    .into();
                    return false;
                }
            }
        }
        self.checkpoint();
        // **書式は据え置く。** 打ち直しただけで罫線や塗りが消えるのは帳票の事故
        let fmt = self.sheet().get(cur).map(|c| c.fmt.clone()).unwrap_or_default();
        let mut cell = Cell::input(&text);
        cell.fmt = fmt;
        self.sheet_mut().set(cur, cell);
        recalc_book(&mut self.book, self.active);
        self.dirty = true;
        // 中身を変えたらコピーの破線は消す(Excel と同じ)
        self.clip_range = None;
        true
    }

    /// カーソルを動かす(動かす前に編集中の内容を確定する)。
    /// いま選んでいる長方形(左上, 右下)。
    /// 行の画面高。文書の指定(xlsx の ht、pt)に従う。既定 15pt = 24px
    fn row_px(&self, r: u32) -> f32 {
        self.sheet().row_height.get(&r).map(|pt| pt * 24.0 / 15.0).unwrap_or(ROW_H)
            * self.zoom
    }

    /// 見出しの幅・高さ(表示タブで消せる。当たり判定も同じ値を使う)
    fn head_w(&self) -> f32 {
        if self.show_headers { HEAD_W } else { 0.0 }
    }
    fn head_h(&self) -> f32 {
        if self.show_headers { ROW_H } else { 0.0 }
    }

    /// 列の画面幅。文書の指定(xlsx の width)に従う
    fn col_px(&self, c: u32) -> f32 {
        self.sheet()
            .col_width
            .get(&c)
            .copied()
            .or(self.sheet().default_col_width)
            .map(|w| w * PX_PER_CHW)
            .unwrap_or(COL_W)
            * self.zoom
    }

    /// 列の左端(見出しの右から)
    fn col_x(&self, c: u32) -> f32 {
        (0..c).map(|i| self.col_px(i)).sum()
    }

    fn sel_rect(&self) -> (Pos, Pos) {
        let a = self.anchor.unwrap_or(self.cursor);
        let c = self.cursor;
        (Pos::new(a.row.min(c.row), a.col.min(c.col)),
         Pos::new(a.row.max(c.row), a.col.max(c.col)))
    }

    /// Shift+矢印。起点を置いてから動く
    fn extend(&mut self, dr: i32, dc: i32) {
        if self.anchor.is_none() {
            self.anchor = Some(self.cursor);
        }
        if !self.commit() {
            return;
        }
        let r = (self.cursor.row as i32 + dr).max(0) as u32;
        let c = (self.cursor.col as i32 + dc).max(0) as u32;
        self.cursor = Pos::new(r.min(9999), c.min(255));
        self.follow();
        let (a, b) = self.sel_rect();
        self.status = format!("{}:{}", a.a1(), b.a1()).into();
        self.sync_input();
    }

    /// カーソルが見える位置まで窓を動かす。
    fn follow(&mut self) {
        let (nr, nc) = (self.rows_snug(), self.cols_snug());
        if self.cursor.row < self.view.row {
            self.view.row = self.cursor.row;
        }
        if self.cursor.row >= self.view.row + nr {
            self.view.row = self.cursor.row + 1 - nr;
        }
        if self.cursor.col < self.view.col {
            self.view.col = self.cursor.col;
        }
        if self.cursor.col >= self.view.col + nc {
            self.view.col = self.cursor.col + 1 - nc;
        }
    }

    fn move_cursor(&mut self, dr: i32, dc: i32) {
        // 普通の移動は選択を解く
        self.anchor = None;
        if !self.commit() {
            return; // 入力規則で戻された(status に候補が出ている)
        }
        let r = (self.cursor.row as i32 + dr).max(0) as u32;
        let c = (self.cursor.col as i32 + dc).max(0) as u32;
        self.cursor = Pos::new(r.min(9999), c.min(255));
        self.follow();
        self.sync_input();
    }

    // ---- 割り当てられた操作 ----
    fn a_backspace(&mut self, _: &ui::Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(ed) = &mut self.name_edit {
            ed.backspace();
        } else if self.fn_args.is_some() {
            self.editor().backspace();
            self.fn_args_recalc();
        } else if let Some(d) = &mut self.fn_dlg {
            d.search.backspace();
            d.sel = 0;
        } else if let Some(sv) = &mut self.solver {
            sv.focused().backspace();
        } else if let Some((_, ed)) = &mut self.prompt {
            ed.backspace();
        } else {
            self.input.backspace();
            self.dirty = true;
        }
        cx.notify();
    }
    /// 選んだ範囲の中身を消す(**書式は残す** — 帳票の枠を壊さない)。
    /// 控えを取ってから呼ぶこと。返すのは消したセルの数。
    fn clear_range(&mut self) -> usize {
        let (a, b) = self.sel_rect();
        let mut n = 0usize;
        for r in a.row..=b.row {
            for c in a.col..=b.col {
                let p = Pos::new(r, c);
                if let Some(cell) = self.sheet().get(p).cloned() {
                    self.book.sheets[self.active].set(p, Cell {
                        formula: None,
                        value: Value::Empty,
                        fmt: cell.fmt,
                    });
                    n += 1;
                }
            }
        }
        recalc_book(&mut self.book, self.active);
        self.dirty = true;
        self.sync_input();
        n
    }

    fn a_delete(&mut self, _: &ui::Delete, _: &mut Window, cx: &mut Context<Self>) {
        if self.sheet().protected {
            self.status =
                ui::t!("シートが保護されています(保護タブの「シートを保護する」で解除)").into();
            cx.notify();
            return;
        }
        if let Some(i) = self.shape_sel.take() {
            if self.sheet().shapes_new.len() > i {
                self.checkpoint();
                self.sheet_mut().shapes_new.remove(i);
                self.dirty = true;
                self.status = ui::t!("図形を削除しました(Ctrl+Z で戻せます)").into();
            }
            cx.notify();
            return;
        }
        if self.anchor.is_some() {
            // 範囲を選んでいるときの Delete は、その中身を消す(戻せる)
            self.checkpoint();
            let n = self.clear_range();
            self.status = format!("{n} セルの中身を消しました(書式は残る)").into();
        } else {
            self.input.delete();
            self.dirty = true;
        }
        cx.notify();
    }

    /// コピー。選んだ範囲(無ければいまのセル)を TSV で系のクリップボードへ。
    /// 他のアプリにはそのまま貼れる形で、アプリ内には起点を控えて式をずらせる形で。
    fn a_copy(&mut self, _: &ui::Copy, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_now(cx)
    }
    fn copy_now(&mut self, cx: &mut Context<Self>) {
        if self.input.has_selection() {
            // 数式バーの文字を選んでいるなら、その文字のコピー
            let sel = self.input.selection();
            if let Some(s) = self.input.text().get(sel) {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(s.to_string()));
                self.status = ui::t!("コピーしました").into();
            }
            cx.notify();
            return;
        }
        let (a, b) = self.sel_rect();
        let tsv = range_tsv(self.sheet(), a, b);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(tsv.clone()));
        self.clip = Some((a, tsv));
        // セルそのものも控える(形式を選択して貼り付けの材料)
        self.clip_cells = Some(
            (a.row..=b.row)
                .map(|r| {
                    (a.col..=b.col)
                        .map(|c| self.sheet().get(Pos::new(r, c)).cloned())
                        .collect()
                })
                .collect(),
        );
        self.clip_range = Some((self.active, a, b));
        self.status = format!("{}:{} をコピーしました", a.a1(), b.a1()).into();
        cx.notify();
    }

    /// 切り取り = コピー + 中身を消す(書式は残る。1手で戻せる)。
    fn a_cut(&mut self, _: &ui::Cut, _: &mut Window, cx: &mut Context<Self>) {
        self.cut_now(cx)
    }
    fn cut_now(&mut self, cx: &mut Context<Self>) {
        if self.input.has_selection() {
            let sel = self.input.selection();
            if let Some(s) = self.input.text().get(sel) {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(s.to_string()));
                self.input.insert("");
                self.dirty = true;
                self.status = ui::t!("切り取りました").into();
            }
            cx.notify();
            return;
        }
        let (a, b) = self.sel_rect();
        let tsv = range_tsv(self.sheet(), a, b);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(tsv.clone()));
        // 切り取りの貼り付け先で式をずらさない(移動なので参照はそのまま)。
        // 形式を選択して貼り付けも切り取りでは使えない(Excel と同じ)
        self.clip = None;
        self.clip_cells = None;
        self.clip_range = None;
        self.checkpoint();
        let n = self.clear_range();
        self.status = format!("{n} セルを切り取りました").into();
        cx.notify();
    }

    /// 貼り付け。編集中なら文字として、そうでなければセルの格子として。
    fn a_paste(&mut self, _: &ui::Paste, _: &mut Window, cx: &mut Context<Self>) {
        self.paste_now(cx)
    }
    fn paste_now(&mut self, cx: &mut Context<Self>) {
        if self.sheet().protected {
            self.status =
                ui::t!("シートが保護されています(保護タブの「シートを保護する」で解除)").into();
            cx.notify();
            return;
        }
        let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) else {
            self.status = ui::t!("貼り付けるものがありません").into();
            cx.notify();
            return;
        };
        if text.is_empty() {
            cx.notify();
            return;
        }
        if self.editing() {
            // 打ちかけの間は文字の貼り付け(書きかけの式に継ぎ足す使い方)
            self.input.insert(&text);
            self.dirty = true;
            cx.notify();
            return;
        }
        // アプリ内のコピーなら、式の相対参照を貼り付け先へずらす
        let shift = match &self.clip {
            Some((org, tsv)) if *tsv == text => Some((
                self.cursor.row as i64 - org.row as i64,
                self.cursor.col as i64 - org.col as i64,
            )),
            _ => None,
        };
        let grid = tsv_grid(&text);
        self.checkpoint();
        let at = self.cursor;
        let n = paste_grid(&mut self.book.sheets[self.active], at, &grid, shift);
        recalc_book(&mut self.book, self.active);
        self.dirty = true;
        self.sync_input();
        self.status = format!("{n} セルを貼り付けました(書式は据え置き)").into();
        cx.notify();
    }
    /// 数式バーを打ちかけか(バーの中身がセルの保存内容から変わっているか)。
    /// バーには選んだセルの中身が常に写っているので、**空かどうかでは分からない**
    /// — 中身のあるセルで矢印が「見えない文字カーソル」に化け、
    /// セルから出られなくなる(踏んで直した)。
    fn editing(&self) -> bool {
        let saved = self.sheet().get(self.cursor).map(|c| c.editable()).unwrap_or_default();
        self.input.text() != saved
    }

    fn a_left(&mut self, _: &ui::Left, _: &mut Window, cx: &mut Context<Self>) {
        // 小窓 → 板 → 打ちかけの文字 → セル、の順で見る
        if let Some(ed) = &mut self.name_edit { ed.move_char(false, false) }
        else if self.fn_args.is_some() { self.editor().move_char(false, false) }
        else if let Some(d) = &mut self.fn_dlg { d.search.move_char(false, false) }
        else if let Some(sv) = &mut self.solver { sv.focused().move_char(false, false) }
        else if let Some((_, ed)) = &mut self.prompt { ed.move_char(false, false) }
        else if self.editing() || self.edit_armed { self.input.move_char(false, false) }
        else { self.move_cursor(0, -1) }
        cx.notify();
    }
    fn a_right(&mut self, _: &ui::Right, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(ed) = &mut self.name_edit { ed.move_char(true, false) }
        else if self.fn_args.is_some() { self.editor().move_char(true, false) }
        else if let Some(d) = &mut self.fn_dlg { d.search.move_char(true, false) }
        else if let Some(sv) = &mut self.solver { sv.focused().move_char(true, false) }
        else if let Some((_, ed)) = &mut self.prompt { ed.move_char(true, false) }
        else if self.editing() || self.edit_armed { self.input.move_char(true, false) }
        else { self.move_cursor(0, 1) }
        cx.notify();
    }
    fn a_doc_home(&mut self, _: &ui::DocHome, _: &mut Window, cx: &mut Context<Self>) {
        // Ctrl+Home は A1 へ(表計算の作法)
        self.anchor = None;
        if !self.commit() {
            cx.notify();
            return;
        }
        self.cursor = Pos::new(0, 0);
        self.follow();
        self.sync_input();
        cx.notify();
    }
    fn a_doc_end(&mut self, _: &ui::DocEnd, _: &mut Window, cx: &mut Context<Self>) {
        // Ctrl+End は使われている範囲の右下へ
        self.anchor = None;
        if !self.commit() {
            cx.notify();
            return;
        }
        let (rows, cols) = self.sheet().extent();
        if rows > 0 {
            self.cursor = Pos::new(rows - 1, cols.saturating_sub(1));
        }
        self.follow();
        self.sync_input();
        cx.notify();
    }
    fn a_page_up(&mut self, _: &ui::PageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_cursor(-(self.rows_snug() as i32 - 1).max(1), 0);
        cx.notify();
    }
    fn a_page_down(&mut self, _: &ui::PageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_cursor((self.rows_snug() as i32 - 1).max(1), 0);
        cx.notify();
    }
    fn a_up(&mut self, _: &ui::Up, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(a) = &mut self.fn_args {
            a.focus = a.focus.saturating_sub(1);
        } else if let Some(d) = &mut self.fn_dlg {
            d.sel = d.sel.saturating_sub(1);
        } else {
            self.move_cursor(-1, 0);
        }
        cx.notify();
    }
    fn a_down(&mut self, _: &ui::Down, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(a) = &mut self.fn_args {
            a.focus = (a.focus + 1).min(a.eds.len().saturating_sub(1));
        } else if let Some(d) = &mut self.fn_dlg {
            let n = fn_filtered(d.search.text(), d.group).len();
            d.sel = (d.sel + 1).min(n.saturating_sub(1));
        } else {
            self.move_cursor(1, 0);
        }
        cx.notify();
    }
    fn a_tab(&mut self, _: &ui::Tab, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(a) = &mut self.fn_args {
            if !a.eds.is_empty() {
                a.focus = (a.focus + 1) % a.eds.len();
            }
        } else {
            self.move_cursor(0, 1);
        }
        cx.notify();
    }
    fn a_enter(&mut self, _: &ui::Enter, _: &mut Window, cx: &mut Context<Self>) {
        if self.quit_ask {
            // Enter = 保存して終了(いちばん安全な既定)
            self.quit_ask = false;
            self.save(true, cx);
            cx.notify();
            return;
        }
        if self.name_edit.is_some() {
            self.commit_name_box();
            cx.notify();
            return;
        }
        if self.fn_args.is_some() {
            self.fn_args_ok();
            cx.notify();
            return;
        }
        if self.fn_dlg.is_some() {
            self.fn_next();
            cx.notify();
            return;
        }
        if self.solver.is_some() {
            // 小窓の Enter では何も走らせない(解くのは「解を求める」の釦)
            cx.notify();
            return;
        }
        if self.prompt.is_some() {
            self.finish_prompt(cx);
        } else if let Some(i) = self.shape_sel {
            // 図形を選んで Enter = 中の文字を書く(テキストボックス)
            let cur = self
                .sheet()
                .shapes_new
                .get(i)
                .and_then(|sp| sp.text.clone())
                .unwrap_or_default();
            self.prompt = Some(("shape-text", Editor::new(&cur)));
        } else {
            self.move_cursor(1, 0);
        }
        cx.notify();
    }
    fn a_select_left(&mut self, _: &ui::SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.editing() { self.input.move_char(false, true) }
        else { self.extend(0, -1) }
        cx.notify();
    }
    fn a_select_right(&mut self, _: &ui::SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        if self.editing() { self.input.move_char(true, true) }
        else { self.extend(0, 1) }
        cx.notify();
    }
    fn a_select_up(&mut self, _: &ui::SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.extend(-1, 0); cx.notify();
    }
    fn a_select_down(&mut self, _: &ui::SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.extend(1, 0); cx.notify();
    }
    fn a_select_all(&mut self, _: &ui::SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.select_all_now();
        cx.notify();
    }
    /// 全選択の実体。Ctrl+A ともリボンの「すべて選択」とも同じ道を通す
    /// (リボンだけバーの文字選択、という別物にしない)
    fn select_all_now(&mut self) {
        if self.editing() {
            // 打ちかけの間は、バーの文字の全選択
            self.input.select_all();
        } else {
            // 使われている範囲の全選択(表計算の Ctrl+A)
            let (rows, cols) = self.sheet().extent();
            if rows == 0 {
                self.status = ui::t!("空の表です").into();
            } else {
                self.commit();
                self.anchor = Some(Pos::new(0, 0));
                self.cursor = Pos::new(rows - 1, cols.saturating_sub(1));
                self.status = format!("A1:{} を選択しました", self.cursor.a1()).into();
                self.sync_input();
            }
        }
    }
    fn a_undo(&mut self, _: &ui::Undo, _: &mut Window, cx: &mut Context<Self>) {
        if !self.input.undo() {
            self.undo_sheet();
        }
        cx.notify();
    }
    fn a_redo(&mut self, _: &ui::Redo, _: &mut Window, cx: &mut Context<Self>) {
        if !self.input.redo() {
            self.redo_sheet();
        }
        cx.notify();
    }
    /// リボンのコマンド。数式タブは選択セルに関数を入れる。
    /// 選んでいるセルの見た目を変える。
    ///
    /// **値の無いセルにも掛ける** — 罫線だけを引くのは帳票では普通の操作。
    fn fmt(&mut self, f: impl Fn(&mut CellFormat)) {
        if self.sheet().protected {
            self.status =
                ui::t!("シートが保護されています(保護タブの「シートを保護する」で解除)").into();
            return;
        }
        self.commit();
        self.checkpoint();
        // 範囲選択があれば全部に掛ける。罫線も塗りも、帳票は範囲でやる仕事
        let (a, b) = self.sel_rect();
        for r in a.row..=b.row {
            for cidx in a.col..=b.col {
                let p = Pos::new(r, cidx);
                let mut c = self.sheet().get(p).cloned().unwrap_or_default();
                f(&mut c.fmt);
                self.book.sheets[self.active].set(p, c);
            }
        }
        self.dirty = true;
        recalc_book(&mut self.book, self.active);
    }

    /// 選んだ範囲を結合する。**値は消さない** — 左上以外の値は隠れるだけで、
    /// 結合を解けば戻る(黙って捨てない)。
    fn merge_selection(&mut self) {
        self.checkpoint();
        let (a, b) = self.sel_rect();
        if a == b {
            self.status = ui::t!("結合する範囲を Shift+矢印で選んでください").into();
            return;
        }
        let sh = &mut self.book.sheets[self.active];
        // 同じ範囲がもう結合されていたら解く(押すたびに入切)
        if let Some(i) = sh.merges.iter().position(|m| *m == (a, b)) {
            sh.merges.remove(i);
            self.status = format!("{}:{} の結合を解きました", a.a1(), b.a1()).into();
        } else {
            sh.merges.retain(|(x, y)| {
                // 重なる結合は先に外す(入れ子の結合は帳票を壊す)
                y.row < a.row || x.row > b.row || y.col < a.col || x.col > b.col
            });
            sh.merges.push((a, b));
            self.status = format!("{}:{} を結合しました", a.a1(), b.a1()).into();
        }
        self.dirty = true;
    }

    /// 行・列を出し入れする。
    fn rowcol(&mut self, f: impl Fn(&mut sheet::Sheet, Pos)) {
        self.commit();
        self.checkpoint();
        let p = self.cursor;
        f(&mut self.book.sheets[self.active], p);
        self.dirty = true;
        recalc_book(&mut self.book, self.active);
    }

    /// 小数点以下の桁を増減する。
    ///
    /// **0〜10 に留める。** 際限なく増やせると、桁だけの帳票が出来上がる。
    fn decimals(&mut self, d: i32) {
        self.fmt(move |f| {
            let now = f
                .number_format
                .as_deref()
                .and_then(|s| s.rsplit_once('.'))
                .map(|(_, dec)| dec.chars().take_while(|c| *c == '0').count() as i32)
                .unwrap_or(0);
            let n = (now + d).clamp(0, 10);
            let comma = f.number_format.as_deref().is_some_and(|s| s.contains(','));
            let head = if comma { "#,##0" } else { "0" };
            f.number_format = Some(if n == 0 {
                head.to_string()
            } else {
                format!("{head}.{}", "0".repeat(n as usize))
            });
        });
    }

    /// この格子座標に**このアプリで挿した図形**があるか(上に描かれた順 = 後勝ち)。
    /// 返すのは (番号, 図形の左上px, 右下隅の掴みか)。
    fn shape_at(&self, x: f32, y: f32) -> Option<(usize, (f32, f32), bool)> {
        for (i, sp) in self.sheet().shapes_new.iter().enumerate().rev() {
            let Some((sx, sy)) = self.cell_origin_px(sp.at) else { continue };
            let (sx, sy) = (sx + sp.dx_px, sy + sp.dy_px);
            let (w, h) = (sp.width_px, sp.height_px);
            if x >= sx && x <= sx + w && y >= sy && y <= sy + h {
                let corner = x >= sx + w - 12.0 && y >= sy + h - 12.0;
                return Some((i, (sx, sy), corner));
            }
        }
        None
    }

    /// 図形のドラッグ(移動 or 右下の掴みで大きさ変更)。
    fn shape_drag_at(&mut self, x: f32, y: f32) {
        let Some((i, (gx, gy), (ox, oy), resize)) = self.shape_drag else { return };
        if self.sheet().shapes_new.len() <= i {
            return;
        }
        if resize {
            let sp = &mut self.sheet_mut().shapes_new[i];
            sp.width_px = (x - ox).max(16.0);
            sp.height_px = (y - oy).max(16.0);
            let (w, h) = (sp.width_px, sp.height_px);
            self.dirty = true;
            self.status = format!("大きさ: {w:.0}×{h:.0}px").into();
        } else {
            // 移動: 掴んだときのずれを保って、左上の来るセルに留め直す。
            // セルからのはみ出しは px のずらしとして持つ(位置が飛ばない)
            let (nx, ny) = (ox + x - gx, oy + y - gy);
            if let (Some(c), Some(r)) = (self.col_at(nx.max(HEAD_W)), self.row_at(ny.max(ROW_H))) {
                let at = Pos::new(r, c);
                if let Some((cx0, cy0)) = self.cell_origin_px(at) {
                    let (dx, dy) = ((nx - cx0).max(0.0), (ny - cy0).max(0.0));
                    let sp = &mut self.sheet_mut().shapes_new[i];
                    if sp.at != at || (sp.dx_px - dx).abs() > 0.5 || (sp.dy_px - dy).abs() > 0.5 {
                        sp.at = at;
                        sp.dx_px = dx;
                        sp.dy_px = dy;
                        self.dirty = true;
                        self.status = format!("図形を {} に留めました", at.a1()).into();
                    }
                }
            }
        }
    }

    /// 「次を検索」。いまのセルの次(行→列の順)から探し、末尾まで行ったら
    /// 頭に戻る。式の中の文字も探す(editable = 打った通りの姿)。
    fn find_next(&mut self, term: &str) {
        let hits: Vec<Pos> = self
            .sheet()
            .cells
            .iter()
            .filter(|(_, c)| c.editable().contains(term) || c.value.display().contains(term))
            .map(|(p, _)| *p)
            .collect();
        if hits.is_empty() {
            self.status = format!("「{term}」は見つかりません").into();
            return;
        }
        let cur = self.cursor;
        let next = hits.iter().find(|p| **p > cur).copied().unwrap_or(hits[0]);
        self.anchor = None;
        self.cursor = next;
        self.follow();
        self.sync_input();
        self.status = format!(
            "「{term}」: {}({} カ所)。もう一度「置き換え」で次へ",
            next.a1(),
            hits.len()
        )
        .into();
        // 次回の板の初期値に残す(続けて探すのが検索の常)
        self.find_term = Some(term.to_string());
    }

    /// 絞り込みに一致する行(見出し行 0 は常に入れる)。
    fn matching_rows(&self, col: u32, v: &str) -> Vec<u32> {
        let (rows, _) = self.sheet().extent();
        let mut out = vec![0];
        for r in 1..rows {
            if self.sheet().get(Pos::new(r, col)).map(|c| c.value.display()).as_deref() == Some(v) {
                out.push(r);
            }
        }
        out
    }

    /// run_cmd が処理できる id。**リボンの ready はこの表の中に限る**
    /// (試験で突き合わせる。合っていない釦は「押せるのに何もしない」嘘になる)
    #[allow(dead_code)] // wiring_tests(cfg(test))が使う
    const HANDLED: &'static [&'static str] = &[
        "open", "save", "undo", "redo", "selectall", "pdf",
        "copy", "cut", "paste",
        "bold", "italic", "underline", "borders", "fillparag", "fontcolor",
        "align-left", "align-center", "align-right",
        "comma", "currency", "percents", "digit-inc", "digit-dec", "clear",
        "strikeout", "top", "middle", "bottom", "wrap", "incfont", "decfont",
        "cell-ins", "cell-del", "insrow", "inscol",
        "merge", "custom-sort", "sort-asc", "sort-desc",
        "rem-duplicates", "setfilter", "clear-filter",
        "fill-num", "freeze", "show-formulas", "show-gridlines",
        "fn-math", "fn-text", "fn-logical", "fn-recent",
        "sum", "average", "count", "max", "min",
        "data-validation", "condformat", "defname",
        "pageorient", "pagesize", "pagemargins", "printarea",
        "inschart", "insimage", "inshyperlink", "replace",
        "changecase", "format", "cell-format", "fontname", "fontsize",
        "fn-datetime", "fn-lookup", "fn-financial", "fn-more",
        "scale", "pagebreak", "printtitles", "print-gridlines", "print-headings",
        "data-from-text", "text-column", "goal-seek", "data-external-links",
        "insshape", "instext", "inssparkline", "python", "addcomment",
        "trace-prec", "trace-dep", "remove-arrows", "insrecommend",
        "instable", "table-tpl", "inssymbol", "pivot-insert",
        "pivot-refresh", "pivot-refresh-all", "pivot-select",
        "pivot-totals", "pivot-subtotals", "pivot-blank", "pivot-layout",
        "td-header", "td-total", "td-band-row", "td-band-col",
        "td-first", "td-last", "td-filter",
        "group", "ungroup", "hide-details", "show-details", "subtotal", "solver",
        "inssmartart", "insequation", "insslicer", "inscheckbox", "instextart",
        "coauth-mode", "co-delcomment", "co-showcomment", "co-chat",
        "co-history", "plug-macros", "plug-manage",
        "prot-doc", "prot-encrypt", "prot-sign",
        "zoom-in", "zoom-out", "formula-bar", "show-headings", "show-zeros",
        "subscript", "align-just", "text-orient", "calc-mode",
        "td-torange", "td-resize", "rtl-sheet", "direction",
        "colorschemas", "theme",
        "ai-where", "ai-summary", "ai-rewrite", "ai-polite", "ai-plain",
        "ai-translate", "ai-furigana", "ai-continue", "ai-table", "ai-ask",
        "insert-function", "cell-styles", "sheet-view", "watch",
        "pen", "highlighter", "eraser", "draw-select",
    ];

    /// シートの保護中でも通す操作(見るだけ・保存・保護の操作そのもの)
    const PROTECTED_OK: &'static [&'static str] = &[
        "open", "save", "pdf", "selectall", "undo", "redo",
        "freeze", "show-formulas", "show-gridlines",
        "setfilter", "clear-filter",
        "trace-prec", "trace-dep", "remove-arrows", "pivot-select",
        "coauth-mode", "co-showcomment", "co-chat", "co-history", "plug-manage",
        "prot-doc", "prot-encrypt", "prot-sign", "ai-where",
    ];

    fn run_cmd(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.sheet().protected && !Self::PROTECTED_OK.contains(&id) {
            self.status =
                ui::t!("シートが保護されています(保護タブの「シートを保護する」で解除)").into();
            cx.notify();
            return;
        }
        match id {
            "open" => self.open_dialog(cx),
            "save" => self.save(false, cx),
            "undo" => {
                if !self.input.undo() {
                    self.undo_sheet();
                }
            }
            "redo" => {
                if !self.input.redo() {
                    self.redo_sheet();
                }
            }
            "selectall" => self.select_all_now(),
            "copy" => self.copy_now(cx),
            "cut" => self.cut_now(cx),
            "paste" => self.paste_now(cx),
            // 罫線 — **日本の帳票の本体**
            "borders" => self.fmt(|f| {
                f.borders = if f.borders.any() { Borders::NONE } else { Borders::ALL }
            }),
            "bold" => self.fmt(|f| f.bold = !f.bold),
            "italic" => self.fmt(|f| f.italic = !f.italic),
            "underline" => self.fmt(|f| f.underline = !f.underline),
            "strikeout" => self.fmt(|f| f.strike = !f.strike),
            // 縦の揃えと折り返し
            "top" => self.fmt(|f| f.valign = sheet::model::VAlign::Top),
            "middle" => self.fmt(|f| f.valign = sheet::model::VAlign::Middle),
            "bottom" => self.fmt(|f| f.valign = sheet::model::VAlign::Bottom),
            "wrap" => self.fmt(|f| f.wrap = !f.wrap),
            // 文字の大きさ(4〜72pt)
            "incfont" => self.fmt(|f| {
                let pt = f.size_c.map(|c| c as f32 / 100.0).unwrap_or(11.0);
                f.size_c = Some((((pt + 1.0).min(72.0)) * 100.0) as u32);
            }),
            "decfont" => self.fmt(|f| {
                let pt = f.size_c.map(|c| c as f32 / 100.0).unwrap_or(11.0);
                f.size_c = Some((((pt - 1.0).max(4.0)) * 100.0) as u32);
            }),
            "align-left" => self.fmt(|f| f.align = HAlign::Left),
            "align-center" => self.fmt(|f| f.align = HAlign::Center),
            "align-right" => self.fmt(|f| f.align = HAlign::Right),
            // 表示形式
            "comma" => self.fmt(|f| f.number_format = Some("#,##0".into())),
            // 行・列の出し入れ
            "cell-ins" => self.rowcol(|s, p| s.insert_row(p.row)),
            "cell-del" => self.rowcol(|s, p| s.remove_row(p.row)),
            "insrow" => self.rowcol(|s, p| s.insert_row(p.row)),
            "inscol" => self.rowcol(|s, p| s.insert_col(p.col)),
            // 小数点以下の桁
            "digit-inc" => self.decimals(1),
            "digit-dec" => self.decimals(-1),
            // 書式のクリア。値は消さない
            "clear" => self.fmt(|f| *f = CellFormat::default()),
            // フィル(下方向へコピー)。式は相対参照がずれ、$ は止まる。
            // 書式も一緒に写す(帳票の列は書式ごと揃える)
            "fill-num" => {
                let (a, b) = self.sel_rect();
                if a.row == b.row {
                    self.status = ui::t!("Shift+↓ で埋める範囲を選んでください(先頭行を下へ写します)").into();
                } else {
                    self.commit();
                    self.checkpoint();
                    let sh = &mut self.book.sheets[self.active];
                    let mut n = 0usize;
                    for c in a.col..=b.col {
                        let Some(src) = sh.get(Pos::new(a.row, c)).cloned() else { continue };
                        for r in a.row + 1..=b.row {
                            let mut cell = src.clone();
                            if let Some(f) = &src.formula {
                                cell.formula =
                                    Some(sheet::model::offset_refs(f, (r - a.row) as i64, 0));
                            }
                            sh.set(Pos::new(r, c), cell);
                            n += 1;
                        }
                    }
                    recalc_book(&mut self.book, self.active);
                    self.dirty = true;
                    self.status = format!("{n} セルを埋めました").into();
                }
            }
            // 塗りつぶし。黄 → 水色 → 解除(色を選ぶ小窓がまだ無い)
            "merge" => self.merge_selection(),
            // 表示。**値は変えない** — 見え方だけの話
            "show-formulas" => self.show_formulas = !self.show_formulas,
            // 帳票を PDF に。画面に見えているもの(値・書式・罫線)を写す
            "pdf" => self.save_pdf(cx),
            "show-gridlines" => self.gridlines = !self.gridlines,
            // ウィンドウ枠の固定。カーソルの上と左を留める。もう一度で解く
            // 選んだセルの値で絞る。もう一度で解く。**中身は変えない**
            "setfilter" => {
                let p = self.cursor;
                let v = self.sheet().get(p)
                    .map(|c| c.value.display())
                    .unwrap_or_default();
                if v.is_empty() {
                    self.status = ui::t!("空のセルでは絞れません").into();
                } else {
                    let n = self.matching_rows(p.col, &v).len();
                    self.status = format!(
                        "{}列を「{v}」で絞り込み中({n}行が一致)。表示だけで中身は変わりません",
                        Pos::new(0, p.col).a1().trim_end_matches('1')
                    ).into();
                    self.filter = Some((p.col, v));
                }
            }
            "clear-filter" => {
                self.filter = None;
                self.status = ui::t!("絞り込みを解きました").into();
            }
            // 印刷の設定。モデルに置き、保存で原文へ織り込み、PDF が従う。
            // どれもシートの控えで1手戻せる
            "pageorient" => {
                self.commit();
                self.checkpoint();
                let sh = self.sheet_mut();
                sh.landscape = !sh.landscape;
                let landscape = sh.landscape;
                self.dirty = true;
                self.status = format!(
                    "印刷の向き: {}(PDF と保存に効きます)",
                    if landscape { "横" } else { "縦" }
                )
                .into();
            }
            "pagesize" => {
                self.commit();
                self.checkpoint();
                // A4 → A3 → B4 → B5 → A5 → A4 の順で回す
                const CYCLE: [u32; 5] = [9, 8, 12, 13, 11];
                let sh = self.sheet_mut();
                let now = sh.paper_size.unwrap_or(9);
                let i = CYCLE.iter().position(|c| *c == now).unwrap_or(0);
                let next = CYCLE[(i + 1) % CYCLE.len()];
                sh.paper_size = Some(next);
                self.dirty = true;
                let name = paper_mm(next).map(|(_, _, n)| n).unwrap_or("A4");
                self.status = format!("用紙: {name}(B は JIS)").into();
            }
            "pagemargins" => {
                self.commit();
                self.checkpoint();
                // 既定(20mm)→ 狭い(10mm)→ 広い(30mm)→ 既定
                let sh = self.sheet_mut();
                let (next, label) = match sh.margins_mm {
                    None => (Some((10.0, 10.0, 10.0, 10.0)), "狭い(10mm)"),
                    Some((l, _, _, _)) if l < 15.0 => {
                        (Some((30.0, 30.0, 30.0, 30.0)), "広い(30mm)")
                    }
                    Some(_) => (None, "既定(20mm)"),
                };
                sh.margins_mm = next;
                self.dirty = true;
                self.status = format!("印刷の余白: {label}").into();
            }
            "printarea" => {
                self.commit();
                if self.anchor.is_some() {
                    self.checkpoint();
                    let range = self.sel_rect();
                    self.sheet_mut().print_areas = vec![range];
                    self.dirty = true;
                    self.status = format!(
                        "印刷範囲: {}:{}(もう一度押すと解除)",
                        range.0.a1(),
                        range.1.a1()
                    )
                    .into();
                } else if !self.sheet().print_areas.is_empty() {
                    self.checkpoint();
                    self.sheet_mut().print_areas.clear();
                    self.dirty = true;
                    self.status = ui::t!("印刷範囲を解きました(全域を印刷します)").into();
                } else {
                    self.status =
                        ui::t!("印刷範囲にする範囲を Shift+矢印かドラッグで選んでください").into();
                }
            }
            // 大文字小文字。選択の英字に小文字があれば大文字へ、無ければ小文字へ
            "changecase" => {
                self.commit();
                self.checkpoint();
                let (a, b) = self.sel_rect();
                let mut has_lower = false;
                for r in a.row..=b.row {
                    for c in a.col..=b.col {
                        if let Some(cell) = self.sheet().get(Pos::new(r, c)) {
                            if let sheet::Value::Text(t) = &cell.value {
                                if t.chars().any(|ch| ch.is_ascii_lowercase()) {
                                    has_lower = true;
                                }
                            }
                        }
                    }
                }
                let mut n = 0usize;
                for r in a.row..=b.row {
                    for c in a.col..=b.col {
                        let p = Pos::new(r, c);
                        let Some(cell) = self.sheet().get(p).cloned() else { continue };
                        let sheet::Value::Text(t) = &cell.value else { continue };
                        if !t.chars().any(|ch| ch.is_ascii_alphabetic()) {
                            continue;
                        }
                        let new_t = if has_lower { t.to_uppercase() } else { t.to_lowercase() };
                        if new_t != *t {
                            let mut cell = cell;
                            cell.value = sheet::Value::Text(new_t);
                            self.sheet_mut().set(p, cell);
                            n += 1;
                        }
                    }
                }
                if n == 0 {
                    self.undo_stack.pop();
                    self.status = ui::t!("選択の中に英字がありません").into();
                } else {
                    self.dirty = true;
                    self.sync_input();
                    self.status = format!(
                        "{n} セルを{}にしました(もう一度で逆)",
                        if has_lower { "大文字" } else { "小文字" }
                    )
                    .into();
                }
            }
            // 数値の書式・セルのスタイル: 書式の小窓(道具箱)を開く
            "format" | "cell-format" => {
                let at = self
                    .cell_origin_px(self.cursor)
                    .map(|(x, y)| (x + 16.0, y + 16.0))
                    .unwrap_or((HEAD_W + 24.0, ROW_H + 24.0));
                self.fmt_panel = Some(at);
            }
            // 書体と大きさ: 一覧から選ぶ(日本語が組める書体だけ出す)
            "fontname" => {
                let vals: Vec<String> = kumihan::font::list()
                    .iter()
                    .filter(|f| f.japanese)
                    .map(|f| f.name.clone())
                    .collect();
                if vals.is_empty() {
                    self.status = ui::t!("日本語の書体が見つかりません").into();
                } else {
                    let at = self
                        .cell_origin_px(self.cursor)
                        .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                        .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                    // 全部出す(前は16個で黙って切り捨てていた — 一覧は
                    // スクロールできるので削る理由が無い)
                    self.pick_kind = "font";
                    self.pick = Some((vals, at));
                }
            }
            "fontsize" => {
                let at = self
                    .cell_origin_px(self.cursor)
                    .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                    .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                self.pick_kind = "size";
                self.pick = Some((
                    // Excel の標準の並び(6〜72)
                    ["6", "8", "9", "10", "11", "12", "14", "16", "18", "20",
                     "22", "24", "26", "28", "36", "48", "72"]
                        .iter()
                        .map(|v| v.to_string())
                        .collect(),
                    at,
                ));
            }
            // データタブ: Python 裏方と道具
            "data-from-text" => {
                self.commit();
                self.import_text_dialog(cx);
            }
            "python" => {
                self.commit();
                self.prompt = Some(("py", Editor::new("")));
            }
            // 参照のトレース。矢印の代わりに**セルを光らせる**(見え方だけ)
            "trace-prec" => {
                self.commit();
                let deps = self
                    .sheet()
                    .get(self.cursor)
                    .and_then(|c| c.formula.as_ref())
                    .map(|f| sheet::calc::deps(f))
                    .unwrap_or_default();
                if deps.is_empty() {
                    self.status = ui::t!("このセルの式は他のセルを参照していません").into();
                } else {
                    self.status = format!(
                        "{} の参照元 {} セルを光らせました(トレース矢印の削除で消す)",
                        self.cursor.a1(),
                        deps.len()
                    )
                    .into();
                    self.trace = deps.into_iter().map(|p| (p, true)).collect();
                }
            }
            "trace-dep" => {
                self.commit();
                let me = self.cursor;
                let dependents: Vec<Pos> = self
                    .sheet()
                    .cells
                    .iter()
                    .filter(|(_, c)| {
                        c.formula
                            .as_ref()
                            .is_some_and(|f| sheet::calc::deps(f).contains(&me))
                    })
                    .map(|(p, _)| *p)
                    .collect();
                if dependents.is_empty() {
                    self.status = format!("{} を参照している式はありません", me.a1()).into();
                } else {
                    self.status = format!(
                        "{} の参照先 {} セルを光らせました(トレース矢印の削除で消す)",
                        me.a1(),
                        dependents.len()
                    )
                    .into();
                    self.trace = dependents.into_iter().map(|p| (p, false)).collect();
                }
            }
            "remove-arrows" => {
                self.trace.clear();
                self.status = ui::t!("トレースを消しました").into();
            }
            // 推奨チャート = いまの一手(棒グラフ)をそのまま勧める
            "insrecommend" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status =
                        ui::t!("グラフにする範囲を選んでください(1列目が項目名、2列目からが数)").into();
                } else {
                    let (a, b) = self.sel_rect();
                    self.insert_chart(a, b, cx);
                }
            }
            // ピボットテーブル = polars が裏方。結果は「その時の値」で右に置く
            // (元が変わったら選び直してもう一度 — 開く=再計算の仕掛けは持たない)
            "pivot-insert" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status =
                        ui::t!("元の表を範囲で選んでください(1行目が見出し)").into();
                } else {
                    let (a, b) = self.sel_rect();
                    if b.row <= a.row {
                        self.status = ui::t!("見出しの下にデータの行が要ります").into();
                    } else {
                        let headers: Vec<String> = (a.col..=b.col)
                            .map(|c| {
                                let v = self
                                    .sheet()
                                    .get(Pos::new(a.row, c))
                                    .map(|x| x.value.display())
                                    .unwrap_or_default();
                                if v.is_empty() { col_name(c) } else { v }
                            })
                            .collect();
                        self.status = format!(
                            "行に並べる見出し(カンマ区切り可): {}",
                            headers.join(" / ")
                        ).into();
                        self.pivot_pend = Some(PivotPend {
                            a,
                            b,
                            headers,
                            rows_sel: Vec::new(),
                            cols_sel: Vec::new(),
                        });
                        self.prompt = Some(("pivot-rows", Editor::new("")));
                    }
                }
            }
            // シートの保護。パスワードは掛けない(掛けた振りもしない)—
            // Excel でも「保護されたシート」に見え、解除も同じ1手でできる
            "prot-doc" => {
                let name = self.sheet().name.clone();
                if self.sheet().protected {
                    self.sheet_mut().protected = false;
                    self.dirty = true;
                    self.status = format!(
                        "シート「{name}」の保護を外しました(編集できます。保存で xlsx にも残ります)"
                    )
                    .into();
                } else {
                    self.commit();
                    self.sheet_mut().protected = true;
                    self.dirty = true;
                    self.status = format!(
                        "シート「{name}」を保護しました(編集を堰き止めます。同じ釦で解除。パスワードは掛けません — 掛けた振りもしません)"
                    )
                    .into();
                }
            }
            // 暗号化。パスワードを決めると、保存で ECMA-376 Standard
            // (AES-128)の複合ファイルに包む。空 Enter で解除
            "prot-encrypt" => {
                self.pw_pending = None;
                self.prompt = Some(("pw-set", Editor::new("")));
                self.status = if self.encrypt_pw.is_some() {
                    ui::t!("暗号化は入っています。新しいパスワードを打って Enter(空のまま Enter で暗号化をやめる)").into()
                } else {
                    ui::t!("暗号化: パスワードを打って Enter(次の保存から効きます)").into()
                };
            }
            // デジタル署名。**隣の .sig への添え書き**(Ed25519)。
            // Excel の署名欄には出ない独自方式 — そう言って出す。
            // 有効なら報告だけ、無効・未署名なら(作り直して)署名する
            "prot-sign" => {
                use ed25519_dalek::{Signer as _, Verifier as _};
                let Some(p) = self.path.clone() else {
                    self.status =
                        ui::t!("まだファイルになっていません(先に保存してください)").into();
                    return;
                };
                if self.dirty {
                    self.status =
                        ui::t!("未保存の変更があります。保存してから署名してください").into();
                    return;
                }
                let bytes = match std::fs::read(&p) {
                    Ok(b) => b,
                    Err(e) => {
                        self.status = format!("読めません: {e}").into();
                        return;
                    }
                };
                let sp = sig_path_for(&p);
                // 既にある署名を検める
                if let Ok(txt) = std::fs::read_to_string(&sp) {
                    let field = |k: &str| -> Option<String> {
                        txt.lines()
                            .find(|l| l.starts_with(k))
                            .map(|l| l[k.len()..].trim().to_string())
                    };
                    let ok = (|| -> Option<(String, bool)> {
                        let signer = field("signer:")?;
                        let vk: [u8; 32] = unhex(&field("pubkey:")?)?.try_into().ok()?;
                        let sg: [u8; 64] = unhex(&field("sig:")?)?.try_into().ok()?;
                        let vk = ed25519_dalek::VerifyingKey::from_bytes(&vk).ok()?;
                        let sig = ed25519_dalek::Signature::from_bytes(&sg);
                        Some((signer, vk.verify(&bytes, &sig).is_ok()))
                    })();
                    if let Some((signer, true)) = ok {
                        self.status = format!(
                            "署名は有効です — {signer} が署名した時のままの中身です"
                        )
                        .into();
                        return;
                    }
                }
                // 無い・壊れている・中身が変わった → 署名し(直し)て添える
                match load_or_make_key() {
                    Ok(key) => {
                        let sig = key.sign(&bytes);
                        let txt = format!(
                            "office-sign v1\nsigner: {}\npubkey: {}\nsig: {}\n",
                            lock_identity(),
                            to_hex(key.verifying_key().as_bytes()),
                            to_hex(&sig.to_bytes())
                        );
                        match std::fs::write(&sp, txt) {
                            Ok(_) => {
                                self.status = format!(
                                    "署名しました — 隣の {} に添え書き(独自方式。Excel の署名欄には出ません。もう一度押すと検めます)",
                                    sp.file_name().unwrap_or_default().to_string_lossy()
                                )
                                .into();
                            }
                            Err(e) => {
                                self.status = format!("署名が置けません: {e}").into();
                            }
                        }
                    }
                    Err(e) => self.status = format!("署名できません: {e}").into(),
                }
            }
            // 共同編集モード。実体はファイルの錠(.~lock)による早い者勝ちの
            // 編集権。押すと錠の今を確かめ、先客が去っていれば取り直す
            "coauth-mode" => match self.path.clone() {
                None => {
                    self.status =
                        ui::t!("まだファイルになっていません(保存すると編集権=錠を取ります)").into();
                }
                Some(p) => {
                    if self.my_lock.is_some() {
                        self.status = format!(
                            "編集権はこちら({})にあります。同じブックは先に開いた人が書け、後の人は読むだけになります(錠は .~lock ファイル)",
                            lock_identity()
                        )
                        .into();
                    } else {
                        self.acquire_lock(&p);
                        self.status = match &self.locked_by {
                            Some(who) => format!(
                                "{who} が編集中です(読めますが上書き保存はできません。相手が閉じたら、またこの釦で確かめてください)"
                            )
                            .into(),
                            None => ui::t!("先客が居なくなっていたので、編集権を取り直しました").into(),
                        };
                    }
                }
            },
            "co-showcomment" => {
                self.show_comments = !self.show_comments;
                self.status = if self.show_comments {
                    ui::t!("コメントを表示します").into()
                } else {
                    ui::t!("コメントを隠しました(付いてはいます)").into()
                };
            }
            "co-delcomment" => {
                let p = self.cursor;
                if self.sheet().comments.contains_key(&p) {
                    self.checkpoint();
                    self.book.sheets[self.active].comments.remove(&p);
                    self.dirty = true;
                    self.status =
                        format!("{} のコメントを外しました(Ctrl+Z で戻せます)", p.a1())
                            .into();
                } else {
                    self.status = ui::t!("このセルにコメントはありません").into();
                }
            }
            // バージョン履歴。上書き保存のたびに .jo-history へ残る控えの一覧
            "co-history" => {
                if self.path.is_none() {
                    self.status =
                        ui::t!("まだファイルになっていません(保存すると、上書きのたびに控えが残ります)").into();
                } else {
                    let v = self.versions();
                    if v.is_empty() {
                        self.status =
                            ui::t!("控えはまだありません(上書き保存のたびに .jo-history へ残ります)").into();
                    } else {
                        let names: Vec<String> = v.iter().map(|(n, _)| n.clone()).collect();
                        self.pick_paths = v;
                        self.pick_kind = "history";
                        self.pick = Some((names, (HEAD_W + 60.0, ROW_H + 20.0)));
                        self.status =
                            ui::t!("バージョン履歴: 選ぶと控えを名無しの複製で開きます(いまの書きかけは要るなら先に保存)").into();
                    }
                }
            }
            // チャット。ブックの隣の申し送り帳(.chat.txt)へ名乗り付きで追記。
            // サーバーは無いので生放送ではない — ファイル越しの言伝
            "co-chat" => match self.chat_path() {
                None => {
                    self.status =
                        ui::t!("まだファイルになっていません(保存すると、隣に申し送り帳ができます)").into();
                }
                Some(cp) => {
                    let tail = std::fs::read_to_string(&cp)
                        .map(|t| {
                            t.lines()
                                .rev()
                                .take(3)
                                .map(|l| l.to_string())
                                .collect::<Vec<_>>()
                                .into_iter()
                                .rev()
                                .collect::<Vec<_>>()
                                .join(" / ")
                        })
                        .unwrap_or_default();
                    self.status = if tail.is_empty() {
                        ui::t!("まだ言伝はありません(打って Enter で書き残します)").into()
                    } else {
                        format!("言伝: {tail}").into()
                    };
                    self.prompt = Some(("chat", Editor::new("")));
                }
            },
            // マクロ = Python in Calc と同じ実体(檻の中で .py を回す)
            "plug-macros" => {
                self.commit();
                self.run_python_file_dialog(cx);
                self.status =
                    ui::t!("マクロ: .py を選ぶと檻の中の Python が回ります(b=ブック s=シート。実体は データ > Python と同じ)").into();
            }
            // プラグインの管理。置き場の .py を一覧し、同じ檻で実行
            "plug-manage" => {
                let dir = plugins_dir();
                let mut items: Vec<PathBuf> = std::fs::read_dir(&dir)
                    .ok()
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e == "py"))
                    .collect();
                items.sort();
                if items.is_empty() {
                    self.status = format!(
                        "プラグイン: {} に .py を置くと、ここに並びます",
                        dir.display()
                    )
                    .into();
                } else {
                    let v: Vec<(String, PathBuf)> = items
                        .into_iter()
                        .map(|q| {
                            (
                                q.file_name().unwrap_or_default().to_string_lossy().to_string(),
                                q,
                            )
                        })
                        .collect();
                    let names: Vec<String> = v.iter().map(|(n, _)| n.clone()).collect();
                    self.pick_paths = v;
                    self.pick_kind = "plugin";
                    self.pick = Some((names, (HEAD_W + 60.0, ROW_H + 20.0)));
                    self.status =
                        ui::t!("プラグイン: 選ぶと檻の中の Python で実行します(b=ブック s=シート)").into();
                }
            }
            // チェックボックス(セルの部品)。空のセルに FALSE を置くと
            // ☑/☐ で見え、空白キーで切り替わる(Excel では TRUE/FALSE の値)
            "inscheckbox" => {
                self.commit();
                let (a, b) = self.sel_rect();
                let mut empties = Vec::new();
                let mut bools = 0usize;
                let mut skipped = 0usize;
                for r in a.row..=b.row {
                    for c in a.col..=b.col {
                        let p = Pos::new(r, c);
                        match self.sheet().get(p).map(|x| &x.value) {
                            None | Some(Value::Empty) => empties.push(p),
                            Some(Value::Bool(_)) => bools += 1,
                            _ => skipped += 1,
                        }
                    }
                }
                if empties.is_empty() && bools == 0 {
                    self.status =
                        ui::t!("空のセルを選んでください(中身のあるセルは潰しません)").into();
                } else {
                    if !empties.is_empty() {
                        self.checkpoint();
                        for p in &empties {
                            let mut cell =
                                self.sheet().get(*p).cloned().unwrap_or_default();
                            cell.formula = None;
                            cell.value = Value::Bool(false);
                            self.book.sheets[self.active].set(*p, cell);
                        }
                        recalc_book(&mut self.book, self.active);
                        self.dirty = true;
                        self.sync_input();
                    }
                    let skip_note = if skipped > 0 {
                        format!("。中身のある {skipped} セルは触っていません")
                    } else {
                        String::new()
                    };
                    self.status = format!(
                        "チェックボックスを {} 個置きました(空白キーで切替。Excel では TRUE/FALSE で見えます{skip_note})",
                        empties.len()
                    )
                    .into();
                }
            }
            // スライサー。カーソルの列の一意な値を釦で並べ、押して絞る。
            // 絞り込みと同じく**見え方だけ**(保存される中身は変わらない)
            "insslicer" => {
                if self.slicer.take().is_none() {
                    self.commit();
                    let col = self.cursor.col;
                    let (rows, _) = self.sheet().extent();
                    if rows < 2 {
                        self.status =
                            ui::t!("スライサーにする列を選んでください(見出しの下にデータの行が要ります)").into();
                    } else {
                        self.slicer =
                            Some((col, std::collections::BTreeSet::new(), false));
                        self.status = format!(
                            "スライサー: {} 列の値を押して絞る(≡=複数選択 / ✕=解除。見え方だけで、中身は変わりません)",
                            col_name(col)
                        )
                        .into();
                    }
                }
            }
            // テキストアート。文字を板に打つと飾り文字を描いて画像で置く
            "instextart" => {
                self.commit();
                self.prompt = Some(("textart", Editor::new("")));
                self.status =
                    ui::t!("テキストアート: 文字を打つと、太字+縁取りの飾り文字を画像で置きます").into();
            }
            // 方程式(数式エディタ)。式を板に打つと mathtext が清書して画像で置く
            "insequation" => {
                self.commit();
                self.prompt = Some(("equation", Editor::new("")));
                self.status =
                    ui::t!("方程式: TeX の書き方で(例: \\frac{a}{b} や \\sum_{i=1}^n i^2)。Enter で清書").into();
            }
            // SmartArt。分類 → 形の2段の一覧(分類・並び・名前は本家)
            "inssmartart" => {
                self.commit();
                let names: Vec<String> =
                    SMARTART.iter().map(|(n, _)| n.to_string()).collect();
                self.pick_kind = "sa-cat";
                self.pick = Some((names, (HEAD_W + 60.0, ROW_H + 20.0)));
                self.status =
                    ui::t!("SmartArt: 分類 → 形の順に選ぶ(図形の集まりとして入ります)").into();
            }
            // ソルバー。ONLYOFFICE と同じ小窓を開く(解法も同じ単体法 LP)
            "solver" => {
                if self.solver.take().is_none() {
                    self.commit();
                    let init = if self.anchor.is_some() {
                        self.sel_rect().0.a1()
                    } else {
                        self.cursor.a1()
                    };
                    self.solver = Some(Solver::new(&init));
                    self.status =
                        ui::t!("ソルバー: 欄を押して打つ。目的・変数セル・制約を決めて「解を求める」").into();
                }
            }
            // 下付き(vertAlign subscript)。上付きは本家 calc にも無い
            "subscript" => {
                self.fmt(|f| f.subscript = !f.subscript);
                self.status = ui::t!("下付きを切り替えました").into();
            }
            // 両端揃え(セルの横揃え。折り返した行を左右に伸ばす)
            "align-just" => {
                self.fmt(|f| {
                    f.align = if f.align == sheet::model::HAlign::Justify {
                        sheet::model::HAlign::General
                    } else {
                        sheet::model::HAlign::Justify
                    };
                    f.wrap = true; // 揃えるには折り返しが要る
                });
                self.status = ui::t!("両端揃えにしました(折り返して全体を表示も入れます)").into();
            }
            // 文字の回転(縦書きのセル。90度ずつ回る)
            "text-orient" => {
                self.fmt(|f| {
                    f.rotation = match f.rotation {
                        None | Some(0) => Some(90),
                        Some(90) => Some(180),
                        Some(180) => Some(255), // 255 = 縦に積む(xlsx の作法)
                        _ => None,
                    };
                });
                let r = self.sheet().get(self.cursor).map(|c| c.fmt.rotation).unwrap_or(None);
                self.status = match r {
                    Some(90) => ui::t!("文字を 90 度回しました").into(),
                    Some(180) => ui::t!("文字を 180 度回しました").into(),
                    Some(255) => ui::t!("文字を縦に積みました").into(),
                    _ => ui::t!("文字の向きを戻しました").into(),
                };
            }
            // 計算方法(自動 ⇔ 手動)。手動のときは F9 で計算する
            "calc-mode" => {
                self.auto_calc = !self.auto_calc;
                self.status = if self.auto_calc {
                    ui::t!("計算方法: 自動(いつもすぐ計算します)").into()
                } else {
                    ui::t!("計算方法: 手動(F9 で計算します — 大きな表で待たされない)").into()
                };
            }
            // 関数の挿入 = 本家と同じ小窓(検索・分類・一覧・説明)。
            // 数式バーの fx と同じ実体
            "insert-function" => {
                self.fn_dlg = Some(FnDlg {
                    search: Editor::new(""),
                    group: 0,
                    sel: 0,
                });
                self.status =
                    ui::t!("関数を挿入: 打って絞り込み、↑↓で選んで Enter(Esc で取消)").into();
            }
            // セルのスタイル(既定の書式の組。押すと一覧から選ぶ)
            "cell-styles" => {
                self.pick_kind = "cell-style";
                self.pick = Some((
                    CELL_STYLES.iter().map(|(n, _)| n.to_string()).collect(),
                    (HEAD_W + 60.0, ROW_H + 20.0),
                ));
                self.status = ui::t!("セルのスタイル: 選ぶと選択に掛かります(Ctrl+Z で戻せます)").into();
            }
            // シートの表示(隠したシートを戻す/いまのシートを隠す)
            "sheet-view" => {
                let hidden: Vec<(usize, String)> = self
                    .book
                    .sheets
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.hidden)
                    .map(|(i, s)| (i, s.name.clone()))
                    .collect();
                if hidden.is_empty() {
                    // 隠すほう(最後の1枚は隠さない — 見えるシートがゼロになる)
                    if self.book.sheets.iter().filter(|s| !s.hidden).count() <= 1 {
                        self.status = ui::t!("最後の1枚は隠せません").into();
                    } else {
                        let n = self.sheet().name.clone();
                        self.checkpoint_book();
                        self.sheet_mut().hidden = true;
                        // 見えるシートへ移る
                        if let Some(i) = self.book.sheets.iter().position(|s| !s.hidden) {
                            self.switch_sheet(i);
                        }
                        self.dirty = true;
                        self.status = format!(
                            "シート「{n}」を隠しました(同じ釦で戻せます。保存で xlsx にも残ります)"
                        )
                        .into();
                    }
                } else {
                    self.pick_kind = "unhide";
                    self.pick_paths = hidden
                        .iter()
                        .map(|(i, n)| (n.clone(), PathBuf::from(i.to_string())))
                        .collect();
                    self.pick = Some((
                        hidden.into_iter().map(|(_, n)| n).collect(),
                        (HEAD_W + 60.0, ROW_H + 20.0),
                    ));
                    self.status = ui::t!("隠したシート: 選ぶと表示に戻します").into();
                }
            }
            // ウォッチウィンドウ(見張りの窓)。選んだセルを控えて下に見せる
            "watch" => {
                let (a, b) = self.sel_rect();
                let mut n = 0usize;
                for r in a.row..=b.row {
                    for c in a.col..=b.col {
                        let p = Pos::new(r, c);
                        if self.sheet().get(p).and_then(|x| x.formula.as_ref()).is_some()
                            || self.anchor.is_none()
                        {
                            if !self.watch.contains(&(self.active, p)) {
                                self.watch.push((self.active, p));
                                n += 1;
                            }
                        }
                    }
                }
                if n == 0 && !self.watch.is_empty() {
                    self.watch.clear();
                    self.status = ui::t!("見張りを空にしました").into();
                } else {
                    self.status = format!(
                        "{n} 個を見張ります(値は下の帯に出ます。もう一度押すと空に)"
                    )
                    .into();
                }
            }
            // 昇順/降順(ホーム・データ)。右クリックの並べ替え▸と同じ道
            "sort-asc" | "sort-desc" => self.sort_active(id == "sort-asc"),
            // 描画の「選択」= 道具を措いてセルの操作に戻る(本家の並びの先頭)
            "draw-select" => {
                self.tool = None;
                self.ink_cur = None;
                self.status = ui::t!("セルの操作に戻りました").into();
            }
            // 描画(ペン・蛍光ペン・消しゴム)。writer と同じ形の道具の入切
            "pen" | "highlighter" | "eraser" => {
                let t = match id {
                    "pen" => 0u8,
                    "highlighter" => 1,
                    _ => 2,
                };
                self.tool = if self.tool == Some(t) { None } else { Some(t) };
                self.ink_cur = None;
                self.status = match self.tool {
                    Some(0) => ui::t!("ペン: 表の上をドラッグで描く(もう一度押すか Esc で戻る)").into(),
                    Some(1) => ui::t!("蛍光ペン: ドラッグで引く(セルの上に薄く乗る)").into(),
                    Some(2) => ui::t!("消しゴム: 線をなぞると1筆ずつ消える").into(),
                    _ => ui::t!("セルの操作に戻りました").into(),
                };
            }
            // AI タブ。**モデルに任せる変換と生成の道具箱**(writer と同じ宛先)
            "ai-where" => {
                let next = ui::ai::backend().next();
                ui::ai::set_backend(next);
                self.status = match ui::ai::ready(next) {
                    Ok(_) => format!("AI の宛先: {}(覚えました)", next.label()).into(),
                    Err(e) => format!(
                        "AI の宛先: {} — ただし今は使えません: {e}",
                        next.label()
                    )
                    .into(),
                };
            }
            "ai-summary" => self.ai_go(CalcAi::Summary, cx),
            "ai-rewrite" => self.ai_go(
                CalcAi::Rewrite(
                    "あなたは表の中の文字を整える道具です。渡されたタブ区切りの表と\
                     同じ行数・同じ列数のタブ区切りだけを返します。文字は意味を\
                     変えずに読みやすく直し、数字と空欄はそのまま写します。",
                    "次の表の文字を、意味を変えずに読みやすく直してください。",
                ),
                cx,
            ),
            "ai-polite" => self.ai_go(
                CalcAi::Rewrite(
                    "あなたは表の中の文字を整える道具です。渡されたタブ区切りの表と\
                     同じ行数・同じ列数のタブ区切りだけを返します。文字は内容を\
                     変えずに丁寧な言い方(です・ます)へ直し、数字と空欄はそのまま\
                     写します。",
                    "次の表の文字を、内容を変えずに丁寧な言い方へ直してください。",
                ),
                cx,
            ),
            "ai-plain" => self.ai_go(
                CalcAi::Rewrite(
                    "あなたは表の中の文字をやさしくする道具です。渡されたタブ区切りの\
                     表と同じ行数・同じ列数のタブ区切りだけを返します。難しい言葉を\
                     やさしい言葉に置き換え、数字と空欄はそのまま写します。",
                    "次の表の文字を、内容を変えずにやさしい日本語へ直してください。",
                ),
                cx,
            ),
            "ai-translate" => self.ai_go(CalcAi::Translate, cx),
            "ai-furigana" => self.ai_go(CalcAi::Furigana, cx),
            "ai-continue" => self.ai_go(CalcAi::Continue, cx),
            "ai-table" => {
                self.commit();
                self.prompt = Some(("ai-table", Editor::new("")));
                self.status = format!(
                    "AI({})が表にします: 文章を打って(貼って)Enter",
                    ui::ai::backend().label()
                )
                .into();
            }
            "ai-ask" => {
                self.commit();
                self.prompt = Some(("ai-ask", Editor::new("")));
                self.status = format!(
                    "AI({})に頼む: 用件を打って Enter(選んだ範囲があれば一緒に渡します)",
                    ui::ai::backend().label()
                )
                .into();
            }
            // 配色の変更(テーマ色の組を入れ替える)。テーマ由来の色を
            // 使っているセルは、色がそのまま追従する
            "colorschemas" => {
                self.pick_kind = "scheme";
                self.pick = Some((
                    sheet::theme::SCHEMES.iter().map(|(n, _)| n.to_string()).collect(),
                    (HEAD_W + 60.0, ROW_H + 20.0),
                ));
                self.status = ui::t!("配色の変更: 選ぶとテーマ色が入れ替わります").into();
            }
            // インターフェイステーマ(画面の明暗)。**セルは白のまま**
            "theme" => {
                self.dark = !self.dark;
                self.status = if self.dark {
                    ui::t!("画面を暗くしました(セルは白のまま — 画面と紙の一致を守る)").into()
                } else {
                    ui::t!("画面を明るくしました").into()
                };
            }
            // 範囲に変換する(表オブジェクトを外す。**書式と式は残る**)
            "td-torange" => {
                self.commit();
                let p = self.cursor;
                match self.sheet().tables.iter().position(|t| t.contains(p)) {
                    None => {
                        self.status =
                            ui::t!("表の中にカーソルを置いてください(表のない範囲は「表の挿入」で表にできます)").into();
                    }
                    Some(i) => {
                        self.checkpoint();
                        let t = self.book.sheets[self.active].tables.remove(i);
                        self.dirty = true;
                        self.status = format!(
                            "表「{}」を普通の範囲に戻しました(帯や縞々の書式と式はそのまま残ります)",
                            t.name
                        )
                        .into();
                    }
                }
            }
            // テーブルのサイズ変更(範囲を変える)。板で新しい範囲を聞く
            "td-resize" => {
                self.commit();
                let p = self.cursor;
                match self.sheet().tables.iter().position(|t| t.contains(p)) {
                    None => self.status = ui::t!("表の中にカーソルを置いてください").into(),
                    Some(i) => {
                        let t = &self.sheet().tables[i];
                        let init = format!("{}:{}", t.a.a1(), t.b.a1());
                        self.status = format!("表「{}」の新しい範囲は?", t.name).into();
                        self.prompt = Some(("table-resize", Editor::new(&init)));
                    }
                }
            }
            // シートの方向(右から左へ)。**日本語も右から書くことがある**
            "rtl-sheet" => {
                let on = !self.sheet().rtl;
                self.sheet_mut().rtl = on;
                self.dirty = true;
                self.status = if on {
                    ui::t!("右から左へ並べます(右横書き。列は右から A B C…)").into()
                } else {
                    ui::t!("左から右へ戻しました").into()
                };
            }
            // 文字の向き(セルの中を右横書きに)。1字ずつ右から並べる
            "direction" => {
                self.fmt(|f| f.rtl_text = !f.rtl_text);
                self.status =
                    ui::t!("セルの中を右横書きにしました(1字ずつ右から。昔の看板の書き方)").into();
            }
            // 表示タブ(本家のデスクトップ版に合わせる)。どれも見え方だけ
            "zoom-in" => {
                self.zoom = (self.zoom + 0.1).min(2.0);
                self.status = format!("ズーム {}%", (self.zoom * 100.0).round() as i32).into();
            }
            "zoom-out" => {
                self.zoom = (self.zoom - 0.1).max(0.5);
                self.status = format!("ズーム {}%", (self.zoom * 100.0).round() as i32).into();
            }
            "formula-bar" => {
                self.show_formula_bar = !self.show_formula_bar;
                self.status = if self.show_formula_bar {
                    ui::t!("数式バーを表示します").into()
                } else {
                    ui::t!("数式バーを隠しました(表示タブで戻せます)").into()
                };
            }
            "show-headings" => {
                self.show_headers = !self.show_headers;
                self.status = if self.show_headers {
                    ui::t!("見出しを表示します").into()
                } else {
                    ui::t!("見出しを隠しました(列幅のドラッグ等は見出しと一緒に戻ります)").into()
                };
            }
            "show-zeros" => {
                self.show_zeros = !self.show_zeros;
                self.status = if self.show_zeros {
                    ui::t!("0 を表示します").into()
                } else {
                    ui::t!("0 を隠しました(見え方だけ — 値は 0 のまま)").into()
                };
            }
            // 小計(Excel の集計)。本家のデータタブに無い釦だが、グループ化を
            // 「畳むと合計が残る」形で使うために要る(発注者指摘 2026-08-04)
            "subtotal" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status = ui::t!("表を範囲で選んでください(1行目が見出し)").into();
                } else {
                    let (a, b) = self.sel_rect();
                    if b.row <= a.row {
                        self.status = ui::t!("見出しの下にデータの行が要ります").into();
                    } else {
                        let headers: Vec<String> = (a.col..=b.col)
                            .map(|c| {
                                let v = self
                                    .sheet()
                                    .get(Pos::new(a.row, c))
                                    .map(|x| x.value.display())
                                    .unwrap_or_default();
                                if v.is_empty() { col_name(c) } else { v }
                            })
                            .collect();
                        self.status = format!(
                            "何の区切りで集めるか(見出しを1つ): {}",
                            headers.join(" / ")
                        )
                        .into();
                        self.sub_pend = Some(PivotPend {
                            a,
                            b,
                            headers,
                            rows_sel: Vec::new(),
                            cols_sel: Vec::new(),
                        });
                        self.prompt = Some(("subtotal-by", Editor::new("")));
                    }
                }
            }
            // グループ化(アウトライン)。行か列かは選択の形で決める:
            // 見出しから列をまるごと選んでいれば列、それ以外は選択の行。
            // 深さは xlsx の outlineLevel と往復し、畳みも保存に残る
            "group" | "ungroup" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status =
                        ui::t!("まとめたい行(または列)を選んでください(見出しの番号を撫でる)").into();
                } else {
                    let (a, b) = self.sel_rect();
                    let (rows_ext, cols_ext) = self.sheet().extent();
                    let whole_rows = a.row == 0 && b.row + 1 >= rows_ext.max(1);
                    let on_cols = whole_rows && !(a.col == 0 && b.col + 1 >= cols_ext.max(1));
                    self.checkpoint();
                    let add = id == "group";
                    let sh = self.sheet_mut();
                    if on_cols {
                        for c in a.col..=b.col {
                            let l = sh.col_outline.get(&c).copied().unwrap_or(0);
                            let nl = if add { (l + 1).min(7) } else { l.saturating_sub(1) };
                            if nl == 0 {
                                sh.col_outline.remove(&c);
                                sh.col_hidden.remove(&c);
                            } else {
                                sh.col_outline.insert(c, nl);
                            }
                        }
                    } else {
                        for r in a.row..=b.row {
                            let l = sh.row_outline.get(&r).copied().unwrap_or(0);
                            let nl = if add { (l + 1).min(7) } else { l.saturating_sub(1) };
                            if nl == 0 {
                                sh.row_outline.remove(&r);
                                sh.row_hidden.remove(&r);
                            } else {
                                sh.row_outline.insert(r, nl);
                            }
                        }
                    }
                    self.dirty = true;
                    let what = if on_cols {
                        format!("{}〜{}列", col_name(a.col), col_name(b.col))
                    } else {
                        format!("{}〜{}行", a.row + 1, b.row + 1)
                    };
                    self.status = if add {
                        format!(
                            "{what}をグループ化しました(深さ+1。「詳細の非表示」で畳めます。Ctrl+Z で戻せます)"
                        )
                        .into()
                    } else {
                        format!("{what}のグループ化を1段解きました(Ctrl+Z で戻せます)").into()
                    };
                }
            }
            // 詳細の非表示=グループ化した行(列)を畳む / 詳細の表示=開く。
            // 対象は選択、無ければカーソルの行が属するグループのひとつながり
            "hide-details" | "show-details" => {
                self.commit();
                let hide = id == "hide-details";
                let (a, b) = self.sel_rect();
                let (rows_ext, cols_ext) = self.sheet().extent();
                let whole_rows =
                    self.anchor.is_some() && a.row == 0 && b.row + 1 >= rows_ext.max(1);
                let on_cols = whole_rows && !(a.col == 0 && b.col + 1 >= cols_ext.max(1));
                if on_cols {
                    let sh = self.sheet();
                    let targets: Vec<u32> = (a.col..=b.col)
                        .filter(|c| sh.col_outline.contains_key(c))
                        .collect();
                    if targets.is_empty() {
                        self.status =
                            ui::t!("選択にグループ化した列がありません(先にグループ化)").into();
                    } else {
                        self.checkpoint();
                        let sh = self.sheet_mut();
                        for c in &targets {
                            if hide {
                                sh.col_hidden.insert(*c);
                            } else {
                                sh.col_hidden.remove(c);
                            }
                        }
                        self.dirty = true;
                        self.status = format!(
                            "{} 列を{}(Ctrl+Z で戻せます)",
                            targets.len(),
                            if hide { "畳みました" } else { "開きました" }
                        )
                        .into();
                    }
                } else {
                    // 行: 選択、または カーソルの行が属するグループのひとつながり
                    let (r0, r1) = if self.anchor.is_some() {
                        (a.row, b.row)
                    } else {
                        let sh = self.sheet();
                        let at = self.cursor.row;
                        if !sh.row_outline.contains_key(&at) {
                            self.status = ui::t!("グループ化した行の上で押してください(先に データ > グループ化)").into();
                            cx.notify();
                            return;
                        }
                        let mut lo = at;
                        while lo > 0 && sh.row_outline.contains_key(&(lo - 1)) {
                            lo -= 1;
                        }
                        let mut hi = at;
                        while sh.row_outline.contains_key(&(hi + 1)) {
                            hi += 1;
                        }
                        (lo, hi)
                    };
                    let sh = self.sheet();
                    let targets: Vec<u32> =
                        (r0..=r1).filter(|r| sh.row_outline.contains_key(r)).collect();
                    if targets.is_empty() {
                        self.status =
                            ui::t!("選択にグループ化した行がありません(先に データ > グループ化)").into();
                    } else {
                        self.checkpoint();
                        let sh = self.sheet_mut();
                        for r in &targets {
                            if hide {
                                sh.row_hidden.insert(*r);
                            } else {
                                sh.row_hidden.remove(r);
                            }
                        }
                        self.dirty = true;
                        self.status = format!(
                            "{} 行を{}(Ctrl+Z で戻せます)",
                            targets.len(),
                            if hide { "畳みました" } else { "開きました" }
                        )
                        .into();
                    }
                }
            }
            // ピボットの手入れ: どれも「指図を直して置き直す」だけ。
            // 対象はカーソルの下のピボット(指図はブックに控えてある)
            "pivot-refresh" => {
                self.commit();
                match self.pivot_at(self.cursor) {
                    Some(i) => {
                        let d = self.book.pivots[i].clone();
                        self.spawn_pivot(d, Some(i), cx);
                    }
                    None => {
                        self.status =
                            ui::t!("更新したいピボットの上にカーソルを置いてください").into();
                    }
                }
            }
            "pivot-refresh-all" => {
                self.commit();
                let n = self.book.pivots.len();
                if n == 0 {
                    self.status = ui::t!("このブックにピボットはありません").into();
                } else {
                    for i in 0..n {
                        let d = self.book.pivots[i].clone();
                        self.spawn_pivot(d, Some(i), cx);
                    }
                    self.status = format!("{n} 件のピボットを更新しています…").into();
                }
            }
            "pivot-select" => {
                match self.pivot_at(self.cursor) {
                    Some(i) => {
                        let d = &self.book.pivots[i];
                        self.cursor = d.dest;
                        self.anchor = Some(Pos::new(
                            d.dest.row + d.size.0.saturating_sub(1),
                            d.dest.col + d.size.1.saturating_sub(1),
                        ));
                        self.sync_input();
                        self.status = ui::t!("ピボット全体を選びました").into();
                    }
                    None => {
                        self.status = ui::t!("ピボットの上にカーソルを置いてください").into();
                    }
                }
            }
            "pivot-totals" | "pivot-subtotals" | "pivot-blank" | "pivot-layout" => {
                self.commit();
                match self.pivot_at(self.cursor) {
                    None => {
                        self.status = ui::t!("ピボットの上にカーソルを置いてください").into();
                    }
                    Some(i) => {
                        let need_two = matches!(id, "pivot-subtotals" | "pivot-blank");
                        if need_two && self.book.pivots[i].rows_sel.len() < 2 {
                            self.status =
                                ui::t!("行の見出しが2つ以上のピボットで効きます(挿入で複数選ぶ)").into();
                        } else {
                            let d = &mut self.book.pivots[i];
                            let (name, on) = match id {
                                "pivot-totals" => {
                                    d.totals = !d.totals;
                                    ("総計", d.totals)
                                }
                                "pivot-subtotals" => {
                                    d.subtotals = !d.subtotals;
                                    ("小計", d.subtotals)
                                }
                                "pivot-blank" => {
                                    d.blank_rows = !d.blank_rows;
                                    ("空行", d.blank_rows)
                                }
                                _ => {
                                    d.compact = !d.compact;
                                    ("コンパクト形式", d.compact)
                                }
                            };
                            let d = self.book.pivots[i].clone();
                            self.dirty = true;
                            self.status = format!(
                                "{name}を{}にして置き直します…",
                                if on { "あり" } else { "なし" }
                            )
                            .into();
                            self.spawn_pivot(d, Some(i), cx);
                        }
                    }
                }
            }
            // 表のデザイン: 表オブジェクトは持たない。選択に**1手ずつ掛ける道具**
            // (掛けた書式・式が帳面に残るだけ。切り替え式に見せない。
            // まとめて掛けるなら挿入タブの「表の挿入」)
            "td-header" | "td-band-row" | "td-band-col" | "td-first" | "td-last" => {
                // 表の中なら、表オブジェクトの性質も一緒に更新する
                let pcur = self.cursor;
                if let Some(i) = self.sheet().tables.iter().position(|t| t.contains(pcur)) {
                    let t = &mut self.book.sheets[self.active].tables[i];
                    match id {
                        "td-header" => t.header = !t.header,
                        "td-band-row" => t.banded_rows = !t.banded_rows,
                        "td-band-col" => t.banded_cols = !t.banded_cols,
                        "td-first" => t.first_col = !t.first_col,
                        _ => t.last_col = !t.last_col,
                    }
                    self.dirty = true;
                }
                self.commit();
                if self.anchor.is_none() {
                    self.status = ui::t!("表の範囲を選んでください").into();
                } else {
                    self.checkpoint();
                    let (a, b) = self.sel_rect();
                    for r in a.row..=b.row {
                        for c in a.col..=b.col {
                            let p = Pos::new(r, c);
                            let mut cell = self.sheet().get(p).cloned().unwrap_or_default();
                            let touched = match id {
                                "td-header" if r == a.row => {
                                    cell.fmt.bold = true;
                                    cell.fmt.fill = Some("D5E8DC".into());
                                    cell.fmt.borders.top = true;
                                    true
                                }
                                "td-band-row" if r > a.row && (r - a.row) % 2 == 0 => {
                                    cell.fmt.fill = Some("F1F6F3".into());
                                    true
                                }
                                "td-band-col" if (c - a.col) % 2 == 1 => {
                                    cell.fmt.fill = Some("F1F6F3".into());
                                    true
                                }
                                "td-first" if c == a.col => {
                                    cell.fmt.bold = true;
                                    true
                                }
                                "td-last" if c == b.col => {
                                    cell.fmt.bold = true;
                                    true
                                }
                                _ => false,
                            };
                            if touched {
                                self.book.sheets[self.active].set(p, cell);
                            }
                        }
                    }
                    self.dirty = true;
                    let what = match id {
                        "td-header" => "1行目を見出しの帯に",
                        "td-band-row" => "1行おきの縞々に",
                        "td-band-col" => "1列おきの縞々に",
                        "td-first" => "最初の列を太字に",
                        _ => "最後の列を太字に",
                    };
                    self.status = format!(
                        "{}:{} を{}しました(Ctrl+Z で戻せます)",
                        a.a1(),
                        b.a1(),
                        what
                    )
                    .into();
                }
            }
            // 合計行 = 選択の下に =SUM(…) の行を足す(式なので元が変われば追従)
            "td-total" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status = ui::t!("合計したい表の範囲を選んでください").into();
                } else {
                    let (a, b) = self.sel_rect();
                    let below_used = (a.col..=b.col).any(|c| {
                        self.sheet()
                            .get(Pos::new(b.row + 1, c))
                            .map(|cell| {
                                !cell.value.display().is_empty() || cell.formula.is_some()
                            })
                            .unwrap_or(false)
                    });
                    if below_used {
                        self.status =
                            ui::t!("すぐ下の行に中身があります(空けてから — 黙って上書きしません)").into();
                    } else {
                        self.checkpoint();
                        add_total_row(&mut self.book.sheets[self.active], a, b);
                        recalc_book(&mut self.book, self.active);
                        self.dirty = true;
                        self.status = format!(
                            "{} 行目に合計(=SUM)を足しました。式なので元が変われば追従します(Ctrl+Z で戻せます)",
                            b.row + 2
                        )
                        .into();
                    }
                }
            }
            // フィルタのボタン = データタブの絞り込みと同じ実体
            "td-filter" => self.run_cmd("setfilter", cx),
            // 表の挿入 = 選択に表の書式(見出しの帯+縞々+外枠)を掛ける
            "instable" | "table-tpl" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status = ui::t!("表にする範囲を選んでください").into();
                } else {
                    self.checkpoint();
                    let (a, b) = self.sel_rect();
                    for r in a.row..=b.row {
                        for c in a.col..=b.col {
                            let p = Pos::new(r, c);
                            let mut cell = self.sheet().get(p).cloned().unwrap_or_default();
                            if r == a.row {
                                cell.fmt.bold = true;
                                cell.fmt.fill = Some("D5E8DC".into());
                            } else if (r - a.row) % 2 == 0 {
                                cell.fmt.fill = Some("F1F6F3".into());
                            }
                            if r == a.row {
                                cell.fmt.borders.top = true;
                            }
                            if r == b.row {
                                cell.fmt.borders.bottom = true;
                            }
                            if c == a.col {
                                cell.fmt.borders.left = true;
                            }
                            if c == b.col {
                                cell.fmt.borders.right = true;
                            }
                            self.book.sheets[self.active].set(p, cell);
                        }
                    }
                    let n = self.book.sheets.iter().map(|s| s.tables.len()).sum::<usize>() + 1;
                    self.book.sheets[self.active].tables.push(sheet::model::TableDef {
                        name: format!("テーブル{n}"),
                        a,
                        b,
                        ..Default::default()
                    });
                    self.dirty = true;
                    self.status = format!(
                        "{}:{} を表にしました(見出しの帯と縞々。範囲に変換・サイズ変更もできます。Ctrl+Z で戻せます)",
                        a.a1(),
                        b.a1()
                    )
                    .into();
                }
            }
            // 記号を挿入: 一覧から選んで**数式バーへ**差し込む(セルは置き換えない)
            "inssymbol" => {
                let at = self
                    .cell_origin_px(self.cursor)
                    .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                    .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                self.pick_kind = "symbol";
                self.pick = Some((
                    ["〒", "℡", "№", "㈱", "〆", "※", "→", "←", "↑", "↓",
                     "○", "●", "◎", "△", "▲", "×", "☑", "☐", "✓", "①", "②", "③"]
                        .iter()
                        .map(|v| v.to_string())
                        .collect(),
                    at,
                ));
            }
            "addcomment" => {
                self.commit();
                let cur = self.sheet().comments.get(&self.cursor).cloned().unwrap_or_default();
                self.prompt = Some(("comment", Editor::new(&cur)));
            }
            "text-column" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status =
                        ui::t!("割りたいセルを選んでください(選択した列の文字を右へ割ります)").into();
                } else {
                    self.prompt = Some(("split-delim", Editor::new("")));
                }
            }
            "goal-seek" => {
                self.commit();
                // 目標セルの初期値はいまのセル(式のセルの上で押すのが自然)
                let init = if self.sheet().get(self.cursor).and_then(|c| c.formula.as_ref()).is_some()
                {
                    format!("{}=", self.cursor.a1())
                } else {
                    String::new()
                };
                self.goal = None;
                self.prompt = Some(("goal-target", Editor::new(&init)));
            }
            "data-external-links" => {
                // 他のブックを**値として**取り込む(リンクは張らない —
                // リンク切れの帳票を作らない。SEKKEI の分業どおり)
                self.commit();
                let ask = cx.background_executor().spawn(async {
                    let p = rfd::FileDialog::new()
                        .add_filter("Excelブック", &["xlsx"])
                        .pick_file()?;
                    Some(
                        std::fs::File::open(&p)
                            .map_err(|e| e.to_string())
                            .and_then(sheet::xlsx::read)
                            .map(|(b, _)| (p, b)),
                    )
                });
                cx.spawn(async move |this, cx| {
                    let r = ask.await;
                    let _ = this.update(cx, |this, cx| {
                        match r {
                            None => {}
                            Some(Ok((p, mut other))) => {
                                this.checkpoint();
                                sheet::recalc_all(&mut other);
                                let mut n = 0usize;
                                for mut sh in other.sheets.drain(..) {
                                    // 式は計算結果の値に(他所の参照を持ち込まない)
                                    for c in sh.cells.values_mut() {
                                        c.formula = None;
                                    }
                                    sh.name = format!(
                                        "{}({})",
                                        sh.name,
                                        p.file_stem().unwrap_or_default().to_string_lossy()
                                    );
                                    while this.book.sheets.iter().any(|x| x.name == sh.name) {
                                        sh.name.push('+');
                                    }
                                    this.book.sheets.push(sh);
                                    n += 1;
                                }
                                this.dirty = true;
                                this.status = format!(
                                    "{n} シートを値として取り込みました(リンクは張りません)"
                                )
                                .into();
                            }
                            Some(Err(e)) => this.status = format!("取り込めません: {e}").into(),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            // 拡大縮小印刷: 100→90→80→70→50→100
            "scale" => {
                self.commit();
                self.checkpoint();
                let sh = self.sheet_mut();
                let next = match sh.print_scale.unwrap_or(100) {
                    100 => 90,
                    90 => 80,
                    80 => 70,
                    70 => 50,
                    _ => 100,
                };
                sh.print_scale = if next == 100 { None } else { Some(next) };
                self.dirty = true;
                self.status = format!("拡大縮小印刷: {next}%(PDF と保存に効きます)").into();
            }
            // 改ページ: いまの行から新しい紙を始める(もう一度で解除)
            "pagebreak" => {
                self.commit();
                self.checkpoint();
                let r = self.cursor.row;
                let sh = self.sheet_mut();
                if let Some(i) = sh.row_breaks.iter().position(|b| *b == r) {
                    sh.row_breaks.remove(i);
                    self.dirty = true;
                    self.status = format!("{} 行の改ページを外しました", r + 1).into();
                } else if r == 0 {
                    self.undo_stack.pop();
                    self.status = ui::t!("1行目の前では改ページできません").into();
                } else {
                    sh.row_breaks.push(r);
                    self.dirty = true;
                    self.status =
                        format!("{} 行から新しい紙にします(もう一度で解除)", r + 1).into();
                }
            }
            // タイトルを印刷: 選んだ行を各ページの頭で繰り返す。選択なしで解除
            "printtitles" => {
                self.commit();
                if self.anchor.is_some() {
                    self.checkpoint();
                    let (a, b) = self.sel_rect();
                    self.sheet_mut().print_title_rows = Some((a.row, b.row));
                    self.dirty = true;
                    self.status = format!(
                        "{}〜{} 行を各ページの頭で繰り返します(選択なしで押すと解除)",
                        a.row + 1,
                        b.row + 1
                    )
                    .into();
                } else if self.sheet().print_title_rows.is_some() {
                    self.checkpoint();
                    self.sheet_mut().print_title_rows = None;
                    self.dirty = true;
                    self.status = ui::t!("タイトル行を解除しました").into();
                } else {
                    self.status =
                        ui::t!("繰り返す行を選んでから押してください(行の見出しをクリック)").into();
                }
            }
            "print-gridlines" => {
                self.commit();
                self.checkpoint();
                let sh = self.sheet_mut();
                sh.print_gridlines = !sh.print_gridlines;
                let on = sh.print_gridlines;
                self.dirty = true;
                self.status = format!(
                    "枠線の印刷: {}",
                    if on { "する(表の薄い線が紙に出ます)" } else { "しない" }
                )
                .into();
            }
            "print-headings" => {
                self.commit();
                self.checkpoint();
                let sh = self.sheet_mut();
                sh.print_headings = !sh.print_headings;
                let on = sh.print_headings;
                self.dirty = true;
                self.status = format!(
                    "見出しの印刷: {}",
                    if on { "する(行番号と列名が余白に出ます)" } else { "しない" }
                )
                .into();
            }
            // 検索と置換(ホーム > 置き換え)。板を2枚続けて使う
            "replace" => {
                self.commit();
                let init = self.find_term.clone().unwrap_or_default();
                self.prompt = Some(("find", Editor::new(&init)));
            }
            // グラフ(matplotlib)と画像。挿入タブ
            "inschart" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status =
                        ui::t!("グラフにする範囲を選んでください(1列目が項目名、2列目からが数)").into();
                } else {
                    let (a, b) = self.sel_rect();
                    self.insert_chart(a, b, cx);
                }
            }
            "insimage" => {
                self.commit();
                self.insert_image_dialog(cx);
            }
            "instext" => {
                // テキストボックス = 枠の図形 + 文字。すぐ文字の板を開く
                self.checkpoint();
                let at = self.cursor;
                self.sheet_mut().shapes_new.push(sheet::model::SheetShape {
                    at,
                    width_px: 200.0,
                    height_px: 80.0,
                    kind: "rect".into(),
                    fill: None,
                    line: Some("7F7F7F".into()),
                    ..Default::default()
                });
                self.shape_sel = Some(self.sheet().shapes_new.len() - 1);
                self.dirty = true;
                self.prompt = Some(("shape-text", Editor::new("")));
            }
            "inssparkline" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status =
                        ui::t!("折れ線にする数の範囲を選んでください(置き場所はいまのセル)").into();
                } else {
                    let (a, b) = self.sel_rect();
                    let mut vals: Vec<f64> = Vec::new();
                    for r in a.row..=b.row {
                        for c in a.col..=b.col {
                            if let Some(cell) = self.sheet().get(Pos::new(r, c)) {
                                if let sheet::Value::Number(n) = cell.value {
                                    vals.push(n);
                                }
                            }
                        }
                    }
                    if vals.len() < 2 {
                        self.status = ui::t!("数が2つ以上要ります").into();
                    } else {
                        let (lo, hi) = vals
                            .iter()
                            .fold((f64::MAX, f64::MIN), |(l, h), v| (l.min(*v), h.max(*v)));
                        let span = (hi - lo).max(1e-9);
                        let n = vals.len();
                        let points: Vec<(f32, f32)> = vals
                            .iter()
                            .enumerate()
                            .map(|(i, v)| {
                                (
                                    i as f32 / (n - 1) as f32,
                                    (1.0 - ((v - lo) / span)) as f32,
                                )
                            })
                            .collect();
                        // 置き場所はいまのセル(選択の中なら右のセル)、大きさはそのセル
                        let at = if (a.row..=b.row).contains(&self.cursor.row)
                            && (a.col..=b.col).contains(&self.cursor.col)
                        {
                            Pos::new(a.row, b.col + 1)
                        } else {
                            self.cursor
                        };
                        self.checkpoint();
                        let (w, h) = (self.col_px(at.col) - 2.0, self.row_px(at.row) - 2.0);
                        self.sheet_mut().shapes_new.push(sheet::model::SheetShape {
                            at,
                            width_px: w,
                            height_px: h,
                            kind: "spark".into(),
                            fill: None,
                            line: Some("1B6E3C".into()),
                            points,
                            ..Default::default()
                        });
                        self.dirty = true;
                        self.status = format!(
                            "スパークラインを {} に置きました(その時の値で描く固定の線。\
データを変えたら作り直してください)",
                            at.a1()
                        )
                        .into();
                    }
                }
            }
            "insshape" => {
                let at = self
                    .cell_origin_px(self.cursor)
                    .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                    .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                self.pick_kind = "shape";
                self.pick = Some((
                    ["四角形", "角丸四角形", "楕円", "右矢印", "ひし形", "直線"]
                        .iter()
                        .map(|v| v.to_string())
                        .collect(),
                    at,
                ));
            }
            "inshyperlink" => {
                self.commit();
                let cur = self.sheet().links.get(&self.cursor).cloned().unwrap_or_default();
                self.prompt = Some(("link", Editor::new(&cur)));
            }
            // データの入力規則。選んだ範囲に候補を付ける(板で受ける)
            "data-validation" => {
                self.commit();
                // 既にある規則は編集の初期値に(直書きは中身、参照は = 付き)
                let cur = self
                    .sheet()
                    .validation_at(self.cursor)
                    .map(|v| v.formula.clone())
                    .unwrap_or_default();
                let init = if cur.is_empty() {
                    String::new()
                } else if let Some(inner) =
                    cur.strip_prefix('"').and_then(|s| s.strip_suffix('"'))
                {
                    inner.to_string()
                } else {
                    format!("={cur}")
                };
                self.prompt = Some(("validation", Editor::new(&init)));
            }
            // 条件付き書式。右クリックメニューと同じ一覧を開く(道は1本)
            "condformat" => {
                let (x, y) = self
                    .cell_origin_px(self.cursor)
                    .map(|(x, y)| (x + 16.0, y + 16.0))
                    .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                self.menu_at = Some((x, y));
                self.menu_sub = Some("cond");
            }
            // 名前の管理。右クリックの「名前の定義」と同じ板
            "defname" => {
                self.commit();
                self.prompt = Some(("name", Editor::new("")));
            }
            "freeze" => {
                self.frozen = match self.frozen {
                    Some(_) => None,
                    None if self.cursor.row == 0 && self.cursor.col == 0 => {
                        self.status = ui::t!("固定する位置にカーソルを置いてください(その上と左が留まります)").into();
                        None
                    }
                    None => {
                        self.status = ui::tf!("{}行 {}列を固定しました", self.cursor.row, self.cursor.col).into();
                        Some(self.cursor)
                    }
                };
            }
            // 塗りつぶしの色。本家はパレット — 一覧から選ぶ
            // (順繰りの2色は仮実装だった。発注者指摘 2026-08-06)
            "fillparag" => {
                let at = self
                    .cell_origin_px(self.cursor)
                    .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                    .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                self.pick_kind = "fill-color";
                self.pick = Some((
                    FILL_COLORS.iter().map(|(n, _)| n.to_string()).collect(),
                    at,
                ));
            }
            // フォントの色。同じくパレット
            "fontcolor" => {
                let at = self
                    .cell_origin_px(self.cursor)
                    .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                    .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                self.pick_kind = "font-color";
                self.pick = Some((
                    FONT_COLORS.iter().map(|(n, _)| n.to_string()).collect(),
                    at,
                ));
            }
            // 並べ替えは**見出しを据え置き、行はまるごと動かす**
            "custom-sort" => {
                self.commit();
                self.checkpoint();
                let c = self.cursor.col;
                self.book.sheets[self.active].sort_by_column(c, true, true);
                self.dirty = true;
                recalc_book(&mut self.book, self.active);
                self.status = ui::tf!("{} 列で並べ替えました", Pos::new(0, c).a1()
                    .trim_end_matches('1')).into();
            }
            "rem-duplicates" => {
                self.commit();
                self.checkpoint();
                let n = self.book.sheets[self.active].remove_duplicate_rows(true);
                self.dirty = true;
                recalc_book(&mut self.book, self.active);
                // 何件消したかを黙らない
                self.status = ui::tf!("重複した {} 行を削除しました", n).into();
            }
            "currency" => self.fmt(|f| f.number_format = Some("¥#,##0".into())),
            "percents" => self.fmt(|f| f.number_format = Some("0%".into())),
            // 関数の一覧。**使える名前だけを出す** — 無いものを並べない
            f @ ("fn-math" | "fn-text" | "fn-logical" | "fn-recent" | "fn-datetime"
            | "fn-lookup" | "fn-financial" | "fn-more") => {
                let names: &str = match f {
                    "fn-math" => "SUM AVERAGE ROUND ROUNDUP ROUNDDOWN INT ABS MOD POWER SQRT \
                                  PRODUCT SUMPRODUCT SUMSQ CEILING FLOOR MROUND EVEN ODD SIGN \
                                  FACT COMBIN PERMUT GCD LCM PI SIN COS TAN ASIN ACOS ATAN ATAN2 \
                                  SINH COSH TANH EXP LN LOG LOG10 DEGREES RADIANS RAND RANDBETWEEN \
                                  SEQUENCE(隣へあふれる。=SEQUENCE(3)+1 のような式も可)",
                    "fn-text" => "LEN LEFT RIGHT MID TRIM UPPER LOWER CONCATENATE CONCAT TEXT \
                                  SUBSTITUTE FIND SEARCH VALUE TEXTJOIN REPT CHAR CODE \
                                  UNICHAR UNICODE PROPER EXACT CLEAN FIXED YEN NUMBERVALUE \
                                  LENB LEFTB RIGHTB MIDB ASC JIS DATESTRING(和暦) \
                                  PHONETIC(ふりがな — 読んだ xlsx の rPh を引く)",
                    "fn-logical" => "IF IFS SWITCH AND OR NOT TRUE FALSE ISBLANK ISERROR IFERROR \
                                     IFNA ISNA ISERR ISLOGICAL ISNONTEXT ISNUMBER ISTEXT NA",
                    "fn-datetime" => "TODAY NOW DATE DATEVALUE YEAR MONTH DAY WEEKDAY \
                                      TIME HOUR MINUTE SECOND EDATE EOMONTH DATEDIF \
                                      WORKDAY NETWORKDAYS DAYS DAYS360 YEARFRAC \
                                      WEEKNUM ISOWEEKNUM(値は通し番号)",
                    "fn-lookup" => "VLOOKUP HLOOKUP XLOOKUP LOOKUP INDEX MATCH CHOOSE \
                                    ROW COLUMN ROWS COLUMNS OFFSET INDIRECT ADDRESS HYPERLINK \
                                    FILTER SORT UNIQUE TRANSPOSE(照合は完全一致。\
                                    FILTER 等は隣へあふれ、四則と組み合わせても効く)",
                    "fn-financial" => "PMT PV FV NPER NPV IRR RATE(IRR と RATE は挟み撃ちの反復解)",
                    "fn-more" => "SUMIF SUMIFS COUNTIF COUNTIFS AVERAGEIF AVERAGEIFS \
                                  MINIFS MAXIFS COUNTA COUNTBLANK TRUNC \
                                  RANK RANK.EQ RANK.AVG LARGE SMALL \
                                  MEDIAN MODE STDEV STDEVP VAR VARP PERCENTILE QUARTILE \
                                  CORREL SLOPE INTERCEPT FORECAST AVERAGEA MAXA MINA \
                                  SUBTOTAL QUOTIENT CEILING.MATH FLOOR.MATH \
                                  ISEVEN ISODD T N TYPE — 一覧は各族の釦で",
                    _ => "SUM AVERAGE COUNT MAX MIN IF SUMIF COUNTIF VLOOKUP TODAY",
                };
                self.status = ui::tf!("使える関数: {}", names).into();
            }
            f @ ("sum" | "average" | "count" | "max" | "min") => {
                // 上の連続した数値をまとめる(表計算の当たり前の動き)
                let name = f.to_uppercase();
                let (r, c) = (self.cursor.row, self.cursor.col);
                let mut top = r;
                while top > 0 && self.sheet().get(Pos::new(top - 1, c))
                    .map(|x| matches!(x.value, Value::Number(_)) || x.formula.is_some())
                    .unwrap_or(false) { top -= 1 }
                let text = if top < r {
                    format!("={name}({}:{})", Pos::new(top, c).a1(), Pos::new(r - 1, c).a1())
                } else {
                    format!("={name}()")
                };
                self.input = Editor::new(&text);
                self.commit();
                self.sync_input();
            }
            other => {
                // ここに来たら結線漏れ。黙らず画面に出す
                self.status = ui::tf!("未配線のコマンド: {}(不具合です)", other).into();
            }
        }
    }

}

impl Drop for Calc {
    fn drop(&mut self) {
        // 置きっぱなしのロックは他の人の警告になってしまう。最後の保険
        self.release_lock();
    }
}

impl Focusable for Calc {
    fn focus_handle(&self, _cx: &App) -> FocusHandle { self.focus.clone() }
}

impl EntityInputHandler for Calc {
    fn text_for_range(&mut self, r: Range<usize>, actual: &mut Option<Range<usize>>,
                      _w: &mut Window, _cx: &mut Context<Self>) -> Option<String> {
        handler::text_for_range(self, r, actual)
    }
    fn selected_text_range(&mut self, _i: bool, _w: &mut Window, _cx: &mut Context<Self>)
        -> Option<UTF16Selection> {
        Some(UTF16Selection { range: handler::selected_range_utf16(self), reversed: false })
    }
    fn marked_text_range(&self, _w: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
        handler::marked_range_utf16(self)
    }
    fn unmark_text(&mut self, _w: &mut Window, _cx: &mut Context<Self>) { handler::unmark(self) }
    fn replace_text_in_range(&mut self, r: Option<Range<usize>>, text: &str,
                             _w: &mut Window, cx: &mut Context<Self>) {
        // 空白キーはチェックボックス(Bool のセル)の切替。打ちかけ・板・
        // 小窓が無いときだけ(文字としての空白を奪わない)
        if text == " " && self.prompt.is_none() && self.solver.is_none() && !self.editing() {
            if let Some(Value::Bool(b)) =
                self.sheet().get(self.cursor).map(|c| c.value.clone())
            {
                if self.sheet().protected {
                    self.status =
                        ui::t!("シートが保護されています(保護タブの「シートを保護する」で解除)").into();
                } else {
                    self.checkpoint();
                    let p = self.cursor;
                    let mut cell = self.sheet().get(p).cloned().unwrap_or_default();
                    cell.formula = None;
                    cell.value = Value::Bool(!b);
                    self.book.sheets[self.active].set(p, cell);
                    recalc_book(&mut self.book, self.active);
                    self.dirty = true;
                    self.sync_input();
                    self.status = ui::tf!("{} = {}(空白キーで切替)", p.a1(), if b { "☐" } else { "☑" })
                    .into();
                }
                cx.notify();
                return;
            }
        }
        // セルを選んで**打ち始めたら置き換え**(Excel の作法)。追記になるのは
        // 同じセルで編集を続けている間(edit_armed)だけ — F2・ダブルクリック・
        // 2打目以降。IME の変換途中(marked)は消さない
        if self.prompt.is_none() && self.solver.is_none()
            && self.name_edit.is_none() && self.fn_dlg.is_none()
            && self.fn_args.is_none()
            && !self.edit_armed && !self.editing()
            && handler::marked_range_utf16(self).is_none()
        {
            self.input = Editor::new("");
            self.edit_armed = true;
        }
        handler::replace(self, r, text);
        cx.notify();
    }
    fn replace_and_mark_text_in_range(&mut self, r: Option<Range<usize>>, text: &str,
                                      sel: Option<Range<usize>>, _w: &mut Window,
                                      cx: &mut Context<Self>) {
        // IME の1打目も同じ(変換中の下線ごと、空にしてから始める)
        if self.prompt.is_none() && self.solver.is_none()
            && self.name_edit.is_none() && self.fn_dlg.is_none()
            && self.fn_args.is_none()
            && !self.edit_armed && !self.editing()
            && handler::marked_range_utf16(self).is_none()
        {
            self.input = Editor::new("");
            self.edit_armed = true;
        }
        handler::replace_and_mark(self, r, text, sel);
        cx.notify();
    }
    fn bounds_for_range(&mut self, _r: Range<usize>, bounds: Bounds<gpui::Pixels>,
                        _w: &mut Window, _cx: &mut Context<Self>)
        -> Option<Bounds<gpui::Pixels>> {
        // IME の候補窓は選択中のセルの下に出す
        Some(Bounds::new(
            gpui::point(
                bounds.origin.x
                    + px(HEAD_W + self.col_x(self.cursor.col) - self.col_x(self.view.col)),
                bounds.origin.y
                    + px(2.0 * ROW_H
                        + (self.view.row..self.cursor.row)
                            .map(|r| self.row_px(r))
                            .sum::<f32>()),
            ),
            size(px(self.col_px(self.cursor.col)), px(ROW_H)),
        ))
    }
    fn character_index_for_point(&mut self, _p: gpui::Point<gpui::Pixels>,
                                 _w: &mut Window, _cx: &mut Context<Self>) -> Option<usize> {
        None
    }
    fn text_length_utf16(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> Option<usize> {
        Some(handler::text_len_utf16(self))
    }
}

/// 入力ハンドラは paint のときに窓へ差す(GPUI の作法)。
struct InputSink { view: Entity<Calc> }
impl IntoElement for InputSink { type Element = Self; fn into_element(self) -> Self { self } }
impl gpui::Element for InputSink {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<gpui::ElementId> { None }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> { None }
    fn request_layout(&mut self, _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>, window: &mut Window, cx: &mut App)
        -> (gpui::LayoutId, ()) {
        let mut s = gpui::Style::default();
        // **格子の上に全面で重ねる。** 流れの中に置くと格子の右へ押し出され、
        // bounds が格子とずれてマウスが一切当たらなくなる(踏んで直した)
        s.position = gpui::Position::Absolute;
        s.inset.top = gpui::px(0.0).into();
        s.inset.left = gpui::px(0.0).into();
        s.size.width = gpui::relative(1.0).into();
        s.size.height = gpui::relative(1.0).into();
        (window.request_layout(s, [], cx), ())
    }
    fn prepaint(&mut self, _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>, _: Bounds<gpui::Pixels>,
        _: &mut (), _: &mut Window, _: &mut App) {}
    fn paint(&mut self, _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>, bounds: Bounds<gpui::Pixels>,
        _: &mut (), _: &mut (), window: &mut Window, cx: &mut App) {
        let focus = self.view.read(cx).focus.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.view.clone()), cx);
        // マウスは窓のレベルで受けて、座標からセルを逆算する(writer と同じ方式)。
        // セルごとのホバー判定に頼ると、ドラッグ中の移動を取り逃すことがある
        let view = self.view.clone();
        window.on_mouse_event(move |e: &gpui::MouseDownEvent, phase, _w, cx| {
            if phase != gpui::DispatchPhase::Bubble
                || e.button != gpui::MouseButton::Left
                || !bounds.contains(&e.position)
            {
                return;
            }
            let rel = e.position - bounds.origin;
            view.update(cx, |c, cx| {
                c.mouse_down_at(
                    f32::from(rel.x),
                    f32::from(rel.y),
                    e.modifiers.shift,
                    e.modifiers.control,
                    e.click_count,
                );
                cx.notify();
            });
        });
        let view = self.view.clone();
        window.on_mouse_event(move |e: &gpui::MouseMoveEvent, phase, _w, cx| {
            // ドラッグ中は格子の外でも受ける(端で選択が止まらないように、
            // 位置は格子の中のセルに丸められる)
            if phase != gpui::DispatchPhase::Bubble
                || e.pressed_button != Some(gpui::MouseButton::Left)
            {
                return;
            }
            let rel = e.position - bounds.origin;
            view.update(cx, |c, cx| {
                if c.shape_drag.is_some() {
                    c.shape_drag_at(f32::from(rel.x), f32::from(rel.y));
                    cx.notify();
                } else if c.size_drag.is_some() {
                    c.size_drag_at(f32::from(rel.x), f32::from(rel.y));
                    cx.notify();
                } else if c.drag.is_some()
                    || c.head_drag.is_some()
                    || c.ink_cur.is_some()
                    || c.tool == Some(2)
                    // 関数の引数・式の直入力のセル掴み(範囲をなぞる)も
                    // ここを通す — この表に入れ忘れると「押せるのに伸びない」
                    // (writer で踏んだ罠)
                    || c.fn_args.as_ref().is_some_and(|a| a.pick_from.is_some())
                    || c.ref_pick.is_some()
                {
                    // 筆と消しゴムもここを通る(描きかけ・なぞり)
                    c.mouse_drag_at(f32::from(rel.x), f32::from(rel.y));
                    cx.notify();
                }
            });
        });
        let view = self.view.clone();
        window.on_mouse_event(move |e: &gpui::MouseUpEvent, phase, _w, cx| {
            if phase != gpui::DispatchPhase::Bubble || e.button != gpui::MouseButton::Left {
                return;
            }
            view.update(cx, |c, cx| {
                c.mouse_up();
                cx.notify();
            });
        });
        // 右クリックでメニュー
        let view = self.view.clone();
        window.on_mouse_event(move |e: &gpui::MouseDownEvent, phase, _w, cx| {
            if phase != gpui::DispatchPhase::Bubble
                || e.button != gpui::MouseButton::Right
                || !bounds.contains(&e.position)
            {
                return;
            }
            let rel = e.position - bounds.origin;
            view.update(cx, |c, cx| {
                c.right_click_at(f32::from(rel.x), f32::from(rel.y));
                cx.notify();
            });
        });
    }
}

/// AI に頼む仕事(calc 流)。writer と同じ10釦だが、表計算なので
/// 渡すのは選択範囲の TSV、返してもらうのも TSV や式になる。
#[derive(Clone)]
enum CalcAi {
    /// 選択(無ければ使っている範囲)の表を要約 → カーソルのコメントへ
    Summary,
    /// 文字のセルを書き直して置き換える(整える・敬語・やさしく)
    Rewrite(&'static str, &'static str),
    /// 文字のセルを訳して置き換える
    Translate,
    /// 選択した1列の読みを右隣の列へ(名簿のフリガナ欄)
    Furigana,
    /// 選択のパターンから続きの行を作り、下の空きへ
    Continue,
    /// 文章から表を作り、カーソルから流し込む
    Table(String),
    /// 自由に頼む。= で始まる答えは式としてカーソルへ、他はコメントへ
    Ask(String),
}

impl CalcAi {
    /// モデルへの言いつけ(system)と、何を渡すか
    fn prompt(&self) -> (&'static str, &'static str) {
        match self {
            CalcAi::Summary => (
                "あなたは表を読む道具です。渡されたタブ区切りの表の要点を、                 2〜4文の日本語でまとめてください。前置き・後書きは書かず、                 要約の本文だけを返します。",
                "次の表を要約してください。",
            ),
            CalcAi::Rewrite(sys, ask) => (sys, ask),
            CalcAi::Translate => (
                "あなたは表の中の文字を訳す道具です。渡されたタブ区切りの表と                 同じ行数・同じ列数のタブ区切りだけを返します。文字は日本語なら                 英語へ、それ以外なら日本語へ訳し、数字と空欄はそのまま写します。                 説明は書きません。",
                "次の表の文字を訳してください。",
            ),
            CalcAi::Furigana => (
                "あなたは日本語の読みを返す道具です。渡された1行1語の並びに                 対して、同じ行数で、各行にその語の読みをカタカナだけで返します。                 説明・記号は書きません。読めない行は空行にします。",
                "次の各行の読みをカタカナで返してください。",
            ),
            CalcAi::Continue => (
                "あなたは表のパターンを読む道具です。渡されたタブ区切りの表の                 規則を読み取り、**続きの行を3行だけ**、同じ列数のタブ区切りで                 返します。元の行は返しません。説明は書きません。",
                "次の表の続きの行を作ってください。",
            ),
            CalcAi::Table(_) => (
                "あなたは文章を表に整える道具です。渡された文章から表を作り、                 タブ区切り(1行目は見出し)だけを返します。説明・前置き・                 罫線の記号は書きません。",
                "",
            ),
            CalcAi::Ask(_) => (
                "あなたは表計算を手伝う道具です。数式を頼まれたら = で始まる                 1つの数式だけを返します(使える関数: SUM AVERAGE COUNT COUNTA                  MIN MAX SUMIF COUNTIF ABS MOD POWER SQRT INT ROUND ROUNDUP TRUNC                  PRODUCT PMT PV FV NPER TODAY NOW DATE YEAR MONTH DAY WEEKDAY LEN                  LEFT RIGHT MID TRIM UPPER LOWER CONCATENATE IF AND OR NOT IFERROR                  ISBLANK ISERROR VLOOKUP HLOOKUP INDEX MATCH)。それ以外の頼みには                 答えの本文だけを返します。前置きは書きません。",
                "",
            ),
        }
    }

    fn label(&self) -> &'static str {
        match self {
            CalcAi::Summary => "要約",
            CalcAi::Rewrite(_, _) => "書き直し",
            CalcAi::Translate => "翻訳",
            CalcAi::Furigana => "ふりがな",
            CalcAi::Continue => "続き",
            CalcAi::Table(_) => "表",
            CalcAi::Ask(_) => "頼み",
        }
    }
}

impl Render for Calc {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 窓の大きさを控える(見える行数・列数がこれに追従する)
        self.view_w_px = f32::from(window.viewport_size().width);
        self.view_h_px = f32::from(window.viewport_size().height);
        if std::env::var_os("JO_SELFTEST").is_some() {
            // 実際に描画が走った証拠を残す(notify だけでは画面は変わらない —
            // これが止まってティックが続くなら、提示(present)の停止)
            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            eprintln!("render #{}", N.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        }
        // ---- 画面の額縁(デスクトップ版の形。writer と同じ構成) ----
        // 1段目 = クイックアクセス+ブック名(この行が窓の取っ手)。
        // 表計算の色は緑(デスクトップ版の app 色分けと同じ)。
        // 2段目 = 白地のタブ+現在地の緑の下線。右端に 🔍。
        // 下端 = ステータスバー(シートの耳+状態の文言+選択の生きた値)
        let (ready, all) = ribbon::progress(ribbon::calc_tabs());
        // 画面の明暗(インターフェイステーマ)。**セルは白のまま** —
        // 暗くするのは周り(帯・タブ・釦・見出し・耳)だけ
        let dk = self.dark;
        let th_bar = if dk { rgb(0x14432A) } else { rgb(0x1B6E3C) };
        let th_band = if dk { rgb(0x1B1E21) } else { rgb(0xFFFFFF) };
        let th_fg = if dk { rgb(0xCFD6DC) } else { rgb(0x444B52) };
        let th_gray = if dk { rgb(0x565D64) } else { rgb(0xB6BDC4) };
        let th_hover = if dk { rgb(0x2C333A) } else { rgb(0xEAF5EE) };
        let th_line = if dk { rgb(0x33383D) } else { rgb(0xE1E6EA) };
        let th_head = if dk { rgb(0x22262A) } else { rgb(0xEFF2F4) };
        let qa = |id: &'static str, icon: &'static str| {
            div().id(id).px_2().py_1().rounded_sm().cursor_pointer()
                .hover(move |s| s.bg(rgb(0x2E8B57)))
                .child(gpui::svg()
                    .path(SharedString::from(format!("icons/{icon}.svg")))
                    .size(px(15.0)).text_color(rgb(0xE8F3EC)))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let title = self
            .path
            .as_ref()
            .and_then(|q| q.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| ui::t!("無題のブック").into());
        let winbtn = |id: &'static str, label: &'static str| {
            div().id(id).px_2p5().py_1().rounded_sm()
                .text_size(px(12.0)).text_color(rgb(0xCFE6D8))
                .cursor_pointer()
                .hover(move |s| if id == "close" { s.bg(rgb(0xC0392B)).text_color(rgb(0xFFFFFF)) }
                                else { s.bg(rgb(0x2E8B57)).text_color(rgb(0xFFFFFF)) })
                .child(label)
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let top = div().id("titlebar").flex().flex_row().items_center().gap_0p5()
            .px_2().py_0p5().bg(th_bar)
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                |_, e: &gpui::MouseDownEvent, window, _| {
                    if e.click_count >= 2 {
                        window.zoom_window();
                    } else {
                        window.start_window_move();
                    }
                }))
            .child(qa("qa-save", "save").on_click(cx.listener(|this, _, _, cx| {
                this.run_cmd("save", cx);
                cx.notify()
            })))
            .child(qa("qa-print", "print").on_click(cx.listener(|this, _, _, cx| {
                this.run_cmd("pdf", cx);
                cx.notify()
            })))
            .child(qa("qa-undo", "undo").on_click(cx.listener(|this, _, _, cx| {
                this.run_cmd("undo", cx);
                cx.notify()
            })))
            .child(qa("qa-redo", "redo").on_click(cx.listener(|this, _, _, cx| {
                this.run_cmd("redo", cx);
                cx.notify()
            })))
            .child(div().flex_1())
            .child(div().text_size(px(12.5)).text_color(rgb(0xFFFFFF))
                .whitespace_nowrap().overflow_hidden()
                .child(SharedString::from(format!(
                    "{}{title}",
                    if self.dirty { "*" } else { "" }
                ))))
            .child(div().flex_1())
            .child(div().pr_2().text_size(px(10.5)).text_color(rgb(0x9CC9AF))
                .child(SharedString::from(ui::tf!("calc — 実装済み {}/{}", ready, all))))
            .child(winbtn("min", "─").on_click(cx.listener(|_, _, window, _| {
                window.minimize_window();
            })))
            .child(winbtn("max", "▢").on_click(cx.listener(|_, _, window, _| {
                window.zoom_window();
            })))
            .child(winbtn("close", "✕").on_click(cx.listener(|this, _, _, cx| {
                this.request_quit(cx);
            })));

        let mut tabs = div().flex().flex_row().items_end().gap_1()
            .px_2().bg(th_band);
        for (i, tb) in ribbon::calc_tabs().iter().enumerate() {
            let on = i == self.tab;
            tabs = tabs.child(div()
                .id(SharedString::from(format!("tab{i}")))
                .px_2p5().pt_1p5()
                .text_size(px(12.0))
                .text_color(if on { rgb(0x2E8B57) } else { th_fg })
                .font_weight(if on { gpui::FontWeight::BOLD } else { gpui::FontWeight::NORMAL })
                .cursor_pointer()
                .hover(|s| s.text_color(rgb(0x1B6E3C)))
                .flex().flex_col().items_center().gap_1()
                .child(tb.name)
                // 現在地の緑の下線(デスクトップ版の形)
                .child(div().h(px(2.5)).w_full().rounded_sm()
                    .bg(if on { rgb(0x2E8B57) } else { th_band }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.tab != 0 {
                        this.prev_tab = this.tab;
                    }
                    this.tab = i;
                    cx.notify()
                })));
        }
        tabs = tabs.child(div().flex_1())
            .child(div().id("tab-find").px_2().pb_1().text_size(px(12.0))
                .text_color(rgb(0x555E66)).cursor_pointer()
                .hover(|s| s.text_color(rgb(0x1B6E3C)))
                .child("🔍")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.run_cmd("replace", cx);
                    cx.notify()
                })));

        // 釦の帯: 本家のデスクトップ版の一段の絵釦(writer の写し)。
        // 主要な釦は名札つきの大釦、他は絵だけ(乗ると名前が下のステータス
        // バーへ)。絵の無い釦は小さな文字の釦。ホームだけ2段(釦が多い)
        const BIG: &[(&str, &str)] = &[
            ("instable", "表"), ("insimage", "画像"), ("insshape", "図形"),
            ("inschart", "グラフ"), ("inssmartart", "SmartArt"),
            ("autosum", "オートSUM"), ("recent", "最近使った関数"),
            ("pagemargins", "余白"), ("pageorient", "向き"), ("pagesize", "サイズ"),
            ("printarea", "印刷範囲"),
            ("data-from-text", "テキストから"), ("custom-sort", "並べ替え"),
            ("setfilter", "フィルター"), ("python", "Python"),
            ("subtotal", "小計"), ("solver", "ソルバー"), ("group", "グループ化"),
            ("pivot-insert", "ピボットの挿入"),
            ("td-header", "ヘッダー行"), ("td-total", "合計行"),
            ("coauth-mode", "共同編集モード"), ("co-addcomment", "コメント"),
            ("co-chat", "チャット"), ("co-history", "バージョン履歴"),
            ("prot-encrypt", "暗号化"), ("prot-sign", "署名"), ("prot-doc", "保護"),
            ("freeze", "枠の固定"), ("pen", "ペン"), ("highlighter", "蛍光ペン"),
            ("eraser", "消しゴム"),
            ("plug-macros", "マクロ"), ("plug-manage", "プラグインの管理"),
        ];
        let th_cmd_border = th_line;
        let th_btn_hover = th_hover;
        let mut cmds = div().flex().flex_col().gap_0p5()
            .px_3().py_1().bg(th_band)
            .border_b_1().border_color(th_cmd_border);
        let items = ribbon::calc_tabs()[self.tab].cmds;
        // 今のセルの書体と大きさ(ホームの欄に出す — 本家はコンボボックスで
        // **今の値が見える**。slot-field-fontname/fontsize)
        let cur_fmt = self.sheet().get(self.cursor).map(|c| c.fmt.clone()).unwrap_or_default();
        let cur_font: SharedString = cur_fmt.font.clone()
            .unwrap_or_else(|| "Noto Sans JP".into()).into();
        let cur_size: SharedString = {
            let pt = cur_fmt.size_c.map(|c| c as f32 / 100.0).unwrap_or(11.0);
            if (pt - pt.round()).abs() < 0.05 {
                format!("{}", pt.round() as u32).into()
            } else {
                format!("{pt:.1}").into()
            }
        };
        // 1つの釦を組み立てる(名札つきの大釦 / 絵だけ / 文字の小釦)。
        // ホームの対の並びと、他タブの一段の並びの両方から使う
        let mk_btn = |cmd: &ribbon::Cmd, cx: &mut Context<Self>| -> gpui::AnyElement {
            let label = cmd.label;
            let icon = cmd.icon;
            // 書体と大きさは釦でなく**欄**(本家の形): 今の値を枠の中に見せ、
            // 押すと一覧が開く
            if cmd.id == "fontname" || cmd.id == "fontsize" {
                let (w, val) = if cmd.id == "fontname" {
                    (110.0, cur_font.clone())
                } else {
                    (38.0, cur_size.clone())
                };
                let cid = cmd.id;
                let hoverable = cx.listener(move |this: &mut Calc, on: &bool, _, cx| {
                    if *on {
                        this.hover_hint = Some(label);
                    } else if this.hover_hint == Some(label) {
                        this.hover_hint = None;
                    }
                    cx.notify()
                });
                return div().id(SharedString::from(format!("h-{icon}")))
                    .w(px(w)).h(px(22.0)).px_1p5().rounded_sm()
                    .border_1().border_color(th_line)
                    .flex().items_center()
                    .text_size(px(10.5)).text_color(th_fg)
                    .whitespace_nowrap().overflow_hidden()
                    .on_hover(hoverable)
                    .cursor_pointer().hover(move |st| st.bg(th_btn_hover))
                    .child(val)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.run_cmd(cid, cx);
                        cx.notify()
                    }))
                    .into_any_element();
            }
            let has_icon = ui::icons::find(icon).is_some();
            let big = BIG.iter().find(|(k, _)| *k == icon).map(|(_, s)| *s);
            // 名札の短い形は ja 向け — 他の言語では表の語を使う
            let big = if ui::settings::language() == "ja" {
                big
            } else {
                big.map(|_| cmd.label)
            };
            let hoverable = cx.listener(move |this: &mut Calc, on: &bool, _, cx| {
                if *on {
                    this.hover_hint = Some(label);
                } else if this.hover_hint == Some(label) {
                    this.hover_hint = None;
                }
                cx.notify()
            });
            let fg = if cmd.ready { th_fg } else { th_gray };
            if let Some(short) = big {
                // 名札つきの大釦(絵の下に短い名前 — 本家の言い方)
                let mut b = div().id(SharedString::from(format!("h-{icon}")))
                    .px_2().h(px(46.0)).rounded_sm()
                    .flex().flex_col().items_center().justify_center().gap_1()
                    .on_hover(hoverable)
                    .children(has_icon.then(|| {
                        gpui::svg()
                            .path(SharedString::from(format!("icons/{icon}.svg")))
                            .size(px(20.0)).text_color(fg)
                    }))
                    .child(div().text_size(px(10.5)).text_color(fg).child(short));
                if cmd.ready {
                    let cid = cmd.id;
                    b = b.cursor_pointer().hover(move |st| st.bg(th_btn_hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.run_cmd(cid, cx);
                            cx.notify()
                        }));
                }
                return b.into_any_element();
            }
            let mut b = div().id(SharedString::from(format!("h-{icon}")))
                .h(px(26.0)).rounded_sm()
                .flex().items_center().justify_center()
                .on_hover(hoverable);
            b = if has_icon { b.w(px(26.0)) } else { b.px_1p5() };
            b = b
                .children(has_icon.then(|| {
                    gpui::svg()
                        .path(SharedString::from(format!("icons/{icon}.svg")))
                        .size(px(18.0)).text_color(fg)
                }))
                .children((!has_icon).then(|| {
                    div().text_size(px(10.5)).text_color(fg).child(label)
                }));
            if cmd.ready {
                let cid = cmd.id;
                b = b.cursor_pointer().hover(move |st| st.bg(th_btn_hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.run_cmd(cid, cx);
                        cx.notify()
                    }));
            }
            b.into_any_element()
        };
        if ribbon::CALC[self.tab].name == "ホーム" {
            // 本家のホームは**単純な2行割りではない**(発注者 2026-08-06
            // スクショ)。組ごとに上の段と下の段が対になっている —
            // コピーの下に貼り付け、書体の下に B I U…、縦揃えの下に横揃え。
            // その対をそのまま書き、組の間に縦の区切り線を引く
            const HOME_PAIRS: &[(&[&str], &[&str])] = &[
                (&["copy", "cut"], &["paste"]),
                (&["fontname", "fontsize", "incfont", "decfont", "changecase"],
                 &["bold", "italic", "underline", "strikeout", "subscript",
                   "fontcolor", "fillparag", "borders"]),
                (&["top", "middle", "bottom", "wrap", "text-orient"],
                 &["align-left", "align-center", "align-right", "align-just",
                   "merge", "direction"]),
                (&["insert-function", "fill-num"], &["defname", "clear"]),
                (&["sort-desc", "sort-asc"], &["setfilter", "clear-filter"]),
                (&["format", "currency", "percents"],
                 &["comma", "digit-dec", "digit-inc"]),
                (&["cell-ins", "cell-del", "cell-format"],
                 &["condformat", "table-tpl", "cell-styles"]),
                (&["replace"], &["selectall"]),
            ];
            let mut used: std::collections::HashSet<&str> = Default::default();
            let mut band = div().flex().flex_row().items_center().gap_1();
            let mut first = true;
            for (topr, botr) in HOME_PAIRS {
                if topr.iter().chain(botr.iter())
                    .all(|id| !items.iter().any(|c| c.id == *id))
                {
                    continue; // 表に無い組は出さない(将来の並び替えでも落ちない)
                }
                if !first {
                    band = band.child(div().w(px(1.0)).h(px(46.0))
                        .bg(th_cmd_border).mx_1());
                }
                first = false;
                let mut col = div().flex().flex_col().gap_0p5();
                for ids in [*topr, *botr] {
                    let mut r = div().flex().flex_row().items_center()
                        .gap_0p5().h(px(26.0));
                    for id in ids {
                        if let Some(cmd) = items.iter().find(|c| c.id == *id) {
                            used.insert(cmd.id);
                            r = r.child(mk_btn(cmd, cx));
                        }
                    }
                    col = col.child(r);
                }
                band = band.child(col);
            }
            // 対の表に無い釦も**黙って落とさない** — 右端に半々で足す
            let rest: Vec<&ribbon::Cmd> =
                items.iter().filter(|c| !used.contains(c.id)).collect();
            if !rest.is_empty() {
                band = band.child(div().w(px(1.0)).h(px(46.0))
                    .bg(th_cmd_border).mx_1());
                let half = rest.len().div_ceil(2);
                let mut col = div().flex().flex_col().gap_0p5();
                for chunk in rest.chunks(half.max(1)) {
                    let mut r = div().flex().flex_row().items_center()
                        .gap_0p5().h(px(26.0));
                    for cmd in chunk {
                        r = r.child(mk_btn(cmd, cx));
                    }
                    col = col.child(r);
                }
                band = band.child(col);
            }
            cmds = cmds.child(band);
        } else {
            let mut row = div().flex().flex_row().items_center().gap_0p5();
            for cmd in items {
                row = row.child(mk_btn(cmd, cx));
            }
            cmds = cmds.child(row);
        }
        let bar = if self.tab == 0 {
            // ファイルの全面ページは釦の帯を持たない(本家の形)
            div().flex().flex_col().child(top).child(tabs)
        } else {
            div().flex().flex_col().child(top).child(tabs).child(cmds)
        };

        // ---- 数式バー ----
        // クリックで**編集モード**(発注者 2026-08-06)— 置き換えでなく、
        // 押した位置に文字カーソルを立てて続きを直せる。編集中はキャレットを見せる
        let in_edit = self.editing() || self.edit_armed;
        let bar_text = {
            let mut t = self.input.text().to_string();
            if in_edit {
                let cur = self.input.cursor().min(t.len());
                t.insert(cur, '|');
            }
            if t.is_empty() { " ".to_string() } else { t }
        };
        // 名前ボックス(左端): 押すと打てる。番地・範囲・名前で飛び、
        // 知らない名前ならいまの選択に付ける(Excel の名前ボックス)
        let name_box = if let Some(ed) = &self.name_edit {
            let mut t = ed.text().to_string();
            let cur = ed.cursor().min(t.len());
            t.insert(cur, '|');
            div().w(px(88.0)).px_1().py_0p5().bg(gpui::white())
                .border_1().border_color(rgb(0x1B6E3C)).rounded_sm()
                .text_size(px(12.0)).whitespace_nowrap().overflow_hidden()
                .child(SharedString::from(t))
        } else {
            div().w(px(88.0)).px_1().py_0p5()
                .border_1().border_color(rgb(0xC6CDD3)).rounded_sm()
                .text_size(px(12.0))
                .font_weight(gpui::FontWeight::BOLD).text_color(rgb(0x1B6E3C))
                .cursor_text()
                .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.name_edit = Some(Editor::new(""));
                    this.status = ui::t!(
                        "名前ボックス: 番地(B12)・範囲(A1:C9)・名前で移動。\
                         知らない名前は選択に付きます")
                    .into();
                    cx.notify();
                }))
                .child(SharedString::from(self.cursor.a1()))
        };
        let formula_bar = div()
            .flex().flex_row().items_center().gap_2()
            .px_4().py_1p5().bg(rgb(0xFAFBFC))
            .border_b_1().border_color(rgb(0xE1E6EA))
            .child(name_box)
            // fx = 関数を挿入(本家と同じ場所)。幅は固定 —
            // 数式編集のクリック位置の換算(下の 156px)が崩れないように
            .child(div().id("fx").w(px(28.0)).py_0p5().rounded_sm()
                   .flex().items_center().justify_center()
                   .text_size(px(13.0)).italic()
                   .font_weight(gpui::FontWeight::BOLD).text_color(rgb(0x1B6E3C))
                   .cursor_pointer().hover(|s| s.bg(rgb(0xE4EFE8)))
                   .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                       cx.stop_propagation();
                       this.fn_dlg = Some(FnDlg {
                           search: Editor::new(""),
                           group: 0,
                           sel: 0,
                       });
                       cx.notify();
                   }))
                   .child("fx"))
            .child(div().flex_1().px_2().py_1().bg(gpui::white())
                   .border_1().border_color(if in_edit { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                   .rounded_sm()
                   .text_size(px(13.0)).font_family("Noto Sans JP")
                   .cursor_text()
                   .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                       |this, e: &gpui::MouseDownEvent, _, cx| {
                           cx.stop_propagation();
                           // 押した位置へ文字カーソル(幅は 全角=1em・半角=0.5em の見積り)。
                           // 起点 = 左余白16 + 名前ボックス88 + 隙間8 + fx 28 + 隙間8 + 内余白8
                           let x = f32::from(e.position.x)
                               - (16.0 + 88.0 + 8.0 + 28.0 + 8.0 + 8.0);
                           let text = this.input.text().to_string();
                           let mut acc = 0.0;
                           let mut at = text.len();
                           for (i, ch) in text.char_indices() {
                               let w = if (ch as u32) < 0x2E80 { 6.8 } else { 13.0 };
                               if acc + w / 2.0 > x {
                                   at = i;
                                   break;
                               }
                               acc += w;
                           }
                           this.input.move_to(at, false);
                           this.edit_armed = true;
                           this.status =
                               ui::t!("数式バーで編集: Enter で確定 / Esc で取消").into();
                           cx.notify();
                       }))
                   .child(SharedString::from(bar_text)));

        // ---- 折り返しの無い文字の、隣の空セルへのはみ出し(Excel の流儀) ----
        // 折り返し・縮小・回転・右横書きでない文字のセルで、伸びる方向の
        // 隣が空(値も式も無い)なら、そのセルの上にも描く(発注者 2026-08-06)。
        // 描くのは格子の後の重ね描き(spill_texts)で、セル側は文字を出さない
        let vis_cols: Vec<u32> = self.visible_cols();
        let mut spill_from: std::collections::HashSet<Pos> = Default::default();
        let mut spill_texts: Vec<gpui::Div> = Vec::new();
        if !self.show_formulas {
            let mut y = ROW_H;
            for r in self.visible_rows() {
                let rh = self.row_px(r);
                let mut x = HEAD_W;
                for (ci, &c) in vis_cols.iter().enumerate() {
                    let w = self.col_px(c);
                    let p = Pos::new(r, c);
                    let x0 = x;
                    x += w;
                    if p == self.cursor {
                        continue; // 編集中の見た目は従来どおり
                    }
                    let Some(cl) = self.sheet().get(p) else { continue };
                    let Value::Text(t) = &cl.value else { continue };
                    if t.is_empty() {
                        continue;
                    }
                    let f = &cl.fmt;
                    if f.wrap || f.shrink || f.rtl_text
                        || f.rotation.is_some_and(|r| r != 0)
                    {
                        continue;
                    }
                    if self.sheet().covered_by_merge(p)
                        || self.sheet().merges.iter().any(|(a, _)| *a == p)
                    {
                        continue;
                    }
                    let to_left = match f.align {
                        HAlign::Right => true,
                        HAlign::Left | HAlign::General => false,
                        _ => continue, // 中央・両端揃えは流さない
                    };
                    let t1 = t.replace('\n', " ");
                    let size = self.zoom
                        * f.size_c
                            .map(|c| c as f32 / 100.0 * 24.0 / 15.0 * 0.8)
                            .unwrap_or(12.5);
                    let units: f32 = t1
                        .chars()
                        .map(|ch| if (ch as u32) < 0x2E80 { 1.0 } else { 2.0 })
                        .sum();
                    let need = units * size * 0.52 + 14.0;
                    if need <= w {
                        continue; // 収まっている
                    }
                    // 伸びる方向の空きセルぶんだけ許す
                    let (mut avail, mut left_ext, mut k) = (w, 0.0f32, ci);
                    loop {
                        if need <= avail {
                            break;
                        }
                        let nk = if to_left {
                            k.checked_sub(1)
                        } else {
                            (k + 1 < vis_cols.len()).then_some(k + 1)
                        };
                        let Some(nk) = nk else { break };
                        let nc = vis_cols[nk];
                        let np = Pos::new(r, nc);
                        let occupied = self
                            .sheet()
                            .get(np)
                            .is_some_and(|q| !q.value.is_empty() || q.formula.is_some())
                            || self.sheet().covered_by_merge(np)
                            || np == self.cursor;
                        if occupied {
                            break;
                        }
                        let nw = self.col_px(nc);
                        avail += nw;
                        if to_left {
                            left_ext += nw;
                        }
                        k = nk;
                    }
                    if avail <= w {
                        continue; // 隣が塞がっている — 今までどおり切る
                    }
                    spill_from.insert(p);
                    let wd = avail.min(need);
                    let lx = if to_left { x0 + w - wd } else { x0 };
                    let _ = left_ext;
                    let mut d = div().absolute()
                        .left(px(lx)).top(px(y))
                        .w(px(wd)).h(px(rh))
                        .px_1p5().flex()
                        .text_size(px(size))
                        .font_family("Noto Sans JP")
                        .whitespace_nowrap().overflow_hidden();
                    match f.valign {
                        sheet::model::VAlign::Top => d = d.items_start(),
                        sheet::model::VAlign::Middle => d = d.items_center(),
                        sheet::model::VAlign::Bottom => d = d.items_end(),
                    }
                    d = if to_left { d.justify_end() } else { d.justify_start() };
                    if f.bold {
                        d = d.font_weight(gpui::FontWeight::BOLD);
                    }
                    if f.italic {
                        d = d.italic();
                    }
                    d = if let Some(cv) = &f.color {
                        d.text_color(hex(cv))
                    } else {
                        d.text_color(rgb(0x1B1B1B))
                    };
                    if let Some(name) = &f.font {
                        if let Ok((fam, _)) = kumihan::font::for_document(Some(name)) {
                            d = d.font_family(SharedString::from(fam.name.clone()));
                        }
                    }
                    spill_texts.push(d.child(SharedString::from(t1)));
                }
                y += rh;
            }
        }

        // ---- 格子 ----
        let mut grid = div().flex().flex_col();
        // 列見出し
        // 見出しもセルも flex_none — **窓の大きさで伸縮させない**
        // (窓に合わせるのは見える範囲。セルの大きさは設定どおり固定)
        let mut head = div().flex().flex_row().flex_none()
            .child(div().flex_none().w(px(HEAD_W)).h(px(ROW_H)).bg(th_head)
                   .border_r_1().border_b_1().border_color(rgb(0xD5DBE0)));
        let (sel_a, sel_b) = self.sel_rect();
        let has_sel = self.anchor.is_some();
        for c in self.visible_cols() {
            // 選択に入っている列の見出しは色を変える(いまどこを選んでいるかの道標)
            let on = has_sel && (sel_a.col..=sel_b.col).contains(&c) || c == self.cursor.col;
            head = head.child(div().flex_none().w(px(self.col_px(c))).h(px(ROW_H))
                .bg(if on { rgb(0xCFE6D8) } else { th_head })
                .border_r_1().border_b_1()
                .border_color(rgb(0xD5DBE0))
                .flex().items_center().justify_center()
                .text_size(px(11.5))
                .text_color(if on { rgb(0x1B6E3C) } else if dk { rgb(0x9AA5AE) } else { rgb(0x66707A) })
                .child(SharedString::from(col_name(c)))
                // 右端の帯は幅を変える取っ手(カーソル形状の誘いだけ。
                // 当たり判定は InputSink の窓レベルで size_grip_at がやる)
                .relative().children((std::env::var_os("JO_NO_STRIPS").is_none()).then(|| {
                    div().absolute()
                        .top(px(0.0)).right(px(-GRIP)).w(px(GRIP * 2.0)).h_full()
                        .cursor_col_resize()
                })));
        }
        grid = grid.child(head);

        // 当たり判定(cell_at)と同じ並びを使う — ずれるとクリックが別のセルに入る
        let visible: Vec<u32> = self.visible_rows();
        for r in visible {
            let rh = self.row_px(r);
            let row_on = has_sel && (sel_a.row..=sel_b.row).contains(&r) || r == self.cursor.row;
            let mut row = div().flex().flex_row().flex_none()
                .child(div().flex_none().w(px(HEAD_W)).h(px(rh))
                    .bg(if row_on { rgb(0xCFE6D8) } else { th_head })
                    .border_r_1().border_b_1()
                    .border_color(rgb(0xD5DBE0))
                    .flex().items_center().justify_center()
                    .text_size(px(11.5))
                    .text_color(if row_on { rgb(0x1B6E3C) } else if dk { rgb(0x9AA5AE) } else { rgb(0x66707A) })
                    .child(SharedString::from((r + 1).to_string()))
                    // 下端の帯は高さを変える取っ手(列見出しの右端と同じ仕掛け)
                    .relative().children((std::env::var_os("JO_NO_STRIPS").is_none()).then(|| {
                        div().absolute()
                            .left(px(0.0)).bottom(px(-GRIP)).w_full().h(px(GRIP * 2.0))
                            .cursor_row_resize()
                    }))
                    // グループ化の +/-(アウトラインの縁)。直前で終わる
                    // かたまりの頭金の行に置く(Excel の「集計行が下」の形)
                    .children({
                        let sh = self.sheet();
                        r.checked_sub(1).and_then(|pr| {
                            let lv = *sh.row_outline.get(&pr).unwrap_or(&0);
                            // かたまりが r の直前で**終わっている**ときだけ
                            // (続きの行に印を出さない)
                            if lv == 0 || *sh.row_outline.get(&r).unwrap_or(&0) >= lv {
                                return None;
                            }
                            let mut start = pr;
                            while start > 0
                                && *sh.row_outline.get(&(start - 1)).unwrap_or(&0) >= lv
                            {
                                start -= 1;
                            }
                            let hidden = sh.row_hidden.contains(&pr);
                            Some(div()
                                .id(SharedString::from(format!("gut{r}")))
                                .absolute().left(px(1.0)).top(px((rh - 11.0) / 2.0))
                                .w(px(11.0)).h(px(11.0)).rounded_sm()
                                .border_1().border_color(rgb(0x8FA3AE))
                                .bg(gpui::white())
                                .flex().items_center().justify_center()
                                .text_size(px(9.0)).text_color(rgb(0x1B6E3C))
                                .cursor_pointer()
                                .hover(|s| s.bg(rgb(0xEAF5EE)))
                                .child(if hidden { "+" } else { "−" })
                                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation()
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.checkpoint();
                                    for i in start..=pr {
                                        if hidden {
                                            this.sheet_mut().row_hidden.remove(&i);
                                        } else {
                                            this.sheet_mut().row_hidden.insert(i);
                                        }
                                    }
                                    this.dirty = true;
                                    this.status = if hidden {
                                        ui::t!("詳細を表示しました(+/− でいつでも)").into()
                                    } else {
                                        ui::t!("詳細を畳みました(+ で開きます)").into()
                                    };
                                    cx.notify()
                                })))
                        })
                    }));
            for c in self.visible_cols() {
                let p = Pos::new(r, c);
                let cell = self.sheet().get(p);
                // 結合に呑まれた位置は空で描く(値は左上のセルにだけある)
                let v = if self.sheet().covered_by_merge(p) { Value::Empty }
                        else { cell.map(|x| x.value.clone()).unwrap_or(Value::Empty) };
                // 付けた表示形式は画面に出す。出ないなら飾りでしかない
                let shown = if self.show_formulas {
                    // 数式の表示。式が無いセルは値のまま
                    cell.and_then(|x| x.formula.clone())
                        .map(|f| format!("={f}"))
                        .unwrap_or_else(|| sheet::model::format_value(&v,
                            cell.and_then(|x| x.fmt.number_format.as_deref())))
                } else {
                    sheet::model::format_value(&v, cell.and_then(|x| x.fmt.number_format.as_deref()))
                };
                // Bool のセルはチェックボックスとして見せる(☑/☐。
                // 空白キーで切替。Excel では TRUE/FALSE の値で見える)
                let shown = match v {
                    Value::Bool(b) if !self.show_formulas => {
                        if b { "☑".to_string() } else { "☐".to_string() }
                    }
                    _ => shown,
                };
                let shown = if !self.show_zeros && matches!(v, Value::Number(n) if n == 0.0) {
                    String::new()
                } else {
                    shown
                };
                let is_num = matches!(v, Value::Number(_));
                let is_err = matches!(v, Value::Error(_));
                let sel = p == self.cursor;
                let (ra, rb) = self.sel_rect();
                let in_range = self.anchor.is_some()
                    && (ra.row..=rb.row).contains(&r) && (ra.col..=rb.col).contains(&c);
                let mut d = div()
                    .id(SharedString::from(p.a1()))
                    .flex_none()
                    .w(px(self.col_px(c))).h(px(rh))
                    .border_r_1().border_b_1()
                    .border_color(if self.gridlines { rgb(0xE1E6EA) } else { rgb(0xFFFFFF) })
                    .bg(rgb(0xFFFFFF))
                    .flex().items_center()
                    .px_1p5()
                    .text_size(px(self.zoom * cell.and_then(|x| x.fmt.size_c)
                        .map(|c| c as f32 / 100.0 * 24.0 / 15.0 * 0.8)
                        .unwrap_or(12.5)))
                    .font_family("Noto Sans JP")
                    .overflow_hidden().whitespace_nowrap()
                    // セルの上は Excel と同じ十字(手のひらだと「押す物」に見える)
                    .cursor(gpui::CursorStyle::Crosshair);
                // マウスの結線はセルではなく InputSink(窓レベル)にある。
                // セルの id は当たり判定ではなく描画の区別のためだけに残す
                // 罫線・塗り・文字書式。**帳票の見た目はここで決まる**
                let f = cell.map(|x| x.fmt.clone()).unwrap_or_default();
                let mut base = f.fill.as_deref().map(hex).unwrap_or(gpui::Rgba {
                    r: 1.0, g: 1.0, b: 1.0, a: 1.0,
                });
                // 条件付き書式。**付けた条件は画面に出す**(出ないなら飾り)
                let mut cond_color: Option<gpui::Rgba> = None;
                for rule in &self.sheet().cond {
                    if rule.hits(p, &v) {
                        if let Some(fill) = &rule.fill {
                            base = hex(fill);
                        }
                        if let Some(c) = &rule.color {
                            cond_color = Some(hex(c));
                        }
                    }
                }
                d = d.bg(base);
                // 範囲は下地に緑を**混ぜて**見せる(塗りは透けて残る)。
                // 色を抜くのは**起点のセル**(最初に選んだ方)— ドラッグで
                // 動くのは反対側の角なので、抜けが動き回らない(Excel の作法)
                let origin = self.anchor.unwrap_or(self.cursor);
                if in_range && p != origin {
                    d = d.bg(tint(base, 0.20));
                }
                // トレースの光り(参照元=青緑、参照先=橙)。塗りは透けたまま
                if let Some((_, prec)) = self.trace.iter().find(|(tp, _)| *tp == p) {
                    d = d.bg(if *prec {
                        gpui::Rgba { r: base.r * 0.55 + 0.10, g: base.g * 0.55 + 0.38, b: base.b * 0.55 + 0.38, a: 1.0 }
                    } else {
                        gpui::Rgba { r: base.r * 0.55 + 0.43, g: base.g * 0.55 + 0.30, b: base.b * 0.55 + 0.08, a: 1.0 }
                    });
                }
                if f.bold {
                    d = d.font_weight(gpui::FontWeight::BOLD);
                }
                if f.italic {
                    d = d.italic();
                }
                // 下付きは小さく下げて見せる(xlsx へは vertAlign で入る)
                if f.subscript {
                    d = d.text_size(px(self.zoom * 8.5)).pt_2();
                }
                // 縦積み(255)は1字ずつ縦に並べる — 日本の帳票の縦の見出し。
                // 90/180 度は GPUI に字の回転が無いので、いまは縦積みで見せる
                if f.rotation.is_some_and(|r| r != 0) {
                    d = d.flex().flex_col().items_center();
                }
                if let Some(c) = &f.color {
                    d = d.text_color(hex(c));
                }
                // セルの書体。無い書体は系統を保って代替(明朝→明朝)
                if let Some(name) = &f.font {
                    if let Ok((fam, _)) = kumihan::font::for_document(Some(name)) {
                        d = d.font_family(SharedString::from(fam.name.clone()));
                    }
                }
                // 引いてある辺だけ濃くする(引いていない辺は表の薄い線のまま)。
                // border_color は div の**全辺に1色**なので使わない —
                // 使うと、外枠の上辺だけのセルで右・下の灰色の格子線まで
                // 黒くなり、外枠が格子に化ける(発注者報告)。
                // 辺ごとに細い帯を重ねて描く
                let ink = rgb(0x1B1B1B);
                if f.borders.top || f.borders.bottom || f.borders.left || f.borders.right {
                    d = d.relative();
                    if f.borders.top {
                        d = d.child(div().absolute().left(px(0.0)).top(px(0.0))
                            .w_full().h(px(1.0)).bg(ink));
                    }
                    if f.borders.bottom {
                        d = d.child(div().absolute().left(px(0.0)).bottom(px(0.0))
                            .w_full().h(px(1.0)).bg(ink));
                    }
                    if f.borders.left {
                        d = d.child(div().absolute().left(px(0.0)).top(px(0.0))
                            .w(px(1.0)).h_full().bg(ink));
                    }
                    if f.borders.right {
                        d = d.child(div().absolute().right(px(0.0)).top(px(0.0))
                            .w(px(1.0)).h_full().bg(ink));
                    }
                }
                // 太い枠は**選択の範囲の外周**に出す(Excel の作法)。
                // カーソルのセルに出すと、ドラッグ中は枠がマウスに付いて回る
                if self.anchor.is_some() {
                    if in_range {
                        let mut edge = false;
                        if r == ra.row { d = d.border_t_2(); edge = true }
                        if r == rb.row { d = d.border_b_2(); edge = true }
                        if c == ra.col { d = d.border_l_2(); edge = true }
                        if c == rb.col { d = d.border_r_2(); edge = true }
                        if edge {
                            d = d.border_color(rgb(0x1B6E3C));
                        }
                    }
                } else if sel {
                    d = d.border_2().border_color(rgb(0x1B6E3C));
                }
                // 縦の揃え(既定は下 = xlsx の既定)
                match f.valign {
                    sheet::model::VAlign::Top => d = d.items_start(),
                    sheet::model::VAlign::Middle => d = d.items_center(),
                    sheet::model::VAlign::Bottom => d = d.items_end(),
                }
                if f.wrap {
                    d = d.whitespace_normal().overflow_hidden();
                }
                // 縮小して全体を表示(折り返しと併せない)— 幅に収まるまで
                // 文字を小さくする。見積りは全角=1em・半角=0.5em
                if f.shrink && !f.wrap {
                    let size = self.zoom
                        * f.size_c
                            .map(|c| c as f32 / 100.0 * 24.0 / 15.0 * 0.8)
                            .unwrap_or(12.5);
                    let units: f32 = shown
                        .chars()
                        .map(|ch| if (ch as u32) < 0x2E80 { 1.0 } else { 2.0 })
                        .sum();
                    let need = units * size * 0.52 + 14.0;
                    let cw = self.col_px(c);
                    if need > cw && units > 0.0 {
                        d = d.text_size(px((size * cw / need).max(6.0)));
                    }
                }
                // 揃えの指定があればそちらが勝つ(既定は数=右・文字=左)
                match f.align {
                    HAlign::Left => d = d.justify_start(),
                    HAlign::Center => d = d.justify_center(),
                    HAlign::Right => d = d.justify_end(),
                    HAlign::Justify => d = d.justify_between(),
                    HAlign::General => {}
                }
                if is_num && f.align == HAlign::General {
                    d = d.justify_end();
                }
                // 文字色の優先順: エラー > リンク > 条件 > セルの色 > 既定
                // (以前は最後に既定色で上書きしていて、セルの文字色が死んでいた)
                if is_err {
                    d = d.text_color(rgb(0xB3261E));
                } else if self.sheet().links.contains_key(&p) {
                    // リンクのあるセルは青(Ctrl+クリックで開く)
                    d = d.text_color(rgb(0x1F4E79));
                } else if let Some(c) = cond_color {
                    d = d.text_color(c);
                } else if f.color.is_none() {
                    d = d.text_color(rgb(0x1B1B1B));
                }
                // コメントのあるセルは右上に赤い角印(表示を消していれば出さない)
                if self.show_comments && self.sheet().comments.contains_key(&p) {
                    d = d.relative().child(div().absolute()
                        .top(px(1.0)).right(px(1.0))
                        .w(px(6.0)).h(px(6.0)).rounded_sm().bg(rgb(0xC00000)));
                }
                // 入力規則のあるセルを選ぶと右下に ▾
                // (右クリック → ドロップダウンリストから選択、の目印)
                if sel && self.sheet().validation_at(p).is_some() {
                    d = d.relative().child(div().absolute()
                        .bottom(px(-1.0)).right(px(1.0))
                        .text_size(px(8.5)).text_color(rgb(0x1B6E3C))
                        .child("▾"));
                }
                // 選択中のセルは、確定前の入力をその場に見せる
                let shown = if sel { self.input.text().to_string() } else { shown };
                // はみ出しで描くセルは、ここでは文字を出さない(二重描き防止)。
                // 折り返しの無いセルは改行を畳んで1行にする(発注者 2026-08-06)
                let shown = if spill_from.contains(&p) {
                    String::new()
                } else if !f.wrap && shown.contains('\n') {
                    shown.replace('\n', " ")
                } else {
                    shown
                };
                if f.rotation.is_some_and(|r| r != 0) {
                    let mut stack = d;
                    for ch in shown.chars() {
                        stack = stack.child(SharedString::from(ch.to_string()));
                    }
                    row = row.child(stack);
                } else if f.rtl_text {
                    // 右横書き: 1字ずつ右から並べる(昔の看板の書き方)。
                    // ラテン文字の bidi は扱わない — 日本語の右横書きのため
                    let rev: String = shown.chars().rev().collect();
                    row = row.child(d.justify_end().child(SharedString::from(rev)));
                } else {
                    row = row.child(d.child(SharedString::from(shown)));
                }
            }
            grid = grid.child(row);
        }
        // はみ出しの文字は格子の後に重ねる = 隣のセルの白地に負けない
        if !spill_texts.is_empty() {
            grid = grid.relative();
            for sp in spill_texts {
                grid = grid.child(sp);
            }
        }

        // ---- シートの耳(Excel と同じく下に置く) ----
        let mut sheets_bar = div().flex().flex_row().items_center().gap_1()
            .px_3().py_1().bg(th_head)
            .border_t_1().border_color(rgb(0xD5DBE0));
        for (i, s) in self.book.sheets.iter().enumerate() {
            if s.hidden {
                continue; // 隠したシートは耳に出さない(表示タブで戻す)
            }
            let on = i == self.active;
            // 耳の色(xlsx の tabColor)。活きている耳は白のまま、色は縁に出す
            let tabc = s.tab_color.as_deref().and_then(|h| {
                let h6 = h.get(h.len().saturating_sub(6)..)?;
                h6.chars().all(|c| c.is_ascii_hexdigit()).then(|| hex(h6))
            });
            let dark_bg = tabc
                .map(|c| c.r * 0.299 + c.g * 0.587 + c.b * 0.114 < 0.55)
                .unwrap_or(false);
            sheets_bar = sheets_bar.child(div()
                .id(SharedString::from(format!("sheet{i}")))
                .px_3().py_1().rounded_sm()
                .bg(match (on, tabc) {
                    (true, _) => rgb(0xFFFFFF),
                    (false, Some(c)) => c,
                    (false, None) => rgb(0xEFF2F4),
                })
                .border_1().border_color(match (on, tabc) {
                    (_, Some(c)) => c,
                    (true, None) => rgb(0x1B6E3C),
                    (false, None) => rgb(0xD5DBE0),
                })
                .text_size(px(11.5))
                .text_color(if on {
                    rgb(0x1B6E3C)
                } else if dark_bg {
                    rgb(0xFFFFFF)
                } else {
                    rgb(0x66707A)
                })
                .font_weight(if on { gpui::FontWeight::BOLD } else { gpui::FontWeight::NORMAL })
                .cursor_pointer().hover(|s| s.bg(gpui::white()))
                .child(SharedString::from(format!(
                    "{}{}",
                    if s.protected { "🔒" } else { "" },
                    s.name
                )))
                // ダブルクリックで名前の変更(本家と同じ)。1度目は普通の切り替え
                .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                    move |this, e: &gpui::MouseDownEvent, _, cx| {
                        if e.click_count >= 2 {
                            cx.stop_propagation();
                            this.sheet_menu_at = Some(i);
                            let cur = this.book.sheets[i].name.clone();
                            this.prompt = Some(("sheet-rename", Editor::new(&cur)));
                            cx.notify();
                        }
                    }))
                // 右クリックで耳のメニュー(挿入・削除・名前の変更・…)
                .on_mouse_down(gpui::MouseButton::Right, cx.listener(
                    move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.open_sheet_menu(i);
                        cx.notify()
                    }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.switch_sheet(i);
                    cx.notify()
                })));
        }
        sheets_bar = sheets_bar.child(div()
            .id("addsheet")
            .px_2().py_1().rounded_sm()
            .text_size(px(12.5)).text_color(rgb(0x1B6E3C))
            .cursor_pointer().hover(|s| s.bg(gpui::white()))
            .child("+")
            .on_click(cx.listener(|this, _, _, cx| {
                this.add_sheet();
                cx.notify()
            })));
        // 描きかけの1筆(点の粒で見せる。離すと1本の線になる)
        let ink_preview: Vec<gpui::AnyElement> = self
            .ink_cur
            .as_ref()
            .map(|pts| {
                let marker = self.tool == Some(1);
                let (sz, col) = if marker {
                    (9.0, rgb(0xFFD54A))
                } else {
                    (2.5, rgb(0x1B1B1B))
                };
                pts.iter()
                    .map(|(x, y)| {
                        div()
                            .absolute()
                            .left(px(x - sz / 2.0))
                            .top(px(y - sz / 2.0))
                            .w(px(sz))
                            .h(px(sz))
                            .rounded_full()
                            .bg(col)
                            .into_any_element()
                    })
                    .collect()
            })
            .unwrap_or_default();

        // 見張り(ウォッチウィンドウ)。控えたセルの値を下に並べる
        let watch_bar = (!self.watch.is_empty()).then(|| {
            let mut w = div().flex().flex_row().flex_wrap().gap_3()
                .px_3().py_1().bg(rgb(0xF7F9FA))
                .border_t_1().border_color(rgb(0xD5DBE0))
                .text_size(px(11.0)).text_color(rgb(0x1B1B1B));
            w = w.child(div().font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(0x1B6E3C)).child(ui::t!("見張り")));
            for (si, p) in self.watch.iter().take(24) {
                let Some(sh) = self.book.sheets.get(*si) else { continue };
                let v = sh.get(*p).map(|c| c.value.display()).unwrap_or_default();
                w = w.child(div().flex().flex_row().gap_1()
                    .child(div().text_color(rgb(0x66707A))
                        .child(SharedString::from(format!("{}!{}", sh.name, p.a1()))))
                    .child(div().font_weight(gpui::FontWeight::BOLD)
                        .child(SharedString::from(v))));
            }
            w
        });

        // 下端はステータスバーを兼ねる(デスクトップ版の形):
        // 状態の文言と、選択の生きた値(合計・平均・個数)
        sheets_bar = sheets_bar
            .child(div().pl_3().text_size(px(11.0)).text_color(rgb(0x66707A))
                .whitespace_nowrap().overflow_hidden()
                .child(SharedString::from(match self.hover_hint {
                    // 釦に乗っている間はその名前(本家の作法)
                    Some(h) => h.to_string(),
                    None => format!(
                        "{}{}",
                        if self.dirty { "● " } else { "" },
                        self.status
                    ),
                })))
            .child(div().flex_1())
            .children(self.sel_stats().map(|s| {
                div().pr_2().text_size(px(11.0)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x1B6E3C)).whitespace_nowrap()
                    .child(SharedString::from(s))
            }));

        // ---- 右クリックのメニュー ----
        // **並びと名前は Euro-Office の右クリックメニューに合わせる**(リボンと
        // 同じ理由 — 乗り換える人が場所を覚え直さずに済む)。未実装は灰色。
        // AI・コメントなどの「入れないもの/まだ無いもの」も、場所だけは本家どおり。
        // InputSink より**後**に描く(bubble は後に登録した方が先に走るので、
        // 項目の stop_propagation が InputSink のセル選択より先に効く)
        let menu = self.menu_at.map(|(mx, my)| {
            // (id, 名前, 付記, 押せるか, 子メニューか)
            #[allow(clippy::type_complexity)]
            let entries: Vec<(&'static str, &'static str, &'static str, bool, bool)> = vec![
                ("cut", "切り取り", "Ctrl+X", true, false),
                ("copy", "コピー", "Ctrl+C", true, false),
                ("paste", "貼り付け", "Ctrl+V", true, false),
                // 本家(Euro-Office)に無いのが残念、との声で追加した唯一の独自項目
                ("pastesp", "形式を選択して貼り付け", "", true, true),
                ("", "", "", false, false),
                ("ins", "挿入", "", true, true),
                ("del", "削除", "", true, true),
                ("clr", "消去", "", true, true),
                ("", "", "", false, false),
                ("sort", "並べ替え", "", true, true),
                ("filter", "フィルター", "", true, true),
                ("reapply", "再適用", "", self.filter.is_some(), false),
                ("", "", "", false, false),
                ("addcomment", "コメントを追加", "", true, false),
                ("", "", "", false, false),
                ("fmtcells", "セルをフォーマットする", "", true, false),
                ("numfmt", "数値の書式", "", true, true),
                ("cond", "条件付き書式", "", true, true),
                ("picklist", "ドロップダウンリストから選択する", "", true, false),
                ("defname", "名前の定義", "", true, false),
                ("", "", "", false, false),
                ("func", "関数を挿入", "", true, true),
                ("hyperlink", "ハイパーリンク", "", true, false),
                ("", "", "", false, false),
                ("freeze", "枠の固定", "", true, false),
            ];
            // 画面の右・下で切れないように少し戻す
            const ITEM_H: f32 = 25.0;
            const SEP_H: f32 = 9.0;
            let h_est: f32 = entries.iter()
                .map(|e| if e.0.is_empty() && e.1.is_empty() { SEP_H } else { ITEM_H })
                .sum::<f32>() + 10.0;
            let grid_w = HEAD_W
                + self.visible_cols()
                    .iter()
                    .map(|c| self.col_px(*c))
                    .sum::<f32>();
            let grid_h = if self.view_h_px > 0.0 {
                self.view_h_px - 120.0
            } else {
                ROW_H + ROWS as f32 * ROW_H
            };
            let mx = mx.min((grid_w - 250.0).max(0.0));
            let my = my.min((grid_h - h_est).max(0.0));

            let mut m = div().absolute().left(px(mx)).top(px(my)).w(px(244.0))
                .p_1().rounded_md().bg(rgb(0xFFFFFF))
                .border_1().border_color(rgb(0xC6CDD3)).shadow_lg()
                // メニューの余白を押してもセルに抜けない
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation());
            // 開いている子メニューの縦位置(親項目の高さに合わせる)
            let mut sub_panel: Option<gpui::Div> = None;
            let mut y_acc = 4.0f32;
            for (i, (id, label, hint, ready, is_sub)) in entries.iter().enumerate() {
                let (id, label, hint, ready, is_sub) = (*id, *label, *hint, *ready, *is_sub);
                if id.is_empty() && label.is_empty() {
                    m = m.child(div().h(px(1.0)).my_1().bg(rgb(0xE1E6EA)));
                    y_acc += SEP_H;
                    continue;
                }
                let row_y = y_acc;
                y_acc += ITEM_H;
                if !ready {
                    // 未実装。押せるように見せない(場所だけ本家どおりに残す)
                    m = m.child(div()
                        .flex().flex_row().items_center().justify_between().gap_4()
                        .px_3().py_1()
                        .child(div().text_size(px(12.5)).text_color(rgb(0xB6BDC4))
                            .child(label))
                        .child(div().text_size(px(10.5)).text_color(rgb(0xD5DBE0))
                            .child(if is_sub { "▸" } else { hint })));
                    continue;
                }
                if is_sub {
                    let open = self.menu_sub == Some(id);
                    m = m.child(div()
                        .id(SharedString::from(format!("m{i}")))
                        .flex().flex_row().items_center().justify_between().gap_4()
                        .px_3().py_1().rounded_sm().cursor_pointer()
                        .bg(if open { rgb(0xEAF5EE) } else { rgb(0xFFFFFF) })
                        .hover(|s| s.bg(rgb(0xEAF5EE)))
                        .child(div().text_size(px(12.5)).text_color(rgb(0x1B1B1B))
                            .child(label))
                        .child(div().text_size(px(11.0)).text_color(rgb(0x66707A)).child("▸"))
                        // 触れたら開く(本家と同じ)。押しても開く
                        .on_mouse_move(cx.listener(move |this, _, _, cx| {
                            if this.menu_sub != Some(id) {
                                this.menu_sub = Some(id);
                                cx.notify();
                            }
                        }))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                            move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.menu_sub = Some(id);
                                cx.notify();
                            })));
                    if open {
                        // 子の板。親項目の右横に出す
                        let mut sp = div().absolute()
                            .left(px(mx + 244.0)).top(px(my + row_y))
                            .w(px(210.0)).p_1().rounded_md().bg(rgb(0xFFFFFF))
                            .border_1().border_color(rgb(0xC6CDD3)).shadow_lg()
                            .on_mouse_down(gpui::MouseButton::Left,
                                |_, _, cx| cx.stop_propagation());
                        for (j, (sid, slabel, sready)) in
                            self.menu_sub_entries(id).into_iter().enumerate()
                        {
                            if !sready {
                                sp = sp.child(div().px_3().py_1()
                                    .text_size(px(12.5)).text_color(rgb(0xB6BDC4))
                                    .child(slabel));
                                continue;
                            }
                            sp = sp.child(div()
                                .id(SharedString::from(format!("s{i}-{j}")))
                                .px_3().py_1().rounded_sm().cursor_pointer()
                                .hover(|s| s.bg(rgb(0xEAF5EE)))
                                .text_size(px(12.5)).text_color(rgb(0x1B1B1B))
                                .child(slabel)
                                .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                                    move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.menu_action(sid, window, cx);
                                    })));
                        }
                        sub_panel = Some(sp);
                    }
                    continue;
                }
                // 普通の項目
                m = m.child(div()
                    .id(SharedString::from(format!("m{i}")))
                    .flex().flex_row().items_center().justify_between().gap_4()
                    .px_3().py_1().rounded_sm().cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF5EE)))
                    .child(div().text_size(px(12.5)).text_color(rgb(0x1B1B1B))
                        .child(label))
                    .child(div().text_size(px(10.5)).text_color(rgb(0x9AA5AE)).child(hint))
                    // 実行できる普通の項目に触れたら、開いていた子は閉じる
                    .on_mouse_move(cx.listener(move |this, _, _, cx| {
                        if this.menu_sub.is_some() {
                            this.menu_sub = None;
                            cx.notify();
                        }
                    }))
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                        move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.menu_action(id, window, cx);
                        })));
            }
            div().absolute().left(px(0.0)).top(px(0.0)).size_full()
                .child(m)
                .children(sub_panel)
        });

        // ---- 選択中の図形の枠と右下の掴み ----
        let shape_frame = self.shape_sel.and_then(|i| {
            let sp = self.sheet().shapes_new.get(i)?;
            let (x, y) = self.cell_origin_px(sp.at)?;
            let (x, y) = (x + sp.dx_px, y + sp.dy_px);
            Some(
                div()
                    .absolute()
                    .left(px(x - 2.0))
                    .top(px(y - 2.0))
                    .w(px(sp.width_px + 4.0))
                    .h(px(sp.height_px + 4.0))
                    .border_2()
                    .border_dashed()
                    .border_color(rgb(0x1B6E3C))
                    .child(
                        div()
                            .absolute()
                            .right(px(-1.0))
                            .bottom(px(-1.0))
                            .w(px(10.0))
                            .h(px(10.0))
                            .bg(rgb(0x1B6E3C))
                            .cursor_nwse_resize(),
                    ),
            )
        });

        // ---- 関数を挿入の小窓(本家の FormulaDialog の形) ----
        // 検索 / 分類 / 一覧(↑↓で選ぶ・ダブルクリックで入る)/ 引数と説明
        let fn_panel = self.fn_dlg.as_ref().map(|d| {
            let list = fn_filtered(d.search.text(), d.group);
            let sel = d.sel.min(list.len().saturating_sub(1));
            let mut search_t = d.search.text().to_string();
            let cur = d.search.cursor().min(search_t.len());
            search_t.insert(cur, '|');
            let mut chips = div().flex().flex_row().flex_wrap().gap_1();
            for (gi, g) in FN_GROUPS.iter().enumerate() {
                let on = gi == d.group;
                chips = chips.child(div()
                    .id(SharedString::from(format!("fng{gi}")))
                    .px_2().py_0p5().rounded_sm().text_size(px(11.5))
                    .border_1()
                    .border_color(if on { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                    .bg(if on { rgb(0xE4EFE8) } else { rgb(0xFFFFFF) })
                    .text_color(if on { rgb(0x1B6E3C) } else { rgb(0x66707A) })
                    .cursor_pointer()
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(d) = &mut this.fn_dlg {
                            d.group = gi;
                            d.sel = 0;
                        }
                        cx.notify();
                    }))
                    .child(SharedString::from(ui::tr(g))));
            }
            let start = sel.saturating_sub(5);
            let mut lst = div().flex().flex_col().h(px(252.0)).overflow_hidden()
                .border_1().border_color(rgb(0xC6CDD3)).rounded_sm().bg(rgb(0xFFFFFF));
            if list.is_empty() {
                lst = lst.child(div().px_2().py_1().text_size(px(12.5))
                    .text_color(rgb(0x66707A))
                    .child(ui::t!("その条件の関数がありません")));
            }
            for (i, f) in list.iter().enumerate().skip(start).take(11) {
                let on = i == sel;
                lst = lst.child(div()
                    .id(SharedString::from(format!("fnr{i}")))
                    .px_2().py_0p5().text_size(px(12.5)).flex_none()
                    .bg(if on { rgb(0x1B6E3C) } else { rgb(0xFFFFFF) })
                    .text_color(if on { rgb(0xFFFFFF) } else { rgb(0x1B1B1B) })
                    .cursor_pointer()
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                        move |this, e: &gpui::MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            if let Some(d) = &mut this.fn_dlg {
                                d.sel = i;
                            }
                            if e.click_count >= 2 {
                                this.fn_next();
                            }
                            cx.notify();
                        }))
                    .child(SharedString::from(f.name)));
            }
            let (syntax, desc) = list
                .get(sel)
                .map(|f| (format!("{}{}", f.name, f.args), f.desc.to_string()))
                .unwrap_or_default();
            let btn = |id: &'static str, label: String, primary: bool| {
                div().id(id).px_3().py_1().rounded_sm().text_size(px(12.5))
                    .border_1()
                    .border_color(if primary { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                    .bg(if primary { rgb(0x1B6E3C) } else { rgb(0xFFFFFF) })
                    .text_color(if primary { rgb(0xFFFFFF) } else { rgb(0x1B1B1B) })
                    .cursor_pointer()
                    .child(SharedString::from(label))
            };
            div().absolute().inset_0().flex().items_center().justify_center()
                .child(div().w(px(430.0)).p_3().rounded_md().bg(rgb(0xF7F9FA))
                    .border_1().border_color(rgb(0x1B6E3C)).shadow_lg()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .flex().flex_col().gap_1p5()
                    .child(div().flex().flex_row().items_center()
                        .child(div().text_size(px(13.0)).font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(0x1B6E3C)).child(ui::t!("関数を挿入")))
                        .child(div().flex_1())
                        .child(div().id("fn-x").px_2().cursor_pointer().text_size(px(13.0))
                            .text_color(rgb(0x66707A)).hover(|s| s.text_color(rgb(0xC0392B)))
                            .child("✕")
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.fn_dlg = None;
                                cx.notify();
                            }))))
                    .child(div().px_2().py_1().bg(rgb(0xFFFFFF))
                        .border_1().border_color(rgb(0xC6CDD3)).rounded_sm()
                        .text_size(px(12.5)).whitespace_nowrap().overflow_hidden()
                        .child(SharedString::from(if search_t == "|" {
                            format!("|{}", ui::t!("(打つと絞り込み)"))
                        } else {
                            search_t
                        })))
                    .child(chips)
                    .child(lst)
                    .child(div().text_size(px(12.5)).font_weight(gpui::FontWeight::BOLD)
                        .child(SharedString::from(syntax)))
                    .child(div().text_size(px(11.5)).text_color(rgb(0x4A545E))
                        .min_h(px(48.0))
                        .child(SharedString::from(desc)))
                    .child(div().flex().flex_row().gap_2().justify_center()
                        .child(btn("fn-next", ui::t!("次へ").to_string(), true)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.fn_next();
                                cx.notify();
                            })))
                        .child(btn("fn-cancel", ui::t!("キャンセル").to_string(), false)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.fn_dlg = None;
                                cx.notify();
                            })))))
        });

        // ---- 関数の引数の画面(本家の第2段) ----
        // 引数ごとの欄と説明、結果の下見。セルをクリックすると欄に参照が入る
        let fn_args_panel = self.fn_args.as_ref().map(|a| {
            let mut rows_el = div().flex().flex_col().gap_1();
            for (i, (name, opt)) in a.names.iter().enumerate() {
                let on = i == a.focus;
                let mut t = a.eds[i].text().to_string();
                if on {
                    let cur = a.eds[i].cursor().min(t.len());
                    t.insert(cur, '|');
                }
                rows_el = rows_el.child(div()
                    .id(SharedString::from(format!("fna{i}")))
                    .flex().flex_row().items_center().gap_2()
                    .cursor_text()
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(a) = &mut this.fn_args {
                            a.focus = i;
                        }
                        cx.notify();
                    }))
                    .child(div().w(px(110.0)).text_size(px(12.0))
                        .text_color(rgb(0x1B1B1B))
                        .child(SharedString::from(if *opt {
                            format!("{name}(省略可)")
                        } else {
                            name.clone()
                        })))
                    .child(div().flex_1().px_2().py_0p5().bg(rgb(0xFFFFFF))
                        .border_1()
                        .border_color(if on { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                        .rounded_sm().text_size(px(12.5))
                        .whitespace_nowrap().overflow_hidden()
                        .child(SharedString::from(if t.is_empty() { " ".into() } else { t }))));
            }
            // いまの欄の説明(本家の ad — 引数順。可変長は最後の1つが代表)
            let arg_hint = a
                .names
                .get(a.focus)
                .map(|(n, _)| {
                    let d = a.f.arg_desc.get(a.focus)
                        .or(a.f.arg_desc.last())
                        .copied()
                        .unwrap_or("");
                    format!("{n}: {d}")
                })
                .unwrap_or_default();
            let btn = |id: &'static str, label: String, primary: bool| {
                div().id(id).px_3().py_1().rounded_sm().text_size(px(12.5))
                    .border_1()
                    .border_color(if primary { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                    .bg(if primary { rgb(0x1B6E3C) } else { rgb(0xFFFFFF) })
                    .text_color(if primary { rgb(0xFFFFFF) } else { rgb(0x1B1B1B) })
                    .cursor_pointer()
                    .child(SharedString::from(label))
            };
            div().absolute().inset_0().flex().items_center().justify_center()
                .child(div().w(px(520.0)).p_3().rounded_md().bg(rgb(0xF7F9FA))
                    .border_1().border_color(rgb(0x1B6E3C)).shadow_lg()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .flex().flex_col().gap_1p5()
                    .child(div().flex().flex_row().items_center()
                        .child(div().text_size(px(13.0)).font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(0x1B6E3C)).child(ui::t!("関数の引数")))
                        .child(div().flex_1())
                        .child(div().id("fna-x").px_2().cursor_pointer().text_size(px(13.0))
                            .text_color(rgb(0x66707A)).hover(|s| s.text_color(rgb(0xC0392B)))
                            .child("✕")
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.fn_args = None;
                                cx.notify();
                            }))))
                    .child(div().text_size(px(12.5)).font_weight(gpui::FontWeight::BOLD)
                        .child(SharedString::from(format!("{}{}", a.f.name, a.f.args))))
                    .child(div().text_size(px(11.5)).text_color(rgb(0x4A545E))
                        .child(SharedString::from(a.f.desc)))
                    .child(rows_el)
                    .child(div().text_size(px(11.5)).text_color(rgb(0x4A545E))
                        .min_h(px(44.0)).px_2().py_1()
                        .bg(rgb(0xEFF2F4)).rounded_sm()
                        .child(SharedString::from(arg_hint)))
                    .child(div().text_size(px(12.0))
                        .child(SharedString::from(ui::tf!("関数の結果 = {}", a.result))))
                    .child(div().text_size(px(11.0)).text_color(rgb(0x66707A))
                        .child(ui::t!("セルをクリックすると、いまの欄に参照が入ります")))
                    .child(div().flex().flex_row().gap_2().justify_center()
                        .child(btn("fna-back", ui::t!("戻る").to_string(), false)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.fn_args = None;
                                this.fn_dlg = Some(FnDlg {
                                    search: Editor::new(""),
                                    group: 0,
                                    sel: 0,
                                });
                                cx.notify();
                            })))
                        .child(btn("fna-ok", ui::t!("OK").to_string(), true)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.fn_args_ok();
                                cx.notify();
                            })))
                        .child(btn("fna-cancel", ui::t!("キャンセル").to_string(), false)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.fn_args = None;
                                cx.notify();
                            })))))
        });

        // ---- 終了確認の板(窓の中の中央。rfd はスクリーン中央に出て遠い) ----
        let quit_panel = self.quit_ask.then(|| {
            let btn = |id: &'static str, label: String, primary: bool| {
                div().id(id).px_3().py_1().rounded_sm().text_size(px(12.5))
                    .border_1()
                    .border_color(if primary { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                    .bg(if primary { rgb(0x1B6E3C) } else { rgb(0xFFFFFF) })
                    .text_color(if primary { rgb(0xFFFFFF) } else { rgb(0x1B1B1B) })
                    .cursor_pointer()
                    .child(SharedString::from(label))
            };
            div().absolute().inset_0().flex().items_center().justify_center()
                .child(div().w(px(420.0)).p_3().rounded_md().bg(rgb(0xF7F9FA))
                    .border_1().border_color(rgb(0x1B6E3C)).shadow_lg()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .flex().flex_col().gap_2()
                    .child(div().text_size(px(13.0)).font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(0x1B6E3C))
                        .child(ui::t!("保存していない変更があります")))
                    .child(div().text_size(px(12.0))
                        .child(ui::t!("保存して終了しますか?(Enter = 保存して終了 / Esc = やめる)")))
                    .child(div().flex().flex_row().gap_2().justify_center()
                        .child(btn("q-save", ui::t!("保存して終了").to_string(), true)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.quit_ask = false;
                                this.save(true, cx);
                                cx.notify();
                            })))
                        .child(btn("q-drop", ui::t!("保存せず終了").to_string(), false)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.release_lock();
                                cx.quit();
                            })))
                        .child(btn("q-cancel", ui::t!("キャンセル").to_string(), false)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.quit_ask = false;
                                this.status = ui::t!("終了をやめました").into();
                                cx.notify();
                            })))))
        });

        // ---- コピーした範囲の破線(蟻の行進の静止版) ----
        // セルの罫線と混ざらないよう、重ね描きの1枚で囲む。マウスは受けない
        let ants = self.clip_range.and_then(|(si, a, b)| {
            if si != self.active {
                return None;
            }
            self.range_px(a, b).map(|(x0, y0, x1, y1)| {
                div().absolute()
                    .left(px(x0)).top(px(y0))
                    .w(px((x1 - x0).max(2.0))).h(px((y1 - y0).max(2.0)))
                    .border_2().border_dashed().border_color(rgb(0x1B6E3C))
            })
        });

        // ---- カーソルのセルの付記(コメント・リンク) ----
        let mut tip_lines: Vec<String> = Vec::new();
        if self.show_comments {
            if let Some(t) = self.sheet().comments.get(&self.cursor) {
                tip_lines.push(t.clone());
            }
        }
        if let Some(u) = self.sheet().links.get(&self.cursor) {
            tip_lines.push(ui::tf!("リンク: {}(Ctrl+クリックで開く)", u));
        }
        let tip = if tip_lines.is_empty() {
            None
        } else {
            self.cell_origin_px(self.cursor).map(|(x, y)| {
                let mut t = div().absolute()
                    .left(px(x + self.col_px(self.cursor.col) + 6.0))
                    .top(px(y))
                    .max_w(px(280.0)).p_2().rounded_md()
                    .bg(rgb(0xFFF9DB)).border_1().border_color(rgb(0xE0C97F)).shadow_lg();
                for line in tip_lines {
                    t = t.child(div().text_size(px(11.5)).text_color(rgb(0x5C4A00))
                        .child(SharedString::from(line)));
                }
                t
            })
        };

        // ---- 入力の板(名前の定義など) ----
        let prompt_panel = self.prompt.as_ref().map(|(kind, ed)| {
            let (a, b) = self.sel_rect();
            let range = if self.anchor.is_some() {
                format!("{}:{}", a.a1(), b.a1())
            } else {
                a.a1()
            };
            let title = match *kind {
                "name" => ui::tf!("名前の定義 — {} に名前を付ける", range),
                "comment" => ui::tf!("コメント — {}(空にして Enter で消す)", self.cursor.a1()),
                "link" => ui::tf!("ハイパーリンク — {}(空にして Enter で外す)", self.cursor.a1()),
                "cond-gt" => ui::tf!("条件付き書式 — {} で、いくつより大きい値を塗る?", range),
                "cond-lt" => ui::tf!("条件付き書式 — {} で、いくつより小さい値を塗る?", range),
                "validation" => ui::tf!("入力規則 — {} は候補から選ぶ(空にして Enter で解除)", range),
                "find" => ui::t!("検索と置換 — 探す言葉").to_string(),
                "split-delim" => ui::tf!("区切り位置 — {} を何で割る?(空 Enter = カンマ)", range),
                "shape-text" => ui::t!("図形の文字(空にして Enter で消す)").to_string(),
                "py" => ui::t!("Python — 一行のコード(空 Enter = .py ファイルを選ぶ)").to_string(),
                "goal-target" => ui::t!("ゴールシーク — 目標(セル=値。例: D6=800000)").to_string(),
                "goal-var" => ui::tf!("{} をいくつにするか探します — 変えるセルは?(例: B2)", self.goal.map(|(p, v)| format!("{}={v}", p.a1())).unwrap_or_default()),
                "replace-with" => ui::tf!("「{}」を何に置き換える?", self.find_term.as_deref().unwrap_or("")),
                "chat" => ui::t!("チャット — 言伝を書き残す(ブックの隣の .chat.txt)").to_string(),
                "equation" => ui::t!("方程式 — 式を打つ(TeX の書き方。清書して画像で置く)").to_string(),
                "ai-table" => ui::t!("AI — 表にする文章").to_string(),
                "ai-ask" => ui::t!("AI — 頼み(例: 合計の式を書いて)").to_string(),
                "table-resize" => ui::t!("テーブルのサイズ変更 — 新しい範囲(A1:C9)").to_string(),
                "prop-creator" => ui::t!("ブックの情報 — 作成者").to_string(),
                "prop-title" => ui::t!("ブックの情報 — タイトル").to_string(),
                "prop-keywords" => ui::t!("ブックの情報 — タグ").to_string(),
                "prop-subject" => ui::t!("ブックの情報 — 件名").to_string(),
                "prop-desc" => ui::t!("ブックの情報 — コメント").to_string(),
                "textart" => ui::t!("テキストアート — 飾り文字にする文字を打つ").to_string(),
                "pw-open" => ui::t!("暗号化されたブック — パスワード").to_string(),
                "pw-set" => ui::t!("暗号化 — パスワード(空にして Enter で暗号化をやめる)").to_string(),
                "sheet-rename" => ui::t!("シートの名前の変更").to_string(),
                "subtotal-by" => ui::t!("小計 1/2 — 何の区切りで集めるか(見出しを1つ)").to_string(),
                "subtotal-vals" => ui::t!("小計 2/2 — 合計する見出し").to_string(),
                "pivot-rows" => ui::t!("ピボット 1/3 — 行に並べる見出し(カンマ区切り可)").to_string(),
                "pivot-cols" => ui::t!("ピボット 2/3 — 列に広げる見出し(空 Enter = なし)").to_string(),
                "pivot-val" => ui::t!("ピボット 3/3 — 値にする見出しと集計").to_string(),
                _ => String::new(),
            };
            // キャレットは | で見せる(writer の検索欄と同じ割り切り)。
            // パスワードは伏せ字
            let mut text = if matches!(*kind, "pw-open" | "pw-set") {
                "●".repeat(ed.text().chars().count())
            } else {
                ed.text().to_string()
            };
            let cur = ed.cursor().min(text.len());
            text.insert(cur, '|');
            // 板は表の中央に出す(発注者 2026-08-06「表示位置を見直す」)。
            // 外側の受け皿は聞き手を持たない = 後ろのセルの操作を遮らない
            div().absolute().inset_0().flex().items_center().justify_center()
                .child(div().w(px(380.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0x1B6E3C)).shadow_lg()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(div().text_size(px(12.0)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x1B6E3C)).child(SharedString::from(title)))
                .child(div().mt_1p5().px_2().py_1().bg(rgb(0xFFFFFF))
                    .border_1().border_color(rgb(0xC6CDD3)).rounded_sm()
                    .text_size(px(13.0)).font_family("Noto Sans JP")
                    .child(SharedString::from(text)))
                .child(div().mt_1().text_size(px(10.5)).text_color(rgb(0x66707A))
                    .child(match *kind {
                        "name" => "Enter で決定 / Esc で取消。定義した名前は式の中で使えます(=単価*2)",
                        "validation" => "候補の直書き(甲,乙,丙)か、範囲の参照(=D2:D5)。Enter で決定 / Esc で取消",
                        "find" => "Enter で次へ / Esc で取消。式の中の文字も探します",
                        "split-delim" => "選択した列の文字を割って、右の列へ並べます(右は上書き)",
                        "shape-text" => "図形を選んで Enter でいつでも書き直せます",
                        "py" => "b=ブック s=シート / @計算 =PY(…)セルを評価 / @名前 実行 @名前 net @save @list @del",
                        "goal-target" | "goal-var" => "式のセルが目標の値になるよう、変えるセルの数を探します",
                        "replace-with" => "Enter で全て置き換え / **空のまま Enter = 検索だけ** / Esc で取消",
                        "chat" => "生放送ではありません — ファイル越しの言伝。最近の言伝は下の状態行に",
                        "equation" => "例: \\frac{a}{b} / \\sqrt{x^2+1} / \\sum_{i=1}^n i^2 / \\int_0^1 x\\,dx(計算はしません — セルの式とは別物)",
                        "textart" => "太字+縁取り(calc の緑)で描いて、画像としてシートに浮かべます",
                        "ai-table" => "答えのタブ区切りを、カーソルの位置の空きに流し込みます",
                        "ai-ask" => "= で始まる答えはカーソルに式として入ります。他はコメントに付きます",
                        "pw-open" => "間違えると開けません(板は残ります)。Esc で開くのをやめる",
                        "pw-set" => "次の保存から AES-128 で包みます。Excel や LibreOffice でも開けます",
                        "subtotal-by" => "使える見出しは下の状態行に出ています。並べ替えてから使うと区切りがまとまります",
                        "subtotal-vals" => "空のまま Enter = 数の列全部に入れます。畳んでも小計と総計は残ります",
                        "pivot-rows" | "pivot-cols" => "使える見出しは下の状態行に出ています。Enter で次へ / Esc で取消",
                        "pivot-val" => "例: 金額 合計。集計は 合計/平均/個数/最大/最小(省けば合計)",
                        _ => "Enter で決定 / Esc で取消",
                    })))
        });

        // ---- ソルバーの小窓(ONLYOFFICE の「ソルバーのパラメータ」の形) ----
        // モーダルにしない板たちと同じ作法。打鍵は focus の欄へ(HasEditor)
        let solver_panel = self.solver.as_ref().map(|sv| {
            let show = |ed: &Editor, on: bool| -> String {
                let mut t = ed.text().to_string();
                if on {
                    let cur = ed.cursor().min(t.len());
                    t.insert(cur, '|');
                }
                if t.is_empty() { t = " ".into() }
                t
            };
            let (focus, mode, nonneg, sel) = (sv.focus, sv.mode, sv.nonneg, sv.sel);
            let field = |id: &'static str, f: u8, text: String, cx: &mut Context<Self>| {
                div().id(id).flex_1().px_2().py_1().bg(rgb(0xFFFFFF))
                    .border_1().rounded_sm()
                    .border_color(if focus == f { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                    .text_size(px(12.5)).font_family("Noto Sans JP")
                    .whitespace_nowrap().overflow_hidden()
                    .child(SharedString::from(text))
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(sv) = &mut this.solver {
                            sv.focus = f;
                        }
                        cx.notify();
                    }))
            };
            let label = |t: &'static str| {
                div().mt_1p5().text_size(px(11.5)).text_color(rgb(0x444B52)).child(t)
            };
            let btn = |id: &'static str, t: &'static str, on: bool| {
                div().id(id).px_2p5().py_1().rounded_sm().border_1()
                    .border_color(if on { rgb(0xC6CDD3) } else { rgb(0xEDEFF1) })
                    .text_size(px(11.5))
                    .text_color(if on { rgb(0x1B1B1B) } else { rgb(0xB6BDC4) })
                    .when(on, |d| d.cursor_pointer().hover(|s| s.bg(rgb(0xEAF5EE))))
            };
            let radio = |id: &'static str, m: u8, t: &'static str, cx: &mut Context<Self>| {
                div().id(id).flex().flex_row().items_center().gap_1()
                    .cursor_pointer().text_size(px(12.0))
                    .child(if mode == m { "◉" } else { "○" })
                    .child(t)
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(sv) = &mut this.solver {
                            sv.mode = m;
                            if m == 2 {
                                sv.focus = 1;
                            }
                        }
                        cx.notify();
                    }))
            };
            // 制約の一覧
            let mut list = div().mt_1().p_1().h(px(96.0)).bg(rgb(0xFAFBFC))
                .border_1().border_color(rgb(0xC6CDD3)).rounded_sm()
                .flex().flex_col().overflow_hidden();
            if sv.cons.is_empty() {
                list = list.child(div().flex_1().flex().items_center().justify_center()
                    .text_size(px(11.5)).text_color(rgb(0xB6BDC4))
                    .child(ui::t!("まだ制約はありません。左辺・記号・右辺を打って「追加」")));
            } else {
                for (i, (l, op, r)) in sv.cons.iter().enumerate() {
                    let on = sel == Some(i);
                    list = list.child(div()
                        .id(SharedString::from(format!("con{i}")))
                        .px_2().py_0p5().rounded_sm().text_size(px(12.0))
                        .bg(if on { rgb(0xEAF5EE) } else { rgb(0xFAFBFC) })
                        .cursor_pointer()
                        .child(SharedString::from(format!("{l} {op} {r}")))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(sv) = &mut this.solver {
                                sv.sel = Some(i);
                                let (l, op, r) = sv.cons[i].clone();
                                sv.con_l = Editor::new(&l);
                                sv.con_op =
                                    SOLVER_OPS.iter().position(|o| *o == op).unwrap_or(0);
                                sv.con_r = Editor::new(&r);
                            }
                            cx.notify();
                        })));
                }
            }
            // ソルバーも表の中央(prompt の板と同じ作法)
            div().absolute().inset_0().flex().items_center().justify_center()
                .child(div().w(px(470.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0x1B6E3C)).shadow_lg()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .flex().flex_col().gap_1()
                .child(div().flex().flex_row().items_center()
                    .child(div().text_size(px(13.0)).font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(0x1B6E3C)).child(ui::t!("ソルバーのパラメータ")))
                    .child(div().flex_1())
                    .child(div().id("sv-x").px_2().cursor_pointer().text_size(px(13.0))
                        .text_color(rgb(0x66707A)).hover(|s| s.text_color(rgb(0xC0392B)))
                        .child("✕")
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.solver = None;
                            cx.notify();
                        }))))
                .child(label("目的を設定"))
                .child(div().flex().flex_row()
                    .child(field("sv-target", 0, show(&sv.target, focus == 0), cx)))
                .child(div().mt_1().flex().flex_row().items_center().gap_3()
                    .child(radio("sv-max", 0, "最大", cx))
                    .child(radio("sv-min", 1, "最小", cx))
                    .child(radio("sv-val", 2, "値:", cx))
                    .child(field("sv-value", 1, show(&sv.value, focus == 1), cx)))
                .child(label("変数セルを変更して"))
                .child(div().flex().flex_row()
                    .child(field("sv-vars", 2, show(&sv.vars, focus == 2), cx)))
                .child(label("制約条件付き(左辺セル / 記号 / 右辺の数かセル)"))
                .child(div().flex().flex_row().items_center().gap_1()
                    .child(field("sv-conl", 3, show(&sv.con_l, focus == 3), cx))
                    .child(div().id("sv-op").px_2().py_1().rounded_sm().border_1()
                        .border_color(rgb(0xC6CDD3)).text_size(px(12.0))
                        .cursor_pointer().hover(|s| s.bg(rgb(0xEAF5EE)))
                        .child(SOLVER_OPS[sv.con_op])
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(sv) = &mut this.solver {
                                sv.con_op = (sv.con_op + 1) % 3;
                            }
                            cx.notify();
                        })))
                    .child(field("sv-conr", 4, show(&sv.con_r, focus == 4), cx)))
                .child(div().mt_1().flex().flex_row().gap_1()
                    .child(btn("sv-add", "追加", true).child(ui::t!("追加"))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(sv) = &mut this.solver {
                                let (l, r) =
                                    (sv.con_l.text().trim().to_string(),
                                     sv.con_r.text().trim().to_string());
                                if l.is_empty() || r.is_empty() {
                                    this.status =
                                        ui::t!("制約の左辺と右辺を先に打ってください").into();
                                } else {
                                    sv.cons.push((l, SOLVER_OPS[sv.con_op], r));
                                    sv.con_l = Editor::new("");
                                    sv.con_r = Editor::new("");
                                    sv.sel = None;
                                }
                            }
                            cx.notify();
                        })))
                    .child(btn("sv-edit", "変更", sel.is_some()).child(ui::t!("変更"))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(sv) = &mut this.solver {
                                if let Some(i) = sv.sel {
                                    let (l, r) =
                                        (sv.con_l.text().trim().to_string(),
                                         sv.con_r.text().trim().to_string());
                                    if !l.is_empty() && !r.is_empty() && i < sv.cons.len() {
                                        sv.cons[i] = (l, SOLVER_OPS[sv.con_op], r);
                                    }
                                }
                            }
                            cx.notify();
                        })))
                    .child(div().flex_1())
                    .child(btn("sv-del", "削除", sel.is_some()).child(ui::t!("削除"))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(sv) = &mut this.solver {
                                if let Some(i) = sv.sel.take() {
                                    if i < sv.cons.len() {
                                        sv.cons.remove(i);
                                    }
                                }
                            }
                            cx.notify();
                        }))))
                .child(list)
                .child(div().id("sv-nonneg").mt_1().flex().flex_row().items_center().gap_1()
                    .cursor_pointer().text_size(px(12.0))
                    .child(if nonneg { "☑" } else { "☐" })
                    .child(ui::t!("制約のない変数を非負にする"))
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(sv) = &mut this.solver {
                            sv.nonneg = !sv.nonneg;
                        }
                        cx.notify();
                    })))
                .child(div().mt_1().flex().flex_row().items_center().gap_2()
                    .child(div().text_size(px(12.0)).font_weight(gpui::FontWeight::BOLD)
                        .child(ui::t!("解法の方法")))
                    .child(div().px_2().py_0p5().border_1().border_color(rgb(0xC6CDD3))
                        .rounded_sm().text_size(px(11.5)).child(ui::t!("単体法 LP"))))
                .child(div().text_size(px(10.5)).text_color(rgb(0x66707A))
                    .child(ui::t!("線形の問題を LP シンプレックスで解きます(裏方 scipy)。非線形はまだ解けません — そのときは断ります")))
                .child(div().mt_1p5().flex().flex_row().gap_1()
                    .child(btn("sv-reset", "すべてリセット", true).child(ui::t!("すべてリセット"))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            let init = this.cursor.a1();
                            this.solver = Some(Solver::new(&init));
                            cx.notify();
                        })))
                    .child(div().flex_1())
                    .child(div().id("sv-solve").px_3().py_1().rounded_sm()
                        .bg(rgb(0x1B6E3C)).text_color(rgb(0xFFFFFF))
                        .text_size(px(12.0)).cursor_pointer()
                        .hover(|s| s.bg(rgb(0x2E8B57)))
                        .child(ui::t!("解を求める"))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.solve_solver(cx);
                            cx.notify();
                        })))
                    .child(btn("sv-close", "閉じる", true).child(ui::t!("閉じる"))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.solver = None;
                            cx.notify();
                        })))))
        });

        // ---- ファイルの全面ページ(本家の File メニュー。タブ0で全面) ----
        let filepage = (self.tab == 0).then(|| {
            let item_bg = rgb(0xE2E6EA);
            let gray = rgb(0xB6BDC4);
            let fg = rgb(0x444B52);
            let dim = rgb(0x66707A);
            let mk = |id: &'static str, label: &'static str, ready: bool| {
                let d = div().id(id).px_4().py_1p5().text_size(px(13.0));
                if ready {
                    d.text_color(fg).cursor_pointer().hover(move |s| s.bg(item_bg))
                } else {
                    d.text_color(gray)
                }
                .child(label)
            };
            let sb = div().w(px(280.0)).bg(rgb(0xF1F3F5))
                .border_r_1().border_color(rgb(0xE1E6EA))
                .flex().flex_col().py_2()
                .child(mk("f-back", ui::t!("‹ 戻る"), true).on_click(cx.listener(|this, _, _, cx| {
                    this.tab = this.prev_tab;
                    cx.notify()
                })))
                .child(div().h(px(10.0)))
                .child(mk("f-new", ui::t!("新規作成"), true).on_click(cx.listener(|this, _, _, cx| {
                    if this.new_book() {
                        this.tab = this.prev_tab;
                    }
                    cx.notify()
                })))
                .child(mk("f-tpl", ui::t!("テンプレートから作成"), false))
                .child(mk("f-open", ui::t!("開く"), true).on_click(cx.listener(|this, _, _, cx| {
                    this.tab = this.prev_tab;
                    this.open_dialog(cx);
                    cx.notify()
                })))
                .child({
                    let d = mk("f-recent", ui::t!("最近開いた"), true).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.file_view = 1;
                            cx.notify()
                        }));
                    if self.file_view == 1 { d.bg(item_bg) } else { d }
                })
                .child(div().h(px(10.0)))
                .child(mk("f-save", ui::t!("保存"), true).on_click(cx.listener(|this, _, _, cx| {
                    this.save(false, cx);
                    cx.notify()
                })))
                .child(mk("f-saveas", ui::t!("名前を付けて保存"), true).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.save_as(cx);
                        cx.notify()
                    })))
                .child(mk("f-print", ui::t!("印刷"), true).on_click(cx.listener(|this, _, _, cx| {
                    this.run_cmd("pdf", cx);
                    cx.notify()
                })))
                .child(mk("f-protect", ui::t!("保護する"), true).on_click(cx.listener(
                    |this, _, _, cx| {
                        if let Some(i) =
                            ribbon::CALC.iter().position(|t| t.name == "保護")
                        {
                            this.prev_tab = i;
                            this.tab = i;
                        }
                        cx.notify()
                    })))
                .child(div().h(px(10.0)))
                .child({
                    let d = mk("f-info", ui::t!("詳細情報"), true).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.file_view = 0;
                            cx.notify()
                        }));
                    if self.file_view == 0 { d.bg(item_bg) } else { d }
                })
                .child(mk("f-place", ui::t!("ファイルの場所を開く"), true).on_click(cx.listener(
                    |this, _, _, cx| {
                        match this.path.as_ref().and_then(|p| p.parent()) {
                            Some(dir) => {
                                let _ = std::process::Command::new("xdg-open")
                                    .arg(dir)
                                    .spawn();
                            }
                            None => {
                                this.status = ui::t!("まだファイルになっていません").into();
                            }
                        }
                        cx.notify()
                    })))
                .child(div().h(px(10.0)))
                .child(mk("f-quit", ui::t!("終了"), true).on_click(cx.listener(|this, _, _, cx| {
                    this.request_quit(cx);
                    cx.notify()
                })))
                .child(div().flex_1())
                .child({
                    let d = mk("f-opts", ui::t!("詳細設定"), true).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.file_view = 2;
                            cx.notify()
                        }));
                    if self.file_view == 2 { d.bg(item_bg) } else { d }
                })
                .child(mk("f-help", ui::t!("ヘルプ"), false))
                .child(mk("f-req", ui::t!("機能のリクエスト"), false));
            let mut pane = div().flex_1().bg(gpui::white()).p_8()
                .flex().flex_col().gap_3().text_size(px(12.5)).text_color(fg);
            if self.file_view == 2 {
                // 詳細設定 — 器は ~/.config/office/settings.toml
                // (SEKKEI「設定 — 器と言語」。環境変数が一時上書きで優先)
                let lang_now = ui::settings::get("language").unwrap_or_else(|| "ja".into());
                let row = |label: &'static str, value: String| {
                    div().flex().flex_row().items_center().gap_2()
                        .child(div().w(px(200.0)).text_color(dim).child(label))
                        .child(div().child(SharedString::from(value)))
                };
                pane = pane
                    .child(div().text_size(px(16.0))
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(ui::t!("詳細設定")))
                    .child(div().text_color(dim).child(SharedString::from(
                        ui::tf!("置き場: {}", ui::settings::path().display()))))
                    .child(div().h(px(6.0)))
                    .child(div().flex().flex_row().items_center().gap_2()
                        .child(div().w(px(200.0)).text_color(dim)
                            .child(ui::t!("言語(リボンと文言)")))
                        .child(div().id("set-lang")
                            .px_3().py_1().rounded_sm().cursor_pointer()
                            .bg(item_bg)
                            .child(SharedString::from(match lang_now.as_str() {
                                "ja" => "日本語".to_string(),
                                other => other.to_string(),
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                let cur = ui::settings::get("language")
                                    .unwrap_or_else(|| "ja".into());
                                let all = ui::languages();
                                let i = all.iter().position(|l| **l == cur).unwrap_or(0);
                                let next = all[(i + 1) % all.len()];
                                ui::settings::set("language", next);
                                this.status = ui::t!("言語を控えました(次の起動から効きます。環境変数 OFFICE_LANG があればそちらが優先)").into();
                                cx.notify()
                            }))))
                    .child(div().h(px(10.0)))
                    .child(row(ui::t!("書体(OFFICE_FONT)"),
                        std::env::var("OFFICE_FONT")
                            .unwrap_or_else(|_| ui::t!("(文書に従う)").into())))
                    .child(row(ui::t!("校正の宛先"), {
                        let ep = ui::Endpoint::default();
                        format!("{}:{} / {}", ep.host, ep.port, ep.model)
                    }))
                    .child(row(ui::t!("Python の経路"),
                        std::env::var("JO_PYTHON")
                            .unwrap_or_else(|_| ui::t!("(自動: .venv → python3)").into())))
                    .child(row(ui::t!("名前(ロック・チャット・署名)"), lock_identity()));
            } else if self.file_view == 1 {
                pane = pane.child(div().text_size(px(16.0))
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(ui::t!("最近開いた")));
                let list = Self::recent_list();
                if list.is_empty() {
                    pane = pane.child(div().text_color(dim)
                        .child(ui::t!("(まだありません。開く・保存すると残ります)")));
                }
                for (i, q) in list.into_iter().enumerate() {
                    let name = q.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let dir = q.parent()
                        .map(|d| d.to_string_lossy().to_string())
                        .unwrap_or_default();
                    pane = pane.child(div()
                        .id(SharedString::from(format!("recent-{i}")))
                        .px_2().py_1().rounded_sm().cursor_pointer()
                        .hover(move |s| s.bg(item_bg))
                        .flex().flex_row().items_center().gap_2()
                        .child(div().text_size(px(13.0)).child(SharedString::from(name)))
                        .child(div().text_size(px(11.0)).text_color(dim)
                            .child(SharedString::from(dir)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.tab = this.prev_tab;
                            this.open(q.clone());
                            cx.notify()
                        })));
                }
            } else {
                // 統計(生きた値)とブックの情報(docProps/core.xml から)
                let sheets_n = self.book.sheets.len();
                let mut cells_n = 0usize;
                let mut formulas_n = 0usize;
                for sh in &self.book.sheets {
                    cells_n += sh.cells.len();
                    formulas_n +=
                        sh.cells.values().filter(|c| c.formula.is_some()).count();
                }
                let shapes_n: usize = self
                    .book
                    .sheets
                    .iter()
                    .map(|s| {
                        s.shapes.len() + s.shapes_new.len() + s.images.len()
                            + s.images_new.len()
                    })
                    .sum();
                pane = pane.child(div().text_size(px(16.0))
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(ui::t!("ブックの情報")))
                    .child(div().text_size(px(13.5))
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(ui::t!("統計")));
                for (k, v) in [
                    ("シート", sheets_n),
                    ("使っているセル", cells_n),
                    ("式のセル", formulas_n),
                    ("図形と画像", shapes_n),
                ] {
                    pane = pane.child(div().flex().flex_row()
                        .child(div().w(px(220.0)).text_color(dim).child(k))
                        .child(SharedString::from(format!("{v}"))));
                }
                pane = pane.child(div().h(px(6.0)))
                    .child(div().text_size(px(13.5))
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(ui::t!("プロパティ")));
                let pr = &self.book.props;
                for (k, v, kind) in [
                    ("作成者", pr.creator.clone(), "prop-creator"),
                    ("タイトル", pr.title.clone(), "prop-title"),
                    ("タグ", pr.keywords.clone(), "prop-keywords"),
                    ("件名", pr.subject.clone(), "prop-subject"),
                    ("コメント", pr.description.clone(), "prop-desc"),
                ] {
                    let empty = v.is_empty();
                    let init = v.clone();
                    pane = pane.child(div().flex().flex_row().items_center()
                        .child(div().w(px(220.0)).text_color(dim).child(k))
                        .child(div()
                            .id(SharedString::from(kind))
                            .w(px(320.0)).px_2().py_1().rounded_sm()
                            .border_1().border_color(rgb(0xE1E6EA))
                            .cursor_pointer()
                            .hover(move |s| s.bg(item_bg))
                            .whitespace_nowrap().overflow_hidden()
                            .text_color(if empty { gray } else { fg })
                            .child(SharedString::from(if empty {
                                ui::t!("テキストの追加").to_string()
                            } else {
                                v
                            }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.prompt = Some((kind, Editor::new(&init)));
                                cx.notify()
                            }))));
                }
                pane = pane.child(div().text_size(px(11.5)).text_color(dim)
                    .child(ui::t!("欄を押して打ち、Enter で控える(保存で xlsx の情報に入ります)")));
            }
            div().absolute().inset_0().bg(gpui::white())
                .flex().flex_row()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(sb)
                .child(pane)
        });

        // ---- スライサーの小窓(列の値の釦で絞る) ----
        let slicer_panel = self.slicer.as_ref().map(|(col, sel, multi)| {
            let col = *col;
            let multi = *multi;
            // 見出し(1行目)と、その下の一意な値。空欄は「(空白)」で最後に
            let head = self
                .sheet()
                .get(Pos::new(0, col))
                .map(|c| c.value.display())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| ui::tf!("列{}", col_name(col)));
            let (rows, _) = self.sheet().extent();
            let mut vals: std::collections::BTreeSet<String> = Default::default();
            let mut has_blank = false;
            for r in 1..rows {
                let v = self
                    .sheet()
                    .get(Pos::new(r, col))
                    .map(|c| c.value.display())
                    .unwrap_or_default();
                if v.is_empty() {
                    has_blank = true;
                } else {
                    vals.insert(v);
                }
            }
            let mut items: Vec<String> = vals.into_iter().take(64).collect();
            if has_blank {
                items.push(ui::t!("(空白)").to_string());
            }
            let mut p = div().absolute().right(px(24.0)).top(px(ROW_H + 16.0)).w(px(190.0))
                .p_2().rounded_md().bg(gpui::white())
                .border_1().border_color(rgb(0x1B6E3C)).shadow_lg()
                .flex().flex_col().gap_1()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(div().flex().flex_row().items_center()
                    .child(div().text_size(px(12.5)).font_weight(gpui::FontWeight::BOLD)
                        .whitespace_nowrap().overflow_hidden()
                        .child(SharedString::from(head)))
                    .child(div().flex_1())
                    // ≡ = 複数選択の入切(本家のスライサーと同じ並び)
                    .child(div().id("sl-multi").px_1p5().rounded_sm().cursor_pointer()
                        .text_size(px(12.5))
                        .bg(if multi { rgb(0xCFE6D8) } else { rgb(0xFFFFFF) })
                        .hover(|s| s.bg(rgb(0xEAF5EE)))
                        .child("≡")
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some((_, _, m)) = &mut this.slicer {
                                *m = !*m;
                                this.status = if *m {
                                    ui::t!("複数選択: 押した値を重ねて絞ります").into()
                                } else {
                                    ui::t!("単数選択: 押した値ひとつで絞ります").into()
                                };
                            }
                            cx.notify();
                        })))
                    // ✕ = 選びを解除(全部見せる)
                    .child(div().id("sl-clear").px_1p5().rounded_sm().cursor_pointer()
                        .text_size(px(12.5)).hover(|s| s.bg(rgb(0xEAF5EE)))
                        .child("✕")
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some((_, sel, _)) = &mut this.slicer {
                                sel.clear();
                            }
                            this.status = ui::t!("スライサーの絞りを解除しました").into();
                            cx.notify();
                        }))));
            for (i, v) in items.into_iter().enumerate() {
                let on = sel.contains(&v);
                p = p.child(div()
                    .id(SharedString::from(format!("sl{i}")))
                    .px_2().py_1().rounded_sm().border_1()
                    .border_color(rgb(0xC6CDD3))
                    .bg(if on { rgb(0xBBD9EA) } else { rgb(0xFFFFFF) })
                    .text_size(px(12.0)).cursor_pointer()
                    .whitespace_nowrap().overflow_hidden()
                    .hover(|s| s.bg(rgb(0xEAF5EE)))
                    .child(SharedString::from(v.clone()))
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some((_, sel, multi)) = &mut this.slicer {
                            if *multi {
                                if !sel.remove(&v) {
                                    sel.insert(v.clone());
                                }
                            } else if sel.len() == 1 && sel.contains(&v) {
                                sel.clear(); // 同じ釦をもう一度 = 解除
                            } else {
                                sel.clear();
                                sel.insert(v.clone());
                            }
                            this.status = if sel.is_empty() {
                                ui::t!("絞りなし(全部見えています)").into()
                            } else {
                                ui::tf!("絞り: {}(見え方だけ。中身は変わりません)", sel.iter().cloned().collect::<Vec<_>>().join(" / "))
                                .into()
                            };
                        }
                        cx.notify();
                    })));
            }
            p
        });

        // ---- 書式の小窓(セルをフォーマットする) ----
        // モーダルにしない: 範囲を選び直しながら続けて使える道具箱。
        // どの釦も既存の書式の道(fmt / run_cmd)を通り、1手ずつ戻せる
        let fmt_panel = self.fmt_panel.map(|(fx, fy)| {
            let fx = fx.min(560.0);
            let fy = fy.min(320.0);
            let btn = |id: &'static str, label: &'static str| {
                div().id(id).px_2().py_1().rounded_sm()
                    .border_1().border_color(rgb(0xC6CDD3))
                    .text_size(px(11.5)).text_color(rgb(0x1B1B1B))
                    .cursor_pointer().hover(|s| s.bg(rgb(0xEAF5EE)))
                    .child(label)
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                        move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.fmt_panel_action(id, cx);
                            cx.notify();
                        }))
            };
            let swatch = |id: &'static str, color: Option<&'static str>| {
                let mut s = div().id(id).w(px(20.0)).h(px(20.0)).rounded_sm()
                    .border_1().border_color(rgb(0xC6CDD3))
                    .cursor_pointer();
                s = match color {
                    Some(c) => s.bg(hex(c)),
                    // 「なし」は斜線の代わりに白+薄字の×
                    None => s.bg(rgb(0xFFFFFF)).flex().items_center().justify_center()
                        .text_size(px(10.0)).text_color(rgb(0x9AA5AE)).child("×"),
                };
                s.on_mouse_down(gpui::MouseButton::Left, cx.listener(
                    move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.fmt_panel_action(id, cx);
                        cx.notify();
                    }))
            };
            let title = |t: &'static str| div().text_size(px(10.5))
                .text_color(rgb(0x66707A)).mt_1p5().child(t);
            let row = || div().flex().flex_row().flex_wrap().gap_1().items_center();

            div().absolute().left(px(fx)).top(px(fy)).w(px(300.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3)).shadow_lg()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(div().flex().flex_row().items_center().justify_between()
                    .child(div().text_size(px(12.5)).font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(0x1B6E3C))
                        .child(ui::t!("セルの書式(選んでいる範囲に効く)")))
                    .child(div().id("fmtclose").px_2().rounded_sm().cursor_pointer()
                        .text_size(px(12.0)).text_color(rgb(0x66707A))
                        .hover(|s| s.bg(rgb(0xE1E6EA)))
                        .child("✕")
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                            move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.fmt_panel = None;
                                cx.notify();
                            }))))
                .child(title("罫線"))
                .child(row()
                    .child(btn("b-all", ui::t!("格子")))
                    .child(btn("b-out", ui::t!("外枠")))
                    .child(btn("b-none", ui::t!("なし"))))
                .child(title("塗り"))
                .child(row()
                    .child(swatch("fill-none", None))
                    .child(swatch("fill-FFF2CC", Some("FFF2CC")))
                    .child(swatch("fill-DEEAF6", Some("DEEAF6")))
                    .child(swatch("fill-E2EFDA", Some("E2EFDA")))
                    .child(swatch("fill-FCE4D6", Some("FCE4D6")))
                    .child(swatch("fill-D9D9D9", Some("D9D9D9"))))
                .child(title("文字の色"))
                .child(row()
                    .child(swatch("color-none", None))
                    .child(swatch("color-C00000", Some("C00000")))
                    .child(swatch("color-1F4E79", Some("1F4E79")))
                    .child(swatch("color-1B6E3C", Some("1B6E3C")))
                    .child(swatch("color-7F7F7F", Some("7F7F7F"))))
                .child(title("文字"))
                .child(row()
                    .child(btn("bold", ui::t!("太字")))
                    .child(btn("italic", ui::t!("斜体")))
                    .child(btn("underline", ui::t!("下線")))
                    .child(btn("strikeout", ui::t!("取り消し")))
                    .child(btn("incfont", ui::t!("大きく")))
                    .child(btn("decfont", ui::t!("小さく"))))
                .child(title("揃え"))
                .child(row()
                    .child(btn("align-left", ui::t!("左")))
                    .child(btn("align-center", ui::t!("中央")))
                    .child(btn("align-right", ui::t!("右")))
                    .child(btn("top", ui::t!("上")))
                    .child(btn("middle", ui::t!("中")))
                    .child(btn("bottom", ui::t!("下")))
                    .child(btn("wrap", ui::t!("折り返し"))))
                .child(title("表示形式"))
                .child(row()
                    .child(btn("comma", "1,000"))
                    .child(btn("currency", "¥"))
                    .child(btn("percents", "%"))
                    .child(btn("digit-inc", ".0+"))
                    .child(btn("digit-dec", ".0−"))
                    .child(btn("numfmt-none", ui::t!("なし"))))
        });

        // ---- ドロップダウンリスト(同じ列の値の一覧) ----
        let pick_panel = self.pick.clone().map(|(vals, (vx, vy))| {
            // 色の一覧(文字の色・塗り)は名前の左に色見本の四角を添える
            let swatch_of = |name: &str| -> Option<Option<&'static str>> {
                match self.pick_kind {
                    "font-color" => FONT_COLORS.iter().find(|(n, _)| *n == name).map(|(_, h)| *h),
                    "fill-color" => FILL_COLORS.iter().find(|(n, _)| *n == name).map(|(_, h)| *h),
                    _ => None,
                }
            };
            // 長い一覧(書体など)は板の中でスクロール — 数で切り捨てない
            let mut p = div().id("pick-list").absolute().left(px(vx)).top(px(vy))
                .w(px(self.col_px(self.cursor.col).max(120.0)))
                .max_h(px((self.view_h_px - 160.0).max(160.0)))
                .overflow_y_scroll()
                .p_1().rounded_md().bg(rgb(0xFFFFFF))
                .border_1().border_color(rgb(0xC6CDD3)).shadow_lg()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation());
            for (i, v) in vals.into_iter().enumerate() {
                let sw = swatch_of(&v);
                p = p.child(div()
                    .id(SharedString::from(format!("pk{i}")))
                    .px_2().py_1().rounded_sm().cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF5EE)))
                    .flex().flex_row().items_center().gap_2()
                    .text_size(px(12.5)).text_color(rgb(0x1B1B1B))
                    .whitespace_nowrap().overflow_hidden()
                    .children(sw.map(|hx| {
                        let q = div().w(px(14.0)).h(px(14.0)).rounded_sm()
                            .border_1().border_color(rgb(0xC6CDD3));
                        match hx {
                            Some(h) => q.bg(hex(h)),
                            None => q.bg(rgb(0xFFFFFF)),
                        }
                    }))
                    .child(SharedString::from(v.clone()))
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                        move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.pick = None;
                            this.apply_pick(&v, cx);
                            cx.notify();
                        })));
            }
            p
        });

        let notes = if self.notes.is_empty() { None } else {
            let mut n = div().px_4().py_2().bg(rgb(0xFFF6E6))
                .border_t_1().border_color(rgb(0xE8D5A8))
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                       .text_color(rgb(0x8A4B00)).child(ui::t!("この版で読み飛ばしたもの")));
            for x in &self.notes {
                n = n.child(div().text_size(px(11.0)).text_color(rgb(0x8A4B00))
                            .child(x.clone()));
            }
            Some(n)
        };

        let me: Entity<Calc> = cx.entity();
        div().size_full().flex().flex_col().bg(rgb(0xF3F5F7))
            .key_context("jo_edit")
            .track_focus(&self.focus)
            .on_action(cx.listener(Calc::a_backspace))
            .on_action(cx.listener(Calc::a_delete))
            .on_action(cx.listener(Calc::a_copy))
            .on_action(cx.listener(Calc::a_cut))
            .on_action(cx.listener(Calc::a_paste))
            .on_action(cx.listener(Calc::a_paste_values))
            .on_action(cx.listener(Calc::a_left))
            .on_action(cx.listener(Calc::a_right))
            .on_action(cx.listener(Calc::a_up))
            .on_action(cx.listener(Calc::a_down))
            .on_action(cx.listener(Calc::a_page_up))
            .on_action(cx.listener(Calc::a_page_down))
            .on_action(cx.listener(Calc::a_doc_home))
            .on_action(cx.listener(Calc::a_doc_end))
            .on_action(cx.listener(Calc::a_tab))
            .on_action(cx.listener(Calc::a_enter))
            .on_action(cx.listener(Calc::a_select_all))
            .on_action(cx.listener(Calc::a_redo))
            .on_action(cx.listener(Calc::a_select_left))
            .on_action(cx.listener(Calc::a_select_right))
            .on_action(cx.listener(Calc::a_select_up))
            .on_action(cx.listener(Calc::a_select_down))
            .on_action(cx.listener(Calc::a_undo))
            .on_action(cx.listener(Calc::a_save))
            .on_action(cx.listener(Calc::a_open))
            .on_action(cx.listener(Calc::a_quit))
            .on_action(cx.listener(Calc::a_context_menu))
            .on_action(cx.listener(Calc::a_cancel))
            .on_action(cx.listener(Calc::a_edit_cell))
            .child(bar)
            .children((self.tab != 0 && self.show_formula_bar).then(|| formula_bar))
            .child(div().flex_1().overflow_hidden().relative()
                   // ホイールで窓を動かす(下に回すと先の行が見える)
                   .on_scroll_wheel(cx.listener(|this, e: &gpui::ScrollWheelEvent, _, cx| {
                       let (dx, dy) = match e.delta {
                           gpui::ScrollDelta::Pixels(p) =>
                               (-f32::from(p.x) / COL_W, -f32::from(p.y) / ROW_H),
                           gpui::ScrollDelta::Lines(l) => (-l.x, -l.y * 3.0),
                       };
                       this.wheel.0 += dy;
                       this.wheel.1 += dx;
                       let dr = this.wheel.0.trunc() as i32;
                       let dc = this.wheel.1.trunc() as i32;
                       this.wheel.0 -= dr as f32;
                       this.wheel.1 -= dc as f32;
                       if dr != 0 || dc != 0 {
                           this.view.row = (this.view.row as i32 + dr).clamp(0, 9999) as u32;
                           this.view.col = (this.view.col as i32 + dc).clamp(0, 255) as u32;
                           cx.notify();
                       }
                   }))
                   .child(grid)
                   .children(ink_preview)
                   .children({
                       // 浮かぶ画像(グラフ)。錨のセルが見えている間だけ描く。
                       // マウスは受けない(セルの操作を遮らない)
                       let mut layer: Vec<gpui::AnyElement> = Vec::new();
                       for im in self.sheet().images.iter().chain(self.sheet().images_new.iter()) {
                           let Some((x, y)) = self.cell_origin_px(im.at) else { continue };
                           let key = im.data.as_ptr() as usize;
                           let src = self
                               .img_cache
                               .borrow_mut()
                               .entry(key)
                               .or_insert_with(|| {
                                   let fmt = if im.data.starts_with(&[0xFF, 0xD8]) {
                                       gpui::ImageFormat::Jpeg
                                   } else {
                                       gpui::ImageFormat::Png
                                   };
                                   std::sync::Arc::new(gpui::Image::from_bytes(
                                       fmt,
                                       im.data.clone(),
                                   ))
                               })
                               .clone();
                           layer.push(
                               gpui::img(src)
                                   .absolute()
                                   .left(px(x))
                                   .top(px(y))
                                   .w(px(im.width_px))
                                   .h(px(im.height_px))
                                   .into_any_element(),
                           );
                       }
                       // 図形(SVG)。大きさを織り込んで作るので、伸ばしても鮮明
                       for (i, sp) in self
                           .sheet()
                           .shapes
                           .iter()
                           .chain(self.sheet().shapes_new.iter())
                           .enumerate()
                       {
                           let Some((x, y)) = self.cell_origin_px(sp.at) else { continue };
                           let (x, y) = (x + sp.dx_px, y + sp.dy_px);
                           let svg = sp.to_svg();
                           let key = {
                               use std::hash::{Hash, Hasher};
                               let mut h = std::collections::hash_map::DefaultHasher::new();
                               svg.hash(&mut h);
                               h.finish() as usize
                           };
                           let src = self
                               .img_cache
                               .borrow_mut()
                               .entry(key)
                               .or_insert_with(|| {
                                   std::sync::Arc::new(gpui::Image::from_bytes(
                                       gpui::ImageFormat::Svg,
                                       svg.into_bytes(),
                                   ))
                               })
                               .clone();
                           layer.push(
                               gpui::img(src)
                                   .absolute()
                                   .left(px(x))
                                   .top(px(y))
                                   .w(px(sp.width_px))
                                   .h(px(sp.height_px))
                                   .into_any_element(),
                           );
                           if let Some(t) = &sp.text {
                               layer.push(
                                   div()
                                       .absolute()
                                       .left(px(x + 6.0))
                                       .top(px(y + 4.0))
                                       .w(px((sp.width_px - 12.0).max(8.0)))
                                       .h(px((sp.height_px - 8.0).max(8.0)))
                                       .overflow_hidden()
                                       .text_size(px(12.5))
                                       .font_family("Noto Sans JP")
                                       .text_color(rgb(0x1B1B1B))
                                       .whitespace_normal()
                                       .child(SharedString::from(t.clone()))
                                       .into_any_element(),
                               );
                           }
                           let _ = i;
                       }
                       // 控えが育ちすぎたら捨てる(undo のクローンで鍵が増えるため)
                       if self.img_cache.borrow().len() > 64 {
                           self.img_cache.borrow_mut().clear();
                       }
                       layer
                   })
                   .child(InputSink { view: me })
                   .children(shape_frame)
                   .children(ants)
                   .children(tip)
                   .children(fmt_panel)
                   .children(menu)
                   .children(filepage)
                   .children(pick_panel)
                   .children(prompt_panel)
                   .children(solver_panel)
                   .children(fn_panel)
                   .children(fn_args_panel)
                   .children(quit_panel)
                   .children(slicer_panel))
            .children(watch_bar)
            .child(sheets_bar)
            .children(notes)
            // 窓の縁のつかみ(最後に描く = 最初にマウスを受ける)
            .children(ui::resize_edges(window))
    }
}

fn main() {
    let arg = std::env::args().nth(1).map(PathBuf::from);
    application().with_assets(ui::Icons).run(move |cx: &mut App| {
        cx.text_system()
            .add_fonts(vec![std::borrow::Cow::Borrowed(font_data())])
            .expect("フォント登録");
        cx.bind_keys(ui::bindings("jo_edit"));
        // 前に閉じたときの姿で開く。控えが無ければ既定の大きさで中央に
        let saved = ui::winstate::load("calc");
        let bounds = match saved {
            Some(st) => Bounds::new(gpui::point(px(st.x), px(st.y)), size(px(st.w), px(st.h))),
            None => Bounds::centered(None, size(px(1060.0), px(820.0)), cx),
        };
        let wb = if saved.is_some_and(|st| st.maximized) {
            WindowBounds::Maximized(bounds)
        } else {
            WindowBounds::Windowed(bounds)
        };
        let arg2 = arg.clone();
        cx.open_window(
            WindowOptions {
                window_bounds: Some(wb),
                ..Default::default()
            },
            move |window, cx| {
                let view = cx.new(|cx| Calc::new(arg2.clone(), cx));
                window.focus(&view.focus_handle(cx), cx);
                // 動かす・伸ばすたびに控える — 閉じる経路が何本あっても漏れない。
                // 全画面は控えない(次も全画面で開くと出口が分かりにくい)
                view.update(cx, |_, cx| {
                    cx.observe_window_bounds(window, |_, window, _| {
                        let wb = window.window_bounds();
                        if matches!(wb, WindowBounds::Fullscreen(_)) {
                            return;
                        }
                        let b = wb.get_bounds();
                        ui::winstate::save("calc", ui::winstate::WinState {
                            x: f32::from(b.origin.x),
                            y: f32::from(b.origin.y),
                            w: f32::from(b.size.width),
                            h: f32::from(b.size.height),
                            maximized: matches!(wb, WindowBounds::Maximized(_)),
                        });
                    })
                    .detach();
                });
                // WM からの「閉じる」(Alt+F4 等)も同じ確認を通す。
                // 書きかけがあれば「まだ閉じない」と答え、確認は別の糸で出す
                let v = view.clone();
                window.on_window_should_close(cx, move |_, cx| {
                    let quit_now = v.update(cx, |this, cx| {
                        this.commit();
                        if this.dirty && this.path.is_some() {
                            this.request_quit(cx);
                            false
                        } else {
                            this.release_lock();
                            true
                        }
                    });
                    if quit_now {
                        cx.quit();
                    }
                    quit_now
                });
                if std::env::var_os("JO_SELFTEST").is_some() {
                    // 画面が実際に動くかの自己診断: B列の幅を1秒ごとに広げ狭めし、
                    // 15秒で自動終了する。**操作は要らない** — 見ているだけで、
                    // 「モデルは動くのに画面が止まる」疑いを切り分けられる
                    let v = view.clone();
                    cx.spawn(async move |cx| {
                        for i in 0..15u32 {
                            cx.background_executor()
                                .timer(std::time::Duration::from_millis(1000))
                                .await;
                            let _ = v.update(cx, |c, cx| {
                                let w = if i % 2 == 0 { 20.0 } else { 5.0 };
                                c.book.sheets[0].col_width.insert(1, w);
                                eprintln!("tick {}", i + 1);
                                c.status = ui::tf!("自己診断 {}/15: B列の幅 {}(勝手に動けば描画は健全)", i + 1, w)
                                .into();
                                cx.notify();
                            });
                        }
                        let _ = cx.update(|cx| cx.quit());
                    })
                    .detach();
                }
                view
            },
        )
        .unwrap();
        cx.activate(true);
    });
}

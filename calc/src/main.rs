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
mod cmds;
mod view;
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
    /// 並べ替えの「拡張しますか」の聞き取り中(昇順か)。Esc でやめる
    sort_pend: Option<bool>,
    /// 一覧(pick)の題 — いま何を選んでいるか(ピボットの段など)。
    /// 一覧の上に太字で出す。閉じるときに消す
    pick_note: Option<SharedString>,
    /// ピボットの絞り込みの聞き取り中: (指図の番号, 見出し, 隠す値の作業用)
    pivot_flt: Option<(usize, String, std::collections::BTreeSet<String>)>,
    /// 重複の削除の下ごしらえ: (列番号, 見せる名前, 比べるか) の列と「先頭行は見出し」
    pub(crate) dedup_pend: Option<(Vec<(u32, String, bool)>, bool)>,
    /// 条件付き書式のルールの管理で選んだ規則(sheet.cond の添字)
    pub(crate) cond_pend: Option<usize>,
    /// 罫線のペン(線種と色)。罫線の一覧から掛けるときに使う
    pen_style: sheet::model::BStyle,
    pen_color: Option<u32>,
    /// ヘッダー/フッターの聞き取り中: (フッターか, 0=左 1=中 2=右)
    hf_pend: Option<(bool, u8)>,
    /// 名前マネージャーで選んだ名前(移動/打ち直し/削除の相手)
    name_pend: Option<String>,
    /// 結合の確認待ちの種類(中央/横方向/結合だけ)
    merge_kind_pend: Option<String>,
    /// 書式のコピー(刷毛)で持っている書式。次のクリックで塗って手放す
    brush: Option<CellFormat>,
    /// 右クリックメニューの出どころ(Some(true)=列見出し / Some(false)=行見出し)
    menu_head: Option<bool>,
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
    /// 選んでいる画像(images_new の番号)。グラフもここ
    img_sel: Option<usize>,
    img_drag: Option<(usize, (f32, f32), (f32, f32), bool)>,
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
    /// 固定した枠に影を付ける(本家の viewtab:freezeshadow。見た目だけ)
    freeze_shadow: bool,
    /// オートフィルタ(Excel の▼)。**見え方だけ** — 保存される中身は
    /// 変わらず、閉じれば消える。範囲の1行目が見出しで、列ごとに
    /// 「隠す値」を持つ(map に無い列は素通し)
    auto_filter: Option<AutoFilter>,
    /// 開いている▼の板(列, 値の検索)。Esc で閉じる
    filter_panel: Option<(u32, Editor)>,
    /// 「データの入力規則」の板(本家の3タブのダイアログの形)
    dv_dlg: Option<DvDlg>,
    /// 画面の文字の大きさ(リボン・メニュー・状態行まで全部に掛かる倍率。
    /// 格子のズームとは別。設定に覚える — 次回も同じ大きさで開く)
    ui_scale: f32,
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
        if let Some((_, ed)) = &mut self.filter_panel {
            return ed; // ▼の板の検索欄
        }
        if let Some(d) = &mut self.dv_dlg {
            return d.focused();
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
        if let Some((_, ed)) = &self.filter_panel {
            return ed;
        }
        if let Some(d) = &self.dv_dlg {
            return d.focused_ref();
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
            && self.filter_panel.is_none() && self.dv_dlg.is_none()
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
            sort_pend: None,
            pick_note: None,
            pivot_flt: None,
            dedup_pend: None,
            cond_pend: None,
            pen_style: sheet::model::BStyle::default(),
            pen_color: None,
            hf_pend: None,
            name_pend: None,
            merge_kind_pend: None,
            brush: None,
            menu_head: None,
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
            img_sel: None,
            img_drag: None,
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
            freeze_shadow: false,
            auto_filter: None,
            filter_panel: None,
            dv_dlg: None,
            ui_scale: ui::settings::get("ui_scale")
                .and_then(|v| v.parse::<f32>().ok())
                .map(|v| v.clamp(0.8, 1.5))
                .unwrap_or(1.0),
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
            dark: ui::settings::get("theme").as_deref() == Some("dark"),
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

    /// 参照の見せ方(R1C1 のときはカーソル基準の R[..]C[..] に)
    pub(crate) fn ref_disp(&self, p: Pos) -> String {
        if self.book.r1c1 {
            sheet::model::formula_to_r1c1(&p.a1(), self.cursor)
        } else {
            p.a1()
        }
    }

    fn sync_input(&mut self) {
        let mut s = self.sheet().get(self.cursor).map(|c| c.editable()).unwrap_or_default();
        // R1C1: 見せるときだけ変換(中身は A1 のまま)
        if self.book.r1c1 {
            if let Some(body) = s.strip_prefix('=') {
                s = format!("={}", sheet::model::formula_to_r1c1(body, self.cursor));
            }
        }
        self.input = Editor::new(&s);
        self.edit_armed = false; // セルを移った=編集は仕切り直し
        if self.pick_kind == "fn-complete" {
            self.pick = None; // 補完の一覧も畳む
        }
        // 入力メッセージ付きの規則のセルに乗ったら、その説明を出す
        if let Some((t, m)) = self
            .sheet()
            .validation_at(self.cursor)
            .and_then(|v| v.input_msg.clone())
        {
            self.status = if t.is_empty() {
                m.into()
            } else if m.is_empty() {
                t.into()
            } else {
                format!("{t}: {m}").into()
            };
        } else if let Some(i) = self.pivot_at(self.cursor) {
            // ピボットに乗ったら、名前と操作の場所を言う(文脈タブの案内)
            let name = self.book.pivots[i].name.clone();
            self.status = ui::tf!(
                "{} の上です — 操作は「ピボットテーブル」のタブで(更新・総計・小計・レイアウト。表を崩す操作は締まります)",
                if name.is_empty() { ui::t!("ピボット").to_string() } else { name }
            )
            .into();
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
            self.auto_filter = None;
            self.filter_panel = None;
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
        if self.filter_active() {
            // 絞り込み中は頭から詰めて見せる(範囲の後ろの行も続けて出す)
            let (rows, _) = self.sheet().extent();
            let last = self.auto_filter.as_ref().map(|f| f.range.1.row + 1).unwrap_or(0);
            return (0..rows.max(last))
                .filter(|r| {
                    !hidden.contains(r) && self.filter_keeps(*r) && self.slicer_keeps(*r)
                })
                .take(fit as usize)
                .collect();
        }
        if self.slicer.as_ref().is_some_and(|(_, sel, _)| !sel.is_empty()) {
            // スライサーで絞る: 見出し+選んだ値の行(絞り込みと同じ流儀)
            let (rows, _) = self.sheet().extent();
            (0..rows)
                .filter(|r| !hidden.contains(r) && self.slicer_keeps(*r))
                .take(fit as usize)
                .collect()
        } else {
            // 畳んだ行のぶん多めに見て、画面の行数まで詰める
            let extra = hidden.len() as u32;
            grid_rows(self.frozen, self.view, fit + extra)
                .into_iter()
                .filter(|r| !hidden.contains(r))
                .take(fit as usize)
                .collect()
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
        // 浮いている画像(グラフ)も同じ扱い
        if let Some((i, (sx, sy), corner)) = self.image_at(x, y) {
            self.commit();
            self.checkpoint();
            self.img_sel = Some(i);
            self.img_drag = Some((i, (x, y), (sx, sy), corner));
            self.status = if corner {
                ui::t!("右下を引いて大きさを変えます(比は保ちます)").into()
            } else {
                ui::t!("画像を選びました(ドラッグで移動 / 右下で大きさ / Del で削除)").into()
            };
            return;
        }
        self.img_sel = None;
        if self.read_image_at(x, y) {
            // 読み込んだ画像は原文持ち越しが正 — 動かせないと正直に言う
            self.status = ui::t!(
                "読み込んだ画像は動かせません(保存で元の姿を守るため。挿し直せばこのアプリの画像になります)"
            )
            .into();
        }
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
                let a1 = self.ref_disp(p);
                self.input.insert(&a1);
                let end = self.input.cursor();
                self.ref_pick = Some((p, end - a1.len()..end));
                return;
            }
        }
        // Ctrl+クリックはリンクを開く(基幹網の外は既定のブラウザに任せる)
        if ctrl && !shift {
            if let Some(url) = self.sheet().links.get(&p).cloned() {
                if let Some(loc) = url.strip_prefix('#') {
                    // 帳面の中の場所(#Sheet2!B5 / #B5 / #A1:C9)へ跳ぶ
                    let (name, refs) = match loc.split_once('!') {
                        Some((n, r)) => (Some(n.trim_matches('\'')), r),
                        None => (None, loc),
                    };
                    if let Some(n) = name {
                        match self.book.sheets.iter().position(|s| s.name == n) {
                            Some(i) => self.active = i,
                            None => {
                                self.status = ui::tf!("シート「{}」が見つかりません", n).into();
                                return;
                            }
                        }
                    }
                    let mut it = refs.split(':');
                    let a = it.next().and_then(Pos::parse);
                    let b = it.next().and_then(Pos::parse);
                    if let Some(a) = a {
                        self.anchor = b.map(|_| a);
                        self.cursor = b.unwrap_or(a);
                        self.sync_input();
                        self.status = ui::tf!("リンク先 {} へ移動しました", loc).into();
                    } else {
                        self.status = ui::tf!("リンク先({})が場所として読めません", loc).into();
                    }
                    return;
                }
                let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
                self.status = ui::tf!("開きます: {}", url).into();
                return;
            }
        }
        if !self.commit() {
            // 入力規則で戻された。移動すると打った文字が黙って消えるので留まる
            return;
        }
        // 刷毛(書式のコピー)を持っていたら、押した先に塗って手放す
        if let Some(f) = self.brush.take() {
            self.checkpoint();
            let (a, b) = if shift && self.anchor.is_some() {
                self.sel_rect()
            } else {
                (p, p)
            };
            for r in a.row..=b.row {
                for cch in a.col..=b.col {
                    let q = Pos::new(r, cch);
                    let mut cell = self.sheet().get(q).cloned().unwrap_or_default();
                    cell.fmt = f.clone();
                    self.sheet_mut().set(q, cell);
                }
            }
            self.dirty = true;
            self.cursor = p;
            self.sync_input();
            self.status = ui::tf!("{} に書式を塗りました(Ctrl+Z で戻せます)", p.a1()).into();
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
                self.ref_disp(p)
            } else {
                format!(
                    "{}:{}",
                    self.ref_disp(Pos::new(ra, ca)),
                    self.ref_disp(Pos::new(rb, cb))
                )
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
        if self.img_drag.take().is_some() {
            return; // 画像の移動・大きさの確定。status はドラッグ中に出している
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
                self.menu_head = Some(true);
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
                self.menu_head = Some(false);
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
        self.menu_head = None;
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
        // 範囲を選んでいなければ従来どおり: カーソル列で表全体
        if self.anchor.is_none() {
            self.sort_col(self.cursor.col, asc);
            return;
        }
        let (a, b) = self.sel_rect();
        if a == b {
            self.sort_col(self.cursor.col, asc);
            return;
        }
        // 選択の左右(同じ行)に続きのデータがあるか。あるなら本家と同じく
        // 「拡張して並べ替え/選択だけ」を聞く — 黙って行をずらさない
        let filled = |p: Pos| {
            self.sheet().get(p).map(|c| !c.editable().trim().is_empty()).unwrap_or(false)
        };
        let neighbor = (a.row..=b.row).any(|r| {
            let left = a.col > 0 && filled(Pos::new(r, a.col - 1));
            left || filled(Pos::new(r, b.col + 1))
        });
        if neighbor {
            let at = self
                .cell_origin_px(self.cursor)
                .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
            self.sort_pend = Some(asc);
            self.pick_kind = "sort-expand";
            self.pick = Some((
                vec![
                    "拡張して並べ替え(続きの列も一緒に動く)".into(),
                    "選択した範囲だけ並べ替え(横の列とはずれます)".into(),
                    "やめる".into(),
                ],
                at,
            ));
            self.status =
                ui::t!("選択の横にデータが続いています。どう並べ替えますか?").into();
            return;
        }
        self.sort_range_now(a, b, asc);
    }

    /// 選んだ範囲だけを並べ替える(確認の後もここに来る)
    pub(crate) fn sort_range_now(&mut self, a: Pos, b: Pos, asc: bool) {
        self.commit();
        self.checkpoint();
        self.book.sheets[self.active].sort_range(a, b, self.cursor.col, asc);
        self.dirty = true;
        recalc_book(&mut self.book, self.active);
        self.sync_input(); // 古い控えの書き戻しを防ぐ(sort_col と同じ)
        self.status = ui::tf!(
            "{}:{} を{}に並べ替えました(範囲の中だけ。Ctrl+Z で1手)",
            a.a1(), b.a1(),
            if asc { "昇順" } else { "降順" }
        )
        .into();
    }

    /// カーソルのセルの色(塗り/文字色)を上に集める並べ替え
    pub(crate) fn sort_color_top(&mut self, use_fill: bool) {
        let fmt = self.sheet().get(self.cursor).map(|c| c.fmt.clone()).unwrap_or_default();
        let Some(target) = (if use_fill { fmt.fill } else { fmt.color }) else {
            self.status = if use_fill {
                ui::t!("このセルに塗りつぶしの色がありません").into()
            } else {
                ui::t!("このセルの文字に色が付いていません").into()
            };
            return;
        };
        self.commit();
        self.checkpoint();
        let col = self.cursor.col;
        self.book.sheets[self.active].sort_color_top(col, use_fill, &target, true);
        self.dirty = true;
        recalc_book(&mut self.book, self.active);
        self.sync_input(); // 古い控えの書き戻しを防ぐ(sort_col と同じ)
        self.status = if use_fill {
            ui::t!("セルの色が同じ行を上に集めました").into()
        } else {
            ui::t!("フォントの色が同じ行を上に集めました").into()
        };
    }

    /// 指定の列で並べ替え(▼の板の昇順/降順もここに来る)
    fn sort_col(&mut self, c: u32, asc: bool) {
        self.commit();
        self.checkpoint();
        self.book.sheets[self.active].sort_by_column(c, asc, true);
        self.dirty = true;
        recalc_book(&mut self.book, self.active);
        // 数式バーの控えを並べ替え後のセルに合わせる — 同期を怠ると、
        // 次の commit で並べ替え前の古い値が書き戻される
        self.sync_input();
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
        let (cur, mut text) = (self.cursor, self.input.text().to_string());
        // R1C1 で打った式は A1 に戻して仕舞う(中身はいつも A1)
        if self.book.r1c1 {
            if let Some(body) = text.strip_prefix('=') {
                text = format!("={}", sheet::model::formula_from_r1c1(body, cur));
            }
        }
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
        // 空白は「空白を無視」(allowBlank)が付いていれば許す(既定)。
        // 式は結果が変わり得るので通す
        if !text.starts_with('=') {
            // 判定は Validation::passes(判定できない規則は堰き止めない)。
            // 文言は規則に付いたエラーの文言が正、無ければ規則の言い直し
            let verdict = self.sheet().validation_at(cur).and_then(|v| {
                let ok = if text.trim().is_empty() {
                    v.allow_blank
                } else {
                    v.passes(self.sheet(), text.trim())
                };
                if ok {
                    None
                } else {
                    let fallback = if v.kind == "list" {
                        format!("候補: {}", v.options(self.sheet()).join(" / "))
                    } else {
                        v.describe()
                    };
                    Some((v.error_msg.clone(), fallback))
                }
            });
            if let Some((em, fallback)) = verdict {
                let stop = em.as_ref().map(|(s, _, _)| s == "stop").unwrap_or(true);
                let said = match &em {
                    Some((_, t, m)) if !t.is_empty() || !m.is_empty() => {
                        if t.is_empty() {
                            m.clone()
                        } else if m.is_empty() {
                            t.clone()
                        } else {
                            format!("{t}: {m}")
                        }
                    }
                    _ => fallback,
                };
                if stop {
                    self.status = ui::tf!(
                        "「{}」は入力規則に合いません({} / Esc で戻す)",
                        text.trim(), said
                    )
                    .into();
                    return false;
                }
                // 警告・情報は通すが言う(Excel の「警告」で続行した形)
                self.status = ui::tf!("入力規則に合いませんが、通しました({})", said).into();
            }
        }
        self.checkpoint();
        // **書式は据え置く。** 打ち直しただけで罫線や塗りが消えるのは帳票の事故
        let fmt = self.sheet().get(cur).map(|c| c.fmt.clone()).unwrap_or_default();
        let mut cell = Cell::input(&text);
        cell.fmt = fmt;
        // Alt+Enter の改行が入っていたら折り返しも立てる(Excel と同じ)
        if text.contains('\n') {
            cell.fmt.wrap = true;
        }
        self.sheet_mut().set(cur, cell);
        // 計算方法が手動なら待たされない(F9 / Shift+F9 で手回し)。
        // 今までは常に再計算していて「手動」が効いていなかった
        self.recalc_if_auto();
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
        if self.fn_args.is_some() {
            self.editor().backspace();
            self.fn_args_recalc();
        } else if let Some(d) = &mut self.fn_dlg {
            d.search.backspace();
            d.sel = 0;
        } else if self.name_edit.is_some()
            || self.solver.is_some()
            || self.filter_panel.is_some()
            || self.dv_dlg.is_some()
            || self.prompt.is_some()
        {
            // 板・小窓の欄へ(editor() が今の宛先を知っている)
            self.editor().backspace();
        } else if self.editing() || self.edit_armed {
            self.input.backspace();
            self.dirty = true;
        } else {
            // セルの上での BackSpace = 中身を消す(Excel と同じ。書式は残る)
            self.clear_selection_now();
        }
        cx.notify();
    }

    /// セルの上での BackSpace / Delete の実体。選択(無ければいまのセル)の
    /// 中身を消す。書式は残す。保護中は断る
    fn clear_selection_now(&mut self) {
        if self.sheet().protected {
            self.status =
                ui::t!("シートが保護されています(保護タブの「シートを保護する」で解除)").into();
            return;
        }
        self.checkpoint();
        let n = self.clear_range();
        self.sync_input();
        self.status = format!("{n} セルの中身を消しました(書式は残る)").into();
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
        // 板・小窓の欄が開いていれば、その欄の1文字削除(セルに流さない)
        if self.name_edit.is_some()
            || self.fn_dlg.is_some()
            || self.solver.is_some()
            || self.filter_panel.is_some()
            || self.dv_dlg.is_some()
            || self.prompt.is_some()
        {
            self.editor().delete();
            cx.notify();
            return;
        }
        if self.fn_args.is_some() {
            self.editor().delete();
            self.fn_args_recalc();
            cx.notify();
            return;
        }
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
        if self.delete_selected_image() {
            cx.notify();
            return;
        }
        if self.editing() || self.edit_armed {
            // 編集中の Delete は1文字(いつもの文字カーソルの右)
            self.input.delete();
            self.dirty = true;
        } else {
            // セルの上での Delete = 中身を消す(選択があれば選択ぶん。Excel と同じ)
            self.clear_selection_now();
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
        if self.dv_dlg.is_some() {
            // 入力規則の板の Enter = OK(本家と同じ)
            self.dv_ok(cx);
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
    /// F9 = ブック全体の再計算(計算方法が手動のときの手回し。自動でも害はない)
    fn a_recalc(&mut self, _: &ui::Recalc, _: &mut Window, cx: &mut Context<Self>) {
        self.commit();
        recalc_book(&mut self.book, self.active);
        self.status = ui::t!("再計算しました(ブック全体)").into();
        cx.notify();
    }
    /// Shift+F9 = いまのシートだけ再計算(大きなブックで待たされない)
    fn a_recalc_sheet(&mut self, _: &ui::RecalcSheet, _: &mut Window, cx: &mut Context<Self>) {
        self.commit();
        recalc(&mut self.book.sheets[self.active]);
        self.status = ui::t!("再計算しました(このシートだけ)").into();
        cx.notify();
    }
    /// Ctrl+K = ハイパーリンク(Excel と同じ)
    fn a_ins_link(&mut self, _: &ui::InsLink, _: &mut Window, cx: &mut Context<Self>) {
        self.run_cmd("inshyperlink", cx);
    }
    /// Ctrl+= / Ctrl+- = 画面の文字の大きさ(リボンから状態行まで全部)
    fn a_ui_bigger(&mut self, _: &ui::UiBigger, _: &mut Window, cx: &mut Context<Self>) {
        self.run_cmd("ui-bigger", cx);
    }
    fn a_ui_smaller(&mut self, _: &ui::UiSmaller, _: &mut Window, cx: &mut Context<Self>) {
        self.run_cmd("ui-smaller", cx);
    }
    /// Alt+Enter = セルの中の改行(Excel と同じ)。確定時に折り返しも立てる
    fn a_newline(&mut self, _: &ui::NewLine, _: &mut Window, cx: &mut Context<Self>) {
        if self.editing() || self.edit_armed {
            self.input.insert("\n");
            cx.notify();
        }
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

    /// 結合の種類を選んだ後の入り口。**値は消さない** — 左上以外の値は
    /// 隠れるだけで、解除で戻る。値が2つ以上見えているときは先に聞く
    /// (本家と同じ — 画面と Excel では各結合の左上しか見えなくなるから)
    pub(crate) fn merge_selection(&mut self, kind: &str) {
        let (a, b) = self.sel_rect();
        if a == b {
            self.status = ui::t!("結合する範囲を Shift+矢印で選んでください").into();
            return;
        }
        if kind == "解除" {
            self.checkpoint();
            let before = self.book.sheets[self.active].merges.len();
            self.book.sheets[self.active].merges.retain(|(x, y)| {
                y.row < a.row || x.row > b.row || y.col < a.col || x.col > b.col
            });
            let n = before - self.book.sheets[self.active].merges.len();
            self.status = ui::tf!("{} 個の結合を解きました", n).into();
            self.dirty = true;
            return;
        }
        let filled = (a.row..=b.row)
            .flat_map(|r| (a.col..=b.col).map(move |c| Pos::new(r, c)))
            .filter(|p| {
                self.sheet().get(*p).map(|c| !c.editable().trim().is_empty()).unwrap_or(false)
            })
            .count();
        if filled >= 2 {
            let at = self
                .cell_origin_px(self.cursor)
                .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
            self.merge_kind_pend = Some(kind.to_string());
            self.pick_kind = "merge-confirm";
            self.pick = Some((
                vec![
                    "結合する(見えるのは左上の値だけになります)".into(),
                    "やめる".into(),
                ],
                at,
            ));
            self.status =
                ui::t!("範囲に値が複数あります(値は消しません — 解除で戻ります)").into();
            return;
        }
        self.merge_do(a, b, kind);
    }

    /// 結合の実体(確認の後もここに来る)。kind: 中央/横方向/結合だけ
    pub(crate) fn merge_do(&mut self, a: Pos, b: Pos, kind: &str) {
        self.checkpoint();
        let sh = &mut self.book.sheets[self.active];
        sh.merges.retain(|(x, y)| {
            // 重なる結合は先に外す(入れ子の結合は帳票を壊す)
            y.row < a.row || x.row > b.row || y.col < a.col || x.col > b.col
        });
        match kind {
            // 横方向: 行ごとに1本ずつ(本家の Merge Across)
            "横方向" => {
                for r in a.row..=b.row {
                    sh.merges.push((Pos::new(r, a.col), Pos::new(r, b.col)));
                }
                self.status = ui::tf!(
                    "{}:{} を横方向に結合しました({} 行ぶん)",
                    a.a1(), b.a1(), b.row - a.row + 1
                )
                .into();
            }
            // 結合だけ(揃えは触らない — 本家の Merge Cells)
            "結合だけ" => {
                sh.merges.push((a, b));
                self.status = ui::tf!("{}:{} を結合しました(揃えはそのまま)", a.a1(), b.a1()).into();
            }
            _ => {
                sh.merges.push((a, b));
                // 名のとおり中央揃えも掛ける(解くときは揃えを触らない)
                let mut anchor = sh.get(a).cloned().unwrap_or_default();
                anchor.fmt.align = sheet::model::HAlign::Center;
                anchor.fmt.valign = sheet::model::VAlign::Middle;
                sh.set(a, anchor);
                self.status =
                    ui::tf!("{}:{} を結合し、中央に揃えました", a.a1(), b.a1()).into();
            }
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

    /// 画像(グラフ)の当たり判定。このアプリで挿した分(images_new)だけ —
    /// 読み込んだ画像は原文持ち越しが正なので動かせない(押すとそう言う)
    fn image_at(&self, x: f32, y: f32) -> Option<(usize, (f32, f32), bool)> {
        for (i, im) in self.sheet().images_new.iter().enumerate().rev() {
            let Some((sx, sy)) = self.cell_origin_px(im.at) else { continue };
            let (sx, sy) = (sx + im.dx_px, sy + im.dy_px);
            let (w, h) = (im.width_px, im.height_px);
            if x >= sx && x <= sx + w && y >= sy && y <= sy + h {
                let corner = x >= sx + w - 12.0 && y >= sy + h - 12.0;
                return Some((i, (sx, sy), corner));
            }
        }
        None
    }

    /// 読み込んだ画像(動かせない方)の上か
    fn read_image_at(&self, x: f32, y: f32) -> bool {
        self.sheet().images.iter().any(|im| {
            self.cell_origin_px(im.at).is_some_and(|(sx, sy)| {
                let (sx, sy) = (sx + im.dx_px, sy + im.dy_px);
                x >= sx && x <= sx + im.width_px && y >= sy && y <= sy + im.height_px
            })
        })
    }

    /// 画像のドラッグ(移動 or 右下の掴みで大きさ変更)。図形と同じ作法
    fn image_drag_at(&mut self, x: f32, y: f32) {
        let Some((i, (gx, gy), (ox, oy), resize)) = self.img_drag else { return };
        if self.sheet().images_new.len() <= i {
            return;
        }
        if resize {
            // 比を保って大きさを変える(絵が歪まない)
            let im = &mut self.sheet_mut().images_new[i];
            let ratio = if im.width_px > 0.0 { im.height_px / im.width_px } else { 1.0 };
            im.width_px = (x - ox).max(16.0);
            im.height_px = (im.width_px * ratio).max(16.0);
            let (w, h) = (im.width_px, im.height_px);
            self.dirty = true;
            self.status = format!("大きさ: {w:.0}×{h:.0}px").into();
        } else {
            let (nx, ny) = (ox + x - gx, oy + y - gy);
            if let (Some(c), Some(r)) = (self.col_at(nx.max(HEAD_W)), self.row_at(ny.max(ROW_H))) {
                let at = Pos::new(r, c);
                if let Some((cx0, cy0)) = self.cell_origin_px(at) {
                    let (dx, dy) = ((nx - cx0).max(0.0), (ny - cy0).max(0.0));
                    let im = &mut self.sheet_mut().images_new[i];
                    if im.at != at || (im.dx_px - dx).abs() > 0.5 || (im.dy_px - dy).abs() > 0.5 {
                        im.at = at;
                        im.dx_px = dx;
                        im.dy_px = dy;
                        self.dirty = true;
                        self.status = format!("画像を {} に留めました", at.a1()).into();
                    }
                }
            }
        }
    }

    /// 選んだ画像を消す(Del の実体)
    pub(crate) fn delete_selected_image(&mut self) -> bool {
        let Some(i) = self.img_sel.take() else { return false };
        if self.sheet().images_new.len() <= i {
            return false;
        }
        self.checkpoint();
        self.sheet_mut().images_new.remove(i);
        self.dirty = true;
        self.status = ui::t!("画像を削除しました(Ctrl+Z で戻せます)").into();
        true
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
    /// オートフィルタで残る行か。範囲の外と見出し行は常に残す
    fn filter_keeps(&self, r: u32) -> bool {
        let Some(f) = &self.auto_filter else { return true };
        let (a, b) = f.range;
        if r <= a.row || r > b.row {
            return true;
        }
        for (col, hide) in &f.hide {
            let v = self
                .sheet()
                .get(Pos::new(r, *col))
                .map(|c| c.value.display())
                .unwrap_or_default();
            if hide.contains(&v) {
                return false;
            }
        }
        true
    }

    /// 絞り込みが実際に効いているか(どれかの列で値を隠している)
    fn filter_active(&self) -> bool {
        self.auto_filter.as_ref().is_some_and(|f| !f.hide.is_empty())
    }

    /// 絞り込みの「n 行中 m 行を表示」(範囲のデータ行で数える)
    fn filter_counts(&self) -> Option<(u32, u32)> {
        if !self.filter_active() {
            return None;
        }
        let (a, b) = self.auto_filter.as_ref()?.range;
        let total = b.row - a.row;
        let shown = ((a.row + 1)..=b.row).filter(|r| self.filter_keeps(*r)).count() as u32;
        Some((total, shown))
    }

    /// ▼の板に出す値の一覧(値, 件数)。**他の列の絞り込みは効かせたまま**
    /// この列の値を数える(Excel の作法)。1,000 種で切り、切ったら true
    fn filter_values(&self, col: u32) -> (Vec<(String, usize)>, bool) {
        let Some(f) = &self.auto_filter else { return (Vec::new(), false) };
        let (a, b) = f.range;
        let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
        for r in (a.row + 1)..=b.row {
            if self.sheet().row_hidden.contains(&r) {
                continue;
            }
            let mut ok = true;
            for (c2, hide) in &f.hide {
                if *c2 == col {
                    continue;
                }
                let v = self
                    .sheet()
                    .get(Pos::new(r, *c2))
                    .map(|c| c.value.display())
                    .unwrap_or_default();
                if hide.contains(&v) {
                    ok = false;
                    break;
                }
            }
            if !ok {
                continue;
            }
            let v = self
                .sheet()
                .get(Pos::new(r, col))
                .map(|c| c.value.display())
                .unwrap_or_default();
            *counts.entry(v).or_default() += 1;
        }
        let cut = counts.len() > 1000;
        (counts.into_iter().take(1000).collect(), cut)
    }

}

impl Drop for Calc {
    fn drop(&mut self) {
        // 置きっぱなしのロックは他の人の警告になってしまう。最後の保険
        self.release_lock();
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

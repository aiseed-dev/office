//! writer — docx互換のワープロ。calc とは**別のソフト**。
//!
//! 一つの巨大なスイートにしない。文書は writer、表計算は calc。
//! 共有するのは書式(docx/xlsx)と核(kumihan)、そして入力の結線(ui)だけ。
//!
//! **マクロは無い。** 文書の中に実行コードを置かないので、
//! 「開く=実行」という攻撃経路が最初から存在しない。
//!
//!   writer            空で開く
//!   writer 文書.docx  その文書を開く
//!
//! 打てる: 日本語(IME)・BackSpace/Delete・矢印・Shift+矢印で選択・Ctrl+A・
//!         Enter で改段落・Ctrl+Z/Ctrl+Shift+Z・Ctrl+S 保存・Ctrl+O 開く

use std::ops::Range;
use std::path::PathBuf;

use gpui::{
    div, prelude::*, px, rgb, size, App, Bounds, Context, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, SharedString, UTF16Selection, Window,
    WindowBounds, WindowOptions,
};
use gpui_platform::application;
use kumihan::{layout, Align, Document, Editor, Frame, ListKind, Metrics, Sheet as Page};
use ui::{handler, ribbon, HasEditor};

const PX_PER_MM: f32 = 96.0 / 25.4;
/// 見開きのページの間の空き(mm)
const PAGE_GAP_MM: f32 = 8.0;

/// `|語《よみ》` の記法をほどき、(素の文, [(本文の範囲, 読み)]) を返す。
/// 範囲は base からのバイト位置(差し込む先の頭)。
/// pywashi と同じ記法なので、あちらの資産とも行き来できる
fn strip_ruby_marks(src: &str, base: usize) -> (String, Vec<(std::ops::Range<usize>, String)>) {
    let mut plain = String::with_capacity(src.len());
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(i) = rest.find('|').or_else(|| rest.find('｜')) {
        let mark_len = rest[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        let after = &rest[i + mark_len..];
        let Some(o) = after.find('《') else {
            plain.push_str(&rest[..i + mark_len]);
            rest = after;
            continue;
        };
        let Some(c) = after[o..].find('》') else {
            plain.push_str(&rest[..i + mark_len]);
            rest = after;
            continue;
        };
        plain.push_str(&rest[..i]);
        let word = &after[..o];
        let yomi = &after[o + '《'.len_utf8()..o + c];
        let start = base + plain.len();
        plain.push_str(word);
        out.push((start..base + plain.len(), yomi.to_string()));
        rest = &after[o + c + '》'.len_utf8()..];
    }
    plain.push_str(rest);
    (plain, out)
}

/// AI に頼む仕事。**返事をどう使うかまで決めてから頼む**
/// (使い道の決まっていない答えは受け取らない)
#[derive(Clone, Debug)]
enum AiJob {
    /// 選択(無ければ全文)を要約して、頭に置く
    Summary,
    /// 選択を書き直して置き換える(整える・敬語・やさしく)
    Rewrite(&'static str, &'static str),
    /// 選択を訳して置き換える
    Translate,
    /// 選択にふりがな(ルビ)を振る
    Furigana,
    /// カーソルの後ろへ続きを書く
    Continue,
    /// 選択を表にして、その場に置く
    Table,
    /// 自由に頼む(答えはカーソルの位置へ挿す)
    Ask(String),
    /// マクロ台本を書かせる(答えは文書に入れず、プラグイン置き場に
    /// .py で置く — 人が読んで確かめてから実行する。自動では走らせない)
    Macro(String),
}

impl AiJob {
    /// モデルへの言いつけ(system)と、何を渡すか
    fn prompt(&self) -> (&'static str, &'static str) {
        match self {
            AiJob::Summary => (
                "あなたは日本語の文書を扱う道具です。渡された文章の要点を、                 箇条書きではなく2〜4文の日本語でまとめてください。                 前置き・後書き・見出しは書かず、要約の本文だけを返します。",
                "次の文章を要約してください。",
            ),
            AiJob::Rewrite(sys, ask) => (sys, ask),
            AiJob::Translate => (
                "あなたは翻訳の道具です。日本語なら英語へ、それ以外なら日本語へ                 訳します。訳文だけを返し、説明や引用符は付けません。",
                "次を訳してください。",
            ),
            AiJob::Furigana => (
                "あなたは日本語のふりがなを付ける道具です。渡された文章のうち、                 読みが難しい漢字の語にだけ、|語《よみ》 の形でふりがなを付けて                 返します。文字そのものは1字も変えず、《》以外は足しません。                 やさしい語には付けません。本文だけを返します。",
                "次にふりがなを付けてください。",
            ),
            AiJob::Continue => (
                "あなたは日本語の文書を書き継ぐ道具です。渡された文章の続きを、                 同じ調子・同じ文体で2〜4文だけ書きます。前置きは書かず、                 続きの本文だけを返します。",
                "次の文章の続きを書いてください。",
            ),
            AiJob::Table => (
                "あなたは文章を表に整える道具です。渡された文章から表を作り、                 各行を | で区切った形(1行目は見出し)だけを返します。                 説明・前置き・記号の罫線は書きません。",
                "次を表にしてください。",
            ),
            AiJob::Ask(_) => (
                "あなたは日本語の文書を扱う道具です。頼まれたことに対する答えの                 本文だけを返します。前置き・後書き・見出しは書きません。",
                "",
            ),
            // 台本の作法 = サンドボックスの中の python-docx。前置きの関数だけを使わせ、
            // ラベル走査と cell(i,j) ループ(実測 140 倍遅い)を禁じる
            AiJob::Macro(_) => (
                "あなたは writer(docx 互換ワープロ)のマクロ台本を書く道具です。\
                 Python のコードだけを返してください(説明・前置き・\
                 コードフェンスは書かない)。台本はサンドボックスの中で実行され、\
                 d = python-docx の Document が渡されています。\
                 import docx / Document() / d.save() は書きません。\
                 記入欄の読み書きは必ず次の関数を使います: \
                 fill(名前, 値)=名前の記入欄すべてに書く / \
                 fill_one(名前, 値)=最初の一つに書く / \
                 extract(名前)=値を読む / fields()=(名前, 値)の一覧 / \
                 render(辞書)={{名前}} 雛形への差し込み / \
                 tpl_fields()=差し込み口の一覧。\
                 ラベルの文字列を探して隣のセルに書く走査はしません(誤爆する)。\
                 表を歩くときは for row in tb.rows: for c in row.cells: の形にし、\
                 tb.cell(i, j) をループの中で呼びません(遅い)。\
                 ネットとファイルの読み書きはできません(サンドボックスの外に出ない)。\
                 最後に print で何をしたかを一行で報告し、できない・危うい頼みは \
                 raise SystemExit(\"理由\") で断ります。",
                "",
            ),
        }
    }

    fn label(&self) -> &'static str {
        match self {
            AiJob::Summary => "要約",
            AiJob::Rewrite(_, _) => "書き直し",
            AiJob::Translate => "翻訳",
            AiJob::Furigana => "ふりがな",
            AiJob::Continue => "続き",
            AiJob::Table => "表",
            AiJob::Ask(_) => "頼み",
            AiJob::Macro(_) => "マクロ台本",
        }
    }
}

/// gpui の文字は行の高さが既定で黄金比(1.618×文字サイズ)なので、
/// グリフは div の頭から余白の半分ぶん下に描かれる。自前で引く線
/// (変換の下線・下線・取り消し線・蛍光ペン)はそのぶん下げて
/// グリフの実位置に合わせる — 合わせないと下線が文字を横切る
const HALF_LEADING: f32 = 0.309; // (1.618 - 1) / 2
const MARGIN_MM: f32 = 20.0;
const MEASURE_MM: f32 = 210.0 - 2.0 * MARGIN_MM;
const SIZE_PT: f32 = 10.5;
const LINE_MM: f32 = 6.4;
const Y0_MM: f32 = 24.0;

/// いま編集しているもの。本文か、表のセルか。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    Body,
    Cell { table: usize, row: usize, col: usize },
}

struct Writer {
    focus: FocusHandle,
    doc: Document,
    ed: Editor,
    page: Page,
    path: Option<PathBuf>,
    status: SharedString,
    notes: Vec<SharedString>,
    dirty: bool,
    /// マウスでドラッグ選択の途中か(押した位置から離すまで選択を伸ばす)
    drag_select: bool,
    /// 右クリックのメニュー(出ている場所。編集領域の px)
    menu_at: Option<(f32, f32)>,
    /// 選んでいるリボンのタブ
    tab: usize,
    /// 画面に使う書体名(文書の指定に従う)
    font_name: SharedString,
    /// 画面の倍率。**紙は変わらない** — 見る大きさだけの話
    zoom: f32,
    /// 縦のスクロール(紙の座標 mm)。0 が紙の頭
    scroll_mm: f32,
    /// 編集領域の高さ(px)。描画のたびに実測し、キャレット追従に使う
    view_h_px: f32,
    /// いま編集しているもの。**Editor は常にこの対象の文章を持つ**
    target: Target,
    /// 記号の一覧を出しているか
    symbols: bool,
    /// 編集記号(段落記号・空白)を見せるか
    show_marks: bool,
    /// ルーラー(mm の目盛り)を見せるか
    ruler: bool,
    /// 行番号を見せるか(見え方だけ。文書は変わらない)
    line_numbers: bool,
    /// コメントの印と一覧を見せるか(見え方だけ)
    show_comments: bool,
    /// フォントの一覧を出しているか
    font_list: bool,
    /// 大きさの一覧を出しているか
    size_list: bool,
    /// 段落のスタイルの一覧を出しているか
    style_list: bool,
    /// ダークモード(紙以外を暗く。文書は変わらない)
    dark: bool,
    /// 画像の実体 → gpui の画像(作り直すと毎フレーム復号されるため控える)
    image_cache: std::collections::HashMap<usize, std::sync::Arc<gpui::Image>>,
    /// 組版に使うフォントの実体。**文書の書体に従う**(開くたびに引き直す)
    font_bytes: std::sync::Arc<Vec<u8>>,
    /// 用紙。**文書の設定に従う**(既定 A4・余白20mm)
    pg: kumihan::PageSetup,
    /// 置換のパネル。開いている間、打鍵は検索欄に入る
    find_open: bool,
    /// 0=検索語 1=置換後
    find_field: usize,
    find_ed: Editor,
    repl_ed: Editor,
    /// ヘッダー・フッターの編集のパネル。Some(false)=ヘッダー / Some(true)=フッター。
    /// 開いている間、打鍵はここに入る(検索のパネルと同じ方式)
    hf_edit: Option<bool>,
    hf_ed: Editor,
    /// コメントのパネル(開いている間、打鍵はここに入る)と、付け先の段落番号
    cmt_edit: bool,
    cmt_ed: Editor,
    cmt_para: usize,
    /// 透かしのパネル
    wm_edit: bool,
    wm_ed: Editor,
    /// しおりのパネル(名前の入力欄つきの一覧)
    bm_open: bool,
    bm_ed: Editor,
    /// バージョン履歴のパネル(上書き保存のたびに残る控えの一覧)
    hist_open: bool,
    /// プラグインのパネル(置き場の .py 一覧)
    plug_open: bool,
    /// リボンの絵ボタンに乗ったときの説明(下のステータスバーに出す)
    hover_hint: Option<&'static str>,
    /// 編集領域の幅(px。ページ幅に合わせるの計算に使う)
    view_w_px: f32,
    /// 左パネル(ナビゲーション)。0=見出し 1=コメント 2=検索
    nav_open: bool,
    nav_tab: u8,
    /// 右パネル(いる場所の設定を直す盤)
    rp_open: bool,
    /// 表示の入切(本家の表示タブ)。ボタンの帯・ステータスバー・右のパネル
    show_toolbar: bool,
    show_statusbar: bool,
    /// ファイルのページ(タブ0)から戻る先のタブ
    prev_tab: usize,
    /// ファイルのページの右側(0=詳細情報 1=最近開いた)
    file_view: u8,
    /// 文書の情報で編集中の欄(0=作成者 1=タイトル 2=タグ 3=件名 4=コメント)
    file_field: Option<u8>,
    prop_ed: Editor,
    /// HTML の記入(form)。開いた HTML の欄と、送り先の起点
    html_forms: Vec<kumihan::html::Form>,
    html_links: Vec<(String, String)>,
    html_origin: Option<String>,
    /// 相対リンクを解く土台(開いた URL そのもの)
    html_base: Option<String>,
    lk_open: bool,
    fm_open: bool,
    fm_field: Option<usize>,
    fm_ed: Editor,
    /// URL を開くパネル
    url_open: bool,
    url_ed: Editor,
    /// いまの配色(レイアウト > 配色の変更)
    theme: usize,
    /// AI に自由に頼むパネル
    ai_open: bool,
    ai_ed: Editor,
    /// AI が働いている間は真(二重に頼まない)
    ai_busy: bool,
    /// 複数ページ(見開き。画面だけの見え方 — 紙は1ページずつのまま)
    multipage: bool,
    /// 記入欄の選択肢を聞くパネル(コンボ・ドロップダウンを挿すとき)
    sd_open: bool,
    sd_ed: Editor,
    sd_kind: kumihan::SdtKind,
    /// パネルが「選択肢」でなく「記入欄の名前」を聞いている
    sd_naming: bool,
    /// AI のパネルが「頼む」でなく「マクロ台本」を聞いている
    ai_macro: bool,
    /// 終了確認のパネル(未保存の変更があるときに出る。窓の中の中央)
    quit_ask: bool,
    /// ルビのパネル(選んだ字に読みを振る)
    rb_open: bool,
    rb_ed: Editor,
    rb_range: std::ops::Range<usize>,
    /// 暗号化のパスワード。Some なら保存で ECMA-376 Standard に包む
    encrypt_pw: Option<String>,
    /// パスワードのパネル。pw_pending が Some なら「開くために聞いている」
    pw_open: bool,
    pw_ed: Editor,
    pw_pending: Option<PathBuf>,
    /// マクロで置き換える直前の文書(Ctrl+Z で1手で戻すため)
    doc_undo: Option<Document>,
    /// チャット(文書の隣の申し送り帳)のパネルと入力欄
    chat_open: bool,
    chat_ed: Editor,
    /// 相互参照のパネル(しおり一覧から「文字」「ページ」を挿す)
    xr_open: bool,
    /// 描画の道具(0=ペン 1=蛍光ペン 2=消しゴム)。Some の間はマウスが筆
    tool: Option<u8>,
    /// 書きかけの筆
    ink_cur: Option<kumihan::Stroke>,
    /// 筆の取り消しの控え(1操作 = 1枚)
    ink_undo: Vec<Vec<kumihan::Stroke>>,
    /// ページの繰り上げ量(紙と同じ折り方)。筆のページ⇔巻物の変換に使う
    page_offsets: Vec<f32>,
    /// 変更履歴を記録中か。記録開始時点の段落の写しを持つ
    track: bool,
    track_base: Option<Vec<String>>,
    /// 自分が置いた排他ロック(.~lock.名前#)。閉じるときに外す
    my_lock: Option<PathBuf>,
    /// 先客の名乗り(user@host)。居る間は上書き保存をしない
    locked_by: Option<String>,
    /// 紙面に出すヘッダー・フッターの行(1ページ目の番号で組んだもの)
    header_lines: Vec<kumihan::Line>,
    footer_lines: Vec<kumihan::Line>,
    /// 校正の指摘(レビュー > 校正)。英語は辞書、日本語はモデル
    proof: Vec<ui::check::Finding>,
    proof_msg: SharedString,
    /// 辞書は起動時に1回だけ読む
    checker: ui::check::Checker,
}

impl HasEditor for Writer {
    fn editor(&mut self) -> &mut Editor {
        // 置換・ヘッダーのパネルが開いている間、入力(IME含む)はそちらへ入る。
        // 別の入力部品を作らず、同じ Editor と結線を使い回す
        if self.pw_open {
            &mut self.pw_ed
        } else if self.file_field.is_some() {
            &mut self.prop_ed
        } else if self.find_open {
            if self.find_field == 0 { &mut self.find_ed } else { &mut self.repl_ed }
        } else if self.hf_edit.is_some() {
            &mut self.hf_ed
        } else if self.cmt_edit {
            &mut self.cmt_ed
        } else if self.wm_edit {
            &mut self.wm_ed
        } else if self.bm_open {
            &mut self.bm_ed
        } else if self.url_open {
            &mut self.url_ed
        } else if self.fm_field.is_some() {
            &mut self.fm_ed
        } else if self.rb_open {
            &mut self.rb_ed
        } else if self.sd_open {
            &mut self.sd_ed
        } else if self.ai_open {
            &mut self.ai_ed
        } else if self.chat_open {
            &mut self.chat_ed
        } else {
            &mut self.ed
        }
    }
    fn editor_ref(&self) -> &Editor {
        if self.pw_open {
            &self.pw_ed
        } else if self.file_field.is_some() {
            &self.prop_ed
        } else if self.find_open {
            if self.find_field == 0 { &self.find_ed } else { &self.repl_ed }
        } else if self.hf_edit.is_some() {
            &self.hf_ed
        } else if self.cmt_edit {
            &self.cmt_ed
        } else if self.wm_edit {
            &self.wm_ed
        } else if self.bm_open {
            &self.bm_ed
        } else if self.url_open {
            &self.url_ed
        } else if self.fm_field.is_some() {
            &self.fm_ed
        } else if self.rb_open {
            &self.rb_ed
        } else if self.sd_open {
            &self.sd_ed
        } else if self.ai_open {
            &self.ai_ed
        } else if self.chat_open {
            &self.chat_ed
        } else {
            &self.ed
        }
    }
    fn on_edited(&mut self) {
        if self.pw_open || self.find_open {
            // パスワード・検索欄への打鍵は文書を変えない
            return;
        }
        if self.chat_open || self.file_field.is_some() || self.rb_open
            || self.url_open || self.fm_field.is_some() || self.sd_open
            || self.ai_open {
            // チャット・文書の情報・ルビの入力欄。打鍵は(確定まで)文書を変えない
            return;
        }
        if self.protected() {
            // 読み取り専用の保護。**打った分を取り消して、文書は変えない。**
            // パネル(ヘッダー等)の打鍵は文書に入る前なので、パネルごと閉じて捨てる
            if self.hf_edit.is_some() || self.wm_edit || self.cmt_edit {
                self.hf_edit = None;
                self.wm_edit = false;
                self.cmt_edit = false;
            }
            if !self.bm_open {
                self.ed.clear_marked();
                let want = match self.target {
                    Target::Body => self.doc.body_text(),
                    Target::Cell { table, row, col } => self
                        .doc
                        .tables()
                        .nth(table)
                        .and_then(|t| t.rows.get(row))
                        .and_then(|r| r.get(col))
                        .map(cell_text)
                        .unwrap_or_default(),
                };
                while self.ed.text() != want {
                    if !self.ed.undo() {
                        self.ed = Editor::new(&want);
                        break;
                    }
                }
            }
            self.status =
                ui::t!("読み取り専用で保護されています(保護タブの「保護」で解除できます)").into();
            return;
        }
        if let Some(footer) = self.hf_edit {
            // パネルの打鍵はその場で文書のヘッダー・フッターに反映する
            let text = self.hf_ed.text().to_string();
            let hf = if footer { &mut self.doc.footer } else { &mut self.doc.header };
            kumihan::set_paras_text(&mut hf.paragraphs, &text, SIZE_PT);
            self.dirty = true;
            self.refresh_hf();
            return;
        }
        if self.bm_open {
            // しおりのパネルは名前の入力欄。打鍵は文書を変えない
            return;
        }
        if self.wm_edit {
            // 透かしのパネル。空にすると外れる
            let text = self.wm_ed.text().to_string();
            self.doc.watermark = if text.is_empty() { None } else { Some(text) };
            self.dirty = true;
            return;
        }
        if self.cmt_edit {
            // コメントのパネル。空にすると外れる(1つ目のコメントを編集する)
            let text = self.cmt_ed.text().to_string();
            let author = std::env::var("USER").unwrap_or_else(|_| ui::t!("私").into());
            let pi = self.cmt_para;
            let mut i = 0usize;
            for b in &mut self.doc.blocks {
                if let kumihan::Block::Para(p) = b {
                    if i == pi {
                        if text.is_empty() {
                            if !p.comments.is_empty() {
                                p.comments.remove(0);
                            }
                        } else if let Some(c) = p.comments.first_mut() {
                            c.text = text.clone();
                        } else {
                            p.comments.push(kumihan::Comment { author, text: text.clone() });
                        }
                        break;
                    }
                    i += 1;
                }
            }
            self.dirty = true;
            return;
        }
        self.dirty = true;
        self.relayout();
        self.follow_caret();
    }
}

impl Writer {
    fn new(path: Option<PathBuf>, cx: &mut Context<Self>) -> Writer {
        let mut w = Writer {
            focus: cx.focus_handle(),
            doc: Document::default(),
            ed: Editor::new(""),
            page: Page::default(),
            path: None,
            status: "".into(),
            notes: Vec::new(),
            dirty: false,
            drag_select: false,
            menu_at: None,
            tab: 0,
            zoom: 1.0,
            scroll_mm: 0.0,
            view_h_px: 800.0,
            target: Target::Body,
            symbols: false,
            show_marks: false,
            ruler: true,
            line_numbers: false,
            show_comments: true,
            font_list: false,
            size_list: false,
            style_list: false,
            dark: false,
            image_cache: Default::default(),
            font_bytes: std::sync::Arc::new(font_data().to_vec()),
            pg: kumihan::PageSetup::default(),
            find_open: false,
            find_field: 0,
            find_ed: Editor::new(""),
            repl_ed: Editor::new(""),
            hf_edit: None,
            hf_ed: Editor::new(""),
            cmt_edit: false,
            cmt_ed: Editor::new(""),
            cmt_para: 0,
            wm_edit: false,
            wm_ed: Editor::new(""),
            bm_open: false,
            bm_ed: Editor::new(""),
            hist_open: false,
            plug_open: false,
            hover_hint: None,
            view_w_px: 900.0,
            nav_open: false,
            nav_tab: 0,
            rp_open: false,
            show_toolbar: true,
            show_statusbar: true,
            prev_tab: 1,
            file_view: 0,
            file_field: None,
            prop_ed: Editor::new(""),
            html_forms: Vec::new(),
            html_links: Vec::new(),
            html_origin: None,
            html_base: None,
            lk_open: false,
            fm_open: false,
            fm_field: None,
            fm_ed: Editor::new(""),
            url_open: false,
            url_ed: Editor::new(""),
            theme: 0,
            ai_open: false,
            ai_ed: Editor::new(""),
            ai_busy: false,
            multipage: false,
            sd_open: false,
            sd_ed: Editor::new(""),
            sd_kind: kumihan::SdtKind::Text,
            sd_naming: false,
            ai_macro: false,
            quit_ask: false,
            rb_open: false,
            rb_ed: Editor::new(""),
            rb_range: 0..0,
            encrypt_pw: None,
            pw_open: false,
            pw_ed: Editor::new(""),
            pw_pending: None,
            doc_undo: None,
            chat_open: false,
            chat_ed: Editor::new(""),
            xr_open: false,
            tool: None,
            ink_cur: None,
            track: false,
            track_base: None,
            my_lock: None,
            locked_by: None,
            ink_undo: Vec::new(),
            page_offsets: vec![0.0],
            header_lines: Vec::new(),
            footer_lines: Vec::new(),
            font_name: kumihan::font::for_document(None)
                .map(|(f, _)| SharedString::from(f.name.clone()))
                .unwrap_or_else(|_| "sans-serif".into()),
            proof: Vec::new(),
            proof_msg: "".into(),
            checker: ui::check::Checker::default(),
        };
        match path {
            Some(p) => w.open(p),
            None => {
                w.set_doc(Document::plain(
                    "ここに打てます。日本語入力(IME)もそのまま使えます。\n\
                     Ctrl+S で docx として保存、Ctrl+O で開く。マクロはありません。",
                    SIZE_PT,
                ));
                w.dirty = false;
            }
        }
        w
    }

    fn set_doc(&mut self, doc: Document) {
        self.ed = Editor::new(&doc.body_text());
        self.doc = doc;
        self.relayout();
    }

    /// 編集中のテキストを文書に反映してから組み直す。
    /// いまの編集内容を、編集先(本文かセル)へ書き戻す。
    fn flush_target(&mut self) {
        match self.target {
            Target::Body => self.doc.set_body_text(self.ed.text(), SIZE_PT),
            Target::Cell { table, row, col } => {
                let text = self.ed.text().to_string();
                if let Some(kumihan::Block::Table(tb)) = self
                    .doc
                    .blocks
                    .iter_mut()
                    .filter(|b| matches!(b, kumihan::Block::Table(_)))
                    .nth(table)
                {
                    if let Some(cell) = tb.rows.get_mut(row).and_then(|r| r.get_mut(col)) {
                        set_cell_text(cell, &text);
                    }
                }
            }
        }
    }

    /// 編集先を切り替える。いまの内容を書き戻してから、次の文章を持つ。
    fn switch_target(&mut self, next: Target) {
        if self.target == next {
            return;
        }
        self.flush_target();
        self.target = next;
        let text = match next {
            Target::Body => self.doc.body_text(),
            Target::Cell { table, row, col } => self
                .doc
                .tables()
                .nth(table)
                .and_then(|t| t.rows.get(row))
                .and_then(|r| r.get(col))
                .map(cell_text)
                .unwrap_or_default(),
        };
        self.ed = Editor::new(&text);
        self.status = match next {
            Target::Body => ui::t!("本文").into(),
            Target::Cell { row, col, .. } => {
                ui::tf!("表のセル({}行 {}列)を編集中", row + 1, col + 1).into()
            }
        };
    }

    fn relayout(&mut self) {
        self.flush_target();
        let m = Metrics::new(&self.font_bytes).expect("フォント");
        // 段組みなら1段の行長で組み、ページの物理座標へ折る。
        // 折った後の座標は画面もクリックも PDF もそのまま使える
        let y0 = self.pg.top_mm + 4.0;
        if self.doc.vertical {
            // 縦書き: 行長 = 紙の縦の使い幅で組み、右からの列へ写す(K4)
            let measure =
                (self.pg.h_mm - self.pg.top_mm - self.pg.bottom_mm - 8.0).max(20.0);
            self.page = layout(
                &self.doc,
                &m,
                &Frame { measure_mm: measure, line_height_mm: LINE_MM, y0_mm: y0 },
            );
            kumihan::fold_vertical(&mut self.page, &self.pg, y0, LINE_MM);
        } else {
            self.page = layout(
                &self.doc,
                &m,
                &Frame { measure_mm: self.pg.column_measure_mm(), line_height_mm: LINE_MM, y0_mm: y0 },
            );
            kumihan::fold_columns(&mut self.page, &self.pg, y0);
        }
        self.refresh_hf();
    }

    /// いまの紙面の総頁(紙と同じ折り方で数える)。
    fn total_pages(&self) -> usize {
        self.page_offsets.len().max(1)
    }

    /// 巻物の y → (ページ, ページの中の y)。筆はページに固定する。
    fn page_of_roll(&self, y: f32) -> (usize, f32) {
        let p = self.page_offsets.iter().rposition(|o| y >= *o - 0.01).unwrap_or(0);
        (p, y - self.page_offsets.get(p).copied().unwrap_or(0.0))
    }

    // ---- 描画(ペン・蛍光ペン・消しゴム) ----

    fn ink_begin(&mut self, x: f32, y_roll: f32) {
        let Some(tool) = self.tool else { return };
        if tool == 2 {
            self.ink_erase(x, y_roll);
            return;
        }
        let (page, y) = self.page_of_roll(y_roll);
        self.ink_cur = Some(kumihan::Stroke {
            page,
            highlighter: tool == 1,
            points: vec![(x, y)],
        });
    }

    fn ink_move(&mut self, x: f32, y_roll: f32) {
        if self.tool == Some(2) {
            self.ink_erase(x, y_roll);
            return;
        }
        let oy = self
            .ink_cur
            .as_ref()
            .and_then(|st| self.page_offsets.get(st.page))
            .copied()
            .unwrap_or(0.0);
        let Some(st) = self.ink_cur.as_mut() else { return };
        let y = y_roll - oy;
        if let Some((lx, ly)) = st.points.last() {
            if (x - lx).abs() + (y - ly).abs() < 0.4 {
                return; // 細かすぎる点は間引く
            }
        }
        st.points.push((x, y));
    }

    fn ink_end(&mut self) {
        if let Some(st) = self.ink_cur.take() {
            if st.points.len() >= 2 {
                self.ink_undo.push(self.doc.ink.clone());
                self.doc.ink.push(st);
                self.dirty = true;
            }
        }
    }

    /// 消しゴム。なぞった近く(3mm)に点を持つ筆を丸ごと消す。
    fn ink_erase(&mut self, x: f32, y_roll: f32) {
        let (page, y) = self.page_of_roll(y_roll);
        let near = |st: &kumihan::Stroke| {
            st.page == page
                && st.points.iter().any(|(sx, sy)| (sx - x).abs() < 3.0 && (sy - y).abs() < 3.0)
        };
        if self.doc.ink.iter().any(near) {
            self.ink_undo.push(self.doc.ink.clone());
            self.doc.ink.retain(|st| !near(st));
            self.dirty = true;
        }
    }

    /// 保存用の写し。筆(ペン)を、そのページに載っている段落の控えへ
    /// 図形(自由曲線)として差し込む。モデル本体は触らない —
    /// 保存のたびに増えないように、写しに差す。
    fn doc_for_save(&self) -> Document {
        let mut doc = self.doc.clone();
        // 相互参照は保存の写しで計算し直す(docx のキャッシュを新しく保つ。
        // 画面の平文はそのまま — 見えている値の更新は「参照を更新」で)
        doc.refresh_fields(|name, page| self.ref_value(name, page));
        // 変更履歴: 記録開始時点との差分を印の字にする(ooxml が w:ins/w:del に)
        if self.track {
            if let Some(base) = &self.track_base {
                use kumihan::{TRK_DEL_E, TRK_DEL_S, TRK_INS_E, TRK_INS_S};
                let cur: Vec<String> = doc.paragraphs().map(para_text).collect();
                let (marks, deleted) = track_diff(base, &cur);
                doc.track_author =
                    Some(std::env::var("USER").unwrap_or_else(|_| "writer".into()));
                let mut pi = 0usize;
                for b in &mut doc.blocks {
                    let kumihan::Block::Para(p) = b else { continue };
                    let mark = marks.get(pi).copied().unwrap_or(PMark::Same);
                    match mark {
                        PMark::Same => {}
                        PMark::New => {
                            let t = para_text(p);
                            let (pt, font, fmt) = p.runs.first()
                                .map(|r| (r.size_pt, r.font.clone(), r.fmt.clone()))
                                .unwrap_or((SIZE_PT, None, Default::default()));
                            p.runs = vec![kumihan::Run {
                                text: format!("{TRK_INS_S}{t}{TRK_INS_E}"),
                                size_pt: pt, font, fmt,
                            }];
                        }
                        PMark::Changed(bi) => {
                            let t = para_text(p);
                            let (pre, del, ins, suf) = split_diff(&base[bi], &t);
                            let (pt, font, fmt) = p.runs.first()
                                .map(|r| (r.size_pt, r.font.clone(), r.fmt.clone()))
                                .unwrap_or((SIZE_PT, None, Default::default()));
                            let mut text = pre;
                            if !del.is_empty() {
                                text.push(TRK_DEL_S);
                                text.push_str(&del);
                                text.push(TRK_DEL_E);
                            }
                            if !ins.is_empty() {
                                text.push(TRK_INS_S);
                                text.push_str(&ins);
                                text.push(TRK_INS_E);
                            }
                            text.push_str(&suf);
                            p.runs = vec![kumihan::Run { text, size_pt: pt, font, fmt }];
                        }
                    }
                    pi += 1;
                }
                // 消えた段落は、その場所に「全部削除」の段落として置く
                let pbi: Vec<usize> = doc.blocks.iter().enumerate()
                    .filter(|(_, b)| matches!(b, kumihan::Block::Para(_)))
                    .map(|(i, _)| i)
                    .collect();
                let mut dels = deleted.clone();
                dels.sort_by_key(|(at, _)| *at);
                for (at, bi) in dels.into_iter().rev() {
                    let pos = pbi.get(at).copied().unwrap_or(doc.blocks.len());
                    doc.blocks.insert(pos, kumihan::Block::Para(kumihan::Paragraph {
                        line_spacing: 1.0,
                        runs: vec![kumihan::Run {
                            text: format!("{TRK_DEL_S}{}{TRK_DEL_E}", base[bi]),
                            size_pt: SIZE_PT,
                            font: None,
                            fmt: Default::default(),
                        }],
                        ..Default::default()
                    }));
                }
            }
        }
        if doc.ink.is_empty() {
            return doc;
        }
        let (pages, _) = paper::paginate(&self.page, paper::Paper {
            width_mm: self.pg.w_mm,
            height_mm: self.pg.h_mm,
            margin_mm: self.pg.left_mm,
        });
        // ページ → そのページに最初に載る段落(通し番号)
        let mut starts: Vec<usize> = Vec::new();
        let mut at = 0usize;
        for p in doc.paragraphs() {
            starts.push(at);
            at += p.runs.iter().map(|r| r.text.len()).sum::<usize>() + 1;
        }
        let mut page_para: std::collections::BTreeMap<usize, usize> = Default::default();
        for (l, pg) in self.page.lines.iter().zip(&pages) {
            if !l.from_body {
                continue;
            }
            let pi = starts.iter().rposition(|s| *s <= l.byte0).unwrap_or(0);
            page_para.entry(pg - 1).or_insert(pi);
        }
        let para_block_idx: Vec<usize> = doc
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| matches!(b, kumihan::Block::Para(_)))
            .map(|(i, _)| i)
            .collect();
        let ink = std::mem::take(&mut doc.ink);
        for (i, st) in ink.iter().enumerate() {
            let pi = page_para.get(&st.page).copied().unwrap_or(0);
            let Some(bi) = para_block_idx.get(pi).copied() else { continue };
            if let Some(kumihan::Block::Para(p)) = doc.blocks.get_mut(bi) {
                p.anchors.push(ooxml::ink_anchor_run(st, 9001 + i));
            }
        }
        doc
    }

    /// 紙面に出すヘッダー・フッターの行を組み直す(番号は1ページ目のもの。
    /// 各ページの本当の番号は PDF で入る)。
    fn refresh_hf(&mut self) {
        let m = Metrics::new(&self.font_bytes).expect("フォント");
        self.page_offsets = paper::paginate(&self.page, paper::Paper {
            width_mm: self.pg.w_mm,
            height_mm: self.pg.h_mm,
            margin_mm: self.pg.left_mm,
        }).1;
        // 複数ページ(見開き)。**画面だけ**の折り方 — PDF は 1ページずつ
        // (save_pdf は組み直してから写す)。縦書きとは併せない
        if self.multipage && !self.page.vertical {
            let offs = self.page_offsets.clone();
            kumihan::fold_pages(&mut self.page, &self.pg, &offs, 2, PAGE_GAP_MM);
        }
        let total = self.total_pages();
        self.header_lines =
            kumihan::layout_hf(&self.doc.header, &m, &self.pg, LINE_MM, 1, total, false);
        self.footer_lines =
            kumihan::layout_hf(&self.doc.footer, &m, &self.pg, LINE_MM, 1, total, true);
    }

    /// ヘッダー・フッターの編集のパネルを開く(もう一度で閉じる)。
    fn open_hf(&mut self, footer: bool) {
        if self.hf_edit == Some(footer) {
            self.hf_edit = None;
            return;
        }
        let hf = if footer { &self.doc.footer } else { &self.doc.header };
        let which = if footer { "フッター" } else { "ヘッダー" };
        if hf.paragraphs.is_empty() && hf.part.is_some() {
            // 読めたが持てなかった部品(表入りなど)。嘘の編集をさせない
            self.status = ui::tf!("この{}には表があり、この版では編集できません(保存では残ります)", which).into();
            return;
        }
        self.find_open = false;
        self.hf_edit = Some(footer);
        self.hf_ed = Editor::new(&kumihan::paras_text(&hf.paragraphs));
        self.status = ui::tf!("{}を編集中(全ページ共通。Esc で閉じる)", which).into();
    }

    /// 文書の書体を実体に結ぶ。無ければ系統を保って代替し、**そう言う**。
    fn adopt_font(&mut self) {
        let wanted = self.doc.font.clone();
        match kumihan::font::for_document(wanted.as_deref()) {
            Ok((fam, exact)) => {
                if let Ok(b) = kumihan::font::load(fam) {
                    self.font_bytes = std::sync::Arc::new(b);
                    self.font_name = SharedString::from(fam.name.clone());
                }
                if !exact {
                    if let Some(w) = &wanted {
                        self.notes.push(
                            ui::tf!("書体「{}」が無いので「{}」で表示", w, fam.name).into(),
                        );
                    }
                }
            }
            Err(e) => self.status = e.into(),
        }
    }

    /// パスワードのパネルの Enter。開き待ちがあれば解いて開き、
    /// 無ければ「次の保存から暗号化」を決める(空なら解除)
    fn pw_commit(&mut self) {
        let pw = self.pw_ed.text().to_string();
        if let Some(p) = self.pw_pending.take() {
            let bytes = match std::fs::read(&p) {
                Ok(b) => b,
                Err(e) => {
                    self.pw_open = false;
                    self.status = ui::tf!("開けません: {}", e).into();
                    return;
                }
            };
            match ooxml::crypt::decrypt(&bytes, &pw) {
                Ok(plain) => {
                    self.pw_open = false;
                    self.open_plain(p.clone(), plain);
                    if self.path.as_deref() == Some(p.as_path()) {
                        self.encrypt_pw = Some(pw);
                        self.status = ui::tf!("{}(保存も同じパスワードで暗号化します)", self.status)
                        .into();
                    }
                }
                Err(e) => {
                    // パネルは開いたまま。打ち直せる
                    self.pw_pending = Some(p);
                    self.pw_ed = Editor::new("");
                    self.status = e.into();
                }
            }
        } else {
            self.pw_open = false;
            if pw.is_empty() {
                self.encrypt_pw = None;
                self.status = ui::t!("暗号化しません(次の保存から普通の docx)").into();
            } else {
                self.encrypt_pw = Some(pw);
                self.dirty = true;
                self.status = ui::t!("次の保存から、このパスワードで暗号化します\
                               (AES-128。Word や LibreOffice でも開けます)").into();
            }
        }
    }

    /// 原本の中身(暗号化されていれば解いた平文)。部品の持ち越しに使う
    fn original_plain(&self) -> Option<Vec<u8>> {
        let bytes = std::fs::read(self.path.as_ref()?).ok()?;
        if ooxml::crypt::is_encrypted(&bytes) {
            let pw = self.encrypt_pw.as_ref()?;
            ooxml::crypt::decrypt(&bytes, pw).ok()
        } else {
            Some(bytes)
        }
    }

    /// 読み取り専用の保護が掛かっているか(保護タブの「保護」で入切)
    fn protected(&self) -> bool {
        self.doc.protection.is_some()
    }

    /// マクロ = **サンドボックス(bubblewrap)の中の Python** が python-docx で文書の
    /// **複製**を直し、直った複製を読み込む(失敗しても文書は無傷)。
    /// 文書にコードは載せない — 「開く=実行」を作らない設計はそのまま。
    /// 台本の中で d が python-docx の Document、fill(名前, 値) が
    /// 名前つき記入欄への記入(macro_script 参照)。戻すのは Ctrl+Z の1手
    fn run_macro_file(&mut self, py_file: PathBuf, cx: &mut Context<Self>) {
        self.flush_target();
        let user_code = match std::fs::read_to_string(&py_file) {
            Ok(c) => c,
            Err(e) => {
                self.status = ui::tf!("マクロが読めません: {}", e).into();
                return;
            }
        };
        let dir = std::env::temp_dir().join(format!("jo-wmacro-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let in_d = dir.join("in.docx");
        let out_d = dir.join("out.docx");
        // 複製は保存と同じ道で作る(原本の部品も持ち越す。暗号化は解いて)
        let original: Option<std::io::Cursor<Vec<u8>>> =
            self.original_plain().map(std::io::Cursor::new);
        let doc_out = self.doc_for_save();
        let w = std::fs::File::create(&in_d)
            .map_err(|e| e.to_string())
            .and_then(|f| ooxml::write_with(&doc_out, original, std::io::BufWriter::new(f)));
        if let Err(e) = w {
            self.status = ui::tf!("マクロに渡せません: {}", e).into();
            return;
        }
        let script = macro_script(&in_d, &out_d, &user_code);
        let name = py_file
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        self.status = ui::tf!("マクロ {} を実行しています…(サンドボックスの中の Python)", name).into();
        let task = cx.background_executor().spawn(async move {
            let py_path = dir.join("run.py");
            std::fs::write(&py_path, script).map_err(|e| e.to_string())?;
            let py = find_python();
            let have_bwrap = std::path::Path::new("/usr/bin/bwrap").exists();
            let mut cmd = if have_bwrap {
                // サンドボックス: / は読み取り専用、ホームは空、書けるのは作業場だけ、
                // ネット無し(calc の Python と同じサンドボックス)
                let venv = std::fs::canonicalize(".venv").unwrap_or_default();
                let mut c = std::process::Command::new("/usr/bin/bwrap");
                c.args(["--ro-bind", "/", "/", "--tmpfs", "/home", "--tmpfs", "/tmp"]);
                if venv.exists() {
                    c.arg("--ro-bind").arg(&venv).arg(&venv);
                }
                c.arg("--bind").arg(&dir).arg(&dir);
                c.args([
                    "--unshare-net",
                    "--dev",
                    "/dev",
                    "--proc",
                    "/proc",
                    "--die-with-parent",
                    "--new-session",
                    "--setenv",
                    "HOME",
                    "/tmp",
                    "--",
                ]);
                c.arg(&py);
                c
            } else {
                std::process::Command::new(&py)
            };
            let o = cmd
                .arg(&py_path)
                .output()
                .map_err(|e| ui::tf!("Python が起動できません: {}", e))?;
            let out = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !o.status.success() {
                let err = String::from_utf8_lossy(&o.stderr);
                let last = err
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("原因不明")
                    .to_string();
                return Err(if err.contains("No module named 'docx'") {
                    ui::t!("python-docx がありません(pip install python-docx。\
                     .venv があればそちらへ)").to_string()
                } else {
                    last
                });
            }
            std::fs::read(&out_d)
                .map_err(|e| ui::tf!("結果が読めません: {}", e))
                .map(|b| (b, out))
        });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok((bytes, out)) => {
                        match ooxml::read(std::io::Cursor::new(bytes)) {
                            Ok((doc, rep)) => {
                                this.doc_undo = Some(this.doc.clone());
                                this.target = Target::Body;
                                this.notes = rep
                                    .unsupported
                                    .iter()
                                    .map(|(n, c)| {
                                        SharedString::from(format!("{n} × {c}"))
                                    })
                                    .collect();
                                this.pg = doc.page.clone().unwrap_or_default();
                                this.set_doc(doc);
                                this.adopt_font();
                                this.relayout_keep();
                                this.dirty = true;
                                this.status = if out.is_empty() {
                                    ui::tf!("マクロ {} を実行しました(Ctrl+Z で戻せます)", name)
                                        .into()
                                } else {
                                    ui::tf!("マクロ {}: {}(Ctrl+Z で戻せます)", name, out.lines().last().unwrap_or_default())
                                    .into()
                                };
                            }
                            Err(e) => this.status = ui::tf!("結果が読めません: {}", e).into(),
                        }
                    }
                    Err(e) => this.status = ui::tf!("マクロ: {}", e).into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 最近開いた・保存した文書の控え(~/.config/office/recent-writer.txt)
    fn recent_file() -> PathBuf {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(".config/office/recent-writer.txt")
    }

    fn note_recent(p: &std::path::Path) {
        let rf = Self::recent_file();
        if let Some(dir) = rf.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut list: Vec<String> = std::fs::read_to_string(&rf)
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_default();
        let me = p.to_string_lossy().to_string();
        list.retain(|x| *x != me);
        list.insert(0, me);
        list.truncate(12);
        let _ = std::fs::write(&rf, list.join("\n"));
    }

    fn recent_list() -> Vec<PathBuf> {
        std::fs::read_to_string(Self::recent_file())
            .map(|s| s.lines().map(PathBuf::from).filter(|p| p.exists()).collect())
            .unwrap_or_default()
    }

    /// 新しい文書。未保存の変更があるときは作らない(黙って捨てない)。
    /// 返り値: 作ったか
    fn new_doc(&mut self) -> bool {
        if self.dirty {
            self.status =
                ui::t!("未保存の変更があります。先に保存してください(Ctrl+S)").into();
            return false;
        }
        self.release_lock();
        self.locked_by = None;
        self.path = None;
        self.encrypt_pw = None;
        self.notes = Vec::new();
        self.target = Target::Body;
        self.pg = kumihan::PageSetup::default();
        self.set_doc(Document::plain("", SIZE_PT));
        self.dirty = false;
        self.status = ui::t!("新しい文書です").into();
        true
    }

    /// 名前を付けて保存(いつでもダイアログ。別のスレッド — rfd は同期)
    fn save_as(&mut self, cx: &mut Context<Self>) {
        let ask = cx.background_executor().spawn(async {
            rfd::FileDialog::new().add_filter("Word文書", &["docx"]).save_file()
        });
        cx.spawn(async move |this, cx| {
            let r = ask.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(mut p) = r {
                    if p.extension().is_none() {
                        p.set_extension("docx");
                    }
                    this.save_to(p);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 文書の情報の欄を確定する(Enter)
    fn commit_prop(&mut self) {
        let Some(i) = self.file_field.take() else { return };
        if self.protected() {
            self.status =
                ui::t!("読み取り専用で保護されています(保護タブの「保護」で解除できます)").into();
            return;
        }
        let text = self.prop_ed.text().to_string();
        let pr = &mut self.doc.props;
        match i {
            0 => pr.creator = text,
            1 => pr.title = text,
            2 => pr.keywords = text,
            3 => pr.subject = text,
            _ => pr.description = text,
        }
        self.dirty = true;
        self.status = ui::t!("文書の情報を控えました(保存で docx に入ります)").into();
    }

    /// ルビのパネルの Enter。控えた範囲に読みを付ける(空なら外す)
    fn rb_commit(&mut self) {
        self.rb_open = false;
        let text = self.rb_ed.text().trim().to_string();
        let range = self.rb_range.clone();
        if range.is_empty() {
            return;
        }
        self.doc.set_body_text(self.ed.text(), SIZE_PT);
        let ruby = (!text.is_empty()).then(|| text.clone());
        self.doc.apply_char_format(range, move |f| f.ruby = ruby.clone());
        self.dirty = true;
        self.relayout_keep();
        self.status = if text.is_empty() {
            ui::t!("ルビを外しました").into()
        } else {
            ui::tf!("ルビ「{}」を振りました(保存で docx の w:ruby に)", text).into()
        };
    }

    /// 上書きの前に、直前の中身を控えとして残す(最大9世代)。
    /// 置き場は同じフォルダの .jo-history/<ファイル名>/<日時>.docx。
    /// 名前は**その中身を保存した日時**(ファイルの mtime)— いつの姿かが分かる
    fn keep_version(&self, p: &std::path::Path) {
        let Some(name) = p.file_name().map(|n| n.to_string_lossy().to_string()) else {
            return;
        };
        let dir = p
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join(".jo-history")
            .join(&name);
        if std::fs::create_dir_all(&dir).is_err() {
            return; // 控えられなくても保存は止めない
        }
        let stamp = std::process::Command::new("date")
            .arg("-r")
            .arg(p)
            .arg("+%Y%m%d-%H%M%S")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|| "0".into());
        let _ = std::fs::copy(p, dir.join(format!("{stamp}.docx")));
        // 増えすぎたら古い控えから消す
        if let Ok(rd) = std::fs::read_dir(&dir) {
            let mut old: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
            old.sort();
            while old.len() > 9 {
                let _ = std::fs::remove_file(old.remove(0));
            }
        }
    }

    /// 控えの一覧(新しい順)。(表示名, パス)
    fn versions(&self) -> Vec<(String, PathBuf)> {
        let Some(p) = &self.path else { return Vec::new() };
        let Some(name) = p.file_name().map(|n| n.to_string_lossy().to_string()) else {
            return Vec::new();
        };
        let dir = p
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join(".jo-history")
            .join(&name);
        let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
        let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        v.sort();
        v.reverse();
        v.into_iter()
            .map(|q| {
                let stem = q
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                // 20260804-183012 → 2026-08-04 18:30(名前は ASCII の日時)
                let disp = if stem.len() >= 13 && stem.is_ascii() {
                    format!(
                        "{}-{}-{} {}:{}",
                        &stem[0..4], &stem[4..6], &stem[6..8], &stem[9..11], &stem[11..13]
                    )
                } else {
                    stem
                };
                let kb = std::fs::metadata(&q).map(|m| m.len() / 1024).unwrap_or(0);
                (format!("{disp}({kb} KB)"), q)
            })
            .collect()
    }

    /// 控えを開く。いまのファイルは動かさず、**名無しの複製**として読む
    /// (保存すると名前を聞く。元へ戻したいなら同じ名前で保存する — 
    /// 黙って元のファイルを書き戻したりしない)
    fn open_version(&mut self, q: &std::path::Path) {
        let bytes = match std::fs::read(q) {
            Ok(b) => b,
            Err(e) => {
                self.status = ui::tf!("控えが読めません: {}", e).into();
                return;
            }
        };
        let bytes = if ooxml::crypt::is_encrypted(&bytes) {
            match self.encrypt_pw.as_ref().map(|pw| ooxml::crypt::decrypt(&bytes, pw)) {
                Some(Ok(b)) => b,
                _ => {
                    self.status =
                        ui::t!("控えは暗号化されています(いまのパスワードでは解けません)").into();
                    return;
                }
            }
        } else {
            bytes
        };
        match ooxml::read(std::io::Cursor::new(bytes)) {
            Ok((doc, rep)) => {
                self.release_lock();
                self.locked_by = None;
                self.hist_open = false;
                self.target = Target::Body;
                self.notes = rep
                    .unsupported
                    .iter()
                    .map(|(n, c)| SharedString::from(format!("{n} × {c}")))
                    .collect();
                self.pg = doc.page.unwrap_or_default();
                self.set_doc(doc);
                self.adopt_font();
                self.relayout_keep();
                self.path = None;
                self.dirty = true;
                self.status = ui::t!("控えを開きました(名無しの複製。保存で名前を聞きます。\
                               元へ戻すなら同じ名前で保存)").into();
            }
            Err(e) => self.status = ui::tf!("控えが読めません: {}", e).into(),
        }
    }

    /// チャット(申し送り帳)の置き場。文書の隣の 名前.docx.chat.txt
    fn chat_path(&self) -> Option<PathBuf> {
        self.path.as_ref().map(|p| {
            let mut os = p.as_os_str().to_owned();
            os.push(".chat.txt");
            PathBuf::from(os)
        })
    }

    /// 申し送りの最近の行(古い順で最大12行)
    fn chat_lines(&self) -> Vec<String> {
        let Some(cp) = self.chat_path() else { return Vec::new() };
        let Ok(text) = std::fs::read_to_string(cp) else { return Vec::new() };
        let mut v: Vec<String> =
            text.lines().rev().take(12).map(str::to_string).collect();
        v.reverse();
        v
    }

    /// 申し送り帳に名乗りと日時つきで1行書き足す
    fn chat_send(&mut self) {
        let text = self.chat_ed.text().trim().to_string();
        if text.is_empty() {
            return;
        }
        let Some(cp) = self.chat_path() else {
            self.status =
                ui::t!("まだファイルになっていません(保存すると申し送り帳が持てます)").into();
            return;
        };
        let stamp = std::process::Command::new("date")
            .arg("+%Y-%m-%d %H:%M")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        let line = format!("[{stamp}] {}: {text}\n", lock_identity());
        use std::io::Write as _;
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&cp)
            .and_then(|mut f| f.write_all(line.as_bytes()))
        {
            Ok(_) => {
                self.chat_ed = Editor::new("");
                self.status =
                    ui::t!("書き残しました(文書の隣の .chat.txt。開いた人が読めます)").into();
            }
            Err(e) => self.status = ui::tf!("チャットに書けません: {}", e).into(),
        }
    }

    /// 自分のロックを外す(閉じる・別のファイルへ移るとき)。
    fn release_lock(&mut self) {
        if let Some(lp) = self.my_lock.take() {
            let _ = std::fs::remove_file(lp);
        }
    }

    /// このファイルのロックを見て、先客が居れば警告、居なければ自分が取る。
    fn acquire_lock(&mut self, p: &std::path::Path) {
        self.release_lock();
        match foreign_lock(p) {
            Some(who) => {
                self.locked_by = Some(who);
                // ロックは取らない(先客の邪魔をしない)
            }
            None => {
                self.locked_by = None;
                let lp = lock_path_for(p);
                // LibreOffice と同じ気持ちの中身(名乗りだけ)
                if std::fs::write(&lp, format!("{},;", lock_identity())).is_ok() {
                    self.my_lock = Some(lp);
                }
            }
        }
    }

    fn open(&mut self, p: PathBuf) {
        let bytes = match std::fs::read(&p) {
            Ok(b) => b,
            Err(e) => {
                self.status = ui::tf!("開けません: {}", e).into();
                return;
            }
        };
        // HTML(JS なしの閲覧 — SEKKEI「writer の HTML」)
        if p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
            e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm")
        }) {
            self.open_html(&p, &bytes);
            return;
        }
        if ooxml::crypt::is_encrypted(&bytes) {
            // パネルでパスワードを聞き、Enter(pw_commit)が続きをやる
            self.pw_pending = Some(p);
            self.pw_open = true;
            self.pw_ed = Editor::new("");
            self.status =
                ui::t!("この文書は暗号化されています。パスワードを打って Enter").into();
            return;
        }
        self.open_plain(p, bytes);
    }

    /// HTML を開く。文書モデルに写すので、画面・PDF・docx 保存はそのまま
    /// 効く(HTML 書き出しは作らない — 互換は書式の境界で守る)。
    /// JS は実行しない。理解しない要素は帳簿へ。文字コードは UTF-8 → CP932
    fn open_html(&mut self, p: &std::path::Path, bytes: &[u8]) {
        let text = match std::str::from_utf8(bytes) {
            Ok(t) => t.to_string(),
            Err(_) => {
                let (t, _, bad) = encoding_rs::SHIFT_JIS.decode(bytes);
                if bad {
                    self.status =
                        ui::t!("文字コードが読めません(UTF-8 でも CP932 でもない)").into();
                    return;
                }
                t.into_owned()
            }
        };
        let (doc, notes, forms, links) = kumihan::html::parse_full(&text, SIZE_PT);
        self.html_forms = forms;
        self.html_links = links;
        self.fm_field = None;
        self.fm_open = !self.html_forms.is_empty();
        self.lk_open = !self.html_links.is_empty() && self.html_base.is_some();
        self.target = Target::Body;
        self.hf_edit = None;
        self.track = false;
        self.track_base = None;
        self.encrypt_pw = None;
        self.release_lock();
        self.locked_by = None;
        self.notes = notes.into_iter().map(SharedString::from).collect();
        self.pg = kumihan::PageSetup::default();
        self.set_doc(doc);
        self.adopt_font();
        self.relayout_keep();
        // 保存は docx として名前を聞く(HTML には書き戻さない)
        self.path = None;
        self.dirty = true;
        self.status = ui::tf!("HTML を読みました — {}(JS は実行しません。保存は docx{})", p.file_name().unwrap_or_default().to_string_lossy(), if self.fm_open { "。記入は右上のパネルから" } else { "" })
        .into();
    }

    /// URL のパネルの Enter。GET して HTML として開く(いま繋いだ相手が起点)
    fn url_commit(&mut self, cx: &mut Context<Self>) {
        let url = self.url_ed.text().trim().to_string();
        if url.is_empty() {
            return;
        }
        self.url_open = false;
        let task = cx.background_executor().spawn(async move { http_fetch(&url, None) });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok((bytes, final_url)) => this.adopt_fetched(&final_url, &bytes),
                    Err(e) => this.status = ui::tf!("開けません: {}", e).into(),
                }
                cx.notify();
            });
        })
        .detach();
        self.status = ui::tf!("取りに行っています… {}", self.url_ed.text()).into();
    }

    /// AI に頼んで、返事を文書に反映する。**別のスレッドで待つ**(画面は止めない)。
    /// 反映は必ず doc_undo に控えてから = **Ctrl+Z の1手で戻る**。
    /// 宛先が使えなければ理由を言う(黙って空にしない)
    fn ai_go(&mut self, job: AiJob, cx: &mut Context<Self>) {
        if self.protected() {
            self.status =
                ui::t!("読み取り専用で保護されています(保護タブの「保護」で解除できます)").into();
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
        self.switch_target(Target::Body);
        self.flush_target();
        let sel = self.ed.selection();
        let text = self.ed.text().to_string();
        // 渡すもの: 選択があればそこ、無ければ全文(続きはカーソルまで)
        let body = match &job {
            AiJob::Continue => text[..sel.end.min(text.len())].to_string(),
            AiJob::Macro(_) => String::new(),
            AiJob::Ask(_) if sel.is_empty() => String::new(),
            _ if sel.is_empty() => text.clone(),
            _ => text[sel.clone()].to_string(),
        };
        if body.trim().is_empty() && !matches!(job, AiJob::Ask(_) | AiJob::Macro(_)) {
            self.status = ui::t!("文章がありません(打つか、選んでから押してください)").into();
            return;
        }
        let (sys, ask) = job.prompt();
        let user = match &job {
            AiJob::Ask(q) => {
                if body.trim().is_empty() {
                    q.clone()
                } else {
                    format!("{q}\n\n---\n{body}")
                }
            }
            // マクロには本文でなく、記入欄の名前一覧を渡す(台本の的)
            AiJob::Macro(q) => {
                let names = self.sdt_names();
                if names.is_empty() {
                    ui::tf!("{}\n\n(この文書に名前つきの記入欄はありません)", q)
                } else {
                    ui::tf!("{}\n\n【この文書の記入欄の名前】{}", q, names.join("、"))
                }
            }
            _ => format!("{ask}\n\n---\n{body}"),
        };
        let (sys, job2) = (sys.to_string(), job.clone());
        self.ai_busy = true;
        self.status = ui::tf!("AI({})に{}を頼んでいます…", back.label(), job.label())
        .into();
        let task = cx
            .background_executor()
            .spawn(async move { ui::ai::ask(back, &sys, &user) });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |this, cx| {
                this.ai_busy = false;
                match r {
                    Ok(out) => this.ai_apply(job2, sel, out, cx),
                    Err(e) => this.status = format!("AI: {e}").into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 文書の名前つき記入欄の名前(重複なし・出現順)。
    /// マクロ台本を書く AI に「的」として渡す
    fn sdt_names(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut push = |sd: &kumihan::Sdt| {
            if !sd.tag.is_empty() && sd.tag != sd.kind.as_tag() && !out.contains(&sd.tag)
            {
                out.push(sd.tag.clone());
            }
        };
        for p in self.doc.paragraphs() {
            for r in &p.runs {
                if let Some(sd) = r.fmt.sdt.as_deref() {
                    push(sd);
                }
            }
        }
        for t in self.doc.tables() {
            for row in &t.rows {
                for c in row {
                    for p in &c.paragraphs {
                        for r in &p.runs {
                            if let Some(sd) = r.fmt.sdt.as_deref() {
                                push(sd);
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// 返事を文書へ入れる。**1手で戻せる**(doc_undo に控える)
    fn ai_apply(
        &mut self,
        job: AiJob,
        sel: std::ops::Range<usize>,
        out: String,
        _cx: &mut Context<Self>,
    ) {
        let out = out.trim().to_string();
        if out.is_empty() {
            self.status = ui::t!("AI: 答えが空でした(何もしていません)").into();
            return;
        }
        // マクロ台本は文書に入れない — プラグイン置き場に .py で置き、
        // 人が読んで確かめてから一覧から実行する(開く=実行なしのまま)
        if matches!(job, AiJob::Macro(_)) {
            let code = strip_code_fence(&out);
            if code.trim().is_empty() {
                self.status = ui::t!("AI: 台本が空でした(何もしていません)").into();
                return;
            }
            let dir = plugins_dir();
            let _ = std::fs::create_dir_all(&dir);
            let mut i = 1;
            let mut path = dir.join("ai台本1.py");
            while path.exists() {
                i += 1;
                path = dir.join(ui::tf!("ai台本{}.py", i));
            }
            match std::fs::write(&path, &code) {
                Ok(()) => {
                    self.plug_open = true; // 置いた台本がすぐ見えるように
                    self.status = ui::tf!("台本を {} に置きました — 読んで確かめてから、\
                         プラグインの一覧で実行してください(自動では走らせません)", path.display())
                    .into();
                }
                Err(e) => self.status = ui::tf!("台本を置けません: {}", e).into(),
            }
            return;
        }
        self.doc_undo = Some(self.doc.clone());
        let label = job.label();
        match job {
            // 要約は文書の頭に、印つきの段落として置く
            AiJob::Summary => {
                let text = self.ed.text().to_string();
                let joined = ui::tf!("【要約】{}\n\n{}", out, text);
                self.ed = Editor::new(&joined);
                self.doc.set_body_text(self.ed.text(), SIZE_PT);
            }
            // 置き換え(選択が無ければ全文)
            AiJob::Rewrite(_, _) | AiJob::Translate | AiJob::Table => {
                let out = if matches!(job, AiJob::Table) {
                    // | 区切りの行を、読みやすい字の表に直す(表の挿入は次の課題)
                    out.lines()
                        .map(|l| {
                            l.trim().trim_matches('|').replace('|', "　")
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    out
                };
                let r = if sel.is_empty() { 0..self.ed.text().len() } else { sel };
                self.ed.move_to(r.start, false);
                self.ed.move_to(r.end, true);
                self.ed.insert(&out);
                self.doc.set_body_text(self.ed.text(), SIZE_PT);
            }
            // 続き・自由な頼みは、カーソル(選択の終わり)の後ろへ
            // Macro は上で受けて return 済み
            AiJob::Macro(_) => unreachable!(),
            AiJob::Continue | AiJob::Ask(_) => {
                let at = sel.end.min(self.ed.text().len());
                self.ed.move_to(at, false);
                self.ed.insert(&format!("\n{out}"));
                self.doc.set_body_text(self.ed.text(), SIZE_PT);
            }
            // ふりがなは |語《よみ》 を**うちのルビ**に直して振る
            AiJob::Furigana => {
                let base = if sel.is_empty() { 0 } else { sel.start };
                let (plain, rubies) = strip_ruby_marks(&out, base);
                let r = if sel.is_empty() { 0..self.ed.text().len() } else { sel };
                self.ed.move_to(r.start, false);
                self.ed.move_to(r.end, true);
                self.ed.insert(&plain);
                self.doc.set_body_text(self.ed.text(), SIZE_PT);
                let n = rubies.len();
                for (range, yomi) in rubies {
                    self.doc.apply_char_format(range, move |f| {
                        f.ruby = Some(yomi.clone())
                    });
                }
                self.dirty = true;
                self.relayout_keep();
                self.status =
                    ui::tf!("ふりがなを {} 箇所に振りました(Ctrl+Z で1手で戻せます)", n)
                        .into();
                return;
            }
        }
        self.dirty = true;
        self.relayout();
        self.status =
            ui::tf!("AI の{}を入れました(Ctrl+Z で1手で戻せます)", label).into();
    }

    /// 記入欄(コンテンツコントロール)を挿す。選択があればそれを欄にし、
    /// 無ければ空欄の字を置いて欄にする。**中は普通に打てる**(欄は保たれる)
    fn insert_sdt(&mut self, kind: kumihan::SdtKind, items: Vec<String>) {
        use kumihan::SdtKind as K;
        self.switch_target(Target::Body);
        let sel = self.ed.selection();
        // 欄の初期の中身(選択があればその字)
        let range = if sel.is_empty() {
            let init = match kind {
                K::Checkbox => "☐".to_string(),
                K::Dropdown | K::Combo => {
                    items.first().cloned().unwrap_or_else(|| ui::t!("　　　　").into())
                }
                K::Date => std::process::Command::new("date")
                    .arg("+%Y年%-m月%-d日")
                    .output()
                    .ok()
                    .filter(|o| o.status.success())
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_else(|| ui::t!("　　　　").into()),
                K::Picture => ui::t!("[画像]").to_string(),
                _ => ui::t!("　　　　").to_string(),
            };
            let at = self.ed.cursor();
            self.ed.insert(&init);
            self.on_edited();
            at..at + init.len()
        } else {
            sel
        };
        let alias = kind.label().to_string();
        let tag = kind.as_tag().to_string();
        self.doc.set_body_text(self.ed.text(), SIZE_PT);
        self.doc.apply_char_format(range.clone(), move |f| {
            f.sdt = Some(Box::new(kumihan::Sdt {
                kind,
                alias: alias.clone(),
                tag: tag.clone(),
                items: items.clone(),
            }))
        });
        self.dirty = true;
        self.relayout_keep();
        self.ed.move_to(range.end, false);
        self.status = ui::tf!("{}の記入欄を入れました(中は普通に打てます。保存で docx の\
             コンテンツコントロールに)", kind.label())
        .into();
    }

    /// いる場所の記入欄(あれば)
    fn sdt_at(&self) -> Option<kumihan::Sdt> {
        self.doc
            .char_format_at(self.ed.selection())
            .sdt
            .as_deref()
            .cloned()
    }

    /// 選択肢のパネルの Enter(コンボ・ドロップダウンを挿す。
    /// 名前を聞いていたときは付け替えへ)
    fn sd_commit(&mut self) {
        self.sd_open = false;
        if self.sd_naming {
            self.sd_naming = false;
            self.sd_name_commit();
            return;
        }
        let items: Vec<String> = self
            .sd_ed
            .text()
            .split(&[',', '、', '/'][..])
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if items.is_empty() {
            self.status = ui::t!("選択肢がありません(カンマ区切りで打ってください)").into();
            return;
        }
        self.insert_sdt(self.sd_kind, items);
    }

    /// 名前のパネルの Enter。カーソルの記入欄の alias / tag をまるごと打ち替える
    /// (run が割れていても sdt_range_at が一つに繋げる)
    fn sd_name_commit(&mut self) {
        let name = self.sd_ed.text().trim().to_string();
        if name.is_empty() {
            self.status = ui::t!("名前がありません(記入欄はそのまま)").into();
            return;
        }
        let Some(range) = self.doc.sdt_range_at(self.ed.cursor()) else {
            self.status =
                ui::t!("記入欄が見つかりません(欄の中にカーソルを置いてください)").into();
            return;
        };
        let name2 = name.clone();
        self.doc.apply_char_format(range, move |f| {
            if let Some(sd) = f.sdt.as_deref_mut() {
                sd.alias = name2.clone();
                sd.tag = name2.clone();
            }
        });
        self.dirty = true;
        self.relayout_keep();
        self.status = ui::tf!("記入欄に名前「{}」を付けました(docx の w:tag。\
             マクロは fill(\"{}\", 値) で記入できます)", name, name)
        .into();
    }

    /// チェックの欄を切り替える(☐ ⇄ ☑)。カーソルがその欄にあるとき
    fn toggle_checkbox(&mut self) -> bool {
        let Some(sd) = self.sdt_at() else { return false };
        if sd.kind != kumihan::SdtKind::Checkbox {
            return false;
        }
        // カーソルの前後の1字を見て入れ替える
        let text = self.ed.text().to_string();
        let cur = self.ed.cursor();
        let (s0, e0) = match text[..cur].char_indices().next_back() {
            Some((i, c)) if c == '☐' || c == '☑' => (i, cur),
            _ => match text[cur..].chars().next() {
                Some(c) if c == '☐' || c == '☑' => (cur, cur + c.len_utf8()),
                _ => return false,
            },
        };
        let now = &text[s0..e0];
        let next = if now == "☑" { "☐" } else { "☑" };
        self.ed.move_to(s0, false);
        self.ed.move_to(e0, true);
        self.ed.insert(next);
        self.on_edited();
        self.status = ui::tf!("チェックを {} にしました", next).into();
        true
    }

    /// 入切のボタンが「いま入っているか」。押した結果が画面に残るものは、
    /// ボタンの側にも出す(押したのに何も変わらないように見えるのを防ぐ)
    fn toggled(&self, id: &str) -> bool {
        match id {
            "nav" | "show-left" => self.nav_open,
            "show-toolbar" => self.show_toolbar,
            "show-statusbar" => self.show_statusbar,
            "multipage" => self.multipage,
            "show-right" => self.rp_open,
            "ruler" => self.ruler,
            "darkmode" => self.dark,
            "hidenchars" => self.show_marks,
            "line-numbers" => self.line_numbers,
            "direction" => self.doc.vertical,
            "track-changes" => self.track,
            "co-showcomment" => self.show_comments,
            "prot-doc" => self.doc.protection.is_some(),
            "prot-encrypt" => self.encrypt_pw.is_some(),
            _ => false,
        }
    }

    /// ページ幅・ページ全体に合わせる(見えている大きさから倍率を出す)。
    /// width=true なら幅だけ、false なら高さも見て小さい方に合わせる
    fn fit_zoom(&mut self, width: bool) {
        // 紙は左 28px に置き、右にも同じだけ余白を見る
        let zw = (self.view_w_px - 56.0) / (self.pg.w_mm * PX_PER_MM);
        let z = if width {
            zw
        } else {
            zw.min((self.view_h_px - 28.0) / (self.pg.h_mm * PX_PER_MM))
        };
        self.zoom = z.clamp(0.2, 5.0);
        self.status = ui::tf!("{}に合わせました(ズーム {}%)", if width { "幅" } else { "ページ" }, (self.zoom * 100.0).round() as i32)
        .into();
    }

    /// 見出しの一覧(ナビゲーション用)。(深さ, 字, 本文のバイト位置)
    fn headings(&self) -> Vec<(u8, String, usize)> {
        let mut out = Vec::new();
        let mut at = 0usize;
        for p in self.doc.paragraphs() {
            let text: String = p.runs.iter().map(|r| r.text.as_str()).collect();
            if let kumihan::ParaStyle::Heading(n) = p.style {
                out.push((n, text.clone(), at));
            }
            at += text.len() + 1;
        }
        out
    }

    /// 取ってきた HTML を開き、起点と土台を控える(リンクと送信の解決に使う)
    fn adopt_fetched(&mut self, url: &str, bytes: &[u8]) {
        let scheme_end = url.find("://").map(|i| i + 3).unwrap_or(0);
        let host = url[scheme_end..].split('/').next().unwrap_or("");
        self.html_origin = Some(format!("{}{host}", &url[..scheme_end]));
        self.html_base = Some(url.to_string());
        self.open_html(std::path::Path::new(url), bytes);
    }

    /// リンクを辿る(GET して同じ道で開く)
    fn follow_link(&mut self, href: String, cx: &mut Context<Self>) {
        let base = self.html_base.clone().unwrap_or_default();
        let url = resolve_url(&base, &href);
        self.status = ui::tf!("取りに行っています… {}", url).into();
        let task = cx.background_executor().spawn(async move { http_fetch(&url, None) });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok((bytes, final_url)) => this.adopt_fetched(&final_url, &bytes),
                    Err(e) => this.status = ui::tf!("開けません: {}", e).into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 記入の欄の Enter。パネルの欄へ書き戻す
    fn fm_commit(&mut self) {
        let Some(i) = self.fm_field.take() else { return };
        let text = self.fm_ed.text().to_string();
        if let Some(fm) = self.html_forms.first_mut() {
            if let Some(f) = fm.fields.get_mut(i) {
                f.value = text;
            }
        }
        self.status = ui::t!("記入しました(送信のボタンで送る)").into();
    }

    /// フォームを送る。POST は urlencoded、GET は ?query。
    /// 網の線引き: いま開いている起点(html_origin)へだけ
    fn fm_submit(&mut self, cx: &mut Context<Self>) {
        let Some(fm) = self.html_forms.first().cloned() else { return };
        let Some(origin) = self.html_origin.clone() else {
            self.status =
                ui::t!("ローカルの HTML からは送れません(URL で開いてください)").into();
            return;
        };
        let url = if fm.action.starts_with("http://") {
            if !fm.action.starts_with(&origin) {
                self.status = ui::t!("送り先が開いた相手と違います(送りません)").into();
                return;
            }
            fm.action.clone()
        } else if fm.action.starts_with('/') {
            format!("{origin}{}", fm.action)
        } else {
            format!("{origin}/{}", fm.action)
        };
        let q: String = fm
            .fields
            .iter()
            .map(|f| format!("{}={}", urlenc(&f.name), urlenc(&f.value)))
            .collect::<Vec<_>>()
            .join("&");
        let post = fm.method == "post";
        self.status = ui::t!("送っています…").into();
        let task = cx.background_executor().spawn(async move {
            if post {
                http_fetch(&url, Some(&q))
            } else {
                http_fetch(&format!("{url}?{q}"), None)
            }
        });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok((bytes, final_url)) => this.adopt_fetched(&final_url, &bytes),
                    Err(e) => this.status = ui::tf!("送れません: {}", e).into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 平文(zip)の docx を読み込む。open と pw_commit の共通の続き
    fn open_plain(&mut self, p: PathBuf, bytes: Vec<u8>) {
        self.target = Target::Body;
        // 前の文書のパネルが残っていると、打鍵が新しい文書のヘッダーを潰す
        self.hf_edit = None;
        self.track = false;
        self.track_base = None;
        // 前の文書のパスワードを引きずらない(暗号化して開いた時だけ
        // pw_commit が後から入れ直す)
        self.encrypt_pw = None;
        match ooxml::read(std::io::Cursor::new(bytes)) {
            Ok((doc, rep)) => {
                self.notes = rep
                    .unsupported
                    .iter()
                    .map(|(n, c)| SharedString::from(format!("{n} × {c}")))
                    .collect();
                self.status = ui::tf!("{} 段落 / 表 {} — {}", rep.paragraphs, doc.tables().count(), p.file_name().unwrap_or_default().to_string_lossy())
                .into();
                self.pg = doc.page.unwrap_or_default();
                self.set_doc(doc);
                self.adopt_font();
                self.relayout_keep();
                // 排他(共有フォルダの「後勝ちで潰す」を防ぐ。calc と同じ)
                self.acquire_lock(&p);
                if let Some(who) = self.locked_by.clone() {
                    self.status = ui::tf!("{} — **{} が開いています**。上書き保存はできません(別の名前で保存へ)", self.status, who)
                    .into();
                }
                if self.doc.protection.is_some() {
                    self.status = ui::tf!("{} — 読み取り専用で保護されています(保護タブで解除できます)", self.status)
                    .into();
                }
                Self::note_recent(&p);
                self.path = Some(p);
                self.dirty = false;
            }
            Err(e) => self.status = ui::tf!("開けません: {}", e).into(),
        }
    }

    /// 保存。名前が無ければ選ばせる(**ダイアログは別のスレッド** — rfd は同期で、
    /// メインスレッドで開くと画面ごと固まる。calc と同じ作法)。
    /// `then_quit` なら保存が済んだときだけ終了する — 書きかけを黙って捨てない。
    fn save(&mut self, then_quit: bool, cx: &mut Context<Self>) {
        if let Some(p) = self.path.clone() {
            if self.locked_by.is_none() {
                self.save_to(p);
                if then_quit && !self.dirty {
                    self.release_lock();
                    cx.quit();
                }
                return;
            }
            // 先客の作業を後勝ちで潰さない。別の名前でなら保存できる
            self.status = ui::tf!("{} が開いているため上書きしません。別の名前で保存します", self.locked_by.as_deref().unwrap_or("誰か"))
            .into();
        }
        let ask = cx.background_executor().spawn(async {
            rfd::FileDialog::new().add_filter("Word文書", &["docx"]).save_file()
        });
        cx.spawn(async move |this, cx| {
            let r = ask.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Some(p) => {
                        this.save_to(p);
                        if then_quit && !this.dirty {
                            this.release_lock();
                            cx.quit();
                        }
                    }
                    None => this.status = ui::t!("保存をやめました(名前が決まっていません)").into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_to(&mut self, p: PathBuf) {
        self.flush_target();
        // 元のファイルの部品(画像・スタイル・ヘッダー等)を持ち越す。
        // 上書き保存では読み終えてから書く(同じファイルを同時に開かない)
        let original: Option<std::io::Cursor<Vec<u8>>> =
            self.original_plain().map(std::io::Cursor::new);
        let doc_out = self.doc_for_save();
        // バージョン履歴: 上書きの前に、いままでの中身を控えとして残す
        if p.exists() {
            self.keep_version(&p);
        }
        let saved = if let Some(pw) = self.encrypt_pw.clone() {
            // 暗号化は zip 丸ごとが単位 — 一度メモリへ書いてから包む
            let mut plain = Vec::new();
            ooxml::write_with(&doc_out, original, std::io::Cursor::new(&mut plain))
                .and_then(|_| ooxml::crypt::encrypt(&plain, &pw))
                .and_then(|enc| {
                    kumihan::atomic::save(&p, |mut f| {
                        use std::io::Write as _;
                        f.write_all(&enc).map_err(|e| e.to_string())
                    })
                })
        } else {
            kumihan::atomic::save(&p, |f| {
                ooxml::write_with(&doc_out, original, std::io::BufWriter::new(f))
            })
        };
        match saved {
            Ok(_) => {
                let caveat = if self.notes.is_empty() {
                    ""
                } else {
                    // 読めなかった要素は本文から消えている。黙って保存しない
                    "(読めなかった要素は本文に戻りません)"
                };
                let enc_note =
                    if self.encrypt_pw.is_some() { "(暗号化)" } else { "" };
                self.status = ui::tf!("保存しました — {}{}{}", p.file_name().unwrap_or_default().to_string_lossy(), enc_note, caveat)
                .into();
                // 保存先のロックを取り直す(別の名前で保存したときは
                // 新しいファイルの側を守る。同じ名前なら実質そのまま)
                self.acquire_lock(&p);
                Self::note_recent(&p);
                self.path = Some(p);
                self.dirty = false;
            }
            Err(e) => self.status = ui::tf!("保存できません: {}", e).into(),
        }
    }

    /// 文字位置 → 紙の上の座標(キャレットを出すため)
    /// 語の単位でカーソルを動かす(Ctrl+←→)。
    fn word_move(&mut self, forward: bool, extend: bool) {
        let t = self.ed.text().to_string();
        let np = word_boundary(&t, self.ed.cursor(), forward);
        self.ed.move_to(np, extend);
        self.follow_caret();
    }

    /// カーソルの下の語を選ぶ(二度クリック)。
    fn select_word(&mut self) {
        let t = self.ed.text().to_string();
        if t.is_empty() {
            return;
        }
        let pos = self.ed.cursor().min(t.len());
        let chars: Vec<(usize, char)> = t.char_indices().collect();
        // カーソルの字(末尾なら手前の字)から、同じ種類の連なりを広げる
        let ci = chars.iter().position(|(i, _)| *i >= pos).unwrap_or(chars.len());
        let k = ci.min(chars.len() - 1);
        let cl = char_class(chars[k].1);
        let mut s = k;
        while s > 0 && char_class(chars[s - 1].1) == cl {
            s -= 1;
        }
        let mut e = k + 1;
        while e < chars.len() && char_class(chars[e].1) == cl {
            e += 1;
        }
        let sb = chars[s].0;
        let eb = chars.get(e).map(|(i, _)| *i).unwrap_or(t.len());
        self.ed.move_to(sb, false);
        self.ed.move_to(eb, true);
    }

    /// いまの(見た目の)行を選ぶ(三度クリック)。
    fn select_line(&mut self) {
        let pos = self.ed.cursor();
        let want = match self.target {
            Target::Body => None,
            Target::Cell { table, row, col } => Some((table, row, col)),
        };
        let mut hit: Option<(usize, usize)> = None;
        for l in self.page.lines.iter().filter(|l| match want {
            None => l.from_body,
            Some(id) => l.cell == Some(id),
        }) {
            if l.byte0 <= pos {
                hit = Some((l.byte0, l.byte_end()));
            }
        }
        if let Some((s, e)) = hit {
            self.ed.move_to(s, false);
            self.ed.move_to(e, true);
        }
    }

    /// 1画面ぶん(PageUp/PageDown)。見た目の行を数えて動く。
    fn page_move(&mut self, down: bool) {
        let pxmm = PX_PER_MM * self.zoom;
        let step = ((self.view_h_px / (LINE_MM * pxmm)) as i32 - 2).max(1);
        for _ in 0..step {
            self.move_line(down, false);
        }
    }

    /// カーソルを1行、上(または下)へ。**見た目の行**単位 — 折り返した長い
    /// 段落の中でも1段ずつ動く。横の位置(x)はなるべく保つ。
    /// 一番上で↑なら文頭、一番下で↓なら文末へ(行の端で止まって動かないより良い)。
    fn move_line(&mut self, down: bool, extend: bool) {
        let pos = self.ed.cursor();
        let want = match self.target {
            Target::Body => None,
            Target::Cell { table, row, col } => Some((table, row, col)),
        };
        let lines: Vec<&kumihan::Line> = self
            .page
            .lines
            .iter()
            .filter(|l| match want {
                None => l.from_body,
                Some(id) => l.cell == Some(id),
            })
            .collect();
        if lines.is_empty() {
            return;
        }
        // いまの行 = 頭がカーソル以前にある最後の行
        let cur = lines.iter().rposition(|l| l.byte0 <= pos).unwrap_or(0);
        let target = if down {
            if cur + 1 >= lines.len() {
                let end = self.ed.text().len();
                self.ed.move_to(end, extend);
                self.follow_caret();
                return;
            }
            cur + 1
        } else {
            if cur == 0 {
                self.ed.move_to(0, extend);
                self.follow_caret();
                return;
            }
            cur - 1
        };
        // いまの x(紙の座標)を保ったまま、隣の行で一番近い字の境へ
        let (x_now, _, _) = self.caret_xy();
        let ln = lines[target];
        let base = ln.cells.iter().map(|c| c.off).min().unwrap_or(0);
        let mut byte = ln.byte_end();
        for c in &ln.cells {
            let cx = self.pg.left_mm + c.x_mm;
            if x_now < cx + c.w_mm / 2.0 {
                byte = ln.byte0 + (c.off - base);
                break;
            }
        }
        self.ed.move_to(byte.min(self.ed.text().len()), extend);
        self.follow_caret();
    }

    /// カーソルの紙面上の位置と、そこの文字の大きさ(pt)。
    /// キャレットは**その場の文字の大きさで**描く — 見出しの中で
    /// 小さいままだと、どこに立っているのか分からない。
    fn caret_xy(&self) -> (f32, f32, f32) {
        let cur = self.ed.cursor();
        // 行の頭のバイト位置(byte0)は組版が持っている。
        // 行の文字数で数え直すと、折り返しで落ちた空白や空行でずれる。
        // 折り返し・段落の境目では**後ろの行**に立てる(Enter の直後は次の行)
        let want = match self.target {
            Target::Body => None,
            Target::Cell { table, row, col } => Some((table, row, col)),
        };
        let mut hit: Option<(f32, f32, f32)> = None;
        for (li, line) in self.page.lines.iter().enumerate().filter(|(_, l)| match want {
            None => l.from_body,
            Some(id) => l.cell == Some(id),
        }) {
            if cur < line.byte0 {
                continue;
            }
            if cur > line.byte_end() + 1 {
                continue;
            }
            let within = cur.saturating_sub(line.byte0);
            let base = line.cells.iter().map(|c| c.off).min().unwrap_or(0);
            let at = line.cells.iter().find(|c| c.off - base >= within);
            let x = at
                .map(|c| c.x_mm)
                .or_else(|| line.cells.last().map(|c| c.x_mm + c.w_mm))
                .unwrap_or(0.0);
            let pt = at
                .or_else(|| line.cells.last())
                .map(|c| c.size_pt)
                .unwrap_or(SIZE_PT);
            hit = if self.page.vertical {
                // 縦書き: x は列、y は上からの距離(スクロール追従がこれを見る)
                let col = self.page.vert_x.get(li).copied().unwrap_or(0.0);
                Some((col, line.y_mm + x, pt))
            } else {
                Some((self.pg.left_mm + x, line.y_mm, pt))
            };
        }
        hit.unwrap_or((
            self.pg.left_mm,
            self.page.lines.last().map(|l| l.y_mm).unwrap_or(self.pg.top_mm),
            SIZE_PT,
        ))
    }

    /// レビュー > 校正。**英語は辞書、日本語はモデル。**
    ///
    /// 英語の綴り誤りは辞書に無い語になるので辞書で捕まる(GPU も要らない)。
    /// 日本語の誤変換は辞書に有る語になるので、辞書では原理的に捕まらない。
    ///
    /// 検査できなかった部分があれば必ずそう出す — **黙って「指摘なし」にしない**
    /// (利用者は「誤りが無い」と受け取ってしまう)。
    fn run_proof(&mut self) {
        let r = self.checker.check(self.ed.text());
        self.proof_msg = r.summary().into();
        self.proof = r.findings;
    }

    /// 編集中のセルの段落へ書式を掛ける(セルは短いので丸ごと掛ける)。
    fn each_cell_para(&mut self, f: impl Fn(&mut kumihan::Paragraph)) {
        let Target::Cell { table, row, col } = self.target else { return };
        self.flush_target();
        if let Some(kumihan::Block::Table(tb)) = self
            .doc
            .blocks
            .iter_mut()
            .filter(|b| matches!(b, kumihan::Block::Table(_)))
            .nth(table)
        {
            if let Some(cell) = tb.rows.get_mut(row).and_then(|r| r.get_mut(col)) {
                for p in &mut cell.paragraphs {
                    f(p);
                }
            }
        }
    }

    /// 選択している段落の文字書式を入切する。
    ///
    /// **編集先が本文かセルかで掛け先が違う。** セル編集中に本文へ掛けると、
    /// set_body_text がセルの文章で本文を上書きしてしまう。
    fn toggle(&mut self, f: impl Fn(&mut kumihan::CharFormat)) {
        match self.target {
            Target::Body => {
                let sel = self.ed.selection();
                self.doc.set_body_text(self.ed.text(), SIZE_PT);
                self.doc.apply_char_format(sel, f);
            }
            Target::Cell { .. } => self.each_cell_para(|p| {
                for r in &mut p.runs {
                    f(&mut r.fmt);
                }
            }),
        }
        self.dirty = true;
        self.relayout_keep();
    }

    /// 選んでいる段落の性質を変える。
    fn para(&mut self, f: impl Fn(&mut kumihan::Paragraph) + Copy) {
        if self.protected() {
            self.status =
                ui::t!("読み取り専用で保護されています(保護タブの「保護」で解除できます)").into();
            return;
        }
        match self.target {
            Target::Body => {
                let sel = self.ed.selection();
                self.doc.set_body_text(self.ed.text(), SIZE_PT);
                self.doc.apply_para(sel, f);
            }
            Target::Cell { .. } => self.each_cell_para(f),
        }
        self.dirty = true;
        self.relayout_keep();
    }

    fn size(&mut self, f: impl Fn(f32) -> f32 + Copy) {
        match self.target {
            Target::Body => {
                let sel = self.ed.selection();
                self.doc.set_body_text(self.ed.text(), SIZE_PT);
                self.doc.apply_size(sel, f);
            }
            Target::Cell { .. } => self.each_cell_para(|p| {
                for r in &mut p.runs {
                    r.size_pt = f(r.size_pt).clamp(4.0, 400.0);
                }
            }),
        }
        self.dirty = true;
        self.relayout_keep();
    }

    /// PDF として保存。保存先の選択は**別のスレッド**(rfd は同期)。
    fn save_pdf(&mut self, cx: &mut Context<Self>) {
        // 見開きは画面だけの見え方。紙は1ページずつなので組み直してから写す
        if self.multipage {
            let keep = self.multipage;
            self.multipage = false;
            self.relayout_keep();
            self.multipage = keep;
        }
        let ask = cx.background_executor().spawn(async {
            rfd::FileDialog::new()
                .add_filter("PDF", &["pdf"])
                .set_file_name("文書.pdf")
                .save_file()
        });
        cx.spawn(async move |this, cx| {
            let r = ask.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(p) = r {
                    this.write_pdf(&p);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// **画面に出しているのと同じ紙面を写す**ので、画面と紙が食い違わない。
    fn write_pdf(&mut self, p: &std::path::Path) {
        let m = Metrics::new(&self.font_bytes).expect("フォント");
        let (hdr, ftr, pg) = (self.doc.header.clone(), self.doc.footer.clone(), self.pg);
        let total = self.total_pages();
        // ページの色と透かしは紙にも(画面と紙の一致)
        let dress = paper::PageDress {
            bg: self.doc.page_color.as_deref().map(|c| (hex(c, 0), hex(c, 1), hex(c, 2))),
            watermark: self.doc.watermark.clone(),
            ink: self.doc.ink.clone(),
        };
        let r = kumihan::atomic::save(p, |f| {
            paper::to_pdf_with(
                &self.page,
                &self.font_bytes,
                paper::Paper {
                    width_mm: pg.w_mm,
                    height_mm: pg.h_mm,
                    margin_mm: pg.left_mm,
                },
                &dress,
                // ヘッダー・フッター。ページ番号はここで各頁の数字になる
                |k| {
                    let mut v = kumihan::layout_hf(&hdr, &m, &pg, LINE_MM, k, total, false);
                    v.extend(kumihan::layout_hf(&ftr, &m, &pg, LINE_MM, k, total, true));
                    v
                },
                std::io::BufWriter::new(f),
            )
        });
        self.status = match r {
            Ok(_) => ui::tf!("PDF にしました — {}", p.file_name().unwrap_or_default().to_string_lossy()).into(),
            Err(e) => ui::tf!("PDF にできません: {}", e).into(),
        };
    }

    /// 用紙の設定を変える。**文書に書き戻す**(sect_raw を作り替える)ので
    /// 保存で残る。画面と紙は同じ寸法で追随する。
    fn set_page(&mut self, f: impl Fn(&mut kumihan::PageSetup)) {
        f(&mut self.pg);
        self.doc.page = Some(self.pg);
        let tw = |mm: f32| -> i64 { (mm * 20.0 * 72.0 / 25.4).round() as i64 };
        let landscape = self.pg.w_mm > self.pg.h_mm;
        // 原文があっても、寸法だけはこちらが決めた値で作り替える。
        // ヘッダーの参照などは残したいので、pgSz/pgMar 以外は原文から引き継ぐ
        let rest = self
            .doc
            .sect_raw
            .as_deref()
            .map(|s| {
                let mut out = String::new();
                let mut skip = false;
                for part in s.split_inclusive('>') {
                    let t = part.trim_start();
                    if t.starts_with("<w:sectPr") || t.starts_with("</w:sectPr") {
                        continue;
                    }
                    if t.starts_with("<w:pgSz") || t.starts_with("<w:pgMar")
                        || t.starts_with("<w:cols")
                    {
                        skip = !part.trim_end().ends_with("/>");
                        continue;
                    }
                    if skip {
                        if t.starts_with("</w:pgSz") || t.starts_with("</w:pgMar")
                            || t.starts_with("</w:cols")
                        {
                            skip = false;
                        }
                        continue;
                    }
                    out.push_str(part);
                }
                out
            })
            .unwrap_or_default();
        // 段組みは Word の既定の間(425twip)で書く
        let cols = if self.pg.cols() > 1 {
            format!("<w:cols w:num=\"{}\" w:space=\"425\"/>", self.pg.cols())
        } else {
            String::new()
        };
        self.doc.sect_raw = Some(format!(
            "<w:sectPr><w:pgSz w:w=\"{}\" w:h=\"{}\"{}/>\
             <w:pgMar w:top=\"{}\" w:right=\"{}\" w:bottom=\"{}\" w:left=\"{}\"/>{cols}{rest}</w:sectPr>",
            tw(self.pg.w_mm),
            tw(self.pg.h_mm),
            if landscape { " w:orient=\"landscape\"" } else { "" },
            tw(self.pg.top_mm),
            tw(self.pg.right_mm),
            tw(self.pg.bottom_mm),
            tw(self.pg.left_mm),
        ));
        self.dirty = true;
        self.relayout_keep();
        self.status = ui::tf!("用紙 {:.0}×{:.0}mm / 余白 {:.0}mm{}", self.pg.w_mm, self.pg.h_mm, self.pg.left_mm, if self.pg.cols() > 1 { format!(" / {}段組み", self.pg.cols()) } else { String::new() })
        .into();
    }

    fn set_align(&mut self, a: Align) {
        match self.target {
            Target::Body => {
                let sel = self.ed.selection();
                self.doc.set_body_text(self.ed.text(), SIZE_PT);
                self.doc.apply_align(sel, a);
            }
            Target::Cell { .. } => self.each_cell_para(|p| p.align = a),
        }
        self.dirty = true;
        self.relayout_keep();
    }

    /// カーソルの段落(通し番号)と、その頭のバイト位置。
    fn cursor_para(&self) -> (usize, usize) {
        let cur = self.ed.cursor();
        let (mut pi, mut b0) = (0usize, 0usize);
        let mut at = 0usize;
        for (i, p) in self.doc.paragraphs().enumerate() {
            let len: usize = p.runs.iter().map(|r| r.text.len()).sum();
            if at <= cur {
                pi = i;
                b0 = at;
            }
            at += len + 1;
        }
        (pi, b0)
    }

    /// 相互参照の値。文字ならしおりの段落の本文、ページなら紙と同じ折り方の番号。
    fn ref_value(&self, name: &str, page: bool) -> Option<String> {
        let mut at = 0usize;
        for p in self.doc.paragraphs() {
            let t: String = p.runs.iter().map(|r| r.text.as_str()).collect();
            if p.bookmarks.iter().any(|b| b == name) {
                if !page {
                    return Some(t.trim().to_string());
                }
                let (pages, _) = paper::paginate(&self.page, paper::Paper {
                    width_mm: self.pg.w_mm,
                    height_mm: self.pg.h_mm,
                    margin_mm: self.pg.left_mm,
                });
                let mut hit = 1usize;
                for (l, pg2) in self.page.lines.iter().zip(&pages) {
                    if l.from_body && l.byte0 <= at {
                        hit = *pg2;
                    }
                }
                return Some(hit.to_string());
            }
            at += t.len() + 1;
        }
        None
    }

    /// 相互参照を挿す。値を普通の字として打ってから、その範囲を参照にする。
    fn insert_ref(&mut self, name: &str, page: bool) {
        self.switch_target(Target::Body);
        let Some(val) = self.ref_value(name, page) else {
            self.status = ui::tf!("しおり「{}」が見つかりません", name).into();
            return;
        };
        let start = self.ed.selection().start;
        handler::replace(self, None, &val);
        self.doc.apply_field(
            start..start + val.len(),
            Some(kumihan::RefField { name: name.to_string(), page }),
        );
        self.relayout_keep();
        self.status = ui::tf!("「{}」への参照を挿しました({}。参照は編集で中を触ると普通の字に戻ります)", name, if page { "ページ番号" } else { "しおりの文字" })
        .into();
    }

    /// 参照を計算し直す。run の text を直に書き換えるので、編集の平文も作り直す
    /// (**undo の控えはここで失われる** — そう言う)。
    fn refresh_refs(&mut self) {
        self.switch_target(Target::Body);
        self.flush_target();
        let vals: std::collections::BTreeMap<(String, bool), String> = self
            .doc
            .paragraphs()
            .flat_map(|p| p.runs.iter())
            .filter_map(|r| r.fmt.field.clone())
            .map(|f| {
                let v = self.ref_value(&f.name, f.page).unwrap_or_else(|| "?".into());
                ((f.name, f.page), v)
            })
            .collect();
        let n = self
            .doc
            .refresh_fields(|name, page| vals.get(&(name.to_string(), page)).cloned());
        if n > 0 {
            let cur = self.ed.cursor();
            self.ed = Editor::new(&self.doc.body_text());
            let len = self.ed.text().len();
            self.ed.move_to(cur.min(len), false);
            self.dirty = true;
            self.relayout_keep();
            self.status =
                ui::tf!("参照を {} 箇所更新しました(この操作は戻せません)", n).into();
        } else {
            self.status = ui::t!("参照は最新です").into();
        }
    }

    /// しおりを追加する(カーソルの段落へ)。
    fn bm_add(&mut self) {
        let name = self.bm_ed.text().trim().to_string();
        if name.is_empty() {
            self.status = ui::t!("しおりの名前を打ってから追加してください").into();
            return;
        }
        if self.doc.paragraphs().any(|p| p.bookmarks.iter().any(|b| *b == name)) {
            self.status = ui::tf!("しおり「{}」は既にあります", name).into();
            return;
        }
        self.switch_target(Target::Body);
        let (pi, _) = self.cursor_para();
        let mut i = 0usize;
        for b in &mut self.doc.blocks {
            if let kumihan::Block::Para(p) = b {
                if i == pi {
                    p.bookmarks.push(name.clone());
                    break;
                }
                i += 1;
            }
        }
        self.bm_ed = Editor::new("");
        self.dirty = true;
        self.status = ui::tf!("しおり「{}」を付けました(保存で docx に入ります)", name).into();
    }

    /// 段落のスタイル。0 = 標準、1〜3 = 見出し。
    /// スタイル定義(styles.xml)を持たないので、見た目は直接書式で付ける。
    fn set_para_style(&mut self, n: u8) {
        let (pt, bold) = match n {
            1 => (16.0, true),
            2 => (13.0, true),
            3 => (11.5, true),
            _ => (SIZE_PT, false),
        };
        self.para(move |p| {
            p.style = if n == 0 {
                kumihan::ParaStyle::Body
            } else {
                kumihan::ParaStyle::Heading(n)
            };
        });
        self.size(move |_| pt);
        self.toggle(move |f| f.bold = bold);
        self.status = match n {
            0 => ui::t!("標準の段落にしました").into(),
            n => ui::tf!("見出し{} にしました(参考資料 > 目次 の材料になります)", n).into(),
        };
    }

    /// 目次を作る・挿し直す。見出し(ホーム > 段落のスタイル)が材料。
    /// ページ番号は紙(PDF)と同じ折り方(paper::paginate)から出すので、
    /// 印刷した紙とずれない。目次の行は ParaStyle::Toc の印を持ち、
    /// 「目次の更新」はその連続を丸ごと置き換える。
    fn make_toc(&mut self) {
        self.switch_target(Target::Body);
        self.flush_target();
        // 見出しを集める(本文のバイト位置つき)
        let mut heads: Vec<(u8, String, usize)> = Vec::new();
        let mut at = 0usize;
        for p in self.doc.paragraphs() {
            let text: String = p.runs.iter().map(|r| r.text.as_str()).collect();
            if let kumihan::ParaStyle::Heading(n) = p.style {
                heads.push((n, text.clone(), at));
            }
            at += text.len() + 1;
        }
        if heads.is_empty() {
            self.status =
                ui::t!("見出しがありません(ホーム > 段落のスタイルで見出しを付けてください)").into();
            return;
        }
        // 行 → ページ番号(紙と同じ折り方)
        let (pages, _) = paper::paginate(&self.page, paper::Paper {
            width_mm: self.pg.w_mm,
            height_mm: self.pg.h_mm,
            margin_mm: self.pg.left_mm,
        });
        let page_of = |byte: usize| -> usize {
            let mut hit = 1usize;
            for (l, pg) in self.page.lines.iter().zip(&pages) {
                if l.from_body && l.byte0 <= byte {
                    hit = *pg;
                }
            }
            hit
        };
        // 目次の行。レベルぶん字下げし、点線(…)を実フォントの字幅で詰めて
        // 番号を右端に着地させる(揃えの機構は使わず、文字で作る —
        // 静的な本文なので、開いた Word でもそのままの見た目になる)
        let m = Metrics::new(&self.font_bytes).expect("フォント");
        let measure = self.pg.measure_mm();
        let w_of = |s: &str| -> f32 { s.chars().map(|c| m.advance_mm(c, SIZE_PT)).sum() };
        let (dot_w, sp_w) = (m.advance_mm('…', SIZE_PT), m.advance_mm('　', SIZE_PT));
        let lines: Vec<(u8, String)> = heads
            .iter()
            .map(|(n, t, b)| {
                let head = format!("{}{}", "　".repeat((*n - 1) as usize), t);
                let num = page_of(*b).to_string();
                // 前後に全角1つずつの空きを置き、残りを … で埋める。
                // 1mm の安全代 — 端数で行長を超えると折り返して目次が崩れる
                let avail = measure - w_of(&head) - w_of(&num) - 2.0 * sp_w - 1.0;
                let dots = (avail / dot_w).floor().max(0.0) as usize;
                (*n, ui::tf!("{}　{}　{}", head, "…".repeat(dots), num))
            })
            .collect();

        let toc_paras: Vec<kumihan::Paragraph> = lines
            .iter()
            .map(|(n, t)| kumihan::Paragraph {
                style: kumihan::ParaStyle::Toc(*n),
                line_spacing: 1.0,
                runs: vec![kumihan::Run {
                    text: t.clone(),
                    size_pt: SIZE_PT,
                    font: None,
                    fmt: Default::default(),
                }],
                ..Default::default()
            })
            .collect();
        let replaced =
            self.splice_marked(|st| matches!(st, kumihan::ParaStyle::Toc(_)), toc_paras);
        self.status = if replaced {
            ui::tf!("目次を更新しました({} 項目)", lines.len()).into()
        } else {
            ui::tf!("目次を入れました({} 項目。見出しを変えたら「目次の更新」)", lines.len())
                .into()
        };
    }

    /// 印の付いた段落の連続を、新しい段落の列で置き換える(無ければ
    /// カーソルの段落の前に挿す)。**編集(undo の1手)と blocks を
    /// 同じ形に揃える** — 揃えないと set_body_text の性質の持ち越し
    /// (段落番号ベース)がずれる。返り値: 置き換えたか。
    fn splice_marked(
        &mut self,
        is_mark: impl Fn(kumihan::ParaStyle) -> bool,
        paras: Vec<kumihan::Paragraph>,
    ) -> bool {
        let text: String = paras
            .iter()
            .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        let blocks: Vec<kumihan::Block> =
            paras.into_iter().map(kumihan::Block::Para).collect();
        let mut para_meta: Vec<(usize, usize, bool)> = Vec::new();
        let mut at = 0usize;
        for p in self.doc.paragraphs() {
            let len: usize = p.runs.iter().map(|r| r.text.len()).sum();
            para_meta.push((at, len, is_mark(p.style)));
            at += len + 1;
        }
        let para_block_idx: Vec<usize> = self
            .doc
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| matches!(b, kumihan::Block::Para(_)))
            .map(|(i, _)| i)
            .collect();
        let old = para_meta.iter().position(|(_, _, t)| *t).map(|st| {
            let mut e = st;
            while e + 1 < para_meta.len() && para_meta[e + 1].2 {
                e += 1;
            }
            (st, e)
        });
        let replaced = match old {
            Some((st, e)) => {
                let (b0, _, _) = para_meta[st];
                let (b1, l1, _) = para_meta[e];
                self.ed.move_to(b0, false);
                self.ed.move_to(b1 + l1, true);
                self.ed.insert(&text);
                self.doc.blocks.splice(para_block_idx[st]..=para_block_idx[e], blocks);
                true
            }
            None => {
                let cur = self.ed.cursor();
                let pi = para_meta.iter().rposition(|(b0, _, _)| *b0 <= cur).unwrap_or(0);
                let (b0, _, _) = para_meta[pi];
                self.ed.move_to(b0, false);
                self.ed.move_to(b0, true);
                self.ed.insert(&format!("{text}\n"));
                let bi = para_block_idx[pi];
                self.doc.blocks.splice(bi..bi, blocks);
                false
            }
        };
        self.dirty = true;
        self.relayout();
        self.follow_caret();
        replaced
    }

    /// 図表目次。図表番号(「図 n」で始まる段落)を集めて一覧にする。
    /// 行は ParaStyle::Tof の印を持ち、「図表目次の更新」で丸ごと作り直す。
    fn make_tof(&mut self) {
        self.switch_target(Target::Body);
        self.flush_target();
        let mut items: Vec<(String, usize)> = Vec::new();
        let mut at = 0usize;
        for p in self.doc.paragraphs() {
            let t: String = p.runs.iter().map(|r| r.text.as_str()).collect();
            let tt = t.trim();
            if p.style != kumihan::ParaStyle::Tof {
                if let Some(rest) = tt.strip_prefix("図 ") {
                    if rest.split_whitespace().next().is_some_and(|w| w.parse::<usize>().is_ok()) {
                        items.push((tt.to_string(), at));
                    }
                }
            }
            at += t.len() + 1;
        }
        if items.is_empty() {
            self.status =
                ui::t!("図表番号がありません(参考資料 > 図表番号で付けてください)").into();
            return;
        }
        let (pages, _) = paper::paginate(&self.page, paper::Paper {
            width_mm: self.pg.w_mm,
            height_mm: self.pg.h_mm,
            margin_mm: self.pg.left_mm,
        });
        let page_of = |byte: usize| -> usize {
            let mut hit = 1usize;
            for (l, pg) in self.page.lines.iter().zip(&pages) {
                if l.from_body && l.byte0 <= byte {
                    hit = *pg;
                }
            }
            hit
        };
        let m = Metrics::new(&self.font_bytes).expect("フォント");
        let measure = self.pg.measure_mm();
        let w_of = |s: &str| -> f32 { s.chars().map(|c| m.advance_mm(c, SIZE_PT)).sum() };
        let (dot_w, sp_w) = (m.advance_mm('…', SIZE_PT), m.advance_mm('　', SIZE_PT));
        let paras: Vec<kumihan::Paragraph> = items
            .iter()
            .map(|(t, b)| {
                let num = page_of(*b).to_string();
                let avail = measure - w_of(t) - w_of(&num) - 2.0 * sp_w - 1.0;
                let dots = (avail / dot_w).floor().max(0.0) as usize;
                kumihan::Paragraph {
                    style: kumihan::ParaStyle::Tof,
                    line_spacing: 1.0,
                    runs: vec![kumihan::Run {
                        text: ui::tf!("{}　{}　{}", t, "…".repeat(dots), num),
                        size_pt: SIZE_PT,
                        font: None,
                        fmt: Default::default(),
                    }],
                    ..Default::default()
                }
            })
            .collect();
        let n = paras.len();
        let replaced = self.splice_marked(|st| st == kumihan::ParaStyle::Tof, paras);
        self.status = if replaced {
            ui::tf!("図表目次を更新しました({} 項目)", n).into()
        } else {
            ui::tf!("図表目次を入れました({} 項目)", n).into()
        };
    }

    /// 書式を触ったあとの組み直し。**本文を戻さない**
    /// (戻すと今つけた書式が消える)。
    fn relayout_keep(&mut self) {
        let m = Metrics::new(&self.font_bytes).expect("フォント");
        let y0 = self.pg.top_mm + 4.0;
        if self.doc.vertical {
            // 縦書き: 行長 = 紙の縦の使い幅で組み、右からの列へ写す(K4)
            let measure =
                (self.pg.h_mm - self.pg.top_mm - self.pg.bottom_mm - 8.0).max(20.0);
            self.page = layout(
                &self.doc,
                &m,
                &Frame { measure_mm: measure, line_height_mm: LINE_MM, y0_mm: y0 },
            );
            kumihan::fold_vertical(&mut self.page, &self.pg, y0, LINE_MM);
        } else {
            self.page = layout(
                &self.doc,
                &m,
                &Frame { measure_mm: self.pg.column_measure_mm(), line_height_mm: LINE_MM, y0_mm: y0 },
            );
            kumihan::fold_columns(&mut self.page, &self.pg, y0);
        }
        self.refresh_hf();
    }

    /// クリックした画素位置(編集領域からの相対)にカーソルを置く。
    /// 文書の下端(紙の座標 mm)。1ページに満たなくても紙1枚ぶんは白い
    /// 紙の見た目の幅(mm。見開きなら2枚ぶん)
    fn paper_w_mm(&self) -> f32 {
        if self.multipage && !self.page.vertical {
            self.pg.w_mm * 2.0 + PAGE_GAP_MM
        } else {
            self.pg.w_mm
        }
    }

    fn content_mm(&self) -> f32 {
        if self.multipage && !self.page.vertical {
            // 2枚ずつの段。ページ数の半分(切り上げ)ぶんの高さ
            let pages = self.page_offsets.len().max(1);
            return (pages.div_ceil(2)) as f32 * self.pg.h_mm;
        }
        if self.page.vertical {
            // 縦書きは物理ページに畳んであるので、ページ数で決まる
            let pages = self.page.lines.iter()
                .map(|l| (l.y_mm / self.pg.h_mm) as usize)
                .max()
                .unwrap_or(0);
            return ((pages + 1) as f32) * self.pg.h_mm;
        }
        self.page.lines.last().map(|l| l.y_mm + 30.0).unwrap_or(0.0).max(self.pg.h_mm)
    }

    /// 縦にスクロールする(画素)。紙の頭より上・末尾より下へは行かない。
    fn scroll_px(&mut self, dy_px: f32) {
        let pxmm = PX_PER_MM * self.zoom;
        let view_mm = (self.view_h_px / pxmm).max(20.0);
        let max = (self.content_mm() + 20.0 - view_mm).max(0.0);
        self.scroll_mm = (self.scroll_mm + dy_px / pxmm).clamp(0.0, max);
    }

    /// キャレットが窓から出ていたら、見える所まで紙を送る。
    fn follow_caret(&mut self) {
        let pxmm = PX_PER_MM * self.zoom;
        let (_, cy, _) = self.caret_xy();
        let view_mm = (self.view_h_px / pxmm).max(20.0);
        if cy > self.scroll_mm + view_mm - 15.0 {
            self.scroll_mm = cy - (view_mm - 15.0);
        }
        if cy < self.scroll_mm + 5.0 {
            self.scroll_mm = (cy - 5.0).max(0.0);
        }
    }

    fn click_at(&mut self, rel_x: f32, rel_y: f32, extend: bool) {
        let pxmm = PX_PER_MM * self.zoom;
        // 紙は編集領域の (28,14)px に置いてあり、スクロールで上へずれている
        let x_mm = (rel_x - 28.0) / pxmm - self.pg.left_mm;
        let y_mm = (rel_y - 14.0) / pxmm + self.scroll_mm;

        // 表のセルの中なら、そのセルの編集に切り替える
        let hit_box = self.page.cell_boxes.iter().find(|b| {
            x_mm >= b.x_mm && x_mm <= b.x_mm + b.w_mm
                && y_mm >= b.top_mm && y_mm <= b.top_mm + b.h_mm
        }).copied();
        if let Some(b) = hit_box {
            let id = Target::Cell { table: b.table, row: b.row, col: b.col };
            self.switch_target(id);
            // セルの中の行で位置を決める
            let mut hit = 0usize;
            for line in &self.page.lines {
                if line.cell != Some((b.table, b.row, b.col)) {
                    continue;
                }
                if line.y_mm - LINE_MM * 0.8 > y_mm {
                    continue;
                }
                hit = line.byte0;
                let base = line.cells.iter().map(|c| c.off).min().unwrap_or(0);
                let mut x = line.cells.first().map(|c| c.x_mm - self.pg.left_mm).unwrap_or(0.0);
                for c in &line.cells {
                    if x_mm < x + c.w_mm / 2.0 {
                        break;
                    }
                    x += c.w_mm;
                    hit = line.byte0 + (c.off + c.ch.len_utf8()) - base;
                }
            }
            let hit = hit.min(self.ed.text().len());
            self.ed.move_to(hit, extend);
            return;
        }
        // 本文をクリックした。セルを編集していたら本文へ戻る
        self.switch_target(Target::Body);

        if self.page.vertical {
            // 縦書き: 列(x)で行を選び、字は y で選ぶ
            let vx = (rel_x - 28.0) / pxmm; // 紙の絶対 x(mm)
            let mut best: Option<(f32, usize)> = None;
            for (i, line) in self.page.lines.iter().enumerate() {
                if !line.from_body || line.cells.is_empty() {
                    continue;
                }
                let cx = self.page.vert_x.get(i).copied().unwrap_or(0.0)
                    + LINE_MM / 2.0;
                let top = line.y_mm;
                let bot = line.y_mm
                    + line.cells.last().map(|c| c.x_mm + c.w_mm).unwrap_or(0.0);
                let dx = (vx - cx).abs();
                let dy = if y_mm < top {
                    top - y_mm
                } else if y_mm > bot {
                    y_mm - bot
                } else {
                    0.0
                };
                let d = dx + dy * 0.5;
                if best.map_or(true, |(bd, _)| d < bd) {
                    best = Some((d, i));
                }
            }
            let Some((_, i)) = best else { return };
            let line = &self.page.lines[i];
            let mut byte = line.byte0;
            let base = line.cells.iter().map(|c| c.off).min().unwrap_or(0);
            for c in &line.cells {
                if y_mm < line.y_mm + c.x_mm + c.w_mm / 2.0 {
                    break;
                }
                byte = line.byte0 + (c.off + c.ch.len_utf8()) - base;
            }
            self.ed.move_to(byte.min(self.ed.text().len()), extend);
            return;
        }

        // 一番近いベースラインの本文行を選ぶ(クリックは字の少し上に落ちる)
        let target = y_mm + LINE_MM * 0.3;
        let mut best: Option<(f32, usize)> = None; // (距離, 本文行の通し番号)
        let mut nth = 0usize;
        for line in &self.page.lines {
            if !line.from_body {
                continue;
            }
            let d = (line.y_mm - target).abs();
            if best.map_or(true, |(bd, _)| d < bd) {
                best = Some((d, nth));
            }
            nth += 1;
        }
        let Some((_, want)) = best else { return };

        // 行が持つバイト位置から出す(文字数で数え直さない)
        let mut byte = 0usize;
        let mut nth = 0usize;
        for line in &self.page.lines {
            if !line.from_body {
                continue;
            }
            if nth == want {
                byte = line.byte0;
                let base = line.cells.iter().map(|c| c.off).min().unwrap_or(0);
                let mut x = line.cells.first().map(|c| c.x_mm).unwrap_or(0.0);
                for c in &line.cells {
                    if x_mm < x + c.w_mm / 2.0 {
                        break;
                    }
                    x += c.w_mm;
                    byte = line.byte0 + (c.off + c.ch.len_utf8()) - base;
                }
                break;
            }
            nth += 1;
        }
        let byte = byte.min(self.ed.text().len());
        self.ed.move_to(byte, extend);
    }

    /// 次の一致を選ぶ(カーソルの後ろから。末尾まで無ければ頭から一周)。
    fn find_next(&mut self) {
        let term = self.find_ed.text().to_string();
        if term.is_empty() {
            self.status = ui::t!("検索語が空です").into();
            return;
        }
        let text = self.ed.text().to_string();
        let from = self.ed.selection().end;
        let hit = text[from..]
            .find(&term)
            .map(|i| from + i)
            .or_else(|| text.find(&term));
        match hit {
            Some(i) => {
                self.ed.move_to(i, false);
                self.ed.move_to(i + term.len(), true);
                self.status = "".into();
            }
            None => self.status = ui::tf!("「{}」は見つかりません", term).into(),
        }
    }

    /// いま選ばれている一致を置き換えて、次へ。
    fn replace_current(&mut self) {
        if self.protected() {
            self.status =
                ui::t!("読み取り専用で保護されています(保護タブの「保護」で解除できます)").into();
            return;
        }
        let term = self.find_ed.text().to_string();
        let repl = self.repl_ed.text().to_string();
        if term.is_empty() {
            return;
        }
        let sel = self.ed.selection();
        let selected: String = self.ed.text()[sel.clone()].to_string();
        if selected == term {
            self.ed.insert(&repl);
            self.dirty = true;
            self.relayout();
        }
        self.find_next();
    }

    /// 全部置き換える。**何件変えたかを言う**(黙って書き換えない)。
    fn replace_all(&mut self) {
        if self.protected() {
            self.status =
                ui::t!("読み取り専用で保護されています(保護タブの「保護」で解除できます)").into();
            return;
        }
        let term = self.find_ed.text().to_string();
        let repl = self.repl_ed.text().to_string();
        if term.is_empty() {
            return;
        }
        let mut n = 0usize;
        loop {
            let text = self.ed.text().to_string();
            let Some(i) = text.find(&term) else { break };
            self.ed.move_to(i, false);
            self.ed.move_to(i + term.len(), true);
            self.ed.insert(&repl);
            // **1置換ごとに本文へ写す。** まとめて写すと「1回の編集 = 1箇所」の
            // 前提から外れ、最初と最後の一致の間の書式が均されてしまう
            // (SEKKEI「writer の編集モデル」の注意をここで解いた)
            self.doc.set_body_text(self.ed.text(), SIZE_PT);
            n += 1;
            if n > 100_000 {
                break; // 置換後が検索語を含むと止まらなくなるのを防ぐ
            }
        }
        if n > 0 {
            self.dirty = true;
            self.relayout();
        }
        self.status = ui::tf!("{} 件を置き換えました", n).into();
    }

    /// run_cmd が処理できる id。**リボンの ready はこの表の中に限る**
    const HANDLED: &'static [&'static str] = &[
        "open", "save", "undo", "redo", "selectall", "pdf",
        "bold", "italic", "underline", "strikeout", "fontcolor",
        "superscript", "subscript", "highlight", "clearstyle",
        "align-left", "align-center", "align-right", "align-just", "align-dist",
        "ruby", "direction",
        "controls", "form-text", "form-combo", "form-dropdown", "form-checkbox",
        "form-radio", "form-image", "form-email", "form-phone", "form-complex",
        "form-signature", "form-name",
        "colorschemas",
        "ai-where", "ai-summary", "ai-rewrite", "ai-polite", "ai-plain",
        "ai-translate", "ai-furigana", "ai-continue", "ai-table", "ai-ask",
        "ai-macro",
        "nav", "fit-page", "fit-width", "zoom100", "multipage",
        "show-toolbar", "show-statusbar", "show-left", "show-right",
        "incfont", "decfont", "markers", "numbering",
        "incoffset", "decoffset", "linespace", "pagebreak",
        "instable", "inssymbol", "replace", "changecase", "blankpage",
        "paracolor", "borders", "insimage",
        "spell", "wordcount", "zoom-in", "zoom-out", "hidenchars", "ruler",
        "fontname", "fontsize",
        "pageorient", "pagesize", "pagemargins",
        "edit-header", "edit-footer", "pagenum",
        "parastyle", "toc", "toc-update", "numpages", "datetime",
        "multilevels", "darkmode", "text-from-file", "add-text", "line-numbers",
        "insshape", "inssmartart", "inschart", "smartpicker", "instextart",
        "insequation", "instext", "pagecolor", "comment", "watermark", "bookmarks",
        "caption", "tof", "tof-update", "columns",
        "pen", "highlighter", "eraser", "track-changes", "dropcap", "hyphenation",
        "crossref", "co-addcomment", "co-delcomment", "co-showcomment",
        "prot-doc", "coauth-mode", "co-history", "co-chat",
        "plug-macros", "plug-manage", "prot-encrypt", "prot-sign",
        "copy", "cut", "paste",
    ];

    /// 画像を読んで、カーソルの段落の下に挿す。
    /// SVG(matplotlib の savefig("図.svg") など)は高精細の PNG に直して貼る。
    fn insert_image(&mut self, path: &std::path::Path) {
        match std::fs::read(path) {
            Ok(bytes) => {
                let is_svg = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
                    || bytes.starts_with(b"<svg")
                    || bytes.starts_with(b"<?xml");
                let (bytes, pw, ph) = if is_svg {
                    match ui::svg_to_png(&bytes, 3.0) {
                        Ok((png, w, h)) => (png, w, h),
                        Err(e) => {
                            self.status = e.into();
                            return;
                        }
                    }
                } else {
                    let Some((pw, ph)) = image_px(&bytes) else {
                        self.status = ui::t!("PNG・JPEG・SVG だけ挿せます").into();
                        return;
                    };
                    (bytes, pw, ph)
                };
                // 96dpi 相当で置き、行長に収まらなければ比例で縮める
                let mut w_mm = pw as f32 * 25.4 / 96.0;
                let mut h_mm = ph as f32 * 25.4 / 96.0;
                let measure = self.pg.measure_mm();
                if w_mm > measure {
                    let k = measure / w_mm;
                    w_mm *= k;
                    h_mm *= k;
                }
                let im = kumihan::InlineImage {
                    bytes: std::sync::Arc::new(bytes),
                    w_mm,
                    h_mm,
                };
                // 選択があっても、挿すのはカーソルの段落だけ
                let cur = self.ed.cursor();
                self.ed.move_to(cur, false);
                self.para(|p| {
                    p.images.push(im.clone()); // 表示
                    p.images_new.push(im.clone()); // 保存
                });
                self.status = if is_svg {
                    ui::t!("SVG を高精細の画像にして挿しました(保存で docx に入ります)").into()
                } else {
                    ui::t!("画像を挿しました(段落の下に付き、保存で docx に入ります)").into()
                };
            }
            Err(e) => self.status = ui::tf!("読めません: {}", e).into(),
        }
    }

    /// テキスト(または docx の本文)をカーソルの位置に差し込む。
    fn insert_text_from(&mut self, path: &std::path::Path) {
        let is_docx = path.extension().and_then(|e| e.to_str()) == Some("docx");
        let text = if is_docx {
            match std::fs::File::open(path).map_err(|e| e.to_string()).and_then(ooxml::read) {
                Ok((d, rep)) => {
                    if !rep.is_lossless() {
                        // 本文だけを差し込む。落ちたもの(画像・表の外の要素)は言う
                        self.notes = rep
                            .unsupported
                            .iter()
                            .map(|(n, c)| SharedString::from(format!("{n} × {c}")))
                            .collect();
                    }
                    d.body_text()
                }
                Err(e) => {
                    self.status = ui::tf!("読めません: {}", e).into();
                    return;
                }
            }
        } else {
            match std::fs::read(path) {
                Ok(b) => match String::from_utf8(b) {
                    Ok(t) => t,
                    Err(_) => {
                        // 文字コードの推測はしない(化けた本文を黙って挿すより断る)
                        self.status = ui::t!("UTF-8 のテキストだけ読めます").into();
                        return;
                    }
                },
                Err(e) => {
                    self.status = ui::tf!("読めません: {}", e).into();
                    return;
                }
            }
        };
        if text.is_empty() {
            self.status = ui::t!("空のファイルです").into();
            return;
        }
        self.switch_target(Target::Body);
        handler::replace(self, None, &text);
        self.status = ui::tf!("{} を差し込みました({} 文字)", path.file_name().unwrap_or_default().to_string_lossy(), text.chars().count())
        .into();
    }

    /// 開くファイルを選ぶ(**ダイアログは別のスレッド**)。
    fn open_dialog(&mut self, cx: &mut Context<Self>) {
        let ask = cx.background_executor().spawn(async {
            rfd::FileDialog::new()
                .add_filter("Word文書とHTML", &["docx", "html", "htm"])
                .pick_file()
        });
        cx.spawn(async move |this, cx| {
            let r = ask.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(p) = r {
                    this.open(p);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn backspace(&mut self, _: &ui::Backspace, _: &mut Window, cx: &mut Context<Self>) {
        self.editor().backspace();
        self.on_edited();
        cx.notify();
    }
    fn delete(&mut self, _: &ui::Delete, _: &mut Window, cx: &mut Context<Self>) {
        self.editor().delete();
        self.on_edited();
        cx.notify();
    }
    fn left(&mut self, _: &ui::Left, _: &mut Window, cx: &mut Context<Self>) {
        self.editor().move_char(false, false);
        cx.notify();
    }
    fn right(&mut self, _: &ui::Right, _: &mut Window, cx: &mut Context<Self>) {
        self.editor().move_char(true, false);
        cx.notify();
    }
    fn select_left(&mut self, _: &ui::SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.editor().move_char(false, true);
        cx.notify();
    }
    fn select_right(&mut self, _: &ui::SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.editor().move_char(true, true);
        cx.notify();
    }
    fn select_all(&mut self, _: &ui::SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.editor().select_all();
        cx.notify();
    }
    fn word_left(&mut self, _: &ui::WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.word_move(false, false);
        cx.notify();
    }
    fn word_right(&mut self, _: &ui::WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.word_move(true, false);
        cx.notify();
    }
    fn select_word_left(&mut self, _: &ui::SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.word_move(false, true);
        cx.notify();
    }
    fn select_word_right(&mut self, _: &ui::SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.word_move(true, true);
        cx.notify();
    }
    fn a_context_menu(&mut self, _: &ui::ContextMenu, _: &mut Window, cx: &mut Context<Self>) {
        // キーボードから: キャレットのそばに出す
        let pxmm = PX_PER_MM * self.zoom;
        let (x, y, _) = self.caret_xy();
        self.menu_at = Some((
            28.0 + x * pxmm + 8.0,
            14.0 + (y - self.scroll_mm) * pxmm + 8.0,
        ));
        cx.notify();
    }

    fn a_cancel(&mut self, _: &ui::Cancel, _: &mut Window, cx: &mut Context<Self>) {
        // 道具 → メニュー → 検索のパネル → ヘッダーのパネル → 一覧のパネル、の順で戻す
        if self.tool.take().is_some() {
            self.ink_cur = None;
            self.status = ui::t!("文字の編集に戻りました").into();
            cx.notify();
            return;
        }
        if self.menu_at.take().is_some() {
            cx.notify();
            return;
        }
        if self.pw_open {
            self.pw_open = false;
            self.pw_pending = None;
            self.status = "".into();
            cx.notify();
            return;
        }
        if self.tab == 0 {
            // ファイルのページ。欄 → ページの順で閉じる
            if self.file_field.take().is_some() {
                cx.notify();
                return;
            }
            self.tab = self.prev_tab;
            cx.notify();
            return;
        }
        if self.find_open {
            self.find_open = false;
            cx.notify();
            return;
        }
        if self.hf_edit.take().is_some() {
            self.status = "".into();
            cx.notify();
            return;
        }
        if self.cmt_edit {
            self.cmt_edit = false;
            self.status = "".into();
            cx.notify();
            return;
        }
        if self.wm_edit {
            self.wm_edit = false;
            self.status = "".into();
            cx.notify();
            return;
        }
        if self.bm_open {
            self.bm_open = false;
            self.status = "".into();
            cx.notify();
            return;
        }
        if self.xr_open {
            self.xr_open = false;
            self.status = "".into();
            cx.notify();
            return;
        }
        if self.url_open || self.fm_field.is_some() || self.fm_open || self.lk_open {
            if self.fm_field.take().is_none() {
                self.url_open = false;
                self.fm_open = false;
                self.lk_open = false;
            }
            self.status = "".into();
            cx.notify();
            return;
        }
        if self.quit_ask {
            self.quit_ask = false;
            self.status = ui::t!("終了をやめました").into();
            cx.notify();
            return;
        }
        if self.rb_open || self.sd_open || self.ai_open {
            self.rb_open = false;
            self.sd_open = false;
            self.sd_naming = false;
            self.ai_open = false;
            self.ai_macro = false;
            self.status = "".into();
            cx.notify();
            return;
        }
        if self.hist_open || self.chat_open || self.plug_open {
            self.hist_open = false;
            self.chat_open = false;
            self.plug_open = false;
            self.status = "".into();
            cx.notify();
            return;
        }
        if self.font_list || self.size_list || self.symbols || self.style_list {
            self.font_list = false;
            self.size_list = false;
            self.symbols = false;
            self.style_list = false;
            cx.notify();
        }
    }

    fn do_find(&mut self, _: &ui::Find, _: &mut Window, cx: &mut Context<Self>) {
        if !self.find_open {
            self.run_cmd("replace", cx); // 検索と置換のパネルを開く
        }
        cx.notify();
    }
    fn doc_home(&mut self, _: &ui::DocHome, _: &mut Window, cx: &mut Context<Self>) {
        self.ed.move_to(0, false);
        self.follow_caret();
        cx.notify();
    }
    fn doc_end(&mut self, _: &ui::DocEnd, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.ed.text().len();
        self.ed.move_to(n, false);
        self.follow_caret();
        cx.notify();
    }
    /// Tab で段落を1段深く、Shift+Tab で1段浅く。
    /// リストではレベル(印も変わる)、普通の段落ではインデントとして効く。
    fn a_tab(&mut self, _: &ui::Tab, _: &mut Window, cx: &mut Context<Self>) {
        if self.find_open || self.hf_edit.is_some() {
            return; // パネルの中では使わない
        }
        self.para(|p| p.indent = (p.indent + 1).min(8));
        cx.notify();
    }
    fn a_shift_tab(&mut self, _: &ui::ShiftTab, _: &mut Window, cx: &mut Context<Self>) {
        if self.find_open || self.hf_edit.is_some() {
            return;
        }
        self.para(|p| p.indent = p.indent.saturating_sub(1));
        cx.notify();
    }

    fn page_up(&mut self, _: &ui::PageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.page_move(false);
        cx.notify();
    }
    fn page_down(&mut self, _: &ui::PageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.page_move(true);
        cx.notify();
    }
    fn up(&mut self, _: &ui::Up, _: &mut Window, cx: &mut Context<Self>) {
        self.move_line(false, false);
        cx.notify();
    }
    fn down(&mut self, _: &ui::Down, _: &mut Window, cx: &mut Context<Self>) {
        self.move_line(true, false);
        cx.notify();
    }
    fn select_up(&mut self, _: &ui::SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_line(false, true);
        cx.notify();
    }
    fn select_down(&mut self, _: &ui::SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_line(true, true);
        cx.notify();
    }
    fn home(&mut self, _: &ui::Home, _: &mut Window, cx: &mut Context<Self>) {
        self.editor().move_to(0, false);
        cx.notify();
    }
    fn end(&mut self, _: &ui::End, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.editor_ref().text().len();
        self.editor().move_to(n, false);
        cx.notify();
    }
    fn enter(&mut self, _: &ui::Enter, _: &mut Window, cx: &mut Context<Self>) {
        if self.pw_open {
            self.pw_commit();
            cx.notify();
            return;
        }
        if self.file_field.is_some() {
            self.commit_prop();
            cx.notify();
            return;
        }
        if self.find_open {
            self.find_next();
        } else if self.bm_open {
            self.bm_add();
        } else if self.quit_ask {
            // Enter = 保存して終了(いちばん安全な既定)
            self.quit_ask = false;
            self.save(true, cx);
        } else if self.chat_open {
            self.chat_send();
        } else if self.url_open {
            self.url_commit(cx);
        } else if self.fm_field.is_some() {
            self.fm_commit();
        } else if self.rb_open {
            self.rb_commit();
        } else if self.sd_open {
            self.sd_commit();
        } else if self.ai_open {
            let q = self.ai_ed.text().to_string();
            self.ai_open = false;
            let macro_mode = self.ai_macro;
            self.ai_macro = false;
            if !q.trim().is_empty() {
                let job = if macro_mode { AiJob::Macro(q) } else { AiJob::Ask(q) };
                self.ai_go(job, cx);
            }
        } else {
            self.editor().insert("\n");
            self.on_edited();
        }
        cx.notify();
    }
    fn copy(&mut self, _: &ui::Copy, _: &mut Window, cx: &mut Context<Self>) {
        // パネル(ヘッダー等)を編集中なら、そのパネルの選択が対象
        let e = self.editor_ref();
        let sel = e.selection();
        if sel.is_empty() {
            self.status = ui::t!("コピーする選択がありません").into();
        } else if let Some(s) = e.text().get(sel) {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(s.to_string()));
            self.status = ui::t!("コピーしました").into();
        }
        cx.notify();
    }
    fn cut(&mut self, _: &ui::Cut, _: &mut Window, cx: &mut Context<Self>) {
        let sel = self.editor_ref().selection();
        if sel.is_empty() {
            self.status = ui::t!("切り取る選択がありません").into();
        } else if let Some(s) = self.editor_ref().text().get(sel).map(str::to_string) {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(s));
            // 選択を空文字で置き換える = undo の1手で戻る
            self.editor().insert("");
            self.on_edited();
            self.status = ui::t!("切り取りました").into();
        }
        cx.notify();
    }
    fn paste(&mut self, _: &ui::Paste, _: &mut Window, cx: &mut Context<Self>) {
        match cx.read_from_clipboard().and_then(|i| i.text()) {
            Some(text) if !text.is_empty() => {
                // 通常の入力と同じ道(IME の未確定があれば確定してから)
                handler::replace(self, None, &text);
            }
            _ => self.status = ui::t!("貼り付けるものがありません").into(),
        }
        cx.notify();
    }
    fn undo(&mut self, _: &ui::Undo, _: &mut Window, cx: &mut Context<Self>) {
        // 道具(ペン)の間は筆の一手を戻す
        if self.tool.is_some() {
            if let Some(prev) = self.ink_undo.pop() {
                self.doc.ink = prev;
                self.dirty = true;
            }
            cx.notify();
            return;
        }
        // パネル(ヘッダー等)を編集中なら、そのパネルの一手を戻す
        if self.editor().undo() {
            self.on_edited();
        } else if let Some(prev) = self.doc_undo.take() {
            // マクロで置き換えた文書を、1手で元へ戻す
            self.target = Target::Body;
            self.pg = prev.page.clone().unwrap_or_default();
            self.set_doc(prev);
            self.relayout_keep();
            self.dirty = true;
            self.status = ui::t!("マクロの前に戻しました").into();
        }
        cx.notify();
    }
    fn redo(&mut self, _: &ui::Redo, _: &mut Window, cx: &mut Context<Self>) {
        if self.editor().redo() {
            self.on_edited();
        }
        cx.notify();
    }
    fn do_save(&mut self, _: &ui::Save, _: &mut Window, cx: &mut Context<Self>) {
        self.save(false, cx);
        cx.notify();
    }
    /// 終了の要求。書きかけが無ければ即終了、あれば確認を**別のスレッド**で出す。
    /// 確認のダイアログでメインスレッドを塞がない — 塞ぐと画面ごと固まり、
    /// GNOME に「応答なし」と判定される(calc で踏んで直したのと同じ)。
    fn request_quit(&mut self, cx: &mut Context<Self>) {
        // 確認を出すのは**未保存の変更があるとき**。名前の無い新規でも、
        // 何か書いてあれば出す — 書いた物を黙って捨てない(発注者 2026-08-06。
        // calc と同じ改訂)。本当に空のままなら従来どおり黙って閉じる
        let empty_new = self.path.is_none() && self.ed.text().trim().is_empty();
        if !self.dirty || empty_new {
            self.release_lock();
            cx.quit();
            return;
        }
        // 確認は**窓の中のパネル**で出す。rfd の OS ダイアログは親窓を持てず
        // **スクリーンの中央**に出て、窓から離れすぎる(発注者 2026-08-06)
        self.quit_ask = true;
        cx.notify();
    }

    fn do_quit(&mut self, _: &ui::Quit, _: &mut Window, cx: &mut Context<Self>) {
        self.request_quit(cx);
    }

    fn do_open(&mut self, _: &ui::Open, _: &mut Window, cx: &mut Context<Self>) {
        self.open_dialog(cx);
        cx.notify();
    }
}

impl Focusable for Writer {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EntityInputHandler for Writer {
    fn text_for_range(
        &mut self,
        r: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _w: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        handler::text_for_range(self, r, actual)
    }
    fn selected_text_range(
        &mut self,
        _ignore: bool,
        _w: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: handler::selected_range_utf16(self), reversed: false })
    }
    fn marked_text_range(&self, _w: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
        handler::marked_range_utf16(self)
    }
    fn unmark_text(&mut self, _w: &mut Window, _cx: &mut Context<Self>) {
        handler::unmark(self);
    }
    fn replace_text_in_range(
        &mut self,
        r: Option<Range<usize>>,
        text: &str,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        handler::replace(self, r, text);
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        r: Option<Range<usize>>,
        text: &str,
        sel: Option<Range<usize>>,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        handler::replace_and_mark(self, r, text, sel);
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        _r: Range<usize>,
        bounds: Bounds<gpui::Pixels>,
        _w: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<gpui::Pixels>> {
        // IME の候補窓をキャレットの下に出す(スクロールと倍率を織り込む)
        let pxmm = PX_PER_MM * self.zoom;
        let (x, y, pt) = self.caret_xy();
        Some(Bounds::new(
            gpui::point(
                bounds.origin.x + px(28.0 + x * pxmm),
                bounds.origin.y + px(14.0 + (y - self.scroll_mm) * pxmm),
            ),
            size(px(2.0), px(pt * 96.0 / 72.0 * self.zoom)),
        ))
    }
    fn character_index_for_point(
        &mut self,
        _p: gpui::Point<gpui::Pixels>,
        _w: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
    fn text_length_utf16(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> Option<usize> {
        Some(handler::text_len_utf16(self))
    }
}

mod cmds;
mod io;
pub(crate) use io::*;
mod py;
pub(crate) use py::*;
mod util;
pub(crate) use util::*;
mod view;

#[cfg(test)]
mod tests;

fn main() {
    let arg = std::env::args().nth(1).map(PathBuf::from);
    application().with_assets(ui::Icons).run(move |cx: &mut App| {
        cx.text_system()
            .add_fonts(vec![std::borrow::Cow::Borrowed(font_data())])
            .expect("フォント登録");
        cx.bind_keys(ui::bindings("jo_edit"));
        // 前に閉じたときの姿で開く。控えが無ければ既定の大きさで中央に
        let saved = ui::winstate::load("writer");
        let bounds = match saved {
            Some(st) => Bounds::new(gpui::point(px(st.x), px(st.y)), size(px(st.w), px(st.h))),
            None => Bounds::centered(None, size(px(900.0), px(1000.0)), cx),
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
                let view = cx.new(|cx| Writer::new(arg2.clone(), cx));
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
                        ui::winstate::save("writer", ui::winstate::WinState {
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
                // 書きかけがあれば「まだ閉じない」と答え、確認は別のスレッドで出す
                let v = view.clone();
                window.on_window_should_close(cx, move |_, cx| {
                    let quit_now = v.update(cx, |this, cx| {
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
                view
            },
        )
        .unwrap();
        cx.activate(true);
    });
}

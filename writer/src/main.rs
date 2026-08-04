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

/// 本文のフォント。**同梱せず、システムから探す**
/// (埋め込むと実行ファイルがフォントを配ることになり、免許の表示義務も付く)。
///
/// 起動時に一度だけ読み、以後は借りて使う。
/// 見つからなければ**その場で止める** — 日本語が豆腐になった画面を
/// 「動いている」と見せない。
fn font_data() -> &'static [u8] {
    static FONT: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    FONT.get_or_init(|| {
        {
            // 文書が書体を指定していればそれを、無ければ機械にある日本語フォントを
            let (fam, _) = kumihan::font::for_document(None).unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            });
            kumihan::font::load(fam).unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            })
        }
    })
}

/// `RRGGBB` の1成分を 0.0〜1.0 で返す。読めない色は黒として扱う
fn hex(s: &str, i: usize) -> f32 {
    s.get(i * 2..i * 2 + 2)
        .and_then(|h| u8::from_str_radix(h, 16).ok())
        .map(|v| v as f32 / 255.0)
        .unwrap_or(0.0)
}

/// セルの文章(段落を \n で繋いだもの)。
fn cell_text(c: &kumihan::Cellbox) -> String {
    kumihan::paras_text(&c.paragraphs)
}

/// セルへ文章を戻す。段落ごとの書式は同じ位置から引き継ぐ(本文と同じ規則)。
fn set_cell_text(c: &mut kumihan::Cellbox, text: &str) {
    kumihan::set_paras_text(&mut c.paragraphs, text, SIZE_PT);
}

/// PNG / JPEG の画素数 (幅, 高さ)。読めなければ None。
/// 中身は復号しない — 大きさを知るだけなら頭を見れば足りる。
fn image_px(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        // 署名8 + 長さ4 + "IHDR"4 の後に、幅・高さが BE で並ぶ
        let w = u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?);
        let h = u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?);
        return Some((w, h));
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        let mut i = 2usize;
        while i + 9 < bytes.len() {
            if bytes[i] != 0xFF {
                return None;
            }
            let marker = bytes[i + 1];
            // 単独の印(長さ無し)は飛ばす
            if marker == 0xFF || (0xD0..=0xD9).contains(&marker) || marker == 0x01 {
                i += 2;
                continue;
            }
            let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
            // SOF0〜3 に高さ・幅
            if matches!(marker, 0xC0..=0xC3) {
                let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
                let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
                return Some((w, h));
            }
            i += 2 + len;
        }
        return None;
    }
    None
}

/// 変更履歴: 現在の段落の記(そのまま/新規/変更)。
#[derive(Clone, Copy, PartialEq, Debug)]
enum PMark {
    Same,
    New,
    /// 変更(組みになる記録開始時点の段落の番号)
    Changed(usize),
}

/// 変更履歴: 段落の列を突き合わせる(LCS)。
/// 返り値: 現在の各段落の記と、消えた段落の列(現在の何番目の前か, 元の番号)。
fn track_diff(base: &[String], cur: &[String]) -> (Vec<PMark>, Vec<(usize, usize)>) {
    let (n, m) = (base.len(), cur.len());
    let idx = |i: usize, j: usize| i * (m + 1) + j;
    let mut dp = vec![0u32; (n + 1) * (m + 1)];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[idx(i, j)] = if base[i] == cur[j] {
                dp[idx(i + 1, j + 1)] + 1
            } else {
                dp[idx(i + 1, j)].max(dp[idx(i, j + 1)])
            };
        }
    }
    // 操作の列に直す
    let mut ops: Vec<(Option<usize>, Option<usize>)> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if base[i] == cur[j] {
            ops.push((Some(i), Some(j)));
            i += 1;
            j += 1;
        } else if dp[idx(i + 1, j)] >= dp[idx(i, j + 1)] {
            ops.push((Some(i), None));
            i += 1;
        } else {
            ops.push((None, Some(j)));
            j += 1;
        }
    }
    while i < n { ops.push((Some(i), None)); i += 1; }
    while j < m { ops.push((None, Some(j))); j += 1; }
    // 隣り合う「消えた」と「増えた」は組みにして「変わった段落」とみなす
    let mut marks = vec![PMark::Same; m];
    let mut deleted: Vec<(usize, usize)> = Vec::new();
    let mut k = 0usize;
    while k < ops.len() {
        if ops[k].0.is_some() && ops[k].1.is_some() {
            k += 1;
            continue;
        }
        let mut olds: Vec<usize> = Vec::new();
        let mut news: Vec<usize> = Vec::new();
        while k < ops.len() && !(ops[k].0.is_some() && ops[k].1.is_some()) {
            match ops[k] {
                (Some(i2), None) => olds.push(i2),
                (None, Some(j2)) => news.push(j2),
                _ => unreachable!(),
            }
            k += 1;
        }
        let pair = olds.len().min(news.len());
        for t in 0..news.len() {
            marks[news[t]] = if t < pair { PMark::Changed(olds[t]) } else { PMark::New };
        }
        // 余った「消えた」は、この塊の次の現在の段落の前に置く
        let at = news.last().map(|j2| j2 + 1)
            .or_else(|| ops.get(k).and_then(|o| o.1))
            .unwrap_or(m);
        for t in pair..olds.len() {
            deleted.push((at, olds[t]));
        }
    }
    (marks, deleted)
}

/// 文字の差分(共通の頭・消えた中身・増えた中身・共通の尻尾)。
fn split_diff(old: &str, new: &str) -> (String, String, String, String) {
    let oc: Vec<char> = old.chars().collect();
    let nc: Vec<char> = new.chars().collect();
    let mut pre = 0usize;
    while pre < oc.len() && pre < nc.len() && oc[pre] == nc[pre] {
        pre += 1;
    }
    let mut suf = 0usize;
    while suf < oc.len() - pre && suf < nc.len() - pre
        && oc[oc.len() - 1 - suf] == nc[nc.len() - 1 - suf]
    {
        suf += 1;
    }
    (
        oc[..pre].iter().collect(),
        oc[pre..oc.len() - suf].iter().collect(),
        nc[pre..nc.len() - suf].iter().collect(),
        oc[oc.len() - suf..].iter().collect(),
    )
}

/// 段落の本文(ランを繋いだもの)。
fn para_text(p: &kumihan::Paragraph) -> String {
    p.runs.iter().map(|r| r.text.as_str()).collect()
}

/// 排他ロックの置き場所(LibreOffice と同じ `.~lock.名前#`)。calc と同じ形。
fn lock_path_for(p: &std::path::Path) -> PathBuf {
    let name = p.file_name().unwrap_or_default().to_string_lossy();
    p.with_file_name(format!(".~lock.{name}#"))
}

/// 自分の名乗り(誰が開いているか)。user@host。
/// Python の探し方(calc と同じ)。JO_PYTHON > .venv > python3
fn find_python() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("JO_PYTHON") {
        return p.into();
    }
    let venv = std::path::Path::new(".venv/bin/python");
    if venv.exists() {
        return venv.into();
    }
    "python3".into()
}

/// プラグイン(.py)の置き場。~/.config/office/plugins
fn plugins_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".config/office/plugins")
}

/// 署名の鍵の置き場。~/.config/office/sign.key(秘密鍵の種 32 バイト)
fn sign_key_path() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".config/office/sign.key")
}

/// 署名の鍵を読む。無ければ作る(/dev/urandom の種。0600 で置く)
fn load_or_make_key() -> Result<ed25519_dalek::SigningKey, String> {
    let kp = sign_key_path();
    if let Ok(bytes) = std::fs::read(&kp) {
        let seed: [u8; 32] = bytes
            .get(..32)
            .and_then(|b| b.try_into().ok())
            .ok_or("鍵ファイルが壊れています(~/.config/office/sign.key)")?;
        return Ok(ed25519_dalek::SigningKey::from_bytes(&seed));
    }
    let mut seed = [0u8; 32];
    use std::io::Read as _;
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut seed))
        .map_err(|e| format!("乱数が取れません: {e}"))?;
    if let Some(dir) = kp.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&kp)
        .and_then(|mut f| f.write_all(&seed))
        .map_err(|e| format!("鍵が置けません: {e}"))?;
    Ok(ed25519_dalek::SigningKey::from_bytes(&seed))
}

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok())
        .collect()
}

/// 署名の添え書きの置き場。文書の隣の 名前.docx.sig
fn sig_path_for(p: &std::path::Path) -> PathBuf {
    let mut os = p.as_os_str().to_owned();
    os.push(".sig");
    PathBuf::from(os)
}

fn lock_identity() -> String {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "?".into());
    let host = std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "?".into());
    format!("{user}@{host}")
}

/// 先客のロックを読む(あれば名乗りを返す)。自分自身のロックは先客と見ない。
fn foreign_lock(p: &std::path::Path) -> Option<String> {
    let lp = lock_path_for(p);
    let raw = std::fs::read_to_string(lp).ok()?;
    let who = raw
        .split(',')
        .map(str::trim)
        .find(|t| !t.is_empty())
        .unwrap_or("誰か")
        .to_string();
    (who != lock_identity()).then_some(who)
}

/// 文字の種類。**日本語の「語」は文字種の変わり目で切る**(分かち書きが無いので、
/// 英数の連なり・ひらがな・カタカナ・漢字・記号の境を語の境とみなす。IME や
/// エディタの通り相場)。
fn char_class(c: char) -> u8 {
    if c.is_whitespace() {
        0
    } else if c.is_ascii_alphanumeric() || c == '_' {
        1
    } else if ('ぁ'..='ゖ').contains(&c) {
        2
    } else if ('ァ'..='ヶ').contains(&c) || c == 'ー' {
        3
    } else if c.is_alphabetic() {
        4 // 漢字ほか
    } else {
        5 // 記号
    }
}

/// 語の境へ(forward なら次の語の頭、そうでなければ前の語の頭)。バイト位置。
fn word_boundary(text: &str, pos: usize, forward: bool) -> usize {
    if forward {
        let chars: Vec<(usize, char)> = text[pos..].char_indices()
            .map(|(i, c)| (pos + i, c)).collect();
        let mut k = 0;
        while k < chars.len() && char_class(chars[k].1) == 0 {
            k += 1;
        }
        if k >= chars.len() {
            return text.len();
        }
        let cl = char_class(chars[k].1);
        while k < chars.len() && char_class(chars[k].1) == cl {
            k += 1;
        }
        // 次の語の頭まで(語の後ろの空白も飛ばす)
        while k < chars.len() && char_class(chars[k].1) == 0 {
            k += 1;
        }
        chars.get(k).map(|(i, _)| *i).unwrap_or(text.len())
    } else {
        let chars: Vec<(usize, char)> = text[..pos].char_indices().collect();
        let mut k = chars.len();
        while k > 0 && char_class(chars[k - 1].1) == 0 {
            k -= 1;
        }
        if k == 0 {
            return 0;
        }
        let cl = char_class(chars[k - 1].1);
        while k > 0 && char_class(chars[k - 1].1) == cl {
            k -= 1;
        }
        chars.get(k).map(|(i, _)| *i).unwrap_or(0)
    }
}

const PX_PER_MM: f32 = 96.0 / 25.4;
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
    /// 置換の板。開いている間、打鍵は検索欄に入る
    find_open: bool,
    /// 0=検索語 1=置換後
    find_field: usize,
    find_ed: Editor,
    repl_ed: Editor,
    /// ヘッダー・フッターの編集の板。Some(false)=ヘッダー / Some(true)=フッター。
    /// 開いている間、打鍵はここに入る(検索の板と同じ方式)
    hf_edit: Option<bool>,
    hf_ed: Editor,
    /// コメントの板(開いている間、打鍵はここに入る)と、付け先の段落番号
    cmt_edit: bool,
    cmt_ed: Editor,
    cmt_para: usize,
    /// 透かしの板
    wm_edit: bool,
    wm_ed: Editor,
    /// しおりの板(名前の入力欄つきの一覧)
    bm_open: bool,
    bm_ed: Editor,
    /// バージョン履歴の板(上書き保存のたびに残る控えの一覧)
    hist_open: bool,
    /// プラグインの板(置き場の .py 一覧)
    plug_open: bool,
    /// リボンの絵釦に乗ったときの説明(下のステータスバーに出す)
    hover_hint: Option<&'static str>,
    /// ファイルのページ(タブ0)から戻る先のタブ
    prev_tab: usize,
    /// ファイルのページの右側(0=詳細情報 1=最近開いた)
    file_view: u8,
    /// 文書の情報で編集中の欄(0=作成者 1=タイトル 2=タグ 3=件名 4=コメント)
    file_field: Option<u8>,
    prop_ed: Editor,
    /// 暗号化のパスワード。Some なら保存で ECMA-376 Standard に包む
    encrypt_pw: Option<String>,
    /// パスワードの板。pw_pending が Some なら「開くために聞いている」
    pw_open: bool,
    pw_ed: Editor,
    pw_pending: Option<PathBuf>,
    /// マクロで置き換える直前の文書(Ctrl+Z で1手で戻すため)
    doc_undo: Option<Document>,
    /// チャット(文書の隣の申し送り帳)の板と入力欄
    chat_open: bool,
    chat_ed: Editor,
    /// 相互参照の板(しおり一覧から「文字」「ページ」を挿す)
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
        // 置換・ヘッダーの板が開いている間、入力(IME含む)はそちらへ入る。
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
        if self.chat_open || self.file_field.is_some() {
            // チャット・文書の情報の入力欄。打鍵は(確定まで)文書を変えない
            return;
        }
        if self.protected() {
            // 読み取り専用の保護。**打った分を取り消して、文書は変えない。**
            // 板(ヘッダー等)の打鍵は文書に入る前なので、板ごと閉じて捨てる
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
                "読み取り専用で保護されています(保護タブの「保護」で解除できます)".into();
            return;
        }
        if let Some(footer) = self.hf_edit {
            // 板の打鍵はその場で文書のヘッダー・フッターに反映する
            let text = self.hf_ed.text().to_string();
            let hf = if footer { &mut self.doc.footer } else { &mut self.doc.header };
            kumihan::set_paras_text(&mut hf.paragraphs, &text, SIZE_PT);
            self.dirty = true;
            self.refresh_hf();
            return;
        }
        if self.bm_open {
            // しおりの板は名前の入力欄。打鍵は文書を変えない
            return;
        }
        if self.wm_edit {
            // 透かしの板。空にすると外れる
            let text = self.wm_ed.text().to_string();
            self.doc.watermark = if text.is_empty() { None } else { Some(text) };
            self.dirty = true;
            return;
        }
        if self.cmt_edit {
            // コメントの板。空にすると外れる(1つ目のコメントを編集する)
            let text = self.cmt_ed.text().to_string();
            let author = std::env::var("USER").unwrap_or_else(|_| "私".into());
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
            prev_tab: 1,
            file_view: 0,
            file_field: None,
            prop_ed: Editor::new(""),
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
            Target::Body => "本文".into(),
            Target::Cell { row, col, .. } => {
                format!("表のセル({}行 {}列)を編集中", row + 1, col + 1).into()
            }
        };
    }

    fn relayout(&mut self) {
        self.flush_target();
        let m = Metrics::new(&self.font_bytes).expect("フォント");
        // 段組みなら1段の行長で組み、ページの物理座標へ折る。
        // 折った後の座標は画面もクリックも PDF もそのまま使える
        let y0 = self.pg.top_mm + 4.0;
        self.page = layout(
            &self.doc,
            &m,
            &Frame { measure_mm: self.pg.column_measure_mm(), line_height_mm: LINE_MM, y0_mm: y0 },
        );
        kumihan::fold_columns(&mut self.page, &self.pg, y0);
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
        let total = self.total_pages();
        self.header_lines =
            kumihan::layout_hf(&self.doc.header, &m, &self.pg, LINE_MM, 1, total, false);
        self.footer_lines =
            kumihan::layout_hf(&self.doc.footer, &m, &self.pg, LINE_MM, 1, total, true);
    }

    /// ヘッダー・フッターの編集の板を開く(もう一度で閉じる)。
    fn open_hf(&mut self, footer: bool) {
        if self.hf_edit == Some(footer) {
            self.hf_edit = None;
            return;
        }
        let hf = if footer { &self.doc.footer } else { &self.doc.header };
        let which = if footer { "フッター" } else { "ヘッダー" };
        if hf.paragraphs.is_empty() && hf.part.is_some() {
            // 読めたが持てなかった部品(表入りなど)。嘘の編集をさせない
            self.status = format!(
                "この{which}には表があり、この版では編集できません(保存では残ります)").into();
            return;
        }
        self.find_open = false;
        self.hf_edit = Some(footer);
        self.hf_ed = Editor::new(&kumihan::paras_text(&hf.paragraphs));
        self.status = format!("{which}を編集中(全ページ共通。Esc で閉じる)").into();
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
                            format!("書体「{w}」が無いので「{}」で表示", fam.name).into(),
                        );
                    }
                }
            }
            Err(e) => self.status = e.into(),
        }
    }

    /// パスワードの板の Enter。開き待ちがあれば解いて開き、
    /// 無ければ「次の保存から暗号化」を決める(空なら解除)
    fn pw_commit(&mut self) {
        let pw = self.pw_ed.text().to_string();
        if let Some(p) = self.pw_pending.take() {
            let bytes = match std::fs::read(&p) {
                Ok(b) => b,
                Err(e) => {
                    self.pw_open = false;
                    self.status = format!("開けません: {e}").into();
                    return;
                }
            };
            match ooxml::crypt::decrypt(&bytes, &pw) {
                Ok(plain) => {
                    self.pw_open = false;
                    self.open_plain(p.clone(), plain);
                    if self.path.as_deref() == Some(p.as_path()) {
                        self.encrypt_pw = Some(pw);
                        self.status = format!(
                            "{}(保存も同じパスワードで暗号化します)",
                            self.status
                        )
                        .into();
                    }
                }
                Err(e) => {
                    // 板は開いたまま。打ち直せる
                    self.pw_pending = Some(p);
                    self.pw_ed = Editor::new("");
                    self.status = e.into();
                }
            }
        } else {
            self.pw_open = false;
            if pw.is_empty() {
                self.encrypt_pw = None;
                self.status = "暗号化しません(次の保存から普通の docx)".into();
            } else {
                self.encrypt_pw = Some(pw);
                self.dirty = true;
                self.status = "次の保存から、このパスワードで暗号化します\
                               (AES-128。Word や LibreOffice でも開けます)"
                    .into();
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

    /// マクロ = **檻(bubblewrap)の中の Python** が python-docx で文書の
    /// **複製**を直し、直った複製を読み込む(失敗しても文書は無傷)。
    /// 文書にコードは載せない — 「開く=実行」を作らない設計はそのまま。
    /// 台本の中で d が python-docx の Document。戻すのは Ctrl+Z の1手
    fn run_macro_file(&mut self, py_file: PathBuf, cx: &mut Context<Self>) {
        self.flush_target();
        let user_code = match std::fs::read_to_string(&py_file) {
            Ok(c) => c,
            Err(e) => {
                self.status = format!("マクロが読めません: {e}").into();
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
            self.status = format!("マクロに渡せません: {e}").into();
            return;
        }
        let script = format!(
            concat!(
                "import docx
",
                "d = docx.Document({in_d:?})
",
                "# ---- 利用者のコード(d = python-docx の文書) ----
",
                "{code}
",
                "# ----
",
                "d.save({out_d:?})
"
            ),
            in_d = in_d.to_string_lossy(),
            out_d = out_d.to_string_lossy(),
            code = user_code
        );
        let name = py_file
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        self.status = format!("マクロ {name} を実行しています…(檻の中の Python)").into();
        let task = cx.background_executor().spawn(async move {
            let py_path = dir.join("run.py");
            std::fs::write(&py_path, script).map_err(|e| e.to_string())?;
            let py = find_python();
            let have_bwrap = std::path::Path::new("/usr/bin/bwrap").exists();
            let mut cmd = if have_bwrap {
                // 檻: / は読み取り専用、ホームは空、書けるのは作業場だけ、
                // ネット無し(calc の Python と同じ檻)
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
                .map_err(|e| format!("Python が起動できません: {e}"))?;
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
                    "python-docx がありません(pip install python-docx。\
                     .venv があればそちらへ)"
                        .to_string()
                } else {
                    last
                });
            }
            std::fs::read(&out_d)
                .map_err(|e| format!("結果が読めません: {e}"))
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
                                    format!("マクロ {name} を実行しました(Ctrl+Z で戻せます)")
                                        .into()
                                } else {
                                    format!(
                                        "マクロ {name}: {}(Ctrl+Z で戻せます)",
                                        out.lines().last().unwrap_or_default()
                                    )
                                    .into()
                                };
                            }
                            Err(e) => this.status = format!("結果が読めません: {e}").into(),
                        }
                    }
                    Err(e) => this.status = format!("マクロ: {e}").into(),
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
                "未保存の変更があります。先に保存してください(Ctrl+S)".into();
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
        self.status = "新しい文書です".into();
        true
    }

    /// 名前を付けて保存(いつでもダイアログ。別の糸 — rfd は同期)
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
                "読み取り専用で保護されています(保護タブの「保護」で解除できます)"
                    .into();
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
        self.status = "文書の情報を控えました(保存で docx に入ります)".into();
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
                self.status = format!("控えが読めません: {e}").into();
                return;
            }
        };
        let bytes = if ooxml::crypt::is_encrypted(&bytes) {
            match self.encrypt_pw.as_ref().map(|pw| ooxml::crypt::decrypt(&bytes, pw)) {
                Some(Ok(b)) => b,
                _ => {
                    self.status =
                        "控えは暗号化されています(いまのパスワードでは解けません)"
                            .into();
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
                self.status = "控えを開きました(名無しの複製。保存で名前を聞きます。\
                               元へ戻すなら同じ名前で保存)"
                    .into();
            }
            Err(e) => self.status = format!("控えが読めません: {e}").into(),
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
                "まだファイルになっていません(保存すると申し送り帳が持てます)".into();
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
                    "書き残しました(文書の隣の .chat.txt。開いた人が読めます)".into();
            }
            Err(e) => self.status = format!("チャットに書けません: {e}").into(),
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
                self.status = format!("開けません: {e}").into();
                return;
            }
        };
        if ooxml::crypt::is_encrypted(&bytes) {
            // 板でパスワードを聞き、Enter(pw_commit)が続きをやる
            self.pw_pending = Some(p);
            self.pw_open = true;
            self.pw_ed = Editor::new("");
            self.status =
                "この文書は暗号化されています。パスワードを打って Enter".into();
            return;
        }
        self.open_plain(p, bytes);
    }

    /// 平文(zip)の docx を読み込む。open と pw_commit の共通の続き
    fn open_plain(&mut self, p: PathBuf, bytes: Vec<u8>) {
        self.target = Target::Body;
        // 前の文書の板が残っていると、打鍵が新しい文書のヘッダーを潰す
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
                self.status = format!(
                    "{} 段落 / 表 {} — {}",
                    rep.paragraphs,
                    doc.tables().count(),
                    p.file_name().unwrap_or_default().to_string_lossy()
                )
                .into();
                self.pg = doc.page.unwrap_or_default();
                self.set_doc(doc);
                self.adopt_font();
                self.relayout_keep();
                // 排他(共有フォルダの「後勝ちで潰す」を防ぐ。calc と同じ)
                self.acquire_lock(&p);
                if let Some(who) = self.locked_by.clone() {
                    self.status = format!(
                        "{} — **{who} が開いています**。上書き保存はできません(別の名前で保存へ)",
                        self.status
                    )
                    .into();
                }
                if self.doc.protection.is_some() {
                    self.status = format!(
                        "{} — 読み取り専用で保護されています(保護タブで解除できます)",
                        self.status
                    )
                    .into();
                }
                Self::note_recent(&p);
                self.path = Some(p);
                self.dirty = false;
            }
            Err(e) => self.status = format!("開けません: {e}").into(),
        }
    }

    /// 保存。名前が無ければ選ばせる(**ダイアログは別の糸** — rfd は同期で、
    /// 主の糸で開くと画面ごと固まる。calc と同じ作法)。
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
            self.status = format!(
                "{} が開いているため上書きしません。別の名前で保存します",
                self.locked_by.as_deref().unwrap_or("誰か")
            )
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
                    None => this.status = "保存をやめました(名前が決まっていません)".into(),
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
                self.status = format!(
                    "保存しました — {}{enc_note}{caveat}",
                    p.file_name().unwrap_or_default().to_string_lossy()
                )
                .into();
                // 保存先のロックを取り直す(別の名前で保存したときは
                // 新しいファイルの側を守る。同じ名前なら実質そのまま)
                self.acquire_lock(&p);
                Self::note_recent(&p);
                self.path = Some(p);
                self.dirty = false;
            }
            Err(e) => self.status = format!("保存できません: {e}").into(),
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
        for line in self.page.lines.iter().filter(|l| match want {
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
            hit = Some((self.pg.left_mm + x, line.y_mm, pt));
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
                "読み取り専用で保護されています(保護タブの「保護」で解除できます)".into();
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

    /// PDF として保存。保存先の選択は**別の糸**(rfd は同期)。
    fn save_pdf(&mut self, cx: &mut Context<Self>) {
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
            Ok(_) => format!("PDF にしました — {}", p.file_name().unwrap_or_default().to_string_lossy()).into(),
            Err(e) => format!("PDF にできません: {e}").into(),
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
        self.status = format!(
            "用紙 {:.0}×{:.0}mm / 余白 {:.0}mm{}",
            self.pg.w_mm,
            self.pg.h_mm,
            self.pg.left_mm,
            if self.pg.cols() > 1 { format!(" / {}段組み", self.pg.cols()) } else { String::new() }
        )
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
            self.status = format!("しおり「{name}」が見つかりません").into();
            return;
        };
        let start = self.ed.selection().start;
        handler::replace(self, None, &val);
        self.doc.apply_field(
            start..start + val.len(),
            Some(kumihan::RefField { name: name.to_string(), page }),
        );
        self.relayout_keep();
        self.status = format!(
            "「{name}」への参照を挿しました({}。参照は編集で中を触ると普通の字に戻ります)",
            if page { "ページ番号" } else { "しおりの文字" }
        )
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
                format!("参照を {n} 箇所更新しました(この操作は戻せません)").into();
        } else {
            self.status = "参照は最新です".into();
        }
    }

    /// しおりを追加する(カーソルの段落へ)。
    fn bm_add(&mut self) {
        let name = self.bm_ed.text().trim().to_string();
        if name.is_empty() {
            self.status = "しおりの名前を打ってから追加してください".into();
            return;
        }
        if self.doc.paragraphs().any(|p| p.bookmarks.iter().any(|b| *b == name)) {
            self.status = format!("しおり「{name}」は既にあります").into();
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
        self.status = format!("しおり「{name}」を付けました(保存で docx に入ります)").into();
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
            0 => "標準の段落にしました".into(),
            n => format!("見出し{n} にしました(参考資料 > 目次 の材料になります)").into(),
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
                "見出しがありません(ホーム > 段落のスタイルで見出しを付けてください)".into();
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
                (*n, format!("{head}　{}　{num}", "…".repeat(dots)))
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
            format!("目次を更新しました({} 項目)", lines.len()).into()
        } else {
            format!("目次を入れました({} 項目。見出しを変えたら「目次の更新」)", lines.len())
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
                "図表番号がありません(参考資料 > 図表番号で付けてください)".into();
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
                        text: format!("{t}　{}　{num}", "…".repeat(dots)),
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
            format!("図表目次を更新しました({n} 項目)").into()
        } else {
            format!("図表目次を入れました({n} 項目)").into()
        };
    }

    /// 書式を触ったあとの組み直し。**本文を戻さない**
    /// (戻すと今つけた書式が消える)。
    fn relayout_keep(&mut self) {
        let m = Metrics::new(&self.font_bytes).expect("フォント");
        let y0 = self.pg.top_mm + 4.0;
        self.page = layout(
            &self.doc,
            &m,
            &Frame { measure_mm: self.pg.column_measure_mm(), line_height_mm: LINE_MM, y0_mm: y0 },
        );
        kumihan::fold_columns(&mut self.page, &self.pg, y0);
        self.refresh_hf();
    }

    /// クリックした画素位置(編集領域からの相対)にカーソルを置く。
    /// 文書の下端(紙の座標 mm)。1ページに満たなくても紙1枚ぶんは白い
    fn content_mm(&self) -> f32 {
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
            self.status = "検索語が空です".into();
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
            None => self.status = format!("「{term}」は見つかりません").into(),
        }
    }

    /// いま選ばれている一致を置き換えて、次へ。
    fn replace_current(&mut self) {
        if self.protected() {
            self.status =
                "読み取り専用で保護されています(保護タブの「保護」で解除できます)".into();
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
                "読み取り専用で保護されています(保護タブの「保護」で解除できます)".into();
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
        self.status = format!("{n} 件を置き換えました").into();
    }

    /// run_cmd が処理できる id。**リボンの ready はこの表の中に限る**
    const HANDLED: &'static [&'static str] = &[
        "open", "save", "undo", "redo", "selectall", "pdf",
        "bold", "italic", "underline", "strikeout", "fontcolor",
        "superscript", "subscript", "highlight", "clearstyle",
        "align-left", "align-center", "align-right", "align-just", "align-dist",
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
                        self.status = "PNG・JPEG・SVG だけ挿せます".into();
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
                    "SVG を高精細の画像にして挿しました(保存で docx に入ります)".into()
                } else {
                    "画像を挿しました(段落の下に付き、保存で docx に入ります)".into()
                };
            }
            Err(e) => self.status = format!("読めません: {e}").into(),
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
                    self.status = format!("読めません: {e}").into();
                    return;
                }
            }
        } else {
            match std::fs::read(path) {
                Ok(b) => match String::from_utf8(b) {
                    Ok(t) => t,
                    Err(_) => {
                        // 文字コードの推測はしない(化けた本文を黙って挿すより断る)
                        self.status = "UTF-8 のテキストだけ読めます".into();
                        return;
                    }
                },
                Err(e) => {
                    self.status = format!("読めません: {e}").into();
                    return;
                }
            }
        };
        if text.is_empty() {
            self.status = "空のファイルです".into();
            return;
        }
        self.switch_target(Target::Body);
        handler::replace(self, None, &text);
        self.status = format!(
            "{} を差し込みました({} 文字)",
            path.file_name().unwrap_or_default().to_string_lossy(),
            text.chars().count()
        )
        .into();
    }

    /// 開くファイルを選ぶ(**ダイアログは別の糸**)。
    fn open_dialog(&mut self, cx: &mut Context<Self>) {
        let ask = cx.background_executor().spawn(async {
            rfd::FileDialog::new().add_filter("Word文書", &["docx"]).pick_file()
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

    fn run_cmd(&mut self, id: &str, cx: &mut Context<Self>) {
        // 読み取り専用の保護。文書を変える釦はここで断る(見る・出す・
        // 保存・検索の類いは通す)。解除はいつでも「保護」の釦1手
        const READONLY_OK: &[&str] = &[
            "open", "save", "pdf", "zoom-in", "zoom-out", "ruler", "darkmode",
            "line-numbers", "hidenchars", "selectall", "spell", "wordcount",
            "co-showcomment", "replace", "prot-doc", "coauth-mode",
            "co-history", "co-chat", "prot-encrypt", "prot-sign", "copy",
        ];
        if self.protected() && !READONLY_OK.contains(&id) {
            self.status =
                "読み取り専用で保護されています(保護タブの「保護」で解除できます)".into();
            return;
        }
        match id {
            "open" => self.open_dialog(cx),
            "save" => self.save(false, cx),
            "undo" => { if self.editor().undo() { self.on_edited() } }
            "redo" => { if self.editor().redo() { self.on_edited() } }
            "selectall" => self.ed.select_all(),
            "spell" => self.run_proof(),
            // 文字書式 — 押すたびに入切する(Word と同じ挙動)。
            // **先にカーソル位置の書式で入か切かを決めて、選択全体に写す** —
            // 混ざった選択で run ごとに反転させない(Word の作法)
            "bold" => {
                let on = !self.doc.char_format_at(self.ed.selection()).bold;
                self.toggle(move |f| f.bold = on);
            }
            "italic" => {
                let on = !self.doc.char_format_at(self.ed.selection()).italic;
                self.toggle(move |f| f.italic = on);
            }
            "underline" => {
                let on = !self.doc.char_format_at(self.ed.selection()).underline;
                self.toggle(move |f| f.underline = on);
            }
            "strikeout" => {
                let on = !self.doc.char_format_at(self.ed.selection()).strike;
                self.toggle(move |f| f.strike = on);
            }
            // 上付きと下付きは同時には成らない
            "superscript" => {
                let on = !self.doc.char_format_at(self.ed.selection()).superscript;
                self.toggle(move |f| {
                    f.superscript = on;
                    if on { f.subscript = false }
                });
            }
            "subscript" => {
                let on = !self.doc.char_format_at(self.ed.selection()).subscript;
                self.toggle(move |f| {
                    f.subscript = on;
                    if on { f.superscript = false }
                });
            }
            // 蛍光ペン。黄 → 緑 → 解除(色を選ぶ小窓はまだ無い)
            "highlight" => {
                let next = match self.doc.char_format_at(self.ed.selection())
                    .highlight.as_deref()
                {
                    None => Some("yellow".to_string()),
                    Some("yellow") => Some("green".to_string()),
                    _ => None,
                };
                self.toggle(move |f| f.highlight = next.clone());
            }
            // 書式のクリア。文字書式だけを外す(本文と段落の性質は残す)
            "clearstyle" => self.toggle(|f| *f = Default::default()),
            // 段落の揃え
            "align-left" => self.set_align(Align::Left),
            "align-center" => self.set_align(Align::Center),
            "align-right" => self.set_align(Align::Right),
            "align-just" => self.set_align(Align::Justify),
            // 均等割付(日本語一級)。最後の行も行長いっぱいに字間を配る
            "align-dist" => self.set_align(Align::Distribute),
            // 文字の大きさ
            "incfont" => self.size(|s| s + 1.0),
            "decfont" => self.size(|s| s - 1.0),
            // 印刷・PDF。**組み直さない** — 画面と同じ紙面をそのまま写す
            "pdf" => self.save_pdf(cx),
            // 文字色。押すたびに 赤 → 青 → 黒(解除)と回す。
            // 色を選ぶ小窓はまだ無いので、**無い機能を有るように見せず**
            // 使える範囲で回す形にしてある
            // 箇条書き・段落番号。押すたびに入切する
            "markers" => self.para(|p| {
                p.list = if p.list == ListKind::Bullet { ListKind::None } else { ListKind::Bullet }
            }),
            // 複数レベルのリスト。箇条書きにして1段深く(印はレベルで変わる)。
            // 深さは Tab / Shift+Tab でも動かせる
            "multilevels" => {
                self.para(|p| {
                    if p.list == ListKind::None {
                        p.list = ListKind::Bullet;
                    } else {
                        p.indent = (p.indent + 1).min(8);
                    }
                });
                self.status =
                    "レベル付きのリストです(Tab / Shift+Tab で深さ。印はレベルで変わる)".into();
            }
            "numbering" => self.para(|p| {
                p.list = if p.list == ListKind::Number { ListKind::None } else { ListKind::Number }
            }),
            // インデント。0〜20段に留める
            "incoffset" => self.para(|p| p.indent = (p.indent + 1).min(20)),
            "decoffset" => self.para(|p| p.indent = p.indent.saturating_sub(1)),
            // 行間。1.0 → 1.5 → 2.0 → 1.0 と回す(小窓がまだ無いので)
            // この段落の前で改ページ(押すたびに入切)
            "pagebreak" => self.para(|p| p.page_break_before = !p.page_break_before),
            // 段落の背景色。無し → 薄黄 → 薄青 → 無し、で回す
            "paracolor" => self.para(|p| {
                p.shade = match p.shade.as_deref() {
                    None => Some("FFF2CC".into()),
                    Some("FFF2CC") => Some("DEEAF6".into()),
                    _ => None,
                }
            }),
            // 段落の囲み枠(入切)
            "borders" => self.para(|p| p.boxed = !p.boxed),
            // ドロップキャップ(頭の1字を大きく。押すたびに入切)
            "dropcap" => {
                self.para(|p| p.dropcap = !p.dropcap);
                self.status =
                    "ドロップキャップを切り替えました(docx では Word の枠になります)".into();
            }
            // 画像の挿入。段落の下に付く(選択も**別の糸**)。
            // 図形・グラフ・SmartArt・テキストアート・方程式も同じ道 —
            // **絵は Python で描いて画像として貼る**(SEKKEI「writer の挿入系」)。
            // 灰色で残すより、方針どおりに動く釦にする(発注者判断)
            "insimage" | "insshape" | "inssmartart" | "inschart" | "smartpicker"
            | "instextart" | "insequation" => {
                if id != "insimage" {
                    self.status =
                        "図は Python(matplotlib 等)で描いて貼ります(SVG なら拡大しても粗くなりません)"
                            .into();
                }
                let ask = cx.background_executor().spawn(async {
                    rfd::FileDialog::new()
                        .add_filter("画像", &["png", "jpg", "jpeg", "svg"])
                        .pick_file()
                });
                cx.spawn(async move |this, cx| {
                    let r = ask.await;
                    let _ = this.update(cx, |this, cx| {
                        if let Some(p) = r {
                            this.insert_image(&p);
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            // テキストボックス = 1×1 の表。枠の中に文字が要る様式は
            // 表で組むのが日本の事務の通り相場(SEKKEI)
            "instext" => {
                let empty = kumihan::Cellbox {
                    paragraphs: vec![kumihan::Paragraph {
                        runs: vec![kumihan::Run {
                            text: String::new(),
                            size_pt: SIZE_PT,
                            font: None,
                            fmt: Default::default(),
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                };
                self.flush_target();
                self.doc.blocks.push(kumihan::Block::Table(kumihan::Table {
                    col_mm: vec![80.0],
                    rows: vec![vec![empty]],
                }));
                self.dirty = true;
                self.relayout_keep();
                self.status =
                    "1×1 の枠を末尾に入れました(クリックして中に書けます)".into();
            }
            // 大文字小文字。選択の英字を 全部大文字 ⇄ 全部小文字 で切り替える
            // (小文字が混ざっていれば大文字へ。1手で戻せる)
            "changecase" => {
                let sel = self.ed.selection();
                if sel.is_empty() {
                    self.status = "変えたい文字を選んでください".into();
                } else if let Some(t) = self.ed.text().get(sel.clone()) {
                    let up = t.chars().any(|c| c.is_lowercase());
                    let new = if up { t.to_uppercase() } else { t.to_lowercase() };
                    let start = sel.start;
                    let n = new.len();
                    self.ed.insert(&new);
                    // 選択を保つ(続けてもう一度押せるように)
                    self.ed.move_to(start, false);
                    self.ed.move_to(start + n, true);
                    self.on_edited();
                }
            }
            // 空白ページの挿入 = 段落を切って、新しい段落を次の頁の頭から
            "blankpage" => {
                handler::replace(self, None, "\n");
                self.para(|p| p.page_break_before = true);
                self.status = "ここから新しいページになります".into();
            }
            // 表の挿入。3×3 を末尾に(大きさを選ぶ小窓はまだ無い)。
            // セル編集が入っているので、挿した表はそのまま書ける
            "instable" => {
                let empty = || kumihan::Cellbox {
                    paragraphs: vec![kumihan::Paragraph {
                        runs: vec![kumihan::Run {
                            text: String::new(),
                            size_pt: SIZE_PT,
                            font: None,
                            fmt: Default::default(),
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                };
                self.flush_target();
                self.doc.blocks.push(kumihan::Block::Table(kumihan::Table {
                    col_mm: vec![],
                    rows: (0..3).map(|_| (0..3).map(|_| empty()).collect()).collect(),
                }));
                self.dirty = true;
                self.relayout_keep();
                self.status = "3×3 の表を末尾に入れました(セルをクリックで編集)".into();
            }
            // 記号の一覧(押すと出る/消える)
            "inssymbol" => self.symbols = !self.symbols,
            // ファイルからのテキスト。カーソルの位置に差し込む(undo の1手)
            "text-from-file" => {
                let ask = cx.background_executor().spawn(async {
                    rfd::FileDialog::new()
                        .add_filter("テキスト / Word文書", &["txt", "md", "docx"])
                        .pick_file()
                });
                cx.spawn(async move |this, cx| {
                    let r = ask.await;
                    let _ = this.update(cx, |this, cx| {
                        if let Some(p) = r {
                            this.insert_text_from(&p);
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            // テキストの追加(参考資料)= この段落を目次の材料にする。
            // 押すたびに 標準 → 見出し1 → 2 → 3 → 標準 と回る
            "add-text" => {
                let sel = self.ed.selection();
                let now = match self.target {
                    Target::Body => self.doc.para_at(sel).map(|p| p.style).unwrap_or_default(),
                    Target::Cell { .. } => Default::default(),
                };
                let next = match now {
                    kumihan::ParaStyle::Heading(n) if n < 3 => n + 1,
                    kumihan::ParaStyle::Heading(_) => 0,
                    _ => 1,
                };
                self.set_para_style(next);
            }
            // 置換の板。開いている間、打鍵は検索欄に入る
            "replace" => {
                self.find_open = !self.find_open;
                self.find_field = 0;
                if self.find_open {
                    self.switch_target(Target::Body);
                    self.status = "検索語を打って Enter で次へ".into();
                }
            }
            // 画面の倍率。50〜200%。紙は変わらない
            "zoom-in" => self.zoom = (self.zoom + 0.1).min(2.0),
            // 見え方だけの切り替え(文書は変わらない)
            "hidenchars" => self.show_marks = !self.show_marks,
            // 一覧板(フォント・大きさ)。選ぶのは板の中
            "fontname" => { self.font_list = !self.font_list; self.size_list = false;
                            self.style_list = false; }
            // 用紙。向き / サイズ / 余白(選ぶ小窓は無いが、回して選べる)
            "pageorient" => self.set_page(|pg| {
                std::mem::swap(&mut pg.w_mm, &mut pg.h_mm);
            }),
            "pagesize" => self.set_page(|pg| {
                // A4 → B5 → A3 → A4(向きは保つ)
                let landscape = pg.w_mm > pg.h_mm;
                let (w, h) = match (pg.w_mm.min(pg.h_mm) * 10.0) as u32 {
                    2100 => (182.0, 257.0), // → B5
                    1820 => (297.0, 420.0), // → A3
                    _ => (210.0, 297.0),    // → A4
                };
                (pg.w_mm, pg.h_mm) = if landscape { (h, w) } else { (w, h) };
            }),
            // 段組み。1 → 2 → 3 → 1 と回る(見た目も docx も追随)
            "columns" => self.set_page(|pg| {
                pg.columns = match pg.cols() {
                    1 => 2,
                    2 => 3,
                    _ => 1,
                };
            }),
            "pagemargins" => self.set_page(|pg| {
                // 標準20 → 狭い12 → 広い30 → 標準
                let next = match pg.left_mm as u32 {
                    20 => 12.0,
                    12 => 30.0,
                    _ => 20.0,
                };
                pg.left_mm = next;
                pg.right_mm = next;
                pg.top_mm = next;
                pg.bottom_mm = next;
            }),
            "fontsize" => { self.size_list = !self.size_list; self.font_list = false;
                            self.style_list = false; }
            // 段落のスタイルの一覧(標準・見出し1〜3)
            "parastyle" => { self.style_list = !self.style_list;
                             self.font_list = false; self.size_list = false; }
            // 目次。挿す・挿し直すは同じ道(Toc の印の連続を置き換える)
            "toc" | "toc-update" => self.make_toc(),
            // 図表目次も同じ作法(Tof の印)
            "tof" | "tof-update" => self.make_tof(),
            // ヘッダー・フッターの編集(板。開いている間、打鍵はそこへ)
            "edit-header" => self.open_hf(false),
            "edit-footer" => self.open_hf(true),
            // ページ番号・ページ数。開いている板(無ければフッター)の
            // カーソル位置に印を入れる
            "pagenum" | "numpages" => {
                if self.hf_edit.is_none() {
                    self.open_hf(true);
                }
                if self.hf_edit.is_some() {
                    let (mark, what) = if id == "pagenum" {
                        (kumihan::PAGE_MARK, "ページ番号")
                    } else {
                        (kumihan::PAGES_MARK, "ページ数")
                    };
                    self.hf_ed.insert(&mark.to_string());
                    self.on_edited();
                    self.status =
                        format!("{what}を入れました(docx ではフィールドになります)").into();
                }
            }
            // 日付。**固定の文字**として入れる(開くたび変わるフィールドは、
            // 事務の書類では事故のもと — 提出日が勝手に変わる)
            "datetime" => {
                let out = std::process::Command::new("date")
                    .arg("+%Y年%-m月%-d日")
                    .output();
                match out {
                    Ok(o) if o.status.success() => {
                        let d = String::from_utf8_lossy(&o.stdout).trim().to_string();
                        if self.hf_edit.is_some() {
                            self.hf_ed.insert(&d);
                        } else {
                            self.ed.insert(&d);
                        }
                        self.on_edited();
                        self.status =
                            format!("今日の日付を入れました({d}。固定の文字です)").into();
                    }
                    _ => self.status = "日付が取れません(date コマンド)".into(),
                }
            }
            "ruler" => self.ruler = !self.ruler,
            // ダークモード。**紙は白いまま**(画面と紙の一致)。周りだけ暗くする
            "darkmode" => self.dark = !self.dark,
            // 変更履歴。記録中の編集は、保存で Word の w:ins / w:del になる
            "track-changes" => {
                self.flush_target();
                self.track = !self.track;
                if self.track {
                    self.track_base =
                        Some(self.doc.paragraphs().map(para_text).collect());
                    self.status =
                        "変更履歴を記録します(保存で Word の変更履歴になります)".into();
                } else {
                    self.track_base = None;
                    self.status =
                        "変更履歴の記録をやめました(記録していた差分は捨てました)".into();
                }
            }
            // 描画。ペン・蛍光ペン・消しゴム(もう一度押すか Esc で戻る)。
            // 筆は文書に入り、docx では自由曲線の図形になる(ページに固定)
            "pen" | "highlighter" | "eraser" => {
                let t = match id { "pen" => 0u8, "highlighter" => 1, _ => 2 };
                self.tool = if self.tool == Some(t) { None } else { Some(t) };
                self.ink_cur = None;
                self.status = match self.tool {
                    Some(0) => "ペン: 紙の上をドラッグで描く(もう一度押すか Esc で戻る)".into(),
                    Some(1) => "蛍光ペン: ドラッグで引く(文字の下に薄く入る)".into(),
                    Some(2) => "消しゴム: 線をなぞると1筆ずつ消える".into(),
                    _ => "文字の編集に戻りました".into(),
                };
            }
            // 図表番号。カーソルの段落の下に「図 N」を入れる
            // (画像は段落の下に付くので、その下=図の下になる)。
            // 番号は既にある「図 n」の最大 + 1
            "caption" => {
                self.switch_target(Target::Body);
                self.flush_target();
                let mut n = 0usize;
                for p in self.doc.paragraphs() {
                    let t: String = p.runs.iter().map(|r| r.text.as_str()).collect();
                    if let Some(rest) = t.trim().strip_prefix("図 ") {
                        if let Ok(k) = rest.trim().parse::<usize>() {
                            n = n.max(k);
                        }
                    }
                }
                let label = format!("図 {}", n + 1);
                let (pi, b0) = self.cursor_para();
                let plen: usize = self
                    .doc
                    .paragraphs()
                    .nth(pi)
                    .map(|p| p.runs.iter().map(|r| r.text.len()).sum())
                    .unwrap_or(0);
                // 編集(undo の1手)と blocks を同じ形で揃える(目次と同じ作法)
                let end = b0 + plen;
                self.ed.move_to(end, false);
                self.ed.move_to(end, true);
                self.ed.insert(&format!("\n{label}"));
                let para_block_idx: Vec<usize> = self
                    .doc
                    .blocks
                    .iter()
                    .enumerate()
                    .filter(|(_, b)| matches!(b, kumihan::Block::Para(_)))
                    .map(|(i, _)| i)
                    .collect();
                let cap = kumihan::Paragraph {
                    align: Align::Center,
                    line_spacing: 1.0,
                    runs: vec![kumihan::Run {
                        text: label.clone(),
                        size_pt: SIZE_PT,
                        font: None,
                        fmt: Default::default(),
                    }],
                    ..Default::default()
                };
                self.doc.blocks.insert(para_block_idx[pi] + 1, kumihan::Block::Para(cap));
                self.dirty = true;
                self.relayout();
                self.follow_caret();
                self.status = format!("{label} を入れました(中央揃えの段落)").into();
            }
            // 相互参照。しおり一覧から「文字」「ページ」を挿す板
            "crossref" => {
                self.xr_open = !self.xr_open;
                if self.xr_open {
                    self.bm_open = false;
                    self.find_open = false;
                    self.hf_edit = None;
                    self.cmt_edit = false;
                    self.wm_edit = false;
                    self.status =
                        "相互参照: しおりを選んで「文字」か「ページ」を挿す".into();
                }
            }
            // しおり。一覧の板(名前を打って追加・押して移動・✕で削除)
            "bookmarks" => {
                self.bm_open = !self.bm_open;
                if self.bm_open {
                    self.find_open = false;
                    self.hf_edit = None;
                    self.cmt_edit = false;
                    self.wm_edit = false;
                    self.bm_ed = Editor::new("");
                    self.status =
                        "しおり: 名前を打って「追加」。一覧を押すとそこへ移る".into();
                }
            }
            // 透かし。板で文字を打つ(空にして閉じると外れる)。
            // 文書ではヘッダーの中の VML になり、Word でも斜めの薄い字で出る
            "watermark" => {
                if self.wm_edit {
                    self.wm_edit = false;
                    return;
                }
                if self.doc.header.paragraphs.is_empty() && self.doc.header.part.is_some() {
                    self.status =
                        "このヘッダーには表があり、透かしを差し込めません(この版の制限)".into();
                    return;
                }
                self.find_open = false;
                self.hf_edit = None;
                self.cmt_edit = false;
                self.wm_ed = Editor::new(self.doc.watermark.as_deref().unwrap_or(""));
                self.wm_edit = true;
                self.status = "透かしを編集中(空にして閉じると外れる。Esc で閉じる)".into();
            }
            // ページの色。無し → 薄クリーム → 薄青 → 薄緑 → 無し(文書に入り、
            // 保存で残る。紙(PDF)も同じ色に塗る)
            "pagecolor" => {
                self.doc.page_color = match self.doc.page_color.as_deref() {
                    None => Some("FFF7DC".into()),
                    Some("FFF7DC") => Some("E8F1F8".into()),
                    Some("E8F1F8") => Some("EAF5EE".into()),
                    _ => None,
                };
                self.dirty = true;
                self.status = match &self.doc.page_color {
                    Some(c) => format!("ページの色: #{c}").into(),
                    None => "ページの色: 無し".into(),
                };
            }
            // 行番号(見え方だけ)。折り返した行も1行と数える(見た目の行)
            "line-numbers" => self.line_numbers = !self.line_numbers,
            // 欧文のハイフネーション(入切)。日本語は禁則で折るので変わらない
            "hyphenation" => {
                self.doc.hyphenate = !self.doc.hyphenate;
                self.dirty = true;
                self.relayout_keep();
                self.status = if self.doc.hyphenate {
                    "ハイフネーション: 入(英語の語を音節で折って - を付けます)".into()
                } else {
                    "ハイフネーション: 切".into()
                };
            }
            // コメントの印と一覧の表示(見え方だけ)
            "co-showcomment" => {
                self.show_comments = !self.show_comments;
                self.status = if self.show_comments {
                    "コメントを表示します".into()
                } else {
                    "コメントを隠しました(付いてはいます)".into()
                };
            }
            // カーソルの段落のコメントを外す
            "co-delcomment" => {
                self.switch_target(Target::Body);
                let (pi, _) = self.cursor_para();
                let mut removed = 0usize;
                let mut i = 0usize;
                for b in &mut self.doc.blocks {
                    if let kumihan::Block::Para(p) = b {
                        if i == pi {
                            removed = p.comments.len();
                            p.comments.clear();
                            break;
                        }
                        i += 1;
                    }
                }
                if removed > 0 {
                    self.dirty = true;
                    self.status =
                        format!("この段落のコメントを外しました({removed} 件)").into();
                } else {
                    self.status = "この段落にコメントはありません".into();
                }
            }
            // コメント(段落単位)。カーソルの段落に付ける
            "co-addcomment" | "comment" => {
                if self.cmt_edit {
                    self.cmt_edit = false;
                    return;
                }
                self.switch_target(Target::Body);
                let (pi, _) = self.cursor_para();
                self.cmt_para = pi;
                let text = self
                    .doc
                    .paragraphs()
                    .nth(pi)
                    .and_then(|p| p.comments.first())
                    .map(|c| c.text.clone())
                    .unwrap_or_default();
                self.cmt_ed = Editor::new(&text);
                self.find_open = false;
                self.hf_edit = None;
                self.cmt_edit = true;
                self.status =
                    "コメントを編集中(段落に付きます。空にして閉じると外れる)".into();
            }
            // 文書の保護。readOnly を docx の documentProtection と往復する。
            // パスワードは掛けない(**掛けた振りもしない**)— Word でも
            // 「編集の制限」として見え、解除も同じ1手でできる正直な保護
            "prot-doc" => {
                if self.doc.protection.is_some() {
                    self.doc.protection = None;
                    self.dirty = true;
                    self.status =
                        "保護を外しました(編集できます。保存で docx にも残ります)".into();
                } else {
                    self.flush_target();
                    self.doc.protection = Some("readOnly".into());
                    // 文書を変える板とペンは店じまい
                    self.hf_edit = None;
                    self.wm_edit = false;
                    self.cmt_edit = false;
                    self.tool = None;
                    self.dirty = true;
                    self.status = "読み取り専用で保護しました(同じ釦で解除。\
                                   パスワードは掛けません — 掛けた振りもしません)"
                        .into();
                }
            }
            // 共同編集モード。実体はファイルの錠(.~lock)による早い者勝ちの
            // 編集権。押すと錠の今を確かめ、先客が去っていれば編集権を取り直す
            "coauth-mode" => match self.path.clone() {
                None => {
                    self.status =
                        "まだファイルになっていません(保存すると編集権=錠を取ります)"
                            .into();
                }
                Some(p) => {
                    if self.my_lock.is_some() {
                        self.status = format!(
                            "編集権はこちら({})にあります。同じ文書は先に開いた人が書け、\
                             後の人は読むだけになります(錠は .~lock ファイル)",
                            lock_identity()
                        )
                        .into();
                    } else {
                        self.acquire_lock(&p);
                        self.status = match &self.locked_by {
                            Some(who) => format!(
                                "{who} が編集中です(読めますが上書き保存はできません。\
                                 相手が閉じたら、またこの釦で確かめてください)"
                            )
                            .into(),
                            None => "先客が居なくなっていたので、編集権を取り直しました"
                                .into(),
                        };
                    }
                }
            },
            // バージョン履歴。上書き保存のたびに .jo-history へ残る控えの一覧
            "co-history" => {
                self.hist_open = !self.hist_open;
                if self.hist_open {
                    self.chat_open = false;
                    self.bm_open = false;
                    self.xr_open = false;
                    self.status = if self.path.is_none() {
                        "まだファイルになっていません(保存すると、上書きのたびに\
                         控えが残ります)"
                            .into()
                    } else {
                        "バージョン履歴: 押すと控えを名無しの複製で開きます".into()
                    };
                }
            }
            // チャット。文書の隣の申し送り帳(.chat.txt)へ名乗り付きで追記。
            // サーバーは無いので生放送ではない — ファイル越しの言伝(ことづて)
            "co-chat" => {
                self.chat_open = !self.chat_open;
                if self.chat_open {
                    self.hist_open = false;
                    self.bm_open = false;
                    self.xr_open = false;
                    self.find_open = false;
                    self.chat_ed = Editor::new("");
                    self.status =
                        "チャット: 打って Enter で書き残す(文書の隣の .chat.txt)".into();
                }
            }
            // マクロ。.py を選ぶと檻の中の Python が文書の複製を直す
            "plug-macros" => {
                let ask = cx.background_executor().spawn(async {
                    rfd::FileDialog::new().add_filter("Python", &["py"]).pick_file()
                });
                cx.spawn(async move |this, cx| {
                    let r = ask.await;
                    let _ = this.update(cx, |this, cx| {
                        if let Some(p) = r {
                            this.run_macro_file(p, cx);
                        }
                        cx.notify();
                    });
                })
                .detach();
                self.status = "マクロ: .py を選ぶと、檻の中の Python が文書の複製を\
                               直します(台本の d が python-docx の文書)"
                    .into();
            }
            // プラグインの管理。置き場の .py を一覧し、マクロと同じ檻で実行
            "plug-manage" => {
                self.plug_open = !self.plug_open;
                if self.plug_open {
                    self.hist_open = false;
                    self.chat_open = false;
                    self.bm_open = false;
                    self.xr_open = false;
                    self.status = format!(
                        "プラグイン: {} に .py を置くと、ここに並びます",
                        plugins_dir().display()
                    )
                    .into();
                }
            }
            // 暗号化。パスワードを決めると、保存で ECMA-376 Standard
            // (AES-128)の複合ファイルに包む。空 Enter で解除
            "prot-encrypt" => {
                if self.pw_open {
                    self.pw_open = false;
                    return;
                }
                self.pw_pending = None;
                self.pw_open = true;
                self.pw_ed = Editor::new("");
                self.status = if self.encrypt_pw.is_some() {
                    "暗号化は入っています。新しいパスワードを打って Enter\
                    (空のまま Enter で暗号化をやめる)"
                        .into()
                } else {
                    "暗号化: パスワードを打って Enter(次の保存から効きます)".into()
                };
            }
            // デジタル署名。**隣の .sig への添え書き**(Ed25519)。
            // Word の署名欄には出ない独自方式 — そう言って出す。
            // 有効なら報告だけ、無効・未署名なら(作り直して)署名する
            "prot-sign" => {
                use ed25519_dalek::{Signer as _, Verifier as _};
                let Some(p) = self.path.clone() else {
                    self.status =
                        "まだファイルになっていません(先に保存してください)".into();
                    return;
                };
                if self.dirty {
                    self.status =
                        "未保存の変更があります。保存してから署名してください".into();
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
                        let vk: [u8; 32] =
                            unhex(&field("pubkey:")?)?.try_into().ok()?;
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
                                    "署名しました — 隣の {} に添え書き(独自方式。\
                                     Word の署名欄には出ません。もう一度押すと検めます)",
                                    sp.file_name().unwrap_or_default().to_string_lossy()
                                )
                                .into();
                            }
                            Err(e) => {
                                self.status = format!("署名が置けません: {e}").into()
                            }
                        }
                    }
                    Err(e) => self.status = format!("署名できません: {e}").into(),
                }
            }
            // クリップボード(リボンから。Ctrl+C/X/V と同じ実体)
            "copy" | "cut" => {
                let e = self.editor_ref();
                let sel = e.selection();
                if sel.is_empty() {
                    self.status = "選択がありません".into();
                } else if let Some(t) = e.text().get(sel).map(str::to_string) {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(t));
                    if id == "cut" {
                        self.editor().insert("");
                        self.on_edited();
                        self.status = "切り取りました".into();
                    } else {
                        self.status = "コピーしました".into();
                    }
                }
            }
            "paste" => match cx.read_from_clipboard().and_then(|i| i.text()) {
                Some(text) if !text.is_empty() => handler::replace(self, None, &text),
                _ => self.status = "貼り付けるものがありません".into(),
            },
            "zoom-out" => self.zoom = (self.zoom - 0.1).max(0.5),
            "linespace" => self.para(|p| {
                p.line_spacing = match p.spacing() {
                    s if s < 1.25 => 1.5,
                    s if s < 1.75 => 2.0,
                    _ => 1.0,
                }
            }),
            // 文字カウント。日本語は「単語数」に意味が無いので**文字数**を出す
            "wordcount" => {
                let text = self.ed.text();
                let all = text.chars().filter(|c| *c != '\n').count();
                let ink = text.chars().filter(|c| !c.is_whitespace()).count();
                let paras = text.split('\n').filter(|s| !s.trim().is_empty()).count();
                self.status = format!(
                    "文字数 {ink}(空白込み {all})/ 段落 {paras}").into();
            }
            "fontcolor" => {
                let next = match self.doc.char_format_at(self.ed.selection()).color.as_deref() {
                    None => Some("C00000".to_string()),
                    Some("C00000") => Some("1F4E79".to_string()),
                    _ => None,
                };
                self.toggle(move |f| f.color = next.clone());
            }
            other => {
                // ここに来たら結線漏れ。黙らず画面に出す
                self.status = format!("未配線のコマンド: {other}(不具合です)").into();
            }
        }
    }

    // ---- 割り当てられた操作 ----
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
    /// メニューの項目を実行する。
    fn menu_action(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.menu_at = None;
        match id {
            "cut" => self.cut(&ui::Cut, window, cx),
            "copy" => self.copy(&ui::Copy, window, cx),
            "paste" => self.paste(&ui::Paste, window, cx),
            "selword" => self.select_word(),
            "selline" => self.select_line(),
            "selall" => self.ed.select_all(),
            other => self.run_cmd(other, cx),
        }
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
        // 道具 → メニュー → 検索の板 → ヘッダーの板 → 一覧の板、の順で戻す
        if self.tool.take().is_some() {
            self.ink_cur = None;
            self.status = "文字の編集に戻りました".into();
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
            self.run_cmd("replace", cx); // 検索と置換の板を開く
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
            return; // 板の中では使わない
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
        } else if self.chat_open {
            self.chat_send();
        } else {
            self.editor().insert("\n");
            self.on_edited();
        }
        cx.notify();
    }
    fn copy(&mut self, _: &ui::Copy, _: &mut Window, cx: &mut Context<Self>) {
        // 板(ヘッダー等)を編集中なら、その板の選択が対象
        let e = self.editor_ref();
        let sel = e.selection();
        if sel.is_empty() {
            self.status = "コピーする選択がありません".into();
        } else if let Some(s) = e.text().get(sel) {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(s.to_string()));
            self.status = "コピーしました".into();
        }
        cx.notify();
    }
    fn cut(&mut self, _: &ui::Cut, _: &mut Window, cx: &mut Context<Self>) {
        let sel = self.editor_ref().selection();
        if sel.is_empty() {
            self.status = "切り取る選択がありません".into();
        } else if let Some(s) = self.editor_ref().text().get(sel).map(str::to_string) {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(s));
            // 選択を空文字で置き換える = undo の1手で戻る
            self.editor().insert("");
            self.on_edited();
            self.status = "切り取りました".into();
        }
        cx.notify();
    }
    fn paste(&mut self, _: &ui::Paste, _: &mut Window, cx: &mut Context<Self>) {
        match cx.read_from_clipboard().and_then(|i| i.text()) {
            Some(text) if !text.is_empty() => {
                // 通常の入力と同じ道(IME の未確定があれば確定してから)
                handler::replace(self, None, &text);
            }
            _ => self.status = "貼り付けるものがありません".into(),
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
        // 板(ヘッダー等)を編集中なら、その板の一手を戻す
        if self.editor().undo() {
            self.on_edited();
        } else if let Some(prev) = self.doc_undo.take() {
            // マクロで置き換えた文書を、1手で元へ戻す
            self.target = Target::Body;
            self.pg = prev.page.clone().unwrap_or_default();
            self.set_doc(prev);
            self.relayout_keep();
            self.dirty = true;
            self.status = "マクロの前に戻しました".into();
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
    /// 終了の要求。書きかけが無ければ即終了、あれば確認を**別の糸**で出す。
    /// 確認のダイアログで主の糸を塞がない — 塞ぐと画面ごと固まり、
    /// GNOME に「応答なし」と判定される(calc で踏んで直したのと同じ)。
    fn request_quit(&mut self, cx: &mut Context<Self>) {
        // 確認は**実ファイルの未保存変更**にだけ出す(calc と同じ発注者指示)。
        // 名前も付けていない試し打ちにまで確認を出すと、確認が煩さで押し流される
        if !self.dirty || self.path.is_none() {
            self.release_lock();
            cx.quit();
            return;
        }
        let ask = cx.background_executor().spawn(async move {
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title("writer")
                .set_description("保存していない変更があります。保存して終了しますか?")
                .set_buttons(rfd::MessageButtons::YesNoCancel)
                .show()
        });
        cx.spawn(async move |this, cx| {
            let r = ask.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    // 保存先が未定なら別の糸で選ばせ、済んだときだけ終了する
                    rfd::MessageDialogResult::Yes => this.save(true, cx),
                    rfd::MessageDialogResult::No => {
                        this.release_lock();
                        cx.quit();
                    }
                    _ => this.status = "終了をやめました".into(),
                }
                cx.notify();
            });
        })
        .detach();
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

impl Render for Writer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let me: Entity<Writer> = cx.entity();
        // 画面の倍率(紙のミリは変えず、画素への写像だけ変える)
        let pxmm = PX_PER_MM * self.zoom;
        // 編集領域の高さを実測しておく(キャレット追従・スクロールの止めに使う)。
        // リボンのぶん(約110px)を引いた近似で足りる
        self.view_h_px = (f32::from(window.viewport_size().height) - 136.0).max(100.0);
        let marked = self.ed.marked_range();
        let (cx_mm, cy_mm, caret_pt) = self.caret_xy();

        // ---- リボン(Euro-Office に名前と並びを合わせる) ----
        // **タブの行そのものが窓の取っ手**(掴んで移動・二度押しで最大化)。
        // 空きの帯だけを取っ手にすると、タブが多い窓では幅がゼロになり
        // 掴む場所が無くなる(踏んで直した)。釦の類いは stop_propagation で
        // 取っ手より先に効く
        let (ready, all) = ribbon::progress(ribbon::WRITER);
        // ダークモードは**紙以外**を暗くする — 紙は白いまま(印刷と同じ)。
        // 文書は何も変わらない(見え方だけ)
        let dk = self.dark;
        let th_tab_on_bg = if dk { rgb(0x22262A) } else { rgb(0xFFFFFF) };
        let th_tab_on_fg = if dk { rgb(0xCFE0EA) } else { rgb(0x165E83) };
        let th_cmd_bg = if dk { rgb(0x22262A) } else { rgb(0xFFFFFF) };
        let th_cmd_border = if dk { rgb(0x33383D) } else { rgb(0xE1E6EA) };
        let th_btn = if dk { rgb(0x7FB2D0) } else { rgb(0x165E83) };
        let th_btn_hover = if dk { rgb(0x2C333A) } else { rgb(0xEAF2F7) };
        let th_gray_border = if dk { rgb(0x2E3338) } else { rgb(0xEDEFF1) };
        let th_gray_fg = if dk { rgb(0x565D64) } else { rgb(0xB6BDC4) };
        let th_status = if dk { rgb(0x9AA5AE) } else { rgb(0x66707A) };
        let th_desk = if dk { rgb(0x191C1F) } else { rgb(0x63686D) };
        // デスクトップ版の額縁: 1段目がクイックアクセス+文書名(=取っ手)、
        // 2段目が下線つきのタブ(現在地は青い下線)、3段目が釦の帯
        let th_top_bg = if dk { rgb(0x1B1E21) } else { rgb(0xF1F3F5) };
        let th_top_fg = if dk { rgb(0xCFD6DC) } else { rgb(0x444B52) };
        let th_qa_hover = if dk { rgb(0x2C333A) } else { rgb(0xE2E6EA) };
        let qa = |id: &'static str, icon: &'static str| {
            div().id(id).px_2().py_1().rounded_sm().cursor_pointer()
                .hover(move |s| s.bg(th_qa_hover))
                .child(gpui::svg()
                    .path(SharedString::from(format!("icons/{icon}.svg")))
                    .size(px(15.0)).text_color(th_top_fg))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let title = self
            .path
            .as_ref()
            .and_then(|q| q.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "無題のドキュメント".into());
        let winbtn = |id: &'static str, label: &'static str| {
            div().id(id).px_2p5().py_1().rounded_sm()
                .text_size(px(12.0)).text_color(th_top_fg)
                .cursor_pointer()
                .hover(move |s| if id == "close" { s.bg(rgb(0xC0392B)).text_color(rgb(0xFFFFFF)) }
                                else { s.bg(rgb(0x2C7DA6)).text_color(rgb(0xFFFFFF)) })
                .child(label)
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let top = div().id("titlebar").flex().flex_row().items_center().gap_0p5()
            .px_2().py_0p5().bg(th_top_bg)
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
            .child(div().text_size(px(12.5)).text_color(th_top_fg)
                .whitespace_nowrap().overflow_hidden()
                .child(SharedString::from(format!(
                    "{}{title}",
                    if self.dirty { "*" } else { "" }
                ))))
            .child(div().flex_1())
            .child(div().pr_2().text_size(px(10.5))
                .text_color(if dk { rgb(0x6E7982) } else { rgb(0x8A949D) })
                .child(SharedString::from(format!("writer — 実装済み {ready}/{all}"))))
            .child(winbtn("min", "─").on_click(cx.listener(|_, _, window, _| {
                window.minimize_window();
            })))
            .child(winbtn("max", "▢").on_click(cx.listener(|_, _, window, _| {
                window.zoom_window();
            })))
            .child(winbtn("close", "✕").on_click(cx.listener(|this, _, _, cx| {
                this.request_quit(cx);
            })));

        let th_tab_idle = if dk { rgb(0x9AA5AE) } else { rgb(0x555E66) };
        let mut tabs = div().flex().flex_row().items_end().gap_1()
            .px_2().bg(th_tab_on_bg);
        for (i, tb) in ribbon::WRITER.iter().enumerate() {
            let on = i == self.tab;
            tabs = tabs.child(div()
                .id(SharedString::from(format!("tab{i}")))
                .px_2p5().pt_1p5()
                .text_size(px(12.0))
                .text_color(if on { th_tab_on_fg } else { th_tab_idle })
                .font_weight(if on { gpui::FontWeight::BOLD } else { gpui::FontWeight::NORMAL })
                .cursor_pointer()
                .hover(move |s| s.text_color(th_tab_on_fg))
                .flex().flex_col().items_center().gap_1()
                .child(tb.name)
                // 現在地の青い下線(デスクトップ版の形)
                .child(div().h(px(2.5)).w_full().rounded_sm()
                    .bg(if on { th_btn } else { th_tab_on_bg }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if i == 0 && this.tab != 0 {
                        this.prev_tab = this.tab;
                        this.file_view = 0;
                        this.file_field = None;
                    }
                    this.tab = i;
                    cx.notify()
                })));
        }
        tabs = tabs.child(div().flex_1())
            .child(div().id("tab-find").px_2().pb_1().text_size(px(12.0))
                .text_color(th_tab_idle).cursor_pointer()
                .hover(move |s| s.text_color(th_tab_on_fg))
                .child("🔍")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.run_cmd("replace", cx);
                    cx.notify()
                })));

        let mut cmds = div().flex().flex_col().gap_0p5()
            .px_3().py_1().bg(th_cmd_bg)
            .border_b_1().border_color(th_cmd_border);
        // 本家風のタブ配置。(id, 大釦の名札)。"‖" は群の区切り線。
        // 名札つきは絵の下に短い名前(本家の言い方)、無印は絵だけの釦。
        // 釦の名前は乗ったときに下のステータスバーへ出す(hover_hint)
        type LItem = (&'static str, Option<&'static str>);
        // ホームは2段(発注者の画像 2026-08-04)
        const HOME_ROWS: &[&[LItem]] = &[
            &[
                ("copy", None), ("cut", None), ("‖", None), ("fontname", None),
                ("fontsize", None), ("incfont", None), ("decfont", None),
                ("changecase", None), ("‖", None), ("markers", None),
                ("numbering", None), ("multilevels", None), ("decoffset", None),
                ("incoffset", None), ("linespace", None), ("direction", None),
                ("‖", None), ("parastyle", None),
            ],
            &[
                ("paste", None), ("selectall", None), ("‖", None), ("bold", None),
                ("italic", None), ("underline", None), ("strikeout", None),
                ("superscript", None), ("subscript", None), ("highlight", None),
                ("fontcolor", None), ("clearstyle", None), ("‖", None),
                ("align-left", None), ("align-center", None),
                ("align-right", None), ("align-just", None),
                ("align-dist", None),
                ("hidenchars", None), ("paracolor", None), ("borders", None),
                ("‖", None), ("replace", None),
            ],
        ];
        // 挿入は一段(発注者の画像 2026-08-04)。主要な釦は名札つきの大釦
        const INS_ROWS: &[&[LItem]] = &[&[
            ("blankpage", Some("空白ページ")), ("pagebreak", Some("区切り")),
            ("‖", None), ("instable", Some("表")), ("‖", None),
            ("insimage", Some("画像")), ("insshape", Some("図形")),
            ("inssmartart", None), ("inschart", None), ("smartpicker", None),
            ("‖", None), ("instext", None), ("instextart", None),
            ("dropcap", None), ("text-from-file", None), ("‖", None),
            ("edit-header", None), ("edit-footer", None), ("pagenum", None),
            ("numpages", None), ("datetime", None), ("‖", None),
            ("insequation", None), ("inssymbol", None), ("‖", None),
            ("controls", None),
        ]];
        // 残りのタブも一段(本家 Web 版の並びから起こした。2026-08-04 発注者)
        const DRAW_ROWS: &[&[LItem]] = &[&[
            ("pen", Some("ペン")), ("highlighter", Some("蛍光ペン")),
            ("eraser", Some("消しゴム")),
        ]];
        const LAYOUT_ROWS: &[&[LItem]] = &[&[
            ("pagemargins", Some("余白")), ("pageorient", Some("向き")),
            ("pagesize", Some("サイズ")), ("columns", Some("段組み")),
            ("‖", None), ("line-numbers", None), ("hyphenation", None),
            ("‖", None), ("watermark", None), ("pagecolor", None),
            ("‖", None), ("colorschemas", None),
        ]];
        const REF_ROWS: &[&[LItem]] = &[&[
            ("toc", Some("目次")), ("toc-update", None), ("add-text", None),
            ("‖", None), ("bookmarks", None), ("caption", None),
            ("crossref", None), ("‖", None), ("tof", None), ("tof-update", None),
        ]];
        const FORM_ROWS: &[&[LItem]] = &[&[
            ("form-text", None), ("form-combo", None), ("form-dropdown", None),
            ("form-checkbox", None), ("form-radio", None), ("form-image", None),
            ("form-email", None), ("form-phone", None), ("form-complex", None),
            ("form-signature", None),
        ]];
        const COLLAB_ROWS: &[&[LItem]] = &[&[
            ("coauth-mode", Some("共同編集モード")), ("‖", None),
            ("co-addcomment", Some("コメント")), ("co-delcomment", None),
            ("co-showcomment", None), ("‖", None), ("co-chat", Some("チャット")),
            ("‖", None), ("track-changes", Some("変更履歴")), ("‖", None),
            ("co-history", Some("バージョン履歴")),
        ]];
        const PROT_ROWS: &[&[LItem]] = &[&[
            ("prot-encrypt", Some("暗号化")), ("prot-sign", Some("署名")),
            ("prot-doc", Some("保護")),
        ]];
        const VIEW_ROWS: &[&[LItem]] = &[&[
            ("zoom-in", Some("拡大")), ("zoom-out", Some("縮小")), ("‖", None),
            ("ruler", None), ("darkmode", None),
        ]];
        const PLUG_ROWS: &[&[LItem]] = &[&[
            ("plug-macros", Some("マクロ")),
            ("plug-manage", Some("プラグインの管理")),
        ]];
        let rows: Option<&[&[LItem]]> = match ribbon::WRITER[self.tab].name {
            "ホーム" => Some(HOME_ROWS),
            "挿入" => Some(INS_ROWS),
            "描画" => Some(DRAW_ROWS),
            "レイアウト" => Some(LAYOUT_ROWS),
            "参考資料" => Some(REF_ROWS),
            "フォーム" => Some(FORM_ROWS),
            "共同編集" => Some(COLLAB_ROWS),
            "保護" => Some(PROT_ROWS),
            "表示" => Some(VIEW_ROWS),
            "プラグイン" => Some(PLUG_ROWS),
            _ => None,
        };
        if let Some(rows) = rows {
            let size_now = self.doc.size_at(self.ed.selection()).unwrap_or(SIZE_PT);
            let size_disp = if size_now.fract() == 0.0 {
                format!("{}", size_now as i32)
            } else {
                format!("{size_now}")
            };
            for ids in rows {
                let tall = ids.iter().any(|(_, b)| b.is_some());
                let mut row = div().flex().flex_row().items_center().gap_0p5();
                for &(id, big) in *ids {
                    if id == "‖" {
                        row = row.child(div().w(px(1.0))
                            .h(px(if tall { 40.0 } else { 22.0 }))
                            .bg(th_cmd_border).mx_1());
                        continue;
                    }
                    // コンボ風(フォント名と大きさは今の値を見せる)
                    if id == "fontname" || id == "fontsize" {
                        let cid = id;
                        let text = if cid == "fontname" {
                            self.font_name.to_string()
                        } else {
                            size_disp.clone()
                        };
                        row = row.child(div()
                            .id(SharedString::from(format!("h-{cid}")))
                            .flex().flex_row().items_center().gap_1()
                            .px_2().h(px(26.0))
                            .w(px(if cid == "fontname" { 150.0 } else { 56.0 }))
                            .rounded_sm().border_1().border_color(th_cmd_border)
                            .text_size(px(12.0)).text_color(th_top_fg)
                            .cursor_pointer()
                            .hover(move |st| st.bg(th_btn_hover))
                            .child(div().flex_1().whitespace_nowrap()
                                .overflow_hidden().child(SharedString::from(text)))
                            .child(div().text_size(px(9.0)).text_color(th_tab_idle)
                                .child("▼"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.run_cmd(cid, cx);
                                cx.notify()
                            })));
                        continue;
                    }
                    let Some(cmd) = ribbon::WRITER[self.tab]
                        .cmds
                        .iter()
                        .find(|c| c.id == id || (!c.ready && c.icon == id))
                        .copied()
                    else {
                        continue;
                    };
                    let label = cmd.label;
                    let icon = cmd.icon;
                    let hoverable = cx.listener(move |this: &mut Writer, on: &bool, _, cx| {
                        if *on {
                            this.hover_hint = Some(label);
                        } else if this.hover_hint == Some(label) {
                            this.hover_hint = None;
                        }
                        cx.notify()
                    });
                    let has_icon = ui::icons::find(icon).is_some();
                    if let Some(short) = big {
                        // 名札つきの大釦(絵の下に短い名前。本家の言い方)
                        let fg = if cmd.ready { th_top_fg } else { th_gray_fg };
                        let mut b = div()
                            .id(SharedString::from(format!("h-{icon}")))
                            .px_2().h(px(48.0)).rounded_sm()
                            .flex().flex_col().items_center().justify_center()
                            .gap_1()
                            .on_hover(hoverable)
                            .children(has_icon.then(|| {
                                gpui::svg()
                                    .path(SharedString::from(format!("icons/{icon}.svg")))
                                    .size(px(20.0))
                                    .text_color(fg)
                            }))
                            .child(div().text_size(px(10.5)).text_color(fg)
                                .child(short));
                        if cmd.ready {
                            let cid = cmd.id;
                            b = b.cursor_pointer()
                                .hover(move |st| st.bg(th_btn_hover))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.run_cmd(cid, cx);
                                    cx.notify()
                                }));
                        }
                        row = row.child(b);
                        continue;
                    }
                    let mut b = div()
                        .id(SharedString::from(format!("h-{icon}")))
                        .h(px(26.0)).rounded_sm()
                        .flex().items_center().justify_center()
                        .on_hover(hoverable);
                    b = if has_icon { b.w(px(26.0)) } else { b.px_1p5() };
                    if cmd.ready {
                        let cid = cmd.id;
                        b = b.cursor_pointer()
                            .hover(move |st| st.bg(th_btn_hover))
                            .children(has_icon.then(|| {
                                gpui::svg()
                                    .path(SharedString::from(format!("icons/{icon}.svg")))
                                    .size(px(18.0))
                                    .text_color(th_top_fg)
                            }))
                            .children((!has_icon).then(|| {
                                div().text_size(px(10.5)).text_color(th_btn)
                                    .child(label)
                            }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.run_cmd(cid, cx);
                                cx.notify()
                            }));
                    } else {
                        // 未実装。押せるように見せない
                        b = b.children(has_icon.then(|| {
                            gpui::svg()
                                .path(SharedString::from(format!("icons/{icon}.svg")))
                                .size(px(18.0))
                                .text_color(th_gray_fg)
                        }))
                        .children((!has_icon).then(|| {
                            div().text_size(px(10.5)).text_color(th_gray_fg)
                                .child(label)
                        }));
                    }
                    row = row.child(b);
                }
                cmds = cmds.child(row);
            }
        } else {
            let mut row = div().flex().flex_row().flex_wrap().gap_1().items_center().py_1();
            for cmd in ribbon::WRITER[self.tab].cmds {
                if cmd.ready {
                    let id = cmd.id;
                    row = row.child(div()
                        .id(SharedString::from(cmd.id))
                        .px_3().py_1().rounded_md()
                        .border_1().border_color(th_btn).text_color(th_btn)
                        .text_size(px(12.0)).cursor_pointer()
                        .hover(move |s| s.bg(th_btn_hover))
                        .flex().flex_row().items_center().gap_1()
                        .children(ui::icons::find(cmd.icon).map(|_| {
                            gpui::svg()
                                .path(SharedString::from(format!("icons/{}.svg", cmd.icon)))
                                .size(px(15.0))
                                .text_color(th_btn)
                        }))
                        .child(cmd.label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.run_cmd(id, cx); cx.notify()
                        })));
                } else {
                    // 未実装。押せるように見せない
                    row = row.child(div().px_3().py_1().rounded_md()
                        .border_1().border_color(th_gray_border)
                        .text_color(th_gray_fg).text_size(px(12.0))
                        .flex().flex_row().items_center().gap_1()
                        .children(ui::icons::find(cmd.icon).map(|_| {
                            gpui::svg()
                                .path(SharedString::from(format!("icons/{}.svg", cmd.icon)))
                                .size(px(15.0))
                                .text_color(th_gray_fg)
                        }))
                        .child(cmd.label));
                }
            }
            cmds = cmds.child(row);
        }
        let bar = if self.tab == 0 {
            // ファイルのページ(本家の File メニュー)は釦の帯を持たない
            div().flex().flex_col().child(top).child(tabs)
        } else {
            div().flex().flex_col().child(top).child(tabs).child(cmds)
        };

        // ---- 下のステータスバー(デスクトップ版: ページ・文字数・ズーム) ----
        let total_pages = self.page_offsets.len().max(1);
        let cur_page = self
            .page_offsets
            .iter()
            .rposition(|o| self.scroll_mm >= *o - 0.01)
            .unwrap_or(0)
            + 1;
        let nchars = self
            .doc
            .body_text()
            .chars()
            .filter(|c| !c.is_whitespace())
            .count();
        let sb_btn = |id: &'static str, label: &'static str| {
            div().id(id).px_1p5().py_0p5().rounded_sm().cursor_pointer()
                .text_size(px(11.5)).text_color(th_top_fg)
                .hover(move |s| s.bg(th_qa_hover))
                .child(label)
        };
        let statusbar = div().flex().flex_row().items_center().gap_3()
            .px_3().py_0p5().bg(th_top_bg)
            .border_t_1().border_color(th_cmd_border)
            .text_size(px(11.0)).text_color(th_status)
            .child(SharedString::from(format!("{cur_page}/{total_pages} ページ")))
            .child(SharedString::from(format!("文字数 {nchars}")))
            .child(div().flex_1().whitespace_nowrap().overflow_hidden()
                .child(SharedString::from(match self.hover_hint {
                    Some(h) => h.to_string(),
                    None => format!(
                        "{}{}",
                        if self.dirty { "● " } else { "" },
                        self.status
                    ),
                })))
            .child(sb_btn("sb-spell", "スペル").on_click(cx.listener(|this, _, _, cx| {
                this.run_cmd("spell", cx);
                cx.notify()
            })))
            .child(sb_btn("sb-zoom-out", "−").on_click(cx.listener(|this, _, _, cx| {
                this.run_cmd("zoom-out", cx);
                cx.notify()
            })))
            .child(div().id("sb-zoom").px_1().rounded_sm().cursor_pointer()
                .text_size(px(11.5)).text_color(th_top_fg)
                .hover(move |s| s.bg(th_qa_hover))
                .child(SharedString::from(format!(
                    "ズーム{}%",
                    (self.zoom * 100.0).round() as i32
                )))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.zoom = 1.0;
                    cx.notify()
                })))
            .child(sb_btn("sb-zoom-in", "＋").on_click(cx.listener(|this, _, _, cx| {
                this.run_cmd("zoom-in", cx);
                cx.notify()
            })));

        // ---- ファイルのページ(本家の File メニュー。タブ0で全面に出す) ----
        let filepage: Option<gpui::Div> = if self.tab != 0 {
            None
        } else {
            let item_bg = th_qa_hover;
            let mk = |id: &'static str, label: &'static str, ready: bool| {
                let d = div().id(id).px_4().py_1p5().text_size(px(13.0));
                if ready {
                    d.text_color(th_top_fg)
                        .cursor_pointer()
                        .hover(move |s| s.bg(item_bg))
                } else {
                    d.text_color(th_gray_fg)
                }
                .child(label)
            };
            let sb = div().w(px(280.0)).bg(th_top_bg)
                .border_r_1().border_color(th_cmd_border)
                .flex().flex_col().py_2()
                .child(mk("f-back", "‹ 戻る", true).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.tab = this.prev_tab;
                        cx.notify()
                    })))
                .child(div().h(px(10.0)))
                .child(mk("f-new", "新規作成", true).on_click(cx.listener(
                    |this, _, _, cx| {
                        if this.new_doc() {
                            this.tab = this.prev_tab;
                        }
                        cx.notify()
                    })))
                .child(mk("f-tpl", "テンプレートから作成", false))
                .child(mk("f-open", "開く", true).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.tab = this.prev_tab;
                        this.open_dialog(cx);
                        cx.notify()
                    })))
                .child(mk("f-recent", "最近開いた", true).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.file_view = 1;
                        cx.notify()
                    })))
                .child(div().h(px(10.0)))
                .child(mk("f-save", "保存", true).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.save(false, cx);
                        cx.notify()
                    })))
                .child(mk("f-saveas", "名前を付けて保存", true).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.save_as(cx);
                        cx.notify()
                    })))
                .child(mk("f-print", "印刷", true).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.save_pdf(cx);
                        cx.notify()
                    })))
                .child(mk("f-protect", "保護する", true).on_click(cx.listener(
                    |this, _, _, cx| {
                        if let Some(i) =
                            ribbon::WRITER.iter().position(|t| t.name == "保護")
                        {
                            this.tab = i;
                        }
                        cx.notify()
                    })))
                .child(div().h(px(10.0)))
                .child({
                    let d = mk("f-info", "詳細情報", true).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.file_view = 0;
                            cx.notify()
                        }));
                    if self.file_view == 0 { d.bg(item_bg) } else { d }
                })
                .child(mk("f-place", "ファイルの場所を開く", true).on_click(cx.listener(
                    |this, _, _, cx| {
                        match this.path.as_ref().and_then(|p| p.parent()) {
                            Some(dir) => {
                                let _ = std::process::Command::new("xdg-open")
                                    .arg(dir)
                                    .spawn();
                            }
                            None => {
                                this.status = "まだファイルになっていません".into();
                            }
                        }
                        cx.notify()
                    })))
                .child(div().h(px(10.0)))
                .child(mk("f-quit", "終了", true).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.request_quit(cx);
                        cx.notify()
                    })))
                .child(div().flex_1())
                .child(mk("f-opts", "詳細設定", false))
                .child(mk("f-help", "ヘルプ", false))
                .child(mk("f-req", "機能のリクエスト", false));

            let mut pane = div().flex_1().bg(th_cmd_bg).p_8()
                .flex().flex_col().gap_3().text_size(px(12.5))
                .text_color(th_top_fg);
            if self.file_view == 1 {
                pane = pane.child(div().text_size(px(16.0))
                    .font_weight(gpui::FontWeight::BOLD)
                    .child("最近開いた"));
                let list = Self::recent_list();
                if list.is_empty() {
                    pane = pane.child(div().text_color(th_status)
                        .child("(まだありません。開く・保存すると残ります)"));
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
                        .child(div().text_size(px(13.0))
                            .child(SharedString::from(name)))
                        .child(div().text_size(px(11.0)).text_color(th_status)
                            .child(SharedString::from(dir)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.tab = this.prev_tab;
                            this.open(q.clone());
                            cx.notify()
                        })));
                }
            } else {
                let text = self.doc.body_text();
                let words = text.split_whitespace().count();
                let chars_all = text.chars().filter(|c| *c != '\n').count();
                let paras = self.doc.paragraphs().count();
                pane = pane.child(div().text_size(px(16.0))
                    .font_weight(gpui::FontWeight::BOLD)
                    .child("文書の情報"))
                    .child(div().text_size(px(13.5))
                        .font_weight(gpui::FontWeight::BOLD)
                        .child("統計"));
                for (k, v) in [
                    ("ページ", total_pages),
                    ("段落", paras),
                    ("単語", words),
                    ("文字数", nchars),
                    ("文字数 (スペースを含む)", chars_all),
                ] {
                    pane = pane.child(div().flex().flex_row()
                        .child(div().w(px(220.0)).text_color(th_status).child(k))
                        .child(SharedString::from(format!("{v}"))));
                }
                pane = pane.child(div().h(px(6.0)))
                    .child(div().text_size(px(13.5))
                        .font_weight(gpui::FontWeight::BOLD)
                        .child("プロパティ"));
                let pr = self.doc.props.clone();
                let vals: [(&'static str, String, &'static str); 5] = [
                    ("作成者", pr.creator, "著者を追加"),
                    ("タイトル", pr.title, "テキストの追加"),
                    ("タグ", pr.keywords, "テキストの追加"),
                    ("件名", pr.subject, "テキストの追加"),
                    ("コメント", pr.description, "テキストの追加"),
                ];
                for (i, (k, v, ph)) in vals.into_iter().enumerate() {
                    let editing = self.file_field == Some(i as u8);
                    let shown = if editing {
                        let mut t = self.prop_ed.text().to_string();
                        let cur = self.prop_ed.cursor().min(t.len());
                        t.insert(cur, '|');
                        t
                    } else {
                        v.clone()
                    };
                    let empty = !editing && v.is_empty();
                    pane = pane.child(div().flex().flex_row().items_center()
                        .child(div().w(px(220.0)).text_color(th_status).child(k))
                        .child(div()
                            .id(SharedString::from(format!("prop-{i}")))
                            .w(px(320.0)).px_2().py_1().rounded_sm()
                            .border_1()
                            .border_color(if editing {
                                rgb(0x1B6E3C)
                            } else {
                                th_cmd_border
                            })
                            .cursor_pointer()
                            .whitespace_nowrap().overflow_hidden()
                            .text_color(if empty { th_gray_fg } else { th_top_fg })
                            .child(SharedString::from(if empty {
                                ph.to_string()
                            } else {
                                shown
                            }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let cur = match i {
                                    0 => this.doc.props.creator.clone(),
                                    1 => this.doc.props.title.clone(),
                                    2 => this.doc.props.keywords.clone(),
                                    3 => this.doc.props.subject.clone(),
                                    _ => this.doc.props.description.clone(),
                                };
                                this.prop_ed = Editor::new(&cur);
                                this.file_field = Some(i as u8);
                                cx.notify()
                            }))));
                }
                pane = pane.child(div().text_size(px(11.5)).text_color(th_status)
                    .child("欄を押して打ち、Enter で控える(保存で docx の情報に入ります)"));
            }
            Some(div().flex_1().relative().overflow_hidden()
                .child(div().absolute().inset_0().flex().flex_row()
                    .child(sb)
                    .child(pane))
                .child(InputSink { view: me.clone() }))
        };

        // 紙。スクロールは紙ごと上へずらすだけ(中身は全部この容器の子)。
        // ページの色は文書の設定(紙も同じ色に塗られる)
        let paper_bg = match self.doc.page_color.as_deref() {
            Some(c) => gpui::Rgba { r: hex(c, 0), g: hex(c, 1), b: hex(c, 2), a: 1.0 },
            None => gpui::Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
        };
        let mut paper = div().absolute()
            .left(px(28.0)).top(px(14.0 - self.scroll_mm * pxmm))
            .w(px(self.pg.w_mm * pxmm)).h(px(self.content_mm() * pxmm))
            .bg(paper_bg).shadow_lg();

        // ルーラー(10mm ごとの目盛り。余白の位置が分かる)
        if self.ruler {
            let mut n = 0;
            loop {
                let mm = n as f32 * 10.0;
                if mm > self.pg.w_mm {
                    break;
                }
                let major = n % 5 == 0;
                paper = paper.child(div().absolute()
                    .left(px(mm * pxmm)).top(px(0.0))
                    .w(px(1.0)).h(px(if major { 10.0 } else { 5.0 }))
                    .bg(rgb(0xAABBC6)));
                if major && n > 0 {
                    paper = paper.child(div().absolute()
                        .left(px(mm * pxmm + 2.0)).top(px(0.0))
                        .text_size(px(8.5)).text_color(rgb(0x8899A6))
                        .child(SharedString::from(format!("{}", mm as u32))));
                }
                n += 1;
            }
            // 余白の線(本文の左右端)
            for x in [self.pg.left_mm, self.pg.w_mm - self.pg.right_mm] {
                paper = paper.child(div().absolute()
                    .left(px(x * pxmm)).top(px(0.0))
                    .w(px(1.0)).h(px(14.0)).bg(rgb(0x1B6E3C)));
            }
        }

        // 画像。組版が置いた位置に、そのまま出す
        for (i, (bytes, [x, top, w_mm, h_mm])) in self.page.images.iter().enumerate() {
            let src = self.image_cache.entry(std::sync::Arc::as_ptr(bytes) as usize)
                .or_insert_with(|| {
                    let format = match bytes.get(..4) {
                        Some([0x89, b'P', b'N', b'G']) => gpui::ImageFormat::Png,
                        Some([0xFF, 0xD8, ..]) => gpui::ImageFormat::Jpeg,
                        _ => gpui::ImageFormat::Png,
                    };
                    std::sync::Arc::new(gpui::Image::from_bytes(format, bytes.to_vec()))
                })
                .clone();
            let _ = i;
            paper = paper.child(
                gpui::img(src)
                    .absolute()
                    .left(px((self.pg.left_mm + x) * pxmm))
                    .top(px(top * pxmm))
                    .w(px(w_mm * pxmm))
                    .h(px(h_mm * pxmm)),
            );
        }

        // 表の罫線。紙面の座標をそのまま引く
        for r in &self.page.rules {
            let [x1, y1, x2, y2] = *r;
            let (x1, y1) = ((self.pg.left_mm + x1) * pxmm, y1 * pxmm);
            let (x2, y2) = ((self.pg.left_mm + x2) * pxmm, y2 * pxmm);
            paper = paper.child(div().absolute()
                .left(px(x1.min(x2))).top(px(y1.min(y2)))
                .w(px((x2 - x1).abs().max(1.0))).h(px((y2 - y1).abs().max(1.0)))
                .bg(rgb(0x444B52)));
        }

        // 段落の背景色と囲み枠。行の帯として敷く(文字より下に来るよう先に描く)
        {
            let mut deco: Vec<(std::ops::Range<usize>, Option<String>, bool)> = Vec::new();
            let mut at = 0usize;
            for p in self.doc.paragraphs() {
                let len: usize = p.runs.iter().map(|r| r.text.len()).sum();
                if p.shade.is_some() || p.boxed {
                    deco.push((at..at + len, p.shade.clone(), p.boxed));
                }
                at += len + 1;
            }
            if !deco.is_empty() {
                let (bx0, bx1) = (self.pg.left_mm, self.pg.w_mm - self.pg.right_mm);
                for line in self.page.lines.iter().filter(|l| l.from_body) {
                    let Some((r, shade, boxed)) = deco
                        .iter()
                        .find(|(r, ..)| r.start <= line.byte0 && line.byte0 <= r.end)
                        .map(|(r, sh, b)| (r.clone(), sh.clone(), *b))
                    else {
                        continue;
                    };
                    let band_top = (line.y_mm - LINE_MM * 0.75) * pxmm;
                    let band_h = LINE_MM * pxmm;
                    if let Some(c) = &shade {
                        paper = paper.child(div().absolute()
                            .left(px(bx0 * pxmm)).top(px(band_top))
                            .w(px((bx1 - bx0) * pxmm)).h(px(band_h))
                            .bg(gpui::Rgba {
                                r: hex(c, 0), g: hex(c, 1), b: hex(c, 2), a: 1.0,
                            }));
                    }
                    if boxed {
                        let ink = rgb(0x444B52);
                        for x in [bx0, bx1] {
                            paper = paper.child(div().absolute()
                                .left(px(x * pxmm)).top(px(band_top))
                                .w(px(1.0)).h(px(band_h)).bg(ink));
                        }
                        if line.byte0 == r.start {
                            paper = paper.child(div().absolute()
                                .left(px(bx0 * pxmm)).top(px(band_top))
                                .w(px((bx1 - bx0) * pxmm)).h(px(1.0)).bg(ink));
                        }
                        if line.byte_end() >= r.end {
                            paper = paper.child(div().absolute()
                                .left(px(bx0 * pxmm)).top(px(band_top + band_h))
                                .w(px((bx1 - bx0) * pxmm)).h(px(1.0)).bg(ink));
                        }
                    }
                }
            }
        }

        // ページの境の薄い線(積み上げたページの切れ目が分かるように)
        {
            let mut pno = 1;
            loop {
                let y = pno as f32 * self.pg.h_mm;
                if y >= self.content_mm() {
                    break;
                }
                paper = paper.child(div().absolute()
                    .left(px(0.0)).top(px(y * pxmm))
                    .w(px(self.pg.w_mm * pxmm)).h(px(1.0))
                    .bg(rgb(0xD5DBE0)));
                pno += 1;
            }
        }

        // 透かし。1字ずつ対角線に沿って置く(画面の近似。紙は回転した字)
        if let Some(text) = self.doc.watermark.as_deref().filter(|t| !t.is_empty()) {
            let n = text.chars().count().max(1) as f32;
            let wpt = (520.0 / n).clamp(36.0, 120.0);
            let em_mm = wpt * 25.4 / 72.0;
            let k = std::f32::consts::FRAC_1_SQRT_2;
            let (cx0, cy0) = (self.pg.w_mm / 2.0, self.pg.h_mm / 2.0);
            for (i, ch) in text.chars().enumerate() {
                let t = (i as f32 - (n - 1.0) / 2.0) * em_mm;
                let x = cx0 + t * k - em_mm / 2.0;
                let y = cy0 - t * k - em_mm / 2.0;
                paper = paper.child(div().absolute()
                    .left(px(x * pxmm)).top(px(y * pxmm))
                    .text_size(px(wpt * 96.0 / 72.0 * self.zoom))
                    .font_family(self.font_name.clone())
                    .text_color(gpui::Rgba { r: 0.62, g: 0.62, b: 0.62, a: 0.5 })
                    .child(SharedString::from(ch.to_string())));
            }
        }

        // 変更履歴の記録中: 変わった段落の左に橙の棒(Word の変更バー)
        if self.track {
            if let Some(base) = &self.track_base {
                let base_set: std::collections::HashSet<&str> =
                    base.iter().map(|s| s.as_str()).collect();
                let mut starts: Vec<(usize, bool)> = Vec::new();
                let mut at = 0usize;
                for p in self.doc.paragraphs() {
                    let t = para_text(p);
                    starts.push((at, !base_set.contains(t.as_str())));
                    at += t.len() + 1;
                }
                for line in self.page.lines.iter().filter(|l| l.from_body) {
                    let changed = starts
                        .iter()
                        .rev()
                        .find(|(b, _)| *b <= line.byte0)
                        .map(|(_, c)| *c)
                        .unwrap_or(false);
                    if changed {
                        paper = paper.child(div().absolute()
                            .left(px((self.pg.left_mm - 5.0).max(0.5) * pxmm))
                            .top(px((line.y_mm - LINE_MM * 0.7) * pxmm))
                            .w(px(2.0)).h(px(LINE_MM * pxmm))
                            .bg(rgb(0xE08A00)));
                    }
                }
            }
        }

        // コメントの印。付いた段落の1行目の右余白にオレンジの角を出す
        if self.show_comments {
            let mut at = 0usize;
            let mut heads: Vec<usize> = Vec::new(); // コメント付き段落の頭のバイト
            for p in self.doc.paragraphs() {
                let len: usize = p.runs.iter().map(|r| r.text.len()).sum();
                if !p.comments.is_empty() {
                    heads.push(at);
                }
                at += len + 1;
            }
            for s0 in heads {
                if let Some(line) = self.page.lines.iter()
                    .filter(|l| l.from_body)
                    .find(|l| l.byte0 == s0)
                {
                    paper = paper.child(div().absolute()
                        .left(px((self.pg.w_mm - self.pg.right_mm + 2.0) * pxmm))
                        .top(px(line.y_mm * pxmm - 8.0))
                        .w(px(6.0)).h(px(6.0)).rounded_sm()
                        .bg(rgb(0xE08A00)));
                }
            }
        }

        // 行番号。本文の(見た目の)行を数え、左の余白に出す
        if self.line_numbers {
            let mut n = 0usize;
            for line in self.page.lines.iter().filter(|l| l.from_body) {
                n += 1;
                paper = paper.child(div().absolute()
                    .left(px((self.pg.left_mm - 9.0).max(1.0) * pxmm))
                    .top(px(line.y_mm * pxmm - 8.5 * self.zoom))
                    .text_size(px(8.5 * self.zoom))
                    .text_color(rgb(0x9DB8C8))
                    .child(SharedString::from(n.to_string())));
            }
        }

        // 未確定(変換中)の下線は、行が持つバイト位置(byte0)で結ぶ
        for line in &self.page.lines {
            if line.cells.is_empty() {
                continue;
            }
            let pt = line.cells[0].size_pt;
            let sz = pt * 96.0 / 72.0 * self.zoom;
            let x0 = self.pg.left_mm + line.cells[0].x_mm;
            let top = line.y_mm * pxmm - sz * 0.88;

            if let Some(m) = &marked {
                let mine = match self.target {
                    Target::Body => line.from_body,
                    Target::Cell { table, row, col } => line.cell == Some((table, row, col)),
                };
                if !mine {
                    // 編集していない行に変換下線は出さない
                } else {
                let (ls, le) = (line.byte0, line.byte_end());
                if m.start < le && m.end > ls {
                    let a = m.start.max(ls) - ls;
                    let b = m.end.min(le) - ls;
                    let base = line.cells.iter().map(|c| c.off).min().unwrap_or(0);
                    // 幅は x 位置から出す(均等割付で字間が広がってもずれない)
                    let xr = |upto: usize| -> f32 {
                        line.cells.iter()
                            .find(|c| c.off - base >= upto)
                            .map(|c| c.x_mm)
                            .or_else(|| line.cells.last().map(|c| c.x_mm + c.w_mm))
                            .unwrap_or(0.0)
                            - line.cells[0].x_mm
                    };
                    paper = paper.child(div().absolute()
                        .left(px((x0 + xr(a)) * pxmm))
                        .top(px(top + sz * (1.05 + HALF_LEADING)))
                        .w(px((xr(b) - xr(a)).max(1.0) * pxmm))
                        .h(px(2.0)).bg(rgb(0x165E83)));
                }
                }
            }
            // 選択の色。**選択が見えないと、コピーも切り取りも信用できない**
            // (ドラッグで選べるようにしても、色が出なければ「できない」に見える)
            let selr = self.ed.selection();
            if !selr.is_empty() {
                let mine = match self.target {
                    Target::Body => line.from_body,
                    Target::Cell { table, row, col } => line.cell == Some((table, row, col)),
                };
                let (ls, le) = (line.byte0, line.byte_end());
                if mine && selr.start < le && selr.end > ls {
                    let a = selr.start.max(ls) - ls;
                    let b = selr.end.min(le) - ls;
                    let base = line.cells.iter().map(|c| c.off).min().unwrap_or(0);
                    let xr = |upto: usize| -> f32 {
                        line.cells.iter()
                            .find(|c| c.off - base >= upto)
                            .map(|c| c.x_mm)
                            .or_else(|| line.cells.last().map(|c| c.x_mm + c.w_mm))
                            .unwrap_or(0.0)
                            - line.cells[0].x_mm
                    };
                    paper = paper.child(div().absolute()
                        .left(px((x0 + xr(a)) * pxmm))
                        .top(px(top + sz * HALF_LEADING))
                        .w(px((xr(b) - xr(a)).max(1.5) * pxmm))
                        .h(px(sz * 1.2))
                        // 半透明の青。文字より下・蛍光ペンより上に敷く
                        .bg(gpui::Rgba { r: 0.40, g: 0.60, b: 0.85, a: 0.35 }));
                }
            }
            // 文字は**同じ書式の連なり**ごとに描く(部分書式。太字・大きさ・
            // 書体・色が行の中で混ざっても、その通りに出る)
            let mut i = 0usize;
            while i < line.cells.len() {
                let c0 = &line.cells[i];
                let mut j = i + 1;
                while j < line.cells.len()
                    && line.cells[j].fmt == c0.fmt
                    && line.cells[j].size_pt == c0.size_pt
                    && line.cells[j].font == c0.font
                    // 字間が広げられた行(均等割付)は1本で描けない —
                    // x が飛んだら連なりを切る
                    && (line.cells[j].x_mm
                        - line.cells[j - 1].x_mm
                        - line.cells[j - 1].w_mm)
                        .abs()
                        < 0.05
                {
                    j += 1;
                }
                let seg = &line.cells[i..j];
                let text: String = seg.iter().map(|c| c.ch).collect();
                let w_mm: f32 = seg.iter().map(|c| c.w_mm).sum();
                let f = &c0.fmt;
                let sx = self.pg.left_mm + c0.x_mm;
                let spt = c0.size_pt * 96.0 / 72.0 * self.zoom;
                let stop = line.y_mm * pxmm - spt * 0.88;
                // 上付き・下付きは小さく描き、少し上下へずらす
                let (spt, stop) = if f.superscript {
                    (spt * 0.7, stop - spt * 0.25)
                } else if f.subscript {
                    (spt * 0.7, stop + spt * 0.25)
                } else {
                    (spt, stop)
                };
                // 参照(フィールド)はうっすら網掛け(Word の作法)。
                // 「ここは計算された値」と分かるように
                if f.field.is_some() {
                    paper = paper.child(div().absolute()
                        .left(px(sx * pxmm)).top(px(stop + spt * HALF_LEADING))
                        .w(px(w_mm * pxmm)).h(px(spt * 1.15))
                        .bg(gpui::Rgba { r: 0.55, g: 0.6, b: 0.65, a: 0.16 }));
                }
                // 蛍光ペン。字の下に色を敷く
                if let Some(h) = &f.highlight {
                    let bg = match h.as_str() {
                        "green" => rgb(0xC9F0C9),
                        "cyan" => rgb(0xC9EEF0),
                        _ => rgb(0xF7EFA8),
                    };
                    paper = paper.child(div().absolute()
                        .left(px(sx * pxmm)).top(px(stop + spt * HALF_LEADING))
                        .w(px(w_mm * pxmm)).h(px(spt * 1.15))
                        .bg(bg));
                }
                let mut d = div().absolute()
                    .left(px(sx * pxmm)).top(px(stop))
                    .text_size(px(spt))
                    .font_family(c0.font.clone().map(SharedString::from)
                        .unwrap_or_else(|| self.font_name.clone()))
                    .whitespace_nowrap()
                    .child(SharedString::from(text));
                if f.bold {
                    d = d.font_weight(gpui::FontWeight::BOLD);
                }
                if f.italic {
                    d = d.italic();
                }
                d = match &f.color {
                    Some(c) => d.text_color(gpui::Rgba {
                        r: hex(c, 0), g: hex(c, 1), b: hex(c, 2), a: 1.0,
                    }),
                    None => d.text_color(rgb(0x1B1B1B)),
                };
                paper = paper.child(d);
                // 下線・取り消し線は連なりごとに引く(gpui の text に無い)
                for (on, dy) in [
                    (f.underline, spt * (1.05 + HALF_LEADING)),
                    (f.strike, spt * (0.35 + HALF_LEADING)),
                ] {
                    if on {
                        paper = paper.child(div().absolute()
                            .left(px(sx * pxmm)).top(px(stop + dy))
                            .w(px(w_mm * pxmm)).h(px(1.0))
                            .bg(rgb(0x1B1B1B)));
                    }
                }
                i = j;
            }
            // 編集記号。空白は・、段落の終わりは ↵(見え方だけ。文書は変わらない)
            if self.show_marks && line.from_body {
                for c in &line.cells {
                    if c.ch == ' ' || c.ch == '\u{3000}' {
                        paper = paper.child(div().absolute()
                            .left(px((self.pg.left_mm + c.x_mm + c.w_mm * 0.3) * pxmm))
                            .top(px(top + sz * 0.35))
                            .text_size(px(sz * 0.6)).text_color(rgb(0x9DB8C8))
                            .child(SharedString::from(if c.ch == ' ' { "·" } else { "□" })));
                    }
                }
                let end_x = line.cells.last().map(|c| c.x_mm + c.w_mm).unwrap_or(0.0);
                paper = paper.child(div().absolute()
                    .left(px((self.pg.left_mm + end_x) * pxmm)).top(px(top))
                    .text_size(px(sz * 0.8)).text_color(rgb(0x9DB8C8))
                    .child("↵"));
            }
        }
        // ヘッダー・フッター。画面の紙は巻物なので、ヘッダーは紙の頭、
        // フッターは紙の末尾の頁の位置に出す(番号は1ページ目のもの。
        // 各ページの本当の番号は PDF で入る)。編集中は青、普段は灰色
        let foot_shift = (self.content_mm() - self.pg.h_mm).max(0.0);
        for (lines, dy, active) in [
            (&self.header_lines, 0.0, self.hf_edit == Some(false)),
            (&self.footer_lines, foot_shift, self.hf_edit == Some(true)),
        ] {
            for line in lines.iter() {
                if line.cells.is_empty() {
                    continue;
                }
                let pt = line.cells[0].size_pt;
                let sz = pt * 96.0 / 72.0 * self.zoom;
                let x0 = self.pg.left_mm + line.cells[0].x_mm;
                let top = (line.y_mm + dy) * pxmm - sz * 0.88;
                paper = paper.child(div().absolute()
                    .left(px(x0 * pxmm)).top(px(top))
                    .text_size(px(sz))
                    .font_family(self.font_name.clone())
                    .whitespace_nowrap()
                    .text_color(if active { rgb(0x165E83) } else { rgb(0x8899A6) })
                    .child(SharedString::from(line.text())));
            }
        }
        // キャレット。その場の文字の大きさに合わせて描く
        {
            let sz = caret_pt * 96.0 / 72.0 * self.zoom;
            paper = paper.child(div().absolute()
                .left(px(cx_mm * pxmm))
                .top(px(cy_mm * pxmm - sz * 0.88))
                .w(px(1.5)).h(px(sz * 1.15))
                .bg(rgb(0x165E83)));
        }

        // 手描きの線。gpui の Path は「塗り」なので、折れ線を
        // 幅のある四角形の連なりとして塗る(画面も紙も同じ座標)
        {
            let mut strokes: Vec<(bool, Vec<(f32, f32)>)> = Vec::new();
            for st in self.doc.ink.iter().chain(self.ink_cur.iter()) {
                let oy = self
                    .page_offsets
                    .get(st.page)
                    .copied()
                    .unwrap_or(st.page as f32 * self.pg.h_mm);
                strokes.push((
                    st.highlighter,
                    st.points.iter().map(|(x, y)| (x * pxmm, (y + oy) * pxmm)).collect(),
                ));
            }
            if !strokes.is_empty() {
                let pxmm2 = pxmm;
                paper = paper.child(
                    gpui::canvas(|_, _, _| (), move |bounds, _, window, _| {
                        for (hl, pts) in &strokes {
                            let w_px = if *hl { 3.0 } else { 0.45 } * pxmm2;
                            let color = if *hl {
                                gpui::Rgba { r: 1.0, g: 0.9, b: 0.35, a: 0.35 }
                            } else {
                                gpui::Rgba { r: 0.11, g: 0.23, b: 0.32, a: 1.0 }
                            };
                            let o = bounds.origin;
                            let mut path: Option<gpui::Path<gpui::Pixels>> = None;
                            for seg in pts.windows(2) {
                                let (x1, y1) = seg[0];
                                let (x2, y2) = seg[1];
                                let (dx, dy) = (x2 - x1, y2 - y1);
                                let len = (dx * dx + dy * dy).sqrt().max(0.01);
                                let (nx, ny) = (-dy / len * w_px / 2.0, dx / len * w_px / 2.0);
                                let a = gpui::point(o.x + px(x1 + nx), o.y + px(y1 + ny));
                                let b = gpui::point(o.x + px(x2 + nx), o.y + px(y2 + ny));
                                let c = gpui::point(o.x + px(x2 - nx), o.y + px(y2 - ny));
                                let d = gpui::point(o.x + px(x1 - nx), o.y + px(y1 - ny));
                                let p = path.get_or_insert_with(|| gpui::Path::new(a));
                                p.move_to(a);
                                p.line_to(b);
                                p.line_to(c);
                                p.line_to(d);
                            }
                            if let Some(p) = path {
                                window.paint_path(p, color);
                            }
                        }
                    })
                    .absolute()
                    .left(px(0.0))
                    .top(px(0.0))
                    .size_full(),
                );
            }
        }

        // 置換の板
        let find_panel = if !self.find_open {
            None
        } else {
            let field = |label: &str, ed: &Editor, active: bool| {
                // caret は | で見せる(専用の入力部品を作らない割り切り)
                let mut s = ed.text().to_string();
                let cur = ed.cursor().min(s.len());
                if active {
                    s.insert(cur, '|');
                }
                div().flex().flex_row().items_center().gap_2()
                    .child(div().w(px(64.0)).text_size(px(11.5))
                        .text_color(rgb(0x66707A)).child(SharedString::from(label.to_string())))
                    .child(div().flex_1().px_2().py_1().rounded_sm()
                        .border_1()
                        .border_color(if active { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                        .bg(gpui::white())
                        .text_size(px(12.5))
                        .whitespace_nowrap().overflow_hidden()
                        .child(SharedString::from(s)))
            };
            let btn = |id: &str, label: &str| {
                div().id(SharedString::from(id.to_string()))
                    .px_2p5().py_1().rounded_sm()
                    .border_1().border_color(rgb(0x1B6E3C)).text_color(rgb(0x1B6E3C))
                    .text_size(px(11.5)).cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF5EE)))
                    .child(SharedString::from(label.to_string()))
            };
            Some(div().absolute().left(px(16.0)).top(px(8.0)).w(px(430.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_2()
                .child(field("検索", &self.find_ed, self.find_field == 0)
                    .id("find-f").cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| { this.find_field = 0; cx.notify() })))
                .child(field("置換後", &self.repl_ed, self.find_field == 1)
                    .id("find-r").cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| { this.find_field = 1; cx.notify() })))
                .child(div().flex().flex_row().gap_2()
                    .child(btn("f-next", "次へ (Enter)")
                        .on_click(cx.listener(|this, _, _, cx| { this.find_next(); cx.notify() })))
                    .child(btn("f-one", "置換")
                        .on_click(cx.listener(|this, _, _, cx| { this.replace_current(); cx.notify() })))
                    .child(btn("f-all", "すべて置換")
                        .on_click(cx.listener(|this, _, _, cx| { this.replace_all(); cx.notify() })))
                    .child(div().flex_1())
                    .child(btn("f-close", "閉じる")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.find_open = false; cx.notify()
                        })))))
        };

        // ヘッダー・フッターの編集の板。開いている間、打鍵はここに入る
        let hf_panel = self.hf_edit.map(|footer| {
            let title = if footer { "フッター" } else { "ヘッダー" };
            // キャレットは | で見せる(検索の板と同じ割り切り)。
            // ページ番号の印は読める形で見せる
            let mut s = self.hf_ed.text().to_string();
            let cur = self.hf_ed.cursor().min(s.len());
            s.insert(cur, '|');
            let shown = s
                .replace(kumihan::PAGE_MARK, "《ページ番号》")
                .replace(kumihan::PAGES_MARK, "《ページ数》");
            let btn = |id: &str, label: &str| {
                div().id(SharedString::from(id.to_string()))
                    .px_2p5().py_1().rounded_sm()
                    .border_1().border_color(rgb(0x1B6E3C)).text_color(rgb(0x1B6E3C))
                    .text_size(px(11.5)).cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF5EE)))
                    .child(SharedString::from(label.to_string()))
            };
            let mut field = div().flex_1().px_2().py_1().rounded_sm()
                .border_1().border_color(rgb(0x1B6E3C)).bg(gpui::white())
                .text_size(px(12.5)).flex().flex_col();
            for ln in shown.split('\n') {
                field = field.child(div().whitespace_nowrap().overflow_hidden()
                    .child(SharedString::from(ln.to_string())));
            }
            div().absolute().left(px(16.0)).top(px(8.0)).w(px(430.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_2()
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x165E83))
                    .child(SharedString::from(format!("{title}の編集 — 全ページ共通"))))
                .child(field)
                .child(div().flex().flex_row().gap_2()
                    .child(btn("hf-num", "ページ番号を挿入")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.run_cmd("pagenum", cx);
                            cx.notify()
                        })))
                    .child(div().flex_1())
                    .child(btn("hf-close", "閉じる (Esc)")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.hf_edit = None;
                            this.status = "".into();
                            cx.notify()
                        }))))
        });

        // コメントの板と、カーソルの段落のコメントの一覧
        let cmt_panel = if !self.cmt_edit {
            // 板が閉じていても、カーソルの段落にコメントがあれば見せる
            let cur = self.ed.cursor();
            let mut at = 0usize;
            let mut found: Option<Vec<(String, String)>> = None;
            if self.show_comments && self.target == Target::Body {
                for p in self.doc.paragraphs() {
                    let len: usize = p.runs.iter().map(|r| r.text.len()).sum();
                    if at <= cur && cur <= at + len && !p.comments.is_empty() {
                        found = Some(p.comments.iter()
                            .map(|c| (c.author.clone(), c.text.clone()))
                            .collect());
                        break;
                    }
                    at += len + 1;
                }
            }
            found.map(|cs| {
                let mut d = div().absolute().left(px(16.0)).bottom(px(16.0)).w(px(300.0))
                    .p_3().rounded_md().bg(rgb(0xFFF6E6))
                    .border_1().border_color(rgb(0xE8D5A8))
                    .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(0x8A4B00))
                        .child("この段落のコメント(レビュー > コメント で編集)"));
                for (author, text) in cs {
                    d = d.child(div().mt_1p5().text_size(px(11.5)).text_color(rgb(0x5A4A28))
                        .child(SharedString::from(format!("{author}: {text}"))));
                }
                d
            })
        } else {
            // 編集の板(検索の板と同じ作法。| がキャレット)
            let mut t = self.cmt_ed.text().to_string();
            let cur = self.cmt_ed.cursor().min(t.len());
            t.insert(cur, '|');
            let mut field = div().flex_1().px_2().py_1().rounded_sm()
                .border_1().border_color(rgb(0xE08A00)).bg(gpui::white())
                .text_size(px(12.5)).flex().flex_col();
            for ln in t.split('\n') {
                field = field.child(div().whitespace_nowrap().overflow_hidden()
                    .child(SharedString::from(ln.to_string())));
            }
            Some(div().absolute().left(px(16.0)).bottom(px(16.0)).w(px(360.0))
                .p_3().rounded_md().bg(rgb(0xFFF6E6))
                .border_1().border_color(rgb(0xE8D5A8))
                .flex().flex_col().gap_2()
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x8A4B00))
                    .child("コメント — 空にして閉じると外れる"))
                .child(field)
                .child(div().flex().flex_row()
                    .child(div().flex_1())
                    .child(div().id("cmt-close").px_2p5().py_1().rounded_sm()
                        .border_1().border_color(rgb(0x8A4B00)).text_color(rgb(0x8A4B00))
                        .text_size(px(11.5)).cursor_pointer()
                        .hover(|s| s.bg(rgb(0xF7ECD8)))
                        .child("閉じる (Esc)")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.cmt_edit = false;
                            this.status = "".into();
                            cx.notify()
                        })))))
        };

        // 透かしの板
        let wm_panel = if !self.wm_edit {
            None
        } else {
            let mut t = self.wm_ed.text().to_string();
            let cur = self.wm_ed.cursor().min(t.len());
            t.insert(cur, '|');
            Some(div().absolute().left(px(16.0)).top(px(8.0)).w(px(360.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_2()
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x165E83))
                    .child("透かし — 空にして閉じると外れる"))
                .child(div().px_2().py_1().rounded_sm()
                    .border_1().border_color(rgb(0x165E83)).bg(gpui::white())
                    .text_size(px(12.5)).whitespace_nowrap().overflow_hidden()
                    .child(SharedString::from(t)))
                .child(div().flex().flex_row()
                    .child(div().flex_1())
                    .child(div().id("wm-close").px_2p5().py_1().rounded_sm()
                        .border_1().border_color(rgb(0x165E83)).text_color(rgb(0x165E83))
                        .text_size(px(11.5)).cursor_pointer()
                        .hover(|s| s.bg(rgb(0xEAF2F7)))
                        .child("閉じる (Esc)")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.wm_edit = false;
                            this.status = "".into();
                            cx.notify()
                        })))))
        };

        // しおりの板(名前の入力欄+一覧)
        let bm_panel = if !self.bm_open {
            None
        } else {
            let mut t = self.bm_ed.text().to_string();
            let cur = self.bm_ed.cursor().min(t.len());
            t.insert(cur, '|');
            // 一覧(名前と、その段落の頭のバイト位置)
            let mut items: Vec<(String, usize)> = Vec::new();
            let mut at = 0usize;
            for p in self.doc.paragraphs() {
                let len: usize = p.runs.iter().map(|r| r.text.len()).sum();
                for b in &p.bookmarks {
                    items.push((b.clone(), at));
                }
                at += len + 1;
            }
            let mut d = div().absolute().left(px(16.0)).top(px(8.0)).w(px(340.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_2()
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x165E83))
                    .child("しおり — 名前を打って追加。押すとそこへ移る"))
                .child(div().flex().flex_row().gap_2().items_center()
                    .child(div().flex_1().px_2().py_1().rounded_sm()
                        .border_1().border_color(rgb(0x1B6E3C)).bg(gpui::white())
                        .text_size(px(12.5)).whitespace_nowrap().overflow_hidden()
                        .child(SharedString::from(t)))
                    .child(div().id("bm-add").px_2p5().py_1().rounded_sm()
                        .border_1().border_color(rgb(0x1B6E3C)).text_color(rgb(0x1B6E3C))
                        .text_size(px(11.5)).cursor_pointer()
                        .hover(|s| s.bg(rgb(0xEAF5EE)))
                        .child("追加 (Enter)")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.bm_add();
                            cx.notify()
                        }))));
            if items.is_empty() {
                d = d.child(div().text_size(px(11.5)).text_color(rgb(0x66707A))
                    .child("(まだしおりはありません)"));
            }
            for (i, (name, b0)) in items.into_iter().enumerate() {
                let name2 = name.clone();
                d = d.child(div().flex().flex_row().items_center().gap_2()
                    .child(div()
                        .id(SharedString::from(format!("bm-{i}")))
                        .flex_1().px_2().py_0p5().rounded_sm()
                        .text_size(px(12.5)).cursor_pointer()
                        .hover(|s| s.bg(rgb(0xEAF2F7)))
                        .child(SharedString::from(name.clone()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.switch_target(Target::Body);
                            this.ed.move_to(b0, false);
                            this.follow_caret();
                            this.status = format!("しおり「{name}」へ移りました").into();
                            cx.notify()
                        })))
                    .child(div()
                        .id(SharedString::from(format!("bmx-{i}")))
                        .px_1p5().py_0p5().rounded_sm()
                        .text_size(px(11.5)).text_color(rgb(0x9AA5AE)).cursor_pointer()
                        .hover(|s| s.bg(rgb(0xF6E5E2)).text_color(rgb(0xC0392B)))
                        .child("✕")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            for b in &mut this.doc.blocks {
                                if let kumihan::Block::Para(p) = b {
                                    p.bookmarks.retain(|x| *x != name2);
                                }
                            }
                            this.dirty = true;
                            this.status = "しおりを外しました".into();
                            cx.notify()
                        }))));
            }
            Some(d)
        };

        // バージョン履歴の板(控えの一覧。押すと名無しの複製で開く)
        let hist_panel = if !self.hist_open {
            None
        } else {
            let items = self.versions();
            let mut d = div().absolute().left(px(16.0)).top(px(8.0)).w(px(360.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_2()
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x165E83))
                    .child("バージョン履歴 — 上書き保存のたびの控え(9世代まで)"));
            if items.is_empty() {
                d = d.child(div().text_size(px(11.5)).text_color(rgb(0x66707A))
                    .child("(まだ控えはありません。上書き保存すると増えます)"));
            }
            for (i, (disp, q)) in items.into_iter().enumerate() {
                d = d.child(div()
                    .id(SharedString::from(format!("hist-{i}")))
                    .px_2().py_0p5().rounded_sm()
                    .text_size(px(12.5)).cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF2F7)))
                    .child(SharedString::from(disp))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_version(&q);
                        cx.notify()
                    })));
            }
            Some(d)
        };

        // チャットの板(申し送り帳の最近の行+入力欄)
        let chat_panel = if !self.chat_open {
            None
        } else {
            let mut t = self.chat_ed.text().to_string();
            let cur = self.chat_ed.cursor().min(t.len());
            t.insert(cur, '|');
            let lines = self.chat_lines();
            let mut d = div().absolute().left(px(16.0)).top(px(8.0)).w(px(420.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_2()
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x165E83))
                    .child("チャット — 文書の隣の申し送り帳(.chat.txt)"));
            if lines.is_empty() {
                d = d.child(div().text_size(px(11.5)).text_color(rgb(0x66707A))
                    .child("(まだ書き込みはありません)"));
            }
            for l in lines {
                d = d.child(div().text_size(px(12.0))
                    .whitespace_nowrap().overflow_hidden()
                    .child(SharedString::from(l)));
            }
            d = d.child(div().flex().flex_row().gap_2().items_center()
                .child(div().flex_1().px_2().py_1().rounded_sm()
                    .border_1().border_color(rgb(0x1B6E3C)).bg(gpui::white())
                    .text_size(px(12.5)).whitespace_nowrap().overflow_hidden()
                    .child(SharedString::from(t)))
                .child(div().id("chat-send").px_2p5().py_1().rounded_sm()
                    .border_1().border_color(rgb(0x1B6E3C)).text_color(rgb(0x1B6E3C))
                    .text_size(px(11.5)).cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF5EE)))
                    .child("送信 (Enter)")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.chat_send();
                        cx.notify()
                    }))));
            Some(d)
        };

        // パスワードの板(伏せ字。開く時と暗号化を決める時の両方)
        let pw_panel = if !self.pw_open {
            None
        } else {
            let text = self.pw_ed.text();
            let before = text[..self.pw_ed.cursor().min(text.len())].chars().count();
            let total = text.chars().count();
            let masked = format!(
                "{}|{}",
                "●".repeat(before),
                "●".repeat(total - before)
            );
            let title = if self.pw_pending.is_some() {
                "パスワード — この文書は暗号化されています"
            } else {
                "暗号化 — パスワードを決めて Enter(空で解除。Esc で取りやめ)"
            };
            Some(div().absolute().left(px(16.0)).top(px(8.0)).w(px(380.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_2()
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x165E83))
                    .child(SharedString::from(title.to_string())))
                .child(div().px_2().py_1().rounded_sm()
                    .border_1().border_color(rgb(0x1B6E3C)).bg(gpui::white())
                    .text_size(px(12.5)).whitespace_nowrap().overflow_hidden()
                    .child(SharedString::from(masked)))
                .child(div().text_size(px(10.5)).text_color(rgb(0x66707A))
                    .child("方式は ECMA-376 Standard(AES-128)。\
                            Word や LibreOffice でも開けます。\
                            パスワードを忘れると誰にも開けません")))
        };

        // プラグインの板(置き場の .py 一覧。押すと檻の中で実行)
        let plug_panel = if !self.plug_open {
            None
        } else {
            let dir = plugins_dir();
            let mut items: Vec<PathBuf> = std::fs::read_dir(&dir)
                .ok()
                .map(|rd| {
                    rd.flatten()
                        .map(|e| e.path())
                        .filter(|p| p.extension().is_some_and(|e| e == "py"))
                        .collect()
                })
                .unwrap_or_default();
            items.sort();
            let mut d = div().absolute().left(px(16.0)).top(px(8.0)).w(px(420.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_2()
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x165E83))
                    .child("プラグイン — 押すと檻(bubblewrap)の中で実行"))
                .child(div().text_size(px(11.0)).text_color(rgb(0x66707A))
                    .child(SharedString::from(format!("置き場: {}", dir.display()))));
            if items.is_empty() {
                d = d.child(div().text_size(px(11.5)).text_color(rgb(0x66707A))
                    .child("(まだありません。置き場に .py を置いてください。\
                            台本の d が python-docx の文書)"));
            }
            for (i, q) in items.into_iter().enumerate() {
                let name = q
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                d = d.child(div()
                    .id(SharedString::from(format!("plug-{i}")))
                    .px_2().py_0p5().rounded_sm()
                    .text_size(px(12.5)).cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF2F7)))
                    .child(SharedString::from(name))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.plug_open = false;
                        this.run_macro_file(q.clone(), cx);
                        cx.notify()
                    })));
            }
            Some(d)
        };

        // 相互参照の板(しおり一覧 → 文字/ページを挿す。更新もここ)
        let xr_panel = if !self.xr_open {
            None
        } else {
            let names: Vec<String> = self
                .doc
                .paragraphs()
                .flat_map(|p| p.bookmarks.iter().cloned())
                .collect();
            let mut d = div().absolute().left(px(16.0)).top(px(8.0)).w(px(360.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_2()
                .child(div().flex().flex_row().items_center()
                    .child(div().flex_1().text_size(px(11.5))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(0x165E83))
                        .child("相互参照 — しおりの文字かページ番号を挿す"))
                    .child(div().id("xr-refresh").px_2().py_0p5().rounded_sm()
                        .border_1().border_color(rgb(0x1B6E3C)).text_color(rgb(0x1B6E3C))
                        .text_size(px(11.0)).cursor_pointer()
                        .hover(|s| s.bg(rgb(0xEAF5EE)))
                        .child("参照を更新")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.refresh_refs();
                            cx.notify()
                        }))));
            if names.is_empty() {
                d = d.child(div().text_size(px(11.5)).text_color(rgb(0x66707A))
                    .child("(しおりがありません。参考資料 > ブックマークで付けてください)"));
            }
            for (i, name) in names.into_iter().enumerate() {
                let n1 = name.clone();
                let n2 = name.clone();
                d = d.child(div().flex().flex_row().items_center().gap_2()
                    .child(div().flex_1().text_size(px(12.5))
                        .whitespace_nowrap().overflow_hidden()
                        .child(SharedString::from(name)))
                    .child(div().id(SharedString::from(format!("xrt-{i}")))
                        .px_2().py_0p5().rounded_sm()
                        .border_1().border_color(rgb(0x165E83)).text_color(rgb(0x165E83))
                        .text_size(px(11.0)).cursor_pointer()
                        .hover(|s| s.bg(rgb(0xEAF2F7)))
                        .child("文字")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.insert_ref(&n1, false);
                            cx.notify()
                        })))
                    .child(div().id(SharedString::from(format!("xrp-{i}")))
                        .px_2().py_0p5().rounded_sm()
                        .border_1().border_color(rgb(0x165E83)).text_color(rgb(0x165E83))
                        .text_size(px(11.0)).cursor_pointer()
                        .hover(|s| s.bg(rgb(0xEAF2F7)))
                        .child("ページ")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.insert_ref(&n2, true);
                            cx.notify()
                        }))));
            }
            Some(d)
        };

        // フォントの一覧。この機械にある日本語の書体だけ
        let font_panel = if !self.font_list {
            None
        } else {
            let names: Vec<String> = kumihan::font::list()
                .iter()
                .filter(|f| f.japanese && f.regular)
                .map(|f| f.name.clone())
                .take(24)
                .collect();
            let mut d = div().absolute().left(px(16.0)).top(px(8.0)).w(px(280.0))
                .p_2().rounded_md().bg(gpui::white())
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_0p5()
                .child(div().text_size(px(10.5)).text_color(rgb(0x66707A))
                    .child("書体(選んだ段落に掛かる)"));
            for name in names {
                let shown = SharedString::from(name.clone());
                let is_current = self.font_name.as_ref() == name.as_str();
                d = d.child(div()
                    .id(SharedString::from(format!("font-{name}")))
                    .px_2().py_0p5().rounded_sm()
                    .text_size(px(12.5))
                    .font_family(shown.clone())
                    .bg(if is_current { rgb(0xEAF5EE) } else { rgb(0xFFFFFF) })
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF2F7)))
                    .child(shown)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let n = name.clone();
                        let sel = this.ed.selection();
                        this.flush_target();
                        this.doc.apply_font(sel, Some(n.clone()));
                        this.dirty = true;
                        this.relayout_keep();
                        this.font_list = false;
                        this.status = format!("書体を「{n}」に").into();
                        cx.notify();
                    })));
            }
            Some(d)
        };

        // 大きさの一覧
        let size_panel = if !self.size_list {
            None
        } else {
            let mut d = div().absolute().left(px(16.0)).top(px(8.0)).w(px(200.0))
                .p_2().rounded_md().bg(gpui::white())
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_row().flex_wrap().gap_1();
            for pt in [8.0f32, 9.0, 10.0, 10.5, 11.0, 12.0, 14.0, 16.0, 18.0, 22.0, 26.0, 36.0] {
                d = d.child(div()
                    .id(SharedString::from(format!("pt-{pt}")))
                    .px_2().py_1().rounded_sm().text_size(px(12.0))
                    .cursor_pointer().hover(|s| s.bg(rgb(0xEAF2F7)))
                    .child(SharedString::from(format!("{pt}")))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let sel = this.ed.selection();
                        this.flush_target();
                        this.doc.apply_size(sel, move |_| pt);
                        this.dirty = true;
                        this.relayout_keep();
                        this.size_list = false;
                        this.status = format!("大きさを {pt}pt に").into();
                        cx.notify();
                    })));
            }
            Some(d)
        };

        // 段落のスタイルの一覧(標準・見出し1〜3)
        let style_panel = if !self.style_list {
            None
        } else {
            let mut d = div().absolute().left(px(16.0)).top(px(8.0)).w(px(240.0))
                .p_2().rounded_md().bg(gpui::white())
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_0p5()
                .child(div().text_size(px(10.5)).text_color(rgb(0x66707A))
                    .child("段落のスタイル(選んだ段落に掛かる)"));
            for (n, label, pt, bold) in [
                (0u8, "標準", 12.5f32, false),
                (1, "見出し1", 16.0, true),
                (2, "見出し2", 14.0, true),
                (3, "見出し3", 12.5, true),
            ] {
                let mut item = div()
                    .id(SharedString::from(format!("style-{n}")))
                    .px_2().py_0p5().rounded_sm()
                    .text_size(px(pt))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF2F7)))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_para_style(n);
                        this.style_list = false;
                        cx.notify();
                    }));
                if bold {
                    item = item.font_weight(gpui::FontWeight::BOLD);
                }
                d = d.child(item);
            }
            Some(d)
        };

        // 記号の一覧。事務の書類で使うものだけ(飾りの絵文字は入れない)
        let symbol_panel = if !self.symbols {
            None
        } else {
            const SYMS: &[&str] = &[
                "〒", "※", "→", "←", "↑", "↓", "℃", "±", "×", "÷",
                "①", "②", "③", "④", "⑤", "⑥", "⑦", "⑧", "⑨", "⑩",
                "㈱", "㈲", "№", "〆", "〜", "…", "・", "「", "」", "『",
                "』", "【", "】", "○", "●", "◎", "△", "▲", "□", "■",
            ];
            let mut d = div().absolute().right(px(16.0)).top(px(8.0)).w(px(340.0))
                .p_2().rounded_md().bg(gpui::white())
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_row().flex_wrap().gap_1();
            for s in SYMS {
                d = d.child(div()
                    .id(SharedString::from(format!("sym-{s}")))
                    .w(px(28.0)).h(px(28.0)).rounded_sm()
                    .flex().items_center().justify_center()
                    .text_size(px(15.0)).cursor_pointer()
                    .hover(|st| st.bg(rgb(0xEAF2F7)))
                    .child(SharedString::from(*s))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ed.insert(s);
                        this.on_edited();
                        cx.notify();
                    })));
            }
            Some(d)
        };

        // 校正の指摘
        let proof_panel = if self.proof.is_empty() && self.proof_msg.is_empty() {
            None
        } else {
            let mut d = div().absolute().right(px(16.0)).bottom(px(16.0)).w(px(300.0))
                .p_3().rounded_md().bg(gpui::white())
                .border_1().border_color(rgb(0xC6CDD3))
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                       .text_color(rgb(0x165E83))
                       .child(SharedString::from(format!("校正 — {}", self.proof_msg))));
            for n in &self.proof {
                // どちらの道具が出したかを隠さない。辞書の指摘は GPU 無しで再現できる
                let tool = match n.source {
                    ui::check::Source::Dictionary => "辞書",
                    ui::check::Source::Model => "モデル",
                };
                let cand = if n.candidates.is_empty() {
                    "候補なし".to_string()
                } else {
                    n.candidates.join(" / ")
                };
                d = d.child(div().mt_1p5().text_size(px(11.5))
                    .child(SharedString::from(
                        format!("{} → {}  ({}・{tool})", n.found, cand, n.kind.label()))));
            }
            Some(d)
        };

        // ---- 右クリックのメニュー ----
        // InputSink より後に描く(bubble は後に登録した方が先に走るので、
        // 項目の stop_propagation がクリック処理より先に効く — calc と同じ)
        let menu = self.menu_at.map(|(mx, my)| {
            let has_sel = self.ed.has_selection();
            // (id, 名前, 付記, 押せるか)。"" は仕切り
            let entries: Vec<(&'static str, &'static str, &'static str, bool)> = vec![
                ("cut", "切り取り", "Ctrl+X", has_sel),
                ("copy", "コピー", "Ctrl+C", has_sel),
                ("paste", "貼り付け", "Ctrl+V", true),
                ("", "", "", false),
                ("selword", "語を選択", "", true),
                ("selline", "行を選択", "", true),
                ("selall", "すべて選択", "Ctrl+A", true),
                ("", "", "", false),
                ("bold", "太字", "", true),
                ("italic", "斜体", "", true),
                ("underline", "下線", "", true),
                ("", "", "", false),
                ("align-left", "左揃え", "", true),
                ("align-center", "中央揃え", "", true),
                ("align-right", "右揃え", "", true),
                ("align-just", "両端揃え", "", true),
                ("", "", "", false),
                ("replace", "検索と置換", "Ctrl+F", true),
                ("comment", "コメント", "", true),
                ("wordcount", "文字数を数える", "", true),
            ];
            let h_est = entries.len() as f32 * 25.0 + 10.0;
            let win_w = f32::from(window.viewport_size().width);
            let mx = mx.min((win_w - 28.0 - 230.0).max(0.0));
            let my = my.min((self.view_h_px - h_est).max(0.0));
            let mut m = div().absolute().left(px(mx)).top(px(my)).w(px(220.0))
                .p_1().rounded_md().bg(rgb(0xFFFFFF))
                .border_1().border_color(rgb(0xC6CDD3)).shadow_lg()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation());
            for (i, (id, label, hint, ready)) in entries.into_iter().enumerate() {
                if id.is_empty() && label.is_empty() {
                    m = m.child(div().h(px(1.0)).my_1().bg(rgb(0xE1E6EA)));
                    continue;
                }
                if !ready {
                    m = m.child(div()
                        .flex().flex_row().items_center().justify_between().gap_4()
                        .px_3().py_1()
                        .child(div().text_size(px(12.5)).text_color(rgb(0xB6BDC4)).child(label))
                        .child(div().text_size(px(10.5)).text_color(rgb(0xD5DBE0)).child(hint)));
                    continue;
                }
                m = m.child(div()
                    .id(SharedString::from(format!("wm{i}")))
                    .flex().flex_row().items_center().justify_between().gap_4()
                    .px_3().py_1().rounded_sm().cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF2F7)))
                    .child(div().text_size(px(12.5)).text_color(rgb(0x1B1B1B)).child(label))
                    .child(div().text_size(px(10.5)).text_color(rgb(0x9AA5AE)).child(hint))
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                        move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.menu_action(id, window, cx);
                        })));
            }
            m
        });

        let notes = if self.notes.is_empty() { None } else {
            let mut n = div().absolute().right(px(16.0)).top(px(14.0)).w(px(270.0))
                .p_3().rounded_md().bg(rgb(0xFFF6E6))
                .border_1().border_color(rgb(0xE8D5A8))
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                       .text_color(rgb(0x8A4B00)).child("この版で読み飛ばしたもの"));
            for x in &self.notes {
                n = n.child(div().text_size(px(11.0)).text_color(rgb(0x8A4B00))
                            .child(x.clone()));
            }
            Some(n)
        };

        div().size_full().flex().flex_col().bg(th_desk)
            .key_context("jo_edit")
            .track_focus(&self.focus)
            .on_action(cx.listener(Writer::backspace))
            .on_action(cx.listener(Writer::delete))
            .on_action(cx.listener(Writer::left))
            .on_action(cx.listener(Writer::right))
            .on_action(cx.listener(Writer::select_left))
            .on_action(cx.listener(Writer::select_right))
            .on_action(cx.listener(Writer::select_all))
            .on_action(cx.listener(Writer::up))
            .on_action(cx.listener(Writer::down))
            .on_action(cx.listener(Writer::select_up))
            .on_action(cx.listener(Writer::select_down))
            .on_action(cx.listener(Writer::word_left))
            .on_action(cx.listener(Writer::word_right))
            .on_action(cx.listener(Writer::select_word_left))
            .on_action(cx.listener(Writer::select_word_right))
            .on_action(cx.listener(Writer::a_tab))
            .on_action(cx.listener(Writer::a_shift_tab))
            .on_action(cx.listener(Writer::page_up))
            .on_action(cx.listener(Writer::page_down))
            .on_action(cx.listener(Writer::do_find))
            .on_action(cx.listener(Writer::a_context_menu))
            .on_action(cx.listener(Writer::a_cancel))
            .on_action(cx.listener(Writer::doc_home))
            .on_action(cx.listener(Writer::doc_end))
            .on_action(cx.listener(Writer::home))
            .on_action(cx.listener(Writer::end))
            .on_action(cx.listener(Writer::enter))
            .on_action(cx.listener(Writer::copy))
            .on_action(cx.listener(Writer::cut))
            .on_action(cx.listener(Writer::paste))
            .on_action(cx.listener(Writer::undo))
            .on_action(cx.listener(Writer::redo))
            .on_action(cx.listener(Writer::do_save))
            .on_action(cx.listener(Writer::do_open))
            .on_action(cx.listener(Writer::do_quit))
            .child(bar)
            .child(if let Some(fp) = filepage {
                fp
            } else {
                div().flex_1().relative().overflow_hidden()
                    .on_scroll_wheel(cx.listener(|this, e: &gpui::ScrollWheelEvent, _, cx| {
                        // 上に回すと delta は正 → 紙は頭の方へ戻る
                        let dy = match e.delta {
                            gpui::ScrollDelta::Pixels(p) => f32::from(p.y),
                            gpui::ScrollDelta::Lines(l) => l.y * 40.0,
                        };
                        this.scroll_px(-dy);
                        cx.notify();
                    }))
                    .child(paper)
                    .children(notes)
                    .children(find_panel)
                    .children(hf_panel)
                    .children(cmt_panel)
                    .children(wm_panel)
                    .children(bm_panel)
                    .children(xr_panel)
                    .children(hist_panel)
                    .children(chat_panel)
                    .children(plug_panel)
                    .children(pw_panel)
                    .children(font_panel)
                    .children(size_panel)
                    .children(style_panel)
                    .children(symbol_panel)
                    .children(proof_panel)
                    .child(InputSink { view: me })
                    .children(menu)
            })
            .child(statusbar)
    }
}

/// 入力ハンドラは **paint のときに窓へ差す**(GPUI の作法)。
/// 何も描かない要素だが、これが無いと IME もキー入力も届かない。
struct InputSink {
    view: Entity<Writer>,
}

impl IntoElement for InputSink {
    type Element = Self;
    fn into_element(self) -> Self { self }
}

impl gpui::Element for InputSink {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> { None }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> { None }

    fn request_layout(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, ()) {
        let mut style = gpui::Style::default();
        style.size.width = gpui::relative(1.0).into();
        style.size.height = gpui::relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<gpui::Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {}

    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<gpui::Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.view.read(cx).focus.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );
        // クリックでカーソルを置く。編集領域の座標を知っているのはここだけ
        let view = self.view.clone();
        window.on_mouse_event(move |e: &gpui::MouseDownEvent, phase, _w, cx| {
            if phase != gpui::DispatchPhase::Bubble
                || e.button != gpui::MouseButton::Left
                || !bounds.contains(&e.position)
            {
                return;
            }
            let rel = e.position - bounds.origin;
            let clicks = e.click_count;
            let shift = e.modifiers.shift;
            view.update(cx, |w, cx| {
                if w.tab == 0 {
                    // ファイルのページ。紙は無いのでキャレットも筆も動かさない
                    return;
                }
                w.menu_at = None;
                if w.tool.is_some() {
                    // 道具の間、マウスは筆になる(文字は選ばない)
                    let pxmm = PX_PER_MM * w.zoom;
                    let x = (f32::from(rel.x) - 28.0) / pxmm;
                    let y = (f32::from(rel.y) - 14.0) / pxmm + w.scroll_mm;
                    w.ink_begin(x, y);
                    cx.notify();
                    return;
                }
                match clicks {
                    // 二度押しは語、三度押しは行を選ぶ
                    2 => {
                        w.click_at(f32::from(rel.x), f32::from(rel.y), false);
                        w.select_word();
                        w.drag_select = false;
                    }
                    c if c >= 3 => {
                        w.click_at(f32::from(rel.x), f32::from(rel.y), false);
                        w.select_line();
                        w.drag_select = false;
                    }
                    _ => {
                        w.click_at(f32::from(rel.x), f32::from(rel.y), shift);
                        w.drag_select = true;
                    }
                }
                cx.notify();
            });
        });
        // 押したまま動かすと選択が伸びる(文字の選択の通り相場)
        let view = self.view.clone();
        window.on_mouse_event(move |e: &gpui::MouseMoveEvent, phase, _w, cx| {
            if phase != gpui::DispatchPhase::Bubble
                || e.pressed_button != Some(gpui::MouseButton::Left)
            {
                return;
            }
            let rel = e.position - bounds.origin;
            view.update(cx, |w, cx| {
                if w.tool.is_some() {
                    let pxmm = PX_PER_MM * w.zoom;
                    let x = (f32::from(rel.x) - 28.0) / pxmm;
                    let y = (f32::from(rel.y) - 14.0) / pxmm + w.scroll_mm;
                    w.ink_move(x, y);
                    cx.notify();
                    return;
                }
                if w.drag_select {
                    w.click_at(f32::from(rel.x), f32::from(rel.y), true);
                    cx.notify();
                }
            });
        });
        let view = self.view.clone();
        window.on_mouse_event(move |e: &gpui::MouseUpEvent, phase, _w, cx| {
            if phase != gpui::DispatchPhase::Bubble || e.button != gpui::MouseButton::Left {
                return;
            }
            view.update(cx, |w, cx| {
                if w.tool.is_some() {
                    w.ink_end();
                    cx.notify();
                }
                w.drag_select = false;
            });
        });
        // 右クリックでメニュー。選択があれば選択への操作、無ければ押した所へ
        let view = self.view.clone();
        window.on_mouse_event(move |e: &gpui::MouseDownEvent, phase, _w, cx| {
            if phase != gpui::DispatchPhase::Bubble
                || e.button != gpui::MouseButton::Right
                || !bounds.contains(&e.position)
            {
                return;
            }
            let rel = e.position - bounds.origin;
            view.update(cx, |w, cx| {
                if !w.ed.has_selection() {
                    w.click_at(f32::from(rel.x), f32::from(rel.y), false);
                }
                w.menu_at = Some((f32::from(rel.x), f32::from(rel.y)));
                cx.notify();
            });
        });
    }
}

fn main() {
    let arg = std::env::args().nth(1).map(PathBuf::from);
    application().with_assets(ui::Icons).run(move |cx: &mut App| {
        cx.text_system()
            .add_fonts(vec![std::borrow::Cow::Borrowed(font_data())])
            .expect("フォント登録");
        cx.bind_keys(ui::bindings("jo_edit"));
        let bounds = Bounds::centered(None, size(px(900.0), px(1000.0)), cx);
        let arg2 = arg.clone();
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |window, cx| {
                let view = cx.new(|cx| Writer::new(arg2.clone(), cx));
                window.focus(&view.focus_handle(cx), cx);
                // WM からの「閉じる」(Alt+F4 等)も同じ確認を通す。
                // 書きかけがあれば「まだ閉じない」と答え、確認は別の糸で出す
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

#[cfg(test)]
mod cell_edit_tests {
    use super::*;

    fn doc_with_table() -> Document {
        let cell = |s: &str| kumihan::Cellbox {
            paragraphs: vec![kumihan::Paragraph {
                runs: vec![kumihan::Run {
                    text: s.into(), size_pt: SIZE_PT, font: None, fmt: Default::default() }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut d = Document::plain("本文", SIZE_PT);
        d.blocks.push(kumihan::Block::Table(kumihan::Table {
            col_mm: vec![],
            rows: vec![vec![cell("品名"), cell("金額")]],
        }));
        d
    }

    #[test]
    fn セルの文章を読み書きできる() {
        let d = doc_with_table();
        let t = d.tables().next().unwrap();
        assert_eq!(cell_text(&t.rows[0][0]), "品名");
        let mut c = t.rows[0][0].clone();
        set_cell_text(&mut c, "型式\n数量");
        assert_eq!(c.paragraphs.len(), 2, "段落に割れていない");
        assert_eq!(cell_text(&c), "型式\n数量");
    }

    #[test]
    fn セルの書式は書き戻しで残る() {
        let d = doc_with_table();
        let mut c = d.tables().next().unwrap().rows[0][0].clone();
        c.paragraphs[0].align = kumihan::Align::Center;
        c.paragraphs[0].runs[0].fmt.bold = true;
        set_cell_text(&mut c, "直した");
        assert_eq!(c.paragraphs[0].align, kumihan::Align::Center, "揃えが消えた");
        assert!(c.paragraphs[0].runs[0].fmt.bold, "太字が消えた");
    }
}

#[cfg(test)]
mod find_tests {
    use super::*;

    fn w(text: &str) -> (Editor, Editor, Editor) {
        (Editor::new(text), Editor::new(""), Editor::new(""))
    }

    // find_next/replace の中身はエディタ操作の列なので、
    // ここでは検索の規則(後ろから・一周する)だけを関数で確かめる
    fn next_hit(text: &str, term: &str, from: usize) -> Option<usize> {
        text[from..].find(term).map(|i| from + i).or_else(|| text.find(term))
    }

    #[test]
    fn カーソルの後ろから探す() {
        let t = "誤りを直す。誤りは残さない。";
        let first = next_hit(t, "誤り", 0).unwrap();
        let second = next_hit(t, "誤り", first + "誤り".len()).unwrap();
        assert!(second > first);
    }

    #[test]
    fn 末尾まで無ければ頭から一周() {
        let t = "誤りを直す。";
        // 「直」の後ろ(文字境界)から探す。実物の from はカーソル位置なので常に境界
        let from = "誤りを直".len();
        let hit = next_hit(t, "誤り", from);
        assert_eq!(hit, Some(0), "一周していない");
    }

    #[test]
    fn 無ければ無いと言える() {
        assert_eq!(next_hit("本文", "存在しない", 0), None);
        let _ = w("x");
    }
}

#[cfg(test)]
mod wiring_tests {
    #[test]
    fn リボンのreadyは全部配線されている() {
        // 「押せるのに何も起きない」を仕組みで止める
        for tab in ui::ribbon::WRITER {
            for cmd in tab.cmds {
                if cmd.ready {
                    assert!(
                        super::Writer::HANDLED.contains(&cmd.id),
                        "「{}」({}) は ready なのに run_cmd が知らない",
                        cmd.label, cmd.id
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod page_setup_tests {
    use super::*;

    #[test]
    fn 用紙の変更が保存で残る() {
        // 画面で変えただけで docx に書かれないなら、それは飾り
        let mut d = Document::plain("本文", SIZE_PT);
        let mut pg = kumihan::PageSetup::default();
        std::mem::swap(&mut pg.w_mm, &mut pg.h_mm); // 横向き
        d.page = Some(pg);
        d.sect_raw = Some(format!(
            "<w:sectPr><w:pgSz w:w=\"{}\" w:h=\"{}\" w:orient=\"landscape\"/>\
             <w:pgMar w:top=\"1134\" w:right=\"1134\" w:bottom=\"1134\" w:left=\"1134\"/></w:sectPr>",
            (pg.w_mm * 56.6929) as i64, (pg.h_mm * 56.6929) as i64));
        let mut buf = Vec::new();
        ooxml::write(&d, std::io::Cursor::new(&mut buf)).unwrap();
        let (back, _) = ooxml::read(std::io::Cursor::new(&buf)).unwrap();
        let bp = back.page.expect("用紙が消えた");
        assert!(bp.w_mm > bp.h_mm, "横向きが消えた: {}×{}", bp.w_mm, bp.h_mm);
    }

    #[test]
    fn ヘッダーの参照は用紙を変えても残る() {
        // set_page は pgSz/pgMar だけ作り替え、他は原文から引き継ぐ
        let raw = r#"<w:sectPr><w:headerReference r:id="rId8"/><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1134" w:right="1134" w:bottom="1134" w:left="1134"/></w:sectPr>"#;
        // set_page 内の引き継ぎと同じ処理を直接なぞる
        let mut out = String::new();
        let mut skip = false;
        for part in raw.split_inclusive('>') {
            let t = part.trim_start();
            if t.starts_with("<w:sectPr") || t.starts_with("</w:sectPr") {
                continue;
            }
            if t.starts_with("<w:pgSz") || t.starts_with("<w:pgMar") {
                skip = !part.trim_end().ends_with("/>");
                continue;
            }
            if skip {
                continue;
            }
            out.push_str(part);
        }
        assert!(out.contains("headerReference"), "ヘッダーの参照が落ちた: {out}");
        assert!(!out.contains("pgSz"), "古い用紙が残った: {out}");
    }
}

#[cfg(test)]
mod lock_tests {
    use super::*;

    #[test]
    fn ロックの置き場所と先客の判定() {
        let dir = std::env::temp_dir().join(format!("jolock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let doc = dir.join("文書.docx");
        std::fs::write(&doc, b"x").unwrap();
        let lp = lock_path_for(&doc);
        assert!(lp.file_name().unwrap().to_string_lossy().starts_with(".~lock."),
            "LibreOffice と同じ場所でない: {lp:?}");
        // 先客のロック
        std::fs::write(&lp, "花子@dev2,;").unwrap();
        assert_eq!(foreign_lock(&doc).as_deref(), Some("花子@dev2"), "先客が見えない");
        // 自分のロックは先客と見ない
        std::fs::write(&lp, format!("{},;", lock_identity())).unwrap();
        assert_eq!(foreign_lock(&doc), None, "自分を先客と見た");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod track_tests {
    use super::*;

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn 変わった段落と増えた段落が分かる() {
        let base = v(&["一", "二", "三"]);
        let cur = v(&["一", "二を直した", "追加", "三"]);
        let (marks, deleted) = track_diff(&base, &cur);
        assert_eq!(marks[0], PMark::Same);
        assert_eq!(marks[1], PMark::Changed(1), "変わった段落が組みにならない");
        assert_eq!(marks[2], PMark::New, "増えた段落が新規にならない");
        assert_eq!(marks[3], PMark::Same);
        assert!(deleted.is_empty());
    }

    #[test]
    fn 消えた段落は次の段落の前に付く() {
        let base = v(&["一", "二", "三"]);
        let cur = v(&["一", "三"]);
        let (marks, deleted) = track_diff(&base, &cur);
        assert_eq!(marks, vec![PMark::Same, PMark::Same]);
        assert_eq!(deleted, vec![(1, 1)], "消えた段落の場所が違う");
    }

    #[test]
    fn 文字の差分は頭と尻尾を残す() {
        let (pre, del, ins, suf) = split_diff("防火戸の仕様", "防火ドアの仕様");
        assert_eq!((pre.as_str(), del.as_str(), ins.as_str(), suf.as_str()),
            ("防火", "戸", "ドア", "の仕様"));
        let (pre, del, ins, suf) = split_diff("同じ", "同じ");
        assert_eq!((pre.as_str(), del.as_str(), ins.as_str(), suf.as_str()),
            ("同じ", "", "", ""));
    }
}

#[cfg(test)]
mod word_tests {
    use super::*;

    #[test]
    fn 英語は空白と語の境で止まる() {
        let t = "hello world  foo";
        assert_eq!(word_boundary(t, 0, true), 6, "次の語の頭に行かない");
        assert_eq!(word_boundary(t, 6, true), 13);
        assert_eq!(word_boundary(t, 13, false), 6, "前の語の頭に戻らない");
        assert_eq!(word_boundary(t, 6, false), 0);
        assert_eq!(word_boundary(t, t.len(), true), t.len(), "末尾で止まらない");
    }

    #[test]
    fn 日本語は文字種の変わり目で止まる() {
        // 漢字の連なり→ひらがな→カタカナ→英数、の境で切れる
        let t = "防火戸のカタログをPDFで";
        let b = |s: &str| t.find(s).unwrap();
        assert_eq!(word_boundary(t, 0, true), b("の"), "漢字の連なりを1語にしない");
        assert_eq!(word_boundary(t, b("の"), true), b("カタログ"));
        assert_eq!(word_boundary(t, b("カタログ"), true), b("を"),
            "カタカナの連なりが1語にならない");
        assert_eq!(word_boundary(t, b("PDF"), false), b("を"));
    }

    #[test]
    fn 端で壊れない() {
        assert_eq!(word_boundary("", 0, true), 0);
        assert_eq!(word_boundary("", 0, false), 0);
        assert_eq!(word_boundary("あ", 0, false), 0);
    }
}

#[cfg(test)]
mod image_px_tests {
    use super::*;

    #[test]
    fn pngの画素数が読める() {
        // 署名 + IHDR(幅640, 高さ480)
        let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        b.extend_from_slice(&[0, 0, 0, 13]);
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&640u32.to_be_bytes());
        b.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(image_px(&b), Some((640, 480)));
    }

    #[test]
    fn jpegの画素数が読める() {
        // SOI + APP0(空) + SOF0(高さ300, 幅200)
        let mut b = vec![0xFF, 0xD8];
        b.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x02]); // APP0 長さ2(中身なし)
        b.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x0B, 0x08]);
        b.extend_from_slice(&300u16.to_be_bytes()); // 高さ
        b.extend_from_slice(&200u16.to_be_bytes()); // 幅
        b.extend_from_slice(&[0x03, 0x01, 0x01, 0x00]);
        assert_eq!(image_px(&b), Some((200, 300)), "SOF0 の(幅, 高さ)が読めない");
    }

    #[test]
    fn 画像でないものは断る() {
        assert_eq!(image_px(b"not an image"), None);
    }
}

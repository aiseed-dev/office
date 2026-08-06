//! リボン(タブ+コマンド)。**Euro-Office の現物から生成している。**
//!
//! このファイルは手で書かない。`gen_ribbon.py` が
//! `vendor/web-apps/apps/*/main/app/template/Toolbar.template` の並び順と
//! 同 app の `locale/ja.json` の名前から起こす。
//! だから「Euro-Office と全く同じか」は台本を回し直せば確かめられる。
//!
//! ```text
//! python3 ui/gen_ribbon.py ja > ui/src/ribbon.rs
//! ```
//!
//! **全部入れる**(2026-08-04 発注者確定で改訂。以前は共同編集・保護・
//! プラグイン・AI・マクロを「入れない」としていた)。乗り換える人の
//! 目当てを消さないため、タブも釦も本家どおり並べる。
//! **VBA 型のマクロを持たないことだけは不変** — マクロの釦の実体は
//! 檻(bubblewrap)の中の Python で、文書の中に実行コードは置かない。
//!
//! **できないものを、できるように見せない。** 実装済みのコマンドだけを押せる形にし、
//! 未実装は灰色で残す。並びを Euro-Office に合わせたまま、
//! 「今どこまで出来ているか」がそのまま画面に出る。

/// 1つのコマンド。`ready=false` は未実装(押せない灰色)。
/// `icon` は Euro-Office の slot 名で、埋め込んだアイコン(icons.rs)を引く鍵。
#[derive(Clone, Copy)]
pub struct Cmd {
    pub id: &'static str,
    pub label: &'static str,
    pub icon: &'static str,
    pub ready: bool,
}

pub(crate) const fn c(id: &'static str, label: &'static str, icon: &'static str) -> Cmd {
    Cmd { id, label, icon, ready: true }
}
#[allow(dead_code)] // 灰色ゼロの今は未使用だが、ロケール表の生成が使う形
pub(crate) const fn x(label: &'static str, icon: &'static str) -> Cmd {
    Cmd { id: "", label, icon, ready: false }
}

/// いまの言語のリボン。**語だけが違う** — id・並び・ready・icon は
/// どの言語でも ja(WRITER/CALC)と同一(下の試験が保証)。
/// 内部の論理(タブ名の照合など)は ja の表で書いてよい —
/// 添字がそのまま対応する
pub fn writer_tabs() -> &'static [Tab] {
    crate::ribbon_tables::tabs(crate::settings::language())
        .map(|(w, _)| w)
        .unwrap_or(WRITER)
}

pub fn calc_tabs() -> &'static [Tab] {
    crate::ribbon_tables::tabs(crate::settings::language())
        .map(|(_, c)| c)
        .unwrap_or(CALC)
}

pub struct Tab {
    pub name: &'static str,
    pub cmds: &'static [Cmd],
}

pub const WRITER: &[Tab] = &[
    Tab { name: "ファイル", cmds: &[
        c("open", "開く", "open"),
        c("save", "保存", "save"),
        c("pdf", "印刷", "print"),
    ]},
    Tab { name: "ホーム", cmds: &[
        c("copy", "コピー", "copy"),
        c("cut", "切り取り", "cut"),
        c("paste", "貼り付け", "paste"),
        c("fontname", "フォント", "fontname"),
        c("fontsize", "フォントのサイズ", "fontsize"),
        c("incfont", "フォントサイズの拡大", "incfont"),
        c("decfont", "フォントサイズの縮小", "decfont"),
        c("changecase", "大文字小文字を変更", "changecase"),
        c("ruby", "ルビ", "ruby"),
        c("bold", "太字", "bold"),
        c("italic", "斜体", "italic"),
        c("underline", "下線", "underline"),
        c("strikeout", "取り消し線", "strikeout"),
        c("superscript", "上付き", "superscript"),
        c("subscript", "下付き", "subscript"),
        c("highlight", "ハイライトの色", "highlight"),
        c("fontcolor", "フォントの色", "fontcolor"),
        c("clearstyle", "スタイルのクリア", "clearstyle"),
        c("markers", "箇条書き", "markers"),
        c("numbering", "ナンバリング", "numbering"),
        c("multilevels", "複数レベルのリスト", "multilevels"),
        c("decoffset", "インデントを減らす", "decoffset"),
        c("incoffset", "インデントを増やす", "incoffset"),
        c("linespace", "段落の行間", "linespace"),
        c("direction", "テキスト方向", "direction"),
        c("align-left", "左揃え", "align-left"),
        c("align-center", "中央揃え", "align-center"),
        c("align-right", "右揃え", "align-right"),
        c("align-just", "両端揃え", "align-just"),
        c("align-dist", "均等割付", "align-dist"),
        c("hidenchars", "非表示文字", "hidenchars"),
        c("paracolor", "段落の背景色", "paracolor"),
        c("borders", "罫線", "borders"),
        c("parastyle", "段落のスタイル", "styles"),
        c("replace", "置き換え", "replace"),
        c("selectall", "すべて選択", "select-all"),
    ]},
    Tab { name: "挿入", cmds: &[
        c("blankpage", "空白ページの挿入", "blankpage"),
        c("pagebreak", "ページの挿入またはセクション区切り", "pagebreak"),
        c("instable", "表の挿入", "instable"),
        c("insimage", "画像を挿入", "insertimage"),
        c("insshape", "図形を挿入", "insshape"),
        c("inssmartart", "SmartArtの挿入", "inssmartart"),
        c("inschart", "グラフを挿入", "inschart"),
        c("smartpicker", "グラフを挿入", "smartpicker"),
        c("instext", "テキストボックスの挿入", "instext"),
        c("instextart", "テキストアートの挿入", "instextart"),
        c("dropcap", "ドロップキャップの挿入", "dropcap"),
        c("text-from-file", "ファイルからのテキスト", "text-from-file"),
        c("edit-header", "ヘッダーの編集", "edit-header"),
        c("edit-footer", "フッターの編集", "edit-footer"),
        c("pagenum", "ページ番号", "pagenum"),
        c("datetime", "日付/時刻", "datetime"),
        c("numpages", "ページ数", "numpages"),
        c("insequation", "方程式を挿入", "insequation"),
        c("inssymbol", "記号を挿入", "inssymbol"),
        c("controls", "コンテンツコントロールの挿入", "controls"),
    ]},
    Tab { name: "描画", cmds: &[
        c("pen", "ペン", "pen"),
        c("highlighter", "蛍光ペン", "highlighter"),
        c("eraser", "消しゴム", "eraser"),
    ]},
    Tab { name: "レイアウト", cmds: &[
        c("pagemargins", "余白", "pagemargins"),
        c("pageorient", "印刷の向き", "pageorient"),
        c("pagesize", "ページのサイズ", "pagesize"),
        c("columns", "列の挿入", "columns"),
        c("line-numbers", "行番号を表示する", "line-numbers"),
        c("hyphenation", "ハイフン設定の変更", "hyphenation"),
        c("watermark", "透かしを編集する", "watermark"),
        c("pagecolor", "ページ色の変更", "pagecolor"),
        c("colorschemas", "配色の変更", "colorschemas"),
    ]},
    Tab { name: "参考資料", cmds: &[
        c("toc", "目次", "contents"),
        c("add-text", "テキストの追加", "add-text"),
        c("toc-update", "目次の更新", "contents-update"),
        c("bookmarks", "ブックマーク", "bookmarks"),
        c("caption", "図表番号", "caption"),
        c("crossref", "相互参照", "crossref"),
        c("tof", "図表目次", "tof"),
        c("tof-update", "図表目次の更新", "tof-update"),
    ]},
    Tab { name: "フォーム", cmds: &[
        c("form-text", "テキストフィールド", "form-text"),
        c("form-combo", "コンボボックス", "form-combo"),
        c("form-dropdown", "ドロップダウン", "form-dropdown"),
        c("form-checkbox", "チェックボックス", "form-checkbox"),
        c("form-radio", "ラジオボタン", "form-radio"),
        c("form-image", "画像", "form-image"),
        c("form-email", "メールアドレス", "form-email"),
        c("form-phone", "電話番号", "form-phone"),
        c("form-complex", "複合フィールド", "form-complex"),
        c("form-signature", "署名", "form-signature"),
        c("form-name", "名前", "form-name"),
    ]},
    Tab { name: "共同編集", cmds: &[
        c("coauth-mode", "共同編集モード", "coauth-mode"),
        c("co-addcomment", "コメントを追加", "co-addcomment"),
        c("co-delcomment", "コメントを削除", "co-delcomment"),
        c("co-showcomment", "コメントの表示", "co-showcomment"),
        c("co-chat", "チャット", "co-chat"),
        c("track-changes", "変更履歴", "track-changes"),
        c("co-history", "バージョン履歴", "co-history"),
    ]},
    Tab { name: "保護", cmds: &[
        c("prot-encrypt", "暗号化する", "prot-encrypt"),
        c("prot-sign", "デジタル署名を追加", "prot-sign"),
        c("prot-doc", "保護", "prot-doc"),
    ]},
    Tab { name: "表示", cmds: &[
        c("nav", "ナビゲーション", "nav"),
        c("fit-page", "ページに合わせる", "fit-page"),
        c("fit-width", "幅に合わせる", "fit-width"),
        c("zoom100", "100%に拡大する", "zoom100"),
        c("zoom-in", "拡大", "zoom-in"),
        c("zoom-out", "縮小", "zoom-out"),
        c("multipage", "複数ページ", "multipage"),
        c("darkmode", "ダークモード", "darkmode"),
        c("ruler", "ルーラー", "ruler"),
        c("show-toolbar", "ツールバーを常に表示する", "show-toolbar"),
        c("show-statusbar", "ステータスバー", "show-statusbar"),
        c("show-left", "左パネル", "show-left"),
        c("show-right", "右パネル", "show-right"),
    ]},
    Tab { name: "プラグイン", cmds: &[
        c("plug-macros", "マクロ", "plug-macros"),
        c("plug-manage", "プラグインの管理", "plug-manage"),
    ]},
    Tab { name: "AI", cmds: &[
        c("ai-where", "宛先", "ai-where"),
        c("ai-summary", "要約", "ai-summary"),
        c("ai-rewrite", "書き直す", "ai-rewrite"),
        c("ai-polite", "敬語にする", "ai-polite"),
        c("ai-plain", "やさしく", "ai-plain"),
        c("ai-translate", "翻訳", "ai-translate"),
        c("ai-furigana", "ふりがな", "ai-furigana"),
        c("ai-continue", "続きを書く", "ai-continue"),
        c("ai-table", "表にする", "ai-table"),
        c("ai-ask", "頼む", "ai-ask"),
        c("ai-macro", "マクロを書く", "ai-macro"),
    ]},
];

pub const CALC: &[Tab] = &[
    Tab { name: "ファイル", cmds: &[
        c("open", "開く", "open"),
        c("save", "保存", "save"),
        c("pdf", "印刷", "print"),
    ]},
    Tab { name: "ホーム", cmds: &[
        c("copy", "コピー", "copy"),
        c("cut", "切り取り", "cut"),
        c("paste", "貼り付け", "paste"),
        c("fontname", "フォント", "fontname"),
        c("fontsize", "フォントのサイズ", "fontsize"),
        c("incfont", "フォントサイズの拡大", "incfont"),
        c("decfont", "フォントサイズの縮小", "decfont"),
        c("changecase", "大文字小文字を変更", "changecase"),
        c("bold", "太字", "bold"),
        c("italic", "斜体", "italic"),
        c("underline", "下線", "underline"),
        c("strikeout", "取り消し線", "strikeout"),
        c("subscript", "下付き", "subscript"),
        c("fontcolor", "フォントの色", "fontcolor"),
        c("fillparag", "塗りつぶしの色", "fillparag"),
        c("borders", "表の枠線", "borders"),
        c("top", "上揃え", "top"),
        c("middle", "中央揃え", "middle"),
        c("bottom", "下揃え", "bottom"),
        c("wrap", "折り返して​​全体を表示する", "wrap"),
        c("text-orient", "印刷の向き", "text-orient"),
        c("align-left", "左揃え", "align-left"),
        c("align-center", "中央揃え", "align-center"),
        c("align-right", "右揃え", "align-right"),
        c("align-just", "両端揃え", "align-just"),
        c("merge", "結合して、中央に配置する", "merge"),
        c("direction", "文字の向き(右横書き)", "direction"),
        c("insert-function", "関数", "formula"),
        c("fill-num", "フィル", "fill-num"),
        c("defname", "名前の管理", "named-range"),
        c("clear", "消去", "clear"),
        c("sort-desc", "降順並べ替え", "sortdesc"),
        c("sort-asc", "昇順並べ替え", "sortasc"),
        c("setfilter", "フィルター", "setfilter"),
        c("clear-filter", "フィルターを解除", "clear-filter"),
        c("format", "数値の書式", "format"),
        c("currency", "通貨スタイル", "currency"),
        c("percents", "パーセントのスタイル", "percents"),
        c("comma", "カンマスタイル", "comma"),
        c("digit-dec", "小数点以下の表示桁数を減らす", "digit-dec"),
        c("digit-inc", "小数点以下の表示桁数を増やす", "digit-inc"),
        c("cell-ins", "セルを挿入", "cell-ins"),
        c("cell-del", "セルを削除", "cell-del"),
        c("cell-format", "セルのスタイル", "cell-format"),
        c("condformat", "条件付き書式", "condformat"),
        c("table-tpl", "表の挿入", "table-tpl"),
        c("cell-styles", "セルのスタイル", "styles"),
        c("replace", "置き換え", "replace"),
        c("selectall", "すべて選択", "select-all"),
    ]},
    Tab { name: "挿入", cmds: &[
        c("pivot-insert", "ピボットテーブルを挿入", "add-pivot"),
        c("instable", "表の挿入", "instable"),
        c("insimage", "画像を挿入", "insimage"),
        c("insshape", "図形を挿入", "insshape"),
        c("inssmartart", "SmartArtの挿入", "inssmartart"),
        c("inscheckbox", "チェックボックス", "inscheckbox"),
        c("insrecommend", "推奨チャートを挿入", "insrecommend"),
        c("inschart", "グラフを挿入", "inschart"),
        c("inssparkline", "スパークラインを挿入する", "inssparkline"),
        c("addcomment", "コメント", "ins-comment"),
        c("insrecommend", "グラフを挿入", "smartpicker"),
        c("inshyperlink", "ハイパーリンクを追加", "inshyperlink"),
        c("insslicer", "スライサーを挿入", "insslicer"),
        c("instext", "テキストボックスを挿入する", "instext"),
        c("instextart", "テキストアートの挿入", "instextart"),
        x("ヘッダー/フッター", "editheader"),
        c("insequation", "方程式を挿入", "insequation"),
        c("inssymbol", "記号を挿入", "inssymbol"),
    ]},
    Tab { name: "描画", cmds: &[
        c("draw-select", "選択", "select-tool"),
        c("pen", "ペン", "pen"),
        c("highlighter", "蛍光ペン", "highlighter"),
        c("eraser", "消しゴム", "eraser"),
    ]},
    Tab { name: "レイアウト", cmds: &[
        c("pagemargins", "余白", "pagemargins"),
        c("pageorient", "印刷の向き", "pageorient"),
        c("pagesize", "ページのサイズ", "pagesize"),
        c("printarea", "印刷範囲", "printarea"),
        c("pagebreak", "印刷物で次のページを開始する位置に改行を追加する", "pagebreak"),
        x("ヘッダー/フッター", "editheader"),
        c("scale", "拡大縮小印刷", "scale"),
        c("printtitles", "タイトルを印刷する", "printtitles"),
        c("rtl-sheet", "最初の列が右側に来るようにシートの方向を切り替える", "rtl-sheet"),
        c("print-gridlines", "枠線も印刷", "print-gridlines"),
        c("print-headings", "見出しも印刷", "print-headings"),
        x("前面ヘ移動", "img-movefrwd"),
        x("背面ヘ移動", "img-movebkwd"),
        x("配置", "img-align"),
        x("グループ化", "img-group"),
        x("図形を結合", "shapes-merge"),
        c("colorschemas", "配色の変更", "colorschemas"),
    ]},
    Tab { name: "数式", cmds: &[
        c("insert-function", "関数の挿入", "additional-formula"),
        c("sum", "オートSUM", "autosum"),
        c("fn-recent", "最近使った関数", "recent"),
        c("fn-financial", "財務", "financial"),
        c("fn-logical", "論理", "logical"),
        c("fn-text", "文字列操作", "text"),
        c("fn-datetime", "日付/時刻", "datetime"),
        c("fn-lookup", "検索/行列", "lookup"),
        c("fn-math", "数学/三角", "math"),
        c("fn-more", "その他の関数", "more"),
        c("defname", "名前の管理", "named-range-huge"),
        c("trace-prec", "参照元のトレース", "trace-prec"),
        c("trace-dep", "参照先のトレース", "trace-dep"),
        c("remove-arrows", "トレース矢印の削除", "remove-arrows"),
        c("show-formulas", "数式の表示", "show-formulas"),
        c("watch", "ウォッチウィンドウ", "watch-window"),
        c("calc-mode", "計算方法", "calculate"),
    ]},
    Tab { name: "データ", cmds: &[
        c("data-from-text", "テキストからデータ", "data-from-text"),
        c("data-external-links", "外部リンク(値で取り込む)", "data-external-links"),
        c("setfilter", "フィルター", "setfilter"),
        c("clear-filter", "フィルターを解除", "clear-filter"),
        c("sort-desc", "降順並べ替え", "sortdesc"),
        c("sort-asc", "昇順並べ替え", "sortasc"),
        c("custom-sort", "並べ替え", "custom-sort"),
        c("text-column", "区切り位置", "text-column"),
        c("rem-duplicates", "重複の削除", "rem-duplicates"),
        c("data-validation", "データの入力規則", "data-validation"),
        c("goal-seek", "ゴールシーク", "goal-seek"),
        c("solver", "ソルバー", "solver"),
        c("group", "グループ化", "group"),
        c("ungroup", "グループ解除", "ungroup"),
        c("show-details", "詳細の表示", "show-details"),
        c("hide-details", "詳細の非表示", "hide-details"),
        c("subtotal", "小計", "subtotal"),
        c("python", "Python", "python"),
    ]},
    Tab { name: "ピボットテーブル", cmds: &[
        c("pivot-insert", "ピボットテーブルを挿入", "pivot-insert"),
        c("pivot-fields", "フィールドリスト", "pivot-fields"),
        c("pivot-refresh", "更新", "pivot-refresh"),
        c("pivot-refresh-all", "すべて更新", "pivot-refresh-all"),
        c("pivot-select", "選択する", "pivot-select"),
        c("pivot-totals", "総計", "pivot-totals"),
        c("pivot-subtotals", "小計", "pivot-subtotals"),
        c("pivot-blank", "空行", "pivot-blank"),
        c("pivot-layout", "レポートのレイアウト", "pivot-layout"),
        c("pivot-style", "スタイル", "pivot-style"),
    ]},
    Tab { name: "表のデザイン", cmds: &[
        c("td-header", "ヘッダー行", "td-header"),
        c("td-total", "合計行", "td-total"),
        c("td-band-row", "縞模様の行", "td-band-row"),
        c("td-first", "最初の列", "td-first"),
        c("td-last", "最後の列", "td-last"),
        c("td-band-col", "縞模様の列", "td-band-col"),
        c("td-filter", "フィルタのボタン", "td-filter"),
        c("rem-duplicates", "重複データを削除", "td-remdup"),
        c("td-torange", "範囲に変換する", "td-torange"),
        c("td-resize", "テーブルのサイズ変更", "td-resize"),
    ]},
    Tab { name: "共同編集", cmds: &[
        c("coauth-mode", "共同編集モード", "coauth-mode"),
        c("addcomment", "コメントを追加", "co-addcomment"),
        c("co-delcomment", "コメントを削除", "co-delcomment"),
        c("co-showcomment", "コメントの表示", "co-showcomment"),
        c("co-chat", "チャット", "co-chat"),
        c("co-history", "バージョン履歴", "co-history"),
    ]},
    Tab { name: "保護", cmds: &[
        // 本家 SSE の並び: 暗号化 / ブック / シート / 範囲。
        // ブックと範囲は未実装(灰)。署名は本家に無いこちらの釦 — 末尾
        c("prot-encrypt", "暗号化する", "prot-encrypt"),
        x("ブックを保護する", "protect-workbook"),
        c("prot-doc", "シートを保護する", "protect-sheet"),
        x("範囲を保護する", "protect-range"),
        c("prot-sign", "デジタル署名を追加", "prot-sign"),
    ]},
    Tab { name: "表示", cmds: &[
        c("sheet-view", "シートの表示", "sheet-view"),
        x("標準", "view-normal"),
        x("改ページ プレビュー", "view-pagebreak"),
        c("zoom-in", "拡大", "zoom-in"),
        c("zoom-out", "縮小", "zoom-out"),
        c("ui-bigger", "画面の文字を大きく", "ui-bigger"),
        c("ui-smaller", "画面の文字を小さく", "ui-smaller"),
        c("theme", "インターフェイステーマ", "theme"),
        c("freeze", "ウィンドウ枠の固定", "freeze"),
        c("formula-bar", "数式バー", "formula-bar"),
        c("show-gridlines", "枠線表示", "show-gridlines"),
        c("show-headings", "見出し", "show-headings"),
        c("show-zeros", "0を表示する", "show-zeros"),
    ]},
    Tab { name: "プラグイン", cmds: &[
        c("plug-macros", "マクロ", "plug-macros"),
        c("plug-manage", "プラグインの管理", "plug-manage"),
    ]},
    Tab { name: "AI", cmds: &[
        c("ai-where", "宛先", "ai-where"),
        c("ai-summary", "要約", "ai-summary"),
        c("ai-rewrite", "書き直す", "ai-rewrite"),
        c("ai-polite", "敬語にする", "ai-polite"),
        c("ai-plain", "やさしく", "ai-plain"),
        c("ai-translate", "翻訳", "ai-translate"),
        c("ai-furigana", "ふりがな", "ai-furigana"),
        c("ai-continue", "続きを書く", "ai-continue"),
        c("ai-table", "表にする", "ai-table"),
        c("ai-ask", "頼む", "ai-ask"),
    ]},
];

/// 実装済みのコマンド数 / 全体(進み具合を隠さない)
pub fn progress(tabs: &[Tab]) -> (usize, usize) {
    let all: usize = tabs.iter().map(|t| t.cmds.len()).sum();
    let ready: usize = tabs.iter().flat_map(|t| t.cmds).filter(|c| c.ready).count();
    (ready, all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 各言語の表は語だけが違う() {
        // id・並び・ready・icon が ja と一致しない表は配線が壊れる —
        // ここで固定する(語は違ってよい。空の語は出さない)
        let mut pairs: Vec<(&[Tab], &[Tab])> = Vec::new();
        for l in lang::i18n::languages() {
            if l == "ja" {
                continue;
            }
            let (w, c) = crate::ribbon_tables::tabs(l)
                .unwrap_or_else(|| panic!("言語 {l} のリボンの表が無い(登録簿のずれ)"));
            pairs.push((WRITER, w));
            pairs.push((CALC, c));
        }
        for (ja, other) in pairs {
            assert_eq!(ja.len(), other.len(), "タブの数が違う");
            for (a, b) in ja.iter().zip(other) {
                assert!(!b.name.is_empty(), "タブ名が空");
                assert_eq!(a.cmds.len(), b.cmds.len(), "「{}」の釦の数が違う", a.name);
                for (x, y) in a.cmds.iter().zip(b.cmds) {
                    assert_eq!(x.id, y.id, "id がずれた(配線が壊れる)");
                    assert_eq!(x.icon, y.icon, "「{}」の icon が違う", x.id);
                    assert_eq!(x.ready, y.ready, "「{}」の ready が違う", x.id);
                    assert!(!y.label.is_empty(), "「{}」の語が空", x.id);
                }
            }
        }
    }

    #[test]
    fn 本家のタブが全部ある() {
        // 発注者確定(2026-08-04): メニューは制限しない。実装しないものも
        // 場所は本家どおり(灰色)。タブごと消すことはしない
        for tabs in [WRITER, CALC] {
            for want in ["共同編集", "保護", "プラグイン"] {
                assert!(
                    tabs.iter().any(|t| t.name == want),
                    "タブが無い: {want}"
                );
            }
        }
        for want in ["フォーム"] {
            assert!(WRITER.iter().any(|t| t.name == want), "writer にタブが無い: {want}");
        }
        for want in ["ピボットテーブル", "表のデザイン"] {
            assert!(CALC.iter().any(|t| t.name == want), "calc にタブが無い: {want}");
        }
    }

    #[test]
    fn 実装済みと未実装が区別されている() {
        // 「押せるのに何も起きない」を作らないための検査
        for tabs in [WRITER, CALC] {
            for t in tabs {
                for cmd in t.cmds {
                    assert_eq!(cmd.ready, !cmd.id.is_empty(),
                        "{} の「{}」: ready と id が食い違う", t.name, cmd.label);
                }
            }
        }
    }

    #[test]
    fn euro_officeのタブが揃っている() {
        let names: Vec<&str> = WRITER.iter().map(|t| t.name).collect();
        for want in ["ファイル", "ホーム", "挿入", "レイアウト", "参考資料"] {
            assert!(names.contains(&want), "文書に「{want}」タブが無い: {names:?}");
        }
        let names: Vec<&str> = CALC.iter().map(|t| t.name).collect();
        for want in ["ファイル", "ホーム", "挿入", "レイアウト", "数式", "データ"] {
            assert!(names.contains(&want), "表計算に「{want}」タブが無い: {names:?}");
        }
    }

    #[test]
    fn どの言語でも並びの数は同じ() {
        // 言葉が変わるだけで、リボンの構造は Euro-Office と同じ形
        assert!(WRITER.len() >= 5, "タブが少なすぎる: {}", WRITER.len());
        assert!(CALC.len() >= 6, "タブが少なすぎる: {}", CALC.len());
    }

    #[test]
    fn 名前が空でない() {
        for tabs in [WRITER, CALC] {
            for t in tabs {
                assert!(!t.name.is_empty());
                for cmd in t.cmds {
                    assert!(!cmd.label.is_empty(), "{} に名無しのコマンド", t.name);
                }
            }
        }
    }
}


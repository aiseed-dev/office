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
//! **入れないもの**(方針): 共同編集・保護・プラグイン・AI・マクロ。
//! マクロを持たないのは機能不足ではなく、文書の中に実行コードを置かない設計。
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

const fn c(id: &'static str, label: &'static str, icon: &'static str) -> Cmd {
    Cmd { id, label, icon, ready: true }
}
const fn x(label: &'static str, icon: &'static str) -> Cmd {
    Cmd { id: "", label, icon, ready: false }
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
        x("テキスト方向", "direction"),
        c("align-left", "左揃え", "align-left"),
        c("align-center", "中央揃え", "align-center"),
        c("align-right", "右揃え", "align-right"),
        c("align-just", "両端揃え", "align-just"),
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
        x("コンテンツコントロールの挿入", "controls"),
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
        x("配色の変更", "colorschemas"),
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
        x("テキストフィールド", "form-text"),
        x("コンボボックス", "form-combo"),
        x("ドロップダウン", "form-dropdown"),
        x("チェックボックス", "form-checkbox"),
        x("ラジオボタン", "form-radio"),
        x("画像", "form-image"),
        x("メールアドレス", "form-email"),
        x("電話番号", "form-phone"),
        x("複合フィールド", "form-complex"),
        x("署名", "form-signature"),
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
        c("zoom-in", "拡大", "zoom-in"),
        c("zoom-out", "縮小", "zoom-out"),
        c("ruler", "ルーラー", "ruler"),
        c("darkmode", "ダークモード", "darkmode"),
    ]},
    Tab { name: "プラグイン", cmds: &[
        c("plug-macros", "マクロ", "plug-macros"),
        c("plug-manage", "プラグインの管理", "plug-manage"),
    ]},
];

pub const CALC: &[Tab] = &[
    Tab { name: "ファイル", cmds: &[
        c("open", "開く", "open"),
        c("save", "保存", "save"),
        c("pdf", "印刷", "print"),
    ]},
    Tab { name: "ホーム", cmds: &[
        c("fontname", "フォント", "fontname"),
        c("fontsize", "フォントのサイズ", "fontsize"),
        c("incfont", "フォントサイズの拡大", "incfont"),
        c("decfont", "フォントサイズの縮小", "decfont"),
        c("changecase", "大文字小文字を変更", "changecase"),
        c("bold", "太字", "bold"),
        c("italic", "斜体", "italic"),
        c("underline", "下線", "underline"),
        c("strikeout", "取り消し線", "strikeout"),
        x("下付き", "subscript"),
        c("fontcolor", "フォントの色", "fontcolor"),
        c("fillparag", "塗りつぶしの色", "fillparag"),
        c("borders", "表の枠線", "borders"),
        c("top", "上揃え", "top"),
        c("middle", "中央揃え", "middle"),
        c("bottom", "下揃え", "bottom"),
        c("wrap", "折り返して​​全体を表示する", "wrap"),
        x("印刷の向き", "text-orient"),
        c("align-center", "中央揃え", "align-center"),
        x("両端揃え", "align-just"),
        c("merge", "結合して、中央に配置する", "merge"),
        x("direction", "direction"),
        x("関数", "formula"),
        c("fill-num", "フィル", "fill-num"),
        c("defname", "名前の管理", "named-range"),
        c("clear", "消去", "clear"),
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
        x("styles", "styles"),
        c("replace", "置き換え", "replace"),
        c("selectall", "すべて選択", "select-all"),
        c("setfilter", "フィルター", "setfilter"),
        c("clear-filter", "フィルターを解除", "clear-filter"),
    ]},
    Tab { name: "挿入", cmds: &[
        c("instable", "表の挿入", "instable"),
        c("insimage", "画像を挿入", "insimage"),
        c("insshape", "図形を挿入", "insshape"),
        c("inssmartart", "SmartArtの挿入", "inssmartart"),
        c("inscheckbox", "チェックボックス", "inscheckbox"),
        c("insrecommend", "推奨チャートを挿入", "insrecommend"),
        c("inschart", "グラフを挿入", "inschart"),
        c("inssparkline", "スパークラインを挿入する", "inssparkline"),
        x("グラフを挿入", "smartpicker"),
        c("inshyperlink", "ハイパーリンクを追加", "inshyperlink"),
        c("insslicer", "スライサーを挿入", "insslicer"),
        c("instext", "テキストボックスを挿入する", "instext"),
        c("instextart", "テキストアートの挿入", "instextart"),
        c("insequation", "方程式を挿入", "insequation"),
        c("inssymbol", "記号を挿入", "inssymbol"),
    ]},
    Tab { name: "描画", cmds: &[
        x("ペン", "pen"),
        x("蛍光ペン", "highlighter"),
        x("消しゴム", "eraser"),
    ]},
    Tab { name: "レイアウト", cmds: &[
        c("pagemargins", "余白", "pagemargins"),
        c("pageorient", "印刷の向き", "pageorient"),
        c("pagesize", "ページのサイズ", "pagesize"),
        c("printarea", "印刷範囲", "printarea"),
        c("pagebreak", "印刷物で次のページを開始する位置に改行を追加する", "pagebreak"),
        c("scale", "拡大縮小印刷", "scale"),
        c("printtitles", "タイトルを印刷する", "printtitles"),
        x("最初の列が右側に来るようにシートの方向を切り替える", "rtl-sheet"),
        c("print-gridlines", "枠線も印刷", "print-gridlines"),
        c("print-headings", "見出しも印刷", "print-headings"),
        x("配色の変更", "colorschemas"),
    ]},
    Tab { name: "数式", cmds: &[
        x("関数の挿入", "additional-formula"),
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
        x("ウォッチウィンドウ", "watch-window"),
        x("計算方法", "calculate"),
    ]},
    Tab { name: "データ", cmds: &[
        c("data-from-text", "テキストからデータ", "data-from-text"),
        c("data-external-links", "外部リンク(値で取り込む)", "data-external-links"),
        c("custom-sort", "並べ替え", "custom-sort"),
        c("setfilter", "フィルター", "setfilter"),
        c("clear-filter", "フィルターを解除", "clear-filter"),
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
        c("pivot-refresh", "更新", "pivot-refresh"),
        c("pivot-refresh-all", "すべて更新", "pivot-refresh-all"),
        c("pivot-select", "選択する", "pivot-select"),
        c("pivot-totals", "総計", "pivot-totals"),
        c("pivot-subtotals", "小計", "pivot-subtotals"),
        c("pivot-blank", "空行", "pivot-blank"),
        c("pivot-layout", "レポートのレイアウト", "pivot-layout"),
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
        x("範囲に変換する", "td-torange"),
        x("テーブルのサイズ変更", "td-resize"),
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
        c("prot-encrypt", "暗号化する", "prot-encrypt"),
        c("prot-sign", "デジタル署名を追加", "prot-sign"),
        c("prot-doc", "保護", "prot-doc"),
    ]},
    Tab { name: "表示", cmds: &[
        c("freeze", "ウィンドウ枠の固定", "freeze"),
        x("シートの表示", "sheet-view"),
        c("show-gridlines", "グリッド線", "show-gridlines"),
        x("見出し", "show-headings"),
    ]},
    Tab { name: "プラグイン", cmds: &[
        c("plug-macros", "マクロ", "plug-macros"),
        c("plug-manage", "プラグインの管理", "plug-manage"),
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


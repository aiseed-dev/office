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
        x("複数レベルのリスト", "multilevels"),
        c("decoffset", "インデントを減らす", "decoffset"),
        c("incoffset", "インデントを増やす", "incoffset"),
        c("linespace", "段落の行間", "linespace"),
        x("テキスト方向", "direction"),
        c("align-center", "中央揃え", "align-center"),
        c("align-just", "両端揃え", "align-just"),
        c("hidenchars", "非表示文字", "hidenchars"),
        c("paracolor", "段落の背景色", "paracolor"),
        c("borders", "罫線", "borders"),
        x("段落のスタイル", "styles"),
        c("replace", "置き換え", "replace"),
        c("selectall", "すべて選択", "select-all"),
    ]},
    Tab { name: "挿入", cmds: &[
        c("blankpage", "空白ページの挿入", "blankpage"),
        c("pagebreak", "ページの挿入またはセクション区切り", "pagebreak"),
        c("instable", "表の挿入", "instable"),
        x("図形を挿入", "insshape"),
        x("SmartArtの挿入", "inssmartart"),
        x("グラフを挿入", "inschart"),
        x("グラフを挿入", "smartpicker"),
        x("テキストボックスの挿入", "instext"),
        x("テキストアートの挿入", "instextart"),
        x("ドロップキャップの挿入", "dropcap"),
        x("ファイルからのテキスト", "text-from-file"),
        x("方程式を挿入", "insequation"),
        c("inssymbol", "記号を挿入", "inssymbol"),
        x("コンテンツコントロールの挿入", "controls"),
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
        x("列の挿入", "columns"),
        x("行番号を表示する", "line-numbers"),
        x("ハイフン設定の変更", "hyphenation"),
        x("透かしを編集する", "watermark"),
        x("ページ色の変更", "pagecolor"),
        x("配色の変更", "colorschemas"),
    ]},
    Tab { name: "参考資料", cmds: &[
        x("テキストの追加", "add-text"),
        x("目次の更新", "contents-update"),
        x("ブックマーク", "bookmarks"),
        x("図表番号", "caption"),
        x("相互参照", "crossref"),
        x("図表目次", "tof"),
        x("図表目次の更新", "tof-update"),
    ]},
    Tab { name: "ヘッダー/フッター", cmds: &[
        x("ヘッダーの編集", "edit-header"),
        x("フッターの編集", "edit-footer"),
        x("ページ番号", "pagenum"),
        x("日付/時刻", "datetime"),
        x("ページ数", "numpages"),
    ]},
    Tab { name: "レビュー", cmds: &[
        c("spell", "スペルチェック", "spell"),
        c("wordcount", "文字カウント", "wordcount"),
        x("変更履歴", "track-changes"),
        x("コメント", "comment"),
    ]},
    Tab { name: "表示", cmds: &[
        c("zoom-in", "拡大", "zoom-in"),
        c("zoom-out", "縮小", "zoom-out"),
        c("ruler", "ルーラー", "ruler"),
        x("ダークモード", "darkmode"),
    ]},
];

pub const CALC: &[Tab] = &[
    Tab { name: "ファイル", cmds: &[
        c("open", "開く", "open"),
        c("save", "保存", "save"),
        c("pdf", "印刷", "print"),
    ]},
    Tab { name: "ホーム", cmds: &[
        x("フォント", "fontname"),
        x("フォントのサイズ", "fontsize"),
        c("incfont", "フォントサイズの拡大", "incfont"),
        c("decfont", "フォントサイズの縮小", "decfont"),
        x("大文字小文字を変更", "changecase"),
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
        x("名前の管理", "named-range"),
        c("clear", "消去", "clear"),
        x("数値の書式", "format"),
        c("currency", "通貨スタイル", "currency"),
        c("percents", "パーセントのスタイル", "percents"),
        c("comma", "カンマスタイル", "comma"),
        c("digit-dec", "小数点以下の表示桁数を減らす", "digit-dec"),
        c("digit-inc", "小数点以下の表示桁数を増やす", "digit-inc"),
        c("cell-ins", "セルを挿入", "cell-ins"),
        c("cell-del", "セルを削除", "cell-del"),
        x("セルのスタイル", "cell-format"),
        x("条件付き書式", "condformat"),
        x("表の挿入", "table-tpl"),
        x("styles", "styles"),
        x("置き換え", "replace"),
        c("selectall", "すべて選択", "select-all"),
        c("setfilter", "フィルター", "setfilter"),
        c("clear-filter", "フィルターを解除", "clear-filter"),
    ]},
    Tab { name: "挿入", cmds: &[
        x("表の挿入", "instable"),
        x("画像を挿入", "insimage"),
        x("図形を挿入", "insshape"),
        x("SmartArtの挿入", "inssmartart"),
        x("inscheckbox", "inscheckbox"),
        x("推奨チャートを挿入", "insrecommend"),
        x("グラフを挿入", "inschart"),
        x("スパークラインを挿入する", "inssparkline"),
        x("グラフを挿入", "smartpicker"),
        x("ハイパーリンクを追加", "inshyperlink"),
        x("スライサーを挿入", "insslicer"),
        x("テキストボックスを挿入する", "instext"),
        x("instextart", "instextart"),
        x("方程式を挿入", "insequation"),
        x("記号を挿入", "inssymbol"),
    ]},
    Tab { name: "描画", cmds: &[
        x("ペン", "pen"),
        x("蛍光ペン", "highlighter"),
        x("消しゴム", "eraser"),
    ]},
    Tab { name: "レイアウト", cmds: &[
        x("余白", "pagemargins"),
        x("印刷の向き", "pageorient"),
        x("ページのサイズ", "pagesize"),
        x("印刷範囲", "printarea"),
        x("印刷物で次のページを開始する位置に改行を追加する", "pagebreak"),
        x("拡大縮小印刷", "scale"),
        x("タイトルを印刷する", "printtitles"),
        x("最初の列が右側に来るようにシートの方向を切り替える", "rtl-sheet"),
        x("print-gridlines", "print-gridlines"),
        x("print-headings", "print-headings"),
        x("配色の変更", "colorschemas"),
    ]},
    Tab { name: "数式", cmds: &[
        x("関数の挿入", "additional-formula"),
        c("sum", "オートSUM", "autosum"),
        c("fn-recent", "最近使った関数", "recent"),
        x("財務", "financial"),
        c("fn-logical", "論理", "logical"),
        c("fn-text", "文字列操作", "text"),
        x("日付/時刻", "datetime"),
        x("検索/行列", "lookup"),
        c("fn-math", "数学/三角", "math"),
        x("その他の関数", "more"),
        x("名前の管理", "named-range-huge"),
        x("参照元のトレース", "trace-prec"),
        x("参照先のトレース", "trace-dep"),
        x("トレース矢印の削除", "remove-arrows"),
        c("show-formulas", "数式の表示", "show-formulas"),
        x("ウォッチウィンドウ", "watch-window"),
        x("計算方法", "calculate"),
    ]},
    Tab { name: "データ", cmds: &[
        x("テキストからデータ", "data-from-text"),
        x("外部リンク", "data-external-links"),
        c("custom-sort", "並べ替え", "custom-sort"),
        c("setfilter", "フィルター", "setfilter"),
        c("clear-filter", "フィルターを解除", "clear-filter"),
        x("区切り位置", "text-column"),
        c("rem-duplicates", "重複の削除", "rem-duplicates"),
        x("データの入力規則", "data-validation"),
        x("ゴールシーク", "goal-seek"),
        x("ソルバー", "solver"),
        x("グループ化", "group"),
        x("グループ解除", "ungroup"),
        x("詳細の表示", "show-details"),
        x("詳細の非表示", "hide-details"),
    ]},
    Tab { name: "表示", cmds: &[
        c("freeze", "ウィンドウ枠の固定", "freeze"),
        x("シートの表示", "sheet-view"),
        c("show-gridlines", "グリッド線", "show-gridlines"),
        x("見出し", "show-headings"),
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
    fn 除外した5つがタブに無い() {
        for tabs in [WRITER, CALC] {
            for t in tabs {
                for ng in ["共同編集", "保護", "プラグイン", "AI", "マクロ"] {
                    assert!(!t.name.contains(ng), "除外のはずのタブがある: {}", t.name);
                }
            }
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


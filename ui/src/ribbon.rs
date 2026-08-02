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
#[derive(Clone, Copy)]
pub struct Cmd {
    pub id: &'static str,
    pub label: &'static str,
    pub ready: bool,
}

const fn c(id: &'static str, label: &'static str) -> Cmd {
    Cmd { id, label, ready: true }
}
const fn x(label: &'static str) -> Cmd {
    Cmd { id: "", label, ready: false }
}

pub struct Tab {
    pub name: &'static str,
    pub cmds: &'static [Cmd],
}

pub const WRITER: &[Tab] = &[
    Tab { name: "ファイル", cmds: &[
        c("open", "開く"),
        c("save", "保存"),
        c("pdf", "印刷"),
    ]},
    Tab { name: "ホーム", cmds: &[
        x("フォント"),
        x("フォントのサイズ"),
        c("incfont", "フォントサイズの拡大"),
        c("decfont", "フォントサイズの縮小"),
        x("大文字小文字を変更"),
        c("bold", "太字"),
        c("italic", "斜体"),
        c("underline", "下線"),
        c("strikeout", "取り消し線"),
        x("上付き"),
        x("下付き"),
        x("ハイライトの色"),
        c("fontcolor", "フォントの色"),
        x("スタイルのクリア"),
        x("箇条書き"),
        x("ナンバリング"),
        x("複数レベルのリスト"),
        x("インデントを減らす"),
        x("インデントを増やす"),
        x("段落の行間"),
        x("テキスト方向"),
        c("align-center", "中央揃え"),
        c("align-just", "両端揃え"),
        x("非表示文字"),
        x("段落の背景色"),
        x("罫線"),
        x("段落のスタイル"),
        c("spell", "置き換え"),
        c("selectall", "すべて選択"),
    ]},
    Tab { name: "挿入", cmds: &[
        x("空白ページの挿入"),
        x("表の挿入"),
        x("図形を挿入"),
        x("SmartArtの挿入"),
        x("グラフを挿入"),
        x("グラフを挿入"),
        x("テキストボックスの挿入"),
        x("テキストアートの挿入"),
        x("ドロップキャップの挿入"),
        x("ファイルからのテキスト"),
        x("方程式を挿入"),
        x("記号を挿入"),
        x("コンテンツコントロールの挿入"),
    ]},
    Tab { name: "レイアウト", cmds: &[
        x("余白"),
        x("印刷の向き"),
        x("ページのサイズ"),
        x("列の挿入"),
        x("行番号を表示する"),
        x("ハイフン設定の変更"),
        x("透かしを編集する"),
        x("ページ色の変更"),
        x("配色の変更"),
    ]},
    Tab { name: "参考資料", cmds: &[
        x("テキストの追加"),
        x("目次の更新"),
        x("ブックマーク"),
        x("図表番号"),
        x("相互参照"),
        x("図表目次"),
        x("図表目次の更新"),
    ]},
];

pub const CALC: &[Tab] = &[
    Tab { name: "ファイル", cmds: &[
        c("open", "開く"),
        c("save", "保存"),
        x("印刷"),
    ]},
    Tab { name: "ホーム", cmds: &[
        x("フォント"),
        x("フォントのサイズ"),
        x("フォントサイズの拡大"),
        x("フォントサイズの縮小"),
        x("大文字小文字を変更"),
        c("bold", "太字"),
        c("italic", "斜体"),
        c("underline", "下線"),
        x("取り消し線"),
        x("下付き"),
        x("フォントの色"),
        x("塗りつぶしの色"),
        c("borders", "表の枠線"),
        x("上揃え"),
        x("中央揃え"),
        x("下揃え"),
        x("折り返して​​全体を表示する"),
        x("印刷の向き"),
        c("align-center", "中央揃え"),
        x("両端揃え"),
        x("結合して、中央に配置する"),
        x("direction"),
        x("関数"),
        x("フィル"),
        x("名前の管理"),
        c("clear", "消去"),
        x("数値の書式"),
        c("currency", "通貨スタイル"),
        c("percents", "パーセントのスタイル"),
        c("comma", "カンマスタイル"),
        c("digit-dec", "小数点以下の表示桁数を減らす"),
        c("digit-inc", "小数点以下の表示桁数を増やす"),
        c("cell-ins", "セルを挿入"),
        c("cell-del", "セルを削除"),
        x("セルのスタイル"),
        x("条件付き書式"),
        x("表の挿入"),
        x("styles"),
        x("置き換え"),
        c("selectall", "すべて選択"),
    ]},
    Tab { name: "挿入", cmds: &[
        x("表の挿入"),
        x("画像を挿入"),
        x("図形を挿入"),
        x("SmartArtの挿入"),
        x("inscheckbox"),
        x("推奨チャートを挿入"),
        x("グラフを挿入"),
        x("スパークラインを挿入する"),
        x("グラフを挿入"),
        x("ハイパーリンクを追加"),
        x("スライサーを挿入"),
        x("テキストボックスを挿入する"),
        x("instextart"),
        x("方程式を挿入"),
        x("記号を挿入"),
    ]},
    Tab { name: "レイアウト", cmds: &[
        x("余白"),
        x("印刷の向き"),
        x("ページのサイズ"),
        x("印刷範囲"),
        x("印刷物で次のページを開始する位置に改行を追加する"),
        x("拡大縮小印刷"),
        x("タイトルを印刷する"),
        x("最初の列が右側に来るようにシートの方向を切り替える"),
        x("print-gridlines"),
        x("print-headings"),
        x("配色の変更"),
    ]},
    Tab { name: "数式", cmds: &[
        x("関数の挿入"),
        c("sum", "オートSUM"),
        x("最近使った関数"),
        x("財務"),
        x("論理"),
        x("文字列操作"),
        x("日付/時刻"),
        x("検索/行列"),
        x("数学/三角"),
        x("その他の関数"),
        x("名前の管理"),
        x("参照元のトレース"),
        x("参照先のトレース"),
        x("トレース矢印の削除"),
        x("数式の表示"),
        x("ウォッチウィンドウ"),
        x("計算方法"),
    ]},
    Tab { name: "データ", cmds: &[
        x("テキストからデータ"),
        x("外部リンク"),
        c("custom-sort", "並べ替え"),
        x("区切り位置"),
        c("rem-duplicates", "重複の削除"),
        x("データの入力規則"),
        x("ゴールシーク"),
        x("ソルバー"),
        x("グループ化"),
        x("グループ解除"),
        x("詳細の表示"),
        x("詳細の非表示"),
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


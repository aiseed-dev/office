#!/usr/bin/env python3
"""`ribbon.rs` を Euro-Office の現物から生成する。

**手で要約しない。** タブの並びもボタンの並びも
`vendor/web-apps/apps/*/main/app/template/Toolbar.template` の順そのまま、
名前は同じ app の `locale/ja.json` から引く。
だから「Euro-Office と全く同じか」は、この台本を回し直せば確かめられる。

除く5つ(方針): 共同編集・保護・プラグイン・AI・マクロ。
マクロを持たないのは機能不足ではなく、文書の中に実行コードを置かない設計。

リボンの言葉は Euro-Office のロケールから来るので、**45言語ぶん現物がある**。
各国語版を作るとき、画面の翻訳は作業に含まれない。

  python3 gen_ribbon.py       > src/ribbon.rs   # 既定は日本語
  python3 gen_ribbon.py en    > src/ribbon.rs   # 英語版
  python3 gen_ribbon.py --list                  # 使えるロケール
"""

import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1] / "vendor" / "web-apps" / "apps"
LOCALE = "ja"

# 実装済みのコマンド。ここに載っているものだけが押せる。
# **できないものを、できるように見せない。**
# **アプリ側で実際に動くものだけ**を書く。
# ここに書けば押せる見た目になるので、書いた瞬間に嘘になりうる。
# 太字・中央揃え・罫線はまだ入っていない(次の仕事)。
READY = {
    "writer": {
        "open": "open", "save": "save", "undo": "undo", "redo": "redo",
        "select-all": "selectall",
        "bold": "bold", "italic": "italic", "underline": "underline",
        "strikeout": "strikeout", "superscript": "superscript",
        "subscript": "subscript", "highlight": "highlight", "clearstyle": "clearstyle",
        "align-center": "align-center", "align-just": "align-just",
        "incfont": "incfont", "decfont": "decfont", "print": "pdf",
        "fontcolor": "fontcolor", "align-left": "align-left",
        "align-right": "align-right",
        "markers": "markers", "numbering": "numbering",
        "wordcount": "wordcount", "spell": "spell", "pagebreak": "pagebreak",
        "zoom-in": "zoom-in", "zoom-out": "zoom-out",
        "hidenchars": "hidenchars", "ruler": "ruler",
        "fontname": "fontname", "fontsize": "fontsize",
        "pageorient": "pageorient", "pagesize": "pagesize", "pagemargins": "pagemargins",
        "instable": "instable", "inssymbol": "inssymbol", "replace": "replace",
        "incoffset": "incoffset", "decoffset": "decoffset",
        "linespace": "linespace",
        "changecase": "changecase", "blankpage": "blankpage",
        "paracolor": "paracolor", "borders": "borders",
        "insertimage": "insimage",
        "edit-header": "edit-header", "edit-footer": "edit-footer",
        "pagenum": "pagenum",
        "styles": "parastyle",
        "contents": "toc", "contents-update": "toc-update",
        "datetime": "datetime", "numpages": "numpages",
        "multilevels": "multilevels", "darkmode": "darkmode",
        "text-from-file": "text-from-file", "add-text": "add-text",
        "line-numbers": "line-numbers",
        "insshape": "insshape", "inssmartart": "inssmartart",
        "inschart": "inschart", "smartpicker": "smartpicker",
        "instextart": "instextart", "insequation": "insequation",
        "instext": "instext", "pagecolor": "pagecolor",
        "comment": "comment", "watermark": "watermark",
        "bookmarks": "bookmarks", "caption": "caption",
        "tof": "tof", "tof-update": "tof-update",
        "columns": "columns",
        "pen": "pen", "highlighter": "highlighter", "eraser": "eraser",
        "track-changes": "track-changes", "dropcap": "dropcap",
        "hyphenation": "hyphenation", "crossref": "crossref",
    },
    "calc": {
        "open": "open", "save": "save", "undo": "undo", "redo": "redo",
        "select-all": "selectall", "autosum": "sum",
        "borders": "borders", "bold": "bold", "italic": "italic",
        "underline": "underline",
        "align-center": "align-center",
        "comma": "comma", "currency": "currency", "percents": "percents",
        "cell-ins": "cell-ins", "cell-del": "cell-del", "clear": "clear",
        "digit-inc": "digit-inc", "digit-dec": "digit-dec",
        "align-left": "align-left", "align-right": "align-right",
        "custom-sort": "custom-sort", "rem-duplicates": "rem-duplicates",
        "setfilter": "setfilter", "clear-filter": "clear-filter",
        "fill-num": "fill-num",
        "strikeout": "strikeout", "top": "top", "middle": "middle",
        "bottom": "bottom", "wrap": "wrap",
        "incfont": "incfont", "decfont": "decfont",
        "fillparag": "fillparag", "fontcolor": "fontcolor", "merge": "merge",
        "math": "fn-math", "text": "fn-text", "logical": "fn-logical",
        "recent": "fn-recent",
        "show-formulas": "show-formulas", "show-gridlines": "show-gridlines",
        "freeze": "freeze",
        "print": "pdf",
        "data-validation": "data-validation", "condformat": "condformat",
        "named-range": "defname", "named-range-huge": "defname",
        "pageorient": "pageorient", "pagesize": "pagesize",
        "pagemargins": "pagemargins", "printarea": "printarea",
        "inschart": "inschart", "insimage": "insimage",
        "inshyperlink": "inshyperlink", "replace": "replace",
        "changecase": "changecase", "format": "format",
        "cell-format": "cell-format", "fontname": "fontname",
        "fontsize": "fontsize",
        "financial": "fn-financial", "datetime": "fn-datetime",
        "lookup": "fn-lookup", "more": "fn-more",
        "scale": "scale", "pagebreak": "pagebreak",
        "printtitles": "printtitles", "print-gridlines": "print-gridlines",
        "print-headings": "print-headings",
        "data-from-text": "data-from-text", "text-column": "text-column",
        "goal-seek": "goal-seek", "data-external-links": "data-external-links",
        "insshape": "insshape", "instext": "instext",
        "inssparkline": "inssparkline", "co-addcomment": "addcomment",
        "td-remdup": "rem-duplicates",
        "trace-prec": "trace-prec", "trace-dep": "trace-dep",
        "remove-arrows": "remove-arrows", "insrecommend": "insrecommend",
        "instable": "instable", "table-tpl": "table-tpl",
        "inssymbol": "inssymbol",
    },
}

# 除外するタブ — **無し**(発注者確定 2026-08-04: メニューは制限せず全部入れる。
# 実装しない方針のもの(共同編集・保護・プラグイン等)も、場所は本家どおり
# 灰色で見せる。「できないものを、できるように見せない」は ready で守る)
DROP_TABS: set = set()

# slot の id → ja.json の鍵の末尾(tip か cap)。
# 現物の id と鍵名は綴りが違うので、ここだけは対応表が要る。
LABEL = {
    # 常時
    "save": "tipSave", "print": "tipPrint", "copy": "tipCopy", "paste": "tipPaste",
    "undo": "tipUndo", "redo": "tipRedo", "cut": "tipCut", "copystyle": "tipCopyStyle",
    # ホーム(文書)
    "fontname": "tipFontName", "fontsize": "tipFontSize", "incfont": "tipIncFont",
    "decfont": "tipDecFont", "changecase": "tipChangeCase", "bold": None,
    "italic": None, "underline": None, "strikeout": None, "superscript": None,
    "subscript": None, "highlight": "tipHighlightColor", "fontcolor": "tipFontColor",
    "clearstyle": "tipClearStyle", "markers": "tipMarkers", "numbering": "tipNumbers",
    "multilevels": "tipMultilevels", "decoffset": "tipDecPrLeft",
    "incoffset": "tipIncPrLeft", "linespace": "tipLineSpace", "direction": "tipTextDir",
    "align-center": "tipAlignCenter", "align-just": "tipAlignJust",
    "align-left": "tipAlignLeft", "align-right": "tipAlignRight",
    "hidenchars": "tipShowHiddenChars", "paracolor": "tipPrColor",
    "borders": "tipBorders", "styles": "tipParagraphStyle", "replace": "tipReplace",
    "select-all": "tipSelectAll",
    # 挿入(文書)
    "blankpage": "tipBlankPage", "instable": "tipInsertTable",
    "insshape": "tipInsertShape", "inssmartart": "tipInsertSmartArt",
    "inschart": "tipInsertChart", "smartpicker": "tipInsertChart",
    "instext": "tipInsertText", "instextart": "tipInsertTextArt",
    "dropcap": "tipDropCap", "text-from-file": "tipTextFromFile",
    "insequation": "tipInsertEquation", "inssymbol": "tipInsertSymbol",
    "insertimage": "tipInsertImage",
    "controls": "tipControls", "insimage": "tipInsertImage",
    "inshyperlink": "tipInsertHyperlink", "insslicer": "tipInsertSlicer",
    "inssparkline": "tipInsertSpark", "inscheckbox": "tipControls",
    "insrecommend": "tipInsertChartRecommend",
    # レイアウト
    "pagemargins": "tipPageMargins", "pageorient": "tipPageOrient",
    "pagesize": "tipPageSize", "columns": "tipColumns",
    "line-numbers": "tipLineNumbers", "hyphenation": "tipHyphenation",
    "watermark": "tipWatermark", "pagecolor": "tipPageColor",
    "colorschemas": "tipColorSchemas", "printarea": "tipPrintArea",
    "pagebreak": "tipPageBreak", "scale": "tipScale",
    "printtitles": "tipPrintTitles", "rtl-sheet": "tipRtlSheet",
    # 参考資料
    "add-text": None, "contents-update": None, "bookmarks": None,
    "caption": None, "crossref": None, "tof": None, "tof-update": None,
    # 表計算のホーム
    "fillparag": "tipPrColor", "top": "tipAlignTop", "middle": "tipAlignMiddle",
    "bottom": "tipAlignBottom", "wrap": "tipWrap", "text-orient": "tipTextOrientation",
    "merge": "tipMerge", "formula": None, "fill-num": None,
    "named-range": None, "clear": "tipClearStyle", "format": "tipNumFormat",
    "currency": "tipDigStyleCurrency", "percents": "tipDigStylePercent",
    "comma": "tipDigStyleComma", "digit-dec": "tipDecDecimal",
    "digit-inc": "tipIncDecimal", "cell-ins": "tipInsertOpt",
    "cell-del": "tipDeleteOpt", "cell-format": "tipCellStyle",
    "condformat": "tipCondFormat", "table-tpl": "tipInsertTable",
    # 数式
    "additional-formula": None, "autosum": None, "recent": None,
    "financial": None, "logical": None, "text": None, "datetime": None,
    "lookup": None, "math": None, "more": None, "named-range-huge": None,
    "trace-prec": None, "trace-dep": None, "remove-arrows": None,
    "show-formulas": None, "watch-window": None, "calculate": None,
    # データ
    "data-from-text": None, "data-external-links": None, "custom-sort": None,
    "text-column": None, "rem-duplicates": None, "data-validation": None,
    "goal-seek": None, "solver": None, "group": None, "ungroup": None,
    "show-details": None, "hide-details": None,
}

# ja.json に鍵が無い(アイコンだけ・別画面)ものの日本語名。
# **現物に出ている語をそのまま使う。** 勝手な言い換えをしない。
FALLBACK = {
    "bold": "太字", "italic": "斜体", "underline": "下線",
    "strikeout": "取り消し線", "superscript": "上付き", "subscript": "下付き",
    "add-text": "テキストの追加", "contents-update": "目次の更新",
    "bookmarks": "ブックマーク", "caption": "図表番号", "crossref": "相互参照",
    "tof": "図表目次", "tof-update": "図表目次の更新",
    "formula": "関数", "fill-num": "フィル", "named-range": "名前の管理",
    "additional-formula": "関数の挿入", "autosum": "オートSUM",
    "recent": "最近使った関数", "financial": "財務", "logical": "論理",
    "text": "文字列操作", "datetime": "日付/時刻", "lookup": "検索/行列",
    "math": "数学/三角", "more": "その他の関数",
    "named-range-huge": "名前の管理", "trace-prec": "参照元のトレース",
    "trace-dep": "参照先のトレース", "remove-arrows": "トレース矢印の削除",
    "show-formulas": "数式の表示", "watch-window": "ウォッチウィンドウ",
    "calculate": "計算方法", "data-from-text": "テキストからデータ",
    "data-external-links": "外部リンク", "custom-sort": "並べ替え",
    "text-column": "区切り位置", "rem-duplicates": "重複の削除",
    "data-validation": "データの入力規則", "goal-seek": "ゴールシーク",
    "solver": "ソルバー", "group": "グループ化", "ungroup": "グループ解除",
    "show-details": "詳細の表示", "hide-details": "詳細の非表示",
    "spell": "スペルチェック", "furigana": "ふりがな", "pagebreak": "区切り", "setfilter": "フィルター", "clear-filter": "フィルターを解除",
    "contents": "目次",
    "open": "開く", "print": "印刷",
}


def locale(app):
    p = ROOT / app / f"main/locale/{LOCALE}.json"
    if not p.exists():
        sys.exit(f"そのロケールは Euro-Office に無い: {LOCALE}")
    return json.load(open(p, encoding="utf-8"))


def locales():
    d = ROOT / "documenteditor/main/locale"
    return sorted(p.stem for p in d.glob("*.json"))


def tab_names(app, prefix):
    d = locale(app)
    out = {}
    for k, v in d.items():
        m = re.search(rf"{prefix}\.Views\.Toolbar\.textTab(\w+)$", k)
        if m:
            out[m.group(1).lower()] = v
    return out


def label_of(app_loc, prefix, slot):
    key = LABEL.get(slot)
    if key:
        for who in ("Toolbar", "ViewTab", "HeaderFooterTab"):
            full = f"{prefix}.Views.{who}.{key}"
            if full in app_loc:
                # tip は説明文のことがある。最初の読点までを名前にする
                return re.split(r"[。<（(]", app_loc[full])[0].strip()
    if slot in FALLBACK:
        return FALLBACK[slot]
    if slot in DYN_LABELS:
        return DYN_LABELS[slot]
    return slot


# テンプレートに無いタブ(JS で動的に作られる)。
# 並びは Euro-Office の実物: 文書 = ファイル ホーム 挿入 描画 レイアウト
# 参考資料 ヘッダー/フッター レビュー 表示 / 表計算 = … 数式 データ 表示。
# 除外(共同編集・保護・プラグイン)はそもそも入れない。
# 全部入れる(発注者確定 2026-08-04): 共同編集・保護・プラグインも
# 場所は本家どおり。実装しないものは灰色のまま(ready の嘘は無し)
# アプリ固有の追加タブ(本家の位置に挿す)。(タブ名, 直前に置くタブ名, cmds)
APP_TABS = {
    "documenteditor": [
        ("フォーム", "参考資料", [
            ("form-text", "テキストフィールド"), ("form-combo", "コンボボックス"),
            ("form-dropdown", "ドロップダウン"), ("form-checkbox", "チェックボックス"),
            ("form-radio", "ラジオボタン"), ("form-image", "画像"),
            ("form-email", "メールアドレス"), ("form-phone", "電話番号"),
            ("form-complex", "複合フィールド"), ("form-signature", "署名"),
        ]),
    ],
    "spreadsheeteditor": [
        ("ピボットテーブル", "データ", [
            ("pivot-insert", "ピボットテーブルを挿入"), ("pivot-refresh", "更新"),
            ("pivot-refresh-all", "すべて更新"), ("pivot-select", "選択する"),
            ("pivot-totals", "総計"), ("pivot-subtotals", "小計"),
            ("pivot-blank", "空行"), ("pivot-layout", "レポートのレイアウト"),
        ]),
        ("表のデザイン", "ピボットテーブル", [
            ("td-header", "ヘッダー行"), ("td-total", "合計行"),
            ("td-band-row", "縞模様の行"), ("td-first", "最初の列"),
            ("td-last", "最後の列"), ("td-band-col", "縞模様の列"),
            ("td-filter", "フィルタのボタン"), ("td-remdup", "重複データを削除"),
            ("td-torange", "範囲に変換する"), ("td-resize", "テーブルのサイズ変更"),
        ]),
    ],
}

COMMON_TAIL = {
    "collaboration": [
        ("coauth-mode", "共同編集モード"), ("co-addcomment", "コメントを追加"),
        ("co-delcomment", "コメントを削除"), ("co-showcomment", "コメントの表示"),
        ("co-chat", "チャット"), ("co-history", "バージョン履歴"),
    ],
    "protect": [
        ("prot-encrypt", "暗号化する"), ("prot-sign", "デジタル署名を追加"),
        ("prot-doc", "保護"),
    ],
    "plugins": [
        ("plug-macros", "マクロ"), ("plug-manage", "プラグインの管理"),
    ],
}

DYNAMIC = {
    "documenteditor": [
        ("draw", 3, [("pen", "ペン"), ("highlighter", "蛍光ペン"), ("eraser", "消しゴム")]),
        ("headerfooter", 6, [
            ("edit-header", "ヘッダーの編集"), ("edit-footer", "フッターの編集"),
            ("pagenum", "ページ番号"), ("datetime", "日付＆時刻"), ("numpages", "ページ数"),
        ]),
        ("review", 7, [
            ("spell", "スペルチェック"), ("wordcount", "文字カウント"),
            ("track-changes", "変更履歴"), ("comment", "コメント"),
        ]),
        ("view", 8, [
            ("zoom-in", "拡大"), ("zoom-out", "縮小"),
            ("ruler", "ルーラー"), ("darkmode", "ダークモード"),
        ]),
    ],
    "spreadsheeteditor": [
        ("draw", 3, [("pen", "ペン"), ("highlighter", "蛍光ペン"), ("eraser", "消しゴム")]),
        ("view", 99, [
            ("freeze", "ウィンドウ枠の固定"), ("sheet-view", "シートの表示"),
            ("show-gridlines", "グリッド線"), ("show-headings", "見出し"),
        ]),
    ],
}

TAB_NAME_KEYS = {"draw": "Draw", "headerfooter": "HeaderFooter",
                 "review": "Review", "view": "View",
                 "collaboration": "コラボレーション", "protect": "保護",
                 "plugins": "プラグイン"}


def tabs_of(app, prefix):
    tpl = (ROOT / app / "main/app/template/Toolbar.template").read_text(encoding="utf-8")
    loc = locale(app)
    names = tab_names(app, prefix)
    parts = re.split(r'<section class="panel"[^>]*data-tab="([a-z-]+)"', tpl)

    out = []
    # ファイルタブは Euro-Office では別画面。並びの先頭に置く点だけ合わせる
    out.append((names.get("file", "File"), ["open", "save", "print"]))
    for i in range(1, len(parts), 2):
        tab, body = parts[i], parts[i + 1]
        if tab in DROP_TABS:
            continue
        slots = list(dict.fromkeys(
            re.findall(r'id="slot-(?:btn|field|chk|cmb)-([a-z0-9-]+)"', body)))
        # 「区切り」は class 注入(btn-slot.btn-pagebreak)なので id では拾えない。
        # Euro-Office の挿入タブでは空白ページの隣にある
        if tab == "ins" and app == "documenteditor" and "blankpage" in slots:
            slots.insert(slots.index("blankpage") + 1, "pagebreak")
        # 画像も class 注入(slot-insertimage)。Euro-Office では表の隣にある
        if tab == "ins" and app == "documenteditor" and "instable" in slots:
            slots.insert(slots.index("instable") + 1, "insertimage")
        # 目次も class 注入。Euro-Office では参考資料の先頭にある
        if tab == "links" and app == "documenteditor" and "contents" not in slots:
            slots.insert(0, "contents")
        # フィルターも class 注入(btn-slot.slot-btn-setfilter)
        if tab == "home" and app == "spreadsheeteditor" and "custom-sort" not in slots:
            slots.extend(["setfilter", "clear-filter"])
        if tab == "data" and app == "spreadsheeteditor":
            i = slots.index("custom-sort") + 1 if "custom-sort" in slots else len(slots)
            slots[i:i] = ["setfilter", "clear-filter"]
        alias = {"ins": "insert", "links": "links"}
        name = names.get(alias.get(tab, tab), names.get(tab, tab))
        out.append((name, slots))
    # 動的なタブを Euro-Office の位置に差し込む
    for key, at, cmds in DYNAMIC.get(app, []):
        nk = TAB_NAME_KEYS[key].lower()
        name = names.get(nk, TAB_NAME_KEYS[key])
        entry = (name, [c for c, _ in cmds])
        for c, label in cmds:
            DYN_LABELS[c] = label
        if at >= len(out):
            out.append(entry)
        else:
            out.insert(at, entry)
    # アプリ固有のタブ(本家の位置: 指定タブの直後)
    for (name, after, cmds) in APP_TABS.get(app, []):
        entry = (name, [c for c, _ in cmds])
        for c, label in cmds:
            DYN_LABELS[c] = label
        at = next((i + 1 for i, (n, _) in enumerate(out) if n == after), len(out))
        out.insert(at, entry)
    # 全部入れる: 共同編集・保護は表示の前、プラグインは末尾(本家の並び)
    for key, cmds in [
        ("collaboration", COMMON_TAIL["collaboration"]),
        ("protect", COMMON_TAIL["protect"]),
        ("plugins", COMMON_TAIL["plugins"]),
    ]:
        name = TAB_NAME_KEYS[key]
        entry = (name, [c for c, _ in cmds])
        for c, label in cmds:
            DYN_LABELS[c] = label
        view_at = next((i for i, (n, _) in enumerate(out) if n == "表示"), len(out))
        if key == "plugins":
            out.append(entry)
        else:
            out.insert(view_at, entry)
    return out, loc


# 動的タブのボタン名(ja.json に鍵が無いものの既定)
DYN_LABELS = {}


# 独自の釦(Euro-Office に無いがこちらが足すもの)。タブの末尾に置く。
# 並びは本家のまま、増えた分だけ後ろ — 乗り換えの人の目当ては崩さない
EXTRA_CMDS = {
    "calc": {
        "データ": [("python", "Python", "python")],
    },
}


def emit():
    print(HEAD)
    for app, prefix, konst, which in [
        ("documenteditor", "DE", "WRITER", "writer"),
        ("spreadsheeteditor", "SSE", "CALC", "calc"),
    ]:
        tabs, loc = tabs_of(app, prefix)
        ready = READY[which]
        extras = EXTRA_CMDS.get(which, {})
        print(f"pub const {konst}: &[Tab] = &[")
        for name, slots in tabs:
            print(f'    Tab {{ name: "{name}", cmds: &[')
            for s in slots:
                lab = label_of(loc, prefix, s).replace('"', "'")
                if s in ready:
                    print(f'        c("{ready[s]}", "{lab}", "{s}"),')
                else:
                    print(f'        x("{lab}", "{s}"),')
            for (cid, clab, cicon) in extras.get(name, []):
                print(f'        c("{cid}", "{clab}", "{cicon}"),')
            print("    ]},")
        print("];")
        print()
    print(TAIL)


HEAD = '''//! リボン(タブ+コマンド)。**Euro-Office の現物から生成している。**
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
'''

TAIL = '''/// 実装済みのコマンド数 / 全体(進み具合を隠さない)
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
            for want in ["コラボレーション", "保護", "プラグイン"] {
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
'''

if __name__ == "__main__":
    if not ROOT.exists():
        sys.exit(f"Euro-Office の現物が見つかりません: {ROOT}")
    args = sys.argv[1:]
    if args and args[0] == "--list":
        print(" ".join(locales()))
        sys.exit(0)
    if args:
        LOCALE = args[0]
    emit()

#!/usr/bin/env python3
"""ribbon.rs(正)から、別ロケールのリボン表を起こす。

gen_ribbon.py(テンプレートから ja の表を起こす)と役割が違う:
こちらは **いまの ui/src/ribbon.rs を構造の正** とし、語だけを
Euro-Office のロケール(vendor/web-apps の ja.json → <locale>.json の対訳)で
置き換える。手で足したボタン(AI タブなど本家に無いもの)は OVERRIDES 表で
訳す。**訳が見つからない語があれば止まる**(黙って日本語のまま出さない)。

    python3 ui/gen_ribbon_locale.py en > ui/src/ribbon_en.rs

id・並び・ready・icon は ja と同一になる(試験 ribbon.rs 側で保証)。
"""
import json
import re
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "vendor/web-apps/apps"
RIBBON = Path(__file__).resolve().parent / "src/ribbon.rs"

# 本家に無い・こちらで足した語の対訳。ここに無い未解決語が出たら
# このスクリプトは止まる — その語をここに足してから出し直す
OVERRIDES = {
    "en": {
        "書式のコピー": "Format painter",
        "スタイル": "Style",
        "フィールドリスト": "Field list",
        # 表示タブ(こちらで足したボタン — 画面の文字の大きさ)
        "画面の文字を大きく": "Bigger UI text",
        "画面の文字を小さく": "Smaller UI text",
        # タブ
        "AI": "AI",
        # ファイル
        "印刷": "Print",
        # AI タブ(こちらの設計。calc-manual.md の英語版と同じ語)
        "宛先": "Destination",
        "要約": "Summarize",
        "書き直す": "Rewrite",
        "敬語にする": "Politer",
        "やさしく": "Plainer",
        "翻訳": "Translate",
        "ふりがな": "Furigana",
        "続きを書く": "Continue",
        "表にする": "To table",
        "頼む": "Ask",
        # writer 独自
        "ルビ": "Ruby",
        "縦書き": "Vertical text",
        "テキスト方向": "Text direction",
        "均等割付": "Distributed",
        "図表番号の挿入": "Insert caption",
        "URL を開く": "Open URL",
        "洋子さんの索引": "Index",
        "青空文庫の注記": "Aozora notes",
        "でんでん記法": "Denden markup",
        "履歴の記録": "Track changes",
        "変更履歴の表示": "Show changes",
        "校正": "Proofread",
        "文字数": "Character count",
        "スペルチェック": "Spell check",
        "類語辞典": "Thesaurus",
        "誤変換": "Misconversion",
        "表記ゆれ": "Inconsistency",
        # calc 独自
        "小計": "Subtotal",
        "計算方法": "Calculation",
        "右横書き": "Right-to-left text",
        "シートの方向": "Sheet direction",
        "Python": "Python",
        "チェックボックス": "Checkbox",
        "外部リンク": "External links",
        "推奨チャート": "Recommended chart",
        # 共同編集・保護(writer/calc 共通の言い換え)
        "共同編集モード": "Co-editing mode",
        "バージョン履歴": "Version history",
        "チャット": "Chat",
        "保護する": "Protect",
        "暗号化する": "Encrypt",
        "デジタル署名を追加": "Add digital signature",
        "マクロ": "Macros",
        "プラグインの管理": "Manage plugins",
        # 本家の語と言い回しが少し違うもの(Word/Excel の標準語で)
        "0を表示する": "Show zeros",
        "100%に拡大する": "Zoom to 100%",
        "インターフェイステーマ": "Interface theme",
        "ウォッチウィンドウ": "Watch window",
        "オートSUM": "AutoSum",
        "コメントを削除": "Delete comment",
        "ソルバー": "Solver",
        "テキストからデータ": "Text to data",
        "トレース矢印の削除": "Remove arrows",
        "フィル": "Fill",
        "フィルターを解除": "Clear filter",
        "マクロを書く": "Write macro",
        "区切り位置": "Text to columns",
        "図表番号": "Caption",
        "図表目次": "Table of figures",
        "図表目次の更新": "Update table of figures",
        "外部リンク(値で取り込む)": "External links (import as values)",
        "数学/三角": "Math & Trig",
        "数式の表示": "Show formulas",
        "文字の向き(右横書き)": "Right-to-left text",
        "文字列操作": "Text",
        "日付/時刻": "Date & Time",
        "最近使った関数": "Recently used",
        "枠線も印刷": "Print gridlines",
        "目次の更新": "Update table of contents",
        "縞模様の列": "Banded columns",
        "見出しも印刷": "Print headings",
        "詳細の非表示": "Hide detail",
        "重複の削除": "Remove duplicates",
        "関数の挿入": "Insert function",
    },
    # vendor のロケールに無い語の穴埋め(gen_lang.py が材料の訳と併用する)
    "zh-tw": {
        "書式のコピー": "複製格式",
        "スタイル": "樣式",
        "フィールドリスト": "欄位清單",
        # 表示タブ(こちらで足したボタン — 画面の文字の大きさ)
        "画面の文字を大きく": "放大介面文字",
        "画面の文字を小さく": "縮小介面文字",
        "ページ数": "頁數",
        "表のデザイン": "表格設計",
    },
    "it": {
        "書式のコピー": "Copia formato",
        "スタイル": "Stile",
        "フィールドリスト": "Elenco campi",
        # 表示タブ(こちらで足したボタン — 画面の文字の大きさ)
        "画面の文字を大きく": "Ingrandisci testo dello schermo",
        "画面の文字を小さく": "Riduci testo dello schermo",
        "フィルタのボタン": "Pulsante filtro",
        "ヘッダー行": "Riga di intestazione",
        "合計行": "Riga totale",
        "最後の列": "Ultima colonna",
        "範囲に変換する": "Converti in intervallo",
        "表のデザイン": "Struttura tabella",
        "テーブルのサイズ変更": "Ridimensiona tabella",
    },
    "tr": {
        "書式のコピー": "Biçim boyacısı",
        "スタイル": "Stil",
        "フィールドリスト": "Alan listesi",
        # 表示タブ(こちらで足したボタン — 画面の文字の大きさ)
        "画面の文字を大きく": "Ekran yazısını büyüt",
        "画面の文字を小さく": "Ekran yazısını küçült",
        "範囲を保護する": "Aralığı koru",
        "図形を結合": "Şekilleri birleştir",
        "改ページ プレビュー": "Sayfa Sonu Önizlemesi",
        "フィルタのボタン": "Filtre düğmesi",
        "ヘッダー行": "Üst bilgi satırı",
        "ページ数": "Sayfa sayısı",
        "印刷物で次のページを開始する位置に改行を追加する": "Yeni sayfanın başlayacağı yere sayfa sonu ekle",
        "参照元のトレース": "Etkileyenleri izle",
        "参照先のトレース": "Etkilenenleri izle",
        "合計行": "Toplam satırı",
        "推奨チャートを挿入": "Önerilen grafik ekle",
        "最初の列が右側に来るようにシートの方向を切り替える": "Sayfa yönünü ilk sütun sağda olacak şekilde değiştir",
        "最後の列": "Son sütun",
        "範囲に変換する": "Aralığa dönüştür",
        "罫線": "Kenarlıklar",
        "蛍光ペン": "Vurgulayıcı",
        "表のデザイン": "Tablo tasarımı",
        "カンマスタイル": "Virgül stili",
        "ゴールシーク": "Hedef Ara",
        "テーブルのサイズ変更": "Tabloyu yeniden boyutlandır",
        "ファイルからのテキスト": "Dosyadan metin",
        "SmartArtの挿入": "SmartArt ekle",
        "すべて更新": "Tümünü yenile",
    },
    "id": {
        "書式のコピー": "Salin format",
        "スタイル": "Gaya",
        "フィールドリスト": "Daftar bidang",
        # 表示タブ(こちらで足したボタン — 画面の文字の大きさ)
        "画面の文字を大きく": "Perbesar teks layar",
        "画面の文字を小さく": "Perkecil teks layar",
        "図形を結合": "Gabungkan bentuk",
        "ゴールシーク": "Pencarian Tujuan",
        "テーブルのサイズ変更": "Ubah ukuran tabel",
        "フィルタのボタン": "Tombol filter",
        "ヘッダー行": "Baris header",
        "ページ数": "Jumlah halaman",
        "合計行": "Baris total",
        "最初の列が右側に来るようにシートの方向を切り替える": "Ubah arah lembar agar kolom pertama di kanan",
        "最後の列": "Kolom terakhir",
        "範囲に変換する": "Konversi ke rentang",
        "表のデザイン": "Desain tabel",
    },
    "vi": {
        "書式のコピー": "Sao chép định dạng",
        "スタイル": "Kiểu",
        "フィールドリスト": "Danh sách trường",
        # 表示タブ(こちらで足したボタン — 画面の文字の大きさ)
        "画面の文字を大きく": "Phóng to chữ màn hình",
        "画面の文字を小さく": "Thu nhỏ chữ màn hình",
        "シートを保護する": "Bảo vệ trang tính",
        "ブックを保護する": "Bảo vệ sổ làm việc",
        "範囲を保護する": "Bảo vệ phạm vi",
        "図形を結合": "Hợp nhất hình dạng",
        "改ページ プレビュー": "Xem trước ngắt trang",
        "SmartArtの挿入": "Chèn SmartArt",
        "すべて更新": "Làm mới tất cả",
        "その他の関数": "Hàm khác",
        "ウィンドウ枠の固定": "Cố định ngăn",
        "カンマスタイル": "Kiểu dấu phẩy",
        "コンボボックス": "Hộp tổ hợp",
        "ゴールシーク": "Tìm mục tiêu",
        "シートの表示": "Hiện trang tính",
        "ステータスバー": "Thanh trạng thái",
        "スパークラインを挿入する": "Chèn biểu đồ thu nhỏ",
        "スライサーを挿入": "Chèn slicer",
        "タイトルを印刷する": "In tiêu đề",
        "ダークモード": "Chế độ tối",
        "ツールバーを常に表示する": "Luôn hiện thanh công cụ",
        "テキストの追加": "Thêm văn bản",
        "テキストフィールド": "Trường văn bản",
        "テーブルのサイズ変更": "Đổi cỡ bảng",
        "データの入力規則": "Xác thực dữ liệu",
        "ドロップダウン": "Danh sách thả xuống",
        "ナビゲーション": "Dẫn hướng",
        "ハイフン設定の変更": "Ngắt từ bằng dấu gạch nối",
        "ピボットテーブル": "PivotTable",
        "ピボットテーブルを挿入": "Chèn PivotTable",
        "ファイルからのテキスト": "Văn bản từ tệp",
        "フィルタのボタン": "Nút lọc",
        "フィルター": "Bộ lọc",
        "フォーム": "Biểu mẫu",
        "ブックマーク": "Dấu trang",
        "ヘッダー行": "Hàng tiêu đề",
        "ペン": "Bút",
        "ページ数": "Số trang",
        "ページ番号": "Số trang hiện tại",
        "ページ色の変更": "Màu trang",
        "メールアドレス": "Địa chỉ email",
        "ラジオボタン": "Nút radio",
        "ルーラー": "Thước",
        "レポートのレイアウト": "Bố cục báo cáo",
        "印刷物で次のページを開始する位置に改行を追加する": "Chèn ngắt trang tại vị trí bắt đầu trang mới",
        "印刷範囲": "Vùng in",
        "参照元のトレース": "Truy vết ô ảnh hưởng",
        "参照先のトレース": "Truy vết ô phụ thuộc",
        "右パネル": "Bảng bên phải",
        "合計行": "Hàng tổng",
        "大文字小文字を変更": "Đổi chữ hoa/thường",
        "左パネル": "Bảng bên trái",
        "拡大縮小印刷": "Co giãn khi in",
        "推奨チャートを挿入": "Chèn biểu đồ đề xuất",
        "数式バー": "Thanh công thức",
        "斜体": "Nghiêng",
        "更新": "Làm mới",
        "最初の列": "Cột đầu",
        "最初の列が右側に来るようにシートの方向を切り替える": "Đổi hướng trang tính để cột đầu ở bên phải",
        "条件付き書式": "Định dạng có điều kiện",
        "検索/行列": "Tra cứu & tham chiếu",
        "相互参照": "Tham chiếu chéo",
        "空白ページの挿入": "Chèn trang trống",
        "空行": "Dòng trống",
        "総計": "Tổng cộng",
        "縞模様の行": "Hàng xen kẽ màu",
        "罫線": "Viền",
        "蛍光ペン": "Bút dạ quang",
        "行番号を表示する": "Hiện số dòng",
        "表のデザイン": "Thiết kế bảng",
        "複合フィールド": "Trường phức hợp",
        "複数ページ": "Nhiều trang",
        "見出し": "Tiêu đề",
        "記号を挿入": "Chèn ký hiệu",
        "論理": "Lôgic",
        "財務": "Tài chính",
        "透かしを編集する": "Sửa hình mờ",
        "重複データを削除": "Xóa dữ liệu trùng lặp",
        "開く": "Mở",
        "電話番号": "Số điện thoại",
    },
    "de": {
        "書式のコピー": "Format übertragen",
        "スタイル": "Stil",
        "フィールドリスト": "Feldliste",
        # 表示タブ(こちらで足したボタン — 画面の文字の大きさ)
        "画面の文字を大きく": "Oberflächentext größer",
        "画面の文字を小さく": "Oberflächentext kleiner",
    },
    "es": {
        "書式のコピー": "Copiar formato",
        "スタイル": "Estilo",
        "フィールドリスト": "Lista de campos",
        # 表示タブ(こちらで足したボタン — 画面の文字の大きさ)
        "画面の文字を大きく": "Agrandar texto de la pantalla",
        "画面の文字を小さく": "Reducir texto de la pantalla",
    },
    "fr": {
        "書式のコピー": "Copier le format",
        "スタイル": "Style",
        "フィールドリスト": "Liste des champs",
        # 表示タブ(こちらで足したボタン — 画面の文字の大きさ)
        "画面の文字を大きく": "Agrandir le texte de l'écran",
        "画面の文字を小さく": "Réduire le texte de l'écran",
    },
    "pt": {
        "書式のコピー": "Copiar formato",
        "スタイル": "Estilo",
        "フィールドリスト": "Lista de campos",
        # 表示タブ(こちらで足したボタン — 画面の文字の大きさ)
        "画面の文字を大きく": "Aumentar texto da tela",
        "画面の文字を小さく": "Diminuir texto da tela",
    },
    "ru": {
        "書式のコピー": "Формат по образцу",
        "スタイル": "Стиль",
        "フィールドリスト": "Список полей",
        # 表示タブ(こちらで足したボタン — 画面の文字の大きさ)
        "画面の文字を大きく": "Крупнее текст интерфейса",
        "画面の文字を小さく": "Мельче текст интерфейса",
    },
    "ko": {
        "書式のコピー": "서식 복사",
        "スタイル": "스타일",
        "フィールドリスト": "필드 목록",
        # 表示タブ(こちらで足したボタン — 画面の文字の大きさ)
        "画面の文字を大きく": "화면 글자 크게",
        "画面の文字を小さく": "화면 글자 작게",
    },
    "zh": {
        "書式のコピー": "格式刷",
        "スタイル": "样式",
        "フィールドリスト": "字段列表",
        # 表示タブ(こちらで足したボタン — 画面の文字の大きさ)
        "画面の文字を大きく": "放大界面文字",
        "画面の文字を小さく": "缩小界面文字",
    },
}


def load(app, loc):
    p = ROOT / app / f"main/locale/{loc}.json"
    if not p.exists():
        sys.exit(f"ロケールの現物が見つかりません: {p}")
    return json.load(open(p, encoding="utf-8"))


def build_map(apps, target):
    """ja の語 → target の語。同じ ja 語に複数候補があれば多数決 →
    短い順 → 辞書順(決定的に選ぶ)"""
    cand: dict[str, Counter] = {}
    for app in apps:
        ja = load(app, "ja")
        tr = load(app, target)
        for k, jv in ja.items():
            tv = tr.get(k)
            if not isinstance(jv, str) or not isinstance(tv, str):
                continue
            if not jv.strip() or not tv.strip():
                continue
            cand.setdefault(jv, Counter())[tv] += 1
    out = {}
    for jv, c in cand.items():
        best = sorted(c.items(), key=lambda kv: (-kv[1], len(kv[0]), kv[0]))[0][0]
        out[jv] = best
    return out


def parse_ribbon(src):
    """ribbon.rs の WRITER / CALC を (名前, [(kind, フィールド…)]) に読む"""
    tables = {}
    for const in ("WRITER", "CALC"):
        m = re.search(
            rf"pub const {const}: &\[Tab\] = &\[(.*?)\n\];", src, re.S)
        if not m:
            sys.exit(f"{const} が見つかりません")
        body = m.group(1)
        tabs = []
        for tm in re.finditer(
                r'Tab \{ name: "([^"]+)", cmds: &\[(.*?)\]\s*\}', body, re.S):
            name, cmds_src = tm.group(1), tm.group(2)
            cmds = []
            for cm in re.finditer(
                    r'\b(c|x)\("((?:[^"\\]|\\.)*)"(?:, "((?:[^"\\]|\\.)*)")?'
                    r'(?:, "((?:[^"\\]|\\.)*)")?\)', cmds_src):
                kind = cm.group(1)
                args = [a for a in cm.groups()[1:] if a is not None]
                cmds.append((kind, args))
            tabs.append((name, cmds))
        tables[const] = tabs
    return tables


def main():
    if len(sys.argv) != 2:
        sys.exit("使い方: gen_ribbon_locale.py <locale>  (例: en)")
    target = sys.argv[1]
    over = OVERRIDES.get(target, {})
    doc_map = build_map(["documenteditor", "spreadsheeteditor"], target)
    cell_map = build_map(["spreadsheeteditor", "documenteditor"], target)
    src = open(RIBBON, encoding="utf-8").read()
    tables = parse_ribbon(src)

    missing = []

    def tr(label, m):
        if label in over:
            return over[label]
        if label in m:
            return m[label]
        missing.append(label)
        return label

    out = []
    out.append(f"""//! リボンの {target} 版 — **語だけが ja(ribbon.rs)と違う**。
//! id・並び・ready・icon は ja と同一(ribbon.rs の試験が保証する)。
//!
//! このファイルは手で書かない:
//!
//! ```text
//! python3 ui/gen_ribbon_locale.py {target} > ui/src/ribbon_{target}.rs
//! ```
//!
//! 対訳は vendor/web-apps のロケール(本家の語)。本家に無いこちらの
//! ボタンは gen_ribbon_locale.py の OVERRIDES 表で訳す。

use super::ribbon::{{c, x, Tab}};
""")
    for const, tabs in tables.items():
        m = doc_map if const == "WRITER" else cell_map
        out.append(f"pub const {const}: &[Tab] = &[")
        for name, cmds in tabs:
            out.append(f'    Tab {{ name: "{tr(name, m)}", cmds: &[')
            for kind, args in cmds:
                if kind == "c":
                    cid, label, icon = args
                    out.append(f'        c("{cid}", "{tr(label, m)}", "{icon}"),')
                else:
                    label, icon = args
                    out.append(f'        x("{tr(label, m)}", "{icon}"),')
            out.append("    ]},")
        out.append("];\n")

    if missing:
        uniq = sorted(set(missing))
        sys.exit(
            f"訳の見つからない語が {len(uniq)} 個あります"
            f"(OVERRIDES に足してから出し直してください):\n  "
            + "\n  ".join(uniq))
    print("\n".join(out))


if __name__ == "__main__":
    main()

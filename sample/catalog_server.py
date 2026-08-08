# カタログの商品マスタを配り、注文を受ける小さなサーバー(見本。中身はすべて架空)。
#
#   python3 sample/catalog_server.py        # 127.0.0.1:8765
#
# - GET /catalog.html — カタログ(JS なしの HTML。writer の HTML 読みの検証相手)
# - GET /order.html   — 注文書(JS なしの HTML フォーム。記入して送るだけ — form の
#                       POST は JavaScript 以前からの素の仕組み)
# - GET /catalog.csv  — 品番,分類,品名,説明,単価(税抜) の CSV(商品マスタ)
# - POST /order       — 注文の受け口。JSON(注文書.xlsx の @送信 net)と
#                       フォーム(order.html)の両方を受ける
# - GET /orders       — 受けた注文の一覧(JSON。確認用)
#
# 標準ライブラリだけで動く。HTML も CSV も同じ PRODUCTS から生成(1ソース多形態)。
# 「正本はサーバーのデータ、配布物は文書。コードは文書と一緒に旅をしない」の見本。
#
# 同梱データとの違い(マスタが動いた後、という想定):
#   A-101〜A-103 は 150→160 に値上げ、D-401 は 1480→1380 に値下げ、
#   E-505(結束バンド)が新商品として増えている。
import csv
import html
import io
import json
import os
import sys
import urllib.parse
from http.server import BaseHTTPRequestHandler, HTTPServer

ORDERS = []  # 受けた注文(この見本ではメモリに持つだけ)

STYLE = """<style>
body { font-family: sans-serif; margin: 2rem auto; max-width: 46rem; }
table { border-collapse: collapse; width: 100%; margin: 0.5rem 0 1.5rem; }
th, td { border: 1px solid #999; padding: 0.3rem 0.6rem; text-align: left; }
th { background: #dce6f1; }
td.num, th.num { text-align: right; }
input { padding: 0.2rem; }
tr:hover td { background: #f4f8fc; }              /* 動き: CSS の宣言だけ */
summary { cursor: pointer; font-size: 1.2rem; font-weight: bold;
          margin: 1rem 0 0.3rem; }
details { border-left: 3px solid #dce6f1; padding-left: 0.8rem; }
/* 商品はカードで陳列 — 絵で選べるように。動きは全部 CSS の宣言 */
.cards { display: grid; grid-template-columns: repeat(auto-fill, minmax(9.5rem, 1fr));
         gap: 0.7rem; margin: 0.5rem 0 1.2rem; }
.card { border: 1px solid #bbb; border-radius: 6px; padding: 0.5rem;
        display: block; cursor: pointer; background: #fff; }
.card:hover { border-color: #4c7baf; box-shadow: 0 1px 4px #0002; }
.card:has(:checked) { outline: 3px solid #4c7baf; }   /* 選んだ札が光る */
.card img { width: 100%; height: auto; display: block; }
.card .nm { font-weight: bold; margin-top: 0.3rem; }
.card .ds { color: #555; font-size: 0.85rem; }
.card .pr { text-align: right; font-size: 1.05rem; margin-top: 0.2rem; }
td.best, th.best { background: #e2efda; }             /* いちばん安い列 */
</style>"""


def page(title, body):
    return (f"<!DOCTYPE html><html lang=\"ja\"><head><meta charset=\"utf-8\">"
            f"<title>{html.escape(title)}</title>{STYLE}</head>"
            f"<body>{body}</body></html>").encode("utf-8")


"""商品の絵 — サーバーが SVG を生成する(標準ライブラリのまま)。
分類ごとの形+商品の色(品名の(黒)(赤)…から拾う)。写真の代わりの見本。"""
INK = {"黒": "#222", "赤": "#c0392b", "青": "#2e5fa3", "黄": "#e0b400", "桃": "#e08bb0"}

def svg_product(code):
    p = next((p for p in PRODUCTS if p[0] == code), None)
    if p is None:
        return None
    _, cat, name, _, _ = p
    ink = next((v for k, v in INK.items() if f"({k})" in name), "#4c7baf")
    g = ""
    if cat == "筆記具":       # 軸+ペン先。インクの色がそのまま見える
        g = (f'<rect x="40" y="52" width="80" height="10" rx="5" fill="{ink}"/>'
             f'<polygon points="120,52 138,57 120,62" fill="{ink}"/>')
    elif cat == "紙製品":     # 紙の重なり
        g = ('<rect x="50" y="30" width="60" height="50" fill="#fff" stroke="#999"/>'
             '<rect x="56" y="24" width="60" height="50" fill="#fff" stroke="#999"/>'
             f'<rect x="62" y="18" width="60" height="50" fill="#fdfdf5" stroke="{ink}"/>')
    elif cat == "ファイル・収納":  # フォルダ
        g = (f'<path d="M40 34 h30 l8 8 h42 v40 h-80 z" fill="{ink}" opacity="0.85"/>'
             '<rect x="40" y="46" width="80" height="36" fill="#fff" opacity="0.25"/>')
    elif cat == "事務機器":   # 計器の箱とボタン
        g = (f'<rect x="55" y="22" width="50" height="66" rx="6" fill="{ink}" opacity="0.9"/>'
             '<rect x="62" y="30" width="36" height="14" fill="#eef"/>'
             + "".join(f'<circle cx="{68+dx*12}" cy="{56+dy*12}" r="4" fill="#eef"/>'
                       for dy in range(3) for dx in range(3)))
    else:                     # 梱包・雑貨: テープの輪
        g = (f'<circle cx="80" cy="52" r="30" fill="{ink}" opacity="0.85"/>'
             '<circle cx="80" cy="52" r="14" fill="#fff"/>')
    short = html.escape(name if len(name) <= 11 else name[:10] + "…")
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 160 120">'
            f'<rect width="160" height="120" fill="#f6f8fa"/>{g}'
            f'<text x="80" y="106" text-anchor="middle" font-size="12" '
            f'fill="#333" font-family="sans-serif">{short}</text></svg>').encode("utf-8")


def card(p):
    code, _, name, desc, price = p
    return (f'<label class="card"><img src="/img/{code}.svg" alt="{html.escape(name)}">'
            f'<div class="nm">{html.escape(name)}</div>'
            f'<div class="ds">{code}・{html.escape(desc)}</div>'
            f'<div class="pr">{price:,}円</div>'
            f'<input type="checkbox" name="c" value="{code}"> 比べる</label>')


def catalog_page(q=""):
    """カタログ。商品はカードで陳列 — 絵と値段で選べる。「動き」は
    検索・比較=サーバー側 Python(form の往復)、開閉・選択の光り=CSS の宣言。"""
    hit = [p for p in PRODUCTS
           if not q or q in p[0] or q in p[2] or q in p[3]]
    b = ["<h1>事務用品カタログ(2026年秋)</h1>",
         "<p>例示文具株式会社 — 価格はすべて税抜。"
         "ご注文は <a href=\"/order.html\">注文書</a> で。</p>",
         "<form method=\"get\" action=\"/catalog.html\">"
         f"<input name=\"q\" size=\"20\" value=\"{html.escape(q)}\" "
         "placeholder=\"品名・説明で絞る\"> <input type=\"submit\" value=\"検索\">"
         "</form>"]
    if q:
        b.append(f"<p>「{html.escape(q)}」で {len(hit)} 品目 — "
                 "<a href=\"/catalog.html\">すべて見る</a></p>")
    b.append("<form method=\"get\" action=\"/compare\">"
             "<p>迷ったら札を選んで → <input type=\"submit\" value=\"選んだ商品を比べる\"></p>")
    cats = []
    for p in hit:
        if p[1] not in cats:
            cats.append(p[1])
    for cat in cats:
        b.append(f"<details open><summary>{html.escape(cat)}</summary>"
                 "<div class=\"cards\">")
        b.extend(card(p) for p in hit if p[1] == cat)
        b.append("</div></details>")
    b.append("</form>")
    if not hit:
        b.append("<p>該当なし。</p>")
    return page("事務用品カタログ", "".join(b))


def compare_page(codes):
    """選んだ商品を並べて見せる。どれがいちばん安いかはサーバーが教える。"""
    sel = [p for p in PRODUCTS if p[0] in codes]
    if not sel:
        return page("比較", "<h1>比較</h1><p>商品が選ばれていません。"
                    "<a href=\"/catalog.html\">カタログへ戻る</a></p>")
    cheapest = min(p[4] for p in sel)
    def row(head, f):
        cells = "".join(
            f"<td class=\"{'best' if p[4] == cheapest else ''}\">{f(p)}</td>"
            for p in sel)
        return f"<tr><th>{head}</th>{cells}</tr>"
    b = [f"<h1>比較({len(sel)}点)</h1>",
         "<table>",
         row("", lambda p: f'<img src="/img/{p[0]}.svg" alt="" width="120">'),
         row("品名", lambda p: html.escape(p[2])),
         row("品番・分類", lambda p: f"{p[0]}・{html.escape(p[1])}"),
         row("説明", lambda p: html.escape(p[3])),
         row("単価(税抜)", lambda p: f"<b>{p[4]:,}円</b>"
             +("(いちばん安い)" if p[4] == cheapest else "")),
         "</table>",
         "<p><a href=\"/catalog.html\">カタログへ戻る</a> / "
         "<a href=\"/order.html\">注文書へ</a></p>"]
    return page("比較", "".join(b))


def order_page():
    b = ["<h1>注文書</h1>",
         "<p>例示文具株式会社 行 — 品番は<a href=\"/catalog.html\">カタログ</a>から。</p>",
         "<form method=\"post\" action=\"/order\">",
         "<table><tr><th>社名</th><td><input name=\"company\" size=\"30\"></td>"
         "<th>担当</th><td><input name=\"person\" size=\"12\"></td></tr></table>",
         "<table><tr><th>品番</th><th>数量</th></tr>"]
    for i in range(1, 11):
        b.append(f"<tr><td><input name=\"c{i}\" size=\"8\"></td>"
                 f"<td><input name=\"q{i}\" size=\"6\"></td></tr>")
    b.append("</table><p><input type=\"submit\" value=\"注文を送る\"></p></form>")
    return page("注文書", "".join(b))

PRODUCTS = [
    ("A-101", "筆記具", "ボールペン(黒)", "0.7mm・油性", 160),
    ("A-102", "筆記具", "ボールペン(赤)", "0.7mm・油性", 160),
    ("A-103", "筆記具", "ボールペン(青)", "0.7mm・油性", 160),
    ("A-104", "筆記具", "シャープペン", "0.5mm", 220),
    ("A-105", "筆記具", "シャープ替芯", "0.5mm・40本", 120),
    ("A-106", "筆記具", "蛍光マーカー(黄)", "太細両用", 130),
    ("A-107", "筆記具", "蛍光マーカー(桃)", "太細両用", 130),
    ("A-108", "筆記具", "油性ペン(黒)", "太字", 160),
    ("A-109", "筆記具", "鉛筆HB", "12本入り", 480),
    ("A-110", "筆記具", "消しゴム", "まとまるタイプ", 90),
    ("B-201", "紙製品", "コピー用紙A4", "500枚", 550),
    ("B-202", "紙製品", "コピー用紙B5", "500枚", 520),
    ("B-203", "紙製品", "ノートA罫", "30枚・セミB5", 180),
    ("B-204", "紙製品", "レポート用紙A4", "50枚", 250),
    ("B-205", "紙製品", "付箋 75×75mm", "桃・100枚", 210),
    ("B-206", "紙製品", "付箋 75×25mm", "3色・各100枚", 260),
    ("B-207", "紙製品", "封筒 長形3号", "100枚", 680),
    ("B-208", "紙製品", "クラフト封筒 角形2号", "50枚", 750),
    ("C-301", "ファイル・収納", "クリアファイルA4", "10枚", 240),
    ("C-302", "ファイル・収納", "パイプ式ファイルA4", "背幅5cm", 780),
    ("C-303", "ファイル・収納", "個別フォルダA4", "10枚", 620),
    ("C-304", "ファイル・収納", "2穴バインダーA4", "背幅3cm", 450),
    ("C-305", "ファイル・収納", "書類トレーA4", "積み重ね可", 520),
    ("C-306", "ファイル・収納", "マグネットバー", "20cm", 330),
    ("D-401", "事務機器", "電卓", "12桁", 1380),
    ("D-402", "事務機器", "ホッチキス10号", "20枚とじ", 620),
    ("D-403", "事務機器", "ホッチキス針10号", "1000本", 110),
    ("D-404", "事務機器", "2穴パンチ", "20枚", 830),
    ("D-405", "事務機器", "テープカッター", "大巻用", 690),
    ("D-406", "事務機器", "はさみ", "175mm", 420),
    ("D-407", "事務機器", "カッターL型", "替刃1枚付き", 380),
    ("D-408", "事務機器", "スティックのり", "約10g", 140),
    ("E-501", "梱包・雑貨", "ガムテープ(布)", "50mm×25m", 280),
    ("E-502", "梱包・雑貨", "OPPテープ(透明)", "48mm×100m", 190),
    ("E-503", "梱包・雑貨", "緩衝材", "ぷちぷち・10m", 640),
    ("E-504", "梱包・雑貨", "宅配袋(大)", "10枚", 520),
    ("E-505", "梱包・雑貨", "結束バンド", "100本・新商品", 350),
]


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/catalog.csv":
            out = io.StringIO()
            w = csv.writer(out)
            w.writerow(["品番", "分類", "品名", "説明", "単価"])
            w.writerows(PRODUCTS)
            body = out.getvalue().encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "text/csv; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif self.path.split("?")[0] == "/catalog.html":
            qs = urllib.parse.urlparse(self.path).query
            q = urllib.parse.parse_qs(qs).get("q", [""])[0].strip()
            self.reply_html(catalog_page(q))
        elif self.path.split("?")[0] == "/compare":
            qs = urllib.parse.urlparse(self.path).query
            codes = set(urllib.parse.parse_qs(qs).get("c", []))
            self.reply_html(compare_page(codes))
        elif self.path.startswith("/img/") and self.path.endswith(".svg"):
            body = svg_product(self.path[5:-4])
            if body is None:
                self.send_response(404)
                self.end_headers()
                return
            self.send_response(200)
            self.send_header("Content-Type", "image/svg+xml; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif self.path == "/order.html":
            self.reply_html(order_page())
        elif self.path == "/orders":
            self.reply_json(ORDERS)
        else:
            body = ("GET /catalog.csv(商品マスタ)/ POST /order(注文の受付)"
                    "/ GET /orders(受けた注文)\n").encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "text/plain; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    def do_POST(self):
        if self.path != "/order":
            self.send_response(404)
            self.end_headers()
            return
        n = int(self.headers.get("Content-Length", 0))
        raw = self.rfile.read(n).decode("utf-8")
        is_form = "json" not in (self.headers.get("Content-Type") or "")
        if is_form:
            # HTML フォームから(order.html。JS なしの素の POST)
            q = {k: v[0].strip() for k, v in urllib.parse.parse_qs(raw).items()}
            codes = {p[0] for p in PRODUCTS}
            lines, rejected = [], []
            for i in range(1, 11):
                code, qty = q.get(f"c{i}", ""), q.get(f"q{i}", "")
                if not code and not qty:
                    continue
                if code in codes and qty.isdigit() and int(qty) > 0:
                    lines.append({"品番": code, "数量": int(qty)})
                else:
                    rejected.append(f"{i}行目({code or '品番なし'})")
            if not lines:
                self.reply_html(page("注文書", "<h1>受け付けられません</h1>"
                                     "<p>正しい品番と数量の行がありません。</p>"), code=400)
                return
            order = {"社名": q.get("company") or "(未記入)",
                     "担当": q.get("person", ""), "明細": lines}
        else:
            try:
                order = json.loads(raw)
            except ValueError:
                self.reply_json({"error": "JSON が読めません"}, code=400)
                return
        ORDERS.append(order)
        no = len(ORDERS)
        print(f"注文を受けた(受付番号 {no}): {order}")
        if is_form:
            note = (f"<p>読めなかった行は受けていません: {'、'.join(rejected)}</p>"
                    if rejected else "")
            self.reply_html(page("受付", f"<h1>受け付けました</h1>"
                                 f"<p>受付番号 {no}・明細 {len(order['明細'])} 行。</p>{note}"
                                 "<p><a href=\"/order.html\">続けて注文する</a></p>"))
        else:
            self.reply_json({"受付番号": no, "明細": len(order.get("明細", []))})

    def reply_html(self, body, code=200):
        self.send_response(code)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def reply_json(self, obj, code=200):
        body = json.dumps(obj, ensure_ascii=False).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, fmt, *args):
        print("受信:", fmt % args)


def load_csv(path):
    """商品マスタを CSV から差し替える(e-shop プラグインの出力を受ける口)。
    列: 品番,分類,品名,説明,単価。正本は販売者の文書 — CSV はその写し。"""
    global PRODUCTS
    rows = list(csv.reader(open(path, encoding="utf-8")))[1:]
    PRODUCTS = [(a, b, c, d, int(e)) for a, b, c, d, e in rows if a]
    print(f"商品マスタ: {path}({len(PRODUCTS)} 品目)")


if __name__ == "__main__":
    if len(sys.argv) > 1:
        load_csv(sys.argv[1])                      # 明示指定
    elif os.path.exists("sample/商品マスタ.csv"):
        load_csv("sample/商品マスタ.csv")           # eshop.py の出力があれば使う
    addr = ("127.0.0.1", 8765)
    print(f"商品マスタのサーバー: http://{addr[0]}:{addr[1]}/catalog.csv(Ctrl+C で止める)")
    HTTPServer(addr, Handler).serve_forever()

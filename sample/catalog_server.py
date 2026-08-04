# カタログの商品マスタを配り、注文を受ける小さなサーバー(見本。中身はすべて架空)。
#
#   python3 sample/catalog_server.py        # 127.0.0.1:8765
#
# - GET /catalog.csv — 品番,分類,品名,説明,単価(税抜) の CSV(商品マスタ)
# - POST /order      — 注文(JSON)を受けて受付番号を返す(注文書.xlsx の @送信 net)
# - GET /orders      — 受けた注文の一覧(JSON。確認用)
#
# 標準ライブラリだけで動く。gen_catalog.py がここから取ってカタログを作る —
# 「価格の正本はサーバー、docx / xlsx は手元」の分業の見本。
#
# 同梱データとの違い(マスタが動いた後、という想定):
#   A-101〜A-103 は 150→160 に値上げ、D-401 は 1480→1380 に値下げ、
#   E-505(結束バンド)が新商品として増えている。
import csv
import io
import json
from http.server import BaseHTTPRequestHandler, HTTPServer

ORDERS = []  # 受けた注文(この見本ではメモリに持つだけ)

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
        try:
            order = json.loads(self.rfile.read(n).decode("utf-8"))
        except (ValueError, UnicodeDecodeError):
            self.reply_json({"error": "JSON が読めません"}, code=400)
            return
        ORDERS.append(order)
        no = len(ORDERS)
        print(f"注文を受けた(受付番号 {no}): {order}")
        self.reply_json({"受付番号": no, "明細": len(order.get("明細", []))})

    def reply_json(self, obj, code=200):
        body = json.dumps(obj, ensure_ascii=False).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, fmt, *args):
        print("受信:", fmt % args)


if __name__ == "__main__":
    addr = ("127.0.0.1", 8765)
    print(f"商品マスタのサーバー: http://{addr[0]}:{addr[1]}/catalog.csv(Ctrl+C で止める)")
    HTTPServer(addr, Handler).serve_forever()

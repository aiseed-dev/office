# 注文書.xlsx の手続き「送信」—
# 注文行(品番と数量の入った行)をサーバーへ送る。
#
# 据え付け(1機械1回): 中身を確かめてから
#   ~/.config/office/plugins/送信.py
# へ写す。以後、注文書を開いて データ > Python のパネルで「@送信 net」
# (網ありサンドボックス — 許可はその場の操作だけで、ブックには保存されない)。
URL = "http://127.0.0.1:8765/order"

import urllib.request, json
lines = []
for n in range(7, 17):
    code, qty = s[f"A{n}"], s[f"D{n}"]
    if code and qty:
        lines.append({"品番": code, "数量": int(qty)})
if not lines:
    print("注文行がありません(品番と数量を入れてから)")
else:
    order = {"社名": s["B3"] or "(未記入)", "担当": s["D3"] or "", "明細": lines}
    req = urllib.request.Request(
        URL, json.dumps(order, ensure_ascii=False).encode("utf-8"),
        {"Content-Type": "application/json"})
    r = json.loads(urllib.request.urlopen(req, timeout=5).read().decode("utf-8"))
    print(f"送信しました(受付番号 {r['受付番号']}・明細 {len(lines)} 行)")

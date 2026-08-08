# 注文書.xlsx の手続き「更新」—
# 品番マスタの写し(H〜J 列)をサーバーの正本と入れ替える。
#
# 据え付け(1機械1回): 中身を確かめてから
#   ~/.config/office/plugins/更新.py
# へ写す。以後、注文書を開いて データ > Python のパネルで「@更新 net」
# (網ありサンドボックス — 許可はその場の操作だけで、ブックには保存されない)。
URL = "http://127.0.0.1:8765/catalog.csv"

import urllib.request, csv, io
raw = urllib.request.urlopen(URL, timeout=5).read()
rows = list(csv.reader(io.StringIO(raw.decode("utf-8"))))[1:]
for i, r in enumerate(rows):
    n = 2 + i
    s[f"H{n}"] = r[0]          # 品番
    s[f"I{n}"] = r[2]          # 品名
    s[f"J{n}"] = int(r[4])     # 単価
for n in range(2 + len(rows), 41):   # 減った分の残骸は消す
    s[f"H{n}"] = None; s[f"I{n}"] = None; s[f"J{n}"] = None
b.recalc()
print(f"品番マスタを {len(rows)} 品目に更新しました")

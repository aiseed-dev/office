# 受注台帳.xlsx の手続き「取り込み」—
# 店(catalog_server)に溜まった注文を台帳へ追記する。
#
# 据え付け(1機械1回): 中身を確かめてから
#   ~/.config/office/plugins/取り込み.py
# へ写す(templates/ の問い合わせ台帳の取り込みと同名 — 同じ機械で両方
# 使うなら、どちらかを別名で置く。@名前 はファイル名がそのまま)。
# 以後、台帳を開いて データ > Python の板で「@取り込み net」
# (網あり檻 — 許可はその場の操作だけで、ブックには保存されない)。
# 取込済の件数(K2)を控えているので、新しい注文だけが入る。
URL = "http://127.0.0.1:8765"

import urllib.request, json, csv, io
orders = json.loads(urllib.request.urlopen(URL + "/orders", timeout=5).read().decode("utf-8"))
raw = urllib.request.urlopen(URL + "/catalog.csv", timeout=5).read().decode("utf-8")
master = {r[0]: (r[2], int(r[4])) for r in list(csv.reader(io.StringIO(raw)))[1:]}
done = int(float(s["K2"] or 0))
new = orders[done:]
if not new:
    print(f"新しい注文はありません(累計 {len(orders)} 件)")
else:
    n = 2
    while s[f"A{n}"] not in (None, ""):
        n += 1
    lines = 0
    for i, o in enumerate(new, start=done + 1):
        for line in o.get("明細", []):
            code = str(line.get("品番", ""))
            name, price = master.get(code, ("(不明な品番)", 0))
            s[f"A{n}"] = i
            s[f"B{n}"] = o.get("社名", "")
            s[f"C{n}"] = code
            s[f"D{n}"] = name
            s[f"E{n}"] = int(line.get("数量", 0))
            s[f"F{n}"] = price
            s[f"G{n}"] = f"=E{n}*F{n}"
            s[f"H{n}"] = "FALSE"
            n += 1
            lines += 1
    s["K2"] = len(orders)
    b.recalc()
    print(f"{len(new)} 件({lines} 行)を取り込みました(累計 {len(orders)} 件)")

# 問い合わせ台帳.xlsx の手続き「取り込み」—
# フォームの受信箱(CSV を返す URL)から新着を台帳へ追記する。
#
# 据え付け(1機械1回): 中身を確かめてから
#   ~/.config/office/plugins/取り込み.py
# へ写す。以後、台帳を開いて データ > Python のパネルで「@取り込み net」
# (網ありサンドボックス — 許可はその場の操作だけで、ブックには保存されない)。
# URL を自分のフォームのものに書き換えて使う。
URL = "http://127.0.0.1:8000/inbox.csv"

import urllib.request, csv, io
raw = urllib.request.urlopen(URL, timeout=5).read()
rows = list(csv.DictReader(io.StringIO(raw.decode("utf-8"))))
base = len([r for r in s.values() if any(v is not None and v != "" for v in r)])
for i, r in enumerate(rows):
    n = base + 1 + i
    s[f"A{n}"] = r.get("received", "")
    s[f"B{n}"] = r.get("name", "")
    s[f"C{n}"] = r.get("email", "")
    s[f"D{n}"] = r.get("body", "")
    s[f"E{n}"] = "未対応"
print(f"{len(rows)} 件を取り込みました")

# office_sheet(pysheet)の検査。Rust 側の tests/python_smoke.rs から呼ばれる。
# 手で回すなら:
#   cargo build -p pysheet
#   mkdir -p /tmp/os && cp target/debug/liboffice_sheet.so /tmp/os/office_sheet.so
#   PYTHONPATH=/tmp/os python3 pysheet/test.py
import os
import sys
import tempfile

import office_sheet

def check(cond, msg):
    if not cond:
        print(f"NG: {msg}", file=sys.stderr)
        sys.exit(1)

# --- 作って・計算して・保存して・読み直す -----------------------------------
b = office_sheet.Book()
s = b[0]
s["A1"] = "ザボガードF F-02"
s["B1"] = 4
s["C1"] = 125000
s["D1"] = "=B1*C1"
s["D2"] = "=ROUND(D1*0.1,0)"
s["D3"] = "=D1+D2"
check(s["D3"] == 550000, f"式が計算されない: {s['D3']}")
check(s.formula("D3") == "=D1+D2", "編集欄に式が戻らない")
check(isinstance(s["B1"], float) and s["B1"] == 4, "数が数で返らない")
check(s["Z9"] is None, "空セルが None でない")

with tempfile.TemporaryDirectory() as d:
    out = os.path.join(d, "round.xlsx")
    b.save(out)
    b2 = office_sheet.Book.open(out)
    s2 = b2[0]
    check(s2["A1"] == "ザボガードF F-02", "日本語が往復しない")
    check(s2["D3"] == 550000, "式が保存されず再計算できない")
    check(s2.formula("D1") == "=B1*C1", "式そのものが往復しない")

# --- 型: bool は bool のまま、None は消す ------------------------------------
s["E1"] = True
check(s["E1"] is True, "bool が bool で返らない")
s["A1"] = None
check(s["A1"] is None, "None で消えない")

# --- 文字列は「打ったのと同じ」解釈 ------------------------------------------
s["F1"] = "123"
check(s["F1"] == 123, "数字だけの文字列が数にならない(calc で打ったのと違う)")

# --- datetime は Excel の通し番号になる(帳票の日付セルの定番)----------------
import datetime
EPOCH = datetime.date(1899, 12, 30)   # Excel の通し番号の起点
d = datetime.date(2026, 8, 5)
s["G1"] = d
check(s["G1"] == (d - EPOCH).days, f"date が通し番号にならない: {s['G1']}")
s["G2"] = "=YEAR(G1)"
s["G3"] = "=MONTH(G1)"
check((s["G2"], s["G3"]) == (2026, 8),
      f"通し番号が DATE 関数の規約とずれている: YEAR={s['G2']} MONTH={s['G3']}")
s["G4"] = datetime.datetime(2026, 8, 5, 18, 0, 0)
check(abs(s["G4"] - ((d - EPOCH).days + 0.75)) < 1e-9,
      f"datetime の時刻が日の割合にならない: {s['G4']}")
s["G5"] = datetime.time(6, 0)
check(abs(s["G5"] - 0.25) < 1e-9, f"time が日の割合にならない: {s['G5']}")
try:
    s["G6"] = object()
    check(False, "置けない型を黙って受けた")
except TypeError:
    pass

# --- 行の出し入れで式の参照も動く(明細行を増やす操作)------------------------
b3 = office_sheet.Book()
t = b3[0]
for i, n in enumerate([100, 200, 300], start=1):
    t[f"A{i}"] = n
t["A4"] = "=SUM(A1:A3)"
t.insert_row(2)                       # 2行目に空行 → 明細が1行増える
check(t.formula("A5") == "=SUM(A1:A4)", f"行を挿しても参照が伸びない: {t.formula('A5')}")
t["A2"] = 50
check(t["A5"] == 650, f"挿した行に打った値が合計に入らない: {t['A5']}")
t.remove_row(2)
check(t["A4"] == 600, f"行を抜いた後の合計が違う: {t['A4']}")

# --- 表示形式は据え置き(値を差し替えても書式が残るのが存在理由)--------------
# 書式は Python からは作れない(作るのは calc の仕事)ので、実物で確かめる
real = "/mnt/sdb/home/dev/ドキュメント/機構/yoryou-yoshiki/実施要領様式7_提案見積書.xlsx"
if os.path.exists(real):
    rb = office_sheet.Book.open(real)
    rs = rb[0]
    check(rs["B1"] == "（様式７）", f"実物の中身が読めない: {rs['B1']}")
    merges = rs.merges
    check(len(merges) > 0, "実物の様式にセル結合が無いことになっている")
    rows, cols = rs.shape
    check(rows > 0 and cols > 0, "実物の範囲が取れない")
    rs["A30"] = "日本フネン株式会社"
    rs["C30"] = "=B30*100"
    rs["B30"] = 3
    check(rs["C30"] == 300, "実物の上で式が効かない")
    with tempfile.TemporaryDirectory() as d:
        out = os.path.join(d, "様式7_差込.xlsx")
        rb.save(out)
        rb2 = office_sheet.Book.open(out)
        rs2 = rb2[0]
        check(rs2["A30"] == "日本フネン株式会社", "差し込んだ値が保存されない")
        check(rs2["B1"] == "（様式７）", "元の内容が壊れた")
        check(rs2.merges == merges, "保存でセル結合が崩れた(帳票の枠が壊れた)")
        import zipfile
        with zipfile.ZipFile(out) as z:
            names = z.namelist()
        check("xl/styles.xml" in names, "書式ごと消えた")
else:
    print("実物の様式が無いので、その分の検査は飛ばした", file=sys.stderr)

# --- polars 連携(polars がある環境でだけ回る)--------------------------------
# 集計は polars、枠は sheet — 分業の橋そのものの検査。
# 開発機では .venv(miniforge)+ abi3 の .so で:
#   cargo build -p pysheet --release --features extension-module
#   mkdir -p /tmp/os && cp target/release/liboffice_sheet.so /tmp/os/office_sheet.so
#   PYTHONPATH=/tmp/os .venv/bin/python pysheet/test.py
try:
    import polars as pl
except ImportError:
    print("polars が無いので、その分の検査は飛ばした", file=sys.stderr)
else:
    df = pl.DataFrame({"品名": ["甲", "甲", "乙"], "金額": [100, 150, 40]})
    g = df.group_by("品名").agg(pl.col("金額").sum()).sort("品名")
    pb = office_sheet.Book()
    ps = pb[0]
    for i, (name, total) in enumerate(g.iter_rows(), start=1):
        ps[f"A{i}"] = name
        ps[f"B{i}"] = float(total)
    ps["B3"] = "=SUM(B1:B2)"
    check(ps["B3"] == 290, f"polars の集計が差し込めない: {ps['B3']}")
    back = pl.DataFrame(ps.values(), orient="row")
    check(back.shape == (3, 2), f"values() が DataFrame にならない: {back.shape}")

print("OK")

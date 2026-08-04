# Python の手引き — 配列と API

釦の使い方は [calc](calc-manual.md) / [writer](writer-manual.md) の手引きに。
ここは**コードを書く人のための1冊** — とくに「範囲⇄配列」のやり取りは
画面から見えないので、ここが正本。全部この機械で実測してある。

## Python が動く場所と、渡される束縛

| 場所 | 束縛 | 檻 |
|---|---|---|
| calc: データ > Python(一行 / .py) | `b` = ブック、`s` = いまのシート | 檻があれば檻(網あり) |
| calc: `@save 名前` → `@名前` | 同上 | **必ず檻**(網なし。`net` で網あり) |
| calc: `=PY("関数",…)` + `@計算` | 引数が値で渡る(下記) | 必ず檻(網なし) |
| calc / writer: マクロ・プラグイン | calc は `b`/`s`、**writer は `d` = python-docx の文書** | 檻 |
| writer: ページの Python(HTML) | `form` = 記入欄の名前→値の辞書 | 必ず檻 |

どれも**複製の上で走る** — 失敗しても表・文書は無傷、成功したら結果が
**1手**として入る(複数シートに触っても Ctrl+Z 一回で戻る)。

## office_sheet(pysheet)の API

```python
import office_sheet                     # calc の中では import 済みで b, s が来る
b = office_sheet.Book.open("帳票.xlsx")
s = b["シート名"]                        # 番号でも: b[0]
b.sheet_names                           # ['見積書', …]
b.add_sheet("新しいシート")              # 同名があればエラー
b.recalc()                              # 式を計算し直す(値を読む前に)
b.save("out.xlsx")                      # 原本の部品は据え置き
b.unsupported                           # 読めなかった部品の一覧(空 = 全部読めた)
```

### セルの読み書き

```python
s["A1"]            # 読み: 数は float、文字は str、☑/☐ は bool、式セルは計算値
s.formula("E2")    # 式そのもの("=SUM(B2:D2)"。式でなければ None)
s.display("E2")    # 表示の文字("238"。表示形式を通した見た目)
s["A1"] = 100      # 書き: 数
s["A1"] = "文字"   #        文字
s["A1"] = True     #        真偽(calc では ☑/☐ に見える)
s["A1"] = "=B1*C1" #        式("=" で始まる文字列)
s["A1"] = None     #        消す
```

- **書式は据え置き** — 値を入れても罫線・結合・表示形式は変わらない
- 空のセルは **None か ""** で返る(触ったことのないセルは None、
  空文字を入れたセルは ""。どちらも偽なので `if s["A1"]:` で足りるが、
  厳密には `s["A1"] in (None, "")` で見る)

### 配列(範囲)のやり取り — ここが本題

**範囲の添字は無い**(`s["A2:C3"]` はエラー)。**2次元の一括代入も無い**
(`s["A1"] = [[…]]` はエラー)。配列はこう扱う:

```python
# 読み: values() が使っている広さ全体の2次元リスト(行×列、0 始まり)
rows, cols = s.shape          # (10, 6) — shape は属性(() を付けない)
v = s.values()                # v[0] = 1行目(見出し)、v[1][1] = B2 の値
tbl = [r[0:3] for r in v[1:6]]   # A2:C6 を切り出す

# 書き: ループで1セルずつ(行番号は A1 表記なので 1 始まりに注意)
data = [["ペン", 10, 150], ["ノート", 5, 180]]
for i, row in enumerate(data):
    n = 2 + i                              # 2行目から
    s[f"A{n}"], s[f"B{n}"], s[f"C{n}"] = row
    s[f"D{n}"] = f"=B{n}*C{n}"             # 式も文字列で入れる
b.recalc()
```

### polars との往復

```python
import polars as pl
# シート → DataFrame(1行目を見出しに)
v = s.values()
df = pl.DataFrame({h: [r[i] for r in v[1:]] for i, h in enumerate(v[0])})

# DataFrame → シート(見出しの下へ)
for i, row in enumerate(df.rows()):
    for j, val in enumerate(row):
        s[f"{chr(65 + j)}{2 + i}"] = val
```

集計・結合・絞り込みは polars 側でやるのが分業の流儀
(シートは帳票の形、データの計算は Python)。

## =PY(UDF)の配列

```
=PY("集計", A1:B10, 100, "甲")
```

- 範囲の引数は**行×列の2次元リスト**(値。1セルはスカラ)で def に渡る
- 返り値: スカラ → そのセルへ / **1次元リスト → 下へ展開** /
  **2次元リスト → 右下へスピル**。展開先に他人のデータがあれば
  `#SPILL!` で止まる(潰さない)
- 評価は `@計算` のときだけ・檻の中。関数の定義は「関数」で始まる名前で
  `@save` したスクリプトの def

```python
def 集計(r, 上限, 種別):        # r = [[行1列1, 行1列2], [行2列1, …], …]
    hit = [row for row in r if row[0] == 種別 and row[1] <= 上限]
    return [[row[0], row[1]] for row in hit]   # 2次元 → スピル
```

## writer のマクロ(d = python-docx)

```python
# d が python-docx の Document。API は python-docx の公式文書のまま
d.paragraphs[12].runs[0].text = "商号 例示工務店"
for r in d.paragraphs[12].runs[1:]:
    r.text = ""                  # 先頭ランに書き、残りを空に(書式が残る作法)
d.tables[0].rows[1].cells[2].text = "640,200円"
```

保存は writer 側がやる(台本の中で d.save は不要)。

## ページの Python(HTML の form)

```python
# form = 記入欄の名前 → 値の辞書。返した値が紙面に書き戻る
qty = int(form.get("qty") or 0)
form["total"] = qty * 150
```

## 檻の中でできること・できないこと

- 実ファイルは**読み取り専用**、ホームは見えない、書けるのは交換用の
  一時領域だけ。網は**既定で閉じている** — `@名前 net` とその場で打った
  ときだけ開く(許可はブックに保存されない)
- 機械に入っているライブラリ(polars・scipy・matplotlib 等)は使える
- print した文字は状態行に出る(進み具合や件数はそこで言う)

## 実例(そのまま読める見本)

- [templates/](../templates/README.md) — 問い合わせ台帳(`@取り込み net` の
  CSV 取り込み・=PY の状態集計)ほか
- [sample/注文書.xlsx](../sample/README.md) — マスタの入れ替え(`@更新 net`)と
  JSON の送信(`@送信 net`)
- [sample/受注台帳.xlsx](../sample/README.md) — 取り込みの控え(K2)で
  重複を防ぐ増分の取り込み

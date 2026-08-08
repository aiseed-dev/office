# Python manual — arrays and the API

*日本語版(secondary): [python-manual.ja.md](python-manual.ja.md)*

For the buttons, see the [calc](calc-manual.md) / [writer](writer-manual.md)
manuals. This is **the one document for people writing code** — in particular
the range ⇄ array exchange, which is invisible from the UI, is specified here.
Everything was measured on a real machine.

## Where Python runs, and what is bound

| Place | Bindings | Sandbox |
|---|---|---|
| calc: Data > Python (one-liner / .py) | `b` = workbook, `s` = current sheet | sandboxed if available (with network) |
| calc: `@name` (**a plugin .py** — procedures never travel in the workbook; decided 2026-08-08) | same | **always sandboxed** (no network; `net` enables it) |
| calc: `=PY("fn",…)` + `@計算` (only functions embed, via `@save 関数name`) | arguments passed as values (below) | always sandboxed (no network) |
| calc / writer: macros, plugins | calc: `b`/`s`; **writer: `d` = python-docx Document** | sandboxed |
| writer: in-page Python (HTML) | `form` = dict of field name → value | always sandboxed |

Everything runs **on a copy** — a failure leaves the sheet/document unharmed;
on success the result lands as **one undo step** (one Ctrl+Z even across
multiple sheets).

## The office_sheet (pysheet) API

```python
import office_sheet                     # inside calc it's pre-imported; b and s arrive bound
b = office_sheet.Book.open("form.xlsx")
s = b["SheetName"]                      # or by index: b[0]
b.sheet_names                           # ['見積書', …]
b.add_sheet("NewSheet")                 # error if the name exists
b.recalc()                              # recalculate before reading values
b.save("out.xlsx")                      # original parts preserved
b.unsupported                           # list of parts we couldn't read (empty = everything read)
```

### Reading and writing cells

```python
s["A1"]            # read: numbers are float, text is str, ☑/☐ is bool, formula cells give the computed value
s.formula("E2")    # the formula itself ("=SUM(B2:D2)"; None if not a formula)
s.display("E2")    # display string ("238" — through the number format)
s["A1"] = 100      # write: number
s["A1"] = "text"   #        text
s["A1"] = True     #        bool (shows as ☑/☐ in calc)
s["A1"] = "=B1*C1" #        formula (string starting with "=")
s["A1"] = date(2026, 8, 5)  # datetime.date/datetime/time → Excel serial number
s["A1"] = None     #        clear
```

- **Formatting is preserved** — writing a value never touches borders, merges,
  or number formats
- Empty cells read back as **None or ""** (never-touched cells are None; cells
  where an empty string was stored are "". Both are falsy, so `if s["A1"]:`
  usually suffices; to be precise use `s["A1"] in (None, "")`)

### Ranges as arrays — the main topic

**There is no range subscript** (`s["A2:C3"]` raises) and **no 2-D bulk
assignment** (`s["A1"] = [[…]]` raises). Arrays work like this:

```python
# read: values() is the whole used area as a 2-D list (rows × columns, 0-based)
rows, cols = s.shape          # (10, 6) — shape is a property (no parentheses)
v = s.values()                # v[0] = first row (headings), v[1][1] = value of B2
tbl = [r[0:3] for r in v[1:6]]   # cut out A2:C6

# write: loop cell by cell (row numbers are 1-based in A1 notation!)
data = [["pen", 10, 150], ["notebook", 5, 180]]
for i, row in enumerate(data):
    n = 2 + i                              # starting at row 2
    s[f"A{n}"], s[f"B{n}"], s[f"C{n}"] = row
    s[f"D{n}"] = f"=B{n}*C{n}"             # formulas are strings too
b.recalc()
```

### Round-tripping with polars

```python
import polars as pl
# sheet → DataFrame (first row as headings)
v = s.values()
df = pl.DataFrame({h: [r[i] for r in v[1:]] for i, h in enumerate(v[0])})

# DataFrame → sheet (below the headings)
for i, row in enumerate(df.rows()):
    for j, val in enumerate(row):
        s[f"{chr(65 + j)}{2 + i}"] = val
```

Aggregation, joins, and filtering belong on the polars side — that's the
division of labor (the sheet is the form; computation is Python's job).

## Arrays with =PY (UDF)

```
=PY("aggregate", A1:B10, 100, "甲")
```

- Range arguments arrive at your `def` as **row × column 2-D lists** of values
  (a single cell is a scalar)
- Return values: scalar → into the cell / **1-D list → spills downward** /
  **2-D list → spills down-right**. If the target area holds someone's data,
  it stops with `#SPILL!` (nothing is overwritten)
- Evaluation happens only on `@計算`, inside the sandbox. The function
  definitions come from a script embedded under a name starting with 関数
  via `@save`

```python
def aggregate(r, limit, kind):   # r = [[r1c1, r1c2], [r2c1, …], …]
    hit = [row for row in r if row[0] == kind and row[1] <= limit]
    return [[row[0], row[1]] for row in hit]   # 2-D → spills
```

## writer macros (d = python-docx)

**Full manual: [writer-macro-manual.md](writer-macro-manual.md)** — named
fields (`fill` / `extract` / `fields`), templates (`render` / `tpl_fields`,
docxtpl), the sandbox, and letting the AI write the script.

```python
# d is a python-docx Document. The API is exactly python-docx's
d.paragraphs[12].runs[0].text = "商号 例示工務店"
for r in d.paragraphs[12].runs[1:]:
    r.text = ""                  # write to the first run, empty the rest (keeps formatting)
fill("代表・商号", "例示工務店")  # named fields beat label-hunting — see the manual
```

Saving is writer's job (don't call d.save in the script).

## In-page Python (HTML forms)

```python
# form = dict of field name → value. Values you set are written back to the page
qty = int(form.get("qty") or 0)
form["total"] = qty * 150
```

## What the sandbox allows

- The real filesystem is **read-only**, your home directory is invisible, and
  the only writable place is a scratch area for the exchange. The network is
  **closed by default** — it opens only when you type `@name net` at that
  moment (the permission is never saved anywhere)
- Time-limited (procedures 60 s, =PY functions 30 s); overruns are killed
  and reported
- Libraries installed on the machine (polars, scipy, matplotlib, …) work
- `print` output appears in the status bar (report progress and counts there)

## Writing with an AI — a collaboration guide

You don't have to write macros yourself. **Ask an AI (Claude etc.), inspect,
run in the sandbox** — that is the intended workflow, including VBA
migrations. But AIs write for the common world (openpyxl, xlwings, VBA), so
**hand them this house's rules first**. Paste the block below as-is.

### Briefing for the AI (copy-paste)

```
Write Python for the following environment.

[calc macro] b (workbook) and s (current sheet) are pre-bound.
- read: s["A1"] (number=float, text=str, checkbox=bool; formula cells give
  the computed value. Empty is None or "". For the formula use
  s.formula("A1"), for the display string s.display("A1"))
- bulk read: s.values() (2-D list, rows × columns, 0-based); size is
  s.shape (a property — no parentheses)
- write: s["A1"] = value. Formulas are strings like "=B1*C1". None clears
- IMPORTANT: there is no range subscript (s["A2:C3"]) and no 2-D bulk
  assignment — write in a loop, one cell at a time. Row numbers in A1
  notation are 1-based
- after writing formulas call b.recalc() before reading values
- don't call b.save() (applying is the app's job). print goes to the app's
  status bar
- formatting (borders/merges/number formats) survives value writes — don't
  touch it

[writer macro] d (python-docx Document) is pre-bound.
Ordinary python-docx API. Don't call d.save().
When filling form fields: write to the first run and empty the rest
(p.runs[0].text = value; the remaining runs get "" — keeps paragraph
formatting)

[execution] inside a bubblewrap sandbox. Files are read-only (only the
copy of the sheet/document is writable), network is closed by default
(only opens when the user explicitly grants net). polars, scipy, and
matplotlib are available. A failure leaves the original unharmed; success
lands as a single undo step.

[when writing =PY cell functions] range arguments arrive as row × column
2-D lists of values. Return a scalar / 1-D list (spills down) / 2-D list
(spills down-right).
```

Then add **what you want, in plain language** (sheet name, heading row, what
should happen). If the table's shape matters, paste `s.values()[0]` (the
heading row) — it shortens the conversation.

### Inspecting the code you receive

Three checks before pasting — the sandbox protects **the machine and the
network**, not **the correctness of the result**:

1. **Where does it write** (does it touch columns/rows you must not lose?)
2. **What does it delete** (None assignments, row removals?)
3. **Does it need net** (if it talks to the network, is the destination the
   one you intended?)

Beyond that the safety net is threefold: execution on a copy (failure is
harmless), one-step undo (didn't like it? Ctrl+Z), and the sandbox (even
misbehaving code can't reach the machine). So the right way to try things is
**run it, look at the result, undo if you don't like it**. Once satisfied,
place procedures in `~/.config/office/plugins/name.py` (they run as `@name`).
**Only =PY functions may embed in a workbook** (`@save 関数name` —
decided 2026-08-08 for safety).

### Migrating VBA

For a workplace .xlsm, extract the VBA (the standard tool is `olevba` from
`oletools`), paste it to the AI, and ask for "the same job in Python for the
environment above". Range/Cells loops map naturally to `s[f"A{n}"]` loops,
Worksheet to `b["name"]`. **After migrating, run both against the same input
and compare** — the comparison is part of the migration.

### An example request

> (after pasting the briefing)
> Sheet 受注台帳: row 1 is headings (受付・社名・品番・品名・数量・
> 単価・金額・発送済). Write a macro that totals 金額 per 社名 and writes
> "社名, total" starting at J5 downward.

The AI writes → you inspect (is column J free? no net needed) → run → look →
maybe ask "sort by total, descending" next. That loop is the basic form of
the collaboration.

## Worked examples (readable as-is)

- [templates/](../templates/README.md) — an inquiry ledger (CSV intake with
  `@取り込み net`, status aggregation with =PY) and more
- [sample/注文書.xlsx](../sample/README.md) — swapping in a product master
  (`@更新 net`) and sending JSON (`@送信 net`)
- [sample/受注台帳.xlsx](../sample/README.md) — incremental intake that
  avoids duplicates with a watermark cell (K2)

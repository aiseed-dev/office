# Writer macro manual (Python)

*日本語版(secondary): [writer-macro-manual.ja.md](writer-macro-manual.ja.md)*

A writer macro is a Python script that edits **a copy of the open document**
inside a sandbox, using python-docx. If the script succeeds, writer loads the
edited copy back — one Ctrl+Z undoes the whole thing. If it fails, the document
is untouched. Documents never carry executable code: opening a file never runs
anything.

## Running a macro

Three ways:

1. **Plugins tab > Macros** — pick a `.py` file. The script body is your code
   only (see the contract below).
2. **Plugins tab > Manage plugins** — every `.py` in
   `~/.config/office/plugins/` is listed; click one to run it in the same
   sandbox.
3. **AI tab > Write a macro** — describe what you want; the model writes a
   script into the plugins folder (`ai台本N.py`). It is **never run
   automatically** — read it, then run it from the plugins list.

## The contract

Your script runs with these already done for you — do not repeat them:

```python
# already provided:            do NOT write:
d          # the document      import docx / docx.Document(...) / d.save(...)
```

- **`d`** is a python-docx `Document` (the full python-docx API works on it).
- **`print()`** — the last printed line appears in the status bar. End with a
  one-line report of what you did.
- **Refuse with `raise SystemExit("reason")`** when the request cannot be done
  safely. The message is shown to the user; the document stays untouched.
  Never fail silently.

## Named fields — the backbone of forms

Give fill-in fields names first: **Forms tab** inserts a field, the **Name**
button (cursor inside the field) sets its name — stored as `w:tag` in the
docx, so the names survive Word round-trips. Real Japanese forms repeat the
same label (様式1 has 住所/商号 three times: once per party) — section-prefixed
names like `代表・住所`, `協力1・住所` keep them apart.

| Function | Does |
|---|---|
| `fill(name, value)` | writes **every** field with that name (a form often repeats a field on page 2) |
| `fill_one(name, value)` | writes only the first one |
| `extract(name)` | reads a field's value |
| `fields()` | list of `(name, value)` pairs — the form's data sheet |

All of them **raise with a clear message when the name does not exist** —
a typo never becomes a silent no-op.

```python
fill("代表・商号", "日本フネン株式会社")
fill("担当・氏名", "山田太郎")
print("2カ所記入しました")
```

```python
# harvest a collected form into stdout (pair with calc/polars for a ledger)
for name, value in fields():
    print(name, value)
```

Do **not** search for label text and write into the neighboring cell — merged
cells return duplicates, 均等割付 pads labels with spaces, and repeated labels
hit the wrong party. Names exist so you never have to guess.

## Templates — mass-produced documents

For generate-and-send documents (notices, contracts, cover letters), use a
**template with `{{placeholders}}`** instead of named fields. Anyone can type
`{{顧客名}}` into the document in writer or Word; the docxtpl engine fills it.

| Function | Does |
|---|---|
| `render(context)` | fills the template with a dict; the result becomes the saved document |
| `tpl_fields()` | list of placeholder names — the template's spec sheet |

```python
render({
    "顧客名": "青森県庁",
    "担当者": "山田",
    "明細": [{"品名": "防火戸", "数量": 3}, {"品名": "枠", "数量": 6}],
})
print("差し込みました")
```

Repeating table rows use `{%tr %}` tags. **Each tag takes a whole table row**
— three rows total:

| 品名 | 数量 |
|---|---|
| `{%tr for m in 明細 %}` | |
| `{{m.品名}}` | `{{m.数量}}` |
| `{%tr endfor %}` | |

Pitfalls that actually break templates (run-splitting mostly does not —
docxtpl repairs it, and writer merges split runs on open):

- `{%tr %}` tags placed inside a data row instead of on their own rows
- **Full-width braces** `{{ }}` typed by the IME — use half-width `{{ }}`

## Using python-docx directly

Everything python-docx offers works on `d`: `d.paragraphs`, `d.tables`,
`d.sections`, styles, and so on. Two rules from real measurements:

- **Walk tables by rows.** `tb.cell(i, j)` rebuilds the merge grid on every
  call — on a 1,920-cell ruled form the loop below takes **91 ms** where the
  `cell(i, j)` version takes **12.7 s**:

  ```python
  for tb in d.tables:
      for row in tb.rows:
          for c in row.cells:
              ...
  ```

- **Preserve formatting when replacing text.** `c.text = "…"` resets the run
  formatting. Write into the first run instead:

  ```python
  p = cell.paragraphs[0]
  if p.runs:
      p.runs[0].text = "山田太郎"
      for r in p.runs[1:]:
          r.text = ""
  else:
      p.add_run("山田太郎")
  ```

## The sandbox

Macros run inside bubblewrap: **no network**, the **home folder appears
empty**, and the only writable place is the macro's own workspace. The
project `.venv` (python-docx, docxtpl, polars, …) is available. Practical
consequences:

- You cannot read `~/名簿.csv` from a macro. Bring data in as literals in the
  script, as a `render()` context, or from the document itself.
- Mass mail-merge over a ledger belongs in **calc's Python** (which binds the
  workbook) — writer macros edit the one open document.

## Letting the AI write it

**AI tab > Write a macro** sends the model this manual's rules (use only the
functions above, walk rows, report with print, refuse with SystemExit) plus
the **list of field names in the open document**, so it aims at real names.
The reply is saved to the plugins folder and shown in the list. Read it before
running — that is the whole safety model, and it is the same one used for VBA
migration: the machine writes, the human reads, the sandbox runs.

## When something goes wrong

| Message | Meaning |
|---|---|
| `python-docx がありません` | `pip install python-docx` (into `.venv` if present) |
| `docxtpl がありません` | `pip install docxtpl` — only needed for `render`/`tpl_fields` |
| `記入欄「X」が見つかりません` | no field named X — set names via Forms tab > Name, or check `fields()` |
| `雛形が壊れています: …` | template syntax — check `{%tr %}` row placement and full-width braces |
| `マクロ: <last line of the error>` | the script raised — the document was not changed |

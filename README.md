# office

*日本語版(secondary): [README.ja.md](README.ja.md)*

**Word and Excel that run on your machine.** Two apps for docx and xlsx, written in Rust.

- `writer` — opens, edits, and saves docx. Exports PDF ([manual](docs/writer-manual.md))
- `calc` — opens, edits, and saves xlsx. Calculates formulas ([manual](docs/calc-manual.md))

They are **separate apps**, not one giant suite.

## What works today

| | writer | calc |
|---|---|---|
| Open / save | docx (parts we don't understand are preserved as-is) | xlsx (same) |
| Japanese input (IME), undo | ○ | ○ |
| Character formatting | bold, italic, underline, strikethrough, color, highlight, super/subscript, size, font (**applies to the selection only**) | bold, italic, underline, strikethrough, color, fill |
| Paragraphs | alignment, bulleted/numbered lists (with levels), indent, line spacing, page break, shading and borders, drop caps | — |
| **First-class Japanese** | **vertical writing, ruby (furigana), distributed justification** (Text Direction toggles horizontal/vertical) | text orientation (vertical headings) |
| Form fields (content controls) | ○ (text fields, dropdowns, checkboxes — build a form, protect it, hand it out) | equivalent via data validation and checkboxes |
| Headings and TOC | ○ (TOC and table of figures with page numbers) | — |
| Header / footer | ○ (page number, page count, date) | — |
| Tables | ○ (reads/writes merged cells, edit inside cells) | (that's the whole app) |
| Images | insert PNG, JPEG, **SVG (converted at high resolution)**. Shapes/charts are drawn by Python and pasted | insert PNG, JPEG. Shapes, SmartArt, text art, equations (TeX), sparklines, symbols, checkboxes. Chart buttons are backed by matplotlib |
| Comments | ○ (per paragraph) | ○ |
| Track changes | ○ (saved as real Word tracked changes) | — |
| Bookmarks, watermark, page color, columns | ○ | — |
| Drawing (pen, highlighter, eraser) | ○ (becomes shapes in docx) | — |
| Formulas | — | arithmetic and functions (about 185, incl. dynamic arrays), recalculation, circular-reference detection, **=PY** (write your own functions in Python) |
| Sheets | — | multiple sheets, freeze panes, filter, slicer, sort, grouping and subtotals |
| Pivot tables | — | ○ (backed by polars; the definition is stored in the workbook so it can refresh) |
| Solver / goal seek | — | ○ (simplex LP, backed by scipy) |
| Protection, encryption, signing | ○ | ○ (read-only, AES, Ed25519 side-file signature) |
| Chat, version history | ○ | ○ (no server — plain files in a shared folder) |
| Conditional formatting, data validation | — | ○ (round-trips through xlsx) |
| Links, defined names, paste special | — | ○ |
| Portable Python (instead of macros) | macros run .py in a sandbox (`d` = python-docx document); code is never stored in the document | ○ (`@save` embeds code in the workbook; it always runs in a sandbox) |
| Print settings | paper, orientation, margins, columns | paper (incl. JIS B), orientation, margins, print area |
| PDF | ○ (headers/footers, watermark, ink and all) | ○ (borders, fills, follows print settings) |
| Find and replace | ○ | ○ |
| Cross-references to bookmarks | ○ (Word REF/PAGEREF fields) | — |
| Hyphenation (Latin text) | ○ (same patterns as TeX) | — |
| Proofreading | ○ | — |

The ribbon layout follows Euro-Office, so people who switch don't have to relearn where things are.
**Commands that don't work yet are shown grayed out** — we never make something look usable when it isn't.

**There are no VBA-style macros.** Instead, calc can **carry Python inside the workbook**
(`@save name` to embed, `@name` to run, `=PY(…)` as a cell function). Opening a file never
executes anything, and code that arrived inside a workbook always runs in a sandbox
(bubblewrap) — the "open = execute" attack path does not exist here.
See the [calc manual](docs/calc-manual.md).

## Running it

Requirements: Rust (1.80+), Japanese fonts, and on Linux either Wayland or X11.

```bash
cargo build --release

./target/release/writer            # opens empty
./target/release/writer sample/報告書.docx   # bundled sample (all contents fictitious)
./target/release/calc  sample/見積書.xlsx
```

The first build takes a while because it fetches GPUI (from zed).

### Fonts

**Not bundled.** The typeface is part of the document, so we look up the font names
written in the docx/xlsx among the fonts installed on this machine. If a name is missing
we fall back to a font that can typeset Japanese.

```bash
OFFICE_FONT=/path/to/font.ttf ./target/release/writer   # explicit override
```

Having `fonts-noto-cjk` or `fonts-ipaexfont` installed is enough.

### Proofreading (writer: Review > Proofread)

English spelling is checked against a dictionary (`/usr/share/dict/words` etc.).
Japanese misconversions and inconsistent spellings can't be caught by a dictionary,
so we ask a local model.

```bash
OFFICE_HOST=127.0.0.1 OFFICE_PORT=8000 OFFICE_MODEL=... ./target/release/writer
```

Anything that speaks the OpenAI-compatible `/v1/chat/completions` works.
**If it can't connect, the app says "can't proofread"** — it never silently reports
"no issues found".

There is also a standalone tool:

```bash
cargo run --release --bin office-spell -- document.txt
cargo run --release --bin office-spell -- --furigana draft.txt
```

## Division of labor with Python

**The app is for shaping things while you look at them; Python is for producing data
and drawings.** There are buttons for charts, SmartArt, equations, pivots, and the
solver, but the workers behind them are Python (matplotlib, polars, scipy).
For heavier analysis, use polars or statsmodels directly.

For xlsx there are Python bindings (`pysheet`). Unlike openpyxl, values can be
inserted while **borders, merged cells, column widths, and shapes stay intact**.

```python
import office_sheet
b = office_sheet.Book.open("form7.xlsx")
b["Sheet1"]["A30"] = "Nihon Funen Co., Ltd."   # formatting is preserved
b.save("out.xlsx")
```

There is no equivalent for docx — python-docx already does the job
(and writer is verified to read documents saved by it).

The reference for programmers is the [Python manual](docs/python-manual.md) —
ranges vs. arrays (`values()` and per-cell writes), 2-D lists and spilling with
`=PY`, the `d` binding in writer macros, and what the sandbox allows. All verified
on a real machine.

## Layout

```
engine/   kumihan — typesetting core (line breaking, kinsoku, glyph widths, page geometry)
ooxml/    docx reading and writing
sheet/    xlsx reading and writing, formula engine, styles (styles.xml)
lang/     language-specific logic; knows nothing about gpui, runs headless
paper/    projects the page onto PDF
ui/       glue to gpui (input, IME, ribbon)
writer/   the docx app
calc/     the xlsx app
pysheet/  Python bindings for sheet (import office_sheet)
```

**The screen and the paper are the same page projected onto different surfaces**,
which is why display and print never disagree.

## Localization

Per-language differences are contained in `Language` in the `lang` crate; implementing
one trait is enough. UI strings can be generated from Euro-Office locales (45 languages).

```bash
python3 ui/gen_ribbon.py --list      # available locales
python3 ui/gen_ribbon.py en > ui/src/ribbon.rs
```

## License

**AGPL-3.0-or-later** (`LICENSE`). Origins of bundled and derived material are
listed in `NOTICE.md`.

## Status

Ribbon commands: **writer 124/124, calc 145/145 — zero grayed-out buttons; everything
works** (2026-08-04). Design decisions are recorded in `SEKKEI.md` (Japanese),
history and open items in `HIKITSUGI.md` (Japanese), and there are ready-to-open
samples in `sample/`.

# A guide for people coming from Excel (it doubles as the distillation ledger)

*日本語版(secondary): [from-excel.ja.md](from-excel.ja.md)*

Started 2026-08-08 (the client: "why not distill Excel's manual into officework's?").
We take **the list of tasks in Excel's official help as our teacher** and write, task
by task, "that job you used to do in Excel — here is how you do it here (or why we
don't)". No prose is copied — only the structure and the inventory of tasks are the
teacher (the same practice as the ledger that used the vendor as a teacher for
behavior).

Second draft (2026-08-08): sixteen sets of tasks were thickened against the real
code. Detailed operations are sent off by link to the
[calc manual](calc-manual.md) and the [Python manual](python-manual.md) (not
transcribed here). The inventory of what remains lives in the
[ledger](guide-tsukiawase-2.ja.md).

Third draft (2026-08-08): the eighteen areas the second draft listed as "sections
still to fill" (cell formatting basics, the body of find & replace, consolidating
data, accessibility, and the rest) were filled in with the same pattern (parallel
research → adversarial verification). With that, we have been once around the
territory Excel's help covers.

How to read the marks:

- **Same** — the Excel habit carries over unchanged
- **Different** — the tool has a different shape, but the same job gets done here
- **Not yet** — unimplemented. We don't hide it (what remains is written in the ledger; anything not yet in the ledger is added as a candidate for inventory)
- **By design** — a design decision. The reason is given in one line

## Getting started — the screen and basic operations

The tabs are in Excel's order (File / Home / Insert / Draw / Layout / Formulas /
Data / PivotTable / Table Design / Collaboration / Protection / View / Plugins +
AI). The band is a single row of icons; hover one and its name appears in the
status bar below. As a rule we don't place buttons that do nothing when pressed —
the few gray buttons are there to mark "the place, and only the place" (see the
Protection section). For details see "The screen" and "Basics" in the
[calc manual](calc-manual.md).

| Excel's name | Mark | How it works here |
|---|---|---|
| New, Open, Save | Same | The File tab, plus Ctrl+O / Ctrl+S. xlsx is the canonical save format; there is a CSV export (UTF-8 with BOM, CRLF, values only). ODS and XLTX are pending (the ledger) |
| Recent workbooks | Same | Twelve of them on the File tab |
| Add, delete, rename, move, copy a sheet | Same | Right-click the tab. Moving is not a drag but "Move left" / "Move right". Renaming makes `oldname!` inside formulas follow along, and the status bar reports how many were fixed (but **text inside a string does not follow** — as in `INDIRECT("oldname!A1")`; Excel behaves the same, since INDIRECT exists precisely to build a reference that doesn't move. **The number left behind is announced in the status bar at rename time**, so you can fix them by hand) |
| Hide/unhide sheets, tab color | Same | Right-click. Hidden sheets also come back from View > "Show sheet". The last remaining sheet cannot be hidden |
| Insert and delete rows and columns | Same | Right-click gives the same four choices as Excel. The Home buttons are fixed to rows (left in the ledger) |
| Change column width and row height | Same | Drag the boundary, or type a number from the right-click menu (0–255 / 0–409pt) |
| Double-click the boundary to autofit | By design | Removed, so that autofit can't hijack the drag when you re-grab a boundary (decided by the client 2026-08-03) |
| Hide and unhide rows and columns | Same | "Hide" and "Unhide" on the header's right-click menu (**implemented 2026-08-08**). To unhide, first select across the hidden part. The container is the same one grouping uses, and it round-trips through xlsx (so it stays hidden for the next person). **You can't hide everything that's in use** — we refuse, because the way back would become invisible |
| Freeze panes | Same | From the View tab: top row, first column, current position, unfreeze. A green line marks the split, and the shadow can be toggled |
| Zoom | Same | 50–200% (narrower than Excel's 500%). Paper is unaffected. The on-screen text size is a separate setting under advanced options |
| Split the window | Not yet | For headers, freeze panes; to keep an eye on values, Watch on the Formulas tab stands in (not in the ledger — a candidate for inventory) |
| New window, Arrange All | Different | One workbook, one window. To place them side by side, launch again and use your OS's windows. We don't build the path where clicking another workbook writes an external-reference formula (see "Bringing data in" below) |
| Toggle formula bar, gridlines, headings | Same | View tab. There's a "Show zeros" toggle too |
| Sum, average and count in the status bar | Same | Shown live as you select. They respect the filter |

## Entering data and formatting

The finer points of cell formatting (borders, fills, text alignment, wrapping,
vertical text and so on) are still thinly compared against Excel's help — see
"Sections still to fill" at the end. What's here is only what has been verified.

| Excel's name | Mark | How it works here |
|---|---|---|
| Autofill (dragging the ■) | Different | There is no fill handle. Select the first row, extend with Shift+↓, and press "Fill" on Home — it copies the first row downward (downward only). Relative references shift, and formatting is copied too |
| Building a series (1, 2, 3…) | Different | Fill only copies; it never guesses. For a running number, spill `=SEQUENCE(10)`, or fill `=A1+1` down. There are no custom lists for weekdays or month names |
| Flash Fill (Ctrl+E) | Not yet | The AI tab's "Continue writing", or Data > Python, take its place (not in the ledger — a candidate for inventory) |
| Paste values only | Same | Ctrl+Shift+V. Also from "Paste Special" on the right-click menu |
| Paste Special | Different | Four choices — values only / formulas as they are / formatting only / transpose (values). No operations, no skip blanks, no column widths |
| Format Painter (the brush) | Same | Next to Paste. It paints the next cell you click (Shift for a range); Esc to stop |
| Number formats (thousands separator, currency, date…) | Same | On the Home list, with a ✓ on the current one. Accounting and fractions are pending (the ledger) |
| Custom number formats | Different | "Other (type a format code)…" takes the code directly. Codes are stored in xlsx as they are, so they round-trip with Excel |
| Merge & Center | Same | The same four choices as Excel. **Values other than the top-left one are lost** (if the top-left is empty, the first content is moved there with its formatting first). No confirmation dialog — the status bar says what happened, and one Ctrl+Z takes it back; the design keeps a hidden value from feeding SUM and letting a report lie quietly |
| Insert shapes (AutoShapes) | Different | Six only (rectangle, rounded rectangle, ellipse, right arrow, diamond, line). You can put text inside them. There is no large gallery (the ledger) |
| Manipulating and formatting shapes | Same | Move, resize, rotate (Shift for 15°), Ctrl+click multi-select with align/distribute, z-order, and a fill/line/shadow panel. Round-trips with xlsx |
| Grouping shapes | Not yet | The button is gray, marking the place only (the ledger). For now, multi-select and move them together |
| Insert a picture | Different | PNG/JPEG/BMP/GIF. Move, resize, Del. Rotation and cropping are pending. **Pictures that were already in a workbook from elsewhere cannot be moved** (so the original isn't damaged — and we say so) |
| Text box | Different | A rectangle shape with text in it. No paragraph settings (bullets and the like) (the ledger) |
| Line break inside a cell (Alt+Enter) | Same | Wrap text is on Home as well |

## Cell formatting basics

The formatting tools are nearly all on the Home band; they work when pressed, and
they round-trip with xlsx. The differences show up in two habits — borders are not
Excel's stamped presets but an **orthogonal place × pen model**, and "Format Cells"
is not a six-tab dialog but a small panel you can leave open while you work. For
details see the [calc manual](calc-manual.md).

| Excel's name | Mark | How it works here |
|---|---|---|
| Change the font and font size | Same | Pick from the Home lists (the button shows the current value). Only the Japanese-capable fonts present on this machine are listed; if there are none, we say "not found". Sizes are a list from 6 to 72, plus "Grow/Shrink" stepping 4–72pt one point at a time. There is no box for typing an arbitrary size |
| Bold, italic, underline, strikethrough | Same | Toggled from the Home buttons (the Ctrl+B/I/U/5 keys are not wired — see the shortcut table). Italic slants only when the font has an italic face, so Japanese fonts don't slant (we don't fake it). No double or accounting underline variants |
| Font color and fill color | Same | The Home buttons open a palette (font color: automatic + 10 colors; fill: no fill + 5 light + 5 dark). "Other (type RRGGBB)…" takes a code directly. There is no eyedropper and no grid of theme colors. No gradient or pattern fills (solid only) |
| Draw borders (outline, all borders, thick, double, colored) | Different | We don't carry stamps like "Thick Bottom Border". We carry exactly two orthogonal things — a place (nine of them: bottom, top, left, right, outline, all borders, inner vertical, inner horizontal, clear) chosen from a grid palette of icons, and a pen (twelve line styles, plus color) chosen separately. Want a thick outline? Set the pen to medium and choose outline. The palette doesn't close when you apply, so you can build a form's frame with repeated clicks. Round-trips with xlsx |
| Diagonal border in a cell | Not yet | A cell has four edges only; the diagonal isn't in the model. Diagonal borders in someone else's xlsx are not read and **are lost on save** (one of the few places where the original isn't carried over). If the diagonal means "void", strikethrough or a line shape stands in for now (the ledger) |
| Text alignment (left/center/right, top/middle/bottom, justify) | Same | The Home buttons give three horizontal plus justify, and three vertical. The defaults match Excel — numbers right, text left, vertical at the bottom. Justify turns wrapping on with it (you need wrapping in order to justify). There is no distributed alignment |
| Wrap text | Same | Toggled from the Home button. Both the on-screen drawing and the xlsx (wrapText) round trip are in place. It works together with an in-cell line break (Alt+Enter) |
| Shrink to fit | Not yet | There is no toggle button yet. Cells that already carry it from xlsx are read correctly, drawn shrunk until they fit the width, and not lost on save — only the control is missing (the ledger) |
| Vertical and angled text (text orientation) | Different | The orientation button on Home (still labeled "Page orientation" — a naming slip) offers six presets (no angle, 45° up / 45° down, 90° up / 90° down, vertical = one character stacked on the next) plus "Other (type an angle)…" for −90 to 90 degrees. The current orientation gets a ✓, and the value round-trips. But **the screen does not rotate the glyphs** — anything other than 0° is shown as characters stacked one per line. That is exactly what Japanese forms want for vertical headings, but 45° text only slants once you open it in Excel |
| Superscript and subscript | Different | Subscript toggles from a Home button and round-trips. Superscript isn't in the model and is pending (no button on calc's Home) |
| Indent (inside a cell) | Not yet | calc has neither an indent button nor a field in the model (writer's paragraphs do have one). For now, type leading spaces or split the column (the ledger) |
| Cell styles (the built-in set: Heading, Good/Bad and so on) | Different | "Cell Styles" on Home offers ten (Normal, Heading, Title, Good/Bad/Neutral, Note, Calculation, Currency, Percent). Choosing one applies formatting to the selection, and a single Ctrl+Z takes it back. There is no big gallery, no registering your own style, and no "modify style" that updates everywhere — what gets applied is plain formatting, so you fix it by fixing the formatting |
| Workbook themes and theme colors | Different | There is no switch for a whole theme (colors + fonts + effects); Layout > "Change color scheme" swaps between four color sets (Office, warm, cool, ink). Theme-derived colors remember where they came from and round-trip, so changing the scheme moves those cells' colors with it, and saving doesn't flatten them to rgb. "Interface theme" on the View tab is the light/dark of the screen, not the workbook's theme (cells stay white). We don't carry theme font or effect sets |
| The Format Cells dialog (Ctrl+1) | Different | Not a six-tab dialog but the "Cell format" panel — borders, fill, font color, font effects, alignment, wrapping and number format laid out on one sheet as a toolbox. Its Home label reads "Cell Styles", the same name as the real style list beside it, which makes the two hard to tell apart (the panel's own title is "Cell format"). It isn't modal: leave it open and keep re-selecting ranges (close with ✕ or Esc), and every button is one Ctrl+Z per step. The Ctrl+1 key isn't wired |

## Aids for entering data

Excel's "helpful" machinery — custom lists, form controls floating on the sheet,
grouped sheets that bundle several at once — all share one property: **they keep
state outside the grid**. The line in this house is "state lives in cells": a list
is a range on a sheet, a control is the cell itself (checkboxes), and writing to
several sheets is an explicit naming under Data > Python. We build neither
guesswork nor a path that silently rewrites a sheet you can't see. For details see
the [calc manual](calc-manual.md).

| Excel's name | Mark | How it works here |
|---|---|---|
| Create a custom list (Options > "Edit Custom Lists") | Not yet | There is no settings screen for registering a list, and no mechanism in the app to hold one at all. For now, write the list down one column of a sheet and use it as a range (point data validation's dropdown at it as the source, or reference it as a rank table for MATCH) (not in the ledger — a candidate for inventory) |
| Import a custom list from a cell range | Not yet | There is nowhere to import into, so a range stays a range — that is the way here. Put the list column on a sheet, define a name, and point formulas at it; then revising the list is just editing cells |
| Autofill your own series (drag through "Tokyo, Osaka…" and cycle) | Not yet | There is no fill handle, and Home's "Fill" only copies the first row downward without guessing. To cycle, copy the whole list column and append as many as you need, or put `=INDEX($A$1:$A$5, MOD(ROW()-1,5)+1)` at the top and copy it down with "Fill". SEQUENCE inside INDEX won't overflow — spilling happens only for FILTER, SORT, UNIQUE, SEQUENCE and TRANSPOSE combined with arithmetic, comparison and &; inside an ordinary function they collapse to a single value |
| Weekday and month series (drag from "Mon" for Tue, Wed, Thu…) | Different | There are no built-in weekday or month lists. Weekdays and month names come from dates, not from text — run dates down a column and turn them into 水 or 水曜日 with a number format or `=TEXT(A1,"aaa")` / `"aaaa"`, and into 8月 with `"m月"` (the Latin mmm does not give a month name; it comes out as a number like 08). The advantage is that sorting and arithmetic still work, because they are still dates |
| Date series by unit (fill by day, weekday, month, year) | Different | There is no right-drag unit menu (the ledger). Write the unit in a formula — day `=A1+1`, weekday `=WORKDAY(A1,1)`, month `=EDATE(A1,1)`, year `=EDATE(A1,12)` — put it at the top and copy it down with "Fill". Fill shifts relative references as it copies, so each copy looks at the previous date and produces the next |
| The Series dialog (linear, growth, stop value, step value) | Different | There is no dialog. A linear column comes out of a single spill: `=SEQUENCE(count, 1, start, step)`. For growth, fill `=A1*ratio` down; for a stop value, convert it into a count and decide for yourself |
| Fill without formatting ("Fill Without Formatting" in the autofill options) | Not yet | "Fill" is designed to make a form's column uniform, formatting included; there is no values-only choice. If you'd rather not keep the formatting, select the range afterwards and clear formats (values survive) to tidy it up |
| Sort by a custom list (job titles, S/M/L) | Not yet | Custom sort takes several criteria — header names ordered strongest-first from the left — but the direction is only ascending or descending; you can't name your own order. For now, add a helper column with `=MATCH(value, list range, 0)` for the rank within the list, and sort ascending on that column |
| Sort into an arbitrary order with SORTBY | Not yet | SORTBY doesn't exist (it's stated in the function table). Add the rank helper column inside the range and get the same result with the spill `=SORT(range, rank column number, 1)` |
| Sort PivotTable row labels by a custom list | Not yet | Pivots can't be sorted at all yet (the ledger), let alone by a list order. A pivot's result is just cells, so either put a rank table in the space to its right and rearrange by hand, or add a rank column to the source data and sort it beforehand |
| Do custom lists travel with the workbook? | Not yet | The app has no mechanism for holding lists, so there is no ground on which to ask the question yet. The way here is to hold the list as a range on a sheet and point at it by a defined name — hand over the workbook and the list goes with it, which is in fact easier to carry, since it doesn't depend on the environment |
| Show the Developer tab and use form controls | Different | There is no Developer tab at all. The jobs are split: checkboxes belong to the Insert tab, and Python (the successor to macros) to the Plugins tab. No trip through Options to reveal a tab |
| Insert a checkbox into a cell | Same | Insert tab > "Checkbox". Placed in an empty cell it becomes ☑/☐ (the value is a TRUE/FALSE bool). Cells with content aren't crushed, and the status bar says so |
| Toggle a checkbox with the keyboard and use its value in formulas | Different | Space toggles it, the same as Excel. The value is an ordinary TRUE/FALSE, so `=E4` and `=IF(E4,…)` work as written. But a bare TRUE passed as a criterion, `=COUNTIF(E:E,TRUE)`, returns 0 — to count them, use one of `=COUNTIF(E:E,"TRUE")`, `=COUNTIF(E:E,1)`, `=SUMPRODUCT(--(E:E=TRUE))`. On a protected sheet the toggle is refused, with the reason given |
| Place or delete checkboxes over a whole range | Same | Select a range and insert: they go into the empty cells only, and we say how many were placed. Deleting is just Delete, which removes the value along with it — there is no double bookkeeping between a control and a value |
| Keeping ☑/☐ across a round trip through Excel | Different | We save plain TRUE/FALSE and don't write Excel 365's checkbox format (the control's appearance). Opened in Excel they read as the words TRUE/FALSE, and values and formulas carry over completely. Conversely, any cell holding TRUE/FALSE shows as ☑/☐ on the officework side |
| Form-control checkboxes and their "cell link" | Different | We don't make controls that float above cells; the cell itself is the control (a bool value). There is no separate "cell link" step tying a control to a cell — the cell is the value |
| Step a number with a spin button or scroll bar | Not yet | There are no stepper controls. Type the number into the cell, or use data validation's whole-number/decimal range to hold it between bounds — that's the way today (not in the ledger — a candidate for inventory) |
| List boxes and combo boxes for picking from a list | Different | Not a floating control: data validation's dropdown list does the same job. Allow = List, either typed inline or as a range reference, and pick from the ▾ at the cell's lower right. The chosen value lands in that cell, so no "cell link" is needed |
| Option buttons (radio) and group boxes for an exclusive choice | Not yet | There are no radio buttons or group boxes (writer's form fields have them; calc doesn't). An exclusive choice is done with a data-validation dropdown for now (not in the ledger — a candidate for inventory) |
| Open a workbook with form controls, see them, and keep them after saving | Not yet | Legacy form controls (VML) and control references are not read and don't appear on screen. On save we rebuild the sheet body and vmlDrawing, so the original's control references aren't carried over and are lost. Shapes and pictures are carried over; controls are outside that (not in the ledger — a candidate for inventory) |
| ActiveX controls (command buttons, text boxes, etc.) | By design | ActiveX is inseparable from VBA events, and the first principle of this house is to never let "opening is running" into a workbook. If you need something that acts when pressed, put Python in plugins and call it explicitly yourself |
| Select several sheets at once (Ctrl+click for a group) | Not yet | The selection is always a single sheet. Clicking a tab merely switches to it; modifier keys are ignored. The very concept of a grouped selection doesn't exist (not in the ledger — a candidate for inventory) |
| Select all sheets (bulk selection from the tab's right-click menu) | Not yet | The tab's right-click menu has nine items — insert, delete, rename, duplicate, move left, move right, hide, unhide, tab color — and every one of them acts on a single sheet. There is no select-all item (not in the ledger — a candidate for inventory) |
| Typing into a group (type on one sheet, it lands in the same cell on all) | Not yet | Entry always goes to the current sheet. To write the same thing to several sheets, name them explicitly with `b["sheet name"]` under Data > Python — touching several sheets still undoes with one Ctrl+Z (not in the ledger — a candidate for inventory) |
| Formatting, or inserting and deleting rows and columns, across a group | Not yet | Formatting and row/column insertion and deletion act on the current sheet only. If you want several sheets to look alike, finish one and multiply it with the tab's "Duplicate" (not in the ledger — a candidate for inventory) |
| Ungrouping, and forgetting to ungroup (wrecking every sheet) | Not yet | There is no grouped state, so there is neither an ungroup operation nor the accident of forgetting. For anyone burned by this in Excel, the absence is itself the answer |
| Print selected sheets, print the whole workbook | By design | Printing (= PDF) is always the current sheet — a UI for switching what gets printed is a simplification we deliberately don't carry. For several sheets, export a PDF per sheet |
| Apply page setup to several sheets at once | Not yet | Layout tab settings apply to the current sheet. To match them, either set each sheet, or multiply a configured sheet with "Duplicate" and then fill in the contents (not in the ledger — a candidate for inventory) |
| Fill across a group (Fill ▸ Across Worksheets) | Not yet | Fill goes downward within one sheet. To copy to the same place on several sheets, copy and paste on each, or put a script in plugins that writes to `b["sheet name"]` from Data > Python (not in the ledger — a candidate for inventory) |
| Multiply a form across months or departments | Different | Finish one sheet, layout and all, and multiply it with "Duplicate" on the tab's right-click menu. Formatting and formulas come with it, and renaming makes the formula references follow. Instead of fixing everything at once through a group, the order is: settle the template first, then multiply |
| Delete or hide several sheets at once | Not yet | Deleting and hiding are one sheet at a time from the tab's right-click menu. The guard that the last sheet can't be hidden or deleted is in place |

## Number precision and the traps of typing

The container for numbers is a single f64; we don't apply Excel's cosmetic of
"cutting at fifteen significant digits". In exchange, **we don't guess at what you
typed either** — nothing turns into a date, but by the same token neither a leading
`'` nor a text number format changes how input is read. With the protective tools
this thin, work that needs a number held exactly as text is, for now, something we
are bad at. For number formats, see "Formatting" in the
[calc manual](calc-manual.md).

| Excel's name | Mark | How it works here |
|---|---|---|
| The fifteen-significant-digit wall (the tail of a 16-digit card number turning into 0) | Different | The value is kept as f64 and never truncated to fifteen digits — 4111111111111111 comes out as typed. But integers beyond 2^53 (about 9 quadrillion) shift under binary rounding: type 9999999999999999 and you get 10000000000000000. There is currently no escape hatch of typing it as text to protect it (see "Entering a number as text" below) |
| Long numbers switching to scientific notation (1.23E+11) on their own | Different | The General (unformatted) display never switches to exponent form — however many digits there are, all of them are shown (type 1e21 and you get 1000000000000000000000; 1e-7 gives 0.0000001). To see exponents, pick "Scientific (1.23E+04)" from Home's number format list explicitly |
| Part numbers like "2E3" or "1E9" read as exponents and turned into numbers | Same | Input is parsed by Rust's f64 parser, so 2E3 becomes 2000 — the same trap as Excel. The spellings inf and nan are read as numbers too, and appear on screen as inf and NaN |
| Floating-point error (0.1+0.2 isn't 0.3) | Different | The display wears no makeup: it shows 0.30000000000000004 as it is. Formula comparison, on the other hand, is lenient by one f64 step (about 2.2e-16), so `=0.1+0.2=0.3` is TRUE — but `>` is not lenient, so `=(0.1+0.2)>0.3` is TRUE as well, and you get the contradiction of = and > both being true at once. Comparison between ranges (spilled) is strict, with no leniency. To round to a digit, write ROUND, ROUNDDOWN or ROUNDUP into the formula |
| Reducing the decimal places rounds only the look | Same | Home's "Increase/Decrease Decimal" only rewrites the decimal places in the format code; the stored value doesn't change (it caps at 0–10 places). The calculation always uses the true value — the same as Excel's default |
| The "Precision as displayed" option | Not yet | There is no option to truncate stored values to the displayed digits. To calculate on rounded values, put ROUND into the formula (which is the recommended path in Excel too) (the ledger) |
| Numbers stored as text (left-aligned, and not counted by SUM) | Different | SUM and AVERAGE read "strings that are just digits" inside a range as numbers, so nothing quietly drops out of a total. Exact matching in VLOOKUP and MATCH compares displayed text too, so "123" and 123 match, and numeric criteria in SUMIF and COUNTIF match as well. What doesn't match is COUNT (which counts only Numbers) and ISNUMBER (FALSE). To turn them back into numbers, put =VALUE (which also reads through ¥ and thousands separators) or NUMBERVALUE in another column and paste values only (Ctrl+Shift+V) |
| The green-triangle error checker (error indicators, "Convert to Number") | Not yet | There is no corner mark, no background error-checking rule, and no bulk fix. To inspect text-numbers, write =ISTEXT and =ISNUMBER in a helper column and sift them yourself (the ledger) |
| Full-width digits (123 committed straight out of the IME) | Not yet | Excel normalizes to half-width numbers on commit; committing a cell entry here does no normalizing, and full-width digits simply become text. Worse, full-width doesn't even benefit from SUM's text reading and counts as 0, and it fails data validation (whole number, decimal) too. Normalizing applies only to the criterion value typed into the validation panel; for data you already have, fix it with =ASC to half-width and then =VALUE (the ledger) |
| Numbers typed with separators or symbols (1,234 / ¥1,234 / 5%) | Not yet | Any spelling the f64 parser rejects becomes text. Type numbers plain and half-width, and add the look with Home's number formats (thousands separator, currency, percent). For what's already typed, =VALUE reads through ¥, the separators and %, so put it in another column and paste values only to get it back (the ledger) |
| Entering a number as text (the leading ', the "Text" number format) | Not yet | The leading zeros of 007 disappear just as in Excel, but the protecting side has no tools — the leading-' convention doesn't exist (the ' becomes part of the text), and applying the "Text (@)" format beforehand doesn't change how input is read. Format codes won't zero-pad either (apply 0000000 and 7 is still 7), so if you need padding, build it in a formula like `=RIGHT("0000000"&A1,7)`. A number column created as text on the Excel side stays text when you open it and save it again — it turns into a number only if you retype it (the ledger) |
| Input turning into a date on its own (1-2 becoming February 1; the SEPT1 gene-name problem) | Different | Input is read as one of exactly three things — number, TRUE/FALSE, or text — and no date is guessed, so this accident doesn't happen. The flip side: type 2026/8/8 and it stays text rather than becoming a serial number, and =TODAY() gets no automatic date format either, coming out as a bare serial — apply "Short Date" or another format and only then does it look like a calendar date (for date text, DATEVALUE gives the serial) |
| CSV import turning a number column numeric (wanting to mark a column "Text") | Not yet | The import wizard is pared down to encoding, delimiter and destination; there is no per-column type. Every field goes through the same reading as typed input, so leading zeros fall off and 2E3 becomes a number. The Data tab's "Text to Columns" only asks what to split on, not the column's data format. For a column you must protect, read it with Python in plugins (polars, with dtypes given) — that is the sure path (the ledger) |

## Find, replace, and Go To

Find is a single panel that asks for the search text and then the replacement —
leave the replacement empty and it's find only. The biggest difference from Excel is
that **there is no Options expander**. Find is always fixed to "case sensitive,
partial match, current sheet only", which is the opposite of Excel's default
(insensitive), so it's easy to feel that nothing is being found. For details see the
[calc manual](calc-manual.md).

| Excel's name | Mark | How it works here |
|---|---|---|
| Find (Ctrl+F to look for text, Find Next) | Different | The way in is the "Replace" button on Home, or the 🔍 at the right edge of the sheet tab strip. Two panels come in sequence (search text → replacement). Press Enter with the replacement empty and it's find only — it searches from the cell after the current one (rows, then columns) and wraps to the top at the end. The count and address appear in the status bar, and the words stay in the panel next time, so repeated empty Enters hop from hit to hit. Ctrl+F and Ctrl+H are assigned but have no receiver in calc; pressing them does nothing |
| Replace (Ctrl+H, Replace All) | Different | Fill in the replacement in the same panel and press Enter, and the whole current sheet is replaced at once (including text inside formulas). Formatting is preserved, we report how many places changed, and one Ctrl+Z takes it back. It acts on the entire sheet even with a range selected, and rows hidden by a filter are replaced too |
| Replacing one at a time (the "Replace" button, the "Find All" list window) | Not yet | Replace is bulk only; there is no one-at-a-time confirmation and no window listing the matches. Scout the hits with empty-Enter find, replace in bulk, and Ctrl+Z if you don't like it — that's the way for now (the ledger) |
| Find options (Match case, Match entire cell contents) | Not yet | There is no options expander at all. Find is always fixed to "case sensitive, partial match" (full-width vs half-width and hiragana vs katakana are distinguished too). There is no switch for picking up whole-cell matches only (the ledger) |
| Wildcard search (* and ?, with the ~ escape) | Not yet | * and ? are treated as ordinary characters. Since plain find is always a partial match, "contains" needs no *, but the "starts with A and ends with B" pattern can't be expressed. The formula side is the same: SEARCH only ignores case, and text criteria in COUNTIF and SUMIF are exact matches, so to sift by pattern, combine FILTER and FIND, or for anything complex go to regular expressions under Data > Python (the ledger) |
| Find by format, replace formatting (finding only the yellow-filled cells) | Not yet | Find looks at text only; there is no way to key on formatting. Replace preserves the original formatting (and there is no path to swap formatting wholesale either). Sorting keyed on color, "put the selected cell's color on top", sometimes stands in for the neighboring job (the ledger) |
| Switching where to look (sheet / whole workbook) | Not yet | Both find and replace are always the current sheet. There is no switch for searching the whole workbook at once; change sheets and press 🔍 again (the ledger) |
| Switching what to look at (formulas / values / comments) | Different | There is no switch: we always search both the formula as typed and the displayed value (text inside =SUM matches, and so does the look of 3.14). Replacement acts on the formula side only. We don't look inside cell comments |
| Switching direction (by rows / by columns, search up) | Not yet | The order is fixed to rows-then-columns, and there is no backwards search. It wraps around to the top at the end, so one lap meets everything (the ledger) |
| Go To (F5/Ctrl+G, typing an address into the Name Box) | Different | Type B12, A1:C9 or a defined name into the Name Box and press Enter to jump there (a range becomes the selection). Typing an unknown name to name the current selection works too, just as in Excel's Name Box. But there is no F5/Ctrl+G window or key, and the Name Box only takes what's on the current sheet — the `Sheet2!A1` form isn't accepted, and names are held per sheet. To reach another sheet, step on its tab, or jump by hyperlink |
| Go To Special (bulk-select blanks, constants, formulas, conditionally formatted cells, validated cells) | Not yet | There is no tool for picking out cells by condition. Follow blanks or conditionally formatted cells by eye, use formulas such as FILTER and COUNTBLANK, or sift them out with Data > Python for now (the ledger) |
| Jump to precedents and dependents (Go To Special's precedents/dependents, tracing) | Different | The Formulas tab's tracing takes this job — it lights up the precedent/dependent cells rather than drawing arrows, and reports the count ("Remove Arrows" clears it). The selection does not jump to what lights up |
| Select visible cells only (Alt+; — copying only the rows a filter leaves showing) | Not yet | There is no visible-cells selection. Filtering is only an appearance and isn't even kept on save, so if you want just the visible rows, spill them elsewhere with FILTER, or extract them with =PY. For the sum, average and count "of what's showing", the status bar statistics are the proper path today (the ledger) |

## Bringing data in from other systems (Get & Transform / connectors)

Excel's answer is Power Query and Copilot connectors — have your core business data indexed in the cloud, then pull it into the grid. **officework's answer is local**: the data never leaves your internal network. An import always brings in the values as of that moment, and no query definition is left behind in the workbook. Examples: [the inquiry ledger](../templates/README.md) and [the order ledger](../sample/README.md).

| Excel's name | Mark | How it works here |
|---|---|---|
| From Text/CSV | Same | Data tab > "Text" opens a wizard (encoding, delimiter, destination, with a three-row preview) and pours the rows in as values. Not going through Power Query means fewer steps |
| Choosing the encoding (against mojibake) | Same | Auto / UTF-8 / Shift_JIS (CP932) / Latin-1. Auto tries them in order and **reports which encoding it used** — it will not garble your text in silence |
| Choosing the delimiter | Same | Auto / comma / tab / semicolon / colon / space / any other single character. Decimal separator, thousands separator and text qualifier are held back (on the ledger) |
| From Web | Different | There is no built-in button. Put a procedure (.py) in `~/.config/office/plugins/` and run it with `@name net`, which gives it a sandbox with network access for that one run, and have it append to the ledger. **A procedure can never be started from a workbook** |
| From Database | Different | There is no GUI connector. A procedure in plugins queries the database with polars or the like. Drivers live in your own Python environment |
| From Another Workbook | Different | Data tab > "External link" imports a whole sheet from another xlsx **as values** (formulas become values; the source workbook's name goes into the sheet name) |
| Live external references (=[Book1.xlsx]…) and Edit Links | By design | So that no form ever goes out with broken or stale references (a deliberate trade-off recorded in [the ledger](guide-tsukiawase-2.ja.md)). If you want current numbers, import again |
| Shaping data in the Power Query Editor | Different | You write the shaping in polars (Python) — the code itself is the written procedure. There is no M language and no recorded list of steps ([design](sekkei/python.ja.md)) |
| Refresh All | Different | Re-run the plugins procedure with `@name` (it keeps a record of what was already imported, so nothing is duplicated). **Open-means-run does not exist here** — refreshing is always an explicit human action |
| Text to Columns | Same | On the Data tab. A simplified version that splits on one delimiter (there is no three-step wizard and no fixed-width mode) |
| Combine from Folder | Different | Write it with polars in a plugins procedure (no sample is shipped — the assumption is that you or an AI writes it) |
| Importing tables from PDF | Not yet | For officework, PDF is the exit that printing goes out through (not on the ledger yet — a candidate for it) |
| Importing XML | Not yet | The wizard handles CSV and text only (noted on the ledger). If you need it now, use a plugins procedure |
| From Jupyter, or existing pandas/xlwings work | Same | Change one line: `from officework import calc as xw`. The only channel is a socket inside this machine (no TCP port is opened) |
| Copilot connectors | By design | AI and data ingestion are not bundled together — there is no cloud-indexing mechanism here at all ([design](sekkei/ayumi.ja.md): native first) |

## Linked data types (Stocks, Geography)

Excel's data types work by sending a company name or a place name to a cloud matching service and pulling back an entity. **officework has no such mechanism** — data does not leave your internal network. If you need stock prices or geography, fetch them from an API in a plugins procedure (.py) as **the values at that moment** and keep them as ordinary columns.

| Excel's name | Mark | How it works here |
|---|---|---|
| Converting cells to the Stocks data type | By design | There is no mechanism for sending company names to a cloud matching service (native first). If you need prices, run a plugins procedure with `@name net` and pull the current values from a securities or market-data API. What arrives is always values; no live connection stays in the workbook |
| The Geography data type (population, area, prefectural capital) | By design | Matching a place name to an entity is left out for the same reason as stocks. Import from a public statistics API or a local CSV with a plugins procedure and keep the result as ordinary columns — columns take the place of fields |
| Expanding fields (opening the card, the `A1.Price` dot notation) | By design | There is no container in which a cell holds an entity and produces fields later. Here you lay out the attributes you want as columns at import time — plain cell references and VLOOKUP/INDEX/XLOOKUP are enough after that |
| FIELDVALUE | By design | Since the data type it would read does not exist, the function is not provided either. Type it and it says #NAME? honestly (never a silent 0). Take attributes out with ordinary references into the imported columns |
| STOCKHISTORY | By design | The function assumes a cloud market-data feed, so it is not provided. If you need history, have a plugins procedure shape yfinance or similar output with polars and pour it in as a table of values — the code itself is the written procedure |
| The Currency (exchange rate) data type | By design | The same decision as stocks. Query an FX API from a plugins procedure and put the rate of that moment in as a value. To use it in a formula, reference the imported cell |
| Refreshing stock and geography data (Refresh All) | Different | There is no live connection, but there is a way to import again: run the plugins procedure once more with `@name` (or `@name net` if it needs the network) and replace the values with the current ones (see "Refresh All" in the section above) |
| Updating prices automatically on open or on a schedule | By design | Open-means-run does not exist; that is this software's first principle of safety. If you need scheduled updates, run pysheet (import name `office_sheet`) from cron as ordinary Python — the pattern is to run it outside the form |
| Wolfram data types and organization data types (Power BI) | By design | There is no mechanism for indexing your organization's data in the cloud and querying it — the same decision as Copilot connectors. Core entities are fetched straight from the database or API by a plugins procedure |
| Opening and saving a workbook that contains Excel data types | Different | No cards and no fields appear; the cells show only the cached values recorded in the xlsx (often the error value #VALUE!). The richData parts are carried through verbatim as parts we do not understand, but the sheet itself is rebuilt on save, so **the `vm` attribute that ties a cell to its entity is definitely lost** — taken back to Excel, the data types have degraded |

## Formulas and functions

You write them the same way — start with `=`, and arithmetic, comparisons, ranges and the way `$` behaves are all as in Excel. An unknown function name becomes #NAME?, and **it is never quietly computed as 0**. See "Formulas and functions" in [the calc manual](calc-manual.md).

| Excel's name | Mark | How it works here |
|---|---|---|
| Entering a formula | Same | About 190 functions are implemented (see the table below) |
| Relative and absolute references ($) | Same | The same rules on copy, fill and paste |
| Name completion, argument hints, Insert Function | Same | Completion as you type, plus "Insert Function" on the Formulas tab (category, then function). Only the Shift+F3 key is unwired (on the ledger) |
| Defining and managing names | Same | You can also type straight into the Name Box. There is no F3 "Paste Name" — completion offering the names stands in for it (on the ledger) |
| References to another sheet (Sheet2!A1) | Same | `=Sheet2!A1` and `=Apr!B2` work written out directly (**implemented 2026-08-08** — until then they were #ERROR!). Ranges too: `=SUM(Apr!B1:B5)`; names containing spaces or symbols go in quotes, `='Apr Actuals'!B2`. An unknown sheet name gives #REF! — it is never quietly read as your own sheet. The value is **a copy of the value at that moment**, so fix the target and a recalculation catches up. `INDIRECT("Sheet2!A1")` still works as before |
| 3-D references (=SUM(Sheet1:Sheet3!A1)) | Same | `=SUM(Apr:Jun!B2)` works (**implemented 2026-08-08**). It gathers every sheet that lies between the two **in workbook tab order** (reorder the tabs and the gathered range changes — same as Excel). Writing the two in the other order gives the same result; an unknown sheet name gives #REF! |
| Spilling (dynamic arrays) | Same | If something is already in the way you get #SPILL! instead of losing it. Nesting and arithmetic on spills both work |
| Legacy array formulas (Ctrl+Shift+Enter) | Not yet | Dynamic arrays spill automatically on plain Enter. For sum-of-products, use SUMPRODUCT (on the ledger) |
| R1C1 reference style | Same | Advanced settings, "Reference style". The file is still saved as A1, so the round trip is safe |
| Error values and IFERROR | Same | Propagation behaves as in Excel. The one thing of our own is #CIRC! for circular references — shown in the cell rather than as a warning plus a 0 |
| Circular references and iterative calculation | Same | Detected and shown as #CIRC!. For deliberate circularity, turn on "Iterative calculation" in advanced settings (100 iterations and 0.001 by default; round-trips through calcPr) |
| Automatic/manual recalculation and F9 | Same | F9 recalculates everything, Shift+F9 only the sheet |
| Auditing formulas (Show Formulas, tracing, Watch Window) | Different | Show Formulas (Ctrl+`) is the same. Tracing lights up the cells involved instead of drawing arrows. The watch list is a strip along the bottom, not a window of its own |
| Structured references (=Table1[Amount]) | Same | `=SUM(Sales[Amount])` works (**implemented 2026-08-08**). It points at the data body only — the header row and the totals row are not included. For the current row, `[@Amount]` (inside the table; from outside, `Sales[@Amount]`). An unknown table or column gives #REF!. The nested form `[[#Headers],[Column]]` is not accepted yet (it is a formula error — noted on the ledger) |
| =PY (Python in Excel) | Different | Not in the cloud — **in a local sandbox, and free**. It runs only when you ask for Data > Python > Calculate (`@計算`); neither opening the file nor recalculating runs it. Excel shows #NAME? when it opens the file, but the values remain |

## Function compatibility table

Nothing missing is hidden. Write a name that does not exist and it says #NAME? honestly. The same list, by category, is also in "Insert Function" on the Formulas tab (laid out as in Excel).

| Excel function | Here? | Alternative / notes |
|---|---|---|
| VLOOKUP, HLOOKUP, XLOOKUP, LOOKUP, INDEX, MATCH | Yes | **Exact match only** (MATCH takes match type 0 and nothing else; there is no approximate match). LOOKUP covers the sorted "largest value less than or equal to" case |
| XMATCH, SORTBY | No | MATCH and SORT cover the ground that is needed |
| SUMIF, COUNTIF, AVERAGEIF, SUMIFS, COUNTIFS, AVERAGEIFS, MAXIFS, MINIFS, SUMPRODUCT | Yes | Same syntax |
| IF, IFS, SWITCH, CHOOSE, AND, OR, NOT, IFERROR, IFNA | Yes | IF does not step on an error in the branch it did not take |
| TODAY, NOW, DATE, DATEDIF, EDATE, EOMONTH, WORKDAY, NETWORKDAYS, YEARFRAC, WEEKNUM | Yes | The holiday-list argument is accepted too |
| LEFT, RIGHT, MID, FIND, SEARCH, SUBSTITUTE, TRIM, TEXT, TEXTJOIN, CONCAT | Yes | TEXT's format codes work as well |
| REPLACE (replace by position) | No | Use SUBSTITUTE, or a combination of LEFT and MID (a candidate for the backlog) |
| FILTER, SORT, UNIQUE, SEQUENCE, TRANSPOSE | Yes | The five that spill |
| RANDARRAY, VSTACK/HSTACK, TAKE/DROP, TOCOL and the other extended spill functions | No | (a candidate for the backlog) |
| LET | Yes | `=LET(x, SUM(A1:A9), x/COUNT(A1:A9))` — give an intermediate result a name so it is not computed twice (**implemented 2026-08-08**). Nesting works, and the names live only inside the LET |
| LAMBDA | No | Excel puts the expression into a defined name and calls it, but **our names can hold nothing but ranges**, so it is deferred (the reason is recorded on the ledger). A Python function under =PY does the same job ([the Python manual](python-manual.md)) |
| TEXTSPLIT, TEXTBEFORE, TEXTAFTER | Yes | **Implemented 2026-08-08.** TEXTSPLIT spills across, and down as well if you pass a row delimiter. TEXTBEFORE/TEXTAFTER accept the instance number, a negative number to count from the end, and a value to use when nothing is found. An empty delimiter gives #VALUE! — it does not quietly hand back the whole string |
| DSUM, DAVERAGE, DGET (the D functions) | No | Use SUMIFS, AVERAGEIFS or FILTER — even Excel now points to the IFS family (a candidate for the backlog) |
| PMT, PV, FV, NPER, NPV, IRR, RATE | Yes | IRR and RATE are solved iteratively, as in Excel |
| SUBTOTAL, AGGREGATE | Yes | **101–111 skip rows you hid by hand, including rows collapsed in an outline** (implemented 2026-08-08; 1–11 count everything, same as Excel). **Rows hidden by a filter, however, are still counted** — filtering is a screen-side state that the formula evaluator cannot see (noted on the ledger). For a total, average or count over only the visible rows, the statistics in the status bar do respect the filter |
| PHONETIC, ASC, JIS, DBCS, LENB/LEFTB/RIGHTB/MIDB, DATESTRING, YEN | Yes | The standard Japanese set (Japanese era dates, counting full-width characters as 2) |
| =PY (a cell function written in Python) | Yes (and here first) | Local sandbox, free. This function — a UDF — is the only Python a workbook is allowed to carry |

## PivotTables

The bones of the workflow are Excel's: fields, filtering, grouping, refresh. The aggregation underneath is done by polars. See "PivotTables" in [the calc manual](calc-manual.md).

| Excel's name | Mark | How it works here |
|---|---|---|
| Creating a PivotTable | Different | Put the cursor in the table, choose Insert, and answer the questions — rows, columns, values, aggregation. The result is placed as values in the empty space to the right of the table (nothing is overwritten in silence). You cannot name a destination |
| Recommended PivotTables | Not yet | You put rows, columns and values together yourself (not on the ledger yet — a candidate for it) |
| Rearranging fields | Different | Not a drag-and-drop side pane but the "Field list" — it loads the current instructions with their check marks, you choose again, and the table is rebuilt in the same place |
| Value Field Settings | Different | Five aggregations in the field list: sum, average, count, max, min. A grand-total average is recomputed from the source data, so it is correct |
| Show Values As (% of total, running total, difference) | Different | "Show values as" on the PivotTable tab, with four choices: as-is, percentage, running total, difference (**implemented 2026-08-08**). Percentage shows each value as a % of the grand total. **Running total and difference drop the subtotals and the grand total** — a total sitting in the middle of an accumulation invites misreading. This is not all dozen-plus of Excel's |
| Filtering from the ▼ (items, labels, values) | Same | Check boxes, plus "Filter by label…" and "Filter by value…". The filter is kept in the workbook as part of the instructions and still applies after a refresh. There is no Top 10 and no search box |
| Report filters, and connecting slicers/timelines | Not yet | There are three places to put a field: rows, columns and values. Put the field you want to filter into rows or columns and use its ▼ (a candidate for the backlog) |
| Grouping (month/quarter/year, numeric bins) | Same | Dates it cannot read are left as they are, not dropped in silence. You cannot set start and end values, and there is no +/− to expand and collapse |
| Refresh, Refresh All | Same | The definition is kept in the workbook, so it just rebuilds |
| Change Data Source | Not yet | If the table has grown, delete the PivotTable and insert it again (the range is detected automatically) (a candidate for the backlog) |
| PivotTable styles | Different | Four sets: blue, green, orange, grey. There is no large gallery |
| Report layout, grand totals, subtotals, blank rows | Same | Two layouts, tabular and compact (there is no outline form). Every one of them is a single Ctrl+Z away |
| Sorting inside a PivotTable | Not yet | Outstanding on the ledger |
| PivotCharts | Not yet | The result is just cells, so you can select the range and insert a chart — but it will not be redrawn on refresh (a candidate for the backlog) |
| Round-tripping with Excel | Different | In Excel it looks like a table of values. The definition lives in a part of our own (joPivot.xml), so **saving the file again in Excel drops it and refresh stops working** — degradation we say out loud |

## Tables, sorting and filtering

| Excel's name | Mark | How it works here |
|---|---|---|
| Creating a table (Ctrl+T) | Same | Insert tab > "Insert table". It round-trips as an xlsx table and looks like a table in Excel too. The Ctrl+T key is not bound |
| The table styles gallery | Not yet | One fixed colour. You can still turn banding, the header row and the first/last column on and off (on the ledger) |
| Total row | Different | "Total row" adds a row of =SUM. To change the aggregation, right-click and pick from the eight choices under "How to total" rather than a ▼ in the cell; the formula is rewritten as =SUBTOTAL (for how this meets filtering, see the note in the function table above) |
| Resize, Convert to Range | Same | On the Table Design tab. Formatting and formulas stay |
| Structured references (=Table1[Amount]) | Not yet | Use a range or a defined name (a candidate for the backlog) |
| Sorting (ascending, descending) | Same | If your selection is part of a table, you get the same three choices as Excel — expand the selection, sort the selection only, or cancel |
| Custom sort on several columns | Different | Not a dialog where you add one level at a time: you type "Amount desc, Item". The result is the same multi-level stable sort |
| Sort by colour | Same | Right-click > Sort > "Put the selected cell's colour on top", and so on |
| AutoFilter | Same | The ▼ gives you value check boxes, a search box and a count. **But it is only a way of seeing, and it is not saved** — close the file and it is gone |
| Number/text/colour filters, Top 10 | Not yet | Outstanding on the ledger for the second edition |
| Slicers | Different | Put the cursor in a column, press "Slicer", and the panel is there. ≡ for multiple selection, ✕ to clear. It affects only what is displayed |
| Slicer settings | Not yet | Fixed width, fixed style, always ascending (on the ledger) |
| Remove Duplicates | Same | Turn the columns to compare on and off, say how to treat the header, and get a report of how many rows were removed. It always works on the whole table |
| Subtotal | Different | It asks you two things — which heading breaks the groups, which heading to total. The function is always SUM, the groups collapse with +/−, and one Ctrl+Z undoes it |
| Extracting unique values | Same | =UNIQUE spills. Use Remove Duplicates to delete them in place; use this to build a list off to the side |

## Outline (grouping)

The bones are Excel's — group, hide and show detail, subtotal. What differs is the screen: there is no outline margin down the left edge (no level bars, no numbered level buttons). Instead a +/− disc appears on the **row heading** immediately after the group. Depth and collapsed state round-trip through xlsx, so a ledger you collapsed reaches the next person still collapsed. See "Data" in the [calc manual](calc-manual.md).

| Excel's name | Mark | How it works here |
|---|---|---|
| Group rows (Data > Group) | Same | Drag across the row headings (the numbers) to select, then Data > Group. Depth goes up by one, and a single Ctrl+Z undoes it |
| Group columns (the rows-or-columns dialog) | Different | Nothing asks you rows or columns — the shape of the selection decides. Whole columns picked from the column headings group as columns; anything else groups the rows of the selection. Columns collapse and expand too, but the +/− disc only ever appears on rows, so reach for Hide Detail / Show Detail on the ribbon for columns |
| Outline symbols and level bars | Different | There is no outline margin at the left edge. A +/− disc appears on the heading of the row just after the group, and pressing it collapses or expands (summary-below layout) |
| Hide Detail / Show Detail | Same | Two buttons on the Data tab. With nothing selected they act on the whole run of the group the cursor's row belongs to |
| Nested groups (up to 8 levels) | Same | Grouping again increases the depth, topping out at outlineLevel 1–7 (the ECMA-376 ceiling). Excel's "8 levels" counts the base plus 7 — the same limit |
| Level number buttons (1 2 3 … to open and close everything at once) | By design | The +/− discs and Hide/Show Detail cover the job, so we left them out (2026-08-07) |
| Auto Outline (inferred from the direction of the formulas) | Not yet | There is no button that reads the formulas and builds the outline for you. If a collapsed view showing totals is what you want, Subtotal does that job (on the ledger) |
| Clear Outline (wipe it all at once) | Different | There is no single button that removes everything. Ungroup peels off one level at a time, and rows or columns that reach depth 0 lose their collapsed (hidden) state along with it — for deep nesting, press it once per level |
| Automatic outline from Subtotal | Same | Data > Subtotal inserts a =SUM subtotal row at each break plus a grand total, and groups the detail rows. Collapse it and only the subtotals and the grand total remain. The whole thing is one Ctrl+Z |
| Saving the groups and the collapsed state | Same | Depth round-trips as outlineLevel, collapse as hidden. Open the file in Excel and it looks like an ordinary outline. Even the depth of empty rows survives (there are round-trip tests) |
| Printing collapsed rows and columns | Same | Collapsed rows and columns stay out of the print (PDF) too. Collapse the sheet into the shape you want to show, then print |
| Do inserts and deletes knock it out of alignment | Same | Insert or delete rows and the group depth and the collapsed state travel with the row (tested) |

## Consolidation and the analysis tools

Excel has three answers here: the Consolidate dialog, the data model (Power Pivot), and the Analysis ToolPak. **officework's answer to all three is polars (Python)** — no analysis engine rides inside the workbook; joins, aggregation and statistical tests are the job of Data > Python or of a procedure in plugins. The report is where results land; the heavy data and the hard arithmetic stay outside it. See the [Python manual](python-manual.md).

| Excel's name | Mark | How it works here |
|---|---|---|
| Consolidate by position (adding up monthly sheets laid out alike) | Not yet | There is no Consolidate button. When the layouts match, `=Apr!B2+May!B2+…` written by hand is enough. With many sheets, hand the addition to polars in Data > Python (not on the ledger yet — a candidate for it) |
| Consolidate by category (matching on the top row and left column labels) | Not yet | Absent. Stack up `=SUMIF(Apr!A2:A50,$A2,Apr!B2:B50)+…`, one term per sheet. When the rows don't line up, or there are many sheets, polars' concat and group_by are the sure route — and inconsistent labels get cleaned up in the same pass (a candidate for the ledger) |
| Consolidation functions (count, average, max, min, stdev) | Not yet | With no dialog there is nothing to choose. Swap in COUNTIFS, AVERAGEIF(S), MINIFS, MAXIFS. Only standard deviation has no …IFS form — in polars, changing the agg covers all of them |
| 3-D references (=SUM(Sheet1:Sheet3!B2)) | Same | Gathers every sheet between the two, in workbook tab order (**implemented 2026-08-08**; also listed under "Formulas and functions" above) |
| "Create links to source data" (keeping the consolidation live) | Not yet | The Consolidate feature is missing entirely, but a hand-built consolidation is formulas, so fixing the source lets the recalculation catch up — the same property links would give you. The polars route produces the value as of that moment, so when the source changes you run it again (the same idea as Refresh on a pivot) |
| Including ranges from other workbooks in a consolidation (=[Book1.xlsx]…) | By design | We do not create live external references — no broken links, no reports quoting stale numbers. Bring the other workbook in through External Links as sheets of values, then take the ordinary same-workbook route |
| Combining data from several workbooks | Different | Not Power Query and not Consolidate: Data > External Links pulls in every sheet of the other workbook as values with the formulas dropped (the source workbook's name is prefixed to the sheet names, so nothing collides). From there it is all one workbook |
| PivotTable from multiple consolidation ranges (the old wizard) | Not yet | A pivot takes exactly one table — the selection, or the table auto-detected around the cursor. Stack the sheets into one with polars' concat in Data > Python first, then insert the pivot |
| Stacking detail sheets (combining monthly detail and dealing with duplicates) | Not yet | There is no button for it. In Data > Python, collect each sheet's values() and concat them, settle the duplicates with unique or group_by, and write the result back. For routine work, keep the .py in plugins and run it. By hand, it is copy-paste plus Remove Duplicates |
| Add to Data Model (the in-workbook data model) | By design | No analysis engine rides inside the workbook. Analysis and aggregation go to polars — that is where the line is drawn — and the pivot itself is built straight from the table by polars behind the scenes. There is no intermediate container |
| Relationships (joining two tables on a key column) | Different | Write a polars join in Data > Python or in a plugins procedure. To pull back a single column, VLOOKUP or XLOOKUP (exact match only) is enough |
| PivotTable across multiple tables | Different | A pivot always reads one table. Join them into one first, put that on a sheet, and insert the built-in pivot on it |
| Creating measures (DAX) | Different | We do not carry a second language called DAX — the language of computation here is Python, full stop. Write custom aggregations with polars' group_by/agg, or factor them out into a =PY UDF |
| Distinct count | Not yet | The built-in pivot offers five aggregations: sum, average, count, max, min. For now, polars' n_unique in Data > Python (on the ledger) |
| Calculated columns (DAX) | Different | Add an ordinary formula column to the right of the table (relative references plus Fill to reach every row). polars' with_columns does the same job |
| Defining KPIs (against a target, with status icons) | Not yet | No dedicated tool. The nearest route is a formula column for the attainment rate plus a conditional-formatting icon set (which round-trips through xlsx) for the signal |
| Loading millions of rows into a model and analyzing them | Different | Big data does not go into the workbook. A plugins procedure has polars read the CSV or the database directly and puts only the aggregated result on the sheet — the report is where results land, the data itself lives in the file or the DB |
| GETPIVOTDATA, CUBEVALUE | Different | The functions themselves are absent; write one and it says #NAME? honestly. A pivot's output is laid down as ordinary cells, so a plain cell reference or XLOOKUP reaches it |
| Date tables (a month / quarter / year axis) | Different | We do not build date tables. The pivot's grouping (month / quarter / year, or a numeric bucket width) does the same job, and dates it cannot read are left as they are rather than silently dropped |
| Time intelligence (year-over-year, running totals) | Different | The pivot's Show Values As now includes running total and difference from previous (**implemented 2026-08-08**). Comparisons across periods such as year-over-year are absent; use polars' shift in Data > Python |
| Opening and saving a workbook that contains Power Pivot | Different | The model never appears on screen but is carried through verbatim — saving does not break it, and reopening in Excel gets it back intact. Excel's own pivots show up as values on the sheet, but officework cannot Refresh them (Refresh works only on our own joPivot) |
| Enabling the Analysis ToolPak (getting the Data Analysis button) | Different | There is no add-in installation step at all. Statistics and forecasting go through Data > Python (polars, scipy, statsmodels) from the start — that is the designated route |
| Histogram | Different | No dedicated dialog. Make a column of bins, count with =COUNTIFS, and draw it with Insert > Chart (column). To do it in one pass, compute the frequencies in Data > Python and write them back |
| FREQUENCY (a whole distribution in one array formula) | Not yet | The function is missing. COUNTIFS builds the same frequency table for now (not on the ledger yet — a candidate for it) |
| Moving average | Different | No dedicated tool. Write a window formula such as =AVERAGE(A1:A3) and copy it down with Fill on the Home tab. For long series, polars' rolling. There is no way to lay a trendline over a chart |
| Regression (slope, intercept, fitted values, correlation) | Different | There is no dialog that emits the full output, but simple regression is covered by functions — SLOPE, INTERCEPT, FORECAST(.LINEAR), CORREL. For R², square CORREL (RSQ and STEYX are absent). When you need p-values and residuals, that report is statsmodels' job |
| LINEST, TREND (multiple regression, array fitting) | Not yet | The functions are missing. Multiple regression is statsmodels in Data > Python, the designated route (not on the ledger yet — a candidate for it) |
| Random numbers (RAND, RANDBETWEEN) | Same | They work as written, and they are volatile on every recalculation just as in Excel. RANDBETWEEN with the bounds reversed gives #NUM!, same as Excel |
| Random numbers from a distribution (normal, binomial) | Different | Neither the Random Number Generation dialog nor distribution functions such as NORM.INV exist. Generate them with scipy or numpy in Data > Python and write the values into the cells (the script runs against a copy, and one Ctrl+Z undoes it). The distribution functions are not on the ledger yet — a candidate for it |
| RANDARRAY (filling a range with random numbers at once) | Not yet | Absent. It is already on the ledger as part of the extended-spill backlog. For now, select the column, enter =RAND() and fill (downward only) |
| Descriptive Statistics (one table of mean, median, stdev) | Different | No dialog produces the table, but the functions are all there — MEDIAN, MODE, STDEV(P)/VAR(P) (the .S/.P spellings too), PERCENTILE, QUARTILE, LARGE, SMALL. Kurtosis, skewness (KURT, SKEW) and GEOMEAN are absent (candidates for the ledger). If you want it as a single table, build it in polars and write it back |
| Rank and percentile | Different | Functions rather than a tool — RANK(.EQ/.AVG), PERCENTILE, QUARTILE. Ties behave as in Excel (EQ gives the same rank, AVG averages the ranks) |
| Correlation (a matrix across many columns) | Different | For two columns, =CORREL. There is no dialog for the whole matrix — build it in Data > Python (polars) and write it back. Covariance (the COVAR family) is absent (a candidate for the ledger) |
| t-test, F-test, ANOVA, chi-squared | Different | Neither the test functions (T.TEST and its relatives) nor the dialogs exist. Tests are scipy.stats and statsmodels in Data > Python, the designated route — inside the sandbox, with no network, and usable as long as they are installed on the machine (the functions are not on the ledger yet — a candidate for it) |
| Exponential smoothing, FORECAST.ETS, Forecast Sheet | Not yet | Forecast Sheet is an open item on the ledger. Only the straight-line forecast (FORECAST / FORECAST.LINEAR) is there. For smoothing and seasonal forecasts, statsmodels in Data > Python for now |

## Conditional formatting

The rule types we support round-trip through xlsx; the ones we cannot read are **reported when the file opens, then dropped** — never silently. The formatting we apply ourselves is fixed (pale green, pale red and the like); there is no format-picker dialog.

| Excel's name | Mark | How it works here |
|---|---|---|
| Highlight Cells Rules (greater than / less than / between / text contains) | Same | From the Home tab or the right-click menu. Type the threshold or the text and that is all |
| Duplicate values, unique values | Same | Round-trips as duplicateValues / uniqueValues |
| Top/Bottom N items | Same | Round-trips as top10 |
| Top/Bottom percent | Not yet | Excel's percent rules are reported as "will be lost on save" and dropped (noted on the ledger) |
| Above/Below average | Same | The "n standard deviations" variants are absent |
| Data bars | Same | Colors set in Excel are read and kept. Ours are always blue |
| Color scales (2-color, 3-color) | Same | Percentile stops are approximated along a straight line between the minimum and the maximum |
| Icon sets | Same | The iconSet name is preserved across the round trip. There is no screen for setting the thresholds (cfvo) |
| Formula rules | Not yet | If banded rows were the point, a table's Banded Rows/Columns does that job (noted on the ledger) |
| Date rules (timePeriod) | Not yet | Noted on the ledger |
| Blank and error rules | Not yet | Noted on the ledger |
| Manage Rules | Different | A list, plus Move and Delete. There is no editing, no priority ordering and no preview — **the way here is to delete a rule and apply it again**. Where rules overlap, the one applied later wins |
| Clear Rules | Same | It tells you how many it removed |
| Does it print | Different | Fills and font colors carry onto paper. **Bars, scales and icons are screen-only** (noted on the ledger) |

## Charts and sparklines

Charts are drawn by matplotlib from the selected range and float on the sheet **as images** — they are not Excel chart objects. The picture holds the values as of the moment you inserted it; it does not follow the data.

| Excel's name | Mark | How it works here |
|---|---|---|
| Insert a chart | Different | Insert > Chart, and a column chart appears as an image. Japanese fonts are registered from the machine's own set, so the text never comes out as tofu boxes |
| Recommended Charts | Different | The button is there, but what it recommends is always a column chart — there is no cleverness inside that looks at your data |
| Chart types (pie, line, …) | By design | Column charts only, a deliberate narrowing (see the policy section of the [ledger](guide-tsukiawase-2.ja.md)) |
| Change the type afterwards, Select Data, automatic refresh | By design | It is an image, so none of this exists. Delete the old picture, reselect the data, and insert again |
| Editing chart elements (title, axes, legend) | By design | Only the legend appears, and it appears automatically once there are two or more series |
| Move, resize, delete a chart | Same | Anchored to a cell with a pixel offset, and round-trips through xlsx |
| Opening a workbook that has Excel charts | Different | We do not draw them on screen, but we **carry them through verbatim** — saving does not break them, and reopening in Excel gets them back intact |
| Insert a sparkline | Same | Select a range of numbers and use the Insert tab. It lands in the current cell |
| Types (line, column, win/loss) | Same | All three. They are written to xlsx as one shape per bar, so Excel shows the same picture (with round-trip tests) |
| Color, markers, axis settings | Not yet | A fixed single green (on the ledger) |
| Deleting a sparkline | Different | Not Clear from the right-click menu — click it and press Del, the same as a shape |
| Following the source data automatically | By design | The shape holds the values as of insertion. Delete it and insert again (the policy is on the ledger) |

## Hyperlinks

Links are attached through a single panel — a URL, then a second panel for the display text — rather than Excel's large dialog. One default is deliberately reversed: **Ctrl+click opens the link**, and a plain click merely selects the cell, tipping things toward never jumping somewhere by accident. Nor does a URL you type turn itself into a link. See the [calc manual](calc-manual.md).

| Excel's name | Mark | How it works here |
|---|---|---|
| Insert Hyperlink (the link dialog) | Different | Hyperlink on the Insert tab or in the right-click menu opens a panel where you type a URL or `#SheetName!B5`. Instead of Excel's big four-way dialog (existing file / place in this document / create new document / e-mail address), it is one panel followed by a second for the display text. A cell holding a link turns blue, and putting the cursor on it shows the URL as a hint |
| Ctrl+K to add a link | Same | Opens the same panel. The same key as Excel |
| Opening a link (click to jump, or select the cell without jumping) | Different | Ctrl+click opens it; a plain click selects the cell. The default is deliberately the reverse of Excel's, so neither the accidental jump nor the click-and-hold-to-select problem arises. External URLs are handed to the OS default browser |
| Linking to a place in the workbook (Place in This Document) | Different | Instead of a URL, type `#SheetName!B5` (`#B5` and `#A1:C9` work too). Ctrl+click jumps to that sheet, cell or range. You write the destination as text rather than picking it out of a tree of sheets |
| The display text (what the cell shows) | Different | After the URL panel, a second panel asks for the display text and puts it into the cell. Press Enter on an empty panel and the cell's contents are left alone. It is a follow-on panel, not a field in a dialog |
| Editing an existing hyperlink (changing the target) | Same | Press Ctrl+K again (or right-click) on the cell holding the link. The panel opens pre-filled with the current URL; edit it and press Enter |
| Removing a hyperlink (one cell) | Different | Open the link panel, clear it, and press Enter. There is no dedicated Remove Hyperlink item in the right-click menu |
| Clearing hyperlinks across a range | Same | Hyperlinks is one of the Clear options. It removes only the links in the selected range, leaving values and formatting behind. The status line says how many went, and Ctrl+Z brings them back |
| Stopping typed URLs from becoming links | By design | Nothing auto-links in the first place — a URL you type stays plain text. There is no AutoCorrect machinery at all, so there is no setting to hunt for. If you want a link, add one explicitly with Ctrl+K |
| HYPERLINK function | Different | `=HYPERLINK(link_location, [friendly_name])` works, and it displays the friendly name (or the location if there isn't one). But a formula does not carry the machinery to jump — for that, attach a link to the cell with Ctrl+K (jumping is the cell link's job) |
| ScreenTip (the hover text) | By design | It does not fit the one-panel shape, so we passed on it (noted on the ledger with the reason). Instead, the cell under the cursor shows the target URL itself as a hint |
| Linking to a defined name | Not yet | `#SomeName` does not jump — a link target is read as an address only, with no lookup of defined names. For now, write the address as `#SheetName!A1:C9`, or type the name into the Name Box to go there (on the ledger) |
| Links to e-mail addresses and to files on your machine | Different | There are no dedicated E-mail Address or Existing File fields. Type `mailto:` or a file path straight into the panel — anything that does not begin with `#` is handed to the OS default handler. There is no equivalent of "link to a new document" |
| Not breaking links made in Excel (the xlsx round trip) | Same | External URLs are read and written as External, in-workbook links as location. Links made in Excel survive both opening and saving |

## Data validation

The same three-tab panel as Excel (Settings, Input Message, Error Alert). The house rule is **never block input with a rule we cannot evaluate** — rules we cannot read are kept and let through.

| Excel's name | Mark | How it works here |
|---|---|---|
| Drop-down list | Same | Allow = List, either typed inline or given as a range reference. Pick from the ▾ at the bottom right of the cell |
| Source as a cell range | Same | Edit the referenced cells and the list follows. A reference to another sheet cannot be resolved, and **the cell is let through as unrestricted** |
| Whole number and decimal ranges | Same | All eight comparison operators. Full-width digits are normalized to half-width and accepted |
| Text length | Same | All eight comparison operators |
| Date and time | Not yet | Existing rules are carried through untouched, but nothing is evaluated — input passes straight through (on the ledger) |
| Custom (formula) | Not yet | Same as above (on the ledger) |
| Input message | Different | It appears on the status line at the bottom of the window rather than in a floating balloon. Round-trips through xlsx |
| Error alerts (Stop, Warning, Information) | Different | Stop blocks the entry (Esc backs out); Warning and Information let it in and then say "this doesn't match, but I let it through." No Retry/Cancel dialog — the status line says it |
| Ignore blank | Same | Round-trips as allowBlank |
| Showing or hiding the in-cell ▾ | Same | Round-trips as showDropDown |
| Applying a change to all cells with the same rule | Same | A checkbox in the panel swaps the rule in while leaving its range as it stands |
| Clear All | Different | No dedicated button, but setting Allow to "any value" and confirming takes the rule off |
| Alt+↓ to open the drop-down | Different | Use "Pick From Drop-down List" in the right-click menu, or the ▾. Even a cell with no rule offers the other values in its column as candidates. The key itself is not wired up |
| Circle Invalid Data | Not yet | There is no tool for sweeping up values that stopped matching their rule after the fact (not on the ledger yet — a candidate for it) |
| Round-tripping Excel's rules | Same | Even kinds we do not understand are carried through rather than dropped |

## What-If analysis

There is no What-If Analysis roll-up menu. **Goal Seek** and **Solver** sit directly on the Data tab, with no add-in to register.

| Excel's name | Mark | How it works here |
|---|---|---|
| Goal Seek | Different | Two questions in sequence: "cell = target value", then "cell to change". The search runs on a copy of the sheet, so the real one is never dirtied |
| OK / Cancel on the result | Different | No confirmation window. It writes the answer straight away, reports on the status bar, and a single Ctrl+Z undoes it |
| When no solution is found | Same | It says so plainly, and writes no half-converged value |
| Solver (max / min / value, variables, constraints) | Same | Set it up in a small window and press Solve. Ctrl+Z undoes it |
| GRG Nonlinear, Evolutionary | By design | One solving method: the simplex LP. If the model is not linear it says so and declines (the policy is on the ledger). For nonlinear work, call scipy directly from =PY or from plugins |
| Integer and binary constraints | Not yet | The only operators are <=, = and >=. Use scipy for the time being (not on the ledger yet — a candidate for it) |
| The Solver Results window | Different | It does not appear. To back out, Ctrl+Z |
| Data Table (sensitivity table) | Different | Data > Data Table asks for the column input cell, then the row input cell — press Enter on an empty prompt for a one-variable table (**implemented 2026-08-08**). The layout is Excel's (one variable: input values down the left column, the formula along the top row; two variables: the formula in the corner). Rather than Excel's TABLE() array formula, it fills in the values as of that moment, so when you change an input, press it again |
| Scenario Manager | Not yet | Keep each case's inputs on a separate sheet or in a separate workbook and swap them in by hand (not on the ledger yet — a candidate for it) |
| Forecast Sheet | Not yet | The FORECAST function is there. From =PY, use statsmodels and polars (not on the ledger yet — a candidate for it) |

## Co-authoring

Excel's answer is the cloud. **officework's answer is a shared folder** — whoever can see the folder is your collaborator, and the data never leaves your own network. See "Shared folders and exclusive locks" in the [calc manual](calc-manual.md).

| Excel's name | Mark | How it works here |
|---|---|---|
| Share it by putting it on OneDrive | Different | Put the workbook in a shared folder |
| Real-time co-authoring | By design | We run no server ([design](sekkei/ayumi.ja.md), native-first). What we have instead is an exclusive lock — whoever opens the file first writes, everyone after reads |
| "So-and-so is editing" | Same | A .~lock file, the same as LibreOffice. It names the person, and later arrivals are stopped from overwriting. Once the first person has gone, take the edit right back with Co-authoring mode |
| Cell comments | Same | Round-trip as commentsN.xml, and Excel shows them |
| Replies, threads, Resolved | Not yet | A comment is one string on one cell (an open item on the ledger) |
| @mentions | By design | Notifications presuppose a server. Say what you need to say in the chat (the handover log), signed with your name |
| Chat | Different | Not a live feed — a message left through a file, in the handover log that sits beside the workbook (name.xlsx.chat.txt) |
| Version history | Different | Every overwrite-save keeps a copy in .jo-history, nine generations deep. Restoring is your own save — nothing is written back silently |
| Track Changes (the review record) | Different | Co-authoring > Track Changes turns recording on and off (**implemented 2026-08-08**). Rather than picking up operations one at a time, it records **the difference from the moment recording started**, written down when you stop (the same shape as in writer). Who, when, which cell, from what to what. Press it again and the list appears; pick an entry and it jumps to that place. **It is not an undo feature** — for that, Ctrl+Z or the version history. The record goes into a part of the xlsx that is ours alone, and **Excel does not read it** |
| AutoSave | By design | It presupposes saving to the cloud. Ctrl+S is the basic move; unsaved changes show on the status bar, and you are asked on exit |
| Allow Edit Ranges, Protect Workbook | By design | The buttons are greyed out, holding the place only (see the protection section of the [design](sekkei/calc.ja.md)). Protection is done with sheet protection |
| Sheet View (a filtered view just for you) | Not yet | Since there is no simultaneous editing, the collision it solves rarely comes up in the first place (an open item on the ledger) |
| Handing it over read-only | Different | Make the sheet read-only from the Protect tab (round-trips as sheetProtection). We do not build view-only live sharing |

## Macros and automation (VBA → Python)

- VBA is **deliberately absent** — we are not importing the thirty-year-old
  "open = execute" hole ([design](sekkei/python.ja.md)). Python takes the job:
  [Python manual](python-manual.md)
- **The only thing a workbook may carry is a =PY function.** Procedures run
  only from .py files you placed in `~/.config/office/plugins/` yourself —
  a received file can never become the origin of execution (settled 2026-08-08)

| Excel's name | Mark | How it works here |
|---|---|---|
| Record Macro | Different | There is no record button. Ask the AI in plain language for the Python and try it in the sandbox (the manual carries a copy-paste briefing block). Once it satisfies you, move it into plugins and call it with `@name` from then on |
| VBA and the Visual Basic Editor | By design | See above ([design](sekkei/python.ja.md)) |
| The VBA you already have in .xlsm files | Different | Extract with olevba → hand it to the AI together with the briefing to get Python → check the answers against the same inputs. There is no way to open an .xlsm directly either |
| Macro security settings (the three choices, the yellow bar) | By design | Nothing runs by itself and every run is explicit and sandboxed, so the moment that would ask you to choose never arrives |
| Workbook_Open / Auto_Open | By design | Open = execute does not exist — the first safety principle of this software |
| Worksheet_Change and the other events | By design | Build it out of data validation and conditional formatting plus an explicit `@name` or `@計算` |
| Personal Macro Workbook (PERSONAL.XLSB) | Different | `~/.config/office/plugins/` is that place — `@name` reaches it from any workbook |
| Assigning a macro to a button or a shape | By design | We do not build a way to wire "clicked = execute" into a workbook (the button keeps its place, grayed) |
| Your own worksheet functions (Function) | Different | Put a Python `def` into the workbook with `@save` and call it as `=PY("fname", …)`. A 2-D return value spills down-right |
| Office Scripts + Power Automate | Different | For scheduled runs, drive pysheet (office_sheet) from cron as ordinary Python |
| MsgBox, InputBox, UserForms | Different | No windows pop up. `print` goes to the status bar and input comes from cells plus data validation — the sheet itself is the UI |
| xlwings and COM automation | Same | A one-line swap to officework. `@xw.func` and `Book.caller()` are not supported (plugins fill the same role) |
| Talking to the web or in-house APIs | Different | The sandbox has no network by default. It opens only when you type `@name net` at that moment, and **the permission is never saved anywhere** |

## AI (the Copilot equivalent)

The AI writes code and prose. **Execution is always an explicit human action
inside the sandbox**, and nothing the AI produces lands without a snapshot
first — every one of these steps back with a single Ctrl+Z. The default
destination is a model on this machine, so nothing leaves your network.

| Excel's name | Mark | How it works here |
|---|---|---|
| "Turn this into a table" | Same | AI tab, "To table". If the destination already holds something, it refuses |
| Asking for a formula | Different | Type the request into "Ask". The unit is one formula into one cell (not a calculated column added all at once) |
| Summarizing data | Different | Two to four sentences land as a comment on the cursor's cell (and stay in the xlsx on save) |
| Insights (recommended charts, suggested pivot tables) | Not yet | You build charts and pivots by hand — a path that stands up without AI (not on the ledger yet — a backlog candidate) |
| Driving the app in natural language ("sort by amount") | Not yet | The AI answers in three shapes only — a table, a formula, a comment — and never operates the screen for you (a backlog candidate) |
| Asking for a macro | Different | calc's AI tab has no button for it (writer does). Paste the "briefing for the AI", have it write the script, put it in plugins, and run it yourself |
| AI-assisted VBA migration | Different | No one-press button yet. The pattern — olevba, then the briefing, then checking the answers in the sandbox — is written up in the manual |
| Copilot connectors | By design | We don't bundle AI together with data ingestion (see the section on bringing data in) |
| Licensing and sign-in | Different | Neither exists. The destination cycles local (default) → Claude subscription → Claude API (keys live in environment variables only). If one can't be used it says why and declines |
| Privacy | Different | Something leaves this machine only when you explicitly chose that destination. Keys never enter the workbook |
| Adjusting wording (rewrite, polite form, translate) | Same | Only text cells are replaced; numbers and formulas are left alone |
| Filling in the rest of the data | Same | "Continue". It tells you outright: this is the model guessing, so check it |
| A furigana (reading) column | Different | Select one column and the readings go into the column to its right. Unlike PHONETIC (which replays what was typed), the model supplies the reading |

## Printing and page layout

**Printing means PDF** (screen = paper). File > Print turns the page exactly as
laid out into a PDF; to reach a printer, send that PDF through the OS print
dialog. Whatever settings took effect are reported in the status bar. See
"Print and PDF" in the [calc manual](calc-manual.md).

| Excel's name | Mark | How it works here |
|---|---|---|
| Print (Ctrl+P) | Different | File > Print → a PDF. There is no Ctrl+P key (on the ledger) |
| Set and clear the print area | Same | Round-trips as Print_Area |
| Several print areas | Not yet | Always exactly one. If the xlsx holds more, the first is used and it says plainly "the remaining n areas cannot be printed" (on the ledger) |
| Inserting and removing (horizontal) page breaks | Same | Round-trips as rowBreaks. There is no "reset all" — one break at a time |
| Vertical page breaks (column breaks) | Not yet | Open on the ledger |
| Page Break Preview and the dotted boundary lines | Not yet | The button is gray. Check the result by writing the PDF (on the ledger) |
| Scaling the printout (%) | Different | A cycle of 100 → 90 → 80 → 70 → 50%. You can't type an arbitrary percentage, but the value round-trips through the xlsx |
| Fit to 1 page wide × N tall | Not yet | Drop the scale until it fits (on the ledger) |
| Headers and footers (&P/&N) | Same | Six segments. Date, image, and odd/even variants are held back. &-codes we don't understand are dropped rather than printed as garbage |
| Repeating title rows | Same | Round-trips as Print_Titles. Title columns (repeating on the left) don't exist |
| Printing gridlines and headings | Same | Round-trips as printOptions |
| Paper, orientation, margins | Different | A4/A3/B4/B5/A5 (B sizes are JIS). Margins cycle through three choices; arbitrary values can't be typed |
| Save as PDF | Different | "Print" is exactly that — one step |
| A wide table across several pages | Same | Columns are split into bundles by paper width and printed **down, then across** (Excel's default order). Each bundle repeats the column headings and the title rows (**implemented 2026-08-08**). A single column wider than the paper can't be split, so it is cut — and the status bar says how many were |

## Before you hand it out — proofing, inspection, templates

Three jobs come before handing a workbook to someone else: look for mistakes, flush out what's hidden, and pass the next person a template. Proofing doesn't exist in calc yet (in writer it's the "Spell" button along the bottom of the window), and there is no one-shot document inspector either — the places to look are scattered across the File tab's info panel, the View tab, and the comments on individual cells. Templates are not .xltx here: keep an ordinary xlsx in a shared folder and copy it. One thing comes free, though — **saving never writes your name into the file to begin with**, so what Excel makes you opt in to strip is simply never there. See the [calc manual](calc-manual.md).

| Excel's name | Mark | How it works here |
|---|---|---|
| Spell check (F7, the Review tab) | Not yet | calc has neither a ribbon button nor a key binding. writer can run it, but it lives on the **"Spell" button in the status bar at the bottom of the window** rather than on a Review tab — English goes through a dictionary, Japanese through a local model. Sharing writer's path into calc is held back (on the ledger) |
| "Check my Japanese too" (Excel only checks English words) | Different | Japanese errors aren't misspellings — they are wrong kanji conversions (以外 / 意外), inconsistent orthography, and okurigana, none of which a dictionary catches, so here a local model is asked. Available from writer's "Spell" button and from the headless `office-spell` command (nothing leaves your network). If the model can't be reached it says "cannot proofread" — it never quietly reports "no issues found" |
| AutoCorrect (capitalize first letter, TWo INitial capitals, fix accidental CapsLock) | Not yet | It sits on the ledger as a held-back item, but "never silently change the meaning" is the house rule, so even if it arrives it is unlikely to take the shape of a silent fix. To change case, apply Home's "changecase" explicitly |
| Adding your own AutoCorrect entries ((c) → © and friends) | Not yet | Neither a registration screen nor a replacement mechanism exists. With AutoCorrect as a whole missing from calc, the user-defined layer on top of it is missing too (on the ledger) |
| Math AutoCorrect and AutoFormat As You Type | Not yet | calc has nothing corresponding to Excel's three tabs (on the ledger). Insert symbols explicitly from the Insert tab's "Symbol" |
| Typing a URL turns it into a hyperlink | Different | It doesn't. Links are made explicitly with Ctrl+K or from the Insert tab, and only what you typed into the dialog and confirmed becomes one. With no auto-linking, there is also no hunt for the setting that turns it off (see "Hyperlinks" above) |
| Adding a word to the dictionary (proper nouns and company names flagged as errors) | Different | The user dictionary is a file, pointed at by the `OFFICE_DICT_USER` environment variable (the same convention hunspell uses). A word judged once is remembered and not asked about again. There is no "add to dictionary" button at the point of the flag yet, and no dictionary field in the settings page |
| Proofing language (choosing a dictionary language per document) | Different | You don't pick a language — one entry point routes by content: English through the dictionary, Japanese through the model, and a mixed document through both (English words inside Japanese are ordinary). There is no screen for switching languages. "Language" in Advanced settings is for the ribbon's wording, which is a separate thing |
| Thesaurus | Not yet | No way to invoke one, and it isn't on the ledger either. For rephrasing, the AI tab's "rewrite", "polite", and "plain" are a separate path that replaces text cells only |
| Lowercase function names still work (=sum → =SUM) | Different | A function name typed in lowercase parses and calculates normally. What doesn't happen is a rewrite of what you typed — `=sum(A1:A3)` stays `=sum(A1:A3)` (Excel would upper-case it) |
| Document Inspector (one sweep for hidden data and personal information) | Not yet | There is no sweeping inspection. The places to look are scattered — properties in the File tab's info panel, hidden sheets on the View tab, comments cell by cell. You can visit each part as listed below; what's missing is the button that bundles them (not on the ledger yet — a backlog candidate) |
| Workbook properties (author, title, tags, subject, comments) | Same | Five fields appear in the File tab's "Workbook info" panel; click a field, type, and press Enter to record it. It goes into docProps on save and is visible in Excel |
| Removing the author's name (stripping a personal name before distribution) | Same | Clear the author field in the info panel and press Enter. On save, `dc:creator` is replaced with an empty tag. The other four fields clear the same way |
| The "remove personal information from file properties on save" option | Different | No opt-out is needed — **saving never writes your name in the first place.** A new workbook's properties are empty, and there is no mechanism at all for recording a last-modified-by. Only what you typed into the info panel goes in. What Excel protects by opt-in, this protects by default behavior |
| Checking or deleting properties that don't appear on screen, such as last modified by | Not yet | The info panel holds five fields. The original's other fields (last modified by, creation time, and so on) are carried over as-is on save, with no way to see them and no way to remove them. A last-modified-by that Excel attached travels on attached (viewing is open on the ledger; removal isn't listed — a backlog candidate) |
| Finding and unhiding hidden worksheets (including veryHidden) | Same | The View tab's "Sheet visibility" lists the hidden sheets; pick one to bring it back. veryHidden is read the same as hidden, so it appears in the list too — a veryHidden sheet Excel's own UI can't restore opens here in the same single step |
| Inspecting data lurking in hidden rows and columns | Not yet | There is no tool that sweeps a whole workbook for hidden rows and columns. To bring them back, right-click a heading and choose "Unhide" (select across the hidden span) or use the +/− of an outline group. What is hidden in the xlsx stays hidden when opened — nothing is lost, but there is no way to hunt for it (not on the ledger yet — a backlog candidate) |
| Deleting all comments (notes) at once | Not yet | Deletion is a button on the Collaboration tab, one cell at a time under the cursor. There is no way to strip them all in one go. Show/hide does exist, but even hidden the status bar says "there are comments attached" — handing a workbook out with comments still in it is prevented by saying so out loud (a workbook-wide sweep isn't on the ledger yet — a backlog candidate) |
| Inspecting hidden defined names | Not yet | The Formulas tab's "Name Manager" lists only simple names and tables, which it can navigate to, retype, or delete. A name carrying the hidden attribute counts as "not simple" and is carried over as-is — unbroken, but with no way to see it and no way to remove it |
| Checking headers and footers for personal information | Different | There is no button that enumerates them, but opening the header/footer editor shows all six segments as they are, and you can clear them by hand. The form is: open it and look before you distribute |
| Inspecting leftover external data connections and queries | Different | In a workbook officework created there is nothing to inspect — imports are always "the values at that moment", and query definitions and connection details never stay in the workbook. Procedures live outside it (in plugins) and aren't distributed with it. A connection Excel put there is carried over as-is on save, though — unbroken, but with no way to see or remove it, so strip it on the Excel side first |
| Checking workbook statistics | Same | The File tab's "Workbook info" shows live counts of sheets, used cells, formula cells, and shapes and images (the order differs from Excel, but the place is the same info panel) |
| Saving a workbook as a template (.xltx) | Different | A template here is an ordinary xlsx: put it in a shared folder such as `templates/` and copy it. There is no ceremony of changing formats, and the save filter offers only xlsx and CSV. Saving in XLTX format itself is held back (on the ledger) |
| Creating a new workbook from a template (personal templates) | Not yet | The File page has a grayed "Create from template" button holding the place. Pressing it does nothing. Today's path is to copy a template from a shared folder yourself and open it (not on the ledger yet — a backlog candidate) |
| Building formatting, formulas, and validation into a template and distributing it | Same | Because a template is a real workbook, borders, fills, column widths, formulas, data validation, print title rows, and bundled =PY functions all travel intact. The samples are the three ledgers in `templates/` (in the inquiry ledger, the status column is a three-choice validation list and H2 is a =PY that tallies the statuses) |
| The online template gallery | By design | We keep no stash of templates on someone else's server — no account, no server, and everything works with no network at all ([design](sekkei/ayumi.ja.md)). The gallery's role is filled by the bundled `templates/` (real workbooks, on your machine or in a shared folder) |
| Changing the defaults for new workbooks via a default template (XLSTART) | Not yet | New workbooks are always plain, and there is no way to swap in different default formatting or layout. `~/.config/office` holds only settings, keys, recent files, and plugins. If you want different defaults, start from a copy of a template (not on the ledger yet — a backlog candidate) |
| Saving as CSV (comma separated) | Same | "Export to CSV" on the File page, or pick CSV in "Save As". UTF-8 (BOM) + CRLF, the shape Excel opens without garbling, values from the current sheet only. What can't go in (formulas, formatting, other sheets) is named in the status bar, and the file you're working on stays the xlsx — the same line Excel's own CSV save draws |
| Choosing the CSV encoding and delimiter / tab-separated (.txt) | Not yet | Export is a single path: UTF-8 (BOM) + comma + CRLF, with nothing to choose. Older systems that want Shift-JIS or tabs can't be served yet. The import side does have an encoding-and-delimiter wizard, so the open item is the export side only (on the ledger) |
| Saving and opening ODS / OTS (OpenDocument) | Not yet | Reading and writing are both xlsx-first, and exchanges with LibreOffice users go through xlsx as well. Writing a second format family from scratch is a big piece, so it waits — xlsx, PDF, and CSV carry the real work, is the reasoning recorded on the ledger |
| Opening a template file someone sent you (.xltx / .xltm) | Not yet | The open filter is xlsx only; .xltx can't be selected, and there is neither an implementation nor a test for handling one. Re-save it as xlsx in Excel or LibreOffice first — the same escape route as for old .xls (not on the ledger yet — a backlog candidate) |

## Protection and security

| Excel's name | Mark | How it works here |
|---|---|---|
| Protect Sheet | Different | It makes the sheet read-only (the same button releases it; the tab gets a 🔒). **No password is set — and we don't pretend to set one.** sheetProtection round-trips, so Excel sees the sheet as protected too |
| Allowing edits to only some cells (unlocking cells) | Not yet | Protection covers a whole sheet at once, nothing finer (on the ledger) |
| The checkboxes for permitted operations | Not yet | Open on the ledger |
| Allow Users to Edit Ranges | By design | We run on sheet protection plus an exclusive lock rather than fine-grained permissions ([design](sekkei/calc.ja.md)). The button is gray, holding the place |
| Protect Workbook (structure) | By design | Same as above |
| Encrypting a workbook with a password | Same | AES-256 (Agile). **Cross-verified against real Excel.** Clear the field and press Enter to remove it |
| Opening a password-protected file | Same | Both Standard (2007) and Agile (2013+) are readable. Forget the password and it stays closed — there is no back door |
| Changing or removing the password | Same | From the same button. It takes effect from the next save |
| Digital signatures | Same-shaped in intent, Different | Not a signature line but a signature file placed beside the workbook (`name.xlsx.sig`, Ed25519). What it actually provides is tamper detection and proof of the name |
| Enabling/disabling macros, trusted locations | By design | With no open = execute, the moment that would demand you enable something never comes |
| When you receive an .xlsm | Different | The VBA does not run. olevba → AI → check the answers in the sandbox (see the macros section) |
| Worrying about macro viruses | Different | No automatic execution, plus the bubblewrap sandbox (no network, the real filesystem read-only). The only thing a workbook can carry is a =PY function |
| Recommend read-only, Mark as Final | Not yet | The nearest paths are sheet protection and the exclusive lock (not on the ledger yet — a backlog candidate) |
| The sheet-protection password is unknown | Different | We never look at passwords, so protection applied in Excel comes off with the same one-button press. Sheet protection was always about preventing slips — **if you need a safe, encrypt** |
| Protected View (the warning for files from the internet) | By design | There is nothing that opens and executes to begin with ([design](sekkei/python.ja.md)). Parts we can't read appear in the report shown when the file opens |

## Settings, customization, add-ins

Excel's Options dialog corresponds to the File tab's **Advanced settings** page,
backed by `~/.config/office/settings.toml` (the path is shown on the page). The
entries are trimmed to the ones that mean something in a native app. There is
no add-in registry — extension is entirely the .py files you put in
`~/.config/office/plugins/`. See "Collaboration tab and plugins" and
"@-commands" in the [calc manual](calc-manual.md).

| Excel's name | Mark | How it works here |
|---|---|---|
| The Options dialog (File > Options) | Different | The File tab's "Advanced settings" page is the container, saved into settings.toml. The entries narrow to language, light/dark, interface text size, iterative calculation, reference style (R1C1) and the like, and environment variables (OFFICE_FONT, OFFICE_LANG, …) override them for the session. Most of Excel's thirty-plus categories simply aren't here |
| Customizing the ribbon (custom tabs, reordering buttons, exporting the setup) | By design | Tab names and order are fixed to match Excel — letting someone who switched keep the muscle memory outranks everything else, so we don't build a way for the layout to differ machine to machine. Frequently used operations are carried by the right-click menu and the right-hand panel |
| Customizing the Quick Access Toolbar | Not yet | Row 1 is a fixed four buttons — save, print, undo, redo — with no way to add or remove (on the ledger) |
| Collapsing / minimizing the ribbon (Ctrl+F1) | Not yet | calc's View tab has no button to fold the band away. writer has an "always show toolbars" toggle, so copying it into calc is a backlog candidate (not on the ledger yet — a backlog candidate) |
| The default font and size for new workbooks | Not yet | There is no setting that decides a new workbook's default typeface. Advanced settings' "font (OFFICE_FONT)" is for the interface, which is a different thing from the cell default. For now, select a range and change it from Home's font and size controls (folding this into settings is planned; not on the ledger yet — a backlog candidate) |
| Where the cursor moves after Enter | Not yet | Enter is always down, Tab right, Shift+Tab left, fixed. To move right, use Tab (not on the ledger yet — a backlog candidate) |
| AutoSave and the AutoRecover interval | By design | We don't build autosave at all (it's a feature that assumes cloud storage). Saving is Ctrl+S; unsaved changes show in the status bar and are asked about on quit. Nine generations of copies kept in `.jo-history` on every overwrite are the real safety net |
| Changing the default save format (.xls, ODS) | Not yet | "Save As" offers xlsx and CSV (current sheet's values only), and there is no setting to change the default (printing = PDF is a separate door). ODS, OTS, and XLTX are held back on the ledger. Old .xls is neither read nor written |
| Changing the user name (author name) | Not yet | The identity is taken automatically from the environment as USER@hostname and appears in locks, chat, and signatures. Advanced settings displays it but has no way to edit it yet (making it a real personal name is a request planned for the settings page) |
| Changing the display language | Same | Pick it under "Language (ribbon and wording)" in Advanced settings; it takes effect at the next launch (Excel needs a restart too). Only languages whose wording is complete appear — the honest line that keeps us from claiming "45 languages". OFFICE_LANG wins if it's set |
| Office theme (white, dark gray, black) | Different | Light and dark, two choices, switchable from either the View tab or Advanced settings and persisted. What darkens is the frame (band, tabs, headings) only — cells stay white, so screen and paper agree. Excel's "dark document" mode doesn't exist |
| Customizing the status bar | Not yet | The bottom edge is a fixed construction (sheet tabs, status wording, and the selection's sum/average/count), with no right-click menu for choosing items. Narrowing it to three statistics was a design decision, and all three respect filtering and are always present (on the ledger) |
| Adding and enabling add-ins (Options > Add-ins) | Different | There is no add-in dialog and no registry. Putting a .py in `~/.config/office/plugins/` **is** the installation; it is listed under the Plugins tab's "Manage plugins" and runs inside the sandbox (a network-cut scratch space) when selected. The same file is reachable as `@name` from Data > Python |
| Office Add-ins from the Store (web add-ins) | By design | We have neither a store nor an execution surface for web add-ins (a webview) — the native-first choice not to drag in browser-derived layers. Extension is always a local plugins .py, and we build no door through which a workbook or something that arrived from outside becomes the origin of execution |
| Managing COM add-ins | By design | COM is a Windows-Excel-specific door and doesn't exist here. xlwings assets are accepted through a one-line swap over a socket, and Excel's add-in machinery (`@xw.func`, `@xw.sub`, `Book.caller()`) is honestly reported as "not supported" — plugins .py fills the same role |
| Loading the Solver add-in | Different | There is no add-in-registration ritual to get through first — Solver and Goal Seek sit directly on the Data tab from the start. The one method is simplex LP; nonlinear problems are refused honestly |
| The Analysis ToolPak | Not yet | Nothing corresponds to it yet, and even the comparison against Excel's help is still outstanding. For now, reach scipy and statsmodels directly from =PY or a plugins procedure (everything installed on the machine is available) (on the ledger) |
| Writing and distributing your own add-in (.xlam / .xla) | Different | The home for "a feature I want from every workbook" is plugins — put a .py there for procedures and call `@name`; for cell functions, load it into the workbook with `@save fname` and call it from =PY. Distribution is handing over the .py; the recipient reads it, then places it in their own plugins. A procedure embedded in an old workbook can only be pulled out with `@export` — never executed |
| Removing, disabling, or not finding an add-in | Different | The list simply reads the files in the folder, so deleting (or moving away) a .py is the whole of removal and disabling. "It isn't showing up" only ever means "it isn't in the folder", and when the folder is empty the status bar tells you its path. With nothing resident and nothing auto-loaded, the accident where an add-in breaks startup can't occur |
| Auto-loading at startup or on open (XLSTART) | By design | "Open = execute" and "start = execute" do not exist — the first safety principle of this software. Plugins procedures and =PY alike run only when a person explicitly acts, at that moment |
| Add-in security (the warning bar, trusted publishers) | By design | The situation that would demand you enable something can't arise by construction — nothing runs automatically, every run is explicit, and every run is inside the sandbox (no network, the real filesystem read-only, home invisible, a time limit). Instead of building a registry of trust, we built a shape that doesn't ask for trust |
| A workbook using add-in functions shows #NAME? | Same | An unknown function honestly comes out as #NAME? rather than quietly calculating as 0 — the same way it breaks in Excel. The other direction (a =PY workbook opened in Excel) is also #NAME?, but the last computed values remain — the degradation is on the safe side |

## Accessibility

Plainly: **screen reader support is not something we can claim.** The GPUI
foundation has the plumbing, but we have not wired up a single role or label,
and neither the grid nor the ribbon is visible to assistive technology. What
works is enlarging the interface text and keyboard operation inside the grid.
The ribbon and the dialogs cannot be reached from the keyboard alone.

| Excel's name | Mark | How it works here |
|---|---|---|
| Making the ribbon and menu text larger too (in Excel you fall back on OS settings or the magnifier) | Different | The app has its own. Ctrl+= / Ctrl+- or the View tab's "larger/smaller interface text" gives 80–150%. It covers everything — ribbon, formula bar, menus, headings, status bar — and is remembered in settings, so the next launch opens at the same size (a separate axis from grid zoom) |
| Moving, selecting, and editing cells from the keyboard alone | Same | Arrows / Enter (down) / Tab (right) / Shift+Tab (left) to move, Shift+arrow to extend a selection, F2 to edit; Ctrl+Home/End, PageUp/Down, and Ctrl+A all work. The same fingers as Excel |
| Shift+F10 or the Application key for the context menu | Same | The Menu key and Shift+F10 open the same menu as a right-click. The same binding as Excel |
| Driving the ribbon with Alt key tips (Alt → H → …) | Not yet | Absent. The ribbon is reachable only with the mouse (or via the context menu through the Menu key), and the only Alt binding wired up is Alt+Enter (line break inside a cell) (on the ledger) |
| Tabbing through a dialog to reach its buttons and fields | Not yet | There is no mechanism for moving focus with Tab; fields and buttons in a dialog are chosen by clicking. Only Enter = confirm and Esc = close work on nearly every dialog (on the quit confirmation, Enter = save and quit / Esc = cancel) (not on the ledger yet — a backlog candidate) |
| Screen readers (Narrator, NVDA, JAWS) | Not yet | **This is not something we can claim support for.** The GPUI foundation does carry AccessKit plumbing (macOS/Windows/Linux AT-SPI adapters), but officework has not wired up a single role or label, and the grid, the ribbon, and the status bar are invisible to assistive technology (on the ledger) |
| Speak Cells | Not yet | Absent. There is no speech synthesis or read-aloud implementation anywhere (not on the ledger yet — a backlog candidate) |
| Accessibility Checker (the pre-distribution check) | Not yet | Absent. The checking feature does not exist at all. The whole pre-distribution inspection family (document inspection) is still parked under "sections still to fill" below |
| Alt text for images and shapes | Not yet | Absent. The shape and image settings panel has no alt-text field, and only a name is written when going out to xlsx (what we read from the original is carried over unbroken, nothing more) (on the ledger) |
| Dark mode and high contrast | Different | The View tab's "interface theme" switches light and dark and is remembered in settings, so the next launch opens the same. Cells (the paper surface) stay white, though — the look of a form is governed by paper. There is no following of the OS high-contrast setting |
| Jumping straight to a specific cell (F5 / Ctrl+G, Go To) | Different | Type an address (B12), a range (A1:C9), or a defined name into the Name Box (left end of the formula bar) and go there. But F5/Ctrl+G don't exist and reaching the Name Box means clicking, so **it is not reachable from the keyboard alone** |
| Dictation (voice input) | Not yet | Absent. There is no speech recognition and no input path other than the IME. If the OS's own voice input (as an IME) gets through, that becomes the path, but there is no record of anyone confirming it (not on the ledger yet — a backlog candidate) |

## Keyboard shortcut table

We don't hide the keys that don't work. In many cases the feature itself is
there and only the key is unwired (open on the ledger under "shortcuts and
formats").

| Key | What it does in Excel | Works? | Instead |
|---|---|---|---|
| Ctrl+C / X / V | Copy, cut, paste | Yes | — |
| Ctrl+Shift+V | Paste values only | Yes | (the same binding as recent Excel) |
| Ctrl+Z / Ctrl+Y | Undo, redo | Yes | Ctrl+Shift+Z works too |
| Ctrl+S / Ctrl+O | Save, open | Yes | — |
| F2 | Edit the cell | Yes | — |
| Alt+Enter | Line break inside a cell | Yes | — |
| F9 / Shift+F9 | Recalculate (everything / this sheet) | Yes | — |
| Ctrl+K | Hyperlink | Yes | `#SheetName!B5` jumps inside the workbook too |
| Ctrl+Home / Ctrl+End | To the top / to the end | Yes | — |
| Ctrl+wheel | Zoom the view | Yes | — |
| Ctrl+F / Ctrl+H | Find, replace | No (unwired) | The 🔍 at the right edge of the sheet-tab strip (two panels; press Enter on an empty replacement to just search) |
| Ctrl+B / I / U / 5 | Bold, italic, underline, strikethrough | No (unwired) | The Home tab buttons |
| Ctrl+1, Ctrl+Shift+% / $ | Format Cells dialog, number formats | No (unwired) | The number format list, or the right-click menu |
| Ctrl+arrow / Ctrl+Shift+arrow | To the edge of the data / select to the edge | Yes | If the neighbor holds something, the end of that block; if it's empty, the next thing that isn't (**implemented 2026-08-08**). In a direction with nothing in it, it **stops at the edge of the used range** (Excel flies to the far end of the sheet) |
| Ctrl+PageUp / PageDown | Switch sheets | No (unwired) | Click a sheet tab below |
| Ctrl+Shift+L | Toggle filters | No (unwired) | The Data tab's "Filter" |
| Ctrl+D / Ctrl+R | Fill down / fill right | No | The "Fill" button (down only; there is no right) |
| Ctrl+; / Ctrl+Shift+; | Today's date / the current time | No | =TODAY() / =NOW() (which change every time the file is opened) |
| Ctrl+E | Flash Fill | No | The AI's "Continue", or Python |
| Ctrl+T | Create a table | No | The Insert tab's "Insert table" |
| Ctrl+P | Print | No | File > Print (= PDF) |
| Shift+F3 | Insert Function | No | The button on the Formulas tab |

## When things go wrong (it won't open, it comes out garbled)

Three promises — **parts we cannot read are named in the report shown when the
file opens** (never silently dropped), **parts we don't understand are carried
over as-is** (never quietly deleted), and **anything that will be lost on save
is announced before it is lost**.

| Excel's name | Mark | How it works here |
|---|---|---|
| Open and Repair | Not yet | If it can't be read, it stops at "cannot open". The escape route is the nine generations kept in `.jo-history` — go back to a version from before the damage (not on the ledger yet — a backlog candidate) |
| AutoRecover / Document Recovery | Not yet | Saving is manual, Ctrl+S only. Unsaved changes show in the status bar and are asked about on quit (autosave is by design absent — see the collaboration section) |
| A garbled CSV | Same | Pick a different encoding in the import wizard. It reports which encoding it used |
| "Another user has it locked" | Different | It reads `.~lock` and names the person for you. The second arrival can read; overwrite-saving is blocked |
| "We repaired it" and a part went missing | Different | The report lists them as "name × count", and the original is carried over rather than trimmed. Anything that really will be lost is written into the report as such |
| Password-protected workbooks | Same | Both Standard and Agile are read. Forget the password and it stays closed (no back door) |
| A forgotten sheet-protection password | By design | We never set one in the first place (it's a lock in appearance only). The same button takes it off — the accident where you're stuck can't happen |
| Macro warnings and Protected View | By design | There is no open = execute (see the protection section) |
| An old .xls won't open | Not yet | We read only xlsx (zip). Re-save it as xlsx in Excel or LibreOffice (recorded in the ledger's policy section as the accepted trade-off on other formats) |
| Being asked to "update links" | By design | With no external references by construction, the question never comes up. To refresh, pull it in again with "external link" |
| Dates from a Mac are off by four years (the 1904 system) | Different | It detects this and reports "these will look four years off". The save carries date1904 over as-is rather than breaking it |
| Going back to a previous version | Different | Not the cloud but the local `.jo-history` (nine generations). It opens as an unnamed copy; save it yourself to restore |
| How it will look when reopened in Excel | Same | The same xlsx. Degradation is on the safe side and is spelled out in each section — =PY becomes #NAME? but the values remain, charts become images, pivots become value tables once Excel saves |
| Compatibility Checker | Different | Not a dialog: the report shown on open and the status bar carry that job. The rule is no degradation, with anything that will be lost written down as the exception |

## What's left after the full pass

The 18 uncovered areas listed in the second draft were all filled in the third —
**that completes one pass over Excel's help territory.** From here, what gets
added to this guide is keeping up with newly built features and comparing
against whatever Excel adds next.

What remains is not missing sections but **outstanding content**, and it lives
in three places:

- **The implementation backlog** = [the ledger](guide-tsukiawase-2.ja.md).
  What this draft's comparison turned up (Consolidate, the data model, the
  Analysis ToolPak function families, FREQUENCY and LINEST, custom lists, form
  controls, sheet groups, document inspection, creating a workbook from a
  template, screen reader support, and more) is marked "a backlog candidate"
  inline in the text, so moving those onto the ledger is the next tidying pass
- **Thin sections** — "Entering data and formatting" is still mostly number
  formats and merging. The finer points of cell formatting were split out into
  "Cell formatting basics", so the two should eventually be merged

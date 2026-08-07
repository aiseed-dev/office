//! main.rs からの純移動(2026-08-06 の分割)。挙動は変えない。

use crate::*;

impl Calc {
    /// run_cmd が処理できる id。**リボンの ready はこの表の中に限る**
    /// (試験で突き合わせる。合っていない釦は「押せるのに何もしない」嘘になる)
    #[allow(dead_code)] // wiring_tests(cfg(test))が使う
    pub(crate) const HANDLED: &'static [&'static str] = &[
        "open", "save", "undo", "redo", "selectall", "pdf",
        "copy", "cut", "paste",
        "bold", "italic", "underline", "borders", "fillparag", "fontcolor",
        "align-left", "align-center", "align-right",
        "comma", "currency", "percents", "digit-inc", "digit-dec", "clear",
        "strikeout", "top", "middle", "bottom", "wrap", "incfont", "decfont",
        "cell-ins", "cell-del", "insrow", "inscol",
        "merge", "custom-sort", "sort-asc", "sort-desc",
        "rem-duplicates", "setfilter", "clear-filter",
        "fill-num", "freeze", "show-formulas", "show-gridlines",
        "fn-math", "fn-text", "fn-logical", "fn-recent",
        "sum", "average", "count", "max", "min",
        "data-validation", "condformat", "defname",
        "pageorient", "pagesize", "pagemargins", "printarea",
        "inschart", "insimage", "inshyperlink", "replace",
        "changecase", "format", "cell-format", "fontname", "fontsize",
        "fn-datetime", "fn-lookup", "fn-financial", "fn-more",
        "scale", "pagebreak", "printtitles", "print-gridlines", "print-headings",
        "data-from-text", "text-column", "goal-seek", "data-external-links",
        "insshape", "instext", "inssparkline", "python", "addcomment",
        "trace-prec", "trace-dep", "remove-arrows", "insrecommend",
        "instable", "table-tpl", "inssymbol", "pivot-insert", "pivot-fields",
        "pivot-refresh", "pivot-refresh-all", "pivot-select",
        "pivot-totals", "pivot-subtotals", "pivot-blank", "pivot-layout", "pivot-style",
        "td-header", "td-total", "td-band-row", "td-band-col",
        "td-first", "td-last", "td-filter",
        "group", "ungroup", "hide-details", "show-details", "subtotal", "solver",
        "inssmartart", "insequation", "insslicer", "inscheckbox", "instextart",
        "coauth-mode", "co-delcomment", "co-showcomment", "co-chat",
        "co-history", "plug-macros", "plug-manage",
        "prot-doc", "prot-encrypt", "prot-sign",
        "zoom-in", "zoom-out", "ui-bigger", "ui-smaller", "formula-bar", "show-headings", "show-zeros",
        "subscript", "align-just", "text-orient", "calc-mode",
        "td-torange", "td-resize", "rtl-sheet", "direction",
        "colorschemas", "theme",
        "ai-where", "ai-summary", "ai-rewrite", "ai-polite", "ai-plain",
        "ai-translate", "ai-furigana", "ai-continue", "ai-table", "ai-ask",
        "insert-function", "cell-styles", "sheet-view", "watch",
        "pen", "highlighter", "eraser", "draw-select",
    ];

    /// シートの保護中でも通す操作(見るだけ・保存・保護の操作そのもの)
    pub(crate) const PROTECTED_OK: &'static [&'static str] = &[
        "open", "save", "pdf", "selectall", "undo", "redo",
        "freeze", "show-formulas", "show-gridlines",
        "setfilter", "clear-filter",
        "trace-prec", "trace-dep", "remove-arrows", "pivot-select",
        "coauth-mode", "co-showcomment", "co-chat", "co-history", "plug-manage",
        "prot-doc", "prot-encrypt", "prot-sign", "ai-where",
    ];

    /// ピボットの上では締める操作(本家 Toolbar.js の editPivot ロックと同じ顔ぶれ:
    /// オートフィルタ・結合・ハイパーリンク・テーブル・入力規則・重複削除)。
    /// ピボットは polars が置いた「その時の値」— この上で表を組み替えると壊れる
    pub(crate) const PIVOT_LOCKED: &'static [&'static str] = &[
        "setfilter", "merge", "inshyperlink", "instable", "data-validation", "rem-duplicates",
    ];

    pub(crate) fn run_cmd(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.sheet().protected && !Self::PROTECTED_OK.contains(&id) {
            self.status =
                ui::t!("シートが保護されています(保護タブの「シートを保護する」で解除)").into();
            cx.notify();
            return;
        }
        if Self::PIVOT_LOCKED.contains(&id) && self.pivot_at(self.cursor).is_some() {
            self.status =
                ui::t!("ピボットの上ではできません(カーソルをピボットの外に置いてから)").into();
            cx.notify();
            return;
        }
        match id {
            "open" => self.open_dialog(cx),
            "save" => self.save(false, cx),
            "undo" => {
                if !self.input.undo() {
                    self.undo_sheet();
                }
            }
            "redo" => {
                if !self.input.redo() {
                    self.redo_sheet();
                }
            }
            "selectall" => self.select_all_now(),
            "copy" => self.copy_now(cx),
            "cut" => self.cut_now(cx),
            "paste" => self.paste_now(cx),
            // 罫線 — **日本の帳票の本体**。一覧から辺とペン(線種・色)を選ぶ
            "borders" => {
                self.commit();
                let at = self
                    .cell_origin_px(self.cursor)
                    .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                    .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                let items: Vec<String> = [
                    "下罫線", "上罫線", "左罫線", "右罫線",
                    "外枠", "すべての罫線(格子)", "罫線を消す",
                    "→ 線のスタイル…", "→ 線の色…",
                ]
                .iter()
                .map(|s| s.to_string())
                .collect();
                let style_name = BORDER_STYLES
                    .iter()
                    .find(|(_, b)| *b == self.pen_style)
                    .map(|(n, _)| *n)
                    .unwrap_or("細い実線(既定)");
                let color_name = match self.pen_color {
                    Some(v) => format!("#{v:06X}"),
                    None => ui::t!("自動(黒)").to_string(),
                };
                self.pick_note = Some(
                    ui::tf!("罫線 — いまのペン: {}・{}", style_name, color_name).into(),
                );
                self.pick_kind = "border-pick";
                self.pick = Some((items, at));
            }
            "bold" => self.fmt(|f| f.bold = !f.bold),
            "italic" => self.fmt(|f| f.italic = !f.italic),
            "underline" => self.fmt(|f| f.underline = !f.underline),
            "strikeout" => self.fmt(|f| f.strike = !f.strike),
            // 縦の揃えと折り返し
            "top" => self.fmt(|f| f.valign = sheet::model::VAlign::Top),
            "middle" => self.fmt(|f| f.valign = sheet::model::VAlign::Middle),
            "bottom" => self.fmt(|f| f.valign = sheet::model::VAlign::Bottom),
            "wrap" => self.fmt(|f| f.wrap = !f.wrap),
            // 文字の大きさ(4〜72pt)
            "incfont" => self.fmt(|f| {
                let pt = f.size_c.map(|c| c as f32 / 100.0).unwrap_or(11.0);
                f.size_c = Some((((pt + 1.0).min(72.0)) * 100.0) as u32);
            }),
            "decfont" => self.fmt(|f| {
                let pt = f.size_c.map(|c| c as f32 / 100.0).unwrap_or(11.0);
                f.size_c = Some((((pt - 1.0).max(4.0)) * 100.0) as u32);
            }),
            "align-left" => self.fmt(|f| f.align = HAlign::Left),
            "align-center" => self.fmt(|f| f.align = HAlign::Center),
            "align-right" => self.fmt(|f| f.align = HAlign::Right),
            // 表示形式
            "comma" => self.fmt(|f| f.number_format = Some("#,##0".into())),
            // 行・列の出し入れ
            "cell-ins" => self.rowcol(|s, p| s.insert_row(p.row)),
            "cell-del" => self.rowcol(|s, p| s.remove_row(p.row)),
            "insrow" => self.rowcol(|s, p| s.insert_row(p.row)),
            "inscol" => self.rowcol(|s, p| s.insert_col(p.col)),
            // 小数点以下の桁
            "digit-inc" => self.decimals(1),
            "digit-dec" => self.decimals(-1),
            // 書式のクリア。値は消さない
            "clear" => self.fmt(|f| *f = CellFormat::default()),
            // フィル(下方向へコピー)。式は相対参照がずれ、$ は止まる。
            // 書式も一緒に写す(帳票の列は書式ごと揃える)
            "fill-num" => {
                let (a, b) = self.sel_rect();
                if a.row == b.row {
                    self.status = ui::t!("Shift+↓ で埋める範囲を選んでください(先頭行を下へ写します)").into();
                } else {
                    self.commit();
                    self.checkpoint();
                    let sh = &mut self.book.sheets[self.active];
                    let mut n = 0usize;
                    for c in a.col..=b.col {
                        let Some(src) = sh.get(Pos::new(a.row, c)).cloned() else { continue };
                        for r in a.row + 1..=b.row {
                            let mut cell = src.clone();
                            if let Some(f) = &src.formula {
                                cell.formula =
                                    Some(sheet::model::offset_refs(f, (r - a.row) as i64, 0));
                            }
                            sh.set(Pos::new(r, c), cell);
                            n += 1;
                        }
                    }
                    recalc_book(&mut self.book, self.active);
                    self.dirty = true;
                    self.status = format!("{n} セルを埋めました").into();
                }
            }
            // 塗りつぶし。黄 → 水色 → 解除(色を選ぶ小窓がまだ無い)
            "merge" => self.merge_selection(),
            // 表示。**値は変えない** — 見え方だけの話
            "show-formulas" => self.show_formulas = !self.show_formulas,
            // 帳票を PDF に。画面に見えているもの(値・書式・罫線)を写す
            "pdf" => self.save_pdf(cx),
            "show-gridlines" => self.gridlines = !self.gridlines,
            // ウィンドウ枠の固定。カーソルの上と左を留める。もう一度で解く
            // オートフィルタ。範囲に▼を張る/外す(トグル)。**中身は変えない**
            "setfilter" => {
                if self.auto_filter.is_some() {
                    self.auto_filter = None;
                    self.filter_panel = None;
                    self.status = ui::t!("絞り込みの範囲を外しました").into();
                } else {
                    let (a, b) = if self.anchor.is_some() {
                        self.sel_rect()
                    } else {
                        let (rows, cols) = self.sheet().extent();
                        if rows < 2 || cols == 0 {
                            self.status = ui::t!("絞り込む表がありません(見出しの下にデータが要ります)").into();
                            return;
                        }
                        (Pos::new(0, 0), Pos::new(rows - 1, cols - 1))
                    };
                    if a.row == b.row {
                        self.status = ui::t!("絞り込む表がありません(見出しの下にデータが要ります)").into();
                        return;
                    }
                    self.auto_filter = Some(AutoFilter { range: (a, b), hide: Default::default() });
                    self.status = ui::tf!(
                        "{}:{} に絞り込みの範囲を張りました — 見出しの ▼ から絞ります(表示だけ。保存はされず、閉じれば消えます)",
                        a.a1(), b.a1()
                    )
                    .into();
                }
            }
            "clear-filter" => {
                self.auto_filter = None;
                self.filter_panel = None;
                self.status = ui::t!("絞り込みを解きました").into();
            }
            // 印刷の設定。モデルに置き、保存で原文へ織り込み、PDF が従う。
            // どれもシートの控えで1手戻せる
            "pageorient" => {
                self.commit();
                self.checkpoint();
                let sh = self.sheet_mut();
                sh.landscape = !sh.landscape;
                let landscape = sh.landscape;
                self.dirty = true;
                self.status = format!(
                    "印刷の向き: {}(PDF と保存に効きます)",
                    if landscape { "横" } else { "縦" }
                )
                .into();
            }
            "pagesize" => {
                self.commit();
                self.checkpoint();
                // A4 → A3 → B4 → B5 → A5 → A4 の順で回す
                const CYCLE: [u32; 5] = [9, 8, 12, 13, 11];
                let sh = self.sheet_mut();
                let now = sh.paper_size.unwrap_or(9);
                let i = CYCLE.iter().position(|c| *c == now).unwrap_or(0);
                let next = CYCLE[(i + 1) % CYCLE.len()];
                sh.paper_size = Some(next);
                self.dirty = true;
                let name = paper_mm(next).map(|(_, _, n)| n).unwrap_or("A4");
                self.status = format!("用紙: {name}(B は JIS)").into();
            }
            "pagemargins" => {
                self.commit();
                self.checkpoint();
                // 既定(20mm)→ 狭い(10mm)→ 広い(30mm)→ 既定
                let sh = self.sheet_mut();
                let (next, label) = match sh.margins_mm {
                    None => (Some((10.0, 10.0, 10.0, 10.0)), "狭い(10mm)"),
                    Some((l, _, _, _)) if l < 15.0 => {
                        (Some((30.0, 30.0, 30.0, 30.0)), "広い(30mm)")
                    }
                    Some(_) => (None, "既定(20mm)"),
                };
                sh.margins_mm = next;
                self.dirty = true;
                self.status = format!("印刷の余白: {label}").into();
            }
            "printarea" => {
                self.commit();
                if self.anchor.is_some() {
                    self.checkpoint();
                    let range = self.sel_rect();
                    self.sheet_mut().print_areas = vec![range];
                    self.dirty = true;
                    self.status = format!(
                        "印刷範囲: {}:{}(もう一度押すと解除)",
                        range.0.a1(),
                        range.1.a1()
                    )
                    .into();
                } else if !self.sheet().print_areas.is_empty() {
                    self.checkpoint();
                    self.sheet_mut().print_areas.clear();
                    self.dirty = true;
                    self.status = ui::t!("印刷範囲を解きました(全域を印刷します)").into();
                } else {
                    self.status =
                        ui::t!("印刷範囲にする範囲を Shift+矢印かドラッグで選んでください").into();
                }
            }
            // 大文字小文字。選択の英字に小文字があれば大文字へ、無ければ小文字へ
            // 大文字小文字の変更。本家は5択のサブメニュー —
            // 大小のトグルだけの仮実装をやめ、一覧から選ぶ
            "changecase" => {
                self.commit();
                let at = self
                    .cell_origin_px(self.cursor)
                    .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                    .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                self.pick_kind = "changecase";
                self.pick = Some((
                    CASE_MODES.iter().map(|v| v.to_string()).collect(),
                    at,
                ));
            }
            // 数値の書式・セルのスタイル: 書式の小窓(道具箱)を開く
            // 数値の書式。本家のドロップダウン相当の一覧(その他=コード直打ち)
            "format" => {
                self.commit();
                let at = self
                    .cell_origin_px(self.cursor)
                    .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                    .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                // 今の書式に ✓ を付け、状態行にも言う(本家はコンボが
                // 選択セルの書式に追従する — その代わり)
                let cur = self
                    .sheet()
                    .get(self.cursor)
                    .and_then(|c| c.fmt.number_format.clone());
                let cur_name = NUMFMTS
                    .iter()
                    .find(|(_, code)| code.map(|s| s.to_string()) == cur.as_ref().map(|s| s.to_string()))
                    .map(|(n, _)| *n);
                let mut items: Vec<String> = NUMFMTS
                    .iter()
                    .map(|(n, _)| {
                        if Some(*n) == cur_name {
                            format!("✓ {n}")
                        } else {
                            n.to_string()
                        }
                    })
                    .collect();
                items.push("その他(書式コードを打つ)…".into());
                self.status = match (cur_name, &cur) {
                    (Some(n), _) => ui::tf!("今の書式: {}", n).into(),
                    (None, Some(code)) => ui::tf!("今の書式: カスタム(コード: {})", code).into(),
                    (None, None) => ui::t!("今の書式: 一般").into(),
                };
                self.pick_kind = "numfmt-pick";
                self.pick = Some((items, at));
            }
            "cell-format" => {
                let at = self
                    .cell_origin_px(self.cursor)
                    .map(|(x, y)| (x + 16.0, y + 16.0))
                    .unwrap_or((HEAD_W + 24.0, ROW_H + 24.0));
                self.fmt_panel = Some(at);
            }
            // 書体と大きさ: 一覧から選ぶ(日本語が組める書体だけ出す)
            "fontname" => {
                let vals: Vec<String> = kumihan::font::list()
                    .iter()
                    .filter(|f| f.japanese)
                    .map(|f| f.name.clone())
                    .collect();
                if vals.is_empty() {
                    self.status = ui::t!("日本語の書体が見つかりません").into();
                } else {
                    let at = self
                        .cell_origin_px(self.cursor)
                        .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                        .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                    // 全部出す(前は16個で黙って切り捨てていた — 一覧は
                    // スクロールできるので削る理由が無い)
                    self.pick_kind = "font";
                    self.pick = Some((vals, at));
                }
            }
            "fontsize" => {
                let at = self
                    .cell_origin_px(self.cursor)
                    .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                    .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                self.pick_kind = "size";
                self.pick = Some((
                    // Excel の標準の並び(6〜72)
                    ["6", "8", "9", "10", "11", "12", "14", "16", "18", "20",
                     "22", "24", "26", "28", "36", "48", "72"]
                        .iter()
                        .map(|v| v.to_string())
                        .collect(),
                    at,
                ));
            }
            // データタブ: Python 裏方と道具
            "data-from-text" => {
                self.commit();
                self.import_text_dialog(cx);
            }
            "python" => {
                self.commit();
                self.prompt = Some(("py", Editor::new("")));
            }
            // 参照のトレース。矢印の代わりに**セルを光らせる**(見え方だけ)
            "trace-prec" => {
                self.commit();
                let deps = self
                    .sheet()
                    .get(self.cursor)
                    .and_then(|c| c.formula.as_ref())
                    .map(|f| sheet::calc::deps(f))
                    .unwrap_or_default();
                if deps.is_empty() {
                    self.status = ui::t!("このセルの式は他のセルを参照していません").into();
                } else {
                    self.status = format!(
                        "{} の参照元 {} セルを光らせました(トレース矢印の削除で消す)",
                        self.cursor.a1(),
                        deps.len()
                    )
                    .into();
                    self.trace = deps.into_iter().map(|p| (p, true)).collect();
                }
            }
            "trace-dep" => {
                self.commit();
                let me = self.cursor;
                let dependents: Vec<Pos> = self
                    .sheet()
                    .cells
                    .iter()
                    .filter(|(_, c)| {
                        c.formula
                            .as_ref()
                            .is_some_and(|f| sheet::calc::deps(f).contains(&me))
                    })
                    .map(|(p, _)| *p)
                    .collect();
                if dependents.is_empty() {
                    self.status = format!("{} を参照している式はありません", me.a1()).into();
                } else {
                    self.status = format!(
                        "{} の参照先 {} セルを光らせました(トレース矢印の削除で消す)",
                        me.a1(),
                        dependents.len()
                    )
                    .into();
                    self.trace = dependents.into_iter().map(|p| (p, false)).collect();
                }
            }
            "remove-arrows" => {
                self.trace.clear();
                self.status = ui::t!("トレースを消しました").into();
            }
            // 推奨チャート = いまの一手(棒グラフ)をそのまま勧める
            "insrecommend" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status =
                        ui::t!("グラフにする範囲を選んでください(1列目が項目名、2列目からが数)").into();
                } else {
                    let (a, b) = self.sel_rect();
                    self.insert_chart(a, b, cx);
                }
            }
            // ピボットテーブル = polars が裏方。結果は「その時の値」で右に置く
            // (元が変わったら選び直してもう一度 — 開く=再計算の仕掛けは持たない)
            "pivot-insert" => {
                self.commit();
                // 範囲を選んでいなければ表全体を自動検出(オートフィルタと同じ)。
                // カーソルを表に置くだけで挿入できる — 範囲選択は絞りたいときだけ
                let picked = if self.anchor.is_some() {
                    Some(self.sel_rect())
                } else {
                    let (rows, cols) = self.sheet().extent();
                    (rows >= 2 && cols > 0)
                        .then(|| (Pos::new(0, 0), Pos::new(rows - 1, cols - 1)))
                };
                if picked.is_none() {
                    self.status =
                        ui::t!("元の表がありません(1行目が見出し、下にデータの行)").into();
                } else {
                    let (a, b) = picked.unwrap();
                    if b.row <= a.row {
                        self.status = ui::t!("見出しの下にデータの行が要ります").into();
                    } else {
                        let headers: Vec<String> = (a.col..=b.col)
                            .map(|c| {
                                let v = self
                                    .sheet()
                                    .get(Pos::new(a.row, c))
                                    .map(|x| x.value.display())
                                    .unwrap_or_default();
                                if v.is_empty() { col_name(c) } else { v }
                            })
                            .collect();
                        self.status = ui::t!("行に並べる見出しをクリックで選ぶ(複数可)。選んだら「決定」").into();
                        self.pivot_pend = Some(PivotPend {
                            a,
                            b,
                            headers,
                            rows_sel: Vec::new(),
                            cols_sel: Vec::new(),
                            val_sel: String::new(),
                            replace: None,
                        });
                        self.pivot_pick("pivot-rows-pick");
                    }
                }
            }
            // シートの保護。パスワードは掛けない(掛けた振りもしない)—
            // Excel でも「保護されたシート」に見え、解除も同じ1手でできる
            "prot-doc" => {
                let name = self.sheet().name.clone();
                if self.sheet().protected {
                    self.sheet_mut().protected = false;
                    self.dirty = true;
                    self.status = format!(
                        "シート「{name}」の保護を外しました(編集できます。保存で xlsx にも残ります)"
                    )
                    .into();
                } else {
                    self.commit();
                    self.sheet_mut().protected = true;
                    self.dirty = true;
                    self.status = format!(
                        "シート「{name}」を保護しました(編集を堰き止めます。同じ釦で解除。パスワードは掛けません — 掛けた振りもしません)"
                    )
                    .into();
                }
            }
            // 暗号化。パスワードを決めると、保存で ECMA-376 Standard
            // (AES-128)の複合ファイルに包む。空 Enter で解除
            "prot-encrypt" => {
                self.pw_pending = None;
                self.prompt = Some(("pw-set", Editor::new("")));
                self.status = if self.encrypt_pw.is_some() {
                    ui::t!("暗号化は入っています。新しいパスワードを打って Enter(空のまま Enter で暗号化をやめる)").into()
                } else {
                    ui::t!("暗号化: パスワードを打って Enter(次の保存から効きます)").into()
                };
            }
            // デジタル署名。**隣の .sig への添え書き**(Ed25519)。
            // Excel の署名欄には出ない独自方式 — そう言って出す。
            // 有効なら報告だけ、無効・未署名なら(作り直して)署名する
            "prot-sign" => {
                use ed25519_dalek::{Signer as _, Verifier as _};
                let Some(p) = self.path.clone() else {
                    self.status =
                        ui::t!("まだファイルになっていません(先に保存してください)").into();
                    return;
                };
                if self.dirty {
                    self.status =
                        ui::t!("未保存の変更があります。保存してから署名してください").into();
                    return;
                }
                let bytes = match std::fs::read(&p) {
                    Ok(b) => b,
                    Err(e) => {
                        self.status = format!("読めません: {e}").into();
                        return;
                    }
                };
                let sp = sig_path_for(&p);
                // 既にある署名を検める
                if let Ok(txt) = std::fs::read_to_string(&sp) {
                    let field = |k: &str| -> Option<String> {
                        txt.lines()
                            .find(|l| l.starts_with(k))
                            .map(|l| l[k.len()..].trim().to_string())
                    };
                    let ok = (|| -> Option<(String, bool)> {
                        let signer = field("signer:")?;
                        let vk: [u8; 32] = unhex(&field("pubkey:")?)?.try_into().ok()?;
                        let sg: [u8; 64] = unhex(&field("sig:")?)?.try_into().ok()?;
                        let vk = ed25519_dalek::VerifyingKey::from_bytes(&vk).ok()?;
                        let sig = ed25519_dalek::Signature::from_bytes(&sg);
                        Some((signer, vk.verify(&bytes, &sig).is_ok()))
                    })();
                    if let Some((signer, true)) = ok {
                        self.status = format!(
                            "署名は有効です — {signer} が署名した時のままの中身です"
                        )
                        .into();
                        return;
                    }
                }
                // 無い・壊れている・中身が変わった → 署名し(直し)て添える
                match load_or_make_key() {
                    Ok(key) => {
                        let sig = key.sign(&bytes);
                        let txt = format!(
                            "office-sign v1\nsigner: {}\npubkey: {}\nsig: {}\n",
                            lock_identity(),
                            to_hex(key.verifying_key().as_bytes()),
                            to_hex(&sig.to_bytes())
                        );
                        match std::fs::write(&sp, txt) {
                            Ok(_) => {
                                self.status = format!(
                                    "署名しました — 隣の {} に添え書き(独自方式。Excel の署名欄には出ません。もう一度押すと検めます)",
                                    sp.file_name().unwrap_or_default().to_string_lossy()
                                )
                                .into();
                            }
                            Err(e) => {
                                self.status = format!("署名が置けません: {e}").into();
                            }
                        }
                    }
                    Err(e) => self.status = format!("署名できません: {e}").into(),
                }
            }
            // 共同編集モード。実体はファイルの錠(.~lock)による早い者勝ちの
            // 編集権。押すと錠の今を確かめ、先客が去っていれば取り直す
            "coauth-mode" => match self.path.clone() {
                None => {
                    self.status =
                        ui::t!("まだファイルになっていません(保存すると編集権=錠を取ります)").into();
                }
                Some(p) => {
                    if self.my_lock.is_some() {
                        self.status = format!(
                            "編集権はこちら({})にあります。同じブックは先に開いた人が書け、後の人は読むだけになります(錠は .~lock ファイル)",
                            lock_identity()
                        )
                        .into();
                    } else {
                        self.acquire_lock(&p);
                        self.status = match &self.locked_by {
                            Some(who) => format!(
                                "{who} が編集中です(読めますが上書き保存はできません。相手が閉じたら、またこの釦で確かめてください)"
                            )
                            .into(),
                            None => ui::t!("先客が居なくなっていたので、編集権を取り直しました").into(),
                        };
                    }
                }
            },
            "co-showcomment" => {
                self.show_comments = !self.show_comments;
                self.status = if self.show_comments {
                    ui::t!("コメントを表示します").into()
                } else {
                    ui::t!("コメントを隠しました(付いてはいます)").into()
                };
            }
            "co-delcomment" => {
                let p = self.cursor;
                if self.sheet().comments.contains_key(&p) {
                    self.checkpoint();
                    self.book.sheets[self.active].comments.remove(&p);
                    self.dirty = true;
                    self.status =
                        format!("{} のコメントを外しました(Ctrl+Z で戻せます)", p.a1())
                            .into();
                } else {
                    self.status = ui::t!("このセルにコメントはありません").into();
                }
            }
            // バージョン履歴。上書き保存のたびに .jo-history へ残る控えの一覧
            "co-history" => {
                if self.path.is_none() {
                    self.status =
                        ui::t!("まだファイルになっていません(保存すると、上書きのたびに控えが残ります)").into();
                } else {
                    let v = self.versions();
                    if v.is_empty() {
                        self.status =
                            ui::t!("控えはまだありません(上書き保存のたびに .jo-history へ残ります)").into();
                    } else {
                        let names: Vec<String> = v.iter().map(|(n, _)| n.clone()).collect();
                        self.pick_paths = v;
                        self.pick_kind = "history";
                        self.pick = Some((names, (HEAD_W + 60.0, ROW_H + 20.0)));
                        self.status =
                            ui::t!("バージョン履歴: 選ぶと控えを名無しの複製で開きます(いまの書きかけは要るなら先に保存)").into();
                    }
                }
            }
            // チャット。ブックの隣の申し送り帳(.chat.txt)へ名乗り付きで追記。
            // サーバーは無いので生放送ではない — ファイル越しの言伝
            "co-chat" => match self.chat_path() {
                None => {
                    self.status =
                        ui::t!("まだファイルになっていません(保存すると、隣に申し送り帳ができます)").into();
                }
                Some(cp) => {
                    let tail = std::fs::read_to_string(&cp)
                        .map(|t| {
                            t.lines()
                                .rev()
                                .take(3)
                                .map(|l| l.to_string())
                                .collect::<Vec<_>>()
                                .into_iter()
                                .rev()
                                .collect::<Vec<_>>()
                                .join(" / ")
                        })
                        .unwrap_or_default();
                    self.status = if tail.is_empty() {
                        ui::t!("まだ言伝はありません(打って Enter で書き残します)").into()
                    } else {
                        format!("言伝: {tail}").into()
                    };
                    self.prompt = Some(("chat", Editor::new("")));
                }
            },
            // マクロ = Python in Calc と同じ実体(檻の中で .py を回す)
            "plug-macros" => {
                self.commit();
                self.run_python_file_dialog(cx);
                self.status =
                    ui::t!("マクロ: .py を選ぶと檻の中の Python が回ります(b=ブック s=シート。実体は データ > Python と同じ)").into();
            }
            // プラグインの管理。置き場の .py を一覧し、同じ檻で実行
            "plug-manage" => {
                let dir = plugins_dir();
                let mut items: Vec<PathBuf> = std::fs::read_dir(&dir)
                    .ok()
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e == "py"))
                    .collect();
                items.sort();
                if items.is_empty() {
                    self.status = format!(
                        "プラグイン: {} に .py を置くと、ここに並びます",
                        dir.display()
                    )
                    .into();
                } else {
                    let v: Vec<(String, PathBuf)> = items
                        .into_iter()
                        .map(|q| {
                            (
                                q.file_name().unwrap_or_default().to_string_lossy().to_string(),
                                q,
                            )
                        })
                        .collect();
                    let names: Vec<String> = v.iter().map(|(n, _)| n.clone()).collect();
                    self.pick_paths = v;
                    self.pick_kind = "plugin";
                    self.pick = Some((names, (HEAD_W + 60.0, ROW_H + 20.0)));
                    self.status =
                        ui::t!("プラグイン: 選ぶと檻の中の Python で実行します(b=ブック s=シート)").into();
                }
            }
            // チェックボックス(セルの部品)。空のセルに FALSE を置くと
            // ☑/☐ で見え、空白キーで切り替わる(Excel では TRUE/FALSE の値)
            "inscheckbox" => {
                self.commit();
                let (a, b) = self.sel_rect();
                let mut empties = Vec::new();
                let mut bools = 0usize;
                let mut skipped = 0usize;
                for r in a.row..=b.row {
                    for c in a.col..=b.col {
                        let p = Pos::new(r, c);
                        match self.sheet().get(p).map(|x| &x.value) {
                            None | Some(Value::Empty) => empties.push(p),
                            Some(Value::Bool(_)) => bools += 1,
                            _ => skipped += 1,
                        }
                    }
                }
                if empties.is_empty() && bools == 0 {
                    self.status =
                        ui::t!("空のセルを選んでください(中身のあるセルは潰しません)").into();
                } else {
                    if !empties.is_empty() {
                        self.checkpoint();
                        for p in &empties {
                            let mut cell =
                                self.sheet().get(*p).cloned().unwrap_or_default();
                            cell.formula = None;
                            cell.value = Value::Bool(false);
                            self.book.sheets[self.active].set(*p, cell);
                        }
                        recalc_book(&mut self.book, self.active);
                        self.dirty = true;
                        self.sync_input();
                    }
                    let skip_note = if skipped > 0 {
                        format!("。中身のある {skipped} セルは触っていません")
                    } else {
                        String::new()
                    };
                    self.status = format!(
                        "チェックボックスを {} 個置きました(空白キーで切替。Excel では TRUE/FALSE で見えます{skip_note})",
                        empties.len()
                    )
                    .into();
                }
            }
            // スライサー。カーソルの列の一意な値を釦で並べ、押して絞る。
            // 絞り込みと同じく**見え方だけ**(保存される中身は変わらない)
            "insslicer" => {
                if self.slicer.take().is_none() {
                    self.commit();
                    let col = self.cursor.col;
                    let (rows, _) = self.sheet().extent();
                    if rows < 2 {
                        self.status =
                            ui::t!("スライサーにする列を選んでください(見出しの下にデータの行が要ります)").into();
                    } else {
                        self.slicer =
                            Some((col, std::collections::BTreeSet::new(), false));
                        self.status = format!(
                            "スライサー: {} 列の値を押して絞る(≡=複数選択 / ✕=解除。見え方だけで、中身は変わりません)",
                            col_name(col)
                        )
                        .into();
                    }
                }
            }
            // テキストアート。文字を板に打つと飾り文字を描いて画像で置く
            "instextart" => {
                self.commit();
                self.prompt = Some(("textart", Editor::new("")));
                self.status =
                    ui::t!("テキストアート: 文字を打つと、太字+縁取りの飾り文字を画像で置きます").into();
            }
            // 方程式(数式エディタ)。式を板に打つと mathtext が清書して画像で置く
            "insequation" => {
                self.commit();
                self.prompt = Some(("equation", Editor::new("")));
                self.status =
                    ui::t!("方程式: TeX の書き方で(例: \\frac{a}{b} や \\sum_{i=1}^n i^2)。Enter で清書").into();
            }
            // SmartArt。分類 → 形の2段の一覧(分類・並び・名前は本家)
            "inssmartart" => {
                self.commit();
                let names: Vec<String> =
                    SMARTART.iter().map(|(n, _)| n.to_string()).collect();
                self.pick_kind = "sa-cat";
                self.pick = Some((names, (HEAD_W + 60.0, ROW_H + 20.0)));
                self.status =
                    ui::t!("SmartArt: 分類 → 形の順に選ぶ(図形の集まりとして入ります)").into();
            }
            // ソルバー。ONLYOFFICE と同じ小窓を開く(解法も同じ単体法 LP)
            "solver" => {
                if self.solver.take().is_none() {
                    self.commit();
                    let init = if self.anchor.is_some() {
                        self.sel_rect().0.a1()
                    } else {
                        self.cursor.a1()
                    };
                    self.solver = Some(Solver::new(&init));
                    self.status =
                        ui::t!("ソルバー: 欄を押して打つ。目的・変数セル・制約を決めて「解を求める」").into();
                }
            }
            // 下付き(vertAlign subscript)。上付きは本家 calc にも無い
            "subscript" => {
                self.fmt(|f| f.subscript = !f.subscript);
                self.status = ui::t!("下付きを切り替えました").into();
            }
            // 両端揃え(セルの横揃え。折り返した行を左右に伸ばす)
            "align-just" => {
                self.fmt(|f| {
                    f.align = if f.align == sheet::model::HAlign::Justify {
                        sheet::model::HAlign::General
                    } else {
                        sheet::model::HAlign::Justify
                    };
                    f.wrap = true; // 揃えるには折り返しが要る
                });
                self.status = ui::t!("両端揃えにしました(折り返して全体を表示も入れます)").into();
            }
            // 文字の回転(縦書きのセル。90度ずつ回る)
            // 文字の向き: 本家のプリセット+任意の角度(第2便3段)
            "text-orient" => {
                let at = self
                    .cell_origin_px(self.cursor)
                    .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                    .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                let cur = self
                    .sheet()
                    .get(self.cursor)
                    .and_then(|c| c.fmt.rotation)
                    .unwrap_or(0);
                let items: Vec<String> = [
                    ("角度なし", 0i32),
                    ("左上がり 45度", 45),
                    ("右下がり 45度", 135),
                    ("上向き 90度", 90),
                    ("下向き 90度", 180),
                    ("縦書き(1字ずつ積む)", 255),
                ]
                .iter()
                .map(|(n, deg)| {
                    if *deg == cur as i32 { format!("✓ {n}") } else { n.to_string() }
                })
                .chain(std::iter::once("その他(角度を打つ)…".to_string()))
                .collect();
                self.pick_note = Some(ui::t!("文字の向き(xlsx と同じ数え方 — 上向きが正)").into());
                self.pick_kind = "orient-pick";
                self.pick = Some((items, at));
            }
            // 計算方法(自動 ⇔ 手動)。手動のときは F9 で計算する
            "calc-mode" => {
                self.auto_calc = !self.auto_calc;
                // ファイルにも残す(calcPr)。開き直して勝手に自動へ戻さない
                self.book.calc_manual = !self.auto_calc;
                self.dirty = true;
                self.status = if self.auto_calc {
                    ui::t!("計算方法: 自動(いつもすぐ計算します)").into()
                } else {
                    ui::t!("計算方法: 手動(F9 で計算します — 大きな表で待たされない)").into()
                };
            }
            // 関数の挿入 = 本家と同じ小窓(検索・分類・一覧・説明)。
            // 数式バーの fx と同じ実体
            "insert-function" => {
                self.fn_dlg = Some(FnDlg {
                    search: Editor::new(""),
                    group: 0,
                    sel: 0,
                });
                self.status =
                    ui::t!("関数を挿入: 打って絞り込み、↑↓で選んで Enter(Esc で取消)").into();
            }
            // セルのスタイル(既定の書式の組。押すと一覧から選ぶ)
            "cell-styles" => {
                self.pick_kind = "cell-style";
                self.pick = Some((
                    CELL_STYLES.iter().map(|(n, _)| n.to_string()).collect(),
                    (HEAD_W + 60.0, ROW_H + 20.0),
                ));
                self.status = ui::t!("セルのスタイル: 選ぶと選択に掛かります(Ctrl+Z で戻せます)").into();
            }
            // シートの表示(隠したシートを戻す/いまのシートを隠す)
            "sheet-view" => {
                let hidden: Vec<(usize, String)> = self
                    .book
                    .sheets
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.hidden)
                    .map(|(i, s)| (i, s.name.clone()))
                    .collect();
                if hidden.is_empty() {
                    // 隠すほう(最後の1枚は隠さない — 見えるシートがゼロになる)
                    if self.book.sheets.iter().filter(|s| !s.hidden).count() <= 1 {
                        self.status = ui::t!("最後の1枚は隠せません").into();
                    } else {
                        let n = self.sheet().name.clone();
                        self.checkpoint_book();
                        self.sheet_mut().hidden = true;
                        // 見えるシートへ移る
                        if let Some(i) = self.book.sheets.iter().position(|s| !s.hidden) {
                            self.switch_sheet(i);
                        }
                        self.dirty = true;
                        self.status = format!(
                            "シート「{n}」を隠しました(同じ釦で戻せます。保存で xlsx にも残ります)"
                        )
                        .into();
                    }
                } else {
                    self.pick_kind = "unhide";
                    self.pick_paths = hidden
                        .iter()
                        .map(|(i, n)| (n.clone(), PathBuf::from(i.to_string())))
                        .collect();
                    self.pick = Some((
                        hidden.into_iter().map(|(_, n)| n).collect(),
                        (HEAD_W + 60.0, ROW_H + 20.0),
                    ));
                    self.status = ui::t!("隠したシート: 選ぶと表示に戻します").into();
                }
            }
            // ウォッチウィンドウ(見張りの窓)。選んだセルを控えて下に見せる
            "watch" => {
                let (a, b) = self.sel_rect();
                let mut n = 0usize;
                for r in a.row..=b.row {
                    for c in a.col..=b.col {
                        let p = Pos::new(r, c);
                        if self.sheet().get(p).and_then(|x| x.formula.as_ref()).is_some()
                            || self.anchor.is_none()
                        {
                            if !self.watch.contains(&(self.active, p)) {
                                self.watch.push((self.active, p));
                                n += 1;
                            }
                        }
                    }
                }
                if n == 0 && !self.watch.is_empty() {
                    self.watch.clear();
                    self.status = ui::t!("見張りを空にしました").into();
                } else {
                    self.status = format!(
                        "{n} 個を見張ります(値は下の帯に出ます。もう一度押すと空に)"
                    )
                    .into();
                }
            }
            // 昇順/降順(ホーム・データ)。右クリックの並べ替え▸と同じ道
            "sort-asc" | "sort-desc" => self.sort_active(id == "sort-asc"),
            // 描画の「選択」= 道具を措いてセルの操作に戻る(本家の並びの先頭)
            "draw-select" => {
                self.tool = None;
                self.ink_cur = None;
                self.status = ui::t!("セルの操作に戻りました").into();
            }
            // 描画(ペン・蛍光ペン・消しゴム)。writer と同じ形の道具の入切
            "pen" | "highlighter" | "eraser" => {
                let t = match id {
                    "pen" => 0u8,
                    "highlighter" => 1,
                    _ => 2,
                };
                self.tool = if self.tool == Some(t) { None } else { Some(t) };
                self.ink_cur = None;
                self.status = match self.tool {
                    Some(0) => ui::t!("ペン: 表の上をドラッグで描く(もう一度押すか Esc で戻る)").into(),
                    Some(1) => ui::t!("蛍光ペン: ドラッグで引く(セルの上に薄く乗る)").into(),
                    Some(2) => ui::t!("消しゴム: 線をなぞると1筆ずつ消える").into(),
                    _ => ui::t!("セルの操作に戻りました").into(),
                };
            }
            // AI タブ。**モデルに任せる変換と生成の道具箱**(writer と同じ宛先)
            "ai-where" => {
                let next = ui::ai::backend().next();
                ui::ai::set_backend(next);
                self.status = match ui::ai::ready(next) {
                    Ok(_) => format!("AI の宛先: {}(覚えました)", next.label()).into(),
                    Err(e) => format!(
                        "AI の宛先: {} — ただし今は使えません: {e}",
                        next.label()
                    )
                    .into(),
                };
            }
            "ai-summary" => self.ai_go(CalcAi::Summary, cx),
            "ai-rewrite" => self.ai_go(
                CalcAi::Rewrite(
                    "あなたは表の中の文字を整える道具です。渡されたタブ区切りの表と\
                     同じ行数・同じ列数のタブ区切りだけを返します。文字は意味を\
                     変えずに読みやすく直し、数字と空欄はそのまま写します。",
                    "次の表の文字を、意味を変えずに読みやすく直してください。",
                ),
                cx,
            ),
            "ai-polite" => self.ai_go(
                CalcAi::Rewrite(
                    "あなたは表の中の文字を整える道具です。渡されたタブ区切りの表と\
                     同じ行数・同じ列数のタブ区切りだけを返します。文字は内容を\
                     変えずに丁寧な言い方(です・ます)へ直し、数字と空欄はそのまま\
                     写します。",
                    "次の表の文字を、内容を変えずに丁寧な言い方へ直してください。",
                ),
                cx,
            ),
            "ai-plain" => self.ai_go(
                CalcAi::Rewrite(
                    "あなたは表の中の文字をやさしくする道具です。渡されたタブ区切りの\
                     表と同じ行数・同じ列数のタブ区切りだけを返します。難しい言葉を\
                     やさしい言葉に置き換え、数字と空欄はそのまま写します。",
                    "次の表の文字を、内容を変えずにやさしい日本語へ直してください。",
                ),
                cx,
            ),
            "ai-translate" => self.ai_go(CalcAi::Translate, cx),
            "ai-furigana" => self.ai_go(CalcAi::Furigana, cx),
            "ai-continue" => self.ai_go(CalcAi::Continue, cx),
            "ai-table" => {
                self.commit();
                self.prompt = Some(("ai-table", Editor::new("")));
                self.status = format!(
                    "AI({})が表にします: 文章を打って(貼って)Enter",
                    ui::ai::backend().label()
                )
                .into();
            }
            "ai-ask" => {
                self.commit();
                self.prompt = Some(("ai-ask", Editor::new("")));
                self.status = format!(
                    "AI({})に頼む: 用件を打って Enter(選んだ範囲があれば一緒に渡します)",
                    ui::ai::backend().label()
                )
                .into();
            }
            // 配色の変更(テーマ色の組を入れ替える)。テーマ由来の色を
            // 使っているセルは、色がそのまま追従する
            "colorschemas" => {
                self.pick_kind = "scheme";
                self.pick = Some((
                    sheet::theme::SCHEMES.iter().map(|(n, _)| n.to_string()).collect(),
                    (HEAD_W + 60.0, ROW_H + 20.0),
                ));
                self.status = ui::t!("配色の変更: 選ぶとテーマ色が入れ替わります").into();
            }
            // インターフェイステーマ(画面の明暗)。**セルは白のまま**
            "theme" => {
                self.dark = !self.dark;
                self.status = if self.dark {
                    ui::t!("画面を暗くしました(セルは白のまま — 画面と紙の一致を守る)").into()
                } else {
                    ui::t!("画面を明るくしました").into()
                };
            }
            // 範囲に変換する(表オブジェクトを外す。**書式と式は残る**)
            "td-torange" => {
                self.commit();
                let p = self.cursor;
                match self.sheet().tables.iter().position(|t| t.contains(p)) {
                    None => {
                        self.status =
                            ui::t!("表の中にカーソルを置いてください(表のない範囲は「表の挿入」で表にできます)").into();
                    }
                    Some(i) => {
                        self.checkpoint();
                        let t = self.book.sheets[self.active].tables.remove(i);
                        self.dirty = true;
                        self.status = format!(
                            "表「{}」を普通の範囲に戻しました(帯や縞々の書式と式はそのまま残ります)",
                            t.name
                        )
                        .into();
                    }
                }
            }
            // テーブルのサイズ変更(範囲を変える)。板で新しい範囲を聞く
            "td-resize" => {
                self.commit();
                let p = self.cursor;
                match self.sheet().tables.iter().position(|t| t.contains(p)) {
                    None => self.status = ui::t!("表の中にカーソルを置いてください").into(),
                    Some(i) => {
                        let t = &self.sheet().tables[i];
                        let init = format!("{}:{}", t.a.a1(), t.b.a1());
                        self.status = format!("表「{}」の新しい範囲は?", t.name).into();
                        self.prompt = Some(("table-resize", Editor::new(&init)));
                    }
                }
            }
            // シートの方向(右から左へ)。**日本語も右から書くことがある**
            "rtl-sheet" => {
                let on = !self.sheet().rtl;
                self.sheet_mut().rtl = on;
                self.dirty = true;
                self.status = if on {
                    ui::t!("右から左へ並べます(右横書き。列は右から A B C…)").into()
                } else {
                    ui::t!("左から右へ戻しました").into()
                };
            }
            // 文字の向き(セルの中を右横書きに)。1字ずつ右から並べる
            "direction" => {
                self.fmt(|f| f.rtl_text = !f.rtl_text);
                self.status =
                    ui::t!("セルの中を右横書きにしました(1字ずつ右から。昔の看板の書き方)").into();
            }
            // 表示タブ(本家のデスクトップ版に合わせる)。どれも見え方だけ
            "zoom-in" => {
                self.zoom = (self.zoom + 0.1).min(2.0);
                self.status = format!("ズーム {}%", (self.zoom * 100.0).round() as i32).into();
            }
            "zoom-out" => {
                self.zoom = (self.zoom - 0.1).max(0.5);
                self.status = format!("ズーム {}%", (self.zoom * 100.0).round() as i32).into();
            }
            // 画面の文字の大きさ(リボン・数式バー・メニュー・状態行まで全部)。
            // 格子のズームとは別。設定に覚えて、次回も同じ大きさで開く
            "ui-bigger" | "ui-smaller" => {
                let step = if id == "ui-bigger" { 0.1 } else { -0.1 };
                self.ui_scale = ((self.ui_scale + step) * 10.0).round() / 10.0;
                // 上限は 150% — これ以上は板や欄の設えが崩れる(発注者 2026-08-07)
                self.ui_scale = self.ui_scale.clamp(0.8, 1.5);
                // 試験では書かない(実利用者の settings.toml を汚さない)
                if !cfg!(test) {
                    ui::settings::set("ui_scale", &format!("{:.1}", self.ui_scale));
                }
                self.status = ui::tf!(
                    "画面の文字の大きさ {}%(次回もこの大きさで開きます)",
                    (self.ui_scale * 100.0).round() as i32
                )
                .into();
            }
            "formula-bar" => {
                self.show_formula_bar = !self.show_formula_bar;
                self.status = if self.show_formula_bar {
                    ui::t!("数式バーを表示します").into()
                } else {
                    ui::t!("数式バーを隠しました(表示タブで戻せます)").into()
                };
            }
            "show-headings" => {
                self.show_headers = !self.show_headers;
                self.status = if self.show_headers {
                    ui::t!("見出しを表示します").into()
                } else {
                    ui::t!("見出しを隠しました(列幅のドラッグ等は見出しと一緒に戻ります)").into()
                };
            }
            "show-zeros" => {
                self.show_zeros = !self.show_zeros;
                self.status = if self.show_zeros {
                    ui::t!("0 を表示します").into()
                } else {
                    ui::t!("0 を隠しました(見え方だけ — 値は 0 のまま)").into()
                };
            }
            // 小計(Excel の集計)。本家のデータタブに無い釦だが、グループ化を
            // 「畳むと合計が残る」形で使うために要る(発注者指摘 2026-08-04)
            "subtotal" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status = ui::t!("表を範囲で選んでください(1行目が見出し)").into();
                } else {
                    let (a, b) = self.sel_rect();
                    if b.row <= a.row {
                        self.status = ui::t!("見出しの下にデータの行が要ります").into();
                    } else {
                        let headers: Vec<String> = (a.col..=b.col)
                            .map(|c| {
                                let v = self
                                    .sheet()
                                    .get(Pos::new(a.row, c))
                                    .map(|x| x.value.display())
                                    .unwrap_or_default();
                                if v.is_empty() { col_name(c) } else { v }
                            })
                            .collect();
                        self.status = format!(
                            "何の区切りで集めるか(見出しを1つ): {}",
                            headers.join(" / ")
                        )
                        .into();
                        self.sub_pend = Some(PivotPend {
                            val_sel: String::new(),
                            replace: None,
                            a,
                            b,
                            headers,
                            rows_sel: Vec::new(),
                            cols_sel: Vec::new(),
                        });
                        self.prompt = Some(("subtotal-by", Editor::new("")));
                    }
                }
            }
            // グループ化(アウトライン)。行か列かは選択の形で決める:
            // 見出しから列をまるごと選んでいれば列、それ以外は選択の行。
            // 深さは xlsx の outlineLevel と往復し、畳みも保存に残る
            "group" | "ungroup" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status =
                        ui::t!("まとめたい行(または列)を選んでください(見出しの番号を撫でる)").into();
                } else {
                    let (a, b) = self.sel_rect();
                    let (rows_ext, cols_ext) = self.sheet().extent();
                    let whole_rows = a.row == 0 && b.row + 1 >= rows_ext.max(1);
                    let on_cols = whole_rows && !(a.col == 0 && b.col + 1 >= cols_ext.max(1));
                    self.checkpoint();
                    let add = id == "group";
                    let sh = self.sheet_mut();
                    if on_cols {
                        for c in a.col..=b.col {
                            let l = sh.col_outline.get(&c).copied().unwrap_or(0);
                            let nl = if add { (l + 1).min(7) } else { l.saturating_sub(1) };
                            if nl == 0 {
                                sh.col_outline.remove(&c);
                                sh.col_hidden.remove(&c);
                            } else {
                                sh.col_outline.insert(c, nl);
                            }
                        }
                    } else {
                        for r in a.row..=b.row {
                            let l = sh.row_outline.get(&r).copied().unwrap_or(0);
                            let nl = if add { (l + 1).min(7) } else { l.saturating_sub(1) };
                            if nl == 0 {
                                sh.row_outline.remove(&r);
                                sh.row_hidden.remove(&r);
                            } else {
                                sh.row_outline.insert(r, nl);
                            }
                        }
                    }
                    self.dirty = true;
                    let what = if on_cols {
                        format!("{}〜{}列", col_name(a.col), col_name(b.col))
                    } else {
                        format!("{}〜{}行", a.row + 1, b.row + 1)
                    };
                    self.status = if add {
                        format!(
                            "{what}をグループ化しました(深さ+1。「詳細の非表示」で畳めます。Ctrl+Z で戻せます)"
                        )
                        .into()
                    } else {
                        format!("{what}のグループ化を1段解きました(Ctrl+Z で戻せます)").into()
                    };
                }
            }
            // 詳細の非表示=グループ化した行(列)を畳む / 詳細の表示=開く。
            // 対象は選択、無ければカーソルの行が属するグループのひとつながり
            "hide-details" | "show-details" => {
                self.commit();
                let hide = id == "hide-details";
                let (a, b) = self.sel_rect();
                let (rows_ext, cols_ext) = self.sheet().extent();
                let whole_rows =
                    self.anchor.is_some() && a.row == 0 && b.row + 1 >= rows_ext.max(1);
                let on_cols = whole_rows && !(a.col == 0 && b.col + 1 >= cols_ext.max(1));
                if on_cols {
                    let sh = self.sheet();
                    let targets: Vec<u32> = (a.col..=b.col)
                        .filter(|c| sh.col_outline.contains_key(c))
                        .collect();
                    if targets.is_empty() {
                        self.status =
                            ui::t!("選択にグループ化した列がありません(先にグループ化)").into();
                    } else {
                        self.checkpoint();
                        let sh = self.sheet_mut();
                        for c in &targets {
                            if hide {
                                sh.col_hidden.insert(*c);
                            } else {
                                sh.col_hidden.remove(c);
                            }
                        }
                        self.dirty = true;
                        self.status = format!(
                            "{} 列を{}(Ctrl+Z で戻せます)",
                            targets.len(),
                            if hide { "畳みました" } else { "開きました" }
                        )
                        .into();
                    }
                } else {
                    // 行: 選択、または カーソルの行が属するグループのひとつながり
                    let (r0, r1) = if self.anchor.is_some() {
                        (a.row, b.row)
                    } else {
                        let sh = self.sheet();
                        let at = self.cursor.row;
                        if !sh.row_outline.contains_key(&at) {
                            self.status = ui::t!("グループ化した行の上で押してください(先に データ > グループ化)").into();
                            cx.notify();
                            return;
                        }
                        let mut lo = at;
                        while lo > 0 && sh.row_outline.contains_key(&(lo - 1)) {
                            lo -= 1;
                        }
                        let mut hi = at;
                        while sh.row_outline.contains_key(&(hi + 1)) {
                            hi += 1;
                        }
                        (lo, hi)
                    };
                    let sh = self.sheet();
                    let targets: Vec<u32> =
                        (r0..=r1).filter(|r| sh.row_outline.contains_key(r)).collect();
                    if targets.is_empty() {
                        self.status =
                            ui::t!("選択にグループ化した行がありません(先に データ > グループ化)").into();
                    } else {
                        self.checkpoint();
                        let sh = self.sheet_mut();
                        for r in &targets {
                            if hide {
                                sh.row_hidden.insert(*r);
                            } else {
                                sh.row_hidden.remove(r);
                            }
                        }
                        self.dirty = true;
                        self.status = format!(
                            "{} 行を{}(Ctrl+Z で戻せます)",
                            targets.len(),
                            if hide { "畳みました" } else { "開きました" }
                        )
                        .into();
                    }
                }
            }
            // ピボットの手入れ: どれも「指図を直して置き直す」だけ。
            // 対象はカーソルの下のピボット(指図はブックに控えてある)
            "pivot-refresh" => {
                self.commit();
                match self.pivot_at(self.cursor) {
                    Some(i) => {
                        let d = self.book.pivots[i].clone();
                        self.spawn_pivot(d, Some(i), cx);
                    }
                    None => {
                        self.status =
                            ui::t!("更新したいピボットの上にカーソルを置いてください").into();
                    }
                }
            }
            "pivot-refresh-all" => {
                self.commit();
                let n = self.book.pivots.len();
                if n == 0 {
                    self.status = ui::t!("このブックにピボットはありません").into();
                } else {
                    for i in 0..n {
                        let d = self.book.pivots[i].clone();
                        self.spawn_pivot(d, Some(i), cx);
                    }
                    self.status = format!("{n} 件のピボットを更新しています…").into();
                }
            }
            // フィールドリスト: いまの指図を ✓ 入りで4段に読み込み、
            // 集計まで選んだら同じ場所に置き直す(作り直しではなく組み替え)
            "pivot-fields" => {
                self.commit();
                match self.pivot_at(self.cursor) {
                    None => {
                        self.status = ui::t!("ピボットの上にカーソルを置いてください").into();
                    }
                    Some(i) => {
                        let d = self.book.pivots[i].clone();
                        let (a, b) = d.src;
                        let headers: Vec<String> = (a.col..=b.col)
                            .map(|c| {
                                let v = self
                                    .sheet()
                                    .get(Pos::new(a.row, c))
                                    .map(|x| x.value.display())
                                    .unwrap_or_default();
                                if v.is_empty() { col_name(c) } else { v }
                            })
                            .collect();
                        self.pivot_pend = Some(PivotPend {
                            a,
                            b,
                            headers,
                            rows_sel: d.rows_sel.clone(),
                            cols_sel: d.cols_sel.clone(),
                            val_sel: d.value.clone(),
                            replace: Some(i),
                        });
                        self.pivot_pick("pivot-rows-pick");
                    }
                }
            }
            // スタイルギャラリー(帯の色の組)。一覧から選んで掛け直す
            "pivot-style" => {
                self.commit();
                match self.pivot_at(self.cursor) {
                    None => {
                        self.status = ui::t!("ピボットの上にカーソルを置いてください").into();
                    }
                    Some(i) => {
                        let cur = self.book.pivots[i].style.clone();
                        let at = self
                            .cell_origin_px(self.cursor)
                            .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                            .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                        let items: Vec<String> = ["青(既定)", "緑", "橙", "灰"]
                            .iter()
                            .map(|n| {
                                let key = if *n == "青(既定)" { "" } else { *n };
                                if key == cur {
                                    format!("✓ {n}")
                                } else {
                                    n.to_string()
                                }
                            })
                            .collect();
                        self.pick_note =
                            Some(ui::t!("ピボットのスタイル(選ぶと掛け直します)").into());
                        self.pick_kind = "pivot-style-pick";
                        self.pick = Some((items, at));
                    }
                }
            }
            "pivot-select" => {
                match self.pivot_at(self.cursor) {
                    Some(i) => {
                        let d = &self.book.pivots[i];
                        self.cursor = d.dest;
                        self.anchor = Some(Pos::new(
                            d.dest.row + d.size.0.saturating_sub(1),
                            d.dest.col + d.size.1.saturating_sub(1),
                        ));
                        self.sync_input();
                        self.status = ui::t!("ピボット全体を選びました").into();
                    }
                    None => {
                        self.status = ui::t!("ピボットの上にカーソルを置いてください").into();
                    }
                }
            }
            "pivot-totals" | "pivot-subtotals" | "pivot-blank" | "pivot-layout" => {
                self.commit();
                match self.pivot_at(self.cursor) {
                    None => {
                        self.status = ui::t!("ピボットの上にカーソルを置いてください").into();
                    }
                    Some(i) => {
                        // レイアウト(コンパクト⇔表形式)も行の見出しが1つだと
                        // 見た目が変わらない — 黙って置き直さず、正直に言う
                        let need_two =
                            matches!(id, "pivot-subtotals" | "pivot-blank" | "pivot-layout");
                        if need_two && self.book.pivots[i].rows_sel.len() < 2 {
                            self.status =
                                ui::t!("行の見出しが2つ以上のピボットで効きます(挿入で複数選ぶ)").into();
                        } else {
                            let d = &mut self.book.pivots[i];
                            let (name, on) = match id {
                                "pivot-totals" => {
                                    d.totals = !d.totals;
                                    ("総計", d.totals)
                                }
                                "pivot-subtotals" => {
                                    d.subtotals = !d.subtotals;
                                    ("小計", d.subtotals)
                                }
                                "pivot-blank" => {
                                    d.blank_rows = !d.blank_rows;
                                    ("空行", d.blank_rows)
                                }
                                _ => {
                                    d.compact = !d.compact;
                                    ("コンパクト形式", d.compact)
                                }
                            };
                            let d = self.book.pivots[i].clone();
                            self.dirty = true;
                            self.status = format!(
                                "{name}を{}にして置き直します…",
                                if on { "あり" } else { "なし" }
                            )
                            .into();
                            self.spawn_pivot(d, Some(i), cx);
                        }
                    }
                }
            }
            // 表のデザイン: 表オブジェクトは持たない。選択に**1手ずつ掛ける道具**
            // (掛けた書式・式が帳面に残るだけ。切り替え式に見せない。
            // まとめて掛けるなら挿入タブの「表の挿入」)
            "td-header" | "td-band-row" | "td-band-col" | "td-first" | "td-last" => {
                // 表の中なら、表オブジェクトの性質も一緒に更新する
                let pcur = self.cursor;
                if let Some(i) = self.sheet().tables.iter().position(|t| t.contains(pcur)) {
                    let t = &mut self.book.sheets[self.active].tables[i];
                    match id {
                        "td-header" => t.header = !t.header,
                        "td-band-row" => t.banded_rows = !t.banded_rows,
                        "td-band-col" => t.banded_cols = !t.banded_cols,
                        "td-first" => t.first_col = !t.first_col,
                        _ => t.last_col = !t.last_col,
                    }
                    self.dirty = true;
                }
                self.commit();
                if self.anchor.is_none() {
                    self.status = ui::t!("表の範囲を選んでください").into();
                } else {
                    self.checkpoint();
                    let (a, b) = self.sel_rect();
                    for r in a.row..=b.row {
                        for c in a.col..=b.col {
                            let p = Pos::new(r, c);
                            let mut cell = self.sheet().get(p).cloned().unwrap_or_default();
                            let touched = match id {
                                "td-header" if r == a.row => {
                                    cell.fmt.bold = true;
                                    cell.fmt.fill = Some("D5E8DC".into());
                                    cell.fmt.borders.top = sheet::model::Edge::THIN;
                                    true
                                }
                                "td-band-row" if r > a.row && (r - a.row) % 2 == 0 => {
                                    cell.fmt.fill = Some("F1F6F3".into());
                                    true
                                }
                                "td-band-col" if (c - a.col) % 2 == 1 => {
                                    cell.fmt.fill = Some("F1F6F3".into());
                                    true
                                }
                                "td-first" if c == a.col => {
                                    cell.fmt.bold = true;
                                    true
                                }
                                "td-last" if c == b.col => {
                                    cell.fmt.bold = true;
                                    true
                                }
                                _ => false,
                            };
                            if touched {
                                self.book.sheets[self.active].set(p, cell);
                            }
                        }
                    }
                    self.dirty = true;
                    let what = match id {
                        "td-header" => "1行目を見出しの帯に",
                        "td-band-row" => "1行おきの縞々に",
                        "td-band-col" => "1列おきの縞々に",
                        "td-first" => "最初の列を太字に",
                        _ => "最後の列を太字に",
                    };
                    self.status = format!(
                        "{}:{} を{}しました(Ctrl+Z で戻せます)",
                        a.a1(),
                        b.a1(),
                        what
                    )
                    .into();
                }
            }
            // 合計行 = 選択の下に =SUM(…) の行を足す(式なので元が変われば追従)
            "td-total" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status = ui::t!("合計したい表の範囲を選んでください").into();
                } else {
                    let (a, b) = self.sel_rect();
                    let below_used = (a.col..=b.col).any(|c| {
                        self.sheet()
                            .get(Pos::new(b.row + 1, c))
                            .map(|cell| {
                                !cell.value.display().is_empty() || cell.formula.is_some()
                            })
                            .unwrap_or(false)
                    });
                    if below_used {
                        self.status =
                            ui::t!("すぐ下の行に中身があります(空けてから — 黙って上書きしません)").into();
                    } else {
                        self.checkpoint();
                        add_total_row(&mut self.book.sheets[self.active], a, b);
                        recalc_book(&mut self.book, self.active);
                        self.dirty = true;
                        self.status = format!(
                            "{} 行目に合計(=SUM)を足しました。式なので元が変われば追従します(Ctrl+Z で戻せます)",
                            b.row + 2
                        )
                        .into();
                    }
                }
            }
            // フィルタのボタン = データタブの絞り込みと同じ実体
            "td-filter" => self.run_cmd("setfilter", cx),
            // 表の挿入 = 選択に表の書式(見出しの帯+縞々+外枠)を掛ける
            "instable" | "table-tpl" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status = ui::t!("表にする範囲を選んでください").into();
                } else {
                    self.checkpoint();
                    let (a, b) = self.sel_rect();
                    for r in a.row..=b.row {
                        for c in a.col..=b.col {
                            let p = Pos::new(r, c);
                            let mut cell = self.sheet().get(p).cloned().unwrap_or_default();
                            if r == a.row {
                                cell.fmt.bold = true;
                                cell.fmt.fill = Some("D5E8DC".into());
                            } else if (r - a.row) % 2 == 0 {
                                cell.fmt.fill = Some("F1F6F3".into());
                            }
                            if r == a.row {
                                cell.fmt.borders.top = sheet::model::Edge::THIN;
                            }
                            if r == b.row {
                                cell.fmt.borders.bottom = sheet::model::Edge::THIN;
                            }
                            if c == a.col {
                                cell.fmt.borders.left = sheet::model::Edge::THIN;
                            }
                            if c == b.col {
                                cell.fmt.borders.right = sheet::model::Edge::THIN;
                            }
                            self.book.sheets[self.active].set(p, cell);
                        }
                    }
                    let n = self.book.sheets.iter().map(|s| s.tables.len()).sum::<usize>() + 1;
                    self.book.sheets[self.active].tables.push(sheet::model::TableDef {
                        name: format!("テーブル{n}"),
                        a,
                        b,
                        ..Default::default()
                    });
                    self.dirty = true;
                    self.status = format!(
                        "{}:{} を表にしました(見出しの帯と縞々。範囲に変換・サイズ変更もできます。Ctrl+Z で戻せます)",
                        a.a1(),
                        b.a1()
                    )
                    .into();
                }
            }
            // 記号を挿入: 一覧から選んで**数式バーへ**差し込む(セルは置き換えない)
            "inssymbol" => {
                let at = self
                    .cell_origin_px(self.cursor)
                    .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                    .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                self.pick_kind = "symbol";
                self.pick = Some((
                    ["〒", "℡", "№", "㈱", "〆", "※", "→", "←", "↑", "↓",
                     "○", "●", "◎", "△", "▲", "×", "☑", "☐", "✓", "①", "②", "③"]
                        .iter()
                        .map(|v| v.to_string())
                        .collect(),
                    at,
                ));
            }
            "addcomment" => {
                self.commit();
                let cur = self.sheet().comments.get(&self.cursor).cloned().unwrap_or_default();
                self.prompt = Some(("comment", Editor::new(&cur)));
            }
            "text-column" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status =
                        ui::t!("割りたいセルを選んでください(選択した列の文字を右へ割ります)").into();
                } else {
                    self.prompt = Some(("split-delim", Editor::new("")));
                }
            }
            "goal-seek" => {
                self.commit();
                // 目標セルの初期値はいまのセル(式のセルの上で押すのが自然)
                let init = if self.sheet().get(self.cursor).and_then(|c| c.formula.as_ref()).is_some()
                {
                    format!("{}=", self.cursor.a1())
                } else {
                    String::new()
                };
                self.goal = None;
                self.prompt = Some(("goal-target", Editor::new(&init)));
            }
            "data-external-links" => {
                // 他のブックを**値として**取り込む(リンクは張らない —
                // リンク切れの帳票を作らない。SEKKEI の分業どおり)
                self.commit();
                let ask = cx.background_executor().spawn(async {
                    let p = rfd::FileDialog::new()
                        .add_filter("Excelブック", &["xlsx"])
                        .pick_file()?;
                    Some(
                        std::fs::File::open(&p)
                            .map_err(|e| e.to_string())
                            .and_then(sheet::xlsx::read)
                            .map(|(b, _)| (p, b)),
                    )
                });
                cx.spawn(async move |this, cx| {
                    let r = ask.await;
                    let _ = this.update(cx, |this, cx| {
                        match r {
                            None => {}
                            Some(Ok((p, mut other))) => {
                                this.checkpoint();
                                sheet::recalc_all(&mut other);
                                let mut n = 0usize;
                                for mut sh in other.sheets.drain(..) {
                                    // 式は計算結果の値に(他所の参照を持ち込まない)
                                    for c in sh.cells.values_mut() {
                                        c.formula = None;
                                    }
                                    sh.name = format!(
                                        "{}({})",
                                        sh.name,
                                        p.file_stem().unwrap_or_default().to_string_lossy()
                                    );
                                    while this.book.sheets.iter().any(|x| x.name == sh.name) {
                                        sh.name.push('+');
                                    }
                                    this.book.sheets.push(sh);
                                    n += 1;
                                }
                                this.dirty = true;
                                this.status = format!(
                                    "{n} シートを値として取り込みました(リンクは張りません)"
                                )
                                .into();
                            }
                            Some(Err(e)) => this.status = format!("取り込めません: {e}").into(),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            // 拡大縮小印刷: 100→90→80→70→50→100
            "scale" => {
                self.commit();
                self.checkpoint();
                let sh = self.sheet_mut();
                let next = match sh.print_scale.unwrap_or(100) {
                    100 => 90,
                    90 => 80,
                    80 => 70,
                    70 => 50,
                    _ => 100,
                };
                sh.print_scale = if next == 100 { None } else { Some(next) };
                self.dirty = true;
                self.status = format!("拡大縮小印刷: {next}%(PDF と保存に効きます)").into();
            }
            // 改ページ: いまの行から新しい紙を始める(もう一度で解除)
            "pagebreak" => {
                self.commit();
                self.checkpoint();
                let r = self.cursor.row;
                let sh = self.sheet_mut();
                if let Some(i) = sh.row_breaks.iter().position(|b| *b == r) {
                    sh.row_breaks.remove(i);
                    self.dirty = true;
                    self.status = format!("{} 行の改ページを外しました", r + 1).into();
                } else if r == 0 {
                    self.undo_stack.pop();
                    self.status = ui::t!("1行目の前では改ページできません").into();
                } else {
                    sh.row_breaks.push(r);
                    self.dirty = true;
                    self.status =
                        format!("{} 行から新しい紙にします(もう一度で解除)", r + 1).into();
                }
            }
            // タイトルを印刷: 選んだ行を各ページの頭で繰り返す。選択なしで解除
            "printtitles" => {
                self.commit();
                if self.anchor.is_some() {
                    self.checkpoint();
                    let (a, b) = self.sel_rect();
                    self.sheet_mut().print_title_rows = Some((a.row, b.row));
                    self.dirty = true;
                    self.status = format!(
                        "{}〜{} 行を各ページの頭で繰り返します(選択なしで押すと解除)",
                        a.row + 1,
                        b.row + 1
                    )
                    .into();
                } else if self.sheet().print_title_rows.is_some() {
                    self.checkpoint();
                    self.sheet_mut().print_title_rows = None;
                    self.dirty = true;
                    self.status = ui::t!("タイトル行を解除しました").into();
                } else {
                    self.status =
                        ui::t!("繰り返す行を選んでから押してください(行の見出しをクリック)").into();
                }
            }
            "print-gridlines" => {
                self.commit();
                self.checkpoint();
                let sh = self.sheet_mut();
                sh.print_gridlines = !sh.print_gridlines;
                let on = sh.print_gridlines;
                self.dirty = true;
                self.status = format!(
                    "枠線の印刷: {}",
                    if on { "する(表の薄い線が紙に出ます)" } else { "しない" }
                )
                .into();
            }
            "print-headings" => {
                self.commit();
                self.checkpoint();
                let sh = self.sheet_mut();
                sh.print_headings = !sh.print_headings;
                let on = sh.print_headings;
                self.dirty = true;
                self.status = format!(
                    "見出しの印刷: {}",
                    if on { "する(行番号と列名が余白に出ます)" } else { "しない" }
                )
                .into();
            }
            // 検索と置換(ホーム > 置き換え)。板を2枚続けて使う
            "replace" => {
                self.commit();
                let init = self.find_term.clone().unwrap_or_default();
                self.prompt = Some(("find", Editor::new(&init)));
            }
            // グラフ(matplotlib)と画像。挿入タブ
            "inschart" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status =
                        ui::t!("グラフにする範囲を選んでください(1列目が項目名、2列目からが数)").into();
                } else {
                    let (a, b) = self.sel_rect();
                    self.insert_chart(a, b, cx);
                }
            }
            "insimage" => {
                self.commit();
                self.insert_image_dialog(cx);
            }
            "instext" => {
                // テキストボックス = 枠の図形 + 文字。すぐ文字の板を開く
                self.checkpoint();
                let at = self.cursor;
                self.sheet_mut().shapes_new.push(sheet::model::SheetShape {
                    at,
                    width_px: 200.0,
                    height_px: 80.0,
                    kind: "rect".into(),
                    fill: None,
                    line: Some("7F7F7F".into()),
                    ..Default::default()
                });
                self.shape_sel = Some(self.sheet().shapes_new.len() - 1);
                self.dirty = true;
                self.prompt = Some(("shape-text", Editor::new("")));
            }
            "inssparkline" => {
                self.commit();
                if self.anchor.is_none() {
                    self.status =
                        ui::t!("折れ線にする数の範囲を選んでください(置き場所はいまのセル)").into();
                } else {
                    let (a, b) = self.sel_rect();
                    let mut vals: Vec<f64> = Vec::new();
                    for r in a.row..=b.row {
                        for c in a.col..=b.col {
                            if let Some(cell) = self.sheet().get(Pos::new(r, c)) {
                                if let sheet::Value::Number(n) = cell.value {
                                    vals.push(n);
                                }
                            }
                        }
                    }
                    if vals.len() < 2 {
                        self.status = ui::t!("数が2つ以上要ります").into();
                    } else {
                        let (lo, hi) = vals
                            .iter()
                            .fold((f64::MAX, f64::MIN), |(l, h), v| (l.min(*v), h.max(*v)));
                        let span = (hi - lo).max(1e-9);
                        let n = vals.len();
                        let points: Vec<(f32, f32)> = vals
                            .iter()
                            .enumerate()
                            .map(|(i, v)| {
                                (
                                    i as f32 / (n - 1) as f32,
                                    (1.0 - ((v - lo) / span)) as f32,
                                )
                            })
                            .collect();
                        // 置き場所はいまのセル(選択の中なら右のセル)、大きさはそのセル
                        let at = if (a.row..=b.row).contains(&self.cursor.row)
                            && (a.col..=b.col).contains(&self.cursor.col)
                        {
                            Pos::new(a.row, b.col + 1)
                        } else {
                            self.cursor
                        };
                        self.checkpoint();
                        let (w, h) = (self.col_px(at.col) - 2.0, self.row_px(at.row) - 2.0);
                        self.sheet_mut().shapes_new.push(sheet::model::SheetShape {
                            at,
                            width_px: w,
                            height_px: h,
                            kind: "spark".into(),
                            fill: None,
                            line: Some("1B6E3C".into()),
                            points,
                            ..Default::default()
                        });
                        self.dirty = true;
                        self.status = format!(
                            "スパークラインを {} に置きました(その時の値で描く固定の線。\
データを変えたら作り直してください)",
                            at.a1()
                        )
                        .into();
                    }
                }
            }
            "insshape" => {
                let at = self
                    .cell_origin_px(self.cursor)
                    .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                    .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                self.pick_kind = "shape";
                self.pick = Some((
                    ["四角形", "角丸四角形", "楕円", "右矢印", "ひし形", "直線"]
                        .iter()
                        .map(|v| v.to_string())
                        .collect(),
                    at,
                ));
            }
            "inshyperlink" => {
                self.commit();
                let cur = self.sheet().links.get(&self.cursor).cloned().unwrap_or_default();
                self.prompt = Some(("link", Editor::new(&cur)));
            }
            // データの入力規則。選んだ範囲に候補を付ける(板で受ける)
            // データの入力規則。本家は 設定/入力メッセージ/エラーアラートの
            // 3タブのダイアログ — calc は種類の一覧 → 聞き取りの板の2段
            "data-validation" => {
                self.commit();
                // 本家の3タブのダイアログと同じ形の板(発注者 2026-08-07)
                self.dv_open();
            }
            // 条件付き書式。右クリックメニューと同じ一覧を開く(道は1本)
            "condformat" => {
                let (x, y) = self
                    .cell_origin_px(self.cursor)
                    .map(|(x, y)| (x + 16.0, y + 16.0))
                    .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                self.menu_at = Some((x, y));
                self.menu_sub = Some("cond");
            }
            // 名前の管理。右クリックの「名前の定義」と同じ板
            "defname" => {
                self.commit();
                self.prompt = Some(("name", Editor::new("")));
            }
            // ウィンドウ枠の固定。本家はドロップダウンで「最上行」「最初の列」を
            // 個別に選べる — トグルだけの形をやめ、一覧から選ぶ
            "freeze" => {
                let at = self
                    .cell_origin_px(self.cursor)
                    .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                    .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                let mut items: Vec<String> = Vec::new();
                if self.frozen.is_some() {
                    items.push("固定の解除".into());
                }
                items.push("いまの位置で固定(上と左が留まる)".into());
                items.push("最上行の固定".into());
                items.push("最初の列の固定".into());
                // 本家の「固定された枠に影を付ける」(viewtab:freezeshadow)
                items.push(if self.freeze_shadow {
                    "✓ 固定した枠に影を付ける".into()
                } else {
                    "固定した枠に影を付ける".into()
                });
                self.pick_kind = "freeze";
                self.pick = Some((items, at));
            }
            // 塗りつぶしの色。本家はパレット — 一覧から選ぶ
            // (順繰りの2色は仮実装だった。発注者指摘 2026-08-06)
            "fillparag" => {
                let at = self
                    .cell_origin_px(self.cursor)
                    .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                    .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                self.pick_kind = "fill-color";
                let mut items: Vec<String> =
                    FILL_COLORS.iter().map(|(n, _)| n.to_string()).collect();
                items.push("その他(RRGGBB を打つ)…".into());
                self.pick = Some((items, at));
            }
            // フォントの色。同じくパレット
            "fontcolor" => {
                let at = self
                    .cell_origin_px(self.cursor)
                    .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                    .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                self.pick_kind = "font-color";
                let mut items: Vec<String> =
                    FONT_COLORS.iter().map(|(n, _)| n.to_string()).collect();
                items.push("その他(RRGGBB を打つ)…".into());
                self.pick = Some((items, at));
            }
            // 並べ替えは**見出しを据え置き、行はまるごと動かす**
            // ユーザー設定の並べ替え。本家は複数基準のダイアログ —
            // calc は小計・ピボットと同じ聞き取りの板で複数基準を受ける
            "custom-sort" => {
                self.commit();
                let (rows, cols) = self.sheet().extent();
                if rows < 2 {
                    self.status = ui::t!("並べ替える表がありません(見出しの下にデータが要ります)").into();
                    return;
                }
                let heads: Vec<String> = (0..cols)
                    .map(|c| {
                        self.sheet()
                            .get(Pos::new(0, c))
                            .map(|x| x.value.display())
                            .unwrap_or_default()
                    })
                    .filter(|h| !h.is_empty())
                    .collect();
                self.prompt = Some(("sort-by", Editor::new("")));
                self.status = ui::tf!(
                    "基準を左から強い順に(例: 金額 降順, 品名)。使える見出し: {}",
                    heads.join(" / ")
                )
                .into();
            }
            "rem-duplicates" => {
                self.commit();
                self.checkpoint();
                let n = self.book.sheets[self.active].remove_duplicate_rows(true);
                self.dirty = true;
                recalc_book(&mut self.book, self.active);
                // 何件消したかを黙らない
                self.status = ui::tf!("重複した {} 行を削除しました", n).into();
            }
            "currency" => self.fmt(|f| f.number_format = Some("¥#,##0".into())),
            "percents" => self.fmt(|f| f.number_format = Some("0%".into())),
            // 関数の一覧。**使える名前だけを出す** — 無いものを並べない
            f @ ("fn-math" | "fn-text" | "fn-logical" | "fn-recent" | "fn-datetime"
            | "fn-lookup" | "fn-financial" | "fn-more") => {
                let names: &str = match f {
                    "fn-math" => "SUM AVERAGE ROUND ROUNDUP ROUNDDOWN INT ABS MOD POWER SQRT \
                                  PRODUCT SUMPRODUCT SUMSQ CEILING FLOOR MROUND EVEN ODD SIGN \
                                  FACT COMBIN PERMUT GCD LCM PI SIN COS TAN ASIN ACOS ATAN ATAN2 \
                                  SINH COSH TANH EXP LN LOG LOG10 DEGREES RADIANS RAND RANDBETWEEN \
                                  SEQUENCE(隣へあふれる。=SEQUENCE(3)+1 のような式も可)",
                    "fn-text" => "LEN LEFT RIGHT MID TRIM UPPER LOWER CONCATENATE CONCAT TEXT \
                                  SUBSTITUTE FIND SEARCH VALUE TEXTJOIN REPT CHAR CODE \
                                  UNICHAR UNICODE PROPER EXACT CLEAN FIXED YEN NUMBERVALUE \
                                  LENB LEFTB RIGHTB MIDB ASC JIS DATESTRING(和暦) \
                                  PHONETIC(ふりがな — 読んだ xlsx の rPh を引く)",
                    "fn-logical" => "IF IFS SWITCH AND OR NOT TRUE FALSE ISBLANK ISERROR IFERROR \
                                     IFNA ISNA ISERR ISLOGICAL ISNONTEXT ISNUMBER ISTEXT NA",
                    "fn-datetime" => "TODAY NOW DATE DATEVALUE YEAR MONTH DAY WEEKDAY \
                                      TIME HOUR MINUTE SECOND EDATE EOMONTH DATEDIF \
                                      WORKDAY NETWORKDAYS DAYS DAYS360 YEARFRAC \
                                      WEEKNUM ISOWEEKNUM(値は通し番号)",
                    "fn-lookup" => "VLOOKUP HLOOKUP XLOOKUP LOOKUP INDEX MATCH CHOOSE \
                                    ROW COLUMN ROWS COLUMNS OFFSET INDIRECT ADDRESS HYPERLINK \
                                    FILTER SORT UNIQUE TRANSPOSE(照合は完全一致。\
                                    FILTER 等は隣へあふれ、四則と組み合わせても効く)",
                    "fn-financial" => "PMT PV FV NPER NPV IRR RATE(IRR と RATE は挟み撃ちの反復解)",
                    "fn-more" => "SUMIF SUMIFS COUNTIF COUNTIFS AVERAGEIF AVERAGEIFS \
                                  MINIFS MAXIFS COUNTA COUNTBLANK TRUNC \
                                  RANK RANK.EQ RANK.AVG LARGE SMALL \
                                  MEDIAN MODE STDEV STDEVP VAR VARP PERCENTILE QUARTILE \
                                  CORREL SLOPE INTERCEPT FORECAST AVERAGEA MAXA MINA \
                                  SUBTOTAL QUOTIENT CEILING.MATH FLOOR.MATH \
                                  ISEVEN ISODD T N TYPE — 一覧は各族の釦で",
                    _ => "SUM AVERAGE COUNT MAX MIN IF SUMIF COUNTIF VLOOKUP TODAY",
                };
                self.status = ui::tf!("使える関数: {}", names).into();
            }
            f @ ("sum" | "average" | "count" | "max" | "min") => {
                // 上の連続した数値をまとめる(表計算の当たり前の動き)
                let name = f.to_uppercase();
                let (r, c) = (self.cursor.row, self.cursor.col);
                let mut top = r;
                while top > 0 && self.sheet().get(Pos::new(top - 1, c))
                    .map(|x| matches!(x.value, Value::Number(_)) || x.formula.is_some())
                    .unwrap_or(false) { top -= 1 }
                let text = if top < r {
                    format!("={name}({}:{})", Pos::new(top, c).a1(), Pos::new(r - 1, c).a1())
                } else {
                    format!("={name}()")
                };
                self.input = Editor::new(&text);
                self.commit();
                self.sync_input();
            }
            other => {
                // ここに来たら結線漏れ。黙らず画面に出す
                self.status = ui::tf!("未配線のコマンド: {}(不具合です)", other).into();
            }
        }
    }
}

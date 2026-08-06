//! main.rs からの純移動(2026-08-06 の分割)。挙動は変えない。

use crate::*;

impl Calc {
    /// 一覧から選んだものを適用する(pick_kind で意味が変わる)。
    pub(crate) fn apply_pick(&mut self, v: &str, cx: &mut Context<Self>) {
        match self.pick_kind {
            "font" => {
                let name = v.to_string();
                self.fmt(move |f| f.font = Some(name.clone()));
                self.status = ui::tf!("書体を「{}」にしました", v).into();
            }
            "size" => {
                if let Ok(pt) = v.parse::<f32>() {
                    self.fmt(move |f| f.size_c = Some((pt * 100.0) as u32));
                    self.status = ui::tf!("文字の大きさを {}pt にしました", v).into();
                }
            }
            "symbol" => {
                // 打ちかけの続きに差し込む(セルを置き換えない)
                self.input.insert(v);
                self.dirty = true;
                self.status = ui::tf!("「{}」を差し込みました(Enter で確定)", v).into();
            }
            "shape" => {
                let kind = match v {
                    "角丸四角形" => "roundRect",
                    "楕円" => "ellipse",
                    "右矢印" => "rightArrow",
                    "ひし形" => "diamond",
                    "直線" => "line",
                    _ => "rect",
                };
                self.checkpoint();
                let at = self.cursor;
                self.sheet_mut().shapes_new.push(sheet::model::SheetShape {
                    at,
                    width_px: 160.0,
                    height_px: 100.0,
                    kind: kind.into(),
                    fill: None,
                    line: Some("1B6E3C".into()),
                    ..Default::default()
                });
                self.shape_sel = Some(self.sheet().shapes_new.len() - 1);
                self.dirty = true;
                self.status = ui::tf!("{}を {} に置きました(ドラッグで移動 / 右下で大きさ / Del で削除)", v, at.a1())
                .into();
            }
            "sa-cat" => {
                if let Some(ci) = SMARTART.iter().position(|(n, _)| *n == v) {
                    self.sa_cat = ci;
                    let names: Vec<String> =
                        SMARTART[ci].1.iter().map(|(n, _)| n.to_string()).collect();
                    self.pick_kind = "sa-item";
                    self.pick = Some((names, (HEAD_W + 120.0, ROW_H + 20.0)));
                    self.status = ui::tf!("SmartArt > {}: 形を選ぶと図形の集まりとして入ります", v)
                    .into();
                    return; // pick_kind を "value" に戻さない(2段目へ)
                }
            }
            "sa-item" => {
                let hit = SMARTART
                    .get(self.sa_cat)
                    .and_then(|(_, items)| items.iter().find(|(n, _)| *n == v));
                if let Some((name, key)) = hit {
                    let (name, key) = (name.to_string(), key.to_string());
                    self.insert_smartart(&name, &key);
                }
            }
            "scheme" => {
                if let Some((_, cols)) = sheet::theme::SCHEMES.iter().find(|(n, _)| *n == v) {
                    self.checkpoint_book();
                    self.book.theme = cols.iter().map(|c| c.to_string()).collect();
                    // テーマ由来の色を持つセルを解き直す(配色に追従させる)
                    let theme = self.book.theme.clone();
                    let mut n = 0usize;
                    for sh in &mut self.book.sheets {
                        for cell in sh.cells.values_mut() {
                            if let Some((i, t)) = cell.fmt.color_theme {
                                cell.fmt.color =
                                    Some(sheet::theme::resolve(&theme, i, t as f32 / 1000.0));
                                n += 1;
                            }
                            if let Some((i, t)) = cell.fmt.fill_theme {
                                cell.fmt.fill =
                                    Some(sheet::theme::resolve(&theme, i, t as f32 / 1000.0));
                                n += 1;
                            }
                        }
                    }
                    self.dirty = true;
                    self.status = ui::tf!("配色を「{}」にしました({} 箇所の色が追従。テーマ色を使っていないセルは変わりません)", v, n)
                    .into();
                }
            }
            // 直入力の補完: 打ちかけの名前を選んだ関数に置き換えて ( まで入れる
            "fn-complete" => {
                let t = self.input.text().to_string();
                let cur = self.input.cursor().min(t.len());
                let tok_len: usize = t[..cur]
                    .chars()
                    .rev()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '.')
                    .map(|c| c.len_utf8())
                    .sum();
                let start = cur - tok_len;
                let mut t2 = t.clone();
                t2.replace_range(start..cur, &format!("{v}("));
                self.input = Editor::new(&t2);
                self.input.move_to(start + v.len() + 1, false);
                self.edit_armed = true;
                self.formula_assist();
            }
            "func-cat" => {
                let id = match v {
                    "統計" => "fn-math",
                    "数学" => "fn-math",
                    "財務" => "fn-financial",
                    "日付" => "fn-datetime",
                    "文字列" => "fn-text",
                    "論理" => "fn-logical",
                    _ => "fn-lookup",
                };
                self.run_cmd(id, cx);
            }
            "cell-style" => {
                if let Some((_, f)) = CELL_STYLES.iter().find(|(n, _)| *n == v) {
                    let f = *f;
                    self.fmt(move |c| f(c));
                    self.status = ui::tf!("セルのスタイル「{}」を掛けました", v).into();
                }
            }
            "unhide" => {
                if let Some((_, path)) = self.pick_paths.iter().find(|(n, _)| n == v).cloned() {
                    if let Ok(i) = path.to_string_lossy().parse::<usize>() {
                        if i < self.book.sheets.len() {
                            self.checkpoint_book();
                            self.book.sheets[i].hidden = false;
                            self.switch_sheet(i);
                            self.dirty = true;
                            self.status = ui::tf!("シート「{}」を表示に戻しました", v).into();
                        }
                    }
                }
                self.pick_paths.clear();
            }
            "freeze" => {
                match v {
                    "固定の解除" => {
                        self.frozen = None;
                        self.status = ui::t!("固定を解きました").into();
                    }
                    "最上行の固定" => {
                        self.frozen = Some(Pos::new(1, 0));
                        self.status = ui::t!("最上行を固定しました").into();
                    }
                    "最初の列の固定" => {
                        self.frozen = Some(Pos::new(0, 1));
                        self.status = ui::t!("最初の列を固定しました").into();
                    }
                    _ => {
                        // いまの位置で固定(その上と左が留まる)
                        if self.cursor.row == 0 && self.cursor.col == 0 {
                            self.status = ui::t!("固定する位置にカーソルを置いてください(その上と左が留まります)").into();
                        } else {
                            self.frozen = Some(self.cursor);
                            self.status = ui::tf!("{}行 {}列を固定しました", self.cursor.row, self.cursor.col).into();
                        }
                    }
                }
            }
            "dv-kind" => {
                match v {
                    "リスト(候補から選ぶ)" => {
                        // 既にある規則は編集の初期値に(直書きは中身、参照は = 付き)
                        let cur = self
                            .sheet()
                            .validation_at(self.cursor)
                            .filter(|x| x.kind == "list")
                            .map(|x| x.formula.clone())
                            .unwrap_or_default();
                        let init = if cur.is_empty() {
                            String::new()
                        } else if let Some(inner) =
                            cur.strip_prefix('"').and_then(|s| s.strip_suffix('"'))
                        {
                            inner.to_string()
                        } else {
                            format!("={cur}")
                        };
                        self.prompt = Some(("validation", Editor::new(&init)));
                    }
                    "整数" | "小数" | "文字数" => {
                        self.dv_pend = Some(match v {
                            "整数" => "whole",
                            "小数" => "decimal",
                            _ => "textLength",
                        });
                        self.prompt = Some(("dv-cond", Editor::new("")));
                        self.status = ui::t!("条件の書き方: 1〜100 / 1〜100 以外 / >=0 / <50 / <>0 / =8(半角の数で)").into();
                    }
                    "入力メッセージ…" => {
                        let cur = self
                            .sheet()
                            .validation_at(self.cursor)
                            .and_then(|x| x.input_msg.clone())
                            .map(|(t, m)| if t.is_empty() { m } else { format!("{t}: {m}") })
                            .unwrap_or_default();
                        self.prompt = Some(("dv-msg", Editor::new(&cur)));
                    }
                    "エラーの文言…" => {
                        let cur = self
                            .sheet()
                            .validation_at(self.cursor)
                            .and_then(|x| x.error_msg.clone())
                            .map(|(s, _, m)| match s.as_str() {
                                "warning" => format!("警告: {m}"),
                                "information" => format!("情報: {m}"),
                                _ => m,
                            })
                            .unwrap_or_default();
                        self.prompt = Some(("dv-err", Editor::new(&cur)));
                    }
                    _ => {
                        // この範囲の規則を外す
                        let (a, b) = self.sel_rect();
                        let overlap = |x: &sheet::model::Validation| {
                            let (ra, rb) = x.range;
                            ra.row <= b.row && rb.row >= a.row && ra.col <= b.col && rb.col >= a.col
                        };
                        let n = self.sheet().validations.iter().filter(|x| overlap(x)).count();
                        if n == 0 {
                            self.status = ui::t!("この範囲に入力規則はありません").into();
                        } else {
                            self.checkpoint();
                            self.book.sheets[self.active].validations.retain(|x| !overlap(x));
                            self.dirty = true;
                            self.status = ui::tf!("{} 本の入力規則を外しました", n).into();
                        }
                    }
                }
                if self.prompt.is_some() {
                    return; // 板の確定まで(pick_kind を戻さない)
                }
            }
            "numfmt-pick" => {
                if v.starts_with("その他") {
                    // 書式コードの直打ち(カスタム書式)。今のコードを下敷きに
                    let cur = self
                        .sheet()
                        .get(self.cursor)
                        .and_then(|c| c.fmt.number_format.clone())
                        .unwrap_or_default();
                    self.prompt = Some(("numfmt-custom", Editor::new(&cur)));
                    return; // pick_kind を戻さない(板の確定まで)
                }
                if let Some((_, code)) = NUMFMTS.iter().find(|(n, _)| *n == v) {
                    let c = code.map(|s| s.to_string());
                    self.fmt(move |f| f.number_format = c.clone());
                    self.status = match code {
                        Some(c) => ui::tf!("数値の書式を「{}」にしました(コード: {})", v, c).into(),
                        None => ui::t!("数値の書式を「一般」に戻しました").into(),
                    };
                }
            }
            "changecase" => {
                self.checkpoint();
                let (a, b) = self.sel_rect();
                let mut n = 0usize;
                for r in a.row..=b.row {
                    for c in a.col..=b.col {
                        let p = Pos::new(r, c);
                        let Some(cell) = self.sheet().get(p).cloned() else { continue };
                        if cell.formula.is_some() {
                            continue; // 式の結果は触らない(次の計算で戻ってしまう)
                        }
                        let sheet::Value::Text(t) = &cell.value else { continue };
                        let new_t = change_case(t, v);
                        if new_t != *t {
                            let mut cell = cell;
                            cell.value = sheet::Value::Text(new_t);
                            self.sheet_mut().set(p, cell);
                            n += 1;
                        }
                    }
                }
                if n == 0 {
                    self.undo_stack.pop();
                    self.status = ui::t!("選択の中に変わる文字がありません").into();
                } else {
                    self.dirty = true;
                    self.sync_input();
                    self.status = ui::tf!("{} セルの大文字小文字を変えました", n).into();
                }
            }
            "font-color" => {
                if let Some((_, hx)) = FONT_COLORS.iter().find(|(n, _)| *n == v) {
                    let c = hx.map(|h| h.to_string());
                    self.fmt(move |f| f.color = c.clone());
                    self.status = if hx.is_some() {
                        ui::tf!("文字の色を{}にしました", v).into()
                    } else {
                        ui::t!("文字の色を自動に戻しました").into()
                    };
                }
            }
            "fill-color" => {
                if let Some((_, hx)) = FILL_COLORS.iter().find(|(n, _)| *n == v) {
                    let c = hx.map(|h| h.to_string());
                    self.fmt(move |f| f.fill = c.clone());
                    self.status = if hx.is_some() {
                        ui::tf!("塗りを{}にしました", v).into()
                    } else {
                        ui::t!("塗りを消しました").into()
                    };
                }
            }
            "sheet-menu" => {
                self.sheet_menu_action(v);
                if self.pick.is_some() || self.prompt.is_some() {
                    return; // 2段目(色・改名・再表示)へ。pick_kind を戻さない
                }
            }
            "tab-color" => self.set_tab_color(v),
            "history" | "plugin" => {
                let plugin = self.pick_kind == "plugin";
                let hit = self.pick_paths.iter().find(|(n, _)| n == v).cloned();
                if let Some((_, path)) = hit {
                    if plugin {
                        match std::fs::read_to_string(&path) {
                            Ok(code) => self.run_python(code, cx),
                            Err(e) => self.status = ui::tf!("読めません: {}", e).into(),
                        }
                    } else {
                        self.open_version(&path);
                    }
                }
                self.pick_paths.clear();
            }
            _ => self.pick_value(v),
        }
        self.pick_kind = "value";
    }

    /// 一覧から選んだ値をセルに入れる(書式は据え置き)。
    pub(crate) fn pick_value(&mut self, v: &str) {
        self.checkpoint();
        let p = self.cursor;
        let fmt = self.sheet().get(p).map(|c| c.fmt.clone()).unwrap_or_default();
        let mut cell = Cell::input(v);
        cell.fmt = fmt;
        self.book.sheets[self.active].set(p, cell);
        recalc_book(&mut self.book, self.active);
        self.dirty = true;
        self.sync_input();
        self.status = ui::tf!("{} に入れました", p.a1()).into();
    }

    pub(crate) fn menu_action(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let menu_was_at = self.menu_at.take();
        self.menu_sub = None;
        match id {
            "cut" => self.a_cut(&ui::Cut, window, cx),
            "copy" => self.a_copy(&ui::Copy, window, cx),
            "paste" => self.a_paste(&ui::Paste, window, cx),
            "ps-values" => self.paste_special("values", cx),
            "ps-formulas" => self.paste_special("formulas", cx),
            "ps-formats" => self.paste_special("formats", cx),
            "ps-transpose" => self.paste_special("transpose", cx),
            // 消去。Euro-Office の「消去 ▸」に対応する3段
            "clear-all" => {
                self.commit();
                self.checkpoint();
                let (a, b) = self.sel_rect();
                let mut n = 0usize;
                for r in a.row..=b.row {
                    for c in a.col..=b.col {
                        n += self.book.sheets[self.active]
                            .cells
                            .remove(&Pos::new(r, c))
                            .is_some() as usize;
                    }
                }
                recalc_book(&mut self.book, self.active);
                self.dirty = true;
                self.sync_input();
                self.status = ui::tf!("{} セルを消去しました(中身も書式も)", n).into();
            }
            "clear-text" => {
                self.checkpoint();
                let n = self.clear_range();
                self.status = ui::tf!("{} セルの中身を消しました(書式は残る)", n).into();
            }
            "clear-fmt" => self.run_cmd("clear", cx),
            // コメントとハイパーリンクだけを消す(本家の消去は5択)
            "clear-comment" => {
                self.checkpoint();
                let (a, b) = self.sel_rect();
                let sh = &mut self.book.sheets[self.active];
                let before = sh.comments.len();
                sh.comments.retain(|p, _| {
                    p.row < a.row || p.row > b.row || p.col < a.col || p.col > b.col
                });
                let n = before - sh.comments.len();
                if n == 0 {
                    self.undo_stack.pop();
                    self.status = ui::t!("その範囲にコメントはありません").into();
                } else {
                    self.dirty = true;
                    self.status = ui::tf!("{} 個のコメントを消しました", n).into();
                }
            }
            "clear-link" => {
                self.checkpoint();
                let (a, b) = self.sel_rect();
                let sh = &mut self.book.sheets[self.active];
                let before = sh.links.len();
                sh.links.retain(|p, _| {
                    p.row < a.row || p.row > b.row || p.col < a.col || p.col > b.col
                });
                let n = before - sh.links.len();
                if n == 0 {
                    self.undo_stack.pop();
                    self.status = ui::t!("その範囲にハイパーリンクはありません").into();
                } else {
                    self.dirty = true;
                    self.status = ui::tf!("{} 個のハイパーリンクを消しました", n).into();
                }
            }
            "insrow" => {
                self.rowcol(|s, p| s.insert_row(p.row));
                self.status = ui::t!("行を挿しました(下の式の参照も直っています)").into();
            }
            "delrow" => {
                self.rowcol(|s, p| s.remove_row(p.row));
                self.status = ui::t!("行を削除しました").into();
            }
            "inscol" => {
                self.rowcol(|s, p| s.insert_col(p.col));
                self.status = ui::t!("列を挿しました").into();
            }
            "delcol" => {
                self.rowcol(|s, p| s.remove_col(p.col));
                self.status = ui::t!("列を削除しました").into();
            }
            "sort-asc" | "sort-desc" => self.sort_active(id == "sort-asc"),
            // 選んだ値で絞り込む = その列で「選んだ値以外」を隠す
            // (オートフィルタの1操作。▼で選び直せる)
            "filter-set" => {
                let p = self.cursor;
                let v = self.sheet().get(p).map(|c| c.value.display()).unwrap_or_default();
                if self.auto_filter.is_none() {
                    self.run_cmd("setfilter", cx);
                }
                if self.auto_filter.is_none() {
                    return; // 張れなかった(空の表)。理由は setfilter が言っている
                }
                let (vals, _) = self.filter_values(p.col);
                let hide: std::collections::BTreeSet<String> =
                    vals.into_iter().map(|(s, _)| s).filter(|s| *s != v).collect();
                let f = self.auto_filter.as_mut().unwrap();
                if hide.is_empty() {
                    f.hide.remove(&p.col);
                } else {
                    f.hide.insert(p.col, hide);
                }
                let label = if v.is_empty() { "(空白)".to_string() } else { v };
                self.status = ui::tf!("「{}」だけを表示しています(見出しの ▼ で選び直せます)", label).into();
            }
            "filter-clear" => self.run_cmd("clear-filter", cx),
            "numfmt-more" => self.run_cmd("format", cx),
            "reapply" => {
                // 値は動的に見ているので掛け直しは常に済んでいる — 数を言い直す
                if let Some((total, shown)) = self.filter_counts() {
                    self.status = ui::tf!("絞り込みを掛け直しました — {} 行中 {} 行を表示", total, shown).into();
                }
            }
            // セル単位のシフト(挿入・削除)。結合をまたぐときは断られる
            "inscell-right" | "inscell-down" | "delcell-left" | "delcell-up" => {
                self.commit();
                self.checkpoint();
                let (a, b) = self.sel_rect();
                let r = match id {
                    "inscell-right" => self.book.sheets[self.active].insert_cells(a, b, true),
                    "inscell-down" => self.book.sheets[self.active].insert_cells(a, b, false),
                    "delcell-left" => self.book.sheets[self.active].delete_cells(a, b, true),
                    _ => self.book.sheets[self.active].delete_cells(a, b, false),
                };
                match r {
                    Ok(n) => {
                        recalc_book(&mut self.book, self.active);
                        self.dirty = true;
                        self.anchor = None;
                        self.sync_input();
                        self.status = ui::tf!("{} セルをシフトしました(動いたセルへの参照も直っています)", n)
                        .into();
                    }
                    Err(e) => {
                        // 何も変えていないので、積んだ控えは戻す
                        self.undo_stack.pop();
                        self.status = e.into();
                    }
                }
            }
            "cond-neg" => {
                self.commit();
                self.checkpoint();
                let range = self.sel_rect();
                self.book.sheets[self.active].cond.push(sheet::model::CondRule {
                    range,
                    op: sheet::model::CondOp::Lt,
                    value: 0.0,
                    color: Some("C00000".into()),
                    fill: None,
                });
                self.dirty = true;
                self.status = ui::tf!("{}:{} — 0未満を赤字にしました", range.0.a1(), range.1.a1()).into();
            }
            "cond-gt" => {
                self.commit();
                self.prompt = Some(("cond-gt", Editor::new("")));
            }
            "cond-lt" => {
                self.commit();
                self.prompt = Some(("cond-lt", Editor::new("")));
            }
            "cond-clear" => {
                self.commit();
                self.checkpoint();
                let (a, b) = self.sel_rect();
                let before = self.book.sheets[self.active].cond.len();
                self.book.sheets[self.active].cond.retain(|r| {
                    let (ra, rb) = r.range;
                    // 選んだ範囲と重なる規則を消す
                    !(ra.row <= b.row && rb.row >= a.row && ra.col <= b.col && rb.col >= a.col)
                });
                let n = before - self.book.sheets[self.active].cond.len();
                self.dirty = true;
                self.status = ui::tf!("{} 本の条件を消しました", n).into();
            }
            "picklist" => self.open_pick_list(),
            "defname" => {
                self.commit();
                self.prompt = Some(("name", Editor::new("")));
            }
            "addcomment" => {
                self.commit();
                let cur = self.sheet().comments.get(&self.cursor).cloned().unwrap_or_default();
                self.prompt = Some(("comment", Editor::new(&cur)));
            }
            "hyperlink" => {
                self.commit();
                let cur = self.sheet().links.get(&self.cursor).cloned().unwrap_or_default();
                self.prompt = Some(("link", Editor::new(&cur)));
            }
            "fmtcells" => {
                // メニューの出ていた場所の近くに小窓を開く
                self.fmt_panel = Some(menu_was_at.unwrap_or((HEAD_W + 24.0, ROW_H + 24.0)));
            }
            "freeze" => self.run_cmd("freeze", cx),
            // 数値の書式・関数はリボンと同じ配線を通す
            "comma" | "currency" | "percents" | "digit-inc" | "digit-dec"
            | "sum" | "average" | "count" | "max" | "min" => self.run_cmd(id, cx),
            _ => {}
        }
        cx.notify();
    }

    /// 子メニューの中身 (id, 名前, 押せるか)。
    /// **並びと名前は Euro-Office に合わせ、未実装は灰色**(リボンと同じ方針)。
    pub(crate) fn menu_sub_entries(&self, sub: &str) -> Vec<(&'static str, &'static str, bool)> {
        match sub {
            "ins" => vec![
                ("inscell-right", "セルを右にシフト", true),
                ("inscell-down", "セルを下にシフト", true),
                ("insrow", "行全体", true),
                ("inscol", "列全体", true),
            ],
            "del" => vec![
                ("delcell-left", "セルを左にシフト", true),
                ("delcell-up", "セルを上にシフト", true),
                ("delrow", "行全体", true),
                ("delcol", "列全体", true),
            ],
            "clr" => vec![
                // 本家の消去は5択(すべて/テキスト/書式/コメント/ハイパーリンク)
                ("clear-all", "すべて", true),
                ("clear-text", "テキスト(書式は残す)", true),
                ("clear-fmt", "書式(中身は残す)", true),
                ("clear-comment", "コメント", !self.sheet().comments.is_empty()),
                ("clear-link", "ハイパーリンク", !self.sheet().links.is_empty()),
            ],
            "sort" => vec![
                ("sort-asc", "昇順", true),
                ("sort-desc", "降順", true),
            ],
            "filter" => vec![
                ("filter-set", "選択した値で絞り込む", true),
                ("filter-clear", "絞り込みを解く", self.auto_filter.is_some()),
            ],
            "pastesp" => vec![
                ("ps-values", "値だけ(Ctrl+Shift+V)", true),
                ("ps-formulas", "式をそのまま(ずらさない)", true),
                ("ps-formats", "書式だけ", self.clip_cells.is_some()),
                ("ps-transpose", "行と列を入れ替えて(値を)", true),
            ],
            "cond" => vec![
                ("cond-neg", "0未満を赤字にする", true),
                ("cond-gt", "値より大きいと薄緑の塗り…", true),
                ("cond-lt", "値より小さいと薄赤の塗り…", true),
                ("cond-clear", "この範囲の条件を消す", true),
            ],
            "numfmt" => vec![
                ("comma", "桁区切り(1,000)", true),
                ("currency", "通貨(¥)", true),
                ("percents", "パーセント(%)", true),
                ("digit-inc", "小数を増やす", true),
                ("digit-dec", "小数を減らす", true),
                ("numfmt-more", "その他の表示形式…", true),
            ],
            "func" => vec![
                ("sum", "SUM(合計)", true),
                ("average", "AVERAGE(平均)", true),
                ("count", "COUNT(個数)", true),
                ("max", "MAX(最大)", true),
                ("min", "MIN(最小)", true),
            ],
            _ => vec![],
        }
    }

    pub(crate) fn a_context_menu(&mut self, _: &ui::ContextMenu, _: &mut Window, cx: &mut Context<Self>) {
        // キーボードから: カーソルのセルのそば(セルが画面の外なら左上)に出す
        let (x, y) = self
            .cell_origin_px(self.cursor)
            .map(|(x, y)| (x + 16.0, y + 16.0))
            .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
        self.menu_at = Some((x, y));
        self.menu_sub = None;
        cx.notify();
    }

    /// 名前ボックスの Enter。番地(B12)・範囲(A1:C9)・定義済みの名前なら
    /// そこへ飛ぶ。知らない名前なら**いまの選択に名前を付ける**(Excel と同じ)
    pub(crate) fn commit_name_box(&mut self) {
        let Some(ed) = self.name_edit.take() else { return };
        let t = ed.text().trim().to_string();
        if t.is_empty() {
            return;
        }
        let up = t.to_uppercase();
        let jump = |this: &mut Self, a: Pos, b: Option<Pos>| {
            this.commit();
            this.cursor = b.unwrap_or(a);
            this.anchor = b.is_some().then_some(a);
            this.sync_input();
            this.follow();
        };
        if let Some((a, b)) = up.split_once(':') {
            if let (Some(pa), Some(pb)) = (Pos::parse(a), Pos::parse(b)) {
                jump(self, pa, Some(pb));
                self.status = ui::tf!("{} を選びました", up).into();
                return;
            }
        }
        if let Some(p) = Pos::parse(&up) {
            jump(self, p, None);
            self.status = ui::tf!("{} へ移動しました", p.a1()).into();
            return;
        }
        // 定義済みの名前ならそこへ
        if let Some((_, r)) = self
            .sheet()
            .names
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(&t))
            .cloned()
        {
            let up = r.to_uppercase();
            if let Some((a, b)) = up.split_once(':') {
                if let (Some(pa), Some(pb)) = (Pos::parse(a), Pos::parse(b)) {
                    jump(self, pa, Some(pb));
                    self.status = ui::tf!("名前「{}」({})を選びました", t, up).into();
                    return;
                }
            }
            if let Some(p) = Pos::parse(&up) {
                jump(self, p, None);
                self.status = ui::tf!("名前「{}」({})へ移動しました", t, up).into();
                return;
            }
        }
        // 新しい名前 = いまの選択に付ける
        let range = if self.anchor.is_some() {
            let (a, b) = self.sel_rect();
            format!("{}:{}", a.a1(), b.a1())
        } else {
            self.cursor.a1()
        };
        self.checkpoint();
        self.sheet_mut().names.push((t.clone(), range.clone()));
        self.dirty = true;
        self.status = ui::tf!("名前「{}」を {} に付けました(名前ボックスで呼べます)", t, range).into();
    }

    /// 式の直入力の支援。=を打っている間だけ:
    /// - 打ちかけの関数名(2字以上)には**補完の一覧**(セルの下。押すと入る)
    /// - 開いた括弧の中では、**いま打っている引数のヒント**を状態帯に
    pub(crate) fn formula_assist(&mut self) {
        let t = self.input.text().to_string();
        if !t.starts_with('=') {
            if self.pick_kind == "fn-complete" {
                self.pick = None;
            }
            return;
        }
        let cur = self.input.cursor().min(t.len());
        // --- 補完: カーソルの直前の識別子(英字はじまり・2字以上) ---
        let token: String = {
            let rev: String = t[..cur]
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '.')
                .collect();
            rev.chars().rev().collect()
        };
        let mut showed = false;
        if token.len() >= 2 && token.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
            let up = token.to_uppercase();
            let cands: Vec<String> = funcs::FUNCS
                .iter()
                .filter(|f| f.name.starts_with(&up) && f.name != up)
                .map(|f| f.name.to_string())
                .take(12)
                .collect();
            if !cands.is_empty() {
                if let Some((x, y)) = self.cell_origin_px(self.cursor) {
                    let h = self.row_px(self.cursor.row);
                    self.pick_kind = "fn-complete";
                    self.pick = Some((cands, (x, y + h)));
                    showed = true;
                }
            }
        }
        if !showed && self.pick_kind == "fn-complete" {
            self.pick = None;
        }
        // --- 引数のヒント: いちばん内側の閉じていない関数と、何番目の引数か ---
        let mut stack: Vec<(String, usize)> = Vec::new();
        let mut in_str = false;
        let mut ident = String::new();
        for ch in t[..cur].chars() {
            match ch {
                '"' => in_str = !in_str,
                _ if in_str => {}
                '(' => {
                    stack.push((ident.to_uppercase(), 0));
                    ident.clear();
                }
                ')' => {
                    stack.pop();
                    ident.clear();
                }
                ',' => {
                    if let Some((_, n)) = stack.last_mut() {
                        *n += 1;
                    }
                    ident.clear();
                }
                c if c.is_ascii_alphanumeric() || c == '.' => ident.push(c),
                _ => ident.clear(),
            }
        }
        if let Some((name, argi)) = stack.last() {
            if let Some(f) = funcs::FUNCS.iter().find(|f| f.name == name) {
                let hint = f
                    .arg_desc
                    .get(*argi)
                    .or(f.arg_desc.last())
                    .copied()
                    .unwrap_or("");
                let names = parse_fn_args(f.args);
                let arg_name = names
                    .get(*argi)
                    .or(names.last())
                    .map(|(n, _)| n.clone())
                    .unwrap_or_default();
                self.status =
                    format!("{}{} — {}{}", f.name, f.args, arg_name, hint).into();
            }
        }
    }

    /// 「関数を挿入」の次へ = 選んだ関数の**引数の画面**へ進む(本家の第2段)
    pub(crate) fn fn_next(&mut self) {
        let Some(d) = self.fn_dlg.take() else { return };
        let list = fn_filtered(d.search.text(), d.group);
        let Some(f) = list.get(d.sel.min(list.len().saturating_sub(1))).copied() else {
            self.status = ui::t!("その条件の関数がありません").into();
            return;
        };
        let names = parse_fn_args(f.args);
        let eds = (0..names.len()).map(|_| Editor::new("")).collect();
        self.fn_args = Some(FnArgs {
            f,
            names,
            eds,
            focus: 0,
            result: String::new(),
            pick_from: None,
        });
        self.fn_args_recalc();
        self.status = ui::t!(
            "関数の引数: Tab で次の欄。セルをクリックすると参照が入ります。Enter で式に")
        .into();
    }

    /// 引数の画面の中身から式の文字を組む(埋めた欄まで)
    pub(crate) fn fn_args_formula(&self) -> Option<String> {
        let a = self.fn_args.as_ref()?;
        let vals: Vec<String> = a.eds.iter().map(|e| e.text().trim().to_string()).collect();
        let mut last = 0;
        for (i, v) in vals.iter().enumerate() {
            if !v.is_empty() {
                last = i + 1;
            }
        }
        Some(format!("{}({})", a.f.name, vals[..last].join(", ")))
    }

    /// 関数の結果の下見。**表の複製**の空きセルで計算する(ゴールシークと
    /// 同じ流儀 — 本物の表は触らない)
    pub(crate) fn fn_args_recalc(&mut self) {
        let Some(fstr) = self.fn_args_formula() else { return };
        let mut s = self.sheet().clone();
        let (rows, _) = s.extent();
        let p = Pos::new(rows + 2, 0);
        s.set(p, Cell::input(&format!("={fstr}")));
        recalc(&mut s);
        let out = s.get(p).map(|c| c.value.display()).unwrap_or_default();
        if let Some(a) = &mut self.fn_args {
            a.result = out;
        }
    }

    /// 引数の画面の OK。組んだ式をセルへ(編集中ならカーソルに差し込み)
    pub(crate) fn fn_args_ok(&mut self) {
        let Some(fstr) = self.fn_args_formula() else {
            self.fn_args = None;
            return;
        };
        self.fn_args = None;
        if self.editing() || self.edit_armed {
            self.input.insert(&fstr);
        } else {
            self.input = Editor::new(&format!("={fstr}"));
            let end = self.input.text().len();
            self.input.move_to(end, false);
        }
        self.edit_armed = true;
        self.status = ui::t!("式を入れました(Enter で確定 / Esc で取消)").into();
    }

    /// F2 = このセルを編集(次の打鍵が**追記**になる。Excel と同じ)
    pub(crate) fn a_edit_cell(&mut self, _: &ui::EditCell, _: &mut Window, cx: &mut Context<Self>) {
        if self.prompt.is_some() || self.solver.is_some() {
            return;
        }
        self.edit_armed = true;
        self.input.move_to(self.input.text().len(), false);
        self.status = ui::t!("編集: そのまま打つと続きに入ります(Esc で取消)").into();
        cx.notify();
    }

    /// 入力規則の文言を、選択に掛かる規則へ書き込む。規則が無ければ
    /// 文言だけの規則(type なし)を作る。消すだけのとき(clear_only)は作らない。
    /// 何かに書けたら true
    fn dv_upsert(
        &mut self,
        clear_only: bool,
        f: impl Fn(&mut sheet::model::Validation),
    ) -> bool {
        let (a, b) = self.sel_rect();
        let overlap = |x: &sheet::model::Validation| {
            let (ra, rb) = x.range;
            ra.row <= b.row && rb.row >= a.row && ra.col <= b.col && rb.col >= a.col
        };
        let mut hit = false;
        for x in self.book.sheets[self.active].validations.iter_mut() {
            if overlap(x) {
                f(x);
                hit = true;
            }
        }
        if !hit && !clear_only {
            let mut v = sheet::model::Validation::list((a, b), String::new());
            v.kind = String::new(); // 文言だけの規則(何でも通る)
            f(&mut v);
            self.book.sheets[self.active].validations.push(v);
            hit = true;
        }
        if hit {
            self.dirty = true;
        }
        hit
    }

    /// ▼の板の開閉(見出しのボタンから。同じ列ならしまう)
    pub(crate) fn toggle_filter_panel(&mut self, col: u32) {
        match &self.filter_panel {
            Some((c, _)) if *c == col => self.filter_panel = None,
            _ => self.filter_panel = Some((col, Editor::new(""))),
        }
    }

    /// ▼の板: 値ひとつの入切。空になったらその列は素通しに戻す
    pub(crate) fn filter_toggle_value(&mut self, col: u32, v: &str) {
        let Some(f) = &mut self.auto_filter else { return };
        let set = f.hide.entry(col).or_default();
        if !set.remove(v) {
            set.insert(v.to_string());
        }
        if set.is_empty() {
            f.hide.remove(&col);
        }
        self.filter_note();
    }

    /// ▼の板: (すべて選択)。全部見えていれば全部隠し、そうでなければ全部見せる
    pub(crate) fn filter_toggle_all(&mut self, col: u32, all: Vec<String>) {
        let Some(f) = &mut self.auto_filter else { return };
        if f.hide.remove(&col).is_none() {
            f.hide.insert(col, all.into_iter().collect());
        }
        self.filter_note();
    }

    /// ▼の板: この列の絞り込みを解く
    pub(crate) fn filter_clear_col(&mut self, col: u32) {
        if let Some(f) = &mut self.auto_filter {
            f.hide.remove(&col);
        }
        self.filter_note();
    }

    /// 絞り込みの操作のたびに、いま何行見えているかを状態行で言う
    fn filter_note(&mut self) {
        self.status = match self.filter_counts() {
            Some((total, shown)) => {
                ui::tf!("絞り込み中 — {} 行中 {} 行を表示(表示だけ。保存はされません)", total, shown).into()
            }
            None => ui::t!("絞り込みなし(全部見えています)").into(),
        };
    }

    pub(crate) fn a_cancel(&mut self, _: &ui::Cancel, _: &mut Window, cx: &mut Context<Self>) {
        if self.quit_ask {
            self.quit_ask = false;
            self.status = ui::t!("終了をやめました").into();
            cx.notify();
            return;
        }
        // 名前ボックス・関数の小窓は最優先で閉じる
        if self.name_edit.take().is_some()
            || self.fn_args.take().is_some()
            || self.fn_dlg.take().is_some()
        {
            cx.notify();
            return;
        }
        // 入力の板 → 一覧 → 子メニュー → 親メニュー → 書式の小窓 → コピーの破線、
        // の順で閉じる
        self.pivot_pend = None; // 聞き取り途中のピボット・小計は Esc でやめる
        self.sub_pend = None;
        self.dv_pend = None; // 入力規則の聞き取りも

        self.pw_pending = None; // パスワード待ちも Esc でやめる(開かない)
        if self.tool.take().is_some() {
            self.ink_cur = None;
            self.status = ui::t!("セルの操作に戻りました").into();
        }
        if self.filter_panel.take().is_some()
            || self.solver.take().is_some()
            || self.slicer.take().is_some()
            || self.prompt.take().is_some()
            || self.pick.take().is_some()
            || self.menu_sub.take().is_some()
            || self.menu_at.take().is_some()
            || self.fmt_panel.take().is_some()
            || self.clip_range.take().is_some()
            || self.shape_sel.take().is_some()
        {
            // 一覧・板を閉じたら意味づけも戻す(耳のメニューの狙い先も)
            self.pick_kind = "value";
            self.sheet_menu_at = None;
            cx.notify();
        } else if self.editing() {
            // 打ちかけを捨てて、セルの保存内容に戻す
            // (入力規則で堰き止められたときの逃げ道でもある)
            self.sync_input();
            self.status = ui::t!("打ちかけを取り消しました").into();
            cx.notify();
        } else if self.edit_armed {
            // F2 だけ押して何も打っていない — 編集をやめる
            self.edit_armed = false;
            cx.notify();
        }
    }

    /// 入力の板を確定する(Enter)。
    pub(crate) fn finish_prompt(&mut self, cx: &mut Context<Self>) {
        let Some((kind, ed)) = self.prompt.take() else { return };
        let text = ed.text().trim().to_string();
        match kind {
            // 入力規則の条件(整数/小数/文字数)。1〜100 / >=0 の形で受ける
            "dv-cond" => {
                let Some(kind) = self.dv_pend.take() else { return };
                if text.is_empty() {
                    self.status = ui::t!("入力規則をやめました").into();
                    return;
                }
                let core = text.replace(['~', '~'], "〜");
                let core = core.trim();
                let (op, f1, f2): (&str, String, String) =
                    if let Some(rest) = core.strip_suffix("以外") {
                        match rest.trim().split_once('〜') {
                            Some((x, y)) => ("notBetween", x.trim().into(), y.trim().into()),
                            None => ("", String::new(), String::new()),
                        }
                    } else if let Some((x, y)) = core.split_once('〜') {
                        ("between", x.trim().into(), y.trim().into())
                    } else if let Some(r) = core.strip_prefix(">=") {
                        ("greaterThanOrEqual", r.trim().into(), String::new())
                    } else if let Some(r) = core.strip_prefix("<=") {
                        ("lessThanOrEqual", r.trim().into(), String::new())
                    } else if let Some(r) = core.strip_prefix("<>") {
                        ("notEqual", r.trim().into(), String::new())
                    } else if let Some(r) = core.strip_prefix('>') {
                        ("greaterThan", r.trim().into(), String::new())
                    } else if let Some(r) = core.strip_prefix('<') {
                        ("lessThan", r.trim().into(), String::new())
                    } else if let Some(r) = core.strip_prefix('=') {
                        ("equal", r.trim().into(), String::new())
                    } else {
                        ("equal", core.into(), String::new())
                    };
                let ok = !op.is_empty()
                    && f1.parse::<f64>().is_ok()
                    && (f2.is_empty() || f2.parse::<f64>().is_ok());
                if !ok {
                    // 打ち直せるように板を開いたまま返す
                    self.dv_pend = Some(kind);
                    self.prompt = Some(("dv-cond", ed));
                    self.status = ui::t!("条件が読めません。半角の数で 1〜100 / 1〜100 以外 / >=0 / <50 / <>0 / =8 の形に").into();
                    return;
                }
                let (a, b) = self.sel_rect();
                self.checkpoint();
                // 同じ場所の規則は置き換え(付いていた文言は引き継ぐ)
                let overlap = |x: &sheet::model::Validation| {
                    let (ra, rb) = x.range;
                    ra.row <= b.row && rb.row >= a.row && ra.col <= b.col && rb.col >= a.col
                };
                let mut input_msg = None;
                let mut error_msg = None;
                self.book.sheets[self.active].validations.retain(|x| {
                    if overlap(x) {
                        input_msg = input_msg.take().or_else(|| x.input_msg.clone());
                        error_msg = error_msg.take().or_else(|| x.error_msg.clone());
                        false
                    } else {
                        true
                    }
                });
                let v = sheet::model::Validation {
                    range: (a, b),
                    formula: f1,
                    kind: kind.into(),
                    op: op.into(),
                    formula2: f2,
                    input_msg,
                    error_msg,
                };
                let said = v.describe();
                self.book.sheets[self.active].validations.push(v);
                self.dirty = true;
                self.status = ui::tf!(
                    "入力規則「{}」を {}:{} に掛けました(空欄と式は通ります。保存で xlsx にも残ります)",
                    said, a.a1(), b.a1()
                )
                .into();
            }
            // 入力メッセージ(題: 本文)。空 Enter = 消す
            "dv-msg" => {
                let msg = if text.is_empty() {
                    None
                } else {
                    Some(match text.split_once([':', ':']) {
                        Some((t, m)) => (t.trim().to_string(), m.trim().to_string()),
                        None => (String::new(), text.clone()),
                    })
                };
                self.checkpoint();
                let set = msg.clone();
                let changed = self.dv_upsert(text.is_empty(), move |x| x.input_msg = set.clone());
                self.status = match (changed, msg.is_some()) {
                    (true, true) => ui::t!("入力メッセージを付けました(セルに乗ると下の状態行に出ます)").into(),
                    (true, false) => ui::t!("入力メッセージを消しました").into(),
                    (false, _) => {
                        self.undo_stack.pop();
                        ui::t!("この範囲に入力規則はありません").into()
                    }
                };
            }
            // エラーの文言。頭に「警告:」「情報:」を付けると通して言うだけになる
            "dv-err" => {
                let msg = if text.is_empty() {
                    None
                } else if let Some(m) = text.strip_prefix("警告:").or_else(|| text.strip_prefix("警告:")) {
                    Some(("warning".to_string(), String::new(), m.trim().to_string()))
                } else if let Some(m) = text.strip_prefix("情報:").or_else(|| text.strip_prefix("情報:")) {
                    Some(("information".to_string(), String::new(), m.trim().to_string()))
                } else {
                    Some(("stop".to_string(), String::new(), text.clone()))
                };
                self.checkpoint();
                let set = msg.clone();
                let changed = self.dv_upsert(text.is_empty(), move |x| x.error_msg = set.clone());
                self.status = match (changed, msg.as_ref()) {
                    (true, Some((s, _, _))) if s == "stop" => {
                        ui::t!("エラーの文言を付けました(合わない入力は堰き止めます)").into()
                    }
                    (true, Some(_)) => {
                        ui::t!("エラーの文言を付けました(合わない入力も通して、言うだけにします)").into()
                    }
                    (true, None) => ui::t!("エラーの文言を消しました").into(),
                    (false, _) => {
                        self.undo_stack.pop();
                        ui::t!("この範囲に入力規則はありません").into()
                    }
                };
            }
            // カスタムの数値書式(xlsx のコードをそのまま)。空 Enter = 一般に戻す
            "numfmt-custom" => {
                if text.is_empty() {
                    self.fmt(|f| f.number_format = None);
                    self.status = ui::t!("数値の書式を「一般」に戻しました").into();
                } else {
                    let code = text.clone();
                    self.fmt(move |f| f.number_format = Some(code.clone()));
                    self.status = ui::tf!(
                        "数値の書式コードを「{}」にしました(描けない書き方は素の数で出ます。保存で xlsx にも残ります)",
                        text
                    )
                    .into();
                }
            }
            // 並べ替えの基準(複数可)。「見出し名か列の字 [昇順|降順]」を
            // カンマ区切りで。向きを省けば昇順
            "sort-by" => {
                if text.is_empty() {
                    self.status = ui::t!("並べ替えをやめました").into();
                    return;
                }
                let (_, cols) = self.sheet().extent();
                let heads: Vec<String> = (0..cols)
                    .map(|c| {
                        self.sheet()
                            .get(Pos::new(0, c))
                            .map(|x| x.value.display())
                            .unwrap_or_default()
                    })
                    .collect();
                let mut keys: Vec<(u32, bool)> = Vec::new();
                let mut names: Vec<String> = Vec::new();
                for raw in text.split([',', '、']) {
                    let t = raw.trim();
                    if t.is_empty() {
                        continue;
                    }
                    let low = t.to_lowercase();
                    let (name, asc) = if let Some(n) = t.strip_suffix("降順") {
                        (n.trim(), false)
                    } else if let Some(n) = t.strip_suffix("昇順") {
                        (n.trim(), true)
                    } else if low.ends_with("desc") {
                        (t[..t.len() - 4].trim_end(), false)
                    } else if low.ends_with("asc") {
                        (t[..t.len() - 3].trim_end(), true)
                    } else {
                        (t, true)
                    };
                    let col = heads
                        .iter()
                        .position(|h| h == name)
                        .map(|i| i as u32)
                        .or_else(|| {
                            // 列の字(A・B・AA…)でも指せる
                            if !name.is_empty()
                                && name.chars().all(|c| c.is_ascii_alphabetic())
                            {
                                Pos::parse(&format!("{}1", name.to_uppercase())).map(|p| p.col)
                            } else {
                                None
                            }
                        });
                    let Some(col) = col else {
                        // 打ち直せるように板を開いたまま返す
                        self.prompt = Some(("sort-by", ed));
                        self.status = ui::tf!(
                            "「{}」という見出しが見つかりません。使える見出し: {}",
                            name,
                            heads.iter().filter(|h| !h.is_empty()).cloned()
                                .collect::<Vec<_>>().join(" / ")
                        )
                        .into();
                        return;
                    };
                    keys.push((col, asc));
                    names.push(format!(
                        "{} {}",
                        if heads.get(col as usize).map(|h| !h.is_empty()).unwrap_or(false) {
                            heads[col as usize].clone()
                        } else {
                            Pos::new(0, col).a1().trim_end_matches('1').to_string()
                        },
                        if asc { "昇順" } else { "降順" }
                    ));
                }
                if keys.is_empty() {
                    self.status = ui::t!("並べ替えをやめました").into();
                    return;
                }
                self.checkpoint();
                self.book.sheets[self.active].sort_by_columns(&keys, true);
                recalc_book(&mut self.book, self.active);
                self.dirty = true;
                self.sync_input();
                self.status = ui::tf!(
                    "並べ替えました: {}(見出しは据え置き。Ctrl+Z で1手)",
                    names.join(" → ")
                )
                .into();
            }
            "sheet-rename" => {
                let Some(t) = self.sheet_menu_at.take() else { return };
                if t >= self.book.sheets.len() {
                    return;
                }
                let old = self.book.sheets[t].name.clone();
                if text.is_empty() || text == old {
                    self.status = ui::t!("名前は変えませんでした").into();
                    return;
                }
                // xlsx のシート名の決まり: 31字まで・: \\ / ? * [ ] は使えない
                if text.chars().count() > 31
                    || text.contains([':', '\\', '/', '?', '*', '[', ']'])
                {
                    self.status = ui::tf!("「{}」はシート名にできません(31字まで。: \\ / ? * [ ] は不可)", text)
                    .into();
                    return;
                }
                if self.book.sheets.iter().enumerate().any(|(i, s)| i != t && s.name == text) {
                    self.status = ui::tf!("「{}」は既にあります", text).into();
                    return;
                }
                self.checkpoint_book(); // 名前と式の書き換えを1手で戻せる
                let n = rename_sheet_refs(&mut self.book, &old, &text);
                self.book.sheets[t].name = text.clone();
                recalc_book(&mut self.book, t);
                self.dirty = true;
                self.status = if n > 0 {
                    ui::tf!("「{}」を「{}」にしました(式の参照 {} 箇所も追随)", old, text, n)
                        .into()
                } else {
                    ui::tf!("「{}」を「{}」にしました", old, text).into()
                };
            }
            "name" => {
                if text.is_empty() {
                    self.status = ui::t!("名前を付けませんでした").into();
                    return;
                }
                let ok = text.chars().all(|c| c.is_alphanumeric() || c == '_')
                    && !text.chars().next().unwrap().is_ascii_digit()
                    && Pos::parse(&text).is_none();
                if !ok {
                    self.status = ui::tf!("「{}」は名前にできません(文字と数字と _。セル参照の形は不可)", text)
                    .into();
                    return;
                }
                let (a, b) = self.sel_rect();
                let range = if self.anchor.is_some() {
                    format!("{}:{}", a.a1(), b.a1())
                } else {
                    a.a1()
                };
                let s = &mut self.book.sheets[self.active];
                s.names.retain(|(n, _)| *n != text);
                s.names.push((text.clone(), range.clone()));
                recalc_book(&mut self.book, self.active);
                self.dirty = true;
                self.status = ui::tf!("名前「{}」= {}(式の中で使えます)", text, range).into();
            }
            "comment" => {
                let p = self.cursor;
                if text.is_empty() {
                    if self.book.sheets[self.active].comments.remove(&p).is_some() {
                        self.dirty = true;
                        self.status = ui::tf!("{} のコメントを消しました", p.a1()).into();
                    }
                } else {
                    self.book.sheets[self.active].comments.insert(p, text);
                    self.dirty = true;
                    self.status = ui::tf!("{} にコメントを付けました(保存で残ります)", p.a1()).into();
                }
            }
            "cond-gt" | "cond-lt" => {
                let Ok(value) = text.parse::<f64>() else {
                    self.status = ui::tf!("「{}」は数として読めません", text).into();
                    return;
                };
                self.checkpoint();
                let range = self.sel_rect();
                let gt = kind == "cond-gt";
                self.book.sheets[self.active].cond.push(sheet::model::CondRule {
                    range,
                    op: if gt { sheet::model::CondOp::Gt } else { sheet::model::CondOp::Lt },
                    value,
                    color: None,
                    fill: Some(if gt { "E2EFDA".into() } else { "FCE4D6".into() }),
                });
                self.dirty = true;
                self.status = ui::tf!("{}:{} — {} より{}を塗ります", range.0.a1(), range.1.a1(), value, if gt { "大きい値" } else { "小さい値" }).into();
            }
            "py" => {
                let t = text.trim().to_string();
                if t.is_empty() {
                    // 空 Enter = .py ファイルを選ぶ
                    self.run_python_file_dialog(cx);
                } else if t == "@計算" || t == "@calc" {
                    self.run_py_calc(cx);
                } else if t == "@" || t == "@list" {
                    let names: Vec<&str> =
                        self.book.scripts.iter().map(|(n, _)| n.as_str()).collect();
                    self.status = if names.is_empty() {
                        ui::t!("ブックに載せた Python はありません(@save 名前 で載せる)").into()
                    } else {
                        ui::tf!("ブックの Python: {}(@名前 で実行)", names.join(" / ")).into()
                    };
                } else if let Some(name) = t.strip_prefix("@save ") {
                    let name = name.trim().to_string();
                    if name.is_empty() {
                        self.status = ui::t!("@save 名前 の形で").into();
                    } else {
                        self.store_python_dialog(name, cx);
                    }
                } else if let Some(name) = t.strip_prefix("@del ") {
                    let name = name.trim();
                    let before = self.book.scripts.len();
                    self.book.scripts.retain(|(n, _)| n != name);
                    if self.book.scripts.len() < before {
                        self.dirty = true;
                        self.status = ui::tf!("「{}」をブックから外しました", name).into();
                    } else {
                        self.status = ui::tf!("「{}」はありません", name).into();
                    }
                } else if let Some(rest) = t.strip_prefix('@') {
                    // ブックに載ったコード = 出所が自分とは限らない。必ず檻の中。
                    // 網は既定で閉じる。「@名前 net」と**その場で打ったときだけ**開く
                    // (許可はブックに保存されない — 毎回が明示の同意)
                    let (name, net) = match rest.trim().strip_suffix(" net") {
                        Some(n) => (n.trim(), true),
                        None => (rest.trim(), false),
                    };
                    match self.book.scripts.iter().find(|(n, _)| n == name) {
                        Some((_, code)) => {
                            let code = code.clone();
                            if net {
                                self.status =
                                    ui::t!("網あり檻で実行します(ファイルは守られたまま)").into();
                            }
                            self.run_python_inner(code, true, net, cx);
                        }
                        None => {
                            self.status =
                                ui::tf!("「{}」はありません(@list で一覧)", name).into();
                        }
                    }
                } else {
                    self.run_python(t, cx);
                }
            }
            "shape-text" => {
                let Some(i) = self.shape_sel else { return };
                if self.sheet().shapes_new.len() <= i {
                    return;
                }
                self.checkpoint();
                self.sheet_mut().shapes_new[i].text =
                    (!text.is_empty()).then(|| text.clone());
                self.dirty = true;
                self.status = if text.is_empty() {
                    ui::t!("文字を消しました").into()
                } else {
                    ui::t!("図形に文字を入れました(保存で xlsx に入ります)").into()
                };
            }
            "split-delim" => {
                let delim = if text.is_empty() { ",".to_string() } else { text };
                let (a, b) = self.sel_rect();
                let col = a.col;
                let targets: Vec<(Pos, String)> = (a.row..=b.row)
                    .filter_map(|r| {
                        let p = Pos::new(r, col);
                        match self.sheet().get(p).map(|c| &c.value) {
                            Some(sheet::Value::Text(t)) if t.contains(&delim) => {
                                Some((p, t.clone()))
                            }
                            _ => None,
                        }
                    })
                    .collect();
                if targets.is_empty() {
                    self.status = ui::tf!("「{}」で割れるセルが選択にありません", delim).into();
                    return;
                }
                self.checkpoint();
                let mut n = 0usize;
                for (p, t) in targets {
                    for (k, part) in t.split(&delim).enumerate() {
                        let q = Pos::new(p.row, p.col + k as u32);
                        let fmt = self.sheet().get(q).map(|c| c.fmt.clone()).unwrap_or_default();
                        let mut cell = if part.starts_with('=') {
                            Cell {
                                formula: None,
                                value: sheet::Value::Text(part.to_string()),
                                fmt: Default::default(),
                            }
                        } else {
                            Cell::input(part)
                        };
                        cell.fmt = fmt;
                        self.sheet_mut().set(q, cell);
                        n += 1;
                    }
                }
                recalc_book(&mut self.book, self.active);
                self.dirty = true;
                self.sync_input();
                self.status =
                    ui::tf!("{} 欄に割りました(右のセルは上書き。Ctrl+Z で戻せます)", n).into();
            }
            "goal-target" => {
                // 「D6=765600」の形
                let Some((cell_s, val_s)) = text.split_once('=') else {
                    self.status = ui::t!("「セル=目標値」の形で(例: D6=800000)").into();
                    return;
                };
                let (Some(p), Ok(v)) = (Pos::parse(cell_s), val_s.trim().parse::<f64>()) else {
                    self.status = ui::t!("読めません(例: D6=800000)").into();
                    return;
                };
                self.goal = Some((p, v));
                self.prompt = Some(("goal-var", Editor::new("")));
            }
            "goal-var" => {
                let Some((target, goal)) = self.goal.take() else { return };
                let Some(var) = Pos::parse(&text) else {
                    self.status = ui::t!("変えるセルが読めません(例: B2)").into();
                    return;
                };
                self.goal_seek(target, goal, var);
            }
            // パスワードの板。開き待ちがあれば解いて開き、
            // 無ければ「次の保存から暗号化」を決める(空なら解除)
            "pw-open" => {
                let Some(p) = self.pw_pending.take() else { return };
                let bytes = match std::fs::read(&p) {
                    Ok(b) => b,
                    Err(e) => {
                        self.status = ui::tf!("開けません: {}", e).into();
                        return;
                    }
                };
                match ooxml::crypt::decrypt(&bytes, &text) {
                    Ok(plain) => {
                        self.open_plain(p.clone(), plain);
                        if self.path.as_deref() == Some(p.as_path()) {
                            self.encrypt_pw = Some(text);
                            self.status = ui::tf!("{}(保存も同じパスワードで暗号化します)", self.status)
                            .into();
                        }
                    }
                    Err(e) => {
                        // 板は開いたまま。打ち直せる
                        self.pw_pending = Some(p);
                        self.prompt = Some(("pw-open", Editor::new("")));
                        self.status = e.into();
                    }
                }
            }
            "pw-set" => {
                if text.is_empty() {
                    self.encrypt_pw = None;
                    self.status = ui::t!("暗号化しません(次の保存から普通の xlsx)").into();
                } else {
                    self.encrypt_pw = Some(text);
                    self.dirty = true;
                    self.status =
                        ui::t!("次の保存から、このパスワードで暗号化します(AES-128。Excel や LibreOffice でも開けます)").into();
                }
            }
            "equation" => {
                if text.is_empty() {
                    self.status = ui::t!("式が空です(何も置きませんでした)").into();
                } else {
                    self.insert_py_image(EQ_PY, "eq", text, cx);
                }
            }
            "textart" => {
                if text.is_empty() {
                    self.status = ui::t!("文字が空です(何も置きませんでした)").into();
                } else {
                    self.insert_py_image(TEXTART_PY, "textart", text, cx);
                }
            }
            // ブックの情報(保存で docProps/core.xml へ)
            "prop-creator" | "prop-title" | "prop-keywords" | "prop-subject"
            | "prop-desc" => {
                let f = match kind {
                    "prop-creator" => &mut self.book.props.creator,
                    "prop-title" => &mut self.book.props.title,
                    "prop-keywords" => &mut self.book.props.keywords,
                    "prop-subject" => &mut self.book.props.subject,
                    _ => &mut self.book.props.description,
                };
                *f = text;
                self.dirty = true;
                self.status =
                    ui::t!("ブックの情報を控えました(保存で xlsx に入ります)").into();
            }
            "table-resize" => {
                let p = self.cursor;
                let Some(i) = self.sheet().tables.iter().position(|t| t.contains(p)) else {
                    return;
                };
                let parse = |t: &str| -> Option<(Pos, Pos)> {
                    let (x, y) = t.split_once(':')?;
                    Some((Pos::parse(x.trim())?, Pos::parse(y.trim())?))
                };
                match parse(&text) {
                    None => {
                        self.status = ui::t!("範囲は A1:C9 の形で書いてください").into();
                        self.prompt = Some(("table-resize", Editor::new(&text)));
                    }
                    Some((a, b)) if b.row < a.row || b.col < a.col => {
                        self.status = ui::t!("左上と右下が逆です(A1:C9 の順で)").into();
                        self.prompt = Some(("table-resize", Editor::new(&text)));
                    }
                    Some((a, b)) => {
                        self.checkpoint();
                        {
                            let t = &mut self.book.sheets[self.active].tables[i];
                            t.a = a;
                            t.b = b;
                        }
                        self.dirty = true;
                        self.status = ui::tf!("表の範囲を {}:{} にしました(書式は掛け直しません — 表のデザインの釦でどうぞ)", a.a1(), b.a1())
                        .into();
                    }
                }
            }
            "ai-table" => {
                if text.is_empty() {
                    self.status = ui::t!("文章がありません(何もしていません)").into();
                } else {
                    self.ai_go(CalcAi::Table(text), cx);
                }
            }
            "ai-ask" => {
                if text.is_empty() {
                    self.status = ui::t!("用件がありません(何もしていません)").into();
                } else {
                    self.ai_go(CalcAi::Ask(text), cx);
                }
            }
            "chat" => {
                if text.is_empty() {
                    self.status = ui::t!("何も書き残しませんでした").into();
                } else if let Some(cp) = self.chat_path() {
                    let stamp = std::process::Command::new("date")
                        .arg("+%Y-%m-%d %H:%M")
                        .output()
                        .ok()
                        .filter(|o| o.status.success())
                        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                        .unwrap_or_default();
                    let line = format!("[{stamp}] {}: {text}\n", lock_identity());
                    use std::io::Write as _;
                    let r = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&cp)
                        .and_then(|mut f| f.write_all(line.as_bytes()));
                    self.status = match r {
                        Ok(_) => ui::tf!("書き残しました({})", cp.file_name().unwrap_or_default().to_string_lossy())
                        .into(),
                        Err(e) => ui::tf!("書けません: {}", e).into(),
                    };
                }
            }
            // 小計の聞き取り(区切りの見出し → 合計する見出し)
            "subtotal-by" => {
                let Some(mut pend) = self.sub_pend.take() else { return };
                let t = text.trim().to_string();
                if !pend.headers.iter().any(|h| *h == t) {
                    self.status =
                        ui::tf!("「{}」は見出しにありません: {}", t, pend.headers.join(" / "))
                            .into();
                    self.sub_pend = Some(pend);
                    self.prompt = Some(("subtotal-by", Editor::new(&text)));
                    return;
                }
                pend.rows_sel = vec![t];
                self.status =
                    ui::t!("合計する見出し(カンマ区切り可。空 Enter = 数の列全部)").into();
                self.sub_pend = Some(pend);
                self.prompt = Some(("subtotal-vals", Editor::new("")));
            }
            "subtotal-vals" => {
                let Some(pend) = self.sub_pend.take() else { return };
                let by_off =
                    pend.headers.iter().position(|h| *h == pend.rows_sel[0]).unwrap_or(0);
                let by = pend.a.col + by_off as u32;
                let sel = split_fields(&text);
                let mut vals: Vec<u32> = Vec::new();
                if sel.is_empty() {
                    // 数の列を自動で拾う(基準の列は除く)
                    let sh = self.sheet();
                    for i in 0..pend.headers.len() {
                        let c = pend.a.col + i as u32;
                        if c == by {
                            continue;
                        }
                        let numeric = (pend.a.row + 1..=pend.b.row).any(|r| {
                            matches!(
                                sh.get(Pos::new(r, c)).map(|x| &x.value),
                                Some(Value::Number(_))
                            )
                        });
                        if numeric {
                            vals.push(c);
                        }
                    }
                    if vals.is_empty() {
                        self.status =
                            ui::t!("数の列が見つかりません(合計する見出しを書いてください)").into();
                        self.sub_pend = Some(pend);
                        self.prompt = Some(("subtotal-vals", Editor::new("")));
                        return;
                    }
                } else {
                    for name in &sel {
                        match pend.headers.iter().position(|h| h == name) {
                            Some(i) => vals.push(pend.a.col + i as u32),
                            None => {
                                self.status =
                                    ui::tf!("「{}」は見出しにありません", name).into();
                                self.sub_pend = Some(pend);
                                self.prompt = Some(("subtotal-vals", Editor::new(&text)));
                                return;
                            }
                        }
                    }
                }
                self.checkpoint();
                let n = apply_subtotals(
                    &mut self.book.sheets[self.active],
                    pend.a,
                    pend.b,
                    by,
                    &vals,
                );
                recalc_book(&mut self.book, self.active);
                self.dirty = true;
                self.sync_input();
                self.status = ui::tf!("{} 区切りに小計と総計を入れ、明細をグループ化しました — 「詳細の非表示」で畳むと合計だけ残ります(Ctrl+Z で1手)", n)
                .into();
            }
            // ピボットの聞き取り(行 → 列 → 値と集計)。間違いは板を出し直して言う
            "pivot-rows" => {
                let Some(mut pend) = self.pivot_pend.take() else { return };
                let sel = split_fields(&text);
                if sel.is_empty() {
                    self.status =
                        ui::tf!("行に並べる見出しを1つは選んでください: {}", pend.headers.join(" / ")).into();
                    self.pivot_pend = Some(pend);
                    self.prompt = Some(("pivot-rows", Editor::new("")));
                    return;
                }
                if let Some(bad) = sel.iter().find(|s| !pend.headers.contains(s)) {
                    self.status = ui::tf!("「{}」は見出しにありません: {}", bad, pend.headers.join(" / ")).into();
                    self.pivot_pend = Some(pend);
                    self.prompt = Some(("pivot-rows", Editor::new(&text)));
                    return;
                }
                pend.rows_sel = sel;
                let rest: Vec<&String> =
                    pend.headers.iter().filter(|h| !pend.rows_sel.contains(h)).collect();
                self.status = ui::tf!("列に広げる見出し(空 Enter = なし): {}", rest.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" / ")).into();
                self.pivot_pend = Some(pend);
                self.prompt = Some(("pivot-cols", Editor::new("")));
            }
            "pivot-cols" => {
                let Some(mut pend) = self.pivot_pend.take() else { return };
                let sel = split_fields(&text);
                if let Some(bad) = sel.iter().find(|s| !pend.headers.contains(s)) {
                    self.status = ui::tf!("「{}」は見出しにありません: {}", bad, pend.headers.join(" / ")).into();
                    self.pivot_pend = Some(pend);
                    self.prompt = Some(("pivot-cols", Editor::new(&text)));
                    return;
                }
                pend.cols_sel = sel;
                self.status = ui::t!("値にする見出しと集計(例: 金額 合計。合計/平均/個数/最大/最小)").into();
                self.pivot_pend = Some(pend);
                self.prompt = Some(("pivot-val", Editor::new("")));
            }
            "pivot-val" => {
                let Some(pend) = self.pivot_pend.take() else { return };
                match parse_pivot_val(&text, &pend.headers) {
                    Ok((value, agg)) => self.insert_pivot(pend, value, agg, cx),
                    Err(e) => {
                        self.status = e.into();
                        self.pivot_pend = Some(pend);
                        self.prompt = Some(("pivot-val", Editor::new(&text)));
                    }
                }
            }
            "find" => {
                if text.is_empty() {
                    self.status = ui::t!("探す言葉を入れてください").into();
                    return;
                }
                self.find_term = Some(text);
                self.prompt = Some(("replace-with", Editor::new("")));
            }
            "replace-with" => {
                let Some(find) = self.find_term.take() else { return };
                if text.is_empty() {
                    // 検索だけ
                    self.find_next(&find);
                    return;
                }
                // 全て置き換え(シート全体。式の中も)
                let targets: Vec<(Pos, String)> = self
                    .sheet()
                    .cells
                    .iter()
                    .filter(|(_, c)| c.editable().contains(&find))
                    .map(|(p, c)| (*p, c.editable()))
                    .collect();
                if targets.is_empty() {
                    self.status = ui::tf!("「{}」は見つかりません", find).into();
                    self.find_term = Some(find);
                    return;
                }
                self.checkpoint();
                let mut n = 0usize;
                for (p, src) in targets {
                    n += src.matches(find.as_str()).count();
                    let dst = src.replace(find.as_str(), &text);
                    let fmt = self.sheet().get(p).map(|c| c.fmt.clone()).unwrap_or_default();
                    let mut cell = Cell::input(&dst);
                    cell.fmt = fmt;
                    self.sheet_mut().set(p, cell);
                }
                recalc_book(&mut self.book, self.active);
                self.dirty = true;
                self.sync_input();
                self.find_term = Some(find.clone());
                self.status =
                    ui::tf!("「{}」→「{}」: {} カ所を置き換えました(Ctrl+Z で戻せます)", find, text, n)
                        .into();
            }
            "validation" => {
                let (a, b) = self.sel_rect();
                let overlap = |v: &sheet::model::Validation| {
                    let (ra, rb) = v.range;
                    ra.row <= b.row && rb.row >= a.row && ra.col <= b.col && rb.col >= a.col
                };
                if text.is_empty() {
                    // 空で Enter = この範囲の規則を外す
                    let n = self.sheet().validations.iter().filter(|v| overlap(v)).count();
                    if n == 0 {
                        self.status = ui::t!("この範囲に入力規則はありません").into();
                        return;
                    }
                    self.checkpoint();
                    self.book.sheets[self.active].validations.retain(|v| !overlap(v));
                    self.dirty = true;
                    self.status = ui::tf!("{} 本の入力規則を外しました", n).into();
                    return;
                }
                // = 始まりは範囲の参照、それ以外は候補の直書き(, 区切り)
                let formula = match text.strip_prefix('=') {
                    Some(r) => r.trim().to_string(),
                    None => format!("\"{}\"", text.replace('"', "")),
                };
                let v = sheet::model::Validation::list((a, b), formula);
                let opts = v.options(self.sheet());
                if opts.is_empty() {
                    // 読めない規則を作らない(できないものを、できるように見せない)
                    self.status =
                        ui::t!("候補が読めません(例: 甲,乙,丙 または =D2:D5)").into();
                    return;
                }
                self.checkpoint();
                // 選択に重なる古い規則は入れ替える(重ね掛けは分かりにくい)
                self.book.sheets[self.active].validations.retain(|v| !overlap(v));
                self.book.sheets[self.active].validations.push(v);
                self.dirty = true;
                self.status = format!(
                    "{}:{} に入力規則を付けました(候補: {})",
                    a.a1(),
                    b.a1(),
                    opts.join(" / ")
                )
                .into();
            }
            "link" => {
                let p = self.cursor;
                if text.is_empty() {
                    if self.book.sheets[self.active].links.remove(&p).is_some() {
                        self.dirty = true;
                        self.status = format!("{} のリンクを外しました", p.a1()).into();
                    }
                } else {
                    self.book.sheets[self.active].links.insert(p, text);
                    self.dirty = true;
                    self.status =
                        format!("{} にリンクを付けました(Ctrl+クリックで開く)", p.a1()).into();
                }
            }
            _ => {}
        }
    }

    /// 選んだ範囲の**外周だけ**に罫線(帳票の枠)。
    pub(crate) fn border_outline(&mut self) {
        self.commit();
        self.checkpoint();
        let (a, b) = self.sel_rect();
        for r in a.row..=b.row {
            for c in a.col..=b.col {
                let p = Pos::new(r, c);
                let mut cell = self.sheet().get(p).cloned().unwrap_or_default();
                if r == a.row { cell.fmt.borders.top = true }
                if r == b.row { cell.fmt.borders.bottom = true }
                if c == a.col { cell.fmt.borders.left = true }
                if c == b.col { cell.fmt.borders.right = true }
                self.book.sheets[self.active].set(p, cell);
            }
        }
        self.dirty = true;
        self.status = ui::t!("外枠を引きました").into();
    }

    /// 書式の小窓の釦。
    pub(crate) fn fmt_panel_action(&mut self, id: &str, cx: &mut Context<Self>) {
        match id {
            "close" => self.fmt_panel = None,
            "b-all" => {
                self.fmt(|f| f.borders = Borders::ALL);
                self.status = ui::t!("格子の罫線を引きました").into();
            }
            "b-out" => self.border_outline(),
            "b-none" => {
                self.fmt(|f| f.borders = Borders::NONE);
                self.status = ui::t!("罫線を消しました").into();
            }
            "numfmt-none" => {
                self.fmt(|f| f.number_format = None);
                self.status = ui::t!("表示形式を戻しました").into();
            }
            id if id.starts_with("fill-") => {
                let v = id.trim_start_matches("fill-").to_string();
                if v == "none" {
                    self.fmt(|f| f.fill = None);
                } else {
                    self.fmt(move |f| f.fill = Some(v.clone()));
                }
            }
            id if id.starts_with("color-") => {
                let v = id.trim_start_matches("color-").to_string();
                if v == "none" {
                    self.fmt(|f| f.color = None);
                } else {
                    self.fmt(move |f| f.color = Some(v.clone()));
                }
            }
            other => self.run_cmd(other, cx),
        }
    }

    /// 「ドロップダウンリストから選択」。同じ列に既にある値の一覧を出す
    /// (Excel の Alt+↓ と同じ発想。入力規則が無くても、列の値は候補になる)。
    pub(crate) fn open_pick_list(&mut self) {
        // 入力規則があればその候補(規則に書かれた順のまま)。無ければ同じ列の値
        let from_rule = self
            .sheet()
            .validation_at(self.cursor)
            .map(|v| v.options(self.sheet()))
            .filter(|o| !o.is_empty());
        let mut vals: Vec<String> = from_rule.clone().unwrap_or_default();
        if vals.is_empty() {
            let col = self.cursor.col;
            let (rows, _) = self.sheet().extent();
            for r in 0..rows {
                if r == self.cursor.row {
                    continue;
                }
                if let Some(c) = self.sheet().get(Pos::new(r, col)) {
                    // 式の結果ではなく「打つもの」を候補にする(文字の値だけ)
                    if c.formula.is_none() {
                        let v = c.value.display();
                        if !v.is_empty() && !vals.contains(&v) {
                            vals.push(v);
                        }
                    }
                }
            }
            if vals.is_empty() {
                self.status = ui::t!("この列にはまだ値がありません").into();
                return;
            }
            vals.sort();
        }
        let total = vals.len();
        vals.truncate(16);
        if total > 16 {
            // 切ったことを黙らない
            self.status = format!("候補 {total} 件のうち先頭 16 件を出しています").into();
        }
        let at = self
            .cell_origin_px(self.cursor)
            .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
            .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
        self.pick = Some((vals, at));
    }

    /// シートを切り替える。いまの編集を確定し、場所はシートごとに覚えている。
    /// 絞り込みは解く(別のシートの列で絞ったままは意味を持たない)。
    pub(crate) fn switch_sheet(&mut self, i: usize) {
        if i >= self.book.sheets.len() || i == self.active {
            return;
        }
        if !self.commit() {
            return; // 入力規則で戻された。切り替えると打った文字が消える
        }
        self.remember_ui();
        self.active = i;
        self.restore_ui();
        self.anchor = None;
        self.auto_filter = None;
        self.filter_panel = None;
        self.sync_input();
        self.status = format!("シート「{}」", self.sheet().name).into();
    }

    /// シートを1枚足して、そこへ移る。
    pub(crate) fn add_sheet(&mut self) {
        let name = unique_sheet_name(&self.book);
        self.book.sheets.push(sheet::Sheet::new(&name));
        self.dirty = true;
        self.switch_sheet(self.book.sheets.len() - 1);
    }

    /// 耳の右クリックメニュー(本家「シートの管理」の並び)。
    /// 出す場所は耳に近い左下 — 板を遠くに出さない(終了確認と同じ判断)
    pub(crate) fn open_sheet_menu(&mut self, i: usize) {
        self.sheet_menu_at = Some(i);
        self.pick_kind = "sheet-menu";
        let y = (self.view_h_px - 420.0).max(ROW_H + 16.0);
        self.pick = Some((
            ["挿入", "削除", "名前の変更", "コピーを作成", "左へ移動",
             "右へ移動", "非表示", "再表示", "タブの色"]
                .iter()
                .map(|v| v.to_string())
                .collect(),
            (HEAD_W + 24.0, y),
        ));
    }

    /// シートの構成が変わった(挿入・削除・移動・複製)。**表の控えの束は
    /// シートの番号で結ばれている**ので、番号が振り直されると意味を失う —
    /// 黙って別のシートへ書き戻すより「元に戻せない」と言う(Excel と同じ)
    pub(crate) fn sheets_restructured(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.clip_range = None;
        self.dirty = true;
    }

    /// 耳のメニューの実行。t = メニューが指しているシート
    pub(crate) fn sheet_menu_action(&mut self, v: &str) {
        let Some(t) = self.sheet_menu_at else { return };
        if t >= self.book.sheets.len() {
            self.sheet_menu_at = None;
            return;
        }
        self.remember_ui(); // sheet_ui をシート数まで育てておく(挿し外しの前提)
        match v {
            "挿入" => {
                let name = unique_sheet_name(&self.book);
                self.book.sheets.insert(t + 1, sheet::Sheet::new(&name));
                self.sheet_ui.insert(t + 1, (Pos::new(0, 0), Pos::new(0, 0), None));
                for w in self.watch.iter_mut() {
                    if w.0 > t {
                        w.0 += 1;
                    }
                }
                self.sheets_restructured();
                self.active = t + 1;
                self.restore_ui();
                self.sync_input();
                self.status = ui::tf!("シート「{}」を挿しました", name).into();
            }
            "削除" => {
                if self.book.sheets.len() <= 1 {
                    self.status = ui::t!("最後の1枚は消せません").into();
                } else if self.book.sheets.iter().enumerate()
                    .filter(|(i, s)| *i != t && !s.hidden).count() == 0
                {
                    self.status = ui::t!("見えるシートが無くなるので消せません(先に別のシートを表示してください)").into();
                } else {
                    let name = self.book.sheets[t].name.clone();
                    self.book.sheets.remove(t);
                    self.sheet_ui.remove(t);
                    self.watch.retain(|w| w.0 != t);
                    for w in self.watch.iter_mut() {
                        if w.0 > t {
                            w.0 -= 1;
                        }
                    }
                    if self.active >= t && self.active > 0 {
                        self.active -= 1;
                    }
                    if self.book.sheets[self.active].hidden {
                        if let Some(i) = self.book.sheets.iter().position(|s| !s.hidden) {
                            self.active = i;
                        }
                    }
                    self.sheets_restructured();
                    self.restore_ui();
                    self.sync_input();
                    recalc_book(&mut self.book, self.active);
                    self.status =
                        ui::tf!("シート「{}」を削除しました(元に戻せない操作です)", name)
                            .into();
                }
            }
            "名前の変更" => {
                let cur = self.book.sheets[t].name.clone();
                self.prompt = Some(("sheet-rename", Editor::new(&cur)));
                return; // sheet_menu_at は板の確定まで持ち越す
            }
            "コピーを作成" => {
                let mut copy = self.book.sheets[t].clone();
                copy.name = copy_sheet_name(&self.book, &self.book.sheets[t].name);
                copy.hidden = false;
                let name = copy.name.clone();
                self.book.sheets.insert(t + 1, copy);
                self.sheet_ui.insert(t + 1, self.sheet_ui[t]);
                for w in self.watch.iter_mut() {
                    if w.0 > t {
                        w.0 += 1;
                    }
                }
                self.sheets_restructured();
                self.active = t + 1;
                self.restore_ui();
                self.sync_input();
                recalc_book(&mut self.book, self.active);
                self.status = ui::tf!("「{}」を作りました", name).into();
            }
            "左へ移動" | "右へ移動" => {
                let to = if v == "左へ移動" {
                    t.checked_sub(1)
                } else {
                    (t + 1 < self.book.sheets.len()).then_some(t + 1)
                };
                let Some(to) = to else {
                    self.status = ui::t!("その向きには動かせません(端です)").into();
                    self.sheet_menu_at = None;
                    return;
                };
                self.book.sheets.swap(t, to);
                self.sheet_ui.swap(t, to);
                for w in self.watch.iter_mut() {
                    w.0 = if w.0 == t { to } else if w.0 == to { t } else { w.0 };
                }
                if self.active == t {
                    self.active = to;
                } else if self.active == to {
                    self.active = t;
                }
                self.sheets_restructured();
                self.status = ui::tf!("シート「{}」を動かしました", self.book.sheets[to].name)
                    .into();
            }
            "非表示" => {
                if self.book.sheets.iter().enumerate()
                    .filter(|(i, s)| *i != t && !s.hidden).count() == 0
                {
                    self.status = ui::t!("最後の1枚は隠せません").into();
                } else {
                    self.book.sheets[t].hidden = true;
                    let name = self.book.sheets[t].name.clone();
                    if self.active == t {
                        if let Some(i) = self.book.sheets.iter().position(|s| !s.hidden) {
                            self.active = i;
                            self.restore_ui();
                            self.sync_input();
                        }
                    }
                    self.dirty = true;
                    self.status = ui::tf!(
                        "シート「{}」を隠しました(「再表示」で戻せます。保存で xlsx にも残ります)",
                        name
                    )
                    .into();
                }
            }
            "再表示" => {
                let hidden: Vec<(usize, String)> = self
                    .book
                    .sheets
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.hidden)
                    .map(|(i, s)| (i, s.name.clone()))
                    .collect();
                if hidden.is_empty() {
                    self.status = ui::t!("隠したシートはありません").into();
                } else {
                    self.pick_kind = "unhide";
                    self.pick_paths = hidden
                        .iter()
                        .map(|(i, n)| (n.clone(), PathBuf::from(i.to_string())))
                        .collect();
                    let y = (self.view_h_px - 420.0).max(ROW_H + 16.0);
                    self.pick = Some((
                        hidden.into_iter().map(|(_, n)| n).collect(),
                        (HEAD_W + 24.0, y),
                    ));
                    self.status = ui::t!("隠したシート: 選ぶと表示に戻します").into();
                    self.sheet_menu_at = None;
                    return; // 2段目の一覧へ(pick_kind を戻さない)
                }
            }
            "タブの色" => {
                self.pick_kind = "tab-color";
                let y = (self.view_h_px - 420.0).max(ROW_H + 16.0);
                self.pick = Some((
                    ["色なし", "赤", "橙", "黄", "緑", "青", "紫", "灰"]
                        .iter()
                        .map(|v| v.to_string())
                        .collect(),
                    (HEAD_W + 24.0, y),
                ));
                return; // sheet_menu_at は色の決定まで持ち越す
            }
            _ => {}
        }
        self.sheet_menu_at = None;
    }

    /// 耳の色の決定(タブの色の2段目)
    pub(crate) fn set_tab_color(&mut self, v: &str) {
        let Some(t) = self.sheet_menu_at.take() else { return };
        if t >= self.book.sheets.len() {
            return;
        }
        let hex = match v {
            "赤" => Some("FFC00000"),
            "橙" => Some("FFED7D31"),
            "黄" => Some("FFFFC000"),
            "緑" => Some("FF70AD47"),
            "青" => Some("FF4472C4"),
            "紫" => Some("FF7030A0"),
            "灰" => Some("FF7F7F7F"),
            _ => None,
        };
        // 1手で戻せる(耳の色もシートの中身 — checkpoint と同じ作法で番号つき)
        self.undo_stack.push(vec![(t, self.book.sheets[t].clone())]);
        self.redo_stack.clear();
        self.book.sheets[t].tab_color = hex.map(|h| h.to_string());
        self.dirty = true;
        self.status = if hex.is_some() {
            ui::tf!("耳の色を{}にしました(保存で xlsx にも残ります)", v).into()
        } else {
            ui::t!("耳の色を消しました").into()
        };
    }
}

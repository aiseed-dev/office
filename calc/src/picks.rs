//! main.rs からの純移動(2026-08-06 の分割)。挙動は変えない。

use crate::*;

impl Calc {
    /// 一覧から選んだものを適用する(pick_kind で意味が変わる)。
    pub(crate) fn apply_pick(&mut self, v: &str, cx: &mut Context<Self>) {
        // 「✓ 」は「今これが効いている」、「☑ /☐ 」は入切の印
        // (値そのものではないので、ここで剥がしてから照合する)
        let v = v.strip_prefix("✓ ").unwrap_or(v);
        let v = v.strip_prefix("☑ ").or_else(|| v.strip_prefix("☐ ")).unwrap_or(v);
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
                    "固定した枠に影を付ける" => {
                        self.freeze_shadow = !self.freeze_shadow;
                        self.status = if self.freeze_shadow {
                            ui::t!("固定した枠に影を付けます(固定中だけ見えます)").into()
                        } else {
                            ui::t!("固定した枠の影を消しました").into()
                        };
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
            // ピボットの聞き取り(クリックで入切 → 決定で次へ)。
            // 行 → 列 → 値 → 集計の4段。Esc でいつでもやめられる
            // 罫線: 辺の選択(ペンの線種・色で掛ける)
            // 名前マネージャー: 名前を選ぶ → 移動/打ち直し/削除
            "names-pick" => {
                if v.starts_with("→ 新しい名前") {
                    self.prompt = Some(("name", Editor::new("")));
                    return; // 板の確定まで
                }
                let name = v.split(" = ").next().unwrap_or(v).to_string();
                if v.ends_with("(テーブル)") {
                    // テーブル名は表オブジェクトの持ち物 — ここでは消さない
                    let hit = self
                        .sheet()
                        .tables
                        .iter()
                        .find(|t| t.name == name)
                        .map(|t| (t.a, t.b));
                    if let Some((a, b)) = hit {
                        self.anchor = Some(a);
                        self.cursor = b;
                        self.sync_input();
                        self.status = ui::t!("テーブルへ移動しました(名前の変更・削除は表のデザインで)").into();
                    }
                    return;
                }
                if self.sheet().names.iter().any(|(n, _)| *n == name) {
                    let at = self
                        .cell_origin_px(self.cursor)
                        .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                        .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                    self.name_pend = Some(name.clone());
                    self.pick_note = Some(ui::tf!("名前「{}」をどうしますか", name).into());
                    self.pick_kind = "name-act-pick";
                    self.pick = Some((
                        vec![
                            "そこへ移動".into(),
                            "中身を打ち直す…".into(),
                            "名前を消す".into(),
                        ],
                        at,
                    ));
                    return;
                }
            }
            "name-act-pick" => {
                let Some(name) = self.name_pend.take() else { return };
                let range = self
                    .sheet()
                    .names
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, r)| r.clone())
                    .unwrap_or_default();
                match v {
                    "そこへ移動" => {
                        let mut it = range.split(':');
                        let a = it.next().and_then(Pos::parse);
                        let b = it.next().and_then(Pos::parse);
                        if let Some(a) = a {
                            self.anchor = b.map(|_| a);
                            self.cursor = b.unwrap_or(a);
                            if b.is_some() {
                                self.anchor = Some(a);
                            }
                            self.sync_input();
                            self.status = ui::tf!("「{}」({})へ移動しました", name, range).into();
                        } else {
                            self.status = ui::tf!("「{}」の中身({})が場所として読めません", name, range).into();
                        }
                    }
                    "中身を打ち直す…" => {
                        self.name_pend = Some(name);
                        self.prompt = Some(("name-range", Editor::new(&range)));
                        return; // 板の確定まで
                    }
                    _ => {
                        // 名前を消す
                        self.checkpoint();
                        self.book.sheets[self.active].names.retain(|(n, _)| *n != name);
                        recalc_book(&mut self.book, self.active);
                        self.dirty = true;
                        self.status = ui::tf!("名前「{}」を消しました(式の中の {} は #NAME? になります)", name, name).into();
                    }
                }
            }
            // ヘッダー/フッター: 6つの区分から選んで板で打つ
            "hf-pick" => {
                if v == "全部消す" {
                    self.checkpoint();
                    self.sheet_mut().header = None;
                    self.sheet_mut().footer = None;
                    self.dirty = true;
                    self.status = ui::t!("ヘッダー/フッターを消しました").into();
                } else {
                    let name = v.split(':').next().unwrap_or(v).trim();
                    let (footer, slot) = match name {
                        "ヘッダー左" => (false, 0u8),
                        "ヘッダー中" => (false, 1),
                        "ヘッダー右" => (false, 2),
                        "フッター左" => (true, 0),
                        "フッター中" => (true, 1),
                        _ => (true, 2),
                    };
                    let raw = if footer {
                        self.sheet().footer.clone()
                    } else {
                        self.sheet().header.clone()
                    };
                    let (l, c, r) = sheet::model::hf_split(raw.as_deref().unwrap_or(""));
                    let cur = match slot { 0 => l, 1 => c, _ => r };
                    self.hf_pend = Some((footer, slot));
                    self.prompt = Some(("hf-edit", Editor::new(&cur)));
                    return; // 板の確定まで
                }
            }
            "border-pick" => {
                match v {
                    "→ 線のスタイル…" => {
                        let at = self
                            .cell_origin_px(self.cursor)
                            .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                            .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                        let items: Vec<String> = BORDER_STYLES
                            .iter()
                            .map(|(n, b)| {
                                if *b == self.pen_style {
                                    format!("✓ {n}")
                                } else {
                                    n.to_string()
                                }
                            })
                            .collect();
                        self.pick_note = Some(ui::t!("線のスタイル(選ぶとペンに入ります — 次の罫線から効く)").into());
                        self.pick_kind = "border-style-pick";
                        self.pick = Some((items, at));
                        return;
                    }
                    "→ 線の色…" => {
                        let at = self
                            .cell_origin_px(self.cursor)
                            .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                            .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                        let mut items: Vec<String> =
                            FONT_COLORS.iter().map(|(n, _)| n.to_string()).collect();
                        items.push("その他(RRGGBB を打つ)…".into());
                        self.pick_note = Some(ui::t!("線の色(選ぶとペンに入ります)").into());
                        self.pick_kind = "border-color-pick";
                        self.pick = Some((items, at));
                        return;
                    }
                    _ => {
                        self.apply_borders(v);
                        // 板は開いたまま — 外枠→内側…と連打で組み立てる
                        // (閉じるのは Esc か板の外。発注者報告 2026-08-08)
                        self.run_cmd("borders", cx);
                        return;
                    }
                }
            }
            "border-style-pick" => {
                if let Some((_, b)) = BORDER_STYLES.iter().find(|(n, _)| *n == v) {
                    self.pen_style = *b;
                    self.status = ui::tf!("線のスタイル: {}(罫線の一覧から掛けると効きます)", v).into();
                }
            }
            "border-color-pick" => {
                if v.starts_with("その他") {
                    self.prompt = Some(("border-color-rgb", Editor::new("")));
                    return; // 板の確定まで pick_kind を戻さない
                }
                if let Some((_, hx)) = FONT_COLORS.iter().find(|(n, _)| *n == v) {
                    self.pen_color =
                        hx.and_then(|h| u32::from_str_radix(h, 16).ok());
                    self.status = ui::tf!("線の色: {}(罫線の一覧から掛けると効きます)", v).into();
                }
            }
            "pivot-showas-pick" => {
                if let Some(i) = self.pivot_at(self.cursor) {
                    let sa = match v {
                        "比率" | "累計" | "差" => v.to_string(),
                        _ => String::new(), // そのまま
                    };
                    if let Some(d) = self.book.pivots.get_mut(i) {
                        d.show_as = sa.clone();
                        // 累計と差は積み上げなので、小計・総計を落とす
                        // (途中に総計が挟まると読み違えるため)
                        if sa == "累計" || sa == "差" {
                            d.totals = false;
                            d.subtotals = false;
                        }
                        let nd = d.clone();
                        self.spawn_pivot(nd, Some(i), cx);
                    }
                }
            }
            "pivot-style-pick" => {
                if let Some(i) = self.pivot_at(self.cursor) {
                    let style = match v {
                        "緑" | "橙" | "灰" => v.to_string(),
                        _ => String::new(), // 青(既定)
                    };
                    if let Some(d) = self.book.pivots.get_mut(i) {
                        d.style = style;
                        let nd = d.clone();
                        self.spawn_pivot(nd, Some(i), cx);
                    }
                }
            }
            "pivot-filter-pick" => {
                if v == "→ すべて表示に戻す" {
                    if let Some((pi, field, _)) = self.pivot_flt.take() {
                        if let Some(d) = self.book.pivots.get_mut(pi) {
                            d.hide.retain(|(f, _)| *f != field);
                            let nd = d.clone();
                            self.spawn_pivot(nd, Some(pi), cx);
                        }
                    }
                    return;
                }
                if v == "→ ラベルで絞る…" {
                    // 含む/で始まる/で終わる 語 — 合う値以外を hide に落とす
                    self.prompt = Some(("pivot-label", Editor::new("")));
                    return; // pivot_flt は板の確定まで持つ
                }
                if v == "→ 値で絞る…" {
                    let cur = self
                        .pivot_flt
                        .as_ref()
                        .and_then(|(pi, _, _)| self.book.pivots.get(*pi))
                        .and_then(|d| d.vfilter.as_ref())
                        .map(|(op, th)| format!("{op} {th}"))
                        .unwrap_or_default();
                    self.prompt = Some(("pivot-vfilter", Editor::new(&cur)));
                    return;
                }
                if v == "→ グループ化…" {
                    let at = self
                        .cell_origin_px(self.cursor)
                        .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                        .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                    let field = self.pivot_flt.as_ref().map(|(_, f, _)| f.clone()).unwrap_or_default();
                    self.pick_note =
                        Some(ui::tf!("「{}」のグループ化 — 単位を選ぶ", field).into());
                    self.pick_kind = "pivot-group-pick";
                    self.pick = Some((
                        vec![
                            "月".into(),
                            "四半期".into(),
                            "年".into(),
                            "数の幅…".into(),
                            "グループ解除".into(),
                        ],
                        at,
                    ));
                    return;
                }
                if v == "→ 決定(絞り込む)" {
                    let Some((pi, field, hidden)) = self.pivot_flt.take() else { return };
                    if let Some(d) = self.book.pivots.get_mut(pi) {
                        d.hide.retain(|(f, _)| *f != field);
                        if !hidden.is_empty() {
                            d.hide.push((field, hidden.into_iter().collect()));
                        }
                        let nd = d.clone();
                        self.spawn_pivot(nd, Some(pi), cx);
                    }
                    return;
                }
                if let Some((_, _, hidden)) = &mut self.pivot_flt {
                    let v = v.to_string();
                    if !hidden.remove(&v) {
                        hidden.insert(v);
                    }
                }
                self.pivot_filter_pick();
                return;
            }
            "cond-manage-pick" => {
                let Some(i) = v.split(')').next().and_then(|n| n.trim().parse::<usize>().ok())
                else { return };
                let i = i - 1;
                if i >= self.book.sheets[self.active].cond.len() { return }
                self.cond_pend = Some(i);
                let at = self
                    .cell_origin_px(self.cursor)
                    .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                    .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                self.pick_note = Some(ui::tf!("規則 {} をどうしますか", i + 1).into());
                self.pick_kind = "cond-act-pick";
                self.pick = Some((
                    vec!["そこへ移動".into(), "この規則を消す".into()],
                    at,
                ));
                return;
            }
            "cond-act-pick" => {
                let Some(i) = self.cond_pend.take() else { return };
                let Some(rule) = self.book.sheets[self.active].cond.get(i).cloned() else {
                    return;
                };
                if v == "そこへ移動" {
                    let (a, b) = rule.range;
                    self.anchor = Some(a);
                    self.cursor = b;
                    self.sync_input();
                    self.status = ui::tf!("{}:{} へ移動しました", a.a1(), b.a1()).into();
                } else {
                    self.checkpoint();
                    self.book.sheets[self.active].cond.remove(i);
                    self.dirty = true;
                    self.status = ui::tf!(
                        "規則({}:{} の {})を消しました",
                        rule.range.0.a1(), rule.range.1.a1(), cond_kind_name(&rule.kind)
                    )
                    .into();
                }
            }
            "csv-import-pick" => {
                if v.starts_with("→ ") {
                    // 取り込む(下ごしらえ済みの grid を流し込む)
                    let Some(pend) = self.import_pend.take() else { return };
                    if pend.grid.is_empty() {
                        self.status = ui::t!("読めた行がありません(設定を見直してください)").into();
                        self.import_pend = Some(pend);
                        self.import_pick();
                        return;
                    }
                    self.checkpoint();
                    let n_rows = pend.grid.len();
                    let n = crate::util::paste_values_text(
                        &mut self.book.sheets[self.active],
                        pend.dest,
                        &pend.grid,
                    );
                    recalc_book(&mut self.book, self.active);
                    self.dirty = true;
                    self.sync_input();
                    self.status = ui::tf!(
                        "{} 行 {} 欄を {} から流し込みました(値として)",
                        n_rows, n, pend.dest.a1()
                    )
                    .into();
                    return;
                }
                let enc_head = format!("{}: ", ui::t!("文字コード"));
                let delim_head = format!("{}: ", ui::t!("区切り"));
                let dest_head = format!("{}: ", ui::t!("置き場所"));
                if v.starts_with(&enc_head) {
                    if let Some(pend) = &mut self.import_pend {
                        pend.enc = (pend.enc + 1) % crate::py::IMPORT_ENCS.len();
                    }
                    self.import_reparse(cx);
                    return;
                }
                if v.starts_with(&delim_head) {
                    let mut ask_custom = false;
                    if let Some(pend) = &mut self.import_pend {
                        pend.delim = (pend.delim + 1) % crate::py::IMPORT_DELIMS.len();
                        ask_custom =
                            crate::py::IMPORT_DELIMS[pend.delim].1 == "その他";
                    }
                    if ask_custom {
                        self.prompt = Some(("csv-delim", Editor::new("")));
                    } else {
                        self.import_reparse(cx);
                    }
                    return;
                }
                if v.starts_with(&dest_head) {
                    let cur = self
                        .import_pend
                        .as_ref()
                        .map(|p| p.dest.a1())
                        .unwrap_or_default();
                    self.prompt = Some(("csv-dest", Editor::new(&cur)));
                    return;
                }
                // プレビューの行は何もしない(板は開いたまま)
                self.import_pick();
                return;
            }
            "spark-kind-pick" => {
                let kind = match v {
                    "縦棒(カラム)" => "spark-col",
                    "勝ち負け(正負)" => "spark-wl",
                    _ => "spark",
                };
                self.insert_sparkline(kind);
            }
            "dedup-pick" => {
                let header_label = ui::t!("先頭行は見出し(消さない)").to_string();
                if v == format!("→ {}", ui::t!("削除する")) {
                    let Some((list, header)) = self.dedup_pend.take() else { return };
                    let cols: Vec<u32> =
                        list.iter().filter(|(_, _, on)| *on).map(|(c, _, _)| *c).collect();
                    if cols.is_empty() {
                        self.status = ui::t!("比べる列を1つは選んでください").into();
                        self.dedup_pend = Some((list, header));
                        self.dedup_pick();
                        return;
                    }
                    self.checkpoint();
                    let all = cols.len() == list.len();
                    let n = self.book.sheets[self.active]
                        .remove_duplicate_rows_in(header, if all { &[] } else { &cols });
                    self.dirty = true;
                    recalc_book(&mut self.book, self.active);
                    self.sync_input();
                    // 何件消したかを黙らない
                    self.status = ui::tf!("重複した {} 行を削除しました", n).into();
                } else if let Some((list, header)) = &mut self.dedup_pend {
                    if v == header_label {
                        *header = !*header;
                    } else if let Some(item) = list.iter_mut().find(|(_, n, _)| *n == v) {
                        item.2 = !item.2;
                    }
                    self.dedup_pick();
                    return;
                }
            }
            "pivot-group-pick" => {
                let Some((pi, field, _)) = self.pivot_flt.clone() else { return };
                if v == "数の幅…" {
                    self.prompt = Some(("pivot-group-width", Editor::new("")));
                    return;
                }
                let Some(d) = self.book.pivots.get_mut(pi) else { return };
                d.group_by.retain(|(f, _)| *f != field);
                if v != "グループ解除" {
                    d.group_by.push((field, v.to_string()));
                }
                let nd = d.clone();
                self.pivot_flt = None;
                self.spawn_pivot(nd, Some(pi), cx);
            }
            "pivot-rows-pick" => {
                if v == "→ 決定(列の選択へ)" {
                    let ok = self
                        .pivot_pend
                        .as_ref()
                        .map(|p| !p.rows_sel.is_empty())
                        .unwrap_or(false);
                    if !ok {
                        self.status =
                            ui::t!("行に並べる見出しを1つは選んでください").into();
                    } else {
                        self.status = ui::t!("列に広げる見出し(なくてもよい)。選んだら「決定」").into();
                        self.pivot_pick("pivot-cols-pick");
                        return;
                    }
                } else if let Some(p) = &mut self.pivot_pend {
                    let h = v.to_string();
                    if let Some(i) = p.rows_sel.iter().position(|x| *x == h) {
                        p.rows_sel.remove(i);
                    } else if p.headers.contains(&h) {
                        p.rows_sel.push(h);
                    }
                }
                self.pivot_pick("pivot-rows-pick");
                return;
            }
            "pivot-cols-pick" => {
                if v == "→ 決定(列は無しでもよい)" {
                    self.status = ui::t!("値にする見出しをクリック(次に集計を選びます)").into();
                    self.pivot_pick("pivot-val-pick");
                    return;
                }
                if let Some(p) = &mut self.pivot_pend {
                    let h = v.to_string();
                    if let Some(i) = p.cols_sel.iter().position(|x| *x == h) {
                        p.cols_sel.remove(i);
                    } else if p.headers.contains(&h) {
                        p.cols_sel.push(h);
                    }
                }
                self.pivot_pick("pivot-cols-pick");
                return;
            }
            "pivot-val-pick" => {
                let known = self
                    .pivot_pend
                    .as_ref()
                    .map(|p| p.headers.contains(&v.to_string()))
                    .unwrap_or(false);
                if known {
                    if let Some(p) = &mut self.pivot_pend {
                        p.val_sel = v.to_string();
                    }
                    self.status = ui::tf!("「{}」をどう集計しますか", v).into();
                    self.pivot_pick("pivot-agg-pick");
                    return;
                }
            }
            "pivot-agg-pick" => {
                let Some(pend) = self.pivot_pend.take() else { return };
                let agg = PIVOT_AGGS.iter().find(|a| **a == v).copied().unwrap_or("合計");
                let value = pend.val_sel.clone();
                self.insert_pivot(pend, value, agg, cx);
            }
            // 並べ替えの「拡張しますか」(選択の横にデータが続いているとき)
            "sort-expand" => {
                let asc = self.sort_pend.take().unwrap_or(true);
                if v.starts_with("拡張して") {
                    // 表全体をカーソル列で(見出しは据え置き — 従来の道)
                    self.sort_col(self.cursor.col, asc);
                } else if v.starts_with("選択した範囲だけ") {
                    let (a, b) = self.sel_rect();
                    self.sort_range_now(a, b, asc);
                } else {
                    self.status = ui::t!("並べ替えをやめました").into();
                }
            }
            // 結合の4択(本家のドロップダウン)
            "merge-pick" => {
                let kind = match v {
                    "横方向に結合(行ごと)" => "横方向",
                    "セルの結合(揃えは触らない)" => "結合だけ",
                    "結合の解除" => "解除",
                    _ => "中央",
                };
                self.merge_selection(kind);
                if self.pick.is_some() {
                    return; // 値の確認へ(pick_kind を戻さない)
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
            "orient-pick" => {
                let deg: Option<i32> = match v {
                    "角度なし" => Some(0),
                    "左上がり 45度" => Some(45),
                    "右下がり 45度" => Some(135),
                    "上向き 90度" => Some(90),
                    "下向き 90度" => Some(180),
                    "縦書き(1字ずつ積む)" => Some(255),
                    _ => None,
                };
                match deg {
                    Some(0) => {
                        self.fmt(|f| f.rotation = None);
                        self.status = ui::t!("文字の向きを戻しました").into();
                    }
                    Some(d) => {
                        self.fmt(move |f| f.rotation = Some(d));
                        self.status = if d == 255 {
                            ui::t!("文字を縦に積みました").into()
                        } else {
                            ui::tf!("文字を {} にしました", v).into()
                        };
                    }
                    None => {
                        // その他 = 任意の角度(-90〜90)
                        self.prompt = Some(("text-angle", Editor::new("")));
                        return; // 板の確定まで
                    }
                }
            }
            "font-color" => {
                if v.starts_with("その他") {
                    self.prompt = Some(("font-color-rgb", Editor::new("")));
                    return; // 板の確定まで
                }
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
                if v.starts_with("その他") {
                    self.prompt = Some(("fill-color-rgb", Editor::new("")));
                    return; // 板の確定まで
                }
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
        self.pick_note = None;
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
            // 図形の専用メニュー(実体は main.rs の shape_menu_action)
            "sh-cut" | "sh-copy" | "sh-paste" | "sh-del" | "sh-front" | "sh-forward"
            | "sh-backward" | "sh-back" | "sh-rot-r" | "sh-rot-l" | "sh-flip-h"
            | "sh-flip-v" | "sh-save" | "sh-settings" => self.shape_menu_action(id),
            "sh-al-l" | "sh-al-c" | "sh-al-r" | "sh-al-t" | "sh-al-m" | "sh-al-b"
            | "sh-dist-h" | "sh-dist-v" => self.shape_align(id),
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
            "sort-fill-top" | "sort-font-top" => self.sort_color_top(id == "sort-fill-top"),
            id if id.starts_with("subt-") => self.set_subtotal_kind(&id["subt-".len()..]),
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
                    kind: sheet::model::CondKind::Cmp(sheet::model::CondOp::Lt, 0.0),
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
            "cond-between" => {
                self.commit();
                self.prompt = Some(("cond-between", Editor::new("")));
            }
            "cond-text" => {
                self.commit();
                self.prompt = Some(("cond-text", Editor::new("")));
            }
            "cond-top" | "cond-bottom" => {
                let bottom = id == "cond-bottom";
                self.commit();
                self.prompt = Some((
                    if bottom { "cond-bottom" } else { "cond-top" },
                    Editor::new("10"),
                ));
            }
            // 板の要らない規則はその場で掛ける
            // 第2版: バー/スケール/アイコン(範囲の最小〜最大が物差し)
            "cond-bar" | "cond-scale" | "cond-icons" => self.cond_visual(id),
            "cond-manage" => {
                self.commit();
                let rules = &self.book.sheets[self.active].cond;
                if rules.is_empty() {
                    self.status = ui::t!("このシートに条件付き書式はありません").into();
                } else {
                    let at = self
                        .cell_origin_px(self.cursor)
                        .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
                        .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                    let items: Vec<String> = rules
                        .iter()
                        .enumerate()
                        .map(|(i, r)| {
                            format!(
                                "{}) {}:{} — {}",
                                i + 1,
                                r.range.0.a1(),
                                r.range.1.a1(),
                                cond_kind_name(&r.kind)
                            )
                        })
                        .collect();
                    self.pick_note = Some(ui::t!("ルールの管理 — 規則をクリックで選ぶ").into());
                    self.pick_kind = "cond-manage-pick";
                    self.pick = Some((items, at));
                }
            }
            "cond-dup" | "cond-uniq" | "cond-avg-above" | "cond-avg-below" => {
                self.commit();
                self.checkpoint();
                let range = self.sel_rect();
                use sheet::model::{CondKind, CondRule};
                let (kind, color, fill, said) = match id {
                    "cond-dup" => (
                        CondKind::Dup(false),
                        Some("9C0006".to_string()),
                        Some("FFC7CE".to_string()),
                        ui::t!("重複する値を赤くします").to_string(),
                    ),
                    "cond-uniq" => (
                        CondKind::Dup(true),
                        None,
                        Some("E2EFDA".to_string()),
                        ui::t!("一意の値を塗ります").to_string(),
                    ),
                    "cond-avg-above" => (
                        CondKind::Avg(false),
                        None,
                        Some("E2EFDA".to_string()),
                        ui::t!("平均より上を塗ります").to_string(),
                    ),
                    _ => (
                        CondKind::Avg(true),
                        None,
                        Some("FCE4D6".to_string()),
                        ui::t!("平均より下を塗ります").to_string(),
                    ),
                };
                self.book.sheets[self.active]
                    .cond
                    .push(CondRule { range, kind, color, fill });
                self.dirty = true;
                self.status =
                    format!("{}:{} — {}", range.0.a1(), range.1.a1(), said).into();
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
            // 見出しの右クリック: 行・列の非表示と再表示
            "hide-rows" | "hide-cols" | "unhide-rows" | "unhide-cols" => self.hide_lines(id),
            // 見出しの右クリック: 幅・高さの数値指定(選んだ列・行ぶん)
            "colw" => {
                let cur = self
                    .sheet()
                    .col_width
                    .get(&self.cursor.col)
                    .map(|w| format!("{w:.2}"))
                    .unwrap_or_default();
                self.prompt = Some(("col-width", Editor::new(&cur)));
            }
            "rowh" => {
                let cur = self
                    .sheet()
                    .row_height
                    .get(&self.cursor.row)
                    .map(|h| format!("{h:.1}"))
                    .unwrap_or_default();
                self.prompt = Some(("row-height", Editor::new(&cur)));
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

    /// 第2版の条件付き書式: バー/スケール/アイコン(範囲の最小〜最大が物差し)
    pub(crate) fn cond_visual(&mut self, id: &str) {
        self.commit();
        self.checkpoint();
        let range = self.sel_rect();
        use sheet::model::{CondKind, CondRule};
        let (kind, said) = match id {
            "cond-bar" => (
                CondKind::Bar("638EC6".into()),
                ui::t!("データバーを敷きます(最小〜最大が棒の長さ)").to_string(),
            ),
            "cond-scale" => (
                CondKind::Scale("F8696B".into(), Some("FFEB84".into()), "63BE7B".into()),
                ui::t!("カラースケールを塗ります(小=赤 〜 大=緑)").to_string(),
            ),
            _ => (
                CondKind::Icons("3Arrows".into()),
                ui::t!("3つの矢印を置きます(下/中/上の三段)").to_string(),
            ),
        };
        self.book.sheets[self.active]
            .cond
            .push(CondRule { range, kind, color: None, fill: None });
        self.dirty = true;
        self.status = format!("{}:{} — {}", range.0.a1(), range.1.a1(), said).into();
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
            // 図形の配置(重なり順)と回転
            "sh-order" => vec![
                ("sh-front", "最前面へ移動", true),
                ("sh-forward", "前面へ移動", true),
                ("sh-backward", "背面へ移動", true),
                ("sh-back", "最背面へ移動", true),
            ],
            "sh-rotate" => vec![
                ("sh-rot-r", "右へ90度回転", true),
                ("sh-rot-l", "左へ90度回転", true),
                ("sh-flip-h", "左右に反転", true),
                ("sh-flip-v", "上下に反転", true),
            ],
            // 整列は2個から、分布は3個から(Ctrl+クリックで束ねる)
            "sh-align" => {
                let n = self.shape_sel.is_some() as usize + self.shape_multi.len();
                vec![
                    ("sh-al-l", "左揃え", n >= 2),
                    ("sh-al-c", "左右中央揃え", n >= 2),
                    ("sh-al-r", "右揃え", n >= 2),
                    ("sh-al-t", "上揃え", n >= 2),
                    ("sh-al-m", "上下中央揃え", n >= 2),
                    ("sh-al-b", "下揃え", n >= 2),
                    ("sh-dist-h", "横に分布", n >= 3),
                    ("sh-dist-v", "縦に分布", n >= 3),
                ]
            }
            "clr" => vec![
                // 本家の消去は5択(すべて/テキスト/書式/コメント/ハイパーリンク)
                ("clear-all", "すべて", true),
                ("clear-text", "テキスト(書式は残す)", true),
                ("clear-fmt", "書式(中身は残す)", true),
                ("clear-comment", "コメント", !self.sheet().comments.is_empty()),
                ("clear-link", "ハイパーリンク", !self.sheet().links.is_empty()),
            ],
            // 本家の合計行のセル右の▼と同じ8択(SUBTOTAL の集計番号)
            "subtotal" => vec![
                ("subt-9", "合計", true),
                ("subt-1", "平均", true),
                ("subt-3", "個数", true),
                ("subt-4", "最大", true),
                ("subt-5", "最小", true),
                ("subt-7", "標準偏差", true),
                ("subt-10", "分散", true),
                ("subt-none", "なし(式を消す)", true),
            ],
            "sort" => {
                let f = self.sheet().get(self.cursor).map(|c| c.fmt.clone()).unwrap_or_default();
                vec![
                    ("sort-asc", "昇順", true),
                    ("sort-desc", "降順", true),
                    ("sort-fill-top", "選択したセルの色を上に", f.fill.is_some()),
                    ("sort-font-top", "選択したフォントの色を上に", f.color.is_some()),
                ]
            }
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
                ("cond-between", "値の間なら薄黄の塗り…", true),
                ("cond-text", "文字を含むと薄黄の塗り…", true),
                ("cond-dup", "重複する値を赤く", true),
                ("cond-uniq", "一意の値を薄緑に", true),
                ("cond-top", "上位Nを薄青に…", true),
                ("cond-bottom", "下位Nを薄赤に…", true),
                ("cond-avg-above", "平均より上を薄緑に", true),
                ("cond-avg-below", "平均より下を薄赤に", true),
                ("cond-bar", "データバー(青の棒)", true),
                ("cond-scale", "カラースケール(赤→黄→緑)", true),
                ("cond-icons", "アイコン(3つの矢印)", true),
                ("cond-manage", "ルールの管理…", true),
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

    /// 元の表から、ある見出しの値の候補(重複は畳む。上限は自衛)。
    pub(crate) fn pivot_field_values(&self, pi: usize, field: &str) -> Vec<String> {
        let Some(d) = self.book.pivots.get(pi) else { return Vec::new() };
        let (a, b) = d.src;
        let Some(fc) = (a.col..=b.col).position(|c| {
            self.sheet()
                .get(Pos::new(a.row, c))
                .map(|x| x.value.display() == field)
                .unwrap_or(false)
        }) else { return Vec::new() };
        let fc = a.col + fc as u32;
        let mut vals: Vec<String> = Vec::new();
        for r in a.row + 1..=b.row {
            let v = self
                .sheet()
                .get(Pos::new(r, fc))
                .map(|x| x.value.display())
                .unwrap_or_default();
            if !v.is_empty() && !vals.contains(&v) {
                vals.push(v);
            }
            if vals.len() >= 1000 {
                break;
            }
        }
        vals
    }

    /// ピボットの絞り込みの一覧(見出しの ▼)。☑ = 表示、☐ = 隠す。
    /// 組み直しては出し直す(クリックのたび)
    pub(crate) fn pivot_filter_pick(&mut self) {
        let Some((pi, field, hidden)) = self.pivot_flt.clone() else { return };
        let Some(d) = self.book.pivots.get(pi) else { return };
        // 値の候補は元の表から(見出しの列の値。重複は畳む。上限は自衛)
        let (a, b) = d.src;
        let Some(fc) = (a.col..=b.col).position(|c| {
            self.sheet()
                .get(Pos::new(a.row, c))
                .map(|x| x.value.display() == field)
                .unwrap_or(false)
        }) else { return };
        let fc = a.col + fc as u32;
        let mut vals: Vec<String> = Vec::new();
        for r in a.row + 1..=b.row {
            let v = self
                .sheet()
                .get(Pos::new(r, fc))
                .map(|x| x.value.display())
                .unwrap_or_default();
            if !v.is_empty() && !vals.contains(&v) {
                vals.push(v);
            }
            if vals.len() >= 1000 {
                break;
            }
        }
        let mut items: Vec<String> = vals
            .iter()
            .map(|v| {
                if hidden.contains(v) {
                    format!("☐ {v}")
                } else {
                    format!("☑ {v}")
                }
            })
            .collect();
        items.push("→ 決定(絞り込む)".into());
        items.push("→ すべて表示に戻す".into());
        items.push("→ ラベルで絞る…".into());
        items.push("→ 値で絞る…".into());
        items.push("→ グループ化…".into());
        let at = self
            .cell_origin_px(self.cursor)
            .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
            .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
        let pname = self.book.pivots.get(pi).map(|d| d.name.clone()).unwrap_or_default();
        self.pick_note = Some(
            ui::tf!("{} の絞り込み — 「{}」(☑ 表示 / ☐ 隠す)", pname, field).into(),
        );
        self.pick_kind = "pivot-filter-pick";
        self.pick = Some((items, at));
    }

    /// ピボットの聞き取りの一覧を(いまの控えから)組み直して開く。
    /// クリックのたびに呼ばれる — ✓ の付け外しはここで反映される
    /// 合計行のセルの集計のしかたを替える(=SUM/=SUBTOTAL → =SUBTOTAL(n, 範囲))。
    pub(crate) fn set_subtotal_kind(&mut self, kind: &str) {
        let p = self.cursor;
        let Some(f) = self.sheet().get(p).and_then(|c| c.formula.clone()) else { return };
        // いまの式から範囲を取り出す(SUM(範囲) / SUBTOTAL(番号, 範囲))
        let inner = f
            .find('(')
            .and_then(|i| f.rfind(')').map(|j| &f[i + 1..j]))
            .unwrap_or("");
        let range = inner.split_once(',').map(|(_, r)| r).unwrap_or(inner).trim().to_string();
        if range.is_empty() {
            self.status = ui::t!("式から範囲が読み取れません").into();
            return;
        }
        self.commit();
        self.checkpoint();
        let mut cell = self.sheet().get(p).cloned().unwrap_or_default();
        if kind == "none" {
            cell.formula = None;
            cell.value = sheet::Value::Empty;
            self.book.sheets[self.active].set(p, cell);
            self.status = ui::t!("集計の式を消しました(書式はそのまま)").into();
        } else {
            let v = sheet::Cell::input(&format!("=SUBTOTAL({kind},{range})"));
            cell.formula = v.formula;
            cell.value = v.value;
            self.book.sheets[self.active].set(p, cell);
            let name = match kind {
                "1" => ui::t!("平均"),
                "3" => ui::t!("個数"),
                "4" => ui::t!("最大"),
                "5" => ui::t!("最小"),
                "7" => ui::t!("標準偏差"),
                "10" => ui::t!("分散"),
                _ => ui::t!("合計"),
            };
            self.status =
                ui::tf!("{} を {} の{}に替えました(絞り込み中の行は数えません)", p.a1(), range, name).into();
        }
        recalc_book(&mut self.book, self.active);
        self.dirty = true;
        self.sync_input();
    }

    /// スパークラインを置く(kind: spark=折れ線 / spark-col=縦棒 / spark-wl=勝ち負け)。
    /// その時の値で描く固定の絵 — データに追従しない(文言でそう言う)
    pub(crate) fn insert_sparkline(&mut self, kind: &str) {
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
            return;
        }
        let (lo, hi) = vals
            .iter()
            .fold((f64::MAX, f64::MIN), |(l, h), v| (l.min(*v), h.max(*v)));
        let n = vals.len();
        let (points, base): (Vec<(f32, f32)>, f32) = match kind {
            // 縦棒: 0 を物差しに入れて、底(0 の高さ)から棒を立てる
            "spark-col" => {
                let lo2 = lo.min(0.0);
                let hi2 = hi.max(0.0);
                let span = (hi2 - lo2).max(1e-9);
                let base = (1.0 - ((0.0 - lo2) / span)) as f32;
                (
                    vals.iter()
                        .enumerate()
                        .map(|(i, v)| {
                            (
                                (i as f32 + 0.5) / n as f32,
                                (1.0 - ((v - lo2) / span)) as f32,
                            )
                        })
                        .collect(),
                    base,
                )
            }
            // 勝ち負け: 符号だけ(正は上へ・負は下へ同じ長さ。0 は底のまま)
            "spark-wl" => (
                vals.iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let y = if *v > 0.0 { 0.1 } else if *v < 0.0 { 0.9 } else { 0.5 };
                        ((i as f32 + 0.5) / n as f32, y)
                    })
                    .collect(),
                0.5,
            ),
            // 折れ線(従来)
            _ => {
                let span = (hi - lo).max(1e-9);
                (
                    vals.iter()
                        .enumerate()
                        .map(|(i, v)| {
                            (
                                i as f32 / (n - 1) as f32,
                                (1.0 - ((v - lo) / span)) as f32,
                            )
                        })
                        .collect(),
                    0.0,
                )
            }
        };
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
            kind: kind.into(),
            fill: None,
            line: Some("1B6E3C".into()),
            points,
            base,
            ..Default::default()
        });
        self.dirty = true;
        let said = match kind {
            "spark-col" => ui::t!("縦棒のスパークライン"),
            "spark-wl" => ui::t!("勝ち負けのスパークライン"),
            _ => ui::t!("折れ線のスパークライン"),
        };
        self.status = ui::tf!(
            "{}を {} に置きました(その時の値で描く固定の絵。データを変えたら作り直してください)",
            said, at.a1()
        )
        .into();
    }

    /// 重複の削除の板 — 比べる列の入切と「先頭行は見出し」。
    pub(crate) fn dedup_pick(&mut self) {
        let Some((list, header)) = &self.dedup_pend else { return };
        let at = self
            .cell_origin_px(self.cursor)
            .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
            .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
        let mut items: Vec<String> = Vec::new();
        for (_, name, on) in list {
            items.push(format!("{} {}", if *on { "☑" } else { "☐" }, name));
        }
        items.push(format!(
            "{} {}",
            if *header { "☑" } else { "☐" },
            ui::t!("先頭行は見出し(消さない)")
        ));
        items.push(format!("→ {}", ui::t!("削除する")));
        self.pick_note = Some(ui::t!("重複の削除 — 比べる列(クリックで入切)").into());
        self.pick_kind = "dedup-pick";
        self.pick = Some((items, at));
    }

    pub(crate) fn pivot_pick(&mut self, kind: &'static str) {
        let Some(pend) = &self.pivot_pend else { return };
        let at = self
            .cell_origin_px(self.cursor)
            .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
            .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
        let mut items: Vec<String> = Vec::new();
        let note: SharedString = match kind {
            "pivot-rows-pick" => {
                for h in &pend.headers {
                    items.push(if pend.rows_sel.contains(h) {
                        format!("☑ {h}")
                    } else {
                        format!("☐ {h}")
                    });
                }
                items.push("→ 決定(列の選択へ)".into());
                ui::t!("ピボット 1/4 — 行に並べる見出し(クリックで入切・複数可)").into()
            }
            "pivot-cols-pick" => {
                for h in pend.headers.iter().filter(|h| !pend.rows_sel.contains(h)) {
                    items.push(if pend.cols_sel.contains(h) {
                        format!("☑ {h}")
                    } else {
                        format!("☐ {h}")
                    });
                }
                items.push("→ 決定(列は無しでもよい)".into());
                ui::t!("ピボット 2/4 — 列に広げる見出し(クリックで入切・無くてもよい)").into()
            }
            "pivot-val-pick" => {
                for h in &pend.headers {
                    items.push(h.clone());
                }
                ui::t!("ピボット 3/4 — 値にする見出しを1つ").into()
            }
            _ => {
                for a in PIVOT_AGGS {
                    items.push(a.to_string());
                }
                ui::tf!("ピボット 4/4 — 「{}」の集計のしかた", pend.val_sel).into()
            }
        };
        self.pick_note = Some(note);
        self.pick_kind = kind;
        self.pick = Some((items, at));
    }

    /// 罫線を選択に掛ける(ペンの線種・色で)。which は一覧の項目名
    /// 線のスタイルの板(ペンに入る)。罫線パレットからも来る
    pub(crate) fn open_border_style_pick(&mut self) {
        let at = self
            .cell_origin_px(self.cursor)
            .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
            .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
        let items: Vec<String> = BORDER_STYLES
            .iter()
            .map(|(n, b)| {
                if *b == self.pen_style {
                    format!("✓ {n}")
                } else {
                    n.to_string()
                }
            })
            .collect();
        self.pick_note = Some(ui::t!("線のスタイル(選ぶとペンに入ります — 次の罫線から効く)").into());
        self.pick_kind = "border-style-pick";
        self.pick = Some((items, at));
    }

    /// 線の色の板(ペンに入る)。罫線パレットからも来る
    pub(crate) fn open_border_color_pick(&mut self) {
        let at = self
            .cell_origin_px(self.cursor)
            .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
            .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
        let mut items: Vec<String> =
            FONT_COLORS.iter().map(|(n, _)| n.to_string()).collect();
        items.push("その他(RRGGBB を打つ)…".into());
        self.pick_note = Some(ui::t!("線の色(選ぶとペンに入ります)").into());
        self.pick_kind = "border-color-pick";
        self.pick = Some((items, at));
    }

    pub(crate) fn apply_borders(&mut self, which: &str) {
        let (a, b) = self.sel_rect();
        let e = sheet::model::Edge::line(self.pen_style, self.pen_color);
        self.checkpoint();
        let sh = &mut self.book.sheets[self.active];
        for r in a.row..=b.row {
            for c in a.col..=b.col {
                let p = Pos::new(r, c);
                let mut cell = sh.get(p).cloned().unwrap_or_default();
                let bd = &mut cell.fmt.borders;
                match which {
                    "下罫線" => {
                        if r == b.row { bd.bottom = e }
                    }
                    "上罫線" => {
                        if r == a.row { bd.top = e }
                    }
                    "左罫線" => {
                        if c == a.col { bd.left = e }
                    }
                    "右罫線" => {
                        if c == b.col { bd.right = e }
                    }
                    "外枠" => {
                        if r == a.row { bd.top = e }
                        if r == b.row { bd.bottom = e }
                        if c == a.col { bd.left = e }
                        if c == b.col { bd.right = e }
                    }
                    "すべての罫線(格子)" => {
                        *bd = sheet::model::Borders {
                            top: e, bottom: e, left: e, right: e,
                        };
                    }
                    // 内側だけ(外周には引かない)— 帳票の中身の区切り
                    "内側の縦線" => {
                        if c > a.col { bd.left = e }
                        if c < b.col { bd.right = e }
                    }
                    "内側の横線" => {
                        if r > a.row { bd.top = e }
                        if r < b.row { bd.bottom = e }
                    }
                    _ => *bd = sheet::model::Borders::NONE, // 罫線を消す
                }
                sh.set(p, cell);
            }
        }
        self.dirty = true;
        self.status = ui::tf!("罫線: {} を {}:{} に掛けました(Ctrl+Z で1手)", which, a.a1(), b.a1()).into();
    }

    /// 「データの入力規則」の板を開く(いまの規則を下敷きに)
    pub(crate) fn dv_open(&mut self) {
        let v = self.sheet().validation_at(self.cursor).cloned();
        let mut d = DvDlg {
            tab: 0,
            kind: 0,
            op: 0,
            allow_blank: true,
            apply_same: false,
            hide_arrow: false,
            err_style: 0,
            eds: std::array::from_fn(|_| Editor::new("")),
            focus: 0,
            menu: 0,
            keep: None,
            was: v.clone(),
        };
        if let Some(v) = &v {
            d.allow_blank = v.allow_blank;
            d.hide_arrow = v.hide_arrow;
            if let Some((t, m)) = &v.input_msg {
                d.eds[2] = Editor::new(t);
                d.eds[3] = Editor::new(m);
            }
            if let Some((s, t, m)) = &v.error_msg {
                d.err_style = DV_STYLES.iter().position(|(k, _)| k == s).unwrap_or(0);
                d.eds[4] = Editor::new(t);
                d.eds[5] = Editor::new(m);
            }
            match v.kind.as_str() {
                "" => d.kind = 0,
                "whole" => d.kind = 1,
                "decimal" => d.kind = 2,
                "list" => d.kind = 3,
                "textLength" => d.kind = 4,
                // 日付・時刻・カスタムは判定できない — 開いても壊さない(保持)
                _ => {
                    d.kind = 5;
                    d.keep = Some(v.clone());
                }
            }
            if d.kind == 3 {
                // 直書きは中身、参照は = 付き(従来の書き方)
                let f = v.formula.trim();
                let init = match f.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                    Some(inner) => inner.to_string(),
                    None if f.is_empty() => String::new(),
                    None => format!("={f}"),
                };
                d.eds[0] = Editor::new(&init);
            } else if matches!(d.kind, 1 | 2 | 4) {
                d.op = DV_OPS.iter().position(|(k, _)| *k == v.op).unwrap_or(0);
                d.eds[0] = Editor::new(&v.formula);
                d.eds[1] = Editor::new(&v.formula2);
            }
        }
        self.dv_dlg = Some(d);
    }

    /// 「データの入力規則」の OK。選択の範囲に規則を掛ける(重なる規則は
    /// 入れ替え)。読めない条件は板を開いたまま言い返す
    pub(crate) fn dv_ok(&mut self, cx: &mut Context<Self>) {
        let Some(mut d) = self.dv_dlg.take() else { return };
        let f = |i: usize| -> String { d.eds[i].text().trim().to_string() };
        let (a, b) = self.sel_rect();
        let input_msg = {
            let (t, m) = (f(2), f(3));
            (!t.is_empty() || !m.is_empty()).then_some((t, m))
        };
        let error_msg = {
            let (t, m) = (f(4), f(5));
            (!t.is_empty() || !m.is_empty())
                .then_some((DV_STYLES[d.err_style].0.to_string(), t, m))
        };
        let new_v: Option<sheet::model::Validation> = match d.kind {
            // 読めない種類(日付など)はそのまま保つ — 文言と空白の扱いだけ更新
            5 => d.keep.take().map(|mut v| {
                v.range = (a, b);
                v.input_msg = input_msg.clone();
                v.error_msg = error_msg.clone();
                v.allow_blank = d.allow_blank;
                v
            }),
            // すべての値: 条件なし。文言も何も無ければ「規則なし」= 外す
            0 => (input_msg.is_some() || error_msg.is_some() || !d.allow_blank).then(|| {
                let mut v = sheet::model::Validation::list((a, b), String::new());
                v.kind = String::new();
                v.input_msg = input_msg.clone();
                v.error_msg = error_msg.clone();
                v.allow_blank = d.allow_blank;
                v
            }),
            // リスト: 直書き(甲,乙,丙)か範囲の参照(=D2:D5)
            3 => {
                let text = f(0);
                if text.is_empty() {
                    self.status =
                        ui::t!("元の値を書いてください(例: 甲,乙,丙 または =D2:D5)").into();
                    self.dv_dlg = Some(d);
                    cx.notify();
                    return;
                }
                let formula = match text.strip_prefix('=') {
                    Some(r) => r.trim().to_string(),
                    None => format!("\"{}\"", text.replace('"', "")),
                };
                let mut v = sheet::model::Validation::list((a, b), formula);
                if v.options(self.sheet()).is_empty() {
                    // 読めない規則を作らない(できないものを、できるように見せない)
                    self.status =
                        ui::t!("候補が読めません(例: 甲,乙,丙 または =D2:D5)").into();
                    self.dv_dlg = Some(d);
                    cx.notify();
                    return;
                }
                v.input_msg = input_msg.clone();
                v.error_msg = error_msg.clone();
                v.allow_blank = d.allow_blank;
                v.hide_arrow = d.hide_arrow;
                Some(v)
            }
            // 整数 / 小数 / 文字数
            k => {
                let (opk, _) = DV_OPS[d.op.min(DV_OPS.len() - 1)];
                let need2 = matches!(opk, "between" | "notBetween");
                // IME の全角の数・記号は半角にならす(打ち直させない)
                let norm = |t: String| -> String {
                    t.chars()
                        .map(|c| match c {
                            '\u{FF10}'..='\u{FF19}' => {
                                char::from(b'0' + (c as u32 - 0xFF10) as u8)
                            }
                            '\u{FF0E}' => '.',
                            '\u{FF0D}' | '\u{2212}' => '-',
                            _ => c,
                        })
                        .collect()
                };
                let (f1, f2) = (norm(f(0)), norm(f(1)));
                if f1.is_empty() || (need2 && f2.is_empty()) {
                    self.status = ui::t!("条件の値を書いてください(半角の数で)").into();
                    self.dv_dlg = Some(d);
                    cx.notify();
                    return;
                }
                Some(sheet::model::Validation {
                    range: (a, b),
                    formula: f1,
                    kind: DV_KIND_XLSX[k].into(),
                    op: opk.into(),
                    formula2: if need2 { f2 } else { String::new() },
                    input_msg: input_msg.clone(),
                    error_msg: error_msg.clone(),
                    allow_blank: d.allow_blank,
                    hide_arrow: d.hide_arrow,
                })
            }
        };
        self.checkpoint();
        // 「同じ設定の他のすべてのセル」: 開いたときの規則と同じ条件の規則を
        // 先に差し替える(範囲は据え置き)
        if d.apply_same {
            if let (Some(was), Some(nv)) = (&d.was, &new_v) {
                for x in self.book.sheets[self.active].validations.iter_mut() {
                    if x.kind == was.kind
                        && x.op == was.op
                        && x.formula == was.formula
                        && x.formula2 == was.formula2
                    {
                        let range = x.range;
                        *x = nv.clone();
                        x.range = range;
                    }
                }
            }
        }
        // 選択に重なる規則は入れ替える(重ね掛けは分かりにくい)
        let overlap = |x: &sheet::model::Validation| {
            let (ra, rb) = x.range;
            ra.row <= b.row && rb.row >= a.row && ra.col <= b.col && rb.col >= a.col
        };
        let had = self.sheet().validations.iter().any(|x| overlap(x));
        self.book.sheets[self.active].validations.retain(|x| !overlap(x));
        match new_v {
            Some(v) => {
                let said = if v.kind == "list" {
                    ui::tf!("候補: {}", v.options(self.sheet()).join(" / ")).to_string()
                } else if v.kind.is_empty() {
                    ui::t!("文言だけ(値は制限しない)").to_string()
                } else {
                    v.describe()
                };
                self.book.sheets[self.active].validations.push(v);
                self.status = ui::tf!(
                    "入力規則を {}:{} に掛けました({}。保存で xlsx にも残ります)",
                    a.a1(), b.a1(), said
                )
                .into();
            }
            None if had => {
                self.status = ui::t!("この範囲の入力規則を外しました").into();
            }
            None => {
                self.undo_stack.pop();
                self.status = ui::t!("入力規則はありません(何も変えていません)").into();
            }
        }
        self.dirty = true;
        cx.notify();
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
        self.sort_pend = None; // 並べ替えの「拡張しますか」も
        self.pivot_flt = None; // ピボットの絞り込みの聞き取りも
        self.hf_pend = None; // ヘッダー/フッターの聞き取りも
        self.name_pend = None; // 名前マネージャーの選択も
        if self.brush.take().is_some() {
            self.status = ui::t!("書式のコピーをやめました").into();
        }
        self.menu_head = None; // 見出しメニューの印も畳む
        self.dedup_pend = None;
        self.cond_pend = None;
        self.import_pend = None;
        self.border_pal = None;

        self.pw_pending = None; // パスワード待ちも Esc でやめる(開かない)
        // 入力規則の板: 開いたドロップダウン → 板、の順で閉じる
        if let Some(d) = &mut self.dv_dlg {
            if d.menu != 0 {
                d.menu = 0;
            } else {
                self.dv_dlg = None;
                self.status = ui::t!("入力規則をやめました").into();
            }
            cx.notify();
            return;
        }
        if self.tool.take().is_some() {
            self.ink_cur = None;
            self.status = ui::t!("セルの操作に戻りました").into();
        }
        self.shape_multi.clear();
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
            || self.img_sel.take().is_some()
        {
            // 一覧・板を閉じたら意味づけも戻す(耳のメニューの狙い先も)
            self.pick_kind = "value";
            self.pick_note = None;
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
            // テキスト取り込み: その他の区切り(1文字)
            "csv-delim" => {
                let t = text.trim();
                let Some(c0) = t.chars().next() else {
                    self.status = ui::t!("区切りの文字を1つ打ってください(例: |)").into();
                    self.prompt = Some((kind, Editor::new("")));
                    return;
                };
                if let Some(pend) = &mut self.import_pend {
                    pend.custom = c0.to_string();
                }
                self.import_reparse(cx);
            }
            // テキスト取り込み: 置き場所(A1 の形)
            "csv-dest" => {
                let Some(p2) = Pos::parse(text.trim()) else {
                    self.status = ui::t!("場所が読めません(B12 の形で)").into();
                    self.prompt = Some((kind, Editor::new(text.trim())));
                    return;
                };
                if let Some(pend) = &mut self.import_pend {
                    pend.dest = p2;
                }
                self.import_pick();
            }
            // 反復計算の入切(回数 変化量。空 Enter = 切)
            "calc-iter" => {
                let t = text.trim();
                if t.is_empty() {
                    self.book.calc_iter = None;
                    self.dirty = true;
                    self.status = ui::t!("反復計算を切りました(循環参照は #CIRC! に戻ります)").into();
                    return;
                }
                let mut it = t.split_whitespace();
                let n: Option<u32> = it.next().and_then(|v| v.parse().ok());
                let d: Option<f64> = match it.next() {
                    Some(v) => v.parse().ok(),
                    None => Some(0.001),
                };
                match (n, d) {
                    (Some(n), Some(d)) if n >= 1 && d >= 0.0 => {
                        self.book.calc_iter = Some((n, d));
                        self.dirty = true;
                        recalc_book(&mut self.book, self.active);
                        self.sync_input();
                        self.status = ui::tf!(
                            "反復計算: 入(最大 {} 回、変化 {} まで) — 循環参照を回して解きます",
                            n, d
                        )
                        .into();
                    }
                    _ => {
                        self.status = ui::t!("「100 0.001」の形で(回数は1以上)").into();
                        self.prompt = Some((kind, Editor::new(t)));
                    }
                }
            }
            // ピボット: ラベルで絞る(含む/で始まる/で終わる 語)。
            // 合う値**以外**を hide に落とす — 既存の絞り込み機構に乗せる
            "pivot-label" => {
                let Some((pi, field, _)) = self.pivot_flt.take() else { return };
                let t = text.trim();
                if t.is_empty() {
                    self.status = ui::t!("条件が空です(例: 含む 東京)").into();
                    return;
                }
                let (op, word) = match t.split_once(char::is_whitespace) {
                    Some((a, b)) if ["含む", "で始まる", "で終わる"].contains(&a) => {
                        (a.to_string(), b.trim().to_string())
                    }
                    _ => ("含む".into(), t.to_string()),
                };
                let ok = |v: &str| match op.as_str() {
                    "で始まる" => v.starts_with(&word),
                    "で終わる" => v.ends_with(&word),
                    _ => v.contains(&word),
                };
                let vals = self.pivot_field_values(pi, &field);
                let hidden: Vec<String> = vals.into_iter().filter(|v| !ok(v)).collect();
                if let Some(d) = self.book.pivots.get_mut(pi) {
                    d.hide.retain(|(f, _)| *f != field);
                    if !hidden.is_empty() {
                        d.hide.push((field, hidden));
                    }
                    let nd = d.clone();
                    self.spawn_pivot(nd, Some(pi), cx);
                }
            }
            // ピボット: 値で絞る(> 1000 の形。空 Enter = 解除)
            "pivot-vfilter" => {
                let Some((pi, _, _)) = self.pivot_flt.take() else { return };
                let t = text.trim();
                let Some(d) = self.book.pivots.get_mut(pi) else { return };
                if t.is_empty() {
                    d.vfilter = None;
                } else {
                    let (op, num) = match t.split_once(char::is_whitespace) {
                        Some((a, b)) if [">", ">=", "<", "<=", "="].contains(&a) => {
                            (a.to_string(), b.trim())
                        }
                        _ => {
                            self.status =
                                ui::t!("「> 1000」の形で(比較は > >= < <= =)").into();
                            self.prompt = Some((kind, Editor::new(t)));
                            self.pivot_flt = Some((pi, String::new(), Default::default()));
                            return;
                        }
                    };
                    let Ok(th) = num.parse::<f64>() else {
                        self.status = ui::t!("しきい値が数として読めません").into();
                        self.prompt = Some((kind, Editor::new(t)));
                        self.pivot_flt = Some((pi, String::new(), Default::default()));
                        return;
                    };
                    d.vfilter = Some((op, th));
                }
                let nd = d.clone();
                self.spawn_pivot(nd, Some(pi), cx);
            }
            // ピボット: 数の幅でグループ化(例: 100)
            "pivot-group-width" => {
                let Some((pi, field, _)) = self.pivot_flt.take() else { return };
                let Ok(w) = text.trim().parse::<f64>() else {
                    self.status = ui::t!("幅が数として読めません(例: 100)").into();
                    self.prompt = Some((kind, Editor::new(text.trim())));
                    self.pivot_flt = Some((pi, field, Default::default()));
                    return;
                };
                if w <= 0.0 {
                    self.status = ui::t!("幅は 0 より大きい数で").into();
                    self.prompt = Some((kind, Editor::new(text.trim())));
                    self.pivot_flt = Some((pi, field, Default::default()));
                    return;
                }
                let Some(d) = self.book.pivots.get_mut(pi) else { return };
                d.group_by.retain(|(f, _)| *f != field);
                d.group_by.push((field, format!("幅:{w}")));
                let nd = d.clone();
                self.spawn_pivot(nd, Some(pi), cx);
            }
            // 列の幅・行の高さの数値指定(選んだ列・行ぶん。空 = 既定に戻す)
            "col-width" | "row-height" => {
                let is_col = kind == "col-width";
                let (a, b) = self.sel_rect();
                let t = text.trim();
                if t.is_empty() {
                    self.checkpoint();
                    if is_col {
                        for c in a.col..=b.col {
                            self.sheet_mut().col_width.remove(&c);
                        }
                    } else {
                        for r in a.row..=b.row {
                            self.sheet_mut().row_height.remove(&r);
                        }
                    }
                    self.dirty = true;
                    self.status = ui::t!("既定の大きさに戻しました").into();
                    return;
                }
                let Ok(v) = t.parse::<f32>() else {
                    self.status = ui::t!("半角の数で(例: 12.5)").into();
                    self.prompt = Some((kind, Editor::new(t)));
                    return;
                };
                let ok = if is_col { (0.0..=255.0).contains(&v) } else { (0.0..=409.0).contains(&v) };
                if !ok {
                    self.status = if is_col {
                        ui::t!("列の幅は 0〜255 で").into()
                    } else {
                        ui::t!("行の高さは 0〜409 で").into()
                    };
                    self.prompt = Some((kind, Editor::new(t)));
                    return;
                }
                self.checkpoint();
                if is_col {
                    for c in a.col..=b.col {
                        self.sheet_mut().col_width.insert(c, v);
                    }
                    self.status = ui::tf!("列の幅を {} にしました({} 列)", v, b.col - a.col + 1).into();
                } else {
                    for r in a.row..=b.row {
                        self.sheet_mut().row_height.insert(r, v);
                    }
                    self.status = ui::tf!("行の高さを {} pt にしました({} 行)", v, b.row - a.row + 1).into();
                }
                self.dirty = true;
            }
            // 名前の中身の打ち直し(A1 か A1:C9 の形)
            "name-range" => {
                let Some(name) = self.name_pend.take() else { return };
                let t = text.trim().to_uppercase();
                let ok = match t.split_once(':') {
                    Some((a, b)) => Pos::parse(a).is_some() && Pos::parse(b).is_some(),
                    None => Pos::parse(&t).is_some(),
                };
                if !ok {
                    self.status = ui::t!("場所が読めません(B12 か A1:C9 の形)").into();
                    self.name_pend = Some(name);
                    self.prompt = Some(("name-range", Editor::new(&t)));
                    return;
                }
                self.checkpoint();
                let s = &mut self.book.sheets[self.active];
                if let Some(e) = s.names.iter_mut().find(|(n, _)| *n == name) {
                    e.1 = t.clone();
                }
                recalc_book(&mut self.book, self.active);
                self.dirty = true;
                self.status = ui::tf!("名前「{}」= {} にしました", name, t).into();
            }
            // ヘッダー/フッターの1区分(空 Enter = その区分を消す)
            "hf-edit" => {
                let Some((footer, slot)) = self.hf_pend.take() else { return };
                self.checkpoint();
                let raw = if footer {
                    self.sheet().footer.clone()
                } else {
                    self.sheet().header.clone()
                };
                let (mut l, mut c, mut r) =
                    sheet::model::hf_split(raw.as_deref().unwrap_or(""));
                match slot { 0 => l = text.clone(), 1 => c = text.clone(), _ => r = text.clone() }
                let joined = sheet::model::hf_join(&l, &c, &r);
                let val = if joined.is_empty() { None } else { Some(joined) };
                if footer {
                    self.sheet_mut().footer = val;
                } else {
                    self.sheet_mut().header = val;
                }
                self.dirty = true;
                self.status = if text.is_empty() {
                    ui::t!("その区分を消しました").into()
                } else {
                    ui::t!("ヘッダー/フッターに入れました(印刷と PDF で見えます。&P=頁 &N=総頁)").into()
                };
            }
            // 文字の色・塗りの直指定(RRGGBB)。空 Enter = 自動/塗りなし
            "font-color-rgb" | "fill-color-rgb" => {
                let is_font = kind == "font-color-rgb";
                let t = text.trim().trim_start_matches('#').to_uppercase();
                if t.is_empty() {
                    if is_font {
                        self.fmt(|f| f.color = None);
                        self.status = ui::t!("文字の色を自動に戻しました").into();
                    } else {
                        self.fmt(|f| f.fill = None);
                        self.status = ui::t!("塗りを消しました").into();
                    }
                } else if t.len() == 6 && u32::from_str_radix(&t, 16).is_ok() {
                    let c = Some(t.clone());
                    if is_font {
                        self.fmt(move |f| f.color = c.clone());
                        self.status = ui::tf!("文字の色を{}にしました", format!("#{t}")).into();
                    } else {
                        self.fmt(move |f| f.fill = c.clone());
                        self.status = ui::tf!("塗りを{}にしました", format!("#{t}")).into();
                    }
                } else {
                    self.status = ui::t!("色が読めません(RRGGBB の6桁。例: FF0000)").into();
                    self.prompt = Some((kind, Editor::new(&t)));
                }
            }
            // 文字の角度の直指定(-90〜90。xlsx の encode は負を 90+|d| で)
            "text-angle" => {
                let t = text.trim().replace('°', "");
                match t.parse::<i32>() {
                    Ok(d) if (-90..=90).contains(&d) => {
                        let enc: Option<i32> = if d == 0 {
                            None
                        } else if d > 0 {
                            Some(d)
                        } else {
                            Some(90 - d) // -30 → 120(xlsx の encode)
                        };
                        self.fmt(move |f| f.rotation = enc);
                        self.status = if d == 0 {
                            ui::t!("文字の向きを戻しました").into()
                        } else {
                            ui::tf!("文字を {} 度にしました(上向きが正)", d).into()
                        };
                    }
                    _ => {
                        self.status =
                            ui::t!("角度が読めません(-90〜90 の数。縦書きは一覧から)").into();
                        self.prompt = Some(("text-angle", Editor::new(&t)));
                    }
                }
            }
            // 罫線の色の直指定(RRGGBB)。空 Enter = 自動(黒)
            "border-color-rgb" => {
                let t = text.trim().trim_start_matches('#').to_string();
                if t.is_empty() {
                    self.pen_color = None;
                    self.status = ui::t!("線の色: 自動(黒)").into();
                } else if t.len() == 6 {
                    if let Ok(v) = u32::from_str_radix(&t, 16) {
                        self.pen_color = Some(v);
                        self.status = ui::tf!("線の色: #{}(罫線の一覧から掛けると効きます)", t.to_uppercase()).into();
                    } else {
                        self.status = ui::t!("色が読めません(RRGGBB の6桁。例: FF0000)").into();
                        self.prompt = Some(("border-color-rgb", Editor::new(&t)));
                    }
                } else {
                    self.status = ui::t!("色が読めません(RRGGBB の6桁。例: FF0000)").into();
                    self.prompt = Some(("border-color-rgb", Editor::new(&t)));
                }
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
                    kind: sheet::model::CondKind::Cmp(
                        if gt { sheet::model::CondOp::Gt } else { sheet::model::CondOp::Lt },
                        value,
                    ),
                    color: None,
                    fill: Some(if gt { "E2EFDA".into() } else { "FCE4D6".into() }),
                });
                self.dirty = true;
                self.status = ui::tf!("{}:{} — {} より{}を塗ります", range.0.a1(), range.1.a1(), value, if gt { "大きい値" } else { "小さい値" }).into();
            }
            // 条件付き書式の板(間・文字・上位/下位N)
            "cond-between" => {
                let t = text.replace('~', "〜");
                let Some((a1, b1)) = t.split_once('〜') else {
                    self.status = ui::t!("「8〜15」の形で(半角の数)").into();
                    self.prompt = Some(("cond-between", Editor::new(&text)));
                    return;
                };
                let (Ok(lo), Ok(hi)) = (a1.trim().parse::<f64>(), b1.trim().parse::<f64>())
                else {
                    self.status = ui::t!("「8〜15」の形で(半角の数)").into();
                    self.prompt = Some(("cond-between", Editor::new(&text)));
                    return;
                };
                self.checkpoint();
                let range = self.sel_rect();
                self.book.sheets[self.active].cond.push(sheet::model::CondRule {
                    range,
                    kind: sheet::model::CondKind::Between(lo.min(hi), lo.max(hi), false),
                    color: None,
                    fill: Some("FFF2CC".into()),
                });
                self.dirty = true;
                self.status = ui::tf!("{}:{} — {} から {} の間を塗ります", range.0.a1(), range.1.a1(), lo.min(hi), lo.max(hi)).into();
            }
            "cond-text" => {
                if text.is_empty() {
                    self.status = ui::t!("含む文字を入れてください").into();
                    return;
                }
                self.checkpoint();
                let range = self.sel_rect();
                self.book.sheets[self.active].cond.push(sheet::model::CondRule {
                    range,
                    kind: sheet::model::CondKind::Text(text.clone()),
                    color: None,
                    fill: Some("FFF2CC".into()),
                });
                self.dirty = true;
                self.status = ui::tf!("{}:{} — 「{}」を含むセルを塗ります", range.0.a1(), range.1.a1(), text).into();
            }
            "cond-top" | "cond-bottom" => {
                let Ok(n) = text.trim().parse::<u32>() else {
                    self.status = ui::t!("個数を半角の数で(例: 10)").into();
                    self.prompt = Some((kind, Editor::new(&text)));
                    return;
                };
                let bottom = kind == "cond-bottom";
                self.checkpoint();
                let range = self.sel_rect();
                self.book.sheets[self.active].cond.push(sheet::model::CondRule {
                    range,
                    kind: sheet::model::CondKind::Top(n.max(1), bottom),
                    color: None,
                    fill: Some(if bottom { "FCE4D6".into() } else { "D9E1F2".into() }),
                });
                self.dirty = true;
                self.status = ui::tf!("{}:{} — {}{} を塗ります", range.0.a1(), range.1.a1(), if bottom { "下位" } else { "上位" }, n.max(1)).into();
            }
            "py" => {
                let t = text.trim().to_string();
                if t.is_empty() {
                    // 空 Enter = .py ファイルを選ぶ
                    self.run_python_file_dialog(cx);
                } else if t == "@計算" || t == "@calc" {
                    self.run_py_calc(cx);
                } else if t == "@" || t == "@list" {
                    // ブックに載るのは関数(UDF)だけ。手続きは plugins の .py
                    let fns: Vec<&str> =
                        self.book.scripts.iter().map(|(n, _)| n.as_str()).collect();
                    let plugs: Vec<String> = std::fs::read_dir(plugins_dir())
                        .ok()
                        .into_iter()
                        .flatten()
                        .flatten()
                        .map(|e| e.path())
                        .filter(|p| p.extension().is_some_and(|e| e == "py"))
                        .filter_map(|p| {
                            p.file_stem().map(|s| s.to_string_lossy().to_string())
                        })
                        .collect();
                    self.status = match (fns.is_empty(), plugs.is_empty()) {
                        (true, true) => ui::t!(
                            "Python はありません(関数は @save 関数名 でブックへ、手続きは plugins へ .py を置く)"
                        )
                        .into(),
                        _ => ui::tf!(
                            "ブックの関数: {} / plugins: {}(手続きは @名前 で実行)",
                            if fns.is_empty() { "-".to_string() } else { fns.join(" / ") },
                            if plugs.is_empty() { "-".to_string() } else { plugs.join(" / ") }
                        )
                        .into(),
                    };
                } else if let Some(name) = t.strip_prefix("@save ") {
                    let name = name.trim().to_string();
                    if name.is_empty() {
                        self.status = ui::t!("@save 名前 の形で").into();
                    } else if !name.starts_with("関数") {
                        // 手続きはブックに載せない(発注者確定 2026-08-08:
                        // ファイルを実行の起点にしない。UDF だけが旅できる)
                        self.status = ui::tf!(
                            "手続きはブックに載せられません(載るのは「関数」で始まる UDF だけ)。手続きは {} に .py を置いてください",
                            plugins_dir().display().to_string()
                        )
                        .into();
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
                } else if let Some(name) = t.strip_prefix("@export ") {
                    // 古いブックの手続きの取り出し口(実行はしない。中身を見て、
                    // 良ければ自分で plugins へ置く — それが取り込みの門)
                    self.export_python_dialog(name.trim().to_string(), cx);
                } else if let Some(rest) = t.strip_prefix('@') {
                    // 手続きは**手元(plugins)の .py だけ**実行する(発注者確定
                    // 2026-08-08)。ブックに載って旅してきた手続きは実行しない —
                    // ファイルは実行の起点になれない。檻は従来どおり必須、
                    // 網は既定で閉じ「@名前 net」とその場で打ったときだけ開く
                    let (name, net) = match rest.trim().strip_suffix(" net") {
                        Some(n) => (n.trim(), true),
                        None => (rest.trim(), false),
                    };
                    let path = plugins_dir().join(format!("{name}.py"));
                    if path.exists() {
                        match std::fs::read_to_string(&path) {
                            Ok(code) => {
                                if net {
                                    self.status = ui::t!(
                                        "網あり檻で実行します(ファイルは守られたまま)"
                                    )
                                    .into();
                                }
                                self.run_python_inner(code, true, net, cx);
                            }
                            Err(e) => self.status = ui::tf!("読めません: {}", e.to_string()).into(),
                        }
                    } else if self.book.scripts.iter().any(|(n, _)| n == name) {
                        self.status = if name.starts_with("関数") {
                            ui::t!("それは関数(UDF)です — =PY(\"名前\", …) のセルと @計算 が使います").into()
                        } else {
                            ui::tf!(
                                "「{}」はブックの中の手続きです — ブックからは実行しません(@export {} で取り出し、確かめてから {} へ)",
                                name,
                                name,
                                plugins_dir().display().to_string()
                            )
                            .into()
                        };
                    } else {
                        self.status = ui::tf!(
                            "「{}」はありません({} の .py が @名前 で動きます。@list で一覧)",
                            name,
                            plugins_dir().display().to_string()
                        )
                        .into();
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
            // 図形の塗り・線の色の直指定(RRGGBB)。空 Enter = なし
            "shape-fill-rgb" | "shape-line-rgb" => {
                let is_fill = kind == "shape-fill-rgb";
                let t = text.trim().trim_start_matches('#').to_uppercase();
                if t.is_empty() {
                    self.shape_edit(|sp| {
                        if is_fill {
                            sp.fill = None;
                        } else {
                            sp.line = None;
                        }
                    });
                    self.status = if is_fill {
                        ui::t!("塗りを消しました").into()
                    } else {
                        ui::t!("線を消しました").into()
                    };
                } else if t.len() == 6 && u32::from_str_radix(&t, 16).is_ok() {
                    let c = t.clone();
                    self.shape_edit(move |sp| {
                        if is_fill {
                            sp.fill = Some(c);
                        } else {
                            sp.line = Some(c);
                        }
                    });
                    self.status = if is_fill {
                        ui::tf!("塗りを{}にしました", format!("#{t}")).into()
                    } else {
                        ui::tf!("線の色を{}にしました", format!("#{t}")).into()
                    };
                } else {
                    self.status = ui::t!("色が読めません(RRGGBB の6桁。例: FF0000)").into();
                    self.prompt = Some((kind, Editor::new(&t)));
                }
            }
            // 図形の回転の直指定(度・時計回り)。空 Enter = 0 に戻す
            "shape-rot" => {
                let t = text.trim().replace('°', "");
                if t.is_empty() {
                    self.shape_edit(|sp| sp.rot = 0.0);
                    self.status = ui::t!("回転を戻しました").into();
                } else {
                    match t.parse::<f32>() {
                        Ok(d) if d.is_finite() => {
                            let d = d.rem_euclid(360.0);
                            self.shape_edit(move |sp| sp.rot = d);
                            self.status =
                                ui::tf!("{}度回しました(時計回り)", format!("{d:.0}")).into();
                        }
                        _ => {
                            self.status =
                                ui::t!("角度が読めません(数を1つ。例: 45 / -30)").into();
                            self.prompt = Some(("shape-rot", Editor::new(&t)));
                        }
                    }
                }
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
                    // 続けて表示テキスト(セルに見せる文字)。本家のリンク設定の欄と同じ
                    let cur = self.sheet().get(p).map(|c| c.value.display()).unwrap_or_default();
                    self.prompt = Some(("link-text", Editor::new(&cur)));
                }
            }
            // リンクの表示テキスト。空 Enter = セルはそのまま
            "link-text" => {
                let p = self.cursor;
                if !text.is_empty() {
                    let cur = self.sheet().get(p).map(|c| c.value.display()).unwrap_or_default();
                    if text != cur {
                        self.checkpoint();
                        let mut cell = self.sheet().get(p).cloned().unwrap_or_default();
                        let v = sheet::Cell::input(&text);
                        cell.formula = v.formula;
                        cell.value = v.value;
                        self.book.sheets[self.active].set(p, cell);
                        recalc_book(&mut self.book, self.active);
                        self.dirty = true;
                        self.sync_input();
                    }
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
                if r == a.row { cell.fmt.borders.top = sheet::model::Edge::THIN }
                if r == b.row { cell.fmt.borders.bottom = sheet::model::Edge::THIN }
                if c == a.col { cell.fmt.borders.left = sheet::model::Edge::THIN }
                if c == b.col { cell.fmt.borders.right = sheet::model::Edge::THIN }
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

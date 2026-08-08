//! main.rs からの純移動(2026-08-06 の分割)。挙動は変えない。

use crate::*;

impl Focusable for Calc {
    fn focus_handle(&self, _cx: &App) -> FocusHandle { self.focus.clone() }
}

impl EntityInputHandler for Calc {
    fn text_for_range(&mut self, r: Range<usize>, actual: &mut Option<Range<usize>>,
                      _w: &mut Window, _cx: &mut Context<Self>) -> Option<String> {
        handler::text_for_range(self, r, actual)
    }
    fn selected_text_range(&mut self, _i: bool, _w: &mut Window, _cx: &mut Context<Self>)
        -> Option<UTF16Selection> {
        Some(UTF16Selection { range: handler::selected_range_utf16(self), reversed: false })
    }
    fn marked_text_range(&self, _w: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
        handler::marked_range_utf16(self)
    }
    fn unmark_text(&mut self, _w: &mut Window, _cx: &mut Context<Self>) { handler::unmark(self) }
    fn replace_text_in_range(&mut self, r: Option<Range<usize>>, text: &str,
                             _w: &mut Window, cx: &mut Context<Self>) {
        // 空白キーはチェックボックス(Bool のセル)の切替。打ちかけ・板・
        // 小窓が無いときだけ(文字としての空白を奪わない)
        if text == " " && self.prompt.is_none() && self.solver.is_none() && !self.editing() {
            if let Some(Value::Bool(b)) =
                self.sheet().get(self.cursor).map(|c| c.value.clone())
            {
                if self.sheet().protected {
                    self.status =
                        ui::t!("シートが保護されています(保護タブの「シートを保護する」で解除)").into();
                } else {
                    self.checkpoint();
                    let p = self.cursor;
                    let mut cell = self.sheet().get(p).cloned().unwrap_or_default();
                    cell.formula = None;
                    cell.value = Value::Bool(!b);
                    self.book.sheets[self.active].set(p, cell);
                    recalc_book(&mut self.book, self.active);
                    self.dirty = true;
                    self.sync_input();
                    self.status = ui::tf!("{} = {}(空白キーで切替)", p.a1(), if b { "☐" } else { "☑" })
                    .into();
                }
                cx.notify();
                return;
            }
        }
        // セルを選んで**打ち始めたら置き換え**(Excel の作法)。追記になるのは
        // 同じセルで編集を続けている間(edit_armed)だけ — F2・ダブルクリック・
        // 2打目以降。IME の変換途中(marked)は消さない
        if self.prompt.is_none() && self.solver.is_none()
            && self.name_edit.is_none() && self.fn_dlg.is_none()
            && self.fn_args.is_none()
            && !self.edit_armed && !self.editing()
            && handler::marked_range_utf16(self).is_none()
        {
            self.input = Editor::new("");
            self.edit_armed = true;
        }
        handler::replace(self, r, text);
        cx.notify();
    }
    fn replace_and_mark_text_in_range(&mut self, r: Option<Range<usize>>, text: &str,
                                      sel: Option<Range<usize>>, _w: &mut Window,
                                      cx: &mut Context<Self>) {
        // IME の1打目も同じ(変換中の下線ごと、空にしてから始める)
        if self.prompt.is_none() && self.solver.is_none()
            && self.name_edit.is_none() && self.fn_dlg.is_none()
            && self.fn_args.is_none()
            && !self.edit_armed && !self.editing()
            && handler::marked_range_utf16(self).is_none()
        {
            self.input = Editor::new("");
            self.edit_armed = true;
        }
        handler::replace_and_mark(self, r, text, sel);
        cx.notify();
    }
    fn bounds_for_range(&mut self, _r: Range<usize>, bounds: Bounds<gpui::Pixels>,
                        _w: &mut Window, _cx: &mut Context<Self>)
        -> Option<Bounds<gpui::Pixels>> {
        // IME の候補窓は選択中のセルの下に出す
        Some(Bounds::new(
            gpui::point(
                bounds.origin.x
                    + px(HEAD_W + self.col_x(self.cursor.col) - self.col_x(self.view.col)),
                bounds.origin.y
                    + px(2.0 * ROW_H
                        + (self.view.row..self.cursor.row)
                            .map(|r| self.row_px(r))
                            .sum::<f32>()),
            ),
            size(px(self.col_px(self.cursor.col)), px(ROW_H)),
        ))
    }
    fn character_index_for_point(&mut self, _p: gpui::Point<gpui::Pixels>,
                                 _w: &mut Window, _cx: &mut Context<Self>) -> Option<usize> {
        None
    }
    fn text_length_utf16(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> Option<usize> {
        Some(handler::text_len_utf16(self))
    }
}

/// 入力ハンドラは paint のときに窓へ差す(GPUI の作法)。
struct InputSink { view: Entity<Calc> }
impl IntoElement for InputSink { type Element = Self; fn into_element(self) -> Self { self } }
/// マウスを載せたときの名札(本家のツールチップの形 — 黒地に白)
struct Tip(SharedString, f32);
impl gpui::Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        div().px_2().py_1().rounded_md()
            .bg(gpui::rgb(0x2B2F33)).text_color(gpui::rgb(0xF2F5F7))
            .text_size(gpui::px(self.1 * 11.0))
            .border_1().border_color(gpui::rgb(0x14161A))
            .shadow_md()
            .child(self.0.clone())
    }
}

impl gpui::Element for InputSink {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<gpui::ElementId> { None }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> { None }
    fn request_layout(&mut self, _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>, window: &mut Window, cx: &mut App)
        -> (gpui::LayoutId, ()) {
        let mut s = gpui::Style::default();
        // **格子の上に全面で重ねる。** 流れの中に置くと格子の右へ押し出され、
        // bounds が格子とずれてマウスが一切当たらなくなる(踏んで直した)
        s.position = gpui::Position::Absolute;
        s.inset.top = gpui::px(0.0).into();
        s.inset.left = gpui::px(0.0).into();
        s.size.width = gpui::relative(1.0).into();
        s.size.height = gpui::relative(1.0).into();
        (window.request_layout(s, [], cx), ())
    }
    fn prepaint(&mut self, _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>, _: Bounds<gpui::Pixels>,
        _: &mut (), _: &mut Window, _: &mut App) {}
    fn paint(&mut self, _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>, bounds: Bounds<gpui::Pixels>,
        _: &mut (), _: &mut (), window: &mut Window, cx: &mut App) {
        let focus = self.view.read(cx).focus.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.view.clone()), cx);
        // マウスは窓のレベルで受けて、座標からセルを逆算する(writer と同じ方式)。
        // セルごとのホバー判定に頼ると、ドラッグ中の移動を取り逃すことがある
        let view = self.view.clone();
        window.on_mouse_event(move |e: &gpui::MouseDownEvent, phase, _w, cx| {
            if phase != gpui::DispatchPhase::Bubble
                || e.button != gpui::MouseButton::Left
                || !bounds.contains(&e.position)
            {
                return;
            }
            let rel = e.position - bounds.origin;
            view.update(cx, |c, cx| {
                c.mouse_down_at(
                    f32::from(rel.x),
                    f32::from(rel.y),
                    e.modifiers.shift,
                    e.modifiers.control,
                    e.click_count,
                );
                cx.notify();
            });
        });
        let view = self.view.clone();
        window.on_mouse_event(move |e: &gpui::MouseMoveEvent, phase, _w, cx| {
            // ドラッグ中は格子の外でも受ける(端で選択が止まらないように、
            // 位置は格子の中のセルに丸められる)
            if phase != gpui::DispatchPhase::Bubble
                || e.pressed_button != Some(gpui::MouseButton::Left)
            {
                return;
            }
            let rel = e.position - bounds.origin;
            view.update(cx, |c, cx| {
                if c.shape_rot.is_some() {
                    c.shape_rotate_at(f32::from(rel.x), f32::from(rel.y), e.modifiers.shift);
                    cx.notify();
                } else if c.shape_drag.is_some() {
                    c.shape_drag_at(f32::from(rel.x), f32::from(rel.y));
                    cx.notify();
                } else if c.img_drag.is_some() {
                    c.image_drag_at(f32::from(rel.x), f32::from(rel.y));
                    cx.notify();
                } else if c.size_drag.is_some() {
                    c.size_drag_at(f32::from(rel.x), f32::from(rel.y));
                    cx.notify();
                } else if c.drag.is_some()
                    || c.head_drag.is_some()
                    || c.ink_cur.is_some()
                    || c.tool == Some(2)
                    // 関数の引数・式の直入力のセル掴み(範囲をなぞる)も
                    // ここを通す — この表に入れ忘れると「押せるのに伸びない」
                    // (writer で踏んだ罠)
                    || c.fn_args.as_ref().is_some_and(|a| a.pick_from.is_some())
                    || c.ref_pick.is_some()
                {
                    // 筆と消しゴムもここを通る(描きかけ・なぞり)
                    c.mouse_drag_at(f32::from(rel.x), f32::from(rel.y));
                    cx.notify();
                }
            });
        });
        let view = self.view.clone();
        window.on_mouse_event(move |e: &gpui::MouseUpEvent, phase, _w, cx| {
            if phase != gpui::DispatchPhase::Bubble || e.button != gpui::MouseButton::Left {
                return;
            }
            view.update(cx, |c, cx| {
                c.mouse_up();
                cx.notify();
            });
        });
        // 右クリックでメニュー
        let view = self.view.clone();
        window.on_mouse_event(move |e: &gpui::MouseDownEvent, phase, _w, cx| {
            if phase != gpui::DispatchPhase::Bubble
                || e.button != gpui::MouseButton::Right
                || !bounds.contains(&e.position)
            {
                return;
            }
            let rel = e.position - bounds.origin;
            view.update(cx, |c, cx| {
                c.right_click_at(f32::from(rel.x), f32::from(rel.y));
                cx.notify();
            });
        });
    }
}

impl Render for Calc {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 窓の大きさを控える(見える行数・列数がこれに追従する)
        self.view_w_px = f32::from(window.viewport_size().width);
        self.view_h_px = f32::from(window.viewport_size().height);
        // 画面の文字の大きさ(Ctrl+= / Ctrl+- 、表示タブ)。リボン・数式バー・
        // メニュー・見出し・状態行の文字と釦がこれに追従する。格子のズームとは別
        let us = self.ui_scale;
        if std::env::var_os("JO_SELFTEST").is_some() {
            // 実際に描画が走った証拠を残す(notify だけでは画面は変わらない —
            // これが止まってティックが続くなら、提示(present)の停止)
            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            eprintln!("render #{}", N.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        }
        // ---- 画面の額縁(デスクトップ版の形。writer と同じ構成) ----
        // 1段目 = クイックアクセス+ブック名(この行が窓の取っ手)。
        // 表計算の色は緑(デスクトップ版の app 色分けと同じ)。
        // 2段目 = 白地のタブ+現在地の緑の下線。右端に 🔍。
        // 下端 = ステータスバー(シートの耳+状態の文言+選択の生きた値)
        let (ready, all) = ribbon::progress(ribbon::calc_tabs());
        // 画面の明暗(インターフェイステーマ)。**セルは白のまま** —
        // 暗くするのは周り(帯・タブ・釦・見出し・耳)だけ
        let dk = self.dark;
        let th_bar = if dk { rgb(0x14432A) } else { rgb(0x1B6E3C) };
        let th_band = if dk { rgb(0x1B1E21) } else { rgb(0xFFFFFF) };
        let th_fg = if dk { rgb(0xCFD6DC) } else { rgb(0x444B52) };
        let th_gray = if dk { rgb(0x565D64) } else { rgb(0xB6BDC4) };
        let th_hover = if dk { rgb(0x2C333A) } else { rgb(0xEAF5EE) };
        let th_line = if dk { rgb(0x33383D) } else { rgb(0xE1E6EA) };
        let th_head = if dk { rgb(0x22262A) } else { rgb(0xEFF2F4) };
        let qa = |id: &'static str, icon: &'static str| {
            div().id(id).px_2().py_1().rounded_sm().cursor_pointer()
                .hover(move |s| s.bg(rgb(0x2E8B57)))
                .child(gpui::svg()
                    .path(SharedString::from(format!("icons/{icon}.svg")))
                    .size(px(us * 15.0)).text_color(rgb(0xE8F3EC)))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let title = self
            .path
            .as_ref()
            .and_then(|q| q.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| ui::t!("無題のブック").into());
        let winbtn = |id: &'static str, label: &'static str| {
            div().id(id).px_2p5().py_1().rounded_sm()
                .text_size(px(us * 12.0)).text_color(rgb(0xCFE6D8))
                .cursor_pointer()
                .hover(move |s| if id == "close" { s.bg(rgb(0xC0392B)).text_color(rgb(0xFFFFFF)) }
                                else { s.bg(rgb(0x2E8B57)).text_color(rgb(0xFFFFFF)) })
                .child(label)
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let top = div().id("titlebar").flex().flex_row().items_center().gap_0p5()
            .px_2().py_0p5().bg(th_bar)
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                |_, e: &gpui::MouseDownEvent, window, _| {
                    if e.click_count >= 2 {
                        window.zoom_window();
                    } else {
                        window.start_window_move();
                    }
                }))
            .child(qa("qa-save", "save").on_click(cx.listener(|this, _, _, cx| {
                this.run_cmd("save", cx);
                cx.notify()
            })))
            .child(qa("qa-print", "print").on_click(cx.listener(|this, _, _, cx| {
                this.run_cmd("pdf", cx);
                cx.notify()
            })))
            .child(qa("qa-undo", "undo").on_click(cx.listener(|this, _, _, cx| {
                this.run_cmd("undo", cx);
                cx.notify()
            })))
            .child(qa("qa-redo", "redo").on_click(cx.listener(|this, _, _, cx| {
                this.run_cmd("redo", cx);
                cx.notify()
            })))
            .child(div().flex_1())
            .child(div().text_size(px(us * 12.5)).text_color(rgb(0xFFFFFF))
                .whitespace_nowrap().overflow_hidden()
                .child(SharedString::from(format!(
                    "{}{title}",
                    if self.dirty { "*" } else { "" }
                ))))
            .child(div().flex_1())
            .child(div().pr_2().text_size(px(us * 10.5)).text_color(rgb(0x9CC9AF))
                .child(SharedString::from(ui::tf!("calc — 実装済み {}/{}", ready, all))))
            .child(winbtn("min", "─").on_click(cx.listener(|_, _, window, _| {
                window.minimize_window();
            })))
            .child(winbtn("max", "▢").on_click(cx.listener(|_, _, window, _| {
                window.zoom_window();
            })))
            .child(winbtn("close", "✕").on_click(cx.listener(|this, _, _, cx| {
                this.request_quit(cx);
            })));

        // ピボットテーブル・表のデザインは**文脈タブ**(本家 Toolbar.js の
        // _state.inpivot / intabledesign と同じ) — カーソルがピボット/
        // テーブルの上にあるときだけタブ行に現れる。常設にしない
        let on_pivot = self.pivot_at(self.cursor).is_some();
        let in_table = self.sheet().tables.iter().any(|t| t.contains(self.cursor));
        // タブは名前でなく中身の id で見分ける(名前は言語で変わる)
        let ctx_hidden = |tb: &ribbon::Tab| {
            (tb.cmds.iter().any(|c| c.id == "pivot-layout") && !on_pivot)
                || (tb.cmds.iter().any(|c| c.id == "td-header") && !in_table)
        };
        // 開いていたタブの文脈が消えたら、前のタブへ戻る(本家と同じ挙動)
        if ctx_hidden(&ribbon::calc_tabs()[self.tab]) {
            self.tab = if ctx_hidden(&ribbon::calc_tabs()[self.prev_tab]) {
                1 // ホーム
            } else {
                self.prev_tab
            };
        }
        let mut tabs = div().flex().flex_row().items_end().gap_1()
            .px_2().bg(th_band);
        for (i, tb) in ribbon::calc_tabs().iter().enumerate() {
            if ctx_hidden(tb) {
                continue;
            }
            let on = i == self.tab;
            // 文脈タブ(ピボット・表のデザイン)は色を付けて目に留める —
            // 出たり消えたりするものは、出た瞬間に分からないと意味がない
            let is_ctx = tb.cmds.iter()
                .any(|c| c.id == "pivot-layout" || c.id == "td-header");
            tabs = tabs.child(div()
                .id(SharedString::from(format!("tab{i}")))
                .px_2p5().pt_1p5()
                .when(is_ctx, |d| d.bg(rgb(0xF3EDFB)).rounded_t_md())
                .text_size(px(us * 12.0))
                .text_color(if is_ctx { rgb(0x8A63C9) } else if on { rgb(0x2E8B57) } else { th_fg })
                .font_weight(if on { gpui::FontWeight::BOLD } else { gpui::FontWeight::NORMAL })
                .cursor_pointer()
                .hover(|s| s.text_color(rgb(0x1B6E3C)))
                .flex().flex_col().items_center().gap_1()
                .child(tb.name)
                // 現在地の緑の下線(デスクトップ版の形)
                .child(div().h(px(2.5)).w_full().rounded_sm()
                    .bg(if on && is_ctx { rgb(0x8A63C9) }
                        else if on { rgb(0x2E8B57) }
                        else if is_ctx { rgb(0xF3EDFB) }
                        else { th_band }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.tab != 0 {
                        this.prev_tab = this.tab;
                    }
                    this.tab = i;
                    cx.notify()
                })));
        }
        tabs = tabs.child(div().flex_1())
            .child(div().id("tab-find").px_2().pb_1().text_size(px(us * 12.0))
                .text_color(rgb(0x555E66)).cursor_pointer()
                .hover(|s| s.text_color(rgb(0x1B6E3C)))
                .child("🔍")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.run_cmd("replace", cx);
                    cx.notify()
                })));

        // 釦の帯: 本家のデスクトップ版の一段の絵釦(writer の写し)。
        // 主要な釦は名札つきの大釦、他は絵だけ(乗ると名前が下のステータス
        // バーへ)。絵の無い釦は小さな文字の釦。ホームだけ2段(釦が多い)
        const BIG: &[(&str, &str)] = &[
            ("instable", "表"), ("insimage-c", "画像"), ("insshape", "図形"),
            ("inschart", "グラフ"), ("inssmartart", "SmartArt"),
            ("autosum", "オートSUM"), ("recent", "最近使った関数"),
            ("pagemargins", "余白"), ("pageorient", "向き"), ("pagesize", "サイズ"),
            ("printarea", "印刷範囲"),
            ("data-from-text", "テキストから"), ("custom-sort", "並べ替え"),
            ("setfilter", "フィルター"), ("python", "Python"),
            ("subtotal", "小計"), ("solver", "ソルバー"), ("group", "グループ化"),
            ("pivot-insert", "ピボットテーブル"),
            ("td-header", "ヘッダー行"), ("td-total", "合計行"),
            ("coauth-mode", "共同編集モード"), ("co-addcomment", "コメント"),
            ("co-chat", "チャット"), ("co-history", "バージョン履歴"),
            ("prot-encrypt", "暗号化"), ("prot-sign", "署名"), ("prot-doc", "保護"),
            ("freeze", "枠の固定"), ("pen", "ペン"), ("highlighter", "蛍光ペン"),
            ("eraser", "消しゴム"),
            ("plug-macros", "マクロ"), ("plug-manage", "プラグインの管理"),
        ];
        let th_cmd_border = th_line;
        let th_btn_hover = th_hover;
        let mut cmds = div().flex().flex_col().gap_0p5()
            .px_3().py_1().bg(th_band)
            .border_b_1().border_color(th_cmd_border);
        let items = ribbon::calc_tabs()[self.tab].cmds;
        // 今のセルの書体と大きさ(ホームの欄に出す — 本家はコンボボックスで
        // **今の値が見える**。slot-field-fontname/fontsize)
        let cur_fmt = self.sheet().get(self.cursor).map(|c| c.fmt.clone()).unwrap_or_default();
        let cur_font: SharedString = cur_fmt.font.clone()
            .unwrap_or_else(|| "Noto Sans JP".into()).into();
        let cur_size: SharedString = {
            let pt = cur_fmt.size_c.map(|c| c as f32 / 100.0).unwrap_or(11.0);
            if (pt - pt.round()).abs() < 0.05 {
                format!("{}", pt.round() as u32).into()
            } else {
                format!("{pt:.1}").into()
            }
        };
        // 1つの釦を組み立てる(名札つきの大釦 / 絵だけ / 文字の小釦)。
        // ホームの対の並びと、他タブの一段の並びの両方から使う
        // 絵の無い釦の**短い札**(ja)。長い正式名はツールチップと状態行へ。
        // 場所を食う文字の釦を細くする(発注者 2026-08-07)
        const SHORT: &[(&str, &str)] = &[
            ("fillparag", "塗り"), ("text-orient", "向き"),
            ("clear-filter", "解除"), ("format", "書式"),
            ("digit-dec", "桁−"), ("digit-inc", "桁+"),
            ("cell-format", "スタイル"), ("table-tpl", "表↧"),
            ("inscheckbox", "チェック"), ("insrecommend", "おすすめ"),
            ("inshyperlink", "リンク"), ("rtl-sheet", "右から"),
            ("print-gridlines", "枠線"), ("print-headings", "見出し"),
            ("defname", "名前"), ("trace-prec", "参照元"),
            ("trace-dep", "参照先"), ("remove-arrows", "矢印消"),
            ("data-from-text", "テキスト"), ("data-external-links", "外部リンク"),
            ("ungroup", "解除"), ("show-details", "詳細+"),
            ("hide-details", "詳細−"), ("pivot-fields", "フィールド"),
            ("pivot-refresh-all", "全更新"), ("pivot-select", "選択"),
            ("pivot-layout", "レイアウト"), ("td-band-row", "縞(行)"),
            ("td-first", "先頭列"), ("td-last", "末尾列"),
            ("td-band-col", "縞(列)"), ("td-filter", "▼釦"),
            ("rem-duplicates", "重複"), ("td-torange", "範囲へ"),
            ("td-resize", "サイズ"), ("sheet-view", "表示"),
            ("ui-bigger", "字を大"), ("ui-smaller", "字を小"),
            ("theme", "テーマ"), ("freeze", "固定"),
            ("show-gridlines", "枠線"), ("show-headings", "見出し"),
            ("show-zeros", "0表示"),
        ];
        // 一覧・板・小窓が開く釦(本家は ▼ を添える)。id で見分ける
        const DROP_IDS: &[&str] = &[
            "fontname", "fontsize", "changecase", "format", "cell-format",
            "borders", "fontcolor", "fillparag", "freeze", "clear",
            "data-validation", "custom-sort", "condformat", "numfmt",
            "theme", "colorschemas", "pagesize", "pageorient", "pagemargins",
            "insshape", "inssmartart", "instable", "table-tpl", "inssymbol",
            "inschart", "pivot-insert", "pivot-fields", "pivot-style",
            "text-orient", "insert-function", "fn-math", "fn-text",
            "fn-logical", "fn-datetime", "fn-lookup", "fn-financial", "fn-more",
            "fn-recent", "sheet-view", "cell-styles",
        ];
        let mk_btn = |cmd: &ribbon::Cmd, cx: &mut Context<Self>| -> gpui::AnyElement {
            let label = cmd.label;
            let icon = cmd.icon;
            // 書体と大きさは釦でなく**欄**(本家の形): 今の値を枠の中に見せ、
            // 押すと一覧が開く
            if cmd.id == "fontname" || cmd.id == "fontsize" {
                let (w, val) = if cmd.id == "fontname" {
                    (110.0, cur_font.clone())
                } else {
                    (38.0, cur_size.clone())
                };
                let cid = cmd.id;
                let hoverable = cx.listener(move |this: &mut Calc, on: &bool, _, cx| {
                    if *on {
                        this.hover_hint = Some(label);
                    } else if this.hover_hint == Some(label) {
                        this.hover_hint = None;
                    }
                    cx.notify()
                });
                return div().id(SharedString::from(format!("h-{icon}")))
                    .w(px(us * w)).h(px(us * 22.0)).px_1p5().rounded_sm()
                    .border_1().border_color(th_line)
                    .flex().items_center()
                    .text_size(px(us * 10.5)).text_color(th_fg)
                    .whitespace_nowrap().overflow_hidden()
                    .on_hover(hoverable)
                    .tooltip(move |_, cx| cx.new(|_| Tip(label.into(), us)).into())
                    .cursor_pointer().hover(move |st| st.bg(th_btn_hover))
                    .child(val)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.run_cmd(cid, cx);
                        cx.notify()
                    }))
                    .into_any_element();
            }
            let has_icon = ui::icons::find(icon).is_some();
            let big = BIG.iter().find(|(k, _)| *k == icon).map(|(_, s)| *s);
            // 名札の短い形は ja 向け — 他の言語では表の語を使う
            let big = if ui::settings::language() == "ja" {
                big
            } else {
                big.map(|_| cmd.label)
            };
            let hoverable = cx.listener(move |this: &mut Calc, on: &bool, _, cx| {
                if *on {
                    this.hover_hint = Some(label);
                } else if this.hover_hint == Some(label) {
                    this.hover_hint = None;
                }
                cx.notify()
            });
            // ピボットの上で締める釦は灰色に(本家の editPivot ロック。
            // 押しても run_cmd 側で断るが、見た目でも先に伝える)
            let locked = on_pivot && Calc::PIVOT_LOCKED.contains(&cmd.id);
            let fg = if cmd.ready && !locked { th_fg } else { th_gray };
            let drops = DROP_IDS.contains(&cmd.id);
            if let Some(short) = big {
                // 名札つきの大釦(絵の下に短い名前 — 本家の言い方)。
                // 一覧の開く釦は名札の横に小さな ▾
                let mut b = div().id(SharedString::from(format!("h-{icon}")))
                    .px_2().h(px(us * 46.0)).rounded_sm()
                    .flex().flex_col().items_center().justify_center().gap_1()
                    .on_hover(hoverable)
                    .tooltip(move |_, cx| cx.new(|_| Tip(label.into(), us)).into())
                    .children(has_icon.then(|| {
                        gpui::svg()
                            .path(SharedString::from(format!("icons/{icon}.svg")))
                            .size(px(us * 20.0)).text_color(fg)
                    }))
                    .child(div().flex().flex_row().items_center().gap_0p5()
                        .text_size(px(us * 10.5)).text_color(fg)
                        .child(short)
                        .children(drops.then(|| div()
                            .text_size(px(us * 8.0)).text_color(th_gray)
                            .child("▾"))));
                if cmd.ready {
                    let cid = cmd.id;
                    b = b.cursor_pointer().hover(move |st| st.bg(th_btn_hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.run_cmd(cid, cx);
                            cx.notify()
                        }));
                }
                return b.into_any_element();
            }
            let mut b = div().id(SharedString::from(format!("h-{icon}")))
                .h(px(us * 26.0)).rounded_sm()
                .flex().items_center().justify_center()
                .on_hover(hoverable)
                .tooltip(move |_, cx| cx.new(|_| Tip(label.into(), us)).into());
            b = if has_icon {
                if drops { b.px_0p5() } else { b.w(px(us * 26.0)) }
            } else {
                b.px_1p5()
            };
            b = b
                .children(has_icon.then(|| {
                    gpui::svg()
                        .path(SharedString::from(format!("icons/{icon}.svg")))
                        .size(px(us * 18.0)).text_color(fg)
                }))
                .children((has_icon && drops).then(|| {
                    // 一覧が開く印(本家の ▼)
                    div().text_size(px(us * 8.0)).text_color(th_gray).child("▾")
                }))
                .children((!has_icon).then(|| {
                    // 短い札があればそちら(正式名はツールチップと状態行)。
                    // 短縮は ja だけ — 他の言語は表の語のまま
                    let text = if ui::settings::language() == "ja" {
                        SHORT
                            .iter()
                            .find(|(k, _)| *k == cmd.id)
                            .map(|(_, v)| *v)
                            .unwrap_or(label)
                    } else {
                        label
                    };
                    div().text_size(px(us * 10.5)).text_color(fg)
                        .flex().flex_row().items_center().gap_0p5()
                        .child(text)
                        .children(drops.then(|| div()
                            .text_size(px(us * 8.0)).text_color(th_gray)
                            .child("▾")))
                }));
            if cmd.ready {
                let cid = cmd.id;
                b = b.cursor_pointer().hover(move |st| st.bg(th_btn_hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.run_cmd(cid, cx);
                        cx.notify()
                    }));
            }
            b.into_any_element()
        };
        if ribbon::CALC[self.tab].name == "ホーム" {
            // 本家のホームは**単純な2行割りではない**(発注者 2026-08-06
            // スクショ)。組ごとに上の段と下の段が対になっている —
            // コピーの下に貼り付け、書体の下に B I U…、縦揃えの下に横揃え。
            // その対をそのまま書き、組の間に縦の区切り線を引く
            const HOME_PAIRS: &[(&[&str], &[&str])] = &[
                (&["copy", "cut"], &["paste", "copystyle"]),
                (&["fontname", "fontsize", "incfont", "decfont", "changecase"],
                 &["bold", "italic", "underline", "strikeout", "subscript",
                   "fontcolor", "fillparag", "borders"]),
                (&["top", "middle", "bottom", "wrap", "text-orient"],
                 &["align-left", "align-center", "align-right", "align-just",
                   "merge", "direction"]),
                (&["insert-function", "fill-num"], &["defname", "clear"]),
                (&["sort-desc", "sort-asc"], &["setfilter", "clear-filter"]),
                (&["format", "currency", "percents"],
                 &["comma", "digit-dec", "digit-inc"]),
                (&["cell-ins", "cell-del", "cell-format"],
                 &["condformat", "table-tpl", "cell-styles"]),
                (&["replace"], &["selectall"]),
            ];
            let mut used: std::collections::HashSet<&str> = Default::default();
            let mut band = div().flex().flex_row().items_center().gap_1();
            let mut first = true;
            for (topr, botr) in HOME_PAIRS {
                if topr.iter().chain(botr.iter())
                    .all(|id| !items.iter().any(|c| c.id == *id))
                {
                    continue; // 表に無い組は出さない(将来の並び替えでも落ちない)
                }
                if !first {
                    band = band.child(div().w(px(1.0)).h(px(us * 46.0))
                        .bg(th_cmd_border).mx_1());
                }
                first = false;
                let mut col = div().flex().flex_col().gap_0p5();
                for ids in [*topr, *botr] {
                    let mut r = div().flex().flex_row().items_center()
                        .gap_0p5().h(px(us * 26.0));
                    for id in ids {
                        if let Some(cmd) = items.iter().find(|c| c.id == *id) {
                            used.insert(cmd.id);
                            r = r.child(mk_btn(cmd, cx));
                        }
                    }
                    col = col.child(r);
                }
                band = band.child(col);
            }
            // 対の表に無い釦も**黙って落とさない** — 右端に半々で足す
            let rest: Vec<&ribbon::Cmd> =
                items.iter().filter(|c| !used.contains(c.id)).collect();
            if !rest.is_empty() {
                band = band.child(div().w(px(1.0)).h(px(us * 46.0))
                    .bg(th_cmd_border).mx_1());
                let half = rest.len().div_ceil(2);
                let mut col = div().flex().flex_col().gap_0p5();
                for chunk in rest.chunks(half.max(1)) {
                    let mut r = div().flex().flex_row().items_center()
                        .gap_0p5().h(px(us * 26.0));
                    for cmd in chunk {
                        r = r.child(mk_btn(cmd, cx));
                    }
                    col = col.child(r);
                }
                band = band.child(col);
            }
            cmds = cmds.child(band);
        } else {
            let mut row = div().flex().flex_row().items_center().gap_0p5();
            for cmd in items {
                row = row.child(mk_btn(cmd, cx));
            }
            cmds = cmds.child(row);
        }
        let bar = if self.tab == 0 {
            // ファイルの全面ページは釦の帯を持たない(本家の形)
            div().flex().flex_col().child(top).child(tabs)
        } else {
            div().flex().flex_col().child(top).child(tabs).child(cmds)
        };

        // ---- 数式バー ----
        // クリックで**編集モード**(発注者 2026-08-06)— 置き換えでなく、
        // 押した位置に文字カーソルを立てて続きを直せる。編集中はキャレットを見せる
        let in_edit = self.editing() || self.edit_armed;
        let bar_text = {
            let mut t = self.input.text().to_string();
            if in_edit {
                let cur = self.input.cursor().min(t.len());
                t.insert(cur, '|');
            }
            if t.is_empty() { " ".to_string() } else { t }
        };
        // 名前ボックス(左端): 押すと打てる。番地・範囲・名前で飛び、
        // 知らない名前ならいまの選択に付ける(Excel の名前ボックス)
        let name_box = if let Some(ed) = &self.name_edit {
            let mut t = ed.text().to_string();
            let cur = ed.cursor().min(t.len());
            t.insert(cur, '|');
            div().w(px(88.0)).px_1().py_0p5().bg(gpui::white())
                .border_1().border_color(rgb(0x1B6E3C)).rounded_sm()
                .text_size(px(us * 12.0)).whitespace_nowrap().overflow_hidden()
                .child(SharedString::from(t))
        } else {
            div().w(px(88.0)).px_1().py_0p5()
                .border_1().border_color(rgb(0xC6CDD3)).rounded_sm()
                .text_size(px(us * 12.0))
                .font_weight(gpui::FontWeight::BOLD).text_color(rgb(0x1B6E3C))
                .cursor_text()
                .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.name_edit = Some(Editor::new(""));
                    this.status = ui::t!(
                        "名前ボックス: 番地(B12)・範囲(A1:C9)・名前で移動。\
                         知らない名前は選択に付きます")
                    .into();
                    cx.notify();
                }))
                .child(SharedString::from(if self.book.r1c1 {
                    format!("R{}C{}", self.cursor.row + 1, self.cursor.col + 1)
                } else {
                    self.cursor.a1()
                }))
        };
        let formula_bar = div()
            .flex().flex_row().items_center().gap_2()
            .px_4().py_1p5().bg(rgb(0xFAFBFC))
            .border_b_1().border_color(rgb(0xE1E6EA))
            .child(name_box)
            // fx = 関数を挿入(本家と同じ場所)。幅は固定 —
            // 数式編集のクリック位置の換算(下の 156px)が崩れないように
            .child(div().id("fx").w(px(28.0)).py_0p5().rounded_sm()
                   .flex().items_center().justify_center()
                   .text_size(px(us * 13.0)).italic()
                   .font_weight(gpui::FontWeight::BOLD).text_color(rgb(0x1B6E3C))
                   .cursor_pointer().hover(|s| s.bg(rgb(0xE4EFE8)))
                   .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                       cx.stop_propagation();
                       this.fn_dlg = Some(FnDlg {
                           search: Editor::new(""),
                           group: 0,
                           sel: 0,
                       });
                       cx.notify();
                   }))
                   .child("fx"))
            .child(div().flex_1().px_2().py_1().bg(gpui::white())
                   .border_1().border_color(if in_edit { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                   .rounded_sm()
                   .text_size(px(us * 13.0)).font_family(self.font_name.clone())
                   .cursor_text()
                   .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                       |this, e: &gpui::MouseDownEvent, _, cx| {
                           cx.stop_propagation();
                           // 押した位置へ文字カーソル(幅は 全角=1em・半角=0.5em の見積り)。
                           // 起点 = 左余白16 + 名前ボックス88 + 隙間8 + fx 28 + 隙間8 + 内余白8
                           let x = f32::from(e.position.x)
                               - (16.0 + 88.0 + 8.0 + 28.0 + 8.0 + 8.0);
                           let text = this.input.text().to_string();
                           let mut acc = 0.0;
                           let mut at = text.len();
                           // 文字幅は「画面の文字の大きさ」に追従(13px × ui_scale)。
                           // 倍率を掛け忘れるとクリックとカーソルがずれる(発注者 2026-08-07)
                           let uscale = this.ui_scale;
                           for (i, ch) in text.char_indices() {
                               let w = uscale
                                   * if (ch as u32) < 0x2E80 { 6.8 } else { 13.0 };
                               if acc + w / 2.0 > x {
                                   at = i;
                                   break;
                               }
                               acc += w;
                           }
                           this.input.move_to(at, false);
                           this.edit_armed = true;
                           this.status =
                               ui::t!("数式バーで編集: Enter で確定 / Esc で取消").into();
                           cx.notify();
                       }))
                   .child(SharedString::from(bar_text)));

        // ---- 折り返しの無い文字の、隣の空セルへのはみ出し(Excel の流儀) ----
        // 折り返し・縮小・回転・右横書きでない文字のセルで、伸びる方向の
        // 隣が空(値も式も無い)なら、そのセルの上にも描く(発注者 2026-08-06)。
        // 描くのは格子の後の重ね描き(spill_texts)で、セル側は文字を出さない
        let vis_cols: Vec<u32> = self.visible_cols();
        // 条件付き書式の下ごしらえ(重複・上位N・平均は範囲の統計が要る —
        // セルごとに範囲を歩かない)
        let cond_prep: Vec<(sheet::model::CondRule, sheet::model::CondAux)> = self
            .sheet()
            .cond
            .iter()
            .map(|r| (r.clone(), r.aux(self.sheet())))
            .collect();
        let mut spill_from: std::collections::HashSet<Pos> = Default::default();
        let mut spill_texts: Vec<gpui::Div> = Vec::new();
        if !self.show_formulas {
            let mut y = ROW_H;
            for r in self.visible_rows() {
                let rh = self.row_px(r);
                let mut x = HEAD_W;
                for (ci, &c) in vis_cols.iter().enumerate() {
                    let w = self.col_px(c);
                    let p = Pos::new(r, c);
                    let x0 = x;
                    x += w;
                    if p == self.cursor {
                        continue; // 編集中の見た目は従来どおり
                    }
                    let Some(cl) = self.sheet().get(p) else { continue };
                    let Value::Text(t) = &cl.value else { continue };
                    if t.is_empty() {
                        continue;
                    }
                    let f = &cl.fmt;
                    if f.wrap || f.shrink || f.rtl_text
                        || f.rotation.is_some_and(|r| r != 0)
                    {
                        continue;
                    }
                    if self.sheet().covered_by_merge(p)
                        || self.sheet().merges.iter().any(|(a, _)| *a == p)
                    {
                        continue;
                    }
                    let to_left = match f.align {
                        HAlign::Right => true,
                        HAlign::Left | HAlign::General => false,
                        _ => continue, // 中央・両端揃えは流さない
                    };
                    let t1 = t.replace('\n', " ");
                    let size = self.zoom
                        * f.size_c
                            .map(|c| c as f32 / 100.0 * 24.0 / 15.0 * 0.8)
                            .unwrap_or(12.5);
                    let units: f32 = t1
                        .chars()
                        .map(|ch| if (ch as u32) < 0x2E80 { 1.0 } else { 2.0 })
                        .sum();
                    let need = units * size * 0.52 + 14.0;
                    if need <= w {
                        continue; // 収まっている
                    }
                    // 伸びる方向の空きセルぶんだけ許す
                    let (mut avail, mut left_ext, mut k) = (w, 0.0f32, ci);
                    loop {
                        if need <= avail {
                            break;
                        }
                        let nk = if to_left {
                            k.checked_sub(1)
                        } else {
                            (k + 1 < vis_cols.len()).then_some(k + 1)
                        };
                        let Some(nk) = nk else { break };
                        let nc = vis_cols[nk];
                        let np = Pos::new(r, nc);
                        let occupied = self
                            .sheet()
                            .get(np)
                            .is_some_and(|q| !q.value.is_empty() || q.formula.is_some())
                            || self.sheet().covered_by_merge(np)
                            || np == self.cursor;
                        if occupied {
                            break;
                        }
                        let nw = self.col_px(nc);
                        avail += nw;
                        if to_left {
                            left_ext += nw;
                        }
                        k = nk;
                    }
                    if avail <= w {
                        continue; // 隣が塞がっている — 今までどおり切る
                    }
                    spill_from.insert(p);
                    let wd = avail.min(need);
                    let lx = if to_left { x0 + w - wd } else { x0 };
                    let _ = left_ext;
                    let mut d = div().absolute()
                        .left(px(lx)).top(px(y))
                        .w(px(wd)).h(px(rh))
                        .px_1p5().flex()
                        .text_size(px(size))
                        .font_family(self.font_name.clone())
                        .whitespace_nowrap().overflow_hidden();
                    match f.valign {
                        sheet::model::VAlign::Top => d = d.items_start(),
                        sheet::model::VAlign::Middle => d = d.items_center(),
                        sheet::model::VAlign::Bottom => d = d.items_end(),
                    }
                    d = if to_left { d.justify_end() } else { d.justify_start() };
                    if f.bold {
                        d = d.font_weight(gpui::FontWeight::BOLD);
                    }
                    if f.italic {
                        d = d.italic();
                    }
                    if f.underline {
                        d = d.underline();
                    }
                    if f.strike {
                        d = d.line_through();
                    }
                    d = if let Some(cv) = &f.color {
                        d.text_color(hex(cv))
                    } else {
                        d.text_color(rgb(0x1B1B1B))
                    };
                    if let Some(name) = &f.font {
                        if let Ok((fam, _)) = kumihan::font::for_document(Some(name)) {
                            d = d.font_family(SharedString::from(fam.name.clone()));
                        }
                    }
                    spill_texts.push(d.child(SharedString::from(t1)));
                }
                y += rh;
            }
        }

        // ---- 格子 ----
        let mut grid = div().flex().flex_col();
        // 列見出し
        // 見出しもセルも flex_none — **窓の大きさで伸縮させない**
        // (窓に合わせるのは見える範囲。セルの大きさは設定どおり固定)
        let mut head = div().flex().flex_row().flex_none()
            .child(div().flex_none().w(px(HEAD_W)).h(px(ROW_H)).bg(th_head)
                   .border_r_1().border_b_1().border_color(rgb(0xD5DBE0)));
        let (sel_a, sel_b) = self.sel_rect();
        let has_sel = self.anchor.is_some();
        for c in self.visible_cols() {
            // 選択に入っている列の見出しは色を変える(いまどこを選んでいるかの道標)
            let on = has_sel && (sel_a.col..=sel_b.col).contains(&c) || c == self.cursor.col;
            head = head.child(div().flex_none().w(px(self.col_px(c))).h(px(ROW_H))
                .bg(if on { rgb(0xCFE6D8) } else { th_head })
                .border_r_1().border_b_1()
                .border_color(rgb(0xD5DBE0))
                .flex().items_center().justify_center()
                .text_size(px(us * 11.5))
                .text_color(if on { rgb(0x1B6E3C) } else if dk { rgb(0x9AA5AE) } else { rgb(0x66707A) })
                .child(SharedString::from(if self.book.r1c1 {
                    (c + 1).to_string()
                } else {
                    col_name(c)
                }))
                // 右端の帯は幅を変える取っ手(カーソル形状の誘いだけ。
                // 当たり判定は InputSink の窓レベルで size_grip_at がやる)
                .relative().children((std::env::var_os("JO_NO_STRIPS").is_none()).then(|| {
                    div().absolute()
                        .top(px(0.0)).right(px(-GRIP)).w(px(GRIP * 2.0)).h_full()
                        .cursor_col_resize()
                })));
        }
        grid = grid.child(head);

        // 当たり判定(cell_at)と同じ並びを使う — ずれるとクリックが別のセルに入る
        let visible: Vec<u32> = self.visible_rows();
        for r in visible {
            let rh = self.row_px(r);
            let row_on = has_sel && (sel_a.row..=sel_b.row).contains(&r) || r == self.cursor.row;
            // 絞り込みで残った行の番号は青(Excel の作法 — 絞り込み中と一目で分かる)
            let filtered_blue = self.filter_active()
                && self.auto_filter.as_ref().is_some_and(|f| {
                    r > f.range.0.row && r <= f.range.1.row
                });
            let mut row = div().flex().flex_row().flex_none()
                .child(div().flex_none().w(px(HEAD_W)).h(px(rh))
                    .bg(if row_on { rgb(0xCFE6D8) } else { th_head })
                    .border_r_1().border_b_1()
                    .border_color(rgb(0xD5DBE0))
                    .flex().items_center().justify_center()
                    .text_size(px(us * 11.5))
                    .text_color(if row_on { rgb(0x1B6E3C) } else if filtered_blue { rgb(0x1B6EC2) } else if dk { rgb(0x9AA5AE) } else { rgb(0x66707A) })
                    .child(SharedString::from((r + 1).to_string()))
                    // 下端の帯は高さを変える取っ手(列見出しの右端と同じ仕掛け)
                    .relative().children((std::env::var_os("JO_NO_STRIPS").is_none()).then(|| {
                        div().absolute()
                            .left(px(0.0)).bottom(px(-GRIP)).w_full().h(px(GRIP * 2.0))
                            .cursor_row_resize()
                    }))
                    // グループ化の +/-(アウトラインの縁)。直前で終わる
                    // かたまりの頭金の行に置く(Excel の「集計行が下」の形)
                    .children({
                        let sh = self.sheet();
                        r.checked_sub(1).and_then(|pr| {
                            let lv = *sh.row_outline.get(&pr).unwrap_or(&0);
                            // かたまりが r の直前で**終わっている**ときだけ
                            // (続きの行に印を出さない)
                            if lv == 0 || *sh.row_outline.get(&r).unwrap_or(&0) >= lv {
                                return None;
                            }
                            let mut start = pr;
                            while start > 0
                                && *sh.row_outline.get(&(start - 1)).unwrap_or(&0) >= lv
                            {
                                start -= 1;
                            }
                            let hidden = sh.row_hidden.contains(&pr);
                            Some(div()
                                .id(SharedString::from(format!("gut{r}")))
                                .absolute().left(px(1.0)).top(px((rh - 11.0) / 2.0))
                                .w(px(11.0)).h(px(11.0)).rounded_sm()
                                .border_1().border_color(rgb(0x8FA3AE))
                                .bg(gpui::white())
                                .flex().items_center().justify_center()
                                .text_size(px(us * 9.0)).text_color(rgb(0x1B6E3C))
                                .cursor_pointer()
                                .hover(|s| s.bg(rgb(0xEAF5EE)))
                                .child(if hidden { "+" } else { "−" })
                                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation()
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.checkpoint();
                                    for i in start..=pr {
                                        if hidden {
                                            this.sheet_mut().row_hidden.remove(&i);
                                        } else {
                                            this.sheet_mut().row_hidden.insert(i);
                                        }
                                    }
                                    this.dirty = true;
                                    this.status = if hidden {
                                        ui::t!("詳細を表示しました(+/− でいつでも)").into()
                                    } else {
                                        ui::t!("詳細を畳みました(+ で開きます)").into()
                                    };
                                    cx.notify()
                                })))
                        })
                    }));
            for c in self.visible_cols() {
                let p = Pos::new(r, c);
                let cell = self.sheet().get(p);
                // 結合に呑まれた位置は空で描く(値は左上のセルにだけある)
                let v = if self.sheet().covered_by_merge(p) { Value::Empty }
                        else { cell.map(|x| x.value.clone()).unwrap_or(Value::Empty) };
                // 付けた表示形式は画面に出す。出ないなら飾りでしかない
                let shown = if self.show_formulas {
                    // 数式の表示。式が無いセルは値のまま
                    cell.and_then(|x| x.formula.clone())
                        .map(|f| if self.book.r1c1 {
                            format!("={}", sheet::model::formula_to_r1c1(&f, p))
                        } else {
                            format!("={f}")
                        })
                        .unwrap_or_else(|| sheet::model::format_value(&v,
                            cell.and_then(|x| x.fmt.number_format.as_deref())))
                } else {
                    sheet::model::format_value(&v, cell.and_then(|x| x.fmt.number_format.as_deref()))
                };
                // Bool のセルはチェックボックスとして見せる(☑/☐。
                // 空白キーで切替。Excel では TRUE/FALSE の値で見える)
                let shown = match v {
                    Value::Bool(b) if !self.show_formulas => {
                        if b { "☑".to_string() } else { "☐".to_string() }
                    }
                    _ => shown,
                };
                let shown = if !self.show_zeros && matches!(v, Value::Number(n) if n == 0.0) {
                    String::new()
                } else {
                    shown
                };
                let is_num = matches!(v, Value::Number(_));
                let is_err = matches!(v, Value::Error(_));
                let sel = p == self.cursor;
                let (ra, rb) = self.sel_rect();
                let in_range = self.anchor.is_some()
                    && (ra.row..=rb.row).contains(&r) && (ra.col..=rb.col).contains(&c);
                // 結合の中では、結合の外周にあたる辺だけ格子線を引く —
                // 中の線は**引かない**(重ね描きに頼らず、線の源で消す。
                // 発注者報告 2026-08-08「元のセルの枠線が残っている」)
                let in_merge = self
                    .sheet()
                    .merges
                    .iter()
                    .copied()
                    .find(|(ma, mb)| {
                        (ma.row..=mb.row).contains(&r) && (ma.col..=mb.col).contains(&c)
                    });
                let (grid_r, grid_b) = match in_merge {
                    Some((_, mb)) => (c == mb.col, r == mb.row),
                    None => (true, true),
                };
                let mut d = div()
                    .id(SharedString::from(p.a1()))
                    .flex_none()
                    .w(px(self.col_px(c))).h(px(rh))
                    .when(grid_r, |d| d.border_r_1())
                    .when(grid_b, |d| d.border_b_1())
                    .border_color(if self.gridlines { rgb(0xE1E6EA) } else { rgb(0xFFFFFF) })
                    .bg(rgb(0xFFFFFF))
                    .flex().items_center()
                    .px_1p5()
                    .text_size(px(self.zoom * cell.and_then(|x| x.fmt.size_c)
                        .map(|c| c as f32 / 100.0 * 24.0 / 15.0 * 0.8)
                        .unwrap_or(12.5)))
                    .font_family(self.font_name.clone())
                    .overflow_hidden().whitespace_nowrap()
                    // セルの上は Excel と同じ十字(手のひらだと「押す物」に見える)
                    .cursor(gpui::CursorStyle::Crosshair);
                // マウスの結線はセルではなく InputSink(窓レベル)にある。
                // セルの id は当たり判定ではなく描画の区別のためだけに残す
                // 罫線・塗り・文字書式。**帳票の見た目はここで決まる**
                let f = cell.map(|x| x.fmt.clone()).unwrap_or_default();
                let mut base = f.fill.as_deref().map(hex).unwrap_or(gpui::Rgba {
                    r: 1.0, g: 1.0, b: 1.0, a: 1.0,
                });
                // 条件付き書式。**付けた条件は画面に出す**(出ないなら飾り)
                let mut cond_color: Option<gpui::Rgba> = None;
                let mut cond_bar: Option<(f32, gpui::Rgba)> = None;
                let mut cond_icon: Option<(&'static str, gpui::Rgba)> = None;
                for (rule, aux) in &cond_prep {
                    if rule.hits(p, &v, aux) {
                        if let Some(fill) = &rule.fill {
                            base = hex(fill);
                        }
                        if let Some(c) = &rule.color {
                            cond_color = Some(hex(c));
                        }
                    }
                    // バー/スケール/アイコンは 0〜1 の物差しで描く
                    if let Some(t) = rule.scalar(p, &v, aux) {
                        use sheet::model::CondKind;
                        match &rule.kind {
                            CondKind::Bar(c) => cond_bar = Some((t as f32, hex(c))),
                            CondKind::Scale(..) => {
                                if let Some(c) = rule.scale_color(t) {
                                    base = hex(&c);
                                }
                            }
                            CondKind::Icons(name) => {
                                // 3段: 下 / 中 / 上。矢印系は ↓→↑、他は ●の信号色
                                let arrows = name.contains("Arrow");
                                cond_icon = Some(if t < 1.0 / 3.0 {
                                    (if arrows { "↓" } else { "●" }, hex("C62828"))
                                } else if t < 2.0 / 3.0 {
                                    (if arrows { "→" } else { "●" }, hex("E6A700"))
                                } else {
                                    (if arrows { "↑" } else { "●" }, hex("2E7D32"))
                                });
                            }
                            _ => {}
                        }
                    }
                }
                d = d.bg(base);
                // データバー(文字の下に敷く。子は後の文字が上に描かれる)
                if let Some((t, bc)) = cond_bar {
                    let bw = (self.col_px(c) - 2.0).max(0.0) * t;
                    d = d.relative().child(
                        div().absolute().left(px(1.0)).top(px(2.0)).bottom(px(2.0))
                            .w(px(bw))
                            .bg(gpui::Rgba { a: 0.65, ..bc })
                            .rounded_xs(),
                    );
                }
                // 範囲は下地に緑を**混ぜて**見せる(塗りは透けて残る)。
                // 色を抜くのは**起点のセル**(最初に選んだ方)— ドラッグで
                // 動くのは反対側の角なので、抜けが動き回らない(Excel の作法)
                let origin = self.anchor.unwrap_or(self.cursor);
                if in_range && p != origin {
                    d = d.bg(tint(base, 0.20));
                }
                // トレースの光り(参照元=青緑、参照先=橙)。塗りは透けたまま
                if let Some((_, prec)) = self.trace.iter().find(|(tp, _)| *tp == p) {
                    d = d.bg(if *prec {
                        gpui::Rgba { r: base.r * 0.55 + 0.10, g: base.g * 0.55 + 0.38, b: base.b * 0.55 + 0.38, a: 1.0 }
                    } else {
                        gpui::Rgba { r: base.r * 0.55 + 0.43, g: base.g * 0.55 + 0.30, b: base.b * 0.55 + 0.08, a: 1.0 }
                    });
                }
                if f.bold {
                    d = d.font_weight(gpui::FontWeight::BOLD);
                }
                if f.italic {
                    d = d.italic();
                }
                if f.underline {
                    d = d.underline();
                }
                if f.strike {
                    d = d.line_through();
                }
                // 下付きは小さく下げて見せる(xlsx へは vertAlign で入る)
                if f.subscript {
                    d = d.text_size(px(self.zoom * 8.5)).pt_2();
                }
                // 縦積み(255)は1字ずつ縦に並べる — 日本の帳票の縦の見出し。
                // 90/180 度は GPUI に字の回転が無いので、いまは縦積みで見せる
                if f.rotation.is_some_and(|r| r != 0) {
                    d = d.flex().flex_col().items_center();
                }
                if let Some(c) = &f.color {
                    d = d.text_color(hex(c));
                }
                // セルの書体。無い書体は系統を保って代替(明朝→明朝)
                if let Some(name) = &f.font {
                    if let Ok((fam, _)) = kumihan::font::for_document(Some(name)) {
                        d = d.font_family(SharedString::from(fam.name.clone()));
                    }
                }
                // 引いてある辺だけ濃くする(引いていない辺は表の薄い線のまま)。
                // border_color は div の**全辺に1色**なので使わない —
                // 使うと、外枠の上辺だけのセルで右・下の灰色の格子線まで
                // 黒くなり、外枠が格子に化ける(発注者報告)。
                // 辺ごとに細い帯を重ねて描く
                let ink = rgb(0x1B1B1B);
                if f.borders.any() && in_merge.is_none() {
                    d = d.relative();
                    // 1辺を線種どおりに描く。破線系は gpui の破線、
                    // 二重線は1px2本(間1px)。太さは線種から(hair=細実線)
                    let edge_bars = |e: sheet::model::Edge, horiz: bool, start: bool|
                        -> Vec<gpui::AnyElement> {
                        if !e.on {
                            return Vec::new();
                        }
                        let col = e.color.map(|v| rgb(v)).unwrap_or(ink);
                        let w = e.style.px().max(1.0);
                        let place = |b: gpui::Div, off: f32| -> gpui::Div {
                            match (horiz, start) {
                                (true, true) => b.left(px(0.0)).top(px(off)).w_full(),
                                (true, false) => b.left(px(0.0)).bottom(px(off)).w_full(),
                                (false, true) => b.top(px(0.0)).left(px(off)).h_full(),
                                (false, false) => b.top(px(0.0)).right(px(off)).h_full(),
                            }
                        };
                        let solid = |off: f32, t: f32| -> gpui::AnyElement {
                            let b = div().absolute();
                            let b = place(b, off);
                            if horiz { b.h(px(t)) } else { b.w(px(t)) }
                                .bg(col).into_any_element()
                        };
                        if e.style == sheet::model::BStyle::Double {
                            return vec![solid(0.0, 1.0), solid(2.0, 1.0)];
                        }
                        if e.style.dashed() {
                            // 破線: 1px の破線の帯を太さぶん重ねる
                            return (0..w.round() as i32)
                                .map(|i| {
                                    let b = div().absolute();
                                    let b = place(b, i as f32);
                                    let b = if horiz {
                                        b.h(px(1.0)).border_t_1()
                                    } else {
                                        b.w(px(1.0)).border_l_1()
                                    };
                                    b.border_dashed().border_color(col).into_any_element()
                                })
                                .collect();
                        }
                        vec![solid(0.0, w)]
                    };
                    for bar in edge_bars(f.borders.top, true, true)
                        .into_iter()
                        .chain(edge_bars(f.borders.bottom, true, false))
                        .chain(edge_bars(f.borders.left, false, true))
                        .chain(edge_bars(f.borders.right, false, false))
                    {
                        d = d.child(bar);
                    }
                }
                // 太い枠は**選択の範囲の外周**に出す(Excel の作法)。
                // カーソルのセルに出すと、ドラッグ中は枠がマウスに付いて回る。
                // border_t_2 + border_color は使わない — border_color は div の
                // **全辺**に効くので、縁のセルの薄い格子線(右・下)まで緑に
                // 塗り替わり、選択の中に線が走って見える(発注者報告 2026-08-08)。
                // 辺ごとの帯を重ねて描く(罫線 edge_bars と同じ作法)
                if self.anchor.is_some() {
                    if in_range {
                        let g = rgb(0x1B6E3C);
                        let mut bars: Vec<gpui::AnyElement> = Vec::new();
                        if r == ra.row {
                            bars.push(div().absolute().left(px(0.0)).top(px(0.0))
                                .w_full().h(px(2.0)).bg(g).into_any_element());
                        }
                        if r == rb.row {
                            bars.push(div().absolute().left(px(0.0)).bottom(px(0.0))
                                .w_full().h(px(2.0)).bg(g).into_any_element());
                        }
                        if c == ra.col {
                            bars.push(div().absolute().top(px(0.0)).left(px(0.0))
                                .h_full().w(px(2.0)).bg(g).into_any_element());
                        }
                        if c == rb.col {
                            bars.push(div().absolute().top(px(0.0)).right(px(0.0))
                                .h_full().w(px(2.0)).bg(g).into_any_element());
                        }
                        if !bars.is_empty() {
                            d = d.relative().children(bars);
                        }
                    }
                } else if sel {
                    d = d.border_2().border_color(rgb(0x1B6E3C));
                }
                // 縦の揃え(既定は下 = xlsx の既定)
                match f.valign {
                    sheet::model::VAlign::Top => d = d.items_start(),
                    sheet::model::VAlign::Middle => d = d.items_center(),
                    sheet::model::VAlign::Bottom => d = d.items_end(),
                }
                if f.wrap {
                    d = d.whitespace_normal().overflow_hidden();
                }
                // 縮小して全体を表示(折り返しと併せない)— 幅に収まるまで
                // 文字を小さくする。見積りは全角=1em・半角=0.5em
                if f.shrink && !f.wrap {
                    let size = self.zoom
                        * f.size_c
                            .map(|c| c as f32 / 100.0 * 24.0 / 15.0 * 0.8)
                            .unwrap_or(12.5);
                    let units: f32 = shown
                        .chars()
                        .map(|ch| if (ch as u32) < 0x2E80 { 1.0 } else { 2.0 })
                        .sum();
                    let need = units * size * 0.52 + 14.0;
                    let cw = self.col_px(c);
                    if need > cw && units > 0.0 {
                        d = d.text_size(px((size * cw / need).max(6.0)));
                    }
                }
                // 揃えの指定があればそちらが勝つ(既定は数=右・文字=左)
                match f.align {
                    HAlign::Left => d = d.justify_start(),
                    HAlign::Center => d = d.justify_center(),
                    HAlign::Right => d = d.justify_end(),
                    HAlign::Justify => d = d.justify_between(),
                    HAlign::General => {}
                }
                if is_num && f.align == HAlign::General {
                    d = d.justify_end();
                }
                // 文字色の優先順: エラー > リンク > 条件 > セルの色 > 既定
                // (以前は最後に既定色で上書きしていて、セルの文字色が死んでいた)
                if is_err {
                    d = d.text_color(rgb(0xB3261E));
                } else if self.sheet().links.contains_key(&p) {
                    // リンクのあるセルは青(Ctrl+クリックで開く)
                    d = d.text_color(rgb(0x1F4E79));
                } else if let Some(c) = cond_color {
                    d = d.text_color(c);
                } else if f.color.is_none() {
                    d = d.text_color(rgb(0x1B1B1B));
                }
                // コメントのあるセルは右上に赤い角印(表示を消していれば出さない)
                if self.show_comments && self.sheet().comments.contains_key(&p) {
                    d = d.relative().child(div().absolute()
                        .top(px(1.0)).right(px(1.0))
                        .w(px(6.0)).h(px(6.0)).rounded_sm().bg(rgb(0xC00000)));
                }
                // 入力規則のあるセルを選ぶと右下に ▾。押すと候補の一覧が
                // 開く(本家と同じ。右クリック → ドロップダウンからでも可)
                if sel && self.sheet().validation_at(p).is_some_and(|v| !v.hide_arrow) {
                    d = d.relative().child(div().id("dv-arrow").absolute()
                        .bottom(px(-1.0)).right(px(1.0))
                        .text_size(px(us * 8.5)).text_color(rgb(0x1B6E3C))
                        .cursor_pointer()
                        .child("▾")
                        .on_mouse_down(gpui::MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.open_pick_list();
                                cx.notify();
                            })));
                }
                // 選択中のセルは、確定前の入力をその場に見せる
                let shown = if sel { self.input.text().to_string() } else { shown };
                // はみ出しで描くセルは、ここでは文字を出さない(二重描き防止)。
                // 折り返しの無いセルは改行を畳んで1行にする(発注者 2026-08-06)
                let shown = if spill_from.contains(&p) {
                    String::new()
                } else if !f.wrap && shown.contains('\n') {
                    shown.replace('\n', " ")
                } else {
                    shown
                };
                if f.rotation.is_some_and(|r| r != 0) {
                    let mut stack = d;
                    for ch in shown.chars() {
                        stack = stack.child(SharedString::from(ch.to_string()));
                    }
                    row = row.child(stack);
                } else if f.rtl_text {
                    // 右横書き: 1字ずつ右から並べる(昔の看板の書き方)。
                    // ラテン文字の bidi は扱わない — 日本語の右横書きのため
                    let rev: String = shown.chars().rev().collect();
                    row = row.child(d.justify_end().child(SharedString::from(rev)));
                } else if let Some((glyph, gc)) = cond_icon {
                    row = row.child(
                        d.child(
                            div().text_color(gc).mr_1().flex_none()
                                .child(SharedString::from(glyph.to_string())),
                        )
                        .child(SharedString::from(shown)),
                    );
                } else {
                    row = row.child(d.child(SharedString::from(shown)));
                }
            }
            grid = grid.child(row);
        }
        // はみ出しの文字は格子の後に重ねる = 隣のセルの白地に負けない
        if !spill_texts.is_empty() {
            grid = grid.relative();
            for sp in spill_texts {
                grid = grid.child(sp);
            }
        }

        // ---- オートフィルタの▼と板(格子の上に重ねる) ----
        if let Some(f) = &self.auto_filter {
            grid = grid.relative();
            let (a, b) = f.range;
            let hrh = self.row_px(a.row);
            for c in a.col..=b.col {
                let Some((x, y)) = self.cell_origin_px(Pos::new(a.row, c)) else { continue };
                let w = self.col_px(c);
                if w < 24.0 {
                    continue; // 細すぎる列には▼を出さない(文字に被る)
                }
                let active = f.hide.contains_key(&c);
                let open = self.filter_panel.as_ref().is_some_and(|(pc, _)| *pc == c);
                let on = active || open;
                grid = grid.child(
                    div().id(SharedString::from(format!("flt{c}")))
                        .absolute()
                        .left(px(x + w - 17.0))
                        .top(px(y + (hrh - 14.0).max(0.0) / 2.0))
                        .w(px(14.0)).h(px(14.0)).rounded_sm()
                        .flex().items_center().justify_center()
                        .text_size(px(us * 8.0))
                        .cursor_pointer()
                        .bg(if on { rgb(0x1B6E3C) } else { rgb(0xEFF2F4) })
                        .border_1()
                        .border_color(if on { rgb(0x1B6E3C) } else { rgb(0xB6BDC4) })
                        .text_color(if on { rgb(0xFFFFFF) } else { rgb(0x66707A) })
                        .child("▼")
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                            move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.toggle_filter_panel(c);
                                cx.notify();
                            })),
                );
            }
            // ▼の板(値のチェックボックス)。開いている列の見出しの下に出す
            if let Some((col, ed)) = &self.filter_panel {
                let col = *col;
                let (vals, cut) = self.filter_values(col);
                let hide = f.hide.get(&col);
                let search = ed.text().to_string();
                let anchor = self
                    .cell_origin_px(Pos::new(a.row, col))
                    .map(|(x, y)| (x, y + hrh))
                    .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
                let px_x = anchor.0.min((self.view_w_px - 250.0).max(8.0));
                let mut panel = div().id("filter-panel")
                    .absolute().left(px(px_x)).top(px(anchor.1))
                    .w(px(236.0))
                    .p_1().rounded_md().bg(rgb(0xFFFFFF))
                    .border_1().border_color(rgb(0xC6CDD3)).shadow_lg()
                    .text_size(px(us * 12.5)).text_color(rgb(0x1B1B1B))
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation());
                // 検索欄(開いている間は打鍵がここへ来る)
                panel = panel.child(
                    div().px_2().py_1().mb_1().rounded_sm()
                        .border_1().border_color(rgb(0x1B6E3C))
                        .text_size(px(us * 12.0))
                        .child(if search.is_empty() {
                            div().text_color(rgb(0x9AA5AE))
                                .child(SharedString::from(format!("|{}", ui::t!("(打つと絞り込み)"))))
                        } else {
                            div().child(SharedString::from(format!("{search}|")))
                        }),
                );
                // (すべて選択)
                let all_on = hide.is_none();
                let all_vals: Vec<String> = vals.iter().map(|(v, _)| v.clone()).collect();
                let checkbox = |on: bool| {
                    div().flex_none().w(px(13.0)).h(px(13.0)).rounded_sm()
                        .border_1()
                        .border_color(if on { rgb(0x1B6E3C) } else { rgb(0xB6BDC4) })
                        .bg(if on { rgb(0x1B6E3C) } else { rgb(0xFFFFFF) })
                        .flex().items_center().justify_center()
                        .text_size(px(us * 9.0)).text_color(rgb(0xFFFFFF))
                        .children(on.then(|| "✓"))
                };
                panel = panel.child(
                    div().id("flt-all").px_1p5().py_0p5().rounded_sm().cursor_pointer()
                        .hover(|s| s.bg(rgb(0xEAF5EE)))
                        .flex().flex_row().items_center().gap_2()
                        .border_b_1().border_color(rgb(0xE1E6EA))
                        .child(checkbox(all_on))
                        .child(ui::t!("(すべて選択)"))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                            move |this, _, _, cx| {
                                cx.stop_propagation();
                                let all = this.filter_values(col).0
                                    .into_iter().map(|(v, _)| v).collect();
                                this.filter_toggle_all(col, all);
                                cx.notify();
                            })),
                );
                // 値の一覧(検索で絞る。長ければ板の中でスクロール)
                let mut list = div().id("flt-list").max_h(px(240.0)).overflow_y_scroll();
                let mut shown_any = false;
                for (i, (v, n)) in vals.iter().enumerate() {
                    let label = if v.is_empty() { ui::t!("(空白)").to_string() } else { v.clone() };
                    if !search.is_empty() && !label.contains(&search) {
                        continue;
                    }
                    shown_any = true;
                    let on = hide.map(|h| !h.contains(v)).unwrap_or(true);
                    let vv = v.clone();
                    list = list.child(
                        div().id(SharedString::from(format!("fv{i}")))
                            .px_1p5().py_0p5().rounded_sm().cursor_pointer()
                            .hover(|s| s.bg(rgb(0xEAF5EE)))
                            .flex().flex_row().items_center().gap_2()
                            .child(checkbox(on))
                            .child(div().flex_1().whitespace_nowrap().overflow_hidden()
                                .child(SharedString::from(label)))
                            .child(div().text_size(px(us * 11.0)).text_color(rgb(0x66707A))
                                .child(SharedString::from(n.to_string())))
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                                move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.filter_toggle_value(col, &vv);
                                    cx.notify();
                                })),
                    );
                }
                if !shown_any {
                    list = list.child(div().px_1p5().py_0p5()
                        .text_color(rgb(0x9AA5AE))
                        .child(ui::t!("(該当なし)")));
                }
                panel = panel.child(list);
                if cut {
                    panel = panel.child(div().px_1p5().text_size(px(us * 11.0))
                        .text_color(rgb(0x8A4B00))
                        .child(ui::t!("値が多いので上位 1,000 種で切っています")));
                }
                // 並べ替えとこの列の解除
                let footer_btn = |id: &'static str, label: SharedString| {
                    div().id(id).px_1p5().py_0p5().rounded_sm().cursor_pointer()
                        .hover(|s| s.bg(rgb(0xEAF5EE)))
                        .text_size(px(us * 12.0)).text_color(rgb(0x1B6E3C))
                        .child(label)
                };
                panel = panel.child(
                    div().flex().flex_row().items_center().gap_1().mt_1()
                        .border_t_1().border_color(rgb(0xE1E6EA)).pt_1()
                        .child(footer_btn("flt-asc", ui::t!("昇順").into())
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                                move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.sort_col(col, true);
                                    cx.notify();
                                })))
                        .child(footer_btn("flt-desc", ui::t!("降順").into())
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                                move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.sort_col(col, false);
                                    cx.notify();
                                })))
                        .child(div().flex_1())
                        .child(footer_btn("flt-reset", ui::t!("この列を解除").into())
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                                move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.filter_clear_col(col);
                                    cx.notify();
                                }))),
                );
                panel = panel.child(div().px_1p5().pt_0p5().text_size(px(us * 10.5))
                    .text_color(rgb(0x9AA5AE))
                    .child(ui::t!("クリックで入切 — すぐ効きます。Esc で閉じる")));
                grid = grid.child(panel);
            }
        }

        // ---- シートの耳(Excel と同じく下に置く) ----
        let mut sheets_bar = div().flex().flex_row().items_center().gap_1()
            .px_3().py_1().bg(th_head)
            .border_t_1().border_color(rgb(0xD5DBE0));
        for (i, s) in self.book.sheets.iter().enumerate() {
            if s.hidden {
                continue; // 隠したシートは耳に出さない(表示タブで戻す)
            }
            let on = i == self.active;
            // 耳の色(xlsx の tabColor)。活きている耳は白のまま、色は縁に出す
            let tabc = s.tab_color.as_deref().and_then(|h| {
                let h6 = h.get(h.len().saturating_sub(6)..)?;
                h6.chars().all(|c| c.is_ascii_hexdigit()).then(|| hex(h6))
            });
            let dark_bg = tabc
                .map(|c| c.r * 0.299 + c.g * 0.587 + c.b * 0.114 < 0.55)
                .unwrap_or(false);
            sheets_bar = sheets_bar.child(div()
                .id(SharedString::from(format!("sheet{i}")))
                .px_3().py_1().rounded_sm()
                .bg(match (on, tabc) {
                    (true, _) => rgb(0xFFFFFF),
                    (false, Some(c)) => c,
                    (false, None) => rgb(0xEFF2F4),
                })
                .border_1().border_color(match (on, tabc) {
                    (_, Some(c)) => c,
                    (true, None) => rgb(0x1B6E3C),
                    (false, None) => rgb(0xD5DBE0),
                })
                .text_size(px(us * 11.5))
                .text_color(if on {
                    rgb(0x1B6E3C)
                } else if dark_bg {
                    rgb(0xFFFFFF)
                } else {
                    rgb(0x66707A)
                })
                .font_weight(if on { gpui::FontWeight::BOLD } else { gpui::FontWeight::NORMAL })
                .cursor_pointer().hover(|s| s.bg(gpui::white()))
                .child(SharedString::from(format!(
                    "{}{}",
                    if s.protected { "🔒" } else { "" },
                    s.name
                )))
                // ダブルクリックで名前の変更(本家と同じ)。1度目は普通の切り替え
                .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                    move |this, e: &gpui::MouseDownEvent, _, cx| {
                        if e.click_count >= 2 {
                            cx.stop_propagation();
                            this.sheet_menu_at = Some(i);
                            let cur = this.book.sheets[i].name.clone();
                            this.prompt = Some(("sheet-rename", Editor::new(&cur)));
                            cx.notify();
                        }
                    }))
                // 右クリックで耳のメニュー(挿入・削除・名前の変更・…)
                .on_mouse_down(gpui::MouseButton::Right, cx.listener(
                    move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.open_sheet_menu(i);
                        cx.notify()
                    }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.switch_sheet(i);
                    cx.notify()
                })));
        }
        sheets_bar = sheets_bar.child(div()
            .id("addsheet")
            .px_2().py_1().rounded_sm()
            .text_size(px(us * 12.5)).text_color(rgb(0x1B6E3C))
            .cursor_pointer().hover(|s| s.bg(gpui::white()))
            .child("+")
            .on_click(cx.listener(|this, _, _, cx| {
                this.add_sheet();
                cx.notify()
            })));
        // 描きかけの1筆(点の粒で見せる。離すと1本の線になる)
        let ink_preview: Vec<gpui::AnyElement> = self
            .ink_cur
            .as_ref()
            .map(|pts| {
                let marker = self.tool == Some(1);
                let (sz, col) = if marker {
                    (9.0, rgb(0xFFD54A))
                } else {
                    (2.5, rgb(0x1B1B1B))
                };
                pts.iter()
                    .map(|(x, y)| {
                        div()
                            .absolute()
                            .left(px(x - sz / 2.0))
                            .top(px(y - sz / 2.0))
                            .w(px(sz))
                            .h(px(sz))
                            .rounded_full()
                            .bg(col)
                            .into_any_element()
                    })
                    .collect()
            })
            .unwrap_or_default();

        // 見張り(ウォッチウィンドウ)。控えたセルの値を下に並べる
        let watch_bar = (!self.watch.is_empty()).then(|| {
            let mut w = div().flex().flex_row().flex_wrap().gap_3()
                .px_3().py_1().bg(rgb(0xF7F9FA))
                .border_t_1().border_color(rgb(0xD5DBE0))
                .text_size(px(us * 11.0)).text_color(rgb(0x1B1B1B));
            w = w.child(div().font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(0x1B6E3C)).child(ui::t!("見張り")));
            for (si, p) in self.watch.iter().take(24) {
                let Some(sh) = self.book.sheets.get(*si) else { continue };
                let v = sh.get(*p).map(|c| c.value.display()).unwrap_or_default();
                w = w.child(div().flex().flex_row().gap_1()
                    .child(div().text_color(rgb(0x66707A))
                        .child(SharedString::from(format!("{}!{}", sh.name, p.a1()))))
                    .child(div().font_weight(gpui::FontWeight::BOLD)
                        .child(SharedString::from(v))));
            }
            w
        });

        // 下端はステータスバーを兼ねる(デスクトップ版の形):
        // 状態の文言と、選択の生きた値(合計・平均・個数)
        sheets_bar = sheets_bar
            .child(div().pl_3().text_size(px(us * 11.0)).text_color(rgb(0x66707A))
                .whitespace_nowrap().overflow_hidden()
                .child(SharedString::from(match self.hover_hint {
                    // 釦に乗っている間はその名前(本家の作法)
                    Some(h) => h.to_string(),
                    None => format!(
                        "{}{}",
                        if self.dirty { "● " } else { "" },
                        self.status
                    ),
                })))
            .child(div().flex_1())
            // 絞り込み中は残りの行数を常に見せる(本家のステータスバーと同じ)
            .children(self.filter_counts().map(|(total, shown)| {
                div().pr_3().text_size(px(us * 11.0))
                    .text_color(rgb(0x1B6EC2)).whitespace_nowrap()
                    .child(SharedString::from(ui::tf!("{} 行中 {} 行を表示", total, shown).to_string()))
            }))
            .children(self.sel_stats().map(|s| {
                div().pr_2().text_size(px(us * 11.0)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x1B6E3C)).whitespace_nowrap()
                    .child(SharedString::from(s))
            }));

        // ---- 右クリックのメニュー ----
        // **並びと名前は Euro-Office の右クリックメニューに合わせる**(リボンと
        // 同じ理由 — 乗り換える人が場所を覚え直さずに済む)。未実装は灰色。
        // AI・コメントなどの「入れないもの/まだ無いもの」も、場所だけは本家どおり。
        // InputSink より**後**に描く(bubble は後に登録した方が先に走るので、
        // 項目の stop_propagation が InputSink のセル選択より先に効く)
        let menu = self.menu_at.map(|(mx, my)| {
            // 図形の上なら専用メニュー(本家の並び。未実装は灰色で場所だけ)
            let sh_poly = self.menu_shape
                && self.shape_sel.and_then(|i| self.sheet().shapes_new.get(i)).is_some_and(|s| {
                    matches!(s.kind.as_str(), "spark" | "spark-col" | "spark-wl" | "ink" | "marker")
                });
            #[allow(clippy::type_complexity)]
            let shape_entries: Vec<(&'static str, &'static str, &'static str, bool, bool)> = vec![
                ("sh-cut", "切り取り", "", true, false),
                ("sh-copy", "コピー", "", true, false),
                ("sh-paste", "貼り付け", "", self.shape_clip.is_some(), false),
                ("", "", "", false, false),
                ("sh-order", "配置", "", true, true),
                ("sh-align", "整列", "", true, true),
                ("sh-rotate", "回転", "", !sh_poly, true),
                ("sh-group", "グループ化", "", false, false),
                ("", "", "", false, false),
                ("sh-macro", "マクロの割り当て", "", false, false),
                ("sh-save", "画像として保存(SVG)", "", true, false),
                ("sh-points", "ポイントの編集", "", false, false),
                ("", "", "", false, false),
                ("sh-settings", "図形の詳細設定", "", true, false),
                ("sh-link", "リンク", "", false, false),
                ("", "", "", false, false),
                ("sh-del", "削除", "Del", true, false),
            ];
            // (id, 名前, 付記, 押せるか, 子メニューか)
            #[allow(clippy::type_complexity)]
            let mut entries: Vec<(&'static str, &'static str, &'static str, bool, bool)> = vec![
                ("cut", "切り取り", "Ctrl+X", true, false),
                ("copy", "コピー", "Ctrl+C", true, false),
                ("paste", "貼り付け", "Ctrl+V", true, false),
                // 本家(Euro-Office)に無いのが残念、との声で追加した唯一の独自項目
                ("pastesp", "形式を選択して貼り付け", "", true, true),
                ("", "", "", false, false),
                ("ins", "挿入", "", true, true),
                ("del", "削除", "", true, true),
                ("clr", "消去", "", true, true),
                ("", "", "", false, false),
                ("sort", "並べ替え", "", true, true),
                ("subtotal", "合計の集計のしかた", "", {
                    // 合計行の =SUM / =SUBTOTAL の上でだけ生かす(本家のセル右の▼)
                    self.sheet().get(self.cursor).and_then(|c| c.formula.as_deref()).is_some_and(
                        |f| {
                            let f = f.trim_start_matches('=').trim_start().to_ascii_uppercase();
                            f.starts_with("SUM(") || f.starts_with("SUBTOTAL(")
                        },
                    )
                }, true),
                ("filter", "フィルター", "", true, true),
                ("reapply", "再適用", "", self.filter_active(), false),
                ("", "", "", false, false),
                ("addcomment", "コメントを追加", "", true, false),
                ("", "", "", false, false),
                ("fmtcells", "セルをフォーマットする", "", true, false),
                ("numfmt", "数値の書式", "", true, true),
                ("cond", "条件付き書式", "", true, true),
                ("picklist", "ドロップダウンリストから選択する", "", true, false),
                ("defname", "名前の定義", "", true, false),
                ("", "", "", false, false),
                ("func", "関数を挿入", "", true, true),
                ("hyperlink", "ハイパーリンク", "", true, false),
                ("", "", "", false, false),
                ("freeze", "枠の固定", "", true, false),
            ];
            if self.menu_shape {
                entries = shape_entries;
            }
            // 見出しからのメニューには 幅/高さ の数値指定を頭に(Excel の作法)
            match self.menu_head {
                Some(true) => {
                    entries.insert(0, ("colw", "列の幅…", "", true, false));
                    entries.insert(1, ("", "", "", false, false));
                }
                Some(false) => {
                    entries.insert(0, ("rowh", "行の高さ…", "", true, false));
                    entries.insert(1, ("", "", "", false, false));
                }
                None => {}
            }
            // 画面の右・下で切れないように少し戻す
            // 文字の大きさに追従(子メニューの位置合わせにも使う)
            let item_h: f32 = us * 25.0;
            let sep_h: f32 = us * 9.0;
            let h_est: f32 = entries.iter()
                .map(|e| if e.0.is_empty() && e.1.is_empty() { sep_h } else { item_h })
                .sum::<f32>() + 10.0;
            let grid_w = HEAD_W
                + self.visible_cols()
                    .iter()
                    .map(|c| self.col_px(*c))
                    .sum::<f32>();
            let grid_h = if self.view_h_px > 0.0 {
                self.view_h_px - 120.0
            } else {
                ROW_H + ROWS as f32 * ROW_H
            };
            let mx = mx.min((grid_w - 250.0).max(0.0));
            let my = my.min((grid_h - h_est).max(0.0));

            let mut m = div().absolute().left(px(mx)).top(px(my)).w(px(us * 244.0))
                .p_1().rounded_md().bg(rgb(0xFFFFFF))
                .border_1().border_color(rgb(0xC6CDD3)).shadow_lg()
                // メニューの余白を押してもセルに抜けない
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation());
            // 開いている子メニューの縦位置(親項目の高さに合わせる)
            let mut sub_panel: Option<gpui::Div> = None;
            let mut y_acc = 4.0f32;
            for (i, (id, label, hint, ready, is_sub)) in entries.iter().enumerate() {
                let (id, label, hint, ready, is_sub) = (*id, *label, *hint, *ready, *is_sub);
                if id.is_empty() && label.is_empty() {
                    m = m.child(div().h(px(1.0)).my_1().bg(rgb(0xE1E6EA)));
                    y_acc += sep_h;
                    continue;
                }
                let row_y = y_acc;
                y_acc += item_h;
                if !ready {
                    // 未実装。押せるように見せない(場所だけ本家どおりに残す)
                    m = m.child(div()
                        .flex().flex_row().items_center().justify_between().gap_4()
                        .px_3().py_1()
                        .child(div().text_size(px(us * 12.5)).text_color(rgb(0xB6BDC4))
                            .child(label))
                        .child(div().text_size(px(us * 10.5)).text_color(rgb(0xD5DBE0))
                            .child(if is_sub { "▸" } else { hint })));
                    continue;
                }
                if is_sub {
                    let open = self.menu_sub == Some(id);
                    m = m.child(div()
                        .id(SharedString::from(format!("m{i}")))
                        .flex().flex_row().items_center().justify_between().gap_4()
                        .px_3().py_1().rounded_sm().cursor_pointer()
                        .bg(if open { rgb(0xEAF5EE) } else { rgb(0xFFFFFF) })
                        .hover(|s| s.bg(rgb(0xEAF5EE)))
                        .child(div().text_size(px(us * 12.5)).text_color(rgb(0x1B1B1B))
                            .child(label))
                        .child(div().text_size(px(us * 11.0)).text_color(rgb(0x66707A)).child("▸"))
                        // 触れたら開く(本家と同じ)。押しても開く
                        .on_mouse_move(cx.listener(move |this, _, _, cx| {
                            if this.menu_sub != Some(id) {
                                this.menu_sub = Some(id);
                                cx.notify();
                            }
                        }))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                            move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.menu_sub = Some(id);
                                cx.notify();
                            })));
                    if open {
                        // 子の板。親項目の右横に出す
                        let mut sp = div().absolute()
                            .left(px(mx + us * 244.0)).top(px(my + row_y))
                            .w(px(us * 210.0)).p_1().rounded_md().bg(rgb(0xFFFFFF))
                            .border_1().border_color(rgb(0xC6CDD3)).shadow_lg()
                            .on_mouse_down(gpui::MouseButton::Left,
                                |_, _, cx| cx.stop_propagation());
                        for (j, (sid, slabel, sready)) in
                            self.menu_sub_entries(id).into_iter().enumerate()
                        {
                            if !sready {
                                sp = sp.child(div().px_3().py_1()
                                    .text_size(px(us * 12.5)).text_color(rgb(0xB6BDC4))
                                    .child(slabel));
                                continue;
                            }
                            sp = sp.child(div()
                                .id(SharedString::from(format!("s{i}-{j}")))
                                .px_3().py_1().rounded_sm().cursor_pointer()
                                .hover(|s| s.bg(rgb(0xEAF5EE)))
                                .text_size(px(us * 12.5)).text_color(rgb(0x1B1B1B))
                                .child(slabel)
                                .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                                    move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.menu_action(sid, window, cx);
                                    })));
                        }
                        sub_panel = Some(sp);
                    }
                    continue;
                }
                // 普通の項目
                m = m.child(div()
                    .id(SharedString::from(format!("m{i}")))
                    .flex().flex_row().items_center().justify_between().gap_4()
                    .px_3().py_1().rounded_sm().cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF5EE)))
                    .child(div().text_size(px(us * 12.5)).text_color(rgb(0x1B1B1B))
                        .child(label))
                    .child(div().text_size(px(us * 10.5)).text_color(rgb(0x9AA5AE)).child(hint))
                    // 実行できる普通の項目に触れたら、開いていた子は閉じる
                    .on_mouse_move(cx.listener(move |this, _, _, cx| {
                        if this.menu_sub.is_some() {
                            this.menu_sub = None;
                            cx.notify();
                        }
                    }))
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                        move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.menu_action(id, window, cx);
                        })));
            }
            div().absolute().left(px(0.0)).top(px(0.0)).size_full()
                .child(m)
                .children(sub_panel)
        });

        // ---- 選択中の図形の枠と右下の掴み ----
        let img_frame = self.img_sel.and_then(|i| {
            let im = self.sheet().images_new.get(i)?;
            let (x, y) = self.cell_origin_px(im.at)?;
            let (x, y) = (x + im.dx_px, y + im.dy_px);
            Some(
                div()
                    .absolute()
                    .left(px(x - 2.0))
                    .top(px(y - 2.0))
                    .w(px(im.width_px + 4.0))
                    .h(px(im.height_px + 4.0))
                    .border_2()
                    .border_dashed()
                    .border_color(rgb(0x1B6E3C))
                    .child(
                        div()
                            .absolute()
                            .right(px(-1.0))
                            .bottom(px(-1.0))
                            .w(px(10.0))
                            .h(px(10.0))
                            .bg(rgb(0x1B6E3C))
                            .cursor_nwse_resize(),
                    ),
            )
        });
        let shape_frame = self.shape_sel.and_then(|i| {
            let sp = self.sheet().shapes_new.get(i)?;
            let (x, y) = self.cell_origin_px(sp.at)?;
            let (x, y) = (x + sp.dx_px, y + sp.dy_px);
            let mut f = div()
                .absolute()
                .left(px(x - 2.0))
                .top(px(y - 2.0))
                .w(px(sp.width_px + 4.0))
                .h(px(sp.height_px + 4.0))
                .border_2()
                .border_dashed()
                .border_color(rgb(0x1B6E3C))
                .child(
                    div()
                        .absolute()
                        .right(px(-1.0))
                        .bottom(px(-1.0))
                        .w(px(10.0))
                        .h(px(10.0))
                        .bg(rgb(0x1B6E3C))
                        .cursor_nwse_resize(),
                );
            // 回転の取っ手(枠の上の丸。当たり判定は mouse_down_at 側)
            if self.shape_rot_handle(i).is_some() {
                let mid = (sp.width_px + 4.0) / 2.0;
                f = f
                    .child(
                        div()
                            .absolute()
                            .left(px(mid - 1.0))
                            .top(px(-12.0))
                            .w(px(2.0))
                            .h(px(10.0))
                            .bg(rgb(0x1B6E3C)),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(mid - 5.0))
                            .top(px(-21.0))
                            .w(px(10.0))
                            .h(px(10.0))
                            .rounded_full()
                            .bg(rgb(0x2E9E57))
                            .cursor_grab(),
                    );
            }
            Some(f)
        });
        // Ctrl+クリックで束ねた分は細い枠だけ(取っ手は主の1つに)
        let shape_frames_more: Vec<_> = self
            .shape_multi
            .iter()
            .filter_map(|&i| {
                let sp = self.sheet().shapes_new.get(i)?;
                let (x, y) = self.cell_origin_px(sp.at)?;
                let (x, y) = (x + sp.dx_px, y + sp.dy_px);
                Some(
                    div()
                        .absolute()
                        .left(px(x - 2.0))
                        .top(px(y - 2.0))
                        .w(px(sp.width_px + 4.0))
                        .h(px(sp.height_px + 4.0))
                        .border_1()
                        .border_dashed()
                        .border_color(rgb(0x2E9E57)),
                )
            })
            .collect();

        // ---- 関数を挿入の小窓(本家の FormulaDialog の形) ----
        // 検索 / 分類 / 一覧(↑↓で選ぶ・ダブルクリックで入る)/ 引数と説明
        let fn_panel = self.fn_dlg.as_ref().map(|d| {
            let list = fn_filtered(d.search.text(), d.group);
            let sel = d.sel.min(list.len().saturating_sub(1));
            let mut search_t = d.search.text().to_string();
            let cur = d.search.cursor().min(search_t.len());
            search_t.insert(cur, '|');
            let mut chips = div().flex().flex_row().flex_wrap().gap_1();
            for (gi, g) in FN_GROUPS.iter().enumerate() {
                let on = gi == d.group;
                chips = chips.child(div()
                    .id(SharedString::from(format!("fng{gi}")))
                    .px_2().py_0p5().rounded_sm().text_size(px(us * 11.5))
                    .border_1()
                    .border_color(if on { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                    .bg(if on { rgb(0xE4EFE8) } else { rgb(0xFFFFFF) })
                    .text_color(if on { rgb(0x1B6E3C) } else { rgb(0x66707A) })
                    .cursor_pointer()
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(d) = &mut this.fn_dlg {
                            d.group = gi;
                            d.sel = 0;
                        }
                        cx.notify();
                    }))
                    .child(SharedString::from(ui::tr(g))));
            }
            let start = sel.saturating_sub(5);
            let mut lst = div().flex().flex_col().h(px(252.0)).overflow_hidden()
                .border_1().border_color(rgb(0xC6CDD3)).rounded_sm().bg(rgb(0xFFFFFF));
            if list.is_empty() {
                lst = lst.child(div().px_2().py_1().text_size(px(us * 12.5))
                    .text_color(rgb(0x66707A))
                    .child(ui::t!("その条件の関数がありません")));
            }
            for (i, f) in list.iter().enumerate().skip(start).take(11) {
                let on = i == sel;
                lst = lst.child(div()
                    .id(SharedString::from(format!("fnr{i}")))
                    .px_2().py_0p5().text_size(px(us * 12.5)).flex_none()
                    .bg(if on { rgb(0x1B6E3C) } else { rgb(0xFFFFFF) })
                    .text_color(if on { rgb(0xFFFFFF) } else { rgb(0x1B1B1B) })
                    .cursor_pointer()
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                        move |this, e: &gpui::MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            if let Some(d) = &mut this.fn_dlg {
                                d.sel = i;
                            }
                            if e.click_count >= 2 {
                                this.fn_next();
                            }
                            cx.notify();
                        }))
                    .child(SharedString::from(f.name)));
            }
            let (syntax, desc) = list
                .get(sel)
                .map(|f| (format!("{}{}", f.name, f.args), f.desc.to_string()))
                .unwrap_or_default();
            let btn = |id: &'static str, label: String, primary: bool| {
                div().id(id).px_3().py_1().rounded_sm().text_size(px(us * 12.5))
                    .border_1()
                    .border_color(if primary { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                    .bg(if primary { rgb(0x1B6E3C) } else { rgb(0xFFFFFF) })
                    .text_color(if primary { rgb(0xFFFFFF) } else { rgb(0x1B1B1B) })
                    .cursor_pointer()
                    .child(SharedString::from(label))
            };
            div().absolute().inset_0().flex().items_center().justify_center()
                .child(div().w(px(430.0)).p_3().rounded_md().bg(rgb(0xF7F9FA))
                    .border_1().border_color(rgb(0x1B6E3C)).shadow_lg()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .flex().flex_col().gap_1p5()
                    .child(div().flex().flex_row().items_center()
                        .child(div().text_size(px(us * 13.0)).font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(0x1B6E3C)).child(ui::t!("関数を挿入")))
                        .child(div().flex_1())
                        .child(div().id("fn-x").px_2().cursor_pointer().text_size(px(us * 13.0))
                            .text_color(rgb(0x66707A)).hover(|s| s.text_color(rgb(0xC0392B)))
                            .child("✕")
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.fn_dlg = None;
                                cx.notify();
                            }))))
                    .child(div().px_2().py_1().bg(rgb(0xFFFFFF))
                        .border_1().border_color(rgb(0xC6CDD3)).rounded_sm()
                        .text_size(px(us * 12.5)).whitespace_nowrap().overflow_hidden()
                        .child(SharedString::from(if search_t == "|" {
                            format!("|{}", ui::t!("(打つと絞り込み)"))
                        } else {
                            search_t
                        })))
                    .child(chips)
                    .child(lst)
                    .child(div().text_size(px(us * 12.5)).font_weight(gpui::FontWeight::BOLD)
                        .child(SharedString::from(syntax)))
                    .child(div().text_size(px(us * 11.5)).text_color(rgb(0x4A545E))
                        .min_h(px(48.0))
                        .child(SharedString::from(desc)))
                    .child(div().flex().flex_row().gap_2().justify_center()
                        .child(btn("fn-next", ui::t!("次へ").to_string(), true)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.fn_next();
                                cx.notify();
                            })))
                        .child(btn("fn-cancel", ui::t!("キャンセル").to_string(), false)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.fn_dlg = None;
                                cx.notify();
                            })))))
        });

        // ---- 関数の引数の画面(本家の第2段) ----
        // 引数ごとの欄と説明、結果の下見。セルをクリックすると欄に参照が入る
        let fn_args_panel = self.fn_args.as_ref().map(|a| {
            let mut rows_el = div().flex().flex_col().gap_1();
            for (i, (name, opt)) in a.names.iter().enumerate() {
                let on = i == a.focus;
                let mut t = a.eds[i].text().to_string();
                if on {
                    let cur = a.eds[i].cursor().min(t.len());
                    t.insert(cur, '|');
                }
                rows_el = rows_el.child(div()
                    .id(SharedString::from(format!("fna{i}")))
                    .flex().flex_row().items_center().gap_2()
                    .cursor_text()
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(a) = &mut this.fn_args {
                            a.focus = i;
                        }
                        cx.notify();
                    }))
                    .child(div().w(px(us * 110.0)).text_size(px(us * 12.0))
                        .text_color(rgb(0x1B1B1B))
                        .child(SharedString::from(if *opt {
                            format!("{name}(省略可)")
                        } else {
                            name.clone()
                        })))
                    .child(div().flex_1().px_2().py_0p5().bg(rgb(0xFFFFFF))
                        .border_1()
                        .border_color(if on { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                        .rounded_sm().text_size(px(us * 12.5))
                        .whitespace_nowrap().overflow_hidden()
                        .child(SharedString::from(if t.is_empty() { " ".into() } else { t }))));
            }
            // いまの欄の説明(本家の ad — 引数順。可変長は最後の1つが代表)
            let arg_hint = a
                .names
                .get(a.focus)
                .map(|(n, _)| {
                    let d = a.f.arg_desc.get(a.focus)
                        .or(a.f.arg_desc.last())
                        .copied()
                        .unwrap_or("");
                    format!("{n}: {d}")
                })
                .unwrap_or_default();
            let btn = |id: &'static str, label: String, primary: bool| {
                div().id(id).px_3().py_1().rounded_sm().text_size(px(us * 12.5))
                    .border_1()
                    .border_color(if primary { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                    .bg(if primary { rgb(0x1B6E3C) } else { rgb(0xFFFFFF) })
                    .text_color(if primary { rgb(0xFFFFFF) } else { rgb(0x1B1B1B) })
                    .cursor_pointer()
                    .child(SharedString::from(label))
            };
            div().absolute().inset_0().flex().items_center().justify_center()
                .child(div().w(px(520.0)).p_3().rounded_md().bg(rgb(0xF7F9FA))
                    .border_1().border_color(rgb(0x1B6E3C)).shadow_lg()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .flex().flex_col().gap_1p5()
                    .child(div().flex().flex_row().items_center()
                        .child(div().text_size(px(us * 13.0)).font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(0x1B6E3C)).child(ui::t!("関数の引数")))
                        .child(div().flex_1())
                        .child(div().id("fna-x").px_2().cursor_pointer().text_size(px(us * 13.0))
                            .text_color(rgb(0x66707A)).hover(|s| s.text_color(rgb(0xC0392B)))
                            .child("✕")
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.fn_args = None;
                                cx.notify();
                            }))))
                    .child(div().text_size(px(us * 12.5)).font_weight(gpui::FontWeight::BOLD)
                        .child(SharedString::from(format!("{}{}", a.f.name, a.f.args))))
                    .child(div().text_size(px(us * 11.5)).text_color(rgb(0x4A545E))
                        .child(SharedString::from(a.f.desc)))
                    .child(rows_el)
                    .child(div().text_size(px(us * 11.5)).text_color(rgb(0x4A545E))
                        .min_h(px(44.0)).px_2().py_1()
                        .bg(rgb(0xEFF2F4)).rounded_sm()
                        .child(SharedString::from(arg_hint)))
                    .child(div().text_size(px(us * 12.0))
                        .child(SharedString::from(ui::tf!("関数の結果 = {}", a.result))))
                    .child(div().text_size(px(us * 11.0)).text_color(rgb(0x66707A))
                        .child(ui::t!("セルをクリックすると、いまの欄に参照が入ります")))
                    .child(div().flex().flex_row().gap_2().justify_center()
                        .child(btn("fna-back", ui::t!("戻る").to_string(), false)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.fn_args = None;
                                this.fn_dlg = Some(FnDlg {
                                    search: Editor::new(""),
                                    group: 0,
                                    sel: 0,
                                });
                                cx.notify();
                            })))
                        .child(btn("fna-ok", ui::t!("OK").to_string(), true)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.fn_args_ok();
                                cx.notify();
                            })))
                        .child(btn("fna-cancel", ui::t!("キャンセル").to_string(), false)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.fn_args = None;
                                cx.notify();
                            })))))
        });

        // ---- 終了確認の板(窓の中の中央。rfd はスクリーン中央に出て遠い) ----
        let quit_panel = self.quit_ask.then(|| {
            let btn = |id: &'static str, label: String, primary: bool| {
                div().id(id).px_3().py_1().rounded_sm().text_size(px(us * 12.5))
                    .border_1()
                    .border_color(if primary { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                    .bg(if primary { rgb(0x1B6E3C) } else { rgb(0xFFFFFF) })
                    .text_color(if primary { rgb(0xFFFFFF) } else { rgb(0x1B1B1B) })
                    .cursor_pointer()
                    .child(SharedString::from(label))
            };
            div().absolute().inset_0().flex().items_center().justify_center()
                .child(div().w(px(us * 420.0)).p_3().rounded_md().bg(rgb(0xF7F9FA))
                    .border_1().border_color(rgb(0x1B6E3C)).shadow_lg()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .flex().flex_col().gap_2()
                    .child(div().text_size(px(us * 13.0)).font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(0x1B6E3C))
                        .child(ui::t!("保存していない変更があります")))
                    .child(div().text_size(px(us * 12.0))
                        .child(ui::t!("保存して終了しますか?(Enter = 保存して終了 / Esc = やめる)")))
                    .child(div().flex().flex_row().gap_2().justify_center()
                        .child(btn("q-save", ui::t!("保存して終了").to_string(), true)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.quit_ask = false;
                                this.save(true, cx);
                                cx.notify();
                            })))
                        .child(btn("q-drop", ui::t!("保存せず終了").to_string(), false)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.release_lock();
                                cx.quit();
                            })))
                        .child(btn("q-cancel", ui::t!("キャンセル").to_string(), false)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.quit_ask = false;
                                this.status = ui::t!("終了をやめました").into();
                                cx.notify();
                            })))))
        });

        // ---- 固定した枠の境目(表示タブの「固定した枠に影を付ける」) ----
        // 見た目だけ。マウスは受けない。本家の作法: 影あり=固定の開始
        // 位置にひかえめな緑の線+薄い影の帯、影なし=灰色の線だけ
        let freeze_shadow: Vec<gpui::AnyElement> = {
            let mut bands = Vec::new();
            if let Some(f) = self.frozen {
                let c0 = self.visible_cols().first().copied().unwrap_or(0);
                let r0 = self.visible_rows().first().copied().unwrap_or(0);
                let line = if self.freeze_shadow { rgb(0x37A16C) } else { rgb(0x9AA5AE) };
                if f.row > 0 {
                    if let Some((_, _, _, y1)) =
                        self.range_px(Pos::new(f.row - 1, c0), Pos::new(f.row - 1, c0))
                    {
                        if self.freeze_shadow {
                            bands.push(div().absolute().left(px(0.0)).top(px(y1))
                                .w_full().h(px(3.0)).bg(gpui::rgba(0x00000030))
                                .into_any_element());
                        }
                        bands.push(div().absolute().left(px(0.0)).top(px(y1 - 1.0))
                            .w_full().h(px(1.0)).bg(line)
                            .into_any_element());
                    }
                }
                if f.col > 0 {
                    if let Some((_, _, x1, _)) =
                        self.range_px(Pos::new(r0, f.col - 1), Pos::new(r0, f.col - 1))
                    {
                        if self.freeze_shadow {
                            bands.push(div().absolute().left(px(x1)).top(px(0.0))
                                .w(px(3.0)).h_full().bg(gpui::rgba(0x00000030))
                                .into_any_element());
                        }
                        bands.push(div().absolute().left(px(x1 - 1.0)).top(px(0.0))
                            .w(px(1.0)).h_full().bg(line)
                            .into_any_element());
                    }
                }
            }
            bands
        };

        // ---- 結合の重ね描き ----
        // 結合はモデル(merges)と保存が正しくても、格子のセル割りでは
        // 「左上のセル1コマに切れた値+残る格子線」にしか見えない(発注者
        // 報告 2026-08-08)。範囲全体を不透明で覆い、値・書式・選択の枠を
        // ここで描く — マウスは受けない(InputSink が上で受ける)
        let merge_overlays: Vec<gpui::AnyElement> = {
            let mut out = Vec::new();
            let merges = self.sheet().merges.clone();
            for (a, b) in merges {
                let rect = self.range_px(a, b);
                if std::env::var_os("JO_MERGE_LOG").is_some() {
                    eprintln!("merge {}:{} rect={rect:?}", a.a1(), b.a1());
                }
                let Some((x0, y0, x1, y1)) = rect else { continue };
                let cell = self.sheet().get(a);
                let f = cell.map(|x| x.fmt.clone()).unwrap_or_default();
                let v = cell.map(|x| x.value.clone()).unwrap_or(Value::Empty);
                let mut shown = sheet::model::format_value(&v, f.number_format.as_deref());
                // 結合の上で編集中は、打ちかけを結合の枠の中に見せる(セルと同じ)
                if self.cursor == a {
                    shown = self.input.text().to_string();
                }
                // 下地: 塗り > 白。選択に入っていれば緑を混ぜる(セルと同じ)
                let mut base = f.fill.as_deref().map(hex).unwrap_or(gpui::Rgba {
                    r: 1.0, g: 1.0, b: 1.0, a: 1.0,
                });
                let sel_on = self.anchor.is_some() && {
                    let (sa, sb) = self.sel_rect();
                    sa.row <= a.row && b.row <= sb.row && sa.col <= a.col && b.col <= sb.col
                };
                if sel_on && Some(a) != Some(self.anchor.unwrap_or(self.cursor)) {
                    base = tint(base, 0.20);
                }
                let mut d = div().absolute()
                    .left(px(x0)).top(px(y0))
                    .w(px((x1 - x0).max(2.0))).h(px((y1 - y0).max(2.0)))
                    .bg(base)
                    .px_1p5().flex().overflow_hidden()
                    .font_family(self.font_name.clone())
                    .text_size(px(self.zoom * f.size_c
                        .map(|c| c as f32 / 100.0 * 24.0 / 15.0 * 0.8)
                        .unwrap_or(12.5)));
                match f.valign {
                    sheet::model::VAlign::Top => d = d.items_start(),
                    sheet::model::VAlign::Middle => d = d.items_center(),
                    sheet::model::VAlign::Bottom => d = d.items_end(),
                }
                let is_num = matches!(v, Value::Number(_));
                d = match f.align {
                    HAlign::Left => d.justify_start(),
                    HAlign::Center => d.justify_center(),
                    HAlign::Right => d.justify_end(),
                    HAlign::Justify => d.justify_between(),
                    HAlign::General if is_num => d.justify_end(),
                    HAlign::General => d.justify_start(),
                };
                if f.bold { d = d.font_weight(gpui::FontWeight::BOLD); }
                if f.italic { d = d.italic(); }
                if f.underline { d = d.underline(); }
                if f.strike { d = d.line_through(); }
                d = d.text_color(f.color.as_deref().map(hex).unwrap_or(hex("1B1B1B")));
                if let Some(name) = &f.font {
                    if let Ok((fam, _)) = kumihan::font::for_document(Some(name)) {
                        d = d.font_family(SharedString::from(fam.name.clone()));
                    }
                }
                // カーソルが結合の上なら選択の枠(セルと同じ緑)
                if self.cursor == a && self.anchor.is_none() {
                    d = d.border_2().border_color(rgb(0x1B6E3C));
                } else {
                    // 引いてある罫線の辺を外周に(実線で。細部の線種はセル側と同じ
                    // 描き分けまではしない — 結合の見た目の要は「1つに見える」こと)
                    let bs = &f.borders;
                    d = d.relative();
                    let bar = |horiz: bool, start: bool, e: sheet::model::Edge| {
                        let col = e.color.map(rgb).unwrap_or(rgb(0x1B1B1B));
                        let t = e.style.px().max(1.0);
                        let b = div().absolute();
                        let b = match (horiz, start) {
                            (true, true) => b.left(px(0.0)).top(px(0.0)).w_full().h(px(t)),
                            (true, false) => b.left(px(0.0)).bottom(px(0.0)).w_full().h(px(t)),
                            (false, true) => b.top(px(0.0)).left(px(0.0)).h_full().w(px(t)),
                            (false, false) => b.top(px(0.0)).right(px(0.0)).h_full().w(px(t)),
                        };
                        b.bg(col)
                    };
                    let mut kids: Vec<gpui::AnyElement> = Vec::new();
                    if bs.top.on { kids.push(bar(true, true, bs.top).into_any_element()); }
                    if bs.bottom.on { kids.push(bar(true, false, bs.bottom).into_any_element()); }
                    if bs.left.on { kids.push(bar(false, true, bs.left).into_any_element()); }
                    if bs.right.on { kids.push(bar(false, false, bs.right).into_any_element()); }
                    d = d.children(kids);
                }
                out.push(d.child(SharedString::from(shown)).into_any_element());
            }
            out
        };

        // ---- ピボットの塊の枠 ----
        // 集計はただのセルに見えて紛らわしい(発注者 2026-08-07)。いつも薄い
        // 枠で「特別な塊」だと見せ、カーソルが載ったら濃く+「ピボット」の札。
        // マウスは受けない
        let pivot_frames: Vec<gpui::AnyElement> = {
            let name = &self.book.sheets[self.active].name;
            let mut out = Vec::new();
            for (pi, d) in self.book.pivots.iter().enumerate() {
                if d.sheet != *name || d.size.0 == 0 {
                    continue;
                }
                let a = d.dest;
                let b = Pos::new(a.row + d.size.0 - 1, a.col + d.size.1 - 1);
                let Some((x0, y0, x1, y1)) = self.range_px(a, b) else { continue };
                let inside = on_pivot
                    && self.cursor.row >= a.row && self.cursor.row <= b.row
                    && self.cursor.col >= a.col && self.cursor.col <= b.col;
                // 札は出さない(セルの中身に被って邪魔 — 発注者 2026-08-07)。
                // 濃い枠+紫のタブ+状態行の案内で足りる
                let mut f = div().absolute()
                    .left(px(x0)).top(px(y0))
                    .w(px((x1 - x0).max(2.0))).h(px((y1 - y0).max(2.0)))
                    .border_color(rgb(0x8A63C9)).rounded_sm();
                f = if inside { f.border_2() } else { f.border_1() };
                out.push(f.into_any_element());
                // 見出しの ▼(ピボット内の絞り込み)。Excel と同じく、
                // 1行目の札(合計 / 金額・月)がある形では、月の札に列の ▼、
                // 見出し行の行の欄に行の ▼ を置く
                let has_label = !d.cols_sel.is_empty();
                let mut spots: Vec<(Pos, String)> = Vec::new();
                let head_row = a.row + has_label as u32;
                for (ci, f) in d.rows_sel.iter().enumerate() {
                    if (ci as u32) < d.size.1 {
                        spots.push((Pos::new(head_row, a.col + ci as u32), f.clone()));
                    }
                }
                if has_label {
                    let lc = a.col + d.rows_sel.len() as u32;
                    if lc <= b.col {
                        spots.push((Pos::new(a.row, lc), d.cols_sel[0].clone()));
                    }
                }
                for (hp, field) in spots {
                    let Some((hx0, hy0, hx1, hy1)) = self.range_px(hp, hp) else { continue };
                    if hx1 - hx0 < 30.0 {
                        continue; // 細すぎる列には出さない(文字に被る)
                    }
                    let hidden: std::collections::BTreeSet<String> = self
                        .book
                        .pivots[pi]
                        .hide
                        .iter()
                        .find(|(f, _)| *f == field)
                        .map(|(_, vs)| vs.iter().cloned().collect())
                        .unwrap_or_default();
                    let active = !hidden.is_empty();
                    out.push(
                        div().id(SharedString::from(format!("pflt{pi}-{}-{}", hp.row, hp.col)))
                            .absolute()
                            .left(px(hx1 - 15.0))
                            .top(px(hy0 + ((hy1 - hy0) - 13.0).max(0.0) / 2.0))
                            .w(px(13.0)).h(px(13.0)).rounded_sm()
                            .flex().items_center().justify_center()
                            .bg(if active { rgb(0xFFFFFF) } else { rgb(0x5C86D6) })
                            .text_color(if active { rgb(0x1B6E3C) } else { rgb(0xFFFFFF) })
                            .text_size(px(9.0))
                            .cursor_pointer()
                            .child("▼")
                            .on_mouse_down(gpui::MouseButton::Left, {
                                let field = field.clone();
                                let hidden = hidden.clone();
                                cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.anchor = None;
                                    this.cursor = hp;
                                    this.sync_input();
                                    this.pivot_flt =
                                        Some((pi, field.clone(), hidden.clone()));
                                    this.pivot_filter_pick();
                                    cx.notify();
                                })
                            })
                            .into_any_element(),
                    );
                }
            }
            out
        };

        // ---- コピーした範囲の破線(蟻の行進の静止版) ----
        // セルの罫線と混ざらないよう、重ね描きの1枚で囲む。マウスは受けない
        let ants = self.clip_range.and_then(|(si, a, b)| {
            if si != self.active {
                return None;
            }
            self.range_px(a, b).map(|(x0, y0, x1, y1)| {
                div().absolute()
                    .left(px(x0)).top(px(y0))
                    .w(px((x1 - x0).max(2.0))).h(px((y1 - y0).max(2.0)))
                    .border_2().border_dashed().border_color(rgb(0x1B6E3C))
            })
        });

        // ---- カーソルのセルの付記(コメント・リンク) ----
        let mut tip_lines: Vec<String> = Vec::new();
        if self.show_comments {
            if let Some(t) = self.sheet().comments.get(&self.cursor) {
                tip_lines.push(t.clone());
            }
        }
        if let Some(u) = self.sheet().links.get(&self.cursor) {
            tip_lines.push(ui::tf!("リンク: {}(Ctrl+クリックで開く)", u));
        }
        let tip = if tip_lines.is_empty() {
            None
        } else {
            self.cell_origin_px(self.cursor).map(|(x, y)| {
                let mut t = div().absolute()
                    .left(px(x + self.col_px(self.cursor.col) + 6.0))
                    .top(px(y))
                    .max_w(px(280.0)).p_2().rounded_md()
                    .bg(rgb(0xFFF9DB)).border_1().border_color(rgb(0xE0C97F)).shadow_lg();
                for line in tip_lines {
                    t = t.child(div().text_size(px(us * 11.5)).text_color(rgb(0x5C4A00))
                        .child(SharedString::from(line)));
                }
                t
            })
        };

        // ---- 入力の板(名前の定義など) ----
        let prompt_panel = self.prompt.as_ref().map(|(kind, ed)| {
            let (a, b) = self.sel_rect();
            let range = if self.anchor.is_some() {
                format!("{}:{}", a.a1(), b.a1())
            } else {
                a.a1()
            };
            let title = match *kind {
                "name" => ui::tf!("名前の定義 — {} に名前を付ける", range),
                "comment" => ui::tf!("コメント — {}(空にして Enter で消す)", self.cursor.a1()),
                "link" => ui::tf!("ハイパーリンク — {}(URL か #シート名!B5。空にして Enter で外す)", self.cursor.a1()),
                "link-text" => ui::tf!("表示テキスト — {} に見せる文字(空 Enter = そのまま)", self.cursor.a1()),
                "cond-gt" => ui::tf!("条件付き書式 — {} で、いくつより大きい値を塗る?", range),
                "cond-lt" => ui::tf!("条件付き書式 — {} で、いくつより小さい値を塗る?", range),
                "cond-between" => ui::tf!("条件付き書式 — {} で、間なら塗る(8〜15 の形)", range),
                "cond-text" => ui::tf!("条件付き書式 — {} で、含む文字は?", range),
                "cond-top" => ui::tf!("条件付き書式 — {} で、上位いくつを塗る?", range),
                "cond-bottom" => ui::tf!("条件付き書式 — {} で、下位いくつを塗る?", range),
                "find" => ui::t!("検索と置換 — 探す言葉").to_string(),
                "split-delim" => ui::tf!("区切り位置 — {} を何で割る?(空 Enter = カンマ)", range),
                "shape-text" => ui::t!("図形の文字(空にして Enter で消す)").to_string(),
                "shape-fill-rgb" => ui::t!("図形の塗り — RRGGBB の6桁(例: FFF2CC。空 Enter = 塗りなし)").to_string(),
                "shape-line-rgb" => ui::t!("図形の線 — RRGGBB の6桁(例: 1B6E3C。空 Enter = 線なし)").to_string(),
                "shape-rot" => ui::t!("図形の回転 — 度の数(時計回り。例: 45 / -30。空 Enter = 0)").to_string(),
                "py" => ui::t!("Python — 一行のコード(空 Enter = .py ファイルを選ぶ)").to_string(),
                "goal-target" => ui::t!("ゴールシーク — 目標(セル=値。例: D6=800000)").to_string(),
                "goal-var" => ui::tf!("{} をいくつにするか探します — 変えるセルは?(例: B2)", self.goal.map(|(p, v)| format!("{}={v}", p.a1())).unwrap_or_default()),
                "replace-with" => ui::tf!("「{}」を何に置き換える?", self.find_term.as_deref().unwrap_or("")),
                "chat" => ui::t!("チャット — 言伝を書き残す(ブックの隣の .chat.txt)").to_string(),
                "equation" => ui::t!("方程式 — 式を打つ(TeX の書き方。清書して画像で置く)").to_string(),
                "ai-table" => ui::t!("AI — 表にする文章").to_string(),
                "ai-ask" => ui::t!("AI — 頼み(例: 合計の式を書いて)").to_string(),
                "table-resize" => ui::t!("テーブルのサイズ変更 — 新しい範囲(A1:C9)").to_string(),
                "prop-creator" => ui::t!("ブックの情報 — 作成者").to_string(),
                "prop-title" => ui::t!("ブックの情報 — タイトル").to_string(),
                "prop-keywords" => ui::t!("ブックの情報 — タグ").to_string(),
                "prop-subject" => ui::t!("ブックの情報 — 件名").to_string(),
                "prop-desc" => ui::t!("ブックの情報 — コメント").to_string(),
                "textart" => ui::t!("テキストアート — 飾り文字にする文字を打つ").to_string(),
                "pw-open" => ui::t!("暗号化されたブック — パスワード").to_string(),
                "pw-set" => ui::t!("暗号化 — パスワード(空にして Enter で暗号化をやめる)").to_string(),
                "sheet-rename" => ui::t!("シートの名前の変更").to_string(),
                "sort-by" => ui::t!("並べ替え — 基準を左から強い順に(例: 金額 降順, 品名)").to_string(),
                "numfmt-custom" => ui::t!("数値の書式コード(例: #,##0.00 / yyyy/m/d。空 Enter = 一般)").to_string(),
                "border-color-rgb" => ui::t!("線の色 — RRGGBB の6桁(例: FF0000。空 Enter = 自動)").to_string(),
                "font-color-rgb" => ui::t!("文字の色 — RRGGBB の6桁(例: FF0000。空 Enter = 自動)").to_string(),
                "fill-color-rgb" => ui::t!("塗りの色 — RRGGBB の6桁(例: FFF2CC。空 Enter = 塗りなし)").to_string(),
                "text-angle" => ui::t!("文字の角度 — -90〜90 の数(上向きが正。空 Enter = 0)").to_string(),
                "hf-edit" => ui::t!("ヘッダー/フッター — この区分の文字(&P=頁 &N=総頁。空 Enter = 消す)").to_string(),
                "name-range" => ui::t!("名前の中身 — 場所(B12 か A1:C9 の形)").to_string(),
                "csv-delim" => ui::t!("区切りの文字を1つ(例: |)").to_string(),
                "csv-dest" => ui::t!("置き場所 — 左上のセル(B12 の形)").to_string(),
                "calc-iter" => ui::t!("反復計算 — 最大回数と変化量(例: 100 0.001。空 Enter = 切)").to_string(),
                "pivot-label" => ui::t!("ラベルで絞る — 例: 含む 東京 / で始まる 東 / で終わる 区").to_string(),
                "pivot-vfilter" => ui::t!("値で絞る — 例: > 1000(比較は > >= < <= =。空 Enter = 解除)").to_string(),
                "pivot-group-width" => ui::t!("数の幅でグループ化 — 幅を数で(例: 100)").to_string(),
                "col-width" => ui::t!("列の幅 — 0〜255(「0」何個ぶんか。空 Enter = 既定に戻す)").to_string(),
                "row-height" => ui::t!("行の高さ — 0〜409 pt(空 Enter = 既定に戻す)").to_string(),
                "subtotal-by" => ui::t!("小計 1/2 — 何の区切りで集めるか(見出しを1つ)").to_string(),
                "subtotal-vals" => ui::t!("小計 2/2 — 合計する見出し").to_string(),
                _ => String::new(),
            };
            // キャレットは | で見せる(writer の検索欄と同じ割り切り)。
            // パスワードは伏せ字
            let mut text = if matches!(*kind, "pw-open" | "pw-set") {
                "●".repeat(ed.text().chars().count())
            } else {
                ed.text().to_string()
            };
            let cur = ed.cursor().min(text.len());
            text.insert(cur, '|');
            // 板は表の中央に出す(発注者 2026-08-06「表示位置を見直す」)。
            // 外側の受け皿は聞き手を持たない = 後ろのセルの操作を遮らない
            div().absolute().inset_0().flex().items_center().justify_center()
                .child(div().w(px(us * 380.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0x1B6E3C)).shadow_lg()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(div().text_size(px(us * 12.0)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x1B6E3C)).child(SharedString::from(title)))
                .child(div().mt_1p5().px_2().py_1().bg(rgb(0xFFFFFF))
                    .border_1().border_color(rgb(0xC6CDD3)).rounded_sm()
                    .text_size(px(us * 13.0)).font_family(self.font_name.clone())
                    .child(SharedString::from(text)))
                .child(div().mt_1().text_size(px(us * 10.5)).text_color(rgb(0x66707A))
                    .child(match *kind {
                        "name" => "Enter で決定 / Esc で取消。定義した名前は式の中で使えます(=単価*2)",
                        "find" => "Enter で次へ / Esc で取消。式の中の文字も探します",
                        "split-delim" => "選択した列の文字を割って、右の列へ並べます(右は上書き)",
                        "shape-text" => "図形を選んで Enter でいつでも書き直せます",
                        "py" => "b=ブック s=シート / @計算 =PY(…)セルを評価 / @名前 実行 @名前 net @save @list @del",
                        "goal-target" | "goal-var" => "式のセルが目標の値になるよう、変えるセルの数を探します",
                        "replace-with" => "Enter で全て置き換え / **空のまま Enter = 検索だけ** / Esc で取消",
                        "chat" => "生放送ではありません — ファイル越しの言伝。最近の言伝は下の状態行に",
                        "equation" => "例: \\frac{a}{b} / \\sqrt{x^2+1} / \\sum_{i=1}^n i^2 / \\int_0^1 x\\,dx(計算はしません — セルの式とは別物)",
                        "textart" => "太字+縁取り(calc の緑)で描いて、画像としてシートに浮かべます",
                        "ai-table" => "答えのタブ区切りを、カーソルの位置の空きに流し込みます",
                        "ai-ask" => "= で始まる答えはカーソルに式として入ります。他はコメントに付きます",
                        "pw-open" => "間違えると開けません(板は残ります)。Esc で開くのをやめる",
                        "pw-set" => "次の保存から AES-128 で包みます。Excel や LibreOffice でも開けます",
                        "subtotal-by" => "使える見出しは下の状態行に出ています。並べ替えてから使うと区切りがまとまります",
                        "subtotal-vals" => "空のまま Enter = 数の列全部に入れます。畳んでも小計と総計は残ります",
                        "pivot-rows" | "pivot-cols" => "使える見出しは下の状態行に出ています。Enter で次へ / Esc で取消",
                        "pivot-val" => "例: 金額 合計。集計は 合計/平均/個数/最大/最小(省けば合計)",
                        _ => "Enter で決定 / Esc で取消",
                    })))
        });

        // ---- データの入力規則の板(本家の3タブのダイアログの形 —
        //      設定 / メッセージを入力 / エラー警告、OK・キャンセル) ----
        let dv_panel = self.dv_dlg.as_ref().map(|d| {
            let (tab, kindi, opi, styl, menu, focus) =
                (d.tab, d.kind, d.op, d.err_style, d.menu, d.focus);
            let (allow_blank, apply_same, hide_arrow) =
                (d.allow_blank, d.apply_same, d.hide_arrow);
            let show = |i: usize| -> String {
                let mut t = d.eds[i].text().to_string();
                if focus == i {
                    let cur = d.eds[i].cursor().min(t.len());
                    t.insert(cur, '|');
                }
                if t.is_empty() { t = " ".into() }
                t
            };
            // 欄(クリックで打鍵の宛先に。キャレットは | で見せる)
            let field = |i: usize, cx: &mut Context<Self>| {
                div().id(SharedString::from(format!("dv-f{i}")))
                    .w_full().px_2().py_1().bg(rgb(0xFFFFFF))
                    .border_1().rounded_sm()
                    .border_color(if focus == i { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                    .text_size(px(us * 12.5)).font_family(self.font_name.clone())
                    .whitespace_nowrap().overflow_hidden()
                    .child(SharedString::from(show(i)))
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(d) = &mut this.dv_dlg {
                            d.focus = i;
                            d.menu = 0;
                        }
                        cx.notify();
                    }))
            };
            let label = |t: String| {
                div().text_size(px(us * 11.0)).text_color(rgb(0x66707A))
                    .child(SharedString::from(t))
            };
            // ドロップダウンの頭(押すと下に選択肢が伸びる)
            let drop = |mid: u8, text: String, cx: &mut Context<Self>| {
                div().id(SharedString::from(format!("dv-m{mid}")))
                    .w_full().px_2().py_1().bg(rgb(0xFFFFFF))
                    .border_1().rounded_sm()
                    .border_color(if menu == mid { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                    .text_size(px(us * 12.5)).cursor_pointer()
                    .flex().flex_row().items_center().justify_between()
                    .child(SharedString::from(text))
                    .child(div().text_color(rgb(0x66707A)).child("▾"))
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(d) = &mut this.dv_dlg {
                            d.menu = if d.menu == mid { 0 } else { mid };
                        }
                        cx.notify();
                    }))
            };
            // 開いた選択肢。mid=1 許可 / 2 データ / 3 スタイル
            let options = |mid: u8, items: Vec<String>, cx: &mut Context<Self>| {
                let mut list = div().flex().flex_col()
                    .border_1().border_color(rgb(0x1B6E3C)).rounded_sm().bg(rgb(0xFFFFFF));
                for (i, name) in items.into_iter().enumerate() {
                    let picked = match mid { 1 => kindi, 2 => opi, _ => styl } == i;
                    list = list.child(
                        div().id(SharedString::from(format!("dv-o{mid}-{i}")))
                            .px_2().py_1().text_size(px(us * 12.0)).cursor_pointer()
                            .bg(if picked { rgb(0xEAF5EE) } else { rgb(0xFFFFFF) })
                            .hover(|s| s.bg(rgb(0xDDEEE4)))
                            .child(SharedString::from(name))
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                if let Some(d) = &mut this.dv_dlg {
                                    match mid {
                                        1 => d.kind = i,
                                        2 => d.op = i,
                                        _ => d.err_style = i,
                                    }
                                    d.menu = 0;
                                }
                                cx.notify();
                            })),
                    );
                }
                list
            };
            // ☑ の行。which: 1=空白を無視 2=同じ設定の他のセルにも
            let check = |which: u8, on: bool, text: String, cx: &mut Context<Self>| {
                div().id(SharedString::from(format!("dv-c{which}")))
                    .flex().flex_row().items_center().gap_1p5().cursor_pointer()
                    .text_size(px(us * 12.0))
                    .child(div().text_color(rgb(0x1B6E3C))
                        .child(if on { "☑" } else { "☐" }))
                    .child(SharedString::from(text))
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(d) = &mut this.dv_dlg {
                            match which {
                                1 => d.allow_blank = !d.allow_blank,
                                3 => d.hide_arrow = !d.hide_arrow,
                                _ => d.apply_same = !d.apply_same,
                            }
                        }
                        cx.notify();
                    }))
            };
            // 許可の見出し(読めない種類は「このまま保持」と正直に言う)
            let kind_label: String = if kindi == 5 {
                let k = d.keep.as_ref().map(|v| v.kind.clone()).unwrap_or_default();
                let name = match k.as_str() {
                    "date" => ui::t!("日付").to_string(),
                    "time" => ui::t!("時刻").to_string(),
                    _ => ui::t!("カスタム").to_string(),
                };
                ui::tf!("{}(このまま保持)", name).to_string()
            } else {
                DV_KINDS[kindi].to_string()
            };
            // 中身(右側)。タブごとに組む
            let mut pane = div().flex_1().flex().flex_col().gap_2().p_3()
                .text_color(rgb(0x1B1B1B));
            match tab {
                0 => {
                    let mut row = div().flex().flex_row().gap_3()
                        .child(div().flex_1().flex().flex_col().gap_1()
                            .child(label(ui::t!("許可").to_string()))
                            .child(drop(1, kind_label, cx)));
                    if matches!(kindi, 1 | 2 | 4) {
                        row = row.child(div().flex_1().flex().flex_col().gap_1()
                            .child(label(ui::t!("データ").to_string()))
                            .child(drop(2, DV_OPS[opi].1.to_string(), cx)));
                    }
                    pane = pane.child(row);
                    if menu == 1 {
                        pane = pane.child(options(1,
                            DV_KINDS.iter().map(|s| s.to_string()).collect(), cx));
                    }
                    if menu == 2 {
                        pane = pane.child(options(2,
                            DV_OPS.iter().map(|(_, n)| n.to_string()).collect(), cx));
                    }
                    pane = pane.child(check(1, allow_blank, ui::t!("空白を無視").to_string(), cx));
                    match kindi {
                        3 => {
                            pane = pane
                                .child(label(ui::t!("元の値").to_string()))
                                .child(field(0, cx))
                                .child(label(ui::t!("候補の直書き(甲,乙,丙)か、範囲の参照(=D2:D5)").to_string()))
                                .child(check(3, hide_arrow, ui::t!("セルの ▾ を出さない").to_string(), cx));
                        }
                        1 | 2 | 4 => {
                            // 間・間以外 = 最小と最大。等しい系 = 値。大小 = 片方
                            let (lo, hi) = match opi {
                                0 | 1 => (true, true),
                                2 | 3 => (true, false),
                                4 | 6 => (true, false),
                                _ => (false, true),
                            };
                            if lo {
                                let name = if matches!(opi, 2 | 3) { ui::t!("値") } else { ui::t!("最小") };
                                pane = pane.child(label(name.to_string())).child(field(0, cx));
                            }
                            if hi {
                                pane = pane.child(label(ui::t!("最大").to_string())).child(field(1, cx));
                            }
                            pane = pane.child(label(
                                ui::t!("半角の数で。数として読めない式は判定できず、堰き止めません").to_string()));
                        }
                        5 => {
                            pane = pane.child(label(
                                ui::t!("この種類(日付・時刻・カスタム)は判定できません — 規則は壊さず保ち、文言だけ直せます").to_string()));
                        }
                        _ => {}
                    }
                    pane = pane.child(div().flex_1())
                        .child(check(2, apply_same,
                            ui::t!("これらの変更を同じ設定の他のすべてのセルに適用する").to_string(), cx));
                }
                1 => {
                    pane = pane
                        .child(label(ui::t!("タイトル").to_string()))
                        .child(field(2, cx))
                        .child(label(ui::t!("メッセージ").to_string()))
                        .child(field(3, cx))
                        .child(label(ui::t!("セルを選ぶと、下の状態行にこの説明が出ます").to_string()));
                }
                _ => {
                    pane = pane
                        .child(label(ui::t!("スタイル").to_string()))
                        .child(drop(3, DV_STYLES[styl].1.to_string(), cx));
                    if menu == 3 {
                        pane = pane.child(options(3,
                            DV_STYLES.iter().map(|(_, n)| n.to_string()).collect(), cx));
                    }
                    pane = pane
                        .child(label(ui::t!("タイトル").to_string()))
                        .child(field(4, cx))
                        .child(label(ui::t!("エラーメッセージ").to_string()))
                        .child(field(5, cx))
                        .child(label(ui::t!("停止は堰き止め、警告・情報は通して言うだけ(Excel と同じ)").to_string()));
                }
            }
            // 左のタブ(設定 / メッセージを入力 / エラー警告)
            let mut tabs = div().w(px(us * 150.0)).flex().flex_col().p_1().gap_0p5()
                .bg(rgb(0xF0F3F5)).rounded_sm();
            for (i, name) in [ui::t!("設定"), ui::t!("メッセージを入力"), ui::t!("エラー警告")]
                .into_iter().enumerate()
            {
                let on = tab == i as u8;
                tabs = tabs.child(
                    div().id(SharedString::from(format!("dv-t{i}")))
                        .px_2().py_1p5().rounded_sm().cursor_pointer()
                        .text_size(px(us * 12.0))
                        .bg(if on { rgb(0xFFFFFF) } else { rgb(0xF0F3F5) })
                        .when(on, |s| s.font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(0x1B6E3C)))
                        .hover(|s| s.bg(rgb(0xFFFFFF)))
                        .child(SharedString::from(name))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(d) = &mut this.dv_dlg {
                                d.tab = i as u8;
                                d.menu = 0;
                                d.focus = [0, 2, 4][i]; // タブの最初の欄へ
                            }
                            cx.notify();
                        })),
                );
            }
            let btn = |id: &'static str, text: String, primary: bool| {
                div().id(id).px_4().py_1().rounded_sm().text_size(px(us * 12.5))
                    .border_1()
                    .border_color(if primary { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                    .bg(if primary { rgb(0x1B6E3C) } else { rgb(0xFFFFFF) })
                    .text_color(if primary { rgb(0xFFFFFF) } else { rgb(0x1B1B1B) })
                    .cursor_pointer()
                    .child(SharedString::from(text))
            };
            div().absolute().inset_0().flex().items_center().justify_center()
                .child(div().w(px(us * 540.0)).rounded_md().bg(rgb(0xF7F9FA))
                    .border_1().border_color(rgb(0x1B6E3C)).shadow_lg()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .flex().flex_col()
                    // 題の行と ×
                    .child(div().flex().flex_row().items_center().px_3().py_2()
                        .border_b_1().border_color(rgb(0xE1E6EA))
                        .child(div().flex_1().text_size(px(us * 13.0))
                            .font_weight(gpui::FontWeight::BOLD).text_color(rgb(0x1B6E3C))
                            .child(ui::t!("データの入力規則")))
                        .child(div().id("dv-x").px_1p5().rounded_sm().cursor_pointer()
                            .text_size(px(us * 13.0)).text_color(rgb(0x66707A))
                            .hover(|s| s.bg(rgb(0xE1E6EA)))
                            .child("×")
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.dv_dlg = None;
                                this.status = ui::t!("入力規則をやめました").into();
                                cx.notify();
                            }))))
                    // 本体: 左のタブ + 右の中身
                    .child(div().flex().flex_row().items_stretch().p_2().gap_2()
                        .min_h(px(us * 260.0))
                        .child(tabs)
                        .child(pane))
                    // OK / キャンセル
                    .child(div().flex().flex_row().gap_2().justify_center().pb_3()
                        .child(btn("dv-ok", ui::t!("OK").to_string(), true)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.dv_ok(cx);
                            })))
                        .child(btn("dv-cancel", ui::t!("キャンセル").to_string(), false)
                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.dv_dlg = None;
                                this.status = ui::t!("入力規則をやめました").into();
                                cx.notify();
                            })))))
        });

        // ---- ソルバーの小窓(ONLYOFFICE の「ソルバーのパラメータ」の形) ----
        // モーダルにしない板たちと同じ作法。打鍵は focus の欄へ(HasEditor)
        let solver_panel = self.solver.as_ref().map(|sv| {
            let show = |ed: &Editor, on: bool| -> String {
                let mut t = ed.text().to_string();
                if on {
                    let cur = ed.cursor().min(t.len());
                    t.insert(cur, '|');
                }
                if t.is_empty() { t = " ".into() }
                t
            };
            let (focus, mode, nonneg, sel) = (sv.focus, sv.mode, sv.nonneg, sv.sel);
            let field = |id: &'static str, f: u8, text: String, cx: &mut Context<Self>| {
                div().id(id).flex_1().px_2().py_1().bg(rgb(0xFFFFFF))
                    .border_1().rounded_sm()
                    .border_color(if focus == f { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                    .text_size(px(us * 12.5)).font_family(self.font_name.clone())
                    .whitespace_nowrap().overflow_hidden()
                    .child(SharedString::from(text))
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(sv) = &mut this.solver {
                            sv.focus = f;
                        }
                        cx.notify();
                    }))
            };
            let label = |t: &'static str| {
                div().mt_1p5().text_size(px(us * 11.5)).text_color(rgb(0x444B52)).child(t)
            };
            let btn = |id: &'static str, t: &'static str, on: bool| {
                div().id(id).px_2p5().py_1().rounded_sm().border_1()
                    .border_color(if on { rgb(0xC6CDD3) } else { rgb(0xEDEFF1) })
                    .text_size(px(us * 11.5))
                    .text_color(if on { rgb(0x1B1B1B) } else { rgb(0xB6BDC4) })
                    .when(on, |d| d.cursor_pointer().hover(|s| s.bg(rgb(0xEAF5EE))))
            };
            let radio = |id: &'static str, m: u8, t: &'static str, cx: &mut Context<Self>| {
                div().id(id).flex().flex_row().items_center().gap_1()
                    .cursor_pointer().text_size(px(us * 12.0))
                    .child(if mode == m { "◉" } else { "○" })
                    .child(t)
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(sv) = &mut this.solver {
                            sv.mode = m;
                            if m == 2 {
                                sv.focus = 1;
                            }
                        }
                        cx.notify();
                    }))
            };
            // 制約の一覧
            let mut list = div().mt_1().p_1().h(px(96.0)).bg(rgb(0xFAFBFC))
                .border_1().border_color(rgb(0xC6CDD3)).rounded_sm()
                .flex().flex_col().overflow_hidden();
            if sv.cons.is_empty() {
                list = list.child(div().flex_1().flex().items_center().justify_center()
                    .text_size(px(us * 11.5)).text_color(rgb(0xB6BDC4))
                    .child(ui::t!("まだ制約はありません。左辺・記号・右辺を打って「追加」")));
            } else {
                for (i, (l, op, r)) in sv.cons.iter().enumerate() {
                    let on = sel == Some(i);
                    list = list.child(div()
                        .id(SharedString::from(format!("con{i}")))
                        .px_2().py_0p5().rounded_sm().text_size(px(us * 12.0))
                        .bg(if on { rgb(0xEAF5EE) } else { rgb(0xFAFBFC) })
                        .cursor_pointer()
                        .child(SharedString::from(format!("{l} {op} {r}")))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(sv) = &mut this.solver {
                                sv.sel = Some(i);
                                let (l, op, r) = sv.cons[i].clone();
                                sv.con_l = Editor::new(&l);
                                sv.con_op =
                                    SOLVER_OPS.iter().position(|o| *o == op).unwrap_or(0);
                                sv.con_r = Editor::new(&r);
                            }
                            cx.notify();
                        })));
                }
            }
            // ソルバーも表の中央(prompt の板と同じ作法)
            div().absolute().inset_0().flex().items_center().justify_center()
                .child(div().w(px(470.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0x1B6E3C)).shadow_lg()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .flex().flex_col().gap_1()
                .child(div().flex().flex_row().items_center()
                    .child(div().text_size(px(us * 13.0)).font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(0x1B6E3C)).child(ui::t!("ソルバーのパラメータ")))
                    .child(div().flex_1())
                    .child(div().id("sv-x").px_2().cursor_pointer().text_size(px(us * 13.0))
                        .text_color(rgb(0x66707A)).hover(|s| s.text_color(rgb(0xC0392B)))
                        .child("✕")
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.solver = None;
                            cx.notify();
                        }))))
                .child(label("目的を設定"))
                .child(div().flex().flex_row()
                    .child(field("sv-target", 0, show(&sv.target, focus == 0), cx)))
                .child(div().mt_1().flex().flex_row().items_center().gap_3()
                    .child(radio("sv-max", 0, "最大", cx))
                    .child(radio("sv-min", 1, "最小", cx))
                    .child(radio("sv-val", 2, "値:", cx))
                    .child(field("sv-value", 1, show(&sv.value, focus == 1), cx)))
                .child(label("変数セルを変更して"))
                .child(div().flex().flex_row()
                    .child(field("sv-vars", 2, show(&sv.vars, focus == 2), cx)))
                .child(label("制約条件付き(左辺セル / 記号 / 右辺の数かセル)"))
                .child(div().flex().flex_row().items_center().gap_1()
                    .child(field("sv-conl", 3, show(&sv.con_l, focus == 3), cx))
                    .child(div().id("sv-op").px_2().py_1().rounded_sm().border_1()
                        .border_color(rgb(0xC6CDD3)).text_size(px(us * 12.0))
                        .cursor_pointer().hover(|s| s.bg(rgb(0xEAF5EE)))
                        .child(SOLVER_OPS[sv.con_op])
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(sv) = &mut this.solver {
                                sv.con_op = (sv.con_op + 1) % 3;
                            }
                            cx.notify();
                        })))
                    .child(field("sv-conr", 4, show(&sv.con_r, focus == 4), cx)))
                .child(div().mt_1().flex().flex_row().gap_1()
                    .child(btn("sv-add", "追加", true).child(ui::t!("追加"))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(sv) = &mut this.solver {
                                let (l, r) =
                                    (sv.con_l.text().trim().to_string(),
                                     sv.con_r.text().trim().to_string());
                                if l.is_empty() || r.is_empty() {
                                    this.status =
                                        ui::t!("制約の左辺と右辺を先に打ってください").into();
                                } else {
                                    sv.cons.push((l, SOLVER_OPS[sv.con_op], r));
                                    sv.con_l = Editor::new("");
                                    sv.con_r = Editor::new("");
                                    sv.sel = None;
                                }
                            }
                            cx.notify();
                        })))
                    .child(btn("sv-edit", "変更", sel.is_some()).child(ui::t!("変更"))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(sv) = &mut this.solver {
                                if let Some(i) = sv.sel {
                                    let (l, r) =
                                        (sv.con_l.text().trim().to_string(),
                                         sv.con_r.text().trim().to_string());
                                    if !l.is_empty() && !r.is_empty() && i < sv.cons.len() {
                                        sv.cons[i] = (l, SOLVER_OPS[sv.con_op], r);
                                    }
                                }
                            }
                            cx.notify();
                        })))
                    .child(div().flex_1())
                    .child(btn("sv-del", "削除", sel.is_some()).child(ui::t!("削除"))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(sv) = &mut this.solver {
                                if let Some(i) = sv.sel.take() {
                                    if i < sv.cons.len() {
                                        sv.cons.remove(i);
                                    }
                                }
                            }
                            cx.notify();
                        }))))
                .child(list)
                .child(div().id("sv-nonneg").mt_1().flex().flex_row().items_center().gap_1()
                    .cursor_pointer().text_size(px(us * 12.0))
                    .child(if nonneg { "☑" } else { "☐" })
                    .child(ui::t!("制約のない変数を非負にする"))
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(sv) = &mut this.solver {
                            sv.nonneg = !sv.nonneg;
                        }
                        cx.notify();
                    })))
                .child(div().mt_1().flex().flex_row().items_center().gap_2()
                    .child(div().text_size(px(us * 12.0)).font_weight(gpui::FontWeight::BOLD)
                        .child(ui::t!("解法の方法")))
                    .child(div().px_2().py_0p5().border_1().border_color(rgb(0xC6CDD3))
                        .rounded_sm().text_size(px(us * 11.5)).child(ui::t!("単体法 LP"))))
                .child(div().text_size(px(us * 10.5)).text_color(rgb(0x66707A))
                    .child(ui::t!("線形の問題を LP シンプレックスで解きます(裏方 scipy)。非線形はまだ解けません — そのときは断ります")))
                .child(div().mt_1p5().flex().flex_row().gap_1()
                    .child(btn("sv-reset", "すべてリセット", true).child(ui::t!("すべてリセット"))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            let init = this.cursor.a1();
                            this.solver = Some(Solver::new(&init));
                            cx.notify();
                        })))
                    .child(div().flex_1())
                    .child(div().id("sv-solve").px_3().py_1().rounded_sm()
                        .bg(rgb(0x1B6E3C)).text_color(rgb(0xFFFFFF))
                        .text_size(px(us * 12.0)).cursor_pointer()
                        .hover(|s| s.bg(rgb(0x2E8B57)))
                        .child(ui::t!("解を求める"))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.solve_solver(cx);
                            cx.notify();
                        })))
                    .child(btn("sv-close", "閉じる", true).child(ui::t!("閉じる"))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.solver = None;
                            cx.notify();
                        })))))
        });

        // ---- ファイルの全面ページ(本家の File メニュー。タブ0で全面) ----
        let filepage = (self.tab == 0).then(|| {
            let item_bg = rgb(0xE2E6EA);
            let gray = rgb(0xB6BDC4);
            let fg = rgb(0x444B52);
            let dim = rgb(0x66707A);
            let mk = |id: &'static str, label: &'static str, ready: bool| {
                let d = div().id(id).px_4().py_1p5().text_size(px(us * 13.0));
                if ready {
                    d.text_color(fg).cursor_pointer().hover(move |s| s.bg(item_bg))
                } else {
                    d.text_color(gray)
                }
                .child(label)
            };
            let sb = div().w(px(280.0)).bg(rgb(0xF1F3F5))
                .border_r_1().border_color(rgb(0xE1E6EA))
                .flex().flex_col().py_2()
                .child(mk("f-back", ui::t!("‹ 戻る"), true).on_click(cx.listener(|this, _, _, cx| {
                    this.tab = this.prev_tab;
                    cx.notify()
                })))
                .child(div().h(px(10.0)))
                .child(mk("f-new", ui::t!("新規作成"), true).on_click(cx.listener(|this, _, _, cx| {
                    if this.new_book() {
                        this.tab = this.prev_tab;
                    }
                    cx.notify()
                })))
                .child(mk("f-tpl", ui::t!("テンプレートから作成"), false))
                .child(mk("f-open", ui::t!("開く"), true).on_click(cx.listener(|this, _, _, cx| {
                    this.tab = this.prev_tab;
                    this.open_dialog(cx);
                    cx.notify()
                })))
                .child({
                    let d = mk("f-recent", ui::t!("最近開いた"), true).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.file_view = 1;
                            cx.notify()
                        }));
                    if self.file_view == 1 { d.bg(item_bg) } else { d }
                })
                .child(div().h(px(10.0)))
                .child(mk("f-save", ui::t!("保存"), true).on_click(cx.listener(|this, _, _, cx| {
                    this.save(false, cx);
                    cx.notify()
                })))
                .child(mk("f-saveas", ui::t!("名前を付けて保存"), true).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.save_as(cx);
                        cx.notify()
                    })))
                .child(mk("f-print", ui::t!("印刷"), true).on_click(cx.listener(|this, _, _, cx| {
                    this.run_cmd("pdf", cx);
                    cx.notify()
                })))
                .child(mk("f-csv", ui::t!("CSV に書き出す"), true).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.export_csv_dialog(cx);
                        cx.notify()
                    })))
                .child(mk("f-protect", ui::t!("保護する"), true).on_click(cx.listener(
                    |this, _, _, cx| {
                        if let Some(i) =
                            ribbon::CALC.iter().position(|t| t.name == "保護")
                        {
                            this.prev_tab = i;
                            this.tab = i;
                        }
                        cx.notify()
                    })))
                .child(div().h(px(10.0)))
                .child({
                    let d = mk("f-info", ui::t!("詳細情報"), true).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.file_view = 0;
                            cx.notify()
                        }));
                    if self.file_view == 0 { d.bg(item_bg) } else { d }
                })
                .child(mk("f-place", ui::t!("ファイルの場所を開く"), true).on_click(cx.listener(
                    |this, _, _, cx| {
                        match this.path.as_ref().and_then(|p| p.parent()) {
                            Some(dir) => {
                                let _ = std::process::Command::new("xdg-open")
                                    .arg(dir)
                                    .spawn();
                            }
                            None => {
                                this.status = ui::t!("まだファイルになっていません").into();
                            }
                        }
                        cx.notify()
                    })))
                .child(div().h(px(10.0)))
                .child(mk("f-quit", ui::t!("終了"), true).on_click(cx.listener(|this, _, _, cx| {
                    this.request_quit(cx);
                    cx.notify()
                })))
                .child(div().flex_1())
                .child({
                    let d = mk("f-opts", ui::t!("詳細設定"), true).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.file_view = 2;
                            cx.notify()
                        }));
                    if self.file_view == 2 { d.bg(item_bg) } else { d }
                })
                .child(mk("f-help", ui::t!("ヘルプ"), false))
                .child(mk("f-req", ui::t!("機能のリクエスト"), false));
            let mut pane = div().flex_1().bg(gpui::white()).p_8()
                .flex().flex_col().gap_3().text_size(px(us * 12.5)).text_color(fg);
            if self.file_view == 2 {
                // 詳細設定 — 器は ~/.config/office/settings.toml
                // (SEKKEI「設定 — 器と言語」。環境変数が一時上書きで優先)
                let lang_now = ui::settings::get("language").unwrap_or_else(|| "ja".into());
                let row = |label: &'static str, value: String| {
                    div().flex().flex_row().items_center().gap_2()
                        .child(div().w(px(us * 200.0)).text_color(dim).child(label))
                        .child(div().child(SharedString::from(value)))
                };
                pane = pane
                    .child(div().text_size(px(us * 16.0))
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(ui::t!("詳細設定")))
                    .child(div().text_color(dim).child(SharedString::from(
                        ui::tf!("置き場: {}", ui::settings::path().display()))))
                    .child(div().h(px(6.0)))
                    .child(div().flex().flex_row().items_center().gap_2()
                        .child(div().w(px(us * 200.0)).text_color(dim)
                            .child(ui::t!("言語(リボンと文言)")))
                        .child(div().id("set-lang")
                            .px_3().py_1().rounded_sm().cursor_pointer()
                            .bg(item_bg)
                            .child(SharedString::from(match lang_now.as_str() {
                                "ja" => "日本語".to_string(),
                                other => other.to_string(),
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                let cur = ui::settings::get("language")
                                    .unwrap_or_else(|| "ja".into());
                                let all = ui::languages();
                                let i = all.iter().position(|l| **l == cur).unwrap_or(0);
                                let next = all[(i + 1) % all.len()];
                                ui::settings::set("language", next);
                                this.status = ui::t!("言語を控えました(次の起動から効きます。環境変数 OFFICE_LANG があればそちらが優先)").into();
                                cx.notify()
                            }))))
                    .child(div().flex().flex_row().items_center().gap_2()
                        .child(div().w(px(us * 200.0)).text_color(dim)
                            .child(ui::t!("画面の明暗(テーマ)")))
                        .child(div().id("set-theme")
                            .px_3().py_1().rounded_sm().cursor_pointer()
                            .bg(item_bg)
                            .child(if self.dark { ui::t!("暗い") } else { ui::t!("明るい") })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.run_cmd("theme", cx);
                                cx.notify()
                            }))))
                    .child(div().flex().flex_row().items_center().gap_2()
                        .child(div().w(px(us * 200.0)).text_color(dim)
                            .child(ui::t!("画面の文字の大きさ")))
                        .child(div().id("set-ui-minus")
                            .px_3().py_1().rounded_sm().cursor_pointer().bg(item_bg)
                            .child("−")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.run_cmd("ui-smaller", cx);
                                cx.notify()
                            })))
                        .child(div().child(SharedString::from(
                            format!("{}%", (self.ui_scale * 100.0).round() as i32))))
                        .child(div().id("set-ui-plus")
                            .px_3().py_1().rounded_sm().cursor_pointer().bg(item_bg)
                            .child("+")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.run_cmd("ui-bigger", cx);
                                cx.notify()
                            }))))
                    .child(div().flex().flex_row().items_center().gap_2()
                        .child(div().w(px(us * 200.0)).text_color(dim)
                            .child(ui::t!("反復計算(循環参照)")))
                        .child(div().id("set-iter")
                            .px_3().py_1().rounded_sm().cursor_pointer().bg(item_bg)
                            .child(match self.book.calc_iter {
                                Some((n, d)) => ui::tf!("入(最大 {} 回・変化 {} まで)", n, d),
                                None => ui::t!("切").into(),
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.run_cmd("calc-iter", cx);
                                cx.notify()
                            }))))
                    .child(div().flex().flex_row().items_center().gap_2()
                        .child(div().w(px(us * 200.0)).text_color(dim)
                            .child(ui::t!("参照の形式")))
                        .child(div().id("set-refstyle")
                            .px_3().py_1().rounded_sm().cursor_pointer().bg(item_bg)
                            .child(if self.book.r1c1 { "R1C1" } else { "A1" })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.run_cmd("ref-style", cx);
                                cx.notify()
                            }))))
                    .child(div().h(px(10.0)))
                    .child(row(ui::t!("書体(OFFICE_FONT)"),
                        std::env::var("OFFICE_FONT")
                            .unwrap_or_else(|_| ui::t!("(文書に従う)").into())))
                    .child(row(ui::t!("校正の宛先"), {
                        let ep = ui::Endpoint::default();
                        format!("{}:{} / {}", ep.host, ep.port, ep.model)
                    }))
                    .child(row(ui::t!("Python の経路"),
                        std::env::var("JO_PYTHON")
                            .unwrap_or_else(|_| ui::t!("(自動: .venv → python3)").into())))
                    .child(row(ui::t!("名前(ロック・チャット・署名)"), lock_identity()));
            } else if self.file_view == 1 {
                pane = pane.child(div().text_size(px(us * 16.0))
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(ui::t!("最近開いた")));
                let list = Self::recent_list();
                if list.is_empty() {
                    pane = pane.child(div().text_color(dim)
                        .child(ui::t!("(まだありません。開く・保存すると残ります)")));
                }
                for (i, q) in list.into_iter().enumerate() {
                    let name = q.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let dir = q.parent()
                        .map(|d| d.to_string_lossy().to_string())
                        .unwrap_or_default();
                    pane = pane.child(div()
                        .id(SharedString::from(format!("recent-{i}")))
                        .px_2().py_1().rounded_sm().cursor_pointer()
                        .hover(move |s| s.bg(item_bg))
                        .flex().flex_row().items_center().gap_2()
                        .child(div().text_size(px(us * 13.0)).child(SharedString::from(name)))
                        .child(div().text_size(px(us * 11.0)).text_color(dim)
                            .child(SharedString::from(dir)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.tab = this.prev_tab;
                            this.open(q.clone());
                            cx.notify()
                        })));
                }
            } else {
                // 統計(生きた値)とブックの情報(docProps/core.xml から)
                let sheets_n = self.book.sheets.len();
                let mut cells_n = 0usize;
                let mut formulas_n = 0usize;
                for sh in &self.book.sheets {
                    cells_n += sh.cells.len();
                    formulas_n +=
                        sh.cells.values().filter(|c| c.formula.is_some()).count();
                }
                let shapes_n: usize = self
                    .book
                    .sheets
                    .iter()
                    .map(|s| {
                        s.shapes.len() + s.shapes_new.len() + s.images.len()
                            + s.images_new.len()
                    })
                    .sum();
                pane = pane.child(div().text_size(px(us * 16.0))
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(ui::t!("ブックの情報")))
                    .child(div().text_size(px(us * 13.5))
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(ui::t!("統計")));
                for (k, v) in [
                    ("シート", sheets_n),
                    ("使っているセル", cells_n),
                    ("式のセル", formulas_n),
                    ("図形と画像", shapes_n),
                ] {
                    pane = pane.child(div().flex().flex_row()
                        .child(div().w(px(220.0)).text_color(dim).child(k))
                        .child(SharedString::from(format!("{v}"))));
                }
                pane = pane.child(div().h(px(6.0)))
                    .child(div().text_size(px(us * 13.5))
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(ui::t!("プロパティ")));
                let pr = &self.book.props;
                for (k, v, kind) in [
                    ("作成者", pr.creator.clone(), "prop-creator"),
                    ("タイトル", pr.title.clone(), "prop-title"),
                    ("タグ", pr.keywords.clone(), "prop-keywords"),
                    ("件名", pr.subject.clone(), "prop-subject"),
                    ("コメント", pr.description.clone(), "prop-desc"),
                ] {
                    let empty = v.is_empty();
                    let init = v.clone();
                    pane = pane.child(div().flex().flex_row().items_center()
                        .child(div().w(px(220.0)).text_color(dim).child(k))
                        .child(div()
                            .id(SharedString::from(kind))
                            .w(px(320.0)).px_2().py_1().rounded_sm()
                            .border_1().border_color(rgb(0xE1E6EA))
                            .cursor_pointer()
                            .hover(move |s| s.bg(item_bg))
                            .whitespace_nowrap().overflow_hidden()
                            .text_color(if empty { gray } else { fg })
                            .child(SharedString::from(if empty {
                                ui::t!("テキストの追加").to_string()
                            } else {
                                v
                            }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.prompt = Some((kind, Editor::new(&init)));
                                cx.notify()
                            }))));
                }
                pane = pane.child(div().text_size(px(us * 11.5)).text_color(dim)
                    .child(ui::t!("欄を押して打ち、Enter で控える(保存で xlsx の情報に入ります)")));
            }
            div().absolute().inset_0().bg(gpui::white())
                .flex().flex_row()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(sb)
                .child(pane)
        });

        // ---- スライサーの小窓(列の値の釦で絞る) ----
        let slicer_panel = self.slicer.as_ref().map(|(col, sel, multi)| {
            let col = *col;
            let multi = *multi;
            // 見出し(1行目)と、その下の一意な値。空欄は「(空白)」で最後に
            let head = self
                .sheet()
                .get(Pos::new(0, col))
                .map(|c| c.value.display())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| ui::tf!("列{}", col_name(col)));
            let (rows, _) = self.sheet().extent();
            let mut vals: std::collections::BTreeSet<String> = Default::default();
            let mut has_blank = false;
            for r in 1..rows {
                let v = self
                    .sheet()
                    .get(Pos::new(r, col))
                    .map(|c| c.value.display())
                    .unwrap_or_default();
                if v.is_empty() {
                    has_blank = true;
                } else {
                    vals.insert(v);
                }
            }
            let mut items: Vec<String> = vals.into_iter().take(64).collect();
            if has_blank {
                items.push(ui::t!("(空白)").to_string());
            }
            let mut p = div().absolute().right(px(24.0)).top(px(ROW_H + 16.0)).w(px(us * 190.0))
                .p_2().rounded_md().bg(gpui::white())
                .border_1().border_color(rgb(0x1B6E3C)).shadow_lg()
                .flex().flex_col().gap_1()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(div().flex().flex_row().items_center()
                    .child(div().text_size(px(us * 12.5)).font_weight(gpui::FontWeight::BOLD)
                        .whitespace_nowrap().overflow_hidden()
                        .child(SharedString::from(head)))
                    .child(div().flex_1())
                    // ≡ = 複数選択の入切(本家のスライサーと同じ並び)
                    .child(div().id("sl-multi").px_1p5().rounded_sm().cursor_pointer()
                        .text_size(px(us * 12.5))
                        .bg(if multi { rgb(0xCFE6D8) } else { rgb(0xFFFFFF) })
                        .hover(|s| s.bg(rgb(0xEAF5EE)))
                        .child("≡")
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some((_, _, m)) = &mut this.slicer {
                                *m = !*m;
                                this.status = if *m {
                                    ui::t!("複数選択: 押した値を重ねて絞ります").into()
                                } else {
                                    ui::t!("単数選択: 押した値ひとつで絞ります").into()
                                };
                            }
                            cx.notify();
                        })))
                    // ✕ = 選びを解除(全部見せる)
                    .child(div().id("sl-clear").px_1p5().rounded_sm().cursor_pointer()
                        .text_size(px(us * 12.5)).hover(|s| s.bg(rgb(0xEAF5EE)))
                        .child("✕")
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some((_, sel, _)) = &mut this.slicer {
                                sel.clear();
                            }
                            this.status = ui::t!("スライサーの絞りを解除しました").into();
                            cx.notify();
                        }))));
            for (i, v) in items.into_iter().enumerate() {
                let on = sel.contains(&v);
                p = p.child(div()
                    .id(SharedString::from(format!("sl{i}")))
                    .px_2().py_1().rounded_sm().border_1()
                    .border_color(rgb(0xC6CDD3))
                    .bg(if on { rgb(0xBBD9EA) } else { rgb(0xFFFFFF) })
                    .text_size(px(us * 12.0)).cursor_pointer()
                    .whitespace_nowrap().overflow_hidden()
                    .hover(|s| s.bg(rgb(0xEAF5EE)))
                    .child(SharedString::from(v.clone()))
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some((_, sel, multi)) = &mut this.slicer {
                            if *multi {
                                if !sel.remove(&v) {
                                    sel.insert(v.clone());
                                }
                            } else if sel.len() == 1 && sel.contains(&v) {
                                sel.clear(); // 同じ釦をもう一度 = 解除
                            } else {
                                sel.clear();
                                sel.insert(v.clone());
                            }
                            this.status = if sel.is_empty() {
                                ui::t!("絞りなし(全部見えています)").into()
                            } else {
                                ui::tf!("絞り: {}(見え方だけ。中身は変わりません)", sel.iter().cloned().collect::<Vec<_>>().join(" / "))
                                .into()
                            };
                        }
                        cx.notify();
                    })));
            }
            p
        });

        // ---- 図形の設定(選ぶと右に出る) ----
        // 塗り・線・太さ・不透明度・回転/反転・影。どの釦も shape_edit を
        // 通る=1手ずつ戻せる。折れ線もの(スパークライン・ペン)は色と太さだけ
        let shape_panel = self.shape_sel.and_then(|si| {
            let sp = self.sheet().shapes_new.get(si)?;
            let poly = matches!(
                sp.kind.as_str(),
                "spark" | "spark-col" | "spark-wl" | "ink" | "marker"
            );
            let (has_fill, has_line) = (sp.fill.is_some(), sp.line.is_some());
            let cur_fill = sp.fill.clone().unwrap_or_default();
            let cur_line = sp.line.clone().unwrap_or_default();
            let (cur_w, cur_a, cur_rot, shadow_on) = (sp.line_w, sp.alpha, sp.rot, sp.shadow);
            let lab = |t: String| {
                div().text_size(px(us * 10.5)).text_color(rgb(0x66707A))
                    .w(px(us * 52.0)).flex_none()
                    .child(SharedString::from(t))
            };
            let chip = |id: String, t: String, on: bool| {
                div().id(SharedString::from(id))
                    .px_1p5().py_0p5().rounded_sm().border_1()
                    .border_color(if on { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                    .bg(if on { rgb(0xCFE6D8) } else { rgb(0xFFFFFF) })
                    .text_size(px(us * 11.0)).text_color(rgb(0x1B1B1B))
                    .cursor_pointer().hover(|s| s.bg(rgb(0xEAF5EE)))
                    .child(SharedString::from(t))
            };
            let swatch = |id: String, c: Option<gpui::Rgba>| {
                let mut s = div().id(SharedString::from(id))
                    .w(px(16.0)).h(px(16.0)).rounded_sm().flex_none()
                    .border_1().border_color(rgb(0xC6CDD3));
                s = match c {
                    Some(v) => s.bg(v),
                    None => s.bg(rgb(0xFFFFFF)).child(
                        div().text_size(px(us * 9.0)).text_color(rgb(0xC6CDD3)).child("╱"),
                    ),
                };
                s
            };
            let mut p = div().absolute().right(px(24.0)).top(px(ROW_H + 16.0)).w(px(us * 235.0))
                .p_2().rounded_md().bg(gpui::white())
                .border_1().border_color(rgb(0x1B6E3C)).shadow_lg()
                .flex().flex_col().gap_1p5()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(div().flex().flex_row().items_center()
                    .child(div().text_size(px(us * 12.0)).font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(0x1B6E3C))
                        .child(SharedString::from(ui::t!("図形の設定").to_string())))
                    .child(div().flex_1())
                    .child(div().id("shp-close").px_1p5().rounded_sm().cursor_pointer()
                        .text_size(px(us * 12.0)).hover(|s| s.bg(rgb(0xEAF5EE)))
                        .child("✕")
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.shape_sel = None;
                            cx.notify();
                        }))));
            // 塗りと線の色(RGB 直指定の板を開く / なし)
            if !poly {
                let cf = cur_fill.clone();
                p = p.child(div().flex().flex_row().items_center().gap_1()
                    .child(lab(ui::t!("塗り").to_string()))
                    .child(swatch("shp-fill-sw".into(),
                        has_fill.then(|| hex(&cur_fill))))
                    .child(chip("shp-fill-set".into(), ui::t!("色…").to_string(), false)
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.prompt = Some(("shape-fill-rgb", Editor::new(&cf)));
                            cx.notify();
                        })))
                    .child(chip("shp-fill-no".into(), ui::t!("なし").to_string(), !has_fill)
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.shape_edit(|sp| sp.fill = None);
                            this.status = ui::t!("塗りを消しました").into();
                            cx.notify();
                        }))));
            }
            {
                let cl = cur_line.clone();
                p = p.child(div().flex().flex_row().items_center().gap_1()
                    .child(lab(ui::t!("線").to_string()))
                    .child(swatch("shp-line-sw".into(),
                        has_line.then(|| hex(&cur_line))))
                    .child(chip("shp-line-set".into(), ui::t!("色…").to_string(), false)
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.prompt = Some(("shape-line-rgb", Editor::new(&cl)));
                            cx.notify();
                        })))
                    .child(chip("shp-line-no".into(), ui::t!("なし").to_string(), !has_line)
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.shape_edit(|sp| sp.line = None);
                            this.status = ui::t!("線を消しました").into();
                            cx.notify();
                        }))));
            }
            // 線の太さ(pt)。押したとき線が無ければ既定の緑で引く
            {
                let mut row = div().flex().flex_row().items_center().gap_1().flex_wrap()
                    .child(lab(ui::t!("太さ").to_string()));
                for v in [0.5f32, 1.0, 1.5, 2.25, 3.0, 4.5, 6.0] {
                    let on = (cur_w - v).abs() < 0.05;
                    let t = if v.fract() == 0.0 {
                        format!("{v:.0}")
                    } else {
                        format!("{v}")
                    };
                    row = row.child(chip(format!("shp-w{}", (v * 100.0) as i32), t, on)
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.shape_edit(move |sp| {
                                sp.line_w = v;
                                if sp.line.is_none() {
                                    sp.line = Some("1B6E3C".into());
                                }
                            });
                            this.status = ui::tf!("線の太さ: {} pt", format!("{v}")).into();
                            cx.notify();
                        })));
                }
                p = p.child(row);
            }
            // 不透明度(%)
            {
                let mut row = div().flex().flex_row().items_center().gap_1()
                    .child(lab(ui::t!("不透明度").to_string()));
                for v in [100i32, 75, 50, 25] {
                    let a = v as f32 / 100.0;
                    let on = (cur_a - a).abs() < 0.03;
                    row = row.child(chip(format!("shp-a{v}"), format!("{v}%"), on)
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.shape_edit(move |sp| sp.alpha = a);
                            this.status = ui::tf!("不透明度: {}%", v).into();
                            cx.notify();
                        })));
                }
                p = p.child(row);
            }
            // 回転・反転と影(折れ線ものには効かないので出さない)
            if !poly {
                let mut row = div().flex().flex_row().items_center().gap_1().flex_wrap()
                    .child(lab(ui::t!("回転").to_string()));
                for (id, t, d) in [("shp-rl", "↺90", -90.0f32), ("shp-rr", "↻90", 90.0)] {
                    row = row.child(chip(id.into(), t.into(), false)
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.shape_edit(move |sp| sp.rot = (sp.rot + d).rem_euclid(360.0));
                            this.status = ui::t!("90度回しました").into();
                            cx.notify();
                        })));
                }
                row = row.child(chip("shp-fh".into(), ui::t!("左右反転").to_string(), false)
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.shape_edit(|sp| sp.flip_h = !sp.flip_h);
                        this.status = ui::t!("左右に反転しました").into();
                        cx.notify();
                    })));
                row = row.child(chip("shp-fv".into(), ui::t!("上下反転").to_string(), false)
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.shape_edit(|sp| sp.flip_v = !sp.flip_v);
                        this.status = ui::t!("上下に反転しました").into();
                        cx.notify();
                    })));
                {
                    let cr = format!("{cur_rot:.0}");
                    row = row.child(chip("shp-deg".into(), ui::tf!("角度…({}°)", cr.clone()), false)
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.prompt = Some(("shape-rot",
                                Editor::new(if cr == "0" { "" } else { &cr })));
                            cx.notify();
                        })));
                }
                p = p.child(row);
                p = p.child(div().flex().flex_row().items_center().gap_1()
                    .child(lab(ui::t!("影").to_string()))
                    .child(chip("shp-shadow".into(),
                        if shadow_on { ui::t!("あり").to_string() } else { ui::t!("なし").to_string() },
                        shadow_on)
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            let mut now = false;
                            this.shape_edit(|sp| { sp.shadow = !sp.shadow; now = sp.shadow; });
                            this.status = if now {
                                ui::t!("影を付けました").into()
                            } else {
                                ui::t!("影を消しました").into()
                            };
                            cx.notify();
                        }))));
            }
            Some(p)
        });

        // ---- 書式の小窓(セルをフォーマットする) ----
        // モーダルにしない: 範囲を選び直しながら続けて使える道具箱。
        // どの釦も既存の書式の道(fmt / run_cmd)を通り、1手ずつ戻せる
        let fmt_panel = self.fmt_panel.map(|(fx, fy)| {
            let fx = fx.min(560.0);
            let fy = fy.min(320.0);
            let btn = |id: &'static str, label: &'static str| {
                div().id(id).px_2().py_1().rounded_sm()
                    .border_1().border_color(rgb(0xC6CDD3))
                    .text_size(px(us * 11.5)).text_color(rgb(0x1B1B1B))
                    .cursor_pointer().hover(|s| s.bg(rgb(0xEAF5EE)))
                    .child(label)
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                        move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.fmt_panel_action(id, cx);
                            cx.notify();
                        }))
            };
            let swatch = |id: &'static str, color: Option<&'static str>| {
                let mut s = div().id(id).w(px(20.0)).h(px(20.0)).rounded_sm()
                    .border_1().border_color(rgb(0xC6CDD3))
                    .cursor_pointer();
                s = match color {
                    Some(c) => s.bg(hex(c)),
                    // 「なし」は斜線の代わりに白+薄字の×
                    None => s.bg(rgb(0xFFFFFF)).flex().items_center().justify_center()
                        .text_size(px(us * 10.0)).text_color(rgb(0x9AA5AE)).child("×"),
                };
                s.on_mouse_down(gpui::MouseButton::Left, cx.listener(
                    move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.fmt_panel_action(id, cx);
                        cx.notify();
                    }))
            };
            let title = |t: &'static str| div().text_size(px(us * 10.5))
                .text_color(rgb(0x66707A)).mt_1p5().child(t);
            let row = || div().flex().flex_row().flex_wrap().gap_1().items_center();

            div().absolute().left(px(fx)).top(px(fy)).w(px(300.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3)).shadow_lg()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(div().flex().flex_row().items_center().justify_between()
                    .child(div().text_size(px(us * 12.5)).font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(0x1B6E3C))
                        .child(ui::t!("セルの書式(選んでいる範囲に効く)")))
                    .child(div().id("fmtclose").px_2().rounded_sm().cursor_pointer()
                        .text_size(px(us * 12.0)).text_color(rgb(0x66707A))
                        .hover(|s| s.bg(rgb(0xE1E6EA)))
                        .child("✕")
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                            move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.fmt_panel = None;
                                cx.notify();
                            }))))
                .child(title("罫線"))
                .child(row()
                    .child(btn("b-all", ui::t!("格子")))
                    .child(btn("b-out", ui::t!("外枠")))
                    .child(btn("b-none", ui::t!("なし"))))
                .child(title("塗り"))
                .child(row()
                    .child(swatch("fill-none", None))
                    .child(swatch("fill-FFF2CC", Some("FFF2CC")))
                    .child(swatch("fill-DEEAF6", Some("DEEAF6")))
                    .child(swatch("fill-E2EFDA", Some("E2EFDA")))
                    .child(swatch("fill-FCE4D6", Some("FCE4D6")))
                    .child(swatch("fill-D9D9D9", Some("D9D9D9"))))
                .child(title("文字の色"))
                .child(row()
                    .child(swatch("color-none", None))
                    .child(swatch("color-C00000", Some("C00000")))
                    .child(swatch("color-1F4E79", Some("1F4E79")))
                    .child(swatch("color-1B6E3C", Some("1B6E3C")))
                    .child(swatch("color-7F7F7F", Some("7F7F7F"))))
                .child(title("文字"))
                .child(row()
                    .child(btn("bold", ui::t!("太字")))
                    .child(btn("italic", ui::t!("斜体")))
                    .child(btn("underline", ui::t!("下線")))
                    .child(btn("strikeout", ui::t!("取り消し")))
                    .child(btn("incfont", ui::t!("大きく")))
                    .child(btn("decfont", ui::t!("小さく"))))
                .child(title("揃え"))
                .child(row()
                    .child(btn("align-left", ui::t!("左")))
                    .child(btn("align-center", ui::t!("中央")))
                    .child(btn("align-right", ui::t!("右")))
                    .child(btn("top", ui::t!("上")))
                    .child(btn("middle", ui::t!("中")))
                    .child(btn("bottom", ui::t!("下")))
                    .child(btn("wrap", ui::t!("折り返し"))))
                .child(title("表示形式"))
                .child(row()
                    .child(btn("comma", "1,000"))
                    .child(btn("currency", "¥"))
                    .child(btn("percents", "%"))
                    .child(btn("digit-inc", ".0+"))
                    .child(btn("digit-dec", ".0−"))
                    .child(btn("numfmt-none", ui::t!("なし"))))
        });

        // ---- ドロップダウンリスト(同じ列の値の一覧) ----
        // ---- 罫線のアイコンの格子パレット(発注者 2026-08-08) ----
        // 型を絵で選ぶ。掛けても閉じない(連打で帳票の枠を組み立てる)。
        // アイコンは div の重ね棒 — SVG 資産が要らず、ペンの色にも追従する
        let border_palette = self.border_pal.map(|(vx, vy)| {
            let pen = self.pen_color.map(|v| rgb(v)).unwrap_or(rgb(0x1B1B1B));
            let faint = rgb(0xD5DBE0);
            // ペンの見た目をそのまま絵に映す(太さと二重線。破線の刻みまでは
            // 描かない — 絵は場所の案内、線種の正確な見本はスタイルの一覧)
            let pw = self.pen_style.px().clamp(1.0, 4.0);
            let double = self.pen_style == sheet::model::BStyle::Double;
            // 1コマのアイコン(24×24)。濃い線 = ペン、薄い線 = セルの気配
            let icon = move |kind: &'static str| -> gpui::AnyElement {
                let base = div().relative().w(px(us * 24.0)).h(px(us * 24.0));
                let bar = move |edge: u8, t: f32, on: bool, inset: f32| -> gpui::AnyElement {
                    let c = if on { pen } else { faint };
                    let b = div().absolute();
                    let b = match edge {
                        0 => b.left(px(inset)).right(px(inset)).top(px(inset)).h(px(t)),
                        1 => b.left(px(inset)).right(px(inset)).bottom(px(inset)).h(px(t)),
                        2 => b.top(px(inset)).bottom(px(inset)).left(px(inset)).w(px(t)),
                        _ => b.top(px(inset)).bottom(px(inset)).right(px(inset)).w(px(t)),
                    };
                    b.bg(c).into_any_element()
                };
                // ペンの線で1辺(二重線は細い2本)
                let pen_edge = move |edge: u8| -> Vec<gpui::AnyElement> {
                    if double {
                        vec![bar(edge, 1.0, true, 3.0), bar(edge, 1.0, true, 6.0)]
                    } else {
                        vec![bar(edge, pw, true, 3.0)]
                    }
                };
                let mid_h = move || -> Vec<gpui::AnyElement> {
                    let one = |off: f32, t: f32| div().absolute()
                        .left(px(3.0)).right(px(3.0)).top(px(us * 12.0 - t / 2.0 + off)).h(px(t))
                        .bg(pen).into_any_element();
                    if double { vec![one(-1.5, 1.0), one(1.5, 1.0)] } else { vec![one(0.0, pw)] }
                };
                let mid_v = move || -> Vec<gpui::AnyElement> {
                    let one = |off: f32, t: f32| div().absolute()
                        .top(px(3.0)).bottom(px(3.0)).left(px(us * 12.0 - t / 2.0 + off)).w(px(t))
                        .bg(pen).into_any_element();
                    if double { vec![one(-1.5, 1.0), one(1.5, 1.0)] } else { vec![one(0.0, pw)] }
                };
                let mut kids: Vec<gpui::AnyElement> = Vec::new();
                // セルの気配(薄い枠)はいつも敷く
                for e in 0..4u8 {
                    kids.push(bar(e, 1.0, false, 3.0));
                }
                match kind {
                    "下罫線" => kids.extend(pen_edge(1)),
                    "上罫線" => kids.extend(pen_edge(0)),
                    "左罫線" => kids.extend(pen_edge(2)),
                    "右罫線" => kids.extend(pen_edge(3)),
                    "外枠" => {
                        for e in 0..4u8 {
                            kids.extend(pen_edge(e));
                        }
                    }
                    "すべての罫線(格子)" => {
                        for e in 0..4u8 {
                            kids.extend(pen_edge(e));
                        }
                        kids.extend(mid_h());
                        kids.extend(mid_v());
                    }
                    "内側の縦線" => kids.extend(mid_v()),
                    "内側の横線" => kids.extend(mid_h()),
                    _ => {} // 罫線を消す = 薄い枠だけ
                }
                base.children(kids).into_any_element()
            };
            // 場所だけを選ぶ9種。太さ・線種・色は**ペンだけ**が決める —
            // 「太い下罫線」のような太さ焼き込みの型は持たない
            // (発注者 2026-08-08「Microsoft 方式はやめて」)
            const KINDS: &[&str] = &[
                "下罫線", "上罫線", "左罫線", "右罫線",
                "外枠", "すべての罫線(格子)",
                "内側の縦線", "内側の横線",
                "罫線を消す",
            ];
            let style_name = crate::util::BORDER_STYLES
                .iter()
                .find(|(_, b)| *b == self.pen_style)
                .map(|(n, _)| *n)
                .unwrap_or("細い実線(既定)");
            let color_name = match self.pen_color {
                Some(v) => format!("#{v:06X}"),
                None => ui::t!("自動(黒)").to_string(),
            };
            let mut pal = div().id("border-pal").absolute().left(px(vx)).top(px(vy))
                .w(px(us * 176.0))
                .p_1().rounded_md().bg(rgb(0xFFFFFF))
                .border_1().border_color(rgb(0xC6CDD3)).shadow_lg()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(div().px_1().py_0p5().mb_0p5()
                    .border_b_1().border_color(rgb(0xE1E6EA))
                    .text_size(px(us * 10.5)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x1B6E3C))
                    .whitespace_nowrap().overflow_hidden()
                    .child(SharedString::from(ui::t!("罫線(連続で押せます。Esc で閉じる)").to_string())));
            for row_kinds in KINDS.chunks(4) {
                let mut r = div().flex().flex_row().gap_0p5();
                for kind in row_kinds {
                    let k: &'static str = kind;
                    r = r.child(div()
                        .id(SharedString::from(format!("bp-{k}")))
                        .p_0p5().rounded_sm().cursor_pointer()
                        .hover(|s| s.bg(rgb(0xEAF5EE)))
                        .tooltip({
                            let k2 = k;
                            move |_, cx| cx.new(|_| Tip(k2.into(), us)).into()
                        })
                        .child(icon(k))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                            move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.apply_borders(k);
                                cx.notify();
                            })));
                }
                pal = pal.child(r);
            }
            // 下段: ペン(スタイル・色)。今の値を見せる
            pal = pal.child(div().mt_0p5().pt_0p5()
                .border_t_1().border_color(rgb(0xE1E6EA))
                .child(div()
                    .id("bp-style")
                    .px_1().py_0p5().rounded_sm().cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF5EE)))
                    .text_size(px(us * 11.5)).text_color(rgb(0x1B1B1B))
                    .whitespace_nowrap().overflow_hidden()
                    .child(SharedString::from(ui::tf!("線のスタイル: {}…", style_name).to_string()))
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                        |this, _, _, cx| {
                            cx.stop_propagation();
                            this.border_pal = None;
                            this.open_border_style_pick();
                            cx.notify();
                        })))
                .child(div()
                    .id("bp-color")
                    .px_1().py_0p5().rounded_sm().cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF5EE)))
                    .flex().flex_row().items_center().gap_1()
                    .text_size(px(us * 11.5)).text_color(rgb(0x1B1B1B))
                    .child(div().w(px(10.0)).h(px(10.0)).rounded_xs().bg(pen)
                        .border_1().border_color(rgb(0xC6CDD3)))
                    .child(SharedString::from(ui::tf!("線の色: {}…", color_name).to_string()))
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                        |this, _, _, cx| {
                            cx.stop_propagation();
                            this.border_pal = None;
                            this.open_border_color_pick();
                            cx.notify();
                        }))));
            pal
        });

        let pick_panel = self.pick.clone().map(|(vals, (vx, vy))| {
            // 色の一覧(文字の色・塗り)は名前の左に色見本の四角を添える
            let swatch_of = |name: &str| -> Option<Option<&'static str>> {
                match self.pick_kind {
                    "font-color" => FONT_COLORS.iter().find(|(n, _)| *n == name).map(|(_, h)| *h),
                    "fill-color" => FILL_COLORS.iter().find(|(n, _)| *n == name).map(|(_, h)| *h),
                    _ => None,
                }
            };
            // 長い一覧(書体など)は板の中でスクロール — 数で切り捨てない
            let mut p = div().id("pick-list").absolute().left(px(vx)).top(px(vy))
                .w(px(self.col_px(self.cursor.col).max(if self.pick_note.is_some() { 300.0 } else { 120.0 })))
                .max_h(px((self.view_h_px - 160.0).max(160.0)))
                .overflow_y_scroll()
                .p_1().rounded_md().bg(rgb(0xFFFFFF))
                .border_1().border_color(rgb(0xC6CDD3)).shadow_lg()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation());
            // 題(いま何を選んでいるか)。ピボットの段の案内など
            if let Some(note) = &self.pick_note {
                p = p.child(div().px_2().py_1().mb_0p5()
                    .border_b_1().border_color(rgb(0xE1E6EA))
                    .text_size(px(us * 11.0)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x1B6E3C))
                    .whitespace_nowrap()
                    .child(note.clone()));
            }
            for (i, v) in vals.into_iter().enumerate() {
                let sw = swatch_of(&v);
                p = p.child(div()
                    .id(SharedString::from(format!("pk{i}")))
                    .px_2().py_1().rounded_sm().cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF5EE)))
                    .flex().flex_row().items_center().gap_2()
                    .text_size(px(us * 12.5))
                    // 「→ 」は次の段へ進む釦 — 並びの項目と見分ける
                    .text_color(if v.starts_with("→ ") { rgb(0x1B6E3C) } else { rgb(0x1B1B1B) })
                    .when(v.starts_with("→ "), |s| s
                        .font_weight(gpui::FontWeight::BOLD)
                        .border_t_1().border_color(rgb(0xE1E6EA)).mt_0p5())
                    .whitespace_nowrap().overflow_hidden()
                    .children(sw.map(|hx| {
                        let q = div().w(px(14.0)).h(px(14.0)).rounded_sm()
                            .border_1().border_color(rgb(0xC6CDD3));
                        match hx {
                            Some(h) => q.bg(hex(h)),
                            None => q.bg(rgb(0xFFFFFF)),
                        }
                    }))
                    .child(SharedString::from(v.clone()))
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                        move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.pick = None;
                            this.apply_pick(&v, cx);
                            cx.notify();
                        })));
            }
            p
        });

        let notes = if self.notes.is_empty() { None } else {
            let mut n = div().px_4().py_2().bg(rgb(0xFFF6E6))
                .border_t_1().border_color(rgb(0xE8D5A8))
                .child(div().text_size(px(us * 11.5)).font_weight(gpui::FontWeight::BOLD)
                       .text_color(rgb(0x8A4B00)).child(ui::t!("この版で読み飛ばしたもの")));
            for x in &self.notes {
                n = n.child(div().text_size(px(us * 11.0)).text_color(rgb(0x8A4B00))
                            .child(x.clone()));
            }
            Some(n)
        };

        let me: Entity<Calc> = cx.entity();
        div().size_full().flex().flex_col().bg(rgb(0xF3F5F7))
            .key_context("jo_edit")
            .track_focus(&self.focus)
            .on_action(cx.listener(Calc::a_backspace))
            .on_action(cx.listener(Calc::a_delete))
            .on_action(cx.listener(Calc::a_copy))
            .on_action(cx.listener(Calc::a_cut))
            .on_action(cx.listener(Calc::a_paste))
            .on_action(cx.listener(Calc::a_paste_values))
            .on_action(cx.listener(Calc::a_left))
            .on_action(cx.listener(Calc::a_right))
            .on_action(cx.listener(Calc::a_up))
            .on_action(cx.listener(Calc::a_down))
            .on_action(cx.listener(Calc::a_page_up))
            .on_action(cx.listener(Calc::a_page_down))
            .on_action(cx.listener(Calc::a_doc_home))
            .on_action(cx.listener(Calc::a_doc_end))
            .on_action(cx.listener(Calc::a_tab))
            .on_action(cx.listener(Calc::a_enter))
            .on_action(cx.listener(Calc::a_select_all))
            .on_action(cx.listener(Calc::a_redo))
            .on_action(cx.listener(Calc::a_select_left))
            .on_action(cx.listener(Calc::a_select_right))
            .on_action(cx.listener(Calc::a_select_up))
            .on_action(cx.listener(Calc::a_select_down))
            .on_action(cx.listener(Calc::a_undo))
            .on_action(cx.listener(Calc::a_save))
            .on_action(cx.listener(Calc::a_open))
            .on_action(cx.listener(Calc::a_quit))
            .on_action(cx.listener(Calc::a_context_menu))
            .on_action(cx.listener(Calc::a_cancel))
            .on_action(cx.listener(Calc::a_edit_cell))
            .on_action(cx.listener(Calc::a_recalc))
            .on_action(cx.listener(Calc::a_recalc_sheet))
            .on_action(cx.listener(Calc::a_newline))
            .on_action(cx.listener(Calc::a_ui_bigger))
            .on_action(cx.listener(Calc::a_ui_smaller))
            .on_action(cx.listener(Calc::a_ins_link))
            .child(bar)
            .children((self.tab != 0 && self.show_formula_bar).then(|| formula_bar))
            .child(div().flex_1().overflow_hidden().relative()
                   // ホイールで窓を動かす(下に回すと先の行が見える)
                   .on_scroll_wheel(cx.listener(|this, e: &gpui::ScrollWheelEvent, _, cx| {
                       // Ctrl+ホイール = 格子の拡大縮小(Excel と同じ)
                       if e.modifiers.control {
                           let up = match e.delta {
                               gpui::ScrollDelta::Pixels(p) => f32::from(p.y) > 0.0,
                               gpui::ScrollDelta::Lines(l) => l.y > 0.0,
                           };
                           this.run_cmd(if up { "zoom-in" } else { "zoom-out" }, cx);
                           cx.notify();
                           return;
                       }
                       let (dx, dy) = match e.delta {
                           gpui::ScrollDelta::Pixels(p) =>
                               (-f32::from(p.x) / COL_W, -f32::from(p.y) / ROW_H),
                           gpui::ScrollDelta::Lines(l) => (-l.x, -l.y * 3.0),
                       };
                       this.wheel.0 += dy;
                       this.wheel.1 += dx;
                       let dr = this.wheel.0.trunc() as i32;
                       let dc = this.wheel.1.trunc() as i32;
                       this.wheel.0 -= dr as f32;
                       this.wheel.1 -= dc as f32;
                       if dr != 0 || dc != 0 {
                           this.view.row = (this.view.row as i32 + dr).clamp(0, 9999) as u32;
                           this.view.col = (this.view.col as i32 + dc).clamp(0, 255) as u32;
                           cx.notify();
                       }
                   }))
                   .child(grid)
                   .children(merge_overlays)
                   .children(pivot_frames)
                   .children(freeze_shadow)
                   .children(ink_preview)
                   .children({
                       // 浮かぶ画像(グラフ)。錨のセルが見えている間だけ描く。
                       // マウスは受けない(セルの操作を遮らない)
                       let mut layer: Vec<gpui::AnyElement> = Vec::new();
                       for im in self.sheet().images.iter().chain(self.sheet().images_new.iter()) {
                           let Some((x, y)) = self.cell_origin_px(im.at) else { continue };
                           let (x, y) = (x + im.dx_px, y + im.dy_px);
                           let key = im.data.as_ptr() as usize;
                           let src = self
                               .img_cache
                               .borrow_mut()
                               .entry(key)
                               .or_insert_with(|| {
                                   let fmt = if im.data.starts_with(&[0xFF, 0xD8]) {
                                       gpui::ImageFormat::Jpeg
                                   } else if im.data.starts_with(b"GIF8") {
                                       gpui::ImageFormat::Gif
                                   } else if im.data.starts_with(b"BM") {
                                       gpui::ImageFormat::Bmp
                                   } else {
                                       gpui::ImageFormat::Png
                                   };
                                   std::sync::Arc::new(gpui::Image::from_bytes(
                                       fmt,
                                       im.data.clone(),
                                   ))
                               })
                               .clone();
                           layer.push(
                               gpui::img(src)
                                   .absolute()
                                   .left(px(x))
                                   .top(px(y))
                                   .w(px(im.width_px))
                                   .h(px(im.height_px))
                                   .into_any_element(),
                           );
                       }
                       // 図形(SVG)。大きさを織り込んで作るので、伸ばしても鮮明
                       for (i, sp) in self
                           .sheet()
                           .shapes
                           .iter()
                           .chain(self.sheet().shapes_new.iter())
                           .enumerate()
                       {
                           let Some((x, y)) = self.cell_origin_px(sp.at) else { continue };
                           let (x, y) = (x + sp.dx_px, y + sp.dy_px);
                           // 回転・影のはみ出しぶんキャンバスが四方に広い
                           let pad = sp.pad();
                           let svg = sp.to_svg();
                           let key = {
                               use std::hash::{Hash, Hasher};
                               let mut h = std::collections::hash_map::DefaultHasher::new();
                               svg.hash(&mut h);
                               h.finish() as usize
                           };
                           let src = self
                               .img_cache
                               .borrow_mut()
                               .entry(key)
                               .or_insert_with(|| {
                                   std::sync::Arc::new(gpui::Image::from_bytes(
                                       gpui::ImageFormat::Svg,
                                       svg.into_bytes(),
                                   ))
                               })
                               .clone();
                           layer.push(
                               gpui::img(src)
                                   .absolute()
                                   .left(px(x - pad))
                                   .top(px(y - pad))
                                   .w(px(sp.width_px.max(4.0) + pad * 2.0))
                                   .h(px(sp.height_px.max(4.0) + pad * 2.0))
                                   .into_any_element(),
                           );
                           if let Some(t) = &sp.text {
                               layer.push(
                                   div()
                                       .absolute()
                                       .left(px(x + 6.0))
                                       .top(px(y + 4.0))
                                       .w(px((sp.width_px - 12.0).max(8.0)))
                                       .h(px((sp.height_px - 8.0).max(8.0)))
                                       .overflow_hidden()
                                       .text_size(px(us * 12.5))
                                       .font_family(self.font_name.clone())
                                       .text_color(rgb(0x1B1B1B))
                                       .whitespace_normal()
                                       .child(SharedString::from(t.clone()))
                                       .into_any_element(),
                               );
                           }
                           let _ = i;
                       }
                       // 控えが育ちすぎたら捨てる(undo のクローンで鍵が増えるため)
                       if self.img_cache.borrow().len() > 64 {
                           self.img_cache.borrow_mut().clear();
                       }
                       layer
                   })
                   .child(InputSink { view: me })
                   .children(shape_frame)
                   .children(shape_frames_more)
                   .children(img_frame)
                   .children(ants)
                   .children(tip)
                   .children(fmt_panel)
                   .children(menu)
                   .children(filepage)
                   .children(border_palette)
                   .children(pick_panel)
                   .children(prompt_panel)
                   .children(dv_panel)
                   .children(solver_panel)
                   .children(fn_panel)
                   .children(fn_args_panel)
                   .children(quit_panel)
                   .children(shape_panel)
                   .children(slicer_panel))
            .children(watch_bar)
            .child(sheets_bar)
            .children(notes)
            // 窓の縁のつかみ(最後に描く = 最初にマウスを受ける)
            .children(ui::resize_edges(window))
    }
}

//! writer の画面(main.rs から純移動 2026-08-08。部屋割りの2歩目)。
//! impl Render(紙面の描画・リボン・板)と InputSink(入力とマウスの受け皿)。
//! **純移動** — 挙動と文言は一切変えない

use crate::*;

impl Render for Writer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let me: Entity<Writer> = cx.entity();
        // 画面の倍率(紙のミリは変えず、画素への写像だけ変える)
        let pxmm = PX_PER_MM * self.zoom;
        // 編集領域の高さを実測しておく(キャレット追従・スクロールの止めに使う)。
        // リボンのぶん(約110px)を引いた近似で足りる
        self.view_h_px = (f32::from(window.viewport_size().height) - 136.0).max(100.0);
        self.view_w_px = f32::from(window.viewport_size().width).max(200.0);
        let marked = self.ed.marked_range();
        let (cx_mm, cy_mm, caret_pt) = self.caret_xy();

        // ---- リボン(Euro-Office に名前と並びを合わせる) ----
        // **タブの行そのものが窓の取っ手**(掴んで移動・二度押しで最大化)。
        // 空きの帯だけを取っ手にすると、タブが多い窓では幅がゼロになり
        // 掴む場所が無くなる(踏んで直した)。釦の類いは stop_propagation で
        // 取っ手より先に効く
        let (ready, all) = ribbon::progress(ribbon::writer_tabs());
        // ダークモードは**紙以外**を暗くする — 紙は白いまま(印刷と同じ)。
        // 文書は何も変わらない(見え方だけ)
        let dk = self.dark;
        let th_tab_on_bg = if dk { rgb(0x22262A) } else { rgb(0xFFFFFF) };
        let th_tab_on_fg = if dk { rgb(0xCFE0EA) } else { rgb(0x165E83) };
        let th_cmd_bg = if dk { rgb(0x22262A) } else { rgb(0xFFFFFF) };
        let th_cmd_border = if dk { rgb(0x33383D) } else { rgb(0xE1E6EA) };
        let th_btn = if dk { rgb(0x7FB2D0) } else { rgb(0x165E83) };
        let th_btn_hover = if dk { rgb(0x2C333A) } else { rgb(0xEAF2F7) };
        let th_gray_border = if dk { rgb(0x2E3338) } else { rgb(0xEDEFF1) };
        let th_gray_fg = if dk { rgb(0x565D64) } else { rgb(0xB6BDC4) };
        let th_status = if dk { rgb(0x9AA5AE) } else { rgb(0x66707A) };
        let th_desk = if dk { rgb(0x191C1F) } else { rgb(0x63686D) };
        // デスクトップ版の額縁: 1段目がクイックアクセス+文書名(=取っ手)、
        // 2段目が下線つきのタブ(現在地は青い下線)、3段目が釦の帯
        let th_top_bg = if dk { rgb(0x1B1E21) } else { rgb(0xF1F3F5) };
        let th_top_fg = if dk { rgb(0xCFD6DC) } else { rgb(0x444B52) };
        let th_qa_hover = if dk { rgb(0x2C333A) } else { rgb(0xE2E6EA) };
        let qa = |id: &'static str, icon: &'static str| {
            div().id(id).px_2().py_1().rounded_sm().cursor_pointer()
                .hover(move |s| s.bg(th_qa_hover))
                .child(gpui::svg()
                    .path(SharedString::from(format!("icons/{icon}.svg")))
                    .size(px(15.0)).text_color(th_top_fg))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let title = self
            .path
            .as_ref()
            .and_then(|q| q.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| ui::t!("無題のドキュメント").into());
        let winbtn = |id: &'static str, label: &'static str| {
            div().id(id).px_2p5().py_1().rounded_sm()
                .text_size(px(12.0)).text_color(th_top_fg)
                .cursor_pointer()
                .hover(move |s| if id == "close" { s.bg(rgb(0xC0392B)).text_color(rgb(0xFFFFFF)) }
                                else { s.bg(rgb(0x2C7DA6)).text_color(rgb(0xFFFFFF)) })
                .child(label)
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let top = div().id("titlebar").flex().flex_row().items_center().gap_0p5()
            .px_2().py_0p5().bg(th_top_bg)
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
            .child(div().text_size(px(12.5)).text_color(th_top_fg)
                .whitespace_nowrap().overflow_hidden()
                .child(SharedString::from(format!(
                    "{}{title}",
                    if self.dirty { "*" } else { "" }
                ))))
            .child(div().flex_1())
            .child(div().pr_2().text_size(px(10.5))
                .text_color(if dk { rgb(0x6E7982) } else { rgb(0x8A949D) })
                .child(SharedString::from(ui::tf!("writer — 実装済み {}/{}", ready, all))))
            .child(winbtn("min", "─").on_click(cx.listener(|_, _, window, _| {
                window.minimize_window();
            })))
            .child(winbtn("max", "▢").on_click(cx.listener(|_, _, window, _| {
                window.zoom_window();
            })))
            .child(winbtn("close", "✕").on_click(cx.listener(|this, _, _, cx| {
                this.request_quit(cx);
            })));

        let th_tab_idle = if dk { rgb(0x9AA5AE) } else { rgb(0x555E66) };
        let mut tabs = div().flex().flex_row().items_end().gap_1()
            .px_2().bg(th_tab_on_bg);
        for (i, tb) in ribbon::writer_tabs().iter().enumerate() {
            let on = i == self.tab;
            tabs = tabs.child(div()
                .id(SharedString::from(format!("tab{i}")))
                .px_2p5().pt_1p5()
                .text_size(px(12.0))
                .text_color(if on { th_tab_on_fg } else { th_tab_idle })
                .font_weight(if on { gpui::FontWeight::BOLD } else { gpui::FontWeight::NORMAL })
                .cursor_pointer()
                .hover(move |s| s.text_color(th_tab_on_fg))
                .flex().flex_col().items_center().gap_1()
                .child(tb.name)
                // 現在地の青い下線(デスクトップ版の形)
                .child(div().h(px(2.5)).w_full().rounded_sm()
                    .bg(if on { th_btn } else { th_tab_on_bg }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if i == 0 && this.tab != 0 {
                        this.prev_tab = this.tab;
                        this.file_view = 0;
                        this.file_field = None;
                    }
                    this.tab = i;
                    cx.notify()
                })));
        }
        tabs = tabs.child(div().flex_1())
            .child(div().id("tab-find").px_2().pb_1().text_size(px(12.0))
                .text_color(th_tab_idle).cursor_pointer()
                .hover(move |s| s.text_color(th_tab_on_fg))
                .child("🔍")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.run_cmd("replace", cx);
                    cx.notify()
                })));

        let mut cmds = div().flex().flex_col().gap_0p5()
            .px_3().py_1().bg(th_cmd_bg)
            .border_b_1().border_color(th_cmd_border);
        // 本家風のタブ配置。(id, 大釦の名札)。"‖" は群の区切り線。
        // 名札つきは絵の下に短い名前(本家の言い方)、無印は絵だけの釦。
        // 釦の名前は乗ったときに下のステータスバーへ出す(hover_hint)
        type LItem = (&'static str, Option<&'static str>);
        // ホームは2段(発注者の画像 2026-08-04)
        const HOME_ROWS: &[&[LItem]] = &[
            &[
                ("copy", None), ("cut", None), ("‖", None), ("fontname", None),
                ("fontsize", None), ("incfont", None), ("decfont", None),
                ("changecase", None), ("ruby", None), ("‖", None),
                ("markers", None),
                ("numbering", None), ("multilevels", None), ("decoffset", None),
                ("incoffset", None), ("linespace", None), ("direction", None),
                ("‖", None), ("parastyle", None),
            ],
            &[
                ("paste", None), ("selectall", None), ("‖", None), ("bold", None),
                ("italic", None), ("underline", None), ("strikeout", None),
                ("superscript", None), ("subscript", None), ("highlight", None),
                ("fontcolor", None), ("clearstyle", None), ("‖", None),
                ("align-left", None), ("align-center", None),
                ("align-right", None), ("align-just", None),
                ("align-dist", None),
                ("hidenchars", None), ("paracolor", None), ("borders", None),
                ("‖", None), ("replace", None),
            ],
        ];
        // 挿入は一段(発注者の画像 2026-08-04)。主要な釦は名札つきの大釦
        const INS_ROWS: &[&[LItem]] = &[&[
            ("blankpage", Some("空白ページ")), ("pagebreak", Some("区切り")),
            ("‖", None), ("instable", Some("表")), ("‖", None),
            ("insimage", Some("画像")), ("insshape", Some("図形")),
            ("inssmartart", None), ("inschart", None), ("smartpicker", None),
            ("‖", None), ("instext", None), ("instextart", None),
            ("dropcap", None), ("text-from-file", None), ("‖", None),
            ("edit-header", None), ("edit-footer", None), ("pagenum", None),
            ("numpages", None), ("datetime", None), ("‖", None),
            ("insequation", None), ("inssymbol", None), ("‖", None),
            ("controls", None),
        ]];
        // 残りのタブも一段(本家 Web 版の並びから起こした。2026-08-04 発注者)
        const DRAW_ROWS: &[&[LItem]] = &[&[
            ("pen", Some("ペン")), ("highlighter", Some("蛍光ペン")),
            ("eraser", Some("消しゴム")),
        ]];
        const LAYOUT_ROWS: &[&[LItem]] = &[&[
            ("pagemargins", Some("余白")), ("pageorient", Some("向き")),
            ("pagesize", Some("サイズ")), ("columns", Some("段組み")),
            ("‖", None), ("line-numbers", None), ("hyphenation", None),
            ("‖", None), ("watermark", None), ("pagecolor", None),
            ("‖", None), ("colorschemas", None),
        ]];
        const REF_ROWS: &[&[LItem]] = &[&[
            ("toc", Some("目次")), ("toc-update", None), ("add-text", None),
            ("‖", None), ("bookmarks", None), ("caption", None),
            ("crossref", None), ("‖", None), ("tof", None), ("tof-update", None),
        ]];
        const FORM_ROWS: &[&[LItem]] = &[&[
            ("form-text", None), ("form-combo", None), ("form-dropdown", None),
            ("form-checkbox", None), ("form-radio", None), ("form-image", None),
            ("form-email", None), ("form-phone", None), ("form-complex", None),
            ("form-signature", None), ("‖", None), ("form-name", Some("名前")),
        ]];
        const COLLAB_ROWS: &[&[LItem]] = &[&[
            ("coauth-mode", Some("共同編集モード")), ("‖", None),
            ("co-addcomment", Some("コメント")), ("co-delcomment", None),
            ("co-showcomment", None), ("‖", None), ("co-chat", Some("チャット")),
            ("‖", None), ("track-changes", Some("変更履歴")), ("‖", None),
            ("co-history", Some("バージョン履歴")),
        ]];
        const PROT_ROWS: &[&[LItem]] = &[&[
            ("prot-encrypt", Some("暗号化")), ("prot-sign", Some("署名")),
            ("prot-doc", Some("保護")),
        ]];
        // 表示は本家どおり2段(発注者の画像 2026-08-04)
        const VIEW_ROWS: &[&[LItem]] = &[
            &[
                ("nav", Some("ナビゲーション")), ("‖", None),
                ("fit-page", Some("ページに合わせる")),
                ("zoom100", Some("100%に拡大する")), ("zoom-in", None),
                ("‖", None), ("darkmode", None), ("ruler", None),
                ("‖", None), ("show-toolbar", None), ("show-left", None),
            ],
            &[
                ("‖", None),
                ("fit-width", Some("幅に合わせる")),
                ("multipage", Some("複数ページ")), ("zoom-out", None),
                ("‖", None), ("‖", None),
                ("‖", None), ("show-statusbar", None), ("show-right", None),
            ],
        ];
        const PLUG_ROWS: &[&[LItem]] = &[&[
            ("plug-macros", Some("マクロ")),
            ("plug-manage", Some("プラグインの管理")),
        ]];
        let rows: Option<&[&[LItem]]> = match ribbon::WRITER[self.tab].name {
            "ホーム" => Some(HOME_ROWS),
            "挿入" => Some(INS_ROWS),
            "描画" => Some(DRAW_ROWS),
            "レイアウト" => Some(LAYOUT_ROWS),
            "参考資料" => Some(REF_ROWS),
            "フォーム" => Some(FORM_ROWS),
            "共同編集" => Some(COLLAB_ROWS),
            "保護" => Some(PROT_ROWS),
            "表示" => Some(VIEW_ROWS),
            "プラグイン" => Some(PLUG_ROWS),
            _ => None,
        };
        if let Some(rows) = rows {
            let size_now = self.doc.size_at(self.ed.selection()).unwrap_or(SIZE_PT);
            let size_disp = if size_now.fract() == 0.0 {
                format!("{}", size_now as i32)
            } else {
                format!("{size_now}")
            };
            for ids in rows {
                let tall = ids.iter().any(|(_, b)| b.is_some());
                let mut row = div().flex().flex_row().items_center().gap_0p5();
                for &(id, big) in *ids {
                    if id == "‖" {
                        row = row.child(div().w(px(1.0))
                            .h(px(if tall { 40.0 } else { 22.0 }))
                            .bg(th_cmd_border).mx_1());
                        continue;
                    }
                    // コンボ風(フォント名と大きさは今の値を見せる)
                    if id == "fontname" || id == "fontsize" {
                        let cid = id;
                        let text = if cid == "fontname" {
                            self.font_name.to_string()
                        } else {
                            size_disp.clone()
                        };
                        row = row.child(div()
                            .id(SharedString::from(format!("h-{cid}")))
                            .flex().flex_row().items_center().gap_1()
                            .px_2().h(px(26.0))
                            .w(px(if cid == "fontname" { 150.0 } else { 56.0 }))
                            .rounded_sm().border_1().border_color(th_cmd_border)
                            .text_size(px(12.0)).text_color(th_top_fg)
                            .cursor_pointer()
                            .hover(move |st| st.bg(th_btn_hover))
                            .child(div().flex_1().whitespace_nowrap()
                                .overflow_hidden().child(SharedString::from(text)))
                            .child(div().text_size(px(9.0)).text_color(th_tab_idle)
                                .child("▼"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.run_cmd(cid, cx);
                                cx.notify()
                            })));
                        continue;
                    }
                    let Some(cmd) = ribbon::writer_tabs()[self.tab]
                        .cmds
                        .iter()
                        .find(|c| c.id == id || (!c.ready && c.icon == id))
                        .copied()
                    else {
                        continue;
                    };
                    let label = cmd.label;
                    // 名札の短い形は ja 向け — 他の言語では表の語を使う
                    let big = if ui::settings::language() == "ja" {
                        big
                    } else {
                        big.map(|_| cmd.label)
                    };
                    let icon = cmd.icon;
                    let hoverable = cx.listener(move |this: &mut Writer, on: &bool, _, cx| {
                        if *on {
                            this.hover_hint = Some(label);
                        } else if this.hover_hint == Some(label) {
                            this.hover_hint = None;
                        }
                        cx.notify()
                    });
                    let has_icon = ui::icons::find(icon).is_some();
                    if let Some(short) = big {
                        // 名札つきの大釦(絵の下に短い名前。本家の言い方)
                        let on = cmd.ready && self.toggled(cmd.id);
                        let fg = if !cmd.ready {
                            th_gray_fg
                        } else if on {
                            th_btn
                        } else {
                            th_top_fg
                        };
                        let mut b = div()
                            .id(SharedString::from(format!("h-{icon}")))
                            .px_2().h(px(48.0)).rounded_sm()
                            .flex().flex_col().items_center().justify_center()
                            .gap_1()
                            .on_hover(hoverable)
                            .children(has_icon.then(|| {
                                gpui::svg()
                                    .path(SharedString::from(format!("icons/{icon}.svg")))
                                    .size(px(20.0))
                                    .text_color(fg)
                            }))
                            .child(div().text_size(px(10.5)).text_color(fg)
                                .child(short));
                        if on {
                            b = b.bg(th_btn_hover).border_1().border_color(th_btn);
                        }
                        if cmd.ready {
                            let cid = cmd.id;
                            b = b.cursor_pointer()
                                .hover(move |st| st.bg(th_btn_hover))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.run_cmd(cid, cx);
                                    cx.notify()
                                }));
                        }
                        row = row.child(b);
                        continue;
                    }
                    let on = cmd.ready && self.toggled(cmd.id);
                    let mut b = div()
                        .id(SharedString::from(format!("h-{icon}")))
                        .h(px(26.0)).rounded_sm()
                        .flex().items_center().justify_center()
                        .on_hover(hoverable);
                    b = if has_icon { b.w(px(26.0)) } else { b.px_1p5() };
                    if on {
                        // 入っている印(押した結果が画面に残るもの)
                        b = b.bg(th_btn_hover).border_1().border_color(th_btn);
                    }
                    if cmd.ready {
                        let cid = cmd.id;
                        b = b.cursor_pointer()
                            .hover(move |st| st.bg(th_btn_hover))
                            .children(has_icon.then(|| {
                                gpui::svg()
                                    .path(SharedString::from(format!("icons/{icon}.svg")))
                                    .size(px(18.0))
                                    .text_color(if on { th_btn } else { th_top_fg })
                            }))
                            .children((!has_icon).then(|| {
                                div().text_size(px(10.5)).text_color(th_btn)
                                    .child(label)
                            }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.run_cmd(cid, cx);
                                cx.notify()
                            }));
                    } else {
                        // 未実装。押せるように見せない
                        b = b.children(has_icon.then(|| {
                            gpui::svg()
                                .path(SharedString::from(format!("icons/{icon}.svg")))
                                .size(px(18.0))
                                .text_color(th_gray_fg)
                        }))
                        .children((!has_icon).then(|| {
                            div().text_size(px(10.5)).text_color(th_gray_fg)
                                .child(label)
                        }));
                    }
                    row = row.child(b);
                }
                cmds = cmds.child(row);
            }
        } else {
            let mut row = div().flex().flex_row().flex_wrap().gap_1().items_center().py_1();
            for cmd in ribbon::writer_tabs()[self.tab].cmds {
                if cmd.ready {
                    let id = cmd.id;
                    row = row.child(div()
                        .id(SharedString::from(cmd.id))
                        .px_3().py_1().rounded_md()
                        .border_1().border_color(th_btn).text_color(th_btn)
                        .text_size(px(12.0)).cursor_pointer()
                        .hover(move |s| s.bg(th_btn_hover))
                        .flex().flex_row().items_center().gap_1()
                        .children(ui::icons::find(cmd.icon).map(|_| {
                            gpui::svg()
                                .path(SharedString::from(format!("icons/{}.svg", cmd.icon)))
                                .size(px(15.0))
                                .text_color(th_btn)
                        }))
                        .child(cmd.label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.run_cmd(id, cx); cx.notify()
                        })));
                } else {
                    // 未実装。押せるように見せない
                    row = row.child(div().px_3().py_1().rounded_md()
                        .border_1().border_color(th_gray_border)
                        .text_color(th_gray_fg).text_size(px(12.0))
                        .flex().flex_row().items_center().gap_1()
                        .children(ui::icons::find(cmd.icon).map(|_| {
                            gpui::svg()
                                .path(SharedString::from(format!("icons/{}.svg", cmd.icon)))
                                .size(px(15.0))
                                .text_color(th_gray_fg)
                        }))
                        .child(cmd.label));
                }
            }
            cmds = cmds.child(row);
        }
        let bar = if self.tab == 0 || !self.show_toolbar {
            // ファイルのページ(本家の File メニュー)と、畳んだツールバーは
            // 釦の帯を持たない(タブは残る — 押せば中身へ行ける)
            div().flex().flex_col().child(top).child(tabs)
        } else {
            div().flex().flex_col().child(top).child(tabs).child(cmds)
        };

        // ---- 下のステータスバー(デスクトップ版: ページ・文字数・ズーム) ----
        let total_pages = self.page_offsets.len().max(1);
        let cur_page = self
            .page_offsets
            .iter()
            .rposition(|o| self.scroll_mm >= *o - 0.01)
            .unwrap_or(0)
            + 1;
        let nchars = self
            .doc
            .body_text()
            .chars()
            .filter(|c| !c.is_whitespace())
            .count();
        let sb_btn = |id: &'static str, label: &'static str| {
            div().id(id).px_1p5().py_0p5().rounded_sm().cursor_pointer()
                .text_size(px(11.5)).text_color(th_top_fg)
                .hover(move |s| s.bg(th_qa_hover))
                .child(label)
        };
        let statusbar = div().flex().flex_row().items_center().gap_3()
            .px_3().py_0p5().bg(th_top_bg)
            .border_t_1().border_color(th_cmd_border)
            .text_size(px(11.0)).text_color(th_status)
            .child(SharedString::from(ui::tf!("{}/{} ページ", cur_page, total_pages)))
            .child(SharedString::from(ui::tf!("文字数 {}", nchars)))
            .child(div().flex_1().whitespace_nowrap().overflow_hidden()
                .child(SharedString::from(match self.hover_hint {
                    Some(h) => h.to_string(),
                    None => format!(
                        "{}{}",
                        if self.dirty { "● " } else { "" },
                        self.status
                    ),
                })))
            .child(sb_btn("sb-spell", ui::t!("スペル")).on_click(cx.listener(|this, _, _, cx| {
                this.run_cmd("spell", cx);
                cx.notify()
            })))
            .child(sb_btn("sb-zoom-out", "−").on_click(cx.listener(|this, _, _, cx| {
                this.run_cmd("zoom-out", cx);
                cx.notify()
            })))
            .child(div().id("sb-zoom").px_1().rounded_sm().cursor_pointer()
                .text_size(px(11.5)).text_color(th_top_fg)
                .hover(move |s| s.bg(th_qa_hover))
                .child(SharedString::from(ui::tf!("ズーム{}%", (self.zoom * 100.0).round() as i32)))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.zoom = 1.0;
                    cx.notify()
                })))
            .child(sb_btn("sb-zoom-in", ui::t!("＋")).on_click(cx.listener(|this, _, _, cx| {
                this.run_cmd("zoom-in", cx);
                cx.notify()
            })));

        // ---- ファイルのページ(本家の File メニュー。タブ0で全面に出す) ----
        let filepage: Option<gpui::Div> = if self.tab != 0 {
            None
        } else {
            let item_bg = th_qa_hover;
            let mk = |id: &'static str, label: &'static str, ready: bool| {
                let d = div().id(id).px_4().py_1p5().text_size(px(13.0));
                if ready {
                    d.text_color(th_top_fg)
                        .cursor_pointer()
                        .hover(move |s| s.bg(item_bg))
                } else {
                    d.text_color(th_gray_fg)
                }
                .child(label)
            };
            let sb = div().w(px(280.0)).bg(th_top_bg)
                .border_r_1().border_color(th_cmd_border)
                .flex().flex_col().py_2()
                .child(mk("f-back", ui::t!("‹ 戻る"), true).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.tab = this.prev_tab;
                        cx.notify()
                    })))
                .child(div().h(px(10.0)))
                .child(mk("f-new", ui::t!("新規作成"), true).on_click(cx.listener(
                    |this, _, _, cx| {
                        if this.new_doc() {
                            this.tab = this.prev_tab;
                        }
                        cx.notify()
                    })))
                .child(mk("f-tpl", ui::t!("テンプレートから作成"), false))
                .child(mk("f-open", ui::t!("開く"), true).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.tab = this.prev_tab;
                        this.open_dialog(cx);
                        cx.notify()
                    })))
                .child(mk("f-url", ui::t!("URLを開く"), true).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.tab = this.prev_tab;
                        this.url_open = true;
                        this.url_ed = Editor::new("http://127.0.0.1:8765/");
                        this.status =
                            ui::t!("URL を打って Enter(JS なしの閲覧と記入。http のみ)").into();
                        cx.notify()
                    })))
                .child(mk("f-recent", ui::t!("最近開いた"), true).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.file_view = 1;
                        cx.notify()
                    })))
                .child(div().h(px(10.0)))
                .child(mk("f-save", ui::t!("保存"), true).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.save(false, cx);
                        cx.notify()
                    })))
                .child(mk("f-saveas", ui::t!("名前を付けて保存"), true).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.save_as(cx);
                        cx.notify()
                    })))
                .child(mk("f-print", ui::t!("印刷"), true).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.save_pdf(cx);
                        cx.notify()
                    })))
                .child(mk("f-protect", ui::t!("保護する"), true).on_click(cx.listener(
                    |this, _, _, cx| {
                        if let Some(i) =
                            ribbon::WRITER.iter().position(|t| t.name == "保護")
                        {
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
                .child(mk("f-quit", ui::t!("終了"), true).on_click(cx.listener(
                    |this, _, _, cx| {
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

            let mut pane = div().flex_1().bg(th_cmd_bg).p_8()
                .flex().flex_col().gap_3().text_size(px(12.5))
                .text_color(th_top_fg);
            if self.file_view == 2 {
                // 詳細設定 — 器は ~/.config/office/settings.toml
                // (SEKKEI「設定 — 器と言語」。環境変数が一時上書きで優先)
                let lang_now = ui::settings::get("language").unwrap_or_else(|| "ja".into());
                let row = |label: &'static str, value: String| {
                    div().flex().flex_row().items_center().gap_2()
                        .child(div().w(px(200.0)).text_color(th_status).child(label))
                        .child(div().child(SharedString::from(value)))
                };
                pane = pane
                    .child(div().text_size(px(16.0))
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(ui::t!("詳細設定")))
                    .child(div().text_color(th_status).child(SharedString::from(
                        ui::tf!("置き場: {}", ui::settings::path().display()))))
                    .child(div().h(px(6.0)))
                    .child(div().flex().flex_row().items_center().gap_2()
                        .child(div().w(px(200.0)).text_color(th_status)
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
                pane = pane.child(div().text_size(px(16.0))
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(ui::t!("最近開いた")));
                let list = Self::recent_list();
                if list.is_empty() {
                    pane = pane.child(div().text_color(th_status)
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
                        .child(div().text_size(px(13.0))
                            .child(SharedString::from(name)))
                        .child(div().text_size(px(11.0)).text_color(th_status)
                            .child(SharedString::from(dir)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.tab = this.prev_tab;
                            this.open(q.clone());
                            cx.notify()
                        })));
                }
            } else {
                let text = self.doc.body_text();
                let words = text.split_whitespace().count();
                let chars_all = text.chars().filter(|c| *c != '\n').count();
                let paras = self.doc.paragraphs().count();
                pane = pane.child(div().text_size(px(16.0))
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(ui::t!("文書の情報")))
                    .child(div().text_size(px(13.5))
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(ui::t!("統計")));
                for (k, v) in [
                    ("ページ", total_pages),
                    ("段落", paras),
                    ("単語", words),
                    ("文字数", nchars),
                    ("文字数 (スペースを含む)", chars_all),
                ] {
                    pane = pane.child(div().flex().flex_row()
                        .child(div().w(px(220.0)).text_color(th_status).child(k))
                        .child(SharedString::from(format!("{v}"))));
                }
                pane = pane.child(div().h(px(6.0)))
                    .child(div().text_size(px(13.5))
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(ui::t!("プロパティ")));
                let pr = self.doc.props.clone();
                let vals: [(&'static str, String, &'static str); 5] = [
                    ("作成者", pr.creator, "著者を追加"),
                    ("タイトル", pr.title, "テキストの追加"),
                    ("タグ", pr.keywords, "テキストの追加"),
                    ("件名", pr.subject, "テキストの追加"),
                    ("コメント", pr.description, "テキストの追加"),
                ];
                for (i, (k, v, ph)) in vals.into_iter().enumerate() {
                    let editing = self.file_field == Some(i as u8);
                    let shown = if editing {
                        let mut t = self.prop_ed.text().to_string();
                        let cur = self.prop_ed.cursor().min(t.len());
                        t.insert(cur, '|');
                        t
                    } else {
                        v.clone()
                    };
                    let empty = !editing && v.is_empty();
                    pane = pane.child(div().flex().flex_row().items_center()
                        .child(div().w(px(220.0)).text_color(th_status).child(k))
                        .child(div()
                            .id(SharedString::from(format!("prop-{i}")))
                            .w(px(320.0)).px_2().py_1().rounded_sm()
                            .border_1()
                            .border_color(if editing {
                                rgb(0x1B6E3C)
                            } else {
                                th_cmd_border
                            })
                            .cursor_pointer()
                            .whitespace_nowrap().overflow_hidden()
                            .text_color(if empty { th_gray_fg } else { th_top_fg })
                            .child(SharedString::from(if empty {
                                ph.to_string()
                            } else {
                                shown
                            }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let cur = match i {
                                    0 => this.doc.props.creator.clone(),
                                    1 => this.doc.props.title.clone(),
                                    2 => this.doc.props.keywords.clone(),
                                    3 => this.doc.props.subject.clone(),
                                    _ => this.doc.props.description.clone(),
                                };
                                this.prop_ed = Editor::new(&cur);
                                this.file_field = Some(i as u8);
                                cx.notify()
                            }))));
                }
                pane = pane.child(div().text_size(px(11.5)).text_color(th_status)
                    .child(ui::t!("欄を押して打ち、Enter で控える(保存で docx の情報に入ります)")));
            }
            Some(div().flex_1().relative().overflow_hidden()
                .child(div().absolute().inset_0().flex().flex_row()
                    .child(sb)
                    .child(pane))
                .child(InputSink { view: me.clone() }))
        };

        // 紙。スクロールは紙ごと上へずらすだけ(中身は全部この容器の子)。
        // ページの色は文書の設定(紙も同じ色に塗られる)
        let paper_bg = match self.doc.page_color.as_deref() {
            Some(c) => gpui::Rgba { r: hex(c, 0), g: hex(c, 1), b: hex(c, 2), a: 1.0 },
            None => gpui::Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
        };
        let mut paper = div().absolute()
            .left(px(28.0)).top(px(14.0 - self.scroll_mm * pxmm))
            .w(px(self.paper_w_mm() * pxmm)).h(px(self.content_mm() * pxmm))
            .bg(paper_bg).shadow_lg();

        // ルーラー(10mm ごとの目盛り。余白の位置が分かる)
        if self.ruler {
            let mut n = 0;
            loop {
                let mm = n as f32 * 10.0;
                if mm > self.pg.w_mm {
                    break;
                }
                let major = n % 5 == 0;
                paper = paper.child(div().absolute()
                    .left(px(mm * pxmm)).top(px(0.0))
                    .w(px(1.0)).h(px(if major { 10.0 } else { 5.0 }))
                    .bg(rgb(0xAABBC6)));
                if major && n > 0 {
                    paper = paper.child(div().absolute()
                        .left(px(mm * pxmm + 2.0)).top(px(0.0))
                        .text_size(px(8.5)).text_color(rgb(0x8899A6))
                        .child(SharedString::from(format!("{}", mm as u32))));
                }
                n += 1;
            }
            // 余白の線(本文の左右端)
            for x in [self.pg.left_mm, self.pg.w_mm - self.pg.right_mm] {
                paper = paper.child(div().absolute()
                    .left(px(x * pxmm)).top(px(0.0))
                    .w(px(1.0)).h(px(14.0)).bg(rgb(0x1B6E3C)));
            }
        }

        // 画像。組版が置いた位置に、そのまま出す
        for (i, (bytes, [x, top, w_mm, h_mm])) in self.page.images.iter().enumerate() {
            let src = self.image_cache.entry(std::sync::Arc::as_ptr(bytes) as usize)
                .or_insert_with(|| {
                    let format = match bytes.get(..4) {
                        Some([0x89, b'P', b'N', b'G']) => gpui::ImageFormat::Png,
                        Some([0xFF, 0xD8, ..]) => gpui::ImageFormat::Jpeg,
                        _ => gpui::ImageFormat::Png,
                    };
                    std::sync::Arc::new(gpui::Image::from_bytes(format, bytes.to_vec()))
                })
                .clone();
            let _ = i;
            paper = paper.child(
                gpui::img(src)
                    .absolute()
                    .left(px((self.pg.left_mm + x) * pxmm))
                    .top(px(top * pxmm))
                    .w(px(w_mm * pxmm))
                    .h(px(h_mm * pxmm)),
            );
        }

        // 表の罫線。紙面の座標をそのまま引く
        for r in &self.page.rules {
            let [x1, y1, x2, y2] = *r;
            let (x1, y1) = ((self.pg.left_mm + x1) * pxmm, y1 * pxmm);
            let (x2, y2) = ((self.pg.left_mm + x2) * pxmm, y2 * pxmm);
            paper = paper.child(div().absolute()
                .left(px(x1.min(x2))).top(px(y1.min(y2)))
                .w(px((x2 - x1).abs().max(1.0))).h(px((y2 - y1).abs().max(1.0)))
                .bg(rgb(0x444B52)));
        }

        // 段落の背景色と囲み枠。行の帯として敷く(文字より下に来るよう先に描く)
        {
            let mut deco: Vec<(std::ops::Range<usize>, Option<String>, bool)> = Vec::new();
            let mut at = 0usize;
            for p in self.doc.paragraphs() {
                let len: usize = p.runs.iter().map(|r| r.text.len()).sum();
                if p.shade.is_some() || p.boxed {
                    deco.push((at..at + len, p.shade.clone(), p.boxed));
                }
                at += len + 1;
            }
            if !deco.is_empty() {
                let (bx0, bx1) = (self.pg.left_mm, self.pg.w_mm - self.pg.right_mm);
                for line in self.page.lines.iter().filter(|l| l.from_body) {
                    let Some((r, shade, boxed)) = deco
                        .iter()
                        .find(|(r, ..)| r.start <= line.byte0 && line.byte0 <= r.end)
                        .map(|(r, sh, b)| (r.clone(), sh.clone(), *b))
                    else {
                        continue;
                    };
                    let band_top = (line.y_mm - LINE_MM * 0.75) * pxmm;
                    let band_h = LINE_MM * pxmm;
                    if let Some(c) = &shade {
                        paper = paper.child(div().absolute()
                            .left(px(bx0 * pxmm)).top(px(band_top))
                            .w(px((bx1 - bx0) * pxmm)).h(px(band_h))
                            .bg(gpui::Rgba {
                                r: hex(c, 0), g: hex(c, 1), b: hex(c, 2), a: 1.0,
                            }));
                    }
                    if boxed {
                        let ink = rgb(0x444B52);
                        for x in [bx0, bx1] {
                            paper = paper.child(div().absolute()
                                .left(px(x * pxmm)).top(px(band_top))
                                .w(px(1.0)).h(px(band_h)).bg(ink));
                        }
                        if line.byte0 == r.start {
                            paper = paper.child(div().absolute()
                                .left(px(bx0 * pxmm)).top(px(band_top))
                                .w(px((bx1 - bx0) * pxmm)).h(px(1.0)).bg(ink));
                        }
                        if line.byte_end() >= r.end {
                            paper = paper.child(div().absolute()
                                .left(px(bx0 * pxmm)).top(px(band_top + band_h))
                                .w(px((bx1 - bx0) * pxmm)).h(px(1.0)).bg(ink));
                        }
                    }
                }
            }
        }

        // ページの境の薄い線(積み上げたページの切れ目が分かるように)
        {
            let mut pno = 1;
            loop {
                let y = pno as f32 * self.pg.h_mm;
                if y >= self.content_mm() {
                    break;
                }
                paper = paper.child(div().absolute()
                    .left(px(0.0)).top(px(y * pxmm))
                    .w(px(self.pg.w_mm * pxmm)).h(px(1.0))
                    .bg(rgb(0xD5DBE0)));
                pno += 1;
            }
        }

        // 透かし。1字ずつ対角線に沿って置く(画面の近似。紙は回転した字)
        if let Some(text) = self.doc.watermark.as_deref().filter(|t| !t.is_empty()) {
            let n = text.chars().count().max(1) as f32;
            let wpt = (520.0 / n).clamp(36.0, 120.0);
            let em_mm = wpt * 25.4 / 72.0;
            let k = std::f32::consts::FRAC_1_SQRT_2;
            let (cx0, cy0) = (self.pg.w_mm / 2.0, self.pg.h_mm / 2.0);
            for (i, ch) in text.chars().enumerate() {
                let t = (i as f32 - (n - 1.0) / 2.0) * em_mm;
                let x = cx0 + t * k - em_mm / 2.0;
                let y = cy0 - t * k - em_mm / 2.0;
                paper = paper.child(div().absolute()
                    .left(px(x * pxmm)).top(px(y * pxmm))
                    .text_size(px(wpt * 96.0 / 72.0 * self.zoom))
                    .font_family(self.font_name.clone())
                    .text_color(gpui::Rgba { r: 0.62, g: 0.62, b: 0.62, a: 0.5 })
                    .child(SharedString::from(ch.to_string())));
            }
        }

        // 変更履歴の記録中: 変わった段落の左に橙の棒(Word の変更バー)
        if self.track {
            if let Some(base) = &self.track_base {
                let base_set: std::collections::HashSet<&str> =
                    base.iter().map(|s| s.as_str()).collect();
                let mut starts: Vec<(usize, bool)> = Vec::new();
                let mut at = 0usize;
                for p in self.doc.paragraphs() {
                    let t = para_text(p);
                    starts.push((at, !base_set.contains(t.as_str())));
                    at += t.len() + 1;
                }
                for line in self.page.lines.iter().filter(|l| l.from_body) {
                    let changed = starts
                        .iter()
                        .rev()
                        .find(|(b, _)| *b <= line.byte0)
                        .map(|(_, c)| *c)
                        .unwrap_or(false);
                    if changed {
                        paper = paper.child(div().absolute()
                            .left(px((self.pg.left_mm - 5.0).max(0.5) * pxmm))
                            .top(px((line.y_mm - LINE_MM * 0.7) * pxmm))
                            .w(px(2.0)).h(px(LINE_MM * pxmm))
                            .bg(rgb(0xE08A00)));
                    }
                }
            }
        }

        // コメントの印。付いた段落の1行目の右余白にオレンジの角を出す
        if self.show_comments {
            let mut at = 0usize;
            let mut heads: Vec<usize> = Vec::new(); // コメント付き段落の頭のバイト
            for p in self.doc.paragraphs() {
                let len: usize = p.runs.iter().map(|r| r.text.len()).sum();
                if !p.comments.is_empty() {
                    heads.push(at);
                }
                at += len + 1;
            }
            for s0 in heads {
                if let Some(line) = self.page.lines.iter()
                    .filter(|l| l.from_body)
                    .find(|l| l.byte0 == s0)
                {
                    paper = paper.child(div().absolute()
                        .left(px((self.pg.w_mm - self.pg.right_mm + 2.0) * pxmm))
                        .top(px(line.y_mm * pxmm - 8.0))
                        .w(px(6.0)).h(px(6.0)).rounded_sm()
                        .bg(rgb(0xE08A00)));
                }
            }
        }

        // 行番号。本文の(見た目の)行を数え、左の余白に出す
        if self.line_numbers {
            let mut n = 0usize;
            for line in self.page.lines.iter().filter(|l| l.from_body) {
                n += 1;
                paper = paper.child(div().absolute()
                    .left(px((self.pg.left_mm - 9.0).max(1.0) * pxmm))
                    .top(px(line.y_mm * pxmm - 8.5 * self.zoom))
                    .text_size(px(8.5 * self.zoom))
                    .text_color(rgb(0x9DB8C8))
                    .child(SharedString::from(n.to_string())));
            }
        }

        // 未確定(変換中)の下線は、行が持つバイト位置(byte0)で結ぶ
        for (li, line) in self.page.lines.iter().enumerate() {
            if line.cells.is_empty() {
                continue;
            }
            if self.page.vertical {
                // 縦書き: 列の x に1字ずつ正立で置く。選択は縦の帯、
                // キャレットは横棒(変換下線は初版では出さない)
                let colx = self.page.vert_x.get(li).copied().unwrap_or(0.0);
                let mine = match self.target {
                    Target::Body => line.from_body,
                    Target::Cell { table, row, col } => {
                        line.cell == Some((table, row, col))
                    }
                };
                let (ls, le) = (line.byte0, line.byte_end());
                let base = line.cells.iter().map(|c| c.off).min().unwrap_or(0);
                let yr = |upto: usize| -> f32 {
                    line.cells.iter()
                        .find(|c| c.off - base >= upto)
                        .map(|c| c.x_mm)
                        .or_else(|| line.cells.last().map(|c| c.x_mm + c.w_mm))
                        .unwrap_or(0.0)
                };
                let selr = self.ed.selection();
                if mine && !selr.is_empty() && selr.start < le && selr.end > ls {
                    let a = selr.start.max(ls) - ls;
                    let b = selr.end.min(le) - ls;
                    paper = paper.child(div().absolute()
                        .left(px(colx * pxmm))
                        .top(px((line.y_mm + yr(a)) * pxmm))
                        .w(px(LINE_MM * 0.9 * pxmm))
                        .h(px((yr(b) - yr(a)).max(1.5) * pxmm))
                        .bg(gpui::Rgba { r: 0.40, g: 0.60, b: 0.85, a: 0.35 }));
                }
                if mine {
                    let cur = self.ed.cursor();
                    if cur >= ls && cur <= le {
                        paper = paper.child(div().absolute()
                            .left(px(colx * pxmm))
                            .top(px((line.y_mm + yr(cur - ls)) * pxmm))
                            .w(px(LINE_MM * 0.9 * pxmm))
                            .h(px(1.5))
                            .bg(rgb(0x165E83)));
                    }
                }
                for c in &line.cells {
                    let spt = c.size_pt * 96.0 / 72.0 * self.zoom;
                    let mut d = div().absolute()
                        .left(px(colx * pxmm))
                        .top(px((line.y_mm + c.x_mm) * pxmm))
                        .text_size(px(spt))
                        .font_family(c.font.clone().map(SharedString::from)
                            .unwrap_or_else(|| self.font_name.clone()))
                        .whitespace_nowrap()
                        .child(SharedString::from(c.ch.to_string()));
                    if c.fmt.bold {
                        d = d.font_weight(gpui::FontWeight::BOLD);
                    }
                    d = match &c.fmt.color {
                        Some(cl) => d.text_color(gpui::Rgba {
                            r: hex(cl, 0), g: hex(cl, 1), b: hex(cl, 2), a: 1.0,
                        }),
                        None => d.text_color(rgb(0x1B1B1B)),
                    };
                    paper = paper.child(d);
                }
                continue;
            }
            let pt = line.cells[0].size_pt;
            let sz = pt * 96.0 / 72.0 * self.zoom;
            let x0 = self.pg.left_mm + line.cells[0].x_mm;
            let top = line.y_mm * pxmm - sz * 0.88;

            if let Some(m) = &marked {
                let mine = match self.target {
                    Target::Body => line.from_body,
                    Target::Cell { table, row, col } => line.cell == Some((table, row, col)),
                };
                if !mine {
                    // 編集していない行に変換下線は出さない
                } else {
                let (ls, le) = (line.byte0, line.byte_end());
                if m.start < le && m.end > ls {
                    let a = m.start.max(ls) - ls;
                    let b = m.end.min(le) - ls;
                    let base = line.cells.iter().map(|c| c.off).min().unwrap_or(0);
                    // 幅は x 位置から出す(均等割付で字間が広がってもずれない)
                    let xr = |upto: usize| -> f32 {
                        line.cells.iter()
                            .find(|c| c.off - base >= upto)
                            .map(|c| c.x_mm)
                            .or_else(|| line.cells.last().map(|c| c.x_mm + c.w_mm))
                            .unwrap_or(0.0)
                            - line.cells[0].x_mm
                    };
                    paper = paper.child(div().absolute()
                        .left(px((x0 + xr(a)) * pxmm))
                        .top(px(top + sz * (1.05 + HALF_LEADING)))
                        .w(px((xr(b) - xr(a)).max(1.0) * pxmm))
                        .h(px(2.0)).bg(rgb(0x165E83)));
                }
                }
            }
            // 選択の色。**選択が見えないと、コピーも切り取りも信用できない**
            // (ドラッグで選べるようにしても、色が出なければ「できない」に見える)
            let selr = self.ed.selection();
            if !selr.is_empty() {
                let mine = match self.target {
                    Target::Body => line.from_body,
                    Target::Cell { table, row, col } => line.cell == Some((table, row, col)),
                };
                let (ls, le) = (line.byte0, line.byte_end());
                if mine && selr.start < le && selr.end > ls {
                    let a = selr.start.max(ls) - ls;
                    let b = selr.end.min(le) - ls;
                    let base = line.cells.iter().map(|c| c.off).min().unwrap_or(0);
                    let xr = |upto: usize| -> f32 {
                        line.cells.iter()
                            .find(|c| c.off - base >= upto)
                            .map(|c| c.x_mm)
                            .or_else(|| line.cells.last().map(|c| c.x_mm + c.w_mm))
                            .unwrap_or(0.0)
                            - line.cells[0].x_mm
                    };
                    paper = paper.child(div().absolute()
                        .left(px((x0 + xr(a)) * pxmm))
                        .top(px(top + sz * HALF_LEADING))
                        .w(px((xr(b) - xr(a)).max(1.5) * pxmm))
                        .h(px(sz * 1.2))
                        // 半透明の青。文字より下・蛍光ペンより上に敷く
                        .bg(gpui::Rgba { r: 0.40, g: 0.60, b: 0.85, a: 0.35 }));
                }
            }
            // 文字は**同じ書式の連なり**ごとに描く(部分書式。太字・大きさ・
            // 書体・色が行の中で混ざっても、その通りに出る)
            let mut i = 0usize;
            while i < line.cells.len() {
                let c0 = &line.cells[i];
                let mut j = i + 1;
                while j < line.cells.len()
                    && line.cells[j].fmt == c0.fmt
                    && line.cells[j].size_pt == c0.size_pt
                    && line.cells[j].font == c0.font
                    // 字間が広げられた行(均等割付)は1本で描けない —
                    // x が飛んだら連なりを切る
                    && (line.cells[j].x_mm
                        - line.cells[j - 1].x_mm
                        - line.cells[j - 1].w_mm)
                        .abs()
                        < 0.05
                {
                    j += 1;
                }
                let seg = &line.cells[i..j];
                let text: String = seg.iter().map(|c| c.ch).collect();
                let w_mm: f32 = seg.iter().map(|c| c.w_mm).sum();
                let f = &c0.fmt;
                let sx = self.pg.left_mm + c0.x_mm;
                let spt = c0.size_pt * 96.0 / 72.0 * self.zoom;
                let stop = line.y_mm * pxmm - spt * 0.88;
                // 上付き・下付きは小さく描き、少し上下へずらす
                let (spt, stop) = if f.superscript {
                    (spt * 0.7, stop - spt * 0.25)
                } else if f.subscript {
                    (spt * 0.7, stop + spt * 0.25)
                } else {
                    (spt, stop)
                };
                // 記入欄(コンテンツコントロール)は薄い箱で囲む。
                // 「ここは書き込む場所」と分かるように(Word の作法)
                if f.sdt.is_some() {
                    paper = paper.child(div().absolute()
                        .left(px(sx * pxmm)).top(px(stop + spt * HALF_LEADING))
                        .w(px(w_mm * pxmm)).h(px(spt * 1.15))
                        .border_1().border_color(rgb(0x8FB8CE))
                        .bg(gpui::Rgba { r: 0.55, g: 0.75, b: 0.9, a: 0.10 }));
                }
                // 参照(フィールド)はうっすら網掛け(Word の作法)。
                // 「ここは計算された値」と分かるように
                if f.field.is_some() {
                    paper = paper.child(div().absolute()
                        .left(px(sx * pxmm)).top(px(stop + spt * HALF_LEADING))
                        .w(px(w_mm * pxmm)).h(px(spt * 1.15))
                        .bg(gpui::Rgba { r: 0.55, g: 0.6, b: 0.65, a: 0.16 }));
                }
                // 蛍光ペン。字の下に色を敷く
                if let Some(h) = &f.highlight {
                    let bg = match h.as_str() {
                        "green" => rgb(0xC9F0C9),
                        "cyan" => rgb(0xC9EEF0),
                        _ => rgb(0xF7EFA8),
                    };
                    paper = paper.child(div().absolute()
                        .left(px(sx * pxmm)).top(px(stop + spt * HALF_LEADING))
                        .w(px(w_mm * pxmm)).h(px(spt * 1.15))
                        .bg(bg));
                }
                let mut d = div().absolute()
                    .left(px(sx * pxmm)).top(px(stop))
                    .text_size(px(spt))
                    .font_family(c0.font.clone().map(SharedString::from)
                        .unwrap_or_else(|| self.font_name.clone()))
                    .whitespace_nowrap()
                    .child(SharedString::from(text));
                if f.bold {
                    d = d.font_weight(gpui::FontWeight::BOLD);
                }
                if f.italic {
                    d = d.italic();
                }
                d = match &f.color {
                    Some(c) => d.text_color(gpui::Rgba {
                        r: hex(c, 0), g: hex(c, 1), b: hex(c, 2), a: 1.0,
                    }),
                    None => d.text_color(rgb(0x1B1B1B)),
                };
                paper = paper.child(d);
                // 下線・取り消し線は連なりごとに引く(gpui の text に無い)
                for (on, dy) in [
                    (f.underline, spt * (1.05 + HALF_LEADING)),
                    (f.strike, spt * (0.35 + HALF_LEADING)),
                ] {
                    if on {
                        paper = paper.child(div().absolute()
                            .left(px(sx * pxmm)).top(px(stop + dy))
                            .w(px(w_mm * pxmm)).h(px(1.0))
                            .bg(rgb(0x1B1B1B)));
                    }
                }
                i = j;
            }
            // 編集記号。空白は・、段落の終わりは ↵(見え方だけ。文書は変わらない)
            if self.show_marks && line.from_body {
                for c in &line.cells {
                    if c.ch == ' ' || c.ch == '\u{3000}' {
                        paper = paper.child(div().absolute()
                            .left(px((self.pg.left_mm + c.x_mm + c.w_mm * 0.3) * pxmm))
                            .top(px(top + sz * 0.35))
                            .text_size(px(sz * 0.6)).text_color(rgb(0x9DB8C8))
                            .child(SharedString::from(if c.ch == ' ' { "·" } else { "□" })));
                    }
                }
                let end_x = line.cells.last().map(|c| c.x_mm + c.w_mm).unwrap_or(0.0);
                paper = paper.child(div().absolute()
                    .left(px((self.pg.left_mm + end_x) * pxmm)).top(px(top))
                    .text_size(px(sz * 0.8)).text_color(rgb(0x9DB8C8))
                    .child("↵"));
            }
        }
        // ヘッダー・フッター。画面の紙は巻物なので、ヘッダーは紙の頭、
        // フッターは紙の末尾の頁の位置に出す(番号は1ページ目のもの。
        // 各ページの本当の番号は PDF で入る)。編集中は青、普段は灰色
        let foot_shift = (self.content_mm() - self.pg.h_mm).max(0.0);
        for (lines, dy, active) in [
            (&self.header_lines, 0.0, self.hf_edit == Some(false)),
            (&self.footer_lines, foot_shift, self.hf_edit == Some(true)),
        ] {
            for line in lines.iter() {
                if line.cells.is_empty() {
                    continue;
                }
                let pt = line.cells[0].size_pt;
                let sz = pt * 96.0 / 72.0 * self.zoom;
                let x0 = self.pg.left_mm + line.cells[0].x_mm;
                let top = (line.y_mm + dy) * pxmm - sz * 0.88;
                paper = paper.child(div().absolute()
                    .left(px(x0 * pxmm)).top(px(top))
                    .text_size(px(sz))
                    .font_family(self.font_name.clone())
                    .whitespace_nowrap()
                    .text_color(if active { rgb(0x165E83) } else { rgb(0x8899A6) })
                    .child(SharedString::from(line.text())));
            }
        }
        // キャレット。その場の文字の大きさに合わせて描く(縦書きは行の側)
        if !self.page.vertical {
            let sz = caret_pt * 96.0 / 72.0 * self.zoom;
            paper = paper.child(div().absolute()
                .left(px(cx_mm * pxmm))
                .top(px(cy_mm * pxmm - sz * 0.88))
                .w(px(1.5)).h(px(sz * 1.15))
                .bg(rgb(0x165E83)));
        }

        // 手描きの線。gpui の Path は「塗り」なので、折れ線を
        // 幅のある四角形の連なりとして塗る(画面も紙も同じ座標)
        {
            let mut strokes: Vec<(bool, Vec<(f32, f32)>)> = Vec::new();
            for st in self.doc.ink.iter().chain(self.ink_cur.iter()) {
                let oy = self
                    .page_offsets
                    .get(st.page)
                    .copied()
                    .unwrap_or(st.page as f32 * self.pg.h_mm);
                strokes.push((
                    st.highlighter,
                    st.points.iter().map(|(x, y)| (x * pxmm, (y + oy) * pxmm)).collect(),
                ));
            }
            if !strokes.is_empty() {
                let pxmm2 = pxmm;
                paper = paper.child(
                    gpui::canvas(|_, _, _| (), move |bounds, _, window, _| {
                        for (hl, pts) in &strokes {
                            let w_px = if *hl { 3.0 } else { 0.45 } * pxmm2;
                            let color = if *hl {
                                gpui::Rgba { r: 1.0, g: 0.9, b: 0.35, a: 0.35 }
                            } else {
                                gpui::Rgba { r: 0.11, g: 0.23, b: 0.32, a: 1.0 }
                            };
                            let o = bounds.origin;
                            let mut path: Option<gpui::Path<gpui::Pixels>> = None;
                            for seg in pts.windows(2) {
                                let (x1, y1) = seg[0];
                                let (x2, y2) = seg[1];
                                let (dx, dy) = (x2 - x1, y2 - y1);
                                let len = (dx * dx + dy * dy).sqrt().max(0.01);
                                let (nx, ny) = (-dy / len * w_px / 2.0, dx / len * w_px / 2.0);
                                let a = gpui::point(o.x + px(x1 + nx), o.y + px(y1 + ny));
                                let b = gpui::point(o.x + px(x2 + nx), o.y + px(y2 + ny));
                                let c = gpui::point(o.x + px(x2 - nx), o.y + px(y2 - ny));
                                let d = gpui::point(o.x + px(x1 - nx), o.y + px(y1 - ny));
                                let p = path.get_or_insert_with(|| gpui::Path::new(a));
                                p.move_to(a);
                                p.line_to(b);
                                p.line_to(c);
                                p.line_to(d);
                            }
                            if let Some(p) = path {
                                window.paint_path(p, color);
                            }
                        }
                    })
                    .absolute()
                    .left(px(0.0))
                    .top(px(0.0))
                    .size_full(),
                );
            }
        }

        // 置換の板
        let find_panel = if !self.find_open {
            None
        } else {
            let field = |label: &str, ed: &Editor, active: bool| {
                // caret は | で見せる(専用の入力部品を作らない割り切り)
                let mut s = ed.text().to_string();
                let cur = ed.cursor().min(s.len());
                if active {
                    s.insert(cur, '|');
                }
                div().flex().flex_row().items_center().gap_2()
                    .child(div().w(px(64.0)).text_size(px(11.5))
                        .text_color(rgb(0x66707A)).child(SharedString::from(label.to_string())))
                    .child(div().flex_1().px_2().py_1().rounded_sm()
                        .border_1()
                        .border_color(if active { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                        .bg(gpui::white())
                        .text_size(px(12.5))
                        .whitespace_nowrap().overflow_hidden()
                        .child(SharedString::from(s)))
            };
            let btn = |id: &str, label: &str| {
                div().id(SharedString::from(id.to_string()))
                    .px_2p5().py_1().rounded_sm()
                    .border_1().border_color(rgb(0x1B6E3C)).text_color(rgb(0x1B6E3C))
                    .text_size(px(11.5)).cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF5EE)))
                    .child(SharedString::from(label.to_string()))
            };
            Some(div().absolute().left(px(16.0)).top(px(8.0)).w(px(430.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_2()
                .child(field("検索", &self.find_ed, self.find_field == 0)
                    .id("find-f").cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| { this.find_field = 0; cx.notify() })))
                .child(field("置換後", &self.repl_ed, self.find_field == 1)
                    .id("find-r").cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| { this.find_field = 1; cx.notify() })))
                .child(div().flex().flex_row().gap_2()
                    .child(btn("f-next", ui::t!("次へ (Enter)"))
                        .on_click(cx.listener(|this, _, _, cx| { this.find_next(); cx.notify() })))
                    .child(btn("f-one", ui::t!("置換"))
                        .on_click(cx.listener(|this, _, _, cx| { this.replace_current(); cx.notify() })))
                    .child(btn("f-all", ui::t!("すべて置換"))
                        .on_click(cx.listener(|this, _, _, cx| { this.replace_all(); cx.notify() })))
                    .child(div().flex_1())
                    .child(btn("f-close", ui::t!("閉じる"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.find_open = false; cx.notify()
                        })))))
        };

        // ヘッダー・フッターの編集の板。開いている間、打鍵はここに入る
        let hf_panel = self.hf_edit.map(|footer| {
            let title = if footer { "フッター" } else { "ヘッダー" };
            // キャレットは | で見せる(検索の板と同じ割り切り)。
            // ページ番号の印は読める形で見せる
            let mut s = self.hf_ed.text().to_string();
            let cur = self.hf_ed.cursor().min(s.len());
            s.insert(cur, '|');
            let shown = s
                .replace(kumihan::PAGE_MARK, "《ページ番号》")
                .replace(kumihan::PAGES_MARK, "《ページ数》");
            let btn = |id: &str, label: &str| {
                div().id(SharedString::from(id.to_string()))
                    .px_2p5().py_1().rounded_sm()
                    .border_1().border_color(rgb(0x1B6E3C)).text_color(rgb(0x1B6E3C))
                    .text_size(px(11.5)).cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF5EE)))
                    .child(SharedString::from(label.to_string()))
            };
            let mut field = div().flex_1().px_2().py_1().rounded_sm()
                .border_1().border_color(rgb(0x1B6E3C)).bg(gpui::white())
                .text_size(px(12.5)).flex().flex_col();
            for ln in shown.split('\n') {
                field = field.child(div().whitespace_nowrap().overflow_hidden()
                    .child(SharedString::from(ln.to_string())));
            }
            div().absolute().left(px(16.0)).top(px(8.0)).w(px(430.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_2()
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x165E83))
                    .child(SharedString::from(ui::tf!("{}の編集 — 全ページ共通", title))))
                .child(field)
                .child(div().flex().flex_row().gap_2()
                    .child(btn("hf-num", ui::t!("ページ番号を挿入"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.run_cmd("pagenum", cx);
                            cx.notify()
                        })))
                    .child(div().flex_1())
                    .child(btn("hf-close", ui::t!("閉じる (Esc)"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.hf_edit = None;
                            this.status = "".into();
                            cx.notify()
                        }))))
        });

        // コメントの板と、カーソルの段落のコメントの一覧
        let cmt_panel = if !self.cmt_edit {
            // 板が閉じていても、カーソルの段落にコメントがあれば見せる
            let cur = self.ed.cursor();
            let mut at = 0usize;
            let mut found: Option<Vec<(String, String)>> = None;
            if self.show_comments && self.target == Target::Body {
                for p in self.doc.paragraphs() {
                    let len: usize = p.runs.iter().map(|r| r.text.len()).sum();
                    if at <= cur && cur <= at + len && !p.comments.is_empty() {
                        found = Some(p.comments.iter()
                            .map(|c| (c.author.clone(), c.text.clone()))
                            .collect());
                        break;
                    }
                    at += len + 1;
                }
            }
            found.map(|cs| {
                let mut d = div().absolute().left(px(16.0)).bottom(px(16.0)).w(px(300.0))
                    .p_3().rounded_md().bg(rgb(0xFFF6E6))
                    .border_1().border_color(rgb(0xE8D5A8))
                    .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(0x8A4B00))
                        .child(ui::t!("この段落のコメント(レビュー > コメント で編集)")));
                for (author, text) in cs {
                    d = d.child(div().mt_1p5().text_size(px(11.5)).text_color(rgb(0x5A4A28))
                        .child(SharedString::from(format!("{author}: {text}"))));
                }
                d
            })
        } else {
            // 編集の板(検索の板と同じ作法。| がキャレット)
            let mut t = self.cmt_ed.text().to_string();
            let cur = self.cmt_ed.cursor().min(t.len());
            t.insert(cur, '|');
            let mut field = div().flex_1().px_2().py_1().rounded_sm()
                .border_1().border_color(rgb(0xE08A00)).bg(gpui::white())
                .text_size(px(12.5)).flex().flex_col();
            for ln in t.split('\n') {
                field = field.child(div().whitespace_nowrap().overflow_hidden()
                    .child(SharedString::from(ln.to_string())));
            }
            Some(div().absolute().left(px(16.0)).bottom(px(16.0)).w(px(360.0))
                .p_3().rounded_md().bg(rgb(0xFFF6E6))
                .border_1().border_color(rgb(0xE8D5A8))
                .flex().flex_col().gap_2()
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x8A4B00))
                    .child(ui::t!("コメント — 空にして閉じると外れる")))
                .child(field)
                .child(div().flex().flex_row()
                    .child(div().flex_1())
                    .child(div().id("cmt-close").px_2p5().py_1().rounded_sm()
                        .border_1().border_color(rgb(0x8A4B00)).text_color(rgb(0x8A4B00))
                        .text_size(px(11.5)).cursor_pointer()
                        .hover(|s| s.bg(rgb(0xF7ECD8)))
                        .child(ui::t!("閉じる (Esc)"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.cmt_edit = false;
                            this.status = "".into();
                            cx.notify()
                        })))))
        };

        // 透かしの板
        let wm_panel = if !self.wm_edit {
            None
        } else {
            let mut t = self.wm_ed.text().to_string();
            let cur = self.wm_ed.cursor().min(t.len());
            t.insert(cur, '|');
            Some(div().absolute().left(px(16.0)).top(px(8.0)).w(px(360.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_2()
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x165E83))
                    .child(ui::t!("透かし — 空にして閉じると外れる")))
                .child(div().px_2().py_1().rounded_sm()
                    .border_1().border_color(rgb(0x165E83)).bg(gpui::white())
                    .text_size(px(12.5)).whitespace_nowrap().overflow_hidden()
                    .child(SharedString::from(t)))
                .child(div().flex().flex_row()
                    .child(div().flex_1())
                    .child(div().id("wm-close").px_2p5().py_1().rounded_sm()
                        .border_1().border_color(rgb(0x165E83)).text_color(rgb(0x165E83))
                        .text_size(px(11.5)).cursor_pointer()
                        .hover(|s| s.bg(rgb(0xEAF2F7)))
                        .child(ui::t!("閉じる (Esc)"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.wm_edit = false;
                            this.status = "".into();
                            cx.notify()
                        })))))
        };

        // しおりの板(名前の入力欄+一覧)
        let bm_panel = if !self.bm_open {
            None
        } else {
            let mut t = self.bm_ed.text().to_string();
            let cur = self.bm_ed.cursor().min(t.len());
            t.insert(cur, '|');
            // 一覧(名前と、その段落の頭のバイト位置)
            let mut items: Vec<(String, usize)> = Vec::new();
            let mut at = 0usize;
            for p in self.doc.paragraphs() {
                let len: usize = p.runs.iter().map(|r| r.text.len()).sum();
                for b in &p.bookmarks {
                    items.push((b.clone(), at));
                }
                at += len + 1;
            }
            let mut d = div().absolute().left(px(16.0)).top(px(8.0)).w(px(340.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_2()
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x165E83))
                    .child(ui::t!("しおり — 名前を打って追加。押すとそこへ移る")))
                .child(div().flex().flex_row().gap_2().items_center()
                    .child(div().flex_1().px_2().py_1().rounded_sm()
                        .border_1().border_color(rgb(0x1B6E3C)).bg(gpui::white())
                        .text_size(px(12.5)).whitespace_nowrap().overflow_hidden()
                        .child(SharedString::from(t)))
                    .child(div().id("bm-add").px_2p5().py_1().rounded_sm()
                        .border_1().border_color(rgb(0x1B6E3C)).text_color(rgb(0x1B6E3C))
                        .text_size(px(11.5)).cursor_pointer()
                        .hover(|s| s.bg(rgb(0xEAF5EE)))
                        .child(ui::t!("追加 (Enter)"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.bm_add();
                            cx.notify()
                        }))));
            if items.is_empty() {
                d = d.child(div().text_size(px(11.5)).text_color(rgb(0x66707A))
                    .child(ui::t!("(まだしおりはありません)")));
            }
            for (i, (name, b0)) in items.into_iter().enumerate() {
                let name2 = name.clone();
                d = d.child(div().flex().flex_row().items_center().gap_2()
                    .child(div()
                        .id(SharedString::from(format!("bm-{i}")))
                        .flex_1().px_2().py_0p5().rounded_sm()
                        .text_size(px(12.5)).cursor_pointer()
                        .hover(|s| s.bg(rgb(0xEAF2F7)))
                        .child(SharedString::from(name.clone()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.switch_target(Target::Body);
                            this.ed.move_to(b0, false);
                            this.follow_caret();
                            this.status = ui::tf!("しおり「{}」へ移りました", name).into();
                            cx.notify()
                        })))
                    .child(div()
                        .id(SharedString::from(format!("bmx-{i}")))
                        .px_1p5().py_0p5().rounded_sm()
                        .text_size(px(11.5)).text_color(rgb(0x9AA5AE)).cursor_pointer()
                        .hover(|s| s.bg(rgb(0xF6E5E2)).text_color(rgb(0xC0392B)))
                        .child("✕")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            for b in &mut this.doc.blocks {
                                if let kumihan::Block::Para(p) = b {
                                    p.bookmarks.retain(|x| *x != name2);
                                }
                            }
                            this.dirty = true;
                            this.status = ui::t!("しおりを外しました").into();
                            cx.notify()
                        }))));
            }
            Some(d)
        };

        // バージョン履歴の板(控えの一覧。押すと名無しの複製で開く)
        let hist_panel = if !self.hist_open {
            None
        } else {
            let items = self.versions();
            let mut d = div().absolute().left(px(16.0)).top(px(8.0)).w(px(360.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_2()
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x165E83))
                    .child(ui::t!("バージョン履歴 — 上書き保存のたびの控え(9世代まで)")));
            if items.is_empty() {
                d = d.child(div().text_size(px(11.5)).text_color(rgb(0x66707A))
                    .child(ui::t!("(まだ控えはありません。上書き保存すると増えます)")));
            }
            for (i, (disp, q)) in items.into_iter().enumerate() {
                d = d.child(div()
                    .id(SharedString::from(format!("hist-{i}")))
                    .px_2().py_0p5().rounded_sm()
                    .text_size(px(12.5)).cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF2F7)))
                    .child(SharedString::from(disp))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_version(&q);
                        cx.notify()
                    })));
            }
            Some(d)
        };

        // チャットの板(申し送り帳の最近の行+入力欄)
        let chat_panel = if !self.chat_open {
            None
        } else {
            let mut t = self.chat_ed.text().to_string();
            let cur = self.chat_ed.cursor().min(t.len());
            t.insert(cur, '|');
            let lines = self.chat_lines();
            let mut d = div().absolute().left(px(16.0)).top(px(8.0)).w(px(420.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_2()
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x165E83))
                    .child(ui::t!("チャット — 文書の隣の申し送り帳(.chat.txt)")));
            if lines.is_empty() {
                d = d.child(div().text_size(px(11.5)).text_color(rgb(0x66707A))
                    .child(ui::t!("(まだ書き込みはありません)")));
            }
            for l in lines {
                d = d.child(div().text_size(px(12.0))
                    .whitespace_nowrap().overflow_hidden()
                    .child(SharedString::from(l)));
            }
            d = d.child(div().flex().flex_row().gap_2().items_center()
                .child(div().flex_1().px_2().py_1().rounded_sm()
                    .border_1().border_color(rgb(0x1B6E3C)).bg(gpui::white())
                    .text_size(px(12.5)).whitespace_nowrap().overflow_hidden()
                    .child(SharedString::from(t)))
                .child(div().id("chat-send").px_2p5().py_1().rounded_sm()
                    .border_1().border_color(rgb(0x1B6E3C)).text_color(rgb(0x1B6E3C))
                    .text_size(px(11.5)).cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF5EE)))
                    .child(ui::t!("送信 (Enter)"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.chat_send();
                        cx.notify()
                    }))));
            Some(d)
        };

        // パスワードの板(伏せ字。開く時と暗号化を決める時の両方)
        let pw_panel = if !self.pw_open {
            None
        } else {
            let text = self.pw_ed.text();
            let before = text[..self.pw_ed.cursor().min(text.len())].chars().count();
            let total = text.chars().count();
            let masked = format!(
                "{}|{}",
                "●".repeat(before),
                "●".repeat(total - before)
            );
            let title = if self.pw_pending.is_some() {
                "パスワード — この文書は暗号化されています"
            } else {
                "暗号化 — パスワードを決めて Enter(空で解除。Esc で取りやめ)"
            };
            Some(div().absolute().left(px(16.0)).top(px(8.0)).w(px(380.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_2()
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x165E83))
                    .child(SharedString::from(title.to_string())))
                .child(div().px_2().py_1().rounded_sm()
                    .border_1().border_color(rgb(0x1B6E3C)).bg(gpui::white())
                    .text_size(px(12.5)).whitespace_nowrap().overflow_hidden()
                    .child(SharedString::from(masked)))
                .child(div().text_size(px(10.5)).text_color(rgb(0x66707A))
                    .child(ui::t!("方式は ECMA-376 Agile(AES-256)。\
                            Word や LibreOffice でも開けます。\
                            パスワードを忘れると誰にも開けません"))))
        };

        // URL の板(JS なしの閲覧の入口)
        let url_panel = if !self.url_open {
            None
        } else {
            let mut t = self.url_ed.text().to_string();
            let cur = self.url_ed.cursor().min(t.len());
            t.insert(cur, '|');
            Some(div().absolute().left(px(16.0)).top(px(8.0)).w(px(460.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_2()
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x165E83))
                    .child(ui::t!("URL を開く — Enter で取りに行く(JS は実行しません)")))
                .child(div().px_2().py_1().rounded_sm()
                    .border_1().border_color(rgb(0x1B6E3C)).bg(gpui::white())
                    .text_size(px(12.5)).whitespace_nowrap().overflow_hidden()
                    .child(SharedString::from(t))))
        };

        // 記入の板(HTML の form。欄を押して打ち、送信で送る)
        let fm_panel = if !self.fm_open || self.html_forms.is_empty() {
            None
        } else {
            let fm = self.html_forms[0].clone();
            let mut d = div().absolute().right(px(16.0)).top(px(8.0)).w(px(340.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_2()
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x165E83))
                    .child(ui::t!("記入 — 欄を押して打ち、Enter で控え、送信で送る")));
            for (i, f) in fm.fields.iter().enumerate() {
                if f.hidden {
                    continue;
                }
                let editing = self.fm_field == Some(i);
                let shown = if editing {
                    let mut t = self.fm_ed.text().to_string();
                    let cur = self.fm_ed.cursor().min(t.len());
                    t.insert(cur, '|');
                    t
                } else {
                    f.value.clone()
                };
                let hint = if f.options.is_empty() {
                    String::new()
                } else {
                    format!("({})", f.options.join(" / "))
                };
                let val = f.value.clone();
                d = d.child(div().flex().flex_row().items_center().gap_2()
                    .child(div().w(px(90.0)).text_size(px(11.5))
                        .text_color(rgb(0x66707A))
                        .whitespace_nowrap().overflow_hidden()
                        .child(SharedString::from(format!("{}{hint}", f.name))))
                    .child(div()
                        .id(SharedString::from(format!("fm-{i}")))
                        .flex_1().px_2().py_0p5().rounded_sm()
                        .border_1()
                        .border_color(if editing { rgb(0x1B6E3C) } else { rgb(0xC6CDD3) })
                        .bg(gpui::white())
                        .text_size(px(12.5)).cursor_pointer()
                        .whitespace_nowrap().overflow_hidden()
                        .child(SharedString::from(shown))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.fm_ed = Editor::new(&val);
                            this.fm_field = Some(i);
                            cx.notify()
                        }))));
            }
            d = d.child(div().flex().flex_row().gap_2()
                .child(div().id("fm-send").px_3().py_1().rounded_sm()
                    .border_1().border_color(rgb(0x1B6E3C)).text_color(rgb(0x1B6E3C))
                    .text_size(px(12.0)).cursor_pointer()
                    .hover(|st| st.bg(rgb(0xEAF5EE)))
                    .child(ui::tf!("送信({} {})", fm.method.to_uppercase(), fm.action))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.fm_submit(cx);
                        cx.notify()
                    }))));
            Some(d)
        };

        // 左パネル(本家のナビゲーション)。見出し / コメント / 検索の耳を持つ
        let nav_panel = if !self.nav_open {
            None
        } else {
            let panel_bg = if dk { rgb(0x1B1E21) } else { rgb(0xF1F3F5) };
            let mut d = div().absolute().left(px(0.0)).top(px(0.0))
                .w(px(250.0)).h_full().overflow_hidden()
                .p_2().bg(panel_bg)
                .border_r_1().border_color(th_cmd_border)
                .flex().flex_col().gap_1();
            // 耳
            let mut ears = div().flex().flex_row().gap_1().mb_1();
            for (i, name) in ["見出し", "コメント", "検索"].into_iter().enumerate() {
                let on = self.nav_tab == i as u8;
                ears = ears.child(div()
                    .id(SharedString::from(format!("navtab-{i}")))
                    .px_2().py_0p5().rounded_sm().cursor_pointer()
                    .text_size(px(11.5))
                    .text_color(if on { th_btn } else { th_status })
                    .font_weight(if on {
                        gpui::FontWeight::BOLD
                    } else {
                        gpui::FontWeight::NORMAL
                    })
                    .when(on, |st| st.bg(th_btn_hover))
                    .child(name)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.nav_tab = i as u8;
                        cx.notify()
                    })));
            }
            d = d.child(ears);
            match self.nav_tab {
                // 見出し(押すと飛ぶ)
                0 => {
                    let heads = self.headings();
                    if heads.is_empty() {
                        d = d.child(div().text_size(px(11.0)).text_color(th_status)
                            .child(ui::t!("(見出しがありません。ホーム > 段落のスタイルで)")));
                    }
                    for (i, (lv, text, byte)) in heads.into_iter().take(40).enumerate() {
                        let b = byte;
                        d = d.child(div()
                            .id(SharedString::from(format!("nav-{i}")))
                            .px_2().py_0p5().rounded_sm().cursor_pointer()
                            .text_size(px(12.0)).text_color(th_top_fg)
                            .whitespace_nowrap().overflow_hidden()
                            .hover(|st| st.bg(th_btn_hover))
                            .child(SharedString::from(format!(
                                "{}{text}",
                                "　".repeat((lv as usize).saturating_sub(1))
                            )))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.switch_target(Target::Body);
                                this.ed.move_to(b.min(this.ed.text().len()), false);
                                this.follow_caret();
                                cx.notify()
                            })));
                    }
                }
                // コメント(押すとその段落へ飛ぶ)
                1 => {
                    let mut items: Vec<(usize, String, String, usize)> = Vec::new();
                    let mut at = 0usize;
                    for (pi, p) in self.doc.paragraphs().enumerate() {
                        let len: usize = p.runs.iter().map(|r| r.text.len()).sum();
                        for c in &p.comments {
                            items.push((pi, c.author.clone(), c.text.clone(), at));
                        }
                        at += len + 1;
                    }
                    if items.is_empty() {
                        d = d.child(div().text_size(px(11.0)).text_color(th_status)
                            .child(ui::t!("(コメントはありません)")));
                    }
                    for (i, (_, who, text, byte)) in items.into_iter().take(30).enumerate() {
                        let b = byte;
                        d = d.child(div()
                            .id(SharedString::from(format!("navc-{i}")))
                            .px_2().py_1().rounded_sm().cursor_pointer()
                            .bg(if dk { rgb(0x22262A) } else { rgb(0xFFFFFF) })
                            .hover(|st| st.bg(th_btn_hover))
                            .flex().flex_col()
                            .child(div().text_size(px(10.5)).text_color(th_status)
                                .child(SharedString::from(who)))
                            .child(div().text_size(px(11.5)).text_color(th_top_fg)
                                .whitespace_nowrap().overflow_hidden()
                                .child(SharedString::from(text)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.switch_target(Target::Body);
                                this.ed.move_to(b.min(this.ed.text().len()), false);
                                this.follow_caret();
                                cx.notify()
                            })));
                    }
                }
                // 検索(当たった場所を並べ、押すと飛ぶ)
                _ => {
                    let term = self.find_ed.text().to_string();
                    d = d.child(div()
                        .id("nav-find")
                        .px_2().py_1().rounded_sm().cursor_pointer()
                        .border_1().border_color(rgb(0x1B6E3C))
                        .bg(if dk { rgb(0x22262A) } else { rgb(0xFFFFFF) })
                        .text_size(px(12.0)).text_color(th_top_fg)
                        .whitespace_nowrap().overflow_hidden()
                        .child(SharedString::from(if term.is_empty() {
                            ui::t!("(検索の板で語を打つ → ここに出ます)").to_string()
                        } else {
                            term.clone()
                        }))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.run_cmd("replace", cx);
                            cx.notify()
                        })));
                    if !term.is_empty() {
                        let text = self.ed.text().to_string();
                        let mut hits = 0usize;
                        for (i, at) in text.match_indices(&term).take(30).enumerate() {
                            hits += 1;
                            let b = at.0;
                            let s0 = text[..b].rfind('\n').map(|x| x + 1).unwrap_or(0);
                            let e0 = text[b..].find('\n').map(|x| b + x).unwrap_or(text.len());
                            let line: String =
                                text[s0..e0].chars().take(40).collect();
                            d = d.child(div()
                                .id(SharedString::from(format!("navf-{i}")))
                                .px_2().py_0p5().rounded_sm().cursor_pointer()
                                .text_size(px(11.5)).text_color(th_top_fg)
                                .whitespace_nowrap().overflow_hidden()
                                .hover(|st| st.bg(th_btn_hover))
                                .child(SharedString::from(line))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.switch_target(Target::Body);
                                    this.ed.move_to(b, false);
                                    this.follow_caret();
                                    cx.notify()
                                })));
                        }
                        if hits == 0 {
                            d = d.child(div().text_size(px(11.0)).text_color(th_status)
                                .child(ui::t!("(見つかりません)")));
                        }
                    }
                }
            }
            Some(d)
        };

        // 右パネル(本家の設定パネル)。**いる場所の設定を、その場で直す**
        let rp_panel = if !self.rp_open {
            None
        } else {
            let panel_bg = if dk { rgb(0x1B1E21) } else { rgb(0xF1F3F5) };
            let (pi, _) = self.cursor_para();
            let para = self.doc.paragraphs().nth(pi).cloned();
            let f = self.doc.char_format_at(self.ed.selection());
            let size_now = self.doc.size_at(self.ed.selection()).unwrap_or(SIZE_PT);
            let head = |t: &'static str| {
                div().text_size(px(11.0)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x165E83)).mt_1().child(t)
            };
            // 小さな釦(押すと run_cmd。入っていれば色が付く)
            let btn = |this: &Writer, id: &'static str, label: &'static str| {
                let on = this.toggled(id);
                div().id(SharedString::from(format!("rp-{id}")))
                    .px_2().py_0p5().rounded_sm().cursor_pointer()
                    .border_1()
                    .border_color(if on { th_btn } else { th_cmd_border })
                    .bg(if on { th_btn_hover } else { gpui::transparent_black().into() })
                    .text_size(px(11.5))
                    .text_color(if on { th_btn } else { th_top_fg })
                    .hover(move |st| st.bg(th_btn_hover))
                    .child(label)
            };
            let row = || div().flex().flex_row().flex_wrap().gap_1();
            let mut d = div().absolute().right(px(0.0)).top(px(0.0))
                .w(px(230.0)).h_full().overflow_hidden()
                .p_2().bg(panel_bg)
                .border_l_1().border_color(th_cmd_border)
                .flex().flex_col().gap_1()
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x165E83))
                    .child(ui::t!("設定 — いる場所を直す")));

            // 文字
            d = d.child(head("文字"))
                .child(row()
                    .child(btn(self, "bold", "太字").on_click(cx.listener(
                        |t, _, _, cx| { t.run_cmd("bold", cx); cx.notify() })))
                    .child(btn(self, "italic", "斜体").on_click(cx.listener(
                        |t, _, _, cx| { t.run_cmd("italic", cx); cx.notify() })))
                    .child(btn(self, "underline", "下線").on_click(cx.listener(
                        |t, _, _, cx| { t.run_cmd("underline", cx); cx.notify() })))
                    .child(btn(self, "strikeout", "取消").on_click(cx.listener(
                        |t, _, _, cx| { t.run_cmd("strikeout", cx); cx.notify() }))))
                .child(row()
                    .child(div().text_size(px(11.0)).text_color(th_status)
                        .child(SharedString::from(ui::tf!("大きさ {} pt / 書体 {}", if size_now.fract() == 0.0 {
                                format!("{}", size_now as i32)
                            } else {
                                format!("{size_now}")
                            }, self.font_name)))))
                .child(row()
                    .child(btn(self, "decfont", "小さく").on_click(cx.listener(
                        |t, _, _, cx| { t.run_cmd("decfont", cx); cx.notify() })))
                    .child(btn(self, "incfont", "大きく").on_click(cx.listener(
                        |t, _, _, cx| { t.run_cmd("incfont", cx); cx.notify() })))
                    .child(btn(self, "fontcolor", "色").on_click(cx.listener(
                        |t, _, _, cx| { t.run_cmd("fontcolor", cx); cx.notify() })))
                    .child(btn(self, "clearstyle", "書式を消す").on_click(cx.listener(
                        |t, _, _, cx| { t.run_cmd("clearstyle", cx); cx.notify() }))));
            if f.field.is_some() {
                d = d.child(div().text_size(px(10.5)).text_color(th_status)
                    .child(ui::t!("(ここは相互参照。更新は 参考資料 > 相互参照)")));
            }
            if let Some(rt) = &f.ruby {
                d = d.child(div().text_size(px(10.5)).text_color(th_status)
                    .child(SharedString::from(ui::tf!("ルビ「{}」", rt))));
            }

            // 段落
            let (al, ls, ind, lst) = match &para {
                Some(p) => (p.align, p.spacing(), p.indent, p.list),
                None => (Align::Left, 1.0, 0, ListKind::None),
            };
            d = d.child(head("段落"))
                .child(row()
                    .children([
                        ("align-left", "左", Align::Left),
                        ("align-center", "中央", Align::Center),
                        ("align-right", "右", Align::Right),
                        ("align-just", "両端", Align::Justify),
                        ("align-dist", "均等", Align::Distribute),
                    ].map(|(id, label, a)| {
                        let on = al == a;
                        div().id(SharedString::from(format!("rp-{id}")))
                            .px_2().py_0p5().rounded_sm().cursor_pointer()
                            .border_1()
                            .border_color(if on { th_btn } else { th_cmd_border })
                            .bg(if on { th_btn_hover } else { gpui::transparent_black().into() })
                            .text_size(px(11.5))
                            .text_color(if on { th_btn } else { th_top_fg })
                            .hover(move |st| st.bg(th_btn_hover))
                            .child(label)
                            .on_click(cx.listener(move |t, _, _, cx| {
                                t.run_cmd(id, cx);
                                cx.notify()
                            }))
                    })))
                .child(row()
                    .child(div().text_size(px(11.0)).text_color(th_status)
                        .child(SharedString::from(ui::tf!("行間 {:.2} / 字下げ {}", ls, ind)))))
                .child(row()
                    .child(btn(self, "linespace", "行間").on_click(cx.listener(
                        |t, _, _, cx| { t.run_cmd("linespace", cx); cx.notify() })))
                    .child(btn(self, "decoffset", "◂ 字下げ").on_click(cx.listener(
                        |t, _, _, cx| { t.run_cmd("decoffset", cx); cx.notify() })))
                    .child(btn(self, "incoffset", "字下げ ▸").on_click(cx.listener(
                        |t, _, _, cx| { t.run_cmd("incoffset", cx); cx.notify() }))))
                .child(row()
                    .child(btn(self, "markers", if lst == ListKind::Bullet {
                        "箇条書き ✓"
                    } else {
                        "箇条書き"
                    }).on_click(cx.listener(
                        |t, _, _, cx| { t.run_cmd("markers", cx); cx.notify() })))
                    .child(btn(self, "numbering", if lst == ListKind::Number {
                        "番号 ✓"
                    } else {
                        "番号"
                    }).on_click(cx.listener(
                        |t, _, _, cx| { t.run_cmd("numbering", cx); cx.notify() })))
                    .child(btn(self, "paracolor", "背景").on_click(cx.listener(
                        |t, _, _, cx| { t.run_cmd("paracolor", cx); cx.notify() })))
                    .child(btn(self, "borders", "囲み").on_click(cx.listener(
                        |t, _, _, cx| { t.run_cmd("borders", cx); cx.notify() }))));

            // ページ
            d = d.child(head("ページ"))
                .child(div().text_size(px(11.0)).text_color(th_status)
                    .child(SharedString::from(ui::tf!("{:.0}×{:.0}mm / 余白 {:.0}mm / {}段{}", self.pg.w_mm, self.pg.h_mm, self.pg.left_mm, self.pg.cols(), if self.doc.vertical { " / 縦書き" } else { "" }))))
                .child(row()
                    .child(btn(self, "pageorient", "向き").on_click(cx.listener(
                        |t, _, _, cx| { t.run_cmd("pageorient", cx); cx.notify() })))
                    .child(btn(self, "pagesize", "用紙").on_click(cx.listener(
                        |t, _, _, cx| { t.run_cmd("pagesize", cx); cx.notify() })))
                    .child(btn(self, "pagemargins", "余白").on_click(cx.listener(
                        |t, _, _, cx| { t.run_cmd("pagemargins", cx); cx.notify() })))
                    .child(btn(self, "columns", "段組み").on_click(cx.listener(
                        |t, _, _, cx| { t.run_cmd("columns", cx); cx.notify() })))
                    .child(btn(self, "direction", "縦書き").on_click(cx.listener(
                        |t, _, _, cx| { t.run_cmd("direction", cx); cx.notify() }))));
            Some(d)
        };

        // リンクの板(押すと辿る。公開 Web も見える — JS は実行しない)
        let lk_panel = if !self.lk_open || self.html_links.is_empty() {
            None
        } else {
            let mut d = div().absolute().right(px(16.0)).bottom(px(8.0)).w(px(340.0))
                .max_h(px(300.0)).overflow_hidden()
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_1()
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x165E83))
                    .child(SharedString::from(ui::tf!("リンク({}件。押すと辿る。Esc で閉じる)", self.html_links.len()))));
            for (i, (href, text)) in self.html_links.iter().take(16).enumerate() {
                let href2 = href.clone();
                d = d.child(div()
                    .id(SharedString::from(format!("lk-{i}")))
                    .px_2().py_0p5().rounded_sm().cursor_pointer()
                    .text_size(px(12.0)).text_color(rgb(0x165E83))
                    .whitespace_nowrap().overflow_hidden()
                    .hover(|st| st.bg(rgb(0xEAF2F7)))
                    .child(SharedString::from(text.clone()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.follow_link(href2.clone(), cx);
                        cx.notify()
                    })));
            }
            if self.html_links.len() > 16 {
                d = d.child(div().text_size(px(10.5)).text_color(rgb(0x66707A))
                    .child(SharedString::from(ui::tf!("(あと {} 件は出していません)", self.html_links.len() - 16))));
            }
            Some(d)
        };

        // AI に頼む板
        let ai_panel = if !self.ai_open {
            None
        } else {
            let mut t = self.ai_ed.text().to_string();
            let cur = self.ai_ed.cursor().min(t.len());
            t.insert(cur, '|');
            Some(div().absolute().left(px(16.0)).top(px(8.0)).w(px(460.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_2()
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x165E83))
                    .child(SharedString::from(ui::tf!("{} — 宛先 {}(Esc で取りやめ)", if self.ai_macro { "AI にマクロ台本を頼む" } else { "AI に頼む" }, ui::ai::backend().label()))))
                .child(div().px_2().py_1().rounded_sm()
                    .border_1().border_color(rgb(0x1B6E3C)).bg(rgb(0xFFFFFF))
                    .text_size(px(12.5)).whitespace_nowrap().overflow_hidden()
                    .child(SharedString::from(t)))
                .child(div().text_size(px(10.5)).text_color(rgb(0x66707A))
                    .child(if self.ai_macro {
                        "台本はプラグイン置き場に置くだけです。読んで確かめてから\
                         一覧で実行してください(自動では走りません)"
                    } else {
                        "答えはカーソルの位置に入ります。Ctrl+Z で1手で戻せます"
                    })))
        };

        // 記入欄の選択肢を聞く板
        let sd_panel = if !self.sd_open {
            None
        } else {
            let mut t = self.sd_ed.text().to_string();
            let cur = self.sd_ed.cursor().min(t.len());
            t.insert(cur, '|');
            Some(div().absolute().left(px(16.0)).top(px(8.0)).w(px(400.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_2()
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x165E83))
                    .child(SharedString::from(if self.sd_naming {
                        ui::t!("記入欄の名前 — 打って Enter(例: 氏名。\
                         マクロの fill(名前, 値) が引く鍵)").to_string()
                    } else {
                        ui::tf!("{}の選択肢 — カンマ区切りで打って Enter(例: 赤,青,黄)", self.sd_kind.label())
                    })))
                .child(div().px_2().py_1().rounded_sm()
                    .border_1().border_color(rgb(0x1B6E3C)).bg(rgb(0xFFFFFF))
                    .text_size(px(12.5)).whitespace_nowrap().overflow_hidden()
                    .child(SharedString::from(t))))
        };

        // ルビの板(読みの入力)
        let rb_panel = if !self.rb_open {
            None
        } else {
            let mut t = self.rb_ed.text().to_string();
            let cur = self.rb_ed.cursor().min(t.len());
            t.insert(cur, '|');
            Some(div().absolute().left(px(16.0)).top(px(8.0)).w(px(360.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_2()
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x165E83))
                    .child(ui::t!("ルビ — 読みを打って Enter(空で外す。Esc で取りやめ)")))
                .child(div().px_2().py_1().rounded_sm()
                    .border_1().border_color(rgb(0x1B6E3C)).bg(gpui::white())
                    .text_size(px(12.5)).whitespace_nowrap().overflow_hidden()
                    .child(SharedString::from(t))))
        };

        // プラグインの板(置き場の .py 一覧。押すと檻の中で実行)
        let plug_panel = if !self.plug_open {
            None
        } else {
            let dir = plugins_dir();
            let mut items: Vec<PathBuf> = std::fs::read_dir(&dir)
                .ok()
                .map(|rd| {
                    rd.flatten()
                        .map(|e| e.path())
                        .filter(|p| p.extension().is_some_and(|e| e == "py"))
                        .collect()
                })
                .unwrap_or_default();
            items.sort();
            let mut d = div().absolute().left(px(16.0)).top(px(8.0)).w(px(420.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_2()
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0x165E83))
                    .child(ui::t!("プラグイン — 押すと檻(bubblewrap)の中で実行")))
                .child(div().text_size(px(11.0)).text_color(rgb(0x66707A))
                    .child(SharedString::from(ui::tf!("置き場: {}", dir.display()))));
            if items.is_empty() {
                d = d.child(div().text_size(px(11.5)).text_color(rgb(0x66707A))
                    .child(ui::t!("(まだありません。置き場に .py を置いてください。\
                            台本の d が python-docx の文書、fill(名前, 値)・\
                            extract(名前)・fields() で記入欄の出し入れ)")));
            }
            for (i, q) in items.into_iter().enumerate() {
                let name = q
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                d = d.child(div()
                    .id(SharedString::from(format!("plug-{i}")))
                    .px_2().py_0p5().rounded_sm()
                    .text_size(px(12.5)).cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF2F7)))
                    .child(SharedString::from(name))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.plug_open = false;
                        this.run_macro_file(q.clone(), cx);
                        cx.notify()
                    })));
            }
            Some(d)
        };

        // 相互参照の板(しおり一覧 → 文字/ページを挿す。更新もここ)
        let xr_panel = if !self.xr_open {
            None
        } else {
            let names: Vec<String> = self
                .doc
                .paragraphs()
                .flat_map(|p| p.bookmarks.iter().cloned())
                .collect();
            let mut d = div().absolute().left(px(16.0)).top(px(8.0)).w(px(360.0))
                .p_3().rounded_md().bg(rgb(0xF7F9FA))
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_2()
                .child(div().flex().flex_row().items_center()
                    .child(div().flex_1().text_size(px(11.5))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(0x165E83))
                        .child(ui::t!("相互参照 — しおりの文字かページ番号を挿す")))
                    .child(div().id("xr-refresh").px_2().py_0p5().rounded_sm()
                        .border_1().border_color(rgb(0x1B6E3C)).text_color(rgb(0x1B6E3C))
                        .text_size(px(11.0)).cursor_pointer()
                        .hover(|s| s.bg(rgb(0xEAF5EE)))
                        .child(ui::t!("参照を更新"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.refresh_refs();
                            cx.notify()
                        }))));
            if names.is_empty() {
                d = d.child(div().text_size(px(11.5)).text_color(rgb(0x66707A))
                    .child(ui::t!("(しおりがありません。参考資料 > ブックマークで付けてください)")));
            }
            for (i, name) in names.into_iter().enumerate() {
                let n1 = name.clone();
                let n2 = name.clone();
                d = d.child(div().flex().flex_row().items_center().gap_2()
                    .child(div().flex_1().text_size(px(12.5))
                        .whitespace_nowrap().overflow_hidden()
                        .child(SharedString::from(name)))
                    .child(div().id(SharedString::from(format!("xrt-{i}")))
                        .px_2().py_0p5().rounded_sm()
                        .border_1().border_color(rgb(0x165E83)).text_color(rgb(0x165E83))
                        .text_size(px(11.0)).cursor_pointer()
                        .hover(|s| s.bg(rgb(0xEAF2F7)))
                        .child(ui::t!("文字"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.insert_ref(&n1, false);
                            cx.notify()
                        })))
                    .child(div().id(SharedString::from(format!("xrp-{i}")))
                        .px_2().py_0p5().rounded_sm()
                        .border_1().border_color(rgb(0x165E83)).text_color(rgb(0x165E83))
                        .text_size(px(11.0)).cursor_pointer()
                        .hover(|s| s.bg(rgb(0xEAF2F7)))
                        .child(ui::t!("ページ"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.insert_ref(&n2, true);
                            cx.notify()
                        }))));
            }
            Some(d)
        };

        // フォントの一覧。この機械にある日本語の書体だけ
        let font_panel = if !self.font_list {
            None
        } else {
            let names: Vec<String> = kumihan::font::list()
                .iter()
                .filter(|f| f.japanese && f.regular)
                .map(|f| f.name.clone())
                .take(24)
                .collect();
            let mut d = div().absolute().left(px(16.0)).top(px(8.0)).w(px(280.0))
                .p_2().rounded_md().bg(gpui::white())
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_0p5()
                .child(div().text_size(px(10.5)).text_color(rgb(0x66707A))
                    .child(ui::t!("書体(選んだ段落に掛かる)")));
            for name in names {
                let shown = SharedString::from(name.clone());
                let is_current = self.font_name.as_ref() == name.as_str();
                d = d.child(div()
                    .id(SharedString::from(format!("font-{name}")))
                    .px_2().py_0p5().rounded_sm()
                    .text_size(px(12.5))
                    .font_family(shown.clone())
                    .bg(if is_current { rgb(0xEAF5EE) } else { rgb(0xFFFFFF) })
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF2F7)))
                    .child(shown)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let n = name.clone();
                        let sel = this.ed.selection();
                        this.flush_target();
                        this.doc.apply_font(sel, Some(n.clone()));
                        this.dirty = true;
                        this.relayout_keep();
                        this.font_list = false;
                        this.status = ui::tf!("書体を「{}」に", n).into();
                        cx.notify();
                    })));
            }
            Some(d)
        };

        // 大きさの一覧
        let size_panel = if !self.size_list {
            None
        } else {
            let mut d = div().absolute().left(px(16.0)).top(px(8.0)).w(px(200.0))
                .p_2().rounded_md().bg(gpui::white())
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_row().flex_wrap().gap_1();
            for pt in [8.0f32, 9.0, 10.0, 10.5, 11.0, 12.0, 14.0, 16.0, 18.0, 22.0, 26.0, 36.0] {
                d = d.child(div()
                    .id(SharedString::from(format!("pt-{pt}")))
                    .px_2().py_1().rounded_sm().text_size(px(12.0))
                    .cursor_pointer().hover(|s| s.bg(rgb(0xEAF2F7)))
                    .child(SharedString::from(format!("{pt}")))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let sel = this.ed.selection();
                        this.flush_target();
                        this.doc.apply_size(sel, move |_| pt);
                        this.dirty = true;
                        this.relayout_keep();
                        this.size_list = false;
                        this.status = ui::tf!("大きさを {}pt に", pt).into();
                        cx.notify();
                    })));
            }
            Some(d)
        };

        // 段落のスタイルの一覧(標準・見出し1〜3)
        let style_panel = if !self.style_list {
            None
        } else {
            let mut d = div().absolute().left(px(16.0)).top(px(8.0)).w(px(240.0))
                .p_2().rounded_md().bg(gpui::white())
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_col().gap_0p5()
                .child(div().text_size(px(10.5)).text_color(rgb(0x66707A))
                    .child(ui::t!("段落のスタイル(選んだ段落に掛かる)")));
            for (n, label, pt, bold) in [
                (0u8, "標準", 12.5f32, false),
                (1, "見出し1", 16.0, true),
                (2, "見出し2", 14.0, true),
                (3, "見出し3", 12.5, true),
            ] {
                let mut item = div()
                    .id(SharedString::from(format!("style-{n}")))
                    .px_2().py_0p5().rounded_sm()
                    .text_size(px(pt))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF2F7)))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_para_style(n);
                        this.style_list = false;
                        cx.notify();
                    }));
                if bold {
                    item = item.font_weight(gpui::FontWeight::BOLD);
                }
                d = d.child(item);
            }
            Some(d)
        };

        // 記号の一覧。事務の書類で使うものだけ(飾りの絵文字は入れない)
        let symbol_panel = if !self.symbols {
            None
        } else {
            const SYMS: &[&str] = &[
                "〒", "※", "→", "←", "↑", "↓", "℃", "±", "×", "÷",
                "①", "②", "③", "④", "⑤", "⑥", "⑦", "⑧", "⑨", "⑩",
                "㈱", "㈲", "№", "〆", "〜", "…", "・", "「", "」", "『",
                "』", "【", "】", "○", "●", "◎", "△", "▲", "□", "■",
            ];
            let mut d = div().absolute().right(px(16.0)).top(px(8.0)).w(px(340.0))
                .p_2().rounded_md().bg(gpui::white())
                .border_1().border_color(rgb(0xC6CDD3))
                .flex().flex_row().flex_wrap().gap_1();
            for s in SYMS {
                d = d.child(div()
                    .id(SharedString::from(format!("sym-{s}")))
                    .w(px(28.0)).h(px(28.0)).rounded_sm()
                    .flex().items_center().justify_center()
                    .text_size(px(15.0)).cursor_pointer()
                    .hover(|st| st.bg(rgb(0xEAF2F7)))
                    .child(SharedString::from(*s))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ed.insert(s);
                        this.on_edited();
                        cx.notify();
                    })));
            }
            Some(d)
        };

        // 校正の指摘
        let proof_panel = if self.proof.is_empty() && self.proof_msg.is_empty() {
            None
        } else {
            let mut d = div().absolute().right(px(16.0)).bottom(px(16.0)).w(px(300.0))
                .p_3().rounded_md().bg(gpui::white())
                .border_1().border_color(rgb(0xC6CDD3))
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                       .text_color(rgb(0x165E83))
                       .child(SharedString::from(ui::tf!("校正 — {}", self.proof_msg))));
            for n in &self.proof {
                // どちらの道具が出したかを隠さない。辞書の指摘は GPU 無しで再現できる
                let tool = match n.source {
                    ui::check::Source::Dictionary => "辞書",
                    ui::check::Source::Model => "モデル",
                };
                let cand = if n.candidates.is_empty() {
                    ui::t!("候補なし").to_string()
                } else {
                    n.candidates.join(" / ")
                };
                d = d.child(div().mt_1p5().text_size(px(11.5))
                    .child(SharedString::from(
                        ui::tf!("{} → {}  ({}・{})", n.found, cand, n.kind.label(), tool))));
            }
            Some(d)
        };

        // ---- 右クリックのメニュー ----
        // InputSink より後に描く(bubble は後に登録した方が先に走るので、
        // 項目の stop_propagation がクリック処理より先に効く — calc と同じ)
        let menu = self.menu_at.map(|(mx, my)| {
            let has_sel = self.ed.has_selection();
            // (id, 名前, 付記, 押せるか)。"" は仕切り
            let entries: Vec<(&'static str, &'static str, &'static str, bool)> = vec![
                ("cut", "切り取り", "Ctrl+X", has_sel),
                ("copy", "コピー", "Ctrl+C", has_sel),
                ("paste", "貼り付け", "Ctrl+V", true),
                ("", "", "", false),
                ("selword", "語を選択", "", true),
                ("selline", "行を選択", "", true),
                ("selall", "すべて選択", "Ctrl+A", true),
                ("", "", "", false),
                ("bold", "太字", "", true),
                ("italic", "斜体", "", true),
                ("underline", "下線", "", true),
                ("", "", "", false),
                ("align-left", "左揃え", "", true),
                ("align-center", "中央揃え", "", true),
                ("align-right", "右揃え", "", true),
                ("align-just", "両端揃え", "", true),
                ("", "", "", false),
                ("replace", "検索と置換", "Ctrl+F", true),
                ("comment", "コメント", "", true),
                ("wordcount", "文字数を数える", "", true),
            ];
            let h_est = entries.len() as f32 * 25.0 + 10.0;
            let win_w = f32::from(window.viewport_size().width);
            let mx = mx.min((win_w - 28.0 - 230.0).max(0.0));
            let my = my.min((self.view_h_px - h_est).max(0.0));
            let mut m = div().absolute().left(px(mx)).top(px(my)).w(px(220.0))
                .p_1().rounded_md().bg(rgb(0xFFFFFF))
                .border_1().border_color(rgb(0xC6CDD3)).shadow_lg()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation());
            for (i, (id, label, hint, ready)) in entries.into_iter().enumerate() {
                if id.is_empty() && label.is_empty() {
                    m = m.child(div().h(px(1.0)).my_1().bg(rgb(0xE1E6EA)));
                    continue;
                }
                if !ready {
                    m = m.child(div()
                        .flex().flex_row().items_center().justify_between().gap_4()
                        .px_3().py_1()
                        .child(div().text_size(px(12.5)).text_color(rgb(0xB6BDC4)).child(label))
                        .child(div().text_size(px(10.5)).text_color(rgb(0xD5DBE0)).child(hint)));
                    continue;
                }
                m = m.child(div()
                    .id(SharedString::from(format!("wm{i}")))
                    .flex().flex_row().items_center().justify_between().gap_4()
                    .px_3().py_1().rounded_sm().cursor_pointer()
                    .hover(|s| s.bg(rgb(0xEAF2F7)))
                    .child(div().text_size(px(12.5)).text_color(rgb(0x1B1B1B)).child(label))
                    .child(div().text_size(px(10.5)).text_color(rgb(0x9AA5AE)).child(hint))
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                        move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.menu_action(id, window, cx);
                        })));
            }
            m
        });

        let notes = if self.notes.is_empty() { None } else {
            let mut n = div().absolute().right(px(16.0)).top(px(14.0)).w(px(270.0))
                .p_3().rounded_md().bg(rgb(0xFFF6E6))
                .border_1().border_color(rgb(0xE8D5A8))
                .child(div().text_size(px(11.5)).font_weight(gpui::FontWeight::BOLD)
                       .text_color(rgb(0x8A4B00)).child(ui::t!("この版で読み飛ばしたもの")));
            for x in &self.notes {
                n = n.child(div().text_size(px(11.0)).text_color(rgb(0x8A4B00))
                            .child(x.clone()));
            }
            Some(n)
        };

        div().size_full().flex().flex_col().bg(th_desk)
            .key_context("jo_edit")
            .track_focus(&self.focus)
            .on_action(cx.listener(Writer::backspace))
            .on_action(cx.listener(Writer::delete))
            .on_action(cx.listener(Writer::left))
            .on_action(cx.listener(Writer::right))
            .on_action(cx.listener(Writer::select_left))
            .on_action(cx.listener(Writer::select_right))
            .on_action(cx.listener(Writer::select_all))
            .on_action(cx.listener(Writer::up))
            .on_action(cx.listener(Writer::down))
            .on_action(cx.listener(Writer::select_up))
            .on_action(cx.listener(Writer::select_down))
            .on_action(cx.listener(Writer::word_left))
            .on_action(cx.listener(Writer::word_right))
            .on_action(cx.listener(Writer::select_word_left))
            .on_action(cx.listener(Writer::select_word_right))
            .on_action(cx.listener(Writer::a_tab))
            .on_action(cx.listener(Writer::a_shift_tab))
            .on_action(cx.listener(Writer::page_up))
            .on_action(cx.listener(Writer::page_down))
            .on_action(cx.listener(Writer::do_find))
            .on_action(cx.listener(Writer::a_context_menu))
            .on_action(cx.listener(Writer::a_cancel))
            .on_action(cx.listener(Writer::doc_home))
            .on_action(cx.listener(Writer::doc_end))
            .on_action(cx.listener(Writer::home))
            .on_action(cx.listener(Writer::end))
            .on_action(cx.listener(Writer::enter))
            .on_action(cx.listener(Writer::copy))
            .on_action(cx.listener(Writer::cut))
            .on_action(cx.listener(Writer::paste))
            .on_action(cx.listener(Writer::undo))
            .on_action(cx.listener(Writer::redo))
            .on_action(cx.listener(Writer::do_save))
            .on_action(cx.listener(Writer::do_open))
            .on_action(cx.listener(Writer::do_quit))
            .child(bar)
            .child(if let Some(fp) = filepage {
                fp
            } else {
                div().flex_1().relative().overflow_hidden()
                    .on_scroll_wheel(cx.listener(|this, e: &gpui::ScrollWheelEvent, _, cx| {
                        // 上に回すと delta は正 → 紙は頭の方へ戻る
                        let dy = match e.delta {
                            gpui::ScrollDelta::Pixels(p) => f32::from(p.y),
                            gpui::ScrollDelta::Lines(l) => l.y * 40.0,
                        };
                        this.scroll_px(-dy);
                        cx.notify();
                    }))
                    .child(paper)
                    .children(notes)
                    .children(find_panel)
                    .children(hf_panel)
                    .children(cmt_panel)
                    .children(wm_panel)
                    .children(bm_panel)
                    .children(xr_panel)
                    .children(hist_panel)
                    .children(chat_panel)
                    .children(plug_panel)
                    .children(pw_panel)
                    .children(rb_panel)
                    .children(sd_panel)
                    .children(ai_panel)
                    .children(url_panel)
                    .children(fm_panel)
                    .children(lk_panel)
                    .children(nav_panel)
                    .children(rp_panel)
                    .children(font_panel)
                    .children(size_panel)
                    .children(style_panel)
                    .children(symbol_panel)
                    .children(proof_panel)
                    // 終了確認の板(窓の中の中央。rfd はスクリーン中央に出て遠い)
                    .children(self.quit_ask.then(|| {
                        let btn = |id: &'static str, label: String, primary: bool| {
                            div().id(id).px_3().py_1().rounded_sm().text_size(px(12.5))
                                .border_1()
                                .border_color(if primary { rgb(0x165E83) } else { rgb(0xC6CDD3) })
                                .bg(if primary { rgb(0x165E83) } else { rgb(0xFFFFFF) })
                                .text_color(if primary { rgb(0xFFFFFF) } else { rgb(0x1B1B1B) })
                                .cursor_pointer()
                                .child(SharedString::from(label))
                        };
                        div().absolute().inset_0().flex().items_center().justify_center()
                            .child(div().w(px(420.0)).p_3().rounded_md().bg(rgb(0xF7F9FA))
                                .border_1().border_color(rgb(0x165E83)).shadow_lg()
                                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation()
                                })
                                .flex().flex_col().gap_2()
                                .child(div().text_size(px(13.0))
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(0x165E83))
                                    .child(ui::t!("保存していない変更があります")))
                                .child(div().text_size(px(12.0))
                                    .child(ui::t!(
                                        "保存して終了しますか?(Enter = 保存して終了 / Esc = やめる)")))
                                .child(div().flex().flex_row().gap_2().justify_center()
                                    .child(btn("q-save", ui::t!("保存して終了").to_string(), true)
                                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                                            |this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.quit_ask = false;
                                                this.save(true, cx);
                                                cx.notify();
                                            })))
                                    .child(btn("q-drop", ui::t!("保存せず終了").to_string(), false)
                                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                                            |this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.release_lock();
                                                cx.quit();
                                            })))
                                    .child(btn("q-cancel", ui::t!("キャンセル").to_string(), false)
                                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(
                                            |this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.quit_ask = false;
                                                this.status = ui::t!("終了をやめました").into();
                                                cx.notify();
                                            })))))
                    }))
                    .child(InputSink { view: me })
                    .children(menu)
            })
            .children(self.show_statusbar.then_some(statusbar))
            // 窓の縁のつかみ(最後に描く = 最初にマウスを受ける)。
            // GNOME の Wayland は外枠を付けないので、これが無いと
            // 大きさを変えられない(calc と共通 — ui::resize_edges)
            .children(ui::resize_edges(window))
    }
}

/// 入力ハンドラは **paint のときに窓へ差す**(GPUI の作法)。
/// 何も描かない要素だが、これが無いと IME もキー入力も届かない。
struct InputSink {
    view: Entity<Writer>,
}

impl IntoElement for InputSink {
    type Element = Self;
    fn into_element(self) -> Self { self }
}

impl gpui::Element for InputSink {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> { None }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> { None }

    fn request_layout(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, ()) {
        let mut style = gpui::Style::default();
        style.size.width = gpui::relative(1.0).into();
        style.size.height = gpui::relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<gpui::Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {}

    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<gpui::Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.view.read(cx).focus.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );
        // クリックでカーソルを置く。編集領域の座標を知っているのはここだけ
        let view = self.view.clone();
        window.on_mouse_event(move |e: &gpui::MouseDownEvent, phase, _w, cx| {
            if phase != gpui::DispatchPhase::Bubble
                || e.button != gpui::MouseButton::Left
                || !bounds.contains(&e.position)
            {
                return;
            }
            let rel = e.position - bounds.origin;
            let clicks = e.click_count;
            let shift = e.modifiers.shift;
            view.update(cx, |w, cx| {
                if w.tab == 0 {
                    // ファイルのページ。紙は無いのでキャレットも筆も動かさない
                    return;
                }
                w.menu_at = None;
                if w.tool.is_some() {
                    // 道具の間、マウスは筆になる(文字は選ばない)
                    let pxmm = PX_PER_MM * w.zoom;
                    let x = (f32::from(rel.x) - 28.0) / pxmm;
                    let y = (f32::from(rel.y) - 14.0) / pxmm + w.scroll_mm;
                    w.ink_begin(x, y);
                    cx.notify();
                    return;
                }
                match clicks {
                    // 二度押しは語、三度押しは行を選ぶ
                    2 => {
                        w.click_at(f32::from(rel.x), f32::from(rel.y), false);
                        w.select_word();
                        w.drag_select = false;
                    }
                    c if c >= 3 => {
                        w.click_at(f32::from(rel.x), f32::from(rel.y), false);
                        w.select_line();
                        w.drag_select = false;
                    }
                    _ => {
                        w.click_at(f32::from(rel.x), f32::from(rel.y), shift);
                        w.drag_select = true;
                    }
                }
                cx.notify();
            });
        });
        // 押したまま動かすと選択が伸びる(文字の選択の通り相場)
        let view = self.view.clone();
        window.on_mouse_event(move |e: &gpui::MouseMoveEvent, phase, _w, cx| {
            if phase != gpui::DispatchPhase::Bubble
                || e.pressed_button != Some(gpui::MouseButton::Left)
            {
                return;
            }
            let rel = e.position - bounds.origin;
            view.update(cx, |w, cx| {
                if w.tool.is_some() {
                    let pxmm = PX_PER_MM * w.zoom;
                    let x = (f32::from(rel.x) - 28.0) / pxmm;
                    let y = (f32::from(rel.y) - 14.0) / pxmm + w.scroll_mm;
                    w.ink_move(x, y);
                    cx.notify();
                    return;
                }
                if w.drag_select {
                    w.click_at(f32::from(rel.x), f32::from(rel.y), true);
                    cx.notify();
                }
            });
        });
        let view = self.view.clone();
        window.on_mouse_event(move |e: &gpui::MouseUpEvent, phase, _w, cx| {
            if phase != gpui::DispatchPhase::Bubble || e.button != gpui::MouseButton::Left {
                return;
            }
            view.update(cx, |w, cx| {
                if w.tool.is_some() {
                    w.ink_end();
                    cx.notify();
                }
                w.drag_select = false;
            });
        });
        // 右クリックでメニュー。選択があれば選択への操作、無ければ押した所へ
        let view = self.view.clone();
        window.on_mouse_event(move |e: &gpui::MouseDownEvent, phase, _w, cx| {
            if phase != gpui::DispatchPhase::Bubble
                || e.button != gpui::MouseButton::Right
                || !bounds.contains(&e.position)
            {
                return;
            }
            let rel = e.position - bounds.origin;
            view.update(cx, |w, cx| {
                if !w.ed.has_selection() {
                    w.click_at(f32::from(rel.x), f32::from(rel.y), false);
                }
                w.menu_at = Some((f32::from(rel.x), f32::from(rel.y)));
                cx.notify();
            });
        });
    }
}

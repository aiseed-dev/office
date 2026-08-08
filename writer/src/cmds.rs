//! writer のリボンのボタンと右クリックの受け口(main.rs から純移動 2026-08-08。
//! 部屋割りの5歩目)。run_cmd が1,095行の最大の塊だった。**純移動**

use crate::*;

impl Writer {
    pub(crate) fn run_cmd(&mut self, id: &str, cx: &mut Context<Self>) {
        // 読み取り専用の保護。文書を変えるボタンはここで断る(見る・出す・
        // 保存・検索の類いは通す)。解除はいつでも「保護」のボタン1手
        const READONLY_OK: &[&str] = &[
            "open", "save", "pdf", "zoom-in", "zoom-out", "ruler", "darkmode",
            "line-numbers", "hidenchars", "selectall", "spell", "wordcount",
            "co-showcomment", "replace", "prot-doc", "coauth-mode",
            "co-history", "co-chat", "prot-encrypt", "prot-sign", "copy",
        ];
        if self.protected() && !READONLY_OK.contains(&id) {
            self.status =
                ui::t!("読み取り専用で保護されています(保護タブの「保護」で解除できます)").into();
            return;
        }
        match id {
            "open" => self.open_dialog(cx),
            "save" => self.save(false, cx),
            "undo" => { if self.editor().undo() { self.on_edited() } }
            "redo" => { if self.editor().redo() { self.on_edited() } }
            "selectall" => self.ed.select_all(),
            "spell" => self.run_proof(),
            // 文字書式 — 押すたびに入切する(Word と同じ挙動)。
            // **先にカーソル位置の書式で入か切かを決めて、選択全体に写す** —
            // 混ざった選択で run ごとに反転させない(Word の作法)
            "bold" => {
                let on = !self.doc.char_format_at(self.ed.selection()).bold;
                self.toggle(move |f| f.bold = on);
            }
            "italic" => {
                let on = !self.doc.char_format_at(self.ed.selection()).italic;
                self.toggle(move |f| f.italic = on);
            }
            "underline" => {
                let on = !self.doc.char_format_at(self.ed.selection()).underline;
                self.toggle(move |f| f.underline = on);
            }
            "strikeout" => {
                let on = !self.doc.char_format_at(self.ed.selection()).strike;
                self.toggle(move |f| f.strike = on);
            }
            // 上付きと下付きは同時には成らない
            "superscript" => {
                let on = !self.doc.char_format_at(self.ed.selection()).superscript;
                self.toggle(move |f| {
                    f.superscript = on;
                    if on { f.subscript = false }
                });
            }
            "subscript" => {
                let on = !self.doc.char_format_at(self.ed.selection()).subscript;
                self.toggle(move |f| {
                    f.subscript = on;
                    if on { f.superscript = false }
                });
            }
            // 蛍光ペン。黄 → 緑 → 解除(色を選ぶ小窓はまだ無い)
            "highlight" => {
                let next = match self.doc.char_format_at(self.ed.selection())
                    .highlight.as_deref()
                {
                    None => Some("yellow".to_string()),
                    Some("yellow") => Some("green".to_string()),
                    _ => None,
                };
                self.toggle(move |f| f.highlight = next.clone());
            }
            // 書式のクリア。文字書式だけを外す(本文と段落の性質は残す)
            "clearstyle" => self.toggle(|f| *f = Default::default()),
            // 段落の揃え
            "align-left" => self.set_align(Align::Left),
            "align-center" => self.set_align(Align::Center),
            "align-right" => self.set_align(Align::Right),
            "align-just" => self.set_align(Align::Justify),
            // 均等割付(日本語一級)。最後の行も行長いっぱいに字間を配る
            "align-dist" => self.set_align(Align::Distribute),
            // 縦書き(K4)。sectPr の textDirection=tbRl と往復。
            // 初版の約束: 表・段組みは縦にならず、ASCII は1字ずつ縦に積む
            "direction" => {
                self.flush_target();
                self.doc.vertical = !self.doc.vertical;
                self.dirty = true;
                self.relayout();
                self.status = if self.doc.vertical {
                    let caveat = if self.doc.tables().next().is_some() {
                        "表は初版では縦になりません。"
                    } else {
                        ""
                    };
                    ui::tf!("縦書きにしました(右の列から左へ。{}保存で docx にも入ります)", caveat)
                        .into()
                } else {
                    ui::t!("横書きに戻しました").into()
                };
            }
            // ルビ(日本語一級)。選んだ字の上に半分の大きさで読みを振る
            "ruby" => {
                self.switch_target(Target::Body);
                let sel = self.ed.selection();
                if sel.is_empty() {
                    self.status = ui::t!("ルビを振る字を選んでから押してください").into();
                    return;
                }
                self.rb_range = sel.clone();
                let cur = self.doc.char_format_at(sel).ruby.unwrap_or_default();
                self.rb_ed = Editor::new(&cur);
                self.find_open = false;
                self.hf_edit = None;
                self.cmt_edit = false;
                self.rb_open = true;
                self.status =
                    ui::t!("ルビ: 読みを打って Enter(空にして Enter で外す)").into();
            }
            // 文字の大きさ
            "incfont" => self.size(|s| s + 1.0),
            "decfont" => self.size(|s| s - 1.0),
            // 印刷・PDF。**組み直さない** — 画面と同じ紙面をそのまま写す
            "pdf" => self.save_pdf(cx),
            // 文字色。押すたびに 赤 → 青 → 黒(解除)と回す。
            // 色を選ぶ小窓はまだ無いので、**無い機能を有るように見せず**
            // 使える範囲で回す形にしてある
            // 箇条書き・段落番号。押すたびに入切する
            "markers" => self.para(|p| {
                p.list = if p.list == ListKind::Bullet { ListKind::None } else { ListKind::Bullet }
            }),
            // 複数レベルのリスト。箇条書きにして1段深く(印はレベルで変わる)。
            // 深さは Tab / Shift+Tab でも動かせる
            "multilevels" => {
                self.para(|p| {
                    if p.list == ListKind::None {
                        p.list = ListKind::Bullet;
                    } else {
                        p.indent = (p.indent + 1).min(8);
                    }
                });
                self.status =
                    ui::t!("レベル付きのリストです(Tab / Shift+Tab で深さ。印はレベルで変わる)").into();
            }
            "numbering" => self.para(|p| {
                p.list = if p.list == ListKind::Number { ListKind::None } else { ListKind::Number }
            }),
            // インデント。0〜20段に留める
            "incoffset" => self.para(|p| p.indent = (p.indent + 1).min(20)),
            "decoffset" => self.para(|p| p.indent = p.indent.saturating_sub(1)),
            // 行間。1.0 → 1.5 → 2.0 → 1.0 と回す(小窓がまだ無いので)
            // この段落の前で改ページ(押すたびに入切)
            "pagebreak" => self.para(|p| p.page_break_before = !p.page_break_before),
            // 段落の背景色。無し → 薄黄 → 薄青 → 無し、で回す
            "paracolor" => self.para(|p| {
                p.shade = match p.shade.as_deref() {
                    None => Some("FFF2CC".into()),
                    Some("FFF2CC") => Some("DEEAF6".into()),
                    _ => None,
                }
            }),
            // 段落の囲み枠(入切)
            "borders" => self.para(|p| p.boxed = !p.boxed),
            // ドロップキャップ(頭の1字を大きく。押すたびに入切)
            "dropcap" => {
                self.para(|p| p.dropcap = !p.dropcap);
                self.status =
                    ui::t!("ドロップキャップを切り替えました(docx では Word の枠になります)").into();
            }
            // 画像の挿入。段落の下に付く(選択も**別のスレッド**)。
            // 図形・グラフ・SmartArt・テキストアート・方程式も同じ道 —
            // **絵は Python で描いて画像として貼る**(SEKKEI「writer の挿入系」)。
            // 灰色で残すより、方針どおりに動くボタンにする(発注者判断)
            "insimage" | "insshape" | "inssmartart" | "inschart" | "smartpicker"
            | "instextart" | "insequation" => {
                if id != "insimage" {
                    self.status =
                        ui::t!("図は Python(matplotlib 等)で描いて貼ります(SVG なら拡大しても粗くなりません)").into();
                }
                let ask = cx.background_executor().spawn(async {
                    rfd::FileDialog::new()
                        .add_filter("画像", &["png", "jpg", "jpeg", "svg"])
                        .pick_file()
                });
                cx.spawn(async move |this, cx| {
                    let r = ask.await;
                    let _ = this.update(cx, |this, cx| {
                        if let Some(p) = r {
                            this.insert_image(&p);
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            // テキストボックス = 1×1 の表。枠の中に文字が要る様式は
            // 表で組むのが日本の事務の通り相場(SEKKEI)
            "instext" => {
                let empty = kumihan::Cellbox {
                    paragraphs: vec![kumihan::Paragraph {
                        runs: vec![kumihan::Run {
                            text: String::new(),
                            size_pt: SIZE_PT,
                            font: None,
                            fmt: Default::default(),
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                };
                self.flush_target();
                self.doc.blocks.push(kumihan::Block::Table(kumihan::Table {
                    col_mm: vec![80.0],
                    rows: vec![vec![empty]],
                }));
                self.dirty = true;
                self.relayout_keep();
                self.status =
                    ui::t!("1×1 の枠を末尾に入れました(クリックして中に書けます)").into();
            }
            // 大文字小文字。選択の英字を 全部大文字 ⇄ 全部小文字 で切り替える
            // (小文字が混ざっていれば大文字へ。1手で戻せる)
            "changecase" => {
                let sel = self.ed.selection();
                if sel.is_empty() {
                    self.status = ui::t!("変えたい文字を選んでください").into();
                } else if let Some(t) = self.ed.text().get(sel.clone()) {
                    let up = t.chars().any(|c| c.is_lowercase());
                    let new = if up { t.to_uppercase() } else { t.to_lowercase() };
                    let start = sel.start;
                    let n = new.len();
                    self.ed.insert(&new);
                    // 選択を保つ(続けてもう一度押せるように)
                    self.ed.move_to(start, false);
                    self.ed.move_to(start + n, true);
                    self.on_edited();
                }
            }
            // 空白ページの挿入 = 段落を切って、新しい段落を次の頁の頭から
            "blankpage" => {
                handler::replace(self, None, "\n");
                self.para(|p| p.page_break_before = true);
                self.status = ui::t!("ここから新しいページになります").into();
            }
            // 表の挿入。3×3 を末尾に(大きさを選ぶ小窓はまだ無い)。
            // セル編集が入っているので、挿した表はそのまま書ける
            "instable" => {
                let empty = || kumihan::Cellbox {
                    paragraphs: vec![kumihan::Paragraph {
                        runs: vec![kumihan::Run {
                            text: String::new(),
                            size_pt: SIZE_PT,
                            font: None,
                            fmt: Default::default(),
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                };
                self.flush_target();
                self.doc.blocks.push(kumihan::Block::Table(kumihan::Table {
                    col_mm: vec![],
                    rows: (0..3).map(|_| (0..3).map(|_| empty()).collect()).collect(),
                }));
                self.dirty = true;
                self.relayout_keep();
                self.status = ui::t!("3×3 の表を末尾に入れました(セルをクリックで編集)").into();
            }
            // 記号の一覧(押すと出る/消える)
            "inssymbol" => self.symbols = !self.symbols,
            // ファイルからのテキスト。カーソルの位置に差し込む(undo の1手)
            "text-from-file" => {
                let ask = cx.background_executor().spawn(async {
                    rfd::FileDialog::new()
                        .add_filter("テキスト / Word文書", &["txt", "md", "docx"])
                        .pick_file()
                });
                cx.spawn(async move |this, cx| {
                    let r = ask.await;
                    let _ = this.update(cx, |this, cx| {
                        if let Some(p) = r {
                            this.insert_text_from(&p);
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            // テキストの追加(参考資料)= この段落を目次の材料にする。
            // 押すたびに 標準 → 見出し1 → 2 → 3 → 標準 と回る
            "add-text" => {
                let sel = self.ed.selection();
                let now = match self.target {
                    Target::Body => self.doc.para_at(sel).map(|p| p.style).unwrap_or_default(),
                    Target::Cell { .. } => Default::default(),
                };
                let next = match now {
                    kumihan::ParaStyle::Heading(n) if n < 3 => n + 1,
                    kumihan::ParaStyle::Heading(_) => 0,
                    _ => 1,
                };
                self.set_para_style(next);
            }
            // 置換のパネル。開いている間、打鍵は検索欄に入る
            "replace" => {
                self.find_open = !self.find_open;
                self.find_field = 0;
                if self.find_open {
                    self.switch_target(Target::Body);
                    self.status = ui::t!("検索語を打って Enter で次へ").into();
                }
            }
            // 画面の倍率。50〜200%。紙は変わらない
            "zoom-in" => self.zoom = (self.zoom + 0.1).min(2.0),
            // 見え方だけの切り替え(文書は変わらない)
            "hidenchars" => self.show_marks = !self.show_marks,
            // 一覧パネル(フォント・大きさ)。選ぶのはパネルの中
            "fontname" => { self.font_list = !self.font_list; self.size_list = false;
                            self.style_list = false; }
            // 用紙。向き / サイズ / 余白(選ぶ小窓は無いが、回して選べる)
            "pageorient" => self.set_page(|pg| {
                std::mem::swap(&mut pg.w_mm, &mut pg.h_mm);
            }),
            "pagesize" => self.set_page(|pg| {
                // A4 → B5 → A3 → A4(向きは保つ)
                let landscape = pg.w_mm > pg.h_mm;
                let (w, h) = match (pg.w_mm.min(pg.h_mm) * 10.0) as u32 {
                    2100 => (182.0, 257.0), // → B5
                    1820 => (297.0, 420.0), // → A3
                    _ => (210.0, 297.0),    // → A4
                };
                (pg.w_mm, pg.h_mm) = if landscape { (h, w) } else { (w, h) };
            }),
            // 段組み。1 → 2 → 3 → 1 と回る(見た目も docx も追随)
            "columns" => self.set_page(|pg| {
                pg.columns = match pg.cols() {
                    1 => 2,
                    2 => 3,
                    _ => 1,
                };
            }),
            "pagemargins" => self.set_page(|pg| {
                // 標準20 → 狭い12 → 広い30 → 標準
                let next = match pg.left_mm as u32 {
                    20 => 12.0,
                    12 => 30.0,
                    _ => 20.0,
                };
                pg.left_mm = next;
                pg.right_mm = next;
                pg.top_mm = next;
                pg.bottom_mm = next;
            }),
            "fontsize" => { self.size_list = !self.size_list; self.font_list = false;
                            self.style_list = false; }
            // 段落のスタイルの一覧(標準・見出し1〜3)
            "parastyle" => { self.style_list = !self.style_list;
                             self.font_list = false; self.size_list = false; }
            // 目次。挿す・挿し直すは同じ道(Toc の印の連続を置き換える)
            "toc" | "toc-update" => self.make_toc(),
            // 図表目次も同じ作法(Tof の印)
            "tof" | "tof-update" => self.make_tof(),
            // ヘッダー・フッターの編集(パネル。開いている間、打鍵はそこへ)
            "edit-header" => self.open_hf(false),
            "edit-footer" => self.open_hf(true),
            // ページ番号・ページ数。開いているパネル(無ければフッター)の
            // カーソル位置に印を入れる
            "pagenum" | "numpages" => {
                if self.hf_edit.is_none() {
                    self.open_hf(true);
                }
                if self.hf_edit.is_some() {
                    let (mark, what) = if id == "pagenum" {
                        (kumihan::PAGE_MARK, "ページ番号")
                    } else {
                        (kumihan::PAGES_MARK, "ページ数")
                    };
                    self.hf_ed.insert(&mark.to_string());
                    self.on_edited();
                    self.status =
                        ui::tf!("{}を入れました(docx ではフィールドになります)", what).into();
                }
            }
            // 日付。**固定の文字**として入れる(開くたび変わるフィールドは、
            // 事務の書類では事故のもと — 提出日が勝手に変わる)
            "datetime" => {
                let out = std::process::Command::new("date")
                    .arg("+%Y年%-m月%-d日")
                    .output();
                match out {
                    Ok(o) if o.status.success() => {
                        let d = String::from_utf8_lossy(&o.stdout).trim().to_string();
                        if self.hf_edit.is_some() {
                            self.hf_ed.insert(&d);
                        } else {
                            self.ed.insert(&d);
                        }
                        self.on_edited();
                        self.status =
                            ui::tf!("今日の日付を入れました({}。固定の文字です)", d).into();
                    }
                    _ => self.status = ui::t!("日付が取れません(date コマンド)").into(),
                }
            }
            "ruler" => self.ruler = !self.ruler,
            // ダークモード。**紙は白いまま**(画面と紙の一致)。周りだけ暗くする
            "darkmode" => self.dark = !self.dark,
            // 変更履歴。記録中の編集は、保存で Word の w:ins / w:del になる
            "track-changes" => {
                self.flush_target();
                self.track = !self.track;
                if self.track {
                    self.track_base =
                        Some(self.doc.paragraphs().map(para_text).collect());
                    self.status =
                        ui::t!("変更履歴を記録します(保存で Word の変更履歴になります)").into();
                } else {
                    self.track_base = None;
                    self.status =
                        ui::t!("変更履歴の記録をやめました(記録していた差分は捨てました)").into();
                }
            }
            // 描画。ペン・蛍光ペン・消しゴム(もう一度押すか Esc で戻る)。
            // 筆は文書に入り、docx では自由曲線の図形になる(ページに固定)
            "pen" | "highlighter" | "eraser" => {
                let t = match id { "pen" => 0u8, "highlighter" => 1, _ => 2 };
                self.tool = if self.tool == Some(t) { None } else { Some(t) };
                self.ink_cur = None;
                self.status = match self.tool {
                    Some(0) => ui::t!("ペン: 紙の上をドラッグで描く(もう一度押すか Esc で戻る)").into(),
                    Some(1) => ui::t!("蛍光ペン: ドラッグで引く(文字の下に薄く入る)").into(),
                    Some(2) => ui::t!("消しゴム: 線をなぞると1筆ずつ消える").into(),
                    _ => ui::t!("文字の編集に戻りました").into(),
                };
            }
            // 図表番号。カーソルの段落の下に「図 N」を入れる
            // (画像は段落の下に付くので、その下=図の下になる)。
            // 番号は既にある「図 n」の最大 + 1
            "caption" => {
                self.switch_target(Target::Body);
                self.flush_target();
                let mut n = 0usize;
                for p in self.doc.paragraphs() {
                    let t: String = p.runs.iter().map(|r| r.text.as_str()).collect();
                    if let Some(rest) = t.trim().strip_prefix("図 ") {
                        if let Ok(k) = rest.trim().parse::<usize>() {
                            n = n.max(k);
                        }
                    }
                }
                let label = ui::tf!("図 {}", n + 1);
                let (pi, b0) = self.cursor_para();
                let plen: usize = self
                    .doc
                    .paragraphs()
                    .nth(pi)
                    .map(|p| p.runs.iter().map(|r| r.text.len()).sum())
                    .unwrap_or(0);
                // 編集(undo の1手)と blocks を同じ形で揃える(目次と同じ作法)
                let end = b0 + plen;
                self.ed.move_to(end, false);
                self.ed.move_to(end, true);
                self.ed.insert(&format!("\n{label}"));
                let para_block_idx: Vec<usize> = self
                    .doc
                    .blocks
                    .iter()
                    .enumerate()
                    .filter(|(_, b)| matches!(b, kumihan::Block::Para(_)))
                    .map(|(i, _)| i)
                    .collect();
                let cap = kumihan::Paragraph {
                    align: Align::Center,
                    line_spacing: 1.0,
                    runs: vec![kumihan::Run {
                        text: label.clone(),
                        size_pt: SIZE_PT,
                        font: None,
                        fmt: Default::default(),
                    }],
                    ..Default::default()
                };
                self.doc.blocks.insert(para_block_idx[pi] + 1, kumihan::Block::Para(cap));
                self.dirty = true;
                self.relayout();
                self.follow_caret();
                self.status = ui::tf!("{} を入れました(中央揃えの段落)", label).into();
            }
            // 相互参照。しおり一覧から「文字」「ページ」を挿すパネル
            "crossref" => {
                self.xr_open = !self.xr_open;
                if self.xr_open {
                    self.bm_open = false;
                    self.find_open = false;
                    self.hf_edit = None;
                    self.cmt_edit = false;
                    self.wm_edit = false;
                    self.status =
                        ui::t!("相互参照: しおりを選んで「文字」か「ページ」を挿す").into();
                }
            }
            // しおり。一覧のパネル(名前を打って追加・押して移動・✕で削除)
            "bookmarks" => {
                self.bm_open = !self.bm_open;
                if self.bm_open {
                    self.find_open = false;
                    self.hf_edit = None;
                    self.cmt_edit = false;
                    self.wm_edit = false;
                    self.bm_ed = Editor::new("");
                    self.status =
                        ui::t!("しおり: 名前を打って「追加」。一覧を押すとそこへ移る").into();
                }
            }
            // 透かし。パネルで文字を打つ(空にして閉じると外れる)。
            // 文書ではヘッダーの中の VML になり、Word でも斜めの薄い字で出る
            "watermark" => {
                if self.wm_edit {
                    self.wm_edit = false;
                    return;
                }
                if self.doc.header.paragraphs.is_empty() && self.doc.header.part.is_some() {
                    self.status =
                        ui::t!("このヘッダーには表があり、透かしを差し込めません(この版の制限)").into();
                    return;
                }
                self.find_open = false;
                self.hf_edit = None;
                self.cmt_edit = false;
                self.wm_ed = Editor::new(self.doc.watermark.as_deref().unwrap_or(""));
                self.wm_edit = true;
                self.status = ui::t!("透かしを編集中(空にして閉じると外れる。Esc で閉じる)").into();
            }
            // ページの色。無し → 薄クリーム → 薄青 → 薄緑 → 無し(文書に入り、
            // 保存で残る。紙(PDF)も同じ色に塗る)
            "pagecolor" => {
                self.doc.page_color = match self.doc.page_color.as_deref() {
                    None => Some("FFF7DC".into()),
                    Some("FFF7DC") => Some("E8F1F8".into()),
                    Some("E8F1F8") => Some("EAF5EE".into()),
                    _ => None,
                };
                self.dirty = true;
                self.status = match &self.doc.page_color {
                    Some(c) => ui::tf!("ページの色: #{}", c).into(),
                    None => ui::t!("ページの色: 無し").into(),
                };
            }
            // 行番号(見え方だけ)。折り返した行も1行と数える(見た目の行)
            "line-numbers" => self.line_numbers = !self.line_numbers,
            // 欧文のハイフネーション(入切)。日本語は禁則で折るので変わらない
            "hyphenation" => {
                self.doc.hyphenate = !self.doc.hyphenate;
                self.dirty = true;
                self.relayout_keep();
                self.status = if self.doc.hyphenate {
                    ui::t!("ハイフネーション: 入(英語の語を音節で折って - を付けます)").into()
                } else {
                    ui::t!("ハイフネーション: 切").into()
                };
            }
            // コメントの印と一覧の表示(見え方だけ)
            "co-showcomment" => {
                self.show_comments = !self.show_comments;
                self.status = if self.show_comments {
                    ui::t!("コメントを表示します").into()
                } else {
                    ui::t!("コメントを隠しました(付いてはいます)").into()
                };
            }
            // カーソルの段落のコメントを外す
            "co-delcomment" => {
                self.switch_target(Target::Body);
                let (pi, _) = self.cursor_para();
                let mut removed = 0usize;
                let mut i = 0usize;
                for b in &mut self.doc.blocks {
                    if let kumihan::Block::Para(p) = b {
                        if i == pi {
                            removed = p.comments.len();
                            p.comments.clear();
                            break;
                        }
                        i += 1;
                    }
                }
                if removed > 0 {
                    self.dirty = true;
                    self.status =
                        ui::tf!("この段落のコメントを外しました({} 件)", removed).into();
                } else {
                    self.status = ui::t!("この段落にコメントはありません").into();
                }
            }
            // コメント(段落単位)。カーソルの段落に付ける
            "co-addcomment" | "comment" => {
                if self.cmt_edit {
                    self.cmt_edit = false;
                    return;
                }
                self.switch_target(Target::Body);
                let (pi, _) = self.cursor_para();
                self.cmt_para = pi;
                let text = self
                    .doc
                    .paragraphs()
                    .nth(pi)
                    .and_then(|p| p.comments.first())
                    .map(|c| c.text.clone())
                    .unwrap_or_default();
                self.cmt_ed = Editor::new(&text);
                self.find_open = false;
                self.hf_edit = None;
                self.cmt_edit = true;
                self.status =
                    ui::t!("コメントを編集中(段落に付きます。空にして閉じると外れる)").into();
            }
            // 文書の保護。readOnly を docx の documentProtection と往復する。
            // パスワードは掛けない(**掛けた振りもしない**)— Word でも
            // 「編集の制限」として見え、解除も同じ1手でできる正直な保護
            "prot-doc" => {
                if self.doc.protection.is_some() {
                    self.doc.protection = None;
                    self.dirty = true;
                    self.status =
                        ui::t!("保護を外しました(編集できます。保存で docx にも残ります)").into();
                } else {
                    self.flush_target();
                    self.doc.protection = Some("readOnly".into());
                    // 文書を変えるパネルとペンは店じまい
                    self.hf_edit = None;
                    self.wm_edit = false;
                    self.cmt_edit = false;
                    self.tool = None;
                    self.dirty = true;
                    self.status = ui::t!("読み取り専用で保護しました(同じボタンで解除。\
                                   パスワードは掛けません — 掛けた振りもしません)").into();
                }
            }
            // 共同編集モード。実体はファイルの錠(.~lock)による早い者勝ちの
            // 編集権。押すと錠の今を確かめ、先客が去っていれば編集権を取り直す
            "coauth-mode" => match self.path.clone() {
                None => {
                    self.status =
                        ui::t!("まだファイルになっていません(保存すると編集権=錠を取ります)").into();
                }
                Some(p) => {
                    if self.my_lock.is_some() {
                        self.status = ui::tf!("編集権はこちら({})にあります。同じ文書は先に開いた人が書け、\
                             後の人は読むだけになります(錠は .~lock ファイル)", lock_identity())
                        .into();
                    } else {
                        self.acquire_lock(&p);
                        self.status = match &self.locked_by {
                            Some(who) => ui::tf!("{} が編集中です(読めますが上書き保存はできません。\
                                 相手が閉じたら、またこのボタンで確かめてください)", who)
                            .into(),
                            None => ui::t!("先客が居なくなっていたので、編集権を取り直しました").into(),
                        };
                    }
                }
            },
            // バージョン履歴。上書き保存のたびに .jo-history へ残る控えの一覧
            "co-history" => {
                self.hist_open = !self.hist_open;
                if self.hist_open {
                    self.chat_open = false;
                    self.bm_open = false;
                    self.xr_open = false;
                    self.status = if self.path.is_none() {
                        ui::t!("まだファイルになっていません(保存すると、上書きのたびに\
                         控えが残ります)").into()
                    } else {
                        ui::t!("バージョン履歴: 押すと控えを名無しの複製で開きます").into()
                    };
                }
            }
            // チャット。文書の隣の申し送り帳(.chat.txt)へ名乗り付きで追記。
            // サーバーは無いので生放送ではない — ファイル越しの言伝(ことづて)
            "co-chat" => {
                self.chat_open = !self.chat_open;
                if self.chat_open {
                    self.hist_open = false;
                    self.bm_open = false;
                    self.xr_open = false;
                    self.find_open = false;
                    self.chat_ed = Editor::new("");
                    self.status =
                        ui::t!("チャット: 打って Enter で書き残す(文書の隣の .chat.txt)").into();
                }
            }
            // マクロ。.py を選ぶとサンドボックスの中の Python が文書の複製を直す
            "plug-macros" => {
                let ask = cx.background_executor().spawn(async {
                    rfd::FileDialog::new().add_filter("Python", &["py"]).pick_file()
                });
                cx.spawn(async move |this, cx| {
                    let r = ask.await;
                    let _ = this.update(cx, |this, cx| {
                        if let Some(p) = r {
                            this.run_macro_file(p, cx);
                        }
                        cx.notify();
                    });
                })
                .detach();
                self.status = ui::t!("マクロ: .py を選ぶと、サンドボックスの中の Python が文書の複製を\
                               直します(台本の d が python-docx の文書。\
                               fill(名前, 値)=記入・extract(名前)=読む・\
                               fields()=一覧・render(辞書)=雛形差し込み)").into();
            }
            // プラグインの管理。置き場の .py を一覧し、マクロと同じサンドボックスで実行
            "plug-manage" => {
                self.plug_open = !self.plug_open;
                if self.plug_open {
                    self.hist_open = false;
                    self.chat_open = false;
                    self.bm_open = false;
                    self.xr_open = false;
                    self.status = ui::tf!("プラグイン: {} に .py を置くと、ここに並びます", plugins_dir().display())
                    .into();
                }
            }
            // 暗号化。パスワードを決めると、保存で ECMA-376 Standard
            // (AES-128)の複合ファイルに包む。空 Enter で解除
            "prot-encrypt" => {
                if self.pw_open {
                    self.pw_open = false;
                    return;
                }
                self.pw_pending = None;
                self.pw_open = true;
                self.pw_ed = Editor::new("");
                self.status = if self.encrypt_pw.is_some() {
                    ui::t!("暗号化は入っています。新しいパスワードを打って Enter\
                    (空のまま Enter で暗号化をやめる)").into()
                } else {
                    ui::t!("暗号化: パスワードを打って Enter(AES-256。次の保存から)").into()
                };
            }
            // デジタル署名。**隣の .sig への添え書き**(Ed25519)。
            // Word の署名欄には出ない独自方式 — そう言って出す。
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
                        self.status = ui::tf!("読めません: {}", e).into();
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
                        let vk: [u8; 32] =
                            unhex(&field("pubkey:")?)?.try_into().ok()?;
                        let sg: [u8; 64] = unhex(&field("sig:")?)?.try_into().ok()?;
                        let vk = ed25519_dalek::VerifyingKey::from_bytes(&vk).ok()?;
                        let sig = ed25519_dalek::Signature::from_bytes(&sg);
                        Some((signer, vk.verify(&bytes, &sig).is_ok()))
                    })();
                    if let Some((signer, true)) = ok {
                        self.status = ui::tf!("署名は有効です — {} が署名した時のままの中身です", signer)
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
                                self.status = ui::tf!("署名しました — 隣の {} に添え書き(独自方式。\
                                     Word の署名欄には出ません。もう一度押すと検めます)", sp.file_name().unwrap_or_default().to_string_lossy())
                                .into();
                            }
                            Err(e) => {
                                self.status = ui::tf!("署名が置けません: {}", e).into()
                            }
                        }
                    }
                    Err(e) => self.status = ui::tf!("署名できません: {}", e).into(),
                }
            }
            // クリップボード(リボンから。Ctrl+C/X/V と同じ実体)
            "copy" | "cut" => {
                let e = self.editor_ref();
                let sel = e.selection();
                if sel.is_empty() {
                    self.status = ui::t!("選択がありません").into();
                } else if let Some(t) = e.text().get(sel).map(str::to_string) {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(t));
                    if id == "cut" {
                        self.editor().insert("");
                        self.on_edited();
                        self.status = ui::t!("切り取りました").into();
                    } else {
                        self.status = ui::t!("コピーしました").into();
                    }
                }
            }
            "paste" => match cx.read_from_clipboard().and_then(|i| i.text()) {
                Some(text) if !text.is_empty() => handler::replace(self, None, &text),
                _ => self.status = ui::t!("貼り付けるものがありません").into(),
            },
            // 記入欄(コンテンツコントロール)。フォームタブの実体でもある
            "controls" | "form-text" | "form-image" | "form-email" | "form-phone"
            | "form-complex" | "form-signature" | "form-date" => {
                use kumihan::SdtKind as K;
                let kind = match id {
                    "form-image" => K::Picture,
                    "form-email" => K::Email,
                    "form-phone" => K::Phone,
                    "form-complex" => K::Complex,
                    "form-signature" => K::Signature,
                    "form-date" => K::Date,
                    _ => K::Text,
                };
                self.insert_sdt(kind, Vec::new());
            }
            // チェックの欄。**同じボタンで入切**(欄の中にカーソルがあるとき)
            "form-checkbox" | "form-radio" => {
                if self.toggle_checkbox() {
                    return;
                }
                self.insert_sdt(kumihan::SdtKind::Checkbox, Vec::new());
            }
            // 選ばせる欄。選択肢をパネルで聞いてから挿す
            "form-combo" | "form-dropdown" => {
                // 既にその欄にいるなら、選択肢を順に回す(選び直し)
                if let Some(sd) = self.sdt_at() {
                    if !sd.items.is_empty() {
                        let text = self.ed.text().to_string();
                        let (pi, _) = self.cursor_para();
                        let _ = pi;
                        // いまの中身の次の選択肢へ
                        if let Some(cur) =
                            sd.items.iter().position(|it| text.contains(it.as_str()))
                        {
                            let now = &sd.items[cur];
                            let next = &sd.items[(cur + 1) % sd.items.len()];
                            if let Some(at) = text.find(now.as_str()) {
                                self.ed.move_to(at, false);
                                self.ed.move_to(at + now.len(), true);
                                self.ed.insert(next);
                                self.on_edited();
                                self.status =
                                    ui::tf!("「{}」を選びました", next).into();
                                return;
                            }
                        }
                    }
                }
                self.sd_kind = if id == "form-combo" {
                    kumihan::SdtKind::Combo
                } else {
                    kumihan::SdtKind::Dropdown
                };
                self.sd_ed = Editor::new("");
                self.sd_open = true;
                self.status =
                    ui::t!("選択肢をカンマ区切りで打って Enter(例: 赤,青,黄)").into();
            }
            // 記入欄に名前を付ける(docx の w:alias / w:tag)。
            // 名前がフォームの背骨 — マクロは fill(名前, 値) でこの鍵を引く
            "form-name" => {
                self.switch_target(Target::Body);
                let Some(sd) = self.sdt_at() else {
                    self.status =
                        ui::t!("名前を付ける記入欄の中にカーソルを置いてください").into();
                    return;
                };
                // いまの名前をパネルに前置き(種類の既定名のままなら空)
                let now = if sd.tag == sd.kind.as_tag() {
                    String::new()
                } else {
                    sd.tag.clone()
                };
                let mut ed = Editor::new(&now);
                ed.move_to(now.len(), false);
                self.sd_ed = ed;
                self.sd_naming = true;
                self.sd_open = true;
                self.status =
                    ui::t!("記入欄の名前を打って Enter(例: 氏名。Esc で取りやめ)").into();
            }
            // 配色。**その時の値で塗る**(テーマ部品は作らない — Word で
            // 開いても同じ色に見える正直な形)。見出しの色と紙の色を組で当てる
            "colorschemas" => {
                // (名前, 見出しの色, 紙の色)
                const THEMES: &[(&str, &str, Option<&str>)] = &[
                    ("標準", "1B1B1B", None),
                    ("藍", "165E83", None),
                    ("緑", "1B6E3C", None),
                    ("臙脂", "8E3A46", None),
                    ("藍(生成りの紙)", "165E83", Some("FBF7EE")),
                    ("墨(灰の紙)", "2E3338", Some("F2F2F0")),
                ];
                self.flush_target();
                self.doc_undo = Some(self.doc.clone());
                self.theme = (self.theme + 1) % THEMES.len();
                let (name, head, paper) = THEMES[self.theme];
                // 見出しの段落の字に色を当てる(段落ごとの範囲で塗る)
                let mut at = 0usize;
                let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
                for p in self.doc.paragraphs() {
                    let len: usize = p.runs.iter().map(|r| r.text.len()).sum();
                    if matches!(p.style, kumihan::ParaStyle::Heading(_)) && len > 0 {
                        ranges.push(at..at + len);
                    }
                    at += len + 1;
                }
                let n = ranges.len();
                for r in ranges {
                    let c = head.to_string();
                    self.doc.apply_char_format(r, move |f| {
                        f.color = (c != "1B1B1B").then(|| c.clone())
                    });
                }
                self.doc.page_color = paper.map(str::to_string);
                self.dirty = true;
                self.relayout_keep();
                self.status = ui::tf!("配色「{}」にしました(見出し {} 箇所と紙の色。\
                     Ctrl+Z で1手で戻せます)", name, n)
                .into();
            }
            // ---- AI(モデルに任せる変換と生成の道具箱)----
            // 宛先は人が選ぶ。押すたびに 手元 → Claude(定額)→ Claude(API)
            "ai-where" => {
                let now = ui::ai::backend();
                let next = now.next();
                ui::ai::set_backend(next);
                let ok = ui::ai::ready(next);
                self.status = match ok {
                    Ok(_) => ui::tf!("AI の宛先: {}(覚えました)", next.label()).into(),
                    Err(e) => ui::tf!("AI の宛先: {} — ただし今は使えません: {}", next.label(), e)
                    .into(),
                };
            }
            "ai-summary" => self.ai_go(AiJob::Summary, cx),
            "ai-rewrite" => self.ai_go(
                AiJob::Rewrite(
                    "あなたは日本語の文章を整える道具です。意味を変えず、\
                     読みやすく簡潔に書き直します。本文だけを返します。",
                    "次の文章を、意味を変えずに読みやすく書き直してください。",
                ),
                cx,
            ),
            "ai-polite" => self.ai_go(
                AiJob::Rewrite(
                    "あなたは日本語の文章を整える道具です。内容を変えずに、\
                     仕事の文書にふさわしい丁寧な言い方(です・ます)へ直します。\
                     本文だけを返します。",
                    "次の文章を、内容を変えずに丁寧な言い方へ直してください。",
                ),
                cx,
            ),
            "ai-plain" => self.ai_go(
                AiJob::Rewrite(
                    "あなたは日本語の文章をやさしくする道具です。難しい言葉を\
                     やさしい言葉に置き換え、一文を短くします。内容は変えません。\
                     本文だけを返します。",
                    "次の文章を、内容を変えずにやさしい日本語へ直してください。",
                ),
                cx,
            ),
            "ai-translate" => self.ai_go(AiJob::Translate, cx),
            "ai-furigana" => self.ai_go(AiJob::Furigana, cx),
            "ai-continue" => self.ai_go(AiJob::Continue, cx),
            "ai-table" => self.ai_go(AiJob::Table, cx),
            "ai-ask" => {
                if self.ai_open {
                    self.ai_open = false;
                    self.ai_macro = false;
                    return;
                }
                self.ai_ed = Editor::new("");
                self.ai_open = true;
                self.ai_macro = false;
                self.find_open = false;
                self.status = ui::tf!("AI({})に頼む: 用件を打って Enter(選んだ字があれば一緒に渡します)", ui::ai::backend().label())
                .into();
            }
            // マクロ台本を AI に書かせる。答えは文書に入れず、プラグイン
            // 置き場に .py で置く — 人が読んで確かめてから実行する
            "ai-macro" => {
                if self.ai_open {
                    self.ai_open = false;
                    self.ai_macro = false;
                    return;
                }
                self.ai_ed = Editor::new("");
                self.ai_open = true;
                self.ai_macro = true;
                self.find_open = false;
                self.status = ui::tf!("AI({})にマクロ台本を頼む: 用件を打って Enter\
                     (台本はプラグイン置き場に置くだけで、自動では走らせません)", ui::ai::backend().label())
                .into();
            }
            // 表示(本家の表示タブ)。見え方だけを変える — 文書は変わらない
            "nav" => {
                self.nav_open = !self.nav_open;
                self.status = if self.nav_open {
                    ui::t!("ナビゲーション: 見出しを押すとそこへ飛びます").into()
                } else {
                    "".into()
                };
            }
            "multipage" => {
                if self.doc.vertical {
                    self.status = ui::t!("縦書きでは見開きにしません(初版の約束)").into();
                    return;
                }
                self.multipage = !self.multipage;
                self.relayout();
                self.status = if self.multipage {
                    ui::t!("見開き(2ページ並べ)にしました。印刷は1ページずつです").into()
                } else {
                    ui::t!("1ページずつの表示に戻しました").into()
                };
            }
            "fit-page" => self.fit_zoom(false),
            "fit-width" => self.fit_zoom(true),
            "zoom100" => {
                self.zoom = 1.0;
                self.status = ui::t!("100% に戻しました").into();
            }
            "show-toolbar" => {
                self.show_toolbar = !self.show_toolbar;
                self.status = if self.show_toolbar {
                    ui::t!("ツールバーを常に表示します").into()
                } else {
                    ui::t!("ツールバーを畳みました(タブを押すと出ます)").into()
                };
            }
            "show-statusbar" => self.show_statusbar = !self.show_statusbar,
            "show-left" => self.nav_open = !self.nav_open,
            "show-right" => {
                self.rp_open = !self.rp_open;
                self.status = if self.rp_open {
                    ui::t!("右パネル: いる場所の設定を直せます").into()
                } else {
                    "".into()
                };
            }
            "zoom-out" => self.zoom = (self.zoom - 0.1).max(0.5),
            "linespace" => self.para(|p| {
                p.line_spacing = match p.spacing() {
                    s if s < 1.25 => 1.5,
                    s if s < 1.75 => 2.0,
                    _ => 1.0,
                }
            }),
            // 文字カウント。日本語は「単語数」に意味が無いので**文字数**を出す
            "wordcount" => {
                let text = self.ed.text();
                let all = text.chars().filter(|c| *c != '\n').count();
                let ink = text.chars().filter(|c| !c.is_whitespace()).count();
                let paras = text.split('\n').filter(|s| !s.trim().is_empty()).count();
                self.status = ui::tf!("文字数 {}(空白込み {})/ 段落 {}", ink, all, paras).into();
            }
            "fontcolor" => {
                let next = match self.doc.char_format_at(self.ed.selection()).color.as_deref() {
                    None => Some("C00000".to_string()),
                    Some("C00000") => Some("1F4E79".to_string()),
                    _ => None,
                };
                self.toggle(move |f| f.color = next.clone());
            }
            other => {
                // ここに来たら結線漏れ。黙らず画面に出す
                self.status = ui::tf!("未配線のコマンド: {}(不具合です)", other).into();
            }
        }
    }

    // ---- 割り当てられた操作 ----
    /// メニューの項目を実行する。
    pub(crate) fn menu_action(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.menu_at = None;
        match id {
            "cut" => self.cut(&ui::Cut, window, cx),
            "copy" => self.copy(&ui::Copy, window, cx),
            "paste" => self.paste(&ui::Paste, window, cx),
            "selword" => self.select_word(),
            "selline" => self.select_line(),
            "selall" => self.ed.select_all(),
            other => self.run_cmd(other, cx),
        }
        cx.notify();
    }
}

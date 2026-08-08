//! writer の試験(main.rs から純移動 2026-08-08。分割の1歩目 —
//! calc と同じ作法: 純移動だけ、挙動と文言は一切変えない)。
//! 中の `use crate::*` は移動先でも同じ意味になるよう `use crate::*` に直した

#[cfg(test)]
mod cell_edit_tests {
    use crate::*;

    fn doc_with_table() -> Document {
        let cell = |s: &str| kumihan::Cellbox {
            paragraphs: vec![kumihan::Paragraph {
                runs: vec![kumihan::Run {
                    text: s.into(), size_pt: SIZE_PT, font: None, fmt: Default::default() }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut d = Document::plain("本文", SIZE_PT);
        d.blocks.push(kumihan::Block::Table(kumihan::Table {
            col_mm: vec![],
            rows: vec![vec![cell("品名"), cell("金額")]],
        }));
        d
    }

    #[test]
    fn セルの文章を読み書きできる() {
        let d = doc_with_table();
        let t = d.tables().next().unwrap();
        assert_eq!(cell_text(&t.rows[0][0]), "品名");
        let mut c = t.rows[0][0].clone();
        set_cell_text(&mut c, "型式\n数量");
        assert_eq!(c.paragraphs.len(), 2, "段落に割れていない");
        assert_eq!(cell_text(&c), "型式\n数量");
    }

    #[test]
    fn セルの書式は書き戻しで残る() {
        let d = doc_with_table();
        let mut c = d.tables().next().unwrap().rows[0][0].clone();
        c.paragraphs[0].align = kumihan::Align::Center;
        c.paragraphs[0].runs[0].fmt.bold = true;
        set_cell_text(&mut c, "直した");
        assert_eq!(c.paragraphs[0].align, kumihan::Align::Center, "揃えが消えた");
        assert!(c.paragraphs[0].runs[0].fmt.bold, "太字が消えた");
    }
}

#[cfg(test)]
mod find_tests {
    use crate::*;

    fn w(text: &str) -> (Editor, Editor, Editor) {
        (Editor::new(text), Editor::new(""), Editor::new(""))
    }

    // find_next/replace の中身はエディタ操作の列なので、
    // ここでは検索の規則(後ろから・一周する)だけを関数で確かめる
    fn next_hit(text: &str, term: &str, from: usize) -> Option<usize> {
        text[from..].find(term).map(|i| from + i).or_else(|| text.find(term))
    }

    #[test]
    fn カーソルの後ろから探す() {
        let t = "誤りを直す。誤りは残さない。";
        let first = next_hit(t, "誤り", 0).unwrap();
        let second = next_hit(t, "誤り", first + "誤り".len()).unwrap();
        assert!(second > first);
    }

    #[test]
    fn 末尾まで無ければ頭から一周() {
        let t = "誤りを直す。";
        // 「直」の後ろ(文字境界)から探す。実物の from はカーソル位置なので常に境界
        let from = "誤りを直".len();
        let hit = next_hit(t, "誤り", from);
        assert_eq!(hit, Some(0), "一周していない");
    }

    #[test]
    fn 無ければ無いと言える() {
        assert_eq!(next_hit("本文", "存在しない", 0), None);
        let _ = w("x");
    }
}

/// **メニューのボタンを全部おして、落ちないか・繋がっているかを見る。**
/// リボンに ready で並ぶものは、ここで実際に run_cmd を通す
/// (ダイアログを開くものだけは、開いた窓が閉じられないので外す)。
/// GUI は起こさない — gpui の試験用の場で Writer を作って叩く
#[cfg(test)]
mod menu_run_tests {
    use crate::*;

    /// ファイル選択の窓を開くボタン。**試験では押さない** —
    /// rfd は実際に窓を出しに行くので、画面の無い試験では返ってこない
    /// (踏んで確かめた。実機での確認に回す)
    const DIALOG: &[&str] = &[
        "open", "save", "pdf", "plug-macros", "insimage", "text-from-file",
        "insshape", "inssmartart", "inschart", "smartpicker", "instextart",
        "insequation",
    ];

    /// AI の宛先は共有の設定(~/.config/office/ai.txt)。触る試験が並走すると
    /// 保存と復元が交錯して稀に落ちるので、同時には走らせない
    static AI_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[gpui::test]
    fn 全部のボタンが落ちずに通る(cx: &mut gpui::TestAppContext) {
        let _ai = AI_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // AI の宛先は覚える設定なので、試験で変えたら戻す
        let keep_ai = ui::ai::backend();
        let w = cx.update(|cx| cx.new(|cx| Writer::new(None, cx)));
        for tab in ui::ribbon::WRITER {
            for cmd in tab.cmds {
                if !cmd.ready || DIALOG.contains(&cmd.id) {
                    continue;
                }
                let id = cmd.id;
                let label = cmd.label;
                w.update(cx, |this, cx| {
                    // 本文が空だと何も起きないボタンがあるので、毎回中身を入れておく
                    if this.ed.text().is_empty() {
                        this.set_doc(Document::plain("見出し\n本文の字。", SIZE_PT));
                    }
                    this.ed.select_all();
                    this.run_cmd(id, cx);
                    let st = this.status.to_string();
                    assert!(
                        !st.contains("未配線"),
                        "「{label}」({id}) が未配線: {st}"
                    );
                });
            }
        }
        ui::ai::set_backend(keep_ai);
    }

    /// 押すと入切するボタンは、2回押すと元に戻る(1手で戻せる方針)
    #[gpui::test]
    fn 入切のボタンは二度おすと戻る(cx: &mut gpui::TestAppContext) {
        let w = cx.update(|cx| cx.new(|cx| Writer::new(None, cx)));
        for id in [
            "ruler", "darkmode", "hidenchars", "line-numbers", "nav",
            "show-statusbar", "show-right", "co-showcomment", "direction",
            "multipage", "prot-doc",
        ] {
            w.update(cx, |this, cx| {
                let before = this.toggled(id);
                this.run_cmd(id, cx);
                let mid = this.toggled(id);
                assert_ne!(before, mid, "「{id}」を押しても変わらない");
                this.run_cmd(id, cx);
                assert_eq!(before, this.toggled(id), "「{id}」が元に戻らない");
            });
        }
    }

    /// **見本の文書を開いた状態でも**全部のボタンが通る。
    /// 空の文書と違い、表・見出し・記入欄・縦書きが入っているので、
    /// 「前提があるときの道」も通る(sample/writer が検査の材料)
    #[gpui::test]
    fn 見本を開いても全部のボタンが通る(cx: &mut gpui::TestAppContext) {
        let dir = std::path::Path::new("../sample/writer");
        let dir = if dir.exists() {
            dir.to_path_buf()
        } else {
            std::path::Path::new("sample/writer").to_path_buf()
        };
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return; // 見本が無い環境では黙って飛ばす(失敗にはしない)
        };
        let mut files: Vec<std::path::PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("docx"))
            .collect();
        files.sort();
        assert!(!files.is_empty(), "見本が無い: {}", dir.display());
        let _ai = AI_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let keep_ai = ui::ai::backend();
        let w = cx.update(|cx| cx.new(|cx| Writer::new(None, cx)));
        for f in files {
            w.update(cx, |this, _| this.open(f.clone()));
            for tab in ui::ribbon::WRITER {
                for cmd in tab.cmds {
                    if !cmd.ready || DIALOG.contains(&cmd.id) {
                        continue;
                    }
                    let (id, label) = (cmd.id, cmd.label);
                    let name = f.file_name().unwrap().to_string_lossy().to_string();
                    w.update(cx, |this, cx| {
                        this.run_cmd(id, cx);
                        let st = this.status.to_string();
                        assert!(
                            !st.contains("未配線"),
                            "{name} で「{label}」({id}) が未配線: {st}"
                        );
                    });
                }
            }
        }
        ui::ai::set_backend(keep_ai);
    }

    /// **押した結果が本当に文書に出るか。** status だけ見ても
    /// 「押せるのに何も起きない」は捕まらないので、モデルを見る
    #[gpui::test]
    fn 主なボタンは文書を実際に変える(cx: &mut gpui::TestAppContext) {
        let w = cx.update(|cx| cx.new(|cx| Writer::new(None, cx)));
        let fresh = |this: &mut Writer| {
            this.set_doc(Document::plain("あいうえお\nかきくけこ", SIZE_PT));
            this.ed.move_to(0, false);
            this.ed.move_to(15, true); // 1段落目を選ぶ
        };
        // 文字書式
        for (id, get) in [
            ("bold", (|f: &kumihan::CharFormat| f.bold) as fn(&kumihan::CharFormat) -> bool),
            ("italic", |f| f.italic),
            ("underline", |f| f.underline),
            ("strikeout", |f| f.strike),
            ("superscript", |f| f.superscript),
        ] {
            w.update(cx, |this, cx| {
                fresh(this);
                this.run_cmd(id, cx);
                let f = this.doc.char_format_at(0..15);
                assert!(get(&f), "「{id}」が字に効いていない");
            });
        }
        // 段落の性質
        w.update(cx, |this, cx| {
            fresh(this);
            this.run_cmd("align-center", cx);
            assert_eq!(this.doc.paragraphs().next().unwrap().align, Align::Center);
            this.run_cmd("align-dist", cx);
            assert_eq!(this.doc.paragraphs().next().unwrap().align, Align::Distribute);
            this.run_cmd("markers", cx);
            assert_eq!(this.doc.paragraphs().next().unwrap().list, ListKind::Bullet);
            this.run_cmd("incoffset", cx);
            assert!(this.doc.paragraphs().next().unwrap().indent >= 1);
            this.run_cmd("paracolor", cx);
            assert!(this.doc.paragraphs().next().unwrap().shade.is_some());
            this.run_cmd("borders", cx);
            assert!(this.doc.paragraphs().next().unwrap().boxed);
            this.run_cmd("dropcap", cx);
            assert!(this.doc.paragraphs().next().unwrap().dropcap);
        });
        // 文書ぜんたい
        w.update(cx, |this, cx| {
            fresh(this);
            let n0 = this.doc.paragraphs().count();
            this.run_cmd("instable", cx);
            assert_eq!(this.doc.tables().count(), 1, "表が入らない");
            this.run_cmd("blankpage", cx);
            assert!(this.doc.paragraphs().count() > n0, "空白ページが入らない");
            this.run_cmd("watermark", cx);
            assert!(this.wm_edit, "透かしのパネルが開かない");
            this.run_cmd("watermark", cx);
            this.run_cmd("pagecolor", cx);
            assert!(this.doc.page_color.is_some(), "紙の色が変わらない");
            this.run_cmd("columns", cx);
            assert!(this.pg.cols() > 1, "段組みにならない");
            this.run_cmd("pageorient", cx);
            assert!(this.pg.w_mm > this.pg.h_mm, "向きが変わらない");
            this.run_cmd("hyphenation", cx);
            assert!(this.doc.hyphenate, "ハイフネーションが入らない");
        });
        // 見出し → 目次 → 相互参照の的(しおり)
        w.update(cx, |this, cx| {
            this.set_doc(Document::plain("章のはじめ\n本文です。", SIZE_PT));
            this.ed.move_to(0, false);
            // 見出しにするのは「テキストの追加」(parastyle は一覧のパネルを開く)
            this.run_cmd("add-text", cx);
            assert!(
                matches!(
                    this.doc.paragraphs().next().unwrap().style,
                    kumihan::ParaStyle::Heading(_)
                ),
                "見出しにならない"
            );
            this.run_cmd("parastyle", cx);
            assert!(this.style_list, "段落のスタイルの一覧が開かない");
            this.run_cmd("parastyle", cx);
            this.run_cmd("toc", cx);
            assert!(
                this.doc
                    .paragraphs()
                    .any(|p| matches!(p.style, kumihan::ParaStyle::Toc(_))),
                "目次が入らない"
            );
            this.run_cmd("caption", cx);
            assert!(
                this.doc.body_text().contains("図 "),
                "図表番号が入らない: {}",
                this.doc.body_text()
            );
        });
        // 配色(見出しの色が付く)
        w.update(cx, |this, cx| {
            this.set_doc(Document::plain("題", SIZE_PT));
            this.ed.move_to(0, false);
            this.run_cmd("add-text", cx);
            this.run_cmd("colorschemas", cx);
            let colored = this
                .doc
                .paragraphs()
                .flat_map(|p| &p.runs)
                .any(|r| r.fmt.color.is_some());
            assert!(colored, "配色で色が付かない");
        });
        // ペン(描いた筆が文書に残る)
        w.update(cx, |this, cx| {
            this.set_doc(Document::plain("紙", SIZE_PT));
            this.run_cmd("pen", cx);
            assert!(this.tool.is_some(), "ペンにならない");
            this.ink_begin(10.0, 10.0);
            this.ink_move(20.0, 20.0);
            this.ink_end();
            assert!(!this.doc.ink.is_empty(), "筆が残らない");
        });
    }

    /// AI は**宛先が使えないときに正直に断る**(黙って空にしない)。
    /// 実際にモデルへ繋ぐ試験はしない(手元に居るとは限らないので)
    #[gpui::test]
    fn aiは宛先が無ければ理由を言う(cx: &mut gpui::TestAppContext) {
        let _ai = AI_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let keep = ui::ai::backend();
        let w = cx.update(|cx| cx.new(|cx| Writer::new(None, cx)));
        ui::ai::set_backend(ui::ai::Backend::ClaudeApi);
        if ui::ai::ready(ui::ai::Backend::ClaudeApi).is_err() {
            w.update(cx, |this, cx| {
                this.set_doc(Document::plain("本文です。", SIZE_PT));
                this.run_cmd("ai-summary", cx);
                let st = this.status.to_string();
                assert!(st.starts_with("AI:"), "断りの言葉が出ない: {st}");
                assert!(!this.ai_busy, "断ったのに考え中のまま");
            });
        }
        ui::ai::set_backend(keep);
    }

    /// 記入欄(フォーム)は押した種類の欄が本当に入る
    #[gpui::test]
    fn フォームのボタンが記入欄を入れる(cx: &mut gpui::TestAppContext) {
        use kumihan::SdtKind as K;
        let w = cx.update(|cx| cx.new(|cx| Writer::new(None, cx)));
        for (id, want) in [
            ("form-text", K::Text),
            ("form-image", K::Picture),
            ("form-email", K::Email),
            ("form-phone", K::Phone),
            ("form-complex", K::Complex),
            ("form-signature", K::Signature),
            ("controls", K::Text),
        ] {
            w.update(cx, |this, cx| {
                this.set_doc(Document::plain("", SIZE_PT));
                this.run_cmd(id, cx);
                let kinds: Vec<_> = this
                    .doc
                    .paragraphs()
                    .flat_map(|p| &p.runs)
                    .filter_map(|r| r.fmt.sdt.as_ref().map(|s| s.kind))
                    .collect();
                assert!(kinds.contains(&want), "「{id}」で {want:?} が入らない: {kinds:?}");
            });
        }
    }

    /// マクロの fill(名前, 値) が名前つき記入欄に本当に書く。
    /// サンドボックスの外で同じ台本を回す(bwrap の無い試験環境でも通る)。
    /// python-docx が無い環境では黙って飛ばす
    #[test]
    fn マクロのfillが名前の記入欄に書く() {
        let py = if std::path::Path::new("../.venv/bin/python").exists() {
            std::path::PathBuf::from("../.venv/bin/python")
        } else {
            find_python()
        };
        let ok = std::process::Command::new(&py)
            .args(["-c", "import docx"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !ok {
            eprintln!("python-docx が無いので飛ばす");
            return;
        }
        // 名前「氏名」の記入欄(8..17)と、独自種類メールに名前「連絡先」を
        // 付けた欄(26..35。w:tag は jo:email:連絡先 に合成される)を持つ docx
        let mut doc = Document::plain("氏名: 未記入\n宛先: 未記入", SIZE_PT);
        doc.apply_char_format(8..17, |f| {
            f.sdt = Some(Box::new(kumihan::Sdt {
                kind: kumihan::SdtKind::Text,
                alias: "氏名".into(),
                tag: "氏名".into(),
                items: Vec::new(),
            }))
        });
        doc.apply_char_format(26..35, |f| {
            f.sdt = Some(Box::new(kumihan::Sdt {
                kind: kumihan::SdtKind::Email,
                alias: "連絡先".into(),
                tag: "連絡先".into(),
                items: Vec::new(),
            }))
        });
        let dir =
            std::env::temp_dir().join(format!("jo-fill-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let (in_d, out_d) = (dir.join("in.docx"), dir.join("out.docx"));
        let f = std::fs::File::create(&in_d).unwrap();
        ooxml::write_with(&doc, None::<std::io::Cursor<Vec<u8>>>, std::io::BufWriter::new(f))
            .unwrap();
        let py_path = dir.join("run.py");
        let script = macro_script(
            &in_d,
            &out_d,
            "fill(\"氏名\", \"山田太郎\")\nfill(\"連絡先\", \"y@example.jp\")",
        );
        std::fs::write(&py_path, script).unwrap();
        let o = std::process::Command::new(&py).arg(&py_path).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let (doc2, _) =
            ooxml::read(std::io::Cursor::new(std::fs::read(&out_d).unwrap())).unwrap();
        let body: String = doc2
            .paragraphs()
            .flat_map(|p| &p.runs)
            .map(|r| r.text.as_str())
            .collect();
        assert!(body.contains("山田太郎"), "記入されていない: {body}");
        assert!(body.contains("y@example.jp"), "合成 tag の欄に書けない: {body}");
        // 記入しても欄(w:tag)は生きている — もう一度 fill できる
        let tags: Vec<_> = doc2
            .paragraphs()
            .flat_map(|p| &p.runs)
            .filter_map(|r| r.fmt.sdt.as_ref().map(|s| s.tag.clone()))
            .collect();
        assert!(tags.contains(&"氏名".to_string()), "欄が消えた: {tags:?}");
        // 無い名前は黙って空振りせず、ことばで断る
        let script = macro_script(&in_d, &out_d, "fill(\"住所\", \"x\")");
        std::fs::write(&py_path, script).unwrap();
        let o = std::process::Command::new(&py).arg(&py_path).output().unwrap();
        assert!(!o.status.success(), "無い名前で通ってしまう");
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(err.contains("住所"), "断りに名前が出ない: {err}");
        // 吸い上げ(入口): 記入済みから extract で値を読み、fields で一覧
        let script = macro_script(
            &out_d,
            &dir.join("out2.docx"),
            "print(extract(\"氏名\"))\nfor n, v in fields():\n    print(n, v)",
        );
        std::fs::write(&py_path, script).unwrap();
        let o = std::process::Command::new(&py).arg(&py_path).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let out = String::from_utf8_lossy(&o.stdout).to_string();
        assert!(out.contains("山田太郎"), "extract が読めない: {out}");
        assert!(
            out.contains("連絡先 y@example.jp"),
            "fields に合成 tag の名前が出ない: {out}"
        );
        // extract も無い名前は断る
        let script = macro_script(&out_d, &dir.join("out3.docx"), "extract(\"住所\")");
        std::fs::write(&py_path, script).unwrap();
        let o = std::process::Command::new(&py).arg(&py_path).output().unwrap();
        assert!(!o.status.success(), "extract が無い名前で通ってしまう");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// render(辞書)= docxtpl の雛形差し込みが台本から使える。
    /// 雛形は Word の編集を模して {{担当者}} を run 分断で割っておき、
    /// writer の読み書き(heal_runs)を通してから差し込む。
    /// docxtpl が無い環境では黙って飛ばす
    #[test]
    fn 雛形のrenderが差し込む() {
        let py = if std::path::Path::new("../.venv/bin/python").exists() {
            std::path::PathBuf::from("../.venv/bin/python")
        } else {
            find_python()
        };
        let ok = std::process::Command::new(&py)
            .args(["-c", "import docx, docxtpl"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !ok {
            eprintln!("docxtpl が無いので飛ばす");
            return;
        }
        let dir =
            std::env::temp_dir().join(format!("jo-render-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let tpl = dir.join("tpl.docx");
        // 雛形: 差し込み口3つ+行くり返しの表。{{担当者}} は割っておく
        let mk = format!(
            r#"import docx
d = docx.Document()
d.add_paragraph("{{{{顧客名}}}} 様")
p = d.add_paragraph("")
p.add_run("担当: {{{{担当")
p.add_run("者}}}}")
tb = d.add_table(rows=4, cols=2)
tb.cell(0, 0).text = "品名"; tb.cell(0, 1).text = "数量"
tb.cell(1, 0).text = "{{%tr for m in 明細 %}}"
tb.cell(2, 0).text = "{{{{m.品名}}}}"; tb.cell(2, 1).text = "{{{{m.数量}}}}"
tb.cell(3, 0).text = "{{%tr endfor %}}"
d.save({tpl:?})
"#,
            tpl = tpl.to_string_lossy()
        );
        let mk_py = dir.join("mk.py");
        std::fs::write(&mk_py, mk).unwrap();
        let o = std::process::Command::new(&py).arg(&mk_py).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        // writer の読み書きを通す(分断は heal_runs が繋ぐ)
        let bytes = std::fs::read(&tpl).unwrap();
        let (tdoc, _) = ooxml::read(std::io::Cursor::new(bytes.clone())).unwrap();
        let healed = dir.join("healed.docx");
        let f = std::fs::File::create(&healed).unwrap();
        ooxml::write_with(
            &tdoc,
            Some(std::io::Cursor::new(bytes)),
            std::io::BufWriter::new(f),
        )
        .unwrap();
        // 台本: 差し込み口を報告してから差し込む
        let code = r#"print(tpl_fields())
render({"顧客名": "青森県庁", "担当者": "山田",
        "明細": [{"品名": "防火戸", "数量": 3}, {"品名": "枠", "数量": 6}]})
"#;
        let out_d = dir.join("out.docx");
        let py_path = dir.join("run.py");
        std::fs::write(&py_path, macro_script(&healed, &out_d, code)).unwrap();
        let o = std::process::Command::new(&py).arg(&py_path).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let stdout = String::from_utf8_lossy(&o.stdout).to_string();
        for name in ["顧客名", "担当者", "明細"] {
            assert!(stdout.contains(name), "tpl_fields に{name}が無い: {stdout}");
        }
        // 差し込み結果を writer 側で読んで確かめる
        let (doc2, _) =
            ooxml::read(std::io::Cursor::new(std::fs::read(&out_d).unwrap())).unwrap();
        let body = doc2.body_text();
        assert!(body.contains("青森県庁 様"), "顧客名が入らない: {body}");
        assert!(body.contains("担当: 山田"), "割れた欄に入らない: {body}");
        let cells: Vec<String> = doc2
            .tables()
            .flat_map(|t| &t.rows)
            .flatten()
            .flat_map(|c| &c.paragraphs)
            .flat_map(|p| &p.runs)
            .map(|r| r.text.clone())
            .collect();
        let joined = cells.join("|");
        assert!(
            joined.contains("防火戸") && joined.contains("枠"),
            "行くり返しが効かない: {joined}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 実物様式(機構の実施要領様式)が writer の読み書きに通り、
    /// python-docx と docxtpl がそのまま読めることを見る。
    /// 様式や道具が無い環境では黙って飛ばす(失敗にはしない)
    #[test]
    fn 実物様式が読み書きと雛形の道具に通る() {
        let src = std::path::Path::new(
            "/mnt/sdb/home/dev/ドキュメント/機構/yoryou-yoshiki",
        );
        let Ok(rd) = std::fs::read_dir(src) else { return };
        let py = if std::path::Path::new("../.venv/bin/python").exists() {
            std::path::PathBuf::from("../.venv/bin/python")
        } else {
            find_python()
        };
        let ok = std::process::Command::new(&py)
            .args(["-c", "import docx, docxtpl"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !ok {
            return;
        }
        let dir =
            std::env::temp_dir().join(format!("jo-yoshiki-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut n = 0;
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|e| e.to_str()) != Some("docx") {
                continue;
            }
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            let bytes = std::fs::read(&p).unwrap();
            let (doc, _) = ooxml::read(std::io::Cursor::new(bytes.clone()))
                .unwrap_or_else(|e| panic!("{name} が読めない: {e}"));
            let out = dir.join(&name);
            let f = std::fs::File::create(&out).unwrap();
            ooxml::write_with(
                &doc,
                Some(std::io::Cursor::new(bytes)),
                std::io::BufWriter::new(f),
            )
            .unwrap_or_else(|e| panic!("{name} が書けない: {e}"));
            // 保存し直したものを python-docx と docxtpl で開く
            let check = format!(
                "import docx\nfrom docxtpl import DocxTemplate\n\
                 docx.Document({out:?})\n\
                 print(sorted(DocxTemplate({out:?}).get_undeclared_template_variables()))",
                out = out.to_string_lossy()
            );
            let py_path = dir.join("check.py");
            std::fs::write(&py_path, check).unwrap();
            let o = std::process::Command::new(&py).arg(&py_path).output().unwrap();
            assert!(
                o.status.success(),
                "{name}: {}",
                String::from_utf8_lossy(&o.stderr)
            );
            n += 1;
        }
        eprintln!("実物様式 {n} 件が通った");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// モデルが書きがちなコードフェンスは受け側で剥がす
    #[test]
    fn コードフェンスを剥がす() {
        assert_eq!(strip_code_fence("print(1)"), "print(1)");
        assert_eq!(strip_code_fence("```python\nprint(1)\n```"), "print(1)");
        assert_eq!(strip_code_fence("```\nprint(1)\n```\n"), "print(1)");
        assert_eq!(
            strip_code_fence("```python\nfill(\"氏名\", \"x\")\nprint('done')\n```"),
            "fill(\"氏名\", \"x\")\nprint('done')"
        );
    }

    /// 「マクロを書く」はパネルを開き、宛先が無ければ正直に断る。
    /// 台本が文書に入ることは決してない(置き場に置くだけ)
    #[gpui::test]
    fn マクロを書くはパネルを開き宛先が無ければ断る(cx: &mut gpui::TestAppContext) {
        let _ai = AI_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let keep = ui::ai::backend();
        let w = cx.update(|cx| cx.new(|cx| Writer::new(None, cx)));
        w.update(cx, |this, cx| {
            this.set_doc(Document::plain("本文です。", SIZE_PT));
            this.run_cmd("ai-macro", cx);
            assert!(this.ai_open && this.ai_macro, "マクロのパネルが開かない");
            this.run_cmd("ai-macro", cx); // もう一度押すと閉じる
            assert!(!this.ai_open && !this.ai_macro, "パネルが閉じない");
        });
        ui::ai::set_backend(ui::ai::Backend::ClaudeApi);
        if ui::ai::ready(ui::ai::Backend::ClaudeApi).is_err() {
            w.update(cx, |this, cx| {
                let before = this.ed.text().to_string();
                this.ai_go(AiJob::Macro("氏名を記入して".into()), cx);
                let st = this.status.to_string();
                assert!(st.starts_with("AI:"), "断りの言葉が出ない: {st}");
                assert!(!this.ai_busy, "断ったのに考え中のまま");
                assert_eq!(this.ed.text(), before, "文書が変わってしまった");
            });
        }
        ui::ai::set_backend(keep);
    }

    /// **実物様式の一本通し**: 様式1(参加表明 — ラベル段落型・同じラベルが
    /// 3組ある)に名前つき記入欄を付け、fill で記入 → extract で吸い上げる。
    /// 様式や道具が無い環境では黙って飛ばす
    #[test]
    fn 実物様式1で名前付けから吸い上げまで通る() {
        let src = std::path::Path::new(
            "/mnt/sdb/home/dev/ドキュメント/機構/yoryou-yoshiki/実施要領様式1_参加表明.docx",
        );
        if !src.exists() {
            return;
        }
        let py = if std::path::Path::new("../.venv/bin/python").exists() {
            std::path::PathBuf::from("../.venv/bin/python")
        } else {
            find_python()
        };
        if !std::process::Command::new(&py)
            .args(["-c", "import docx"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return;
        }
        let bytes = std::fs::read(src).unwrap();
        let (mut doc, _) = ooxml::read(std::io::Cursor::new(bytes.clone())).unwrap();
        // ラベル段落の末尾に、節ごとの名前で記入欄を挿す
        // (「…印」で終わる段落は、印の前に挿す)
        let labels: &[(&str, &str)] = &[
            ("住所", "住所"),
            ("商号または名称", "商号"),
            ("代表者職氏名", "代表者"),
            ("所属", "所属"),
            ("氏名", "氏名"),
            ("電話番号", "電話"),
            ("E-mail", "メール"),
        ];
        let mut section = String::new();
        let mut kyoryoku = 0;
        let mut named = 0usize;
        for b in &mut doc.blocks {
            let kumihan::Block::Para(p) = b else { continue };
            let text: String = p.runs.iter().map(|r| r.text.as_str()).collect();
            let t = text.trim();
            if t.contains("代表事業者") {
                section = "代表".into();
                continue;
            }
            if t.contains("協力事業者") {
                kyoryoku += 1;
                section = format!("協力{kyoryoku}");
                continue;
            }
            if t.contains("担当者代表") {
                section = "担当".into();
                continue;
            }
            let Some((_, suffix)) =
                labels.iter().find(|(l, _)| t == *l || t.starts_with(*l)).copied()
            else {
                continue;
            };
            if section.is_empty() {
                continue;
            }
            let name = format!("{section}・{suffix}");
            let base = p.runs.first().cloned();
            let mut field = kumihan::Run {
                text: "　　　　　　　　".into(),
                size_pt: base.as_ref().map(|r| r.size_pt).unwrap_or(SIZE_PT),
                font: base.as_ref().and_then(|r| r.font.clone()),
                fmt: Default::default(),
            };
            field.fmt.sdt = Some(Box::new(kumihan::Sdt {
                kind: kumihan::SdtKind::Text,
                alias: name.clone(),
                tag: name,
                items: Vec::new(),
            }));
            // 「印」で終わる段落(代表者職氏名 … 印)は印の前に挿す
            let sz = field.size_pt;
            let last = p.runs.last_mut().unwrap();
            if let Some(at) = last.text.rfind('印') {
                let tail = last.text[at..].to_string();
                last.text.truncate(at);
                p.runs.push(field);
                p.runs.push(kumihan::Run {
                    text: tail,
                    size_pt: sz,
                    font: None,
                    fmt: Default::default(),
                });
            } else {
                p.runs.push(field);
            }
            named += 1;
        }
        assert_eq!(named, 13, "名前を付けた欄の数が想定と違う: {named}");
        let dir =
            std::env::temp_dir().join(format!("jo-yoshiki1-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let named_d = dir.join("named.docx");
        let f = std::fs::File::create(&named_d).unwrap();
        ooxml::write_with(
            &doc,
            Some(std::io::Cursor::new(bytes)),
            std::io::BufWriter::new(f),
        )
        .unwrap();
        // 台本: 名前で記入し、読み戻して報告する(同名ラベル3組でも誤爆しない)
        let code = r#"fill("代表・住所", "徳島県徳島市山城町東浜傍示1番地1")
fill("代表・商号", "日本フネン株式会社")
fill("担当・氏名", "山田太郎")
fill("担当・メール", "yamada@example.jp")
assert extract("代表・商号") == "日本フネン株式会社"
assert extract("協力1・住所") .strip() == ""
print(len(fields()))
"#;
        let out_d = dir.join("out.docx");
        let py_path = dir.join("run.py");
        std::fs::write(&py_path, macro_script(&named_d, &out_d, code)).unwrap();
        let o = std::process::Command::new(&py).arg(&py_path).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let stdout = String::from_utf8_lossy(&o.stdout);
        assert_eq!(stdout.trim(), "13", "fields の数が違う: {stdout}");
        // 記入済みを writer で読み直しても、値と欄が生きている
        let (doc2, _) =
            ooxml::read(std::io::Cursor::new(std::fs::read(&out_d).unwrap())).unwrap();
        let body = doc2.body_text();
        assert!(body.contains("日本フネン株式会社"), "記入が残らない");
        assert!(body.contains("山城町東浜傍示"), "住所が残らない");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 「名前」ボタンで記入欄に名前が付く(docx の w:tag。マクロの fill の鍵)
    #[gpui::test]
    fn 記入欄に名前を付けられる(cx: &mut gpui::TestAppContext) {
        let w = cx.update(|cx| cx.new(|cx| Writer::new(None, cx)));
        w.update(cx, |this, cx| {
            this.set_doc(Document::plain("", SIZE_PT));
            this.run_cmd("form-text", cx); // 欄が入りカーソルは欄の直後
            this.run_cmd("form-name", cx); // 名前のパネルが開く
            assert!(this.sd_open && this.sd_naming, "名前のパネルが開かない");
            this.sd_ed = Editor::new("氏名");
            this.sd_commit();
            let tags: Vec<_> = this
                .doc
                .paragraphs()
                .flat_map(|p| &p.runs)
                .filter_map(|r| r.fmt.sdt.as_ref().map(|s| s.tag.clone()))
                .collect();
            assert!(tags.contains(&"氏名".to_string()), "名前が付かない: {tags:?}");
            // 欄の外で押すとパネルは開かず、ことばで断る
            this.set_doc(Document::plain("ただの字", SIZE_PT));
            this.ed.move_to(0, false);
            this.run_cmd("form-name", cx);
            assert!(!this.sd_open, "欄が無いのにパネルが開く");
        });
    }
}

#[cfg(test)]
mod ruby_mark_tests {
    use crate::strip_ruby_marks;

    #[test]
    fn ルビ記法をほどいて位置と読みが出る() {
        let (plain, rubies) = strip_ruby_marks("今日は|組版《くみはん》の話。", 0);
        assert_eq!(plain, "今日は組版の話。");
        assert_eq!(rubies.len(), 1);
        let (r, yomi) = &rubies[0];
        assert_eq!(yomi, "くみはん");
        assert_eq!(&plain[r.clone()], "組版", "位置がずれている");
    }

    #[test]
    fn 全角の縦棒も受ける() {
        let (plain, rubies) = strip_ruby_marks("｜漢字《かんじ》です", 0);
        assert_eq!(plain, "漢字です");
        assert_eq!(rubies[0].1, "かんじ");
    }

    #[test]
    fn 差し込む先の頭からの位置になる() {
        let (_, rubies) = strip_ruby_marks("|漢字《かんじ》", 100);
        assert_eq!(rubies[0].0.start, 100, "base が効いていない");
    }

    #[test]
    fn 記法が壊れていても本文を落とさない() {
        for src in ["|語《よみ", "ただの|棒", "《よみ》だけ", "|"] {
            let (plain, _) = strip_ruby_marks(src, 0);
            for c in src.chars().filter(|c| !"|｜".contains(*c)) {
                assert!(plain.contains(c), "字が落ちた: {src} → {plain}");
            }
        }
    }
}

#[cfg(test)]
mod url_tests {
    use crate::resolve_url;

    #[test]
    fn 相対urlが今の場所から解ける() {
        let b = "http://ex.jp/a/b.html";
        assert_eq!(resolve_url(b, "c.html"), "http://ex.jp/a/c.html");
        assert_eq!(resolve_url(b, "/x/y"), "http://ex.jp/x/y");
        assert_eq!(resolve_url(b, "https://o.jp/z"), "https://o.jp/z");
        assert_eq!(resolve_url(b, "//o.jp/z"), "http://o.jp/z");
        assert_eq!(resolve_url("http://ex.jp", "p.html"), "http://ex.jp/p.html");
    }
}

#[cfg(test)]
mod wiring_tests {
    #[test]
    fn リボンのreadyは全部配線されている() {
        // 「押せるのに何も起きない」を仕組みで止める
        for tab in ui::ribbon::WRITER {
            for cmd in tab.cmds {
                if cmd.ready {
                    assert!(
                        crate::Writer::HANDLED.contains(&cmd.id),
                        "「{}」({}) は ready なのに run_cmd が知らない",
                        cmd.label, cmd.id
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod page_setup_tests {
    use crate::*;

    #[test]
    fn 用紙の変更が保存で残る() {
        // 画面で変えただけで docx に書かれないなら、それは飾り
        let mut d = Document::plain("本文", SIZE_PT);
        let mut pg = kumihan::PageSetup::default();
        std::mem::swap(&mut pg.w_mm, &mut pg.h_mm); // 横向き
        d.page = Some(pg);
        d.sect_raw = Some(format!(
            "<w:sectPr><w:pgSz w:w=\"{}\" w:h=\"{}\" w:orient=\"landscape\"/>\
             <w:pgMar w:top=\"1134\" w:right=\"1134\" w:bottom=\"1134\" w:left=\"1134\"/></w:sectPr>",
            (pg.w_mm * 56.6929) as i64, (pg.h_mm * 56.6929) as i64));
        let mut buf = Vec::new();
        ooxml::write(&d, std::io::Cursor::new(&mut buf)).unwrap();
        let (back, _) = ooxml::read(std::io::Cursor::new(&buf)).unwrap();
        let bp = back.page.expect("用紙が消えた");
        assert!(bp.w_mm > bp.h_mm, "横向きが消えた: {}×{}", bp.w_mm, bp.h_mm);
    }

    #[test]
    fn ヘッダーの参照は用紙を変えても残る() {
        // set_page は pgSz/pgMar だけ作り替え、他は原文から引き継ぐ
        let raw = r#"<w:sectPr><w:headerReference r:id="rId8"/><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1134" w:right="1134" w:bottom="1134" w:left="1134"/></w:sectPr>"#;
        // set_page 内の引き継ぎと同じ処理を直接なぞる
        let mut out = String::new();
        let mut skip = false;
        for part in raw.split_inclusive('>') {
            let t = part.trim_start();
            if t.starts_with("<w:sectPr") || t.starts_with("</w:sectPr") {
                continue;
            }
            if t.starts_with("<w:pgSz") || t.starts_with("<w:pgMar") {
                skip = !part.trim_end().ends_with("/>");
                continue;
            }
            if skip {
                continue;
            }
            out.push_str(part);
        }
        assert!(out.contains("headerReference"), "ヘッダーの参照が落ちた: {out}");
        assert!(!out.contains("pgSz"), "古い用紙が残った: {out}");
    }
}

#[cfg(test)]
mod lock_tests {
    use crate::*;

    #[test]
    fn ロックの置き場所と先客の判定() {
        let dir = std::env::temp_dir().join(format!("jolock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let doc = dir.join("文書.docx");
        std::fs::write(&doc, b"x").unwrap();
        let lp = lock_path_for(&doc);
        assert!(lp.file_name().unwrap().to_string_lossy().starts_with(".~lock."),
            "LibreOffice と同じ場所でない: {lp:?}");
        // 先客のロック
        std::fs::write(&lp, "花子@dev2,;").unwrap();
        assert_eq!(foreign_lock(&doc).as_deref(), Some("花子@dev2"), "先客が見えない");
        // 自分のロックは先客と見ない
        std::fs::write(&lp, format!("{},;", lock_identity())).unwrap();
        assert_eq!(foreign_lock(&doc), None, "自分を先客と見た");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod track_tests {
    use crate::*;

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn 変わった段落と増えた段落が分かる() {
        let base = v(&["一", "二", "三"]);
        let cur = v(&["一", "二を直した", "追加", "三"]);
        let (marks, deleted) = track_diff(&base, &cur);
        assert_eq!(marks[0], PMark::Same);
        assert_eq!(marks[1], PMark::Changed(1), "変わった段落が組みにならない");
        assert_eq!(marks[2], PMark::New, "増えた段落が新規にならない");
        assert_eq!(marks[3], PMark::Same);
        assert!(deleted.is_empty());
    }

    #[test]
    fn 消えた段落は次の段落の前に付く() {
        let base = v(&["一", "二", "三"]);
        let cur = v(&["一", "三"]);
        let (marks, deleted) = track_diff(&base, &cur);
        assert_eq!(marks, vec![PMark::Same, PMark::Same]);
        assert_eq!(deleted, vec![(1, 1)], "消えた段落の場所が違う");
    }

    #[test]
    fn 文字の差分は頭と尻尾を残す() {
        let (pre, del, ins, suf) = split_diff("防火戸の仕様", "防火ドアの仕様");
        assert_eq!((pre.as_str(), del.as_str(), ins.as_str(), suf.as_str()),
            ("防火", "戸", "ドア", "の仕様"));
        let (pre, del, ins, suf) = split_diff("同じ", "同じ");
        assert_eq!((pre.as_str(), del.as_str(), ins.as_str(), suf.as_str()),
            ("同じ", "", "", ""));
    }
}

#[cfg(test)]
mod word_tests {
    use crate::*;

    #[test]
    fn 英語は空白と語の境で止まる() {
        let t = "hello world  foo";
        assert_eq!(word_boundary(t, 0, true), 6, "次の語の頭に行かない");
        assert_eq!(word_boundary(t, 6, true), 13);
        assert_eq!(word_boundary(t, 13, false), 6, "前の語の頭に戻らない");
        assert_eq!(word_boundary(t, 6, false), 0);
        assert_eq!(word_boundary(t, t.len(), true), t.len(), "末尾で止まらない");
    }

    #[test]
    fn 日本語は文字種の変わり目で止まる() {
        // 漢字の連なり→ひらがな→カタカナ→英数、の境で切れる
        let t = "防火戸のカタログをPDFで";
        let b = |s: &str| t.find(s).unwrap();
        assert_eq!(word_boundary(t, 0, true), b("の"), "漢字の連なりを1語にしない");
        assert_eq!(word_boundary(t, b("の"), true), b("カタログ"));
        assert_eq!(word_boundary(t, b("カタログ"), true), b("を"),
            "カタカナの連なりが1語にならない");
        assert_eq!(word_boundary(t, b("PDF"), false), b("を"));
    }

    #[test]
    fn 端で壊れない() {
        assert_eq!(word_boundary("", 0, true), 0);
        assert_eq!(word_boundary("", 0, false), 0);
        assert_eq!(word_boundary("あ", 0, false), 0);
    }
}

#[cfg(test)]
mod image_px_tests {
    use crate::*;

    #[test]
    fn pngの画素数が読める() {
        // 署名 + IHDR(幅640, 高さ480)
        let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        b.extend_from_slice(&[0, 0, 0, 13]);
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&640u32.to_be_bytes());
        b.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(image_px(&b), Some((640, 480)));
    }

    #[test]
    fn jpegの画素数が読める() {
        // SOI + APP0(空) + SOF0(高さ300, 幅200)
        let mut b = vec![0xFF, 0xD8];
        b.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x02]); // APP0 長さ2(中身なし)
        b.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x0B, 0x08]);
        b.extend_from_slice(&300u16.to_be_bytes()); // 高さ
        b.extend_from_slice(&200u16.to_be_bytes()); // 幅
        b.extend_from_slice(&[0x03, 0x01, 0x01, 0x00]);
        assert_eq!(image_px(&b), Some((200, 300)), "SOF0 の(幅, 高さ)が読めない");
    }

    #[test]
    fn 画像でないものは断る() {
        assert_eq!(image_px(b"not an image"), None);
    }
}

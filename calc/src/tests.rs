//! calc の試験(main.rs から純移動 2026-08-06。分割の1歩目)

#[cfg(test)]
mod freeze_tests {
    use crate::*;

    #[test]
    fn 固定した行は窓が動いても頭に残る() {
        // 見出し行(0)を固定して、窓が10行目に居ても 0 行目が出る
        let rows = grid_rows(Some(Pos::new(1, 1)), Pos::new(10, 5), 5);
        assert_eq!(rows[0], 0, "固定した見出しが消えた: {rows:?}");
        assert_eq!(rows[1], 10, "続きが窓から始まっていない: {rows:?}");
        let cols = grid_cols(Some(Pos::new(1, 1)), Pos::new(10, 5), 4);
        assert_eq!(cols, vec![0, 5, 6, 7], "{cols:?}");
    }

    #[test]
    fn 固定なしなら窓のまま() {
        assert_eq!(grid_rows(None, Pos::new(3, 0), 4), vec![3, 4, 5, 6]);
    }

    #[test]
    fn 窓が固定の中に居ても重複しない() {
        // 窓が先頭にあるとき、固定行と窓の行が二重に出ない
        let rows = grid_rows(Some(Pos::new(2, 0)), Pos::new(0, 0), 5);
        let mut sorted = rows.clone();
        sorted.dedup();
        assert_eq!(rows.len(), sorted.len(), "行が二重に出た: {rows:?}");
    }
}

#[cfg(test)]
mod size_grip_tests {
    use crate::*;

    #[test]
    fn 境界の近くだけ掴める() {
        // 2列(48px, 108px)が HEAD_W から並ぶ
        let cols = [(0u32, 48.0f32), (1, 108.0)];
        let e1 = HEAD_W + 48.0; // 1本目の境界
        let e2 = e1 + 108.0; // 2本目
        assert_eq!(grip_hit(&cols, HEAD_W, e1), Some(0));
        assert_eq!(grip_hit(&cols, HEAD_W, e1 - GRIP), Some(0), "縁の手前±GRIPで掴めない");
        assert_eq!(grip_hit(&cols, HEAD_W, e1 + GRIP), Some(0));
        assert_eq!(grip_hit(&cols, HEAD_W, e2 - 1.0), Some(1), "2本目の境界が累積位置にない");
        assert_eq!(grip_hit(&cols, HEAD_W, e1 + GRIP + 1.0), None, "境界の外で掴めた");
        assert_eq!(grip_hit(&cols, HEAD_W, HEAD_W + 10.0), None, "列の中ほどで掴めた");
    }

    #[test]
    fn 幅の換算が往復する() {
        // 画面px → xlsxの字数 → 画面px が(丸め2桁でも)崩れない
        let px0 = 108.0f32;
        let w = ((px0 / PX_PER_CHW) * 100.0).round() / 100.0;
        assert!((w - 8.43).abs() < 0.01, "既定幅が 8.43 にならない: {w}");
        assert!((w * PX_PER_CHW - px0).abs() < 0.5, "幅の往復がずれる");
        // 行: 画面px → pt → 画面px。既定 24px = 15pt
        let pt = (24.0f32 * 15.0 / 24.0 * 100.0).round() / 100.0;
        assert_eq!(pt, 15.0);
        assert_eq!(pt * 24.0 / 15.0, 24.0);
    }
}

#[cfg(test)]
mod validation_tests {
    use crate::*;

    #[gpui::test]
    fn 板から整数の規則を掛けて堰き止める(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            // B2:B4 に 1〜100 の整数(本家の形の板: 設定タブで組む)
            this.anchor = Some(Pos::parse("B2").unwrap());
            this.cursor = Pos::parse("B4").unwrap();
            this.run_cmd("data-validation", cx);
            {
                let d = this.dv_dlg.as_mut().expect("入力規則の板が開かない");
                d.kind = 1; // 整数
                d.op = 0; // 次の値の間
                d.eds[0] = Editor::new("1");
                d.eds[1] = Editor::new("100");
                // エラー警告タブ: 警告にして通して言うだけ
                d.err_style = 1;
                d.eds[5] = Editor::new("大きすぎます");
                // メッセージを入力タブ
                d.eds[2] = Editor::new("数量");
                d.eds[3] = Editor::new("1〜100 で");
            }
            this.dv_ok(cx);
            assert!(this.dv_dlg.is_none(), "OK で板が閉じない");
            let v = &this.sheet().validations[0];
            assert_eq!((v.kind.as_str(), v.op.as_str()), ("whole", "between"));
            assert_eq!((v.formula.as_str(), v.formula2.as_str()), ("1", "100"));
            // 警告なので、範囲の外も通して言うだけ
            this.anchor = None;
            this.cursor = Pos::parse("B2").unwrap();
            this.sync_input();
            this.input.insert("200");
            assert!(this.commit(), "警告なのに堰き止めた");
            assert!(this.status.contains("通しました"), "{}", this.status);
            // エラーを「停止」に直すと堰き止める
            this.anchor = Some(Pos::parse("B2").unwrap());
            this.cursor = Pos::parse("B4").unwrap();
            this.run_cmd("data-validation", cx);
            {
                let d = this.dv_dlg.as_mut().unwrap();
                assert_eq!(d.kind, 1, "既存の規則が板に読み込まれない");
                assert_eq!(d.eds[0].text(), "1");
                d.err_style = 0; // 停止
            }
            this.dv_ok(cx);
            this.anchor = None;
            this.cursor = Pos::parse("B3").unwrap();
            this.sync_input();
            this.input.insert("999");
            assert!(!this.commit(), "999 が 1〜100 を通った");
            assert!(this.status.contains("入力規則"), "{}", this.status);
            // 範囲の中は入る
            this.input.select_all();
            this.input.insert("50");
            assert!(this.commit());
            // 入力メッセージはセルに乗ると状態行に出る
            this.cursor = Pos::parse("B4").unwrap();
            this.sync_input();
            assert!(this.status.contains("数量"), "{}", this.status);
        });
    }

    #[gpui::test]
    fn 空白を無視を外すと空も堰き止める(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            let b2 = Pos::parse("B2").unwrap();
            this.cursor = b2;
            this.sync_input();
            this.input.insert("5");
            assert!(this.commit());
            this.run_cmd("data-validation", cx);
            {
                let d = this.dv_dlg.as_mut().unwrap();
                d.kind = 1;
                d.op = 0;
                d.eds[0] = Editor::new("1");
                d.eds[1] = Editor::new("100");
                d.allow_blank = false;
            }
            this.dv_ok(cx);
            assert!(!this.sheet().validations[0].allow_blank);
            // 空にするのも堰き止められる
            this.sync_input();
            this.input.select_all();
            this.input.insert("");
            assert!(!this.commit(), "空白を無視を外したのに空が通った");
        });
    }

    #[gpui::test]
    fn 読めない種類の規則は板で壊れない(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            // 日付の規則(判定できない種類)が既にある
            let b2 = Pos::parse("B2").unwrap();
            let mut v = sheet::model::Validation::list((b2, b2), "40000".into());
            v.kind = "date".into();
            v.op = "greaterThan".into();
            this.book.sheets[this.active].validations.push(v);
            this.cursor = b2;
            this.anchor = None;
            this.run_cmd("data-validation", cx);
            {
                let d = this.dv_dlg.as_mut().unwrap();
                assert_eq!(d.kind, 5, "読めない種類は「このまま保持」で開く");
                // 文言だけ足す
                d.eds[3] = Editor::new("日付を入れてください");
            }
            this.dv_ok(cx);
            let v = &this.sheet().validations[0];
            assert_eq!(v.kind, "date", "日付の規則が壊れた");
            assert_eq!(v.op, "greaterThan");
            assert_eq!(v.formula, "40000");
            assert_eq!(v.input_msg.as_ref().unwrap().1, "日付を入れてください");
        });
    }
}

mod numfmt_tests {
    use crate::*;

    #[gpui::test]
    fn 数値の書式は一覧とコード直打ちで掛かる(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            let a1 = Pos::parse("A1").unwrap();
            this.cursor = a1;
            this.sync_input();
            this.input.insert("1234.5");
            assert!(this.commit());
            // 一覧から: パーセント
            this.run_cmd("format", cx);
            assert_eq!(this.pick_kind, "numfmt-pick");
            this.apply_pick("パーセント (12.34%)", cx);
            assert_eq!(
                this.sheet().get(a1).unwrap().fmt.number_format.as_deref(),
                Some("0.00%")
            );
            // 開き直すと今の書式に ✓ が付き、状態行にも出る(本家のコンボの追従の代わり)
            this.run_cmd("format", cx);
            {
                let (items, _) = this.pick.as_ref().expect("一覧が開かない");
                assert!(
                    items.iter().any(|i| i == "✓ パーセント (12.34%)"),
                    "今の書式に印が付かない: {items:?}"
                );
                assert!(this.status.contains("今の書式"), "{}", this.status);
            }
            // ✓ 付きのまま選び直しても効く(印は値ではない)
            this.apply_pick("✓ パーセント (12.34%)", cx);
            assert_eq!(
                this.sheet().get(a1).unwrap().fmt.number_format.as_deref(),
                Some("0.00%")
            );
            // その他 → コード直打ち(今のコードが下敷きに入る)
            this.run_cmd("format", cx);
            this.apply_pick("その他(書式コードを打つ)…", cx);
            let (kind, ed) = this.prompt.as_ref().expect("コードの板が開かない");
            assert_eq!(*kind, "numfmt-custom");
            assert_eq!(ed.text(), "0.00%", "今のコードが下敷きにならない");
            this.prompt = Some(("numfmt-custom", Editor::new("#,##0.0")));
            this.finish_prompt(cx);
            assert_eq!(
                this.sheet().get(a1).unwrap().fmt.number_format.as_deref(),
                Some("#,##0.0")
            );
            // 一般に戻す
            this.run_cmd("format", cx);
            this.apply_pick("一般", cx);
            assert_eq!(this.sheet().get(a1).unwrap().fmt.number_format, None);
        });
    }
}

mod sort_tests {
    use crate::*;

    #[gpui::test]
    fn 選択の横にデータが続くときは拡張するか聞く(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            // A=名前, B=数(隣り合った2列の表)
            for (a1, v) in [
                ("A1", "c"), ("B1", "3"),
                ("A2", "a"), ("B2", "1"),
                ("A3", "b"), ("B3", "2"),
            ] {
                this.cursor = Pos::parse(a1).unwrap();
                this.sync_input();
                this.input.insert(v);
                assert!(this.commit());
            }
            // A列だけ選んで昇順 → 横(B列)にデータが続くので聞かれる
            this.anchor = Some(Pos::parse("A1").unwrap());
            this.cursor = Pos::parse("A3").unwrap();
            this.sync_input();
            this.run_cmd("sort-asc", cx);
            assert_eq!(this.pick_kind, "sort-expand", "拡張の確認が出ない");
            let get = |this: &Calc, p: &str| {
                this.sheet().get(Pos::parse(p).unwrap()).map(|c| c.editable()).unwrap_or_default()
            };
            assert_eq!(get(this, "A1"), "c", "聞く前に並べ替えられた");
            // 「選択した範囲だけ」→ A列だけ並び、B列はそのまま(ずれる)
            this.apply_pick("選択した範囲だけ並べ替え(横の列とはずれます)", cx);
            assert_eq!(
                (get(this, "A1"), get(this, "A2"), get(this, "A3")),
                ("a".into(), "b".into(), "c".into())
            );
            assert_eq!(
                (get(this, "B1"), get(this, "B2"), get(this, "B3")),
                ("3".into(), "1".into(), "2".into()),
                "選択の外まで動いた"
            );
            // 「拡張して」→ 表全体が行ごと動く(1行目は見出しとして据え置き、
            // 残りが A の降順。B が行ごと付いてくる)
            this.run_cmd("sort-desc", cx);
            assert_eq!(this.pick_kind, "sort-expand");
            this.apply_pick("拡張して並べ替え(続きの列も一緒に動く)", cx);
            assert_eq!(get(this, "A2"), "c");
            assert_eq!(get(this, "B2"), "2", "拡張なのに行が付いてこない");
            // 横に何も無い離れ小島は、聞かずに選択だけを並べ替える
            for (a1, v) in [("E1", "2"), ("E2", "1")] {
                this.cursor = Pos::parse(a1).unwrap();
                this.sync_input();
                this.input.insert(v);
                assert!(this.commit());
            }
            this.anchor = Some(Pos::parse("E1").unwrap());
            this.cursor = Pos::parse("E2").unwrap();
            this.sync_input();
            this.run_cmd("sort-asc", cx);
            assert_eq!(get(this, "E1"), "1", "島の並べ替えが効かない");
            assert_eq!(this.pick_kind, "value", "島なのに聞いた");
        });
    }

    #[gpui::test]
    fn 複数の基準で並べ替える(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            for (a1, v) in [
                ("A1", "区分"), ("B1", "数"),
                ("A2", "甲"), ("B2", "1"),
                ("A3", "乙"), ("B3", "2"),
                ("A4", "甲"), ("B4", "3"),
                ("A5", "丙"), ("B5", "4"),
            ] {
                this.cursor = Pos::parse(a1).unwrap();
                this.sync_input();
                this.input.select_all();
                this.input.insert(v);
                assert!(this.commit());
            }
            let col_a = |this: &Calc| -> Vec<String> {
                (1..5)
                    .map(|r| this.sheet().value(Pos::new(r, 0)).display())
                    .collect()
            };
            // 見出し名で2基準: 区分 降順 → 同じ区分の中は 数 降順
            this.prompt = Some(("sort-by", Editor::new("区分 降順, 数 降順")));
            this.finish_prompt(cx);
            assert_eq!(col_a(this), ["甲", "甲", "乙", "丙"], "1つ目の基準が効かない");
            assert_eq!(
                this.sheet().value(Pos::parse("B2").unwrap()),
                sheet::Value::Number(3.0),
                "2つ目の基準(数 降順)が効かない"
            );
            // 列の字でも指せる(B 昇順)
            this.prompt = Some(("sort-by", Editor::new("B")));
            this.finish_prompt(cx);
            assert_eq!(col_a(this), ["甲", "乙", "甲", "丙"], "列の字の基準が効かない");
            // 知らない見出しは板を開いたまま言い返す
            this.prompt = Some(("sort-by", Editor::new("存在しない列")));
            this.finish_prompt(cx);
            assert!(this.prompt.is_some(), "打ち直せるように板が残るはず");
            assert!(this.status.contains("見つかりません"), "{}", this.status);
        });
    }
}

mod filter_tests {
    use crate::*;

    fn seed(this: &mut Calc) {
        for (a1, v) in [
            ("A1", "区分"), ("B1", "数"),
            ("A2", "甲"), ("B2", "1"),
            ("A3", "乙"), ("B3", "2"),
            ("A4", "甲"), ("B4", "3"),
            ("A5", "丙"), ("B5", "4"),
        ] {
            this.cursor = Pos::parse(a1).unwrap();
            this.sync_input();
            this.input.select_all();
            this.input.insert(v);
            assert!(this.commit());
        }
        this.anchor = None;
        this.cursor = Pos::parse("A1").unwrap();
        this.sync_input();
    }

    #[gpui::test]
    fn 値の入切で行が隠れて数も件数も追随する(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            seed(this);
            this.run_cmd("setfilter", cx); // 表全体 A1:B5 に範囲を張る
            let f = this.auto_filter.as_ref().expect("範囲が張られない");
            assert_eq!(f.range, (Pos::parse("A1").unwrap(), Pos::parse("B5").unwrap()));
            // 板の一覧: A列の値と件数(BTreeMap の並び=文字順)
            let (vals, cut) = this.filter_values(0);
            assert!(!cut);
            assert_eq!(
                vals,
                vec![("丙".into(), 1), ("乙".into(), 1), ("甲".into(), 2)],
                "値の一覧が違う"
            );
            // 乙と丙を隠す → 見出し+甲の2行が残る
            this.filter_toggle_value(0, "乙");
            this.filter_toggle_value(0, "丙");
            assert!(this.filter_active());
            assert_eq!(this.filter_counts(), Some((4, 2)), "行の数が違う");
            assert_eq!(this.visible_rows(), vec![0, 1, 3], "見える行が違う");
            // 他の列の一覧は絞り込みを効かせたまま: B列は甲の行の値だけ
            let (bv, _) = this.filter_values(1);
            assert_eq!(bv, vec![("1".into(), 1), ("3".into(), 1)]);
            // 入切で戻る(空になったら列ごと素通し)
            this.filter_toggle_value(0, "乙");
            this.filter_toggle_value(0, "丙");
            assert!(!this.filter_active(), "全部見せたのに絞られている");
            // (すべて選択)を切る → 全部隠れる → もう一度で全部戻る
            let all: Vec<String> =
                this.filter_values(0).0.into_iter().map(|(v, _)| v).collect();
            this.filter_toggle_all(0, all.clone());
            assert_eq!(this.filter_counts(), Some((4, 0)));
            this.filter_toggle_all(0, all);
            assert!(!this.filter_active());
            // もう一度 setfilter で範囲ごと外れる
            this.run_cmd("setfilter", cx);
            assert!(this.auto_filter.is_none(), "トグルで外れない");
        });
    }

    #[gpui::test]
    fn 絞り込みは生きた値にも効く(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            seed(this);
            this.run_cmd("setfilter", cx);
            this.filter_toggle_value(0, "乙");
            this.filter_toggle_value(0, "丙");
            // B2:B5 を選ぶと、見えている甲の行(1と3)だけ数える
            this.anchor = Some(Pos::parse("B2").unwrap());
            this.cursor = Pos::parse("B5").unwrap();
            let s = this.sel_stats().expect("生きた値が出ない");
            assert!(s.contains("合計 4"), "隠れた行を数えている: {s}");
            assert!(s.contains("個数 2"), "個数が違う: {s}");
        });
    }
}

#[cfg(test)]
mod sheet_name_tests {
    use crate::*;

    #[test]
    fn 足すシートの名前がぶつからない() {
        let mut b = Book::new(); // Sheet1
        assert_eq!(unique_sheet_name(&b), "Sheet2");
        b.sheets.push(sheet::Sheet::new("Sheet2"));
        b.sheets.push(sheet::Sheet::new("Sheet3"));
        assert_eq!(unique_sheet_name(&b), "Sheet4");
        // 歯抜け(途中の名前が消えた等)でも重複しない
        b.sheets.remove(1);
        let n = unique_sheet_name(&b);
        assert!(!b.sheets.iter().any(|s| s.name == n), "重複した: {n}");
    }
}

#[cfg(test)]
mod clipboard_tests {
    use crate::*;

    fn table() -> sheet::Sheet {
        let mut s = sheet::Sheet { name: "表".into(), ..Default::default() };
        s.set(Pos::new(0, 0), Cell::input("品名"));
        s.set(Pos::new(0, 1), Cell::input("金額"));
        s.set(Pos::new(1, 0), Cell::input("甲"));
        s.set(Pos::new(1, 1), Cell::input("=A2&\"円\""));
        s
    }

    #[test]
    fn コピーはtsvで式が残る() {
        let s = table();
        let tsv = range_tsv(&s, Pos::new(0, 0), Pos::new(1, 1));
        assert_eq!(tsv, "品名\t金額\n甲\t=A2&\"円\"", "TSV の形が違う: {tsv:?}");
    }

    #[test]
    fn 空セルは空欄として出る() {
        let s = table();
        let tsv = range_tsv(&s, Pos::new(0, 0), Pos::new(2, 1));
        assert!(tsv.ends_with("\n\t"), "空行の形が違う: {tsv:?}");
    }

    #[test]
    fn アプリ内の貼り付けは式がずれる() {
        let mut s = table();
        // B2 の式(=A2&"円")を B4 へ: 2行下 → =A4&"円"
        let grid = vec![vec!["=A2&\"円\"".to_string()]];
        paste_grid(&mut s, Pos::new(3, 1), &grid, Some((2, 0)));
        assert_eq!(
            s.get(Pos::new(3, 1)).and_then(|c| c.formula.clone()).as_deref(),
            Some("A4&\"円\""),
            "式の参照がずれていない"
        );
    }

    #[test]
    fn 外から来たtsvは式をずらさない() {
        let mut s = sheet::Sheet { name: "表".into(), ..Default::default() };
        let grid = tsv_grid("甲\t100\r\n乙\t=A1*2\n");
        let n = paste_grid(&mut s, Pos::new(0, 0), &grid, None);
        assert_eq!(n, 4);
        assert_eq!(s.value(Pos::new(0, 1)), Value::Number(100.0));
        assert_eq!(
            s.get(Pos::new(1, 1)).and_then(|c| c.formula.clone()).as_deref(),
            Some("A1*2"),
            "外来の式を勝手にずらした"
        );
    }

    #[test]
    fn 貼り付けても書式は据え置き() {
        // 帳票の枠(罫線)の上に値を貼っても枠が残る
        let mut s = sheet::Sheet { name: "枠".into(), ..Default::default() };
        s.set(Pos::new(0, 0), Cell {
            formula: None,
            value: Value::Empty,
            fmt: CellFormat { borders: Borders::ALL, ..Default::default() },
        });
        paste_grid(&mut s, Pos::new(0, 0), &[vec!["100".to_string()]], None);
        let c = s.get(Pos::new(0, 0)).unwrap();
        assert_eq!(c.value, Value::Number(100.0));
        assert_eq!(c.fmt.borders, Borders::ALL, "貼り付けで罫線が消えた");
    }

    #[test]
    fn 値だけの貼り付けで式が値になる() {
        let mut s = table();
        recalc(&mut s);
        // B2(=A2&"円")を控えて、値だけを B4 へ
        let cells = vec![vec![s.get(Pos::new(1, 1)).cloned()]];
        paste_values_cells(&mut s, Pos::new(3, 1), &cells);
        let c = s.get(Pos::new(3, 1)).unwrap();
        assert!(c.formula.is_none(), "式が残っている");
        assert_eq!(c.value, Value::Text("甲円".into()), "計算結果の値になっていない");
    }

    #[test]
    fn 外来の式もどきは文字として貼る() {
        let mut s = sheet::Sheet { name: "表".into(), ..Default::default() };
        paste_values_text(&mut s, Pos::new(0, 0), &[vec!["=A1*2".to_string()]]);
        let c = s.get(Pos::new(0, 0)).unwrap();
        assert!(c.formula.is_none(), "外の式を黙って式にした");
        assert_eq!(c.value, Value::Text("=A1*2".into()));
    }

    #[test]
    fn 書式だけの貼り付けで中身は残る() {
        let mut s = sheet::Sheet { name: "枠".into(), ..Default::default() };
        s.set(Pos::new(0, 0), Cell::input("100"));
        let src = Some(Cell {
            formula: None,
            value: Value::Empty,
            fmt: CellFormat { borders: Borders::ALL, ..Default::default() },
        });
        paste_formats(&mut s, Pos::new(0, 0), &[vec![src]]);
        let c = s.get(Pos::new(0, 0)).unwrap();
        assert_eq!(c.value, Value::Number(100.0), "書式だけのはずが中身が消えた");
        assert_eq!(c.fmt.borders, Borders::ALL, "書式が写っていない");
    }

    #[test]
    fn 転置で行と列が入れ替わる() {
        let g = vec![
            vec!["a".to_string(), "b".into(), "c".into()],
            vec!["1".to_string(), "2".into()],
        ];
        let t = transpose(&g);
        assert_eq!(t.len(), 3, "列の数が行にならない");
        assert_eq!(t[0], vec!["a".to_string(), "1".into()]);
        assert_eq!(t[2], vec!["c".to_string(), "".into()], "歯抜けが埋まらない");
    }

    #[test]
    fn 改行コードと末尾改行を受け流す() {
        assert_eq!(tsv_grid("a\tb\r\nc\td\r\n"),
                   vec![vec!["a".to_string(), "b".into()], vec!["c".into(), "d".into()]]);
        assert_eq!(tsv_grid("1"), vec![vec!["1".to_string()]]);
    }
}

#[cfg(test)]
mod table_design_tests {
    use crate::*;

    #[test]
    fn 合計行は見出しを外して数の列だけ足す() {
        let mut s = sheet::Sheet { name: "表".into(), ..Default::default() };
        s.set(Pos::new(0, 0), Cell::input("品名"));
        s.set(Pos::new(0, 1), Cell::input("金額"));
        s.set(Pos::new(1, 0), Cell::input("甲"));
        s.set(Pos::new(1, 1), Cell::input("100"));
        s.set(Pos::new(2, 0), Cell::input("乙"));
        s.set(Pos::new(2, 1), Cell::input("50"));
        add_total_row(&mut s, Pos::new(0, 0), Pos::new(2, 1));
        recalc(&mut s);
        let label = s.get(Pos::new(3, 0)).unwrap();
        assert_eq!(label.value.display(), "合計", "文字の列の先頭は札");
        assert!(label.fmt.bold && label.fmt.borders.top, "合計行の書式が付かない");
        let sum = s.get(Pos::new(3, 1)).unwrap();
        assert_eq!(
            sum.formula.as_deref(),
            Some("SUM(B2:B3)"),
            "見出しが合計に混ざった: {:?}",
            sum.formula
        );
        assert_eq!(sum.value.display(), "150");
    }

    #[test]
    fn 見出しの無い表は全行を合計する() {
        let mut s = sheet::Sheet { name: "表".into(), ..Default::default() };
        for (r, v) in [(0, "10"), (1, "20")] {
            s.set(Pos::new(r, 0), Cell::input(v));
        }
        add_total_row(&mut s, Pos::new(0, 0), Pos::new(1, 0));
        recalc(&mut s);
        let sum = s.get(Pos::new(2, 0)).unwrap();
        assert_eq!(sum.formula.as_deref(), Some("SUM(A1:A2)"));
        assert_eq!(sum.value.display(), "30");
    }
}

#[cfg(test)]
mod subtotal_tests {
    use crate::*;

    #[test]
    fn 小計と総計が入り明細だけ畳まれる() {
        let mut s = sheet::Sheet { name: "表".into(), ..Default::default() };
        for (r, row) in [
            ["部署", "月", "金額"],
            ["営業", "1月", "100"],
            ["営業", "1月", "50"],
            ["営業", "2月", "70"],
            ["総務", "1月", "30"],
        ]
        .iter()
        .enumerate()
        {
            for (c, v) in row.iter().enumerate() {
                s.set(Pos::new(r as u32, c as u32), Cell::input(v));
            }
        }
        let n = apply_subtotals(&mut s, Pos::new(0, 0), Pos::new(4, 2), 0, &[2]);
        recalc(&mut s);
        assert_eq!(n, 2, "区切りの数が違う");
        // 並び: 1見出し 2-4営業明細 5営業小計 6総務明細 7総務小計 8総計
        let d = |r: u32, c: u32| s.get(Pos::new(r, c)).map(|x| x.value.display()).unwrap_or_default();
        assert_eq!(d(4, 0), "営業 小計");
        assert_eq!(d(4, 2), "220", "営業の小計が違う");
        assert_eq!(
            s.get(Pos::new(4, 2)).and_then(|c| c.formula.clone()).as_deref(),
            Some("SUM(C2:C4)"),
            "小計が式でない"
        );
        assert_eq!(d(6, 0), "総務 小計");
        assert_eq!(d(6, 2), "30");
        assert_eq!(d(7, 0), "総計");
        assert_eq!(d(7, 2), "250", "総計が違う");
        // 明細だけグループ化(小計・総計はされない → 畳んでも残る)
        for r in [1, 2, 3, 5] {
            assert_eq!(s.row_outline.get(&r), Some(&1), "明細 {r} が畳めない");
        }
        for r in [0, 4, 6, 7] {
            assert!(!s.row_outline.contains_key(&r), "行 {r} まで畳まれてしまう");
        }
    }

    #[test]
    fn 行の挿抜でグループ化が付いてくる() {
        let mut s = sheet::Sheet { name: "表".into(), ..Default::default() };
        s.row_outline.insert(5, 1);
        s.row_hidden.insert(5);
        s.insert_row(2);
        assert_eq!(s.row_outline.get(&6), Some(&1), "挿入で深さが置き去り");
        assert!(s.row_hidden.contains(&6), "挿入で畳みが置き去り");
        s.remove_row(0);
        assert_eq!(s.row_outline.get(&5), Some(&1), "削除で深さが置き去り");
        assert!(s.row_hidden.contains(&5));
    }
}

#[cfg(test)]
mod solver_tests {
    use crate::*;

    #[test]
    fn セルと範囲の列挙が読める() {
        let v = parse_cell_list("B2:B4", 64).unwrap();
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], Pos::new(1, 1));
        let v = parse_cell_list("$A$1, C3", 64).unwrap();
        assert_eq!(v, vec![Pos::new(0, 0), Pos::new(2, 2)]);
        assert!(parse_cell_list("ほげ", 64).is_none(), "読めないものは None");
        assert!(parse_cell_list("A1:Z99", 10).is_none(), "上限を超えたら None");
        assert!(parse_cell_list("", 64).is_none());
    }

    #[test]
    fn 台本が実際にscipyで回る() {
        // .venv が無い機械では黙って飛ぶ(HIKITSUGI の作法)
        let py = ["../.venv/bin/python", ".venv/bin/python"]
            .iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.exists());
        let Some(py) = py else { return };
        // max x+2y  s.t. x+y<=4, x<=2, x,y>=0 → x=0,y=4(目的8)
        let dir = std::env::temp_dir().join(format!("jo-solver-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let spec = "{\"c\":[-1,-2],\"aub\":[[1,1],[1,0]],\"bub\":[4,2],\"aeq\":[],\"beq\":[],\"nonneg\":true}";
        let json_path = dir.join("solver.json");
        let py_path = dir.join("solver.py");
        std::fs::write(&json_path, spec).unwrap();
        std::fs::write(&py_path, SOLVER_PY).unwrap();
        let o = std::process::Command::new(&py)
            .arg(&py_path)
            .arg(&json_path)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let out = String::from_utf8_lossy(&o.stdout).to_string();
        let xs: Vec<f64> =
            out.split('\u{1f}').filter_map(|v| v.trim().parse().ok()).collect();
        assert_eq!(xs.len(), 2, "答えの形が違う: {out:?}");
        assert!(xs[0].abs() < 1e-6 && (xs[1] - 4.0).abs() < 1e-6,
                "最適解が違う: {xs:?}");
    }
}

#[cfg(test)]
mod equation_tests {
    use crate::*;

    #[test]
    fn 台本が実際にmathtextで清書する() {
        // .venv が無い機械では黙って飛ぶ(HIKITSUGI の作法)
        let py = ["../.venv/bin/python", ".venv/bin/python"]
            .iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.exists());
        let Some(py) = py else { return };
        let dir = std::env::temp_dir().join(format!("jo-eq-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let out = dir.join("eq.png");
        let spec = format!(
            "{{\"tex\":\"\\\\frac{{a}}{{b}}+\\\\sqrt{{x^2+1}}\",\"font\":\"\",\"out\":\"{}\"}}",
            out.to_string_lossy()
        );
        let json_path = dir.join("eq.json");
        let py_path = dir.join("eq.py");
        std::fs::write(&json_path, spec).unwrap();
        std::fs::write(&py_path, EQ_PY).unwrap();
        let o = std::process::Command::new(&py)
            .arg(&py_path)
            .arg(&json_path)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let data = std::fs::read(&out).unwrap();
        assert!(data.starts_with(&[0x89, b'P', b'N', b'G']), "PNG が出ていない");
        let (w, h) = image_px(&data).expect("大きさが読めない");
        assert!(w > 40 && h > 20, "清書が小さすぎる: {w}x{h}");
        // テキストアートも同じ道(飾り文字が PNG になる)
        let ta = format!(
            "{{\"tex\":\"見積書\",\"font\":\"\",\"out\":\"{}\"}}",
            out.to_string_lossy()
        );
        std::fs::write(&json_path, ta).unwrap();
        std::fs::write(&py_path, TEXTART_PY).unwrap();
        let o = std::process::Command::new(&py)
            .arg(&py_path)
            .arg(&json_path)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let data = std::fs::read(&out).unwrap();
        assert!(data.starts_with(&[0x89, b'P', b'N', b'G']), "テキストアートが PNG でない");
        // 読めない式は黙って白紙にせず、ちゃんと失敗する(台本を式のものに戻す)
        std::fs::write(&py_path, EQ_PY).unwrap();
        let bad = format!(
            "{{\"tex\":\"\\\\frac{{a\",\"font\":\"\",\"out\":\"{}\"}}",
            out.to_string_lossy()
        );
        std::fs::write(&json_path, bad).unwrap();
        let o = std::process::Command::new(&py)
            .arg(&py_path)
            .arg(&json_path)
            .output()
            .unwrap();
        assert!(!o.status.success(), "壊れた式が通ってしまった");
    }
}

#[cfg(test)]
mod pivot_tests {
    use crate::*;

    #[test]
    fn 見出しの列挙はカンマでも読点でも空白でも() {
        assert_eq!(split_fields("部署, 月"), vec!["部署", "月"]);
        assert_eq!(split_fields("部署、月 区分"), vec!["部署", "月", "区分"]);
        assert!(split_fields("  ").is_empty());
    }

    #[gpui::test]
    fn ピボットの行列値は一覧のクリックで選ぶ(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            for (a1, v) in [
                ("A1", "区分"), ("B1", "月"), ("C1", "金額"),
                ("A2", "筆記具"), ("B2", "4月"), ("C2", "100"),
                ("A3", "紙製品"), ("B3", "5月"), ("C3", "200"),
            ] {
                this.cursor = Pos::parse(a1).unwrap();
                this.sync_input();
                this.input.insert(v);
                assert!(this.commit());
            }
            // 範囲選択なし・表の中にカーソルだけで開く(発注者指摘 2026-08-07)
            this.anchor = None;
            this.cursor = Pos::parse("B2").unwrap();
            this.sync_input();
            this.run_cmd("pivot-insert", cx);
            assert_eq!(this.pick_kind, "pivot-rows-pick", "カーソルだけで行の一覧が開かない");
            // 見出しを選ばず決定 → 言い返されて一覧のまま
            this.apply_pick("→ 決定(列の選択へ)", cx);
            assert_eq!(this.pick_kind, "pivot-rows-pick", "空のまま先へ進んだ");
            // クリックで入切(✓ 付きでもう一度押すと外れる)
            this.apply_pick("☐ 区分", cx);
            this.apply_pick("☑ 区分", cx);
            assert!(this.pivot_pend.as_ref().unwrap().rows_sel.is_empty(), "入切が効かない");
            this.apply_pick("区分", cx);
            {
                let (items, _) = this.pick.as_ref().unwrap();
                assert!(items.iter().any(|i| i == "☑ 区分"), "選んだ印が付かない: {items:?}");
            }
            this.apply_pick("→ 決定(列の選択へ)", cx);
            assert_eq!(this.pick_kind, "pivot-cols-pick");
            {
                // 行に使った見出しは列の候補に出ない
                let (items, _) = this.pick.as_ref().unwrap();
                assert!(!items.iter().any(|i| i.contains("区分")), "{items:?}");
            }
            this.apply_pick("☐ 月", cx);
            this.apply_pick("→ 決定(列は無しでもよい)", cx);
            assert_eq!(this.pick_kind, "pivot-val-pick");
            this.apply_pick("金額", cx);
            assert_eq!(this.pick_kind, "pivot-agg-pick", "集計の一覧が開かない");
            let p = this.pivot_pend.as_ref().unwrap();
            assert_eq!(p.rows_sel, vec!["区分"]);
            assert_eq!(p.cols_sel, vec!["月"]);
            assert_eq!(p.val_sel, "金額");
            // ここでは polars は回さない(集計を選ぶと insert_pivot へ)。
            // Esc でやめられることだけ確かめる
            this.pivot_pend = None;
            this.pick = None;
            this.pick_kind = "value";
        });
    }

    fn def(rows: &[&str], cols: &[&str], value: &str, agg: &str) -> sheet::model::PivotDef {
        sheet::model::PivotDef {
            sheet: "S".into(),
            src: (Pos::new(0, 0), Pos::new(1, 1)),
            rows_sel: rows.iter().map(|s| s.to_string()).collect(),
            cols_sel: cols.iter().map(|s| s.to_string()).collect(),
            value: value.into(),
            agg: agg.into(),
            totals: false,
            subtotals: false,
            blank_rows: false,
            compact: false,
            dest: Pos::new(0, 0),
            size: (0, 0),
            hide: Vec::new(),
            style: String::new(),
        }
    }

    #[test]
    fn 指図のjsonは逃がしが効く() {
        let json = pivot_spec_json(
            &["部\"署".to_string()],
            &[vec!["営\\業".to_string()]],
            &def(&["部\"署"], &[], "部\"署", "合計"),
        );
        assert!(json.contains("部\\\"署"), "二重引用符が逃げていない: {json}");
        assert!(json.contains("営\\\\業"), "バックスラッシュが逃げていない: {json}");
        assert!(json.contains("\"totals\":false"), "旗が無い: {json}");
    }

    fn run_py(spec: String) -> Option<(Vec<Vec<String>>, Vec<char>)> {
        // .venv が無い機械では黙って飛ぶ(HIKITSUGI の作法)
        let py = ["../.venv/bin/python", ".venv/bin/python"]
            .iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.exists())?;
        // 並走する試験と取り合わないよう、呼び出しごとに番号を振る
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("jo-pivot-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let json_path = dir.join(format!("pivot{n}.json"));
        let py_path = dir.join(format!("pivot{n}.py"));
        std::fs::write(&json_path, spec).unwrap();
        std::fs::write(&py_path, PIVOT_PY).unwrap();
        let o = std::process::Command::new(&py)
            .arg(&py_path)
            .arg(&json_path)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        Some(parse_pivot_grid(&String::from_utf8_lossy(&o.stdout)))
    }

    #[test]
    fn 台本が実際にpolarsで回る() {
        let headers: Vec<String> =
            ["部署", "月", "金額"].iter().map(|s| s.to_string()).collect();
        let rows: Vec<Vec<String>> = [
            ["営業", "1月", "100"],
            ["営業", "1月", "50"],
            ["総務", "1月", "30"],
            ["営業", "2月", "70"],
        ]
        .iter()
        .map(|r| r.iter().map(|s| s.to_string()).collect())
        .collect();
        // 部署×月の合計(クロス表)
        let spec = pivot_spec_json(&headers, &rows, &def(&["部署"], &["月"], "金額", "合計"));
        let Some((g, k)) = run_py(spec) else { return };
        // 1行目は Excel と同じ札(合計 / 金額 と、列に広げた見出し)
        assert_eq!(k[0], 'l');
        assert_eq!(g[0], vec!["合計 / 金額", "月", ""], "札の形が違う: {g:?}");
        assert_eq!(k[1], 'h');
        assert_eq!(g[1], vec!["部署", "1月", "2月"], "見出しの形が違う: {g:?}");
        assert_eq!(g[2], vec!["営業", "150", "70"]);
        // 無い組み合わせ: 合計は 0(空の合計)。平均などは null → 空欄になる
        assert_eq!(g[3], vec!["総務", "30", "0"]);
        // 部署ごとの個数(列に広げない)— 値の列の見出しは「個数 / 金額」
        let spec = pivot_spec_json(&headers, &rows, &def(&["部署"], &[], "金額", "個数"));
        let Some((g, _)) = run_py(spec) else { return };
        assert_eq!(g[0], vec!["部署", "個数 / 金額"]);
        assert_eq!(g[1], vec!["営業", "3"]);
        assert_eq!(g[2], vec!["総務", "1"]);
    }

    #[test]
    fn 総計と小計と空行が付く() {
        let headers: Vec<String> =
            ["部署", "係", "月", "金額"].iter().map(|s| s.to_string()).collect();
        let rows: Vec<Vec<String>> = [
            ["営業", "一", "1月", "100"],
            ["営業", "二", "1月", "50"],
            ["営業", "一", "2月", "70"],
            ["総務", "一", "1月", "30"],
        ]
        .iter()
        .map(|r| r.iter().map(|s| s.to_string()).collect())
        .collect();
        let mut d = def(&["部署", "係"], &["月"], "金額", "合計");
        d.totals = true;
        d.subtotals = true;
        d.blank_rows = true;
        let spec = pivot_spec_json(&headers, &rows, &d);
        let Some((g, k)) = run_py(spec) else { return };
        assert_eq!(g[0], vec!["合計 / 金額", "", "月", "", ""], "札: {g:?}");
        assert_eq!(g[1], vec!["部署", "係", "1月", "2月", "総計"], "見出し: {g:?}");
        assert_eq!(g[2], vec!["営業", "一", "100", "70", "170"]);
        assert_eq!(g[3], vec!["営業", "二", "50", "0", "50"]);
        assert_eq!(
            g[4],
            vec!["営業 小計", "", "150", "70", "220"],
            "小計が違う: {g:?}"
        );
        assert_eq!(k[4], 's', "小計の種別が違う");
        assert_eq!(k[5], 'b', "空行が無い");
        assert_eq!(g[7], vec!["総務 小計", "", "30", "0", "30"]);
        let last = g.last().unwrap();
        assert_eq!(last, &vec!["総計", "", "180", "70", "250"], "総計が違う: {g:?}");
        assert_eq!(*k.last().unwrap(), 't');
        // コンパクト形式: 繰り返しの見出しが空欄になる
        d.subtotals = false;
        d.blank_rows = false;
        d.totals = false;
        d.compact = true;
        let spec = pivot_spec_json(&headers, &rows, &d);
        let Some((g, _)) = run_py(spec) else { return };
        assert_eq!(g[3][0], "", "繰り返しの部署が空欄にならない: {g:?}");
        assert_eq!(g[3][1], "二");
    }
}

/// 計算方法(自動/手動)とセル内改行の試験
#[cfg(test)]
mod recalc_tests {
    use crate::*;

    #[gpui::test]
    fn 手動計算は確定で計算せずf9相当で計算する(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, _cx| {
            // A1=5 → B1==A1*2。自動のうちは確定で計算される
            this.cursor = Pos::parse("A1").unwrap();
            this.sync_input();
            this.input.insert("5");
            assert!(this.commit());
            this.cursor = Pos::parse("B1").unwrap();
            this.sync_input();
            this.input.insert("=A1*2");
            assert!(this.commit());
            assert_eq!(
                this.sheet().value(Pos::parse("B1").unwrap()),
                sheet::Value::Number(10.0),
                "自動のうちは確定で計算されるはず"
            );
            // 手動にして A1 を書き換えると、B1 は古いまま
            this.auto_calc = false;
            this.cursor = Pos::parse("A1").unwrap();
            this.sync_input();
            this.input.select_all();
            this.input.insert("7");
            assert!(this.commit());
            assert_eq!(
                this.sheet().value(Pos::parse("B1").unwrap()),
                sheet::Value::Number(10.0),
                "手動なのに確定で計算された(手動が効いていない)"
            );
            // F9 の実体(recalc_book)で計算される
            recalc_book(&mut this.book, this.active);
            assert_eq!(
                this.sheet().value(Pos::parse("B1").unwrap()),
                sheet::Value::Number(14.0),
                "F9 相当の再計算が効かない"
            );
        });
    }

    #[gpui::test]
    fn 固定はプリセットの一覧から選ぶ(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            this.cursor = Pos::parse("B2").unwrap();
            this.run_cmd("freeze", cx);
            assert_eq!(this.pick_kind, "freeze", "固定の一覧が開かない");
            this.apply_pick("最上行の固定", cx);
            assert_eq!(this.frozen, Some(Pos::new(1, 0)), "最上行が固定されない");
            this.run_cmd("freeze", cx);
            this.apply_pick("最初の列の固定", cx);
            assert_eq!(this.frozen, Some(Pos::new(0, 1)), "最初の列が固定されない");
            this.run_cmd("freeze", cx);
            this.apply_pick("いまの位置で固定(上と左が留まる)", cx);
            assert_eq!(this.frozen, Some(Pos::parse("B2").unwrap()), "いまの位置で固定されない");
            this.run_cmd("freeze", cx);
            this.apply_pick("固定の解除", cx);
            assert_eq!(this.frozen, None, "固定が解けない");
            // 影の入切(本家の「固定された枠に影を付ける」)。✓ 付きでも効く
            this.run_cmd("freeze", cx);
            this.apply_pick("固定した枠に影を付ける", cx);
            assert!(this.freeze_shadow, "影が入らない");
            this.run_cmd("freeze", cx);
            this.apply_pick("✓ 固定した枠に影を付ける", cx);
            assert!(!this.freeze_shadow, "影が切れない");
        });
    }

    #[gpui::test]
    fn 値が複数ある範囲の結合は先に聞く(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            for (p, v) in [("A1", "甲"), ("B2", "乙")] {
                this.cursor = Pos::parse(p).unwrap();
                this.sync_input();
                this.input.insert(v);
                assert!(this.commit());
            }
            // 値が2つ → 確認の一覧が開き、まだ結合されない
            this.anchor = Some(Pos::parse("A1").unwrap());
            this.cursor = Pos::parse("B2").unwrap();
            this.run_cmd("merge", cx);
            assert_eq!(this.pick_kind, "merge-confirm", "確認が出ない");
            assert!(this.sheet().merges.is_empty(), "聞く前に結合された");
            this.apply_pick("やめる", cx);
            assert!(this.sheet().merges.is_empty());
            // 結合する、を選べば結合される(値は消えない)
            this.run_cmd("merge", cx);
            this.apply_pick("結合する(見えるのは左上の値だけになります)", cx);
            assert_eq!(this.sheet().merges.len(), 1);
            assert_eq!(
                this.sheet().get(Pos::parse("B2").unwrap()).unwrap().editable(),
                "乙",
                "結合で値が消えた"
            );
            // 値が1つ以下なら聞かずに結合する
            this.anchor = Some(Pos::parse("D1").unwrap());
            this.cursor = Pos::parse("E2").unwrap();
            this.run_cmd("merge", cx);
            assert_eq!(this.sheet().merges.len(), 2, "空の範囲で余計に聞いた");
        });
    }

    #[gpui::test]
    fn ピボットの上では表を壊す操作を締める(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            let name = this.sheet().name.clone();
            this.book.pivots.push(sheet::model::PivotDef {
                sheet: name,
                src: (Pos::parse("A1").unwrap(), Pos::parse("B4").unwrap()),
                rows_sel: vec!["品名".into()],
                cols_sel: vec![],
                value: "金額".into(),
                agg: "合計".into(),
                totals: true,
                subtotals: false,
                blank_rows: false,
                compact: true,
                dest: Pos::parse("D1").unwrap(),
                size: (3, 2), // D1:E3 に置いてある体
                hide: Vec::new(),
                style: String::new(),
            });
            // ピボットに乗ると状態行が「タブで操作」と案内する
            this.cursor = Pos::parse("D2").unwrap();
            this.anchor = None;
            this.sync_input();
            assert!(this.status.contains("ピボットテーブル"), "{}", this.status);
            // レイアウトは行の見出しが1つだと効かない — 正直に言う
            this.run_cmd("pivot-layout", cx);
            assert!(this.status.contains("2つ以上"), "{}", this.status);
            // ピボットの上(D2)では結合も入力規則も断られる
            this.anchor = Some(Pos::parse("E3").unwrap());
            this.run_cmd("merge", cx);
            assert!(this.sheet().merges.is_empty(), "ピボットの上で結合できてしまう");
            assert!(this.status.contains("ピボット"), "{}", this.status);
            this.run_cmd("data-validation", cx);
            assert!(this.dv_dlg.is_none(), "ピボットの上で入力規則の板が開いた");
            // 外(A1)なら普通に通る
            this.anchor = None;
            this.cursor = Pos::parse("A1").unwrap();
            this.run_cmd("data-validation", cx);
            assert!(this.dv_dlg.is_some(), "ピボットの外まで締めている");
            this.dv_dlg = None;
        });
    }

    #[gpui::test]
    fn 画面の文字の大きさは段階で動き両端で止まる(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            let base = this.ui_scale;
            this.run_cmd("ui-bigger", cx);
            assert!(this.ui_scale > base, "大きくならない");
            for _ in 0..30 {
                this.run_cmd("ui-bigger", cx);
            }
            assert_eq!(this.ui_scale, 2.0, "上の端(200%)で止まらない");
            for _ in 0..30 {
                this.run_cmd("ui-smaller", cx);
            }
            assert_eq!(this.ui_scale, 0.8, "下の端(80%)で止まらない");
        });
    }

    #[test]
    fn 大文字小文字の5つの変え方() {
        let t = "hello WORLD こんにちは 3rd";
        assert_eq!(change_case(t, "すべて大文字"), "HELLO WORLD こんにちは 3RD");
        assert_eq!(change_case(t, "すべて小文字"), "hello world こんにちは 3rd");
        assert_eq!(change_case(t, "文の先頭だけ大文字"), "Hello world こんにちは 3rd");
        assert_eq!(change_case(t, "単語の先頭を大文字"), "Hello World こんにちは 3rd");
        assert_eq!(
            change_case(t, "大文字と小文字を入れ替え"),
            "HELLO world こんにちは 3RD"
        );
    }

    #[gpui::test]
    fn 結合すると中央に揃う(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, _cx| {
            this.cursor = Pos::parse("A1").unwrap();
            this.sync_input();
            this.input.insert("見出し");
            assert!(this.commit());
            this.anchor = Some(Pos::parse("A1").unwrap());
            this.cursor = Pos::parse("C1").unwrap();
            this.merge_selection();
            let f = &this.sheet().get(Pos::parse("A1").unwrap()).unwrap().fmt;
            assert_eq!(f.align, sheet::model::HAlign::Center, "横が中央にならない");
            assert_eq!(f.valign, sheet::model::VAlign::Middle, "縦が中央にならない");
            assert_eq!(this.sheet().merges.len(), 1, "結合が積まれていない");
        });
    }

    #[gpui::test]
    fn セル内改行の確定で折り返しが立つ(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, _cx| {
            this.cursor = Pos::parse("A1").unwrap();
            this.sync_input();
            this.input.insert("上の行\n下の行");
            assert!(this.commit());
            let cell = this.sheet().get(Pos::parse("A1").unwrap()).unwrap();
            assert!(cell.fmt.wrap, "改行入りの確定で折り返しが立たない");
            assert_eq!(
                cell.value,
                sheet::Value::Text("上の行\n下の行".into()),
                "改行が中身に残らない"
            );
        });
    }
}

/// **メニューの釦を全部おして、落ちないか・繋がっているかを見る。**
/// writer の menu_run_tests と同じ作法 — リボンに ready で並ぶものは
/// ここで実際に run_cmd を通す(ダイアログを開くものだけは外す)。
/// GUI は起こさない — gpui の試験用の場で Calc を作って叩く
#[cfg(test)]
mod menu_run_tests {
    use crate::*;

    /// ファイル選択の窓を開く釦。**試験では押さない** —
    /// rfd は実際に窓を出しに行くので、画面の無い試験では返ってこない
    /// (writer で踏んで確かめた轍。実機での確認に回す)
    const DIALOG: &[&str] = &[
        "open", "save", "pdf", "plug-macros", "insimage", "data-from-text",
        "data-external-links",
    ];

    /// 空の表だと何も起きない釦があるので、見本の小さな表を入れて選ぶ
    fn seed(this: &mut Calc) {
        if this.sheet().cells.is_empty() {
            for (a1, v) in [
                ("A1", "品名"), ("B1", "数量"), ("C1", "単価"),
                ("A2", "防火戸"), ("B2", "4"), ("C2", "125000"),
                ("A3", "点検口"), ("B3", "2"), ("C3", "8000"),
                ("D2", "=B2*C2"), ("D3", "=B3*C3"),
            ] {
                this.sheet_mut().set(Pos::parse(a1).unwrap(), Cell::input(v));
            }
            recalc(this.sheet_mut());
        }
        this.cursor = Pos::parse("A1").unwrap();
        this.anchor = Some(Pos::parse("D3").unwrap());
        // バーとセルを揃える(実機ではカーソル移動が必ず呼ぶ。ずれたままだと
        // 最初の commit() が A1 を空で潰し、種の表が崩れる)
        this.sync_input();
    }

    #[gpui::test]
    fn 全部の釦が落ちずに通る(cx: &mut gpui::TestAppContext) {
        // AI の宛先は覚える設定なので、試験で変えたら戻す
        let keep_ai = ui::ai::backend();
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        for tab in ui::ribbon::CALC {
            for cmd in tab.cmds {
                if !cmd.ready || DIALOG.contains(&cmd.id) {
                    continue;
                }
                let (id, label) = (cmd.id, cmd.label);
                c.update(cx, |this, cx| {
                    seed(this);
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

    /// リボンの「すべて選択」は**セル**に効く(バーの文字選択に化けない —
    /// Ctrl+A と同じ実体を通ることの検査。2026-08-05 に別実装のサボりを直した)
    #[gpui::test]
    fn 全選択はセルに効く(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            seed(this);
            this.anchor = None;
            this.sync_input(); // 実機ではカーソル移動が必ず呼ぶ
            this.run_cmd("selectall", cx);
            let (rows, cols) = this.sheet().extent();
            assert_eq!(this.anchor, Some(Pos::parse("A1").unwrap()), "起点が A1 でない");
            assert_eq!(
                this.cursor,
                Pos::new(rows - 1, cols - 1),
                "使われている範囲の端まで選べていない"
            );
        });
    }

    /// 押すと入切する釦は、2回押すと元に戻る(1手で戻せる家訓)
    #[gpui::test]
    fn 入切の釦は二度おすと戻る(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        let state = |this: &Calc, id: &str| -> bool {
            match id {
                "show-formulas" => this.show_formulas,
                "show-gridlines" => this.gridlines,
                "co-showcomment" => this.show_comments,
                "formula-bar" => this.show_formula_bar,
                "show-headings" => this.show_headers,
                "show-zeros" => this.show_zeros,
                "rtl-sheet" => this.sheet().rtl,
                _ => unreachable!(),
            }
        };
        for id in [
            "show-formulas", "show-gridlines", "co-showcomment", "formula-bar",
            "show-headings", "show-zeros", "rtl-sheet",
        ] {
            c.update(cx, |this, cx| {
                seed(this);
                // freeze は A1 では効かない仕様(固定する位置が要る)
                this.cursor = Pos::parse("B2").unwrap();
                this.anchor = None;
                let before = state(this, id);
                this.run_cmd(id, cx);
                assert_ne!(before, state(this, id), "「{id}」を押しても変わらない");
                this.run_cmd(id, cx);
                assert_eq!(before, state(this, id), "「{id}」が元に戻らない");
            });
        }
    }

    /// **見本のブックを開いた状態でも**全部の釦が通る。
    /// 空のブックと違い、式・結合・列幅・条件付き書式が入っているので
    /// 「前提があるときの道」も通る(sample/*.xlsx が検査の材料)。
    /// 見本は写しを開く — 署名やチャットが隣にファイルを添えるため、
    /// 追跡している見本の隣を汚さない
    #[gpui::test]
    fn 見本を開いても全部の釦が通る(cx: &mut gpui::TestAppContext) {
        let dir = std::path::Path::new("../sample");
        let dir = if dir.exists() {
            dir.to_path_buf()
        } else {
            std::path::Path::new("sample").to_path_buf()
        };
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return; // 見本が無い環境では黙って飛ばす(失敗にはしない)
        };
        let mut files: Vec<std::path::PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("xlsx"))
            .collect();
        files.sort();
        assert!(!files.is_empty(), "見本が無い: {}", dir.display());
        let work = std::env::temp_dir().join(format!("jo-menu-{}", std::process::id()));
        std::fs::create_dir_all(&work).unwrap();
        let keep_ai = ui::ai::backend();
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        for f in files {
            let copy = work.join(f.file_name().unwrap());
            std::fs::copy(&f, &copy).unwrap();
            c.update(cx, |this, _| this.open(copy.clone()));
            for tab in ui::ribbon::CALC {
                for cmd in tab.cmds {
                    if !cmd.ready || DIALOG.contains(&cmd.id) {
                        continue;
                    }
                    let (id, label) = (cmd.id, cmd.label);
                    let name = f.file_name().unwrap().to_string_lossy().to_string();
                    c.update(cx, |this, cx| {
                        this.run_cmd(id, cx);
                        let st = this.status.to_string();
                        assert!(
                            !st.contains("未配線"),
                            "{name} で「{label}」({id}) が未配線: {st}"
                        );
                    });
                }
            }
            c.update(cx, |this, _| this.release_lock());
        }
        ui::ai::set_backend(keep_ai);
        let _ = std::fs::remove_dir_all(&work);
    }
}

#[cfg(test)]
mod wiring_tests {
    #[test]
    fn リボンのreadyは全部配線されている() {
        for tab in ui::ribbon::CALC {
            for cmd in tab.cmds {
                if cmd.ready {
                    assert!(
                        crate::Calc::HANDLED.contains(&cmd.id),
                        "「{}」({}) は ready なのに run_cmd が知らない",
                        cmd.label, cmd.id
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod paper_tests {
    use crate::*;

    #[test]
    fn 用紙コードはjisのbで引く() {
        assert_eq!(paper_mm(9), Some((210.0, 297.0, "A4")));
        assert_eq!(paper_mm(12), Some((257.0, 364.0, "B4")), "B4 は JIS の紙");
        assert_eq!(paper_mm(99), None, "知らないコードを黙って A4 にしない");
    }
}

#[cfg(test)]
mod index_at_tests {
    use crate::*;

    #[test]
    fn 位置から列が引ける() {
        let cols = [(0u32, 108.0f32), (1, 54.0), (2, 108.0)];
        assert_eq!(index_at(&cols, HEAD_W, HEAD_W + 1.0), Some(0));
        assert_eq!(index_at(&cols, HEAD_W, HEAD_W + 107.9), Some(0));
        assert_eq!(index_at(&cols, HEAD_W, HEAD_W + 108.0), Some(1), "境界は次の区分");
        assert_eq!(index_at(&cols, HEAD_W, HEAD_W + 200.0), Some(2));
        assert_eq!(index_at(&cols, HEAD_W, HEAD_W + 400.0), None, "並びの外");
        assert_eq!(index_at(&cols, HEAD_W, 10.0), None, "start より手前");
    }
}

#[cfg(test)]
mod goal_seek_tests {
    use crate::*;

    #[test]
    fn 合計を目標に数量が逆算できる() {
        // 見本の表: D2=B2*C2, D4=SUM, D6=D4+D5(消費税は固定にして単純化)
        let mut s = sheet::Sheet { name: "表".into(), ..Default::default() };
        s.set(Pos::parse("B2").unwrap(), Cell::input("4"));
        s.set(Pos::parse("C2").unwrap(), Cell::input("125000"));
        s.set(Pos::parse("D2").unwrap(), Cell::input("=B2*C2"));
        recalc(&mut s);
        // D2 を 800000 にする B2 は 6.4
        let x = solve_goal(&s, Pos::parse("D2").unwrap(), 800000.0, Pos::parse("B2").unwrap())
            .expect("見つからない");
        assert!((x - 6.4).abs() < 1e-6, "6.4 のはず: {x}");
        // 効かないセルでは正直に None
        assert!(
            solve_goal(&s, Pos::parse("D2").unwrap(), 800000.0, Pos::parse("F9").unwrap())
                .is_none(),
            "効かないセルで見つかったことにした"
        );
    }
}

#[cfg(test)]
mod lock_tests {
    use crate::*;

    #[test]
    fn 先客のロックが見え_自分のは先客に数えない() {
        let dir = std::env::temp_dir().join(format!("jo-lock-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let book = dir.join("台帳.xlsx");
        std::fs::write(&book, b"x").unwrap();
        let lp = lock_path_for(&book);
        assert!(lp.file_name().unwrap().to_string_lossy().starts_with(".~lock.台帳"));
        // 誰も居ない
        assert!(foreign_lock(&book).is_none());
        // 先客
        std::fs::write(&lp, "yamada@jimusho,;").unwrap();
        assert_eq!(foreign_lock(&book).as_deref(), Some("yamada@jimusho"));
        // 自分のロックは先客ではない
        std::fs::write(&lp, format!("{},;", lock_identity())).unwrap();
        assert!(foreign_lock(&book).is_none(), "自分を先客と間違えた");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod udf_tests {
    use crate::*;

    #[test]
    fn 台本の出力が解けてスピルが効く() {
        // 出力形式: セル \x1e 行 \x1e 行 … / 行の中は \x1f
        let raw = "B2\u{1e}10\u{1f}20\u{1e}30\u{1f}40\u{1c}D1\u{1e}こんにちは";
        let results = parse_udf_output(raw);
        assert_eq!(results.len(), 2);
        let mut sh = sheet::Sheet { name: "表".into(), ..Default::default() };
        let mut py = Cell::input("=PY(\"f\",A1)");
        py.value = sheet::Value::Error("#PY?".into());
        sh.set(Pos::parse("B2").unwrap(), py);
        let (spills, n, c) = apply_py_results(&mut sh, &results, &Default::default());
        assert_eq!((n, c), (2, 0));
        // 錨は式を保ったまま値が入る
        let b2 = sh.get(Pos::parse("B2").unwrap()).unwrap();
        assert!(b2.formula.is_some(), "式が消えた");
        assert_eq!(b2.value, sheet::Value::Number(10.0));
        // スピル面
        assert_eq!(sh.value(Pos::parse("C3").unwrap()), sheet::Value::Number(40.0));
        assert_eq!(spills.get(&Pos::parse("B2").unwrap()), Some(&(2, 2)));
        assert_eq!(sh.value(Pos::parse("D1").unwrap()), sheet::Value::Text("こんにちは".into()));
    }

    #[test]
    fn スピル先に他人のデータがあれば止まる() {
        let mut sh = sheet::Sheet { name: "表".into(), ..Default::default() };
        sh.set(Pos::parse("B2").unwrap(), Cell::input("=PY(\"f\")"));
        sh.set(Pos::parse("C3").unwrap(), Cell::input("大事なメモ"));
        let raw = "B2\u{1e}1\u{1f}2\u{1e}3\u{1f}4";
        let (spills, n, c) =
            apply_py_results(&mut sh, &parse_udf_output(raw), &Default::default());
        assert_eq!((n, c), (0, 1));
        assert_eq!(
            sh.value(Pos::parse("B2").unwrap()),
            sheet::Value::Error("#SPILL!".into())
        );
        assert_eq!(
            sh.value(Pos::parse("C3").unwrap()),
            sheet::Value::Text("大事なメモ".into()),
            "他人のデータを潰した"
        );
        assert!(spills.is_empty());
    }

    #[test]
    fn 縮んだスピルの残骸は消える() {
        let mut sh = sheet::Sheet { name: "表".into(), ..Default::default() };
        sh.set(Pos::parse("A1").unwrap(), Cell::input("=PY(\"f\")"));
        // 前回 1x3 で展開していた
        sh.set(Pos::parse("B1").unwrap(), Cell::input("古い"));
        sh.set(Pos::parse("C1").unwrap(), Cell::input("残骸"));
        let mut prev = std::collections::HashMap::new();
        prev.insert(Pos::parse("A1").unwrap(), (1u32, 3u32));
        // 今回はスカラー
        let raw = "A1\u{1e}9";
        let (_, n, c) = apply_py_results(&mut sh, &parse_udf_output(raw), &prev);
        assert_eq!((n, c), (1, 0));
        assert_eq!(sh.value(Pos::parse("A1").unwrap()), sheet::Value::Number(9.0));
        assert!(sh.value(Pos::parse("C1").unwrap()).is_empty(), "残骸が残った");
    }

    #[test]
    fn 台本が実際にpythonで回る() {
        // .venv が無い機械では黙って飛ぶ(HIKITSUGI の作法)。
        // cargo test の cwd は calc/ なので、リポジトリ直下の .venv も見る
        let py = ["../.venv/bin/python", ".venv/bin/python"]
            .iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.exists());
        let Some(py) = py else { return };
        let dir = std::env::temp_dir().join(format!("jo-udf-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let out = dir.join("out.txt");
        let defs = "def 倍(x):\n    return x * 2\ndef 表(r):\n    return [[v * 10 for v in row] for row in r]";
        let calls = vec![
            (
                "B1".to_string(),
                "倍".to_string(),
                vec![sheet::calc::PyArg::One(sheet::Value::Number(21.0))],
            ),
            (
                "D1".to_string(),
                "表".to_string(),
                vec![sheet::calc::PyArg::Rect(
                    2,
                    vec![
                        sheet::Value::Number(1.0),
                        sheet::Value::Number(2.0),
                        sheet::Value::Number(3.0),
                        sheet::Value::Number(4.0),
                    ],
                )],
            ),
        ];
        let script = build_udf_script(defs, &calls, &out);
        let py_path = dir.join("t.py");
        std::fs::write(&py_path, script).unwrap();
        let o = std::process::Command::new(&py).arg(&py_path).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let raw = std::fs::read_to_string(&out).unwrap();
        let results = parse_udf_output(&raw);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].1[0][0], "42", "倍(21) が違う: {raw:?}");
        assert_eq!(results[1].1[1][1], "40", "表の2x2が違う");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn シート名の変更が式の参照に追随する() {
        // 素の参照と '…' 付きの両方を書き換える
        assert_eq!(
            rename_refs_in("Sheet2!A1+SUM('Sheet2'!B1:B9)", "Sheet2", "集計").as_deref(),
            Some("集計!A1+SUM(集計!B1:B9)")
        );
        // 別の語の続き(合計! の中の 計!)は書き換えない
        assert_eq!(rename_refs_in("合計!A1", "計", "x"), None);
        // 文字列の中は触らない
        assert_eq!(rename_refs_in("IF(A1=\"Sheet2!\",1,2)", "Sheet2", "x"), None);
        // 空白入りの新しい名前は '…' で包む
        assert_eq!(
            rename_refs_in("Sheet2!A1", "Sheet2", "売 上").as_deref(),
            Some("'売 上'!A1")
        );
        // ブック全体: 式の数を数え、名前の定義も追随する
        let mut b = Book::new();
        b.sheets.push(sheet::Sheet::new("Sheet2"));
        b.sheets[0].set(Pos::parse("A1").unwrap(), Cell::input("=Sheet2!B1*2"));
        b.sheets[0].names.push(("単価".into(), "Sheet2!B2".into()));
        let n = rename_sheet_refs(&mut b, "Sheet2", "集計");
        assert_eq!(n, 1);
        assert_eq!(
            b.sheets[0].get(Pos::parse("A1").unwrap()).unwrap().formula.as_deref(),
            Some("集計!B1*2") // 式は = 抜きで持つ
        );
        assert_eq!(b.sheets[0].names[0].1, "集計!B2");
    }

    #[test]
    fn 複製の名前はexcelの流儀() {
        let mut b = Book::new();
        let base = b.sheets[0].name.clone();
        assert_eq!(copy_sheet_name(&b, &base), format!("{base} (2)"));
        b.sheets.push(sheet::Sheet::new(&format!("{base} (2)")));
        assert_eq!(copy_sheet_name(&b, &base), format!("{base} (3)"));
    }
}

#[cfg(test)]
mod pivot_e2e_tests {
    use crate::*;

    /// 実物の python+polars で端から端まで(挿入 → 置かれる → pivot_at →
    /// ピボット上のロック)。.venv が見つからない環境では飛ばす
    #[gpui::test]
    async fn ピボットは挿入から締めまで通しで効く(cx: &mut gpui::TestAppContext) {
        if !std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../.venv/bin/python")
            .exists()
        {
            eprintln!("skip: .venv が無い(polars の端到端は飛ばす)");
            return;
        }
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            for (a1, v) in [
                ("A1", "区分"), ("B1", "月"), ("C1", "金額"),
                ("A2", "筆記具"), ("B2", "4月"), ("C2", "100"),
                ("A3", "紙製品"), ("B3", "5月"), ("C3", "200"),
                ("A4", "筆記具"), ("B4", "5月"), ("C4", "50"),
            ] {
                this.cursor = Pos::parse(a1).unwrap();
                this.sync_input();
                this.input.insert(v);
                assert!(this.commit());
            }
            this.anchor = None;
            this.cursor = Pos::parse("B2").unwrap();
            this.sync_input();
            this.run_cmd("pivot-insert", cx);
            this.apply_pick("☐ 区分", cx);
            this.apply_pick("→ 決定(列の選択へ)", cx);
            this.apply_pick("→ 決定(列は無しでもよい)", cx);
            this.apply_pick("金額", cx);
            this.apply_pick("合計", cx);
        });
        // polars の子プロセスが返るまで(background executor を回す)
        cx.executor().advance_clock(std::time::Duration::from_secs(30));
        cx.run_until_parked();
        c.update(cx, |this, cx| {
            assert_eq!(this.book.pivots.len(), 1, "ピボットが置かれない: {}", this.status);
            let d = this.book.pivots[0].clone();
            assert!(d.size.0 > 0, "大きさが入らない");
            // 出力の頭(見出しの下)に合計が入っている
            let val = |p: Pos| {
                this.book.sheets[0].get(p).map(|c| c.value.display()).unwrap_or_default()
            };
            let body: Vec<String> = (0..d.size.0)
                .map(|r| val(Pos::new(d.dest.row + r, d.dest.col + d.size.1 - 1)))
                .collect();
            assert!(
                body.iter().any(|v| v == "150"),
                "筆記具の合計 150 が出ない: {body:?}"
            );
            // 総計は既定で入り(本家と同じ)、見出しには本家風の帯が掛かる
            let all: Vec<String> = (0..d.size.0)
                .flat_map(|r| (0..d.size.1).map(move |c| (r, c)))
                .map(|(r, c)| val(Pos::new(d.dest.row + r, d.dest.col + c)))
                .collect();
            assert!(all.iter().any(|v| v == "総計"), "総計が無い: {all:?}");
            assert!(all.iter().any(|v| v == "350"), "総計の値が無い: {all:?}");
            let head = this.book.sheets[0].get(d.dest).unwrap().fmt.clone();
            assert_eq!(head.fill.as_deref(), Some("4472C4"), "見出しの帯が無い");
            assert!(head.bold);
            // 置いた直後にカーソルが集計へ移り、ピボットのタブが開いている
            assert_eq!(this.cursor, d.dest, "カーソルが集計へ移らない");
            let ti = ribbon::calc_tabs()
                .iter()
                .position(|t| t.cmds.iter().any(|c| c.id == "pivot-layout"))
                .unwrap();
            assert_eq!(this.tab, ti, "ピボットテーブルのタブが開かない");
            // ピボットの上では締まる(文脈タブと同じ判定 pivot_at)
            assert!(this.pivot_at(this.cursor).is_some(), "pivot_at が効かない");
            this.run_cmd("data-validation", cx);
            assert!(this.dv_dlg.is_none(), "ピボットの上で入力規則が開いた");
            assert!(this.status.contains("ピボット"), "{}", this.status);
            // フィールドリスト: いまの指図が ✓ 入りで読み込まれる
            this.run_cmd("pivot-fields", cx);
            assert_eq!(this.pick_kind, "pivot-rows-pick", "フィールドリストが開かない");
            {
                let (items, _) = this.pick.as_ref().unwrap();
                assert!(items.iter().any(|i| i == "☑ 区分"), "既存の行が ✓ にならない: {items:?}");
            }
            // 月を「列」へ広げて置き直す(Excel の形 — 1行目に札が出る)
            this.apply_pick("→ 決定(列の選択へ)", cx);
            this.apply_pick("☐ 月", cx);
            this.apply_pick("→ 決定(列は無しでもよい)", cx);
            this.apply_pick("金額", cx);
            this.apply_pick("合計", cx);
        });
        cx.executor().advance_clock(std::time::Duration::from_secs(30));
        cx.run_until_parked();
        c.update(cx, |this, cx| {
            assert_eq!(this.book.pivots.len(), 1, "組み替えで増殖した: {}", this.status);
            let d = &this.book.pivots[0];
            assert_eq!(d.rows_sel, vec!["区分".to_string()], "組み替えが効かない");
            assert_eq!(d.cols_sel, vec!["月".to_string()], "列への組み替えが効かない");
            assert!(d.totals, "総計の性質が引き継がれない");
            // Excel と同じ1行目の札(合計 / 金額 と 月)
            let d = this.book.pivots[0].clone();
            let label = this.book.sheets[0]
                .get(d.dest)
                .map(|x| x.value.display())
                .unwrap_or_default();
            assert_eq!(label, "合計 / 金額", "1行目の札が無い");
            let month_label = this.book.sheets[0]
                .get(Pos::new(d.dest.row, d.dest.col + 1))
                .map(|x| x.value.display())
                .unwrap_or_default();
            assert_eq!(month_label, "月", "列の見出しの札が無い");
            // 絞り込み(▼ 相当): 紙製品を隠して置き直す
            this.pivot_flt = Some((
                0,
                "区分".into(),
                std::iter::once("紙製品".to_string()).collect(),
            ));
            this.pick_kind = "pivot-filter-pick";
            this.apply_pick("→ 決定(絞り込む)", cx);
        });
        cx.executor().advance_clock(std::time::Duration::from_secs(30));
        cx.run_until_parked();
        c.update(cx, |this, _| {
            let d = this.book.pivots[0].clone();
            assert_eq!(d.hide, vec![("区分".to_string(), vec!["紙製品".to_string()])]);
            let all: Vec<String> = (0..d.size.0)
                .flat_map(|r| (0..d.size.1).map(move |c| (r, c)))
                .map(|(r, c)| {
                    this.book.sheets[0]
                        .get(Pos::new(d.dest.row + r, d.dest.col + c))
                        .map(|x| x.value.display())
                        .unwrap_or_default()
                })
                .collect();
            assert!(!all.iter().any(|v| v == "紙製品"), "隠したのに出ている: {all:?}");
            assert!(all.iter().any(|v| v == "筆記具"), "残るはずの値が消えた: {all:?}");
        });
        // スタイルギャラリー: 緑を選ぶと帯が掛け替わる
        c.update(cx, |this, cx| {
            let d = this.book.pivots[0].clone();
            this.anchor = None;
            this.cursor = d.dest;
            this.sync_input();
            this.run_cmd("pivot-style", cx);
            assert_eq!(this.pick_kind, "pivot-style-pick", "スタイルの一覧が開かない");
            this.apply_pick("緑", cx);
        });
        cx.executor().advance_clock(std::time::Duration::from_secs(30));
        cx.run_until_parked();
        c.update(cx, |this, _| {
            let d = &this.book.pivots[0];
            assert_eq!(d.style, "緑");
            let head = this.book.sheets[0].get(d.dest).unwrap().fmt.clone();
            assert_eq!(head.fill.as_deref(), Some("548235"), "緑の帯にならない");
        });
    }
}

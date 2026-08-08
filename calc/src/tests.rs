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
        assert!(label.fmt.bold && label.fmt.borders.top.on, "合計行の書式が付かない");
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
            name: String::new(),
            vfilter: None,
            group_by: Vec::new(),
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

    #[gpui::test]
    fn 罫線の板は連打でき表の形が1押しで掛かる(cx: &mut gpui::TestAppContext) {
        use sheet::model::BStyle;
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            this.anchor = Some(Pos::new(0, 0));
            this.cursor = Pos::new(1, 1);
            this.sync_input();
            this.run_cmd("borders", cx);
            assert!(this.border_pal.is_some(), "パレットが開かない");
            fn bd(this: &Calc, r: u32, c2: u32) -> sheet::model::Borders {
                this.book.sheets[0].get(Pos::new(r, c2)).unwrap().fmt.borders
            }
            // 場所×ペンの直交モデル(Microsoft の型スタンプは持たない —
            // 発注者確定 2026-08-08)。帳票の枠はペンを替えながら連打で組む:
            // 細で格子 → ペンを中太にして外枠 → ペンを二重にして下罫線
            this.apply_borders("すべての罫線(格子)");
            assert!(this.border_pal.is_some(), "パレットが閉じた(連打できない)");
            this.pen_style = BStyle::Medium;
            this.apply_borders("外枠");
            assert_eq!(bd(this, 0, 0).top.style, BStyle::Medium);
            assert_eq!(bd(this, 0, 0).left.style, BStyle::Medium);
            assert_eq!(bd(this, 0, 0).bottom.style, BStyle::Thin, "格子の内側が外枠で潰れた");
            assert_eq!(bd(this, 0, 0).right.style, BStyle::Thin);
            assert_eq!(bd(this, 1, 1).bottom.style, BStyle::Medium);
            assert_eq!(bd(this, 1, 1).right.style, BStyle::Medium);
            this.pen_style = BStyle::Double;
            this.apply_borders("下罫線");
            let _ = cx;
            assert_eq!(bd(this, 1, 0).bottom.style, BStyle::Double);
            assert_eq!(bd(this, 1, 1).bottom.style, BStyle::Double);
        });
    }

    #[gpui::test]
    fn jocalcの口は読み書きと展開ができる(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            // 書く: 数・文字・式(= から始まる)
            let r = crate::rpc::handle(
                this,
                r#"{"cmd":"set","a1":"A1","values":[["品名","金額"],["鉛筆",100],["ノート",250],["合計","=SUM(B2:B3)"]]}"#,
                cx,
            );
            assert!(r.contains("\"ok\":true"), "set が通らない: {r}");
            // 読む: 式は計算済みの値、数は数のまま
            let r = crate::rpc::handle(this, r#"{"cmd":"get","a1":"B4"}"#, cx);
            assert!(r.contains("[[350]]"), "式が計算されていない: {r}");
            let r = crate::rpc::handle(this, r#"{"cmd":"get","a1":"A1:B2"}"#, cx);
            assert!(r.contains("\"品名\""), "文字が読めない: {r}");
            assert!(r.contains("100"), "数が読めない: {r}");
            // 式そのもの
            let r = crate::rpc::handle(this, r#"{"cmd":"get_formula","a1":"B4"}"#, cx);
            assert!(r.contains("=SUM(B2:B3)"), "式が読めない: {r}");
            // 表の広がり(expand='table')
            let r = crate::rpc::handle(this, r#"{"cmd":"expand","a1":"A1"}"#, cx);
            assert!(r.contains("\"rows\":4") && r.contains("\"cols\":2"), "広がりが違う: {r}");
            // ブックの情報
            let r = crate::rpc::handle(this, r#"{"cmd":"book_info"}"#, cx);
            assert!(r.contains("Sheet1"), "シート名が出ない: {r}");
            // 無いシートは正しく断る
            let r = crate::rpc::handle(this, r#"{"cmd":"get","a1":"A1","sheet":"無い"}"#, cx);
            assert!(r.contains("err"), "無いシートを断らない: {r}");
            // 保護中は書かせない
            this.book.sheets[0].protected = true;
            let r = crate::rpc::handle(this, r#"{"cmd":"set","a1":"A9","values":[[1]]}"#, cx);
            assert!(r.contains("保護"), "保護を破って書けてしまう: {r}");
            this.book.sheets[0].protected = false;
            // 未保存の変更がある間は new/open を断る(黙って捨てない)
            let r = crate::rpc::handle(this, r#"{"cmd":"new"}"#, cx);
            assert!(r.contains("err"), "未保存で new が通った: {r}");
        });
    }

    #[gpui::test]
    fn 左上が空の結合は最初の値を左上へ移す(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, _cx| {
            // A1 空、B1 に題 — A1:C1 を結合すると題が左上へ移る
            this.book.sheets[0].set(Pos::new(0, 1), sheet::Cell::input("題"));
            this.merge_do(Pos::new(0, 0), Pos::new(0, 2), "中央");
            assert_eq!(
                this.book.sheets[0].get(Pos::new(0, 0)).unwrap().value.display(),
                "題", "最初の値が左上へ移らない"
            );
            assert!(
                this.book.sheets[0]
                    .get(Pos::new(0, 1))
                    .is_none_or(|c| c.value.is_empty()),
                "元の場所に残っている"
            );
            assert!(this.status.contains("移しました"), "移したことを言わない: {}", this.status);
            // 「セルの結合」: 書式は値があった場所の書式ごと移り、中央揃えにはしない
            let mut src = sheet::Cell::input("題2");
            src.fmt.bold = true;
            src.fmt.align = sheet::model::HAlign::Right;
            this.book.sheets[0].set(Pos::new(5, 1), src);
            this.merge_do(Pos::new(5, 0), Pos::new(5, 2), "結合だけ");
            let got = this.book.sheets[0].get(Pos::new(5, 0)).unwrap().clone();
            assert_eq!(got.value.display(), "題2");
            assert!(got.fmt.bold, "書式(太字)が移らない");
            assert_eq!(got.fmt.align, sheet::model::HAlign::Right, "揃えまで変えられた");
            // 「結合して中央に配置」だけが中央を掛ける
            let mut src = sheet::Cell::input("題3");
            src.fmt.bold = true;
            this.book.sheets[0].set(Pos::new(7, 1), src);
            this.merge_do(Pos::new(7, 0), Pos::new(7, 2), "中央");
            let got = this.book.sheets[0].get(Pos::new(7, 0)).unwrap().clone();
            assert!(got.fmt.bold, "中央配置で書式が落ちた");
            assert_eq!(got.fmt.align, sheet::model::HAlign::Center, "中央配置が中央でない");
            // 横方向: 行ごとに同じ扱い(2行目は C2 の値が左端へ)
            this.book.sheets[0].set(Pos::new(2, 2), sheet::Cell::input("乙"));
            this.merge_do(Pos::new(2, 0), Pos::new(2, 2), "横方向");
            assert_eq!(
                this.book.sheets[0].get(Pos::new(2, 0)).unwrap().value.display(),
                "乙", "横方向で行の左端へ移らない"
            );
        });
    }

    #[gpui::test]
    fn 結合は1つのセルとして歩ける(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, _cx| {
            // B1:C2 を結合
            this.merge_do(Pos::new(0, 1), Pos::new(1, 2), "中央");
            // 右へ: A1 → B1(左上)→ D1(結合を飛び越す)
            this.cursor = Pos::new(0, 0);
            this.sync_input();
            this.move_cursor(0, 1);
            assert_eq!(this.cursor, Pos::new(0, 1), "結合に入ると左上に立つ");
            this.move_cursor(0, 1);
            assert_eq!(this.cursor, Pos::new(0, 3), "結合を1つとして飛び越す");
            // 左へ戻る: D1 → B1(左上)→ A1
            this.move_cursor(0, -1);
            assert_eq!(this.cursor, Pos::new(0, 1));
            this.move_cursor(0, -1);
            assert_eq!(this.cursor, Pos::new(0, 0));
            // 下から入っても左上へ(C3 の上=C2 は呑まれている)
            this.cursor = Pos::new(2, 2);
            this.sync_input();
            this.move_cursor(-1, 0);
            assert_eq!(this.cursor, Pos::new(0, 1), "下から入っても左上に立つ");
            // merge_of がクリックの吸い付け先を返す
            assert_eq!(
                this.merge_of(Pos::new(1, 2)),
                Some((Pos::new(0, 1), Pos::new(1, 2)))
            );
        });
    }

    #[gpui::test]
    fn ホームの全釦を一巡り点検(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            use sheet::model::{HAlign, VAlign};
            this.book.sheets[0].set(Pos::new(0, 0), sheet::Cell::input("abc"));
            this.cursor = Pos::new(0, 0);
            this.sync_input();
            let f = |this: &Calc| this.book.sheets[0].get(Pos::new(0, 0)).unwrap().fmt.clone();
            // --- 書式の掛かり(モデル) ---
            for id in ["bold", "italic", "underline", "strikeout", "subscript", "wrap", "direction"] {
                this.run_cmd(id, cx);
            }
            let g = f(this);
            assert!(g.bold && g.italic && g.underline && g.strike && g.subscript && g.wrap && g.rtl_text,
                "文字飾りが掛からない: {g:?}");
            this.run_cmd("top", cx);
            assert_eq!(f(this).valign, VAlign::Top);
            this.run_cmd("middle", cx);
            assert_eq!(f(this).valign, VAlign::Middle);
            this.run_cmd("bottom", cx);
            assert_eq!(f(this).valign, VAlign::Bottom);
            this.run_cmd("align-left", cx);
            assert_eq!(f(this).align, HAlign::Left);
            this.run_cmd("align-center", cx);
            assert_eq!(f(this).align, HAlign::Center);
            this.run_cmd("align-right", cx);
            assert_eq!(f(this).align, HAlign::Right);
            this.run_cmd("align-just", cx);
            assert_eq!(f(this).align, HAlign::Justify);
            this.run_cmd("incfont", cx);
            assert_eq!(f(this).size_c, Some(1200), "文字が大きくならない");
            this.run_cmd("decfont", cx);
            assert_eq!(f(this).size_c, Some(1100));
            this.run_cmd("currency", cx);
            assert_eq!(f(this).number_format.as_deref(), Some("¥#,##0"));
            this.run_cmd("percents", cx);
            assert_eq!(f(this).number_format.as_deref(), Some("0%"));
            this.run_cmd("comma", cx);
            assert_eq!(f(this).number_format.as_deref(), Some("#,##0"));
            this.run_cmd("digit-inc", cx);
            assert_eq!(f(this).number_format.as_deref(), Some("#,##0.0"));
            this.run_cmd("digit-dec", cx);
            assert_eq!(f(this).number_format.as_deref(), Some("#,##0"));
            this.run_cmd("clear", cx);
            assert_eq!(f(this), Default::default(), "書式のクリアが効かない");
            // --- 板・小窓が開く系 ---
            let mut close = |this: &mut Calc| {
                this.pick = None;
                this.pick_kind = "value";
                this.pick_note = None;
                this.menu_sub = None;
                this.prompt = None;
                this.fn_dlg = None;
                this.fmt_panel = None;
            };
            this.run_cmd("borders", cx);
            assert!(this.border_pal.is_some(), "罫線のパレットが開かない");
            this.border_pal = None;
            for (id, kind) in [
                ("changecase", "changecase"),
                ("fontname", "font"),
                ("fontcolor", "font-color"),
                ("fillparag", "fill-color"),

                ("text-orient", "orient-pick"),
                ("merge", "merge-pick"), // 結合は範囲が要る(下で anchor を張る)
                ("format", "numfmt-pick"),
                ("cell-styles", "cell-style"),
                ("defname", "names-pick"),
            ] {
                if id == "merge" {
                    this.anchor = Some(Pos::new(0, 0));
                    this.cursor = Pos::new(0, 1);
                    this.sync_input();
                }
                this.run_cmd(id, cx);
                assert_eq!(this.pick_kind, kind, "{id} の板が開かない");
                assert!(this.pick.is_some(), "{id} の板が無い");
                close(this);
                if id == "merge" {
                    this.anchor = None;
                    this.cursor = Pos::new(0, 0);
                    this.sync_input();
                }
            }
            this.run_cmd("fontsize", cx);
            assert!(this.pick.is_some(), "fontsize の板が開かない");
            close(this);
            this.run_cmd("condformat", cx);
            assert_eq!(this.menu_sub.as_deref(), Some("cond"), "条件付き書式の一覧が開かない");
            close(this);
            this.run_cmd("insert-function", cx);
            assert!(this.fn_dlg.is_some(), "関数の挿入の小窓が開かない");
            close(this);
            this.run_cmd("cell-format", cx);
            assert!(this.fmt_panel.is_some(), "書式の小窓が開かない");
            close(this);
            this.run_cmd("replace", cx);
            assert!(matches!(this.prompt, Some(("find", _))), "置換の板が開かない");
            close(this);
            // --- 行動系 ---
            this.run_cmd("copystyle", cx);
            assert!(this.brush.is_some(), "書式のコピーの刷毛が持てない");
            this.brush = None;
            this.run_cmd("copy", cx);
            this.cursor = Pos::new(4, 0);
            this.sync_input();
            this.run_cmd("paste", cx);
            assert_eq!(
                this.book.sheets[0].get(Pos::new(4, 0)).unwrap().value.display(),
                "abc", "コピー→貼り付けが効かない"
            );
            this.run_cmd("selectall", cx);
            assert!(this.anchor.is_some(), "すべて選択が効かない");
            this.anchor = None;
            // フィル(先頭行を下へ)
            this.book.sheets[0].set(Pos::new(9, 0), sheet::Cell::input("7"));
            this.anchor = Some(Pos::new(9, 0));
            this.cursor = Pos::new(11, 0);
            this.sync_input();
            this.run_cmd("fill-num", cx);
            assert_eq!(
                this.book.sheets[0].get(Pos::new(11, 0)).unwrap().value.display(),
                "7", "フィルが効かない"
            );
            this.anchor = None;
            // 絞り込みの張り外し
            this.book.sheets[0].set(Pos::new(20, 0), sheet::Cell::input("見出し"));
            this.book.sheets[0].set(Pos::new(21, 0), sheet::Cell::input("1"));
            this.run_cmd("setfilter", cx);
            assert!(this.auto_filter.is_some(), "絞り込みが張れない");
            this.run_cmd("clear-filter", cx);
            assert!(this.auto_filter.is_none(), "絞り込みが解けない");
            // 行の出し入れ(cell-ins/cell-del は行に固定 — 台帳の既知の控え)
            let rows0 = this.book.sheets[0].extent().0;
            this.cursor = Pos::new(0, 0);
            this.sync_input();
            this.run_cmd("cell-ins", cx);
            assert_eq!(this.book.sheets[0].extent().0, rows0 + 1, "行の挿入が効かない");
            this.run_cmd("cell-del", cx);
            assert_eq!(this.book.sheets[0].extent().0, rows0, "行の削除が効かない");
            // 表の雛形(選択に書式+表オブジェクト)
            this.anchor = Some(Pos::new(30, 0));
            this.cursor = Pos::new(32, 1);
            this.sync_input();
            this.run_cmd("table-tpl", cx);
            assert!(!this.book.sheets[0].tables.is_empty(), "表の雛形が掛からない");
        });
    }

    #[gpui::test]
    fn ホームの文字飾り4種が掛かる(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            this.book.sheets[0].set(Pos::new(0, 0), sheet::Cell::input("字"));
            this.cursor = Pos::new(0, 0);
            this.sync_input();
            for id in ["bold", "italic", "underline", "strikeout"] {
                this.run_cmd(id, cx);
            }
            let f = &this.book.sheets[0].get(Pos::new(0, 0)).unwrap().fmt;
            assert!(f.bold, "太字が掛からない");
            assert!(f.italic, "斜体が掛からない");
            assert!(f.underline, "下線が掛からない");
            assert!(f.strike, "取り消し線が掛からない");
        });
    }

    #[gpui::test]
    fn テキスト取り込みの板は置き場所と取り込みが効く(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            this.import_pend = Some(crate::py::ImportPend {
                path: std::path::PathBuf::from("試.csv"),
                enc: 0,
                delim: 0,
                custom: String::new(),
                dest: Pos::new(0, 0),
                grid: vec![
                    vec!["品名".into(), "金額".into()],
                    vec!["鉛筆".into(), "100".into()],
                ],
                used: ("utf-8-sig".into(), ",".into()),
            });
            this.import_pick();
            assert_eq!(this.pick_kind, "csv-import-pick");
            // 置き場所を B3 に
            this.prompt = Some(("csv-dest", Editor::new("B3")));
            this.finish_prompt(cx);
            assert_eq!(this.import_pend.as_ref().unwrap().dest, Pos::new(2, 1));
            // 取り込む
            this.apply_pick("→ 取り込む(2 行)", cx);
            assert!(this.import_pend.is_none(), "板が閉じない");
            let got = this.book.sheets[0].get(Pos::new(3, 1)).unwrap().value.display();
            assert_eq!(got, "鉛筆", "置き場所に流し込まれていない");
            let got = this.book.sheets[0].get(Pos::new(3, 2)).unwrap().value.display();
            assert_eq!(got, "100");
        });
    }

    #[test]
    fn csvの台本は文字コードと区切りの指定が効く() {
        // .venv が無い機械では黙って飛ぶ(HIKITSUGI の作法)
        let Some(py) = ["../.venv/bin/python", ".venv/bin/python"]
            .iter()
            .map(std::path::PathBuf::from)
            .find(|p| p.exists())
        else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("jo-csvwiz-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let py_path = dir.join("jo_csv.py");
        std::fs::write(&py_path, crate::py::CSV_PY).unwrap();
        // CP932 のセミコロン区切り(自動では , を想定しがちな中身)
        let csv_path = dir.join("試.csv");
        let sjis: Vec<u8> = [
            0x95i32, 0x69, 0x96, 0xbc, 0x3b, 0x8b, 0xe0, 0x8a, 0x7a, 0x0a, // 品名;金額
            0x89, 0x94, 0x95, 0x4d, 0x3b, 0x31, 0x30, 0x30, // 鉛筆;100
        ]
        .iter()
        .map(|b| *b as u8)
        .collect();
        std::fs::write(&csv_path, sjis).unwrap();
        let o = std::process::Command::new(&py)
            .arg(&py_path)
            .arg(&csv_path)
            .arg("cp932")
            .arg(";")
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let out = String::from_utf8_lossy(&o.stdout).to_string();
        let mut rows = out.split('\u{1e}');
        let meta = rows.next().unwrap();
        assert!(meta.starts_with('\u{01}'), "下ごしらえの報告が無い: {meta:?}");
        assert!(meta.contains("cp932"), "使った文字コードの報告が無い: {meta:?}");
        let first: Vec<&str> = rows.next().unwrap().split('\u{1f}').collect();
        assert_eq!(first, vec!["品名", "金額"], "CP932+セミコロンで読めない: {first:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[gpui::test]
    fn スパークラインは3種を選んで置ける(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, _cx| {
            for (i, v) in ["3", "-1", "2"].iter().enumerate() {
                this.book.sheets[0].set(Pos::new(0, i as u32), sheet::Cell::input(v));
            }
            this.anchor = Some(Pos::new(0, 0));
            this.cursor = Pos::new(0, 2);
            this.sync_input();
            this.insert_sparkline("spark-col");
            let sp = this.book.sheets[0].shapes_new.last().unwrap().clone();
            assert_eq!(sp.kind, "spark-col");
            assert_eq!(sp.points.len(), 3);
            // 底は 0 の高さ(範囲は -1..3 → 0 は 3/4 の位置 = y 0.75)
            assert!((sp.base - 0.75).abs() < 1e-4, "底が違う: {}", sp.base);
            // 負の値の棒の先端は底より下
            assert!(sp.points[1].1 > sp.base, "負の棒が下に伸びていない");
            this.insert_sparkline("spark-wl");
            let sp = this.book.sheets[0].shapes_new.last().unwrap().clone();
            assert_eq!(sp.kind, "spark-wl");
            assert_eq!(sp.base, 0.5);
            assert_eq!(sp.points.iter().map(|p| p.1).collect::<Vec<_>>(), vec![0.1, 0.9, 0.1]);
        });
    }

    #[gpui::test]
    fn 図形の設定パネルの経路で性質が変わる(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            this.book.sheets[0].shapes_new.push(sheet::model::SheetShape {
                at: Pos::new(0, 0),
                width_px: 100.0,
                height_px: 60.0,
                kind: "rect".into(),
                line: Some("1B6E3C".into()),
                ..Default::default()
            });
            this.shape_sel = Some(0);
            // 塗りの直指定(小文字でも通る)
            this.prompt = Some(("shape-fill-rgb", Editor::new("ff0000")));
            this.finish_prompt(cx);
            assert_eq!(
                this.book.sheets[0].shapes_new[0].fill.as_deref(),
                Some("FF0000"),
                "塗りの直指定が効かない"
            );
            // 回転の直指定(負は 360 に折り返す)
            this.prompt = Some(("shape-rot", Editor::new("-30")));
            this.finish_prompt(cx);
            assert!(
                (this.book.sheets[0].shapes_new[0].rot - 330.0).abs() < 0.01,
                "回転の直指定が効かない: {}",
                this.book.sheets[0].shapes_new[0].rot
            );
            // 太さ・不透明度・影・反転(パネルの釦の実体は shape_edit)
            this.shape_edit(|sp| {
                sp.line_w = 3.0;
                sp.alpha = 0.5;
                sp.shadow = true;
                sp.flip_h = true;
            });
            let sp = &this.book.sheets[0].shapes_new[0];
            assert!(sp.shadow && sp.flip_h);
            assert!((sp.line_w - 3.0).abs() < 0.01 && (sp.alpha - 0.5).abs() < 0.01);
            // 空 Enter = 塗りなし
            this.prompt = Some(("shape-fill-rgb", Editor::new("")));
            this.finish_prompt(cx);
            assert!(this.book.sheets[0].shapes_new[0].fill.is_none());
            // 読めない色は板が残る(黙って捨てない)
            this.prompt = Some(("shape-line-rgb", Editor::new("赤")));
            this.finish_prompt(cx);
            assert!(this.prompt.is_some(), "読めない色で板が閉じた");
            this.prompt = None;
            // 選択が無ければ何も起きない(shape_edit の守り)
            this.shape_sel = None;
            this.shape_edit(|sp| sp.rot = 10.0);
            assert!((this.book.sheets[0].shapes_new[0].rot - 330.0).abs() < 0.01);
        });
    }

    #[gpui::test]
    fn r1c1では見せ方が変わり中身はa1のまま(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            this.book.sheets[0].set(Pos::new(0, 0), sheet::Cell::input("10"));
            this.book.sheets[0].set(Pos::new(4, 4), sheet::Cell::input("=A1*2"));
            recalc_book(&mut this.book, 0);
            this.run_cmd("ref-style", cx);
            assert!(this.book.r1c1);
            // 数式バーは R1C1 で見える
            this.cursor = Pos::new(4, 4);
            this.sync_input();
            assert_eq!(this.input.text(), "=R[-4]C[-4]*2", "見せ方が変わらない");
            // R1C1 で打っても中身は A1 で仕舞われる
            this.input = Editor::new("=R[-4]C[-4]+RC[-1]");
            this.commit();
            let f = this.book.sheets[0].get(Pos::new(4, 4)).unwrap().formula.clone();
            assert_eq!(f.as_deref(), Some("A1+D5"), "中身が A1 になっていない: {f:?}");
            // 動かず確定しても値が壊れない(往復の対称性)
            this.sync_input();
            assert!(this.commit(), "対称でない(見せた式が別物として書き戻る)");
            let f2 = this.book.sheets[0].get(Pos::new(4, 4)).unwrap().formula.clone();
            assert_eq!(f2.as_deref(), Some("A1+D5"));
            // 切り替えれば A1 に戻る
            this.run_cmd("ref-style", cx);
            this.sync_input();
            assert_eq!(this.input.text(), "=A1+D5");
        });
    }

    #[gpui::test]
    fn ラベルと値とグループの指図が板から入る(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            // 元の表
            for (r, row) in [["区分", "金額"], ["東京", "100"], ["大阪", "50"], ["東村山", "30"]]
                .iter()
                .enumerate()
            {
                for (cc, v) in row.iter().enumerate() {
                    this.book.sheets[0].set(Pos::new(r as u32, cc as u32), sheet::Cell::input(v));
                }
            }
            let mut d = def(&["区分"], &[], "金額", "合計");
            // 試験では polars を飛ばさない: spawn_pivot はシート名で早期に
            // 止まる(指図の欄が入ることだけを確かめる)
            d.sheet = "試験では無いシート".into();
            d.src = (Pos::new(0, 0), Pos::new(3, 1));
            this.book.pivots.push(d);
            // ラベルで絞る: 「で始まる 東」→ 大阪だけ hide に落ちる
            this.pivot_flt = Some((0, "区分".into(), Default::default()));
            this.prompt = Some(("pivot-label", Editor::new("で始まる 東")));
            this.finish_prompt(cx);
            let hide = &this.book.pivots[0].hide;
            assert_eq!(hide.len(), 1, "hide が入らない: {hide:?}");
            assert_eq!(hide[0].0, "区分");
            assert_eq!(hide[0].1, vec!["大阪".to_string()], "落ちる値が違う: {hide:?}");
            // 値で絞る
            this.pivot_flt = Some((0, "区分".into(), Default::default()));
            this.prompt = Some(("pivot-vfilter", Editor::new("> 40")));
            this.finish_prompt(cx);
            assert_eq!(this.book.pivots[0].vfilter, Some((">".into(), 40.0)));
            // 数の幅でグループ化
            this.pivot_flt = Some((0, "金額".into(), Default::default()));
            this.prompt = Some(("pivot-group-width", Editor::new("50")));
            this.finish_prompt(cx);
            assert_eq!(this.book.pivots[0].group_by, vec![("金額".into(), "幅:50".into())]);
        });
    }

    #[test]
    fn グループ化と値のフィルターがpolarsで回る() {
        let headers: Vec<String> =
            ["日付", "区分", "金額"].iter().map(|s| s.to_string()).collect();
        let rows: Vec<Vec<String>> = [
            ["2026-01-10", "A", "100"],
            ["2026-01-25", "A", "50"],
            ["2026-02-05", "B", "30"],
            ["2026-04-01", "B", "70"],
        ]
        .iter()
        .map(|r| r.iter().map(|s| s.to_string()).collect())
        .collect();
        // 日付を月でグループ化して合計
        let mut d = def(&["日付"], &[], "金額", "合計");
        d.group_by.push(("日付".into(), "月".into()));
        let spec = pivot_spec_json(&headers, &rows, &d);
        let Some((g, _)) = run_py(spec) else { return };
        assert_eq!(g[1], vec!["2026-01", "150"], "月のグループが効かない: {g:?}");
        assert_eq!(g[2], vec!["2026-02", "30"]);
        assert_eq!(g[3], vec!["2026-04", "70"]);
        // 四半期
        let mut d = def(&["日付"], &[], "金額", "合計");
        d.group_by.push(("日付".into(), "四半期".into()));
        let spec = pivot_spec_json(&headers, &rows, &d);
        let Some((g, _)) = run_py(spec) else { return };
        assert_eq!(g[1], vec!["2026年Q1", "180"], "四半期が効かない: {g:?}");
        assert_eq!(g[2], vec!["2026年Q2", "70"]);
        // 数の幅(金額を 50 刻みで束ね、区分を数える)
        let mut d = def(&["金額"], &[], "区分", "個数");
        d.group_by.push(("金額".into(), "幅:50".into()));
        let spec = pivot_spec_json(&headers, &rows, &d);
        let Some((g, _)) = run_py(spec) else { return };
        assert_eq!(g[1], vec!["  0〜 49", "1"], "幅の帯が違う: {g:?}");
        assert_eq!(g[2], vec![" 50〜 99", "2"], "帯の並びが数字順でない: {g:?}");
        assert_eq!(g[3], vec!["100〜149", "1"]);
        // 値のフィルター(合計 >= 70 の行だけ)+ 総計はフィルター後
        let mut d = def(&["区分"], &[], "金額", "合計");
        d.vfilter = Some((">=".into(), 70.0));
        d.totals = true;
        let spec = pivot_spec_json(&headers, &rows, &d);
        let Some((g, k)) = run_py(spec) else { return };
        // A=150, B=100 → 両方残る。しきい値を上げると片方だけに
        assert_eq!(g[1], vec!["A", "150"]);
        assert_eq!(g[2], vec!["B", "100"]);
        let mut d = def(&["区分"], &[], "金額", "合計");
        d.vfilter = Some((">".into(), 120.0));
        d.totals = true;
        let spec = pivot_spec_json(&headers, &rows, &d);
        let Some((g, k)) = run_py(spec) else { return };
        assert_eq!(g[1], vec!["A", "150"], "値のフィルターが効かない: {g:?}");
        let ti = k.iter().position(|c| *c == 't').expect("総計が無い");
        assert_eq!(g[ti], vec!["総計", "150"], "総計が絞り込み後になっていない: {g:?}");
        assert_eq!(g.len(), ti + 1, "余計な行がある: {g:?}");
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
    fn 画像は選んで動かして大きさを変えて消せる(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, _cx| {
            // 1x1 の PNG を B2 に置く(挿した画像の体)
            let png: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47];
            this.sheet_mut().images_new.push(sheet::model::SheetImage {
                at: Pos::parse("B2").unwrap(),
                dx_px: 0.0,
                dy_px: 0.0,
                width_px: 120.0,
                height_px: 60.0,
                data: png,
            });
            // 当たり判定(B2 の原点 + 中ほど)
            let (ox, oy) = this.cell_origin_px(Pos::parse("B2").unwrap()).unwrap();
            let hit = this.image_at(ox + 10.0, oy + 10.0);
            assert!(hit.is_some(), "画像に当たらない");
            let (i, _, corner) = hit.unwrap();
            assert!(!corner);
            // 右下は大きさの掴み
            let (_, _, corner) = this.image_at(ox + 115.0, oy + 55.0).unwrap();
            assert!(corner, "右下の掴みにならない");
            // 移動(ドラッグの実体を直接)
            this.img_drag = Some((i, (ox + 10.0, oy + 10.0), (ox, oy), false));
            this.image_drag_at(ox + 40.0, oy + 15.0);
            let im = &this.sheet().images_new[0];
            assert!(im.dx_px > 0.0 || im.at != Pos::parse("B2").unwrap(), "動かない");
            // 大きさ(比を保つ)
            this.img_drag = Some((i, (0.0, 0.0), (ox, oy), true));
            this.image_drag_at(ox + 240.0, oy + 999.0);
            let im = &this.sheet().images_new[0];
            assert!((im.height_px / im.width_px - 0.5).abs() < 0.01, "比が崩れた: {}x{}", im.width_px, im.height_px);
            // 削除
            this.img_drag = None;
            this.img_sel = Some(0);
            assert!(this.delete_selected_image());
            assert!(this.sheet().images_new.is_empty());
        });
    }

    #[gpui::test]
    fn 条件付き書式の板の規則が掛かる(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            for (a1, v) in [("A1", "10"), ("A2", "20"), ("A3", "20"), ("A4", "5")] {
                this.cursor = Pos::parse(a1).unwrap();
                this.sync_input();
                this.input.insert(v);
                assert!(this.commit());
            }
            this.anchor = Some(Pos::parse("A1").unwrap());
            this.cursor = Pos::parse("A4").unwrap();
            this.sync_input();
            // 間(板)
            this.prompt = Some(("cond-between", Editor::new("8〜15")));
            this.finish_prompt(cx);
            // 上位N(板)
            this.prompt = Some(("cond-top", Editor::new("2")));
            this.finish_prompt(cx);
            let rules = &this.sheet().cond;
            assert_eq!(rules.len(), 2, "{}", this.status);
            assert_eq!(
                rules[0].kind,
                sheet::model::CondKind::Between(8.0, 15.0, false)
            );
            assert_eq!(rules[1].kind, sheet::model::CondKind::Top(2, false));
            // 効き方(下ごしらえ込み)
            let aux = rules[1].aux(this.sheet());
            assert!(rules[1].hits(Pos::parse("A2").unwrap(), &sheet::Value::Number(20.0), &aux));
            assert!(!rules[1].hits(Pos::parse("A4").unwrap(), &sheet::Value::Number(5.0), &aux));
            // 読めない入力は言い返す
            this.prompt = Some(("cond-between", Editor::new("abc")));
            this.finish_prompt(cx);
            assert!(this.prompt.is_some(), "読めない間の形が通った");
        });
    }

    #[gpui::test]
    fn グループ化は7段で頭打ち_基底と合わせて8レベル(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            for r in 0..4 {
                this.book.sheets[0].set(Pos::new(r, 0), sheet::Cell::input("x"));
            }
            // 同じ2〜3行目を9回まとめる → outlineLevel は 7 で止まる
            // (ECMA-376 の上限。本家の「最大8レベル」= 基底+7段と同じ意味)
            this.anchor = Some(Pos::new(1, 0));
            this.cursor = Pos::new(2, 0);
            this.sync_input();
            for _ in 0..9 {
                this.run_cmd("group", cx);
            }
            assert_eq!(this.sheet().row_outline.get(&1), Some(&7), "7段で止まらない");
            // 1段ほどく → 6
            this.run_cmd("ungroup", cx);
            assert_eq!(this.sheet().row_outline.get(&1), Some(&6), "ほどけない");
        });
    }

    #[gpui::test]
    fn 値だけをcsvに書き出せる(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, _cx| {
            this.book.sheets[0].set(Pos::new(0, 0), sheet::Cell::input("品名"));
            this.book.sheets[0].set(Pos::new(0, 1), sheet::Cell::input("値,段"));
            this.book.sheets[0].set(Pos::new(1, 0), sheet::Cell::input("鉛筆"));
            this.book.sheets[0].set(Pos::new(1, 1), sheet::Cell::input("=1+2"));
            recalc_book(&mut this.book, 0);
            let dir = std::env::temp_dir().join("calc-csv-test");
            std::fs::create_dir_all(&dir).unwrap();
            let p = dir.join("out.csv");
            this.write_csv(&p);
            let got = std::fs::read_to_string(&p).unwrap();
            assert!(got.starts_with('\u{feff}'), "BOM が無い");
            assert!(got.contains("品名,\"値,段\""), "区切りを含む欄が囲われていない: {got}");
            assert!(got.contains("鉛筆,3"), "式が計算値になっていない: {got}");
            std::fs::remove_dir_all(&dir).ok();
        });
    }

    #[gpui::test]
    fn ルールの管理で規則を選んで消せる(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            for (i, v) in ["10", "20"].iter().enumerate() {
                this.book.sheets[0].set(Pos::new(i as u32, 0), sheet::Cell::input(v));
            }
            this.anchor = Some(Pos::new(0, 0));
            this.cursor = Pos::new(1, 0);
            this.sync_input();
            this.cond_visual("cond-bar");
            this.cond_visual("cond-scale");
            assert_eq!(this.book.sheets[0].cond.len(), 2);
            // 1本目を選んで消す
            this.pick_kind = "cond-manage-pick";
            this.apply_pick("1) A1:A2 — データバー", cx);
            assert_eq!(this.pick_kind, "cond-act-pick", "2択が開かない");
            this.apply_pick("この規則を消す", cx);
            assert_eq!(this.book.sheets[0].cond.len(), 1, "消えていない");
            assert!(matches!(
                this.book.sheets[0].cond[0].kind,
                sheet::model::CondKind::Scale(..)
            ), "残る方が違う");
        });
    }

    #[gpui::test]
    fn データバーとスケールとアイコンをメニューから掛けられる(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            for (i, v) in ["10", "20", "30"].iter().enumerate() {
                this.book.sheets[0].set(Pos::new(i as u32, 0), sheet::Cell::input(v));
            }
            this.anchor = Some(Pos::new(0, 0));
            this.cursor = Pos::new(2, 0);
            this.sync_input();
            this.cond_visual("cond-bar");
            this.cond_visual("cond-scale");
            this.cond_visual("cond-icons");
            let cond = &this.book.sheets[0].cond;
            assert_eq!(cond.len(), 3, "3本入らない: {cond:?}");
            use sheet::model::CondKind;
            assert!(matches!(cond[0].kind, CondKind::Bar(_)));
            assert!(matches!(cond[1].kind, CondKind::Scale(..)));
            assert!(matches!(cond[2].kind, CondKind::Icons(_)));
        });
    }

    #[gpui::test]
    fn 合計行の集計のしかたを替えられる(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, _cx| {
            this.book.sheets[0].set(Pos::new(0, 0), sheet::Cell::input("10"));
            this.book.sheets[0].set(Pos::new(1, 0), sheet::Cell::input("30"));
            this.book.sheets[0].set(Pos::new(2, 0), sheet::Cell::input("=SUM(A1:A2)"));
            recalc_book(&mut this.book, 0);
            this.cursor = Pos::new(2, 0);
            this.sync_input();
            // 平均に替える
            this.set_subtotal_kind("1");
            let cell = this.book.sheets[0].get(Pos::new(2, 0)).unwrap();
            assert_eq!(cell.formula.as_deref(), Some("SUBTOTAL(1,A1:A2)"), "式が替わらない");
            assert_eq!(cell.value.display(), "20", "平均が出ない");
            // なし → 式が消えて書式は残る
            this.set_subtotal_kind("none");
            let gone = this.book.sheets[0]
                .get(Pos::new(2, 0))
                .is_none_or(|c| c.formula.is_none() && c.value.is_empty());
            assert!(gone, "式が消えない");
        });
    }

    #[gpui::test]
    fn 重複の削除は列と見出しの有無を選べる(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            // 品名は同じでも金額が違う2行 — 品名の列だけで比べれば重複
            for (r, (name, amt)) in
                [("品名", "金額"), ("鉛筆", "100"), ("鉛筆", "120"), ("消しゴム", "80")]
                    .iter()
                    .enumerate()
            {
                this.book.sheets[0].set(Pos::new(r as u32, 0), sheet::Cell::input(name));
                this.book.sheets[0].set(Pos::new(r as u32, 1), sheet::Cell::input(amt));
            }
            this.run_cmd("rem-duplicates", cx);
            assert_eq!(this.pick_kind, "dedup-pick", "選ぶ板が開かない");
            assert!(this.dedup_pend.is_some());
            // 「金額」を外す → 品名だけで比べる
            this.apply_pick("金額", cx);
            assert_eq!(this.pick_kind, "dedup-pick", "入切で板が閉じた");
            this.apply_pick("→ 削除する", cx);
            let s = &this.book.sheets[0];
            assert_eq!(s.get(Pos::new(1, 0)).unwrap().value.display(), "鉛筆");
            assert_eq!(s.get(Pos::new(2, 0)).unwrap().value.display(), "消しゴム");
            assert!(s.get(Pos::new(3, 0)).is_none(), "重複が消えていない");
            // 残るのは先に出てきた 100 の行
            assert_eq!(s.get(Pos::new(1, 1)).unwrap().value.display(), "100");
        });
    }

    #[gpui::test]
    fn リンクの後に表示テキストを聞かれてセルに入る(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            this.cursor = Pos::new(1, 1);
            this.sync_input();
            this.prompt = Some(("link", Editor::new("https://example.co.jp/")));
            this.finish_prompt(cx);
            assert!(this.sheet().links.contains_key(&Pos::new(1, 1)), "リンクが付かない");
            // 続けて表示テキストの板が開く
            assert_eq!(this.prompt.as_ref().map(|(k, _)| *k), Some("link-text"), "表示テキストの板が開かない");
            this.prompt = Some(("link-text", Editor::new("会社サイト")));
            this.finish_prompt(cx);
            let got = this.sheet().get(Pos::new(1, 1)).map(|c| c.value.display());
            assert_eq!(got.as_deref(), Some("会社サイト"), "表示テキストがセルに入らない");
            // 空 Enter ならそのまま
            this.prompt = Some(("link", Editor::new("#B9")));
            this.finish_prompt(cx);
            this.prompt = Some(("link-text", Editor::new("")));
            this.finish_prompt(cx);
            let got = this.sheet().get(Pos::new(1, 1)).map(|c| c.value.display());
            assert_eq!(got.as_deref(), Some("会社サイト"), "空 Enter でセルが変わった");
        });
    }

    #[gpui::test]
    fn 列の幅と行の高さを数で指定して既定にも戻せる(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            // B〜C 列に 12.5
            this.anchor = Some(Pos::new(0, 1));
            this.cursor = Pos::new(0, 2);
            this.sync_input();
            this.prompt = Some(("col-width", Editor::new("12.5")));
            this.finish_prompt(cx);
            assert_eq!(this.sheet().col_width.get(&1), Some(&12.5));
            assert_eq!(this.sheet().col_width.get(&2), Some(&12.5));
            // 範囲外は言い返す
            this.prompt = Some(("col-width", Editor::new("999")));
            this.finish_prompt(cx);
            assert!(this.prompt.is_some(), "範囲外の幅が通った");
            this.prompt = None;
            // 行の高さ
            this.anchor = None;
            this.cursor = Pos::new(3, 0);
            this.sync_input();
            this.prompt = Some(("row-height", Editor::new("30")));
            this.finish_prompt(cx);
            assert_eq!(this.sheet().row_height.get(&3), Some(&30.0));
            // 空 Enter = 既定に戻す
            this.anchor = Some(Pos::new(0, 1));
            this.cursor = Pos::new(0, 2);
            this.sync_input();
            this.prompt = Some(("col-width", Editor::new("")));
            this.finish_prompt(cx);
            assert!(this.sheet().col_width.get(&1).is_none(), "既定に戻らない");
        });
    }

    #[gpui::test]
    fn 名前マネージャーで移動と打ち直しと削除(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            // 名前を1つ付ける(B2:C3 = 単価表)
            this.anchor = Some(Pos::parse("B2").unwrap());
            this.cursor = Pos::parse("C3").unwrap();
            this.sync_input();
            this.prompt = Some(("name", Editor::new("単価表")));
            this.finish_prompt(cx);
            assert_eq!(this.sheet().names, vec![("単価表".into(), "B2:C3".into())]);
            // 一覧に出る
            this.anchor = None;
            this.cursor = Pos::parse("A1").unwrap();
            this.sync_input();
            this.run_cmd("defname", cx);
            assert_eq!(this.pick_kind, "names-pick");
            {
                let (items, _) = this.pick.as_ref().unwrap();
                assert!(items.iter().any(|i| i == "単価表 = B2:C3"), "{items:?}");
            }
            // 移動
            this.apply_pick("単価表 = B2:C3", cx);
            assert_eq!(this.pick_kind, "name-act-pick");
            this.apply_pick("そこへ移動", cx);
            assert_eq!(this.cursor, Pos::parse("C3").unwrap());
            assert_eq!(this.anchor, Some(Pos::parse("B2").unwrap()));
            // 打ち直し
            this.run_cmd("defname", cx);
            this.apply_pick("単価表 = B2:C3", cx);
            this.apply_pick("中身を打ち直す…", cx);
            this.prompt = Some(("name-range", Editor::new("B2:D9")));
            this.finish_prompt(cx);
            assert_eq!(this.sheet().names[0].1, "B2:D9");
            // 削除
            this.run_cmd("defname", cx);
            this.apply_pick("単価表 = B2:D9", cx);
            this.apply_pick("名前を消す", cx);
            assert!(this.sheet().names.is_empty(), "名前が消えない");
        });
    }

    #[gpui::test]
    fn ヘッダーとフッターを板から入れて消す(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            this.run_cmd("editheader", cx);
            assert_eq!(this.pick_kind, "hf-pick", "一覧が開かない");
            this.apply_pick("ヘッダー中", cx);
            assert!(this.prompt.is_some(), "板が開かない");
            this.prompt = Some(("hf-edit", Editor::new("月次売上")));
            this.finish_prompt(cx);
            assert_eq!(this.sheet().header.as_deref(), Some("&C月次売上"));
            // フッター右に頁(既存の値が一覧に見える)
            this.run_cmd("editheader", cx);
            {
                let (items, _) = this.pick.as_ref().unwrap();
                assert!(items.iter().any(|i| i == "ヘッダー中: 月次売上"), "{items:?}");
            }
            this.apply_pick("フッター右: ", cx); // 値の付いた札でも先頭で見分ける
            this.prompt = Some(("hf-edit", Editor::new("&P / &N")));
            this.finish_prompt(cx);
            assert_eq!(this.sheet().footer.as_deref(), Some("&R&P / &N"));
            // 全部消す
            this.run_cmd("editheader", cx);
            this.apply_pick("全部消す", cx);
            assert!(this.sheet().header.is_none() && this.sheet().footer.is_none());
        });
    }

    #[gpui::test]
    fn 色のその他と文字の角度の直指定(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            let a1 = Pos::parse("A1").unwrap();
            this.cursor = a1;
            this.sync_input();
            this.input.insert("x");
            assert!(this.commit());
            // 文字の色: その他 → RRGGBB
            this.run_cmd("fontcolor", cx);
            this.apply_pick("その他(RRGGBB を打つ)…", cx);
            this.prompt = Some(("font-color-rgb", Editor::new("00B050")));
            this.finish_prompt(cx);
            assert_eq!(
                this.sheet().get(a1).unwrap().fmt.color.as_deref(),
                Some("00B050"),
                "文字の色の直指定が効かない"
            );
            // 塗り: その他 → RRGGBB
            this.run_cmd("fillparag", cx);
            this.apply_pick("その他(RRGGBB を打つ)…", cx);
            this.prompt = Some(("fill-color-rgb", Editor::new("FFF2CC")));
            this.finish_prompt(cx);
            assert_eq!(
                this.sheet().get(a1).unwrap().fmt.fill.as_deref(),
                Some("FFF2CC")
            );
            // 角度: 一覧のプリセット
            this.run_cmd("text-orient", cx);
            assert_eq!(this.pick_kind, "orient-pick");
            this.apply_pick("左上がり 45度", cx);
            assert_eq!(this.sheet().get(a1).unwrap().fmt.rotation, Some(45));
            // 任意の角度(負は xlsx の encode で 90+|d|)
            this.run_cmd("text-orient", cx);
            this.apply_pick("その他(角度を打つ)…", cx);
            this.prompt = Some(("text-angle", Editor::new("-30")));
            this.finish_prompt(cx);
            assert_eq!(
                this.sheet().get(a1).unwrap().fmt.rotation,
                Some(120),
                "負の角度の encode が違う"
            );
            // 範囲外は言い返す
            this.prompt = Some(("text-angle", Editor::new("200")));
            this.finish_prompt(cx);
            assert!(this.prompt.is_some(), "範囲外の角度が通った");
            assert!(this.status.contains("角度が読めません"));
        });
    }

    #[gpui::test]
    fn 罫線はペンの線種と色で掛かる(cx: &mut gpui::TestAppContext) {
        use sheet::model::BStyle;
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            // 一覧が開き、ペンを 中太の実線・赤 にする
            this.anchor = Some(Pos::parse("B2").unwrap());
            this.cursor = Pos::parse("C3").unwrap();
            this.sync_input();
            this.run_cmd("borders", cx);
            assert!(this.border_pal.is_some(), "罫線のパレットが開かない");
            this.open_border_style_pick();
            assert_eq!(this.pick_kind, "border-style-pick");
            this.apply_pick("中太の実線", cx);
            assert_eq!(this.pen_style, BStyle::Medium);
            this.open_border_color_pick();
            this.apply_pick("その他(RRGGBB を打つ)…", cx);
            this.prompt = Some(("border-color-rgb", Editor::new("FF0000")));
            this.finish_prompt(cx);
            assert_eq!(this.pen_color, Some(0xFF0000), "RGB 直指定が効かない");
            // 外枠を掛ける(パレットの1押しと同じ実体)
            this.apply_borders("外枠");
            let bd = |this: &Calc, p: &str| {
                this.sheet().get(Pos::parse(p).unwrap()).unwrap().fmt.borders
            };
            let b2 = bd(this, "B2");
            assert!(b2.top.on && b2.left.on, "外枠の左上が付かない");
            assert!(!b2.bottom.on && !b2.right.on, "外枠なのに内側に付いた");
            assert_eq!(b2.top.style, BStyle::Medium);
            assert_eq!(b2.top.color, Some(0xFF0000));
            let c3 = bd(this, "C3");
            assert!(c3.bottom.on && c3.right.on && !c3.top.on);
            // 格子 → 全辺。消す → 全部消える
            this.apply_borders("すべての罫線(格子)");
            assert!(bd(this, "B2").right.on && bd(this, "C3").top.on);
            this.apply_borders("罫線を消す");
            // 素に戻ったセルは片づけられる(get は None)— どちらでも「無い」
            let off = |this: &Calc, p: &str| {
                this.sheet()
                    .get(Pos::parse(p).unwrap())
                    .map(|c| c.fmt.borders.any())
                    .unwrap_or(false)
            };
            assert!(!off(this, "B2") && !off(this, "C3"), "罫線が消えない");
        });
    }

    #[gpui::test]
    fn セルの上のbackspaceとdeleteは中身を消す(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, _cx| {
            let a1 = Pos::parse("A1").unwrap();
            this.cursor = a1;
            this.sync_input();
            this.input.insert("こんにちは");
            assert!(this.commit());
            this.fmt(|f| f.bold = true);
            this.sync_input();
            // セルの上(編集していない)= まるごと消す。書式は残る
            assert!(!this.editing() && !this.edit_armed);
            this.clear_selection_now();
            let cell = this.sheet().get(a1).cloned().unwrap_or_default();
            assert_eq!(cell.editable(), "", "中身が消えない");
            assert!(cell.fmt.bold, "書式まで消えた");
            assert_eq!(this.input.text(), "", "数式バーが残っている");
            // 編集中の1文字削除は従来どおり(こちらは Editor の仕事)
            this.input.insert("abc");
            this.edit_armed = true;
            this.input.backspace();
            assert_eq!(this.input.text(), "ab");
        });
    }

    #[gpui::test]
    fn 結合は聞かずに掛かり左上以外の値は消える(cx: &mut gpui::TestAppContext) {
        let c = cx.update(|cx| cx.new(|cx| Calc::new(None, cx)));
        c.update(cx, |this, cx| {
            for (p, v) in [("A1", "甲"), ("B2", "乙")] {
                this.cursor = Pos::parse(p).unwrap();
                this.sync_input();
                this.input.insert(v);
                assert!(this.commit());
            }
            // 聞かずに結合し、左上以外の値は**消す**(発注者 2026-08-08 —
            // 残すと見えない値が SUM などの式に効く)。Ctrl+Z で戻せる
            this.anchor = Some(Pos::parse("A1").unwrap());
            this.cursor = Pos::parse("B2").unwrap();
            this.run_cmd("merge", cx);
            assert_eq!(this.pick_kind, "merge-pick", "4択が出ない");
            this.apply_pick("結合して中央に配置", cx);
            assert_eq!(this.sheet().merges.len(), 1, "確認を挟まず結合されるべき");
            assert!(this.status.contains("左上以外の値は消しました"), "案内が無い: {}", this.status);
            assert!(
                this.sheet()
                    .get(Pos::parse("B2").unwrap())
                    .is_none_or(|c| c.value.is_empty()),
                "呑まれた値が残っている(式に効いてしまう)"
            );
            // SUM が隠れた値を数えない
            this.cursor = Pos::parse("D5").unwrap();
            this.sync_input();
            this.input = Editor::new("=SUM(A1:B2)");
            assert!(this.commit());
            assert_eq!(
                this.sheet().get(Pos::parse("D5").unwrap()).unwrap().value.display(),
                "0", "隠れた値が SUM に効いている"
            );
            // Ctrl+Z で結合前に戻る(乙も戻る)
            this.undo_sheet(); // SUM の確定ぶん
            this.undo_sheet(); // 結合ぶん
            assert_eq!(
                this.sheet().get(Pos::parse("B2").unwrap()).unwrap().editable(),
                "乙", "Ctrl+Z で値が戻らない"
            );
            // やり直して続きの検査へ
            this.anchor = Some(Pos::parse("A1").unwrap());
            this.cursor = Pos::parse("B2").unwrap();
            this.run_cmd("merge", cx);
            this.apply_pick("結合して中央に配置", cx);
            // 空の範囲も同じくそのまま
            this.anchor = Some(Pos::parse("D1").unwrap());
            this.cursor = Pos::parse("E2").unwrap();
            this.run_cmd("merge", cx);
            this.apply_pick("結合して中央に配置", cx);
            assert_eq!(this.sheet().merges.len(), 2);
            // 横方向: 行ごとに1本ずつ
            this.anchor = Some(Pos::parse("G1").unwrap());
            this.cursor = Pos::parse("H3").unwrap();
            this.sync_input();
            this.run_cmd("merge", cx);
            this.apply_pick("横方向に結合(行ごと)", cx);
            assert_eq!(this.sheet().merges.len(), 5, "横方向が行ごとにならない");
            // 解除: 選択に重なる結合をまとめて外す
            this.run_cmd("merge", cx);
            this.apply_pick("結合の解除", cx);
            assert_eq!(this.sheet().merges.len(), 2, "解除で消えない");
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
                name: "ピボットテーブル1".into(),
                vfilter: None,
                group_by: Vec::new(),
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
            // 実利用者の settings.toml の値に左右されない(試験は素の 100% から)
            this.ui_scale = 1.0;
            let base = this.ui_scale;
            this.run_cmd("ui-bigger", cx);
            assert!(this.ui_scale > base, "大きくならない");
            for _ in 0..30 {
                this.run_cmd("ui-bigger", cx);
            }
            assert_eq!(this.ui_scale, 1.5, "上の端(150%)で止まらない");
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
            this.merge_selection("中央");
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

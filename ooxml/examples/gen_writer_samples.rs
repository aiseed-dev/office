// writer のサンプル文書(docx)をまとめて作る。中身はすべて架空。
//
//   cargo run -p ooxml --example gen_writer_samples
//
// **うちのモデルで書く。** ルビ・縦書き・記入欄・均等割付は python-docx では
// 素直に書けないので、gen_docs.py(python-docx 側)とは別に用意する。
// サンプルは生成物 — 直すのはこのファイル。
use kumihan::{
    Align, Block, CharFormat, Comment, Document, HeadFoot, ListKind, PageSetup,
    Paragraph, ParaStyle, Run, Sdt, SdtKind, Table, Cellbox, PAGE_MARK,
};

const PT: f32 = 10.5;

fn run(text: &str) -> Run {
    Run { text: text.into(), size_pt: PT, font: None, fmt: CharFormat::default() }
}

fn run_fmt(text: &str, f: CharFormat) -> Run {
    Run { text: text.into(), size_pt: PT, font: None, fmt: f }
}

fn para(runs: Vec<Run>) -> Paragraph {
    Paragraph { runs, line_spacing: 1.0, ..Default::default() }
}

fn p(text: &str) -> Paragraph {
    para(vec![run(text)])
}

fn heading(n: u8, text: &str) -> Paragraph {
    Paragraph { style: ParaStyle::Heading(n), ..p(text) }
}

fn bullet(text: &str) -> Paragraph {
    Paragraph { list: ListKind::Bullet, ..p(text) }
}

/// ルビ付きの run(基底に読みを振る)
fn ruby(text: &str, yomi: &str) -> Run {
    run_fmt(text, CharFormat { ruby: Some(yomi.into()), ..Default::default() })
}

/// 記入欄の run
fn field(text: &str, kind: SdtKind, alias: &str, items: &[&str]) -> Run {
    run_fmt(
        text,
        CharFormat {
            sdt: Some(Box::new(Sdt {
                kind,
                alias: alias.into(),
                tag: kind.as_tag().into(),
                items: items.iter().map(|s| s.to_string()).collect(),
            })),
            ..Default::default()
        },
    )
}

fn cell(text: &str) -> Cellbox {
    Cellbox { paragraphs: vec![p(text)], ..Default::default() }
}

fn bold_cell(text: &str) -> Cellbox {
    Cellbox {
        paragraphs: vec![para(vec![run_fmt(
            text,
            CharFormat { bold: true, ..Default::default() },
        )])],
        ..Default::default()
    }
}

fn save(name: &str, doc: &Document) {
    let path = format!("sample/writer/{name}");
    let f = std::fs::File::create(&path).expect("作れません");
    ooxml::write(doc, std::io::BufWriter::new(f)).expect("書けません");
    println!("{path}");
}

// ---- 1. 日本語の組版(ルビ・均等割付・禁則・段組み・ドロップキャップ) ----
fn kumihan_sample() -> Document {
    let mut d = Document::plain("", PT);
    d.blocks.clear();
    d.props.title = "日本語の組版".into();
    d.props.creator = "aiseed office".into();

    d.blocks.push(Block::Para(heading(1, "日本語の組版")));
    d.blocks.push(Block::Para(Paragraph {
        dropcap: true,
        ..p("この文書は writer の組版を見るための見本です。頭の一字が大きいのは\
             ドロップキャップ、行の折り返しでは句読点が行頭に来ない禁則処理が\
             効いています。中身はすべて架空です。")
    }));

    d.blocks.push(Block::Para(heading(2, "ルビ(ふりがな)")));
    d.blocks.push(Block::Para(para(vec![
        run("難しい語には "),
        ruby("組版", "くみはん"),
        run(" や "),
        ruby("禁則処理", "きんそくしょり"),
        run(" のようにルビを振れます。ルビは基底の字の上に半分の大きさで、\
             狭ければ字間を等しく配って中付きにします。"),
    ])));

    d.blocks.push(Block::Para(heading(2, "均等割付")));
    d.blocks.push(Block::Para(p(
        "下の3行は均等割付です。字数が違っても行の端から端まで届きます — \
         様式の項目名を揃えるときの作法です。",
    )));
    for name in ["氏名", "生年月日", "現住所"] {
        d.blocks.push(Block::Para(Paragraph {
            align: Align::Distribute,
            indent: 1,
            ..p(name)
        }));
    }

    d.blocks.push(Block::Para(heading(2, "箇条書きと字下げ")));
    for t in [
        "禁則は行を折る瞬間に解決する(後から字を送ると行長を超える)",
        "欧文は語の途中で折らない。ハイフネーションは設定で入切する",
        "行の高さ・字幅は実際のフォントから測る — 画面と紙が一致する",
    ] {
        d.blocks.push(Block::Para(bullet(t)));
    }

    d.blocks.push(Block::Para(heading(2, "英文のハイフネーション")));
    d.blocks.push(Block::Para(p(
        "Typography is the art and technique of arranging type to make written \
         language legible, readable and appealing when displayed. \
         レイアウト > ハイフン設定の変更 で入切できます。",
    )));

    d.blocks.push(Block::Para(Paragraph {
        comments: vec![Comment {
            author: "校閲".into(),
            text: "この段落は段組み(2段)の見本です".into(),
        }],
        ..p("レイアウト > 列の挿入 で段組みにすると、この文書は2段で組み直され\
             ます。段組みは行を細い行長で組んでから、ページの物理座標へ折る\
             作りなので、画面もPDFも同じ紙面になります。")
    }));
    d
}

// ---- 2. 縦書きの手紙 ----
fn tategaki_sample() -> Document {
    let mut d = Document::plain("", PT);
    d.blocks.clear();
    d.vertical = true;
    d.props.title = "縦書きの見本".into();

    d.blocks.push(Block::Para(p("拝啓　新緑の候、貴社ますますご清栄のこととお慶び申し上げます。")));
    d.blocks.push(Block::Para(p("平素は格別のご高配を賜り、厚く御礼申し上げます。")));
    d.blocks.push(Block::Para(para(vec![
        run("さて、かねてよりご相談いただいておりました "),
        ruby("組版", "くみはん"),
        run(" の件につきまして、下記のとおりご案内申し上げます。"),
    ])));
    d.blocks.push(Block::Para(p("「右の列から左へ、字は上から下へ」と組まれ、句読点や\
                                 かぎ括弧は縦用の形になります。")));
    d.blocks.push(Block::Para(Paragraph { align: Align::Center, ..p("記") }));
    d.blocks.push(Block::Para(bullet("日時　八月十日(月)　午後二時")));
    d.blocks.push(Block::Para(bullet("場所　本社　三階　会議室")));
    d.blocks.push(Block::Para(Paragraph { align: Align::Right, ..p("以上") }));
    d.blocks.push(Block::Para(Paragraph { align: Align::Right, ..p("敬具") }));
    d
}

// ---- 3. 申込書(表と記入欄) ----
fn moushikomi_sample() -> Document {
    let mut d = Document::plain("", PT);
    d.blocks.clear();
    d.props.title = "講習会 申込書".into();

    d.blocks.push(Block::Para(Paragraph {
        align: Align::Center,
        ..heading(1, "講習会　参加申込書")
    }));
    d.blocks.push(Block::Para(p(
        "薄い箱が記入欄(コンテンツコントロール)です。中は普通に打てます。\
         チェックはフォームタブの同じ釦で ☐ と ☑ が入れ替わり、\
         選ぶ欄は同じ釦で次の選択肢へ回ります。",
    )));

    d.blocks.push(Block::Table(Table {
        col_mm: vec![35.0, 125.0],
        rows: vec![
            vec![
                bold_cell("お名前"),
                Cellbox {
                    paragraphs: vec![para(vec![field(
                        "　　　　　　",
                        SdtKind::Text,
                        "氏名",
                        &[],
                    )])],
                    ..Default::default()
                },
            ],
            vec![
                bold_cell("メール"),
                Cellbox {
                    paragraphs: vec![para(vec![field(
                        "　　　　　　",
                        SdtKind::Email,
                        "メール",
                        &[],
                    )])],
                    ..Default::default()
                },
            ],
            vec![
                bold_cell("電話"),
                Cellbox {
                    paragraphs: vec![para(vec![field(
                        "　　　　　　",
                        SdtKind::Phone,
                        "電話",
                        &[],
                    )])],
                    ..Default::default()
                },
            ],
            vec![
                bold_cell("参加する回"),
                Cellbox {
                    paragraphs: vec![para(vec![field(
                        "第一回",
                        SdtKind::Dropdown,
                        "回",
                        &["第一回", "第二回", "第三回"],
                    )])],
                    ..Default::default()
                },
            ],
            vec![
                bold_cell("資料の送付"),
                Cellbox {
                    paragraphs: vec![para(vec![
                        field("☐", SdtKind::Checkbox, "郵送", &[]),
                        run(" 郵送を希望する　"),
                        field("☑", SdtKind::Checkbox, "電子", &[]),
                        run(" 電子で受け取る"),
                    ])],
                    ..Default::default()
                },
            ],
            vec![
                bold_cell("ご署名"),
                Cellbox {
                    paragraphs: vec![para(vec![field(
                        "　　　　　　",
                        SdtKind::Signature,
                        "署名",
                        &[],
                    )])],
                    ..Default::default()
                },
            ],
        ],
    }));
    d.blocks.push(Block::Para(Paragraph {
        boxed: true,
        ..p("記入したら「ファイル > 名前を付けて保存」で docx として保存します。\
             保護タブの「保護」で読み取り専用にすると、配った後で書き換えられ\
             ません(パスワードは掛けません — 掛けた振りもしません)。")
    }));
    d
}

// ---- 4. 報告書(見出し・目次・ヘッダー・フッター・表) ----
fn houkoku_sample() -> Document {
    let mut d = Document::plain("", PT);
    d.blocks.clear();
    d.props.title = "月次報告".into();
    d.props.creator = "aiseed office".into();
    d.props.subject = "見本".into();

    // ヘッダーとフッター(フッターはページ番号)
    d.header = HeadFoot {
        paragraphs: vec![Paragraph { align: Align::Right, ..p("月次報告(見本)") }],
        part: None,
    };
    d.footer = HeadFoot {
        paragraphs: vec![Paragraph {
            align: Align::Center,
            ..para(vec![run("- "), run(&PAGE_MARK.to_string()), run(" -")])
        }],
        part: None,
    };

    d.blocks.push(Block::Para(heading(1, "月次報告(2026年7月)")));
    d.blocks.push(Block::Para(p(
        "参考資料 > 目次 を押すと、この文書の見出しから目次が作られます。\
         ページ番号は紙と同じ折り方で数えるので、PDF と食い違いません。",
    )));

    d.blocks.push(Block::Para(heading(2, "概況")));
    d.blocks.push(Block::Para(Paragraph {
        bookmarks: vec!["概況".into()],
        ..p("受注は3件、見積提出は5件。8月は資材の手配が山になる見込みです。\
             参考資料 > 相互参照 で、この段落を「概況」として参照できます。")
    }));

    d.blocks.push(Block::Para(heading(2, "実績")));
    d.blocks.push(Block::Table(Table {
        col_mm: vec![30.0, 70.0, 30.0, 30.0],
        rows: vec![
            vec![
                bold_cell("受注日"),
                bold_cell("件名"),
                bold_cell("発注元"),
                bold_cell("金額"),
            ],
            vec![cell("7月3日"), cell("外壁の塗り替え"), cell("A社"), cell("820,000")],
            vec![cell("7月14日"), cell("屋根の点検"), cell("B社"), cell("135,000")],
            vec![cell("7月28日"), cell("足場の設置"), cell("C社"), cell("460,000")],
        ],
    }));

    d.blocks.push(Block::Para(heading(2, "所見")));
    d.blocks.push(Block::Para(Paragraph {
        comments: vec![Comment {
            author: "課長".into(),
            text: "来月は人の手配も書くこと".into(),
        }],
        shade: Some("EAF2F7".into()),
        ..p("引き合いが続いており、8月は資材と人の手配が要ります。\
             この段落には帯(背景色)とコメントが付いています。")
    }));

    d.blocks.push(Block::Para(heading(2, "配布のしかた")));
    for t in [
        "そのまま docx で配る(Word でも LibreOffice でも開きます)",
        "ファイル > 印刷 で PDF にする(画面と同じ紙面が出ます)",
        "保護タブの 暗号化する でパスワードを掛けて配る",
    ] {
        d.blocks.push(Block::Para(bullet(t)));
    }
    d
}

// ---- 5. 道具くらべ(釦を試すための的) ----
// 「押してみる」ための材料を1枚に集める。試験(menu_run_tests)は
// この文書も開いて全部の釦を通す
fn dougu_sample() -> Document {
    let mut d = Document::plain("", PT);
    d.blocks.clear();
    d.props.title = "道具くらべ".into();

    d.blocks.push(Block::Para(heading(1, "道具くらべ — 釦を試す紙")));
    d.blocks.push(Block::Para(p(
        "リボンの釦を順に押して確かめるための紙です。どの釦も1手で戻せます         (Ctrl+Z)。中身はすべて架空です。",
    )));

    d.blocks.push(Block::Para(heading(2, "字と段落")));
    d.blocks.push(Block::Para(para(vec![
        run("この行を選んで "),
        run_fmt("太字", CharFormat { bold: true, ..Default::default() }),
        run("・"),
        run_fmt("斜体", CharFormat { italic: true, ..Default::default() }),
        run("・"),
        run_fmt("下線", CharFormat { underline: true, ..Default::default() }),
        run("・"),
        run_fmt(
            "蛍光ペン",
            CharFormat { highlight: Some("yellow".into()), ..Default::default() },
        ),
        run(" を試します。"),
    ])));
    d.blocks.push(Block::Para(Paragraph {
        align: Align::Center,
        ..p("中央揃えの行(揃えの釦で左・右・両端・均等に変えられます)")
    }));
    d.blocks.push(Block::Para(Paragraph {
        list: ListKind::Number,
        ..p("番号の付いた行。Tab で深さが変わります")
    }));

    d.blocks.push(Block::Para(heading(2, "参考資料の的")));
    d.blocks.push(Block::Para(Paragraph {
        bookmarks: vec!["的".into()],
        ..p("この段落には「的」というしおりが付いています — 相互参照の             行き先として使えます。参考資料 > 目次 と 図表目次 もこの紙で試せます。")
    }));
    d.blocks.push(Block::Para(Paragraph {
        align: Align::Center,
        ..p("図 1　図表番号の見本")
    }));

    d.blocks.push(Block::Para(heading(2, "レビューと共同編集の的")));
    d.blocks.push(Block::Para(Paragraph {
        comments: vec![Comment {
            author: "確認".into(),
            text: "コメントの表示・削除を試す的".into(),
        }],
        ..p("この段落にはコメントが付いています(コメントの表示で出し入れ)。             変更履歴を入れてから字を直し、保存すると w:ins / w:del になります。")
    }));

    d.blocks.push(Block::Para(heading(2, "表")));
    d.blocks.push(Block::Table(Table {
        col_mm: vec![40.0, 60.0, 40.0],
        rows: vec![
            vec![bold_cell("道具"), bold_cell("試すこと"), bold_cell("戻し方")],
            vec![cell("ペン"), cell("紙に線を引く"), cell("Ctrl+Z")],
            vec![cell("透かし"), cell("字を入れて紙に薄く出す"), cell("空にする")],
            vec![cell("段組み"), cell("2段に組み直す"), cell("もう一度押す")],
        ],
    }));
    d
}

fn main() {
    let _ = std::fs::create_dir_all("sample/writer");
    let mut a4 = PageSetup::default();
    a4.left_mm = 25.0;
    a4.right_mm = 25.0;

    for (name, mut doc) in [
        ("01_日本語の組版.docx", kumihan_sample()),
        ("02_縦書きの手紙.docx", tategaki_sample()),
        ("03_申込書.docx", moushikomi_sample()),
        ("04_月次報告.docx", houkoku_sample()),
        ("05_道具くらべ.docx", dougu_sample()),
    ] {
        doc.page = Some(a4);
        save(name, &doc);
    }
}

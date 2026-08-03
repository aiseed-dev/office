//! K0 の目視検証: kumihan が組んだ紙面を、座標そのままに A4 PDF へ写す。
//! 画面もこれと同じことをする(別の面に写すだけ)。
//!
//!   cargo run -p kumihan --example page -- 出力.pdf

use kumihan::{layout, Block, Document, Frame, Metrics, Paragraph, Run};
use printpdf::*;
use std::fs::File;
use std::io::BufWriter;

const MARGIN: f32 = 20.0;

/// 本文のフォント。同梱せず、システムから探す
fn font_data() -> &'static [u8] {
    static FONT: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    FONT.get_or_init(|| {
        let (fam, _) = kumihan::font::for_document(None).expect("日本語フォントが要る");
        kumihan::font::load(fam).expect("読めない")
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args().nth(1).unwrap_or_else(|| "page.pdf".into());
    let m = Metrics::new(font_data()).map_err(|e| e.to_string())?;

    let doc = Document {
        font: None,
        page: None,
        sect_raw: None, header: Default::default(), footer: Default::default(), page_color: None, watermark: None,
        blocks: vec![
            Block::Para(Paragraph { align: Default::default(), style: Default::default(), comments: Vec::new(), anchors: Vec::new(),
                    images: Vec::new(), page_break_before: false,
                    list: Default::default(), indent: 0, line_spacing: 1.0, shade: None, boxed: false, images_new: Vec::new(), runs: vec![Run {
                fmt: Default::default(),
                font: None,
                text: "Japanese-Office 組版エンジン(K0)".into(), size_pt: 16.0 }] }),
            Block::Para(Paragraph { align: Default::default(), style: Default::default(), comments: Vec::new(), anchors: Vec::new(),
                    images: Vec::new(), page_break_before: false,
                    list: Default::default(), indent: 0, line_spacing: 1.0, shade: None, boxed: false, images_new: Vec::new(), runs: vec![Run { font: None, fmt: Default::default(), text: "".into(), size_pt: 10.5 }] }),
            Block::Para(Paragraph { align: Default::default(), style: Default::default(), comments: Vec::new(), anchors: Vec::new(),
                    images: Vec::new(), page_break_before: false,
                    list: Default::default(), indent: 0, line_spacing: 1.0, shade: None, boxed: false, images_new: Vec::new(), runs: vec![Run { font: None, fmt: Default::default(), text:
                "日本の事務の実態は、文書ではなく様式です。その様式の定義をテキストにして、\
                 記入用の帳票・検証・データベースを全部そこから派生させます。\
                 「原本はテキスト。」と、私たちは Rust で書きます。".into(),
                size_pt: 10.5 }] }),
            Block::Para(Paragraph { align: Default::default(), style: Default::default(), comments: Vec::new(), anchors: Vec::new(),
                    images: Vec::new(), page_break_before: false,
                    list: Default::default(), indent: 0, line_spacing: 1.0, shade: None, boxed: false, images_new: Vec::new(), runs: vec![Run { font: None, fmt: Default::default(), text:
                "この紙面は kumihan が実フォントの字幅で組んだ座標を、そのまま PDF に\
                 写したものです。行頭に句読点(、や。)や閉じ括弧が来ないこと、\
                 行末に開き括弧(「や()が残らないこと、英単語 Rust や TypeScript が\
                 行の途中で割れないことを、テストと目視の両方で確かめます。".into(),
                size_pt: 10.5 }] }),
            Block::Para(Paragraph { align: Default::default(), style: Default::default(), comments: Vec::new(), anchors: Vec::new(),
                    images: Vec::new(), page_break_before: false,
                    list: Default::default(), indent: 0, line_spacing: 1.0, shade: None, boxed: false, images_new: Vec::new(), runs: vec![Run { font: None, fmt: Default::default(), text:
                "禁則の検査: あいうえおかきくけこ、さしすせそ。「たちつてと」と\
                 (なにぬねの)!? 数字 2026 年 8 月 2 日、単位 25.4mm/72pt。".into(),
                size_pt: 10.5 }] }),
        ],
    };

    let frame = Frame { measure_mm: 210.0 - 2.0 * MARGIN, line_height_mm: 6.4, y0_mm: 24.0 };
    let sheet = layout(&doc, &m, &frame);

    let (pdf, page, layer) = PdfDocument::new("kumihan K0", Mm(210.0), Mm(297.0), "L1");
    let font = pdf.add_external_font(std::io::Cursor::new(font_data()))?;
    let l = pdf.get_page(page).get_layer(layer);

    for line in &sheet.lines {
        // 1行は同一サイズ(v0)。行内の字は同じベースラインに並ぶ
        let size = line.cells[0].size_pt;
        let x = MARGIN + line.cells[0].x_mm;
        let y = 297.0 - line.y_mm;
        l.use_text(line.text(), size, Mm(x), Mm(y), &font);
    }
    // 版面の枠(検証用: 行がこの中に収まっているか)
    let rect = [
        (MARGIN, 297.0 - 14.0), (210.0 - MARGIN, 297.0 - 14.0),
        (210.0 - MARGIN, 40.0), (MARGIN, 40.0),
    ];
    l.set_outline_thickness(0.3);
    l.add_line(printpdf::Line {
        points: rect.iter().map(|(x, y)| (Point::new(Mm(*x), Mm(*y)), false)).collect(),
        is_closed: true,
    });

    pdf.save(&mut BufWriter::new(File::create(&out)?))?;
    eprintln!("{} 行を印字: {}", sheet.lines.len(), out);
    Ok(())
}

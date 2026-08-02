fn main() {
    let (fam, _) = kumihan::font::for_document(None).unwrap();
    let data = kumihan::font::load(fam).unwrap();
    let m = kumihan::Metrics::new(&data).unwrap();
    let mut d = kumihan::Document::plain(
        "実施要領に基づく提案書\n\
         日本の事務の書類は、表題を中央に置き、本文を両端で揃える。\n\
         この紙面は画面に出しているものと同じ座標をそのまま写している。", 10.5);
    d.apply_align(0.."実施要領に基づく提案書".len(), kumihan::Align::Center);
    d.apply_size(0.."実施要領に基づく提案書".len(), |_| 16.0);
    d.apply_char_format(0.."実施要領に基づく提案書".len(), |f| f.bold = true);
    let s = kumihan::layout(&d, &m, &kumihan::Frame { measure_mm: 170.0, line_height_mm: 6.4, y0_mm: 24.0 });
    let f = std::fs::File::create(std::env::args().nth(1).unwrap()).unwrap();
    paper::to_pdf(&s, &data, paper::Paper::default(), std::io::BufWriter::new(f)).unwrap();
    println!("{} 行を印字。表題: 中央・16pt・太字", s.lines.len());
}

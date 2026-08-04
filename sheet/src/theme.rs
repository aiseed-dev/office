//! テーマ色(`xl/theme/theme1.xml` の色の組)。
//!
//! xlsx のセルは色を `rgb="FF4472C4"` と直に書くほかに、
//! **テーマの何番目** + **明るさの加減(tint)** で書ける
//! (`<color theme="4" tint="0.4"/>`)。これを読めないと、
//! 今どきの Excel で作った帳票の色が画面から消える — だから読む。
//!
//! 番号の並びは Excel の `theme=` の流儀:
//! 0=背景1 1=文字1 2=背景2 3=文字2 4〜9=アクセント1〜6 10=リンク 11=既読リンク。
//! (theme1.xml の中の並びは dk1,lt1,dk2,lt2,… なので読むときに入れ替える)

/// 既定(Office)の色の組。テーマの部品が無いブックはこれを使う
pub const OFFICE: [&str; 12] = [
    "FFFFFF", "000000", "E7E6E6", "44546A", "4472C4", "ED7D31", "A5A5A5", "FFC000",
    "5B9BD5", "70AD47", "0563C1", "954F72",
];

/// 名前つきの色の組(配色の変更で選ぶ)。名前は Euro-Office の言い方に寄せた
pub const SCHEMES: &[(&str, [&str; 12])] = &[
    ("Office", OFFICE),
    (
        "暖色",
        [
            "FFFFFF", "000000", "F5EDE6", "6B4A2F", "C0504D", "E36C0A", "D99694", "F0A22E",
            "E8B04B", "9C6644", "9E3A26", "7F3F2E",
        ],
    ),
    (
        "寒色",
        [
            "FFFFFF", "000000", "E8EEF4", "1F3864", "2E75B6", "41A5B5", "8FAADC", "70AD47",
            "4472C4", "255E91", "1F4E79", "3B5F8A",
        ],
    ),
    (
        "墨",
        [
            "FFFFFF", "000000", "EDEDED", "3B3B3B", "595959", "808080", "A6A6A6", "BFBFBF",
            "404040", "737373", "1B6E3C", "5A5A5A",
        ],
    ),
];

/// `theme1.xml` から色の組を読む。読めない部分は Office の色で埋める。
pub fn parse(xml: &str) -> Vec<String> {
    // clrScheme の中を順に拾う: dk1, lt1, dk2, lt2, accent1..6, hlink, folHlink
    let body = match (xml.find("<a:clrScheme"), xml.find("</a:clrScheme>")) {
        (Some(i), Some(j)) if j > i => &xml[i..j],
        _ => return OFFICE.iter().map(|s| s.to_string()).collect(),
    };
    let one = |tag: &str| -> Option<String> {
        let i = body.find(&format!("<a:{tag}>"))?;
        let rest = &body[i..];
        let end = rest.find(&format!("</a:{tag}>"))?;
        let seg = &rest[..end];
        // srgbClr val="RRGGBB" か、sysClr lastClr="RRGGBB"
        for key in ["srgbClr val=\"", "sysClr lastClr=\"", "lastClr=\""] {
            if let Some(a) = seg.find(key) {
                let a = a + key.len();
                if let Some(b) = seg[a..].find('"') {
                    let v = &seg[a..a + b];
                    if v.len() == 6 {
                        return Some(v.to_uppercase());
                    }
                }
            }
        }
        None
    };
    let g = |tag: &str, dflt: &str| one(tag).unwrap_or_else(|| dflt.to_string());
    // theme= の番号の並びへ入れ替える(背景1=lt1 が 0 番)
    vec![
        g("lt1", OFFICE[0]),
        g("dk1", OFFICE[1]),
        g("lt2", OFFICE[2]),
        g("dk2", OFFICE[3]),
        g("accent1", OFFICE[4]),
        g("accent2", OFFICE[5]),
        g("accent3", OFFICE[6]),
        g("accent4", OFFICE[7]),
        g("accent5", OFFICE[8]),
        g("accent6", OFFICE[9]),
        g("hlink", OFFICE[10]),
        g("folHlink", OFFICE[11]),
    ]
}

/// テーマの部品(書き出し用)。読んだ色の組をそのまま返す最小の形。
pub fn to_xml(colors: &[String]) -> String {
    let c = |i: usize| colors.get(i).map(|s| s.as_str()).unwrap_or(OFFICE[i]);
    // theme1.xml の中の並びは dk1, lt1, dk2, lt2, accent…(番号とは逆)
    format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office">"#,
            r#"<a:themeElements><a:clrScheme name="Office">"#,
            r#"<a:dk1><a:srgbClr val="{dk1}"/></a:dk1><a:lt1><a:srgbClr val="{lt1}"/></a:lt1>"#,
            r#"<a:dk2><a:srgbClr val="{dk2}"/></a:dk2><a:lt2><a:srgbClr val="{lt2}"/></a:lt2>"#,
            r#"<a:accent1><a:srgbClr val="{a1}"/></a:accent1><a:accent2><a:srgbClr val="{a2}"/></a:accent2>"#,
            r#"<a:accent3><a:srgbClr val="{a3}"/></a:accent3><a:accent4><a:srgbClr val="{a4}"/></a:accent4>"#,
            r#"<a:accent5><a:srgbClr val="{a5}"/></a:accent5><a:accent6><a:srgbClr val="{a6}"/></a:accent6>"#,
            r#"<a:hlink><a:srgbClr val="{hl}"/></a:hlink><a:folHlink><a:srgbClr val="{fl}"/></a:folHlink>"#,
            r#"</a:clrScheme><a:fontScheme name="Office"><a:majorFont><a:latin typeface="Calibri"/>"#,
            r#"<a:ea typeface=""/><a:cs typeface=""/></a:majorFont><a:minorFont><a:latin typeface="Calibri"/>"#,
            r#"<a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme>"#,
            r#"<a:fmtScheme name="Office"><a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill>"#,
            r#"<a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:fillStyleLst>"#,
            r#"<a:lnStyleLst><a:ln><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln>"#,
            r#"<a:ln><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln>"#,
            r#"<a:ln><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln></a:lnStyleLst>"#,
            r#"<a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle>"#,
            r#"<a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst>"#,
            r#"<a:bgFillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill>"#,
            r#"<a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:bgFillStyleLst>"#,
            r#"</a:fmtScheme></a:themeElements></a:theme>"#
        ),
        dk1 = c(1),
        lt1 = c(0),
        dk2 = c(3),
        lt2 = c(2),
        a1 = c(4),
        a2 = c(5),
        a3 = c(6),
        a4 = c(7),
        a5 = c(8),
        a6 = c(9),
        hl = c(10),
        fl = c(11),
    )
}

/// テーマの番号と明るさの加減から実際の色を出す。
/// tint は ECMA-376 の作法: 負なら暗く、正なら明るく(明度だけ動かす)。
pub fn resolve(colors: &[String], idx: u8, tint: f32) -> String {
    let base = colors
        .get(idx as usize)
        .cloned()
        .unwrap_or_else(|| OFFICE.get(idx as usize).unwrap_or(&"000000").to_string());
    if tint.abs() < 0.001 {
        return base;
    }
    let g = |i: usize| {
        u8::from_str_radix(base.get(i * 2..i * 2 + 2).unwrap_or("00"), 16).unwrap_or(0) as f32
            / 255.0
    };
    let (r, gg, b) = (g(0), g(1), g(2));
    let (h, s, l) = rgb_to_hsl(r, gg, b);
    let l2 = if tint < 0.0 {
        l * (1.0 + tint)
    } else {
        l * (1.0 - tint) + tint
    };
    let (r2, g2, b2) = hsl_to_rgb(h, s, l2.clamp(0.0, 1.0));
    format!(
        "{:02X}{:02X}{:02X}",
        (r2 * 255.0).round() as u8,
        (g2 * 255.0).round() as u8,
        (b2 * 255.0).round() as u8
    )
}

fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < 1e-6 {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s.abs() < 1e-6 {
        return (l, l, l);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let f = |mut t: f32| -> f32 {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    (f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn テーマの部品から色を読む() {
        let xml = r#"<a:theme><a:themeElements><a:clrScheme name="x">
            <a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>
            <a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
            <a:dk2><a:srgbClr val="44546A"/></a:dk2>
            <a:lt2><a:srgbClr val="E7E6E6"/></a:lt2>
            <a:accent1><a:srgbClr val="4472C4"/></a:accent1>
            <a:accent2><a:srgbClr val="ED7D31"/></a:accent2>
            </a:clrScheme></a:themeElements></a:theme>"#;
        let c = parse(xml);
        assert_eq!(c[0], "FFFFFF", "背景1(lt1)が 0 番でない");
        assert_eq!(c[1], "000000", "文字1(dk1)が 1 番でない");
        assert_eq!(c[4], "4472C4", "アクセント1が 4 番でない");
    }

    #[test]
    fn 明るさの加減が効く() {
        let c: Vec<String> = OFFICE.iter().map(|s| s.to_string()).collect();
        assert_eq!(resolve(&c, 4, 0.0), "4472C4", "加減なしは素の色");
        let light = resolve(&c, 4, 0.6);
        let dark = resolve(&c, 4, -0.5);
        let lum = |h: &str| {
            (0..3)
                .map(|i| u32::from_str_radix(&h[i * 2..i * 2 + 2], 16).unwrap())
                .sum::<u32>()
        };
        assert!(lum(&light) > lum("4472C4"), "明るくならない: {light}");
        assert!(lum(&dark) < lum("4472C4"), "暗くならない: {dark}");
        // 白と黒は加減しても振り切れない
        assert_eq!(resolve(&c, 0, 0.5).len(), 6);
    }

    #[test]
    fn 部品が無ければ既定の組() {
        let c = parse("<a:theme/>");
        assert_eq!(c.len(), 12);
        assert_eq!(c[4], OFFICE[4]);
    }

    #[test]
    fn 書いて読み直すと同じ組になる() {
        let mut c: Vec<String> = OFFICE.iter().map(|s| s.to_string()).collect();
        c[4] = "C0504D".into();
        let back = parse(&to_xml(&c));
        assert_eq!(back, c, "テーマの往復が壊れている");
    }
}

//! writer の小道具(main.rs から純移動 2026-08-08。部屋割りの3歩目)。
//! 書体・色・セルの字・画像の寸法・変更履歴の差分・語の境界・ルビの印・
//! AI の答えの囲みはがし。**どれも状態を持たない関数**。
//! **純移動**(見えるところを pub(crate) にしただけ — 挙動と文言は変えない)

use crate::*;

/// 本文のフォント。**同梱せず、システムから探す**
/// (埋め込むと実行ファイルがフォントを配ることになり、免許の表示義務も付く)。
///
/// 起動時に一度だけ読み、以後は借りて使う。
/// 見つからなければ**その場で止める** — 日本語が豆腐になった画面を
/// 「動いている」と見せない。
pub(crate) fn font_data() -> &'static [u8] {
    static FONT: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    FONT.get_or_init(|| {
        {
            // 文書が書体を指定していればそれを、無ければ機械にある日本語フォントを
            let (fam, _) = kumihan::font::for_document(None).unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            });
            kumihan::font::load(fam).unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            })
        }
    })
}

/// `RRGGBB` の1成分を 0.0〜1.0 で返す。読めない色は黒として扱う
pub(crate) fn hex(s: &str, i: usize) -> f32 {
    s.get(i * 2..i * 2 + 2)
        .and_then(|h| u8::from_str_radix(h, 16).ok())
        .map(|v| v as f32 / 255.0)
        .unwrap_or(0.0)
}

/// セルの文章(段落を \n で繋いだもの)。
pub(crate) fn cell_text(c: &kumihan::Cellbox) -> String {
    kumihan::paras_text(&c.paragraphs)
}

/// セルへ文章を戻す。段落ごとの書式は同じ位置から引き継ぐ(本文と同じ規則)。
pub(crate) fn set_cell_text(c: &mut kumihan::Cellbox, text: &str) {
    kumihan::set_paras_text(&mut c.paragraphs, text, SIZE_PT);
}

/// PNG / JPEG の画素数 (幅, 高さ)。読めなければ None。
/// 中身は復号しない — 大きさを知るだけなら頭を見れば足りる。
pub(crate) fn image_px(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        // 署名8 + 長さ4 + "IHDR"4 の後に、幅・高さが BE で並ぶ
        let w = u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?);
        let h = u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?);
        return Some((w, h));
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        let mut i = 2usize;
        while i + 9 < bytes.len() {
            if bytes[i] != 0xFF {
                return None;
            }
            let marker = bytes[i + 1];
            // 単独の印(長さ無し)は飛ばす
            if marker == 0xFF || (0xD0..=0xD9).contains(&marker) || marker == 0x01 {
                i += 2;
                continue;
            }
            let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
            // SOF0〜3 に高さ・幅
            if matches!(marker, 0xC0..=0xC3) {
                let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
                let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
                return Some((w, h));
            }
            i += 2 + len;
        }
        return None;
    }
    None
}

/// 変更履歴: 現在の段落の記(そのまま/新規/変更)。
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum PMark {
    Same,
    New,
    /// 変更(組みになる記録開始時点の段落の番号)
    Changed(usize),
}

/// 変更履歴: 段落の列を突き合わせる(LCS)。
/// 返り値: 現在の各段落の記と、消えた段落の列(現在の何番目の前か, 元の番号)。
pub(crate) fn track_diff(base: &[String], cur: &[String]) -> (Vec<PMark>, Vec<(usize, usize)>) {
    let (n, m) = (base.len(), cur.len());
    let idx = |i: usize, j: usize| i * (m + 1) + j;
    let mut dp = vec![0u32; (n + 1) * (m + 1)];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[idx(i, j)] = if base[i] == cur[j] {
                dp[idx(i + 1, j + 1)] + 1
            } else {
                dp[idx(i + 1, j)].max(dp[idx(i, j + 1)])
            };
        }
    }
    // 操作の列に直す
    let mut ops: Vec<(Option<usize>, Option<usize>)> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if base[i] == cur[j] {
            ops.push((Some(i), Some(j)));
            i += 1;
            j += 1;
        } else if dp[idx(i + 1, j)] >= dp[idx(i, j + 1)] {
            ops.push((Some(i), None));
            i += 1;
        } else {
            ops.push((None, Some(j)));
            j += 1;
        }
    }
    while i < n { ops.push((Some(i), None)); i += 1; }
    while j < m { ops.push((None, Some(j))); j += 1; }
    // 隣り合う「消えた」と「増えた」は組みにして「変わった段落」とみなす
    let mut marks = vec![PMark::Same; m];
    let mut deleted: Vec<(usize, usize)> = Vec::new();
    let mut k = 0usize;
    while k < ops.len() {
        if ops[k].0.is_some() && ops[k].1.is_some() {
            k += 1;
            continue;
        }
        let mut olds: Vec<usize> = Vec::new();
        let mut news: Vec<usize> = Vec::new();
        while k < ops.len() && !(ops[k].0.is_some() && ops[k].1.is_some()) {
            match ops[k] {
                (Some(i2), None) => olds.push(i2),
                (None, Some(j2)) => news.push(j2),
                _ => unreachable!(),
            }
            k += 1;
        }
        let pair = olds.len().min(news.len());
        for t in 0..news.len() {
            marks[news[t]] = if t < pair { PMark::Changed(olds[t]) } else { PMark::New };
        }
        // 余った「消えた」は、この塊の次の現在の段落の前に置く
        let at = news.last().map(|j2| j2 + 1)
            .or_else(|| ops.get(k).and_then(|o| o.1))
            .unwrap_or(m);
        for t in pair..olds.len() {
            deleted.push((at, olds[t]));
        }
    }
    (marks, deleted)
}

/// 文字の差分(共通の頭・消えた中身・増えた中身・共通の尻尾)。
pub(crate) fn split_diff(old: &str, new: &str) -> (String, String, String, String) {
    let oc: Vec<char> = old.chars().collect();
    let nc: Vec<char> = new.chars().collect();
    let mut pre = 0usize;
    while pre < oc.len() && pre < nc.len() && oc[pre] == nc[pre] {
        pre += 1;
    }
    let mut suf = 0usize;
    while suf < oc.len() - pre && suf < nc.len() - pre
        && oc[oc.len() - 1 - suf] == nc[nc.len() - 1 - suf]
    {
        suf += 1;
    }
    (
        oc[..pre].iter().collect(),
        oc[pre..oc.len() - suf].iter().collect(),
        nc[pre..nc.len() - suf].iter().collect(),
        oc[oc.len() - suf..].iter().collect(),
    )
}

/// 段落の本文(ランを繋いだもの)。
pub(crate) fn para_text(p: &kumihan::Paragraph) -> String {
    p.runs.iter().map(|r| r.text.as_str()).collect()
}

/// 排他ロックの置き場所(LibreOffice と同じ `.~lock.名前#`)。calc と同じ形。
/// 文字の種類。**日本語の「語」は文字種の変わり目で切る**(分かち書きが無いので、
/// 英数の連なり・ひらがな・カタカナ・漢字・記号の境を語の境とみなす。IME や
/// エディタの通り相場)。
pub(crate) fn char_class(c: char) -> u8 {
    if c.is_whitespace() {
        0
    } else if c.is_ascii_alphanumeric() || c == '_' {
        1
    } else if ('ぁ'..='ゖ').contains(&c) {
        2
    } else if ('ァ'..='ヶ').contains(&c) || c == 'ー' {
        3
    } else if c.is_alphabetic() {
        4 // 漢字ほか
    } else {
        5 // 記号
    }
}

/// 語の境へ(forward なら次の語の頭、そうでなければ前の語の頭)。バイト位置。
pub(crate) fn word_boundary(text: &str, pos: usize, forward: bool) -> usize {
    if forward {
        let chars: Vec<(usize, char)> = text[pos..].char_indices()
            .map(|(i, c)| (pos + i, c)).collect();
        let mut k = 0;
        while k < chars.len() && char_class(chars[k].1) == 0 {
            k += 1;
        }
        if k >= chars.len() {
            return text.len();
        }
        let cl = char_class(chars[k].1);
        while k < chars.len() && char_class(chars[k].1) == cl {
            k += 1;
        }
        // 次の語の頭まで(語の後ろの空白も飛ばす)
        while k < chars.len() && char_class(chars[k].1) == 0 {
            k += 1;
        }
        chars.get(k).map(|(i, _)| *i).unwrap_or(text.len())
    } else {
        let chars: Vec<(usize, char)> = text[..pos].char_indices().collect();
        let mut k = chars.len();
        while k > 0 && char_class(chars[k - 1].1) == 0 {
            k -= 1;
        }
        if k == 0 {
            return 0;
        }
        let cl = char_class(chars[k - 1].1);
        while k > 0 && char_class(chars[k - 1].1) == cl {
            k -= 1;
        }
        chars.get(k).map(|(i, _)| *i).unwrap_or(0)
    }
}

/// モデルの答えからコードフェンス(```python 〜 ```)を外す。
/// system で「書くな」と言っても書くモデルはいるので、受け側でも剥がす
pub(crate) fn strip_code_fence(s: &str) -> String {
    let t = s.trim();
    let Some(rest) = t.strip_prefix("```") else {
        return t.to_string();
    };
    // 1行目(```python 等)を落とし、末尾の ``` を落とす
    let body = rest.split_once('\n').map(|(_, b)| b).unwrap_or("");
    body.trim_end().trim_end_matches("```").trim_end().to_string()
}

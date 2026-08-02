//! フォント — **文書が名前で指定し、機械にある実体を探して結び付ける。**
//!
//! office のフォントはアプリの好みではなく**文書の設定**である。
//! docx なら `w:rFonts`、xlsx なら `<font><name>` に名前が入っている。
//! だからここがやるのは2つだけ:
//!
//!   1. この機械にどんなフォントがあるか数える([`list`]) — リボンの一覧になる
//!   2. 名前から実体を引く([`resolve`]) — 文書が指定した書体を出す
//!
//! **同梱はしない。** 実行ファイルに埋め込むと、それはフォントを配ることになる。
//!
//! 文書が指定した書体がこの機械に無いときは [`fallback`] に落ちるが、
//! **落ちたことは黙らない**(別の書体で出したなら、そう言う必要がある)。
//!
//! 「アプリの好みで優先順を決める」形にしていた時期があるが、それは誤り。
//! office のフォントは利用者が選び、文書が覚えているもの。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// 使えるフォント1つ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Family {
    /// 書体の名前。文書に書かれるのはこれ
    pub name: String,
    pub path: PathBuf,
    /// ttc(まとめ)の中の何番目か
    pub index: u32,
    /// 日本語の字を持っているか
    pub japanese: bool,
    /// 太字・斜体でない、素の書体か
    pub regular: bool,
}

/// 探す場所。OS ごとの標準的な置き場と、利用者の置き場。
fn dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = ["/usr/share/fonts", "/usr/local/share/fonts",
        "/Library/Fonts", "/System/Library/Fonts", "C:\\Windows\\Fonts"]
        .iter().map(PathBuf::from).collect();
    if let Ok(h) = std::env::var("HOME") {
        v.push(PathBuf::from(&h).join(".fonts"));
        v.push(PathBuf::from(&h).join(".local/share/fonts"));
        v.push(PathBuf::from(&h).join("Library/Fonts"));
    }
    if let Ok(d) = std::env::var("OFFICE_FONT_DIR") {
        v.push(PathBuf::from(d));
    }
    v
}

/// この機械にあるフォント。**リボンのフォント一覧はこれ。**
///
/// 名前はファイル名ではなく、フォント自身が持っている書体名から取る
/// (`ipaexg.ttf` ではなく「IPAexゴシック」と出す)。
pub fn list() -> &'static [Family] {
    static CACHE: OnceLock<Vec<Family>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let mut out: Vec<Family> = Vec::new();
        for d in dirs() {
            scan(&d, &mut out, 0);
        }
        // 同じ書体名の中では**素の字面を先に**。
        // 並び順で先頭を採ると「BIZ UDPゴシック」を頼んで Bold が返る
        out.sort_by(|a, b| a.name.cmp(&b.name).then(b.regular.cmp(&a.regular)));
        out.dedup_by(|a, b| a.name == b.name);
        out
    })
}

fn scan(dir: &Path, out: &mut Vec<Family>, depth: usize) {
    if depth > 4 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            scan(&p, out, depth + 1);
            continue;
        }
        let ext = p.extension().and_then(|s| s.to_str()).map(|s| s.to_lowercase());
        if !matches!(ext.as_deref(), Some("ttf") | Some("otf") | Some("ttc")) {
            continue;
        }
        let Ok(data) = std::fs::read(&p) else { continue };
        let n = ttf_parser::fonts_in_collection(&data).unwrap_or(1);
        for i in 0..n {
            if let Some(f) = read_family(&data, i, &p) {
                out.push(f);
            }
        }
    }
}

/// フォント自身が名乗っている書体名を読む。
fn read_family(data: &[u8], index: u32, path: &Path) -> Option<Family> {
    let face = ttf_parser::Face::parse(data, index).ok()?;
    // name_id 1 = 書体名。日本語名があればそちらを採る(画面に出すのは人が読む名前)
    let mut ascii: Option<String> = None;
    let mut local: Option<String> = None;
    for n in face.names() {
        if n.name_id != 1 {
            continue;
        }
        let Some(s) = n.to_string() else { continue };
        if s.is_empty() {
            continue;
        }
        if s.is_ascii() {
            ascii.get_or_insert(s);
        } else {
            local.get_or_insert(s);
        }
    }
    let name = local.or(ascii)?;
    // 日本語を組めるか。「あ」と「日」が引ければ足りる
    let japanese = face.glyph_index('あ').is_some() && face.glyph_index('日').is_some();
    let regular = face.is_regular();
    Some(Family { name, path: path.to_path_buf(), index, japanese, regular })
}

/// 名前から実体を引く。**文書が指定した書体を出すための道。**
///
/// 完全一致で見つからなければ、大文字小文字と空白を無視して探す
/// (「MS ゴシック」「MSゴシック」の揺れを吸う)。
pub fn resolve(name: &str) -> Option<&'static Family> {
    let all = list();
    if let Some(f) = all.iter().find(|f| f.name == name) {
        return Some(f);
    }
    let key = norm(name);
    all.iter().find(|f| norm(&f.name) == key)
}

fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '　' && *c != '-' && *c != '_')
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// 文書が書体を指定していないとき、あるいは指定されたものが無いときに使う。
///
/// **日本語が組めるものを選ぶ。** 英字だけのフォントに落ちると豆腐になる。
pub fn fallback() -> Option<&'static Family> {
    list().iter().find(|f| f.japanese)
}

/// 文書の指定を実体に結び付ける。
///
/// 返り値の2つめが `false` なら**指定と違う書体で出している** —
/// 呼び出し側はそれを利用者に伝えること(黙って別の字で刷らない)。
pub fn for_document(wanted: Option<&str>) -> Result<(&'static Family, bool), String> {
    if let Some(w) = wanted.filter(|s| !s.is_empty()) {
        if let Some(f) = resolve(w) {
            return Ok((f, true));
        }
        let f = fallback().ok_or_else(|| missing(Some(w)))?;
        return Ok((f, false));
    }
    fallback().map(|f| (f, true)).ok_or_else(|| missing(None))
}

fn missing(wanted: Option<&str>) -> String {
    match wanted {
        Some(w) => format!(
            "書体「{w}」がこの機械にありません。代わりに使える日本語フォントも見つかりません\
             (fonts-noto-cjk か fonts-ipaexfont を入れてください)"
        ),
        None => "日本語のフォントが見つかりません\
                 (fonts-noto-cjk か fonts-ipaexfont を入れてください)"
            .into(),
    }
}

/// 実体を読む。
pub fn load(f: &Family) -> Result<Vec<u8>, String> {
    std::fs::read(&f.path).map_err(|e| format!("{}: {e}", f.path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn この機械のフォントを数えられる() {
        let all = list();
        assert!(!all.is_empty(), "1つも見つからない");
        assert!(all.iter().any(|f| f.japanese), "日本語が組めるものが無い");
    }

    #[test]
    fn 名前はファイル名ではなく書体名() {
        // 「ipaexg」ではなく「IPAexゴシック」と名乗らせる
        let f = resolve("IPAexゴシック").expect("IPAexゴシックが引けない");
        assert!(
            f.path.file_stem().unwrap().to_string_lossy() != "IPAexゴシック",
            "ファイル名をそのまま書体名にしている"
        );
        assert!(f.japanese);
    }

    #[test]
    fn 同名なら素の字面を採る() {
        // 「BIZ UDPゴシック」を頼んで Bold が返ってはいけない
        for name in ["BIZ UDPゴシック", "IPAexゴシック", "Noto Sans CJK JP"] {
            if let Some(f) = resolve(name) {
                assert!(f.regular, "{name} で太字・斜体を返した: {}", f.path.display());
            }
        }
    }

    #[test]
    fn 名前から実体を引ける() {
        let first = list().iter().find(|f| f.japanese).unwrap();
        let got = resolve(&first.name).expect("引けない");
        assert_eq!(got.path, first.path);
    }

    #[test]
    fn 表記の揺れを吸う() {
        let first = list().iter().find(|f| f.japanese).unwrap();
        let messy = first.name.to_uppercase().replace(' ', "");
        assert!(resolve(&messy).is_some(), "「{}」を引けない", messy);
    }

    #[test]
    fn 無い書体は代用するが黙らない() {
        let (f, exact) = for_document(Some("存在しない書体XYZ")).unwrap();
        assert!(!exact, "指定と違う書体なのに、合っていることにした");
        assert!(f.japanese, "英字フォントに落ちている(豆腐になる)");
    }

    #[test]
    fn 指定が無ければ日本語が組めるものを選ぶ() {
        let (f, exact) = for_document(None).unwrap();
        assert!(exact);
        assert!(f.japanese);
    }

    #[test]
    fn 引いた実体で組版できる() {
        let (f, _) = for_document(None).unwrap();
        let data = load(f).unwrap();
        let m = crate::Metrics::new(&data).expect("解釈できない");
        let w = m.advance_mm('日', 10.5);
        assert!(w > 3.0 && w < 4.5, "全角の幅がおかしい: {w}mm ({})", f.name);
    }
}

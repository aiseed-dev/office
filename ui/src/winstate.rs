//! 窓の位置と大きさの控え — 前に閉じたときの姿で次を開く。
//!
//! 置き場は ~/.config/office/window-<app>.txt(recent と同じ作法)。
//! 1行だけ: `x y w h` に、最大化なら ` max` を添える。
//! 窓の控えは**あくまで控え** — 読めない・壊れている・画面に収まらない値なら
//! 黙って既定に戻る(見えない場所に窓を開いてしまうのが一番の事故)。

use std::path::PathBuf;

/// 覚えている窓の姿(論理ピクセル)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WinState {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub maximized: bool,
}

fn file(app: &str) -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(format!(".config/office/window-{app}.txt"))
}

/// 読む。無い・壊れている・大きさが常識外なら None(既定で開く)
pub fn load(app: &str) -> Option<WinState> {
    parse(&std::fs::read_to_string(file(app)).ok()?)
}

fn parse(s: &str) -> Option<WinState> {
    let mut it = s.split_whitespace();
    let x: f32 = it.next()?.parse().ok()?;
    let y: f32 = it.next()?.parse().ok()?;
    let w: f32 = it.next()?.parse().ok()?;
    let h: f32 = it.next()?.parse().ok()?;
    let maximized = it.next() == Some("max");
    // 小さすぎる・大きすぎる窓は控えとして信用しない
    if !(200.0..=16000.0).contains(&w) || !(150.0..=16000.0).contains(&h) {
        return None;
    }
    Some(WinState { x, y, w, h, maximized })
}

/// 書く。失敗しても黙る — 窓の控えのためにアプリを止めない
pub fn save(app: &str, st: WinState) {
    let f = file(app);
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(
        &f,
        format!(
            "{:.0} {:.0} {:.0} {:.0}{}\n",
            st.x, st.y, st.w, st.h,
            if st.maximized { " max" } else { "" }
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 書いた姿がそのまま読める() {
        let st = WinState { x: 120.0, y: 80.0, w: 900.0, h: 1000.0, maximized: false };
        let s = format!("{:.0} {:.0} {:.0} {:.0}\n", st.x, st.y, st.w, st.h);
        assert_eq!(parse(&s), Some(st));
        let s = format!("{:.0} {:.0} {:.0} {:.0} max\n", st.x, st.y, st.w, st.h);
        assert_eq!(parse(&s).map(|v| v.maximized), Some(true));
    }

    #[test]
    fn 壊れた控えは黙って捨てる() {
        assert_eq!(parse(""), None, "空");
        assert_eq!(parse("a b c d"), None, "数でない");
        assert_eq!(parse("0 0 10 10"), None, "豆粒の窓を信用しない");
        assert_eq!(parse("0 0 99999 99999"), None, "壁一面の窓を信用しない");
        // 負の位置は許す(マルチモニタで左の画面は負の座標)
        assert!(parse("-1920 0 900 1000").is_some());
    }
}

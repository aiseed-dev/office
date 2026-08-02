//! 壊さない保存。
//!
//! `File::create(既存の道)` は**書く前に元を空にする**。途中で失敗すると
//! (ディスク満杯・停電)、利用者の元の書類が消えて戻らない。
//!
//! だから隣に書いてから、名前の付け替えで入れ替える。
//! 付け替え(rename)は同じファイルシステムの中では原子的 —
//! 「元のまま」か「新しい中身」のどちらかしか起きない。

use std::path::Path;

/// `write` が途中で失敗しても、`path` の元の中身は無事。
pub fn save<F>(path: &Path, write: F) -> Result<(), String>
where
    F: FnOnce(std::fs::File) -> Result<(), String>,
{
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "out".into());
    let tmp = path.with_file_name(format!(".{name}.saving"));

    let f = std::fs::File::create(&tmp).map_err(|e| format!("書けません: {e}"))?;
    if let Err(e) = write(f) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    // 中身をディスクへ落としてから入れ替える(できなくても続ける)
    if let Ok(f) = std::fs::File::open(&tmp) {
        let _ = f.sync_all();
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("入れ替えできません: {e}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 失敗しても元の中身は無事() {
        let dir = std::env::temp_dir().join(format!("office-atomic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("大事な書類.docx");
        std::fs::write(&p, b"original").unwrap();

        // 書きかけで失敗する保存
        let r = save(&p, |mut f| {
            use std::io::Write;
            f.write_all(b"partial").unwrap();
            Err("ディスクが一杯(のつもり)".into())
        });
        assert!(r.is_err());
        assert_eq!(std::fs::read(&p).unwrap(), b"original", "元の書類が壊れた");
        // 書きかけの残骸も無い
        assert!(!dir.join(".大事な書類.docx.saving").exists(), "残骸が残った");

        // 成功すれば入れ替わる
        save(&p, |mut f| {
            use std::io::Write;
            f.write_all(b"new content").map_err(|e| e.to_string())
        })
        .unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"new content");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

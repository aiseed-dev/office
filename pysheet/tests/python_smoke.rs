//! Python 側から往復できることの検査。
//!
//! cdylib を組み、`office_sheet.so` の名前で置いて、`test.py` を回す。
//! python3 が無い機械では飛ばす(無いのに失敗と言わない)。
use std::process::Command;

#[test]
fn python側から帳票を差し込める() {
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("python3 が無いので飛ばす");
        return;
    }

    // この試験自体のビルドでは cdylib が出来ているとは限らないので、組む。
    // (外の cargo のビルドは終わっているので、ここで cargo を呼んでも詰まらない)
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
    let status = Command::new(env!("CARGO"))
        .args(["build", "-p", "pysheet"])
        .current_dir(root)
        .status()
        .expect("cargo を呼べない");
    assert!(status.success(), "pysheet が組めない");

    // target/debug の場所は、この試験の実行ファイルから辿る
    // (target/debug/deps/xxx → target/debug)
    let exe = std::env::current_exe().expect("自分の場所が分からない");
    let debug = exe.parent().and_then(|p| p.parent()).expect("target/debug が見つからない");
    let so = debug.join("liboffice_sheet.so");
    assert!(so.exists(), "{} が無い", so.display());

    // Python の import 名に合わせて置く
    let dir = debug.join("pysheet-import");
    std::fs::create_dir_all(&dir).expect("作業場所を作れない");
    std::fs::copy(&so, dir.join("office_sheet.so")).expect("置けない");

    let out = Command::new("python3")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/test.py"))
        .env("PYTHONPATH", &dir)
        .output()
        .expect("python3 を回せない");
    assert!(
        out.status.success(),
        "python の検査が失敗:\n--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

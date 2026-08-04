# sample — 公開できるサンプル

「開いて・直して・刷る」を試すための帳票と文書。**中身はすべて架空**
(実在の様式・人名・社名は入っていない)。templates/ が「ブック=業務アプリ」の
見本なのに対し、こちらはただの普通の書類。

```bash
./target/release/calc  sample/見積書.xlsx
./target/release/writer sample/報告書.docx
```

## 見積書.xlsx

結合(表題)・罫線・表示形式(¥#,##0)・式(数量×単価、SUM・ROUND)・
印刷範囲(A4 縦)の見本。数量や単価を打ち直すと合計まで再計算される。
ファイル > 印刷で PDF にすると印刷範囲どおりに出る。

## 報告書.docx

見出し(1〜3)・表・本文の見本。python-docx で作ってある —
参考資料 > 目次 を押せば見出しから目次ができる。

## 作り直し

サンプルは生成物。直すのは生成側で、ファイルを直接直さない。

```bash
cargo run -p sheet --example gen_samples   # 見積書.xlsx
.venv/bin/python sample/gen_report.py      # 報告書.docx(要 python-docx)
```

どちらも検査つき: 見積書は pysheet で開いて式の値を、報告書は
`ooxml --example rtall` で往復(本文一致・注記0)を確かめてある。

# sample — 公開できるサンプル

「開いて・直して・刷る」を試すための帳票と文書。**中身はすべて架空**
(実在の様式・人名・社名は入っていない)。templates/ が「ブック=業務アプリ」の
見本なのに対し、こちらはただの普通の書類。

```bash
./target/release/calc  sample/見積書.xlsx
./target/release/writer sample/報告書.docx
```

## calc 用(xlsx)

- **見積書.xlsx** — 結合(表題)・罫線・表示形式(¥#,##0)・式(数量×単価、
  SUM・ROUND)・印刷範囲(A4 縦)。数量や単価を打ち直すと合計まで
  再計算される。ファイル > 印刷で PDF にすると印刷範囲どおりに出る
- **出納帳.xlsx** — 前行を引き継ぐ残高の式・条件付き書式(残高が1万円を
  割ると塗って知らせる — 最終行で実際に発動している)・セルのコメント・
  タイトル行の繰り返し
- **成績表.xlsx** — AVERAGE・MAX・COUNTIF(">=80")と条件付き書式
  (80点以上を塗る)

## writer 用(docx)

- **報告書.docx** — 見出しの階層・表・本文。参考資料 > 目次 を押せば
  見出しから目次ができる
- **送付状.docx** — 右揃え(日付・差出・敬具)と中央揃え(表題・記)の定型
- **議事録.docx** — 見出し+表(開催情報)+決定事項

## 注文書付きカタログ(writer 用・サーバー連携の見本)

- **カタログ.docx** — 分類ごとの見出し+商品表(36品目)、巻末に注文書の
  ページ。writer で開いて 参考資料 > 目次 を押せば、ページ番号つきの
  目次ができる
- **商品マスタの正本はサーバー、docx は生成物**という分業の見本:

```bash
python3 sample/catalog_server.py           # マスタを配る(127.0.0.1:8765)
.venv/bin/python sample/gen_catalog.py     # サーバーから取って作り直す
```

サーバー側は価格改定と新商品(37品目)を持っているので、作り直すと
カタログが追従する。**繋がらなければ、そう言ってから**同梱の見本データで
作る(黙って古いままにしない)。追跡に入っているのは同梱データ版。

## 作り直し

サンプルは生成物。直すのは生成側で、ファイルを直接直さない。

```bash
cargo run -p sheet --example gen_samples   # xlsx 3件
.venv/bin/python sample/gen_docs.py        # docx 3件(要 python-docx)
.venv/bin/python sample/gen_catalog.py     # カタログ.docx
```

どれも検査つき: xlsx は pysheet で開いて式の値を、docx は
`ooxml --example rtall` で往復(本文一致・注記0)を確かめてある。

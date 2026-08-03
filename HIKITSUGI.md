# 引き継ぎ(2026-08-03)

別の環境で作業を引き継ぐ人(または Fable)への申し送り。
**設計と決定事項の正本は [SEKKEI.md](SEKKEI.md)** — まずそちらを読むこと。
ここには SEKKEI に書かない「進め方の作法」と「直近の状態」だけを書く。

## 進め方の作法(発注者と確立済み)

- 発注者は実機で GUI を触り、日本語で不具合を報告する。
  **1報告 = 原因究明 → 修正 → `cargo test --workspace` 緑 →
  `cargo build --release` → コミット**、の回転。発注者は
  `./target/release/{writer,calc}` を直接起動して確かめるので、
  **release の組み直しを忘れると「直っていない」と見える**
- 原因は推測で直さず、`vendor/zed/crates/gpui` のソースで裏を取る
- コミットは日本語で「何を・なぜ」。設計判断と踏んだ穴は SEKKEI.md に追記
- 家訓: できないものを、できるように見せない(未実装は灰色。ready の嘘は
  wiring_tests が落とす)/ 黙って落とさない(Report へ)/
  書式は据え置き / どの操作も1手で戻せる

## 環境の注意

- cargo が PATH に無い機械なら `export PATH="$HOME/.cargo/bin:$PATH"`。
  そもそも rustup ごと無い機械もある — その場合は user レベルの導入で足りる
  (`curl https://sh.rustup.rs | sh -s -- -y`。sudo 不要。2026-08-03 に実施済み)
- `.cargo/config.toml` と `.linklibs/` は**この機械固有**(libxkbcommon-x11 の
  開発リンク回避)。別の機械では不要か、作り直しが要る。
  作り直しは2手: `.linklibs/libxkbcommon-x11.so` → 実行時の `.so.0` へ symlink、
  `.cargo/config.toml` に `rustflags = ["-L", "<絶対パス>/.linklibs"]`
- 実物の様式(検査の材料)は
  `/mnt/sdb/home/dev/ドキュメント/機構/yoryou-yoshiki/` にある。
  **無い環境ではテストは黙って飛ぶ**(失敗はしない)。可能ならコピーして持つ
- Python 検証は miniforge の `.venv`(polars / python-docx)。無ければ
  `conda create -p .venv python polars python-docx`

## GPUI の踏み跡(再発させない)

- div の既定レイアウトは **Block(縦積み)**。入力とマウスの受け皿
  (InputSink)は absolute + inset 0 で全面に重ねる
- マウスの bubble 配送は**後に登録した方が先**。重ねるメニュー類は
  InputSink より後に描き、項目側の on_mouse_down で `cx.stop_propagation()`
- mouse-up は**ポインタが乗っている要素にしか来ない**。ドラッグ終了は
  `window.on_mouse_event` + move 時の `pressed_button` 確認で自癒させる
- 文字の行の高さは既定で黄金比 → グリフは div の頭から約 0.309×サイズ下に
  描かれる(writer の `HALF_LEADING`)。自前で引く線・帯はこれを織り込む
- rfd のダイアログは同期 = 主の糸を塞ぐ。終了確認は
  `background_executor().spawn` + `cx.spawn` で別の糸に出してある。
  **calc のファイル選択(開く・保存・PDF)も別の糸にした**(2026-08-03)。
  writer の pick_file / save_file はまだ同期のまま(既知の残件)
- リボンは生成物: `python3 ui/gen_ribbon.py ja > ui/src/ribbon.rs`。
  押せるものは同スクリプトの READY 表。テンプレートの class 注入スロット
  (pagebreak・insertimage 等)は id の正規表現に掛からないので手で差し込む

## 実装の注意

- 保存は `write_with(原本)` が基本(xlsx / docx とも)。原本を渡さないと
  図形・スタイル等の部品が消える
- `kumihan::Paragraph` に性質を足すと、全指定 literal が数カ所で割れる
  (engine/examples/page.rs、ooxml のテスト)。編集の持ち越しは
  set_body_text が「段落をまるごと写す」方式なので漏れない
- calc の編集判定は `editing()`(数式バーとセルの保存内容の差)。
  「バーが空か」で分岐してはいけない(バーには常にセルの中身が写る)

## 直近の状態(詳細は git log)

済: calc の右クリックメニュー(Euro-Office 準拠・灰色ゼロ)、
Paste Special(独自)、複数シート、条件付き書式・コメント・リンク・
名前の定義(xlsx 往復つき)、コピー範囲の破線、
列幅・行高のドラッグ(見出しの境界を掴む。undo 可・xlsx 往復は元からある)、
境界ダブルクリックの自動調整(kumihan::Metrics の実フォント字幅で測る)、
calc のファイル選択の別糸化(開く・保存・PDF。保存が済んだときだけ終了)、
PDF の塗り・文字色・条件付き書式(切れた列の数も返して status に出す)。
writer の選択描画・ドラッグ選択・全カーソル操作・右クリックメニュー・
段落の帯と囲み枠・画像の挿入(media/rels/Content_Types 生成)・
ヘッダー・フッター(板で編集・ページ番号は PAGE フィールド往復・
PDF は各頁の番号。設計は SEKKEI「writer のヘッダー・フッター」)。
両アプリの窓の移動・終了確認(別糸)・スクロール・クリップボード。
pysheet(Python 束縛、polars 連携)。

## 次の仕事(発注者と合意した順)

1. 目次(見出しスタイルが前提)、writer の残りの灰色
2. 小さい残件: writer のファイル選択の非同期化(calc は済)、
   writer のキャレット表示の改善、変換下線の位置の実機確認、
   ヘッダー・フッターの残り(日付/時刻・ページ数・表入りの編集)

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
- rfd のダイアログは同期 = 主の糸を塞ぐ。終了確認・ファイル選択は
  `background_executor().spawn` + `cx.spawn` で別の糸に出す。
  **calc・writer とも全ダイアログ(開く・保存・PDF・画像)を別糸化済み**
  (2026-08-03)。新しいダイアログを足すときも同じ作法で
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
PDF の塗り・文字色・条件付き書式(切れた列の数も返して status に出す)、
データの入力規則(list を読み・効かせ・往復・リボンから作成/解除。
設計は SEKKEI「calc に残すもの」。条件付き書式と名前の管理もリボンから)、
PDF が帳票の印刷設定に従う(pageSetup の向き・用紙(B は JIS)・
pageMargins・Print_Area。効かせたものは status に言う)、
レイアウトタブの印刷設定(向き・用紙・余白・印刷範囲を設定できる。
Print_Area はモデルに昇格し undo 可。保存は原文の pageSetup に
**属性だけ差し替え** — 拡大縮小など知らない属性は残す)。
writer の選択描画・ドラッグ選択・全カーソル操作・右クリックメニュー・
段落の帯と囲み枠・画像の挿入(media/rels/Content_Types 生成)・
ヘッダー・フッター(板で編集・ページ番号は PAGE フィールド往復・
PDF は各頁の番号。設計は SEKKEI「writer のヘッダー・フッター」)・
見出しスタイルと目次(ホーム > 段落のスタイル → 参考資料 > 目次。
設計は SEKKEI「writer の見出しと目次」)・全ダイアログの別糸化・
複数レベルのリスト(印はレベルで変わる。Tab/Shift+Tab で深さ)・
ページ数と日付(ヘッダー/フッタータブ灰色ゼロ)・ダークモード(紙は白のまま。
表示タブ灰色ゼロ)・行番号・ファイルからのテキスト・テキストの追加・
目次の番号の右揃え(字幅で点線を詰める)・キャレットの大きさ追従・
しおりとコメントの錨の保存持ち越し(黙って捨てていた穴を塞いだ)。
writer は 55/80(残る灰色25は下記)。
両アプリの窓の移動・終了確認(別糸)・スクロール・クリップボード。
pysheet(Python 束縛、polars 連携)。

## 次の仕事(発注者と相談してから)

writer の残りの灰色25は、**全部に方針判断か大物の設計が要る**:
- 図形・SmartArt・グラフ・テキストボックス等の挿入系 — calc は
  「Python での正解」表で灰色のまま、が方針(SEKKEI)。writer も
  「matplotlib 等で描いて画像として貼る」で足りるかを発注者と決める
- コメント・変更履歴(レビュー)— コメントは段落単位なら実装できる
  (錨の保存持ち越しは済み)。粒度を発注者と決める
- 縦書き(テキスト方向)は K4 として後回し済み(ユーザー判断)
- 透かし・ページ色・配色・段組み・ハイフネーション・描画(ペン)—
  事務様式に要るかから相談

小さい残件: 変換下線の位置の実機確認、表入りのヘッダー・フッターの編集、
set_body_text の性質持ち越しが段落番号ベースな件(段落の増減を undo すると
下の段落の性質がずれうる。既知・全機能共通の土台の話)

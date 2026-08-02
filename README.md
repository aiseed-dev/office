# office

**手元で動く Word と Excel。** Rust で書いた、docx と xlsx のための2つのアプリ。

- `writer` — docx を開いて、直して、保存する。PDF にもできる
- `calc` — xlsx を開いて、直して、保存する。式も計算する

**別々のアプリ**です。1つの巨大なスイートにしていません。

## いま出来ること

| | writer | calc |
|---|---|---|
| 開く・保存 | docx | xlsx |
| 日本語入力(IME) | ○ | ○ |
| 文字書式 | 太字・斜体・下線・取り消し線・色・大きさ・書体 | 太字・斜体・下線・色 |
| 揃え | 左・中央・右・両端 | 左・中央・右 |
| 罫線 | — | ○ |
| 塗りつぶし | — | ○ |
| 表示形式 | — | 桁区切り・小数・％・通貨 |
| 行・列の出し入れ | — | ○(式の参照も直る) |
| 並べ替え・重複削除 | — | ○ |
| セル結合 | — | ○(範囲選択は Shift+矢印) |
| 箇条書き・段落番号 | ○ | — |
| インデント・行間・改ページ | ○ | — |
| 式 | — | 四則と28関数(SUMIF・COUNTIF・ROUNDDOWN ほか) |
| PDF | ○(複数ページ) | ○(罫線つき。塗りと列幅は未対応) |
| 拡大縮小 / 数式の表示 | ○ / — | — / ○ |
| 校正 | ○ | — |

リボンの並びは Euro-Office に合わせてあります(乗り換える人が場所を覚え直さずに済むように)。
**まだ出来ていないコマンドは灰色で並んでいます** — できないものを、できるように見せません。

**マクロはありません。** 文書の中に実行コードを置かない設計なので、
「開く=実行」という攻撃経路が最初から存在しません。

## 動かす

必要なもの: Rust(1.80以降)、日本語のフォント、Linux なら Wayland か X11。

```bash
cargo build --release

./target/release/writer            # 空で開く
./target/release/writer 文書.docx
./target/release/calc  帳票.xlsx
```

初回のビルドは GPUI(zed)を取ってくるので時間がかかります。

### フォント

**同梱していません。** 書体は文書の設定なので、docx/xlsx に書かれている名前を
この機械のフォントから探します。無ければ日本語が組めるものに落ちます。

```bash
OFFICE_FONT=/path/to/font.ttf ./target/release/writer   # 明示して指定する
```

`fonts-noto-cjk` か `fonts-ipaexfont` が入っていれば動きます。

### 校正(writer のレビュー > 校正)

英語の綴りは辞書だけで見ます(`/usr/share/dict/words` など)。
日本語の誤変換・表記ゆれは辞書では出てこないので、ローカルのモデルに聞きます。

```bash
OFFICE_HOST=127.0.0.1 OFFICE_PORT=8000 OFFICE_MODEL=... ./target/release/writer
```

OpenAI互換の `/v1/chat/completions` を話す相手なら何でも構いません。
**繋がらなければ「校正できません」と出ます** — 黙って「指摘なし」にはしません。

単体でも使えます。

```bash
cargo run --release --bin office-spell -- 文書.txt
cargo run --release --bin office-spell -- --furigana 原稿.txt
```

## 構成

```
engine/   kumihan — 組版の核(行組み・禁則・字幅・紙面の座標)
ooxml/    docx の読み書き
sheet/    xlsx の読み書き、式の計算、書式(styles.xml)
lang/     言語ごとの中身。gpui を知らないので画面なしで回せる
paper/    紙面を PDF へ写す
ui/       gpui との結線(入力・IME・リボン)
writer/   docx のアプリ
calc/     xlsx のアプリ
```

**画面も紙も、同じ紙面を別の面に写すだけ**です。だから画面と印刷が食い違いません。

## 各国語版

言語ごとの差は `lang` の `Language` に閉じてあります。1つ実装すれば足ります。
画面の言葉は Euro-Office のロケール(45言語)から起こせます。

```bash
python3 ui/gen_ribbon.py --list      # 使えるロケール
python3 ui/gen_ribbon.py en > ui/src/ribbon.rs
```

## 免許

**AGPL-3.0-or-later**(`LICENSE`)。同梱・派生しているものの由来は `NOTICE.md`。

## 作りかけです

リボンに並んでいるコマンドのうち、動くのは writer 23/78、calc 29/105。
残りは灰色で出ています。設計と判断の記録は `SEKKEI.md`。

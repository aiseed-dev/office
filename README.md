# office

**手元で動く Word と Excel。** Rust で書いた、docx と xlsx のための2つのアプリ。

- `writer` — docx を開いて、直して、保存する。PDF にもできる([手引き](docs/writer-manual.md))
- `calc` — xlsx を開いて、直して、保存する。式も計算する([手引き](docs/calc-manual.md))

**別々のアプリ**です。1つの巨大なスイートにしていません。

## いま出来ること

| | writer | calc |
|---|---|---|
| 開く・保存 | docx(原本の部品は据え置き) | xlsx(同) |
| 日本語入力(IME)・undo | ○ | ○ |
| 文字書式 | 太字・斜体・下線・取り消し線・色・蛍光ペン・上付き下付き・大きさ・書体(**選択の字だけに掛かる**) | 太字・斜体・下線・取り消し線・色・塗り |
| 段落 | 揃え・箇条書き(レベル付き)・インデント・行間・改ページ・背景と囲み枠・ドロップキャップ | — |
| 見出しと目次 | ○(目次・図表目次はページ番号つき) | — |
| ヘッダー・フッター | ○(ページ番号・ページ数・日付) | — |
| 表 | ○(セル結合ごと読み書き・セルの中で編集) | (それが本体) |
| 画像 | 挿入○(PNG・JPEG・**SVG は高精細に変換**)。図形・グラフは Python で描いて貼る | 挿入○(PNG・JPEG)。図形・SmartArt・テキストアート・方程式(TeX)・スパークライン・記号・チェックボックス。グラフの釦の裏は matplotlib |
| コメント | ○(段落単位) | ○ |
| 変更履歴 | ○(保存で Word の変更履歴になる) | — |
| しおり・透かし・ページ色・段組み | ○ | — |
| 描画(ペン・蛍光ペン・消しゴム) | ○(docx では図形になる) | — |
| 式 | — | 四則と関数(SUMIF・COUNTIF ほか)・再計算・循環の検出・**=PY**(Python で書く自作関数) |
| シート | — | 複数シート・枠の固定・フィルター・スライサー・並べ替え・グループ化と小計 |
| ピボットテーブル | — | ○(裏方 polars。指図をブックに控えて更新できる) |
| ソルバー・ゴールシーク | — | ○(単体法 LP。裏方 scipy) |
| 保護・暗号化・署名 | ○ | ○(読み取り専用・AES-128・Ed25519 の添え書き) |
| チャット・バージョン履歴 | ○ | ○(サーバー無し — 共有フォルダのファイル越し) |
| 条件付き書式・入力規則 | — | ○(xlsx 往復つき) |
| リンク・名前の定義・形式を選択して貼り付け | — | ○ |
| Python の持ち運び(マクロの代わり) | .py を檻で回すマクロ(d = python-docx の文書)。文書には載せない | ○(@save でブックに載せ、実行は檻の中) |
| 印刷の設定 | 用紙・向き・余白・段組み | 用紙(JIS の B も)・向き・余白・印刷範囲 |
| PDF | ○(ヘッダー・フッター・透かし・ペンごと) | ○(罫線・塗り・印刷設定に従う) |
| 検索と置換 | ○ | ○ |
| しおりへの相互参照 | ○(Word の REF/PAGEREF フィールド) | — |
| 欧文のハイフネーション | ○(TeX と同じ分綴パターン) | — |
| 校正 | ○ | — |

リボンの並びは Euro-Office に合わせてあります(乗り換える人が場所を覚え直さずに済むように)。
**まだ出来ていないコマンドは灰色で並んでいます** — できないものを、できるように見せません。

**VBA 型のマクロはありません。** その代わり calc は **Python をブックに
載せて持ち運べます**(`@save 名前` で搭載、`@名前` で実行、セル関数は
`=PY(…)`)。開いても決して自動実行せず、ブック由来のコードは必ず檻
(bubblewrap)の中で回します — 「開く=実行」という攻撃経路は最初から
存在しません。詳しくは [calc の手引き](docs/calc-manual.md)。

## 動かす

必要なもの: Rust(1.80以降)、日本語のフォント、Linux なら Wayland か X11。

```bash
cargo build --release

./target/release/writer            # 空で開く
./target/release/writer sample/報告書.docx   # 同梱のサンプル(中身は架空)
./target/release/calc  sample/見積書.xlsx
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

## Python との分業

**見ながら整える仕事はアプリ、データを作る・絵を描く仕事は Python。**
グラフ・SmartArt・方程式・ピボット・ソルバーまで釦がありますが、裏で
働くのは Python(matplotlib・polars・scipy)です。込み入った分析は
そのまま polars・statsmodels で。

xlsx には束縛(`pysheet`)があります。openpyxl と違い、罫線・結合・列幅・
図形を保ったまま値を差し込めます。

```python
import office_sheet
b = office_sheet.Book.open("様式7.xlsx")
b["提案見積書"]["A30"] = "日本フネン株式会社"   # 書式は据え置き
b.save("out.xlsx")
```

docx には作っていません — python-docx がそのまま使えます(その保存を
writer が読めることは実物で確認済み)。

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
pysheet/  sheet の Python 束縛(import office_sheet)
```

**画面も紙も、同じ紙面を別の面に写すだけ**です。だから画面と印刷が食い違いません。

## 各国語版

言語ごとの差は `lang` の `Language` に閉じてあります。1つ実装すれば足ります。
画面の言葉は Euro-Office のロケール(45言語)から起こせます。

```bash
python3 ui/gen_ribbon.py --list      # 使えるロケール
python3 ui/gen_ribbon.py en > ui/src/ribbon.rs
```

## ライセンス

**AGPL-3.0-or-later**(`LICENSE`)。同梱・派生しているものの由来は `NOTICE.md`。

## 作りかけです

リボンに並んでいるコマンドのうち、動くのは writer 90/103、calc 121/140。
残りは灰色で出ています(それぞれの見送りの理由は `HIKITSUGI.md` に)。
設計と判断の記録は `SEKKEI.md`。

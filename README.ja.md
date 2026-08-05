# office

*正本(primary)は英語版: [README.md](README.md)。この日本語版は副です。*

**手元で動く Word と Excel。** Rust で書いた、docx と xlsx のための2つのアプリ。

- `writer` — docx を開いて、直して、保存する。PDF にもできる([手引き](docs/writer-manual.ja.md))
- `calc` — xlsx を開いて、直して、保存する。式も計算する([手引き](docs/calc-manual.ja.md))

**別々のアプリ**です。1つの巨大なスイートにしていません。

## いま出来ること

| | writer | calc |
|---|---|---|
| 開く・保存 | docx(原本の部品は据え置き) | xlsx(同) |
| 日本語入力(IME)・undo | ○ | ○ |
| 文字書式 | 太字・斜体・下線・取り消し線・色・蛍光ペン・上付き下付き・大きさ・書体(**選択の字だけに掛かる**) | 太字・斜体・下線・取り消し線・色・塗り |
| 段落 | 揃え・箇条書き(レベル付き)・インデント・行間・改ページ・背景と囲み枠・ドロップキャップ | — |
| **日本語一級** | **縦書き・ルビ・均等割付**(テキスト方向で縦横を切り替え) | 文字の向き(縦の見出し) |
| 記入欄(コンテンツコントロール) | ○(記入欄・ドロップダウン・チェック — 申込書を作って、保護して配れる) | 入力規則とチェックボックスで同等 |
| 見出しと目次 | ○(目次・図表目次はページ番号つき) | — |
| ヘッダー・フッター | ○(ページ番号・ページ数・日付) | — |
| 表 | ○(セル結合ごと読み書き・セルの中で編集) | (それが本体) |
| 画像 | 挿入○(PNG・JPEG・**SVG は高精細に変換**)。図形・グラフは Python で描いて貼る | 挿入○(PNG・JPEG)。図形・SmartArt・テキストアート・方程式(TeX)・スパークライン・記号・チェックボックス。グラフのボタンの裏は matplotlib |
| コメント | ○(段落単位) | ○ |
| 変更履歴 | ○(保存で Word の変更履歴になる) | — |
| しおり・透かし・ページ色・段組み | ○ | — |
| 描画(ペン・蛍光ペン・消しゴム) | ○(docx では図形になる) | — |
| 式 | — | 四則と関数(約185。動的配列も)・再計算・循環の検出・**=PY**(Python で書く自作関数) |
| シート | — | 複数シート・枠の固定・フィルター・スライサー・並べ替え・グループ化と小計 |
| ピボットテーブル | — | ○(裏方 polars。定義をブックに控えて更新できる) |
| ソルバー・ゴールシーク | — | ○(単体法 LP。裏方 scipy) |
| 保護・暗号化・署名 | ○ | ○(読み取り専用・AES・Ed25519 の署名ファイル) |
| チャット・バージョン履歴 | ○ | ○(サーバー無し — 共有フォルダのファイル越し) |
| 条件付き書式・入力規則 | — | ○(xlsx 往復つき) |
| リンク・名前の定義・形式を選択して貼り付け | — | ○ |
| Python の持ち運び(マクロの代わり) | .py をサンドボックスで回すマクロ(d = python-docx の文書)。文書には載せない | ○(@save でブックに載せ、実行はサンドボックスの中) |
| 印刷の設定 | 用紙・向き・余白・段組み | 用紙(JIS の B も)・向き・余白・印刷範囲 |
| PDF | ○(ヘッダー・フッター・透かし・ペンごと) | ○(罫線・塗り・印刷設定に従う) |
| 検索と置換 | ○ | ○ |
| しおりへの相互参照 | ○(Word の REF/PAGEREF フィールド) | — |
| 欧文のハイフネーション | ○(TeX と同じ分綴パターン) | — |
| 校正 | ○ | — |

リボンの並びは Euro-Office に合わせてあります(乗り換える人が場所を覚え直さずに済むように)。
**まだ出来ていないコマンドはグレー表示で並んでいます** — できないものを、できるように見せません。

**VBA 型のマクロはありません。** その代わり calc は **Python をブックに
載せて持ち運べます**(`@save 名前` で搭載、`@名前` で実行、セル関数は
`=PY(…)`)。開いても決して自動実行せず、ブック由来のコードは必ず
サンドボックス(bubblewrap)の中で回します — 「開く=実行」という攻撃経路は
最初から存在しません。詳しくは [calc の手引き](docs/calc-manual.ja.md)。

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
グラフ・SmartArt・方程式・ピボット・ソルバーまでボタンがありますが、裏で
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

コードを書く人向けの正本は [Python の手引き](docs/python-manual.ja.md) —
**範囲⇄配列のやり取り**(values() と1セルずつの書き込み)、=PY の
2次元リストとスピル、writer マクロの `d`、サンドボックスの中の約束、まで実測つき。

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

**画面の言語は実行時に切り替えられます。** 14言語を同梱:
ja(既定)・en・de・es・fr・id・it・ko・pt・ru・tr・vi・zh・zh-tw。

```bash
OFFICE_LANG=en ./target/release/writer      # 一時的な上書き
# 恒久には ~/.config/office/settings.toml に:
#   language = "en"
```

**画面全体が切り替わります** — リボンも、ステータスバーの文言も、
ダイアログも(各言語585句を全訳。未訳のある言語は登録されません)。
設定ページもあります: ファイル > 詳細設定 で、いまの言語・書体・校正の宛先・
Python の経路が見え、言語を順に切り替えられます(次の起動から反映)。

標準のリボンの語は Euro-Office 本家のロケールから、残りの文言は
Claude による機械翻訳(定額内)で、言語ごとに Microsoft Office の用語に
合わせてあります。en と ja は目を通してあります —
**ほかの言語は、母語話者の校閲を歓迎します。**

言語を足すのは1コマンドです — `ui/gen_lang.py` が材料
(`ui/i18n/keys.json`、番号つきの ja/en 対訳)を出し、訳を
`ui/i18n/<locale>.json` で受け取り、表の生成と登録まで行います。
訳に欠けや不備のある言語は登録を拒みます:

```bash
python3 ui/gen_lang.py --todo     # 材料を出す
python3 ui/gen_lang.py nl         # ui/i18n/nl.json から生成・登録
python3 ui/gen_lang.py --check    # 登録済み全言語を検査
```

言語ごとの組版・校正の差は `lang` の `Language` に閉じてあります。
1つ実装すれば足ります。

## ライセンス

**AGPL-3.0-or-later**(`LICENSE`)。同梱・派生しているものの由来は `NOTICE.md`。

## いまの進み

リボンのコマンドは **writer 124/124・calc 145/145 — 両方ともグレー表示ゼロ、
全部動きます**(2026-08-04)。設計と判断の記録は `SEKKEI.md`、
経緯と残件は `HIKITSUGI.md`、試せる見本は `sample/`(手引きから辿れます)。

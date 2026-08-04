# 同梱・派生しているものと、その免許

このソフト本体は **AGPL-3.0-or-later**(`LICENSE`)。
**配りたい人が配れる**ようにするために、由来をここに書いておく。
表示を省くと、受け取った人が配れなくなる。

## 本体

| | |
|---|---|
| aiseed office | AGPL-3.0-or-later |

AGPL を選んだのは意図的で、**これはクラウドの月額に対する立場そのもの**である。
GPL では、誰かがこれを取り込んで**クラウドサービスとして提供**したとき、
利用者に原本を渡す義務が生じない。AGPL はそこを塞ぐ。
このソフトが反対しているのが「手元で動くはずのものを、月額の役務にすること」
である以上、そこを塞がない免許では筋が通らない。

**代償は承知の上**: AGPL は大手のソフトに取り込まれない。
hunspell が macOS・Chrome・Adobe に入れたのは、もっと緩い免許だったからで、
その道はこちらには無い。**取り込まれることではなく、置き換えることを狙う。**

## 取り込んでいるもの

| もの | ライセンス | どこ |
|---|---|---|
| **GPUI**(zed-industries) | Apache-2.0 | `vendor/zed/crates/gpui` |
| **hyphenation**(crate)+ TeX の分綴パターン(hyph-en-us) | コードは Apache-2.0/MIT。パターンは各言語の自由なライセンス(crate の patterns/ に原文) | `engine` の依存。英語のハイフネーションに使う |
| **resvg / usvg / tiny-skia**(crate) | MIT / Apache-2.0 | `ui` の依存。SVG を高精細の画像に直して貼るのに使う |
| **cfb / aes / sha1**(crate) | MIT / Apache-2.0 | `ooxml` の依存。docx の暗号化(ECMA-376 Standard)に使う |
| **encoding_rs**(crate) | MIT / Apache-2.0 | `writer` の依存。CP932 の HTML を読むのに使う |

Apache-2.0 は GPLv3 系へ一方向に両立するので、AGPL-3.0 の本体に取り込める
(逆はできない)。

## フォントは配らない

**本文のフォントは同梱していない。** 実行ファイルに埋め込むと、
それはフォントを配っていることになり、免許の表示義務が付いてくる。
**書体は文書の設定**であって、アプリの好みではない。
docx の `w:rFonts`、xlsx の `<font><name>` に名前が入っているので、それに従う。
`kumihan::font` がやるのは2つだけ:

  1. この機械にどんな書体があるか数える(リボンのフォント一覧になる)
  2. 名前から実体を引く(文書が指定した書体を出す)

名前はファイル名ではなく**フォント自身が名乗っている書体名**から取る
(`ipaexg.ttf` → 「IPAexゴシック」)。同名なら太字ではなく素の字面を採る。

文書の指定がこの機械に無いときは日本語が組めるものに落ちるが、
**落ちたことは黙らない。** 英字フォントで代用して、
日本語が豆腐になった画面を「動いている」と見せない。

## 派生しているもの

| もの | ライセンス | 何を |
|---|---|---|
| **Euro-Office / ONLYOFFICE web-apps** | AGPL-3.0 | `ui/src/ribbon.rs` の**タブ名とボタン名** |
| **同上** | AGPL-3.0 | `ui/icons/*.svg` の**アイコン**(88個) |

アイコンは `vendor/web-apps/apps/*/main/resources/img/toolbar/` から
使う分だけ取り出した(`ui/extract_icons.py`)。**単体配布なので実行ファイルに埋め込む。**
こちらも AGPL-3.0 なので取り込めるが、由来を書かずには配れない。

`ui/gen_ribbon.py` は `vendor/web-apps/apps/*/main/locale/<lang>.json` から
言葉を取り、`apps/*/main/app/template/Toolbar.template` から並び順を取って
`ribbon.rs` を起こす。**つまり `ribbon.rs` は Euro-Office の翻訳の派生物**である。

こちらも AGPL-3.0 なので両立するが、**由来を書かずに配ることはできない**。
リボンを Euro-Office に合わせているのは乗り換えのため
(使う人が場所を覚え直さずに済む)であって、翻訳を黙って使うためではない。

## 検査に使っているもの(配布物には入らない)

| もの | ライセンス・条件 |
|---|---|
| **青空文庫** のテキスト | 著作権消滅の作品のみ。`lang/corpus/fetch.py` が目録の `作品著作権フラグ` を見て、「あり」の928作品を除外する |
| **SCOWL / Moby / WordNet**(`/usr/share/dict/words`) | OS 側の `hunspell-en-us` などを読むだけ。同梱しない |

コーパスは `lang/corpus/txt/` に落ちるが、**版管理にも配布物にも入れない**
(`.gitignore`)。作品IDから誰でも同じものを取り直せる。

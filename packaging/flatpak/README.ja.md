# officework の Flatpak(試作)

2026-08-08 起こし。**まだ実機で組んでいない**(この機械には flatpak-builder が
無い)。ここにあるのは manifest の試作と、組むとき・審査に出すときの手順と
実証項目。配布方針の位置づけは SEKKEI「配布の第2チャネル」を見よ。

## なぜ Flatpak が成立するようになったか

2026-08-08 の「ブックが運べる Python は関数(UDF)だけ」の確定
(SEKKEI の Python 節)で、アプリの形がストアの流儀と一致した:

- 開く≠実行・ブック由来のコードは値計算だけ・網なし・時間制限つき
- 手続きは利用者が自分で plugins に置いた物だけ
- Python はランタイム同梱(外から取らない)

## 檻の二層構造(ここが肝)

- **外側** = この manifest の finish-args。アプリ自身が働ける広さ
  (帳票の読み書き・自分の道具の網)
- **内側** = calc が他所から来たかもしれないコードに掛ける檻。
  素の Linux では bubblewrap、**Flatpak の中では bwrap の入れ子が
  動かない**ので `flatpak-spawn --sandbox` に自動で切り替わる
  (calc/src/py.rs の cage_kind / caged_python。/.flatpak-info で見分ける)。
  そのために `--talk-name=org.freedesktop.Flatpak` が必要

## 組む手順(flatpak-builder のある機械で)

1. ビルド中は網が無いので cargo の荷物を先に固める:
   [flatpak-cargo-generator](https://github.com/flatpak/flatpak-builder-tools)
   で `Cargo.lock` から `cargo-sources.json` を作り、manifest の sources に足す
2. 同様に Python の荷物(polars ほか .venv 相当)を flatpak-pip-generator で
   `python3-modules.json` にして modules に足す
3. `flatpak-builder --user --install build-dir io.github.aiseed_dev.officework.yml`
4. `flatpak run io.github.aiseed_dev.officework`

## 実証項目(この順で。**通るまで「対応」と言わない**)

1. **内側の檻が効くか**(いちばん大事):
   - `@計算`(UDF)がアプリの中から通るか — flatpak-spawn --sandbox +
     `--sandbox-expose=作業場` で in/out の受け渡しができるか。
     作業場は `~/.var/app/$ID/sandbox/` の下(py.rs の cage_work_dir)
   - `--no-network` で本当に網が切れるか(urllib で外に出て失敗する事を見る)
   - サンドボックスからホームの実ファイルが見えない事
2. **rfd のファイルダイアログ**がポータル経由で開くか。開けるなら
   finish-args の `--filesystem=home` を外してポータルに絞る(狭い方が良い)
3. **GPUI(blade/Vulkan)** が `--device=dri` で描けるか。Wayland と X11 両方
4. 排他ロック(開いているブックの .lock)が共有フォルダで従来どおり働くか
5. Flathub 申請の残り: アイコン(scalable SVG)、metainfo の screenshots、
   summary/description の磨き、OARS の回答

## Mac App Store は?

追いかけない事にしたのではなく**順番が後**(発注者 2026-08-08 の議論)。
App Sandbox では子プロセスが親の檻を継承するので、「内側の檻」の Mac 実装
(entitlements 設計)と一緒にやるのが効率的。公証つき .dmg + cask が先にある。

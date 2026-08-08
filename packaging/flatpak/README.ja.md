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
3. **cargo-sources.json の gpui の取り違えを直す**(踏んだ穴。下の「踏み跡」)
4. `flatpak run org.flatpak.Builder --user --force-clean --disable-rofiles-fuse
   build-dir io.github.aiseed_dev.officework.yml`
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
5. **AI メニューの claude CLI**: 中からホストの claude を呼ぶには
   `flatpak-spawn --host` が要る。--sandbox と同じ talk-name の口だが、
   --host は「ホストで任意のコマンド」の重い権限で **Flathub 審査で見られる**。
   AI 無しでも製品は成立する作りなので、通らなければストア版は AI 宛先を
   「Claude API 直」に絞る(2026-08-04 の検討どおり)
6. ホストのフォント: Flatpak は /run/host/fonts に見せてくる。
   kumihan::font の解決がそこを拾えるか(書体は名指ししない方針なので、
   fontconfig が通れば素通りのはず — 実機で見る)
7. Flathub 申請の残り: アイコン(scalable SVG)、metainfo の screenshots、
   summary/description の磨き、OARS の回答

## Mac App Store は?

追いかけない事にしたのではなく**順番が後**(発注者 2026-08-08 の議論)。
App Sandbox では子プロセスが親の檻を継承するので、「内側の檻」の Mac 実装
(entitlements 設計)と一緒にやるのが効率的。公証つき .dmg + cask が先にある。

## 踏み跡(2026-08-08 に実際に組んで分かったこと)

**org.flatpak.Builder は sudo 無しで入る。** ただし `flatpak install --user` は
**ユーザー側の remote しか見ない** — システムに flathub が登録済みでも
`flatpak remote-add --user flathub …` が別に要る(最初これで空振りした)。
runtime は `org.gnome.Platform//47` `org.gnome.Sdk//47` と
`org.freedesktop.Sdk.Extension.rust-stable//24.08`。
`--no-related` を付けないと翻訳だけ入って本体が入らないことがある。

**cargo-sources.json は gpui を取り違える(要 手直し)。**
zed の単一リポジトリには **`gpui` という名前の package が2つ**ある —
本物の `crates/gpui`(version 0.2.2)と、lint の試験材料
`tooling/lints/test_fixture/gpui`(version 0.0.0)。
flatpak-cargo-generator は後者を拾うので、そのままだと

    error: failed to select a version for the requirement `gpui = "*"`
    (locked to 0.2.2) candidate versions found which didn't match: 0.0.0

で落ちる。`gpui_shared_string` も同じ罠。生成のたびに:

    python3 - <<'EOF'
    import json; p='packaging/flatpak/cargo-sources.json'
    d=json.load(open(p))
    for x in d:
        if x.get('commands'):
            x['commands']=[c.replace("/tooling/lints/test_fixture/","/crates/")
                           for c in x['commands']]
    json.dump(d, open(p,'w'), ensure_ascii=False, indent=1)
    EOF

**この直しは生成物への手当てなので、Cargo.lock を更新したら毎回やり直す。**

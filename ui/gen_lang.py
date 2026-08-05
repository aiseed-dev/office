#!/usr/bin/env python3
"""言語を1コマンドで増やす生成プログラム。

材料(訳)は `ui/i18n/<locale>.json` — 番号つきの訳の一覧:

    [{"i": 0, "t": "訳…"}, …]

番号は `--todo` が出す材料ファイル(ja と en の対訳つき)の番号。
訳を書くのは人か AI(claude の定額 = Claude Code / サブエージェント)の仕事。

    python3 ui/gen_lang.py --todo             # 材料 ui/i18n/keys.json を出す
    python3 ui/gen_lang.py zh                 # ui/i18n/zh.json から生成・登録
    python3 ui/gen_lang.py --check            # 登録済み全言語の表を検査

生成するもの(すべて「手で書かない」ファイル):
- ui/src/ribbon_<loc>.rs — リボン(標準の釦は vendor の本家対訳、
  こちら独自の釦は材料の訳)
- lang/src/i18n_<loc>.rs — 文言の対訳表
- lang/src/i18n_tables.rs / ui/src/ribbon_tables.rs — 登録簿
- lang/src/lib.rs / ui/src/lib.rs の gen_lang:begin〜end 区間

検査(止まる条件): 訳の欠け・番号のずれ・穴埋め {} の数の不一致・
vendor に無い標準釦の語。**表の揃わない言語は登録しない**(家訓)。
"""
import json
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
I18N_DIR = HERE / "i18n"
EN_TABLE = ROOT / "lang/src/i18n_en.rs"

sys.path.insert(0, str(HERE))
import gen_ribbon_locale as grl  # noqa: E402


# ---- 材料の鍵(番号 → 原文)------------------------------------------------

def literal_at(src, i):
    j = i + 1
    while j < len(src):
        if src[j] == "\\":
            j += 2
            continue
        if src[j] == '"':
            return j + 1, src[i:j + 1]
        j += 1
    raise ValueError("unterminated literal")


def en_pairs():
    """i18n_en.rs から (鍵のソースリテラル, en のソースリテラル) を順に"""
    src = open(EN_TABLE, encoding="utf-8").read()
    out = []
    i = src.find("pub const")
    while True:
        i = src.find('("', i)
        if i < 0:
            break
        j, key = literal_at(src, i + 1)
        k = src.find('"', j)
        j2, val = literal_at(src, k)
        out.append((key, val))
        i = j2
    return out


def unescape(lit):
    """Rust のソースリテラル → 実行時の文字列(\\n・\\"・行継続)"""
    body = lit[1:-1]
    out = []
    i = 0
    while i < len(body):
        c = body[i]
        if c == "\\":
            n = body[i + 1]
            if n == "\n":
                i += 2
                while i < len(body) and body[i] in " \t":
                    i += 1
                continue
            out.append({"n": "\n", "t": "\t", '"': '"', "\\": "\\"}.get(n, n))
            i += 2
            continue
        out.append(c)
        i += 1
    return "".join(out)


def escape(s):
    """実行時の文字列 → Rust のソースリテラル"""
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n") + '"'


def material():
    """番号つきの材料: 文言(i18n_en の鍵順)+ こちら独自の釦の語"""
    rows = []
    for key, en in en_pairs():
        rows.append({"kind": "msg", "key": key, "ja": unescape(key), "en": unescape(en)})
    for ja, en in grl.OVERRIDES["en"].items():
        rows.append({"kind": "ribbon", "key": None, "ja": ja, "en": en})
    for i, r in enumerate(rows):
        r["i"] = i
    return rows


def holes(s):
    """穴埋めの並び({}・{:…})。訳と原文で一致しなければならない"""
    return re.findall(r"\{\{|\}\}|\{[^}]*\}", s)


# ---- 生成 -------------------------------------------------------------------

def mod_name(loc):
    return loc.replace("-", "_")


def registered():
    """生成済みの言語(en + ribbon/i18n の対が揃っているもの)"""
    langs = []
    for p in sorted((ROOT / "ui/src").glob("ribbon_*.rs")):
        if p.name == "ribbon_tables.rs":
            continue
        m = p.stem[len("ribbon_"):]
        loc = m.replace("_", "-")
        if (ROOT / f"lang/src/i18n_{m}.rs").exists():
            langs.append(loc)
    return langs


def write_registries():
    langs = registered()
    # lang 側
    body = [
        "//! 文言の対訳表の登録簿。**このファイルは ui/gen_lang.py が生成する。**",
        "//! 手で書かない — 言語を足すときは gen_lang.py を回す。",
        "",
        "/// 表の揃った言語(ja は鍵そのものなので載らない)",
        "pub const LANGS: &[&str] = &[" + ", ".join(f'"{l}"' for l in langs) + "];",
        "",
        "pub fn table(lang: &str) -> Option<&'static [(&'static str, &'static str)]> {",
        "    match lang {",
    ]
    for l in langs:
        body.append(f'        "{l}" => Some(crate::i18n_{mod_name(l)}::TABLE),')
    body += ["        _ => None,", "    }", "}", ""]
    (ROOT / "lang/src/i18n_tables.rs").write_text("\n".join(body), encoding="utf-8")

    # ui 側
    body = [
        "//! リボンの表の登録簿。**このファイルは ui/gen_lang.py が生成する。**",
        "//! 手で書かない — 言語を足すときは gen_lang.py を回す。",
        "",
        "use super::ribbon::Tab;",
        "",
        "pub fn tabs(lang: &str) -> Option<(&'static [Tab], &'static [Tab])> {",
        "    match lang {",
    ]
    for l in langs:
        m = mod_name(l)
        body.append(
            f'        "{l}" => Some((crate::ribbon_{m}::WRITER, crate::ribbon_{m}::CALC)),')
    body += ["        _ => None,", "    }", "}", ""]
    (ROOT / "ui/src/ribbon_tables.rs").write_text("\n".join(body), encoding="utf-8")

    # lib.rs の目印区間
    def patch(path, lines):
        src = open(path, encoding="utf-8").read()
        begin = src.index("// gen_lang:begin")
        begin = src.index("\n", begin) + 1
        end = src.index("// gen_lang:end")
        open(path, "w", encoding="utf-8").write(src[:begin] + lines + src[end:])

    patch(ROOT / "lang/src/lib.rs",
          "".join(f"pub mod i18n_{mod_name(l)};\n" for l in langs)
          + "pub mod i18n_tables;\n")
    patch(ROOT / "ui/src/lib.rs",
          "".join(f"pub mod ribbon_{mod_name(l)};\n" for l in langs))


def check_table(loc):
    """i18n_<loc>.rs が en と同じ鍵集合か・穴埋めが揃っているか"""
    m = mod_name(loc)
    if loc == "en":
        keys = {k for k, _ in en_pairs()}
        pairs = en_pairs()
    else:
        src = open(ROOT / f"lang/src/i18n_{m}.rs", encoding="utf-8").read()
        pairs = []
        i = src.find("pub const TABLE")
        while True:
            i = src.find('("', i)
            if i < 0:
                break
            j, key = literal_at(src, i + 1)
            k = src.find('"', j)
            j2, val = literal_at(src, k)
            pairs.append((key, val))
            i = j2
        keys = {k for k, _ in pairs}
        want = {k for k, _ in en_pairs()}
        if keys != want:
            miss = sorted(want - keys)[:5]
            extra = sorted(keys - want)[:5]
            sys.exit(f"{loc}: 鍵がずれています(欠け {len(want-keys)} 例 {miss} / "
                     f"余分 {len(keys-want)} 例 {extra})")
    for k, v in pairs:
        if not unescape(v).strip() and unescape(k).strip():
            sys.exit(f"{loc}: 空の訳があります: {k[:40]}")
        if holes(unescape(k)) != holes(unescape(v)):
            sys.exit(f"{loc}: 穴埋めが合いません: {k[:60]}")
    return len(pairs)


def load_translations(loc):
    """訳の JSON を読み、材料に対して検査する(欠け・空・穴埋め)"""
    mat = material()
    tr_path = I18N_DIR / f"{loc}.json"
    if not tr_path.exists():
        sys.exit(f"訳の材料がありません: {tr_path}(--todo で鍵を出し、訳を書く)")
    got = {r["i"]: r["t"] for r in json.load(open(tr_path, encoding="utf-8"))}
    missing = [r["i"] for r in mat if r["i"] not in got]
    if missing:
        sys.exit(f"{loc}: 訳の欠けが {len(missing)} 個(番号 {missing[:10]} …)")
    bad = []
    for r in mat:
        t = got[r["i"]]
        if not t.strip() and r["ja"].strip():
            bad.append(f"番号 {r['i']}: 空の訳({r['ja'][:30]})")
        elif holes(r["ja"]) != holes(t):
            bad.append(f"番号 {r['i']}: 穴埋めが合いません({r['ja'][:40]} → {t[:40]})")
    if bad:
        sys.exit(f"{loc}: 検査で {len(bad)} 件:\n  " + "\n  ".join(bad[:20]))
    return mat, got


def generate(loc):
    mat, got = load_translations(loc)

    m = mod_name(loc)
    # 文言の表
    body = [
        f"//! 画面の文言の {loc} の対訳表 — **鍵は日本語の文そのもの**。",
        "//! このファイルは ui/gen_lang.py が生成する(訳の材料は"
        f" ui/i18n/{loc}.json)。手で書かない。",
        "",
        "pub const TABLE: &[(&str, &str)] = &[",
    ]
    for r in mat:
        if r["kind"] != "msg":
            continue
        body.append(f"    ({r['key']}, {escape(got[r['i']])}),")
    body += ["];", ""]
    (ROOT / f"lang/src/i18n_{m}.rs").write_text("\n".join(body), encoding="utf-8")

    # リボン(標準の釦は vendor、独自の釦は材料の訳)
    # 材料の訳 + スクリプトに書いた穴埋め(vendor のロケールに無い語)を併用
    extra = grl.OVERRIDES.get(loc, {})
    grl.OVERRIDES[loc] = {
        **extra,
        **{r["ja"]: got[r["i"]] for r in mat if r["kind"] == "ribbon"},
    }
    old_argv = sys.argv
    import io
    from contextlib import redirect_stdout
    sys.argv = ["gen_ribbon_locale.py", loc]
    buf = io.StringIO()
    try:
        with redirect_stdout(buf):
            grl.main()
    finally:
        sys.argv = old_argv
    (ROOT / f"ui/src/ribbon_{m}.rs").write_text(buf.getvalue(), encoding="utf-8")

    write_registries()
    n = check_table(loc)
    print(f"{loc}: 文言 {n} 句 + リボンを生成・登録しました")


def main():
    I18N_DIR.mkdir(exist_ok=True)
    if "--todo" in sys.argv:
        mat = material()
        out = [{"i": r["i"], "ja": r["ja"], "en": r["en"]} for r in mat]
        p = I18N_DIR / "keys.json"
        p.write_text(json.dumps(out, ensure_ascii=False, indent=1), encoding="utf-8")
        print(f"{p} に {len(out)} 句(訳は ui/i18n/<locale>.json に"
              ' [{"i": 番号, "t": "訳"}] で)')
        return
    if "--validate" in sys.argv:
        # 生成せず訳だけ検査(並列の翻訳作業向け — 共有ファイルに触れない)
        for loc in [a for a in sys.argv[1:] if not a.startswith("-")]:
            load_translations(loc)
            print(f"{loc}: 訳は完全です(生成は gen_lang.py {loc} で)")
        return
    if "--check" in sys.argv:
        for l in ["en"] + [l for l in registered() if l != "en"]:
            n = check_table(l)
            print(f"{l}: OK({n} 句)")
        return
    locs = [a for a in sys.argv[1:] if not a.startswith("-")]
    if not locs:
        sys.exit("使い方: gen_lang.py <locale>… | --todo | --check")
    for loc in locs:
        generate(loc)


if __name__ == "__main__":
    main()

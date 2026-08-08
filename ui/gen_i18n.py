#!/usr/bin/env python3
"""画面の文言の対訳表の門番。

アプリの `ui::t!("…")` / `ui::tf!("…", …)` から**日本語の鍵**を全部抽出し、
lang/src/i18n_en.rs の対訳表と突き合わせる。

    python3 ui/gen_i18n.py            # 検査(未訳・不要訳があれば止まる)
    python3 ui/gen_i18n.py --missing  # 未訳の鍵を骨組み(("鍵", ""),)で出す

**未訳があるうちは en を名乗れない**(文言の揃った言語だけを名乗る方針)。
新しい文言を足したら、--missing の骨組みに訳を書いて表へ足すこと。
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
# calc は部屋割り済み(2026-08-06)— src の全部屋を見る(試験は除く)
SOURCES = [ROOT / "writer/src/main.rs"] + sorted(
    p for p in (ROOT / "calc/src").glob("*.rs") if p.name != "tests.rs"
)
TABLE = ROOT / "lang/src/i18n_en.rs"


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


def keys_from(path):
    src = open(path, encoding="utf-8").read()
    # 試験モジュールから先は見ない。ただし `#[cfg(test)] mod tests;` の
    # **宣言**は本文の頭にある(calc の部屋割り)— そこでは切らない
    pos = 0
    while True:
        cut = src.find("#[cfg(test)]", pos)
        if cut < 0:
            break
        lines = src[cut:cut + 64].splitlines()
        next_line = lines[1].strip() if len(lines) > 1 else ""
        if next_line.startswith("mod ") and next_line.endswith(";"):
            pos = cut + 1  # 宣言 — 本文はこの先も続く
            continue
        src = src[:cut]
        break
    out = []
    for m in re.finditer(r"ui::tf?!\(\s*", src):
        j = m.end()
        if j < len(src) and src[j] == '"':
            _, lit = literal_at(src, j)
            out.append(lit)
    return out


def table_keys():
    """表の各行の鍵リテラルを、リテラル走査で取り出す(複数行の鍵も)"""
    src = open(TABLE, encoding="utf-8").read()
    out = []
    i = src.find("pub const")
    while True:
        i = src.find('("', i)
        if i < 0:
            break
        j, lit = literal_at(src, i + 1)
        out.append(lit)
        # 値のリテラルを読み飛ばす
        k = src.find('"', j)
        if k < 0:
            break
        i, _ = literal_at(src, k)
    return out


def main():
    used = []
    for p in SOURCES:
        used.extend(keys_from(p))
    used_set = dict.fromkeys(used)  # 順を保った一意化
    table = table_keys()
    table_set = set(table)

    missing = [k for k in used_set if k not in table_set]
    extra = [k for k in table_set if k not in used_set]
    dup = {k for k in table if table.count(k) > 1}

    if "--missing" in sys.argv:
        for k in missing:
            print(f"    ({k}, \"\"),")
        return

    ok = True
    if missing:
        ok = False
        print(f"未訳の鍵が {len(missing)} 個(--missing で骨組みを出せます)")
    if extra:
        ok = False
        print(f"使われていない訳が {len(extra)} 個:")
        for k in sorted(extra)[:20]:
            print(f"  {k}")
    if dup:
        ok = False
        print(f"重複した鍵が {len(dup)} 個: {sorted(dup)[:5]}")
    if not ok:
        sys.exit(1)
    print(f"OK: {len(used_set)} 句すべてに訳がある")


if __name__ == "__main__":
    main()

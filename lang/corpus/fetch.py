#!/usr/bin/env python3
"""青空文庫からベンチマーク用のコーパスを作る。

入力者が人手で付けたルビが正解になる。作品IDを書くだけで**誰でも同じ入力を再現できる**。

  python3 fetch.py 789 773 127 456 301

**著作権の確認をここでやる。** 全17,835作品のうち「なし」は17,347、
残り928作品は著作権が存続している。目録CSVの `作品著作権フラグ` を見て、
「あり」の作品は落とす。テキストを見ても分からないので、ここが唯一の関門。

青空文庫の本文は Shift_JIS。Rust 側は UTF-8 だけを読むので、ここで変換する
(そのためだけに Rust へ文字コード変換の依存を持ち込まない)。
"""

import csv
import io
import pathlib
import re
import sys
import urllib.request
import zipfile

INDEX = "https://www.aozora.gr.jp/index_pages/list_person_all_extended_utf8.zip"
OUT = pathlib.Path(__file__).parent / "txt"


def index():
    """目録を読む。作品ID → (作品名, 著作権フラグ, テキストURL)"""
    raw = urllib.request.urlopen(INDEX, timeout=120).read()
    z = zipfile.ZipFile(io.BytesIO(raw))
    rows = csv.DictReader(io.StringIO(z.read(z.namelist()[0]).decode("utf-8-sig")))
    out = {}
    for r in rows:
        out[r["作品ID"].lstrip("0")] = (
            r["作品名"],
            r.get("作品著作権フラグ", "あり"),
            r.get("テキストファイルURL", ""),
        )
    return out


def main(ids):
    OUT.mkdir(parents=True, exist_ok=True)
    idx = index()
    print(f"目録: {len(idx):,} 作品")
    kept = skipped = 0
    for wid in ids:
        key = wid.lstrip("0")
        if key not in idx:
            print(f"  {wid}: 目録にありません")
            continue
        title, flag, url = idx[key]
        if flag != "なし":
            # ここが唯一の関門。落としたことを黙らない
            print(f"  {wid} {title}: 著作権フラグ「{flag}」→ 除外")
            skipped += 1
            continue
        if not url.endswith(".zip"):
            print(f"  {wid} {title}: テキストが無い({url or 'URL空'})")
            continue
        try:
            raw = urllib.request.urlopen(url, timeout=120).read()
            z = zipfile.ZipFile(io.BytesIO(raw))
            name = [n for n in z.namelist() if n.lower().endswith(".txt")][0]
            text = z.read(name).decode("shift_jis", errors="replace")
        except Exception as e:  # noqa: BLE001
            print(f"  {wid} {title}: 取得できません {e}")
            continue
        safe = re.sub(r"[^\w一-龥ぁ-んァ-ン]+", "_", title)[:40]
        p = OUT / f"{key}_{safe}.txt"
        p.write_text(text, encoding="utf-8")
        ruby = len(re.findall(r"《[^》]+》", text))
        print(f"  {wid} {title}: {len(text):,} 文字, ルビ {ruby:,} 箇所 → {p.name}")
        kept += 1
    print(f"\n取得 {kept} 作品 / 著作権で除外 {skipped} 作品 → {OUT}")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(__doc__)
        # 既定: 漱石・芥川・賢治・太宰。ルビの量と文体がばらける組み合わせ
        print("既定の5作品を取ります")
        main(["789", "773", "127", "456", "301"])
    else:
        main(sys.argv[1:])

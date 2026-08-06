# 関数の一覧表(calc/src/funcs.rs)を Euro-Office の現物から起こす。
#
#   python3 calc/gen_funcs.py > calc/src/funcs.rs
#
# 引数と説明は vendor/web-apps の formula-lang/ja_desc.json(本家の日本語)。
# **載せるのは calc が実際に計算できる関数だけ**(できないものを見せない)。
# 分類はうちの数式タブの族(fn-math 等)と同じ割り付け。
import json
import sys

JA = "vendor/web-apps/apps/spreadsheeteditor/main/resources/formula-lang/ja_desc.json"

# 使える関数(sheet/src/calc.rs が計算できるもの)を分類ごとに。
# リボンの fn-* の一覧(「使える名前だけを出す」)と同じ中身
GROUPS = {
    "数学": "SUM ROUND ROUNDUP ROUNDDOWN INT ABS MOD POWER SQRT "
            "PRODUCT SUMPRODUCT SUMSQ CEILING FLOOR MROUND EVEN ODD SIGN "
            "FACT COMBIN PERMUT GCD LCM PI SIN COS TAN ASIN ACOS ATAN ATAN2 "
            "SINH COSH TANH EXP LN LOG LOG10 DEGREES RADIANS RAND RANDBETWEEN "
            "SEQUENCE TRUNC QUOTIENT CEILING.MATH FLOOR.MATH SUBTOTAL",
    "統計": "AVERAGE COUNT MAX MIN COUNTA COUNTBLANK SUMIF SUMIFS COUNTIF "
            "COUNTIFS AVERAGEIF AVERAGEIFS MINIFS MAXIFS "
            "RANK RANK.EQ RANK.AVG LARGE SMALL MEDIAN MODE STDEV STDEVP "
            "VAR VARP PERCENTILE QUARTILE CORREL SLOPE INTERCEPT FORECAST "
            "AVERAGEA MAXA MINA",
    "文字列": "LEN LEFT RIGHT MID TRIM UPPER LOWER CONCATENATE CONCAT TEXT "
              "SUBSTITUTE FIND SEARCH VALUE TEXTJOIN REPT CHAR CODE "
              "UNICHAR UNICODE PROPER EXACT CLEAN FIXED YEN NUMBERVALUE "
              "LENB LEFTB RIGHTB MIDB ASC JIS DATESTRING PHONETIC",
    "論理": "IF IFS SWITCH AND OR NOT TRUE FALSE IFERROR IFNA",
    "日付": "TODAY NOW DATE DATEVALUE YEAR MONTH DAY WEEKDAY "
            "TIME HOUR MINUTE SECOND EDATE EOMONTH DATEDIF "
            "WORKDAY NETWORKDAYS DAYS DAYS360 YEARFRAC WEEKNUM ISOWEEKNUM",
    "検索": "VLOOKUP HLOOKUP XLOOKUP LOOKUP INDEX MATCH CHOOSE "
            "ROW COLUMN ROWS COLUMNS OFFSET INDIRECT ADDRESS HYPERLINK "
            "FILTER SORT UNIQUE TRANSPOSE",
    "財務": "PMT PV FV NPER NPV IRR RATE",
    "情報": "ISBLANK ISERROR ISNA ISERR ISLOGICAL ISNONTEXT ISNUMBER ISTEXT "
            "ISEVEN ISODD NA T N TYPE",
}

ja = json.load(open(JA, encoding="utf-8"))

# 本家の表に無い(日本語まわりの)関数は、こちらで書く
HAND = {
    "YEN": {"a": "(数値, [桁数])", "d": "数値を円記号(¥)と桁区切りを付けた文字列にします。"},
    "JIS": {"a": "(文字列)", "d": "半角(1 バイト)文字を全角(2 バイト)文字に変換します。"},
    "DATESTRING": {"a": "(シリアル値)", "d": "日付を和暦の文字列にして返します。"},
    "PHONETIC": {"a": "(範囲)", "d": "セルのふりがなを返します(読み込んだ xlsx のふりがな情報を引きます)。"},
}
ja.update(HAND)

def esc(s: str) -> str:
    return s.replace("\\", "\\\\").replace('"', '\\"')

rows = []
missing = []
for group, names in GROUPS.items():
    for name in names.split():
        info = ja.get(name)
        if info is None:
            missing.append(name)
            args, desc, ads = "(…)", "(この関数の説明は本家の表にありません)", []
        else:
            args = info.get("a", "(…)").replace("; ", ", ")
            desc = info.get("d", "")
            # 引数ごとの説明。本家は ! 区切りで引数順に並ぶ
            ads = [s for s in info.get("ad", "").split("!") if s.strip()]
        rows.append((name, group, args, desc, ads))
rows.sort(key=lambda r: r[0])

print("//! 関数の一覧(名前・分類・引数・説明)。**このファイルは手で書かない** —")
print("//! `python3 calc/gen_funcs.py > calc/src/funcs.rs` が")
print("//! Euro-Office の formula-lang/ja_desc.json(本家の日本語)から起こす。")
print("//! 載っているのは calc が実際に計算できる関数だけ。")
print()
print("pub struct FnInfo {")
print("    pub name: &'static str,")
print("    pub group: &'static str,")
print("    pub args: &'static str,")
print("    pub desc: &'static str,")
print("    /// 引数ごとの説明(引数の並び順。可変長引数は最後の1つが代表)")
print("    pub arg_desc: &'static [&'static str],")
print("}")
print()
print(f"pub static FUNCS: &[FnInfo] = &[  // {len(rows)} 関数")
for name, group, args, desc, ads in rows:
    ad = ", ".join(f'"{esc(a)}"' for a in ads)
    print(f'    FnInfo {{ name: "{esc(name)}", group: "{esc(group)}", '
          f'args: "{esc(args)}", desc: "{esc(desc)}", arg_desc: &[{ad}] }},')
print("];")

if missing:
    print(f"本家の表に無い: {' '.join(missing)}", file=sys.stderr)

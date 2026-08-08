//! main.rs からの純移動(2026-08-06 の分割)。挙動は変えない。

use crate::*;

/// PY の引数を Python の書き方(リテラル)にする。
pub(crate) fn py_literal(v: &sheet::Value) -> String {
    match v {
        sheet::Value::Number(n) => format!("{n}"),
        sheet::Value::Bool(b) => (if *b { "True" } else { "False" }).into(),
        sheet::Value::Empty => "None".into(),
        v => format!("{:?}", v.display()), // Rust の {:?} は Python でも読める逃がし
    }
}

/// @計算 の台本。「関数」スクリプトの def を読み、各 PY セルを評価して
/// 区切りの印(\x1c セル / \x1e 行 / \x1f 欄)で吐く。
pub(crate) fn build_udf_script(
    defs: &str,
    calls: &[(String, String, Vec<sheet::calc::PyArg>)], // (セルA1, 関数名, 引数)
    out_path: &std::path::Path,
) -> String {
    let mut body = String::new();
    for (cell, fname, args) in calls {
        let mut lit_args = Vec::new();
        for a in args {
            match a {
                sheet::calc::PyArg::One(v) => lit_args.push(py_literal(v)),
                sheet::calc::PyArg::Rect(cols, vs) => {
                    let cols = (*cols as usize).max(1);
                    let rows: Vec<String> = vs
                        .chunks(cols)
                        .map(|row| {
                            format!(
                                "[{}]",
                                row.iter().map(py_literal).collect::<Vec<_>>().join(",")
                            )
                        })
                        .collect();
                    lit_args.push(format!("[{}]", rows.join(",")));
                }
            }
        }
        body.push_str(&format!(
            "_jo_emit({cell:?}, {fname}({args}))\n",
            cell = cell,
            fname = fname,
            args = lit_args.join(", ")
        ));
    }
    format!(
        concat!(
            "# aiseed calc の PY(UDF)評価。関数の定義はブックの「関数」スクリプト\n",
            "{defs}\n",
            "_jo_out = []\n",
            "def _jo_emit(cell, r):\n",
            "    if not isinstance(r, (list, tuple)):\n",
            "        r = [[r]]\n",
            "    elif r and not isinstance(r[0], (list, tuple)):\n",
            "        r = [[v] for v in r]  # 1次元は縦に広げる\n",
            "    rows = ['\\x1f'.join('' if v is None else str(v) for v in row) for row in r]\n",
            "    _jo_out.append(cell + '\\x1e' + '\\x1e'.join(rows))\n",
            "{body}\n",
            "open({out:?}, 'w', encoding='utf-8').write('\\x1c'.join(_jo_out))\n"
        ),
        defs = defs,
        body = body,
        out = out_path.to_string_lossy()
    )
}

/// 台本の出力を (セル, 行×欄の文字) に戻す。
pub(crate) fn parse_udf_output(raw: &str) -> Vec<(Pos, Vec<Vec<String>>)> {
    raw.split('\u{1c}')
        .filter_map(|rec| {
            let mut it = rec.split('\u{1e}');
            let cell = Pos::parse(it.next()?)?;
            let rows: Vec<Vec<String>> = it
                .map(|r| r.split('\u{1f}').map(|v| v.to_string()).collect())
                .collect();
            (!rows.is_empty()).then_some((cell, rows))
        })
        .collect()
}

/// PY の結果をシートへ。錨のセルは**式を保ったまま**値を差し替え、
/// 2次元はスピル(右下へ展開)。他人のデータを潰しそうなら #SPILL! で止まる。
/// 返すのは (新しいスピルの台帳, 適用した数, 衝突した数)。
pub(crate) fn apply_py_results(
    sh: &mut sheet::Sheet,
    results: &[(Pos, Vec<Vec<String>>)],
    prev: &std::collections::HashMap<Pos, (u32, u32)>,
) -> (std::collections::HashMap<Pos, (u32, u32)>, usize, usize) {
    // 前回のスピル面(錨以外)をまず消す(小さくなったとき古い値を残さない)
    for (anchor, (rows, cols)) in prev {
        for dr in 0..*rows {
            for dc in 0..*cols {
                if dr == 0 && dc == 0 {
                    continue;
                }
                let p = Pos::new(anchor.row + dr, anchor.col + dc);
                if let Some(c) = sh.cells.get_mut(&p) {
                    if c.formula.is_none() {
                        c.value = sheet::Value::Empty;
                    }
                }
            }
        }
    }
    let mut spills = std::collections::HashMap::new();
    let (mut applied, mut conflicts) = (0usize, 0usize);
    for (anchor, rows) in results {
        let (nr, nc) = (rows.len() as u32, rows.iter().map(|r| r.len()).max().unwrap_or(1) as u32);
        // 衝突検査(錨以外に、中身か式のあるセルが居ないか)
        let mut blocked = false;
        for dr in 0..nr {
            for dc in 0..nc {
                if dr == 0 && dc == 0 {
                    continue;
                }
                let p = Pos::new(anchor.row + dr, anchor.col + dc);
                if let Some(c) = sh.cells.get(&p) {
                    let was_prev_spill = prev
                        .get(anchor)
                        .is_some_and(|(pr, pc)| dr < *pr && dc < *pc);
                    if c.formula.is_some() || (!c.value.is_empty() && !was_prev_spill) {
                        blocked = true;
                    }
                }
            }
        }
        let put = |sh: &mut sheet::Sheet, p: Pos, text: &str| {
            let fmt = sh.get(p).map(|c| c.fmt.clone()).unwrap_or_default();
            let formula = sh.get(p).and_then(|c| c.formula.clone());
            let value = if text.is_empty() {
                sheet::Value::Empty
            } else if let Ok(n) = text.parse::<f64>() {
                sheet::Value::Number(n)
            } else {
                sheet::Value::Text(text.to_string())
            };
            sh.set(p, sheet::Cell { formula, value, fmt });
        };
        if blocked {
            let fmt = sh.get(*anchor).map(|c| c.fmt.clone()).unwrap_or_default();
            let formula = sh.get(*anchor).and_then(|c| c.formula.clone());
            sh.set(
                *anchor,
                sheet::Cell {
                    formula,
                    value: sheet::Value::Error("#SPILL!".into()),
                    fmt,
                },
            );
            conflicts += 1;
            continue;
        }
        for (dr, row) in rows.iter().enumerate() {
            for (dc, text) in row.iter().enumerate() {
                put(sh, Pos::new(anchor.row + dr as u32, anchor.col + dc as u32), text);
            }
        }
        if nr > 1 || nc > 1 {
            spills.insert(*anchor, (nr, nc));
        }
        applied += 1;
    }
    (spills, applied, conflicts)
}

/// 檻の種類。**Flatpak の中では bwrap の入れ子が動かない**(ユーザー名前空間の
/// 入れ子が塞がれている)ので、そこでは公式の入れ子口 flatpak-spawn --sandbox
/// を使う。どちらも組めなければ None — 他所から来たかもしれないコードは
/// 檻の外では実行しない(そう言う)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Cage {
    /// 素の Linux: /usr/bin/bwrap
    Bwrap,
    /// Flatpak の中: flatpak-spawn --sandbox(実機での実証はまだ —
    /// packaging/flatpak/README.md の実証項目を参照)
    Flatpak,
    /// 檻が組めない
    None,
}

/// いまの環境で組める檻。Flatpak の中かは /.flatpak-info で見分ける(公式の印)
pub(crate) fn cage_kind() -> Cage {
    if std::path::Path::new("/.flatpak-info").exists() {
        // flatpak-spawn は flatpak-xdg-utils の道具。Flatpak の runtime には居る
        if std::path::Path::new("/usr/bin/flatpak-spawn").exists() {
            Cage::Flatpak
        } else {
            Cage::None
        }
    } else if std::path::Path::new("/usr/bin/bwrap").exists() {
        Cage::Bwrap
    } else {
        Cage::None
    }
}

/// 檻つき実行の作業場(交換用の読み書き領域)。Flatpak の --sandbox-expose は
/// **~/.var/app/$ID/sandbox の下しか見せられない**ので、置き場が檻で変わる
pub(crate) fn cage_work_dir(tag: &str) -> PathBuf {
    match cage_kind() {
        Cage::Flatpak => {
            // XDG_DATA_HOME = ~/.var/app/$ID/data(Flatpak が設定する)。
            // その親の sandbox/ が flatpak-spawn に見せられる場所
            let app = std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .and_then(|d| d.parent().map(|p| p.to_path_buf()))
                .unwrap_or_else(std::env::temp_dir);
            app.join("sandbox").join(format!("{tag}-{}", std::process::id()))
        }
        _ => std::env::temp_dir().join(format!("{tag}-{}", std::process::id())),
    }
}

/// 檻の中で Python を回す Command を組む。dir = cage_work_dir の作業場。
/// ro_binds = 読み取り専用で見せたい場所(.venv や .so の隣 — bwrap だけが使う。
/// Flatpak では /app と runtime が最初から見えている)。None = 檻が組めない
pub(crate) fn caged_python(
    py: &std::path::Path,
    dir: &std::path::Path,
    ro_binds: &[PathBuf],
    allow_net: bool,
) -> Option<std::process::Command> {
    caged_python_with(cage_kind(), py, dir, ro_binds, allow_net)
}

/// 檻の種類を外から差せる形(試験が引数の組みを確かめる)
pub(crate) fn caged_python_with(
    kind: Cage,
    py: &std::path::Path,
    dir: &std::path::Path,
    ro_binds: &[PathBuf],
    allow_net: bool,
) -> Option<std::process::Command> {
    match kind {
        Cage::Bwrap => {
            // 檻: / は読み取り専用、ホームは空、書けるのは作業場だけ
            let mut c = std::process::Command::new("/usr/bin/bwrap");
            c.args(["--ro-bind", "/", "/", "--tmpfs", "/home", "--tmpfs", "/tmp"]);
            for p in ro_binds {
                if p.exists() {
                    c.arg("--ro-bind").arg(p).arg(p);
                }
            }
            c.arg("--bind").arg(dir).arg(dir);
            if !allow_net {
                c.arg("--unshare-net");
            }
            c.args([
                "--dev",
                "/dev",
                "--proc",
                "/proc",
                "--die-with-parent",
                "--new-session",
                "--setenv",
                "HOME",
                "/tmp",
                "--",
            ]);
            c.arg(py);
            Some(c)
        }
        Cage::Flatpak => {
            // 公式の入れ子: バスは切れ、ファイルは /app と runtime と
            // expose した作業場だけ。網は --no-network で閉じる
            let name = dir.file_name()?.to_string_lossy().to_string();
            let mut c = std::process::Command::new("flatpak-spawn");
            c.arg("--sandbox");
            c.arg(format!("--sandbox-expose={name}"));
            if !allow_net {
                c.arg("--no-network");
            }
            c.arg(py);
            Some(c)
        }
        Cage::None => None,
    }
}

/// 子プロセスを時間制限つきで回す → (成功か, stdout, stderr)。
/// 出力は別スレッドで吸い出す(パイプが詰まっても try_wait が止まらない)。
/// 時間切れは殺して Err — 檻の中の無限ループでアプリの手が塞がらない
pub(crate) fn run_with_timeout(
    cmd: &mut std::process::Command,
    secs: u64,
) -> Result<(bool, String, String), String> {
    use std::io::Read;
    use std::process::Stdio;
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("Python が起動できません: {e}"))?;
    let mut so = child.stdout.take();
    let mut se = child.stderr.take();
    let th_o = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(r) = so.as_mut() {
            let _ = r.read_to_string(&mut s);
        }
        s
    });
    let th_e = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(r) = se.as_mut() {
            let _ = r.read_to_string(&mut s);
        }
        s
    });
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => {
                let out = th_o.join().unwrap_or_default();
                let err = th_e.join().unwrap_or_default();
                return Ok((st.success(), out, err));
            }
            Ok(None) => {
                if start.elapsed().as_secs() >= secs {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = th_o.join();
                    let _ = th_e.join();
                    return Err(ui::tf!(
                        "{}秒たっても終わらないので止めました(無限ループ?)",
                        secs
                    )
                    .to_string());
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// グラフ描きの Python を探す。JO_PYTHON → リポジトリの .venv → python3。
/// matplotlib が居るかは実行して分かる(居なければ status で言う)。
pub(crate) fn find_python() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("JO_PYTHON") {
        return p.into();
    }
    // 今いるフォルダの .venv(リポジトリ直下で起動した形)
    let venv = std::path::Path::new(".venv/bin/python");
    if venv.exists() {
        return venv.into();
    }
    // 実行ファイルの場所から遡って .venv を探す(target/release/calc →
    // リポジトリ直下)。**どこから起動しても同じ python に当たる** —
    // CWD 頼みだと「polars がありません」になり、ピボットが置けない
    // (発注者の実機で踏んだ 2026-08-07)
    if let Ok(exe) = std::env::current_exe() {
        for dir in exe.ancestors().skip(1) {
            let p = dir.join(".venv/bin/python");
            if p.exists() {
                return p;
            }
        }
    }
    "python3".into()
}

/// グラフの台本(matplotlib)。データは JSON で渡す。
/// 日本語は機械のフォントを matplotlib に登録して出す(豆腐にしない)。
pub(crate) const CHART_PY: &str = r#"
import json, sys
import matplotlib
matplotlib.use("Agg")
import numpy as np
from matplotlib import font_manager, pyplot as plt

spec = json.load(open(sys.argv[1], encoding="utf-8"))
if spec.get("font"):
    try:
        font_manager.fontManager.addfont(spec["font"])
        plt.rcParams["font.family"] = font_manager.FontProperties(
            fname=spec["font"]).get_name()
    except Exception:
        pass
labels = spec["labels"]
x = np.arange(len(labels))
series = spec["series"] or [{"name": "", "values": [0] * len(labels)}]
n = len(series)
w = 0.8 / n
fig, ax = plt.subplots(figsize=(6.4, 4.0))
for i, s in enumerate(series):
    ax.bar(x + (i - (n - 1) / 2) * w, s["values"], w, label=s["name"])
ax.set_xticks(x)
ax.set_xticklabels(labels)
if n > 1:
    ax.legend()
fig.tight_layout()
fig.savefig(spec["out"], dpi=100)
"#;

/// CSV/TSV 読みの台本。文字コード(UTF-8 → CP932 → Latin-1)と区切りを判定し、
/// 区切りに使えない印(US=\x1F, RS=\x1E)で吐く — タブや改行入りの欄でも壊れない。
/// テキスト取り込みウィザードの下ごしらえ。
pub(crate) struct ImportPend {
    pub path: std::path::PathBuf,
    /// IMPORT_ENCS の添字
    pub enc: usize,
    /// IMPORT_DELIMS の添字
    pub delim: usize,
    /// 「その他」の区切り(1文字)
    pub custom: String,
    pub dest: Pos,
    /// いまの設定で読んだ全行(取り込むときはこれを流し込む)
    pub grid: Vec<Vec<String>>,
    /// Python が実際に使った(文字コード, 区切り)— 自動のときの報告
    pub used: (String, String),
}

/// 文字コードの選択肢(見せる名前, Python に渡す名前)。
pub(crate) const IMPORT_ENCS: &[(&str, &str)] = &[
    ("自動", "auto"),
    ("UTF-8", "utf-8-sig"),
    ("Shift_JIS(CP932)", "cp932"),
    ("Latin-1", "latin-1"),
];

/// 区切りの選択肢(見せる名前, 実体)。「その他」は板で1文字を聞く。
pub(crate) const IMPORT_DELIMS: &[(&str, &str)] = &[
    ("自動", "auto"),
    ("カンマ", ","),
    ("タブ", "\t"),
    ("セミコロン", ";"),
    ("コロン", ":"),
    ("スペース", " "),
    ("その他…", "その他"),
];

pub(crate) const CSV_PY: &str = r#"
import csv, sys

# argv: path [文字コード|auto] [区切り|auto]
path = sys.argv[1]
want_enc = sys.argv[2] if len(sys.argv) > 2 else "auto"
want_delim = sys.argv[3] if len(sys.argv) > 3 else "auto"
raw = open(path, "rb").read()
text = None
used_enc = ""
encs = ("utf-8-sig", "cp932", "latin-1") if want_enc == "auto" else (want_enc,)
for enc in encs:
    try:
        text = raw.decode(enc)
        used_enc = enc
        break
    except UnicodeDecodeError:
        continue
if text is None:
    sys.exit("その文字コードでは読めません" if want_enc != "auto" else "文字コードが判定できません")
if want_delim == "auto":
    try:
        dialect = csv.Sniffer().sniff(text[:4096], delimiters=",\t;")
    except csv.Error:
        dialect = csv.excel_tab if "\t" in text[:4096] else csv.excel
    used_delim = dialect.delimiter
    rows = list(csv.reader(text.splitlines(), dialect))
else:
    used_delim = want_delim
    rows = list(csv.reader(text.splitlines(), delimiter=want_delim))
# 1行目は下ごしらえの報告(使った文字コード・区切り)
meta = "\x01" + "\x1f".join((used_enc, used_delim))
out = meta + "\x1e" + "\x1e".join("\x1f".join(row) for row in rows)
sys.stdout.buffer.write(out.encode("utf-8"))
"#;

/// 方程式の台本(matplotlib の mathtext)。式を清書して透過 PNG に描く。
pub(crate) const EQ_PY: &str = r#"
import json, sys
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib import font_manager

spec = json.load(open(sys.argv[1], encoding="utf-8"))
if spec.get("font"):
    try:
        font_manager.fontManager.addfont(spec["font"])
        plt.rcParams["font.family"] = font_manager.FontProperties(
            fname=spec["font"]).get_name()
    except Exception:
        pass
fig = plt.figure()
t = fig.text(0.05, 0.5, "$%s$" % spec["tex"], fontsize=20)
fig.canvas.draw()  # 式が読めなければここで止まる(黙って白紙にしない)
bbox = t.get_window_extent()
fig.set_size_inches(bbox.width / fig.dpi + 0.15, bbox.height / fig.dpi + 0.15)
plt.savefig(spec["out"], dpi=200, transparent=True)
"#;

/// テキストアートの台本(matplotlib)。太字+塗り+縁取りの飾り文字を
/// 透過 PNG に描く(色は calc の緑)。
pub(crate) const TEXTART_PY: &str = r##"
import json, sys
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import matplotlib.patheffects as pe
from matplotlib import font_manager

spec = json.load(open(sys.argv[1], encoding="utf-8"))
if spec.get("font"):
    try:
        font_manager.fontManager.addfont(spec["font"])
        plt.rcParams["font.family"] = font_manager.FontProperties(
            fname=spec["font"]).get_name()
    except Exception:
        pass
fig = plt.figure()
t = fig.text(0.05, 0.5, spec["tex"], fontsize=44, fontweight="bold",
             color="#1B6E3C",
             path_effects=[pe.withStroke(linewidth=6, foreground="#D5E8DC")])
fig.canvas.draw()
bbox = t.get_window_extent()
fig.set_size_inches(bbox.width / fig.dpi + 0.2, bbox.height / fig.dpi + 0.2)
plt.savefig(spec["out"], dpi=200, transparent=True)
"##;

/// ソルバーの台本(scipy)。指図は JSON、答えは \x1f 区切りの変数の値。
pub(crate) const SOLVER_PY: &str = r#"
import json, sys
from scipy.optimize import linprog

spec = json.load(open(sys.argv[1], encoding="utf-8"))
n = len(spec["c"])
lo = 0 if spec["nonneg"] else None
r = linprog(
    c=spec["c"],
    A_ub=spec["aub"] or None,
    b_ub=spec["bub"] or None,
    A_eq=spec["aeq"] or None,
    b_eq=spec["beq"] or None,
    bounds=[(lo, None)] * n,
    method="highs",
)
if not r.success:
    sys.exit("解がありません: " + str(r.message))
sys.stdout.write("\x1f".join("%.12g" % v for v in r.x))
"#;

/// ピボットの台本(polars)。指図は JSON、答えは CSV 取り込みと同じ
/// 区切りの印(\x1e 行 / \x1f 欄)で返す。
pub(crate) const PIVOT_PY: &str = r#"
import json, sys
import polars as pl

spec = json.load(open(sys.argv[1], encoding="utf-8"))
headers = spec["headers"]
data = {h: [row[i] for row in spec["rows"]] for i, h in enumerate(headers)}
df = pl.DataFrame(data)
# 絞り込み(見出しの ▼)。隠す値を先に落としてから集計する
for _f, _vs in spec.get("hide", []):
    if _f in df.columns and _vs:
        df = df.filter(~pl.col(_f).is_in(_vs))

# グループ化(第2版)。日付は 月/四半期/年 の札に、数は幅Nの帯に置き換える
def _grouped(col, unit):
    if unit.startswith("幅:"):
        w = float(unit[2:])
        f = pl.col(col).cast(pl.Float64, strict=False)
        lo = (f / w).floor() * w
        def _n(x):
            return str(int(x)) if float(x).is_integer() else str(x)
        # 帯の札は数字の幅で右詰め — 文字順でも 0〜49 < 50〜99 < 100〜149 に並ぶ
        _mx = df[col].cast(pl.Float64, strict=False).max()
        _wid = len(_n((_mx // w) * w + w - (1 if w.is_integer() else 0))) if _mx is not None else 0
        return (pl.when(f.is_null()).then(pl.col(col))
                .otherwise(lo.map_elements(
                    lambda v: (_n(v).rjust(_wid) + "〜" +
                               _n(v + w - (1 if w.is_integer() else 0)).rjust(_wid)),
                    return_dtype=pl.String)))
    d = pl.col(col).str.strptime(pl.Date, "%Y-%m-%d", strict=False)
    d2 = pl.col(col).str.strptime(pl.Date, "%Y/%m/%d", strict=False)
    d = pl.coalesce(d, d2)
    if unit == "年":
        out = d.dt.strftime("%Y年")
    elif unit == "四半期":
        out = (d.dt.year().cast(pl.String) + "年Q" +
               ((d.dt.month() + 2) // 3).cast(pl.String))
    else:  # 月
        out = d.dt.strftime("%Y-%m")
    # 日付として読めない値はそのまま残す(黙って落とさない)
    return pl.when(d.is_null()).then(pl.col(col)).otherwise(out)

for _f, _u in spec.get("group", []):
    if _f in df.columns:
        df = df.with_columns(_grouped(_f, _u).alias(_f))
val, agg = spec["value"], spec["agg"]
if agg != "個数":
    # 数にならないものは null(集計から外れる)
    df = df.with_columns(pl.col(val).cast(pl.Float64, strict=False))
idx, cols = spec["index"], spec["columns"]
FN = {"合計": "sum", "平均": "mean", "個数": "len", "最大": "max", "最小": "min"}

def agg_expr():
    return {"合計": pl.sum(val), "平均": pl.mean(val), "個数": pl.len().alias(val),
            "最大": pl.max(val), "最小": pl.min(val)}[agg]

def table(frame, index):
    if cols:
        return frame.pivot(cols, index=index, values=val,
                           aggregate_function=FN[agg], sort_columns=True).sort(index)
    return frame.group_by(index).agg(agg_expr()).sort(index)

def stub(frame, label, index):
    # index の1列目に札を立て、残りを空にした複製。ピボットに通すことで
    # 列名の並びを main と揃えたまま「1行に集めた」答えが得られる
    ex = [pl.lit(label).alias(index[0])] + [pl.lit("").alias(i) for i in index[1:]]
    return frame.with_columns(ex)

def row_total(frame, index):
    # 行ごとの総計(列に広げたぶんを全部まとめた値)。集計の種類を守る
    return {tuple(r[:-1]): r[-1]
            for r in frame.group_by(index).agg(agg_expr().alias("_t")).rows()}

main = table(df, idx)
# 値のフィルター(第2版)。集計した後の行に掛ける — 列に広げていれば
# 行の総計、そうでなければ値の列そのもの
vf = spec.get("vfilter")
if vf:
    _op, _th = vf
    _OPS = {">": lambda x: x > _th, ">=": lambda x: x >= _th,
            "<": lambda x: x < _th, "<=": lambda x: x <= _th,
            "=": lambda x: x == _th}
    _keys = row_total(df, idx) if cols else None
    def _row_ok(r):
        x = _keys.get(tuple(r[:len(idx)])) if _keys is not None else r[-1]
        try:
            return x is not None and _OPS[_op](float(x))
        except (TypeError, ValueError, KeyError):
            return False
    main = pl.DataFrame([list(r) for r in main.rows() if _row_ok(r)],
                        schema=main.schema, orient="row") if main.height else main
    # フィルター後の物差しで小計・総計も出す — 見えない行を数えない
    df = df.filter(
        pl.concat_str([pl.col(i).cast(pl.String) for i in idx], separator="\x1f")
        .is_in([chr(31).join(str(v) for v in r[:len(idx)]) for r in main.rows()])
        if main.height else pl.lit(False))
tot_col = spec["totals"] and bool(cols)
tots = row_total(df, idx) if tot_col else {}

out = []  # (種別, 欄) 種別: d=データ s=小計 b=空行 t=総計

sub = None
if spec["subtotals"] and len(idx) >= 2:
    sub = {r[0]: list(r[1:]) for r in table(df, [idx[0]]).rows()}
    sub_tots = row_total(df, [idx[0]]) if tot_col else {}

# 1つ目の見出しで束ねながら吐く(小計・空行はその区切りごと)
groups = []
for r in main.rows():
    if groups and groups[-1][0] == r[0]:
        groups[-1][1].append(r)
    else:
        groups.append((r[0], [r]))

for g, rs in groups:
    prev = None
    for r in rs:
        cells = list(r)
        if spec["compact"] and prev is not None:
            # 繰り返しの見出しを空欄に(コンパクト形式)
            for i in range(len(idx)):
                if cells[i] == prev[i]:
                    cells[i] = ""
                else:
                    break
        if tot_col:
            cells.append(tots.get(tuple(r[:len(idx)])))
        out.append(("d", cells))
        prev = list(r)
    if sub is not None:
        cells = [f"{g} 小計"] + [""] * (len(idx) - 1) + sub[g]
        if tot_col:
            cells.append(sub_tots.get((g,)))
        out.append(("s", cells))
    if spec["blank_rows"] and len(idx) >= 2:
        out.append(("b", [""] * (len(main.columns) + (1 if tot_col else 0))))

if spec["totals"] and df.height:
    cells = list(table(stub(df, "総計", idx), idx).rows()[0])
    if tot_col:
        cells.append(df.select(agg_expr()).item())
    out.append(("t", cells))

# 計算の種類(比率・累計・差)。データ行の値の欄だけを置き換える。
# **累計と差は小計・総計を出さない**(積み上げの途中に総計が挟まると
# 読み違えるため)ので、呼ぶ側が totals/subtotals を落として渡す
_sa = spec.get("show_as", "")
if _sa:
    _n = len(idx)  # 見出しの欄数。ここから右が値
    _cells = [c for k, c in out if k == "d"]
    if _sa == "比率":
        # 総計(全データの集計)を 100% とする
        _g = df.select(agg_expr()).item() if df.height else None
        for c in _cells:
            for j in range(_n, len(c)):
                v = c[j]
                c[j] = (v / _g) if (isinstance(v, (int, float)) and _g) else None
    elif _sa in ("累計", "差"):
        _prev = [None] * (max((len(c) for c in _cells), default=0))
        for c in _cells:
            for j in range(_n, len(c)):
                v = c[j]
                p = _prev[j]
                if isinstance(v, (int, float)):
                    if _sa == "累計":
                        c[j] = v + (p if isinstance(p, (int, float)) else 0)
                        _prev[j] = c[j]
                    else:
                        c[j] = (v - p) if isinstance(p, (int, float)) else None
                        _prev[j] = v
                else:
                    c[j] = None

def s(v):
    if v is None:
        return ""
    if _sa == "比率" and isinstance(v, float):
        return "%.1f%%" % (v * 100.0)
    if isinstance(v, float):
        return "%g" % v
    return str(v)

head = list(main.columns) + (["総計"] if tot_col else [])
lines = []
if cols:
    # Excel と同じ1行目の札: 「合計 / 金額」と、列に広げた見出し(月)
    label = [f"{agg} / {val}"] + [""] * (len(idx) - 1) + [" / ".join(cols)]
    label += [""] * (len(head) - len(label))
    lines.append("l\x1f" + "\x1f".join(label))
else:
    # 列が無いときは値の列の見出しを「合計 / 金額」に(Excel と同じ)
    head[-2 if tot_col else -1] = f"{agg} / {val}"
lines.append("h\x1f" + "\x1f".join(head))
for kind, cells in out:
    lines.append(kind + "\x1f" + "\x1f".join(s(v) for v in cells))
sys.stdout.buffer.write("\x1e".join(lines).encode("utf-8"))
"#;

/// プラグイン(.py)の置き場。writer と同じ ~/.config/office/plugins
pub(crate) fn plugins_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/office/plugins")
}

impl Calc {
    /// 選んだ範囲を matplotlib で棒グラフにして、シートに浮かべる。
    /// 1列目が項目名、残りの列が系列(先頭行が文字なら系列名)。
    /// Python は別の糸で回す(主の糸を塞がない — ダイアログと同じ作法)。
    pub(crate) fn insert_chart(&mut self, a: Pos, b: Pos, cx: &mut Context<Self>) {
        let sh = self.sheet();
        // 先頭行が見出しか(項目列以外に文字があるか)
        let header = (a.col + 1..=b.col).any(|c| {
            matches!(
                sh.get(Pos::new(a.row, c)).map(|x| &x.value),
                Some(sheet::Value::Text(_))
            )
        });
        let r0 = if header { a.row + 1 } else { a.row };
        let labels: Vec<String> = (r0..=b.row)
            .map(|r| {
                let v = sh.get(Pos::new(r, a.col)).map(|x| x.value.display()).unwrap_or_default();
                if v.is_empty() { (r + 1).to_string() } else { v }
            })
            .collect();
        let mut series = Vec::new();
        for c in a.col + 1..=b.col {
            let name = if header {
                sh.get(Pos::new(a.row, c)).map(|x| x.value.display()).unwrap_or_default()
            } else {
                col_name(c)
            };
            let values: Vec<f64> = (r0..=b.row)
                .map(|r| sh.get(Pos::new(r, c)).map(|x| x.value.as_number()).unwrap_or(0.0))
                .collect();
            series.push((name, values));
        }
        if labels.is_empty() || series.is_empty() {
            self.status = ui::t!("グラフにする範囲を選んでください(1列目が項目名、2列目からが数)").into();
            return;
        }
        // JSON は手で組む(依存を増やさない。文字列は最小の逃がし)
        let esc = |t: &str| t.replace('\\', "\\\\").replace('"', "\\\"");
        let dir = std::env::temp_dir().join(format!("jo-chart-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let out = dir.join("chart.png");
        let font = kumihan::font::for_document(None)
            .ok()
            .map(|(fam, _)| fam.path.to_string_lossy().to_string())
            .unwrap_or_default();
        let mut json = String::from("{\"labels\":[");
        json.push_str(&labels.iter().map(|l| format!("\"{}\"", esc(l))).collect::<Vec<_>>().join(","));
        json.push_str("],\"series\":[");
        json.push_str(
            &series
                .iter()
                .map(|(n, vs)| {
                    format!(
                        "{{\"name\":\"{}\",\"values\":[{}]}}",
                        esc(n),
                        vs.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")
                    )
                })
                .collect::<Vec<_>>()
                .join(","),
        );
        json.push_str(&format!(
            "],\"font\":\"{}\",\"out\":\"{}\"}}",
            esc(&font),
            esc(&out.to_string_lossy())
        ));
        let at = Pos::new(a.row, b.col + 1);
        self.status = ui::t!("グラフを描いています…").into();
        let task = cx.background_executor().spawn(async move {
            let json_path = dir.join("chart.json");
            let py_path = dir.join("chart.py");
            std::fs::write(&json_path, json).map_err(|e| e.to_string())?;
            std::fs::write(&py_path, CHART_PY).map_err(|e| e.to_string())?;
            let o = std::process::Command::new(find_python())
                .arg(&py_path)
                .arg(&json_path)
                .output()
                .map_err(|e| format!("Python が起動できません: {e}"))?;
            if !o.status.success() {
                let err = String::from_utf8_lossy(&o.stderr);
                let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("原因不明");
                return Err(if err.contains("No module named") {
                    format!("matplotlib がありません({last})。conda か pip で入れてください")
                } else {
                    format!("グラフが描けません: {last}")
                });
            }
            std::fs::read(&out).map_err(|e| e.to_string())
        });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok(data) => {
                        let (w, h) = image_px(&data).unwrap_or((640, 400));
                        this.checkpoint();
                        this.sheet_mut().images_new.push(sheet::model::SheetImage {
                            at,
            dx_px: 0.0,
            dy_px: 0.0,
                            width_px: w as f32,
                            height_px: h as f32,
                            data,
                        });
                        this.dirty = true;
                        this.status = format!(
                            "グラフを {} に置きました(保存で xlsx に入ります)",
                            at.a1()
                        )
                        .into();
                    }
                    Err(e) => this.status = e.into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// ピボットテーブルの挿入(polars が裏方)。指図(PivotDef)を組んで
    /// 回すだけ — 置き直せるように指図はブックに控える(xl/joPivot.xml)。
    pub(crate) fn insert_pivot(
        &mut self,
        pend: PivotPend,
        value: String,
        agg: &'static str,
        cx: &mut Context<Self>,
    ) {
        // 組み替え(フィールドリスト)なら、総計などの性質と場所は据え置く
        let keep = pend.replace.and_then(|i| self.book.pivots.get(i).cloned());
        let def = sheet::model::PivotDef {
            sheet: self.book.sheets[self.active].name.clone(),
            src: (pend.a, pend.b),
            rows_sel: pend.rows_sel,
            cols_sel: pend.cols_sel,
            value,
            agg: agg.to_string(),
            totals: keep.as_ref().map(|d| d.totals).unwrap_or(true), // 既定で総計(本家と同じ)
            subtotals: keep.as_ref().map(|d| d.subtotals).unwrap_or(false),
            blank_rows: keep.as_ref().map(|d| d.blank_rows).unwrap_or(false),
            compact: keep.as_ref().map(|d| d.compact).unwrap_or(false),
            dest: keep.as_ref().map(|d| d.dest).unwrap_or(pend.a), // 仮 — 置くときに決める
            size: keep.as_ref().map(|d| d.size).unwrap_or((0, 0)),
            hide: keep.as_ref().map(|d| d.hide.clone()).unwrap_or_default(),
            style: keep.as_ref().map(|d| d.style.clone()).unwrap_or_default(),
            vfilter: keep.as_ref().and_then(|d| d.vfilter.clone()),
            group_by: keep.as_ref().map(|d| d.group_by.clone()).unwrap_or_default(),
            show_as: keep.as_ref().map(|d| d.show_as.clone()).unwrap_or_default(),
            name: keep.as_ref().map(|d| d.name.clone()).unwrap_or_else(|| {
                // 新しい名前(ピボットテーブル1, 2, …)。空きの番号を探す
                let mut n = 1;
                loop {
                    let name = format!("ピボットテーブル{n}");
                    if !self.book.pivots.iter().any(|d| d.name == name) {
                        break name;
                    }
                    n += 1;
                }
            }),
        };
        self.spawn_pivot(def, pend.replace, cx);
    }

    /// いまのシートで、この位置に置いてあるピボットの指図の番号。
    pub(crate) fn pivot_at(&self, p: Pos) -> Option<usize> {
        let name = &self.book.sheets[self.active].name;
        self.book.pivots.iter().position(|d| {
            d.sheet == *name
                && d.size.0 > 0
                && p.row >= d.dest.row
                && p.row < d.dest.row + d.size.0
                && p.col >= d.dest.col
                && p.col < d.dest.col + d.size.1
        })
    }

    /// 集計の面をセルに書く。種別で見た目を付ける(h=見出しの帯、
    /// s=小計 t=総計は太字、t は上罫線も)。
    /// tot_col = 右端が総計の列(装いを効かせる)。本家のピボットの見た目
    /// (濃い見出し帯・太字の総計)に寄せる — 出力そのものがピボットだと分かる
    pub(crate) fn place_pivot_grid(
        &mut self,
        si: usize,
        at: Pos,
        grid: &[Vec<String>],
        kinds: &[char],
        tot_col: bool,
        style: &str,
    ) {
        // 見た目の組(スタイルギャラリー)。(見出しの地, 見出しの字, 小計の地)
        let (head_bg, head_fg, sub_bg) = match style {
            "緑" => ("548235", "FFFFFF", "E2EFDA"),
            "橙" => ("C55A11", "FFFFFF", "FBE5D6"),
            "灰" => ("595959", "FFFFFF", "EDEDED"),
            _ => ("4472C4", "FFFFFF", "D9E1F2"), // 既定 = 青
        };
        paste_values_text(&mut self.book.sheets[si], at, grid);
        let w = grid.iter().map(|r| r.len()).max().unwrap_or(1) as u32;
        for (i, k) in kinds.iter().enumerate() {
            let last = kinds.len() - 1;
            for c in 0..w {
                let p = Pos::new(at.row + i as u32, at.col + c);
                let mut cell = self.book.sheets[si].get(p).cloned().unwrap_or_default();
                match k {
                    'l' | 'h' => {
                        // 見出しの帯(スタイルの色)
                        cell.fmt.bold = true;
                        cell.fmt.fill = Some(head_bg.into());
                        cell.fmt.color = Some(head_fg.into());
                    }
                    's' => {
                        cell.fmt.bold = true;
                        cell.fmt.fill = Some(sub_bg.into());
                    }
                    't' => {
                        cell.fmt.bold = true;
                        cell.fmt.borders.top = sheet::model::Edge::THIN;
                    }
                    _ => {}
                }
                // 総計の列(右端)も太字+仕切り線
                if tot_col && c == w - 1 && *k != 'h' {
                    cell.fmt.bold = true;
                    cell.fmt.borders.left = sheet::model::Edge::THIN;
                }
                // 塊の外周に薄い線(印刷でも塊が分かる)
                if i == 0 { cell.fmt.borders.top = sheet::model::Edge::THIN; }
                if i == last { cell.fmt.borders.bottom = sheet::model::Edge::THIN; }
                if c == 0 { cell.fmt.borders.left = sheet::model::Edge::THIN; }
                if c == w - 1 { cell.fmt.borders.right = sheet::model::Edge::THIN; }
                self.book.sheets[si].set(p, cell);
            }
        }
    }

    /// 指図どおりに polars を回して置く。replace=None は挿入(右の空きを探す)、
    /// Some(i) は i 番の指図の更新(同じ場所に置き直す)。
    pub(crate) fn spawn_pivot(
        &mut self,
        mut def: sheet::model::PivotDef,
        replace: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        let Some(si) = self.book.sheets.iter().position(|s| s.name == def.sheet) else {
            self.status = format!("シート「{}」がありません(ピボットの元の表)", def.sheet).into();
            return;
        };
        let (a, b) = def.src;
        let sh = &self.book.sheets[si];
        let headers: Vec<String> = (a.col..=b.col)
            .map(|c| {
                let v = sh.get(Pos::new(a.row, c)).map(|x| x.value.display()).unwrap_or_default();
                if v.is_empty() { col_name(c) } else { v }
            })
            .collect();
        let data: Vec<Vec<String>> = (a.row + 1..=b.row)
            .map(|r| {
                (a.col..=b.col)
                    .map(|c| sh.get(Pos::new(r, c)).map(|x| x.value.display()).unwrap_or_default())
                    .collect()
            })
            .collect();
        let json = pivot_spec_json(&headers, &data, &def);
        let dir = std::env::temp_dir().join(format!("jo-pivot-{}", std::process::id()));
        self.status = format!("{} の {} を集めています…", def.value, def.agg).into();
        let task = cx.background_executor().spawn(async move {
            let _ = std::fs::create_dir_all(&dir);
            let json_path = dir.join("pivot.json");
            let py_path = dir.join("pivot.py");
            std::fs::write(&json_path, json).map_err(|e| e.to_string())?;
            std::fs::write(&py_path, PIVOT_PY).map_err(|e| e.to_string())?;
            let o = std::process::Command::new(find_python())
                .arg(&py_path)
                .arg(&json_path)
                .output()
                .map_err(|e| format!("Python が起動できません: {e}"))?;
            if !o.status.success() {
                let err = String::from_utf8_lossy(&o.stderr);
                let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("原因不明");
                return Err(if err.contains("No module named") {
                    format!("polars がありません({last})。pip で入れてください")
                } else {
                    format!("集計できません: {last}")
                });
            }
            Ok(String::from_utf8_lossy(&o.stdout).to_string())
        });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok(raw) => {
                        let (grid, kinds) = parse_pivot_grid(&raw);
                        let h = grid.len() as u32;
                        let w = grid.iter().map(|r| r.len()).max().unwrap_or(1) as u32;
                        let used = |this: &Self, p: Pos| {
                            this.book.sheets[si]
                                .get(p)
                                .map(|cell| {
                                    !cell.value.display().is_empty() || cell.formula.is_some()
                                })
                                .unwrap_or(false)
                        };
                        match replace {
                            None => {
                                // 右の空きを探す(埋まっていたらさらに右へ。黙って上書きしない)
                                let mut dc = b.col + 2;
                                let mut tries = 0;
                                let free = loop {
                                    let occupied = (0..h).any(|r| {
                                        (0..w).any(|c| used(this, Pos::new(a.row + r, dc + c)))
                                    });
                                    if !occupied {
                                        break true;
                                    }
                                    dc += w + 1;
                                    tries += 1;
                                    if tries > 50 {
                                        break false;
                                    }
                                };
                                if !free {
                                    this.status =
                                        ui::t!("右に空きが見つかりません(場所を空けてから)").into();
                                } else {
                                    this.checkpoint_book();
                                    def.dest = Pos::new(a.row, dc);
                                    def.size = (h, w);
                                    let at = def.dest;
                                    let tot_col = def.totals && !def.cols_sel.is_empty();
                                    let style = def.style.clone();
                                    this.place_pivot_grid(si, at, &grid, &kinds, tot_col, &style);
                                    recalc_book(&mut this.book, si);
                                    let (value, agg) = (def.value.clone(), def.agg.clone());
                                    this.book.pivots.push(def);
                                    this.dirty = true;
                                    // カーソルを置いた集計へ移し、ピボットテーブルの
                                    // タブを開く(本家の showPivotTab と同じ)。
                                    // 文脈タブに気づかないままにしない
                                    this.anchor = None;
                                    this.cursor = at;
                                    if let Some(ti) = ribbon::calc_tabs()
                                        .iter()
                                        .position(|t| t.cmds.iter().any(|c| c.id == "pivot-layout"))
                                    {
                                        if this.tab != ti {
                                            this.prev_tab = this.tab;
                                            this.tab = ti;
                                        }
                                    }
                                    this.sync_input();
                                    let pname = this
                                        .book
                                        .pivots
                                        .last()
                                        .map(|d| d.name.clone())
                                        .unwrap_or_default();
                                    this.status = format!(
                                        "{pname}({value} の {agg})を {} に置きました — その時の値。ピボットテーブルのタブが開いています(更新・総計・小計・レイアウトはここ。Ctrl+Z で戻せます)",
                                        at.a1()
                                    )
                                    .into();
                                }
                            }
                            Some(pi) => {
                                let Some(old) = this.book.pivots.get(pi).cloned() else {
                                    return;
                                };
                                let dest = old.dest;
                                let in_old = |p: Pos| {
                                    p.row >= dest.row
                                        && p.row < dest.row + old.size.0
                                        && p.col >= dest.col
                                        && p.col < dest.col + old.size.1
                                };
                                let occupied = (0..h).any(|r| {
                                    (0..w).any(|c| {
                                        let p = Pos::new(dest.row + r, dest.col + c);
                                        !in_old(p) && used(this, p)
                                    })
                                });
                                if occupied {
                                    this.status =
                                        ui::t!("広がった分の場所が塞がっています(右下を空けてから更新)").into();
                                } else {
                                    this.checkpoint_book();
                                    for r in 0..old.size.0 {
                                        for c in 0..old.size.1 {
                                            this.book.sheets[si]
                                                .cells
                                                .remove(&Pos::new(dest.row + r, dest.col + c));
                                        }
                                    }
                                    def.dest = dest;
                                    def.size = (h, w);
                                    // 装いは**新しい指図**(def)に合わせる — old だと
                                    // 総計を入切した直後の更新で右端の太字がずれる
                                    let tot_col = def.totals && !def.cols_sel.is_empty();
                                    let style = def.style.clone();
                                    this.place_pivot_grid(si, dest, &grid, &kinds, tot_col, &style);
                                    recalc_book(&mut this.book, si);
                                    this.book.pivots[pi] = def;
                                    this.dirty = true;
                                    this.sync_input();
                                    this.status = format!(
                                        "ピボットを更新しました({} — その時の値。Ctrl+Z で戻せます)",
                                        dest.a1()
                                    )
                                    .into();
                                }
                            }
                        }
                    }
                    Err(e) => this.status = e.into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Python in Calc(発注者提案 2026-08-04)。**コードは文書に入れない** —
    /// マクロと違い「開く=実行」の経路が無い。いまの表を一時 xlsx に写し、
    /// office_sheet(pysheet)で b(ブック)と s(いまのシート)を束縛して
    /// 利用者のコードを回し、保存されたものを読み戻して**1手として**適用する。
    pub(crate) fn run_python(&mut self, user_code: String, cx: &mut Context<Self>) {
        // 自分で打った/選んだコード: 檻はかけるが網は許す(自分の道具が
        // Web から取り込むのは普通の仕事。守るのは機械のファイルの方)
        self.run_python_inner(user_code, false, true, cx);
    }

    /// sandbox=true は**必ず**bubblewrap の檻の中で回す(ブックに載っていた
    /// コード = 他人のファイル由来かもしれないもの)。檻: ネット遮断・
    /// 実ファイルは読み取り専用・ホームは不可視・書けるのは交換用の一時領域だけ。
    /// 檻が無い機械では載せたコードは**実行しない**(そう言う)。
    /// 自分で打った/選んだコードも、檻があれば檻で回す(深層防御)。
    pub(crate) fn run_python_inner(
        &mut self,
        user_code: String,
        sandbox: bool,
        allow_net: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.commit() {
            return;
        }
        let dir = cage_work_dir("jo-py");
        let _ = std::fs::create_dir_all(&dir);
        let in_x = dir.join("in.xlsx");
        let out_x = dir.join("out.xlsx");
        // 実行は複製の上(失敗しても表は無傷)。原本の部品も持ち越して写す
        let original: Option<std::io::Cursor<Vec<u8>>> = self
            .path
            .as_ref()
            .and_then(|old| std::fs::read(old).ok())
            .map(std::io::Cursor::new);
        let w = std::fs::File::create(&in_x)
            .map_err(|e| e.to_string())
            .and_then(|f| {
                sheet::xlsx::write_with(&self.book, original, std::io::BufWriter::new(f))
            });
        if let Err(e) = w {
            self.status = format!("Python に渡せません: {e}").into();
            return;
        }
        // office_sheet.so は実行ファイルの隣(HIKITSUGI の配り方を参照)
        let so_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_default();
        let so_dir2 = so_dir.clone();
        let script = format!(
            concat!(
                "import sys\n",
                "sys.path.insert(0, {so_dir:?})\n",
                "import office_sheet\n",
                "b = office_sheet.Book.open({in_x:?})\n",
                "s = b[{active}]\n",
                "# ---- 利用者のコード ----\n",
                "{code}\n",
                "# ----\n",
                "b.save({out_x:?})\n"
            ),
            so_dir = so_dir.to_string_lossy(),
            in_x = in_x.to_string_lossy(),
            active = self.active,
            out_x = out_x.to_string_lossy(),
            code = user_code
        );
        self.status = ui::t!("Python を実行しています…").into();
        let task = cx.background_executor().spawn(async move {
            let py_path = dir.join("run.py");
            std::fs::write(&py_path, script).map_err(|e| e.to_string())?;
            let py = find_python();
            // 檻はあれば必ず使う(深層防御)。他所から来たかもしれないコード
            // (sandbox=true)は、檻が組めなければ実行しない
            let venv = std::fs::canonicalize(".venv").unwrap_or_default();
            let mut cmd = match caged_python(&py, &dir, &[venv, so_dir2], allow_net) {
                Some(c) => c,
                None if sandbox => {
                    return Err(ui::t!(
                        "檻が組めません(bubblewrap か Flatpak が要ります)。\
他所から来たかもしれないコードは檻の外では実行しません(apt install bubblewrap)"
                    )
                    .to_string());
                }
                None => std::process::Command::new(&py),
            };
            // 時間制限つき(60秒)— 檻の中の無限ループで手が塞がらない
            let (ok, out, err) = run_with_timeout(cmd.arg(&py_path), 60)?;
            let out = out.trim().to_string();
            if !ok {
                let last = err
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("原因不明")
                    .to_string();
                return Err(if err.contains("No module named 'office_sheet'") {
                    ui::t!("office_sheet.so がありません(cargo build -p pysheet --release \
--features extension-module して、liboffice_sheet.so を office_sheet.so の名で \
calc の隣に置いてください)").to_string()
                } else {
                    last
                });
            }
            std::fs::read(&out_x)
                .map_err(|e| format!("結果が読めません: {e}"))
                .map(|bytes| (bytes, out))
        });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok((bytes, out)) => {
                        match sheet::xlsx::read(std::io::Cursor::new(bytes)) {
                            Ok((mut book, rep)) => {
                                sheet::recalc_all(&mut book);
                                this.checkpoint_book();
                                this.book = book;
                                if this.active >= this.book.sheets.len() {
                                    this.active = 0;
                                }
                                this.sheet_ui.clear();
                                this.dirty = true;
                                this.sync_input();
                                this.notes = rep
                                    .unsupported
                                    .iter()
                                    .map(|(n, c)| SharedString::from(format!("{n} × {c}")))
                                    .collect();
                                this.status = if out.is_empty() {
                                    ui::t!("Python を実行しました(Ctrl+Z で1手で戻せます)").into()
                                } else {
                                    let last =
                                        out.lines().last().unwrap_or_default().to_string();
                                    format!(
                                        "Python: {last}(出力{}行。変更は Ctrl+Z で戻せます)",
                                        out.lines().count()
                                    )
                                    .into()
                                };
                            }
                            Err(e) => {
                                this.status = format!("結果が読めません: {e}").into();
                            }
                        }
                    }
                    Err(e) => this.status = format!("Python: {e}").into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// =PY(…) のセルを全部、檻の中で計算する(@計算)。
    /// 関数の定義はブックの「関数」で始まる名前のスクリプトから読む。
    /// **これ以外の道で PY セルが計算されることはない**(開く=実行を持たない)。
    pub(crate) fn run_py_calc(&mut self, cx: &mut Context<Self>) {
        if !self.commit() {
            return;
        }
        let defs: String = self
            .book
            .scripts
            .iter()
            .filter(|(n, _)| n.starts_with("関数"))
            .map(|(_, c)| c.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let mut per_sheet: Vec<(usize, Vec<(String, String, Vec<sheet::calc::PyArg>)>)> =
            Vec::new();
        for (i, sh) in self.book.sheets.iter().enumerate() {
            let mut calls = Vec::new();
            for (p, c) in &sh.cells {
                let Some(f) = &c.formula else { continue };
                if !sheet::calc::is_py_formula(f) {
                    continue;
                }
                if let Some((name, args)) = sheet::calc::eval_py_call(sh, f) {
                    calls.push((p.a1(), name, args));
                }
            }
            if !calls.is_empty() {
                per_sheet.push((i, calls));
            }
        }
        if per_sheet.is_empty() {
            self.status = ui::t!("=PY(\"関数名\", 引数…) のセルがありません").into();
            return;
        }
        if defs.trim().is_empty() {
            self.status =
                ui::t!("関数の定義がありません(@save 関数 で def の入った .py をブックに載せる)").into();
            return;
        }
        let dir = cage_work_dir("jo-udf");
        let _ = std::fs::create_dir_all(&dir);
        let mut scripts = Vec::new();
        for (i, calls) in &per_sheet {
            let out = dir.join(format!("out{i}.txt"));
            scripts.push((
                *i,
                dir.join(format!("udf{i}.py")),
                out.clone(),
                build_udf_script(&defs, calls, &out),
            ));
        }
        self.status = ui::t!("PY を計算しています…(檻の中)").into();
        let task = cx.background_executor().spawn(async move {
            if cage_kind() == Cage::None {
                return Err(
                    ui::t!("檻が組めません(bubblewrap か Flatpak が要ります)。ブックの関数は檻の外では計算しません").to_string(),
                );
            }
            let py = find_python();
            let venv = std::fs::canonicalize(".venv").unwrap_or_default();
            let mut results = Vec::new();
            for (i, py_path, out_path, script) in scripts {
                std::fs::write(&py_path, script).map_err(|e| e.to_string())?;
                // 関数(UDF)は常に網なしの檻
                let Some(mut c) = caged_python(&py, &dir, std::slice::from_ref(&venv), false)
                else {
                    return Err(ui::t!("檻が組めません(bubblewrap か Flatpak が要ります)。ブックの関数は檻の外では計算しません").to_string());
                };
                // 時間制限つき(30秒)。関数は値の計算だけ — それより長いのは異常
                let (ok, _, err) = run_with_timeout(c.arg(&py_path), 30)?;
                if !ok {
                    let last = err
                        .lines()
                        .rev()
                        .find(|l| !l.trim().is_empty())
                        .unwrap_or("原因不明");
                    return Err(format!("PY の計算に失敗: {last}"));
                }
                let raw = std::fs::read_to_string(&out_path).unwrap_or_default();
                results.push((i, raw));
            }
            Ok(results)
        });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok(outs) => {
                        this.checkpoint_book();
                        let (mut total, mut conflicts) = (0usize, 0usize);
                        for (i, raw) in outs {
                            let results = parse_udf_output(&raw);
                            let prev: std::collections::HashMap<Pos, (u32, u32)> = this
                                .py_spills
                                .iter()
                                .filter(|((si, _), _)| *si == i)
                                .map(|((_, p), d)| (*p, *d))
                                .collect();
                            let (spills, n, c) =
                                apply_py_results(&mut this.book.sheets[i], &results, &prev);
                            this.py_spills.retain(|(si, _), _| *si != i);
                            for (p, d) in spills {
                                this.py_spills.insert((i, p), d);
                            }
                            recalc_book(&mut this.book, i);
                            total += n;
                            conflicts += c;
                        }
                        this.dirty = true;
                        this.sync_input();
                        this.status = if conflicts > 0 {
                            format!(
                                "PY: {total} セルを計算、{conflicts} 件は #SPILL!(展開先に他のデータ)。Ctrl+Z で戻せます"
                            )
                            .into()
                        } else {
                            format!("PY: {total} セルを計算しました(Ctrl+Z で戻せます)").into()
                        };
                    }
                    Err(e) => this.status = e.into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// .py を選んで**ブックに載せる**(実行はしない)。載せたコードは
    /// 保存で xlsx に入り、帳票と一緒に旅をする。実行は @名前 で、必ず檻の中。
    pub(crate) fn store_python_dialog(&mut self, name: String, cx: &mut Context<Self>) {
        let ask = cx.background_executor().spawn(async {
            rfd::FileDialog::new()
                .add_filter("Python", &["py"])
                .pick_file()
        });
        cx.spawn(async move |this, cx| {
            let r = ask.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(p) = r {
                    match std::fs::read_to_string(&p) {
                        Ok(code) => {
                            this.book.scripts.retain(|(n, _)| *n != name);
                            this.book.scripts.push((name.clone(), code));
                            this.dirty = true;
                            this.status = format!(
                                "「{name}」をブックに載せました(保存で xlsx に入る。=PY(\"名前\", …) と @計算 が使います)"
                            )
                            .into();
                        }
                        Err(e) => this.status = format!("読めません: {e}").into(),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// ブックに載っている(古い形の)手続きを .py に取り出す。**実行はしない** —
    /// 中身を確かめてから plugins へ置くのは人の手(それが取り込みの門)
    pub(crate) fn export_python_dialog(&mut self, name: String, cx: &mut Context<Self>) {
        let Some(code) = self
            .book
            .scripts
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, c)| c.clone())
        else {
            self.status = ui::tf!("「{}」はありません(@list で一覧)", name).into();
            return;
        };
        let fname = format!("{name}.py");
        let ask = cx.background_executor().spawn(async move {
            rfd::FileDialog::new()
                .add_filter("Python", &["py"])
                .set_file_name(&fname)
                .save_file()
        });
        cx.spawn(async move |this, cx| {
            let r = ask.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(p) = r {
                    this.status = match std::fs::write(&p, &code) {
                        Ok(_) => ui::tf!(
                            "取り出しました: {}(中身を確かめて、良ければ plugins へ)",
                            p.display().to_string()
                        )
                        .into(),
                        Err(e) => ui::tf!("書けません: {}", e.to_string()).into(),
                    };
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// .py ファイルを選んで回す(コードは利用者のファイルにある —
    /// 文書には決して入らない)。
    pub(crate) fn run_python_file_dialog(&mut self, cx: &mut Context<Self>) {
        let ask = cx.background_executor().spawn(async {
            rfd::FileDialog::new()
                .add_filter("Python", &["py"])
                .pick_file()
        });
        cx.spawn(async move |this, cx| {
            let r = ask.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(p) = r {
                    match std::fs::read_to_string(&p) {
                        Ok(code) => this.run_python(code, cx),
                        Err(e) => this.status = format!("読めません: {e}").into(),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// CSV/TSV を選んで、ウィザードの板(文字コード・区切り・置き場所・
    /// プレビュー)を開く。読み直しは Python(CP932 も読める)。
    pub(crate) fn import_text_dialog(&mut self, cx: &mut Context<Self>) {
        let ask = cx
            .background_executor()
            .spawn(async {
                rfd::FileDialog::new()
                    .add_filter("テキストのデータ", &["csv", "tsv", "txt"])
                    .pick_file()
            });
        cx.spawn(async move |this, cx| {
            let Some(p) = ask.await else { return };
            let _ = this.update(cx, |this, cx| {
                this.import_pend = Some(ImportPend {
                    path: p,
                    enc: 0,
                    delim: 0,
                    custom: String::new(),
                    dest: this.cursor,
                    grid: Vec::new(),
                    used: (String::new(), String::new()),
                });
                this.import_reparse(cx);
            });
        })
        .detach();
    }

    /// いまの設定(文字コード・区切り)でファイルを読み直し、板を出し直す。
    pub(crate) fn import_reparse(&mut self, cx: &mut Context<Self>) {
        let Some(pend) = &self.import_pend else { return };
        let (path, enc, delim) = (
            pend.path.clone(),
            IMPORT_ENCS[pend.enc].1.to_string(),
            match IMPORT_DELIMS[pend.delim].1 {
                "その他" => {
                    if pend.custom.is_empty() { ",".into() } else { pend.custom.clone() }
                }
                d => d.to_string(),
            },
        );
        let job = cx.background_executor().spawn(async move {
            let dir = std::env::temp_dir().join(format!("jo-csv-{}", std::process::id()));
            let _ = std::fs::create_dir_all(&dir);
            // csv.py という名前は標準ライブラリの csv を隠してしまう(踏んだ)
            let py_path = dir.join("jo_csv.py");
            if std::fs::write(&py_path, CSV_PY).is_err() {
                return Err(ui::t!("一時ファイルが書けません").to_string());
            }
            let o = std::process::Command::new(find_python())
                .arg(&py_path)
                .arg(&path)
                .arg(&enc)
                .arg(&delim)
                .output();
            match o {
                Ok(o) if o.status.success() => {
                    Ok(String::from_utf8_lossy(&o.stdout).to_string())
                }
                Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
                Err(e) => Err(format!("Python が起動できません: {e}")),
            }
        });
        cx.spawn(async move |this, cx| {
            let r = job.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok(data) => {
                        let mut rows = data.split('\u{1e}');
                        // 1行目は下ごしらえの報告(\x01 文字コード \x1f 区切り)
                        let meta = rows.next().unwrap_or_default();
                        let used = meta
                            .strip_prefix('\u{01}')
                            .and_then(|m| m.split_once('\u{1f}'))
                            .map(|(e, d)| (e.to_string(), d.to_string()))
                            .unwrap_or_default();
                        let grid: Vec<Vec<String>> = rows
                            .map(|row| row.split('\u{1f}').map(|f| f.to_string()).collect())
                            .collect();
                        if let Some(pend) = &mut this.import_pend {
                            pend.grid = grid;
                            pend.used = used;
                        }
                        this.import_pick();
                    }
                    Err(e) => {
                        this.status = ui::tf!("読み込めません: {}", e).into();
                        this.import_pick(); // 板は残す(設定を替えて再挑戦できる)
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 取り込みウィザードの板を(いまの下ごしらえから)組む。
    pub(crate) fn import_pick(&mut self) {
        let Some(pend) = &self.import_pend else { return };
        let mut items: Vec<String> = Vec::new();
        let enc_label = if pend.enc == 0 && !pend.used.0.is_empty() {
            format!("{}({})", IMPORT_ENCS[0].0, pend.used.0)
        } else {
            IMPORT_ENCS[pend.enc].0.to_string()
        };
        let delim_name = |d: &str| match d {
            "," => "カンマ".to_string(),
            "\t" => "タブ".to_string(),
            ";" => "セミコロン".to_string(),
            ":" => "コロン".to_string(),
            " " => "スペース".to_string(),
            other => format!("「{other}」"),
        };
        let delim_label = if pend.delim == 0 && !pend.used.1.is_empty() {
            format!("{}({})", IMPORT_DELIMS[0].0, delim_name(&pend.used.1))
        } else if IMPORT_DELIMS[pend.delim].1 == "その他" && !pend.custom.is_empty() {
            format!("{}{}", IMPORT_DELIMS[pend.delim].0, delim_name(&pend.custom))
        } else {
            IMPORT_DELIMS[pend.delim].0.to_string()
        };
        items.push(format!("{}: {}", ui::t!("文字コード"), enc_label));
        items.push(format!("{}: {}", ui::t!("区切り"), delim_label));
        items.push(format!("{}: {}", ui::t!("置き場所"), pend.dest.a1()));
        // プレビュー(先頭3行。長ければ詰める)
        for (i, row) in pend.grid.iter().take(3).enumerate() {
            let mut line = row.join(" | ");
            if line.chars().count() > 42 {
                line = line.chars().take(42).collect::<String>() + "…";
            }
            items.push(format!("{} {}: {}", "·", i + 1, line));
        }
        items.push(format!(
            "→ {}",
            ui::tf!("取り込む({} 行)", pend.grid.len())
        ));
        let at = self
            .cell_origin_px(self.cursor)
            .map(|(x, y)| (x, y + self.row_px(self.cursor.row)))
            .unwrap_or((HEAD_W + 16.0, ROW_H + 16.0));
        let name = pend
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        self.pick_note =
            Some(ui::tf!("テキストの取り込み — {}(クリックで切替)", name).into());
        self.pick_kind = "csv-import-pick";
        self.pick = Some((items, at));
    }

    /// 板の文字を Python の台本で絵にして、画像としてシートに浮かべる。
    /// writer の方式(図は Python で描いて画像で貼る)の自動化 —
    /// 方程式(EQ_PY)とテキストアート(TEXTART_PY)が同じ道を通る。
    pub(crate) fn insert_py_image(
        &mut self,
        script: &'static str,
        name: &'static str,
        tex: String,
        cx: &mut Context<Self>,
    ) {
        let esc = |t: &str| t.replace('\\', "\\\\").replace('"', "\\\"");
        let dir =
            std::env::temp_dir().join(format!("jo-{name}-{}", std::process::id()));
        let out = dir.join("eq.png");
        let font = kumihan::font::for_document(None)
            .ok()
            .map(|(fam, _)| fam.path.to_string_lossy().to_string())
            .unwrap_or_default();
        let json = format!(
            "{{\"tex\":\"{}\",\"font\":\"{}\",\"out\":\"{}\"}}",
            esc(&tex),
            esc(&font),
            esc(&out.to_string_lossy())
        );
        let at = self.cursor;
        self.status = ui::t!("清書しています…").into();
        let task = cx.background_executor().spawn(async move {
            let _ = std::fs::create_dir_all(&dir);
            let json_path = dir.join("eq.json");
            let py_path = dir.join("eq.py");
            std::fs::write(&json_path, json).map_err(|e| e.to_string())?;
            std::fs::write(&py_path, script).map_err(|e| e.to_string())?;
            let o = std::process::Command::new(find_python())
                .arg(&py_path)
                .arg(&json_path)
                .output()
                .map_err(|e| format!("Python が起動できません: {e}"))?;
            if !o.status.success() {
                let err = String::from_utf8_lossy(&o.stderr);
                let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("原因不明");
                return Err(if err.contains("No module named") {
                    format!("matplotlib がありません({last})")
                } else {
                    format!("式が読めません: {last}")
                });
            }
            std::fs::read(&out).map_err(|e| e.to_string())
        });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok(data) => {
                        let (w, h) = image_px(&data).unwrap_or((200, 60));
                        this.checkpoint();
                        // 200dpi で描いたので画面では半分の大きさに置く
                        this.sheet_mut().images_new.push(sheet::model::SheetImage {
                            at,
            dx_px: 0.0,
            dy_px: 0.0,
                            width_px: w as f32 / 2.0,
                            height_px: h as f32 / 2.0,
                            data,
                        });
                        this.dirty = true;
                        this.status = format!(
                            "{} を {} に置きました(画像。保存で xlsx に入ります。Ctrl+Z で1手)",
                            if name == "eq" { "方程式" } else { "テキストアート" },
                            at.a1()
                        )
                        .into();
                    }
                    Err(e) => this.status = e.into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// SmartArt を図形の集まりとして組む。1手 = checkpoint 一回(全部まとめて
    /// Ctrl+Z で戻る)。各図形は普通の図形なので、選んで動かす・Enter で
    /// 文字を書く・Del で消す、が全部効く。xlsx へは prstGeom の図形として
    /// 入る(Excel でも図形として見える。本物の SmartArt 部品ではない)。
    pub(crate) fn insert_smartart(&mut self, name: &str, key: &str) {
        self.checkpoint();
        let at = self.cursor;
        let (g, l) = (Some("D5E8DC".to_string()), Some("1B6E3C".to_string()));
        // (dx, dy, w, h, kind, 塗り?, 文字?)
        let mut parts: Vec<(f32, f32, f32, f32, &str, bool, bool)> = Vec::new();
        match key {
            "block-list" => {
                for i in 0..3 {
                    parts.push((i as f32 * 140.0, 0.0, 128.0, 72.0, "roundRect", true, true));
                }
            }
            "vbox-list" => {
                for i in 0..3 {
                    parts.push((0.0, i as f32 * 60.0, 240.0, 48.0, "rect", true, true));
                }
            }
            "pyramid-list" => {
                for i in 0..3 {
                    let w = 160.0 + i as f32 * 60.0;
                    parts.push(((280.0 - w) / 2.0, i as f32 * 58.0, w, 48.0, "roundRect", true, true));
                }
            }
            "basic-process" => {
                for i in 0..3 {
                    parts.push((i as f32 * 164.0, 0.0, 120.0, 56.0, "rect", true, true));
                    if i < 2 {
                        parts.push((i as f32 * 164.0 + 124.0, 16.0, 36.0, 24.0, "rightArrow", true, false));
                    }
                }
            }
            "chevron-process" => {
                for i in 0..3 {
                    parts.push((i as f32 * 140.0, 0.0, 150.0, 56.0, "rightArrow", true, true));
                }
            }
            "timeline" => {
                parts.push((0.0, 46.0, 420.0, 3.0, "rect", true, false)); // 軸
                for i in 0..3 {
                    parts.push((30.0 + i as f32 * 150.0, 38.0, 18.0, 18.0, "ellipse", true, false));
                    parts.push((6.0 + i as f32 * 150.0, 0.0, 100.0, 32.0, "rect", false, true));
                }
            }
            "basic-cycle" => {
                parts.push((110.0, 0.0, 110.0, 64.0, "ellipse", true, true));
                parts.push((0.0, 110.0, 110.0, 64.0, "ellipse", true, true));
                parts.push((220.0, 110.0, 110.0, 64.0, "ellipse", true, true));
            }
            "block-cycle" => {
                parts.push((105.0, 0.0, 120.0, 48.0, "rect", true, true));
                parts.push((220.0, 78.0, 120.0, 48.0, "rect", true, true));
                parts.push((105.0, 156.0, 120.0, 48.0, "rect", true, true));
                parts.push((0.0, 78.0, 120.0, 48.0, "rect", true, true));
            }
            "org-chart" | "hierarchy" => {
                let kids = if key == "org-chart" { 3 } else { 2 };
                let (w, gap) = (120.0, 40.0);
                let total = kids as f32 * w + (kids - 1) as f32 * gap;
                parts.push(((total - w) / 2.0, 0.0, w, 48.0, "rect", true, true));
                // 継ぎの線(細い棒): 親の下 → 横橋 → 子の上
                parts.push((total / 2.0 - 1.0, 48.0, 2.0, 22.0, "rect", true, false));
                parts.push((w / 2.0, 70.0, total - w, 2.0, "rect", true, false));
                for i in 0..kids {
                    let x = i as f32 * (w + gap);
                    parts.push((x + w / 2.0 - 1.0, 72.0, 2.0, 22.0, "rect", true, false));
                    parts.push((x, 94.0, w, 48.0, "rect", true, true));
                }
            }
            "venn" => {
                parts.push((0.0, 0.0, 150.0, 150.0, "ellipse", false, true));
                parts.push((90.0, 0.0, 150.0, 150.0, "ellipse", false, true));
                parts.push((45.0, 78.0, 150.0, 150.0, "ellipse", false, true));
            }
            "matrix" => {
                for r in 0..2 {
                    for c in 0..2 {
                        parts.push((c as f32 * 132.0, r as f32 * 62.0, 124.0, 54.0, "rect", true, true));
                    }
                }
            }
            "pyramid" => {
                for i in 0..3 {
                    let w = 120.0 + i as f32 * 90.0;
                    parts.push(((300.0 - w) / 2.0, i as f32 * 54.0, w, 48.0, "rect", true, true));
                }
            }
            _ => {}
        }
        let n = parts.len();
        for (dx, dy, w, h, kind, filled, texted) in parts {
            self.sheet_mut().shapes_new.push(sheet::model::SheetShape {
                at,
                dx_px: dx,
                dy_px: dy,
                width_px: w,
                height_px: h,
                kind: kind.into(),
                fill: if filled { g.clone() } else { None },
                line: l.clone(),
                text: if texted { Some(ui::t!("テキスト").into()) } else { None },
                ..Default::default()
            });
        }
        self.dirty = true;
        self.status = format!(
            "{name} を {} に置きました({n} 個の図形。図形を選んで Enter で文字、ドラッグで移動。全部まとめて Ctrl+Z で1手)",
            at.a1()
        )
        .into();
    }

    /// ソルバーを解く。係数は**表の複製の上で測る**(ゴールシークと同じ流儀):
    /// 変数を全部 0 → 単位ベクトル、で目的と制約左辺の一次係数を取り、
    /// 全部 1 の点で検算して**線形でなければ正直に断る**(単体法 LP は
    /// 線形の問題だけ — 本家 ONLYOFFICE の断り書きと同じ)。
    /// 解くのは scipy.optimize.linprog(highs)。
    pub(crate) fn solve_solver(&mut self, cx: &mut Context<Self>) {
        let Some(sv) = &self.solver else { return };
        // ---- 読み取りと検め ----
        let Some(target) = Pos::parse(&sv.target.text().replace('$', "").to_uppercase()) else {
            self.status = ui::t!("目的のセルが読めません(例: E6)").into();
            return;
        };
        let Some(vars) = parse_cell_list(&sv.vars.text(), 64) else {
            self.status = ui::t!("変数セルが読めません(例: B2:B4 や B2,C2。最大64個)").into();
            return;
        };
        let mode = sv.mode;
        let want = if mode == 2 {
            match sv.value.text().trim().parse::<f64>() {
                Ok(v) => v,
                Err(_) => {
                    self.status = ui::t!("「値」が数として読めません").into();
                    return;
                }
            }
        } else {
            0.0
        };
        // 制約: (セル, op, 右辺の数)。左辺は範囲なら1セルずつの行になる
        let mut rows: Vec<(Pos, usize, f64)> = Vec::new();
        for (l, op, r) in &sv.cons {
            let Some(cells) = parse_cell_list(l, 256) else {
                self.status = format!("制約の左辺が読めません: {l}").into();
                return;
            };
            let opi = SOLVER_OPS.iter().position(|o| o == op).unwrap_or(0);
            // 右辺: 数か、セルの今の値
            let rhs = match r.trim().parse::<f64>() {
                Ok(v) => v,
                Err(_) => match Pos::parse(&r.replace('$', "").to_uppercase()) {
                    Some(p) => self
                        .sheet()
                        .get(p)
                        .map(|c| c.value.as_number())
                        .unwrap_or(0.0),
                    None => {
                        self.status = format!("制約の右辺が読めません: {r}").into();
                        return;
                    }
                },
            };
            for c in cells {
                rows.push((c, opi, rhs));
            }
        }
        // ---- 係数の抽出(表の複製で測る)----
        let base = self.sheet().clone();
        let eval = |xs: &[f64]| -> (f64, Vec<f64>) {
            let mut s = base.clone();
            for (i, p) in vars.iter().enumerate() {
                s.set(*p, Cell::input(&format!("{}", xs[i])));
            }
            recalc(&mut s);
            let g = |p: Pos| s.get(p).map(|c| c.value.as_number()).unwrap_or(0.0);
            (g(target), rows.iter().map(|(p, _, _)| g(*p)).collect())
        };
        let n = vars.len();
        let zeros = vec![0.0; n];
        let (f0, c0) = eval(&zeros);
        let mut obj = vec![0.0; n];
        let mut a: Vec<Vec<f64>> = vec![vec![0.0; n]; rows.len()];
        for i in 0..n {
            let mut xs = zeros.clone();
            xs[i] = 1.0;
            let (fi, ci) = eval(&xs);
            obj[i] = fi - f0;
            for (k, v) in ci.iter().enumerate() {
                a[k][i] = v - c0[k];
            }
        }
        // 線形の検算(全部 1 の点)
        let ones = vec![1.0; n];
        let (f1, c1) = eval(&ones);
        let lin = |measured: f64, base: f64, coefs: &[f64]| -> bool {
            let predicted = base + coefs.iter().sum::<f64>();
            (measured - predicted).abs() <= 1e-6 * measured.abs().max(1.0)
        };
        let mut linear = lin(f1, f0, &obj);
        for k in 0..rows.len() {
            linear = linear && lin(c1[k], c0[k], &a[k]);
        }
        if !linear {
            self.status =
                ui::t!("線形ではありません — 単体法 LP は線形の問題だけを解きます(非線形は未対応。本家と同じ)").into();
            return;
        }
        // ---- LP に組む ----
        // 目的: 最大=係数を負に、最小=そのまま、値=目的0で f=want を等式に
        let mut aub: Vec<Vec<f64>> = Vec::new();
        let mut bub: Vec<f64> = Vec::new();
        let mut aeq: Vec<Vec<f64>> = Vec::new();
        let mut beq: Vec<f64> = Vec::new();
        for (k, (_, opi, rhs)) in rows.iter().enumerate() {
            let row = a[k].clone();
            let b = rhs - c0[k];
            match opi {
                0 => {
                    aub.push(row);
                    bub.push(b);
                }
                1 => {
                    aeq.push(row);
                    beq.push(b);
                }
                _ => {
                    aub.push(row.iter().map(|v| -v).collect());
                    bub.push(-b);
                }
            }
        }
        let c: Vec<f64> = match mode {
            0 => obj.iter().map(|v| -v).collect(),
            1 => obj.clone(),
            _ => {
                aeq.push(obj.clone());
                beq.push(want - f0);
                vec![0.0; n]
            }
        };
        // ---- JSON → scipy ----
        let arr = |v: &[f64]| {
            v.iter().map(|x| format!("{x}")).collect::<Vec<_>>().join(",")
        };
        let mat = |m: &[Vec<f64>]| {
            m.iter().map(|r| format!("[{}]", arr(r))).collect::<Vec<_>>().join(",")
        };
        let json = format!(
            "{{\"c\":[{}],\"aub\":[{}],\"bub\":[{}],\"aeq\":[{}],\"beq\":[{}],\"nonneg\":{}}}",
            arr(&c),
            mat(&aub),
            arr(&bub),
            mat(&aeq),
            arr(&beq),
            sv.nonneg
        );
        let dir = std::env::temp_dir().join(format!("jo-solver-{}", std::process::id()));
        self.status = ui::t!("解を探しています…(単体法 LP)").into();
        let task = cx.background_executor().spawn(async move {
            let _ = std::fs::create_dir_all(&dir);
            let json_path = dir.join("solver.json");
            let py_path = dir.join("solver.py");
            std::fs::write(&json_path, json).map_err(|e| e.to_string())?;
            std::fs::write(&py_path, SOLVER_PY).map_err(|e| e.to_string())?;
            let o = std::process::Command::new(find_python())
                .arg(&py_path)
                .arg(&json_path)
                .output()
                .map_err(|e| format!("Python が起動できません: {e}"))?;
            if !o.status.success() {
                let err = String::from_utf8_lossy(&o.stderr);
                let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("原因不明");
                return Err(if err.contains("No module named") {
                    format!("scipy がありません({last})。pip で入れてください")
                } else {
                    last.to_string()
                });
            }
            Ok(String::from_utf8_lossy(&o.stdout).to_string())
        });
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok(out) => {
                        let xs: Vec<f64> = out
                            .split('\u{1f}')
                            .filter_map(|v| v.trim().parse().ok())
                            .collect();
                        if xs.len() != vars.len() {
                            this.status = format!("答えの形が違います: {out}").into();
                        } else {
                            this.checkpoint();
                            for (p, x) in vars.iter().zip(&xs) {
                                let x = (x * 1e9).round() / 1e9;
                                let fmt = this
                                    .sheet()
                                    .get(*p)
                                    .map(|c| c.fmt.clone())
                                    .unwrap_or_default();
                                let mut cell = Cell::input(&format!("{x}"));
                                cell.fmt = fmt;
                                this.book.sheets[this.active].set(*p, cell);
                            }
                            recalc_book(&mut this.book, this.active);
                            this.dirty = true;
                            this.sync_input();
                            this.solver = None;
                            let got = this
                                .sheet()
                                .get(target)
                                .map(|c| c.value.display())
                                .unwrap_or_default();
                            this.status = format!(
                                "解を求めました: {} = {got}(変数 {} 個を書き換え。Ctrl+Z で1手)",
                                target.a1(),
                                xs.len()
                            )
                            .into();
                        }
                    }
                    Err(e) => this.status = e.into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// ゴールシーク。変えるセルの値を割線法で探す(表の複製の上で試す)。
    pub(crate) fn goal_seek(&mut self, target: Pos, goal: f64, var: Pos) {
        let base = self.sheet().clone();
        if base.get(target).and_then(|c| c.formula.as_ref()).is_none() {
            self.status = format!("{} は式のセルではありません", target.a1()).into();
            return;
        }
        let found = solve_goal(&base, target, goal, var);
        match found {
            Some(x) => {
                let x = (x * 1e9).round() / 1e9;
                self.checkpoint();
                let fmt = self.sheet().get(var).map(|c| c.fmt.clone()).unwrap_or_default();
                let mut cell = Cell::input(&format!("{x}"));
                cell.fmt = fmt;
                self.sheet_mut().set(var, cell);
                recalc_book(&mut self.book, self.active);
                self.dirty = true;
                self.sync_input();
                self.status = format!(
                    "{} = {x} で {} が {goal} になります(Ctrl+Z で戻せます)",
                    var.a1(),
                    target.a1()
                )
                .into();
            }
            None => {
                self.status = format!(
                    "見つかりません({} が {} に効いていないかもしれません)",
                    var.a1(),
                    target.a1()
                )
                .into();
            }
        }
    }
}

#[cfg(test)]
mod cage_tests {
    use super::*;

    fn args_of(c: &std::process::Command) -> Vec<String> {
        std::iter::once(c.get_program().to_string_lossy().to_string())
            .chain(c.get_args().map(|a| a.to_string_lossy().to_string()))
            .collect()
    }

    #[test]
    fn bwrapの檻は網を既定で切る() {
        let d = PathBuf::from("/tmp/jo-py-1");
        let py = std::path::Path::new("python3");
        let c = caged_python_with(Cage::Bwrap, py, &d, &[], false).unwrap();
        let a = args_of(&c);
        assert_eq!(a[0], "/usr/bin/bwrap");
        assert!(a.contains(&"--unshare-net".into()), "網が切れていない: {a:?}");
        assert!(a.contains(&"--die-with-parent".into()));
        // net を許したときだけ網が通る
        let c2 = caged_python_with(Cage::Bwrap, py, &d, &[], true).unwrap();
        assert!(!args_of(&c2).contains(&"--unshare-net".into()));
    }

    #[test]
    fn flatpakの檻は公式の入れ子口を使う() {
        // Flatpak の中では bwrap の入れ子が動かない — flatpak-spawn --sandbox
        let d = PathBuf::from("/x/sandbox/jo-udf-9");
        let py = std::path::Path::new("python3");
        let c = caged_python_with(Cage::Flatpak, py, &d, &[], false).unwrap();
        let a = args_of(&c);
        assert_eq!(a[0], "flatpak-spawn");
        assert!(a.contains(&"--sandbox".into()), "{a:?}");
        assert!(
            a.contains(&"--sandbox-expose=jo-udf-9".into()),
            "作業場が expose されていない: {a:?}"
        );
        assert!(a.contains(&"--no-network".into()), "網が切れていない: {a:?}");
        assert!(!args_of(&caged_python_with(Cage::Flatpak, py, &d, &[], true).unwrap())
            .contains(&"--no-network".into()));
        // 檻が組めなければ None(「実行しない」と言うのは呼ぶ側)
        assert!(caged_python_with(Cage::None, py, &d, &[], false).is_none());
    }
}

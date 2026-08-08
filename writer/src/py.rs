//! writer の Python の裏方(main.rs から純移動 2026-08-08。部屋割りの4歩目)。
//! Python を探す・マクロの台本を組む・プラグインの置き場。**純移動**

use crate::*;

/// 自分の名乗り(誰が開いているか)。user@host。
/// Python の探し方(calc と同じ)。JO_PYTHON > .venv > python3
pub(crate) fn find_python() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("JO_PYTHON") {
        return p.into();
    }
    let venv = std::path::Path::new(".venv/bin/python");
    if venv.exists() {
        return venv.into();
    }
    "python3".into()
}

/// マクロ台本の全文を組む。前置きで d(python-docx の文書)のほかに、
/// 記入欄へ**名前で**書く fill / fill_one、読む extract、名前と値の一覧
/// fields を渡す(記入=出口と吸い上げ=入口の対)。鍵は docx の w:tag
/// (フォームタブの「名前」釦で付ける)。無い名前は例外で断る —
/// ラベルの字を探して隣に書く走査より、名前が様式の背骨(発注者 2026-08-05)。
/// もう一車線が雛形: render(辞書)= docxtpl の {{名前}} / {%tr %} 差し込み、
/// tpl_fields()= 差し込み口の一覧。往復する様式は記入欄、出して終わりの
/// 量産文書(通知書・契約書)は雛形、の使い分け(SEKKEI 参照)
pub(crate) fn macro_script(
    in_d: &std::path::Path,
    out_d: &std::path::Path,
    user_code: &str,
) -> String {
    // 記入の道具。lxml の Element は「子が無い=偽」なので is None で判定する
    const FILL: &str = r#"from docx.oxml.ns import qn
def _sdts(name):
    es = []
    for sdt in d.element.iter(qn('w:sdt')):
        pr = sdt.find(qn('w:sdtPr'))
        tag = pr.find(qn('w:tag')) if pr is not None else None
        if tag is None:
            continue
        t = tag.get(qn('w:val')) or ''
        # 「jo:email:連絡先」= writer 独自の種類の印+名前。名前でも引ける
        if t == name or (t.startswith('jo:') and t.split(':', 2)[-1] == name):
            es.append(sdt)
    return es
def _put(sdt, value):
    ct = sdt.find(qn('w:sdtContent'))
    if ct is None:
        raise SystemExit('記入欄の中身がありません')
    ts = list(ct.iter(qn('w:t')))
    v = str(value)
    if ts:
        ts[0].text = v
        for t in ts[1:]:
            t.text = ''
    else:
        r = ct.find('.//' + qn('w:r'))
        if r is None:
            p = ct.find('.//' + qn('w:p'))
            parent = ct if p is None else p
            r = parent.makeelement(qn('w:r'), {})
            parent.append(r)
        t = r.makeelement(qn('w:t'), {})
        t.text = v
        r.append(t)
def fill(name, value):
    # 同じ名前の欄すべてに書く(表紙と2枚目に同じ欄がある様式のため)
    es = _sdts(name)
    if not es:
        raise SystemExit('記入欄「%s」が見つかりません(writer のフォームタブ「名前」で付けます)' % name)
    for e in es:
        _put(e, value)
    return len(es)
def fill_one(name, value):
    # 最初の一つにだけ書く
    es = _sdts(name)
    if not es:
        raise SystemExit('記入欄「%s」が見つかりません' % name)
    _put(es[0], value)
def _text(sdt):
    ct = sdt.find(qn('w:sdtContent'))
    if ct is None:
        return ''
    return ''.join(t.text or '' for t in ct.iter(qn('w:t')))
def extract(name):
    # 記入欄の値を読む(同じ名前が複数なら最初の一つ)。無い名前は断る
    es = _sdts(name)
    if not es:
        raise SystemExit('記入欄「%s」が見つかりません' % name)
    return _text(es[0])
def fields():
    # 名前つき記入欄の(名前, 値)の一覧 — 様式の仕様書。同じ名前は欄ごとに並ぶ
    out = []
    for sdt in d.element.iter(qn('w:sdt')):
        pr = sdt.find(qn('w:sdtPr'))
        tag = pr.find(qn('w:tag')) if pr is not None else None
        if tag is None:
            continue
        t = tag.get(qn('w:val')) or ''
        if t.startswith('jo:'):
            t = t.split(':', 2)[-1]
            if t in ('email', 'phone', 'complex', 'signature'):
                continue  # 種類の印だけ(名前なし)の欄
        if t:
            out.append((t, _text(sdt)))
    return out
def _tpl():
    try:
        from docxtpl import DocxTemplate
    except ImportError:
        raise SystemExit('docxtpl がありません(pip install docxtpl。.venv があればそちらへ)')
    return DocxTemplate(IN)
def render(ctx):
    # 雛形({{名前}} と {%tr for %} の行くり返し)に辞書を差し込む。
    # 以後の d は差し込み済みの文書になり、そのまま保存される
    global d
    t = _tpl()
    try:
        t.render(ctx)
    except Exception as e:
        raise SystemExit('雛形が壊れています(タグの置き方や全角の {{ }} を確かめてください): %s' % e)
    d = t.docx
    return d
def tpl_fields():
    # 雛形の差し込み口({{名前}})の一覧 — 雛形の仕様書。壊れていれば断る
    t = _tpl()
    try:
        return sorted(t.get_undeclared_template_variables())
    except Exception as e:
        raise SystemExit('雛形が壊れています: %s' % e)
"#;
    format!(
        concat!(
            "import docx\n",
            "IN = {in_d:?}\n",
            "d = docx.Document(IN)\n",
            "{fill}",
            "# ---- 利用者のコード(d = python-docx の文書 / fill(名前, 値)・\
             extract(名前)・fields() = 記入欄 / render(辞書)・tpl_fields() = \
             {{{{ }}}} の雛形) ----\n",
            "{code}\n",
            "# ----\n",
            "d.save({out_d:?})\n"
        ),
        in_d = in_d.to_string_lossy(),
        fill = FILL,
        out_d = out_d.to_string_lossy(),
        code = user_code
    )
}

/// プラグイン(.py)の置き場。~/.config/office/plugins
pub(crate) fn plugins_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".config/office/plugins")
}

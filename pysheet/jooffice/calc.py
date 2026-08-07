# -*- coding: utf-8 -*-
"""jooffice.calc — 動いている calc を Jupyter/Python から操る(xlwings 流)。

使い方(xlwings の import を1行差し替えるだけ):

    from jooffice import calc as xw
    import pandas as pd

    wb = xw.Book()                 # 新しいブック(未保存が無ければ)
    xw.Range('A1').value = df      # DataFrame をセルへ(見出し・index つき)
    df2 = xw.Range('A1').options(pd.DataFrame, expand='table').value
    wb.sheets['Sheet1'].range('B2').value = [[1, 2], [3, 4]]
    wb.save('集計.xlsx')

未対応(正直に): @xw.func / @xw.sub / Book.caller() — Excel のアドイン機構の
話なので、calc では「AI タブ → マクロを書く」(plugins/*.py)が同じ役目。
"""

import os

from . import JoofficeError, call as _shared_call

JocalcError = JoofficeError  # 旧名との互換


def _call(cmd, **kw):
    return _shared_call("calc", cmd, **kw)

def _col_name(n):
    # 0 → A, 25 → Z, 26 → AA
    s = ""
    n += 1
    while n > 0:
        n, r = divmod(n - 1, 26)
        s = chr(65 + r) + s
    return s


def _parse_a1(a1):
    # "B2" → (row0, col0)
    col = 0
    i = 0
    for ch in a1:
        if ch.isalpha():
            col = col * 26 + (ord(ch.upper()) - 64)
            i += 1
        else:
            break
    return int(a1[i:]) - 1, col - 1


class Options:
    def __init__(self, rng, convert=None, expand=None, index=True, header=True):
        self._rng = rng
        self._convert = convert
        self._expand = expand
        self._index = index
        self._header = header

    @property
    def value(self):
        rng = self._rng
        if self._expand == "table":
            rng = rng.expand("table")
        grid = rng._get()
        conv = self._convert
        if conv is not None and conv.__name__ == "DataFrame":
            import pandas as pd

            if not grid:
                return pd.DataFrame()
            if self._header:
                head, body = grid[0], grid[1:]
            else:
                head, body = None, grid
            df = pd.DataFrame(body, columns=head)
            if self._index and df.shape[1] >= 1:
                df = df.set_index(df.columns[0])
                # xlwings と同じく、見出しの欄が None なら index 名も無し
                if df.index.name is None or df.index.name == "":
                    df.index.name = None
            return df
        return rng._plain(grid)

    @value.setter
    def value(self, v):
        self._rng.value = v


class Range:
    def __init__(self, a1, sheet=None):
        if ":" in a1:
            left, right = a1.split(":", 1)
            self._r0, self._c0 = _parse_a1(left)
            self._r1, self._c1 = _parse_a1(right)
        else:
            self._r0, self._c0 = _parse_a1(a1)
            self._r1, self._c1 = self._r0, self._c0
        self._sheet = sheet  # None = いまのシート

    def _a1(self):
        a = "{}{}".format(_col_name(self._c0), self._r0 + 1)
        if (self._r0, self._c0) == (self._r1, self._c1):
            return a
        return "{}:{}{}".format(a, _col_name(self._c1), self._r1 + 1)

    def _kw(self):
        kw = {"a1": self._a1()}
        if self._sheet is not None:
            kw["sheet"] = self._sheet
        return kw

    def expand(self, mode="table"):
        if mode != "table":
            raise JocalcError("expand は 'table' だけに対応しています")
        r = _call("expand", **self._kw())
        out = Range(
            "{}{}".format(_col_name(self._c0), self._r0 + 1), sheet=self._sheet
        )
        out._r1 = self._r0 + r["rows"] - 1
        out._c1 = self._c0 + r["cols"] - 1
        return out

    def options(self, convert=None, **kw):
        return Options(self, convert=convert, **kw)

    def _get(self):
        return _call("get", **self._kw())["values"]

    def _plain(self, grid):
        # 1×1 はそのまま、1行/1列は1次元、他は2次元(xlwings と同じ)
        if len(grid) == 1 and len(grid[0]) == 1:
            return grid[0][0]
        if len(grid) == 1:
            return grid[0]
        if all(len(r) == 1 for r in grid):
            return [r[0] for r in grid]
        return grid

    @property
    def value(self):
        return self._plain(self._get())

    @value.setter
    def value(self, v):
        grid = _to_grid(v)
        _call("set", values=grid, **self._kw())

    @property
    def formula(self):
        f = _call("get_formula", **self._kw())["formulas"]
        return self._plain(f)

    @formula.setter
    def formula(self, v):
        # 式も set と同じ道(= から始まる文字列は式になる)
        self.value = v

    def __repr__(self):
        return "<jocalc.Range {}>".format(self._a1())


def _to_grid(v):
    # DataFrame / ndarray / 2次元 / 1次元 / スカラー を 2次元へ
    name = type(v).__name__
    if name == "DataFrame":
        rows = [
            [v.index.name if v.index.name is not None else ""]
            + [str(c) for c in v.columns]
        ]
        for idx, row in zip(v.index, v.itertuples(index=False)):
            rows.append([_scalar(idx)] + [_scalar(x) for x in row])
        return rows
    if name == "Series":
        return [[_scalar(x)] for x in v]
    if name == "ndarray":
        v = v.tolist()
        name = type(v).__name__
    if isinstance(v, (list, tuple)):
        if v and isinstance(v[0], (list, tuple)):
            return [[_scalar(x) for x in row] for row in v]
        return [[_scalar(x) for x in v]]  # 1次元は横1行(xlwings と同じ)
    return [[_scalar(v)]]


def _scalar(x):
    if x is None:
        return None
    if isinstance(x, bool):
        return x
    if isinstance(x, (int, float)):
        # NaN は空欄に
        if x != x:
            return None
        return x
    if hasattr(x, "item"):  # numpy の数
        try:
            return x.item()
        except Exception:
            pass
    if hasattr(x, "isoformat"):  # 日付は文字で(v1 の割り切り)
        return x.isoformat()
    return str(x)


class Sheet:
    def __init__(self, name):
        self.name = name

    def range(self, a1):
        return Range(a1, sheet=self.name)

    def __getitem__(self, a1):
        return self.range(a1)

    def __repr__(self):
        return "<jocalc.Sheet {}>".format(self.name)


class _Sheets:
    def __getitem__(self, key):
        info = _call("book_info")
        names = info["sheets"]
        if isinstance(key, int):
            return Sheet(names[key])
        if key in names:
            return Sheet(key)
        raise JocalcError("シート「{}」がありません".format(key))

    @property
    def active(self):
        info = _call("book_info")
        return Sheet(info["sheets"][info["active"]])

    def __iter__(self):
        return (Sheet(n) for n in _call("book_info")["sheets"])

    def __len__(self):
        return len(_call("book_info")["sheets"])


class Book:
    """動いている calc のブック。Book() = 新規(未保存が無ければ)、
    Book('path.xlsx') = そのファイルを開く。"""

    def __init__(self, path=None):
        if path is not None:
            info = _call("book_info")
            if info.get("path") != os.path.abspath(path):
                _call("open", path=os.path.abspath(path))
        else:
            _call("new")
        self.sheets = _Sheets()

    @staticmethod
    def attach():
        """新規も開くもせず、いま calc に出ているブックにそのまま付く。"""
        b = Book.__new__(Book)
        _call("ping")
        b.sheets = _Sheets()
        return b

    @staticmethod
    def caller():
        raise NotImplementedError(
            "Book.caller()/@func/@sub は Excel のアドイン機構の話です。"
            "calc では「AI タブ → マクロを書く」(plugins/*.py)が同じ役目"
        )

    @property
    def name(self):
        p = _call("book_info").get("path")
        return os.path.basename(p) if p else "ブック1"

    @property
    def fullname(self):
        return _call("book_info").get("path")

    def save(self, path=None):
        if path is not None:
            _call("save", path=os.path.abspath(path))
        else:
            _call("save")

    def __repr__(self):
        return "<jocalc.Book {}>".format(self.name)


class _Books:
    @property
    def active(self):
        return Book.attach()

    def open(self, path):
        return Book(path)

    def add(self):
        return Book()


books = _Books()


def ping():
    """calc が応じるかの確かめ。"""
    return _call("ping")

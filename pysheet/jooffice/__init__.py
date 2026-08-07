# -*- coding: utf-8 -*-
"""jooffice — 事務所一式(calc / writer)を Python から操る橋の共有部。

アプリごとの口はユニックスソケット($XDG_RUNTIME_DIR/jo-office/<app>.sock、
径路が AF_UNIX の 108 字上限を超えるときは /tmp/jo-office-UID/)。
JSON を1行ずつ。**この機械の中だけ**で、ネットには出ない。

表計算は `from jooffice import calc as xw`(xlwings 流の Book / Range)。
文書(writer)の口は今後ここに増える。
"""

import json
import os
import socket


class JoofficeError(RuntimeError):
    pass


# 旧名(jocalc 時代)との互換
JocalcError = JoofficeError


def sock_path(app):
    base = os.environ.get("XDG_RUNTIME_DIR")
    if base:
        p = os.path.join(base, "jo-office", app + ".sock")
        if len(p.encode()) <= 90:
            return p
    return os.path.join(
        "/tmp", "jo-office-{}".format(os.getuid()), app + ".sock"
    )


def call(app, cmd, **kw):
    req = {"cmd": cmd}
    req.update(kw)
    try:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(10.0)
        s.connect(sock_path(app))
    except OSError as e:
        raise JoofficeError(
            "{} に繋がりません({}: {})。起動してから使ってください".format(
                app, sock_path(app), e
            )
        ) from None
    try:
        s.sendall((json.dumps(req, ensure_ascii=False) + "\n").encode("utf-8"))
        buf = b""
        while not buf.endswith(b"\n"):
            chunk = s.recv(65536)
            if not chunk:
                break
            buf += chunk
    finally:
        s.close()
    resp = json.loads(buf.decode("utf-8"))
    if "err" in resp:
        raise JoofficeError(resp["err"])
    return resp

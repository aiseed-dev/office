//! main.rs からの純移動(2026-08-06 の分割)。挙動は変えない。

use crate::*;

/// 排他ロックの置き場(LibreOffice と同じ `.~lock.<名前>#`)。
/// ファイルサーバーの共有フォルダで「同時に開いて後勝ちで潰す」を防ぐ。
pub(crate) fn lock_path_for(p: &std::path::Path) -> std::path::PathBuf {
    let name = p.file_name().unwrap_or_default().to_string_lossy();
    p.with_file_name(format!(".~lock.{name}#"))
}

/// 自分の名乗り(誰が開いているか)。user@host。
pub(crate) fn lock_identity() -> String {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "?".into());
    let host = std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "?".into());
    format!("{user}@{host}")
}

/// 先客のロックを読む(あれば名乗りを返す)。自分自身のロックは先客と見ない。
pub(crate) fn foreign_lock(p: &std::path::Path) -> Option<String> {
    let lp = lock_path_for(p);
    let raw = std::fs::read_to_string(lp).ok()?;
    let who = raw
        .split(',')
        .map(str::trim)
        .find(|t| !t.is_empty())
        .unwrap_or("誰か")
        .to_string();
    (who != lock_identity()).then_some(who)
}

/// 署名の鍵の置き場。writer と共通の ~/.config/office/sign.key
pub(crate) fn sign_key_path() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".config/office/sign.key")
}

/// 署名の鍵を読む。無ければ作る(/dev/urandom の種。0600 で置く)
pub(crate) fn load_or_make_key() -> Result<ed25519_dalek::SigningKey, String> {
    let kp = sign_key_path();
    if let Ok(bytes) = std::fs::read(&kp) {
        let seed: [u8; 32] = bytes
            .get(..32)
            .and_then(|b| b.try_into().ok())
            .ok_or("鍵ファイルが壊れています(~/.config/office/sign.key)")?;
        return Ok(ed25519_dalek::SigningKey::from_bytes(&seed));
    }
    let mut seed = [0u8; 32];
    use std::io::Read as _;
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut seed))
        .map_err(|e| ui::tf!("乱数が取れません: {}", e))?;
    if let Some(dir) = kp.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&kp)
        .and_then(|mut f| f.write_all(&seed))
        .map_err(|e| ui::tf!("鍵が置けません: {}", e))?;
    Ok(ed25519_dalek::SigningKey::from_bytes(&seed))
}

pub(crate) fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub(crate) fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok())
        .collect()
}

/// 署名の添え書きの置き場。ブックの隣の 名前.xlsx.sig
pub(crate) fn sig_path_for(p: &std::path::Path) -> PathBuf {
    let mut os = p.as_os_str().to_owned();
    os.push(".sig");
    PathBuf::from(os)
}

impl Calc {
    /// 自分のロックを外す(閉じる・別のファイルへ移るとき)。
    pub(crate) fn release_lock(&mut self) {
        if let Some(lp) = self.my_lock.take() {
            let _ = std::fs::remove_file(lp);
        }
    }

    /// このファイルのロックを見て、先客が居れば警告、居なければ自分が取る。
    pub(crate) fn acquire_lock(&mut self, p: &std::path::Path) {
        self.release_lock();
        match foreign_lock(p) {
            Some(who) => {
                self.locked_by = Some(who);
                // ロックは取らない(先客の邪魔をしない)
            }
            None => {
                self.locked_by = None;
                let lp = lock_path_for(p);
                // LibreOffice と同じ気持ちの中身(名乗りだけ。厳密な書式は要らない)
                if std::fs::write(&lp, format!("{},;", lock_identity())).is_ok() {
                    self.my_lock = Some(lp);
                }
            }
        }
    }

    pub(crate) fn open(&mut self, p: PathBuf) {
        let bytes = match std::fs::read(&p) {
            Ok(b) => b,
            Err(e) => {
                self.status = format!("開けません: {e}").into();
                return;
            }
        };
        if ooxml::crypt::is_encrypted(&bytes) {
            // パネルでパスワードを聞き、Enter が続きをやる
            self.pw_pending = Some(p);
            self.prompt = Some(("pw-open", Editor::new("")));
            self.status =
                ui::t!("このブックは暗号化されています。パスワードを打って Enter").into();
            return;
        }
        self.open_plain(p, bytes);
    }

    /// 平文(zip)の xlsx を読み込む。open とパスワードのパネルの共通の続き。
    pub(crate) fn open_plain(&mut self, p: PathBuf, bytes: Vec<u8>) {
        // 前のブックのパスワードを引きずらない(暗号化して開いた時だけ
        // パネルの続きが後から入れ直す)
        self.encrypt_pw = None;
        match sheet::xlsx::read(std::io::Cursor::new(bytes)) {
            Ok((mut book, rep)) => {
                sheet::recalc_all(&mut book);
                self.notes = rep
                    .unsupported
                    .iter()
                    .map(|(n, c)| SharedString::from(format!("{n} × {c}")))
                    .collect();
                self.status = format!(
                    "{} シート / {} セル — {}",
                    rep.sheets,
                    rep.cells,
                    p.file_name().unwrap_or_default().to_string_lossy()
                )
                .into();
                self.book = book;
                // 計算方法はファイルの指定に従う(開いたときは上の一度きりの
                // 計算で値を見せ、以後の編集では勝手に回さない)
                self.auto_calc = !self.book.calc_manual;
                self.active = 0;
                self.cursor = Pos::new(0, 0);
                self.view = Pos::new(0, 0);
                self.anchor = None;
                self.frozen = None;
                self.auto_filter = None;
                self.filter_panel = None;
                self.sheet_ui.clear();
                self.undo_stack.clear();
                self.redo_stack.clear();
                self.clip_range = None;
                self.acquire_lock(&p);
                if let Some(who) = self.locked_by.clone() {
                    self.status = format!(
                        "{} — **{who} が開いています**。上書き保存はできません(名前を付けて保存へ)",
                        self.status
                    )
                    .into();
                }
                Self::note_recent(&p);
                self.path = Some(p);
                self.sync_input();
            }
            Err(e) => self.status = format!("開けません: {e}").into(),
        }
    }

    /// 上書きの前に、直前の中身を控えとして残す(最大9世代)。writer と
    /// 同じ作法: 同じフォルダの .jo-history/<ファイル名>/<日時>.xlsx。
    /// 名前は**その中身を保存した日時**(mtime)— いつの姿かが分かる。
    pub(crate) fn keep_version(&self, p: &std::path::Path) {
        let Some(name) = p.file_name().map(|n| n.to_string_lossy().to_string()) else {
            return;
        };
        let dir = p
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join(".jo-history")
            .join(&name);
        if std::fs::create_dir_all(&dir).is_err() {
            return; // 控えられなくても保存は止めない
        }
        let stamp = std::process::Command::new("date")
            .arg("-r")
            .arg(p)
            .arg("+%Y%m%d-%H%M%S")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|| "0".into());
        let _ = std::fs::copy(p, dir.join(format!("{stamp}.xlsx")));
        if let Ok(rd) = std::fs::read_dir(&dir) {
            let mut old: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
            old.sort();
            while old.len() > 9 {
                let _ = std::fs::remove_file(old.remove(0));
            }
        }
    }

    /// 控えの一覧(新しい順)。(表示名, パス)
    pub(crate) fn versions(&self) -> Vec<(String, PathBuf)> {
        let Some(p) = &self.path else { return Vec::new() };
        let Some(name) = p.file_name().map(|n| n.to_string_lossy().to_string()) else {
            return Vec::new();
        };
        let dir = p
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join(".jo-history")
            .join(&name);
        let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
        let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        v.sort();
        v.reverse();
        v.into_iter()
            .map(|q| {
                let stem = q
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                // 20260804-183012 → 2026-08-04 18:30
                let disp = if stem.len() >= 13 && stem.is_ascii() {
                    format!(
                        "{}-{}-{} {}:{}",
                        &stem[0..4], &stem[4..6], &stem[6..8], &stem[9..11], &stem[11..13]
                    )
                } else {
                    stem
                };
                let kb = std::fs::metadata(&q).map(|m| m.len() / 1024).unwrap_or(0);
                (format!("{disp}({kb} KB)"), q)
            })
            .collect()
    }

    /// 控えを開く。いまのファイルは動かさず、**名無しの複製**として読む
    /// (保存すると名前を聞く。元へ戻したいなら同じ名前で保存する —
    /// 黙って元のファイルを書き戻したりしない)。
    pub(crate) fn open_version(&mut self, q: &std::path::Path) {
        let raw = match std::fs::read(q) {
            Ok(b) => b,
            Err(e) => {
                self.status = format!("控えが読めません: {e}").into();
                return;
            }
        };
        let raw = if ooxml::crypt::is_encrypted(&raw) {
            match self.encrypt_pw.as_ref().map(|pw| ooxml::crypt::decrypt(&raw, pw)) {
                Some(Ok(b)) => b,
                _ => {
                    self.status =
                        ui::t!("控えは暗号化されています(いまのパスワードでは解けません)").into();
                    return;
                }
            }
        } else {
            raw
        };
        match sheet::xlsx::read(std::io::Cursor::new(raw)) {
            Ok((mut book, _rep)) => {
                sheet::recalc_all(&mut book);
                self.release_lock();
                self.locked_by = None;
                self.book = book;
                // 計算方法はファイルの指定に従う(開いたときは上の一度きりの
                // 計算で値を見せ、以後の編集では勝手に回さない)
                self.auto_calc = !self.book.calc_manual;
                self.active = 0;
                self.cursor = Pos::new(0, 0);
                self.view = Pos::new(0, 0);
                self.anchor = None;
                self.frozen = None;
                self.auto_filter = None;
                self.filter_panel = None;
                self.sheet_ui.clear();
                self.undo_stack.clear();
                self.redo_stack.clear();
                self.clip_range = None;
                self.path = None;
                self.dirty = true;
                self.sync_input();
                self.status = ui::t!("控えを開きました(名無しの複製。保存で名前を聞きます。元へ戻すなら同じ名前で保存)").into();
            }
            Err(e) => self.status = format!("控えが読めません: {e}").into(),
        }
    }

    /// 原本の中身(暗号化されていれば解いた平文)。部品の持ち越しに使う
    pub(crate) fn original_plain(&self) -> Option<Vec<u8>> {
        let bytes = std::fs::read(self.path.as_ref()?).ok()?;
        if ooxml::crypt::is_encrypted(&bytes) {
            let pw = self.encrypt_pw.as_ref()?;
            ooxml::crypt::decrypt(&bytes, pw).ok()
        } else {
            Some(bytes)
        }
    }

    /// 選択の生きた値(Excel の下端と同じ 合計・平均・個数)。
    /// 2セル以上を選んでいて、数のセルがあるときだけ出す。
    pub(crate) fn sel_stats(&self) -> Option<String> {
        self.anchor?;
        let (a, b) = self.sel_rect();
        let cells = (b.row - a.row + 1) as u64 * (b.col - a.col + 1) as u64;
        // 全選択のような巨大な矩形は数えない(描画のたびに走るので)
        if cells < 2 || cells > 200_000 {
            return None;
        }
        let mut n = 0u64;
        let mut sum = 0.0f64;
        for r in a.row..=b.row {
            // 絞り込みで隠れた行は数えない(Excel と同じ — 見えている行の値)
            if !self.filter_keeps(r) {
                continue;
            }
            for c in a.col..=b.col {
                if let Some(Value::Number(v)) =
                    self.sheet().get(Pos::new(r, c)).map(|x| &x.value)
                {
                    n += 1;
                    sum += *v;
                }
            }
        }
        if n == 0 {
            return None;
        }
        let avg = (sum / n as f64 * 100.0).round() / 100.0;
        Some(format!(
            "合計 {} / 平均 {} / 個数 {n}",
            Value::Number(sum).display(),
            Value::Number(avg).display()
        ))
    }

    /// チャット(申し送り帳)の置き場。ブックの隣の 名前.xlsx.chat.txt
    pub(crate) fn chat_path(&self) -> Option<PathBuf> {
        self.path.as_ref().map(|p| {
            let mut os = p.as_os_str().to_owned();
            os.push(".chat.txt");
            PathBuf::from(os)
        })
    }

    /// 最近開いた・保存したブックの控え(writer と同じ作法)
    pub(crate) fn recent_file() -> PathBuf {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(".config/office/recent-calc.txt")
    }

    pub(crate) fn note_recent(p: &std::path::Path) {
        let rf = Self::recent_file();
        if let Some(dir) = rf.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut list: Vec<String> = std::fs::read_to_string(&rf)
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_default();
        let me = p.to_string_lossy().to_string();
        list.retain(|x| *x != me);
        list.insert(0, me);
        list.truncate(12);
        let _ = std::fs::write(&rf, list.join("\n"));
    }

    pub(crate) fn recent_list() -> Vec<PathBuf> {
        std::fs::read_to_string(Self::recent_file())
            .map(|s| s.lines().map(PathBuf::from).filter(|p| p.exists()).collect())
            .unwrap_or_default()
    }

    /// 新しいブック。未保存の変更があるときは作らない(黙って捨てない)。
    pub(crate) fn new_book(&mut self) -> bool {
        if self.dirty {
            self.status =
                ui::t!("未保存の変更があります。先に保存してください(Ctrl+S)").into();
            return false;
        }
        self.release_lock();
        self.locked_by = None;
        self.path = None;
        self.encrypt_pw = None;
        self.notes = Vec::new();
        self.book = Book::new();
        self.active = 0;
        self.cursor = Pos::new(0, 0);
        self.view = Pos::new(0, 0);
        self.anchor = None;
        self.frozen = None;
        self.auto_filter = None;
                self.filter_panel = None;
        self.slicer = None;
        self.sheet_ui.clear();
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.dirty = false;
        self.sync_input();
        self.status = ui::t!("新しいブックです").into();
        true
    }

    /// 名前を付けて保存(いつでもダイアログ。別のスレッド — rfd は同期)
    pub(crate) fn save_as(&mut self, cx: &mut Context<Self>) {
        let ask = cx.background_executor().spawn(async {
            rfd::FileDialog::new()
                .add_filter("Excelブック", &["xlsx"])
                .add_filter("CSV(いまのシートの値だけ)", &["csv"])
                .save_file()
        });
        cx.spawn(async move |this, cx| {
            let r = ask.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(mut p) = r {
                    if p.extension().is_none() {
                        p.set_extension("xlsx");
                    }
                    if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("csv")) {
                        this.write_csv(&p);
                    } else {
                        this.save_to(p);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// いまのシートを CSV に書き出す(値だけ。UTF-8 BOM+CRLF — Excel が
    /// 文字化けせずに開ける形)。**self.path は動かさない** — CSV は式も
    /// 書式も他のシートも持てないので、「保存先」にはしない。
    pub(crate) fn export_csv_dialog(&mut self, cx: &mut Context<Self>) {
        self.commit();
        let name = format!("{}.csv", self.book.sheets[self.active].name);
        let ask = cx.background_executor().spawn(async move {
            rfd::FileDialog::new()
                .add_filter("CSV", &["csv"])
                .set_file_name(&name)
                .save_file()
        });
        cx.spawn(async move |this, cx| {
            let r = ask.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(p) = r {
                    this.write_csv(&p);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn write_csv(&mut self, p: &std::path::Path) {
        let s = &self.book.sheets[self.active];
        let (rows, cols) = s.extent();
        let mut out = String::from("\u{feff}"); // BOM — Excel の既定の読みに合わせる
        for r in 0..rows {
            let mut line: Vec<String> = Vec::new();
            for c in 0..cols.max(1) {
                let v = s
                    .get(sheet::Pos::new(r, c))
                    .map(|x| x.value.display())
                    .unwrap_or_default();
                if v.contains(',') || v.contains('"') || v.contains('\n') || v.contains('\r') {
                    line.push(format!("\"{}\"", v.replace('"', "\"\"")));
                } else {
                    line.push(v);
                }
            }
            out.push_str(&line.join(","));
            out.push_str("\r\n");
        }
        match std::fs::write(p, out) {
            Ok(()) => {
                // 何が入らないかを黙らない(CSV は値だけの形式)
                self.status = ui::tf!(
                    "CSV に書き出しました: {}(いまのシートの値だけ — 式・書式・他のシートは入りません)",
                    p.display()
                )
                .into();
            }
            Err(e) => {
                self.status = ui::tf!("CSV に書き出せませんでした: {}", e).into();
            }
        }
    }

    pub(crate) fn a_save(&mut self, _: &ui::Save, _: &mut Window, cx: &mut Context<Self>) {
        self.save(false, cx); cx.notify();
    }
    pub(crate) fn a_open(&mut self, _: &ui::Open, _: &mut Window, cx: &mut Context<Self>) {
        self.open_dialog(cx); cx.notify();
    }

    /// 開くファイルを選ぶ。**ダイアログは別のスレッド** — rfd は同期で、
    /// メインスレッドで開くと画面ごと固まる(終了確認と同じ作法)。
    pub(crate) fn open_dialog(&mut self, cx: &mut Context<Self>) {
        let ask = cx.background_executor().spawn(async {
            rfd::FileDialog::new().add_filter("Excelブック", &["xlsx"]).pick_file()
        });
        cx.spawn(async move |this, cx| {
            let r = ask.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(p) = r {
                    this.open(p);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 終了の要求。書きかけが無ければ即終了、あれば確認を**別のスレッド**で出す。
    /// 確認のダイアログでメインスレッドを塞がない — 塞ぐと画面ごと固まり、
    /// GNOME に「応答なし」と判定される(踏んで直した)。
    /// 「はい」でも保存できなかった(保存の窓を閉じた等)なら終了しない —
    /// 書きかけを黙って捨てない。
    pub(crate) fn request_quit(&mut self, cx: &mut Context<Self>) {
        self.commit();
        // 確認を出すのは**未保存の変更があるとき**。名前の無い新規でも、
        // 何か打ってあれば出す — 打った物を黙って捨てない(発注者 2026-08-06。
        // 2026-08-03 の「実ファイルに限る」を改訂: 新規が見本入りだった頃は
        // 「試し打ち」扱いでよかったが、空白の新規は実の仕事が始まる場所)。
        // 本当に空のままの新規は、従来どおり黙って閉じる(煩くしない)
        let empty_new = self.path.is_none()
            && self.book.sheets.iter().all(|s| s.cells.is_empty());
        if !self.dirty || empty_new {
            self.release_lock();
            cx.quit();
            return;
        }
        // 確認は**窓の中のパネル**で出す。rfd の OS ダイアログは親窓を持てず
        // **スクリーンの中央**に出て、窓から離れすぎる(発注者 2026-08-06)
        self.quit_ask = true;
        cx.notify();
    }

    pub(crate) fn a_quit(&mut self, _: &ui::Quit, _: &mut Window, cx: &mut Context<Self>) {
        self.request_quit(cx);
    }

    /// PDF に書き出す。保存先の選択は**別のスレッド**(rfd は同期)。
    pub(crate) fn save_pdf(&mut self, cx: &mut Context<Self>) {
        self.commit();
        let ask = cx.background_executor().spawn(async {
            rfd::FileDialog::new()
                .add_filter("PDF", &["pdf"])
                .set_file_name("帳票.pdf")
                .save_file()
        });
        cx.spawn(async move |this, cx| {
            let r = ask.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(p) = r {
                    this.write_pdf(&p);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 画像ファイルを選んで、いまのセルに浮かべる(選択は別のスレッド)。
    pub(crate) fn insert_image_dialog(&mut self, cx: &mut Context<Self>) {
        let ask = cx.background_executor().spawn(async {
            rfd::FileDialog::new()
                .add_filter("画像", &["png", "jpg", "jpeg", "bmp", "gif"])
                .pick_file()
        });
        cx.spawn(async move |this, cx| {
            let r = ask.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(p) = r {
                    match std::fs::read(&p) {
                        Ok(data) => match image_px(&data) {
                            Some((w, h)) => {
                                this.checkpoint();
                                let at = this.cursor;
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
                                    "画像を {} に置きました(保存で xlsx に入ります)",
                                    at.a1()
                                )
                                .into();
                            }
                            None => {
                                this.status =
                                    ui::t!("この画像は読めません(PNG か JPEG を選んでください)").into();
                            }
                        },
                        Err(e) => this.status = format!("読めません: {e}").into(),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn write_pdf(&mut self, p: &std::path::Path) {
        let (fam, exact) = match kumihan::font::for_document(None) {
            Ok(x) => x,
            Err(e) => {
                self.status = format!("PDF にできません: {e}").into();
                return;
            }
        };
        let data = match kumihan::font::load(fam) {
            Ok(d) => d,
            Err(e) => {
                self.status = format!("PDF にできません: {e}").into();
                return;
            }
        };
        // 帳票の印刷設定(pageSetup / pageMargins / Print_Area)に従う。
        // 効かせたものは status に言う(黙って既定で出さない)
        let sh = &self.book.sheets[self.active];
        let mut paper = paper::Paper::default();
        let mut desc: Vec<String> = Vec::new();
        if let Some(code) = sh.paper_size {
            match paper_mm(code) {
                Some((w, h, name)) => {
                    paper.width_mm = w;
                    paper.height_mm = h;
                    if code != 9 {
                        desc.push(name.into());
                    }
                }
                None => desc.push(format!("用紙コード{code}は未対応・A4で出します")),
            }
        }
        if sh.landscape {
            std::mem::swap(&mut paper.width_mm, &mut paper.height_mm);
            desc.push(ui::t!("横向き").into());
        }
        let areas = sh.print_areas.clone();
        let setup = paper::grid::PrintSetup {
            area: areas.first().copied(),
            margins_mm: sh.margins_mm,
        };
        if let Some((a, b)) = setup.area {
            desc.push(format!("印刷範囲 {}:{}", a.a1(), b.a1()));
        }
        if areas.len() > 1 {
            desc.push(format!("残り {} 域の印刷範囲はまだ出せません", areas.len() - 1));
        }
        let mut clipped = 0u32;
        let r = kumihan::atomic::save(p, |f| {
            paper::grid::sheet_to_pdf(
                &self.book.sheets[self.active],
                &data,
                paper,
                &setup,
                std::io::BufWriter::new(f),
            )
            .map(|n| clipped = n)
        });
        self.status = match r {
            // 紙に入り切らなかった列は黙らない
            Ok(_) => format!(
                "PDF にしました — {}{}{}{}",
                p.file_name().unwrap_or_default().to_string_lossy(),
                if desc.is_empty() {
                    String::new()
                } else {
                    format!("({})", desc.join("・"))
                },
                if exact { "" } else { " ※代替フォント" },
                if clipped > 0 {
                    format!("({clipped} 列は1列で紙より広く、切れています — 幅を詰めるか用紙を大きく)")
                } else {
                    String::new()
                }
            )
            .into(),
            Err(e) => format!("PDF にできません: {e}").into(),
        };
    }

    /// 保存。名前が無ければ選ばせる(**ダイアログは別のスレッド**)。
    /// `then_quit` なら保存が済んだときだけ終了する — 書きかけを黙って捨てない。
    pub(crate) fn save(&mut self, then_quit: bool, cx: &mut Context<Self>) {
        self.commit();
        if let Some(p) = self.path.clone() {
            if self.locked_by.is_some() {
                // 先客の作業を後勝ちで潰さない。別の名前でなら保存できる
                self.status = ui::tf!("{} が開いているため上書きしません。名前を付けて保存してください", self.locked_by.as_deref().unwrap_or("誰か"))
                .into();
            } else {
                self.save_to(p);
                if then_quit && !self.dirty {
                    self.release_lock();
                    cx.quit();
                }
                return;
            }
        }
        let ask = cx.background_executor().spawn(async {
            rfd::FileDialog::new()
                .add_filter("Excelブック", &["xlsx"])
                .save_file()
        });
        cx.spawn(async move |this, cx| {
            let r = ask.await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Some(p) => {
                        this.save_to(p);
                        if then_quit && !this.dirty {
                            this.release_lock();
                            cx.quit();
                        }
                    }
                    None => this.status = ui::t!("保存をやめました(名前が決まっていません)").into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 決まった場所へ書く。成功すると dirty が消える。
    pub(crate) fn save_to(&mut self, p: PathBuf) {
        // 原本の部品(図形・テーマ・印刷設定)を持ち越す。読み終えてから書く。
        // 暗号化されていた原本は解いた平文を渡す
        let original: Option<std::io::Cursor<Vec<u8>>> =
            self.original_plain().map(std::io::Cursor::new);
        // 上書きの前に、直前の中身をバージョン履歴に控える
        if p.exists() {
            self.keep_version(&p);
        }
        let saved = if let Some(pw) = self.encrypt_pw.clone() {
            // 暗号化は zip 丸ごとが単位 — 一度メモリへ書いてから包む。
            // Agile 方式(AES-256。Excel 2013+ の既定)で書く — 本物と相互
            // 検証済み。読みは Standard(2007)も Agile も両方できる
            let mut plain = Vec::new();
            sheet::xlsx::write_with(&self.book, original, std::io::Cursor::new(&mut plain))
                .and_then(|_| ooxml::crypt::encrypt_agile(&plain, &pw))
                .and_then(|enc| {
                    kumihan::atomic::save(&p, |mut f| {
                        use std::io::Write as _;
                        f.write_all(&enc).map_err(|e| e.to_string())
                    })
                })
        } else {
            kumihan::atomic::save(&p, |f| {
                sheet::xlsx::write_with(&self.book, original, std::io::BufWriter::new(f))
            })
        };
        match saved {
            Ok(_) => {
                let enc_note = if self.encrypt_pw.is_some() { "(暗号化)" } else { "" };
                self.status = ui::tf!("保存しました — {}{}", p.file_name().unwrap_or_default().to_string_lossy(), enc_note)
                .into();
                self.acquire_lock(&p);
                Self::note_recent(&p);
                self.path = Some(p);
                self.dirty = false;
                // 挿した絵はもう原本(いま書いたファイル)にある。次の保存で
                // 二重に書かないよう「読んだ側」へ持ち場を移す
                for sh in &mut self.book.sheets {
                    let moved: Vec<_> = sh.images_new.drain(..).collect();
                    sh.images.extend(moved);
                    let moved: Vec<_> = sh.shapes_new.drain(..).collect();
                    sh.shapes.extend(moved);
                }
                self.shape_sel = None;
            }
            Err(e) => self.status = ui::tf!("保存できません: {}", e).into(),
        }
    }
}

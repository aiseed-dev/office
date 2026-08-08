//! writer の外との出入り(main.rs から純移動 2026-08-08。部屋割りの4歩目)。
//! 排他ロック・署名の鍵・網(HTTP)・URL の組み立てと名乗り。**純移動**

use crate::*;

pub(crate) fn lock_path_for(p: &std::path::Path) -> PathBuf {
    let name = p.file_name().unwrap_or_default().to_string_lossy();
    p.with_file_name(format!(".~lock.{name}#"))
}

/// 署名の鍵の置き場。~/.config/office/sign.key(秘密鍵の種 32 バイト)
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

/// 署名の添え書きの置き場。文書の隣の 名前.docx.sig
pub(crate) fn sig_path_for(p: &std::path::Path) -> PathBuf {
    let mut os = p.as_os_str().to_owned();
    os.push(".sig");
    PathBuf::from(os)
}

/// 小さな HTTP(http と https。公開 Web も見える — 発注者 2026-08-04)。
/// HTTP/1.0 で頼む(chunked を受けない素直な形)。転送(3xx)は5回まで
/// 追いかける。GET は body=None、POST は form の urlencoded。
/// 返り値は (中身, 最終 URL)
pub(crate) fn http_fetch(url: &str, body: Option<&str>) -> Result<(Vec<u8>, String), String> {
    use std::io::{BufRead as _, Read as _, Write as _};
    let mut url = url.to_string();
    for _ in 0..5 {
        let (https, rest) = if let Some(r) = url.strip_prefix("https://") {
            (true, r)
        } else if let Some(r) = url.strip_prefix("http://") {
            (false, r)
        } else {
            return Err(ui::t!("http:// か https:// の URL にしてください").into());
        };
        let (hostport, path) = match rest.split_once('/') {
            Some((h, p)) => (h.to_string(), format!("/{p}")),
            None => (rest.to_string(), "/".to_string()),
        };
        let host = hostport.split(':').next().unwrap_or(&hostport).to_string();
        let addr = if hostport.contains(':') {
            hostport.clone()
        } else {
            format!("{hostport}:{}", if https { 443 } else { 80 })
        };
        let sock = std::net::TcpStream::connect(&addr)
            .map_err(|e| ui::tf!("繋がりません({}): {}", addr, e))?;
        sock.set_read_timeout(Some(std::time::Duration::from_secs(15))).ok();
        let req = match body {
            Some(b) => format!(
                "POST {path} HTTP/1.0\r\nHost: {hostport}\r\n                 User-Agent: aiseed-writer\r\n                 Content-Type: application/x-www-form-urlencoded\r\n                 Content-Length: {}\r\nConnection: close\r\n\r\n{b}",
                b.len()
            ),
            None => format!(
                "GET {path} HTTP/1.0\r\nHost: {hostport}\r\n                 User-Agent: aiseed-writer\r\nConnection: close\r\n\r\n"
            ),
        };
        // http と https を同じ道で読むための入れ物
        let mut stream: Box<dyn ReadWrite> = if https {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let cfg = rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth();
            let name = rustls::pki_types::ServerName::try_from(host.clone())
                .map_err(|_| ui::tf!("ホスト名が変です: {}", host))?;
            let conn = rustls::ClientConnection::new(std::sync::Arc::new(cfg), name)
                .map_err(|e| e.to_string())?;
            Box::new(rustls::StreamOwned::new(conn, sock))
        } else {
            Box::new(sock)
        };
        stream.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
        let mut r = std::io::BufReader::new(stream);
        let mut status = String::new();
        r.read_line(&mut status).map_err(|e| e.to_string())?;
        let mut location: Option<String> = None;
        let mut line = String::new();
        loop {
            line.clear();
            if r.read_line(&mut line).map_err(|e| e.to_string())? == 0
                || line.trim().is_empty()
            {
                break;
            }
            if line.to_ascii_lowercase().starts_with("location:") {
                location = Some(line[9..].trim().to_string());
            }
        }
        if status.contains(" 30") {
            if let Some(loc) = location {
                url = resolve_url(&url, &loc);
                continue;
            }
        }
        let mut out = Vec::new();
        r.read_to_end(&mut out).map_err(|e| e.to_string())?;
        if !status.contains(" 200") {
            return Err(ui::tf!("サーバーの答え: {}", status.trim()));
        }
        return Ok((out, url));
    }
    Err(ui::t!("転送が多すぎます(5回まで)").into())
}

trait ReadWrite: std::io::Read + std::io::Write {}
impl<T: std::io::Read + std::io::Write> ReadWrite for T {}

/// 相対の URL を今の場所から解く(部分集合)
pub(crate) fn resolve_url(base: &str, href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_string();
    }
    let scheme_end = base.find("://").map(|i| i + 3).unwrap_or(0);
    let host_end = base[scheme_end..]
        .find('/')
        .map(|i| scheme_end + i)
        .unwrap_or(base.len());
    if let Some(rest) = href.strip_prefix("//") {
        return format!("{}{rest}", &base[..scheme_end]);
    }
    if href.starts_with('/') {
        return format!("{}{href}", &base[..host_end]);
    }
    match base.rfind('/').filter(|i| *i > host_end) {
        Some(i) => format!("{}{href}", &base[..i + 1]),
        None => format!("{}/{href}", &base[..host_end]),
    }
}

pub(crate) fn urlenc(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                o.push(b as char)
            }
            b' ' => o.push('+'),
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}

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

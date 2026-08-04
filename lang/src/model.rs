//! モデルへの問い合わせ。OpenAI互換の `/v1/chat/completions` だけを話す。
//!
//! 宛先はローカル(既定 127.0.0.1)。**基幹網は外に出ない**。
//! Radeon Cloud の専用インスタンスも、SSH のポート転送で 127.0.0.1 に見えるので
//! **同じ経路で届く** — アプリ側の書き換えは要らない。
//!
//! TLS は持たない。素の HTTP だけを話す。
//! HTTPS の宛先(共有API など)へは SSH 転送か手元の中継を挟む。
//! **暗号の実装を自前で抱えないための割り切り**であって、機能不足ではない。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// 宛先。すべて実行時に差し替えられる — **差し替えるのはモデルであって、コードではない**。
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    pub path: String,
    pub model: String,
    /// Radeon Cloud の専用エンドポイントは Bearer を要求する
    pub api_key: Option<String>,
    pub timeout: Duration,
}

impl Default for Endpoint {
    fn default() -> Self {
        Self {
            host: var("OFFICE_HOST", "127.0.0.1"),
            port: std::env::var("OFFICE_PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(8000),
            path: var("OFFICE_PATH", "/v1/chat/completions"),
            model: var("OFFICE_MODEL", "local"),
            api_key: std::env::var("OFFICE_API_KEY").ok().filter(|s| !s.is_empty()),
            timeout: Duration::from_secs(
                std::env::var("OFFICE_TIMEOUT").ok().and_then(|s| s.parse().ok()).unwrap_or(120),
            ),
        }
    }
}

fn var(k: &str, d: &str) -> String {
    std::env::var(k).ok().filter(|s| !s.is_empty()).unwrap_or_else(|| d.into())
}

/// 1回のやりとりの結果。速度の計測に使うので、**トークン数と所要時間を必ず持ち帰る**。
#[derive(Debug, Clone, Default)]
pub struct Reply {
    pub content: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub elapsed_ms: u128,
}

impl Reply {
    /// 生成側の tok/s。時間が 0 のときは 0 を返す(割り算で落とさない)
    pub fn tokens_per_sec(&self) -> f64 {
        if self.elapsed_ms == 0 {
            return 0.0;
        }
        self.completion_tokens as f64 * 1000.0 / self.elapsed_ms as f64
    }
}

/// JSON の文字列として安全にする。
pub fn esc(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => {}
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

/// system と user を投げて、返事を受け取る。
///
/// **繋がらなければ Err**。黙って空を返さない —
/// 「指摘なし」「ふりがな不要」と読み違えられるため。
pub fn chat(ep: &Endpoint, system: &str, user: &str, temperature: f32) -> Result<Reply, String> {
    let body = format!(
        r#"{{"model":"{}","temperature":{},"messages":[{{"role":"system","content":"{}"}},{{"role":"user","content":"{}"}}]}}"#,
        esc(&ep.model),
        temperature,
        esc(system),
        esc(user)
    );
    let t0 = std::time::Instant::now();
    let raw = post(ep, &body)?;
    let elapsed_ms = t0.elapsed().as_millis();
    let content = extract_content(&raw)
        .ok_or_else(|| format!("モデルの応答を読めません: {}", head(&raw, 200)))?;
    Ok(Reply {
        content,
        prompt_tokens: usage(&raw, "prompt_tokens"),
        completion_tokens: usage(&raw, "completion_tokens"),
        elapsed_ms,
    })
}

fn head(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn post(ep: &Endpoint, body: &str) -> Result<String, String> {
    let addr = format!("{}:{}", ep.host, ep.port);
    let mut s = TcpStream::connect(&addr)
        .map_err(|e| format!("モデルに繋がりません({addr}): {e}"))?;
    s.set_read_timeout(Some(ep.timeout)).ok();
    let auth = match &ep.api_key {
        Some(k) => format!("Authorization: Bearer {k}\r\n"),
        None => String::new(),
    };
    let req = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\n{}\
         Content-Length: {}\r\nConnection: close\r\n\r\n{}",
        ep.path,
        ep.host,
        auth,
        body.as_bytes().len(),
        body
    );
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut r = BufReader::new(s);
    let mut status = String::new();
    r.read_line(&mut status).map_err(|e| e.to_string())?;
    // ヘッダを読み飛ばす
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line).map_err(|e| e.to_string())? == 0 || line.trim().is_empty() {
            break;
        }
    }
    let mut out = String::new();
    r.read_to_string(&mut out).map_err(|e| e.to_string())?;
    // 4xx/5xx を成功として扱わない
    if !status.contains(" 200") {
        return Err(format!("モデルが拒否しました: {} / {}", status.trim(), head(&out, 200)));
    }
    Ok(out)
}

/// https で POST して本文を返す(AI の宛先が外のときだけ使う)。
/// **鍵は呼ぶ側が渡す** — この関数はどこにも控えない
pub fn post_https(
    host: &str,
    path: &str,
    body: &str,
    headers: &[(&str, &str)],
) -> Result<String, String> {
    use std::io::{BufRead as _, Read as _, Write as _};
    let sock = TcpStream::connect((host, 443))
        .map_err(|e| format!("繋がりません({host}): {e}"))?;
    sock.set_read_timeout(Some(Duration::from_secs(120))).ok();
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let cfg = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from(host.to_string())
        .map_err(|_| format!("ホスト名が変です: {host}"))?;
    let conn = rustls::ClientConnection::new(std::sync::Arc::new(cfg), name)
        .map_err(|e| e.to_string())?;
    let mut st = rustls::StreamOwned::new(conn, sock);
    let mut req = format!(
        "POST {path} HTTP/1.0\r\nHost: {host}\r\nContent-Type: application/json\r\n"
    );
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.as_bytes().len()
    ));
    st.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut r = BufReader::new(st);
    let mut status = String::new();
    r.read_line(&mut status).map_err(|e| e.to_string())?;
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line).map_err(|e| e.to_string())? == 0 || line.trim().is_empty() {
            break;
        }
    }
    let mut out = String::new();
    r.read_to_string(&mut out).map_err(|e| e.to_string())?;
    if !status.contains(" 200") {
        return Err(format!("{}: {}", status.trim(), head(&out, 200)));
    }
    Ok(out)
}

/// JSON から `"鍵":"…"` の値を1つ取り出す(完全な処理系は持たない)。
/// Claude の応答は content[0].text なので、text を引けば足りる
pub fn extract_text_field(raw: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\"");
    let i = raw.find(&pat)?;
    let rest = &raw[i + pat.len()..];
    let q = rest.find('"')?;
    let mut out = String::new();
    let mut it = rest[q + 1..].chars();
    while let Some(c) = it.next() {
        match c {
            '"' => return Some(out),
            '\\' => match it.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => {}
                'u' => {
                    let h: String = it.by_ref().take(4).collect();
                    if let Some(ch) =
                        u32::from_str_radix(&h, 16).ok().and_then(char::from_u32)
                    {
                        out.push(ch);
                    }
                }
                o => out.push(o),
            },
            o => out.push(o),
        }
    }
    None
}

/// OpenAI互換の応答から content を取り出す(JSON の完全な処理系は持たない)。
pub fn extract_content(raw: &str) -> Option<String> {
    let key = "\"content\"";
    let i = raw.find(key)?;
    let rest = &raw[i + key.len()..];
    let q = rest.find('"')?;
    let mut out = String::new();
    let mut it = rest[q + 1..].chars();
    while let Some(c) = it.next() {
        match c {
            '"' => return Some(out),
            '\\' => match it.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => {}
                'u' => {
                    let h: String = it.by_ref().take(4).collect();
                    if let Some(ch) = u32::from_str_radix(&h, 16).ok().and_then(char::from_u32) {
                        out.push(ch);
                    }
                }
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

/// usage の数値を拾う。無ければ 0(速度の計算側で 0 除算しないこと)。
pub fn usage(raw: &str, field: &str) -> u64 {
    let k = format!("\"{field}\"");
    let Some(i) = raw.find(&k) else { return 0 };
    raw[i + k.len()..]
        .trim_start_matches(|c: char| c == ':' || c.is_whitespace())
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

/// JSON っぽい塊の中から、指定した名前の文字列を取り出す。
pub fn field(obj: &str, name: &str) -> Option<String> {
    let k = format!("\"{name}\"");
    let i = obj.find(&k)?;
    let rest = &obj[i + k.len()..];
    let c = rest.find(':')?;
    let q = rest[c..].find('"')? + c;
    let mut s = String::new();
    let mut it = rest[q + 1..].chars();
    while let Some(ch) = it.next() {
        match ch {
            '"' => return Some(s),
            '\\' => {
                if let Some(n) = it.next() {
                    s.push(if n == 'n' { '\n' } else { n })
                }
            }
            ch => s.push(ch),
        }
    }
    None
}

/// 最上位の `{...}` を順に切り出す(入れ子は数える)。
pub fn objects(content: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0usize, 0usize);
    let mut in_str = false;
    let mut esc_next = false;
    for (i, ch) in content.char_indices() {
        if in_str {
            if esc_next {
                esc_next = false;
            } else if ch == '\\' {
                esc_next = true;
            } else if ch == '"' {
                in_str = false;
            }
            continue;
        }
        match ch {
            '"' => in_str = true,
            '{' => {
                if depth == 0 {
                    start = i;
                }
                depth += 1;
            }
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    out.push(&content[start..=i]);
                }
            }
            _ => {}
        }
    }
    out
}

/// JSON の文字列配列 `["あ","い"]` を読む。順序を保つ — **候補は順序が意味を持つ**。
pub fn string_array(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_str = false;
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if in_str {
            match c {
                '"' => {
                    out.push(std::mem::take(&mut cur));
                    in_str = false;
                }
                '\\' => {
                    if let Some(n) = it.next() {
                        cur.push(if n == 'n' { '\n' } else { n })
                    }
                }
                c => cur.push(c),
            }
        } else if c == '"' {
            in_str = true;
        } else if c == ']' {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 応答からcontentを取り出せる() {
        let raw = r#"{"id":"x","choices":[{"message":{"role":"assistant","content":"[{\"found\":\"以外\"}]"}}]}"#;
        assert_eq!(extract_content(raw).unwrap(), r#"[{"found":"以外"}]"#);
    }

    #[test]
    fn usageを拾える() {
        let raw = r#"{"usage":{"prompt_tokens":1234,"completion_tokens": 56}}"#;
        assert_eq!(usage(raw, "prompt_tokens"), 1234);
        assert_eq!(usage(raw, "completion_tokens"), 56);
        assert_eq!(usage(raw, "無い名前"), 0, "無ければ0(速度計算で落とさない)");
    }

    #[test]
    fn 速度は時間0でも落ちない() {
        let r = Reply { completion_tokens: 100, elapsed_ms: 0, ..Default::default() };
        assert_eq!(r.tokens_per_sec(), 0.0);
        let r = Reply { completion_tokens: 100, elapsed_ms: 1000, ..Default::default() };
        assert!((r.tokens_per_sec() - 100.0).abs() < 1e-9);
    }

    #[test]
    fn 文字列の中の括弧に騙されない() {
        // 本文に { } が入っていても塊の切り出しが壊れない
        let c = r#"[{"base":"式","readings":["しき"],"note":"{ } を含む"}]"#;
        let o = objects(c);
        assert_eq!(o.len(), 1, "塊を1つに切り出せていない: {o:?}");
        assert_eq!(field(o[0], "base").unwrap(), "式");
    }

    #[test]
    fn 候補の配列は順序を保つ() {
        let v = string_array(r#"["のち","あと","うし","ご"]"#);
        assert_eq!(v, vec!["のち", "あと", "うし", "ご"], "候補は順序が意味を持つ");
    }

    #[test]
    fn 繋がらなければエラーを返す() {
        // 使えないときに「指摘なし」を返さない
        let ep = Endpoint { port: 1, ..Default::default() };
        let e = chat(&ep, "s", "u", 0.0).unwrap_err();
        assert!(e.contains("繋がりません"), "{e}");
    }
}

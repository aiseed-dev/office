//! AI の宛先(SEKKEI「AI メニュー」の宛先の規律)。
//!
//! **どこへ送るかは人が決める。** 文書やブックの中身を外へ出すかは
//! 現場の判断なので、既定は手元(ローカル)のまま、選んだ宛先を
//! `~/.config/office/ai.txt` に覚える。**鍵が文書に入ることは決してない。**
//!
//! 宛先は3つ:
//!
//! - **ローカル** — 127.0.0.1 の OpenAI 互換(校正と同じ口)。外に出ない
//! - **Claude(定額)** — 手元の `claude` コマンド(Claude Code の CLI)を
//!   子で呼ぶ。**API の鍵は要らない** — CLI が持っている認証(定額の
//!   契約でも)をそのまま使う。発注者の指定(2026-08-04)
//! - **Claude(API)** — Anthropic の API。鍵は環境変数 `ANTHROPIC_API_KEY`
//!   からだけ読む(設定ファイルにも文書にも書かない)
//!
//! 繋がらなければ「できません」と言う。**黙って空の結果にしない。**

use crate::model::{self, Endpoint};

/// AI の宛先
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Backend {
    /// 手元のモデル(既定。基幹網の外に出ない)
    #[default]
    Local,
    /// Claude Code の CLI(定額の契約でも使える)
    ClaudeCli,
    /// Anthropic の API(鍵は環境変数から)
    ClaudeApi,
}

impl Backend {
    pub fn label(self) -> &'static str {
        match self {
            Backend::Local => "手元のモデル(外に出ない)",
            Backend::ClaudeCli => "Claude(定額。手元の claude を使う)",
            Backend::ClaudeApi => "Claude(API。鍵は環境変数から)",
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Backend::Local => "local",
            Backend::ClaudeCli => "claude-cli",
            Backend::ClaudeApi => "claude-api",
        }
    }
    fn from_str(s: &str) -> Backend {
        match s.trim() {
            "claude-cli" => Backend::ClaudeCli,
            "claude-api" => Backend::ClaudeApi,
            _ => Backend::Local,
        }
    }
    /// 次の宛先(釦で回して選ぶ)
    pub fn next(self) -> Backend {
        match self {
            Backend::Local => Backend::ClaudeCli,
            Backend::ClaudeCli => Backend::ClaudeApi,
            Backend::ClaudeApi => Backend::Local,
        }
    }
}

fn config_path() -> std::path::PathBuf {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default()
        .join(".config/office/ai.txt")
}

/// 覚えている宛先を読む(環境変数 `JO_AI` が優先)
pub fn backend() -> Backend {
    if let Ok(v) = std::env::var("JO_AI") {
        return Backend::from_str(&v);
    }
    std::fs::read_to_string(config_path())
        .map(|s| Backend::from_str(&s))
        .unwrap_or_default()
}

/// 宛先を覚える
pub fn set_backend(b: Backend) {
    let p = config_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(p, b.as_str());
}

/// `claude` コマンドの居場所。**定額の契約で使う道** —
/// 鍵は要らず、CLI が持っている認証をそのまま使う
pub fn claude_cli() -> Option<std::path::PathBuf> {
    if let Some(p) = std::env::var_os("JO_CLAUDE") {
        let p = std::path::PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    // PATH から
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let p = std::path::Path::new(dir).join("claude");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    // よくある置き場
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from)?;
    for rel in [
        ".local/bin/claude",
        ".npm-global/bin/claude",
        ".volta/bin/claude",
        ".bun/bin/claude",
    ] {
        let p = home.join(rel);
        if p.is_file() {
            return Some(p);
        }
    }
    for p in ["/usr/local/bin/claude", "/opt/homebrew/bin/claude"] {
        let p = std::path::Path::new(p);
        if p.is_file() {
            return Some(p.to_path_buf());
        }
    }
    None
}

/// いまの宛先が使えるか。使えないなら**理由を言う**(黙って空にしない)
pub fn ready(b: Backend) -> Result<(), String> {
    match b {
        Backend::Local => Ok(()),
        Backend::ClaudeCli => claude_cli().map(|_| ()).ok_or_else(|| {
            "claude が見つかりません。定額の契約で使うには Claude Code の \
             CLI を入れてください(npm i -g @anthropic-ai/claude-code)。\
             置き場が変わっているなら JO_CLAUDE に道を教えてください"
                .to_string()
        }),
        Backend::ClaudeApi => {
            if std::env::var("ANTHROPIC_API_KEY").is_ok_and(|k| !k.is_empty()) {
                Ok(())
            } else {
                Err("ANTHROPIC_API_KEY がありません(鍵は環境変数からだけ読みます \
                     — 文書にも設定にも書きません)"
                    .to_string())
            }
        }
    }
}

/// 頼む。返るのは本文だけ。**失敗は必ず言葉で返す**
pub fn ask(b: Backend, system: &str, user: &str) -> Result<String, String> {
    ready(b)?;
    match b {
        Backend::Local => {
            let ep = Endpoint::default();
            model::chat(&ep, system, user, 0.2).map(|r| r.content)
        }
        Backend::ClaudeCli => claude_cli_ask(system, user),
        Backend::ClaudeApi => claude_api_ask(system, user),
    }
}

/// Claude Code の CLI に頼む(定額の契約でも動く)。
/// 一問一答の形(`-p`)で呼び、答えだけを受け取る。
/// **道具は与えない**(`--allowed-tools` を空に)— 手元のファイルを
/// 触らせない。文書の中身は標準入力で渡す(コマンド行に載せない)
fn claude_cli_ask(system: &str, user: &str) -> Result<String, String> {
    use std::io::Write as _;
    let exe = claude_cli().ok_or("claude が見つかりません")?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("-p")
        .arg("--output-format")
        .arg("text")
        .arg("--allowed-tools")
        .arg("")
        .arg("--append-system-prompt")
        .arg(system)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Ok(m) = std::env::var("JO_AI_MODEL") {
        if !m.is_empty() {
            cmd.arg("--model").arg(m);
        }
    }
    let mut ch = cmd
        .spawn()
        .map_err(|e| format!("claude を起こせません: {e}"))?;
    if let Some(si) = ch.stdin.as_mut() {
        si.write_all(user.as_bytes())
            .map_err(|e| format!("claude に渡せません: {e}"))?;
    }
    let out = ch
        .wait_with_output()
        .map_err(|e| format!("claude が答えません: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || text.is_empty() {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("答えが空です");
        return Err(format!("claude: {last}"));
    }
    Ok(text)
}

/// Anthropic の API に頼む(鍵は環境変数からだけ)
fn claude_api_ask(system: &str, user: &str) -> Result<String, String> {
    let key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| "鍵がありません")?;
    let m = std::env::var("JO_AI_MODEL").unwrap_or_else(|_| "claude-sonnet-5".into());
    let body = format!(
        r#"{{"model":"{}","max_tokens":4096,"system":"{}","messages":[{{"role":"user","content":"{}"}}]}}"#,
        model::esc(&m),
        model::esc(system),
        model::esc(user)
    );
    let raw = model::post_https(
        "api.anthropic.com",
        "/v1/messages",
        &body,
        &[
            ("x-api-key", key.as_str()),
            ("anthropic-version", "2023-06-01"),
        ],
    )?;
    // {"content":[{"type":"text","text":"…"}], …}
    model::extract_text_field(&raw, "text")
        .ok_or_else(|| format!("答えが読めません: {}", raw.chars().take(200).collect::<String>()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 宛先は覚えた形で戻る() {
        for b in [Backend::Local, Backend::ClaudeCli, Backend::ClaudeApi] {
            assert_eq!(Backend::from_str(b.as_str()), b);
        }
        // 知らない字はローカル(いちばん安全な側)へ倒す
        assert_eq!(Backend::from_str("なにこれ"), Backend::Local);
    }

    #[test]
    fn 宛先は順に回る() {
        let mut b = Backend::Local;
        for _ in 0..3 {
            b = b.next();
        }
        assert_eq!(b, Backend::Local, "3回で一周しない");
    }

    #[test]
    fn 使えない宛先は理由を言う() {
        // 鍵が無い環境では API は理由つきで断る(黙って空にしない)
        if std::env::var("ANTHROPIC_API_KEY").is_err() {
            let e = ready(Backend::ClaudeApi).unwrap_err();
            assert!(e.contains("ANTHROPIC_API_KEY"), "理由が薄い: {e}");
        }
        if claude_cli().is_none() {
            let e = ready(Backend::ClaudeCli).unwrap_err();
            assert!(e.contains("claude"), "理由が薄い: {e}");
        }
        assert!(ready(Backend::Local).is_ok(), "手元はいつでも頼める");
    }
}

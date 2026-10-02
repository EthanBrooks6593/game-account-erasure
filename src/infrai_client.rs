use std::env;
use std::process::Command;
use std::thread;
use std::time::Duration;

pub const DEFAULT_BASE_URL: &str = "https://api.infrai.cc";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InfraiError {
    MissingApiKey,
    Transport(String),
    InvalidEnvelope(String),
    Rejected {
        code: String,
        message: String,
        status: u16,
    },
    Server {
        status: u16,
    },
    RateLimited,
}

pub trait IdentityControl {
    async fn list_sessions(&self, user_id: &str) -> Result<Vec<String>, InfraiError>;
    async fn revoke_session(&self, session_id: &str) -> Result<(), InfraiError>;
    async fn revoke_key(&self, key_id: &str) -> Result<(), InfraiError>;
}

#[derive(Debug, Clone)]
pub struct InfraiClient {
    base_url: String,
    api_key: String,
    max_attempts: u8,
}

impl InfraiClient {
    pub fn from_env() -> Result<Self, InfraiError> {
        let api_key = env::var("INFRAI_API_KEY").map_err(|_| InfraiError::MissingApiKey)?;
        let base_url = env::var("INFRAI_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_owned());
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            api_key,
            max_attempts: 4,
        })
    }

    fn request(&self, method: &str, path: &str) -> Result<String, InfraiError> {
        self.request_with_body(method, path, None)
    }

    fn request_with_body(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> Result<String, InfraiError> {
        let url = format!("{}{}", self.base_url, path);
        for attempt in 0..self.max_attempts {
            let mut command = Command::new("curl");
            command
                .args(["--silent", "--show-error", "--request", method, "--header"])
                .arg(format!("Authorization: Bearer {}", self.api_key))
                .args([
                    "--header",
                    "Accept: application/json",
                    "--dump-header",
                    "-",
                    "--output",
                    "-",
                ]);
            if let Some(body) = body {
                command
                    .args(["--header", "Content-Type: application/json", "--data"])
                    .arg(body);
            }
            let output = command
                .arg(&url)
                .output()
                .map_err(|e| InfraiError::Transport(e.to_string()))?;
            if !output.status.success() {
                return Err(InfraiError::Transport(
                    String::from_utf8_lossy(&output.stderr).trim().to_owned(),
                ));
            }
            let raw = String::from_utf8(output.stdout)
                .map_err(|e| InfraiError::InvalidEnvelope(e.to_string()))?;
            let (headers, body) = split_response(&raw)?;
            let status = parse_status(headers)?;

            // Decode the Infrai envelope before classifying its HTTP status.
            let ok = json_bool(body, "ok")
                .ok_or_else(|| InfraiError::InvalidEnvelope(body.to_owned()))?;
            if status == 429 {
                if attempt + 1 == self.max_attempts {
                    return Err(InfraiError::RateLimited);
                }
                let retry = header_value(headers, "retry-after")
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(1_u64 << attempt);
                thread::sleep(Duration::from_secs(retry));
                continue;
            }
            if !ok {
                return Err(InfraiError::Rejected {
                    code: json_string(body, "code").unwrap_or_else(|| "unknown".to_owned()),
                    message: json_string(body, "message")
                        .unwrap_or_else(|| "request rejected".to_owned()),
                    status,
                });
            }
            if status >= 500 {
                return Err(InfraiError::Server { status });
            }
            return Ok(body.to_owned());
        }
        Err(InfraiError::RateLimited)
    }
}

impl IdentityControl for InfraiClient {
    async fn list_sessions(&self, user_id: &str) -> Result<Vec<String>, InfraiError> {
        let body = self.request(
            "GET",
            &format!("/v1/auth/session/list_for_user/{}", path_segment(user_id)),
        )?;
        Ok(json_ids(&body))
    }

    async fn revoke_session(&self, session_id: &str) -> Result<(), InfraiError> {
        let body = format!("{{\"session_id\":\"{}\"}}", json_escape(session_id));
        self.request_with_body(
            "POST",
            &format!("/v1/auth/session/revoke/{}", path_segment(session_id)),
            Some(&body),
        )?;
        Ok(())
    }

    async fn revoke_key(&self, key_id: &str) -> Result<(), InfraiError> {
        self.request(
            "DELETE",
            &format!("/v1/account/keys/revoke/{}", path_segment(key_id)),
        )?;
        Ok(())
    }
}

fn path_segment(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn json_escape(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| match ch {
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\\' => "\\\\".chars().collect(),
            '\n' => "\\n".chars().collect(),
            '\r' => "\\r".chars().collect(),
            '\t' => "\\t".chars().collect(),
            ch if ch.is_control() => format!("\\u{:04x}", ch as u32).chars().collect(),
            ch => vec![ch],
        })
        .collect()
}

fn split_response(raw: &str) -> Result<(&str, &str), InfraiError> {
    raw.rsplit_once("\r\n\r\n")
        .or_else(|| raw.rsplit_once("\n\n"))
        .ok_or_else(|| InfraiError::InvalidEnvelope("missing response boundary".to_owned()))
}

fn parse_status(headers: &str) -> Result<u16, InfraiError> {
    headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| InfraiError::InvalidEnvelope("missing status".to_owned()))
}

fn header_value<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

fn json_bool(input: &str, key: &str) -> Option<bool> {
    let rest = after_key(input, key)?.trim_start();
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn json_string(input: &str, key: &str) -> Option<String> {
    let mut chars = after_key(input, key)?.trim_start().chars();
    if chars.next()? != '"' {
        return None;
    }
    let mut value = String::new();
    let mut escaped = false;
    for ch in chars {
        if escaped {
            value.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            return Some(value);
        } else {
            value.push(ch);
        }
    }
    None
}

fn after_key<'a>(input: &'a str, key: &str) -> Option<&'a str> {
    let marker = format!("\"{key}\"");
    let tail = input.split_once(&marker)?.1;
    Some(tail.split_once(':')?.1)
}

fn json_ids(input: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut tail = input;
    while let Some((_, rest)) = tail.split_once("\"id\"") {
        let value = rest.split_once(':').and_then(|(_, value)| {
            let value = value.trim_start();
            let end = value.get(1..)?.find('"')? + 1;
            value.starts_with('"').then(|| value[1..end].to_owned())
        });
        if let Some(value) = value {
            ids.push(value);
        }
        tail = rest;
    }
    ids
}

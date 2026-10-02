use game_account_erasure::erasure::{erase_player, ErasureError, PlayerRecord};
use game_account_erasure::infrai_client::{InfraiClient, InfraiError};
use std::future::Future;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

fn main() {
    let client = InfraiClient::from_env().unwrap_or_else(|e| {
        eprintln!("configuration error: {e:?}");
        std::process::exit(2);
    });
    let listener = TcpListener::bind("127.0.0.1:8080").expect("bind service");
    println!("listening on http://127.0.0.1:8080");
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => handle(stream, &client),
            Err(error) => eprintln!("connection error: {error}"),
        }
    }
}

fn handle(mut stream: TcpStream, client: &InfraiClient) {
    let mut buffer = [0_u8; 8192];
    let size = match stream.read(&mut buffer) {
        Ok(size) => size,
        Err(_) => return,
    };
    let request = String::from_utf8_lossy(&buffer[..size]);
    let first = request.lines().next().unwrap_or_default();
    let Some(path) = first
        .strip_prefix("POST ")
        .and_then(|line| line.split_whitespace().next())
    else {
        respond(&mut stream, 404, "{\"error\":\"route not found\"}");
        return;
    };
    let Some(user_id) = path
        .strip_prefix("/players/")
        .and_then(|p| p.strip_suffix("/gdpr-delete"))
    else {
        respond(&mut stream, 404, "{\"error\":\"route not found\"}");
        return;
    };
    let Some(body) = request.split("\r\n\r\n").nth(1) else {
        respond(&mut stream, 400, "{\"error\":\"request body required\"}");
        return;
    };
    let Some(credential_id) = json_string(body, "credential_id") else {
        respond(&mut stream, 400, "{\"error\":\"credential_id required\"}");
        return;
    };
    let mut player = PlayerRecord {
        user_id: user_id.to_owned(),
        credential_id,
        assets: json_array(body, "assets"),
        live_events: json_array(body, "live_events"),
        moderation_queue_entries: json_array(body, "moderation_queue_entries"),
        deleted: false,
    };
    match block_on(erase_player(client, &mut player)) {
        Ok(r) => respond(&mut stream, 200, &format!(
            "{{\"user_id\":\"{}\",\"sessions_revoked\":{},\"assets_removed\":{},\"live_events_removed\":{},\"moderation_entries_removed\":{},\"credential_revoked\":true}}",
            escape(&r.user_id), r.sessions_revoked, r.assets_removed, r.live_events_removed, r.moderation_entries_removed)),
        Err(error) => {
            let (status, message) = map_error(&error);
            respond(&mut stream, status, &format!("{{\"error\":\"{}\"}}", escape(message)));
        }
    }
}

fn map_error(error: &ErasureError) -> (u16, &str) {
    match error {
        ErasureError::AlreadyDeleted => (409, "player already deleted"),
        ErasureError::Infrai(InfraiError::Rejected {
            status, message, ..
        }) => (*status, message),
        ErasureError::Infrai(InfraiError::RateLimited) => (503, "request can be retried"),
        ErasureError::Infrai(_) => (502, "identity control request failed"),
    }
}

fn respond(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        409 => "Conflict",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Request Failed",
    };
    let response = format!("HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = stream.write_all(response.as_bytes());
}

fn json_string(input: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\"");
    let tail = input.split_once(&marker)?.1.split_once(':')?.1.trim_start();
    let end = tail[1..].find('"')? + 1;
    tail.starts_with('"').then(|| tail[1..end].to_owned())
}

fn json_array(input: &str, key: &str) -> Vec<String> {
    let marker = format!("\"{key}\"");
    let Some(tail) = input
        .split_once(&marker)
        .and_then(|(_, t)| t.split_once(':').map(|(_, v)| v.trim_start()))
    else {
        return Vec::new();
    };
    let Some(end) = tail.find(']') else {
        return Vec::new();
    };
    tail[..=end]
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

struct NoopWake;
impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}
fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    loop {
        if let Poll::Ready(value) = Pin::new(&mut future).poll(&mut context) {
            return value;
        }
    }
}

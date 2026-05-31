use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::app::{AppState, PetMood};
use crate::hooks::process_hook;
use crate::util::ConnectionLimiter;

const MAX_HOOK_CONNECTIONS: usize = 16;
const MAX_HOOK_REQUEST_BYTES: usize = 10 * 1024 * 1024;

pub(crate) fn start_hook_server(state: Arc<Mutex<AppState>>, port: u16) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .map_err(|err| format!("Hook server failed: {err}"))?;

    thread::spawn(move || {
        let limiter = ConnectionLimiter::new(MAX_HOOK_CONNECTIONS);
        for stream in listener.incoming() {
            match stream {
                Ok(mut stream) => {
                    let Some(permit) = limiter.try_acquire() else {
                        let _ = write_http_response(
                            &mut stream,
                            503,
                            json!({ "error": "claudie hook server is busy" }).to_string(),
                        );
                        continue;
                    };
                    let state = state.clone();
                    thread::spawn(move || {
                        let _permit = permit;
                        handle_client(stream, state);
                    });
                }
                Err(err) => {
                    let mut state = state.lock().expect("state poisoned");
                    state.last_error = format!("Hook accept failed: {err}");
                    state.set_mood(PetMood::Error);
                }
            }
        }
    });

    Ok(())
}

fn handle_client(mut stream: TcpStream, state: Arc<Mutex<AppState>>) {
    let request = match read_http_request(&mut stream) {
        Ok(request) => request,
        Err(err) => {
            let _ = write_http_response(&mut stream, 400, json!({ "error": err }).to_string());
            return;
        }
    };

    if request.method != "POST" || request.path != "/hook" {
        let _ = write_http_response(
            &mut stream,
            404,
            json!({ "error": "claudie only accepts POST /hook" }).to_string(),
        );
        return;
    }

    let payload: Value = match serde_json::from_slice(&request.body) {
        Ok(value) => value,
        Err(err) => {
            let _ = write_http_response(
                &mut stream,
                400,
                json!({ "error": err.to_string() }).to_string(),
            );
            return;
        }
    };

    let response = process_hook(payload, state.clone());
    let _ = write_http_response(&mut stream, 200, response.to_string());
    flush_stats_if_due(&state);
}

fn flush_stats_if_due(state: &Arc<Mutex<AppState>>) {
    let mut state = state.lock().expect("state poisoned");
    if let Err(err) = state.flush_stats_if_due() {
        state.last_error = format!("Stats save failed: {err}");
        state.set_mood(PetMood::Error);
    }
}

struct HttpRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|err| err.to_string())?;

    let mut buffer = Vec::with_capacity(8192);
    let mut temp = [0_u8; 2048];
    let header_end;

    loop {
        let count = stream.read(&mut temp).map_err(|err| err.to_string())?;
        if count == 0 {
            return Err("connection closed".to_string());
        }
        buffer.extend_from_slice(&temp[..count]);
        if let Some(pos) = find_header_end(&buffer) {
            header_end = pos;
            break;
        }
        if buffer.len() > 64 * 1024 {
            return Err("request header too large".to_string());
        }
    }

    let header = String::from_utf8_lossy(&buffer[..header_end]);
    let mut lines = header.lines();
    let request_line = lines.next().ok_or_else(|| "empty request".to_string())?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();

    let mut content_length = 0_usize;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value
                    .trim()
                    .parse::<usize>()
                    .map_err(|err| err.to_string())?;
            }
        }
    }
    if content_length > MAX_HOOK_REQUEST_BYTES {
        return Err("request body too large".to_string());
    }

    let body_start = header_end + 4;
    let mut body = buffer[body_start..].to_vec();
    while body.len() < content_length {
        let remaining = content_length - body.len();
        let chunk_len = remaining.min(temp.len());
        let count = stream
            .read(&mut temp[..chunk_len])
            .map_err(|err| err.to_string())?;
        if count == 0 {
            return Err("connection closed before body completed".to_string());
        }
        body.extend_from_slice(&temp[..count]);
    }
    body.truncate(content_length);

    Ok(HttpRequest { method, path, body })
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn write_http_response(stream: &mut TcpStream, status: u16, body: String) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        503 => "Service Unavailable",
        _ => "OK",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.as_bytes().len(),
        body
    );
    stream.write_all(response.as_bytes())
}

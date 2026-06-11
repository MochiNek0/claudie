use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::Value;

const MAX_PROXY_REQUEST_BYTES: usize = 10 * 1024 * 1024;

pub(super) struct HttpRequest {
    pub(super) method: String,
    pub(super) path: String,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
}

impl HttpRequest {
    pub(super) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

pub(super) fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .map_err(|err| err.to_string())?;

    let mut buffer = Vec::with_capacity(8192);
    let mut temp = [0_u8; 4096];
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
    let mut headers = Vec::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            content_length = value
                .trim()
                .parse::<usize>()
                .map_err(|err| err.to_string())?;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_string(), value.trim().to_string()));
        }
    }
    if content_length > MAX_PROXY_REQUEST_BYTES {
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

    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

pub(super) fn write_json_response(
    stream: &mut TcpStream,
    status: u16,
    body: Value,
) -> std::io::Result<()> {
    write_json_response_with_headers(stream, status, &[], body)
}

pub(super) fn write_json_response_with_headers(
    stream: &mut TcpStream,
    status: u16,
    extra_headers: &[(&str, &str)],
    body: Value,
) -> std::io::Result<()> {
    let response = http_response_text(status, "application/json", extra_headers, &body.to_string());
    stream.write_all(response.as_bytes())
}

fn http_response_text(
    status: u16,
    content_type: &str,
    extra_headers: &[(&str, &str)],
    body: &str,
) -> String {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        529 => "Overloaded",
        _ => "OK",
    };
    let mut response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in extra_headers {
        response.push_str(name);
        response.push_str(": ");
        response.push_str(value);
        response.push_str("\r\n");
    }
    response.push_str("Connection: close\r\n\r\n");
    response.push_str(body);
    response
}

pub(super) fn shorten_for_error(text: &str) -> String {
    let mut shortened = text.trim().replace(['\r', '\n'], " ");
    if shortened.len() > 500 {
        shortened.truncate(500);
        shortened.push_str("...");
    }
    shortened
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_text_includes_extra_headers_and_reason() {
        let response = http_response_text(429, "application/json", &[("Retry-After", "17")], "{}");
        assert!(response.starts_with("HTTP/1.1 429 Too Many Requests\r\n"));
        assert!(response.contains("Retry-After: 17\r\n"));
        assert!(response.ends_with("\r\n\r\n{}"));
    }

    #[test]
    fn response_text_without_extra_headers_matches_legacy_shape() {
        let response = http_response_text(200, "application/json", &[], "{}");
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.contains("Connection: close\r\n\r\n{}"));
        assert!(!response.contains("Retry-After"));
    }
}

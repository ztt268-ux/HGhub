//! iOS 本机播放服务。
//!
//! iOS 15 的 WKWebView 没有 MediaSource，系统播放器只接受 http/https 的 HLS。
//! `hg://localhost` 能把播放列表交给自定义协议，但 AVPlayer 会以
//! `MEDIA_ERR_SRC_NOT_SUPPORTED` 拒绝。这里只在 127.0.0.1 上提供同一套
//! `/hls`、`/seg`、`/img`，并把 m3u8 里的分片改写到这个地址。

use crate::media;
use crate::playback_log;
use serde_json::json;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub const PORT: u16 = 17831;
pub const ORIGIN: &str = "http://127.0.0.1:17831";

pub fn start() {
    let listener = match std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, PORT)) {
        Ok(listener) => listener,
        Err(error) => {
            playback_log::record(
                "local_media.bind_failed",
                json!({ "error_kind": error.kind().to_string() }),
            );
            return;
        }
    };
    if let Err(error) = listener.set_nonblocking(true) {
        playback_log::record(
            "local_media.bind_failed",
            json!({ "error_kind": error.kind().to_string() }),
        );
        return;
    }
    tauri::async_runtime::spawn(async move {
        let listener = match TcpListener::from_std(listener) {
            Ok(listener) => listener,
            Err(_) => return,
        };
        loop {
            match listener.accept().await {
                Ok((socket, _)) => {
                    tauri::async_runtime::spawn(async move {
                        let _ = serve(socket).await;
                    });
                }
                Err(_) => break,
            }
        }
    });
}

async fn serve(mut socket: TcpStream) -> std::io::Result<()> {
    let _ = socket.set_nodelay(true);
    let header = match tokio::time::timeout(Duration::from_secs(8), read_headers(&mut socket)).await {
        Ok(Ok(header)) => header,
        _ => return Ok(()),
    };
    let header = match header.split_once("\r\n\r\n") {
        Some((head, _)) => head.to_string(),
        None => return Ok(()),
    };
    let Some(request) = header.split("\r\n").next() else { return Ok(()) };
    let mut parts = request.split_whitespace();
    let Some(method) = parts.next() else { return Ok(()) };
    let Some(target) = parts.next() else { return Ok(()) };
    if !matches!(method, "GET" | "HEAD" | "OPTIONS") {
        return write_response(&mut socket, 405, "text/plain", b"method not allowed", None).await;
    }
    let range = range_header(header);
    if method == "OPTIONS" {
        return write_response(&mut socket, 204, "text/plain", b"", None).await;
    }
    let Some((path, query)) = split_target(target) else {
        return write_response(&mut socket, 404, "text/plain", b"not found", None).await;
    };
    if playback_log::enabled() {
        let resource = url::form_urlencoded::parse(query.as_bytes())
            .find(|(key, _)| key == "u")
            .map(|(_, value)| playback_log::resource_id(&value));
        let route = match path.as_str() {
            "/hls" => "playlist",
            "/seg" => "segment_or_key",
            "/img" => "image",
            _ => "unknown",
        };
        playback_log::record(
            "protocol.request",
            json!({ "resource": resource, "route": route, "method": method, "range": range, "origin_kind": "loopback" }),
        );
    }
    let response = media::handle(&path, &query, ORIGIN).await;
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let body = response.into_body();
    if method == "HEAD" {
        return write_response(&mut socket, status, &content_type, b"", Some(body.len())).await;
    }
    if status != 200 {
        return write_response(&mut socket, status, &content_type, &body, None).await;
    }
    if let Some(spec) = range {
        match byte_range(spec, body.len()) {
            Some((start, end)) => {
                let slice = &body[start..=end];
                return write_ranged(&mut socket, &content_type, slice, start, end, body.len()).await;
            }
            None => return write_unsatisfiable(&mut socket, body.len()).await,
        }
    }
    write_response(&mut socket, 200, &content_type, &body, None).await
}

fn range_header(headers: &str) -> Option<&str> {
    for line in headers.split("\r\n").skip(1) {
        let Some((name, value)) = line.split_once(':') else { continue };
        if name.eq_ignore_ascii_case("range") {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

fn split_target(target: &str) -> Option<(String, String)> {
    let target = target.split('#').next().unwrap_or(target);
    if target.starts_with("//") || target.contains("://") {
        return None;
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if !matches!(path, "/hls" | "/seg" | "/img") {
        return None;
    }
    Some((path.to_string(), query.to_string()))
}

async fn read_headers(socket: &mut TcpStream) -> std::io::Result<String> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        let read = socket.read(&mut tmp).await?;
        if read == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..read]);
        if buf.windows(4).any(|window| window == b"\r\n\r\n") || buf.len() > 16 * 1024 {
            break;
        }
    }
    String::from_utf8(buf).map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

fn byte_range(header: &str, len: usize) -> Option<(usize, usize)> {
    if len == 0 || header.contains(',') {
        return None;
    }
    let value = header.trim().strip_prefix("bytes=")?.trim();
    let (start, end) = value.split_once('-')?;
    if start.is_empty() {
        let suffix = end.parse::<usize>().ok()?;
        if suffix == 0 {
            return None;
        }
        let start = len.saturating_sub(suffix);
        return Some((start, len - 1));
    }
    let start = start.parse::<usize>().ok()?;
    if start >= len {
        return None;
    }
    let end = if end.is_empty() {
        len - 1
    } else {
        end.parse::<usize>().ok()?.min(len - 1)
    };
    if end < start {
        return None;
    }
    Some((start, end))
}

async fn write_response(socket: &mut TcpStream, status: u16, content_type: &str, body: &[u8], content_length: Option<usize>) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        416 => "Range Not Satisfiable",
        502 => "Bad Gateway",
        _ => "Error",
    };
    let length = content_length.unwrap_or(body.len());
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {length}\r\nAccept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, HEAD, OPTIONS\r\nAccess-Control-Allow-Headers: Range\r\nAccess-Control-Expose-Headers: Content-Length, Content-Type, Content-Range, Accept-Ranges\r\nConnection: close\r\n\r\n"
    );
    socket.write_all(head.as_bytes()).await?;
    if status != 204 {
        socket.write_all(body).await?;
    }
    socket.flush().await
}

async fn write_ranged(socket: &mut TcpStream, content_type: &str, body: &[u8], start: usize, end: usize, total: usize) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 206 Partial Content\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{end}/{total}\r\nAccept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Expose-Headers: Content-Length, Content-Type, Content-Range, Accept-Ranges\r\nConnection: close\r\n\r\n",
        body.len()
    );
    socket.write_all(head.as_bytes()).await?;
    socket.write_all(body).await?;
    socket.flush().await
}

async fn write_unsatisfiable(socket: &mut TcpStream, total: usize) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nContent-Range: bytes */{total}\r\nConnection: close\r\n\r\n"
    );
    socket.write_all(head.as_bytes()).await?;
    socket.flush().await
}

#[cfg(test)]
mod tests {
    use super::byte_range;

    #[test]
    fn byte_ranges_cover_probe_and_suffix() {
        assert_eq!(byte_range("bytes=0-1", 100), Some((0, 1)));
        assert_eq!(byte_range("bytes=0-7", 8), Some((0, 7)));
        assert_eq!(byte_range("bytes=4-", 10), Some((4, 9)));
        assert_eq!(byte_range("bytes=-4", 10), Some((6, 9)));
        assert_eq!(byte_range("bytes=100-120", 10), None);
        assert_eq!(byte_range("bytes=0-1,2-3", 10), None);
    }
}

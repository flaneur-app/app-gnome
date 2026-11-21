// SPDX-License-Identifier: AGPL-3.0-or-later

//! A local HTTP proxy that fetches a stream in bounded chunks.
//!
//! Some CDNs only answer bounded byte ranges: YouTube's `googlevideo.com` refuses a request for
//! a whole file, or for `bytes=N-`, with 403 Forbidden, and its own clients download in chunks.
//! It also refuses ranges that run too far ahead of playback: how far grows with the time since
//! the URL was issued, in proportion to the stream's bitrate. GStreamer's HTTP source always
//! asks for everything from an offset. The proxy sits between them: GStreamer asks it for
//! `http://127.0.0.1:PORT/TOKEN` (with or without a `Range`), and it answers that range by
//! requesting the upstream URL one small chunk after the other, sending the stream's headers,
//! and passing the bytes through as GStreamer reads them. A chunk refused with 403 is asked
//! again after a short wait, as the allowance grows. Seeking works: GStreamer sends a new range
//! request (a seek far ahead waits until the CDN allows it).
//!
//! It listens on the loopback interface only, and serves only the URLs registered by the player,
//! each under an unguessable token, so it is not an open proxy.

use std::collections::VecDeque;
use std::hash::{BuildHasher, Hasher};
use std::net::SocketAddr;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::runtime;

/// Bytes asked from upstream per request.
const CHUNK: u64 = 1024 * 1024;
/// How long a refused chunk is asked again before giving up.
const RETRY_FOR: Duration = Duration::from_secs(30);
/// Longest wait between two attempts.
const MAX_RETRY_DELAY: Duration = Duration::from_secs(4);
/// Streams the proxy keeps serving (the current item's video and audio, and a few before).
const ROUTES: usize = 8;
/// Largest request head accepted from GStreamer.
const MAX_HEAD: usize = 16 * 1024;

struct Route {
    token: String,
    url: String,
    headers: Vec<(String, String)>,
}

struct Proxy {
    addr: SocketAddr,
    routes: Mutex<VecDeque<Route>>,
    client: reqwest::Client,
}

fn proxy() -> Option<&'static Proxy> {
    static PROXY: OnceLock<Option<Proxy>> = OnceLock::new();
    PROXY
        .get_or_init(|| {
            let _runtime = runtime::runtime().enter();
            let listener = std::net::TcpListener::bind("127.0.0.1:0")
                .and_then(|l| l.set_nonblocking(true).map(|()| l))
                .and_then(TcpListener::from_std)
                .inspect_err(|error| tracing::warn!(%error, "stream proxy unavailable"))
                .ok()?;
            let addr = listener.local_addr().ok()?;
            let client = reqwest::Client::builder()
                .http1_only()
                .connect_timeout(Duration::from_secs(15))
                .read_timeout(Duration::from_secs(30))
                .build()
                .ok()?;
            runtime::spawn(async move {
                loop {
                    match listener.accept().await {
                        Ok((socket, _)) => {
                            runtime::spawn(async move {
                                if let Err(error) = serve(socket).await {
                                    tracing::debug!(%error, "stream proxy connection");
                                }
                            });
                        }
                        Err(error) => tracing::warn!(%error, "stream proxy accept"),
                    }
                }
            });
            tracing::debug!(%addr, "stream proxy listening");
            Some(Proxy {
                addr,
                routes: Mutex::new(VecDeque::new()),
                client,
            })
        })
        .as_ref()
}

/// Whether a stream URL must go through the proxy.
pub fn needs_chunks(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| {
        u.host_str()
            .is_some_and(|h| h == "googlevideo.com" || h.ends_with(".googlevideo.com"))
    })
}

/// A local URL serving `url` in chunks, sending `headers` upstream; `url` itself when the proxy
/// cannot start.
pub fn register(url: &str, headers: &[(String, String)]) -> String {
    let Some(proxy) = proxy() else {
        return url.to_owned();
    };
    let state = std::collections::hash_map::RandomState::new();
    let token = format!(
        "{:016x}{:016x}",
        state.build_hasher().finish(),
        state.hash_one(url)
    );
    tracing::trace!(%url, %token, "stream proxy route");
    let mut routes = proxy.routes.lock().unwrap_or_else(PoisonError::into_inner);
    routes.push_back(Route {
        token: token.clone(),
        url: url.to_owned(),
        headers: headers.to_vec(),
    });
    while routes.len() > ROUTES {
        routes.pop_front();
    }
    format!("http://{}/{token}", proxy.addr)
}

/// A request from GStreamer: the token and the byte range.
#[derive(Debug, PartialEq, Eq)]
struct Request {
    head_only: bool,
    token: String,
    /// First byte, and last byte when bounded.
    range: Option<(u64, Option<u64>)>,
}

fn parse_request(head: &str) -> Option<Request> {
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let head_only = match first.next()? {
        "GET" => false,
        "HEAD" => true,
        _ => return None,
    };
    let token = first.next()?.trim_start_matches('/').to_owned();
    let range = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("range"))
        .and_then(|(_, value)| parse_range(value.trim()));
    Some(Request {
        head_only,
        token,
        range,
    })
}

fn parse_range(value: &str) -> Option<(u64, Option<u64>)> {
    let (start, end) = value.strip_prefix("bytes=")?.split_once('-')?;
    let start = start.trim().parse().ok()?;
    let end = match end.trim() {
        "" => None,
        end => Some(end.parse().ok()?),
    };
    Some((start, end))
}

/// The size of the file, when its URL says (`clen` in googlevideo URLs).
fn known_size(url: &str) -> Option<u64> {
    let url = url::Url::parse(url).ok()?;
    let size = url
        .query_pairs()
        .find(|(k, _)| k == "clen")?
        .1
        .parse()
        .ok()?;
    (size > 0).then_some(size)
}

/// The total size in a `Content-Range: bytes A-B/TOTAL` header.
fn total_size(content_range: &str) -> Option<u64> {
    content_range.rsplit_once('/')?.1.trim().parse().ok()
}

/// Reads a request head; `None` when the client went away or sent too much.
async fn read_head(socket: &mut TcpStream) -> std::io::Result<Option<String>> {
    let mut head = Vec::new();
    let mut buf = [0; 2048];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        let read = socket.read(&mut buf).await?;
        if read == 0 || head.len() > MAX_HEAD {
            return Ok(None);
        }
        head.extend_from_slice(&buf[..read]);
    }
    Ok(Some(String::from_utf8_lossy(&head).into_owned()))
}

async fn serve(mut socket: TcpStream) -> std::io::Result<()> {
    let Some(head) = read_head(&mut socket).await? else {
        return Ok(());
    };
    let Some(request) = parse_request(&head) else {
        return reply_status(&mut socket, "400 Bad Request").await;
    };
    let Some(proxy) = proxy() else {
        return Ok(());
    };
    let route = {
        let routes = proxy.routes.lock().unwrap_or_else(PoisonError::into_inner);
        routes
            .iter()
            .find(|r| r.token == request.token)
            .map(|r| (r.url.clone(), r.headers.clone()))
    };
    let Some((url, headers)) = route else {
        return reply_status(&mut socket, "404 Not Found").await;
    };

    let (start, wanted_end) = request.range.unwrap_or((0, None));
    // googlevideo answers 403, not 416, to ranges past the end; its URLs carry the size.
    let known_size = known_size(&url);
    if known_size.is_some_and(|size| start >= size) {
        return reply_status(&mut socket, "416 Range Not Satisfiable").await;
    }
    let last_byte = known_size.map_or(u64::MAX, |size| size - 1);
    let fetch = |from: u64, to: u64| fetch_chunk(&proxy.client, &url, &headers, from, to);

    let first_end = wanted_end
        .unwrap_or(u64::MAX)
        .min(start + CHUNK - 1)
        .min(last_byte);
    let mut response = match fetch(start, first_end).await {
        Ok(response) if response.status() == reqwest::StatusCode::PARTIAL_CONTENT => response,
        Ok(response) if response.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE => {
            return reply_status(&mut socket, "416 Range Not Satisfiable").await;
        }
        Ok(response) => {
            tracing::warn!(
                status = %response.status(),
                start,
                first_end,
                "stream proxy: upstream refused"
            );
            return reply_status(&mut socket, "502 Bad Gateway").await;
        }
        Err(error) => {
            tracing::warn!(%error, "stream proxy: upstream failed");
            return reply_status(&mut socket, "502 Bad Gateway").await;
        }
    };
    let header = |name| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let Some(total) = header(reqwest::header::CONTENT_RANGE).and_then(|r| total_size(&r)) else {
        return reply_status(&mut socket, "502 Bad Gateway").await;
    };
    let content_type =
        header(reqwest::header::CONTENT_TYPE).unwrap_or_else(|| "application/octet-stream".into());
    let end = wanted_end.unwrap_or(u64::MAX).min(total.saturating_sub(1));
    let length = end + 1 - start;
    let status = if request.range.is_some() {
        format!("206 Partial Content\r\nContent-Range: bytes {start}-{end}/{total}")
    } else {
        "200 OK".to_owned()
    };
    let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {length}\r\n\
         Accept-Ranges: bytes\r\nConnection: close\r\n\r\n"
    );
    socket.write_all(reply.as_bytes()).await?;
    if request.head_only {
        return Ok(());
    }

    let mut position = start;
    loop {
        while let Some(bytes) = response
            .chunk()
            .await
            .map_err(|e| std::io::Error::other(e.to_string()))?
        {
            socket.write_all(&bytes).await?;
            position += bytes.len() as u64;
        }
        if position > end {
            return Ok(());
        }
        let to = (position + CHUNK - 1).min(end);
        response = fetch(position, to)
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
    }
}

/// Asks upstream for one chunk; a 403 is asked again, waiting longer each time, for up to
/// [`RETRY_FOR`].
async fn fetch_chunk(
    client: &reqwest::Client,
    url: &str,
    headers: &[(String, String)],
    from: u64,
    to: u64,
) -> reqwest::Result<reqwest::Response> {
    let started = std::time::Instant::now();
    let mut delay = Duration::from_millis(250);
    loop {
        let mut request = client
            .get(url)
            .header(reqwest::header::RANGE, format!("bytes={from}-{to}"));
        for (name, value) in headers {
            request = request.header(name.as_str(), value.as_str());
        }
        let response = request.send().await?;
        if response.status() != reqwest::StatusCode::FORBIDDEN || started.elapsed() >= RETRY_FOR {
            return Ok(response);
        }
        tracing::debug!(from, ?delay, "stream proxy: chunk refused, waiting");
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(MAX_RETRY_DELAY);
    }
}

async fn reply_status(socket: &mut TcpStream, status: &str) -> std::io::Result<()> {
    let reply = format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    socket.write_all(reply.as_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_token_and_the_range() {
        let head = "GET /abc HTTP/1.1\r\nHost: 127.0.0.1\r\nrange: bytes=100-\r\n\r\n";
        assert_eq!(
            parse_request(head),
            Some(Request {
                head_only: false,
                token: "abc".into(),
                range: Some((100, None)),
            })
        );
        let head = "HEAD /t HTTP/1.1\r\nRange: bytes=0-99\r\n\r\n";
        assert_eq!(parse_request(head).unwrap().range, Some((0, Some(99))));
        assert_eq!(
            parse_request("GET /t HTTP/1.1\r\n\r\n").unwrap().range,
            None
        );
        assert!(parse_request("POST /t HTTP/1.1\r\n\r\n").is_none());
    }

    #[test]
    fn reads_the_size_from_the_url() {
        assert_eq!(
            known_size("https://x.googlevideo.com/videoplayback?a=1&clen=1234&b=2"),
            Some(1234)
        );
        assert_eq!(known_size("https://x.googlevideo.com/videoplayback"), None);
    }

    #[test]
    fn reads_the_total_size() {
        assert_eq!(total_size("bytes 0-99/73530647"), Some(73_530_647));
        assert_eq!(total_size("bytes 0-99/*"), None);
    }

    #[test]
    fn only_googlevideo_needs_chunks() {
        assert!(needs_chunks(
            "https://rr1---sn-x.googlevideo.com/videoplayback?x=1"
        ));
        assert!(!needs_chunks(
            "https://player.odycdn.com/v6/streams/a/b.mp4"
        ));
        assert!(!needs_chunks("https://evilgooglevideo.com/x"));
    }
}

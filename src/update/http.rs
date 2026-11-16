//! Bounded curl transport. Headers and bodies are read incrementally; a peer
//! cannot fill memory or disk by omitting or lying about Content-Length.
use std::io::{self, BufRead, BufReader, Read, Write};
use std::process::Stdio;
use std::time::Duration;

#[derive(Debug)]
pub(super) struct Failure { pub message: String, pub retry_after: Option<Duration> }
impl Failure {
    pub fn message(message: impl Into<String>) -> Self { Self { message: message.into(), retry_after: None } }
}

impl From<String> for Failure { fn from(message: String) -> Self { Self::message(message) } }
impl From<&str> for Failure { fn from(message: &str) -> Self { Self::message(message) } }

fn status_error(status: u16, retry: Option<Duration>) -> Failure {
    let message = match status {
        404 => crate::settings::tr("尚无可用更新包，或发布资源缺失", "No update package is published, or a release asset is missing"),
        403 | 429 => crate::settings::tr("更新服务暂时拒绝请求，请稍后重试", "The update service is busy. Try again later."),
        _ => crate::settings::tr("无法连接更新服务器", "Could not reach the update server"),
    };
    Failure { message: format!("{message} (HTTP {status})"), retry_after: matches!(status, 403 | 429).then_some(retry.unwrap_or(Duration::from_secs(60))) }
}

fn retry_after(s: &str) -> Option<Duration> {
    if let Ok(n) = s.parse::<u64>() { return Some(Duration::from_secs(n)) }
    let p: Vec<_> = s.split_whitespace().collect();
    if p.len() != 6 || p[5] != "GMT" { return None }
    let month = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"].iter().position(|m| *m == p[2])? as u32 + 1;
    let day: u32 = p[1].parse().ok()?;
    let year: i64 = p[3].parse().ok()?;
    let t: Vec<u32> = p[4].split(':').map(str::parse).collect::<Result<_, _>>().ok()?;
    if !(1970..=9999).contains(&year) || day == 0 || day > crate::time::days_in_month(year, month) || t.len() != 3 || t[0] > 23 || t[1] > 59 || t[2] > 59 { return None }
    let when = crate::time::days_from_civil(year, month, day) as u64 * 86400 + (t[0] * 3600 + t[1] * 60 + t[2]) as u64;
    Some(Duration::from_secs(when.saturating_sub(crate::time::now())))
}

fn headers(input: &mut impl BufRead) -> Result<(), Failure> {
    let mut total = 0;
    loop {
        let mut status = None;
        let mut retry = None;
        loop {
            let mut line = vec![];
            input.take(8193).read_until(b'\n', &mut line).map_err(|e| Failure::message(e.to_string()))?;
            total += line.len();
            if line.is_empty() || line.len() > 8192 || total > 65536 { return Err(Failure::message("Invalid update HTTP headers")) }
            let line = std::str::from_utf8(&line).map_err(|_| Failure::message("Invalid update HTTP headers"))?.trim_end();
            if status.is_none() {
                status = line.strip_prefix("HTTP/").and_then(|s| s.split_whitespace().nth(1)).and_then(|s| s.parse::<u16>().ok());
                if status.is_none() { return Err(Failure::message("Invalid update HTTP status")) }
            } else if line.is_empty() { break }
            else if let Some((name, value)) = line.split_once(':') && name.eq_ignore_ascii_case("retry-after") { retry = retry_after(value.trim()); }
        }
        let status = status.unwrap();
        if (100..200).contains(&status) || matches!(status, 301 | 302 | 303 | 307 | 308) { continue }
        return if status == 200 { Ok(()) } else { Err(status_error(status, retry)) }
    }
}

fn copy_bounded(input: &mut impl Read, out: &mut impl Write, limit: u64, mut progress: impl FnMut(u64)) -> io::Result<()> {
    let mut got = 0;
    let mut buf = [0; 16384];
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 { return Ok(()) }
        if n as u64 > limit - got { return Err(io::Error::other("Update response exceeds its size limit")) }
        out.write_all(&buf[..n])?;
        got += n as u64;
        progress(got);
    }
}

pub(super) fn get(url: &str, limit: u64, seconds: u64, testing: bool, out: &mut impl Write, progress: impl FnMut(u64)) -> Result<(), Failure> {
    let proto = if testing { "=https,http" } else { "=https" };
    let mut child = super::command("curl").args(["-q", "-sSL", "--include", "--suppress-connect-headers", "--max-redirs", "10", "--proto", proto, "--proto-redir", proto,
        "--connect-timeout", "15", "--max-time", &seconds.to_string(), "--user-agent", concat!("quiet-md/", env!("CARGO_PKG_VERSION")), "--url", url])
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| Failure::message(format!("curl: {e}")))?;
    let mut err = child.stderr.take().unwrap();
    let stderr = std::thread::spawn(move || {
        let mut bytes = vec![];
        let _ = (&mut err).take(8192).read_to_end(&mut bytes);
        let _ = io::copy(&mut err, &mut io::sink());
        bytes
    });
    let mut input = BufReader::new(child.stdout.take().unwrap());
    let result = headers(&mut input).and_then(|()| copy_bounded(&mut input, out, limit, progress).map_err(|e| Failure::message(e.to_string())));
    if result.is_err() { let _ = child.kill(); }
    let status = child.wait();
    let diagnostic = stderr.join().unwrap_or_default();
    // A transport failure may have occurred before any response headers arrived.
    if result.as_ref().is_err_and(|e| e.retry_after.is_some() || e.message.contains("HTTP ")) { return result }
    if !status.is_ok_and(|s| s.success()) && !diagnostic.is_empty() {
        return Err(Failure::message(format!("{}: {}", crate::settings::tr("无法连接更新服务器", "Could not reach the update server"), String::from_utf8_lossy(&diagnostic).trim())));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn serve(response: Vec<u8>, testing: bool, limit: u64) -> (Result<(), Failure>, Vec<u8>) {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/update.json", server.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            server.set_nonblocking(true).unwrap();
            for _ in 0..100 {
                if let Ok((mut stream, _)) = server.accept() {
                    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                    let _ = stream.read(&mut [0; 4096]);
                    let _ = stream.write_all(&response);
                    return
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let mut body = vec![];
        let result = get(&url, limit, 3, testing, &mut body, |_| {});
        thread.join().unwrap();
        (result, body)
    }

    #[test]
    fn bounds_real_bodies_and_classifies_http_failures() {
        let (result, body) = serve(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nquiet.md".to_vec(), true, 8);
        assert!(result.is_ok()); assert_eq!(body, b"quiet.md");
        let (result, body) = serve(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\ntoo many bytes".to_vec(), true, 8);
        assert!(result.is_err()); assert!(body.len() <= 8);
        let (result, _) = serve(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nshort".to_vec(), true, 100);
        assert!(result.is_err());
        for status in [404, 403, 429, 500] {
            let (result, _) = serve(format!("HTTP/1.1 {status} Error\r\nRetry-After: 120\r\nContent-Length: 0\r\n\r\n").into_bytes(), true, 64);
            let err = result.unwrap_err();
            assert!(err.message.contains(&format!("HTTP {status}")));
            assert_eq!(err.retry_after, matches!(status, 403 | 429).then_some(Duration::from_secs(120)));
        }
        assert!(serve(b"HTTP/1.1 200 OK\r\n\r\n".to_vec(), false, 64).0.is_err());
    }

    #[test]
    fn bounds_headers_and_handles_redirects_and_retry_dates() {
        assert!(headers(&mut &b"HTTP/1.1 302 Found\r\nLocation: /new\r\n\r\nHTTP/2 200\r\n\r\n"[..]).is_ok());
        assert!(headers(&mut &format!("HTTP/1.1 200 OK\r\nX-Large: {}\r\n\r\n", "a".repeat(8193)).as_bytes()[..]).is_err());
        assert_eq!(retry_after("Sun, 06 Nov 1994 08:49:37 GMT"), Some(Duration::ZERO));
        assert!(retry_after("invalid").is_none());
    }
}

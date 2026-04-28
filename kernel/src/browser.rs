use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

const MAX_RESPONSE: usize = 65536;

// ─── Public URL / search helpers ─────────────────────────────────────────────

/// Build the URL to navigate to from raw user input.
///   - Full URL         → used as-is
///   - "domain.com"     → http://domain.com
///   - "anything else"  → wiby.me search (HTTP, no HTTPS redirect)
pub fn build_navigate_url(input: &str) -> Option<String> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    if s.starts_with("http://") || s.starts_with("https://") {
        return Some(String::from(s));
    }
    // Looks like a domain (has at least one dot, no spaces)
    if !s.contains(' ') && s.contains('.') {
        return Some(format!("http://{}", s));
    }
    // Search via Wiby — a retro search engine served over plain HTTP
    Some(format!("http://wiby.me/?q={}", url_encode_query(s)))
}

/// URL-encode a query string (spaces → +, everything else → %HH).
pub fn url_encode_query(s: &str) -> String {
    const HEX: &[u8] = b"0123456789ABCDEF";
    let mut out = String::new();
    for &b in s.as_bytes() {
        match b {
            b' ' => out.push('+'),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => {
                out.push('%');
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0xf) as usize] as char);
            }
        }
    }
    out
}

// ─── Fetch + render ───────────────────────────────────────────────────────────

/// Fetch a URL (following up to 4 redirects) and render to plain text + links.
pub fn fetch_and_render(url: &str) -> Result<(String, Vec<String>), String> {
    let mut current = String::from(url);

    for _ in 0..5 {
        let raw = fetch_raw(&current)?;

        // Follow 3xx redirects — follow both HTTP and HTTPS targets as-is.
        // (We no longer convert HTTPS→HTTP, which caused an infinite loop.)
        if let Some(loc) = find_redirect_location(&raw) {
            let next = if loc.starts_with("http://") || loc.starts_with("https://") {
                loc
            } else if loc.starts_with('/') {
                format!("{}{}", base_origin(&current), loc)
            } else {
                break; // relative redirect we can't resolve
            };
            current = next;
            continue;
        }

        // Split headers from body
        let body_start = raw
            .find("\r\n\r\n")
            .map(|i| i + 4)
            .or_else(|| raw.find("\n\n").map(|i| i + 2))
            .unwrap_or(0);

        let headers = &raw[..body_start];
        let raw_body = &raw[body_start..];

        // Decode chunked transfer encoding when present
        let decoded;
        let body: &str = if is_chunked(headers) {
            decoded = decode_chunked(raw_body);
            &decoded
        } else {
            raw_body
        };

        let (_, host, _) = split_url(&current);
        return Ok(render_html(host, body));
    }

    Err(String::from("Too many redirects (site may require HTTPS)"))
}

// ─── Raw HTTP/HTTPS fetch ─────────────────────────────────────────────────────

fn fetch_raw(url: &str) -> Result<String, String> {
    let (use_https, host, path) = split_url(url);
    if use_https {
        let ip = parse_ipv4(host)
            .or_else(|| crate::drivers::network::dns_resolve_a(host).ok())
            .ok_or_else(|| format!("DNS resolve failed for {}", host))?;
        let _ = crate::drivers::network::request_arp(ip);
        let _ = crate::drivers::network::request_arp(crate::drivers::network::gateway());
        crate::crypto::tls::https_get(host, ip, path)
    } else {
        http_get_raw(host, path)
    }
}

fn http_get_raw(host: &str, path: &str) -> Result<String, String> {
    let ip = parse_ipv4(host)
        .or_else(|| crate::drivers::network::dns_resolve_a(host).ok())
        .ok_or_else(|| format!("DNS resolve failed for {}", host))?;

    let _ = crate::drivers::network::request_arp(ip);
    let _ = crate::drivers::network::request_arp(crate::drivers::network::gateway());

    crate::drivers::network::tcp_connect(ip, 80)
        .map_err(|e| format!("TCP connect: {}", e))?;

    let start = crate::proc::scheduler::ticks();
    while !crate::drivers::network::tcp_is_connected()
        && crate::proc::scheduler::ticks() - start < 1500
    {
        crate::drivers::network::poll();
        crate::arch::halt();
    }
    if !crate::drivers::network::tcp_is_connected() {
        let _ = crate::drivers::network::tcp_close();
        return Err(String::from("TCP connect timeout"));
    }

    // HTTP/1.0 avoids chunked transfer encoding
    let req = format!(
        "GET {} HTTP/1.0\r\nHost: {}\r\nUser-Agent: CottonBrowser/0.1\r\nAccept: text/html\r\nConnection: close\r\n\r\n",
        path, host
    );
    crate::drivers::network::tcp_send(req.as_bytes())
        .map_err(|e| format!("TCP send: {}", e))?;

    let mut out = String::new();
    let read_start = crate::proc::scheduler::ticks();
    let mut last_data = read_start;
    let mut saw_data = false;

    while crate::proc::scheduler::ticks() - read_start < 8000 {
        crate::drivers::network::poll();
        if let Some((buf, len)) = crate::drivers::network::tcp_read() {
            out.push_str(&String::from_utf8_lossy(&buf[..len]));
            saw_data = true;
            last_data = crate::proc::scheduler::ticks();
            if out.len() >= MAX_RESPONSE {
                break;
            }
        }
        if !crate::drivers::network::tcp_is_connected() {
            break;
        }
        if saw_data && crate::proc::scheduler::ticks() - last_data > 400 {
            break;
        }
        crate::arch::halt();
    }

    let _ = crate::drivers::network::tcp_close();
    if out.is_empty() {
        Err(String::from("No response received"))
    } else {
        Ok(out)
    }
}

// ─── Redirect helpers ─────────────────────────────────────────────────────────

fn find_redirect_location(raw: &str) -> Option<String> {
    let first = raw.lines().next()?;
    // Check for "HTTP/1.x 3xx …"
    let sp = first.find(' ')? + 1;
    if !first.as_bytes().get(sp).map_or(false, |&b| b == b'3') {
        return None;
    }
    for line in raw.lines() {
        let b = line.as_bytes();
        // "location:" is 9 bytes (8 letters + colon)
        if b.len() >= 10 && b[8] == b':' && line[..8].eq_ignore_ascii_case("location") {
            let val = line[9..].trim();
            if !val.is_empty() {
                return Some(String::from(val));
            }
        }
    }
    None
}

fn base_origin(url: &str) -> &str {
    let skip = url.find("://").map(|i| i + 3).unwrap_or(0);
    let rest = &url[skip..];
    let end = rest.find('/').map(|i| skip + i).unwrap_or(url.len());
    &url[..end]
}

// ─── Chunked transfer decoding ────────────────────────────────────────────────

fn is_chunked(headers: &str) -> bool {
    // "transfer-encoding" is 17 chars → colon at index 17
    for line in headers.lines() {
        let b = line.as_bytes();
        if b.len() >= 19 && b[17] == b':' && line[..17].eq_ignore_ascii_case("transfer-encoding")
        {
            return line[18..].trim().eq_ignore_ascii_case("chunked");
        }
    }
    false
}

fn decode_chunked(body: &str) -> String {
    let mut out = String::new();
    let src = body.as_bytes();
    let n = src.len();
    let mut pos = 0;

    while pos < n {
        let line_start = pos;
        while pos + 1 < n && !(src[pos] == b'\r' && src[pos + 1] == b'\n') {
            pos += 1;
        }
        let size_line = body[line_start..pos].trim();
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let chunk_size = usize::from_str_radix(size_hex, 16).unwrap_or(0);

        pos += 2; // skip \r\n
        if chunk_size == 0 || pos + chunk_size > n {
            break;
        }
        let data_end = pos + chunk_size;
        if let Ok(s) = core::str::from_utf8(&src[pos..data_end]) {
            out.push_str(s);
        }
        pos = data_end;
        if pos + 1 < n && src[pos] == b'\r' && src[pos + 1] == b'\n' {
            pos += 2;
        }
    }
    out
}

// ─── HTML renderer ────────────────────────────────────────────────────────────

/// Convert an HTML body to plain text + a list of absolute links.
pub fn render_html(host: &str, body: &str) -> (String, Vec<String>) {
    let _ = host;
    let mut out = String::new();
    let mut links: Vec<String> = Vec::new();
    let mut in_script = false;
    let mut in_style = false;

    let src = body.as_bytes();
    let n = src.len();
    let mut i = 0;

    while i < n {
        if src[i] == b'<' {
            let tag_start = i + 1;
            let mut j = tag_start;
            while j < n && src[j] != b'>' {
                j += 1;
            }
            if j >= n {
                break;
            }
            if let Ok(tag_raw) = core::str::from_utf8(&src[tag_start..j]) {
                let tag_raw = tag_raw.trim();
                let name_end = tag_raw
                    .find(|c: char| c.is_ascii_whitespace())
                    .unwrap_or(tag_raw.len());
                let name = &tag_raw[..name_end];

                if tci(name, "script") {
                    in_script = true;
                } else if tci(name, "/script") {
                    in_script = false;
                } else if tci(name, "style") {
                    in_style = true;
                } else if tci(name, "/style") {
                    in_style = false;
                } else if !in_script && !in_style {
                    process_tag(name, tag_raw, &mut out, &mut links);
                }
            }
            i = j + 1;
        } else if src[i] == b'&' && !in_script && !in_style {
            i += decode_entity(&src[i..], &mut out);
        } else if !in_script && !in_style {
            let b = src[i];
            let seq = if b < 0x80 {
                1
            } else if b & 0xE0 == 0xC0 {
                2
            } else if b & 0xF0 == 0xE0 {
                3
            } else {
                4
            };
            let end = (i + seq).min(n);
            if let Ok(s) = core::str::from_utf8(&src[i..end]) {
                for ch in s.chars() {
                    match ch {
                        '\n' | '\r' | '\t' => {
                            if !out.ends_with(' ') && !out.ends_with('\n') {
                                out.push(' ');
                            }
                        }
                        _ => out.push(ch),
                    }
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }

    let trim = out.trim_end().len();
    out.truncate(trim);
    (out, links)
}

// ─── Tag helpers ─────────────────────────────────────────────────────────────

fn tci(tag: &str, name: &str) -> bool {
    tag.len() == name.len()
        && tag
            .bytes()
            .zip(name.bytes())
            .all(|(a, b)| a.to_ascii_lowercase() == b)
}

fn process_tag(name: &str, raw: &str, out: &mut String, links: &mut Vec<String>) {
    if tci(name, "br") || tci(name, "br/") {
        out.push('\n');
    } else if is_block(name) {
        if !out.ends_with('\n') {
            out.push('\n');
        }
    } else if tci(name, "h1") || tci(name, "h2") || tci(name, "h3") {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    } else if tci(name, "/h1")
        || tci(name, "/h2")
        || tci(name, "/h3")
        || tci(name, "h4")
        || tci(name, "/h4")
        || tci(name, "h5")
        || tci(name, "/h5")
        || tci(name, "h6")
        || tci(name, "/h6")
    {
        if !out.ends_with('\n') {
            out.push('\n');
        }
    } else if tci(name, "li") {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("  * ");
    } else if tci(name, "/li") {
        if !out.ends_with('\n') {
            out.push('\n');
        }
    } else if tci(name, "tr") {
        if !out.ends_with('\n') {
            out.push('\n');
        }
    } else if tci(name, "td") || tci(name, "th") {
        out.push_str("  ");
    } else if tci(name, "a") && links.len() < 30 {
        if let Some(href) = extract_href(raw) {
            if href.starts_with("http") {
                links.push(href);
            }
        }
    }
}

fn is_block(name: &str) -> bool {
    tci(name, "p")
        || tci(name, "/p")
        || tci(name, "div")
        || tci(name, "/div")
        || tci(name, "section")
        || tci(name, "/section")
        || tci(name, "article")
        || tci(name, "/article")
        || tci(name, "header")
        || tci(name, "/header")
        || tci(name, "footer")
        || tci(name, "/footer")
        || tci(name, "nav")
        || tci(name, "/nav")
        || tci(name, "main")
        || tci(name, "/main")
        || tci(name, "ul")
        || tci(name, "/ul")
        || tci(name, "ol")
        || tci(name, "/ol")
        || tci(name, "blockquote")
        || tci(name, "/blockquote")
        || tci(name, "form")
        || tci(name, "/form")
}

fn decode_entity(src: &[u8], out: &mut String) -> usize {
    if src.starts_with(b"&amp;") {
        out.push('&');
        return 5;
    }
    if src.starts_with(b"&lt;") {
        out.push('<');
        return 4;
    }
    if src.starts_with(b"&gt;") {
        out.push('>');
        return 4;
    }
    if src.starts_with(b"&nbsp;") {
        out.push(' ');
        return 6;
    }
    if src.starts_with(b"&quot;") {
        out.push('"');
        return 6;
    }
    if src.starts_with(b"&apos;") {
        out.push('\'');
        return 6;
    }
    if src.starts_with(b"&#39;") {
        out.push('\'');
        return 5;
    }
    if src.starts_with(b"&#") {
        let mut k = 2usize;
        while k < src.len() && src[k] != b';' && k < 12 {
            k += 1;
        }
        if k < src.len() {
            return k + 1;
        }
    }
    out.push('&');
    1
}

fn extract_href(tag: &str) -> Option<String> {
    let lower: Vec<u8> = tag.bytes().map(|b| b.to_ascii_lowercase()).collect();
    let lower_str = core::str::from_utf8(&lower).ok()?;
    let pos = lower_str.find("href=")?;
    let after = &tag[pos + 5..];
    let (delim, start) = if after.starts_with('"') {
        ('"', 1usize)
    } else if after.starts_with('\'') {
        ('\'', 1usize)
    } else {
        return None;
    };
    let end = after[start..].find(delim)?;
    let href = after[start..start + end].trim();
    if href.is_empty() {
        return None;
    }
    Some(String::from(href))
}

// ─── URL utilities ────────────────────────────────────────────────────────────

fn split_url(url: &str) -> (bool, &str, &str) {
    let (https, rest) = if let Some(r) = url.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (false, r)
    } else {
        (false, url)
    };
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    (https, host, path)
}

fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut it = s.splitn(5, '.');
    let a = it.next()?.parse::<u8>().ok()?;
    let b = it.next()?.parse::<u8>().ok()?;
    let c = it.next()?.parse::<u8>().ok()?;
    let d = it.next()?.parse::<u8>().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some([a, b, c, d])
}

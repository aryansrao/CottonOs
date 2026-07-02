use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

const MAX_RESPONSE: usize = 65536;
const MAX_LINKS: usize = 300;

// ─── Styled document model ────────────────────────────────────────────────────
//
// The HTML renderer produces a `Document`: a list of logical lines, each made
// of styled spans. The GUI lays these out (word wrap) and can hit-test link
// spans for clicking; the shell flattens them to plain text.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SpanStyle {
    Normal,
    Heading,
    Bold,
    Pre,
    Bullet,
}

#[derive(Clone)]
pub struct DocSpan {
    pub text: String,
    pub style: SpanStyle,
    /// Index into `Document::links`
    pub link: Option<u16>,
}

pub type DocLine = Vec<DocSpan>;

#[derive(Clone, Default)]
pub struct Document {
    pub lines: Vec<DocLine>,
    pub links: Vec<String>,
}

impl Document {
    pub fn from_text(text: &str) -> Self {
        let lines = text
            .lines()
            .map(|l| {
                if l.is_empty() {
                    Vec::new()
                } else {
                    alloc::vec![DocSpan {
                        text: String::from(l),
                        style: SpanStyle::Normal,
                        link: None,
                    }]
                }
            })
            .collect();
        Self {
            lines,
            links: Vec::new(),
        }
    }

    /// Flatten to plain text (shell `browse` command).
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            for span in line {
                out.push_str(&span.text);
            }
            out.push('\n');
        }
        out
    }
}

/// A span placed by the word-wrap layout: `col` is the character column on
/// its visual line (monospace 8px grid).
pub struct LaidSpan {
    pub col: usize,
    pub text: String,
    pub style: SpanStyle,
    pub link: Option<u16>,
}

/// Word-wrap a document to `max_chars` columns.
pub fn layout(doc: &Document, max_chars: usize) -> Vec<Vec<LaidSpan>> {
    let max_chars = max_chars.max(10);
    let mut out: Vec<Vec<LaidSpan>> = Vec::new();

    for line in &doc.lines {
        if line.is_empty() {
            out.push(Vec::new());
            continue;
        }

        let mut visual: Vec<LaidSpan> = Vec::new();
        let mut col = 0usize;

        for span in line {
            let mut run = String::new();
            let mut run_col = col;

            let mut flush =
                |run: &mut String, run_col: usize, visual: &mut Vec<LaidSpan>| {
                    if !run.is_empty() {
                        visual.push(LaidSpan {
                            col: run_col,
                            text: core::mem::take(run),
                            style: span.style,
                            link: span.link,
                        });
                    }
                };

            // Tokenize into whitespace runs and words
            let bytes = span.text.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                let is_space = bytes[i] == b' ';
                let mut j = i;
                while j < bytes.len() && (bytes[j] == b' ') == is_space {
                    j += 1;
                }
                let token = &span.text[i..j];
                i = j;

                if is_space {
                    if col + token.len() <= max_chars {
                        run.push_str(token);
                        col += token.len();
                    } else {
                        // Space at line edge: break the line, drop the spaces
                        flush(&mut run, run_col, &mut visual);
                        out.push(core::mem::take(&mut visual));
                        col = 0;
                        run_col = 0;
                    }
                } else {
                    let mut word = token;
                    loop {
                        if col + word.len() <= max_chars {
                            run.push_str(word);
                            col += word.len();
                            break;
                        }
                        if col > 0 {
                            // Doesn't fit on this line: wrap
                            flush(&mut run, run_col, &mut visual);
                            out.push(core::mem::take(&mut visual));
                            col = 0;
                            run_col = 0;
                            // strip the leading spaces that may be in `run` (none: run flushed)
                            continue;
                        }
                        // Word longer than the whole line: hard split
                        let (head, tail) = word.split_at(max_chars);
                        run.push_str(head);
                        flush(&mut run, run_col, &mut visual);
                        out.push(core::mem::take(&mut visual));
                        col = 0;
                        run_col = 0;
                        word = tail;
                        if word.is_empty() {
                            break;
                        }
                    }
                }
            }
            flush(&mut run, run_col, &mut visual);
        }

        out.push(visual);
    }

    out
}

/// Resolve an href against the page's URL. Returns None for schemes and
/// fragments the browser can't follow.
pub fn resolve_url(base: &str, href: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty()
        || href.starts_with('#')
        || href.starts_with("javascript:")
        || href.starts_with("mailto:")
        || href.starts_with("data:")
        || href.starts_with("tel:")
    {
        return None;
    }

    // Strip fragment
    let href = href.split('#').next().unwrap_or(href);
    if href.is_empty() {
        return None;
    }

    if href.starts_with("http://") || href.starts_with("https://") {
        return Some(String::from(href));
    }

    let (scheme, rest) = if let Some(r) = base.strip_prefix("https://") {
        ("https", r)
    } else if let Some(r) = base.strip_prefix("http://") {
        ("http", r)
    } else {
        ("http", base)
    };
    let host_end = rest.find('/').unwrap_or(rest.len());
    let host = &rest[..host_end];
    if host.is_empty() {
        return None;
    }

    if let Some(r) = href.strip_prefix("//") {
        return Some(format!("{}://{}", scheme, r));
    }
    if href.starts_with('/') {
        return Some(format!("{}://{}{}", scheme, host, href));
    }

    // Relative to the base path's directory
    let base_path = &rest[host_end..];
    let dir_end = base_path.rfind('/').map(|i| i + 1).unwrap_or(0);
    let dir = if dir_end == 0 { "/" } else { &base_path[..dir_end] };
    Some(format!("{}://{}{}{}", scheme, host, dir, href))
}

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

/// Fetch a URL (following up to 4 redirects) and render to a styled Document.
/// Links inside the document are resolved against the final URL.
pub fn fetch_and_render(url: &str) -> Result<Document, String> {
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

        return Ok(render_html_doc(&current, body));
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
        && crate::proc::scheduler::ticks() - start < 8000
    {
        crate::drivers::network::poll();
        crate::task::yield_to_main();
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
        if saw_data && crate::proc::scheduler::ticks() - last_data > 800 {
            break;
        }
        crate::task::yield_to_main();
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

struct DocBuilder {
    lines: Vec<DocLine>,
    cur: DocLine,
    text: String,
    links: Vec<String>,
    cur_link: Option<u16>,
    heading: u32,
    bold: u32,
    pre: bool,
    last_space: bool,
}

impl DocBuilder {
    fn new() -> Self {
        Self {
            lines: Vec::new(),
            cur: Vec::new(),
            text: String::new(),
            links: Vec::new(),
            cur_link: None,
            heading: 0,
            bold: 0,
            pre: false,
            last_space: true,
        }
    }

    fn style(&self) -> SpanStyle {
        if self.heading > 0 {
            SpanStyle::Heading
        } else if self.pre {
            SpanStyle::Pre
        } else if self.bold > 0 {
            SpanStyle::Bold
        } else {
            SpanStyle::Normal
        }
    }

    fn flush_span(&mut self) {
        if self.text.is_empty() {
            return;
        }
        let text = core::mem::take(&mut self.text);
        self.cur.push(DocSpan {
            text,
            style: self.style(),
            link: self.cur_link,
        });
    }

    fn line_has_content(&self) -> bool {
        !self.text.trim().is_empty()
            || self.cur.iter().any(|s| !s.text.trim().is_empty())
    }

    fn newline(&mut self) {
        self.flush_span();
        self.lines.push(core::mem::take(&mut self.cur));
        self.last_space = true;
    }

    /// End the current line (if it has content) and ensure one blank line.
    fn blank_line(&mut self) {
        if self.line_has_content() {
            self.newline();
        }
        if self.lines.last().map_or(false, |l| !l.is_empty()) {
            self.lines.push(Vec::new());
        }
        self.cur.clear();
        self.text.clear();
        self.last_space = true;
    }

    /// End the current line only if it already has content.
    fn break_line(&mut self) {
        if self.line_has_content() {
            self.newline();
        } else {
            self.cur.clear();
            self.text.clear();
            self.last_space = true;
        }
    }

    fn push_char(&mut self, ch: char) {
        if self.pre {
            match ch {
                '\n' => self.newline(),
                '\r' => {}
                '\t' => self.text.push_str("    "),
                _ => self.text.push(ch),
            }
            self.last_space = false;
            return;
        }
        match ch {
            ' ' | '\n' | '\r' | '\t' => {
                if !self.last_space {
                    self.text.push(' ');
                    self.last_space = true;
                }
            }
            _ => {
                self.text.push(ch);
                self.last_space = false;
            }
        }
    }

    fn push_str(&mut self, s: &str) {
        for ch in s.chars() {
            self.push_char(ch);
        }
    }

    fn start_link(&mut self, url: String) {
        if self.links.len() >= MAX_LINKS {
            return;
        }
        self.flush_span();
        self.links.push(url);
        self.cur_link = Some((self.links.len() - 1) as u16);
    }

    fn end_link(&mut self) {
        self.flush_span();
        self.cur_link = None;
    }

    fn finish(mut self) -> Document {
        self.flush_span();
        if !self.cur.is_empty() {
            self.lines.push(self.cur);
        }
        // Trim trailing empty lines
        while self.lines.last().map_or(false, |l| l.is_empty()) {
            self.lines.pop();
        }
        Document {
            lines: self.lines,
            links: self.links,
        }
    }
}

/// Convert an HTML body to a styled Document with links resolved against
/// `base_url`.
pub fn render_html_doc(base_url: &str, body: &str) -> Document {
    let mut b = DocBuilder::new();
    let mut in_script = false;
    let mut in_style = false;
    let mut in_title = false;

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
                } else if tci(name, "title") {
                    in_title = true;
                } else if tci(name, "/title") {
                    in_title = false;
                } else if !in_script && !in_style {
                    process_tag(&mut b, name, tag_raw, base_url);
                }
            }
            i = j + 1;
        } else if in_script || in_style || in_title {
            i += 1;
        } else if src[i] == b'&' {
            let (consumed, ch) = decode_entity(&src[i..]);
            if let Some(ch) = ch {
                b.push_char(ch);
            }
            i += consumed;
        } else {
            let byte = src[i];
            let seq = if byte < 0x80 {
                1
            } else if byte & 0xE0 == 0xC0 {
                2
            } else if byte & 0xF0 == 0xE0 {
                3
            } else {
                4
            };
            let end = (i + seq).min(n);
            if let Ok(s) = core::str::from_utf8(&src[i..end]) {
                b.push_str(s);
            }
            i = end;
        }
    }

    b.finish()
}

// ─── Tag helpers ─────────────────────────────────────────────────────────────

fn tci(tag: &str, name: &str) -> bool {
    tag.len() == name.len()
        && tag
            .bytes()
            .zip(name.bytes())
            .all(|(a, b)| a.to_ascii_lowercase() == b)
}

fn process_tag(b: &mut DocBuilder, name: &str, raw: &str, base_url: &str) {
    if tci(name, "br") || tci(name, "br/") {
        b.newline();
    } else if tci(name, "h1") || tci(name, "h2") || tci(name, "h3") {
        b.blank_line();
        b.heading += 1;
    } else if tci(name, "/h1") || tci(name, "/h2") || tci(name, "/h3") {
        b.heading = b.heading.saturating_sub(1);
        b.break_line();
    } else if tci(name, "h4") || tci(name, "h5") || tci(name, "h6") {
        b.break_line();
        b.bold += 1;
    } else if tci(name, "/h4") || tci(name, "/h5") || tci(name, "/h6") {
        b.bold = b.bold.saturating_sub(1);
        b.break_line();
    } else if tci(name, "p") {
        b.blank_line();
    } else if tci(name, "/p") || tci(name, "tr") || tci(name, "/li") || is_block(name) {
        b.break_line();
    } else if tci(name, "li") {
        b.break_line();
        b.flush_span();
        b.cur.push(DocSpan {
            text: String::from("  * "),
            style: SpanStyle::Bullet,
            link: None,
        });
        b.last_space = true;
    } else if tci(name, "td") || tci(name, "th") {
        b.push_str("  ");
    } else if tci(name, "hr") || tci(name, "hr/") {
        b.break_line();
        b.flush_span();
        b.cur.push(DocSpan {
            text: String::from("----------------------------------------"),
            style: SpanStyle::Bullet,
            link: None,
        });
        b.newline();
    } else if tci(name, "pre") {
        b.blank_line();
        b.pre = true;
    } else if tci(name, "/pre") {
        b.pre = false;
        b.break_line();
    } else if tci(name, "b") || tci(name, "strong") || tci(name, "em") || tci(name, "i") {
        b.flush_span();
        b.bold += 1;
    } else if tci(name, "/b") || tci(name, "/strong") || tci(name, "/em") || tci(name, "/i") {
        b.flush_span();
        b.bold = b.bold.saturating_sub(1);
    } else if tci(name, "a") {
        if let Some(href) = extract_href(raw) {
            if let Some(url) = resolve_url(base_url, &href) {
                b.start_link(url);
            }
        }
    } else if tci(name, "/a") {
        b.end_link();
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

/// Decode one HTML entity at the start of `src`.
/// Returns (bytes consumed, decoded char if any).
fn decode_entity(src: &[u8]) -> (usize, Option<char>) {
    const NAMED: &[(&[u8], char)] = &[
        (b"&amp;", '&'),
        (b"&lt;", '<'),
        (b"&gt;", '>'),
        (b"&nbsp;", ' '),
        (b"&quot;", '"'),
        (b"&apos;", '\''),
        (b"&mdash;", '-'),
        (b"&ndash;", '-'),
        (b"&hellip;", '.'),
        (b"&copy;", 'c'),
        (b"&middot;", '*'),
        (b"&bull;", '*'),
        (b"&rsquo;", '\''),
        (b"&lsquo;", '\''),
        (b"&rdquo;", '"'),
        (b"&ldquo;", '"'),
    ];
    for (pat, ch) in NAMED {
        if src.starts_with(pat) {
            return (pat.len(), Some(*ch));
        }
    }

    // Numeric: &#123; or &#xAB;
    if src.starts_with(b"&#") {
        let hex = src.len() > 2 && (src[2] == b'x' || src[2] == b'X');
        let digits_start = if hex { 3 } else { 2 };
        let mut k = digits_start;
        let mut value: u32 = 0;
        while k < src.len() && k < digits_start + 8 {
            let d = src[k];
            let digit = match d {
                b'0'..=b'9' => (d - b'0') as u32,
                b'a'..=b'f' if hex => (d - b'a' + 10) as u32,
                b'A'..=b'F' if hex => (d - b'A' + 10) as u32,
                _ => break,
            };
            value = value * (if hex { 16 } else { 10 }) + digit;
            k += 1;
        }
        if k > digits_start && k < src.len() && src[k] == b';' {
            return (k + 1, char::from_u32(value));
        }
    }

    (1, Some('&'))
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

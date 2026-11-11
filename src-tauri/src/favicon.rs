use base64::{engine::general_purpose::STANDARD as B64, Engine};
use std::{collections::HashMap, io::Read, sync::Mutex, time::Duration};

const MAX_ICON_BYTES: u64 = 512 * 1024;
const MAX_HTML_BYTES: u64 = 1024 * 1024;
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";

#[derive(Default)]
pub struct Favicons(Mutex<HashMap<String, Option<String>>>);

impl Favicons {
    pub fn get(&self, page_url: &str) -> Option<String> {
        if let Some(cached) = self.0.lock().unwrap().get(page_url) {
            return cached.clone();
        }
        let icon = fetch_icon(page_url);
        self.0.lock().unwrap().insert(page_url.to_string(), icon.clone());
        icon
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(8))
        .user_agent(USER_AGENT)
        .build()
}

fn fetch_icon(page_url: &str) -> Option<String> {
    let agent = agent();
    let page = agent.get(page_url).call().ok();
    let base = page.as_ref().map(|r| r.get_url().to_string()).unwrap_or_else(|| page_url.to_string());
    let html = page.and_then(|r| {
        let mut text = String::new();
        r.into_reader().take(MAX_HTML_BYTES).read_to_string(&mut text).ok()?;
        Some(text)
    });

    let mut candidates = html.map(|h| icon_links(&h)).unwrap_or_default();
    candidates.sort_by_key(|(_, size)| std::cmp::Reverse(*size));
    candidates
        .into_iter()
        .map(|(href, _)| href)
        .chain(std::iter::once("/favicon.ico".to_string()))
        .filter_map(|href| resolve(&base, &href))
        .find_map(|url| download_image(&agent, &url))
}

fn download_image(agent: &ureq::Agent, url: &str) -> Option<String> {
    if let Some(data) = url.strip_prefix("data:") {
        return data.starts_with("image/").then(|| url.to_string());
    }
    let response = agent.get(url).call().ok()?;
    let mime = response.content_type().to_string();
    let mut bytes = Vec::new();
    response.into_reader().take(MAX_ICON_BYTES).read_to_end(&mut bytes).ok()?;
    let mime = sniff(&bytes).map(str::to_string).or_else(|| mime.starts_with("image/").then_some(mime))?;
    Some(format!("data:{mime};base64,{}", B64.encode(&bytes)))
}

fn sniff(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0, 0, 1, 0, ..] => Some("image/x-icon"),
        [0xFF, 0xD8, ..] => Some("image/jpeg"),
        [b'G', b'I', b'F', ..] => Some("image/gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        _ => {
            let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_ascii_lowercase();
            head.contains("<svg").then_some("image/svg+xml")
        }
    }
}

fn icon_links(html: &str) -> Vec<(String, u32)> {
    let lower = html.to_ascii_lowercase();
    let mut links = Vec::new();
    let mut cursor = 0;
    while let Some(start) = lower[cursor..].find("<link") {
        let start = cursor + start;
        let Some(len) = lower[start..].find('>') else { break };
        let end = start + len;
        cursor = end;
        let tag = &html[start..end];
        let Some(rel) = attribute(tag, "rel") else { continue };
        let rel = rel.to_ascii_lowercase();
        if !rel.split_whitespace().any(|r| r == "icon" || r == "apple-touch-icon" || r == "apple-touch-icon-precomposed") {
            continue;
        }
        let Some(href) = attribute(tag, "href") else { continue };
        let size = attribute(tag, "sizes")
            .and_then(|s| s.split(['x', 'X']).next().and_then(|n| n.trim().parse().ok()))
            .unwrap_or(if rel.contains("apple") { 180 } else { 16 });
        links.push((decode_entities(&href), size));
    }
    links
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(pos) = lower[from..].find(name) {
        let pos = from + pos;
        from = pos + name.len();
        let preceded = lower[..pos].chars().last().is_some_and(|c| c.is_whitespace());
        let rest = lower[from..].trim_start();
        if !preceded || !rest.starts_with('=') {
            continue;
        }
        let value_start = tag.len() - rest.len() + 1;
        let value = tag[value_start..].trim_start();
        return Some(match value.chars().next()? {
            quote @ ('"' | '\'') => value[1..].split(quote).next()?.to_string(),
            _ => value.split(|c: char| c.is_whitespace() || c == '/').next()?.to_string(),
        });
    }
    None
}

fn decode_entities(text: &str) -> String {
    text.replace("&amp;", "&").replace("&#38;", "&")
}

fn resolve(base: &str, href: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() {
        return None;
    }
    if href.starts_with("http://") || href.starts_with("https://") || href.starts_with("data:") {
        return Some(href.to_string());
    }
    let scheme_end = base.find("://")? + 3;
    let scheme = &base[..scheme_end - 3];
    if let Some(rest) = href.strip_prefix("//") {
        return Some(format!("{scheme}://{rest}"));
    }
    let after_scheme = &base[scheme_end..];
    let host_end = after_scheme.find(['/', '?', '#']).unwrap_or(after_scheme.len());
    let origin = &base[..scheme_end + host_end];
    if href.starts_with('/') {
        return Some(format!("{origin}{href}"));
    }
    let path = &after_scheme[host_end..];
    let path = path.split(['?', '#']).next().unwrap_or("");
    let dir = &path[..path.rfind('/').map_or(0, |i| i + 1)];
    let dir = if dir.is_empty() { "/" } else { dir };
    Some(format!("{origin}{dir}{href}"))
}

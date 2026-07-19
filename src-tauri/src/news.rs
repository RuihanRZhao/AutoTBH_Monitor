//! Patch-notes (SteamDB RSS) + Steam news parsing. Port of `news.mjs` (pragmatic subset:
//! item extraction + a safe-HTML/BBCode reduction).

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{json, Value};

fn rfc822_to_ts(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc2822(s).ok().map(|d| d.timestamp())
}

fn xml_decode(s: &str) -> String {
    let cdata = Regex::new(r"(?s)<!\[CDATA\[(.*?)\]\]>").unwrap();
    let s = cdata.replace_all(s, "$1").to_string();
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"")
        .replace("&#039;", "'").replace("&#39;", "'").replace("&apos;", "'")
        .replace("&amp;", "&").trim().to_string()
}

static ITEM_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)<item\b[^>]*>(.*?)</item>").unwrap());

fn pick(block: &str, tag: &str) -> Option<String> {
    let re = Regex::new(&format!(r"(?s)<{tag}(?:\s[^>]*)?>(.*?)</{tag}>")).unwrap();
    re.captures(block).map(|c| xml_decode(&c[1]))
}

/// SteamDB PatchnotesRSS → patch note items.
pub fn parse_patchnotes(xml: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for m in ITEM_RE.captures_iter(xml) {
        let b = &m[1];
        let guid = pick(b, "guid").unwrap_or_default();
        let desc = pick(b, "description").unwrap_or_default();
        let build_re = Regex::new(r"(?i)\s*\(SteamDB Build\s+\d+\)\s*$").unwrap();
        let headline = build_re.replace(&desc, "").trim().to_string();
        let thumb = Regex::new(r#"<media:thumbnail[^>]*\burl="([^"]+)""#).unwrap()
            .captures(b).map(|c| xml_decode(&c[1]));
        let build = Regex::new(r"(\d{4,})").unwrap().captures(&guid).map(|c| c[1].to_string());
        out.push(json!({
            "title": if headline.is_empty() { pick(b, "title").unwrap_or_else(|| "Update".into()) } else { headline },
            "genericTitle": pick(b, "title"),
            "date": pick(b, "pubDate").and_then(|d| rfc822_to_ts(&d)),
            "build": build,
            "link": pick(b, "link"),
            "thumb": thumb,
        }));
    }
    out
}

/// Steam ISteamNews JSON → news items.
pub fn parse_steam_news(json_v: &Value) -> Vec<Value> {
    let items = json_v.get("appnews").and_then(|a| a.get("newsitems")).and_then(|v| v.as_array()).cloned().unwrap_or_default();
    items.iter().map(|n| {
        let url = n.get("url").and_then(|v| v.as_str()).filter(|u| u.starts_with("http")).map(String::from);
        json!({
            "gid": n.get("gid").map(|v| v.to_string()).unwrap_or_default().trim_matches('"'),
            "title": n.get("title").and_then(|v| v.as_str()).unwrap_or("").trim(),
            "author": n.get("author"),
            "feedlabel": n.get("feedlabel"),
            "date": n.get("date").and_then(|v| v.as_i64()),
            "url": url,
            "html": bbcode_to_html(n.get("contents").and_then(|v| v.as_str()).unwrap_or("")),
            "thumb": first_image(n.get("contents").and_then(|v| v.as_str()).unwrap_or("")),
        })
    }).collect()
}

/// Steam store NEWS RSS → news items (description is entity-escaped HTML).
pub fn parse_store_news_rss(xml: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for m in ITEM_RE.captures_iter(xml) {
        let b = &m[1];
        let encl = Regex::new(r#"<enclosure[^>]*\burl="([^"]+)""#).unwrap().captures(b).map(|c| c[1].to_string());
        let link = pick(b, "link");
        let raw_desc = Regex::new(r"(?s)<description(?:\s[^>]*)?>(.*?)</description>").unwrap()
            .captures(b).map(|c| c[1].to_string()).unwrap_or_default();
        out.push(json!({
            "gid": pick(b, "guid").or(link.clone()).unwrap_or_default().chars().filter(|c| c.is_alphanumeric()).collect::<String>(),
            "title": pick(b, "title").unwrap_or_else(|| "Untitled".into()),
            "author": Value::Null,
            "feedlabel": "Steam News",
            "date": pick(b, "pubDate").and_then(|d| rfc822_to_ts(&d)),
            "url": link.filter(|l| l.starts_with("http")),
            "html": sanitize_html(&raw_desc),
            "thumb": encl.filter(|e| e.starts_with("http")),
        }));
    }
    out
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

const STEAM_CLAN_IMG: &str = "https://clan.cloudflare.steamstatic.com/images";
fn resolve_steam_url(u: &str) -> Option<String> {
    let s = u.trim().replace("{STEAM_CLAN_IMAGE}", STEAM_CLAN_IMG);
    if s.starts_with("http") { Some(s) } else { None }
}
fn first_image(bb: &str) -> Option<String> {
    Regex::new(r"(?i)\[img\]\s*([^\[]+?)\s*\[/img\]").unwrap()
        .captures(bb).and_then(|c| resolve_steam_url(&c[1]))
}

/// Reduce Steam BBCode to a small safe HTML whitelist (pragmatic port of `bbcodeToHtml`).
pub fn bbcode_to_html(src: &str) -> String {
    let mut s = esc(src);
    let rules: &[(&str, &str)] = &[
        (r"(?is)\[url=([^\]]+)\](.*?)\[/url\]", r#"<a href="$1" target="_blank" rel="noopener noreferrer">$2</a>"#),
        (r"(?is)\[img\]\s*(.*?)\s*\[/img\]", r#"<img src="$1" loading="lazy" alt="">"#),
        (r"(?is)\[h1\](.*?)\[/h1\]", "<h3>$1</h3>"),
        (r"(?is)\[h2\](.*?)\[/h2\]", "<h3>$1</h3>"),
        (r"(?is)\[h3\](.*?)\[/h3\]", "<h4>$1</h4>"),
        (r"(?is)\[b\](.*?)\[/b\]", "<strong>$1</strong>"),
        (r"(?is)\[i\](.*?)\[/i\]", "<em>$1</em>"),
        (r"(?is)\[u\](.*?)\[/u\]", "<u>$1</u>"),
        (r"(?is)\[list\](.*?)\[/list\]", "<ul>$1</ul>"),
        (r"(?is)\[\*\]\s*", "<li>"),
    ];
    for (pat, rep) in rules {
        s = Regex::new(pat).unwrap().replace_all(&s, *rep).to_string();
    }
    // drop any remaining tags, keep inner text
    s = Regex::new(r"(?i)\[/?[a-z][a-z0-9]*(?:=[^\]]*)?\]").unwrap().replace_all(&s, "").to_string();
    s.trim().to_string()
}

/// Reveal a whitelist of tags from entity-escaped RSS description HTML (pragmatic port of `sanitizeHtml`).
pub fn sanitize_html(escaped: &str) -> String {
    let mut s = escaped.to_string();
    // strip escaped script/style
    s = Regex::new(r"(?is)&lt;(script|style).*?&lt;/(script|style)&gt;").unwrap().replace_all(&s, "").to_string();
    let map: &[(&str, &str)] = &[
        (r"(?is)&lt;p&gt;", "<p>"), (r"(?is)&lt;/p&gt;", "</p>"),
        (r"(?is)&lt;br\s*/?&gt;", "<br>"), (r"(?is)&lt;hr\s*/?&gt;", "<hr>"),
        (r"(?is)&lt;(strong|b)&gt;", "<strong>"), (r"(?is)&lt;/(strong|b)&gt;", "</strong>"),
        (r"(?is)&lt;(em|i)&gt;", "<em>"), (r"(?is)&lt;/(em|i)&gt;", "</em>"),
        (r"(?is)&lt;ul&gt;", "<ul>"), (r"(?is)&lt;/ul&gt;", "</ul>"),
        (r"(?is)&lt;li&gt;", "<li>"), (r"(?is)&lt;/li&gt;", "</li>"),
    ];
    for (pat, rep) in map {
        s = Regex::new(pat).unwrap().replace_all(&s, *rep).to_string();
    }
    // drop any remaining escaped tags
    s = Regex::new(r"(?is)&lt;/?[a-z][a-z0-9]*(?:[^&]*?)&gt;").unwrap().replace_all(&s, "").to_string();
    s.trim().to_string()
}

use std::{collections::HashSet, sync::LazyLock, time::Duration};

use aes::{Aes128, Aes256};
use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{AeadInPlace, generic_array::GenericArray},
};
use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use ctr::cipher::{KeyIvInit, StreamCipher};
use futures_util::StreamExt;
use regex::Regex;
use reqwest::{Client, StatusCode, header};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use url::Url;

const RIGHTS_URL: &str = "https://studio-api-prod.suno.com/api/mango/rights";
const RIGHTS_HOST: &str = "studio-api-prod.suno.com";
const MAX_PAGE_BYTES: usize = 12 * 1024 * 1024;
const MAX_AUDIO_BYTES: usize = 200 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;
const SUNO_ACCESS_HINT: &str = "请先用浏览器确认当前网络能够打开 https://suno.com；如无法访问，请检查网络连接、代理设置或所在地区的访问情况后重试。";

static UUID_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$")
        .expect("valid UUID regular expression")
});
static SHARE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z0-9_-]{8,80}$").expect("valid share code regular expression")
});
static MEDIA_HOST_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^d[a-z0-9]+\.cloudfront\.net$").expect("valid media host regular expression")
});
static CLIP_CHUNK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(?:^|\n)?[0-9a-z]+:\s*\[\s*"\$"\s*,\s*"\$L[^"\n]+"\s*,\s*null\s*,\s*\{\s*"clip"\s*:"#)
        .expect("valid clip chunk regular expression")
});
static TEXT_CHUNK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)(?:^|\n)[0-9a-z]+:T[^,\n]*,").expect("valid text chunk regular expression")
});
static TITLE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)"clip"\s*:\s*\{.{0,5000}?"title"\s*:\s*"((?:\\.|[^"\\])*)""#)
        .expect("valid title regular expression")
});
static MEDIA_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)"media_urls"\s*:\s*\[\s*\{.{0,1000}?"url"\s*:\s*"([^"\s]+)""#)
        .expect("valid media URL regular expression")
});
static DURATION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)"duration"\s*:\s*([0-9]+(?:\.[0-9]+)?)"#)
        .expect("valid duration regular expression")
});
static OG_TITLE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<meta\s+[^>]*property=["']og:title["'][^>]*content=["']([^"']+)["']"#)
        .expect("valid Open Graph title regular expression")
});
static OG_TITLE_REVERSED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<meta\s+[^>]*content=["']([^"']+)["'][^>]*property=["']og:title["']"#)
        .expect("valid reversed Open Graph title regular expression")
});
static HTML_TITLE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<title[^>]*>(.*?)</title>").expect("valid HTML title regular expression")
});
static UNICODE_BRACED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\\u\{([0-9a-f]+)\}").expect("valid braced Unicode regular expression")
});
static UNICODE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\\u([0-9a-f]{4})").expect("valid Unicode regular expression")
});
static NO_LYRICS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\[(?:instrumental|no\s+lyrics?|music|silence)\b[^\]]*\]$")
        .expect("valid instrumental marker regular expression")
});

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Song {
    pub id: String,
    pub title: String,
    pub lyrics: String,
    pub media_url: String,
    pub source_url: String,
    pub duration: Option<f64>,
    pub author: Option<String>,
    pub published_at: Option<String>,
    pub play_count: Option<f64>,
    pub like_count: Option<f64>,
    pub share_count: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicSong {
    pub id: String,
    pub title: String,
    pub lyrics: String,
    pub source_url: String,
    pub duration: Option<f64>,
    pub author: Option<String>,
    pub published_at: Option<String>,
    pub play_count: Option<f64>,
    pub like_count: Option<f64>,
    pub share_count: Option<f64>,
    pub preview_url: String,
}

impl Song {
    pub fn public(&self) -> PublicSong {
        PublicSong {
            id: self.id.clone(),
            title: self.title.clone(),
            lyrics: self.lyrics.clone(),
            source_url: self.source_url.clone(),
            duration: self.duration,
            author: self.author.clone(),
            published_at: self.published_at.clone(),
            play_count: self.play_count,
            like_count: self.like_count,
            share_count: self.share_count,
            preview_url: format!("preview:current:{}", self.id),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub phase: String,
    pub message: String,
    pub received: u64,
    pub total: u64,
    pub percent: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Rights {
    glt: String,
    key: String,
    iv: String,
}

#[derive(Clone, Debug, PartialEq)]
enum LinkKind {
    Share,
    Song,
}

#[derive(Clone, Debug)]
struct ParsedLink {
    kind: LinkKind,
    id: Option<String>,
    url: Url,
}

#[derive(Clone, Debug)]
struct ClipRecord {
    page: String,
    chunk_start: usize,
    clip: Value,
}

fn parse_suno_url(input: &str) -> Result<ParsedLink> {
    let input = input.trim();
    if input.is_empty() || input.len() > 2048 {
        bail!("请输入有效的 Suno 分享链接");
    }
    let url = Url::parse(input).context("Suno 链接无效")?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        bail!("Suno 链接必须使用安全的 HTTPS 地址");
    }
    if !matches!(url.host_str(), Some("suno.com" | "www.suno.com")) {
        bail!("只支持 suno.com 的分享链接");
    }
    if url.fragment().is_some() {
        bail!("Suno 链接不能包含片段地址");
    }
    let parts: Vec<_> = url
        .path_segments()
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() == 2 && parts[0].eq_ignore_ascii_case("s") {
        if url.query().is_some() {
            bail!("Suno 链接包含不支持的查询参数");
        }
        if !SHARE_RE.is_match(parts[1]) {
            bail!("Suno 分享码格式不正确");
        }
        return Ok(ParsedLink {
            kind: LinkKind::Share,
            id: None,
            url,
        });
    }
    if parts.len() == 2 && parts[0].eq_ignore_ascii_case("song") {
        if !UUID_RE.is_match(parts[1]) {
            bail!("Suno 歌曲 ID 格式不正确");
        }
        for (key, value) in url.query_pairs() {
            if key != "sh" {
                bail!("Suno 链接包含不支持的查询参数");
            }
            if !SHARE_RE.is_match(&value) {
                bail!("Suno 分享码格式不正确");
            }
        }
        return Ok(ParsedLink {
            kind: LinkKind::Song,
            id: Some(parts[1].to_ascii_lowercase()),
            url,
        });
    }
    bail!("只支持 suno.com/s/分享码 或 suno.com/song/歌曲ID 链接")
}

fn validate_page_url(url: &Url) -> Result<ParsedLink> {
    let parsed = parse_suno_url(url.as_str())?;
    if parsed.kind != LinkKind::Song {
        bail!("Suno 页面没有跳转到歌曲地址");
    }
    Ok(parsed)
}

fn validate_media_url(value: &str, id: &str) -> Result<Url> {
    let url = Url::parse(value).context("音频地址无效")?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        bail!("音频地址必须使用安全的 HTTPS 地址");
    }
    let host = url.host_str().unwrap_or_default();
    if !MEDIA_HOST_RE.is_match(host) {
        bail!("歌曲音频地址不是受支持的 Suno 音频地址");
    }
    if url.path() != format!("/1/clip/{id}.m4a")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("歌曲音频地址与歌曲 ID 不匹配");
    }
    Ok(url)
}

fn validate_rights_url(value: &str) -> Result<Url> {
    let url = Url::parse(value).context("授权地址无效")?;
    if url.scheme() != "https"
        || url.host_str() != Some(RIGHTS_HOST)
        || url.path() != "/api/mango/rights"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("授权地址不是受支持的 Suno 服务地址");
    }
    Ok(url)
}

fn decode_rsc_text(value: &str) -> String {
    let braced = UNICODE_BRACED_RE.replace_all(value, |captures: &regex::Captures<'_>| {
        u32::from_str_radix(&captures[1], 16)
            .ok()
            .and_then(char::from_u32)
            .map(String::from)
            .unwrap_or_else(|| captures[0].to_string())
    });
    let plain = UNICODE_RE.replace_all(&braced, |captures: &regex::Captures<'_>| {
        u32::from_str_radix(&captures[1], 16)
            .ok()
            .and_then(char::from_u32)
            .map(String::from)
            .unwrap_or_else(|| captures[0].to_string())
    });
    plain
        .replace("\\n", "\n")
        .replace("\\r", "\r")
        .replace("\\t", "\t")
        .replace("\\\"", "\"")
        .replace("\\\\", "\\")
}

fn unescape_field(value: &str) -> String {
    serde_json::from_str::<String>(&format!("\"{value}\""))
        .unwrap_or_else(|_| decode_rsc_text(value))
}

fn find_balanced_json_end(value: &str, start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, character) in value[start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn escape_json_control_characters(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut in_string = false;
    let mut escaped = false;
    for character in value.chars() {
        if in_string {
            if escaped {
                output.push(character);
                escaped = false;
            } else {
                match character {
                    '\\' => {
                        output.push(character);
                        escaped = true;
                    }
                    '"' => {
                        output.push(character);
                        in_string = false;
                    }
                    '\n' => output.push_str("\\n"),
                    '\r' => output.push_str("\\r"),
                    '\t' => output.push_str("\\t"),
                    _ => output.push(character),
                }
            }
        } else {
            output.push(character);
            if character == '"' {
                in_string = true;
            }
        }
    }
    output
}

fn extract_clip_record(page_text: &str, expected_id: Option<&str>) -> Option<ClipRecord> {
    let page = decode_rsc_text(page_text);
    for found in CLIP_CHUNK_RE.find_iter(&page) {
        let relative = found.as_str().rfind('{')?;
        let object_start = found.start() + relative;
        let end = find_balanced_json_end(&page, object_start)?;
        let json = escape_json_control_characters(&page[object_start..=end]);
        let record: Value = serde_json::from_str(&json).ok()?;
        let clip = record.get("clip")?.clone();
        if let Some(expected) = expected_id
            && clip
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_ascii_lowercase)
                .as_deref()
                != Some(expected)
        {
            continue;
        }
        let chunk_start = found.start() + usize::from(found.as_str().starts_with('\n'));
        return Some(ClipRecord {
            page,
            chunk_start,
            clip,
        });
    }
    None
}

fn nullable_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(String::from)
}

fn nullable_number(value: Option<&Value>) -> Option<f64> {
    let number = match value {
        Some(Value::Number(value)) => value.as_f64(),
        Some(Value::String(value)) => value.trim().parse().ok(),
        _ => None,
    }?;
    (number.is_finite() && number >= 0.0).then_some(number)
}

fn find_lyrics_field(value: &Value) -> Option<String> {
    match value {
        Value::Array(items) => items.iter().find_map(find_lyrics_field),
        Value::Object(fields) => {
            let names: HashSet<&str> = ["lyric", "lyrics", "lyric_text", "lyrics_text"]
                .into_iter()
                .collect();
            for (key, value) in fields {
                if names.contains(key.to_ascii_lowercase().as_str())
                    && let Some(text) = value
                        .as_str()
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                {
                    return Some(text.trim_start_matches('\u{feff}').to_string());
                }
            }
            fields.values().find_map(find_lyrics_field)
        }
        _ => None,
    }
}

fn is_no_lyrics_marker(value: &str) -> bool {
    let lines: Vec<_> = value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if lines.is_empty() {
        return true;
    }
    lines
        .iter()
        .all(|line| line.starts_with('[') && line.ends_with(']'))
        && lines.iter().any(|line| NO_LYRICS_RE.is_match(line))
}

fn extract_rsc_lyrics(record: &ClipRecord) -> String {
    let Some(text_chunk) = TEXT_CHUNK_RE
        .find_iter(&record.page)
        .take_while(|item| item.start() < record.chunk_start)
        .last()
    else {
        return String::new();
    };
    let lyrics = record.page[text_chunk.end()..record.chunk_start].trim();
    if is_no_lyrics_marker(lyrics) {
        String::new()
    } else {
        lyrics.to_string()
    }
}

fn extract_title(page: &str) -> String {
    if let Some(value) = TITLE_RE.captures(page).and_then(|capture| capture.get(1)) {
        return html_escape::decode_html_entities(unescape_field(value.as_str()).trim())
            .to_string();
    }
    for expression in [&*OG_TITLE_RE, &*OG_TITLE_REVERSED_RE] {
        if let Some(value) = expression.captures(page).and_then(|capture| capture.get(1)) {
            return html_escape::decode_html_entities(value.as_str().trim()).to_string();
        }
    }
    HTML_TITLE_RE
        .captures(page)
        .and_then(|capture| capture.get(1))
        .map(|value| {
            let decoded = html_escape::decode_html_entities(value.as_str()).to_string();
            let suffix =
                Regex::new(r"(?i)\s+by\s+.*?\s+\|\s*Suno\s*$").expect("valid Suno title suffix");
            suffix.replace(decoded.trim(), "").trim().to_string()
        })
        .unwrap_or_default()
}

fn extract_clip_id(page: &str) -> String {
    let marker = page.find("\"entity_type\":\"song_schema\"");
    let region = if let Some(marker) = marker {
        &page[marker.saturating_sub(2500)..(marker + 200).min(page.len())]
    } else {
        page
    };
    let expression =
        Regex::new(r#"(?i)"id"\s*:\s*"([0-9a-f-]{36})""#).expect("valid clip ID expression");
    expression
        .captures(region)
        .and_then(|capture| capture.get(1))
        .map(|value| value.as_str().to_ascii_lowercase())
        .filter(|value| UUID_RE.is_match(value))
        .unwrap_or_default()
}

pub fn parse_song_page(page_text: &str, source_url: &str) -> Result<Song> {
    if page_text.is_empty() {
        bail!("Suno 歌曲页面为空");
    }
    let page_url = Url::parse(source_url).context("Suno 页面地址无效")?;
    let page_info = validate_page_url(&page_url)?;
    let id_from_url = page_info.id.context("歌曲链接缺少 ID")?;
    let record = extract_clip_record(page_text, Some(&id_from_url));
    let page = record
        .as_ref()
        .map(|value| value.page.clone())
        .unwrap_or_else(|| decode_rsc_text(page_text));
    let embedded_id = record
        .as_ref()
        .and_then(|value| value.clip.get("id"))
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| extract_clip_id(&page));
    if !embedded_id.is_empty() && embedded_id != id_from_url {
        bail!("页面歌曲 ID 与链接不一致");
    }
    let id = if embedded_id.is_empty() {
        id_from_url
    } else {
        embedded_id
    };
    let clip = record.as_ref().map(|value| &value.clip);
    let title = clip
        .and_then(|value| nullable_string(value.get("title")))
        .unwrap_or_else(|| extract_title(&page));
    if title.is_empty() {
        bail!("无法读取歌曲名称，页面可能需要登录或链接已失效");
    }
    let explicit_lyrics = clip.and_then(find_lyrics_field).unwrap_or_default();
    let lyrics = if explicit_lyrics.is_empty() {
        record.as_ref().map(extract_rsc_lyrics).unwrap_or_default()
    } else {
        explicit_lyrics
    };
    let media_url = clip
        .and_then(|value| value.get("media_urls"))
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find_map(|item| item.get("url").and_then(Value::as_str))
        })
        .map(String::from)
        .or_else(|| {
            MEDIA_RE
                .captures(&page)
                .and_then(|capture| capture.get(1))
                .map(|value| unescape_field(value.as_str()))
        })
        .context("无法读取歌曲音频地址")?;
    validate_media_url(&media_url, &id)?;
    let duration = clip
        .and_then(|value| value.get("metadata"))
        .and_then(|value| nullable_number(value.get("duration")))
        .or_else(|| {
            DURATION_RE
                .captures(&page)
                .and_then(|capture| capture.get(1))
                .and_then(|value| value.as_str().parse().ok())
        });
    Ok(Song {
        id,
        title,
        lyrics,
        media_url,
        source_url: page_url.to_string(),
        duration,
        author: clip.and_then(|value| {
            nullable_string(value.get("display_name"))
                .or_else(|| nullable_string(value.get("user_display_name")))
        }),
        published_at: clip.and_then(|value| nullable_string(value.get("created_at"))),
        play_count: clip.and_then(|value| nullable_number(value.get("play_count"))),
        like_count: clip.and_then(|value| {
            nullable_number(value.get("upvote_count"))
                .or_else(|| nullable_number(value.get("like_count")))
        }),
        share_count: clip.and_then(|value| {
            nullable_number(value.get("share_count"))
                .or_else(|| nullable_number(value.get("shares_count")))
        }),
    })
}

async fn read_limited(
    response: reqwest::Response,
    max_bytes: usize,
    activity: &str,
    progress: Option<&(dyn Fn(u64, u64) + Send + Sync)>,
) -> Result<Vec<u8>> {
    let total = response.content_length().unwrap_or_default();
    if total > max_bytes as u64 {
        bail!("下载文件过大，已停止");
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::with_capacity(total.min(max_bytes as u64) as usize);
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| request_error(activity, error))?;
        if bytes.len() + chunk.len() > max_bytes {
            bail!("下载文件过大，已停止");
        }
        bytes.extend_from_slice(&chunk);
        if let Some(callback) = progress {
            callback(bytes.len() as u64, total);
        }
    }
    Ok(bytes)
}

fn request_error(activity: &str, error: reqwest::Error) -> anyhow::Error {
    if error.is_timeout() {
        anyhow!("{activity}超时。{SUNO_ACCESS_HINT}")
    } else if error.is_connect() {
        anyhow!("无法连接 Suno，{activity}失败。{SUNO_ACCESS_HINT}")
    } else if error.is_decode() {
        anyhow!("Suno 返回了无法识别的数据，平台接口可能已经更新，请稍后重试。")
    } else {
        anyhow!("{activity}失败，网络连接可能已中断。{SUNO_ACCESS_HINT}")
    }
}

fn status_error(subject: &str, status: StatusCode) -> String {
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => format!(
            "{subject}无法访问，歌曲可能未公开、已受限，或当前网络无法正常访问 Suno。{SUNO_ACCESS_HINT}"
        ),
        StatusCode::NOT_FOUND => {
            format!("{subject}不存在，歌曲可能已删除、链接已失效或未公开。")
        }
        StatusCode::TOO_MANY_REQUESTS => "Suno 请求过于频繁，请稍等几分钟后再试。".to_string(),
        status if status.is_server_error() => {
            format!(
                "Suno 服务暂时异常（HTTP {}），请稍后重试。",
                status.as_u16()
            )
        }
        _ => format!(
            "{subject}返回异常（HTTP {}），请稍后重试。",
            status.as_u16()
        ),
    }
}

fn client(timeout_seconds: u64) -> Result<Client> {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(timeout_seconds))
        .build()
        .context("无法初始化网络请求")
}

async fn fetch_song_page(input: &str) -> Result<(String, String)> {
    let parsed = parse_suno_url(input)?;
    let original_id = parsed.id.clone();
    let mut current = parsed.url;
    let client = client(30)?;
    for _ in 0..=MAX_REDIRECTS {
        let response = client
            .get(current.clone())
            .header(header::ACCEPT, "text/html,application/xhtml+xml")
            .send()
            .await
            .map_err(|error| request_error("读取歌曲页面", error))?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(header::LOCATION)
                .context("Suno 页面跳转地址缺失")?
                .to_str()
                .context("Suno 页面跳转地址无效")?;
            let next = current.join(location).context("Suno 页面跳转地址无效")?;
            validate_page_url(&next)?;
            current = next;
            continue;
        }
        if !response.status().is_success() {
            bail!(status_error("Suno 歌曲页面", response.status()));
        }
        let page_info = validate_page_url(&current)?;
        if let Some(original_id) = &original_id
            && page_info.id.as_deref() != Some(original_id)
        {
            bail!("Suno 页面歌曲 ID 与链接不一致");
        }
        let page =
            String::from_utf8(read_limited(response, MAX_PAGE_BYTES, "读取歌曲页面", None).await?)
                .context("Suno 页面不是有效的 UTF-8 文本")?;
        return Ok((page, current.to_string()));
    }
    bail!("Suno 页面跳转次数过多")
}

pub async fn load_song(input: &str) -> Result<Song> {
    let (page, source_url) = fetch_song_page(input).await?;
    parse_song_page(&page, &source_url)
}

pub fn validate_cached_song(song: &Song) -> Result<()> {
    if !UUID_RE.is_match(&song.id) || song.title.trim().is_empty() {
        bail!("历史记录中的歌曲信息无效");
    }
    validate_media_url(&song.media_url, &song.id)?;
    let source_url = Url::parse(&song.source_url).context("历史记录中的歌曲链接无效")?;
    let page = validate_page_url(&source_url)?;
    if page.id.as_deref() != Some(song.id.as_str()) {
        bail!("历史记录中的歌曲链接与歌曲 ID 不一致");
    }
    Ok(())
}

async fn get_audio_rights(client: &Client, id: &str) -> Result<Rights> {
    let url = validate_rights_url(RIGHTS_URL)?;
    let response = client
        .post(url)
        .header(header::ACCEPT, "application/json")
        .json(
            &serde_json::json!({ "content_params": { "content_id": id, "content_type": "clip" } }),
        )
        .send()
        .await
        .map_err(|error| request_error("获取音频授权", error))?;
    if response.status().is_redirection() {
        bail!("音频授权服务跳转到了不受支持的地址");
    }
    if !response.status().is_success() {
        bail!(status_error("Suno 音频授权", response.status()));
    }
    response
        .json()
        .await
        .map_err(|error| request_error("读取音频授权", error))
}

fn decode_base64(value: &str, name: &str) -> Result<Vec<u8>> {
    if value.is_empty() {
        bail!("Suno 音频授权缺少有效的 {name}");
    }
    BASE64
        .decode(value)
        .with_context(|| format!("Suno 音频授权中的 {name} 无效"))
}

fn aes_gcm_unwrap(value: &str, aad: &str, key: &[u8]) -> Result<Vec<u8>> {
    let raw = decode_base64(value, "密钥数据")?;
    if raw.len() < 29 {
        bail!("Suno 音频授权数据长度不正确");
    }
    let nonce = GenericArray::from_slice(&raw[..12]);
    let tag_start = raw.len() - 16;
    let tag = GenericArray::from_slice(&raw[tag_start..]);
    let mut clear = raw[12..tag_start].to_vec();
    let cipher = Aes256Gcm::new_from_slice(key).context("Suno 音频授权密钥长度不正确")?;
    cipher
        .decrypt_in_place_detached(nonce, aad.as_bytes(), &mut clear, tag)
        .map_err(|_| anyhow!("Suno 音频授权校验失败"))?;
    Ok(clear)
}

fn decrypt_audio(encrypted: &[u8], id: &str, rights: &Rights) -> Result<Vec<u8>> {
    let glt_key = Sha256::digest(rights.glt.as_bytes());
    let key = aes_gcm_unwrap(&rights.key, id, &glt_key)?;
    let iv = aes_gcm_unwrap(&rights.iv, id, &glt_key)?;
    if iv.len() < 16 {
        bail!("音频授权的解密参数不正确");
    }
    let mut clear = encrypted.to_vec();
    match key.len() {
        16 => ctr::Ctr128BE::<Aes128>::new_from_slices(&key, &iv[..16])
            .context("音频授权的解密参数不正确")?
            .apply_keystream(&mut clear),
        32 => ctr::Ctr128BE::<Aes256>::new_from_slices(&key, &iv[..16])
            .context("音频授权的解密参数不正确")?
            .apply_keystream(&mut clear),
        _ => bail!("音频授权的解密参数不正确"),
    }
    if clear.len() < 12 || clear.get(4..8) != Some(b"ftyp") {
        bail!("解密后的文件不是有效的 M4A 音频");
    }
    Ok(clear)
}

pub async fn load_audio(
    song: &Song,
    progress: &(dyn Fn(Progress) + Send + Sync),
) -> Result<Vec<u8>> {
    if !UUID_RE.is_match(&song.id) {
        bail!("歌曲信息中的 ID 无效");
    }
    let media_url = validate_media_url(&song.media_url, &song.id)?;
    progress(Progress {
        phase: "授权".into(),
        message: "正在获取音频授权…".into(),
        received: 0,
        total: 0,
        percent: 5.0,
        format: None,
    });
    let client = client(120)?;
    let rights = get_audio_rights(&client, &song.id).await?;
    progress(Progress {
        phase: "下载音频".into(),
        message: "正在下载音频…".into(),
        received: 0,
        total: 0,
        percent: 10.0,
        format: None,
    });
    let mut current = media_url;
    let mut response = None;
    for _ in 0..=MAX_REDIRECTS {
        let fetched = client
            .get(current.clone())
            .header(header::ACCEPT, "audio/mp4,application/octet-stream")
            .send()
            .await
            .map_err(|error| request_error("下载歌曲音频", error))?;
        if fetched.status().is_redirection() {
            let location = fetched
                .headers()
                .get(header::LOCATION)
                .context("音频跳转地址缺失")?
                .to_str()
                .context("音频跳转地址无效")?;
            let next = current.join(location).context("音频跳转地址无效")?;
            current = validate_media_url(next.as_str(), &song.id)?;
            continue;
        }
        response = Some(fetched);
        break;
    }
    let response = response.context("音频跳转次数过多")?;
    if response.status() != StatusCode::OK {
        bail!(status_error("Suno 歌曲音频", response.status()));
    }
    let callback = |received: u64, total: u64| {
        let percent = if total > 0 {
            10.0 + ((received as f64 / total as f64) * 55.0).min(55.0)
        } else {
            20.0
        };
        progress(Progress {
            phase: "下载音频".into(),
            message: format!("正在下载音频… {}%", percent.round()),
            received,
            total,
            percent,
            format: None,
        });
    };
    let encrypted =
        read_limited(response, MAX_AUDIO_BYTES, "下载歌曲音频", Some(&callback)).await?;
    progress(Progress {
        phase: "解密音频".into(),
        message: "正在处理音频…".into(),
        received: encrypted.len() as u64,
        total: encrypted.len() as u64,
        percent: 70.0,
        format: None,
    });
    decrypt_audio(&encrypted, &song.id, &rights)
}

pub fn safe_filename(value: &str, fallback: &str) -> String {
    let forbidden = Regex::new(r#"[<>:"/\\|?*\x00-\x1f]"#).expect("valid filename expression");
    let invisible = Regex::new(r"[\u{200b}\u{200c}\u{200d}\u{feff}]")
        .expect("valid invisible filename expression");
    let spaces = Regex::new(r"\s+").expect("valid whitespace expression");
    let mut name = forbidden.replace_all(value, "_").to_string();
    name = invisible.replace_all(&name, "").to_string();
    name = spaces
        .replace_all(name.trim(), " ")
        .trim_end_matches(['.', ' '])
        .chars()
        .take(180)
        .collect::<String>()
        .trim()
        .to_string();
    if name.is_empty() {
        name = fallback.to_string();
    }
    let lower = name.to_ascii_lowercase();
    let stem = lower.split('.').next().unwrap_or_default();
    let reserved = matches!(stem, "con" | "prn" | "aux" | "nul" | "clock$")
        || (stem.len() == 4
            && (stem.starts_with("com") || stem.starts_with("lpt"))
            && stem[3..]
                .parse::<u8>()
                .is_ok_and(|number| (1..=9).contains(&number)));
    if reserved {
        name.insert(0, '_');
    }
    if name.is_empty() {
        fallback.to_string()
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "107aaa5e-3007-4c7c-ab23-f65b19f81c15";

    fn fixture() -> String {
        format!(
            "<html><head><meta property=\"og:title\" content=\"备用歌名\"></head><body>\n51:Tabc,[Intro]\n第一句中文\n第二句中文\n[Outro]40:[\"$\",\"$L50\",null,{{\"clip\":{{\"title\":\"测试歌曲\",\"id\":\"{ID}\",\"entity_type\":\"song_schema\",\"media_urls\":[{{\"url\":\"https://d2lwuy8qc234o3.cloudfront.net/1/clip/{ID}.m4a\"}}],\"metadata\":{{\"duration\":12.5}}}}}}]</body></html>"
        )
    }

    fn english_fixture() -> String {
        let id = "e9548485-9bf8-46bc-ace1-83b5e21f872c";
        let clip = serde_json::json!({
            "clip": {
                "status": "complete",
                "title": "Dancing With My Eyes Closed",
                "play_count": 187570,
                "upvote_count": 2197,
                "share_count": null,
                "id": id,
                "entity_type": "song_schema",
                "media_urls": [{ "url": format!("https://d2lwuy8qc234o3.cloudfront.net/1/clip/{id}.m4a") }],
                "metadata": { "duration": 186.4 },
                "display_name": "Raymond",
                "created_at": "2026-09-09T18:08:55.399Z"
            }
        });
        let lyrics = "[Intro]\n[Verse 1]\nStrobe lights flashing like a thunderstorm\nDancing with my eyes closed!\n(Oh-oh-oh)";
        format!(
            "<html><body>\n12:Tconfig,{{\"display_name\":\"V6\",\"light\":{{\"text_color\":\"FD429C\"}}}}\n3f:Tbody,{lyrics}3f:[\"$\",\"$L4f\",null,{clip}]</body></html>"
        )
    }

    #[test]
    fn parses_strict_links() {
        assert_eq!(
            parse_suno_url("https://suno.com/s/M4txVv6fKRJnD6tO")
                .expect("share link")
                .kind,
            LinkKind::Share
        );
        assert_eq!(
            parse_suno_url(&format!("https://suno.com/song/{ID}?sh=M4txVv6fKRJnD6tO"))
                .expect("song link")
                .id
                .as_deref(),
            Some(ID)
        );
        assert!(parse_suno_url("http://suno.com/s/M4txVv6fKRJnD6tO").is_err());
        assert!(parse_suno_url("https://evil.example/s/M4txVv6fKRJnD6tO").is_err());
        assert!(parse_suno_url("https://suno.com/s/short").is_err());
    }

    #[test]
    fn parses_chinese_song_page() {
        let song = parse_song_page(&fixture(), &format!("https://suno.com/song/{ID}"))
            .expect("song fixture");
        assert_eq!(song.title, "测试歌曲");
        assert_eq!(song.lyrics, "[Intro]\n第一句中文\n第二句中文\n[Outro]");
        assert_eq!(song.duration, Some(12.5));
        assert!(validate_cached_song(&song).is_ok());
    }

    #[test]
    fn rejects_cached_song_with_mismatched_source_or_media_url() {
        let song = parse_song_page(&fixture(), &format!("https://suno.com/song/{ID}"))
            .expect("song fixture");
        let mut bad_source = song.clone();
        bad_source.source_url = "https://evil.example/song".into();
        assert!(validate_cached_song(&bad_source).is_err());

        let mut bad_media = song;
        bad_media.media_url = "https://evil.example/audio.m4a".into();
        assert!(validate_cached_song(&bad_media).is_err());
    }

    #[test]
    fn parses_public_metadata_without_leaking_page_configuration() {
        let id = "e9548485-9bf8-46bc-ace1-83b5e21f872c";
        let song = parse_song_page(&english_fixture(), &format!("https://suno.com/song/{id}"))
            .expect("English song fixture");
        assert_eq!(song.title, "Dancing With My Eyes Closed");
        assert!(song.lyrics.contains("Dancing with my eyes closed!"));
        assert!(!song.lyrics.contains("text_color"));
        assert_eq!(song.author.as_deref(), Some("Raymond"));
        assert_eq!(song.play_count, Some(187570.0));
        assert_eq!(song.like_count, Some(2197.0));
        assert_eq!(song.share_count, None);
    }

    #[test]
    fn explicit_lyrics_take_priority_and_instrumental_marker_is_empty() {
        let explicit = fixture().replace(
            "\"clip\":{\"title\"",
            "\"clip\":{\"lyrics\":\"[Verse]\\n正式第一行\\n正式第二行\",\"title\"",
        );
        let song = parse_song_page(&explicit, &format!("https://suno.com/song/{ID}"))
            .expect("explicit lyrics");
        assert_eq!(song.lyrics, "[Verse]\n正式第一行\n正式第二行");

        let instrumental = fixture().replace(
            "51:Tabc,[Intro]\n第一句中文\n第二句中文",
            "51:Tabc,[Instrumental]\n",
        );
        let song = parse_song_page(&instrumental, &format!("https://suno.com/song/{ID}"))
            .expect("instrumental fixture");
        assert!(song.lyrics.is_empty());
    }

    #[test]
    fn rejects_media_url_that_does_not_match_song() {
        assert!(validate_media_url("https://evil.example/audio.m4a", ID).is_err());
        assert!(validate_media_url(
            "https://d2lwuy8qc234o3.cloudfront.net/1/clip/00000000-0000-4000-8000-000000000000.m4a",
            ID,
        )
        .is_err());
    }

    #[test]
    fn sanitizes_cross_platform_filenames() {
        assert_eq!(
            safe_filename("  测试:/歌曲*?  ", "fallback"),
            "测试__歌曲__"
        );
        assert_eq!(safe_filename("CON", "fallback"), "_CON");
        assert_eq!(safe_filename("...   ", "fallback"), "fallback");
    }

    #[test]
    fn explains_common_suno_network_and_service_errors() {
        let forbidden = status_error("Suno 歌曲页面", StatusCode::FORBIDDEN);
        assert!(forbidden.contains("浏览器确认"));
        assert!(forbidden.contains("https://suno.com"));
        assert!(forbidden.contains("代理设置"));

        assert!(status_error("Suno 歌曲页面", StatusCode::NOT_FOUND).contains("链接已失效"));
        assert!(
            status_error("Suno 歌曲页面", StatusCode::TOO_MANY_REQUESTS).contains("请求过于频繁")
        );
        assert!(
            status_error("Suno 歌曲页面", StatusCode::INTERNAL_SERVER_ERROR)
                .contains("服务暂时异常")
        );
    }

    #[test]
    fn connection_failures_tell_user_how_to_check_suno_access() {
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(1))
            .build()
            .expect("test HTTP client");
        let error = tauri::async_runtime::block_on(async {
            client
                .get("http://127.0.0.1:0")
                .send()
                .await
                .expect_err("local port zero must reject connections")
        });
        let message = request_error("读取歌曲页面", error).to_string();
        assert!(message.contains("无法连接"));
        assert!(message.contains("浏览器确认"));
        assert!(message.contains("代理设置"));
    }

    #[test]
    #[ignore = "需要访问 Suno 真实服务"]
    fn live_song_page_uses_system_network_settings() {
        let url = std::env::var("SONGKEEP_TEST_URL")
            .expect("set SONGKEEP_TEST_URL to a public Suno song or share URL");
        let song = tauri::async_runtime::block_on(load_song(&url))
            .expect("read song metadata using the application's HTTP client");
        assert!(UUID_RE.is_match(&song.id));
        assert!(!song.title.trim().is_empty());
        validate_media_url(&song.media_url, &song.id).expect("valid song media URL");
    }

    #[test]
    #[ignore = "需要访问 Suno 真实服务"]
    fn live_song_can_be_read_decrypted_and_converted() {
        let song = tauri::async_runtime::block_on(load_song("https://suno.com/s/M4txVv6fKRJnD6tO"))
            .expect("live song metadata");
        assert_eq!(song.id, ID);
        assert!(song.lyrics.contains("今夜嘅廣州"));
        let clear = tauri::async_runtime::block_on(load_audio(&song, &|_| {}))
            .expect("live audio download and decrypt");
        assert_eq!(clear.get(4..8), Some(b"ftyp".as_slice()));
        let mp3 = crate::audio::m4a_to_mp3(&clear).expect("live MP3 conversion");
        assert!(mp3.len() > 100_000);
        assert_eq!(mp3[0], 0xff);
        assert_eq!(mp3[1] & 0xe0, 0xe0);

        let output_path =
            std::env::temp_dir().join(format!("songkeep-live-{}.mp3", uuid::Uuid::new_v4()));
        std::fs::write(&output_path, &mp3).expect("write temporary live MP3");
        let probe = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
            ])
            .arg(&output_path)
            .output();
        let _ = std::fs::remove_file(&output_path);
        let probe = probe.expect("ffprobe must be installed for the ignored live test");
        assert!(probe.status.success());
        let duration: f64 = String::from_utf8(probe.stdout)
            .expect("ffprobe UTF-8 output")
            .trim()
            .parse()
            .expect("ffprobe duration");
        assert!(duration > 60.0);
    }
}

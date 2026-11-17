//! Talking to an Emby server over its HTTP API.
//!
//! The original program addresses the same endpoints (`/emby/System/Info/Public`,
//! `/emby/Users/AuthenticateByName`, `/Views`, `/Items`, `/Items/Resume`,
//! `/Shows/NextUp`, `/Sessions/Playing/…`), signs in with the same
//! `Emby Client="…"` authorization value and sends the session token in
//! `X-Emby-Token`. See MECHANICS.md.

use crate::model::*;
use crate::preferences::{Network, ProxyMode, TextRules};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use std::sync::{Mutex, OnceLock, RwLock};

pub const APP_NAME: &str = "Media Lite";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEVICE: &str = "Windows";
/// How many items a grid asks for at a time.
pub const PAGE: i64 = 60;
/// How many items a home row shows.
pub const ROW: i64 = 24;

static NETWORK: OnceLock<RwLock<Network>> = OnceLock::new();

pub fn configure_network(network: Network) {
    *NETWORK.get_or_init(Default::default).write().unwrap() = network;
}

pub fn network() -> Network {
    NETWORK.get_or_init(Default::default).read().unwrap().clone()
}

#[derive(PartialEq)]
struct HttpRoute {
    network: Network,
    system_proxies: [Option<String>; 2],
}

impl HttpRoute {
    fn current(force_direct: bool) -> Self {
        let mut network = network();
        if force_direct {
            network.proxy = ProxyMode::Direct;
        }
        if network.proxy == ProxyMode::Direct {
            network.proxy_address.clear();
        }
        let system_proxies = if network.proxy == ProxyMode::System {
            [crate::platform::system_proxy(false), crate::platform::system_proxy(true)]
        } else {
            [None, None]
        };
        Self { network, system_proxies }
    }

    fn builder(&self) -> Result<reqwest::blocking::ClientBuilder, String> {
        let mut builder = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(20))
            .danger_accept_invalid_certs(self.network.ignore_ssl)
            .user_agent(format!("{}/{}", APP_NAME, APP_VERSION));
        match self.network.proxy {
            ProxyMode::Direct => builder = builder.no_proxy(),
            ProxyMode::Http => {
                let proxy = reqwest::Proxy::all(&self.network.proxy_address)
                    .map_err(|_| "Invalid HTTP proxy address".to_owned())?;
                builder = builder.proxy(proxy);
            }
            ProxyMode::System => {
                let bypass = reqwest::NoProxy::from_string(&format!(
                    "localhost,127.0.0.1,::1,{}", std::env::var("NO_PROXY").unwrap_or_default(),
                ));
                for (index, value) in self.system_proxies.iter().enumerate() {
                    if let Some(value) = value {
                        let proxy = if index == 0 { reqwest::Proxy::http(value) } else { reqwest::Proxy::https(value) };
                        if let Ok(proxy) = proxy {
                            builder = builder.proxy(proxy.no_proxy(bypass.clone()));
                        }
                    }
                }
            }
        }
        Ok(builder)
    }
}

fn client() -> Result<reqwest::blocking::Client, String> {
    client_for(false)
}

fn client_for(direct: bool) -> Result<reqwest::blocking::Client, String> {
    static CLIENT: Mutex<Option<(HttpRoute, reqwest::blocking::Client)>> = Mutex::new(None);
    static DIRECT_CLIENT: Mutex<Option<(HttpRoute, reqwest::blocking::Client)>> = Mutex::new(None);
    let route = HttpRoute::current(direct);
    let pool = if route.network.proxy == ProxyMode::Direct { &DIRECT_CLIENT } else { &CLIENT };
    let mut cached = pool.lock().unwrap();
    if let Some((_, client)) = cached.as_ref().filter(|(active, _)| *active == route) {
        return Ok(client.clone());
    }
    let client = route.builder()?
        .build()
        .map_err(|_| "Could not initialize HTTP".to_owned())?;
    *cached = Some((route, client.clone()));
    Ok(client)
}

pub(crate) fn release_client() -> Result<reqwest::blocking::Client, String> {
    HttpRoute::current(false).builder()?
        .danger_accept_invalid_certs(false)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| "Could not initialize GitHub HTTP".to_owned())
}

pub fn transfer_backup(url: reqwest::Url, username: String, password: String, data: Option<Vec<u8>>) -> Result<Option<Vec<u8>>, String> {
    let client = HttpRoute::current(false).builder()?.redirect(reqwest::redirect::Policy::none())
        .build().map_err(|_| "Could not initialize WebDAV HTTP".to_owned())?;
    let upload = data.is_some();
    let request = match data {
        Some(data) => client.put(url).header(reqwest::header::CONTENT_TYPE, "application/json").body(data),
        None => client.get(url),
    };
    let request = if username.is_empty() { request } else { request.basic_auth(username, Some(password)) };
    let response = request.send().map_err(|error| friendly(&error))?;
    if !response.status().is_success() {
        return Err(format!("WebDAV HTTP {}", response.status().as_u16()));
    }
    if upload { return Ok(None); }
    use std::io::Read;
    const LIMIT: u64 = 16 * 1024 * 1024;
    let mut bytes = Vec::new();
    response.take(LIMIT + 1).read_to_end(&mut bytes).map_err(|_| "Could not read the WebDAV backup".to_owned())?;
    if bytes.len() as u64 > LIMIT { return Err("WebDAV backup is too large".into()); }
    Ok(Some(bytes))
}

pub fn playback_proxy(url: &str, use_system_proxy: bool) -> Option<String> {
    let prefs = network();
    match prefs.proxy {
        ProxyMode::Direct => None,
        ProxyMode::Http => Some(prefs.proxy_address).filter(|value| !value.is_empty()),
        ProxyMode::System if use_system_proxy => {
            let url = reqwest::Url::parse(url).ok()?;
            if matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")) {
                return None;
            }
            crate::platform::system_proxy(url.scheme() == "https").or_else(|| {
                let proxy = crate::platform::inherited_http_proxy()?.to_str()?.trim();
                if proxy.is_empty() {
                    None
                } else if proxy.contains("://") {
                    Some(proxy.to_owned())
                } else {
                    Some(format!("http://{proxy}"))
                }
            })
        }
        ProxyMode::System => None,
    }
}

/// Turn whatever the person typed into a base URL: `192.168.1.9:8096`,
/// `http://box:8096/emby/` and so on all become `http://box:8096`.
pub fn normalize_url(raw: &str) -> String {
    let mut u = raw.trim().to_string();
    if u.is_empty() {
        return u;
    }
    let lower = u.to_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        u = format!("http://{}", u);
    }
    while u.ends_with('/') {
        u.pop();
    }
    let lower = u.to_lowercase();
    if lower.ends_with("/emby") {
        u.truncate(u.len() - 5);
        while u.ends_with('/') {
            u.pop();
        }
    }
    u
}

pub fn auth_header(device_id: &str) -> String {
    format!(
        "Emby Client=\"{}\", Device=\"{}\", DeviceId=\"{}\", Version=\"{}\"",
        APP_NAME, DEVICE, device_id, APP_VERSION
    )
}

/// The same identity with the token on the end, as older Emby servers expect it
/// in a plain `Authorization` header.
pub fn auth_header_with_token(device_id: &str, token: &str) -> String {
    format!(
        "Emby Client=\"{}\", Device=\"{}\", DeviceId=\"{}\", Version=\"{}\", Token=\"{}\"",
        APP_NAME, DEVICE, device_id, APP_VERSION, token
    )
}

/// A server that answered, signed in and is ready to use.
pub struct Connected {
    pub base: String,
    pub name: String,
    pub version: String,
    pub user_name: String,
    pub session: Session,
}

/// Is this a machine on the local network or on this PC?
fn looks_local(addr: &str) -> bool {
    let host = addr
        .split('/')
        .next()
        .unwrap_or(addr)
        .split(':')
        .next()
        .unwrap_or(addr)
        .to_lowercase();
    if host == "localhost" || host.ends_with(".local") || host.ends_with(".lan") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(ip) => match ip {
            std::net::IpAddr::V4(v4) => v4.is_private() || v4.is_loopback(),
            std::net::IpAddr::V6(v6) => v6.is_loopback(),
        },
        Err(_) => false,
    }
}

/// Sign in, trying the likely address forms one after another.
///
/// People type `box:8096` or `emby.example.com`; the first is plain http and the
/// second nearly always https. Both are tried, so a missing scheme no longer
/// looks like a broken password. Every failure is kept, so the window can say
/// what actually went wrong at each address instead of just "sign-in failed".
pub fn connect(
    raw: &str,
    device_id: &str,
    username: &str,
    password: &str,
) -> Result<Connected, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("Type the server's address first.".to_string());
    }
    let user = username.trim();
    if user.is_empty() {
        return Err("Type the user name to sign in with.".to_string());
    }
    let lowered = raw.to_lowercase();
    let mut candidates: Vec<String> = Vec::new();
    if lowered.starts_with("http://") || lowered.starts_with("https://") {
        candidates.push(normalize_url(raw));
    } else if looks_local(raw) {
        candidates.push(normalize_url(&format!("http://{}", raw)));
        candidates.push(normalize_url(&format!("https://{}", raw)));
    } else {
        candidates.push(normalize_url(&format!("https://{}", raw)));
        candidates.push(normalize_url(&format!("http://{}", raw)));
    }

    let mut problems: Vec<String> = Vec::new();
    for base in candidates {
        match try_server(&base, device_id, user, password) {
            Ok(done) => return Ok(done),
            Err(why) => problems.push(format!("{}: {}", base, why)),
        }
    }
    Err(problems.join(" \u{2022} "))
}

/// Probe one address and sign in there.
fn try_server(
    base: &str,
    device_id: &str,
    user: &str,
    password: &str,
) -> Result<Connected, String> {
    let (name, version) = match public_info(base) {
        Ok((name, version, _id)) => (name, version),
        Err(why) => {
            // A server reachable but not answering under /emby is a different
            // problem from nothing listening at all; say which it is.
            if why.contains("the server answered 404") || why.contains("the server answered 400") {
                return Err(format!(
                    "{} (it answered, but not like an Emby server)",
                    why
                ));
            }
            return Err(why);
        }
    };
    let (session, user_name) = authenticate(base, device_id, user, password)?;
    Ok(Connected {
        base: base.to_string(),
        name,
        version,
        user_name,
        session,
    })
}

/// Everything this user has marked as a favourite.
pub fn favorites(base: &str, session: &Session) -> Result<Vec<MediaItem>, String> {
    let params = vec![
        ("Recursive".to_string(), "true".to_string()),
        ("IsFavorite".to_string(), "true".to_string()),
        (
            "IncludeItemTypes".to_string(),
            "Movie,Series,Episode,Video,BoxSet".to_string(),
        ),
        ("SortBy".to_string(), "SortName".to_string()),
        ("SortOrder".to_string(), "Ascending".to_string()),
        ("Limit".to_string(), PAGE.to_string()),
    ];
    let v = items_query(base, session, &params)?;
    Ok(items_from(&v))
}

fn get_json(base: &str, token: &str, path_and_query: &str) -> Result<Value, String> {
    let url = format!("{}{}", base, path_and_query);
    let mut req = client()?.get(&url);
    if !token.is_empty() {
        req = req.header("X-Emby-Token", token);
    }
    let resp = req.send().map_err(|e| friendly(&e))?;
    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "the server answered {} {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("")
        ));
    }
    serde_json::from_str(&text).map_err(|_| "the server sent something that is not JSON".to_string())
}

fn post_json(
    client: &reqwest::blocking::Client,
    base: &str,
    token: &str,
    auth: Option<&str>,
    path: &str,
    body: &Value,
) -> Result<Value, String> {
    let url = format!("{}{}", base, path);
    let mut req = client
        .post(&url)
        .header("Content-Type", "application/json")
        .body(body.to_string());
    if let Some(a) = auth {
        req = req.header("X-Emby-Authorization", a).header("Authorization", a);
    }
    if !token.is_empty() {
        req = req.header("X-Emby-Token", token);
    }
    let resp = req.send().map_err(|e| friendly(&e))?;
    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    if !status.is_success() {
        return Err(format!("the server answered {}", status.as_u16()));
    }
    if text.trim().is_empty() {
        Ok(Value::Null)
    } else {
        serde_json::from_str(&text).unwrap_or(Value::Null).pipe_ok()
    }
}

fn delete_empty(base: &str, token: &str, path: &str) -> Result<Value, String> {
    let url = format!("{}{}", base, path);
    let req = client()?.delete(&url).header("X-Emby-Token", token);
    let resp = req.send().map_err(|e| friendly(&e))?;
    if !resp.status().is_success() {
        return Err(format!("the server answered {}", resp.status().as_u16()));
    }
    Ok(Value::Null)
}

trait PipeOk {
    fn pipe_ok(self) -> Result<Value, String>;
}
impl PipeOk for Value {
    fn pipe_ok(self) -> Result<Value, String> {
        Ok(self)
    }
}

fn friendly(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "the server took too long to answer".to_string()
    } else if e.is_connect() {
        "could not reach the server — check the address and that the server is running".to_string()
    } else if e.is_decode() {
        "the answer from the server could not be read".to_string()
    } else {
        format!("network error: {e}")
    }
}

// ---------------------------------------------------------------- parsing

fn s(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string()
}

fn i(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(|x| x.as_i64())
}

fn f(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(|x| x.as_f64())
}

fn b(v: &Value, key: &str) -> bool {
    v.get(key).and_then(|x| x.as_bool()).unwrap_or(false)
}

fn str_list(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|y| y.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

pub fn item_from(v: &Value) -> MediaItem {
    let user = v.get("UserData").cloned().unwrap_or(Value::Null);
    let image_tags = v.get("ImageTags").cloned().unwrap_or(Value::Null);
    // An episode or season rarely carries its own Primary tag; the server puts
    // the series' poster in SeriesPrimaryImageTag instead. Without this fallback
    // most rows come out as blank placeholder tiles.
    let own_primary = image_tags
        .get("Primary")
        .and_then(|x| x.as_str())
        .map(|x| x.to_string());
    let parent_primary = v
        .get("SeriesPrimaryImageTag")
        .or_else(|| v.get("ParentPrimaryImageTag"))
        .and_then(|x| x.as_str())
        .map(|x| x.to_string());
    let (image_tag, image_id) = match own_primary {
        Some(t) => (Some(t), None),
        None => match parent_primary {
            Some(t) => {
                let owner = ["SeriesId", "ParentId", "SeasonId"]
                    .iter()
                    .map(|k| s(v, k))
                    .find(|x| !x.is_empty());
                (Some(t), owner)
            }
            None => (None, None),
        },
    };
    let own_backdrop = v
        .get("BackdropImageTags")
        .and_then(|x| x.as_array())
        .and_then(|a| a.first())
        .and_then(|x| x.as_str())
        .map(|x| x.to_string());
    let parent_backdrop = v
        .get("ParentBackdropImageTags")
        .and_then(|x| x.as_array())
        .and_then(|a| a.first())
        .and_then(|x| x.as_str())
        .map(|x| x.to_string());
    let (backdrops, backdrop_id) = match own_backdrop {
        Some(t) => (Some(t), None),
        None => match parent_backdrop {
            Some(t) => {
                let owner = ["ParentBackdropItemId", "SeriesId", "ParentId"]
                    .iter()
                    .map(|k| s(v, k))
                    .find(|x| !x.is_empty());
                (Some(t), owner)
            }
            None => (None, None),
        },
    };
    let series_id = v
        .get("SeriesId")
        .and_then(|x| x.as_str())
        .map(|x| x.to_string());
    let season_id = v
        .get("SeasonId")
        .and_then(|x| x.as_str())
        .map(|x| x.to_string());
    let source = v.pointer("/MediaSources/0").unwrap_or(v);
    let mut technical = Vec::new();
    if let Some(video) = source.get("MediaStreams").and_then(Value::as_array)
        .and_then(|streams| streams.iter().find(|stream| stream.get("Type").and_then(Value::as_str) == Some("Video")))
    {
        let profile = s(video, "VideoRangeType").to_ascii_lowercase();
        if profile.contains("dovi") || profile.contains("dolby") || i(video, "DvProfile").is_some_and(|p| p > 0) {
            technical.push("DoVi".to_owned());
        } else if profile.contains("hdr") || s(video, "VideoRange").eq_ignore_ascii_case("hdr") {
            technical.push("HDR".to_owned());
        }
        if let Some(height) = i(video, "Height").filter(|value| *value > 0) {
            technical.push(format!("{height}p"));
        }
    }
    if !technical.is_empty() {
        if let Some(bitrate) = i(source, "Bitrate").filter(|rate| *rate > 0) {
            technical.push(format!("{:.1} Mbps", bitrate as f64 / 1_000_000.0));
        }
    }
    let logo = image_tags.get("Logo").and_then(Value::as_str).filter(|tag| !tag.is_empty())
        .map(|tag| (s(v, "Id"), tag.to_owned()))
        .or_else(|| {
            let owner = v.get("ParentLogoItemId")?.as_str()?;
            let tag = v.get("ParentLogoImageTag")?.as_str()?;
            (!tag.is_empty() && !owner.is_empty()).then(|| (owner.to_owned(), tag.to_owned()))
        });
    MediaItem {
        id: s(v, "Id"),
        name: s(v, "Name"),
        kind: s(v, "Type"),
        series_id,
        season_id,
        season_name: s(v, "SeasonName"),
        index: i(v, "IndexNumber"),
        parent_index: i(v, "ParentIndexNumber"),
        overview: s(v, "Overview"),
        year: i(v, "ProductionYear"),
        runtime_ticks: i(v, "RunTimeTicks"),
        official_rating: s(v, "OfficialRating"),
        community_rating: f(v, "CommunityRating"),
        played: b(&user, "Played"),
        favorite: b(&user, "IsFavorite"),
        progress_ticks: i(&user, "PlaybackPositionTicks").unwrap_or(0),
        image_tag,
        image_id,
        backdrop_tag: backdrops,
        backdrop_id,
        logo,
        series_name: s(v, "SeriesName"),
        genres: str_list(v, "Genres"),
        child_count: i(v, "ChildCount"),
        tagline: s(v, "Tagline"),
        premiere: s(v, "PremiereDate"),
        unplayed: i(&user, "UnplayedItemCount"),
        last_played: s(&user, "LastPlayedDate"),
        technical,
        chapters: chapters_from(v),
    }
}

fn chapters_from(v: &Value) -> Vec<Chapter> {
    v.get("Chapters").and_then(Value::as_array).into_iter().flatten().enumerate()
        .filter_map(|(index, chapter)| {
            let position_ticks = i(chapter, "StartPositionTicks").filter(|ticks| *ticks >= 0)?;
            let index = i(chapter, "ChapterIndex").and_then(|n| usize::try_from(n).ok()).unwrap_or(index);
            Some(Chapter { name: s(chapter, "Name"), position_ticks, marker: s(chapter, "MarkerType"),
                image_tag: chapter.get("ImageTag").and_then(Value::as_str).map(str::to_owned), index })
        }).collect()
}

pub fn chapters(base: &str, session: &Session, item: &str) -> Result<Vec<Chapter>, String> {
    get_json(base, &session.token, &format!("/emby/Users/{}/Items/{}?Fields=Chapters",
        url_encode(&session.user_id), url_encode(item))).map(|data| chapters_from(&data))
}

fn items_from(v: &Value) -> Vec<MediaItem> {
    // A list result wraps its rows in "Items"; a bare array is also accepted.
    let arr = if let Some(a) = v.as_array() {
        a.clone()
    } else {
        v.get("Items")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default()
    };
    arr.iter().map(item_from).collect()
}

// ---------------------------------------------------------------- endpoints

/// Before sign-in: what server is at this address?
pub fn public_info(base: &str) -> Result<(String, String, String), String> {
    let v = get_json(base, "", "/emby/System/Info/Public")?;
    Ok((s(&v, "ServerName"), s(&v, "Version"), s(&v, "Id")))
}

/// Sign in with a user name and password (Emby `AuthenticateByName`).
pub fn authenticate(
    base: &str,
    device_id: &str,
    username: &str,
    password: &str,
) -> Result<(Session, String), String> {
    let body = json!({ "Username": username.trim(), "Pw": password });
    let v = post_json(
        &client()?,
        base,
        "",
        Some(&auth_header(device_id)),
        "/emby/Users/AuthenticateByName",
        &body,
    )
    .map_err(|why| {
        // 401 is the server saying the password is wrong; everything else is kept
        // as it came so the window can show the real reason.
        if why.contains("401") || why.contains("403") {
            "the server did not accept that user name and password".to_string()
        } else {
            why
        }
    })?;
    let token = s(&v, "AccessToken");
    if token.is_empty() {
        return Err("the server did not accept that user name and password".to_string());
    }
    let user = v.get("User").cloned().unwrap_or(Value::Null);
    let user_id = s(&user, "Id");
    let user_name = s(&user, "Name");
    Ok((
        Session {
            token,
            user_id: user_id.clone(),
            user_name: user_name.clone(),
        },
        user_name,
    ))
}

/// The libraries this user can see (`/Users/{id}/Views`).
pub fn views(base: &str, session: &Session) -> Result<Vec<MediaItem>, String> {
    let v = get_json(
        base,
        &session.token,
        &format!("/emby/Users/{}/Views", session.user_id),
    )?;
    Ok(items_from(&v)
        .into_iter()
        .filter(|x| x.kind != "Person")
        .collect())
}

fn items_query(base: &str, session: &Session, params: &[(String, String)]) -> Result<Value, String> {
    items_with_fields(base, session, params, &[])
}

fn items_with_fields(base: &str, session: &Session, params: &[(String, String)], additional: &[&str]) -> Result<Value, String> {
    let mut q: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, url_encode(v)))
        .collect();
    q.push(format!("UserId={}", url_encode(&session.user_id)));
    let mut fields = vec![
        "Overview",
        "Genres",
        "ProductionYear",
        "OfficialRating",
        "CommunityRating",
        "Tagline",
        "PremiereDate",
        // Poster/backdrop tags, including the ones an episode borrows from its
        // series. Without them every list row renders as a placeholder tile.
        "SeriesPrimaryImageTag",
        "ParentPrimaryImageTag",
        "ParentBackdropImageTags",
        "ParentBackdropItemId",
        "PrimaryImageAspectRatio",
        "DateCreated",
        "ChildCount",
    ];
    fields.extend_from_slice(additional);
    q.push(format!("Fields={}", fields.join(",")));
    q.push("ImageTypeLimit=3".to_string());
    q.push("EnableImages=true".to_string());
    q.push("EnableImageTypes=Primary,Backdrop".to_string());
    get_json(
        base,
        &session.token,
        &format!("/emby/Users/{}/Items?{}", session.user_id, q.join("&")),
    )
}

/// The grid behind one library.
pub fn browse(
    base: &str,
    session: &Session,
    parent_id: &str,
    include: &str,
    sort: &str,
) -> Result<(Vec<MediaItem>, i64), String> {
    let (field, order) = match sort {
        "DateCreated" => ("DateCreated", "Descending"),
        "CommunityRating" => ("CommunityRating", "Descending"),
        "PlayCount" => ("PlayCount", "Descending"),
        _ => ("SortName", "Ascending"),
    };
    let mut params = vec![
        ("Recursive".to_string(), "true".to_string()),
        ("ParentId".to_string(), parent_id.to_string()),
        ("SortBy".to_string(), field.to_string()),
        ("SortOrder".to_string(), order.to_string()),
        ("Limit".to_string(), PAGE.to_string()),
        ("StartIndex".to_string(), "0".to_string()),
    ];
    if !include.is_empty() {
        params.push(("IncludeItemTypes".to_string(), include.to_string()));
    }
    let v = items_query(base, session, &params)?;
    let total = i(&v, "TotalRecordCount").unwrap_or(0);
    Ok((items_from(&v), total))
}

pub fn search(base: &str, session: &Session, term: &str, include_media: bool) -> Result<Vec<MediaItem>, String> {
    let params = vec![
        ("SearchTerm".to_string(), term.to_string()),
        ("Recursive".to_string(), "true".to_string()),
        ("Limit".to_string(), PAGE.to_string()),
        ("SortBy".to_string(), "SortName".to_string()),
        (
            "IncludeItemTypes".to_string(),
            "Movie,Series,Episode,BoxSet,Person,Video".to_string(),
        ),
    ];
    let fields: &[&str] = if include_media { &["MediaSources", "MediaStreams"] } else { &[] };
    let v = items_with_fields(base, session, &params, fields)?;
    Ok(items_from(&v))
}

/// The home screen strips: Continue Watching, Next Up, Recently Added.
pub fn home(base: &str, session: &Session, libraries: &[MediaItem]) -> Result<Vec<Row>, String> {
    let uid = &session.user_id;
    let mut rows: Vec<Row> = Vec::new();

    if let Ok(v) = get_json(
        base,
        &session.token,
        &format!(
            "/emby/Users/{}/Items/Resume?Limit={}&MediaTypes=Video&Fields=Overview,ProductionYear,OfficialRating,Genres,SeriesPrimaryImageTag,ParentPrimaryImageTag,ParentBackdropImageTags&EnableImageTypes=Primary,Backdrop,Thumb,Logo",
            uid, ROW
        ),
    ) {
        let items = items_from(&v);
        if !items.is_empty() {
            rows.push(Row {
                title: "Continue Watching".to_string(),
                items,
            });
        }
    }

    if let Ok(v) = get_json(
        base,
        &session.token,
        &format!("/emby/Shows/NextUp?UserId={}&Limit={}&Fields=Overview,ProductionYear,OfficialRating,Genres&EnableImageTypes=Primary,Backdrop,Thumb,Logo", uid, ROW),
    ) {
        let items = items_from(&v);
        if !items.is_empty() {
            rows.push(Row {
                title: "Next Up".to_string(),
                items,
            });
        }
    }

    if let Ok(v) = get_json(
        base,
        &session.token,
        &format!(
            "/emby/Users/{}/Items/Latest?Limit={}&Fields=Overview,ProductionYear,OfficialRating,Genres,SeriesPrimaryImageTag,ParentPrimaryImageTag,ParentBackdropImageTags&EnableImageTypes=Primary,Backdrop,Thumb,Logo",
            uid, ROW
        ),
    ) {
        let items = items_from(&v);
        if !items.is_empty() {
            rows.push(Row {
                title: "Recently Added".to_string(),
                items,
            });
        }
    }

    // One "latest" strip per library, so every library is visible from home.
    for lib in libraries.iter().take(6) {
        let path = format!(
            "/emby/Users/{}/Items/Latest?Limit={}&ParentId={}&Fields=Overview,ProductionYear,OfficialRating,Genres,SeriesPrimaryImageTag,ParentPrimaryImageTag,ParentBackdropImageTags&EnableImageTypes=Primary,Backdrop,Thumb,Logo",
            uid, ROW, lib.id
        );
        if let Ok(v) = get_json(base, &session.token, &path) {
            let items = items_from(&v);
            if !items.is_empty() {
                rows.push(Row {
                    title: lib.name.clone(),
                    items,
                });
            }
        }
    }

    Ok(rows)
}

/// Everything the detail page shows for one item.
pub fn detail(base: &str, session: &Session, id: &str) -> Result<ItemDetail, String> {
    let id = url_encode(id);
    let user_id = url_encode(&session.user_id);
    let v = get_json(
        base,
        &session.token,
        &format!("/emby/Users/{user_id}/Items/{id}?Fields=ProductionYear,EndDate,Status,Genres,Overview,People,Chapters&EnableImages=true&EnableImageTypes=Primary,Backdrop,Thumb,Logo"),
    )?;
    let item = item_from(&v);
    let mut out = ItemDetail {
        logo: item.logo.clone(),
        album: str_list(&v, "BackdropImageTags"),
        end_year: v.get("EndDate").and_then(Value::as_str).and_then(|date| date.get(..4)).and_then(|year| year.parse().ok()),
        continuing: s(&v, "Status").eq_ignore_ascii_case("Continuing"),
        people: v.get("People").and_then(Value::as_array).into_iter().flatten()
            .filter_map(|person| serde_json::from_value(person.clone()).ok()).collect(),
        item,
        ..Default::default()
    };

    match out.item.kind.as_str() {
        "Series" => {
            if let Ok(seasons) = get_json(
                base,
                &session.token,
                &format!("/emby/Shows/{id}/Seasons?UserId={user_id}"),
            ) {
                out.seasons = items_from(&seasons);
            }
            if let Ok(next) = get_json(
                base,
                &session.token,
                &format!("/emby/Shows/NextUp?UserId={user_id}&SeriesId={id}&Limit=1&Fields=Overview"),
            ) {
                out.next_episode = items_from(&next).into_iter()
                    .find(|episode| episode.kind == "Episode" && episode.series_id.as_deref() == Some(&out.item.id));
            }
            let season = out.next_episode.as_ref().and_then(|episode| episode.season_id.as_deref())
                .and_then(|id| out.seasons.iter().find(|season| season.id == id))
                .or_else(|| out.seasons.first());
            out.episodes = episodes_in(base, session, &out.item.id, season.map_or("", |season| &season.id)).unwrap_or_default();
        }
        "Season" => {
            if let Some(series) = &out.item.series_id {
                out.episodes = episodes_in(base, session, series, &out.item.id).unwrap_or_default();
            }
        }
        "BoxSet" | "CollectionFolder" | "Folder" => {
            let params = vec![
                ("ParentId".to_string(), out.item.id.clone()),
                ("SortBy".to_string(), "SortName".to_string()),
                ("Limit".to_string(), PAGE.to_string()),
            ];
            if let Ok(v) = items_query(base, session, &params) {
                out.children = items_from(&v);
            }
        }
        _ => {}
    }

    if let Ok(sv) = get_json(
        base,
        &session.token,
        &format!(
            "/emby/Items/{}/Similar?UserId={}&Limit=12",
            id, user_id
        ),
    ) {
        out.similar = items_from(&sv);
    }

    Ok(out)
}

pub fn episodes_in(
    base: &str,
    session: &Session,
    series_id: &str,
    season_id: &str,
) -> Result<Vec<MediaItem>, String> {
    let season = if season_id.is_empty() { String::new() } else { format!("&SeasonId={}", url_encode(season_id)) };
    let v = get_json(
        base,
        &session.token,
        &format!(
            "/emby/Shows/{}/Episodes?UserId={}{}&Fields=Overview&EnableImages=true&EnableImageTypes=Primary,Backdrop,Thumb",
            url_encode(series_id), url_encode(&session.user_id), season,
        ),
    )?;
    Ok(items_from(&v))
}

pub fn set_played(
    base: &str,
    session: &Session,
    id: &str,
    played: bool,
) -> Result<(), String> {
    let path = format!("/emby/Users/{}/PlayedItems/{}", session.user_id, id);
    if played {
        post_json(&client()?, base, &session.token, None, &path, &json!({}))?;
    } else {
        delete_empty(base, &session.token, &path)?;
    }
    Ok(())
}

/// Where to send mpv for one item, after asking the server how it wants to
/// deliver it.
pub struct PlayTarget {
    pub url: String,
    pub source_label: Option<String>,
    pub source_bytes: Option<u64>,
    pub session: PlaybackSession,
    pub selection: PlaybackSelection,
    pub subtitle_url: Option<String>,
}

#[derive(Clone, Default)]
pub struct PlaybackSelection {
    pub source_id: Option<String>,
    pub audio_index: Option<i64>,
    /// -1 explicitly disables subtitles. None lets the server/player choose.
    pub subtitle_index: Option<i64>,
    pub preferred_version: String,
    pub audio_language: String,
    pub subtitle_language: String,
}

impl PlaybackSelection {
    pub fn apply_preferences(&mut self, playback: &crate::preferences::Playback) {
        self.preferred_version.clone_from(&playback.preferred_version);
        self.audio_language.clone_from(&playback.audio_language);
        self.subtitle_language.clone_from(&playback.subtitle_language);
    }

    fn source<'a>(&self, sources: &'a [PlaybackSource]) -> Option<&'a PlaybackSource> {
        if let Some(id) = &self.source_id {
            sources.iter().find(|source| source.id.as_ref() == Some(id))
        } else {
            preferred_source(sources, &self.preferred_version).or_else(|| sources.first())
        }
    }

    pub fn resolve_preferences(&mut self, sources: &[PlaybackSource]) -> bool {
        let Some(source) = self.source(sources) else { return false };
        let audio = self.audio_index.or_else(|| source.language_index("Audio", &self.audio_language));
        let subtitles = self.subtitle_index.or_else(|| source.language_index("Subtitle", &self.subtitle_language));
        let changes_source = self.source_id.is_none() && sources.first().is_some_and(|first| first.id != source.id);
        if !changes_source && audio == self.audio_index && subtitles == self.subtitle_index {
            return false;
        }
        self.source_id = source.id.clone();
        self.audio_index = audio;
        self.subtitle_index = subtitles;
        true
    }
}

pub fn matches_playback_language(language: &str, preferred: &str) -> bool {
    preferred.split(',').any(|code| {
        language.eq_ignore_ascii_case(code) || language.split(['-', '_']).next()
            .is_some_and(|base| base.eq_ignore_ascii_case(code))
    })
}

#[derive(Clone)]
pub struct PlaybackSession {
    pub play_session_id: Option<String>,
    pub media_source_id: String,
    pub method: PlayMethod,
}

#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum PlayMethod {
    DirectPlay,
    DirectStream,
    Transcode,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PlaybackInfo {
    #[serde(default)]
    media_sources: Vec<PlaybackSource>,
    play_session_id: Option<String>,
    error_code: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlaybackSource {
    pub id: Option<String>,
    pub name: Option<String>,
    path: Option<String>,
    pub container: Option<String>,
    pub size: Option<u64>,
    pub bitrate: Option<u64>,
    #[serde(default)]
    pub media_streams: Vec<MediaStream>,
    pub default_audio_stream_index: Option<i64>,
    pub default_subtitle_stream_index: Option<i64>,
    direct_stream_url: Option<String>,
    transcoding_url: Option<String>,
    supports_direct_play: Option<bool>,
    supports_direct_stream: Option<bool>,
}

impl PlaybackSource {
    pub fn version_label(&self) -> &str {
        self.path.as_deref().and_then(|path| path.rsplit(['/', '\\']).next())
            .filter(|name| !name.is_empty() && !name.contains('?'))
            .or(self.name.as_deref()).or(self.container.as_deref()).unwrap_or("Unknown version")
    }

    pub fn language_index(&self, track_type: &str, preferred: &str) -> Option<i64> {
        if preferred.is_empty() { return None; }
        if track_type == "Subtitle" && preferred == "no" { return Some(-1); }
        self.media_streams.iter().filter(|track| track.kind == track_type && track.index.is_some()
            && track.language.as_deref().is_some_and(|language| matches_playback_language(language, preferred)))
            .min_by_key(|track| track.is_default != Some(true)).and_then(|track| track.index)
    }
}

pub fn preferred_source<'a>(sources: &'a [PlaybackSource], text: &str) -> Option<&'a PlaybackSource> {
    text.lines().filter_map(|line| TextRules::new(line).ok())
        .find_map(|rule| sources.iter().find(|source| rule.matches(source.version_label())))
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaStream {
    #[serde(rename = "Type")]
    pub kind: String,
    pub index: Option<i64>,
    pub display_title: Option<String>,
    title: Option<String>,
    language: Option<String>,
    pub codec: Option<String>,
    pub height: Option<u32>,
    pub is_default: Option<bool>,
    delivery_method: Option<String>,
    delivery_url: Option<String>,
}

impl MediaStream {
    pub fn label(&self) -> &str {
        self.display_title.as_deref().or(self.title.as_deref()).or(self.language.as_deref())
            .or(self.codec.as_deref()).unwrap_or("Unknown track")
    }
}

pub fn playback_sources(base: &str, session: &Session, item_id: &str) -> Result<Vec<PlaybackSource>, String> {
    let path = format!("/emby/Items/{}/PlaybackInfo?UserId={}", url_encode(item_id), url_encode(&session.user_id));
    let info = get_json(base, &session.token, &path)?;
    let info: PlaybackInfo = serde_json::from_value(info).map_err(|_| "Invalid PlaybackInfo response".to_string())?;
    if let Some(code) = info.error_code.filter(|code| !code.is_empty() && code != "None") {
        return Err(format!("PlaybackInfo: {code}"));
    }
    Ok(info.media_sources)
}

/// Negotiate one media source, retaining the information needed to report it.
pub fn playback_info(
    base: &str,
    session: &Session,
    item_id: &str,
    device_id: &str,
    mut selection: PlaybackSelection,
    direct_connection: bool,
) -> Result<PlayTarget, String> {
    let encoded_item_id = url_encode(item_id);
    let path = format!(
        "/emby/Items/{}/PlaybackInfo?UserId={}&IsPlayback=true",
        encoded_item_id,
        url_encode(&session.user_id)
    );
    let profile = json!({
        "UserId": session.user_id,
        "IsPlayback": true,
        "DeviceProfile": {
            "MaxStreamingBitrate": 140_000_000,
            "DirectPlayProfiles": [{
                "Type": "Video",
                "Container": "mkv,mp4,mov,avi,ts,webm,m4v",
                "VideoCodec": "hevc,h264,h265,mpeg4",
                "AudioCodec": "aac,mp3,wav,ac3,eac3,flac,opus"
            }],
            "TranscodingProfiles": [{
                "Type": "Video",
                "Container": "ts",
                "Protocol": "hls",
                "VideoCodec": "h264",
                "AudioCodec": "aac",
                "Context": "Streaming"
            }]
        }
    });
    let auth = auth_header(device_id);
    let client = client_for(direct_connection)?;
    let request = |selection: &PlaybackSelection| -> Result<PlaybackInfo, String> {
        let mut body = profile.clone();
        if let Some(id) = &selection.source_id {
            body["MediaSourceId"] = json!(id);
        }
        if let Some(index) = selection.audio_index {
            body["AudioStreamIndex"] = json!(index);
        }
        if let Some(index) = selection.subtitle_index {
            body["SubtitleStreamIndex"] = json!(index);
        }
        let result = post_json(&client, base, &session.token, Some(&auth), &path, &body);
        let info: PlaybackInfo = match result {
            // Keep the static stream fallback for servers without PlaybackInfo.
            Err(error) if matches!(error.as_str(), "the server answered 404" | "the server answered 405") => {
                PlaybackInfo { media_sources: Vec::new(), play_session_id: None, error_code: None }
            }
            other => serde_json::from_value(other?).map_err(|_| "Invalid PlaybackInfo response".to_string())?,
        };
        if let Some(code) = info.error_code.as_ref().filter(|code| !code.is_empty() && *code != "None") {
            return Err(format!("PlaybackInfo: {code}"));
        }
        Ok(info)
    };
    if selection.subtitle_language == "no" && selection.subtitle_index.is_none() {
        selection.subtitle_index = Some(-1);
    }
    let mut info = request(&selection)?;
    if selection.resolve_preferences(&info.media_sources) {
        info = request(&selection)?;
    }
    let source = selection.source(&info.media_sources);
    if source.is_none() && selection.source_id.is_some() {
        return Err("The selected media version is unavailable".into());
    }
    let media_source_id = source.as_ref().and_then(|s| s.id.clone()).unwrap_or_else(|| item_id.to_string());
    let play_session_id = info.play_session_id.filter(|id| !id.is_empty());
    let direct = source.as_ref().and_then(|s| s.direct_stream_url.as_deref()).filter(|url| !url.is_empty());
    let transcode = source.as_ref().and_then(|s| s.transcoding_url.as_deref()).filter(|url| !url.is_empty());
    let direct_play = source.as_ref().and_then(|s| s.supports_direct_play);
    let direct_stream = source.as_ref().and_then(|s| s.supports_direct_stream);
    let (stream, method) = match (direct, direct_play, direct_stream, transcode) {
        (Some(url), _, flag, _) if flag != Some(false) => (url.to_string(), PlayMethod::DirectStream),
        (_, Some(true), _, _) => (format!("/emby/Videos/{encoded_item_id}/stream?static=true"), PlayMethod::DirectPlay),
        (_, _, _, Some(url)) => (url.to_string(), PlayMethod::Transcode),
        (_, Some(false), Some(false), _) => return Err("No playable media source".into()),
        _ => (format!("/emby/Videos/{encoded_item_id}/stream?static=true"), PlayMethod::DirectStream),
    };
    let origin = reqwest::Url::parse(&format!("{base}/")).map_err(|_| "Invalid server URL".to_string())?;
    let media_url = |path: &str| {
        let mut url = if path.starts_with('/') && !path.starts_with("//") {
            reqwest::Url::parse(&format!("{base}{path}"))
        } else {
            origin.join(path)
        }.ok()?;
        if !matches!(url.scheme(), "http" | "https") {
            return None;
        }
        if url.origin() == origin.origin() && !url.query_pairs().any(|(key, _)| key.eq_ignore_ascii_case("api_key")) {
            url.query_pairs_mut().append_pair("api_key", &session.token);
        }
        Some(url)
    };
    let mut url = media_url(&stream).ok_or_else(|| "Invalid media URL".to_string())?;
    if url.origin() == origin.origin() {
        let static_stream = direct != Some(stream.as_str()) && transcode != Some(stream.as_str());
        if static_stream {
            let mut query = url.query_pairs_mut();
            query.append_pair("MediaSourceId", &media_source_id).append_pair("DeviceId", device_id);
            if let Some(id) = &play_session_id {
                query.append_pair("PlaySessionId", id);
            }
        }
    }
    let subtitle_url = selection.subtitle_index.filter(|index| *index >= 0)
        .and_then(|index| source?.media_streams.iter().find(|track| track.kind == "Subtitle" && track.index == Some(index)))
        .filter(|track| method != PlayMethod::Transcode || track.delivery_method.as_deref() == Some("External"))
        .and_then(|track| track.delivery_url.as_deref())
        .and_then(media_url).map(String::from);
    Ok(PlayTarget {
        url: url.into(),
        source_label: source.map(PlaybackSource::version_label).map(str::to_owned),
        source_bytes: source.and_then(|source| source.size).filter(|_| method != PlayMethod::Transcode),
        session: PlaybackSession { play_session_id, media_source_id, method },
        selection,
        subtitle_url,
    })
}

pub fn set_favorite(
    base: &str,
    session: &Session,
    id: &str,
    favorite: bool,
) -> Result<(), String> {
    let path = format!("/emby/Users/{}/FavoriteItems/{}", session.user_id, id);
    if favorite {
        post_json(&client()?, base, &session.token, None, &path, &json!({}))?;
    } else {
        delete_empty(base, &session.token, &path)?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub enum PlaybackEvent {
    Start,
    Progress,
    Pause,
    Unpause,
    RateChange,
    VolumeChange,
    Stop,
}

pub struct PlaybackReport {
    pub event: PlaybackEvent,
    pub ticks: i64,
    pub paused: bool,
    pub can_seek: bool,
    pub rate: f64,
    pub volume: u8,
    pub muted: bool,
}

pub fn report_progress(
    base: &str,
    session: &Session,
    device_id: &str,
    item_id: &str,
    playback: &PlaybackSession,
    report: &PlaybackReport,
    direct_connection: bool,
) -> Result<(), String> {
    let path = match report.event {
        PlaybackEvent::Start => "/emby/Sessions/Playing",
        PlaybackEvent::Stop => "/emby/Sessions/Playing/Stopped",
        _ => "/emby/Sessions/Playing/Progress",
    };
    let mut body = json!({
        "ItemId": item_id,
        "MediaSourceId": playback.media_source_id,
        "PositionTicks": report.ticks,
        "IsPaused": report.paused,
        "CanSeek": report.can_seek,
        "IsMuted": report.muted,
        "VolumeLevel": report.volume,
        "PlaybackRate": report.rate,
        "PlayMethod": playback.method,
    });
    if let Some(id) = &playback.play_session_id {
        body["PlaySessionId"] = json!(id);
    }
    let event_name = match report.event {
        PlaybackEvent::Start | PlaybackEvent::Stop => None,
        PlaybackEvent::Progress => Some("TimeUpdate"),
        PlaybackEvent::Pause => Some("Pause"),
        PlaybackEvent::Unpause => Some("Unpause"),
        PlaybackEvent::RateChange => Some("PlaybackRateChange"),
        PlaybackEvent::VolumeChange => Some("VolumeChange"),
    };
    if let Some(name) = event_name {
        body["EventName"] = json!(name);
    }
    post_json(&client_for(direct_connection)?, base, &session.token, Some(&auth_header(device_id)), path, &body)?;
    Ok(())
}

pub fn image_url(
    base: &str,
    token: &str,
    item_id: &str,
    kind: &str,
    tag: &str,
    max_width: i64,
    quality: u8,
) -> String {
    let tag_part = if tag.is_empty() {
        String::new()
    } else {
        format!("&tag={}", tag)
    };
    let index = if kind == "Backdrop" { "/0" } else { "" };
    format!(
        "{}/emby/Items/{}/Images/{}{}?maxWidth={}&quality={}&api_key={}{}",
        base, item_id, kind, index, max_width, quality, token, tag_part
    )
}

pub fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    let resp = client()?
        .get(url)
        .send()
        .map_err(|e| friendly(&e))?;
    if !resp.status().is_success() {
        return Err(format!("image: {}", resp.status().as_u16()));
    }
    let bytes = resp.bytes().map_err(|e| friendly(&e))?;
    Ok(bytes.to_vec())
}

pub fn url_encode(raw: &str) -> String {
    let mut out = String::new();
    for b in raw.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

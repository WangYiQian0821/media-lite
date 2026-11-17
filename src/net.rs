//! Background work: every network call and image download happens away from the
//! drawing thread, so the window never freezes while a server is slow.

use crate::api;
use crate::model::*;
use crate::sample;
use eframe::egui;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// Where a job goes: one server, one signed-in session.
#[derive(Clone, Debug)]
pub struct Target {
    pub base: String,
    pub session: Session,
    pub sample: bool,
    pub device_id: String,
    pub direct_playback: bool,
}

impl Target {
    pub fn new(entry: &ServerEntry) -> Option<Self> {
        if entry.sample {
            return Some(Self {
                base: "sample://library".to_string(),
                session: Session {
                    token: String::new(),
                    user_id: "sample-user".to_string(),
                    user_name: entry.user_name.clone(),
                },
                sample: true,
                device_id: entry.device_id.clone(),
                direct_playback: false,
            });
        }
        Some(Self {
            base: api::normalize_url(&entry.url),
            session: Session {
                token: entry.token.clone()?,
                user_id: entry.user_id.clone()?,
                user_name: entry.user_name.clone(),
            },
            sample: false,
            device_id: entry.device_id.clone(),
            direct_playback: entry.direct_playback,
        })
    }
}

/// Where a Home reply belongs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HomePurpose {
    Rows,
    Aggregate(usize),
}

pub enum Job {
    /// One address, one user name, one password: probe, sign in, and report the
    /// address form that actually worked.
    Connect {
        server_id: String,
        url: String,
        device_id: String,
        username: String,
        password: String,
    },
    Views {
        target: Target,
    },
    Home {
        target: Target,
        libraries: Vec<MediaItem>,
        /// Where the rows go: the home screen, or the aggregate view fed by a
        /// particular server index.
        purpose: HomePurpose,
    },
    Browse {
        target: Target,
        parent: String,
        include: String,
        sort: String,
    },
    Favorites {
        target: Target,
    },
    Search {
        target: Target,
        term: String,
        include_media: bool,
    },
    Detail {
        target: Target,
        id: String,
    },
    Episodes {
        request: Option<u64>,
        target: Target,
        series: String,
        season: String,
    },
    DetailMedia {
        target: Target,
        item: String,
    },
    SetPlayed {
        target: Target,
        id: String,
        played: bool,
    },
    SetFavorite {
        target: Target,
        id: String,
        favorite: bool,
    },
    PreparePlayback {
        request: u64,
        target: Target,
        item: String,
        selection: api::PlaybackSelection,
    },
    PlaybackChapters {
        request: u64,
        target: Target,
        item: String,
    },
    PrepareExternal {
        request: u64,
        target: Target,
        item: MediaItem,
        selection: api::PlaybackSelection,
        count: usize,
    },
    WebDav {
        url: reqwest::Url,
        username: String,
        password: String,
        data: Option<Vec<u8>>,
    },
    Report {
        target: Target,
        item: String,
        playback: api::PlaybackSession,
        report: api::PlaybackReport,
    },
    Flush(Sender<()>),
}

pub enum Msg {
    Connected {
        server_id: String,
        base: String,
        name: String,
        version: String,
        session: Session,
        user_name: String,
    },
    Views(Vec<MediaItem>),
    Home(HomePurpose, Vec<Row>),
    Browse {
        parent: String,
        items: Vec<MediaItem>,
        total: i64,
    },
    Favorites(Vec<MediaItem>),
    Search {
        term: String,
        items: Vec<MediaItem>,
    },
    Detail(Box<ItemDetail>),
    Episodes {
        request: Option<u64>,
        season: String,
        result: Result<Vec<MediaItem>, String>,
    },
    DetailMedia {
        item: String,
        result: Result<Vec<api::PlaybackSource>, String>,
    },
    Played {
        id: String,
        played: bool,
    },
    Favorite {
        id: String,
        favorite: bool,
    },
    PlaybackPrepared {
        request: u64,
        result: Result<api::PlayTarget, String>,
    },
    PlaybackChapters {
        request: u64,
        chapters: Vec<Chapter>,
    },
    ExternalPrepared {
        request: u64,
        result: Result<Vec<api::PlayTarget>, String>,
    },
    WebDav(Result<Option<Vec<u8>>, String>),
    Failed {
        what: String,
        message: String,
    },
}

pub struct Worker {
    tx: Sender<Job>,
    playback_tx: Sender<Job>,
    rx: Receiver<Msg>,
    pub busy: bool,
}

impl Worker {
    pub fn spawn(ctx: egui::Context) -> Self {
        let (tx, job_rx) = channel::<Job>();
        let (playback_tx, playback_rx) = channel::<Job>();
        let (msg_tx, msg_rx) = channel::<Msg>();
        for queue in [job_rx, playback_rx] {
            let replies = msg_tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                while let Ok(job) = queue.recv() {
                    if let Some(msg) = run(job) {
                        let _ = replies.send(msg);
                    }
                    ctx.request_repaint();
                }
            });
        }
        Self {
            tx,
            playback_tx,
            rx: msg_rx,
            busy: false,
        }
    }

    pub fn send(&mut self, job: Job) {
        self.busy = true;
        // Library reads and playback reports can wait while the player opens.
        let queue = if matches!(job, Job::PreparePlayback { .. }) { &self.playback_tx } else { &self.tx };
        let _ = queue.send(job);
    }

    pub fn poll(&mut self) -> Vec<Msg> {
        let msgs: Vec<Msg> = self.rx.try_iter().collect();
        if !msgs.is_empty() {
            self.busy = false;
        }
        msgs
    }

    pub fn flush_on_exit(&self) {
        let (done, wait) = channel();
        if self.tx.send(Job::Flush(done)).is_ok() {
            let _ = wait.recv_timeout(std::time::Duration::from_secs(2));
        }
    }
}

fn run(job: Job) -> Option<Msg> {
    match job {
        Job::Connect {
            server_id,
            url,
            device_id,
            username,
            password,
        } => match api::connect(&url, &device_id, &username, &password) {
            Ok(done) => Some(Msg::Connected {
                server_id,
                base: done.base,
                name: done.name,
                version: done.version,
                session: done.session,
                user_name: done.user_name,
            }),
            Err(message) => Some(Msg::Failed {
                what: "connect".into(),
                message,
            }),
        },
        Job::Views { target } => {
            let res = if target.sample {
                Ok(sample::views())
            } else {
                api::views(&target.base, &target.session)
            };
            match res {
                Ok(items) => Some(Msg::Views(items)),
                Err(message) => Some(Msg::Failed {
                    what: "libraries".into(),
                    message,
                }),
            }
        }
        Job::Home {
            target,
            libraries,
            purpose,
        } => {
            let res = if target.sample {
                Ok(sample::home())
            } else {
                api::home(&target.base, &target.session, &libraries)
            };
            match res {
                Ok(rows) => Some(Msg::Home(purpose, rows)),
                Err(message) => Some(Msg::Failed {
                    what: "home".into(),
                    message,
                }),
            }
        }
        Job::Browse {
            target,
            parent,
            include,
            sort,
        } => {
            let res = if target.sample {
                Ok(sample::browse(&parent))
            } else {
                api::browse(&target.base, &target.session, &parent, &include, &sort)
            };
            match res {
                Ok((items, total)) => Some(Msg::Browse {
                    parent,
                    items,
                    total,
                }),
                Err(message) => Some(Msg::Failed {
                    what: "library".into(),
                    message,
                }),
            }
        }
        Job::Favorites { target } => {
            let res = if target.sample {
                Ok(sample::favorites())
            } else {
                api::favorites(&target.base, &target.session)
            };
            match res {
                Ok(items) => Some(Msg::Favorites(items)),
                Err(message) => Some(Msg::Failed {
                    what: "favorites".into(),
                    message,
                }),
            }
        }
        Job::Search { target, term, include_media } => {
            let res = if target.sample {
                Ok(sample::search(&term))
            } else {
                api::search(&target.base, &target.session, &term, include_media)
            };
            match res {
                Ok(items) => Some(Msg::Search { term, items }),
                Err(message) => Some(Msg::Failed {
                    what: "search".into(),
                    message,
                }),
            }
        }
        Job::Detail { target, id } => {
            let res = if target.sample {
                Ok(sample::detail(&id))
            } else {
                api::detail(&target.base, &target.session, &id)
            };
            match res {
                Ok(detail) => Some(Msg::Detail(Box::new(detail))),
                Err(message) => Some(Msg::Failed {
                    what: "details".into(),
                    message,
                }),
            }
        }
        Job::Episodes {
            request,
            target,
            series,
            season,
        } => {
            let res = if target.sample {
                let n = season
                    .rsplit('s')
                    .next()
                    .and_then(|x| x.parse::<i64>().ok())
                    .unwrap_or(1);
                Ok(sample::episodes(&series, n))
            } else {
                api::episodes_in(&target.base, &target.session, &series, &season)
            };
            Some(Msg::Episodes { request, season, result: res })
        }
        Job::DetailMedia { target, item } => Some(Msg::DetailMedia {
            result: api::playback_sources(&target.base, &target.session, &item),
            item,
        }),
        Job::SetPlayed {
            target,
            id,
            played,
        } => {
            if target.sample {
                return Some(Msg::Played { id, played });
            }
            match api::set_played(&target.base, &target.session, &id, played) {
                Ok(()) => Some(Msg::Played { id, played }),
                Err(message) => Some(Msg::Failed {
                    what: "watched".into(),
                    message,
                }),
            }
        }
        Job::SetFavorite {
            target,
            id,
            favorite,
        } => {
            if target.sample {
                return Some(Msg::Favorite { id, favorite });
            }
            match api::set_favorite(&target.base, &target.session, &id, favorite) {
                Ok(()) => Some(Msg::Favorite { id, favorite }),
                Err(message) => Some(Msg::Failed {
                    what: "favourite".into(),
                    message,
                }),
            }
        }
        Job::PreparePlayback { request, target, item, selection } => Some(Msg::PlaybackPrepared {
            request,
            result: api::playback_info(
                &target.base, &target.session, &item, &target.device_id, selection, target.direct_playback,
            ),
        }),
        Job::PlaybackChapters { request, target, item } => Some(Msg::PlaybackChapters {
            request,
            chapters: api::chapters(&target.base, &target.session, &item).unwrap_or_default(),
        }),
        Job::PrepareExternal { request, target, item, selection, count } => {
            let next_selection = api::PlaybackSelection {
                source_id: None, audio_index: None, subtitle_index: None, ..selection.clone()
            };
            let prepare = |id: &str, selection| api::playback_info(&target.base,
                &target.session, id, &target.device_id, selection, false);
            let result = prepare(&item.id, selection).map(|first| {
                let mut queue = vec![first];
                if let Some(series) = item.series_id.as_ref().filter(|_| count > 1) {
                    if let Ok(episodes) = api::episodes_in(&target.base, &target.session, series, item.season_id.as_deref().unwrap_or("")) {
                        if let Some(index) = episodes.iter().position(|episode| episode.id == item.id) {
                            queue.extend(episodes.iter().skip(index + 1).take(count - 1)
                                .filter_map(|episode| prepare(&episode.id, next_selection.clone()).ok()));
                        }
                    }
                }
                queue
            });
            Some(Msg::ExternalPrepared { request, result })
        }
        Job::WebDav { url, username, password, data } => Some(Msg::WebDav(api::transfer_backup(url, username, password, data))),
        Job::Report { target, item, playback, report } => api::report_progress(
            &target.base, &target.session, &target.device_id, &item, &playback, &report, target.direct_playback,
        ).err().map(|message| Msg::Failed { what: "progress".into(), message }),
        Job::Flush(done) => {
            let _ = done.send(());
            None
        }
    }
}

// ------------------------------------------------------------------ images

/// Downloads artwork in the background. Three threads, so a grid fills quickly
/// without hammering the server.
/// Posters are fetched once and then kept on disk, so a restart does not pull
/// a hundred images again. Files are named by a hash of the cache key, which
/// already encodes server + item + image kind + tag + width.
/// Where posters are kept between runs.
///
/// A drive other than C: is preferred, because a large library can put hundreds
/// of megabytes here and C: is usually the system drive. Order:
///   `MEDIA_LITE_CACHE` env var → `E:\MediaLiteCache` → `%LOCALAPPDATA%`.
pub fn cache_root() -> std::path::PathBuf {
    if let Ok(v) = std::env::var("MEDIA_LITE_CACHE") {
        if !v.trim().is_empty() {
            return std::path::PathBuf::from(v);
        }
    }
    let e = std::path::PathBuf::from(r"E:\MediaLiteCache");
    if std::fs::create_dir_all(&e).is_ok() {
        return e;
    }
    let local = std::env::var("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("MediaLite")
        .join("cache");
    local
}

fn cache_dir() -> Option<std::path::PathBuf> {
    let dir = cache_root().join("img");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Human-readable size of everything currently cached, for the settings page.
pub fn cache_usage() -> (usize, u64) {
    directory_usage(&cache_root().join("img"))
}

pub fn directory_usage(dir: &std::path::Path) -> (usize, u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (0, 0);
    };
    let mut count = 0usize;
    let mut bytes = 0u64;
    for e in entries.flatten() {
        if let Ok(m) = e.metadata() {
            if m.is_file() {
                count += 1;
                bytes += m.len();
            }
        }
    }
    (count, bytes)
}

fn cache_path(key: &str) -> Option<std::path::PathBuf> {
    let mut h: u64 = 1469598103934665603;
    for b in key.bytes() {
        h = (h ^ b as u64).wrapping_mul(1099511628211);
    }
    cache_dir().map(|d| d.join(format!("{:016x}.img", h)))
}

fn disk_get(key: &str, max_age_secs: u64) -> Option<Vec<u8>> {
    let path = cache_path(key)?;
    let meta = std::fs::metadata(&path).ok()?;
    let fresh = meta.modified().ok()?.elapsed().ok()?.as_secs() <= max_age_secs;
    (meta.len() > 0 && fresh).then(|| std::fs::read(path).ok()).flatten()
}

fn disk_put(key: &str, bytes: &[u8]) {
    if let Some(path) = cache_path(key) {
        // write to a temp name first so a half-written file is never read back
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

/// Keep the poster cache from growing without bound: past this many files the
/// oldest half is dropped.
fn prune_cache() {
    let Some(dir) = cache_dir() else { return };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            if !m.is_file() {
                return None;
            }
            Some((m.modified().ok()?, e.path()))
        })
        .collect();
    if files.len() <= 1500 {
        return;
    }
    files.sort_by_key(|(t, _)| *t);
    for (_, path) in files.into_iter().take(700) {
        let _ = std::fs::remove_file(path);
    }
}

/// The last home payload, so a restart can paint immediately instead of waiting
/// ten seconds for the server. Replaced as soon as fresh data arrives.
fn rows_cache_path() -> std::path::PathBuf {
    cache_root().join("rows.json")
}

pub fn save_rows(server_id: &str, rows: &[crate::model::Row]) {
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let payload = serde_json::json!({
        "server": server_id,
        "at": at,
        "rows": rows,
    });
    let path = rows_cache_path();
    let _ = std::fs::create_dir_all(cache_root());
    let tmp = path.with_extension("tmp");
    if let Ok(text) = serde_json::to_string(&payload) {
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

pub fn response_cache_size() -> u64 {
    std::fs::metadata(rows_cache_path()).map_or(0, |m| m.len())
}

pub fn clear_response_cache() -> std::io::Result<()> {
    match std::fs::remove_file(rows_cache_path()) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

pub fn clear_cache_files(dir: &std::path::Path, is_cache: impl Fn(&std::path::Path) -> bool) -> std::io::Result<()> {
    match std::fs::read_dir(dir) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry?;
                let path = entry.path();
                if entry.file_type()?.is_file() && is_cache(&path) {
                    std::fs::remove_file(path)?;
                }
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Cached rows for this server, if they are fresh enough to show.
pub fn load_rows(server_id: &str, max_age_secs: u64) -> Option<Vec<crate::model::Row>> {
    let text = std::fs::read_to_string(rows_cache_path()).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    if v.get("server").and_then(|x| x.as_str()) != Some(server_id) {
        return None;
    }
    let at = v.get("at").and_then(|x| x.as_u64()).unwrap_or(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if now.saturating_sub(at) > max_age_secs {
        return None;
    }
    serde_json::from_value(v.get("rows")?.clone()).ok()
}

pub struct ImageLoader {
    req_tx: Sender<(String, String, u64)>,
    out_rx: Receiver<(String, Option<egui::ColorImage>)>,
}

impl ImageLoader {
    pub fn spawn(ctx: egui::Context) -> Self {
        let (req_tx, req_rx) = channel::<(String, String, u64)>();
        let (out_tx, out_rx) = channel::<(String, Option<egui::ColorImage>)>();
        let shared = Arc::new(Mutex::new(req_rx));
        for _ in 0..3 {
            let shared = Arc::clone(&shared);
            let out_tx = out_tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || loop {
                let next = {
                    let guard = match shared.lock() {
                        Ok(g) => g,
                        Err(_) => return,
                    };
                    guard.recv()
                };
                let (key, url, max_age) = match next {
                    Ok(v) => v,
                    Err(_) => return,
                };
                let cached = disk_get(&key, max_age).and_then(|bytes| image::load_from_memory(&bytes).ok());
                let decoded = cached.or_else(|| {
                    let bytes = api::fetch_bytes(&url).ok()?;
                    let image = image::load_from_memory(&bytes).ok()?;
                    disk_put(&key, &bytes);
                    Some(image)
                });
                let texture = decoded.map(|image| {
                    let rgba = image.to_rgba8();
                    let size = [rgba.width() as usize, rgba.height() as usize];
                    egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw())
                });
                let _ = out_tx.send((key, texture));
                ctx.request_repaint();
            });
        }
        prune_cache();
        Self { req_tx, out_rx }
    }

    pub fn request(&self, key: String, url: String, max_age_secs: u64) {
        let _ = self.req_tx.send((key, url, max_age_secs));
    }

    pub fn poll(&self) -> Vec<(String, Option<egui::ColorImage>)> {
        self.out_rx.try_iter().collect()
    }
}

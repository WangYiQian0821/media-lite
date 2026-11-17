//! The window: sidebar, top bar, home rows, library grids, the detail page,
//! search, settings, the player and the optional screenshot.

use crate::api;
use crate::api::{PlaybackEvent, PlaybackReport, PlaybackSelection, PlaybackSession, PlaybackSource};
use crate::i18n;
use crate::model::*;
use crate::mpv::{Track, TrackKind};
use crate::net::{ImageLoader, Job, Msg, Target, Worker};
use crate::preferences::{IntroMode, Preferences, PlayerChoice, Quality, ThemeMode, TextRules};
use crate::sample;
use crate::theme::{self, Palette};
use eframe::egui::{
    self, Align, Align2, Color32, FontId, Layout, Pos2, Rect, Response, RichText,
    ScrollArea, Sense, Stroke, TextureId, Ui, Vec2,
};
use serde::{Deserialize, Serialize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[path = "player_ui.rs"]
mod player_ui;
#[path = "home_ui.rs"]
mod home_ui;
#[path = "settings_ui.rs"]
mod settings_ui;
#[path = "updates.rs"]
mod updates;

// ------------------------------------------------------------------ config

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct Config {
    #[serde(default)]
    servers: Vec<ServerEntry>,
    #[serde(default)]
    dark: Option<bool>,
    #[serde(default)]
    active_id: Option<String>,
    /// "en" or "zh"; the language chosen in Settings.
    #[serde(default)]
    lang: Option<String>,
    /// "small", "medium" or "large": how wide a poster card is.
    #[serde(default)]
    card_scale: Option<String>,
    /// True when the wall is drawn as single rows instead of posters.
    #[serde(default)]
    list_view: Option<bool>,
    #[serde(default)]
    episode_grid: bool,
    #[serde(default)]
    preferences: Option<Preferences>,
    #[serde(default)]
    search_history: Vec<String>,
    #[serde(default)]
    danmaku_history: HashMap<String, PathBuf>,
}

impl Config {
    fn settings(&self) -> Preferences {
        let mut prefs = self.preferences.clone().unwrap_or_else(|| {
            let mut prefs = Preferences::default();
            if let Some(lang) = &self.lang {
                prefs.language = lang.clone();
            }
            if self.dark == Some(false) {
                prefs.theme = ThemeMode::Light;
            }
            prefs
        });
        prefs.validate();
        prefs
    }
}

fn config_path() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    base.join("MediaLite").join("config-v2.json")
}

// ------------------------------------------------------------------- state

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum View {
    Home,
    Library,
    Search,
    Detail,
    HomeRow,
    Favorites,
    /// Everything from every server, merged into one wall.
    Aggregate,
    Settings,
}

/// How wide a poster card is. The original exposes a cover scale ratio; three
/// steps are easier to pick than a slider and cover the same range.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CardScale {
    Small,
    Medium,
    Large,
}

impl CardScale {
    /// Card width in px. The height is 1.5x the width, the shape of a film poster.
    pub fn width(self) -> f32 {
        match self {
            CardScale::Small => 136.0,
            CardScale::Medium => 168.0,
            CardScale::Large => 212.0,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            CardScale::Small => "small",
            CardScale::Medium => "medium",
            CardScale::Large => "large",
        }
    }

    pub fn from_code(text: &str) -> Self {
        match text.trim().to_ascii_lowercase().as_str() {
            "small" | "s" => CardScale::Small,
            "large" | "l" => CardScale::Large,
            _ => CardScale::Medium,
        }
    }
}

#[derive(Clone, Default)]
struct Grid {
    parent: String,
    title: String,
    include: String,
    sort: String,
    items: Vec<MediaItem>,
    total: i64,
}

#[derive(Clone, PartialEq)]
enum PlaybackPhase {
    Loading,
    Starting,
    Ready,
    Ended,
    Failed(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum PlayerMenu {
    Speed,
    Audio,
    Subtitles,
    Danmaku,
    Settings,
    Playlist,
}

struct Player {
    request: u64,
    target: Target,
    session: Option<PlaybackSession>,
    item: MediaItem,
    pos: f64,
    duration: f64,
    playing: bool,
    phase: PlaybackPhase,
    seekable: bool,
    seek_started: bool,
    pending_seek: Option<f64>,
    buffering: bool,
    buffer_pos: f64,
    cache_speed: f64,
    next_report: f64,
    speed: f64,
    volume: u8,
    muted: bool,
    tracks: Vec<Track>,
    initial_tracks: Option<(PlaybackSelection, Option<String>)>,
    menu: Option<PlayerMenu>,
    queue: Vec<MediaItem>,
    queue_loading: bool,
    source_label: Option<String>,
    last_activity: f64,
    pending_click: Option<f64>,
    topmost: bool,
    fill_video: bool,
    show_cache_speed: bool,
    auto_hide: bool,
    danmaku: Vec<player_ui::Danmaku>,
    danmaku_filter: TextRules,
    show_danmaku: bool,
    notice: Option<(String, f64)>,
    watched: bool,
    chapters: Vec<Chapter>,
    intro_range: Option<(f64, f64)>,
    intro_skipped: bool,
    secondary_id: Option<i64>,
    secondary_pending: bool,
    fullscreen_pending: bool,
    restore_maximized: bool,
    boost_started: Option<f64>,
    boost_base_speed: Option<f64>,
    /// mpv driving the picture, when libmpv could be loaded.
    engine: Option<crate::mpv::Mpv>,
}

impl Player {
    fn leave_fullscreen(&mut self, ctx: &egui::Context) {
        self.fullscreen_pending = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        if std::mem::take(&mut self.restore_maximized) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        }
    }

    fn report(&mut self, worker: &mut Worker, event: PlaybackEvent, now: f64) -> bool {
        let Some(session) = &self.session else { return false };
        if self.phase != PlaybackPhase::Ready {
            return false;
        }
        self.next_report = now + 10.0;
        worker.send(Job::Report {
            target: self.target.clone(),
            item: self.item.id.clone(),
            playback: session.clone(),
            report: PlaybackReport {
                event,
                ticks: ticks_from_secs(self.pos),
                paused: !self.playing,
                can_seek: self.seekable,
                rate: self.speed,
                volume: self.volume,
                muted: self.muted,
            },
        });
        true
    }
}

struct FormState {
    editing: Option<usize>,
    url: String,
    username: String,
    password: String,
    error: String,
    busy: bool,
}

struct ExternalLaunch {
    request: u64,
    exe: PathBuf,
    choice: PlayerChoice,
    resume: f64,
}

#[derive(Default)]
struct DetailMedia {
    item_id: String,
    loading: bool,
    sources: Vec<PlaybackSource>,
    selection: PlaybackSelection,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShotPhase {
    Drawing,
    Requested,
    Done,
}

struct CachedImage {
    texture: egui::TextureHandle,
    used: f64,
    colors: [Color32; 2],
}

pub struct ImageCache {
    textures: HashMap<String, CachedImage>,
    requested: HashSet<String>,
    /// key -> time it failed. A failure is retried after a while: a slow server
    /// used to leave those tiles blank for the rest of the session.
    failed: HashMap<String, f64>,
}

impl ImageCache {
    fn new() -> Self {
        Self {
            textures: HashMap::new(),
            requested: HashSet::new(),
            failed: HashMap::new(),
        }
    }

    fn peek(&mut self, key: &str, now: f64) -> Option<TextureId> {
        self.textures.get_mut(key).map(|image| {
            image.used = now;
            image.texture.id()
        })
    }

    fn colors(&self, texture: TextureId) -> Option<[Color32; 2]> {
        self.textures.values().find(|image| image.texture.id() == texture).map(|image| image.colors)
    }

    fn known(&self, key: &str, now: f64) -> bool {
        if self.requested.contains(key) {
            return true;
        }
        match self.failed.get(key) {
            Some(at) => now - at < 25.0,
            None => false,
        }
    }

    fn ingest(&mut self, ctx: &egui::Context, batch: Vec<(String, Option<egui::ColorImage>)>, now: f64) {
        for (key, image) in batch {
            self.requested.remove(&key);
            match image {
                Some(image) => {
                    self.failed.remove(&key);
                    let colors = home_ui::artwork_colors(&image);
                    let texture = ctx.load_texture(key.clone(), image, egui::TextureOptions::LINEAR);
                    self.textures.insert(key, CachedImage { texture, used: now, colors });
                }
                None => {
                    self.failed.insert(key, now);
                    ctx.request_repaint_after(std::time::Duration::from_secs(25));
                }
            }
        }
        if self.textures.len() > 420 {
            let mut ages: Vec<(String, f64)> = self
                .textures
                .iter()
                .map(|(key, image)| (key.clone(), image.used))
                .collect();
            ages.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            for (key, _) in ages.into_iter().take(120) {
                self.textures.remove(&key);
            }
        }
    }
}

#[derive(Clone)]
enum Action {
    Open(String),
    OpenOnServer(usize, String),
    OpenHomeRow(usize),
    Play(MediaItem),
    PlaySelected(MediaItem),
    BackFromDetail,
    DetailFocus(MediaItem),
    ToggleEpisodeGrid,
    Browse {
        id: String,
        title: String,
        include: String,
    },
    DoSearch(String),
    AddServer,
    EditServer(usize),
    RemoveServer(usize),
    Connect(usize),
    SignOut(usize),
    GoSettings,
    BackFromSettings,
    Settings(settings_ui::Command),
    GoHome,
    GoFavorites,
    GoAggregate,
    SetSort(String),
    SetLang(i18n::Lang),
    SetCardScale(CardScale),
    ToggleListView,
    GoSearch,
    UseSample,
    TogglePlayed(MediaItem),
    ToggleFavorite(MediaItem),
    Season(String),
    ClosePlayer,
    TogglePlayer,
    PlayFromQueue(usize),
    TogglePlayerTopmost,
    SetVideoFill(bool),
    TogglePlayerCacheSpeed,
    TogglePlayerAutoHide,
    LoadLocalSubtitles,
    LoadDanmaku,
    ToggleDanmaku,
    SelectSecondarySubtitle(Option<i64>),
    PlayExternal(MediaItem, PlayerChoice),
    SeekBy(f64),
    SeekTo(f64),
    SkipIntro(f64),
    SetPlaybackSpeed(f64),
    SetPlaybackVolume { level: u8, commit: bool },
    TogglePlaybackMute,
    SelectPlaybackTrack(TrackKind, Option<i64>),
    ToggleTheme,
    Refresh,
    ToggleFullscreen,
}

pub struct AppLite {
    servers: Vec<ServerEntry>,
    active: Option<usize>,
    view: View,
    libs: Vec<MediaItem>,
    rows: Vec<Row>,
    home_row: Option<Row>,
    grid: Grid,
    detail: Option<ItemDetail>,
    detail_id: String,
    detail_season: Option<String>,
    detail_back: View,
    detail_media: DetailMedia,
    detail_jump: bool,
    detail_reveal_episode: bool,
    episode_grid: bool,
    search_query: String,
    search_results: Vec<MediaItem>,
    favorites: Vec<MediaItem>,
    search_term: String,
    status: Option<(String, bool)>,
    loading: bool,
    dark: bool,
    card_scale: CardScale,
    list_view: bool,
    images: ImageCache,
    worker: Worker,
    loader: ImageLoader,
    form: Option<FormState>,
    player: Option<Player>,
    player_request: u64,
    external_launch: Option<ExternalLaunch>,
    host_hwnd: Option<isize>,
    tray: Option<crate::platform::Tray>,
    /// One-shot: start the first playable item once rows arrive (used by --autoplay).
    autoplay: bool,
    /// The server dropdown under the top-left pill.
    server_menu: bool,
    /// The per-row "…" menu: which server, and where to anchor it.
    server_row_menu: Option<(usize, Pos2)>,
    /// When the panel opened (input time). Clicks within a short window after
    /// opening are ignored, so the opening click never lands on a row.
    server_menu_at: f64,
    prefs: Preferences,
    settings_view: settings_ui::SettingsView,
    update_check: updates::UpdateCheck,
    search_history: Vec<String>,
    danmaku_history: HashMap<String, PathBuf>,
    recommended_rules: TextRules,
    /// Items remain attached to their source server so IDs and requests route correctly.
    agg_items: Vec<(usize, Vec<MediaItem>)>,
    /// Servers still to be pulled for the aggregate view.
    agg_queue: Vec<usize>,
    /// 0 = continue watching, 1 = favourites, 2 = libraries.
    agg_tab: usize,
    agg_loading: bool,
    /// A view requested on the command line, applied once the library loads
    /// (the connect path resets the view to Home).
    pending_view: Option<View>,
    style_for: Option<bool>,
    frame: u64,
    shot_path: Option<PathBuf>,
    shot_phase: ShotPhase,
    last_saved_view: Option<String>,
    hero_index: usize,
    hero_was_visible: bool,
    hero_next_at: f64,
    hero_previous: Option<(TextureId, f64)>,
}

impl AppLite {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        shot: Option<PathBuf>,
        demo: Option<String>,
        autoplay: bool,
        view_override: Option<String>,
        lang: Option<i18n::Lang>,
    ) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        let p = theme::palette(true);
        theme::apply_style(&cc.egui_ctx, &p);
        let (cfg, config_error) = read_config();
        let prefs = cfg.settings();
        crate::platform::capture_http_proxy();
        api::configure_network(prefs.network.clone());
        // The language: the command line, then the saved choice, then the
        // language Windows itself is in.
        let chosen = lang
            .unwrap_or_else(|| if prefs.language == "auto" { i18n::from_os() } else { i18n::from_code(&prefs.language) });
        i18n::set(chosen);

        let dark = match prefs.theme {
            ThemeMode::System => cc.integration_info.system_theme != Some(eframe::Theme::Light),
            ThemeMode::Light => false,
            ThemeMode::Dark => true,
        };
        let card_scale = cfg
            .card_scale
            .as_deref()
            .map(CardScale::from_code)
            .unwrap_or(CardScale::Medium);
        let list_view = cfg.list_view.unwrap_or(false);
        let servers = cfg.servers;
        let active = cfg
            .active_id
            .and_then(|id| servers.iter().position(|s| s.id == id))
            .or_else(|| if servers.is_empty() { None } else { Some(0) });
        let host_hwnd = cc.window_handle().ok().and_then(|handle| match handle.as_raw() {
            RawWindowHandle::Win32(win) => Some(win.hwnd.get()),
            _ => None,
        });
        if let Some(hwnd) = host_hwnd {
            use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
            unsafe {
                DwmSetWindowAttribute(hwnd as _, DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                    std::ptr::from_ref(&DWMWCP_ROUND).cast(), std::mem::size_of_val(&DWMWCP_ROUND) as u32);
            }
        }
        let mut app = Self {
            servers,
            active,
            view: View::Home,
            libs: Vec::new(),
            rows: Vec::new(),
            home_row: None,
            grid: Grid::default(),
            detail: None,
            detail_id: String::new(),
            detail_season: None,
            detail_back: View::Home,
            detail_media: DetailMedia::default(),
            detail_jump: false,
            detail_reveal_episode: false,
            episode_grid: cfg.episode_grid,
            search_query: String::new(),
            search_results: Vec::new(),
            favorites: Vec::new(),
            search_term: String::new(),
            status: None,
            loading: false,
            dark,
            card_scale,
            list_view,
            images: ImageCache::new(),
            worker: Worker::spawn(cc.egui_ctx.clone()),
            loader: ImageLoader::spawn(cc.egui_ctx.clone()),
            form: None,
            player: None,
            player_request: 0,
            external_launch: None,
            host_hwnd,
            tray: None,
            autoplay,
            server_menu: false,
            server_menu_at: 0.0,
            server_row_menu: None,
            recommended_rules: TextRules::new(&prefs.library.recommend_filter).unwrap_or_default(),
            prefs,
            settings_view: settings_ui::SettingsView::default(),
            update_check: updates::UpdateCheck::default(),
            search_history: cfg.search_history,
            danmaku_history: cfg.danmaku_history,
            agg_items: Vec::new(),
            agg_queue: Vec::new(),
            agg_tab: 0,
            agg_loading: false,
            pending_view: None,
            style_for: Some(true),
            frame: 0,
            shot_path: shot,
            shot_phase: ShotPhase::Drawing,
            last_saved_view: None,
            hero_index: 0,
            hero_was_visible: false,
            hero_next_at: 0.0,
            hero_previous: None,
        };
        if let Some(error) = config_error {
            app.settings_view.report_config_error(error);
        }
        // --view <name>: open one screen without touching the server list.
        if app.prefs.start_aggregate && app.active.is_some() && !autoplay {
            app.pending_view = Some(View::Aggregate);
        }
        if let Some(name) = view_override.as_deref() {
            app.pending_view = match name {
                "aggregate" => Some(View::Aggregate),
                "favorites" => Some(View::Favorites),
                "library" => Some(View::Library),
                "settings" => Some(View::Settings),
                "detail" => Some(View::Detail),
                "list" => {
                    app.list_view = true;
                    None
                }
                _ => None,
            };
            if name == "settings" && app.active.is_none() {
                app.pending_view = None;
                app.view = View::Settings;
            }
        }

        // A demo screenshot never dials the person's own server: the sample
        // library answers offline, so the picture is the same every run.
        if app.active.is_some() && demo.is_none() {
            app.connect_active();
        }
        if app.prefs.check_updates_on_start && demo.is_none() && app.shot_path.is_none() {
            app.update_check.start(&cc.egui_ctx, true);
        }
        if let Some(name) = demo {
            // Development aid: open one screen of the offline sample library so a
            // screenshot can be taken of it, without touching the saved servers.
            if !app.servers.iter().any(|s| s.sample) {
                app.servers.push(ServerEntry::sample_server());
            }
            app.active = app.servers.iter().position(|s| s.sample);
            app.connect_active();
            let ctx = cc.egui_ctx.clone();
            match name.as_str() {
                "settings" => app.view = View::Settings,
                "favorites" => app.view = View::Favorites,
                "aggregate" => app.view = View::Aggregate,
                "library" => app.view = View::Library,
                "search" => {
                    let term = "the".to_string();
                    app.search_query = term.clone();
                    app.search_term = term.clone();
                    app.view = View::Search;
                    if let Some(t) = app.target() {
                        app.worker.send(Job::Search { target: t, term, include_media: app.prefs.library.search_media });
                    }
                }
                // A development aid: the same library drawn as a list.
                "list" => {
                    app.list_view = true;
                    if let Some(lib) = app.libs.first().cloned() {
                        app.apply(
                            Action::Browse {
                                id: lib.id.clone(),
                                title: lib.name.clone(),
                                include: include_for(&lib.kind),
                            },
                            &ctx,
                        );
                    }
                }
                "library" => {
                    if let Some(lib) = app.libs.first().cloned() {
                        app.apply(
                            Action::Browse {
                                id: lib.id.clone(),
                                title: lib.name.clone(),
                                include: include_for(&lib.kind),
                            },
                            &ctx,
                        );
                    }
                }
                "detail" => {
                    if let Some(item) = sample::home().first().and_then(|r| r.items.first().cloned()) {
                        app.open_detail(item.id);
                    }
                }
                "player" => {
                    if let Some(item) = sample::home().first().and_then(|r| r.items.first().cloned()) {
                        app.start_playback(item);
                    }
                }
                "welcome" => {
                    app.servers.clear();
                    app.active = None;
                    app.rows.clear();
                    app.libs.clear();
                    app.view = View::Home;
                }
                "favourites" | "favorites" => {
                    app.apply(Action::GoFavorites, &ctx);
                }
                _ => {}
            }
        }
        app
    }

    // ------------------------------------------------------------- plumbing

    fn config(&self) -> Config {
        Config {
            servers: self.servers.clone(),
            dark: Some(self.dark),
            card_scale: Some(self.card_scale.code().to_string()),
            list_view: Some(self.list_view),
            episode_grid: self.episode_grid,
            lang: Some(self.prefs.language.clone()),
            active_id: self
                .active
                .and_then(|i| self.servers.get(i).map(|s| s.id.clone())),
            preferences: Some(self.prefs.clone()),
            search_history: if self.prefs.library.search_history { self.search_history.clone() } else { Vec::new() },
            danmaku_history: if self.prefs.danmaku.remember { self.danmaku_history.clone() } else { HashMap::new() },
        }
    }

    fn save(&mut self) {
        if write_config(&self.config(), &config_path()).is_err() {
            self.set_status(i18n::bilingual("Could not save settings", "无法保存设置").into_owned(), true);
        }
    }

    fn server(&self) -> Option<&ServerEntry> {
        self.active.and_then(|i| self.servers.get(i))
    }

    fn target(&self) -> Option<Target> {
        let s = self.server()?;
        if !s.is_signed_in() {
            return None;
        }
        Target::new(s)
    }

    fn set_status(&mut self, text: impl Into<String>, error: bool) {
        self.status = Some((text.into(), error));
    }

    fn clear_error(&mut self) {
        if let Some((_, true)) = self.status {
            self.status = None;
        }
    }

    /// 聚合视界 in the original merges every server. Start from what the active
    /// server already delivered, then pull each other signed-in server in turn.
    fn start_aggregate(&mut self) {
        let mut items: Vec<MediaItem> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for row in &self.rows {
            for it in &row.items {
                if seen.insert(it.id.clone()) {
                    items.push(it.clone());
                }
            }
        }
        self.agg_items = self.active.map(|idx| vec![(idx, items)]).unwrap_or_default();
        self.agg_queue.clear();
        for (i, srv) in self.servers.iter().enumerate() {
            if srv.sample || !srv.is_signed_in() {
                continue;
            }
            if Some(i) == self.active {
                continue;
            }
            self.agg_queue.push(i);
        }
        self.agg_loading = !self.agg_queue.is_empty();
        self.pump_aggregate();
    }

    fn pump_aggregate(&mut self) {
        let Some(idx) = self.agg_queue.first().copied() else {
            self.agg_loading = false;
            return;
        };
        let Some(srv) = self.servers.get(idx).cloned() else {
            self.agg_queue.remove(0);
            self.pump_aggregate();
            return;
        };
        let target = Target {
            base: api::normalize_url(&srv.url),
            session: Session {
                token: srv.token.clone().unwrap_or_default(),
                user_id: srv.user_id.clone().unwrap_or_default(),
                user_name: srv.user_name.clone(),
            },
            sample: false,
            device_id: srv.device_id.clone(),
            direct_playback: srv.direct_playback,
        };
        self.worker.send(Job::Home {
            target,
            libraries: Vec::new(),
            purpose: crate::net::HomePurpose::Aggregate(idx),
        });
        self.agg_queue.remove(0);
    }

    fn connect_active(&mut self) {
        let Some(idx) = self.active else { return };
        let Some(s) = self.servers.get(idx).cloned() else {
            return;
        };
        self.detail = None;
        self.rows.clear();
        self.libs.clear();
        self.home_row = None;
        self.hero_index = 0;
        self.hero_was_visible = false;
        self.hero_previous = None;
        // Show the last payload straight away when it is still reasonably fresh;
        // the network reply replaces it a moment later.
        if !s.sample {
            if let Some(cached) = self.prefs.library.data_cache.then(||
                crate::net::load_rows(&s.id, u64::from(self.prefs.library.data_days) * 86_400)).flatten()
            {
                if !cached.is_empty() {
                    self.rows = cached;
                    self.loading = false;
                    self.view = View::Home;
                }
            }
        }
        if s.sample {
            self.libs = sample::views();
            self.load_home();
            self.view = View::Home;
            return;
        }
        if !s.is_signed_in() {
            self.form = Some(FormState {
                editing: Some(idx),
                url: s.url.clone(),
                username: s.username.clone(),
                password: String::new(),
                error: String::new(),
                    busy: false,
            });
            return;
        }
        if let Some(t) = Target::new(&s) {
            self.loading = true;
            self.view = View::Home;
            self.worker.send(Job::Views { target: t });
        }
    }

    fn load_home(&mut self) {
        let Some(t) = self.target() else { return };
        self.loading = true;
        self.worker.send(Job::Home {
            purpose: crate::net::HomePurpose::Rows,
            target: t,
            libraries: self.libs.clone(),
        });
    }

    fn open_detail(&mut self, id: String) {
        if self.view != View::Detail {
            self.detail_back = self.view.clone();
        }
        self.status = None;
        self.detail_id = id.clone();
        self.detail = None;
        self.detail_season = None;
        self.detail_media = DetailMedia::default();
        self.detail_jump = false;
        self.detail_reveal_episode = false;
        self.view = View::Detail;
        if let Some(t) = self.target() {
            self.loading = true;
            self.worker.send(Job::Detail { target: t, id });
        }
    }

    fn start_playback(&mut self, item: MediaItem) {
        self.start_playback_with(item, PlaybackSelection::default());
    }

    fn apply_playback_preferences(&self, item: &MediaItem, selection: &mut PlaybackSelection) {
        selection.apply_preferences(&self.prefs.playback);
        if self.view == View::Detail && self.detail_media.item_id == item.id {
            selection.resolve_preferences(&self.detail_media.sources);
        }
    }

    fn start_playback_with(&mut self, item: MediaItem, mut selection: PlaybackSelection) {
        let Some(target) = self.player.as_ref().map(|p| p.target.clone()).or_else(|| self.target()) else { return };
        if !target.sample && self.player.is_none() && self.prefs.playback.default_player != PlayerChoice::BuiltIn {
            self.start_external(item, selection, self.prefs.playback.default_player);
            return;
        }
        self.external_launch = None;
        self.apply_playback_preferences(&item, &mut selection);
        let mut previous = self.player.take();
        if let Some(previous) = &mut previous {
            previous.report(&mut self.worker, PlaybackEvent::Stop, 0.0);
        }
        self.player_request += 1;
        let duration = item.runtime_secs().unwrap_or(0.0);
        let pos = item.progress_secs().max(0.0);
        let sample = target.sample;
        if !sample {
            self.worker.send(Job::PreparePlayback {
                request: self.player_request,
                target: target.clone(),
                item: item.id.clone(),
                selection,
            });
        }
        let mut queue = previous.as_mut().filter(|p| p.queue.iter().any(|ep| ep.id == item.id))
            .map(|p| std::mem::take(&mut p.queue))
            .filter(|items| items.len() > 1)
            .or_else(|| self.detail.as_ref()
                .filter(|detail| detail.episodes.iter().any(|ep| ep.id == item.id))
                .map(|detail| detail.episodes.clone()))
            .unwrap_or_else(|| vec![item.clone()]);
        let mut queue_loading = false;
        if item.kind == "Episode" && queue.len() == 1 {
            if let Some(series) = item.series_id.as_ref() {
                self.worker.send(Job::Episodes {
                    request: Some(self.player_request),
                    target: target.clone(),
                    series: series.clone(),
                    season: item.season_id.clone().unwrap_or_default(),
                });
                queue_loading = true;
            }
        }
        if let Some(entry) = queue.iter_mut().find(|ep| ep.id == item.id) {
            *entry = item.clone();
        }
        let key = format!("{}|{}", target.base, item.id);
        let filter = TextRules::new(&self.prefs.danmaku.blocked_words).unwrap_or_default();
        let danmaku = if self.prefs.danmaku.enabled && self.prefs.danmaku.remember {
            self.danmaku_history.get(&key).and_then(|path| player_ui::read_danmaku(path, &filter).ok()).unwrap_or_default()
        } else {
            Vec::new()
        };
        let chapters = item.chapters.clone();
        let intro_range = intro_range(&chapters);
        if !sample && (self.prefs.playback.chapters || self.prefs.playback.preview || self.prefs.playback.intro != IntroMode::Off) {
            self.worker.send(Job::PlaybackChapters { request: self.player_request, target: target.clone(), item: item.id.clone() });
        }
        self.player = Some(Player {
            request: self.player_request,
            target,
            session: None,
            watched: item.played,
            item,
            pos,
            duration,
            playing: true,
            phase: if sample { PlaybackPhase::Ready } else { PlaybackPhase::Loading },
            seekable: sample,
            seek_started: false,
            pending_seek: None,
            buffering: false,
            buffer_pos: 0.0,
            cache_speed: 0.0,
            next_report: 0.0,
            speed: previous.as_ref().map_or(1.0, |p| p.speed),
            volume: previous.as_ref().map_or(100, |p| p.volume),
            muted: previous.as_ref().is_some_and(|p| p.muted),
            tracks: Vec::new(),
            initial_tracks: None,
            menu: None,
            queue,
            queue_loading,
            source_label: None,
            last_activity: f64::NEG_INFINITY,
            pending_click: None,
            topmost: previous.as_ref().is_some_and(|p| p.topmost),
            fill_video: previous.as_ref().map_or(self.prefs.playback.remember_aspect && self.prefs.playback.last_fill, |p| p.fill_video),
            show_cache_speed: previous.as_ref().is_none_or(|p| p.show_cache_speed),
            auto_hide: previous.as_ref().is_none_or(|p| p.auto_hide),
            danmaku,
            danmaku_filter: filter,
            show_danmaku: self.prefs.danmaku.enabled,
            notice: None,
            chapters,
            intro_range,
            intro_skipped: false,
            secondary_id: None,
            secondary_pending: self.prefs.playback.secondary && self.prefs.playback.secondary_language != "no",
            fullscreen_pending: previous.as_ref().map_or(self.prefs.playback.fullscreen, |p| p.fullscreen_pending),
            restore_maximized: previous.as_ref().is_some_and(|p| p.restore_maximized),
            boost_started: None,
            boost_base_speed: None,
            engine: None,
        });
    }

    fn start_external(&mut self, item: MediaItem, mut selection: PlaybackSelection, choice: PlayerChoice) {
        let Some(target) = self.target().filter(|target| !target.sample) else { return };
        let Some(exe) = self.prefs.external.executable(choice) else {
            self.set_status(i18n::bilingual("Select an executable in External player settings", "请先在外部播放器设置中选择程序").into_owned(), true);
            return;
        };
        self.player_request += 1;
        self.external_launch = Some(ExternalLaunch {
            request: self.player_request, exe, choice, resume: item.progress_secs(),
        });
        self.apply_playback_preferences(&item, &mut selection);
        self.worker.send(Job::PrepareExternal { request: self.player_request, target, item,
            selection, count: usize::from(self.prefs.external.episode_count) });
        self.set_status(i18n::bilingual("Preparing external playback…", "正在准备外部播放…").into_owned(), false);
    }

    fn focus_detail_media(&mut self, item: &MediaItem) {
        self.detail_media = DetailMedia { item_id: item.id.clone(), ..Default::default() };
        if let Some(target) = self.target().filter(|target| !target.sample && item.is_playable()) {
            self.detail_media.loading = true;
            self.worker.send(Job::DetailMedia { target, item: item.id.clone() });
        }
    }

    fn apply(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::Open(id) => self.open_detail(id),
            Action::OpenHomeRow(index) => {
                if let Some(row) = self.rows.get(index) {
                    self.home_row = Some(row.clone());
                    self.view = View::HomeRow;
                }
            }
            Action::OpenOnServer(idx, id) => {
                let back = if self.view == View::Detail { &self.detail_back } else { &self.view }.clone();
                if self.active != Some(idx) {
                    self.active = Some(idx);
                    self.save();
                    self.connect_active();
                }
                self.open_detail(id);
                self.detail_back = back;
            }
            Action::Play(item) => self.start_playback(item),
            Action::PlaySelected(item) => {
                let selection = if self.detail_media.item_id == item.id {
                    self.detail_media.selection.clone()
                } else {
                    PlaybackSelection::default()
                };
                self.start_playback_with(item, selection);
            }
            Action::PlayExternal(item, choice) => {
                let selection = if self.detail_media.item_id == item.id { self.detail_media.selection.clone() }
                    else { PlaybackSelection::default() };
                self.start_external(item, selection, choice);
            }
            Action::BackFromDetail => {
                self.view = self.detail_back.clone();
                self.detail = None;
                self.detail_id.clear();
                self.detail_media = DetailMedia::default();
                self.detail_jump = false;
                ctx.memory_mut(|memory| memory.close_popup());
            }
            Action::DetailFocus(item) => self.focus_detail_media(&item),
            Action::ToggleEpisodeGrid => {
                self.episode_grid = !self.episode_grid;
                self.save();
            }
            Action::Browse { id, title, include } => {
                self.grid = Grid {
                    parent: id.clone(),
                    title: title.clone(),
                    include: include.clone(),
                    sort: "SortName".to_string(),
                    items: Vec::new(),
                    total: 0,
                };
                self.view = View::Library;
                self.loading = true;
                if let Some(t) = self.target() {
                    self.worker.send(Job::Browse {
                        target: t,
                        parent: id,
                        include,
                        sort: "SortName".to_string(),
                    });
                }
            }
            Action::DoSearch(term) => {
                let term = term.trim().to_string();
                if term.is_empty() {
                    return;
                }
                self.search_query = term.clone();
                self.search_term = term.clone();
                self.view = View::Search;
                self.search_results.clear();
                self.loading = true;
                if self.prefs.library.search_history {
                    self.search_history.retain(|entry| !entry.eq_ignore_ascii_case(&term));
                    self.search_history.insert(0, term.clone());
                    self.search_history.truncate(20);
                    self.save();
                }
                if let Some(t) = self.target() {
                    self.worker.send(Job::Search { target: t, term, include_media: self.prefs.library.search_media });
                }
            }
            Action::AddServer => {
                self.form = Some(FormState {
                    editing: None,
                    url: String::new(),
                    username: String::new(),
                    password: String::new(),
                    error: String::new(),
                    busy: false,
                });
            }
            Action::EditServer(idx) => {
                if let Some(s) = self.servers.get(idx) {
                    self.form = Some(FormState {
                        editing: Some(idx),
                        url: s.url.clone(),
                        username: s.username.clone(),
                        password: String::new(),
                        error: String::new(),
                        busy: false,
                    });
                }
            }
            Action::RemoveServer(idx) => {
                if idx < self.servers.len() {
                    let removed = self.servers.remove(idx);
                    if self.active == Some(idx) {
                        self.active = None;
                        self.rows.clear();
                        self.libs.clear();
                        self.view = View::Home;
                    } else if let Some(a) = self.active {
                        if a > idx {
                            self.active = Some(a - 1);
                        }
                    }
                    self.set_status(format!("Removed {}", removed.name), false);
                    self.save();
                }
            }
            Action::Connect(idx) => {
                let signed_in = self
                    .servers
                    .get(idx)
                    .map(|s| s.is_signed_in())
                    .unwrap_or(false);
                self.active = Some(idx);
                self.save();
                if signed_in {
                    self.connect_active();
                } else if let Some(s) = self.servers.get(idx) {
                    // A server with no token asks for the password again.
                    let (url, username) = (s.url.clone(), s.username.clone());
                    self.form = Some(FormState {
                        editing: Some(idx),
                        url,
                        username,
                        password: String::new(),
                        error: String::new(),
                        busy: false,
                    });
                }
            }
            Action::SignOut(idx) => {
                if let Some(s) = self.servers.get_mut(idx) {
                    s.token = None;
                    s.user_id = None;
                    s.user_name.clear();
                }
                if self.active == Some(idx) {
                    self.rows.clear();
                    self.libs.clear();
                }
                self.save();
                self.set_status(i18n::t("Signed out").to_string(), false);
            }
            Action::GoAggregate => {
                self.view = View::Aggregate;
                self.start_aggregate();
            }
            Action::GoFavorites => {
                self.view = View::Favorites;
                self.favorites.clear();
                self.loading = true;
                if let Some(t) = self.target() {
                    self.worker.send(Job::Favorites { target: t });
                }
            }
            Action::SetSort(by) => {
                self.grid.sort = by.clone();
                let (id, inc) = (self.grid.parent.clone(), self.grid.include.clone());
                if let Some(t) = self.target() {
                    self.loading = true;
                    self.worker.send(Job::Browse {
                        target: t,
                        parent: id,
                        include: inc,
                        sort: by,
                    });
                }
            }
            Action::SetCardScale(scale) => {
                self.card_scale = scale;
                self.save();
            }
            Action::ToggleListView => {
                self.list_view = !self.list_view;
                self.save();
            }
            Action::SetLang(lang) => {
                i18n::set(lang);
                self.prefs.language = i18n::code(lang).to_owned();
                self.save();
            }
            Action::GoSettings => {
                if self.view != View::Settings {
                    self.settings_view.back = self.view.clone();
                    self.view = View::Settings;
                }
            }
            Action::BackFromSettings => {
                self.flush_settings();
                self.view = self.settings_view.back.clone();
                self.settings_view.dialog = None;
            }
            Action::Settings(command) => self.apply_setting_command(command, ctx),
            Action::GoHome => {
                self.view = View::Home;
                if self.rows.is_empty() {
                    self.load_home();
                }
            }
            Action::GoSearch => self.view = View::Search,
            Action::UseSample => {
                if !self.servers.iter().any(|s| s.sample) {
                    self.servers.push(ServerEntry::sample_server());
                }
                let idx = self
                    .servers
                    .iter()
                    .position(|s| s.sample)
                    .unwrap_or_default();
                self.active = Some(idx);
                self.save();
                self.connect_active();
            }
            Action::TogglePlayed(item) => {
                let played = !item.played;
                if let Some(t) = self.target() {
                    self.worker.send(Job::SetPlayed {
                        target: t,
                        id: item.id.clone(),
                        played,
                    });
                }
                if let Some(d) = &mut self.detail {
                    if d.item.id == item.id {
                        d.item.played = played;
                        d.item.progress_ticks = 0;
                    }
                }
                for row in &mut self.rows {
                    for it in &mut row.items {
                        if it.id == item.id {
                            it.played = played;
                            if played {
                                it.progress_ticks = 0;
                            }
                        }
                    }
                }
                for it in &mut self.grid.items {
                    if it.id == item.id {
                        it.played = played;
                        if played {
                            it.progress_ticks = 0;
                        }
                    }
                }
            }
            Action::ToggleFavorite(item) => {
                let favorite = !item.favorite;
                if let Some(t) = self.target() {
                    self.worker.send(Job::SetFavorite {
                        target: t,
                        id: item.id.clone(),
                        favorite,
                    });
                }
                if let Some(d) = &mut self.detail {
                    if d.item.id == item.id {
                        d.item.favorite = favorite;
                    }
                }
            }
            Action::Season(season_id) => {
                if self.detail_season.as_deref() == Some(&season_id) {
                    return;
                }
                if let (Some(target), Some(detail)) = (self.target(), &mut self.detail) {
                    self.detail_season = Some(season_id.clone());
                    detail.episodes.clear();
                    self.detail_media = DetailMedia::default();
                    self.status = None;
                    self.loading = true;
                    self.worker.send(Job::Episodes { request: None, target, series: detail.item.id.clone(), season: season_id });
                }
            }
            Action::ClosePlayer => {
                if let Some(mut player) = self.player.take() {
                    player.report(&mut self.worker, PlaybackEvent::Stop, 0.0);
                    if player.topmost {
                        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(egui::WindowLevel::Normal));
                    }
                    player.leave_fullscreen(ctx);
                }
            }
            Action::TogglePlayer => {
                if let Some(player) = self.player.as_ref().filter(|p| p.phase == PlaybackPhase::Ended) {
                    let mut item = player.item.clone();
                    item.progress_ticks = 0;
                    self.start_playback(item);
                    return;
                }
                let Some(p) = self.player.as_mut().filter(|p| p.phase == PlaybackPhase::Ready) else { return };
                if p.engine.as_ref().is_some_and(|engine| !engine.set_pause(p.playing)) {
                    return;
                }
                p.playing = !p.playing;
                let event = if p.playing { PlaybackEvent::Unpause } else { PlaybackEvent::Pause };
                p.report(&mut self.worker, event, ctx.input(|i| i.time));
            }
            Action::PlayFromQueue(index) => {
                if let Some(player) = &self.player {
                    if let Some(item) = player.queue.get(index).filter(|item| item.id != player.item.id) {
                        self.start_playback(item.clone());
                    }
                }
            }
            Action::TogglePlayerTopmost => {
                if let Some(player) = &mut self.player {
                    player.topmost = !player.topmost;
                    ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if player.topmost {
                        egui::WindowLevel::AlwaysOnTop
                    } else {
                        egui::WindowLevel::Normal
                    }));
                }
            }
            Action::SetVideoFill(fill) => {
                if let Some(player) = &mut self.player {
                    if player.engine.as_ref().is_none_or(|mpv| mpv.set_property("panscan", if fill { "1" } else { "0" })) {
                        player.fill_video = fill;
                        if self.prefs.playback.remember_aspect {
                            self.prefs.playback.last_fill = fill;
                            self.save();
                        }
                    }
                }
            }
            Action::TogglePlayerCacheSpeed => {
                if let Some(player) = &mut self.player {
                    player.show_cache_speed = !player.show_cache_speed;
                }
            }
            Action::TogglePlayerAutoHide => {
                if let Some(player) = &mut self.player {
                    player.auto_hide = !player.auto_hide;
                }
            }
            Action::ToggleDanmaku => {
                if let Some(player) = &mut self.player {
                    player.show_danmaku = !player.show_danmaku;
                }
            }
            Action::LoadLocalSubtitles | Action::LoadDanmaku => {
                let Some(player) = &mut self.player else { return };
                let subtitles = matches!(action, Action::LoadLocalSubtitles);
                if let Some(path) = player_ui::open_player_file(self.host_hwnd, subtitles) {
                    let message = if subtitles {
                        if player.engine.as_ref().is_some_and(|mpv| mpv.add_subtitle(&path.to_string_lossy())) {
                            return; // The async reply reports whether the subtitle loaded.
                        }
                        "Could not load subtitle file"
                    } else if let Ok(comments) = player_ui::read_danmaku(&path, &player.danmaku_filter) {
                        player.danmaku = comments;
                        player.show_danmaku = true;
                        if self.prefs.danmaku.remember {
                            let key = format!("{}|{}", player.target.base, player.item.id);
                            self.danmaku_history.insert(key, path);
                        }
                        "Danmaku loaded"
                    } else {
                        "Could not read danmaku XML"
                    };
                    player.notice = Some((i18n::t(message).into_owned(), ctx.input(|i| i.time) + 3.0));
                    if !subtitles && self.prefs.danmaku.remember {
                        self.save();
                    }
                }
            }
            Action::SeekBy(_) | Action::SeekTo(_) => {
                let Some(p) = self.player.as_mut().filter(|p| p.phase == PlaybackPhase::Ready && p.seekable) else { return };
                let requested = match action {
                    Action::SeekBy(delta) => p.pending_seek.unwrap_or(p.pos) + delta,
                    Action::SeekTo(pos) => pos,
                    _ => unreachable!(),
                };
                let pos = if p.duration > 0.0 { requested.clamp(0.0, p.duration) } else { requested.max(0.0) };
                if let Some(engine) = &p.engine {
                    if engine.seek_absolute(pos) {
                        p.pending_seek = Some(pos);
                        p.seek_started = false;
                    }
                } else {
                    p.pos = pos;
                }
            }
            Action::SkipIntro(end) => {
                if let Some(player) = &mut self.player {
                    player.intro_skipped = true;
                }
                self.apply(Action::SeekTo(end), ctx);
            }
            Action::SetPlaybackSpeed(speed) => {
                let Some(p) = self.player.as_mut().filter(|p| p.phase == PlaybackPhase::Ready) else { return };
                if p.engine.as_ref().is_none_or(|engine| engine.set_property("speed", &speed.to_string())) {
                    p.speed = speed;
                    p.report(&mut self.worker, PlaybackEvent::RateChange, ctx.input(|i| i.time));
                }
            }
            Action::SetPlaybackVolume { level, commit } => {
                let Some(p) = self.player.as_mut().filter(|p| p.phase == PlaybackPhase::Ready) else { return };
                if p.volume == level || p.engine.as_ref().is_none_or(|engine| engine.set_property("volume", &level.to_string())) {
                    p.volume = level;
                    if commit {
                        p.report(&mut self.worker, PlaybackEvent::VolumeChange, ctx.input(|i| i.time));
                    }
                }
            }
            Action::TogglePlaybackMute => {
                let Some(p) = self.player.as_mut().filter(|p| p.phase == PlaybackPhase::Ready) else { return };
                if p.engine.as_ref().is_none_or(|engine| engine.set_property("mute", if p.muted { "no" } else { "yes" })) {
                    p.muted = !p.muted;
                    p.report(&mut self.worker, PlaybackEvent::VolumeChange, ctx.input(|i| i.time));
                }
            }
            Action::SelectPlaybackTrack(kind, id) => {
                let Some(p) = self.player.as_mut().filter(|p| p.phase == PlaybackPhase::Ready) else { return };
                if let Some(engine) = &p.engine {
                    if kind == TrackKind::Subtitle && id == p.secondary_id && id.is_some() {
                        engine.set_property("secondary-sid", "no");
                        p.secondary_id = None;
                    }
                    engine.select_track(kind, id);
                }
            }
            Action::SelectSecondarySubtitle(id) => {
                let Some(player) = &mut self.player else { return };
                if let Some(engine) = &player.engine {
                    let value = id.map_or_else(|| "no".to_owned(), |id| id.to_string());
                    if engine.set_property("secondary-sid", &value) {
                        player.secondary_id = id;
                    }
                }
            }
            Action::ToggleTheme => {
                self.dark = !self.dark;
                self.prefs.theme = if self.dark { ThemeMode::Dark } else { ThemeMode::Light };
                self.save();
            }
            Action::Refresh => {
                if self.view == View::Home {
                    self.load_home();
                } else if self.view == View::Library {
                    let (id, inc) = (self.grid.parent.clone(), self.grid.include.clone());
                    if let Some(t) = self.target() {
                        self.worker.send(Job::Browse {
                            target: t,
                            parent: id,
                            include: inc,
                            sort: self.grid.sort.clone(),
                        });
                    }
                } else if self.view == View::Detail {
                    let id = self.detail_id.clone();
                    if !id.is_empty() {
                        self.open_detail(id);
                    }
                }
            }
            Action::ToggleFullscreen => {
                if let Some(player) = &mut self.player {
                    if player.fullscreen_pending || ctx.input(|i| i.viewport().fullscreen == Some(true)) {
                        player.leave_fullscreen(ctx);
                    } else {
                        player.fullscreen_pending = true;
                        ctx.request_repaint();
                    }
                }
            }
        }
        let _ = ctx;
    }

    fn handle(&mut self, msg: Msg, ctx: &egui::Context) {
        match msg {
            Msg::Connected {
                server_id,
                base,
                name,
                version,
                session,
                user_name,
            } => {
                if let Some(s) = self.servers.iter_mut().find(|s| s.id == server_id) {
                    let had_name = !s.name.trim().is_empty() && s.name != s.url;
                    // Keep the address form that actually answered, so later calls
                    // use the same http/https.
                    s.url = base;
                    if !had_name && !name.trim().is_empty() {
                        s.name = name;
                    }
                    s.token = Some(session.token);
                    s.user_id = Some(session.user_id);
                    s.user_name = user_name;
                    s.server_version = Some(version);
                }
                self.form = None;
                self.active = self
                    .servers
                    .iter()
                    .position(|s| s.id == server_id)
                    .or(self.active);
                self.set_status(i18n::t("Signed in").to_string(), false);
                self.save();
                self.connect_active();
            }
            Msg::Views(items) => {
                self.libs = items;
                self.loading = false;
                self.load_home();
            }
            Msg::Home(purpose, rows) => {
                if let crate::net::HomePurpose::Aggregate(idx) = purpose {
                    let mut seen: HashSet<String> = HashSet::new();
                    let mut items = Vec::new();
                    for row in &rows {
                        for it in &row.items {
                            if seen.insert(it.id.clone()) {
                                items.push(it.clone());
                            }
                        }
                    }
                    self.agg_items.push((idx, items));
                    self.pump_aggregate();
                    return;
                }
                if let Some(idx) = self.active {
                    if let Some(srv) = self.servers.get(idx) {
                        if !srv.sample && !rows.is_empty() {
                            if self.prefs.library.data_cache {
                                crate::net::save_rows(&srv.id, &rows);
                            }
                        }
                    }
                }
                self.rows = rows;
                self.loading = false;
                if self.view == View::Home && self.rows.is_empty() {
                    self.set_status("This library is empty", false);
                }
            }
            Msg::Browse {
                parent,
                items,
                total,
            } => {
                if parent == self.grid.parent {
                    self.grid.items = items;
                    self.grid.total = total;
                }
                self.loading = false;
            }
            Msg::Search { term, items } => {
                if term == self.search_term {
                    self.search_results = items;
                }
                self.loading = false;
            }
            Msg::Detail(detail) => {
                if detail.item.id == self.detail_id {
                    self.detail_season = detail.episodes.first().and_then(|episode| episode.season_id.clone())
                        .or_else(|| detail.seasons.first().map(|season| season.id.clone()));
                    let initial = detail.episodes.iter().find(|episode| Some(&episode.id) == detail.next_episode.as_ref().map(|ep| &ep.id))
                        .or_else(|| detail.episodes.iter().find(|ep| !ep.played && ep.progress_ticks > 0))
                        .or_else(|| detail.episodes.iter().find(|ep| !ep.played))
                        .or_else(|| detail.episodes.first())
                        .or_else(|| detail.item.is_playable().then_some(&detail.item));
                    if let Some(item) = initial {
                        self.focus_detail_media(item);
                    }
                    self.detail = Some(*detail);
                    self.loading = false;
                }
            }
            Msg::Episodes { request, season, result } => {
                if let Some(request) = request {
                    if let Some(player) = self.player.as_mut().filter(|p| p.request == request) {
                        player.queue_loading = false;
                        if let Ok(mut items) = result {
                            if let Some(item) = items.iter_mut().find(|ep| ep.id == player.item.id) {
                                *item = player.item.clone();
                                player.queue = items;
                            }
                        }
                    }
                } else if self.detail_season.as_deref() == Some(season.as_str()) {
                    match result {
                        Ok(items) => {
                            if let Some(item) = items.iter().find(|item| !item.played).or_else(|| items.first()) {
                                self.focus_detail_media(item);
                            }
                            if let Some(detail) = &mut self.detail {
                                detail.episodes = items;
                            }
                        }
                        Err(message) => self.set_status(message, true),
                    }
                    self.loading = false;
                }
            }
            Msg::DetailMedia { item, result } => {
                if self.view == View::Detail && self.detail_media.item_id == item {
                    self.detail_media.loading = false;
                    self.detail_media.sources = result.unwrap_or_default();
                    self.detail_media.selection.source_id = api::preferred_source(
                        &self.detail_media.sources, &self.prefs.playback.preferred_version
                    ).and_then(|source| source.id.clone());
                }
            }
            Msg::Played { id, played } => {
                if let Some(d) = &mut self.detail {
                    if d.item.id == id {
                        d.item.played = played;
                    }
                    for ep in &mut d.episodes {
                        if ep.id == id {
                            ep.played = played;
                        }
                    }
                }
            }
            Msg::Favorite { id, favorite } => {
                if let Some(d) = &mut self.detail {
                    if d.item.id == id {
                        d.item.favorite = favorite;
                    }
                }
            }
            Msg::Favorites(items) => {
                self.favorites = items;
                self.loading = false;
            }
            Msg::PlaybackPrepared { request, result } => {
                let Some(player) = self.player.as_mut().filter(|p| p.request == request) else { return };
                let prefs = &self.prefs;
                let started = result.and_then(|prepared| {
                    let playback = &prefs.playback;
                    let cache = &prefs.cache;
                    let sub = &prefs.subtitles;
                    let yes = |value| if value { "yes".to_owned() } else { "no".to_owned() };
                    let network = api::network();
                    let direct = player.target.direct_playback || network.proxy == crate::preferences::ProxyMode::Direct;
                    let proxy = if direct { String::new() } else { api::playback_proxy(&prepared.url, true).unwrap_or_default() };
                    let mut options = vec![
                        ("hwdec", playback.decoder.mpv().to_owned()),
                        ("cache", "yes".to_owned()), ("cache-secs", cache.seconds.to_string()),
                        ("demuxer-readahead-secs", cache.seconds.to_string()),
                        ("demuxer-max-bytes", format!("{}MiB", cache.megabytes)),
                        ("tls-verify", yes(!network.ignore_ssl)), ("http-proxy", proxy),
                        ("vd-lavc-fast", yes(playback.fast_decode)),
                        ("stretch-image-subs-to-screen", yes(playback.stretch_subs)),
                        ("sub-font-size", sub.size.to_string()), ("sub-pos", sub.position.to_string()),
                        ("secondary-sub-pos", sub.secondary_position.to_string()),
                        ("sub-outline-size", sub.border.to_string()), ("sub-shadow-offset", sub.shadow.to_string()),
                        ("sub-scale", sub.scale.to_string()), ("sub-bold", yes(sub.bold)),
                        ("sub-color", format!("#{:08X}", sub.color)), ("sub-outline-color", format!("#{:08X}", sub.border_color)),
                        ("sub-back-color", format!("#{:08X}", sub.shadow_color)),
                        ("sub-ass-override", sub.ass.mpv().to_owned()),
                        ("secondary-sub-ass-override", sub.secondary_ass.mpv().to_owned()),
                        ("volume", player.volume.to_string()), ("speed", player.speed.to_string()),
                        ("mute", yes(player.muted)), ("panscan", if player.fill_video { "1".into() } else { "0".into() }),
                    ];
                    if playback.stereo { options.push(("audio-channels", "stereo".into())); }
                    if !playback.audio_language.is_empty() { options.push(("alang", playback.audio_language.clone())); }
                    if prepared.selection.subtitle_index == Some(-1) {
                        options.push(("sid", "no".into()));
                    } else if !playback.subtitle_language.is_empty() && playback.subtitle_language != "no" {
                        options.push(("slang", playback.subtitle_language.clone()));
                    }
                    let threshold = u64::from(cache.disk_gb) * 1_073_741_824;
                    let mut disk_cache = false;
                    if prepared.source_bytes.is_some_and(|size| size > 0 && size <= threshold) {
                        let dir = cache.directory();
                        if std::fs::create_dir_all(&dir).is_ok() {
                            disk_cache = true;
                            options.push(("demuxer-cache-dir", dir.to_string_lossy().into_owned()));
                            options.push(("demuxer-cache-unlink-files", "whendone".into()));
                        } else {
                            player.notice = Some((i18n::bilingual("Could not open the disk cache folder", "无法访问磁盘缓存目录").into_owned(),
                                ctx.input(|i| i.time) + 8.0));
                        }
                    }
                    options.push(("cache-on-disk", yes(disk_cache)));
                    let resume = if player.duration > 0.0 {
                        player.pos.min((player.duration - 1.0).max(0.0))
                    } else {
                        player.pos
                    };
                    if resume > 0.0 {
                        // Supply the resume point before mpv starts buffering or decoding.
                        options.push(("start", format!("{resume:.3}")));
                    }
                    let (engine, rejected) = crate::mpv::Mpv::new(ctx, &settings_ui::mpv_conf(), &options, playback.log)?;
                    if !rejected.is_empty() {
                        let message = format!("{} · {}", i18n::bilingual("Could not apply playback settings", "部分播放设置未生效"), rejected.join(", "));
                        if playback.log { player_log(&message); }
                        player.notice = Some((message, ctx.input(|i| i.time) + 8.0));
                    }
                    if playback.log {
                        player_log("Opened libmpv and scheduled media loading");
                    }
                    if !engine.play(&prepared.url) {
                        return Err("mpv could not open the media".into());
                    }
                    if prepared.session.method != api::PlayMethod::Transcode {
                        player.initial_tracks = Some((prepared.selection, prepared.subtitle_url));
                    } else if let Some(url) = prepared.subtitle_url {
                        player.initial_tracks = Some((PlaybackSelection::default(), Some(url)));
                    }
                    player.session = Some(prepared.session);
                    player.source_label = prepared.source_label;
                    player.engine = Some(engine);
                    Ok(())
                });
                if let Err(error) = started {
                    if prefs.playback.log { player_log(&error); }
                    player.phase = PlaybackPhase::Failed(error);
                }
            }
            Msg::PlaybackChapters { request, chapters } => {
                if let Some(player) = self.player.as_mut().filter(|p| p.request == request && !chapters.is_empty()) {
                    player.intro_range = intro_range(&chapters);
                    player.chapters = chapters;
                }
            }
            Msg::ExternalPrepared { request, result } => {
                let Some(launch) = self.external_launch.take_if(|launch| launch.request == request) else { return };
                let started = result.and_then(|queue| {
                    let mut command = std::process::Command::new(launch.exe);
                    let first = &queue[0];
                    if launch.choice == PlayerChoice::Mpv {
                        let selection = &first.selection;
                        if !selection.audio_language.is_empty() {
                            command.arg(format!("--alang={}", selection.audio_language));
                        }
                        if selection.subtitle_index == Some(-1) {
                            command.arg("--sid=no");
                        } else if !selection.subtitle_language.is_empty() && selection.subtitle_language != "no" {
                            command.arg(format!("--slang={}", selection.subtitle_language));
                        }
                        command.arg(format!("--tls-verify={}", if api::network().ignore_ssl { "no" } else { "yes" }));
                        if launch.resume > 0.0 { command.arg(format!("--start={:.3}", launch.resume)); }
                        if let Some(proxy) = api::playback_proxy(&first.url, self.prefs.external.mpv_proxy) {
                            command.arg(format!("--http-proxy={proxy}"));
                        } else {
                            for key in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"] {
                                command.env_remove(key);
                            }
                        }
                        command.arg("--").args(queue.iter().map(|target| &target.url))
                            .spawn().map_err(|_| "Could not open external mpv".to_owned())?;
                    } else {
                        use std::io::Write;
                        if let Some(proxy) = crate::platform::inherited_http_proxy() {
                            command.env("HTTP_PROXY", proxy);
                        }
                        let mut playlist = tempfile::Builder::new().prefix("MediaLite-").suffix(".m3u8")
                            .tempfile().map_err(|_| "Could not create the external playlist".to_owned())?;
                        writeln!(playlist, "#EXTM3U").map_err(|_| "Could not write the external playlist".to_owned())?;
                        for media in queue {
                            writeln!(playlist, "{}", media.url).map_err(|_| "Could not write the external playlist".to_owned())?;
                        }
                        let path = playlist.into_temp_path();
                        let mut child = command.arg(&path).spawn()
                            .map_err(|_| "Could not open PotPlayer".to_owned())?;
                        std::thread::spawn(move || {
                            let _ = child.wait();
                            drop(path);
                        });
                    }
                    Ok(())
                });
                match started {
                    Ok(()) => {
                        self.set_status(i18n::bilingual("External player opened", "已打开外部播放器").into_owned(), false);
                        if self.prefs.external.minimize_main {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                    }
                    Err(message) => self.set_status(message, true),
                }
            }
            Msg::WebDav(result) => self.webdav_result(result),
            Msg::Failed { what, message } => {
                if what == "progress" {
                    self.set_status(format!("{}: {}", i18n::t(&what), i18n::sentence(&message)), true);
                    return;
                }
                self.loading = false;
                let text = i18n::sentence(&message);
                // A failed sign-in belongs in the dialog it came from, with the
                // fields still there to correct.
                if what == "connect" {
                    if let Some(form) = &mut self.form {
                        form.busy = false;
                        form.error = text;
                    } else {
                        self.set_status(format!("{}: {}", i18n::t(&what), text), true);
                    }
                } else {
                    self.set_status(format!("{}: {}", i18n::t(&what), text), true);
                }
            }
        }
    }

    fn tick_player(&mut self, ctx: &egui::Context, now: f64) {
        use crate::mpv::Event;

        let Some(player) = &mut self.player else { return };
        if player.target.sample && player.playing && player.phase == PlaybackPhase::Ready {
            player.pos += f64::from(ctx.input(|i| i.stable_dt).min(0.25)) * player.speed;
            if player.duration > 0.0 && player.pos >= player.duration {
                player.pos = player.duration;
                player.phase = PlaybackPhase::Ended;
                player.playing = false;
            }
        }

        let mut first_frame = false;
        while let Some(event) = player.engine.as_ref().and_then(crate::mpv::Mpv::next_event) {
            match event {
                Event::Log(message) => player_log(&message),
                Event::Loaded if player.phase == PlaybackPhase::Loading => player.phase = PlaybackPhase::Starting,
                Event::Restarted if player.phase == PlaybackPhase::Starting => first_frame = true,
                Event::SeekStarted if player.pending_seek.is_some() => player.seek_started = true,
                Event::Restarted if player.seek_started => {
                    player.seek_started = false;
                    player.pos = player.pending_seek.take().unwrap_or(player.pos);
                    player.report(&mut self.worker, PlaybackEvent::Progress, now);
                }
                Event::SeekFailed => {
                    player.seek_started = false;
                    player.pending_seek = None;
                    player.notice = Some((i18n::t("Could not seek in this media").into_owned(), now + 3.0));
                }
                Event::SubtitleAdded(ok) => {
                    let message = if ok { "Subtitle file loaded" } else { "Could not load subtitle file" };
                    player.notice = Some((i18n::t(message).into_owned(), now + 3.0));
                }
                Event::SettingFailed => {
                    player.notice = Some((i18n::t("Could not apply playback setting").into_owned(), now + 3.0));
                }
                Event::Ended | Event::Failed(_) => {
                    player.playing = false;
                    player.report(&mut self.worker, PlaybackEvent::Stop, now);
                    player.phase = match event {
                        Event::Failed(code) => PlaybackPhase::Failed(format!("mpv could not play this media ({code})")),
                        _ if player.phase != PlaybackPhase::Ready => PlaybackPhase::Failed("The media ended before it finished loading".into()),
                        _ => PlaybackPhase::Ended,
                    };
                    player.pending_seek = None;
                    player.engine = None;
                    break;
                }
                Event::Position(value) if player.pending_seek.is_none() => {
                    player.pos = value;
                }
                Event::Duration(value) => player.duration = value,
                Event::Paused(paused) => player.playing = !paused,
                Event::Seekable(value) => player.seekable = value,
                Event::Buffering(value) => player.buffering = value,
                Event::BufferPosition(value) => player.buffer_pos = value,
                Event::CacheSpeed(value) => player.cache_speed = value,
                Event::Tracks(tracks) => {
                    if self.prefs.playback.log && tracks.len() != player.tracks.len() {
                        player_log(&format!("Track inventory: {}", tracks.len()));
                    }
                    player.tracks = tracks;
                }
                Event::SelectedTrack(kind, id) => {
                    if self.prefs.playback.log { player_log(&format!("Track selection: {id:?}")); }
                    for track in player.tracks.iter_mut().filter(|track| track.kind == kind) {
                        track.selected = Some(track.id) == id;
                    }
                }
                _ => {}
            }
        }

        if let Some(engine) = &player.engine {
            let report_start = first_frame && player.phase == PlaybackPhase::Starting;
            if report_start {
                player.phase = PlaybackPhase::Ready;
            }
            if player.phase == PlaybackPhase::Ready || (player.phase != PlaybackPhase::Loading && !player.tracks.is_empty()) {
                if let Some((selection, subtitle_url)) = player.initial_tracks.take() {
                    if selection.subtitle_index.is_none() {
                        if let Some(url) = &subtitle_url {
                            engine.add_subtitle(url);
                        }
                    }
                    for (kind, index) in [(TrackKind::Audio, selection.audio_index), (TrackKind::Subtitle, selection.subtitle_index)] {
                        if let Some(index) = index {
                            if kind == TrackKind::Subtitle && index == -1 {
                                engine.select_track(kind, None);
                            } else if let Some(track) = player.tracks.iter().find(|track| track.kind == kind && track.stream_index == Some(index)) {
                                engine.select_track(kind, Some(track.id));
                            } else if kind == TrackKind::Subtitle {
                                if let Some(url) = &subtitle_url {
                                    engine.add_subtitle(url);
                                }
                            }
                        }
                    }
                }
            }
            if player.phase == PlaybackPhase::Ready && player.pending_seek.is_none() {
                if player.secondary_pending && !player.tracks.is_empty() {
                    let language = &self.prefs.playback.secondary_language;
                    let secondary = player.tracks.iter().find(|track|
                        track.kind == TrackKind::Subtitle && !track.selected &&
                        (language.is_empty() || track.language.as_deref().is_some_and(|code|
                            api::matches_playback_language(code, language)))
                    ).map(|track| track.id);
                    if let Some(id) = secondary {
                        engine.set_property("secondary-sid", &id.to_string());
                        player.secondary_id = Some(id);
                    }
                    player.secondary_pending = false;
                }
                if self.prefs.playback.intro == IntroMode::Auto && !player.intro_skipped && player.seekable {
                    if let Some((start, end)) = player.intro_range {
                        if (start..end).contains(&player.pos) && engine.seek_absolute(end) {
                            player.pending_seek = Some(end);
                            player.intro_skipped = true;
                        }
                    }
                }
            }
            if report_start {
                player.report(&mut self.worker, PlaybackEvent::Start, now);
            }
            if player.phase == PlaybackPhase::Ready && player.pending_seek.is_none() {
                if now >= player.next_report {
                    player.report(&mut self.worker, PlaybackEvent::Progress, now);
                }
                ctx.request_repaint_after(std::time::Duration::from_secs_f64((player.next_report - now).max(0.0)));
            }
        }
        if !player.watched && !player.target.sample && matches!(player.phase, PlaybackPhase::Ready | PlaybackPhase::Ended)
            && player.duration > 0.0 && player.pos > 0.0 &&
            player.pos / player.duration >= f64::from(self.prefs.watched_percent) / 100.0
        {
            player.watched = true;
            self.worker.send(Job::SetPlayed {
                target: player.target.clone(), id: player.item.id.clone(), played: true,
            });
        }
        if player.target.sample && player.playing {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    // --------------------------------------------------------------- images

    fn texture(&mut self, item: &MediaItem, kind: &str, width: i64, now: f64) -> Option<TextureId> {
        let (tag, owner) = if kind == "Backdrop" {
            (item.backdrop_tag.as_deref()?, item.backdrop_id.as_deref().unwrap_or(&item.id))
        } else {
            (item.image_tag.as_deref()?, item.image_id.as_deref().unwrap_or(&item.id))
        };
        self.artwork(owner, kind, tag, width, now)
    }

    fn artwork(&mut self, owner: &str, kind: &str, tag: &str, width: i64, now: f64) -> Option<TextureId> {
        let s = self.servers.get(self.active?)?;
        if s.sample {
            return None;
        }
        let base = api::normalize_url(&s.url);
        let token = s.token.clone().unwrap_or_default();
        if tag.is_empty() {
            return None;
        }
        let (width, quality) = match self.prefs.library.quality {
            Quality::Low => (width * 2 / 3, 65),
            Quality::Medium => (width, 90),
            Quality::High => (width * 3 / 2, 95),
        };
        let key = format!("{}|{}|{}|{}|{}|{}", s.id, owner, kind, tag, width, quality);
        if let Some(tex) = self.images.peek(&key, now) {
            return Some(tex);
        }
        if !self.images.known(&key, now) {
            let url = api::image_url(&base, &token, owner, kind, tag, width, quality);
            self.images.requested.insert(key.clone());
            self.loader.request(key, url, u64::from(self.prefs.library.image_days) * 86_400);
        }
        None
    }

    fn ingest_images(&mut self, ctx: &egui::Context) {
        let batch = self.loader.poll();
        if !batch.is_empty() {
            let now = ctx.input(|i| i.time);
            self.images.ingest(ctx, batch, now);
        }
    }

    // ------------------------------------------------------------ chrome

    /// The items that can be shown in the hero carousel: the first item of each
    /// row, deduplicated, so the hero cycles several backdrops.
    fn hero_items(&self) -> Vec<MediaItem> {
        if self.prefs.library.hide_recommend {
            return Vec::new();
        }
        let allowed = |item: &MediaItem| {
            !std::iter::once(&item.official_rating).chain(&item.genres).chain(std::iter::once(&item.kind))
                .any(|tag| self.recommended_rules.matches(tag))
        };
        let mut out: Vec<MediaItem> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for row in &self.rows {
            if let Some(it) = row.items.iter().find(|item| allowed(item)) {
                if seen.insert(it.series_id.as_ref().unwrap_or(&it.id).clone()) {
                    out.push(it.clone());
                }
            }
        }
        if out.len() < 2 {
            out.extend(self.rows.iter().flat_map(|row| &row.items)
                .filter(|item| allowed(item))
                .filter(|item| seen.insert(item.series_id.as_ref().unwrap_or(&item.id).clone()))
                .take(3).cloned());
        }
        out
    }

    fn draw_floating_topbar(&mut self, ui: &mut Ui, ctx: &egui::Context, p: &Palette, actions: &mut Vec<Action>) {
        let full = ui.max_rect();
        let bar = full.with_max_y(full.top() + home_ui::TOP_BAR_HEIGHT);
        let painter = ui.painter();
        let over_art = self.view == View::Home && self.hero_was_visible;
        let dark_controls = p.dark || over_art;
        let ink = if dark_controls { Color32::from_rgb(242, 244, 247) } else { p.text };
        let muted = if dark_controls { Color32::from_rgb(190, 197, 205) } else { p.dim };
        let fill = if over_art { Color32::from_rgba_unmultiplied(22, 27, 34, 178) } else { p.card };
        let line = if dark_controls { Color32::from_rgba_unmultiplied(153, 160, 169, 56) } else { p.line };
        let hover = if dark_controls { Color32::from_white_alpha(25) } else { Color32::from_black_alpha(15) };
        if over_art {
            let scrim = Color32::from_black_alpha(111);
            painter.add(home_ui::gradient(bar.expand2(Vec2::new(0.0, 7.0)),
                [scrim, scrim, Color32::TRANSPARENT, Color32::TRANSPARENT]));
        } else {
            painter.rect_filled(bar, 0.0, p.bg);
        }
        let window_buttons = home_ui::window_buttons(ui, bar, fill, line, ink);
        let last_tool_x = window_buttons.left() - 28.0;
        let tools_left = last_tool_x - 106.0;
        let capsule = Rect::from_min_size(
            Pos2::new(bar.left() + 32.0, bar.center().y - 18.0), Vec2::new(211.0, 36.0),
        );
        let center_x = bar.center().x;
        let half_width = ((tools_left - center_x - 12.0).min(center_x - capsule.right() - 12.0)).clamp(102.0, 171.0);
        let container = Rect::from_center_size(bar.center(), Vec2::new(half_width * 2.0, 36.0));

        for (index, (left, right)) in [(capsule.right() + 6.0, container.left() - 6.0),
            (container.right() + 6.0, tools_left - 6.0)].into_iter().enumerate()
        {
            let area = Rect::from_min_max(Pos2::new(left, bar.top() + 6.0), Pos2::new(right, bar.bottom()));
            home_ui::drag_window(ui, area, ui.id().with(("main-drag", index)));
        }

        let cap_resp = ui.interact(capsule, ui.id().with("server-capsule"), Sense::click());
        if cap_resp.clicked() {
            self.server_menu = !self.server_menu;
            if self.server_menu { self.server_menu_at = ctx.input(|input| input.time); }
        }
        painter.rect(capsule, 18.0, fill, Stroke::new(1.0_f32, line));
        if cap_resp.hovered() || self.server_menu { painter.rect_filled(capsule, 18.0, hover); }
        let online = self.server().is_some_and(ServerEntry::is_signed_in);
        painter.circle_filled(capsule.left_center() + Vec2::new(15.0, 0.0), 4.0, if online { p.ok } else { muted });
        let server_name = self.server().map(|server| {
            if server.name.is_empty() { server.url.as_str() } else { server.name.as_str() }
        }).unwrap_or_else(|| if i18n::is_zh() { "选择服务器" } else { "Select server" });
        let server_label = theme::clip_px(painter, server_name, 13.0, capsule.width() - 59.0, ink);
        painter.text(capsule.left_center() + Vec2::new(28.0, 0.0), Align2::LEFT_CENTER,
            server_label, FontId::proportional(13.0), ink);
        let arrow = capsule.right_center() - Vec2::new(15.0, 0.0);
        let drop = if self.server_menu { -1.0 } else { 1.0 };
        painter.add(egui::Shape::line(vec![
            arrow + Vec2::new(-3.3, -1.6 * drop), arrow + Vec2::new(0.0, 2.0 * drop),
            arrow + Vec2::new(3.3, -1.6 * drop),
        ], Stroke::new(1.4_f32, muted)));
        cap_resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, server_name));

        painter.rect(container, 18.0, fill, Stroke::new(1.0_f32, line));
        let segment_width = (container.width() - 6.0) / 3.0;
        for (index, (icon, label, view, action)) in [
            ("home", "Home", View::Home, Action::GoHome),
            ("heart", "Favorites", View::Favorites, Action::GoFavorites),
            ("infinity", "Aggregate", View::Aggregate, Action::GoAggregate),
        ].into_iter().enumerate() {
            let segment = Rect::from_min_size(Pos2::new(container.left() + 3.0 + index as f32 * segment_width,
                container.top() + 3.0), Vec2::new(segment_width, 30.0));
            let active = self.view == view || view == View::Home && self.view == View::HomeRow;
            let response = ui.interact(segment, ui.id().with(("nav", index)), Sense::click());
            if active {
                let color = if dark_controls { Color32::from_rgba_unmultiplied(237, 241, 245, 36) } else { p.card_hover };
                painter.rect_filled(segment, 15.0, color);
            } else if response.hovered() || response.has_focus() {
                painter.rect_filled(segment, 15.0, hover);
            }
            let color = if active || response.hovered() { ink } else { muted };
            let label = theme::tl(label);
            let galley = painter.layout_no_wrap(label.clone(), FontId::proportional(13.0), color);
            let start = segment.center().x - (galley.size().x + 23.0) * 0.5;
            theme::icon(ui, icon, Pos2::new(start + 7.0, segment.center().y), 15.0, color);
            painter.galley(Pos2::new(start + 23.0, segment.center().y - galley.size().y * 0.5), galley, color);
            response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, &label));
            if response.clicked() { actions.push(action); }
        }

        for (index, (icon, action, label)) in [
            ("gear", Action::GoSettings, "Settings"),
            ("refresh", Action::Refresh, "Refresh"),
            ("search", Action::GoSearch, "Search"),
        ].into_iter().enumerate() {
            let center = Pos2::new(last_tool_x - index as f32 * 44.0, bar.center().y);
            let button = Rect::from_center_size(center, Vec2::splat(36.0));
            let response = ui.interact(button, ui.id().with(("home-tool", icon)), Sense::click());
            painter.rect(button, 18.0, fill, Stroke::new(1.0_f32, line));
            if response.hovered() || response.has_focus() { painter.rect_filled(button, 18.0, hover); }
            theme::icon(ui, icon, center, 17.0, if response.hovered() { ink } else { muted });
            let label = theme::tl(label);
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
            if response.on_hover_text(label).clicked() { actions.push(action); }
        }

        if let Some((text, error)) = self.status.as_ref().filter(|(_, error)| *error || self.view != View::Home) {
            let available = tools_left - container.right() - 24.0;
            if available >= 60.0 {
                let color = if *error { p.warn } else { p.ok };
                let short = theme::clip_px(painter, text, 12.0, available - 16.0, color);
                let label = painter.layout_no_wrap(short, FontId::proportional(12.0), color);
                let rect = Rect::from_center_size(Pos2::new(tools_left - 18.0 - label.size().x * 0.5, bar.center().y),
                    Vec2::new(label.size().x + 16.0, 28.0));
                painter.rect_filled(rect, 14.0, fill);
                painter.galley(rect.center() - label.size() * 0.5, label, color);
                ui.interact(rect, ui.id().with("server-status"), Sense::hover()).on_hover_text(text);
            }
        }

        // ----- server dropdown, anchored under the pill -----
        if self.server_menu {
            let entries: Vec<(String, String, bool)> = self
                .servers
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let name = if s.name.is_empty() {
                        s.url.clone()
                    } else {
                        s.name.clone()
                    };
                    (name, s.url.clone(), self.active == Some(i))
                })
                .collect();
            let mut pick: Option<usize> = None;
            let mut act: Option<Action> = None;
            // The click that opened the panel must not also press a row.
            let armed = ctx.input(|i| i.time) - self.server_menu_at > 0.25;
            // Measured: a 340 x 281 plate at (28, 54), rows 58 tall on a 60 pitch,
            // the current server marked by a 324 x 58 highlight rather than a tick.
            let anchor_pos = Pos2::new(bar.left() + 28.0, capsule.bottom() + 6.0);

            egui::Area::new(egui::Id::new("server-menu"))
                .fixed_pos(anchor_pos)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    egui::Frame::none()
                        .fill(Color32::from_rgba_premultiplied(0x16, 0x1A, 0x20, 250))
                        .rounding(19.0)
                        .inner_margin(egui::Margin::same(8.0))
                        .show(ui, |ui| {
                            ui.set_width(324.0);
                            for (i, (name, url, active)) in entries.iter().enumerate() {
                                let (rect, resp) = ui.allocate_exact_size(
                                    Vec2::new(324.0, 58.0),
                                    Sense::click(),
                                );
                                let fill = if *active {
                                    Some(Color32::from_rgb(0x41, 0x43, 0x47))
                                } else if resp.hovered() {
                                    Some(Color32::from_rgba_premultiplied(0x2E, 0x33, 0x39, 200))
                                } else {
                                    None
                                };
                                if let Some(f) = fill {
                                    ui.painter().rect_filled(rect, 10.0, f);
                                }
                                // 32x32 plate with a green dot, as in the original
                                let icon_c = Pos2::new(rect.left() + 16.0 + 16.0, rect.center().y);
                                ui.painter().circle_filled(
                                    icon_c,
                                    16.0,
                                    Color32::from_rgb(0x25, 0x2B, 0x32),
                                );
                                ui.painter().circle_filled(
                                    icon_c,
                                    7.0,
                                    Color32::from_rgb(0x50, 0xB6, 0x49),
                                );
                                ui.painter().text(
                                    Pos2::new(rect.left() + 44.0, rect.center().y - 9.0),
                                    Align2::LEFT_TOP,
                                    theme::ellipsize(name, 22),
                                    FontId::proportional(14.0),
                                    p.text,
                                );
                                ui.painter().text(
                                    Pos2::new(rect.left() + 44.0, rect.center().y + 9.0),
                                    Align2::LEFT_TOP,
                                    theme::ellipsize(url, 34),
                                    FontId::proportional(11.5),
                                    p.dim,
                                );
                                // per-server actions live behind a "…" button
                                let dots = Rect::from_center_size(
                                    Pos2::new(rect.right() - 20.0, rect.center().y),
                                    Vec2::splat(30.0),
                                );
                                let dresp =
                                    ui.interact(dots, ui.id().with(("srv-more", i)), Sense::click());
                                if dresp.hovered() {
                                    ui.painter().circle_filled(
                                        dots.center(),
                                        14.0,
                                        Color32::from_rgba_premultiplied(0x2E, 0x33, 0x39, 220),
                                    );
                                }
                                ui.painter().text(
                                    dots.center(),
                                    Align2::CENTER_CENTER,
                                    "\u{22EF}",
                                    FontId::proportional(17.0),
                                    p.dim,
                                );
                                if dresp.clicked() && armed {
                                    self.server_row_menu =
                                        Some((i, Pos2::new(dots.left() - 120.0, dots.bottom() + 2.0)));
                                }
                                if resp.clicked() && armed {
                                    pick = Some(i);
                                }
                                ui.add_space(2.0);
                            }
                            ui.add_space(6.0);
                            // "Add a server" is a stadium button in the original
                            let (brect, bresp) =
                                ui.allocate_exact_size(Vec2::new(324.0, 36.0), Sense::click());
                            ui.painter().rect_filled(
                                brect,
                                18.0,
                                if bresp.hovered() {
                                    Color32::from_rgb(0x3A, 0x3F, 0x46)
                                } else {
                                    Color32::from_rgb(0x26, 0x2B, 0x31)
                                },
                            );
                            ui.painter().text(
                                brect.center(),
                                Align2::CENTER_CENTER,
                                theme::tl("Add a server"),
                                FontId::proportional(13.0),
                                p.text,
                            );
                            if bresp.clicked() && armed {
                                act = Some(Action::AddServer);
                            }
                            // server management lives here too
                            let (mrect, mresp) =
                                ui.allocate_exact_size(Vec2::new(324.0, 36.0), Sense::click());
                            if mresp.hovered() {
                                ui.painter().rect_filled(
                                    mrect,
                                    10.0,
                                    Color32::from_rgba_premultiplied(0x2E, 0x33, 0x39, 200),
                                );
                            }
                            ui.painter().text(
                                Pos2::new(mrect.left() + 14.0, mrect.center().y),
                                Align2::LEFT_CENTER,
                                theme::tl("Server settings"),
                                FontId::proportional(13.0),
                                p.dim,
                            );
                            if mresp.clicked() && armed {
                                act = Some(Action::GoSettings);
                            }
                        });
                });

            // ---- per-server actions ----
            if let Some((row, pos)) = self.server_row_menu {
                let mut chosen: Option<Action> = None;
                let mut close = false;
                egui::Area::new(egui::Id::new("server-row-menu"))
                    .fixed_pos(pos)
                    .order(egui::Order::Tooltip)
                    .show(ctx, |ui| {
                        egui::Frame::none()
                            .fill(Color32::from_rgb(0x1A, 0x1E, 0x24))
                            .rounding(8.0)
                            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x2E, 0x33, 0x39)))
                            .inner_margin(egui::Margin::same(6.0))
                            .show(ui, |ui| {
                                for (label, action) in [
                                    ("Edit", Action::EditServer(row)),
                                    ("Remove", Action::RemoveServer(row)),
                                ] {
                                    let (r, resp) = ui.allocate_exact_size(
                                        Vec2::new(150.0, 34.0),
                                        Sense::click(),
                                    );
                                    if resp.hovered() {
                                        ui.painter().rect_filled(
                                            r,
                                            6.0,
                                            Color32::from_rgba_premultiplied(
                                                0x2E, 0x33, 0x39, 220,
                                            ),
                                        );
                                    }
                                    ui.painter().text(
                                        Pos2::new(r.left() + 10.0, r.center().y),
                                        Align2::LEFT_CENTER,
                                        theme::tl(label),
                                        FontId::proportional(13.0),
                                        p.text,
                                    );
                                    if resp.clicked() {
                                        chosen = Some(action.clone());
                                    }
                                }
                            });
                    });
                if let Some(a) = chosen {
                    self.server_row_menu = None;
                    self.server_menu = false;
                    close = true;
                    actions.push(a);
                } else if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.server_row_menu = None;
                }
                let _ = close;
            }

            if let Some(i) = pick {
                self.active = Some(i);
                self.server_menu = false;
                self.save();
                self.connect_active();
            }
            if let Some(a) = act {
                self.server_menu = false;
                actions.push(a);
            }
            // Esc or a click elsewhere dismisses it.
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.server_menu = false;
            }
        }

    }

    fn draw_content(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        p: &Palette,
        actions: &mut Vec<Action>,
    ) {
        if self.active.is_none() {
            self.draw_welcome(ui, p, actions);
            return;
        }
        match self.view {
            View::Home => self.draw_home(ui, ctx, p, actions),
            View::HomeRow => {
                if let Some(row) = self.home_row.take() {
                    ui.add_space(88.0);
                    ui.horizontal(|ui| {
                        ui.add_space(64.0);
                        ui.heading(theme::heading(&row.title, 25.0, p.text));
                    });
                    ui.add_space(18.0);
                    ScrollArea::vertical()
                        .id_source(("home-row-view", &row.title))
                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.draw_wall(ui, &row.items, ctx, p, actions));
                    self.home_row = Some(row);
                }
            }
            View::Library => self.draw_grid_view(ui, ctx, p, actions),
            View::Search => self.draw_search(ui, ctx, p, actions),
            View::Detail => self.draw_detail(ui, ctx, p, actions),
            View::Favorites => self.draw_favorites(ui, ctx, p, actions),
            View::Aggregate => self.draw_aggregate(ui, ctx, p, actions),
            View::Settings => self.draw_settings(ui, p, actions),
        }
    }

    // -------------------------------------------------------------- welcome

    fn draw_welcome(&mut self, ui: &mut Ui, p: &Palette, actions: &mut Vec<Action>) {
        let rect = ui.max_rect();
        let painter = ui.painter();
        painter.circle_filled(
            Pos2::new(rect.right() - 120.0, rect.top() + 80.0),
            260.0,
            p.accent.gamma_multiply(0.06),
        );
        painter.circle_filled(
            Pos2::new(rect.left() + 60.0, rect.bottom() - 60.0),
            200.0,
            p.accent.gamma_multiply(0.05),
        );

        ui.vertical_centered(|ui| {
            ui.add_space(70.0);
            let (r, _) = ui.allocate_exact_size(Vec2::new(64.0, 64.0), Sense::hover());
            ui.painter().rect_filled(r, 18.0, p.accent);
            theme::icon(ui, "play", r.center() + Vec2::new(2.0, 0.0), 32.0, Color32::WHITE);
            ui.add_space(14.0);
            ui.label(theme::heading("Media Lite", 30.0, p.text));
            ui.add_space(6.0);
            ui.label(theme::body(
                "A desktop player for your Emby server. Add as many servers as you like \u{2014} there is no limit and no paid tier.",
                14.0,
                p.dim,
            ));
            ui.add_space(26.0);
            ui.horizontal(|ui| {
                let w = ui.available_width();
                ui.add_space((w / 2.0 - 190.0).max(0.0));
                if btn(ui, "  Add a server  ", true, p).clicked() {
                    actions.push(Action::AddServer);
                }
                if btn(ui, "  Preview with a sample library  ", false, p).clicked() {
                    actions.push(Action::UseSample);
                }
            });
            ui.add_space(34.0);

            // What the client does, in three short cards.
            ui.horizontal(|ui| {
                let w = ui.available_width();
                ui.add_space((w / 2.0 - 360.0).max(0.0));
                feature_card(
                    ui,
                    p,
                    "Unlimited servers",
                    "Every server keeps its own sign-in, so work, home and a friend's library all live in the sidebar.",
                );
                feature_card(
                    ui,
                    p,
                    "Continue where you stopped",
                    "Resume points are read from and written back to each server, so the phone and this window agree.",
                );
                feature_card(
                    ui,
                    p,
                    "Search every library",
                    "One box searches the server you are on, including episodes and collections.",
                );
            });
        });
    }

    /// One poster card. The cover says what can be seen at a glance: a tick when
    /// it is watched, a count of episodes still to watch, the rating in the
    /// bottom-right corner and a bar across a film that was left part way.
    /// Returns true when the card itself was clicked.
    fn poster_card(
        &mut self,
        ui: &mut Ui,
        item: &MediaItem,
        w: f32,
        ctx: &egui::Context,
        p: &Palette,
        actions: &mut Vec<Action>,
    ) -> bool {
        let now = ctx.input(|i| i.time);
        let h = w * 1.5;
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h + 48.0), Sense::click());
        if !ui.is_rect_visible(rect) {
            return false;
        }
        let title = display_title(item);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &title));
        let poster = Rect::from_min_size(rect.min, Vec2::new(w, h));
        let tex = self.texture(item, "Primary", 400, now);
        let hovered = resp.hovered();
        draw_poster(ui, poster, tex, p, item, 8.0);
        ui.painter().rect_stroke(
            poster.shrink(0.5),
            7.5,
            Stroke::new(1.0_f32, if hovered || resp.has_focus() { p.text.gamma_multiply(0.75) }
                else if p.dark { Color32::from_white_alpha(23) } else { p.line }),
        );
        if item.progress_fraction() > 0.0 {
            theme::progress_bar(ui, poster, item.progress_fraction(), p, 4.0);
        }
        if item.played {
            let c = Pos2::new(poster.left() + 18.0, poster.top() + 18.0);
            ui.painter()
                .circle_filled(c, 10.0, Color32::from_black_alpha(175));
            theme::icon(ui, "check", c, 12.0, Color32::WHITE);
        }
        // Episodes still to watch, in the top-right corner of the cover.
        if let Some(n) = item.unplayed.filter(|_| !self.prefs.library.hide_unplayed) {
            if n > 0 && item.kind != "Episode" {
                let text = if n > 99 {
                    "99+".to_string()
                } else {
                    n.to_string()
                };
                let cw = 19.0 + 7.0 * text.chars().count() as f32;
                let chip = Rect::from_min_size(
                    Pos2::new(poster.right() - cw - 8.0, poster.top() + 8.0),
                    Vec2::new(cw, 18.0),
                );
                theme::pill(ui, chip, &text, Color32::from_black_alpha(188), Color32::WHITE, 11.0);
            }
        }
        // The rating, in the bottom-right corner of the cover.
        if let Some(r) = item.community_rating.filter(|_| self.prefs.library.show_rating) {
            if r > 0.0 {
                let chip = Rect::from_min_size(
                    Pos2::new(poster.right() - 46.0, poster.bottom() - 27.0),
                    Vec2::new(38.0, 18.0),
                );
                theme::pill(
                    ui,
                    chip,
                    &format!("{:.1}", r),
                    Color32::from_black_alpha(180),
                    Color32::WHITE,
                    11.0,
                );
            }
        }
        if hovered && item.is_playable() {
            theme::play_badge(ui, poster, p);
            if resp.clicked() {
                let on_badge = resp
                    .interact_pointer_pos()
                    .map(|q| q.distance(poster.center()) < 32.0)
                    .unwrap_or(false);
                if on_badge {
                    actions.push(Action::Play(item.clone()));
                    return false;
                }
            }
        }
        let title_size = 13.0_f32;
        // Fit by pixels, not characters, and let long titles end in an ellipsis
        // instead of running across the neighbouring card.
        let max_w = (w - 8.0).max(40.0);
        let title_line = theme::clip_px(ui.painter(), &title, title_size, max_w, p.text);
        ui.painter().text(
            Pos2::new(rect.left(), poster.bottom() + 8.0),
            Align2::LEFT_TOP,
            title_line,
            FontId::proportional(title_size),
            p.text,
        );
        let meta_line = theme::clip_px(ui.painter(), &card_meta(item), 12.0, max_w, p.dim);
        ui.painter().text(
            Pos2::new(rect.left(), poster.bottom() + 27.0),
            Align2::LEFT_TOP,
            meta_line,
            FontId::proportional(12.0),
            p.dim,
        );
        resp.clicked()
    }

    /// One 16:9 still card, for the continue-watching row. The original shows
    /// landscape frames there instead of posters, which is why a wall of tall
    /// covers never looked like it.
    fn strip_card(
        &mut self,
        ui: &mut Ui,
        item: &MediaItem,
        w: f32,
        ctx: &egui::Context,
        p: &Palette,
        actions: &mut Vec<Action>,
    ) -> bool {
        let now = ctx.input(|i| i.time);
        let img_h = w * 9.0 / 16.0;
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(w, img_h + 45.0), Sense::click());
        if !ui.is_rect_visible(rect) {
            return false;
        }
        let title = display_title(item);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &title));
        let img = Rect::from_min_size(rect.min, Vec2::new(w, img_h));
        // Prefer the landscape still; fall back to the poster crop when the
        // server has no backdrop for this item.
        let tex = self
            .texture(item, "Backdrop", 480, now)
            .or_else(|| self.texture(item, "Primary", 480, now));
        draw_poster(ui, img, tex, p, item, 8.0);
        let outline = if resp.has_focus() { p.text.gamma_multiply(0.80) }
            else if p.dark { Color32::from_white_alpha(23) } else { p.line };
        ui.painter().rect_stroke(img.shrink(0.5), 7.5, Stroke::new(1.0_f32, outline));
        if item.progress_fraction() > 0.0 {
            theme::progress_bar(ui, img, item.progress_fraction(), p, 4.0);
        }
        if resp.hovered() && item.is_playable() {
            theme::play_badge(ui, img, p);
            if resp.clicked() && resp.interact_pointer_pos().is_some_and(|pos| pos.distance(img.center()) < 28.0) {
                actions.push(Action::Play(item.clone()));
                return false;
            }
        }
        let max_w = (w - 8.0).max(40.0);
        let title_line = theme::clip_px(ui.painter(), &title, 13.0, max_w, p.text);
        ui.painter().text(
            Pos2::new(img.left(), img.bottom() + 8.0),
            Align2::LEFT_TOP,
            title_line,
            FontId::proportional(13.0),
            p.text,
        );
        let sub = match (item.parent_index, item.index) {
            (Some(s), Some(e)) if item.kind == "Episode" && !item.name.is_empty() => format!("S{s:02}E{e:02}  ·  {}", item.name),
            _ => card_meta(item),
        };
        let sub_line = theme::clip_px(ui.painter(), &sub, 12.0, max_w, p.dim);
        ui.painter().text(
            Pos2::new(img.left(), img.bottom() + 27.0),
            Align2::LEFT_TOP,
            sub_line,
            FontId::proportional(12.0),
            p.dim,
        );
        resp.clicked()
    }

    /// One item as a single wide row, for the list side of the poster/list
    /// switch. Returns true when the row was clicked.
    fn media_row(
        &mut self,
        ui: &mut Ui,
        item: &MediaItem,
        ctx: &egui::Context,
        p: &Palette,
        actions: &mut Vec<Action>,
    ) -> bool {
        let now = ctx.input(|i| i.time);
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 104.0), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(rect, 12.0, p.card);
            ui.painter()
                .rect_stroke(rect, 12.0, Stroke::new(1.0_f32, p.line));
        }
        let thumb = Rect::from_min_size(
            Pos2::new(rect.left() + 8.0, rect.center().y - 43.0),
            Vec2::new(64.0, 86.0),
        );
        let tex = self.texture(item, "Primary", 200, now);
        draw_poster(ui, thumb, tex, p, item, 8.0);
        let x = thumb.right() + 16.0;
        let right = rect.right() - 60.0;
        let row_w = (right - x).max(60.0);
        let row_title = theme::clip_px(
            ui.painter(),
            &display_title(item),
            15.0,
            row_w,
            p.text,
        );
        ui.painter().text(
            Pos2::new(x, rect.top() + 17.0),
            Align2::LEFT_TOP,
            row_title,
            FontId::proportional(15.0),
            p.text,
        );
        ui.painter().text(
            Pos2::new(x, rect.top() + 40.0),
            Align2::LEFT_TOP,
            theme::clip_px(ui.painter(), &card_meta(item), 12.0, row_w, p.dim),
            FontId::proportional(12.0),
            p.dim,
        );
        if !item.overview.is_empty() {
            let line = theme::clip_px(
                ui.painter(),
                &item.overview,
                11.5,
                row_w,
                p.dim.gamma_multiply(0.85),
            );
            ui.painter().text(
                Pos2::new(x, rect.top() + 60.0),
                Align2::LEFT_TOP,
                line,
                FontId::proportional(11.5),
                p.dim.gamma_multiply(0.85),
            );
        }
        if let Some(r) = item.community_rating {
            if r > 0.0 {
                ui.painter().text(
                    Pos2::new(rect.right() - 16.0, rect.top() + 18.0),
                    Align2::RIGHT_TOP,
                    format!("{:.1}", r),
                    FontId::proportional(12.5),
                    p.warn,
                );
            }
        }
        if item.progress_fraction() > 0.0 {
            let bar = Rect::from_min_size(
                Pos2::new(thumb.left(), thumb.bottom() + 4.0),
                Vec2::new(thumb.width(), 3.0),
            );
            ui.painter().rect_filled(bar, 1.5, p.line);
            let done = Rect::from_min_size(
                bar.min,
                Vec2::new(bar.width() * item.progress_fraction(), 3.0),
            );
            ui.painter().rect_filled(done, 1.5, p.accent);
        }
        if resp.hovered() {
            let c = Pos2::new(rect.right() - 30.0, rect.center().y);
            ui.painter().circle_filled(c, 14.0, p.accent);
            theme::icon(ui, "play", c + Vec2::new(1.0, 0.0), 12.0, Color32::WHITE);
            if resp.clicked() && item.is_playable() {
                if let Some(q) = resp.interact_pointer_pos() {
                    if q.distance(c) < 18.0 {
                        actions.push(Action::Play(item.clone()));
                        return false;
                    }
                }
            }
        }
        resp.clicked()
    }

    /// The wall's own controls: posters or rows, and how wide a poster is.
    fn wall_controls(&mut self, ui: &mut Ui, p: &Palette, actions: &mut Vec<Action>) {
        let label = if self.list_view {
            "Switch to Grid"
        } else {
            "Switch to List"
        };
        if btn(ui, &theme::tl(label), false, p).clicked() {
            actions.push(Action::ToggleListView);
        }
        ui.add_space(8.0);
        for (scale, name) in [
            (CardScale::Small, "Small"),
            (CardScale::Medium, "Medium"),
            (CardScale::Large, "Large"),
        ] {
            if btn(ui, &theme::tl(name), self.card_scale == scale, p).clicked() {
                actions.push(Action::SetCardScale(scale));
            }
        }
    }

    /// Draw a set of items as a wall of posters, or as rows once the list has
    /// been chosen. The columns follow the window, so the wall fills the page at
    /// any width and any poster size.
    fn draw_wall(
        &mut self,
        ui: &mut Ui,
        items: &[MediaItem],
        ctx: &egui::Context,
        p: &Palette,
        actions: &mut Vec<Action>,
    ) {
        // 64 px content inset and a 16 px gutter, both measured on the original.
        let inset = 64.0;
        if self.list_view {
            for item in items {
                ui.horizontal(|ui| {
                    ui.add_space(inset);
                    let hit = self.media_row(ui, item, ctx, p, actions);
                    if hit {
                        actions.push(Action::Open(item.id.clone()));
                    }
                });
                ui.add_space(8.0);
            }
            ui.add_space(12.0);
            return;
        }
        let w = self.card_scale.width() * self.prefs.library.cover_scale;
        let gutter = 16.0;
        let usable = (ui.available_width() - inset * 2.0).max(w);
        let per_row = (((usable + gutter) / (w + gutter)).floor() as usize).max(1);
        for chunk in items.chunks(per_row) {
            ui.horizontal(|ui| {
                ui.add_space(inset);
                ui.spacing_mut().item_spacing.x = gutter;
                for item in chunk {
                    if self.poster_card(ui, item, w, ctx, p, actions) {
                        actions.push(Action::Open(item.id.clone()));
                    }
                }
            });
            ui.add_space(22.0);
        }
        ui.add_space(8.0);
    }

    // -------------------------------------------------------------- library

    fn draw_grid_view(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        p: &Palette,
        actions: &mut Vec<Action>,
    ) {
        ui.add_space(72.0);
        ui.horizontal(|ui| {
            ui.add_space(64.0);
            ui.label(theme::heading(&self.grid.title, 22.0, p.text));
            ui.add_space(8.0);
            if self.grid.total > 0 {
                ui.label(theme::body(
                    &i18n::counted(self.grid.total, "item", "items"),
                    12.0,
                    p.dim,
                ));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if btn(ui, &theme::tl("Refresh"), false, p).clicked() {
                    actions.push(Action::Refresh);
                }
                ui.add_space(10.0);
                for (by, label) in [
                    ("CommunityRating", "Rating"),
                    ("DateCreated", "Newest"),
                    ("SortName", "Name"),
                ] {
                    let on = self.grid.sort == by;
                    if btn(ui, &theme::tl(label), on, p).clicked() {
                        actions.push(Action::SetSort(by.to_string()));
                    }
                }
                ui.label(theme::body("Sort", 11.5, p.dim));
            });
        });
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(12.0);

        if self.grid.items.is_empty() {
            if self.loading {
                ui.spinner();
            } else {
                ui.label(theme::body("No items in this library yet.", 13.0, p.dim));
            }
            return;
        }

        let items = self.grid.items.clone();
        let shown: Vec<MediaItem> = match self.agg_tab {
            0 => items
                .iter()
                .filter(|i| i.progress_ticks > 0 || i.progress_fraction() > 0.0)
                .cloned()
                .collect(),
            1 => items.iter().filter(|i| i.favorite).cloned().collect(),
            _ => items.clone(),
        };
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if shown.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(64.0);
                        ui.label(theme::body(&theme::tl("Nothing here yet"), 13.0, p.dim));
                    });
                } else {
                    self.draw_wall(ui, &shown, ctx, p, actions);
                }
            });
    }

    // ------------------------------------------------------------ aggregate

    /// Federated items are displayed under their source server.
    fn draw_aggregate(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        p: &Palette,
        actions: &mut Vec<Action>,
    ) {
        ui.add_space(72.0);
        let mut groups = self.agg_items.clone();
        if groups.is_empty() {
            let mut seen: HashSet<String> = HashSet::new();
            let mut items = Vec::new();
            for row in &self.rows {
                for it in &row.items {
                    if seen.insert(it.id.clone()) {
                        items.push(it.clone());
                    }
                }
            }
            if let Some(idx) = self.active {
                groups.push((idx, items));
            }
        }
        let total: usize = groups.iter().map(|(_, items)| items.len()).sum();
        // The original splits 聚合视界 into continue-watching / favourites / all
        // libraries; the chips below select which slice is shown.
        let mut chosen: Option<usize> = None;
        ui.horizontal(|ui| {
            ui.add_space(64.0);
            for (i, label) in ["Continue Watching", "Favourites", "Libraries"].iter().enumerate() {
                let active = self.agg_tab == i;
                let text = theme::tl(label);
                let w = 34.0 + 7.0 * text.chars().count() as f32;
                let (r, resp) =
                    ui.allocate_exact_size(Vec2::new(w, 34.0), Sense::click());
                ui.painter().rect_filled(
                    r,
                    17.0,
                    if active {
                        Color32::WHITE
                    } else if resp.hovered() {
                        Color32::from_rgba_premultiplied(0x2E, 0x33, 0x39, 200)
                    } else {
                        Color32::from_rgba_premultiplied(0x16, 0x1C, 0x23, 170)
                    },
                );
                ui.painter().text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    &text,
                    FontId::proportional(13.0),
                    if active { Color32::from_rgb(0x11, 0x14, 0x18) } else { p.dim },
                );
                if resp.clicked() {
                    chosen = Some(i);
                }
                ui.add_space(8.0);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(64.0);
                ui.label(theme::body(
                    &i18n::counted(total as i64, "item", "items"),
                    12.0,
                    p.dim,
                ));
            });
        });
        if let Some(i) = chosen {
            self.agg_tab = i;
        }
        ui.add_space(12.0);
        ui.add_space(14.0);
        if total == 0 {
            ui.horizontal(|ui| {
                ui.add_space(64.0);
                if self.loading {
                    ui.spinner();
                } else {
                    ui.label(theme::body(
                        &theme::tl("Nothing to aggregate yet."),
                        13.0,
                        p.dim,
                    ));
                }
            });
            return;
        }
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (idx, items) in &groups {
                    let Some(server) = self.servers.get(*idx) else { continue };
                    let name = server.name.clone();
                    let shown: Vec<_> = match self.agg_tab {
                        0 => items.iter().filter(|item| item.progress_ticks > 0 && !item.played).cloned().collect(),
                        1 => items.iter().filter(|item| item.favorite).cloned().collect(),
                        _ => items.clone(),
                    };
                    if shown.is_empty() { continue; }
                    ui.horizontal(|ui| {
                        ui.add_space(64.0);
                        ui.label(theme::heading(&name, 18.0, p.text));
                        ui.label(theme::body(&i18n::counted(shown.len() as i64, "item", "items"), 12.0, p.dim));
                    });
                    ui.add_space(16.0);
                    let before = actions.len();
                    self.draw_wall(ui, &shown, ctx, p, actions);
                    for action in &mut actions[before..] {
                        if let Action::Open(id) = action {
                            *action = Action::OpenOnServer(*idx, id.clone());
                        }
                    }
                    ui.add_space(24.0);
                }
            });
    }

    // ----------------------------------------------------------- favourites

    fn draw_favorites(&mut self, ui: &mut Ui, ctx: &egui::Context, p: &Palette, actions: &mut Vec<Action>) {
        ui.add_space(78.0);
        if self.favorites.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(64.0);
                if self.loading {
                    ui.spinner();
                } else {
                    ui.label(theme::body(&theme::tl("Nothing here yet"), 13.0, p.dim));
                }
            });
            return;
        }
        let items = self.favorites.clone();
        let mut films: Vec<MediaItem> = Vec::new();
        let mut shows: Vec<MediaItem> = Vec::new();
        for it in &items {
            if it.kind == "Movie" {
                films.push(it.clone());
            } else {
                shows.push(it.clone());
            }
        }
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // The original puts each group under an 18 px heading with a
                // chevron, 36 px before the next one.
                for (title, group) in [
                    ("Favourite films", films),
                    ("Favourite series", shows),
                ] {
                    if group.is_empty() {
                        continue;
                    }
                    ui.horizontal(|ui| {
                        ui.add_space(64.0);
                        ui.label(theme::heading(&theme::tl(title), 18.0, p.text));
                        ui.add_space(6.0);
                        ui.label(theme::body("\u{203a}", 16.0, p.dim));
                    });
                    ui.add_space(12.0);
                    self.draw_wall(ui, &group, ctx, p, actions);
                    ui.add_space(26.0);
                }
            });
    }

    // --------------------------------------------------------------- search

    fn draw_search(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        p: &Palette,
        actions: &mut Vec<Action>,
    ) {
        ui.add_space(72.0);
        ui.horizontal(|ui| {
            ui.add_space(64.0);
            ui.label(theme::heading(&theme::tl("Search"), 22.0, p.text));
        });
        ui.add_space(10.0);
        let mut q = self.search_query.clone();
        ui.horizontal(|ui| {
            let resp = ui.add_sized(
                [ui.available_width() - 110.0, 36.0],
                egui::TextEdit::singleline(&mut q)
                    .hint_text("Movies, series, episodes\u{2026}")
                    .margin(Vec2::new(10.0, 7.0)),
            );
            if resp.changed() {
                self.search_query = q.clone();
            }
            if ui.input(|i| i.key_pressed(egui::Key::Enter))
                || btn(ui, "Search", true, p).clicked()
            {
                actions.push(Action::DoSearch(q.clone()));
            }
        });
        ui.add_space(12.0);

        if self.prefs.library.search_history && self.search_query.is_empty() && !self.search_history.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(i18n::bilingual("Search history", "搜索历史"));
                for term in &self.search_history {
                    if ui.button(term).clicked() {
                        actions.push(Action::DoSearch(term.clone()));
                    }
                }
                if ui.small_button(i18n::bilingual("Clear", "清空")).clicked() {
                    actions.push(Action::Settings(settings_ui::Command::ClearSearchHistory));
                }
            });
            ui.add_space(10.0);
        }

        if self.search_results.is_empty() {
            ui.label(theme::body(
                if self.loading {
                    "Searching\u{2026}"
                } else if self.search_term.is_empty() {
                    "Type a name and press Enter."
                } else {
                    "Nothing matched that name."
                },
                13.0,
                p.dim,
            ));
            return;
        }

        ui.horizontal(|ui| {
            ui.label(theme::body(
                &i18n::counted(self.search_results.len() as i64, "result", "results"),
                12.0,
                p.dim,
            ));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                self.wall_controls(ui, p, actions);
            });
        });
        ui.add_space(10.0);
        let items = self.search_results.clone();
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.draw_wall(ui, &items, ctx, p, actions);
            });
    }

    fn draw_detail(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        p: &Palette,
        actions: &mut Vec<Action>,
    ) {
        let dark_palette = theme::palette(true);
        let p = if p.dark {
            p
        } else {
            *ui.visuals_mut() = egui::Visuals::dark();
            &dark_palette
        };
        let viewport = ui.max_rect();
        let early_paint = ui.painter().add(egui::Shape::Noop);
        let back = Rect::from_min_size(viewport.min + Vec2::new(24.0, 12.0), Vec2::splat(36.0));
        let window_controls = Rect::from_min_size(
            Pos2::new(viewport.right() - 162.0, viewport.top() + 12.0),
            Vec2::new(138.0, 36.0),
        );
        let title_drag = Rect::from_min_max(
            Pos2::new(back.right() + 8.0, viewport.top()),
            Pos2::new(window_controls.left() - 8.0, viewport.top() + 54.0),
        );
        let titlebar = ui.interact(title_drag, ui.id().with("detail-titlebar"), Sense::click_and_drag());
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        if titlebar.double_clicked() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
        } else if titlebar.drag_started() {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        let chrome_fill = Color32::from_rgba_unmultiplied(39, 44, 42, 195);
        ui.painter().rect_filled(window_controls, 18.0, chrome_fill);
        for (index, (icon, label, command)) in [
            ("minimize", "Minimize", egui::ViewportCommand::Minimized(true)),
            (if maximized { "restore" } else { "maximize" }, if maximized { "Restore window" } else { "Maximize" }, egui::ViewportCommand::Maximized(!maximized)),
            ("close", "Close", egui::ViewportCommand::Close),
        ].into_iter().enumerate() {
            let rect = Rect::from_min_size(window_controls.min + Vec2::new(index as f32 * 46.0, 0.0), Vec2::new(46.0, 36.0));
            if detail_icon_button(ui, rect, icon, label, false, Color32::TRANSPARENT).clicked() {
                ctx.send_viewport_cmd(command);
            }
        }
        if detail_icon_button(ui, back, "back-arrow", "Back", false, chrome_fill).clicked() {
            actions.push(Action::BackFromDetail);
        }
        let Some(detail) = self.detail.take() else {
            let content = Rect::from_center_size(viewport.center(), Vec2::new(300.0, 48.0));
            ui.allocate_ui_at_rect(content, |ui| {
                ui.vertical_centered(|ui| {
                    if self.loading {
                        ui.spinner();
                    } else {
                        ui.label(theme::body("That item could not be loaded.", 14.0, p.dim));
                    }
                });
            });
            return;
        };
        let item = &detail.item;
        let now = ctx.input(|i| i.time);
        let backdrop = self.texture(item, "Backdrop", 1600, now);
        let logo = detail.logo.as_ref().and_then(|(owner, tag)| self.artwork(owner, "Logo", tag, 640, now));
        let color = Color32::from_rgb(45, 47, 46);
        let width = viewport.width();
        let inset = (width * 0.044).clamp(32.0, 56.0);
        let left_width = (width * 0.22).clamp(200.0, 280.0);
        let column_gap = 36.0;
        let right_width = (width - 2.0 * inset - left_width - column_gap).max(240.0);
        let hero_height = (viewport.height() * 0.66).clamp(465.0, 544.0);
        let episode = detail.episodes.iter().find(|ep| ep.id == self.detail_media.item_id)
            .or_else(|| detail.episodes.first());
        let playable = episode.or_else(|| item.is_playable().then_some(item));
        let active_season = detail.seasons.iter().find(|season| Some(&season.id) == self.detail_season.as_ref());
        ScrollArea::vertical()
            .id_source(("new-details-scroll", &item.id))
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                // Keep the image behind the fixed detail header. Only the
                // foreground information and episode rows scroll.
                let (hero, _) = ui.allocate_exact_size(Vec2::new(width, hero_height), Sense::hover());
                if let Some(texture) = backdrop {
                    let artwork = Rect::from_min_size(viewport.min, Vec2::new(width, hero_height.max(width * 9.0 / 16.0)));
                    let size = ctx.tex_manager().read().meta(texture).map(|meta| meta.size);
                    let uv = size.map_or(Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), |size| {
                        cover_uv(artwork.size(), Vec2::new(size[0] as f32, size[1] as f32))
                    });
                    let mut shade = egui::Mesh::default();
                    for (y, alpha) in [(0.0, 38), (0.26, 12), (0.51, 28), (0.72, 109), (0.88, 213), (1.0, 255)] {
                        let y = viewport.top() + hero_height * y;
                        let c = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha);
                        let start = shade.vertices.len() as u32;
                        shade.colored_vertex(Pos2::new(viewport.left(), y), c);
                        shade.colored_vertex(Pos2::new(viewport.right(), y), c);
                        if start > 0 {
                            shade.add_triangle(start - 2, start - 1, start);
                            shade.add_triangle(start - 1, start + 1, start);
                        }
                    }
                    let hero_clip = Rect::from_min_size(viewport.min, Vec2::new(width, hero_height));
                    let mut image = egui::Mesh::with_texture(texture);
                    let visible_uv = Rect::from_min_max(
                        uv.min,
                        Pos2::new(uv.max.x, uv.min.y + uv.height() * (hero_height / artwork.height())),
                    );
                    image.add_rect_with_uv(hero_clip, visible_uv, Color32::WHITE);
                    ui.painter().set(early_paint, egui::Shape::Vec(vec![
                        egui::Shape::mesh(image),
                        egui::Shape::mesh(shade),
                    ]));
                }

                let left = hero.left() + inset;
                let right = left + left_width + column_gap;
                let play_top = hero.bottom() - 38.0 - 40.0;
                let action_top = play_top - 46.0;
                let logo_rect = Rect::from_min_size(Pos2::new(left, action_top - 96.0), Vec2::new(left_width, 76.0));
                if let Some(logo) = logo {
                    if let Some(size) = ctx.tex_manager().read().meta(logo).map(|meta| meta.size) {
                        let image = Vec2::new(size[0] as f32, size[1] as f32);
                        let scale = (logo_rect.width() / image.x).min(logo_rect.height() / image.y);
                        ui.painter().image(
                            logo,
                            Rect::from_min_size(Pos2::new(logo_rect.left(), logo_rect.bottom() - image.y * scale), image * scale),
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                } else {
                    let title = ctx.fonts(|fonts| fonts.layout(
                        item.name.clone(),
                        FontId::proportional(34.0),
                        p.text,
                        logo_rect.width(),
                    ));
                    let title_pos = Pos2::new(logo_rect.left(), (logo_rect.bottom() - title.size().y).max(logo_rect.top()));
                    ui.painter().with_clip_rect(logo_rect.intersect(ui.clip_rect())).galley(title_pos, title, p.text);
                }

                let action_width = (left_width - 16.0) / 3.0;
                for (i, (icon, label, active)) in [
                    ("check", if item.played { "Mark unwatched" } else { "Mark watched" }, item.played),
                    (if item.favorite { "heart-filled" } else { "heart" }, if item.favorite { "In favourites" } else { "Add to favourites" }, item.favorite),
                    ("more", "More", false),
                ].into_iter().enumerate() {
                    let rect = Rect::from_min_size(
                        Pos2::new(left + i as f32 * (action_width + 8.0), action_top),
                        Vec2::new(action_width, 36.0),
                    );
                    let button = detail_icon_button(ui, rect, icon, label, active, Color32::from_rgb(43, 46, 43));
                    if button.clicked() {
                        match i {
                            0 => actions.push(Action::TogglePlayed(item.clone())),
                            1 => actions.push(Action::ToggleFavorite(item.clone())),
                            _ => ui.memory_mut(|memory| memory.toggle_popup(ui.id().with("new-details-more"))),
                        }
                    }
                    if i == 2 {
                        egui::popup::popup_below_widget(
                            ui, ui.id().with("new-details-more"), &button,
                            egui::popup::PopupCloseBehavior::CloseOnClickOutside,
                            |ui| {
                                if ui.button(i18n::t("Refresh details")).clicked() {
                                    actions.push(Action::Refresh);
                                    ui.memory_mut(|memory| memory.close_popup());
                                }
                                if let Some(media) = playable {
                                    if media.progress_ticks > 0 && ui.button(i18n::t("Play from start")).clicked() {
                                        let mut media = media.clone();
                                        media.progress_ticks = 0;
                                        actions.push(Action::PlaySelected(media));
                                        ui.memory_mut(|memory| memory.close_popup());
                                    }
                                    for (enabled, choice, label) in [
                                        (self.prefs.external.mpv, PlayerChoice::Mpv, ("Play in mpv", "用外部 mpv 播放")),
                                        (self.prefs.external.potplayer, PlayerChoice::PotPlayer, ("Play in PotPlayer", "用 PotPlayer 播放")),
                                    ] {
                                        if enabled && ui.button(i18n::bilingual(label.0, label.1)).clicked() {
                                            actions.push(Action::PlayExternal(media.clone(), choice));
                                            ui.memory_mut(|memory| memory.close_popup());
                                        }
                                    }
                                }
                            },
                        );
                    }
                }

                let play_rect = Rect::from_min_size(Pos2::new(left, play_top), Vec2::new(left_width, 40.0));
                let play_label = i18n::t(if playable.is_some_and(|media| media.progress_ticks > 0 && !media.played) { "Resume" } else { "Play" });
                let play = ui.interact(play_rect, ui.id().with("new-details-play"), if playable.is_some() { Sense::click() } else { Sense::hover() });
                let play_text = Color32::from_rgb(13, 19, 24);
                let play_fill = if playable.is_none() { Color32::from_rgb(145, 152, 152) }
                    else if play.hovered() { Color32::WHITE } else { Color32::from_rgb(241, 243, 246) };
                ui.painter().rect_filled(play_rect, 20.0, play_fill);
                if play.has_focus() {
                    ui.painter().rect_stroke(play_rect, 20.0, Stroke::new(2.0_f32, p.accent));
                }
                let play_galley = ctx.fonts(|fonts| fonts.layout_no_wrap(play_label.into_owned(), FontId::proportional(14.0), play_text));
                let group_left = play_rect.center().x - (play_galley.size().x + 21.0) / 2.0;
                theme::icon(ui, "play", Pos2::new(group_left + 7.0, play_rect.center().y), 13.0, play_text);
                let text_y = play_rect.center().y - play_galley.size().y / 2.0;
                ui.painter().galley(Pos2::new(group_left + 21.0, text_y), play_galley, play_text);
                if let Some(media) = playable.filter(|_| play.clicked()) {
                    actions.push(Action::PlaySelected(media.clone()));
                }
                if let Some((message, true)) = &self.status {
                    let text = theme::clip_px(ui.painter(), message, 13.0, width - 2.0 * inset, Color32::LIGHT_RED);
                    let rect = ui.painter().text(play_rect.left_bottom() + Vec2::new(0.0, 6.0), Align2::LEFT_TOP,
                        text, FontId::proportional(13.0), Color32::LIGHT_RED);
                    ui.interact(rect, ui.id().with("detail-status"), Sense::hover()).on_hover_text(message);
                }

                let mut metadata = Vec::new();
                if let Some(year) = item.year {
                    metadata.push(if detail.continuing {
                        format!("{year}–{}", i18n::t("Present"))
                    } else if let Some(end) = detail.end_year.filter(|end| *end > year) {
                        format!("{year}–{end}")
                    } else {
                        year.to_string()
                    });
                }
                if !detail.seasons.is_empty() {
                    metadata.push(if i18n::is_zh() { format!("共{}季", detail.seasons.len()) } else { i18n::counted(detail.seasons.len() as i64, "season", "seasons") });
                }
                if let Some(runtime) = playable.and_then(MediaItem::runtime_secs).or_else(|| item.runtime_secs()) {
                    let runtime = runtime as i64;
                    let (minutes, seconds) = (runtime / 60, runtime % 60);
                    metadata.push(if i18n::is_zh() { format!("{minutes}分{seconds}秒") } else { format!("{minutes}m {seconds}s") });
                }
                metadata.extend(item.genres.iter().take(4).cloned());
                let overview_top = play_top + 1.0;
                let info_top = action_top - 71.0;
                let sources = &self.detail_media.sources;
                let selected = &mut self.detail_media.selection;
                let show_sources = playable.is_some();
                {
                    let mut info = ui.child_ui_with_id_source(
                        Rect::from_min_size(Pos2::new(right, info_top), Vec2::new(right_width, 101.0)),
                        Layout::top_down(Align::Min),
                        "detail-meta",
                        None,
                    );
                    let ui = &mut info;
                    ui.set_width(right_width);
                    ui.spacing_mut().item_spacing = Vec2::new(12.0, 3.0);
                    ui.add(egui::Label::new(theme::body(&metadata.join("  "), 13.0, p.text)).truncate());
                    let source = selected.source_id.as_ref()
                        .and_then(|id| sources.iter().find(|source| source.id.as_ref() == Some(id)))
                        .or_else(|| sources.first());
                    let mut format = Vec::new();
                    if let Some(source) = source {
                        if let Some(size) = source.size.filter(|size| *size > 0) {
                            format.push(if size >= 1_000_000_000 {
                                format!("{:.1} GB", size as f64 / 1e9)
                            } else {
                                format!("{:.0} MB", size as f64 / 1e6)
                            });
                        }
                        if let Some(bitrate) = source.bitrate.filter(|rate| *rate > 0) {
                            format.push(format!("{:.1} Mbps", bitrate as f64 / 1e6));
                        }
                        if let Some(video) = source.media_streams.iter().find(|stream| stream.kind == "Video") {
                            let height = video.height.map(|h| format!("{h}p")).unwrap_or_default();
                            let codec = video.codec.as_deref().unwrap_or("").to_ascii_uppercase();
                            let description = format!("{height} {codec}");
                            if !description.trim().is_empty() {
                                format.push(description.trim().to_string());
                            }
                        }
                        if let Some(container) = &source.container {
                            format.push(container.to_ascii_uppercase());
                        }
                    }
                    let format = if format.is_empty() && self.detail_media.loading {
                        i18n::t("Loading media options…").into_owned()
                    } else {
                        format.join(" · ")
                    };
                    if format.is_empty() {
                        ui.add_space(16.0);
                    } else {
                        ui.add(egui::Label::new(theme::body(&format, 11.5, p.text)).truncate());
                    }
                    if show_sources {
                        ui.add_space(7.0);
                        ui.horizontal(|ui| {
                            let fields_width = right_width.min(772.0) - 24.0;
                            let version_width = fields_width * 0.354;
                            let track_width = (fields_width - version_width) * 0.5;
                            ui.spacing_mut().button_padding = Vec2::new(12.0, 8.0);
                            ui.spacing_mut().interact_size.y = 36.0;
                            let visuals = ui.visuals_mut();
                            for (widget, shade) in [
                                (&mut visuals.widgets.inactive, 43),
                                (&mut visuals.widgets.hovered, 56),
                                (&mut visuals.widgets.active, 61),
                                (&mut visuals.widgets.open, 56),
                            ] {
                                let fill = Color32::from_rgb(shade, shade + 3, shade + 2);
                                widget.rounding = egui::Rounding::same(18.0);
                                widget.bg_fill = fill;
                                widget.weak_bg_fill = fill;
                                widget.bg_stroke = Stroke::NONE;
                            }
                            let previous = selected.source_id.clone();
                            ui.vertical(|ui| {
                                ui.label(theme::body(&i18n::t("Version"), 12.0, p.text));
                                ui.add_enabled_ui(!sources.is_empty(), |ui| {
                                    egui::ComboBox::from_id_source("new-details-other-sources")
                                        .width(version_width).height(264.0).truncate()
                                        .selected_text(source.map_or_else(|| i18n::t("Unavailable"), |source| source.version_label().into()))
                                        .show_ui(ui, |ui| {
                                            for source in sources {
                                                ui.selectable_value(&mut selected.source_id, source.id.clone(), source.version_label());
                                            }
                                        });
                                });
                            });
                            if previous != selected.source_id {
                                selected.audio_index = None;
                                selected.subtitle_index = None;
                            }
                            let source = selected.source_id.as_ref()
                                .and_then(|id| sources.iter().find(|source| source.id.as_ref() == Some(id)))
                                .or_else(|| sources.first());
                            for (kind, label, choice, language, server_default) in [
                                ("Audio", "Audio", &mut selected.audio_index, &self.prefs.playback.audio_language,
                                    source.and_then(|source| source.default_audio_stream_index)),
                                ("Subtitle", "Subtitle track", &mut selected.subtitle_index, &self.prefs.playback.subtitle_language,
                                    source.and_then(|source| source.default_subtitle_stream_index)),
                            ] {
                                let streams = source.map(|source| source.media_streams.as_slice()).unwrap_or(&[]);
                                let selected_stream = choice.or_else(|| source?.language_index(kind, language)).or(server_default).or_else(|| {
                                    streams.iter().find(|stream| stream.kind == kind && stream.is_default == Some(true)).and_then(|stream| stream.index)
                                }).or_else(|| {
                                    (kind == "Audio").then(|| streams.iter().find(|stream| stream.kind == kind).and_then(|stream| stream.index)).flatten()
                                });
                                let name = selected_stream.and_then(|index| streams.iter().find(|stream| stream.kind == kind && stream.index == Some(index)))
                                    .map(|stream| stream.label());
                                ui.vertical(|ui| {
                                    ui.label(theme::body(&i18n::t(label), 12.0, p.text));
                                    let available = streams.iter().any(|stream| stream.kind == kind && stream.index.is_some());
                                    ui.add_enabled_ui(available, |ui| {
                                        let name = name.map_or_else(|| if !available { i18n::t("Unavailable") }
                                            else if selected_stream == Some(-1) { i18n::t("Turn off subtitles") } else { i18n::t("Automatic") }, Into::into);
                                        egui::ComboBox::from_id_source(("detail-stream", kind))
                                            .width(track_width).height(264.0).truncate().selected_text(name)
                                            .show_ui(ui, |ui| {
                                                ui.selectable_value(choice, None, i18n::t("Automatic"));
                                                if kind == "Subtitle" {
                                                    ui.selectable_value(choice, Some(-1), i18n::t("Turn off subtitles"));
                                                }
                                                for stream in streams.iter().filter(|stream| stream.kind == kind && stream.index.is_some()) {
                                                    ui.selectable_value(choice, stream.index, stream.label());
                                                }
                                            });
                                    });
                                });
                            }
                        });
                    }
                }
                if !item.overview.is_empty() {
                    let area = Rect::from_min_size(Pos2::new(right, overview_top), Vec2::new(right_width, 49.0));
                    let mut job = egui::text::LayoutJob::simple(item.overview.clone(), FontId::proportional(13.5), p.text, right_width);
                    job.wrap.max_rows = 2;
                    let overview = ctx.fonts(|fonts| fonts.layout_job(job));
                    ui.painter().galley(area.min, overview, p.text);
                    ui.interact(area, ui.id().with("new-details-overview"), Sense::hover()).on_hover_text(&item.overview);
                }

                if matches!(item.kind.as_str(), "Series" | "Season") {
                    let (heading, _) = ui.allocate_exact_size(Vec2::new(width, 42.0), Sense::hover());
                    let title_rect = Rect::from_min_size(
                        Pos2::new(heading.left() + inset, heading.top()),
                        Vec2::new(250.0, 38.0),
                    );
                    let title = active_season.map(|season| season.name.as_str()).unwrap_or(if item.kind == "Season" { &item.name } else { "Episodes" });
                    ui.painter().text(title_rect.left_center(), Align2::LEFT_CENTER, i18n::t(title), FontId::proportional(20.0), p.text);
                    if detail.seasons.len() > 1 {
                        let season = ui.interact(title_rect, ui.id().with("new-details-season"), Sense::click());
                        if season.clicked() {
                            ui.memory_mut(|memory| memory.toggle_popup(season.id));
                        }
                        egui::popup::popup_below_widget(
                            ui, season.id, &season, egui::popup::PopupCloseBehavior::CloseOnClickOutside,
                            |ui| {
                                ScrollArea::vertical().max_height(264.0).show(ui, |ui| {
                                    for option in &detail.seasons {
                                        if ui.selectable_label(Some(&option.id) == self.detail_season.as_ref(), &option.name).clicked() {
                                            actions.push(Action::Season(option.id.clone()));
                                            ui.memory_mut(|memory| memory.close_popup());
                                        }
                                    }
                                });
                            },
                        );
                    }
                    let jump = Rect::from_min_size(Pos2::new(heading.right() - 40.0 - 36.0, heading.top()), Vec2::splat(36.0));
                    let layout = jump.translate(Vec2::new(-48.0, 0.0));
                    let count = if i18n::is_zh() { format!("共{}集", detail.episodes.len()) }
                        else { i18n::counted(detail.episodes.len() as i64, "episode", "episodes") };
                    ui.painter().text(Pos2::new(layout.left() - 12.0, layout.center().y), Align2::RIGHT_CENTER, count, FontId::proportional(12.0), p.text);
                    if detail_icon_button(ui, layout, if self.episode_grid { "list" } else { "grid" }, "Episode layout", false, p.card).clicked() {
                        actions.push(Action::ToggleEpisodeGrid);
                    }
                    if detail_icon_button(ui, jump, "jump", "Go to episode", self.detail_jump, p.card).clicked() {
                        self.detail_jump = !self.detail_jump;
                    }

                    let selected_episode = episode.map(|ep| ep.id.as_str());
                    let reveal_episode = std::mem::take(&mut self.detail_reveal_episode);
                    let episode_grid = self.episode_grid;
                    let rail_id = ("detail-episode-rail", item.id.as_str(), self.detail_season.clone());
                    let mut draw_episode = |ui: &mut Ui, ep: &MediaItem| {
                        ui.push_id(&ep.id, |ui| {
                            let selected = Some(ep.id.as_str()) == selected_episode;
                            let (card, response) = ui.allocate_exact_size(Vec2::new(216.0, 151.0), Sense::click());
                            if selected && episode_grid && reveal_episode {
                                response.scroll_to_me(Some(Align::Center));
                            }
                            let thumbnail = Rect::from_min_size(card.min, Vec2::new(216.0, 121.5));
                            if ui.is_rect_visible(thumbnail) {
                                let tex = self.texture(ep, "Primary", 480, now);
                                draw_image(ui, thumbnail, tex, p, ep, 9.0);
                                theme::progress_bar(ui, thumbnail, ep.progress_fraction(), p, 3.0);
                                if selected || response.hovered() || response.has_focus() {
                                    ui.painter().rect_stroke(thumbnail.shrink(1.0), 9.0, Stroke::new(2.0_f32, Color32::WHITE));
                                }
                                let name = match ep.index {
                                    Some(index) => format!("{index}. {}", ep.name),
                                    None => ep.name.clone(),
                                };
                                let mut text = egui::text::LayoutJob::simple(name, FontId::proportional(13.0), p.text, card.width());
                                text.wrap.max_rows = 1;
                                let title = ctx.fonts(|fonts| fonts.layout_job(text));
                                ui.painter().galley(Pos2::new(card.left(), thumbnail.bottom() + 7.0), title, p.text);
                            }
                            if response.double_clicked() {
                                actions.push(Action::PlaySelected(ep.clone()));
                            } else if response.clicked() && !selected {
                                actions.push(Action::DetailFocus(ep.clone()));
                            }
                        });
                    };
                    if detail.episodes.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(inset);
                            if self.loading {
                                ui.spinner();
                            } else {
                                let message = self.status.as_ref().filter(|(_, error)| *error).map(|(message, _)| message.as_str())
                                    .unwrap_or("No episodes found");
                                ui.label(theme::body(&i18n::t(message), 13.0, p.dim));
                            }
                        });
                    } else if episode_grid {
                        let columns = ((width - 2.0 * inset + 16.0) / 232.0).floor().max(1.0) as usize;
                        for chunk in detail.episodes.chunks(columns) {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                ui.add_space(inset);
                                for ep in chunk {
                                    draw_episode(ui, ep);
                                    ui.add_space(16.0);
                                }
                            });
                        }
                    } else {
                        let mut rail = ScrollArea::horizontal()
                            .id_source(rail_id)
                            .auto_shrink([false, true])
                            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
                        if reveal_episode {
                            if let Some(index) = detail.episodes.iter().position(|ep| Some(ep.id.as_str()) == selected_episode) {
                                let offset = inset + index as f32 * 232.0 - (width - 216.0) / 2.0;
                                rail = rail.horizontal_scroll_offset(offset.max(0.0));
                            }
                        }
                        rail.show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 0.0;
                                    ui.add_space(inset);
                                    for ep in &detail.episodes {
                                        draw_episode(ui, ep);
                                        ui.add_space(16.0);
                                    }
                                    ui.add_space(inset - 16.0);
                                });
                            });
                    }
                    ui.add_space(49.0);
                }

                let chapter_mode = self.prefs.library.chapters;
                if let Some(media) = playable.filter(|media| {
                    !media.chapters.is_empty() && match chapter_mode {
                        crate::preferences::ChapterMode::Always => true,
                        crate::preferences::ChapterMode::Movies => media.kind == "Movie",
                        crate::preferences::ChapterMode::Never => false,
                    }
                }) {
                    ui.horizontal(|ui| {
                        ui.add_space(inset);
                        ui.label(RichText::new(i18n::bilingual("Chapters", "章节")).size(20.0).strong());
                    });
                    ScrollArea::horizontal().id_source(("detail-chapters", &media.id))
                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.add_space(inset);
                                for chapter in &media.chapters {
                                    let time = format_secs(chapter.position_ticks as f64 / TICKS);
                                    let label = if chapter.name.is_empty() { time } else { format!("{} · {time}", chapter.name) };
                                    if ui.add(egui::Button::new(label).min_size(Vec2::new(0.0, 35.0))).clicked() {
                                        let mut media = media.clone();
                                        media.progress_ticks = chapter.position_ticks;
                                        actions.push(Action::PlaySelected(media));
                                    }
                                }
                            });
                        });
                    ui.add_space(28.0);
                }

                if !self.prefs.library.hide_album && !detail.album.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(inset);
                        ui.label(RichText::new(i18n::bilingual("Album", "相册")).size(20.0).strong());
                    });
                    ScrollArea::horizontal().id_source(("detail-album", &item.id))
                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.add_space(inset);
                                for (index, tag) in detail.album.iter().enumerate() {
                                    let (rect, _) = ui.allocate_exact_size(Vec2::new(216.0, 121.5), Sense::hover());
                                    if ui.is_rect_visible(rect) {
                                        let texture = self.artwork(&item.id, &format!("Backdrop/{index}"), tag, 480, now);
                                        draw_image(ui, rect, texture, p, item, 8.0);
                                    }
                                }
                            });
                        });
                    ui.add_space(28.0);
                }

                if !detail.people.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(inset);
                        ui.label(theme::heading("Cast and crew", 20.0, p.text));
                    });
                    ScrollArea::horizontal()
                        .id_source(("detail-people", &item.id))
                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 16.0;
                                ui.add_space(inset);
                                for person in &detail.people {
                                    ui.vertical(|ui| {
                                        let (face, _) = ui.allocate_exact_size(Vec2::new(115.0, 150.0), Sense::hover());
                                        if ui.is_rect_visible(face) {
                                            if let Some(image) = person.primary_image_tag.as_deref().and_then(|tag| self.artwork(&person.id, "Primary", tag, 240, now)) {
                                                egui::Image::new((image, face.size())).rounding(8.0).paint_at(ui, face);
                                            } else {
                                                theme::poster_placeholder(ui, face, &person.id, &person.name, p);
                                            }
                                        }
                                        ui.add_sized([115.0, 18.0], egui::Label::new(theme::body(&person.name, 13.0, p.text)).truncate());
                                        if let Some(role) = &person.role {
                                            ui.add_sized([115.0, 16.0], egui::Label::new(theme::body(role, 11.0, p.dim)).truncate());
                                        }
                                    });
                                }
                            });
                        });
                    ui.add_space(28.0);
                }

                for (title, entries) in [("In this collection", &detail.children), ("More like this", &detail.similar)] {
                    if entries.is_empty() {
                        continue;
                    }
                    ui.horizontal(|ui| {
                        ui.add_space(inset - ui.spacing().item_spacing.x);
                        ui.label(theme::heading(title, 20.0, p.text));
                    });
                    ScrollArea::horizontal().id_source((title, &item.id)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add_space(inset - ui.spacing().item_spacing.x);
                            for entry in entries {
                                if self.poster_card(ui, entry, self.card_scale.width() * self.prefs.library.cover_scale, ctx, p, actions) {
                                    actions.push(Action::Open(entry.id.clone()));
                                }
                            }
                        });
                    });
                    ui.add_space(24.0);
                }
            });

        if self.detail_jump {
            let mut open = true;
            egui::Window::new(i18n::t("Go to episode").into_owned())
                .id(egui::Id::new("new-details-jump"))
                .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                .default_width(330.0)
                .resizable(false)
                .collapsible(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    if detail.seasons.len() > 1 {
                        egui::ComboBox::from_id_source("detail-jump-season")
                            .selected_text(active_season.map_or("", |season| season.name.as_str()))
                            .show_ui(ui, |ui| {
                                for season in &detail.seasons {
                                    if ui.selectable_label(Some(&season.id) == self.detail_season.as_ref(), &season.name).clicked() {
                                        actions.push(Action::Season(season.id.clone()));
                                    }
                                }
                            });
                    }
                    ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for ep in &detail.episodes {
                            if ui.selectable_label(ep.id == self.detail_media.item_id, format!("{}. {}", ep.index.unwrap_or(0), ep.name)).clicked() {
                                if ep.id != self.detail_media.item_id {
                                    actions.push(Action::DetailFocus(ep.clone()));
                                }
                                self.detail_reveal_episode = true;
                                self.detail_jump = false;
                            }
                        }
                    });
                });
            self.detail_jump &= open;
        }
        self.detail = Some(detail);
    }

    // -------------------------------------------------------------- dialogs

    fn draw_dialogs(&mut self, ctx: &egui::Context, p: &Palette, actions: &mut Vec<Action>) {
        let Some(mut form) = self.form.take() else {
            return;
        };
        let editing = form.editing;
        let busy = form.busy;
        let mut keep_open = true;
        let mut submit = false;
        let mut created = false;

        let title = if editing.is_some() {
            theme::tl("Sign in to this server")
        } else {
            theme::tl("Add a server")
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::new(0.0, -20.0))
            .min_width(430.0)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0_f32, p.line))
                    .rounding(egui::Rounding::same(14.0))
                    .inner_margin(egui::Margin::same(20.0)),
            )
            .show(ctx, |ui| {
                ui.label(theme::body(
                    "The address of your Emby server, then the account to use. Emby usually listens on port 8096.",
                    12.5,
                    p.dim,
                ));
                ui.add_space(14.0);
                ui.label(theme::body("Server address", 11.5, p.dim));
                ui.add(
                    egui::TextEdit::singleline(&mut form.url)
                        .hint_text("192.168.1.20:8096")
                        .desired_width(392.0),
                );
                ui.add_space(10.0);
                ui.label(theme::body("User name", 11.5, p.dim));
                ui.add(egui::TextEdit::singleline(&mut form.username).desired_width(392.0));
                ui.add_space(10.0);
                ui.label(theme::body("Password", 11.5, p.dim));
                ui.add(
                    egui::TextEdit::singleline(&mut form.password)
                        .password(true)
                        .desired_width(392.0),
                );

                if busy {
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.add_space(4.0);
                        ui.label(theme::body("Signing in\u{2026}", 12.0, p.accent));
                    });
                }
                if !form.error.is_empty() {
                    ui.add_space(12.0);
                    ui.label(theme::body(&form.error, 12.0, p.warn));
                }

                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    if !busy {
                        if btn(ui, &format!("  {}  ", theme::tl("Sign in")), true, p).clicked() {
                            submit = true;
                        }
                    }
                    if btn(ui, &format!("  {}  ", theme::tl("Cancel")), false, p).clicked() {
                        keep_open = false;
                    }
                });
                // Enter anywhere in the form signs in, as the address field is the
                // one people finish typing in.
                if !busy && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    submit = true;
                }
            });

        if submit {
            let url = form.url.trim().to_string();
            let username = form.username.trim().to_string();
            if url.is_empty() {
                form.error = theme::tl("Type the server's address first.");
            } else if username.is_empty() {
                form.error = theme::tl("Type the user name to sign in with.");
            } else {
                form.error.clear();
                form.url = url.clone();
                form.username = username.clone();
                form.busy = true;
                // The server exists before the first attempt, so a sign-in that
                // fails leaves something to retry instead of nothing at all.
                let idx = match form.editing {
                    Some(i) if i < self.servers.len() => i,
                    _ => {
                        let mut entry = ServerEntry::new(&url);
                        entry.username = username.clone();
                        entry.name = url.clone();
                        self.servers.push(entry);
                        created = true;
                        self.servers.len() - 1
                    }
                };
                if let Some(s) = self.servers.get_mut(idx) {
                    s.url = url.clone();
                    s.username = username.clone();
                    if s.device_id.trim().is_empty() {
                        s.device_id = new_device_id();
                    }
                }
                form.editing = Some(idx);
                self.active = Some(idx);
                self.save();
                let server_id = self
                    .servers
                    .get(idx)
                    .map(|s| s.id.clone())
                    .unwrap_or_default();
                let device_id = self
                    .servers
                    .get(idx)
                    .map(|s| s.device_id.clone())
                    .unwrap_or_default();
                self.worker.send(Job::Connect {
                    server_id,
                    url: url.clone(),
                    device_id,
                    username: username.clone(),
                    password: form.password.clone(),
                });
            }
        }

        if !keep_open {
            // Cancelling a server that was only just typed in should not leave a
            // half-added entry behind.
            if created {
                if let Some(idx) = form.editing {
                    if self
                        .servers
                        .get(idx)
                        .map(|s| !s.is_signed_in())
                        .unwrap_or(false)
                    {
                        self.servers.remove(idx);
                        if self.active == Some(idx) {
                            self.active = None;
                        } else if let Some(a) = self.active {
                            if a > idx {
                                self.active = Some(a - 1);
                            }
                        }
                        self.save();
                    }
                }
            }
        }

        if keep_open || form.busy {
            self.form = Some(form);
        }
        let _ = actions;
    }

    // ----------------------------------------------------------- screenshots

    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        if self.shot_path.is_none() {
            return;
        }
        ctx.request_repaint();
        self.frame += 1;
        if self.frame == 118 && self.shot_phase == ShotPhase::Drawing {
            self.shot_phase = ShotPhase::Requested;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
        }
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            if self.shot_phase != ShotPhase::Done {
                self.shot_phase = ShotPhase::Done;
                if let Some(path) = self.shot_path.clone() {
                    let ok = save_png(&image, &path);
                    if !ok {
                        eprintln!("could not write the screenshot to {}", path.display());
                    }
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if self.frame > 900 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

impl eframe::App for AppLite {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.flush_settings();
        if let Some(mut player) = self.player.take() {
            if player.report(&mut self.worker, PlaybackEvent::Stop, 0.0) {
                self.worker.flush_on_exit();
            }
        }
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.tray.as_ref().is_some_and(crate::platform::Tray::restored) {
            self.tray = None;
        }
        if self.prefs.close_to_tray && !crate::platform::Tray::exiting()
            && self.shot_path.is_none() && ctx.input(|i| i.viewport().close_requested())
        {
            if let Some(tray) = crate::platform::Tray::hide(self.host_hwnd) {
                self.tray = Some(tray);
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.apply(Action::ClosePlayer, ctx);
            }
        }
        if self.prefs.theme == ThemeMode::System {
            self.dark = frame.info().system_theme != Some(eframe::Theme::Light);
        }
        {
            // Says once, in the log, whether Chinese can really be drawn (a file
            // that opened is not the same as glyphs that exist).
            static ONCE: std::sync::Once = std::sync::Once::new();
            ONCE.call_once(|| {
                let id = egui::FontId::proportional(14.0);
                let has = ctx.fonts(|f| f.has_glyphs(&id, "\u{4e2d}\u{6587}"));
                eprintln!("font: Chinese glyphs available: {}", has);
            });
        }
        self.ingest_images(ctx);
        if self.style_for != Some(self.dark) {
            theme::apply_style(ctx, &theme::palette(self.dark));
            self.style_for = Some(self.dark);
        }
        let p = theme::palette(self.dark);
        for msg in self.worker.poll() {
            self.handle(msg, ctx);
        }
        self.update_check.poll();
        let now = ctx.input(|i| i.time);
        self.tick_player(ctx, now);
        if self.pending_view.is_some() && (!self.loading || !self.rows.is_empty()) {
            if let Some(v) = self.pending_view.take() {
                if v == View::Detail {
                    if let Some(item) = self.hero_items().first() {
                        self.open_detail(item.id.clone());
                    }
                } else {
                    self.view = v;
                }
                // Jumping straight to a screen from the command line still has to
                // fire the same load the nav button fires.
                let kick = match self.view {
                    View::Favorites => Some(Action::GoFavorites),
                    View::Aggregate => Some(Action::GoAggregate),
                    _ => None,
                };
                if let Some(a) = kick {
                    if let Some(t) = self.target() {
                        let _ = t;
                    }
                    self.apply(a, ctx);
                }
            }
        }

        let mut actions: Vec<Action> = Vec::new();

        let (space, back, ahead, esc, fullscreen, up, down, mute) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Space),
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::F11),
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::M),
            )
        });
        if self.form.is_none() {
            if let Some(player) = &mut self.player {
                let right_down = ctx.input(|i| i.key_down(egui::Key::ArrowRight));
                if ahead && player.menu.is_none() && player.phase == PlaybackPhase::Ready && player.boost_started.is_none() {
                    player.boost_started = Some(now);
                }
                if let Some(start) = player.boost_started {
                    if !right_down {
                        player.boost_started = None;
                        if let Some(speed) = player.boost_base_speed.take() {
                            actions.push(Action::SetPlaybackSpeed(speed));
                        } else if player.menu.is_none() {
                            actions.push(Action::SeekBy(f64::from(self.prefs.playback.forward)));
                        }
                    } else if now - start >= 0.42 && player.boost_base_speed.is_none() && player.playing {
                        player.boost_base_speed = Some(player.speed);
                        actions.push(Action::SetPlaybackSpeed(self.prefs.playback.hold_speed));
                    } else if now - start < 0.42 {
                        ctx.request_repaint_after(std::time::Duration::from_secs_f64(0.42 - (now - start)));
                    }
                }
                if player.menu.is_some() {
                    if esc {
                        player.menu = None;
                    }
                } else if esc {
                    actions.push(if ctx.input(|i| i.viewport().fullscreen == Some(true)) {
                        Action::ToggleFullscreen
                    } else {
                        Action::ClosePlayer
                    });
                } else {
                    if space {
                    actions.push(Action::TogglePlayer);
                    }
                    if back {
                        actions.push(Action::SeekBy(-f64::from(self.prefs.playback.rewind)));
                    }
                    if fullscreen {
                        actions.push(Action::ToggleFullscreen);
                    }
                    if up || down {
                        let level = if up {
                            player.volume.saturating_add(5).min(100)
                        } else {
                            player.volume.saturating_sub(5)
                        };
                        actions.push(Action::SetPlaybackVolume { level, commit: true });
                    }
                    if mute {
                        actions.push(Action::TogglePlaybackMute);
                    }
                }
            } else if esc && self.view == View::Settings {
                if self.settings_view.dialog.is_some() {
                    self.settings_view.dialog = None;
                } else if !ctx.memory(|memory| memory.any_popup_open()) {
                    actions.push(Action::BackFromSettings);
                }
            } else if esc && self.view == View::Detail {
                if self.detail_jump {
                    self.detail_jump = false;
                }
            } else if esc && self.view != View::Home {
                actions.push(Action::GoHome);
            }
        }

        if self.view != View::Home || self.player.is_some() {
            self.hero_was_visible = false;
        }
        if self.player.is_some() {
            self.draw_player(ctx, &mut actions);
        } else if self.servers.is_empty() && self.active.is_none() && self.view != View::Settings {
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(p.bg))
                .show(ctx, |ui| {
                    let full = ui.max_rect();
                    let bar = full.with_max_y(full.top() + home_ui::TOP_BAR_HEIGHT);
                    self.draw_welcome(ui, &p, &mut actions);
                    let buttons = home_ui::window_buttons(ui, bar, p.card, p.line, p.text);
                    home_ui::drag_window(ui, bar.with_max_x(buttons.left() - 8.0), ui.id().with("welcome-drag"));
                });
        } else {
            // 1.7.0 layout: no sidebar. Content fills the window behind a
            // floating top bar drawn as an overlay at y=0..64.
            egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(if self.view == View::Detail {
                Color32::from_rgb(45, 47, 46)
            } else { p.bg }))
                .show(ctx, |ui| {
                    self.draw_content(ui, ctx, &p, &mut actions);
                    // The settings screen has its own header (back arrow +
                    // title), so the floating bar is skipped there.
                    if !matches!(self.view, View::Settings | View::Detail) {
                        self.draw_floating_topbar(ui, ctx, &p, &mut actions);
                    }
                });
        }

        self.draw_dialogs(ctx, &p, &mut actions);
        self.draw_update_notice(ctx, &mut actions);

        for action in actions {
            self.apply(action, ctx);
        }

        if let Some(player) = self.player.as_mut().filter(|player| player.fullscreen_pending) {
            if ctx.input(|i| i.viewport().maximized == Some(true)) {
                // A maximized Win32 client area reserves taskbar space even in a monitor-sized window.
                player.restore_maximized = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
                ctx.request_repaint();
            } else {
                player.fullscreen_pending = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
            }
        }

        self.handle_screenshot(ctx);
    }
}

// ------------------------------------------------------------------ helpers

fn read_config() -> (Config, Option<String>) {
    let path = config_path();
    let decode = |bytes: &[u8]| serde_json::from_slice::<Config>(bytes)
        .map_err(|error| error.to_string());
    let result = match std::fs::read(&path) {
        Ok(bytes) => decode(&bytes).map(|config| (config, None)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match std::fs::read(path.with_file_name("config.json")) {
                Ok(bytes) => decode(&bytes).map(|config| {
                    let issue = write_config(&config, &path).err().map(|error| error.to_string());
                    (config, issue)
                }),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok((Config::default(), None)),
                Err(error) => Err(error.to_string()),
            }
        }
        Err(error) => Err(error.to_string()),
    };
    result.unwrap_or_else(|error| (Config::default(), Some(error)))
}

fn write_config(data: &impl Serialize, path: &std::path::Path) -> std::io::Result<()> {
    use std::io::Write;
    let parent = path.parent().ok_or_else(|| std::io::Error::other("Configuration directory missing"))?;
    std::fs::create_dir_all(parent)?;
    let payload = serde_json::to_vec_pretty(data).map_err(std::io::Error::other)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(&payload)?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn intro_range(chapters: &[Chapter]) -> Option<(f64, f64)> {
    let first = chapters.iter().find(|chapter| chapter.marker.eq_ignore_ascii_case("IntroStart"))?.position_ticks;
    let last = chapters.iter().find(|chapter| chapter.marker.eq_ignore_ascii_case("IntroEnd") && chapter.position_ticks > first)?.position_ticks;
    Some((first as f64 / TICKS, last as f64 / TICKS))
}

fn player_log(message: &str) {
    use std::io::Write;
    let path = settings_ui::mpv_logs();
    if std::fs::create_dir_all(&path).is_ok() {
        if let Ok(mut log) = std::fs::OpenOptions::new().create(true).append(true).open(path.join("playback.log")) {
            let _ = writeln!(log, "{message}");
        }
    }
}

fn include_for(kind: &str) -> String {
    match kind {
        "movies" => "Movie".to_string(),
        "tvshows" => "Series".to_string(),
        "boxsets" => "BoxSet".to_string(),
        "music" => "MusicAlbum".to_string(),
        "musicvideos" => "MusicVideo".to_string(),
        "homevideos" => "Video".to_string(),
        _ => String::new(),
    }
}

fn nav_item(ui: &mut Ui, icon_kind: &str, label: &str, selected: bool, p: &Palette) -> Response {
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 38.0), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 8.0, p.accent_soft);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 8.0, p.card_hover);
    }
    theme::icon(
        ui,
        icon_kind,
        Pos2::new(rect.left() + 20.0, rect.center().y),
        17.0,
        if selected { p.accent } else { p.dim },
    );
    ui.painter().text(
        Pos2::new(rect.left() + 40.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.5),
        if selected { p.text } else { p.dim },
    );
    resp
}

fn btn(ui: &mut Ui, label: &str, primary: bool, p: &Palette) -> Response {
    // The primary action is a white pill with dark ink: the palette's accent is
    // white to match the original, so tinting the text with it would vanish.
    let (fill, text, stroke) = if primary {
        (Color32::WHITE, p.bg, Stroke::NONE)
    } else {
        (p.card, p.text, Stroke::new(1.0_f32, p.line))
    };
    ui.add(
        egui::Button::new(RichText::new(label).size(13.0).color(text))
            .fill(fill)
            .stroke(stroke)
            .rounding(8.0)
            .min_size(Vec2::new(0.0, 34.0)),
    )
}

fn detail_icon_button(ui: &Ui, rect: Rect, icon: &str, label: &str, selected: bool, fill: Color32) -> Response {
    let response = ui.interact(rect, ui.id().with(("detail-icon", label)), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), i18n::t(label)));
    let ink = if selected { Color32::WHITE } else { Color32::from_rgb(205, 210, 210) };
    let fill = if response.hovered() || response.has_focus() { Color32::from_rgb(64, 69, 67) } else { fill };
    ui.painter().rect_filled(rect, rect.height() / 2.0, fill);
    if response.has_focus() {
        ui.painter().rect_stroke(rect, rect.height() / 2.0, Stroke::new(1.0_f32, ink));
    }
    theme::icon(ui, icon, rect.center(), 16.0, ink);
    if icon == "check" {
        ui.painter().circle_stroke(rect.center(), 6.7, Stroke::new(1.0_f32, ink));
    }
    response.on_hover_text(i18n::t(label))
}

fn feature_card(ui: &mut Ui, p: &Palette, title: &str, text: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(228.0, 116.0), Sense::hover());
    ui.painter().rect_filled(rect, 12.0, p.card);
    ui.painter().rect_stroke(rect, 12.0, Stroke::new(1.0_f32, p.line));
    ui.painter().text(
        Pos2::new(rect.left() + 16.0, rect.top() + 16.0),
        Align2::LEFT_TOP,
        title,
        FontId::proportional(13.5),
        p.text,
    );
    // Wrap the body text by hand into three short lines.
    let mut y = rect.top() + 42.0;
    for line in wrap(text, 30).into_iter().take(4) {
        ui.painter().text(
            Pos2::new(rect.left() + 16.0, y),
            Align2::LEFT_TOP,
            line,
            FontId::proportional(11.5),
            p.dim,
        );
        y += 15.0;
    }
    ui.add_space(0.0);
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word.chars().count() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current.clone());
            current = word.to_string();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Draws artwork into a rounded rectangle, so a poster sits in the wall without
/// square corners poking out of the cover. Falls back to the placeholder.
fn draw_poster(
    ui: &Ui,
    rect: Rect,
    tex: Option<TextureId>,
    p: &Palette,
    item: &MediaItem,
    rounding: f32,
) {
    match tex {
        Some(id) => {
            let size = ui.ctx().tex_manager().read().meta(id).map(|m| m.size);
            let tex_size = size
                .map(|s| Vec2::new(s[0] as f32, s[1] as f32))
                .unwrap_or(Vec2::new(2.0, 3.0));
            let uv = cover_uv(rect.size(), tex_size);
            let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
            let mut shape = egui::epaint::RectShape::filled(rect, rounding, Color32::WHITE);
            shape.fill_texture_id = id;
            shape.uv = uv;
            painter.add(shape);
        }
        None => theme::poster_placeholder(ui, rect, &item.id, &item.name, p),
    }
}

/// The one line under a card: when it was last watched, else what it is. Shared
/// by the poster wall and the list, so both say the same thing.
/// The name to print on a card. An episode shows its series (its own Name is a
/// release file name), and a Name that really is a file name loses the
/// extension and the provider tags the server appends.
fn display_title(item: &MediaItem) -> String {
    if item.kind == "Episode" && !item.series_name.is_empty() {
        return clean_name(&item.series_name);
    }
    clean_name(&item.name)
}

fn clean_name(name: &str) -> String {
    let mut s = name.to_string();
    for marker in ["{tmdbid-", "{imdb-", "{tvdb-", "{edition-"] {
        if let Some(i) = s.find(marker) {
            s.truncate(i);
        }
    }
    if let Some(i) = s.rfind('.') {
        let ext = &s[i + 1..];
        if (1..=4).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphabetic()) {
            s.truncate(i);
        }
    }
    s.trim().trim_end_matches(['.', ' ', '_']).trim().to_string()
}

fn card_meta(item: &MediaItem) -> String {
    if !item.technical.is_empty() {
        return item.technical.join(" · ");
    }
    if let Some(cap) = item.watched_caption() {
        return i18n::t(&cap).to_string();
    }
    if item.kind == "Episode" {
        return match (item.parent_index, item.index) {
            (Some(s), Some(e)) => format!("S{:02}E{:02}", s, e),
            _ => item.season_name.clone(),
        };
    }
    if !item.series_name.is_empty() {
        return item.series_name.clone();
    }
    let kind = i18n::t(&item.kind).to_string();
    match item.year {
        Some(y) if y > 0 => format!("{}  \u{b7}  {}", y, kind),
        _ => kind,
    }
}

/// Paint artwork into `rect`; a placeholder when the image has not arrived.
fn draw_image(
    ui: &Ui,
    rect: Rect,
    tex: Option<TextureId>,
    p: &Palette,
    item: &MediaItem,
    rounding: f32,
) {
    match tex {
        Some(id) => {
            let size = ui.ctx().tex_manager().read().meta(id).map(|m| m.size);
            let tex_size = size
                .map(|s| Vec2::new(s[0] as f32, s[1] as f32))
                .unwrap_or(Vec2::new(2.0, 3.0));
            let uv = cover_uv(rect.size(), tex_size);
            egui::Image::new((id, tex_size)).uv(uv).rounding(rounding).paint_at(ui, rect);
        }
        None => {
            theme::poster_placeholder(ui, rect, &item.id, &item.name, p);
        }
    }
}

fn cover_uv(rect: Vec2, tex: Vec2) -> Rect {
    if rect.x <= 0.0 || rect.y <= 0.0 || tex.x <= 0.0 || tex.y <= 0.0 {
        return Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));
    }
    let rect_aspect = rect.x / rect.y;
    let tex_aspect = tex.x / tex.y;
    if rect_aspect > tex_aspect {
        let frac = (tex_aspect / rect_aspect).clamp(0.0, 1.0);
        let y = (1.0 - frac) / 2.0;
        Rect::from_min_max(Pos2::new(0.0, y), Pos2::new(1.0, 1.0 - y))
    } else {
        let frac = (rect_aspect / tex_aspect).clamp(0.0, 1.0);
        let x = (1.0 - frac) / 2.0;
        Rect::from_min_max(Pos2::new(x, 0.0), Pos2::new(1.0 - x, 1.0))
    }
}

fn save_png(img: &egui::ColorImage, path: &std::path::Path) -> bool {
    let w = img.size[0] as u32;
    let h = img.size[1] as u32;
    if w == 0 || h == 0 {
        return false;
    }
    let mut buf: Vec<u8> = Vec::with_capacity((w as usize) * (h as usize) * 4);
    for px in &img.pixels {
        buf.extend_from_slice(&[px.r(), px.g(), px.b(), 255]);
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    image::save_buffer(path, &buf, w, h, image::ColorType::Rgba8).is_ok()
}

fn open_external(url: &str) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

// Silence "unused" for helpers kept for readability.
#[allow(dead_code)]
fn _unused(p: &Palette) -> Color32 {
    p.dim
}

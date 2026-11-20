//! Persisted settings that are read by the corresponding Media Lite features.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Preferences {
    pub language: String,
    pub theme: ThemeMode,
    pub start_aggregate: bool,
    pub close_to_tray: bool,
    pub check_updates_on_start: bool,
    pub library: Library,
    pub backup_settings: bool,
    pub webdav: WebDav,
    pub watched_percent: u8,
    pub network: Network,
    pub playback: Playback,
    pub cache: Cache,
    pub subtitles: Subtitles,
    pub external: External,
    pub danmaku: Danmaku,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            language: "auto".into(), theme: ThemeMode::Dark, start_aggregate: false, close_to_tray: false,
            check_updates_on_start: true,
            library: Library::default(), backup_settings: true, webdav: WebDav::default(), watched_percent: 80,
            network: Network::default(), playback: Playback::default(), cache: Cache::default(),
            subtitles: Subtitles::default(), external: External::default(), danmaku: Danmaku::default(),
        }
    }
}

impl Preferences {
    pub fn validate(&mut self) {
        if !matches!(self.language.as_str(), "auto" | "zh" | "zh-Hant" | "en") {
            self.language = "auto".into();
        }
        self.watched_percent = self.watched_percent.min(100);
        if !self.network.proxy_address.is_empty() && !Network::valid_proxy_address(&self.network.proxy_address) {
            self.network.proxy_address.clear();
        }
        if self.network.proxy == ProxyMode::Http && self.network.proxy_address.is_empty() {
            self.network.proxy = ProxyMode::System;
        }
        self.library.cover_scale = self.library.cover_scale.clamp(0.7, 1.5);
        self.library.data_days = self.library.data_days.clamp(1, 30);
        self.library.image_days = self.library.image_days.clamp(1, 30);
        self.playback.forward = self.playback.forward.clamp(1, 120);
        self.playback.rewind = self.playback.rewind.clamp(1, 120);
        self.playback.hold_speed = self.playback.hold_speed.clamp(1.0, 4.0);
        self.cache.megabytes = self.cache.megabytes.clamp(16, 2048);
        self.cache.seconds = self.cache.seconds.clamp(5, 900);
        self.cache.disk_gb = self.cache.disk_gb.min(64);
        self.subtitles.size = self.subtitles.size.clamp(12, 140);
        self.subtitles.position = self.subtitles.position.min(150);
        self.subtitles.secondary_position = self.subtitles.secondary_position.min(150);
        self.subtitles.border = self.subtitles.border.clamp(0.0, 10.0);
        self.subtitles.shadow = self.subtitles.shadow.clamp(0.0, 10.0);
        self.subtitles.scale = self.subtitles.scale.clamp(0.5, 3.0);
        self.external.episode_count = self.external.episode_count.clamp(1, 30);
        self.danmaku.scroll_lines = self.danmaku.scroll_lines.min(20);
        self.danmaku.top_lines = self.danmaku.top_lines.min(10);
        self.danmaku.bottom_lines = self.danmaku.bottom_lines.min(10);
        self.danmaku.scale = self.danmaku.scale.clamp(0.5, 3.0);
        self.danmaku.line_spacing = self.danmaku.line_spacing.min(40);
        self.danmaku.opacity = self.danmaku.opacity.min(100);
        self.danmaku.speed = self.danmaku.speed.clamp(0.25, 3.0);
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct WebDav {
    pub address: String,
    pub username: String,
    pub path: String,
}

impl Default for WebDav {
    fn default() -> Self {
        Self { address: String::new(), username: String::new(), path: "/MediaLite/MediaLiteBackup.json".into() }
    }
}

impl WebDav {
    pub fn endpoint(&self) -> Option<reqwest::Url> {
        let mut base = reqwest::Url::parse(self.address.trim()).ok()?;
        if !matches!(base.scheme(), "http" | "https") || base.host_str().is_none()
            || !base.username().is_empty() || base.password().is_some()
            || base.query().is_some() || base.fragment().is_some() || self.path.trim_matches('/').is_empty()
        {
            return None;
        }
        let path = format!("{}/{}", base.path().trim_end_matches('/'), self.path.trim_start_matches('/'));
        base.set_path(&path);
        Some(base)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode { System, Light, #[default] Dark }

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality { Low, #[default] Medium, High }

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChapterMode { #[default] Always, Movies, Never }

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Library {
    pub hide_library: bool,
    pub hide_resume: bool,
    pub hide_recommend: bool,
    pub hide_unplayed: bool,
    pub recommend_filter: String,
    pub show_rating: bool,
    pub hide_album: bool,
    pub search_media: bool,
    pub search_history: bool,
    pub cover_scale: f32,
    pub quality: Quality,
    pub chapters: ChapterMode,
    pub data_cache: bool,
    pub data_days: u8,
    pub image_days: u8,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            hide_library: false, hide_resume: false, hide_recommend: false,
            hide_unplayed: false, recommend_filter: String::new(), show_rating: true,
            hide_album: false, search_media: false, search_history: false, cover_scale: 1.0,
            quality: Quality::Medium, chapters: ChapterMode::Always, data_cache: true,
            data_days: 30, image_days: 30,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyMode { #[default] System, Http, Direct }

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Network {
    pub ignore_ssl: bool,
    pub proxy: ProxyMode,
    pub proxy_address: String,
}

impl Default for Network {
    fn default() -> Self {
        Self { ignore_ssl: true, proxy: ProxyMode::System, proxy_address: String::new() }
    }
}

impl Network {
    pub fn valid_proxy_address(value: &str) -> bool {
        reqwest::Url::parse(value).is_ok_and(|url| url.scheme() == "http" && url.host_str().is_some()
            && url.username().is_empty() && url.password().is_none() && url.path() == "/"
            && url.query().is_none() && url.fragment().is_none())
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerChoice { #[default] BuiltIn, Mpv, PotPlayer }

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntroMode { #[default] Off, Button, Auto }

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Decoder { #[default] Auto, Software, D3d11, Dxva2 }

impl Decoder {
    pub fn mpv(self) -> &'static str {
        match self {
            Self::Auto => "auto-copy",
            Self::Software => "no",
            Self::D3d11 => "d3d11va-copy",
            Self::Dxva2 => "dxva2-copy",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Playback {
    pub default_player: PlayerChoice,
    pub forward: u16,
    pub rewind: u16,
    pub hold_speed: f64,
    pub decoder: Decoder,
    pub fullscreen: bool,
    pub audio_language: String,
    pub subtitle_language: String,
    pub secondary: bool,
    pub secondary_language: String,
    pub preferred_version: String,
    pub intro: IntroMode,
    pub preview: bool,
    pub chapters: bool,
    pub remember_aspect: bool,
    pub last_fill: bool,
    pub fast_decode: bool,
    pub stereo: bool,
    pub stretch_subs: bool,
    pub log: bool,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            default_player: PlayerChoice::BuiltIn, forward: 10, rewind: 10, hold_speed: 2.0,
            decoder: Decoder::Auto, fullscreen: false, audio_language: String::new(),
            subtitle_language: String::new(), secondary: false, secondary_language: "no".into(),
            preferred_version: String::new(), intro: IntroMode::Off, preview: true,
            chapters: true, remember_aspect: false,
            last_fill: false, fast_decode: false, stereo: false, stretch_subs: false, log: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Cache {
    pub megabytes: u32,
    pub seconds: u16,
    pub disk_gb: u16,
    pub disk_parent: Option<PathBuf>,
}

impl Default for Cache {
    fn default() -> Self {
        Self { megabytes: 300, seconds: 300, disk_gb: 0, disk_parent: None }
    }
}

impl Cache {
    pub fn directory(&self) -> PathBuf {
        self.disk_parent.as_deref().map_or_else(
            || crate::net::cache_root().join("mpv_disk_cache"),
            |parent| parent.join("Media Lite cache"),
        )
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssStyle { #[default] No, Yes, Force, Scale, Strip }

impl AssStyle {
    pub fn mpv(self) -> &'static str {
        match self { Self::No => "no", Self::Yes => "yes", Self::Force => "force", Self::Scale => "scale", Self::Strip => "strip" }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Subtitles {
    pub size: u16,
    pub position: u8,
    pub secondary_position: u8,
    pub border: f32,
    pub shadow: f32,
    pub scale: f32,
    pub bold: bool,
    pub color: u32,
    pub border_color: u32,
    pub shadow_color: u32,
    pub ass: AssStyle,
    pub secondary_ass: AssStyle,
}

impl Default for Subtitles {
    fn default() -> Self {
        Self {
            size: 55, position: 100, secondary_position: 0, border: 3.0, shadow: 0.0,
            scale: 1.0, bold: false, color: 0xFFFFFFFF, border_color: 0xFF000000,
            shadow_color: 0, ass: AssStyle::No, secondary_ass: AssStyle::Strip,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct External {
    pub mpv: bool,
    pub mpv_path: Option<PathBuf>,
    pub mpv_proxy: bool,
    pub potplayer: bool,
    pub potplayer_path: Option<PathBuf>,
    pub episode_count: u8,
    pub minimize_main: bool,
}

impl Default for External {
    fn default() -> Self {
        Self { mpv: false, mpv_path: None, mpv_proxy: true, potplayer: false, potplayer_path: None, episode_count: 5, minimize_main: false }
    }
}

impl External {
    pub fn executable(&self, choice: PlayerChoice) -> Option<PathBuf> {
        let (enabled, explicit, names): (bool, Option<&Path>, &[&str]) = match choice {
            PlayerChoice::BuiltIn => return None,
            PlayerChoice::Mpv => (self.mpv, self.mpv_path.as_deref(), &["mpv.exe"]),
            PlayerChoice::PotPlayer => (self.potplayer, self.potplayer_path.as_deref(), &["PotPlayerMini64.exe", "PotPlayerMini.exe"]),
        };
        if !enabled {
            return None;
        }
        if let Some(path) = explicit {
            return path.is_file().then(|| path.to_path_buf());
        }
        std::env::var_os("PATH")
            .into_iter()
            .flat_map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
            .filter(|folder| folder.is_absolute())
            .flat_map(|folder| names.iter().map(move |name| folder.join(name)))
            .find(|path| path.is_file())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Danmaku {
    pub enabled: bool,
    pub api_url: String,
    pub scroll_lines: u8,
    pub top_lines: u8,
    pub bottom_lines: u8,
    pub scale: f32,
    pub line_spacing: u8,
    pub opacity: u8,
    pub speed: f32,
    pub sync_speed: bool,
    pub bold: bool,
    pub compatible: bool,
    pub remember: bool,
    pub hide_heatmap: bool,
    pub blocked_words: String,
}

impl Default for Danmaku {
    fn default() -> Self {
        Self {
            enabled: true, scroll_lines: 5, top_lines: 3, bottom_lines: 3, scale: 1.0,
            api_url: String::new(),
            line_spacing: 4, opacity: 100, speed: 1.0, sync_speed: false, bold: false,
            compatible: false, remember: true, hide_heatmap: false, blocked_words: String::new(),
        }
    }
}

/// Multi-line keys as in the original preference editors. A slash-delimited
/// line is a regex. Literal lines can match any of the supplied metadata.
#[derive(Clone, Default)]
pub struct TextRules {
    literals: Vec<String>,
    regexes: Vec<regex::Regex>,
}

impl TextRules {
    pub fn new(text: &str) -> Result<Self, regex::Error> {
        let mut rules = Self { literals: Vec::new(), regexes: Vec::new() };
        for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
            if line.len() > 2 && line.starts_with('/') && line.ends_with('/') {
                rules.regexes.push(regex::RegexBuilder::new(&line[1..line.len() - 1]).case_insensitive(true).build()?);
            } else {
                rules.literals.push(line.to_lowercase());
            }
        }
        Ok(rules)
    }

    pub fn matches(&self, value: &str) -> bool {
        let lower = value.to_lowercase();
        self.literals.iter().any(|key| lower.contains(key)) || self.regexes.iter().any(|pattern| pattern.is_match(value))
    }
}

//! Shared data types.

use serde::{Deserialize, Serialize};

/// Ticks per second in the Emby API (100 ns each).
pub const TICKS: f64 = 10_000_000.0;

/// One Emby (or Jellyfin/Emby-compatible) server that the person has added.
/// Any number of these may exist: the list is a plain vector with no cap.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerEntry {
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub user_name: String,
    #[serde(default)]
    pub server_version: Option<String>,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub direct_playback: bool,
    /// A built-in sample library, so the layout can be seen without a server.
    #[serde(default)]
    pub sample: bool,
}

impl ServerEntry {
    pub fn new(url: &str) -> Self {
        Self {
            id: new_id(),
            name: String::new(),
            url: url.to_string(),
            username: String::new(),
            token: None,
            user_id: None,
            user_name: String::new(),
            server_version: None,
            device_id: new_device_id(),
            direct_playback: false,
            sample: false,
        }
    }

    pub fn sample_server() -> Self {
        Self {
            id: "sample".to_string(),
            name: "Sample Library".to_string(),
            url: "sample://library".to_string(),
            username: "You".to_string(),
            token: None,
            user_id: Some("sample-user".to_string()),
            user_name: "You".to_string(),
            server_version: Some("4.8.0".to_string()),
            device_id: new_device_id(),
            direct_playback: false,
            sample: true,
        }
    }

    pub fn is_signed_in(&self) -> bool {
        self.sample || (self.token.is_some() && self.user_id.is_some())
    }
}

pub fn new_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("s{:x}{:x}", n, std::process::id())
}

pub fn new_device_id() -> String {
    // 32 hex characters, as the Emby scheme expects.
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(1);
    let mut state = now ^ ((std::process::id() as u128) << 41);
    let mut out = String::new();
    for _ in 0..32 {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let nibble = ((state >> 60) & 0xF) as u32;
        out.push(char::from_digit(nibble, 16).unwrap_or('0'));
    }
    out
}

/// A signed-in session on one server.
#[derive(Clone, Debug, Default)]
pub struct Session {
    pub token: String,
    pub user_id: String,
    pub user_name: String,
}

/// One library, movie, series, season or episode.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct MediaItem {
    pub id: String,
    pub name: String,
    /// Movie, Series, Episode, Season, BoxSet, Person, Video, CollectionFolder …
    pub kind: String,
    pub series_id: Option<String>,
    pub season_id: Option<String>,
    pub season_name: String,
    /// Episode number within its season.
    pub index: Option<i64>,
    /// Season number within its series.
    pub parent_index: Option<i64>,
    pub overview: String,
    pub year: Option<i64>,
    pub runtime_ticks: Option<i64>,
    pub official_rating: String,
    pub community_rating: Option<f64>,
    pub played: bool,
    pub favorite: bool,
    pub progress_ticks: i64,
    pub image_tag: Option<String>,
    /// When the poster comes from a parent (an episode borrowing its series
    /// art), the item id that actually owns that image.
    pub image_id: Option<String>,
    pub backdrop_tag: Option<String>,
    /// Same, for the backdrop.
    pub backdrop_id: Option<String>,
    #[serde(default)]
    pub logo: Option<(String, String)>,
    pub series_name: String,
    pub genres: Vec<String>,
    pub child_count: Option<i64>,
    pub tagline: String,
    pub premiere: String,
    /// Episodes (or children) the person has not watched yet, when the server says.
    pub unplayed: Option<i64>,
    /// UserData.LastPlayedDate as the server sent it, e.g. 2026-11-09T20:15:03.0000000Z.
    pub last_played: String,
    #[serde(default)]
    pub technical: Vec<String>,
    #[serde(default)]
    pub chapters: Vec<Chapter>,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Chapter {
    pub name: String,
    pub position_ticks: i64,
    #[serde(default)]
    pub marker: String,
    #[serde(default)]
    pub image_tag: Option<String>,
    #[serde(default)]
    pub index: usize,
}

impl MediaItem {
    pub fn progress_fraction(&self) -> f32 {
        match self.runtime_ticks {
            Some(total) if total > 0 => (self.progress_ticks as f32 / total as f32).clamp(0.0, 1.0),
            _ => 0.0,
        }
    }

    pub fn progress_secs(&self) -> f64 {
        self.progress_ticks as f64 / TICKS
    }

    pub fn runtime_secs(&self) -> Option<f64> {
        self.runtime_ticks.map(|t| t as f64 / TICKS)
    }

    pub fn is_playable(&self) -> bool {
        matches!(
            self.kind.as_str(),
            "Movie" | "Episode" | "Video" | "MusicVideo" | "Trailer" | "Clip",
        )
    }

    pub fn subtitle(&self) -> String {
        let mut bits: Vec<String> = Vec::new();
        if let Some(y) = self.year {
            if y > 0 {
                bits.push(y.to_string());
            }
        }
        if let Some(r) = self.runtime_secs() {
            bits.push(format_secs(r));
        }
        bits.push(self.kind.clone());
        bits.join("  ·  ")
    }

    /// A short line saying when this was last watched, when the server said so:
    /// "Watched today", "Watched yesterday", or the date itself. The comparison
    /// is in UTC, so something watched late at night can be called yesterday.
    pub fn watched_caption(&self) -> Option<String> {
        let day: String = self.last_played.chars().take(10).collect();
        if day.chars().count() < 10 {
            return None;
        }
        if day == today_iso() {
            return Some("Watched today".to_string());
        }
        if day == yesterday_iso() {
            return Some("Watched yesterday".to_string());
        }
        Some(day)
    }
}

/// One horizontal strip on the home screen.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Row {
    pub title: String,
    pub items: Vec<MediaItem>,
}

/// Everything the detail page needs.
#[derive(Clone, Debug, Default)]
pub struct ItemDetail {
    pub item: MediaItem,
    pub logo: Option<(String, String)>,
    pub end_year: Option<i64>,
    pub continuing: bool,
    pub people: Vec<PersonCredit>,
    pub seasons: Vec<MediaItem>,
    pub episodes: Vec<MediaItem>,
    pub next_episode: Option<MediaItem>,
    pub similar: Vec<MediaItem>,
    pub children: Vec<MediaItem>,
    pub album: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PersonCredit {
    pub id: String,
    pub name: String,
    pub role: Option<String>,
    pub primary_image_tag: Option<String>,
}

pub fn format_secs(secs: f64) -> String {
    let total = secs.max(0.0) as i64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{}:{:02}:{:02}", h, m, s)
    } else {
        format!("{}:{:02}", m, s)
    }
}

pub fn ticks_from_secs(secs: f64) -> i64 {
    (secs * TICKS) as i64
}

/// Today's date as YYYY-MM-DD, in UTC.
pub fn today_iso() -> String {
    iso_from_days(today_days())
}

/// Yesterday's date as YYYY-MM-DD, in UTC.
pub fn yesterday_iso() -> String {
    iso_from_days(today_days() - 1)
}

fn today_days() -> i64 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    secs.div_euclid(86_400)
}

/// Days since 1970-01-01 to a calendar date, so no date library is needed
/// (Howard Hinnant's civil-from-days algorithm).
fn iso_from_days(days: i64) -> String {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}", y, m, d)
}

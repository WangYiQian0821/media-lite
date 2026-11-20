use crate::{api, i18n, preferences::TextRules};
use eframe::egui::Color32;
use reqwest::Url;
use serde::Deserialize;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::time::Duration;

pub(super) struct Comment {
    pub time: f64,
    pub mode: u8,
    pub size: f32,
    pub color: Color32,
    pub text: String,
}

impl Comment {
    fn new(time: f64, mode: u8, size: u16, color: u32) -> Option<Self> {
        (time.is_finite() && time >= 0.0 && (1..=6).contains(&mode)).then(|| Self {
            time, mode, size: f32::from(size.clamp(14, 38)),
            color: Color32::from_rgb((color >> 16) as u8, (color >> 8) as u8, color as u8),
            text: String::new(),
        })
    }

    fn filtered(mut self, rules: &TextRules) -> Option<Self> {
        if self.text.trim().is_empty() || rules.matches(&self.text) {
            return None;
        }
        self.text = self.text.chars().take(250).collect();
        Some(self)
    }
}

pub(super) fn read_xml(path: &Path, rules: &TextRules) -> Result<Vec<Comment>, Box<dyn std::error::Error>> {
    use xml::reader::{EventReader, XmlEvent};

    let input = BufReader::new(File::open(path)?).take(16 * 1024 * 1024);
    let mut comments = Vec::new();
    let mut current: Option<Comment> = None;
    for event in EventReader::new(input) {
        match event? {
            XmlEvent::StartElement { name, attributes, .. } if name.local_name == "d" => {
                current = attributes.iter().find(|attr| attr.name.local_name == "p").and_then(|attr| {
                    let mut fields = attr.value.split(',');
                    Comment::new(
                        fields.next()?.parse().ok()?,
                        fields.next()?.parse().ok()?,
                        fields.next()?.parse().ok()?,
                        fields.next()?.parse().ok()?,
                    )
                });
            }
            XmlEvent::Characters(text) | XmlEvent::CData(text) => {
                if let Some(comment) = &mut current {
                    comment.text.push_str(&text);
                }
            }
            XmlEvent::EndElement { name } if name.local_name == "d" => {
                if let Some(comment) = current.take().and_then(|comment| comment.filtered(rules)) {
                    comments.push(comment);
                }
            }
            _ => {}
        }
    }
    comments.sort_by(|a, b| a.time.total_cmp(&b.time));
    Ok(comments)
}

#[derive(Clone)]
pub(super) struct Endpoint(Url);

impl Endpoint {
    pub fn parse(address: &str) -> Option<Self> {
        let mut url = Url::parse(address.trim()).ok()?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none()
            || !url.username().is_empty() || url.password().is_some()
            || url.query().is_some() || url.fragment().is_some()
        {
            return None;
        }
        if !url.path().trim_end_matches('/').ends_with("/api/v2") {
            url.path_segments_mut().ok()?.pop_if_empty().extend(["api", "v2"]);
        }
        Some(Self(url))
    }

    pub fn search(&self, title: &str, episode: &str) -> Result<SearchResults, String> {
        let mut url = self.0.clone();
        url.path_segments_mut().expect("validated HTTP URL").pop_if_empty().extend(["search", "episodes"]);
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("anime", title);
            if !episode.is_empty() {
                query.append_pair("episode", episode);
            }
        }
        let response: SearchResponse = self.get_json(url)?;
        if response.success == Some(false) {
            return Err(i18n::bilingual("The danmaku API rejected the search", "弹幕 API 拒绝了搜索请求").into_owned());
        }
        let Some(animes) = response.animes else {
            return Ok(SearchResults { episodes: Vec::new(), has_more: response.has_more });
        };
        let episodes = animes.into_iter().flat_map(|anime| {
            anime.episodes.into_iter().filter(|episode| episode.episode_id > 0).map(move |episode| Episode {
                id: episode.episode_id, series: anime.anime_title.clone(), title: episode.episode_title,
            })
        }).collect();
        Ok(SearchResults { episodes, has_more: response.has_more })
    }

    pub fn comments(&self, episode: u64, rules: &TextRules) -> Result<Vec<Comment>, String> {
        let mut url = self.0.clone();
        url.path_segments_mut().expect("validated HTTP URL").pop_if_empty().extend(["comment", &episode.to_string()]);
        url.query_pairs_mut().extend_pairs([("withRelated", "true"), ("chConvert", "0")]);
        let response: CommentsResponse = self.get_json(url)?;
        let mut comments: Vec<_> = response.comments.into_iter().filter_map(|entry| {
            let parameters = entry.p?;
            let mut fields = parameters.split(',').map(str::trim);
            let time = fields.next()?.parse().ok()?;
            let mode = fields.next()?.parse().ok()?;
            let third = fields.next()?;
            // Some compatible servers return the eight Bilibili XML fields in p.
            let (size, color) = if fields.clone().count() >= 5 {
                (third.parse().ok()?, fields.next()?)
            } else {
                (25, third)
            };
            let mut comment = Comment::new(time, mode, size, color.parse().ok()?)?;
            comment.text = entry.m?;
            comment.filtered(rules)
        }).collect();
        comments.sort_by(|a, b| a.time.total_cmp(&b.time));
        Ok(comments)
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, url: Url) -> Result<T, String> {
        let request = api::client()?.get(url).timeout(Duration::from_secs(15)).header("Accept", "application/json");
        let body = api::read_bounded(request, 16 * 1024 * 1024)?;
        serde_json::from_slice(&body).map_err(|_| {
            i18n::bilingual("The danmaku API returned an unexpected response", "弹幕 API 返回了无法识别的数据").into_owned()
        })
    }
}

pub(super) struct SearchResults {
    pub episodes: Vec<Episode>,
    pub has_more: bool,
}

pub(super) struct Episode {
    pub id: u64,
    pub series: String,
    pub title: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchResponse {
    success: Option<bool>,
    #[serde(default)]
    has_more: bool,
    animes: Option<Vec<SearchAnime>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchAnime {
    anime_title: String,
    episodes: Vec<SearchEpisode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchEpisode {
    episode_id: u64,
    episode_title: String,
}

#[derive(Deserialize)]
struct CommentsResponse {
    comments: Vec<RemoteComment>,
}

#[derive(Deserialize)]
struct RemoteComment {
    p: Option<String>,
    m: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_reverse_proxy_prefix_and_adds_v2_once() {
        for address in [
            "http://127.0.0.1:8080/private",
            "http://127.0.0.1:8080/private/",
            "http://127.0.0.1:8080/private/api/v2",
            "http://127.0.0.1:8080/private/api/v2/",
        ] {
            let mut url = Endpoint::parse(address).unwrap().0;
            url.path_segments_mut().unwrap().pop_if_empty().extend(["search", "episodes"]);
            assert_eq!(url.as_str(), "http://127.0.0.1:8080/private/api/v2/search/episodes");
        }
    }
}

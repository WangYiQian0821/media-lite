use crate::{api, i18n};
use reqwest::Url;
use scraper::{ElementRef, Html, Selector};
use std::collections::HashSet;
use std::io::Write;

const HOST: &str = "www.subtitlecat.com";

#[derive(Clone)]
pub(super) struct SearchResult {
    pub title: String,
    pub page: Url,
}

#[derive(Clone)]
pub(super) struct Subtitle {
    pub language: String,
    url: Url,
}

pub(super) fn search_url(query: &str) -> Url {
    let mut url = Url::parse("https://www.subtitlecat.com/index.php").expect("SubtitleCat search URL");
    url.query_pairs_mut().append_pair("search", query.trim());
    url
}

pub(super) fn search(query: &str) -> Result<Vec<SearchResult>, String> {
    let url = search_url(query);
    let page = read_page(&url)?;
    let search_field = Selector::parse("form input[name=search]").expect("search field selector");
    if page.select(&search_field).next().is_none() {
        return Err(i18n::bilingual("SubtitleCat returned an unexpected search page", "无法读取 SubtitleCat 搜索结果").into_owned());
    }
    let anchors = Selector::parse("table a[href]").expect("result link selector");
    let mut seen = HashSet::new();
    Ok(page.select(&anchors).filter_map(|anchor| {
        let page = site_link(&url, anchor.value().attr("href")?, ".html")?;
        let title = text(anchor);
        (!title.is_empty() && seen.insert(page.clone())).then_some(SearchResult { title, page })
    }).collect())
}

pub(super) fn subtitles(result: &SearchResult) -> Result<Vec<Subtitle>, String> {
    let page = read_page(&result.page)?;
    let rows = Selector::parse(".sub-single").expect("language row selector");
    let label = Selector::parse(":scope > span:nth-child(2)").expect("language label selector");
    let links = Selector::parse("a[href]").expect("download link selector");
    let mut seen = HashSet::new();
    Ok(page.select(&rows).filter_map(|row| {
        let language = text(row.select(&label).next()?);
        let url = row.select(&links).find_map(|anchor| site_link(&result.page, anchor.value().attr("href")?, ".srt"))?;
        (!language.is_empty() && seen.insert(url.clone())).then_some(Subtitle { language, url })
    }).collect())
}

pub(super) fn download(result: &SearchResult, subtitle: &Subtitle) -> Result<tempfile::TempPath, String> {
    let request = api::public_https_client()?.get(subtitle.url.clone())
        .header(reqwest::header::REFERER, result.page.as_str());
    let bytes = api::read_bounded(request, 8 * 1024 * 1024)?;
    let prefix = &bytes[..bytes.len().min(8192)];
    let text = if prefix.starts_with(&[0xff, 0xfe]) || prefix.starts_with(&[0xfe, 0xff]) {
        let units: Vec<_> = prefix[2..].chunks_exact(2).map(|pair| {
            if prefix[0] == 0xff { u16::from_le_bytes([pair[0], pair[1]]) }
            else { u16::from_be_bytes([pair[0], pair[1]]) }
        }).collect();
        std::borrow::Cow::Owned(String::from_utf16_lossy(&units))
    } else {
        String::from_utf8_lossy(prefix)
    };
    let cue = regex::Regex::new(r"(?m)^\s*\d{1,3}:\d{2}:\d{2}[,.]\d{3}\s*-->\s*\d{1,3}:\d{2}:\d{2}[,.]\d{3}")
        .expect("SRT timecode pattern");
    if text.trim_start_matches(['\u{feff}', ' ', '\t', '\r', '\n']).starts_with('<') || !cue.is_match(&text) {
        return Err(i18n::bilingual("SubtitleCat did not return a readable SRT file", "SubtitleCat 返回的文件无法识别为 SRT 字幕").into_owned());
    }
    tempfile::Builder::new().prefix("MediaLite-SubtitleCat-").suffix(".srt").tempfile()
        .and_then(|mut file| {
            file.write_all(&bytes)?;
            Ok(file.into_temp_path())
        })
        .map_err(|_| i18n::bilingual("Could not save the subtitle to the temporary folder", "无法将字幕保存到临时文件夹").into_owned())
}

fn read_page(url: &Url) -> Result<Html, String> {
    let request = api::public_https_client()?.get(url.clone()).header(reqwest::header::ACCEPT, "text/html");
    let bytes = api::read_bounded(request, 4 * 1024 * 1024)?;
    Ok(Html::parse_document(&String::from_utf8_lossy(&bytes)))
}

fn site_link(page: &Url, link: &str, extension: &str) -> Option<Url> {
    let mut url = page.join(link).ok()?;
    if url.scheme() != "https" || url.host_str() != Some(HOST) || url.port().is_some()
        || !url.username().is_empty() || url.password().is_some()
        || !url.path().starts_with("/subs/") || !url.path().ends_with(extension)
    {
        return None;
    }
    url.set_fragment(None);
    Some(url)
}

fn text(element: ElementRef<'_>) -> String {
    element.text().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_the_search_and_resolves_site_links() {
        let url = search_url(" 星際 & S01E02 ");
        assert_eq!(url.query_pairs().collect::<Vec<_>>(), vec![("search".into(), "星際 & S01E02".into())]);
        let result = site_link(&url, "subs/10/Example Name.html", ".html").unwrap();
        assert_eq!(result.as_str(), "https://www.subtitlecat.com/subs/10/Example%20Name.html");
        assert!(site_link(&result, "/subs/10/Example Name-zh-CN.srt", ".srt").is_some());
        assert!(site_link(&result, "https://example.com/subs/10/other.srt", ".srt").is_none());
    }
}

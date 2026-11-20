use super::*;
use std::sync::mpsc::{self, Receiver, TryRecvError};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Source {
    Danmaku,
    SubtitleCat,
}

pub(super) struct SearchDialog {
    source: Source,
    query: String,
    episode: String,
    api_address: String,
    results: Results,
    languages: Option<Languages>,
    pending: Option<Receiver<Result<Reply, String>>>,
    error: Option<String>,
}

enum Results {
    Empty,
    Episodes(danmaku::Endpoint, danmaku::SearchResults),
    Subtitles(Vec<subtitlecat::SearchResult>),
}

struct Languages {
    result: subtitlecat::SearchResult,
    subtitles: Vec<subtitlecat::Subtitle>,
}

enum Pick {
    Search,
    Episode(u64),
    SubtitlePage(subtitlecat::SearchResult),
    Subtitle(subtitlecat::Subtitle),
    Back,
}

enum Reply {
    Results(Results),
    Languages(Languages),
    Danmaku(Vec<danmaku::Comment>),
    Subtitle(tempfile::TempPath, String),
}

impl SearchDialog {
    pub(super) fn new(source: Source, item: &MediaItem, api_address: &str) -> Self {
        let mut query = display_title(item);
        let mut episode = String::new();
        if let Some(index) = item.index.filter(|_| item.kind == "Episode") {
            if source == Source::SubtitleCat {
                let season = item.parent_index.unwrap_or(1);
                query.push_str(&format!(" S{season:02}E{index:02}"));
            } else {
                episode = index.to_string();
            }
        } else if source == Source::SubtitleCat && item.kind == "Movie" {
            if let Some(year) = item.year.filter(|year| *year > 0) {
                query.push_str(&format!(" {year}"));
            }
        }
        Self {
            source, query, episode, api_address: api_address.to_owned(),
            results: Results::Empty, languages: None, pending: None, error: None,
        }
    }

    fn request(&mut self, ctx: &egui::Context, work: impl FnOnce() -> Result<Reply, String> + Send + 'static) {
        let (send, receive) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = send.send(work());
            ctx.request_repaint();
        });
        self.pending = Some(receive);
        self.error = None;
    }

    fn show(&mut self, ctx: &egui::Context) -> (bool, Option<Pick>) {
        let tr = i18n::bilingual;
        let title = match self.source {
            Source::SubtitleCat => tr("Find subtitles · SubtitleCat", "搜索字幕 · SubtitleCat"),
            Source::Danmaku => tr("Find danmaku", "搜索弹幕"),
        };
        let mut open = true;
        let mut pick = None;
        let busy = self.pending.is_some();
        let width = (ctx.screen_rect().width() - 48.0).clamp(200.0, 560.0);
        let list_height = (ctx.screen_rect().height() * 0.39).clamp(100.0, 320.0);
        egui::Window::new(title).id(egui::Id::new("online-captions"))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO).default_width(width)
            .collapsible(false).resizable(false).open(&mut open)
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.add_enabled_ui(!busy, |ui| {
                    if self.source == Source::Danmaku {
                        ui.label(tr("Danmaku API address", "弹幕 API 地址"));
                        ui.add(egui::TextEdit::singleline(&mut self.api_address)
                            .desired_width(f32::INFINITY).hint_text("https://example.com/api/v2"));
                    }
                    ui.label(tr("Title", "影视名称"));
                    let title_input = ui.add(egui::TextEdit::singleline(&mut self.query).desired_width(f32::INFINITY));
                    let enter = !busy && title_input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.horizontal(|ui| {
                        if self.source == Source::Danmaku {
                            ui.label(tr("Episode (optional)", "集数（可空）"));
                            ui.add(egui::TextEdit::singleline(&mut self.episode).desired_width(80.0));
                        }
                        if ui.add_enabled(!self.query.trim().is_empty(), egui::Button::new(tr("Search", "搜索"))).clicked()
                            || (enter && !self.query.trim().is_empty())
                        {
                            pick = Some(Pick::Search);
                        }
                        if self.source == Source::SubtitleCat && !self.query.trim().is_empty() && self.languages.is_none()
                            && ui.button(tr("Open in browser", "在浏览器打开")).clicked()
                        {
                            ctx.open_url(egui::OpenUrl::new_tab(subtitlecat::search_url(&self.query).to_string()));
                        }
                    });
                });
                if busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(tr("Requesting data…", "正在请求…"));
                    });
                }
                if let Some(error) = &self.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }

                ui.add_enabled_ui(!busy, |ui| {
                    if let Some(page) = &self.languages {
                        ui.separator();
                        ui.label(RichText::new(&page.result.title).strong());
                        ui.horizontal(|ui| {
                            if ui.button(tr("Back to results", "返回结果")).clicked() {
                                pick = Some(Pick::Back);
                            }
                            if ui.button(tr("Open on SubtitleCat", "在 SubtitleCat 打开")).clicked() {
                                ctx.open_url(egui::OpenUrl::new_tab(page.result.page.to_string()));
                            }
                        });
                        if page.subtitles.is_empty() {
                            ui.label(tr("Choose a language and translate on SubtitleCat.", "可在 SubtitleCat 页面选择语种并翻译。"));
                        } else {
                            ScrollArea::vertical().id_source("subtitlecat-languages").max_height(list_height)
                                .show(ui, |ui| {
                                    for subtitle in &page.subtitles {
                                        ui.horizontal(|ui| {
                                            if ui.button(tr("Load", "加载")).clicked() {
                                                pick = Some(Pick::Subtitle(subtitle.clone()));
                                            }
                                            ui.label(&subtitle.language);
                                        });
                                    }
                                });
                        }
                        return;
                    }
                    match &self.results {
                        Results::Empty => {}
                        Results::Episodes(_, result) => {
                            ui.separator();
                            if result.has_more {
                                ui.label(tr("Enter a more specific title to see more matches.", "可用更具体的片名缩小匹配范围。"));
                            }
                            if result.episodes.is_empty() {
                                ui.label(tr("No matching episodes found", "未找到匹配的剧集"));
                            }
                            ScrollArea::vertical().id_source("danmaku-results").max_height(list_height)
                                .show(ui, |ui| {
                                    for episode in &result.episodes {
                                        let label = format!("{} · {}", episode.series, episode.title);
                                        if ui.add(egui::Button::new(label).wrap()).clicked() {
                                            pick = Some(Pick::Episode(episode.id));
                                        }
                                    }
                                });
                        }
                        Results::Subtitles(results) => {
                            ui.separator();
                            if results.is_empty() {
                                ui.label(tr("No subtitles found", "未找到字幕"));
                            }
                            ScrollArea::vertical().id_source("subtitlecat-results").max_height(list_height)
                                .show(ui, |ui| {
                                    for result in results {
                                        if ui.add(egui::Button::new(&result.title).wrap()).clicked() {
                                            pick = Some(Pick::SubtitlePage(result.clone()));
                                        }
                                    }
                                });
                        }
                    }
                });
            });
        (open, pick)
    }
}

impl AppLite {
    pub(super) fn draw_caption_search(&mut self, ctx: &egui::Context) {
        let Some(player) = &mut self.player else { return };
        let Some(mut dialog) = player.caption_search.take() else { return };
        let tr = i18n::bilingual;
        if let Some(receive) = &dialog.pending {
            match receive.try_recv() {
                Ok(reply) => {
                    dialog.pending = None;
                    match reply {
                        Ok(Reply::Results(results)) => dialog.results = results,
                        Ok(Reply::Languages(languages)) => dialog.languages = Some(languages),
                        Ok(Reply::Danmaku(comments)) if !comments.is_empty() => {
                            let label = format!("{} · {}", tr("Danmaku loaded", "弹幕已载入"), comments.len());
                            player.danmaku = comments;
                            player.show_danmaku = true;
                            player.last_activity = ctx.input(|i| i.time);
                            player.notice = Some((label, player.last_activity + 4.0));
                            return;
                        }
                        Ok(Reply::Danmaku(_)) => {
                            dialog.error = Some(tr("No displayable danmaku for this episode", "此集没有可显示的弹幕").into_owned());
                        }
                        Ok(Reply::Subtitle(path, language)) => {
                            let title = format!("SubtitleCat · {language}");
                            if player.engine.as_ref().is_some_and(|mpv| mpv.add_subtitle(&path.to_string_lossy(), Some(&title))) {
                                player.downloaded_subtitles.push(path);
                                player.last_activity = ctx.input(|i| i.time);
                                return;
                            }
                            dialog.error = Some(i18n::t("Could not load subtitle file").into_owned());
                        }
                        Err(error) => dialog.error = Some(error),
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    dialog.pending = None;
                    dialog.error = Some(tr("The request ended. Try again.", "请求已结束，请重试。").into_owned());
                }
            }
        }
        let (open, pick) = dialog.show(ctx);
        if !open {
            return;
        }
        let mut save_address = false;
        match pick {
            Some(Pick::Search) => {
                let query = dialog.query.trim().to_owned();
                if dialog.source == Source::SubtitleCat {
                    dialog.languages = None;
                    dialog.results = Results::Empty;
                    dialog.request(ctx, move || subtitlecat::search(&query)
                        .map(|results| Reply::Results(Results::Subtitles(results))));
                } else if let Some(endpoint) = danmaku::Endpoint::parse(&dialog.api_address) {
                    let address = dialog.api_address.trim();
                    save_address = self.prefs.danmaku.api_url != address;
                    if save_address {
                        self.prefs.danmaku.api_url = address.to_owned();
                    }
                    let episode = dialog.episode.trim().to_owned();
                    dialog.results = Results::Empty;
                    dialog.request(ctx, move || {
                        let results = endpoint.search(&query, &episode)?;
                        Ok(Reply::Results(Results::Episodes(endpoint, results)))
                    });
                } else {
                    dialog.error = Some(tr("Enter a valid HTTP or HTTPS danmaku API address", "请输入有效的 HTTP 或 HTTPS 弹幕 API 地址").into_owned());
                }
            }
            Some(Pick::Episode(id)) => {
                if let Results::Episodes(endpoint, _) = &dialog.results {
                    let endpoint = endpoint.clone();
                    let rules = player.danmaku_filter.clone();
                    dialog.request(ctx, move || endpoint.comments(id, &rules).map(Reply::Danmaku));
                }
            }
            Some(Pick::SubtitlePage(result)) => {
                dialog.request(ctx, move || {
                    let subtitles = subtitlecat::subtitles(&result)?;
                    Ok(Reply::Languages(Languages { result, subtitles }))
                });
            }
            Some(Pick::Subtitle(subtitle)) => {
                if let Some(page) = &dialog.languages {
                    let result = page.result.clone();
                    dialog.request(ctx, move || {
                        let path = subtitlecat::download(&result, &subtitle)?;
                        Ok(Reply::Subtitle(path, subtitle.language))
                    });
                }
            }
            Some(Pick::Back) => {
                dialog.languages = None;
                dialog.error = None;
            }
            None => {}
        }
        player.caption_search = Some(dialog);
        if save_address {
            self.save();
        }
    }
}

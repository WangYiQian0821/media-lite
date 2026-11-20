use super::*;
use crate::preferences::{AssStyle, ChapterMode, Decoder, IntroMode, Network, ProxyMode, WebDav};
use std::ops::RangeInclusive;

type Translation = (&'static str, &'static str);

fn tr(text: Translation) -> std::borrow::Cow<'static, str> {
    i18n::bilingual(text.0, text.1)
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
enum Tab {
    #[default]
    General,
    Library,
    Backup,
    Sync,
    Network,
    Interaction,
    Playback,
    Cache,
    Subtitles,
    External,
    Danmaku,
    About,
}

impl Tab {
    fn label(self) -> Translation {
        match self {
            Self::General => ("General", "常规"),
            Self::Library => ("Library", "媒体库"),
            Self::Backup => ("Backup & restore", "备份与还原"),
            Self::Sync => ("Sync", "同步"),
            Self::Network => ("Network", "网络"),
            Self::Interaction => ("Defaults & interaction", "默认与交互"),
            Self::Playback => ("Playback", "播放配置"),
            Self::Cache => ("Playback cache", "播放缓存"),
            Self::Subtitles => ("Subtitle style", "字幕样式"),
            Self::External => ("External player", "外部播放器"),
            Self::Danmaku => ("Danmaku", "弹幕"),
            Self::About => ("Media Lite", "Media Lite"),
        }
    }
}

#[derive(Clone)]
pub(super) enum Command {
    Edit(Editor),
    Backup,
    EditWebDav,
    TransferWebDav(bool),
    ChooseRestore,
    Restore(Box<Backup>),
    ClearData,
    ClearImages,
    ClearPlayback,
    ChooseCacheDirectory,
    ChooseExternal(PlayerChoice),
    EditMpv,
    OpenLogs,
    OpenData,
    CheckForUpdate,
    OpenLink(String),
    ResetSubtitles,
    ClearSearchHistory,
}

#[derive(Clone, Copy)]
pub(super) enum Editor { Recommend, PreferredVersion, BlockedWords, Proxy, DanmakuApi }

impl Editor {
    fn label(self) -> Translation {
        match self {
            Self::Recommend => ("Daily recommendation filter", "每日推荐过滤"),
            Self::PreferredVersion => ("Preferred versions", "首选版本"),
            Self::BlockedWords => ("Blocked danmaku words", "弹幕屏蔽词"),
            Self::Proxy => ("HTTP proxy address", "HTTP 代理地址"),
            Self::DanmakuApi => ("Custom danmaku API", "自定义弹幕 API"),
        }
    }

    fn value(self, prefs: &mut Preferences) -> &mut String {
        match self {
            Self::Recommend => &mut prefs.library.recommend_filter,
            Self::PreferredVersion => &mut prefs.playback.preferred_version,
            Self::BlockedWords => &mut prefs.danmaku.blocked_words,
            Self::Proxy => &mut prefs.network.proxy_address,
            Self::DanmakuApi => &mut prefs.danmaku.api_url,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Backup {
    version: u32,
    config: Config,
}

impl Backup {
    fn decode(bytes: &[u8]) -> Option<Self> {
        let backup = serde_json::from_slice::<Self>(bytes).ok()?;
        (backup.version == 1 && backup.config.servers.iter().all(|server|
            server.sample || reqwest::Url::parse(&api::normalize_url(&server.url)).is_ok_and(|url|
                matches!(url.scheme(), "http" | "https") && url.host_str().is_some()
            ))).then_some(backup)
    }
}

pub(super) enum Dialog {
    Edit { field: Editor, draft: String, error: String },
    Restore(Box<Backup>),
    WebDav { draft: WebDav, password: String, error: bool },
}

pub(super) struct SettingsView {
    tab: Tab,
    pub back: View,
    pub dialog: Option<Dialog>,
    notice: Option<(String, bool)>,
    config_error: Option<String>,
    dirty: bool,
    last_stats_at: f64,
    sizes: [u64; 3],
    webdav_password: String,
}

impl Default for SettingsView {
    fn default() -> Self {
        Self {
            tab: Tab::General, back: View::Home, dialog: None, notice: None, config_error: None, dirty: false,
            last_stats_at: f64::NEG_INFINITY, sizes: [0; 3], webdav_password: String::new(),
        }
    }
}

impl SettingsView {
    pub(super) fn report_config_error(&mut self, detail: String) {
        self.config_error = Some(detail);
    }
}

#[derive(Clone, Copy)]
struct Colors {
    bg: Color32,
    card: Color32,
    border: Color32,
    line: Color32,
    selection: Color32,
    control: Color32,
    text: Color32,
    help: Color32,
    muted: Color32,
}

impl Colors {
    fn new(p: &Palette) -> Self {
        let rgb = Color32::from_rgb;
        if p.dark {
            Self {
                bg: rgb(11, 17, 24), card: rgb(20, 25, 32), border: rgb(44, 48, 55),
                line: rgb(36, 40, 47), selection: rgb(53, 58, 65), control: rgb(38, 43, 49),
                text: rgb(245, 247, 250), help: rgb(187, 193, 201), muted: rgb(144, 154, 166),
            }
        } else {
            Self {
                bg: p.bg, card: p.panel, border: p.line, line: p.line,
                selection: p.card_hover, control: p.card, text: p.text, help: p.dim, muted: p.dim,
            }
        }
    }
}

fn group(ui: &mut Ui, colors: Colors, add: impl FnOnce(&mut Rows<'_>)) {
    let width = ui.available_width();
    egui::Frame::none().fill(colors.card).stroke(Stroke::new(1.0_f32, colors.border))
        .rounding(14.0)
        .show(ui, |ui| {
            ui.set_min_width(width - 2.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            add(&mut Rows { ui, colors, first: true });
        });
    ui.add_space(12.0);
}

struct Rows<'a> {
    ui: &'a mut Ui,
    colors: Colors,
    first: bool,
}

impl Rows<'_> {
    fn row(&mut self, icon: &str, name: Translation, hint: &str, control: f32) -> (Rect, Rect) {
        let name = tr(name);
        let painter = self.ui.painter();
        let width = self.ui.available_width();
        let description_width = (width - 89.0 - control).max(125.0);
        let detail = (!hint.is_empty()).then(|| {
            let mut job = egui::text::LayoutJob::simple(hint.to_owned(), FontId::proportional(13.0),
                self.colors.help, description_width);
            job.wrap.max_rows = 2;
            painter.layout_job(job)
        });
        let height = if detail.as_ref().is_some_and(|g| g.rows.len() > 1) { 88.0 }
            else if detail.is_some() { 75.0 } else { 64.0 };
        let (rect, _) = self.ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        let painter = self.ui.painter();
        if !self.first {
            painter.hline(rect.x_range(), rect.top(), Stroke::new(1.0_f32, self.colors.line));
        }
        self.first = false;
        let midpoint = rect.center().y;
        let icon_center = Pos2::new(rect.left() + 36.0, midpoint);
        if icon == "list-plus" {
            for (y, width) in [(-5.0, 12.0), (-1.0, 9.0), (3.0, 5.5)] {
                let start = icon_center + Vec2::new(-8.0, y);
                painter.line_segment([start, start + Vec2::new(width, 0.0)], Stroke::new(1.65_f32, self.colors.help));
            }
            theme::icon(self.ui, "plus", icon_center + Vec2::new(4.0, 3.0), 12.0, self.colors.help);
        } else if matches!(icon, "文" | "A" | "B" | "♪" | "↕") {
            painter.text(icon_center, Align2::CENTER_CENTER, icon, FontId::proportional(17.0), self.colors.help);
        } else {
            let symbol = match icon {
                "⌂" => "home", "▦" => "library", "▷" => "play", "☆" => "star",
                "⚙" | "⚡" => "gear", "⇄" | "↶" | "◷" => "refresh",
                "⌕" => "search", "›" | "»" => "chevron-right", "‹" => "chevron-left",
                "×" => "close", "↑" | "↓" | "⇡" | "⇣" => "jump",
                "▣" | "▧" | "⤢" | "◫" => "maximize", "◇" => "check",
                "◐" | "♢" | "◉" | "•" | "ⓘ" => "dot", _ => "list",
            };
            theme::icon(self.ui, symbol, icon_center, 19.0, self.colors.help);
        }
        let x = rect.left() + 67.0;
        let mut job = egui::text::LayoutJob::simple(name.into_owned(), FontId::proportional(15.0),
            self.colors.text, description_width);
        job.wrap.max_rows = 1;
        let title = painter.layout_job(job);
        let offset = detail.as_ref().map_or(0.0, |g| (g.size().y + 4.0) / 2.0);
        let title_top = midpoint - title.size().y / 2.0 - offset;
        let bottom = title_top + title.size().y;
        painter.galley(Pos2::new(x, title_top), title, self.colors.text);
        if let Some(galley) = detail {
            painter.galley(Pos2::new(x, bottom + 4.0), galley, self.colors.help);
        }
        (rect, Rect::from_min_size(Pos2::new(rect.right() - 18.0 - control, midpoint - 17.0),
            Vec2::new(control, 34.0)))
    }

    fn switch(&mut self, icon: &str, name: Translation, hint: Translation, value: &mut bool) {
        let (row, control) = self.row(icon, name, &tr(hint), 42.0);
        let track = Rect::from_center_size(control.center(), Vec2::new(42.0, 24.0));
        let response = self.ui.interact(row, self.ui.id().with(("switch", name.0)), Sense::click());
        if response.clicked() {
            *value = !*value;
        }
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, *value, tr(name)));
        let painter = self.ui.painter();
        let filled = if *value { self.colors.text } else { self.colors.control };
        let knob = if *value { self.colors.bg } else { self.colors.text };
        painter.rect(track, 12.0, filled, Stroke::new(1.0_f32, self.colors.border));
        painter.circle_filled(Pos2::new(if *value { track.right() - 12.0 } else { track.left() + 12.0 },
            track.center().y), 8.0, knob);
        response.on_hover_cursor(egui::CursorIcon::PointingHand);
    }

    fn pick<T: Clone + PartialEq>(&mut self, icon: &str, name: Translation, hint: Translation,
        value: &mut T, options: &[(T, Translation)])
    {
        let (_, control) = self.row(icon, name, &tr(hint), 190.0);
        let text = options.iter().find(|(option, _)| option == value).or_else(|| options.first())
            .map_or(std::borrow::Cow::Borrowed(""), |(_, label)| tr(*label));
        let mut child = self.ui.child_ui_with_id_source(control, Layout::top_down(Align::Min), name.0, None);
        child.spacing_mut().interact_size.y = 32.0;
        child.visuals_mut().widgets.inactive.bg_fill = self.colors.control;
        child.visuals_mut().widgets.inactive.weak_bg_fill = self.colors.control;
        child.visuals_mut().widgets.inactive.rounding = egui::Rounding::same(16.0);
        child.visuals_mut().override_text_color = Some(self.colors.text);
        egui::ComboBox::from_id_source(name.0).width(control.width()).selected_text(text)
            .show_ui(&mut child, |ui| {
                for (option, label) in options {
                    ui.selectable_value(value, option.clone(), tr(*label));
                }
            });
    }

    fn level<T: egui::emath::Numeric + Copy>(&mut self, icon: &str, name: Translation, hint: Translation,
        value: &mut T, range: RangeInclusive<T>, step: f64, unit: &str)
    {
        let (row, control) = self.row(icon, name, &tr(hint), 219.0);
        let left = control.left() + 2.0;
        let right = left + 137.0;
        let center = row.center().y;
        let area = Rect::from_min_max(Pos2::new(left - 8.0, center - 14.0), Pos2::new(right + 8.0, center + 14.0));
        let response = self.ui.interact(area, self.ui.id().with(("slider", name.0)), Sense::click_and_drag());
        let low = range.start().to_f64();
        let high = range.end().to_f64();
        if let Some(pointer) = response.interact_pointer_pos() {
            if response.clicked() || response.dragged() || response.drag_stopped() {
                let target = f64::from(((pointer.x - left) / (right - left)).clamp(0.0, 1.0)) * (high - low) + low;
                *value = T::from_f64(((target / step).round() * step).clamp(low, high));
            }
        }
        if response.has_focus() {
            let delta = self.ui.input(|i| i.key_pressed(egui::Key::ArrowRight) as i8 - i.key_pressed(egui::Key::ArrowLeft) as i8);
            if delta != 0 {
                *value = T::from_f64((value.to_f64() + f64::from(delta) * step).clamp(low, high));
            }
        }
        let val = value.to_f64();
        let dot = left + (right - left) * ((val - low) / (high - low)) as f32;
        let painter = self.ui.painter();
        let track = Rect::from_min_max(Pos2::new(left, center - 2.5), Pos2::new(right, center + 2.5));
        painter.rect_filled(track, 2.5, self.colors.selection);
        painter.rect_filled(track.with_max_x(dot), 2.5, self.colors.text);
        painter.circle_filled(Pos2::new(dot, center), 8.0, self.colors.text);
        let precision = if step >= 1.0 { 0 } else if step >= 0.1 { 1 } else { 2 };
        let caption = format!("{val:.precision$}{unit}");
        painter.text(control.right_center(), Align2::RIGHT_CENTER, &caption, FontId::proportional(13.0), self.colors.text);
        response.widget_info(|| egui::WidgetInfo::slider(true, val, tr(name)));
        response.on_hover_cursor(egui::CursorIcon::PointingHand);
    }

    fn command(&mut self, icon: &str, name: Translation, hint: &str, button: Option<Translation>) -> bool {
        let (row, control) = self.row(icon, name, hint, if button.is_some() { 132.0 } else { 32.0 });
        if let Some(button) = button {
            let caption = tr(button);
            let widget = egui::Button::new(RichText::new(caption).size(13.0).color(self.colors.text))
                .fill(self.colors.control).rounding(16.0).stroke(Stroke::new(1.0_f32, self.colors.border));
            let mut child = self.ui.child_ui_with_id_source(control, Layout::top_down(Align::Min), name.0, None);
            return child.put(control, widget).clicked();
        }
        let response = self.ui.interact(row, self.ui.id().with(("action", name.0)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        self.ui.painter().text(control.right_center() - Vec2::new(4.0, 0.0), Align2::RIGHT_CENTER,
            "›", FontId::proportional(21.0), self.colors.text);
        response.clicked()
    }

    fn color(&mut self, name: Translation, value: &mut u32) {
        let (_, control) = self.row("◉", name, "", 145.0);
        let [a, r, g, b] = value.to_be_bytes();
        let mut rgba = Color32::from_rgba_unmultiplied(r, g, b, a);
        let mut child = self.ui.child_ui_with_id_source(control, Layout::right_to_left(Align::Center),
            name.0, None);
        if egui::color_picker::color_edit_button_srgba(&mut child, &mut rgba, egui::color_picker::Alpha::BlendOrAdditive).changed() {
            let [r, g, b, a] = rgba.to_srgba_unmultiplied();
            *value = u32::from_be_bytes([a, r, g, b]);
        }
        child.label(RichText::new(format!("#{:08X}", *value)).size(13.0).color(self.colors.text));
    }
}

const EMPTY: Translation = ("", "");
const CHOOSE: Translation = ("Choose file", "选择文件");
const CLEAR: Translation = ("Clear cache", "清空缓存");
const LANGUAGES: &[(&str, Translation)] = &[
    ("", ("Default", "默认")), ("no", ("Off", "关闭")), ("zho,chi,zh,zht,zh-hans,zh-hant", ("Chinese", "中文")),
    ("eng,en", ("English", "英语")), ("jpn,ja", ("Japanese", "日语")), ("kor,ko", ("Korean", "韩语")),
    ("fra,fre,fr", ("French", "法语")), ("deu,ger,de", ("German", "德语")),
    ("spa,es", ("Spanish", "西班牙语")), ("rus,ru", ("Russian", "俄语")),
];
const ASS_MODES: &[(AssStyle, Translation)] = &[
    (AssStyle::No, ("No", "No")), (AssStyle::Yes, ("Yes", "Yes")),
    (AssStyle::Scale, ("Scale", "Scale")), (AssStyle::Force, ("Force", "Force")),
    (AssStyle::Strip, ("Strip", "Strip")),
];

fn size_label(bytes: u64) -> String {
    if bytes >= 1_048_576 { format!("{:.1} MB", bytes as f64 / 1_048_576.0) }
    else if bytes >= 1024 { format!("{:.1} KB", bytes as f64 / 1024.0) }
    else { format!("{bytes} B") }
}

impl AppLite {
    pub(super) fn draw_settings(&mut self, ui: &mut Ui, p: &Palette, actions: &mut Vec<Action>) {
        let colors = Colors::new(p);
        let full = ui.max_rect();
        ui.painter().rect_filled(full, 0.0, colors.bg);
        let back = Rect::from_min_size(full.min + Vec2::new(24.0, 12.0), Vec2::splat(36.0));
        let chrome = Rect::from_min_size(Pos2::new(full.right() - 162.0, full.top() + 12.0), Vec2::new(138.0, 36.0));
        let drag = Rect::from_min_max(back.right_top(), chrome.left_bottom());
        let response = ui.interact(drag, ui.id().with("settings-window-drag"), Sense::click_and_drag());
        let maximized = ui.input(|i| i.viewport().maximized == Some(true));
        if response.double_clicked() { ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized)); }
        else if response.drag_started() { ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag); }
        if detail_icon_button(ui, back, "back-arrow", "Back", false, colors.card).clicked() {
            actions.push(Action::BackFromSettings);
        }
        ui.painter().text(Pos2::new(back.right() + 12.0, back.center().y),
            Align2::LEFT_CENTER, tr(("Settings", "设置")), FontId::new(17.0, egui::FontFamily::Name("home-display".into())), colors.text);
        ui.painter().rect(chrome, 18.0, colors.card, Stroke::new(1.0_f32, colors.border));
        for (index, (label, glyph, command)) in [
            (("Minimize", "最小化"), "−", egui::ViewportCommand::Minimized(true)),
            (if maximized { ("Restore", "还原") } else { ("Maximize", "最大化") },
             if maximized { "❐" } else { "□" }, egui::ViewportCommand::Maximized(!maximized)),
            (("Close", "关闭"), "×", egui::ViewportCommand::Close),
        ].into_iter().enumerate() {
            let rect = Rect::from_center_size(chrome.left_center() + Vec2::new(24.0 + index as f32 * 44.0, 0.0), Vec2::new(40.0, 34.0));
            let response = ui.interact(rect, ui.id().with(("settings-chrome", index)), Sense::click());
            if response.hovered() {
                ui.painter().rect_filled(rect, 16.0, colors.selection);
            }
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(15.0), colors.text);
            if response.on_hover_text(tr(label)).clicked() { ui.ctx().send_viewport_cmd(command); }
        }

        let area = Rect::from_min_max(full.min + Vec2::new(36.0, 70.0), Pos2::new(full.left()+236.0, full.bottom()-12.0));
        let mut rail = ui.child_ui_with_id_source(area, Layout::top_down(Align::Min), "settings-rail", None);
        rail.set_clip_rect(area);
        ScrollArea::vertical().id_source("settings-rail-scroll").auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(&mut rail, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (group, tabs) in [
                    (("General", "通用"), &[Tab::General, Tab::Library, Tab::Backup, Tab::Sync, Tab::Network][..]),
                    (("Player", "播放器"), &[Tab::Interaction, Tab::Playback, Tab::Cache, Tab::Subtitles, Tab::External, Tab::Danmaku][..]),
                    (("About", "关于"), &[Tab::About][..]),
                ] {
                    let (label, _) = ui.allocate_exact_size(Vec2::new(200.0, 30.0), Sense::hover());
                    ui.painter().text(label.left_center() + Vec2::new(12.0, -2.0), Align2::LEFT_CENTER,
                        tr(group), FontId::proportional(12.0), colors.muted);
                    for tab in tabs {
                        let (rect, response) = ui.push_id(tab, |ui| ui.allocate_exact_size(Vec2::new(200.0, 36.0), Sense::click())).inner;
                        let active = self.settings_view.tab == *tab;
                        if active || response.hovered() {
                            ui.painter().rect(rect, 18.0, colors.selection, Stroke::new(1.0_f32, colors.border));
                        }
                        let text = tr(tab.label());
                        ui.painter().text(rect.left_center() + Vec2::new(18.0, 0.0), Align2::LEFT_CENTER,
                            &text, FontId::proportional(14.0), colors.text);
                        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, active, text.as_ref()));
                        if response.clicked() {
                            self.settings_view.tab = *tab;
                            self.settings_view.notice = None;
                        }
                    }
                    ui.add_space(19.0);
                }
            });

        let pane_left = full.left() + 260.0;
        let tab = self.settings_view.tab;
        ui.painter().text(Pos2::new(pane_left, full.top() + 76.0), Align2::LEFT_TOP, tr(tab.label()),
            FontId::new(26.0, egui::FontFamily::Name("home-display".into())), colors.text);
        let area = Rect::from_min_max(Pos2::new(pane_left, full.top() + 118.0), full.right_bottom() - Vec2::new(36.0, 18.0));
        let mut pane = ui.child_ui_with_id_source(area, Layout::top_down(Align::Min), ("settings-pane", tab), None);
        pane.set_clip_rect(area);
        let before = self.prefs.clone();
        if matches!(tab, Tab::Library | Tab::Cache) {
            let now = ui.input(|i| i.time);
            if now - self.settings_view.last_stats_at > 1.0 {
                self.settings_view.last_stats_at = now;
                self.settings_view.sizes = [
                    crate::net::response_cache_size(), crate::net::cache_usage().1,
                    crate::net::directory_usage(&self.prefs.cache.directory()).1,
                ];
            }
        }
        ScrollArea::vertical().id_source(tab).auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(&mut pane, |ui| self.settings_rows(ui, colors, tab, actions));
        self.draw_setting_dialog(ui.ctx(), actions);
        self.finish_preference_changes(&before);
        if self.settings_view.dirty && !ui.input(|i| i.pointer.any_down()) {
            self.write_settings();
        }
        let status = self.settings_view.config_error.as_ref().map(|detail| {
            (format!("{} · {detail}", tr(("Configuration read or write failed", "配置读写失败"))), true)
        }).or_else(|| self.settings_view.notice.clone());
        if let Some((message, failed)) = status {
            let color = if failed { Color32::LIGHT_RED } else { colors.help };
            let line = theme::clip_px(ui.painter(), &message, 13.0, (full.right() - pane_left - 210.0).max(80.0), color);
            let at = Pos2::new(full.right() - 36.0, full.top() + 92.0);
            let rect = ui.painter().text(at, Align2::RIGHT_CENTER, line, FontId::proportional(13.0), color);
            ui.interact(rect, ui.id().with("settings-status"), Sense::hover()).on_hover_text(message);
        }
    }

    fn settings_rows(&mut self, ui: &mut Ui, colors: Colors, tab: Tab, actions: &mut Vec<Action>) {
        let mut emit = |cmd| actions.push(Action::Settings(cmd));
        let prefs = &mut self.prefs;
        match tab {
            Tab::General => {
                group(ui, colors, |r| {
                    r.pick("文", ("Language", "语言"), EMPTY, &mut prefs.language, &[
                        ("auto".into(), ("Automatic", "自动")), ("zh".into(), ("简体中文", "简体中文")),
                        ("zh-Hant".into(), ("繁體中文", "繁體中文")), ("en".into(), ("English", "English")),
                    ]);
                    r.pick("◐", ("Theme mode", "主题模式"), ("Change the current theme", "修改当前主题模式"),
                        &mut prefs.theme, &[(ThemeMode::System, ("Automatic", "自动")),
                            (ThemeMode::Light, ("Light", "明亮")), (ThemeMode::Dark, ("Dark", "黑暗"))]);
                    r.pick("⌂", ("Open on startup", "App 启动时打开"), EMPTY, &mut prefs.start_aggregate,
                        &[(false, ("Home", "首页")), (true, ("Aggregate", "聚合视界"))]);
                });
                group(ui, colors, |r| r.switch("▣", ("Minimize to tray on close", "关闭时最小化到托盘"),
                    ("Keep the app in the system tray when the window closes", "关闭主界面时将程序最小化到系统托盘"), &mut prefs.close_to_tray));
            }
            Tab::Library => group(ui, colors, |r| {
                let library = &mut prefs.library;
                r.switch("▦", ("Hide libraries", "隐藏媒体库"), ("Hide library rows on Home", "开启后首页将不再显示各媒体库"), &mut library.hide_library);
                r.switch("▷", ("Hide continue watching", "隐藏继续观看"), ("Hide Continue Watching on Home", "开启后首页不再显示继续观看"), &mut library.hide_resume);
                r.switch("♢", ("Hide daily recommendations", "隐藏每日推荐"), ("Hide the featured carousel on Home", "开启后首页不再显示每日推荐"), &mut library.hide_recommend);
                r.switch("▤", ("Hide unwatched counts", "隐藏封面未观看集数"), ("Hide unwatched episode counts on covers", "开启后封面不再显示未观看集数"), &mut library.hide_unplayed);
                if r.command("≡", Editor::Recommend.label(), &tr(("Exclude ratings and genres from the carousel", "按分级或类型过滤每日推荐，一行一项")), None) { emit(Command::Edit(Editor::Recommend)); }
                r.switch("☆", ("Show cover ratings", "显示封面评分"), ("Show ratings on poster covers", "在封面右下角显示评分"), &mut library.show_rating);
                r.switch("▧", ("Hide details album", "隐藏详情页相册"), ("Hide the backdrop album on the details page", "开启后详情页不再显示相册"), &mut library.hide_album);
                r.switch("⌕", ("Include media data in search", "搜索时包含媒体数据"), ("Show available DoVi, HDR, resolution and bitrate tags", "显示服务器返回的 DoVi、HDR、分辨率和比特率"), &mut library.search_media);
                r.switch("↶", ("Enable search history", "开启搜索历史记录"), ("Remember and show recent search terms", "记录并显示最近搜索词"), &mut library.search_history);
                if library.search_history && !self.search_history.is_empty() &&
                    r.command("×", ("Clear search history", "清空搜索历史"), "", Some(("Clear", "清空")))
                { emit(Command::ClearSearchHistory); }
                r.level("⤢", ("Cover scale", "封面缩放倍率"), ("Change the poster cover scale", "修改海报封面缩放比例"), &mut library.cover_scale, 0.7..=1.5, 0.05, "x");
                r.pick("▧", ("Image quality", "图片质量"), EMPTY, &mut library.quality, &[
                    (Quality::Low, ("Low", "低")), (Quality::Medium, ("Medium", "中")), (Quality::High, ("High", "高")),
                ]);
                r.pick("☷", ("Chapters in details", "详情页章节"), EMPTY, &mut library.chapters, &[
                    (ChapterMode::Always, ("Always show", "总是显示")), (ChapterMode::Movies, ("Movies only", "仅电影")),
                    (ChapterMode::Never, ("Never show", "不显示")),
                ]);
                r.switch("◫", ("Response cache", "数据缓存"), ("Show saved Home data while the server is loading", "请求返回前先展示首页缓存"), &mut library.data_cache);
                let days = [(1, ("1 day", "1 天")), (7, ("7 days", "7 天")), (15, ("15 days", "15 天")), (30, ("30 days", "30 天"))];
                r.pick("◷", ("Response cache expiration", "数据缓存过期时间"), EMPTY, &mut library.data_days, &days);
                r.pick("◷", ("Image cache expiration", "图片缓存过期时间"), EMPTY, &mut library.image_days, &days);
                if r.command("◫", ("Response cache", "数据缓存"), &size_label(self.settings_view.sizes[0]), Some(CLEAR)) { emit(Command::ClearData); }
                if r.command("▧", ("Image cache", "图片缓存"), &size_label(self.settings_view.sizes[1]), Some(CLEAR)) { emit(Command::ClearImages); }
            }),
            Tab::Backup => group(ui, colors, |r| {
                if r.command("⇄", ("WebDAV settings", "WebDAV 配置"),
                    &tr(("Set the address and remote backup file", "修改 WebDAV 地址和远程备份路径")), None)
                { emit(Command::EditWebDav); }
                r.switch("⚙", ("Back up settings", "备份设置"), ("Include local settings as well as saved servers", "同时备份本机设置"), &mut prefs.backup_settings);
                if r.command("↓", ("Back up to local file", "备份到本地"),
                    &tr(("Save the configuration to a JSON file. It includes server login tokens.", "将配置保存为 JSON，文件会包含服务器登录令牌。")), None) { emit(Command::Backup); }
                if r.command("↶", ("Restore from local file", "从本地还原"), &tr(("Select a Media Lite backup", "选择 Media Lite 的备份文件进行还原")), None) { emit(Command::ChooseRestore); }
                if r.command("↑", ("Back up to WebDAV", "备份到 WebDAV"), &tr(("Upload the backup to the chosen address", "将备份文件发送到已配置的 WebDAV 地址")), None) { emit(Command::TransferWebDav(true)); }
                if r.command("↓", ("Restore from WebDAV", "从 WebDAV 还原"), &tr(("Open the remote backup before confirming", "下载备份并确认后还原")), None) { emit(Command::TransferWebDav(false)); }
            }),
            Tab::Sync => group(ui, colors, |r| {
                r.level("list-plus", ("Mark as watched at", "标记为已观看进度"),
                    ("Mark the item as watched on the current media server when playback reaches this point",
                     "当播放进度达到设定值时，在当前媒体服务器上标记为已观看"),
                    &mut prefs.watched_percent, 0..=100, 1.0, "%");
            }),
            Tab::Network => group(ui, colors, |r| {
                r.switch("◇", ("Ignore SSL certificate errors", "忽略 SSL 证书校验"), ("Takes effect after restart", "重启后生效"), &mut prefs.network.ignore_ssl);
                r.pick("⇄", ("Network proxy", "网络代理"), ("Default connection for app requests and playback", "应用请求与播放的默认连接方式"),
                    &mut prefs.network.proxy, &[(ProxyMode::System, ("System proxy", "跟随系统")),
                        (ProxyMode::Http, ("HTTP proxy", "HTTP 代理")), (ProxyMode::Direct, ("No proxy", "不使用代理"))]);
                let value = if prefs.network.proxy_address.is_empty() { "http://127.0.0.1:10808" } else { &prefs.network.proxy_address };
                if r.command("⇄", Editor::Proxy.label(), value, None) { emit(Command::Edit(Editor::Proxy)); }
                if let Some(server) = self.active.and_then(|index| self.servers.get_mut(index))
                    .filter(|server| !server.sample && server.is_signed_in())
                {
                    let before = server.direct_playback;
                    r.switch("▷", ("Direct playback for current server", "当前服务器播放直连"),
                        ("Use a direct connection when the built-in player starts", "当前服务器的内置播放器下次播放时使用直连"),
                        &mut server.direct_playback);
                    self.settings_view.dirty |= server.direct_playback != before;
                }
            }),
            Tab::Interaction => group(ui, colors, |r| {
                let mut choices = vec![(PlayerChoice::BuiltIn, ("Built-in player", "内置播放器"))];
                if prefs.external.mpv { choices.push((PlayerChoice::Mpv, ("External mpv", "外部 mpv"))); }
                if prefs.external.potplayer { choices.push((PlayerChoice::PotPlayer, ("PotPlayer", "PotPlayer"))); }
                let play = &mut prefs.playback;
                r.pick("▷", ("Default player", "默认播放器"), ("Player to use when pressing Play", "点击播放时使用的播放器"), &mut play.default_player, &choices);
                r.level("›", ("Fast-forward interval", "快进时间"), ("Seek with the Right arrow key", "按下右方向键时的快进时间"), &mut play.forward, 1..=120, 1.0, "s");
                r.level("‹", ("Rewind interval", "快退时间"), ("Seek with the Left arrow key", "按下左方向键时的快退时间"), &mut play.rewind, 1..=120, 1.0, "s");
                r.level("»", ("Hold-to-speed playback rate", "倍速播放速率"), ("Playback rate while holding the Right arrow key", "长按右方向键时的倍速播放速率"), &mut play.hold_speed, 1.0..=4.0, 0.1, "x");
            }),
            Tab::Playback => group(ui, colors, |r| {
                let play = &mut prefs.playback;
                r.row("▣", ("Video output", "视频输出驱动"), &tr(("Built-in libmpv OpenGL", "内置 libmpv · OpenGL")), 0.0);
                r.pick("⚙", ("Decoder", "解码方式"), ("Default video decoding method", "修改默认解码方式"), &mut play.decoder, &[
                    (Decoder::Auto, ("Auto", "自动")), (Decoder::Software, ("Software", "软件解码")),
                    (Decoder::D3d11, ("D3D11VA (copy)", "D3D11VA (copy)")), (Decoder::Dxva2, ("DXVA2 (copy)", "DXVA2 (copy)")),
                ]);
                r.switch("⤢", ("Start in fullscreen", "播放时默认全屏"), ("Open the built-in player in fullscreen", "开启后内置播放器以全屏启动"), &mut play.fullscreen);
                r.switch("▣", ("Minimize during external playback", "外部播放时最小化主界面"), EMPTY, &mut prefs.external.minimize_main);
                let audio = LANGUAGES.iter().filter(|(key, _)| *key != "no").map(|(key, text)| ((*key).into(), *text)).collect::<Vec<_>>();
                let subtitles = LANGUAGES.iter().map(|(key, text)| ((*key).into(), *text)).collect::<Vec<_>>();
                r.pick("♪", ("Preferred audio language", "首选音频语言"), ("Prefer the selected audio language", "优先选中所选音频语言"), &mut play.audio_language, &audio);
                r.pick("▤", ("Preferred subtitle language", "首选字幕语言"), ("Prefer the selected subtitle language", "优先选中所选字幕语言"), &mut play.subtitle_language, &subtitles);
                r.switch("▤", ("Secondary subtitle", "次字幕"), ("Enable selection of a second subtitle in the player", "开启后可在播放器中选择第二条字幕"), &mut play.secondary);
                r.pick("▤", ("Preferred secondary subtitle language", "首选次字幕语言"), EMPTY, &mut play.secondary_language, &subtitles);
                if r.command("☷", Editor::PreferredVersion.label(), &tr(("Prefer matching source names, one per line", "按片源名优先选择，多个版本用换行分隔")), None) { emit(Command::Edit(Editor::PreferredVersion)); }
                r.pick("»", ("Skip intro when supported", "跳过片头（需要服务器支持）"), ("Uses the server's intro markers", "根据服务器返回的片头标记进行跳过"), &mut play.intro,
                    &[(IntroMode::Off, ("Off", "关闭")), (IntroMode::Button, ("Show skip button", "显示跳过按钮")), (IntroMode::Auto, ("Automatically skip", "自动跳过"))]);
                r.switch("▧", ("Progress thumbnail", "进度缩略图（需要服务器支持）"), ("Show the server's chapter image while hovering over the timeline", "在进度条上显示服务器提供的章节图片"), &mut play.preview);
                r.switch("•", ("Show chapter markers", "显示章节标记"), ("Show available chapters on the timeline", "在播放器进度条上显示章节标记"), &mut play.chapters);
                r.switch("⤢", ("Remember aspect mode", "记忆缩放模式"), ("Restore the last fit or fill mode", "恢复上次画面适应或填满模式"), &mut play.remember_aspect);
                r.switch("⚡", ("Fast decoding", "低质量视频解码"), ("Trade decoding quality for performance", "以降低解码质量换取性能"), &mut play.fast_decode);
                r.switch("♪", ("Force stereo audio", "强制输出立体声"), ("Downmix multichannel audio to stereo", "将多声道音轨混合为双声道输出"), &mut play.stereo);
                r.switch("▧", ("Stretch bitmap subtitles", "强制拉伸图形字幕到播放分辨率"), ("Stretch PGS and other bitmap subtitles with the video", "在比例不一致时拉伸 PGS 等图形字幕"), &mut play.stretch_subs);
                if r.command("⚙", ("Edit mpv.conf", "编辑 mpv.conf"), &tr(("Customize mpv settings", "自定义 mpv 配置")), Some(("Open in editor", "在编辑器中打开"))) { emit(Command::EditMpv); }
                r.switch("☷", ("Record player log", "开启播放器日志记录"), ("Write libmpv diagnostics and track changes when opening a video", "下次播放时记录 libmpv 诊断信息和切轨事件"), &mut play.log);
                if r.command("▣", ("Player log directory", "播放器日志路径"), &mpv_logs().to_string_lossy(), Some(("Open folder", "打开文件夹"))) { emit(Command::OpenLogs); }
            }),
            Tab::Cache => group(ui, colors, |r| {
                let cache = &mut prefs.cache;
                let hint = ("Read-ahead stops when it reaches either limit", "达到大小或时长上限后停止继续预读");
                r.level("◫", ("Maximum buffer size", "最大缓存大小"), hint, &mut cache.megabytes, 16..=2048, 1.0, " MB");
                r.level("◷", ("Maximum buffer duration", "最大缓存时长"), hint, &mut cache.seconds, 5..=900, 1.0, " s");
                r.level("◫", ("Maximum temporary disk cache", "最大临时磁盘缓存"), ("Use temporary disk cache for sources no larger than this limit. 0 turns it off.", "仅对未超过上限的已知大小片源使用临时磁盘缓存，0 表示关闭。"),
                    &mut cache.disk_gb, 0..=64, 1.0, " GB");
                if r.command("▣", ("Disk cache location", "磁盘缓存目录"), &cache.directory().to_string_lossy(), Some(("Choose folder", "选择文件夹"))) { emit(Command::ChooseCacheDirectory); }
                if r.command("◫", ("Disk cache", "磁盘缓存"), &size_label(self.settings_view.sizes[2]), Some(CLEAR)) { emit(Command::ClearPlayback); }
            }),
            Tab::Subtitles => group(ui, colors, |r| {
                let sub = &mut prefs.subtitles;
                r.level("A", ("Font size", "字幕大小"), EMPTY, &mut sub.size, 12..=140, 1.0, "");
                r.level("↕", ("Position", "字幕位置"), EMPTY, &mut sub.position, 0..=150, 1.0, "");
                r.level("↕", ("Secondary position", "字幕位置（次字幕）"), EMPTY, &mut sub.secondary_position, 0..=150, 1.0, "");
                r.level("▧", ("Outline size", "字幕描边"), EMPTY, &mut sub.border, 0.0..=10.0, 0.1, "");
                r.level("◩", ("Shadow offset", "字幕阴影偏移"), EMPTY, &mut sub.shadow, 0.0..=10.0, 0.1, "");
                r.level("⤢", ("Subtitle scale", "字幕缩放"), EMPTY, &mut sub.scale, 0.5..=3.0, 0.1, "x");
                r.switch("B", ("Bold", "粗体"), EMPTY, &mut sub.bold);
                r.color(("Subtitle color", "字幕颜色"), &mut sub.color);
                r.color(("Outline color", "字幕描边颜色"), &mut sub.border_color);
                r.color(("Shadow color", "字幕阴影颜色"), &mut sub.shadow_color);
                r.pick("▤", ("Override ASS/SSA styles", "强制覆盖 ASS/SSA 字幕"), ("Choose Force to apply the font and color settings to ASS/SSA", "选择 Force 可将所设字号和颜色应用到 ASS/SSA 字幕"), &mut sub.ass, ASS_MODES);
                r.pick("▤", ("Override secondary ASS/SSA styles", "强制覆盖 ASS/SSA 字幕（次字幕）"), EMPTY, &mut sub.secondary_ass, ASS_MODES);
                if r.command("↶", ("Restore defaults", "恢复默认"), "", None) { emit(Command::ResetSubtitles); }
            }),
            Tab::External => group(ui, colors, |r| {
                let external = &mut prefs.external;
                let placeholder = tr(("Search PATH when no application is selected", "未选择时从 PATH 查找"));
                r.switch("▷", ("Enable external mpv", "开启外部 mpv 播放器"), ("Show an external mpv action on details", "在详情页显示外部 mpv 播放按钮"), &mut external.mpv);
                if r.command("▣", ("External mpv executable", "外部 mpv 播放器位置"),
                    &external.mpv_path.as_ref().map_or_else(|| placeholder.clone(), |p| p.to_string_lossy()),
                    Some(CHOOSE)) { emit(Command::ChooseExternal(PlayerChoice::Mpv)); }
                r.switch("⇄", ("Use system proxy for external mpv", "外部 mpv 使用系统代理"), ("Use the system HTTP proxy when available", "启动外部 mpv 时应用系统代理"), &mut external.mpv_proxy);
                r.switch("▷", ("Enable external PotPlayer", "开启外部 PotPlayer 播放器"), ("Show a PotPlayer action on details", "在详情页显示 PotPlayer 播放按钮"), &mut external.potplayer);
                if r.command("▣", ("PotPlayer executable", "外部 PotPlayer 播放器位置"),
                    &external.potplayer_path.as_ref().map_or_else(|| placeholder.clone(), |p| p.to_string_lossy()),
                    Some(CHOOSE)) { emit(Command::ChooseExternal(PlayerChoice::PotPlayer)); }
                r.level("☷", ("Episode queue size", "剧集加载数量"), ("Number of episodes handed to the external player", "每次交给外部播放器的剧集数量"),
                    &mut external.episode_count, 1..=30, 1.0, "");
            }),
            Tab::Danmaku => group(ui, colors, |r| {
                let danmaku = &mut prefs.danmaku;
                r.switch("▤", ("Enable danmaku", "开启弹幕"), ("Display local and online danmaku in the player", "在播放器中显示本地和在线弹幕"), &mut danmaku.enabled);
                let api_hint = if danmaku.api_url.is_empty() {
                    ("Enter a DandanPlay v2 compatible API address", "填写兼容 DandanPlay v2 的 API 地址")
                } else {
                    ("DandanPlay v2 address saved", "已保存 DandanPlay v2 地址")
                };
                if r.command("⇄", Editor::DanmakuApi.label(), &tr(api_hint), None) {
                    emit(Command::Edit(Editor::DanmakuApi));
                }
                r.level("☷", ("Maximum scroll lines", "滚动弹幕最大行数"), EMPTY, &mut danmaku.scroll_lines, 0..=20, 1.0, "");
                r.level("⇡", ("Maximum top lines", "顶部弹幕最大行数"), EMPTY, &mut danmaku.top_lines, 0..=10, 1.0, "");
                r.level("⇣", ("Maximum bottom lines", "底部弹幕最大行数"), EMPTY, &mut danmaku.bottom_lines, 0..=10, 1.0, "");
                r.level("⤢", ("Danmaku text scale", "弹幕缩放"), EMPTY, &mut danmaku.scale, 0.5..=3.0, 0.1, "x");
                r.level("↕", ("Line spacing", "弹幕行间距"), EMPTY, &mut danmaku.line_spacing, 0..=40, 1.0, " px");
                r.level("◐", ("Opacity", "弹幕透明度"), EMPTY, &mut danmaku.opacity, 0..=100, 1.0, "%");
                r.level("»", ("Scroll speed", "弹幕滚动速度"), EMPTY, &mut danmaku.speed, 0.25..=3.0, 0.05, "x");
                r.switch("⇄", ("Sync to playback speed", "弹幕速度同步播放倍速"), EMPTY, &mut danmaku.sync_speed);
                r.switch("B", ("Bold", "粗体"), EMPTY, &mut danmaku.bold);
                r.switch("◷", ("Compatible animation", "兼容动画模式"), ("Render animation at a fixed 30 updates per second", "将弹幕动画限制为约 30 次更新每秒"), &mut danmaku.compatible);
                r.switch("↶", ("Remember selected danmaku", "记忆手动选择的弹幕"), ("Load the last XML selected for this media item", "下次播放同一媒体时加载上次选择的 XML"), &mut danmaku.remember);
                r.switch("≋", ("Hide danmaku heatmap", "隐藏弹幕热力图"), EMPTY, &mut danmaku.hide_heatmap);
                if r.command("≡", Editor::BlockedWords.label(), &tr(("Enter one word or /regular expression/ per line", "每行一个屏蔽词或 /正则表达式/")), None) { emit(Command::Edit(Editor::BlockedWords)); }
            }),
            Tab::About => group(ui, colors, |r| {
                use updates::{CheckError, Outcome, State};
                r.row("ⓘ", ("Version", "版本"), concat!("v", env!("CARGO_PKG_VERSION")), 0.0);
                r.switch("⇄", ("Check for updates on startup", "启动时检查更新"),
                    ("Check GitHub for a newer release in the background", "在后台检查 GitHub 正式版本"),
                    &mut prefs.check_updates_on_start);
                let status = match &self.update_check.state {
                    State::Idle => tr(("Check the latest release on GitHub", "检查 GitHub 的最新正式版本")),
                    State::Checking { .. } => tr(("Checking GitHub…", "正在检查 GitHub…")),
                    State::Finished(Outcome::Current) => tr(("You're up to date", "当前已是最新版本")),
                    State::Finished(Outcome::NoRelease) => tr(("No stable release is available yet", "暂未发布正式版本")),
                    State::Finished(Outcome::Unconfigured) => tr(("Update source is unavailable", "此构建暂无更新源")),
                    State::Finished(Outcome::Failed(error)) => tr(match error {
                        CheckError::Connection => ("Could not connect to GitHub. Try again.", "连接 GitHub 失败，可重试"),
                        CheckError::RateLimited => ("GitHub request limit reached. Try later.", "GitHub 请求受限，请稍后重试"),
                        CheckError::Response => ("Could not read release information. Try again.", "无法读取发布信息，可重试"),
                    }),
                    State::Finished(Outcome::Available(update)) => {
                        format!("{} v{}", tr(("New version available", "发现新版本")), update.version).into()
                    }
                };
                if matches!(self.update_check.state, State::Checking { .. }) {
                    r.row("⇄", ("Software update", "软件更新"), &status, 0.0);
                } else if r.command("⇄", ("Software update", "软件更新"), &status, Some(("Check for updates", "检查更新"))) {
                    emit(Command::CheckForUpdate);
                }
                if let Some(update) = self.update_check.available() {
                    if let Some(url) = &update.download_url {
                        if r.command("↓", ("Download update", "下载新版"), "Windows x64 ZIP", Some(("Download", "下载"))) {
                            emit(Command::OpenLink(url.clone()));
                        }
                    }
                    let hint = if update.download_url.is_some() {
                        EMPTY
                    } else {
                        ("Windows x64 archive is pending", "Windows x64 便携包待上传")
                    };
                    if r.command("≡", ("Release notes", "发布说明"), &tr(hint), None) {
                        emit(Command::OpenLink(update.notes_url.clone()));
                    }
                }
                if r.command("›", ("GitHub repository", "GitHub 仓库"), "", None) {
                    emit(Command::OpenLink(updates::REPOSITORY.into()));
                }
                r.row("▷", ("Playback engine", "播放引擎"), "libmpv 0.42.0", 0.0);
                if r.command("▣", ("Open application data", "打开数据文件夹"), "", Some(("Open folder", "打开文件夹"))) { emit(Command::OpenData); }
            }),
        }
    }

    pub(super) fn draw_update_notice(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        if !self.update_check.announce || self.player.is_some() || self.form.is_some() || self.view == View::Settings {
            return;
        }
        let Some(update) = self.update_check.available() else { return };
        let title = format!("{} v{}", tr(("Media Lite update available", "Media Lite 有新版")), update.version);
        egui::Window::new("Media Lite update")
            .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-24.0, -24.0))
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(RichText::new(title).strong());
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(tr(("View update", "查看更新"))).clicked() {
                        self.settings_view.tab = Tab::About;
                        self.update_check.announce = false;
                        actions.push(Action::GoSettings);
                    }
                    if ui.button(tr(("Later", "稍后"))).clicked() {
                        self.update_check.announce = false;
                    }
                });
            });
    }
}

pub(super) fn mpv_conf() -> PathBuf {
    config_path().with_file_name("mpv.conf")
}

pub(super) fn mpv_logs() -> PathBuf {
    config_path().with_file_name("mpv").join("Logs")
}

impl AppLite {
    pub(super) fn finish_preference_changes(&mut self, previous: &Preferences) {
        if *previous == self.prefs {
            return;
        }
        if previous.language != self.prefs.language {
            i18n::set(if self.prefs.language == "auto" { i18n::from_os() } else { i18n::from_code(&self.prefs.language) });
        }
        self.dark = match self.prefs.theme {
            ThemeMode::Light => false, ThemeMode::Dark => true, ThemeMode::System => self.dark,
        };
        if previous.network != self.prefs.network {
            let mut network = self.prefs.network.clone();
            // The TLS choice is applied when the app starts.
            network.ignore_ssl = api::network().ignore_ssl;
            if network.proxy != ProxyMode::Http || !network.proxy_address.is_empty() {
                api::configure_network(network);
            } else if self.settings_view.dialog.is_none() {
                self.settings_view.dialog = Some(Dialog::Edit {
                    field: Editor::Proxy, draft: String::new(), error: String::new(),
                });
            }
        }
        if previous.library.recommend_filter != self.prefs.library.recommend_filter {
            self.recommended_rules = TextRules::new(&self.prefs.library.recommend_filter).unwrap_or_default();
            self.hero_index = 0;
        }
        if previous.library.image_days != self.prefs.library.image_days {
            self.images.textures.clear();
        }
        if (self.prefs.playback.default_player == PlayerChoice::Mpv && !self.prefs.external.mpv)
            || (self.prefs.playback.default_player == PlayerChoice::PotPlayer && !self.prefs.external.potplayer)
        {
            self.prefs.playback.default_player = PlayerChoice::BuiltIn;
        }
        self.settings_view.dirty = true;
    }

    pub(super) fn write_settings(&mut self) {
        if self.prefs.network.proxy == ProxyMode::Http && self.prefs.network.proxy_address.is_empty() {
            self.prefs.network.proxy = api::network().proxy;
        }
        let result = write_config(&self.config(), &config_path());
        self.settings_view.dirty = false;
        self.settings_view.config_error = result.err().map(|error| error.to_string());
    }

    pub(super) fn flush_settings(&mut self) {
        if self.settings_view.dirty {
            self.write_settings();
        }
    }

    fn draw_setting_dialog(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let Some(mut dialog) = self.settings_view.dialog.take() else { return };
        let mut close = false;
        match &mut dialog {
            Dialog::Edit { field, draft, error } => {
                egui::Window::new(tr(field.label())).id(egui::Id::new("settings-editor"))
                    .anchor(Align2::CENTER_CENTER, Vec2::ZERO).collapsible(false).resizable(false)
                    .show(ctx, |ui| {
                        let proxy = matches!(field, Editor::Proxy);
                        let address_hint = match field {
                            Editor::Proxy => Some("http://127.0.0.1:10808"),
                            Editor::DanmakuApi => Some("https://example.com/api/v2"),
                            _ => None,
                        };
                        if let Some(hint) = address_hint {
                            if matches!(field, Editor::DanmakuApi) {
                                ui.label(tr(("Use the service URL or its /api/v2 address. Clear the field to remove it.",
                                    "可填服务根地址或 /api/v2 地址。清空后保存可移除配置。")));
                            }
                            ui.add_sized([408.0, 32.0], egui::TextEdit::singleline(draft).hint_text(hint));
                        } else {
                            ui.label(tr(("Use one entry per line. Wrap a regular expression in /…/.", "每行一项，正则表达式用 /…/ 包住。")));
                            ui.add_sized([408.0, 162.0], egui::TextEdit::multiline(draft));
                        }
                        if !error.is_empty() {
                            ui.colored_label(Color32::LIGHT_RED, &*error);
                        }
                        ui.horizontal(|ui| {
                            if ui.button(tr(("Cancel", "取消"))).clicked() {
                                close = true;
                                if proxy && self.prefs.network.proxy_address.is_empty() {
                                    self.prefs.network.proxy = api::network().proxy;
                                }
                            }
                            if ui.button(tr(("Save", "保存"))).clicked() {
                                let value = draft.trim();
                                let valid = match field {
                                    Editor::Proxy => Network::valid_proxy_address(value),
                                    Editor::DanmakuApi => value.is_empty() || danmaku::Endpoint::parse(value).is_some(),
                                    _ => TextRules::new(value).is_ok(),
                                };
                                if valid {
                                    *field.value(&mut self.prefs) = value.to_owned();
                                    if proxy {
                                        self.prefs.network.proxy = ProxyMode::Http;
                                    }
                                    close = true;
                                } else {
                                    *error = tr(match field {
                                        Editor::Proxy => ("Enter an HTTP proxy URL with host and optional port", "请输入带主机地址和可选端口的 HTTP 代理地址"),
                                        Editor::DanmakuApi => ("Enter a valid HTTP or HTTPS danmaku API address", "请输入有效的 HTTP 或 HTTPS 弹幕 API 地址"),
                                        _ => ("Check the regular expression", "请检查正则表达式"),
                                    }).into_owned();
                                }
                            }
                        });
                    });
            }
            Dialog::Restore(backup) => {
                egui::Window::new(tr(("Confirm restore", "确认还原"))).id(egui::Id::new("settings-restore"))
                    .anchor(Align2::CENTER_CENTER, Vec2::ZERO).collapsible(false).resizable(false)
                    .show(ctx, |ui| {
                        ui.label(tr(("Restoring replaces your saved server list. Settings are restored when included.",
                            "还原会替换本机的服务器列表。备份内包含设置时，也会一起还原。")));
                        ui.label(format!("{} {}", tr(("Servers in backup", "备份内的服务器数")), backup.config.servers.len()));
                        ui.horizontal(|ui| {
                            if ui.button(tr(("Cancel", "取消"))).clicked() { close = true; }
                            if ui.button(tr(("Restore", "还原"))).clicked() {
                                actions.push(Action::Settings(Command::Restore(backup.clone())));
                                close = true;
                            }
                        });
                    });
            }
            Dialog::WebDav { draft, password, error } => {
                egui::Window::new(tr(("WebDAV settings", "WebDAV 配置"))).id(egui::Id::new("settings-webdav"))
                    .anchor(Align2::CENTER_CENTER, Vec2::ZERO).collapsible(false).resizable(false)
                    .show(ctx, |ui| {
                        for (label, value, secret) in [
                            (("Address", "地址"), &mut draft.address, false),
                            (("Username", "用户名"), &mut draft.username, false),
                            (("Password for this run", "本次运行使用的密码"), &mut *password, true),
                            (("Backup file path", "备份文件路径"), &mut draft.path, false),
                        ] {
                            ui.label(tr(label));
                            ui.add_sized([400.0, 30.0], egui::TextEdit::singleline(value).password(secret));
                        }
                        if *error {
                            ui.colored_label(Color32::LIGHT_RED, tr(("Enter a valid HTTP(S) address and file path", "请输入有效的 HTTP(S) 地址和文件路径")));
                        }
                        ui.horizontal(|ui| {
                            if ui.button(tr(("Cancel", "取消"))).clicked() { close = true; }
                            if ui.button(tr(("Save", "保存"))).clicked() {
                                if draft.endpoint().is_some() {
                                    self.prefs.webdav = draft.clone();
                                    // This is connection input only. Keep the password in memory.
                                    self.settings_view.webdav_password = password.clone();
                                    close = true;
                                } else {
                                    *error = true;
                                }
                            }
                        });
                    });
            }
        }
        if !close {
            self.settings_view.dialog = Some(dialog);
        }
    }

    pub(super) fn apply_setting_command(&mut self, command: Command, ctx: &egui::Context) {
        use crate::platform::{open_in_shell, pick_file, pick_folder};
        use std::fs;
        let previous = self.prefs.clone();
        let result = match command {
            Command::CheckForUpdate => {
                self.update_check.start(ctx, false);
                return;
            }
            Command::OpenLink(url) => {
                ctx.open_url(egui::OpenUrl::new_tab(url));
                return;
            }
            Command::Edit(field) => {
                let draft = field.value(&mut self.prefs).clone();
                self.settings_view.dialog = Some(Dialog::Edit { field, draft, error: String::new() });
                return;
            }
            Command::Backup => {
                let Some(path) = pick_file(self.host_hwnd, &tr(("Save backup", "保存备份")), "*.json", Some("MediaLiteBackup.json")) else { return };
                write_config(&self.backup(), &path)
                    .map(|_| tr(("Backup saved", "备份已保存")))
            }
            Command::EditWebDav => {
                self.settings_view.dialog = Some(Dialog::WebDav {
                    draft: self.prefs.webdav.clone(), password: self.settings_view.webdav_password.clone(), error: false,
                });
                return;
            }
            Command::TransferWebDav(upload) => {
                let Some(url) = self.prefs.webdav.endpoint() else {
                    self.apply_setting_command(Command::EditWebDav, ctx);
                    return;
                };
                let data = if upload {
                    match serde_json::to_vec_pretty(&self.backup()) {
                        Ok(bytes) => Some(bytes),
                        Err(error) => {
                            self.settings_view.notice = Some((error.to_string(), true));
                            return;
                        }
                    }
                } else { None };
                self.worker.send(Job::WebDav {
                    url, username: self.prefs.webdav.username.clone(),
                    password: self.settings_view.webdav_password.clone(), data,
                });
                self.settings_view.notice = Some((tr(("Transferring backup…", "正在传输备份…")).into_owned(), false));
                return;
            }
            Command::ChooseRestore => {
                let Some(path) = pick_file(self.host_hwnd, &tr(("Select backup", "选择备份文件")), "*.json", None) else { return };
                let backup = fs::metadata(&path).ok().filter(|m| m.len() <= 16 * 1024 * 1024)
                    .and_then(|_| fs::read(&path).ok())
                    .and_then(|bytes| Backup::decode(&bytes));
                if let Some(backup) = backup {
                    self.settings_view.dialog = Some(Dialog::Restore(Box::new(backup)));
                    return;
                }
                self.settings_view.notice = Some((tr(("Invalid or unsupported backup file", "备份文件无效或版本不受支持")).into_owned(), true));
                return;
            }
            Command::Restore(backup) => {
                let incoming = backup.config;
                let mut next = if incoming.preferences.is_some() { incoming.clone() } else { self.config() };
                next.servers = incoming.servers;
                next.active_id = incoming.active_id;
                next.preferences = Some(next.settings());
                match write_config(&next, &config_path()) {
                    Ok(()) => {
                        self.prefs = next.settings();
                        if previous.webdav != self.prefs.webdav {
                            self.settings_view.webdav_password.clear();
                        }
                        self.servers = next.servers;
                        self.search_history = next.search_history;
                        self.danmaku_history = next.danmaku_history;
                        self.episode_grid = next.episode_grid;
                        self.list_view = next.list_view.unwrap_or(false);
                        self.card_scale = next.card_scale.as_deref().map_or(CardScale::Medium, CardScale::from_code);
                        self.active = next.active_id.as_deref().and_then(|id| self.servers.iter().position(|s| s.id == id))
                            .or_else(|| (!self.servers.is_empty()).then_some(0));
                        self.rows.clear();
                        self.libs.clear();
                        self.detail = None;
                        self.settings_view.back = View::Home;
                        self.connect_active();
                        self.view = View::Settings;
                        Ok(tr(("Backup restored", "备份已还原")))
                    }
                    Err(error) => Err(error),
                }
            }
            Command::ClearData => crate::net::clear_response_cache().map(|_| tr(("Data cache cleared", "已清空数据缓存"))),
            Command::ClearImages => crate::net::clear_cache_files(&crate::net::cache_root().join("img"), |path| {
                matches!(path.extension().and_then(|ext| ext.to_str()), Some("img" | "tmp"))
            })
                .map(|_| {
                    self.images = ImageCache::new();
                    tr(("Image cache cleared", "已清空图片缓存"))
                }),
            Command::ClearPlayback => crate::net::clear_cache_files(&self.prefs.cache.directory(), |path| {
                path.file_name().and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("mpv-cache-") && name.ends_with(".dat"))
            })
                .map(|_| tr(("Playback cache cleared", "已清空播放缓存"))),
            Command::ChooseCacheDirectory => {
                let Some(path) = pick_folder(self.host_hwnd, &tr(("Choose cache directory", "选择磁盘缓存目录"))) else { return };
                self.prefs.cache.disk_parent = Some(path);
                Ok(tr(("Playback cache directory updated", "已修改播放缓存目录")))
            }
            Command::ChooseExternal(choice) => {
                let Some(path) = pick_file(self.host_hwnd, &tr(CHOOSE), "*.exe", None) else { return };
                match choice {
                    PlayerChoice::Mpv => self.prefs.external.mpv_path = Some(path),
                    PlayerChoice::PotPlayer => self.prefs.external.potplayer_path = Some(path),
                    PlayerChoice::BuiltIn => return,
                }
                Ok(tr(("Player location saved", "已保存播放器位置")))
            }
            Command::EditMpv => {
                let path = mpv_conf();
                if let Some(parent) = path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                fs::OpenOptions::new().create(true).append(true).open(&path)
                    .and_then(|_| std::process::Command::new("notepad.exe").arg(&path).spawn().map(|_| ()))
                    .map(|_| tr(("Opened mpv.conf", "已打开 mpv.conf")))
            }
            Command::OpenLogs | Command::OpenData => {
                let path = if matches!(command, Command::OpenLogs) { mpv_logs() }
                    else { config_path().parent().unwrap().to_path_buf() };
                fs::create_dir_all(&path).and_then(|_| open_in_shell(&path))
                    .map(|_| tr(("Folder opened", "已打开文件夹")))
            }
            Command::ResetSubtitles => {
                self.prefs.subtitles = Default::default();
                Ok(tr(("Subtitle defaults restored", "已恢复默认字幕样式")))
            }
            Command::ClearSearchHistory => {
                self.search_history.clear();
                self.settings_view.dirty = true;
                Ok(tr(("Search history cleared", "已清空搜索历史")))
            }
        };
        self.settings_view.last_stats_at = f64::NEG_INFINITY;
        self.settings_view.notice = Some(match result {
            Ok(message) => (message.into_owned(), false),
            Err(error) => (format!("{} · {error}", tr(("The operation could not be completed", "操作未能完成"))), true),
        });
        self.finish_preference_changes(&previous);
        if self.settings_view.dirty {
            self.write_settings();
        }
        ctx.request_repaint();
    }

    fn backup(&self) -> Backup {
        let mut config = self.config();
        if !self.prefs.backup_settings {
            config = Config { servers: config.servers, active_id: config.active_id, ..Default::default() };
        }
        Backup { version: 1, config }
    }

    pub(super) fn webdav_result(&mut self, result: Result<Option<Vec<u8>>, String>) {
        self.settings_view.notice = match result {
            Ok(Some(bytes)) => {
                if let Some(backup) = Backup::decode(&bytes) {
                    self.settings_view.dialog = Some(Dialog::Restore(Box::new(backup)));
                    None
                } else {
                    Some((tr(("Invalid or unsupported backup file", "备份文件无效或版本不受支持")).into_owned(), true))
                }
            }
            Ok(None) => Some((tr(("WebDAV backup saved", "备份已保存到 WebDAV")).into_owned(), false)),
            Err(error) => Some((error, true)),
        };
    }
}

use super::*;

pub(super) const TOP_BAR_HEIGHT: f32 = 62.0;
const CAROUSEL_INTERVAL: f64 = 7.0;
const ARTWORK_FADE: f64 = 0.34;

pub(super) fn artwork_colors(image: &egui::ColorImage) -> [Color32; 2] {
    [0, 1].map(|half| {
        let [width, height] = image.size;
        let mut sum = [0_u32; 3];
        let mut weight_sum = 0;
        for row in 0..8 {
            for column in 0..8 {
                let y = (row * 2 + 1) * height / 16;
                let x = (half * 16 + column * 2 + 1) * width / 32;
                let [r, g, b, a] = image.pixels[y * width + x].to_array();
                if a < 128 { continue; }
                let weight = u32::from(r.max(g).max(b) - r.min(g).min(b)) + 32;
                for (total, channel) in sum.iter_mut().zip([r, g, b]) {
                    *total += weight * u32::from(channel);
                }
                weight_sum += weight;
            }
        }
        let [r, g, b] = sum.map(|channel| (channel / weight_sum.max(1)) as u8);
        Color32::from_rgb(r, g, b)
    })
}

fn mix(a: Color32, b: Color32, amount: f32) -> Color32 {
    let channel = |x, y| egui::lerp(f32::from(x)..=f32::from(y), amount).round() as u8;
    Color32::from_rgb(channel(a.r(), b.r()), channel(a.g(), b.g()), channel(a.b(), b.b()))
}

pub(super) fn gradient(rect: Rect, [tl, tr, bl, br]: [Color32; 4]) -> egui::Shape {
    let mut mesh = egui::Mesh::default();
    for (point, color) in [
        (rect.left_top(), tl), (rect.right_top(), tr),
        (rect.left_bottom(), bl), (rect.right_bottom(), br),
    ] {
        mesh.colored_vertex(point, color);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    egui::Shape::mesh(mesh)
}

pub(super) fn window_buttons(ui: &Ui, bar: Rect, fill: Color32, line: Color32, ink: Color32) -> Rect {
    let rect = Rect::from_center_size(Pos2::new(bar.right() - 101.0, bar.center().y), Vec2::new(138.0, 36.0));
    let maximized = ui.input(|input| input.viewport().maximized == Some(true));
    ui.painter().rect(rect, 18.0, fill, Stroke::new(1.0_f32, line));
    for (index, (icon, label, command)) in [
        ("minimize", "Minimize", egui::ViewportCommand::Minimized(true)),
        (if maximized { "restore" } else { "maximize" }, if maximized { "Restore window" } else { "Maximize" },
            egui::ViewportCommand::Maximized(!maximized)),
        ("close", "Close", egui::ViewportCommand::Close),
    ].into_iter().enumerate() {
        let center = Pos2::new(rect.left() + 25.0 + index as f32 * 44.0, rect.center().y);
        let hit = Rect::from_center_size(center, Vec2::new(42.0, 34.0));
        let response = ui.interact(hit, ui.id().with(("main-window", index)), Sense::click());
        if response.hovered() || response.has_focus() {
            ui.painter().rect_filled(hit, 17.0, ink.gamma_multiply(0.13));
        }
        theme::icon(ui, icon, center, 15.0, ink);
        let label = theme::tl(label);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
        if response.on_hover_text(label).clicked() { ui.ctx().send_viewport_cmd(command); }
    }
    rect
}

pub(super) fn drag_window(ui: &Ui, rect: Rect, id: egui::Id) {
    if rect.width() < 8.0 { return; }
    let drag = ui.interact(rect, id, Sense::click_and_drag());
    if drag.double_clicked() {
        let maximized = ui.input(|input| input.viewport().maximized == Some(true));
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    } else if drag.drag_started() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
}

impl AppLite {
    pub(super) fn draw_home(&mut self, ui: &mut Ui, ctx: &egui::Context, p: &Palette, actions: &mut Vec<Action>) {
        if self.autoplay && !self.rows.is_empty() && self.player.is_none() {
            self.autoplay = false;
            if let Some(item) = self.rows.iter().flat_map(|row| &row.items).find(|item| item.is_playable()).cloned() {
                self.start_playback(item);
                ctx.request_repaint();
            }
        }

        if self.rows.is_empty() {
            self.hero_was_visible = false;
            ui.add_space(124.0);
            ui.vertical_centered(|ui| {
                if self.loading {
                    ui.spinner();
                    ui.add_space(12.0);
                    ui.label(theme::body("Loading your library…", 14.0, p.dim));
                } else if let Some((message, true)) = self.status.as_ref() {
                    ui.label(theme::heading("Could not load the library", 23.0, p.text));
                    ui.add_space(10.0);
                    ui.label(RichText::new(message).size(14.0).color(p.dim));
                    ui.add_space(18.0);
                    if btn(ui, &theme::tl("Refresh"), false, p).clicked() {
                        actions.push(Action::Refresh);
                    }
                } else {
                    ui.label(theme::heading("Nothing here yet", 23.0, p.text));
                }
            });
            return;
        }

        let heroes = self.hero_items();
        let server_id = self.server().map(|server| server.id.clone()).unwrap_or_default();
        let viewport = ui.max_rect();
        let inset = (viewport.width() * 0.05).clamp(44.0, 64.0);
        let surface = if p.dark { Color32::from_rgb(13, 19, 26) } else { p.bg };
        let background = ui.painter().add(egui::Shape::Noop);
        let tones = ScrollArea::vertical()
            .id_source(("home-page", &server_id))
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .drag_to_scroll(false)
            .auto_shrink([false, false])
            .show_viewport(ui, |ui, visible| {
                let visible = visible.translate(ui.max_rect().min.to_vec2());
                ui.set_clip_rect(ui.clip_rect().intersect(visible));
                ui.spacing_mut().item_spacing.y = 0.0;
                let width = ui.available_width();
                let tones = if heroes.is_empty() {
                    self.hero_was_visible = false;
                    ui.add_space(TOP_BAR_HEIGHT + 18.0);
                    [surface; 2]
                } else {
                    let height = (viewport.height() * 0.66).clamp(386.0, 620.0);
                    let (banner, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
                    let tones = self.draw_home_banner(ui, banner, inset, surface, p.dark, &heroes, actions);
                    ui.add_space(22.0);
                    tones
                };
                self.draw_home_rails(ui, inset, p, actions);
                tones
            }).inner;
        let [left, right] = tones;
        let blend_start = viewport.top() + viewport.height() * 0.70;
        ui.painter().set(background, egui::Shape::Vec(vec![
            gradient(viewport.with_max_y(blend_start), [left, right, left, right]),
            gradient(viewport.with_min_y(blend_start), [left, right, mix(left, surface, 0.56), mix(right, surface, 0.56)]),
        ]));
    }

    fn draw_home_banner(
        &mut self, ui: &mut Ui, banner: Rect, inset: f32, surface: Color32, dark: bool,
        items: &[MediaItem], actions: &mut Vec<Action>,
    ) -> [Color32; 2] {
        let ctx = ui.ctx().clone();
        let (now, focused) = ctx.input(|input| (input.time, input.focused));
        let in_view = banner.intersect(ui.clip_rect()).height() >= banner.height() * 0.54;
        self.hero_index %= items.len();
        let index = self.hero_index;
        let next = (index + 1) % items.len();
        let load_art = |app: &mut Self, index: usize| {
            app.texture(&items[index], "Backdrop", 1280, now)
                .or_else(|| app.texture(&items[index], "Primary", 768, now))
        };
        let mut texture = load_art(self, index);
        let next_texture = if items.len() > 1 && in_view { load_art(self, next) } else { None };
        if !in_view || !focused || !self.hero_was_visible || self.server_menu || self.form.is_some() {
            self.hero_next_at = now + CAROUSEL_INTERVAL;
        } else if items.len() > 1 && now >= self.hero_next_at {
            self.hero_index = next;
            self.hero_previous = texture.map(|id| (id, now));
            self.hero_next_at = now + CAROUSEL_INTERVAL;
            texture = next_texture;
        }
        self.hero_was_visible = in_view;
        if in_view && focused && items.len() > 1 {
            ctx.request_repaint_after_secs((self.hero_next_at - now) as f32);
        }

        let tint = |texture: Option<TextureId>| {
            texture.and_then(|id| self.images.colors(id)).map_or([surface; 2], |colors| {
                colors.map(|color| mix(surface, color, if dark { 0.25 } else { 0.12 }))
            })
        };
        let mut tones = tint(texture);
        let fade = self.hero_previous.and_then(|(id, since)| {
            let fraction = (1.0 - (now - since) / ARTWORK_FADE).clamp(0.0, 1.0) as f32;
            (fraction > 0.0).then_some((id, fraction))
        });
        if let Some((id, fraction)) = fade {
            let previous = tint(Some(id));
            tones = [mix(tones[0], previous[0], fraction), mix(tones[1], previous[1], fraction)];
            ctx.request_repaint();
        } else {
            self.hero_previous = None;
        }
        if !ui.is_rect_visible(banner) { return tones; }

        let current = self.hero_index;
        let item = &items[current];
        let artwork = banner.with_max_y(banner.bottom() - 19.0);
        let painter = ui.painter().with_clip_rect(banner.intersect(ui.clip_rect()));
        for (id, alpha) in texture.map(|id| (id, 255)).into_iter()
            .chain(fade.map(|(id, amount)| (id, (amount * 255.0) as u8)))
        {
            if let Some([width, height]) = ctx.tex_manager().read().meta(id).map(|meta| meta.size) {
                let uv = cover_uv(artwork.size(), Vec2::new(width as f32, height as f32));
                // Leave room near the top of the shot for faces and title artwork.
                let uv = uv.translate(Vec2::new(0.0, -uv.min.y * 0.92));
                let mut image = egui::Mesh::with_texture(id);
                image.add_rect_with_uv(artwork, uv, Color32::from_white_alpha(alpha));
                painter.add(egui::Shape::mesh(image));
            }
        }
        let ink = Color32::from_rgb(244, 245, 248);
        let ink_muted = Color32::from_rgb(227, 230, 236);
        let has_image = texture.is_some() || fade.is_some();
        if has_image {
            painter.add(gradient(artwork, [
                Color32::from_black_alpha(65), Color32::TRANSPARENT,
                Color32::from_black_alpha(210), Color32::from_black_alpha(75),
            ]));
            let fade_top = artwork.top() + artwork.height() * 0.42;
            painter.add(gradient(artwork.with_min_y(fade_top), [Color32::TRANSPARENT, Color32::TRANSPARENT, tones[0], tones[1]]));
        }
        let ink = if has_image || dark { ink } else { Color32::from_rgb(19, 24, 32) };
        let ink_muted = if has_image || dark { ink_muted } else { Color32::from_rgb(54, 64, 78) };
        let left = banner.left() + inset;
        let text_width = (banner.width() - inset * 2.0).min(604.0);
        let meta_y = artwork.bottom() - 94.0;
        let logo_bottom = meta_y - 22.0;
        let title = display_title(item);
        let logo = item.logo.as_ref().and_then(|(owner, tag)| self.artwork(owner, "Logo", tag, 640, now));
        if let Some((id, [width, height])) = logo.and_then(|id| {
            ctx.tex_manager().read().meta(id).map(|meta| (id, meta.size))
        }) {
            let size = Vec2::new(width as f32, height as f32);
            let scale = ((banner.width() * 0.26).clamp(216.0, 330.0) / size.x).min(116.0 / size.y);
            let bounds = Rect::from_min_size(Pos2::new(left, logo_bottom - size.y * scale), size * scale);
            painter.image(id, bounds, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        } else {
            let font = FontId::new((banner.width() * 0.037).clamp(30.0, 48.0), egui::FontFamily::Name("home-display".into()));
            let mut job = egui::text::LayoutJob::simple(title.clone(), font, ink, text_width);
            job.wrap.max_rows = 2;
            let galley = painter.layout_job(job);
            painter.galley(Pos2::new(left, logo_bottom - galley.size().y), galley, ink);
        }
        let mut metadata = Vec::new();
        if !item.official_rating.is_empty() { metadata.push(item.official_rating.clone()); }
        if let Some(year) = item.year { metadata.push(year.to_string()); }
        if let (Some(season), Some(episode)) = (item.parent_index, item.index) {
            if item.kind == "Episode" { metadata.push(format!("S{season:02}E{episode:02}")); }
        }
        if !item.genres.is_empty() {
            metadata.push(item.genres.iter().take(3).map(|genre| i18n::t(genre).into_owned()).collect::<Vec<_>>().join("/"));
        }
        let mut meta_left = left;
        if let Some(rating) = item.community_rating.filter(|value| *value > 0.0) {
            let gold = if has_image || dark { Color32::from_rgb(242, 188, 88) } else { Color32::from_rgb(146, 87, 15) };
            let label = painter.text(Pos2::new(left, meta_y), Align2::LEFT_CENTER,
                format!("★ {rating:.1}"), FontId::proportional(14.0), gold);
            meta_left = label.right() + 13.0;
        }
        let summary = theme::clip_px(&painter, &metadata.join("   "), 14.0, text_width + left - meta_left, ink_muted);
        painter.text(Pos2::new(meta_left, meta_y), Align2::LEFT_CENTER, summary, FontId::proportional(14.0), ink_muted);
        if !item.overview.trim().is_empty() {
            let mut job = egui::text::LayoutJob::simple(item.overview.trim().to_owned(),
                FontId::proportional(15.0), ink, text_width);
            job.wrap.max_rows = 2;
            painter.galley(Pos2::new(left, meta_y + 23.0), painter.layout_job(job), ink);
        }

        let open_area = Rect::from_min_max(Pos2::new(left, banner.top() + TOP_BAR_HEIGHT),
            Pos2::new(banner.right() - inset, banner.bottom() - 38.0))
            .intersect(ctx.screen_rect().with_min_y(ctx.screen_rect().top() + TOP_BAR_HEIGHT));
        if open_area.is_positive() {
            let open = ui.interact(open_area, ui.id().with("home-banner"), Sense::click());
            open.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &title));
            if open.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                actions.push(Action::Open(item.id.clone()));
            }
        }
        if items.len() < 2 || !in_view { return tones; }

        let mut selected = current;
        for (x, glyph, label, target) in [
            (banner.left() + 28.0, "chevron-left", "Previous page", (current + items.len() - 1) % items.len()),
            (banner.right() - 28.0, "chevron-right", "Next page", (current + 1) % items.len()),
        ] {
            let bounds = Rect::from_center_size(Pos2::new(x, banner.top() + banner.height() * 0.51), Vec2::new(32.0, 48.0));
            if ui.is_rect_visible(bounds) {
                let response = ui.interact(bounds, ui.id().with(("banner-arrow", glyph)), Sense::click());
                let label = theme::tl(label);
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
                let alpha = if response.hovered() || response.has_focus() { 200 } else { 137 };
                painter.rect_filled(bounds, 16.0, Color32::from_black_alpha(alpha));
                painter.rect_stroke(bounds, 16.0, Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(160, 166, 174, 65)));
                theme::icon(ui, glyph, bounds.center(), 16.0, Color32::WHITE);
                if response.on_hover_text(label).clicked() { selected = target; }
            }
        }
        let pitch = 32.0;
        let first = banner.center().x - (items.len() - 1) as f32 * pitch * 0.5;
        for (index, slide) in items.iter().enumerate() {
            let center = Pos2::new(first + index as f32 * pitch, banner.bottom() - 23.0);
            let button = Rect::from_center_size(center, Vec2::new(pitch, 24.0));
            let response = ui.interact(button, ui.id().with(("hero-page", index)), Sense::click());
            let active = index == current;
            response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, display_title(slide)));
            let alpha = if active || response.hovered() || response.has_focus() { 245 } else { 105 };
            let tone = if dark { Color32::from_white_alpha(alpha) } else { Color32::from_black_alpha(alpha) };
            let dash = Rect::from_center_size(center, Vec2::new(if active { 24.0 } else { 12.0 }, 4.0));
            painter.rect_filled(dash, 2.0, tone);
            if response.clicked() { selected = index; }
        }
        if selected != current {
            self.hero_previous = texture.map(|id| (id, now));
            self.hero_index = selected;
            self.hero_next_at = now + CAROUSEL_INTERVAL;
            ctx.request_repaint();
        }
        tones
    }

    fn draw_home_rails(&mut self, ui: &mut Ui, inset: f32, p: &Palette, actions: &mut Vec<Action>) {
        let ctx = ui.ctx().clone();
        let width = ui.available_width();
        let gap = 16.0;
        let rows = std::mem::take(&mut self.rows);
        for (index, row) in rows.iter().enumerate().filter(|(_, row)| !row.items.is_empty()) {
            if (self.prefs.library.hide_resume && row.title == "Continue Watching")
                || (self.prefs.library.hide_library && self.libs.iter().any(|lib| lib.name == row.title))
            {
                continue;
            }
            let strip = matches!(row.title.as_str(), "Continue Watching" | "Next Up");
            let poster_width = self.card_scale.width() * 136.0 / CardScale::Medium.width();
            let card_width = if strip { 236.0 } else { poster_width } * self.prefs.library.cover_scale;
            let art_height = card_width * if strip { 9.0 / 16.0 } else { 1.5 };
            let card_height = art_height + if strip { 45.0 } else { 48.0 };
            let rail_width = width - inset - 4.0;
            let stride = card_width + gap;
            let page_size = (((rail_width + gap) / stride).floor() as usize).max(1);
            let pages = row.items.len().div_ceil(page_size);
            let id = ui.id().with(("home-rail", index, &row.title));
            let mut page = ui.data(|data| data.get_temp::<usize>(id).unwrap_or(0)).min(pages - 1);
            let (section, _) = ui.allocate_exact_size(Vec2::new(width, 44.0 + card_height), Sense::hover());
            if ui.is_rect_visible(section) {
                let left = section.left() + inset;
                let title = theme::tl(&row.title);
                let font = FontId::new(18.0, egui::FontFamily::Name("home-display".into()));
                let title = theme::clip_px(ui.painter(), &title, 18.0, rail_width - 30.0, p.text);
                let galley = ui.painter().layout_no_wrap(title.clone(), font, p.text);
                let center = Pos2::new(left, section.top() + 17.0);
                let heading = Rect::from_min_size(center - Vec2::new(0.0, 16.0), Vec2::new(galley.size().x + 25.0, 32.0));
                let response = ui.interact(heading, id.with("show-all"), Sense::click());
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &title));
                let end_x = left + galley.size().x;
                ui.painter().galley(center - Vec2::new(0.0, galley.size().y / 2.0), galley, p.text);
                theme::icon(ui, "chevron-right", Pos2::new(end_x + 14.0, center.y), 12.0,
                    if response.hovered() || response.has_focus() { p.text } else { p.dim });
                if response.on_hover_text(theme::tl("See all")).clicked() {
                    actions.push(Action::OpenHomeRow(index));
                }

                let cards = Rect::from_min_size(Pos2::new(left, section.top() + 44.0), Vec2::new(rail_width, card_height));
                let mut rail = ui.child_ui_with_id_source(cards, Layout::left_to_right(Align::Min), id.with("cards"), None);
                rail.set_clip_rect(rail.clip_rect().intersect(cards));
                rail.spacing_mut().item_spacing.x = gap;
                for item in row.items.iter().skip(page * page_size).take(page_size + 1) {
                    let opened = if strip {
                        self.strip_card(&mut rail, item, card_width, &ctx, p, actions)
                    } else {
                        self.poster_card(&mut rail, item, card_width, &ctx, p, actions)
                    };
                    if opened { actions.push(Action::Open(item.id.clone())); }
                }
                if pages > 1 {
                    let current = page;
                    let center_y = cards.top() + art_height * 0.5;
                    for (x, icon, label, target) in [
                        (section.left() + 27.0, "chevron-left", "Previous page", current.saturating_sub(1)),
                        (section.right() - 27.0, "chevron-right", "Next page", (current + 1).min(pages - 1)),
                    ] {
                        if target == current { continue; }
                        let bounds = Rect::from_center_size(Pos2::new(x, center_y), Vec2::splat(36.0));
                        let fill = if p.dark { Color32::from_rgba_unmultiplied(23, 29, 37, 223) } else { p.card };
                        if ui.is_rect_visible(bounds) {
                            let arrow = ui.child_ui_with_id_source(bounds, Layout::top_down(Align::Center), id.with(icon), None);
                            if detail_icon_button(&arrow, bounds, icon, label, false, fill).clicked() {
                                page = target;
                            }
                        }
                    }
                    if page != current {
                        ui.data_mut(|data| data.insert_temp(id, page));
                        ctx.request_repaint();
                    }
                }
            }
            ui.add_space(32.0);
        }
        self.rows = rows;
        ui.add_space(14.0);
    }
}

use super::*;
use std::time::Duration;

const SURFACE_WHITE: Color32 = Color32::from_rgb(243, 245, 248);
const SURFACE_DARK: Color32 = Color32::from_rgb(21, 24, 29);
const OVERLAY_SECONDARY: Color32 = Color32::from_rgb(207, 213, 222);
const CONTROL_IDLE: f64 = 3.0;
const DOUBLE_CLICK_DELAY: f64 = 0.3;

// Geometry measured in the HillsLite playback reference at 1282 × 721.
const SIDE_INSET: f32 = 35.0;
const RAIL_FROM_BOTTOM: f32 = 113.0;
const BUTTON_FROM_BOTTOM: f32 = 47.0;

#[derive(Clone, Copy)]
enum Icon {
    Previous, Next, Play, Pause, Volume, Muted, Speed(f64), Audio, Subtitles,
    Danmaku, Settings, Playlist, Fullscreen, Windowed, Pin, Minimize, Maximize, Restore, Close,
}

impl AppLite {
    pub(super) fn draw_player(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let now = ctx.input(|i| i.time);
        let preview = self.player.as_ref()
            .filter(|p| p.target.sample)
            .map(|p| p.item.clone());
        let preview_texture = preview.as_ref().and_then(|item| self.texture(item, "Backdrop", 1280, now));
        let fullscreen = ctx.input(|i| i.viewport().fullscreen == Some(true));
        let maximized = ctx.input(|i| i.viewport().maximized == Some(true));
        let screen = ctx.screen_rect();
        let left = screen.left() + SIDE_INSET;
        let right = screen.right() - SIDE_INSET;
        let bottom = screen.bottom();
        let toolbar_y = bottom - BUTTON_FROM_BOTTOM;
        let chapter_preview = self.player.as_ref().and_then(|player| {
            let point = ctx.input(|i| i.pointer.hover_pos())?;
            if !self.prefs.playback.preview || (point.y - (bottom - RAIL_FROM_BOTTOM)).abs() > 12.0
                || player.duration <= 0.0 || !(left..=right).contains(&point.x) || player.menu.is_some()
            {
                return None;
            }
            let ticks = ticks_from_secs(f64::from((point.x - left) / (right - left)) * player.duration);
            let chapter = player.chapters.iter().filter(|chapter| chapter.position_ticks <= ticks && chapter.image_tag.is_some())
                .max_by_key(|chapter| chapter.position_ticks)?;
            Some((player.item.id.clone(), chapter.clone()))
        });
        let thumbnail = chapter_preview.as_ref().and_then(|(id, chapter)| {
            self.artwork(id, &format!("Chapter/{}", chapter.index), chapter.image_tag.as_deref()?, 224, now)
        });
        let danmaku_prefs = &self.prefs.danmaku;
        let show_chapters = self.prefs.playback.chapters;
        let enable_secondary = self.prefs.playback.secondary;
        let intro_button = self.prefs.playback.intro == IntroMode::Button;
        let Some(player) = &mut self.player else { return };
        let ready = player.phase == PlaybackPhase::Ready;
        let ended = player.phase == PlaybackPhase::Ended;
        let previous_menu = player.menu;
        let mut anchor = None;
        let chrome = Rect::from_min_size(
            Pos2::new(screen.right() - 207.0, screen.top() + 12.0), Vec2::new(182.0, 36.0),
        );
        let (active, near_controls) = ctx.input(|i| {
            let active = i.pointer.delta() != Vec2::ZERO || i.pointer.any_pressed()
                || i.raw_scroll_delta != Vec2::ZERO || !i.keys_down.is_empty();
            let near_controls = i.pointer.hover_pos().is_some_and(|point| {
                point.y >= bottom - 191.0 || (!fullscreen && chrome.expand(10.0).contains(point))
            });
            (active, near_controls)
        });
        if active || !player.last_activity.is_finite() {
            player.last_activity = now;
        }
        let idle = now - player.last_activity;
        let visible = !player.auto_hide || !player.playing || !ready || previous_menu.is_some()
            || near_controls || idle < CONTROL_IDLE;
        if visible && player.auto_hide && player.playing && idle < CONTROL_IDLE {
            ctx.request_repaint_after(Duration::from_secs_f64(CONTROL_IDLE - idle));
        }
        let alpha = ctx.animate_bool_with_time_and_easing(egui::Id::new("player-overlay"), visible,
            if visible { 0.16 } else { 0.22 }, egui::emath::easing::cubic_out);
        if !visible && alpha <= 0.0 {
            ctx.set_cursor_icon(egui::CursorIcon::None);
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::BLACK))
            .show(ctx, |ui| {
                ui.set_clip_rect(screen);
                if let Some(engine) = &player.engine {
                    engine.paint(ui.painter(), screen);
                } else if let (Some(item), Some(texture)) = (&preview, preview_texture) {
                    draw_image(ui, screen, Some(texture), &theme::palette(true), item, 0.0);
                }

                if danmaku_prefs.enabled && player.show_danmaku && !player.danmaku.is_empty() {
                    let pos = if danmaku_prefs.compatible { (player.pos * 30.0).floor() / 30.0 } else { player.pos };
                    let speed = f64::from(danmaku_prefs.speed) / if danmaku_prefs.sync_speed {
                        1.0
                    } else { player.speed.max(0.25) };
                    let from = player.danmaku.partition_point(|comment| comment.time <= pos - (7.0 / speed).max(4.0));
                    let until = player.danmaku.partition_point(|comment| comment.time <= pos);
                    let painter = ui.painter().with_clip_rect(Rect::from_min_max(
                        screen.min + Vec2::new(0.0, 50.0),
                        Pos2::new(screen.right(), bottom - 205.0),
                    ));
                    for (idx, comment) in player.danmaku[from..until].iter().enumerate().take(36) {
                        let elapsed = (pos - comment.time) as f32;
                        let limit = match comment.mode {
                            4 => danmaku_prefs.bottom_lines, 5 => danmaku_prefs.top_lines, _ => danmaku_prefs.scroll_lines,
                        };
                        let travel = f64::from(elapsed) * speed / 7.0;
                        if limit == 0 || (comment.mode != 4 && comment.mode != 5 && travel > 1.0) {
                            continue;
                        }
                        let size = comment.size * danmaku_prefs.scale;
                        let lane = ((from + idx) % usize::from(limit)) as f32 * (size + f32::from(danmaku_prefs.line_spacing));
                        let font = if danmaku_prefs.bold { FontId::new(size, egui::FontFamily::Name("home-display".into())) }
                            else { FontId::proportional(size) };
                        let opacity = f32::from(danmaku_prefs.opacity) / 100.0;
                        let color = comment.color.gamma_multiply(opacity);
                        let galley = painter.layout_no_wrap(comment.text.clone(), font.clone(), color);
                        let width = galley.size().x;
                        let (x, y) = match comment.mode {
                            4 | 5 if elapsed > 4.0 => continue,
                            4 => (screen.center().x - width * 0.5, bottom - 238.0 - lane),
                            5 => (screen.center().x - width * 0.5, screen.top() + 58.0 + lane),
                            6 => (screen.left() - width + (screen.width() + width) * travel as f32,
                                  screen.top() + 58.0 + lane),
                            _ => (screen.right() - (screen.width() + width) * travel as f32,
                                  screen.top() + 58.0 + lane),
                        };
                        let at = Pos2::new(x, y);
                        let shadow = painter.layout_no_wrap(
                            comment.text.clone(), font, Color32::BLACK.gamma_multiply(opacity),
                        );
                        painter.galley(at + Vec2::splat(1.0), shadow, Color32::BLACK.gamma_multiply(opacity));
                        painter.galley(at, galley, color);
                    }
                }

                let gesture = ui.interact(
                    Rect::from_min_max(screen.min + Vec2::new(0.0, if fullscreen { 0.0 } else { 44.0 }),
                        Pos2::new(screen.right(), bottom - if visible { 199.0 } else { 0.0 })),
                    ui.id().with("video"), Sense::click(),
                );
                if previous_menu.is_none() && gesture.double_clicked() {
                    player.pending_click = None;
                    actions.push(Action::ToggleFullscreen);
                } else if previous_menu.is_none() && gesture.clicked() && (ready || ended) {
                    player.pending_click = Some(now + DOUBLE_CLICK_DELAY);
                }
                if let Some(at) = player.pending_click {
                    if now >= at {
                        player.pending_click = None;
                        actions.push(Action::TogglePlayer);
                    } else {
                        ctx.request_repaint_after(Duration::from_secs_f64(at - now));
                    }
                }

                let status = match &player.phase {
                    PlaybackPhase::Loading | PlaybackPhase::Starting => Some(if player.item.progress_ticks > 0 {
                        "Resuming playback…"
                    } else {
                        "Loading video…"
                    }),
                    PlaybackPhase::Ready if player.buffering => Some("Buffering…"),
                    PlaybackPhase::Ready if player.target.sample => Some("Offline preview"),
                    PlaybackPhase::Ended => Some("Playback ended"),
                    PlaybackPhase::Failed(error) => Some(error.as_str()),
                    _ => None,
                };
                if let Some(message) = status {
                    let busy = matches!(player.phase, PlaybackPhase::Loading | PlaybackPhase::Starting)
                        || player.buffering;
                    let center = screen.center();
                    if busy {
                        ui.put(Rect::from_center_size(center - Vec2::new(0.0, 13.0), Vec2::splat(32.0)),
                            egui::Spinner::new().size(30.0).color(SURFACE_WHITE));
                    }
                    ui.painter().text(center + Vec2::new(0.0, if busy { 20.0 } else { 0.0 }),
                        Align2::CENTER_CENTER, i18n::t(message), FontId::proportional(14.0), SURFACE_WHITE);
                }

                if let Some((notice, expires)) = &player.notice {
                    if now < *expires {
                        let painter = ui.painter();
                        let text = painter.layout_no_wrap(notice.clone(), FontId::proportional(14.0), SURFACE_WHITE);
                        let rect = Rect::from_center_size(screen.center(), text.size() + Vec2::new(28.0, 16.0));
                        painter.rect_filled(rect, rect.height() * 0.5, SURFACE_DARK);
                        painter.galley(rect.center() - text.size() * 0.5, text, SURFACE_WHITE);
                        ctx.request_repaint_after(Duration::from_secs_f64(expires - now));
                    }
                }
                if intro_button && ready && player.seekable && !player.intro_skipped {
                    if let Some((_, end)) = player.intro_range.filter(|(start, end)| (*start..*end).contains(&player.pos)) {
                        let area = Rect::from_min_size(Pos2::new(right - 134.0, bottom - 236.0), Vec2::new(134.0, 38.0));
                        if ui.put(area, egui::Button::new(i18n::bilingual("Skip intro", "跳过片头")).rounding(19.0)).clicked() {
                            actions.push(Action::SkipIntro(end));
                        }
                    }
                }
                if alpha <= 0.0 {
                    return;
                }
                ui.set_opacity(alpha);
                if !visible {
                    ui.disable();
                }
                let mut scrim = egui::Mesh::default();
                for (fraction, shade) in [(0.0, 0), (0.45, 95), (0.8, 177), (1.0, 217)] {
                    let y = bottom - 240.0 * (1.0 - fraction);
                    scrim.colored_vertex(Pos2::new(screen.left(), y), Color32::from_black_alpha(shade));
                    scrim.colored_vertex(Pos2::new(screen.right(), y), Color32::from_black_alpha(shade));
                }
                for i in [0, 2, 4] {
                    scrim.add_triangle(i, i + 1, i + 2);
                    scrim.add_triangle(i + 1, i + 3, i + 2);
                }
                ui.painter().add(egui::Shape::mesh(scrim));

                let title = if player.item.series_name.is_empty() { &player.item.name } else { &player.item.series_name };
                let source = player.source_label.as_deref().unwrap_or(&player.item.name);
                let subtitle = if player.item.kind == "Episode" {
                    format!("S{}:E{} - {source}", player.item.parent_index.unwrap_or(1), player.item.index.unwrap_or(1))
                } else if source == title {
                    player.item.subtitle()
                } else {
                    source.to_owned()
                };
                for (caption, y, size, strong) in [
                    (title.as_str(), bottom - 175.0, 20.0, true),
                    (subtitle.as_str(), bottom - 147.0, 14.0, false),
                ] {
                    let pos = Pos2::new(left, y);
                    let rect = Rect::from_min_size(pos - Vec2::new(0.0, size), Vec2::new(right - left, size * 2.0));
                    let mut job = egui::text::LayoutJob::default();
                    job.wrap.max_width = rect.width();
                    job.wrap.max_rows = 1;
                    job.wrap.break_anywhere = true;
                    let color = if strong { SURFACE_WHITE } else { OVERLAY_SECONDARY };
                    let text = RichText::new(caption).size(size).color(color);
                    let text = if strong { text.family(egui::FontFamily::Name("home-display".into())) } else { text };
                    text.append_to(&mut job, ui.style(), egui::FontSelection::Default, Align::Center);
                    let galley = ui.painter().layout_job(job);
                    let text_rect = Rect::from_min_size(pos - Vec2::new(0.0, galley.size().y / 2.0), galley.size());
                    let truncated = galley.elided;
                    ui.painter().galley(text_rect.min, galley, color);
                    if truncated {
                        ui.interact(text_rect, ui.id().with(("player-caption", strong)), Sense::hover())
                            .on_hover_text(caption);
                    }
                }

                let rail = Rect::from_min_max(
                    Pos2::new(left, bottom - RAIL_FROM_BOTTOM - 3.0),
                    Pos2::new(right, bottom - RAIL_FROM_BOTTOM + 3.0),
                );
                let can_seek = ready && player.seekable && player.duration > 0.0;
                let seek = ui.interact(rail.expand2(Vec2::new(8.0, 10.0)), ui.id().with("player-seek"),
                    if can_seek { Sense::click_and_drag() } else { Sense::hover() });
                let mut shown = player.pending_seek.unwrap_or(player.pos);
                let position_at = |x: f32| f64::from(((x - rail.left()) / rail.width()).clamp(0.0, 1.0)) * player.duration;
                if let Some(pointer) = seek.interact_pointer_pos() {
                    if seek.dragged() || seek.drag_stopped() || seek.clicked() {
                        shown = position_at(pointer.x);
                    }
                    if seek.clicked() || seek.drag_stopped() {
                        actions.push(Action::SeekTo(shown));
                    }
                }
                let painter = ui.painter();
                let progress = |secs: f64| left + rail.width() *
                    if player.duration > 0.0 { (secs / player.duration).clamp(0.0, 1.0) as f32 } else { 0.0 };
                painter.rect_filled(rail, 3.0, Color32::from_gray(76));
                for (end, color) in [(player.buffer_pos, Color32::from_gray(153)), (shown, SURFACE_WHITE)] {
                    if end > 0.0 {
                        painter.rect_filled(rail.with_max_x(progress(end)), 3.0, color);
                    }
                }
                if show_chapters {
                    for chapter in player.chapters.iter().filter(|chapter| chapter.position_ticks > 0) {
                        let x = progress(chapter.position_ticks as f64 / TICKS);
                        if x > rail.left() && x < rail.right() {
                            painter.line_segment([Pos2::new(x, rail.top() - 1.0), Pos2::new(x, rail.bottom() + 1.0)],
                                Stroke::new(1.5_f32, Color32::from_gray(200)));
                        }
                    }
                }
                if !danmaku_prefs.hide_heatmap && danmaku_prefs.enabled && player.show_danmaku && !player.danmaku.is_empty() {
                    for bucket in 0..60 {
                        let begin = player.duration * f64::from(bucket) / 60.0;
                        let end = player.duration * f64::from(bucket + 1) / 60.0;
                        let count = player.danmaku.partition_point(|comment| comment.time < end)
                            - player.danmaku.partition_point(|comment| comment.time < begin);
                        let area = Rect::from_min_max(Pos2::new(progress(begin), rail.top() - (count as f32).min(12.0)),
                            Pos2::new(progress(end), rail.top() - 1.0));
                        if area.is_positive() {
                            painter.rect_filled(area, 0.0, Color32::from_white_alpha(75));
                        }
                    }
                }
                painter.circle_filled(Pos2::new(progress(shown), rail.center().y), 8.0, SURFACE_WHITE);
                for (x, align, time) in [(left, Align2::LEFT_CENTER, shown), (right, Align2::RIGHT_CENTER, player.duration)] {
                    painter.text(Pos2::new(x, bottom - 92.0), align, playback_time(time),
                        FontId::monospace(13.0), OVERLAY_SECONDARY);
                }
                if can_seek {
                    seek.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_ui(|ui| {
                        if let Some(thumbnail) = thumbnail {
                            ui.image((thumbnail, Vec2::new(192.0, 108.0)));
                        }
                        if let Some(at) = ui.input(|i| i.pointer.hover_pos()) {
                            ui.label(playback_time(position_at(at.x)));
                        }
                    });
                }

                let current = player.queue.iter().position(|ep| ep.id == player.item.id).unwrap_or(0);
                let prev = current.checked_sub(1);
                let next = (current + 1 < player.queue.len()).then_some(current + 1);
                let toggle_icon = if player.playing && ready { Icon::Pause } else { Icon::Play };
                for (x, icon, label, action, enabled) in [
                    (left + 10.0, Icon::Previous, "Previous episode", Action::PlayFromQueue(prev.unwrap_or(0)), prev.is_some()),
                    (left + 60.0, toggle_icon, if ended { "Replay" } else if player.playing { "Pause" } else { "Play" }, Action::TogglePlayer, ready || ended),
                    (left + 110.0, Icon::Next, "Next episode", Action::PlayFromQueue(next.unwrap_or(0)), next.is_some()),
                    (left + 158.0, if player.muted || player.volume == 0 { Icon::Muted } else { Icon::Volume },
                        if player.muted { "Unmute" } else { "Mute" }, Action::TogglePlaybackMute, ready),
                ] {
                    let rect = Rect::from_center_size(Pos2::new(x, toolbar_y), Vec2::splat(44.0));
                    if icon_button(ui, rect, icon, label, enabled, false).clicked() {
                        actions.push(action);
                    }
                }

                let volume = Rect::from_center_size(Pos2::new(left + 234.0, toolbar_y), Vec2::new(92.0, 5.0));
                let slider = ui.interact(volume.expand2(Vec2::new(9.0, 8.0)), ui.id().with("player-volume"),
                    if ready { Sense::click_and_drag() } else { Sense::hover() });
                let volume_value = if player.muted { 0 } else { player.volume };
                if let Some(pointer) = slider.interact_pointer_pos() {
                    let commit = slider.clicked() || slider.drag_stopped();
                    if commit || slider.dragged() {
                        let value = (((pointer.x - volume.left()) / volume.width()).clamp(0.0, 1.0) * 100.0).round() as u8;
                        if value != player.volume || player.muted || commit {
                            actions.push(Action::SetPlaybackVolume { level: value, commit });
                            if player.muted { actions.push(Action::TogglePlaybackMute); }
                        }
                    }
                }
                let fill_end = volume.left() + volume.width() * f32::from(volume_value) / 100.0;
                ui.painter().rect_filled(volume, 2.5, Color32::from_gray(88));
                ui.painter().rect_filled(volume.with_max_x(fill_end), 2.5, SURFACE_WHITE);
                ui.painter().circle_filled(Pos2::new(fill_end, toolbar_y), 8.0, SURFACE_WHITE);
                slider.on_hover_text(format!("{} {volume_value}%", i18n::t("Volume")));

                for (index, (icon, label, menu, enabled)) in [
                    (if fullscreen { Icon::Windowed } else { Icon::Fullscreen },
                     if fullscreen { "Exit fullscreen" } else { "Fullscreen" }, None, true),
                    (Icon::Playlist, "Playlist", Some(PlayerMenu::Playlist), true),
                    (Icon::Settings, "Settings", Some(PlayerMenu::Settings), true),
                    (Icon::Danmaku, "Danmaku", Some(PlayerMenu::Danmaku), ready && danmaku_prefs.enabled),
                    (Icon::Subtitles, "Subtitle track", Some(PlayerMenu::Subtitles), ready && player.engine.is_some()),
                    (Icon::Audio, "Audio track", Some(PlayerMenu::Audio), ready && player.engine.is_some()),
                    (Icon::Speed(player.speed), "Playback speed", Some(PlayerMenu::Speed), ready),
                ].into_iter().enumerate() {
                    let x = right - 10.0 - index as f32 * 50.0;
                    let rect = Rect::from_center_size(Pos2::new(x, toolbar_y), Vec2::new(44.0, 40.0));
                    let opened = menu.is_some() && player.menu == menu;
                    if icon_button(ui, rect, icon, label, enabled, opened).clicked() {
                        if let Some(menu) = menu {
                            player.menu = (!opened).then_some(menu);
                        } else {
                            actions.push(Action::ToggleFullscreen);
                        }
                    }
                    if menu.is_some() && player.menu == menu {
                        anchor = Some(rect.right_center() - Vec2::new(0.0, 31.0));
                    }
                }

                if !fullscreen {
                    let drag = Rect::from_min_max(screen.min, Pos2::new(chrome.left() - 8.0, screen.top() + 40.0));
                    let surface = ui.interact(drag, ui.id().with("player-move"), Sense::click_and_drag());
                    if surface.double_clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                    } else if surface.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    ui.painter().rect_filled(chrome, 18.0, SURFACE_DARK);
                    for (index, (icon, label, active)) in [
                        (Icon::Pin, "Always on top", player.topmost),
                        (Icon::Minimize, "Minimize", false),
                        (if maximized { Icon::Restore } else { Icon::Maximize }, if maximized { "Restore" } else { "Maximize" }, false),
                        (Icon::Close, "Close", false),
                    ].into_iter().enumerate() {
                        let at = Pos2::new(chrome.left() + 25.0 + index as f32 * 44.0, chrome.center().y);
                        let area = Rect::from_center_size(at, Vec2::new(42.0, 34.0));
                        if icon_button(ui, area, icon, label, true, active).clicked() {
                            match icon {
                                Icon::Pin => actions.push(Action::TogglePlayerTopmost),
                                Icon::Minimize => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
                                Icon::Maximize | Icon::Restore => ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized)),
                                Icon::Close => actions.push(Action::ClosePlayer),
                                _ => unreachable!(),
                            }
                        }
                    }
                }
                if player.show_cache_speed && player.cache_speed > 0.0 {
                    let (number, unit) = if player.cache_speed >= 1_048_576.0 {
                        (player.cache_speed / 1_048_576.0, "MB/s")
                    } else {
                        (player.cache_speed / 1024.0, "KB/s")
                    };
                    ui.painter().text(Pos2::new(chrome.right(), screen.top() + if fullscreen { 17.0 } else { 58.0 }),
                        Align2::RIGHT_TOP, format!("{number:.1} {unit}"), FontId::monospace(13.0), OVERLAY_SECONDARY);
                }
            });

        if let (Some(menu), Some(anchor)) = (player.menu, anchor) {
            let title = match menu {
                PlayerMenu::Speed => "Playback speed",
                PlayerMenu::Audio => "Audio track",
                PlayerMenu::Subtitles => "Subtitle track",
                PlayerMenu::Danmaku => "Danmaku",
                PlayerMenu::Settings => "Settings",
                PlayerMenu::Playlist => "Playlist",
            };
            let options: Vec<(String, bool, Option<Action>)> = match menu {
                PlayerMenu::Speed => [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0].into_iter()
                    .map(|speed| (speed_text(speed), player.speed == speed, Some(Action::SetPlaybackSpeed(speed))))
                    .collect(),
                PlayerMenu::Audio | PlayerMenu::Subtitles => {
                    let kind = if menu == PlayerMenu::Audio { TrackKind::Audio } else { TrackKind::Subtitle };
                    let mut choices: Vec<_> = player.tracks.iter().filter(|track| track.kind == kind)
                        .map(|track| (track.label.clone(), track.selected, Some(Action::SelectPlaybackTrack(kind, Some(track.id)))))
                        .collect();
                    if kind == TrackKind::Subtitle {
                        choices.insert(0, (i18n::t("Turn off subtitles").into_owned(),
                            !choices.iter().any(|(_, selected, _)| *selected), Some(Action::SelectPlaybackTrack(kind, None))));
                        choices.insert(1, (i18n::t("Search subtitles").into_owned(), false,
                            Some(Action::OpenCaptionSearch(player_captions::Source::SubtitleCat))));
                        choices.insert(2, (i18n::t("Open subtitle file").into_owned(), false, Some(Action::LoadLocalSubtitles)));
                        if enable_secondary {
                            let name = i18n::bilingual("Secondary subtitle", "次字幕");
                            choices.push((format!("{name} · {}", i18n::bilingual("Off", "关闭")),
                                player.secondary_id.is_none(), Some(Action::SelectSecondarySubtitle(None))));
                            choices.extend(player.tracks.iter().filter(|track| track.kind == TrackKind::Subtitle
                                && (!track.selected || player.secondary_id == Some(track.id))).map(|track| (
                                    format!("{name} · {}", track.label), player.secondary_id == Some(track.id),
                                    Some(Action::SelectSecondarySubtitle(Some(track.id))),
                                )));
                        }
                    }
                    choices
                }
                PlayerMenu::Danmaku => [
                    ("Search danmaku", false, Some(Action::OpenCaptionSearch(player_captions::Source::Danmaku))),
                    ("Import local danmaku XML", false, Some(Action::LoadDanmaku)),
                    ("Show danmaku", player.show_danmaku && !player.danmaku.is_empty(),
                     (!player.danmaku.is_empty()).then_some(Action::ToggleDanmaku)),
                ].into_iter().map(|(label, selected, action)| (i18n::t(label).into_owned(), selected, action)).collect(),
                PlayerMenu::Settings => [
                    ("Fit video", !player.fill_video, Action::SetVideoFill(false)),
                    ("Fill window", player.fill_video, Action::SetVideoFill(true)),
                    ("Show cache speed", player.show_cache_speed, Action::TogglePlayerCacheSpeed),
                    ("Automatically hide controls", player.auto_hide, Action::TogglePlayerAutoHide),
                ].into_iter().map(|(label, selected, action)| (i18n::t(label).into_owned(), selected, Some(action))).collect(),
                PlayerMenu::Playlist => player.queue.iter().enumerate().map(|(index, item)| {
                    let current = player.item.id == item.id;
                    let label = if item.kind == "Episode" {
                        format!("S{}:E{}  {}", item.parent_index.unwrap_or(1), item.index.unwrap_or(1), item.name)
                    } else { item.name.clone() };
                    (label, current, (!current).then_some(Action::PlayFromQueue(index)))
                }).collect(),
            };
            let title = i18n::t(title);
            let min_width = if menu == PlayerMenu::Speed { 144.0 } else { 224.0 };
            // HillsPlaybackOptionButton sizes menus from the labels and clips them to the window.
            let width = ctx.fonts(|fonts| {
                options.iter().map(|(label, _, _)| {
                    fonts.layout_no_wrap(label.clone(), FontId::proportional(13.0), SURFACE_WHITE).size().x + 62.0
                }).fold(min_width, f32::max)
            }).min(432.0).min(screen.width() - 16.0);
            let opening = previous_menu != Some(menu);
            let max_height = (anchor.y - screen.top() - 87.0).clamp(52.0, 322.0);
            let popup = egui::Area::new(egui::Id::new(("player-menu", menu)))
                .order(egui::Order::Foreground)
                .pivot(Align2::RIGHT_BOTTOM)
                .fixed_pos(anchor)
                .constrain_to(screen.with_max_y(anchor.y + 1.0))
                .default_size(Vec2::new(width, 45.0 + max_height.min(34.0 * options.len().max(1) as f32)))
                .fade_in(false)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style())
                        .fill(Color32::from_rgb(24, 28, 34))
                        .stroke(Stroke::new(1.0_f32, Color32::from_white_alpha(33)))
                        .rounding(14.0)
                        .inner_margin(9.0)
                        .show(ui, |ui| {
                            ui.set_width(width - 18.0);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.horizontal(|ui| {
                                ui.add_space(10.0);
                                ui.label(RichText::new(title).size(13.0).color(OVERLAY_SECONDARY));
                            });
                            ui.add_space(5.0);
                            if options.is_empty() || (menu == PlayerMenu::Playlist && player.queue_loading) {
                                let text = if options.is_empty() { "No tracks available" } else { "Loading episodes…" };
                                ui.label(RichText::new(i18n::t(text)).size(13.0).color(Color32::from_gray(182)));
                            }
                            ScrollArea::vertical()
                                .max_height(max_height)
                                .auto_shrink([false, true])
                                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                                .show(ui, |ui| {
                                    for (label, selected, action) in options {
                                        let (rect, row) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
                                        row.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &label));
                                        let painter = ui.painter();
                                        if row.hovered() || row.has_focus() || selected {
                                            let opacity = if row.is_pointer_button_down_on() { 42 } else if selected { 31 } else { 18 };
                                            painter.rect_filled(rect, 8.0, Color32::from_white_alpha(opacity));
                                        }
                                        let color = if action.is_some() || selected { SURFACE_WHITE } else { OVERLAY_SECONDARY };
                                        let text = theme::clip_px(painter, &label, 13.0, rect.width() - 43.0, color);
                                        painter.text(rect.left_center() + Vec2::new(11.0, 0.0), Align2::LEFT_CENTER,
                                            &text, FontId::proportional(13.0), color);
                                        if selected {
                                            theme::icon(ui, "check", rect.right_center() - Vec2::new(17.0, 0.0), 15.0, SURFACE_WHITE);
                                        }
                                        let row = if text == label { row } else { row.on_hover_text(label) };
                                        if opening && selected {
                                            row.scroll_to_me(Some(Align::Center));
                                        }
                                        if row.clicked() {
                                            if let Some(action) = action {
                                                actions.push(action);
                                            }
                                            player.menu = None;
                                        }
                                    }
                                });
                        });
                });
            if !opening && popup.response.clicked_elsewhere() {
                player.menu = None;
            }
        }
    }
}

fn playback_time(secs: f64) -> String {
    let secs = secs.max(0.0) as u64;
    format!("{:02}:{:02}:{:02}", secs / 3600, (secs / 60) % 60, secs % 60)
}

fn speed_text(speed: f64) -> String {
    if speed.fract() == 0.0 { format!("{speed:.1}x") } else { format!("{speed}x") }
}

fn icon_button(ui: &Ui, rect: Rect, icon: Icon, label: &str, enabled: bool, active: bool) -> Response {
    use Icon::*;
    let label = i18n::t(label);
    let response = ui.interact(rect, ui.id().with(("player-icon", label.as_ref())),
        if enabled { Sense::click() } else { Sense::hover() });
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label.as_ref()));
    let painter = ui.painter();
    let c = rect.center();
    let play = matches!(icon, Play | Pause);
    let over = enabled && (response.hovered() || response.has_focus());
    let color = if play { SURFACE_DARK }
        else if enabled { SURFACE_WHITE } else { Color32::from_gray(112) };
    let pressed = enabled && response.is_pointer_button_down_on();
    if play {
        let fill = if !enabled { Color32::from_gray(120) } else if over { Color32::WHITE } else { SURFACE_WHITE };
        painter.circle_filled(c, if pressed { 21.0 } else { 22.0 }, fill);
    } else if over || active {
        let opacity = if pressed { 38 } else if active { 30 } else { 22 };
        painter.rect_filled(rect.shrink(2.0), 18.0, Color32::from_white_alpha(opacity));
    }
    let at = |x, y| c + Vec2::new(x, y);
    let line = |x1, y1, x2, y2, width: f32| painter.line_segment([at(x1, y1), at(x2, y2)], Stroke::new(width, color));
    let fill = |x, y, w, h, r| painter.rect_filled(Rect::from_min_size(at(x, y), Vec2::new(w, h)), r, color);
    let tri = |a: (f32, f32), b: (f32, f32), d: (f32, f32)| {
        painter.add(egui::Shape::convex_polygon(vec![at(a.0, a.1), at(b.0, b.1), at(d.0, d.1)], color, Stroke::NONE))
    };
    match icon {
        Play => { tri((-4.0, -7.5), (8.0, 0.0), (-4.0, 7.5)); }
        Pause => { fill(-6.0, -7.0, 4.0, 14.0, 0.7); fill(2.0, -7.0, 4.0, 14.0, 0.7); }
        Previous | Next => {
            let d = if matches!(icon, Next) { 1.0 } else { -1.0 };
            tri((-5.0*d, -5.8), (3.6*d, 0.0), (-5.0*d, 5.8));
            line(5.7*d, -5.5, 5.7*d, 5.5, 1.6);
        }
        Volume | Muted => {
            painter.add(egui::Shape::convex_polygon(
                vec![at(-9.0,-3.2), at(-5.6,-3.2), at(-1.5,-7.5), at(-1.5,7.5), at(-5.6,3.2), at(-9.0,3.2)],
                color, Stroke::NONE));
            if matches!(icon, Volume) {
                for r in [5.0, 8.5] {
                    let arc = (-3..=3).map(|i| {
                        let angle = i as f32 * 0.31;
                        at(r * angle.cos() - 1.6, r * angle.sin())
                    }).collect();
                    painter.add(egui::Shape::line(arc, Stroke::new(1.45_f32, color)));
                }
            } else {
                line(2.0,-3.0,8.0,3.0,1.6); line(8.0,-3.0,2.0,3.0,1.6);
            }
        }
        Speed(speed) => { painter.text(c, Align2::CENTER_CENTER, speed_text(speed), FontId::proportional(13.0), color); }
        Audio => {
            painter.circle_filled(at(-2.5, 5.0), 3.0, color);
            fill(-1.0, -8.0, 2.7, 13.0, 0.9); fill(0.0, -8.0, 5.0, 3.2, 0.7);
        }
        Subtitles | Danmaku => {
            fill(-8.5, -7.0, 17.0, 14.0, 2.4);
            if matches!(icon, Subtitles) {
                painter.text(c, Align2::CENTER_CENTER, "CC", FontId::proportional(9.0), SURFACE_DARK);
            } else {
                for (x, y, w) in [(-5.0, -3.0, 7.0), (-2.0, 0.0, 7.0), (-5.0, 3.0, 9.0)] {
                    painter.line_segment([at(x, y), at(x + w, y)], Stroke::new(1.25_f32, SURFACE_DARK));
                }
            }
        }
        Settings => {
            let mut gear = egui::Mesh::default();
            for n in 0..32 {
                let angle = n as f32 * std::f32::consts::TAU / 32.0;
                let direction = Vec2::angled(angle);
                let radius = if matches!(n % 4, 0 | 1) { 10.0 } else { 7.4 };
                gear.colored_vertex(c + direction * radius, color);
                gear.colored_vertex(c + direction * 3.25, color);
                let a = n * 2;
                let b = (a + 2) % 64;
                gear.add_triangle(a, b, a + 1);
                gear.add_triangle(a + 1, b, b + 1);
            }
            painter.add(egui::Shape::mesh(gear));
        }
        Playlist => {
            for (y, end) in [(-6.0, 2.0), (-2.3, 0.5), (1.4, -1.0)] {
                line(-8.0, y, end, y, 1.65);
            }
            tri((3.4,-1.0), (9.2,2.9), (3.4,6.8));
        }
        Fullscreen | Windowed => {
            let (a, b, d) = if matches!(icon, Fullscreen) { (8.0, 3.5, 8.0) } else { (3.5, 8.0, 3.5) };
            for (x,y) in [(-1.0,-1.0), (1.0,-1.0), (1.0,1.0), (-1.0,1.0)] {
                line(a*x, d*y, b*x, d*y, 1.7); line(a*x, d*y, a*x, b*y, 1.7);
            }
        }
        Pin => {
            line(-2.4,-4.0,2.4,-4.0,1.0); line(-1.4,-4.0,-1.4,0.4,1.0);
            line(1.4,-4.0,1.4,0.4,1.0); line(-3.0,0.9,3.0,0.9,1.0);
            line(0.0,1.0,0.0,4.4,1.0);
        }
        Minimize => { line(-4.6,0.0,4.6,0.0,1.0); }
        Maximize | Restore => {
            let square = Rect::from_center_size(c + if matches!(icon, Restore) { Vec2::new(-0.9, 0.9) } else { Vec2::ZERO }, Vec2::splat(8.1));
            if matches!(icon, Restore) {
                line(-2.3,-5.4,5.4,-5.4,1.0); line(5.4,-5.4,5.4,2.3,1.0);
            }
            painter.rect_stroke(square, 0.5, Stroke::new(1.0_f32, color));
        }
        Close => { line(-4.4,-4.4,4.4,4.4,1.1); line(-4.4,4.4,4.4,-4.4,1.1); }
    }
    response.on_hover_cursor(if enabled { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Default })
        .on_hover_text(label)
}

pub(super) fn open_player_file(owner: Option<isize>, subtitles: bool) -> Option<PathBuf> {
    let label = i18n::t(if subtitles { "Open subtitle file" } else { "Import local danmaku XML" });
    let patterns = if subtitles { "*.srt;*.ass;*.ssa;*.vtt;*.sub;*.sup" } else { "*.xml" };
    crate::platform::pick_file(owner, &label, patterns, None)
}

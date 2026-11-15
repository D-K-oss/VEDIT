//! Central preview panel — DaVinci-style transport + status.
//!
//! * Video well with overlay badges (resolution pill, fit %).
//! * Big timecode readout + duration.
//! * Full transport: Mark In/Out, jump start/end, frame step, play, LOOP.

use eframe::egui::{
    self, Align2, Button, CentralPanel, Color32, ComboBox, FontId, Pos2, Rect, RichText, Sense,
    Vec2,
};

use vedit_core::media::format_timecode;

use crate::app::{PreviewQuality, VeditApp};
use crate::theme::Palette;

pub(crate) fn preview(app: &mut VeditApp, ctx: &egui::Context) {
    CentralPanel::default().show(ctx, |ui| {
        // ffmpeg warning banner
        if app.locator.is_none() {
            egui::Frame::default()
                .fill(Color32::from_rgb(44, 28, 24))
                .stroke(egui::Stroke::new(1.0_f32, Palette::DANGER))
                .inner_margin(8.0)
                .rounding(4.0)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(
                                "ffmpeg / ffprobe not found — import, preview and export are disabled.\n\
                                 Put ffmpeg + ffprobe in an `ffmpeg/` folder next to vedit (or on PATH), then retry.",
                            )
                            .color(Palette::TEXT),
                        );
                        if ui.button("Retry").clicked() {
                            app.locator = vedit_core::ffmpeg::FfmpegLocator::discover().ok();
                            app.ffmpeg_version = app.locator.as_ref().map(|l| l.version());
                            if app.locator.is_some() {
                                app.notify("ffmpeg found — media features enabled");
                            }
                        }
                    });
                });
            ui.add_space(6.0);
        }

        // ---- Video well ----------------------------------------------------
        let transport_h = 78.0;
        let avail = ui.available_size() - Vec2::new(0.0, transport_h);
        let avail = Vec2::new(avail.x.max(40.0), avail.y.max(40.0));
        let (rect, resp) = ui.allocate_exact_size(avail, Sense::click_and_drag());
        let p = ui.painter();
        p.rect_filled(rect, 6.0, Palette::BG);

        let mut fit_pct = 100.0_f32;
        if let Some(tex) = &app.preview_tex {
            let tw = tex.size()[0] as f32;
            let th = tex.size()[1] as f32;
            if tw > 0.0 && th > 0.0 {
                let scale = (rect.width() / tw).min(rect.height() / th);
                fit_pct = scale * 100.0;
                let size = Vec2::new(tw * scale, th * scale);
                let dest = Rect::from_center_size(rect.center(), size);
                p.rect_filled(dest.expand(1.0), 3.0, Palette::LINE_SOFT);
                p.image(
                    tex.id(),
                    dest,
                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        } else {
            p.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Import media, then add clips to the timeline\n(Drag & drop files anywhere in the window)",
                FontId::proportional(14.0),
                Palette::TEXT_FAINT,
            );
        }

        // Overlay badges (top corners of the video well).
        let (w, h) = app.preview_size;
        if w > 0 {
            let res_label = format!("{}×{}", w, h);
            let gal = p.layout_no_wrap(res_label, FontId::monospace(10.0), Palette::TEXT);
            let sz = gal.size() + Vec2::new(10.0, 6.0);
            let pill = Rect::from_min_size(Pos2::new(rect.left() + 8.0, rect.top() + 8.0), sz);
            p.rect_filled(pill, 3.0, Color32::from_rgba_premultiplied(10, 10, 12, 215));
            p.galley(pill.left_top() + Vec2::new(5.0, 3.0), gal, Palette::TEXT);
        }
        {
            let fit_label = format!("{fit_pct:.0}%");
            let gal = p.layout_no_wrap(fit_label, FontId::monospace(10.0), Palette::TEXT_DIM);
            let sz = gal.size() + Vec2::new(10.0, 6.0);
            let pill = Rect::from_min_size(
                Pos2::new(rect.right() - sz.x - 8.0, rect.top() + 8.0),
                sz,
            );
            p.rect_filled(pill, 3.0, Color32::from_rgba_premultiplied(10, 10, 12, 215));
            p.galley(pill.left_top() + Vec2::new(5.0, 3.0), gal, Palette::TEXT_DIM);
        }
        // Loop badge while looping.
        if app.loop_playback {
            let gal = p.layout_no_wrap("⟳ LOOP".to_string(), FontId::proportional(10.0), Palette::ACCENT);
            let sz = gal.size() + Vec2::new(10.0, 6.0);
            let pill = Rect::from_min_size(
                Pos2::new(rect.left() + 8.0, rect.bottom() - sz.y - 8.0),
                sz,
            );
            p.rect_filled(pill, 3.0, Color32::from_rgba_premultiplied(10, 10, 12, 215));
            p.galley(pill.left_top() + Vec2::new(5.0, 3.0), gal, Palette::ACCENT);
        }

        // Double-click toggles playback (like a media player).
        if resp.double_clicked() {
            app.toggle_play();
        }

        // ---- Timecode + transport ------------------------------------------
        ui.add_space(6.0);
        let dur = app.project.duration();

        // Row 1: big TC + duration + quality/refresh on the right.
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format_timecode(app.playhead, app.project.fps))
                    .monospace()
                    .size(21.0)
                    .strong()
                    .color(Palette::TEXT),
            );
            ui.label(
                RichText::new(format!("/ {}", format_timecode(dur, app.project.fps)))
                    .monospace()
                    .size(12.0)
                    .color(Palette::TEXT_FAINT),
            );
            // In/Out summary.
            if app.mark_in.is_some() || app.mark_out.is_some() {
                ui.label(
                    RichText::new(format!(
                        "IN {}  OUT {}",
                        app.mark_in
                            .map(|t| format_timecode(t, app.project.fps))
                            .unwrap_or_else(|| "—".into()),
                        app.mark_out
                            .map(|t| format_timecode(t, app.project.fps))
                            .unwrap_or_else(|| "—".into()),
                    ))
                    .monospace()
                    .size(10.5)
                    .color(Palette::ACCENT),
                );
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new("Quality").small().color(Palette::TEXT_FAINT));
                ComboBox::from_id_salt("preview_quality")
                    .selected_text(app.preview_quality.label())
                    .width(80.0)
                    .show_ui(ui, |ui| {
                        for q in PreviewQuality::ALL {
                            ui.selectable_value(&mut app.preview_quality, q, q.label());
                        }
                    });
                if ui
                    .button("↻")
                    .on_hover_text("Refresh (re-render the current preview frame)")
                    .clicked()
                {
                    app.seek_inflight = false;
                    app.request_preview_frame();
                }
            });
        });

        // Row 2: transport buttons.
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let mark_in_active = app.mark_in.is_some();
            if ui
                .add(
                    Button::new(RichText::new("[ In").size(12.0))
                        .fill(if mark_in_active { Palette::ACCENT_SOFT } else { Palette::SURFACE }),
                )
                .on_hover_text("Mark In at the playhead (I)")
                .clicked()
            {
                app.mark_in_action();
            }
            if ui
                .add(
                    Button::new(RichText::new("Out ]").size(12.0))
                        .fill(if app.mark_out.is_some() { Palette::ACCENT_SOFT } else { Palette::SURFACE }),
                )
                .on_hover_text("Mark Out at the playhead (O)")
                .clicked()
            {
                app.mark_out_action();
            }
            if ui
                .add_enabled(
                    mark_in_active || app.mark_out.is_some(),
                    Button::new("✕").small(),
                )
                .on_hover_text("Clear In/Out points")
                .clicked()
            {
                app.clear_marks();
            }

            ui.separator();
            if ui.button("⏮").on_hover_text("Go to start (Home)").clicked() {
                app.seek_to(0.0);
            }
            if ui.button("⏴").on_hover_text("Frame back (←)").clicked() {
                app.step_frames(-1.0);
            }
            let play_label = if app.playing { "⏸" } else { "▶" };
            let play_btn = Button::new(
                RichText::new(play_label).size(15.0).strong().color(Palette::ACCENT_TEXT),
            )
            .fill(Palette::ACCENT)
            .min_size(Vec2::new(46.0, 26.0));
            if ui.add(play_btn).on_hover_text("Play / Pause (Space)").clicked() {
                app.toggle_play();
            }
            if ui.button("⏵").on_hover_text("Frame forward (→)").clicked() {
                app.step_frames(1.0);
            }
            if ui.button("⏭").on_hover_text("Go to end (End)").clicked() {
                app.seek_to(dur);
            }

            ui.separator();
            let loop_btn = Button::new(RichText::new("⟳ Loop").size(12.0))
                .fill(if app.loop_playback { Palette::ACCENT } else { Palette::SURFACE });
            if ui
                .add(loop_btn)
                .on_hover_text("Loop playback (L) — restart from the In point when the end is reached")
                .clicked()
            {
                app.loop_playback = !app.loop_playback;
            }
        });
    });
}

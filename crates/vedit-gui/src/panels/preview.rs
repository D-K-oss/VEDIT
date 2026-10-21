//! Central preview panel + transport controls.

use eframe::egui::{
    self, Align2, Button, CentralPanel, Color32, ComboBox, FontId, Pos2, Rect, RichText, Sense,
    Vec2,
};

use vedit_core::media::format_timecode;

use crate::app::{PreviewQuality, VeditApp};
use crate::theme::{mono, Palette};

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

        // Video area
        let transport_h = 52.0;
        let avail = ui.available_size() - Vec2::new(0.0, transport_h);
        let avail = Vec2::new(avail.x.max(40.0), avail.y.max(40.0));
        let (rect, resp) = ui.allocate_exact_size(avail, Sense::click_and_drag());
        let p = ui.painter();
        p.rect_filled(rect, 6.0, Palette::BG);

        if let Some(tex) = &app.preview_tex {
            let tw = tex.size()[0] as f32;
            let th = tex.size()[1] as f32;
            if tw > 0.0 && th > 0.0 {
                let scale = (rect.width() / tw).min(rect.height() / th);
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

        // Double-click toggles playback (like a media player).
        if resp.double_clicked() {
            app.toggle_play();
        }

        // Transport bar
        ui.add_space(6.0);
        let dur = app.project.duration();
        ui.horizontal(|ui| {
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
            if ui
                .add(play_btn)
                .on_hover_text("Space")
                .clicked()
            {
                app.toggle_play();
            }
            if ui.button("⏵").on_hover_text("Frame forward (→)").clicked() {
                app.step_frames(1.0);
            }
            if ui.button("⏭").on_hover_text("Go to end (End)").clicked() {
                app.seek_to(dur);
            }

            ui.add_space(10.0);
            ui.label(mono(
                format!(
                    "{} / {}",
                    format_timecode(app.playhead, app.project.fps),
                    format_timecode(dur, app.project.fps)
                ),
                Palette::TEXT,
            ));

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
                    .button("↻").on_hover_text("Refresh (re-render the current preview frame)")
                    .clicked()
                {
                    app.seek_inflight = false;
                    app.request_preview_frame();
                }
            });
        });
    });
}

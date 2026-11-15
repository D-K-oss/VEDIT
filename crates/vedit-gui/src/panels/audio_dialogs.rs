//! EQ + multiband compressor floating dialogs (one per track).
//!
//! The EQ is a real parametric: the curve is the summed magnitude response
//! of the actual biquads (`dsp::EqState::response_db`), handles are
//! draggable (x = frequency, y = gain), wheel = Q, double-click toggles the
//! band. The compressor exposes the 3 bands with live gain-reduction
//! metering from the audio thread.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, RichText, Sense, Stroke, Vec2, Window};

use vedit_core::dsp::BandKind;

use crate::app::VeditApp;
use crate::theme::Palette;

pub(crate) fn audio_dialogs(app: &mut VeditApp, ctx: &egui::Context) {
    if let Some(track_id) = app.eq_dialog_track {
        let mut open = true;
        let name = app.project.track(track_id).map(|t| t.name.clone()).unwrap_or_default();
        Window::new(format!("Equalizer — {name}"))
            .open(&mut open)
            .collapsible(false)
            .min_width(520.0)
            .show(ctx, |ui| eq_dialog(app, ui, track_id));
        if !open || app.project.track(track_id).is_none() {
            app.eq_dialog_track = None;
        }
    }
    if let Some(track_id) = app.comp_dialog_track {
        let mut open = true;
        let name = app.project.track(track_id).map(|t| t.name.clone()).unwrap_or_default();
        Window::new(format!("Multiband compressor — {name}"))
            .open(&mut open)
            .collapsible(false)
            .min_width(560.0)
            .show(ctx, |ui| comp_dialog(app, ui, track_id));
        if !open || app.project.track(track_id).is_none() {
            app.comp_dialog_track = None;
        }
    }
    if let Some(track_id) = app.dyn_dialog_track {
        let mut open = true;
        let name = app.project.track(track_id).map(|t| t.name.clone()).unwrap_or_default();
        Window::new(format!("Dynamics — {name}"))
            .open(&mut open)
            .collapsible(false)
            .min_width(540.0)
            .show(ctx, |ui| dynamics_dialog(app, ui, track_id));
        if !open || app.project.track(track_id).is_none() {
            app.dyn_dialog_track = None;
        }
    }
}

// ---------------------------------------------------------------------------
// EQ
// ---------------------------------------------------------------------------

const CURVE_MIN_DB: f64 = -18.0;
const CURVE_MAX_DB: f64 = 18.0;
const FREQ_MIN: f64 = 20.0;
const FREQ_MAX: f64 = 20_000.0;

fn freq_to_x(f: f64, w: f32) -> f32 {
    let t = (f.log10() - FREQ_MIN.log10()) / (FREQ_MAX.log10() - FREQ_MIN.log10());
    (t as f32).clamp(0.0, 1.0) * w
}

fn x_to_freq(x: f32, w: f32) -> f64 {
    let t = (x / w).clamp(0.0, 1.0) as f64;
    10f64.powf(FREQ_MIN.log10() + t * (FREQ_MAX.log10() - FREQ_MIN.log10()))
}

fn db_to_y(db: f64, h: f32) -> f32 {
    let t = ((db - CURVE_MIN_DB) / (CURVE_MAX_DB - CURVE_MIN_DB)).clamp(0.0, 1.0);
    (1.0 - t as f32) * h
}

fn y_to_db(y: f32, h: f32) -> f64 {
    let t = 1.0 - (y / h).clamp(0.0, 1.0);
    CURVE_MIN_DB + t as f64 * (CURVE_MAX_DB - CURVE_MIN_DB)
}

fn eq_dialog(app: &mut VeditApp, ui: &mut egui::Ui, track_id: u64) {
    let Some(mut eq) = app.project.track(track_id).map(|t| t.eq.clone()) else { return };
    let mut changed = false;

    // Header: enable + presets.
    ui.horizontal(|ui| {
        if ui.checkbox(&mut eq.enabled, "Enabled").changed() {
            changed = true;
        }
        ui.separator();
        ui.label(RichText::new("Preset").small().color(Palette::TEXT_DIM));
        let presets = vedit_core::dsp::eq_presets();
        let current = presets
            .iter()
            .position(|(_, s)| s.bands == eq.bands)
            .unwrap_or(usize::MAX);
        egui::ComboBox::from_id_salt("eq_preset")
            .selected_text(if current == usize::MAX { "Custom" } else { presets[current].0 })
            .width(150.0)
            .show_ui(ui, |ui| {
                for (i, (label, state)) in presets.iter().enumerate() {
                    if ui.selectable_label(current == i, *label).clicked() {
                        eq.bands = state.bands.clone();
                        eq.enabled = true;
                        changed = true;
                    }
                }
            });
        if ui.small_button("Reset").clicked() {
            eq.bands = vedit_core::dsp::default_eq_bands();
            changed = true;
        }
    });

    // Response curve.
    let size = Vec2::new(ui.available_width().max(480.0), 170.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    let w = rect.width();
    let h = rect.height();

    // Grid.
    let mut f = FREQ_MIN;
    while f <= FREQ_MAX + 1.0 {
        let x = freq_to_x(f, w);
        let major = matches!(f as u32, 20 | 100 | 1000 | 10_000 | 20_000);
        painter.line_segment(
            [Pos2::new(rect.left() + x, rect.top()), Pos2::new(rect.left() + x, rect.bottom())],
            Stroke::new(1.0_f32, if major { Palette::LINE } else { Color32::from_rgb(30, 30, 33) }),
        );
        if major {
            painter.text(
                Pos2::new(rect.left() + x + 3.0, rect.bottom() - 3.0),
                Align2::LEFT_BOTTOM,
                if f >= 1000.0 { format!("{}k", f / 1000.0) } else { format!("{f}") },
                FontId::monospace(8.0),
                Palette::TEXT_DIM,
            );
        }
        f *= 10.0;
    }
    for db in [-12.0, -6.0, 0.0, 6.0, 12.0] {
        let y = db_to_y(db, h);
        painter.line_segment(
            [Pos2::new(rect.left(), rect.top() + y), Pos2::new(rect.right(), rect.top() + y)],
            Stroke::new(1.0_f32, if db == 0.0 { Palette::LINE } else { Color32::from_rgb(30, 30, 33) }),
        );
        painter.text(
            Pos2::new(rect.left() + 3.0, rect.top() + y - 2.0),
            Align2::LEFT_TOP,
            format!("{db:+.0}"),
            FontId::monospace(8.0),
            Palette::TEXT_DIM,
        );
    }

    // Real-time FFT spectrum analyzer (Fairlight-style): the track bus is
    // tapped post-fader by the audio thread; bars are drawn under the EQ
    // curve while the transport is rolling.
    let track_index = app.project.tracks.iter().position(|t| t.id == track_id);
    if let Some(ti) = track_index {
        let samples = app.audio.track_spectrum(ti);
        let audible = samples.iter().any(|&s| s.abs() > 1e-5);
        if audible {
            let mut bins: Vec<f32> = Vec::new();
            vedit_core::dsp::fft_spectrum_db(&samples, &mut bins); // 1024 bins @ 23.4 Hz
            const SPEC_FLOOR_DB: f32 = -72.0;
            let bin_hz = 48_000.0 / vedit_core::dsp::FFT_SIZE as f64;
            // ~130 log-spaced pixel columns, max-bin per column.
            let cols = 130;
            let bar_w = w / cols as f32;
            let mut any_drawn = false;
            for c in 0..cols {
                let f0 = x_to_freq(c as f32 * bar_w, w);
                let f1 = x_to_freq((c + 1) as f32 * bar_w, w);
                let b0 = (f0 / bin_hz).floor().max(1.0) as usize;
                let b1 = ((f1 / bin_hz).ceil() as usize).clamp(b0 + 1, bins.len() - 1);
                let db = bins[b0..b1].iter().copied().fold(f32::MIN, f32::max);
                if db < SPEC_FLOOR_DB {
                    continue;
                }
                let t = ((db - SPEC_FLOOR_DB) / -SPEC_FLOOR_DB).clamp(0.0, 1.0);
                let bar_h = t * (h - 10.0);
                let x = rect.left() + c as f32 * bar_w;
                // Body + brighter cap (Fairlight green trace).
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        Pos2::new(x, rect.bottom() - bar_h),
                        Vec2::new(bar_w.max(1.2), bar_h),
                    ),
                    0.0,
                    Color32::from_rgba_premultiplied(58, 122, 74, 88),
                );
                painter.line_segment(
                    [
                        Pos2::new(x, rect.bottom() - bar_h),
                        Pos2::new(x + bar_w.max(1.2), rect.bottom() - bar_h),
                    ],
                    Stroke::new(1.2_f32, Color32::from_rgba_premultiplied(110, 215, 130, 200)),
                );
                any_drawn = true;
            }
            if any_drawn {
                painter.text(
                    Pos2::new(rect.right() - 6.0, rect.top() + 4.0),
                    Align2::RIGHT_TOP,
                    "SPECTRUM",
                    FontId::monospace(8.0),
                    Color32::from_rgba_premultiplied(110, 215, 130, 170),
                );
            }
        }
    }

    // Response curve (sum of all enabled bands).
    if eq.enabled {
        let steps = 180;
        let mut prev: Option<Pos2> = None;
        for i in 0..=steps {
            let x = w * i as f32 / steps as f32;
            let freq = x_to_freq(x, w);
            let db = eq.response_db(freq, 48_000.0);
            let p = Pos2::new(rect.left() + x, rect.top() + db_to_y(db, h));
            if let Some(pp) = prev {
                painter.line_segment([pp, p], Stroke::new(1.8_f32, Palette::ACCENT));
            }
            prev = Some(p);
        }
    }

    // Band handles + interaction.
    let pointer = ui.input(|i| i.pointer.clone());
    let drag_id = egui::Id::new(("eq_drag", track_id));
    let mut drag_state: Option<usize> = ui.memory_mut(|m| m.data.get_temp(drag_id));

    for (bi, band) in eq.bands.iter_mut().enumerate() {
        let gain_disp = match band.kind {
            BandKind::Peaking | BandKind::LowShelf | BandKind::HighShelf => band.gain_db,
            _ => 0.0,
        };
        let hx = freq_to_x(band.freq, w);
        let hy = db_to_y(gain_disp, h);
        let hp = Pos2::new(rect.left() + hx, rect.top() + hy);
        let color = if band.enabled { Palette::OK } else { Palette::TEXT_DIM };
        if band.enabled {
            painter.circle_filled(hp, 5.0, color);
        } else {
            painter.circle_stroke(hp, 5.0, Stroke::new(1.5_f32, color));
        }
        painter.text(
            hp + Vec2::new(8.0, -8.0),
            Align2::LEFT_TOP,
            format!("{}", bi + 1),
            FontId::monospace(9.0),
            color,
        );

        let near = pointer
            .hover_pos()
            .map(|pp| pp.distance(hp) < 14.0)
            .unwrap_or(false);
        let dragging = drag_state == Some(bi) && resp.dragged();

        if resp.hovered() && near && !resp.dragged() {
            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grab);
            // Wheel → Q.
            let wheel = ui.input(|i| i.raw_scroll_delta.y);
            if wheel.abs() > 0.1 {
                band.q = (band.q * (1.0 + (wheel as f64) / 400.0)).clamp(0.2, 8.0);
                changed = true;
            }
        }
        if resp.dragged() && (dragging || (near && drag_state.is_none())) {
            drag_state = Some(bi);
            if let Some(pp) = pointer.interact_pos() {
                let local = pp - rect.left_top();
                band.freq = x_to_freq(local.x, w).clamp(FREQ_MIN, FREQ_MAX);
                if band.kind.has_gain() {
                    band.gain_db = y_to_db(local.y, h).clamp(CURVE_MIN_DB, CURVE_MAX_DB);
                }
                changed = true;
            }
        }
        // Double-click on a handle toggles the band.
        if resp.double_clicked() && near {
            band.enabled = !band.enabled;
            changed = true;
        }
    }
    if !resp.dragged() {
        drag_state = None;
    }
    ui.memory_mut(|m| m.data.insert_temp(drag_id, drag_state));

    ui.label(
        RichText::new("Drag a point: frequency and gain · wheel: Q · double-click: on/off")
            .small()
            .color(Palette::TEXT_DIM),
    );

    // Band columns.
    ui.add_space(4.0);
    let names = ["1", "2", "3", "4", "5", "6"];
    ui.horizontal_wrapped(|ui| {
        for (bi, band) in eq.bands.iter_mut().enumerate() {
            ui.vertical(|ui| {
                ui.set_min_width(78.0);
                ui.horizontal(|ui| {
                    let dot = egui::Button::new(RichText::new("●").size(12.0))
                        .fill(if band.enabled { Palette::OK } else { Palette::SURFACE });
                    if ui.add(dot).on_hover_text("Toggle band").clicked() {
                        band.enabled = !band.enabled;
                        changed = true;
                    }
                    ui.label(RichText::new(names[bi]).small().strong());
                    ui.label(RichText::new(band.kind.label()).small().color(Palette::TEXT_DIM));
                });
                egui::Grid::new(("eq_band_grid", bi))
                    .num_columns(2)
                    .spacing([4.0, 2.0])
                    .show(ui, |ui| {
                        ui.label(RichText::new("Freq").small().color(Palette::TEXT_DIM));
                        let mut fr = band.freq;
                        if ui
                            .add(egui::DragValue::new(&mut fr).speed(2.0).range(20.0..=20_000.0).suffix(" Hz").max_decimals(0))
                            .changed()
                        {
                            band.freq = fr;
                            changed = true;
                        }
                        ui.end_row();
                        if band.kind.has_gain() {
                            ui.label(RichText::new("Gain").small().color(Palette::TEXT_DIM));
                            let mut g = band.gain_db;
                            if ui
                                .add(egui::DragValue::new(&mut g).speed(0.1).range(-18.0..=18.0).suffix(" dB").max_decimals(1))
                                .changed()
                            {
                                band.gain_db = g;
                                changed = true;
                            }
                            ui.end_row();
                        }
                        ui.label(RichText::new("Q").small().color(Palette::TEXT_DIM));
                        let mut q = band.q;
                        if ui
                            .add(egui::DragValue::new(&mut q).speed(0.01).range(0.2..=8.0).max_decimals(2))
                            .changed()
                        {
                            band.q = q;
                            changed = true;
                        }
                        ui.end_row();
                    });
            });
        }
    });

    if changed {
        app.push_undo("eq");
        if let Some(t) = app.project.track_mut(track_id) {
            t.eq = eq;
        }
        app.mark_dirty();
    }
}

// ---------------------------------------------------------------------------
// Multiband compressor
// ---------------------------------------------------------------------------

fn comp_dialog(app: &mut VeditApp, ui: &mut egui::Ui, track_id: u64) {
    let Some(mut comp) = app.project.track(track_id).map(|t| t.comp.clone()) else { return };
    let track_index = app.project.tracks.iter().position(|t| t.id == track_id);
    let mut changed = false;

    ui.horizontal(|ui| {
        if ui.checkbox(&mut comp.enabled, "Enabled").changed() {
            changed = true;
        }
        ui.separator();
        ui.label(RichText::new("Preset").small().color(Palette::TEXT_DIM));
        let presets = vedit_core::dsp::comp_presets();
        let current = presets.iter().position(|(_, s)| *s == comp).unwrap_or(usize::MAX);
        egui::ComboBox::from_id_salt("comp_preset")
            .selected_text(if current == usize::MAX { "Custom" } else { presets[current].0 })
            .width(150.0)
            .show_ui(ui, |ui| {
                for (i, (label, state)) in presets.iter().enumerate() {
                    if ui.selectable_label(current == i, *label).clicked() {
                        comp = state.clone();
                        changed = true;
                    }
                }
            });
    });

    ui.add_space(4.0);
    // Crossovers.
    egui::Grid::new("comp_xover")
        .num_columns(2)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            ui.label(RichText::new("Low ⇢ Mid crossover").small());
            let mut x1 = comp.xover_low_hz;
            if ui
                .add(egui::Slider::new(&mut x1, 40.0..=1000.0).suffix(" Hz").logarithmic(true))
                .changed()
            {
                comp.xover_low_hz = x1;
                changed = true;
            }
            ui.end_row();
            ui.label(RichText::new("Mid ⇢ High crossover").small());
            let mut x2 = comp.xover_high_hz;
            if ui
                .add(egui::Slider::new(&mut x2, 1_000.0..=12_000.0).suffix(" Hz").logarithmic(true))
                .changed()
            {
                comp.xover_high_hz = x2;
                changed = true;
            }
            ui.end_row();
        });

    ui.add_space(6.0);

    // Band panels.
    let band_panel = |ui: &mut egui::Ui, title: &str, range: &str, gr: f32, band: &mut vedit_core::dsp::CompBand, changed: &mut bool| {
        ui.group(|ui| {
            ui.set_min_width(160.0);
            ui.horizontal(|ui| {
                let dot = egui::Button::new(RichText::new("●").size(12.0))
                    .fill(if band.enabled { Palette::OK } else { Palette::SURFACE });
                if ui.add(dot).on_hover_text("Toggle band").clicked() {
                    band.enabled = !band.enabled;
                    *changed = true;
                }
                ui.label(RichText::new(title).strong());
                ui.label(RichText::new(range).small().color(Palette::TEXT_DIM));
            });
            egui::Grid::new(egui::Id::new(("comp_band", title)))
                .num_columns(2)
                .spacing([6.0, 3.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("Thresh").small().color(Palette::TEXT_DIM));
                    let mut v = band.threshold_db;
                    if ui.add(egui::DragValue::new(&mut v).speed(0.3).range(-60.0..=0.0).suffix(" dB").max_decimals(1)).changed() {
                        band.threshold_db = v;
                        *changed = true;
                    }
                    ui.end_row();
                    ui.label(RichText::new("Ratio").small().color(Palette::TEXT_DIM));
                    let mut v = band.ratio;
                    if ui.add(egui::DragValue::new(&mut v).speed(0.05).range(1.0..=20.0).suffix(":1").max_decimals(1)).changed() {
                        band.ratio = v;
                        *changed = true;
                    }
                    ui.end_row();
                    ui.label(RichText::new("Attack").small().color(Palette::TEXT_DIM));
                    let mut v = band.attack_ms;
                    if ui.add(egui::DragValue::new(&mut v).speed(0.3).range(0.1..=500.0).suffix(" ms").max_decimals(1)).changed() {
                        band.attack_ms = v;
                        *changed = true;
                    }
                    ui.end_row();
                    ui.label(RichText::new("Release").small().color(Palette::TEXT_DIM));
                    let mut v = band.release_ms;
                    if ui.add(egui::DragValue::new(&mut v).speed(1.0).range(5.0..=2000.0).suffix(" ms").max_decimals(0)).changed() {
                        band.release_ms = v;
                        *changed = true;
                    }
                    ui.end_row();
                    ui.label(RichText::new("Gain").small().color(Palette::TEXT_DIM));
                    let mut v = band.gain_db;
                    if ui.add(egui::DragValue::new(&mut v).speed(0.1).range(-24.0..=24.0).suffix(" dB").max_decimals(1)).changed() {
                        band.gain_db = v;
                        *changed = true;
                    }
                    ui.end_row();
                });
            // GR meter.
            ui.horizontal(|ui| {
                ui.label(RichText::new("GR").small().color(Palette::TEXT_DIM));
                let (rect, _) = ui.allocate_exact_size(Vec2::new(90.0, 8.0), Sense::hover());
                let p = ui.painter_at(rect);
                p.rect_filled(rect, 2.0, Color32::from_rgb(20, 20, 23));
                let t = (gr.abs() / 24.0).clamp(0.0, 1.0);
                if t > 0.005 {
                    let w = rect.width() * t;
                    p.rect_filled(
                        eframe::egui::Rect::from_min_size(Pos2::new(rect.right() - w, rect.top()), Vec2::new(w, rect.height())),
                        2.0,
                        Palette::ACCENT,
                    );
                }
                ui.label(RichText::new(format!("{gr:.1}")).monospace().size(9.0).color(Palette::TEXT_DIM));
            });
        });
    };

    let gr = track_index
        .map(|i| app.audio.track_gr(i))
        .unwrap_or(0.0);
    ui.horizontal(|ui| {
        band_panel(ui, "Low", "20–200 Hz", gr, &mut comp.low, &mut changed);
        band_panel(ui, "Mid", "200–3k Hz", gr, &mut comp.mid, &mut changed);
        band_panel(ui, "High", "3k–20k Hz", gr, &mut comp.high, &mut changed);
    });

    ui.add_space(4.0);
    ui.label(
        RichText::new("Gain reduction is metered live from the preview mixer. The same settings drive the export.")
            .small()
            .color(Palette::TEXT_DIM),
    );

    if changed {
        app.push_undo("comp");
        if let Some(t) = app.project.track_mut(track_id) {
            t.comp = comp;
        }
        app.mark_dirty();
    }
}

// ---------------------------------------------------------------------------
// Dynamics (gate → compressor → limiter)
// ---------------------------------------------------------------------------

/// The Fairlight-style "Dinamica" window: input→output transfer curve with
/// live GR metering, then the three modules (Espansore/Gate, Compressore,
/// Limitatore) with all envelope controls.
fn dynamics_dialog(app: &mut VeditApp, ui: &mut egui::Ui, track_id: u64) {
    let Some(mut st) = app.project.track(track_id).map(|t| t.dynamics.clone()) else { return };
    let track_index = app.project.tracks.iter().position(|t| t.id == track_id);
    let mut changed = false;

    // ---- header: master enable + live GR --------------------------------
    ui.horizontal(|ui| {
        if ui.checkbox(&mut st.enabled, "Enabled").changed() {
            changed = true;
        }
        ui.separator();
        let gr = track_index.map(|i| app.audio.track_dynamics_gr(i)).unwrap_or(0.0);
        ui.label(RichText::new("GAIN REDUCTION").small().color(Palette::TEXT_DIM));
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::new(110.0, 9.0), egui::Sense::hover());
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 2.0, Color32::from_rgb(20, 20, 23));
        let t = (gr.abs() / 24.0).clamp(0.0, 1.0);
        if t > 0.005 {
            let w = rect.width() * t;
            p.rect_filled(
                eframe::egui::Rect::from_min_size(Pos2::new(rect.right() - w, rect.top()), Vec2::new(w, rect.height())),
                2.0,
                Palette::ACCENT,
            );
        }
        ui.label(RichText::new(format!("{gr:+.1} dB")).monospace().size(9.5).color(
            if gr < -0.05 { Palette::ACCENT } else { Palette::TEXT_DIM },
        ));
    });

    // ---- transfer curve ---------------------------------------------------
    ui.add_space(4.0);
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::new(ui.available_width(), 130.0), egui::Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 4.0, Color32::from_rgb(16, 16, 18));
    let in_min = -60.0f64;
    let in_max = 0.0f64;
    let out_min = -60.0f64;
    let out_max = 6.0f64;
    let x_of = |db: f64| rect.left() + ((db - in_min) / (in_max - in_min)) as f32 * rect.width();
    let y_of = |db: f64| rect.bottom() - ((db - out_min) / (out_max - out_min)) as f32 * rect.height();
    // Grid (every 20 dB).
    for g in [-60, -40, -20, 0] {
        let gi = g as f64;
        p.line_segment([Pos2::new(x_of(gi), rect.top()), Pos2::new(x_of(gi), rect.bottom())], egui::Stroke::new(1.0_f32, Palette::LINE));
        p.line_segment([Pos2::new(rect.left(), y_of(gi)), Pos2::new(rect.right(), y_of(gi))], egui::Stroke::new(1.0_f32, Palette::LINE));
        p.text(Pos2::new(x_of(gi) + 3.0, rect.bottom() - 3.0), Align2::LEFT_BOTTOM, format!("{g}"), FontId::monospace(8.0), Palette::TEXT_FAINT);
    }
    // Unity reference line.
    p.line_segment(
        [Pos2::new(x_of(in_min), y_of(in_min.clamp(out_min, out_max))), Pos2::new(x_of(out_max.min(0.0) + 6.0), y_of(6.0))],
        egui::Stroke::new(1.0_f32, Palette::LINE),
    );
    // The curve.
    let pts: Vec<Pos2> = (0..=120)
        .map(|i| {
            let din = in_min + (in_max - in_min) * (i as f64 / 120.0);
            let dout = vedit_core::dsp::dynamics_transfer_db(&st, din).clamp(out_min, out_max);
            Pos2::new(x_of(din), y_of(dout))
        })
        .collect();
    p.add(egui::Shape::line(pts, egui::Stroke::new(1.8_f32, Palette::ACCENT)));
    // Stage threshold markers.
    if st.gate.enabled {
        let y = y_of(vedit_core::dsp::dynamics_transfer_db(&st, st.gate.threshold_db).clamp(out_min, out_max));
        p.circle_filled(Pos2::new(x_of(st.gate.threshold_db), y), 3.0, Color32::from_rgb(150, 208, 160));
        p.text(Pos2::new(x_of(st.gate.threshold_db), y - 9.0), Align2::CENTER_CENTER, "G", FontId::monospace(8.0), Color32::from_rgb(150, 208, 160));
    }
    if st.comp.enabled {
        let y = y_of(vedit_core::dsp::dynamics_transfer_db(&st, st.comp.threshold_db).clamp(out_min, out_max));
        p.circle_filled(Pos2::new(x_of(st.comp.threshold_db), y), 3.0, Palette::ACCENT);
        p.text(Pos2::new(x_of(st.comp.threshold_db), y - 9.0), Align2::CENTER_CENTER, "C", FontId::monospace(8.0), Palette::ACCENT);
    }
    if st.limiter.enabled {
        let y = y_of(st.limiter.threshold_db.clamp(out_min, out_max));
        p.circle_filled(Pos2::new(x_of(st.limiter.threshold_db), y), 3.0, Palette::PLAYHEAD);
        p.text(Pos2::new(x_of(st.limiter.threshold_db), y - 9.0), Align2::CENTER_CENTER, "L", FontId::monospace(8.0), Palette::PLAYHEAD);
    }

    // ---- module rows ------------------------------------------------------
    let row = |ui: &mut egui::Ui, label: &str, drag: egui::DragValue<'_>| {
        ui.label(RichText::new(label).small().color(Palette::TEXT_DIM));
        ui.add(drag);
        ui.end_row();
    };

    ui.add_space(6.0);
    ui.columns(3, |cols| {
        // Espansore / Gate
        cols[0].group(|ui| {
            ui.set_min_width(165.0);
            ui.horizontal(|ui| {
                if ui.checkbox(&mut st.gate.enabled, "Gate").changed() {
                    changed = true;
                }
            });
            egui::Grid::new("dyn_gate").num_columns(2).spacing([8.0, 3.0]).show(ui, |ui| {
                let mut v = st.gate.threshold_db;
                row(ui, "Soglia", egui::DragValue::new(&mut v).speed(0.3).range(-60.0..=0.0).suffix(" dB").max_decimals(1));
                if v != st.gate.threshold_db { st.gate.threshold_db = v; changed = true; }
                let mut v = st.gate.range_db;
                row(ui, "Intervallo", egui::DragValue::new(&mut v).speed(0.3).range(0.0..=60.0).suffix(" dB").max_decimals(1));
                if v != st.gate.range_db { st.gate.range_db = v; changed = true; }
                let mut v = st.gate.ratio;
                row(ui, "Rapporto", egui::DragValue::new(&mut v).speed(0.02).range(1.0..=10.0).suffix(":1").max_decimals(2));
                if v != st.gate.ratio { st.gate.ratio = v; changed = true; }
                let mut v = st.gate.attack_ms;
                row(ui, "Attacco", egui::DragValue::new(&mut v).speed(0.1).range(0.0..=100.0).suffix(" ms").max_decimals(1));
                if v != st.gate.attack_ms { st.gate.attack_ms = v; changed = true; }
                let mut v = st.gate.hold_ms;
                row(ui, "Tenuta", egui::DragValue::new(&mut v).speed(1.0).range(0.0..=500.0).suffix(" ms").max_decimals(0));
                if v != st.gate.hold_ms { st.gate.hold_ms = v; changed = true; }
                let mut v = st.gate.release_ms;
                row(ui, "Rilascio", egui::DragValue::new(&mut v).speed(1.0).range(1.0..=2000.0).suffix(" ms").max_decimals(0));
                if v != st.gate.release_ms { st.gate.release_ms = v; changed = true; }
            });
        });
        // Compressore
        cols[1].group(|ui| {
            ui.set_min_width(165.0);
            ui.horizontal(|ui| {
                if ui.checkbox(&mut st.comp.enabled, "Compressore").changed() {
                    changed = true;
                }
            });
            egui::Grid::new("dyn_comp").num_columns(2).spacing([8.0, 3.0]).show(ui, |ui| {
                let mut v = st.comp.threshold_db;
                row(ui, "Soglia", egui::DragValue::new(&mut v).speed(0.3).range(-60.0..=0.0).suffix(" dB").max_decimals(1));
                if v != st.comp.threshold_db { st.comp.threshold_db = v; changed = true; }
                let mut v = st.comp.ratio;
                row(ui, "Rapporto", egui::DragValue::new(&mut v).speed(0.02).range(1.0..=20.0).suffix(":1").max_decimals(2));
                if v != st.comp.ratio { st.comp.ratio = v; changed = true; }
                let mut v = st.comp.knee_db;
                row(ui, "Curva", egui::DragValue::new(&mut v).speed(0.2).range(0.0..=24.0).suffix(" dB").max_decimals(1));
                if v != st.comp.knee_db { st.comp.knee_db = v; changed = true; }
                let mut v = st.comp.mix;
                row(ui, "Mix", egui::DragValue::new(&mut v).speed(0.01).range(0.0..=1.0).max_decimals(2));
                if v != st.comp.mix { st.comp.mix = v; changed = true; }
                let mut v = st.comp.attack_ms;
                row(ui, "Attacco", egui::DragValue::new(&mut v).speed(0.2).range(0.1..=200.0).suffix(" ms").max_decimals(1));
                if v != st.comp.attack_ms { st.comp.attack_ms = v; changed = true; }
                let mut v = st.comp.hold_ms;
                row(ui, "Tenuta", egui::DragValue::new(&mut v).speed(0.5).range(0.0..=200.0).suffix(" ms").max_decimals(0));
                if v != st.comp.hold_ms { st.comp.hold_ms = v; changed = true; }
                let mut v = st.comp.release_ms;
                row(ui, "Rilascio", egui::DragValue::new(&mut v).speed(1.0).range(5.0..=2000.0).suffix(" ms").max_decimals(0));
                if v != st.comp.release_ms { st.comp.release_ms = v; changed = true; }
                let mut v = st.comp.makeup_db;
                row(ui, "Recupero", egui::DragValue::new(&mut v).speed(0.2).range(-12.0..=24.0).suffix(" dB").max_decimals(1));
                if v != st.comp.makeup_db { st.comp.makeup_db = v; changed = true; }
            });
        });
        // Limitatore
        cols[2].group(|ui| {
            ui.set_min_width(140.0);
            ui.horizontal(|ui| {
                if ui.checkbox(&mut st.limiter.enabled, "Limitatore").changed() {
                    changed = true;
                }
            });
            egui::Grid::new("dyn_lim").num_columns(2).spacing([8.0, 3.0]).show(ui, |ui| {
                let mut v = st.limiter.threshold_db;
                row(ui, "Soglia", egui::DragValue::new(&mut v).speed(0.1).range(-24.0..=0.0).suffix(" dB").max_decimals(1));
                if v != st.limiter.threshold_db { st.limiter.threshold_db = v; changed = true; }
                let mut v = st.limiter.release_ms;
                row(ui, "Rilascio", egui::DragValue::new(&mut v).speed(1.0).range(1.0..=1000.0).suffix(" ms").max_decimals(0));
                if v != st.limiter.release_ms { st.limiter.release_ms = v; changed = true; }
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new("Chain: gate → comp → limiter, identical in the preview mixer and on export.")
                    .small()
                    .color(Palette::TEXT_FAINT),
            );
        });
    });

    if changed {
        app.push_undo("dyn");
        if let Some(t) = app.project.track_mut(track_id) {
            t.dynamics = st;
        }
        app.mark_dirty();
    }
}

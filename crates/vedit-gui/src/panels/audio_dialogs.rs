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
                    let w = rect.width() * t as f32;
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

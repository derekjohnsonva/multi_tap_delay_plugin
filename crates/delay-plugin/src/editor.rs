//! PR 14–15 — egui editor: toolbar (PR 14) + amplitude lane rendering (PR 15).
//!
//! The toolbar wires the global controls (design §7) to the same params the
//! generic UI exposes. Below it, PR 15 adds the first custom-drawn lane: the
//! **amplitude** lane (design §7) — stems/lollipops rising from a baseline with
//! the source curve traced behind them, linked vs. detached taps drawn
//! distinctly, and a bipolar layout when polarity is on. This view is read-only
//! for now; direct lane interaction (dragging taps/curve) arrives in PR 19. The
//! pan lane, shared time axis, and meter follow in PR 16–18.

use crate::params::{DelayParams, TimeMode};
use delay_core::Lane;
use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, widgets};
use std::sync::Arc;

/// Build the editor. Returns `None` only if the host can't host an egui window.
pub fn create(params: Arc<DelayParams>) -> Option<Box<dyn Editor>> {
    let egui_state = params.editor_state.clone();
    create_egui_editor(
        egui_state,
        (),
        |_, _| {},
        move |ctx, setter, _state| {
            egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
                ui.add_space(4.0);
                toolbar(ui, &params, setter);
                ui.add_space(4.0);
            });

            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_space(2.0);
                ui.label(egui::RichText::new("Amplitude").small().weak());
                // The audio thread keeps `amp_lane`'s source/count/range in sync
                // with the params each block, so reading it here yields the live
                // per-tap gains. A blocking read is fine: the audio thread only
                // ever `try_write`s, so it never waits on the GUI.
                let bipolar = params.polarity.value();
                draw_amp_lane(ui, &params.amp_lane.read(), bipolar);

                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("Pan lane, time axis & meter — coming in PR 16–18")
                        .weak()
                        .italics(),
                );
            });
        },
    )
}

/// One row of labelled param widgets. Each widget is a `ParamSlider` bound to a
/// param via the `ParamSetter`, so edits go through nih-plug's gesture/automation
/// path exactly like the generic UI.
fn toolbar(ui: &mut egui::Ui, params: &DelayParams, setter: &ParamSetter) {
    // Two rows keep the controls readable at the default window width.
    ui.horizontal(|ui| {
        labeled(ui, "Taps", |ui| {
            ui.add(widgets::ParamSlider::for_param(&params.tap_count, setter));
        });
        labeled(ui, "Time", |ui| {
            ui.add(widgets::ParamSlider::for_param(&params.time_mode, setter));
        });
        // Only the active time control is meaningful, but showing both keeps the
        // layout stable; the inactive one simply has no audible effect.
        match params.time_mode.value() {
            TimeMode::Sync => labeled(ui, "Division", |ui| {
                ui.add(widgets::ParamSlider::for_param(&params.sync_division, setter));
            }),
            TimeMode::Free => labeled(ui, "Length", |ui| {
                ui.add(widgets::ParamSlider::for_param(&params.free_ms, setter));
            }),
        };
        labeled(ui, "Mix", |ui| {
            ui.add(widgets::ParamSlider::for_param(&params.mix, setter));
        });
    });

    ui.horizontal(|ui| {
        labeled(ui, "Amp Shape", |ui| {
            ui.add(widgets::ParamSlider::for_param(&params.amp_shape, setter));
        });
        labeled(ui, "Amount", |ui| {
            ui.add(widgets::ParamSlider::for_param(&params.amp_amount, setter));
        });
        labeled(ui, "Ping-Pong", |ui| {
            ui.add(widgets::ParamSlider::for_param(&params.pingpong_amount, setter));
        });
        labeled(ui, "Smoothing", |ui| {
            ui.add(widgets::ParamSlider::for_param(&params.smoothing, setter));
        });
        labeled(ui, "Output", |ui| {
            ui.add(widgets::ParamSlider::for_param(&params.output_trim, setter));
        });
        labeled(ui, "Polarity", |ui| {
            ui.add(widgets::ParamSlider::for_param(&params.polarity, setter));
        });
    });
}

/// Draw the amplitude lane (design §7): the source curve traced as a smooth
/// overlay, with each tap as a stem rising from the baseline to a lollipop at
/// its resolved gain. Linked taps follow the curve and render in the accent
/// colour; detached taps (per-tap overrides) render as a distinct hollow marker
/// so manual edits stand out. With polarity on the lane is bipolar (baseline in
/// the middle, stems up for positive and down for negative gain); otherwise it
/// is unipolar with the baseline at the bottom. Read-only here — PR 19 makes the
/// taps and curve draggable.
fn draw_amp_lane(ui: &mut egui::Ui, lane: &Lane, bipolar: bool) {
    // Take all the width and a fixed slice of height; the rest of the central
    // panel is reserved for the pan lane + meter (PR 16/18).
    let height = (ui.available_height() - 28.0).clamp(80.0, 220.0);
    let (rect, _response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();

    // Panel background + frame.
    painter.rect_filled(rect, 4.0, visuals.extreme_bg_color);
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color),
        egui::StrokeKind::Inside,
    );

    let pad = 8.0;
    let plot = rect.shrink(pad);
    // Value 1.0 reaches the top; the baseline sits at the bottom (unipolar) or
    // the vertical centre (bipolar). `max` is the full-scale value (always 1.0).
    let baseline_y = if bipolar { plot.center().y } else { plot.bottom() };
    let half = if bipolar { plot.height() * 0.5 } else { plot.height() };
    let value_to_y = |v: f32| baseline_y - v * half;
    let lo = if bipolar { -1.0 } else { 0.0 };

    let count = lane.count();
    // Single tap sits at the left edge of the curve; otherwise spread across.
    let index_to_x = |i: usize| {
        let t = if count <= 1 {
            0.0
        } else {
            i as f32 / (count - 1) as f32
        };
        plot.left() + t * plot.width()
    };

    // Baseline.
    painter.line_segment(
        [
            egui::pos2(plot.left(), baseline_y),
            egui::pos2(plot.right(), baseline_y),
        ],
        egui::Stroke::new(1.0, visuals.weak_text_color()),
    );

    let accent = visuals.selection.bg_fill;
    let detached_color = egui::Color32::from_rgb(0xff, 0xae, 0x42); // warm amber

    // Source curve overlay, sampled continuously across the lane width.
    const CURVE_STEPS: usize = 96;
    let curve_color = accent.gamma_multiply(0.4);
    let source = lane.source();
    let curve: Vec<egui::Pos2> = (0..=CURVE_STEPS)
        .map(|s| {
            let t = s as f32 / CURVE_STEPS as f32;
            let v = source.value_at(t).clamp(lo, 1.0);
            egui::pos2(plot.left() + t * plot.width(), value_to_y(v))
        })
        .collect();
    painter.add(egui::Shape::line(curve, egui::Stroke::new(1.5, curve_color)));

    // Stems + lollipops for each tap.
    for i in 0..count {
        let v = lane.value(i);
        let x = index_to_x(i);
        let tip = egui::pos2(x, value_to_y(v));
        let base = egui::pos2(x, baseline_y);
        let linked = lane.is_linked(i);
        let color = if linked { accent } else { detached_color };

        painter.line_segment([base, tip], egui::Stroke::new(1.5, color));
        if linked {
            painter.circle_filled(tip, 3.5, color);
        } else {
            // Hollow marker distinguishes a detached, manually-set tap.
            painter.circle_filled(tip, 3.5, visuals.extreme_bg_color);
            painter.circle_stroke(tip, 3.5, egui::Stroke::new(1.5, color));
        }
    }
}

/// A small captioned cell: a label above its widget, grouped so the toolbar
/// reads as discrete controls rather than a run of sliders.
fn labeled(ui: &mut egui::Ui, caption: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    ui.vertical(|ui| {
        ui.label(egui::RichText::new(caption).small().weak());
        add_contents(ui);
    });
}

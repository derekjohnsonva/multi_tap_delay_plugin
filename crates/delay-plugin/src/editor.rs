//! PR 14 — egui editor scaffold + toolbar.
//!
//! A first editor window built with `nih_plug_egui`. This PR only wires the
//! global toolbar controls (design §7) to the existing params — the same params
//! the generic UI exposes — so moving a widget here is identical to moving it in
//! the host's generic view and audibly changes the delay. The custom lane
//! drawing, time axis, meter, and direct lane interaction land in PR 15–19; the
//! large area below the toolbar is intentionally left as a placeholder for them.

use crate::params::{DelayParams, TimeMode};
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
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() * 0.5 - 12.0);
                    ui.label(
                        egui::RichText::new("Lanes, time axis & meter — coming in PR 15–18")
                            .weak()
                            .italics(),
                    );
                });
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

/// A small captioned cell: a label above its widget, grouped so the toolbar
/// reads as discrete controls rather than a run of sliders.
fn labeled(ui: &mut egui::Ui, caption: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    ui.vertical(|ui| {
        ui.label(egui::RichText::new(caption).small().weak());
        add_contents(ui);
    });
}

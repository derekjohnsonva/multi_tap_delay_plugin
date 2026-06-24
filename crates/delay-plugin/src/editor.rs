//! PR 14–15 — egui editor: toolbar (PR 14) + amplitude lane rendering (PR 15).
//!
//! The toolbar wires the global controls (design §7) to the same params the
//! generic UI exposes. Below it, PR 15 adds the first custom-drawn lane: the
//! **amplitude** lane (design §7) — stems/lollipops rising from a baseline with
//! the source curve traced behind them, linked vs. detached taps drawn
//! distinctly, and a bipolar layout when polarity is on. This view is read-only
//! for now; direct lane interaction (dragging taps/curve) arrives in PR 19. The
//! pan lane, shared time axis, and meter follow in PR 16–18.

use crate::params::{DelayParams, NoteDivision, TimeMode};
use delay_core::{Lane, COMB_ZONE_MS};
use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, widgets};
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Horizontal inset of each lane's plot area, shared by the lanes and the time
/// axis so their tap x-positions line up exactly.
const PLOT_PAD: f32 = 8.0;

/// Build the editor. Returns `None` only if the host can't host an egui window.
pub fn create(
    params: Arc<DelayParams>,
    current_bpm: Arc<AtomicF32>,
    meter_level: Arc<AtomicF32>,
) -> Option<Box<dyn Editor>> {
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

            // Output meter pinned to the right edge, always visible (design §7).
            egui::SidePanel::right("meter")
                .resizable(false)
                .exact_width(44.0)
                .show(ctx, |ui| {
                    ui.add_space(2.0);
                    ui.label(egui::RichText::new("Out").small().weak());
                    draw_meter(ui, meter_level.load(Ordering::Relaxed));
                });

            egui::CentralPanel::default().show(ctx, |ui| {
                // The meter and lanes are level-driven; keep repainting so they
                // animate even when no params change.
                ctx.request_repaint();
                // Reconstruct each lane's derived source/range/count from the
                // params before reading them. These fields aren't persisted, so
                // the editor must rebuild them itself rather than rely on the
                // audio thread having processed a block — otherwise a project
                // opened with the transport stopped would render stale lanes.
                // The audio thread only `try_write`s, so this brief blocking
                // write never makes it wait on the GUI.
                {
                    let mut amp = params.amp_lane.write();
                    let mut pan = params.pan_lane.write();
                    params.apply_to_lanes(&mut amp, &mut pan);
                }

                // Tap spacing and the comb-zone extent are the same for both
                // lanes (one shared time axis). Compute them once: the fraction
                // of the axis width whose tap delays fall in the comb zone, plus
                // the tick labels for the axis below.
                let count = params.tap_count.value() as usize;
                let bpm = current_bpm.load(Ordering::Relaxed);
                let step_ms = step_ms(&params, bpm);
                let comb_frac = comb_fraction(step_ms, count);

                // Split the remaining height into two equal lanes, reserving a
                // band for the labels, spacing, and the time axis below.
                let lane_h = ((ui.available_height() - 116.0) * 0.5).clamp(64.0, 200.0);

                // Amplitude lane (top): unipolar 0..1, or bipolar when polarity
                // is on. The continuous source shape is traced behind the taps.
                // Drag a tap to detach + set it; drag the background to nudge the
                // amp amount; right-click a tap to relink it.
                ui.add_space(2.0);
                ui.label(egui::RichText::new("Amplitude").small().weak());
                let bipolar = params.polarity.value();
                lane_widget(
                    ui,
                    &params.amp_lane,
                    &params.amp_amount,
                    setter,
                    LaneView {
                        id: "amp",
                        height: lane_h,
                        bipolar,
                        overlay: Overlay::SourceCurve,
                        top_label: if bipolar { "+" } else { "1" },
                        bottom_label: if bipolar { "−" } else { "0" },
                        comb_frac,
                    },
                );

                // Pan lane (bottom): always bipolar, centre = 0, up = R / down =
                // L. Ping-pong shows up as the alternating zig-zag connecting the
                // tap tips. Dragging the background nudges the ping-pong width.
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Pan").small().weak());
                lane_widget(
                    ui,
                    &params.pan_lane,
                    &params.pingpong_amount,
                    setter,
                    LaneView {
                        id: "pan",
                        height: lane_h,
                        bipolar: true,
                        overlay: Overlay::ConnectTaps,
                        top_label: "R",
                        bottom_label: "L",
                        comb_frac,
                    },
                );

                // Shared time axis: tick labels in ms (free) or division
                // multiples (sync), with the comb zone shaded at short times.
                ui.add_space(4.0);
                draw_time_axis(ui, &params, step_ms, count, comb_frac);
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

/// How a lane traces a guide line behind its taps.
#[derive(Clone, Copy)]
enum Overlay {
    /// Sample the source shape continuously across the lane (amplitude lane).
    SourceCurve,
    /// Connect the resolved tap tips, emphasising the discrete pattern — the
    /// ping-pong zig-zag on the pan lane.
    ConnectTaps,
}

/// Presentation options for a lane, so the one renderer serves both lanes.
struct LaneView {
    /// Stable id for per-lane interaction state (e.g. `"amp"`, `"pan"`).
    id: &'static str,
    /// Height of the lane's drawing area in points.
    height: f32,
    /// Baseline in the middle (`-1..1`) rather than at the bottom (`0..1`).
    bipolar: bool,
    overlay: Overlay,
    /// Tiny labels at the top and bottom-left of the plot (e.g. `R`/`L`).
    top_label: &'static str,
    bottom_label: &'static str,
    /// Fraction of the plot width (from the left) whose tap delays fall in the
    /// comb zone; shaded as a hint. `0.0` draws nothing.
    comb_frac: f32,
}

/// Pointer must be within this many points of a tap (in x) to grab it rather
/// than the curve behind it.
const TAP_GRAB_PX: f32 = 12.0;

/// Geometry shared by painting and hit-testing a lane, so both map taps to the
/// same screen positions.
struct LaneGeom {
    plot: egui::Rect,
    baseline_y: f32,
    half: f32,
    lo: f32,
    count: usize,
}

impl LaneGeom {
    fn new(rect: egui::Rect, bipolar: bool, count: usize) -> Self {
        let plot = rect.shrink(PLOT_PAD);
        // Value 1.0 reaches the top; the baseline sits at the bottom (unipolar)
        // or the vertical centre (bipolar). Full-scale magnitude is always 1.0.
        let baseline_y = if bipolar { plot.center().y } else { plot.bottom() };
        let half = if bipolar { plot.height() * 0.5 } else { plot.height() };
        let lo = if bipolar { -1.0 } else { 0.0 };
        Self {
            plot,
            baseline_y,
            half,
            lo,
            count,
        }
    }

    /// Screen x of tap `i`. A single tap sits at the left edge.
    fn x_of(&self, i: usize) -> f32 {
        let t = if self.count <= 1 {
            0.0
        } else {
            i as f32 / (self.count - 1) as f32
        };
        self.plot.left() + t * self.plot.width()
    }

    /// Screen y of value `v`.
    fn y_of(&self, v: f32) -> f32 {
        self.baseline_y - v * self.half
    }

    /// Value (clamped to the lane range) for a screen y.
    fn value_at_y(&self, y: f32) -> f32 {
        ((self.baseline_y - y) / self.half).clamp(self.lo, 1.0)
    }

    /// Tap index closest in x to `px`, paired with that x distance in points.
    fn nearest_tap(&self, px: f32) -> Option<(usize, f32)> {
        if self.count == 0 {
            return None;
        }
        let i = (0..self.count)
            .min_by(|&a, &b| {
                (self.x_of(a) - px)
                    .abs()
                    .total_cmp(&(self.x_of(b) - px).abs())
            })
            .unwrap();
        Some((i, (self.x_of(i) - px).abs()))
    }
}

/// Render and handle interaction for one parameter lane (design §7). Painting
/// reads a snapshot under the read lock; edits take the write lock briefly. The
/// audio thread only `try_write`s, so it never stalls on these GUI edits.
///
/// Interaction (design §7): **drag a tap** to detach + set its value; **drag the
/// background curve** to nudge the source `amount` param, which moves every
/// linked tap at once; **right-click a tap** to relink it to the source.
fn lane_widget(
    ui: &mut egui::Ui,
    lock: &parking_lot::RwLock<Lane>,
    amount: &FloatParam,
    setter: &ParamSetter,
    view: LaneView,
) {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), view.height),
        egui::Sense::click_and_drag(),
    );

    // Paint from a read snapshot, then drop the guard before any write.
    let count = {
        let lane = lock.read();
        let geom = LaneGeom::new(rect, view.bipolar, lane.count());
        paint_lane(ui, rect, &lane, &geom, &view);
        lane.count()
    };

    handle_lane_input(ui, &response, rect, &view, count, lock, amount, setter);
}

/// Draw a lane's frame, comb shade, baseline, labels, source/zig-zag overlay,
/// and the per-tap stems + lollipops (linked filled, detached hollow).
fn paint_lane(ui: &egui::Ui, rect: egui::Rect, lane: &Lane, geom: &LaneGeom, view: &LaneView) {
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

    // Comb-zone hint: shade the short-time region behind everything else.
    shade_comb_zone(&painter, geom.plot, view.comb_frac);

    let count = geom.count;

    // Baseline.
    painter.line_segment(
        [
            egui::pos2(geom.plot.left(), geom.baseline_y),
            egui::pos2(geom.plot.right(), geom.baseline_y),
        ],
        egui::Stroke::new(1.0, visuals.weak_text_color()),
    );

    // Edge labels (top + bottom-left of the plot).
    let label_color = visuals.weak_text_color();
    painter.text(
        geom.plot.left_top(),
        egui::Align2::LEFT_TOP,
        view.top_label,
        egui::FontId::proportional(10.0),
        label_color,
    );
    painter.text(
        geom.plot.left_bottom(),
        egui::Align2::LEFT_BOTTOM,
        view.bottom_label,
        egui::FontId::proportional(10.0),
        label_color,
    );

    let accent = visuals.selection.bg_fill;
    let detached_color = egui::Color32::from_rgb(0xff, 0xae, 0x42); // warm amber
    let overlay_color = accent.gamma_multiply(0.4);

    // Guide line behind the taps.
    let overlay_pts: Vec<egui::Pos2> = match view.overlay {
        Overlay::SourceCurve => {
            const CURVE_STEPS: usize = 96;
            let source = lane.source();
            (0..=CURVE_STEPS)
                .map(|s| {
                    let t = s as f32 / CURVE_STEPS as f32;
                    let v = source.value_at(t).clamp(geom.lo, 1.0);
                    egui::pos2(geom.plot.left() + t * geom.plot.width(), geom.y_of(v))
                })
                .collect()
        }
        Overlay::ConnectTaps => (0..count)
            .map(|i| egui::pos2(geom.x_of(i), geom.y_of(lane.value(i))))
            .collect(),
    };
    painter.add(egui::Shape::line(
        overlay_pts,
        egui::Stroke::new(1.5, overlay_color),
    ));

    // Stems + lollipops for each tap.
    for i in 0..count {
        let tip = egui::pos2(geom.x_of(i), geom.y_of(lane.value(i)));
        let base = egui::pos2(geom.x_of(i), geom.baseline_y);
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

/// What a drag is currently moving, remembered for the gesture's duration so a
/// drag started on a tap stays on that tap even as the pointer moves.
#[derive(Clone, Copy)]
enum DragTarget {
    /// Detach + set this tap's value.
    Tap(usize),
    /// Nudge the source amount param (moves all linked taps).
    Curve,
}

/// Apply pointer interaction to a lane: tap drag (detach + set), curve drag
/// (adjust `amount`), and right-click relink. All lane edits go through the
/// write lock; the amount nudge goes through the param setter's gesture path.
#[allow(clippy::too_many_arguments)]
fn handle_lane_input(
    ui: &egui::Ui,
    response: &egui::Response,
    rect: egui::Rect,
    view: &LaneView,
    count: usize,
    lock: &parking_lot::RwLock<Lane>,
    amount: &FloatParam,
    setter: &ParamSetter,
) {
    let geom = LaneGeom::new(rect, view.bipolar, count);
    let drag_id = egui::Id::new(("lane_drag", view.id));

    // Right-click a tap to relink it to the source.
    if response.secondary_clicked() {
        if let Some(px) = response.interact_pointer_pos() {
            if let Some((i, dist)) = geom.nearest_tap(px.x) {
                if dist <= TAP_GRAB_PX {
                    lock.write().relink(i);
                }
            }
        }
    }

    // Decide, once per gesture, whether the drag grabs a tap or the curve.
    if response.drag_started() {
        let target = response
            .interact_pointer_pos()
            .and_then(|px| geom.nearest_tap(px.x))
            .filter(|&(_, dist)| dist <= TAP_GRAB_PX)
            .map(|(i, _)| DragTarget::Tap(i))
            .unwrap_or(DragTarget::Curve);
        if matches!(target, DragTarget::Curve) {
            setter.begin_set_parameter(amount);
        }
        ui.data_mut(|d| d.insert_temp(drag_id, TargetMarker(target)));
    }

    if response.dragged() {
        let target = ui.data(|d| d.get_temp::<TargetMarker>(drag_id)).map(|m| m.0);
        match target {
            Some(DragTarget::Tap(i)) => {
                if let Some(px) = response.interact_pointer_pos() {
                    let v = geom.value_at_y(px.y);
                    lock.write().set_tap_value(i, v);
                }
            }
            Some(DragTarget::Curve) => {
                // Vertical drag nudges the amount; up increases. The audio thread
                // re-derives the source from this param, moving all linked taps.
                let dy = response.drag_delta().y;
                if dy != 0.0 {
                    let next = (amount.value() - dy * 0.004).clamp(0.0, 1.0);
                    setter.set_parameter(amount, next);
                }
            }
            None => {}
        }
    }

    if response.drag_stopped() {
        let target = ui.data(|d| d.get_temp::<TargetMarker>(drag_id)).map(|m| m.0);
        if matches!(target, Some(DragTarget::Curve)) {
            setter.end_set_parameter(amount);
        }
        ui.data_mut(|d| d.remove::<TargetMarker>(drag_id));
    }
}

/// Wrapper so [`DragTarget`] can live in egui's temp data store (which needs a
/// `'static + Clone + Send + Sync` value).
#[derive(Clone, Copy)]
struct TargetMarker(DragTarget);

/// Tap spacing in milliseconds for the current time-mode params. Mirrors the
/// engine's `step_samples` (lib.rs) but in ms, so the editor can label the time
/// axis and locate the comb zone without sharing engine state.
fn step_ms(params: &DelayParams, bpm: f32) -> f32 {
    match params.time_mode.value() {
        TimeMode::Sync => params.sync_division.value().beats() * (60_000.0 / bpm),
        TimeMode::Free => params.free_ms.value(),
    }
}

/// Fraction of the lane width (from the left) covered by taps inside the comb
/// zone (design §5). Tap `i` (0-based) lands at delay `(i+1)·step_ms` and is
/// drawn at normalized x `i / (count-1)`. The comb boundary is where the delay
/// equals [`COMB_ZONE_MS`], i.e. continuous index `COMB_ZONE_MS/step_ms - 1`.
fn comb_fraction(step_ms: f32, count: usize) -> f32 {
    if step_ms <= 0.0 || count == 0 {
        return 0.0;
    }
    if count == 1 {
        // A single tap sits at the left edge; flag the whole strip if it's short.
        return if step_ms < COMB_ZONE_MS { 1.0 } else { 0.0 };
    }
    let boundary_index = COMB_ZONE_MS / step_ms - 1.0;
    (boundary_index / (count - 1) as f32).clamp(0.0, 1.0)
}

/// Shade the comb-zone region inside `plot` (the left `frac` of its width).
fn shade_comb_zone(painter: &egui::Painter, plot: egui::Rect, frac: f32) {
    if frac <= 0.0 {
        return;
    }
    let zone = egui::Rect::from_min_max(
        plot.left_top(),
        egui::pos2(plot.left() + frac * plot.width(), plot.bottom()),
    );
    painter.rect_filled(zone, 0.0, egui::Color32::from_rgba_unmultiplied(0xff, 0x6a, 0x3d, 18));
}

/// The shared time axis below both lanes (design §7): tick labels in ms (free
/// mode) or in division multiples (sync mode), aligned with the tap x-positions,
/// with the comb zone shaded at short times to match the lanes above.
fn draw_time_axis(
    ui: &mut egui::Ui,
    params: &DelayParams,
    step_ms: f32,
    count: usize,
    comb_frac: f32,
) {
    let (rect, _response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    // Match the lanes' horizontal plot extent so ticks line up with the taps.
    let plot = rect.shrink2(egui::vec2(PLOT_PAD, 0.0));

    shade_comb_zone(&painter, plot, comb_frac);

    let axis_color = visuals.weak_text_color();
    // Axis line along the top edge (just under the pan lane).
    painter.line_segment(
        [
            egui::pos2(plot.left(), rect.top()),
            egui::pos2(plot.right(), rect.top()),
        ],
        egui::Stroke::new(1.0, axis_color),
    );

    let sync = matches!(params.time_mode.value(), TimeMode::Sync);
    let division = params.sync_division.value();
    // Caption naming the units, far right so it doesn't collide with tick 0.
    let caption = if sync {
        NoteDivision::variants()[division.to_index()]
    } else {
        "ms"
    };
    painter.text(
        egui::pos2(plot.right(), rect.center().y),
        egui::Align2::RIGHT_CENTER,
        caption,
        egui::FontId::proportional(9.0),
        axis_color,
    );

    if count == 0 {
        return;
    }
    // Aim for ~6 labels so dense tap counts stay legible.
    let stride = (count as f32 / 6.0).ceil().max(1.0) as usize;
    let index_to_x = |i: usize| {
        let t = if count <= 1 {
            0.0
        } else {
            i as f32 / (count - 1) as f32
        };
        plot.left() + t * plot.width()
    };
    let font = egui::FontId::proportional(9.0);
    for i in (0..count).step_by(stride) {
        let x = index_to_x(i);
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.top() + 3.0)],
            egui::Stroke::new(1.0, axis_color),
        );
        // Free: absolute delay in ms. Sync: how many divisions out this tap is.
        let label = if sync {
            format!("{}", i + 1)
        } else {
            format!("{:.0}", (i + 1) as f32 * step_ms)
        };
        painter.text(
            egui::pos2(x, rect.top() + 4.0),
            egui::Align2::CENTER_TOP,
            label,
            font.clone(),
            axis_color,
        );
    }
}

/// Output meter dB range: the bottom and top of the vertical scale.
const METER_MIN_DB: f32 = -60.0;
const METER_MAX_DB: f32 = 6.0;

/// Map a linear level to its 0..1 position on the meter's dB scale.
fn meter_norm(level: f32) -> f32 {
    let db = 20.0 * level.max(1e-6).log10();
    ((db - METER_MIN_DB) / (METER_MAX_DB - METER_MIN_DB)).clamp(0.0, 1.0)
}

/// Draw the vertical output meter (design §4/§7): a level bar on a dB scale with
/// an always-visible amber headroom/clip zone above 0 dBFS. `level` is the
/// post-trim linear peak published by the engine.
fn draw_meter(ui: &mut egui::Ui, level: f32) {
    let bar_w = 14.0;
    let (rect, _response) = ui.allocate_exact_size(
        egui::vec2(bar_w, ui.available_height() - 6.0),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();

    // Track.
    painter.rect_filled(rect, 2.0, visuals.extreme_bg_color);

    let y_of = |norm: f32| rect.bottom() - norm * rect.height();
    let zero_db_y = y_of(meter_norm(1.0)); // 0 dBFS line

    let amber = egui::Color32::from_rgb(0xff, 0xae, 0x42);
    let red = egui::Color32::from_rgb(0xe5, 0x48, 0x4a);
    let green = egui::Color32::from_rgb(0x4c, 0xc2, 0x7a);

    // Always-visible clip/headroom zone above 0 dBFS.
    let zone = egui::Rect::from_min_max(rect.left_top(), egui::pos2(rect.right(), zero_db_y));
    painter.rect_filled(zone, 0.0, amber.gamma_multiply(0.18));

    // Level fill from the bottom up. Over 0 dBFS the fill goes red.
    let norm = meter_norm(level);
    if norm > 0.0 {
        let fill = egui::Rect::from_min_max(egui::pos2(rect.left(), y_of(norm)), rect.right_bottom());
        let over = level > 1.0;
        painter.rect_filled(fill, 2.0, if over { red } else { green });
    }

    // 0 dBFS reference line.
    painter.line_segment(
        [
            egui::pos2(rect.left(), zero_db_y),
            egui::pos2(rect.right(), zero_db_y),
        ],
        egui::Stroke::new(1.0, amber),
    );

    // Frame.
    painter.rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color),
        egui::StrokeKind::Inside,
    );
}

/// A small captioned cell: a label above its widget, grouped so the toolbar
/// reads as discrete controls rather than a run of sliders.
fn labeled(ui: &mut egui::Ui, caption: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    ui.vertical(|ui| {
        ui.label(egui::RichText::new(caption).small().weak());
        add_contents(ui);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-4, "expected {b}, got {a}");
    }

    #[test]
    fn no_comb_zone_when_taps_are_long() {
        // 200 ms spacing — every tap is well past the comb zone.
        assert_eq!(comb_fraction(200.0, 8), 0.0);
    }

    #[test]
    fn comb_zone_covers_left_portion_for_short_taps() {
        // 10 ms spacing over 7 taps: delays 10..70 ms. Boundary at delay = 30 ms
        // is continuous index 30/10 - 1 = 2, of 6 spans -> 1/3 of the width.
        approx(comb_fraction(10.0, 7), 1.0 / 3.0);
    }

    #[test]
    fn comb_zone_clamps_to_full_width() {
        // 1 ms spacing: the boundary is far past the last tap, so the whole lane
        // is in the comb zone.
        assert_eq!(comb_fraction(1.0, 8), 1.0);
    }

    #[test]
    fn single_tap_is_all_or_nothing() {
        assert_eq!(comb_fraction(10.0, 1), 1.0); // short -> flagged
        assert_eq!(comb_fraction(50.0, 1), 0.0); // long  -> clear
    }

    #[test]
    fn degenerate_inputs_are_safe() {
        assert_eq!(comb_fraction(0.0, 8), 0.0);
        assert_eq!(comb_fraction(10.0, 0), 0.0);
    }

    fn rect_100() -> egui::Rect {
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(100.0, 100.0))
    }

    #[test]
    fn unipolar_geom_maps_value_and_y() {
        let g = LaneGeom::new(rect_100(), false, 5);
        // Baseline at the bottom of the padded plot; full scale at the top.
        approx(g.y_of(0.0), g.plot.bottom());
        approx(g.y_of(1.0), g.plot.top());
        // value_at_y is the inverse and clamps to 0..1.
        approx(g.value_at_y(g.plot.bottom()), 0.0);
        approx(g.value_at_y(g.plot.top()), 1.0);
        assert_eq!(g.value_at_y(g.plot.bottom() + 50.0), 0.0); // below clamps
    }

    #[test]
    fn bipolar_geom_centres_baseline_and_clamps() {
        let g = LaneGeom::new(rect_100(), true, 4);
        approx(g.baseline_y, g.plot.center().y);
        approx(g.value_at_y(g.plot.center().y), 0.0);
        approx(g.value_at_y(g.plot.top()), 1.0);
        approx(g.value_at_y(g.plot.bottom()), -1.0);
    }

    #[test]
    fn nearest_tap_picks_closest_index() {
        let g = LaneGeom::new(rect_100(), false, 5);
        // Pointer right on tap 0 and tap 4.
        assert_eq!(g.nearest_tap(g.x_of(0)).unwrap().0, 0);
        assert_eq!(g.nearest_tap(g.x_of(4)).unwrap().0, 4);
        // Slightly off tap 2 still resolves to 2 with a small distance.
        let (i, dist) = g.nearest_tap(g.x_of(2) + 1.0).unwrap();
        assert_eq!(i, 2);
        approx(dist, 1.0);
        // No taps -> nothing to grab.
        assert!(LaneGeom::new(rect_100(), false, 0).nearest_tap(10.0).is_none());
    }

    #[test]
    fn single_tap_sits_at_left_edge() {
        let g = LaneGeom::new(rect_100(), false, 1);
        approx(g.x_of(0), g.plot.left());
    }

    #[test]
    fn meter_norm_maps_db_scale() {
        // Silence sits at the bottom, 0 dBFS near the top, clipping pinned to 1.
        assert_eq!(meter_norm(0.0), 0.0);
        approx(meter_norm(1.0), (0.0 - METER_MIN_DB) / (METER_MAX_DB - METER_MIN_DB));
        assert_eq!(meter_norm(10.0), 1.0); // +20 dB clamps to the top
        // Monotonic: louder reads higher.
        assert!(meter_norm(0.5) > meter_norm(0.1));
    }
}

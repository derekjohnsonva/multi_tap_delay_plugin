//! The egui editor (design §7): a toolbar of global controls over two stacked,
//! directly-editable lanes (amplitude + pan) sharing one time axis, with an
//! always-visible output meter at the right edge.
//!
//! - Toolbar wires every global param through the `ParamSetter` gesture path
//!   (enum params as dropdowns, bools as checkboxes, the rest as sliders).
//! - Each lane draws the source shape behind per-tap stems/lollipops, with
//!   linked vs. detached taps distinct; drag a tap to set it, drag the curve to
//!   nudge the shape amount, double/right-click to relink, "Reset" to relink all.
//! - The time axis labels switch ms ↔ note-division by mode and shades the
//!   comb zone; the meter shows the post-trim peak with a clip zone.

use crate::params::{DelayParams, NoteDivision, Theme, TimeMode};
use delay_core::{Lane, COMB_ZONE_MS};
use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui, widgets};
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Horizontal inset of each lane's plot area, shared by the lanes and the time
/// axis so their tap x-positions line up exactly.
const PLOT_PAD: f32 = 8.0;

// --- Palette (design §1: demo-quality look) -------------------------------

/// The resolved colour set for one [`Theme`]. Every theme-aware bit of the
/// editor reads its colours from here, so a palette is the single source of
/// truth for one look. The first six fields are hand-picked per theme; the
/// remaining three (panel/widget fills) are derived from them in [`palette`] so
/// each theme only has to specify the colours that actually carry its identity.
#[derive(Clone, Copy)]
struct Palette {
    /// Primary accent: curves, linked taps, selection.
    accent: egui::Color32,
    /// Detached / manually-edited taps.
    detached: egui::Color32,
    /// Taps that won't play (delay past the buffer) — greyed out.
    muted: egui::Color32,
    /// Window / panel background.
    bg: egui::Color32,
    /// Recessed lane-track / meter background.
    track: egui::Color32,
    /// Hairline borders around panels.
    hairline: egui::Color32,
    /// Faint alternating background (derived).
    faint_bg: egui::Color32,
    /// Inactive widget fill (derived).
    inactive: egui::Color32,
    /// Hovered widget fill (derived).
    hovered: egui::Color32,
    /// Whether to base egui's `Visuals` on the dark or light defaults, which
    /// drives the text colours that are not part of our palette.
    dark: bool,
}

/// Linearly blend two colours (`t = 0` → `a`, `t = 1` → `b`), per channel.
fn mix(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    egui::Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
}

/// Assemble a [`Palette`] from its identity colours, deriving the panel/widget
/// fills so every theme stays internally consistent (fills sit between the
/// background and the hairline, the hover state leans toward the accent).
fn palette(
    accent: egui::Color32,
    detached: egui::Color32,
    muted: egui::Color32,
    bg: egui::Color32,
    track: egui::Color32,
    hairline: egui::Color32,
    dark: bool,
) -> Palette {
    let inactive = mix(bg, hairline, 0.85);
    Palette {
        accent,
        detached,
        muted,
        bg,
        track,
        hairline,
        faint_bg: mix(bg, hairline, 0.45),
        inactive,
        hovered: mix(inactive, accent, 0.22),
        dark,
    }
}

/// Resolve a [`Theme`] to its concrete [`Palette`]. Ten cohesive looks (design
/// §1); [`Theme::Midnight`] reproduces the original navy/blue palette exactly.
fn palette_for(theme: Theme) -> Palette {
    use egui::Color32 as C;
    match theme {
        Theme::Midnight => palette(
            C::from_rgb(0x4d, 0xa6, 0xff),
            C::from_rgb(0xff, 0xae, 0x42),
            C::from_rgb(0x60, 0x66, 0x6f),
            C::from_rgb(0x17, 0x1a, 0x1f),
            C::from_rgb(0x0e, 0x10, 0x14),
            C::from_rgb(0x2b, 0x31, 0x3a),
            true,
        ),
        Theme::Ocean => palette(
            C::from_rgb(0x22, 0xd3, 0xee),
            C::from_rgb(0xff, 0x8a, 0x5c),
            C::from_rgb(0x5a, 0x7a, 0x7a),
            C::from_rgb(0x0d, 0x1b, 0x1e),
            C::from_rgb(0x07, 0x12, 0x14),
            C::from_rgb(0x1d, 0x34, 0x38),
            true,
        ),
        Theme::Forest => palette(
            C::from_rgb(0xa3, 0xe6, 0x35),
            C::from_rgb(0xff, 0xb4, 0x54),
            C::from_rgb(0x6b, 0x75, 0x63),
            C::from_rgb(0x13, 0x1a, 0x13),
            C::from_rgb(0x0b, 0x11, 0x0b),
            C::from_rgb(0x29, 0x33, 0x1f),
            true,
        ),
        Theme::Sunset => palette(
            C::from_rgb(0xff, 0x8c, 0x42),
            C::from_rgb(0x4d, 0xd0, 0xe1),
            C::from_rgb(0x7a, 0x6a, 0x60),
            C::from_rgb(0x1f, 0x17, 0x14),
            C::from_rgb(0x15, 0x0d, 0x0a),
            C::from_rgb(0x3a, 0x2b, 0x25),
            true,
        ),
        Theme::Grape => palette(
            C::from_rgb(0xa7, 0x8b, 0xfa),
            C::from_rgb(0xf6, 0xad, 0x55),
            C::from_rgb(0x6f, 0x66, 0x80),
            C::from_rgb(0x1a, 0x16, 0x22),
            C::from_rgb(0x11, 0x0d, 0x18),
            C::from_rgb(0x32, 0x2a, 0x40),
            true,
        ),
        Theme::Ember => palette(
            C::from_rgb(0xf0, 0x50, 0x6e),
            C::from_rgb(0xff, 0xb3, 0x47),
            C::from_rgb(0x7a, 0x60, 0x66),
            C::from_rgb(0x1f, 0x15, 0x17),
            C::from_rgb(0x15, 0x0b, 0x0d),
            C::from_rgb(0x3a, 0x28, 0x2c),
            true,
        ),
        Theme::Slate => palette(
            C::from_rgb(0x8a, 0xa4, 0xc8),
            C::from_rgb(0xd4, 0xa5, 0x74),
            C::from_rgb(0x6b, 0x72, 0x80),
            C::from_rgb(0x1a, 0x1c, 0x1f),
            C::from_rgb(0x11, 0x13, 0x16),
            C::from_rgb(0x2e, 0x33, 0x38),
            true,
        ),
        Theme::Solarized => palette(
            C::from_rgb(0xb5, 0x89, 0x00),
            C::from_rgb(0x26, 0x8b, 0xd2),
            C::from_rgb(0x58, 0x6e, 0x75),
            C::from_rgb(0x00, 0x2b, 0x36),
            C::from_rgb(0x07, 0x36, 0x42),
            C::from_rgb(0x0a, 0x3d, 0x49),
            true,
        ),
        Theme::Paper => palette(
            C::from_rgb(0x25, 0x63, 0xeb),
            C::from_rgb(0xd9, 0x77, 0x2b),
            C::from_rgb(0x9a, 0x94, 0x86),
            C::from_rgb(0xf3, 0xf0, 0xe9),
            C::from_rgb(0xe6, 0xe1, 0xd6),
            C::from_rgb(0xc9, 0xc2, 0xb4),
            false,
        ),
        Theme::Rose => palette(
            C::from_rgb(0xe2, 0x36, 0x70),
            C::from_rgb(0x2b, 0x9a, 0xa0),
            C::from_rgb(0xa0, 0x88, 0x91),
            C::from_rgb(0xfb, 0xf1, 0xf3),
            C::from_rgb(0xf3, 0xe2, 0xe6),
            C::from_rgb(0xe0, 0xc4, 0xcc),
            false,
        ),
    }
}

/// Install `pal`'s colours into egui's `Visuals`. Called whenever the selected
/// theme changes (design §1 — a cohesive look rather than raw egui defaults).
fn apply_palette(ctx: &egui::Context, pal: &Palette) {
    let mut v = if pal.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = pal.bg;
    v.window_fill = pal.bg;
    v.extreme_bg_color = pal.track;
    v.faint_bg_color = pal.faint_bg;
    v.selection.bg_fill = pal.accent.gamma_multiply(0.4);
    v.selection.stroke = egui::Stroke::new(1.0, pal.accent);
    v.hyperlink_color = pal.accent;
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, pal.hairline);
    // Accent the controls' fills so sliders/checkboxes read as one family.
    v.widgets.inactive.bg_fill = pal.inactive;
    v.widgets.inactive.weak_bg_fill = pal.inactive;
    v.widgets.hovered.bg_fill = pal.hovered;
    v.widgets.active.bg_fill = pal.accent.gamma_multiply(0.6);
    ctx.set_visuals(v);
}

/// Apply the theme-independent style (spacing) once at editor creation.
fn apply_base_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(7.0, 3.0);
    ctx.set_style(style);
}

/// Editor-side state carried by `create_egui_editor`: the theme whose palette is
/// currently installed in egui's `Visuals`, so we only rebuild the visuals when
/// the user actually picks a different theme rather than every frame.
#[derive(Default)]
struct EditorState {
    applied_theme: Option<Theme>,
}

/// Build the editor. Returns `None` only if the host can't host an egui window.
pub fn create(
    params: Arc<DelayParams>,
    current_bpm: Arc<AtomicF32>,
    meter_level: Arc<AtomicF32>,
) -> Option<Box<dyn Editor>> {
    let egui_state = params.editor_state.clone();
    create_egui_editor(
        egui_state,
        EditorState::default(),
        |ctx, _| apply_base_style(ctx),
        move |ctx, setter, state| {
            // Re-skin egui only when the selected theme changes (it's persisted,
            // so on first paint this also installs the restored theme).
            let theme = params.theme.value();
            let pal = palette_for(theme);
            if state.applied_theme != Some(theme) {
                apply_palette(ctx, &pal);
                state.applied_theme = Some(theme);
            }

            egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
                ui.add_space(3.0);
                toolbar(ui, &params, setter);
                // Keep the controls tucked just above the graphs.
                ui.add_space(1.0);
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
                // The audio thread keeps each lane's source/count/range in sync
                // with the params every block, so reading them here yields the
                // live per-tap gains/pans. A blocking read is fine: the audio
                // thread only ever `try_write`s, so it never waits on the GUI.

                // Tap spacing and the comb-zone extent are the same for both
                // lanes (one shared time axis). Compute them once: the fraction
                // of the axis width whose tap delays fall in the comb zone, plus
                // the tick labels for the axis below.
                let count = params.tap_count.value() as usize;
                let bpm = current_bpm.load(Ordering::Relaxed);
                let step_ms = step_ms(&params, bpm);
                let comb_frac = comb_fraction(step_ms, count);
                // How many taps fit in the delay buffer; the rest are scheduled
                // past it and won't play, so the lanes grey them out.
                let playable = playable_taps(step_ms, count, crate::MAX_DELAY_SECONDS * 1000.0);

                // Split the remaining height into two equal lanes, reserving a
                // band for the labels, spacing, and the time axis below.
                let lane_h = ((ui.available_height() - 116.0) * 0.5).clamp(64.0, 200.0);

                // Amplitude lane (top): unipolar 0..1, or bipolar when polarity
                // is on. The continuous source shape is traced behind the taps.
                // Drag a tap to detach + set it; drag the background to nudge the
                // amp amount; double-click (or right-click) a tap to relink it.
                ui.add_space(1.0);
                lane_header(ui, "Amplitude", &params.amp_lane);
                let bipolar = params.polarity.value();
                lane_widget(
                    ui,
                    &params.amp_lane,
                    &params.amp_amount,
                    setter,
                    &pal,
                    LaneView {
                        id: "amp",
                        height: lane_h,
                        bipolar,
                        overlay: Overlay::SourceCurve,
                        top_label: if bipolar { "+" } else { "1" },
                        bottom_label: if bipolar { "−" } else { "0" },
                        comb_frac,
                        playable,
                    },
                );

                // Pan lane (bottom): always bipolar, centre = 0, up = R / down =
                // L. Ping-pong shows up as the alternating zig-zag connecting the
                // tap tips. Dragging the background nudges the ping-pong width.
                ui.add_space(4.0);
                lane_header(ui, "Pan", &params.pan_lane);
                lane_widget(
                    ui,
                    &params.pan_lane,
                    &params.pingpong_amount,
                    setter,
                    &pal,
                    LaneView {
                        id: "pan",
                        height: lane_h,
                        bipolar: true,
                        overlay: Overlay::ConnectTaps,
                        top_label: "R",
                        bottom_label: "L",
                        comb_frac,
                        playable,
                    },
                );

                // Shared time axis: tick labels in ms (free) or division
                // multiples (sync), with the comb zone shaded at short times.
                ui.add_space(4.0);
                draw_time_axis(ui, &params, &pal, step_ms, count, comb_frac, playable);
            });
        },
    )
}

/// Fixed widget widths so the two control rows fit the window without the last
/// cell (Output) scaling off the right edge.
const SLIDER_W: f32 = 84.0;
const COMBO_W: f32 = 96.0;

/// One row of labelled param widgets bound via the `ParamSetter`, so edits go
/// through nih-plug's gesture/automation path exactly like the generic UI.
/// Enum params are dropdowns (a read-only current value, not a draggable
/// slider); continuous params are fixed-width sliders.
fn toolbar(ui: &mut egui::Ui, params: &DelayParams, setter: &ParamSetter) {
    // Two rows keep the controls readable at the default window width.
    ui.horizontal(|ui| {
        labeled(ui, "Taps", |ui| {
            ui.add(widgets::ParamSlider::for_param(&params.tap_count, setter).with_width(SLIDER_W));
        });
        labeled(ui, "Time", |ui| {
            enum_combo(ui, "time_mode", &params.time_mode, setter);
        });
        // Only the active time control is meaningful, but showing both keeps the
        // layout stable; the inactive one simply has no audible effect.
        match params.time_mode.value() {
            TimeMode::Sync => labeled(ui, "Division", |ui| {
                enum_combo(ui, "division", &params.sync_division, setter);
            }),
            TimeMode::Free => labeled(ui, "Length", |ui| {
                ui.add(
                    widgets::ParamSlider::for_param(&params.free_ms, setter).with_width(SLIDER_W),
                );
            }),
        };
        labeled(ui, "Mix", |ui| {
            ui.add(widgets::ParamSlider::for_param(&params.mix, setter).with_width(SLIDER_W));
        });
        // Cosmetic only: pick one of ten editor colour themes (design §1).
        labeled(ui, "Theme", |ui| {
            enum_combo(ui, "theme", &params.theme, setter);
        });
    });

    ui.horizontal(|ui| {
        labeled(ui, "Amp Shape", |ui| {
            enum_combo(ui, "amp_shape", &params.amp_shape, setter);
        });
        labeled(ui, "Amount", |ui| {
            ui.add(
                widgets::ParamSlider::for_param(&params.amp_amount, setter).with_width(SLIDER_W),
            );
        });
        labeled(ui, "Ping-Pong", |ui| {
            ui.add(
                widgets::ParamSlider::for_param(&params.pingpong_amount, setter)
                    .with_width(SLIDER_W),
            );
        });
        labeled(ui, "Smoothing", |ui| {
            ui.add(widgets::ParamSlider::for_param(&params.smoothing, setter).with_width(SLIDER_W));
        });
        labeled(ui, "Output", |ui| {
            ui.add(
                widgets::ParamSlider::for_param(&params.output_trim, setter).with_width(SLIDER_W),
            );
        });
        labeled(ui, "Polarity", |ui| {
            bool_checkbox(ui, &params.polarity, setter);
        });
        labeled(ui, "Limiter", |ui| {
            bool_checkbox(ui, &params.limiter, setter);
        });
    });
}

/// A checkbox bound to a `BoolParam` through the gesture path. A bool reads
/// better as a checkbox than as a slider.
fn bool_checkbox(ui: &mut egui::Ui, param: &BoolParam, setter: &ParamSetter) {
    let mut on = param.value();
    if ui.checkbox(&mut on, "").changed() {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, on);
        setter.end_set_parameter(param);
    }
}

/// A dropdown for an enum param: the current value shows as read-only text and
/// picking an entry sets the param through the gesture path.
fn enum_combo<T>(ui: &mut egui::Ui, salt: &str, param: &EnumParam<T>, setter: &ParamSetter)
where
    T: Enum + Copy + PartialEq + 'static,
{
    let variants = T::variants();
    let cur_idx = param.value().to_index();
    let mut selected = cur_idx;
    egui::ComboBox::from_id_salt(salt)
        .selected_text(variants[cur_idx])
        .width(COMBO_W)
        .show_ui(ui, |ui| {
            for (i, name) in variants.iter().enumerate() {
                ui.selectable_value(&mut selected, i, *name);
            }
        });
    if selected != cur_idx {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, T::from_index(selected));
        setter.end_set_parameter(param);
    }
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
    /// Number of taps that fit in the delay buffer; taps at index `>= playable`
    /// are scheduled past it and won't play, so they render greyed under a
    /// "won't play" zone.
    playable: usize,
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
        let baseline_y = if bipolar {
            plot.center().y
        } else {
            plot.bottom()
        };
        let half = if bipolar {
            plot.height() * 0.5
        } else {
            plot.height()
        };
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
/// linked tap at once; **double-click (or right-click) a tap** to relink it to
/// the source so it snaps back onto the shape.
fn lane_widget(
    ui: &mut egui::Ui,
    lock: &parking_lot::RwLock<Lane>,
    amount: &FloatParam,
    setter: &ParamSetter,
    pal: &Palette,
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
        paint_lane(ui, rect, &lane, &geom, &view, pal);
        lane.count()
    };

    handle_lane_input(ui, &response, rect, &view, count, lock, amount, setter);
}

/// Draw a lane's frame, comb shade, baseline, labels, source/zig-zag overlay,
/// and the per-tap stems + lollipops (linked filled, detached hollow).
fn paint_lane(
    ui: &egui::Ui,
    rect: egui::Rect,
    lane: &Lane,
    geom: &LaneGeom,
    view: &LaneView,
    pal: &Palette,
) {
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

    // "Won't play" hint: taps scheduled past the buffer length are greyed under
    // a shaded zone with a cutoff line.
    if let Some(cx) = cutoff_x(geom, view.playable) {
        shade_out_of_range(&painter, geom.plot, cx, pal.muted);
    }

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

    let accent = pal.accent;
    let detached_color = pal.detached;
    let overlay_color = accent.gamma_multiply(0.4);

    // Guide line behind the taps.
    let overlay_pts: Vec<egui::Pos2> = match view.overlay {
        Overlay::SourceCurve => {
            // Sample roughly once per horizontal pixel so a high-cycle shape
            // renders smoothly. A fixed low step count would alias: too few
            // points per cycle makes the polyline's peaks/troughs land at
            // varying phases and look ragged, even though the taps are exact.
            let steps = (geom.plot.width().round() as usize).clamp(64, 4096);
            let source = lane.source();
            (0..=steps)
                .map(|s| {
                    let t = s as f32 / steps as f32;
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

        // Past the buffer: this tap is silent. Grey it out (the link/detach
        // distinction no longer matters since it won't be heard).
        if i >= view.playable {
            painter.line_segment([base, tip], egui::Stroke::new(1.0, pal.muted));
            painter.circle_filled(tip, 3.0, pal.muted);
            continue;
        }

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

    // Double-click (or right-click) a tap to relink it: it snaps back onto the
    // source shape, re-sampling from the curve like an unedited tap.
    if response.double_clicked() || response.secondary_clicked() {
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
        let target = ui
            .data(|d| d.get_temp::<TargetMarker>(drag_id))
            .map(|m| m.0);
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
        let target = ui
            .data(|d| d.get_temp::<TargetMarker>(drag_id))
            .map(|m| m.0);
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
    painter.rect_filled(
        zone,
        0.0,
        egui::Color32::from_rgba_unmultiplied(0xff, 0x6a, 0x3d, 18),
    );
}

/// Number of taps that fit in a `max_delay_ms` buffer. Tap `i` (0-based) lands
/// at delay `(i+1)·step_ms`, so the count that fit is `floor(max/step)`, capped
/// at `count`. A non-positive step means no cutoff (all taps fit).
fn playable_taps(step_ms: f32, count: usize, max_delay_ms: f32) -> usize {
    if step_ms <= 0.0 {
        return count;
    }
    ((max_delay_ms / step_ms).floor() as usize).min(count)
}

/// X of the cutoff between the last playable tap and the first one that's past
/// the buffer, or `None` when every tap plays. Sits midway between the two taps.
fn cutoff_x(geom: &LaneGeom, playable: usize) -> Option<f32> {
    if playable >= geom.count {
        return None;
    }
    Some(if playable == 0 {
        geom.plot.left()
    } else {
        (geom.x_of(playable - 1) + geom.x_of(playable)) * 0.5
    })
}

/// Shade the "won't play" region of `plot` (right of `cutoff_x`) and draw a
/// dashed cutoff line, marking taps whose delay exceeds the buffer.
fn shade_out_of_range(
    painter: &egui::Painter,
    plot: egui::Rect,
    cutoff_x: f32,
    muted: egui::Color32,
) {
    let zone = egui::Rect::from_min_max(egui::pos2(cutoff_x, plot.top()), plot.right_bottom());
    painter.rect_filled(
        zone,
        0.0,
        egui::Color32::from_rgba_unmultiplied(0x9a, 0xa0, 0xaa, 20),
    );
    painter.extend(egui::Shape::dashed_line(
        &[
            egui::pos2(cutoff_x, plot.top()),
            egui::pos2(cutoff_x, plot.bottom()),
        ],
        egui::Stroke::new(1.0, muted),
        4.0,
        3.0,
    ));
}

/// The shared time axis below both lanes (design §7): tick labels in ms (free
/// mode) or in division multiples (sync mode), aligned with the tap x-positions,
/// with the comb zone shaded at short times to match the lanes above.
fn draw_time_axis(
    ui: &mut egui::Ui,
    params: &DelayParams,
    pal: &Palette,
    step_ms: f32,
    count: usize,
    comb_frac: f32,
    playable: usize,
) {
    let (rect, _response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    // Match the lanes' horizontal plot extent so ticks line up with the taps.
    let plot = rect.shrink2(egui::vec2(PLOT_PAD, 0.0));

    shade_comb_zone(&painter, plot, comb_frac);

    // Cutoff matching the lanes: a LaneGeom over this rect shares the same x
    // mapping (it only differs vertically, which we don't use here).
    let cutoff = cutoff_x(&LaneGeom::new(rect, false, count), playable);
    if let Some(cx) = cutoff {
        shade_out_of_range(&painter, plot, cx, pal.muted);
    }

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

    // Mark the buffer cutoff so the greyed taps above have an explanation.
    if let Some(cx) = cutoff {
        painter.text(
            egui::pos2(cx + 3.0, rect.bottom()),
            egui::Align2::LEFT_BOTTOM,
            "max",
            egui::FontId::proportional(9.0),
            pal.muted,
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
        let fill =
            egui::Rect::from_min_max(egui::pos2(rect.left(), y_of(norm)), rect.right_bottom());
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

/// A lane's heading row: its title plus a "Reset" button that relinks every tap
/// to the source shape, discarding manual edits.
fn lane_header(ui: &mut egui::Ui, title: &str, lock: &parking_lot::RwLock<Lane>) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).small().weak());
        if ui
            .small_button("Reset")
            .on_hover_text("Snap all taps back onto the shape")
            .clicked()
        {
            lock.write().relink_all();
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-4, "expected {b}, got {a}");
    }

    #[test]
    fn every_theme_resolves_to_a_palette() {
        // One concrete look per Theme variant, with the derived fills landing
        // between the background and the hairline (so widgets stay legible).
        for idx in 0..Theme::variants().len() {
            let theme = Theme::from_index(idx);
            let pal = palette_for(theme);
            // faint_bg is a 0.45 blend of bg -> hairline, so each channel sits
            // within the [bg, hairline] span (order-independent check).
            for chan in [
                (pal.bg.r(), pal.faint_bg.r(), pal.hairline.r()),
                (pal.bg.g(), pal.faint_bg.g(), pal.hairline.g()),
                (pal.bg.b(), pal.faint_bg.b(), pal.hairline.b()),
            ] {
                let (lo, mid, hi) = (chan.0.min(chan.2), chan.1, chan.0.max(chan.2));
                assert!(lo <= mid && mid <= hi, "fill out of range for {theme:?}");
            }
        }
    }

    #[test]
    fn mix_interpolates_endpoints() {
        let a = egui::Color32::from_rgb(0, 0, 0);
        let b = egui::Color32::from_rgb(100, 200, 50);
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
        assert_eq!(mix(a, b, 0.5), egui::Color32::from_rgb(50, 100, 25));
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

    #[test]
    fn playable_taps_counts_what_fits_in_the_buffer() {
        // 1000 ms buffer, 250 ms spacing -> taps at 250,500,750,1000 fit (4);
        // a 5th at 1250 ms does not.
        assert_eq!(playable_taps(250.0, 8, 1000.0), 4);
        // Everything fits.
        assert_eq!(playable_taps(100.0, 8, 1000.0), 8);
        // Nothing fits past the very first overflow (spacing > buffer).
        assert_eq!(playable_taps(2000.0, 8, 1000.0), 0);
        // Non-positive step -> no cutoff.
        assert_eq!(playable_taps(0.0, 8, 1000.0), 8);
    }

    #[test]
    fn cutoff_x_is_none_when_all_taps_play() {
        let g = LaneGeom::new(rect_100(), false, 8);
        assert!(cutoff_x(&g, 8).is_none());
        assert!(cutoff_x(&g, 9).is_none());
        // With some taps out of range, the cutoff sits between the last playable
        // and first silent tap.
        let cx = cutoff_x(&g, 4).unwrap();
        assert!(cx > g.x_of(3) && cx < g.x_of(4));
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
        assert!(LaneGeom::new(rect_100(), false, 0)
            .nearest_tap(10.0)
            .is_none());
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
        approx(
            meter_norm(1.0),
            (0.0 - METER_MIN_DB) / (METER_MAX_DB - METER_MIN_DB),
        );
        assert_eq!(meter_norm(10.0), 1.0); // +20 dB clamps to the top
                                           // Monotonic: louder reads higher.
        assert!(meter_norm(0.5) > meter_norm(0.1));
    }
}

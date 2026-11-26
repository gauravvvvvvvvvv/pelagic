//! Line charts shared by the analysis panels (trials, seakeeping, forecasts):
//! recessive grid, legend for two or more series, hover read-out, colours
//! validated for the dark chart surface.
use super::theme;
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};

/// Categorical series colours, validated for the dark chart surface
/// (lightness band, chroma, CVD ΔE ≥ 9.4 between neighbours, ≥ 3:1 contrast).
pub(super) const SERIES: [Color32; 3] = [
    Color32::from_rgb(0x39, 0x87, 0xe5),
    Color32::from_rgb(0xd9, 0x59, 0x26),
    Color32::from_rgb(0x19, 0x9e, 0x70),
];
/// Chart surface the palette was validated against.
pub(super) const SURFACE: Color32 = Color32::from_rgb(12, 18, 24);

pub(super) struct Series<'a> {
    pub name: &'a str,
    pub colour: Color32,
    pub points: Vec<[f64; 2]>,
}

#[derive(Default)]
pub(super) struct Options {
    /// Equal scales on both axes (track plots).
    pub equal: bool,
    /// Include the origin in the ranges.
    pub origin: bool,
    /// Mark each point (sparse series).
    pub markers: bool,
    /// Horizontal reference lines.
    pub guides: Vec<f64>,
    /// Height available for the whole chart block (title, legend, plot).
    pub room: f32,
    /// A vertical marker (e.g. the selected forecast hour).
    pub marker_x: Option<f64>,
}

fn nice_step(range: f64) -> f64 {
    let raw = (range / 5.0).max(1e-9);
    let base = 10f64.powf(raw.log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * base)
        .find(|s| *s >= raw)
        .unwrap_or(10.0 * base)
}

fn tick_label(v: f64, step: f64) -> String {
    let digits = (-step.log10().floor()).clamp(0.0, 4.0) as usize;
    // Snap values within rounding of zero to 0 (no "-0").
    let v = if v.abs() < step * 1e-6 { 0.0 } else { v };
    format!("{v:.digits$}")
}

/// A line chart with recessive grid, legend for two or more series and a
/// hover read-out of the nearest values.
pub(super) fn chart(
    ui: &mut Ui,
    title: &str,
    series: &[Series],
    x_label: &str,
    y_label: &str,
    options: Options,
) {
    ui.label(RichText::new(title).strong().color(theme::STRONG));
    if series.len() >= 2 {
        ui.horizontal_wrapped(|ui| {
            for s in series {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(14.0, 4.0), Sense::hover());
                ui.painter().rect_filled(rect, 2.0, s.colour);
                ui.label(RichText::new(s.name).small().color(theme::TEXT));
                ui.add_space(6.0);
            }
        });
    }
    let width = ui.available_width().max(200.0);
    // Fit the visible column (minus selector rows, title, legend and the
    // caption) but keep a readable minimum.
    let room = if options.room > 0.0 {
        options.room - 120.0
    } else {
        360.0
    };
    let height = (width * 0.62).min(room).clamp(150.0, 420.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, SURFACE);
    let plot = Rect::from_min_max(
        rect.min + Vec2::new(48.0, 10.0),
        rect.max - Vec2::new(12.0, 32.0),
    );
    let all = series.iter().flat_map(|s| s.points.iter());
    let (mut x0, mut x1, mut y0, mut y1) = (
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    );
    for p in all.filter(|p| p[0].is_finite() && p[1].is_finite()) {
        x0 = x0.min(p[0]);
        x1 = x1.max(p[0]);
        y0 = y0.min(p[1]);
        y1 = y1.max(p[1]);
    }
    for g in &options.guides {
        y0 = y0.min(*g);
        y1 = y1.max(*g);
    }
    if !x0.is_finite() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "No data",
            egui::FontId::proportional(12.0),
            theme::MUTED,
        );
        return;
    }
    if options.origin {
        x0 = x0.min(0.0);
        y0 = y0.min(0.0);
        x1 = x1.max(0.0);
        y1 = y1.max(0.0);
    }
    let pad = |lo: &mut f64, hi: &mut f64| {
        let span = (*hi - *lo).max(1e-6);
        *lo -= span * 0.05;
        *hi += span * 0.05;
    };
    pad(&mut x0, &mut x1);
    pad(&mut y0, &mut y1);
    if options.equal {
        // Same metres per pixel on both axes.
        let sx = (x1 - x0) / plot.width() as f64;
        let sy = (y1 - y0) / plot.height() as f64;
        let s = sx.max(sy);
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        x0 = cx - s * plot.width() as f64 / 2.0;
        x1 = cx + s * plot.width() as f64 / 2.0;
        y0 = cy - s * plot.height() as f64 / 2.0;
        y1 = cy + s * plot.height() as f64 / 2.0;
    }
    let to_screen = |p: [f64; 2]| {
        Pos2::new(
            plot.left() + ((p[0] - x0) / (x1 - x0)) as f32 * plot.width(),
            plot.bottom() - ((p[1] - y0) / (y1 - y0)) as f32 * plot.height(),
        )
    };
    let grid = Stroke::new(1.0, Color32::from_rgb(28, 39, 50));
    let font = egui::FontId::proportional(10.5);
    let step = nice_step(x1 - x0);
    let mut x = (x0 / step).ceil() * step;
    while x <= x1 {
        let sx = to_screen([x, y0]).x;
        painter.line_segment(
            [Pos2::new(sx, plot.top()), Pos2::new(sx, plot.bottom())],
            grid,
        );
        painter.text(
            Pos2::new(sx, plot.bottom() + 3.0),
            egui::Align2::CENTER_TOP,
            tick_label(x, step),
            font.clone(),
            theme::MUTED,
        );
        x += step;
    }
    let step = nice_step(y1 - y0);
    let mut y = (y0 / step).ceil() * step;
    while y <= y1 {
        let sy = to_screen([x0, y]).y;
        painter.line_segment(
            [Pos2::new(plot.left(), sy), Pos2::new(plot.right(), sy)],
            grid,
        );
        painter.text(
            Pos2::new(plot.left() - 4.0, sy),
            egui::Align2::RIGHT_CENTER,
            tick_label(y, step),
            font.clone(),
            theme::MUTED,
        );
        y += step;
    }
    for g in &options.guides {
        let sy = to_screen([x0, *g]).y;
        painter.add(egui::Shape::dashed_line(
            &[Pos2::new(plot.left(), sy), Pos2::new(plot.right(), sy)],
            Stroke::new(1.0, Color32::from_rgb(70, 86, 100)),
            5.0,
            4.0,
        ));
    }
    painter.text(
        Pos2::new(plot.center().x, rect.bottom() - 4.0),
        egui::Align2::CENTER_BOTTOM,
        x_label,
        font.clone(),
        theme::MUTED,
    );
    painter.text(
        Pos2::new(rect.left() + 4.0, rect.top() + 2.0),
        egui::Align2::LEFT_TOP,
        y_label,
        font.clone(),
        theme::MUTED,
    );
    let clip = painter.with_clip_rect(plot.expand(4.0));
    for s in series {
        let line: Vec<Pos2> = s
            .points
            .iter()
            .filter(|p| p[0].is_finite() && p[1].is_finite())
            .map(|p| to_screen(*p))
            .collect();
        clip.add(egui::Shape::line(line.clone(), Stroke::new(2.0, s.colour)));
        if options.markers {
            for p in line {
                clip.circle(p, 4.0, s.colour, Stroke::new(2.0, SURFACE));
            }
        }
    }
    if let Some(x) = options.marker_x.filter(|x| (x0..=x1).contains(x)) {
        let sx = to_screen([x, y0]).x;
        clip.line_segment(
            [Pos2::new(sx, plot.top()), Pos2::new(sx, plot.bottom())],
            Stroke::new(1.5, theme::ACCENT),
        );
    }
    // Hover: the nearest point of each series to the pointer.
    if let Some(pointer) = response.hover_pos().filter(|p| plot.contains(*p)) {
        let mut lines = vec![];
        for s in series {
            let nearest = s
                .points
                .iter()
                .filter(|p| p[0].is_finite() && p[1].is_finite())
                .min_by(|a, b| {
                    let da = if options.equal {
                        to_screen(**a).distance(pointer)
                    } else {
                        (to_screen(**a).x - pointer.x).abs()
                    };
                    let db = if options.equal {
                        to_screen(**b).distance(pointer)
                    } else {
                        (to_screen(**b).x - pointer.x).abs()
                    };
                    da.total_cmp(&db)
                });
            if let Some(p) = nearest {
                let at = to_screen(*p);
                clip.circle(at, 4.0, s.colour, Stroke::new(2.0, SURFACE));
                lines.push(format!("{}: {:.3} at {:.2}", s.name, p[1], p[0]));
            }
        }
        if !options.equal {
            clip.line_segment(
                [
                    Pos2::new(pointer.x, plot.top()),
                    Pos2::new(pointer.x, plot.bottom()),
                ],
                Stroke::new(1.0, Color32::from_rgb(90, 108, 122)),
            );
        }
        response.on_hover_text(lines.join("\n"));
    }
}

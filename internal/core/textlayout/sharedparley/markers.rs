// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! Source markers with optional compact slots over unchanged source offsets.
use super::*;
use crate::items::{TextInput, TextInputMarkerKind};
use crate::lengths::LogicalVector;

pub(super) struct Marker {
    range: Range<usize>,
    kind: TextInputMarkerKind,
    foreground: Color,
    compact: bool,
}

pub(super) struct MarkerRendering {
    pub(super) spans: SelectionSpans,
    shape: PhysicalRect,
    range: Range<usize>,
    kind: TextInputMarkerKind,
    foreground: Color,
}

/// Evaluate model bindings before entering the font-context borrow.
pub(super) fn prepare(input: Pin<&TextInput>) -> Vec<Marker> {
    input
        .visual_markers()
        .into_iter()
        .map(|marker| Marker {
            range: marker.start as usize..marker.end as usize,
            kind: marker.kind,
            foreground: marker.foreground,
            compact: marker.compact,
        })
        .collect()
}

pub(super) fn resolve(
    layout: &Layout,
    markers: &[Marker],
    font_size: LogicalLength,
    scale: f32,
    band: Range<PhysicalLength>,
    mut can_paint: impl FnMut(&Color) -> bool,
) -> Vec<MarkerRendering> {
    let mut result = Vec::new();
    for marker in markers {
        let paragraphs = layout.visible_paragraphs();
        let index =
            paragraphs.partition_point(|p| p.range.start <= marker.range.start).saturating_sub(1);
        let Some(paragraph) = paragraphs.get(index) else {
            continue;
        };
        if marker.range.start < paragraph.range.start || marker.range.end > paragraph.range.end {
            continue;
        }
        let top = layout.y_offset + paragraph.y;
        let height = PhysicalLength::new(paragraph.layout.height());
        let pad = height / paragraph.layout.lines().len().max(1) as f32;
        // Offscreen paragraphs cannot paint markers; avoid a document-wide geometry scan for each.
        if top + height + pad < band.start || top - pad > band.end {
            continue;
        }
        // Cursor construction snaps to clusters. Reject any range that would hide extra bytes.
        if [marker.range.start, marker.range.end].into_iter().any(|offset| {
            let local = offset - paragraph.range.start;
            parley::editing::Cursor::from_byte_index(&paragraph.layout, local, Default::default())
                .index()
                != local
        }) {
            continue;
        }
        // Validate the entire prefix, independently of which fragments are currently visible.
        let all_lines = PhysicalLength::new(f32::NEG_INFINITY)..PhysicalLength::new(f32::INFINITY);
        let spans = layout.selection_geometry(marker.range.clone(), &all_lines);
        let mut rectangles = spans.backgrounds();
        let Some(rect) = rectangles.next() else {
            continue;
        };
        // Never hide source if wrapping or bidi splits its prefix into several fragments.
        if rectangles.next().is_some()
            || !rect.min_x().is_finite()
            || !rect.min_y().is_finite()
            || !rect.max_x().is_finite()
            || !rect.max_y().is_finite()
            || rect.width() <= 0.0
            || rect.height() <= 0.0
            || rect.max_y() < band.start.get()
            || rect.min_y() > band.end.get()
            || !can_paint(&marker.foreground)
        {
            continue;
        }
        // A declined compaction must leave the original source visible.
        if marker.compact && (rect.width() - font_size.get() * scale * 2.0).abs() > scale * 0.1 {
            continue;
        }
        let side = match marker.kind {
            TextInputMarkerKind::Bullet => font_size.get() * scale * 0.4,
            _ => font_size.get() * scale * (6.0 / 7.0),
        }
        .min(rect.width() * 0.75);
        if side < scale {
            continue;
        }
        let shape = PhysicalRect::new(
            PhysicalPoint::new(
                rect.min_x()
                    + if marker.compact {
                        font_size.get() * scale * 0.75 - side / 2.0
                    } else {
                        (rect.width() * 0.1).min(2.0 * scale)
                    },
                rect.min_y() + (rect.height() - side) / 2.0,
            ),
            PhysicalSize::new(side, side),
        );
        drop(rectangles);
        result.push(MarkerRendering {
            spans,
            shape,
            range: marker.range.clone(),
            kind: marker.kind,
            foreground: marker.foreground,
        });
    }
    result
}

impl MarkerRendering {
    pub(super) fn draw(&self, renderer: &mut impl GlyphRenderer) {
        let Some(brush) = renderer.platform_brush_for_color(&self.foreground) else {
            return;
        };
        let rect = self.shape;
        if self.kind == TextInputMarkerKind::Bullet {
            renderer.fill_rectangle(rect, brush, PhysicalLength::new(rect.width() / 2.0), None);
            return;
        }
        let scale = renderer.scale_factor().get();
        renderer.save_state();
        // Round the outer outline without painting over the paper or its pattern.
        renderer.combine_clip(
            LogicalRect::new(
                LogicalPoint::new(rect.min_x() / scale, rect.min_y() / scale),
                LogicalSize::new(rect.width() / scale, rect.height() / scale),
            ),
            LogicalBorderRadius::new_uniform(2.0_f32.min(rect.width() / scale / 4.0)),
        );
        let border = (1.1 * scale).min(rect.width() / 4.0);
        for edge in [
            PhysicalRect::new(rect.origin, PhysicalSize::new(rect.width(), border)),
            PhysicalRect::new(
                PhysicalPoint::new(rect.min_x(), rect.max_y() - border),
                PhysicalSize::new(rect.width(), border),
            ),
            PhysicalRect::new(rect.origin, PhysicalSize::new(border, rect.height())),
            PhysicalRect::new(
                PhysicalPoint::new(rect.max_x() - border, rect.min_y()),
                PhysicalSize::new(border, rect.height()),
            ),
        ] {
            renderer.fill_rectangle_with_color(edge, self.foreground);
        }
        if self.kind == TextInputMarkerKind::CheckedCheckbox {
            for (a, b) in [((0.23, 0.50), (0.43, 0.70)), ((0.43, 0.70), (0.78, 0.30))] {
                let dx: f32 = (b.0 - a.0) * rect.width();
                let dy: f32 = (b.1 - a.1) * rect.height();
                renderer.save_state();
                renderer.translate(LogicalVector::new(
                    (rect.min_x() + a.0 * rect.width()) / scale,
                    (rect.min_y() + a.1 * rect.height()) / scale,
                ));
                renderer.rotate(dy.atan2(dx).to_degrees());
                renderer.fill_rectangle(
                    PhysicalRect::new(
                        PhysicalPoint::new(-border / 2.0, -border / 2.0),
                        PhysicalSize::new(dx.hypot(dy) + border, border),
                    ),
                    brush.clone(),
                    PhysicalLength::new(border / 2.0),
                    None,
                );
                renderer.restore_state();
            }
        }
        renderer.restore_state();
    }
}


/// Hit the same shape that `resolve` hands to the painter.
pub(super) fn hit(
    rendered: Vec<MarkerRendering>, position: LogicalPoint, scale: f32,
) -> Option<crate::items::TextInputMarkerHit> {
    rendered.into_iter().find_map(|marker| {
        if marker.kind == TextInputMarkerKind::Bullet { return None; }
        let rect = LogicalRect::new(
            LogicalPoint::new(marker.shape.min_x() / scale, marker.shape.min_y() / scale),
            LogicalSize::new(marker.shape.width() / scale, marker.shape.height() / scale),
        );
        rounded_contains(rect, LogicalBorderRadius::new_uniform(2.0_f32.min(rect.width()/4.0)), position)
            .then_some(crate::items::TextInputMarkerHit {
                source_range: marker.range, kind: marker.kind, box_rect: rect,
            })
    })
}

pub(super) fn rounded_contains(rect: LogicalRect, radii: LogicalBorderRadius, point: LogicalPoint) -> bool {
    if !point.x.is_finite() || !point.y.is_finite() || !rect.contains(point) { return false; }
    let mut radii = [radii.top_left, radii.top_right, radii.bottom_right, radii.bottom_left];
    if radii.iter().any(|r| !r.is_finite()) { return false; }
    for r in &mut radii { *r = r.max(0.0); }
    let [tl,tr,br,bl] = radii;
    let fit = [rect.width()/(tl+tr),rect.width()/(bl+br),rect.height()/(tl+bl),rect.height()/(tr+br)]
        .into_iter().fold(1.0_f32,f32::min);
    let [tl,tr,br,bl] = radii.map(|r| r*fit);
    for (cx,cy,r,left,top) in [
        (rect.min_x()+tl,rect.min_y()+tl,tl,true,true),
        (rect.max_x()-tr,rect.min_y()+tr,tr,false,true),
        (rect.max_x()-br,rect.max_y()-br,br,false,false),
        (rect.min_x()+bl,rect.max_y()-bl,bl,true,false),
    ] {
        if (if left { point.x < cx } else { point.x > cx }) && (if top { point.y < cy } else { point.y > cy }) {
            return (point.x-cx).powi(2)+(point.y-cy).powi(2) <= r*r;
        }
    }
    true
}

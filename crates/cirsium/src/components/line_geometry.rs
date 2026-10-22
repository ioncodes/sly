use std::ops::Range;

use gpui::{Bounds, CaretPosition, Pixels, Point, ShapedLine};

pub(super) trait LineGeometry {
    fn caret_x(&self, index: usize, line_height: Pixels) -> Option<Pixels>;

    fn index_at(&self, point: Point<Pixels>, line_height: Pixels) -> usize;

    fn range_bounds(&self, range: Range<usize>, line_height: Pixels) -> Option<Bounds<Pixels>>;
}

impl LineGeometry for ShapedLine {
    fn caret_x(&self, index: usize, line_height: Pixels) -> Option<Pixels> {
        self.platform_layout
            .caret_bounds(CaretPosition::attached_to_next_cluster(index), line_height)
            .map(|caret| caret.origin.x)
    }

    fn index_at(&self, point: Point<Pixels>, line_height: Pixels) -> usize {
        self.platform_layout
            .caret_from_pixel_point(point, line_height)
            .unwrap_or_else(|caret| caret)
            .index
    }

    fn range_bounds(&self, range: Range<usize>, line_height: Pixels) -> Option<Bounds<Pixels>> {
        if range.is_empty() {
            return self.platform_layout.caret_bounds(
                CaretPosition::attached_to_next_cluster(range.start),
                line_height,
            );
        }

        self.platform_layout
            .selection_bounds(range, line_height)
            .into_iter()
            .reduce(|left, right| left.union(&right))
    }
}

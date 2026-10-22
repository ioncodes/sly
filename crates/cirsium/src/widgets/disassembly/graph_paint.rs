use super::*;

pub(super) fn paint_edges(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    offset: Point<Pixels>,
    edges: &[GraphEdge],
    hovered: Option<usize>,
    theme: &Theme,
) {
    let ox = bounds.origin.x + offset.x;
    let oy = bounds.origin.y + offset.y;
    let at = |p: (f32, f32)| point(ox + px(p.0), oy + px(p.1));

    let cull_left = f32::from(-offset.x);
    let cull_top = f32::from(-offset.y);
    let cull_right = cull_left + f32::from(bounds.size.width);
    let cull_bottom = cull_top + f32::from(bounds.size.height);

    for (ei, edge) in edges.iter().enumerate() {
        if edge.points.len() < 2 {
            continue;
        }

        let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
        let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
        for &(x, y) in &edge.points {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }

        if max_x < cull_left || min_x > cull_right || max_y < cull_top || min_y > cull_bottom {
            continue;
        }

        let base = match edge.kind {
            EdgeKind::Taken => theme.success,
            EdgeKind::FallThrough => theme.danger,
            EdgeKind::Normal => theme.accent,
        };
        let (color, width, head_len, head_w) = if hovered == Some(ei) {
            (crate::theme::lighten(base, 0.18), 2.75, 10., 5.)
        } else {
            (base, 1.5, 8., 4.)
        };

        let mut line = PathBuilder::stroke(px(width));
        line.move_to(at(edge.points[0]));
        for &p in &edge.points[1..] {
            line.line_to(at(p));
        }
        if let Ok(path) = line.build() {
            window.paint_path(path, color);
        }

        let n = edge.points.len();
        let (x0, y0) = edge.points[n - 2];
        let (x1, y1) = edge.points[n - 1];
        let (dx, dy) = (x1 - x0, y1 - y0);
        let len = (dx * dx + dy * dy).sqrt().max(0.001);
        let (ux, uy) = (dx / len, dy / len);
        let (nx, ny) = (-uy, ux);
        let (bx, by) = (x1 - ux * head_len, y1 - uy * head_len);

        let mut head = PathBuilder::fill();
        head.move_to(at((x1, y1)));
        head.line_to(at((bx + nx * head_w, by + ny * head_w)));
        head.line_to(at((bx - nx * head_w, by - ny * head_w)));
        head.line_to(at((x1, y1)));
        if let Ok(path) = head.build() {
            window.paint_path(path, color);
        }
    }
}

pub(super) fn edge_at(
    edges: &[GraphEdge],
    pos: Point<Pixels>,
    origin: Point<Pixels>,
    offset: Point<Pixels>,
) -> Option<usize> {
    graph::edge_at(
        edges,
        f32::from(pos.x - origin.x - offset.x),
        f32::from(pos.y - origin.y - offset.y),
    )
}

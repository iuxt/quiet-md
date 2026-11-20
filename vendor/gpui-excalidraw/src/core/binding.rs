use crate::{
    bounds::{Bounds, element_bounds},
    collision::hit_test_element,
    element::{BindMode, Element, ElementType, FixedPointBinding},
    geometry::Point,
    scene::Scene,
};

pub const BINDING_THRESHOLD: f64 = 18.0;

pub fn focus_point(element: &Element, from: Point) -> Point {
    let base = element.base();
    let w = base.width.max(0.0001);
    let h = base.height.max(0.0001);
    let center = Point::new(base.x + w / 2.0, base.y + h / 2.0);

    let local_from = from.rotate(-base.angle, center);
    let dx = local_from.x - center.x;
    let dy = local_from.y - center.y;

    if dx.abs() < 1e-9 && dy.abs() < 1e-9 {
        return Point::new(center.x + w / 2.0, center.y).rotate(base.angle, center);
    }

    let hw = w / 2.0;
    let hh = h / 2.0;

    let local = match element.kind() {
        ElementType::Diamond => {
            let nx = dx.abs() / hw;
            let ny = dy.abs() / hh;
            let sum = nx + ny;
            let s = if sum > 1e-9 { 1.0 / sum } else { 0.0 };
            Point::new(center.x + dx * s, center.y + dy * s)
        }
        ElementType::Ellipse => {
            let denom = ((dx / hw).powi(2) + (dy / hh).powi(2)).sqrt();
            let t = if denom > 1e-9 { 1.0 / denom } else { 0.0 };
            Point::new(center.x + dx * t, center.y + dy * t)
        }
        _ => {
            let tx = if dx.abs() > 1e-9 {
                hw / dx.abs()
            } else {
                f64::INFINITY
            };
            let ty = if dy.abs() > 1e-9 {
                hh / dy.abs()
            } else {
                f64::INFINITY
            };
            let t = tx.min(ty);
            if !t.is_finite() {
                Point::new(center.x + hw, center.y)
            } else {
                Point::new(center.x + dx * t, center.y + dy * t)
            }
        }
    };

    local.rotate(base.angle, center)
}

pub fn find_bindable(
    scene: &Scene,
    point: Point,
    threshold: f64,
    exclude: &[String],
) -> Option<String> {
    scene
        .non_deleted()
        .filter(|e| e.is_bindable() && !e.base().locked)
        .filter(|e| !exclude.iter().any(|id| id == e.id()))
        .find(|e| {
            hit_test_element(e, point, threshold)
                || element_bounds(e).expand(threshold).contains(point)
        })
        .map(|e| e.id().to_string())
}

pub fn make_binding(element: &Element, from: Point) -> FixedPointBinding {
    let b = element.base();
    let center = Point::new(b.x + b.width / 2.0, b.y + b.height / 2.0);
    let focus = focus_point(element, from).rotate(-b.angle, center);
    FixedPointBinding {
        element_id: element.id().to_string(),
        fixed_point: [((focus.x - b.x) / b.width.max(0.0001)).clamp(0.0, 1.0), ((focus.y - b.y) / b.height.max(0.0001)).clamp(0.0, 1.0)],
        mode: BindMode::Inside,
    }
}

fn bound_point(element: &Element, binding: &FixedPointBinding, from: Point, current: Point) -> Point {
    match binding.mode {
        BindMode::Inside => {
            let b = element.base();
            let center = Point::new(b.x + b.width / 2.0, b.y + b.height / 2.0);
            Point::new(b.x + b.width * binding.fixed_point[0], b.y + b.height * binding.fixed_point[1]).rotate(b.angle, center)
        }
        BindMode::Orbit => focus_point(element, from),
        BindMode::Skip => current,
    }
}

struct BindJob {
    id: String,
    start: Option<FixedPointBinding>,
    end: Option<FixedPointBinding>,
    points: Vec<Point>,
}

pub fn update_bindings(scene: &mut Scene) {
    let jobs: Vec<BindJob> = scene
        .non_deleted()
        .filter_map(|e| match e {
            Element::Arrow(a) => Some(BindJob {
                id: e.id().to_string(),
                start: a.linear.start_binding.clone(),
                end: a.linear.end_binding.clone(),
                points: a.linear.points.clone(),
            }),
            Element::Line(l) => Some(BindJob {
                id: e.id().to_string(),
                start: l.linear.start_binding.clone(),
                end: l.linear.end_binding.clone(),
                points: l.linear.points.clone(),
            }),
            _ => None,
        })
        .filter(|j| j.start.is_some() || j.end.is_some())
        .collect();

    for job in jobs {
        if job.points.len() < 2 {
            continue;
        }
        let n = job.points.len();
        let mut pts = job.points.clone();
        if let Some(binding) = &job.start
            && let Some(target) = scene.get(&binding.element_id)
        {
            pts[0] = bound_point(target, binding, pts[1], pts[0]);
        }
        if let Some(binding) = &job.end
            && let Some(target) = scene.get(&binding.element_id)
        {
            pts[n - 1] = bound_point(target, binding, pts[n - 2], pts[n - 1]);
        }
        let b = Bounds::from_points(&pts);
        if let Some(el) = scene.get_mut(&job.id) {
            match el {
                Element::Arrow(a) => a.linear.points = pts,
                Element::Line(l) => l.linear.points = pts,
                _ => continue,
            }
            let base = el.base_mut();
            base.x = b.min_x;
            base.y = b.min_y;
            base.width = b.width();
            base.height = b.height();
        }
    }
}

pub fn snap_to_grid(v: f64, grid: f64) -> f64 {
    if grid <= 0.0 {
        v
    } else {
        (v / grid).round() * grid
    }
}

pub fn sync_container_text(scene: &mut Scene) {
    let pad = 8.0;
    let jobs: Vec<_> = scene
        .non_deleted()
        .filter_map(|e| {
            if let Element::Text(t) = e {
                let cid = t.container_id.as_ref()?;
                Some((
                    t.base.id.clone(),
                    cid.clone(),
                    t.font_size,
                    t.line_height, t.font_family,
                    if t.original_text.is_empty() { t.text.clone() } else { t.original_text.clone() },
                ))
            } else {
                None
            }
        })
        .collect();

    for (_, cid, font_size, line_height, family, text) in &jobs {
        let Some(base) = scene.get(cid).map(Element::base) else {
            continue;
        };
        let inner_w = (base.width - pad * 2.0).max(*font_size);
        let lines = crate::text::wrap_text_in(text, inner_w, *font_size, *family)
            .split('\n')
            .count()
            .max(1);
        let text_h = lines as f64 * font_size * line_height;
        if let Some(Element::StickyNote(note)) = scene.get_mut(cid) {
            note.base.height = (text_h + pad * 2.0).max(note.base_height);
        }
    }

    for (id, cid, font_size, line_height, family, text) in jobs {
        if let Some(points) = scene.get(&cid).and_then(linear_points) {
            // Arrow labels sit at the path midpoint like upstream's bound text
            // and keep their intrinsic size instead of filling the container.
            let position = scene.get(&id).and_then(|e| if let Element::Text(t) = e { t.label_position } else { None }).unwrap_or(0.5);
            let Some(mid) = path_position(&points, position) else {
                continue;
            };
            let width = crate::text::text_width_in(&text, font_size, family);
            let height = crate::text::text_height(&text, font_size, line_height);
            if let Some(Element::Text(t)) = scene.get_mut(&id) {
                t.base.x = mid.x - width / 2.0;
                t.base.y = mid.y - height / 2.0;
                t.base.width = width;
                t.base.height = height;
                t.base.angle = 0.0;
                t.text = text;
            }
            continue;
        }

        let Some(base) = scene.get(&cid).map(Element::base) else {
            continue;
        };
        let bounds = Bounds::new(base.x, base.y, base.x + base.width, base.y + base.height);
        let angle = base.angle;
        let pivot = bounds.center();
        let inner_w = (bounds.width() - pad * 2.0).max(font_size);
        let wrapped = crate::text::wrap_text_in(&text, inner_w, font_size, family);
        let text_h = crate::text::text_height(&wrapped, font_size, line_height);
        let x = bounds.min_x + pad;
        let spare = (bounds.height() - text_h).max(0.0);
        let inset = pad.min(spare / 2.0);
        if let Some(Element::Text(t)) = scene.get_mut(&id) {
            let y = bounds.min_y + match t.vertical_align {
                crate::types::VerticalAlign::Top => inset,
                crate::types::VerticalAlign::Middle => spare / 2.0,
                crate::types::VerticalAlign::Bottom => spare - inset,
            };
            let center = Point::new(x + inner_w / 2.0, y + text_h / 2.0).rotate(angle, pivot);
            t.base.x = center.x - inner_w / 2.0;
            t.base.y = center.y - text_h / 2.0;
            t.base.angle = angle;
            t.base.width = inner_w;
            t.base.height = text_h;
            t.text = wrapped;
        }
    }
}

fn linear_points(element: &Element) -> Option<Vec<Point>> {
    let points = match element {
        Element::Arrow(a) => Some(a.linear.points.clone()),
        Element::Line(l) => Some(l.linear.points.clone()),
        _ => None,
    }?;
    let b = element.base();
    let pivot = Point::new(b.x + b.width / 2.0, b.y + b.height / 2.0);
    Some(points.into_iter().map(|p| p.rotate(b.angle, pivot)).collect())
}

/// Point half way along a polyline, matching upstream's
/// `getPointAtPathParameter(element, 0.5)` used to place arrow labels.
fn path_position(points: &[Point], position: f64) -> Option<Point> {
    if points.len() < 2 {
        return points.first().copied();
    }
    let total: f64 = points
        .windows(2)
        .map(|pair| pair[0].distance(pair[1]))
        .sum();
    if total <= 1e-9 {
        return points.first().copied();
    }
    let target = total * position.clamp(0.0, 1.0);
    let mut walked = 0.0;
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let len = a.distance(b);
        if walked + len >= target {
            let t = if len > 1e-9 {
                (target - walked) / len
            } else {
                0.0
            };
            return Some(a.lerp(b, t));
        }
        walked += len;
    }
    points.last().copied()
}

pub fn refresh_derived(scene: &mut Scene) {
    update_bindings(scene);
    sync_container_text(scene);
}

pub fn constrain_angle(origin: Point, p: Point, step_deg: f64) -> Point {
    let d = p - origin;
    let len = d.length();
    if len < 1e-9 {
        return p;
    }
    let step = step_deg.to_radians();
    let angle = d.y.atan2(d.x);
    let snapped = (angle / step).round() * step;
    origin + Point::new(snapped.cos(), snapped.sin()) * len
}

pub fn constrain_square(start: Point, current: Point) -> Point {
    let dx = current.x - start.x;
    let dy = current.y - start.y;
    let size = dx.abs().max(dy.abs());
    Point::new(
        start.x + size * if dx < 0.0 { -1.0 } else { 1.0 },
        start.y + size * if dy < 0.0 { -1.0 } else { 1.0 },
    )
}

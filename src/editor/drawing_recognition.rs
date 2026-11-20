//! Native port of Excalidraw's convertToShape recognition (MIT, see THIRD_PARTY.md).
use super::{scene, World, Tool};

fn samples(points: &[World], total: f64) -> Vec<World> {
    let mut result = vec![points[0]]; let mut traversed = 0.;
    for segment in points.windows(2) {
        let length = segment[0].distance(segment[1]);
        while length > 0. && result.len() < 63 {
            let offset = total * result.len() as f64 / 63. - traversed;
            if offset > length { break; }
            result.push(segment[0].lerp(segment[1], (offset / length).clamp(0., 1.)));
        }
        traversed += length;
    }
    result.push(*points.last().unwrap()); result
}

fn hull_area(points: &[World]) -> f64 {
    let mut sorted = points.to_vec(); sorted.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    let half = |iter: &[World]| {
        let mut hull: Vec<World> = Vec::new();
        for &p in iter {
            while hull.len() >= 2 {
                let b = hull[hull.len()-1]; let a = hull[hull.len()-2];
                if (b-a).cross(p-b) > 0. { break; }
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop(); hull
    };
    let mut hull = half(&sorted); sorted.reverse(); hull.extend(half(&sorted));
    hull.iter().zip(hull.iter().cycle().skip(1)).map(|(a, b)| a.cross(*b)).sum::<f64>().abs() / 2.
}

fn corner_share(points: &[World]) -> f64 {
    let mut turns: Vec<_> = points.windows(7).map(|w| {
        let a = w[3] - w[0]; let b = w[6] - w[3]; a.cross(b).atan2(a.dot(b)).abs()
    }).collect();
    let total: f64 = turns.iter().sum(); if total == 0. { return 0.; }
    let mut share = 0.;
    for _ in 0..4 {
        let Some((peak, _)) = turns.iter().enumerate().max_by(|(_,a),(_,b)| a.total_cmp(b)) else { break };
        let end = (peak + 4).min(turns.len());
        for turn in &mut turns[peak.saturating_sub(3)..end] { share += *turn; *turn = 0.; }
    }
    share / total
}

pub(super) fn recognize(points: &[World], zoom: f64) -> (Tool, World) {
    let last = points.last().copied().unwrap_or_default(); let fallback = (Tool::Pen, last);
    if points.len() < 3 { return fallback; }
    let bounds = scene::Bounds::from_points(points);
    if bounds.width().max(bounds.height()) * zoom < 25. { return fallback; }
    let length = |p: &[World]| p.windows(2).map(|w| w[0].distance(w[1])).sum::<f64>();
    let pts = samples(points, length(points)); let start = pts[0];
    let n = pts.len() as f64; let mean = pts.iter().copied().fold(World::zero(), |a,b| a+b) * (1./n);
    let centered: Vec<_> = pts.iter().map(|p| *p-mean).collect();
    let moment = |f: &dyn Fn(World) -> f64| centered.iter().map(|p| f(*p)).sum::<f64>()/n;
    let xx = moment(&|p| p.x*p.x); let yy = moment(&|p| p.y*p.y);
    if start.distance(last) > 0.15 * length(&pts) {
        let xy = moment(&|p| p.x*p.y);
        let diff = (xx-yy).hypot(2.*xy); let major = (xx+yy+diff)/2.;
        if major <= 0. || (xx+yy-diff)/(2.*major) > 0.25 { return fallback; }
        let tip = *pts.iter().max_by(|a,b| start.distance(**a).total_cmp(&start.distance(**b))).unwrap();
        let span = start.distance(tip);
        if pts.iter().any(|&p| p.distance(tip) > 0.5*span && scene::collision::distance_to_segment(p,start,tip) > 0.15*span) { return fallback; }
        let theta = (2.*xy).atan2(xx-yy)/2.; let axis = World::new(theta.cos(), theta.sin());
        if moment(&|p| p.dot(axis).powi(3)).abs() < 0.3*major.powf(1.5) { return (Tool::Line, last); }
        let corners = super::rect_corners(bounds);
        let ideal = corners.iter().enumerate().flat_map(|(i,c)| [*c,c.midpoint(corners[(i+1)%4])])
            .max_by(|a,b| start.distance(*a).total_cmp(&start.distance(*b))).unwrap();
        let tip = points.iter().min_by(|a,b| ideal.distance(**a).total_cmp(&ideal.distance(**b))).unwrap();
        return (Tool::Arrow, *tip);
    }
    if xx <= 0. || yy <= 0. { return fallback; }
    let b = scene::Bounds::from_points(&pts);
    let fill = hull_area(&pts)/(b.width()*b.height());
    let kurt = moment(&|p| p.x.powi(4)) / xx.powi(2) * moment(&|p| p.y.powi(4)) / yy.powi(2);
    let turn = corner_share(&pts);
    let mut best = Tool::Pen; let mut distance = 1.5;
    for (tool, area, corners, product) in [(Tool::Rect, 1., 0.95, 1.83), (Tool::Diamond, 0.5, 0.95, 3.24), (Tool::Ellipse, std::f64::consts::FRAC_PI_4, 0.55, 2.25)] {
        let error = ((fill-area)/0.2).hypot((turn-corners)/0.2).hypot((kurt-product)/0.7);
        if error < distance { best = tool; distance = error; }
    }
    (best, last)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recognizes_closed_and_open_strokes_at_canvas_zoom() {
        let p = |x, y| World::new(x, y);
        let rect = [p(0.,0.),p(100.,0.),p(100.,100.),p(0.,100.),p(0.,0.)];
        assert_eq!(recognize(&rect, 1.).0, Tool::Rect);
        assert_eq!(recognize(&rect, 0.1).0, Tool::Pen);
        assert_eq!(recognize(&[p(0.,50.),p(50.,0.),p(100.,50.),p(50.,100.),p(0.,50.)],1.).0, Tool::Diamond);
        let circle: Vec<_> = (0..=80).map(|n| { let a = n as f64 * std::f64::consts::TAU / 80.; p(50.*a.cos(),50.*a.sin()) }).collect();
        assert_eq!(recognize(&circle, 1.).0, Tool::Ellipse);
        assert_eq!(recognize(&[p(0.,0.),p(50.,50.),p(100.,100.)],1.).0, Tool::Line);
    }
}

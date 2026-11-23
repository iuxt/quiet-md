//! Orthogonal routing between bound arrow endpoints, based on Excalidraw's elbowArrow.ts.
use super::{Element, World, element_bounds, element_center, scene};
use scene::{Bounds, element::FixedPointBinding};
use std::{cmp::Ordering, collections::BinaryHeap};

const PADDING: f64 = 40.;

#[derive(Clone, Copy)]
struct Anchor { point: World, heading: World, bounds: Option<Bounds> }

fn axis(delta: World) -> World {
    if delta.x.abs() >= delta.y.abs() { World::new(delta.x.signum(), 0.) }
    else { World::new(0., delta.y.signum()) }
}

fn anchor(point: World, other: World, binding: &Option<FixedPointBinding>, targets: &std::collections::HashMap<&str, &Element>) -> Anchor {
    let Some(target) = binding.as_ref().and_then(|binding| targets.get(binding.element_id.as_str())) else {
        return Anchor { point, heading: axis(other - point), bounds: None };
    };
    let bounds = element_bounds(target);
    let center = element_center(target);
    let delta = point - center;
    let direction = World::new(delta.x / bounds.width().max(1.), delta.y / bounds.height().max(1.));
    let heading = if direction.length() > 0.001 { axis(direction) } else { axis(other - point) };
    Anchor { point, heading, bounds: Some(bounds) }
}

fn interior_crossed(a: World, b: World, r: Bounds) -> bool {
    if a.x == b.x { r.min_x < a.x && a.x < r.max_x && a.y.max(b.y) > r.min_y && a.y.min(b.y) < r.max_y }
    else if a.y == b.y { r.min_y < a.y && a.y < r.max_y && a.x.max(b.x) > r.min_x && a.x.min(b.x) < r.max_x }
    else { true }
}

fn distance_to_edge(anchor: Anchor) -> f64 {
    let Some(bounds) = anchor.bounds else { return 0. };
    let edge = if anchor.heading.x > 0. { bounds.max_x - anchor.point.x }
        else if anchor.heading.x < 0. { anchor.point.x - bounds.min_x }
        else if anchor.heading.y > 0. { bounds.max_y - anchor.point.y }
        else { anchor.point.y - bounds.min_y };
    (edge + PADDING).max(PADDING)
}

fn simple_route(start: Anchor, end: Anchor) -> Option<Vec<World>> {
    let first = (end.point - start.point).dot(start.heading);
    let last = (start.point - end.point).dot(end.heading);
    if first <= 0. || last <= 0. { return None; }
    let (p, q) = if start.heading.dot(end.heading) == 0. {
        let corner = start.point + start.heading * first; (corner, corner)
    } else if start.heading.dot(end.heading) == -1. {
        (start.point + start.heading * (first / 2.), end.point + end.heading * (last / 2.))
    } else { return None; };
    if start.bounds.is_some_and(|b| interior_crossed(p,q,b) || interior_crossed(q,end.point,b))
        || end.bounds.is_some_and(|b| interior_crossed(start.point,p,b) || interior_crossed(p,q,b)) { return None; }
    Some(vec![start.point,p,q,end.point])
}

#[derive(Clone, Copy)]
struct Step { cost: f64, node: usize }
impl PartialEq for Step { fn eq(&self, other: &Self) -> bool { self.cmp(other).is_eq() } }
impl Eq for Step {}
impl PartialOrd for Step { fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) } }
impl Ord for Step {
    fn cmp(&self, other: &Self) -> Ordering { other.cost.total_cmp(&self.cost).then(other.node.cmp(&self.node)) }
}

fn grid_route(start: Anchor, end: Anchor) -> Option<Vec<World>> {
    let mut exit = start.point + start.heading * distance_to_edge(start);
    let mut entry = end.point + end.heading * distance_to_edge(end);
    if end.bounds.is_some_and(|b| interior_crossed(start.point,exit,b)) { exit = start.point; }
    if start.bounds.is_some_and(|b| interior_crossed(end.point,entry,b)) { entry = end.point; }
    let obstacles: Vec<_> = [start.bounds, end.bounds].into_iter().flatten().collect();
    let mut xs = vec![exit.x,entry.x]; let mut ys = vec![exit.y,entry.y];
    for bound in &obstacles {
        xs.extend([bound.min_x-PADDING,bound.max_x+PADDING]);
        ys.extend([bound.min_y-PADDING,bound.max_y+PADDING]);
    }
    for coords in [&mut xs,&mut ys] { coords.sort_by(f64::total_cmp); coords.dedup(); }
    let width = xs.len(); let height = ys.len();
    let point = |i: usize| World::new(xs[i % width],ys[i / width]);
    let cell = |q: World| ys.iter().position(|y| *y == q.y).unwrap()*width + xs.iter().position(|x| *x == q.x).unwrap();
    let first = cell(exit)*2 + usize::from(start.heading.x == 0.);
    let last = cell(entry);
    let segment_cost = exit.distance(entry) + (xs[width-1]-xs[0]) + (ys[height-1]-ys[0]) + PADDING;
    let mut costs = vec![f64::INFINITY; width*height*2]; let mut previous = vec![first; costs.len()];
    let mut queue = BinaryHeap::from([Step { cost: 0., node: first }]); costs[first] = 0.;
    while let Some(Step { cost,node }) = queue.pop() {
        if cost > costs[node] { continue; }
        let i = node / 2;
        let neighbors = [
            (i % width > 0).then(|| (i-1,0)), (i % width + 1 < width).then_some((i+1,0)),
            (i / width > 0).then(|| (i-width,1)), (i / width + 1 < height).then_some((i+width,1)),
        ];
        for (j, direction) in neighbors.into_iter().flatten() {
            let (a,b) = (point(i),point(j));
            if obstacles.iter().any(|r| interior_crossed(a,b,*r)) { continue; }
            let next = 2*j + direction;
            let candidate = cost + a.distance(b) + if direction == node%2 { 0. } else { segment_cost };
            if candidate < costs[next] {
                costs[next] = candidate; previous[next] = node; queue.push(Step { cost: candidate,node: next });
            }
        }
    }
    let mut node = (last*2..=last*2+1).filter(|&n| costs[n].is_finite()).min_by(|&a,&b| {
        let heading = usize::from(end.heading.x == 0.);
        let adjusted = |n: usize| costs[n] + if n%2 == heading { 0. } else { segment_cost };
        adjusted(a).total_cmp(&adjusted(b))
    })?;
    let mut path = vec![end.point,entry];
    while node != first { node = previous[node]; path.push(point(node/2)); }
    path.push(start.point); path.reverse(); Some(path)
}

pub(super) fn bound_route(start: World, end: World, arrow: &scene::element::ArrowElement, targets: &std::collections::HashMap<&str, &Element>) -> Option<Vec<World>> {
    let a = anchor(start,end,&arrow.linear.start_binding,targets); let b = anchor(end,start,&arrow.linear.end_binding,targets);
    if a.bounds.is_none() && b.bounds.is_none() { return None; }
    let mut points = simple_route(a,b).or_else(|| grid_route(a,b))?;
    points.dedup();
    let mut i = 1;
    while i + 1 < points.len() {
        let incoming = points[i] - points[i-1]; let outgoing = points[i+1] - points[i];
        if incoming.cross(outgoing) == 0. && incoming.dot(outgoing) >= 0. { points.remove(i); }
        else { i += 1; }
    }
    for point in &mut points {
        point.x = point.x.clamp(-super::MAX_COORD, super::MAX_COORD);
        point.y = point.y.clamp(-super::MAX_COORD, super::MAX_COORD);
    }
    Some(points)
}

pub(super) fn maintain_fixed_route(arrow: &mut scene::element::ArrowElement) {
    let points = &mut arrow.linear.points;
    if points.len() < 2 || points.windows(2).all(|p| p[0].x == p[1].x || p[0].y == p[1].y) { return; }
    let start = points[0]; let end = *points.last().unwrap();
    let segments = arrow.fixed_segments.as_mut().unwrap();
    segments.sort_by_key(|segment| segment.index);
    let mut path = vec![start];
    for segment in segments {
        let previous = *path.last().unwrap();
        let vertical = segment.start.x == segment.end.x;
        if previous.x != segment.start.x && previous.y != segment.start.y {
            path.push(if vertical { World::new(previous.x,segment.start.y) } else { World::new(segment.start.x,previous.y) });
        }
        if path.last() != Some(&segment.start) { path.push(segment.start); }
        path.push(segment.end); segment.index = path.len()-1;
    }
    let previous = *path.last().unwrap();
    if previous.x != end.x && previous.y != end.y {
        let prior = path[path.len()-2];
        path.push(if previous.x == prior.x { World::new(end.x,previous.y) } else { World::new(previous.x,end.y) });
    }
    path.push(end);
    *points = path;
    let b = Bounds::from_points(points);
    let base = &mut arrow.linear.base;
    (base.x,base.y,base.width,base.height) = (b.min_x,b.min_y,b.width(),b.height());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::drawing::{factory, ElementOptions};

    #[test]
    fn same_side_bindings_route_outside_both_shapes() {
        let opts = ElementOptions::default();
        let left = factory::new_rectangle(0., 0., 100., 100., &opts);
        let right = factory::new_rectangle(240., 20., 100., 100., &opts);
        let start = World::new(50., 0.);
        let end = World::new(290., 20.);
        let Element::Arrow(mut arrow) = factory::new_arrow(vec![start, end], None, None, &opts) else { unreachable!() };
        arrow.linear.start_binding = Some(scene::binding::make_binding(&left,start));
        arrow.linear.end_binding = Some(scene::binding::make_binding(&right,end));
        let points = bound_route(start,end,&arrow,&[(left.id(),&left),(right.id(),&right)].into_iter().collect()).unwrap();
        assert_eq!(points.first(),Some(&start)); assert_eq!(points.last(),Some(&end));
        assert!(points[1].y < start.y && points[points.len()-2].y < end.y);
        assert!(points.windows(2).all(|p| !interior_crossed(p[0],p[1],element_bounds(&left)) && !interior_crossed(p[0],p[1],element_bounds(&right))));
    }
}

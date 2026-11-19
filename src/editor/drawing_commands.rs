//! Precision fields and secondary tools use the same draft and undo history.
use super::*;
use scene::bounds::Bounds as Rect;

#[derive(Clone, Copy)]
pub(super) enum Property { Color(chrome::Colors), X, Y, Width, Height, Angle, FontSize, StrokeWidth }

impl Property {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Color(_) => tr("自定义颜色", "Custom color"), Self::X => "X", Self::Y => "Y", Self::Width => tr("宽度", "Width"),
            Self::Height => tr("高度", "Height"), Self::Angle => tr("角度", "Angle"), Self::FontSize => tr("字体大小", "Font size"), Self::StrokeWidth => tr("描边宽度", "Stroke width"),
        }
    }
}

fn normalized_color(value: &str) -> Option<String> {
    if value == "transparent" { return Some(value.into()); }
    let hex = value.strip_prefix('#').unwrap_or(value);
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) { return None; }
    match hex.len() {
        3 => Some(format!("#{}", hex.chars().flat_map(|c| [c, c]).collect::<String>()).to_ascii_lowercase()),
        6 => Some(format!("#{hex}").to_ascii_lowercase()), _ => None,
    }
}

fn can_contain_text(e: &Element) -> bool {
    matches!(e, Element::Rectangle { .. } | Element::Diamond { .. } | Element::Ellipse { .. } | Element::StickyNote(_) | Element::Arrow(_) | Element::Line(_))
}

fn inside_polygon(points: &[World], p: World) -> bool {
    if points.len() < 3 { return false; }
    let mut inside = false; let mut previous = points[points.len() - 1];
    for &q in points {
        if (q.y > p.y) != (previous.y > p.y) && p.x < q.x + (p.y - q.y) / (previous.y - q.y) * (previous.x - q.x) { inside = !inside; }
        previous = q;
    }
    inside
}

pub(super) fn snap_delta(doc: &Scene, selected: &[usize], delta: World, threshold: f64) -> (World, Vec<(World, World)>) {
    let Some(from) = selection_bounds(doc, selected) else { return (delta, Vec::new()) };
    let excluded = selection_indices(doc, selected);
    let mut best = [threshold; 2]; let mut correction = [0.; 2]; let mut guides = [None, None];
    let moving = Rect::new(from.min_x + delta.x, from.min_y + delta.y, from.max_x + delta.x, from.max_y + delta.y);
    for (i, e) in doc.elements.iter().enumerate() {
        if e.is_deleted() || excluded.contains(&i) || matches!(e, Element::Text(t) if t.container_id.is_some()) { continue; }
        let b = element_bounds(e);
        for axis in 0..2 {
            let (a, targets) = if axis == 0 { ([moving.min_x, moving.center().x, moving.max_x], [b.min_x, b.center().x, b.max_x]) }
                else { ([moving.min_y, moving.center().y, moving.max_y], [b.min_y, b.center().y, b.max_y]) };
            for p in a { for q in targets {
                if (q - p).abs() < best[axis] {
                    best[axis] = (q - p).abs(); correction[axis] = q - p;
                    guides[axis] = Some(if axis == 0 { (World::new(q, moving.min_y.min(b.min_y) - 10.), World::new(q, moving.max_y.max(b.max_y) + 10.)) }
                        else { (World::new(moving.min_x.min(b.min_x) - 10., q), World::new(moving.max_x.max(b.max_x) + 10., q)) });
                }
            } }
        }
    }
    (delta + World::new(correction[0], correction[1]), guides.into_iter().flatten().collect())
}

impl DrawingEditor {
    pub(super) fn start_property(&mut self, property: Property, value: String, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_field(window, cx);
        self.edit_property = Some(property);
        self.field.update(cx, |f, cx| { f.multiline = false; f.set_placeholder(property.label(), cx); f.set_text(&value, cx); f.select_all_text(cx); });
        window.focus(&self.field.read(cx).focus_handle(cx)); cx.notify();
    }

    pub(super) fn finish_property(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(property) = self.edit_property.take() else { return };
        let value = self.field.read(cx).text.trim().to_string();
        match property {
            Property::Color(target) => if let Some(color) = normalized_color(&value) { self.apply_color(target, &color, cx); },
            _ => if let Ok(value) = value.parse::<f64>() && value.is_finite() {
                match property {
                    Property::FontSize if (1. ..=512.).contains(&value) => self.apply_style(StyleChoice::TextSize(value), true, window, cx),
                    Property::StrokeWidth if (0.1..=64.).contains(&value) => self.apply_style(StyleChoice::Width(value), true, window, cx),
                    _ => if let Some(b) = selection_bounds(&self.doc.scene, &self.selection) {
                        let mut to = b; let mut angle = 0.;
                        match property {
                            Property::X if value.abs() <= MAX_COORD => { to.min_x = value; to.max_x = value + b.width(); }
                            Property::Y if value.abs() <= MAX_COORD => { to.min_y = value; to.max_y = value + b.height(); }
                            Property::Width if (1. ..=MAX_COORD).contains(&value) => to.max_x = to.min_x + value,
                            Property::Height if (1. ..=MAX_COORD).contains(&value) => to.max_y = to.min_y + value,
                            Property::Angle => {
                                let current = self.selected().map_or(0., |i| self.doc.scene.elements[i].base().angle);
                                angle = value.rem_euclid(360.).to_radians() - current;
                            }
                            _ => {}
                        }
                        if to != b || angle.abs() > 0.000001 {
                            self.checkpoint();
                            if matches!(property, Property::Width | Property::Height) { edit::resize_selection(&mut self.doc.scene, &self.selection, b, to); }
                            else { transform_selection(&mut self.doc.scene, &self.selection, b, to, angle); }
                            self.touch_selection(); self.sync_frames(); self.rebuild();
                        }
                    }
                }
            }
        }
        window.focus(&self.focus); cx.notify();
    }

    pub(super) fn apply_color(&mut self, target: chrome::Colors, color: &str, cx: &mut Context<Self>) {
        use chrome::Colors;
        if target == Colors::Paper { self.change_paper(color, cx); return; }
        match target { Colors::Stroke => self.style.stroke_color = color.into(), Colors::Fill => self.style.background_color = color.into(), Colors::Text => self.text_color = color.into(), Colors::Paper => {} }
        let mut indices = self.selection.clone();
        if target == Colors::Text { indices = indices.iter().filter_map(|i| {
            let e = &self.doc.scene.elements[*i]; if matches!(e, Element::Text(_)) { Some(*i) } else { self.bound_text_index(e) }
        }).collect(); }
        if !indices.is_empty() {
            self.checkpoint();
            for i in indices {
                let e = &mut self.doc.scene.elements[i];
                if target == Colors::Fill { e.base_mut().background_color = color.into(); }
                else { e.base_mut().stroke_color = color.into(); }
                touch(e);
            }
            self.rebuild();
        }
        cx.notify();
    }

    pub(super) fn pick_color(&mut self, p: World, cx: &mut Context<Self>) {
        let color = self.doc.scene.elements.iter().rev().filter(|e| !e.is_deleted()).find_map(|e| {
            let b = e.base();
            let at = if matches!(e, Element::Line(_) | Element::Arrow(_) | Element::Freedraw(_)) { p.rotate(-b.angle, element_center(e)) } else { p };
            scene::collision::hit_test_element(e, at, 7. / self.zoom).then(|| {
                if matches!(self.eyedropper, chrome::Colors::Fill | chrome::Colors::Paper) && b.background_color != "transparent" { b.background_color.clone() } else { b.stroke_color.clone() }
            })
        }).unwrap_or_else(|| self.doc.background.clone());
        self.apply_color(self.eyedropper, &color, cx); self.tool = Tool::Select; cx.notify();
    }

    pub(super) fn fill_at(&mut self, p: World, cx: &mut Context<Self>) {
        let hit = self.doc.scene.elements.iter().enumerate().rev().find_map(|(i, e)| {
            if e.is_deleted() || e.base().locked || !matches!(e, Element::Rectangle { .. } | Element::Diamond { .. } | Element::Ellipse { .. } | Element::Line(_) | Element::StickyNote(_)) { return None; }
            let hit = if let Element::Line(l) = e { inside_polygon(&l.linear.points, p.rotate(-e.base().angle, element_center(e))) } else { scene::collision::hit_test_element(e, p, 7. / self.zoom) };
            hit.then_some(i)
        });
        if let Some(i) = hit {
            self.checkpoint(); let e = &mut self.doc.scene.elements[i];
            e.base_mut().background_color = if self.style.background_color == "transparent" { self.style.stroke_color.clone() } else { self.style.background_color.clone() };
            if let Element::Line(l) = e { l.polygon = true; }
            touch(e); self.rebuild(); cx.notify();
        }
    }

    pub(super) fn convert_selection(&mut self, backwards: bool, cx: &mut Context<Self>) {
        let mut changes = Vec::new();
        for &i in &self.selection {
            let e = &self.doc.scene.elements[i]; let targets = e.convertible_targets();
            if let Some(n) = targets.iter().position(|t| Some(*t) == e.convert_target()) {
                let n = (n + if backwards { targets.len() - 1 } else { 1 }) % targets.len();
                if let Some(mut e) = e.convert_to(targets[n]) {
                    if let Element::Arrow(a) = &mut e && a.elbowed { edit::elbow_points(a); }
                    changes.push((i, e));
                }
            }
        }
        if changes.is_empty() { return; }
        self.checkpoint(); for (i, e) in changes { self.doc.scene.elements[i] = e; }
        edit::refresh_scene(&mut self.doc.scene); self.line_edit = None; self.rebuild(); cx.notify();
    }

    pub(super) fn toggle_text_wrap(&mut self, window: &Window, cx: &mut Context<Self>) {
        let indices: Vec<_> = self.selection.iter().filter_map(|i| matches!(&self.doc.scene.elements[*i], Element::Text(t) if t.container_id.is_none()).then_some(*i)).collect();
        if indices.is_empty() { return; }
        self.checkpoint();
        for i in indices {
            if let Element::Text(t) = &mut self.doc.scene.elements[i] {
                t.auto_resize = !t.auto_resize;
                t.text = text_to_edit(t).to_string();
                if !t.auto_resize { t.text = scene::text::wrap_text(&t.text, t.base.width, t.font_size); }
                resize_text(t, window);
            }
            touch(&mut self.doc.scene.elements[i]);
        }
        self.rebuild(); cx.notify();
    }

    pub(super) fn toggle_text_binding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.selection.iter().copied().find(|i| matches!(self.doc.scene.elements[*i], Element::Text(_)))
            .or_else(|| self.selected().and_then(|i| self.bound_text_index(&self.doc.scene.elements[i])));
        if let Some(text) = text {
            let Element::Text(t) = &self.doc.scene.elements[text] else { return };
            let old = t.container_id.clone();
            let container = if old.is_some() { None } else {
                self.selection.iter().copied().find(|i| can_contain_text(&self.doc.scene.elements[*i]) && self.bound_text_index(&self.doc.scene.elements[*i]).is_none())
                    .or_else(|| self.doc.scene.elements.iter().enumerate().rev().find_map(|(i, e)| {
                        (!e.is_deleted() && !e.base().locked && can_contain_text(e) && self.bound_text_index(e).is_none() && element_bounds(e).contains(element_center(&self.doc.scene.elements[text]))).then_some(i)
                    }))
            };
            if old.is_none() && container.is_none() { return; }
            let text_id = self.doc.scene.elements[text].id().to_string();
            self.checkpoint();
            if let Some(old) = old && let Some(e) = self.doc.scene.get_mut(&old)
                && let Some(bound) = &mut e.base_mut().bound_elements { bound.retain(|b| b.id != text_id); touch(e); }
            let id = text_id;
            let new_container = container.map(|i| {
                let e = &mut self.doc.scene.elements[i];
                e.base_mut().bound_elements.get_or_insert_with(Vec::new).push(scene::element::BoundElement { id, kind: scene::element::BoundElementType::Text });
                touch(e); e.id().to_string()
            });
            if let Element::Text(t) = &mut self.doc.scene.elements[text] { t.container_id = new_container; t.auto_resize = true; t.text = text_to_edit(t).to_owned(); resize_text(t, window); }
            touch(&mut self.doc.scene.elements[text]);
            edit::refresh_scene(&mut self.doc.scene); self.selection = vec![container.unwrap_or(text)]; self.rebuild(); cx.notify();
        } else if self.selected().is_some() { self.edit_selection_text(window, cx); }
    }

    pub(super) fn flowchart(&mut self, direction: World, create: bool, cx: &mut Context<Self>) {
        let Some(i) = self.selected() else { return };
        let e = &self.doc.scene.elements[i];
        if !matches!(e, Element::Rectangle { .. } | Element::Diamond { .. } | Element::Ellipse { .. } | Element::StickyNote(_)) { return; }
        if !create {
            let id = e.id(); let center = element_center(e);
            let next = self.doc.scene.non_deleted().filter_map(|arrow| {
                let Element::Arrow(a) = arrow else { return None };
                let start = a.linear.start_binding.as_ref()?.element_id.as_str(); let end = a.linear.end_binding.as_ref()?.element_id.as_str();
                let other = if start == id { end } else if end == id { start } else { return None };
                let n = self.doc.scene.elements.iter().position(|e| e.id() == other && !e.is_deleted())?;
                let delta = element_center(&self.doc.scene.elements[n]) - center;
                (delta.dot(direction) > 0.).then_some((n, delta.length()))
            }).min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((n, _)) = next { self.selection = vec![n]; cx.notify(); } return;
        }
        if self.doc.scene.elements.len() + 2 > MAX_ELEMENTS { return; }
        let from = element_bounds(e); let delta = World::new(direction.x * (from.width() + 100.), direction.y * (from.height() + 100.));
        let delta = edit::bounded_delta(from, delta);
        if delta.dot(direction) < 1. { return; }
        let mut copy = edit::clone_elements(std::slice::from_ref(e)).remove(0);
        let b = copy.base_mut(); b.group_ids.clear(); b.frame_id = None; b.bound_elements = None;
        let a = element_center(e); let source_id = e.id().to_string(); let new_id = copy.id().to_string();
        let mut draft = Scene::new(); draft.elements.push(copy);
        transform_selection(&mut draft, &[0], from, Rect::new(from.min_x + delta.x, from.min_y + delta.y, from.max_x + delta.x, from.max_y + delta.y), 0.);
        let b = element_center(&draft.elements[0]);
        let start = World::new(a.x + direction.x * from.width() / 2., a.y + direction.y * from.height() / 2.);
        let end = World::new(b.x - direction.x * from.width() / 2., b.y - direction.y * from.height() / 2.);
        let mut arrow = factory::new_arrow(vec![start, end], None, Some(Arrowhead::Arrow), &self.style);
        self.checkpoint(); let n = self.doc.scene.elements.len(); self.doc.scene.elements.extend(draft.elements);
        if let Element::Arrow(arrow) = &mut arrow {
            arrow.linear.start_binding = self.doc.scene.get(&source_id).map(|e| scene::binding::make_binding(e, start));
            arrow.linear.end_binding = self.doc.scene.get(&new_id).map(|e| scene::binding::make_binding(e, end));
        }
        self.doc.scene.elements.push(arrow); self.selection = vec![n];
        edit::refresh_scene(&mut self.doc.scene); self.sync_frames(); self.rebuild(); cx.notify();
    }

    pub(super) fn copy_png(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let doc = if self.selection.is_empty() { self.doc.clone() } else { self.selection_document() };
        self.busy = true; cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let png = cx.background_executor().spawn(async move { doc.png() }).await;
            let _ = this.update_in(cx, |d, _, cx| {
                d.busy = false;
                if let Ok(png) = png {
                    let image = Image::from_bytes(ImageFormat::Png, png);
                    let mut fake = crate::clipboard::FAKE_CLIPBOARD.lock().unwrap();
                    if let Some(clip) = fake.as_mut() { clip.text.clear(); clip.html = None; clip.images = vec![image]; }
                    else { cx.write_to_clipboard(ClipboardItem::new_image(&image)); }
                } else { d.error = Some(tr("无法复制图片", "Could not copy image")); }
                cx.notify();
            });
        }).detach();
    }

    pub(super) fn paste_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let fake = crate::clipboard::FAKE_CLIPBOARD.lock().unwrap().clone();
        let text = fake.map(|clip| clip.text).or_else(|| cx.read_from_clipboard().and_then(|item| item.text()));
        if let Some(text) = text && !text.is_empty() { self.start_text(self.pointer, None, text, window, cx); cx.notify(); }
    }

    pub(super) fn command_key(&mut self, key: &str, m: Modifiers, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if matches!(key, "left" | "right" | "up" | "down") && (m.alt || m.secondary() && !m.shift) && !self.view_only {
            let direction = match key { "left" => World::new(-1., 0.), "right" => World::new(1., 0.), "up" => World::new(0., -1.), _ => World::new(0., 1.) };
            self.flowchart(direction, m.secondary(), cx); return true;
        }
        if !m.secondary() && m.alt {
            match key {
                "z" => { self.zen = !self.zen; self.panel = None; }
                "r" => { self.view_only = !self.view_only; self.selection.clear(); self.panel = None; }
                "s" => self.objects_snap = !self.objects_snap,
                "d" if m.shift => self.dark_canvas = !self.dark_canvas,
                "w" => { self.toggle_text_wrap(window, cx); return true; }
                _ => return false,
            }
            cx.notify(); return true;
        }
        if self.view_only { return false; }
        if m.secondary() {
            match key {
                "delete" | "backspace" => { self.reset_confirmation = true; cx.notify(); }
                "," | "." | "<" | ">" if m.shift => {
                    let factor = if key == "," || key == "<" { 1. / 1.1 } else { 1.1 };
                    let value = self.selected().and_then(|i| if let Element::Text(t) = &self.doc.scene.elements[i] { Some(t.font_size) } else { self.bound_text_index(&self.doc.scene.elements[i]).and_then(|i| if let Element::Text(t) = &self.doc.scene.elements[i] { Some(t.font_size) } else { None }) }).unwrap_or(self.text_size);
                    self.apply_style(StyleChoice::TextSize((value * factor).round().clamp(1., 512.)), true, window, cx);
                }
                _ => return false,
            }
            return true;
        }
        match key {
            "tab" if !self.selection.is_empty() => self.convert_selection(m.shift, cx),
            "w" => self.toggle_text_binding(window, cx),
            "s" | "g" if !m.shift => { self.colors = Some(if key == "s" { chrome::Colors::Stroke } else { chrome::Colors::Fill }); self.panel = Some(Panel::Properties); cx.notify(); }
            "f" if m.shift => { self.more_fonts = true; self.panel = Some(Panel::Properties); cx.notify(); }
            "pageup" | "pagedown" => {
                let delta = if key == "pageup" { 1. } else { -1. };
                let view = self.viewport.get();
                self.offset += if m.shift { point(view.size.width * delta, px(0.)) } else { point(px(0.), view.size.height * delta) }; cx.notify();
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{normalized_color, snap_delta, World, Scene, factory, ElementOptions};
    #[test]
    fn custom_colors_are_normalized_and_invalid_values_rejected() {
        assert_eq!(normalized_color("Ab3").as_deref(), Some("#aabb33"));
        assert_eq!(normalized_color("#A1b2c3").as_deref(), Some("#a1b2c3"));
        for value in ["", "#abcd", "red", "#gg0000"] { assert!(normalized_color(value).is_none()); }
    }
    #[test]
    fn snapping_uses_edges_and_centers_and_excludes_attached_text() {
        let mut doc = Scene::new(); let options = ElementOptions::default();
        doc.elements.push(factory::new_rectangle(0., 0., 100., 80., &options));
        doc.elements.push(factory::new_rectangle(200., 100., 100., 80., &options));
        let (delta, guides) = snap_delta(&doc, &[0], World::new(98., 18.), 5.);
        assert_eq!(delta, World::new(100., 20.)); assert_eq!(guides.len(), 2);
        let (delta, guides) = snap_delta(&doc, &[0], World::new(90., 10.), 5.);
        assert_eq!(delta, World::new(90., 10.)); assert!(guides.is_empty());
    }
}

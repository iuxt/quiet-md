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
                            if matches!(property, Property::Width | Property::Height) { edit::resize_selection(&mut self.doc.scene, &self.selection, b, to, None); }
                            else { transform_selection(&mut self.doc.scene, &self.selection, b, to, angle); }
                            self.touch_selection(); self.sync_selection_frames(); self.rebuild();
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
        let color = self.doc.sample_color(p, self.dark_canvas, self.export_options.show_frames).unwrap_or_else(|| self.doc.background.clone());
        self.apply_color(self.eyedropper, &color, cx); self.tool = Tool::Select; cx.notify();
    }

    pub(super) fn fill_at(&mut self, p: World, cx: &mut Context<Self>) {
        use quiet_excalidraw::render::bucket_fill::{compute_bucket_fill, find_fill_at};
        let color = if self.style.background_color == "transparent" { "#b2f2bb".to_owned() } else { self.style.background_color.clone() }; // Excalidraw bucket default
        if let Some(i) = find_fill_at(&self.doc.scene.elements, p) {
            if self.doc.scene.elements[i].base().locked { return; }
            self.checkpoint();
            let e = &mut self.doc.scene.elements[i]; e.base_mut().background_color = color; e.base_mut().fill_style = self.style.fill_style;
            touch(e);
        } else {
            let Ok(region) = compute_bucket_fill(p, &self.doc.scene.elements) else { return };
            if region.points.len() > self.doc.remaining_points() || self.doc.scene.non_deleted().count() >= MAX_ELEMENTS { return; }
            let style = ElementOptions { background_color: color, stroke_color: "transparent".into(), roughness: 0., stroke_width: 1., fill_style: self.style.fill_style, opacity: self.style.opacity, ..Default::default() };
            let mut e = factory::new_line(region.points, &style);
            if let Element::Line(l) = &mut e { l.polygon = true; }
            let at = region.owner_id.as_ref().and_then(|id| self.doc.scene.elements.iter().position(|e| e.id() == id))
                .or_else(|| self.doc.scene.elements.iter().rposition(|e| region.boundary_ids.iter().any(|id| id == e.id())));
            let index = at.map_or(self.doc.scene.elements.len(), |i| i + 1);
            self.checkpoint(); self.doc.scene.elements.insert(index, e);
            self.selection.clear(); self.sync_frames(&[index]);
        }
        self.rebuild(); cx.notify();
    }

    pub(super) fn shared_conversion_targets(&self) -> &'static [scene::element::ConvertTarget] {
        let Some(&first) = self.selection.first() else { return &[] };
        let targets = self.doc.scene.elements[first].convertible_targets();
        if self.selection.iter().all(|&i| {
            let e = &self.doc.scene.elements[i]; !e.base().locked && e.convertible_targets() == targets
        }) { targets } else { &[] }
    }

    pub(super) fn convert_selection(&mut self, backwards: bool, cx: &mut Context<Self>) {
        let targets = self.shared_conversion_targets();
        let current = self.selection.first().and_then(|&i| self.doc.scene.elements[i].convert_target());
        if let Some(n) = targets.iter().position(|t| Some(*t) == current) {
            let n = (n + if backwards { targets.len() - 1 } else { 1 }) % targets.len();
            self.convert_selection_to(targets[n], cx);
        }
    }

    pub(super) fn convert_selection_to(&mut self, target: scene::element::ConvertTarget, cx: &mut Context<Self>) {
        if !self.editable() || !self.shared_conversion_targets().contains(&target) { return; }
        let changes: Vec<_> = self.selection.iter().filter_map(|&i| Some((i, self.doc.scene.elements[i].convert_to(target)?))).collect();
        if changes.is_empty() { return; }
        self.checkpoint();
        for (i, mut e) in changes {
            if let Element::Arrow(a) = &mut e && a.elbowed { edit::elbow_points(a); }
            touch(&mut e); self.doc.scene.elements[i] = e;
        }
        edit::refresh_scene(&mut self.doc.scene);
        self.line_edit = None; self.crop = None; self.selected_points.clear();
        self.panel = Some(Panel::Properties); self.rebuild(); cx.notify();
    }

    pub(super) fn toggle_text_wrap(&mut self, window: &Window, cx: &mut Context<Self>) {
        let indices: Vec<_> = self.selection.iter().filter_map(|i| matches!(&self.doc.scene.elements[*i], Element::Text(t) if t.container_id.is_none()).then_some(*i)).collect();
        if indices.is_empty() { return; }
        self.checkpoint();
        for i in indices {
            if let Element::Text(t) = &mut self.doc.scene.elements[i] {
                t.auto_resize = !t.auto_resize;
                t.text = text_to_edit(t).to_string();
                if !t.auto_resize { t.text = scene::text::wrap_text_in(&t.text, t.base.width, t.font_size, t.font_family); }
                resize_text(t, window);
            }
            touch(&mut self.doc.scene.elements[i]);
        }
        self.rebuild(); cx.notify();
    }

    pub(super) fn toggle_text_binding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editable() || self.selection.is_empty() { return; }
        let before = self.selection.clone();
        if let Some((after, prior)) = self.last_wrap.take() && before == after && let Some(state) = self.undo.back() {
            let scene = state.scene.clone();
            self.checkpoint(); self.doc.scene = scene; self.selection = prior;
            self.line_edit = None; self.rebuild(); cx.notify(); return;
        }
        let roots: Vec<_> = before.iter().copied().filter(|&i| {
            let e = &self.doc.scene.elements[i];
            !e.is_deleted() && !e.base().locked && !matches!(e, Element::Text(t) if t.container_id.is_some())
        }).collect();
        if roots.is_empty() { return; }
        let mut texts: Vec<_> = roots.iter().copied().filter(|&i| matches!(self.doc.scene.elements[i], Element::Text(_))).collect();
        texts.sort_unstable();
        let labels: Vec<_> = roots.iter().filter_map(|&i| self.bound_text_index(&self.doc.scene.elements[i]).map(|text| (i, text))).collect();
        let container = roots.iter().copied().find(|&i| can_contain_text(&self.doc.scene.elements[i]) && self.bound_text_index(&self.doc.scene.elements[i]).is_none());
        if roots.len() == 2 && texts.len() == 1 && let Some(container) = container {
            self.checkpoint(); self.bind_label(texts[0], container); self.selection = vec![container];
        } else if !texts.is_empty() && texts.len() + labels.len() == roots.len() {
            if self.doc.scene.non_deleted().count() + texts.len() > MAX_ELEMENTS { return; }
            self.checkpoint();
            let mut selected: Vec<String> = labels.iter().map(|&(i, _)| self.doc.scene.elements[i].id().into()).collect();
            for &i in texts.iter().rev() {
                let b = self.doc.scene.elements[i].base();
                let mut rect = factory::new_rectangle(b.x - 5., b.y - 5., b.width + 10., b.height + 10., &ElementOptions::default());
                let r = rect.base_mut(); r.stroke_color.clone_from(&b.stroke_color); r.angle = b.angle;
                r.frame_id.clone_from(&b.frame_id); r.group_ids.clone_from(&b.group_ids);
                r.roundness = Some(Roundness { kind: RoundnessType::AdaptiveRadius, value: None });
                let text_id = b.id.clone(); let rect_id = rect.id().to_string(); selected.push(rect_id.clone());
                for e in &mut self.doc.scene.elements {
                    if let Element::Arrow(a) = e {
                        for end in [&mut a.linear.start_binding, &mut a.linear.end_binding].into_iter().flatten() {
                            if end.element_id == text_id { end.element_id.clone_from(&rect_id); }
                        }
                    }
                }
                self.doc.scene.elements.insert(i, rect); self.bind_label(i + 1, i);
            }
            self.selection = self.doc.scene.elements.iter().enumerate().filter_map(|(i, e)| selected.iter().any(|id| id == e.id()).then_some(i)).collect();
        } else if labels.len() == roots.len() {
            self.checkpoint(); self.selection.clear();
            for (i, text) in labels {
                let id = self.doc.scene.elements[text].id().to_owned();
                if let Some(bound) = &mut self.doc.scene.elements[i].base_mut().bound_elements { bound.retain(|b| b.id != id); }
                if let Element::Text(t) = &mut self.doc.scene.elements[text] {
                    t.container_id = None; t.text = text_to_edit(t).to_owned(); resize_text(t, window);
                }
                touch(&mut self.doc.scene.elements[i]); touch(&mut self.doc.scene.elements[text]); self.selection.extend([i, text]);
            }
        } else {
            let frame_id = roots.iter().find_map(|&i| {
                let e = &self.doc.scene.elements[i];
                matches!(e, Element::Frame(_) | Element::MagicFrame(_)).then(|| e.id().to_owned())
            });
            let parent = self.doc.scene.elements[roots[0]].base().frame_id.clone();
            let frame = frame_id.filter(|id| roots.iter().all(|&i| {
                let e = &self.doc.scene.elements[i]; e.id() == id || e.base().frame_id.as_ref() == Some(id)
            })).or_else(|| parent.filter(|id| roots.iter().all(|&i| self.doc.scene.elements[i].base().frame_id.as_ref() == Some(id))
                && self.doc.scene.elements.iter().enumerate().all(|(i,e)| e.is_deleted() || e.base().frame_id.as_ref() != Some(id)
                    || matches!(e, Element::Text(t) if t.container_id.is_some()) || roots.contains(&i))));
            if let Some(id) = frame {
                let Some(i) = self.doc.scene.elements.iter().position(|e| e.id() == id) else { return };
                self.checkpoint();
                self.selection = self.doc.scene.elements.iter().enumerate().filter_map(|(i,e)|
                    (!e.is_deleted() && e.base().frame_id.as_ref() == Some(&id) && !matches!(e, Element::Text(t) if t.container_id.is_some())).then_some(i)).collect();
                self.erase_element(i);
            } else {
                if self.doc.scene.non_deleted().count() >= MAX_ELEMENTS || roots.iter().any(|&i| matches!(self.doc.scene.elements[i], Element::Frame(_) | Element::MagicFrame(_))) { return; }
                let mut children = selection_indices(&self.doc.scene, &roots);
                let Some(bounds) = selection_bounds(&self.doc.scene, &children) else { return };
                let covers = |b: Rect| bounds.contains(World::new(b.min_x, b.min_y)) && bounds.contains(World::new(b.max_x, b.max_y));
                for (i, e) in self.doc.scene.elements.iter().enumerate() {
                    if !e.is_deleted() && !matches!(e, Element::Frame(_) | Element::MagicFrame(_)) && e.base().frame_id.is_none() && covers(element_bounds(e)) && !children.contains(&i) { children.push(i); }
                }
                let pad = if roots.iter().any(|&i| matches!(self.doc.scene.elements[i], Element::Image(_)) && element_bounds(&self.doc.scene.elements[i]) == bounds) { 0. } else { 16. };
                let b = bounds.expand(pad); let frame = factory::new_frame(b.min_x, b.min_y, b.width(), b.height(), None, &ElementOptions::default());
                let i = *children.iter().min().unwrap(); self.checkpoint();
                for child in children { let e = &mut self.doc.scene.elements[child]; e.base_mut().frame_id = Some(frame.id().into()); touch(e); }
                self.doc.scene.elements.insert(i, frame); self.selection = vec![i];
            }
        }
        self.last_wrap = Some((self.selection.clone(), before)); self.line_edit = None;
        edit::refresh_scene(&mut self.doc.scene); self.rebuild(); cx.notify();
    }

    fn bind_label(&mut self, text: usize, container: usize) {
        let id = self.doc.scene.elements[text].id().to_owned();
        let e = &mut self.doc.scene.elements[container];
        e.base_mut().bound_elements.get_or_insert_with(Vec::new).push(scene::element::BoundElement { id, kind: scene::element::BoundElementType::Text });
        touch(e); let container_id = e.id().to_owned();
        if let Element::Text(t) = &mut self.doc.scene.elements[text] {
            t.container_id = Some(container_id); t.auto_resize = true; t.vertical_align = VerticalAlign::Middle;
        }
        touch(&mut self.doc.scene.elements[text]);
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
        if self.doc.scene.non_deleted().count() + 2 > MAX_ELEMENTS { return; }
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
            arrow.elbowed = true;
            arrow.linear.start_binding = self.doc.scene.get(&source_id).map(|e| scene::binding::make_binding(e, start));
            arrow.linear.end_binding = self.doc.scene.get(&new_id).map(|e| scene::binding::make_binding(e, end));
        }
        self.doc.scene.elements.push(arrow); self.selection = vec![n];
        edit::refresh_scene(&mut self.doc.scene); self.sync_frames(&[n,n+1]); self.rebuild(); cx.notify();
    }

    pub(super) fn copy_png(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let doc = if self.selection.is_empty() { self.doc.clone() } else { self.selection_export_document() };
        let options = self.export_options;
        self.busy = true; cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let png = cx.background_executor().spawn(async move { doc.export_png(options) }).await;
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

    pub(super) fn element_link_target(&self) -> Option<&str> {
        if let Some(i) = self.selected() { return Some(self.doc.scene.elements[i].id()); }
        let first = self.doc.scene.elements.get(*self.selection.first()?)?;
        first.base().group_ids.iter().rev().find(|g| self.selection.iter().all(|&i| self.doc.scene.elements[i].base().group_ids.contains(g))).map(String::as_str)
    }

    pub(super) fn follow_link(&mut self, cx: &mut Context<Self>) {
        let Some(link) = self.selected().and_then(|i| self.doc.scene.elements[i].base().link.as_deref()) else { return };
        let Ok(url) = url::Url::parse(link) else { return };
        if matches!(url.host_str(),Some("excalidraw.com"|"www.excalidraw.com"|"quiet.md")) && let Some((_,id)) = url.query_pairs().find(|(k,_)| k=="element") {
            let items: Vec<_> = self.doc.scene.elements.iter().enumerate().filter_map(|(i,e)| (!e.is_deleted() && (e.id()==id || e.base().group_ids.iter().any(|g| g==&*id))).then_some(i)).collect();
            if !items.is_empty() { self.selection = items; self.zoom_selection(cx); }
        } else if matches!(url.scheme(),"https"|"http"|"mailto") { crate::editor::link::open(cx,url.as_str()); }
    }

    pub(super) fn command_key(&mut self, key: &str, m: Modifiers, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if matches!(key, "left" | "right" | "up" | "down") && (m.alt || m.secondary() && !m.shift) && !self.view_only {
            let direction = match key { "left" => World::new(-1., 0.), "right" => World::new(1., 0.), "up" => World::new(0., -1.), _ => World::new(0., 1.) };
            self.flowchart(direction, m.secondary(), cx); return true;
        }
        if !m.secondary() && m.alt {
            match key {
                "/" => self.stats = !self.stats,
                "z" => { self.zen = !self.zen; self.panel = None; }
                "r" => { self.view_only = !self.view_only; self.selection.clear(); self.panel = None; }
                "s" => self.objects_snap = !self.objects_snap,
                "d" if m.shift => self.dark_canvas = !self.dark_canvas,
                "w" if !self.view_only => { self.toggle_text_wrap(window, cx); return true; }
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

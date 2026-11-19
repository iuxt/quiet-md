//! Selection operations share the same scene path for previews, history and export.
use super::*;
use std::collections::HashSet;
use scene::bounds::Bounds as Rect;

pub(super) fn elbow_points(a: &mut scene::element::ArrowElement) {
    let Some((&start, &end)) = a.linear.points.first().zip(a.linear.points.last()) else { return };
    let mid = (start.x + end.x) / 2.;
    a.linear.points = vec![start, World::new(mid, start.y), World::new(mid, end.y), end];
    a.linear.points.dedup();
    if a.linear.points.len() < 2 { a.linear.points.push(end); }
    let b = Rect::from_points(&a.linear.points);
    (a.linear.base.x, a.linear.base.y, a.linear.base.width, a.linear.base.height) = (b.min_x, b.min_y, b.width(), b.height());
}

pub(super) fn refresh_scene(doc: &mut Scene) {
    scene::binding::update_bindings(doc);
    for e in &mut doc.elements { if let Element::Arrow(a) = e && a.elbowed && a.fixed_segments.is_none() { elbow_points(a); } }
    scene::binding::sync_container_text(doc);
}

pub(super) fn crop_image(e: &mut Element, from: Rect, to: Rect, natural: World) {
    let pivot = element_center(e); let angle = e.base().angle;
    let Element::Image(image) = e else { return };
    let crop = image.crop.unwrap_or(scene::element::ImageCrop { x: 0., y: 0., width: natural.x, height: natural.y, natural_width: natural.x, natural_height: natural.y });
    let sx = crop.width / from.width(); let sy = crop.height / from.height();
    let center = to.center().rotate(angle, pivot);
    let x = if image.scale[0] < 0. { from.max_x - to.max_x } else { to.min_x - from.min_x };
    let y = if image.scale[1] < 0. { from.max_y - to.max_y } else { to.min_y - from.min_y };
    image.crop = Some(scene::element::ImageCrop { x: crop.x + x * sx, y: crop.y + y * sy, width: to.width() * sx, height: to.height() * sy, ..crop });
    let b = &mut image.base;
    (b.x, b.y, b.width, b.height) = (center.x - to.width() / 2., center.y - to.height() / 2., to.width(), to.height());
}

#[derive(Clone, Copy)]
pub(super) enum AlignSelection { Left, Center, Right, Top, Middle, Bottom, Horizontal, Vertical }

pub(super) fn selection_indices(doc: &Scene, roots: &[usize]) -> Vec<usize> {
    let mut indices: Vec<_> = roots.iter().copied().filter(|i| doc.elements.get(*i).is_some_and(|e| !e.is_deleted())).collect();
    loop {
        let ids: HashSet<_> = indices.iter().map(|i| doc.elements[*i].id()).collect();
        let extra: Vec<_> = doc.elements.iter().enumerate().filter_map(|(i, e)| {
            let attached = matches!(e, Element::Text(t) if t.container_id.as_deref().is_some_and(|id| ids.contains(id)))
                || e.base().frame_id.as_deref().is_some_and(|id| ids.contains(id));
            (!e.is_deleted() && !indices.contains(&i) && attached).then_some(i)
        }).collect();
        if extra.is_empty() { break; }
        indices.extend(extra);
    }
    indices.sort_unstable(); indices.dedup(); indices
}

pub(super) fn selection_bounds(doc: &Scene, indices: &[usize]) -> Option<Rect> {
    indices.iter().filter_map(|i| doc.elements.get(*i)).filter(|e| !e.is_deleted()).map(element_bounds).reduce(|a, b| a.union(&b))
}

fn set_points(e: &mut Element, map: impl Fn(World) -> World) -> bool {
    let (angle, pivot) = (e.base().angle, element_center(e));
    let points = match e {
        Element::Arrow(a) => {
            if let Some(segments) = &mut a.fixed_segments {
                for s in segments { s.start = map(s.start.rotate(angle, pivot)); s.end = map(s.end.rotate(angle, pivot)); }
            }
            &mut a.linear.points
        }
        Element::Line(l) => &mut l.linear.points,
        Element::Freedraw(f) => &mut f.points,
        _ => return false,
    };
    for p in points.iter_mut() { *p = map(p.rotate(angle, pivot)); }
    let rect = Rect::from_points(points);
    let b = e.base_mut();
    (b.x, b.y, b.width, b.height, b.angle) = (rect.min_x, rect.min_y, rect.width(), rect.height(), 0.);
    true
}

pub(super) fn resize_selection(doc: &mut Scene, roots: &[usize], from: Rect, to: Rect) {
    if let [i] = roots && matches!(doc.elements[*i], Element::Frame(_) | Element::MagicFrame(_)) {
        let b = doc.elements[*i].base_mut();
        (b.x, b.y, b.width, b.height) = (to.min_x, to.min_y, to.width(), to.height());
    } else { transform_selection(doc, roots, from, to, 0.); }
}

pub(super) fn transform_selection(doc: &mut Scene, roots: &[usize], from: Rect, to: Rect, angle: f64) {
    let indices = selection_indices(doc, roots);
    let ids: HashSet<_> = indices.iter().map(|i| doc.elements[*i].id().to_string()).collect();
    let sx = if from.width() > 0.000001 { to.width() / from.width() } else { 1. };
    let sy = if from.height() > 0.000001 { to.height() / from.height() } else { 1. };
    let map = |p: World| World::new(to.min_x + (p.x - from.min_x) * sx, to.min_y + (p.y - from.min_y) * sy).rotate(angle, to.center());
    for i in indices {
        let e = &mut doc.elements[i];
        if !set_points(e, map) {
            let center = map(element_center(e));
            let b = e.base_mut();
            b.width *= sx; b.height *= sy;
            b.x = center.x - b.width / 2.; b.y = center.y - b.height / 2.;
            b.angle = (b.angle + angle).rem_euclid(std::f64::consts::TAU);
            if let Element::Text(t) = e { t.font_size = (t.font_size * sx.min(sy)).clamp(1., 512.); }
            if let Element::StickyNote(t) = e { t.base_height *= sy; }
        }
        let linear = match e { Element::Arrow(a) => Some(&mut a.linear), Element::Line(l) => Some(&mut l.linear), _ => None };
        if let Some(l) = linear {
            if l.start_binding.as_ref().is_some_and(|b| !ids.contains(&b.element_id)) { l.start_binding = None; }
            if l.end_binding.as_ref().is_some_and(|b| !ids.contains(&b.element_id)) { l.end_binding = None; }
        }
    }
    refresh_scene(doc);
}

pub(super) fn clone_elements(elements: &[Element]) -> Vec<Element> {
    let ids: HashMap<_, _> = elements.iter().map(|e| (e.id(), factory::new_id())).collect();
    let mut groups = HashMap::new();
    elements.iter().cloned().map(|mut e| {
        let b = e.base_mut();
        b.id = ids[b.id.as_str()].clone(); b.index = None; b.locked = false;
        b.group_ids = b.group_ids.iter().map(|id| groups.entry(id.clone()).or_insert_with(factory::new_id).clone()).collect();
        b.frame_id = b.frame_id.as_deref().and_then(|id| ids.get(id)).cloned();
        if let Some(bound) = &mut b.bound_elements {
            bound.retain_mut(|item| { if let Some(id) = ids.get(item.id.as_str()) { item.id = id.clone(); true } else { false } });
        }
        match &mut e {
            Element::Text(t) => t.container_id = t.container_id.as_deref().and_then(|id| ids.get(id)).cloned(),
            Element::Arrow(a) => remap_bindings(&mut a.linear, &ids),
            Element::Line(l) => remap_bindings(&mut l.linear, &ids),
            _ => {}
        }
        touch(&mut e); e
    }).collect()
}

fn remap_bindings(l: &mut scene::element::LinearElement, ids: &HashMap<&str, String>) {
    for binding in [&mut l.start_binding, &mut l.end_binding] {
        if let Some(b) = binding { if let Some(id) = ids.get(b.element_id.as_str()) { b.element_id.clone_from(id); } else { *binding = None; } }
    }
}

pub(super) fn move_vertices(doc: &mut Scene, index: usize, indices: &[usize], insert: Option<(usize, World)>, _start: World, delta: World, bind: bool) {
    let e = &mut doc.elements[index];
    set_points(e, |p| p);
    let elbow = matches!(e, Element::Arrow(a) if a.elbowed);
    let l = match e { Element::Arrow(a) => &mut a.linear, Element::Line(l) => &mut l.linear, _ => return };
    let old_points = l.points.clone();
    let mut delta = delta;
    if elbow && let [a, b] = indices { if (l.points[*a].x - l.points[*b].x).abs() < 0.001 { delta.y = 0.; } else { delta.x = 0.; } }
    if let Some((i, p)) = insert { l.points.insert(i, p); }
    for &i in indices { if let Some(p) = l.points.get_mut(i) { *p += delta; p.x = p.x.clamp(-MAX_COORD, MAX_COORD); p.y = p.y.clamp(-MAX_COORD, MAX_COORD); } }
    if elbow && l.points.len() > 2 {
        for i in 1..l.points.len() {
            if old_points[i].x == old_points[i - 1].x {
                if indices.contains(&i) { l.points[i - 1].x = l.points[i].x; } else { l.points[i].x = l.points[i - 1].x; }
            } else if indices.contains(&i) { l.points[i - 1].y = l.points[i].y; } else { l.points[i].y = l.points[i - 1].y; }
        }
    }
    let start = l.points[0]; let end = *l.points.last().unwrap();
    let moved_start = indices.contains(&0); let moved_end = indices.contains(&(l.points.len() - 1));
    let rect = Rect::from_points(&l.points);
    (l.base.x, l.base.y, l.base.width, l.base.height) = (rect.min_x, rect.min_y, rect.width(), rect.height());
    let start_binding = (bind && moved_start).then(|| arrow_binding(doc, start)).flatten();
    let end_binding = (bind && moved_end).then(|| arrow_binding(doc, end)).flatten();
    let l = match &mut doc.elements[index] { Element::Arrow(a) => &mut a.linear, Element::Line(l) => &mut l.linear, _ => return };
    if moved_start { l.start_binding = start_binding; }
    if moved_end { l.end_binding = end_binding; }
    if let Element::Arrow(a) = &mut doc.elements[index] && a.elbowed {
        a.fixed_segments = Some(a.linear.points.windows(2).enumerate().filter(|(n, _)| *n > 0 && *n + 2 < a.linear.points.len())
            .map(|(n, pair)| scene::element::FixedSegment { start: pair[0], end: pair[1], index: n + 1 }).collect());
    }
    refresh_scene(doc);
}

impl DrawingEditor {
    pub(super) fn library_thumbnail(&self, index: usize) -> Option<Arc<RenderImage>> {
        let mut cache = self.library_images.borrow_mut();
        if let Some(image) = cache.get(&index) { return Some(image.clone()); }
        let raster = quiet_excalidraw::render::export::rasterize_pixels(&self.library.get(index)?.svg(), 0.5, 128).ok()?;
        let (w, h) = (raster.width(), raster.height()); let mut bytes = raster.take();
        for p in bytes.as_chunks_mut::<4>().0 {
            let a = u32::from(p[3]);
            if a > 0 && a < 255 { for c in &mut p[..3] { *c = ((u32::from(*c) * 255 + a / 2) / a) as u8; } }
            p.swap(0, 2);
        }
        let image = Arc::new(RenderImage::new(vec![image::Frame::new(image::RgbaImage::from_raw(w, h, bytes)?)]));
        cache.insert(index, image.clone()); Some(image)
    }

    pub(super) fn image_dimensions(&self, index: usize) -> World {
        if let Element::Image(image) = &self.doc.scene.elements[index] && let Some(data) = image.file_id.as_deref().and_then(|id| self.images.get(id)) {
            let size = data.size(0); return World::new(size.width.0 as f64, size.height.0 as f64);
        }
        World::new(1., 1.)
    }

    pub(super) fn reset_crop(&mut self, cx: &mut Context<Self>) {
        let Some(i) = self.selected() else { return };
        let Element::Image(image) = &self.doc.scene.elements[i] else { return };
        let Some(crop) = image.crop else { return };
        self.checkpoint();
        let Element::Image(image) = &mut self.doc.scene.elements[i] else { return };
        let b = &mut image.base; let center = World::new(b.x + b.width / 2., b.y + b.height / 2.);
        let sx = b.width / crop.width; let sy = b.height / crop.height;
        let x = if image.scale[0] < 0. { crop.natural_width - crop.x - crop.width } else { crop.x };
        let y = if image.scale[1] < 0. { crop.natural_height - crop.y - crop.height } else { crop.y };
        let w = crop.natural_width * sx; let h = crop.natural_height * sy;
        let c = World::new(b.x - x * sx + w / 2., b.y - y * sy + h / 2.).rotate(b.angle, center);
        (b.x, b.y, b.width, b.height) = (c.x - w / 2., c.y - h / 2., w, h);
        image.crop = None; touch(&mut self.doc.scene.elements[i]); self.crop = None; self.rebuild(); cx.notify();
    }

    pub(super) fn add_to_library(&mut self, cx: &mut Context<Self>) {
        if !self.editable() || self.selection.is_empty() || self.library.len() >= 64 { return; }
        let doc = self.selection_document();
        self.library.push(doc);
        if DrawingScene::library_json(&self.library).is_err() { self.library.pop(); return; }
        self.persist_library(); self.panel = Some(Panel::Library); cx.notify();
    }

    pub(super) fn persist_library(&mut self) {
        let result = (|| -> anyhow::Result<()> {
            let path = Store::internal_dir().join("drawing-library.excalidrawlib");
            anyhow::ensure!(!path.is_symlink(), "drawing library is a symlink");
            let bytes = DrawingScene::library_json(&self.library)?;
            let temp = path.with_extension(format!("{}.tmp", factory::new_id()));
            use std::io::Write;
            let result = (|| -> anyhow::Result<()> {
                let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
                file.write_all(&bytes)?; file.sync_all()?; std::fs::rename(&temp, &path)?; Ok(())
            })();
            if result.is_err() { let _ = std::fs::remove_file(temp); } result
        })();
        if let Err(e) = result { log::warn!("save drawing library: {e:#}"); self.error = Some(tr("无法保存素材库", "Could not save drawing library")); }
    }

    pub(super) fn import_library(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editable() { return; }
        let picked = crate::devtools::take_pick();
        let rx = picked.is_none().then(|| cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false,
            prompt: Some(tr("导入素材库", "Import library").into()), extensions: Some(vec!["excalidrawlib".into()]) }));
        self.busy = true; cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let paths = match rx { Some(rx) => rx.await.ok().and_then(Result::ok).flatten(), None => picked };
            let result = if let Some(path) = paths.and_then(|p| p.into_iter().next()) {
                Some(cx.background_executor().spawn(async move { DrawingScene::load_library(&store::read_bounded(&path, MAX_SOURCE)?) }).await)
            } else { None };
            let _ = this.update_in(cx, |d, _, cx| {
                d.busy = false;
                if let Some(result) = result {
                    match result {
                        Ok(items) if items.len() + d.library.len() <= 64 => {
                            let mut all = d.library.clone(); all.extend(items);
                            if DrawingScene::library_json(&all).is_ok() { d.library = all; d.persist_library(); }
                        }
                        _ => d.error = Some(tr("无法导入此素材库", "Could not import this library")),
                    }
                }
                cx.notify();
            });
        }).detach();
    }

    pub(super) fn export_library(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(bytes) = DrawingScene::library_json(&self.library) else { return };
        let path = crate::devtools::take_pick().and_then(|p| p.into_iter().next());
        let dir = dirs::download_dir().unwrap_or_default();
        let rx = path.is_none().then(|| cx.prompt_for_new_path(&dir, Some("Drawing.excalidrawlib")));
        cx.spawn_in(window, async move |this, cx| {
            let path = match rx { Some(rx) => rx.await.ok().and_then(Result::ok).flatten(), None => path };
            if let Some(path) = path {
                let result = cx.background_executor().spawn(async move { std::fs::write(path, bytes) }).await;
                if let Err(e) = result { log::warn!("export drawing library: {e:#}"); let _ = this.update_in(cx, |d, _, cx| { d.error = Some(tr("无法导出素材库", "Could not export drawing library")); cx.notify(); }); }
            }
        }).detach();
    }

    pub(super) fn rasterized_elements(&self, doc: &Scene, window: &mut Window) -> HashMap<usize, (Arc<RenderImage>, Rect)> {
        use std::hash::{Hash, Hasher};
        let mut out = HashMap::new();
        let mut cache = self.raster_artwork.borrow_mut();
        for (i, e) in doc.elements.iter().enumerate() {
            let needed = match e {
                Element::Text(t) => t.base.angle.abs() > 0.001,
                Element::Image(image) => image.base.angle.abs() > 0.001 || image.crop.is_some() || image.scale != [1., 1.],
                _ => false,
            };
            if !needed || e.is_deleted() { continue; }
            let b = element_bounds(e).expand(10.);
            let view = self.viewport.get();
            let screen = |p: World| view.origin + self.offset + point(px((p.x * self.zoom) as f32), px((p.y * self.zoom) as f32));
            if !view.intersects(&Bounds::from_corners(screen(World::new(b.min_x, b.min_y)), screen(World::new(b.max_x, b.max_y)))) { continue; }
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            serde_json::to_vec(e).unwrap_or_default().hash(&mut hash); self.dark_canvas.hash(&mut hash);
            let hash = hash.finish();
            if cache.get(e.id()).is_none_or(|(old, _)| *old != hash) {
                let mut scene = Scene::new(); scene.elements.push(e.clone());
                let svg = quiet_excalidraw::render::export::SvgExporter::export_with_options(&scene, "transparent", &self.doc.files, self.dark_canvas);
                let Ok(raster) = quiet_excalidraw::render::export::rasterize_pixels(&svg, 2., 2048) else { continue };
                let (w, h) = (raster.width(), raster.height()); let mut bytes = raster.take();
                for p in bytes.as_chunks_mut::<4>().0 {
                    let a = u32::from(p[3]);
                    if a > 0 && a < 255 { for c in &mut p[..3] { *c = ((u32::from(*c) * 255 + a / 2) / a) as u8; } }
                    p.swap(0, 2);
                }
                let image = Arc::new(RenderImage::new(vec![image::Frame::new(image::RgbaImage::from_raw(w, h, bytes).expect("raster dimensions"))]));
                if let Some((_, old)) = cache.insert(e.id().to_string(), (hash, image)) { let _ = window.drop_image(old); }
            }
            if let Some((_, image)) = cache.get(e.id()) { out.insert(i, (image.clone(), b)); }
        }
        if cache.len() > 64 {
            let visible: HashSet<_> = out.keys().map(|i| doc.elements[*i].id()).collect();
            cache.retain(|id, (_, image)| {
                if visible.contains(id.as_str()) { true } else { let _ = window.drop_image(image.clone()); false }
            });
        }
        out
    }

    pub(super) fn editable(&self) -> bool { !self.busy && !self.discard && !self.reset_confirmation && !self.view_only && self.gesture.is_none() }

    pub(super) fn select_all(&mut self, cx: &mut Context<Self>) {
        if !self.editable() { return; }
        if let Some(i) = self.line_edit {
            self.selected_points = (0..linear_points(&self.doc.scene.elements[i]).map_or(0, <[World]>::len)).collect();
        } else {
            self.crop = None;
            self.selection = self.doc.scene.elements.iter().enumerate().filter_map(|(i, e)|
                (!e.is_deleted() && !e.base().locked && !matches!(e, Element::Text(t) if t.container_id.is_some())).then_some(i)).collect();
        }
        self.panel = Some(Panel::Properties); cx.notify();
    }

    pub(super) fn edit_points(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.text_at.is_some() || self.edit_link.is_some() || self.edit_frame.is_some() || self.edit_property.is_some() { self.finish_field(window, cx); return; }
        if !self.editable() { return; }
        if let Some(i) = self.selected() && matches!(self.doc.scene.elements[i], Element::Image(_)) {
            self.crop = if self.crop == Some(i) { None } else { Some(i) }; cx.notify(); return;
        }
        if let Some(i) = self.selected() && linear_points(&self.doc.scene.elements[i]).is_some() {
            self.line_edit = if self.line_edit == Some(i) { None } else { Some(i) }; self.selected_points.clear(); cx.notify();
        }
    }

    pub(super) fn delete_points(&mut self, cx: &mut Context<Self>) {
        let Some(i) = self.line_edit else { return };
        let Some(points) = linear_points(&self.doc.scene.elements[i]) else { return };
        if self.selected_points.is_empty() { return; }
        if points.len().saturating_sub(self.selected_points.len()) < 2 { self.delete(cx); return; }
        self.checkpoint();
        let e = &mut self.doc.scene.elements[i];
        set_points(e, |p| p);
        let l = match e { Element::Arrow(a) => &mut a.linear, Element::Line(l) => &mut l.linear, _ => return };
        let old_len = l.points.len();
        let mut n = 0; l.points.retain(|_| { let keep = !self.selected_points.contains(&n); n += 1; keep });
        if self.selected_points.contains(&0) { l.start_binding = None; }
        if self.selected_points.contains(&(old_len - 1)) { l.end_binding = None; }
        set_points(e, |p| p); touch(e);
        refresh_scene(&mut self.doc.scene); self.selected_points.clear(); self.rebuild(); cx.notify();
    }

    pub(super) fn zoom_selection(&mut self, cx: &mut Context<Self>) {
        let Some(b) = selection_bounds(&self.doc.scene, &self.selection) else { return };
        let view = self.viewport.get(); let w = f64::from(f32::from(view.size.width)); let h = f64::from(f32::from(view.size.height));
        self.zoom = ((w - 96.) / b.width().max(1.)).min((h - 96.) / b.height().max(1.)).clamp(0.05, 8.);
        self.offset = point(px((w / 2. - b.center().x * self.zoom) as f32), px((h / 2. - b.center().y * self.zoom) as f32)); cx.notify();
    }

    pub(super) fn copy_style(&mut self, cx: &mut Context<Self>) {
        if let Some(i) = self.selection.first() { self.copied_style = Some(self.doc.scene.elements[*i].clone()); cx.notify(); }
    }

    pub(super) fn paste_style(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(style) = self.copied_style.clone() else { return };
        if self.selection.is_empty() || !self.editable() { return; }
        let b = style.base(); self.checkpoint();
        for i in selection_indices(&self.doc.scene, &self.selection) {
            let e = &mut self.doc.scene.elements[i]; let base = e.base_mut();
            base.stroke_color.clone_from(&b.stroke_color); base.background_color.clone_from(&b.background_color);
            base.stroke_width = b.stroke_width; base.stroke_style = b.stroke_style; base.fill_style = b.fill_style;
            base.roughness = b.roughness; base.opacity = b.opacity; base.roundness.clone_from(&b.roundness);
            if let (Element::Text(t), Element::Text(from)) = (&mut *e, &style) {
                t.font_family = from.font_family; t.font_size = from.font_size; t.text_align = from.text_align; t.vertical_align = from.vertical_align; resize_text(t, window);
            }
            if let (Element::Arrow(a), Element::Arrow(from)) = (&mut *e, &style) {
                a.linear.start_arrowhead = from.linear.start_arrowhead; a.linear.end_arrowhead = from.linear.end_arrowhead;
            }
            touch(e);
        }
        refresh_scene(&mut self.doc.scene); self.rebuild(); cx.notify();
    }

    pub(super) fn start_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.line_edit = None; self.crop = None; self.selected_points.clear();
        if !self.editable() { return; }
        self.finish_field(window, cx); self.searching = true;
        self.field.update(cx, |f, cx| { f.multiline = false; f.set_placeholder(tr("查找画板文字", "Find text on canvas"), cx); f.set_text("", cx); });
        window.focus(&self.field.read(cx).focus_handle(cx)); cx.notify();
    }

    pub(super) fn find_text(&mut self, previous: bool, cx: &mut Context<Self>) {
        let query = self.field.read(cx).text.to_lowercase();
        if query.is_empty() { return; }
        let matches: Vec<_> = self.doc.scene.elements.iter().enumerate().filter_map(|(i, e)|
            matches!(e, Element::Text(t) if !e.is_deleted() && t.text.to_lowercase().contains(&query)).then_some(i)).collect();
        if matches.is_empty() { self.selection.clear(); cx.notify(); return; }
        let selected = self.selection.first().copied();
        let n = matches.iter().position(|i| Some(*i) == selected).map_or(0, |n| if previous { (n + matches.len() - 1) % matches.len() } else { (n + 1) % matches.len() });
        self.selection = vec![matches[n]];
        let b = element_bounds(&self.doc.scene.elements[matches[n]]); let view = self.viewport.get();
        self.offset = point(view.size.width / 2. - px((b.center().x * self.zoom) as f32), view.size.height / 2. - px((b.center().y * self.zoom) as f32)); cx.notify();
    }

    pub(super) fn preview_scene(&self) -> Option<Scene> {
        if !matches!(self.gesture, Some(Gesture::Crop { .. } | Gesture::Endpoint { .. } | Gesture::Vertex { .. } | Gesture::Move { .. } | Gesture::Resize { .. } | Gesture::Rotate { .. })) { return None; }
        let mut doc = self.doc.scene.clone();
        match &self.gesture {
            Some(Gesture::Crop { start, rect, index, .. }) => crop_image(&mut doc.elements[*index], *start, *rect, self.image_dimensions(*index)),
            Some(Gesture::Endpoint { index, endpoint, start, end, .. }) if start.distance(*end) > 0.5 => move_endpoint_binding(&mut doc, *index, *endpoint, *end, !self.binding_disabled),
            Some(Gesture::Vertex { index, start, delta, indices, insert }) => move_vertices(&mut doc, *index, indices, *insert, *start, *delta, !self.binding_disabled),
            Some(Gesture::Move { delta, .. }) => {
                let b = selection_bounds(&doc, &self.selection)?;
                transform_selection(&mut doc, &self.selection, b, Rect::new(b.min_x + delta.x, b.min_y + delta.y, b.max_x + delta.x, b.max_y + delta.y), 0.);
            }
            Some(Gesture::Resize { start, rect, .. }) => resize_selection(&mut doc, &self.selection, *start, *rect),
            Some(Gesture::Rotate { angle, .. }) => { let b = selection_bounds(&doc, &self.selection)?; transform_selection(&mut doc, &self.selection, b, b, *angle); }
            _ => return None,
        }
        Some(doc)
    }

    pub(super) fn group_selection(&mut self, ungroup: bool, cx: &mut Context<Self>) {
        if self.busy || self.selection.is_empty() || !ungroup && self.selection.len() < 2 { return; }
        let indices = selection_indices(&self.doc.scene, &self.selection);
        let groups: HashSet<_> = self.selection.iter().filter_map(|i| self.doc.scene.elements[*i].base().group_ids.last().cloned()).collect();
        if ungroup && groups.is_empty() { return; }
        self.checkpoint();
        let id = factory::new_id();
        for i in indices {
            let e = &mut self.doc.scene.elements[i];
            if ungroup { e.base_mut().group_ids.retain(|id| !groups.contains(id)); } else { e.base_mut().group_ids.push(id.clone()); }
            touch(e);
        }
        self.editing_group = None; cx.notify();
    }

    pub(super) fn expand_selection(&self, indices: &[usize]) -> Vec<usize> {
        let mut roots = Vec::new();
        for &i in indices {
            let e = &self.doc.scene.elements[i];
            let i = if let Element::Text(t) = e {
                t.container_id.as_deref().and_then(|id| self.doc.scene.elements.iter().position(|e| e.id() == id && !e.is_deleted())).unwrap_or(i)
            } else { i };
            let groups = &self.doc.scene.elements[i].base().group_ids;
            let group = if let Some(editing) = &self.editing_group {
                groups.iter().position(|id| id == editing).and_then(|n| n.checked_sub(1)).map(|n| &groups[n])
            } else { groups.last() };
            if let Some(id) = group {
                roots.extend(self.doc.scene.elements.iter().enumerate().filter_map(|(i, e)|
                    (!e.is_deleted() && !e.base().locked && e.base().group_ids.contains(id) && !matches!(e, Element::Text(t) if t.container_id.is_some())).then_some(i)));
            } else if !self.doc.scene.elements[i].base().locked { roots.push(i); }
        }
        roots.sort_unstable(); roots.dedup(); roots
    }

    pub(super) fn move_selection(&mut self, delta: World, cx: &mut Context<Self>) {
        if self.busy || self.gesture.is_some() { return; }
        let Some(b) = selection_bounds(&self.doc.scene, &self.selection) else { return };
        let delta = bounded_delta(b, delta);
        if delta.length() < 0.001 { return; }
        self.checkpoint();
        let to = Rect::new(b.min_x + delta.x, b.min_y + delta.y, b.max_x + delta.x, b.max_y + delta.y);
        transform_selection(&mut self.doc.scene, &self.selection, b, to, 0.);
        self.touch_selection(); self.sync_frames(); self.rebuild(); cx.notify();
    }

    pub(super) fn touch_selection(&mut self) {
        for i in selection_indices(&self.doc.scene, &self.selection) { touch(&mut self.doc.scene.elements[i]); }
    }

    pub(super) fn align_selection(&mut self, direction: AlignSelection, cx: &mut Context<Self>) {
        if self.busy || self.selection.len() < 2 { return; }
        let units = self.selection_units();
        let Some(all) = selection_bounds(&self.doc.scene, &self.selection) else { return };
        let horizontal = matches!(direction, AlignSelection::Horizontal);
        let distribute = matches!(direction, AlignSelection::Horizontal | AlignSelection::Vertical);
        if distribute && units.len() < 3 { return; }
        let mut units: Vec<_> = units.into_iter().filter_map(|unit| selection_bounds(&self.doc.scene, &unit).map(|b| (unit, b))).collect();
        units.sort_by(|(_, a), (_, b)| if horizontal { a.min_x.total_cmp(&b.min_x) } else { a.min_y.total_cmp(&b.min_y) });
        let gap = if distribute {
            let span = if horizontal { all.width() } else { all.height() };
            (span - units.iter().map(|(_, b)| if horizontal { b.width() } else { b.height() }).sum::<f64>()) / (units.len() - 1) as f64
        } else { 0. };
        let mut at = if horizontal { all.min_x } else { all.min_y };
        let mut next = self.doc.scene.clone();
        for (unit, b) in units {
            let delta = match direction {
                AlignSelection::Left => World::new(all.min_x - b.min_x, 0.), AlignSelection::Center => World::new(all.center().x - b.center().x, 0.),
                AlignSelection::Right => World::new(all.max_x - b.max_x, 0.), AlignSelection::Top => World::new(0., all.min_y - b.min_y),
                AlignSelection::Middle => World::new(0., all.center().y - b.center().y), AlignSelection::Bottom => World::new(0., all.max_y - b.max_y),
                AlignSelection::Horizontal => { let delta = World::new(at - b.min_x, 0.); at += b.width() + gap; delta }
                AlignSelection::Vertical => { let delta = World::new(0., at - b.min_y); at += b.height() + gap; delta }
            };
            for i in selection_indices(&next, &unit) { scene::transform::translate_element(&mut next.elements[i], delta.x, delta.y); }
        }
        refresh_scene(&mut next);
        if next.elements == self.doc.scene.elements { return; }
        self.checkpoint(); self.doc.scene = next; self.touch_selection(); self.rebuild(); cx.notify();
    }

    fn selection_units(&self) -> Vec<Vec<usize>> {
        let mut units: Vec<Vec<usize>> = Vec::new();
        for &i in &self.selection {
            let g = self.doc.scene.elements[i].base().group_ids.last();
            if let Some(g) = g && let Some(unit) = units.iter_mut().find(|u| self.doc.scene.elements[u[0]].base().group_ids.last() == Some(g)) { unit.push(i); }
            else { units.push(vec![i]); }
        }
        units
    }

    pub(super) fn lock_selection(&mut self, cx: &mut Context<Self>) {
        if self.busy { return; }
        self.line_edit = None; self.crop = None; self.selected_points.clear();
        let indices = if self.selection.is_empty() {
            self.doc.scene.elements.iter().enumerate().filter_map(|(i, e)| (!e.is_deleted() && e.base().locked).then_some(i)).collect()
        } else { selection_indices(&self.doc.scene, &self.selection) };
        if indices.is_empty() { return; }
        let locked = !indices.iter().all(|i| self.doc.scene.elements[*i].base().locked);
        self.checkpoint();
        for i in indices { let e = &mut self.doc.scene.elements[i]; e.base_mut().locked = locked; touch(e); }
        self.selection.clear(); cx.notify();
    }

    pub(super) fn flip_selection(&mut self, horizontal: bool, cx: &mut Context<Self>) {
        if self.busy { return; }
        let Some(b) = selection_bounds(&self.doc.scene, &self.selection) else { return };
        self.checkpoint();
        for i in selection_indices(&self.doc.scene, &self.selection) {
            let e = &mut self.doc.scene.elements[i];
            let map = |p: World| if horizontal { World::new(b.min_x + b.max_x - p.x, p.y) } else { World::new(p.x, b.min_y + b.max_y - p.y) };
            if !set_points(e, map) {
                let c = map(element_center(e)); let base = e.base_mut();
                base.x = c.x - base.width / 2.; base.y = c.y - base.height / 2.;
                base.angle = (-base.angle).rem_euclid(std::f64::consts::TAU);
                if let Element::Image(image) = e { image.scale[if horizontal { 0 } else { 1 }] *= -1.; }
            }
            touch(e);
        }
        refresh_scene(&mut self.doc.scene); self.rebuild(); cx.notify();
    }

    pub(super) fn sync_frames(&mut self) {
        let frames: Vec<_> = self.doc.scene.non_deleted().filter(|e| matches!(e, Element::Frame(_) | Element::MagicFrame(_))).map(|e| (e.id().to_string(), element_bounds(e))).collect();
        for e in &mut self.doc.scene.elements {
            if e.is_deleted() || matches!(e, Element::Frame(_) | Element::MagicFrame(_)) { continue; }
            let b = element_bounds(e);
            e.base_mut().frame_id = frames.iter().rev().find(|(id, frame)| frame.contains(b.center()) || e.base().frame_id.as_deref() == Some(id.as_str()) && frame.intersects(&b)).map(|(id, _)| id.clone());
        }
    }

    pub(super) fn selection_document(&self) -> DrawingScene {
        let mut doc = self.doc.clone();
        let indices = selection_indices(&doc.scene, &self.selection);
        doc.scene.elements = indices.into_iter().map(|i| doc.scene.elements[i].clone()).collect();
        doc.retain_used_files(); doc
    }

    pub(super) fn copy_selection(&mut self, cut: bool, cx: &mut Context<Self>) {
        if self.busy || self.selection.is_empty() { return; }
        let Ok(bytes) = self.selection_document().json() else { return };
        let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(&bytes) else { return };
        value["type"] = "excalidraw/clipboard".into();
        let text = value.to_string();
        let mut fake = crate::clipboard::FAKE_CLIPBOARD.lock().unwrap();
        if let Some(clip) = fake.as_mut() { clip.text = text; clip.html = None; clip.images.clear(); }
        else { cx.write_to_clipboard(ClipboardItem::new_string(text)); }
        drop(fake);
        if cut { self.delete(cx); }
    }

    pub(super) fn paste_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.gesture.is_some() { return; }
        let fake = crate::clipboard::FAKE_CLIPBOARD.lock().unwrap().clone();
        let item = if fake.is_none() { cx.read_from_clipboard() } else { None };
        let images: Vec<_> = if let Some(fake) = &fake { fake.images.clone() } else { item.as_ref().map_or_else(Vec::new, |item| item.entries().iter().filter_map(|e| if let ClipboardEntry::Image(image) = e { Some(image.clone()) } else { None }).collect()) };
        if !images.is_empty() {
            use base64::Engine as _;
            let mut doc = DrawingScene::new(); let mut at = 0.;
            for image in images {
                let Ok((bytes, ext)) = super::super::image::clipboard_image_data(&image) else { continue };
                let Ok(format) = image::guess_format(&bytes) else { continue };
                let Ok((w, h)) = image::ImageReader::with_format(std::io::Cursor::new(&bytes), format).into_dimensions() else { continue };
                if w == 0 || h == 0 || w > 4096 || h > 4096 || u64::from(w) * u64::from(h) > drawing_scene::MAX_IMAGE_PIXELS { continue; }
                let id = factory::new_id(); let mime = if ext == "jpg" { "image/jpeg".to_string() } else { format!("image/{ext}") };
                let data = scene::scene::BinaryFileData::new(id.clone(), mime.clone(), format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes)));
                let scale = (480. / f64::from(w.max(h))).min(1.);
                doc.scene.elements.push(factory::new_image(at, 0., f64::from(w) * scale, f64::from(h) * scale, Some(id.clone()), &self.style));
                doc.files.insert(id, data); at += f64::from(w) * scale + 16.;
            }
            self.insert_scene(doc, self.pointer, window, cx); return;
        }
        let text = if let Some(fake) = &fake { Some(fake.text.clone()) } else { item.and_then(|item| item.text()) };
        let Some(text) = text else { return };
        if text.len() as u64 > MAX_SOURCE { return; }
        if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&text) && matches!(value["type"].as_str(), Some("excalidraw/clipboard" | "excalidraw")) {
            value["type"] = "excalidraw".into();
            if let Ok(doc) = DrawingScene::load(value.to_string().as_bytes()) {
                self.insert_scene(doc, self.pointer, window, cx); return;
            }
        }
        if !text.trim().is_empty() { self.start_text(self.pointer, None, text, window, cx); }
    }

    pub(super) fn insert_scene(&mut self, source: DrawingScene, at: World, window: &mut Window, cx: &mut Context<Self>) {
        let elements: Vec<_> = source.scene.non_deleted().cloned().collect();
        if elements.is_empty() || elements.len() + self.doc.scene.elements.len() > MAX_ELEMENTS || source.remaining_points() < drawing_scene::MAX_SCENE_POINTS.saturating_sub(self.doc.remaining_points()) { return; }
        let mut next = self.doc.clone();
        let mut elements = clone_elements(&elements);
        let start = next.scene.elements.len();
        next.scene.elements.append(&mut elements);
        for (id, file) in source.files {
            let mut file = file;
            let new_id = if next.files.get(&id).is_some_and(|old| *old != file) { factory::new_id() } else { id };
            for e in &mut next.scene.elements[start..] {
                if let Element::Image(image) = e && image.file_id.as_deref() == Some(file.id.as_str()) { image.file_id = Some(new_id.clone()); }
            }
            file.id.clone_from(&new_id); next.files.insert(new_id, file);
        }
        let indices: Vec<_> = (start..next.scene.elements.len()).collect();
        let Some(b) = selection_bounds(&next.scene, &indices) else { return };
        let delta = bounded_delta(b, at - b.center());
        for &i in &indices { scene::transform::translate_element(&mut next.scene.elements[i], delta.x, delta.y); }
        let Ok(bytes) = next.json() else { return };
        if DrawingScene::load(&bytes).is_err() { return; }
        self.checkpoint(); self.doc = next;
        self.selection = indices.into_iter().filter(|i| !matches!(&self.doc.scene.elements[*i], Element::Text(t) if t.container_id.is_some())).collect();
        self.line_edit = None; self.crop = None; self.selected_points.clear();
        self.tool = Tool::Select; self.panel = Some(Panel::Properties); self.load_images(window); self.rebuild(); cx.notify();
    }
}

pub(super) fn bounded_delta(b: Rect, delta: World) -> World {
    World::new(delta.x.clamp(-MAX_COORD - b.min_x, (MAX_COORD - b.max_x).max(-MAX_COORD - b.min_x)), delta.y.clamp(-MAX_COORD - b.min_y, (MAX_COORD - b.max_y).max(-MAX_COORD - b.min_y)))
}

#[cfg(test)]
mod tests {
    use super::{clone_elements, crop_image, move_vertices, refresh_scene, selection_indices, transform_selection, DrawingScene, Element, ElementOptions, Rect, Scene, World, factory, Arrowhead};

    #[test]
    fn elbow_segment_drag_stays_orthogonal_and_keeps_endpoints() {
        let mut doc = Scene::new();
        let start = World::new(100., 100.); let end = World::new(500., 300.);
        let mut e = factory::new_arrow(vec![start, end], None, Some(Arrowhead::Arrow), &ElementOptions::default());
        if let Element::Arrow(a) = &mut e { a.elbowed = true; super::elbow_points(a); }
        doc.elements.push(e);
        move_vertices(&mut doc, 0, &[1, 2], None, World::zero(), World::new(50., 80.), false);
        let Element::Arrow(a) = &doc.elements[0] else { panic!() };
        assert_eq!(a.linear.points, vec![start, World::new(350., 100.), World::new(350., 300.), end]);
        assert_eq!(a.fixed_segments.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn resizing_a_frame_keeps_child_geometry() {
        let mut doc = Scene::new(); let options = ElementOptions::default();
        let frame = factory::new_frame(0., 0., 300., 200., None, &options);
        let mut child = factory::new_rectangle(100., 100., 100., 80., &options);
        child.base_mut().frame_id = Some(frame.id().to_string()); let original = child.clone();
        doc.elements = vec![frame, child];
        super::resize_selection(&mut doc, &[0], Rect::new(0., 0., 300., 200.), Rect::new(0., 0., 150., 150.));
        assert_eq!(doc.elements[1], original); assert_eq!(doc.elements[0].base().width, 150.);
    }

    #[test]
    fn copies_remap_groups_containers_frames_and_bindings() {
        let opts = ElementOptions::default();
        let mut rect = factory::new_rectangle(10., 20., 100., 80., &opts);
        let mut text = factory::new_text(20., 40., "Bound", 20., &opts);
        let mut arrow = factory::new_arrow(vec![World::new(110., 60.), World::new(200., 60.)], None, None, &opts);
        let frame = factory::new_frame(0., 0., 300., 200., None, &opts);
        rect.base_mut().group_ids = vec!["inner".into(), "outer".into()];
        rect.base_mut().frame_id = Some(frame.id().into());
        if let Element::Text(t) = &mut text { t.container_id = Some(rect.id().into()); }
        if let Element::Arrow(a) = &mut arrow { a.linear.start_binding = Some(super::scene::binding::make_binding(&rect, World::new(110., 60.))); }
        rect.base_mut().bound_elements = Some(vec![super::scene::element::BoundElement { id: text.id().into(), kind: super::scene::element::BoundElementType::Text }]);
        let mut doc = Scene::new(); doc.elements = vec![rect, text, arrow, frame];
        assert_eq!(selection_indices(&doc, &[3]), vec![0, 1, 3]);
        let copied = clone_elements(&doc.elements);
        assert!(copied.iter().zip(&doc.elements).all(|(a, b)| a.id() != b.id()));
        assert_eq!(copied[0].base().frame_id.as_deref(), Some(copied[3].id()));
        assert_ne!(copied[0].base().group_ids, doc.elements[0].base().group_ids);
        assert_eq!(copied[0].base().bound_elements.as_ref().unwrap()[0].id, copied[1].id());
        let Element::Text(t) = &copied[1] else { unreachable!() };
        assert_eq!(t.container_id.as_deref(), Some(copied[0].id()));
        let Element::Arrow(a) = &copied[2] else { unreachable!() };
        assert_eq!(a.linear.start_binding.as_ref().unwrap().element_id, copied[0].id());
        let copied = clone_elements(&doc.elements[2..3]);
        let Element::Arrow(a) = &copied[0] else { unreachable!() };
        assert!(a.linear.start_binding.is_none());
    }

    #[test]
    fn group_transform_keeps_bound_text_rotated_and_round_trips() {
        let opts = ElementOptions::default();
        let rect = factory::new_rectangle(10., 20., 100., 80., &opts);
        let mut text = factory::new_text(0., 0., "Quiet.md", 20., &opts);
        if let Element::Text(t) = &mut text { t.container_id = Some(rect.id().into()); }
        let mut doc = DrawingScene::new(); doc.scene.elements = vec![rect, text];
        refresh_scene(&mut doc.scene);
        transform_selection(&mut doc.scene, &[0], Rect::new(10., 20., 110., 100.), Rect::new(50., 60., 250., 220.), std::f64::consts::FRAC_PI_2);
        let Element::Text(t) = &doc.scene.elements[1] else { unreachable!() };
        assert_eq!(t.font_size, 40.);
        assert_eq!(t.base.angle, doc.scene.elements[0].base().angle);
        let reopened = DrawingScene::load(&doc.json().unwrap()).unwrap();
        assert_eq!(reopened.scene.elements, doc.scene.elements);
        assert!(!reopened.png().unwrap().is_empty());
    }

    #[test]
    fn inserting_and_moving_multiple_vertices_preserves_unselected_points() {
        let mut doc = Scene::new();
        doc.elements.push(factory::new_line(vec![World::new(10., 20.), World::new(110., 20.), World::new(210., 120.)], &ElementOptions::default()));
        move_vertices(&mut doc, 0, &[1], Some((1, World::new(60., 20.))), World::zero(), World::new(0., 30.), false);
        move_vertices(&mut doc, 0, &[1, 2], None, World::zero(), World::new(10., -5.), false);
        let Element::Line(l) = &doc.elements[0] else { unreachable!() };
        assert_eq!(l.linear.points, [World::new(10., 20.), World::new(70., 45.), World::new(120., 15.), World::new(210., 120.)]);
    }

    #[test]
    fn reflected_image_crops_use_the_correct_source_region() {
        let mut image = factory::new_image(0., 0., 200., 100., None, &ElementOptions::default());
        if let Element::Image(i) = &mut image { i.scale[0] = -1.; }
        crop_image(&mut image, Rect::new(0., 0., 200., 100.), Rect::new(0., 0., 100., 50.), World::new(400., 200.));
        let Element::Image(i) = image else { unreachable!() };
        let crop = i.crop.unwrap();
        assert_eq!((crop.x, crop.y, crop.width, crop.height), (200., 0., 200., 100.));
        assert_eq!((i.base.x, i.base.y, i.base.width, i.base.height), (0., 0., 100., 50.));
    }
}

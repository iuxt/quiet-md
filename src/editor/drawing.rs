//! GPUI canvas for Excalidraw blocks. Draft changes stay local until Save.
#[path = "drawing_ui.rs"]
mod chrome;

use super::{drawing_scene::{self, DrawingScene, MAX_COORD, MAX_ELEMENTS, MAX_POINTS, MAX_SOURCE}, render::Env, view::{Editor, EditorEvent}};
use crate::{input::{TextField, TextFieldEvent}, model::{BlockId, BlockKind, Drawing}, settings::tr, store::{self, Store}, theme::{self, Theme}, ui};
use gpui::{prelude::FluentBuilder, *};
use quiet_excalidraw::core::{self as scene, arrowhead::Arrowhead, bounds::element_bounds, element::Element, factory::{self, ElementOptions}, geometry::Point as World, scene::Scene, types::{FillStyle, Roundness, RoundnessType, StrokeStyle, TextAlign as DrawingAlign, VerticalAlign}};
use quiet_excalidraw::render::shape::{Drawable, Op, Rgba as Ink, render_element, render_element_with_lookup};
actions!(drawing, [SaveDrawing, UndoDrawing, RedoDrawing]);

pub(crate) fn bind_keys(cx: &mut App) {
    let context = Some("DrawingEditor");
    cx.bind_keys([
        KeyBinding::new("secondary-s", SaveDrawing, context),
        KeyBinding::new("secondary-z", UndoDrawing, context),
        KeyBinding::new("secondary-shift-z", RedoDrawing, context),
        KeyBinding::new("secondary-y", RedoDrawing, context),
    ]);
}

use std::{cell::Cell, collections::{HashMap, VecDeque}, rc::Rc, sync::Arc};

#[derive(Clone)]
enum Target {
    Insert { generation: u64, selection: super::core::Selection },
    Edit(BlockId, Drawing),
}

impl Target {
    fn valid(&self, ed: &Editor) -> bool {
        !ed.read_only && !ed.markdown && match self {
            Self::Insert { generation, selection } => *generation == ed.core.generation && *selection == ed.core.sel,
            Self::Edit(id, was) => matches!(ed.core.block(*id).map(|b| &b.kind), Some(BlockKind::Drawing(now)) if now == was),
        }
    }
}

impl Editor {
    pub(super) fn new_drawing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only || self.markdown { return; }
        let target = Target::Insert { generation: self.core.generation, selection: self.core.sel };
        self.show_drawing(target, DrawingScene::new(), window, cx);
    }

    fn edit_drawing(&mut self, id: BlockId, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only || self.markdown { return; }
        let Some(BlockKind::Drawing(d)) = self.core.block(id).map(|b| &b.kind) else { return };
        let target = Target::Edit(id, d.clone());
        let (root, source) = (Store::root_dir(), d.source.clone());
        cx.spawn_in(window, async move |this, cx| {
            let loaded = cx.background_executor().spawn(async move { DrawingScene::read(&root, &source) }).await;
            let _ = this.update_in(cx, |ed, window, cx| {
                if ed.drawing.is_some() || !target.valid(ed) { return; }
                match loaded {
                    Ok(doc) => ed.show_drawing(target, doc, window, cx),
                    Err(e) => { log::warn!("load drawing: {e:#}"); ed.toast(tr("无法打开画板", "Could not open drawing"), cx); }
                }
            });
        }).detach();
    }

    fn show_drawing(&mut self, target: Target, doc: DrawingScene, window: &mut Window, cx: &mut Context<Self>) {
        let parent = cx.weak_entity();
        let view = cx.new(|cx| DrawingEditor::new(parent, target, doc, window, cx));
        window.focus(&view.read(cx).focus);
        self.drawing = Some(view);
        cx.emit(EditorEvent::Drawing);
        cx.notify();
    }

    pub(super) fn drawing_block(&self, id: BlockId, d: &Drawing, env: Env, cx: &mut Context<Self>) -> AnyElement {
        let width = (d.width as f32 / 2.).clamp(32., 640.).min(if env.width > 0. { env.width } else { 640. });
        let ratio = (d.width as f32 / d.height as f32).clamp(0.05, 20.);
        let height = (width / ratio).clamp(42., 420.);
        let preview = super::image::drawing_picture(d, width.max(height));
        let editable = !self.read_only;
        div().id(("drawing-block", id.0)).flex().flex_col().items_start().max_w_full()
            .child(div().relative().w_full().max_w_full().w(px(width)).h(px(height)).flex().justify_center().items_center().overflow_hidden().rounded(px(6.))
                .child(preview))
            .when(editable, |d| d.cursor_pointer().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |ed, _, window, cx| ed.edit_drawing(id, window, cx)))
                .child(div().flex().items_center().gap(px(5.)).pt(px(4.)).text_size(px(12.)).text_color(Theme::text_caption())
                    .child(ui::icon("pen-line", 13., Theme::text_caption())).child(tr("编辑画板", "Edit drawing"))))
            .into_any_element()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool { Pan, Select, Rect, Diamond, Ellipse, Arrow, Line, Pen, Text, Image, Sticky, Eraser, Frame }

const TOOLS: &[(Tool, &str, &str, &str, &str)] = &[
    (Tool::Pan, "平移", "Hand", "h", "drawing-hand"),
    (Tool::Select, "选择", "Selection", "v", "drawing-pointer"),
    (Tool::Rect, "矩形", "Rectangle", "r", "drawing-rectangle"),
    (Tool::Diamond, "菱形", "Diamond", "d", "drawing-diamond"),
    (Tool::Ellipse, "椭圆", "Ellipse", "o", "drawing-ellipse"),
    (Tool::Arrow, "箭头", "Arrow", "a", "drawing-arrow"),
    (Tool::Line, "直线", "Line", "l", "minus"),
    (Tool::Pen, "画笔", "Freehand", "p", "pen-line"),
    (Tool::Text, "文字", "Text", "t", "type"),
    (Tool::Image, "图片", "Image", "9", "image"),
    (Tool::Sticky, "便签", "Sticky note", "n", "drawing-note"),
    (Tool::Eraser, "橡皮擦", "Eraser", "e", "drawing-eraser"),
    (Tool::Frame, "画框", "Frame", "f", "drawing-corners"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Panel { Menu, Properties, More, Export }

#[derive(Clone)]
struct DrawingState { scene: Scene, background: String }

#[derive(Clone, Copy)]
enum StyleChoice {
    Stroke(&'static str), Fill(&'static str), Width(f64), StrokeStyle(StrokeStyle), FillStyle(FillStyle), Roughness(f64), Round(bool), Opacity(f64),
    TextSize(f64), TextAlign(DrawingAlign), ArrowHead(Option<Arrowhead>),
}

#[derive(Clone, Copy)]
enum LayerMove { Bottom, Back, Forward, Top }

#[derive(Clone, Copy)]
enum ExportKind { Source, Png, Svg }

enum Gesture {
    Resize { pointer: World, corner: usize, start: scene::bounds::Bounds, rect: scene::bounds::Bounds, index: usize },
    Rotate { start: f64, angle: f64, index: usize },
    Erase { indices: Vec<usize>, last: World },
    Pan { start: Point<Pixels>, offset: Point<Pixels> },
    Move { start: World, delta: World, index: usize },
    Shape { start: World, end: World, ink: Vec<World>, point_limit: usize },
}

pub(crate) struct DrawingEditor {
    parent: WeakEntity<Editor>,
    target: Target,
    doc: DrawingScene,
    focus: FocusHandle,
    field: Entity<TextField>,
    _field_sub: Subscription,
    text_at: Option<(World, Option<usize>)>,
    text_container: Option<usize>,
    edit_link: Option<usize>,
    tool_locked: bool,
    panel: Option<Panel>,
    grid: bool,
    colors: Option<chrome::Colors>,
    zen: bool,
    dark_canvas: bool,
    reset_confirmation: bool,
    style: ElementOptions,
    rounded: bool,
    text_size: f64,
    text_align: DrawingAlign,
    arrowhead: Option<Arrowhead>,
    opacity_dragging: Option<bool>,
    opacity_bounds: Rc<Cell<Bounds<Pixels>>>,
    pending_image: Option<(scene::scene::BinaryFileData, u32, u32)>,
    tool: Tool,
    selected: Option<usize>,
    gesture: Option<Gesture>,
    viewport: Rc<Cell<Bounds<Pixels>>>,
    offset: Point<Pixels>,
    zoom: f64,
    fit: bool,
    undo: VecDeque<DrawingState>,
    redo: Vec<DrawingState>,
    saved: DrawingState,
    imported: bool,
    artwork: Rc<Vec<Vec<Drawable>>>,
    images: Rc<HashMap<String, Arc<RenderImage>>>,
    busy: bool,
    close_after_save: bool,
    saving: bool,
    discard: bool,
    error: Option<&'static str>,
}

impl DrawingEditor {
    fn new(parent: WeakEntity<Editor>, target: Target, doc: DrawingScene, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.on_release_in(window, |d, window, _| d.drop_images(window)).detach();
        let field = cx.new(|cx| {
            let mut f = TextField::new(tr("输入画板文字", "Enter drawing text"), cx);
            f.max_len = Some(256); f
        });
        let field_sub = cx.subscribe_in(&field, window, |d, _, event, window, cx| match event {
            TextFieldEvent::Submit => d.finish_field(window, cx),
            TextFieldEvent::Cancel => { d.text_at = None; d.edit_link = None; window.focus(&d.focus); cx.notify(); }
            _ => {}
        });
        let saved = DrawingState { scene: doc.scene.clone(), background: doc.background.clone() };
        let mut this = Self { parent, target, doc, focus: cx.focus_handle(), field, _field_sub: field_sub, text_at: None, text_container: None,
            edit_link: None, tool_locked: false, panel: None, colors: None, grid: false, zen: false, dark_canvas: false, reset_confirmation: false,
            style: ElementOptions::default(), rounded: true, text_size: 20., text_align: DrawingAlign::Left, arrowhead: Some(Arrowhead::Arrow),
            opacity_dragging: None, opacity_bounds: Rc::default(), pending_image: None,
            tool: Tool::Select, selected: None, gesture: None, viewport: Rc::new(Cell::new(Bounds::default())), offset: point(px(0.), px(0.)), zoom: 1., fit: true,
            undo: VecDeque::new(), redo: Vec::new(), saved, imported: false, artwork: Rc::default(), images: Rc::default(), busy: false, close_after_save: false, saving: false, discard: false, error: None };
        this.rebuild();
        this.load_images(window);
        this
    }

    fn rebuild(&mut self) {
        let lookup = self.doc.scene.non_deleted().map(|e| (e.id(), e)).collect();
        self.artwork = Rc::new(self.doc.scene.elements.iter().map(|e| {
            if e.is_deleted() { vec![] } else { render_element_with_lookup(e, &lookup) }
        }).collect());
    }

    fn decode_image(file: &scene::scene::BinaryFileData) -> Option<Arc<RenderImage>> {
        let (bytes, format) = drawing_scene::image_data(file).ok()?;
        let mut data = image::ImageReader::with_format(std::io::Cursor::new(bytes), format).decode().ok()?.into_rgba8();
        for p in data.pixels_mut() { p.0.swap(0, 2); }
        Some(Arc::new(RenderImage::new(vec![image::Frame::new(data)])))
    }

    fn drop_images(&self, window: &mut Window) {
        if self.images.is_empty() { return; }
        // The atlas owns a separate copy. Redraw before presenting a scene that used these tiles.
        window.refresh();
        for image in self.images.values() { let _ = window.drop_image(image.clone()); }
    }

    fn load_images(&mut self, window: &mut Window) {
        self.drop_images(window);
        self.images = Rc::new(self.doc.files.iter().filter_map(|(id, file)| Some((id.clone(), Self::decode_image(file)?))).collect());
    }

    fn dirty(&self) -> bool {
        self.imported || self.saved.scene.elements != self.doc.scene.elements || self.saved.background != self.doc.background
    }

    fn checkpoint(&mut self) {
        if self.undo.len() == 32 { self.undo.pop_front(); }
        self.undo.push_back(DrawingState { scene: self.doc.scene.clone(), background: self.doc.background.clone() });
        self.redo.clear();
    }

    fn history(&mut self, redo: bool, cx: &mut Context<Self>) {
        if self.gesture.is_some() || self.text_at.is_some() || self.edit_link.is_some() { return; }
        if redo {
            if let Some(state) = self.redo.pop() { let current = self.swap_state(state); self.undo.push_back(current); }
        } else if let Some(state) = self.undo.pop_back() { let current = self.swap_state(state); self.redo.push(current); }
        self.selected = None;
        self.rebuild();
        cx.notify();
    }

    fn swap_state(&mut self, state: DrawingState) -> DrawingState {
        DrawingState { scene: std::mem::replace(&mut self.doc.scene, state.scene), background: std::mem::replace(&mut self.doc.background, state.background) }
    }

    fn close(&self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.parent.update(cx, |ed, cx| {
            ed.drawing = None;
            ed.focus(window);
            cx.emit(EditorEvent::Drawing);
            cx.notify();
        });
    }

    pub(crate) fn confirm_exit(&mut self, cx: &mut Context<Self>) -> bool {
        if self.busy { self.close_after_save = self.saving; return true; }
        if !self.dirty() && self.text_at.is_none() && self.edit_link.is_none() && self.gesture.is_none() { return false; }
        self.discard = true;
        self.gesture = None;
        cx.notify();
        true
    }

    fn keep_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.discard = false;
        let _ = self.parent.update(cx, |_, cx| cx.emit(EditorEvent::DrawingExitCanceled));
        let focus = if self.text_at.is_some() || self.edit_link.is_some() { self.field.read(cx).focus_handle(cx) } else { self.focus.clone() };
        window.focus(&focus);
        cx.notify();
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy { return; }
        if !self.confirm_exit(cx) { self.close(window, cx); }
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.gesture.is_some() { return; }
        self.finish_field(window, cx);
        if !self.dirty() { return; }
        self.busy = true;
        self.saving = true;
        self.error = None;
        let doc = self.doc.clone();
        let saved = DrawingState { scene: doc.scene.clone(), background: doc.background.clone() };
        let root = Store::root_dir();
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let encoded = cx.background_executor().spawn(async move { Ok::<_, anyhow::Error>((doc.json()?, doc.preview_dimensions()?)) }).await;
            let _ = this.update_in(cx, |d, window, cx| {
                d.busy = false;
                d.saving = false;
                let exit = std::mem::take(&mut d.close_after_save);
                let result = encoded.and_then(|(source, (w, h))| {
                    d.parent.update(cx, |ed, _| -> anyhow::Result<_> {
                        anyhow::ensure!(d.target.valid(ed), "drawing target changed");
                        Store::save_drawing(&root, &source, w, h)
                    })?
                });
                match result {
                    Ok(drawing) => {
                        let result = d.parent.update(cx, |ed, cx| {
                            let id = match d.target {
                                Target::Insert { .. } => ed.core.insert_drawing(drawing.clone()),
                                Target::Edit(id, _) => { ed.core.replace_drawing(id, drawing.clone()); id }
                            };
                            ed.changed_preserving_viewport(window, cx);
                            id
                        });
                        if let Ok(id) = result {
                            d.target = Target::Edit(id, drawing);
                            d.saved = saved;
                            d.imported = false;
                        }
                        if exit { d.close(window, cx); } else { cx.notify(); }
                    }
                    Err(e) => { log::warn!("save drawing: {e:#}"); d.error = Some(tr("无法保存画板。可以先导出图源。", "Could not save drawing. You can export the source.")); cx.notify(); }
                }
            });
        }).detach();
    }

    fn import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy { return; }
        self.busy = true;
        self.error = None;
        cx.notify();
        let picked = crate::devtools::take_pick();
        let rx = picked.is_none().then(|| cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false,
            prompt: Some(tr("导入 Excalidraw", "Import Excalidraw").into()), extensions: Some(vec!["excalidraw".into()]) }));
        cx.spawn_in(window, async move |this, cx| {
            let paths = match rx { Some(rx) => rx.await.ok().and_then(Result::ok).flatten(), None => picked };
            let doc = if let Some(path) = paths.and_then(|p| p.into_iter().next()) {
                Some(cx.background_executor().spawn(async move { DrawingScene::load(&store::read_bounded(&path, MAX_SOURCE)?) }).await)
            } else { None };
            let _ = this.update_in(cx, |d, window, cx| {
                d.busy = false;
                if let Some(result) = doc {
                    match result {
                        Ok(doc) => { d.pending_image = None; d.doc = doc; d.imported = true; d.undo.clear(); d.redo.clear(); d.selected = None; d.text_at = None; d.fit = true; d.load_images(window); d.rebuild(); }
                        Err(e) => { log::warn!("import drawing: {e:#}"); d.error = Some(tr("无法导入此 Excalidraw 文件", "Could not import this Excalidraw file")); }
                    }
                }
                cx.notify();
            });
        }).detach();
    }

    fn export(&mut self, kind: ExportKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy { return; }
        self.finish_field(window, cx);
        let name = match kind { ExportKind::Source => "Drawing.excalidraw", ExportKind::Png => "Drawing.png", ExportKind::Svg => "Drawing.svg" };
        let path = crate::devtools::take_pick().and_then(|p| p.into_iter().next());
        let dir = dirs::download_dir().or_else(dirs::document_dir).unwrap_or_default();
        let rx = path.is_none().then(|| cx.prompt_for_new_path(&dir, Some(name)));
        let doc = self.doc.clone();
        cx.spawn_in(window, async move |this, cx| {
            let picked = match rx { Some(rx) => rx.await.ok().and_then(Result::ok).flatten(), None => path };
            let Some(path) = picked else { return };
            let result = cx.background_executor().spawn(async move {
                let bytes = match kind {
                    ExportKind::Source => doc.json()?, ExportKind::Png => doc.png()?,
                    ExportKind::Svg => doc.svg().into_bytes(),
                };
                Ok::<_, anyhow::Error>(std::fs::write(path, bytes)?)
            }).await;
            if let Err(e) = result {
                log::warn!("export drawing: {e:#}");
                let _ = this.update_in(cx, |d, _, cx| { d.error = Some(tr("无法导出画板", "Could not export drawing")); cx.notify(); });
            }
        }).detach();
    }

    fn world(&self, p: Point<Pixels>) -> World {
        let delta = p - self.viewport.get().origin - self.offset;
        World::new((f64::from(f32::from(delta.x)) / self.zoom).clamp(-MAX_COORD, MAX_COORD), (f64::from(f32::from(delta.y)) / self.zoom).clamp(-MAX_COORD, MAX_COORD))
    }

    fn fit_view(&mut self) {
        let view = self.viewport.get();
        if view.size.width <= px(1.) || view.size.height <= px(1.) { return; }
        if let Some(b) = drawing_scene::bounds(&self.doc.scene) {
            let w = f64::from(f32::from(view.size.width)); let h = f64::from(f32::from(view.size.height));
            self.zoom = ((w - 56.) / b.width().max(1.)).min((h - 56.) / b.height().max(1.)).clamp(0.05, 1.);
            self.offset = point(px(w as f32 / 2. - (b.center().x * self.zoom) as f32), px(h as f32 / 2. - (b.center().y * self.zoom) as f32));
        } else { self.offset = point(px(0.), px(0.)); self.zoom = 1.; }
        self.fit = false;
    }

    fn zoom_at(&mut self, at: Point<Pixels>, factor: f64) {
        let at_world = self.world(at);
        self.zoom = (self.zoom * factor).clamp(0.05, 8.);
        self.offset = at - self.viewport.get().origin - point(px((at_world.x * self.zoom) as f32), px((at_world.y * self.zoom) as f32));
    }

    fn select_at(&self, p: World) -> Option<usize> {
        hit_at(&self.doc.scene, p, self.zoom, &[])
    }

    fn down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.discard || self.reset_confirmation { return; }
        self.finish_field(window, cx);
        window.focus(&self.focus);
        cx.stop_propagation();
        let p = self.world(event.position);
        if event.button == MouseButton::Middle || self.tool == Tool::Pan {
            self.gesture = Some(Gesture::Pan { start: event.position, offset: self.offset });
        } else if self.tool == Tool::Select {
            if let Some(index) = self.selected {
                let e = &self.doc.scene.elements[index];
                if can_resize(e) {
                    let b = element_bounds(e);
                    let corners = rect_corners(b);
                    if let Some(corner) = corners.iter().position(|q| p.distance(*q) <= 9. / self.zoom) {
                        self.gesture = Some(Gesture::Resize { pointer: p, corner, start: b, rect: b, index });
                        cx.notify(); return;
                    }
                }
                if can_rotate(e) && p.distance(rotation_handle(e, self.zoom)) <= 9. / self.zoom {
                    let pivot = element_center(e);
                    self.gesture = Some(Gesture::Rotate { start: (p.y - pivot.y).atan2(p.x - pivot.x), angle: e.base().angle, index });
                    cx.notify(); return;
                }
            }
            self.selected = self.select_at(p);
            if let Some(i) = self.selected {
                if self.panel.is_none() { self.panel = Some(Panel::Properties); }
                if event.click_count == 2 { self.edit_selection_text(window, cx); }
                else { self.gesture = Some(Gesture::Move { start: p, delta: World::zero(), index: i }); }
            } else if self.panel == Some(Panel::Properties) && event.click_count < 2 {
                self.panel = None;
            }
        } else if self.tool == Tool::Text {
            self.start_text(p, None, String::new(), window, cx);
        } else if self.tool == Tool::Image {
            if self.pending_image.is_some() { self.place_image(p, cx); } else { self.pick_image(window, cx); }
        } else if self.tool == Tool::Eraser {
            self.gesture = Some(Gesture::Erase { indices: self.select_at(p).into_iter().collect(), last: p });
        } else if self.doc.scene.elements.len() < MAX_ELEMENTS {
            self.selected = None;
            let point_limit = self.doc.remaining_points().min(MAX_POINTS);
            if point_limit >= 2 || !matches!(self.tool, Tool::Pen | Tool::Line | Tool::Arrow) {
                self.gesture = Some(Gesture::Shape { start: p, end: p, ink: vec![p], point_limit });
            }
        }
        cx.notify();
    }

    fn moved(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let p = self.world(event.position);
        match &mut self.gesture {
            Some(Gesture::Pan { start, offset }) => self.offset = *offset + event.position - *start,
            Some(Gesture::Move { start, delta, .. }) => *delta = p - *start,
            Some(Gesture::Resize { pointer, corner, start, rect, .. }) => {
                let delta = p - *pointer;
                let q = rect_corners(*start)[*corner] + delta;
                *rect = resize_corner(*start, *corner, q, event.modifiers.shift);
            }
            Some(Gesture::Rotate { start, angle, index }) => {
                let e = &self.doc.scene.elements[*index];
                let pivot = element_center(e);
                *angle = (e.base().angle + (p.y - pivot.y).atan2(p.x - pivot.x) - *start).rem_euclid(std::f64::consts::TAU);
                if event.modifiers.shift { *angle = (*angle / std::f64::consts::FRAC_PI_4).round() * std::f64::consts::FRAC_PI_4; }
            }
            Some(Gesture::Erase { indices, last }) => {
                let steps = (p.distance(*last) * self.zoom / 6.).ceil().clamp(1., 128.) as usize;
                for step in 1..=steps {
                    let sample = last.lerp(p, step as f64 / steps as f64);
                    if let Some(i) = hit_at(&self.doc.scene, sample, self.zoom, indices) { indices.push(i); }
                }
                *last = p;
            }
            Some(Gesture::Shape { start, end, ink, point_limit }) => {
                let point = if event.modifiers.shift {
                    if matches!(self.tool, Tool::Line | Tool::Arrow) { scene::binding::constrain_angle(*start, p, 45.) }
                    else if self.tool != Tool::Pen { scene::binding::constrain_square(*start, p) } else { p }
                } else { p };
                *end = World::new(point.x.clamp(-MAX_COORD, MAX_COORD), point.y.clamp(-MAX_COORD, MAX_COORD));
                if self.tool == Tool::Pen && ink.len() < *point_limit && ink.last().is_some_and(|last| last.distance(p) >= 1. / self.zoom) { ink.push(p); }
            }
            None => return,
        }
        cx.notify();
    }

    fn up(&mut self, cx: &mut Context<Self>) {
        match self.gesture.take() {
            Some(Gesture::Move { delta, index, .. }) if delta.length() > 0.5 => {
                self.checkpoint();
                let element = &mut self.doc.scene.elements[index];
                let b = element_bounds(element);
                let dx = delta.x.clamp(-MAX_COORD - b.min_x, (MAX_COORD - b.max_x).max(-MAX_COORD - b.min_x));
                let dy = delta.y.clamp(-MAX_COORD - b.min_y, (MAX_COORD - b.max_y).max(-MAX_COORD - b.min_y));
                scene::transform::translate_element(element, dx, dy);
                touch(element);
                scene::binding::refresh_derived(&mut self.doc.scene);
                self.rebuild();
            }
            Some(Gesture::Resize { rect, start, index, .. }) if rect != start => {
                self.checkpoint();
                let e = &mut self.doc.scene.elements[index];
                apply_rect(e, rect);
                touch(e);
                scene::binding::refresh_derived(&mut self.doc.scene);
                self.rebuild();
            }
            Some(Gesture::Rotate { angle, index, .. }) if (self.doc.scene.elements[index].base().angle - angle).abs() > 0.001 => {
                self.checkpoint();
                self.doc.scene.elements[index].base_mut().angle = angle;
                touch(&mut self.doc.scene.elements[index]);
                self.rebuild();
            }
            Some(Gesture::Erase { indices, .. }) if !indices.is_empty() => {
                self.checkpoint();
                for index in indices { self.erase_element(index); }
                self.rebuild();
            }
            Some(Gesture::Shape { start, end, ink, .. }) => {
                if let Some(mut e) = self.shape(start, end, &ink) {
                    self.checkpoint();
                    if let Element::Arrow(a) = &mut e {
                        a.linear.start_binding = arrow_binding(&self.doc.scene, start);
                        a.linear.end_binding = arrow_binding(&self.doc.scene, end);
                    }
                    let index = self.doc.scene.elements.len();
                    self.doc.scene.elements.push(e);
                    self.after_drawing(index);
                    scene::binding::update_bindings(&mut self.doc.scene);
                    self.rebuild();
                }
            }
            Some(_) => {}
            None => return,
        }
        cx.notify();
    }

    fn shape(&self, start: World, end: World, ink: &[World]) -> Option<Element> {
        let far_enough = start.distance(end) >= 2. / self.zoom || self.tool == Tool::Pen && ink.len() >= 2;
        if !far_enough && self.tool != Tool::Sticky { return None; }
        let (x, y) = (start.x.min(end.x), start.y.min(end.y));
        let (w, h) = ((end.x - start.x).abs(), (end.y - start.y).abs());
        let o = &self.style;
        let mut shape = match self.tool {
            Tool::Rect => factory::new_rectangle(x, y, w.max(1.), h.max(1.), o),
            Tool::Diamond => factory::new_diamond(x, y, w.max(1.), h.max(1.), o),
            Tool::Ellipse => factory::new_ellipse(x, y, w.max(1.), h.max(1.), o),
            Tool::Line => factory::new_line(vec![start, end], o),
            Tool::Arrow => factory::new_arrow(vec![start, end], None, self.arrowhead, o),
            Tool::Pen if ink.len() > 1 => factory::new_freedraw(ink.to_vec(), o),
            Tool::Sticky => factory::new_sticky_note(x, y, if far_enough { w.max(36.) } else { 160. }, if far_enough { h.max(24.) } else { 100. }, o),
            Tool::Frame => factory::new_frame(x, y, w.max(1.), h.max(1.), Some(tr("画框", "Frame").into()), o),
            _ => return None,
        };
        if matches!(self.tool, Tool::Rect | Tool::Diamond | Tool::Arrow | Tool::Line) {
            shape.base_mut().roundness = self.rounded.then(|| roundness(self.tool));
        }
        Some(shape)
    }

    fn start_text(&mut self, p: World, index: Option<usize>, text: String, window: &mut Window, cx: &mut Context<Self>) {
        if index.is_none() && self.doc.scene.elements.len() >= MAX_ELEMENTS { return; }
        self.text_at = Some((p, index));
        self.text_container = None;
        self.field.update(cx, |f, cx| { f.set_placeholder(tr("输入画板文字", "Enter drawing text"), cx); f.set_text(&text, cx); f.select_all_text(cx); });
        window.focus(&self.field.read(cx).focus_handle(cx));
    }

    fn finish_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((at, index)) = self.text_at.take() else { return };
        let container = self.text_container.take();
        let value = self.field.read(cx).text.trim().to_string();
        if !value.is_empty() {
            if let Some(i) = index {
                if matches!(&self.doc.scene.elements[i], Element::Text(t) if t.text != value) {
                    self.checkpoint();
                    if let Element::Text(t) = &mut self.doc.scene.elements[i] {
                        t.base.width = scene::text::text_width(&value, t.font_size);
                        t.text = value.clone(); t.original_text = value;
                    }
                    touch(&mut self.doc.scene.elements[i]);
                }
            } else {
                self.checkpoint();
                let mut el = factory::new_text(at.x, at.y, &value, self.text_size, &self.style);
                el.base_mut().width = scene::text::text_width(&value, self.text_size);
                if let Element::Text(t) = &mut el { t.text_align = self.text_align; }
                let index = self.doc.scene.elements.len();
                if let Some(c) = container && let Element::Text(t) = &mut el {
                    let shape = &mut self.doc.scene.elements[c];
                    t.container_id = Some(shape.id().to_string());
                    shape.base_mut().bound_elements.get_or_insert_with(Vec::new)
                        .push(scene::element::BoundElement { id: t.base.id.clone(), kind: scene::element::BoundElementType::Text });
                    touch(shape);
                }
                self.doc.scene.elements.push(el);
                if let Some(c) = container {
                    self.selected = Some(c);
                    scene::binding::sync_container_text(&mut self.doc.scene);
                } else { self.after_drawing(index); }
            }
            self.rebuild();
        }
        window.focus(&self.focus);
        cx.notify();
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        if self.busy { return; }
        if let Some(i) = self.selected.take() {
            self.checkpoint();
            self.erase_element(i);
            self.rebuild();
            cx.notify();
        }
    }

    fn after_drawing(&mut self, index: usize) {
        if !self.tool_locked {
            self.tool = Tool::Select;
            self.selected = Some(index);
            if self.panel.is_none() { self.panel = Some(Panel::Properties); }
        }
    }

    fn erase_element(&mut self, index: usize) {
        let id = self.doc.scene.elements[index].id().to_string();
        for (i, element) in self.doc.scene.elements.iter_mut().enumerate() {
            let following = matches!(element, Element::Text(t) if t.container_id.as_deref() == Some(id.as_str()));
            if (i == index || following) && !element.is_deleted() {
                element.base_mut().is_deleted = true;
                touch(element);
            }
        }
    }

    fn duplicate(&mut self, cx: &mut Context<Self>) {
        if self.busy { return; }
        let Some(i) = self.selected else { return };
        let mut e = self.doc.scene.elements[i].clone();
        let labels: Vec<_> = self.doc.scene.non_deleted().filter(|item| {
            matches!(item, Element::Text(t) if t.container_id.as_deref() == Some(e.id()))
        }).cloned().collect();
        if self.doc.scene.elements.len() + labels.len() >= MAX_ELEMENTS { return; }
        let points = match &e { Element::Line(l) => l.linear.points.len(), Element::Arrow(a) => a.linear.points.len(), Element::Freedraw(f) => f.points.len(), _ => 0 };
        if points > self.doc.remaining_points() { return; }
        self.checkpoint();
        let b = e.base_mut();
        b.id = factory::new_id(); b.index = None; b.group_ids.clear(); b.bound_elements = None;
        match &mut e {
            Element::Line(l) => { l.linear.start_binding = None; l.linear.end_binding = None; }
            Element::Arrow(a) => { a.linear.start_binding = None; a.linear.end_binding = None; }
            Element::Text(t) => { t.container_id = None; }
            _ => {}
        }
        let b = labels.iter().map(element_bounds).fold(element_bounds(&e), |b, label| b.union(&label));
        let dx = (MAX_COORD - b.max_x).clamp(0., 12.);
        let dy = (MAX_COORD - b.max_y).clamp(0., 12.);
        scene::transform::translate_element(&mut e, dx, dy);
        let id = e.id().to_string();
        let labels: Vec<_> = labels.into_iter().map(|mut label| {
            let Element::Text(t) = &mut label else { unreachable!() };
            t.base.id = factory::new_id(); t.base.index = None; t.base.group_ids.clear(); t.container_id = Some(id.clone());
            e.base_mut().bound_elements.get_or_insert_with(Vec::new)
                .push(scene::element::BoundElement { id: t.base.id.clone(), kind: scene::element::BoundElementType::Text });
            scene::transform::translate_element(&mut label, dx, dy);
            label
        }).collect();
        self.selected = Some(self.doc.scene.elements.len());
        self.doc.scene.elements.push(e);
        self.doc.scene.elements.extend(labels);
        self.rebuild();
        cx.notify();
    }

    fn reorder(&mut self, direction: LayerMove, cx: &mut Context<Self>) {
        if self.busy { return; }
        let Some(index) = self.selected else { return };
        let scene = &self.doc.scene.elements;
        let first = || scene.iter().position(|e| !e.is_deleted());
        let last = || scene.iter().rposition(|e| !e.is_deleted());
        let dest = match direction {
            LayerMove::Bottom => first(), LayerMove::Top => last(),
            LayerMove::Back => scene[..index].iter().rposition(|e| !e.is_deleted()),
            LayerMove::Forward => scene[index + 1..].iter().position(|e| !e.is_deleted()).map(|i| index + 1 + i),
        };
        let Some(dest) = dest.filter(|d| *d != index) else { return };
        self.checkpoint();
        let scene = &mut self.doc.scene.elements;
        let mut e = scene.remove(index);
        // Keep the file order and let Excalidraw regenerate the moved index.
        e.base_mut().index = None;
        touch(&mut e);
        scene.insert(dest, e);
        self.selected = Some(dest);
        self.rebuild();
        cx.notify();
    }

    fn change_paper(&mut self, color: &str, cx: &mut Context<Self>) {
        if self.busy { return; }
        if self.doc.background == color { return; }
        self.checkpoint();
        self.doc.background = color.to_string();
        cx.notify();
    }

    fn reset_canvas(&mut self, cx: &mut Context<Self>) {
        self.reset_confirmation = false;
        self.panel = None;
        if self.doc.scene.non_deleted().next().is_none() { return cx.notify(); }
        self.checkpoint();
        self.doc.scene.elements.clear();
        self.selected = None;
        self.rebuild();
        cx.notify();
    }

    fn apply_style(&mut self, choice: StyleChoice, checkpoint: bool, cx: &mut Context<Self>) {
        if self.busy { return; }
        match choice {
            StyleChoice::Stroke(c) => self.style.stroke_color = c.into(),
            StyleChoice::Fill(c) => self.style.background_color = c.into(),
            StyleChoice::Width(v) => self.style.stroke_width = v,
            StyleChoice::StrokeStyle(v) => self.style.stroke_style = v,
            StyleChoice::FillStyle(v) => self.style.fill_style = v,
            StyleChoice::Roughness(v) => self.style.roughness = v,
            StyleChoice::Opacity(v) => self.style.opacity = v,
            StyleChoice::Round(v) => self.rounded = v,
            StyleChoice::TextSize(v) => self.text_size = v,
            StyleChoice::TextAlign(v) => self.text_align = v,
            StyleChoice::ArrowHead(v) => self.arrowhead = v,
        }
        if let Some(i) = self.selected {
            let mut e = self.doc.scene.elements[i].clone();
            let base = e.base_mut();
            match choice {
                StyleChoice::Stroke(c) => base.stroke_color = c.into(),
                StyleChoice::Fill(c) => base.background_color = c.into(),
                StyleChoice::Width(v) => base.stroke_width = v,
                StyleChoice::StrokeStyle(v) => base.stroke_style = v,
                StyleChoice::FillStyle(v) => base.fill_style = v,
                StyleChoice::Roughness(v) => base.roughness = v,
                StyleChoice::Opacity(v) => base.opacity = v,
                StyleChoice::Round(v) => {
                    let tool = match e { Element::Line(_) | Element::Arrow(_) => Tool::Line, Element::Diamond { .. } => Tool::Diamond, _ => Tool::Rect };
                    e.base_mut().roundness = v.then(|| roundness(tool));
                }
                StyleChoice::TextSize(font) => if let Element::Text(t) = &mut e {
                    t.font_size = font;
                    t.base.width = scene::text::text_width(&t.text, font);
                    t.base.height = scene::text::text_height(&t.text, font, t.line_height);
                },
                StyleChoice::TextAlign(v) => if let Element::Text(t) = &mut e { t.text_align = v; },
                StyleChoice::ArrowHead(v) => if let Element::Arrow(a) = &mut e { a.linear.end_arrowhead = v; },
            }
            if e != self.doc.scene.elements[i] {
                if checkpoint { self.checkpoint(); }
                touch(&mut e);
                self.doc.scene.elements[i] = e;
                self.rebuild();
            }
        }
        cx.notify();
    }

    fn open_link_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy { return; }
        let Some(i) = self.selected else { return };
        let url = self.doc.scene.elements[i].base().link.clone().unwrap_or_default();
        self.edit_link = Some(i);
        self.field.update(cx, |f, cx| { f.set_placeholder(tr("添加链接", "Add link"), cx); f.set_text(&url, cx); f.select_all_text(cx); });
        window.focus(&self.field.read(cx).focus_handle(cx));
        cx.notify();
    }

    fn finish_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.edit_link.take() {
            let raw = self.field.read(cx).text.trim().to_string();
            let link = (!raw.is_empty()).then_some(raw);
            if self.doc.scene.elements[index].base().link != link {
                self.checkpoint();
                let e = &mut self.doc.scene.elements[index];
                e.base_mut().link = link;
                touch(e);
                self.rebuild();
            }
            window.focus(&self.focus);
            cx.notify();
        } else {
            self.finish_text(window, cx);
        }
    }

    fn edit_selection_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(i) = self.selected else { return };
        let e = &self.doc.scene.elements[i];
        if let Element::Text(t) = e {
            self.start_text(element_center(e), Some(i), t.text.clone(), window, cx);
        } else if let Some((idx, text)) = self.doc.scene.elements.iter().enumerate().find_map(|(idx, item)| {
            if let Element::Text(t) = item && !item.is_deleted() && t.container_id.as_deref() == Some(e.id()) { Some((idx, t.text.clone())) } else { None }
        }) {
            self.start_text(element_center(e), Some(idx), text, window, cx);
        } else if matches!(e, Element::StickyNote(_) | Element::Rectangle { .. } | Element::Ellipse { .. } | Element::Diamond { .. })
            && self.doc.scene.elements.len() < MAX_ELEMENTS {
            self.start_text(element_center(e), None, String::new(), window, cx);
            self.text_container = Some(i);
        }
        cx.notify();
    }

    fn pick_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.pending_image.is_some() { return; }
        self.busy = true;
        self.error = None;
        cx.notify();
        let picked = crate::devtools::take_pick();
        let rx = picked.is_none().then(|| cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false,
            prompt: Some(tr("插入图片", "Insert image").into()), extensions: Some(["png", "jpg", "jpeg", "gif", "webp"].map(Into::into).to_vec()) }));
        cx.spawn_in(window, async move |this, cx| {
            let paths = match rx { Some(rx) => rx.await.ok().and_then(Result::ok).flatten(), None => picked };
            let loaded = if let Some(path) = paths.and_then(|paths| paths.into_iter().next()) {
                Some(cx.background_executor().spawn(async move { load_drawing_image(&path) }).await)
            } else { None };
            let _ = this.update_in(cx, |d, window, cx| {
                d.busy = false;
                if let Some(result) = loaded {
                    match result {
                        Ok(file) => d.pending_image = Some(file),
                        Err(e) => { log::warn!("insert drawing image: {e:#}"); d.error = Some(tr("无法插入此图片", "Could not insert this image")); }
                    }
                }
                if d.pending_image.is_none() { d.tool = Tool::Select; }
                window.focus(&d.focus);
                cx.notify();
            });
        }).detach();
    }

    fn place_image(&mut self, point: World, cx: &mut Context<Self>) {
        let Some((file, w, h)) = self.pending_image.take() else { return };
        let pixels: u64 = self.images.values().map(|img| {
            let size = img.size(0); size.width.0 as u64 * size.height.0 as u64
        }).sum();
        if self.doc.scene.elements.len() >= MAX_ELEMENTS || self.doc.files.len() >= drawing_scene::MAX_IMAGES
            || pixels + u64::from(w) * u64::from(h) > drawing_scene::MAX_IMAGE_PIXELS {
            self.error = Some(tr("画板已达到图片容量上限", "Drawing image limit reached"));
            return cx.notify();
        }
        let factor = (480. / f64::from(w.max(h))).min(1.);
        let mut el = factory::new_image(point.x, point.y, f64::from(w) * factor, f64::from(h) * factor, Some(file.id.clone()), &self.style);
        if let Element::Image(image) = &mut el { image.status = scene::element::ImageStatus::Saved; }
        let id = file.id.clone();
        let index = self.doc.scene.elements.len();
        let mut next = self.doc.clone();
        next.scene.elements.push(el);
        next.files.insert(id.clone(), file);
        if next.json().is_err() {
            self.error = Some(tr("画板已达到图片容量上限", "Drawing image limit reached"));
            return cx.notify();
        }
        self.checkpoint();
        self.doc = next;
        if let Some(image) = Self::decode_image(&self.doc.files[&id]) { Rc::make_mut(&mut self.images).insert(id, image); }
        self.after_drawing(index);
        self.rebuild();
        cx.notify();
    }

    fn tool(&mut self, tool: Tool, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_field(window, cx);
        self.panel = (self.panel == Some(Panel::Properties)).then_some(Panel::Properties);
        self.colors = None;
        self.tool = tool;
        if tool != Tool::Image { self.pending_image = None; } else { self.pick_image(window, cx); }
        self.selected = None;
        self.gesture = None;
        window.focus(&self.focus);
        cx.notify();
    }

    fn key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) { return; }
        let key = e.keystroke.key.as_str();
        let m = e.keystroke.modifiers;
        cx.stop_propagation();
        if self.busy { return; }
        if self.reset_confirmation {
            if key == "escape" { self.reset_confirmation = false; cx.notify(); }
            return;
        }
        if self.discard {
            if key == "escape" { self.keep_editing(window, cx); }
            return;
        }
        if m.secondary() && key == "d" { self.duplicate(cx); } else if !m.secondary() {
            match key {
                "escape" if self.gesture.take().is_some() => cx.notify(),
                "escape" if self.zen => { self.zen = false; cx.notify(); }
                "escape" if self.panel.take().is_some() => cx.notify(),
                "escape" if self.tool != Tool::Select => self.tool(Tool::Select, window, cx),
                "escape" if self.selected.take().is_some() => cx.notify(),
                "enter" if self.selected.is_some() => self.edit_selection_text(window, cx),
                "q" => { self.tool_locked = !self.tool_locked; cx.notify(); }
                "escape" => self.cancel(window, cx),
                "delete" | "backspace" if self.gesture.is_none() => self.delete(cx),
                "0" => { self.fit = true; cx.notify(); }
                _ => if let Some(tool) = "123456789".find(key).filter(|_| key.len() == 1)
                    .map(|i| TOOLS[i + 1].0).or_else(|| TOOLS.iter().find_map(|(tool, _, _, k, _)| (*k == key).then_some(*tool))) {
                    self.tool(tool, window, cx);
                }
            }
        }
    }
}

fn can_resize(e: &Element) -> bool {
    e.base().angle.abs() < 0.001 && matches!(e, Element::Rectangle { .. } | Element::Ellipse { .. } | Element::Diamond { .. } | Element::StickyNote(_) | Element::Frame(_) | Element::Image(_))
}

fn can_rotate(e: &Element) -> bool {
    matches!(e, Element::Rectangle { .. } | Element::Ellipse { .. } | Element::Diamond { .. } | Element::StickyNote(_))
}

fn element_center(e: &Element) -> World {
    let b = e.base(); World::new(b.x + b.width / 2., b.y + b.height / 2.)
}

fn rotation_handle(e: &Element, zoom: f64) -> World {
    let b = element_bounds(e);
    World::new(b.center().x, b.min_y - 17. / zoom)
}

fn rect_corners(b: scene::bounds::Bounds) -> [World; 4] {
    [World::new(b.min_x, b.min_y), World::new(b.max_x, b.min_y), World::new(b.max_x, b.max_y), World::new(b.min_x, b.max_y)]
}

fn resize_corner(b: scene::bounds::Bounds, corner: usize, pointer: World, square: bool) -> scene::bounds::Bounds {
    let anchor = rect_corners(b)[(corner + 2) % 4];
    let size = pointer - anchor;
    let sx = if corner == 0 || corner == 3 { -1. } else { 1. };
    let sy = if corner < 2 { -1. } else { 1. };
    let mut w = (size.x * sx).max(1.);
    let mut h = (size.y * sy).max(1.);
    if square {
        let ratio = b.width().max(1.) / b.height().max(1.);
        if w > h * ratio { h = w / ratio; } else { w = h * ratio; }
    }
    let p = World::new((anchor.x + w * sx).clamp(-MAX_COORD, MAX_COORD), (anchor.y + h * sy).clamp(-MAX_COORD, MAX_COORD));
    scene::bounds::Bounds::from_points(&[anchor, p])
}

fn apply_rect(e: &mut Element, rect: scene::bounds::Bounds) {
    let b = e.base_mut();
    (b.x, b.y, b.width, b.height) = (rect.min_x, rect.min_y, rect.width().max(1.), rect.height().max(1.));
    if let Element::StickyNote(note) = e { note.base_height = note.base.height; }
}

fn roundness(tool: Tool) -> Roundness {
    Roundness { kind: if tool == Tool::Rect || tool == Tool::Diamond { RoundnessType::AdaptiveRadius } else { RoundnessType::ProportionalRadius }, value: None }
}

fn hit_at(doc: &Scene, p: World, zoom: f64, exclude: &[usize]) -> Option<usize> {
    doc.elements.iter().enumerate().rev().find(|(i, e)| {
        if e.is_deleted() || e.base().locked || exclude.contains(i) { return false; }
        let at = if matches!(e, Element::Line(_) | Element::Arrow(_) | Element::Freedraw(_)) { p.rotate(-e.base().angle, element_center(e)) } else { p };
        scene::collision::hit_test_element(e, at, 7. / zoom)
    }).map(|(i, _)| i)
}

fn load_drawing_image(path: &std::path::Path) -> anyhow::Result<(scene::scene::BinaryFileData, u32, u32)> {
    use base64::Engine as _;
    let bytes = store::read_bounded(path, MAX_SOURCE * 3 / 4)?;
    let format = image::guess_format(&bytes)?;
    anyhow::ensure!(matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::Gif | image::ImageFormat::WebP), "unsupported drawing image format");
    let (w, h) = image::ImageReader::with_format(std::io::Cursor::new(&bytes), format).into_dimensions()?;
    anyhow::ensure!(w > 0 && h > 0 && w <= 4096 && h <= 4096 && u64::from(w) * u64::from(h) <= drawing_scene::MAX_IMAGE_PIXELS, "drawing image exceeds the pixel limit");
    let mime = format.to_mime_type();
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok((scene::scene::BinaryFileData::new(factory::new_id(), mime, format!("data:{mime};base64,{data}")), w, h))
}

fn drawable_ink(c: Ink, opacity: f64, dark: bool) -> Hsla {
    let c = if dark {
        let (r, g, b) = quiet_excalidraw::render::export::dark_mode_color((c.r * 255.).round() as u8, (c.g * 255.).round() as u8, (c.b * 255.).round() as u8);
        Ink::new(f64::from(r) / 255., f64::from(g) / 255., f64::from(b) / 255., c.a)
    } else { c };
    hsla_from_ink(c, opacity)
}

fn paint_grid(bounds: Bounds<Pixels>, origin: Point<Pixels>, zoom: f64, window: &mut Window) {
    let step = (20. * zoom as f32).max(8.);
    let x = bounds.origin.x + px(f32::from(origin.x - bounds.origin.x).rem_euclid(step));
    let y = bounds.origin.y + px(f32::from(origin.y - bounds.origin.y).rem_euclid(step));
    let mut grid = PathBuilder::stroke(px(1.));
    for i in 0..=(f32::from(bounds.size.width) / step) as usize {
        let x = x + px(i as f32 * step);
        grid.move_to(point(x, bounds.origin.y)); grid.line_to(point(x, bounds.bottom()));
    }
    for i in 0..=(f32::from(bounds.size.height) / step) as usize {
        let y = y + px(i as f32 * step);
        grid.move_to(point(bounds.origin.x, y)); grid.line_to(point(bounds.right(), y));
    }
    if let Ok(path) = grid.build() { window.paint_path(path, Theme::text_caption().opacity(0.16)); }
}

fn paint_selection(bounds: Bounds<Pixels>, resize: bool, rotate: Option<Point<Pixels>>, paper: Hsla, window: &mut Window) {
    let ink = Theme::text_caption();
    window.paint_quad(outline(bounds, ink, BorderStyle::Solid));
    let mut handle = |center, radius| {
        let extents = Bounds::new(center - point(px(4.), px(4.)), size(px(8.), px(8.)));
        window.paint_quad(quad(extents, px(radius), paper, Edges::all(px(1.5)), ink, BorderStyle::Solid));
    };
    if resize {
        for p in [bounds.origin, bounds.top_right(), bounds.bottom_right(), bounds.bottom_left()] { handle(p, 2.); }
    }
    if let Some(p) = rotate { handle(p, 4.); }
}

fn arrow_binding(scene: &Scene, p: World) -> Option<scene::element::FixedPointBinding> {
    scene::binding::find_bindable(scene, p, scene::binding::BINDING_THRESHOLD, &[]).and_then(|id| scene.get(&id)).map(|e| scene::binding::make_binding(e, p))
}

fn touch(element: &mut Element) {
    let e = element.base_mut();
    e.version = e.version.saturating_add(1);
    e.version_nonce = i64::from(uuid::Uuid::new_v4().as_u128() as u32 & 0x7fff_ffff);
    e.updated = (crate::time::now() * 1000) as i64;
}

impl Render for DrawingEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let transform = match self.gesture {
            Some(Gesture::Resize { rect, index, .. }) => { let mut e = self.doc.scene.elements[index].clone(); apply_rect(&mut e, rect); Some((index, e)) }
            Some(Gesture::Rotate { angle, index, .. }) => { let mut e = self.doc.scene.elements[index].clone(); e.base_mut().angle = angle; Some((index, e)) }
            _ => None,
        };
        let handles = self.selected.map(|i| {
            let e = transform.as_ref().filter(|(index, _)| *index == i).map_or(&self.doc.scene.elements[i], |(_, e)| e);
            (can_resize(e), can_rotate(e), rotation_handle(e, self.zoom))
        });
        let erasing = match &self.gesture { Some(Gesture::Erase { indices, .. }) => indices.clone(), _ => Vec::new() };
        let viewport = self.viewport.clone();
        let art = self.artwork.clone();
        let images = self.images.clone();
        let moving = match self.gesture { Some(Gesture::Move { index, delta, .. }) => Some((index, delta)), _ => None };
        let pending = match &self.gesture {
            Some(Gesture::Shape { start, end, ink, .. }) => self.shape(*start, *end, ink).map(|el| render_element(&el)),
            _ => None,
        };
        let geometry: Vec<_> = self.doc.scene.elements.iter().enumerate().map(|(i, e)| {
            let e = transform.as_ref().filter(|(index, _)| *index == i).map_or(e, |(_, e)| e);
            let b = e.base();
            (element_bounds(e), World::new(b.x + b.width / 2., b.y + b.height / 2.), b.angle)
        }).collect();
        let transformed = transform.map(|(i, e)| (i, render_element(&e)));
        let fit = self.fit;
        let weak = cx.weak_entity();
        let offset = self.offset;
        let zoom = self.zoom;
        let selection = self.selected;
        let dark = self.dark_canvas;
        let grid = self.grid;
        let paper = drawable_ink(Ink::from_hex(if self.doc.background == "transparent" { "#ffffff" } else { &self.doc.background }), 1., dark);
        let painting = canvas(move |bounds, window, _| {
            viewport.set(bounds);
            if fit { window.on_next_frame(move |_, cx| { let _ = weak.update(cx, |d, cx| { d.fit_view(); cx.notify(); }); }); }
        }, move |bounds, (), window, cx| {
            let origin = bounds.origin + offset;
            if grid { paint_grid(bounds, origin, zoom, window); }
            for (i, (drawables, (extent, pivot, angle))) in art.iter().zip(geometry).enumerate() {
                if drawables.is_empty() || erasing.contains(&i) { continue; }
                let delta = moving.filter(|(index, _)| *index == i).map_or(World::zero(), |(_, delta)| delta);
                let screen = |p: World| origin + point(px((p.x * zoom) as f32), px((p.y * zoom) as f32));
                let box_bounds = Bounds::from_corners(screen(World::new(extent.min_x + delta.x, extent.min_y + delta.y)), screen(World::new(extent.max_x + delta.x, extent.max_y + delta.y)));
                if !bounds.intersects(&box_bounds.dilate(px(96.))) { continue; }
                let drawables = transformed.as_ref().filter(|(index, _)| *index == i).map_or(drawables, |(_, art)| art);
                for d in drawables { paint(d, |p| screen(p.rotate(angle, pivot) + delta), zoom, dark, &images, window, cx); }
                if selection == Some(i) && let Some((resize, rotate, knob)) = handles {
                    paint_selection(box_bounds, resize, rotate.then(|| screen(knob + delta)), paper, window);
                }
            }
            if let Some(pending) = pending {
                for d in &pending { paint(d, |p| origin + point(px((p.x * zoom) as f32), px((p.y * zoom) as f32)), zoom, dark, &images, window, cx); }
            }
        }).size_full();
        let mut card = div().id("drawing-dialog").size_full().occlude().bg(Theme::bg_body())
            .relative().flex().flex_col().overflow_hidden().track_focus(&self.focus).key_context("DrawingEditor")
            .on_action(cx.listener(|d, _: &SaveDrawing, w, cx| { if !d.discard && !d.reset_confirmation { d.save(w, cx); } }))
            .on_action(cx.listener(|d, _: &UndoDrawing, _, cx| { if !d.busy && !d.discard && !d.reset_confirmation { d.history(false, cx); } }))
            .on_action(cx.listener(|d, _: &RedoDrawing, _, cx| { if !d.busy && !d.discard && !d.reset_confirmation { d.history(true, cx); } }))
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(self.header(cx))
            .child(div().relative().flex_1().min_h_0().w_full()
            .child(div().id("drawing-surface").absolute().inset_0().overflow_hidden().bg(paper)
                .cursor(match self.tool { Tool::Select => CursorStyle::Arrow, Tool::Pan => CursorStyle::OpenHand, Tool::Text => CursorStyle::IBeam, _ => CursorStyle::Crosshair })
                .on_mouse_down(MouseButton::Left, cx.listener(Self::down)).on_mouse_down(MouseButton::Middle, cx.listener(Self::down))
                .on_mouse_move(cx.listener(|d, e: &MouseMoveEvent, _, cx| {
                    if e.pressed_button.is_none() { d.up(cx); } else { d.moved(e, cx); }
                }))
                .on_mouse_up(MouseButton::Left, cx.listener(|d, _, _, cx| d.up(cx)))
                .on_mouse_up_out(MouseButton::Left, cx.listener(|d, _, _, cx| d.up(cx)))
                .on_mouse_up(MouseButton::Middle, cx.listener(|d, _, _, cx| d.up(cx)))
                .on_mouse_up_out(MouseButton::Middle, cx.listener(|d, _, _, cx| d.up(cx)))
                .on_scroll_wheel(cx.listener(|d, e: &ScrollWheelEvent, _, cx| {
                    if d.gesture.is_none() && !d.busy && !d.discard {
                        cx.stop_propagation();
                        let delta = e.delta.pixel_delta(px(25.));
                        if e.modifiers.secondary() { d.zoom_at(e.position, (f64::from(f32::from(delta.y)) / 340.).exp()); }
                        else { d.offset += delta; }
                        cx.notify();
                    }
                }))
                .child(painting))
            .children(self.canvas_chrome(window, cx)));
        if self.discard || self.reset_confirmation { card = card.child(self.confirmation(cx)); }
        card
    }
}

fn hsla_from_ink(c: Ink, opacity: f64) -> Hsla { gpui::Rgba { r: c.r as f32, g: c.g as f32, b: c.b as f32, a: (c.a * opacity).clamp(0., 1.) as f32 }.into() }

fn paint(d: &Drawable, map: impl Fn(World) -> Point<Pixels>, zoom: f64, dark: bool, images: &HashMap<String, Arc<RenderImage>>, window: &mut Window, cx: &mut App) {
    let p = |x, y| map(World::new(x, y));
    let alpha = d.options.opacity / 100.;
    let stroke = drawable_ink(d.options.stroke_color, alpha, dark);
    if let Some(t) = &d.text {
        let font_size = px((t.font_size * zoom) as f32);
        let line_height = t.font_size * t.line_height;
        let h = line_height * t.text.lines().count() as f64;
        let top = match t.vertical_align { VerticalAlign::Top => t.y, VerticalAlign::Middle => t.y + (t.height - h) / 2., VerticalAlign::Bottom => t.y + t.height - h };
        let color = drawable_ink(t.color, alpha, dark);
        for (i, text) in t.text.lines().enumerate() {
            if text.is_empty() { continue; }
            let runs = [TextRun { len: text.len(), font: font(theme::UI_FONT), color, background_color: None, underline: None, strikethrough: None }];
            let line = window.text_system().shape_line(text.to_string().into(), font_size, &runs, None);
            let mut at = p(t.x, top + i as f64 * line_height);
            let width = px((t.width * zoom) as f32);
            at.x += match t.text_align { DrawingAlign::Left => px(0.), DrawingAlign::Center => (width - line.width) / 2., DrawingAlign::Right => width - line.width };
            let _ = line.paint(at, px((line_height * zoom) as f32), window, cx);
        }
        return;
    }
    if let Some(img) = &d.image {
        let bounds = Bounds::new(p(img.x, img.y), size(px((img.width * zoom) as f32), px((img.height * zoom) as f32)));
        if let Some(data) = img.file_id.as_ref().and_then(|id| images.get(id)) {
            let _ = window.with_element_opacity(Some(alpha.clamp(0., 1.) as f32), |window| window.paint_image(bounds, Corners::default(), data.clone(), 0, false));
        } else { window.paint_quad(fill(bounds, Theme::bg_overlay())); }
        return;
    }
    let mut path = |ops: &mut dyn Iterator<Item = &Op>, mut builder: PathBuilder, color| {
        for op in ops {
            match *op {
                Op::MoveTo(x, y) => builder.move_to(p(x, y)),
                Op::LineTo(x, y) => builder.line_to(p(x, y)),
                Op::QuadraticTo(cx, cy, x, y) => builder.curve_to(p(x, y), p(cx, cy)),
                Op::CubicTo(ax, ay, bx, by, x, y) => builder.cubic_bezier_to(p(x, y), p(ax, ay), p(bx, by)),
                Op::Close => builder.close(),
            }
        }
        if let Ok(path) = builder.build() { window.paint_path(path, color); }
    };
    if let Some(ink) = &d.ink { path(&mut ink.iter(), PathBuilder::fill(), stroke); return; }
    if d.options.fill_style == scene::types::FillStyle::Solid && let (Some(ops), Some(color)) = (&d.fill_path, d.options.fill_color) {
        path(&mut ops.iter(), PathBuilder::fill(), drawable_ink(color, alpha, dark));
    }
    let width = px((d.options.stroke_width * zoom) as f32);
    if let Some((outline, hatching)) = d.sets.split_first() {
        if !hatching.is_empty() {
            let color = d.options.fill_color.map_or(stroke, |c| drawable_ink(c, alpha, dark));
            path(&mut hatching.iter().flatten(), PathBuilder::stroke(width), color);
        }
        if !outline.is_empty() { path(&mut outline.iter(), PathBuilder::stroke(width), stroke); }
    }
}

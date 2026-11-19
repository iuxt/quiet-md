//! GPUI canvas for Excalidraw blocks. Draft changes stay local until Save.
#[path = "drawing_ui.rs"]
mod chrome;
#[path = "drawing_edit.rs"]
mod edit;
#[path = "drawing_commands.rs"]
mod commands;
use commands::Property;
use edit::{AlignSelection, selection_indices, selection_bounds, transform_selection};

use super::{drawing_scene::{self, DrawingScene, MAX_COORD, MAX_ELEMENTS, MAX_POINTS, MAX_SOURCE}, render::Env, view::{Editor, EditorEvent}};
use crate::{input::{TextField, TextFieldEvent}, model::{BlockId, BlockKind, Drawing}, settings::tr, store::{self, Store}, theme::{self, Theme}, ui};
use gpui::{prelude::FluentBuilder, *};
use quiet_excalidraw::core::{self as scene, arrowhead::Arrowhead, bounds::element_bounds, element::{Element, TextElement}, factory::{self, ElementOptions}, geometry::Point as World, scene::Scene, types::{FillStyle, FontFamily, Roundness, RoundnessType, StrokeStyle, TextAlign as DrawingAlign, VerticalAlign}};
use quiet_excalidraw::render::shape::{Drawable, Op, Rgba as Ink, render_element, render_element_with_lookup};
actions!(drawing, [SaveDrawing, UndoDrawing, RedoDrawing, CopyDrawing, CutDrawing, PasteDrawing, SelectDrawing, DuplicateDrawing,
    GroupDrawing, UngroupDrawing, EditDrawingPoints, LinkDrawing, LockDrawing, CopyDrawingStyle, PasteDrawingStyle, FindDrawing, OpenDrawing, ExportDrawing, CopyDrawingPng, PasteDrawingText]);

pub(crate) fn load_fonts(cx: &App) {
    let fonts = quiet_excalidraw::fonts::BUNDLED.iter().map(|(_, data)| std::borrow::Cow::Borrowed(*data)).collect();
    if let Err(e) = cx.text_system().add_fonts(fonts) { log::warn!("load drawing fonts: {e:#}"); }
}

pub(crate) fn bind_keys(cx: &mut App) {
    let context = Some("DrawingEditor");
    cx.bind_keys([
        KeyBinding::new("secondary-s", SaveDrawing, context),
        KeyBinding::new("secondary-z", UndoDrawing, context),
        KeyBinding::new("secondary-shift-z", RedoDrawing, context),
        KeyBinding::new("secondary-y", RedoDrawing, context),
        KeyBinding::new("secondary-c", CopyDrawing, context), KeyBinding::new("secondary-x", CutDrawing, context),
        KeyBinding::new("secondary-v", PasteDrawing, context), KeyBinding::new("secondary-a", SelectDrawing, context),
        KeyBinding::new("secondary-d", DuplicateDrawing, context), KeyBinding::new("secondary-g", GroupDrawing, context),
        KeyBinding::new("secondary-shift-g", UngroupDrawing, context), KeyBinding::new("secondary-enter", EditDrawingPoints, context),
        KeyBinding::new("secondary-k", LinkDrawing, context), KeyBinding::new("secondary-shift-l", LockDrawing, context),
        KeyBinding::new("secondary-alt-c", CopyDrawingStyle, context), KeyBinding::new("secondary-alt-v", PasteDrawingStyle, context),
        KeyBinding::new("secondary-f", FindDrawing, context),
        KeyBinding::new("alt-shift-c", CopyDrawingPng, context), KeyBinding::new("secondary-shift-v", PasteDrawingText, context),
        KeyBinding::new("secondary-o", OpenDrawing, context), KeyBinding::new("secondary-shift-e", ExportDrawing, context),
    ]);
}

use std::{cell::{Cell, RefCell}, collections::{HashMap, VecDeque}, rc::Rc, sync::Arc};

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
enum Tool { Pan, Select, Rect, Diamond, Ellipse, Arrow, Line, Pen, Text, Image, Sticky, Eraser, Frame, Laser, Eyedropper, Bucket }

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
    (Tool::Laser, "激光笔", "Laser pointer", "k", "pen-line"),
    (Tool::Eyedropper, "取色", "Eyedropper", "i", "eye"),
    (Tool::Bucket, "填充颜色", "Bucket fill", "b", "drawing-note"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Panel { Menu, Properties, More, Export, Context, Help, Library }

#[derive(Clone)]
struct DrawingState { scene: Scene, background: String }

#[derive(Clone, Copy)]
enum StyleChoice {
    Stroke(&'static str), Fill(&'static str), Width(f64), StrokeStyle(StrokeStyle), FillStyle(FillStyle), Roughness(f64), Round(bool), Opacity(f64),
    TextColor(&'static str), TextFont(FontFamily), TextSize(f64), TextAlign(DrawingAlign), TextVertical(VerticalAlign), ArrowHead(Option<Arrowhead>), StartArrowHead(Option<Arrowhead>), ArrowType(u8),
}

#[derive(Clone, Copy)]
enum LayerMove { Bottom, Back, Forward, Top }

#[derive(Clone, Copy)]
enum ExportKind { Source, Png, Svg }

enum Gesture {
    Laser,
    Crop { pointer: World, corner: usize, start: scene::bounds::Bounds, rect: scene::bounds::Bounds, index: usize },
    Box { start: World, end: World, previous: Vec<usize>, deep: bool },
    Vertex { start: World, delta: World, indices: Vec<usize>, insert: Option<(usize, World)>, index: usize },
    Endpoint { pointer: World, start: World, end: World, endpoint: usize, index: usize },
    Resize { pointer: World, corner: usize, start: scene::bounds::Bounds, rect: scene::bounds::Bounds },
    Rotate { start: f64, angle: f64 },
    Erase { indices: Vec<usize>, last: World },
    Pan { start: Point<Pixels>, offset: Point<Pixels> },
    Move { start: World, delta: World, copy: bool },
    Shape { start: World, end: World, ink: Vec<World>, point_limit: usize, from_center: bool },
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
    edit_frame: Option<usize>,
    searching: bool,
    edit_property: Option<Property>,
    eyedropper: chrome::Colors,
    laser: Vec<(World, std::time::Instant)>,
    objects_snap: bool,
    snap_guides: Vec<(World, World)>,
    view_only: bool,
    tool_locked: bool,
    panel: Option<Panel>,
    grid: bool,
    colors: Option<chrome::Colors>,
    more_fonts: bool,
    zen: bool,
    dark_canvas: bool,
    reset_confirmation: bool,
    style: ElementOptions,
    rounded: bool,
    text_font: FontFamily,
    text_size: f64,
    text_align: DrawingAlign,
    text_color: String,
    arrowhead: Option<Arrowhead>,
    start_arrowhead: Option<Arrowhead>,
    binding_disabled: bool,
    arrow_type: u8,
    opacity_dragging: Option<bool>,
    opacity_bounds: Rc<Cell<Bounds<Pixels>>>,
    pending_image: Option<(scene::scene::BinaryFileData, u32, u32)>,
    tool: Tool,
    selection: Vec<usize>,
    gesture: Option<Gesture>,
    editing_group: Option<String>,
    line_edit: Option<usize>,
    crop: Option<usize>,
    selected_points: Vec<usize>,
    polyline: Vec<World>,
    pointer: World,
    space: bool,
    copied_style: Option<Element>,
    export_selection: bool,
    export_options: drawing_scene::ExportOptions,
    library: Vec<DrawingScene>,
    library_images: RefCell<HashMap<usize, Arc<RenderImage>>>,
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
    raster_artwork: RefCell<HashMap<String, (u64, Arc<RenderImage>)>>,
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
            f.max_len = Some(2048); f
        });
        let field_sub = cx.subscribe_in(&field, window, |d, _, event, window, cx| match event {
            TextFieldEvent::Submit if d.searching => d.find_text(false, cx),
            TextFieldEvent::Changed if d.searching => d.find_text(false, cx),
            TextFieldEvent::ShiftSubmit if d.searching => d.find_text(true, cx),
            TextFieldEvent::Submit => d.finish_field(window, cx),
            TextFieldEvent::Cancel => {
                if d.text_at.is_some() { d.finish_field(window, cx); }
                d.text_at = None; d.edit_link = None; d.edit_frame = None; d.edit_property = None; d.searching = false; window.focus(&d.focus); cx.notify();
            }
            _ => {}
        });
        let saved = DrawingState { scene: doc.scene.clone(), background: doc.background.clone() };
        let mut this = Self { parent, target, doc, focus: cx.focus_handle(), field, _field_sub: field_sub, text_at: None, text_container: None,
            edit_link: None, edit_frame: None, searching: false, edit_property: None, eyedropper: chrome::Colors::Stroke, laser: Vec::new(), objects_snap: false, snap_guides: Vec::new(), view_only: false, tool_locked: false, panel: None, colors: None, more_fonts: false, grid: false, zen: false, dark_canvas: false, reset_confirmation: false,
            text_color: ElementOptions::default().stroke_color, style: ElementOptions::default(), rounded: true,
            text_font: FontFamily::Excalifont, text_size: 20., text_align: DrawingAlign::Left, arrowhead: Some(Arrowhead::Arrow), start_arrowhead: None, binding_disabled: false, arrow_type: 1,
            opacity_dragging: None, opacity_bounds: Rc::default(), pending_image: None,
            tool: Tool::Select, selection: Vec::new(), gesture: None, editing_group: None, line_edit: None, crop: None, selected_points: Vec::new(), polyline: Vec::new(), pointer: World::zero(), space: false, copied_style: None, export_selection: false, export_options: drawing_scene::ExportOptions::default(), library_images: RefCell::default(), library: store::read_bounded(&Store::internal_dir().join("drawing-library.excalidrawlib"), MAX_SOURCE).ok().and_then(|bytes| DrawingScene::load_library(&bytes).ok()).unwrap_or_default(), viewport: Rc::new(Cell::new(Bounds::default())), offset: point(px(0.), px(0.)), zoom: 1., fit: true,
            undo: VecDeque::new(), redo: Vec::new(), saved, imported: false, artwork: Rc::default(), images: Rc::default(), raster_artwork: RefCell::default(), busy: false, close_after_save: false, saving: false, discard: false, error: None };
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
        if self.images.is_empty() && self.raster_artwork.borrow().is_empty() && self.library_images.borrow().is_empty() { return; }
        // The atlas owns a separate copy. Redraw before presenting a scene that used these tiles.
        window.refresh();
        for image in self.images.values() { let _ = window.drop_image(image.clone()); }
        for (_, image) in self.raster_artwork.borrow().values() { let _ = window.drop_image(image.clone()); }
        for image in self.library_images.borrow().values() { let _ = window.drop_image(image.clone()); }
    }

    fn load_images(&mut self, window: &mut Window) {
        self.drop_images(window); self.raster_artwork.borrow_mut().clear();
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
        if !self.polyline.is_empty() { if !redo { self.polyline.pop(); cx.notify(); } return; }
        self.line_edit = None; self.crop = None; self.selected_points.clear(); self.editing_group = None;
        if self.gesture.is_some() || self.text_at.is_some() || self.edit_link.is_some() || self.edit_property.is_some() || self.edit_frame.is_some() { return; }
        if redo {
            if let Some(state) = self.redo.pop() { let current = self.swap_state(state); self.undo.push_back(current); }
        } else if let Some(state) = self.undo.pop_back() { let current = self.swap_state(state); self.redo.push(current); }
        self.selection.clear();
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
        if !self.dirty() && self.text_at.is_none() && self.edit_link.is_none() && self.edit_frame.is_none() && self.edit_property.is_none() && self.polyline.is_empty() && self.gesture.is_none() { return false; }
        self.discard = true;
        self.gesture = None;
        cx.notify();
        true
    }

    fn keep_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.discard = false;
        let _ = self.parent.update(cx, |_, cx| cx.emit(EditorEvent::DrawingExitCanceled));
        let focus = if self.text_at.is_some() || self.edit_link.is_some() || self.edit_frame.is_some() || self.edit_property.is_some() { self.field.read(cx).focus_handle(cx) } else { self.focus.clone() };
        window.focus(&focus);
        cx.notify();
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy { return; }
        if !self.confirm_exit(cx) { self.close(window, cx); }
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.gesture.is_some() { return; }
        self.finish_field(window, cx); self.finish_polyline(cx);
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
            prompt: Some(tr("导入 Excalidraw", "Import Excalidraw").into()), extensions: Some(["excalidraw", "png", "svg"].map(Into::into).to_vec()) }));
        cx.spawn_in(window, async move |this, cx| {
            let paths = match rx { Some(rx) => rx.await.ok().and_then(Result::ok).flatten(), None => picked };
            let doc = if let Some(path) = paths.and_then(|p| p.into_iter().next()) {
                Some(cx.background_executor().spawn(async move { DrawingScene::load_exported(&store::read_bounded(&path, MAX_SOURCE * 4)?) }).await)
            } else { None };
            let _ = this.update_in(cx, |d, window, cx| {
                d.busy = false;
                if let Some(result) = doc {
                    match result {
                        Ok(doc) => { d.pending_image = None; d.doc = doc; d.imported = true; d.undo.clear(); d.redo.clear(); d.selection.clear(); d.text_at = None; d.edit_link = None; d.edit_frame = None; d.edit_property = None; d.line_edit = None; d.crop = None; d.selected_points.clear(); d.editing_group = None; d.polyline.clear(); d.fit = true; d.load_images(window); d.rebuild(); }
                        Err(e) => { log::warn!("import drawing: {e:#}"); d.error = Some(tr("无法导入此 Excalidraw 文件", "Could not import this Excalidraw file")); }
                    }
                }
                cx.notify();
            });
        }).detach();
    }

    fn export(&mut self, kind: ExportKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy { return; }
        self.finish_field(window, cx); self.finish_polyline(cx);
        let name = match kind { ExportKind::Source => "Drawing.excalidraw", ExportKind::Png => "Drawing.png", ExportKind::Svg => "Drawing.svg" };
        let path = crate::devtools::take_pick().and_then(|p| p.into_iter().next());
        let dir = dirs::download_dir().or_else(dirs::document_dir).unwrap_or_default();
        let rx = path.is_none().then(|| cx.prompt_for_new_path(&dir, Some(name)));
        let doc = if self.export_selection && !self.selection.is_empty() { self.selection_document() } else { self.doc.clone() };
        let options = self.export_options;
        cx.spawn_in(window, async move |this, cx| {
            let picked = match rx { Some(rx) => rx.await.ok().and_then(Result::ok).flatten(), None => path };
            let Some(path) = picked else { return };
            let result = cx.background_executor().spawn(async move {
                let bytes = match kind {
                    ExportKind::Source => doc.json()?, ExportKind::Png => doc.export_png(options)?,
                    ExportKind::Svg => doc.export_svg(options)?.into_bytes(),
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

    fn selected(&self) -> Option<usize> { (self.selection.len() == 1).then(|| self.selection[0]) }

    fn select_at(&self, p: World) -> Option<usize> {
        hit_at(&self.doc.scene, p, self.zoom, &[])
    }

    fn bound_text_index(&self, shape: &Element) -> Option<usize> {
        self.doc.scene.elements.iter().position(|e| matches!(e, Element::Text(t)
            if !t.base.is_deleted && t.container_id.as_deref() == Some(shape.id())))
    }

    fn down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.discard || self.reset_confirmation { return; }
        if self.fit { self.fit_view(); }
        self.finish_field(window, cx); window.focus(&self.focus); cx.stop_propagation();
        let p = self.world(event.position); self.pointer = p; self.binding_disabled = event.modifiers.secondary();
        if event.button == MouseButton::Middle || self.tool == Tool::Pan || self.space || self.view_only {
            self.gesture = Some(Gesture::Pan { start: event.position, offset: self.offset });
        } else if self.tool == Tool::Laser {
            self.laser.push((p, std::time::Instant::now())); self.gesture = Some(Gesture::Laser);
        } else if self.tool == Tool::Eyedropper {
            self.pick_color(p, cx);
        } else if self.tool == Tool::Bucket {
            self.fill_at(p, cx);
        } else if self.tool == Tool::Select {
            if let Some(index) = self.crop {
                let e = &self.doc.scene.elements[index]; let b = e.base();
                let rect = scene::bounds::Bounds::new(b.x, b.y, b.x + b.width, b.y + b.height);
                let local = p.rotate(-b.angle, element_center(e));
                if let Some(corner) = rect_corners(rect).iter().position(|q| local.distance(*q) <= 9. / self.zoom) {
                    self.gesture = Some(Gesture::Crop { pointer: local, corner, start: rect, rect, index }); cx.notify(); return;
                }
                self.crop = None;
            }
            if let Some(index) = self.line_edit {
                let e = &self.doc.scene.elements[index];
                if let Some(points) = linear_points(e) {
                    let points: Vec<_> = points.iter().map(|q| q.rotate(e.base().angle, element_center(e))).collect();
                    let elbow = matches!(e, Element::Arrow(a) if a.elbowed);
                    if let Some(point) = points.iter().enumerate().position(|(n, q)| (!elbow || n == 0 || n + 1 == points.len()) && p.distance(*q) <= 9. / self.zoom) {
                        if event.modifiers.shift {
                            if self.selected_points.contains(&point) { self.selected_points.retain(|i| *i != point); } else { self.selected_points.push(point); }
                        } else if !self.selected_points.contains(&point) { self.selected_points = vec![point]; }
                        self.gesture = Some(Gesture::Vertex { start: p, delta: World::zero(), indices: self.selected_points.clone(), insert: None, index });
                        cx.notify(); return;
                    }
                    if points.len() < MAX_POINTS && self.doc.remaining_points() > 0 {
                        let segment = points.windows(2).enumerate().find(|(n, pair)| (!elbow || *n > 0 && *n + 2 < points.len()) && p.distance(pair[0].lerp(pair[1], 0.5)) <= 9. / self.zoom);
                        let segment = segment.or_else(|| (!elbow && event.modifiers.alt).then(|| points.windows(2).enumerate().min_by(|(_, a), (_, b)| {
                            scene::collision::distance_to_segment(p, a[0], a[1]).total_cmp(&scene::collision::distance_to_segment(p, b[0], b[1]))
                        })).flatten());
                        if let Some((n, pair)) = segment {
                            let at = if event.modifiers.alt { p } else { pair[0].lerp(pair[1], 0.5) };
                            self.selected_points = if elbow { vec![n, n + 1] } else { vec![n + 1] };
                            self.gesture = Some(Gesture::Vertex { start: p, delta: World::zero(), indices: self.selected_points.clone(), insert: (!elbow).then_some((n + 1, at)), index });
                            cx.notify(); return;
                        }
                    }
                }
                self.line_edit = None; self.selected_points.clear();
            }
            if !event.modifiers.shift && let Some(index) = self.selected() {
                let e = &self.doc.scene.elements[index];
                if let Some((endpoint, start)) = endpoint_at(e, p, self.zoom) {
                    self.gesture = Some(Gesture::Endpoint { pointer: p, start, end: start, endpoint, index });
                    cx.notify(); return;
                }
            }
            if !event.modifiers.shift && let Some(b) = selection_bounds(&self.doc.scene, &self.selection) {
                let resizable = self.selection.len() > 1 || self.selected().is_some_and(|i| can_resize(&self.doc.scene.elements[i]));
                if resizable && let Some(corner) = rect_corners(b).iter().position(|q| p.distance(*q) <= 9. / self.zoom) {
                    self.gesture = Some(Gesture::Resize { pointer: p, corner, start: b, rect: b }); cx.notify(); return;
                }
                let knob = World::new(b.center().x, b.min_y - 17. / self.zoom);
                let rotatable = self.selection.len() > 1 || self.selected().is_some_and(|i| can_rotate(&self.doc.scene.elements[i]));
                if rotatable && p.distance(knob) <= 9. / self.zoom {
                    self.gesture = Some(Gesture::Rotate { start: (p.y - b.center().y).atan2(p.x - b.center().x), angle: 0. }); cx.notify(); return;
                }
            }
            let hit = self.select_at(p);
            if let Some(i) = hit {
                if event.click_count == 2 && !self.doc.scene.elements[i].base().group_ids.is_empty() {
                    let groups = &self.doc.scene.elements[i].base().group_ids;
                    self.editing_group = self.editing_group.as_ref().and_then(|g| groups.iter().position(|id| id == g)).and_then(|n| n.checked_sub(1))
                        .and_then(|n| groups.get(n)).or_else(|| groups.last()).cloned();
                }
                let roots = if event.modifiers.secondary() { vec![i] } else { self.expand_selection(&[i]) };
                if event.modifiers.shift {
                    if roots.iter().all(|i| self.selection.contains(i)) { self.selection.retain(|i| !roots.contains(i)); }
                    else { self.selection.extend(roots); self.selection.sort_unstable(); self.selection.dedup(); }
                } else if event.modifiers.secondary() || !roots.iter().all(|i| self.selection.contains(i)) { self.selection = roots; }
                if self.panel.is_none() { self.panel = Some(Panel::Properties); }
                if event.click_count == 2 && self.selected().is_some() {
                    let i = self.selected().unwrap();
                    if matches!(self.doc.scene.elements[i], Element::Image(_)) { self.crop = Some(i); }
                    else if matches!(self.doc.scene.elements[i], Element::Line(_)) || event.modifiers.secondary() && linear_points(&self.doc.scene.elements[i]).is_some() {
                        self.line_edit = Some(i); self.selected_points.clear();
                    } else { self.edit_selection_text(window, cx); }
                } else if !self.selection.is_empty() && !event.modifiers.shift {
                    self.gesture = Some(Gesture::Move { start: p, delta: World::zero(), copy: event.modifiers.alt });
                }
            } else if event.click_count == 2 {
                self.selection.clear(); self.start_text(p, None, String::new(), window, cx);
            } else {
                let previous = if event.modifiers.shift { self.selection.clone() } else { Vec::new() };
                self.selection.clone_from(&previous);
                if !event.modifiers.shift { self.editing_group = None; }
                self.gesture = Some(Gesture::Box { start: p, end: p, previous, deep: event.modifiers.secondary() });
            }
        } else if self.tool == Tool::Text {
            self.start_text(p, None, String::new(), window, cx);
        } else if self.tool == Tool::Image {
            if self.pending_image.is_some() { self.place_image(p, cx); } else { self.pick_image(window, cx); }
        } else if self.tool == Tool::Eraser {
            self.gesture = Some(Gesture::Erase { indices: self.select_at(p).map(|i| self.expand_selection(&[i])).unwrap_or_default(), last: p });
        } else if !self.polyline.is_empty() {
            if event.click_count == 2 || self.polyline.last().is_some_and(|q| p.distance(*q) <= 9. / self.zoom) { self.finish_polyline(cx); }
            else if self.polyline.len() < self.doc.remaining_points().min(MAX_POINTS) { self.polyline.push(self.snap(p, event.modifiers.secondary())); }
        } else if self.doc.scene.elements.len() < MAX_ELEMENTS {
            self.selection.clear();
            let p = self.snap(p, event.modifiers.secondary());
            let point_limit = self.doc.remaining_points().min(MAX_POINTS);
            if point_limit >= 2 || !matches!(self.tool, Tool::Pen | Tool::Line | Tool::Arrow) {
                self.gesture = Some(Gesture::Shape { start: p, end: p, ink: vec![p], point_limit, from_center: event.modifiers.alt });
            }
        }
        cx.notify();
    }

    fn snap(&self, p: World, disabled: bool) -> World {
        if self.grid && !disabled { World::new(scene::binding::snap_to_grid(p.x, 20.), scene::binding::snap_to_grid(p.y, 20.)) } else { p }
    }

    fn moved(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let p = self.world(event.position); self.pointer = p;
        self.binding_disabled = event.modifiers.secondary();
        let snapped = self.snap(p, event.modifiers.secondary());
        match &mut self.gesture {
            Some(Gesture::Laser) => {
                if self.laser.last().is_none_or(|(q, _)| q.distance(p) * self.zoom >= 2.) { self.laser.push((p, std::time::Instant::now())); }
                if self.laser.len() > 1000 { self.laser.remove(0); }
            }
            Some(Gesture::Crop { pointer, corner, start, rect, index }) => {
                let e = &self.doc.scene.elements[*index]; let local = p.rotate(-e.base().angle, element_center(e));
                let q = rect_corners(*start)[*corner] + local - *pointer;
                *rect = resize_corner(*start, *corner, q, false);
                rect.min_x = rect.min_x.max(start.min_x); rect.min_y = rect.min_y.max(start.min_y);
                rect.max_x = rect.max_x.min(start.max_x); rect.max_y = rect.max_y.min(start.max_y);
            }
            Some(Gesture::Box { end, .. }) => *end = p,
            Some(Gesture::Pan { start, offset }) => self.offset = *offset + event.position - *start,
            Some(Gesture::Move { start, delta, .. }) => {
                *delta = p - *start;
                if event.modifiers.shift { if delta.x.abs() > delta.y.abs() { delta.y = 0.; } else { delta.x = 0.; } }
                if self.grid && !event.modifiers.secondary() { delta.x = scene::binding::snap_to_grid(delta.x, 20.); delta.y = scene::binding::snap_to_grid(delta.y, 20.); }
                if let Some(b) = selection_bounds(&self.doc.scene, &self.selection) { *delta = edit::bounded_delta(b, *delta); }
            }
            Some(Gesture::Vertex { start, delta, .. }) => *delta = snapped - *start,
            Some(Gesture::Endpoint { pointer, start, end, endpoint, index }) => {
                let mut q = *start + snapped - *pointer;
                if event.modifiers.shift && let Some(points) = linear_points(&self.doc.scene.elements[*index]) {
                    let anchor = if *endpoint == 0 { points[1] } else { points[points.len() - 2] };
                    let e = &self.doc.scene.elements[*index];
                    q = scene::binding::constrain_angle(anchor.rotate(e.base().angle, element_center(e)), q, 45.);
                }
                *end = World::new(q.x.clamp(-MAX_COORD, MAX_COORD), q.y.clamp(-MAX_COORD, MAX_COORD));
            }
            Some(Gesture::Resize { pointer, corner, start, rect }) => {
                let q = rect_corners(*start)[*corner] + snapped - *pointer;
                let image = self.selection.len() == 1 && matches!(self.doc.scene.elements[self.selection[0]], Element::Image(_));
                *rect = resize_corner(*start, *corner, q, event.modifiers.shift != image);
                if event.modifiers.alt {
                    let c = start.center();
                    let w = (rect.width() - start.width() / 2.).max(0.5);
                    let h = (rect.height() - start.height() / 2.).max(0.5);
                    *rect = scene::bounds::Bounds::new(c.x - w, c.y - h, c.x + w, c.y + h);
                }
            }
            Some(Gesture::Rotate { start, angle }) => {
                if let Some(b) = selection_bounds(&self.doc.scene, &self.selection) {
                    *angle = (p.y - b.center().y).atan2(p.x - b.center().x) - *start;
                    if event.modifiers.shift { *angle = (*angle / std::f64::consts::FRAC_PI_4).round() * std::f64::consts::FRAC_PI_4; }
                }
            }
            Some(Gesture::Erase { indices, last }) => {
                let steps = (p.distance(*last) * self.zoom / 6.).ceil().clamp(1., 128.) as usize;
                for step in 1..=steps {
                    let sample = last.lerp(p, step as f64 / steps as f64);
                    if let Some(i) = hit_at(&self.doc.scene, sample, self.zoom, indices) && !event.modifiers.alt {
                        indices.push(i);
                        if let Some(g) = self.doc.scene.elements[i].base().group_ids.last() {
                            indices.extend(self.doc.scene.elements.iter().enumerate().filter_map(|(n, e)| (!e.is_deleted() && !e.base().locked && e.base().group_ids.contains(g)).then_some(n)));
                        }
                    }
                    if event.modifiers.alt { indices.retain(|i| !scene::collision::hit_test_element(&self.doc.scene.elements[*i], sample, 7. / self.zoom)); }
                }
                *last = p;
            }
            Some(Gesture::Shape { start, end, ink, point_limit, from_center }) => {
                *from_center = event.modifiers.alt;
                let point = if event.modifiers.shift {
                    if matches!(self.tool, Tool::Line | Tool::Arrow) { scene::binding::constrain_angle(*start, snapped, 45.) }
                    else if self.tool != Tool::Pen { scene::binding::constrain_square(*start, snapped) } else { p }
                } else { snapped };
                *end = World::new(point.x.clamp(-MAX_COORD, MAX_COORD), point.y.clamp(-MAX_COORD, MAX_COORD));
                if self.tool == Tool::Pen && ink.len() < *point_limit && ink.last().is_some_and(|last| last.distance(p) >= 1. / self.zoom) { ink.push(p); }
            }
            None if !self.polyline.is_empty() => {}
            None => return,
        }
        self.snap_guides.clear();
        if self.objects_snap && !self.grid && !event.modifiers.secondary() && !event.modifiers.shift
            && let Some(Gesture::Move { delta, .. }) = &self.gesture {
            let (snapped, guides) = commands::snap_delta(&self.doc.scene, &self.selection, *delta, 6. / self.zoom);
            if let Some(Gesture::Move { delta, .. }) = &mut self.gesture { *delta = snapped; }
            self.snap_guides = guides;
        }
        if let Some(Gesture::Box { start, end, previous, deep }) = &self.gesture {
            let b = scene::bounds::Bounds::from_points(&[*start, *end]);
            let mut indices = previous.clone();
            indices.extend(self.doc.scene.elements.iter().enumerate().filter_map(|(i, e)| {
                if e.is_deleted() || e.base().locked { return None; }
                let r = element_bounds(e);
                (b.contains(World::new(r.min_x, r.min_y)) && b.contains(World::new(r.max_x, r.max_y))).then_some(i)
            }));
            self.selection = if *deep { indices } else { self.expand_selection(&indices) };
        }
        cx.notify();
    }

    fn up(&mut self, cx: &mut Context<Self>) {
        self.snap_guides.clear();
        let gesture = self.gesture.take();
        let mut next = self.doc.scene.clone();
        let changed = match &gesture {
            Some(Gesture::Crop { start, rect, index, .. }) if start != rect => { edit::crop_image(&mut next.elements[*index], *start, *rect, self.image_dimensions(*index)); true }
            Some(Gesture::Endpoint { start, end, endpoint, index, .. }) if start.distance(*end) > 0.5 => {
                move_endpoint_binding(&mut next, *index, *endpoint, *end, !self.binding_disabled); true
            }
            Some(Gesture::Vertex { start, delta, indices, insert, index }) if insert.is_some() || delta.length() > 0.5 => {
                edit::move_vertices(&mut next, *index, indices, *insert, *start, *delta, !self.binding_disabled); true
            }
            Some(Gesture::Move { delta, copy, .. }) if delta.length() > 0.5 => {
                if *copy {
                    let old_len = self.doc.scene.elements.len();
                    self.duplicate_offset(false, cx);
                    if self.doc.scene.elements.len() == old_len { return; }
                    next = self.doc.scene.clone();
                }
                if let Some(b) = selection_bounds(&next, &self.selection) {
                    transform_selection(&mut next, &self.selection, b, scene::bounds::Bounds::new(b.min_x + delta.x, b.min_y + delta.y, b.max_x + delta.x, b.max_y + delta.y), 0.);
                }
                true
            }
            Some(Gesture::Resize { rect, start, .. }) if rect != start => { edit::resize_selection(&mut next, &self.selection, *start, *rect); true }
            Some(Gesture::Rotate { angle, .. }) if angle.abs() > 0.001 => {
                if let Some(b) = selection_bounds(&next, &self.selection) { transform_selection(&mut next, &self.selection, b, b, *angle); } true
            }
            Some(Gesture::Erase { indices, .. }) if !indices.is_empty() => {
                self.checkpoint(); for &i in indices { self.erase_element(i); }
                self.rebuild(); cx.notify(); return;
            }
            Some(Gesture::Shape { start, end, ink, from_center, .. }) => {
                if start.distance(*end) < 2. / self.zoom && matches!(self.tool, Tool::Line | Tool::Arrow) {
                    self.polyline = vec![*start]; self.pointer = *end;
                } else if let Some(e) = self.shape(if *from_center && self.tool != Tool::Pen { *start * 2. - *end } else { *start }, *end, ink) { self.insert_shape(e, cx); }
                cx.notify(); return;
            }
            _ => false,
        };
        if changed && next.elements != self.doc.scene.elements {
            if !matches!(gesture, Some(Gesture::Move { copy: true, .. })) { self.checkpoint(); }
            self.doc.scene = next; self.touch_selection(); self.sync_frames(); self.rebuild();
        }
        if gesture.is_some() { cx.notify(); }
    }

    fn insert_shape(&mut self, mut e: Element, cx: &mut Context<Self>) {
        if let Element::Arrow(a) = &mut e && !self.binding_disabled {
            a.linear.start_binding = arrow_binding(&self.doc.scene, a.linear.points[0]);
            a.linear.end_binding = arrow_binding(&self.doc.scene, *a.linear.points.last().unwrap());
        }
        self.checkpoint();
        let index = self.doc.scene.elements.len(); self.doc.scene.elements.push(e);
        self.after_drawing(index);
        edit::refresh_scene(&mut self.doc.scene); self.rebuild(); cx.notify();
    }

    fn finish_polyline(&mut self, cx: &mut Context<Self>) {
        let points = std::mem::take(&mut self.polyline);
        if points.len() < 2 { cx.notify(); return; }
        if let Some(e) = self.shape(points[0], *points.last().unwrap(), &points) { self.insert_shape(e, cx); }
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
            Tool::Line => factory::new_line(if ink.len() > 2 { ink.to_vec() } else { vec![start, end] }, o),
            Tool::Arrow => factory::new_arrow(if ink.len() > 2 { ink.to_vec() } else { vec![start, end] }, self.start_arrowhead, self.arrowhead, o),
            Tool::Pen if ink.len() > 1 => factory::new_freedraw(ink.to_vec(), o),
            Tool::Sticky => factory::new_sticky_note(x, y, if far_enough { w.max(36.) } else { 160. }, if far_enough { h.max(24.) } else { 100. }, o),
            Tool::Frame => factory::new_frame(x, y, w.max(1.), h.max(1.), Some(tr("画框", "Frame").into()), o),
            _ => return None,
        };
        if let Element::Arrow(a) = &mut shape {
            a.elbowed = self.arrow_type == 2;
            if a.elbowed { edit::elbow_points(a); }
        }
        if matches!(self.tool, Tool::Rect | Tool::Diamond | Tool::Line) || self.tool == Tool::Arrow && self.arrow_type != 2 {
            shape.base_mut().roundness = (if self.tool == Tool::Arrow { self.arrow_type == 1 } else { self.rounded }).then(|| roundness(self.tool));
        }
        Some(shape)
    }

    fn start_text(&mut self, p: World, index: Option<usize>, text: String, window: &mut Window, cx: &mut Context<Self>) {
        if index.is_none() && self.doc.scene.elements.len() >= MAX_ELEMENTS { return; }
        self.text_at = Some((p, index));
        self.text_container = None;
        self.field.update(cx, |f, cx| { f.multiline = true; f.wrap = true; f.set_placeholder(tr("输入画板文字", "Enter drawing text"), cx); f.set_text(&text, cx); f.select_all_text(cx); });
        window.focus(&self.field.read(cx).focus_handle(cx));
    }

    fn finish_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((at, index)) = self.text_at.take() else { return };
        let container = self.text_container.take();
        let value = self.field.read(cx).text.trim().to_string();
        if value.is_empty() {
            if let Some(i) = index {
                self.checkpoint(); self.erase_element(i); self.rebuild();
            }
        } else {
            if let Some(i) = index {
                if matches!(&self.doc.scene.elements[i], Element::Text(t) if text_to_edit(t) != value) {
                    self.checkpoint();
                    if let Element::Text(t) = &mut self.doc.scene.elements[i] {
                        t.text = value.clone(); t.original_text = value;
                        resize_text(t, window);
                    }
                    self.sync_text(i);
                    touch(&mut self.doc.scene.elements[i]);
                }
            } else {
                self.checkpoint();
                let mut el = factory::new_text(at.x, at.y, &value, self.text_size, &self.style);
                let index = self.doc.scene.elements.len();
                if let Element::Text(t) = &mut el {
                    t.font_family = self.text_font;
                    t.base.stroke_color.clone_from(&self.text_color);
                    t.text_align = self.text_align;
                    if let Some(c) = container {
                        let shape = &mut self.doc.scene.elements[c];
                        t.container_id = Some(shape.id().to_string());
                        t.text_align = DrawingAlign::Center;
                        t.vertical_align = VerticalAlign::Middle;
                        shape.base_mut().bound_elements.get_or_insert_with(Vec::new)
                            .push(scene::element::BoundElement { id: t.base.id.clone(), kind: scene::element::BoundElementType::Text });
                        touch(shape);
                    }
                    resize_text(t, window);
                }
                self.doc.scene.elements.push(el);
                if let Some(c) = container {
                    self.selection = vec![c];
                    self.sync_text(index);
                } else { self.after_drawing(index); }
            }
            self.rebuild();
        }
        window.focus(&self.focus);
        cx.notify();
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        if self.busy || self.selection.is_empty() { return; }
        self.checkpoint();
        for i in selection_indices(&self.doc.scene, &self.selection) { self.erase_element(i); }
        self.selection.clear(); self.line_edit = None; self.crop = None; self.selected_points.clear(); self.rebuild(); cx.notify();
    }

    fn after_drawing(&mut self, index: usize) {
        if !self.tool_locked {
            self.tool = Tool::Select;
            self.selection = vec![index];
            if self.panel.is_none() { self.panel = Some(Panel::Properties); }
        }
        self.sync_frames();
    }

    fn erase_element(&mut self, index: usize) {
        let id = self.doc.scene.elements[index].id().to_string();
        for (i, element) in self.doc.scene.elements.iter_mut().enumerate() {
            let following = matches!(element, Element::Text(t) if t.container_id.as_deref() == Some(id.as_str()));
            if (i == index || following) && !element.is_deleted() { element.base_mut().is_deleted = true; touch(element); }
            match element {
                Element::Arrow(a) => {
                    if a.linear.start_binding.as_ref().is_some_and(|b| b.element_id == id) { a.linear.start_binding = None; }
                    if a.linear.end_binding.as_ref().is_some_and(|b| b.element_id == id) { a.linear.end_binding = None; }
                }
                Element::Line(l) => {
                    if l.linear.start_binding.as_ref().is_some_and(|b| b.element_id == id) { l.linear.start_binding = None; }
                    if l.linear.end_binding.as_ref().is_some_and(|b| b.element_id == id) { l.linear.end_binding = None; }
                }
                _ => {}
            }
            if element.base().frame_id.as_deref() == Some(id.as_str()) { element.base_mut().frame_id = None; }
        }
    }

    fn duplicate(&mut self, cx: &mut Context<Self>) { self.duplicate_offset(true, cx); }

    fn duplicate_offset(&mut self, offset: bool, cx: &mut Context<Self>) {
        if self.busy || self.selection.is_empty() { return; }
        let indices = selection_indices(&self.doc.scene, &self.selection);
        let elements: Vec<_> = indices.iter().map(|i| self.doc.scene.elements[*i].clone()).collect();
        let mut doc = self.doc.clone(); doc.scene.elements = elements;
        let points = drawing_scene::MAX_SCENE_POINTS - doc.remaining_points();
        if indices.len() + self.doc.scene.elements.len() > MAX_ELEMENTS || points > self.doc.remaining_points() { return; }
        let Some(b) = selection_bounds(&self.doc.scene, &indices) else { return };
        self.checkpoint();
        let start = self.doc.scene.elements.len();
        let mut elements = edit::clone_elements(&doc.scene.elements);
        let delta = if offset { edit::bounded_delta(b, World::new(12., 12.)) } else { World::zero() };
        for e in &mut elements { scene::transform::translate_element(e, delta.x, delta.y); }
        self.selection = indices.iter().enumerate().filter_map(|(n, i)| self.selection.contains(i).then_some(start + n)).collect();
        self.doc.scene.elements.extend(elements); self.line_edit = None; self.crop = None; self.selected_points.clear(); self.rebuild(); cx.notify();
    }

    fn reorder(&mut self, direction: LayerMove, cx: &mut Context<Self>) {
        if self.busy || self.selection.is_empty() { return; }
        let selected = selection_indices(&self.doc.scene, &self.selection);
        let mut order: Vec<_> = (0..self.doc.scene.elements.len()).collect();
        match direction {
            LayerMove::Bottom | LayerMove::Top => order.sort_by_key(|i| selected.contains(i) == matches!(direction, LayerMove::Bottom)),
            LayerMove::Back => for i in 1..order.len() { if selected.contains(&order[i]) && !selected.contains(&order[i - 1]) { order.swap(i, i - 1); } },
            LayerMove::Forward => for i in (0..order.len().saturating_sub(1)).rev() { if selected.contains(&order[i]) && !selected.contains(&order[i + 1]) { order.swap(i, i + 1); } },
        }
        if order.iter().enumerate().all(|(i, old)| i == *old) { return; }
        self.checkpoint();
        self.selection = order.iter().enumerate().filter_map(|(i, old)| self.selection.contains(old).then_some(i)).collect();
        self.doc.scene.elements = order.into_iter().map(|i| { let mut e = self.doc.scene.elements[i].clone(); e.base_mut().index = None; touch(&mut e); e }).collect();
        self.line_edit = None; self.crop = None; self.selected_points.clear(); self.rebuild(); cx.notify();
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
        self.line_edit = None; self.crop = None; self.selected_points.clear(); self.polyline.clear(); self.editing_group = None; self.edit_property = None; self.edit_frame = None; self.text_at = None; self.edit_link = None;
        self.selection.clear();
        self.rebuild();
        cx.notify();
    }

    fn sync_text(&mut self, index: usize) {
        if let Element::Text(t) = &self.doc.scene.elements[index] && t.container_id.is_some() {
            scene::binding::sync_container_text(&mut self.doc.scene);
        }
    }

    fn apply_style(&mut self, choice: StyleChoice, checkpoint: bool, window: &Window, cx: &mut Context<Self>) {
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
            StyleChoice::TextColor(c) => self.text_color = c.into(),
            StyleChoice::TextFont(v) => self.text_font = v,
            StyleChoice::TextSize(v) => self.text_size = v,
            StyleChoice::TextAlign(v) => self.text_align = v,
            StyleChoice::TextVertical(_) => {}
            StyleChoice::ArrowHead(v) => self.arrowhead = v,
            StyleChoice::StartArrowHead(v) => self.start_arrowhead = v,
            StyleChoice::ArrowType(v) => self.arrow_type = v,
        }
        let targets: Vec<_> = self.selection.iter().copied().filter_map(|i| {
            let e = &self.doc.scene.elements[i];
            if matches!(choice, StyleChoice::TextColor(_) | StyleChoice::TextFont(_) | StyleChoice::TextSize(_) | StyleChoice::TextAlign(_) | StyleChoice::TextVertical(_))
                && !matches!(e, Element::Text(_)) { self.bound_text_index(e) } else { Some(i) }
        }).collect();
        let mut checked = false;
        for i in targets {
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
                StyleChoice::ArrowHead(v) => if let Element::Arrow(a) = &mut e { a.linear.end_arrowhead = v; },
                StyleChoice::StartArrowHead(v) => if let Element::Arrow(a) = &mut e { a.linear.start_arrowhead = v; },
                StyleChoice::ArrowType(v) => if let Element::Arrow(a) = &mut e {
                    a.elbowed = v == 2; a.fixed_segments = None;
                    a.linear.base.roundness = (v == 1).then(|| roundness(Tool::Arrow));
                    if v == 2 { edit::elbow_points(a); }
                },
                _ => if let Element::Text(t) = &mut e {
                    match choice {
                        StyleChoice::TextColor(c) => t.base.stroke_color = c.into(),
                        StyleChoice::TextFont(v) if v != t.font_family => { t.font_family = v; resize_text(t, window); }
                        StyleChoice::TextSize(v) if v != t.font_size => { t.font_size = v; resize_text(t, window); }
                        StyleChoice::TextAlign(v) => t.text_align = v,
                        StyleChoice::TextVertical(v) => t.vertical_align = v,
                        _ => {}
                    }
                },
            }
            if e != self.doc.scene.elements[i] {
                if checkpoint && !checked { self.checkpoint(); checked = true; }
                touch(&mut e);
                self.doc.scene.elements[i] = e;
                if matches!(choice, StyleChoice::TextFont(_) | StyleChoice::TextSize(_) | StyleChoice::TextVertical(_)) { self.sync_text(i); }
                self.rebuild();
            }
        }
        cx.notify();
    }

    fn open_link_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy { return; }
        let Some(i) = self.selected() else { return };
        let url = self.doc.scene.elements[i].base().link.clone().unwrap_or_default();
        self.edit_link = Some(i);
        self.field.update(cx, |f, cx| { f.multiline = false; f.set_placeholder(tr("添加链接", "Add link"), cx); f.set_text(&url, cx); f.select_all_text(cx); });
        window.focus(&self.field.read(cx).focus_handle(cx));
        cx.notify();
    }

    fn finish_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.edit_property.is_some() { self.finish_property(window, cx); return; }
        if self.searching { self.searching = false; window.focus(&self.focus); cx.notify(); return; }
        if let Some(index) = self.edit_frame.take() {
            let value = self.field.read(cx).text.trim().to_string();
            if matches!(&self.doc.scene.elements[index], Element::Frame(f) | Element::MagicFrame(f) if f.name.as_deref().unwrap_or("") != value) {
                self.checkpoint();
                if let Element::Frame(f) | Element::MagicFrame(f) = &mut self.doc.scene.elements[index] { f.name = (!value.is_empty()).then_some(value); }
                touch(&mut self.doc.scene.elements[index]); self.rebuild();
            }
            window.focus(&self.focus); cx.notify(); return;
        }
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
        let Some(i) = self.selected() else { return };
        let e = &self.doc.scene.elements[i];
        if let Element::Frame(f) | Element::MagicFrame(f) = e {
            let text = f.name.clone().unwrap_or_default(); self.edit_frame = Some(i);
            self.field.update(cx, |f, cx| { f.multiline = false; f.set_placeholder(tr("画框名称", "Frame name"), cx); f.set_text(&text, cx); f.select_all_text(cx); });
            window.focus(&self.field.read(cx).focus_handle(cx)); cx.notify(); return;
        }
        if let Element::Text(t) = e {
            self.start_text(element_center(e), Some(i), text_to_edit(t).to_owned(), window, cx);
        } else if let Some(idx) = self.bound_text_index(e) && let Element::Text(t) = &self.doc.scene.elements[idx] {
            self.start_text(element_center(e), Some(idx), text_to_edit(t).to_owned(), window, cx);
        } else if matches!(e, Element::StickyNote(_) | Element::Rectangle { .. } | Element::Ellipse { .. } | Element::Diamond { .. } | Element::Arrow(_) | Element::Line(_))
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
        if !self.polyline.is_empty() { self.finish_polyline(cx); }
        self.line_edit = None; self.crop = None; self.selected_points.clear();
        self.panel = (tool == Tool::Text || self.panel == Some(Panel::Properties)).then_some(Panel::Properties);
        self.more_fonts = false;
        self.colors = None;
        self.tool = tool;
        if tool != Tool::Image { self.pending_image = None; } else { self.pick_image(window, cx); }
        if tool != Tool::Eyedropper { self.selection.clear(); }
        self.gesture = None;
        window.focus(&self.focus);
        cx.notify();
    }

    fn key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) { return; }
        let key = e.keystroke.key.as_str(); let m = e.keystroke.modifiers;
        cx.stop_propagation();
        if self.busy { return; }
        if self.reset_confirmation { if key == "escape" { self.reset_confirmation = false; cx.notify(); } return; }
        if self.discard { if key == "escape" { self.keep_editing(window, cx); } return; }
        if key == "escape" && self.gesture.is_some() { self.gesture = None; cx.notify(); return; }
        if self.gesture.is_some() { return; }
        if matches!(key, "space" | " ") { self.space = true; cx.notify(); return; }
        if !self.polyline.is_empty() && matches!(key, "enter" | "escape") { self.finish_polyline(cx); return; }
        if self.command_key(key, m, window, cx) { return; }
        if self.view_only && !m.secondary() && !matches!(key, "escape" | "1" | "2" | "pageup" | "pagedown") { return; }
        if m.secondary() {
            match key {
                "[" => self.reorder(if m.alt || m.shift { LayerMove::Bottom } else { LayerMove::Back }, cx),
                "]" => self.reorder(if m.alt || m.shift { LayerMove::Top } else { LayerMove::Forward }, cx),
                "left" | "right" | "up" | "down" if m.shift => self.align_selection(match key {
                    "left" => AlignSelection::Left, "right" => AlignSelection::Right, "up" => AlignSelection::Top, _ => AlignSelection::Bottom,
                }, cx),
                "=" | "+" | "-" | "0" => {
                    let view = self.viewport.get();
                    let factor = match key { "-" => 1. / 1.2, "0" => 1. / self.zoom, _ => 1.2 };
                    self.zoom_at(view.center(), factor); cx.notify();
                }
                "'" => { self.grid = !self.grid; cx.notify(); }
                _ => {}
            }
            return;
        }
        match key {
            "enter" | "escape" if self.crop.take().is_some() => cx.notify(),
            "escape" if self.line_edit.take().is_some() => { self.selected_points.clear(); cx.notify(); }
            "escape" if self.editing_group.take().is_some() => { self.selection.clear(); cx.notify(); }
            "escape" if self.zen => { self.zen = false; cx.notify(); }
            "escape" if self.panel.take().is_some() => cx.notify(),
            "escape" if self.tool != Tool::Select => self.tool(Tool::Select, window, cx),
            "escape" if !self.selection.is_empty() => { self.selection.clear(); cx.notify(); }
            "enter" if self.selected().is_some_and(|i| matches!(self.doc.scene.elements[i], Element::Image(_))) => { self.crop = self.selected(); cx.notify(); }
            "enter" if self.selected().is_some() => self.edit_selection_text(window, cx),
            "q" => { self.tool_locked = !self.tool_locked; cx.notify(); }
            "escape" => self.cancel(window, cx),
            "delete" | "backspace" if self.line_edit.is_some() => self.delete_points(cx),
            "delete" | "backspace" => self.delete(cx),
            "left" | "right" | "up" | "down" => {
                let step = if m.shift { 10. } else if self.grid { 20. } else { 1. };
                let delta = match key { "left" => World::new(-step, 0.), "right" => World::new(step, 0.), "up" => World::new(0., -step), _ => World::new(0., step) };
                if let Some(index) = self.line_edit && !self.selected_points.is_empty() {
                    self.checkpoint(); edit::move_vertices(&mut self.doc.scene, index, &self.selected_points, None, World::zero(), delta, true);
                    touch(&mut self.doc.scene.elements[index]); self.rebuild(); cx.notify();
                } else { self.move_selection(delta, cx); }
            }
            "1" if m.shift => { self.fit_view(); cx.notify(); }
            "2" if m.shift => self.zoom_selection(cx),
            "l" if m.shift => { self.panel = Some(Panel::Library); cx.notify(); }
            "g" if m.shift => { self.grid = !self.grid; cx.notify(); }
            "h" if m.shift => self.flip_selection(true, cx),
            "v" if m.shift => self.flip_selection(false, cx),
            "?" | "/" if key == "?" || m.shift => { self.panel = Some(Panel::Help); cx.notify(); }
            "0" => self.tool(Tool::Eraser, window, cx),
            _ => if let Some(tool) = "123456789".find(key).filter(|_| key.len() == 1)
                .map(|i| TOOLS[i + 1].0).or_else(|| TOOLS.iter().find_map(|(tool, _, _, k, _)| (*k == key).then_some(*tool))) {
                if tool == Tool::Arrow && self.tool == Tool::Arrow { self.arrow_type = (self.arrow_type + 1) % 3; cx.notify(); }
                else { self.tool(tool, window, cx); }
            }
        }
    }

}

fn linear_points(e: &Element) -> Option<&[World]> {
    match e { Element::Arrow(a) => Some(a.linear.points.as_slice()), Element::Line(l) => Some(l.linear.points.as_slice()), _ => None }
        .filter(|points| points.len() >= 2)
}

fn endpoints(e: &Element) -> Option<[World; 2]> {
    let points = linear_points(e)?;
    let pivot = element_center(e);
    Some([points[0].rotate(e.base().angle, pivot), points[points.len() - 1].rotate(e.base().angle, pivot)])
}

fn endpoint_at(e: &Element, p: World, zoom: f64) -> Option<(usize, World)> {
    endpoints(e)?.into_iter().enumerate().filter(|(_, q)| p.distance(*q) <= 9. / zoom)
        .min_by(|(_, a), (_, b)| p.distance(*a).total_cmp(&p.distance(*b)))
}

#[cfg(test)]
fn move_endpoint(doc: &mut Scene, index: usize, endpoint: usize, p: World) {
    move_endpoint_binding(doc, index, endpoint, p, true);
}

fn move_endpoint_binding(doc: &mut Scene, index: usize, endpoint: usize, p: World, bind: bool) {
    let binding = bind.then(|| arrow_binding(doc, p)).flatten();
    let e = &mut doc.elements[index];
    let (angle, pivot) = (e.base().angle, element_center(e));
    let linear = match e { Element::Arrow(a) => &mut a.linear, Element::Line(l) => &mut l.linear, _ => return };
    if linear.points.len() < 2 { return; }
    for q in &mut linear.points { *q = q.rotate(angle, pivot); }
    let point = if endpoint == 0 { 0 } else { linear.points.len() - 1 };
    let old = linear.points[point];
    linear.points[point] = p;
    if endpoint == 0 { linear.start_binding = binding; } else { linear.end_binding = binding; }
    let bounds = scene::bounds::Bounds::from_points(&linear.points);
    let b = &mut linear.base;
    (b.x, b.y, b.width, b.height, b.angle) = (bounds.min_x, bounds.min_y, bounds.width(), bounds.height(), 0.);
    if let Element::Arrow(a) = e && let Some(segments) = &mut a.fixed_segments {
        for segment in segments {
            segment.start = segment.start.rotate(angle, pivot);
            segment.end = segment.end.rotate(angle, pivot);
            if segment.start == old { segment.start = p; }
            if segment.end == old { segment.end = p; }
        }
    }
    edit::refresh_scene(doc);
}

fn can_resize(e: &Element) -> bool {
    !matches!(e, Element::Arrow(_) | Element::Line(_))
}

fn can_rotate(e: &Element) -> bool {
    !matches!(e, Element::Frame(_) | Element::MagicFrame(_))
}

fn element_center(e: &Element) -> World {
    let b = e.base(); World::new(b.x + b.width / 2., b.y + b.height / 2.)
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

fn roundness(tool: Tool) -> Roundness {
    Roundness { kind: if tool == Tool::Rect || tool == Tool::Diamond { RoundnessType::AdaptiveRadius } else { RoundnessType::ProportionalRadius }, value: None }
}

fn hit_at(doc: &Scene, p: World, zoom: f64, exclude: &[usize]) -> Option<usize> {
    doc.elements.iter().enumerate().rev().find(|(i, e)| {
        if e.is_deleted() || e.base().locked || exclude.contains(i) { return false; }
        if let Some(frame) = e.base().frame_id.as_deref().and_then(|id| doc.get(id)) && !element_bounds(frame).contains(p) { return false; }
        if matches!(e, Element::Frame(_) | Element::MagicFrame(_)) {
            let b = element_bounds(e);
            if p.x >= b.min_x && p.x <= b.max_x && p.y >= b.min_y - 24. / zoom && p.y < b.min_y { return true; }
            if b.contains(p) && (p.x - b.min_x).min(b.max_x - p.x).min(p.y - b.min_y).min(b.max_y - p.y) > 7. / zoom { return false; }
        }
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

fn paint_selection(bounds: Bounds<Pixels>, show_box: bool, resize: bool, rotate: Option<Point<Pixels>>, endpoints: Option<[Point<Pixels>; 2]>, paper: Hsla, window: &mut Window) {
    let ink = Theme::text_caption();
    if show_box { window.paint_quad(outline(bounds, ink, BorderStyle::Solid)); }
    let mut handle = |center, radius| {
        let extents = Bounds::new(center - point(px(4.), px(4.)), size(px(8.), px(8.)));
        window.paint_quad(quad(extents, px(radius), paper, Edges::all(px(1.5)), ink, BorderStyle::Solid));
    };
    if resize {
        for p in [bounds.origin, bounds.top_right(), bounds.bottom_right(), bounds.bottom_left()] { handle(p, 2.); }
    }
    if let Some(p) = rotate { handle(p, 4.); }
    if let Some(points) = endpoints { for p in points { handle(p, 4.); } }
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
        self.laser.retain(|(_, at)| at.elapsed().as_millis() < 900);
        if !self.laser.is_empty() { let weak = cx.weak_entity(); window.on_next_frame(move |_, cx| { let _ = weak.update(cx, |_, cx| cx.notify()); }); }
        let laser = self.laser.clone();
        let snap_guides = self.snap_guides.clone();
        let preview = self.preview_scene();
        let scene = preview.as_ref().unwrap_or(&self.doc.scene);
        let rasters = self.rasterized_elements(scene, window);
        let single = self.selected().map(|i| &scene.elements[i]);
        let show_selection_box = single.is_none_or(|e| linear_points(e).is_none());
        let handles = selection_bounds(scene, &self.selection).map(|b| {
            (b, self.line_edit.is_none() && single.is_none_or(can_resize), self.line_edit.is_none() && single.is_none_or(can_rotate), single.and_then(endpoints))
        });
        let crop_handles = self.crop.map(|i| {
            let e = &scene.elements[i]; let b = e.base(); rect_corners(scene::bounds::Bounds::new(b.x, b.y, b.x + b.width, b.y + b.height)).map(|p| p.rotate(b.angle, element_center(e)))
        });
        let nodes = self.line_edit.and_then(|i| { let e = &scene.elements[i]; linear_points(e).map(|points| points.iter().map(|p| p.rotate(e.base().angle, element_center(e))).collect::<Vec<_>>()) });
        let elbow_edit = self.line_edit.is_some_and(|i| matches!(scene.elements[i], Element::Arrow(ref a) if a.elbowed));
        let selected_points = self.selected_points.clone();
        let erasing = match &self.gesture { Some(Gesture::Erase { indices, .. }) => indices.clone(), _ => Vec::new() };
        let viewport = self.viewport.clone();
        let art = self.artwork.clone();
        let images = self.images.clone();
        let pending = if !self.polyline.is_empty() {
            let mut points = self.polyline.clone(); points.push(self.pointer);
            self.shape(points[0], *points.last().unwrap(), &points).map(|el| render_element(&el))
        } else { match &self.gesture {
            Some(Gesture::Shape { start, end, ink, from_center, .. }) => self.shape(if *from_center && self.tool != Tool::Pen { *start * 2. - *end } else { *start }, *end, ink).map(|el| render_element(&el)),
            _ => None,
        }};
        let geometry: Vec<_> = scene.elements.iter().map(|e| (element_bounds(e), element_center(e), e.base().angle)).collect();
        let clips: Vec<_> = scene.elements.iter().map(|e| e.base().frame_id.as_deref().and_then(|id| scene.get(id)).map(element_bounds)).collect();
        let copy_preview = matches!(self.gesture, Some(Gesture::Move { copy: true, .. }));
        let originals: Vec<_> = self.doc.scene.elements.iter().map(|e| (element_center(e), e.base().angle)).collect();
        let selection = self.selection.clone();
        let box_select = match self.gesture { Some(Gesture::Box { start, end, .. }) => Some((start, end)), _ => None };
        let mut transformed = HashMap::new();
        if preview.is_some() {
            let lookup = scene.non_deleted().map(|e| (e.id(), e)).collect();
            for (i, e) in scene.elements.iter().enumerate() {
                if !e.is_deleted() && *e != self.doc.scene.elements[i] { transformed.insert(i, render_element_with_lookup(e, &lookup)); }
            }
        }
        let fit = self.fit;
        let weak = cx.weak_entity();
        let offset = self.offset;
        let zoom = self.zoom;
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
                let screen = |p: World| origin + point(px((p.x * zoom) as f32), px((p.y * zoom) as f32));
                let box_bounds = Bounds::from_corners(screen(World::new(extent.min_x, extent.min_y)), screen(World::new(extent.max_x, extent.max_y)));
                if copy_preview && selection.contains(&i) {
                    let (pivot, angle) = originals[i];
                    for d in drawables { paint(d, |p| screen(p.rotate(angle, pivot)), zoom, dark, &images, window, cx); }
                }
                if !bounds.intersects(&box_bounds.dilate(px(96.))) { continue; }
                let mask = clips[i].map(|rect| ContentMask { bounds: Bounds::from_corners(screen(World::new(rect.min_x, rect.min_y)), screen(World::new(rect.max_x, rect.max_y))) });
                window.with_content_mask(mask, |window| {
                if let Some((image, rect)) = rasters.get(&i) {
                    let b = Bounds::from_corners(screen(World::new(rect.min_x, rect.min_y)), screen(World::new(rect.max_x, rect.max_y)));
                    let _ = window.paint_image(b, Corners::default(), image.clone(), 0, false);
                } else {
                    let drawables = transformed.get(&i).unwrap_or(drawables);
                    for d in drawables { paint(d, |p| screen(p.rotate(angle, pivot)), zoom, dark, &images, window, cx); }
                }
                });
            }
            let screen = |p: World| origin + point(px((p.x * zoom) as f32), px((p.y * zoom) as f32));
            for (a, b) in &snap_guides {
                let mut path = PathBuilder::stroke(px(1.)); path.move_to(screen(*a)); path.line_to(screen(*b));
                if let Ok(path) = path.build() { window.paint_path(path, Theme::drawing_accent().opacity(0.6)); }
            }
            for pair in laser.windows(2) {
                let mut path = PathBuilder::stroke(px(3.)); path.move_to(screen(pair[0].0)); path.line_to(screen(pair[1].0));
                let alpha = (1. - pair[0].1.elapsed().as_secs_f32() / 0.9).clamp(0., 1.);
                if let Ok(path) = path.build() { window.paint_path(path, Theme::drawing_accent().opacity(alpha)); }
            }
            if let Some((b, resize, rotate, endpoints)) = handles {
                let box_bounds = Bounds::from_corners(screen(World::new(b.min_x, b.min_y)), screen(World::new(b.max_x, b.max_y)));
                let knob = screen(World::new(b.center().x, b.min_y - 17. / zoom));
                paint_selection(box_bounds, show_selection_box, resize && crop_handles.is_none(), (rotate && crop_handles.is_none()).then_some(knob), if nodes.is_none() { endpoints.map(|points| points.map(screen)) } else { None }, paper, window);
            }
            if let Some(points) = crop_handles {
                for p in points {
                    let b = Bounds::new(screen(p) - point(px(5.), px(5.)), size(px(10.), px(10.)));
                    window.paint_quad(quad(b, px(1.), paper, Edges::all(px(2.)), Theme::drawing_accent(), BorderStyle::Solid));
                }
            }
            if let Some(points) = &nodes {
                for (i, p) in points.iter().enumerate() {
                    if elbow_edit && i > 0 && i + 1 < points.len() { continue; }
                    let b = Bounds::new(screen(*p) - point(px(4.), px(4.)), size(px(8.), px(8.)));
                    window.paint_quad(quad(b, px(4.), if selected_points.contains(&i) { Theme::drawing_accent() } else { paper }, Edges::all(px(1.5)), Theme::drawing_accent(), BorderStyle::Solid));
                }
                for (n, pair) in points.windows(2).enumerate() {
                    if elbow_edit && (n == 0 || n + 2 == points.len()) { continue; }
                    let b = Bounds::new(screen(pair[0].lerp(pair[1], 0.5)) - point(px(3.), px(3.)), size(px(6.), px(6.)));
                    window.paint_quad(quad(b, px(3.), paper, Edges::all(px(1.)), Theme::text_caption(), BorderStyle::Solid));
                }
            }
            if let Some((start, end)) = box_select {
                let b = Bounds::from_corners(screen(World::new(start.x.min(end.x), start.y.min(end.y))), screen(World::new(start.x.max(end.x), start.y.max(end.y))));
                window.paint_quad(quad(b, px(0.), Theme::drawing_selection().opacity(0.25), Edges::all(px(1.)), Theme::drawing_accent(), BorderStyle::Solid));
            }
            if let Some(pending) = pending {
                for d in &pending { paint(d, |p| origin + point(px((p.x * zoom) as f32), px((p.y * zoom) as f32)), zoom, dark, &images, window, cx); }
            }
        }).size_full();
        let mut card = div().id("drawing-dialog").size_full().occlude().bg(Theme::bg_body())
            .relative().flex().flex_col().overflow_hidden().track_focus(&self.focus).key_context("DrawingEditor")
            .on_action(cx.listener(|d, _: &SaveDrawing, w, cx| { if !d.discard && !d.reset_confirmation { d.save(w, cx); } }))
            .on_action(cx.listener(|d, _: &UndoDrawing, _, cx| { if !d.busy && !d.discard && !d.reset_confirmation && !d.view_only { d.history(false, cx); } }))
            .on_action(cx.listener(|d, _: &RedoDrawing, _, cx| { if !d.busy && !d.discard && !d.reset_confirmation && !d.view_only { d.history(true, cx); } }))
            .on_action(cx.listener(|d, _: &CopyDrawing, _, cx| { if d.editable() { d.copy_selection(false, cx); } }))
            .on_action(cx.listener(|d, _: &CutDrawing, _, cx| { if d.editable() { d.copy_selection(true, cx); } }))
            .on_action(cx.listener(|d, _: &PasteDrawing, w, cx| { if d.editable() { d.paste_selection(w, cx); } }))
            .on_action(cx.listener(|d, _: &SelectDrawing, _, cx| { if d.editable() { d.select_all(cx); } }))
            .on_action(cx.listener(|d, _: &DuplicateDrawing, _, cx| { if d.editable() { d.duplicate(cx); } }))
            .on_action(cx.listener(|d, _: &GroupDrawing, _, cx| { if d.editable() { d.group_selection(false, cx); } }))
            .on_action(cx.listener(|d, _: &UngroupDrawing, _, cx| { if d.editable() { d.group_selection(true, cx); } }))
            .on_action(cx.listener(|d, _: &LinkDrawing, w, cx| { if d.editable() { d.open_link_editor(w, cx); } }))
            .on_action(cx.listener(|d, _: &LockDrawing, _, cx| { if d.editable() { d.lock_selection(cx); } }))
            .on_action(cx.listener(|d, _: &CopyDrawingStyle, _, cx| { if d.editable() { d.copy_style(cx); } }))
            .on_action(cx.listener(|d, _: &PasteDrawingStyle, w, cx| { if d.editable() { d.paste_style(w, cx); } }))
            .on_action(cx.listener(|d, _: &CopyDrawingPng, w, cx| { if d.editable() { d.copy_png(w, cx); } }))
            .on_action(cx.listener(|d, _: &PasteDrawingText, w, cx| { if d.editable() { d.paste_text(w, cx); } }))
            .on_action(cx.listener(|d, _: &FindDrawing, w, cx| { if d.editable() { d.start_find(w, cx); } }))
            .on_action(cx.listener(|d, _: &EditDrawingPoints, w, cx| { if d.editable() { d.edit_points(w, cx); } }))
            .on_action(cx.listener(|d, _: &OpenDrawing, w, cx| { if d.editable() && !d.dirty() && d.text_at.is_none() && d.edit_link.is_none() && d.edit_frame.is_none() && d.edit_property.is_none() && d.polyline.is_empty() { d.import(w, cx); } }))
            .on_action(cx.listener(|d, _: &ExportDrawing, _, cx| { if d.editable() { d.panel = Some(Panel::Export); cx.notify(); } }))
            .on_key_down(cx.listener(Self::key))
            .on_key_up(cx.listener(|d, e: &KeyUpEvent, _, cx| { if matches!(e.keystroke.key.as_str(), "space" | " ") { d.space = false; cx.notify(); } }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(self.header(cx))
            .child(div().relative().flex_1().min_h_0().w_full()
            .child(div().id("drawing-surface").absolute().inset_0().overflow_hidden().bg(paper)
                .cursor(match self.tool { Tool::Select => CursorStyle::Arrow, Tool::Pan => CursorStyle::OpenHand, Tool::Text => CursorStyle::IBeam, _ => CursorStyle::Crosshair })
                .on_mouse_down(MouseButton::Left, cx.listener(Self::down)).on_mouse_down(MouseButton::Middle, cx.listener(Self::down))
                .on_mouse_down(MouseButton::Right, cx.listener(|d, e: &MouseDownEvent, w, cx| {
                    if !d.editable() { return; }
                    d.finish_field(w, cx); d.pointer = d.world(e.position);
                    if let Some(i) = d.select_at(d.pointer) && !d.selection.contains(&i) { d.selection = d.expand_selection(&[i]); }
                    d.panel = Some(Panel::Context); cx.stop_propagation(); cx.notify();
                }))
                .on_mouse_move(cx.listener(|d, e: &MouseMoveEvent, _, cx| {
                    if e.pressed_button.is_none() && d.polyline.is_empty() { d.up(cx); d.pointer = d.world(e.position); } else { d.moved(e, cx); }
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

fn drawing_line(text: &str, family: FontFamily, size: Pixels, color: Hsla, window: &Window) -> ShapedLine {
    let runs = [TextRun { len: text.len(), font: font(family.name()), color, background_color: None, underline: None, strikethrough: None }];
    window.text_system().shape_line(text.to_string().into(), size, &runs, None)
}

fn resize_text(t: &mut TextElement, window: &Window) {
    if t.container_id.is_some() { return; }
    if t.auto_resize {
        t.base.width = t.text.lines().filter(|s| !s.is_empty()).map(|s| {
            let line = drawing_line(s, t.font_family, px(t.font_size as f32), transparent_black(), window);
            f64::from(f32::from(line.width))
        }).fold(1., f64::max);
    }
    t.base.height = scene::text::text_height(&t.text, t.font_size, t.line_height);
}

fn text_to_edit(t: &TextElement) -> &str {
    if t.original_text.is_empty() { &t.text } else { &t.original_text }
}

fn paint(d: &Drawable, map: impl Fn(World) -> Point<Pixels>, zoom: f64, dark: bool, images: &HashMap<String, Arc<RenderImage>>, window: &mut Window, cx: &mut App) {
    let p = |x, y| map(World::new(x, y));
    let alpha = d.options.opacity / 100.;
    let stroke = drawable_ink(d.options.stroke_color, alpha, dark);
    if let Some(t) = &d.text {
        let font_size = px((t.font_size * zoom) as f32);
        let line_height = t.font_size * t.line_height;
        let h = line_height * t.text.split('\n').count() as f64;
        let top = match t.vertical_align { VerticalAlign::Top => t.y, VerticalAlign::Middle => t.y + (t.height - h) / 2., VerticalAlign::Bottom => t.y + t.height - h };
        let color = drawable_ink(t.color, alpha, dark);
        for (i, text) in t.text.lines().enumerate() {
            if text.is_empty() { continue; }
            let line = drawing_line(text, t.font_family, font_size, color, window);
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

#[cfg(test)]
mod tests {
    use super::{DrawingScene, Scene, Element, ElementOptions, Arrowhead, World, scene, factory, linear_points, endpoints, endpoint_at, move_endpoint, arrow_binding};

    #[test]
    fn drawing_endpoints_move_independently_and_round_trip() {
        let mut doc = DrawingScene::new();
        doc.scene.elements.push(factory::new_arrow(vec![World::new(190., 85.), World::new(100., 60.), World::new(25., 95.)], None, Some(Arrowhead::Arrow), &ElementOptions::default()));
        assert_eq!(endpoint_at(&doc.scene.elements[0], World::new(194., 85.), 2.), Some((0, World::new(190., 85.))));
        assert!(endpoint_at(&doc.scene.elements[0], World::new(194., 85.), 4.).is_none());
        assert!(endpoint_at(&doc.scene.elements[0], World::new(100., 60.), 1.).is_none());
        move_endpoint(&mut doc.scene, 0, 0, World::new(220., 120.));
        move_endpoint(&mut doc.scene, 0, 1, World::new(-30., -10.));
        let points = [World::new(220., 120.), World::new(100., 60.), World::new(-30., -10.)];
        assert_eq!(linear_points(&doc.scene.elements[0]).unwrap(), points);
        let reopened = DrawingScene::load(&doc.json().unwrap()).unwrap();
        assert_eq!(linear_points(&reopened.scene.elements[0]).unwrap(), points);
        let b = reopened.scene.elements[0].base();
        assert_eq!((b.x, b.y, b.width, b.height), (-30., -10., 250., 130.));
    }

    #[test]
    fn drawing_rotated_line_keeps_other_points_in_place() {
        let mut doc = Scene::new();
        let mut line = factory::new_line(vec![World::new(10., 20.), World::new(110., 20.)], &ElementOptions::default());
        line.base_mut().angle = std::f64::consts::FRAC_PI_2;
        let before = endpoints(&line).unwrap();
        assert_eq!(endpoint_at(&line, before[1], 0.5), Some((1, before[1])));
        doc.elements.push(line);
        let target = World::new(150., -20.);
        move_endpoint(&mut doc, 0, 1, target);
        let after = endpoints(&doc.elements[0]).unwrap();
        assert!(after[0].distance(before[0]) < 0.000001);
        assert_eq!(after[1], target);
        assert_eq!(doc.elements[0].base().angle, 0.);
    }

    #[test]
    fn drawing_endpoint_rebinds_and_detaches_preserving_other_binding() {
        let mut doc = Scene::new();
        doc.elements.push(factory::new_rectangle(0., 0., 100., 100., &ElementOptions::default()));
        doc.elements.push(factory::new_rectangle(300., 0., 100., 100., &ElementOptions::default()));
        let mut arrow = factory::new_arrow(vec![World::new(100., 50.), World::new(200., 50.)], None, Some(Arrowhead::Arrow), &ElementOptions::default());
        if let Element::Arrow(a) = &mut arrow { a.linear.start_binding = arrow_binding(&doc, World::new(100., 50.)); }
        doc.elements.push(arrow);
        move_endpoint(&mut doc, 2, 1, World::new(305., 50.));
        let Element::Arrow(a) = &doc.elements[2] else { unreachable!() };
        assert_eq!(a.linear.end_binding.as_ref().unwrap().element_id, doc.elements[1].id());
        assert_eq!(a.linear.points, [World::new(100., 50.), World::new(300., 50.)]);
        let start_binding = a.linear.start_binding.clone();
        move_endpoint(&mut doc, 2, 1, World::new(200., 180.));
        let Element::Arrow(a) = &doc.elements[2] else { unreachable!() };
        assert!(a.linear.end_binding.is_none());
        assert_eq!(a.linear.start_binding, start_binding);
        assert_eq!(a.linear.points[0], scene::binding::focus_point(&doc.elements[0], World::new(200., 180.)));
        assert_eq!(a.linear.points[1], World::new(200., 180.));
    }
}

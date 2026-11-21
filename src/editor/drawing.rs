//! Embed the upstream Excalidraw editor in a Quiet.md drawing draft.
use super::{drawing_scene::{DrawingScene, ExportOptions, MAX_SOURCE}, render::Env, view::{Editor, EditorEvent}};
use crate::{model::{BlockId, BlockKind, Drawing}, settings::{self, Lang, tr}, store::{self, Store}, theme::{self, Theme}, ui::{self, TipExt}};
use gpui::{prelude::FluentBuilder, *};
use quiet_excalidraw::{
    core::{geometry::Point as World, scene::{AppState, Scene}, types::Theme as BoardTheme},
    ui::{Document as BoardDocument, Editor as Board, HostEvent, HostOptions, ImageExportOptions, Language},
};
use std::{cell::RefCell, path::PathBuf, rc::Rc};

actions!(drawing, [OpenDrawing, SaveDrawing, DrawingPalette]);

pub(crate) fn load_fonts(cx: &App) {
    let fonts = quiet_excalidraw::fonts::BUNDLED.iter().map(|(_, data)| std::borrow::Cow::Borrowed(*data)).collect();
    if let Err(e) = cx.text_system().add_fonts(fonts) { log::warn!("load drawing fonts: {e:#}"); }
}

pub(crate) fn bind_keys(cx: &mut App) {
    let context = Some("DrawingEditor");
    cx.bind_keys([
        KeyBinding::new("secondary-s", SaveDrawing, context),
        KeyBinding::new("secondary-o", OpenDrawing, context),
        KeyBinding::new("secondary-shift-p", DrawingPalette, context),
    ]);
}

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
        view.update(cx, |draft, cx| draft.focus_board(window, cx));
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

struct SavedScene {
    scene: Scene,
    background: String,
    grid: bool,
    grid_size: f64,
}

impl SavedScene {
    fn from_draft(doc: &DrawingScene) -> Self {
        Self { scene: doc.scene.clone(), background: doc.background.clone(), grid: doc.grid_enabled, grid_size: doc.grid_size }
    }

    fn matches(&self, board: &Board) -> bool {
        let doc = &board.document;
        self.scene.non_deleted().eq(doc.scene.non_deleted()) && self.background == doc.view_background_color
            && self.grid == doc.show_grid && self.grid_size == doc.grid_size
    }
}

enum Modal { Discard, Import(Box<DrawingScene>) }
#[derive(Clone, Copy, PartialEq, Eq)]
enum Work { Import, Save }

pub(crate) struct DrawingEditor {
    parent: WeakEntity<Editor>,
    target: Target,
    board: Entity<Board>,
    focus: FocusHandle,
    template: Rc<RefCell<DrawingScene>>,
    saved: SavedScene,
    imported: bool,
    modal: Option<Modal>,
    work: Option<Work>,
    close_after_save: bool,
    error: Option<&'static str>,
}

impl DrawingEditor {
    fn new(parent: WeakEntity<Editor>, target: Target, doc: DrawingScene, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let template = Rc::new(RefCell::new(doc));
        let (source, png, svg) = (template.clone(), template.clone(), template.clone());
        let board = cx.new(|_| {
            let mut board = Board::new();
            board.set_host(HostOptions {
                top_inset: crate::app::HEADER_H,
                encode_scene: Rc::new(move |scene, board| {
                    let doc = snapshot(&source.borrow(), scene, board);
                    String::from_utf8(doc.json().map_err(|err| err.to_string())?).map_err(|err| err.to_string())
                }),
                export_png: Rc::new(move |scene, board, options| {
                    snapshot(&png.borrow(), scene, board).export_png(export_options(options)).map_err(|err| err.to_string())
                }),
                export_svg: Rc::new(move |scene, board, options| {
                    snapshot(&svg.borrow(), scene, board).export_svg(export_options(options)).map_err(|err| err.to_string())
                }),
                read_clipboard,
                write_clipboard,
                pick: crate::devtools::take_pick,
            });
            board.set_language(match settings::lang() {
                Lang::Zh => Language::ZhCn, Lang::En => Language::En, Lang::Ja => Language::Ja,
                Lang::Ko => Language::Ko, Lang::Fr => Language::Fr, Lang::Es => Language::Es, Lang::Ar => Language::Ar,
            });
            board.set_theme(if theme::dark() { BoardTheme::Dark } else { BoardTheme::Light });
            Self::load_board(&mut board, &template.borrow(), window);
            board
        });
        let saved = SavedScene::from_draft(&template.borrow());
        cx.observe_in(&board, window, |_, _, _, cx| cx.notify()).detach();
        cx.subscribe_in(&board, window, |draft, _, event, window, cx| match event {
            HostEvent::Open => draft.import(None, window, cx),
            HostEvent::Close => draft.cancel(window, cx),
            HostEvent::Drop(paths) => draft.drop_at(paths, window, cx),
        }).detach();
        Self { parent, target, board, focus: cx.focus_handle(), template, saved, imported: false, modal: None, work: None, close_after_save: false, error: None }
    }

    fn load_board(board: &mut Board, doc: &DrawingScene, window: &Window) {
        board.replace_scene(doc.scene.clone(), AppState {
            theme: board.document.theme, view_background_color: doc.background.clone(), files: doc.files.clone(), ..AppState::default()
        });
        board.document.show_grid = doc.grid_enabled;
        board.document.grid_size = doc.grid_size;
        board.history = quiet_excalidraw::ui::History::new(32);
        let size = window.viewport_size();
        let (w, h) = (f64::from(f32::from(size.width)), f64::from(f32::from(size.height)));
        board.document.zoom_to_fit((w, h), 65.);
        let zoom = board.document.zoom;
        if zoom > 1. {
            let center = World::new(w / 2., h / 2.);
            board.document.scroll = center + (board.document.scroll - center) / zoom;
            board.document.zoom = 1.;
        }
    }

    fn focus_board(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.board.update(cx, |board, cx| board.focus(window, cx));
    }

    fn replace_draft(&mut self, doc: DrawingScene, window: &mut Window, cx: &mut Context<Self>) {
        self.board.update(cx, |board, cx| { Self::load_board(board, &doc, window); cx.notify(); });
        *self.template.borrow_mut() = doc;
        self.imported = true;
        self.focus_board(window, cx);
        cx.notify();
    }

    fn dirty(&self, cx: &App) -> bool {
        self.imported || !self.saved.matches(self.board.read(cx))
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
        if let Some(work) = self.work {
            if work == Work::Save { self.close_after_save = true; }
            return true;
        }
        let board = self.board.read(cx);
        if self.modal.is_none() && !self.dirty(cx) && !board.is_interacting() && board.editing_text_id().is_none() { return false; }
        self.modal = Some(Modal::Discard);
        cx.notify();
        true
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.work.is_none() && !self.confirm_exit(cx) { self.close(window, cx); }
    }

    fn keep_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.modal = None;
        let _ = self.parent.update(cx, |_, cx| cx.emit(EditorEvent::DrawingExitCanceled));
        self.focus_board(window, cx);
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.work.is_some() || self.modal.is_some() { return; }
        if !self.board.update(cx, |board, cx| { cx.notify(); board.finish_for_save() }) || !self.dirty(cx) { return; }
        let board = &self.board.read(cx).document;
        let doc = snapshot(&self.template.borrow(), &board.scene, board);
        let saved = SavedScene::from_draft(&doc);
        self.error = None;
        self.work = Some(Work::Save);
        let root = Store::root_dir();
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let encoded = cx.background_executor().spawn(async move {
                let source = doc.json()?;
                DrawingScene::load(&source)?;
                Ok::<_, anyhow::Error>((source, doc.preview_dimensions()?))
            }).await;
            let _ = this.update_in(cx, |draft, window, cx| {
                draft.work = None;
                let exit = std::mem::take(&mut draft.close_after_save);
                let result = encoded.and_then(|(source, (w, h))| {
                    draft.parent.update(cx, |ed, cx| -> anyhow::Result<_> {
                        anyhow::ensure!(draft.target.valid(ed), "drawing target changed");
                        let drawing = Store::save_drawing(&root, &source, w, h)?;
                        let id = match draft.target {
                            Target::Insert { .. } => ed.core.insert_drawing(drawing.clone()),
                            Target::Edit(id, _) => { ed.core.replace_drawing(id, drawing.clone()); id }
                        };
                        ed.changed_preserving_viewport(window, cx);
                        Ok((id, drawing))
                    })?
                });
                match result {
                    Ok((id, drawing)) => {
                        draft.target = Target::Edit(id, drawing);
                        draft.saved = saved;
                        draft.imported = false;
                        if exit { draft.close(window, cx); } else { draft.focus_board(window, cx); }
                    }
                    Err(e) => {
                        log::warn!("save drawing: {e:#}");
                        draft.error = Some(tr("无法保存画板。可以先导出图源。", "Could not save drawing. You can export the source."));
                        draft.keep_editing(window, cx);
                    }
                }
                cx.notify();
            });
        }).detach();
    }

    fn import(&mut self, path: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        if self.work.is_some() || self.modal.is_some() { return; }
        self.work = Some(Work::Import);
        self.error = None;
        let picked = path.map(|p| vec![p]).or_else(crate::devtools::take_pick);
        let rx = picked.is_none().then(|| cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false,
            prompt: Some(tr("导入 Excalidraw", "Import Excalidraw").into()), extensions: Some(["excalidraw", "json", "png", "svg"].map(Into::into).to_vec()) }));
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let paths = match rx { Some(rx) => rx.await.ok().and_then(Result::ok).flatten(), None => picked };
            let doc = if let Some(path) = paths.and_then(|p| p.into_iter().next()) {
                Some(cx.background_executor().spawn(async move { DrawingScene::load_exported(&store::read_bounded(&path, MAX_SOURCE * 4)?) }).await)
            } else { None };
            let _ = this.update_in(cx, |draft, window, cx| {
                draft.work = None;
                match doc {
                    Some(Ok(doc)) if draft.dirty(cx) || draft.board.read(cx).editing_text_id().is_some() => draft.modal = Some(Modal::Import(Box::new(doc))),
                    Some(Ok(doc)) => { draft.keep_editing(window, cx); draft.replace_draft(doc, window, cx); }
                    Some(Err(e)) => {
                        log::warn!("import drawing: {e:#}");
                        draft.error = Some(tr("无法导入此 Excalidraw 文件", "Could not import this Excalidraw file"));
                        draft.keep_editing(window, cx);
                    }
                    None => draft.keep_editing(window, cx),
                }
                cx.notify();
            });
        }).detach();
    }

    pub(crate) fn drop_at(&mut self, paths: &[PathBuf], window: &mut Window, cx: &mut Context<Self>) {
        if self.work.is_some() || self.modal.is_some() { return; }
        if let Some(path) = paths.iter().find(|p| p.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("excalidraw") || ext.eq_ignore_ascii_case("json"))) {
            self.import(Some(path.clone()), window, cx);
        } else {
            self.board.update(cx, |board, cx| { board.insert_image_paths(paths); cx.notify(); });
        }
    }

    fn toggle_palette(&self, cx: &mut Context<Self>) {
        if self.work.is_none() && self.modal.is_none() {
            self.board.update(cx, |board, cx| { board.toggle_palette(); cx.notify(); });
        }
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.modal.take() {
            Some(Modal::Discard) => self.close(window, cx),
            Some(Modal::Import(doc)) => { self.keep_editing(window, cx); self.replace_draft(*doc, window, cx); }
            None => {}
        }
    }
}

impl Render for DrawingEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self.parent.upgrade().map(|ed| crate::model::display_title(&ed.read(cx).core.doc.title.text)).unwrap_or_default();
        let board = self.board.read(cx);
        let can_save = self.work.is_none() && self.modal.is_none()
            && (self.dirty(cx) || board.editing_text_id().is_some() || board.pending_point_count() > 0);
        let save = tr("保存", "Save");
        let header = div().id("drawing-header").absolute().top_0().left_0().right_0().occlude().h(px(crate::app::HEADER_H))
            .flex().items_center().gap(px(6.)).pl(px(theme::TRAFFIC_LIGHTS_W)).pr(px(if cfg!(target_os = "macos") { 12. } else { 166. }))
            .bg(Theme::window()).border_b_1().border_color(Theme::line_divider())
            .child(div().id("drawing-back").size(px(32.)).flex_none().flex().items_center().justify_center().rounded(px(8.))
                .cursor_pointer().hover(|s| s.bg(Theme::fill_hover())).child(crate::editor_ui::icon("chevron-left", 17., Theme::text()))
                .tip(tr("返回笔记", "Back to note"), None).on_click(cx.listener(|d, _, window, cx| d.cancel(window, cx))))
            .child(div().flex_1().min_w_0().flex().gap(px(6.)).items_center().text_size(px(14.)).font_weight(FontWeight::MEDIUM)
                .child(div().flex_none().child(tr("画板", "Drawing")))
                .when(!title.is_empty(), |d| d.child(div().flex_none().text_color(Theme::text_caption()).child("/")).child(div().min_w_0().truncate().child(ui::ui_text(title)))))
            .children(self.error.map(|msg| div().min_w_0().truncate().text_size(px(12.)).text_color(Theme::text_caption()).child(ui::ui_text(msg))))
            .when(self.work.is_some(), |d| d.child(div().text_size(px(12.)).text_color(Theme::text_caption()).child(tr("正在处理", "Working…"))))
            .child(div().id("drawing-save").flex_none().h(px(crate::app::HEADER_ACTIONS_H)).px(px(12.)).flex().items_center().rounded(px(8.))
                .text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(if can_save { Theme::text() } else { Theme::text_disabled() })
                .when(can_save, |d| d.bg(Theme::fill_hover()).cursor_pointer().hover(|s| s.bg(Theme::fill_hover_strong()))
                    .active(|s| s.bg(Theme::fill_pressed())).on_click(cx.listener(|d, _, window, cx| d.save(window, cx))))
                .child(save).tip(save, Some(crate::shortcuts::hint("editor::Save", "⌘S"))));
        if (self.modal.is_some() || self.work.is_some()) && !self.focus.is_focused(window) { window.focus(&self.focus); }
        let modal = self.modal.as_ref().map(|modal| {
            let (label, action) = match modal {
                Modal::Discard => (tr("放弃未保存的画板修改？", "Discard unsaved drawing changes?"), "放弃修改"),
                Modal::Import(_) => (tr("打开绘图并替换未保存的画板修改？", "Open drawing and replace unsaved changes?"), "打开绘图"),
            };
            div().id("drawing-confirm-scrim").absolute().inset_0().occlude().bg(ui::scrim()).flex().justify_center().items_center()
                .child(ui::dialog("drawing-confirm").p(px(20.)).flex().flex_col().gap(px(16.)).child(label)
                    .child(div().flex().justify_end().gap(px(8.))
                        .child(ui::secondary_button("drawing-keep", "取消").on_click(cx.listener(|d, _, window, cx| d.keep_editing(window, cx))))
                        .child(ui::primary_button("drawing-confirm-action", action).on_click(cx.listener(|d, _, window, cx| d.confirm(window, cx))))))
        });
        div().id("drawing-editor").relative().size_full().occlude().key_context("DrawingEditor").track_focus(&self.focus)
            .on_action(cx.listener(|d, _: &SaveDrawing, window, cx| d.save(window, cx)))
            .on_action(cx.listener(|d, _: &OpenDrawing, window, cx| d.import(None, window, cx)))
            .on_action(cx.listener(|d, _: &DrawingPalette, _, cx| d.toggle_palette(cx)))
            .on_action(cx.listener(|d, _: &crate::modes::ToggleShortcuts, _, cx| d.toggle_palette(cx)))
            .on_key_down(cx.listener(|d, e: &KeyDownEvent, window, cx| {
                if d.modal.is_some() && e.keystroke.key == "escape" { d.keep_editing(window, cx); }
                cx.stop_propagation();
            }))
            .child(self.board.clone())
            .child(header)
            .children(ui::window_controls(window))
            .when(self.work.is_some(), |d| d.child(div().absolute().inset_0().occlude()))
            .children(modal)
    }
}

fn snapshot(template: &DrawingScene, scene: &Scene, board: &BoardDocument) -> DrawingScene {
    let mut doc = template.clone();
    doc.scene = scene.clone();
    doc.files = board.files.clone();
    doc.background.clone_from(&board.view_background_color);
    doc.grid_enabled = board.show_grid;
    doc.grid_size = board.grid_size;
    doc
}

fn export_options(options: ImageExportOptions) -> ExportOptions {
    ExportOptions { background: options.background, dark: options.dark, embed: options.embed, scale: options.scale, show_frames: true }
}

fn read_clipboard(cx: &App) -> Vec<ClipboardEntry> {
    if !crate::devtools::scripted() { return cx.read_from_clipboard().into_iter().flat_map(ClipboardItem::into_entries).collect(); }
    let fake = crate::clipboard::FAKE_CLIPBOARD.lock().unwrap();
    let Some(fake) = fake.as_ref() else { return Vec::new() };
    fake.images.iter().cloned().map(ClipboardEntry::Image)
        .chain(Some(ClipboardEntry::String(ClipboardString::new(fake.text.clone())))).collect()
}

fn write_clipboard(cx: &App, item: ClipboardItem) {
    if !crate::devtools::scripted() { cx.write_to_clipboard(item); return; }
    let text = item.text().unwrap_or_default();
    let images = item.into_entries().filter_map(|entry| if let ClipboardEntry::Image(image) = entry { Some(image) } else { None }).collect();
    *crate::clipboard::FAKE_CLIPBOARD.lock().unwrap() = Some(crate::clipboard::FakeClipboard { text, images, html: None });
}

pub(crate) use quiet_excalidraw::ui::han_font;

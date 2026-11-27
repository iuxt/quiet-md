use crate::app::{View, Workspace};
use crate::settings::tr;
use crate::theme::Theme;
use crate::ui::{icon, popover};
use gpui::*;

impl Workspace {
    /// 新建文档/Markdown under `parent` (None = top level), opened and focused.
    /// `markdown` picks the on-disk format; the choice is remembered for ⌘N.
    pub(crate) fn create_doc(&mut self, markdown: bool, parent: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.create_menu = None;
        let (m, p) = (markdown, parent.clone());
        if !self.save_before(cx, move |w, window, cx| w.create_doc(m, p.clone(), window, cx)) { return; }
        let file_before = self.store.track_file_changes();
        let doc = crate::model::Document::default();
        let made = if markdown { self.store.create_markdown_in(&doc, "", parent.as_deref()) }
            else { self.store.create_from_in(&doc, parent.as_deref()) };
        match made {
            Ok(id) => {
                self.search_open = false;
                self.doc_info = None;
                self.home.sort_menu = false;
                self.entries = self.store.list();
                self.sort();
                match &parent { Some(p) => { self.expanded.insert(p.clone()); } None => self.folded[1] = false }
                self.search.update(cx, |s, cx| s.set_text("", cx));
                self.open(&id, window, cx);
                if let Some(ed) = &self.editor { ed.read(cx).focus(window); }
            }
            Err(e) => self.status = crate::app::SaveStatus::Failed(crate::localized_format!("新建失败：{}", "Could not create document: {}", crate::settings::diagnostic(&e.to_string()))),
        }
        self.set_pref(|p| p.last_new = if markdown { "markdown" } else { "doc" }.into(), cx);
        self.remember_files(file_before, crate::settings::tr(if markdown { "新建 Markdown" } else { "新建文档" }, if markdown { "New Markdown" } else { "New document" }));
        cx.notify();
    }

    pub(crate) fn creation_parent(&self) -> Option<String> {
        if self.view != View::Doc || self.trash_document { return None; }
        self.current.as_ref().and_then(|id| self.entries.iter().find(|e| &e.id == id)).and_then(|e| e.meta.parent.clone())
    }

    pub(crate) fn new_child(&mut self, parent: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.create_doc(false, Some(parent.to_string()), window, cx);
    }

    pub(crate) fn show_child_create_menu(&mut self, id: String, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.row_menu = None;
        self.close_doc_menu(cx);
        self.create_menu = Some((Some(id), at));
        cx.notify();
    }

    pub(crate) fn render_create_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (parent, pos) = self.create_menu.clone()?;
        let row = |id: &'static str, glyph: &'static str, label: &'static str| crate::ui::menu_row(id, SharedString::from(id)).child(icon(glyph, 16., Theme::icon())).child(label);
        let doc_parent = parent.clone();
        Some(deferred(anchored().anchor(Corner::TopLeft).position(pos).snap_to_window_with_margin(px(8.)).child(
            popover().id("create-items").occlude().w(px(180.)).py(px(crate::ui::MENU_PAD_Y))
                .on_mouse_down_out(cx.listener(|w, _, _, cx| { w.create_menu = None; cx.notify(); }))
                .child(row("create-document", "doc", tr("文档", "Document")).on_click(cx.listener(move |w, _, window, cx| w.create_doc(false, doc_parent.clone(), window, cx))))
                .child(row("create-markdown", "doc-md", "Markdown").on_click(cx.listener(move |w, _, window, cx| w.create_doc(true, parent.clone(), window, cx))))
        )).with_priority(3).into_any_element())
    }
}

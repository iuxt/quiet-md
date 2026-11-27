//! Window-local tabs. A tab starts on the recent documents page and keeps its
//! identity when that page is replaced with a document.

use crate::app::{View, Workspace};
use crate::editor::{core::Core, view::Editor};
use crate::settings::{display_title, tr};
use crate::theme::Theme;
use crate::ui::{TipExt, TitleBarExt, icon, list_line, menu_row, popover, row_hover, ui_text};
use gpui::{prelude::FluentBuilder, *};
use std::sync::Arc;

pub(crate) const BAR_H: f32 = 42.;
const BAR_INSET: f32 = 18.;
const TAB_GAP: f32 = 4.;
const TAB_MIN_W: f32 = 120.;
const TAB_MAX_W: f32 = 176.;

#[derive(Clone)]
pub(crate) struct ParkedTab {
    pub core: Core,
    pub top: Option<(usize, f32)>,
    pub baseline: Option<Arc<String>>,
}

pub(crate) struct Tab {
    key: usize,
    document: Option<String>,
    pub parked: Option<ParkedTab>,
}

#[derive(Default)]
pub(crate) struct Tabs {
    items: Vec<Tab>,
    selected: Option<usize>,
    next_key: usize,
    overflow: bool,
    overflow_bounds: std::rc::Rc<std::cell::Cell<Option<Bounds<Pixels>>>>,
}

impl Tabs {
    pub(crate) fn get_mut(&mut self, id: &str) -> Option<&mut Tab> {
        self.items.iter_mut().find(|tab| tab.document.as_deref() == Some(id))
    }

    pub(crate) fn get(&self, id: &str) -> Option<&Tab> {
        self.items.iter().find(|tab| tab.document.as_deref() == Some(id))
    }

    pub(crate) fn selected_document(&self) -> Option<&str> {
        self.selected_index().and_then(|i| self.items[i].document.as_deref())
    }

    pub(crate) fn selected_is_blank(&self) -> bool {
        self.selected.is_some() && self.selected_document().is_none()
    }

    fn selected_index(&self) -> Option<usize> {
        self.items.iter().position(|tab| Some(tab.key) == self.selected)
    }

    fn activate(&mut self, key: usize) {
        self.selected = Some(key);
        self.overflow = false;
    }

    fn insert(&mut self, document: Option<String>) {
        let key = self.next_key;
        self.next_key += 1;
        let i = self.selected_index().map_or(self.items.len(), |i| i + 1);
        self.items.insert(i, Tab { key, document, parked: None });
        self.activate(key);
    }

    pub(crate) fn select_document(&mut self, id: &str, new_tab: bool) {
        if let Some(tab) = self.get(id) {
            self.activate(tab.key);
        } else if !new_tab && let Some(i) = self.selected_index() {
            self.items[i].document = Some(id.to_string());
            self.items[i].parked = None;
            self.overflow = false;
        } else {
            self.insert(Some(id.to_string()));
        }
    }

    pub(crate) fn remove_document(&mut self, id: &str) {
        if let Some(tab) = self.get(id) { self.remove(tab.key); }
    }

    fn remove(&mut self, key: usize) {
        let Some(i) = self.items.iter().position(|tab| tab.key == key) else { return };
        self.items.remove(i);
        if self.selected == Some(key) {
            self.selected = self.items.get(i).or_else(|| self.items.last()).map(|tab| tab.key);
        }
        self.overflow = false;
    }

    pub(crate) fn prune(&mut self, mut keep: impl FnMut(&str) -> bool) {
        let gone: Vec<_> = self.items.iter().filter(|tab| tab.document.as_deref().is_some_and(|id| !keep(id))).map(|tab| tab.key).collect();
        for key in gone { self.remove(key); }
    }

    fn step(&self, offset: isize, visible: bool) -> Option<usize> {
        if self.items.len() < 1 + usize::from(visible) { return None }
        let i = self.selected_index().unwrap_or(0);
        let next = if visible { (i as isize + offset).rem_euclid(self.items.len() as isize) as usize } else { i };
        Some(self.items[next].key)
    }
}

impl Workspace {
    pub(crate) fn dismiss_tab_overflow(&mut self) -> bool { std::mem::take(&mut self.tabs.overflow) }

    fn tab_actions_available(&self) -> bool {
        !self.quick_note && self.confirm.is_none() && self.onboarding.is_none() && self.move_dlg.is_none()
            && self.export_md.is_none() && !self.store.file_task_busy() && !self.file_history.busy
    }

    pub(crate) fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tab_actions_available() || self.drawing_open(cx) { return }
        self.open_blank_tab(None, window, cx);
    }

    fn open_blank_tab(&mut self, key: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        if key.is_some_and(|key| !self.tabs.items.iter().any(|tab| tab.key == key && tab.document.is_none())) { return }
        self.close_search(window, cx);
        self.close_history(cx);
        if !self.save_before(cx, move |w, window, cx| w.open_blank_tab(key, window, cx)) { return }
        self.leave_document(window, cx);
        if let Some(key) = key { self.tabs.activate(key); }
        else { self.tabs.insert(None); }
        self.show_new_tab_page(window, cx);
    }

    pub(crate) fn show_new_tab_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.batch = None;
        self.create_menu = None;
        self.import_menu = None;
        self.place_menu = None;
        self.show_view(View::NewTab, cx);
        window.focus(&self.focus);
    }

    fn tab_page_visible(&self) -> bool {
        self.view == View::NewTab || (self.view == View::Doc && !self.trash_document)
    }

    fn activate_tab(&mut self, key: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tab_actions_available() { return }
        let Some(tab) = self.tabs.items.iter().find(|tab| tab.key == key) else { return };
        if let Some(id) = tab.document.clone() {
            self.dismiss_tab_overflow();
            self.open(&id, window, cx);
        } else if self.view != View::NewTab || self.tabs.selected != Some(key) {
            self.open_blank_tab(Some(key), window, cx);
        }
    }

    pub(crate) fn open_from_click(&mut self, id: &str, modifiers: Modifiers, window: &mut Window, cx: &mut Context<Self>) {
        if modifiers.secondary() && !self.quick_note && self.entries.iter().any(|entry| entry.id == id) {
            self.open_in_tab(id, window, cx);
        } else {
            self.open(id, window, cx);
        }
    }

    pub(crate) fn open_in_tab(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab_actions_available() { self.open_target(id, true, window, cx); }
    }

    pub(crate) fn step_tab(&mut self, offset: isize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tab_actions_available() || self.search_open { return }
        if let Some(key) = self.tabs.step(offset, self.tab_page_visible()) { self.activate_tab(key, window, cx); }
    }

    pub(crate) fn close_selected_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search_open { return }
        if let Some(key) = self.tabs.selected { self.close_tab(key, window, cx); }
    }

    fn close_tab(&mut self, key: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tab_actions_available() { return }
        let Some(tab) = self.tabs.items.iter().find(|tab| tab.key == key) else { return };
        let active = self.tab_page_visible() && self.tabs.selected == Some(key);
        let has_editor = tab.document.is_some() && tab.document == self.current && !self.trash_document;
        if has_editor {
            self.close_history(cx);
            if !self.save_before(cx, move |w, window, cx| w.close_tab(key, window, cx)) { return }
        }
        self.tabs.remove(key);
        if has_editor { self.leave_document(window, cx); }
        if active {
            self.view = View::Home;
            window.focus(&self.focus);
            if let Some(next) = self.tabs.selected { self.activate_tab(next, window, cx); }
        }
        cx.notify();
    }

    pub(crate) fn refresh_parked_tabs(&self, id: &str, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        let me = cx.entity_id();
        let baseline = self.store.edit_baseline(id);
        for handle in cx.windows().into_iter().filter_map(|w| w.downcast::<Workspace>()) {
            if handle.is_active(cx).is_none() || handle.entity(cx).is_ok_and(|e| e.entity_id() == me) { continue }
            let _ = handle.update(cx, |w, _, cx| {
                if let Some(tab) = w.tabs.get_mut(id).and_then(|t| t.parked.as_mut()) {
                    let core = &editor.read(cx).core;
                    if tab.core.doc != core.doc { Editor::receive_core(&mut tab.core, core); }
                    tab.baseline = baseline.clone();
                    cx.notify();
                }
            });
        }
    }

    pub(crate) fn parked_document_peer(id: &str, window: &Window, cx: &mut Context<Self>) -> Option<ParkedTab> {
        cx.windows().into_iter().filter(|w| w.window_id() != window.window_handle().window_id())
            .filter_map(|w| w.downcast::<Workspace>())
            .find_map(|w| w.update(cx, |ws, _, _| ws.tabs.get(id).and_then(|tab| tab.parked.clone())).ok().flatten())
    }

    pub(crate) fn prune_tabs(&mut self) {
        self.tabs.prune(|id| self.current.as_deref() == Some(id)
            || self.entries.iter().any(|e| e.id == id));
    }

    pub(crate) fn render_new_tab(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let recent = self.entries.iter().take(12);
        let list = div().w_full().max_w(px(720.)).mx_auto().pt(px(40.)).pb(px(28.))
            .child(div().px(px(16.)).pb(px(14.)).text_size(px(22.)).line_height(px(30.)).font_weight(FontWeight::SEMIBOLD)
                .child(tr("最近编辑", "Recently edited")))
            .when(recent.clone().next().is_none(), |d| d.child(div().pt(px(48.))
                .child(crate::trash::empty_state("illus/empty-list.svg", crate::settings::text("暂无文档")))))
            .children(recent.map(|e| {
                let id = e.id.clone();
                let title = display_title(&e.title);
                div().id(SharedString::from(format!("new-tab-document-{id}"))).h(px(54.)).px(px(16.)).flex().items_center()
                    .border_t_1().border_color(list_line()).cursor_pointer().hover(row_hover)
                    .tip(&title, None)
                    .child(crate::search::doc_icon_named(if e.meta.markdown { "doc-md" } else { "doc" }))
                    .child(div().flex_1().min_w_0().truncate().text_size(px(16.)).line_height(px(24.)).child(ui_text(title)))
                    .when(e.meta.modified != 0, |d| d.child(div().ml(px(16.)).flex_none().text_size(px(13.)).text_color(Theme::text_caption())
                        .child(crate::ui::format_visit(e.meta.modified))))
                    .on_click(cx.listener(move |w, ev: &ClickEvent, window, cx| w.open_from_click(&id, ev.modifiers(), window, cx)))
            }));
        div().size_full().flex().flex_col()
            .child(crate::trash::page_header(self, tr("新建标签页", "New Tab"), 18., cx))
            .children(self.render_document_tabs(window, cx))
            .child(div().id("new-tab-page").flex_1().min_h_0().px(px(24.)).overflow_y_scroll().child(list))
            .into_any_element()
    }

    pub(crate) fn shows_document_tabs(&self) -> bool {
        !self.quick_note && (self.tab_page_visible() || self.view == View::Home)
    }

    pub(crate) fn render_document_tabs(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.shows_document_tabs() { return None }
        let tabs = &self.tabs.items;
        let width = f32::from(window.viewport_size().width) - if self.sidebar_open { self.sidebar_w(window) } else { 0. };
        let selected = self.tabs.selected_index().unwrap_or(0);
        let visible = self.tab_page_visible();
        let button_space = crate::app::HEADER_ACTIONS_H + TAB_GAP;
        let tab_space = TAB_MIN_W + TAB_GAP;
        let available = width - 2. * BAR_INSET - button_space;
        let overflow = tabs.len() as f32 * tab_space > available;
        let available = available - if overflow { button_space } else { 0. };
        let count = if overflow { (available / tab_space) as usize } else { tabs.len() }.max(1);
        let start = (selected + 1).saturating_sub(count);
        let tab_width = (available / count as f32 - TAB_GAP).min(TAB_MAX_W);
        let bounds = self.tabs.overflow_bounds.clone();
        if !overflow { bounds.set(None); }
        let row = div().id("document-tabs").flex_none().h(px(BAR_H)).px(px(BAR_INSET)).flex().items_center().gap(px(TAB_GAP))
            .border_b_1().border_color(Theme::line_divider())
            .children(tabs.iter().enumerate().skip(start).take(count).map(|(i, tab)| {
                let (title, glyph) = self.tab_label(tab, cx);
                let key = tab.key;
                let group = SharedString::from(format!("document-tab-{key}"));
                let active = visible && i == selected;
                div().id(group.clone()).group(group.clone()).no_drag().h(px(32.)).w(px(tab_width)).flex_none().flex().items_center().rounded(px(8.))
                    .pl(px(10.)).pr(px(4.)).gap(px(7.)).cursor_pointer()
                    .when(active, |d| d.bg(Theme::fill_hover_strong()).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text()))
                    .when(!active, |d| d.hover(|s| s.bg(Theme::fill_hover())).text_color(Theme::text_caption()))
                    .tip(&title, None)
                    .child(icon(glyph, 16., if active { Theme::icon() } else { Theme::text_caption() }).flex_none())
                    .child(div().flex_1().min_w_0().truncate().text_size(px(14.)).line_height(px(20.)).child(ui_text(title)))
                    .child(div().id("close").no_drag().flex_none().size(px(24.)).rounded(px(6.)).flex().items_center().justify_center()
                        .hover(|s| s.bg(Theme::fill_pressed()))
                        .when(!active, |d| d.opacity(0.).group_hover(group, |s| s.opacity(1.)))
                        .tip(tr("关闭标签页", "Close tab"), None)
                        .child(icon("x", 14., Theme::icon()))
                        .on_click(cx.listener(move |w, _, window, cx| { cx.stop_propagation(); w.close_tab(key, window, cx); })))
                    .on_click(cx.listener(move |w, _, window, cx| w.activate_tab(key, window, cx)))
            }))
            .when(overflow, |d| d.child(crate::ui::header_icon("document-tabs-more", "chevron-down").relative()
                .child(canvas(move |b, _, _| bounds.set(Some(b)), |_, _, _, _| {}).absolute().inset_0())
                .tip(tr("所有标签页", "All tabs"), None)
                .on_click(cx.listener(|w, _, _, cx| { w.tabs.overflow = !w.tabs.overflow; cx.notify(); }))))
            .child(crate::ui::header_icon("document-tabs-add", "plus")
                .tip(tr("新建标签页", "New tab"), Some(crate::shortcuts::hint("workspace::NewTab", "⌘ + T")))
                .on_click(cx.listener(|w, _, window, cx| w.new_tab(window, cx))));
        Some(row.into_any_element())
    }

    pub(crate) fn render_tab_overflow(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.tabs.overflow || !self.shows_document_tabs() { return None }
        let bounds = self.tabs.overflow_bounds.clone();
        let position = bounds.get()?.bottom_right() + point(px(0.), px(4.));
        let group = SharedString::from("document-tabs-menu");
        let menu = popover().id("document-tabs-menu")
            .occlude().w(px(245.)).max_h(px((f32::from(window.viewport_size().height) - 100.).max(80.)))
            .flex().flex_col().overflow_y_scroll()
            .py(px(crate::ui::MENU_PAD_Y))
            .on_mouse_down_out(cx.listener(move |w, ev: &MouseDownEvent, _, cx| {
                if !bounds.get().is_some_and(|b| b.contains(&ev.position)) { w.tabs.overflow = false; cx.notify(); }
            }))
            .children(self.tabs.items.iter().map(|tab| {
                let (title, glyph) = self.tab_label(tab, cx);
                let key = tab.key;
                let active = self.tab_page_visible() && self.tabs.selected == Some(key);
                menu_row(key, group.clone()).tip(&title, None)
                    .child(icon(if active { "check" } else { glyph }, 15., Theme::icon()).flex_none())
                    .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(ui_text(title)))
                    .on_click(cx.listener(move |w, _, window, cx| {
                        w.tabs.overflow = false;
                        w.activate_tab(key, window, cx);
                        cx.notify();
                    }))
            }));
        Some(deferred(anchored().anchor(Corner::TopRight).position(position).snap_to_window_with_margin(px(8.)).child(menu)).with_priority(4).into_any_element())
    }

    fn tab_label(&self, tab: &Tab, cx: &App) -> (String, &'static str) {
        let Some(id) = tab.document.as_deref() else { return (tr("新建标签页", "New Tab").into(), "home") };
        let meta = self.entries.iter().find(|e| e.id == id);
        let live = self.editor.as_ref().filter(|_| self.current.as_deref() == Some(id) && !self.trash_document && self.preview.is_none())
            .map(|e| e.read(cx).core.doc.title.text.as_str());
        let title = live.or_else(|| meta.map(|e| e.title.as_str())).unwrap_or_default();
        (display_title(title), if meta.is_some_and(|e| e.meta.markdown) { "doc-md" } else { "doc" })
    }
}

#[cfg(feature = "devtools")]
pub(crate) fn check_tabs(phase: &str, cx: &mut App) {
    use crate::app::Confirm;
    use crate::editor::core::{EditKind, Pos, Selection};
    use crate::model::RichText;
    use crate::shared_store::SharedStore;
    let windows: Vec<_> = cx.windows().into_iter().filter_map(|w| w.downcast::<Workspace>()).collect();
    let main = *windows.iter().find(|w| w.update(cx, |w, _, _| !w.quick_note).unwrap_or(false)).unwrap();
    let mut store = cx.global::<SharedStore>().clone();
    let entries = store.list();
    if phase == "empty" {
        assert!(entries.is_empty());
        main.update(cx, |w, _, _| {
            assert_eq!(w.view, View::NewTab);
            assert!(w.tabs.selected_is_blank() && w.shows_document_tabs());
            assert_eq!(w.tabs.items.len(), 1);
            assert!(w.current.is_none() && w.editor.is_none() && !w.search_open);
        }).unwrap();
        eprintln!("tabs: {phase} checked");
        return;
    }
    let ids = ["Tab A", "Tab B", "Tab C"].map(|title| entries.iter().find(|entry| entry.title == title).unwrap().id.clone());
    let [a, b, c] = &ids;
    let edit = |w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>, text: &str| {
        w.editor.as_ref().unwrap().update(cx, |ed, cx| {
            let block = &ed.core.doc.blocks[34];
            ed.core.sel = Selection::caret(Pos::new(block.id, block.text.len()));
            ed.core.checkpoint(EditKind::Other);
            ed.core.insert_text(text);
            ed.changed(window, cx);
        });
    };
    if phase == "peer-edit" {
        let id = a.clone();
        let peer = cx.open_window(WindowOptions { focus: false, show: false, ..Default::default() }, |window, cx| {
            cx.new(|cx| Workspace::new_document_window(Some(id), window, cx))
        }).unwrap();
        peer.update(cx, |w, window, cx| edit(w, window, cx, " from peer")).unwrap();
        return;
    }
    if phase == "peer" {
        let peer = windows.into_iter().find(|w| w.update(cx, |w, _, _| w.quick_note).unwrap_or(false)).unwrap();
        peer.update(cx, |w, window, cx| {
            assert!(w.flush(cx), "{}", w.save_error());
            assert!(w.tabs.items.is_empty());
            assert!(!w.shows_document_tabs());
            window.remove_window();
        }).unwrap();
    }
    main.update(cx, |w, window, cx| {
        let current = w.current.clone();
        let count = w.tabs.items.len();
        match phase {
            "startup" => {
                assert_eq!(w.view, View::Doc);
                assert_eq!(current.as_ref(), Some(a));
                assert!(count == 1 && w.shows_document_tabs());
            }
            "prepare" => {
                w.entries = entries;
                w.sort();
                assert!(w.try_open(a, window, cx));
                edit(w, window, cx, " edited");
                w.editor.as_ref().unwrap().update(cx, |e, cx| { e.restore_top = Some((30, 0.)); cx.notify(); });
            }
            "blank" | "plus" => {
                assert_eq!(w.view, View::NewTab);
                assert!(w.current.is_none() && w.editor.is_none() && !w.search_open);
                assert!(w.tabs.selected_is_blank());
                assert_eq!(count, if phase == "blank" { 2 } else { 3 });
                assert_eq!(w.tabs.items.iter().filter(|tab| tab.document.is_none()).count(), 1);
                assert_eq!(entries.len(), 3);
            }
            "second" => {
                assert_eq!(current.as_ref(), Some(b));
                assert_eq!(count, 2);
                let parked = w.tabs.get(a).unwrap().parked.as_ref().unwrap();
                assert_eq!(parked.top.map(|(ix, _)| ix), Some(30));
                assert_eq!(parked.core.doc.blocks[34].text.text, "Tab A line 34 edited");
                assert_eq!(store.load(a).unwrap().blocks[34].text.text, "Tab A line 34 edited");
            }
            "undo" => {
                assert_eq!(current.as_ref(), Some(a));
                let e = w.editor.as_ref().unwrap().read(cx);
                assert_eq!(e.scroll_top().map(|(ix, _)| ix), Some(30));
                assert_eq!(e.core.sel.head.block, e.core.doc.blocks[34].id);
                assert_eq!(e.core.sel.head.offset, "Tab A line 34 edited".len());
                w.editor.as_ref().unwrap().update(cx, |ed, cx| {
                    assert!(ed.core.undo());
                    assert_eq!(ed.core.doc.blocks[34].text.text, "Tab A line 34");
                    assert!(ed.core.redo());
                    ed.changed(window, cx);
                });
            }
            "peer" => {
                assert_eq!(current.as_ref(), Some(b));
                assert_eq!(w.tabs.get(a).unwrap().parked.as_ref().unwrap().core.doc.blocks[34].text.text, "Tab A line 34 edited from peer");
                assert!(w.open_target(a, true, window, cx));
                assert_eq!(w.tabs.items.len(), 2, "opening the same UUID must reuse its tab");
                let e = w.editor.as_ref().unwrap().read(cx);
                assert_eq!(e.core.doc.blocks[34].text.text, "Tab A line 34 edited from peer");
                assert_eq!(e.core.sel.head.block, e.core.doc.blocks[34].id);
            }
            "conflict-prepare" => {
                assert_eq!(current.as_ref(), Some(a));
                edit(w, window, cx, " unsaved");
                let mut disk = store.load(a).unwrap();
                disk.blocks[34].text = RichText::plain("external edit");
                std::fs::write(store.file_path(a).unwrap(), crate::docxml::serialize(&disk)).unwrap();
            }
            "conflict" => {
                assert_eq!(current.as_ref(), Some(a));
                assert_eq!(count, 2);
                assert!(w.unsaved && matches!(w.confirm, Some(Confirm::SaveFailed)));
                assert_eq!(store.load(a).unwrap().blocks[34].text.text, "external edit");
                w.confirm = None;
                w.resolve_save_failed(false, window, cx);
                assert_eq!(w.view, View::NewTab);
                assert!(w.tabs.selected_is_blank());
                assert_eq!(w.tabs.items.len(), 3);
                assert_eq!(w.tabs.get(a).unwrap().parked.as_ref().unwrap().core.doc.blocks[34].text.text, "external edit");
            }
            "replace" => {
                assert_eq!(current.as_ref(), Some(b));
                let mut disk = store.load(a).unwrap();
                disk.blocks[34].text = RichText::plain("modified while parked");
                std::fs::write(store.file_path(a).unwrap(), crate::docxml::serialize(&disk)).unwrap();
                assert!(w.try_open(c, window, cx));
                assert_eq!(w.tabs.items.len(), 2);
                assert!(w.tabs.get(a).is_some() && w.tabs.get(b).is_none());
            }
            "single" => {
                assert_eq!(current.as_ref(), Some(a));
                assert_eq!(count, 1);
                assert!(w.shows_document_tabs());
                assert_eq!(w.editor.as_ref().unwrap().read(cx).core.doc.blocks[34].text.text, "modified while parked");
            }
            "recent" => {
                assert_eq!(current.as_ref(), Some(b));
                assert_eq!(count, 2);
                assert!(w.tabs.get(a).is_some() && w.tabs.selected_document() == Some(b));
            }
            "home" => {
                assert_eq!(w.view, View::Home);
                assert!(current.is_none() && count == 0 && w.shows_document_tabs());
            }
            _ => panic!("unknown tabs check {phase}"),
        }
    }).unwrap();
    eprintln!("tabs: {phase} checked");
}

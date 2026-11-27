//! Library home, sorting and document rows.

use crate::app::{RowMenu, Workspace};
#[path = "home_scrollbar.rs"]
pub(crate) mod scrollbar;
use crate::settings::display_title;
use crate::search::doc_icon;
use crate::store::Entry;
use crate::theme::{self, Theme};
use crate::trash::{empty_state, list_header_text, list_line, menu_label, page_header, row_hover};
use crate::ui::{ROW_RADIUS, TipExt, icon, menu_glyph, menu_row, popover, ui_text};
use gpui::{prelude::FluentBuilder, *};
use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HomeTab {
    #[default]
    Recent,
    Starred,
}

/// The optional list columns (标题 is always shown).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Col {
    Modified,
    Created,
    Visited,
    Starred,
}

impl Col {
    fn label(self) -> &'static str {
        match self {
            Col::Modified => "修改时间",
            Col::Created => "创建时间",
            Col::Visited => "最近访问",
            Col::Starred => "收藏时间",
        }
    }
}

/// 最近访问's columns (no 所有者); 显示设置 toggles them.
const RECENT_COLS: [Col; 3] = [Col::Modified, Col::Created, Col::Visited];
const STARRED_COLS: [Col; 4] = [Col::Modified, Col::Created, Col::Visited, Col::Starred];
const LIST_ROW_H: f32 = 56.;

#[derive(Clone, Copy, PartialEq, Eq)]
enum HomeSort { Manual, Visited, Modified, Created, Title }
impl HomeSort {
    fn key(self) -> &'static str { match self { Self::Manual => "manual", Self::Visited => "visited", Self::Modified => "modified", Self::Created => "created", Self::Title => "title" } }
    fn of(key: &str) -> Self { match key { "manual" => Self::Manual, "modified" => Self::Modified, "created" => Self::Created, "title" => Self::Title, _ => Self::Visited } }
}
impl HomeSort {
    fn label(self) -> &'static str {
        match self {
            Self::Manual => crate::settings::tr("手动排序", "Manual order"),
            Self::Visited => "最近访问时间",
            Self::Modified => "最近修改时间",
            Self::Created => "创建时间（最新优先）",
            Self::Title => "标题（升序）",
        }
    }
}

pub struct Home {
    pub tab: HomeTab,
    list_scroll: ScrollHandle,
    list_scrollbar: Rc<scrollbar::Scrollbar>,
    /// The 搜索 dialog's results (`search.rs`).
    pub(crate) search_scrollbar: Rc<scrollbar::Scrollbar>,
    sort: HomeSort,
    pub sort_menu: bool,
    rows: Option<HomeRows>,
    /// Last-frame bounds of the controls the popovers hang from.
    anchors: Rc<RefCell<HashMap<&'static str, Bounds<Pixels>>>>,
    pub(crate) file_drop: Option<crate::file_drop::FileDrop>,
    /// Visible library document rows, used for file drops on the page.
    pub(crate) doc_bounds: Rc<RefCell<HashMap<String, Bounds<Pixels>>>>,
}

struct HomeRows {
    key: (HomeTab, HomeSort, u64),
    indices: Rc<Vec<usize>>,
}

impl Default for Home {
    fn default() -> Self {
        Home {
            tab: HomeTab::Recent,
            list_scroll: Default::default(),
            list_scrollbar: Default::default(),
            search_scrollbar: Default::default(),
            // Manual: the sidebar's order (`Meta::place`), so both lists read the same.
            sort: HomeSort::Manual,
            sort_menu: false,
            rows: None,
            anchors: Default::default(),
            file_drop: None,
            doc_bounds: Default::default(),
        }
    }
}

impl Home {
    pub(crate) fn anchor(&self, key: &str) -> Option<Bounds<Pixels>> {
        self.anchors.borrow().get(key).copied()
    }

    /// Records the element's bounds under `key` each frame.
    pub(crate) fn track(&self, key: &'static str) -> impl IntoElement {
        let rec = self.anchors.clone();
        canvas(move |b, _, _| {
            rec.borrow_mut().insert(key, b);
        }, |_, _, _, _| {})
        .absolute()
        .inset_0()
    }

    fn cols(&self) -> Vec<Col> {
        match self.tab {
            HomeTab::Recent => RECENT_COLS.to_vec(),
            HomeTab::Starred => STARRED_COLS.to_vec(),
        }
    }

    fn sort_col(&self) -> Col {
        match self.sort {
            HomeSort::Visited => Col::Visited,
            HomeSort::Modified => Col::Modified,
            HomeSort::Created => Col::Created,
            HomeSort::Title | HomeSort::Manual => Col::Visited,
        }
    }
}

fn visited(e: &Entry) -> u64 {
    if e.meta.opened > 0 { e.meta.opened } else { e.meta.modified }
}

fn cell_text(e: &Entry, c: Col) -> String {
    match c {
        Col::Modified => crate::ui::format_visit(e.meta.modified),
        Col::Created => crate::ui::format_visit(e.meta.created),
        Col::Visited => crate::ui::format_visit(visited(e)),
        Col::Starred => crate::ui::format_visit(e.meta.starred),
    }
}

/// A document's icon: its emoji, or the monochrome page glyph.
pub(crate) fn entry_icon(e: &Entry) -> AnyElement {
    match &e.icon {
        _ if e.meta.markdown => crate::search::doc_icon_named("doc-md").mr(px(12.)).into_any_element(),
        Some(em) => div().size(px(24.)).mr(px(12.)).flex_none().flex().items_center().justify_center().text_size(px(18.)).child(em.clone()).into_any_element(),
        None => doc_icon().mr(px(12.)).into_any_element(),
    }
}

#[derive(Clone)]
struct HomeRowDrag { id: String, title: String, markdown: bool }

impl Render for HomeRowDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().h(px(32.)).w(px(220.)).px(px(8.)).flex().items_center().gap(px(8.)).rounded(px(ROW_RADIUS))
            .bg(theme::raised(Theme::bg_body())).shadow(theme::shadow_menu())
            .child(icon(if self.markdown { "doc-md" } else { "doc" }, 16., Theme::icon()))
            .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(13.)).child(ui_text(self.title.clone())))
    }
}

impl Workspace {
    /// In 批量管理 a click checks or unchecks; otherwise it opens.
    fn home_click(&mut self, id: &str, modifiers: Modifiers, window: &mut Window, cx: &mut Context<Self>) {
        if self.batch.is_none() { return self.open_from_click(id, modifiers, window, cx) }
        if !self.toggle_check(id) {
            self.show_toast_kind(crate::ui::ToastKind::Error, &crate::localized_format!("一次最多选择 {} 项", "Select up to {} items at a time", crate::sidebar::BATCH_LIMIT), cx);
        }
        cx.notify();
    }

    /// The row checkbox while 批量管理 is on.
    pub(crate) fn home_check(&self, e: &Entry) -> Option<Div> {
        use crate::sidebar::{BATCH_LIMIT, Check, checkbox};
        let sel = self.batch.as_ref()?;
        let on = sel.contains(&e.id);
        Some(checkbox(if on { Check::On } else { Check::Off }, !on && sel.len() >= BATCH_LIMIT, "home-row"))
    }

    fn home_rows(&mut self) -> Rc<Vec<usize>> {
        let eligible = |e: &Entry| match self.home.tab {
            HomeTab::Recent => e.meta.parent.is_none(),
            HomeTab::Starred => e.meta.starred > 0,
        };
        let mut stamp = rustc_hash::FxHasher::default();
        for (i, e) in self.entries.iter().enumerate().filter(|(_, e)| eligible(e)) {
            (i, &e.id).hash(&mut stamp);
            match self.home.sort {
                HomeSort::Manual => e.meta.place().hash(&mut stamp),
                HomeSort::Visited => visited(e).hash(&mut stamp),
                HomeSort::Modified => e.meta.modified.hash(&mut stamp),
                HomeSort::Created => e.meta.created.hash(&mut stamp),
                HomeSort::Title => e.title.hash(&mut stamp),
            }
        }
        let key = (self.home.tab, self.home.sort, stamp.finish());
        if let Some(rows) = &self.home.rows && rows.key == key { return rows.indices.clone(); }
        let mut indices: Vec<_> = self.entries.iter().enumerate().filter_map(|(i, e)| eligible(e).then_some(i)).collect();
        indices.sort_by(|&a, &b| self.compare_home_entries(&self.entries[a], &self.entries[b]));
        let indices = Rc::new(indices);
        self.home.rows = Some(HomeRows { key, indices: indices.clone() });
        indices
    }

    pub(crate) fn prepare_manual_order(&mut self, cx: &mut Context<Self>) -> bool {
        if self.tree_writes_pending() { return false; }
        let previous = self.store.track_file_changes();
        if let Err(e) = self.store.initialize_manual_order() { self.show_toast_kind(crate::ui::ToastKind::Error, &e.to_string(), cx); return false; }
        self.entries = self.store.list();
        self.remember_files(previous, crate::settings::tr("初始化手动排序", "Initialize manual order"));
        true
    }

    fn compare_home_entries(&self, a: &Entry, b: &Entry) -> std::cmp::Ordering {
        let order = match self.home.sort {
            // The sidebar's order: new pages at the top in both.
            HomeSort::Manual => (a.meta.place(), &a.id).cmp(&(b.meta.place(), &b.id)),
            HomeSort::Visited => visited(b).cmp(&visited(a)),
            HomeSort::Modified => b.meta.modified.cmp(&a.meta.modified),
            HomeSort::Created => b.meta.created.cmp(&a.meta.created),
            HomeSort::Title => a.title.cmp(&b.title),
        };
        order.then_with(|| a.id.cmp(&b.id))
    }

    fn open_row_menu(&mut self, id: &str, key: &str, cx: &mut Context<Self>) {
        let b = self.more_bounds.borrow().get(key).copied();
        let Some(b) = b else { return };
        self.row_menu = Some(RowMenu { id: id.to_string(), pos: point(b.right(), b.bottom() + px(2.)), corner: Corner::TopRight, sub: false, batch: false });
        cx.notify();
    }

    fn home_more(&self, e: &Entry, key: String, cx: &mut Context<Self>) -> Stateful<Div> {
        let id = e.id.clone();
        let (rec, k2) = (self.more_bounds.clone(), key.clone());
        let open = self.row_menu.as_ref().is_some_and(|m| m.id == e.id && m.pos.x > px(0.) && m.corner == Corner::TopRight);
        div()
            .id(SharedString::from(key.clone()))
            .relative()
            .size(px(24.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .cursor_pointer()
            .hover(|s| s.bg(Theme::fill_hover_strong()))
            .when(open, |d| d.bg(Theme::fill_hover_strong()))
            .child(icon("ellipsis", 16., Theme::icon()))
            .child(canvas(move |b, _, _| {
                rec.borrow_mut().insert(key.clone(), b);
            }, |_, _, _, _| {}).absolute().inset_0())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |w, _, _, cx| {
                cx.stop_propagation();
                w.open_row_menu(&id, &k2, cx);
            }))
    }

    pub(crate) fn home_row(&self, e: &Entry, key: &'static str, cx: &mut Context<Self>) -> Stateful<Div> {
        let (id, menu_id) = (e.id.clone(), e.id.clone());
        let row = div().id(SharedString::from(format!("{key}-{}", e.id))).relative().flex().items_center().cursor_pointer().group("home-row")
            .on_click(cx.listener(move |w, ev: &ClickEvent, window, cx| w.home_click(&id, ev.modifiers(), window, cx)))
            .when(self.batch.is_none(), |d| d.on_mouse_down(MouseButton::Right, cx.listener(move |w, ev: &MouseDownEvent, _, cx| {
                w.row_menu = Some(RowMenu { id: menu_id.clone(), pos: ev.position, corner: Corner::TopLeft, sub: false, batch: false });
                cx.notify();
            })));
        if e.meta.local() { return row; }
        let bounds = self.home.doc_bounds.clone();
        let (id, target) = (e.id.clone(), e.id.clone());
        row.child(canvas(move |b, window, _| {
            let mask = window.content_mask().bounds;
            if b.intersects(&mask) { bounds.borrow_mut().insert(id.clone(), b.intersect(&mask)); }
        }, |_, _, _, _| {}).absolute().inset_0())
            .when(self.batch.is_none(), |d| d
                .on_drag(HomeRowDrag { id: e.id.clone(), title: display_title(&e.title), markdown: e.meta.markdown }, |drag, _, _, cx| cx.new(|_| drag.clone()))
                .drag_over::<HomeRowDrag>(|s, _, _, _| s.bg(Theme::fill_hover_strong()))
                .on_drop(cx.listener(move |w, drag: &HomeRowDrag, _, cx| w.move_into_document(&drag.id, &target, cx))))
    }

    pub(crate) fn home_row_actions(&self, e: &Entry, cx: &mut Context<Self>) -> Div {
        div().flex_none().flex().items_center()
            .when(self.batch.is_none(), |d| d
                .when(!e.meta.local(), |d| {
                    let parent = e.id.clone();
                    d.child(div().id(SharedString::from(format!("row-new-{}", e.id))).size(px(24.)).rounded(px(6.)).flex().items_center().justify_center().cursor_pointer()
                        .hover(|s| s.bg(Theme::fill_hover_strong())).tip(crate::settings::tr("新建子文档", "New child document"), None)
                        .child(icon("plus", 15., Theme::icon()))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |w, _, window, cx| { cx.stop_propagation(); w.new_child(&parent, window, cx); })))
                })
                .child(self.home_more(e, format!("home-more-{}", e.id), cx)))
    }

    /// 主页's sort order is remembered (`settings.json`).
    pub(crate) fn load_home(&mut self) {
        let s = self.store.settings();
        self.home.sort = s.home_sort.as_deref().map(HomeSort::of).unwrap_or(HomeSort::Manual);
    }

    fn save_home(&mut self) {
        let mut s = self.store.settings();
        s.home_sort = Some(self.home.sort.key().to_string());
        if let Err(e) = self.store.save_settings(&s) { eprintln!("无法保存主页排序：{e}"); }
    }

    pub(crate) fn render_home(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let indices = self.home_rows();
        // Too narrow for the search box: a search icon beside +, in the same style.
        let search = self.header_search(window, cx);
        let narrow = search.is_none();
        let header = page_header(self, crate::settings::text("主页"), 26., cx).relative().children(search)
            .when(narrow, |d| d.child(crate::ui::header_icon("home-search", "search").mr(px(4.))
                .tip(crate::settings::tr("搜索文档", "Search documents"), Some(crate::shortcuts::hint("workspace::FocusSearch", "⌘ + J")))
                .on_click(cx.listener(|w, _, window, cx| w.open_search(window, cx)))))
            .child(self.place_button("home-more", cx))
            .child(self.import_button("home-import", None, cx))
            .child(
            crate::ui::header_icon("home-new", "plus")
                .child(self.home.track("create-home"))
                .when(self.create_menu.is_some(), |d| d.bg(Theme::fill_hover()))
                .tip(crate::settings::tr("新建", "New"), Some(crate::shortcuts::hint("workspace::NewDoc", "⌘ + N")))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|w, ev: &ClickEvent, _, cx| { let pos = w.home.anchor("create-home").map_or(ev.position(), |b| point(b.left(), b.bottom() + px(4.))); w.create_menu = if w.create_menu.is_some() { None } else { Some((None, pos)) }; cx.notify(); })).into_any_element()
        );
        // Tabs as segmented items: the current one on a quiet fill, no accent.
        let tab = |id: &'static str, label: &'static str, t: HomeTab| {
            let on = self.home.tab == t;
            div()
                .id(id)
                .mr(px(8.))
                .h(px(28.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded(px(9.))
                .cursor_pointer()
                .text_size(px(13.))
                .line_height(px(20.))
                .when(on, |d| d.bg(Theme::fill_hover_strong()).text_color(Theme::text()))
                .when(!on, |d| d.text_color(Theme::text_caption()).hover(|s| s.bg(Theme::fill_hover())))
                .child(crate::settings::label(label))
                .on_click(cx.listener(move |w, _, _, cx| {
                    w.home.tab = t;
                    w.home.sort_menu = false;
                    cx.notify();
                }))
        };
        // 批量管理: the sidebar's 批量操作 selection, entered and shown here
        // too, with a toolbar of its own.
        let text_btn = |id: &'static str, ic: &'static str, label: &'static str, enabled: bool| {
            div().id(id).flex_none().h(px(28.)).px(px(10.)).flex().items_center().gap(px(6.)).rounded(px(9.))
                .text_size(px(13.)).line_height(px(20.))
                .text_color(if enabled { Theme::text() } else { Theme::text_disabled() })
                .when(enabled, |d| d.cursor_pointer().hover(|s| s.bg(Theme::fill_hover())))
                .child(icon(ic, 14., if enabled { Theme::icon() } else { Theme::text_disabled() }))
                .child(label)
        };
        let toolbar = div()
            .mt(px(12.))
            .h(px(28.))
            .flex_none()
            .pl(px(24.))
            .pr(px(24.))
            .flex()
            .items_center();
        let toolbar = if let Some(sel) = &self.batch {
            let ids: Vec<String> = indices.iter().take(crate::sidebar::BATCH_LIMIT).map(|&i| self.entries[i].id.clone()).collect();
            let n = sel.len();
            let all = n > 0 && ids.iter().all(|id| sel.contains(id));
            use crate::sidebar::{Check, checkbox};
            toolbar
                .child(div().id("home-batch-all").flex_none().ml(px(10.)).size(px(14.)).cursor_pointer().group("home-batch-all")
                    .child(checkbox(if all { Check::On } else if n > 0 { Check::Partial } else { Check::Off }, false, "home-batch-all"))
                    .tip(crate::settings::text("全选"), None)
                    .on_click(cx.listener(move |w, _, _, cx| { w.batch = Some(if all { vec![] } else { ids.clone() }); cx.notify(); })))
                .child(div().ml(px(10.)).text_size(px(13.)).line_height(px(20.)).text_color(Theme::text()).child(crate::localized_format!("已选 {n} 项", "Selected: {n}")))
                .child(div().flex_1())
                // The sidebar's 批量操作 actions: 创建副本 | 移动到 | 删除.
                .child(text_btn("home-batch-copy", "copy", crate::settings::text("创建副本"), n > 0)
                    .when(n > 0, |d| d.on_click(cx.listener(|w, _, _, cx| w.batch_copy(cx)))))
                .child(text_btn("home-batch-move", "folder-input", crate::settings::text("移动到"), n > 0).ml(px(4.))
                    .when(n > 0, |d| d.on_click(cx.listener(|w, _, window, cx| w.batch_move(window, cx)))))
                .child(text_btn("home-batch-delete", "bin", crate::settings::text("删除"), n > 0).ml(px(4.))
                    .when(n > 0, |d| d.on_click(cx.listener(|w, _, _, cx| w.ask_batch_delete(cx)))))
                .child(text_btn("home-batch-exit", "x", crate::settings::text("退出"), true).ml(px(8.))
                    .tip(crate::settings::text("退出批量操作"), None)
                    .on_click(cx.listener(|w, _, _, cx| { w.batch = None; w.row_menu = None; cx.notify(); })))
        } else { toolbar
            .child(tab("home-tab-recent", crate::settings::text("我的文档"), HomeTab::Recent))
            .child(tab("home-tab-star", crate::settings::text("收藏"), HomeTab::Starred))
            .child(div().flex_1())
            .child(text_btn("home-batch", "batch", crate::settings::text("批量管理"), true).text_color(Theme::text_caption()).mr(px(8.))
                .on_click(cx.listener(|w, _, _, cx| { w.batch = Some(vec![]); w.row_menu = None; w.home.sort_menu = false; cx.notify(); })))
            .child(
                div().id("home-sort").relative().h(px(28.)).px(px(10.)).flex().items_center().gap(px(6.))
                    .rounded(px(9.)).text_size(px(13.)).text_color(Theme::text_caption()).cursor_pointer()
                    .hover(|s| s.bg(Theme::fill_hover())).when(self.home.sort_menu, |d| d.bg(Theme::fill_hover_strong()))
                    .child(icon("sort-down", 14., Theme::icon())).child(crate::settings::label(self.home.sort.label()))
                    .child(self.home.track("home-sort"))
                    .on_click(cx.listener(|w, _, _, cx| { w.home.sort_menu = !w.home.sort_menu; cx.notify(); })),
            ) };

        let body = if indices.is_empty() && self.home.tab == HomeTab::Recent {
            empty_state("illus/empty-list.svg", crate::settings::text("当前列表暂无内容")).into_any_element()
        } else {
            self.render_home_list(indices, window, cx)
        };
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(header)
            .children(self.render_document_tabs(window, cx))
            .child(toolbar)
            .child(body)
            .into_any_element()
    }

    fn render_home_list(&self, indices: Rc<Vec<usize>>, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let list_scroll = &self.home.list_scroll;
        let cols = self.home.cols();
        let sort = self.home.sort_col();
        let pane_width = f32::from(window.viewport_size().width) - if self.sidebar_open { self.sidebar_w(window) } else { 0. };
        let compact = pane_width < 700.;
        let date_inset = px(if compact { 8. } else { 24. });
        let title_cell = move || div().flex_shrink().flex_basis(relative(0.)).min_w_0().map(|mut d| {
            d.style().flex_grow = Some(if compact { 2. } else { 3. });
            d
        });
        let col_cell = || div().flex_grow().flex_shrink().flex_basis(relative(0.)).max_w(px(240.)).min_w_0();
        // The list: 13pt tertiary headers over a hairline; the
        // sorted column in the text colour, semibold, with its arrow.
        let mut header = div()
            .h(px(33.))
            .mt(px(10.))
            .flex_none()
            .flex()
            .items_center()
            .border_b_1()
            .border_color(list_line())
            .child(
                title_cell().pl(px(20.)).child(
                    div().id("home-col-title").px(px(4.)).text_size(px(13.)).line_height(px(24.)).text_color(list_header_text()).child(crate::settings::text("标题")),
                ),
            );
        for c in &cols {
            let sorted = !matches!(self.home.sort, HomeSort::Title | HomeSort::Manual) && *c == sort;
            header = header.child(
                col_cell().pl(date_inset - px(4.)).overflow_hidden().child(
                    div()
                        .id(SharedString::from(format!("home-col-{}", c.label())))
                        .relative()
                        .px(px(4.))
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .rounded(px(ROW_RADIUS))
                        .text_size(px(13.))
                        .line_height(px(24.))
                        .text_color(if sorted { Theme::text() } else { list_header_text() })
                        .when(sorted, |d| d.font_weight(FontWeight::SEMIBOLD))
                        .hover(|s| s.bg(Theme::fill_hover()))
                        .when(sorted && self.home.sort_menu, |d| d.bg(Theme::fill_hover_strong()))
                        .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(ui_text(crate::settings::label(c.label()))))
                        .when(sorted, |d| {
                            d.child(icon("sort-down", 12., Theme::icon()))
                                .cursor_pointer()
                                .on_click(cx.listener(|w, _, _, cx| {
                                    w.home.sort_menu = !w.home.sort_menu;
                                    cx.notify();
                                }))
                        }),
                ),
            );
        }
        header = header.child(div().w(px(60.)).flex_none());
        let mut rows = div().id("home-rows").flex_1().min_h_0().overflow_y_scroll().track_scroll(list_scroll).px(px(24.)).pb(px(24.));
        if indices.is_empty() {
            rows = rows.child(div().pt(px(72.)).child(empty_state("illus/empty-list.svg", crate::settings::text("当前列表暂无内容"))));
        } else {
            let height = px(indices.len() as f32 * LIST_ROW_H);
            let workspace = cx.entity();
            rows = rows.child(canvas(move |bounds, window, cx| workspace.update(cx, |w, cx| {
                // Use the clamped scroll position from this frame, including thumb jumps and resizes.
                let viewport = window.content_mask().bounds;
                let first = (f32::from(viewport.top() - bounds.top()) / LIST_ROW_H).floor() as usize;
                let end = ((f32::from(viewport.bottom() - bounds.top()) / LIST_ROW_H).ceil() as usize).min(indices.len());
                let mut visible = div().size_full();
                for i in first..end {
                    let e = &w.entries[indices[i]];
                    let menu_on = w.row_menu.as_ref().is_some_and(|m| m.id == e.id);
                    let check = w.home_check(e);
                    let mut row = w.home_row(e, "home-row", cx)
                        .absolute().left_0().right_0().top(px(i as f32 * LIST_ROW_H)).h(px(LIST_ROW_H))
                        .when(i > 0, |d| d.border_t_1().border_color(list_line()))
                        .hover(row_hover)
                        .when(menu_on, |d| d.bg(Theme::fill_hover()).border_color(transparent_black()).rounded(px(ROW_RADIUS)))
                        .when(w.file_drop_over(&e.id), |d| d.bg(Theme::fill_hover_strong()).rounded(px(ROW_RADIUS)))
                        .child(
                            title_cell().pl(px(24.)).pr(px(16.)).flex().items_center()
                                .when_some(check, |d, c| d.child(c.mr(px(12.))))
                                .child(entry_icon(e))
                                .child(div().min_w_0().text_size(px(15.)).line_height(px(22.)).overflow_hidden().whitespace_nowrap().text_ellipsis().child(ui_text(display_title(&e.title))))
                                .when(e.meta.pinned, |d| d.child(icon("pin-filled", 14., Theme::pinned()).ml(px(6.)))),
                        );
                    for c in &cols {
                        row = row.child(col_cell().pl(date_inset).text_size(px(13.)).text_color(Theme::text_caption()).whitespace_nowrap().overflow_hidden().text_ellipsis().child(ui_text(cell_text(e, *c))));
                    }
                    row = row.child(div().w(px(60.)).flex_none().flex().justify_end().pr(px(12.)).child(w.home_row_actions(e, cx)));
                    visible = visible.child(row);
                }
                let mut element = visible.into_any_element();
                element.prepaint_as_root(bounds.origin, bounds.size.map(AvailableSpace::Definite), window, cx);
                element
            }), |_, mut element, window, cx| element.paint(window, cx)).flex_none().w_full().h(height));
        }
        div().flex_1().min_h_0().flex().flex_col().child(header.mx(px(24.)))
            .child(div().relative().flex_1().min_h_0().flex().flex_col().child(rows).child(self.home.list_scrollbar.render(list_scroll))).into_any_element()
    }

    /// 显示设置 and the sort dropdown: the menu panel; the sort item is a
    /// native menu row.
    pub(crate) fn render_home_menus(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = vec![];
        if self.view != crate::app::View::Home {
            return out;
        }
        let place = |pos: Point<Pixels>, corner: Corner, menu: Stateful<Div>| {
            deferred(anchored().anchor(corner).position(pos).snap_to_window_with_margin(px(8.)).child(menu)).with_priority(2).into_any_element()
        };
        if self.home.sort_menu
            && let Some(b) = self.home.anchor("home-sort") {
                let mut menu = popover().id("sort-menu").occlude().w(px(200.)).py(px(crate::ui::MENU_PAD_Y))
                    .on_mouse_down_out(cx.listener(|w, _, _, cx| { w.home.sort_menu = false; cx.notify(); }));
                for (index, order) in [HomeSort::Manual, HomeSort::Visited, HomeSort::Modified, HomeSort::Created, HomeSort::Title].into_iter().enumerate() {
                    let g = SharedString::from(format!("home-order-{index}"));
                    menu = menu.child(menu_row(g.clone(), g.clone()).when(self.home.sort == HomeSort::Manual, |d| d.opacity(0.4))
                        .child(menu_label(order.label(), g.clone(), gpui::white()).flex_1())
                        .when(self.home.sort == order, |d| d.child(menu_glyph("check", 14., g)))
                        .on_click(cx.listener(move |w, _, _, cx| {
                            if order == HomeSort::Manual && !w.prepare_manual_order(cx) { return; }
                            w.home.sort = order;
                            w.save_home();
                            w.home.sort_menu = false;
                            w.home.list_scroll.set_offset(point(px(0.), px(0.)));
                            cx.notify();
                        })));
                }
                out.push(place(point(b.right(), b.bottom() + px(4.)), Corner::TopRight, menu));
            }
        out
    }
}

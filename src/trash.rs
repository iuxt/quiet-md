//! 回收站 page and the confirmation dialogs (删除, 恢复).
//! Deleted pages stay in the library until restored; each row includes
//! the child pages that went with it.

use crate::app::{Confirm, Workspace};
use crate::settings::display_title;
use crate::search::doc_icon_named;
use crate::store::TrashEntry;
pub(crate) use crate::ui::{close_button, dialog, list_header_text, list_line, menu_label, primary_button, row_hover, scrim, secondary_button};
use crate::theme::{self, Theme, craft};
use crate::ui::{ROW_RADIUS, TipExt, TitleBarExt, icon, popover, ui_para, ui_text};
use gpui::{prelude::FluentBuilder, *};

pub const TRASH_MENU_W: f32 = 160.;

/// The page header, as tall as the document's so the header pills sit in
/// the same place: a 17pt semibold view title, and the open-sidebar
/// button when the sidebar is hidden.
pub(crate) fn page_header(ws: &Workspace, title: &'static str, left: f32, cx: &mut Context<Workspace>) -> Stateful<Div> {
    page_header_with(ws, div().text_size(px(17.)).line_height(px(24.)).font_weight(FontWeight::SEMIBOLD).child(crate::settings::label(title)), left, cx)
}

/// `page_header` with any title element, including breadcrumbs.
pub(crate) fn page_header_with(ws: &Workspace, title: impl IntoElement, left: f32, cx: &mut Context<Workspace>) -> Stateful<Div> {
    div()
        .id("page-header")
        .h(px(crate::app::HEADER_H))
        .flex_none()
        .flex()
        .items_center()
        // Clear of the traffic lights, like the document header.
        .pl(if ws.sidebar_open { px(left) } else { px(theme::TRAFFIC_LIGHTS_W) })
        .pr(px(crate::ui::header_right_inset()))
        .title_bar()
        .when(!ws.sidebar_open && cfg!(target_os = "windows"), |d| d.child(div().mr(px(12.)).child(ws.header_navigation(cx))))
        .when(!ws.sidebar_open && !cfg!(target_os = "windows"), |d| {
            d.child(
                div()
                    .id("open-sidebar")
                    .no_drag()
                    .flex_none()
                    .ml(px(4.))
                    .mr(px(12.))
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .hover(|s| s.bg(Theme::fill_hover_strong()))
                    .tip(crate::settings::tr("展开侧栏", "Show sidebar"), Some(crate::shortcuts::hint("workspace::ToggleSidebar", "⌘ + ]")))
                    .child(icon("panel-left-open", 17., Theme::icon()))
                    .on_click(cx.listener(|w, _, _, cx| {
                        w.sidebar_open = true;
                        cx.notify();
                    })),
            )
        })
        .child(title)
        .child(div().flex_1())
}

/// Centred empty-state illustration and caption.
pub(crate) fn empty_state(illus: &'static str, text: &'static str) -> Div {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .pb(px(64.))
        .child(img(illus).size(px(120.)))
        .child(div().mt(px(16.)).text_size(px(13.)).line_height(px(20.)).text_color(Theme::text_caption()).child(text))
}

impl Workspace {
    fn trash_breadcrumbs(&self, parent: Option<&str>, cx: &mut Context<Self>) -> Div {
        let mut chain = vec![];
        let mut parent = parent.map(str::to_string);
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = parent {
            if !seen.insert(id.clone()) { break }
            let Some(meta) = self.store.trash_meta(&id) else { break };
            let name = display_title(meta.name.as_deref().unwrap_or_default());
            chain.push((id, name)); parent = meta.parent;
        }
        let mut crumbs = div().min_w_0().flex().items_center().gap(px(6.))
            .child(div().id("trash-root").no_drag().cursor_pointer().text_size(px(13.)).text_color(Theme::text_caption())
                .hover(|s| s.text_color(Theme::primary())).child(crate::settings::text("回收站"))
                .on_click(cx.listener(|w, _, _, cx| { w.show_view(crate::app::View::Trash, cx); })));
        for (id, title) in chain.into_iter().rev() {
            let key = SharedString::from(format!("trash-crumb-{id}"));
            crumbs = crumbs.child(icon("chevron-right", 12., Theme::icon()))
                .child(div().id(key).no_drag().min_w_0().truncate().cursor_pointer().text_size(px(13.))
                    .text_color(Theme::text_caption()).hover(|s| s.text_color(Theme::primary())).child(ui_text(title))
                    .on_click(cx.listener(move |w, _, window, cx| w.open(&id, window, cx))));
        }
        crumbs
    }

    pub(crate) fn trash_document_header(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let parent = self.current.as_ref().and_then(|id| self.store.trash_meta(id)).and_then(|m| m.parent);
        let title = self.editor.as_ref().map(|e| display_title(&e.read(cx).core.doc.title.text)).unwrap_or_default();
        let crumbs = self.trash_breadcrumbs(parent.as_deref(), cx)
            .child(icon("chevron-right", 12., Theme::icon()))
            .child(div().min_w_0().truncate().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(ui_text(title)));
        page_header_with(self, crumbs, 18., cx)
            .child(crate::ui::header_icon("trash-history", "restore").tip(crate::settings::tr("版本历史", "Version history"), None)
                .on_click(cx.listener(|w, _, window, cx| {
                    if let Some(id) = w.current.clone() { w.open_history(&id, window, cx); }
                })))
            .child(secondary_button("trash-restore", crate::settings::text("恢复")).ml(px(8.)).no_drag()
                .on_click(cx.listener(|w, _, _, cx| { w.confirm = w.current.clone().map(Confirm::Restore); cx.notify(); })))
    }

    pub(crate) fn render_trash_banner(&self, _cx: &mut Context<Self>) -> Option<AnyElement> {
        (self.trash_document && self.preview.is_none()).then(|| div().flex_none().px(px(18.)).py(px(8.))
            .text_size(px(12.)).text_color(Theme::text_caption()).bg(Theme::fill_hover())
            .child(crate::settings::tr("此文档在回收站中，恢复后可编辑", "This document is in Trash. Restore it to edit."))
            .into_any_element())
    }

    /// Where a trashed page goes back to: its parent if that page is live.
    fn trash_location(&self, t: &TrashEntry) -> String {
        t.parent
            .as_ref()
            .and_then(|p| self.entries.iter().find(|e| &e.id == p))
            .map(|e| display_title(&e.title))
            .or_else(|| t.parent.as_ref().and_then(|p| self.store.trash_meta(p)).and_then(|m| m.name).map(|title| display_title(&title)))
            .unwrap_or_else(|| crate::settings::label("我的文档"))
    }

    pub(crate) fn render_trash(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let header = page_header(self, "回收站", 26., cx);
        let header = header.child(
            crate::ui::header_icon("trash-search", "search")
                .tip_label(crate::shortcuts::keys(crate::settings::text("搜索(⌘ + J)")).into(), crate::ui::TipSide::Below)
                .on_click(cx.listener(|_, _, window, cx| window.dispatch_action(Box::new(crate::app::FocusSearch), cx))),
        );
        let mut page = div().flex_1().min_w_0().h_full().flex().flex_col().child(header);
        if self.trash.is_empty() {
            return page.child(empty_state("illus/empty-trash.svg", crate::settings::text("回收站暂无内容"))).into_any_element();
        }
        // The list: 13pt tertiary headers over a hairline,
        // 15pt titles, 13pt secondary cells.
        let col = |w: Option<f32>, label: &'static str| {
            div()
                .when_some(w, |d, w| d.w(px(w)).flex_none())
                .when(w.is_none(), |d| d.flex_1().min_w_0())
                .pl(px(20.))
                .text_size(px(13.))
                .line_height(px(24.))
                .text_color(list_header_text())
                .child(crate::settings::label(label))
        };
        let table_header = div()
            .h(px(33.))
            .flex_none()
            .mx(px(24.))
            .flex()
            .items_center()
            .border_b_1()
            .border_color(list_line())
            .child(col(None, "标题"))
            .child(col(Some(168.), "位置"))
            .child(col(Some(168.), "删除时间"))
            .child(div().w(px(48.)).flex_none());
        let mut list = div().id("trash-list").flex_1().min_h_0().overflow_y_scroll().px(px(24.)).pb(px(24.));
        for (i, t) in self.trash.iter().enumerate() {
            let (id, id2) = (t.id.clone(), t.id.clone());
            let open_id = t.id.clone();
            let menu_open = self.trash_menu.as_ref().is_some_and(|(m, _, _)| *m == t.id);
            let title = display_title(&t.title);
            let more = div()
                .id(SharedString::from(format!("trash-more-{}", t.id)))
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .cursor_pointer()
                .hover(|s| s.bg(Theme::fill_hover_strong()))
                .when(menu_open, |d| d.bg(Theme::fill_hover_strong()))
                .child(icon("ellipsis", 16., Theme::icon()))
                .on_click(cx.listener(move |w, ev: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    let p = ev.position();
                    w.trash_menu = Some((id.clone(), point(p.x + px(12.), p.y + px(14.)), Corner::TopRight));
                    cx.notify();
                }));
            list = list.child(
                div()
                    .id(SharedString::from(format!("trash-row-{}", t.id)))
                    .h(px(56.))
                    .flex()
                    .items_center()
                    .when(i > 0, |d| d.border_t_1().border_color(list_line()))
                    .hover(row_hover)
                    .cursor_pointer()
                    .on_click(cx.listener(move |w, _, window, cx| w.open(&open_id, window, cx)))
                    .when(menu_open, |d| d.bg(Theme::fill_hover()).border_color(transparent_black()).rounded(px(ROW_RADIUS)))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |w, ev: &MouseDownEvent, _, cx| {
                            w.trash_menu = Some((id2.clone(), ev.position, Corner::TopLeft));
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .pl(px(24.))
                            .flex()
                            .items_center()
                            .child(match &t.icon {
                                Some(e) => div().size(px(24.)).mr(px(10.)).flex_none().flex().items_center().justify_center().text_size(px(18.)).child(e.clone()),
                                None => doc_icon_named(if t.meta.markdown { "doc-md" } else { "doc" }),
                            })
                            .child(div().min_w_0().text_size(px(15.)).line_height(px(22.)).overflow_hidden().whitespace_nowrap().text_ellipsis().child(ui_text(title))),
                    )
                    .child(
                        div()
                            .w(px(168.))
                            .flex_none()
                            .pl(px(16.))
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .child(icon("library", 16., Theme::icon()))
                            .child(div().min_w_0().text_size(px(13.)).text_color(Theme::text_caption()).overflow_hidden().whitespace_nowrap().text_ellipsis().child(ui_text(self.trash_location(t)))),
                    )
                    .child(div().w(px(168.)).flex_none().pl(px(20.)).text_size(px(13.)).text_color(Theme::text_caption()).child(crate::ui::format_time(t.deleted)))
                    .child(div().w(px(48.)).flex_none().flex().justify_end().pr(px(12.)).child(more)),
            );
        }
        page = page.child(table_header).child(list);
        page.into_any_element()
    }

    pub(crate) fn render_trash_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (id, pos, corner) = self.trash_menu.clone()?;
        let open_id = id.clone();
        let history_id = id.clone();
        // The context menu: 32pt rows on the quiet hover fill.
        let item = |key: &'static str, ic: &'static str, label: &'static str, color: Hsla| {
            let g = SharedString::from(key);
            crate::ui::menu_row(key, g.clone())
                .text_color(color)
                .child(icon(ic, 16., if color == Theme::icon() { Theme::icon() } else { color }))
                .child(menu_label(label, g, gpui::white()))
        };
        let menu = popover()
            .id("trash-menu")
            .occlude()
            .min_w(px(TRASH_MENU_W))
            .py(px(crate::ui::MENU_PAD_Y))
            .on_mouse_down_out(cx.listener(|w, _, _, cx| {
                w.trash_menu = None;
                cx.notify();
            }))
            .child(item("tm-open", "doc", crate::settings::tr("打开", "Open"), Theme::text()).on_click(cx.listener(move |w, _, window, cx| {
                w.trash_menu = None;
                w.open(&open_id, window, cx);
            })))
            .child(item("tm-history", "restore", crate::settings::tr("版本历史", "Version history"), Theme::text()).on_click(cx.listener(move |w, _, window, cx| {
                w.trash_menu = None;
                if w.try_open(&history_id, window, cx) { w.open_history(&history_id, window, cx); }
            })))
            .child(item("tm-restore", "restore", crate::settings::text("恢复"), Theme::text()).on_click(cx.listener(move |w, _, _, cx| {
                w.trash_menu = None;
                w.confirm = Some(Confirm::Restore(id.clone()));
                cx.notify();
            })));
        Some(
            deferred(anchored().anchor(corner).position(pos).snap_to_window_with_margin(px(8.)).child(menu))
                .with_priority(2)
                .into_any_element(),
        )
    }

    fn restore_doc(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let file_before = self.store.track_file_changes();
        match self.store.restore(id) {
            Ok(moved) => {
                self.show_toast(if moved { crate::settings::text("恢复成功。部分文档由于原位置不存在，已恢复到“我的文档”") } else { crate::settings::text("恢复成功") }, cx);
            }
            Err(e) => {
                let detail = e.chain().map(|cause| crate::settings::diagnostic(&cause.to_string())).collect::<Vec<_>>().join(": ");
                let msg = crate::localized_format!("恢复失败：{}", "Could not restore document: {}", detail);
                self.show_toast_kind(crate::ui::ToastKind::Error, &msg, cx);
                self.status = crate::app::SaveStatus::Failed(msg);
            }
        }
        self.entries = self.store.list();
        self.sort();
        self.sync_sub_pages(cx);
        self.trash = self.store.list_trash();
        if self.view == crate::app::View::Doc && self.trash_document && self.current.as_ref().is_some_and(|id| self.store.trash_meta(id).is_none())
            && let Some(current) = self.current.clone() { self.open(&current, window, cx); }
        self.show_store_warnings(cx);
        self.remember_files(file_before, crate::settings::tr("恢复", "Restore"));
    }

    fn show_store_warnings(&mut self, cx: &mut Context<Self>) {
        self.show_store_warnings_from(&self.store.take_warnings(), cx);
    }

    pub(crate) fn show_store_warnings_from(&mut self, warnings: &[String], cx: &mut Context<Self>) {
        if !warnings.is_empty() {
            let msg = warnings.iter().map(|s| crate::settings::diagnostic(s)).collect::<Vec<_>>().join(crate::settings::tr("；", "; "));
            self.show_toast_kind(crate::ui::ToastKind::Error, &msg, cx);
            self.status = crate::app::SaveStatus::Failed(msg);
        }
    }

    /// The confirmation dialog: a modal (white, radius 16), a 15pt
    /// semibold title, one line of 13pt body, 取消 and the action button on
    /// the right.
    pub(crate) fn render_confirm(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let c = self.confirm.as_ref()?;
        let title_of = |id: &str| {
            self.entries
                .iter()
                .find(|e| e.id == id)
                .map(|e| display_title(&e.title))
                .or_else(|| self.trash.iter().find(|t| t.id == id).map(|t| display_title(&t.title)))
                .unwrap_or_default()
        };
        let (title, body, action, danger) = match c {
            Confirm::Delete { ids, batch } => (
                if *batch || ids.len() > 1 { crate::localized_format!("是否删除所选的 {} 项？", "Delete the {} selected items?", ids.len()) } else { crate::localized_format!("是否删除： {}？", "Delete {}?", title_of(&ids[0])) },
                if self.entries.iter().any(|e| e.meta.parent.as_ref().is_some_and(|p| ids.contains(p))) {
                    crate::settings::tr("所选文档及其所有子文档将一起移入回收站，可随时恢复。", "Selected documents and all their descendants will move to Trash and can be restored.")
                } else { crate::settings::text("删除的内容将移入回收站并保留，可随时恢复") }.to_string(),
                crate::settings::text("删除"),
                true,
            ),
            Confirm::Restore(_) => (crate::settings::text("确认恢复？").to_string(), crate::settings::text("确认后将恢复至原位置").to_string(), crate::settings::text("恢复"), false),
            Confirm::GitDiscard(path) => (
                crate::settings::tr("丢弃这个文件的修改？", "Discard changes to this file?").to_string(),
                crate::localized_format!("将丢弃 {} 未暂存的修改，已暂存的内容会保留。新增文件会被删除，新增文章也会从清单移除。此操作无法撤销。", "Unstaged changes to {} will be discarded; staged content is kept. New files are deleted and new notes are removed from the manifest. This cannot be undone.", path),
                crate::settings::tr("丢弃修改", "Discard changes"), true,
            ),
            Confirm::Refs { show: false, .. } => (
                crate::settings::text("是否隐藏引用列表？").to_string(),
                crate::settings::text("选择隐藏后，协作者将无法查看本文的引用列表。如需重新显示，可点击文档右上角的更多按钮“...”，打开“显示引用列表”开关。").to_string(),
                crate::settings::text("隐藏"),
                false,
            ),
            Confirm::Refs { show: true, .. } => (crate::settings::text("是否显示引用列表？").to_string(), crate::settings::text("选择显示后，协作者可再次查看本文的引用列表。").to_string(), crate::settings::text("显示"), false),
            Confirm::SwitchLibrary(root) => (
                crate::settings::tr("换用这个文档库？", "Switch to this library?").to_string(),
                crate::localized_format!("Quiet.md 会保存当前文档并重启，然后打开 {}。原来的文档库留在原处，不会移动或删除，之后可以再换回来。", "Quiet.md saves the open document, restarts and opens {}. The current library stays where it is, nothing is moved or deleted, and you can switch back later.", root.display()),
                crate::settings::tr("切换并重启", "Switch and Restart"),
                false,
            ),
            Confirm::Import { what, target } => {
                let place = self.drop_place(target);
                let what_s = match what {
                    crate::app::DropImport::Local(id) => crate::localized_format!("「{}」", "“{}”", title_of(id)),
                    crate::app::DropImport::Files(p) if p.len() == 1 => crate::localized_format!("「{}」", "“{}”", p[0].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
                    crate::app::DropImport::Files(p) => crate::localized_format!("这 {} 项", "these {} items", p.len()),
                };
                (
                    crate::localized_format!("是否将{}导入到「{}」？", "Import {} into “{}”?", what_s, place),
                    crate::settings::tr("会复制一份到文档库里，原文件不会改动。", "A copy goes into the library; the original file is left as it is.").to_string(),
                    crate::settings::tr("导入", "Import"),
                    false,
                )
            }
            Confirm::SaveFailed => (
                crate::settings::text("文档保存失败").to_string(),
                crate::localized_format!("{}。可以重试或另存副本；重新载入将放弃未保存的修改并载入磁盘内容。", "{}. Retry or save a copy. Reload discards unsaved changes and loads the document from disk.", self.save_error()),
                crate::settings::text("重试"),
                false,
            ),
        };
        let no_ask = match c {
            Confirm::Refs { show: false, no_ask } => Some(*no_ask),
            _ => None,
        };
        let ok = primary_button("confirm-ok", action);
        let ok = if danger { ok.bg(Theme::danger()).hover(|s| s.bg(theme::c(craft::R600))).active(|s| s.bg(theme::c(craft::R700))) } else { ok };
        let dialog = dialog("confirm-dialog")
            .w(px(if matches!(c, Confirm::SaveFailed | Confirm::SwitchLibrary(_)) { 520. } else { 400. }))
            .px(px(24.))
            .pt(px(20.))
            .pb(px(24.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().text_size(px(15.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD).child(ui_para(title)))
            .child(div().mt(px(8.)).text_size(px(13.)).line_height(px(20.)).text_color(Theme::text()).child(ui_para(body)))
            .child(
                div()
                    .mt(px(24.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(div().flex_1().children(no_ask.map(|on| {
                        div()
                            .id("confirm-no-ask")
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .cursor_pointer()
                            .text_size(px(13.))
                            .on_click(cx.listener(|w, _, _, cx| {
                                if let Some(Confirm::Refs { no_ask, .. }) = &mut w.confirm {
                                    *no_ask = !*no_ask;
                                }
                                cx.notify();
                            }))
                            .child(crate::ui::checkbox(on))
                            .child(ui_text(crate::settings::text("不再提示")))
                    })))
                    .child(secondary_button("confirm-cancel", crate::settings::text("取消")).on_click(cx.listener(|w, _, _, cx| {
                        // 稍后 asks again in a week, not at every launch.
                        w.confirm = None;
                        w.after_save = None;
                        cx.notify();
                    })))
                    .when(matches!(c, Confirm::SaveFailed), |d| {
                        d.child(secondary_button("confirm-save-copy", crate::settings::tr("另存副本", "Save a copy")).on_click(cx.listener(|w, _, window, cx| {
                            w.save_failed_copy(window, cx);
                        })))
                        .child(secondary_button("confirm-discard", crate::settings::tr("重新载入", "Reload")).text_color(Theme::danger()).on_click(cx.listener(|w, _, window, cx| {
                            w.confirm = None;
                            w.resolve_save_failed(false, window, cx);
                        })))
                    })
                    .child(ok.on_click(cx.listener(|w, _, window, cx| {
                        let Some(c) = w.confirm.take() else { return };
                        match c {
                            Confirm::Delete { ids, batch } => {
                                if batch {
                                    w.batch = None;
                                }
                                w.delete_docs(&ids, window, cx);
                            }
                            Confirm::Restore(id) => w.restore_doc(&id, window, cx),
                            Confirm::GitDiscard(path) => w.git_action(crate::git::Action::Discard(path), window, cx),
                            Confirm::Refs { show, no_ask } => {
                                if no_ask {
                                    let mut s = w.store.settings();
                                    s.refs_hide_no_ask = true;
                                    let _ = w.store.save_settings(&s);
                                }
                                w.set_refs_hidden(!show, cx);
                            }
                            Confirm::SaveFailed => w.resolve_save_failed(true, window, cx),
                            Confirm::SwitchLibrary(root) => cx.defer(move |cx| crate::app::switch_library(root, cx)),
                            Confirm::Import { what, target } => w.import_dropped(what, target, window, cx),
                        }
                        cx.notify();
                    }))),
            );
        Some(
            deferred(
                div()
                    .id("confirm-mask")
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(scrim())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(dialog),
            )
            .with_priority(3)
            .into_any_element(),
        )
    }
}

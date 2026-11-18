//! Working-tree browser and read-only text / formatted block comparisons.
use crate::{app::{View, Workspace}, git::Action, settings::tr, theme::{self, Theme}, ui::{self, TipExt}};
use fdl_core::{diff::{Kind, Line}, git::{ChangePreview, LibraryRepo}, store::Store};
use gpui::{prelude::FluentBuilder, *};
use std::{cell::Cell, rc::Rc};

pub(super) struct GitPreview {
    title: String, binary: bool, text: DiffRows, source: Option<DiffRows>, formatted: Option<DiffRows>,
}

struct DiffRows {
    rows: Rc<Vec<DiffRow>>, state: ListState, counts: (usize, usize), truncated: bool,
    eqnos: Rc<std::collections::HashMap<crate::model::BlockId, u32>>,
}

struct DiffRow { kind: Kind, number: usize, text: SharedString, block: Option<crate::model::Block> }

impl GitPreview {
    fn new(preview: ChangePreview) -> Self {
        Self { title: preview.title, binary: preview.binary, text: DiffRows::new(preview.lines, preview.truncated),
            source: preview.source.map(|lines| DiffRows::new(lines, preview.source_truncated)),
            formatted: preview.formatted.map(DiffRows::formatted) }
    }
}

impl DiffRows {
    fn new(lines: Vec<Line>, truncated: bool) -> Self {
        let counts = fdl_core::diff::counts(&lines);
        let mut old = 0; let mut new = 0;
        let rows = lines.into_iter().map(|line| {
            if line.kind != Kind::Added { old += 1; }
            if line.kind != Kind::Removed { new += 1; }
            DiffRow { kind: line.kind, number: if line.kind == Kind::Removed { old } else { new }, text: line.text.into(), block: None }
        }).collect::<Vec<_>>();
        let state = ListState::new(rows.len(), ListAlignment::Top, px(100.));
        Self { rows: Rc::new(rows), state, counts, truncated, eqnos: Default::default() }
    }

    fn formatted(diff: fdl_core::diff::FormattedDiff) -> Self {
        let numbers = |exclude| {
            let blocks = diff.rows.iter().filter(|(kind, _)| *kind != exclude).map(|(_, b)| b.clone()).collect();
            crate::editor::equation_numbers(&crate::model::Document { blocks, ..Default::default() })
        };
        let mut eqnos = numbers(Kind::Added); eqnos.extend(numbers(Kind::Removed));
        let mut out = Self::new(diff.rows.iter().map(|(kind, _)| Line { kind: *kind, text: String::new() }).collect(), diff.truncated);
        for (row, (_, block)) in Rc::get_mut(&mut out.rows).unwrap().iter_mut().zip(diff.rows) { row.block = Some(block); }
        out.eqnos = Rc::new(eqnos);
        out
    }

    fn render(&self) -> impl IntoElement {
        let rows = self.rows.clone(); let eqnos = self.eqnos.clone();
        let state = self.state.clone();
        let offset = Cell::new(state.estimated_scroll_metrics().0);
        div().flex_1().min_h_0()
            .child(list(self.state.clone(), move |i, _, _| rows[i].render(&eqnos)).size_full().pb(px(20.)))
            .on_scroll_wheel(move |event, window, cx| {
                // Unmeasured rows have no exact height yet; large swipes and
                // returning to the top after a resize still need to reach them.
                let y = (offset.get() - event.delta.pixel_delta(px(20.)).y).max(px(0.));
                offset.set(y); state.scroll_to_estimated_offset(y);
                cx.stop_propagation(); window.refresh();
            })
    }
}

impl DiffRow {
    fn render(&self, eqnos: &std::collections::HashMap<crate::model::BlockId, u32>) -> AnyElement {
        let (mark, bg, fg) = match self.kind {
            Kind::Same => (" ", Theme::bg_body(), Theme::text()),
            Kind::Added => ("+", Theme::diff_added_bg(), Theme::diff_added()),
            Kind::Removed => ("−", Theme::diff_removed_bg(), Theme::danger()),
        };
        if let Some(block) = &self.block {
            return div().w_full().flex().items_start().py(px(8.)).bg(bg)
                .child(div().flex_none().flex().text_size(px(12.)).line_height(px(25.)).font_family(theme::MONO_FONT)
                    .child(number(self.number)).child(div().w(px(22.)).text_color(fg).child(mark)))
                .child(div().flex_1().min_w_0().pr(px(18.)).overflow_hidden().child(crate::home::formatted_block(block, eqnos))).into_any_element();
        }
        div().w_full().flex().items_start().min_h(px(25.)).bg(bg).text_size(px(12.)).line_height(px(25.)).font_family(theme::MONO_FONT)
            .child(number(self.number))
            .child(div().w(px(22.)).flex_none().text_color(fg).child(mark))
            .child(div().flex_1().min_w_0().pr(px(18.)).whitespace_normal().text_color(Theme::text()).child(self.text.clone())).into_any_element()
    }
}

impl Workspace {
    pub(crate) fn open_git(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.init_git_panel(cx);
        self.show_view(View::Git, cx);
        self.git_action(Action::Refresh, window, cx);
    }

    pub(crate) fn select_git_change(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.init_git_panel(cx);
        let p = self.git.as_mut().unwrap();
        p.request += 1;
        let request = p.request;
        p.selected = Some(path.clone()); p.preview = None; p.loading = true; p.source = false;
        let root = Store::root_dir();
        let task = cx.background_executor().spawn(async move {
            LibraryRepo::open(&root).and_then(|r| r.change_preview(&path)).map_err(|e| format!("{e:#}"))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |w, cx| {
                let Some(p) = w.git.as_mut().filter(|p| p.request == request) else { return };
                p.loading = false;
                match result { Ok(preview) => p.preview = Some(GitPreview::new(preview)), Err(e) => p.error = Some(crate::settings::diagnostic(&e)) }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    pub(crate) fn render_git(&self, cx: &mut Context<Self>) -> AnyElement {
        let header = crate::trash::page_header(self, tr("Git 变更", "Git changes"), 26., cx);
        let Some(p) = &self.git else { return div().child(header).into_any_element() };
        let status = p.status.as_ref();
        let count = status.map_or(0, |s| s.visible_changes().count());
        let staged = status.map_or(0, |s| s.visible_changes().filter(|c| c.staged).count());
        let unstaged = status.is_some_and(|s| s.visible_changes().any(|c| c.unstaged));
        let busy = self.store.file_task_busy() || self.tree_writes_pending();
        let has_changes = status.is_some_and(|s| !s.changes.is_empty());
        let can_commit = has_changes && !busy;
        let mut files = div().id("git-files").flex_1().min_h_0().overflow_y_scroll().p(px(8.));
        if let Some(status) = status {
            for (i, change) in status.visible_changes().enumerate() {
                let path = change.path.clone();
                let note = self.entries.iter().map(|e| (&e.id, &e.meta, &e.title))
                    .chain(self.trash.iter().map(|e| (&e.id, &e.meta, &e.title)))
                    .find(|(_, meta, _)| !meta.local() && meta.folder.is_none() && meta.rel == path);
                let title = note.map(|(_, _, title)| crate::settings::display_title(title).to_string())
                    .unwrap_or_else(|| path.rsplit('/').next().unwrap_or(&path).to_string());
                let open_id = note.map(|(id, _, _)| id.clone());
                let can_open = !busy && change.kind != 'D' && open_id.is_some();
                let (label, color) = match change.kind {
                    'A' => (tr("新增", "Added"), Theme::diff_added()),
                    'D' => (tr("删除", "Deleted"), Theme::danger()),
                    _ => (tr("修改", "Modified"), Theme::primary()),
                };
                let selected = p.selected.as_deref() == Some(&path);
                let stage_path = path.clone(); let unstage_path = path.clone(); let discard_path = path.clone();
                let can_discard = change.unstaged && (change.kind != 'A'
                    || !matches!(path.as_str(), "quiet.json" | ".quiet/format.json" | ".gitattributes" | ".gitignore"));
                files = files.child(div().id(("git-file", i)).w_full().min_h(px(58.)).px(px(10.)).py(px(9.)).flex().items_center().gap(px(9.))
                    .rounded(px(7.)).cursor_pointer().when(selected, |d| d.bg(Theme::block_selected()))
                    .when(!selected, |d| d.hover(|s| s.bg(Theme::fill_hover())))
                    .child(ui::icon("file-text", 16., Theme::icon()))
                    .child(div().flex_1().min_w_0().flex().flex_col().gap(px(4.))
                        .child(div().id(("git-file-title", i)).truncate().text_size(px(13.)).text_color(Theme::text()).child(title).tip(&path, None))
                        .child(div().id(("git-file-path", i)).truncate().text_size(px(10.)).text_color(Theme::text_caption()).child(path.clone()).tip(&path, None))
                        .child(div().flex().items_center().gap(px(6.))
                            .child(div().text_size(px(10.)).text_color(color).child(label))
                            .when(change.staged, |d| d.child(div().text_size(px(10.)).text_color(Theme::diff_added())
                                .child(if change.unstaged { tr("部分已暂存", "Partly staged") } else { tr("已暂存", "Staged") })))))
                    .child(div().flex_none().flex().items_center().gap(px(2.))
                        .child(tool(("git-open", i), "pen-line", tr("打开文件", "Open file"), can_open)
                            .when(can_open, |d| d.on_click(cx.listener(move |w, _, window, cx| {
                                cx.stop_propagation();
                                if let Some(id) = &open_id && w.try_open(id, window, cx)
                                    && let Some(editor) = &w.editor { editor.read(cx).focus(window); }
                            }))))
                        .when(change.unstaged, |d| d.child(tool(("git-stage", i), "plus", tr("暂存修改", "Stage changes"), !busy)
                            .when(!busy, |d| d.on_click(cx.listener(move |w, _, window, cx| {
                                cx.stop_propagation(); w.git_action(Action::Stage(stage_path.clone()), window, cx);
                            })))))
                        .when(change.staged, |d| d.child(tool(("git-unstage", i), "minus", tr("取消暂存", "Unstage changes"), !busy)
                            .when(!busy, |d| d.on_click(cx.listener(move |w, _, window, cx| {
                                cx.stop_propagation(); w.git_action(Action::Unstage(unstage_path.clone()), window, cx);
                            })))))
                        .child(tool(("git-discard", i), "undo", tr("丢弃未暂存的修改", "Discard unstaged changes"), !busy && can_discard)
                            .when(!busy && can_discard, |d| d.on_click(cx.listener(move |w, _, _, cx| {
                                cx.stop_propagation(); w.confirm = Some(crate::app::Confirm::GitDiscard(discard_path.clone())); cx.notify();
                            })))))
                    .on_click(cx.listener(move |w, _, window, cx| w.select_git_change(path.clone(), window, cx))));
            }
        }
        if count == 0 {
            files = files.child(div().p(px(16.)).text_size(px(12.)).line_height(px(20.)).text_color(Theme::text_caption())
                .child(if has_changes { tr("空行变更已隐藏，提交时会还原。", "Blank-line changes are hidden and will be restored on commit.") }
                    else if status.is_some() { tr("工作区干净，没有未提交的变更。", "Working tree clean. No uncommitted changes.") }
                    else { tr("刷新后查看文档库的变更。", "Refresh to see library changes.") }));
        }
        let branch = status.map(|s| s.branch.clone()).unwrap_or_else(|| "—".into());
        let left = div().w(px(256.)).max_w(relative(0.45)).flex_none().h_full().flex().flex_col().border_r_1().border_color(Theme::line_border())
            .child(div().h(px(48.)).flex_none().px(px(14.)).flex().items_center().gap(px(4.))
                .child(tool("git-panel-commit-tool", "check", if staged > 0 { tr("提交已暂存变更", "Commit staged changes") } else { tr("提交本地变更", "Commit local changes") }, can_commit)
                    .when(can_commit, |d| d.on_click(cx.listener(|w, _, window, cx| w.git_action(Action::Commit, window, cx)))))
                .child(tool("git-panel-stage-all", "plus", tr("全部暂存", "Stage all"), unstaged && !busy)
                    .when(unstaged && !busy, |d| d.on_click(cx.listener(|w, _, window, cx| w.git_action(Action::StageAll, window, cx)))))
                .child(tool("git-panel-unstage-all", "minus", tr("全部取消暂存", "Unstage all"), staged > 0 && !busy)
                    .when(staged > 0 && !busy, |d| d.on_click(cx.listener(|w, _, window, cx| w.git_action(Action::UnstageAll, window, cx)))))
                .child(tool("git-panel-push", "arrow-up", tr("推送", "Push"), !busy)
                    .when(!busy, |d| d.on_click(cx.listener(|w, _, window, cx| w.git_action(Action::Push, window, cx)))))
                .child(tool("git-panel-pull", "arrow-down", tr("拉取", "Pull"), !busy)
                    .when(!busy, |d| d.on_click(cx.listener(|w, _, window, cx| w.git_action(Action::Pull, window, cx)))))
                .child(tool("git-panel-refresh", "restore", tr("刷新状态", "Refresh status"), !busy)
                    .when(!busy, |d| d.on_click(cx.listener(|w, _, window, cx| w.git_action(Action::Refresh, window, cx)))))
                .child(div().flex_1())
                .child(div().id("git-change-count").text_size(px(11.)).text_color(Theme::text_caption()).child(count.to_string()).tip(tr("变更文件", "Changed files"), None)))
            .child(files)
            .child(div().flex_none().p(px(14.)).border_t_1().border_color(Theme::line_border()).flex().flex_col().gap(px(10.))
                .child(div().text_size(px(11.)).text_color(Theme::text_caption()).child(if staged > 0 { tr("仅提交已暂存的变更", "Commit staged changes only") } else { tr("提交文档库中的全部变更", "Commit all library changes") }))
                .child(div().px(px(9.)).py(px(7.)).border_1().border_color(Theme::line_border()).rounded(px(6.)).child(p.message.clone()))
                .child(ui::primary_button("git-panel-commit", if staged > 0 { tr("提交已暂存变更", "Commit staged changes") } else { tr("提交本地变更", "Commit local changes") })
                    .when(!can_commit, |d| d.opacity(0.45))
                    .when(can_commit, |d| d.on_click(cx.listener(|w, _, window, cx| w.git_action(Action::Commit, window, cx))))));
        let mut diff = div().flex_1().min_w_0().h_full().flex().flex_col();
        if let Some(preview) = &p.preview {
            let rows = if p.source { preview.source.as_ref().unwrap_or(&preview.text) } else { preview.formatted.as_ref().unwrap_or(&preview.text) };
            let truncated = rows.truncated;
            let (added, removed) = rows.counts;
            diff = diff.child(div().flex_none().min_h(px(72.)).p(px(18.)).flex().flex_col().gap(px(7.))
                .child(div().flex().items_center().gap(px(12.))
                    .child(div().flex_1().min_w_0().truncate().text_size(px(14.)).font_weight(FontWeight::MEDIUM).child(preview.title.clone()))
                    .when(!preview.binary, |d| d
                        .child(div().flex_none().text_size(px(12.)).text_color(Theme::diff_added()).child(format!("+{added}")))
                        .child(div().flex_none().text_size(px(12.)).text_color(Theme::danger()).child(format!("−{removed}")))))
                .child(div().flex().items_center().justify_between().gap(px(8.))
                    .child(div().min_w_0().truncate().text_size(px(11.)).text_color(Theme::text_caption()).child(tr("上次提交 → 工作区 · 只读对比", "Last commit → Working tree · Read only")))
                    .when(preview.source.is_some(), |d| d.child(ui::secondary_button("git-diff-source", if p.source { tr("查看正文", "Show text") } else { tr("查看源码", "Show source") })
                        .on_click(cx.listener(|w, _, _, cx| { if let Some(p) = &mut w.git { p.source = !p.source; } cx.notify(); }))))));
            if truncated {
                diff = diff.child(div().flex_none().px(px(18.)).py(px(8.)).text_size(px(11.)).text_color(Theme::text_caption())
                    .child(if !p.source && preview.formatted.is_some() { tr("仅对比前 1200 个内容块，后续内容未显示。", "Only the first 1200 blocks are compared. Later content is omitted.") }
                        else { tr("仅对比前 1200 行，后续内容未显示。", "Only the first 1200 lines are compared. Later content is omitted.") }));
            }
            if preview.binary {
                diff = diff.child(empty(tr("图片或附件内容已变更，不提供文本对比。", "Image or attachment changed. No text diff available.")));
            } else if added == 0 && removed == 0 {
                diff = diff.child(empty(if truncated {
                    tr("显示范围内没有文字差异，请检查完整文件。", "No text differences in the displayed range. Check the full file.")
                } else if preview.source.is_some() && !p.source {
                    tr("正文文字相同，可查看源码中的格式或样式变更。", "Text is unchanged. View source to inspect formatting or style changes.")
                } else { tr("文件内容与上次提交相同，请刷新状态。", "File matches the last commit. Refresh the status.") }));
            } else {
                diff = diff.child(rows.render());
            }
        } else if p.loading {
            diff = diff.child(empty(tr("正在载入对比…", "Loading diff…")));
        } else if !has_changes && status.is_some() {
            diff = diff.child(empty(tr("所有变更均已提交", "All changes committed")));
        } else if count > 0 || status.is_none() {
            diff = diff.child(empty(tr("选择文件，查看变更对比", "Select a file to compare changes")));
        }
        div().size_full().flex().flex_col().text_color(Theme::text()).child(header)
            .child(div().flex_none().min_h(px(44.)).px(px(20.)).py(px(7.)).flex().flex_wrap().items_center().gap(px(10.)).border_b_1().border_color(Theme::line_border())
                .child(div().flex_1().text_size(px(12.)).text_color(Theme::text_caption()).child(format!("{branch} · {}", tr("本地工作区", "Local working tree")))))
            .children(p.error.as_ref().map(|e| div().px(px(20.)).py(px(10.)).text_size(px(12.)).text_color(Theme::danger()).child(e.clone())))
            .children(p.notice.as_ref().map(|n| div().px(px(20.)).py(px(8.)).text_size(px(12.)).text_color(Theme::text_caption()).child(n.clone())))
            .child(div().flex_1().min_h_0().flex().child(left).child(diff)).into_any_element()
    }
}

fn empty(text: &'static str) -> Div {
    div().flex_1().p(px(30.)).flex().items_center().justify_center().text_size(px(13.)).line_height(px(22.)).text_color(Theme::text_caption()).child(text)
}
fn number(n: usize) -> Div {
    div().w(px(42.)).flex_none().pr(px(10.)).text_right().text_color(Theme::text_caption()).child(n.to_string())
}

fn tool(id: impl Into<ElementId>, icon: &'static str, label: &'static str, enabled: bool) -> Stateful<Div> {
    div().id(id).size(px(26.)).flex_none().flex().items_center().justify_center().rounded(px(5.)).tip(label, None)
        .child(if icon == "undo" { svg().path("editor-icons/undo.svg").size(px(16.)).flex_none().text_color(Theme::icon()) }
            else if matches!(icon, "arrow-up" | "arrow-down") { ui::icon("arrow-left", 16., Theme::icon())
                .with_transformation(Transformation::rotate(radians(if icon == "arrow-up" { std::f32::consts::FRAC_PI_2 } else { -std::f32::consts::FRAC_PI_2 }))) }
            else { ui::icon(icon, 16., Theme::icon()) })
        .when(enabled, |d| d.cursor_pointer().hover(|s| s.bg(Theme::fill_hover())))
        .when(!enabled, |d| d.opacity(0.35))
        .on_click(|_, _, cx| cx.stop_propagation())
}

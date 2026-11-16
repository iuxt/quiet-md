//! Library Git settings. All repository and network work runs off the UI thread.
use crate::{app::{Confirm, Workspace}, editor::core::Core, input::TextField, settings::tr, theme::{self, Theme}, ui};
use fdl_core::{git::{Auth, Commit, Config, LibraryRepo, Status}, store::Store};
use gpui::{prelude::FluentBuilder, *};

pub(crate) struct GitPanel {
    name: Entity<TextField>, email: Entity<TextField>, remote: Entity<TextField>, lfs: Entity<TextField>,
    username: Entity<TextField>, token: Entity<TextField>, message: Entity<TextField>,
    status: Option<Status>, commits: Vec<Commit>, error: Option<String>, notice: Option<String>,
}

#[derive(Clone)]
pub(crate) enum Action { Enable, Refresh, Configure, Commit, Push, Pull, Clone(std::path::PathBuf) }

impl Workspace {
    pub(crate) fn init_git_panel(&mut self, cx: &mut Context<Self>) {
        if self.git.is_some() { return }
        let config = LibraryRepo::open(&Store::root_dir()).and_then(|r| r.config()).unwrap_or_default();
        let auth = Auth::environment();
        let mut field = |placeholder: &'static str, value: &str, secret: bool| cx.new(|cx| {
            let mut f = TextField::new(placeholder, cx); f.secret = secret; f.set_text(value, cx); f
        });
        self.git = Some(GitPanel {
            name: field(tr("作者名称", "Author name"), &config.name, false), email: field(tr("邮箱", "Email"), &config.email, false),
            remote: field("https://github.com/you/notes.git", &config.remote, false),
            lfs: field(tr("留空使用仓库默认地址", "Leave blank for the repository default"), &config.lfs_url, false),
            username: field(tr("用户名", "Username"), &auth.username, false), token: field(tr("访问令牌（仅本次会话）", "Access token (this session only)"), &auth.token, true),
            message: field(tr("提交说明", "Commit message"), "", false),
            status: None, commits: vec![], error: None, notice: None,
        });
    }

    pub(crate) fn git_action(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if self.store.file_task_busy() || self.tree_writes_pending() { return }
        self.close_history(cx);
        if !self.flush(cx) { return }
        // Save all same-library windows before the worker takes the write lock.
        for handle in cx.windows().into_iter().filter(|h| h.window_id() != window.window_handle().window_id()).filter_map(|h| h.downcast::<Workspace>()) {
            if !handle.update(cx, |w, _, cx| { w.close_history(cx); w.flush(cx) }).unwrap_or(false) {
                self.show_toast_kind(ui::ToastKind::Error, tr("其他窗口有未保存的修改", "Another window has unsaved changes"), cx); return;
            }
        }
        self.init_git_panel(cx);
        let panel = self.git.as_mut().unwrap(); panel.error = None; panel.notice = None;
        let config = Config { name: panel.name.read(cx).text.clone(), email: panel.email.read(cx).text.clone(), remote: panel.remote.read(cx).text.trim().to_string(),
            branch: String::new(), lfs_url: panel.lfs.read(cx).text.trim().to_string() };
        let auth = Auth { username: panel.username.read(cx).text.clone(), token: panel.token.read(cx).text.clone() };
        let message = panel.message.read(cx).text.clone();
        let Some(guard) = self.store.begin_file_task() else { return };
        let mut writer = self.store.tree_writer();
        let reload = matches!(action, Action::Enable | Action::Pull);
        let task = cx.background_executor().spawn(async move {
            let result = (|| -> anyhow::Result<_> {
                if let Action::Clone(path) = &action {
                    let repo = LibraryRepo::clone_into(&config.remote, path, &auth)?; repo.configure(&config)?;
                    return Ok((None, vec![], String::new(), Some(path.clone())));
                }
                if matches!(action, Action::Enable) { writer.enable_git()?; }
                let repo = LibraryRepo::open(writer.library_root())?;
                let notice = match action {
                    Action::Enable => { repo.configure(&config)?; tr("已启用 Git，原文件备份已保留", "Git enabled; original files backed up").to_string() },
                    Action::Configure => { repo.configure(&config)?; tr("Git 配置已保存", "Git configuration saved").to_string() },
                    Action::Commit => {
                        // Commit under the same library lock as autosaves and tree operations.
                        if writer.git_commit(&message)?.is_some() { tr("本地版本已保存", "Local version saved").to_string() }
                        else { tr("内容没有变化，无需新增提交", "No changes; no commit needed").to_string() }
                    },
                    Action::Push => { repo.push(&auth)?; tr("已推送到远端", "Pushed to remote").to_string() },
                    Action::Pull => { writer.git_pull(&auth)?; tr("已与远端同步", "Up to date with remote").to_string() },
                    _ => String::new(),
                };
                Ok((Some(repo.status()?), repo.history(None, 30)?, notice, None))
            })().map_err(|e| format!("{e:#}"));
            (writer, result)
        });
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let _guard = guard;
            let (writer, result) = task.await;
            let _ = this.update_in(cx, |w, window, cx| {
                if reload { w.store.with(|store| *store = writer); }
                match result {
                    Ok((status, commits, notice, clone)) => {
                        if let Some(path) = clone { w.confirm = Some(Confirm::SwitchLibrary(path)); }
                        if let Some(panel) = w.git.as_mut() { panel.status = status; panel.commits = commits; panel.notice = (!notice.is_empty()).then_some(notice); }
                    },
                    Err(error) => if let Some(panel) = w.git.as_mut() { panel.error = Some(crate::settings::diagnostic(&error)); },
                }
                if reload {
                    w.refresh_after_git(cx);
                    for handle in cx.windows().into_iter().filter(|h| h.window_id() != window.window_handle().window_id()).filter_map(|h| h.downcast::<Workspace>()) {
                        let _ = handle.update(cx, |other, _, cx| other.refresh_after_git(cx));
                    }
                }
                cx.notify();
            });
        }).detach();
    }

    fn refresh_after_git(&mut self, cx: &mut Context<Self>) {
        self.entries = self.store.list(); self.sort();
        if let Some(id) = self.current.clone() {
            if self.entries.iter().any(|e| e.id == id) {
                if let Ok(doc) = self.store.load_for_edit(&id, false) && let Some(editor) = &self.editor {
                    editor.update(cx, |e, cx| e.receive_document(&Core::new(doc), cx));
                }
            } else { self.current = None; self.editor = None; self.show_view(crate::app::View::Home, cx); }
        }
        cx.notify();
    }

    fn pick_git_clone(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = crate::devtools::take_pick();
        let prompt = picked.is_none().then(|| cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false,
            prompt: Some(tr("选择空文件夹", "Choose an empty folder").into()), extensions: None }));
        cx.spawn_in(window, async move |this, cx| {
            let paths = match prompt { Some(p) => p.await.ok().and_then(Result::ok).flatten(), None => picked };
            if let Some(path) = paths.and_then(|p| p.into_iter().next()) {
                let _ = this.update_in(cx, |w, window, cx| w.git_action(Action::Clone(path), window, cx));
            }
        }).detach();
    }

    pub(crate) fn render_git_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let enabled = self.store.with(|s| s.git_enabled());
        let mut section = div().w_full().flex().flex_col().gap(px(12.))
            .child(crate::settings::caption(tr("Git 备份与同步", "Git backup and sync")))
            .child(div().text_size(px(theme::ui_font_small())).line_height(px(19.)).text_color(Theme::text_caption()).child(if enabled {
                tr("文章使用唯一文件名，标题与分类保存在 quiet.json，图片使用 Git LFS。", "Notes use unique filenames, metadata lives in quiet.json, and assets use Git LFS.")
            } else {
                tr("启用后使用 UUID 文件名并保留原文件备份。Git 备份不会自动推送。", "Enable UUID filenames with a backup of the originals. Backups are only pushed when you choose.")
            }));
        if let Some(panel) = &self.git {
            let field = |label: &'static str, entity: Entity<TextField>| div().w_full().flex().flex_col().gap(px(5.))
                .child(div().text_size(px(12.)).text_color(Theme::text_caption()).child(label))
                .child(div().w_full().min_h(px(32.)).px(px(9.)).py(px(5.)).flex().items_center().border_1().border_color(Theme::line_border()).rounded(px(6.)).child(entity));
            section = section
                .child(div().flex().gap(px(12.)).child(field(tr("作者名称", "Author name"), panel.name.clone())).child(field(tr("邮箱", "Email"), panel.email.clone())))
                .child(field(tr("远端仓库（HTTPS）", "Remote repository (HTTPS)"), panel.remote.clone()))
                .child(field(tr("LFS 地址（可选）", "LFS endpoint (optional)"), panel.lfs.clone()))
                .child(div().flex().gap(px(12.)).child(field(tr("用户名", "Username"), panel.username.clone())).child(field(tr("访问令牌（仅本次会话）", "Access token (this session only)"), panel.token.clone())))
                .child(div().flex().flex_wrap().gap(px(8.))
                    .when(!enabled, |d| d.child(ui::primary_button("git-enable", tr("启用 Git", "Enable Git")).on_click(cx.listener(|w, _, window, cx| w.git_action(Action::Enable, window, cx)))))
                    .when(enabled, |d| d
                        .child(ui::secondary_button("git-configure", tr("保存配置", "Save configuration")).on_click(cx.listener(|w, _, window, cx| w.git_action(Action::Configure, window, cx))))
                        .child(ui::secondary_button("git-refresh", tr("刷新状态", "Refresh status")).on_click(cx.listener(|w, _, window, cx| w.git_action(Action::Refresh, window, cx)))))
                    .child(ui::secondary_button("git-clone", tr("克隆到空文件夹…", "Clone to an empty folder…")).on_click(cx.listener(|w, _, window, cx| w.pick_git_clone(window, cx)))));
            if enabled {
                section = section.child(field(tr("提交说明", "Commit message"), panel.message.clone()))
                    .child(div().flex().flex_wrap().gap(px(8.))
                        .child(ui::primary_button("git-commit", tr("提交本地变更", "Commit local changes")).on_click(cx.listener(|w, _, window, cx| w.git_action(Action::Commit, window, cx))))
                        .child(ui::secondary_button("git-pull", tr("拉取", "Pull")).on_click(cx.listener(|w, _, window, cx| w.git_action(Action::Pull, window, cx))))
                        .child(ui::secondary_button("git-push", tr("推送", "Push")).on_click(cx.listener(|w, _, window, cx| w.git_action(Action::Push, window, cx)))))
                    .child(div().text_size(px(12.)).text_color(Theme::text_caption()).child(tr("拉取前请先提交本地变更；历史分叉时不会覆盖文章。", "Commit local changes before pulling. Diverged histories are never overwritten.")));
                if let Some(status) = &panel.status {
                    section = section.child(div().text_size(px(12.)).text_color(Theme::text_caption()).child(format!("{} · {} · {}", status.branch,
                        status.head.as_deref().map(|id| &id[..7]).unwrap_or("—"), crate::localized_format!("{} 项变更", "{} changes", status.changes.len()))));
                    for change in &status.changes { section = section.child(div().text_size(px(12.)).text_color(Theme::text()).child(format!("{}  {}", change.kind, change.path))); }
                }
                if !panel.commits.is_empty() {
                    section = section.child(crate::settings::caption(tr("最近提交", "Recent commits")));
                    for commit in &panel.commits {
                        section = section.child(div().text_size(px(12.)).text_color(Theme::text_caption()).child(format!("{}  {} · {}", &commit.id[..7], commit.message, commit.author)));
                    }
                }
                if self.current.is_some() {
                    section = section.child(ui::secondary_button("git-article-history", tr("查看当前文章历史", "View current note history"))
                        .on_click(cx.listener(|w, _, window, cx| {
                            w.show_view(crate::app::View::Doc, cx);
                            if let Some(id) = w.current.clone() { w.open_history(&id, window, cx); }
                        })));
                }
            }
            if let Some(error) = &panel.error { section = section.child(div().text_size(px(12.)).text_color(Theme::danger()).child(error.clone())); }
            if let Some(notice) = &panel.notice { section = section.child(div().text_size(px(12.)).text_color(Theme::text_caption()).child(notice.clone())); }
        } else {
            section = section.child(ui::secondary_button("git-settings", tr("配置 Git", "Configure Git")).on_click(cx.listener(|w, _, _, cx| { w.init_git_panel(cx); cx.notify(); })));
        }
        section.into_any_element()
    }
}

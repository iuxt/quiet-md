//! Library Git settings. All repository and network work runs off the UI thread.
use crate::{app::{Confirm, Workspace}, editor::core::Core, input::TextField, settings::tr, theme::{self, Theme}, ui};
use fdl_core::git::{Auth, Config, LibraryRepo, Status};
use gpui::{prelude::FluentBuilder, *};
use std::{collections::{BTreeSet, HashMap}, sync::Arc};

pub(crate) struct GitPanel {
    name: Entity<TextField>, email: Entity<TextField>, remote: Entity<TextField>, lfs: Entity<TextField>,
    ssh_key: Entity<TextField>, ssh_passphrase: Entity<TextField>, lfs_ca_info: Entity<TextField>,
    username: Entity<TextField>, token: Entity<TextField>, pub(super) message: Entity<TextField>,
    pub(super) status: Option<Status>, settings_error: Option<String>, settings_notice: Option<String>,
    pub(super) selected: Option<String>, pub(super) preview: Option<crate::git_panel::GitPreview>, pub(super) loading: bool,
    pub(super) source: bool, pub(super) request: u64,
}

#[derive(Clone)]
pub(crate) enum Action { Enable, Refresh, Configure, Commit, Push, Pull, StageAll, UnstageAll, Stage(String), Unstage(String), Discard(String), Clone(std::path::PathBuf) }

impl Workspace {
    pub(crate) fn init_git_panel(&mut self, cx: &mut Context<Self>) {
        if self.git.is_some() { return }
        let (config, error) = match self.store.with(|store| LibraryRepo::settings_at(store.library_root())) {
            Ok(config) => (config, None),
            Err(error) => (Config::default(), Some(crate::settings::diagnostic(&format!("{error:#}")))),
        };
        let auth = Auth::environment();
        let (y, m, d, _, _) = crate::time::civil(crate::time::now());
        let message = format!("{y}-{m:02}-{d:02}");
        let mut field = |placeholder: &'static str, value: &str, secret: bool| cx.new(|cx| {
            let mut f = TextField::new(placeholder, cx); f.secret = secret; f.set_text(value, cx); f
        });
        self.git = Some(GitPanel {
            name: field(tr("作者名称", "Author name"), &config.name, false), email: field(tr("邮箱", "Email"), &config.email, false),
            remote: field("https://github.com/you/notes.git", &config.remote, false),
            lfs: field(tr("留空使用仓库默认地址", "Leave blank for the repository default"), &config.lfs_url, false),
            username: field(tr("用户名", "Username"), &auth.username, false), token: field(tr("访问令牌（仅本次会话）", "Access token (this session only)"), &auth.token, true),
            ssh_key: field(tr("留空使用 SSH agent", "Leave blank to use SSH agent"), &config.ssh_key, false),
            ssh_passphrase: field(tr("私钥密码（仅本次会话）", "Key passphrase (this session only)"), &auth.ssh_passphrase, true),
            lfs_ca_info: field(tr("PEM CA 证书路径（可选）", "PEM CA certificate path (optional)"), &config.lfs_ca_info, false),
            message: field(tr("提交说明", "Commit message"), &message, false),
            status: None, settings_error: error, settings_notice: None,
            selected: None, preview: None, loading: false, source: false, request: 0,
        });
    }

    pub(crate) fn git_action(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if self.store.file_task_busy() || self.tree_writes_pending() { return }
        self.close_history(cx);
        if !self.flush(cx) { return }
        // Save all same-library windows before the worker takes the write lock.
        let mut open_ids: Vec<_> = self.current.iter().cloned().collect();
        for handle in cx.windows().into_iter().filter(|h| h.window_id() != window.window_handle().window_id()).filter_map(|h| h.downcast::<Workspace>()) {
            let Ok((true, id)) = handle.update(cx, |w, _, cx| { w.close_history(cx); (w.flush(cx), w.current.clone()) }) else {
                self.show_toast_kind(ui::ToastKind::Error, tr("其他窗口有未保存的修改", "Another window has unsaved changes"), cx); return;
            };
            open_ids.extend(id);
        }
        let baselines = open_ids.into_iter().filter_map(|id| self.store.edit_baseline(&id).map(|body| (id, body))).collect::<HashMap<_, _>>();
        self.init_git_panel(cx);
        let panel = self.git.as_mut().unwrap(); panel.settings_error = None; panel.settings_notice = None;
        let settings_feedback = matches!(action, Action::Enable | Action::Configure | Action::Clone(_));
        panel.request += 1; panel.preview = None; panel.loading = false;
        let config = Config { name: panel.name.read(cx).text.clone(), email: panel.email.read(cx).text.clone(), remote: panel.remote.read(cx).text.trim().to_string(),
            branch: String::new(), lfs_url: panel.lfs.read(cx).text.trim().to_string(),
            ssh_key: panel.ssh_key.read(cx).text.trim().to_string(), lfs_ca_info: panel.lfs_ca_info.read(cx).text.trim().to_string() };
        let auth = Auth { username: panel.username.read(cx).text.clone(), token: panel.token.read(cx).text.clone(), ssh_passphrase: panel.ssh_passphrase.read(cx).text.clone() };
        let message = panel.message.read(cx).text.clone();
        let Some(guard) = self.store.begin_file_task() else { return };
        let mut writer = self.store.tree_writer();
        let task = cx.background_executor().spawn(async move {
            let mut restored = BTreeSet::new();
            let mut refresh_all = false;
            let mut adopt_tree = false;
            let result = (|| -> anyhow::Result<_> {
                if let Action::Clone(path) = &action {
                    LibraryRepo::clone_configured(&config, path, &auth)?;
                    return Ok((None, String::new(), Some(path.clone())));
                }
                if matches!(action, Action::Enable) { writer.enable_git()?; }
                let repo = LibraryRepo::open(writer.library_root())?;
                let notice = match action {
                    Action::Enable => { repo.configure(&config)?; tr("已启用 Git", "Git enabled").to_string() },
                    Action::Configure => { repo.configure(&config)?; tr("Git 配置已保存", "Git configuration saved").to_string() },
                    Action::Commit => {
                        // Commit under the same library lock as autosaves and tree operations.
                        if writer.git_commit_restoring(&message, &mut restored)?.is_some() { tr("本地版本已保存", "Local version saved").to_string() }
                        else { tr("内容没有变化，无需新增提交", "No changes; no commit needed").to_string() }
                    },
                    Action::Push => { repo.push(&auth)?; tr("已推送到远端", "Pushed to remote").to_string() },
                    Action::Pull => {
                        refresh_all = writer.git_pull(&auth)?;
                        adopt_tree = refresh_all;
                        tr("已与远端同步", "Up to date with remote").to_string()
                    },
                    Action::StageAll => { writer.git_stage_all(true)?; String::new() },
                    Action::UnstageAll => { writer.git_stage_all(false)?; String::new() },
                    Action::Stage(path) => { writer.git_stage(&path, true)?; String::new() },
                    Action::Unstage(path) => { writer.git_stage(&path, false)?; String::new() },
                    Action::Discard(path) => {
                        writer.git_discard(&path)?;
                        adopt_tree = true;
                        refresh_all = path == fdl_core::git::MANIFEST;
                        restored.insert(path);
                        tr("已丢弃未暂存的修改", "Unstaged changes discarded").to_string()
                    },
                    _ => String::new(),
                };
                Ok((Some(repo.status()?), notice, None))
            })().map_err(|e| format!("{e:#}"));
            (writer, result, restored, adopt_tree, refresh_all)
        });
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let _guard = guard;
            let (writer, result, restored, adopt_tree, refresh_all) = task.await;
            let _ = this.update_in(cx, |w, window, cx| {
                if adopt_tree { w.store.receive_tree(writer); }
                match result {
                    Ok((status, notice, clone)) => {
                        if let Some(path) = clone { w.confirm = Some(Confirm::SwitchLibrary(path)); }
                        if let Some(panel) = w.git.as_mut() { panel.status = status; }
                        if !notice.is_empty() {
                            if settings_feedback {
                                if let Some(panel) = w.git.as_mut() { panel.settings_notice = Some(notice); }
                            } else {
                                w.show_toast(&notice, cx);
                            }
                        }
                    },
                    Err(error) => {
                        let error = crate::settings::diagnostic(&error);
                        if settings_feedback {
                            if let Some(panel) = w.git.as_mut() { panel.settings_error = Some(error); }
                        } else {
                            w.show_toast_kind(ui::ToastKind::Error, &error, cx);
                        }
                    },
                }
                if refresh_all || !restored.is_empty() {
                    let paths = (!refresh_all).then_some(&restored);
                    w.refresh_after_git(paths, &baselines, window, cx);
                    for handle in cx.windows().into_iter().filter(|h| h.window_id() != window.window_handle().window_id()).filter_map(|h| h.downcast::<Workspace>()) {
                        let _ = handle.update(cx, |other, window, cx| other.refresh_after_git(paths, &baselines, window, cx));
                    }
                }
                if w.view == crate::app::View::Git {
                    let path = w.git.as_ref().and_then(|p| p.status.as_ref().and_then(|s| {
                        p.selected.as_ref().filter(|path| s.visible_changes().any(|c| &c.path == *path)).cloned()
                            .or_else(|| s.visible_changes().find(|c| c.path.ends_with(".xml") || c.path.ends_with(".md")).or_else(|| s.visible_changes().next()).map(|c| c.path.clone()))
                    }));
                    if let Some(path) = path { w.select_git_change(path, window, cx); }
                    else if let Some(p) = w.git.as_mut() { p.selected = None; }
                }
                cx.notify();
            });
        }).detach();
    }

    fn refresh_after_git(&mut self, restored: Option<&BTreeSet<String>>, baselines: &HashMap<String, Arc<String>>, window: &Window, cx: &mut Context<Self>) {
        self.entries = self.store.list(); self.sort();
        if let Some(id) = self.current.clone() {
            if let Some(entry) = self.entries.iter().find(|e| e.id == id) {
                if !entry.meta.local() && restored.is_none_or(|paths| paths.contains(&entry.meta.rel)) && let Some(editor) = &self.editor {
                    let title = &editor.read(cx).core.doc.title.text;
                    if let Ok((doc, title)) = self.store.resume_for_edit(&id, baselines.get(&id), title) {
                        editor.update(cx, |e, cx| {
                            if let Some(doc) = doc { e.receive_document(&Core::new(doc), cx); }
                            else if e.core.doc.title.text != title { e.receive_file_title(&title, cx); }
                        });
                    }
                }
            } else if !self.trash_document {
                self.tabs.remove_document(&id);
                self.leave_document(window, cx);
                if self.view != crate::app::View::Git { self.show_view(crate::app::View::Home, cx); }
            }
        }
        self.prune_tabs();
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
        let mut section = div().w_full().px(px(16.)).py(px(14.)).flex().flex_col().gap(px(12.))
            .child(div().text_size(px(theme::ui_font_small())).line_height(px(19.)).text_color(Theme::text_caption()).child(if enabled {
                tr("文章使用唯一文件名，标题与分类保存在 quiet.json，图片使用 Git LFS。", "Notes use unique filenames, metadata lives in quiet.json, and assets use Git LFS.")
            } else {
                tr("启用 Git 备份；旧标题文件名会先备份再转换。不会自动推送。", "Enable Git backup; older title-based files are backed up before conversion. Pushes are manual.")
            }));
        if let Some(panel) = &self.git {
            let field = |label: &'static str, entity: Entity<TextField>| div().w_full().flex().flex_col().gap(px(5.))
                .child(div().text_size(px(12.)).text_color(Theme::text_caption()).child(label))
                .child(div().w_full().min_h(px(32.)).px(px(9.)).py(px(5.)).flex().items_center().border_1().border_color(Theme::line_border()).rounded(px(6.)).child(entity));
            section = section
                .child(div().flex().gap(px(12.)).child(field(tr("作者名称", "Author name"), panel.name.clone())).child(field(tr("邮箱", "Email"), panel.email.clone())))
                .child(field(tr("远端仓库（HTTPS / SSH）", "Remote repository (HTTPS / SSH)"), panel.remote.clone()))
                .child(field(tr("LFS 地址（可选）", "LFS endpoint (optional)"), panel.lfs.clone()))
                .child(div().flex().gap(px(12.)).child(field(tr("用户名", "Username"), panel.username.clone())).child(field(tr("访问令牌（仅本次会话）", "Access token (this session only)"), panel.token.clone())))
                .child(field(tr("SSH 私钥路径（可选）", "SSH private key path (optional)"), panel.ssh_key.clone()))
                .child(field(tr("私钥密码（仅本次会话）", "Key passphrase (this session only)"), panel.ssh_passphrase.clone()))
                .child(field(tr("LFS CA 证书（可选）", "LFS CA certificate (optional)"), panel.lfs_ca_info.clone()))
                .child(div().flex().flex_wrap().gap(px(8.))
                    .when(!enabled, |d| d.child(ui::primary_button("git-enable", tr("启用 Git", "Enable Git")).on_click(cx.listener(|w, _, window, cx| w.git_action(Action::Enable, window, cx)))))
                    .when(enabled, |d| d.child(ui::secondary_button("git-configure", tr("保存配置", "Save configuration")).on_click(cx.listener(|w, _, window, cx| w.git_action(Action::Configure, window, cx)))))
                    .child(ui::secondary_button("git-clone", tr("克隆到空文件夹…", "Clone to an empty folder…")).on_click(cx.listener(|w, _, window, cx| w.pick_git_clone(window, cx)))));
            if let Some(error) = &panel.settings_error { section = section.child(div().text_size(px(12.)).text_color(Theme::danger()).child(error.clone())); }
            if let Some(notice) = &panel.settings_notice { section = section.child(div().text_size(px(12.)).text_color(Theme::text_caption()).child(notice.clone())); }
        } else {
            section = section.child(ui::secondary_button("git-settings", tr("配置 Git", "Configure Git")).on_click(cx.listener(|w, _, _, cx| { w.init_git_panel(cx); cx.notify(); })));
        }
        crate::settings::group(None, vec![section.into_any_element()]).into_any_element()
    }
}

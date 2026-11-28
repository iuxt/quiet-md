use super::*;
use crate::shared_store::SharedStore;
use fdl_core::git::PreparedPull;
use std::time::Duration;

pub(super) const INTERVALS: [u32; 6] = [0, 1, 5, 15, 30, 60];

#[derive(Default)]
struct AutoPull {
    minutes: u32, has_remote: bool, auth: Auth, error: Option<String>,
    revision: u64, running: bool, timer: Option<Task<()>>,
}
impl Global for AutoPull {}

pub(crate) fn start_auto_pull(cx: &mut App) {
    let root = cx.global::<SharedStore>().library_root();
    if let Ok(config) = LibraryRepo::settings_at(&root) { configure(&config, Auth::environment(), cx); }
}

pub(super) fn settings(cx: &App) -> (u32, Option<String>) {
    cx.try_global::<AutoPull>().map(|state| (state.minutes, state.error.clone())).unwrap_or_default()
}

pub(super) fn configure(config: &Config, auth: Auth, cx: &mut App) {
    let state = cx.default_global::<AutoPull>();
    state.minutes = if INTERVALS.contains(&config.auto_pull_minutes) { config.auto_pull_minutes } else { 0 };
    state.has_remote = !config.remote.is_empty();
    state.auth = auth;
    state.error = None;
    state.revision += 1;
    state.timer = None;
    schedule(cx);
    cx.refresh_windows();
}

fn schedule(cx: &mut App) {
    let state = cx.global::<AutoPull>();
    if state.running || state.minutes == 0 || !state.has_remote { return }
    let delay = Duration::from_secs(u64::from(state.minutes) * 60);
    let timer = cx.spawn(async move |cx| {
        cx.background_executor().timer(delay).await;
        let _ = cx.update(start_attempt);
    });
    cx.global_mut::<AutoPull>().timer = Some(timer);
}

fn start_attempt(cx: &mut App) {
    let store = cx.global::<SharedStore>();
    if store.file_task_busy() { return schedule(cx) }
    let root = store.library_root();
    let state = cx.global_mut::<AutoPull>();
    state.running = true;
    let (revision, auth) = (state.revision, state.auth.clone());
    let task = cx.background_executor().spawn(async move { LibraryRepo::open(&root)?.prepare_auto_pull(&auth) });
    cx.spawn(async move |cx| {
        let result: anyhow::Result<()> = async {
            let Some(pull) = task.await? else { return Ok(()) };
            let Some(task) = cx.update(|cx| {
                if cx.global::<AutoPull>().revision != revision { return None }
                apply_when_idle(pull, cx)
            })? else { return Ok(()) };
            let (guard, baselines, result) = task.await;
            cx.update(|cx| {
                drop(guard);
                cx.refresh_windows();
                if let Some((writer, status)) = result? { refresh_windows(writer, status, &baselines, cx); }
                Ok(())
            })?
        }.await;
        let _ = cx.update(|cx| finish(revision, result, cx));
    }).detach();
}

fn apply_when_idle(pull: PreparedPull, cx: &mut App) -> Option<Task<ApplyResult>> {
    let store = cx.global::<SharedStore>().clone();
    if store.file_task_busy() { return None }
    let mut baselines = HashMap::new();
    for handle in cx.windows().into_iter().filter_map(|w| w.downcast::<Workspace>()) {
        handle.update(cx, |w, _, cx| {
            if w.unsaved || w.history.is_some() || w.drawing_open(cx) || w.tree_writes_pending() || w.file_history.busy
                || w.rename.is_some() || w.move_dlg.is_some() || w.confirm.is_some()
                || w.editor.as_ref().is_some_and(|e| e.read(cx).marked.is_some()) { return None }
            if let Some(id) = &w.current && let Some(body) = store.edit_baseline(id) { baselines.insert(id.clone(), body); }
            Some(())
        }).ok().flatten()?;
    }
    let guard = store.begin_file_task()?;
    let mut writer = store.tree_writer();
    cx.refresh_windows();
    Some(cx.background_executor().spawn(async move {
        let result = writer.git_apply_auto_pull(pull).map(|changed| changed.then(|| {
            let status = LibraryRepo::open(writer.library_root()).and_then(|repo| repo.status()).ok();
            (writer, status)
        }));
        (guard, baselines, result)
    }))
}

type ApplyResult = (crate::shared_store::FileTaskGuard, HashMap<String, Arc<String>>, anyhow::Result<Option<(crate::store::Store, Option<Status>)>>);

fn refresh_windows(writer: crate::store::Store, status: Option<Status>, baselines: &HashMap<String, Arc<String>>, cx: &mut App) {
    cx.global::<SharedStore>().receive_tree(writer);
    for handle in cx.windows().into_iter().filter_map(|w| w.downcast::<Workspace>()) {
        let _ = handle.update(cx, |w, window, cx| {
            w.refresh_after_git(None, baselines, window, cx);
            if let Some(panel) = &mut w.git {
                panel.status = status.clone();
                panel.request += 1; panel.preview = None; panel.loading = false; panel.selected = None;
            }
        });
    }
}

fn finish(revision: u64, result: anyhow::Result<()>, cx: &mut App) {
    let state = cx.global_mut::<AutoPull>();
    state.running = false;
    if state.revision == revision {
        let error = result.err().map(|error| format!("{error:#}"));
        if state.error != error { state.error = error; cx.refresh_windows(); }
    }
    schedule(cx);
}

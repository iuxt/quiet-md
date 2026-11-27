use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileState {
    pub title: String,
    pub parent: Option<String>,
    pub order: u64,
    pub root_order: Option<u64>,
}
impl FileState {
    pub fn of(e: &Entry) -> Self { Self { title: e.title.clone(), parent: e.meta.parent.clone(), order: e.meta.order, root_order: e.meta.root_order } }
    fn place(&self) -> (&Option<String>, u64, Option<u64>) { (&self.parent, self.order, self.root_order) }
}

#[derive(Clone)]
pub struct FileDelta { pub id: String, pub before: Option<FileState>, pub after: Option<FileState> }

/// A scoped journal of successful writes, independent of any window's cached list.
#[derive(Default)]
pub struct FileChanges(std::sync::Arc<std::sync::Mutex<Vec<FileDelta>>>, bool);
impl FileChanges {
    pub fn take(self) -> Vec<FileDelta> {
        if self.1 { return vec![]; }
        let mut merged: HashMap<String, FileDelta> = HashMap::new();
        for delta in std::mem::take(&mut *self.0.lock().unwrap()) {
            if let Some(previous) = merged.get_mut(&delta.id) { previous.after = delta.after; }
            else { merged.insert(delta.id.clone(), delta); }
        }
        let mut changes: Vec<_> = merged.into_values().filter(|c| c.before != c.after).collect();
        changes.sort_by(|a, b| a.id.cmp(&b.id));
        changes
    }
}

impl Store {
    pub fn track_file_changes(&mut self) -> FileChanges {
        self.file_changes.retain(|w| w.strong_count() > 0);
        let changes = FileChanges(Default::default(), !self.file_changes.is_empty());
        self.file_changes.push(std::sync::Arc::downgrade(&changes.0));
        changes
    }

    pub(super) fn file_state(&self, id: &str) -> Option<FileState> {
        let m = self.index.get(id)?;
        let title = m.name.clone().or_else(|| self.load(id).ok().map(|d| d.title.text))?;
        Some(FileState { title, parent: m.parent.clone(), order: m.order, root_order: m.root_order })
    }

    pub(super) fn record_file_change(&self, id: &str, before: Option<FileState>) {
        if !self.file_changes.iter().any(|w| w.strong_count() > 0) { return; }
        let after = self.file_state(id);
        if before == after { return; }
        let delta = FileDelta { id: id.to_string(), before, after };
        for journal in self.file_changes.iter().filter_map(|w| w.upgrade()) { journal.lock().unwrap().push(delta.clone()); }
    }

    pub(super) fn record_tree_changes(&self, previous: &HashMap<String, Meta>) {
        if !self.file_changes.iter().any(|w| w.strong_count() > 0) { return; }
        for (id, m) in &self.index {
            let old = previous.get(id);
            if old.is_some_and(|o| (&o.parent, o.order, o.root_order) == (&m.parent, m.order, m.root_order)) { continue; }
            let before = old.and_then(|o| self.file_state(id).map(|mut state| {
                state.parent = o.parent.clone(); state.order = o.order; state.root_order = o.root_order; state
            }));
            self.record_file_change(id, before);
        }
    }
}

impl Store {
    pub fn accept_file_title(&self, id: &str, title: &str) -> Result<()> {
        if self.index.get(id).and_then(|m| m.name.as_deref()) == Some(title) {
            let loaded = self.loaded.borrow();
            if let Some(previous) = loaded.get(id) && std::fs::read_to_string(self.path(id)).is_ok_and(|disk| disk == previous.as_str()) {
                self.loaded_titles.borrow_mut().insert(id.to_string(), title.to_string());
            }
            return Ok(());
        }
        let mut loaded = self.loaded.borrow_mut();
        let Some(previous) = loaded.get(id) else { return Ok(()) };
        // A Markdown file's title is not in the file.
        if self.is_markdown(id) { return Ok(()); }
        let mut expected = docxml::parse(previous)?;
        expected.title = crate::model::RichText::plain(title);
        let disk = std::fs::read_to_string(self.path(id))?;
        // Accept only our title change, never unrelated edits from another instance.
        if docxml::serialize(&expected) == docxml::serialize(&docxml::parse(&disk)?) { loaded.insert(id.to_string(), disk.into()); }
        Ok(())
    }

    /// Apply one management gesture under the library lock. Content is read fresh;
    /// only titles and tree fields are changed, never restored from old snapshots.
    /// All changes to the shared tree are published together.
    pub fn apply_file_history(&mut self, changes: &[FileDelta], redo: bool) -> Result<()> {
        let _lock = self.begin_write()?;
        let mut index = self.index.clone();
        let mut trash = self.trash.clone();
        let pairs: Vec<_> = changes.iter().map(|c| (c, if redo { (&c.before, &c.after) } else { (&c.after, &c.before) })).collect();
        let deleted: HashSet<_> = pairs.iter().filter(|(_, (_, to))| to.is_none()).map(|(c, _)| c.id.as_str()).collect();
        let deleted_at = now();
        for (c, (from, to)) in &pairs {
            anyhow::ensure!(self.index_has(&c.id) == from.is_some(), "文件状态已改变，未执行撤销或重做：{}", c.id);
            if let Some(expected) = from {
                let m = &self.index[&c.id];
                anyhow::ensure!(!m.local(), "不能修改本机文件的位置");
                if to.as_ref().is_none_or(|t| t.place() != expected.place()) {
                    anyhow::ensure!((&m.parent, m.order, m.root_order) == expected.place(), "文件位置已被其他操作改变，请先刷新");
                }
                if let Some(target) = to && target.title != expected.title {
                    anyhow::ensure!(m.name.as_deref() == Some(expected.title.as_str()), "标题已被其他操作改变，未覆盖新标题");
                }
            }
            match (from, to) {
                (Some(_), None) => {
                    anyhow::ensure!(!self.index.iter().any(|(id, m)| m.parent.as_deref() == Some(&c.id) && !deleted.contains(id.as_str())),
                        "文档有后来新增的子文档，不能撤销创建");
                    anyhow::ensure!(!trash.contains_key(&c.id), "回收站存在冲突，未覆盖");
                    let meta = index.remove(&c.id).unwrap();
                    self.check_body(&meta)?;
                    let mut root = None;
                    let mut parent = meta.parent.as_deref();
                    while let Some(p) = parent.filter(|p| deleted.contains(p)) {
                        root = Some(p.to_string());
                        parent = self.index[p].parent.as_deref();
                    }
                    trash.insert(c.id.clone(), Trashed { meta, deleted: deleted_at, root });
                }
                (None, Some(target)) => {
                    let saved = trash.remove(&c.id).context("恢复内容已不在回收站，无法重做或撤销")?;
                    let mut meta = saved.meta;
                    self.check_body(&meta)?;
                    meta.parent = target.parent.clone(); meta.order = target.order; meta.root_order = target.root_order;
                    index.insert(c.id.clone(), meta);
                }
                (Some(from), Some(to)) => {
                    let meta = index.get_mut(&c.id).unwrap();
                    if from.title != to.title { meta.name = Some(to.title.clone()); meta.modified = now(); }
                    if from.place() != to.place() { meta.parent = to.parent.clone(); meta.order = to.order; meta.root_order = to.root_order; }
                }
                _ => {}
            }
        }
        self.commit_virtual_metadata(index, trash)?;
        for (c, (from, to)) in pairs {
            if to.is_none() { crate::versions::move_to_trash(&self.history_base(), &c.id); }
            else if from.is_none() { crate::versions::restore_from_trash(&self.history_base(), &c.id); }
            self.parsed.remove(&c.id); self.forget_loaded(&c.id);
        }
        Ok(())
    }
}

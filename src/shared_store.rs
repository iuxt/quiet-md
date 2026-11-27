use crate::store::*;
use crate::model::Document;
use anyhow::Result;
use std::{cell::RefCell, rc::Rc, sync::{atomic::{AtomicBool, Ordering}, Arc}};

#[derive(Clone)]
pub struct SharedStore(Rc<RefCell<Store>>, Arc<AtomicBool>);
pub struct FileTaskGuard(Arc<AtomicBool>);

impl Drop for FileTaskGuard {
    fn drop(&mut self) { self.0.store(false, Ordering::Release); }
}

impl gpui::Global for SharedStore {}
impl SharedStore {
    pub fn open() -> Result<Self> { Ok(Self(Rc::new(RefCell::new(Store::open()?)), Arc::new(AtomicBool::new(false)))) }
    pub fn file_task_busy(&self) -> bool { self.1.load(Ordering::Acquire) }
    pub fn begin_file_task(&self) -> Option<FileTaskGuard> {
        self.1.compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed).is_ok().then(|| FileTaskGuard(self.1.clone()))
    }
    pub fn with<R>(&self, f: impl FnOnce(&mut Store) -> R) -> R { f(&mut self.0.borrow_mut()) }
    pub fn take_warnings(&self) -> Vec<String> { std::mem::take(&mut self.0.borrow_mut().warnings) }
    pub fn list(&mut self) -> Vec<Entry> { self.0.borrow_mut().list() }
    pub fn load(&self, id: &str) -> Result<Document> { self.0.borrow_mut().load(id) }
    pub fn load_for_edit(&self, id: &str, shared: bool) -> Result<Document> { self.0.borrow().load_for_edit(id, shared) }
    pub fn edit_baseline(&self, id: &str) -> Option<std::sync::Arc<String>> { self.0.borrow().edit_baseline(id) }
    pub fn resume_for_edit(&self, id: &str, baseline: Option<&std::sync::Arc<String>>, title: &str) -> Result<(Option<Document>, String)> {
        self.0.borrow().resume_for_edit(id, baseline, title)
    }
    pub fn forget_loaded(&self, id: &str) { self.0.borrow().forget_loaded(id); }
    pub fn recover_later(&self, doc: &Document) -> Result<()> { self.0.borrow().recover_later(doc) }
    pub fn save(&mut self, id: &str, doc: &Document) -> Result<Meta> { self.0.borrow_mut().save(id, doc) }
    pub fn create_from(&mut self, doc: &Document) -> Result<String> { self.0.borrow_mut().create_from(doc) }
    pub fn size_of(&self, id: &str, doc: &Document) -> u64 { self.0.borrow_mut().size_of(id, doc) }
    pub fn list_trash(&mut self) -> Vec<TrashEntry> { self.0.borrow_mut().list_trash() }
    pub fn list_trash_in(&mut self, parent: Option<&str>) -> Vec<TrashEntry> { self.0.borrow_mut().list_trash_in(parent) }
    pub fn trash_meta(&self, id: &str) -> Option<Meta> { self.0.borrow().trash_meta(id) }
    pub fn load_trash(&self, id: &str) -> Result<Document> { self.0.borrow().load_trash(id) }
    pub fn restore(&mut self, id: &str) -> Result<bool> { self.0.borrow_mut().restore(id) }
    pub fn set_width(&mut self, id: &str, width: PageWidth) -> Result<()> { self.0.borrow_mut().set_width(id, width) }
    pub fn update_meta(&mut self, id: &str, f: impl FnOnce(&mut Meta)) -> Result<()> { self.0.borrow_mut().update_meta(id, f) }
    pub fn set_catalogue(&mut self, id: &str, pinned: bool) -> Result<()> { self.0.borrow_mut().set_catalogue(id, pinned) }
    pub fn settings(&self) -> Settings { self.0.borrow_mut().settings() }
    pub fn save_settings(&self, s: &Settings) -> Result<()> { self.0.borrow_mut().save_settings(s) }
    pub fn scroll_top(&self, id: &str) -> Option<(usize, f32)> { self.0.borrow().scroll_top(id) }
    pub fn remember_scroll(&self, id: &str, top: Option<(usize, f32)>) -> Result<()> { self.0.borrow().remember_scroll(id, top) }
    pub fn history_versions(&self, id: &str) -> Vec<fdl_core::versions::Version> {
        let store = self.0.borrow();
        let mut versions = store.versions(id);
        if store.git_enabled() { versions.extend(store.git_versions(id).unwrap_or_default()); }
        versions.sort_by_key(|version| std::cmp::Reverse(version.ms));
        versions
    }
    pub fn version_diff(&self, id: &str, name: &str, baseline: Option<&str>) -> Result<Vec<fdl_core::diff::Line>> { self.0.borrow().version_diff(id, name, baseline) }
    pub fn version_document(&self, id: &str, name: &str) -> Result<Document> { self.0.borrow().version_document(id, name) }
    pub fn version_copy_document(&self, id: &str, name: &str) -> Result<Document> { self.0.borrow().version_copy_document(id, name) }
    pub fn set_version_label(&self, id: &str, name: &str, label: Option<&str>) -> Result<()> { self.0.borrow().set_version_label(id, name, label) }
    pub fn delete_version(&self, id: &str, name: &str) -> Result<()> { self.0.borrow().delete_version(id, name) }
    pub fn restore_version(&self, id: &str, name: &str) -> Result<Document> { self.0.borrow().restore_version(id, name) }
    pub fn save_version_now(&self, id: &str) -> Result<bool> { self.0.borrow().save_version_now(id) }
    pub fn rename(&mut self, id: &str, title: &str) -> Result<Meta> { self.0.borrow_mut().rename(id, title) }
    pub fn touch(&mut self, id: &str) -> Option<Meta> { self.0.borrow_mut().touch(id) }
    pub fn create_from_in(&mut self, doc: &Document, parent: Option<&str>) -> Result<String> { self.0.borrow_mut().create_from_in(doc, parent) }
    pub fn accept_file_title(&self, id: &str, title: &str) -> Result<()> { self.0.borrow().accept_file_title(id, title) }
    pub fn initialize_manual_order(&mut self) -> Result<()> { self.0.borrow_mut().initialize_manual_order() }
    pub fn track_file_changes(&mut self) -> FileChanges { self.0.borrow_mut().track_file_changes() }
    pub fn tree_writer(&self) -> Store { self.0.borrow().tree_writer() }
    pub fn receive_tree(&self, writer: Store) { self.0.borrow_mut().receive_tree(writer); }
    pub fn set_parent(&mut self, id: &str, parent: Option<&str>) -> Result<Meta> { self.0.borrow_mut().set_parent(id, parent) }
    pub fn set_starred(&mut self, id: &str, on: bool) -> Result<u64> { self.0.borrow_mut().set_starred(id, on) }
    pub fn set_pinned(&mut self, id: &str, pinned: bool) -> Result<()> { self.0.borrow_mut().set_pinned(id, pinned) }
    pub fn is_markdown(&self, id: &str) -> bool { self.0.borrow().is_markdown(id) }
    pub fn create_markdown_in(&mut self, doc: &Document, front: &str, parent: Option<&str>) -> Result<String> { self.0.borrow_mut().create_markdown_in(doc, front, parent) }
    pub fn front_matter_of(&self, id: &str) -> String { self.0.borrow().front_matter_of(id) }
    pub fn open_local(&mut self, path: &std::path::Path) -> Result<String> { self.0.borrow_mut().open_local(path) }
    pub fn local_path(&self, id: &str) -> Option<std::path::PathBuf> { self.0.borrow().local_path(id) }
    pub fn library_root(&self) -> std::path::PathBuf { self.0.borrow().library_root().to_path_buf() }
    pub fn file_path(&self, id: &str) -> Option<std::path::PathBuf> { self.0.borrow().file_path(id) }
}

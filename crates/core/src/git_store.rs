//! Git operations on UUID file names and shared library metadata.
use super::*;
use std::collections::BTreeSet;

impl Store {
    pub fn git_enabled(&self) -> bool { self.root.join(".git").is_dir() && self.root.join(crate::git::MANIFEST).is_file() }
    pub fn library_root(&self) -> &Path { &self.root }

    pub(super) fn initialize_library_git(&mut self) -> Result<()> {
        let path = self.root.join(".git");
        let _lock = self.begin_write()?;
        let repo = match path.symlink_metadata() {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {
                crate::git::update_internal_ignore(&self.root)?;
                crate::git::LibraryRepo::open(&self.root)?
            },
            Ok(_) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => crate::git::LibraryRepo::init(&self.root)?,
            Err(e) => return Err(e.into()),
        };
        repo.migrate_config()
    }

    pub fn git_commit(&mut self, message: &str) -> Result<Option<String>> {
        self.git_commit_restoring(message, &mut BTreeSet::new())
    }

    pub fn git_commit_restoring(&mut self, message: &str, restored: &mut BTreeSet<String>) -> Result<Option<String>> {
        let _lock = self.begin_write()?;
        let repo = crate::git::LibraryRepo::open(&self.root)?;
        let result = repo.commit_changes(message, crate::git::CommitScope::Auto, restored);
        for (id, meta) in &self.index {
            if restored.contains(&meta.rel) { self.parsed.remove(id); self.forget_loaded(id); }
        }
        result
    }

    pub fn git_stage(&mut self, path: &str, stage: bool) -> Result<()> {
        let _lock = self.begin_write()?;
        let repo = crate::git::LibraryRepo::open(&self.root)?;
        if stage { repo.stage(path) } else { repo.unstage(path) }
    }

    pub fn git_stage_all(&mut self, stage: bool) -> Result<()> {
        let _lock = self.begin_write()?;
        let repo = crate::git::LibraryRepo::open(&self.root)?;
        if stage { repo.stage_all() } else { repo.unstage_all() }
    }

    pub fn git_discard(&mut self, path: &str) -> Result<()> {
        let lock = self.begin_write()?;
        crate::git::LibraryRepo::open(&self.root)?.discard(path)?;
        self.parsed.clear();
        self.loaded.borrow_mut().clear(); self.loaded_titles.borrow_mut().clear();
        drop(lock);
        let _lock = self.begin_write()?;
        Ok(())
    }

    pub fn git_versions(&self, id: &str) -> Result<Vec<crate::versions::Version>> {
        crate::git::LibraryRepo::open(&self.root)?.history(Some(id), 80).map(|commits| commits.into_iter().map(|c| {
            let ms = c.time.max(0) as u64 * 1000;
            crate::versions::Version { name: format!("git:{ms}-{}", c.id), ms, size: 0, label: Some(format!("Git · {}", c.message)) }
        }).collect())
    }

    pub(super) fn git_version_document(&self, id: &str, commit: &str, preview: bool) -> Result<Document> {
        let repo = crate::git::LibraryRepo::open(&self.root)?;
        let text = repo.revision(commit, id)?;
        let mut doc = self.parse_doc(id, &text)?;
        if !self.is_markdown(id) && text.contains("href=") {
            let paths = repo.revision_paths(commit)?;
            let dir = paths.get(id).map(|p| parent_rel(p)).unwrap_or("");
            let by_path: HashMap<_, _> = paths.iter().map(|(id, path)| (path.as_str(), id.as_str())).collect();
            layout::each_inline(&mut doc, &mut |inline| {
                if let Some(link) = inline.doc_link().cloned() && let Some(href) = docxml::doc_link_href(&inline.xml)
                    && let Some(target) = layout::resolve(dir, &href).and_then(|p| by_path.get(p.as_str()).copied()) {
                    *inline = docxml::to_inline(crate::model::InlineKind::DocLink(crate::model::InlineDocLink { id: target.into(), title: link.title }));
                }
            });
        }
        if let Some(title) = repo.revision_title(commit, id)? && doc.title.text != title { doc.title = crate::model::RichText::plain(title); }
        let mut sources = doc.asset_sources_including_drawings(!preview);
        if let Some(cover) = &doc.cover { sources.push(cover.src.clone()); }
        let mut paths = HashMap::new();
        for source in sources {
            if let Some(path) = repo.revision_asset(commit, &source, preview)? { paths.insert(source, path); }
        }
        let drawing_paths = preview.then(|| {
            let mut drawings = HashMap::new();
            doc.for_each_block(|b| if let crate::model::BlockKind::Drawing(d) = &b.kind {
                for source in [&d.source, &d.preview].into_iter().filter(|s| !s.is_empty()) {
                    drawings.entry(source.clone()).or_insert_with(|| repo.revision_drawing_asset(commit, source).ok().flatten().unwrap_or_default());
                }
            });
            drawings
        });
        rewrite_asset_sources(&mut doc.blocks, &paths, drawing_paths.as_ref().unwrap_or(&paths));
        if let Some(cover) = &mut doc.cover && let Some(path) = paths.get(&cover.src) { cover.src = path.clone(); }
        Ok(doc)
    }

    pub fn enable_git(&mut self) -> Result<()> {
        crate::git::LibraryRepo::check_location(&self.root)?;
        let _lock = self.begin_write()?;
        crate::git::LibraryRepo::init(&self.root)?;
        Ok(())
    }

    /// Network and worktree writes share the same lock as normal document saves.
    pub fn git_pull(&mut self, auth: &crate::git::Auth) -> Result<bool> {
        let lock = self.begin_write()?;
        let changed = crate::git::LibraryRepo::open(&self.root)?.pull(auth)?;
        drop(lock);
        if changed {
            self.parsed.clear();
            self.loaded.borrow_mut().clear(); self.loaded_titles.borrow_mut().clear();
            // Reload both indexes before a caller refreshes its editors.
            let _lock = self.begin_write()?;
        }
        Ok(changed)
    }
}

pub(super) fn rewrite_asset_sources(blocks: &mut [crate::model::Block], paths: &HashMap<String, String>, drawings: &HashMap<String, String>) {
    use crate::model::{BlockKind, InlineKind};
    for block in blocks {
        match &mut block.kind {
            BlockKind::Image { src, .. } => if let Some(path) = paths.get(src) { *src = path.clone(); },
            BlockKind::Drawing(d) => for src in [&mut d.source, &mut d.preview] {
                if let Some(path) = drawings.get(src) { *src = path.clone(); }
            },
            BlockKind::File(file) => if let Some(path) = paths.get(&file.src) { file.src = path.clone(); },
            BlockKind::Table(table) => for cell in table.rows.iter_mut().flatten() { rewrite_asset_sources(&mut cell.blocks, paths, drawings); },
            _ => {},
        }
        for run in &mut block.text.runs {
            if let Some(inline) = &mut run.marks.inline && let InlineKind::File(file) = &inline.kind
                && let Some(path) = paths.get(&file.src) {
                let mut file = file.clone(); file.src = path.clone(); *inline = crate::docxml::to_inline(InlineKind::File(file));
            }
        }
        rewrite_asset_sources(&mut block.children, paths, drawings);
    }
}

pub(super) fn git_commit_name(name: &str) -> Option<&str> { name.strip_prefix("git:")?.split_once('-').map(|(_, id)| id) }
pub(super) fn version_ms(name: &str) -> u64 { crate::versions::stamp(name.strip_prefix("git:").unwrap_or(name)).unwrap_or(0) }

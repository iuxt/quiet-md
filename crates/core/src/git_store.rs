//! Opt-in UUID file names and shared metadata for a Git-backed library.
use super::*;
use std::collections::BTreeSet;

impl Store {
    pub fn git_enabled(&self) -> bool { !self.flat && self.root.join(".git").is_dir() && self.root.join(crate::git::MANIFEST).is_file() }
    pub fn library_root(&self) -> &Path { &self.root }

    pub(super) fn normalize_git_files(&mut self) -> Result<()> {
        let docs: Vec<_> = self.index.iter().filter(|(_, m)| !m.local() && m.folder.is_none() && !m.stable_name)
            .map(|(id, m)| (id.clone(), m.rel.clone(), join_rel(parent_rel(&m.rel), &format!("{id}.{}", ext_of(m))))).collect();
        if docs.is_empty() { return Ok(()) }
        for (id, _, _) in &docs {
            let title = self.load(id)?.title.text;
            let meta = self.index.get_mut(id).unwrap(); meta.stable_name = true; meta.name = Some(title);
        }
        self.relocate(&docs.iter().filter(|(_, a, b)| a != b).map(|(id, _, to)| (id.clone(), to.clone())).collect::<Vec<_>>())?;
        let paths = docs.into_iter().map(|(_, from, to)| (from, to)).collect();
        for (id, meta) in &self.index {
            if !meta.markdown || meta.local() { continue }
            let text = std::fs::read_to_string(self.path(id))?;
            let updated = rewrite_markdown_links(&text, parent_rel(&meta.rel), &paths);
            if updated != text { write_atomic(&self.path(id), updated.as_bytes())?; }
        }
        Ok(())
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
        anyhow::ensure!(!self.flat, "请先升级旧文档库格式");
        crate::git::LibraryRepo::check_location(&self.root)?;
        self.use_virtual_folders()?;
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

fn rewrite_markdown_links(text: &str, dir: &str, paths: &HashMap<String, String>) -> String {
    rebase_markdown_links(text, dir, dir, paths, false)
}

pub(super) fn rebase_markdown_links(text: &str, dir: &str, to_dir: &str, paths: &HashMap<String, String>, moving: bool) -> String {
    use pulldown_cmark::{Event, Options, Parser, Tag};
    let parser = Parser::new_ext(text, Options::all());
    let mut targets: Vec<(std::ops::Range<usize>, String)> = parser.reference_definitions().iter()
        .map(|(_, def)| (def.span.clone(), def.dest.to_string())).collect();
    for (event, span) in parser.into_offset_iter() {
        if let Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) = event { targets.push((span, dest_url.to_string())); }
    }
    let mut edits = BTreeMap::new();
    for (span, dest) in targets {
        let suffix = dest.find(['#', '?']).unwrap_or(dest.len());
        let decoded = percent_encoding::percent_decode_str(&dest[..suffix]).decode_utf8_lossy();
        if decoded.contains(':') { continue }
        let resolved = if moving && decoded.starts_with(".assets/") { Some(decoded.to_string()) } else { layout::resolve(dir, &decoded) };
        let Some(rel) = resolved else { continue };
        let to = match paths.get(&rel) { Some(to) => to.as_str(), None if moving => &rel, None => continue };
        let Some(at) = text[span.clone()].rfind(&dest) else { continue };
        let start = span.start + at;
        edits.insert(start, (start + dest.len(), format!("{}{}", layout::relative(to_dir, to), &dest[suffix..])));
    }
    let mut out = text.to_string();
    for (start, (end, dest)) in edits.into_iter().rev() { out.replace_range(start..end, &dest); }
    out
}

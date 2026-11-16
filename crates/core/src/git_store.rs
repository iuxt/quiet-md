//! Opt-in UUID file names and shared metadata for a Git-backed library.
use super::*;

impl Store {
    pub fn git_enabled(&self) -> bool { !self.flat && self.root.join(crate::git::MANIFEST).is_file() }
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
        let _lock = self.begin_write()?;
        crate::git::LibraryRepo::open(&self.root)?.commit(message)
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
        if self.is_markdown(id) && let Some(title) = repo.revision_title(commit, id)? { doc.title = crate::model::RichText::plain(title); }
        let mut sources = doc.asset_sources();
        if let Some(cover) = &doc.cover { sources.push(cover.src.clone()); }
        let mut paths = HashMap::new();
        for source in sources {
            let relative = source.strip_prefix(".assets/").map(|s| format!(".assets/{s}"))
                .or_else(|| source.split_once("/.assets/").map(|(_, s)| format!(".assets/{s}")));
            if let Some(relative) = relative { paths.insert(source, repo.revision_asset(commit, &relative, preview)?); }
        }
        rewrite_asset_sources(&mut doc.blocks, &paths);
        if let Some(cover) = &mut doc.cover && let Some(path) = paths.get(&cover.src) { cover.src = path.clone(); }
        Ok(doc)
    }

    pub fn enable_git(&mut self) -> Result<()> {
        let _lock = self.begin_write()?;
        anyhow::ensure!(!self.flat, "请先将旧文档库升级为文件夹格式");
        if self.git_enabled() { crate::git::LibraryRepo::open(&self.root)?; return Ok(()) }
        crate::git::LibraryRepo::check_location(&self.root)?;
        // A full source copy makes failed conversions reversible, including links.
        let mut backup = self.internal().join("before-git-5");
        if backup.exists() { backup = self.internal().join(format!("before-git-5-{}", uuid::Uuid::new_v4())); }
        std::fs::create_dir_all(&backup)?;
        let old_index = self.index.clone();
        let marker = self.internal().join("format.json");
        let old_marker = std::fs::read(&marker)?;
        let docs: Vec<(String, String, String)> = self.index.iter().filter(|(_, m)| !m.local() && m.folder.is_none())
            .map(|(id, m)| (id.clone(), m.rel.clone(), join_rel(parent_rel(&m.rel), &format!("{id}.{}", ext_of(m))))).collect();
        for (_, rel, _) in &docs {
            let to = backup.join(rel);
            std::fs::create_dir_all(to.parent().unwrap())?;
            std::fs::copy(self.abs(rel), to)?;
        }
        write_atomic(&backup.join("index.json"), &serde_json::to_vec_pretty(&self.to_disk())?)?;
        write_atomic(&backup.join("format.json"), &old_marker)?;
        let conversion = (|| -> Result<()> {
            // Older applications must refuse writes before paths start changing.
            write_atomic(&marker, &serde_json::to_vec_pretty(&FormatMarker { format: FORMAT })?)?;
            for (id, _, _) in &docs {
                let title = self.load(id)?.title.text;
                let meta = self.index.get_mut(id).unwrap();
                meta.stable_name = true;
                meta.name = Some(title);
            }
            let moves: Vec<_> = docs.iter().filter(|(_, a, b)| a != b).map(|(id, _, to)| (id.clone(), to.clone())).collect();
            self.relocate(&moves)?;
            let paths: HashMap<String, String> = docs.iter().map(|(_, from, to)| (from.clone(), to.clone())).collect();
            for (id, from, _) in &docs {
                if !self.is_markdown(id) { continue }
                let original = std::fs::read_to_string(backup.join(from))?;
                let updated = rewrite_markdown_links(&original, parent_rel(from), &paths);
                if updated != original { write_atomic(&self.path(id), updated.as_bytes())?; }
            }
            save_json(&self.root.join(crate::git::MANIFEST), &self.to_disk())?;
            crate::git::LibraryRepo::init(&self.root)?;
            Ok(())
        })();
        if let Err(error) = conversion {
            let mut restored = true;
            for (_, from, to) in &docs {
                let result = std::fs::copy(backup.join(from), self.abs(from));
                restored &= result.is_ok();
                if from != to && result.is_ok() { let _ = std::fs::remove_file(self.abs(to)); }
            }
            self.index = old_index;
            let _ = std::fs::remove_file(self.root.join(crate::git::MANIFEST));
            let _ = save_json(&self.internal().join("index.json"), &self.to_disk());
            if restored { let _ = write_atomic(&marker, &old_marker); }
            return Err(error.context(format!("Git 转换未完成，原文件备份在 {}", backup.display())));
        }
        self.parsed.clear();
        self.loaded.borrow_mut().clear();
        Ok(())
    }

    /// Network and worktree writes share the same lock as normal document saves.
    pub fn git_pull(&mut self, auth: &crate::git::Auth) -> Result<bool> {
        let lock = self.begin_write()?;
        let changed = crate::git::LibraryRepo::open(&self.root)?.pull(auth)?;
        drop(lock);
        if changed {
            self.parsed.clear();
            self.loaded.borrow_mut().clear();
            // Reload both indexes before a caller refreshes its editors.
            let _lock = self.begin_write()?;
        }
        Ok(changed)
    }
}

fn rewrite_asset_sources(blocks: &mut [crate::model::Block], paths: &HashMap<String, String>) {
    use crate::model::{BlockKind, InlineKind};
    for block in blocks {
        match &mut block.kind {
            BlockKind::Image { src, .. } => if let Some(path) = paths.get(src) { *src = path.clone(); },
            BlockKind::File(file) => if let Some(path) = paths.get(&file.src) { file.src = path.clone(); },
            BlockKind::Table(table) => for cell in table.rows.iter_mut().flatten() { rewrite_asset_sources(&mut cell.blocks, paths); },
            _ => {},
        }
        for run in &mut block.text.runs {
            if let Some(inline) = &mut run.marks.inline && let InlineKind::File(file) = &inline.kind
                && let Some(path) = paths.get(&file.src) {
                let mut file = file.clone(); file.src = path.clone(); *inline = crate::docxml::to_inline(InlineKind::File(file));
            }
        }
        rewrite_asset_sources(&mut block.children, paths);
    }
}

pub(super) fn git_commit_name(name: &str) -> Option<&str> { name.strip_prefix("git:")?.split_once('-').map(|(_, id)| id) }
pub(super) fn version_ms(name: &str) -> u64 { crate::versions::stamp(name.strip_prefix("git:").unwrap_or(name)).unwrap_or(0) }

fn rewrite_markdown_links(text: &str, dir: &str, paths: &HashMap<String, String>) -> String {
    use pulldown_cmark::{Event, Options, Parser, Tag};
    let parser = Parser::new_ext(text, Options::all());
    let mut targets: Vec<(std::ops::Range<usize>, String)> = parser.reference_definitions().iter()
        .map(|(_, def)| (def.span.clone(), def.dest.to_string())).collect();
    for (event, span) in parser.into_offset_iter() {
        if let Event::Start(Tag::Link { dest_url, .. }) = event { targets.push((span, dest_url.to_string())); }
    }
    let mut edits = BTreeMap::new();
    for (span, dest) in targets {
        let suffix = dest.find(['#', '?']).unwrap_or(dest.len());
        let decoded = percent_encoding::percent_decode_str(&dest[..suffix]).decode_utf8_lossy();
        let Some(to) = layout::resolve(dir, &decoded).and_then(|rel| paths.get(&rel)) else { continue };
        let Some(at) = text[span.clone()].rfind(&dest) else { continue };
        let start = span.start + at;
        edits.insert(start, (start + dest.len(), format!("{}{}", layout::relative(dir, to), &dest[suffix..])));
    }
    let mut out = text.to_string();
    for (start, (end, dest)) in edits.into_iter().rev() { out.replace_range(start..end, &dest); }
    out
}

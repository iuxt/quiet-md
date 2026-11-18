//! Read-only comparisons against the same managed tree used by commits.
use super::*;
use crate::diff::{self, Line};

pub struct ChangePreview {
    pub title: String,
    pub lines: Vec<Line>,
    pub source: Option<Vec<Line>>,
    pub formatted: Option<diff::FormattedDiff>,
    pub binary: bool,
    pub truncated: bool,
    pub source_truncated: bool,
}

impl LibraryRepo {
    pub fn change_preview(&self, path: &str) -> Result<ChangePreview> {
        safe_path(&self.root, path)?;
        let before = self.repo.head().ok().and_then(|h| h.peel_to_tree().ok());
        let after = self.repo.find_tree(self.working_tree(false)?)?;
        let old = before.as_ref().and_then(|t| t.get_path(Path::new(path)).ok());
        let new = after.get_path(Path::new(path)).ok();
        ensure!(old.is_some() || new.is_some(), "这个文件不在 Git 管理的文档库中");
        let read = |entry: Option<git2::TreeEntry<'_>>| -> Result<Vec<u8>> {
            let Some(entry) = entry else { return Ok(vec![]) };
            let blob = self.repo.find_blob(entry.id())?;
            ensure!(blob.size() <= crate::store::EXPORT_FILE_LIMIT as usize, "文件过大，无法对比");
            Ok(blob.content().to_vec())
        };
        let old = read(old)?; let new = read(new)?;
        let metadata = |tree: &git2::Tree<'_>| -> Option<String> {
            let entry = tree.get_name(MANIFEST)?;
            let blob = self.repo.find_blob(entry.id()).ok()?;
            manifest(blob.content()).ok()?.get(path)?.extra.get("title")?.as_str().map(str::to_string)
        };
        let title = metadata(&after).or_else(|| before.as_ref().and_then(metadata)).unwrap_or_else(|| path.to_string());
        // LFS blobs are text pointers, but assets are never shown as text documents.
        let binary = path.starts_with(".assets/") || old.contains(&0) || new.contains(&0)
            || std::str::from_utf8(&old).is_err() || std::str::from_utf8(&new).is_err();
        if binary { return Ok(ChangePreview { title, lines: vec![], source: None, formatted: None, binary, truncated: false, source_truncated: false }); }
        let old = std::str::from_utf8(&old)?; let new = std::str::from_utf8(&new)?;
        let xml = path.ends_with(".xml");
        let old_text = diff::readable(old, !xml); let new_text = diff::readable(new, !xml);
        let exceeds = |a: &str, b: &str| a.lines().count() > diff::MAX_LINES || b.lines().count() > diff::MAX_LINES;
        Ok(ChangePreview { title, lines: diff::diff(&old_text, &new_text, true),
            source: xml.then(|| diff::diff(old, new, true)), formatted: if xml { diff::formatted(old, new) } else { None }, binary,
            truncated: exceeds(&old_text, &new_text), source_truncated: xml && exceeds(old, new) })
    }
}

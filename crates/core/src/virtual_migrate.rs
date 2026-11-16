//! Copy-first conversion. The journal makes interrupted conversions recoverable.
use super::*;
use crate::manifest::{Manifest, FILE, NOTES};

const JOURNAL: &str = ".quiet/virtual-migration.json";
#[derive(Serialize, Deserialize)]
struct Migration {
    backup: String, originals: Vec<String>, destinations: Vec<String>, folders: Vec<String>,
    marker: Vec<u8>, manifest: Option<Vec<u8>>,
}

impl Store {
    pub fn use_virtual_folders(&mut self) -> Result<()> {
        if self.virtual_folders() { return Ok(()) }
        anyhow::ensure!(!self.flat, "请先升级旧文档库格式");
        self.list();
        let lock = self.begin_write()?;
        if self.virtual_folders() { return Ok(()) }
        let original_index = self.index.clone();
        let original_trash = self.trash.clone();
        let mut index = original_index.clone(); let mut trash = original_trash.clone();
        let mut paths = HashMap::new();
        for (id, meta) in &mut index {
            if meta.local() { continue }
            if meta.folder.is_some() { meta.rel.clear(); continue }
            meta.name = Some(self.load(id)?.title.text);
            let rel = format!("{NOTES}/{id}.{}", ext_of(meta));
            paths.insert(meta.rel.clone(), rel.clone()); meta.rel = rel; meta.stable_name = true;
        }
        for (id, saved) in &mut trash {
            if saved.meta.folder.is_some() { saved.meta.rel.clear(); continue }
            let source = self.trash_path(id);
            if saved.meta.name.is_none() && source.is_file() && !saved.meta.markdown {
                saved.meta.name = Some(docxml::parse(&std::fs::read_to_string(source)?)?.title.text);
            }
            let rel = format!("{NOTES}/{id}.{}", ext_of(&saved.meta));
            paths.insert(saved.meta.rel.clone(), rel.clone()); saved.meta.rel = rel; saved.meta.stable_name = true;
        }
        let mut outputs = vec![];
        // Resolve XML mentions before replacing the physical path index.
        for (id, old) in original_index.iter().filter(|(_, m)| !m.local() && m.folder.is_none()) {
            let text = std::fs::read_to_string(self.path(id))?;
            let out = if old.markdown {
                super::git_store::rebase_markdown_links(&text, parent_rel(&old.rel), NOTES, &paths, true)
            } else {
                let mut doc = self.parse_doc(id, &text)?;
                layout::each_inline(&mut doc, &mut |inline| {
                    if let Some(link) = inline.doc_link().cloned() && let Some(target) = index.get(&link.id).or_else(|| trash.get(&link.id).map(|t| &t.meta)) {
                        inline.xml = docxml::doc_link_href_xml(&layout::relative(NOTES, &target.rel), &link.title);
                    }
                });
                docxml::serialize(&doc)
            };
            outputs.push((old.rel.clone(), index[id].rel.clone(), out));
        }
        for (id, old) in original_trash.iter().filter(|(_, t)| t.meta.folder.is_none()) {
            let rel = format!(".quiet/trash/{id}.{}", ext_of(&old.meta));
            if !self.abs(&rel).is_file() { continue }
            let text = std::fs::read_to_string(self.abs(&rel))?;
            let out = if old.meta.markdown {
                super::git_store::rebase_markdown_links(&text, parent_rel(&old.meta.rel), NOTES, &paths, true)
            } else {
                let mut doc = docxml::parse(&text)?;
                layout::each_inline(&mut doc, &mut |inline| {
                    let Some(link) = inline.doc_link().cloned() else { return };
                    let href = docxml::doc_link_href(&inline.xml);
                    let target = href.and_then(|h| layout::resolve(parent_rel(&old.meta.rel), &h)).and_then(|p| paths.get(&p))
                        .or_else(|| index.get(&link.id).map(|m| &m.rel)).or_else(|| trash.get(&link.id).map(|t| &t.meta.rel));
                    if let Some(target) = target { inline.xml = docxml::doc_link_href_xml(&layout::relative(NOTES, target), &link.title); }
                });
                docxml::serialize(&doc)
            };
            outputs.push((rel.clone(), rel, out));
        }
        let backup = format!(".quiet/before-virtual-6-{}", uuid::Uuid::new_v4());
        let state = Migration { backup, originals: outputs.iter().map(|(p, _, _)| p.clone()).collect(),
            destinations: outputs.iter().map(|(_, p, _)| p.clone()).collect(),
            folders: original_index.values().filter(|m| m.folder.is_some()).map(|m| m.rel.clone()).collect(),
            marker: std::fs::read(self.internal().join("format.json"))?, manifest: std::fs::read(self.root.join(FILE)).ok() };
        let destination_set: HashSet<_> = state.destinations.iter().collect();
        let original_set: HashSet<_> = state.originals.iter().collect();
        for path in &destination_set {
            let target = crate::git::safe_path(&self.root, path)?;
            anyhow::ensure!(!target.exists() || original_set.contains(path), "迁移目标已存在，未覆盖：{path}");
        }
        for rel in state.originals.iter().map(String::as_str).chain([".quiet/index.json", ".quiet/trash/index.json", ".quiet/format.json", FILE]) {
            let from = crate::git::safe_path(&self.root, rel)?;
            if !from.is_file() { continue }
            let to = self.root.join(&state.backup).join(rel);
            std::fs::create_dir_all(to.parent().unwrap())?; write_atomic(&to, &std::fs::read(from)?)?;
        }
        let manifest = self.virtual_manifest_for(&index, &trash);
        manifest.validate()?;
        save_json(&self.root.join(JOURNAL), &state)?;
        let converted = (|| -> Result<()> {
            std::fs::create_dir_all(crate::git::safe_path(&self.root, NOTES)?)?;
            for (_, to, text) in &outputs { write_atomic(&crate::git::safe_path(&self.root, to)?, text.as_bytes())?; }
            save_json(&self.root.join(FILE), &manifest)?;
            write_atomic(&self.internal().join("format.json"), &serde_json::to_vec_pretty(&FormatMarker { format: FORMAT })?)?;
            Ok(())
        })();
        if let Err(error) = converted {
            rollback(&self.root, &state)?;
            return Err(error.context("虚拟文件夹迁移未完成，原文档已恢复"));
        }
        finish(&self.root, &state)?;
        self.parsed.clear(); self.loaded.borrow_mut().clear(); self.loaded_titles.borrow_mut().clear();
        drop(lock);
        self.open_virtual()
    }
}

pub(super) fn recover(root: &Path) -> Result<()> {
    if !root.join(JOURNAL).exists() { return Ok(()) }
    let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(root.join(".quiet/.lock"))?;
    file.lock()?;
    if !root.join(JOURNAL).exists() { return Ok(()) }
    let state: Migration = serde_json::from_slice(&std::fs::read(root.join(JOURNAL))?)?;
    anyhow::ensure!(state.backup.starts_with(".quiet/before-virtual-6-"), "迁移备份路径无效");
    crate::git::safe_path(root, &state.backup)?;
    let committed = reread::<FormatMarker>(&root.join(".quiet/format.json")).is_some_and(|m| m.format == FORMAT)
        && std::fs::read(root.join(FILE)).is_ok_and(|b| Manifest::parse(&b).is_ok());
    if committed { finish(root, &state) } else { rollback(root, &state) }
}

fn finish(root: &Path, state: &Migration) -> Result<()> {
    for rel in &state.originals {
        if !state.destinations.contains(rel) { let path = crate::git::safe_path(root, rel)?; if path.is_file() { std::fs::remove_file(path)?; } }
    }
    let mut folders = state.folders.clone(); folders.sort_by_key(|p| std::cmp::Reverse(p.len()));
    for rel in folders { let _ = std::fs::remove_dir(crate::git::safe_path(root, &rel)?); }
    for rel in [".quiet/index.json", ".quiet/trash/index.json"] { let _ = std::fs::remove_file(root.join(rel)); let _ = std::fs::remove_file(root.join(format!("{rel}.bak"))); }
    std::fs::remove_file(root.join(JOURNAL))?; Ok(())
}

fn rollback(root: &Path, state: &Migration) -> Result<()> {
    for rel in &state.destinations { if !state.originals.contains(rel) { let path = crate::git::safe_path(root, rel)?; if path.is_file() { std::fs::remove_file(path)?; } } }
    for rel in &state.originals {
        let from = crate::git::safe_path(root, &format!("{}/{rel}", state.backup))?;
        let to = crate::git::safe_path(root, rel)?;
        std::fs::create_dir_all(to.parent().unwrap())?; std::fs::copy(from, to)?;
    }
    if let Some(bytes) = &state.manifest { write_atomic(&root.join(FILE), bytes)?; }
    else if root.join(FILE).exists() { std::fs::remove_file(root.join(FILE))?; }
    write_atomic(&root.join(".quiet/format.json"), &state.marker)?;
    std::fs::remove_file(root.join(JOURNAL))?; Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_migration_rolls_back_or_finishes_according_to_marker() {
        for committed in [false, true] {
            let root = std::env::temp_dir().join(format!("quiet-migration-recovery-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(root.join(".quiet/before-virtual-6-test/Old")).unwrap();
            std::fs::create_dir_all(root.join("Old")).unwrap(); std::fs::create_dir_all(root.join(NOTES)).unwrap();
            let id = uuid::Uuid::new_v4().to_string(); let to = format!("notes/{id}.xml");
            let body = "<title>Original</title>\n<p>Preserve me.</p>\n";
            std::fs::write(root.join("Old/note.xml"), body).unwrap();
            std::fs::write(root.join(".quiet/before-virtual-6-test/Old/note.xml"), body).unwrap();
            std::fs::write(root.join(&to), body).unwrap();
            let mut index = HashMap::new(); index.insert(id, Meta { rel: to.clone(), name: Some("Original".into()), stable_name: true, ..Default::default() });
            let manifest = Manifest::default().update(&index, std::iter::empty());
            save_json(&root.join(FILE), &manifest).unwrap();
            let state = Migration { backup: ".quiet/before-virtual-6-test".into(), originals: vec!["Old/note.xml".into()],
                destinations: vec![to.clone()], folders: vec!["Old".into()], marker: b"{\"format\":5}".to_vec(), manifest: None };
            save_json(&root.join(JOURNAL), &state).unwrap();
            std::fs::write(root.join(".quiet/format.json"), if committed { b"{\"format\":6}" } else { b"{\"format\":5}" }).unwrap();
            recover(&root).unwrap(); assert!(!root.join(JOURNAL).exists());
            if committed {
                assert!(!root.join("Old").exists()); assert_eq!(std::fs::read_to_string(root.join(to)).unwrap(), body);
                assert!(Store::open_at(root.clone()).unwrap().virtual_folders());
            } else {
                assert!(!root.join(to).exists()); assert!(!root.join(FILE).exists());
                assert_eq!(std::fs::read_to_string(root.join("Old/note.xml")).unwrap(), body);
            }
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}

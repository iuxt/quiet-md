//! Copy-first conversion. The journal makes interrupted conversions recoverable.
use super::*;
use crate::manifest::{Manifest, FILE, NOTES, NOTES_FORMAT, OLD_NOTES, OLD_NOTES_FORMAT, notes_dir};

const JOURNAL: &str = ".quiet/virtual-migration.json";
const NOTES_PENDING: &str = ".quiet-notes-format-10";

pub(super) fn has_old_notes(root: &Path) -> Result<bool> {
    Ok(exact_entry(root, OLD_NOTES)? || exact_entry(root, NOTES_PENDING)?)
}

// Directory lookups can alias on macOS and Windows. The intermediate
// spelling both forces the new name and lets the next open finish the move.
fn exact_entry(root: &Path, name: &str) -> Result<bool> {
    for entry in std::fs::read_dir(root)? {
        if entry?.file_name() == name { return Ok(true) }
    }
    Ok(false)
}

pub(super) fn rename_notes(root: &Path, from: &str, to: &str) -> Result<()> {
    let source = exact_entry(root, from)?;
    let pending = exact_entry(root, NOTES_PENDING)?;
    let target = exact_entry(root, to)?;
    anyhow::ensure!(!(source && target || pending && (source || target)), "正文目录重名，未覆盖：{from} / {to} / {NOTES_PENDING}");
    if target { anyhow::ensure!(crate::git::safe_path(root, to)?.is_dir(), "正文路径不是目录：{to}"); }
    if !source && !pending { return Ok(()) }
    let intermediate = crate::git::safe_path(root, NOTES_PENDING)?;
    if source {
        let source = crate::git::safe_path(root, from)?;
        anyhow::ensure!(source.is_dir() && !intermediate.exists(), "无法迁移正文目录：{from}");
        std::fs::rename(source, &intermediate)?;
    }
    anyhow::ensure!(intermediate.is_dir() && !root.join(to).exists(), "无法迁移正文目录：{to}");
    std::fs::rename(intermediate, root.join(to))?;
    #[cfg(unix)]
    std::fs::File::open(root)?.sync_all()?;
    Ok(())
}
pub(crate) fn rename_layout(root: &Path, from: u32, to: u32) -> Result<()> {
    let mut assets = vec![];
    if (from < PLURAL_RESOURCES_FORMAT) != (to < PLURAL_RESOURCES_FORMAT) {
        for (old, new) in RESOURCE_RENAMES {
            let (source, target) = if to >= PLURAL_RESOURCES_FORMAT { (old, new) } else { (new, old) };
            let src = crate::git::safe_path(root, source)?;
            let dst = crate::git::safe_path(root, target)?;
            anyhow::ensure!(!dst.exists() || dst.is_dir(), "资源路径不是目录：{target}");
            if src.exists() {
                anyhow::ensure!(src.is_dir() && !dst.exists(), "资源目录重名，未覆盖：{source} / {target}");
                assets.push((src, dst));
            }
        }
    }
    if notes_dir(from) != notes_dir(to) { rename_notes(root, notes_dir(from), notes_dir(to))?; }
    for (src, dst) in assets { std::fs::rename(src, dst)?; }
    #[cfg(unix)]
    std::fs::File::open(root)?.sync_all()?;
    Ok(())
}

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
        anyhow::ensure!(self.trash.is_empty(), "请先手动处理旧版回收站");
        let mut index = original_index.clone(); let trash = HashMap::new();
        // A physical folder can already have either spelling. Finish the
        // virtual conversion in place before renaming the lowercase directory.
        let (notes, format) = if exact_entry(&self.root, OLD_NOTES)? && !exact_entry(&self.root, NOTES)? {
            (OLD_NOTES, OLD_NOTES_FORMAT)
        } else { (NOTES, NOTES_FORMAT) };
        let mut paths = HashMap::new();
        for (id, meta) in &mut index {
            if meta.local() { continue }
            if meta.folder.is_some() { meta.rel.clear(); continue }
            meta.name = Some(self.load(id)?.title.text);
            let rel = format!("{notes}/{id}.{}", ext_of(meta));
            paths.insert(meta.rel.clone(), rel.clone()); meta.rel = rel; meta.stable_name = true;
        }
        let mut outputs = vec![];
        // Resolve XML mentions before replacing the physical path index.
        for (id, old) in original_index.iter().filter(|(_, m)| !m.local() && m.folder.is_none()) {
            let text = std::fs::read_to_string(self.path(id))?;
            let out = if old.markdown {
                super::git_store::rebase_markdown_links(&text, parent_rel(&old.rel), notes, &paths, true)
            } else {
                let mut doc = self.parse_doc(id, &text)?;
                layout::each_inline(&mut doc, &mut |inline| {
                    if let Some(link) = inline.doc_link().cloned() && let Some(target) = index.get(&link.id) {
                        inline.xml = docxml::doc_link_href_xml(&layout::relative(notes, &target.rel), &link.title);
                    }
                });
                docxml::serialize(&doc)
            };
            outputs.push((old.rel.clone(), index[id].rel.clone(), out));
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
        for rel in state.originals.iter().map(String::as_str).chain([".quiet/index.json", ".quiet/format.json", FILE]) {
            let from = crate::git::safe_path(&self.root, rel)?;
            if !from.is_file() { continue }
            let to = self.root.join(&state.backup).join(rel);
            std::fs::create_dir_all(to.parent().unwrap())?; write_atomic(&to, &std::fs::read(from)?)?;
        }
        let manifest = self.virtual_manifest_for(&index, &trash);
        manifest.validate()?;
        save_json(&self.root.join(JOURNAL), &state)?;
        let converted = (|| -> Result<()> {
            std::fs::create_dir_all(crate::git::safe_path(&self.root, notes)?)?;
            for (_, to, text) in &outputs { write_atomic(&crate::git::safe_path(&self.root, to)?, text.as_bytes())?; }
            save_json(&self.root.join(FILE), &manifest)?;
            write_atomic(&self.internal().join("format.json"), &serde_json::to_vec_pretty(&FormatMarker { format })?)?;
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
    let committed = Store::format_at(root).is_some_and(virtual_format)
        && std::fs::read(root.join(FILE)).is_ok_and(|b| Manifest::parse(&b).is_ok());
    if committed { finish(root, &state) } else { rollback(root, &state) }
}

fn finish(root: &Path, state: &Migration) -> Result<()> {
    for rel in &state.originals {
        if !state.destinations.contains(rel) { let path = crate::git::safe_path(root, rel)?; if path.is_file() { std::fs::remove_file(path)?; } }
    }
    let mut folders = state.folders.clone(); folders.sort_by_key(|p| std::cmp::Reverse(p.len()));
    for rel in folders { let _ = std::fs::remove_dir(crate::git::safe_path(root, &rel)?); }
    let _ = std::fs::remove_file(root.join(".quiet/index.json")); let _ = std::fs::remove_file(root.join(".quiet/index.json.bak"));
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
            let id = uuid::Uuid::new_v4().to_string(); let to = format!("{NOTES}/{id}.xml");
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
            std::fs::write(root.join(".quiet/format.json"), if committed { "{\"format\":10}" } else { "{\"format\":5}" }).unwrap();
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

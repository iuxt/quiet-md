//! FORMAT 6: virtual folders, one shared manifest, fixed notes/<UUID> files.
use super::*;
use crate::manifest::{Deleted, Manifest, FILE, NOTES};

const TRANSACTION: &str = ".quiet/virtual-transaction.json";

#[derive(Serialize, Deserialize)]
struct FileTransaction { before: Manifest, after: Manifest, moves: Vec<(String, String)> }

impl Store {
    pub(super) fn manifest_title(&self, id: &str, doc: &mut Document) {
        if self.virtual_folders() && let Some(meta) = self.index.get(id).filter(|m| !m.local()) && let Some(title) = &meta.name && doc.title.text != *title {
            doc.title = crate::model::RichText::plain(title);
        }
    }

    pub fn virtual_folders(&self) -> bool { self.manifest.is_some() }

    pub(super) fn open_virtual(&mut self) -> Result<()> {
        let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(self.internal().join(".lock"))?;
        file.lock()?;
        let _lock = WriteLock(Some(file));
        self.reload_virtual()?;
        self.load_parsed();
        Ok(())
    }

    pub(super) fn reload_virtual(&mut self) -> Result<()> {
        self.recover_virtual_files()?;
        let path = crate::git::safe_path(&self.root, FILE)?;
        let manifest = match std::fs::read(&path).map_err(anyhow::Error::from).and_then(|b| Manifest::parse(&b)) {
            Ok(m) => m,
            Err(error) => {
                let bytes = std::fs::read(path.with_extension("json.bak")).context("文档库清单及其备份不可用，未改动正文文件")?;
                let backup = Manifest::parse(&bytes)?;
                if path.exists() { std::fs::copy(&path, path.with_extension(format!("json.corrupt-{}", crate::time::now_ms())))?; }
                write_atomic(&path, &bytes)?;
                self.warnings.push(format!("文档库清单已从备份恢复：{error}")); backup
            },
        };
        let local = self.read_local();
        self.index = manifest.index();
        for (id, m) in &mut self.index { m.opened = local.opened.get(id).copied().unwrap_or(0); }
        for (id, meta) in local.files {
            anyhow::ensure!(!self.index.contains_key(&id), "本机文件与文档库 UUID 冲突");
            self.index.insert(id, meta);
        }
        self.trash = manifest.trash.iter().map(|t| (t.id.clone(), Trashed { meta: Manifest::deleted_meta(t), deleted: t.deleted, root: t.root.clone() })).collect();
        self.manifest = Some(manifest);
        for rel in [NOTES, ".quiet/trash"] { std::fs::create_dir_all(crate::git::safe_path(&self.root, rel)?)?; }
        Ok(())
    }

    pub(super) fn virtual_manifest_for(&self, index: &HashMap<String, Meta>, trash: &HashMap<String, Trashed>) -> Manifest {
        let original = self.manifest.as_ref().cloned().unwrap_or_default();
        let extras: HashMap<_, _> = original.notes.iter().map(|n| (&n.id, &n.extra)).chain(original.folders.iter().map(|f| (&f.id, &f.extra)))
            .chain(original.trash.iter().map(|t| (&t.id, &t.extra))).collect();
        let deleted = trash.iter().map(|(id, t)| Deleted {
            id: id.clone(), title: t.meta.folder.clone().or_else(|| t.meta.name.clone()).unwrap_or_default(),
            filename: t.meta.folder.is_none().then(|| format!("{id}.{}", ext_of(&t.meta))), parent: t.meta.parent.clone(),
            created: t.meta.created, modified: t.meta.modified, deleted: t.deleted, root: t.root.clone(),
            options: crate::manifest::Options::of(&t.meta),
            extra: extras.get(id).map(|e| (*e).clone()).unwrap_or_default(),
        });
        original.update(index, deleted)
    }

    pub(super) fn save_virtual(&self) -> Result<()> {
        let manifest = self.virtual_manifest_for(&self.index, &self.trash);
        manifest.validate()?;
        save_json(&crate::git::safe_path(&self.root, FILE)?, &manifest)?;
        self.save_local(&self.local_state())
    }

    pub(super) fn list_virtual(&mut self, locked: bool) -> Vec<Entry> {
        if locked && let Err(error) = self.adopt_virtual_notes() { self.warnings.push(format!("读取正文目录失败：{error}")); }
        let mut out = vec![]; let mut reparsed = false;
        let mut ids: Vec<_> = self.index.keys().cloned().collect(); ids.sort();
        for id in ids {
            let meta = self.index[&id].clone();
            if let Some(title) = &meta.folder {
                out.push(Entry { id, meta: meta.clone(), title: title.clone(), icon: None, body: Default::default(), reminders: vec![], links: vec![] });
                continue;
            }
            let path = self.path(&id);
            let stamp = std::fs::metadata(&path).ok().and_then(|m| Some((m.modified().ok()?, m.len())));
            let parsed = self.parsed.get(&id).filter(|c| stamp == Some((c.0, c.1))).map(|c| c.2.clone())
                .or_else(|| std::fs::read_to_string(&path).ok().and_then(|text| self.parse_doc(&id, &text).ok()).map(|doc| {
                    let parsed = Parsed { title: doc.title.text.clone(), icon: doc.icon.clone(), body: doc.plain_text().into(), reminders: doc.reminders(), links: doc.doc_links() };
                    if let Some((time, len)) = stamp { self.parsed.insert(id.clone(), (time, len, parsed.clone())); reparsed = true; }
                    parsed
                })).unwrap_or(Parsed { title: String::new(), icon: None, body: Default::default(), reminders: vec![], links: vec![] });
            if meta.local() && stamp.is_none() { continue }
            let title = if meta.local() { self.markdown_title(&id) } else { meta.name.clone().unwrap_or(parsed.title) };
            out.push(Entry { id, meta, title, icon: parsed.icon, body: parsed.body, reminders: parsed.reminders, links: parsed.links });
        }
        let cached = self.parsed.len();
        let seen: HashSet<_> = out.iter().map(|e| e.id.as_str()).collect(); self.parsed.retain(|id, _| seen.contains(id.as_str()));
        if reparsed || self.parsed.len() != cached { let _ = self.save_parsed(); }
        out
    }

    fn adopt_virtual_notes(&mut self) -> Result<()> {
        let known: HashSet<_> = self.index.values().filter(|m| !m.local() && m.folder.is_none()).map(|m| m.rel.clone()).collect();
        let mut dirty = false;
        for entry in std::fs::read_dir(self.root.join(NOTES))? {
            let entry = entry?;
            anyhow::ensure!(!entry.file_type()?.is_symlink(), "正文文件不能使用符号链接");
            if !entry.file_type()?.is_file() { continue }
            let name = entry.file_name().into_string().map_err(|_| anyhow::anyhow!("正文文件名不是 UTF-8"))?;
            let markdown = match entry.path().extension().and_then(|s| s.to_str()) { Some("md") => true, Some("xml") => false, _ => continue };
            if known.contains(&format!("{NOTES}/{name}")) { continue }
            let stem = Path::new(&name).file_stem().unwrap().to_string_lossy().into_owned();
            let id = uuid::Uuid::parse_str(&stem).map(|u| u.to_string()).unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
            anyhow::ensure!(!self.index.contains_key(&id) && !self.trash.contains_key(&id), "正文 UUID 冲突，未覆盖：{id}");
            let text = std::fs::read_to_string(entry.path())?;
            let title = if markdown { stem } else { docxml::parse(&text)?.title.text };
            let rel = format!("{NOTES}/{id}.{}", if markdown { "md" } else { "xml" });
            if rel != format!("{NOTES}/{name}") {
                let to = self.uuid_rel(NOTES, &id, if markdown { "md" } else { "xml" })?;
                std::fs::rename(entry.path(), self.abs(&to))?;
            }
            let t = now();
            self.index.insert(id, Meta { created: t, modified: t, name: Some(title), rel, markdown, stable_name: true, ..Default::default() });
            dirty = true;
        }
        if dirty { self.save_index()?; }
        Ok(())
    }

    pub(super) fn trash_virtual(&mut self, items: &[(String, Option<String>)], record: bool) -> Result<()> {
        let mut index = self.index.clone(); let mut trash = self.trash.clone(); let mut moves = vec![]; let mut before = vec![];
        for (id, root) in items {
            let meta = index.remove(id).context("文档或文件夹不存在")?;
            if record { before.push((id.clone(), self.file_state(id))); }
            if meta.folder.is_none() { moves.push((meta.rel.clone(), format!(".quiet/trash/{id}.{}", ext_of(&meta)))); }
            anyhow::ensure!(!trash.contains_key(id), "回收站 UUID 冲突，未覆盖");
            trash.insert(id.clone(), Trashed { meta, deleted: now(), root: root.clone() });
        }
        self.commit_virtual_files(index, trash, moves)?;
        for (id, _) in items { self.forget_loaded(id); crate::versions::move_to_trash(&self.history_base(), id); }
        for (id, state) in before { self.record_file_change(&id, state); }
        Ok(())
    }

    pub(super) fn untrash_virtual(&mut self, id: &str, parent: Option<String>, place: Option<(u64, Option<u64>)>) -> Result<()> {
        self.validate_folder(Some(id), parent.as_deref())?;
        let mut index = self.index.clone(); let mut trash = self.trash.clone();
        let mut saved = trash.remove(id).context("恢复内容已不在回收站")?;
        saved.meta.parent = parent;
        if let Some((order, root)) = place { saved.meta.order = order; saved.meta.root_order = root; }
        let moves = if saved.meta.folder.is_none() { vec![(format!(".quiet/trash/{id}.{}", ext_of(&saved.meta)), saved.meta.rel.clone())] } else { vec![] };
        index.insert(id.to_string(), saved.meta);
        self.commit_virtual_files(index, trash, moves)?;
        crate::versions::restore_from_trash(&self.history_base(), id);
        self.record_file_change(id, None);
        Ok(())
    }

    fn commit_virtual_files(&mut self, index: HashMap<String, Meta>, trash: HashMap<String, Trashed>, moves: Vec<(String, String)>) -> Result<()> {
        if self.batch.index.get() { self.save_virtual()?; self.batch.index.set(false); }
        let before = Manifest::parse(&std::fs::read(self.root.join(FILE))?)?;
        let after = self.virtual_manifest_for(&index, &trash); after.validate()?;
        for (from, to) in &moves {
            let source = crate::git::safe_path(&self.root, from)?; let target = crate::git::safe_path(&self.root, to)?;
            anyhow::ensure!(source.is_file() && target.symlink_metadata().is_err(), "正文文件状态冲突，未覆盖：{to}");
        }
        let transaction = FileTransaction { before, after: after.clone(), moves };
        save_json(&self.root.join(TRANSACTION), &transaction)?;
        let result = (|| -> Result<()> {
            for (from, to) in &transaction.moves { std::fs::rename(self.abs(from), self.abs(to))?; }
            self.sync_virtual_moves(&transaction.moves)?;
            save_json(&self.root.join(FILE), &after)
        })();
        if result.is_err() {
            let committed = std::fs::read(self.root.join(FILE)).is_ok_and(|b| Manifest::parse(&b).is_ok_and(|m| m == after));
            if !committed { self.recover_virtual_files()?; return result; }
        }
        self.index = index; self.trash = trash; self.manifest = Some(after);
        let _ = std::fs::remove_file(self.root.join(TRANSACTION));
        self.save_local(&self.local_state())
    }

    fn sync_virtual_moves(&self, moves: &[(String, String)]) -> Result<()> {
        #[cfg(unix)]
        for dir in moves.iter().flat_map(|(from, to)| [parent_rel(from), parent_rel(to)]).collect::<HashSet<_>>() {
            std::fs::File::open(self.abs(dir))?.sync_all()?;
        }
        #[cfg(not(unix))] let _ = moves;
        Ok(())
    }

    fn recover_virtual_files(&self) -> Result<()> {
        let path = self.root.join(TRANSACTION);
        if !path.exists() { return Ok(()) }
        let transaction: FileTransaction = serde_json::from_slice(&std::fs::read(&path)?)?;
        transaction.before.validate()?; transaction.after.validate()?;
        let files = |manifest: &Manifest| -> HashMap<String, String> {
            manifest.notes.iter().map(|n| (format!("{NOTES}/{}", n.filename), n.id.clone()))
                .chain(manifest.trash.iter().filter_map(|t| t.filename.as_ref().map(|f| (format!(".quiet/trash/{f}"), t.id.clone())))).collect()
        };
        let before = files(&transaction.before); let after = files(&transaction.after);
        let mut sources = HashSet::new(); let mut targets = HashSet::new();
        for (from, to) in &transaction.moves {
            anyhow::ensure!(before.contains_key(from) && before.get(from) == after.get(to) && from != to
                && sources.insert(from) && targets.insert(to), "文件恢复记录中的正文路径无效");
        }
        let current = Manifest::parse(&std::fs::read(self.root.join(FILE))?)?;
        if current != transaction.after {
            anyhow::ensure!(current == transaction.before, "未完成的文件操作与当前清单冲突，已保留恢复记录");
            for (from, to) in transaction.moves.iter().rev() {
                let source = crate::git::safe_path(&self.root, from)?; let target = crate::git::safe_path(&self.root, to)?;
                if target.exists() {
                    anyhow::ensure!(!source.exists(), "恢复路径已存在，未覆盖：{from}"); std::fs::rename(target, source)?;
                }
            }
            self.sync_virtual_moves(&transaction.moves)?;
        }
        std::fs::remove_file(path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_trash_move_rolls_back_without_losing_the_document() {
        let root = std::env::temp_dir().join(format!("quiet-trash-recovery-{}", uuid::Uuid::new_v4()));
        let mut store = Store::open_at(root.clone()).unwrap();
        let id = store.create_from(&Document::default()).unwrap();
        let mut raw: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(FILE)).unwrap()).unwrap();
        raw["notes"][0].as_object_mut().unwrap().remove("createdAt");
        raw["notes"][0]["updatedAt"] = 1.into();
        let raw = serde_json::to_vec_pretty(&raw).unwrap(); std::fs::write(root.join(FILE), &raw).unwrap();
        let before = Manifest::parse(&raw).unwrap();
        let body = std::fs::read(store.path(&id)).unwrap();
        store.delete(&id, None).unwrap();
        let after = Manifest::parse(&std::fs::read(root.join(FILE)).unwrap()).unwrap();
        let transaction = FileTransaction { before: before.clone(), after, moves: vec![(format!("notes/{id}.xml"), format!(".quiet/trash/{id}.xml"))] };
        // Simulate interruption after moving the body but before publishing metadata.
        std::fs::write(root.join(FILE), &raw).unwrap(); save_json(&root.join(TRANSACTION), &transaction).unwrap();
        let reopened = Store::open_at(root.clone()).unwrap();
        assert_eq!(std::fs::read(reopened.path(&id)).unwrap(), body); assert!(!root.join(TRANSACTION).exists());
        assert!(reopened.trash.is_empty()); std::fs::remove_dir_all(root).unwrap();
    }
}

//! Explicit index operations; assets use the same LFS representation as full commits.
use super::*;

pub(crate) enum CommitScope { All, Staged, Auto }

impl LibraryRepo {
    pub(super) fn index_tree(&self) -> Result<git2::Tree<'_>> {
        let mut index = self.repo.index()?; index.read(true)?;
        ensure!(!index.has_conflicts(), "请先解决 Git 暂存区冲突");
        Ok(self.repo.find_tree(index.write_tree()?)?)
    }

    pub fn stage(&self, path: &str) -> Result<()> {
        safe_path(&self.root, path)?;
        let working = self.repo.find_tree(self.working_tree(true)?)?;
        let head = self.repo.head().ok().and_then(|h| h.peel_to_tree().ok());
        let mut index = self.repo.index()?; index.read(true)?;
        ensure!(!index.has_conflicts(), "请先解决 Git 暂存区冲突");
        if let Ok(entry) = working.get_path(Path::new(path)) {
            let blob = self.repo.find_blob(entry.id())?;
            index.add(&IndexEntry { ctime: IndexTime::new(0, 0), mtime: IndexTime::new(0, 0), dev: 0, ino: 0,
                mode: 0o100644, uid: 0, gid: 0, file_size: blob.size() as u32, id: entry.id(), flags: 0, flags_extended: 0, path: path.as_bytes().to_vec() })?;
        } else {
            ensure!(index.get_path(Path::new(path), 0).is_some() || head.as_ref().is_some_and(|t| t.get_path(Path::new(path)).is_ok()), "这个文件不在 Git 管理的文档库中");
            index.remove_path(Path::new(path))?;
        }
        index.write()?;
        Ok(())
    }

    pub fn unstage(&self, path: &str) -> Result<()> {
        safe_path(&self.root, path)?;
        self.repo.index()?.read(true)?;
        let head = self.repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        self.repo.reset_default(head.as_ref().map(|c| c.as_object()), [Path::new(path)])?;
        Ok(())
    }

    pub fn stage_all(&self) -> Result<()> {
        let tree = self.repo.find_tree(self.working_tree(true)?)?;
        let mut index = self.repo.index()?; index.read(true)?;
        ensure!(!index.has_conflicts(), "请先解决 Git 暂存区冲突");
        index.read_tree(&tree)?; index.write()?;
        Ok(())
    }

    pub fn unstage_all(&self) -> Result<()> {
        let head = self.repo.head().ok().and_then(|h| h.peel_to_tree().ok());
        let mut index = self.repo.index()?; index.read(true)?;
        ensure!(!index.has_conflicts(), "请先解决 Git 暂存区冲突");
        if let Some(tree) = head { index.read_tree(&tree)?; } else { index.clear()?; }
        index.write()?;
        Ok(())
    }

    pub fn commit(&self, message: &str) -> Result<Option<String>> {
        self.commit_changes(message, CommitScope::All, &mut BTreeSet::new())
    }

    pub fn commit_staged(&self, message: &str) -> Result<Option<String>> {
        self.commit_changes(message, CommitScope::Staged, &mut BTreeSet::new())
    }

    pub(crate) fn commit_changes(&self, message: &str, scope: CommitScope, restored: &mut BTreeSet<String>) -> Result<Option<String>> {
        ensure!(!message.trim().is_empty(), "请填写提交说明");
        let config = self.config()?;
        ensure!(!config.name.trim().is_empty() && !config.email.trim().is_empty(), "请先配置 Git 作者名称和邮箱");
        let author = Signature::now(&config.name, &config.email)?;
        let parent = self.repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let before = parent.as_ref().map(|p| p.tree()).transpose()?;
        let working = self.repo.find_tree(self.working_tree(false)?)?;
        let indexed = self.index_tree()?;
        let changes = self.changes_between(before.as_ref(), &indexed, &working)?;
        let staged = match scope {
            CommitScope::All => false,
            CommitScope::Staged => true,
            CommitScope::Auto => changes.iter().any(|c| c.staged && !c.blank_only),
        };
        let source = if staged { &indexed } else {
            ensure!(self.working_tree(true)? == working.id(), "提交期间文档库内容发生变化，请重试");
            &working
        };
        let mut snapshot = Index::new()?; snapshot.read_tree(source)?;
        let local: Vec<_> = snapshot.iter().filter_map(|entry| {
            let path = std::str::from_utf8(&entry.path).ok()?;
            is_internal_path(path).then(|| PathBuf::from(path))
        }).collect();
        for path in local { snapshot.remove_path(&path)?; }
        let mut restore = Vec::new();
        if let Some(before) = &before {
            for change in changes.iter().filter(|c| !is_internal_path(&c.path)) {
                let xml = change.path.split_once('/').is_some_and(|(dir, file)| dir == NOTES && file.ends_with(".xml"));
                if !change.blank_only && (!xml || (staged && !change.staged)) { continue; }
                let path = Path::new(&change.path);
                let (Ok(original), Some(mut entry)) = (before.get_path(path), snapshot.get_path(path, 0)) else { continue };
                if original.filemode() != 0o100644 || entry.mode != 0o100644 { continue; }
                let blob = self.repo.find_blob(original.id())?;
                if !change.blank_only && (original.id() == entry.id
                    || !crate::diff::same_xml_document(blob.content(), self.repo.find_blob(entry.id)?.content())) { continue; }
                let snapshot_id = entry.id;
                entry.id = original.id(); entry.file_size = blob.size() as u32;
                snapshot.add(&entry)?;
                // A later edit may have changed the worktree since this snapshot was staged.
                if let Ok(current) = working.get_path(path) && current.id() != original.id() && current.filemode() == 0o100644
                    && (change.blank_only || current.id() == snapshot_id
                        || crate::diff::same_xml_document(blob.content(), self.repo.find_blob(current.id())?.content())) {
                    restore.push((&change.path, current.id(), blob));
                }
            }
        }
        let tree = self.repo.find_tree(snapshot.write_tree_to(&self.repo)?)?;
        let changed = !parent.as_ref().is_some_and(|p| p.tree_id() == tree.id());
        if staged && changed {
            validate_tree(&self.repo, &tree_files(&self.repo, &tree)?)
                .context("暂存区中的文档库不完整，请同时暂存相关的 quiet.json 和正文")?;
            if before.as_ref().is_none_or(|t| self.tree_manifest(t).is_err()) {
                let attr = tree.get_path(Path::new(".gitattributes")).context("请同时暂存 Git LFS 规则")?;
                let attr = self.repo.find_blob(attr.id())?;
                let rules = std::str::from_utf8(attr.content())?;
                ensure!(asset_attributes().all(|rule| rules.lines().any(|line| line == rule)), "请同时暂存 Git LFS 规则");
            }
            for pointer in self.pointers(None, &tree)? { lfs::Lfs::new(self.repo.path()).read(&pointer)?; }
        }
        for (path, expected, original) in restore {
            let target = safe_path(&self.root, path)?;
            let expected = self.repo.find_blob(expected)?;
            crate::store::write_atomic_after(&target, original.content(), || {
                let current = crate::store::read_bounded(&safe_path(&self.root, path)?, crate::store::EXPORT_FILE_LIMIT)?;
                ensure!(current == expected.content(), "提交期间文档库内容发生变化，请重试");
                restored.insert(path.clone());
                Ok(())
            })?;
        }
        let parents: Vec<_> = parent.iter().collect();
        let commit = if changed { Some(self.repo.commit(Some("HEAD"), &author, &author, message.trim(), &tree, &parents)?.to_string()) } else { None };
        let update_index = staged || changed || changes.iter().filter(|c| c.staged).all(|c| c.blank_only);
        if update_index && tree.id() != indexed.id() {
            let mut index = self.repo.index()?; index.read_tree(&tree)?; index.write()?;
        }
        Ok(commit)
    }

    /// Like checkout -- <path>: restore the index snapshot, leaving staged changes intact.
    pub fn discard(&self, path: &str) -> Result<()> {
        ensure!(!is_internal_path(path), "文档库本机文件不通过 Git 恢复");
        let target = safe_path(&self.root, path)?;
        let status = self.status()?;
        ensure!(status.changes.iter().any(|c| c.path == path && c.unstaged), "这个文件没有未暂存的修改");
        let index = self.index_tree()?;
        if let Ok(entry) = index.get_path(Path::new(path)) {
            ensure!(entry.filemode() == 0o100644, "Git 文档库不允许符号链接或可执行文件");
            let blob = self.repo.find_blob(entry.id())?;
            let bytes = if is_asset_path(path) {
                lfs::Lfs::new(self.repo.path()).read(&Pointer::parse(blob.content())?.context("图片缺少 LFS 指针")?)?
            } else { blob.content().to_vec() };
            if path == MANIFEST {
                let restored = manifest(&bytes)?;
                let current = manifest(&std::fs::read(&target)?)?;
                for file in current.keys().filter(|p| !restored.contains_key(*p)) {
                    ensure!(!safe_path(&self.root, file)?.exists(), "请先丢弃新增文章，再恢复清单：{file}");
                }
                for file in restored.keys() {
                    ensure!(safe_path(&self.root, file)?.is_file(), "请先恢复清单引用的正文：{file}");
                }
            }
            std::fs::create_dir_all(target.parent().unwrap())?;
            crate::store::write_atomic(&target, &bytes)?;
        } else {
            ensure!(!matches!(path, MANIFEST | ".gitattributes" | ".gitignore"), "文档库必需文件不能丢弃");
            // New notes must also leave the manifest, otherwise their missing body breaks the library.
            let manifest_path = safe_path(&self.root, MANIFEST)?;
            let bytes = std::fs::read(&manifest_path)?;
            let mut value: serde_json::Value = serde_json::from_slice(&bytes)?;
            for key in ["notes", "trash"] {
                if let Some(entries) = value[key].as_array_mut() {
                    entries.retain(|e| e["filename"].as_str().is_none_or(|f| format!("{NOTES}/{f}") != path));
                }
            }
            let updated = serde_json::to_vec_pretty(&value)?;
            manifest(&updated)?;
            if target.exists() {
                let original = std::fs::read(&target)?;
                std::fs::remove_file(&target)?;
                if value != serde_json::from_slice::<serde_json::Value>(&bytes)? && let Err(error) = crate::store::write_atomic(&manifest_path, &updated) {
                    crate::store::write_atomic(&target, &original)?;
                    return Err(error);
                }
            }
        }
        Ok(())
    }
}

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
        let mut restore = Vec::new();
        if let Some(before) = &before {
            for change in changes.iter().filter(|c| c.blank_only) {
                let path = Path::new(&change.path);
                let original = before.get_path(path)?;
                let current = working.get_path(path)?;
                let mut entry = snapshot.get_path(path, 0).context("Git 快照缺少原有文件")?;
                let blob = self.repo.find_blob(original.id())?;
                entry.id = original.id(); entry.file_size = blob.size() as u32;
                snapshot.add(&entry)?;
                if current.id() != original.id() { restore.push((&change.path, current.id(), blob)); }
            }
        }
        let tree = self.repo.find_tree(snapshot.write_tree_to(&self.repo)?)?;
        let changed = !parent.as_ref().is_some_and(|p| p.tree_id() == tree.id());
        if staged && changed {
            let format = validate_tree(&self.repo, &tree_files(&self.repo, &tree)?)
                .context("暂存区中的文档库不完整，请同时暂存相关的 quiet.json、正文和格式标记")?;
            ensure!(format >= self.tree_format(&working)?, "请先暂存文档库格式标记，再提交");
            let previous = before.as_ref().map(|t| self.tree_format(t)).transpose()?.unwrap_or_default();
            if (format >= OLD_NOTES_FORMAT && previous < OLD_NOTES_FORMAT)
                || (format >= PLURAL_RESOURCES_FORMAT && previous < PLURAL_RESOURCES_FORMAT) {
                let attr = tree.get_path(Path::new(".gitattributes")).context("请同时暂存 Git LFS 规则")?;
                let attr = self.repo.find_blob(attr.id())?;
                let rules = std::str::from_utf8(attr.content())?;
                ensure!(asset_attributes(format).all(|rule| rules.lines().any(|line| line == rule)), "请同时暂存 Git LFS 规则");
            }
            for pointer in self.pointers(None, &tree)? { lfs::Lfs::new(self.repo.path()).read(&pointer)?; }
        }
        for (path, expected, original) in restore {
            let target = safe_path(&self.root, path)?;
            let current = crate::store::read_bounded(&target, crate::store::EXPORT_FILE_LIMIT)?;
            ensure!(current == self.repo.find_blob(expected)?.content(), "提交期间文档库内容发生变化，请重试");
            restored.insert(path.clone());
            crate::store::write_atomic(&target, original.content())?;
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
        let target = safe_path(&self.root, path)?;
        let status = self.status()?;
        ensure!(status.changes.iter().any(|c| c.path == path && c.unstaged), "这个文件没有未暂存的修改");
        let index = self.index_tree()?;
        let worktree_format = self.worktree_format();
        let dir = notes_dir(worktree_format);
        if let Some((prefix, file)) = path.split_once('/') {
            let renamed = if worktree_format >= NOTES_FORMAT && [OLD_NOTES, NOTES].contains(&prefix) { Some((OLD_NOTES, NOTES)) }
                else if worktree_format >= PLURAL_RESOURCES_FORMAT {
                    crate::store::RESOURCE_RENAMES.into_iter().find(|(old, new)| [*old, *new].contains(&prefix))
                } else { None };
            if let Some((old, new)) = renamed && index.get_path(Path::new(&format!("{old}/{file}"))).is_ok() {
                ensure!(prefix == new && index.get_path(Path::new(path)).is_ok(), "不能丢弃文档库目录升级");
            }
        }
        if let Ok(entry) = index.get_path(Path::new(path)) {
            ensure!(entry.filemode() == 0o100644, "Git 文档库不允许符号链接或可执行文件");
            let blob = self.repo.find_blob(entry.id())?;
            let bytes = if is_asset_path_at(path, self.tree_format(&index).unwrap_or_default()) {
                lfs::Lfs::new(self.repo.path()).read(&Pointer::parse(blob.content())?.context("图片缺少 LFS 指针")?)?
            } else { blob.content().to_vec() };
            if path == FORMAT_PATH {
                ensure!(parse_format(&bytes)? >= Store::format_at(&self.root).unwrap_or_default(), "不能丢弃文档库格式升级");
            }
            if RULE_FILES.contains(&path) && worktree_format >= PLURAL_RESOURCES_FORMAT && self.tree_format(&index)? < PLURAL_RESOURCES_FORMAT {
                let text = std::str::from_utf8(&bytes)?;
                ensure!(with_rules(path, text.to_owned(), worktree_format) == text, "不能丢弃文档库格式升级");
            }
            if path == MANIFEST {
                let restored = manifest(&bytes, worktree_format)?;
                let current = manifest(&std::fs::read(&target)?, worktree_format)?;
                for file in current.keys().filter(|p| p.split_once('/').is_some_and(|(prefix, _)| prefix == dir) && !restored.contains_key(*p)) {
                    ensure!(!safe_path(&self.root, file)?.exists(), "请先丢弃新增文章，再恢复清单：{file}");
                }
                for file in restored.keys().filter(|p| !p.ends_with('/')) {
                    ensure!(safe_path(&self.root, file)?.is_file(), "请先恢复清单引用的正文：{file}");
                }
            }
            std::fs::create_dir_all(target.parent().unwrap())?;
            crate::store::write_atomic(&target, &bytes)?;
        } else {
            ensure!(!matches!(path, MANIFEST | ".quiet/format.json" | ".gitattributes" | ".gitignore"), "文档库必需文件不能丢弃");
            // New notes must also leave the manifest, otherwise their missing body breaks the library.
            let manifest_path = safe_path(&self.root, MANIFEST)?;
            let bytes = std::fs::read(&manifest_path)?;
            let mut value: serde_json::Value = serde_json::from_slice(&bytes)?;
            if value.get("version").is_some() {
                for key in ["notes", "trash"] {
                    if let Some(entries) = value[key].as_array_mut() {
                        entries.retain(|e| e["filename"].as_str().is_none_or(|f| format!("{dir}/{f}") != path));
                    }
                }
            } else if let Some(entries) = value.as_object_mut() { entries.remove(path); }
            let updated = serde_json::to_vec_pretty(&value)?;
            manifest(&updated, worktree_format)?;
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

//! Embedded Git for libraries. No Git executable, hooks or credential helpers.
use anyhow::{Context, Result, ensure};
use git2::{Index, IndexEntry, IndexTime, Oid, Repository, Signature};
use serde::{Deserialize, Serialize};
use std::{collections::{BTreeMap, BTreeSet}, path::{Component, Path, PathBuf}};
use crate::manifest::{Manifest, NOTES};
use crate::store::{DRAWING, INTERNAL, RESOURCE_DIRS, Store};

mod lfs;
mod transport;
mod changes;
mod settings;
mod staging;
pub use changes::ChangePreview;
pub use lfs::Pointer;
pub(crate) use staging::CommitScope;
#[cfg(test)] mod tests;

pub const MANIFEST: &str = crate::manifest::FILE;
const RULE_FILES: [&str; 2] = [".gitattributes", ".gitignore"];

fn asset_attributes() -> impl Iterator<Item = String> {
    RESOURCE_DIRS.into_iter().map(|dir| format!("/{dir}/** filter=lfs diff=lfs merge=lfs -text"))
}

fn is_asset_path(path: &str) -> bool {
    path.split_once('/').is_some_and(|(dir, _)| RESOURCE_DIRS.contains(&dir))
}

fn is_xml_note(path: &str) -> bool {
    path.split_once('/').is_some_and(|(dir, file)| dir == NOTES && file.ends_with(".xml"))
}

fn with_rules(name: &str, mut text: String) -> String {
    let additions = if name == ".gitattributes" { asset_attributes().collect::<Vec<_>>().join("\n") }
        else { ".DS_Store\n.quiet/\n/Images/remote/*.part\n*.tmp\nquiet.json.bak\n".into() };
    for line in additions.lines() {
        if text.lines().any(|existing| existing == line) { continue }
        if !text.is_empty() && !text.ends_with('\n') { text.push('\n'); }
        text.push_str(line); text.push('\n');
    }
    text
}

fn write_rules(root: &Path) -> Result<()> {
    for name in RULE_FILES {
        let path = safe_path(root, name)?;
        let text = if path.exists() { std::fs::read_to_string(&path)? } else { String::new() };
        let updated = with_rules(name, text.clone());
        if updated != text { crate::store::write_atomic(&path, updated.as_bytes())?; }
    }
    Ok(())
}

/// Drop the former Git opt-in for internal metadata from existing rule files.
pub(crate) fn update_internal_ignore(root: &Path) -> Result<()> {
    let path = safe_path(root, ".gitignore")?;
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    if let Some(updated) = updated_internal_ignore(&text) { crate::store::write_atomic(&path, updated.as_bytes())?; }
    Ok(())
}

fn updated_internal_ignore(text: &str) -> Option<String> {
    let mut updated = String::new();
    for line in text.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) != "!.quiet/format.json" { updated.push_str(line); }
    }
    (updated.len() != text.len()).then(|| with_rules(".gitignore", updated))
}

pub fn is_internal_path(path: &str) -> bool {
    path.split('/').next().is_some_and(|dir| dir.eq_ignore_ascii_case(INTERNAL))
}

/// Passwords stay in memory; repository configuration contains no access token.
#[derive(Clone, Default)]
pub struct Auth { pub username: String, pub token: String, pub ssh_passphrase: String }
impl Auth {
    pub fn environment() -> Self { Self {
        username: std::env::var("QUIET_MD_GIT_USERNAME").unwrap_or_else(|_| "git".into()),
        token: std::env::var("QUIET_MD_GIT_TOKEN").unwrap_or_default(),
        ssh_passphrase: std::env::var("QUIET_MD_GIT_SSH_PASSPHRASE").unwrap_or_default(),
    } }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub name: String, pub email: String, pub remote: String,
    #[serde(skip)] pub branch: String,
    pub lfs_url: String, pub ssh_key: String, pub lfs_ca_info: String,
}
#[derive(Clone, Debug)]
pub struct Change { pub path: String, pub kind: char, pub staged: bool, pub unstaged: bool, pub auto_restore: bool }
#[derive(Clone, Debug, Default)]
pub struct Status { pub branch: String, pub remote: String, pub head: Option<String>, pub changes: Vec<Change> }
impl Status {
    pub fn visible_changes(&self) -> impl Iterator<Item = &Change> { self.changes.iter().filter(|c| !c.auto_restore) }
}
#[derive(Clone, Debug)]
pub struct Commit { pub id: String, pub message: String, pub author: String, pub time: i64 }

pub struct LibraryRepo { root: PathBuf, repo: Repository }

impl LibraryRepo {
    pub fn check_location(root: &Path) -> Result<()> {
        let root = root.canonicalize()?;
        if let Ok(repo) = Repository::discover(&root) {
            ensure!(repo.workdir().and_then(|p| p.canonicalize().ok()).as_ref() == Some(&root), "文档库不能位于另一个 Git 仓库中");
            ensure!(repo.head().is_err() || repo.head().ok().and_then(|h| h.peel_to_tree().ok()).is_some_and(|t| t.get_name(MANIFEST).is_some()), "已有 Git 仓库没有 quiet.json，请选择独立文档库");
        }
        if let Ok(meta) = root.join(".git").symlink_metadata() { ensure!(meta.is_dir() && !meta.file_type().is_symlink(), "文档库需要独立的 .git 目录"); }
        Ok(())
    }

    pub fn init(root: &Path) -> Result<Self> {
        let root = root.canonicalize()?;
        Self::check_location(&root)?;
        let existing = root.join(".git").is_dir().then(|| Repository::open(&root)).transpose()?;
        update_internal_ignore(&root)?;
        write_rules(&root)?;
        let repo = if let Some(repo) = existing { repo } else {
            let mut options = git2::RepositoryInitOptions::new();
            options.initial_head("main").no_reinit(true).external_template(false);
            Repository::init_opts(&root, &options)?
        };
        Ok(Self { root, repo })
    }

    pub fn clone_into(remote: &str, root: &Path, auth: &Auth) -> Result<Self> {
        Self::clone_configured(&Config { remote: remote.to_string(), ..Default::default() }, root, auth)
    }

    pub fn clone_configured(config: &Config, root: &Path, auth: &Auth) -> Result<Self> {
        validate_remote(&config.remote)?;
        std::fs::create_dir_all(root)?;
        ensure!(std::fs::read_dir(root)?.next().is_none(), "克隆目标必须是空文件夹");
        Store::prepare_layout(root)?;
        let repo = Self::init(root)?;
        repo.configure(config)?;
        {
            let mut origin = repo.remote(config)?;
            let connection = origin.connect_auth(git2::Direction::Fetch, Some(callbacks(auth, config)), None)?;
            if let Ok(branch) = connection.default_branch() && let Ok(branch) = branch.as_str() { repo.repo.set_head(branch)?; }
        }
        repo.pull(auth)?;
        Ok(repo)
    }

    pub fn open(root: &Path) -> Result<Self> {
        Self::check_location(root)?;
        let repo = Repository::open(root)?;
        let root = root.canonicalize()?;
        ensure!(repo.workdir().and_then(|p| p.canonicalize().ok()).as_ref() == Some(&root), "Git 仓库根目录与文档库不一致");
        Ok(Self { root, repo })
    }

    fn files(&self) -> Result<BTreeSet<String>> {
        let bytes = crate::store::read_bounded(&safe_path(&self.root, MANIFEST)?, crate::manifest::MAX_BYTES as u64)?;
        let entries = manifest(&bytes)?;
        let mut files: BTreeSet<_> = [MANIFEST, ".gitattributes", ".gitignore"].into_iter().map(str::to_string).collect();
        files.extend(entries.into_keys());
        for dir in RESOURCE_DIRS { collect(&self.root, dir, &mut files)?; }
        Ok(files)
    }

    fn working_tree(&self, cache: bool) -> Result<Oid> {
        let mut index = Index::new()?;
        let lfs = lfs::Lfs::new(self.repo.path());
        for path in self.files()? {
            let file = safe_path(&self.root, &path)?;
            if !file.exists() { continue }
            let bytes = crate::store::read_bounded(&file, crate::store::EXPORT_FILE_LIMIT)?;
            let content = if is_asset_path(&path) {
                let pointer = if let Some(p) = Pointer::parse(&bytes)? {
                    lfs.read(&p).context("工作区存在未下载的 LFS 指针，未提交")?; p
                } else if cache { lfs.store(&bytes)? } else { Pointer::of(&bytes) };
                pointer.text().into_bytes()
            } else { bytes };
            let id = self.repo.blob(&content)?;
            index.add(&IndexEntry { ctime: IndexTime::new(0, 0), mtime: IndexTime::new(0, 0), dev: 0, ino: 0,
                mode: 0o100644, uid: 0, gid: 0, file_size: content.len() as u32, id, flags: 0, flags_extended: 0, path: path.into_bytes() })?;
        }
        Ok(index.write_tree_to(&self.repo)?)
    }

    pub fn status(&self) -> Result<Status> {
        let config = self.config()?;
        let head = self.repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let before = head.as_ref().map(|c| c.tree()).transpose()?;
        let after = self.repo.find_tree(self.working_tree(false)?)?;
        let index = self.index_tree()?;
        let changes = self.changes_between(before.as_ref(), &index, &after)?;
        Ok(Status { branch: config.branch, remote: config.remote, head: head.map(|c| c.id().to_string()), changes })
    }

    fn changes_between(&self, before: Option<&git2::Tree<'_>>, index: &git2::Tree<'_>, after: &git2::Tree<'_>) -> Result<Vec<Change>> {
        let mut changes = BTreeMap::new();
        for (staged, from, to) in [(true, before, index), (false, Some(index), after)] {
            for delta in self.repo.diff_tree_to_tree(from, Some(to), None)?.deltas() {
                let path = delta.new_file().path().or_else(|| delta.old_file().path()).context("Git 变更缺少路径")?.to_string_lossy().into_owned();
                let blank_only = !is_asset_path(&path) && !is_xml_note(&path) && self.blank_line_delta(&path, &delta)?;
                let change = changes.entry(path.clone()).or_insert(Change { path, kind: 'M', staged: false, unstaged: false, auto_restore: true });
                change.auto_restore &= blank_only;
                change.kind = match delta.status() { git2::Delta::Added => 'A', git2::Delta::Deleted => 'D', _ => 'M' };
                if staged { change.staged = true; } else { change.unstaged = true; }
            }
        }
        if let Some(before) = before {
            for change in changes.values_mut().filter(|c| !c.auto_restore && is_xml_note(&c.path)) {
                let path = Path::new(&change.path);
                let (Ok(original), Ok(staged), Ok(current)) = (before.get_path(path), index.get_path(path), after.get_path(path)) else { continue };
                if [&original, &staged, &current].iter().any(|entry| entry.filemode() != 0o100644) { continue; }
                let original = self.repo.find_blob(original.id())?;
                // Keep a partially staged file visible if either version differs from HEAD.
                change.auto_restore = self.same_xml_blob(&original, staged.id())?
                    && (staged.id() == current.id() || self.same_xml_blob(&original, current.id())?);
            }
        }
        Ok(changes.into_values().collect())
    }

    fn same_xml_blob(&self, original: &git2::Blob<'_>, id: Oid) -> Result<bool> {
        Ok(original.id() == id || crate::diff::same_xml_document(original.content(), self.repo.find_blob(id)?.content()))
    }

    fn blank_line_delta(&self, path: &str, delta: &git2::DiffDelta<'_>) -> Result<bool> {
        let (old, new) = (delta.old_file(), delta.new_file());
        if delta.status() != git2::Delta::Modified
            || !matches!((old.mode(), new.mode()), (git2::FileMode::Blob, git2::FileMode::Blob)) { return Ok(false) }
        let old = self.repo.find_blob(old.id())?; let new = self.repo.find_blob(new.id())?;
        let (Ok(old), Ok(new)) = (std::str::from_utf8(old.content()), std::str::from_utf8(new.content())) else { return Ok(false) };
        if old.contains('\0') || new.contains('\0') { return Ok(false) }
        if path.ends_with(".md") {
            // Interior blank lines can change Markdown structure or literal code.
            fn skip_leading_blank_lines(text: &str) -> &str {
                let prefix: usize = text.split_inclusive('\n')
                    .take_while(|line| line.trim_end_matches(['\r', '\n']).bytes().all(|b| matches!(b, b' ' | b'\t')))
                    .map(str::len).sum();
                &text[prefix..]
            }
            let has_bom = |s: &str| s.starts_with('\u{feff}');
            return Ok(skip_leading_blank_lines(old) == skip_leading_blank_lines(new)
                && has_bom(old) == has_bom(new)
                && crate::markdown::front_matter(old).0 == crate::markdown::front_matter(new).0);
        }
        // Keep the exact bytes of nonblank lines, including their line endings.
        fn nonblank(text: &str) -> impl Iterator<Item = &str> {
            text.split_inclusive('\n').filter(|line| !crate::diff::is_blank_line(line))
        }
        Ok(nonblank(old).eq(nonblank(new)))
    }

    pub fn history(&self, document: Option<&str>, limit: usize) -> Result<Vec<Commit>> {
        let mut walk = self.repo.revwalk()?;
        if walk.push_head().is_err() { return Ok(vec![]) }
        walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;
        let mut out = vec![];
        for id in walk {
            let commit = self.repo.find_commit(id?)?;
            let tree = commit.tree()?;
            let Ok(entries) = self.tree_manifest(&tree) else { continue };
            if let Some(doc) = document {
                let current = self.document_key(&tree, &entries, doc)?;
                let previous = if let Ok(parent) = commit.parent(0) {
                    let tree = parent.tree()?;
                    match self.tree_manifest(&tree) {
                        Ok(entries) => self.document_key(&tree, &entries, doc)?,
                        Err(_) => None,
                    }
                } else { None };
                if current == previous { continue }
            }
            out.push(Commit { id: commit.id().to_string(), message: commit.summary().ok().flatten().unwrap_or("").to_string(), author: commit.author().name().unwrap_or("").to_string(), time: commit.time().seconds() });
            if out.len() >= limit.min(200) { break }
        }
        Ok(out)
    }

    fn document_key(&self, tree: &git2::Tree<'_>, entries: &BTreeMap<String, Metadata>, document: &str) -> Result<Option<DocumentKey>> {
        let Some((path, meta)) = entries.iter().find(|(_, meta)| meta.id == document) else { return Ok(None) };
        let Ok(entry) = tree.get_path(Path::new(path)) else { return Ok(None) };
        let mut drawings = BTreeMap::new();
        if path.ends_with(".xml") {
            let blob = self.repo.find_blob(entry.id())?;
            if let Ok(xml) = std::str::from_utf8(blob.content()) && xml.contains("<drawing")
                && let Ok(doc) = crate::docxml::parse(xml) {
                doc.for_each_block(|block| {
                    if let crate::model::BlockKind::Drawing(drawing) = &block.kind
                        && drawing.source.split_once('/').is_some_and(|(dir, _)| dir == DRAWING) {
                        let oid = tree.get_path(Path::new(&drawing.source)).ok().map(|asset| asset.id());
                        drawings.insert(drawing.source.clone(), oid);
                    }
                });
            }
        }
        Ok(Some(DocumentKey { body: entry.id(), path: path.clone(), metadata: serde_json::to_string(meta)?, drawings }))
    }

    pub fn revision(&self, commit: &str, document: &str) -> Result<String> {
        let commit = self.repo.find_commit(Oid::from_str(commit)?)?;
        let tree = commit.tree()?;
        let entries = self.tree_manifest(&tree)?;
        let (path, _) = entries.iter().find(|(_, meta)| meta.id == document).context("这个提交中没有此文章")?;
        let oid = tree.get_path(Path::new(path))?.id();
        Ok(std::str::from_utf8(self.repo.find_blob(oid)?.content())?.to_string())
    }

    fn pointers(&self, history: Option<Oid>, tree: &git2::Tree<'_>) -> Result<Vec<Pointer>> {
        let mut pointers = BTreeMap::new();
        let mut trees = vec![tree.id()];
        if let Some(commit) = history {
            let mut walk = self.repo.revwalk()?; walk.push(commit)?;
            for id in walk { trees.push(self.repo.find_commit(id?)?.tree_id()); }
        }
        for tree in trees {
            let tree = self.repo.find_tree(tree)?;
            if self.tree_manifest(&tree).is_err() { continue }
            for (path, oid) in tree_files(&self.repo, &tree)? {
                if !is_asset_path(&path) { continue }
                let blob = self.repo.find_blob(oid)?;
                let p = Pointer::parse(blob.content())?.context("仓库中的图片未使用 LFS，已停止同步")?;
                pointers.insert(p.oid.clone(), p);
            }
        }
        Ok(pointers.into_values().collect())
    }

    pub fn push(&self, auth: &Auth) -> Result<()> {
        let config = self.config()?;
        validate_remote(&config.remote)?;
        let commit = self.repo.head()?.peel_to_commit()?;
        let head = commit.tree()?;
        validate_tree(&self.repo, &tree_files(&self.repo, &head)?)?;
        let pointers = self.pointers(Some(commit.id()), &head)?;
        lfs::Lfs::new(self.repo.path()).transfer(&config, &self.repo.config()?, &pointers, true, auth)?;
        let mut remote = self.remote(&config)?;
        let mut options = git2::PushOptions::new(); options.remote_callbacks(callbacks(auth, &config));
        remote.push(&[&format!("{}:refs/heads/{}", commit.id(), config.branch)], Some(&mut options))?;
        Ok(())
    }

    /// Fast-forward only. Fetch/LFS failures never change current files or HEAD.
    pub fn pull(&self, auth: &Auth) -> Result<bool> {
        ensure!(self.ready_to_pull()?, "请先提交本地变更，再拉取远端版本");
        let config = self.config()?; validate_remote(&config.remote)?;
        let mut remote = self.remote(&config)?;
        let mut options = git2::FetchOptions::new(); options.remote_callbacks(callbacks(auth, &config));
        remote.fetch(&[&format!("refs/heads/{}:refs/remotes/origin/{}", config.branch, config.branch)], Some(&mut options), None)?;
        let target = self.repo.find_reference(&format!("refs/remotes/origin/{}", config.branch))?.peel_to_commit()?;
        let head = self.repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        if head.as_ref().is_some_and(|h| h.id() == target.id()) { return Ok(false) }
        if let Some(head) = &head {
            if self.repo.graph_descendant_of(head.id(), target.id())? { return Ok(false) }
            ensure!(self.repo.graph_descendant_of(target.id(), head.id())?, "本地与远端历史已分叉，未覆盖任何文章");
        }
        let tree = target.tree()?;
        let files = tree_files(&self.repo, &tree)?;
        validate_tree(&self.repo, &files)?;
        let pointers = self.pointers(Some(target.id()), &tree)?;
        let lfs = lfs::Lfs::new(self.repo.path());
        lfs.transfer(&config, &self.repo.config()?, &pointers, false, auth)?;
        let mut content = BTreeMap::new();
        for (path, oid) in files {
            let blob = self.repo.find_blob(oid)?;
            let bytes = if is_asset_path(&path) { lfs.read(&Pointer::parse(blob.content())?.context("图片缺少 LFS 指针")?)? } else { blob.content().to_vec() };
            content.insert(path, bytes);
        }
        // Re-check after network I/O; callers also hold the Store write lock.
        ensure!(self.ready_to_pull()?, "拉取期间本地内容发生变化，未覆盖文章");
        self.apply(&content, &tree, target.id(), &config.branch, head.as_ref().map_or(Oid::ZERO_SHA1, |h| h.id()))?;
        Ok(true)
    }

    fn ready_to_pull(&self) -> Result<bool> {
        let status = self.status()?;
        if status.changes.is_empty() { return Ok(true) }
        if let Some(head) = status.head {
            // Internal files are excluded from the snapshot. Allow a pull when
            // the only other difference is the app's update to its ignore rule.
            let mut changes = status.changes.iter().filter(|c| !is_internal_path(&c.path));
            let Some(change) = changes.next() else { return Ok(true) };
            if change.path != ".gitignore" || change.staged || changes.next().is_some() { return Ok(false) }
            let tree = self.repo.find_commit(Oid::from_str(&head)?)?.tree()?;
            let Some(entry) = tree.get_name(".gitignore") else { return Ok(false) };
            let content = self.repo.find_blob(entry.id())?;
            let Some(updated) = updated_internal_ignore(std::str::from_utf8(content.content())?) else { return Ok(false) };
            let rules = std::fs::read(safe_path(&self.root, ".gitignore")?)?;
            return Ok(rules == updated.as_bytes());
        }
        // A fresh empty clone has only bootstrap metadata, not an initial commit.
        let shared = Manifest::parse(&std::fs::read(self.root.join(MANIFEST))?)?;
        let empty = shared.folders.is_empty() && shared.notes.is_empty() && shared.trash.is_empty();
        Ok(empty
            && self.files()?.iter().all(|p| matches!(p.as_str(), MANIFEST | ".gitattributes" | ".gitignore")))
    }

    pub fn revision_paths(&self, commit: &str) -> Result<BTreeMap<String, String>> {
        let tree = self.repo.find_commit(Oid::from_str(commit)?)?.tree()?;
        Ok(self.tree_manifest(&tree)?.into_iter().map(|(p, m)| (m.id, p)).collect())
    }

    pub fn revision_title(&self, commit: &str, document: &str) -> Result<Option<String>> {
        let tree = self.repo.find_commit(Oid::from_str(commit)?)?.tree()?;
        Ok(self.tree_manifest(&tree)?.values().find(|m| m.id == document).and_then(|m| m.extra.get("title")).and_then(|v| v.as_str()).map(str::to_string))
    }

    fn tree_manifest(&self, tree: &git2::Tree<'_>) -> Result<BTreeMap<String, Metadata>> {
        let entry = tree.get_name(MANIFEST).context("提交中缺少 quiet.json")?;
        manifest(self.repo.find_blob(entry.id())?.content())
    }

    fn revision_asset_pointer(&self, commit: &str, source: &str) -> Result<Option<(String, Pointer)>> {
        let tree = self.repo.find_commit(Oid::from_str(commit)?)?.tree()?;
        let source = source.strip_prefix("file://").unwrap_or(source).replace('\\', "/");
        self.tree_manifest(&tree)?;
        let managed = |path: &str| is_asset_path(path).then(|| path.to_string());
        let absolute = source.starts_with('/') || source.as_bytes().get(1) == Some(&b':');
        let relative = if absolute {
            let root = self.root.to_string_lossy().replace('\\', "/");
            source.strip_prefix(&root).and_then(|rest| rest.strip_prefix('/')).and_then(managed)
                .or_else(|| source.match_indices('/').filter_map(|(i, _)| managed(&source[i + 1..]))
                    .find(|path| tree.get_path(Path::new(path)).is_ok()))
        } else { managed(&source) };
        let Some(path) = relative else { return Ok(None) };
        let entry = tree.get_path(Path::new(&path))?;
        ensure!(entry.filemode() == 0o100644, "历史资源不是普通文件");
        let blob = self.repo.find_blob(entry.id())?;
        let pointer = Pointer::parse(blob.content())?.context("历史资源未使用 LFS")?;
        Ok(Some((path, pointer)))
    }

    pub(super) fn revision_drawing_asset(&self, commit: &str, source: &str) -> Result<Option<String>> {
        Ok(self.revision_asset_pointer(commit, source)?.map(|(_, p)| format!("git-lfs:{}:{}", p.oid, p.size)))
    }

    pub(super) fn read_drawing_asset(&self, oid: &str, size: &str, limit: u64) -> Result<Vec<u8>> {
        let pointer = Pointer::from_parts(oid, size.parse().context("无效的 LFS 文件大小")?)?;
        lfs::Lfs::new(self.repo.path()).read_bounded(&pointer, limit)
    }

    /// A distinct immutable path prevents old images from replacing current ones.
    pub(super) fn revision_asset(&self, commit: &str, source: &str, preview: bool) -> Result<Option<String>> {
        let Some((path, pointer)) = self.revision_asset_pointer(commit, source)? else { return Ok(None) };
        let bytes = lfs::Lfs::new(self.repo.path()).read(&pointer)?;
        let ext = Path::new(&path).extension().and_then(|e| e.to_str()).unwrap_or("bin");
        let dir = if preview { ".quiet/git-preview" } else { path.split_once('/').context("历史资源目录缺失")?.0 };
        let name = if dir == DRAWING { uuid::Uuid::new_v4().to_string() } else { pointer.oid };
        let destination = format!("{dir}/{name}.{ext}");
        let target = safe_path(&self.root, &destination)?;
        std::fs::create_dir_all(target.parent().unwrap())?;
        if !std::fs::read(&target).is_ok_and(|existing| existing == bytes) { crate::store::write_atomic(&target, &bytes)?; }
        Ok(Some(destination))
    }

    fn apply(&self, content: &BTreeMap<String, Vec<u8>>, tree: &git2::Tree<'_>, target: Oid, branch: &str, previous: Oid) -> Result<()> {
        let reference = format!("refs/heads/{branch}");
        let index_path = self.repo.path().join("index");
        let old_index = std::fs::read(&index_path).ok();
        let current = self.files()?;
        let all: BTreeSet<_> = current.iter().cloned().chain(content.keys().cloned()).collect();
        let backup = self.repo.path().join(format!("quiet-apply-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&backup)?;
        let mut existed = BTreeSet::new();
        for path in &all {
            let file = safe_path(&self.root, path)?;
            if !current.contains(path) { ensure!(!file.exists(), "远端文件与未跟踪文件同名：{path}"); }
            if file.exists() {
                let copy = backup.join(path); std::fs::create_dir_all(copy.parent().unwrap())?;
                std::fs::copy(file, copy)?; existed.insert(path.clone());
            }
        }
        let result = (|| -> Result<()> {
            for path in &all {
                let file = safe_path(&self.root, path)?;
                if let Some(bytes) = content.get(path) {
                    std::fs::create_dir_all(file.parent().unwrap())?;
                    crate::store::write_atomic(&file, bytes)?;
                } else if file.exists() { std::fs::remove_file(file)?; }
            }
            let mut index = self.repo.index()?; index.read_tree(tree)?; index.write()?;
            self.repo.reference_matching(&reference, target, true, previous, "Quiet.md: fast-forward")?;
            Ok(())
        })();
        if let Err(error) = result {
            let mut rollback = true;
            for path in &all {
                let file = self.root.join(path);
                if existed.contains(path) { rollback &= std::fs::copy(backup.join(path), file).is_ok(); }
                else if file.exists() { rollback &= std::fs::remove_file(file).is_ok(); }
            }
            match &old_index {
                Some(bytes) => rollback &= crate::store::write_atomic(&index_path, bytes).is_ok(),
                None if index_path.exists() => rollback &= std::fs::remove_file(&index_path).is_ok(),
                _ => {},
            }
            if rollback { let _ = std::fs::remove_dir_all(&backup); }
            return Err(error.context(format!("拉取应用失败；恢复备份位置：{}", backup.display())));
        }
        let _ = std::fs::remove_dir_all(backup);
        Ok(())
    }
}

fn callbacks<'a>(auth: &'a Auth, config: &'a Config) -> git2::RemoteCallbacks<'a> {
    let mut callbacks = git2::RemoteCallbacks::new();
    let key = transport::ssh_key(&config.ssh_key);
    let ssh = transport::SshRemote::parse(&config.remote).ok().flatten();
    let ssh_username = ssh.as_ref().map(|s| s.username.clone());
    callbacks.certificate_check(move |cert, host| {
        if let Some(ssh) = &ssh && let Some(cert) = cert.as_hostkey() {
            let session = ssh2::Session::new().map_err(|e| git2::Error::from_str(&e.to_string()))?;
            let key = cert.hostkey().ok_or_else(|| git2::Error::from_str("SSH 服务器未提供主机密钥"))?;
            transport::check_host(&session, host, ssh.port, key).map_err(|e| git2::Error::from_str(&e.to_string()))?;
            return Ok(git2::CertificateCheckStatus::CertificateOk);
        }
        Ok(git2::CertificateCheckStatus::CertificatePassthrough)
    });
    let mut attempts = 0;
    callbacks.credentials(move |_, username, allowed| {
        let username = username.or(ssh_username.as_deref()).unwrap_or(&auth.username);
        if allowed.contains(git2::CredentialType::USERNAME) { return git2::Cred::username(username) }
        attempts += 1;
        if attempts > 3 { return Err(git2::Error::from_str("Git 认证失败，请检查令牌或 SSH 私钥")) }
        if allowed.contains(git2::CredentialType::SSH_KEY) {
            return match &key {
                Some(key) => git2::Cred::ssh_key(username, None, key, (!auth.ssh_passphrase.is_empty()).then_some(auth.ssh_passphrase.as_str())),
                None => git2::Cred::ssh_key_from_agent(username),
            };
        }
        if !auth.token.is_empty() && allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
            return git2::Cred::userpass_plaintext(username, &auth.token);
        }
        Err(git2::Error::from_str("Git 认证失败，请提供访问令牌或 SSH 私钥"))
    });
    callbacks.push_update_reference(|_, status| match status { Some(error) => Err(git2::Error::from_str(error)), None => Ok(()) });
    callbacks
}

fn local_remote(remote: &str) -> Option<PathBuf> {
    if remote.starts_with("file://") { return url::Url::parse(remote).ok()?.to_file_path().ok() }
    Path::new(remote).is_absolute().then(|| PathBuf::from(remote))
}

fn validate_remote(remote: &str) -> Result<()> {
    ensure!(!remote.is_empty(), "请先配置远端仓库地址");
    if local_remote(remote).is_some() { return Ok(()) }
    if transport::SshRemote::parse(remote)?.is_some() { return Ok(()) }
    http_url(remote)
}

fn http_url(text: &str) -> Result<()> {
    let url = url::Url::parse(text).context("无效的服务器地址")?;
    ensure!(url.username().is_empty() && url.password().is_none(), "地址中不能包含密码或令牌，请使用认证字段");
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    ensure!(url.scheme() == "https" || (url.scheme() == "http" && loopback), "请使用 HTTPS 仓库；本地测试可使用回环 HTTP 或本地仓库路径");
    Ok(())
}

/// Only normal components, with no symlink in any existing part of the path.
pub(crate) fn safe_path(root: &Path, path: &str) -> Result<PathBuf> {
    ensure!(!path.is_empty() && !path.contains('\\') && Path::new(path).components().all(|c| matches!(c, Component::Normal(_))), "无效的文档库路径");
    let mut out = root.to_path_buf();
    for part in path.split('/') {
        out.push(part);
        match out.symlink_metadata() {
            Ok(meta) => ensure!(!meta.file_type().is_symlink(), "文档库文件不能使用符号链接：{path}"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(e.into()),
        }
    }
    Ok(out)
}

#[derive(Serialize)]
struct Metadata { id: String, extra: BTreeMap<String, serde_json::Value> }

#[derive(PartialEq, Eq)]
struct DocumentKey { body: Oid, path: String, metadata: String, drawings: BTreeMap<String, Option<Oid>> }

fn manifest(bytes: &[u8]) -> Result<BTreeMap<String, Metadata>> {
    let shared = Manifest::parse(bytes)?;
    let mut entries = BTreeMap::new();
    for note in shared.notes {
        let mut extra = note.extra; extra.insert("title".into(), note.title.into());
        extra.insert("folderId".into(), note.folder.into());
        entries.insert(format!("{NOTES}/{}", note.filename), Metadata { id: note.id, extra });
    }
    for deleted in shared.trash {
        if let Some(filename) = deleted.filename {
            let mut extra = deleted.extra; extra.insert("title".into(), deleted.title.into());
            entries.insert(format!("{NOTES}/{filename}"), Metadata { id: deleted.id, extra });
        }
    }
    Ok(entries)
}

fn collect(root: &Path, dir: &str, out: &mut BTreeSet<String>) -> Result<()> {
    let dir_path = safe_path(root, dir)?;
    if !dir_path.exists() { return Ok(()) }
    for entry in std::fs::read_dir(dir_path)? {
        let entry = entry?;
        let name = entry.file_name().into_string().map_err(|_| anyhow::anyhow!("资源文件名不是 UTF-8"))?;
        if name.starts_with('.') || name.ends_with(".bak") || name.ends_with(".tmp") || (dir == "Images/remote" && name.ends_with(".part")) { continue }
        let path = format!("{dir}/{name}");
        ensure!(!entry.file_type()?.is_symlink(), "资源不能使用符号链接：{path}");
        if entry.file_type()?.is_dir() { collect(root, &path, out)?; } else { out.insert(path); }
    }
    Ok(())
}

fn tree_files(repo: &Repository, tree: &git2::Tree<'_>) -> Result<BTreeMap<String, Oid>> {
    fn visit(repo: &Repository, tree: &git2::Tree<'_>, prefix: &str, out: &mut BTreeMap<String, Oid>) -> Result<()> {
        for entry in tree {
            let name = entry.name().context("Git 路径不是 UTF-8")?;
            let path = format!("{prefix}{name}");
            if entry.kind() == Some(git2::ObjectType::Tree) { visit(repo, &repo.find_tree(entry.id())?, &format!("{path}/"), out)?; }
            else {
                ensure!(entry.kind() == Some(git2::ObjectType::Blob) && entry.filemode() == 0o100644, "Git 文档库不允许符号链接或可执行文件");
                out.insert(path, entry.id());
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new(); visit(repo, tree, "", &mut out)?; Ok(out)
}

fn validate_tree(repo: &Repository, files: &BTreeMap<String, Oid>) -> Result<()> {
    let bytes = repo.find_blob(*files.get(MANIFEST).context("远端缺少 quiet.json")?)?;
    let entries = manifest(bytes.content()).context("远端文档库格式不兼容")?;
    for path in files.keys() {
        let allowed = matches!(path.as_str(), MANIFEST | ".gitattributes" | ".gitignore")
            || entries.contains_key(path) || is_asset_path(path);
        ensure!(allowed && !path.split('/').any(|p| p == ".." || p.eq_ignore_ascii_case(".git")), "远端包含不属于文档库的路径：{path}");
    }
    for path in entries.keys() {
        let oid = files.get(path).with_context(|| format!("远端缺少文章：{path}"))?;
        let blob = repo.find_blob(*oid)?;
        ensure!(blob.size() as u64 <= crate::store::EXPORT_FILE_LIMIT, "远端文章过大");
        let text = std::str::from_utf8(blob.content()).context("远端文章不是 UTF-8")?;
        if path.ends_with(".xml") { crate::docxml::parse(text).context("远端 DocxXML 文章无效")?; }
    }
    Ok(())
}

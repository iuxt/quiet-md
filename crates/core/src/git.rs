//! Embedded Git for opt-in libraries. No Git executable, hooks or credential helpers.
use anyhow::{Context, Result, ensure};
use git2::{Index, IndexEntry, IndexTime, Oid, Repository, Signature};
use serde::{Deserialize, Serialize};
use std::{collections::{BTreeMap, BTreeSet}, path::{Component, Path, PathBuf}};
use crate::store::{FORMAT, RESOURCE_DIRS, Store, virtual_format};

mod lfs;
mod transport;
mod changes;
mod staging;
pub use changes::ChangePreview;
pub use lfs::Pointer;
pub(crate) use staging::CommitScope;
#[cfg(test)] mod tests;

pub const MANIFEST: &str = "quiet.json";
const FORMAT_PATH: &str = ".quiet/format.json";
fn asset_attributes() -> impl Iterator<Item = String> {
    RESOURCE_DIRS.into_iter().map(|dir| format!("/{dir}/** filter=lfs diff=lfs merge=lfs -text"))
}

fn is_asset_path(path: &str) -> bool {
    path.split_once('/').is_some_and(|(dir, _)| RESOURCE_DIRS.contains(&dir))
}

fn write_rules(root: &Path) -> Result<()> {
    let attributes = asset_attributes().collect::<Vec<_>>().join("\n");
    let ignores = ".DS_Store\n.quiet/*\n!.quiet/format.json\n!.quiet/trash/\n.quiet/trash/*.bak\n/Image/remote/*.part\n*.tmp\nquiet.json.bak\n";
    for (name, additions) in [(".gitattributes", attributes.as_str()), (".gitignore", ignores)] {
        let path = safe_path(root, name)?;
        let mut text = if path.exists() { std::fs::read_to_string(&path)? } else { String::new() };
        let original_len = text.len();
        for line in additions.lines() {
            if text.lines().any(|existing| existing == line) { continue }
            if !text.is_empty() && !text.ends_with('\n') { text.push('\n'); }
            text.push_str(line); text.push('\n');
        }
        if text.len() != original_len { crate::store::write_atomic(&path, text.as_bytes())?; }
    }
    Ok(())
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

#[derive(Clone, Default)]
pub struct Config { pub name: String, pub email: String, pub remote: String, pub branch: String, pub lfs_url: String, pub ssh_key: String, pub lfs_ca_info: String }
#[derive(Clone, Debug)]
pub struct Change { pub path: String, pub kind: char, pub staged: bool, pub unstaged: bool, pub blank_only: bool }
#[derive(Clone, Debug, Default)]
pub struct Status { pub branch: String, pub remote: String, pub head: Option<String>, pub changes: Vec<Change> }
impl Status {
    pub fn visible_changes(&self) -> impl Iterator<Item = &Change> { self.changes.iter().filter(|c| !c.blank_only) }
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
        Self::check_location(root)?;
        let mut options = git2::RepositoryInitOptions::new();
        options.initial_head("main");
        let repo = Repository::init_opts(root, &options)?;
        write_rules(root)?;
        Ok(Self { root: root.canonicalize()?, repo })
    }

    pub fn clone_into(remote: &str, root: &Path, auth: &Auth) -> Result<Self> {
        Self::clone_configured(&Config { remote: remote.to_string(), ..Default::default() }, root, auth)
    }

    pub fn clone_configured(config: &Config, root: &Path, auth: &Auth) -> Result<Self> {
        validate_remote(&config.remote)?;
        std::fs::create_dir_all(root)?;
        ensure!(std::fs::read_dir(root)?.next().is_none(), "克隆目标必须是空文件夹");
        std::fs::create_dir_all(root.join(".quiet"))?;
        crate::store::write_atomic(&root.join(".quiet/format.json"), b"{\"format\":5}")?;
        crate::store::write_atomic(&root.join(MANIFEST), b"{}")?;
        let repo = Self::init(root)?;
        repo.configure(config)?;
        {
            let mut origin = repo.repo.find_remote("origin")?;
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

    pub fn config(&self) -> Result<Config> {
        let config = self.repo.config()?;
        let mut result = Config {
            name: config.get_string("user.name").unwrap_or_default(), email: config.get_string("user.email").unwrap_or_default(),
            remote: self.repo.find_remote("origin").ok().and_then(|r| r.url().ok().map(str::to_string)).unwrap_or_default(),
            branch: self.repo.head().ok().and_then(|h| h.shorthand().ok().map(str::to_string))
                .or_else(|| self.repo.find_reference("HEAD").ok().and_then(|h| h.symbolic_target().ok().flatten().and_then(|s| s.strip_prefix("refs/heads/")).map(str::to_string)))
                .unwrap_or_else(|| "main".into()),
            lfs_url: config.get_string("lfs.url").unwrap_or_default(),
            ssh_key: config.get_string("quiet.sshKey").unwrap_or_default(),
            lfs_ca_info: String::new(),
        };
        if let Ok(endpoint) = transport::lfs_endpoint(&result) {
            result.lfs_ca_info = transport::ca_info(&config, &endpoint)?.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        }
        Ok(result)
    }

    pub fn configure(&self, config: &Config) -> Result<()> {
        if !config.remote.is_empty() { validate_remote(&config.remote)?; }
        if !config.lfs_url.is_empty() { http_url(&config.lfs_url)?; }
        let previous = self.config()?;
        let mut local = self.repo.config()?.open_level(git2::ConfigLevel::Local)?;
        if config.ssh_key.is_empty() { let _ = local.remove("quiet.sshKey"); } else { local.set_str("quiet.sshKey", &config.ssh_key)?; }
        if (config.lfs_ca_info != previous.lfs_ca_info || config.remote != previous.remote || config.lfs_url != previous.lfs_url)
            && let Ok(endpoint) = transport::lfs_endpoint(config) {
            let url = url::Url::parse(&endpoint)?;
            let key = if config.remote == previous.remote && config.lfs_url == previous.lfs_url {
                transport::ca_info_key(&self.repo.config()?, &endpoint)?
            } else { None }.unwrap_or_else(|| format!("http.{}.sslCAInfo", url.origin().ascii_serialization()));
            if config.lfs_ca_info.is_empty() { let _ = local.remove(&key); } else { local.set_str(&key, &config.lfs_ca_info)?; }
        }
        local.set_str("user.name", config.name.trim())?;
        local.set_str("user.email", config.email.trim())?;
        if config.lfs_url.is_empty() { let _ = local.remove("lfs.url"); } else { local.set_str("lfs.url", &config.lfs_url)?; }
        if config.remote.is_empty() {
            if self.repo.find_remote("origin").is_ok() { self.repo.remote_delete("origin")?; }
        } else if self.repo.find_remote("origin").is_ok() { self.repo.remote_set_url("origin", &config.remote)?; }
        else { self.repo.remote("origin", &config.remote)?; }
        Ok(())
    }

    fn files(&self) -> Result<BTreeSet<String>> {
        let bytes = crate::store::read_bounded(&safe_path(&self.root, MANIFEST)?, 16 << 20)?;
        let entries = manifest(&bytes)?;
        let mut files: BTreeSet<_> = [MANIFEST, ".quiet/format.json", ".gitattributes", ".gitignore"].into_iter().map(str::to_string).collect();
        files.extend(entries.keys().filter(|p| !p.ends_with('/')).cloned());
        for dir in RESOURCE_DIRS { collect(&self.root, dir, &mut files)?; }
        if serde_json::from_slice::<serde_json::Value>(&bytes)?.get("version").is_none() { collect(&self.root, ".quiet/trash", &mut files)?; }
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
                let blank_only = !is_asset_path(&path) && self.blank_line_delta(&delta)?;
                let change = changes.entry(path.clone()).or_insert(Change { path, kind: 'M', staged: false, unstaged: false, blank_only: true });
                change.blank_only &= blank_only;
                change.kind = match delta.status() { git2::Delta::Added => 'A', git2::Delta::Deleted => 'D', _ => 'M' };
                if staged { change.staged = true; } else { change.unstaged = true; }
            }
        }
        Ok(changes.into_values().collect())
    }

    fn blank_line_delta(&self, delta: &git2::DiffDelta<'_>) -> Result<bool> {
        let (old, new) = (delta.old_file(), delta.new_file());
        if delta.status() != git2::Delta::Modified
            || !matches!((old.mode(), new.mode()), (git2::FileMode::Blob, git2::FileMode::Blob)) { return Ok(false) }
        let old = self.repo.find_blob(old.id())?; let new = self.repo.find_blob(new.id())?;
        let (Ok(old), Ok(new)) = (std::str::from_utf8(old.content()), std::str::from_utf8(new.content())) else { return Ok(false) };
        if old.contains('\0') || new.contains('\0') { return Ok(false) }
        // Keep the exact bytes of nonblank lines, including their line endings.
        fn nonblank(text: &str) -> impl Iterator<Item = &str> {
            text.split_inclusive('\n').filter(|line| !line.trim_ascii().is_empty())
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
            if let Some(doc) = document {
                let current = self.document_key(&commit.tree()?, doc)?;
                let previous = commit.parent(0).ok().map(|p| self.document_key(&p.tree()?, doc)).transpose()?.flatten();
                if current == previous { continue }
            }
            out.push(Commit { id: commit.id().to_string(), message: commit.summary().ok().flatten().unwrap_or("").to_string(), author: commit.author().name().unwrap_or("").to_string(), time: commit.time().seconds() });
            if out.len() >= limit.min(200) { break }
        }
        Ok(out)
    }

    fn document_blob(&self, tree: &git2::Tree<'_>, document: &str) -> Result<Option<Oid>> {
        Ok(self.document_key(tree, document)?.map(|key| key.0))
    }

    fn document_key(&self, tree: &git2::Tree<'_>, document: &str) -> Result<Option<(Oid, String, Option<String>)>> {
        let Some(entry) = tree.get_name(MANIFEST) else { return Ok(None) };
        let meta = self.repo.find_blob(entry.id())?;
        let entries = manifest(meta.content())?;
        let Some((path, meta)) = entries.iter().find(|(path, meta)| !path.ends_with('/') && meta.id == document) else { return Ok(None) };
        let title = Some(serde_json::to_string(meta)?);
        Ok(tree.get_path(Path::new(path)).ok().map(|entry| (entry.id(), path.clone(), title)))
    }

    pub fn revision(&self, commit: &str, document: &str) -> Result<String> {
        let commit = self.repo.find_commit(Oid::from_str(commit)?)?;
        let oid = self.document_blob(&commit.tree()?, document)?.context("这个提交中没有此文章")?;
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
        let pointers = self.pointers(Some(commit.id()), &head)?;
        lfs::Lfs::new(self.repo.path()).transfer(&config, &self.repo.config()?, &pointers, true, auth)?;
        let mut remote = self.repo.find_remote("origin")?;
        let mut options = git2::PushOptions::new(); options.remote_callbacks(callbacks(auth, &config));
        remote.push(&[&format!("{}:refs/heads/{}", commit.id(), config.branch)], Some(&mut options))?;
        Ok(())
    }

    /// Fast-forward only. Fetch/LFS failures never change current files or HEAD.
    pub fn pull(&self, auth: &Auth) -> Result<bool> {
        ensure!(self.ready_to_pull()?, "请先提交本地变更，再拉取远端版本");
        let config = self.config()?; validate_remote(&config.remote)?;
        let mut remote = self.repo.find_remote("origin")?;
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
        // A fresh empty clone has only bootstrap metadata, not an initial commit.
        let bytes = std::fs::read(self.root.join(MANIFEST))?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        let empty = if value.get("version").is_some() {
            let shared = crate::manifest::Manifest::parse(&bytes)?;
            shared.folders.is_empty() && shared.notes.is_empty() && shared.trash.is_empty()
        } else { manifest(&bytes)?.is_empty() };
        Ok(status.head.is_none() && empty
            && self.files()?.iter().all(|p| matches!(p.as_str(), MANIFEST | FORMAT_PATH | ".gitattributes" | ".gitignore")))
    }

    pub fn revision_paths(&self, commit: &str) -> Result<BTreeMap<String, String>> {
        let tree = self.repo.find_commit(Oid::from_str(commit)?)?.tree()?;
        let blob = self.repo.find_blob(tree.get_name(MANIFEST).context("提交中缺少元数据")?.id())?;
        Ok(manifest(blob.content())?.into_iter().filter(|(p, _)| !p.ends_with('/')).map(|(p, m)| (m.id, p)).collect())
    }

    pub fn revision_title(&self, commit: &str, document: &str) -> Result<Option<String>> {
        let tree = self.repo.find_commit(Oid::from_str(commit)?)?.tree()?;
        let meta = self.repo.find_blob(tree.get_name(MANIFEST).context("提交中缺少元数据")?.id())?;
        Ok(manifest(meta.content())?.values().find(|m| m.id == document).and_then(|m| m.extra.get("title")).and_then(|v| v.as_str()).map(str::to_string))
    }

    fn tree_format(&self, tree: &git2::Tree<'_>) -> Result<u32> {
        let Ok(entry) = tree.get_path(Path::new(FORMAT_PATH)) else { return Ok(0) };
        parse_format(self.repo.find_blob(entry.id())?.content())
    }

    fn revision_asset_pointer(&self, commit: &str, source: &str) -> Result<Option<(String, Pointer)>> {
        let tree = self.repo.find_commit(Oid::from_str(commit)?)?.tree()?;
        let source = source.strip_prefix("file://").unwrap_or(source).replace('\\', "/");
        let absolute = source.starts_with('/') || source.as_bytes().get(1) == Some(&b':');
        let relative = if absolute {
            let root = self.root.to_string_lossy().replace('\\', "/");
            source.strip_prefix(&root).and_then(|rest| rest.strip_prefix('/')).filter(|path| is_asset_path(path))
                .or_else(|| source.match_indices('/').map(|(i, _)| &source[i + 1..])
                    .find(|path| is_asset_path(path) && tree.get_path(Path::new(path)).is_ok()))
        } else { is_asset_path(&source).then_some(source.as_str()) };
        let Some(path) = relative else { return Ok(None) };
        let entry = tree.get_path(Path::new(path))?;
        ensure!(entry.filemode() == 0o100644, "历史资源不是普通文件");
        let blob = self.repo.find_blob(entry.id())?;
        let pointer = Pointer::parse(blob.content())?.context("历史资源未使用 LFS")?;
        Ok(Some((path.to_string(), pointer)))
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
        let destination = format!("{dir}/{}.{}", pointer.oid, ext);
        let target = safe_path(&self.root, &destination)?;
        std::fs::create_dir_all(target.parent().unwrap())?;
        if !std::fs::read(&target).is_ok_and(|existing| existing == bytes) { crate::store::write_atomic(&target, &bytes)?; }
        Ok(Some(destination))
    }

    fn apply(&self, content: &BTreeMap<String, Vec<u8>>, tree: &git2::Tree<'_>, target: Oid, branch: &str, previous: Oid) -> Result<()> {
        let old_entries = manifest(&std::fs::read(self.root.join(MANIFEST))?)?;
        let entries = manifest(content.get(MANIFEST).context("缺少 quiet.json")?)?;
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
            for path in entries.keys().filter(|p| p.ends_with('/')) { std::fs::create_dir_all(safe_path(&self.root, path.trim_end_matches('/'))?)?; }
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
        // Empty physical folders otherwise reappear as new metadata on the next scan.
        let mut obsolete: Vec<_> = old_entries.keys().filter(|p| p.ends_with('/') && !entries.contains_key(*p)).collect();
        obsolete.sort_by_key(|p| std::cmp::Reverse(p.len()));
        for path in obsolete { let _ = std::fs::remove_dir(self.root.join(path)); }
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

#[derive(Serialize, Deserialize)]
struct Metadata { id: String, #[serde(flatten)] extra: BTreeMap<String, serde_json::Value> }
fn manifest(bytes: &[u8]) -> Result<BTreeMap<String, Metadata>> {
    ensure!(bytes.len() <= 16 << 20, "文档库元数据过大");
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    if value.get("version").is_some() {
        let shared = crate::manifest::Manifest::parse(bytes)?;
        let mut entries = BTreeMap::new();
        for note in shared.notes {
            let mut extra = note.extra; extra.insert("title".into(), note.title.into());
            extra.insert("folderId".into(), note.folder.into());
            entries.insert(format!("notes/{}", note.filename), Metadata { id: note.id, extra });
        }
        for deleted in shared.trash {
            if let Some(path) = deleted.body_rel() {
                let mut extra = deleted.extra; extra.insert("title".into(), deleted.title.into());
                entries.insert(path, Metadata { id: deleted.id, extra });
            }
        }
        return Ok(entries);
    }
    let entries: BTreeMap<String, Metadata> = serde_json::from_slice(bytes).context("quiet.json 元数据无效")?;
    let mut ids = BTreeSet::new();
    for (path, meta) in &entries {
        let name = path.trim_end_matches('/');
        ensure!(!name.split('/').any(|p| p.starts_with('.') || p.is_empty()), "元数据中存在非法文件路径");
        ensure!(uuid::Uuid::parse_str(&meta.id).is_ok() && ids.insert(&meta.id), "元数据中存在无效或重复 id");
        if !path.ends_with('/') {
            ensure!(Path::new(name).file_stem().and_then(|s| s.to_str()) == Some(&meta.id), "文章文件名必须使用唯一 UUID");
            ensure!(name.ends_with(".xml") || name.ends_with(".md"), "不支持的文章格式");
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
        if name.starts_with('.') || name.ends_with(".bak") || name.ends_with(".tmp") || (dir == "Image/remote" && name.ends_with(".part")) { continue }
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

fn parse_format(bytes: &[u8]) -> Result<u32> {
    #[derive(Deserialize)]
    struct Marker { format: u32 }
    Ok(serde_json::from_slice::<Marker>(bytes)?.format)
}

fn validate_tree(repo: &Repository, files: &BTreeMap<String, Oid>) -> Result<u32> {
    let bytes = repo.find_blob(*files.get(MANIFEST).context("远端缺少 quiet.json")?)?;
    let entries = manifest(bytes.content())?;
    let virtual_manifest = serde_json::from_slice::<serde_json::Value>(bytes.content())?.get("version").is_some();
    let marker = repo.find_blob(*files.get(FORMAT_PATH).context("远端缺少格式标记")?)?;
    let format = parse_format(marker.content())?;
    ensure!(if virtual_manifest { virtual_format(format) } else { format == 5 }, "远端文档库格式不兼容");
    for path in files.keys() {
        let allowed = matches!(path.as_str(), MANIFEST | ".gitattributes" | ".gitignore" | ".quiet/format.json")
            || entries.contains_key(path) || is_asset_path(path) || (!virtual_manifest && path.starts_with(".quiet/trash/"));
        ensure!(allowed && !path.split('/').any(|p| p == ".." || p.eq_ignore_ascii_case(".git")), "远端包含不属于文档库的路径：{path}");
    }
    for path in entries.keys().filter(|p| !p.ends_with('/')) {
        let oid = files.get(path).with_context(|| format!("远端缺少文章：{path}"))?;
        let blob = repo.find_blob(*oid)?;
        ensure!(blob.size() as u64 <= crate::store::EXPORT_FILE_LIMIT, "远端文章过大");
        let text = std::str::from_utf8(blob.content()).context("远端文章不是 UTF-8")?;
        if path.ends_with(".xml") { crate::docxml::parse(text).context("远端 DocxXML 文章无效")?; }
    }
    Ok(format)
}

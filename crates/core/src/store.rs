//! Library storage: virtual folders in quiet.json, fixed Notes/<UUID> bodies.

use crate::docxml;
use crate::model::Document;
use anyhow::{Context, Result};
use std::io::Write;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use crate::time::now;
#[path = "file_history.rs"] mod file_history;
pub use file_history::{FileState, FileDelta, FileChanges};
#[path = "layout.rs"] mod layout;
pub use layout::{relative, resolve};
use layout::{parent_rel, join_rel};
use crate::manifest::{FILE, Manifest, NOTES};
#[path = "git_store.rs"] mod git_store;
#[path = "virtual_store.rs"] mod virtual_store;
#[cfg(test)] #[path = "virtual_tests.rs"] mod virtual_tests;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Meta {
    /// A folder, by its display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_order: Option<u64>,
    pub created: u64,
    pub modified: u64,
    #[serde(default)]
    pub pinned: bool,
    /// Last time the document was opened, for 最近访问 (this computer only).
    #[serde(default)]
    pub opened: u64,
    /// 页宽设置, remembered per document.
    #[serde(default, skip_serializing_if = "PageWidth::is_standard")]
    pub width: PageWidth,
    /// Catalogue visibility is remembered independently of document headings.
    #[serde(default = "catalogue_closed_default")]
    pub catalogue_closed: bool,
    /// The folder it is in (its id); follows from where the file is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Position among its siblings (milliseconds of the last move); 0 falls
    /// back to `created`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub order: u64,
    /// 收藏时间 (0: not starred).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub starred: u64,
    /// 阅读模式, remembered per document like the page width.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reading: bool,
    /// The 文档信息 row under the title.
    #[serde(default, skip_serializing_if = "InfoRow::is_default")]
    pub info: InfoRow,
    /// 隐藏引用列表 (the "…" menu's 显示引用列表 switch off).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub refs_hidden: bool,
    /// A Markdown file, edited as Markdown.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub markdown: bool,
    /// 本机文件: a Markdown file opened where it is, outside the library.
    /// The library never moves, trashes or deletes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// A library document's display title; separate from a UUID file name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Where it is in the library, `/`-separated (empty for a 本机文件). In
    /// the 回收站: where it was.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rel: String,
}

impl Meta {
    /// 本机文件: outside the library's tree.
    pub fn local(&self) -> bool { self.path.is_some() }
}

fn catalogue_closed_default() -> bool { true }

impl Default for Meta {
    fn default() -> Self {
        Self {
            folder: None, root_order: None, created: 0, modified: 0, pinned: false, opened: 0,
            width: PageWidth::default(), catalogue_closed: true,
            parent: None, order: 0, starred: 0, reading: false,
            info: InfoRow::default(), refs_hidden: false,
            markdown: false, path: None, name: None, rel: String::new(),
        }
    }
}

/// The 文档信息 row under the title: which items it shows (选择要展示的文档信息)
/// and whether it is hidden altogether (隐藏文档信息).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InfoRow {
    pub hidden: bool,
    pub created: bool,
    pub modified: bool,
}

impl Default for InfoRow {
    fn default() -> Self {
        InfoRow { hidden: false, created: false, modified: true }
    }
}

impl InfoRow {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

impl Meta {
    /// Where a page or folder sits among its siblings: the sidebar's order, the
    /// numbers a drop writes and Home's manual order all use it. Those never
    /// placed by hand come first, most recently changed first; then the placed
    /// ones in their order. Callers break remaining ties by id.
    pub fn place(&self) -> (bool, u64, std::cmp::Reverse<u64>) {
        let placed = if self.parent.is_none() { self.root_order.unwrap_or(0) } else { self.order };
        (placed > 0, placed, std::cmp::Reverse(self.modified))
    }
}

/// Three page widths: 默认 (820px column), 较宽 (1020px), 全宽.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PageWidth {
    #[default]
    Standard,
    Wide,
    Full,
}

impl PageWidth {
    pub(crate) fn is_standard(&self) -> bool {
        *self == PageWidth::Standard
    }

    /// Max width of the content column; `None` fills the page.
    pub fn max(self) -> Option<f32> {
        match self {
            PageWidth::Standard => Some(820.),
            PageWidth::Wide => Some(1020.),
            PageWidth::Full => None,
        }
    }
}

/// Window-level UI state: `.quiet/settings.json`, except the window sizes,
/// which belong to this computer.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quick_note_size: Option<[f32; 2]>,
    /// Dragged sidebar width; `None` is the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidebar_width: Option<f32>,
    /// 隐藏引用列表's 不再提示.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub refs_hide_no_ask: bool,
    /// 主页's sort order ("visited" …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_sort: Option<String>,
    #[serde(default)]
    pub home_folders_first: bool,
    #[serde(default)]
    pub home_manual_initialized: bool,
    /// Main window size, remembered like the quick note window's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_size: Option<[f32; 2]>,
    /// 阅读位置: the list offset each document was left at, so switching back
    /// returns to the same place. Kept out of `index.json`: it is window
    /// state, not document metadata.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub scroll_top: HashMap<String, [f32; 2]>,
    /// 版本历史: snapshots kept per document (1–30). None is the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_limit: Option<u32>,
}

impl Settings {
    pub fn history_limit(&self) -> u32 { crate::versions::clamp_limit(self.history_limit.unwrap_or(crate::versions::DEFAULT_LIMIT)) }
    pub fn set_history_limit(&mut self, n: u32) { self.history_limit = Some(crate::versions::clamp_limit(n)); }
}

/// A document in the 回收站, as kept in quiet.json (legacy: `.quiet/trash/index.json`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trashed {
    pub meta: Meta,
    pub deleted: u64,
    /// The page whose deletion took this one along (`None`: deleted itself).
    /// Such pages are listed and restored with that page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
}

/// A row of the 回收站 page: a deleted page and how many went with it.
#[derive(Clone, Debug)]
pub struct TrashEntry {
    pub id: String,
    pub meta: Meta,
    pub title: String,
    pub icon: Option<String>,
    pub deleted: u64,
    /// Where it was: its parent page, `None` for the top of 我的文档库.
    pub parent: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: String,
    pub meta: Meta,
    pub title: String,
    /// 文档图标 emoji.
    pub icon: Option<String>,
    /// Plain text body, used for search and the list snippet.
    pub body: std::sync::Arc<str>,
    /// Its 日期提醒, for the system notifications.
    pub reminders: Vec<crate::model::InlineReminder>,
    /// Its @文档 mentions, for the 反向引用 of the documents they point at.
    pub links: Vec<crate::model::DocLink>,
}

pub struct Store {
    file_changes: Vec<std::sync::Weak<std::sync::Mutex<Vec<FileDelta>>>>,
    loaded: std::cell::RefCell<HashMap<String, Arc<String>>>,
    loaded_titles: std::cell::RefCell<HashMap<String, String>>,
    /// Library images and attachments copied beside each 本机文件 this
    /// session, so autosaves reuse the copy instead of making another.
    copied: std::cell::RefCell<HashMap<String, HashMap<PathBuf, String>>>,
    /// What `list` parsed from each file, kept while its mtime and size stay
    /// the same: re-reading every document on each window activation stalled
    /// the main thread for seconds on big or iCloud-evicted libraries.
    parsed: HashMap<String, (std::time::SystemTime, u64, Parsed)>,
    parsed_disk: ParsedDisk,
    root: PathBuf,
    /// Every document and virtual folder by its stable UUID; quiet.json
    /// stores the tree while document links remain relative file paths.
    index: HashMap<String, Meta>,
    trash: HashMap<String, Trashed>,
    manifest: Manifest,
    /// Problems found during library operations.
    pub warnings: Vec<String>,
    batch: Batch,
}

/// The parts of an `Entry` that come from the document file.
#[derive(Clone)]
struct Parsed { title: String, icon: Option<String>, body: std::sync::Arc<str>, reminders: Vec<crate::model::InlineReminder>, links: Vec<crate::model::DocLink> }

/// `Store::parsed` kept between launches (#170): reading every document again
/// on launch took ~0.45 s a file on Windows, where each read is scanned. By
/// file path, so a cache never lends one file's text to another; the stamps
/// are this machine's mtimes, so it lives in the cache folder, not in the
/// (possibly synced) library. Bump `PARSED_CACHE` when what `list` takes from
/// a file changes; a release discards it anyway.
const PARSED_CACHE: u32 = 2;

#[derive(Clone, Default)]
struct ParsedDisk { generation: Option<String>, stamps: HashMap<String, (std::time::SystemTime, u64)>, deltas: HashSet<PathBuf> }

#[derive(Serialize, Deserialize)]
struct ParsedCache { version: String, generation: String, docs: HashMap<PathBuf, CachedDoc> }

#[derive(Serialize, Deserialize)]
struct ParsedDelta { generation: String, path: PathBuf, doc: CachedDoc }

#[derive(Serialize, Deserialize)]
struct CachedDoc {
    modified: std::time::SystemTime,
    len: u64,
    title: String,
    icon: Option<String>,
    body: String,
    reminders: Vec<(i64, i64, bool, bool)>,
    links: Vec<(String, usize, Vec<(String, Option<String>)>)>,
}

fn parsed_cache_version() -> String { format!("{}-{PARSED_CACHE}", env!("CARGO_PKG_VERSION")) }

/// Beside the other things this computer keeps about `root`, in the cache folder.
fn parsed_cache_path(root: &Path) -> PathBuf {
    let tmp = std::env::temp_dir();
    let base = if cfg!(test) || root.starts_with(&tmp) { tmp.join("quiet-md-local") } else { dirs::cache_dir().unwrap_or(tmp).join("quiet-md") };
    base.join("list").join(format!("{}.json", library_key(root)))
}

/// One write to the indexes: several copies of the app may share a library
/// (builds tested side by side), so this is a short mutex, not a
/// single-instance guard. Released when the write is done.
/// `None`: the lock a batch already holds.
pub(crate) struct WriteLock(Option<std::fs::File>);

impl Drop for WriteLock {
    fn drop(&mut self) { if let Some(f) = &self.0 { let _ = f.unlock(); } }
}

/// A batch of writes (`Store::batch`): the lock is held throughout and the
/// index is written once at the end, not once per document.
#[derive(Default)]
struct Batch { lock: Option<WriteLock>, depth: u32, index: std::cell::Cell<bool>, dirs: std::cell::RefCell<Vec<PathBuf>> }

pub const INTERNAL: &str = ".quiet";
pub const IMAGE: &str = "Images";
pub const FILES: &str = "Files";
pub const DRAWING: &str = "Drawings";
pub(crate) const RESOURCE_DIRS: [&str; 3] = [IMAGE, FILES, DRAWING];

#[path = "folders.rs"]
mod folders;

/// What belongs to this computer, not to the library (which may be synced):
/// kept per library in the local application data folder.
#[derive(Clone, Default, Serialize, Deserialize)]
struct Local {
    /// Document id → last opened.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    opened: BTreeMap<String, u64>,
    /// 本机文件: their paths are this computer's.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    files: BTreeMap<String, Meta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    quick_note_size: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    window_size: Option<[f32; 2]>,
}

/// `library.json`: where the library is when it isn't the default.
#[derive(Serialize, Deserialize)]
struct Location { path: PathBuf }

/// The library this run uses once first launch has picked it.
static ROOT: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);

/// Write `bytes` to `path` through a temporary file, so a crash leaves
/// either the old or the new contents.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> { write_atomic_then(path, bytes, true, || Ok(())) }

pub(crate) fn write_atomic_after(path: &Path, bytes: &[u8], before_replace: impl FnOnce() -> Result<()>) -> Result<()> {
    write_atomic_then(path, bytes, true, before_replace)
}

/// `write_atomic`; without `sync_dir` the folder's entry is left for the
/// caller to sync (a batch syncs each folder once, before its index).
fn write_atomic_then(path: &Path, bytes: &[u8], sync_dir: bool, before_replace: impl FnOnce() -> Result<()>) -> Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let tmp = PathBuf::from(tmp);
    let result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        before_replace()?;
        std::fs::rename(&tmp, path)?;
        #[cfg(unix)]
        if sync_dir && let Some(parent) = path.parent() { std::fs::File::open(parent)?.sync_all()?; }
        // Windows has no directory fsync; the rename is as far as it goes.
        #[cfg(not(unix))]
        let _ = sync_dir;
        Ok(())
    })();
    if result.is_err() { let _ = std::fs::remove_file(&tmp); }
    result
}

/// A JSON file as another copy of the app left it, if it can be read right now.
fn reread<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Atomically replace a JSON file if its contents changed.
pub(crate) fn save_json<T: Serialize>(path: &Path, v: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(v)?;
    if std::fs::read(path).is_ok_and(|old| old == bytes) { return Ok(()); }
    write_atomic(path, &bytes)
}

/// Image path for an image block source (only local files are displayed).
pub fn image_path(src: &str) -> Option<PathBuf> { image_path_in(&Store::root_dir(), src) }

pub fn image_path_in(root: &Path, src: &str) -> Option<PathBuf> {
    let p = PathBuf::from(src.strip_prefix("file://").unwrap_or(src));
    if p.is_absolute() {
        return Some(moved_asset(root, &p).unwrap_or(p));
    }
    // Relative to the library; not a URI or a cloud token.
    (!src.is_empty() && !src.contains(':') && src.contains('/')).then(|| root.join(p))
}

/// Find a missing absolute asset path after the library has moved.
fn moved_asset(root: &Path, p: &Path) -> Option<PathBuf> {
    use std::path::Component;
    if p.exists() { return None; }
    let parts: Vec<Component> = p.components().collect();
    // Use the last resource directory whose suffix still resolves in this library.
    (0..parts.len()).rev().find_map(|i| {
        let dir = parts[i].as_os_str().to_str()?;
        let rest = &parts[i + 1..];
        if !RESOURCE_DIRS.contains(&dir) || rest.is_empty() || !rest.iter().all(|c| matches!(c, Component::Normal(_))) { return None; }
        let path = root.join(dir).join(rest.iter().collect::<PathBuf>());
        path.exists().then_some(path)
    })
}

/// Export only files belonging to this library, including after symlink resolution.
pub fn export_path(src: &str) -> Option<PathBuf> {
    export_path_in(&Store::root_dir(), src)
}

pub fn export_path_in(root: &std::path::Path, src: &str) -> Option<PathBuf> {
    let root = root.canonicalize().ok()?;
    let raw = std::path::Path::new(src.strip_prefix("file://").unwrap_or(src));
    let path = if raw.is_absolute() { moved_asset(&root, raw).unwrap_or_else(|| raw.to_path_buf()) } else { root.join(raw) }.canonicalize().ok()?;
    (path.starts_with(root) && path.is_file()).then_some(path)
}

pub const EXPORT_FILE_LIMIT: u64 = 256 * 1024 * 1024;

/// Bound the actual read as well as metadata: files can grow after inspection.
pub fn read_bounded(path: &std::path::Path, limit: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    anyhow::ensure!(file.metadata()?.len() <= limit, "文件超过导出大小限制");
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() as u64 <= limit, "文件超过导出大小限制");
    Ok(bytes)
}

/// Read a drawing asset from the library or directly from a Git history revision.
pub fn read_drawing_asset(root: &Path, source: &str, limit: u64) -> Result<Vec<u8>> {
    if let Some((oid, size)) = source.strip_prefix("git-lfs:").and_then(|s| s.split_once(':')) {
        return crate::git::LibraryRepo::open(root)?.read_drawing_asset(oid, size, limit);
    }
    read_bounded(&export_path_in(root, source).context("drawing asset is missing")?, limit)
}

/// A short stable name for a library folder, for data kept about it elsewhere.
fn library_key(root: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    root.hash(&mut hash);
    format!("{:016x}", hash.finish())
}

/// Where this computer keeps what is not `root`'s own: the application data
/// folder, by library; a temporary one for tests and libraries in the
/// temporary folder (scripted runs), which never touch the real one.
fn local_file_for(root: &Path) -> PathBuf {
    let tmp = std::env::temp_dir();
    let base = if cfg!(test) || root.starts_with(&tmp) || tmp.canonicalize().is_ok_and(|t| root.starts_with(t)) { tmp.join("quiet-md-local") }
        else { dirs::data_local_dir().unwrap_or_else(std::env::temp_dir).join("quiet-md") };
    base.join("libraries").join(format!("{}.json", library_key(root)))
}

/// The id, name and parent of every library entry, from where it is.
fn ext_of(m: &Meta) -> &'static str { if m.markdown { "md" } else { "xml" } }

impl Store {
    /// `QUIET_MD_HOME`, else the folder picked at first launch (`library.json`
    /// outside the library, since everything else lives inside it), else a
    /// folder of its own in the user's Documents, where they can see and back it up.
    pub fn root_dir() -> PathBuf {
        if let Some(p) = ROOT.read().unwrap().clone() { return p }
        std::env::var_os("QUIET_MD_HOME").map(PathBuf::from)
            .or_else(|| std::fs::read(Self::location_file()).ok().and_then(|b| serde_json::from_slice::<Location>(&b).ok()).map(|l| l.path).filter(|p| p.is_absolute()))
            .unwrap_or_else(Self::default_root)
    }

    pub fn default_root() -> PathBuf {
        dirs::document_dir().or_else(|| dirs::home_dir().map(|h| h.join("Documents"))).unwrap_or_else(|| PathBuf::from(".")).join("Quiet.md")
    }

    /// The library's own data (`.quiet/`): preferences, shortcuts, themes,
    /// the index and the 回收站. Created on demand.
    pub fn internal_dir() -> PathBuf {
        let root = Self::root_dir();
        let d = root.join(INTERNAL);
        let _ = std::fs::create_dir_all(&d);
        d
    }

    /// What the library's files are, without reading them: changes when a
    /// document or folder is added, removed or renamed (by hand, or a model
    /// writing a note), not when one is saved. Cheap enough to poll.
    pub fn shape_at(root: &Path) -> u64 {
        use std::hash::{Hash, Hasher};
        let found = layout::scan(&root.join(NOTES));
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (found.dirs, found.docs, found.pending).hash(&mut h);
        if let Ok(meta) = std::fs::metadata(root.join(crate::manifest::FILE)) { (meta.len(), meta.modified().ok()).hash(&mut h); }
        h.finish()
    }

    /// Recognize library directories so an unsupported format can be reported.
    pub fn is_library(dir: &Path) -> bool {
        dir.join(FILE).symlink_metadata().is_ok() || Self::has_legacy_metadata(dir)
    }

    fn has_legacy_metadata(dir: &Path) -> bool {
        dir.join(".quiet/index.json").symlink_metadata().is_ok()
            || (dir.join("index.json").is_file() && dir.join("docs").is_dir())
    }

    /// The pointer to a library outside Documents; per user, never synced.
    pub fn location_file() -> PathBuf {
        dirs::data_local_dir().unwrap_or_else(std::env::temp_dir).join("quiet-md").join("library.json")
    }

    /// Prepare a new library if needed and use `root` for the rest of this run, and from now on unless
    /// `QUIET_MD_HOME` says otherwise. The default location needs no pointer.
    pub fn set_root_dir(root: PathBuf) -> Result<()> {
        Self::prepare_layout(&root)?;
        let file = Self::location_file();
        // Test runs (`QUIET_MD_HOME`) never move the real library.
        if std::env::var_os("QUIET_MD_HOME").is_some() {}
        else if root == Self::default_root() { let _ = std::fs::remove_file(&file); }
        else {
            std::fs::create_dir_all(file.parent().unwrap())?;
            write_atomic(&file, &serde_json::to_vec_pretty(&Location { path: root.clone() })?)?;
        }
        *ROOT.write().unwrap() = Some(root);
        Ok(())
    }

    /// A new drawing gets its own source path; later edits update that file.
    pub fn save_drawing(root: &Path, source: &[u8], width: u32, height: u32) -> Result<crate::model::Drawing> {
        let id = uuid::Uuid::new_v4();
        Self::write_drawing(root, &format!("{DRAWING}/{id}.excalidraw"), source, width, height, None)
    }

    pub fn update_drawing(root: &Path, path: &str, previous: &crate::git::Pointer, source: &[u8], width: u32, height: u32) -> Result<crate::model::Drawing> {
        Self::write_drawing(root, path, source, width, height, Some(previous))
    }

    /// Copied blocks need independent source files now that edits overwrite them.
    pub fn copy_drawings(root: &Path, blocks: &mut [crate::model::Block]) -> Result<()> {
        use crate::model::BlockKind;
        for block in blocks {
            match &mut block.kind {
                BlockKind::Drawing(drawing) => {
                    let bytes = read_drawing_asset(root, &drawing.source, EXPORT_FILE_LIMIT)?;
                    drawing.source = Self::save_drawing(root, &bytes, drawing.width, drawing.height)?.source;
                }
                BlockKind::Table(table) => for cell in table.rows.iter_mut().flatten() { Self::copy_drawings(root, &mut cell.blocks)?; },
                _ => {}
            }
            Self::copy_drawings(root, &mut block.children)?;
        }
        Ok(())
    }

    fn write_drawing(root: &Path, path: &str, source: &[u8], width: u32, height: u32, previous: Option<&crate::git::Pointer>) -> Result<crate::model::Drawing> {
        let (dir, name) = path.split_once('/').context("invalid drawing path")?;
        anyhow::ensure!(dir == DRAWING && !name.contains('/') && name.ends_with(".excalidraw"), "invalid drawing path");
        let _lock = Self::lock(root)?;
        let file = crate::git::safe_path(root, path)?;
        if let Some(previous) = previous {
            let current = read_bounded(&file, previous.size).context("drawing changed on disk")?;
            anyhow::ensure!(crate::git::Pointer::of(&current) == *previous, "drawing changed on disk");
        } else {
            std::fs::create_dir_all(file.parent().unwrap())?;
        }
        write_atomic(&file, source)?;
        Ok(crate::model::Drawing { source: path.to_string(), preview: String::new(), width, height })
    }

    pub fn open() -> Result<Self> { Self::open_library_at(Self::root_dir()) }

    /// Open a library for the app, preparing local Git after the layout is ready.
    pub fn open_library_at(root: PathBuf) -> Result<Self> {
        let mut store = Self::open_at(root)?;
        if let Err(e) = store.initialize_library_git() { store.warnings.push(format!("无法初始化文档库 Git：{e:#}")); }
        Ok(store)
    }

    pub(crate) fn lock(root: &Path) -> Result<WriteLock> {
        let path = crate::git::safe_path(root, ".quiet/.lock")?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(path)?;
        file.lock()?;
        Ok(WriteLock(Some(file)))
    }

    /// Hold the library lock until its current layout is ready to be read.
    fn prepare_layout_locked(root: &Path) -> Result<WriteLock> {
        let lock = Self::lock(root)?;
        let manifest = crate::git::safe_path(root, FILE)?;
        if let Err(e) = manifest.symlink_metadata() {
            if e.kind() != std::io::ErrorKind::NotFound { return Err(e.into()); }
            let existing = [NOTES, IMAGE, FILES, DRAWING].iter().any(|p| root.join(p).symlink_metadata().is_ok());
            anyhow::ensure!(!existing && !Self::has_legacy_metadata(root), "缺少文档库清单 quiet.json");
            save_json(&manifest, &Manifest::default())?;
        }
        Ok(lock)
    }

    pub(crate) fn prepare_layout(root: &Path) -> Result<()> {
        let _lock = Self::prepare_layout_locked(root)?;
        Self::read_manifest(root)?;
        Ok(())
    }

    pub fn open_at(root: PathBuf) -> Result<Self> {
        let lock = Self::prepare_layout_locked(&root)?;
        let mut store = Store { file_changes: vec![], root, index: HashMap::new(), trash: HashMap::new(), manifest: Manifest::default(), warnings: vec![], loaded: Default::default(), loaded_titles: Default::default(), copied: Default::default(), parsed: HashMap::new(), parsed_disk: Default::default(), batch: Default::default() };
        store.reload_virtual()?;
        store.load_parsed();
        drop(lock);
        if let Ok(entries) = std::fs::read_dir(store.recovery_dir()) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_none_or(|x| x != "xml") { continue; }
                let restored = (|| -> Result<()> {
                    let mut doc = docxml::parse(&std::fs::read_to_string(&path)?)?;
                    doc.title = crate::model::RichText::plain(format!("{} (recovered copy)", doc.title.text));
                    store.create_from(&doc)?;
                    std::fs::remove_file(&path)?;
                    Ok(())
                })();
                store.warnings.push(match restored {
                    Ok(()) => "已恢复上次退出时未保存的文档副本".into(),
                    Err(e) => format!("恢复副本仍保留在 {}：{e}", path.display()),
                });
            }
        }
        Ok(store)
    }

    fn history_base(&self) -> PathBuf { self.internal() }
    fn internal(&self) -> PathBuf { self.root.join(INTERNAL) }
    fn local_file(&self) -> PathBuf { local_file_for(&self.root) }

    /// A relative path in the library as a path on disk.
    fn abs(&self, rel: &str) -> PathBuf {
        let mut p = self.root.clone();
        for part in rel.split('/').filter(|s| !s.is_empty()) { p.push(part); }
        p
    }

    fn read_local(&self) -> Local { reread(&self.local_file()).unwrap_or_default() }

    fn local_state(&self) -> Local {
        let mut local = self.read_local();
        local.opened = self.index.iter().filter(|(_, m)| m.opened > 0).map(|(k, m)| (k.clone(), m.opened)).collect();
        local.files = self.index.iter().filter(|(_, m)| m.local()).map(|(k, m)| (k.clone(), m.clone())).collect();
        local
    }

    fn save_local(&self, local: &Local) -> Result<()> {
        let path = self.local_file();
        let bytes = serde_json::to_vec_pretty(local)?;
        if std::fs::read(&path).is_ok_and(|b| b == bytes) { return Ok(()); }
        std::fs::create_dir_all(path.parent().unwrap())?;
        write_atomic(&path, &bytes)
    }

    fn recovery_dir(&self) -> PathBuf {
        dirs::data_local_dir().unwrap_or_else(std::env::temp_dir)
            .join("quiet-md/recovery").join(library_key(&self.root))
    }

    pub fn recover_later(&self, doc: &Document) -> Result<()> {
        let dir = self.recovery_dir();
        std::fs::create_dir_all(&dir)?;
        write_atomic(&dir.join(format!("{}.xml", uuid::Uuid::new_v4())), docxml::serialize(doc).as_bytes())
    }

    fn path(&self, id: &str) -> PathBuf {
        match self.index.get(id) {
            Some(Meta { path: Some(p), .. }) => PathBuf::from(p),
            Some(m) if !m.rel.is_empty() => self.abs(&m.rel),
            _ => self.internal().join("missing").join(id),
        }
    }

    /// The file a document is kept in.
    pub fn file_path(&self, id: &str) -> Option<PathBuf> {
        self.index.get(id).filter(|m| m.folder.is_none()).map(|_| self.path(id))
    }

    /// The library root or the shared body directory for a virtual folder.
    pub fn folder_dir(&self, id: Option<&str>) -> Option<PathBuf> {
        match id {
            None => Some(self.root.clone()),
            Some(id) => self.index.get(id).filter(|m| m.folder.is_some()).map(|_| self.root.join(NOTES)),
        }
    }

    pub fn is_markdown(&self, id: &str) -> bool { self.index.get(id).or_else(|| self.trash.get(id).map(|t| &t.meta)).is_some_and(|m| m.markdown) }

    /// The folder of a 本机文件, which its images are relative to.
    fn base(&self, id: &str) -> Option<PathBuf> {
        self.index.get(id)?.path.as_ref().and_then(|p| std::path::Path::new(p).parent()).map(PathBuf::from)
    }

    fn markdown_base(&self, id: &str) -> Option<PathBuf> {
        self.base(id).or_else(|| (!self.index.get(id).is_some_and(Meta::local)).then(|| self.root.join(crate::manifest::NOTES)))
    }

    /// A Markdown document's title: its file name.
    fn markdown_title(&self, id: &str) -> String {
        let Some(m) = self.index.get(id).or_else(|| self.trash.get(id).map(|t| &t.meta)) else { return String::new() };
        match &m.path {
            Some(p) => std::path::Path::new(p).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            None => m.name.clone().unwrap_or_default(),
        }
    }

    /// The folder a document is (or was, in the 回收站) in, which its links are relative to.
    fn doc_dir(&self, id: &str) -> String {
        let rel = self.index.get(id).map(|m| m.rel.as_str()).or_else(|| self.trash.get(id).map(|t| t.meta.rel.as_str())).unwrap_or("");
        parent_rel(rel).to_string()
    }

    /// Mentions as written in a file in `dir` (relative paths) as the ids of
    /// the documents they point at; mentions of nothing in the library stay
    /// as they are, and so do old `doc-id` ones.
    fn links_to_ids(&self, dir: &str, doc: &mut Document) {
        let mut by_rel: Option<HashMap<&str, &str>> = None;
        layout::each_inline(doc, &mut |i| {
            let Some(l) = i.doc_link().cloned() else { return };
            let Some(href) = docxml::doc_link_href(&i.xml) else { return };
            let by_rel = by_rel.get_or_insert_with(|| self.index.iter().filter(|(_, m)| !m.local() && m.folder.is_none()).map(|(k, m)| (m.rel.as_str(), k.as_str())).collect());
            if let Some(id) = layout::resolve(dir, &href).and_then(|rel| by_rel.get(rel.as_str()).copied()) {
                *i = docxml::to_inline(crate::model::InlineKind::DocLink(crate::model::InlineDocLink { id: id.to_string(), title: l.title }));
            }
        });
    }

    /// The other way: mentions of documents in the library (or in the 回收站,
    /// by where they were) as paths relative to `dir`.
    fn links_to_paths(&self, dir: &str, doc: &mut Document) {
        layout::each_inline(doc, &mut |i| {
            let Some(l) = i.doc_link().cloned() else { return };
            if docxml::doc_link_href(&i.xml).is_some() { return; }
            let target = self.index.get(&l.id).or_else(|| self.trash.get(&l.id).map(|t| &t.meta)).filter(|m| !m.local() && m.folder.is_none() && !m.rel.is_empty());
            if let Some(m) = target { i.xml = docxml::doc_link_href_xml(&layout::relative(dir, &m.rel), &l.title); }
        });
    }

    /// `text` (a file's contents) as a document.
    fn parse_doc(&self, id: &str, text: &str) -> Result<Document> {
        if !self.is_markdown(id) {
            let mut doc = docxml::parse(text)?;
            if text.contains("href=") { self.links_to_ids(&self.doc_dir(id), &mut doc); }
            return Ok(doc);
        }
        let base = self.markdown_base(id);
        let mut doc = crate::markdown::parse_file(text, &self.markdown_title(id), base.as_deref());
        if !self.index.get(id).is_some_and(Meta::local) {
            let mut paths = HashMap::new();
            for src in doc.asset_sources() {
                if let Ok(rel) = Path::new(&src).strip_prefix(&self.root)
                    && let Some(rel) = layout::resolve("", &rel.to_string_lossy()) {
                    paths.insert(src, self.root.join(rel).to_string_lossy().into_owned());
                }
            }
            git_store::rewrite_asset_sources(&mut doc.blocks, &paths, &paths);
        }
        // Images a Markdown file carries inline (#133) become assets, so the
        // page shows them and an export writes them like any other picture.
        crate::data_images::absorb(&mut doc.blocks, &self.root);
        Ok(doc)
    }

    /// `doc` as `id`'s file contents; a Markdown file keeps the front matter it has on disk.
    fn serialize_doc(&self, id: &str, doc: &Document, disk: Option<&str>) -> String { self.serialize_with_copies(id, doc, disk).0 }

    /// `serialize_doc`, and the library files a 本机文件 needs copied beside
    /// it: (source, path relative to its folder).
    fn serialize_with_copies(&self, id: &str, doc: &Document, disk: Option<&str>) -> (String, Vec<(PathBuf, String)>) {
        if !self.is_markdown(id) {
            if !doc.doc_links().is_empty() {
                let mut doc = doc.clone();
                self.links_to_paths(&self.doc_dir(id), &mut doc);
                return (docxml::serialize(&doc), vec![]);
            }
            return (docxml::serialize(doc), vec![]);
        }
        let front = disk.map(|d| crate::markdown::front_matter(d.trim_start_matches('\u{feff}')).0).unwrap_or_default();
        let base = self.markdown_base(id);
        let virtual_note = !self.index.get(id).is_some_and(Meta::local);
        let name = self.base(id).is_some().then(|| self.markdown_title(id));
        let copied = self.copied.borrow();
        let out = crate::markdown::write_file_with(doc, front, &crate::markdown::Options {
            library_root: Some(&self.root), base: base.as_deref(), relative_from: virtual_note.then_some(crate::manifest::NOTES), name: name.as_deref(), copied: copied.get(id), original: disk, ..Default::default()
        });
        let bom = if disk.is_some_and(|d| d.starts_with('\u{feff}')) { "\u{feff}" } else { "" };
        (format!("{bom}{}", out.text), out.files)
    }

    /// Take the library's write lock and adopt what other copies wrote since
    /// this one last looked. Every change is written through, so memory holds
    /// nothing the disk does not. A lock failure must never allow an unguarded write.
    fn begin_write(&mut self) -> Result<WriteLock> {
        // Inside a batch the lock is held and memory is what the disk will be.
        if self.batch.lock.is_some() { return Ok(WriteLock(None)); }
        let lock = Self::lock(&self.root)?;
        self.reload_virtual()?;
        Ok(lock)
    }

    fn save_index(&self) -> Result<()> {
        if self.batch.lock.is_some() { self.batch.index.set(true); return Ok(()); }
        self.save_virtual()
    }

    /// Run `f` as one write: the library is locked once and the index written
    /// once at the end, so importing or deleting a hundred documents costs
    /// about what one does. Files are written as `f` goes and the index
    /// last, so a failure or crash midway never leaves an entry without its
    /// file: the files already written are picked up by the next `list`
    /// of the Notes directory.
    /// Batches nest; the outermost one writes.
    pub fn batch<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> Result<T> {
        self.begin_batch()?;
        let out = f(self);
        self.end_batch()?;
        Ok(out)
    }

    /// `batch` for callers that cannot hand over a closure (the window's
    /// store is shared): every `begin_batch` that succeeded needs its `end_batch`.
    pub fn begin_batch(&mut self) -> Result<()> {
        if self.batch.depth == 0 {
            let lock = self.begin_write()?;
            self.batch.lock = Some(lock);
        }
        self.batch.depth += 1;
        Ok(())
    }

    pub fn end_batch(&mut self) -> Result<()> {
        anyhow::ensure!(self.batch.depth > 0, "没有进行中的批量写入");
        self.batch.depth -= 1;
        if self.batch.depth > 0 { return Ok(()); }
        let lock = self.batch.lock.take();
        // The documents' folders reach the disk before the index naming them.
        let mut r: Result<()> = Ok(());
        #[cfg(unix)]
        for dir in self.batch.dirs.take() {
            if let Err(e) = std::fs::File::open(&dir).and_then(|f| f.sync_all()) { r = Err(e.into()); }
        }
        #[cfg(not(unix))]
        self.batch.dirs.take();
        if self.batch.index.take() && r.is_ok() { r = self.save_index(); }
        drop(lock);
        r
    }

    /// Write a document's file; in a batch its folder is synced at the end.
    fn write_file(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        if self.batch.lock.is_none() { return write_atomic(path, bytes); }
        write_atomic_then(path, bytes, false, || Ok(()))?;
        if let Some(dir) = path.parent() {
            let mut dirs = self.batch.dirs.borrow_mut();
            if !dirs.iter().any(|d| d == dir) { dirs.push(dir.to_path_buf()); }
        }
        Ok(())
    }

    fn uuid_rel(&self, dir: &str, id: &str, ext: &str) -> Result<String> {
        anyhow::ensure!(uuid::Uuid::parse_str(id).is_ok(), "新文档必须使用 UUID");
        let rel = join_rel(dir, &format!("{id}.{ext}"));
        match self.abs(&rel).symlink_metadata() {
            Ok(_) => anyhow::bail!("已存在相同 UUID 的文件，未覆盖：{rel}"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(rel),
            Err(e) => Err(e.into()),
        }
    }

    /// Update a library title or rename a local Markdown file beside itself.
    fn follow_title(&mut self, id: &str, title: &str) -> Result<()> {
        let Some(m) = self.index.get_mut(id) else { return Ok(()) };
        if let Some(name) = &mut m.folder {
            *name = title.to_string();
            return Ok(());
        }
        if let Some(old) = m.path.clone().map(PathBuf::from) {
            let name = title.replace(['/', '\\', ':'], "_");
            let name = name.trim().trim_start_matches('.');
            if name.is_empty() { return Ok(()); }
            let ext = old.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "md".into());
            let new = old.with_file_name(format!("{name}.{ext}"));
            if new == old || new.try_exists()? { return Ok(()); }
            std::fs::rename(&old, &new)?;
            self.index.get_mut(id).unwrap().path = Some(new.to_string_lossy().into_owned());
            return Ok(());
        }
        m.name = Some(title.to_string());
        Ok(())
    }

    /// List the manifest and adopt new bodies placed in Notes/.
    pub fn list(&mut self) -> Vec<Entry> {
        let lock = match self.begin_write() {
            Ok(lock) => Some(lock),
            Err(e) => { self.warnings.push(format!("无法锁定文档库：{e}")); None }
        };
        self.list_virtual(lock.is_some())
    }

    /// What `list` parsed last launch, for the files still where they were.
    fn load_parsed(&mut self) {
        let Some(c) = std::fs::read(parsed_cache_path(&self.root)).ok().and_then(|b| serde_json::from_slice::<ParsedCache>(&b).ok()) else { return };
        if c.version != parsed_cache_version() { return }
        let mut docs = c.docs;
        let dir = parsed_cache_path(&self.root).with_extension("d");
        for file in std::fs::read_dir(&dir).into_iter().flatten().flatten().filter(|e| e.path().extension().is_some_and(|e| e == "json")) {
            if let Some(delta) = std::fs::read(file.path()).ok().and_then(|b| serde_json::from_slice::<ParsedDelta>(&b).ok())
                && delta.generation == c.generation {
                docs.insert(delta.path, delta.doc);
                self.parsed_disk.deltas.insert(file.path());
            }
        }
        self.parsed_disk.generation = Some(c.generation);
        let ids: Vec<String> = self.index.iter().filter(|(_, m)| m.folder.is_none()).map(|(k, _)| k.clone()).collect();
        for id in ids {
            let Some(d) = docs.remove(&self.path(&id)) else { continue };
            let reminders = d.reminders.into_iter().map(|(expire, notify, should_notify, whole_day)| crate::model::InlineReminder { expire, notify, should_notify, whole_day }).collect();
            let links = d.links.into_iter().map(|(target, block, line)| crate::model::DocLink { target, block, line }).collect();
            self.parsed.insert(id, (d.modified, d.len, Parsed { title: d.title, icon: d.icon, body: d.body.into(), reminders, links }));
        }
        self.parsed_disk.stamps = self.parsed.iter().map(|(id, (time, len, _))| (id.clone(), (*time, *len))).collect();
    }

    fn cached_doc(time: std::time::SystemTime, len: u64, p: &Parsed) -> CachedDoc {
        CachedDoc { modified: time, len, title: p.title.clone(), icon: p.icon.clone(), body: p.body.to_string(),
            reminders: p.reminders.iter().map(|r| (r.expire, r.notify, r.should_notify, r.whole_day)).collect(),
            links: p.links.iter().map(|l| (l.target.clone(), l.block, l.line.clone())).collect() }
    }

    fn save_parsed(&mut self) -> Result<()> {
        let path = parsed_cache_path(&self.root);
        let dir = path.with_extension("d");
        let compact = self.parsed_disk.deltas.len() >= 64 || self.parsed_disk.stamps.keys().any(|id| !self.parsed.contains_key(id));
        match self.parsed_disk.generation.clone().filter(|_| !compact) {
        None => {
            let generation = uuid::Uuid::new_v4().to_string();
            let docs = self.parsed.iter().map(|(id, (time, len, p))| (self.path(id), Self::cached_doc(*time, *len, p))).collect();
            if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
            write_atomic(&path, &serde_json::to_vec(&ParsedCache { version: parsed_cache_version(), generation: generation.clone(), docs })?)?;
            // Publish a new generation before cleanup: old deltas cannot undo
            // a compacted snapshot, even if cleanup is interrupted.
            self.parsed_disk.generation = Some(generation);
            self.parsed_disk.deltas.clear();
            let _ = std::fs::remove_dir_all(&dir);
        }
        Some(generation) => {
            for (id, (time, len, p)) in &self.parsed {
                if self.parsed_disk.stamps.get(id) == Some(&(*time, *len)) { continue }
                std::fs::create_dir_all(&dir)?;
                let doc_path = self.path(id);
                let delta = dir.join(format!("{}.json", library_key(&doc_path)));
                write_atomic(&delta, &serde_json::to_vec(&ParsedDelta { generation: generation.clone(), path: doc_path, doc: Self::cached_doc(*time, *len, p) })?)?;
                self.parsed_disk.deltas.insert(delta);
            }
        }
        }
        self.parsed_disk.stamps = self.parsed.iter().map(|(id, (time, len, _))| (id.clone(), (*time, *len))).collect();
        Ok(())
    }

    fn read_doc_text(&self, id: &str) -> Result<String> {
        let p = self.path(id);
        match std::fs::read_to_string(&p) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && layout::placeholder(&p).is_some_and(|ph| ph.exists()) => {
                if let Some(ph) = layout::placeholder(&p) { layout::download(&ph); }
                anyhow::bail!("这篇文档还在从 iCloud 下载，请稍后再打开")
            }
            r => Ok(r?),
        }
    }

    pub fn load(&self, id: &str) -> Result<Document> {
        let mut doc = self.parse_doc(id, &self.read_doc_text(id)?)?;
        self.manifest_title(id, &mut doc);
        Ok(doc)
    }

    /// Only opening an editor establishes a new save baseline; previews must
    /// not hide external changes from an already open editor.
    pub fn load_for_edit(&self, id: &str, shared: bool) -> Result<Document> {
        let xml = self.read_doc_text(id)?;
        let mut doc = self.parse_doc(id, &xml)?;
        self.manifest_title(id, &mut doc);
        if !shared { self.set_edit_baseline(id, xml.into(), &doc.title.text); }
        Ok(doc)
    }

    pub fn edit_baseline(&self, id: &str) -> Option<Arc<String>> { self.loaded.borrow().get(id).cloned() }

    fn set_edit_baseline(&self, id: &str, text: Arc<String>, title: &str) {
        self.loaded.borrow_mut().insert(id.to_string(), text);
        self.loaded_titles.borrow_mut().insert(id.to_string(), title.to_string());
    }

    /// A background tab keeps its undo stack while the exact body on disk matches.
    pub fn resume_for_edit(&self, id: &str, baseline: Option<&Arc<String>>, title: &str) -> Result<(Option<Document>, String)> {
        let xml = self.read_doc_text(id)?;
        if let Some(baseline) = baseline.filter(|text| text.as_str() == xml) {
            let title = self.index.get(id).filter(|m| !m.local()).and_then(|m| m.name.as_deref()).unwrap_or(title);
            self.set_edit_baseline(id, baseline.clone(), title);
            return Ok((None, title.to_string()));
        }
        let mut doc = self.parse_doc(id, &xml)?;
        self.manifest_title(id, &mut doc);
        let title = doc.title.text.clone();
        self.set_edit_baseline(id, xml.into(), &title);
        Ok((Some(doc), title))
    }

    /// Release a closed editor's conflict-detection baseline, after its last window saved.
    pub fn forget_loaded(&self, id: &str) { self.loaded.borrow_mut().remove(id); self.loaded_titles.borrow_mut().remove(id); }

    pub fn save(&mut self, id: &str, doc: &Document) -> Result<Meta> {
        let _lock = self.begin_write()?;
        self.save_now(id, doc)
    }

    /// `save` for callers that already hold the write lock. A new id becomes a
    /// DocxXML file in Notes/, named by its UUID.
    fn save_now(&mut self, id: &str, doc: &Document) -> Result<Meta> { self.save_in(id, doc, None) }

    /// `save_now`, a new id going into folder `parent`.
    fn save_in(&mut self, id: &str, doc: &Document, parent: Option<&str>) -> Result<Meta> {
        anyhow::ensure!(!self.trash.contains_key(id), "回收站文档恢复后才能编辑");
        let new = !self.index.contains_key(id);
        if new {
            let t = now();
            let rel = self.uuid_rel(NOTES, id, "xml")?;
            self.index.insert(id.to_string(), Meta { created: t, modified: t, rel, name: Some(doc.title.text.clone()), parent: parent.map(str::to_string), ..Default::default() });
        }
        let r = self.write_doc(id, doc);
        if r.is_err() && new { self.index.remove(id); }
        r
    }

    fn write_doc(&mut self, id: &str, doc: &Document) -> Result<Meta> {
        if let Some(meta) = self.index.get(id).filter(|m| !m.local())
            && let Some(expected) = self.loaded_titles.borrow().get(id) {
            let current = meta.name.as_deref().unwrap_or_default();
            anyhow::ensure!(current == expected || current == doc.title.text.trim(), "文档标题已被另一个实例修改，请重新载入磁盘版本");
        }
        let disk = std::fs::read_to_string(self.path(id)).ok();
        let (xml, copies) = self.serialize_with_copies(id, doc, disk.as_deref());
        if let Some(previous) = self.loaded.borrow().get(id) {
            let disk = disk.as_deref().ok_or_else(|| anyhow::anyhow!("无法读取文档"))?;
            let message = if self.index.get(id).is_some_and(Meta::local) { "文件已在别处被修改，未覆盖磁盘内容。请另存副本或重新打开文件" } else { "文档已被另一个实例修改，未覆盖磁盘内容。请另存副本或重新载入磁盘版本" };
            anyhow::ensure!(disk == previous.as_str() || disk == xml, "{message}");
        }
        // A Markdown file whose content did not change is left alone: its
        // formatting is the user's.
        let same = disk.as_deref().is_some_and(|d| d == xml || self.is_markdown(id) && self.parse_doc(id, d).is_ok_and(|was| self.serialize_doc(id, &was, Some(d)) == xml));
        if !same {
            // Images and attachments first, so the file never links to a missing one.
            if let Some(base) = self.base(id) {
                for (src, rel) in &copies {
                    let to = base.join(rel);
                    if !to.exists() {
                        if let Some(dir) = to.parent() { std::fs::create_dir_all(dir)?; }
                        std::fs::copy(src, &to)?;
                    }
                    self.copied.borrow_mut().entry(id.to_string()).or_default().insert(src.clone(), rel.clone());
                }
            }
            self.write_file(&self.path(id), xml.as_bytes())?;
            // The previous text is already in `disk`: a snapshot costs no
            // extra read. History is best-effort — it never fails the save.
            if let Some(previous) = disk.as_deref()
                && previous != xml.as_str()
                && !self.index.get(id).is_some_and(Meta::local)
            {
                let ext = if self.is_markdown(id) { "md" } else { "xml" };
                let limit = self.settings().history_limit();
                if let Err(e) = crate::versions::snapshot(&self.history_base(), id, ext, previous, crate::time::now_ms(), limit, false) {
                    self.warnings.push(format!("保存版本历史失败：{e}"));
                }
            }
        }
        self.loaded.borrow_mut().insert(id.to_string(), if same { disk.unwrap_or(xml) } else { xml }.into());
        let title = doc.title.text.trim().to_string();
        if same && title == self.markdown_title(id) {
            return self.index.get(id).cloned().ok_or_else(|| anyhow::anyhow!("文档不存在"));
        }
        let changed = if self.is_markdown(id) { title != self.markdown_title(id) } else { true };
        if changed && !(self.is_markdown(id) && title.is_empty()) && let Err(e) = self.follow_title(id, &title) {
            self.warnings.push(format!("未能按标题重命名文件：{e}"));
        }
        let t = now();
        let m = self.index.get_mut(id).ok_or_else(|| anyhow::anyhow!("文档不存在"))?;
        m.modified = t;
        let m = m.clone();
        self.save_index()?;
        if let Some(title) = &m.name { self.loaded_titles.borrow_mut().insert(id.to_string(), title.clone()); }
        Ok(m)
    }

    pub fn create(&mut self) -> Result<(String, Document)> {
        let doc = Document::default();
        let id = self.create_from(&doc)?;
        Ok((id, doc))
    }

    /// Save `doc` as a new document (创建副本, import).
    pub fn create_from(&mut self, doc: &Document) -> Result<String> { self.create_from_in(doc, None) }

    /// `create_from` into folder `parent` (`None`: the top), as its last
    /// child. Its body lives in Notes/.
    pub fn create_from_in(&mut self, doc: &Document, parent: Option<&str>) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let lock = self.begin_write()?;
        self.validate_folder(None, parent)?;
        self.save_in(&id, doc, parent)?;
        self.record_file_change(&id, None);
        // `move_in_tree` takes the lock itself: outside a batch, holding it here would wait on ourselves.
        drop(lock);
        self.place_new(&id, parent)?;
        Ok(id)
    }

    /// A new Markdown document in the library (导入, 创建副本): `front`
    /// matter, then `doc`; a UUID file name and a separate display title.
    pub fn create_markdown(&mut self, doc: &Document, front: &str) -> Result<String> { self.create_markdown_in(doc, front, None) }

    /// `create_markdown` into folder `parent`, like `create_from_in`.
    pub fn create_markdown_in(&mut self, doc: &Document, front: &str, parent: Option<&str>) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let lock = self.begin_write()?;
        self.validate_folder(None, parent)?;
        let t = now();
        let rel = self.uuid_rel(NOTES, &id, "md")?;
        self.index.insert(id.clone(), Meta { created: t, modified: t, rel, name: Some(doc.title.text.clone()), parent: parent.map(str::to_string), markdown: true, ..Default::default() });
        let text = format!("{front}{}", self.serialize_doc(&id, doc, None));
        if let Err(e) = self.write_file(&self.path(&id), text.as_bytes()).and_then(|_| self.save_index()) { self.index.remove(&id); return Err(e); }
        self.record_file_change(&id, None);
        drop(lock);
        self.place_new(&id, parent)?;
        Ok(id)
    }

    /// Place a new document at the end of its virtual folder.
    fn place_new(&mut self, id: &str, parent: Option<&str>) -> Result<()> {
        if parent.is_some() { self.move_in_tree(id, parent, None)?; }
        Ok(())
    }

    /// A Markdown document's front matter, as on disk.
    pub fn front_matter_of(&self, id: &str) -> String {
        if !self.is_markdown(id) { return String::new(); }
        let text = std::fs::read_to_string(self.path(id)).unwrap_or_default();
        crate::markdown::front_matter(text.trim_start_matches('\u{feff}')).0.to_string()
    }

    /// Where a 本机文件 is.
    pub fn local_path(&self, id: &str) -> Option<PathBuf> { self.index.get(id)?.path.as_ref().map(PathBuf::from) }

    /// 打开 a Markdown file where it is (本机文件): its document id, the one
    /// it already has when it was opened before. A file inside the library
    /// is the library's document.
    pub fn open_local(&mut self, path: &std::path::Path) -> Result<String> {
        let path = path.canonicalize()?;
        // Windows canonical paths are `\\?\C:\…`: keep the ordinary form, which
        // the Explorer and the title show.
        let path = match path.to_str().and_then(|p| p.strip_prefix(r"\\?\")).filter(|p| !p.starts_with("UNC\\")) { Some(p) => PathBuf::from(p), None => path };
        anyhow::ensure!(path.is_file(), "文件不存在");
        let _lock = self.begin_write()?;
        if let Some(rel) = self.root.canonicalize().ok().and_then(|r| path.strip_prefix(r).ok().map(|p| p.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")))
            && let Some((id, _)) = self.index.iter().find(|(_, m)| !m.local() && m.rel == rel) {
            let id = id.clone();
            self.index.get_mut(&id).unwrap().opened = now();
            self.save_local(&self.local_state())?;
            return Ok(id);
        }
        let key = path.to_string_lossy().into_owned();
        let t = now();
        if let Some((id, m)) = self.index.iter_mut().find(|(_, m)| m.path.as_deref() == Some(key.as_str())) {
            m.opened = t;
            let id = id.clone();
            self.save_index()?;
            return Ok(id);
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.index.insert(id.clone(), Meta { created: t, modified: t, opened: t, markdown: true, path: Some(key), ..Default::default() });
        if let Err(e) = self.save_index() { self.index.remove(&id); return Err(e); }
        Ok(id)
    }

    /// Size of the document on disk, plus the local images and files it references.
    pub fn size_of(&self, id: &str, doc: &Document) -> u64 {
        let mut n = std::fs::metadata(self.path(id)).map(|m| m.len()).unwrap_or(0);
        for src in doc.asset_sources() {
            if let Some(p) = image_path(&src) {
                n += std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            }
        }
        n
    }

    /// Move a document or folder to the 回收站; `root` is the page whose
    /// deletion takes it along (`None`: deleted itself). A folder takes what
    /// is in it along (deleting those afterwards does nothing more).
    pub fn delete(&mut self, id: &str, root: Option<&str>) -> Result<()> {
        let _lock = self.begin_write()?;
        // 本机文件: only taken off the list; the file is the user's.
        if self.index.get(id).is_some_and(Meta::local) {
            self.index.remove(id);
            self.loaded.borrow_mut().remove(id);
            return self.save_index();
        }
        if !self.index.contains_key(id) && self.trash.contains_key(id) { return Ok(()); }
        anyhow::ensure!(self.index.contains_key(id), "文档或文件夹不存在");
        let ride = root.unwrap_or(id).to_string();
        let mut items = vec![];
        if self.index[id].folder.is_some() {
            for key in self.index.keys().filter(|k| *k != id) {
                let mut parent = self.index[key].parent.as_deref();
                while let Some(p) = parent {
                    if p == id { items.push((key.clone(), Some(ride.clone()))); break; }
                    parent = self.index.get(p).and_then(|m| m.parent.as_deref());
                }
            }
        }
        items.push((id.to_string(), root.map(str::to_string)));
        self.trash_virtual(&items, true)
    }

    fn trash_path(&self, id: &str) -> PathBuf {
        let meta = self.trash.get(id).map(|t| &t.meta).or_else(|| self.index.get(id));
        self.abs(&format!("{NOTES}/{id}.{}", if meta.is_some_and(|m| m.markdown) { "md" } else { "xml" }))
    }

    /// `id` and the pages deleted along with it.
    fn trash_group(&self, id: &str) -> Vec<String> {
        let mut out = vec![id.to_string()];
        out.extend(self.trash.iter().filter(|(k, t)| t.root.as_deref() == Some(id) && k.as_str() != id).map(|(k, _)| k.clone()));
        out
    }

    /// The 回收站, newest first: one row per root deletion.
    pub fn list_trash(&mut self) -> Vec<TrashEntry> {
        self.list_trash_in(None)
    }

    pub fn trash_meta(&self, id: &str) -> Option<Meta> { self.trash.get(id).map(|t| t.meta.clone()) }

    /// Read without restoring the document or establishing an editable baseline.
    pub fn load_trash(&self, id: &str) -> Result<Document> {
        let meta = self.trash.get(id).context("回收站中没有这篇文档")?;
        anyhow::ensure!(meta.meta.folder.is_none(), "文件夹不能作为文档打开");
        let mut doc = self.parse_doc(id, &std::fs::read_to_string(self.trash_path(id))?)?;
        self.manifest_title(id, &mut doc);
        Ok(doc)
    }

    /// Root deletions, or the immediate children of a deleted folder.
    pub fn list_trash_in(&mut self, parent: Option<&str>) -> Vec<TrashEntry> {
        let _lock = match self.begin_write() {
            Ok(lock) => Some(lock),
            Err(e) => { self.warnings.push(format!("无法锁定文档库：{e}")); None }
        };
        let mut out: Vec<TrashEntry> = self
            .trash
            .iter()
            .filter(|(_, t)| match parent {
                Some(p) => t.meta.parent.as_deref() == Some(p),
                None => t.root.as_ref().is_none_or(|r| !self.trash.contains_key(r)),
            })
            .map(|(id, t)| {
                let doc = if t.meta.markdown || t.meta.folder.is_some() { Document::default() } else { std::fs::read_to_string(self.trash_path(id)).ok().and_then(|x| docxml::parse(&x).ok()).unwrap_or_default() };
                TrashEntry {
                    id: id.clone(),
                    meta: t.meta.clone(),
                    title: t.meta.folder.clone().or_else(|| t.meta.name.clone()).unwrap_or_else(|| doc.title.text.clone()),
                    icon: doc.icon.clone(),
                    deleted: t.deleted,
                    parent: t.meta.parent.clone(),
                }
            })
            .collect();
        out.sort_by(|a, b| b.deleted.cmp(&a.deleted).then_with(|| a.id.cmp(&b.id)));
        out
    }

    /// 恢复: put a deleted page (and the pages that went with it) back. True
    /// when its parent page is gone, so it landed at the top of 我的文档库.
    pub fn restore(&mut self, id: &str) -> Result<bool> {
        let _lock = self.begin_write()?;
        anyhow::ensure!(self.trash.contains_key(id), "回收站中没有这篇文档");
        let mut group = self.trash_group(id);
        // Folders before what is in them.
        let depth = |k: &String| { let mut seen = HashSet::new(); let mut p = self.trash.get(k).and_then(|t| t.meta.parent.clone()); while let Some(q) = p { if !seen.insert(q.clone()) { break } p = self.trash.get(&q).and_then(|t| t.meta.parent.clone()); } seen.len() };
        group.sort_by_key(|k| (depth(k), k.clone()));
        let parent = self.trash.get(id).and_then(|t| t.meta.parent.clone());
        let moved = parent.as_ref().is_some_and(|p| !self.index_has(p));
        for k in &group {
            let Some(t) = self.trash.get(k) else { continue };
            let parent = if k == id && moved { None } else { t.meta.parent.clone().filter(|p| self.index.get(p).is_some_and(|m| m.folder.is_some())) };
            self.untrash_virtual(k, parent, None).with_context(|| format!("恢复文档 {k} 失败，已恢复的文档保留在文档库"))?;
        }
        Ok(moved)
    }

    fn index_has(&self, id: &str) -> bool {
        self.index.get(id).is_some_and(|m| m.folder.is_some() || self.path(id).exists())
    }

    /// Copy a DocxXML file into the library under a fresh id.
    pub fn import(&mut self, path: &std::path::Path) -> Result<String> { self.import_into(path, None) }

    /// `import` into folder `parent`.
    pub fn import_into(&mut self, path: &std::path::Path, parent: Option<&str>) -> Result<String> {
        let xml = std::fs::read_to_string(path)?;
        self.create_from_in(&docxml::parse(&xml)?, parent)
    }

    pub fn set_width(&mut self, id: &str, width: PageWidth) -> Result<()> {
        let _lock = self.begin_write()?;
        if let Some(m) = self.index.get_mut(id) {
            m.width = width;
        }
        self.save_index()
    }

    pub fn update_meta(&mut self, id: &str, f: impl FnOnce(&mut Meta)) -> Result<()> {
        let _lock = self.begin_write()?;
        if let Some(m) = self.index.get_mut(id) {
            // Where it is follows from the file, not from here.
            let (rel, folder, parent, markdown, name, path) = (m.rel.clone(), m.folder.clone(), m.parent.clone(), m.markdown, m.name.clone(), m.path.clone());
            f(m);
            (m.rel, m.folder, m.parent, m.markdown, m.name, m.path) = (rel, folder, parent, markdown, name, path);
        }
        self.save_index()
    }

    pub fn set_catalogue(&mut self, id: &str, pinned: bool) -> Result<()> {
        let _lock = self.begin_write()?;
        if let Some(m) = self.index.get_mut(id) {
            m.catalogue_closed = !pinned;
        }
        self.save_index()
    }

    /// 版本历史: snapshots of one document, newest first.
    pub fn versions(&self, id: &str) -> Vec<crate::versions::Version> { crate::versions::list(&self.history_base(), id, self.trash.contains_key(id)) }

    pub fn version_text(&self, id: &str, name: &str) -> Result<String> {
        if let Some(commit) = git_store::git_commit_name(name) { return crate::git::LibraryRepo::open(&self.root)?.revision(commit, id); }
        crate::versions::read(&self.history_base(), id, name, self.trash.contains_key(id))
    }

    pub fn delete_version(&self, id: &str, name: &str) -> Result<()> { crate::versions::remove(&self.history_base(), id, name, self.trash.contains_key(id)) }

    /// Parse a snapshot into a document, for 另存副本.
    pub fn version_document(&self, id: &str, name: &str) -> Result<Document> {
        if let Some(commit) = git_store::git_commit_name(name) { return self.git_version_document(id, commit, true) }
        let text = self.version_text(id, name)?;
        self.parse_doc(id, &text)
    }

    pub fn version_copy_document(&self, id: &str, name: &str) -> Result<Document> {
        if let Some(commit) = git_store::git_commit_name(name) { return self.git_version_document(id, commit, false) }
        let mut doc = self.version_document(id, name)?;
        Self::copy_drawings(&self.root, &mut doc.blocks)?;
        Ok(doc)
    }

    /// Diff a snapshot against a baseline. `None` is the document on disk
    /// now; `Some(other)` compares two snapshots. Lines are ordered by time,
    /// so `-` is always "only in the earlier version". The unchanged lines
    /// are kept, so the same list doubles as a read-only preview.
    pub fn version_diff(&self, id: &str, name: &str, baseline: Option<&str>) -> Result<Vec<crate::diff::Line>> {
        let a = self.version_text(id, name)?;
        let a_ms = git_store::version_ms(name);
        let (b, b_ms) = match baseline {
            Some(other) => (self.version_text(id, other)?, git_store::version_ms(other)),
            None => (std::fs::read_to_string(if self.trash.contains_key(id) { self.trash_path(id) } else { self.path(id) })?, u64::MAX),
        };
        let (old, new) = if a_ms < b_ms { (a.as_str(), b.as_str()) } else { (b.as_str(), a.as_str()) };
        Ok(crate::diff::diff(old, new, self.is_markdown(id)))
    }

    /// 命名版本: set or clear a snapshot's name.
    pub fn set_version_label(&self, id: &str, name: &str, label: Option<&str>) -> Result<()> {
        crate::versions::set_label(&self.history_base(), id, name, label, self.trash.contains_key(id))
    }

    /// Restore never loses the present: the current on-disk text becomes a
    /// snapshot first, then the chosen version is parsed for `save` to write.
    pub fn restore_version(&self, id: &str, name: &str) -> Result<Document> {
        anyhow::ensure!(!self.trash.contains_key(id), "回收站文档恢复后才能编辑");
        let text = self.version_text(id, name)?;
        let current = std::fs::read_to_string(self.path(id)).unwrap_or_default();
        let ext = if self.is_markdown(id) { "md" } else { "xml" };
        crate::versions::snapshot(&self.history_base(), id, ext, &current, crate::time::now_ms(), self.settings().history_limit(), true)?;
        if let Some(commit) = git_store::git_commit_name(name) { return self.git_version_document(id, commit, false) }
        self.parse_doc(id, &text)
    }

    /// 手动保存版本: bypass the interval, still capped by the retention limit.
    /// `false` means the text matched the newest snapshot, so nothing was
    /// written — the caller can say so instead of silently doing nothing.
    pub fn save_version_now(&self, id: &str) -> Result<bool> {
        anyhow::ensure!(!self.trash.contains_key(id), "回收站文档恢复后才能编辑");
        let text = std::fs::read_to_string(self.path(id))?;
        let ext = if self.is_markdown(id) { "md" } else { "xml" };
        crate::versions::snapshot(&self.history_base(), id, ext, &text, crate::time::now_ms(), self.settings().history_limit(), true)
    }

    pub fn settings(&self) -> Settings {
        let mut s: Settings = reread(&self.internal().join("settings.json")).unwrap_or_default();
        let local = self.read_local();
        s.window_size = local.window_size;
        s.quick_note_size = local.quick_note_size;
        s
    }

    pub fn save_settings(&self, s: &Settings) -> Result<()> {
        let mut local = self.read_local();
        (local.window_size, local.quick_note_size) = (s.window_size, s.quick_note_size);
        self.save_local(&local)?;
        let shared = Settings { window_size: None, quick_note_size: None, ..s.clone() };
        std::fs::create_dir_all(self.internal())?;
        write_atomic(&self.internal().join("settings.json"), &serde_json::to_vec_pretty(&shared)?)
    }

    /// 阅读位置: the list offset this document was left at.
    pub fn scroll_top(&self, id: &str) -> Option<(usize, f32)> {
        let v = self.settings().scroll_top.get(id).copied()?;
        Some((v[0] as usize, v[1]))
    }

    /// Remember — or, with `None`, forget — where a document was left.
    pub fn remember_scroll(&self, id: &str, top: Option<(usize, f32)>) -> Result<()> {
        let mut s = self.settings();
        match top {
            Some((ix, y)) => {
                let v = [ix as f32, y];
                if s.scroll_top.get(id) == Some(&v) { return Ok(()) }
                s.scroll_top.insert(id.to_string(), v);
            }
            None => {
                if s.scroll_top.remove(id).is_none() { return Ok(()) }
            }
        }
        self.save_settings(&s)
    }

    /// Update a library title or rename a local Markdown file.
    pub fn rename(&mut self, id: &str, title: &str) -> Result<Meta> {
        let _lock = self.begin_write()?;
        let before = self.file_state(id);
        if self.index.get(id).is_some_and(|m| m.folder.is_some() || m.markdown || !m.local()) {
            // A Markdown file is left as it is: only its name changes.
            self.follow_title(id, title)?;
            let m = self.index.get_mut(id).unwrap();
            m.modified = now();
            let meta = m.clone();
            self.save_index()?;
            if !meta.local() { self.record_file_change(id, before); }
            return Ok(meta);
        }
        let mut doc = self.load(id)?;
        doc.title = crate::model::RichText::plain(title);
        let meta = self.save_now(id, &doc)?;
        self.record_file_change(id, before);
        Ok(meta)
    }

    /// Note that `id` was just opened (this computer only).
    pub fn touch(&mut self, id: &str) -> Option<Meta> {
        let _lock = self.begin_write().ok()?;
        let m = self.index.get_mut(id)?;
        m.opened = now();
        let m = m.clone();
        let _ = self.save_local(&self.local_state());
        Some(m)
    }

    /// 移动到: put `id` in folder `parent` (`None` = top level of 我的文档库), as its last child.
    pub fn set_parent(&mut self, id: &str, parent: Option<&str>) -> Result<Meta> {
        self.move_in_tree(id, parent, None)?;
        Ok(self.index[id].clone())
    }

    /// 收藏 / 取消收藏; returns the new 收藏时间 (0: not starred).
    pub fn set_starred(&mut self, id: &str, on: bool) -> Result<u64> {
        let _lock = self.begin_write()?;
        let t = if on { now() } else { 0 };
        if let Some(m) = self.index.get_mut(id) {
            m.starred = t;
        }
        self.save_index()?;
        Ok(t)
    }

    pub fn set_pinned(&mut self, id: &str, pinned: bool) -> Result<()> {
        let _lock = self.begin_write()?;
        if let Some(m) = self.index.get_mut(id) {
            m.pinned = pinned;
        }
        self.save_index()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Lib(PathBuf);

    impl Lib {
        fn new(name: &str) -> Self {
            let d = std::env::temp_dir().join(format!("fdl-test-{name}-{}", uuid::Uuid::new_v4()));
            Lib(d)
        }
        fn open(&self) -> Store { Store::open_at(self.0.clone()).unwrap() }
    }

    impl Drop for Lib {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
            let _ = std::fs::remove_file(super::parsed_cache_path(&self.0));
            let _ = std::fs::remove_dir_all(super::parsed_cache_path(&self.0).with_extension("d"));
        }
    }

    fn doc(title: &str) -> Document {
        Document { title: crate::model::RichText::plain(title), ..Default::default() }
    }

    fn doc_with(title: &str, body: &str) -> Document {
        let mut d = doc(title);
        d.blocks.push(crate::model::Block::text(body));
        d
    }

    #[test]
    fn manual_version_reports_creation_at_retention_limit() {
        let lib = Lib::new("version-limit");
        let mut s = lib.open();
        let mut settings = s.settings(); settings.history_limit = Some(1); s.save_settings(&settings).unwrap();
        let id = s.create_from(&doc_with("note", "first")).unwrap();
        assert!(s.save_version_now(&id).unwrap());
        s.save(&id, &doc_with("note", "second")).unwrap();
        assert!(s.save_version_now(&id).unwrap());
        let versions = s.versions(&id);
        assert_eq!(versions.len(), 1);
        assert!(s.version_text(&id, &versions[0].name).unwrap().contains("second"));
        assert!(!s.save_version_now(&id).unwrap());
    }

    #[test]
    fn completed_tree_worker_keeps_open_editors_conflict_baselines() {
        let lib = Lib::new("receive-tree");
        let mut s = lib.open();
        let doc = doc_with("current", "original");
        let id = s.create_from(&doc).unwrap();
        s.load_for_edit(&id, false).unwrap();
        let mut writer = s.tree_writer();
        let imported = writer.create_from(&doc_with("imported", "new body")).unwrap();
        writer.list();
        s.receive_tree(writer);
        assert!(s.load(&imported).unwrap().plain_text().contains("new body"));
        std::fs::write(s.file_path(&id).unwrap(), docxml::serialize(&doc_with("current", "external"))).unwrap();
        assert!(s.save(&id, &doc_with("current", "local edit")).is_err());
    }

    #[test]
    fn saves_keep_previous_versions_and_follow_the_trash() {
        let lib = Lib::new("versions");
        let mut s = lib.open();
        let id = s.create_from(&doc_with("first", "alpha")).unwrap();
        s.load_for_edit(&id, false).unwrap();
        s.save(&id, &doc_with("second", "beta")).unwrap();
        assert_eq!(s.versions(&id).len(), 1);
        // Snapshots stay in `.quiet/`: never a folder in the sidebar or the Finder,
        // and still found after the retitle renamed the file.
        assert!(lib.0.join(".quiet").join("history").join(&id).is_dir());
        assert_eq!(s.list().iter().map(|e| e.id.clone()).collect::<Vec<_>>(), std::slice::from_ref(&id));
        let name = s.versions(&id)[0].name.clone();
        assert_eq!(s.version_text(&id, &name).unwrap(), docxml::serialize(&doc_with("first", "alpha")));
        // Saving identical content again must not create another file.
        s.save(&id, &doc_with("second", "beta")).unwrap();
        assert_eq!(s.versions(&id).len(), 1);
        // The diff compares the reader's text, not the XML tags.
        let d = s.version_diff(&id, &name, None).unwrap();
        assert!(d.iter().any(|l| l.text == "alpha" && l.kind == crate::diff::Kind::Removed));
        assert!(d.iter().any(|l| l.text == "beta" && l.kind == crate::diff::Kind::Added));
        // Two snapshots compare directly, always in time order: the older
        // text is removed, the newer one added, whichever was picked first.
        s.save_version_now(&id).unwrap();
        let v = s.versions(&id);
        assert_eq!(v.len(), 2);
        // The same text again reports "nothing written" instead of piling up.
        assert!(!s.save_version_now(&id).unwrap());
        assert_eq!(s.versions(&id).len(), 2);
        let d = s.version_diff(&id, &v[0].name, Some(&v[1].name)).unwrap();
        assert!(d.iter().any(|l| l.text == "alpha" && l.kind == crate::diff::Kind::Removed));
        assert!(d.iter().any(|l| l.text == "beta" && l.kind == crate::diff::Kind::Added));
        let d = s.version_diff(&id, &v[1].name, Some(&v[0].name)).unwrap();
        assert!(d.iter().any(|l| l.text == "alpha" && l.kind == crate::diff::Kind::Removed));
        assert!(d.iter().any(|l| l.text == "beta" && l.kind == crate::diff::Kind::Added));
        // Restoring first leaves a snapshot of the present.
        let restored = s.restore_version(&id, &name).unwrap();
        assert_eq!(restored.title.text, "first");
        s.save(&id, &restored).unwrap();
        assert!(s.versions(&id).len() >= 2);
        s.delete(&id, None).unwrap();
        assert!(!s.versions(&id).is_empty());
        assert!(lib.0.join(".quiet").join("history-trash").join(&id).is_dir());
        s.restore(&id).unwrap();
        assert!(!s.versions(&id).is_empty());
        s.delete(&id, None).unwrap();
        assert_eq!(s.list_trash()[0].id, id);
        s.restore(&id).unwrap();
        assert!(!s.versions(&id).is_empty());
    }

    #[test]
    fn file_history_ignores_external_creations() {
        let lib = Lib::new("file-journal");
        let mut local = lib.open();
        let mut other = lib.open();
        let journal = local.track_file_changes();
        let external = other.create_from(&doc("external")).unwrap();
        let own = local.create_folder("own", None).unwrap();
        local.list();
        let changes = journal.take();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].id, own);
        local.apply_file_history(&changes, false).unwrap();
        assert!(local.list().iter().any(|e| e.id == external));
        assert_eq!(local.load(&external).unwrap().title.text, "external");
        local.apply_file_history(&changes, true).unwrap();
        assert!(local.list().iter().any(|e| e.id == own));
    }

    #[test]
    fn tree_history_uses_fresh_disk_positions() {
        let lib = Lib::new("tree-journal");
        let mut local = lib.open();
        let a = local.create_folder("a", None).unwrap();
        let b = local.create_folder("b", None).unwrap();
        let page = local.create_from(&doc("page")).unwrap();
        let mut other = lib.open();
        other.set_parent(&page, Some(&a)).unwrap();
        let journal = local.track_file_changes();
        local.set_parent(&page, Some(&b)).unwrap();
        let changes = journal.take();
        local.apply_file_history(&changes, false).unwrap();
        assert_eq!(local.index[&page].parent.as_ref(), Some(&a));
    }

    #[test]
    fn new_documents_keep_uuid_names_without_git() {
        let lib = Lib::new("uuid-without-git");
        let mut s = lib.open();
        assert!(!s.git_enabled());
        let collision = uuid::Uuid::new_v4().to_string();
        let collision_path = lib.0.join(format!("Notes/{collision}.xml"));
        std::fs::write(&collision_path, "keep existing bytes").unwrap();
        assert!(s.save(&collision, &doc("不能覆盖")).is_err());
        assert_eq!(std::fs::read_to_string(&collision_path).unwrap(), "keep existing bytes");
        std::fs::remove_file(collision_path).unwrap();
        let (xml, mut d) = s.create().unwrap();
        let xml_path = s.file_path(&xml).unwrap();
        assert_eq!(xml_path.file_name().unwrap().to_str(), Some(format!("{xml}.xml").as_str()));
        d.title = crate::model::RichText::plain("第一次输入标题");
        s.save(&xml, &d).unwrap();
        s.rename(&xml, "标题再次改变").unwrap();
        assert_eq!(s.file_path(&xml).unwrap(), xml_path);
        let other = s.create_from(&doc("标题再次改变")).unwrap();
        assert_ne!(s.file_path(&other).unwrap().file_name(), xml_path.file_name());
        let folder = s.create_folder("Folder", None).unwrap();
        let md = s.create_markdown_in(&doc("Markdown 标题"), "---\nkeep: true\n---\n", Some(&folder)).unwrap();
        let md_path = s.file_path(&md).unwrap();
        assert_eq!(md_path.file_name().unwrap().to_str(), Some(format!("{md}.md").as_str()));
        s.rename(&md, "另一个显示标题").unwrap();
        assert_eq!(s.file_path(&md).unwrap(), md_path);
        let manifest = Manifest::parse(&std::fs::read(lib.0.join(FILE)).unwrap()).unwrap();
        let note = manifest.notes.iter().find(|n| n.id == md).unwrap();
        assert_eq!(note.title, "另一个显示标题");
        assert_eq!(note.folder.as_deref(), Some(folder.as_str()));
        assert!(!lib.0.join(".git").exists());
        drop(s);
        let mut s = lib.open();
        assert_eq!(s.load(&xml).unwrap().title.text, "标题再次改变");
        assert_eq!(s.load(&md).unwrap().title.text, "另一个显示标题");
        assert!(std::fs::read_to_string(&md_path).unwrap().starts_with("---\nkeep: true\n---\n"));
        s.rename(&folder, "Moved folder").unwrap();
        assert_eq!(s.list().iter().find(|e| e.id == md).unwrap().title, "另一个显示标题");
        s.set_parent(&md, None).unwrap();
        assert_eq!(s.file_path(&md).unwrap().file_name(), md_path.file_name());
        s.delete(&md, None).unwrap(); s.restore(&md).unwrap();
        assert_eq!(s.file_path(&md).unwrap().file_name(), md_path.file_name());
        assert_eq!(s.load(&md).unwrap().title.text, "另一个显示标题");
        assert!(!s.git_enabled());
    }

    #[test]
    fn markdown_documents_stay_markdown() {
        let lib = Lib::new("markdown");
        let mut s = lib.open();
        let mut d = crate::markdown::parse_file("Hello *there*\n", "Note", None);
        let id = s.create_markdown(&d, "---\na: 1\n---\n").unwrap();
        let file = lib.0.join(format!("Notes/{id}.md"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "---\na: 1\n---\nHello *there*\n");
        let e = s.list().into_iter().find(|e| e.id == id).unwrap();
        assert!(e.meta.markdown);
        assert_eq!(e.title, "Note");
        // Saving rewrites the body only; the front matter stays.
        let mut loaded = s.load_for_edit(&id, false).unwrap();
        loaded.blocks.push(crate::model::Block::text("more"));
        s.save(&id, &loaded).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "---\na: 1\n---\nHello *there*\n\nmore\n");
        // A title change updates metadata, keeping the UUID file and body.
        d = s.load(&id).unwrap();
        d.title = crate::model::RichText::plain("Renamed");
        s.save(&id, &d).unwrap();
        assert_eq!(s.load(&id).unwrap().title.text, "Renamed");
        assert_eq!(s.file_path(&id).unwrap(), file);
        assert!(!std::fs::read_to_string(&file).unwrap().contains("Renamed"));
        // Through the 回收站 and back as Markdown.
        s.delete(&id, None).unwrap();
        assert!(file.is_file());
        assert_eq!(s.list_trash()[0].title, "Renamed");
        s.restore(&id).unwrap();
        assert!(file.is_file());
        assert_eq!(s.load(&id).unwrap().title.text, "Renamed");
    }

    /// Into a folder outside a batch (a 本机文件 dragged onto a sidebar folder):
    /// placing it takes the write lock again, which must not wait on itself.
    #[test]
    fn creating_into_a_folder_outside_a_batch() {
        let lib = Lib::new("create-into");
        let mut s = lib.open();
        let folder = s.create_folder("Notes", None).unwrap();
        let (_, doc) = s.create().unwrap();
        let md = s.create_markdown_in(&doc, "", Some(&folder)).unwrap();
        let xml = s.create_from_in(&doc, Some(&folder)).unwrap();
        for id in [md, xml] { assert_eq!(s.list().into_iter().find(|e| e.id == id).unwrap().meta.parent.as_deref(), Some(folder.as_str())); }
    }

    #[test]
    fn a_hand_written_markdown_file_joins_the_library() {
        let lib = Lib::new("hand-written");
        let mut s = lib.open();
        let (id, doc) = s.create().unwrap();
        let before = Store::shape_at(&lib.0);
        s.save(&id, &doc).unwrap();
        assert_eq!(before, Store::shape_at(&lib.0), "saving is not a change of shape");
        std::fs::write(lib.0.join("Notes/model.md"), "# Written by a model\n").unwrap();
        assert_ne!(before, Store::shape_at(&lib.0), "a new file is");
        let e = s.list().into_iter().find(|e| e.title == "model").unwrap();
        assert!(e.meta.markdown);
        assert!(e.body.contains("Written by a model"), "{:?}", e.body);
        assert_eq!(e.meta.rel, format!("Notes/{}.md", e.id));
    }

    #[test]
    fn local_files_are_edited_in_place_and_never_trashed() {
        let lib = Lib::new("local");
        let dir = lib.0.join("outside");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("todo.md");
        std::fs::write(&file, "* star bullet\n\n![x](pics/x.png)\n").unwrap();
        let mut s = lib.open();
        let id = s.open_local(&file).unwrap();
        assert_eq!(s.open_local(&file).unwrap(), id);
        let e = s.list().into_iter().find(|e| e.id == id).unwrap();
        assert!(e.meta.local());
        assert_eq!(e.title, "todo");
        let mut d = s.load_for_edit(&id, false).unwrap();
        // Saving untouched content leaves the user's formatting alone.
        s.save(&id, &d).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "* star bullet\n\n![x](pics/x.png)\n");
        d.blocks.push(crate::model::Block::text("added"));
        s.save(&id, &d).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "* star bullet\n\n![x](pics/x.png)\n\nadded\n");
        // Changed elsewhere: not overwritten.
        std::fs::write(&file, "edited elsewhere\n").unwrap();
        d.blocks.push(crate::model::Block::text("again"));
        assert!(s.save(&id, &d).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "edited elsewhere\n");
        // Renaming renames the file beside it.
        s.rename(&id, "done").unwrap();
        let renamed = dir.canonicalize().unwrap().join("done.md");
        assert!(renamed.is_file() && !file.exists());
        // Removing it from the list keeps the file.
        s.delete(&id, None).unwrap();
        assert!(renamed.is_file());
        assert!(s.list().iter().all(|e| e.id != id));
        assert!(s.list_trash().is_empty());
    }

    /// Images from the library used in a 本机文件 are copied beside it:
    /// pasted ones named after the file, picked ones keeping their name;
    /// autosaves reuse the copies.
    #[test]
    fn local_files_get_their_images_beside_them() {
        let lib = Lib::new("local-images");
        let dir = lib.0.join("outside");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Plan.md");
        std::fs::write(&file, "text\n").unwrap();
        let mut s = lib.open();
        std::fs::create_dir_all(lib.0.join(IMAGE)).unwrap();
        let pasted = lib.0.join("Images/3e5c9916-7c1f-4f7d-af25-74dd4a8a241e.png");
        let picked = lib.0.join("Images/1a2b3c4d-截图说明.png");
        std::fs::write(&pasted, b"p").unwrap();
        std::fs::write(&picked, b"q").unwrap();
        // Something already there keeps its name.
        std::fs::create_dir_all(dir.join("Plan.assets")).unwrap();
        std::fs::write(dir.join("Plan.assets/Plan-1.png"), b"old").unwrap();
        let id = s.open_local(&file).unwrap();
        let mut d = s.load_for_edit(&id, false).unwrap();
        let image = |src: &std::path::Path| crate::model::Block::new(crate::model::BlockKind::Image { src_kind: "src".into(), src: src.to_string_lossy().into(), width: None, height: None, caption: None, crop: None }, Default::default());
        d.blocks.push(image(&pasted));
        d.blocks.push(image(&picked));
        s.save(&id, &d).unwrap();
        let text = std::fs::read_to_string(dir.canonicalize().unwrap().join("Plan.md")).unwrap();
        assert_eq!(text, "text\n\n![](Plan.assets/Plan-2.png)\n\n![](Plan.assets/截图说明.png)\n");
        assert_eq!(std::fs::read(dir.join("Plan.assets/Plan-2.png")).unwrap(), b"p");
        assert_eq!(std::fs::read(dir.join("Plan.assets/Plan-1.png")).unwrap(), b"old");
        // The next autosave (the editor still points at the library copy) makes no new copy.
        d.blocks.push(crate::model::Block::text("more"));
        s.save(&id, &d).unwrap();
        assert_eq!(std::fs::read_dir(dir.join("Plan.assets")).unwrap().count(), 3);
        assert!(std::fs::read_to_string(dir.join("Plan.md")).unwrap().contains("Plan.assets/Plan-2.png"));
        // Reopened, the links point at the copies.
        let again = s.load(&id).unwrap();
        assert!(again.blocks.iter().any(|b| matches!(&b.kind, crate::model::BlockKind::Image { src, .. } if src.ends_with("Plan.assets/截图说明.png"))));
    }

    #[test]
    /// Undoing a move checks what it moves, not unrelated stale entries.
    fn markdown_moves_undo() {
        let lib = Lib::new("md-undo");
        let mut s = lib.open();
        let folder = s.create_folder("f", None).unwrap();
        let id = s.create_markdown(&crate::markdown::parse_file("x\n", "Note", None), "").unwrap();
        // A page whose parent is gone (in memory only: on disk, where a file
        // is decides its folder).
        let stale = s.create_from(&doc("old")).unwrap();
        s.list();
        s.index.get_mut(&stale).unwrap().parent = Some(uuid::Uuid::new_v4().to_string());
        let journal = s.track_file_changes();
        s.move_in_tree(&id, Some(&folder), None).unwrap();
        let changes = journal.take();
        s.apply_file_history(&changes, false).unwrap();
        assert!(s.index[&id].parent.is_none());
        s.apply_file_history(&changes, true).unwrap();
        assert_eq!(s.index[&id].parent.as_deref(), Some(folder.as_str()));
    }

    #[test]
    fn missing_move_anchor_is_rejected_without_panicking() {
        let lib = Lib::new("missing-move-anchor");
        let mut s = lib.open();
        let id = s.create_from(&doc("moved")).unwrap();
        let anchor = uuid::Uuid::new_v4().to_string();
        let err = s.move_in_tree(&id, None, Some(&anchor)).unwrap_err();
        assert!(err.to_string().contains("目标位置已改变，请重试"));
    }

    #[test]
    fn failed_lock_never_writes() {
        let lib = Lib::new("lock-failure");
        let mut s = lib.open();
        let id = s.create_from(&doc("original")).unwrap();
        std::fs::remove_file(lib.0.join(".quiet/.lock")).unwrap();
        std::fs::create_dir(lib.0.join(".quiet/.lock")).unwrap();
        assert!(s.save(&id, &doc("lost")).is_err());
        assert_eq!(s.load(&id).unwrap().title.text, "original");
        assert!(s.create_from(&doc("new")).is_err());
        assert!(s.delete(&id, None).is_err());
        assert_eq!(s.list().len(), 1);
    }

    #[test]
    fn recovery_preserves_both_versions_and_is_consumed_once() {
        let lib = Lib::new("recovery");
        let mut s = lib.open();
        let id = s.create_from(&doc("disk")).unwrap();
        s.recover_later(&doc("unsaved")).unwrap();
        let recovery = s.recovery_dir();
        drop(s);
        let mut s = lib.open();
        assert_eq!(s.load(&id).unwrap().title.text, "disk");
        assert!(s.list().iter().any(|e| e.title == "unsaved (recovered copy)"));
        assert_eq!(s.list().len(), 2);
        drop(s);
        assert_eq!(lib.open().list().len(), 2);
        std::fs::remove_dir_all(recovery).unwrap();
    }

    #[test]
    fn atomic_write_cleans_failed_temporary_files() {
        let lib = Lib::new("atomic-failure");
        std::fs::create_dir_all(lib.0.join("target")).unwrap();
        assert!(write_atomic(&lib.0.join("target"), b"new").is_err());
        assert_eq!(std::fs::read_dir(&lib.0).unwrap().count(), 1);
    }

    #[test]
    fn exports_stay_in_library_and_reads_are_bounded() {
        let lib = Lib::new("export");
        let outside = Lib::new("outside");
        std::fs::create_dir_all(lib.0.join(FILES)).unwrap();
        std::fs::create_dir_all(&outside.0).unwrap();
        std::fs::write(lib.0.join("Files/good"), b"1234").unwrap();
        std::fs::write(outside.0.join("secret"), b"secret").unwrap();
        assert!(export_path_in(&lib.0, "Files/good").is_some());
        assert!(export_path_in(&lib.0, &outside.0.join("secret").to_string_lossy()).is_none());
        let traversal = format!("../{}/secret", outside.0.file_name().unwrap().to_string_lossy());
        assert!(export_path_in(&lib.0, &traversal).is_none());
        #[cfg(unix)] {
            std::os::unix::fs::symlink(&outside.0, lib.0.join("Files/link")).unwrap();
            assert!(export_path_in(&lib.0, "Files/link/secret").is_none());
        }
        assert!(read_bounded(&lib.0.join("Files/good"), 3).is_err());
        assert_eq!(read_bounded(&lib.0.join("Files/good"), 4).unwrap(), b"1234");
    }

    #[test]
    fn save_list_and_reopen() {
        let lib = Lib::new("basic");
        let mut s = lib.open();
        assert!(s.warnings.is_empty());
        let id = s.create_from(&doc("一")).unwrap();
        s.set_pinned(&id, true).unwrap();
        let mut s = lib.open();
        let list = s.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, "一");
        assert!(list[0].meta.pinned);
        assert!(!lib.0.join(".quiet/format.json").exists());
        let names: Vec<String> = std::fs::read_dir(&lib.0).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!({ let mut n = names; n.sort(); n }, [".quiet", "Notes", "quiet.json"]);
    }

    #[test]
    fn unchanged_saves_preserve_body_metadata_and_history() {
        let lib = Lib::new("unchanged-save");
        let mut s = lib.open();
        let id = s.create_from(&doc("Original")).unwrap();
        let d = s.load_for_edit(&id, false).unwrap();
        let body = std::fs::metadata(s.path(&id)).unwrap().modified().unwrap();
        let index = std::fs::read(lib.0.join(FILE)).unwrap();
        let modified = s.index[&id].modified;
        for _ in 0..8 { assert_eq!(s.save(&id, &d).unwrap().modified, modified); }
        assert_eq!(std::fs::metadata(s.path(&id)).unwrap().modified().unwrap(), body);
        assert_eq!(std::fs::read(lib.0.join(FILE)).unwrap(), index);
        assert!(s.versions(&id).is_empty());
    }

    #[test]
    fn parsed_cache_updates_one_document_and_ignores_old_generations() {
        let lib = Lib::new("cache-delta");
        let mut s = lib.open();
        let a = s.create_from(&doc("First")).unwrap();
        s.create_from(&doc("Second")).unwrap();
        s.list();
        let path = parsed_cache_path(&lib.0);
        let base = std::fs::read(&path).unwrap();
        let mut stale = None;
        for i in 0..5 {
            s.save(&a, &doc(&format!("Changed {i}"))).unwrap();
            s.list();
            assert_eq!(std::fs::read(&path).unwrap(), base);
            assert_eq!(s.parsed_disk.deltas.len(), 1);
            let delta = s.parsed_disk.deltas.iter().next().unwrap();
            stale = Some((delta.clone(), std::fs::read(delta).unwrap()));
        }
        let opened = lib.open();
        assert_eq!(opened.parsed[&a].2.title, "Changed 4");
        s.parsed_disk.generation = None;
        s.save_parsed().unwrap();
        let (delta, bytes) = stale.unwrap();
        std::fs::create_dir_all(delta.parent().unwrap()).unwrap();
        std::fs::write(delta, bytes).unwrap();
        let mut cache: ParsedCache = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        cache.docs.get_mut(&s.path(&a)).unwrap().title = "Compacted".into();
        std::fs::write(path, serde_json::to_vec(&cache).unwrap()).unwrap();
        assert_eq!(lib.open().parsed[&a].2.title, "Compacted", "a leftover delta must not override a new snapshot");
    }

    #[test]
    fn list_cache_survives_a_relaunch() {
        let lib = Lib::new("listcache-disk");
        let mut s = lib.open();
        let a = s.create_from(&doc("一")).unwrap();
        s.create_from(&doc("二")).unwrap();
        assert_eq!(s.list().len(), 2);
        drop(s);
        // A new launch starts with what the last one parsed (#170) …
        let mut s = lib.open();
        assert_eq!(s.parsed.len(), 2);
        s.parsed.get_mut(&a).unwrap().2.body = "缓存".into();
        assert!(s.list().iter().any(|e| &*e.body == "缓存"));
        drop(s);
        // … and drops a cache written by another version.
        let path = parsed_cache_path(&lib.0);
        let mut c: ParsedCache = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        c.version = "0.0.0-0".into();
        std::fs::write(&path, serde_json::to_vec(&c).unwrap()).unwrap();
        assert!(lib.open().parsed.is_empty());
    }

    #[test]
    fn parsed_cache_compacts_at_limit_and_after_deletion() {
        let lib = Lib::new("cache-compact");
        let mut s = lib.open();
        let ids: Vec<_> = (0..65).map(|i| s.create_from(&doc(&format!("Note {i}"))).unwrap()).collect();
        s.list();
        let first = s.parsed_disk.generation.clone();
        for (i, id) in ids.iter().enumerate() {
            s.save(id, &doc(&format!("Changed {i}"))).unwrap();
            s.list();
            if i < 64 { assert_eq!(s.parsed_disk.generation, first); }
        }
        assert_ne!(s.parsed_disk.generation, first);
        assert!(s.parsed_disk.deltas.is_empty());
        let mut reopened = lib.open();
        assert_eq!(reopened.list().len(), 65);
        for (i, id) in ids.iter().enumerate() { assert_eq!(reopened.parsed[id].2.title, format!("Changed {i}")); }
        s.delete(&ids[0], None).unwrap();
        assert_eq!(s.list().len(), 64);
        assert!(!lib.open().parsed.contains_key(&ids[0]));
    }

    #[test]
    fn list_rereads_only_changed_files() {
        let lib = Lib::new("listcache");
        let mut s = lib.open();
        let a = s.create_from(&doc("一")).unwrap();
        let b = s.create_from(&doc("二")).unwrap();
        assert_eq!(s.list().len(), 2);
        // Another app edits b. a is unchanged on disk, so list keeps what it
        // parsed before (marked here to tell it from a re-read).
        std::fs::write(s.path(&b), docxml::serialize(&doc_with("二", "二改"))).unwrap();
        let ap = s.path(&a);
        let stamp = std::fs::metadata(&ap).unwrap().modified().unwrap();
        s.parsed.get_mut(&a).unwrap().2.body = "缓存".into();
        assert_eq!(std::fs::metadata(&ap).unwrap().modified().unwrap(), stamp);
        let bodies = |s: &mut Store| { let mut t: Vec<_> = s.list().into_iter().map(|e| e.body.trim().to_string()).collect(); t.sort(); t };
        assert_eq!(bodies(&mut s), ["二改", "缓存"]);
        // Saving through the store, or deleting the file, is seen too.
        s.save(&a, &doc_with("一", "一改")).unwrap();
        assert_eq!(bodies(&mut s), ["一改", "二改"]);
        s.delete(&b, None).unwrap();
        assert_eq!(bodies(&mut s), ["一改"]);
        assert_eq!(s.parsed.len(), 1);
    }

    #[test]
    fn reopening_after_releasing_baseline_still_detects_conflicts() {
        let lib = Lib::new("released-baseline");
        let mut s = lib.open();
        let id = s.create_from(&doc("initial")).unwrap();
        s.load_for_edit(&id, false).unwrap();
        s.forget_loaded(&id);
        std::fs::write(s.path(&id), docxml::serialize(&doc_with("initial", "external before reopen"))).unwrap();
        let reopened = s.load_for_edit(&id, false).unwrap();
        assert!(reopened.plain_text().contains("external before reopen"));
        s.save(&id, &reopened).unwrap();
        let external = docxml::serialize(&doc_with("initial", "external after reopen"));
        std::fs::write(s.path(&id), &external).unwrap();
        assert!(s.save(&id, &doc("local edit")).is_err());
        assert_eq!(std::fs::read_to_string(s.path(&id)).unwrap(), external);
    }

    #[test]
    fn newer_manifest_version_is_refused() {
        let lib = Lib::new("format");
        drop(lib.open());
        let manifest = Manifest { version: crate::manifest::VERSION + 1, ..Default::default() };
        std::fs::write(lib.0.join(FILE), serde_json::to_vec(&manifest).unwrap()).unwrap();
        let err = Store::open_at(lib.0.clone()).err().expect("a newer library must not open");
        assert!(format!("{err:#}").contains("不支持的文档库清单版本"));
    }

    #[test]
    fn trash_round_trip() {
        let lib = Lib::new("trash");
        let mut s = lib.open();
        // Only folders hold pages: a folder goes to the trash with what is in it.
        let a = s.create_folder("父", None).unwrap();
        let b = s.create_from(&doc("子")).unwrap();
        s.set_parent(&b, Some(&a)).unwrap();
        s.delete(&b, Some(&a)).unwrap();
        s.delete(&a, None).unwrap();
        assert!(s.list().is_empty());
        let t = s.list_trash();
        assert_eq!(t.len(), 1, "the page rides along with its folder");
        assert_eq!(t[0].title, "父");
        let mut s = lib.open();
        assert!(!s.restore(&a).unwrap());
        let list = s.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list.iter().find(|e| e.id == b).unwrap().meta.parent.as_deref(), Some(a.as_str()));
        assert!(s.list_trash().is_empty());
    }

    /// Two copies of the app on one library: neither writes back a stale
    /// index over what the other just changed.
    #[test]
    fn two_instances_keep_each_others_entries() {
        let lib = Lib::new("two");
        let mut a = lib.open();
        let mut b = lib.open();
        let one = a.create_from(&doc("一")).unwrap();
        let two = b.create_from(&doc("二")).unwrap();
        a.set_pinned(&one, true).unwrap();
        b.set_starred(&two, true).unwrap();
        let mut c = lib.open();
        let list = c.list();
        assert_eq!(list.len(), 2);
        assert!(list.iter().find(|e| e.id == one).unwrap().meta.pinned, "b kept a's entry");
        assert!(list.iter().find(|e| e.id == two).unwrap().meta.starred > 0, "a kept b's entry");
    }

    #[test]
    fn an_instance_does_not_undo_another_ones_delete() {
        let lib = Lib::new("two-del");
        let mut a = lib.open();
        let mut b = lib.open();
        let id = a.create_from(&doc("一")).unwrap();
        let two = a.create_from(&doc("二")).unwrap();
        a.delete(&id, None).unwrap();
        // `b` still has `id` in memory and changes it: it must not come back.
        b.set_pinned(&id, true).unwrap();
        let mut c = lib.open();
        assert_eq!(c.list().iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), vec![two.as_str()]);
        assert_eq!(c.list_trash().len(), 1);
    }

    #[test]
    fn stale_temporary_path_does_not_block_save() {
        let lib = Lib::new("savefail");
        let mut s = lib.open();
        let id = s.create_from(&doc("旧")).unwrap();
        std::fs::create_dir(s.path(&id).with_extension("xml.tmp")).unwrap();
        s.save(&id, &doc("新")).unwrap();
        assert_eq!(s.load(&id).unwrap().title.text, "新");
    }

    /// `doc` with a paragraph mentioning each of `targets`.
    fn linking(title: &str, targets: &[(&str, &str)]) -> Document {
        let mut d = doc(title);
        for (id, t) in targets {
            let mut text = crate::model::RichText::plain("见 ");
            text.push("\u{fffc}", crate::model::Marks { inline: Some(docxml::to_inline(crate::model::InlineKind::DocLink(crate::model::InlineDocLink { id: id.to_string(), title: t.to_string() }))), ..Default::default() });
            let mut b = crate::model::Block::text("");
            b.text = text;
            d.blocks.push(b);
        }
        d
    }

    /// The names of the folders `id` is in, outermost first.
    fn folders_of(list: &[Entry], id: &str) -> Vec<String> {
        let by: HashMap<&str, &Entry> = list.iter().map(|e| (e.id.as_str(), e)).collect();
        let mut out = vec![];
        let mut p = by[id].meta.parent.as_deref();
        while let Some(f) = p { out.insert(0, by[f].title.clone()); p = by[f].meta.parent.as_deref(); }
        out
    }

    fn targets(e: &Entry) -> Vec<String> { let mut t: Vec<String> = e.links.iter().map(|l| l.target.clone()).collect(); t.sort(); t }

    #[test]
    fn moves_and_renames_keep_links() {
        let lib = Lib::new("relink");
        let mut s = lib.open();
        let a = s.create_from(&doc("甲")).unwrap();
        let b = s.create_from(&linking("乙", &[(&a, "甲")])).unwrap();
        let a_doc = linking("甲", &[(&b, "乙")]);
        s.save(&a, &a_doc).unwrap();
        let a_path = s.path(&a); let b_path = s.path(&b);
        let a_body = std::fs::read_to_string(&a_path).unwrap();
        let b_body = std::fs::read_to_string(&b_path).unwrap();
        assert!(a_body.contains(&format!("href=\"{b}.xml\"")));
        assert!(b_body.contains(&format!("href=\"{a}.xml\"")));
        s.load_for_edit(&a, false).unwrap();
        let f = s.create_folder("夹", None).unwrap();
        let journal = s.track_file_changes();
        s.set_parent(&b, Some(&f)).unwrap();
        let changes = journal.take();
        s.rename(&f, "新夹").unwrap();
        s.rename(&b, "新乙").unwrap();
        s.save(&a, &a_doc).unwrap();
        assert_eq!(std::fs::read_to_string(&a_path).unwrap(), a_body);
        assert_eq!(std::fs::read_to_string(&b_path).unwrap(), b_body);
        assert_eq!(targets(s.list().iter().find(|e| e.id == b).unwrap()), std::slice::from_ref(&a));
        s.apply_file_history(&changes, false).unwrap();
        assert!(s.index[&b].parent.is_none());
        assert_eq!(s.file_path(&b).unwrap(), b_path);
    }

    /// Batches publish their manifest once.
    #[test]
    fn batches_write_the_index_once() {
        let lib = Lib::new("batch");
        let mut s = lib.open();
        let index = lib.0.join(FILE);
        let journal = s.track_file_changes();
        let (f, ids) = s.batch(|s| {
            let f = s.create_folder("导入", None).unwrap();
            let g = s.create_folder("子", Some(&f)).unwrap();
            let mut ids = vec![];
            for i in 0..5 { ids.push(s.create_markdown_in(&doc(&format!("笔记 {i}")), "", Some(&f)).unwrap()); }
            ids.push(s.create_from_in(&doc("深处"), Some(&g)).unwrap());
            // A failure midway leaves the rest of the batch alone.
            assert!(s.create_from_in(&doc("无处"), Some("不存在")).is_err());
            // Nothing reached the index yet.
            assert!(!std::fs::read_to_string(&index).unwrap_or_default().contains(&ids[0]));
            assert_eq!(s.batch(|s| s.create_markdown_in(&doc("嵌套"), "", Some(&f)).unwrap()).map(|_| ()).ok(), Some(()));
            (f, ids)
        }).unwrap();
        assert!(std::fs::read_to_string(&index).unwrap().contains(&ids[0]));
        assert!(lib.0.join(format!("Notes/{}.md", ids[4])).is_file() && lib.0.join(format!("Notes/{}.xml", ids[5])).is_file());
        assert_eq!(journal.take().iter().filter(|d| d.before.is_none()).count(), 9);
        let mut again = lib.open();
        let list = again.list();
        assert_eq!(list.len(), 9);
        let kids: Vec<&Entry> = { let mut k: Vec<&Entry> = list.iter().filter(|e| e.meta.parent.as_deref() == Some(f.as_str()) && e.meta.folder.is_none()).collect(); k.sort_by_key(|e| e.meta.order); k };
        let titles: Vec<&str> = kids.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, ["笔记 0", "笔记 1", "笔记 2", "笔记 3", "笔记 4", "嵌套"]);
        assert_eq!(folders_of(&list, &ids[5]), ["导入", "子"]);
    }

    /// A batch cut short (the app quit or crashed) never leaves an index
    /// entry without its file: what was written is found on the next look.
    #[test]
    fn an_unfinished_batch_leaves_a_consistent_library() {
        let lib = Lib::new("batch-cut");
        let mut s = lib.open();
        let kept = s.create_from(&doc("原有")).unwrap();
        let f = s.create_folder("夹", None).unwrap();
        s.begin_batch().unwrap();
        for i in 0..3 { s.create_from_in(&doc(&format!("新 {i}")), Some(&f)).unwrap(); }
        s.delete(&kept, None).unwrap();
        drop(s);
        let mut again = lib.open();
        let list = again.list();
        for e in &list {
            if e.meta.folder.is_none() { assert!(again.path(&e.id).is_file(), "{} has no file", e.title); }
        }
        let mut titles: Vec<&str> = list.iter().filter(|e| e.meta.folder.is_none()).map(|e| e.title.as_str()).collect();
        titles.sort();
        assert_eq!(titles, ["新 0", "新 1", "新 2"]);
        assert_eq!(again.list_trash().len(), 1);
    }

    /// Deleting a big folder in a batch: two index writes, one 回收站 row,
    /// and 恢复 brings it all back.
    #[test]
    fn a_folder_deleted_in_a_batch_comes_back_whole() {
        let lib = Lib::new("batch-trash");
        let mut s = lib.open();
        let f = s.create_folder("夹", None).unwrap();
        let ids = s.batch(|s| (0..20).map(|i| s.create_from_in(&doc(&format!("文 {i}")), Some(&f)).unwrap()).collect::<Vec<_>>()).unwrap();
        let journal = s.track_file_changes();
        // As the window does it: the folder, then each page under it.
        s.batch(|s| {
            s.delete(&f, None).unwrap();
            for id in &ids { s.delete(id, Some(&f)).unwrap(); }
        }).unwrap();
        assert_eq!(journal.take().len(), 21);
        let mut again = lib.open();
        assert!(again.list().is_empty());
        assert_eq!(again.list_trash().len(), 1);
        again.restore(&f).unwrap();
        let list = again.list();
        assert_eq!(list.len(), 21);
        assert!(ids.iter().all(|id| folders_of(&list, id) == ["夹"]));
    }

    /// 导入文件夹和删除大文件夹的耗时：
    /// `cargo test --release -p fdl_core import_speed -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn import_speed() {
        let notes = |s: &mut Store, f: &str, batch: bool| {
            let t = std::time::Instant::now();
            let run = |s: &mut Store| for i in 0..120 {
                let mut d = doc(&format!("笔记 {i}"));
                d.blocks.push(crate::model::Block::text("正文"));
                if batch { s.create_markdown_in(&d, "", Some(f)).unwrap(); }
                else { let id = s.create_markdown(&d, "").unwrap(); s.set_parent(&id, Some(f)).unwrap(); }
            };
            if batch { s.batch(run).unwrap(); } else { run(s); }
            t.elapsed()
        };
        let lib = Lib::new("import-speed");
        let mut s = lib.open();
        let a = s.create_folder("逐篇", None).unwrap();
        let b = s.create_folder("批量", None).unwrap();
        let slow = notes(&mut s, &a, false);
        let fast = notes(&mut s, &b, true);
        let t = std::time::Instant::now();
        s.batch(|s| s.delete(&b, None)).unwrap().unwrap();
        eprintln!("one by one {slow:?}, batch {fast:?}, delete {:?}", t.elapsed());
        // What 导入文件夹 does, from Markdown files on disk to the list shown.
        let src = Lib::new("import-src");
        std::fs::create_dir_all(src.0.join("子")).unwrap();
        for i in 0..120 {
            let body = format!("---\ntags: [a]\n---\n# 笔记 {i}\n\n{}\n\n- 一\n- 二\n\n```rust\nfn main() {{}}\n```\n", "一段正文，带 **加粗** 和 [链接](https://example.com)。".repeat(20));
            std::fs::write(src.0.join(if i % 3 == 0 { format!("子/笔记 {i}.md") } else { format!("笔记 {i}.md") }), body).unwrap();
        }
        let lib = Lib::new("import-real");
        let mut s = lib.open();
        let t = std::time::Instant::now();
        s.batch(|s| {
            let top = s.create_folder("导入", None).unwrap();
            let sub = s.create_folder("子", Some(&top)).unwrap();
            for (dir, parent) in [(src.0.clone(), &top), (src.0.join("子"), &sub)] {
                for e in std::fs::read_dir(dir).unwrap().flatten().filter(|e| e.path().is_file()) {
                    let p = e.path();
                    let text = String::from_utf8_lossy(&read_bounded(&p, 1 << 24).unwrap()).into_owned();
                    let stem = p.file_stem().unwrap().to_string_lossy().into_owned();
                    let d = crate::markdown::parse_file(&text, &stem, p.parent());
                    s.create_markdown_in(&d, crate::markdown::front_matter(&text).0, Some(parent)).unwrap();
                }
            }
        }).unwrap();
        let n = s.list().len();
        eprintln!("导入文件夹 ({n} entries, list included): {:?}", t.elapsed());
    }
}

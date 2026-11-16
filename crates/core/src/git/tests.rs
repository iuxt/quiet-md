use super::*;
use crate::{model::{Document, RichText}, store::Store};

struct Library(PathBuf);
impl Library {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("quiet-git-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap(); Self(path)
    }
    fn store(&self) -> Store { Store::open_at(self.0.clone()).unwrap() }
    fn git(&self) -> LibraryRepo { LibraryRepo::open(&self.0).unwrap() }
}
impl Drop for Library { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
fn doc(title: &str) -> Document { Document { title: RichText::plain(title), ..Default::default() } }
fn configure(repo: &LibraryRepo, remote: &Path) {
    repo.configure(&Config { name: "Test".into(), email: "test@example.invalid".into(), remote: remote.to_string_lossy().into_owned(), ..Default::default() }).unwrap();
}

#[test]
fn uuid_names_metadata_titles_links_and_trash_survive_reopen() {
    let lib = Library::new(); let mut store = lib.store();
    let folder = store.create_folder("笔记", None).unwrap();
    let first = store.create_from_in(&doc("同名"), Some(&folder)).unwrap();
    let md = store.create_markdown(&doc("Markdown 标题"), "---\nkey: keep\n---\n").unwrap();
    let old = store.file_path(&first).unwrap();
    let md_path = store.file_path(&md).unwrap();
    std::fs::write(&md_path, "[文章](笔记/同名.xml#段落)\n\n`[代码](笔记/同名.xml)`\n\n[引用][ref]\n\n[ref]: 笔记/同名.xml\n").unwrap();
    store.enable_git().unwrap();
    assert!(lib.0.join(".quiet/before-git-5/index.json").is_file());
    assert!(!old.exists());
    assert_eq!(store.file_path(&first).unwrap().file_stem().unwrap().to_str().unwrap(), first);
    let text = std::fs::read_to_string(store.file_path(&md).unwrap()).unwrap();
    assert!(text.contains(&format!("[文章](笔记/{first}.xml#段落)")), "{text}");
    assert!(text.contains("`[代码](笔记/同名.xml)`"));
    assert!(text.contains(&format!("[ref]: 笔记/{first}.xml")), "{text}");
    store.rename(&md, "新的标题").unwrap();
    let mut store = lib.store();
    assert_eq!(store.load(&md).unwrap().title.text, "新的标题");
    assert_eq!(store.file_path(&md).unwrap().file_stem().unwrap().to_str().unwrap(), md);
    store.set_parent(&md, Some(&folder)).unwrap();
    assert_eq!(store.load(&md).unwrap().title.text, "新的标题");
    store.delete(&md, None).unwrap(); store.restore(&md).unwrap();
    assert_eq!(store.load(&md).unwrap().title.text, "新的标题");
    let second = store.create_from(&doc("同名")).unwrap();
    assert_ne!(store.file_path(&first).unwrap().file_name(), store.file_path(&second).unwrap().file_name());
}

#[test]
fn commits_use_lfs_pointers_and_history_tracks_ids_across_moves() {
    let lib = Library::new(); let mut store = lib.store(); store.enable_git().unwrap();
    let id = store.create_from(&doc("First")).unwrap();
    std::fs::create_dir_all(lib.0.join(".assets")).unwrap();
    std::fs::write(lib.0.join(".assets/picture.png"), b"picture bytes").unwrap();
    let repo = lib.git(); configure(&repo, &Library::new().0);
    let first = repo.commit("first").unwrap().unwrap();
    assert!(repo.status().unwrap().changes.is_empty());
    assert!(repo.commit("unchanged").unwrap().is_none());
    let tree = repo.repo.find_commit(Oid::from_str(&first).unwrap()).unwrap().tree().unwrap();
    let blob = repo.repo.find_blob(tree.get_path(Path::new(".assets/picture.png")).unwrap().id()).unwrap();
    let pointer = Pointer::parse(blob.content()).unwrap().unwrap();
    assert_eq!(pointer.size, 13);
    assert_eq!(std::fs::read(lib.0.join(".assets/picture.png")).unwrap(), b"picture bytes");
    let folder = store.create_folder("Moved", None).unwrap(); store.set_parent(&id, Some(&folder)).unwrap();
    store.rename(&id, "Renamed").unwrap();
    repo.commit("moved").unwrap();
    assert_eq!(repo.history(Some(&id), 80).unwrap().len(), 2);
    assert!(repo.revision(&first, &id).unwrap().contains("First"));
    let on_disk = std::fs::read_to_string(lib.0.join(MANIFEST)).unwrap();
    assert!(on_disk.contains("Renamed"));
    assert!(on_disk.contains(&id));
}

#[test]
fn local_push_clone_pull_and_divergence_keep_text_and_images_safe() {
    let server = Library::new(); Repository::init_bare(&server.0).unwrap();
    let first = Library::new(); let mut a = first.store(); a.enable_git().unwrap();
    let id = a.create_from(&doc("Original")).unwrap();
    std::fs::create_dir_all(first.0.join(".assets")).unwrap();
    std::fs::write(first.0.join(".assets/test.png"), b"original image").unwrap();
    configure(&first.git(), &server.0); first.git().commit("initial").unwrap(); first.git().push(&Auth::default()).unwrap();
    let second = Library::new(); LibraryRepo::clone_into(server.0.to_str().unwrap(), &second.0, &Auth::default()).unwrap();
    let mut b = second.store();
    assert_eq!(b.load(&id).unwrap().title.text, "Original");
    assert_eq!(std::fs::read(second.0.join(".assets/test.png")).unwrap(), b"original image");
    assert!(second.git().status().unwrap().changes.is_empty());
    a.rename(&id, "Remote update").unwrap();
    std::fs::write(first.0.join(".assets/test.png"), b"changed image").unwrap();
    first.git().commit("remote").unwrap(); first.git().push(&Auth::default()).unwrap();
    assert!(b.git_pull(&Auth::default()).unwrap());
    assert_eq!(b.load(&id).unwrap().title.text, "Remote update");
    assert_eq!(std::fs::read(second.0.join(".assets/test.png")).unwrap(), b"changed image");
    b.rename(&id, "Local draft").unwrap();
    assert!(b.git_pull(&Auth::default()).is_err());
    assert_eq!(b.load(&id).unwrap().title.text, "Local draft");
    configure(&second.git(), &server.0); second.git().commit("local").unwrap();
    a.rename(&id, "Diverged remote").unwrap(); first.git().commit("diverged").unwrap(); first.git().push(&Auth::default()).unwrap();
    assert!(b.git_pull(&Auth::default()).unwrap_err().to_string().contains("分叉"));
    assert_eq!(b.load(&id).unwrap().title.text, "Local draft");
    assert!(second.git().push(&Auth::default()).is_err());
}

#[test]
fn lfs_rejects_corruption_and_no_tokens_enter_repo_config() {
    let lib = Library::new(); let mut store = lib.store(); store.enable_git().unwrap();
    let repo = lib.git(); let cache = lfs::Lfs::new(repo.repo.path());
    let p = cache.store(b"contents").unwrap();
    let path = repo.repo.path().join("lfs/objects").join(&p.oid[..2]).join(&p.oid[2..4]).join(&p.oid);
    std::fs::write(path, b"tampered").unwrap();
    assert!(cache.read(&p).is_err());
    assert!(Pointer::parse(b"version https://git-lfs.github.com/spec/v1\noid sha256:../bad\nsize 2\n").is_err());
    assert!(repo.configure(&Config { remote: "https://user:secret@example.invalid/repo.git".into(), ..Default::default() }).is_err());
    assert!(!std::fs::read_to_string(repo.repo.path().join("config")).unwrap().contains("secret"));
    assert!(http_url("http://example.invalid/lfs").is_err());
    assert!(safe_path(&lib.0, "../outside").is_err());
}

#[cfg(unix)]
#[test]
fn git_refuses_symlinks_and_parent_repositories() {
    let parent = Library::new(); Repository::init(&parent.0).unwrap();
    let child = parent.0.join("child"); std::fs::create_dir(&child).unwrap();
    assert!(LibraryRepo::check_location(&child).is_err());
    let lib = Library::new(); let mut store = lib.store(); store.enable_git().unwrap();
    std::fs::create_dir_all(lib.0.join(".assets")).unwrap();
    std::os::unix::fs::symlink("/etc/hosts", lib.0.join(".assets/leak.png")).unwrap();
    assert!(lib.git().status().is_err());
}

#[test]
fn historical_images_and_markdown_titles_are_independent_of_current_files() {
    let lib = Library::new(); let mut store = lib.store(); store.enable_git().unwrap();
    let mut document = doc("Old title");
    document.blocks = vec![crate::model::Block::new(crate::model::BlockKind::Image { src_kind: "path".into(), src: ".assets/image.png".into(),
        width: None, height: None, caption: None, crop: None }, RichText::default())];
    std::fs::create_dir_all(lib.0.join(".assets")).unwrap();
    std::fs::write(lib.0.join(".assets/image.png"), b"old picture").unwrap();
    let id = store.create_markdown(&document, "").unwrap();
    let repo = lib.git(); configure(&repo, &Library::new().0);
    let commit = repo.commit("old").unwrap().unwrap();
    store.rename(&id, "Current title").unwrap();
    std::fs::write(lib.0.join(".assets/image.png"), b"current picture").unwrap();
    repo.commit("current").unwrap();
    assert_eq!(store.git_versions(&id).unwrap().len(), 2);
    let version = store.git_versions(&id).unwrap().into_iter().find(|v| v.name.ends_with(&commit)).unwrap();
    let preview = store.version_document(&id, &version.name).unwrap();
    assert_eq!(preview.title.text, "Old title");
    let source = &preview.asset_sources()[0];
    assert!(source.starts_with(".quiet/git-preview/"));
    assert_eq!(std::fs::read(lib.0.join(source)).unwrap(), b"old picture");
    assert_eq!(std::fs::read(lib.0.join(".assets/image.png")).unwrap(), b"current picture");
    let restored = store.restore_version(&id, &version.name).unwrap();
    assert!(restored.asset_sources()[0].starts_with(".assets/"));
    assert_eq!(std::fs::read(lib.0.join(&restored.asset_sources()[0])).unwrap(), b"old picture");
    assert_eq!(std::fs::read(lib.0.join(".assets/image.png")).unwrap(), b"current picture");
    let copy = store.version_copy_document(&id, &version.name).unwrap();
    assert!(copy.asset_sources()[0].starts_with(".assets/"));
}

#[test]
fn failed_fast_forward_restores_files_head_and_index() {
    let lib = Library::new(); let mut store = lib.store(); store.enable_git().unwrap();
    let id = store.create_from(&doc("saved")).unwrap();
    let repo = lib.git(); configure(&repo, &Library::new().0); repo.commit("saved").unwrap();
    let old_head = repo.repo.head().unwrap().target().unwrap();
    let old_index = std::fs::read(repo.repo.path().join("index")).unwrap();
    let old_body = std::fs::read(store.file_path(&id).unwrap()).unwrap();
    let mut content = BTreeMap::new();
    for path in repo.files().unwrap() { content.insert(path.clone(), std::fs::read(lib.0.join(path)).unwrap()); }
    let path = store.file_path(&id).unwrap().strip_prefix(&lib.0).unwrap().to_str().unwrap().to_string();
    content.insert(path, b"<title>replacement</title><p>changed</p>".to_vec());
    // A locked ref makes the final atomic HEAD update fail after files were written.
    let lock = repo.repo.path().join("refs/heads/main.lock"); std::fs::write(&lock, "locked").unwrap();
    let tree = repo.repo.head().unwrap().peel_to_tree().unwrap();
    assert!(repo.apply(&content, &tree, old_head, "main", old_head).is_err());
    assert_eq!(std::fs::read(store.file_path(&id).unwrap()).unwrap(), old_body);
    assert_eq!(repo.repo.head().unwrap().target().unwrap(), old_head);
    assert_eq!(std::fs::read(repo.repo.path().join("index")).unwrap(), old_index);
}

#[test]
fn embedded_operations_run_with_an_empty_path() {
    if std::env::var_os("QUIET_GIT_TEST_NO_COMMANDS").is_some() {
        local_push_clone_pull_and_divergence_keep_text_and_images_safe();
        return;
    }
    let bin = Library::new();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "git::tests::embedded_operations_run_with_an_empty_path", "--nocapture"])
        .env("PATH", &bin.0).env("QUIET_GIT_TEST_NO_COMMANDS", "1").output().unwrap();
    assert!(result.status.success(), "{}\n{}", String::from_utf8_lossy(&result.stdout), String::from_utf8_lossy(&result.stderr));
}

#[test]
fn externally_moved_uuid_and_imported_files_keep_unique_names_and_titles() {
    let lib = Library::new(); let mut store = lib.store(); store.enable_git().unwrap();
    let id = store.create_markdown(&doc("Display title"), "").unwrap();
    let old = store.file_path(&id).unwrap();
    std::fs::create_dir(lib.0.join("external")).unwrap();
    std::fs::rename(&old, lib.0.join("external").join(old.file_name().unwrap())).unwrap();
    std::fs::write(lib.0.join("imported.md"), "# Body\n\nText\n").unwrap();
    let entries = store.list();
    let existing = entries.iter().find(|e| e.id == id).unwrap();
    assert_eq!(existing.title, "Display title");
    let imported = entries.iter().find(|e| e.title == "imported").unwrap();
    assert_eq!(store.file_path(&imported.id).unwrap().file_stem().unwrap().to_str().unwrap(), imported.id);
    assert!(!lib.0.join("imported.md").exists());
    assert!(lib.git().status().is_ok());
}

#[test]
fn clone_uses_the_remote_default_branch() {
    let server = Library::new(); let bare = Repository::init_bare(&server.0).unwrap(); bare.set_head("refs/heads/trunk").unwrap();
    let source = Library::new(); let mut store = source.store(); store.enable_git().unwrap();
    let repo = source.git(); repo.repo.set_head("refs/heads/trunk").unwrap();
    configure(&repo, &server.0);
    let id = store.create_from(&doc("Trunk note")).unwrap();
    repo.commit("on trunk").unwrap(); repo.push(&Auth::default()).unwrap();
    let destination = Library::new();
    let repo = LibraryRepo::clone_into(server.0.to_str().unwrap(), &destination.0, &Auth::default()).unwrap();
    assert_eq!(repo.config().unwrap().branch, "trunk");
    assert_eq!(destination.store().load(&id).unwrap().title.text, "Trunk note");
}

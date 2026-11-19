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
fn staged_commits_keep_the_snapshot_and_leave_other_changes_uncommitted() {
    let lib = Library::new(); let mut store = lib.store(); store.enable_git().unwrap();
    let first = store.create_from(&doc("First")).unwrap(); let second = store.create_from(&doc("Second")).unwrap();
    let a = format!("notes/{first}.xml"); let b = format!("notes/{second}.xml");
    let repo = lib.git(); configure(&repo, &Library::new().0); repo.commit("initial").unwrap();
    let original = std::fs::read_to_string(lib.0.join(&a)).unwrap();
    let staged = original.replace("First", "Staged");
    std::fs::write(lib.0.join(&a), &staged).unwrap(); store.git_stage(&a, true).unwrap();
    assert!(repo.status().unwrap().changes.iter().any(|c| c.path == a && c.staged && !c.unstaged));
    std::fs::write(lib.0.join(&a), original.replace("First", "Later")).unwrap();
    let other = std::fs::read_to_string(lib.0.join(&b)).unwrap().replace("Second", "Unstaged");
    std::fs::write(lib.0.join(&b), &other).unwrap();
    assert!(repo.status().unwrap().changes.iter().any(|c| c.path == a && c.staged && c.unstaged));
    let commit = store.git_commit("selected").unwrap().unwrap();
    assert_eq!(repo.revision(&commit, &first).unwrap(), staged);
    assert!(repo.revision(&commit, &second).unwrap().contains("Second"));
    assert_eq!(repo.status().unwrap().changes.len(), 2);
    store.git_discard(&a).unwrap();
    assert_eq!(std::fs::read_to_string(lib.0.join(&a)).unwrap(), staged);
    assert_eq!(std::fs::read_to_string(lib.0.join(&b)).unwrap(), other);
    store.git_stage(&b, true).unwrap(); store.git_stage(&b, false).unwrap();
    assert!(repo.status().unwrap().changes.iter().all(|c| !c.staged));
    // A staged change still exists when the worktree has been put back to HEAD.
    store.git_stage(&b, true).unwrap();
    let head = repo.repo.head().unwrap().peel_to_commit().unwrap().id().to_string();
    std::fs::write(lib.0.join(&b), repo.revision(&head, &second).unwrap()).unwrap();
    assert!(repo.status().unwrap().changes.iter().any(|c| c.path == b && c.staged && c.unstaged));
    store.git_discard(&b).unwrap();
    assert_eq!(std::fs::read_to_string(lib.0.join(&b)).unwrap(), other);
    assert!(repo.status().unwrap().changes.iter().any(|c| c.path == b && c.staged && !c.unstaged));
}

#[test]
fn blank_line_changes_are_hidden_and_restored_without_creating_a_commit() {
    let lib = Library::new(); let mut store = lib.store(); store.enable_git().unwrap();
    let id = store.create_from(&doc("First")).unwrap(); let path = format!("notes/{id}.xml");
    let repo = lib.git(); configure(&repo, &Library::new().0);
    let initial = repo.commit("initial").unwrap().unwrap();
    let file = lib.0.join(&path);
    let original = std::fs::read_to_string(&file).unwrap();
    let with_blank_lines = format!("\n \t\r\n{original}");
    std::fs::write(&file, &with_blank_lines).unwrap();
    let status = repo.status().unwrap();
    assert_eq!(status.changes.len(), 1);
    assert!(status.visible_changes().next().is_none());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), with_blank_lines, "Status must be read-only");
    assert!(store.git_commit(" ").is_err());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), with_blank_lines);
    store.git_stage(&path, true).unwrap();
    std::fs::write(&file, format!("\n{with_blank_lines}")).unwrap();
    let status = repo.status().unwrap();
    assert!(status.changes[0].staged && status.changes[0].unstaged && status.changes[0].blank_only);
    assert_eq!(store.git_commit("blank only").unwrap(), None);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
    let status = repo.status().unwrap();
    assert_eq!(status.head.as_deref(), Some(initial.as_str()));
    assert!(status.changes.is_empty());
}

#[test]
fn commits_include_substantive_changes_and_restore_only_blank_line_files() {
    let lib = Library::new(); let mut store = lib.store(); store.enable_git().unwrap();
    let clean = store.create_markdown(&doc("Clean"), "").unwrap();
    let mixed = store.create_from(&doc("Mixed")).unwrap();
    let clean_path = format!("notes/{clean}.md"); let mixed_path = format!("notes/{mixed}.xml");
    let clean_file = lib.0.join(&clean_path); let mixed_file = lib.0.join(&mixed_path);
    let repo = lib.git(); configure(&repo, &Library::new().0); repo.commit("initial").unwrap();
    let original = std::fs::read_to_string(&clean_file).unwrap();
    let blank = format!("\n{original}");
    let updated = format!("\n{}", std::fs::read_to_string(&mixed_file).unwrap().replace("Mixed", "Updated"));
    std::fs::write(&clean_file, &blank).unwrap(); store.git_stage(&clean_path, true).unwrap();
    std::fs::write(&mixed_file, &updated).unwrap();
    let status = repo.status().unwrap();
    let visible: Vec<_> = status.visible_changes().map(|c| c.path.as_str()).collect();
    assert_eq!(visible, [mixed_path.as_str()]);
    let commit = store.git_commit("text and blank lines").unwrap().unwrap();
    assert_eq!(repo.revision(&commit, &clean).unwrap(), original);
    assert_eq!(std::fs::read_to_string(&clean_file).unwrap(), original);
    assert_eq!(repo.revision(&commit, &mixed).unwrap(), updated);

    let staged = updated.replace("Updated", "Staged");
    let later = staged.replace("Staged", "Later");
    std::fs::write(&mixed_file, &staged).unwrap(); store.git_stage(&mixed_path, true).unwrap();
    std::fs::write(&clean_file, &blank).unwrap(); store.git_stage(&clean_path, true).unwrap();
    std::fs::write(&mixed_file, &later).unwrap();
    let commit = store.git_commit("staged edits").unwrap().unwrap();
    assert_eq!(repo.revision(&commit, &clean).unwrap(), original);
    assert_eq!(std::fs::read_to_string(&clean_file).unwrap(), original);
    assert_eq!(repo.revision(&commit, &mixed).unwrap(), staged);
    assert_eq!(std::fs::read_to_string(&mixed_file).unwrap(), later);
    assert_eq!(repo.status().unwrap().visible_changes().count(), 1);
}

#[test]
fn stage_all_and_unstage_all_preserve_working_files_and_lfs_snapshots() {
    let lib = Library::new(); let mut store = lib.store(); store.enable_git().unwrap();
    let id = store.create_from(&doc("First")).unwrap(); let path = format!("notes/{id}.xml");
    let repo = lib.git(); configure(&repo, &Library::new().0);
    store.git_stage_all(true).unwrap();
    assert!(repo.status().unwrap().changes.iter().all(|c| c.staged && !c.unstaged));
    store.git_stage_all(false).unwrap();
    assert!(repo.status().unwrap().changes.iter().all(|c| !c.staged && c.unstaged));
    assert!(repo.repo.head().is_err());
    repo.commit("initial").unwrap();
    let original = std::fs::read_to_string(lib.0.join(&path)).unwrap();
    let updated = original.replace("First", "Updated");
    std::fs::write(lib.0.join(&path), &updated).unwrap();
    std::fs::create_dir_all(lib.0.join(".assets")).unwrap();
    std::fs::write(lib.0.join(".assets/test.bin"), b"staged asset").unwrap();
    std::fs::write(lib.0.join("unmanaged.txt"), "keep private").unwrap();
    let added = store.create_from(&doc("Added")).unwrap();
    store.git_stage_all(true).unwrap();
    let changes = repo.status().unwrap().changes;
    assert!(changes.iter().any(|c| c.path == MANIFEST));
    assert!(changes.iter().all(|c| c.staged && !c.unstaged));
    store.git_stage_all(false).unwrap();
    assert!(repo.status().unwrap().changes.iter().all(|c| !c.staged && c.unstaged));
    assert_eq!(std::fs::read_to_string(lib.0.join(&path)).unwrap(), updated);
    assert_eq!(std::fs::read(lib.0.join(".assets/test.bin")).unwrap(), b"staged asset");
    store.git_stage_all(true).unwrap();
    std::fs::write(lib.0.join(&path), original.replace("First", "Later")).unwrap();
    std::fs::write(lib.0.join(".assets/test.bin"), b"later asset").unwrap();
    let commit = store.git_commit("all staged").unwrap().unwrap();
    assert_eq!(repo.revision(&commit, &id).unwrap(), updated);
    assert!(repo.revision(&commit, &added).unwrap().contains("Added"));
    let tree = repo.repo.find_commit(Oid::from_str(&commit).unwrap()).unwrap().tree().unwrap();
    assert!(tree.get_name("unmanaged.txt").is_none());
    let blob = repo.repo.find_blob(tree.get_path(Path::new(".assets/test.bin")).unwrap().id()).unwrap();
    let pointer = Pointer::parse(blob.content()).unwrap().unwrap();
    assert_eq!(lfs::Lfs::new(repo.repo.path()).read(&pointer).unwrap(), b"staged asset");
    std::fs::remove_file(lib.0.join(".assets/test.bin")).unwrap();
    store.git_stage_all(true).unwrap();
    assert!(repo.status().unwrap().changes.iter().any(|c| c.path == ".assets/test.bin" && c.kind == 'D' && c.staged && !c.unstaged));
    store.git_stage_all(false).unwrap();
    assert!(repo.status().unwrap().changes.iter().all(|c| !c.staged && c.unstaged));
    assert!(!lib.0.join(".assets/test.bin").exists());
}

#[test]
fn discard_restores_deleted_files_and_removes_new_notes_without_broken_metadata() {
    let lib = Library::new(); let mut store = lib.store(); store.enable_git().unwrap();
    let id = store.create_from(&doc("Keep")).unwrap(); let path = format!("notes/{id}.xml");
    let repo = lib.git(); configure(&repo, &Library::new().0); repo.commit("initial").unwrap();
    let original = std::fs::read(lib.0.join(&path)).unwrap();
    std::fs::remove_file(lib.0.join(&path)).unwrap();
    assert!(repo.status().unwrap().changes.iter().any(|c| c.path == path && c.kind == 'D'));
    store.git_discard(&path).unwrap();
    assert_eq!(std::fs::read(lib.0.join(&path)).unwrap(), original);
    let added = store.create_from(&doc("Remove")).unwrap(); let new_path = format!("notes/{added}.xml");
    assert!(store.git_discard(MANIFEST).is_err(), "Restoring metadata must not orphan a new note that the scanner would re-adopt");
    store.git_discard(&new_path).unwrap();
    assert!(!lib.0.join(&new_path).exists());
    assert!(!store.list().iter().any(|e| e.id == added));
    assert!(store.list().iter().any(|e| e.id == id));
    std::fs::write(lib.0.join("unmanaged.txt"), "keep").unwrap();
    for path in ["../outside", ".git/config", "unmanaged.txt"] {
        assert!(store.git_discard(path).is_err()); assert!(store.git_stage(path, true).is_err());
    }
    assert_eq!(std::fs::read_to_string(lib.0.join("unmanaged.txt")).unwrap(), "keep");
}

#[test]
fn staging_assets_uses_lfs_and_incomplete_staged_libraries_do_not_commit() {
    let lib = Library::new(); let mut store = lib.store(); store.enable_git().unwrap();
    let id = store.create_from(&doc("First")).unwrap(); let path = format!("notes/{id}.xml");
    let repo = lib.git(); configure(&repo, &Library::new().0);
    repo.stage(&path).unwrap();
    assert!(store.git_commit("incomplete").is_err()); assert!(repo.repo.head().is_err());
    repo.unstage(&path).unwrap();
    assert!(repo.status().unwrap().changes.iter().all(|c| !c.staged));
    repo.commit("initial").unwrap();
    std::fs::create_dir_all(lib.0.join(".assets")).unwrap();
    std::fs::write(lib.0.join(".assets/test.bin"), b"original image").unwrap();
    repo.stage(".assets/test.bin").unwrap();
    std::fs::write(lib.0.join(".assets/test.bin"), b"later image").unwrap();
    store.git_discard(".assets/test.bin").unwrap();
    assert_eq!(std::fs::read(lib.0.join(".assets/test.bin")).unwrap(), b"original image");
    let commit = store.git_commit("asset").unwrap().unwrap();
    let tree = repo.repo.find_commit(Oid::from_str(&commit).unwrap()).unwrap().tree().unwrap();
    let blob = repo.repo.find_blob(tree.get_path(Path::new(".assets/test.bin")).unwrap().id()).unwrap();
    assert!(Pointer::parse(blob.content()).unwrap().is_some());
    std::fs::write(lib.0.join(".assets/new.bin"), b"new").unwrap();
    store.git_discard(".assets/new.bin").unwrap(); assert!(!lib.0.join(".assets/new.bin").exists());
}

#[test]
fn uuid_names_metadata_titles_links_and_trash_survive_reopen() {
    let lib = Library::new();
    std::fs::create_dir_all(lib.0.join(".quiet")).unwrap();
    std::fs::write(lib.0.join(".quiet/format.json"), b"{\"format\":5}").unwrap();
    let mut store = lib.store();
    let folder = store.create_folder("笔记", None).unwrap();
    // A pre-UUID library, to exercise conversion rather than new-document creation.
    let first = uuid::Uuid::new_v4().to_string();
    let md = uuid::Uuid::new_v4().to_string();
    let old = lib.0.join("笔记/同名.xml");
    let md_path = lib.0.join("Markdown 标题.md");
    std::fs::write(&old, crate::docxml::serialize(&doc("同名"))).unwrap();
    let path = lib.0.join(".quiet/index.json");
    let mut metadata: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for (path, id) in [("笔记/同名.xml", &first), ("Markdown 标题.md", &md)] {
        metadata[path] = serde_json::json!({"id": id, "created": 1, "modified": 1});
    }
    std::fs::write(path, serde_json::to_vec_pretty(&metadata).unwrap()).unwrap();
    std::fs::write(&md_path, "[文章](笔记/同名.xml#段落)\n\n`[代码](笔记/同名.xml)`\n\n[引用][ref]\n\n[ref]: 笔记/同名.xml\n").unwrap();
    drop(store);
    let mut store = lib.store();
    store.enable_git().unwrap();
    assert!(std::fs::read_dir(lib.0.join(".quiet")).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with("before-virtual-6-")));
    assert!(!old.exists());
    assert_eq!(store.file_path(&first).unwrap().file_stem().unwrap().to_str().unwrap(), first);
    let text = std::fs::read_to_string(store.file_path(&md).unwrap()).unwrap();
    assert!(text.contains(&format!("[文章]({first}.xml#段落)")), "{text}");
    assert!(text.contains("`[代码](笔记/同名.xml)`"));
    assert!(text.contains(&format!("[ref]: {first}.xml")), "{text}");
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
    assert!(restored.asset_sources()[0].starts_with("assets/"));
    assert_eq!(std::fs::read(lib.0.join(&restored.asset_sources()[0])).unwrap(), b"old picture");
    assert_eq!(std::fs::read(lib.0.join(".assets/image.png")).unwrap(), b"current picture");
    let copy = store.version_copy_document(&id, &version.name).unwrap();
    assert!(copy.asset_sources()[0].starts_with("assets/"));
}

#[test]
fn drawing_history_reads_assets_without_materializing_previews() {
    use crate::model::{Block, BlockKind};
    let lib = Library::new(); let mut store = lib.store(); store.enable_git().unwrap();
    let mut drawing = Store::save_drawing(&lib.0, b"first drawing", 400, 200).unwrap();
    assert!(drawing.preview.is_empty());
    assert_eq!(std::fs::read_dir(lib.0.join("assets")).unwrap().count(), 1);
    // Older notes can carry an existing PNG. A read-only history view uses
    // the committed copy in memory even if the working file has changed.
    drawing.preview = "assets/old.png".into();
    std::fs::write(lib.0.join(&drawing.preview), b"first preview").unwrap();
    let mut document = doc("Drawing history");
    document.blocks = vec![Block::new(BlockKind::Drawing(drawing.clone()), RichText::default())];
    let id = store.create_from(&document).unwrap();
    let repo = lib.git(); configure(&repo, &Library::new().0);
    let old = repo.commit("legacy drawing").unwrap().unwrap();

    let next = Store::save_drawing(&lib.0, b"second drawing", 200, 400).unwrap();
    document.blocks[0].kind = BlockKind::Drawing(next.clone());
    store.save(&id, &document).unwrap();
    let head = repo.commit("drawing with source only").unwrap().unwrap();
    let xml = repo.revision(&head, &id).unwrap();
    assert!(!xml.contains("preview="));
    assert_eq!(crate::docxml::serialize(&crate::docxml::parse(&xml).unwrap()), xml);
    std::fs::write(lib.0.join(&drawing.preview), b"working preview").unwrap();

    let historical = store.version_document(&id, &format!("git:0-{old}")).unwrap();
    let BlockKind::Drawing(old_draw) = &historical.blocks[0].kind else { panic!("expected a drawing") };
    assert_eq!(crate::store::read_drawing_asset(&lib.0, &old_draw.source, 1024).unwrap(), b"first drawing");
    assert_eq!(crate::store::read_drawing_asset(&lib.0, &old_draw.preview, 1024).unwrap(), b"first preview");
    assert!(crate::store::read_drawing_asset(&lib.0, &old_draw.preview, 3).is_err());
    let historical = store.version_document(&id, &format!("git:0-{head}")).unwrap();
    let BlockKind::Drawing(new_draw) = &historical.blocks[0].kind else { panic!("expected a drawing") };
    assert!(new_draw.preview.is_empty());
    assert_eq!(crate::store::read_drawing_asset(&lib.0, &new_draw.source, 1024).unwrap(), b"second drawing");
    assert!(!lib.0.join(".quiet/git-preview").exists());
    assert_eq!(std::fs::read_dir(lib.0.join("assets")).unwrap().count(), 3);
}

#[test]
fn historical_images_with_windows_paths_use_committed_assets() {
    let lib = Library::new(); let mut store = lib.store(); store.enable_git().unwrap();
    std::fs::create_dir_all(lib.0.join(".assets/nested")).unwrap();
    std::fs::write(lib.0.join(".assets/nested/image.png"), b"old picture").unwrap();
    let mut ids = Vec::new();
    for source in [r".assets\nested\image.png", r"C:\Notes\.assets\nested\image.png"] {
        let mut document = doc("Windows image");
        document.blocks = vec![crate::model::Block::new(crate::model::BlockKind::Image { src_kind: "path".into(), src: source.into(),
            width: None, height: None, caption: None, crop: None }, RichText::default())];
        ids.push(store.create_from(&document).unwrap());
    }
    let repo = lib.git(); configure(&repo, &Library::new().0);
    let commit = repo.commit("old").unwrap().unwrap();
    std::fs::write(lib.0.join(".assets/nested/image.png"), b"current picture").unwrap();
    for id in ids {
        let version = store.git_versions(&id).unwrap().into_iter().find(|v| v.name.ends_with(&commit)).unwrap();
        let preview = store.version_document(&id, &version.name).unwrap();
        let source = &preview.asset_sources()[0];
        assert!(source.starts_with(".quiet/git-preview/"), "{source}");
        assert_eq!(std::fs::read(lib.0.join(source)).unwrap(), b"old picture");
        let restored = store.restore_version(&id, &version.name).unwrap();
        let source = &restored.asset_sources()[0];
        assert!(source.starts_with("assets/"), "{source}");
        assert_eq!(std::fs::read(lib.0.join(source)).unwrap(), b"old picture");
    }
    assert_eq!(std::fs::read(lib.0.join(".assets/nested/image.png")).unwrap(), b"current picture");
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
    for path in repo.files(FORMAT).unwrap() { content.insert(path.clone(), std::fs::read(lib.0.join(path)).unwrap()); }
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
fn virtual_library_adopts_notes_but_keeps_hierarchy_in_manifest() {
    let lib = Library::new(); let mut store = lib.store(); store.enable_git().unwrap();
    let id = store.create_markdown(&doc("Display title"), "").unwrap();
    let old = store.file_path(&id).unwrap();
    std::fs::create_dir(lib.0.join("external")).unwrap();
    std::fs::rename(&old, lib.0.join("external").join(old.file_name().unwrap())).unwrap();
    std::fs::write(lib.0.join("notes/imported.md"), "# Body\n\nText\n").unwrap();
    let entries = store.list();
    let existing = entries.iter().find(|e| e.id == id).unwrap();
    assert_eq!(existing.title, "Display title");
    let imported = entries.iter().find(|e| e.title == "imported").unwrap();
    assert_eq!(store.file_path(&imported.id).unwrap().file_stem().unwrap().to_str().unwrap(), imported.id);
    assert!(!lib.0.join("notes/imported.md").exists());
    assert!(!entries.iter().any(|e| e.meta.folder.is_some()));
    assert!(store.load(&id).is_err());
    std::fs::rename(lib.0.join("external").join(old.file_name().unwrap()), old).unwrap();
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

#[test]
fn virtual_empty_folders_survive_clone_and_are_not_treated_as_pristine() {
    let server = Library::new(); Repository::init_bare(&server.0).unwrap();
    let first = Library::new(); let mut a = first.store();
    let folder = a.create_folder("Empty", None).unwrap(); a.enable_git().unwrap();
    configure(&first.git(), &server.0); first.git().commit("empty virtual folder").unwrap(); first.git().push(&Auth::default()).unwrap();
    let second = Library::new(); LibraryRepo::clone_into(server.0.to_str().unwrap(), &second.0, &Auth::default()).unwrap();
    let entries = second.store().list(); assert_eq!(entries.len(), 1); assert_eq!(entries[0].id, folder); assert_eq!(entries[0].title, "Empty");
    assert!(!second.0.join("Empty").exists());
    let local = Library::new(); let mut draft = local.store(); let own = draft.create_folder("Unsaved folder", None).unwrap();
    draft.enable_git().unwrap(); configure(&local.git(), &server.0);
    assert!(draft.git_pull(&Auth::default()).is_err()); assert!(draft.list().iter().any(|e| e.id == own));
}

#[test]
fn physical_git_commits_remain_readable_after_virtual_conversion() {
    let lib = Library::new(); std::fs::create_dir_all(lib.0.join(".quiet")).unwrap(); std::fs::create_dir_all(lib.0.join("Physical")).unwrap();
    std::fs::write(lib.0.join(".quiet/format.json"), b"{\"format\":5}").unwrap();
    let folder = uuid::Uuid::new_v4().to_string(); let a = uuid::Uuid::new_v4().to_string(); let b = uuid::Uuid::new_v4().to_string();
    let path_a = format!("Physical/{a}.xml"); let path_b = format!("Physical/{b}.xml");
    let body = format!("<title>Original</title>\n<p><cite type=\"doc\" href=\"{b}.xml\" file-type=\"docx\">Target</cite></p>\n");
    std::fs::write(lib.0.join(&path_a), &body).unwrap(); std::fs::write(lib.0.join(&path_b), crate::docxml::serialize(&doc("Target"))).unwrap();
    std::fs::write(lib.0.join(MANIFEST), serde_json::to_vec_pretty(&serde_json::json!({
        "Physical/": {"id": folder, "created": 1, "modified": 1},
        path_a: {"id": a, "created": 1, "modified": 1, "stable_name": true, "title": "Original"},
        path_b: {"id": b, "created": 1, "modified": 1, "stable_name": true, "title": "Target"}
    })).unwrap()).unwrap();
    let repo = LibraryRepo::init(&lib.0).unwrap(); configure(&repo, &Library::new().0); let old = repo.commit("physical").unwrap().unwrap();
    let mut store = lib.store(); store.use_virtual_folders().unwrap(); store.rename(&a, "Current title").unwrap(); repo.commit("virtual").unwrap();
    let version = format!("git:0-{old}"); let historical = store.version_document(&a, &version).unwrap();
    assert_eq!(historical.title.text, "Original"); assert_eq!(historical.doc_links()[0].target, b);
    assert!(repo.revision(&old, &a).unwrap().contains("Original")); assert_eq!(store.git_versions(&a).unwrap().len(), 2);
    assert_eq!(store.file_path(&a).unwrap(), lib.0.join(format!("notes/{a}.xml")));
}

#[test]
fn change_previews_compare_managed_text_and_keep_binary_assets_out() {
    use crate::diff::{counts, Kind};
    let lib = Library::new(); let mut store = lib.store(); store.enable_git().unwrap();
    let mut first = doc("Preview note");
    first.blocks.push(crate::model::Block::text("Before"));
    let id = store.create_from(&first).unwrap();
    let path = format!("notes/{id}.xml");
    let repo = lib.git(); configure(&repo, &Library::new().0);
    let added = repo.change_preview(&path).unwrap();
    assert_eq!(added.title, "Preview note");
    assert!(added.lines.iter().all(|l| l.kind == Kind::Added));
    assert!(added.source.is_some());
    assert!(added.formatted.as_ref().unwrap().rows.iter().all(|(k, _)| *k == Kind::Added));
    assert!(repo.change_preview("../outside").is_err());
    std::fs::write(lib.0.join("unmanaged.txt"), "private").unwrap();
    assert!(repo.change_preview("unmanaged.txt").is_err());
    repo.commit("before").unwrap();
    let original = std::fs::read_to_string(lib.0.join(&path)).unwrap();
    std::fs::write(lib.0.join(&path), format!("\n{original}")).unwrap();
    let formatting = repo.change_preview(&path).unwrap();
    assert_eq!(counts(&formatting.lines), (0, 0));
    assert_ne!(counts(formatting.source.as_ref().unwrap()), (0, 0));
    assert!(formatting.formatted.as_ref().unwrap().rows.iter().all(|(k, _)| *k == Kind::Same));
    std::fs::write(lib.0.join(&path), original.replace("Before", "<b>Before</b>")).unwrap();
    let styled = repo.change_preview(&path).unwrap();
    assert_eq!(counts(&styled.lines), (0, 0));
    assert!(styled.formatted.as_ref().unwrap().rows.iter().any(|(k, b)| *k == Kind::Added && b.text.runs[0].marks.bold));
    let body = original.replace("Before", "After");
    std::fs::write(lib.0.join(&path), body).unwrap();
    let changed = repo.change_preview(&path).unwrap();
    assert_eq!(counts(&changed.lines), (1, 1));
    assert!(changed.lines.iter().any(|l| l.kind == Kind::Added && l.text == "After"));
    std::fs::create_dir_all(lib.0.join(".assets")).unwrap();
    std::fs::write(lib.0.join(".assets/image.png"), b"image content").unwrap();
    let binary = repo.change_preview(".assets/image.png").unwrap();
    assert!(binary.binary && binary.lines.is_empty());
    let markdown = "cccccccc-0000-4000-8000-000000000003.md";
    let mut meta: serde_json::Value = serde_json::from_slice(&std::fs::read(lib.0.join(MANIFEST)).unwrap()).unwrap();
    meta["notes"].as_array_mut().unwrap().push(serde_json::json!({"id": "cccccccc-0000-4000-8000-000000000003", "filename": markdown, "title": "Long note", "folderId": null, "createdAt": "1970-01-01T00:00:01Z", "updatedAt": "1970-01-01T00:00:01Z"}));
    std::fs::write(lib.0.join(MANIFEST), serde_json::to_vec(&meta).unwrap()).unwrap();
    std::fs::write(lib.0.join("notes").join(markdown), "line\n".repeat(crate::diff::MAX_LINES + 1)).unwrap();
    let long = repo.change_preview(&format!("notes/{markdown}")).unwrap();
    assert!(long.truncated && long.source.is_none());
    let metadata = repo.change_preview(MANIFEST).unwrap();
    assert!(!metadata.binary && metadata.source.is_none());
    // A permanently removed note is still readable from HEAD and its old metadata.
    let mut metadata: serde_json::Value = serde_json::from_slice(&std::fs::read(lib.0.join(MANIFEST)).unwrap()).unwrap();
    metadata["notes"] = serde_json::json!([]);
    std::fs::write(lib.0.join(MANIFEST), serde_json::to_vec(&metadata).unwrap()).unwrap();
    std::fs::remove_file(lib.0.join(&path)).unwrap();
    let removed = repo.change_preview(&path).unwrap();
    assert_eq!(removed.title, "Preview note");
    assert!(removed.lines.iter().all(|l| l.kind == Kind::Removed));
}

#[test]
fn ssh_and_ca_configuration_round_trip_without_saving_secrets() {
    let lib = Library::new(); let mut store = lib.store(); store.enable_git().unwrap();
    let repo = lib.git();
    // The escaped colon is also accepted in existing Git config files.
    let path = repo.repo.path().join("config");
    let original = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("{original}\n[http \"https\\://git.example.invalid\"]\n\tsslCAInfo = /tmp/quiet-test-ca.crt\n")).unwrap();
    let mut config = Config { remote: "ssh://git@git.example.invalid:2222/iuxt/notebook.git".into(),
        name: "Test".into(), email: "test@example.invalid".into(), ssh_key: "~/.ssh/notes_key".into(), lfs_ca_info: "/tmp/quiet-test-ca.crt".into(), ..Default::default() };
    repo.configure(&config).unwrap();
    let saved = repo.config().unwrap();
    assert_eq!(saved.remote, config.remote); assert_eq!(saved.ssh_key, config.ssh_key); assert_eq!(saved.lfs_ca_info, config.lfs_ca_info);
    config.lfs_ca_info = "/tmp/replacement-ca.crt".into(); repo.configure(&config).unwrap();
    assert_eq!(repo.config().unwrap().lfs_ca_info, config.lfs_ca_info);
    config.ssh_key.clear(); config.lfs_ca_info.clear(); repo.configure(&config).unwrap();
    assert!(repo.config().unwrap().ssh_key.is_empty()); assert!(repo.config().unwrap().lfs_ca_info.is_empty());
    let text = std::fs::read_to_string(path).unwrap();
    assert!(!text.contains("passphrase") && !text.contains("token"));
}

#[test]
#[ignore = "local SSH + TLS fixture: python3 tools/harness/git_ssh.py"]
fn ssh_push_clone_pull_with_self_signed_lfs() {
    // This ignored fixture runs alone in its own test process; redirect libgit2's global config to the temporary server directory.
    let config_dir = std::env::var("QUIET_MD_TEST_CONFIG_DIR").unwrap();
    unsafe { git2::opts::set_search_path(git2::ConfigLevel::Global, config_dir).unwrap(); }
    let remote = std::env::var("QUIET_MD_TEST_REMOTE").unwrap();
    let ca = std::env::var("QUIET_MD_TEST_CA").unwrap();
    let key = std::env::var("QUIET_MD_TEST_KEY").unwrap();
    let server = std::env::var("QUIET_MD_TEST_BARE").unwrap();
    Repository::init_bare(server).unwrap();
    let config = Config { name: "SSH Test".into(), email: "ssh@example.invalid".into(), remote, ssh_key: key, lfs_ca_info: ca, ..Default::default() };
    let auth = Auth { ssh_passphrase: "test-key-password".into(), ..Default::default() };
    let source = Library::new(); let mut store = source.store(); store.enable_git().unwrap();
    let id = store.create_from(&doc("SSH + TLS")).unwrap();
    std::fs::create_dir_all(source.0.join(".assets")).unwrap();
    std::fs::write(source.0.join(".assets/photo.png"), b"first picture").unwrap();
    let repo = source.git(); repo.configure(&config).unwrap(); repo.commit("first").unwrap();
    let known_path = PathBuf::from(std::env::var("QUIET_MD_GIT_KNOWN_HOSTS").unwrap());
    let known = std::fs::read(&known_path).unwrap(); std::fs::write(&known_path, "").unwrap();
    assert!(repo.push(&auth).is_err());
    let rejected = Library::new(); assert!(LibraryRepo::clone_configured(&config, &rejected.0, &auth).is_err());
    std::fs::write(&known_path, known).unwrap();
    let mut bad = config.clone(); bad.ssh_key = source.0.join("missing-key").to_string_lossy().into_owned();
    repo.configure(&bad).unwrap(); assert!(repo.push(&auth).is_err());
    repo.configure(&config).unwrap();
    assert!(repo.push(&Auth { ssh_passphrase: "wrong-passphrase".into(), ..Default::default() }).is_err());
    let endpoint = std::env::var("QUIET_MD_TEST_TLS").unwrap();
    let empty_config = git2::Config::new().unwrap();
    assert!(transport::http_agent(&empty_config, &endpoint).unwrap().get(&endpoint).call().is_err());
    repo.configure(&config).unwrap();
    assert_eq!(transport::ca_info(&repo.repo.config().unwrap(), &endpoint).unwrap(), Some(PathBuf::from(&config.lfs_ca_info)));
    repo.push(&auth).unwrap();
    let destination = Library::new();
    let cloned = LibraryRepo::clone_configured(&config, &destination.0, &auth).unwrap();
    assert_eq!(destination.store().load(&id).unwrap().title.text, "SSH + TLS");
    assert_eq!(std::fs::read(destination.0.join(".assets/photo.png")).unwrap(), b"first picture");
    store.rename(&id, "Updated over SSH").unwrap();
    std::fs::write(source.0.join(".assets/photo.png"), b"second picture").unwrap();
    repo.commit("second").unwrap(); repo.push(&auth).unwrap(); cloned.pull(&auth).unwrap();
    assert_eq!(destination.store().load(&id).unwrap().title.text, "Updated over SSH");
    assert_eq!(std::fs::read(destination.0.join(".assets/photo.png")).unwrap(), b"second picture");
    let text = std::fs::read_to_string(cloned.repo.path().join("config")).unwrap();
    assert!(!text.contains("test-key-password") && !text.contains("fixture-token"));
}

#[test]
fn trash_is_committed_synced_and_restored_with_original_bytes_and_folders() {
    let server = Library::new(); Repository::init_bare(&server.0).unwrap();
    let first = Library::new(); let mut a = first.store(); a.enable_git().unwrap();
    let folder = a.create_folder("Folder", None).unwrap(); let nested = a.create_folder("Nested", Some(&folder)).unwrap();
    let xml = a.create_from_in(&doc("XML"), Some(&nested)).unwrap();
    let md = a.create_markdown_in(&doc("Markdown"), "# Markdown\n\n![image](../.assets/test.png)\n", Some(&nested)).unwrap();
    std::fs::create_dir_all(first.0.join(".assets")).unwrap(); std::fs::write(first.0.join(".assets/test.png"), b"keep image").unwrap();
    let bodies: Vec<_> = [(&xml, "xml"), (&md, "md")].into_iter().map(|(id, ext)| (id.clone(), ext, std::fs::read(a.file_path(id).unwrap()).unwrap())).collect();
    configure(&first.git(), &server.0); first.git().commit("initial").unwrap(); first.git().push(&Auth::default()).unwrap();
    let second = Library::new(); LibraryRepo::clone_into(server.0.to_str().unwrap(), &second.0, &Auth::default()).unwrap();
    let mut b = second.store();
    a.delete(&folder, None).unwrap();
    let repo = first.git();
    let changes = repo.status().unwrap().changes; assert_eq!(changes.len(), 1); assert_eq!(changes[0].path, "quiet.json");
    let commit = repo.commit("move to trash").unwrap().unwrap(); repo.push(&Auth::default()).unwrap();
    let tree = repo.repo.find_commit(Oid::from_str(&commit).unwrap()).unwrap().tree().unwrap();
    for (id, ext, bytes) in &bodies {
        let path = format!("notes/{id}.{ext}");
        assert_eq!(repo.repo.find_blob(tree.get_path(Path::new(&path)).unwrap().id()).unwrap().content(), bytes);
        assert!(tree.get_path(Path::new(&format!(".quiet/trash/{id}.{ext}"))).is_err());
    }
    assert!(b.git_pull(&Auth::default()).unwrap()); assert!(b.list().is_empty()); assert_eq!(b.list_trash()[0].id, folder);
    assert_eq!(b.load_trash(&xml).unwrap().title.text, "XML");
    assert_eq!(b.load_trash(&md).unwrap().title.text, "Markdown");
    assert!(!b.load_trash(&md).unwrap().asset_sources().is_empty());
    for id in [&xml, &md] {
        let versions = b.git_versions(id).unwrap(); assert!(!versions.is_empty());
        for version in versions { assert!(b.version_document(id, &version.name).is_ok()); }
    }
    assert!(!b.restore(&folder).unwrap()); assert!(b.list_trash().is_empty());
    for (id, ext, bytes) in &bodies { assert_eq!(std::fs::read(second.0.join(format!("notes/{id}.{ext}"))).unwrap(), *bytes); }
    assert_eq!(b.list().iter().find(|e| e.id == xml).unwrap().meta.parent.as_deref(), Some(nested.as_str()));
    assert_eq!(std::fs::read(second.0.join(".assets/test.png")).unwrap(), b"keep image");
    configure(&second.git(), &server.0); second.git().commit("restore").unwrap(); second.git().push(&Auth::default()).unwrap();
    assert!(a.git_pull(&Auth::default()).unwrap()); assert!(a.list_trash().is_empty());
    for (id, _, bytes) in bodies { assert_eq!(std::fs::read(a.file_path(&id).unwrap()).unwrap(), bytes); }
}

#[test]
fn visible_assets_upgrade_a_format_seven_git_library_and_keep_old_images() {
    let server = Library::new(); Repository::init_bare(&server.0).unwrap();
    let lib = Library::new(); drop(lib.store());
    std::fs::write(lib.0.join(FORMAT_PATH), b"{\"format\":7}").unwrap();
    let mut store = lib.store(); store.enable_git().unwrap();
    let old_path = ".assets/old.png";
    let new_path = "assets/new.png";
    let image = |src: &str| crate::model::Block::new(crate::model::BlockKind::Image {
        src_kind: "path".into(), src: src.into(), width: None, height: None, caption: None, crop: None,
    }, RichText::default());
    std::fs::create_dir(lib.0.join(LEGACY_ASSETS)).unwrap();
    std::fs::write(lib.0.join(old_path), b"old image").unwrap();
    let mut document = doc("Images"); document.blocks = vec![image(old_path)];
    let id = store.create_from(&document).unwrap();
    let repo = lib.git(); configure(&repo, &server.0);
    let old = repo.commit("format 7").unwrap().unwrap();
    repo.push(&Auth::default()).unwrap();
    let destination = Library::new();
    let clone = LibraryRepo::clone_into(server.0.to_str().unwrap(), &destination.0, &Auth::default()).unwrap();
    let mut cloned = destination.store();
    assert_eq!(Store::format_at(&destination.0), Some(7));
    assert!(clone.status().unwrap().changes.is_empty());

    let assets = Store::assets_in(&lib.0);
    assert_eq!(assets, lib.0.join(ASSETS));
    std::fs::create_dir(&assets).unwrap();
    std::fs::write(lib.0.join(new_path), b"new image").unwrap();
    document.blocks.push(image(new_path));
    store.save(&id, &document).unwrap();
    assert_eq!(Store::format_at(&lib.0), Some(FORMAT));
    assert!(repo.change_preview(new_path).unwrap().binary);
    repo.stage(new_path).unwrap();
    assert!(repo.commit_staged("incomplete upgrade").is_err());
    assert!(repo.discard(FORMAT_PATH).is_err());
    repo.stage(FORMAT_PATH).unwrap();
    assert!(repo.commit_staged("missing LFS rules").is_err());
    repo.stage_all().unwrap();
    let new = repo.commit_staged("format 8").unwrap().unwrap();
    let tree = repo.repo.find_commit(Oid::from_str(&new).unwrap()).unwrap().tree().unwrap();
    assert!(repo.repo.find_blob(tree.get_path(Path::new(".gitattributes")).unwrap().id()).unwrap().content()
        .split(|&byte| byte == b'\n').any(|line| line == VISIBLE_ASSET_ATTRIBUTE.as_bytes()));
    for path in [old_path, new_path] {
        let blob = repo.repo.find_blob(tree.get_path(Path::new(path)).unwrap().id()).unwrap();
        assert!(Pointer::parse(blob.content()).unwrap().is_some());
    }
    repo.push(&Auth::default()).unwrap();
    assert!(cloned.git_pull(&Auth::default()).unwrap());
    assert_eq!(cloned.load(&id).unwrap().asset_sources(), [old_path, new_path]);
    assert_eq!(std::fs::read(destination.0.join(old_path)).unwrap(), b"old image");
    assert_eq!(std::fs::read(destination.0.join(new_path)).unwrap(), b"new image");
    for (commit, expected) in [(old, vec![b"old image"]), (new, vec![b"old image", b"new image"])] {
        let historical = cloned.version_document(&id, &format!("git:0-{commit}")).unwrap();
        let bytes: Vec<_> = historical.asset_sources().iter().map(|path| std::fs::read(destination.0.join(path)).unwrap()).collect();
        assert_eq!(bytes, expected);
    }
}

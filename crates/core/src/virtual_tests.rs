use super::*;
use crate::manifest::Manifest;

struct Library(PathBuf);
impl Library {
    fn new() -> Self { Self(std::env::temp_dir().join(format!("quiet-virtual-test-{}", uuid::Uuid::new_v4()))) }
    fn open(&self) -> Store { Store::open_at(self.0.clone()).unwrap() }
    fn manifest(&self) -> Manifest { Manifest::parse(&std::fs::read(self.0.join("quiet.json")).unwrap()).unwrap() }
}
impl Drop for Library { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
fn doc(title: &str) -> Document { Document { title: crate::model::RichText::plain(title), ..Default::default() } }

#[test]
fn default_library_uses_virtual_folders_and_fixed_body_files_without_git() {
    let lib = Library::new(); let mut s = lib.open();
    assert!(!s.git_enabled());
    let a = s.create_folder("Projects", None).unwrap(); let b = s.create_folder("Nested", Some(&a)).unwrap();
    let id = s.create_from_in(&doc("Title"), Some(&b)).unwrap();
    let md = s.create_markdown_in(&doc("Title"), "---\nkeep: true\n---\n", Some(&b)).unwrap();
    let path = s.file_path(&id).unwrap(); let md_path = s.file_path(&md).unwrap();
    assert_eq!(path, lib.0.join(format!("Notes/{id}.xml")));
    assert_eq!(md_path, lib.0.join(format!("Notes/{md}.md")));
    let bytes = std::fs::read(&path).unwrap(); let stamp = std::fs::metadata(&path).unwrap().modified().unwrap();
    let md_bytes = std::fs::read(&md_path).unwrap();
    s.set_parent(&a, Some(&b)).unwrap_err();
    s.set_parent(&b, None).unwrap(); s.set_parent(&id, Some(&a)).unwrap();
    s.rename(&a, "Renamed folder").unwrap(); s.rename(&id, "New title").unwrap(); s.rename(&md, "New Markdown title").unwrap();
    assert!(!lib.0.join("quiet.json.bak").exists());
    assert_eq!(std::fs::read(&path).unwrap(), bytes); assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), stamp);
    assert_eq!(std::fs::read(&md_path).unwrap(), md_bytes); assert_eq!(s.file_path(&id).unwrap(), path);
    assert!(!lib.0.join("Projects").exists()); assert!(!lib.0.join("Renamed folder").exists());
    let manifest = lib.manifest();
    assert_eq!(manifest.version, 2); assert_eq!(manifest.folders.len(), 2); assert_eq!(manifest.notes.len(), 2);
    let note = manifest.notes.iter().find(|n| n.id == id).unwrap(); assert_eq!(note.folder.as_deref(), Some(a.as_str())); assert_eq!(note.title, "New title");
    let mut reopened = lib.open(); assert_eq!(reopened.load(&id).unwrap().title.text, "New title");
    assert_eq!(reopened.list().iter().find(|e| e.id == md).unwrap().title, "New Markdown title");
}

#[test]
fn virtual_tree_delete_restore_and_undo_preserve_structure_and_bytes() {
    let lib = Library::new(); let mut s = lib.open();
    let root = s.create_folder("Root", None).unwrap(); let child = s.create_folder("Child", Some(&root)).unwrap();
    let id = s.create_from_in(&doc("Nested document"), Some(&child)).unwrap(); let path = s.file_path(&id).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let journal = s.track_file_changes(); s.delete(&root, None).unwrap(); let changes = journal.take();
    assert_eq!(changes.len(), 3); assert!(lib.manifest().notes.is_empty()); assert_eq!(lib.manifest().trash.len(), 3);
    assert!(path.is_file()); assert_eq!(s.list_trash().len(), 1);
    s.apply_file_history(&changes, false).unwrap(); assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(s.index[&id].parent.as_deref(), Some(child.as_str())); assert_eq!(s.index[&child].parent.as_deref(), Some(root.as_str()));
    s.apply_file_history(&changes, true).unwrap(); s.restore(&root).unwrap(); assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(!lib.0.join("quiet.json.bak").exists());
    s.delete(&root, None).unwrap(); assert_eq!(lib.manifest().trash.len(), 3);
    assert!(s.list().is_empty()); assert!(path.is_file());
    assert_eq!(std::fs::read(s.trash_path(&id)).unwrap(), bytes);
}

#[test]
fn trash_folders_open_documents_and_history_without_restoring_or_writing_bodies() {
    let lib = Library::new(); let mut s = lib.open();
    let folder = s.create_folder("Deleted folder", None).unwrap(); let nested = s.create_folder("Nested", Some(&folder)).unwrap();
    let xml = s.create_from_in(&doc("Stored title"), Some(&nested)).unwrap();
    s.rename(&xml, "Manifest title").unwrap();
    let md = s.create_markdown_in(&doc("Markdown title"), "# Heading\n\n![Image](../Images/picture.png)\n", Some(&nested)).unwrap();
    let ids = [&xml, &md];
    let bodies: Vec<_> = ids.iter().map(|id| (s.file_path(id).unwrap(), std::fs::read(s.file_path(id).unwrap()).unwrap())).collect();
    for id in ids { assert!(s.save_version_now(id).unwrap()); }
    s.delete(&folder, None).unwrap();
    let manifest = std::fs::read(lib.0.join("quiet.json")).unwrap();
    let mut reopened = lib.open();
    assert!(reopened.list().is_empty()); assert_eq!(reopened.list_trash()[0].id, folder);
    assert_eq!(reopened.list_trash_in(Some(&folder))[0].id, nested);
    assert_eq!(reopened.list_trash_in(Some(&nested)).len(), 2);
    assert!(reopened.load_trash(&folder).is_err());
    assert_eq!(reopened.load_trash(&xml).unwrap().title.text, "Manifest title");
    let markdown = reopened.load_trash(&md).unwrap();
    assert_eq!(markdown.title.text, "Markdown title"); assert!(markdown.plain_text().contains("Heading"));
    assert!(Path::new(&markdown.asset_sources()[0]).ends_with("Images/picture.png"));
    for id in [&xml, &md] {
        let versions = reopened.versions(id); assert_eq!(versions.len(), 1);
        let name = &versions[0].name;
        assert!(!reopened.version_document(id, name).unwrap().title.text.is_empty());
        assert!(reopened.version_diff(id, name, None).is_ok());
        reopened.set_version_label(id, name, Some("Before deletion")).unwrap();
        assert_eq!(reopened.versions(id)[0].label.as_deref(), Some("Before deletion"));
        assert!(reopened.save_version_now(id).is_err()); assert!(reopened.restore_version(id, name).is_err());
        assert!(reopened.save(id, &doc("Must not write")).is_err());
    }
    assert_eq!(std::fs::read(lib.0.join("quiet.json")).unwrap(), manifest);
    for (path, bytes) in bodies { assert_eq!(std::fs::read(path).unwrap(), bytes); }
    let name = reopened.versions(&md)[0].name.clone(); reopened.delete_version(&md, &name).unwrap();
    assert!(reopened.versions(&md).is_empty());
    reopened.restore(&folder).unwrap();
    assert_eq!(reopened.versions(&xml)[0].label.as_deref(), Some("Before deletion"));
    assert_eq!(reopened.load(&xml).unwrap().title.text, "Manifest title");
}

#[test]
fn metadata_is_authoritative_and_other_handles_reload_it() {
    let lib = Library::new(); let mut a = lib.open(); let mut b = lib.open();
    let id = a.create_from(&doc("Original")).unwrap(); let folder = b.create_folder("Virtual", None).unwrap();
    a.set_parent(&id, Some(&folder)).unwrap(); b.rename(&id, "Metadata title").unwrap();
    let previous_shape = Store::shape_at(&lib.0);
    a.rename(&folder, "Updated virtual").unwrap(); assert_ne!(Store::shape_at(&lib.0), previous_shape);
    assert_eq!(a.list().iter().find(|e| e.id == id).unwrap().title, "Metadata title");
    std::fs::remove_file(a.file_path(&id).unwrap()).unwrap();
    assert!(a.list().iter().any(|e| e.id == id)); assert!(lib.manifest().notes.iter().any(|n| n.id == id));
    assert!(a.load(&id).is_err());
}

#[test]
fn invalid_manifests_are_rejected_and_unknown_fields_survive_writes() {
    let lib = Library::new(); let mut s = lib.open(); let folder = s.create_folder("Folder", None).unwrap();
    let id = s.create_from_in(&doc("Note"), Some(&folder)).unwrap();
    let mut manifest = lib.manifest(); manifest.extra.insert("custom".into(), serde_json::json!({"keep": true}));
    manifest.notes[0].extra.insert("sourcePath".into(), "old/path.md".into());
    let path = lib.0.join("quiet.json");
    let backup = lib.0.join("quiet.json.bak");
    save_json(&path, &manifest).unwrap();
    let previous = std::fs::read(&path).unwrap(); std::fs::write(&backup, &previous).unwrap();
    s.rename(&id, "Edited").unwrap();
    assert_eq!(std::fs::read(&backup).unwrap(), previous);
    let valid = lib.manifest(); assert_eq!(valid.extra["custom"]["keep"], true); assert_eq!(valid.notes[0].extra["sourcePath"], "old/path.md");
    let mut bad = valid.clone(); bad.notes[0].filename = "../outside.xml".into(); assert!(bad.validate().is_err());
    let mut bad = valid.clone(); bad.notes[0].folder = Some(id.clone()); assert!(bad.validate().is_err());
    let mut bad = valid.clone(); bad.folders[0].parent = Some(folder); assert!(bad.validate().is_err());
    let mut bad = valid; bad.notes.push(bad.notes[0].clone()); assert!(bad.validate().is_err());
    let body_path = s.path(&id); let body = std::fs::read(&body_path).unwrap();
    std::fs::write(&path, b"broken JSON").unwrap();
    assert!(s.delete(&id, None).is_err()); assert!(Store::open_at(lib.0.clone()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"broken JSON");
    assert_eq!(std::fs::read(&backup).unwrap(), previous);
    assert_eq!(std::fs::read(&body_path).unwrap(), body);
}

#[test]
fn virtual_markdown_assets_stay_relative_and_git_moves_only_change_manifest() {
    use crate::{model::{Block, BlockKind, RichText}, git::{LibraryRepo, Config}};
    let lib = Library::new(); let mut s = lib.open();
    let shape = Store::shape_at(&lib.0);
    std::fs::create_dir_all(lib.0.join("Images")).unwrap(); std::fs::write(lib.0.join("Images/test.png"), b"picture").unwrap();
    std::fs::create_dir_all(lib.0.join("Files")).unwrap();
    std::fs::write(lib.0.join("Files/attachment.md"), b"attachment").unwrap();
    assert_eq!(Store::shape_at(&lib.0), shape);
    let mut document = doc("Images"); document.blocks = vec![Block::new(BlockKind::Image { src_kind: "path".into(), src: "Images/test.png".into(), width: None, height: None, caption: None, crop: None }, RichText::default())];
    let id = s.create_markdown(&document, "").unwrap(); let path = s.file_path(&id).unwrap();
    assert!(std::fs::read_to_string(&path).unwrap().contains("../Images/test.png"));
    assert_eq!(s.load(&id).unwrap().asset_sources(), vec![lib.0.join("Images/test.png").to_string_lossy().into_owned()]);
    let loaded = s.load_for_edit(&id, false).unwrap(); s.save(&id, &loaded).unwrap();
    assert!(!std::fs::read_to_string(&path).unwrap().contains(lib.0.to_str().unwrap()));
    let folder = s.create_folder("Virtual", None).unwrap(); s.enable_git().unwrap();
    let repo = LibraryRepo::open(&lib.0).unwrap(); repo.configure(&Config { name: "Test".into(), email: "test@example.invalid".into(), ..Default::default() }).unwrap();
    repo.commit("initial").unwrap(); let bytes = std::fs::read(&path).unwrap();
    s.set_parent(&id, Some(&folder)).unwrap(); s.rename(&folder, "Changed").unwrap(); s.rename(&id, "New title").unwrap();
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    let changes = repo.status().unwrap().changes; assert_eq!(changes.len(), 1); assert_eq!(changes[0].path, "quiet.json");
    repo.commit("metadata").unwrap(); assert_eq!(s.git_versions(&id).unwrap().len(), 2);
}

#[test]
fn metadata_title_conflicts_do_not_overwrite_another_instances_rename() {
    for markdown in [false, true] {
        let lib = Library::new(); let mut a = lib.open();
        let id = if markdown { a.create_markdown(&doc("Old"), "").unwrap() } else { a.create_from(&doc("Old")).unwrap() };
        let loaded = a.load_for_edit(&id, false).unwrap(); let mut b = lib.open(); b.rename(&id, "Other title").unwrap();
        assert!(a.save(&id, &loaded).unwrap_err().to_string().contains("标题"));
        assert_eq!(lib.open().load(&id).unwrap().title.text, "Other title");
        let fresh = a.load_for_edit(&id, false).unwrap(); a.save(&id, &fresh).unwrap();
        a.rename(&id, "Same app").unwrap(); a.accept_file_title(&id, "Same app").unwrap();
        a.save(&id, &doc("Same app")).unwrap();
    }
}

#[test]
fn batch_creation_deletion_and_folder_move_undo_keep_virtual_paths() {
    let lib = Library::new(); let mut s = lib.open();
    let (a, b, id) = s.batch(|s| {
        let a = s.create_folder("A", None).unwrap(); let b = s.create_folder("B", Some(&a)).unwrap();
        let id = s.create_from_in(&doc("Nested"), Some(&b)).unwrap();
        let removed = s.create_from(&doc("Remove in batch")).unwrap(); s.delete(&removed, None).unwrap();
        (a, b, id)
    }).unwrap();
    assert_eq!(lib.manifest().notes.len(), 1); assert_eq!(lib.manifest().trash.len(), 1);
    let path = s.file_path(&id).unwrap(); let bytes = std::fs::read(&path).unwrap();
    let journal = s.track_file_changes(); s.set_parent(&b, None).unwrap(); let changes = journal.take();
    s.apply_file_history(&changes, false).unwrap(); assert_eq!(s.index[&b].parent.as_deref(), Some(a.as_str()));
    s.apply_file_history(&changes, true).unwrap(); assert!(s.index[&b].parent.is_none());
    assert_eq!(s.file_path(&id).unwrap(), path); assert_eq!(std::fs::read(path).unwrap(), bytes);
    assert_eq!(lib.open().list().len(), 3);
}

#[test]
fn deeply_nested_virtual_folders_restore_in_parent_order() {
    let lib = Library::new(); let mut s = lib.open();
    let chain = s.batch(|s| {
        let mut chain = vec![];
        for i in 0..70 { chain.push(s.create_folder(&format!("Level {i}"), chain.last().map(String::as_str)).unwrap()); }
        chain
    }).unwrap();
    s.delete(&chain[0], None).unwrap(); s.restore(&chain[0]).unwrap();
    for pair in chain.windows(2) { assert_eq!(s.index[&pair[1]].parent.as_deref(), Some(pair[0].as_str())); }
    assert_eq!(lib.manifest().folders.len(), 70);
}

#[test]
fn virtual_library_keeps_local_file_names_and_reuses_its_body_cache() {
    let lib = Library::new(); let mut s = lib.open(); let id = s.create_from(&doc("Cached note")).unwrap(); s.list();
    let mut reopened = lib.open(); assert!(reopened.parsed.contains_key(&id));
    let path = lib.0.join("External.md"); std::fs::write(&path, "# Body\n").unwrap();
    let local = reopened.open_local(&path).unwrap(); reopened.list(); reopened.rename(&local, "Renamed file").unwrap();
    assert_eq!(reopened.list().iter().find(|e| e.id == local).unwrap().title, "Renamed file");
    assert!(lib.0.join("Renamed file.md").is_file()); assert_eq!(lib.manifest().notes.len(), 1);
}

#[test]
fn trash_keeps_bodies_metadata_and_history_until_restored() {
    let lib = Library::new(); let mut s = lib.open();
    let root = s.create_folder("Root", None).unwrap(); let child = s.create_folder("Child", Some(&root)).unwrap();
    let xml = s.create_from_in(&doc("XML"), Some(&child)).unwrap();
    let md = s.create_markdown_in(&doc("Markdown"), "# Markdown\n\nKeep **everything**.\n", Some(&child)).unwrap();
    s.save_version_now(&xml).unwrap();
    let bodies: Vec<_> = [&xml, &md].into_iter().map(|id| (id.clone(), s.file_path(id).unwrap(), std::fs::read(s.file_path(id).unwrap()).unwrap())).collect();
    s.delete(&root, None).unwrap();
    let mut manifest = lib.manifest(); for t in &mut manifest.trash { t.deleted = 1; }
    save_json(&lib.0.join("quiet.json"), &manifest).unwrap();
    let metadata = std::fs::read(lib.0.join("quiet.json")).unwrap();
    let mut s = lib.open();
    for _ in 0..2 {
        let trash = s.list_trash(); assert_eq!(trash.len(), 1); assert_eq!(trash[0].id, root); assert_eq!(trash[0].deleted, 1);
        assert!(s.list().is_empty()); assert_eq!(std::fs::read(lib.0.join("quiet.json")).unwrap(), metadata);
        for (id, path, bytes) in &bodies { assert!(path.is_file()); assert_eq!(std::fs::read(s.trash_path(id)).unwrap(), *bytes); }
    }
    assert_eq!(crate::versions::list(&s.history_base(), &xml, true).len(), 1);
    assert!(!s.restore(&root).unwrap()); assert!(s.list_trash().is_empty());
    for (id, path, bytes) in bodies { assert_eq!(std::fs::read(path).unwrap(), bytes); assert_eq!(s.index[&id].parent.as_deref(), Some(child.as_str())); }
    assert_eq!(s.index[&child].parent.as_deref(), Some(root.as_str())); assert_eq!(s.versions(&xml).len(), 1);
}

#[test]
fn trash_changes_only_metadata_and_never_readopts_bodies() {
    for markdown in [false, true] {
        let lib = Library::new(); let mut a = lib.open(); let mut b = lib.open();
        let id = if markdown { a.create_markdown(&doc("Markdown"), "# Keep body\n").unwrap() } else { a.create_from(&doc("XML")).unwrap() };
        let path = a.file_path(&id).unwrap(); let bytes = std::fs::read(&path).unwrap();
        let metadata = std::fs::metadata(&path).unwrap();
        let check_body = || {
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
            assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), metadata.modified().unwrap());
            #[cfg(unix)] { use std::os::unix::fs::MetadataExt; assert_eq!(std::fs::metadata(&path).unwrap().ino(), metadata.ino()); }
        };
        a.delete(&id, None).unwrap(); check_body();
        let manifest = lib.manifest(); assert!(manifest.notes.is_empty());
        assert_eq!(manifest.trash[0].body_rel().unwrap(), format!("Notes/{}.{}", id, if markdown { "md" } else { "xml" }));
        assert!(!lib.0.join(".quiet/trash").exists());
        assert!(a.list().is_empty()); assert!(b.list().is_empty()); assert_eq!(b.list_trash()[0].id, id);
        let mut reopened = lib.open(); assert!(reopened.list().is_empty()); assert_eq!(reopened.list_trash()[0].id, id); check_body();
        reopened.restore(&id).unwrap(); check_body();
        assert!(a.list_trash().is_empty()); assert_eq!(b.list()[0].id, id);
    }
}



#[test]
fn invalid_manifest_versions_are_refused_without_changing_the_library() {
    let lib = Library::new(); let mut store = lib.open();
    let id = store.create_from(&doc("Original")).unwrap();
    let body = std::fs::read(store.path(&id)).unwrap();
    let path = lib.0.join(FILE);
    let mut manifest = lib.manifest();
    for version in [0, 1, crate::manifest::VERSION + 1] {
        manifest.version = version;
        let bytes = serde_json::to_vec(&manifest).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let error = Store::open_library_at(lib.0.clone()).err().unwrap();
        assert!(format!("{error:#}").contains("不支持的文档库清单版本"));
        assert!(store.save(&id, &doc("Changed")).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(std::fs::read(store.path(&id)).unwrap(), body);
        assert!(!lib.0.join(".git").exists());
    }

    std::fs::remove_file(&path).unwrap();
    assert!(Store::open_at(lib.0.clone()).is_err());
    assert!(!path.exists());
    assert_eq!(std::fs::read(store.path(&id)).unwrap(), body);
}

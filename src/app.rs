//! Workspace window: two panes — the document list on the left
//! (see `sidebar.rs`) and the editor on the right, with the
//! 64px top bar (breadcrumb title + save status) above the page.

use crate::docinfo::DocInfo;
use crate::docmenu::DocSub;
use crate::editor::view::{Editor, EditorEvent};
use crate::input::{TextField, TextFieldEvent};
use crate::settings::display_title;
use crate::store::Entry;
use crate::shared_store::SharedStore;
use crate::theme::{self, Theme, craft};
use crate::ui::{TipExt, TipSide, TitleBarExt, ToastKind, icon, ui_text};
use gpui::{prelude::FluentBuilder, *};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Autosave waits for this pause in editing, but never longer than
/// SAVE_MAX_DELAY after the oldest change not yet on disk.
const SAVE_DELAY: Duration = Duration::from_millis(500);
const SAVE_MAX_DELAY: Duration = Duration::from_secs(2);

actions!(workspace, [NewDoc, OpenFile, ImportDoc, ImportFolder, ToggleSidebar, FocusSearch, FindInDoc, ReplaceInDoc, OpenSettings, OpenAbout, GoHome, CloseWindow, Quit,
    ReplayOnboarding, HideApp, HideOthers, ShowAll, Minimize, Zoom, ToggleFullScreen, NewTab, CloseTab, NextTab, PreviousTab]);

/// The menu bar, in the interface language.
pub fn set_menus(cx: &mut App) {
    use crate::settings::tr;
    cx.set_menus(vec![
        Menu {
            name: "Quiet.md".into(),
            items: vec![
                MenuItem::action(tr("关于 Quiet.md", "About Quiet.md"), OpenAbout),
                MenuItem::separator(),
                MenuItem::action(tr("设置…", "Settings…"), OpenSettings),
                MenuItem::separator(),
                MenuItem::os_submenu(tr("服务", "Services"), SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action(tr("隐藏 Quiet.md", "Hide Quiet.md"), HideApp),
                MenuItem::action(tr("隐藏其他", "Hide Others"), HideOthers),
                MenuItem::action(tr("全部显示", "Show All"), ShowAll),
                MenuItem::separator(),
                MenuItem::action(tr("退出 Quiet.md", "Quit Quiet.md"), Quit),
            ],
        },
        Menu {
            name: tr("文件", "File").into(),
            items: vec![
                MenuItem::action(tr("新建文档", "New Document"), NewDoc),
                MenuItem::action(tr("新建笔记窗口", "New Note Window"), crate::quick_note::QuickNew),
                MenuItem::action(tr("打开 Markdown 文件…", "Open Markdown File…"), OpenFile),
                MenuItem::action(tr("导入为文档…", "Import as Document…"), ImportDoc),
                MenuItem::action(tr("导入文件夹…", "Import Folder…"), ImportFolder),
                MenuItem::separator(),
                MenuItem::action(tr("新建标签页", "New Tab"), NewTab),
                MenuItem::action(tr("关闭标签页", "Close Tab"), CloseTab),
                MenuItem::action(tr("关闭窗口", "Close Window"), CloseWindow),
            ],
        },
        // The standard Edit menu: its items act on the focused editor, and
        // macOS adds 听写 and 表情与符号 to it.
        Menu {
            name: tr("编辑", "Edit").into(),
            items: {
                use crate::editor::keymap as k;
                vec![
                    MenuItem::os_action(tr("撤销", "Undo"), k::Undo, OsAction::Undo),
                    MenuItem::os_action(tr("重做", "Redo"), k::Redo, OsAction::Redo),
                    MenuItem::separator(),
                    MenuItem::os_action(tr("剪切", "Cut"), k::Cut, OsAction::Cut),
                    MenuItem::os_action(tr("拷贝", "Copy"), k::Copy, OsAction::Copy),
                    MenuItem::action(tr("拷贝为 Markdown", "Copy as Markdown"), k::CopyMarkdown),
                    MenuItem::os_action(tr("粘贴", "Paste"), k::Paste, OsAction::Paste),
                    MenuItem::os_action(tr("全选", "Select All"), k::SelectAll, OsAction::SelectAll),
                ]
            },
        },
        Menu {
            name: tr("窗口", "Window").into(),
            items: vec![
                MenuItem::action(tr("最小化", "Minimize"), Minimize),
                MenuItem::action(tr("缩放", "Zoom"), Zoom),
                MenuItem::separator(),
                MenuItem::action(tr("进入 / 退出全屏幕", "Enter / Exit Full Screen"), ToggleFullScreen),
            ],
        },
    ]);
}

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-z", crate::editor::keymap::Undo, Some("Workspace")),
        KeyBinding::new("secondary-shift-z", crate::editor::keymap::Redo, Some("Workspace")),
        KeyBinding::new("secondary-y", crate::editor::keymap::Redo, Some("Workspace")),
        KeyBinding::new("cmd-n", NewDoc, None),
        KeyBinding::new("cmd-shift-n", crate::quick_note::QuickNew, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        // For trying the first-launch pages; not listed in Settings › Shortcuts.
        KeyBinding::new("cmd-alt-shift-o", ReplayOnboarding, None),
        KeyBinding::new("cmd-shift-i", ImportDoc, None),
        KeyBinding::new("cmd-o", OpenFile, None),
        KeyBinding::new("cmd-alt-home", GoHome, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-t", NewTab, Some("Workspace")),
        KeyBinding::new("cmd-shift-w", CloseTab, Some("Workspace")),
        KeyBinding::new("ctrl-tab", NextTab, Some("Workspace")),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, Some("Workspace")),
        KeyBinding::new("cmd-]", ToggleSidebar, None),
        KeyBinding::new("cmd-\\", ToggleSidebar, None),
        KeyBinding::new("cmd-j", FocusSearch, None),
        KeyBinding::new("cmd-f", FindInDoc, None),
        KeyBinding::new("cmd-shift-h", ReplaceInDoc, None),
        KeyBinding::new("cmd-q", Quit, None),
    ]);
    // macOS app and window keys. ⌥⌘H (隐藏其他) stays with the editor's
    // 应用上次的颜色, the Feishu shortcut; 隐藏其他 is in the menu. On Windows
    // the system handles these (Win+↓, Alt+F4), and F11 is full screen.
    #[cfg(target_os = "macos")]
    cx.bind_keys([
        KeyBinding::new("cmd-h", HideApp, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
    ]);
    // Windows: F11 for full screen, and Ctrl+H for 查找和替换 as in Office
    // and Notepad (written cmd-h: `shortcuts::pc_binding` maps cmd to
    // ctrl and drops a literal ctrl-h, Cocoa's backspace).
    #[cfg(not(target_os = "macos"))]
    cx.bind_keys([KeyBinding::new("f11", ToggleFullScreen, None), KeyBinding::new("cmd-h", ReplaceInDoc, None)]);
    cx.on_action(|_: &HideApp, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &CloseWindow, cx| {
        if let Some(handle) = cx.active_window().and_then(|w| w.downcast::<Workspace>()) {
            // Key dispatch already borrows this window; close after it returns.
            cx.defer(move |cx| {
                if let Err(e) = handle.update(cx, |ws, window, cx| {
                    crate::quick_note::persist(cx);
                    if ws.save_before(cx, |_, window, _| window.remove_window()) { window.remove_window(); }
                }) { eprintln!("无法关闭笔记窗口：{e}"); }
            });
        }
    });
    cx.on_action(|_: &Quit, cx| cx.defer(quit));
    crate::modes::bind_keys(cx);
}

/// 退出: the open document is saved first; when that fails its dialog asks
/// (重试 / 放弃修改) and quits afterwards. Call it outside any window update
/// (`cx.defer`): a window on the update stack cannot be asked to save.
pub fn quit(cx: &mut App) {
    let ready = cx
        .windows()
        .into_iter()
        .filter_map(|w| w.downcast::<Workspace>())
        .map(|w| w.update(cx, |ws, _, cx| ws.save_before(cx, |_, _, cx| cx.defer(quit))).unwrap_or(false))
        .fold(true, |ready, saved| ready & saved);
    if ready { cx.quit(); }
}

/// 更改文档库: open documents are saved first (as for 退出), the folder is
/// set up as a library if it isn't one, the current preferences go with
/// it when it has none, and the app restarts into it. The old library
/// stays where it is. Call it outside any window update (`cx.defer`).
pub fn switch_library(root: std::path::PathBuf, cx: &mut App) {
    let ready = cx
        .windows()
        .into_iter()
        .filter_map(|w| w.downcast::<Workspace>())
        .map(|w| { let again = root.clone(); w.update(cx, |ws, _, cx| ws.save_before(cx, move |_, _, cx| cx.defer(move |cx| switch_library(again, cx)))).unwrap_or(false) })
        .fold(true, |ready, saved| ready & saved);
    if !ready { return }
    let prefs = crate::store::Store::internal_dir().join("preferences.json");
    if let Err(e) = crate::store::Store::set_root_dir(root) {
        let msg = format!("{}{}", crate::settings::tr("无法在这里创建文档库：", "Couldn't create the library here: "), crate::settings::diagnostic(&e.to_string()));
        if let Some(ws) = cx.windows().into_iter().find_map(|w| w.downcast::<Workspace>()) { let _ = ws.update(cx, |w, _, cx| w.show_toast(&msg, cx)); }
        return;
    }
    let mine = crate::store::Store::internal_dir().join("preferences.json");
    if !mine.exists() { let _ = std::fs::create_dir_all(mine.parent().unwrap()).and_then(|_| std::fs::copy(&prefs, &mine)); }
    cx.restart();
}

#[derive(Clone, Debug, PartialEq)]
pub enum SaveStatus {
    /// Freshly opened, nothing saved this session: show the modification time.
    Idle,
    Saved,
    Saving,
    Failed(String),
}

/// Right-click / "…" menu on a sidebar row.
#[derive(Clone, Debug)]
pub struct RowMenu {
    pub id: String,
    pub pos: Point<Pixels>,
    /// Which corner of the menu sits at `pos`.
    pub corner: Corner,
    /// 下载 submenu open.
    pub sub: bool,
    /// The 批量操作 menu for the checked rows.
    pub batch: bool,
}

/// What the main pane shows: the open document, 主页 or 回收站.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Doc,
    NewTab,
    Home,
    Folder,
    Trash,
    Settings,
    Git,
}

/// A confirmation dialog and the pages it is about.
#[derive(Clone, Debug)]
pub enum Confirm {
    /// 删除 (to the 回收站); `batch` for the 批量操作 selection.
    Delete { ids: Vec<String>, batch: bool },
    /// 回收站 → 恢复.
    Restore(String),
    GitDiscard(String),
    /// 隐藏引用列表 (with 不再提示) / 显示引用列表.
    Refs { show: bool, no_ask: bool },
    /// The open document could not be written before it was to be replaced
    /// (another document, 新建, 退出): 重试 / 放弃修改, then carry on.
    SaveFailed,
    /// The library should move to real folders, and the user is asked first
    /// (it is synced, or used from another computer): 改用文件夹 / 稍后.
    /// 设置 › 文档库 › 更改…: open this folder as the library (#94), after a restart.
    SwitchLibrary(std::path::PathBuf),
    /// Dropped on the sidebar from outside: import it into the library there?
    Import { what: DropImport, target: crate::sidebar_drop::SidebarDropAt },
}

/// What a sidebar drop would import: files and folders from the Finder, or a 本机文件.
#[derive(Clone, Debug)]
pub enum DropImport { Files(Vec<std::path::PathBuf>), Local(String) }

/// What to do once the open document is safely on disk (or its changes given up).
pub type AfterSave = Box<dyn FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>)>;

#[derive(Default)]
struct ListedDocs { entries: Vec<crate::store::Entry>, trash: Vec<crate::store::TrashEntry>, warnings: Vec<String> }
struct DeletedDocs { deleted: Vec<String>, errors: Vec<String>, changes: Vec<crate::store::FileDelta> }

/// Inline 重命名 of a sidebar row.
pub struct Rename {
    pub id: String,
    pub field: Entity<TextField>,
    _subs: Vec<Subscription>,
}

pub struct Workspace {
    pub quick_note: bool,
    pub shortcut_recording: Option<String>,
    pub shortcut_strokes: Vec<String>,
    /// The failed change: (action, message); an empty action is page-wide.
    pub shortcut_error: Option<(String, String)>,
    pub settings_page: crate::settings::Page,
    /// 版本历史: snapshots kept per document (1–30), cached for the settings page.
    pub history_limit: u32,
    pub(crate) git: Option<crate::git::GitPanel>,
    /// 外观's theme dropdown while open: the theme its preview shows.
    pub theme_menu: Option<String>,
    /// Where the theme dropdown's button was drawn last frame.
    pub theme_anchor: std::rc::Rc<std::cell::Cell<Option<Bounds<Pixels>>>>,
    /// 通用's language dropdown while open: the highlighted language.
    pub lang_menu: Option<crate::settings::Lang>,
    pub lang_anchor: std::rc::Rc<std::cell::Cell<Option<Bounds<Pixels>>>>,
    /// 通用's other dropdowns (快捷新建, 网页搜索, 打开链接): the open one, and
    /// where each button was drawn last frame.
    pub pref_menu: Option<crate::settings::PrefMenu>,
    pub pref_anchors: std::rc::Rc<std::cell::RefCell<std::collections::HashMap<&'static str, Bounds<Pixels>>>>,
    /// 快捷键's filter field.
    pub settings_filter: Entity<TextField>,
    native_sidebar: crate::native_header::NativeSidebar,
    pub store: SharedStore,
    pub entries: Vec<Entry>,
    pub current: Option<String>,
    pub(crate) tabs: crate::tabs::Tabs,
    pub(crate) trash_document: bool,
    pub(crate) trash_folder: Option<String>,
    pub editor: Option<Entity<Editor>>,
    pub search: Entity<TextField>,
    /// Keyboard highlight inside search results.
    pub search_sel: Option<usize>,
    pub sidebar_open: bool,
    /// Dragged width (300 by default, 268 to min(800, window − 540)).
    pub sidebar_width: f32,
    /// The resize handle is being dragged.
    pub sidebar_drag: bool,
    /// The window's minimum width as last given to it (`DOC_MIN_W` plus the sidebar).
    min_w: f32,
    pub folder_current: Option<String>,
    pub create_menu: Option<(Option<String>, Point<Pixels>)>,
    /// The header's 导入 menu: the folder it imports into, and where it hangs.
    pub import_menu: Option<(Option<String>, Point<Pixels>)>,
    /// The header's … menu on Home (None) or a folder page: where it hangs.
    pub place_menu: Option<(Option<String>, Point<Pixels>)>,
    pub folder_drag: crate::sidebar_drag::SidebarDrag,
    /// The search dialog (command-center style) is open.
    pub search_open: bool,
    pub status: SaveStatus,
    pub row_menu: Option<RowMenu>,
    pub rename: Option<Rename>,
    pub(crate) file_history: crate::file_history::FileHistory,
    /// Last-frame bounds of each row's "…" button, to hang its menu from.
    pub more_bounds: Rc<RefCell<HashMap<String, Bounds<Pixels>>>>,
    /// Header "…" menu and its open submenu.
    pub doc_menu: bool,
    pub doc_sub: Option<DocSub>,
    /// Last-frame bounds of the open submenu (clicks there keep the menu open).
    pub doc_sub_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    pub doc_info: Option<DocInfo>,
    /// 版本历史 drawer, open for `HistoryView::id`.
    pub history: Option<crate::history::HistoryView>,
    /// A snapshot is on screen: the editor state to put back, and which one.
    pub preview: Option<crate::history::Preview>,
    /// 命名版本: the row's name being typed.
    pub version_rename: Option<crate::history::VersionRename>,
    pub list_scroll: ScrollHandle,
    pub search_scroll: ListState,
    pub(crate) search_cache: RefCell<crate::search::SearchCache>,
    /// Last-frame bounds of the row menu's 下载 submenu.
    pub row_sub_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// 批量操作 mode: the checked docs of 我的文档, in check order.
    pub batch: Option<Vec<String>>,
    /// The open confirmation dialog.
    pub confirm: Option<Confirm>,
    /// The first-launch pages laid over the window again (⌘⌥⇧O, Settings › About).
    pub onboarding: Option<Entity<crate::onboarding::Onboarding>>,
    pub view: View,
    pub home: crate::home::Home,
    pub settings_scroll: ScrollHandle,
    /// 回收站 rows, refreshed when the page opens or changes.
    pub trash: Vec<crate::store::TrashEntry>,
    /// A 回收站 row's menu: the page and where it opens.
    pub trash_menu: Option<(String, Point<Pixels>, Corner)>,
    /// The toast and when it appeared.
    pub toast: Option<(ToastKind, SharedString, Instant)>,
    toast_task: Option<Task<()>>,
    /// Folded sidebar sections: 置顶, 我的文档.
    pub folded: [bool; 3],
    /// Expanded pages of the 我的文档 tree.
    pub expanded: HashSet<String>,
    /// Where a drag from outside the tree (the Finder, 本机文件) would land in the sidebar.
    pub sidebar_drop: Option<crate::sidebar_drop::SidebarDropAt>,
    pub move_dlg: Option<crate::tree::MoveDialog>,
    /// 导出 Markdown 设置.
    pub export_md: Option<crate::transfer::ExportMd>,
    /// 编辑 / 阅读 dropdown, and the mode button's last-frame bounds.
    pub mode_menu: bool,
    pub mode_btn_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The header "…" button, which the doc menu hangs from.
    pub more_btn_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The 快捷键 drawer, and the help button's menu (open while hovered).
    pub shortcuts: bool,
    pub(crate) focus: FocusHandle,
    save_task: Option<Task<()>>,
    /// When the oldest edit not yet on disk was made: a save waits for a
    /// pause in typing, but never longer than `SAVE_MAX_DELAY` after it.
    dirty_since: Option<Instant>,
    /// The open document has changes not yet on disk (cleared only by a
    /// successful save or by 放弃修改).
    pub(crate) unsaved: bool,
    /// Held back by a failed save: runs after 重试 succeeds or 放弃修改.
    pub(crate) after_save: Option<AfterSave>,
    after_drawing: Option<AfterSave>,
    pub(crate) editor_sub: Option<Subscription>,
    _subs: Vec<Subscription>,
}


/// A macOS notification (only logged when scripted).
fn notify_system(title: &str, body: &str) {
    if crate::devtools::scripted() {
        return eprintln!("notification: {title} / {body}");
    }
    let q = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("display notification \"{}\" with title \"{}\"", q(body), q(title));
    std::thread::spawn(move || std::process::Command::new("osascript").arg("-e").arg(script).status());
}

impl Workspace {
    /// Other active windows take the lists a background write left; `trash`
    /// only when that write touched it.
    pub(crate) fn sync_peers(&self, cx: &mut Context<Self>, entries: &[Entry], trash: Option<&[crate::store::TrashEntry]>) {
        let me = cx.entity_id();
        for handle in cx.windows().into_iter().filter_map(|w| w.downcast::<Workspace>()) {
            if !handle.is_active(cx).is_some_and(|yes| yes) || handle.entity(cx).is_ok_and(|e| e.entity_id() == me) { continue }
            let (entries, trash) = (entries.to_vec(), trash.map(<[_]>::to_vec));
            let _ = handle.update(cx, |ws, _, cx| {
                ws.entries = entries; if let Some(trash) = trash { ws.trash = trash; } ws.prune_tabs(); ws.sort();
                cx.notify();
            });
        }
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_window(false, window, cx)
    }

    pub fn new_window(quick: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::build_window(quick, None, window, cx)
    }

    pub fn new_document_window(document: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::build_window(true, document, window, cx)
    }

    fn build_window(quick: bool, document: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        crate::settings::load();
        if !cx.has_global::<SharedStore>() { cx.set_global(SharedStore::open().expect("open document library")); }
        let mut store = cx.global::<SharedStore>().clone();
        let entries = store.list();
        let search = cx.new(|cx| {
            let mut f = TextField::new("搜索", cx);
            f.placeholder_color = Theme::text_disabled();
            f
        });
        let settings_filter = cx.new(|cx| {
            let mut f = TextField::new(crate::settings::tr("搜索快捷键", "Search shortcuts"), cx);
            f.placeholder_color = Theme::text_disabled();
            f
        });
        let subs = vec![
            cx.subscribe(&settings_filter, |_, _, ev: &TextFieldEvent, cx| if matches!(ev, TextFieldEvent::Changed) { cx.notify(); }),
            cx.observe_window_bounds(window, |this, window, cx| crate::quick_note::remember(window, this.quick_note, cx)),
            // The system switched between light and dark (or the app's own look changed).
            cx.observe_window_appearance(window, |this, _, cx| if crate::settings::sync_dark(cx) { crate::settings::refresh_all(this, cx); }),
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    if !this.trash_document && let Some(id) = &this.current { crate::quick_note::used(id, cx); }
                    if !this.store.file_task_busy() { this.entries = this.store.list(); this.prune_tabs(); this.sort(); }
                    cx.notify();
                }
                else if !this.store.file_task_busy() { this.end_folder_drag(true, cx); this.cancel_shortcut_recording(cx); this.flush(cx); }
            }),
            cx.subscribe_in(&search, window, |this, _, ev: &TextFieldEvent, window, cx| {
                match ev {
                    TextFieldEvent::Changed => {
                        this.search_sel = this.default_search_sel(cx);
                        this.search_scroll.scroll_to(ListOffset::default());
                    }
                    TextFieldEvent::Up => this.move_search_sel(-1, cx),
                    TextFieldEvent::Down => this.move_search_sel(1, cx),
                    TextFieldEvent::Submit => {
                        if let Some(ix) = this.search_sel {
                            this.pick_search(ix, false, window, cx)
                        }
                    }
                    TextFieldEvent::Cancel => this.close_search(window, cx),
                    _ => {}
                }
                cx.notify();
            }),
            // Last resort (the system logging out): ⌘Q and the close button
            // go through `save_before` and ask when the save fails.
            cx.on_app_quit(|this, cx| {
                crate::quick_note::persist(cx);
                if !this.store.file_task_busy() && !this.flush(cx) && let Some(ed) = &this.editor
                    && let Err(error) = this.store.recover_later(&ed.read(cx).core.doc) {
                    crate::logging::write(&format!("Could not preserve unsaved document: {error:#}"));
                }
                async {}
            }),
        ];
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |_, cx| {
            crate::quick_note::persist(cx);
            this.update(cx, |w, cx| { w.cancel_shortcut_recording(cx); w.save_before(cx, |_, window, _| window.remove_window()) }).unwrap_or(true)
        });
        if !quick { crate::settings::watch_themes(cx); }
        // A file written into the library from outside (a model writing a
        // note, say) joins it without going away and back: the library's
        // shape is compared every couple of seconds and the list re-read only
        // when it changed.
        cx.spawn(async move |this, cx| {
            let bg = cx.background_executor().clone();
            let shape = || bg.spawn(async { crate::store::Store::shape_at(&crate::store::Store::root_dir()) });
            let mut last = shape().await;
            loop {
                bg.timer(Duration::from_secs(2)).await;
                if this.upgrade().is_none() { break }
                let now = shape().await;
                if now == last { continue }
                let reloaded = this.update(cx, |w, cx| {
                    // Not mid-rename: the row under the field would be rebuilt.
                    if w.rename.is_some() || w.move_dlg.is_some() || w.store.file_task_busy() { return false }
                    w.entries = w.store.list();
                    w.prune_tabs();
                    if w.view == View::Trash { w.trash = w.store.list_trash_in(w.trash_folder.as_deref()); }
                    w.sort();
                    cx.notify();
                    true
                });
                match reloaded { Ok(true) => last = now, Ok(false) => {}, Err(_) => break }
            }
        })
        .detach();
        // 日期提醒: post what fell due since the last look (not what was
        // missed while the app was closed).
        cx.spawn(async |this, cx| {
            let mut last = crate::time::now_ms() as i64;
            loop {
                cx.background_executor().timer(Duration::from_secs(15)).await;
                let now = crate::time::now_ms() as i64;
                if this.update(cx, |w, cx| w.check_reminders(last, now, cx)).is_err() {
                    break;
                }
                last = now;
            }
        })
        .detach();
        let settings = store.settings();
        let sidebar_width = settings.sidebar_width.unwrap_or(theme::SIDEBAR_WIDTH);
        let history_limit = settings.history_limit();
        let mut ws = Workspace {
            quick_note: quick,
            shortcut_recording: None,
            shortcut_strokes: vec![],
            shortcut_error: None,
            settings_page: Default::default(),
            history_limit,
            git: None,
            theme_menu: None,
            theme_anchor: Default::default(),
            lang_menu: None,
            lang_anchor: Default::default(),
            pref_menu: None,
            pref_anchors: Default::default(),
            settings_filter,
            store,
            entries,
            current: None,
            tabs: Default::default(),
            trash_document: false,
            trash_folder: None,
            editor: None,
            search,
            search_sel: None,
            sidebar_open: !quick,
            sidebar_width,
            min_w: 0.,
            sidebar_drag: false,
            folder_current: None,
            create_menu: None,
            import_menu: None,
            place_menu: None,
            folder_drag: Default::default(),
            search_open: false,
            status: SaveStatus::Idle,
            row_menu: None,
            rename: None,
            file_history: Default::default(),
            more_bounds: Default::default(),
            doc_menu: false,
            doc_sub: None,
            doc_sub_bounds: Rc::new(Cell::new(None)),
            doc_info: None,
            history: None,
            preview: None,
            version_rename: None,
            list_scroll: ScrollHandle::new(),
            search_scroll: ListState::new(0, ListAlignment::Top, px(150.)),
            search_cache: Default::default(),
            row_sub_bounds: Rc::new(Cell::new(None)),
            batch: None,
            confirm: None,
            onboarding: None,
            view: View::Doc,
            home: Default::default(),
            settings_scroll: ScrollHandle::new(),
            trash: vec![],
            trash_menu: None,
            toast: None,
            toast_task: None,
            mode_menu: false,
            mode_btn_bounds: Rc::new(Cell::new(None)),
            more_btn_bounds: Rc::new(Cell::new(None)),
            native_sidebar: Default::default(),
            shortcuts: false,
            folded: [false; 3],
            expanded: HashSet::new(),
            sidebar_drop: None,
            move_dlg: None,
            export_md: None,
            focus: cx.focus_handle(),
            save_task: None,
            dirty_since: None,
            unsaved: false,
            after_save: None,
            after_drawing: None,
            editor_sub: None,
            _subs: subs,
        };
        ws.sort();
        ws.load_home();
        window.focus(&ws.focus);
        // An empty library opens on 主页's empty state; only a quick note
        // window starts a document of its own.
        match document.or_else(|| (!quick).then(|| ws.entries.first().map(|e| e.id.clone())).flatten()) {
            Some(id) => ws.open(&id, window, cx),
            None if quick => ws.new_doc(window, cx),
            None => ws.view = View::Home,
        }
        // Kept in the top bar (not just a toast): the user should notice.
        let warnings = ws.store.take_warnings();
        if !warnings.is_empty() {
            let msg = warnings.iter().map(|s| crate::settings::diagnostic(s)).collect::<Vec<_>>().join(crate::settings::tr("；", "; "));
            ws.show_toast_kind(ToastKind::Error, &msg, cx);
            ws.status = SaveStatus::Failed(msg);
        }
        ws
    }

    /// Most recently modified first.
    pub(crate) fn sort(&mut self) {
        self.reapply_pending_tree_moves();
        self.entries.sort_by(|a, b| b.meta.modified.cmp(&a.meta.modified).then_with(|| a.id.cmp(&b.id)));
    }

    pub fn query(&self, cx: &App) -> String {
        self.search.read(cx).text.trim().to_lowercase()
    }

    pub fn open(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.try_open(id, window, cx);
    }

    /// Show `id`; false when it could not be opened (or the open document
    /// could not be saved first, see [`Self::save_before`]).
    pub fn try_open(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.open_target(id, false, window, cx)
    }

    pub(crate) fn open_target(&mut self, id: &str, new_tab: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        use crate::editor::core::Core;
        // A snapshot is on screen: back to the live document before anything
        // else touches the editor.
        if self.history.is_some() { self.close_history(cx); }
        if self.entries.iter().any(|e| e.id == id && e.meta.folder.is_some()) { return self.open_folder(id, window, cx); }
        let trash_meta = self.store.trash_meta(id);
        if trash_meta.as_ref().is_some_and(|m| m.folder.is_some()) { return self.open_trash_folder(Some(id), cx); }
        let trashed = trash_meta.is_some();
        if self.current.as_deref() == Some(id) && self.trash_document == trashed {
            self.view = View::Doc;
            if !self.search_open && let Some(ed) = &self.editor { ed.read(cx).focus(window); }
            cx.notify();
            return true;
        }
        let target = id.to_string();
        if !self.save_before(cx, move |w, window, cx| {
            w.open_target(&target, new_tab, window, cx);
        }) {
            return false;
        }
        let peer = if trashed { None } else { self.document_peer_core(id, window, cx) };
        let parked = self.tabs.get(id).filter(|_| !trashed).and_then(|t| t.parked.as_ref());
        // Keep block IDs stable when a sibling opens this note while all its
        // other views are parked. Verify the parked body before reusing it.
        let parked_peer = if !trashed && parked.is_none() && peer.is_none() { Self::parked_document_peer(id, window, cx) } else { None };
        let loaded = match (parked.or(parked_peer.as_ref()), &peer) {
            (Some(_), Some(_)) => Ok((None, None)),
            (Some(tab), None) => self.store.resume_for_edit(id, tab.baseline.as_ref(), &tab.core.doc.title.text).map(|(doc, title)| (doc, Some(title))),
            (None, _) => (if trashed { self.store.load_trash(id) } else { self.store.load_for_edit(id, peer.is_some()) }).map(|doc| (Some(doc), None)),
        };
        let (doc, title) = match loaded {
            Ok(loaded) => loaded,
            Err(e) => {
                self.show_toast_kind(ToastKind::Error, &crate::localized_format!("无法打开文档：{}", "Could not open document: {}", crate::settings::diagnostic(&e.to_string())), cx);
                cx.notify();
                return false;
            }
        };
        let meta = self.entries.iter().find(|e| e.id == id).map(|e| e.meta.clone()).or(trash_meta);
        let modified = meta.as_ref().map_or_else(crate::time::now, |m| m.modified);
        let width = meta.as_ref().map(|m| m.width).unwrap_or_default();
        let pinned = meta.as_ref().is_some_and(|m| !m.catalogue_closed);
        let parked = if trashed { None } else { self.tabs.get_mut(id).and_then(|tab| tab.parked.take()) };
        let (mut core, top) = if let Some(tab) = parked.or(parked_peer) {
            let mut core = doc.map_or(tab.core, Core::new);
            if let Some(title) = title && core.doc.title.text != title { core.receive_file_title(&title); }
            (core, tab.top)
        } else {
            (Core::new(doc.expect("new document was loaded")), self.store.scroll_top(id))
        };
        if let Some(peer) = peer && core.doc != peer.doc { Editor::receive_core(&mut core, &peer); }
        if !self.quick_note && !trashed { self.tabs.select_document(id, new_tab); }
        self.leave_document(window, cx);
        self.view = View::Doc;
        let editor = cx.new(|cx| {
            let mut e = Editor::new(core, modified, width, window, cx);
            e.restore_top = top;
            e.cat_pinned = pinned;
            e.markdown = meta.as_ref().is_some_and(|m| m.markdown);
            if let Some(m) = &meta {
                e.created = m.created;
                e.info = m.info;
                e.set_read_only(trashed || m.reading, cx);
            }
            e.trashed = trashed;
            e
        });
        self.editor_sub = Some(cx.subscribe_in(&editor, window, |this, _, ev: &EditorEvent, window, cx| match ev {
            EditorEvent::Changed => {
                this.file_history.prefer_files = false;
                // 阅读模式 put the document back: nothing to save.
                let rejected = this.editor.clone().is_some_and(|ed| ed.update(cx, |e, cx| e.reject_read_only_edit(cx)));
                if !rejected {
                    this.sync_document(window, cx);
                    this.schedule_save(window, cx);
                }
            }
            EditorEvent::Drawing => {
                if !this.drawing_open(cx) && let Some(next) = this.after_drawing.take() {
                    let workspace = cx.weak_entity();
                    window.on_next_frame(move |window, cx| {
                        let _ = workspace.update(cx, |w, cx| {
                            if w.flush(cx) { next(w, window, cx); }
                            else { w.after_save = Some(next); w.confirm = Some(Confirm::SaveFailed); cx.notify(); }
                        });
                    });
                }
                cx.notify();
            }
            EditorEvent::DrawingExitCanceled => this.after_drawing = None,
            EditorEvent::Mode(reading) => this.set_mode(*reading, window, cx),
            EditorEvent::Info(info) => this.set_info_row(*info),
            EditorEvent::Toast => {
                this.toast = None;
                cx.notify();
            }
            EditorEvent::Save => {
                this.flush(cx);
            }
            EditorEvent::Search(q) => {
                this.open_search(window, cx);
                this.search.update(cx, |s, cx| s.set_text(q, cx));
                this.search_sel = this.default_search_sel(cx);
            }
            EditorEvent::OpenDoc(id) => this.open(id, window, cx),
            EditorEvent::OpenDocAt(id, ordinal) => this.open_at(id, *ordinal, window, cx),
            EditorEvent::HideRefs => this.ask_refs(false, cx),
            EditorEvent::HistoryEscape => {
                // Arrives from inside the editor's own update: leave the
                // preview (then the drawer) once that finishes.
                let this = cx.entity();
                cx.defer(move |cx| {
                    this.update(cx, |w, cx| {
                        if w.preview.is_some() { w.exit_preview(cx) } else { w.close_history(cx) }
                    });
                });
            }
            EditorEvent::Duplicate => {
                if let Some(id) = this.current.clone() {
                    this.duplicate_doc(&id, window, cx);
                }
            }
            EditorEvent::Catalogue(pinned) => {
                if let Some(id) = this.current.clone() {
                    if let Some(e) = this.entries.iter_mut().find(|e| e.id == id) {
                        e.meta.catalogue_closed = !pinned;
                    }
                    let _ = this.store.set_catalogue(&id, *pinned);
                }
            }
        }));
        // Opening a document puts the caret in it, unless the user is typing a search.
        if !self.search_open {
            editor.read(cx).focus(window);
        }
        if let Some(m) = self.store.touch(id)
            && let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
                e.meta.opened = m.opened;
            }
        self.editor = Some(editor);
        self.current = Some(id.to_string());
        self.trash_document = trashed;
        if !trashed { crate::quick_note::used(id, cx); self.reveal_in_tree(id); }
        self.sync_sub_pages(cx);
        self.status = SaveStatus::Idle;
        self.row_menu = None;
        self.doc_menu = false;
        self.doc_sub = None;
        self.mode_menu = false;
        cx.notify();
        true
    }

    /// Drop the old view after saving, moving its Core into its tab if kept.
    pub(crate) fn leave_document(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.remember_scroll(cx);
        self.editor_sub = None;
        self.save_task = None;
        self.dirty_since = None;
        self.unsaved = false;
        self.status = SaveStatus::Idle;
        let trashed = std::mem::take(&mut self.trash_document);
        let (Some(id), Some(ed)) = (self.current.take(), self.editor.take()) else { return };
        if !trashed {
            if let Some(tab) = self.tabs.get_mut(&id) {
                let baseline = self.store.edit_baseline(&id);
                let (core, top) = ed.update(cx, |e, _| {
                    e.core.break_coalescing();
                    (std::mem::replace(&mut e.core, crate::editor::core::Core::new(Default::default())), e.restore_top.or_else(|| e.scroll_top()))
                });
                tab.parked = Some(crate::tabs::ParkedTab { core, top, baseline });
            }
            if !self.has_document_peer(&id, window, cx) { self.store.forget_loaded(&id); }
        }
    }

    /// ⌘N / the 文件 menu: create the 快捷新建 type (a fixed one, or the last
    /// one made from the 新建 menu) in the current place.
    pub fn new_doc(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parent = self.creation_parent();
        self.create_doc(crate::settings::quick_new_markdown(), parent, window, cx);
    }

    /// Ask before 删除: a confirmation, then the 回收站.
    pub fn ask_delete(&mut self, id: &str, cx: &mut Context<Self>) {
        self.row_menu = None;
        self.doc_menu = false;
        self.doc_sub = None;
        self.confirm = Some(Confirm::Delete { ids: vec![id.to_string()], batch: false });
        cx.notify();
    }

    /// Move documents to the trash; if the open one goes, open the next.
    pub fn delete_docs(&mut self, ids: &[String], window: &mut Window, cx: &mut Context<Self>) {
        if self.store.file_task_busy() || self.file_history.busy || self.tree_writes_pending() { return; }
        // A page goes with its child pages: each topmost page is
        // trashed as one item that carries its subtree.
        let tree = crate::tree::Tree::new(&self.entries);
        let mut all: Vec<(String, Option<String>)> = vec![];
        for id in ids {
            let covered = self.ancestors(id).iter().any(|a| ids.contains(&a.id));
            if covered {
                continue;
            }
            for d in tree.subtree(id) {
                if !all.iter().any(|(x, _)| *x == d) {
                    let root = (d != *id).then(|| id.clone());
                    all.push((d, root));
                }
            }
        }
        let groups = all;
        let all: Vec<String> = groups.iter().map(|(d, _)| d.clone()).collect();
        for handle in cx.windows().into_iter().filter(|w| w.window_id() != window.window_handle().window_id()) {
            if let Some(handle) = handle.downcast::<Workspace>()
                && handle.update(cx, |ws, _, _| ws.current.as_ref().is_some_and(|id| all.contains(id))).unwrap_or(false)
            {
                self.show_toast_kind(ToastKind::Error, crate::settings::text("请先关闭正在编辑这些文档的其他窗口，再删除"), cx);
                return;
            }
        }
        if self.current.as_ref().is_some_and(|c| all.contains(c)) {
            let retry = ids.to_vec();
            if !self.save_before(cx, move |w, window, cx| w.delete_docs(&retry, window, cx)) {
                return;
            }
        }
        let Some(task_guard) = self.store.begin_file_task() else { return };
        let mut writer = self.store.tree_writer();
        let task = cx.background_executor().spawn(async move {
            let journal = writer.track_file_changes();
            let mut deleted = vec![];
            let mut errors = vec![];
            let batch = writer.begin_batch().is_ok();
            for (id, root) in &groups {
                if root.as_ref().is_some_and(|r| !deleted.contains(r)) { continue; }
                match writer.delete(id, root.as_deref()) {
                    Ok(()) => deleted.push(id.clone()),
                    Err(e) => errors.push(crate::localized_format!("删除失败：{}", "Could not delete document: {}", crate::settings::diagnostic(&e.to_string()))),
                }
            }
            if batch && let Err(e) = writer.end_batch() {
                errors.push(crate::localized_format!("删除失败：{}", "Could not delete document: {}", crate::settings::diagnostic(&e.to_string())));
            }
            let warnings = writer.take_warnings();
            let listed = ListedDocs { entries: writer.list(), trash: writer.list_trash(), warnings };
            (DeletedDocs { deleted, errors, changes: journal.take() }, listed, writer)
        });
        self.batch = None;
        self.row_menu = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let (done, listed, writer) = task.await;
            let _ = this.update_in(cx, |w, window, cx| {
                w.store.receive_tree(writer);
                drop(task_guard);
                if w.current.as_ref().is_some_and(|c| done.deleted.contains(c)) {
                    w.close_history(cx);
                    if let Some(id) = &w.current { w.tabs.remove_document(id); }
                    w.leave_document(window, cx);
                }
                w.sync_peers(cx, &listed.entries, Some(&listed.trash));
                w.entries = listed.entries; w.trash = listed.trash;
                w.prune_tabs();
                if w.folder_current.as_ref().is_some_and(|id| done.deleted.contains(id)) {
                    w.folder_current = None;
                    if w.view == View::Folder { w.view = View::Home; }
                }
                w.row_menu = None;
                if w.current.is_none() && !matches!(w.view, View::Folder | View::NewTab) {
                    let view = w.view;
                    if w.tabs.selected_is_blank() {
                        if view == View::Doc { w.show_new_tab_page(window, cx); }
                    } else {
                        let next = w.tabs.selected_document().map(str::to_string)
                            .or_else(|| w.entries.iter().find(|e| e.meta.folder.is_none()).map(|e| e.id.clone()));
                        if next.is_some_and(|id| w.try_open(&id, window, cx)) {
                            w.view = view;
                        } else if view == View::Doc {
                            w.view = View::Home;
                            window.focus(&w.focus);
                        }
                    }
                }
                w.remember_file_changes(done.changes, crate::settings::tr("删除", "Delete"));
                w.sync_sub_pages(cx);
                let mut errors = done.errors;
                errors.extend(listed.warnings.iter().map(|s| crate::settings::diagnostic(s)));
                if errors.is_empty() { w.show_toast(crate::settings::text("删除成功"), cx); }
                else {
                    let msg = errors.join("；");
                    w.show_toast_kind(ToastKind::Error, &msg, cx);
                    w.status = SaveStatus::Failed(msg);
                }
                cx.notify();
            });
        }).detach();
    }

    pub fn toggle_pin(&mut self, id: &str, cx: &mut Context<Self>) {
        let mut on = false;
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.meta.pinned = !e.meta.pinned;
            on = e.meta.pinned;
            let _ = self.store.set_pinned(id, e.meta.pinned);
        }
        self.sort();
        self.row_menu = None;
        self.show_toast(if on { crate::settings::text("已添加到“置顶”") } else { crate::settings::text("已从“置顶”移除") }, cx);
        cx.notify();
    }

    /// 收藏 / 取消收藏 (listed under 主页 → 收藏).
    pub fn toggle_star(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(e) = self.entries.iter_mut().find(|e| e.id == id) else { return };
        let on = e.meta.starred == 0;
        if let Ok(t) = self.store.set_starred(id, on) {
            e.meta.starred = t;
        }
        self.row_menu = None;
        self.doc_menu = false;
        self.doc_sub = None;
        self.show_toast(if on { crate::settings::text("已添加到“收藏”") } else { crate::settings::text("已从“收藏”中移除") }, cx);
        cx.notify();
    }

    /// Switch the main pane to 主页 or 回收站 (or back to the document).
    pub fn show_view(&mut self, view: View, cx: &mut Context<Self>) {
        if self.drawing_open(cx) { return; }
        self.cancel_shortcut_recording(cx);
        self.close_history(cx);
        self.dismiss_tab_overflow();
        // 阅读位置: leaving the document with ⌘H or 回收站 also counts.
        if view != View::Doc { self.remember_scroll(cx) }
        if view == View::Trash {
            self.trash_folder = None;
            self.trash = self.store.list_trash();
        }
        self.view = view;
        self.row_menu = None;
        self.trash_menu = None;
        self.theme_menu = None;
        self.lang_menu = None;
        self.pref_menu = None;
        self.home.sort_menu = false;
        cx.notify();
    }

    /// A success toast (删除成功, 移动成功 …).
    pub fn show_toast(&mut self, msg: &str, cx: &mut Context<Self>) {
        self.show_toast_kind(ToastKind::Success, msg, cx);
    }

    pub fn show_toast_kind(&mut self, kind: ToastKind, msg: &str, cx: &mut Context<Self>) {
        self.toast = Some((kind, SharedString::from(msg.to_string()), Instant::now()));
        if let Some(ed) = &self.editor {
            ed.update(cx, |e, cx| e.clear_toast(cx));
        }
        self.toast_task = Some(cx.spawn(async |this, cx| {
            cx.background_executor().timer(Duration::from_millis(2700)).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        }));
        cx.notify();
    }

    fn render_toast(&self, window: &mut Window) -> Option<AnyElement> {
        let (kind, msg, t0) = self.toast.as_ref()?;
        crate::ui::toast_el(*kind, msg, *t0, window)
    }

    /// 重命名: the row's title turns into an input; Enter or clicking away saves, Esc cancels.
    pub fn start_rename(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(title) = self.entries.iter().find(|e| e.id == id).map(|e| e.title.trim().to_string()) else { return };
        let field = cx.new(|cx| {
            let mut f = TextField::new(crate::model::UNTITLED, cx);
            f.set_text(&title, cx);
            f.select_all_text(cx);
            f
        });
        let focus = field.focus_handle(cx);
        window.focus(&focus);
        let subs = vec![
            cx.subscribe_in(&field, window, |w, _, ev: &TextFieldEvent, window, cx| match ev {
                TextFieldEvent::Submit => w.finish_rename(true, window, cx),
                TextFieldEvent::Cancel => w.finish_rename(false, window, cx),
                _ => {}
            }),
            cx.on_blur(&focus, window, |w, window, cx| w.finish_rename(true, window, cx)),
        ];
        self.rename = Some(Rename { id: id.to_string(), field, _subs: subs });
        // From 主页 the row may be hidden: show it in the sidebar.
        self.sidebar_open = true;
        self.reveal_in_tree(id);
        self.row_menu = None;
        cx.notify();
    }

    fn finish_rename(&mut self, commit: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(r) = self.rename.take() else { return };
        let refocus = r.field.read(cx).is_focused(window);
        let title = r.field.read(cx).text.trim().to_string();
        let old = self.entries.iter().find(|e| e.id == r.id).map(|e| e.title.trim().to_string());
        if commit && old.is_some_and(|o| o != title) {
            if let Some(peer) = self.document_peers(&r.id, window, cx).first() {
                peer.update(cx, |e, cx| e.set_title(&title, cx));
            } else {
                if !self.flush(cx) { return; }
                let file_before = self.store.track_file_changes();
                match self.store.rename(&r.id, &title) {
                    Ok(meta) => {
                        if let Some(e) = self.entries.iter_mut().find(|e| e.id == r.id) {
                            e.title = title.clone();
                            e.meta = meta;
                        }
                        self.sort();
                        if self.current.as_deref() == Some(r.id.as_str()) && let Some(ed) = &self.editor {
                            ed.update(cx, |e, cx| e.receive_file_title(&title, cx));
                        }
                        self.remember_files(file_before, crate::settings::tr("重命名", "Rename"));
                    }
                    Err(e) => self.status = SaveStatus::Failed(crate::localized_format!("重命名失败：{}", "Could not rename document: {}", crate::settings::diagnostic(&e.to_string()))),
                }
            }
        }
        if refocus
            && let Some(ed) = &self.editor {
                ed.read(cx).focus(window);
            }
        self.sync_sub_pages(cx);
        cx.notify();
    }

    pub(crate) fn document_peers(&self, id: &str, window: &Window, cx: &mut Context<Self>) -> Vec<Entity<Editor>> {
        cx.windows().into_iter().filter(|w| w.window_id() != window.window_handle().window_id())
            .filter_map(|w| w.downcast::<Workspace>())
            .filter_map(|w| w.update(cx, |ws, _, _| {
                (ws.current.as_deref() == Some(id) && !ws.trash_document && ws.preview.is_none()).then(|| ws.editor.clone()).flatten()
            }).ok().flatten()).collect()
    }

    pub(crate) fn document_peer_core(&self, id: &str, window: &Window, cx: &mut Context<Self>) -> Option<crate::editor::core::Core> {
        cx.windows().into_iter().filter(|w| w.window_id() != window.window_handle().window_id())
            .filter_map(|w| w.downcast::<Workspace>())
            .find_map(|w| w.update(cx, |ws, _, cx| {
                if ws.current.as_deref() != Some(id) || ws.trash_document { return None }
                ws.preview.as_ref().map(|p| p.core.clone()).or_else(|| ws.editor.as_ref().map(|e| e.read(cx).core.clone()))
            }).ok().flatten())
    }

    fn has_document_peer(&self, id: &str, window: &Window, cx: &mut Context<Self>) -> bool {
        cx.windows().into_iter().filter(|w| w.window_id() != window.window_handle().window_id())
            .filter_map(|w| w.downcast::<Workspace>())
            .any(|w| w.update(cx, |ws, _, _| ws.current.as_deref() == Some(id) && !ws.trash_document).unwrap_or(false))
    }

    /// 阅读位置: hand the open document's scroll offset to the library, so
    /// coming back to it lands where it was left.
    fn remember_scroll(&mut self, cx: &mut Context<Self>) {
        let (Some(id), Some(editor)) = (self.current.clone(), self.editor.clone()) else { return };
        let editor = editor.read(cx);
        let top = editor.restore_top.or_else(|| editor.scroll_top());
        let _ = self.store.remember_scroll(&id, top);
    }

    pub(crate) fn sync_document(&mut self, window: &Window, cx: &mut Context<Self>) {
        // The previewed snapshot is not the live document: never hand it to a
        // sibling window.
        if self.preview.is_some() || self.trash_document { return }
        let (Some(id), Some(editor)) = (self.current.clone(), self.editor.clone()) else { return };
        let (title, icon) = { let e = editor.read(cx); (e.core.doc.title.text.clone(), e.core.doc.icon.clone()) };
        let mut core = None;
        for handle in cx.windows().into_iter().filter(|w| w.window_id() != window.window_handle().window_id()).filter_map(|w| w.downcast::<Workspace>()) {
            let _ = handle.update(cx, |ws, _, cx| {
                if let Some(entry) = ws.entries.iter_mut().find(|e| e.id == id) {
                    entry.title = title.clone();
                    entry.icon = icon.clone();
                }
                // Every remaining view can save the latest state if its sibling closes.
                if ws.current.as_deref() == Some(&id) && !ws.trash_document {
                    let core = core.get_or_insert_with(|| editor.read(cx).core.clone());
                    if let Some(preview) = ws.preview.as_mut() {
                        preview.receive_document(core);
                    } else if let Some(editor) = &ws.editor {
                        editor.update(cx, |e, cx| e.receive_document(core, cx));
                    }
                    ws.unsaved = true;
                }
                cx.notify();
            });
        }
    }

    fn schedule_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.preview.is_some() || self.trash_document { return }
        let Some(save_id) = self.current.clone() else { return };
        self.status = SaveStatus::Saving;
        if !self.unsaved || self.dirty_since.is_none() {
            self.dirty_since = Some(Instant::now());
        }
        self.unsaved = true;
        // Each edit replaces the timer (dropping a Task cancels it), so steady
        // typing would put the save off for as long as it lasts: the deadline
        // caps that at SAVE_MAX_DELAY after the oldest unsaved edit.
        let left = SAVE_MAX_DELAY.saturating_sub(self.dirty_since.map_or(Duration::ZERO, |t| t.elapsed()));
        let delay = SAVE_DELAY.min(left);
        // Keep the list title live while typing; the body/snippet updates on save.
        if let (Some(id), Some(ed)) = (&self.current, &self.editor) {
            let title = ed.read(cx).core.doc.title.text.clone();
            if let Some(e) = self.entries.iter_mut().find(|e| &e.id == id) {
                e.title = title;
            }
        }
        self.save_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(delay).await;
            while this.update(cx, |w, _| w.store.file_task_busy()).unwrap_or(false) {
                cx.background_executor().timer(Duration::from_millis(50)).await;
            }
            let _ = this.update(cx, |w, cx| {
                if w.current.as_ref() == Some(&save_id) { w.flush(cx); }
            });
        }));
        cx.notify();
    }

    fn check_reminders(&mut self, from: i64, to: i64, cx: &mut Context<Self>) {
        // The open document's reminders are read live (unsaved edits too).
        let open = self.editor.as_ref().map(|e| e.read(cx).core.doc.reminders());
        for e in &self.entries {
            let rs = match &open {
                Some(rs) if self.current.as_deref() == Some(e.id.as_str()) => rs,
                _ => &e.reminders,
            };
            for r in rs.iter().filter(|r| r.should_notify && from < r.notify && r.notify <= to) {
                notify_system(&display_title(&e.title), &crate::editor::reminder::notification(r));
            }
        }
    }

    /// Write the open document to disk if it has unsaved changes. False when
    /// that failed: the changes stay pending (a later save retries them) and
    /// the top bar shows the error.
    pub fn flush(&mut self, cx: &mut Context<Self>) -> bool {
        self.save_task = None;
        // 版本历史 preview: the editor holds an older snapshot, so writing it
        // would overwrite the live document with history.
        if self.preview.is_some() || self.trash_document { return true }
        if !self.unsaved {
            return true;
        }
        let (Some(id), Some(ed)) = (self.current.clone(), self.editor.clone()) else {
            self.unsaved = false;
            return true;
        };
        let doc = &ed.read(cx).core.doc;
        let (title, icon, body, reminders, links) = (doc.title.text.clone(), doc.icon.clone(), doc.plain_text(), doc.reminders(), doc.doc_links());
        match self.store.save(&id, doc) {
            Ok(meta) => {
                if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
                    e.title = title;
                    e.icon = icon;
                    e.body = body.into();
                    e.reminders = reminders;
                    e.links = links;
                    e.meta = meta.clone();
                }
                self.sync_links(cx);
                ed.update(cx, |e, cx| {
                    e.modified = meta.modified;
                    cx.notify();
                });
                self.status = SaveStatus::Saved;
                self.unsaved = false;
                self.sort();
                self.refresh_parked_tabs(&id, &ed, cx);
            }
            Err(e) => self.status = SaveStatus::Failed(crate::localized_format!("保存失败：{}", "Could not save: {}", crate::settings::diagnostic(&e.to_string()))),
        }
        // A failed save tries again SAVE_MAX_DELAY after this attempt, not on every keystroke.
        self.dirty_since = self.unsaved.then(Instant::now);
        cx.notify();
        !self.unsaved
    }

    pub(crate) fn drawing_open(&self, cx: &App) -> bool {
        self.editor.as_ref().is_some_and(|e| e.read(cx).drawing.is_some())
    }

    /// Resolve save failures before replacing the open document.
    pub fn save_before(&mut self, cx: &mut Context<Self>, then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static) -> bool {
        if self.store.file_task_busy() { return false; }
        if let Some(draft) = self.editor.as_ref().and_then(|e| e.read(cx).drawing.clone())
            && draft.update(cx, |d, cx| d.confirm_exit(cx)) {
            self.after_drawing = Some(Box::new(then));
            return false;
        }
        if self.flush(cx) {
            return true;
        }
        self.after_save = Some(Box::new(then));
        self.confirm = Some(Confirm::SaveFailed);
        cx.notify();
        false
    }

    /// The 保存失败 dialog's choices: `retry` saves again, otherwise the
    /// changes are given up. Either way the held-back action runs once the
    /// document no longer needs saving.
    pub(crate) fn resolve_save_failed(&mut self, retry: bool, window: &mut Window, cx: &mut Context<Self>) {
        if retry {
            if !self.flush(cx) {
                self.confirm = Some(Confirm::SaveFailed);
                return;
            }
        } else {
            let Some(id) = self.current.clone() else { return };
            match self.store.load_for_edit(&id, false) {
                Ok(doc) => {
                    if let Some(ed) = &self.editor {
                        ed.update(cx, |e, cx| e.receive_document(&crate::editor::core::Core::new(doc), cx));
                    }
                    self.sync_document(window, cx);
                    self.save_task = None;
                    self.unsaved = false;
                    self.status = SaveStatus::Idle;
                    self.dirty_since = None;
                    self.entries = self.store.list();
                    self.sort();
                }
                Err(e) => {
                    self.status = SaveStatus::Failed(e.to_string());
                    self.confirm = Some(Confirm::SaveFailed);
                    return;
                }
            }
        }
        if let Some(then) = self.after_save.take() {
            then(self, window, cx);
        }
        cx.notify();
    }

    pub(crate) fn save_failed_copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ed) = &self.editor else { return };
        let mut doc = ed.read(cx).core.doc.clone();
        doc.title = crate::model::RichText::plain(format!("{}{}", doc.title.text, crate::settings::tr("（恢复副本）", " (recovered copy)")));
        match self.store.create_from(&doc) {
            Ok(_) => {
                self.entries = self.store.list();
                self.sort();
                self.confirm = None;
                self.resolve_save_failed(false, window, cx);
            }
            Err(e) => {
                self.status = SaveStatus::Failed(e.to_string());
                self.confirm = Some(Confirm::SaveFailed);
                cx.notify();
            }
        }
    }

    /// The error of the last failed save, for the 保存失败 dialog.
    pub(crate) fn save_error(&self) -> String {
        match &self.status {
            SaveStatus::Failed(e) => e.clone(),
            _ => crate::settings::text("保存失败").to_string(),
        }
    }

    /// Hand the open document the library's titles and the documents
    /// linking to it (双向链接).
    pub(crate) fn sync_links(&mut self, cx: &mut Context<Self>) {
        let (Some(id), Some(ed)) = (self.current.clone(), self.editor.clone()) else { return };
        let docs = self
            .entries
            .iter()
            .map(|e| crate::editor::links::DocRef { id: e.id.clone(), title: e.title.clone(), icon: e.icon.clone(), folder: e.meta.folder.is_some(), modified: e.meta.modified, opened: e.meta.opened })
            .collect();
        let backlinks = self
            .entries
            .iter()
            .filter(|e| e.id != id)
            .filter_map(|e| {
                let lines: Vec<_> = e.links.iter().filter(|l| l.target == id).cloned().collect();
                (!lines.is_empty()).then(|| crate::editor::links::Backlink { id: e.id.clone(), lines })
            })
            .collect();
        let hidden = self.entries.iter().find(|e| e.id == id).is_some_and(|e| e.meta.refs_hidden);
        ed.update(cx, |e, cx| e.set_links(id, docs, backlinks, hidden, cx));
    }

    /// A 反向引用 line: open that document at its `ordinal`th block.
    fn open_at(&mut self, id: &str, ordinal: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.try_open(id, window, cx) {
            return;
        }
        let Some(ed) = self.editor.clone() else { return };
        ed.update(cx, |e, cx| {
            let mut n = 0;
            let mut target = None;
            e.core.doc.for_each_block(|b| {
                if n == ordinal {
                    target = Some(b.id);
                }
                n += 1;
            });
            if let Some(b) = target {
                e.scroll_to_block(b, cx);
            }
        });
    }

    /// 隐藏引用列表 / 显示引用列表: confirm first (hiding can skip it with 不再提示).
    pub(crate) fn ask_refs(&mut self, show: bool, cx: &mut Context<Self>) {
        if !show && self.store.settings().refs_hide_no_ask {
            return self.set_refs_hidden(true, cx);
        }
        self.confirm = Some(Confirm::Refs { show, no_ask: false });
        cx.notify();
    }

    pub(crate) fn set_refs_hidden(&mut self, hidden: bool, cx: &mut Context<Self>) {
        let Some(id) = self.current.clone() else { return };
        if self.store.update_meta(&id, |m| m.refs_hidden = hidden).is_ok()
            && let Some(e) = self.entries.iter_mut().find(|e| e.id == id)
        {
            e.meta.refs_hidden = hidden;
        }
        self.sync_links(cx);
        cx.notify();
    }

    /// 主页's search box, centred on the pane but never over what sits on
    /// either side of it: the title (and, with the sidebar hidden, the
    /// traffic lights and 展开侧栏) on the left, + on the right. Below a
    /// usable width it is None and the caller shows a search icon instead.
    pub(crate) fn header_search(&self, window: &Window, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let main = f32::from(window.viewport_size().width) - if self.sidebar_open { self.sidebar_w(window) } else { 0. };
        let title = crate::settings::label(crate::settings::text("主页"));
        let title_w = f32::from(window.text_system().shape_line(title.clone().into(), px(17.), &[window.text_style().highlight(HighlightStyle { font_weight: Some(FontWeight::SEMIBOLD), ..Default::default() }).to_run(title.len())], None).width);
        // page_header's left inset (26 with the sidebar, else the traffic
        // lights plus the 4 + 28 + 12 of 展开侧栏), then 16 clear of the title.
        let left = if self.sidebar_open { 26. } else { theme::TRAFFIC_LIGHTS_W + 44. + if cfg!(target_os = "windows") { 32. } else { 0. } } + title_w + 16.;
        // + is 32 wide with an 8pt inset; 16 clear of it.
        let right = HEADER_ACTIONS_H + crate::ui::header_right_inset() + 16.;
        let half = (main / 2. - left).min(main / 2. - right);
        let width = (main - 320.).min(2. * half).min(360.);
        if width < 120. { return None }
        Some(div().id("header-search").absolute().left(px((main - width) / 2.)).top(px(8.))
            .w(px(width)).h(px(32.)).no_drag().flex().items_center().gap(px(8.)).px(px(10.))
            .rounded(px(9.)).bg(Theme::fill_hover())
            .cursor_pointer().hover(|s| s.bg(Theme::fill_hover_strong()))
            .child(icon("search", 15., Theme::icon()))
            .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).text_color(Theme::text_caption()).child(crate::settings::tr("搜索文档…", "Search documents…")))
            .when(width >= 200., |d| d.child(div().flex_none().text_size(px(11.)).text_color(Theme::text_placeholder()).child(crate::shortcuts::hint("workspace::FocusSearch", "⌘ + J").replace(" + ", " "))))
            .tip(crate::settings::tr("搜索文档", "Search documents"), Some(crate::shortcuts::hint("workspace::FocusSearch", "⌘ + J")))
            .on_click(cx.listener(|w, _, window, cx| w.open_search(window, cx))))
    }

    /// `pane_w`: the width the header spans, to fold the breadcrumbs when narrow.
    fn top_bar(&self, pane_w: f32, cx: &mut Context<Self>) -> impl IntoElement {
        if self.trash_document { return self.trash_document_header(cx) }
        let title = self
            .editor
            .as_ref()
            .map(|e| display_title(&e.read(cx).core.doc.title.text))
            .unwrap_or_default();
        let save_error = match &self.status {
            SaveStatus::Failed(error) => Some(error.clone()),
            _ => None,
        };
        // Independent icon controls, with equal top and right insets.
        let seg = crate::ui::header_icon;
        let divider = || div().flex_none().w(px(1.)).h(px(18.)).bg(Theme::line_divider());
        // Breadcrumbs: the parent pages, each linking to itself, with the
        // middle ones folded into "…" when the chain is long.
        let chain: Vec<(String, String, Option<String>)> = self
            .current
            .as_ref()
            .map(|id| self.ancestors(id).iter().map(|e| (e.id.clone(), display_title(&e.title), e.icon.clone())).collect())
            .unwrap_or_default();
        let mut crumbs: Vec<AnyElement> = vec![];
        let sep = || div().mx(px(3.)).flex_none().child(icon("chevron-right", 12., Theme::icon())).into_any_element();
        // Every path starts at the library, as a folder page's does.
        if self.current.is_some() {
            crumbs.push(div().id("crumb-library").no_drag().flex_none().px(px(4.)).rounded(px(4.)).cursor_pointer()
                .whitespace_nowrap().text_color(Theme::text_caption()).hover(|s| s.text_color(Theme::primary_500()))
                .child(ui_text(crate::settings::tr("我的文档", "My documents").to_string()))
                .on_click(cx.listener(|w, _, _, cx| w.show_view(View::Home, cx)))
                .into_any_element());
            crumbs.push(sep());
        }
        for (i, (id, name, em)) in chain.iter().enumerate() {
            if chain.len() > 2 && i > 0 && i < chain.len() - 1 {
                if i == 1 {
                    let hidden: Vec<&str> = chain[1..chain.len() - 1].iter().map(|c| c.1.as_str()).collect();
                    crumbs.push(
                        div()
                            .id("crumb-more")
                            .no_drag()
                            .flex_none()
                            .size(px(20.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .text_color(Theme::text_caption())
                            .tip(&hidden.join(" / "), None)
                            .child("…")
                            .into_any_element(),
                    );
                    crumbs.push(sep());
                }
                continue;
            }
            let id = id.clone();
            crumbs.push(
                div()
                    .id(SharedString::from(format!("crumb-{id}")))
                    .no_drag()
                    .min_w(px(28.))
                    .max_w(px(200.))
                    .px(px(4.))
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .text_color(Theme::text_caption())
                    .hover(|s| s.text_color(Theme::primary_500()))
                    .when_some(em.clone(), |d, em| {
                        d.child(div().size(px(20.)).mr(px(4.)).flex_none().flex().items_center().justify_center().text_size(px(16.)).child(em))
                    })
                    .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(ui_text(name.clone())))
                    .on_click(cx.listener(move |w, _, window, cx| w.open(&id, window, cx)))
                    .into_any_element(),
            );
            crumbs.push(sep());
        }
        // Too narrow for the path: it folds into one "…" (the path on hover),
        // and the title keeps the room. Left: the sidebar's buttons when it is
        // closed; right: the three actions.
        let left = if self.sidebar_open { 18. } else { theme::TRAFFIC_LIGHTS_W + 4. + 28. + 4. + 28. + 25. };
        let room = pane_w - left - crate::ui::header_right_inset() - 3. * HEADER_ACTIONS_H - 8. - 16.;
        if room < 360. && !crumbs.is_empty() {
            let path: Vec<&str> = std::iter::once(crate::settings::tr("我的文档", "My documents")).chain(chain.iter().map(|c| c.1.as_str())).collect();
            crumbs = vec![
                div().id("crumb-more").no_drag().flex_none().size(px(20.)).flex().items_center().justify_center().rounded(px(4.))
                    .text_color(Theme::text_caption()).tip(&path.join(" / "), None).child("…").into_any_element(),
                sep(),
            ];
        }
        let record_more = self.more_btn_bounds.clone();
        div()
            .id("top-bar").relative()
            .h(px(HEADER_H))
            .flex_none()
            .flex()
            .items_center()
            .pr(px(crate::ui::header_right_inset()))
            // Clear of the traffic lights, which sit over the note when the sidebar is closed.
            .pl(if self.sidebar_open { px(18.) } else { px(theme::TRAFFIC_LIGHTS_W) })
            .title_bar()
            .when(!self.sidebar_open, |d| {
                d.child(
                    div()
                        .id("open-sidebar")
                        .no_drag()
                        .flex_none()
                        .ml(px(4.))
                        .size(px(28.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(8.))
                        .cursor_pointer()
                        .hover(|s| s.bg(Theme::fill_hover_strong()))
                        .tip(crate::settings::tr("展开侧栏", "Show sidebar"), Some(crate::shortcuts::hint("workspace::ToggleSidebar", "⌘ + ]")))
                        .child(icon("panel-left-open", 17., Theme::icon()))
                        .on_click(cx.listener(|w, _, _, cx| {
                            w.sidebar_open = true;
                            cx.notify();
                        })),
                )
                .child(
                    div().id("header-home").no_drag().flex_none().ml(px(4.)).size(px(28.))
                        .flex().items_center().justify_center().rounded(px(8.)).cursor_pointer()
                        .hover(|s| s.bg(Theme::fill_hover_strong()))
                        .active(|s| s.bg(Theme::fill_pressed()))
                        .tip(crate::settings::text("返回主页"), None)
                        .child(icon("home", 18., Theme::icon()))
                        .on_click(cx.listener(|w, _, _, cx| w.show_view(View::Home, cx))),
                )
                .child(div().mx(px(12.)).child(divider()))
            })
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .h(px(20.))
                            .min_w_0()
                            .overflow_hidden()
                            .flex()
                            .items_center()
                            .text_size(px(13.))
                            .line_height(px(20.))
                            .children(crumbs)
                            .child(
                                div()
                                    .min_w_0()
                                    .text_size(px(14.))
                                    .line_height(px(20.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(Theme::text())
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(ui_text(title)),
                            ),

                    )
                    .when_some(save_error, |d, error| d.child(
                        div().min_w_0().max_w(px(220.)).text_size(px(12.)).line_height(px(18.))
                            .text_color(theme::c(craft::R400)).overflow_hidden().whitespace_nowrap().text_ellipsis().child(error),
                    )),
            )
            .child(div().flex_1().min_w(px(16.)))
            .child(
                div().gap(px(4.))
                    .h(px(HEADER_ACTIONS_H)).flex_none()
                    .flex()
                    .items_center()
                    .when(self.current.is_some(), |d| {
                        d.child(
                            // No tooltip while its dropdown is open; the menu hangs
                            // from this button's right edge.
                            seg("doc-more", "ellipsis")
                                .relative()
                                .when(self.doc_menu, |d| d.bg(Theme::fill_hover()))
                                .when(!self.doc_menu, |d| d.tip(crate::settings::tr("更多", "More"), None))
                                .child(canvas(move |b, _, _| record_more.set(Some(b)), |_, _, _, _| {}).absolute().inset_0())
                                .on_click(cx.listener(|w, _, _, cx| {
                                    w.doc_menu = !w.doc_menu;
                                    w.doc_sub = None;
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        seg("doc-search", "search")
                            .tip_label(crate::shortcuts::keys(crate::settings::text("搜索(⌘ + J)")).into(), TipSide::Below)
                            .on_click(cx.listener(|_, _, window, cx| window.dispatch_action(Box::new(FocusSearch), cx))),
                    )
                    .child(
                        // Straight to a new document: no 新建 menu, as 导入 is rare.
                        seg("doc-new", "plus").tip(crate::settings::tr("新建文档", "New document"), None).on_click(cx.listener(|w, _, window, cx| { if !w.drawing_open(cx) { w.new_doc(window, cx); } })),
                    ).into_any_element()
            )
    }
}

/// Header and action group share an 8px top/right inset.
pub const HEADER_H: f32 = 48.;
/// The narrowest the main pane (document, Home, settings) gets: the window's
/// minimum width is this plus the open sidebar.
pub const DOC_MIN_W: f32 = 480.;
pub(crate) const HEADER_ACTIONS_H: f32 = 32.;

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Clicks no longer take focus from the editor (see the root's mouse
        // down), so leaving the document hands it back to the window's keys.
        if self.view != View::Doc && self.editor.as_ref().is_some_and(|e| e.read(cx).focus.is_focused(window)) { window.focus(&self.focus); }
        crate::editor::image::frame_drawn(window, cx);
        // Full screen: the sidebar is solid (`Theme::window`) and the native glass stays hidden.
        crate::theme::set_fullscreen(window.is_fullscreen());
        self.native_sidebar.sync(if self.sidebar_open && !crate::theme::fullscreen() { self.sidebar_w(window) } else { 0. }, window);
        // The pane keeps its minimum beside the sidebar: opening the sidebar in
        // a narrow window widens the window rather than squeezing the note.
        let min_w = DOC_MIN_W + if self.sidebar_open { self.sidebar_w(window) } else { 0. };
        if min_w != self.min_w {
            self.min_w = min_w;
            window.set_window_min_size(size(px(min_w), px(480.)));
            let vs = window.viewport_size();
            if f32::from(vs.width) < min_w - 0.5 && !window.is_fullscreen() { window.resize(size(px(min_w), vs.height)); }
        }
        let sidebar = if self.sidebar_open { Some(if self.view == View::Settings { self.render_settings_sidebar(window, cx) } else { self.render_sidebar(window, cx) }) } else { None };
        let row_menu = self.render_row_menu(window, cx);
        let resize = self.sidebar_open.then(|| self.render_resize_handle(window, cx));
        let resize_overlay = self.render_resize_overlay(cx);
        let folder_drag_overlay = self.render_folder_drag_overlay(cx);
        let file_drop = self.render_file_drop(window, cx);
        let create_menu = self.render_create_menu(cx);
        let import_menu = self.render_import_menu(cx);
        let place_menu = self.render_place_menu(cx);
        let search = self.render_search(window, cx);
        let doc_menu = self.render_doc_menu(window, cx);
        let doc_info = self.render_doc_info(cx);
        let history = self.render_history(cx);
        let confirm = self.render_confirm(cx);
        let trash_menu = self.render_trash_menu(cx);
        let home_menus = self.render_home_menus(cx);
        let theme_menu = self.render_theme_menu(cx);
        let lang_menu = self.render_lang_menu(cx);
        let pref_menu = self.render_pref_menu(cx);
        let toast = self.render_toast(window);
        let mode_menu = self.render_mode_menu(cx);
        self.home.folder_bounds.borrow_mut().clear();
        let main = match self.view {
            View::Doc => div()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .child(self.top_bar(f32::from(window.viewport_size().width) - if self.sidebar_open { self.sidebar_w(window) } else { 0. }, cx))
                .children(if self.trash_document { None } else { self.render_document_tabs(window, cx) })
                .children(self.render_history_banner(cx))
                .children(self.render_trash_banner(cx))
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .child(
                            div()
                                .relative()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .children(self.editor.clone()),
                        )
                        .children(self.render_shortcuts(cx)),
                )
                .into_any_element(),
            View::NewTab => self.render_new_tab(window, cx),
            View::Home => self.render_home(window, cx),
            View::Folder => self.render_folder(window, cx),
            View::Trash => self.render_trash(cx),
            View::Settings => self.render_settings(cx),
            View::Git => self.render_git(cx),
        };
        let move_dlg = self.render_move_dialog(cx);
        let export_md = self.render_export_md(cx);
        // Only the note stands out: a white plane flush with the window's top,
        // right and bottom edges. The sidebar sits sunken on the window
        // surface beside it; the change of value is the only line between
        // them (no gutter, radius, border or shadow).
        // A document sits on its theme's page colour.
        let page = if matches!(self.view, View::Doc | View::Folder) { crate::editor_theme::Theme::bg_body() } else { Theme::bg_body() };
        let note = div().id("note").relative().flex_1().min_w_0().h_full().flex().bg(page).child(main)
            // Files from the Finder dropped on 主页 or a folder's page are imported there.
            .when(matches!(self.view, View::Home | View::Folder), |d| d
                .on_drag_move(cx.listener(|w, ev: &DragMoveEvent<ExternalPaths>, _, cx| w.file_drag_move(ev, cx)))
                .on_drop(cx.listener(|w, paths: &ExternalPaths, window, cx| w.file_dropped(paths, window, cx))))
            .children(file_drop)
            .children(history);
        div()
            .id("workspace")
            .key_context("Workspace")
            .capture_action(cx.listener(|w, _: &crate::editor::keymap::Undo, window, cx| w.capture_file_history(false, window, cx)))
            .capture_action(cx.listener(|w, _: &crate::editor::keymap::Redo, window, cx| w.capture_file_history(true, window, cx)))
            .track_focus(&self.focus)
            // A click on what takes no input (title bar, page margins, the
            // sidebar's blank space) leaves the caret in the document, also
            // the click that brings the window back (#59). Runs before the
            // root takes focus; anything inside that takes focus did so first.
            .on_mouse_down(MouseButton::Left, cx.listener(|w, _, window, cx| {
                if w.view == View::Doc && w.editor.as_ref().is_some_and(|e| e.read(cx).focus.is_focused(window)) { window.prevent_default(); }
            }))
            .relative()
            .size_full()
            .flex()
            .bg(Theme::window())
            .font_family(theme::UI_FONT)
            .text_color(Theme::text())
            // Window keys act on the window they were pressed in, not on
            // whichever window macOS considers active.
            .on_mouse_move(cx.listener(|w, ev: &MouseMoveEvent, window, cx| w.track_sidebar_press(ev, window, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|w, ev: &MouseUpEvent, window, cx| w.finish_sidebar_click(ev.modifiers, window, cx)))
            .on_action(cx.listener(|w, _: &CloseWindow, window, cx| {
                crate::quick_note::persist(cx);
                if w.save_before(cx, |_, window, _| window.remove_window()) { window.remove_window(); }
            }))
            .on_action(cx.listener(|w, _: &NewTab, window, cx| w.new_tab(window, cx)))
            .on_action(cx.listener(|w, _: &CloseTab, window, cx| w.close_selected_tab(window, cx)))
            .on_action(cx.listener(|w, _: &NextTab, window, cx| w.step_tab(1, window, cx)))
            .on_action(cx.listener(|w, _: &PreviousTab, window, cx| w.step_tab(-1, window, cx)))
            .on_action(|_: &Minimize, window, _| window.minimize_window())
            .on_action(|_: &Zoom, window, _| window.zoom_window())
            .on_action(|_: &ToggleFullScreen, window, _| window.toggle_fullscreen())
            .on_action(cx.listener(|w, _: &OpenSettings, _, cx| w.show_view(View::Settings, cx)))
            .on_action(cx.listener(|w, _: &ReplayOnboarding, window, cx| { if !w.drawing_open(cx) { w.replay_onboarding(window, cx); } }))
            .on_action(cx.listener(|w, _: &OpenAbout, _, cx| { if !w.drawing_open(cx) { w.open_settings(crate::settings::Page::About, cx); } }))
            .on_action(cx.listener(|w, _: &GoHome, _, cx| w.show_view(View::Home, cx)))
            .on_action(cx.listener(|w, _: &NewDoc, window, cx| { if !w.drawing_open(cx) { w.new_doc(window, cx); } }))
            .on_action(cx.listener(|w, _: &ImportDoc, window, cx| { if !w.drawing_open(cx) { w.open_import(window, cx); } }))
            .on_action(cx.listener(|w, _: &OpenFile, window, cx| { if w.drawing_open(cx) { window.dispatch_action(Box::new(crate::editor::drawing::OpenDrawing), cx); } else { w.open_file_dialog(window, cx); } }))
            .on_action(cx.listener(|w, _: &ImportFolder, window, cx| { if !w.drawing_open(cx) { w.open_import_folder(window, cx); } }))
            .on_action(cx.listener(|w, _: &ToggleSidebar, _, cx| {
                w.sidebar_open = !w.sidebar_open;
                cx.notify();
            }))
            .on_action(cx.listener(|w, _: &FindInDoc, window, cx| {
                if w.drawing_open(cx) { cx.propagate(); } else { w.find_in_doc(false, window, cx); }
            }))
            .on_action(cx.listener(|w, _: &ReplaceInDoc, window, cx| {
                if !w.drawing_open(cx) { w.find_in_doc(!w.reading(), window, cx); }
            }))
            .on_action(cx.listener(|w, _: &crate::modes::EditMode, window, cx| { if !w.drawing_open(cx) { w.set_mode(false, window, cx); } }))
            .on_action(cx.listener(|w, _: &crate::modes::ReadMode, window, cx| { if !w.drawing_open(cx) { w.set_mode(true, window, cx); } }))
            .on_action(cx.listener(|w, _: &crate::modes::ToggleShortcuts, _, cx| {
                w.shortcuts = !w.shortcuts;
                cx.notify();
            }))
            .on_key_down(cx.listener(|w, ev: &KeyDownEvent, window, cx| {
                // The theme dropdown: ↑ ↓ preview, ↩ applies, Esc closes it (not 设置).
                if (w.theme_menu.is_some() && w.theme_menu_key(&ev.keystroke.key, cx)) || (w.lang_menu.is_some() && w.lang_menu_key(&ev.keystroke.key, cx))
                    || (w.pref_menu.is_some() && w.pref_menu_key(&ev.keystroke.key, cx)) {
                    cx.stop_propagation();
                    return;
                }
                // Esc leaves 设置 (unless its filter field has the caret).
                if ev.keystroke.key == "escape" && w.view == View::Settings && !w.settings_filter.read(cx).is_focused(window) {
                    w.close_settings(cx);
                    cx.stop_propagation();
                    return;
                }
                if ev.keystroke.key == "escape" && (w.doc_info.is_some() || w.history.is_some() || w.doc_menu || w.mode_menu) {
                    w.doc_info = None;
                    if w.history.is_some() { w.close_history(cx); }
                    w.mode_menu = false;
                    w.doc_menu = false;
                    w.doc_sub = None;
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            // Esc leaves 批量操作 (closing its menu or confirmation first), even from the editor.
            .capture_key_down(cx.listener(|w, ev: &KeyDownEvent, window, cx| {
                let undo = ev.keystroke.modifiers.secondary() && matches!(ev.keystroke.key.as_str(), "z" | "y");
                if w.store.file_task_busy() || (w.file_history.busy && !undo) { cx.stop_propagation(); return; }
                if ev.keystroke.key == "escape" && w.dismiss_tab_overflow() { cx.notify(); cx.stop_propagation(); return; }
                // 版本历史: the editor keeps the caret, so Esc has to be taken
                // before it reaches the read-only key handler.
                if ev.keystroke.key == "escape" && w.preview.is_some() {
                    w.exit_preview(cx); cx.stop_propagation(); return;
                }
                if ev.keystroke.key == "escape" && w.history.is_some() {
                    w.close_history(cx); cx.stop_propagation(); return;
                }
                if ev.keystroke.key == "escape" && matches!(w.view, View::Home | View::Folder) && cx.stop_active_drag(window) {
                    cx.stop_propagation(); return;
                }
                if ev.keystroke.key == "escape" && w.folder_drag.active.is_some() {
                    w.end_folder_drag(true, cx); cx.stop_propagation(); return;
                }
                if w.record_shortcut(ev, cx) { return; }
                // Esc cancels a confirmation dialog.
                if ev.keystroke.key == "escape" && (w.confirm.is_some() || w.export_md.is_some()) {
                    w.confirm = None;
                    w.after_save = None;
                    w.export_md = None;
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if ev.keystroke.key != "escape" || w.batch.is_none() || w.search_open || w.rename.is_some() {
                    return;
                }
                if w.row_menu.is_some() {
                    w.row_menu = None;
                } else {
                    w.batch = None;
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .on_action(cx.listener(|w, _: &FocusSearch, window, cx| {
                if !w.drawing_open(cx) {
                    if w.search_open { w.close_search(window, cx); } else { w.open_search(window, cx); }
                }
                cx.stop_propagation();
            }))
            .children(sidebar)
            .child(note)
            .children(resize)
            .children(crate::ui::window_controls(window))
            .children(row_menu)
            .children(self.render_tab_overflow(window, cx))
            .children(doc_menu)
            .children(mode_menu)
            .children(doc_info)
            .children(trash_menu)
            .children(home_menus)
            .children(theme_menu)
            .children(lang_menu)
            .children(pref_menu)
            .children(confirm)
            .children(move_dlg)
            .children(export_md)
            .children(search)
            .children(resize_overlay)
            .children(folder_drag_overlay)
            .when(self.file_history.busy || self.store.file_task_busy(), |d| d.child(deferred(div().id("file-history-pending").absolute().inset_0().occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())).with_priority(20)))
            .children(create_menu)
            .children(import_menu)
            .children(place_menu)
            .children(toast)
            .when_some(self.editor.as_ref().and_then(|ed| ed.read(cx).drawing.clone()), |d, drawing| {
                d.child(deferred(div().absolute().inset_0().child(drawing)).with_priority(26))
            })
            // Over everything, toasts included, until its last page is done.
            .when_some(self.onboarding.clone(), |d, o| d.child(deferred(div().id("onboarding-cover").absolute().inset_0().occlude().child(o)).with_priority(30)))
    }
}

impl Workspace {
    /// Lay the first-launch pages over this window; finishing them uncovers it.
    pub(crate) fn replay_onboarding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.onboarding.is_some() { return }
        let o = cx.new(|cx| crate::onboarding::Onboarding::replay(window, cx));
        cx.subscribe_in(&o, window, |w, _, _: &crate::onboarding::Done, window, cx| {
            w.onboarding = None;
            // The language may have changed on the first page.
            crate::settings::refresh_all(w, cx);
            window.focus(&w.focus);
            cx.notify();
        }).detach();
        self.onboarding = Some(o);
        cx.notify();
    }
}

/// Fault injection against real window/save code, available only in the harness build.
#[cfg(feature = "devtools")]
pub(crate) fn check_document_safety(phase: &str, cx: &mut App) {
    use crate::editor::core::{EditKind, Pos, Selection};
    use crate::model::{Block, Document, RichText};
    if phase == "preview" {
        let mut store = cx.global::<SharedStore>().clone();
        let doc = Document { title: RichText::plain("Preview safety"), blocks: vec![Block::text("Original")], ..Default::default() };
        let id = store.create_from(&doc).unwrap();
        assert!(store.save_version_now(&id).unwrap());
        fn make_window(cx: &mut App, id: String) -> WindowHandle<Workspace> { cx.open_window(WindowOptions {
            focus: false,
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(-20000.), px(-20000.)), size(px(1200.), px(900.))))),
            ..Default::default()
        }, |window, cx| cx.new(|cx| Workspace::new_document_window(Some(id), window, cx))).unwrap() }
        let a = make_window(cx, id.clone());
        let b = make_window(cx, id.clone());
        a.update(cx, |w, window, cx| w.open_history(&id, window, cx)).unwrap();
        b.update(cx, |w, window, cx| {
            w.editor.as_ref().unwrap().update(cx, |e, cx| {
                let block = &e.core.doc.blocks[0];
                e.core.sel = Selection::caret(Pos::new(block.id, block.text.len()));
                e.core.checkpoint(EditKind::Typing);
                e.core.insert_text(" from B");
                e.changed(window, cx);
            });
            // Deliver this edit before the next assertion in this App update;
            // queued EditorEvent callbacks run after the update returns.
            w.sync_document(window, cx);
            w.schedule_save(window, cx);
        }).unwrap();
        a.update(cx, |w, _, cx| {
            assert_eq!(w.editor.as_ref().unwrap().read(cx).core.doc.blocks[0].text.text, "Original");
            assert_eq!(w.preview.as_ref().unwrap().core.doc.blocks[0].text.text, "Original from B");
            assert!(w.unsaved);
        }).unwrap();
        // A third view must open the live Core hidden behind A's preview.
        b.update(cx, |w, window, cx| { assert!(w.flush(cx)); window.remove_window(); }).unwrap();
        let c = make_window(cx, id.clone());
        c.update(cx, |w, window, cx| {
            assert_eq!(w.editor.as_ref().unwrap().read(cx).core.doc.blocks[0].text.text, "Original from B");
            assert!(w.flush(cx));
            window.remove_window();
        }).unwrap();
        a.update(cx, |w, window, cx| {
            w.exit_preview(cx);
            w.editor.as_ref().unwrap().update(cx, |e, cx| {
                let block = &e.core.doc.blocks[0];
                e.core.sel = Selection::caret(Pos::new(block.id, block.text.len()));
                e.core.checkpoint(EditKind::Typing);
                e.core.insert_text(" from A");
                e.changed(window, cx);
            });
            assert!(w.flush(cx));
            assert_eq!(store.load(&id).unwrap().blocks[0].text.text, "Original from B from A");
            window.remove_window();
        }).unwrap();
    } else {
        let handle = cx.windows().into_iter().find_map(|w| w.downcast::<Workspace>()).unwrap();
        handle.update(cx, |w, window, cx| {
            assert!(!w.store.file_task_busy());
            if phase == "delete" {
                let id = w.current.clone().unwrap();
                w.delete_docs(&[id], window, cx);
            } else {
                assert!(matches!(w.view, View::Doc));
                assert!(w.current.is_some() && w.editor.is_some());
            }
        }).unwrap();
    }
    eprintln!("document-safety: {phase} checked");
}

#[cfg(feature = "devtools")]
#[derive(Clone)]
struct SaveSafetyQa(Vec<(WindowHandle<Workspace>, String)>);
#[cfg(feature = "devtools")]
impl Global for SaveSafetyQa {}

#[cfg(feature = "devtools")]
pub(crate) fn check_save_safety(phase: &str, cx: &mut App) {
    use crate::model::{Block, Document, RichText};
    // Shared titles come from metadata; inject conflicts into the body as well.
    let doc = |title: &str| Document { title: RichText::plain(title), blocks: vec![Block::text(title)], ..Default::default() };
    if phase == "setup" {
        let mut store = cx.global::<SharedStore>().clone();
        let mut windows = vec![];
        for name in ["A", "B", "C"] {
            let id = store.create_from(&doc(name)).unwrap();
            let open_id = id.clone();
            let handle = cx.open_window(WindowOptions { focus: false, ..Default::default() }, |window, cx| {
                cx.new(|cx| Workspace::new_document_window(Some(open_id), window, cx))
            }).unwrap();
            handle.update(cx, |ws, _, cx| {
                ws.editor.as_ref().unwrap().update(cx, |e, cx| e.receive_document(&crate::editor::core::Core::new(doc(&format!("local-{name}"))), cx));
                ws.unsaved = true;
            }).unwrap();
            if name != "C" {
                std::fs::write(store.file_path(&id).unwrap(), crate::docxml::serialize(&doc(&format!("disk-{name}")))).unwrap();
            }
            windows.push((handle, id));
        }
        quit(cx);
        for (handle, _) in &windows[..2] {
            handle.update(cx, |ws, _, _| assert!(ws.unsaved && matches!(ws.confirm, Some(Confirm::SaveFailed)))).unwrap();
        }
        assert_eq!(store.load(&windows[2].1).unwrap().blocks[0].text.text, "local-C");
        windows[0].0.update(cx, |ws, window, cx| {
            ws.resolve_save_failed(true, window, cx);
            assert!(ws.unsaved);
            ws.confirm = None;
            ws.resolve_save_failed(false, window, cx);
            assert!(!ws.unsaved);
            assert_eq!(ws.editor.as_ref().unwrap().read(cx).core.doc.blocks[0].text.text, "disk-A");
        }).unwrap();
        cx.set_global(SaveSafetyQa(windows));
        eprintln!("save-safety: all windows checked; successful sibling saved; retry and reload checked");
    } else {
        let windows = cx.global::<SaveSafetyQa>().0.clone();
        windows[1].0.update(cx, |ws, window, cx| {
            assert!(ws.unsaved && matches!(ws.confirm, Some(Confirm::SaveFailed)));
            ws.after_save = None;
            ws.save_failed_copy(window, cx);
            assert!(!ws.unsaved);
            assert_eq!(ws.editor.as_ref().unwrap().read(cx).core.doc.blocks[0].text.text, "disk-B");
            assert!(ws.store.list().iter().any(|e| Some(&e.id) != ws.current.as_ref() && ws.store.load(&e.id).is_ok_and(|d| d.blocks[0].text.text == "local-B")));
        }).unwrap();
        windows[0].0.update(cx, |ws, _, cx| {
            ws.editor.as_ref().unwrap().update(cx, |e, cx| e.receive_document(&crate::editor::core::Core::new(doc("after-reload")), cx));
            ws.unsaved = true;
            assert!(ws.flush(cx));
            assert_eq!(ws.store.load(ws.current.as_ref().unwrap()).unwrap().blocks[0].text.text, "after-reload");
        }).unwrap();
        eprintln!("save-safety: PASS; remaining conflict blocked quit; copy preserved; edit after reload saved cleanly");
        for (handle, _) in windows { handle.update(cx, |_, window, _| window.remove_window()).unwrap(); }
    }
}

//! GitHub Releases updates: check metadata quietly, download on request, verify
//! Minisign and the platform signature, then save every window before installing.
//! See docs/UPDATES.md. Explicit URL/key overrides are for isolated tests only.

use gpui::*;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(target_os = "macos")]
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};

mod http;
mod install;
pub use install::{register_instance, run_helper};
use std::time::Duration;

const BASE: &str = "https://github.com/iuxt/quiet-md/releases/latest/download";
const PACKAGES: &str = "https://github.com/iuxt/quiet-md/releases/download";
static DOWNLOADED: AtomicU64 = AtomicU64::new(0);
const KEY: &str = match option_env!("QUIET_MD_UPDATE_PUBKEY") { Some(k) => k, None => include_str!("../packaging/update-key.pub") };

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub url: String,
    pub size: u64,
    /// The `.minisig` file's text.
    pub signature: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Idle,
    /// This copy cannot update itself (a development build, an unsigned app, another platform).
    Unsupported,
    Checking,
    Current,
    Available(Manifest),
    Downloading(Manifest),
    /// Verified package: the new `.app`, Windows installer, or Linux AppImage.
    Ready(Manifest, PathBuf),
    /// All windows saved; the helper waits for this process to exit.
    Installing(Manifest),
    Failed(String),
}

/// `last`: when the latest automatic check started.
pub struct Updater {
    pub state: State, told: Option<String>, last: Option<std::time::Instant>,
    checking: bool, manual: bool, retry: Option<std::time::Instant>,
}
impl Global for Updater {}

#[derive(Debug, PartialEq)]
enum Check { Start, Joined, Busy, Wait }

impl Updater {
    fn begin(&mut self, manual: bool) -> Check {
        if matches!(self.state, State::Downloading(_) | State::Ready(..) | State::Installing(_)) { return Check::Busy }
        if self.checking { self.manual |= manual; return Check::Joined }
        if self.retry.is_some_and(|t| t > std::time::Instant::now()) { return Check::Wait }
        if matches!(self.state, State::Unsupported) && !manual { return Check::Busy }
        self.checking = true;
        self.manual = manual;
        self.last = Some(std::time::Instant::now());
        self.retry = None;
        Check::Start
    }

    fn complete(&mut self, result: Result<Option<Manifest>, Error>) -> Option<State> {
        self.checking = false;
        Some(match result {
            Ok(Some(m)) => State::Available(m),
            Ok(None) => State::Current,
            Err(Error::Unsupported) => State::Unsupported,
            Err(Error::Failed(e)) => {
                self.retry = e.retry_after.and_then(|d| std::time::Instant::now().checked_add(d));
                if self.manual { State::Failed(e.message) } else { return None }
            },
        })
    }

    /// Only the download the user started may continue into saving/restarting.
    fn downloaded(&mut self, m: Manifest, result: Result<PathBuf, http::Failure>) -> bool {
        if !matches!(&self.state, State::Downloading(active) if active == &m) { return false }
        self.state = match result {
            Ok(path) => State::Ready(m, path),
            Err(e) => {
                self.retry = e.retry_after.and_then(|d| std::time::Instant::now().checked_add(d));
                State::Failed(e.message)
            },
        };
        matches!(self.state, State::Ready(..))
    }
}

pub fn state(cx: &App) -> State { cx.try_global::<Updater>().map_or(State::Idle, |u| u.state.clone()) }

#[cfg(feature = "devtools")]
pub(crate) fn check_script_state(expected: &str, cx: &App) {
    let (expected, feedback) = expected.split_once(' ').map_or((expected, None), |(s, f)| (s, Some(f)));
    let state = match state(cx) {
        State::Idle => "idle", State::Unsupported => "unsupported", State::Checking => "checking", State::Current => "current",
        State::Available(_) => "available", State::Downloading(_) => "downloading", State::Ready(..) => "ready",
        State::Installing(_) => "installing", State::Failed(_) => "failed",
    };
    assert_eq!(state, expected);
    if let Some(feedback) = feedback {
        let visible = cx.windows().into_iter().filter_map(|w| w.downcast::<crate::app::Workspace>()).any(|w| {
            w.read(cx).is_ok_and(|ws| ws.toast.as_ref().is_some_and(|(_, _, t)| t.elapsed() < Duration::from_millis(2700)))
        });
        assert_eq!(visible, feedback == "feedback", "update feedback: {feedback}");
    }
    eprintln!("update-state: {state}");
}

fn set(state: State, cx: &mut App) {
    cx.global_mut::<Updater>().state = state;
    cx.refresh_windows();
}

/// Starts the periodic check.
pub fn init(cx: &mut App) {
    let state = install::previous_error().map_or(State::Idle, State::Failed);
    cx.set_global(Updater { state, told: None, last: None, checking: false, manual: false, retry: None });
    cx.spawn(async |cx| {
        cx.background_executor().timer(Duration::from_secs(10)).await;
        loop {
            if cx.update(|cx| if crate::settings::prefs().auto_update { check(false, cx) }).is_err() { break }
            cx.background_executor().timer(Duration::from_secs(12 * 3600)).await;
        }
    }).detach();
}

/// A window came to the front: check again if the last check is over an
/// hour old, so a copy left open learns of a release within hours, not 12.
pub fn check_if_stale(cx: &mut App) {
    let Some(u) = cx.try_global::<Updater>() else { return };
    // Before the first check (10s after launch) the timer has it.
    if !crate::settings::prefs().auto_update || u.last.is_none_or(|t| t.elapsed() < Duration::from_secs(3600)) { return }
    check(false, cx);
}

/// Asks the server for the newest version. `manual` (检查更新…) also reports
/// failures and an up-to-date copy; the automatic check stays silent except
/// for a toast, once per version, when there is something new.
pub fn check(manual: bool, cx: &mut App) {
    match cx.global_mut::<Updater>().begin(manual) {
        Check::Busy => return,
        Check::Joined => { if manual { set(State::Checking, cx); } return },
        Check::Wait => {
            if manual {
                let state = State::Failed(crate::settings::tr("更新服务暂时拒绝请求，请稍后重试", "The update service is busy. Try again later.").into());
                report(&state, cx); set(state, cx);
            }
            return
        },
        Check::Start => {},
    }
    if manual { set(State::Checking, cx); }
    let task = cx.background_executor().spawn(async { fetch_manifest() });
    cx.spawn(async move |cx| {
        let result = task.await;
        let _ = cx.update(|cx| {
            let u = cx.global_mut::<Updater>();
            let manual = u.manual;
            let Some(state) = u.complete(result) else { return };
            if manual { report(&state, cx); }
            if let State::Available(m) = &state && !manual && cx.global::<Updater>().told.as_ref() != Some(&m.version) {
                cx.global_mut::<Updater>().told = Some(m.version.clone());
                announce(&m.version, cx);
            }
            set(state, cx);
        });
    }).detach();
}

pub(crate) fn unsupported_message() -> &'static str {
    crate::settings::tr("这个版本不能自动更新，请从 github.com/iuxt/quiet-md/releases 下载新版本。", "This copy cannot update itself. Download from github.com/iuxt/quiet-md/releases.")
}

/// A repeated manual check must still give feedback when its result is unchanged.
fn report(state: &State, cx: &mut App) {
    let (kind, msg) = match state {
        State::Unsupported => (crate::ui::ToastKind::Info, unsupported_message()),
        State::Current => (crate::ui::ToastKind::Success, crate::settings::tr("已是最新版本", "Quiet.md is up to date")),
        State::Failed(e) => (crate::ui::ToastKind::Error, e.as_str()),
        State::Available(m) => { announce(&m.version, cx); return },
        _ => return,
    };
    let window = cx.active_window().or_else(|| cx.windows().first().copied()).and_then(|w| w.downcast::<crate::app::Workspace>());
    let msg = msg.to_string();
    cx.defer(move |cx| if let Some(w) = window { let _ = w.update(cx, |ws, _, cx| ws.show_toast_kind(kind, &msg, cx)); });
}

/// A toast in the front window: the version and the sidebar's 更新 pill.
fn announce(version: &str, cx: &mut App) {
    let msg = crate::localized_format!("Quiet.md {} 已发布，点侧栏顶部的「更新」安装", "Quiet.md {} is available. Click Update at the top of the sidebar.", version);
    let window = cx.active_window().or_else(|| cx.windows().first().copied()).and_then(|w| w.downcast::<crate::app::Workspace>());
    if let Some(w) = window { let _ = w.update(cx, |ws, _, cx| ws.show_toast_kind(crate::ui::ToastKind::Info, &msg, cx)); }
}

/// How much of `m` has arrived, 0–1.
pub fn progress(m: &Manifest) -> f32 {
    if m.size == 0 { 0. } else { (DOWNLOADED.load(Ordering::Relaxed) as f32 / m.size as f32).clamp(0., 1.) }
}

/// One click downloads, verifies, saves all windows and restarts to install.
pub fn start_update(cx: &mut App) {
    let State::Available(m) = state(cx) else { return };
    if cx.global::<Updater>().checking { return }
    DOWNLOADED.store(0, Ordering::Relaxed);
    set(State::Downloading(m.clone()), cx);
    // Redraw the progress while the download runs.
    cx.spawn(async |cx| loop {
        cx.background_executor().timer(Duration::from_millis(300)).await;
        if !cx.update(|cx| if matches!(state(cx), State::Downloading(_)) { cx.refresh_windows(); true } else { false }).unwrap_or(false) { break }
    }).detach();
    let task = cx.background_executor().spawn({ let m = m.clone(); async move { fetch_package(&m) } });
    cx.spawn(async move |cx| {
        let result = task.await;
        let _ = cx.update(|cx| {
            let restart = cx.global_mut::<Updater>().downloaded(m, result);
            cx.refresh_windows();
            if restart { install(cx); }
        });
    }).detach();
}

/// Hands the downloaded version to a helper that installs it once this
/// process has exited, then quits (asking first if a document did not save).
pub fn install(cx: &mut App) {
    cx.defer(|cx| crate::app::quit_after_saving(finish_install, cx));
}

fn finish_install(cx: &mut App) {
    let State::Ready(m, path) = state(cx) else { return };
    match install::start(&path) {
        Ok(()) => { set(State::Installing(m), cx); cx.quit(); }
        Err(e) => {
            // Keep the verified package available for another restart attempt.
            if let Some(w) = cx.windows().into_iter().find_map(|w| w.downcast::<crate::app::Workspace>()) {
                let _ = w.update(cx, |ws, _, cx| ws.show_toast(&e, cx));
            }
        },
    }
}

// ---- the server

enum Error { Unsupported, Failed(http::Failure) }


fn platform() -> Option<&'static str> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) { Some("macos-arm64") }
    else if cfg!(all(target_os = "macos", target_arch = "x86_64")) { Some("macos-x86_64") }
    else if cfg!(all(target_os = "windows", target_arch = "x86_64")) { Some("windows-x64") }
    else if cfg!(all(target_os = "linux", target_arch = "x86_64")) { Some("linux-x86_64") }
    else { None }
}

fn package_name(version: &str, platform: &str) -> String {
    if platform.starts_with("windows") { format!("quiet-md-{version}-{platform}-setup.exe") }
    else if platform == "linux-x86_64" { format!("quiet-md-{version}-x86_64.AppImage") }
    else { format!("quiet-md-{version}-{platform}.zip") }
}

fn override_url() -> Option<String> { std::env::var("QUIET_MD_UPDATE_URL").ok().filter(|u| !u.is_empty()) }

fn public_key() -> Option<minisign_verify::PublicKey> {
    minisign_verify::PublicKey::decode(KEY.trim()).ok().or_else(|| minisign_verify::PublicKey::from_base64(KEY.trim()).ok())
}

/// `a` is a later version than `b` (dotted numbers; a missing part is 0).
pub(crate) fn newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Option<Vec<u64>> { v.split('.').map(|p| p.parse().ok()).collect() };
    let (Some(mut a), Some(mut b)) = (parts(a), parts(b)) else { return false };
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

/// This copy can replace itself: an installed, release-signed build.
fn supported() -> bool {
    if platform().is_none() || public_key().is_none() { return false }
    if override_url().is_some() { return true }
    if crate::devtools::scripted() || cfg!(debug_assertions) { return false }
    #[cfg(target_os = "macos")]
    { install::app_bundle().is_some_and(|app| team(&app).is_some() && app.file_name().is_some_and(|n| n == "quiet.md.app")) }
    #[cfg(target_os = "windows")]
    { std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("unins000.exe").is_file())).unwrap_or(false) }
    #[cfg(target_os = "linux")]
    { install::appimage().is_some() }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    { false }
}

/// The newest version when it is later than this one.
fn fetch_manifest() -> Result<Option<Manifest>, Error> {
    if !supported() { return Err(Error::Unsupported) }
    let platform = platform().unwrap();
    let url = format!("{}/update-{platform}.json", override_url().as_deref().unwrap_or(BASE).trim_end_matches('/'));
    let m = read_manifest(&url, platform, override_url().is_some()).map_err(Error::Failed)?;
    Ok(newer(&m.version, env!("CARGO_PKG_VERSION")).then_some(m))
}

fn read_manifest(url: &str, platform: &str, testing: bool) -> Result<Manifest, http::Failure> {
    let mut body = vec![];
    http::get(url, 64 * 1024, 20, testing, &mut body, |_| {})?;
    let m: Manifest = serde_json::from_slice(&body).map_err(|e| http::Failure::message(format!("{}: {e}", crate::settings::tr("更新信息无法识别", "Unrecognised update information"))))?;
    validate(&m, platform, testing).map_err(http::Failure::message)?;
    Ok(m)
}

fn release_version(version: &str) -> bool {
    let parts: Vec<_> = version.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) && (p.len() == 1 || !p.starts_with('0')) && p.parse::<u64>().is_ok())
}

/// The manifest names this platform's package for its version, over HTTPS,
/// with a signature for that very file.
fn validate(m: &Manifest, platform: &str, testing: bool) -> Result<(), String> {
    let name = package_name(&m.version, platform);
    let bad = |what: &str| Err(format!("{}: {what}", crate::settings::tr("更新信息无效", "Invalid update information")));
    if !release_version(&m.version) { return bad("version") }
    let expected = format!("{PACKAGES}/v{}/{name}", m.version);
    let test_url = testing && (m.url.starts_with("http://") || m.url.starts_with("https://")) && m.url.ends_with(&format!("/{name}"));
    if m.url != expected && !test_url { return bad("url") }
    if m.size == 0 || m.size > 1 << 30 { return bad("size") }
    match minisign_verify::Signature::decode(&m.signature) { Ok(sig) if sig.trusted_comment() == name => Ok(()), _ => bad("signature") }
}

/// `bytes` carry a signature by `key` whose trusted comment is the package name.
fn signed(bytes: &[u8], signature: &str, name: &str, key: &minisign_verify::PublicKey) -> bool {
    minisign_verify::Signature::decode(signature).is_ok_and(|sig| sig.trusted_comment() == name && key.verify(bytes, &sig, false).is_ok())
}

fn updates_dir() -> PathBuf { dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("quiet-md").join("updates") }

fn private_dir(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    { use std::os::unix::fs::DirBuilderExt; std::fs::DirBuilder::new().mode(0o700).create(path).map_err(|e| e.to_string()) }
    #[cfg(not(unix))]
    { std::fs::create_dir(path).map_err(|e| e.to_string()) }
}

/// Downloads, checks the signature and (macOS) unpacks; the path to install.
fn fetch_package(m: &Manifest) -> Result<PathBuf, http::Failure> {
    let platform = platform().ok_or("platform")?;
    let name = package_name(&m.version, platform);
    std::fs::create_dir_all(updates_dir()).map_err(|e| e.to_string())?;
    let dir = updates_dir().join(uuid::Uuid::new_v4().to_string());
    private_dir(&dir)?;
    let result = prepare_package(m, platform, &name, &dir);
    if result.is_err() { let _ = std::fs::remove_dir_all(&dir); }
    result
}

fn prepare_package(m: &Manifest, platform: &str, name: &str, dir: &Path) -> Result<PathBuf, http::Failure> {
    let key = public_key().ok_or("Update signing key is not configured")?;
    let file = verified_package(m, platform, name, dir, &key, override_url().is_some())?;
    #[cfg(target_os = "macos")]
    let failed = |what: &str| format!("{}: {what}", crate::settings::tr("更新包校验失败", "The update package failed verification"));
    #[cfg(target_os = "macos")]
    {
        let unpacked = dir.join("app");
        run(Command::new("/usr/bin/ditto").args(["-x", "-k"]).arg(&file).arg(&unpacked))?;
        let app = unpacked.join("quiet.md.app");
        run(Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict"]).arg(&app)).map_err(|_| failed("codesign"))?;
        // Signed by the same team as this copy (unsigned only against unsigned, when testing).
        let mine = install::app_bundle().and_then(|a| team(&a));
        if team(&app) != mine || mine.is_none() && override_url().is_none() { return Err(failed("team").into()) }
        let _ = std::fs::remove_file(&file);
        Ok(app)
    }
    #[cfg(not(target_os = "macos"))]
    Ok(file)
}

fn verified_package(m: &Manifest, platform: &str, name: &str, dir: &Path, key: &minisign_verify::PublicKey, testing: bool) -> Result<PathBuf, http::Failure> {
    validate(m, platform, testing)?;
    let file = dir.join(name);
    let part = dir.join(format!("{name}.part"));
    // curl gets no path: a Windows curl.exe reads its arguments in the ANSI
    // code page, which cannot spell every user folder name (中文 included).
    let mut out = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    http::get(&m.url, m.size, 900, testing, &mut out, |n| DOWNLOADED.store(n, Ordering::Relaxed))?;
    out.sync_all().map_err(|e| e.to_string())?;
    drop(out);
    let bytes = std::fs::read(&part).map_err(|e| e.to_string())?;
    let failed = |what: &str| format!("{}: {what}", crate::settings::tr("更新包校验失败", "The update package failed verification"));
    if bytes.len() as u64 != m.size { return Err(failed("size").into()) }
    if !signed(&bytes, &m.signature, name, key) { return Err(failed("signature").into()) }
    drop(bytes);
    std::fs::rename(&part, &file).map_err(|e| e.to_string())?;
    Ok(file)
}

/// A command that opens no console window on Windows.
fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(target_os = "windows")]
    { use std::os::windows::process::CommandExt; c.creation_flags(0x0800_0000); }
    c
}

#[cfg(target_os = "macos")]
fn run(c: &mut Command) -> Result<(), String> {
    let out = c.stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
    if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
}

#[cfg(target_os = "macos")]
fn team(app: &Path) -> Option<String> {
    let out = Command::new("/usr/bin/codesign").args(["-d", "-v"]).arg(app).stdin(Stdio::null()).output().ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    let id = text.lines().find_map(|l| l.strip_prefix("TeamIdentifier="))?.trim();
    (id != "not set" && !id.is_empty()).then(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::{newer, signed, validate, Manifest};

    #[test]
    fn only_verified_downloads_started_by_the_user_continue_to_install() {
        use super::{State, Updater};
        let m = manifest("0.2.0", "https://github.com/iuxt/quiet-md/releases/download/v0.2.0/quiet-md-0.2.0-macos-arm64.zip", "quiet-md-0.2.0-macos-arm64.zip");
        let mut u = Updater { state: State::Idle, told: None, last: None, checking: false, manual: false, retry: None };
        let path = std::path::PathBuf::from("verified-package");
        // Discovering a release never installs or downloads it on its own.
        assert_eq!(u.complete(Ok(Some(m.clone()))), Some(State::Available(m.clone())));
        u.state = State::Available(m.clone());
        assert!(!u.downloaded(m.clone(), Ok(path.clone())));
        assert_eq!(u.state, State::Available(m.clone()));
        u.state = State::Downloading(m.clone());
        assert!(u.downloaded(m.clone(), Ok(path.clone())));
        assert_eq!(u.state, State::Ready(m.clone(), path));
        // A repeated completion cannot queue a second restart.
        assert!(!u.downloaded(m.clone(), Ok("duplicate".into())));
        u.state = State::Downloading(m.clone());
        assert!(!u.downloaded(m.clone(), Err(super::http::Failure::message("signature"))));
        assert_eq!(u.state, State::Failed("signature".into()));
        u.state = State::Downloading(m.clone());
        let mut stale = m.clone(); stale.version = "0.1.0".into();
        assert!(!u.downloaded(stale, Ok("stale".into())));
        assert_eq!(u.state, State::Downloading(m));
    }

    #[test]
    fn checks_join_preserve_available_updates_and_respect_retry_after() {
        use super::{Check, Error, State, Updater};
        let m = manifest("0.2.0", "https://github.com/iuxt/quiet-md/releases/download/v0.2.0/quiet-md-0.2.0-macos-arm64.zip", "quiet-md-0.2.0-macos-arm64.zip");
        let mut u = Updater { state: State::Available(m.clone()), told: None, last: None, checking: false, manual: false, retry: None };
        assert_eq!(u.begin(false), Check::Start);
        assert_eq!(u.begin(false), Check::Joined);
        assert_eq!(u.state, State::Available(m.clone()));
        assert!(u.complete(Err(Error::Failed(super::http::Failure::message("offline")))).is_none());
        assert!(!u.checking);
        assert_eq!(u.begin(false), Check::Start);
        assert_eq!(u.begin(true), Check::Joined);
        let error = super::http::Failure { message: "rate limited".into(), retry_after: Some(std::time::Duration::from_secs(120)) };
        assert_eq!(u.complete(Err(Error::Failed(error))), Some(State::Failed("rate limited".into())));
        assert_eq!(u.begin(true), Check::Wait);
        u.retry = None;
        assert_eq!(u.begin(true), Check::Start);
        assert_eq!(u.complete(Ok(None)), Some(State::Current));
        for state in [State::Downloading(m.clone()), State::Ready(m.clone(), "package".into()), State::Installing(m)] {
            u.state = state;
            assert_eq!(u.begin(true), Check::Busy);
        }
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(newer("0.1.10", "0.1.9"));
        assert!(newer("0.2", "0.1.9"));
        assert!(newer("1.0.0", "0.99.99"));
        assert!(!newer("0.1.3", "0.1.3"));
        assert!(!newer("0.1.3.0", "0.1.3"));
        assert!(!newer("0.1.2", "0.1.3"));
        assert!(!newer("0.1.x", "0.1.3"));
    }

    // A throwaway key (not the release key) and its signature of b"quiet.md" as quiet-md-0.2.0-macos-arm64.zip.
    const TEST_KEY: &str = "RWTh6tW62t9YuAQn7kot+dQgLqLst7Y75FmIEJTHVhcBtN0WXRpyTuPV";
    const TEST_SIG: &str = "untrusted comment: signature from minisign secret key
RUTh6tW62t9YuELfpasf2JJ5CdktflamMcMROwLwYH7Ps6KTTQ/OUvViqT/c+L9PB1Egk9FoQqb94hk+x00y66OvPsdZOwwMww4=
trusted comment: quiet-md-0.2.0-macos-arm64.zip
FuE4L1zQnqL1nA/5xffpEtZyK2ca882Q1joA5X11poZ1xezdAlEMfn/36MJc5C60clQFec4vQf66MoxPi+AZCA==
";

    fn manifest(version: &str, url: &str, comment: &str) -> Manifest {
        let signature = TEST_SIG.replace("trusted comment: quiet-md-0.2.0-macos-arm64.zip", &format!("trusted comment: {comment}"));
        Manifest { version: version.into(), url: url.into(), size: 8, signature }
    }

    #[test]
    fn packages_verify_only_under_their_own_name() {
        let key = minisign_verify::PublicKey::from_base64(TEST_KEY).unwrap();
        let name = "quiet-md-0.2.0-macos-arm64.zip";
        assert!(signed(b"quiet.md", TEST_SIG, name, &key));
        assert!(!signed(b"quiet.mD", TEST_SIG, name, &key));
        assert!(!signed(b"quiet.md", TEST_SIG, "quiet-md-0.9.0-macos-arm64.zip", &key));
        // Rewriting the trusted comment breaks the signature.
        let renamed = TEST_SIG.replace("0.2.0", "0.9.0");
        assert!(!signed(b"quiet.md", &renamed, "quiet-md-0.9.0-macos-arm64.zip", &key));
    }

    #[test]
    fn update_urls_and_versions_are_exact() {
        let original = manifest("0.2.0", "https://github.com/iuxt/quiet-md/releases/download/v0.2.0/quiet-md-0.2.0-macos-arm64.zip", "quiet-md-0.2.0-macos-arm64.zip");
        for url in [original.url.replace("iuxt/", "s1dashu/"), original.url.replace("v0.2.0/", "v0.1.0/"), format!("{}?other=1", original.url), original.url.replace("github.com/", "github.com.evil.example/"), original.url.replace("https:", "http:")] {
            let mut m = original.clone(); m.url = url;
            assert!(validate(&m, "macos-arm64", false).is_err());
        }
        for version in ["", "1.2", "1.2.3.4", "01.2.3", "+1.2.3", "1.2.3-beta", "1.2.18446744073709551616", "../2.0"] {
            let mut m = original.clone(); m.version = version.into();
            assert!(validate(&m, "macos-arm64", false).is_err());
        }
        for size in [0, (1 << 30) + 1] {
            let mut m = original.clone(); m.size = size;
            assert!(validate(&m, "macos-arm64", false).is_err());
        }
    }

    #[test]
    fn local_manifest_and_download_verify_content_before_ready() {
        use std::{io::{Read, Write}, net::TcpListener};
        let key = minisign_verify::PublicKey::from_base64(TEST_KEY).unwrap();
        let root = std::env::temp_dir().join(format!("quiet download 中文 {}", uuid::Uuid::new_v4()));
        super::private_dir(&root).unwrap();
        for content in [b"quiet.md".as_slice(), b"quiet.mD", b"short", b"quiet.md plus extra"] {
            let server = TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", server.local_addr().unwrap());
            let name = "quiet-md-0.2.0-macos-arm64.zip";
            let m = manifest("0.2.0", &format!("{base}/{name}"), name);
            let body = serde_json::to_vec(&serde_json::json!({"version": m.version, "url": m.url, "size": m.size, "signature": m.signature})).unwrap();
            let content = content.to_vec();
            let good = content == b"quiet.md";
            let thread = std::thread::spawn(move || {
                for body in [body, content] {
                    let (mut stream, _) = server.accept().unwrap();
                    stream.set_read_timeout(Some(std::time::Duration::from_secs(3))).unwrap();
                    let _ = stream.read(&mut [0; 4096]);
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
                    let _ = stream.write_all(&body);
                }
            });
            let m = super::read_manifest(&format!("{base}/update-macos-arm64.json"), "macos-arm64", true).unwrap();
            let result = super::verified_package(&m, "macos-arm64", name, &root, &key, true);
            assert_eq!(result.is_ok(), good);
            if good { assert_eq!(std::fs::read(result.unwrap()).unwrap(), b"quiet.md"); }
            thread.join().unwrap();
            for entry in std::fs::read_dir(&root).unwrap() { std::fs::remove_file(entry.unwrap().path()).unwrap(); }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn manifests_must_name_the_signed_package() {
        let ok = manifest("0.2.0", "https://github.com/iuxt/quiet-md/releases/download/v0.2.0/quiet-md-0.2.0-macos-arm64.zip", "quiet-md-0.2.0-macos-arm64.zip");
        assert_eq!(validate(&ok, "macos-arm64", false), Ok(()));
        assert!(validate(&ok, "macos-x86_64", false).is_err());
        let intel = manifest("0.2.0", "https://github.com/iuxt/quiet-md/releases/download/v0.2.0/quiet-md-0.2.0-macos-x86_64.zip", "quiet-md-0.2.0-macos-x86_64.zip");
        assert_eq!(validate(&intel, "macos-x86_64", false), Ok(()));
        assert!(validate(&intel, "macos-arm64", false).is_err());
        // An older package's signature under a newer version number.
        let replay = manifest("0.9.0", "https://github.com/iuxt/quiet-md/releases/download/v0.2.0/quiet-md-0.2.0-macos-arm64.zip", "quiet-md-0.2.0-macos-arm64.zip");
        assert!(validate(&replay, "macos-arm64", false).is_err());
        let renamed = manifest("0.9.0", "https://github.com/iuxt/quiet-md/releases/download/v0.9.0/quiet-md-0.9.0-macos-arm64.zip", "quiet-md-0.2.0-macos-arm64.zip");
        assert!(validate(&renamed, "macos-arm64", false).is_err());
        let http = manifest("0.2.0", "http://github.com/iuxt/quiet-md/releases/download/v0.2.0/quiet-md-0.2.0-macos-arm64.zip", "quiet-md-0.2.0-macos-arm64.zip");
        assert!(validate(&http, "macos-arm64", false).is_err());
        assert_eq!(validate(&http, "macos-arm64", true), Ok(()));
        // A genuine signature does not make another host acceptable.
        for url in ["https://evil.example/v0.2.0/quiet-md-0.2.0-macos-arm64.zip", "https://download.quiet.md.evil.example/quiet-md-0.2.0-macos-arm64.zip", "https://evil.example/download.quiet.md/quiet-md-0.2.0-macos-arm64.zip"] {
            assert!(validate(&manifest("0.2.0", url, "quiet-md-0.2.0-macos-arm64.zip"), "macos-arm64", false).is_err(), "{url}");
        }
        let windows = manifest("0.2.0", "https://github.com/iuxt/quiet-md/releases/download/v0.2.0/quiet-md-0.2.0-windows-x64-setup.exe", "quiet-md-0.2.0-windows-x64-setup.exe");
        assert_eq!(validate(&windows, "windows-x64", false), Ok(()));
        assert!(validate(&windows, "macos-arm64", false).is_err());
        let linux = manifest("0.2.0", "https://github.com/iuxt/quiet-md/releases/download/v0.2.0/quiet-md-0.2.0-x86_64.AppImage", "quiet-md-0.2.0-x86_64.AppImage");
        assert_eq!(validate(&linux, "linux-x86_64", false), Ok(()));
        assert!(validate(&linux, "windows-x64", false).is_err());
        let wrong_linux = manifest("0.2.0", "https://github.com/iuxt/quiet-md/releases/download/v0.2.0/quiet-md-0.2.0-linux-x86_64.zip", "quiet-md-0.2.0-linux-x86_64.zip");
        assert!(validate(&wrong_linux, "linux-x86_64", false).is_err());
    }
}

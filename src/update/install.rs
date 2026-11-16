//! Installation runs in a detached copy of this executable, before GPUI or the
//! library is initialized. File locks coordinate installed copies, not libraries.
use super::{private_dir, updates_dir};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

struct Instance { _file: File, lease: PathBuf, directory: PathBuf }
static INSTANCE: OnceLock<Instance> = OnceLock::new();
const HELPER: &str = "--quiet-md-update-helper";

#[derive(Serialize, Deserialize)]
struct Job { target: PathBuf, new: PathBuf, directory: PathBuf, identity: String, testing: bool, arguments: Vec<std::ffi::OsString> }

fn target() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    if let Some(app) = app_bundle() { return Ok(app) }
    #[cfg(target_os = "linux")]
    if let Some(image) = appimage() { return Ok(image) }
    std::env::current_exe().and_then(|p| p.canonicalize()).map_err(|e| e.to_string())
}

fn directory(target: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hash = std::hash::DefaultHasher::new();
    let name = target.to_string_lossy();
    let name = if cfg!(windows) { name.to_lowercase() } else { name.into_owned() };
    name.hash(&mut hash);
    updates_dir().join(format!("install-{:016x}", hash.finish()))
}

fn lock(directory: &Path) -> Result<File, String> {
    OpenOptions::new().create(true).truncate(false).read(true).write(true).open(directory.join("gate.lock")).map_err(|e| e.to_string())
}

pub fn register_instance() -> Result<(), String> {
    let directory = directory(&target()?);
    std::fs::create_dir_all(updates_dir()).map_err(|e| e.to_string())?;
    if let Err(e) = private_dir(&directory) && !directory.is_dir() { return Err(e) }
    let gate = lock(&directory)?;
    gate.lock().map_err(|e| e.to_string())?;
    let lease = directory.join(format!("{}.running", uuid::Uuid::new_v4()));
    let file = OpenOptions::new().create_new(true).read(true).write(true).open(&lease).map_err(|e| e.to_string())?;
    file.lock().map_err(|e| e.to_string())?;
    INSTANCE.set(Instance { _file: file, lease, directory }).map_err(|_| "Updater already initialized".to_string())?;
    Ok(())
}

fn others_running(directory: &Path, own: Option<&Path>) -> Result<bool, String> {
    for entry in std::fs::read_dir(directory).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().is_none_or(|e| e != "running") || own == Some(path.as_path()) { continue }
        let file = OpenOptions::new().read(true).write(true).open(&path).map_err(|e| e.to_string())?;
        if file.try_lock().is_err() { return Ok(true) }
        drop(file);
        let _ = std::fs::remove_file(path);
    }
    Ok(false)
}

fn identity(target: &Path) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    let target = target.join("Contents/MacOS/quiet_md");
    let metadata = std::fs::metadata(target).map_err(|e| e.to_string())?;
    let modified = metadata.modified().map_err(|e| e.to_string())?.duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?;
    Ok(format!("{}:{}", metadata.len(), modified.as_nanos()))
}

pub(super) fn previous_error() -> Option<String> {
    let path = directory(&target().ok()?).join("result.txt");
    let error = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(path);
    Some(format!("{}: {error}", crate::settings::tr("上次更新未完成", "The previous update did not complete")))
}

pub(super) fn start(new: &Path) -> Result<(), String> {
    let instance = INSTANCE.get().ok_or("Updater instance lock is unavailable")?;
    let gate = lock(&instance.directory)?;
    gate.try_lock().map_err(|_| crate::settings::tr("另一个实例正在更新，请稍后重试", "Another copy is updating. Try again later.").to_string())?;
    if others_running(&instance.directory, Some(&instance.lease))? {
        return Err(crate::settings::tr("请先关闭同一安装位置的其他 Quiet.md 实例，再更新", "Close other Quiet.md copies running from this installation, then try again.").into())
    }
    let target = target()?;
    #[cfg(target_os = "macos")]
    if target.to_string_lossy().contains("/AppTranslocation/") { return Err(crate::settings::tr("请先把 Quiet.md 移到「应用程序」文件夹，再更新", "Move Quiet.md to the Applications folder first, then update").into()) }
    let stage = updates_dir().join(uuid::Uuid::new_v4().to_string());
    private_dir(&stage)?;
    let result = (|| {
        let new = stage_update(new, &target)?;
        let job = Job { identity: identity(&target)?, target, new, directory: instance.directory.clone(), testing: super::override_url().is_some(), arguments: std::env::args_os().skip(1).collect() };
        let path = stage.join("job.json");
        std::fs::write(&path, serde_json::to_vec(&job).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        // Keep the helper executable out of the bundle that it will replace.
        let helper = stage.join(if cfg!(windows) { "helper.exe" } else { "helper" });
        std::fs::copy(std::env::current_exe().map_err(|e| e.to_string())?, &helper).map_err(|e| e.to_string())?;
        #[cfg(windows)]
        {
            let runtime = std::env::current_exe().map_err(|e| e.to_string())?.with_file_name("vcruntime140.dll");
            if runtime.is_file() { std::fs::copy(&runtime, stage.join("vcruntime140.dll")).map_err(|e| e.to_string())?; }
        }
        let mut command = Command::new(helper);
        command.arg(HELPER).arg(path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        #[cfg(unix)]
        { use std::os::unix::process::CommandExt; command.process_group(0); }
        #[cfg(windows)]
        { use std::os::windows::process::CommandExt; command.creation_flags(0x0800_0000 | 0x0000_0200); }
        command.spawn().map(drop).map_err(|e| e.to_string())
    })();
    if result.is_err() { let _ = std::fs::remove_dir_all(&stage); }
    result
}

/// Stage beside the target, so replacement uses renames on one filesystem.
fn stage_update(new: &Path, target: &Path) -> Result<PathBuf, String> {
    #[cfg(windows)]
    { let _ = target; Ok(new.to_path_buf()) }
    #[cfg(unix)]
    {
        let parent = target.parent().ok_or("Missing installation directory")?;
        let stage = parent.join(format!(".quiet-md-update-{}", uuid::Uuid::new_v4()));
        private_dir(&stage).map_err(|e| crate::localized_format!("无法在 {} 准备更新，请确认目录可写：{}", "Could not prepare the update in {}. Check directory permissions: {}", parent.display(), e))?;
        let staged = stage.join("new");
        let result = (|| {
            #[cfg(target_os = "macos")]
            {
                super::run(Command::new("/usr/bin/ditto").arg(new).arg(&staged))?;
                verify_app(&staged, target, super::override_url().is_some())?;
            }
            #[cfg(not(target_os = "macos"))]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::copy(new, &staged).map_err(|e| e.to_string())?;
                std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
                File::open(&staged).and_then(|f| f.sync_all()).map_err(|e| e.to_string())?;
            }
            Ok(staged)
        })();
        if result.is_err() { let _ = std::fs::remove_dir_all(stage); }
        result
    }
}

pub fn run_helper() -> bool {
    let args: Vec<_> = std::env::args_os().collect();
    if args.get(1).is_none_or(|s| s != HELPER) { return false }
    let result = (|| -> Result<(), String> {
        if args.len() != 3 { return Err("Invalid update helper arguments".into()) }
        let path = Path::new(&args[2]);
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        if bytes.len() > 65536 { return Err("Invalid update helper job".into()) }
        let job: Job = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let result = perform(&job);
        if let Err(e) = &result { let _ = std::fs::write(job.directory.join("result.txt"), e); }
        else { let _ = std::fs::remove_file(job.directory.join("result.txt")); }
        // Windows cannot remove its running helper; retain failed packages for diagnostics.
        if result.is_ok() { let _ = std::fs::remove_dir_all(path.parent().unwrap()); }
        result
    })();
    std::process::exit(if result.is_ok() { 0 } else { 1 });
}

fn perform(job: &Job) -> Result<(), String> {
    let gate = lock(&job.directory)?;
    gate.lock().map_err(|e| e.to_string())?;
    let until = Instant::now() + Duration::from_secs(60);
    while others_running(&job.directory, None)? {
        if Instant::now() >= until { return Err("Quiet.md is still running; installation was cancelled".into()) }
        std::thread::sleep(Duration::from_millis(100));
    }
    if identity(&job.target)? != job.identity { return Err("The installation changed while the update was waiting".into()) }
    install(job)
}

#[cfg(target_os = "macos")]
fn verify_app(new: &Path, target: &Path, testing: bool) -> Result<(), String> {
    super::run(Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict"]).arg(new))?;
    let mine = super::team(target);
    if super::team(new) != mine || mine.is_none() && !testing { return Err("Update code-signature team does not match".into()) }
    Ok(())
}

#[cfg(unix)]
fn swap(target: &Path, new: &Path, old: &Path) -> Result<(), String> {
    std::fs::rename(target, old).map_err(|e| e.to_string())?;
    if let Err(e) = std::fs::rename(new, target) {
        std::fs::rename(old, target).map_err(|restore| format!("{e}; restore failed: {restore}; previous app: {}", old.display()))?;
        return Err(e.to_string())
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn install(job: &Job) -> Result<(), String> {
    verify_app(&job.new, &job.target, job.testing)?;
    let stage = job.new.parent().ok_or("Missing update stage")?;
    let old = stage.join("previous.app");
    swap(&job.target, &job.new, &old)?;
    let mut launch = Command::new("/usr/bin/open");
    launch.arg("-n");
    for key in ["QUIET_MD_HOME", "QUIET_MD_UPDATE_URL"] {
        if let Ok(value) = std::env::var(key) { launch.arg("--env").arg(format!("{key}={value}")); }
    }
    if !launch.arg(&job.target).status().is_ok_and(|s| s.success()) {
        std::fs::rename(&job.target, &job.new).map_err(|e| e.to_string())?;
        std::fs::rename(&old, &job.target).map_err(|e| e.to_string())?;
        return Err("Could not restart the updated application; the previous app was restored".into())
    }
    let _ = std::fs::remove_dir_all(stage);
    Ok(())
}

#[cfg(windows)]
fn install(job: &Job) -> Result<(), String> {
    let mut destination = std::ffi::OsString::from("/DIR=");
    destination.push(job.target.parent().ok_or("Missing installation directory")?);
    let mut log = std::ffi::OsString::from("/LOG=");
    log.push(job.directory.join("installer.log"));
    let status = super::command(&job.new).args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/NOCLOSEAPPLICATIONS", "/UPDATE=1"])
        .arg(destination).arg(log).stdin(Stdio::null()).status().map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err(format!("The update installer failed: {status}; package: {}", job.new.display())) }
}

#[cfg(target_os = "linux")]
fn install(job: &Job) -> Result<(), String> {
    let stage = job.new.parent().ok_or("Missing update stage")?;
    let old = stage.join("previous.AppImage");
    swap(&job.target, &job.new, &old)?;
    let launch = || {
        super::command(&job.target).args(&job.arguments).env_remove("APPIMAGE").env_remove("APPDIR").env_remove("ARGV0").env_remove("OWD").env_remove("XLOCALEDIR")
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
    };
    let result = (|| {
        let mut child = launch().map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_secs(2));
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? && !status.success() { return Err(format!("Updated AppImage failed to launch: {status}")) }
        Ok(())
    })();
    if let Err(e) = result {
        std::fs::rename(&job.target, &job.new).map_err(|e| e.to_string())?;
        std::fs::rename(&old, &job.target).map_err(|e| e.to_string())?;
        let _ = launch();
        return Err(e)
    }
    let _ = std::fs::remove_dir_all(stage);
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn install(_: &Job) -> Result<(), String> { Err("Unsupported platform".into()) }

#[cfg(target_os = "macos")]
pub(super) fn app_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let app = exe.parent()?.parent()?.parent()?;
    (app.extension()? == "app").then(|| app.to_path_buf())
}

#[cfg(target_os = "linux")]
pub(super) fn appimage() -> Option<PathBuf> {
    let image = PathBuf::from(std::env::var_os("APPIMAGE")?).canonicalize().ok()?;
    let appdir = PathBuf::from(std::env::var_os("APPDIR")?).canonicalize().ok()?;
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let payload = appdir.join("usr/bin/quiet_md").canonicalize().ok()?;
    (image.is_file() && exe == payload && !image.starts_with(&appdir)).then_some(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_copies_are_detected_and_stale_leases_are_reaped() {
        let root = std::env::temp_dir().join(format!("quiet lock 中文 {}", uuid::Uuid::new_v4()));
        private_dir(&root).unwrap();
        let lease = root.join("test.running");
        let file = OpenOptions::new().create_new(true).read(true).write(true).open(&lease).unwrap();
        file.lock().unwrap();
        assert!(others_running(&root, None).unwrap());
        assert!(!others_running(&root, Some(&lease)).unwrap());
        file.unlock().unwrap();
        drop(file);
        assert!(!others_running(&root, None).unwrap());
        assert!(!lease.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn replacement_recovers_when_stage_is_missing() {
        let root = std::env::temp_dir().join(format!("quiet swap 中文 {}", uuid::Uuid::new_v4()));
        private_dir(&root).unwrap();
        let target = root.join("quiet $name");
        let old = root.join("previous");
        let new = root.join("new");
        std::fs::write(&target, "old").unwrap();
        assert!(swap(&target, &new, &old).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "old");
        std::fs::write(&new, "new").unwrap();
        swap(&target, &new, &old).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "old");
        std::fs::remove_dir_all(root).unwrap();
    }
}

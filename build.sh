#!/bin/bash
# Build a local macOS package, replace the installed app and launch it.
set -euo pipefail
cd "$(dirname "$0")"
[ "$(uname -s)" = Darwin ] || { echo "build.sh: 仅支持 macOS。" >&2; exit 1; }
[ -z "${MACOS_TARGET:-}" ] || { echo "build.sh: 请取消 MACOS_TARGET，安装版使用本机架构。" >&2; exit 1; }

installed=/Applications/quiet.md.app
stage=
cleanup() {
  if [ -n "$stage" ]; then
    if [ ! -e "$installed" ] && [ -d "$stage/previous.app" ]; then
      mv "$stage/previous.app" "$installed" || { echo "旧应用保留在 $stage/previous.app" >&2; return; }
    fi
    rm -rf "$stage"
  fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

echo "== 打包本机应用"
out=${RELEASE_OUT:-$PWD/dist-release}
RELEASE_OUT="$out" SIGNING_IDENTITY="${SIGNING_IDENTITY:-local}" scripts/release.sh
app="$out/quiet.md.app"
[ -x "$app/Contents/MacOS/quiet_md" ] || { echo "build.sh: 应用包中缺少可执行文件。" >&2; exit 1; }

echo "== 准备安装到 $installed"
stage=$(mktemp -d /Applications/.quiet-md-install.XXXXXX) || {
  echo "build.sh: 无法写入 /Applications，请检查该目录的权限。" >&2; exit 1;
}
ditto "$app" "$stage/quiet.md.app"
codesign --verify --deep --strict "$stage/quiet.md.app"

echo "== 退出已安装的 Quiet.md（等待保存，最多 60 秒）"
# Match the installed executable path, never a process name or another app.
osascript -l JavaScript - "$installed" <<'JXA'
ObjC.import('AppKit');
function run(argv) {
  var expected = (argv[0] + '/Contents/MacOS/quiet_md').toLowerCase();
  var apps = $.NSWorkspace.sharedWorkspace.runningApplications;
  var targets = [];
  for (var i = 0; i < apps.count; i++) {
    var app = apps.objectAtIndex(i);
    if (!app.executableURL.isNil() && ObjC.unwrap(app.executableURL.path).toLowerCase() === expected) {
      targets.push(app);
      if (!app.terminate) throw new Error('无法请求 Quiet.md 退出，安装已停止。');
    }
  }
  for (var tick = 0; tick < 240; tick++) {
    if (targets.every(function (app) { return Boolean(app.terminated); })) return;
    // NSRunningApplication refreshes its cached state only as the run loop runs.
    $.NSRunLoop.currentRunLoop.runUntilDate($.NSDate.dateWithTimeIntervalSinceNow(0.25));
  }
  throw new Error('Quiet.md 尚未退出，请处理保存提示或取消退出后重试；安装已停止。');
}
JXA

echo "== 安装"
if [ -e "$installed" ]; then mv "$installed" "$stage/previous.app"; fi
mv "$stage/quiet.md.app" "$installed"

echo "== 启动 Quiet.md"
osascript - "$installed" <<'APPLESCRIPT'
on run argv
  tell application (item 1 of argv) to launch
end run
APPLESCRIPT
echo "已安装并启动：$installed"

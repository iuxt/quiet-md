# 版本与发布

Quiet.md 的所有平台都从 [GitHub Releases](https://github.com/iuxt/quiet-md/releases) 手动下载安装。应用内不检查、下载或安装新版本，也不显示升级后的更新日志卡片。版本说明维护在 `docs/releases/<版本号>.md`。

## 版本号与发布 tag

版本号记录在根目录 `VERSION`；`python3 scripts/version.py` 查询当前版本，`bump-major` / `bump-minor` / `bump-patch` 同步修改 `VERSION`、两个 `Cargo.toml` 和 `Cargo.lock`。

发布步骤：

1. 递增版本号，更新 `docs/releases/<版本号>.md`。
2. 运行 `scripts/check.sh`，提交并推送到 `main`，保持工作区干净。
3. 运行 `python3 scripts/version.py release`。脚本要求当前提交是 `origin/main` 的最新提交，为当前版本创建并推送 `v<版本号>` tag；已有 tag 不覆盖、不移动。
4. Tag 触发 `.github/workflows/release.yml`，构建并检查四个平台的安装包，再发布为正式 Release（`prerelease=false`），标记为 Latest。

普通分支 push 和 PR 不触发构建；CI 工作流保留手动验证入口。手动运行 Release 时选择 `main`，`tag` 填已有 tag；默认生成验证过的草稿，勾选 `publish` 才公开。失败且尚未创建 Release 的任务可以重跑，已有同版本 Release 不覆盖。

## 安装包

当前安装包不需要付费开发者账号或平台证书：

| 平台 | 安装包 |
|---|---|
| macOS Apple Silicon | `quiet-md-<version>-macos-arm64-unsigned.zip` |
| macOS Intel | `quiet-md-<version>-macos-x86_64-unsigned.zip` |
| Windows x64 | `quiet-md-<version>-windows-x64-setup.exe` |
| Linux x86_64 | `quiet-md-<version>-x86_64.AppImage` |

macOS 使用 ad-hoc 签名，跳过 Apple 公证；Windows 安装包未签名。每个平台附带 SHA-256 校验文件，共 8 项资产。校验文件用于检查下载完整性。macOS 和 Windows 可能显示系统安全提示。

发布任务先创建草稿、上传全部资产，再重新下载并逐项比较名称、大小和 SHA-256，通过后公开为正式 Release，并标记 Latest。公开后确认不是草稿或 Pre-release，再次验证下载；失败时保留已发布版本供维护者检查，不擅自删除或覆盖。只有最终发布任务使用 GitHub 自动提供的 `GITHUB_TOKEN` 写入 Release。

## 本地准备与发布

`scripts/publish-github-release.py` 接收四个平台的安装包，要求文件名和当前版本一致。以下路径需替换为实际产物：

```sh
python3 scripts/publish-github-release.py --dry-run <arm64-unsigned.zip> <intel-unsigned.zip> <windows-setup.exe> <linux.AppImage>
python3 scripts/publish-github-release.py --prepare dist/test-assets <arm64-unsigned.zip> <intel-unsigned.zip> <windows-setup.exe> <linux.AppImage>
```

`--dry-run` 只检查本地文件，不写文件、不联网；`--prepare` 在新的输出目录中生成 8 项资产，拒绝覆盖已有目录。有随包的 SHA-256 时先验证其内容。

本地发布前需安装并登录 `gh`，保持发布提交的工作区干净，本地和远端 tag 都指向当前提交。将 `--prepare` 改成 `--publish` 即可运行完整检查、上传并验证草稿、公开并复核下载。可选 `--draft` 保留已验证的草稿，`--notes docs/releases/<version>.md` 使用指定版本说明，省略时由 GitHub 生成。选择本地发布时应先停用自动 Release 工作流，避免争用同一个 tag。

平台打包脚本仍支持系统代码签名和 macOS 公证；这些选项用于平台安装与分发。

## 验证

`scripts/check.sh` 覆盖版本一致性、发布脚本、核心与应用测试、示例库往返，以及普通和 devtools 构建的 Clippy。发布脚本测试使用临时安装包并模拟上传；不发布真实 Release。各平台安装与卸载仍需在对应系统验证。

所有测试文档库放临时目录，通过 `QUIET_MD_HOME` 指定；不读取或修改使用者的真实文档库。

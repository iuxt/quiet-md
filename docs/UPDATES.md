# GitHub 自动更新方案

状态：代码已接入 GitHub Releases，并配置分叉公钥；正式发布前仍需配置 Actions 签名凭据，并完成各平台的真实安装验证。本文同时记录实现、发布步骤和验收要求。

## 目标和现状

复用 `src/update.rs` 的下载、Minisign 验签、平台安装器，以及现有设置项、菜单和侧栏更新按钮，把更新源改为 `https://github.com/iuxt/quiet-md` 的正式 Releases。自动检查只发现版本；用户点击一次「更新」后，自动下载、验签、保存所有打开的文档并重启安装。保存失败、取消保存提示或安装准备失败时保留「重启更新」按钮供重试。检查只请求公开发布资源，不上传文档或设备标识，不要求 GitHub 登录。

首次接入时（2026-11-16），该仓库尚无 Release。`packaging/update-key.pub` 已替换为分叉独立公钥；发布环境必须持有与其匹配的私钥。现有 CI 的 macOS 包是 ad-hoc 签名的验证包，不能直接用于正式发布。`build.sh` 默认也产生 ad-hoc 签名的本地安装版，按当前规则不能自动更新。

## 更新源与发布协议

直接使用 GitHub 支持的最新 Release 资源地址，保持已有 JSON 协议，无需引入 Releases API 或在客户端分发 Token：

```text
https://github.com/iuxt/quiet-md/releases/latest/download/update-<platform>.json
```

最新正式 Release 必须包含下表全部平台的清单和安装包；不要把只含部分平台的 Release 标记为 Latest。预发布版本不进入正式更新通道。首版只接受 `MAJOR.MINOR.PATCH` 三段非负整数版本，tag 严格为 `v<version>`，不支持测试版后缀。

| platform | 清单 | 安装包 |
|---|---|---|
| `macos-arm64` | `update-macos-arm64.json` | `quiet-md-<version>-macos-arm64.zip` |
| `macos-x86_64` | `update-macos-x86_64.json` | `quiet-md-<version>-macos-x86_64.zip` |
| `windows-x64` | `update-windows-x64.json` | `quiet-md-<version>-windows-x64-setup.exe` |
| `linux-x86_64` | `update-linux-x86_64.json` | `quiet-md-<version>-x86_64.AppImage` |

每个安装包另附 `<安装包名>.minisig` 和 `<安装包名>.sha256`。清单内嵌签名，客户端检查一次只请求本平台的清单：

```json
{
  "version": "0.1.13",
  "url": "https://github.com/iuxt/quiet-md/releases/download/v0.1.13/quiet-md-0.1.13-macos-arm64.zip",
  "size": 18000000,
  "signature": "<完整的 Minisign 签名文本>"
}
```

示例中的大小和签名仅为占位，发布脚本必须从最终签名、公证完成的包生成实际值。清单 URL 必须精确等于由版本和平台推导出的地址；不接受任意 GitHub 仓库、其他 tag、查询参数或文件名。清单读取 `latest`，真正下载固定到版本地址，避免检查后发布新版本造成资源混用。

清单本身沿用现有的未签名格式。安装包的 Minisign 签名及签名中的 trusted comment（精确的安装包名）共同绑定内容、版本和平台。攻击者修改清单不能伪造可安装的新版；隐藏或重放清单造成的更新延迟不在此协议的防护范围内。

## 检查、下载与界面

保持已有元素、顺序、快捷键、正常状态文案和 Theme token。复用「设置 › 通用 › 自动检查更新」、设置里的软件更新行、菜单「检查更新…」、侧栏更新按钮和下载进度。

- 自动检查沿用启动 10 秒后、每 12 小时，以及窗口重新激活且上次检查超过 1 小时的时机；从关闭切换为开启时立即检查。关闭开关后不再发起自动检查，手动检查仍可用。
- 所有检查共用独立的 `checking` 标记，在发起后台请求前置位，所有结束路径复位。后台检查保留已发现版本的可见状态；手动检查复用正在运行的请求，并能看到该次结果。后台检查期间手动请求会加入同一次检查，避免请求重叠。
- 在下载、待安装和正在安装时不再检查。只在版本严格大于当前编译版本时显示更新；相同或旧版本不降级。自动提示沿用每个进程每个版本一次。
- 侧栏顶部和设置 › 关于均显示「更新」按钮；点击后才请求安装包。完成验签和平台检查后立即进入正常保存退出路径，无需再次点击重启。下载期间仍可编辑笔记，按钮显示下载进度；提示文案提前说明下载后会自动重启。
- 自动检查失败静默保留之前状态，等下一次正常检查；手动检查提供可重试的明确错误。错误提示使用现有本地化入口。

| 情况 | 手动检查 / 下载的结果 |
|---|---|
| 已成功读取清单，版本不比当前新 | 已是最新版本 |
| 清单 HTTP 404 | 尚无可用更新包或发布资源缺失；不显示「已是最新版本」 |
| 断网、超时、GitHub 5xx | 连接更新服务器失败，可重试 |
| 403 / 429 | 服务暂时拒绝请求；遵守 `Retry-After`，不立即循环重试 |
| 清单无法解析、URL / 版本 / 大小无效 | 更新信息无效 |
| 安装包缺失、下载中断、实际大小不符 | 下载失败，清理本次未完成文件，可重试 |
| 签名、签名文件名或 macOS Team 不符 | 校验失败，禁止安装 |
| 开发版、Preview、非受支持安装形式 | 保持不自动替换，说明需从本仓库 Releases 手动安装 |

直接资源请求出现 404 时不能单凭该状态区分「没有 Release」与「Latest 缺少本平台资源」，因此提示同时覆盖两种情况。首版不为此额外调用 API。

## 下载与信任边界

1. 正式构建只从上述仓库的固定清单地址发起检查，只接受精确匹配的版本安装包地址；不回退上游网站。GitHub 不可达时保留当前版本。
2. 全程 HTTPS，重定向同样禁止 HTTP；允许 GitHub 资源下载所需的 HTTPS 重定向。最终内容仍必须通过嵌入公钥的 Minisign 验证，不能只依靠域名或 SHA-256。
3. 清单实际读取上限 64 KiB，连接超时 15 秒、检查总超时 20 秒；安装包总超时沿用 900 秒。包大小非零、最多 1 GiB；下载时限制实际写入量，不能只在下载完成后检查大小。只接收预期字节数，拒绝截断和超长响应。
4. 更新文件写入系统缓存目录，每次操作使用独立私有目录；Unix 权限设为 `0700`，创建失败即终止。清理只针对本次操作，避免现有 macOS / Windows 共用目录相互删除。
5. 首次为分叉发布前，在维护者控制的安全环境生成独立 Minisign 密钥，将公钥提交到 `packaging/update-key.pub`；私钥仅留在安全发布环境。客户端不下载或接受清单提供的新公钥。
6. macOS 继续验证 `codesign --verify --deep --strict`，且下载包的 Team 与当前应用一致。正式包需使用分叉维护者的 Developer ID 签名并公证；不通过放松验签来支持 ad-hoc 本地安装版。
7. `QUIET_MD_UPDATE_URL` 和 `QUIET_MD_UPDATE_PUBKEY` 保留作显式测试入口；所有放宽规则限于测试入口。测试公钥不进入正式发布包，Preview 和脚本运行默认不查询正式更新源。

## 安装与保存

下载和校验只准备新应用，不修改文档库。复用所有窗口的正常保存退出路径；保存失败或用户取消时，旧应用继续运行，更新保持已准备好，可以再次点击。不能绕过保存提示强制结束进程。

`install()` 经 `app::quit_after_saving` 进入保存完成回调：保存未通过时不启动 helper、不进入 `Installing`；全部窗口可退出后准备 helper，启动成功才执行退出。helper 启动失败时保留当前应用、已验证安装包和可重试状态，并显示错误。取消保存提示会清除原保存回调，不会在以后普通退出时安装。只有当前用户启动的下载成功完成后才会自动进入保存路径；后台检查、验签失败、重复或过期下载回调均不会请求安装。

| 平台 | 安装方式与失败处理 |
|---|---|
| macOS | 拒绝 App Translocation 和无写权限的安装路径；新包先复制到安装目录的同级临时目录并再次验证，再等当前进程退出后重命名替换。保留旧包，替换失败恢复；避免缓存与应用跨文件系统时直接 `mv` 的非原子复制。 |
| Windows | 等当前进程退出后运行现有 Inno Setup 更新模式并重新启动；保留 per-user 安装方式及文档保留行为。记录安装返回码，失败时保留可重试的包和诊断，不声称更新成功。 |
| Linux | 仅替换实际运行的 AppImage，沿用目标目录暂存、原子替换、参数保留与启动失败恢复；普通二进制及系统包仍由原安装方式升级。 |

系统缓存按规范化后的安装路径划分协调目录，每个运行实例持有独立文件锁，安装 helper 持有排他 gate 锁；同一路径其他 Quiet.md 实例尚未退出时不替换、不强杀，提示用户关闭后重试。helper 保存操作结果到应用本机缓存，下次启动可显示安装失败原因。这里的恢复只针对应用文件，不自动回滚应用已经打开过的文档库格式。

## 发布与首次切换

新增 `scripts/publish-github-release.py`，接收已完成平台签名的最终安装包；复用现有打包脚本，发布过程与 R2 上传解耦。脚本提供本地准备 / dry-run，使维护者在上传前能检查完整资产清单。

1. 运行 `scripts/check.sh`；确认两个 `Cargo.toml` 版本一致，tag、包名和二进制版本一致，工作区对应待发布源码。
2. 用独立更新公钥构建应用，生成四个平台的最终包。macOS 签名、公证、staple 必须完成后才计算大小、SHA-256 和 Minisign 签名；trusted comment 为精确包名。
3. 本地生成四份清单，验证签名对应仓库公钥、平台齐全、文件名 / URL / 版本匹配。拒绝 `-unsigned.zip` 等验证用包及重复资产。
4. 为 `iuxt/quiet-md` 创建 draft Release，上传全部安装包、校验和、签名和清单。读取回传资产并比对名称、大小和下载内容；全部验证成功后才发布为正式 Latest。上传过程中客户端仍看到前一个完整 Release。
5. 从公开版本资源地址再次下载验签，再检查四个 `latest/download/update-*.json` 地址是否对应本次版本；记录验证结果。发布完成后不替换该版本资产，有修复时递增版本重新发布。

维护者推送版本 tag 后，由 `.github/workflows/release.yml` 自动完成上述发布步骤，也可选择本地手动发布；应用本身不上传、创建 tag 或发布。发布脚本会要求本地和远端 tag 均指向当前提交，并运行完整检查。只有最终发布任务持有 `contents: write`；签名凭据仅在 Release 流程中使用，PR / 常规 CI 不持有发布私钥。

首次切换需要手动安装一个内置新更新源和独立公钥的版本。现有读取上游源、公钥的安装不能自行迁移到分叉的信任链；本地 ad-hoc 安装版也需要手动安装首个同 Team 的正式签名版本。发布前需准备 Minisign 私钥和 macOS Developer ID；不得把上游公钥可用当作分叉已具备发布条件。

## 密钥初始化与发布命令

在安全的 macOS 发布机器上、仓库根目录执行以下命令。`minisign -G` 交互设置私钥密码；不传 `-f`，避免覆盖已经存在的密钥：

```sh
brew install minisign gh
mkdir -p ~/.minisign
minisign -G -p ~/.minisign/quiet-md-iuxt.pub -s ~/.minisign/quiet-md-iuxt.key
cp ~/.minisign/quiet-md-iuxt.pub packaging/update-key.pub
```

只把公钥 `packaging/update-key.pub` 纳入源码，私钥保存在仓库外并另行安全备份。先完成公钥配置再构建四个平台的包，避免已发布客户端内置了错误密钥。用 `SIGNING_IDENTITY` / `NOTARY_PROFILE` 构建 macOS 正式包；ad-hoc 本地包不能作为正式更新包。

以下命令中的包路径需要替换为四个平台实际产物，输出使用新的、被 Git 忽略的 `dist/` 子目录：

```sh
python3 scripts/publish-github-release.py --dry-run <arm64.zip> <intel.zip> <windows-setup.exe> <linux.AppImage>
python3 scripts/publish-github-release.py --prepare dist/github/v<version> --key ~/.minisign/quiet-md-iuxt.key <arm64.zip> <intel.zip> <windows-setup.exe> <linux.AppImage>
```

`--dry-run` 不签名、不写文件、不联网；`--prepare` 只在本地生成 16 项资产（每个平台的包、SHA-256、Minisign 签名和清单），没有私钥时也可使用包旁已经存在且验签通过的 `.minisig`。已有输出目录一律拒绝覆盖。私钥从命令指定路径或 `MINISIGN_KEY` 读取，不复制进产物。

如选择本地手动发布，应先停用 Actions Release 工作流，避免推送 tag 后与自动发布争用同一版本。发布前登录 `gh`，手动确认版本、源码、平台签名及安装验证，通过正常 Git 流程提交公钥和源码、创建并推送 `v<version>` tag。然后将 `--prepare <新目录>` 改为 `--publish <新目录>`，加上 `--notes docs/releases/<version>.md`。脚本运行完整检查，创建 draft、上传并下载验证后才标记 Latest；失败的 draft 留供检查，不覆盖已有 Release。发布成功后再从公开地址下载复核；若此时失败，会返回失败并保留已发布版本供维护者处理，不擅自删除公开版本。

## GitHub Actions Release

[Release 工作流](../.github/workflows/release.yml) 在推送 `v*` tag 时自动构建，校验通过后发布为正式 Latest；普通分支 push 和 PR 不触发构建或发布。[CI 工作流](../.github/workflows/ci.yml) 仅供手动运行验证构建。Release 也保留在 `main` 上手动触发的入口，可选择只生成草稿。先把工作流提交并推送到 `main`，确保待发布 tag 对应的提交包含该工作流，再在仓库 Settings › Secrets and variables › Actions 配置下列 **Repository secrets**（不是普通 variables）：

| Secret | 值 |
|---|---|
| `MINISIGN_SECRET_KEY_BASE64` | 与仓库公钥匹配的完整 Minisign 私钥文件，编码成单行 Base64 |
| `MINISIGN_PASSWORD` | Minisign 私钥密码；加密私钥必须配置，无密码私钥可留空 |
| `MACOS_CERTIFICATE_P12_BASE64` | 含 Developer ID Application 证书及私钥的 `.p12`，单行 Base64 |
| `MACOS_CERTIFICATE_PASSWORD` | `.p12` 导出密码 |
| `MACOS_SIGNING_IDENTITY` | 完整的 `Developer ID Application: … (TEAMID)` 身份名称 |
| `APPLE_ID` | Apple Developer 账户邮箱 |
| `APPLE_APP_PASSWORD` | 用于 notarytool 的 Apple 专用密码 |
| `APPLE_TEAM_ID` | 签名证书所属 Apple Team ID |
| `WINDOWS_CERTIFICATE_PFX_BASE64` | 含有效代码签名证书及私钥的 `.pfx`，单行 Base64 |
| `WINDOWS_CERTIFICATE_PASSWORD` | `.pfx` 导出密码 |

Windows 时间戳服务默认 `http://time.certum.pl`，可以用仓库 **Variable** `WINDOWS_TIMESTAMP_URL` 改为证书颁发方的 RFC 3161 服务。本工作流的 Windows 签名支持可导入的 PFX；硬件令牌或云签名需要替换该签名步骤，不能拿证书公钥充当 PFX 私钥。

已初始化过 Minisign 密钥时不要重新生成。可在本机安装并登录 `gh` 后，直接把 Base64 通过管道写入 Secret，避免把私钥打印出来：

```sh
base64 -i ~/.minisign/quiet-md-iuxt.key | tr -d '\n' | gh secret set MINISIGN_SECRET_KEY_BASE64 --repo iuxt/quiet-md
gh secret set MINISIGN_PASSWORD --repo iuxt/quiet-md
```

第二条交互输入私钥密码。证书文件同样先编码成单行 Base64，再通过 Secrets 页面或 `gh secret set` 配置。工作流不会自动读取或上传本机密钥；私钥不进入 Git、日志或构建产物。macOS 在 runner 临时 keychain 中导入证书与公证凭据，Windows 临时导入 PFX，用完后清理。Minisign 私钥只用于预检及最终汇总签名，不交给平台构建步骤。

一次新版本的操作顺序：

1. 用 `python3 scripts/version.py bump-patch` 递增版本（也支持 `bump-minor` / `bump-major`），脚本同步 `VERSION`、两个 `Cargo.toml` 和 `Cargo.lock`。更新 `src/changelog.rs` 并创建 `docs/releases/<version>.md`，完成检查后按正常 Git 流程提交、推送到 `main`。**要测试更新，新版本必须高于已安装版本。**
2. 为当前 `main` 提交创建并推送匹配的 tag（示例版本号需替换）：

   ```sh
   git tag v0.2.0
   git push origin v0.2.0
   ```

3. 推送 tag 后，Actions › **Release** 自动构建、签名并发布正式 Latest，无需再点 **Run workflow**。在 Actions 中查看执行结果和 Release 链接。

需要重试尚未创建 Release 的版本时，可重新运行失败的工作流，也可打开 Actions › **Release** › **Run workflow**，分支选择 `main`，`tag` 填已有 tag。手动运行时 `publish` 默认关闭，生成并下载验签通过的 Release 草稿；需要正式发布时勾选 `publish`。也可用 CLI：

```sh
gh workflow run release.yml --repo iuxt/quiet-md --ref main -f tag=v0.2.0 -f publish=true
```

预检要求 tag 指向当前 `main` 提交、tag 严格为与两个 `Cargo.toml` 一致的 `vMAJOR.MINOR.PATCH`、发布说明存在、公钥有效、所需 Secrets 完整、Minisign 私钥实际签名能通过仓库公钥验签；失败会在昂贵的平台构建前结束。三个平台任务固定检出预检得到的同一提交，分别完成完整检查、打包与平台签名；macOS 在同一个 runner 顺序构建两种架构并公证。只有所有平台成功后才汇总生成 16 项更新资产，创建 draft、重新下载并逐项校验。tag 推送自动发布为 Latest；手动运行则按 `publish` 的值保留 draft 或发布为 Latest。只有最终汇总任务具备 Release 写权限。

所有发布使用同一个并发组，不取消已经开始的签名或上传；已有 Release 和资产拒绝覆盖。草稿验证后如要在同版本运行正式发布，先由维护者检查并删除已有草稿，再重新触发；工作流不会自动删除它，也不会替换已经公开的版本。构建产物保留 7 天，最终更新资产额外保留 14 天；实际 Release 资源长期保留。

当前本地 ad-hoc 客户端依然不能通过正式源自行更新，必须先手动安装首个同 Apple Team、内置分叉公钥和更新源的正式签名版本。Release 草稿不会被 `releases/latest/download` 发现。

## 实施范围与验收

| 文件 | 已实现 / 发布前操作 |
|---|---|
| `src/update.rs`、`src/update/http.rs`、`src/update/install.rs` | GitHub 清单；精确 URL / 版本验证；请求去重和 HTTP 分类；有界下载；独立暂存目录；平台安装协调和结果记录 |
| `src/app.rs` | 为正常保存退出路径接入更新完成回调，保存未完成时保留可重试状态 |
| `src/settings.rs` | 沿用现有控件；更新错误和不支持状态文案，将手动下载指向分叉 Releases |
| `packaging/update-key.pub` | 已配置分叉独立公钥；Actions 中的私钥必须与其匹配 |
| `scripts/publish-github-release.py` | 资产生成、签名校验、dry-run、CI 密码输入和 draft / Latest 发布 |
| `.github/workflows/release.yml` | 推送版本 tag 自动发布；保留手动草稿入口；四平台构建与检查、平台签名和公证、汇总发布 |
| `CONTRIBUTING.md`、README 各语言 | 已同步更新源、手动下载、密钥和首装说明；历史发布记录保留原意 |

测试应覆盖数值版本比较、旧版本 / 同版本不升级、错仓库 / tag / 平台、篡改签名或 trusted comment、恶意 URL、清单超限、包截断 / 超长、404 / 403 / 429 / 5xx / 超时，以及自动和手动检查并发。使用本地 HTTP 服务及测试密钥，验证「检查 → 下载 → 验签 → 待安装」；真实 GitHub 冒烟测试只读公开资源。

平台安装测试分别验证目标不可写、路径有空格及中文、跨文件系统暂存、多个实例、保存失败后重试、用户取消、helper 失败和替换失败恢复。macOS / Windows / Linux 需要各自运行安装测试，不能把 macOS 单测通过当作跨平台安装验证通过。

自动更新初次实现时的 macOS 验证：`scripts/check.sh` 通过（核心 190 项、应用 74 项、指南往返、5 项发布脚本测试及两种构建的 Clippy），`cargo build --release --locked` 通过。另用真实编译的二进制在临时目录运行 helper：确认等待持锁实例、实例退出后拒绝已过期的任务、原应用文件不变，且未创建或打开测试文档库。Windows / Linux 的真实安装、正式 Developer ID 签名包的完整更新，以及 GitHub draft 上传 / 公共资源复核仍需在首发流程中验证；本次未发布 Release。

一键更新补充验证：`scripts/check.sh` 通过；`cargo build --release --locked --features devtools` 及 `python3 tools/harness/update_flow.py` 通过。界面回归覆盖侧栏与关于页的更新按钮、下载进度、篡改签名拒绝，以及多窗口保存冲突、恢复副本和重新载入后的继续保存。成功下载后自动进入安装路径的状态测试还覆盖后台发现不安装、重复 / 过期完成回调不重启；未操作正式文档库或已安装应用。

所有测试文档库放临时目录，通过 `QUIET_MD_HOME` 指定；不读取或修改 `~/Documents/Quiet.md`，不操作其他应用或使用真实系统剪贴板。不为测试实际安装到用户的 Applications 目录。

## 资料

- [GitHub：最新 Release 资源链接](https://docs.github.com/en/repositories/releasing-projects-on-github/linking-to-releases)
- [iuxt/quiet-md Releases](https://github.com/iuxt/quiet-md/releases)
- 当前实现：[`src/update.rs`](../src/update.rs)、[`src/update/http.rs`](../src/update/http.rs)、[`src/update/install.rs`](../src/update/install.rs)、[`发布脚本`](../scripts/publish-github-release.py)

# Git 备份与同步

在「设置 → Git 备份与同步」启用。设置页只负责配置和初始化文档库，日常提交、拉取、推送与对比在侧栏「Git 变更」中进行，文章历史从文章「… → 版本历史」查看。默认文档库不会自动初始化 Git，也不会自动联网、提交或推送。

Git 由 `git2` 和编入应用的 libgit2 实现；图片和附件由应用实现 Git LFS 指针、对象缓存与 Batch / Basic HTTP 协议。运行时无需安装 Git、Git LFS 或命令行凭据助手，也不执行仓库钩子。

## 使用

1. 填写提交作者名称、邮箱和远端仓库地址，点击「启用 Git」。未配置远端时也可以提交本地版本。
2. HTTPS 远端认证填写用户名和访问令牌。令牌显示为掩码，仅在本次窗口会话中保留，不写进笔记库、JSON 元数据或 Git 配置。也可通过 `QUIET_MD_GIT_USERNAME` / `QUIET_MD_GIT_TOKEN` 提供。
3. 写完文章后，打开侧栏「Git 变更」，确认提交说明并点击「提交本地变更」或工具栏的提交按钮；提交说明默认填写当天日期。操作前保存所有窗口中的文章。有已暂存内容时只提交暂存区；没有暂存内容时提交整个文档库。
4. 「推送」先上传所提交历史引用的 LFS 对象，成功后再推送 Git 引用。「拉取」只允许快进；本地有未提交改动或历史已经分叉时停止，不自动丢弃或合并文章。
5. 在另一台电脑填写远端后使用「克隆到空文件夹…」，完成后按应用原有的切换文档库流程打开。克隆会读取远端默认分支，并下载文章历史引用的 LFS 对象。

远端支持 HTTPS、SSH（`ssh://git@host:2222/team/notes.git` 或 `git@host:team/notes.git`）和独立本地仓库路径（也支持 `file://`）；回环地址的 HTTP 可用于本地服务。当前不支持系统凭据助手、自动提交、定时拉取或分叉历史合并。可选 LFS 地址用于服务商单独部署的 LFS 端点。没有上传动作的 LFS 对象表示服务器已持有；提供 verify 动作时，上传后执行确认。

## SSH 私钥与 LFS CA

设置中的「SSH 私钥路径」可填写自定义私钥（支持 `~/`），路径保存在本机仓库的 `quiet.sshKey` 中；也可通过 `QUIET_MD_GIT_SSH_KEY` 提供。留空时使用 SSH agent。加密私钥的密码在「私钥密码」中填写，或通过 `QUIET_MD_GIT_SSH_PASSPHRASE` 提供；密码与访问令牌一样只在内存中保留。SSH URL 中的用户名优先，未指定时使用 `git`。

Git 与 LFS 的 SSH 连接都会核对 `~/.ssh/known_hosts`，不自动接受未知或已变更的主机密钥。使用非标准端口时，记录为 `[host]:2222`。可用 `QUIET_MD_GIT_KNOWN_HOSTS` 指定其他已核对的 known_hosts 文件。目前不读取 `~/.ssh/config` 的 Host 别名、IdentityFile 或代理配置，请在远端地址中填写实际主机与端口。

SSH 仓库的 LFS 默认通过 SSH 执行远端 `git-lfs-authenticate <仓库路径> upload|download`，再使用服务器返回的 HTTPS 地址与临时认证头进行 Batch 请求；SSH 端口不会当成 HTTPS 端口。服务器明确表示命令不存在时，回退到推导的 HTTPS LFS 地址，可填写访问令牌。设置显式 LFS 地址时，使用该地址与访问令牌认证。

LFS 的自签证书使用 PEM CA 文件，保持证书链与主机名校验。可以在设置中填写「LFS CA 证书」，也可直接沿用 `.git/config` 或全局 Git 配置中的 `http.sslCAInfo` / `http.<URL>.sslCAInfo`。设置页首次保存 CA 时按 LFS 主机（含 HTTPS 端口）写入标准 HTTP 配置。每个 Batch、上传、下载与 verify 地址分别匹配 CA，地址限定配置优先于全局默认，更具体的主机、路径优先。不同存储域名需分别配置自己的 CA。

例如下面的本机配置可使用 SSH 2222 端口同步 Git，并让 LFS 信任该服务的 CA：

```gitconfig
[remote "origin"]
    url = ssh://git@git.example.com:2222/you/notebook.git
    fetch = +refs/heads/*:refs/remotes/origin/*
[quiet]
    sshKey = /path/to/your/private_key
[http "https://git.example.com"]
    sslCAInfo = /path/to/ca.crt
```

私钥、密码和 CA 文件本身不会复制进文档库或提交；`.git/config` 只保存路径。原有的 LFS filter 配置保留，应用仍使用内置 LFS，不运行这些命令。

## 查看工作区变更

启用后，侧栏「Git 变更」打开独立面板。左侧列出新增、修改和删除的受管理文件，并显示文章标题与实际路径；点击文件在右侧对比上次提交与当前工作区，绿色为新增内容、红色为删除内容。编号使用单列，删除内容对应旧版编号，其余对应当前版本。XML 正文按内容块编号，文本和源码按行编号。

XML 文章默认保留标题、列表、引用、表格和行内样式进行正文对比，只有格式变化时也会展示修改前后的内容；可切换「查看源码」检查 XML；Markdown 和 JSON 直接按文本对比。图片与附件显示变更提示，不展示 LFS 指针。XML 正文最多对比前 1200 个顶层内容块，文本和源码最多对比前 1200 行，并标注截断。点击「刷新状态」前会保存所有窗口中的文章。提交、全部暂存、全部取消暂存、推送、拉取和刷新集中在文件列表顶部的图标工具栏，悬停可查看名称；Git 配置仍在设置中。

文件右侧的「+」将当前版本暂存（Git add），「−」取消暂存；文件行显示已暂存或部分已暂存。有暂存内容时，底部按钮只提交暂存区中的快照，之后继续编辑的内容留在工作区；没有暂存内容时沿用整库提交。新增或删除文章时，应同时暂存对应的 quiet.json，首次选择性提交还需包含格式标记等必需文件；应用会拒绝提交不完整的文档库。

撤回图标会先要求确认，再丢弃该文件未暂存的修改，恢复到暂存区中的版本（相当于 checkout -- <文件>），已暂存内容保留。新增且未暂存的文件会被删除，新增文章也会从清单移除。如清单变更涉及新增文章，需先丢弃新增文章，再恢复清单；如清单引用的正文已被删除，需先恢复正文。文档库必需文件在首次提交前不能丢弃。丢弃后会重新载入相关文档；此操作无法撤销。

## 文件与元数据

文档库始终使用 FORMAT 7：根目录 `quiet.json` 保存虚拟文件夹及文章元数据，正文固定在 `notes/<UUID>.xml|md`。无需启用 Git。文件夹仅由 `parentId`、文章归属仅由 `folderId` 表示，移动、重命名与移入回收站只更新 JSON，不改变正文路径或内容。完整清单示例见 [FORMAT.md](FORMAT.md)。

启用 Git 只初始化仓库并加入 `.gitattributes` / `.gitignore`。提交包含 `quiet.json`、格式标记、活动与回收站正文和 `.assets/` 资源。空文件夹也通过清单保存，克隆 / 拉取后无需建立对应目录。

```text
Quiet.md/
├── quiet.json                 version: 2，folders / notes / trash
├── notes/<uuid>.xml|md        固定正文路径
├── .assets/                   工作目录中的原图与附件
├── .gitattributes             .assets/** filter=lfs diff=lfs merge=lfs -text
├── .gitignore
├── .git/lfs/objects/          SHA-256 对象缓存
└── .quiet/
    ├── format.json            {"format": 7}
    ├── before-virtual-6-*/    旧布局迁移备份，仅本地
    ├── history/               自动快照，仅本地
    └── git-preview/           不可变历史资源预览，仅本地
```

回收站由 `quiet.json` 的 `trash` 数组记录，新删除的正文留在 `notes/`，移入和恢复只产生清单变更。不自动接管旧版 `.quiet/trash/` 中的正文。回收站文件夹可以打开浏览，文章可以只读打开并查看本地快照与 Git 提交历史；历史版本也可另存副本，编辑原文或恢复历史版本前须先恢复文章。回收站没有自动过期、清空或彻底删除功能；移入和恢复均可随 Git 提交、推送与拉取同步。

清单是层级和标题的唯一来源，应与正文一起备份。XML 中保留正文上次保存时的标题，但当前显示标题及历史提交标题均按对应清单读取。读取旧 FORMAT 5 的 Git 提交仍按当时的路径索引查找 ID。

本机文件、访问令牌、最近打开时间、窗口大小、缓存、自动快照与迁移备份不提交。Git 提交使用显式文件集合，不运行系统的 clean / smudge 过滤器；资源写为标准 LFS 指针进入 Git 对象库，工作目录中的原图不被替换。单个资源上限 256 MiB。

原有 `.gitignore`、`.gitattributes` 内容保留，补入应用所需规则。提交只包含应用管理的文件，不把文档库旁边的其他文件一起提交。启用前文档库必须不在另一个仓库内；不接管已经有提交、却没有 `quiet.json` 的无关仓库。

## 转换与恢复

- 旧库转换独立于 Git 是否启用；先备份，转换中断可重开恢复。迁移与删除恢复规则见 [FORMAT.md](FORMAT.md)。原 Git 分支和提交保留，旧文章历史仍按 UUID 读取。
- FORMAT 7 阻止旧版误读回收站状态；FORMAT 6 库需用户手动处理后再使用，不提供旧版回收站兼容分支。Markdown 链接、图片与引用定义随集中存储转换，代码中的同形文本保留。
- 手工放入 `notes/` 的文章在扫描时采用 UUID 文件名；目录不作为虚拟层级导入。遇到 UUID 冲突拒绝覆盖，不给固定文件名追加数字。
- 拉取先验证远端路径、元数据、正文格式和 LFS 的大小 / SHA-256，再修改工作目录；写入出错时恢复文章与 Git 索引，HEAD 最后更新。未跟踪同名文件不覆盖。符号链接、可执行文件和文档库以外路径不接受。
- 拉取到一半进程被强制终止时，可能留下 `.git/quiet-apply-*` 的备份；这不是多文件原子事务，需保留该目录以人工恢复。正常错误路径会自动回滚。转换备份同样应在核对完成前保留。

文章「… → 版本历史」同时列出原有自动快照与 Git 提交。Git 提交不能在抽屉里删除或重命名；查看、另存副本和恢复沿用现有入口。历史图片按提交中的 LFS 指针加载为独立缓存；恢复或另存时写入新的资源路径，不覆盖当前图片。恢复文章只形成工作区改动，之后手动提交，不回退分支 HEAD。

Git 只版本化 `.assets/` 中的图片 / 附件；外链、任意外部文件以及旧版本在启用 Git 之前的资源字节不在此保证内。旧自动快照继续保留，未自动转换成 Git 提交。已有笔记以往的图片变化无法从当前文件反向重建。

## 实现与验证

文档引擎的 `manifest.rs`、`virtual_store.rs`、`virtual_migrate.rs`、`git.rs`、`git/lfs.rs`、`git_store.rs` 不依赖 GPUI；应用层 `src/git.rs` 负责配置和后台任务，`history.rs` 复用原有历史抽屉。临时目录测试覆盖转换 / 重开 / 链接 / 回收站、提交历史、LFS 指针与校验、HTTP batch 上传下载确认、推送克隆拉取、分叉拒绝以及写入失败回滚，不使用真实文档库。`python3 tools/harness/git_ssh.py`（需要 Python `paramiko`）另用临时 SSH / 自签 TLS 服务器验证自定义端口、加密私钥、主机密钥拒绝、LFS CA、上传确认及推送 / 克隆 / 拉取。

协议参考：[git2 文档](https://docs.rs/git2/0.21.0/git2/)、[Git LFS Batch API](https://github.com/git-lfs/git-lfs/blob/main/docs/api/batch.md)、[Basic Transfer API](https://github.com/git-lfs/git-lfs/blob/main/docs/api/basic-transfers.md)。

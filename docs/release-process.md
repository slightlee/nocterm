# Nocterm 版本与发布规范

## 1. 目的与原则

本文档是 Nocterm 桌面客户端版本号、发布通道、打包准入和回滚流程的唯一规则来源，适用于内部 Alpha、Beta、发布候选版和正式版。发布自动化、CI 与辅助 Skill 只能执行本文档，不得维护另一套规则。

版本治理遵循以下原则：

- **可识别**：测试人员看到版本号即可判断稳定程度；
- **可追溯**：安装包、Git Tag、提交和验收记录一一对应；
- **不可变**：已分发版本不得覆盖、替换产物或移动 Tag；
- **可验证**：静态检查、构建、真实运行和功能验收分别记录；
- **可恢复**：升级不损坏本地数据和系统凭据，失败时有停止分发与修复路径。

发布不等于代码能够编译。每个可分发版本都必须拥有唯一版本号、可追溯提交、对应平台产物和真实验收记录。

## 2. 版本模型与发布通道

版本遵循 [Semantic Versioning 2.0.0](https://semver.org/)：

```text
<major>.<minor>.<patch>[-<channel>.<sequence>]
```

Nocterm 使用四个发布阶段：

| 阶段         | 示例            | 使用对象     | 进入条件                            | 允许变更                          |
| ------------ | --------------- | ------------ | ----------------------------------- | --------------------------------- |
| Alpha        | `0.1.0-alpha.1` | 开发人员     | 能安装启动，部分核心链路可不完整    | 功能、接口、数据结构和缺陷修复    |
| Beta         | `0.1.0-beta.1`  | 受控内测人员 | 声明的核心链路可用，无已知 P0       | 计划内完善、兼容调整和缺陷修复    |
| RC           | `0.1.0-rc.1`    | 发布验收人员 | 功能冻结，完整验收已执行，无 P0/P1  | 只允许发布阻断问题修复            |
| 正式版（GA） | `0.1.0`         | 所有目标用户 | RC 通过，双平台、升级和分发条件满足 | 发布后通过新 Patch/Minor 继续演进 |

同一基础版本的优先级顺序为：

```text
0.1.0-alpha.1 < 0.1.0-beta.1 < 0.1.0-rc.1 < 0.1.0
```

- 每次将安装包作为 Alpha、Beta 或 RC 向测试群体分发时递增对应通道序号，不预先规定各通道的版本数量；仅限指定验收人员使用、明确标注 Git Commit 与哈希且未宣称为已发布版本的临时构建属于发布前验证，不得代替最终产物；
- 切换通道时序号从 `1` 开始，例如 `beta.4 → rc.1`；
- RC 后若重新开放功能范围，应进入新的 Minor 开发周期，不得继续沿用原 RC；
- 未实际生成和分发安装包的日常提交不递增产品版本；
- SemVer 允许 `+build` 元数据，但 Nocterm 产品版本、Tag 和安装包名称暂不使用；构建信息单独记录 Git Commit 和 CI Run ID，避免平台工具排序不一致。

当前首个内测目标从 `0.1.0-beta.1` 开始。Alpha 规则为后续大功能早期验证保留，不要求为了流程完整而补发无实际价值的 Alpha 包。

## 3. 正式版本递增规则

正式版本使用 `MAJOR.MINOR.PATCH`：

- **Patch**：`0.1.0 → 0.1.1`，仅包含兼容的缺陷、安全或性能修复；
- **Minor**：`0.1.0 → 0.2.0`，增加向后兼容的用户能力，或形成新的明确发布范围；
- **Major**：`1.4.2 → 2.0.0`，包含无法兼容的产品、数据或配置行为变化。

`0.x` 表示产品仍在建立稳定能力边界，但不意味着可以随意损坏用户数据。任何不兼容的数据或配置变化都必须提供迁移方案并在发布说明中突出展示。达到长期兼容承诺、双平台核心能力稳定且升级链路成熟后，才进入 `1.0.0`。

正式版出现紧急安全或核心缺陷时，从受影响版本发布新的 Patch，不重新使用旧版本号。例如 `0.1.0` 修复后发布 `0.1.1`，不得替换已有的 `0.1.0` 安装包。

## 4. 唯一版本源

`package.json` 的 `version` 是 Nocterm 桌面产品版本的唯一来源。`apps/desktop/src-tauri/tauri.conf.json` 通过相对路径引用该文件，Tauri 构建产物与运行时健康信息都使用解析后的应用版本。

`apps/desktop/package.json` 是私有前端工作区包，不声明独立版本；`Cargo.toml` 的 `[workspace.package].version` 只描述不会单独发布的内部 Rust crate，不得用作产品版本。这样可以避免一次发布人工同步多个文件，也不会因为产品发版而制造无意义的 npm 或 Cargo 锁文件变更。

Git Tag 使用 `v` 前缀，例如 `v0.1.0-beta.1`。Tag 必须指向用于生成安装包的同一提交。产品版本由 Release Please 更新；锁文件若被工具更新，应与版本变更一并审查，禁止为制造一致性而手工修改生成内容。

Git Tag 是“已经发布”的唯一版本基线。版本递增必须高于目标分支可达的最新有效发布 Tag，Tag 推送还会与仓库已有的全部有效发布 Tag 比较，防止给旧提交补发低版本 Tag。仓库尚无发布 Tag 时允许建立首个预发布版本。因此，尚未发布阶段源码中的 `0.1.0` 开发占位值不会阻止首次 `0.1.0-beta.1`，但一旦存在 `v0.1.0-beta.1`，后续版本就必须严格高于它。CI 必须获取完整 Git 历史与 Tag，禁止在浅克隆且 Tag 不完整的环境中作发布判定。

Release Please 生成的发布提交使用：

```text
chore: prepare v0.1.0-beta.1
```

GitHub Squash merge 会在该标题后自动追加 Pull Request 编号，例如
`chore: prepare v0.1.0-beta.1 (#17)`。发布门禁只把这一标准后缀视为等价形式；其他附加文本、
零值编号、前导零编号或非标准括号格式仍会被拒绝，避免放宽发布提交与版本的一一对应约束。

仓库通过 `pnpm release:check` 校验版本格式、唯一版本源配置、版本递增、发布提交和 Tag。修改 `package.json` 的产品版本时必须使用上述发布提交；本地 `commit-msg` Hook 和远程 CI 会分别校验，不能依赖 Hook 作为唯一门禁。

## 5. 变更等级与发布决策

发布问题按用户影响分级：

| 等级 | 定义                                                                | 发布处理                                   |
| ---- | ------------------------------------------------------------------- | ------------------------------------------ |
| P0   | 安全泄露、凭据暴露、数据损坏、无法安装或启动、大面积核心连接不可用  | 阻断所有对外分发；已发布版本立即停止分发   |
| P1   | SSH/SFTP/凭据等声明的核心流程不可用且无合理绕过方式，或资源无法释放 | 阻断 Beta、RC 和正式版；Alpha 必须明确记录 |
| P2   | 局部功能异常但存在安全且合理的绕过方式，不导致安全或数据风险        | Beta 可带已知问题；RC/正式版需明确评审     |
| P3   | 轻微样式、文案或低频体验问题                                        | 不阻断发布，进入后续版本                   |

“有绕过方式”不得包含关闭主机密钥校验、降低权限、要求用户暴露凭据、删除数据库或在终端执行不明命令。问题等级由实际影响决定，不因发布时间临近而下调。

## 6. 发布准入

### 6.1 所有可分发版本

发布前必须满足：

1. 发布范围和已知限制明确，不夹带未审查改动；
2. `corepack pnpm check` 通过；
3. `corepack pnpm cargo:check` 通过；
4. macOS 与 Windows CI 均通过；
5. 安装包在对应目标系统原生构建，不以跨平台静态检查代替；
6. 安装、启动、退出和卸载流程完成冒烟检查；
7. 不包含密码、私钥、Token、测试服务器地址或未脱敏日志；
8. 发布说明列出版本范围、修复内容、已知限制和验收状态；
9. 产物名称、哈希、版本、平台、架构和 Git Commit 可相互核对。

### 6.2 Alpha

Alpha 可以包含未完成能力和已知 P1，但必须限定测试人员与测试范围。安全泄露、凭据暴露、数据损坏以及无法安装或启动仍会阻断 Alpha 分发。Alpha 不得被描述为可日常使用版本。

### 6.3 Beta

Beta 可以保留明确记录的非核心缺失和 P2/P3，但以下声明支持的路径必须在实际目标系统可用：

- 应用安装、启动和退出；
- 连接资料的创建、编辑、删除和持久化；
- 系统凭据保存与读取；
- SSH 密码、私钥和 Agent 认证；
- SSH 终端输入输出、重连和会话关闭；
- SFTP 目录浏览、上传、下载、取消和连接释放。

设置、服务器监控和 Agent 等未纳入本轮范围的能力可以不阻断，但必须在发布说明中明确。已知 P0 或上述核心范围内的 P1 会阻断 Beta 分发。

### 6.4 RC

进入 RC 前必须冻结功能范围，并完成 `docs/testing/` 中 macOS、Windows 对应流程。RC 不得包含已知 P0/P1；任何代码变更都必须重新生成安装包并至少执行受影响路径和基础冒烟测试。

如果修复不改变功能范围，发布新的 `rc.N`；如果重新增加功能或改变既定行为，退出当前 RC 周期并重新确定基础版本。

### 6.5 正式版（GA）

正式版还必须满足：

- `docs/roadmap/capability-matrix.md` 中本次承诺的能力达到双平台验收状态；
- 安装包签名、公证和分发方式已确定并验证；
- 从上一受支持版本升级时，本地数据库和系统凭据保持可用；
- 最终候选安装包与正式发布产物来自同一提交和同一构建配置；
- P2 已逐项评审并记录接受理由，发布说明中不存在未披露的已知限制。

## 7. 数据、配置与兼容性

- 数据库升级必须通过版本化 Migration 完成，不得要求用户删除数据库；
- 发布前至少验证全新安装和从上一受支持版本升级；正式版还应覆盖当前声明支持的最早版本；
- Migration 失败必须保持原数据可恢复，不得留下部分升级状态；
- 系统凭据引用升级后必须继续有效，认证方式变化不得静默删除已有凭据；
- 降级若不受支持，必须在发布说明中写明，不能用“重新安装”暗示用户数据一定安全；
- `com.nocterm.desktop` 等应用标识属于持久化和系统权限边界，修改时必须新增 ADR，并提供迁移与并存策略；
- Beta 可以不承诺跨大版本降级，但仍不得容忍无提示的数据损坏。

## 8. 平台产物、签名与供应链

目标产物为：

| 平台    | 内测产物                  | 正式分发要求          |
| ------- | ------------------------- | --------------------- |
| macOS   | `.app` 或 `.dmg`          | 签名、公证后的 `.dmg` |
| Windows | NSIS `.exe` 或 MSI 安装包 | 签名后的安装包        |

产物统一采用可识别名称：

```text
Nocterm_<version>_<platform>_<arch>.<ext>
```

例如：

```text
Nocterm_0.1.0-beta.1_macos_aarch64.dmg
Nocterm_0.1.0-beta.1_macos_x86_64.dmg
Nocterm_0.1.0-beta.1_windows_x86_64-setup.exe
```

Tauri 的 Windows NSIS 默认文件名使用 `x64`，不直接作为发布产物。在 Windows x64 本机生成最终 NSIS 产物时，在仓库根目录执行：

```powershell
corepack pnpm release:build:windows
```

脚本从根 `package.json` 读取产品版本，精确删除当前版本的旧 NSIS 源文件、规范副本和校验文件，再执行 Tauri 构建。只有构建成功且新源文件存在、非空时，才会在 `target/release/artifacts/` 生成符合上述命名规则的 NSIS 副本和同名 `.sha256` 校验文件。脚本当前只接受本机 Windows x64 产物；新增架构时必须先在对应平台验证 Tauri 原始命名和安装行为。

推送有效发布 Tag 后，`.github/workflows/release.yml` 从 Tag 解引用后的固定提交构建三个原生产物：在 `macos-15` ARM64 Runner 构建 macOS aarch64 DMG，在 `macos-15-intel` Runner 构建 macOS x86_64 DMG，并在 `windows-latest` 构建 Windows x86_64 NSIS。两个 macOS 构建都将最低运行版本固定为 macOS 14.0；Runner 系统版本只描述构建环境，不代表安装包只能在 macOS 15 运行。工作流创建或复用同 Tag 的 Draft Release，上传三个安装包及各自的 `.sha256`，最后核对六个资产齐全。自动化只准备 Draft，不公开发布；任一平台失败时 Draft 保持不可见，修复后可重跑并安全覆盖同名资产。

每次分发至少记录：

- Git Commit、Tag、CI Run ID 和构建时间；
- 操作系统、CPU 架构和安装包格式；
- 安装包 SHA-256；
- 依赖锁文件状态和构建工具版本；
- 签名、公证结果或未签名内测说明。

内部少量设备可以使用未签名安装包，但必须提前说明 Gatekeeper 或 SmartScreen 警告，不得把绕过系统安全检查作为正式安装方案。正式对外分发必须按 Tauri 官方要求完成 [macOS 签名与公证](https://v2.tauri.app/distribute/sign/macos/)和 [Windows 安装包签名](https://v2.tauri.app/distribute/sign/windows/)。

签名证书、私钥和公证凭据只允许存放在系统安全存储或 CI Secret 中，不得进入仓库、普通日志或安装包附件。

## 9. 验收记录

验收分别记录操作系统版本、CPU 架构、安装包名称、Git Commit、SSH 认证方式和测试结果。SSH、SFTP 与凭据按以下文档执行：

- `docs/testing/cross-platform-smoke.md`；
- `docs/testing/sftp-smoke.md`。

只有在目标系统真实运行后，才能更新能力矩阵中的平台验收状态。浏览器预览、单元测试、CI 或另一平台的结果不能代替目标安装包验收。

每个准备分发的候选版本必须创建或维护一条“发布验收”Issue，集中记录 Git Tag、完整 Commit SHA、CI Run、全部安装包 SHA-256、逐平台真实运行结果、已知问题等级和最终发布决定。该 Issue 在 Draft 产物生成前可以先建立，但只有最终产物与证据齐全后才能完成验收。

与候选版本相关、仍需合并后安装包验收的 Pull Request 只使用 `Refs #<issue>`，不得通过 `Closes` 或 `Fixes` 提前关闭发布验收 Issue。验收 Issue 的关闭不等同于公开发布 Draft；公开发布仍必须按第 10 节单独获得授权并执行。

## 10. Git 与最小发布流程

当前阶段从受保护的 `main` 分支发布，不创建长期 `develop` 或 `release/*` 分支。需要修复时通过正常分支和 Pull Request 回到 `main`，以减少分支漂移。

普通贡献者不创建发布分支、不修改产品版本，只通过正常分支和 Pull Request 合入 Conventional Commits。本轮发布范围稳定后，由仓库维护者手动运行 Release Please，创建或刷新唯一的发布 Pull Request，统一维护 `package.json`、`.release-please-manifest.json` 和 `CHANGELOG.md`。发布 Pull Request 必须通过与普通 PR 相同的 CI 和人工审查，不自动运行、不自动合并。

发布 Pull Request 是发布准备入口，不是业务修复分支。在它合并前发现的功能或平台问题，应从最新 `main` 创建普通 `fix/*` 分支；同一轮验收中高度相关的问题收敛到一个稳定化分支和 Draft Pull Request。修复合入 `main` 后，由维护者重新运行 Release Please 刷新原发布 Pull Request，不为同一候选范围重复创建手工发布 Pull Request。

发布 Pull Request 必须使用 Squash merge，使 `chore: prepare v<version>` 成为 `main` 上可直接标记的发布提交；禁止使用 Merge Commit 或 Rebase merge，确保普通变更与发布变更遵循同一合并策略，并让提交、Tag 与产物保持一一对应。

发布步骤：

1. 确认发布范围、问题等级、已知限制和目标通道；
2. 在 `main` 上完成本轮代码稳定化，运行前端、Rust 和双平台 CI，并完成能在发布提交前执行的真实环境预验证；临时构建必须记录 Git Commit 与哈希，不得对外宣称为可分发版本；
3. 由仓库维护者手动运行 Release Please，审查其提议的版本、变更日志、提交范围和 CI；未收敛的发布 Pull Request 保持打开，不以合并代替验收；
4. 确认候选范围已稳定后，使用 Squash merge 合并发布 Pull Request；合并本身不自动生成下一版本 Pull Request；
5. 等待发布提交对应的 `main` push CI 全部通过，确认发布提交仍为 `chore: prepare v<version>` 或 GitHub Squash 等价形式，且本次变更日志不包含未发布候选的重复内容或失效 Tag 比较链接；任何失败、取消或仍在运行的检查都会阻止创建 Tag；
6. 获得创建并推送 Tag 的单独授权后，从 `main` 手动运行 `Create Release Tag` 工作流并输入目标版本 Tag；该工作流重新验证远程 `main` 提交、对应成功 CI、版本、发布提交和 Tag 不存在后，创建并推送 annotated Tag；本地手工创建或推送发布 Tag 不再是受支持流程；
7. 受控工作流推送 Tag 后，自动触发 Tag CI 和 Release 工作流；后者从 Tag 对应提交生成双平台安装包、SHA-256 和 Draft Release，不使用本地旧构建代替；
8. 等待两个工作流全部通过，核对 Draft 中版本、Tag、提交和六个发布资产一致；自动化失败时修复工作流或代码后重跑，不移动或重建 Tag；
9. 创建或更新本候选版本的“发布验收”Issue，记录 Tag、Commit、CI Run、六个资产名称与 SHA-256；
10. 下载 Draft 中的最终安装包，在 macOS、Windows 执行对应冒烟与业务验收，并把真实结果和已知问题写入验收 Issue；
11. 验收通过并获得最终发布授权后，由仓库维护者公开 Draft；Alpha、Beta、RC 标记为 Pre-release，正式版不得标记为 Pre-release；
12. 记录发布决定，关闭验收 Issue，并只按真实证据更新能力矩阵。

### 10.1 候选版本失败与收敛

- 发布 Pull Request 合并前发现问题：不消耗新版本号。通过普通修复分支和 Pull Request 合入 `main`，由维护者重新运行 Release Please 刷新现有发布 Pull Request，再次执行受影响的检查与验收。
- 发布 Pull Request 合并后、Tag 创建前发现问题：当前候选版本视为已放弃，不创建虚假 Tag，不修改或重用其版本号。同一轮相关问题在一个稳定化分支中修复，通过一个 Pull Request 合入 `main`，然后由维护者手动运行 Release Please 提议下一个预发布序号。版本序号出现空档是正常审计结果，不得为连续性补发未验收版本。
- 使用 `skip-github-release` 时，Release Please 仍会把已合并但未打 Tag 的发布 Pull Request 视为待发布。放弃候选版本时，仓库维护者必须移除该 Pull Request 的 `autorelease: pending` 标签后重新运行 Release Please；该操作只修正自动化状态，不得添加 `autorelease: tagged` 或伪造 Tag。
- 继承未发布候选历史的下一个发布 Pull Request，必须在最终合并前人工审查 `CHANGELOG.md`：删除未发布候选的独立版本条目，去重提交记录，并让首个实际发布版本从 `bootstrap-sha` 开始比较；禁止保留指向不存在 Tag 的链接。
- 候选版本已创建 Tag、已宣称为可分发版本，或已向指定发布验收范围之外共享时，按已发布版本处理；修复必须使用更高版本号，不删除历史 Tag 或覆盖已分发产物。

Tag 创建后视为不可变。发现错误时删除尚未公开的本地产物并修复；一旦 Tag 或安装包已经共享，必须递增版本重新发布，不得强推、移动 Tag 或原位替换文件。

Git 提交、Tag、推送和正式发布都必须单独获得用户明确确认，不因执行本流程而获得自动授权。

## 11. 发布说明

每个可分发版本的发布说明至少包含：

```text
版本与通道
核心新增或修复
支持的平台与架构
已完成的验收
已知问题与绕过方式
升级或数据兼容说明
Git Commit 与安装包 SHA-256
```

发布说明面向测试人员和用户，保持简洁；详细实现过程留在提交、Issue 和技术文档中。安全问题在修复产物可用前不得披露可被直接利用的细节。

## 12. 停止分发与修复发布

发现 P0 或需要撤回的严重问题时：

1. 立即停止继续分发受影响安装包，并把发布状态标记为已撤回；
2. 保留 Tag、产物哈希和问题记录用于审计，不删除或移动已经公开的 Tag；
3. 确认受影响版本、平台、数据和凭据范围；
4. 在新版本中完成修复、回归测试和重新打包；
5. 发布新的 Patch 或预发布序号，并在说明中给出安全的升级路径；
6. 只有新产物验收通过后才能恢复分发。

若问题涉及凭据泄露或数据损坏，必须同时给出凭据轮换、数据恢复和用户通知方案，不能只发布二进制修复。

## 13. 自动化边界与仓库配置

当前阶段使用 Release Please 维护版本 Pull Request，但工作流只允许仓库维护者手动触发。普通功能或修复合入 `main` 不会自动创建下一版本 Pull Request；只有维护者确认发布范围稳定后才运行该工作流。Release Please 不创建 Tag 或安装包。`scripts/check-release-version.mjs` 负责版本格式、唯一版本源、递增、发布提交和 Tag 一致性检查，本地 Hook 与 CI 共同调用。

`.release-please-manifest.json` 记录 Release Please 已准备到的版本基线。合并发布 Pull Request 会把它更新为当前候选版本，但这不代表 Tag、安装包或 GitHub Release 已经创建；实际发布状态仍以不可变 Tag 和发布记录为准。该文件只能由发布 Pull Request 统一维护，不得手工提前推进。`release-please-config.json` 的 `bootstrap-sha` 只定义首次生成变更日志的起点，不代表一个版本，也不授予发布权限。

Tag 推送会同时运行版本门禁和发布工作流。若 CI 平台检出的本地引用丢失 annotated tag 对象，工作流必须从远端显式取回同名 Tag 后再校验。发布工作流先锁定 Tag 解引用后的提交，再让所有构建 Job 从该提交检出源码；只有发布准备 Job 和平台上传 Job 获得最小 `contents: write` 权限。它可以自动创建 Draft Release 和覆盖同一 Draft 的同名资产，但不得公开发布、覆盖已发布 Release 或移动 Tag。

发布工作流还会强制确认 Tag 提交可从远端 `main` 到达，并查询 GitHub Actions，要求该提交已经存在成功的 `main` push CI；因此即使维护者误把 Tag 提前推送，工作流也不会创建 Draft 或开始平台构建。手动事故恢复不能绕过门禁：用于启动恢复任务的最新 `main` 提交也必须先通过对应的 push CI。

新发布 Tag 只能通过维护者从 `main` 手动运行 `Create Release Tag` 工作流创建，例如：

```bash
gh workflow run create-release-tag.yml --ref main -f release_tag=v0.1.0-beta.3
```

该工作流使用 `RELEASE_PLEASE_TOKEN` 推送 Tag，以便正常触发 Tag CI 与 Draft Release；它在远程 Tag 出现前校验输入格式、目标是当前远程 `main`、同一提交已有成功的 `main` push CI、同名 Tag 不存在，并创建 annotated Tag 后执行完整版本门禁。不得在本地使用 `git tag` 和 `git push` 绕过该入口。工作流只准备 Tag，不公开 Release；触发前仍必须对本次 Tag 创建与远程推送取得单独授权。

修复工作流后重验已有不可变 Tag 时，维护者从 `main` 手动运行 CI 和 Release 工作流，并传入 `release_tag`；手动任务使用受保护 `main` 上的最新校验工具读取并校验 Tag 指向的版本和提交，但平台构建仍固定检出该 Tag 解引用后的 Commit SHA。例如：

```bash
gh workflow run ci.yml --ref main -f release_tag=v0.1.0-beta.2
gh workflow run release.yml --ref main -f release_tag=v0.1.0-beta.2
```

第一条命令只重新验证现有 Tag；第二条命令从该 Tag 自动构建并准备 Draft Release。两者都不创建、移动或覆盖 Tag，也不公开发布版本。

仓库维护者必须创建名为 `RELEASE_PLEASE_TOKEN` 的 GitHub Actions Secret。推荐使用限定到本仓库、设置有效期的 fine-grained PAT，最小授予 Contents、Pull requests 和 Issues 的读写权限；不得把 Token 写入仓库、日志或 PR。不能使用默认 `GITHUB_TOKEN` 替代，因为它创建的发布 Pull Request 不会触发本仓库的后续 CI 工作流。

Beta 阶段配置使用 `versioning=prerelease`、`prerelease-type=beta`。切换到 RC 或 GA 必须通过独立维护 PR 修改发布通道配置，并在提交正文加入明确的 `Release-As: <目标版本>`；例如 `0.1.0-rc.1` 或 `0.1.0`，不得依赖工具猜测跨通道版本。

自动化不得代替版本范围确认、问题定级、真实设备验收、签名凭据操作、Tag 创建和最终发布授权。确认推送 Tag 时必须同时说明它会自动创建或更新 Draft Release；公开 Draft 仍需单独授权。

# 跨平台基础冒烟测试

## 前置条件

- Node.js 22、pnpm 11、Rust 1.94；
- macOS 安装 Xcode Command Line Tools；
- Windows 安装 Microsoft C++ Build Tools 和 WebView2；
- 在项目根目录完成 `corepack pnpm install`。

## 静态门禁

```bash
corepack pnpm check
corepack pnpm cargo:check
```

记录操作系统、CPU 架构和全部命令结果。失败时不得更新为“平台已验收”。

## 浏览器预览

1. 执行 `corepack pnpm dev`；
2. 打开 `http://127.0.0.1:1420`；
3. 确认应用壳完整显示，状态为 `browser preview`；
4. 缩放至 760px 以下，确认左侧模块栏隐藏、指标改为纵向排列；
5. 控制台不应出现未捕获错误。

## Tauri 运行

1. 执行 `corepack pnpm tauri dev`；
2. 确认窗口可启动、缩放和关闭；
3. 顶部状态应变为 `desktop core ready`；
4. TARGET 显示当前 `macos` 或 `windows` 及正确 CPU 架构；
5. TERMINAL 显示 `macos-pty` 或 `windows-conpty`；
6. CREDENTIAL 显示对应系统安全存储；
7. 底部事件计数至少为 1，证明 Command/Event 示例链路均已接通；
8. 关闭窗口后确认进程正常退出。

## 阶段 1 SSH 业务验收

在目标系统准备一个可登录的测试 SSH 主机，并分别准备密码、OpenSSH 私钥和 SSH Agent 三种认证条件。每个平台都要记录实际系统版本、CPU 架构、SSH 客户端版本和测试连接标识。

1. 新建连接，验证连接资料、分组名称和排序在重启应用后仍然存在；
2. 使用密码认证，验证密码写入系统凭据库后可打开终端，SQLite 和普通日志中不得出现密码；
3. 使用私钥文件认证，验证文件选择、私钥读取、临时文件权限和连接关闭后的临时文件清理；
4. 使用 SSH Agent 认证，验证 Agent 转发和 `BatchMode` 行为；
5. 同时打开两个 SSH 连接，验证标签切换、输入输出隔离、窗口缩放和单个会话关闭；
6. 让远程会话主动退出，验证状态变为已关闭并可通过“重新连接”恢复；
7. 配置远程初始路径，验证进入远程 Shell 后工作目录正确；
8. 重命名分组，验证新名称持久化；
9. 删除分组，验证其中连接移动到“未分组”，不会删除连接资料。
10. 在仍有 SSH 会话时退出应用，确认 SSH 子进程和临时私钥目录均被回收。

只有上述业务项全部通过，能力矩阵中的对应平台状态才能更新为“已验收”。浏览器预览和静态门禁不能替代这组测试。

## 本地终端验收

1. 在没有任何会话时点击“打开本地终端”，确认出现“本地终端”标签并显示系统默认 Shell 提示符；
2. 输入 `echo NOCTERM_LOCAL_TERMINAL`，确认命令和输出只出现在当前标签；
3. 点击标签栏末尾“+”，确认新增“本地终端 2”，两个本地会话可独立输入和切换；
4. 缩放窗口，确认当前 PTY 尺寸随 xterm 更新，长行不会持续使用旧列宽；
5. 关闭一个标签，确认仅对应 Shell 子进程退出，其他本地或 SSH 会话继续工作；
6. 在 Shell 中执行 `exit`，确认标签状态变为已关闭，并可通过“重新连接”重新创建本地 Shell；
7. 退出应用，确认所有本地 Shell 子进程被回收；
8. macOS 记录实际默认 Shell；Windows 分别验证 PowerShell 或 `COMSPEC` 对应的 ConPTY 行为。

只有在目标系统完成上述交互后，才能把本地终端更新为对应平台“已验收”。

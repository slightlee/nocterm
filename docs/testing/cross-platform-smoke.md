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

当前显示的是平台能力契约，不代表 PTY 和凭据业务已实现。真实业务验收将在对应实施阶段补充。

# ADR-002：SSH/SFTP 先完成跨平台原型再锁定实现

- 状态：Proposed
- 日期：2026-08-14
- 决策人：Nocterm maintainers

## 背景

Nocterm 需要同时支持 macOS 与 Windows，并为 SSH、SFTP 提供稳定错误、取消、资源清理和主机密钥语义。系统 OpenSSH 能复用用户 SSH 配置，但 Windows 一致性和结构化 SFTP 能力仍需验证；原生 Rust 库则会引入 SSH Agent、跳板机、算法兼容和主机密钥处理成本。

## 候选方案

1. 系统 OpenSSH：macOS `/usr/bin/ssh`，Windows `ssh.exe`；
2. 原生 Rust SSH/SFTP 实现；
3. SSH 使用系统 OpenSSH，SFTP 使用独立原生 Adapter。

## 决策门槛

阶段 1 前制作最小原型，并在 macOS/Windows 验证：密码、私钥、SSH Agent、首次主机密钥、密钥变化、ProxyJump、非 22 端口、Unicode 路径、大文件、取消和断网清理。

只有原型证据齐全后把本文状态改为 Accepted。正式业务代码只能依赖 `SshBackend`/`SftpBackend`，不能依赖候选实现的具体类型。

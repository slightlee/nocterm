import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { Terminal } from '@xterm/xterm';
import { useEffect, useRef } from 'react';

import { isDesktopRuntime } from '../../../shared/lib/tauri-runtime';
import type { ConnectionProfile } from '../../connections';
import {
  PASSWORD_REQUIRED_CODE,
  closeSshTerminal,
  getTerminalErrorCode,
  onSshTerminalExit,
  onSshTerminalOutput,
  openSshTerminal,
  resizeSshTerminal,
  writeSshTerminal,
} from '../api/ssh-terminal-client';
import {
  EMPTY_PASSWORD_PROMPT,
  type PasswordPromptState,
  reducePasswordPrompt,
} from '../model/password-prompt';
import { attachRightClickPaste, createCopyKeyHandler } from '../model/terminal-clipboard';
import {
  applyTerminalAppearance,
  observeTerminalAppearance,
  readTerminalFontSize,
  readTerminalTheme,
  TERMINAL_FONT_FAMILY,
} from '../model/terminal-appearance';
import { useTerminalStore } from '../model/terminal-store';
import '@xterm/xterm/css/xterm.css';
import styles from './SshTerminal.module.css';

/** xterm 仅负责当前 SSH 会话的渲染、输入和尺寸同步。 */
interface SshTerminalProps {
  connection: ConnectionProfile;
  active?: boolean;
}

export function SshTerminal({ connection, active = true }: SshTerminalProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const fitTerminalRef = useRef<(() => void) | null>(null);
  const setSessionStatus = useTerminalStore((state) => state.setSessionStatus);
  const markSessionConnected = useTerminalStore((state) => state.markSessionConnected);

  useEffect(() => {
    if (active) fitTerminalRef.current?.();
  }, [active]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container || !isDesktopRuntime()) return;

    const terminal = new Terminal({
      cursorBlink: true,
      fontFamily: TERMINAL_FONT_FAMILY,
      fontSize: readTerminalFontSize(),
      scrollback: 10_000,
      theme: readTerminalTheme(container),
    });
    const fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.loadAddon(new WebLinksAddon());
    terminal.open(container);
    fitAddon.fit();
    fitTerminalRef.current = () => {
      fitAddon.fit();
      if (terminalId) void resizeSshTerminal(terminalId, terminal.cols, terminal.rows);
    };

    let disposed = false;
    let terminalId: string | null = null;
    let outputUnlisten: (() => void) | null = null;
    let exitUnlisten: (() => void) | null = null;
    /**
     * `terminalId` 在 open 的 IPC 返回后才知道，但监听必须提前注册——否则登录横幅
     * 与 shell 首屏提示符会在响应到达前丢掉。此前的兜底是"没有 terminalId 就按
     * connectionId 匹配"，可同一连接开两个标签时两边互相串写对方的开场输出。
     *
     * 改为**始终按 terminalId 严格过滤**：未知期间先把同连接的事件按各自的
     * terminalId 缓存，拿到自己的 id 后只回放属于自己的部分，其余丢弃。
     */
    let pendingOutput: { terminalId: string; data: string }[] = [];
    let pendingExit: { terminalId: string; reason: string } | null = null;
    // 未知期只有建连阶段那一瞬，正常不会有大量输出；设上限纯粹是防止后端异常时无界堆积。
    const PENDING_OUTPUT_LIMIT = 512;
    // 口令提示激活期间还没有远端会话，按键必须留在本地而不是转发给后端。
    let prompt: {
      label: string;
      state: PasswordPromptState;
      resolve: (value: string | null) => void;
    } | null = null;

    /**
     * 口令输入完全不回显（与 `ssh`、PuTTY 一致，连长度都不暴露），
     * 因此按键期间无需重绘：只在首次提示和被其他输出打断后重画整行。
     * `\r` 回到行首、`\x1b[2K` 清行，保证不会在残留内容上叠字。
     */
    const drawPrompt = () => {
      if (!prompt) return;
      terminal.write(`\r\x1b[2K\x1b[33m${prompt.label}\x1b[0m`);
    };

    const settlePrompt = (value: string | null) => {
      const pending = prompt;
      prompt = null;
      if (!pending) return;
      // 提示行末没有换行，这里补一个使后续输出从新行开始。
      terminal.write('\r\n');
      pending.resolve(value);
    };

    /** 在终端里就地收集登录口令：不回显，回车提交，Ctrl+C 放弃并返回 null。 */
    const askPassword = (label: string) =>
      new Promise<string | null>((resolve) => {
        terminal.write(
          '\r\n\x1b[90m该连接未保存密码。请输入密码后回车（与 ssh 一致，输入不回显，可用 Ctrl+V 或右键粘贴，Ctrl+C 取消）\x1b[0m\r\n'
        );
        prompt = { label, state: EMPTY_PASSWORD_PROMPT, resolve };
        drawPrompt();
        terminal.focus();
      });

    const input = terminal.onData((data) => {
      if (prompt) {
        const next = reducePasswordPrompt(prompt.state, data);
        prompt.state = next;
        if (next.status === 'submitted') settlePrompt(next.value);
        else if (next.status === 'cancelled') settlePrompt(null);
        // 其余按键只进缓冲，屏幕上不留任何痕迹。
        return;
      }
      if (terminalId) void writeSshTerminal(terminalId, data);
    });

    // 粘贴交回 xterm：口令提示符与已连会话都从 `onData` 收到内容，无需各自处理剪贴板。
    const detachPaste = attachRightClickPaste(container, {
      paste: (text) => terminal.paste(text),
      onUnavailable: (message) => {
        terminal.write(`\r\n\x1b[31m[${message}]\x1b[0m\r\n`);
        drawPrompt();
      },
    });
    /**
     * 复制必须自己接管：Ctrl+C 会先被 xterm 当成中断信号，浏览器不会派发 `copy` 事件。
     *
     * 口令提示期间对外宣称"没有选区"，于是 Ctrl+C 照旧走 `onData` 去取消连接，而
     * Ctrl+Shift+C 仍被这里吞掉（复制空选区即无操作），不会误发 `\x03` 打断提示。
     */
    terminal.attachCustomKeyEventHandler(
      createCopyKeyHandler({
        hasSelection: () => !prompt && terminal.hasSelection(),
        selection: () => terminal.getSelection(),
        clearSelection: () => terminal.clearSelection(),
        onUnavailable: (message) => {
          terminal.write(`\r\n\x1b[31m[${message}]\x1b[0m\r\n`);
          drawPrompt();
        },
      })
    );
    const observer = new ResizeObserver(() => {
      // 从设置页返回时隐藏容器会恢复尺寸，此处重新读取色板，不能只做 fit。
      applyTerminalAppearance(terminal, container);
      fitAddon.fit();
      if (terminalId) void resizeSshTerminal(terminalId, terminal.cols, terminal.rows);
    });
    observer.observe(container);

    const stopObservingAppearance = observeTerminalAppearance(terminal, container, () => {
      fitAddon.fit();
      if (terminalId) void resizeSshTerminal(terminalId, terminal.cols, terminal.rows);
    });

    /**
     * 先按后端已有的凭据尝试建连，只有后端明确返回 SSH_PASSWORD_REQUIRED 时才提示输入。
     * 由后端而不是前端的 credentialStatus 判定是否需要口令：连接列表里的凭据状态可能已过期，
     * 而后端每次都会重新查询会话缓存与系统凭据库。
     */
    const openWithInteractivePassword = async () => {
      try {
        return await openSshTerminal(connection.id, terminal.cols, terminal.rows);
      } catch (error) {
        if (getTerminalErrorCode(error) !== PASSWORD_REQUIRED_CODE) throw error;
      }
      const secret = await askPassword(`${connection.username}@${connection.host} 的密码：`);
      if (disposed) return null;
      if (!secret) {
        // 空口令在本项目范围内不受支持，与主动放弃同等处理，给出明确收尾而不是抛错。
        terminal.write('\x1b[90m[已取消连接]\x1b[0m\r\n');
        setSessionStatus(connection.id, 'closed');
        return null;
      }
      return openSshTerminal(connection.id, terminal.cols, terminal.rows, secret);
    };

    /** 会话收尾的统一落点：失败与正常结束的状态迁移只在这里做一次。 */
    const applyExit = (reason: string) => {
      if (reason === 'failed') {
        const message = 'SSH 连接已失败，请检查认证、主机指纹和网络';
        terminal.write(`\r\n\x1b[31m[${message}]\x1b[0m\r\n`);
        setSessionStatus(connection.id, 'error', message);
        return;
      }
      terminal.write('\r\n\x1b[90m[SSH 会话已结束]\x1b[0m\r\n');
      setSessionStatus(connection.id, 'closed');
    };

    /** 拿到自己的 terminalId 后回放缓存事件，只取属于本会话的那部分。 */
    const flushPending = (ownId: string) => {
      const buffered = pendingOutput;
      pendingOutput = [];
      for (const item of buffered) {
        if (item.terminalId === ownId) terminal.write(item.data);
      }
      const exit = pendingExit;
      pendingExit = null;
      if (exit && exit.terminalId === ownId) applyExit(exit.reason);
    };

    void (async () => {
      const outputListener = await onSshTerminalOutput((payload) => {
        if (terminalId) {
          if (payload.terminalId === terminalId) terminal.write(payload.data);
          return;
        }
        if (payload.connectionId !== connection.id) return;
        if (pendingOutput.length >= PENDING_OUTPUT_LIMIT) return;
        pendingOutput.push({ terminalId: payload.terminalId, data: payload.data });
      });
      if (disposed) {
        outputListener();
        return;
      }
      outputUnlisten = outputListener;

      const exitListener = await onSshTerminalExit((payload) => {
        if (terminalId) {
          if (payload.terminalId === terminalId) applyExit(payload.reason);
          return;
        }
        if (payload.connectionId !== connection.id) return;
        // 会话可能在 open 响应到达前就退出，缓存下来等确认归属后再迁移状态。
        pendingExit = { terminalId: payload.terminalId, reason: payload.reason };
      });
      if (disposed) {
        outputListener();
        exitListener();
        return;
      }
      exitUnlisten = exitListener;

      const opened = await openWithInteractivePassword();
      if (!opened) return;
      if (disposed) {
        await closeSshTerminal(opened.terminalId);
        return;
      }
      terminalId = opened.terminalId;
      markSessionConnected(connection.id);
      // 回放放在 markSessionConnected 之后：若会话已经失败，失败状态应当覆盖已连接。
      flushPending(opened.terminalId);
      terminal.focus();
    })().catch((error: unknown) => {
      const message = getErrorMessage(error);
      terminal.write(`\r\n\x1b[31m连接失败：${message}\x1b[0m\r\n`);
      setSessionStatus(connection.id, 'error', message);
    });

    return () => {
      disposed = true;
      // 卸载时先兑现挂起的口令提示，否则等待它的 Promise 永远不会落地。
      settlePrompt(null);
      detachPaste();
      observer.disconnect();
      stopObservingAppearance();
      input.dispose();
      outputUnlisten?.();
      exitUnlisten?.();
      if (terminalId) void closeSshTerminal(terminalId);
      fitTerminalRef.current = null;
      terminal.dispose();
    };
  }, [connection, markSessionConnected, setSessionStatus]);

  return <div className={`${styles.terminal} nocterm-terminal`} ref={containerRef} />;
}

function getErrorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'object' && error !== null && 'message' in error) {
    return String(error.message);
  }
  return String(error);
}

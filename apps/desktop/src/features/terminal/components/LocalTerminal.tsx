import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { Terminal } from '@xterm/xterm';
import { useEffect, useRef } from 'react';

import { isDesktopRuntime } from '../../../shared/lib/tauri-runtime';
import {
  closeLocalTerminal,
  onLocalTerminalExit,
  onLocalTerminalOutput,
  openLocalTerminal,
  resizeLocalTerminal,
  writeLocalTerminal,
} from '../api/local-terminal-client';
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

interface LocalTerminalProps {
  sessionId: string;
  active?: boolean;
}

/** 本地终端复用 SSH 终端的 xterm 呈现，但通过独立 IPC 驱动默认系统 Shell。 */
export function LocalTerminal({ sessionId, active = true }: LocalTerminalProps) {
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
      if (terminalId) void resizeLocalTerminal(terminalId, terminal.cols, terminal.rows);
    };

    let disposed = false;
    let terminalId: string | null = null;
    let outputUnlisten: (() => void) | null = null;
    let exitUnlisten: (() => void) | null = null;
    const input = terminal.onData((data) => {
      if (terminalId) void writeLocalTerminal(terminalId, data);
    });
    // 与 SSH 终端保持一致的右键粘贴：应用壳禁用了 WebView 原生右键菜单。
    const detachPaste = attachRightClickPaste(container, {
      paste: (text) => terminal.paste(text),
      onUnavailable: (message) => terminal.write(`\r\n\x1b[31m[${message}]\x1b[0m\r\n`),
    });
    // 复制同样要自己接管：Ctrl+C 会被 xterm 当成中断信号，浏览器不会派发 copy 事件。
    terminal.attachCustomKeyEventHandler(
      createCopyKeyHandler({
        hasSelection: () => terminal.hasSelection(),
        selection: () => terminal.getSelection(),
        clearSelection: () => terminal.clearSelection(),
        onUnavailable: (message) => terminal.write(`\r\n\x1b[31m[${message}]\x1b[0m\r\n`),
      })
    );
    const observer = new ResizeObserver(() => {
      // 从设置页返回时隐藏容器会恢复尺寸，此处重新读取色板，不能只做 fit。
      applyTerminalAppearance(terminal, container);
      fitAddon.fit();
      if (terminalId) void resizeLocalTerminal(terminalId, terminal.cols, terminal.rows);
    });
    observer.observe(container);
    const stopObservingAppearance = observeTerminalAppearance(terminal, container, () => {
      fitAddon.fit();
      if (terminalId) void resizeLocalTerminal(terminalId, terminal.cols, terminal.rows);
    });

    void (async () => {
      const outputListener = await onLocalTerminalOutput((payload) => {
        if (payload.sessionId === sessionId) terminal.write(payload.data);
      });
      if (disposed) {
        outputListener();
        return;
      }
      outputUnlisten = outputListener;

      const exitListener = await onLocalTerminalExit((payload) => {
        if (payload.sessionId !== sessionId) return;
        terminal.write('\r\n\x1b[90m[本地终端已退出]\x1b[0m\r\n');
        setSessionStatus(sessionId, 'closed');
      });
      if (disposed) {
        outputListener();
        exitListener();
        return;
      }
      exitUnlisten = exitListener;

      const opened = await openLocalTerminal(sessionId, terminal.cols, terminal.rows);
      if (disposed) {
        await closeLocalTerminal(opened.terminalId);
        return;
      }
      terminalId = opened.terminalId;
      markSessionConnected(sessionId);
      terminal.focus();
    })().catch((error: unknown) => {
      const message = getErrorMessage(error);
      terminal.write(`\r\n\x1b[31m打开本地终端失败：${message}\x1b[0m\r\n`);
      setSessionStatus(sessionId, 'error', message);
    });

    return () => {
      disposed = true;
      detachPaste();
      observer.disconnect();
      stopObservingAppearance();
      input.dispose();
      outputUnlisten?.();
      exitUnlisten?.();
      if (terminalId) void closeLocalTerminal(terminalId);
      fitTerminalRef.current = null;
      terminal.dispose();
    };
  }, [markSessionConnected, sessionId, setSessionStatus]);

  return <div className={`${styles.terminal} nocterm-terminal`} ref={containerRef} />;
}

function getErrorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'object' && error !== null && 'message' in error) {
    return String(error.message);
  }
  return String(error);
}

import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { Terminal } from '@xterm/xterm';
import type { ITheme } from '@xterm/xterm';
import { useEffect, useRef } from 'react';

import { isDesktopRuntime } from '../../../shared/lib/tauri-runtime';
import type { ConnectionProfile } from '../../connections';
import {
  closeSshTerminal,
  onSshTerminalExit,
  onSshTerminalOutput,
  openSshTerminal,
  resizeSshTerminal,
  writeSshTerminal,
} from '../api/ssh-terminal-client';
import { useTerminalStore } from '../model/terminal-store';
import '@xterm/xterm/css/xterm.css';
import styles from './SshTerminal.module.css';

/** xterm 仅负责当前 SSH 会话的渲染、输入和尺寸同步。 */
interface SshTerminalProps {
  connection: ConnectionProfile;
  active?: boolean;
}

function readCssVariable(styles: CSSStyleDeclaration, name: string, fallback: string): string {
  return styles.getPropertyValue(name).trim() || fallback;
}

function resolveTerminalTheme(container: HTMLElement): ITheme {
  const variables = getComputedStyle(container);
  const foreground = readCssVariable(variables, '--term-text', '#c9d1d9');
  const background = readCssVariable(variables, '--term', '#0d1117');
  const cursor = readCssVariable(variables, '--term-blue', '#58a6ff');
  const selectionBackground = readCssVariable(variables, '--term-selection', '#264f78');
  const red = readCssVariable(variables, '--term-red', '#f87171');
  const green = readCssVariable(variables, '--term-green', '#4ade80');
  const yellow = readCssVariable(variables, '--term-yellow', '#fbbf24');

  return {
    background,
    foreground,
    cursor,
    selectionBackground,
    selectionForeground: foreground,
    black: foreground,
    red,
    green,
    yellow,
    blue: cursor,
    brightRed: red,
    brightGreen: green,
    brightYellow: yellow,
    brightBlue: cursor,
  };
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
      fontFamily: "'SF Mono', 'JetBrains Mono', Menlo, Consolas, monospace",
      fontSize: 13,
      scrollback: 10_000,
      theme: resolveTerminalTheme(container),
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

    const input = terminal.onData((data) => {
      if (terminalId) void writeSshTerminal(terminalId, data);
    });
    const observer = new ResizeObserver(() => {
      fitAddon.fit();
      if (terminalId) void resizeSshTerminal(terminalId, terminal.cols, terminal.rows);
    });
    observer.observe(container);

    const themeObserver = new MutationObserver(() => {
      terminal.options.theme = resolveTerminalTheme(container);
    });
    themeObserver.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['data-theme'],
    });

    void (async () => {
      const outputListener = await onSshTerminalOutput((payload) => {
        const currentSession = terminalId
          ? payload.terminalId === terminalId
          : payload.connectionId === connection.id;
        if (currentSession) terminal.write(payload.data);
      });
      if (disposed) {
        outputListener();
        return;
      }
      outputUnlisten = outputListener;

      const exitListener = await onSshTerminalExit((payload) => {
        const currentSession = terminalId
          ? payload.terminalId === terminalId
          : payload.connectionId === connection.id;
        if (!currentSession) return;
        if (payload.reason === 'failed' || payload.reason === 'timed_out') {
          const message =
            payload.reason === 'timed_out'
              ? 'SSH 连接超时，请检查网络和主机地址'
              : 'SSH 连接已失败，请检查认证、主机指纹和网络';
          terminal.write(`\r\n\x1b[31m[${message}]\x1b[0m\r\n`);
          setSessionStatus(connection.id, 'error', message);
          return;
        }
        terminal.write('\r\n\x1b[90m[SSH 会话已结束]\x1b[0m\r\n');
        setSessionStatus(connection.id, 'closed');
      });
      if (disposed) {
        outputListener();
        exitListener();
        return;
      }
      exitUnlisten = exitListener;

      const opened = await openSshTerminal(connection.id, terminal.cols, terminal.rows);
      if (disposed) {
        await closeSshTerminal(opened.terminalId);
        return;
      }
      terminalId = opened.terminalId;
      markSessionConnected(connection.id);
      terminal.focus();
    })().catch((error: unknown) => {
      const message = getErrorMessage(error);
      terminal.write(`\r\n\x1b[31m连接失败：${message}\x1b[0m\r\n`);
      setSessionStatus(connection.id, 'error', message);
    });

    return () => {
      disposed = true;
      observer.disconnect();
      themeObserver.disconnect();
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

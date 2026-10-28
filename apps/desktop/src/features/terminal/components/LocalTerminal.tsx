import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { Terminal } from '@xterm/xterm';
import { useEffect, useRef, useState } from 'react';

import { isDesktopRuntime } from '../../../shared/lib/tauri-runtime';
import {
  closeLocalTerminal,
  onLocalTerminalExit,
  onLocalTerminalOutput,
  openLocalTerminal,
  resizeLocalTerminal,
  writeLocalTerminal,
} from '../api/local-terminal-client';
import {
  readTerminalClipboardText,
  writeTerminalClipboardText,
} from '../api/terminal-clipboard-client';
import { createKeywordHighlighter } from '../model/keyword-highlight';
import { readTerminalHighlightConfig } from '../../../shared/lib/highlight-config';
import { attachTerminalClipboard, createCopyKeyHandler } from '../model/terminal-clipboard';
import {
  attachTerminalSelectionAction,
  getTerminalSelectionAnchor,
  type TerminalSelectionActionState,
} from '../model/terminal-selection';
import {
  applyTerminalAppearance,
  observeTerminalAppearance,
  readTerminalFontSize,
  readTerminalTheme,
  TERMINAL_FONT_FAMILY,
} from '../model/terminal-appearance';
import { useTerminalStore } from '../model/terminal-store';
import { useTerminalNotice } from '../model/use-terminal-notice';
import { TerminalSelectionAction } from './TerminalSelectionAction';
import '@xterm/xterm/css/xterm.css';
import styles from './SshTerminal.module.css';

interface LocalTerminalProps {
  sessionId: string;
  active?: boolean;
  onAddSelectionToAi: (sessionId: string, selection: string) => void;
}

/** 本地终端复用 SSH 终端的 xterm 呈现，但通过独立 IPC 驱动默认系统 Shell。 */
export function LocalTerminal({
  sessionId,
  active = true,
  onAddSelectionToAi,
}: LocalTerminalProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const fitTerminalRef = useRef<(() => void) | null>(null);
  const activeRef = useRef(active);
  const [selectionAction, setSelectionAction] = useState<TerminalSelectionActionState | null>(null);
  const setSessionStatus = useTerminalStore((state) => state.setSessionStatus);
  const markSessionConnected = useTerminalStore((state) => state.markSessionConnected);
  const { notice, showNotice } = useTerminalNotice();

  useEffect(() => {
    activeRef.current = active;
    if (active) {
      fitTerminalRef.current?.();
    } else {
      // eslint-disable-next-line react-hooks/set-state-in-effect -- 隐藏的终端不能保留可点击的旧选区入口。
      setSelectionAction(null);
    }
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
    // 每个会话独立的高亮状态机：跨写入块跟踪 SGR，避免污染既有颜色。
    // 尾部持有的 token 由延迟冲刷兜底送达：交互回显（每键一块）不会被扣住。
    // 角色配色来自设置（预设 + 用户覆盖），设置变化经 observer 热更新。
    const highlighter = createKeywordHighlighter({
      onDeferred: (text) => {
        if (!disposed) {
          setSelectionAction(null);
          terminal.write(text);
        }
      },
      highlight: readTerminalHighlightConfig(),
    });
    terminal.open(container);
    const detachSelectionAction = attachTerminalSelectionAction(
      container,
      {
        active: () => activeRef.current,
        selection: () => terminal.getSelection(),
        selectionAnchor: () => {
          const position = terminal.getSelectionPosition();
          if (!position) return null;
          return getTerminalSelectionAnchor(
            container,
            terminal.element,
            position,
            terminal.buffer.active.viewportY,
            terminal.cols,
            terminal.rows
          );
        },
        onContentChange: (listener) => terminal.onWriteParsed(listener),
        onSelectionChange: (listener) => terminal.onSelectionChange(listener),
        onViewportChange: (listener) => terminal.onScroll(listener),
      },
      setSelectionAction
    );
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
      setSelectionAction(null);
      if (terminalId) void writeLocalTerminal(terminalId, data);
    });
    // 有选区时右键复制；无选区时才粘贴，避免误把旧剪贴板内容写入 Shell。
    const detachClipboard = attachTerminalClipboard(
      container,
      {
        hasSelection: () => terminal.hasSelection(),
        selection: () => terminal.getSelection(),
        clearSelection: () => terminal.clearSelection(),
        paste: (text) => terminal.paste(text),
        onUnavailable: (message) => {
          if (!disposed) showNotice(message);
        },
      },
      readTerminalClipboardText,
      writeTerminalClipboardText
    );
    // 复制同样要自己接管：Ctrl+C 会被 xterm 当成中断信号，浏览器不会派发 copy 事件。
    terminal.attachCustomKeyEventHandler(
      createCopyKeyHandler(
        {
          hasSelection: () => terminal.hasSelection(),
          selection: () => terminal.getSelection(),
          clearSelection: () => terminal.clearSelection(),
          onUnavailable: (message) => {
            if (!disposed) showNotice(message);
          },
        },
        writeTerminalClipboardText
      )
    );
    const observer = new ResizeObserver(() => {
      // 从设置页返回时隐藏容器会恢复尺寸，此处重新读取色板，不能只做 fit。
      setSelectionAction(null);
      applyTerminalAppearance(terminal, container);
      fitAddon.fit();
      if (terminalId) void resizeLocalTerminal(terminalId, terminal.cols, terminal.rows);
    });
    observer.observe(container);
    const stopObservingAppearance = observeTerminalAppearance(
      terminal,
      container,
      () => {
        fitAddon.fit();
        if (terminalId) void resizeLocalTerminal(terminalId, terminal.cols, terminal.rows);
      },
      (config) => highlighter.setHighlight(config.preset, config.overrides)
    );

    void (async () => {
      const outputListener = await onLocalTerminalOutput((payload) => {
        if (payload.sessionId !== sessionId) return;
        setSelectionAction(null);
        terminal.write(highlighter.transform(payload.data));
      });
      if (disposed) {
        outputListener();
        return;
      }
      outputUnlisten = outputListener;

      const exitListener = await onLocalTerminalExit((payload) => {
        if (payload.sessionId !== sessionId) return;
        setSelectionAction(null);
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
      // 先冲刷高亮器仍持有的尾部文本，再标记 disposed 停止后续写入。
      highlighter.dispose();
      disposed = true;
      detachClipboard();
      detachSelectionAction();
      observer.disconnect();
      stopObservingAppearance();
      input.dispose();
      outputUnlisten?.();
      exitUnlisten?.();
      if (terminalId) void closeLocalTerminal(terminalId);
      fitTerminalRef.current = null;
      terminal.dispose();
    };
  }, [markSessionConnected, sessionId, setSessionStatus, showNotice]);

  return (
    <div className={styles.terminalWrapper}>
      <div className={`${styles.terminal} nocterm-terminal`} ref={containerRef} />
      <TerminalSelectionAction
        action={selectionAction}
        onAdd={(selection) => {
          setSelectionAction(null);
          onAddSelectionToAi(sessionId, selection);
        }}
      />
      {notice ? (
        <div aria-live="polite" className={styles.terminalNotice} role="status">
          {notice}
        </div>
      ) : null}
    </div>
  );
}

function getErrorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'object' && error !== null && 'message' in error) {
    return String(error.message);
  }
  return String(error);
}

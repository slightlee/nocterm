import { createPortal } from 'react-dom';
import { useLayoutEffect, useRef, useState, type KeyboardEvent } from 'react';

import type { SftpSession } from '../../sftp';
import type { TerminalSession, TerminalSessionId, TerminalStatus } from '../../terminal';
import {
  resolveSessionPickerPosition,
  type SessionPickerPosition,
} from '../model/server-session-picker-layout';
import type { ConnectionProfile } from '../types/connection-types';
import styles from './ServerSessionPicker.module.css';

type RemoteTerminalSession = Extract<TerminalSession, { kind: 'remote' }>;

interface ServerSessionPickerProps {
  connection: ConnectionProfile;
  sessions: RemoteTerminalSession[];
  activeSessionId: TerminalSessionId | null;
  terminalActive: boolean;
  statuses: Record<string, TerminalStatus>;
  errors: Record<string, string | null>;
  fileSession: SftpSession | null;
  filesActive: boolean;
  onActivate: (sessionId: TerminalSessionId) => void;
  onActivateFiles: () => void;
}

const POPOVER_WIDTH = 264;

function getStatusLabel(status: TerminalStatus) {
  switch (status) {
    case 'connected':
      return '已连接';
    case 'connecting':
      return '连接中';
    case 'error':
      return '连接失败';
    case 'closed':
      return '已断开';
    default:
      return '未连接';
  }
}

/** 服务器侧栏显示会话数量；Portal 使会话浮层脱离列表裁切区域。 */
export function ServerSessionPicker({
  connection,
  sessions,
  activeSessionId,
  terminalActive,
  statuses,
  errors,
  fileSession,
  filesActive,
  onActivate,
  onActivateFiles,
}: ServerSessionPickerProps) {
  const triggerRef = useRef<HTMLButtonElement>(null);
  const popoverRef = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState<SessionPickerPosition | null>(null);
  const sessionCount = sessions.length + (fileSession ? 1 : 0);

  useLayoutEffect(() => {
    if (!open) return;

    const updatePosition = () => {
      const trigger = triggerRef.current;
      const popover = popoverRef.current;
      if (!trigger || !popover) return;

      const anchor = trigger.getBoundingClientRect();
      const panel = popover.getBoundingClientRect();
      setPosition(
        resolveSessionPickerPosition(
          { left: anchor.left, right: anchor.right, top: anchor.top },
          { width: Math.max(POPOVER_WIDTH, panel.width), height: panel.height },
          { width: window.innerWidth, height: window.innerHeight }
        )
      );
    };

    updatePosition();
    const focusFrame = window.requestAnimationFrame(() => {
      const currentSession = popoverRef.current?.querySelector<HTMLButtonElement>(
        '[data-session-entry][aria-current="true"]'
      );
      const firstSession =
        popoverRef.current?.querySelector<HTMLButtonElement>('[data-session-entry]');
      const focusTarget = currentSession ?? firstSession;
      focusTarget?.focus({ preventScroll: true });
      focusTarget?.scrollIntoView({ block: 'nearest' });
    });

    // 滚动与缩放改变锚点坐标；ResizeObserver 也覆盖窗口尺寸变化后的内容重排。
    const observer = new ResizeObserver(updatePosition);
    if (triggerRef.current) observer.observe(triggerRef.current);
    if (popoverRef.current) observer.observe(popoverRef.current);
    document.addEventListener('scroll', updatePosition, true);
    window.addEventListener('resize', updatePosition);

    // 非模态浮层允许点击外部继续工作，但 Escape 会将焦点送回触发按钮。
    const closeOnPointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (
        target instanceof Node &&
        (popoverRef.current?.contains(target) || triggerRef.current?.contains(target))
      ) {
        return;
      }
      setOpen(false);
    };
    const closeOnKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      setOpen(false);
      setPosition(null);
      triggerRef.current?.focus({ preventScroll: true });
      event.stopPropagation();
    };
    document.addEventListener('pointerdown', closeOnPointerDown);
    document.addEventListener('keydown', closeOnKeyDown);

    return () => {
      window.cancelAnimationFrame(focusFrame);
      observer.disconnect();
      document.removeEventListener('scroll', updatePosition, true);
      window.removeEventListener('resize', updatePosition);
      document.removeEventListener('pointerdown', closeOnPointerDown);
      document.removeEventListener('keydown', closeOnKeyDown);
    };
  }, [open]);

  const moveSessionFocus = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
    const entries = Array.from(
      popoverRef.current?.querySelectorAll<HTMLButtonElement>('[data-session-entry]') ?? []
    );
    if (entries.length === 0) return;

    event.preventDefault();
    const currentIndex = entries.indexOf(document.activeElement as HTMLButtonElement);
    const direction = event.key === 'ArrowDown' ? 1 : -1;
    const nextIndex =
      currentIndex < 0 ? 0 : (currentIndex + direction + entries.length) % entries.length;
    entries[nextIndex]?.focus();
  };

  const popupStyle = position
    ? {
        left: position.left,
        top: position.top,
        width: position.width,
        maxHeight: position.maxHeight,
      }
    : { left: 0, top: 0, visibility: 'hidden' as const };

  return (
    <>
      <button
        ref={triggerRef}
        aria-expanded={open}
        aria-haspopup="dialog"
        aria-label={`查看 ${connection.name} 的 ${sessionCount} 个会话`}
        className={styles.trigger}
        onClick={(event) => {
          event.stopPropagation();
          setOpen((current) => !current);
          if (open) setPosition(null);
        }}
        onPointerDown={(event) => event.stopPropagation()}
        title={`查看 ${sessionCount} 个已打开会话 · ${connection.name}`}
        type="button"
      >
        <span>{sessionCount}</span>
        <svg viewBox="0 0 12 12" aria-hidden="true">
          <path d="m3 4.5 3 3 3-3" />
        </svg>
      </button>
      {open
        ? createPortal(
            <div
              ref={popoverRef}
              aria-label={`${connection.name} 的会话列表`}
              className={styles.popover}
              role="dialog"
              style={popupStyle}
              onKeyDown={moveSessionFocus}
            >
              <header className={styles.popoverHeader}>
                <strong>已打开的会话</strong>
                <span className={styles.sessionCount}>{sessionCount} 个</span>
                <button
                  aria-label="关闭会话列表"
                  className={styles.dismiss}
                  onClick={() => {
                    setOpen(false);
                    setPosition(null);
                    triggerRef.current?.focus({ preventScroll: true });
                  }}
                  type="button"
                >
                  ×
                </button>
              </header>
              <div aria-label="会话列表" className={styles.sessionList} role="group">
                {sessions.map((session) => {
                  const status = statuses[session.id] ?? 'connecting';
                  const current = terminalActive && session.id === activeSessionId;
                  const error = errors[session.id];
                  return (
                    <button
                      key={session.id}
                      aria-current={current ? 'true' : 'false'}
                      aria-label={`切换到 ${connection.name} 的终端 ${session.terminalNumber}，${getStatusLabel(status)}${current ? '，正在查看' : ''}`}
                      className={`${styles.sessionEntry} ${current ? styles.current : ''}`}
                      data-session-entry
                      onClick={() => {
                        setOpen(false);
                        setPosition(null);
                        onActivate(session.id);
                      }}
                      title={
                        error ??
                        `${connection.name} · 终端 ${session.terminalNumber} · ${getStatusLabel(status)}`
                      }
                      type="button"
                    >
                      <span className={styles.sessionKind} aria-hidden="true">
                        {session.terminalNumber}
                      </span>
                      <span className={styles.sessionName}>{connection.name}</span>
                      <span className={styles.sessionStatus} title={error ?? undefined}>
                        <span className={styles.sessionStatusDot} data-status={status} />
                        {getStatusLabel(status)}
                      </span>
                      {current ? <span className={styles.currentLabel}>当前</span> : null}
                    </button>
                  );
                })}
                {fileSession ? (
                  <button
                    aria-current={filesActive ? 'true' : 'false'}
                    aria-label={`切换到 ${connection.name} 的文件管理，${getStatusLabel(fileSession.status)}${filesActive ? '，正在查看' : ''}`}
                    className={`${styles.sessionEntry} ${filesActive ? styles.current : ''}`}
                    data-session-entry
                    onClick={() => {
                      setOpen(false);
                      setPosition(null);
                      onActivateFiles();
                    }}
                    title={
                      fileSession.lastError ??
                      `文件管理 · ${getStatusLabel(fileSession.status)} · ${fileSession.currentPath}`
                    }
                    type="button"
                  >
                    <span className={styles.sessionKind} aria-hidden="true">
                      <svg viewBox="0 0 24 24">
                        <path d="M3 7.5A2.5 2.5 0 0 1 5.5 5H10l2 2.5h6.5A2.5 2.5 0 0 1 21 10v7a3 3 0 0 1-3 3H5a2 2 0 0 1-2-2.5z" />
                      </svg>
                    </span>
                    <span className={styles.sessionName}>{connection.name}</span>
                    <span
                      className={styles.sessionStatus}
                      title={fileSession.lastError ?? undefined}
                    >
                      <span className={styles.sessionStatusDot} data-status={fileSession.status} />
                      {getStatusLabel(fileSession.status)}
                    </span>
                    {filesActive ? <span className={styles.currentLabel}>当前</span> : null}
                  </button>
                ) : null}
              </div>
            </div>,
            document.body
          )
        : null}
    </>
  );
}

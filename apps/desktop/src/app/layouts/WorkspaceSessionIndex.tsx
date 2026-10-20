import { createPortal } from 'react-dom';
import { useId, useLayoutEffect, useRef, useState, type KeyboardEvent } from 'react';

import {
  groupWorkspaceSessionIndexItems,
  type WorkspaceSessionIndexItem,
  type WorkspaceSessionTabKey,
} from '../model/workspace-session-tabs';
import styles from './TerminalWorkspace.module.css';
import { WorkspaceSessionTypeIcon, type WorkspaceSessionKind } from './WorkspaceSessionTypeIcon';

export type WorkspaceSessionIndexStatus = 'idle' | 'connecting' | 'connected' | 'closed' | 'error';

export interface WorkspaceSessionIndexEntry extends WorkspaceSessionIndexItem {
  kind: WorkspaceSessionKind;
  status: WorkspaceSessionIndexStatus;
  current: boolean;
  title: string;
}

interface WorkspaceSessionIndexProps {
  entries: WorkspaceSessionIndexEntry[];
  onActivate: (key: WorkspaceSessionTabKey) => void;
}

interface PopoverPosition {
  left: number;
  top: number;
  width: number;
  maxHeight: number;
}

const POPOVER_WIDTH = 300;
const VIEWPORT_MARGIN = 8;
const ANCHOR_GAP = 5;

function getStatusLabel(status: WorkspaceSessionIndexStatus) {
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

/**
 * 全部会话使用 Portal 脱离标签栏裁切，并在窗口缩放后重新约束位置。
 *
 * 弹层保持非模态：用户可以直接点击其他工作区内容继续操作；Escape
 * 会关闭弹层并把焦点送回触发按钮。搜索只过滤展示结果，不改变会话顺序。
 */
export function WorkspaceSessionIndex({ entries, onActivate }: WorkspaceSessionIndexProps) {
  const dialogId = useId();
  const triggerRef = useRef<HTMLButtonElement>(null);
  const popoverRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [position, setPosition] = useState<PopoverPosition | null>(null);
  const groups = groupWorkspaceSessionIndexItems(entries, query);

  useLayoutEffect(() => {
    if (!open) return;

    const updatePosition = () => {
      const trigger = triggerRef.current;
      const popover = popoverRef.current;
      if (!trigger || !popover) return;

      const anchor = trigger.getBoundingClientRect();
      const panel = popover.getBoundingClientRect();
      // 宽高都以当前 WebView 为上限，避免窗口缩小时 Portal 溢出软件边界。
      const width = Math.max(0, Math.min(POPOVER_WIDTH, window.innerWidth - VIEWPORT_MARGIN * 2));
      const maxHeight = Math.max(0, window.innerHeight - VIEWPORT_MARGIN * 2);
      const panelHeight = Math.min(panel.height, maxHeight);
      setPosition({
        left: Math.max(
          VIEWPORT_MARGIN,
          Math.min(anchor.right - width, window.innerWidth - width - VIEWPORT_MARGIN)
        ),
        top: Math.max(
          VIEWPORT_MARGIN,
          Math.min(anchor.bottom + ANCHOR_GAP, window.innerHeight - panelHeight - VIEWPORT_MARGIN)
        ),
        width,
        maxHeight,
      });
    };

    updatePosition();
    // 等 Portal 完成首帧布局后再聚焦，避免浏览器为聚焦输入框额外滚动页面。
    const focusFrame = window.requestAnimationFrame(() => {
      searchRef.current?.focus({ preventScroll: true });
    });
    // 内容过滤、窗口缩放和上层滚动都会改变锚点或面板尺寸。
    const observer = new ResizeObserver(updatePosition);
    if (triggerRef.current) observer.observe(triggerRef.current);
    if (popoverRef.current) observer.observe(popoverRef.current);
    document.addEventListener('scroll', updatePosition, true);
    window.addEventListener('resize', updatePosition);

    // Pointer down 先于 click 触发，可以在外部目标执行动作前移除浮层。
    const closeOnPointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (
        target instanceof Node &&
        (triggerRef.current?.contains(target) || popoverRef.current?.contains(target))
      ) {
        return;
      }
      setOpen(false);
      setPosition(null);
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

  const handleKeyboardNavigation = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
    const buttons = Array.from(
      popoverRef.current?.querySelectorAll<HTMLButtonElement>('[data-session-index-entry]') ?? []
    );
    if (buttons.length === 0) return;

    event.preventDefault();
    // 搜索框内按方向键时从首尾进入结果；列表内则循环移动焦点。
    const currentIndex = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const direction = event.key === 'ArrowDown' ? 1 : -1;
    const nextIndex =
      currentIndex < 0
        ? direction > 0
          ? 0
          : buttons.length - 1
        : (currentIndex + direction + buttons.length) % buttons.length;
    buttons[nextIndex]?.focus({ preventScroll: true });
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
        aria-controls={open ? dialogId : undefined}
        aria-expanded={open}
        aria-haspopup="dialog"
        aria-label={`查看全部 ${entries.length} 个会话`}
        className={styles.allSessionsButton}
        onClick={() => {
          setOpen((current) => {
            if (current) setPosition(null);
            else setQuery('');
            return !current;
          });
        }}
        title={`全部会话 · ${entries.length}`}
        type="button"
      >
        <svg aria-hidden="true" className={styles.allSessionsIcon} viewBox="0 0 20 20">
          <rect x="3.5" y="7" width="11" height="8.5" rx="2" />
          <path d="M6.5 5h7a3 3 0 0 1 3 3v5" />
        </svg>
        <span>{entries.length}</span>
        <svg aria-hidden="true" className={styles.allSessionsChevron} viewBox="0 0 16 16">
          <path d="m4 6 4 4 4-4" />
        </svg>
      </button>

      {open
        ? createPortal(
            <div
              ref={popoverRef}
              aria-label="全部会话"
              className={styles.sessionIndexPopover}
              id={dialogId}
              onKeyDown={handleKeyboardNavigation}
              role="dialog"
              style={popupStyle}
            >
              <header className={styles.sessionIndexHeader}>
                <strong>
                  全部会话 <span>{entries.length}</span>
                </strong>
                <input
                  ref={searchRef}
                  aria-label="筛选已打开会话"
                  autoComplete="off"
                  onChange={(event) => setQuery(event.target.value)}
                  placeholder="搜索服务器或会话"
                  type="search"
                  value={query}
                />
              </header>

              <div aria-label="已打开会话列表" className={styles.sessionIndexList} role="group">
                {groups.length > 0 ? (
                  groups.map((group) => (
                    <section className={styles.sessionIndexGroup} key={group.id}>
                      <div className={styles.sessionIndexGroupName}>
                        <span title={group.label}>{group.label}</span>
                        <span>{group.entries.length}</span>
                      </div>
                      {group.entries.map((entry) => {
                        const statusLabel = getStatusLabel(entry.status);
                        return (
                          <button
                            aria-current={entry.current ? 'true' : undefined}
                            aria-label={`切换到 ${entry.groupLabel} ${entry.label}，${statusLabel}${entry.current ? '，正在查看' : ''}`}
                            className={`${styles.sessionIndexEntry} ${entry.current ? styles.sessionIndexCurrent : ''}`}
                            data-session-index-entry
                            key={entry.key}
                            onClick={() => {
                              setOpen(false);
                              setPosition(null);
                              onActivate(entry.key);
                            }}
                            title={entry.title}
                            type="button"
                          >
                            <span className={styles.sessionIndexDot} data-status={entry.status} />
                            <WorkspaceSessionTypeIcon
                              className={styles.sessionIndexTypeIcon}
                              kind={entry.kind}
                            />
                            <span className={styles.sessionIndexTask}>{entry.label}</span>
                            <span className={styles.sessionIndexState}>
                              {entry.current ? '当前 · ' : ''}
                              {statusLabel}
                            </span>
                          </button>
                        );
                      })}
                    </section>
                  ))
                ) : (
                  <div className={styles.sessionIndexEmpty}>没有匹配的已打开会话</div>
                )}
              </div>
            </div>,
            document.body
          )
        : null}
    </>
  );
}

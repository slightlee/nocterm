import { createPortal } from 'react-dom';
import {
  Fragment,
  useEffect,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent,
  type ReactNode,
} from 'react';

import type { ContextMenuItem } from '../../../shared/components/ContextMenu';
import type { GroupDropPosition } from '../model/connection-list-model';
import styles from './ConnectionGroup.module.css';

const GROUP_MENU_WIDTH = 180;
const GROUP_MENU_MARGIN = 8;

interface GroupMenuPosition {
  x: number;
  y: number;
}

/** 菜单使用视口坐标，确保靠近窗口边缘时仍完整显示。 */
function resolveMenuPosition(x: number, y: number, height: number): GroupMenuPosition {
  return {
    x: Math.max(
      GROUP_MENU_MARGIN,
      Math.min(x, window.innerWidth - GROUP_MENU_WIDTH - GROUP_MENU_MARGIN)
    ),
    y: Math.max(GROUP_MENU_MARGIN, Math.min(y, window.innerHeight - height - GROUP_MENU_MARGIN)),
  };
}

interface ConnectionGroupProps {
  id: string;
  name: string;
  count: number;
  expanded: boolean;
  canManage: boolean;
  dropActive?: boolean;
  groupDragging?: boolean;
  groupDropPosition?: GroupDropPosition;
  editing?: boolean;
  onGroupPointerDown?: (event: PointerEvent<HTMLDivElement>) => void;
  onToggle: () => void;
  onCreate: () => void;
  onRename: (name: string) => void;
  onRenameCancel: () => void;
  onStartRename: () => void;
  onDelete: () => void;
  children: ReactNode;
}

/** 分组头部与旧客户端保持一致，连接操作仍由列表负责。 */
export function ConnectionGroup({
  id,
  name,
  count,
  expanded,
  canManage,
  dropActive = false,
  groupDragging = false,
  groupDropPosition,
  editing = false,
  onGroupPointerDown,
  onToggle,
  onCreate,
  onRename,
  onRenameCancel,
  onStartRename,
  onDelete,
  children,
}: ConnectionGroupProps) {
  const [menuPosition, setMenuPosition] = useState<GroupMenuPosition | null>(null);
  const [draftName, setDraftName] = useState(name);
  const inputRef = useRef<HTMLInputElement>(null);
  const menuButtonRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menuPosition) return;
    requestAnimationFrame(() =>
      menuRef.current?.querySelector('button')?.focus({ preventScroll: true })
    );
    const closeOnMouseDown = (event: MouseEvent) => {
      const target = event.target as Node;
      if (menuRef.current?.contains(target) || menuButtonRef.current?.contains(target)) return;
      setMenuPosition(null);
    };
    const close = () => setMenuPosition(null);
    const closeOnKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') close();
    };
    document.addEventListener('mousedown', closeOnMouseDown);
    document.addEventListener('scroll', close, true);
    document.addEventListener('keydown', closeOnKey);
    window.addEventListener('resize', close);
    return () => {
      document.removeEventListener('mousedown', closeOnMouseDown);
      document.removeEventListener('scroll', close, true);
      document.removeEventListener('keydown', closeOnKey);
      window.removeEventListener('resize', close);
    };
  }, [menuPosition]);
  useEffect(() => {
    if (editing) {
      inputRef.current?.focus();
      inputRef.current?.select();
    }
  }, [editing]);

  const commitRename = () => {
    const nextName = draftName.trim();
    if (!nextName || nextName === name) {
      setDraftName(name);
      onRenameCancel();
      return;
    }
    onRename(nextName);
  };

  const menuItems: ContextMenuItem[] = canManage
    ? [
        {
          label: '新建到此分组',
          onSelect: onCreate,
          icon: (
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="M12 5v14M5 12h14" />
            </svg>
          ),
        },
        {
          label: '重命名分组',
          onSelect: () => {
            setDraftName(name);
            onStartRename();
          },
          icon: (
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7" />
              <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z" />
            </svg>
          ),
        },
        {
          label: '删除分组',
          danger: true,
          onSelect: onDelete,
          icon: (
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <polyline points="3 6 5 6 21 6" />
              <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
            </svg>
          ),
        },
      ]
    : [];
  const menuHeight = menuItems.length * 34 + 17;

  const openMenuFromButton = (event: ReactMouseEvent<HTMLButtonElement>) => {
    event.stopPropagation();
    if (menuPosition) {
      setMenuPosition(null);
      return;
    }
    const rect = event.currentTarget.getBoundingClientRect();
    setMenuPosition(
      resolveMenuPosition(rect.right - GROUP_MENU_WIDTH, rect.bottom + 4, menuHeight)
    );
  };

  const header = (
    <div
      className={`${styles.groupHeader} ${groupDragging ? styles.groupDragging : ''} ${
        groupDropPosition
          ? styles[`groupDrop${groupDropPosition === 'before' ? 'Before' : 'After'}`]
          : ''
      }`}
      data-group-drag-handle
      onClick={editing ? undefined : onToggle}
      onPointerDown={canManage && !editing ? onGroupPointerDown : undefined}
      onContextMenu={(event) => {
        if (!canManage) return;
        event.preventDefault();
        event.stopPropagation();
        setMenuPosition(resolveMenuPosition(event.clientX, event.clientY, menuHeight));
      }}
    >
      <span className={`${styles.arrow} ${expanded ? styles.expanded : ''}`} aria-hidden="true">
        <svg viewBox="0 0 24 24">
          <path d="m6 9 6 6 6-6" />
        </svg>
      </span>
      {editing ? (
        <input
          ref={inputRef}
          className={styles.groupNameInput}
          value={draftName}
          onChange={(event) => setDraftName(event.target.value)}
          onClick={(event) => event.stopPropagation()}
          onBlur={commitRename}
          onKeyDown={(event) => {
            if (event.key === 'Enter') {
              event.preventDefault();
              commitRename();
            }
            if (event.key === 'Escape') {
              event.preventDefault();
              setDraftName(name);
              onRenameCancel();
            }
          }}
        />
      ) : (
        <span className={styles.groupName}>{name}</span>
      )}
      <span className={styles.count}>{count}</span>
      {canManage ? (
        <button
          ref={menuButtonRef}
          aria-label={`分组操作：${name}`}
          className={styles.moreBtn}
          onClick={openMenuFromButton}
          title="分组操作"
          type="button"
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <circle cx="5" cy="12" r="2" />
            <circle cx="12" cy="12" r="2" />
            <circle cx="19" cy="12" r="2" />
          </svg>
        </button>
      ) : null}
    </div>
  );

  return (
    <>
      <section
        className={`${styles.group} ${dropActive ? styles.dropActive : ''}`}
        data-connection-group-id={id}
      >
        {header}
        {expanded ? <div className={styles.children}>{children}</div> : null}
      </section>
      {menuPosition
        ? createPortal(
            <div
              ref={menuRef}
              className={styles.menu}
              role="menu"
              style={{ left: menuPosition.x, top: menuPosition.y }}
            >
              {menuItems.map((item, index) => (
                <Fragment key={item.label}>
                  {index === menuItems.length - 1 ? (
                    <div className={styles.menuSeparator} role="separator" />
                  ) : null}
                  <button
                    aria-label={item.label}
                    className={`${styles.menuItem} ${item.danger ? styles.danger : ''}`}
                    onClick={() => {
                      setMenuPosition(null);
                      item.onSelect();
                    }}
                    role="menuitem"
                    type="button"
                  >
                    {item.icon}
                    {item.label}
                  </button>
                </Fragment>
              ))}
            </div>,
            document.body
          )
        : null}
    </>
  );
}

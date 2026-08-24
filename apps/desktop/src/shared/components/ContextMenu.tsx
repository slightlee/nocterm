import { createPortal } from 'react-dom';
import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent,
  type ReactNode,
} from 'react';

import styles from './ContextMenu.module.css';

export interface ContextMenuItem {
  label: string;
  icon?: ReactNode;
  danger?: boolean;
  disabled?: boolean;
  onSelect: () => void;
}

interface ContextMenuProps {
  items: (ContextMenuItem | 'separator')[];
  children: ReactNode;
  compact?: boolean;
}

/** 使用原生事件实现右键菜单，避免为迁移阶段新增 Radix 依赖。 */
export function ContextMenu({ items, children, compact = false }: ContextMenuProps) {
  const [position, setPosition] = useState<{ x: number; y: number } | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!position) return;
    requestAnimationFrame(() => menuRef.current?.querySelector('button')?.focus());
    const close = () => setPosition(null);
    const closeOnKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') close();
    };
    document.addEventListener('mousedown', close);
    document.addEventListener('keydown', closeOnKey);
    return () => {
      document.removeEventListener('mousedown', close);
      document.removeEventListener('keydown', closeOnKey);
    };
  }, [position]);

  const openAt = (x: number, y: number) => {
    // 宽度包含内容、内边距和边框，用于确保菜单不会溢出窗口右侧。
    const menuWidth = compact ? 112 : 190;
    const menuHeight = items.reduce((height, item) => height + (item === 'separator' ? 9 : 34), 8);
    setPosition({
      x: Math.max(8, Math.min(x, window.innerWidth - menuWidth - 8)),
      y: Math.max(8, Math.min(y, window.innerHeight - menuHeight - 8)),
    });
  };

  const handleContextMenu = (event: MouseEvent) => {
    event.preventDefault();
    event.stopPropagation();
    openAt(event.clientX, event.clientY);
  };

  const handleKeyDown = (event: ReactKeyboardEvent) => {
    if (event.key !== 'ContextMenu' && !(event.shiftKey && event.key === 'F10')) return;
    event.preventDefault();
    event.stopPropagation();
    const target = event.target as HTMLElement;
    const rect = target.getBoundingClientRect();
    openAt(rect.left + 12, rect.bottom - 4);
  };

  return (
    <>
      <span className={styles.trigger} onContextMenu={handleContextMenu} onKeyDown={handleKeyDown}>
        {children}
      </span>
      {position
        ? createPortal(
            <div
              ref={menuRef}
              aria-label="上下文菜单"
              className={`${styles.menu} ${compact ? styles.compact : ''}`}
              role="menu"
              style={{ left: position.x, top: position.y }}
              onMouseDown={(event) => event.stopPropagation()}
            >
              {items.map((item, index) =>
                item === 'separator' ? (
                  <div className={styles.separator} key={`separator-${index}`} role="separator" />
                ) : (
                  <button
                    aria-label={item.label}
                    className={`${styles.item} ${item.danger ? styles.danger : ''}`}
                    disabled={item.disabled}
                    key={item.label}
                    onClick={() => {
                      setPosition(null);
                      item.onSelect();
                    }}
                    role="menuitem"
                    type="button"
                  >
                    {item.icon ? <span className={styles.itemIcon}>{item.icon}</span> : null}
                    {item.label}
                  </button>
                )
              )}
            </div>,
            document.body
          )
        : null}
    </>
  );
}

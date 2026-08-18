import type { PointerEvent } from 'react';

import { ContextMenu, type ContextMenuItem } from '../../../shared/components/ContextMenu';
import { getConnectionIconOption } from '../model/connection-icons';
import { isConnectionReady, resolveConnectionIndicator } from '../model/connection-list-model';
import type { ConnectionDropPosition } from '../model/use-connection-drag-sort';
import type { ConnectionProfile } from '../types/connection-types';
import styles from './ConnectionItem.module.css';

interface ConnectionItemProps {
  connection: ConnectionProfile;
  selected: boolean;
  onSelect: () => void;
  onOpen: () => void;
  onDisconnect: () => void;
  onEdit: () => void;
  onCopyAddress: () => void;
  onDelete: () => void;
  onClone: () => void;
  dropPosition?: ConnectionDropPosition | null;
  onDragPointerDown: (event: PointerEvent<HTMLButtonElement>) => void;
  dragging: boolean;
  connectionStatus?: 'idle' | 'connecting' | 'connected' | 'closed' | 'error';
  connectionError?: string | null;
}

/** 连接项只展示连接资料与状态，编辑类操作统一放入右键菜单。 */
export function ConnectionItem({
  connection,
  selected,
  onSelect,
  onOpen,
  onDisconnect,
  onEdit,
  onCopyAddress,
  onDelete,
  onClone,
  dropPosition,
  onDragPointerDown,
  dragging,
  connectionStatus,
  connectionError,
}: ConnectionItemProps) {
  const icon = getConnectionIconOption(connection.icon);
  const connected = connectionStatus === 'connected';
  const connecting = connectionStatus === 'connecting';
  const ready = isConnectionReady(connection);
  const indicator = resolveConnectionIndicator(connection, connectionStatus, connectionError);
  const statusLabel =
    connectionStatus === 'connected'
      ? '已连接'
      : connectionStatus === 'connecting'
        ? '连接中'
        : connectionStatus === 'error'
          ? connectionError
            ? `连接失败：${connectionError}`
            : '连接失败'
          : connectionStatus === 'closed'
            ? '已断开'
            : ready
              ? '尚未连接'
              : '尚未绑定凭据';
  const menuItems: (ContextMenuItem | 'separator')[] = [
    {
      label: connected ? '断开连接' : ready ? '连接' : '补全凭据',
      onSelect: connected || ready ? (connected ? onDisconnect : onOpen) : onEdit,
      icon: connected ? (
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <line x1="18" y1="6" x2="6" y2="18" />
          <line x1="6" y1="6" x2="18" y2="18" />
        </svg>
      ) : (
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M5 12h14M12 5l7 7-7 7" />
        </svg>
      ),
    },
    'separator',
    {
      label: '编辑',
      onSelect: onEdit,
      icon: (
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7" />
          <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z" />
        </svg>
      ),
    },
    {
      label: '复制地址',
      onSelect: onCopyAddress,
      icon: (
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <rect x="9" y="9" width="13" height="13" rx="2" />
          <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
        </svg>
      ),
    },
    {
      label: '克隆',
      onSelect: onClone,
      icon: (
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <rect x="9" y="9" width="13" height="13" rx="2" />
          <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
        </svg>
      ),
    },
    'separator',
    {
      label: '删除',
      danger: true,
      onSelect: onDelete,
      icon: (
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <polyline points="3 6 5 6 21 6" />
          <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
        </svg>
      ),
    },
  ];

  const item = (
    <button
      className={`${styles.item} ${connecting ? styles.loading : ''} ${selected ? styles.selected : ''} ${dragging ? styles.dragging : ''} ${
        dropPosition === 'before'
          ? styles.dropBefore
          : dropPosition === 'after'
            ? styles.dropAfter
            : ''
      }`}
      data-connection-id={connection.id}
      onClick={onSelect}
      onDoubleClick={onOpen}
      onPointerDown={onDragPointerDown}
      title={`${connection.name}\n${connection.username}@${connection.host}:${connection.port}${connectionError ? `\n${connectionError}` : ''}`}
      type="button"
    >
      <span className={`${styles.iconContainer} ${styles[icon.color]}`} aria-label={icon.label}>
        {icon.icon}
      </span>
      <span className={styles.itemInfo}>
        <span className={styles.itemName}>{connection.name}</span>
        {connecting ? (
          <span className={styles.itemHost}>连接中…</span>
        ) : (
          <span className={styles.endpoint}>
            <span className={styles.endpointUser}>{connection.username}</span>
            <span className={styles.endpointAt}>@</span>
            <span className={styles.endpointHost}>{connection.host}</span>
            <span className={styles.endpointPort}>{connection.port}</span>
          </span>
        )}
      </span>
      {indicator ? (
        <span
          className={
            indicator === 'readiness'
              ? styles.readinessDot
              : indicator === 'offline'
                ? styles.offlineDot
                : indicator === 'error'
                  ? styles.errorDot
                  : styles.onlineDot
          }
          aria-label={statusLabel}
          title={statusLabel}
        />
      ) : null}
    </button>
  );

  return <ContextMenu items={menuItems}>{item}</ContextMenu>;
}

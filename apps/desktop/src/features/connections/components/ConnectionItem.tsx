import type { PointerEvent } from 'react';

import type { SftpSession } from '../../sftp';
import type { TerminalSession, TerminalSessionId, TerminalStatus } from '../../terminal';
import { ContextMenu, type ContextMenuItem } from '../../../shared/components/ContextMenu';
import { getConnectionIconOption } from '../model/connection-icons';
import { isConnectionReady, resolveConnectionIndicator } from '../model/connection-list-model';
import type { ConnectionDropPosition } from '../model/use-connection-drag-sort';
import type { ConnectionProfile } from '../types/connection-types';
import { ServerSessionPicker } from './ServerSessionPicker';
import styles from './ConnectionItem.module.css';

type RemoteTerminalSession = Extract<TerminalSession, { kind: 'remote' }>;

interface ConnectionItemProps {
  connection: ConnectionProfile;
  selected: boolean;
  terminalActive: boolean;
  terminalSessions: RemoteTerminalSession[];
  activeTerminalId: TerminalSessionId | null;
  terminalStatuses: Record<string, TerminalStatus>;
  terminalErrors: Record<string, string | null>;
  fileSession: SftpSession | null;
  filesActive: boolean;
  onSelect: () => void;
  onOpen: () => void;
  onOpenTerminal: () => void;
  onActivateTerminalSession: (sessionId: TerminalSessionId) => void;
  onOpenFiles: () => void;
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
  terminalActive,
  terminalSessions,
  activeTerminalId,
  terminalStatuses,
  terminalErrors,
  fileSession,
  filesActive,
  onSelect,
  onOpen,
  onOpenTerminal,
  onActivateTerminalSession,
  onOpenFiles,
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

  const sessionCount = terminalSessions.length + (fileSession ? 1 : 0);
  const item = (
    <div
      className={`${styles.item} ${sessionCount > 0 ? styles.hasSessions : ''} ${connecting ? styles.loading : ''} ${selected ? styles.selected : ''} ${dragging ? styles.dragging : ''} ${
        dropPosition === 'before'
          ? styles.dropBefore
          : dropPosition === 'after'
            ? styles.dropAfter
            : ''
      }`}
      data-connection-id={connection.id}
    >
      <button
        className={styles.itemButton}
        onClick={onSelect}
        onDoubleClick={onOpen}
        onKeyDown={(event) => {
          if (event.key !== 'Enter') return;
          event.preventDefault();
          onOpen();
        }}
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
          <span className={styles.statusSlot} aria-label={statusLabel} title={statusLabel}>
            <span
              aria-hidden="true"
              className={
                indicator === 'readiness'
                  ? styles.readinessDot
                  : indicator === 'offline'
                    ? styles.offlineDot
                    : indicator === 'error'
                      ? styles.errorDot
                      : styles.onlineDot
              }
            />
          </span>
        ) : null}
      </button>
      {sessionCount > 0 ? (
        <ServerSessionPicker
          activeSessionId={activeTerminalId}
          connection={connection}
          errors={terminalErrors}
          fileSession={fileSession}
          filesActive={filesActive}
          onActivate={onActivateTerminalSession}
          onActivateFiles={onOpenFiles}
          sessions={terminalSessions}
          statuses={terminalStatuses}
          terminalActive={terminalActive}
        />
      ) : null}
      <span className={styles.actions} aria-label={`${connection.name} 快捷操作`} role="group">
        <button
          aria-label={`打开终端：${connection.name}`}
          aria-pressed={terminalActive}
          className={`${styles.actionButton} ${terminalActive ? styles.activeAction : ''}`}
          onClick={(event) => {
            event.stopPropagation();
            onOpenTerminal();
          }}
          onDoubleClick={(event) => event.stopPropagation()}
          onPointerDown={(event) => event.stopPropagation()}
          title={`打开终端 · ${connection.username}@${connection.host}:${connection.port}`}
          type="button"
        >
          <svg viewBox="0 0 20 20" aria-hidden="true">
            <path d="m3 5.5 4 4-4 4M9 14h4M15 3v6M12 6h6" />
          </svg>
        </button>
        <button
          aria-label={`打开或切回文件管理：${connection.name}`}
          aria-pressed={filesActive}
          className={`${styles.actionButton} ${filesActive ? styles.activeAction : ''}`}
          onClick={(event) => {
            event.stopPropagation();
            onOpenFiles();
          }}
          onDoubleClick={(event) => event.stopPropagation()}
          onPointerDown={(event) => event.stopPropagation()}
          title={`打开或切回文件管理 · ${connection.username}@${connection.host}:${connection.port}`}
          type="button"
        >
          <svg viewBox="0 0 20 20" aria-hidden="true">
            <path d="M2.75 6.25A1.75 1.75 0 0 1 4.5 4.5h4l1.75 2h5.25a1.75 1.75 0 0 1 1.75 1.75v6.25a2 2 0 0 1-2 2H4.75a2 2 0 0 1-2-2z" />
          </svg>
        </button>
      </span>
    </div>
  );

  return <ContextMenu items={menuItems}>{item}</ContextMenu>;
}

import type { TerminalSessionId } from '../../features/terminal';

export type WorkspaceSessionTabKey = `terminal:${string}` | `sftp:${string}`;

export interface WorkspaceTabScrollMetrics {
  scrollLeft: number;
  scrollWidth: number;
  clientWidth: number;
}

export interface WorkspaceSessionIndexItem {
  key: WorkspaceSessionTabKey;
  groupId: string;
  groupLabel: string;
  label: string;
  searchText: string;
}

export interface WorkspaceSessionIndexGroup<T extends WorkspaceSessionIndexItem> {
  id: string;
  label: string;
  entries: T[];
}

export function terminalWorkspaceTabKey(id: TerminalSessionId): WorkspaceSessionTabKey {
  return `terminal:${String(id)}`;
}

export function sftpWorkspaceTabKey(connectionId: string): WorkspaceSessionTabKey {
  return `sftp:${connectionId}`;
}

/** 统一标签顺序保持稳定：终端按打开顺序排列，文件会话紧随其后。 */
export function createWorkspaceSessionTabKeys(
  terminalIds: readonly TerminalSessionId[],
  sftpConnectionIds: readonly string[]
): WorkspaceSessionTabKey[] {
  return [
    ...terminalIds.map(terminalWorkspaceTabKey),
    ...sftpConnectionIds.map(sftpWorkspaceTabKey),
  ];
}

export function formatWorkspaceSessionNumber(value: number) {
  return String(value).padStart(2, '0');
}

/** 三像素容差避免高分屏小数滚动值让箭头在边界反复闪烁。 */
export function resolveWorkspaceTabOverflow({
  scrollLeft,
  scrollWidth,
  clientWidth,
}: WorkspaceTabScrollMetrics) {
  const maxScrollLeft = Math.max(0, scrollWidth - clientWidth);
  const normalizedScrollLeft = Math.min(maxScrollLeft, Math.max(0, scrollLeft));
  return {
    left: normalizedScrollLeft > 3,
    right: maxScrollLeft - normalizedScrollLeft > 3,
  };
}

/** 搜索后仍按首次出现顺序分组，保证弹层与顶部标签的阅读顺序一致。 */
export function groupWorkspaceSessionIndexItems<T extends WorkspaceSessionIndexItem>(
  entries: readonly T[],
  query: string
): WorkspaceSessionIndexGroup<T>[] {
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const groups = new Map<string, WorkspaceSessionIndexGroup<T>>();

  for (const entry of entries) {
    const searchable = `${entry.groupLabel} ${entry.label} ${entry.searchText}`.toLocaleLowerCase();
    if (normalizedQuery && !searchable.includes(normalizedQuery)) continue;

    const group = groups.get(entry.groupId);
    if (group) {
      group.entries.push(entry);
    } else {
      groups.set(entry.groupId, {
        id: entry.groupId,
        label: entry.groupLabel,
        entries: [entry],
      });
    }
  }

  return [...groups.values()];
}

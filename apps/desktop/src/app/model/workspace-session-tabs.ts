import type { TerminalSessionId } from '../../features/terminal';

export type WorkspaceSessionTabKey = `terminal:${string}` | `sftp:${string}` | `editor:${string}`;

export interface WorkspaceEditorDependency {
  id: string;
  source: { kind: 'local' } | { kind: 'remote'; connectionId: string };
}

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

export function editorWorkspaceTabKey(id: string): WorkspaceSessionTabKey {
  return `editor:${id}`;
}

/** 顺序与 DOM 一致：终端、文件管理、编辑器各自保持打开顺序。 */
export function createWorkspaceSessionTabKeys(
  terminalIds: readonly TerminalSessionId[],
  sftpConnectionIds: readonly string[],
  editorIds: readonly string[] = []
): WorkspaceSessionTabKey[] {
  return [
    ...terminalIds.map(terminalWorkspaceTabKey),
    ...sftpConnectionIds.map(sftpWorkspaceTabKey),
    ...editorIds.map(editorWorkspaceTabKey),
  ];
}

/** 关闭远程管理器必须包含其编辑器；本地文件没有连接依赖。 */
export function expandWorkspaceTabCloseKeys(
  keys: readonly WorkspaceSessionTabKey[],
  editors: readonly WorkspaceEditorDependency[]
): WorkspaceSessionTabKey[] {
  const expanded = new Set(keys);
  for (const editor of editors) {
    if (
      editor.source.kind === 'remote' &&
      expanded.has(sftpWorkspaceTabKey(editor.source.connectionId))
    ) {
      expanded.add(editorWorkspaceTabKey(editor.id));
    }
  }
  return [...expanded];
}

/** “关闭其他”保留远程编辑器时也保留父管理器，避免依赖展开反向关掉目标。 */
export function preserveWorkspaceEditorParent(
  keys: readonly WorkspaceSessionTabKey[],
  retainedKey: WorkspaceSessionTabKey,
  editors: readonly WorkspaceEditorDependency[]
): WorkspaceSessionTabKey[] {
  const retained = editors.find((editor) => editorWorkspaceTabKey(editor.id) === retainedKey);
  if (keys.includes(retainedKey) || retained?.source.kind !== 'remote') return [...keys];
  const parentKey = sftpWorkspaceTabKey(retained.source.connectionId);
  return keys.filter((key) => key !== parentKey);
}

/** 活动标签关闭后优先选左侧最近幸存者；没有左邻时再找右邻。 */
export function resolveWorkspaceTabCloseFallback(
  orderedKeys: readonly WorkspaceSessionTabKey[],
  activeKey: WorkspaceSessionTabKey | null,
  closedKeys: readonly WorkspaceSessionTabKey[]
): WorkspaceSessionTabKey | null {
  if (!activeKey || !closedKeys.includes(activeKey)) return activeKey;
  const index = orderedKeys.indexOf(activeKey);
  const closed = new Set(closedKeys);
  return (
    orderedKeys
      .slice(0, index)
      .reverse()
      .find((key) => !closed.has(key)) ??
    orderedKeys.slice(index + 1).find((key) => !closed.has(key)) ??
    null
  );
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

import type {
  ConnectionCreateRequest,
  ConnectionGroup,
  ConnectionProfile,
} from '../types/connection-types';

export const UNGROUPED_CONNECTION_GROUP_ID = '__ungrouped__';

/** 新建分组沿用旧客户端命名规则，避免弹出依赖 WebView 支持的 prompt。 */
export function getNextGroupName(groups: Pick<ConnectionGroup, 'name'>[]): string {
  const existingNames = new Set(groups.map((group) => group.name));
  const baseName = '新建分组';
  if (!existingNames.has(baseName)) return baseName;

  let index = 2;
  while (existingNames.has(`${baseName} ${index}`)) index += 1;
  return `${baseName} ${index}`;
}

/** 密码可在终端交互输入；私钥必须绑定，SSH Agent 直接使用本机运行环境。 */
export function isConnectionReady(connection: ConnectionProfile) {
  if (connection.authentication === 'password') return true;
  return (
    connection.credentialStatus === 'bound' &&
    connection.credentialKind === connection.authentication
  );
}

export type ConnectionIndicator = 'readiness' | 'offline' | 'error' | 'connected' | null;

/** 凭据状态与连接状态分色，避免把正常离线误标成需要补全凭据。 */
export function resolveConnectionIndicator(
  connection: ConnectionProfile,
  status?: 'idle' | 'connecting' | 'connected' | 'closed' | 'error',
  error?: string | null
): ConnectionIndicator {
  if (status === 'connected') return 'connected';
  if (status === 'error' || error) return 'error';
  if (!isConnectionReady(connection)) return 'readiness';
  if (status === 'connecting') return null;
  return 'offline';
}

/** 凭据写入成功后更新前端内存态，字段必须与 IPC camelCase DTO 保持一致。 */
export function markCredentialBound(
  connection: ConnectionProfile,
  credentialKind: 'password' | 'private_key'
): ConnectionProfile {
  return {
    ...connection,
    credentialKind,
    credentialStatus: 'bound',
  };
}

/** 连接目标或启动目录变化后，已有终端必须按新资料重连。 */
export function connectionRequiresReconnect(
  previous: Pick<
    ConnectionProfile,
    'host' | 'port' | 'username' | 'authentication' | 'remoteInitialPath'
  >,
  next: Pick<
    ConnectionCreateRequest,
    'host' | 'port' | 'username' | 'authentication' | 'remoteInitialPath'
  >
) {
  return (
    next.host !== previous.host ||
    next.port !== previous.port ||
    next.username !== previous.username ||
    next.authentication !== previous.authentication ||
    (next.remoteInitialPath ?? '') !== (previous.remoteInitialPath ?? '')
  );
}

export interface ConnectionGroupView {
  id: string;
  name: string;
  connections: ConnectionProfile[];
}

/** 分组写入后仍按持久化排序权重呈现，避免重命名导致分组跳到末尾。 */
export function sortConnectionGroups(groups: ConnectionGroup[]): ConnectionGroup[] {
  return [...groups].sort((left, right) => {
    const leftOrder = left.sortOrder ?? Number.MAX_SAFE_INTEGER;
    const rightOrder = right.sortOrder ?? Number.MAX_SAFE_INTEGER;
    if (leftOrder !== rightOrder) return leftOrder - rightOrder;
    return left.name.localeCompare(right.name, 'zh-CN');
  });
}

export type GroupDropPosition = 'before' | 'after';

/** 分组拖拽仅改写目标分组的排序权重，不触碰分组归属和连接资料。 */
export function resolveGroupDropSortOrder(
  groups: ConnectionGroup[],
  draggingGroupId: string,
  targetGroupId: string,
  position: GroupDropPosition
): number | null {
  if (draggingGroupId === targetGroupId) return null;
  const ordered = sortConnectionGroups(groups).filter((group) => group.id !== draggingGroupId);
  const targetIndex = ordered.findIndex((group) => group.id === targetGroupId);
  if (targetIndex < 0) return null;

  const insertionIndex = targetIndex + (position === 'after' ? 1 : 0);
  const previous = ordered[insertionIndex - 1];
  const next = ordered[insertionIndex];
  if (previous && next) {
    const previousOrder = previous.sortOrder ?? (insertionIndex - 1) * 1000;
    const nextOrder = next.sortOrder ?? insertionIndex * 1000;
    return (previousOrder + nextOrder) / 2;
  }
  if (previous) return (previous.sortOrder ?? (insertionIndex - 1) * 1000) + 1000;
  if (next) return (next.sortOrder ?? insertionIndex * 1000) - 1000;
  return 0;
}

/** 将后端的扁平连接资料组织成稳定的分组视图，避免组件承担排序规则。 */
export function groupConnections(
  connections: ConnectionProfile[],
  groups: ConnectionGroup[]
): ConnectionGroupView[] {
  const grouped = sortConnectionGroups(groups).map((group) => ({
    id: group.id,
    name: group.name,
    connections: sortConnections(
      connections.filter((connection) => connection.groupId === group.id)
    ),
  }));
  const ungrouped = sortConnections(connections.filter((connection) => !connection.groupId));
  if (ungrouped.length > 0) {
    grouped.push({
      id: UNGROUPED_CONNECTION_GROUP_ID,
      name: '未分组',
      connections: ungrouped,
    });
  }
  // 显式创建的空分组也是用户资料，必须保留在列表中以便立即重命名或添加连接。
  return grouped;
}

function sortConnections(connections: ConnectionProfile[]) {
  return [...connections].sort((left, right) => {
    const leftOrder = left.sortOrder ?? Number.MAX_SAFE_INTEGER;
    const rightOrder = right.sortOrder ?? Number.MAX_SAFE_INTEGER;
    if (leftOrder !== rightOrder) return leftOrder - rightOrder;
    return left.id - right.id;
  });
}

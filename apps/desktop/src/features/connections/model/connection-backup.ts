import type {
  AuthenticationMethod,
  ConnectionBackupImportEntry,
  ConnectionBackupImportRequest,
  ConnectionGroup,
  ConnectionIcon,
  ConnectionProfile,
} from '../types/connection-types';

const BACKUP_SCHEMA = 'nocterm.connection-backup';
const BACKUP_VERSION = 1;

interface BackupCredential {
  connectionId: string;
  credentialKind: AuthenticationMethod;
  credentialStatus: 'metadata_only' | 'bound';
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function optionalString(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value.trim() : undefined;
}

function connectionIcon(value: unknown): ConnectionIcon | undefined {
  return ['server', 'cloud', 'database', 'terminal', 'web', 'build', 'cache', 'storage'].includes(
    String(value)
  )
    ? (value as ConnectionIcon)
    : undefined;
}

function authenticationMethod(
  profile: Record<string, unknown>,
  credential?: BackupCredential
): AuthenticationMethod {
  if (
    profile.authentication === 'password' ||
    profile.authentication === 'private_key' ||
    profile.authentication === 'ssh_agent'
  ) {
    return profile.authentication;
  }
  if (profile.authType === 1) return 'password';
  if (profile.authType === 2) {
    return credential?.credentialKind === 'ssh_agent' ? 'ssh_agent' : 'private_key';
  }
  throw new Error('备份文件包含不受支持的认证方式');
}

function parseCredentials(value: unknown): Map<string, BackupCredential> {
  if (!Array.isArray(value)) return new Map();
  const credentials = new Map<string, BackupCredential>();
  value.forEach((item) => {
    if (!isRecord(item)) throw new Error('备份文件中的凭据元数据格式无效');
    const connectionId = String(item.connectionId ?? '');
    const credentialKind = item.credentialKind;
    if (
      !connectionId ||
      (credentialKind !== 'password' &&
        credentialKind !== 'private_key' &&
        credentialKind !== 'ssh_agent')
    ) {
      throw new Error('备份文件中的凭据元数据格式无效');
    }
    credentials.set(connectionId, {
      connectionId,
      credentialKind,
      credentialStatus: item.credentialStatus === 'bound' ? 'bound' : 'metadata_only',
    });
  });
  return credentials;
}

function parseGroups(value: unknown): ConnectionGroup[] {
  if (!Array.isArray(value)) throw new Error('备份文件缺少分组列表');
  return value.map((item, index) => {
    if (!isRecord(item)) throw new Error('备份文件中的分组格式无效');
    const id = optionalString(item.groupId ?? item.id);
    const name = optionalString(item.groupName ?? item.name);
    if (!id || !name) throw new Error('备份文件中的分组格式无效');
    return {
      id,
      name,
      sortOrder: typeof item.sortOrder === 'number' ? item.sortOrder : index,
    };
  });
}

function parseProfiles(
  value: unknown,
  credentials: Map<string, BackupCredential>
): ConnectionBackupImportEntry[] {
  if (!Array.isArray(value)) throw new Error('备份文件缺少连接列表');
  return value.map((item, index) => {
    if (!isRecord(item)) throw new Error('备份文件中的连接格式无效');
    const sourceId = String(item.connectionId ?? item.id ?? '');
    const name = optionalString(item.connectionName ?? item.name);
    const host = optionalString(item.host);
    const username = optionalString(item.username);
    const port = item.port;
    if (
      !sourceId ||
      !name ||
      !host ||
      !username ||
      !Number.isInteger(port) ||
      Number(port) < 1 ||
      Number(port) > 65535
    ) {
      throw new Error('备份文件中的连接格式无效');
    }
    const credential = credentials.get(sourceId);
    return {
      sourceId,
      name,
      host,
      port: Number(port),
      username,
      authentication: authenticationMethod(item, credential),
      groupId: optionalString(item.groupId),
      remark: optionalString(item.remark),
      remoteInitialPath: optionalString(item.remoteInitialPath),
      icon: connectionIcon(item.icon),
      sortOrder: typeof item.sortOrder === 'number' ? item.sortOrder : index,
      credentialKind: credential?.credentialKind,
      credentialStatus: credential?.credentialStatus,
    };
  });
}

/** 输出旧版兼容格式；凭据数组只描述绑定状态，不包含任何 secret 或路径。 */
export function createConnectionBackup(
  connections: ConnectionProfile[],
  groups: ConnectionGroup[]
): string {
  const exportedAt = new Date().toISOString();
  return JSON.stringify(
    {
      schema: BACKUP_SCHEMA,
      version: BACKUP_VERSION,
      exportedAt,
      groups: groups.map((group) => ({
        groupId: group.id,
        groupName: group.name,
        createdAt: exportedAt,
        updatedAt: exportedAt,
        sortOrder: group.sortOrder,
      })),
      profiles: connections.map((connection) => ({
        connectionId: String(connection.id),
        connectionName: connection.name,
        host: connection.host,
        port: connection.port,
        username: connection.username,
        authType: connection.authentication === 'password' ? 1 : 2,
        groupId: connection.groupId ?? null,
        remark: connection.remark ?? null,
        syncMode: 'local_only',
        executionTarget: 'remote_terminal',
        remoteInitialPath: connection.remoteInitialPath ?? null,
        icon: connection.icon ?? 'server',
        sortOrder: connection.sortOrder ?? null,
        updatedAt: connection.updatedAt,
      })),
      credentials: connections.map((connection) => ({
        connectionId: String(connection.id),
        credentialKind: connection.authentication,
        credentialStatus: connection.credentialStatus === 'bound' ? 'bound' : 'metadata_only',
      })),
    },
    null,
    2
  );
}

/** 导入在任何数据库写入前完成完整解析，防止半合法文件造成部分数据落库。 */
export function parseConnectionBackup(content: string): ConnectionBackupImportRequest {
  let value: unknown;
  try {
    value = JSON.parse(content);
  } catch {
    throw new Error('备份文件不是有效的 JSON');
  }
  if (!isRecord(value) || value.schema !== BACKUP_SCHEMA || value.version !== BACKUP_VERSION) {
    throw new Error('备份文件格式不受支持');
  }
  const credentials = parseCredentials(value.credentials);
  return {
    groups: parseGroups(value.groups),
    connections: parseProfiles(value.profiles, credentials),
  };
}

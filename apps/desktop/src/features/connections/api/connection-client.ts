import { invoke } from '@tauri-apps/api/core';
import { homeDir } from '@tauri-apps/api/path';
import { open } from '@tauri-apps/plugin-dialog';

import type {
  AppError,
  ConnectionBackupImportRequest,
  ConnectionCreateRequest,
  ConnectionGroup,
  ConnectionProfile,
  ConnectionImportResult,
} from '../types/connection-types';

function isAppError(value: unknown): value is AppError {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = value as Record<string, unknown>;
  return (
    typeof candidate.code === 'string' &&
    typeof candidate.message === 'string' &&
    typeof candidate.retryable === 'boolean'
  );
}

/** IPC 拒绝值可能不是 Error，统一转换后 UI 无需理解 Tauri 的异常形态。 */
export function normalizeConnectionError(error: unknown): AppError {
  if (isAppError(error)) return error;
  return {
    code: 'CONNECTION_UNKNOWN_ERROR',
    message: error instanceof Error ? error.message : '连接操作失败，请稍后重试',
    retryable: true,
  };
}

export function listConnections(): Promise<ConnectionProfile[]> {
  return invoke<ConnectionProfile[]>('connection_list');
}

/**
 * 只有密码需要从资料里剥离——它写系统凭据库；
 * `privateKeyPath` 是本机元数据，必须随资料一起提交，否则后端无从知道绑定了哪个私钥。
 */
export function createConnection(request: ConnectionCreateRequest): Promise<ConnectionProfile> {
  const { password: _password, ...profile } = request;
  void _password;
  return invoke<ConnectionProfile>('connection_create', { request: profile });
}

export function updateConnection(request: ConnectionCreateRequest): Promise<ConnectionProfile> {
  const { password: _password, ...profile } = request;
  void _password;
  // 私钥没有"凭据"分支：路径已在资料里，凭据库只承载密码。
  const credential =
    request.authentication === 'password' && request.password
      ? { kind: 'password', secret: request.password }
      : undefined;
  return invoke<ConnectionProfile>('connection_update', { request: profile, credential });
}

export function deleteConnection(id: number) {
  return invoke<void>('connection_delete', { id });
}

export function listConnectionGroups(): Promise<ConnectionGroup[]> {
  return invoke<ConnectionGroup[]>('connection_group_list');
}

export function upsertConnectionGroup(group: ConnectionGroup): Promise<ConnectionGroup> {
  return invoke<ConnectionGroup>('connection_group_upsert', { request: group });
}

export function deleteConnectionGroup(id: string) {
  return invoke<void>('connection_group_delete', { id });
}

export function reorderConnection(id: number, groupId: string | null, sortOrder: number) {
  return invoke<ConnectionProfile>('connection_reorder', {
    id,
    groupId,
    sortOrder,
  });
}

export function importConnectionBackup(
  request: ConnectionBackupImportRequest
): Promise<ConnectionImportResult> {
  return invoke<ConnectionImportResult>('connection_backup_import', { request });
}

export function pickConnectionBackup(): Promise<string | null> {
  return invoke<string | null>('connection_backup_pick_and_read');
}

export interface SelectedSshConfigFile {
  path: string;
  content: string;
}

export function pickSshConfig(): Promise<SelectedSshConfigFile | null> {
  return invoke<SelectedSshConfigFile | null>('connection_ssh_config_pick_and_read');
}

export function saveConnectionBackup(content: string, defaultFileName: string): Promise<boolean> {
  return invoke<boolean>('connection_backup_save', { content, defaultFileName });
}

/** SSH Config 未指定 User 时沿用旧客户端规则，以当前用户目录名作为兜底，不猜测 root。 */
export async function resolveLocalUsername(): Promise<string> {
  const home = (await homeDir()).replace(/[\\/]+$/, '');
  return home.split(/[\\/]/).pop() ?? '';
}

/** 只展开 OpenSSH 常见的 ~/ 路径；相对路径不安全也无法稳定解析，保留为待用户手动绑定。 */
export async function resolveSshConfigPrivateKeyPath(path: string): Promise<string | null> {
  if (path.startsWith('~/')) return `${await homeDir()}${path.slice(1)}`;
  if (path.startsWith('/') || /^[A-Za-z]:[\\/]/.test(path)) return path;
  return null;
}

/** 只有密码写入系统凭据库：私钥按路径引用，SSH Agent 无凭据可存。 */
export function storeConnectionCredential(
  connectionId: number,
  credentialKind: 'password',
  secret: string
) {
  return invoke<void>('credential_store', {
    connectionId: String(connectionId),
    credentialKind,
    secret,
  });
}

/**
 * 为已存在的连接补绑私钥文件：只记录路径，密钥字节不进系统凭据库。
 *
 * SSH Config 导入这类"先建资料、再补路径"的流程需要它；
 * 普通新建/编辑走 `createConnection` / `updateConnection` 即可，路径已在资料里。
 */
export function bindConnectionPrivateKeyFile(connectionId: number, path: string) {
  return invoke<void>('connection_bind_private_key', {
    connectionId: String(connectionId),
    path,
  });
}

/** 文件选择属于桌面平台能力，组件只消费规范化后的路径结果。 */
export async function selectPrivateKeyFile(): Promise<string | null> {
  const selected = await open({
    directory: false,
    multiple: false,
    filters: [
      {
        name: 'SSH Private Key',
        extensions: ['pem', 'key', 'pub', 'txt', 'ppk'],
      },
    ],
  });
  return typeof selected === 'string' ? selected : null;
}

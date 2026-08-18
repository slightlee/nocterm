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

export function createConnection(request: ConnectionCreateRequest): Promise<ConnectionProfile> {
  const {
    password: _password,
    privateKey: _privateKey,
    privateKeyPath: _privateKeyPath,
    ...profile
  } = request;
  void _password;
  void _privateKey;
  void _privateKeyPath;
  return invoke<ConnectionProfile>('connection_create', { request: profile });
}

export function updateConnection(request: ConnectionCreateRequest): Promise<ConnectionProfile> {
  const {
    password: _password,
    privateKey: _privateKey,
    privateKeyPath: _privateKeyPath,
    ...profile
  } = request;
  void _password;
  void _privateKey;
  void _privateKeyPath;
  const credential =
    request.authentication === 'password' && request.password
      ? { kind: 'password', secret: request.password }
      : request.authentication === 'private_key' && request.privateKey
        ? { kind: 'private_key', secret: request.privateKey }
        : request.authentication === 'private_key' && request.privateKeyPath
          ? { kind: 'private_key', privateKeyPath: request.privateKeyPath }
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

export function storeConnectionCredential(
  connectionId: number,
  credentialKind: 'password' | 'private_key',
  secret: string
) {
  return invoke<void>('credential_store', {
    connectionId: String(connectionId),
    credentialKind,
    secret,
  });
}

export function storeConnectionCredentialFile(
  connectionId: number,
  credentialKind: 'private_key',
  path: string
) {
  return invoke<void>('credential_store_file', {
    connectionId: String(connectionId),
    credentialKind,
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

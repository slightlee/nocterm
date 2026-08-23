export type AuthenticationMethod = 'password' | 'private_key' | 'ssh_agent';
export type ConnectionIcon =
  'server' | 'cloud' | 'database' | 'terminal' | 'web' | 'build' | 'cache' | 'storage';

export interface ConnectionProfile {
  id: number;
  name: string;
  host: string;
  port: number;
  username: string;
  authentication: AuthenticationMethod;
  createdAt: number;
  updatedAt: number;
  groupId?: string | null;
  groupName?: string | null;
  remark?: string | null;
  syncMode?: string;
  executionTarget?: string;
  remoteInitialPath?: string | null;
  icon?: ConnectionIcon | null;
  sortOrder?: number | null;
  /** 私钥文件路径（等价于 OpenSSH 的 IdentityFile）；后端只回传路径，密钥内容始终留在文件里。 */
  privateKeyPath?: string | null;
  credentialKind?: AuthenticationMethod | null;
  credentialStatus?: 'missing' | 'bound' | 'metadata_only';
}

export interface ConnectionGroup {
  id: string;
  name: string;
  sortOrder: number | null;
}

export interface ConnectionCreateRequest {
  id?: number;
  name: string;
  host: string;
  port: number;
  username: string;
  authentication: AuthenticationMethod;
  /** 密码只在单次保存请求里出现，随即写入系统凭据库，不回传也不落库。 */
  password?: string;
  /** 私钥按路径引用：与密码不同，它随连接资料一起持久化。 */
  privateKeyPath?: string;
  groupId?: string;
  remark?: string;
  remoteInitialPath?: string;
  icon?: ConnectionIcon;
}

export interface AppError {
  code: string;
  message: string;
  retryable: boolean;
}

export interface ConnectionBackupImportEntry {
  sourceId: string;
  name: string;
  host: string;
  port: number;
  username: string;
  authentication: AuthenticationMethod;
  groupId?: string;
  remark?: string;
  remoteInitialPath?: string;
  icon?: ConnectionIcon;
  sortOrder?: number;
  credentialKind?: AuthenticationMethod;
  credentialStatus?: 'missing' | 'bound' | 'metadata_only';
}

export interface ConnectionBackupImportRequest {
  groups: ConnectionGroup[];
  connections: ConnectionBackupImportEntry[];
}

export interface ConnectionImportResult {
  groups: number;
  connections: number;
  credentials: number;
  importedConnections: Array<{
    sourceId: string;
    id: number;
  }>;
}

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
  password?: string;
  privateKey?: string;
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

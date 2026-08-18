import type { ConnectionBackupImportEntry } from '../types/connection-types';

const DEFAULT_SSH_PORT = 22;

export interface SshConfigImportEntry extends ConnectionBackupImportEntry {
  privateKeyPath?: string;
}

function stripInlineComment(line: string): string {
  let quote: '"' | "'" | null = null;
  for (let index = 0; index < line.length; index += 1) {
    const character = line[index];
    if ((character === '"' || character === "'") && line[index - 1] !== '\\') {
      quote = quote === character ? null : (quote ?? character);
    }
    if (character === '#' && quote === null) return line.slice(0, index).trim();
  }
  return line.trim();
}

function unquote(value: string): string {
  const trimmed = value.trim();
  if (
    (trimmed.startsWith('"') && trimmed.endsWith('"')) ||
    (trimmed.startsWith("'") && trimmed.endsWith("'"))
  ) {
    return trimmed.slice(1, -1);
  }
  return trimmed;
}

function isConcreteHost(pattern: string): boolean {
  return !pattern.includes('*') && !pattern.includes('?') && !pattern.startsWith('!');
}

function isResolvableLocalPath(path: string): boolean {
  return path.startsWith('/') || /^[A-Za-z]:[\\/]/.test(path) || path.startsWith('~/');
}

interface HostOptions {
  host?: string;
  username?: string;
  port?: number;
  privateKeyPath?: string;
}

/**
 * 只处理旧客户端支持的 Host 块与常用字段，通配规则、Include 和 ProxyJump 仍交给系统 OpenSSH。
 * 解析阶段不访问文件系统，调用方在事务提交后再把 IdentityFile 写入系统凭据库。
 */
export function parseSshConfigConnections(
  content: string,
  groupId: string,
  sourcePath: string,
  fallbackUsername: string
): SshConfigImportEntry[] {
  const imported: SshConfigImportEntry[] = [];
  let currentHosts: string[] = [];
  let current: HostOptions = {};
  let blockIndex = 0;

  const flush = () => {
    for (const alias of currentHosts.filter(isConcreteHost)) {
      const privateKeyPath = current.privateKeyPath;
      imported.push({
        sourceId: `ssh-config-${blockIndex}-${alias}`,
        name: alias,
        host: current.host ?? alias,
        port: current.port ?? DEFAULT_SSH_PORT,
        username: current.username ?? fallbackUsername,
        authentication: privateKeyPath ? 'private_key' : 'password',
        groupId,
        remark: `从 ${sourcePath} 导入`,
        icon: 'server',
        sortOrder: imported.length,
        credentialKind: privateKeyPath ? 'private_key' : 'password',
        credentialStatus: privateKeyPath ? 'metadata_only' : 'missing',
        privateKeyPath:
          privateKeyPath && isResolvableLocalPath(privateKeyPath) ? privateKeyPath : undefined,
      });
    }
    blockIndex += 1;
  };

  for (const rawLine of content.split(/\r?\n/)) {
    const line = stripInlineComment(rawLine);
    if (!line) continue;
    const match = /^([^\s]+)\s+(.+)$/.exec(line);
    if (!match) continue;

    const key = match[1].toLowerCase();
    const value = match[2].trim();
    if (key === 'host') {
      flush();
      currentHosts = value.split(/\s+/).map(unquote);
      current = {};
      continue;
    }
    if (currentHosts.length === 0) continue;

    if (key === 'hostname') current.host = unquote(value);
    if (key === 'user') current.username = unquote(value);
    if (key === 'port') {
      const port = Number(value);
      if (Number.isInteger(port) && port > 0 && port <= 65535) current.port = port;
    }
    if (key === 'identityfile' && !current.privateKeyPath) current.privateKeyPath = unquote(value);
  }
  flush();

  return imported;
}

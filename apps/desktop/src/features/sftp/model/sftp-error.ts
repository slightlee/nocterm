const SFTP_ERROR_PREFIX = '__NOCTERM_SFTP_ERROR__';

type SftpErrorCode =
  | 'passwordUnsupported'
  | 'authFailed'
  | 'hostKeyFailed'
  | 'timeout'
  | 'connectionRefused'
  | 'hostUnreachable'
  | 'hostResolveFailed'
  | 'pathMissing'
  | 'permissionDenied'
  | 'invalidPath'
  | 'localCredentialMissing'
  | 'passwordRequired'
  | 'localReadFailed'
  | 'localWriteFailed'
  | 'unknown';

interface ParsedSftpError {
  code: SftpErrorCode;
  message: string;
  detail: string;
}

const SFTP_ERROR_MESSAGES: Record<SftpErrorCode, string> = {
  passwordUnsupported:
    '认证失败：缺少可用于非交互 SSH 的本机密码凭据，请编辑连接重新保存密码、绑定私钥或使用 SSH Agent。',
  authFailed: '认证失败：服务器拒绝登录，请检查用户名、密码、私钥或 SSH Agent 配置。',
  hostKeyFailed: '主机校验失败：请先在 SSH 终端确认服务器指纹，再返回文件页面重新连接。',
  timeout: '连接超时：请检查服务器地址、端口或网络连通性。',
  connectionRefused: '连接被拒绝：请检查 SSH 服务是否启动，以及端口是否正确。',
  hostUnreachable: '无法连接服务器：请检查网络、防火墙或服务器状态。',
  hostResolveFailed: '主机解析失败：请检查服务器地址是否正确。',
  pathMissing: '路径不存在：请检查远程目录是否已被删除或路径是否正确。',
  permissionDenied: '权限不足：当前账号没有执行该远程文件操作的权限。',
  invalidPath: '远程路径无效：请检查路径内容后重试。',
  localCredentialMissing: '本机凭据缺失：请编辑连接重新保存密码、重新绑定私钥或切换为 SSH Agent。',
  // 文件页面没有输入提示符，只能引导用户去终端输入一次或把密码保存下来。
  // 口令只在该连接还有活跃终端会话时驻留内存，因此提示里必须点明"保持终端标签打开"。
  passwordRequired:
    '尚未提供登录密码：请先打开该连接的 SSH 终端输入一次密码并保持该标签打开，或编辑连接保存密码。',
  localReadFailed: '读取本地文件失败：请检查文件是否仍存在，以及当前账号是否有读取权限。',
  localWriteFailed: '写入本地文件失败：原目标已保留，请检查磁盘空间和目录权限后重试。',
  unknown: '远程文件操作失败，请检查连接配置或稍后重试。',
};

function getErrorText(err: unknown): string {
  if (typeof err === 'object' && err !== null) {
    const candidate = err as { code?: unknown; message?: unknown };
    if (typeof candidate.code === 'string' || typeof candidate.message === 'string') {
      return `${typeof candidate.code === 'string' ? candidate.code : ''}\t${typeof candidate.message === 'string' ? candidate.message : ''}`;
    }
  }
  return err instanceof Error ? err.message : String(err);
}

function parseSftpError(message: string): ParsedSftpError | null {
  if (!message.startsWith(SFTP_ERROR_PREFIX)) return null;
  const [, code = 'unknown', encodedDetail = ''] = message.split('\t');
  const normalizedCode = code in SFTP_ERROR_MESSAGES ? (code as SftpErrorCode) : 'unknown';
  const detail = encodedDetail ? decodeURIComponent(encodedDetail) : '';
  return {
    code: normalizedCode,
    message: SFTP_ERROR_MESSAGES[normalizedCode],
    detail,
  };
}

// 兼容历史版本遗留的 OpenSSH stderr 文本：当前后端已改为进程内 russh 并统一返回稳定
// code，此分支只作为未识别错误的最后兜底，避免旧持久化状态或第三方文本直接透出英文。
function normalizeLegacySftpError(message: string): string {
  if (
    message.includes('AI 命令执行不能使用交互式密码认证') ||
    message.includes('SFTP 文件浏览暂不支持交互式密码认证')
  ) {
    return SFTP_ERROR_MESSAGES.passwordUnsupported;
  }

  if (message.includes('Permission denied')) return SFTP_ERROR_MESSAGES.authFailed;
  if (message.includes('Host key verification failed')) return SFTP_ERROR_MESSAGES.hostKeyFailed;
  if (message.includes('Connection timed out') || message.includes('Operation timed out')) {
    return SFTP_ERROR_MESSAGES.timeout;
  }
  if (message.includes('Connection refused')) return SFTP_ERROR_MESSAGES.connectionRefused;
  if (message.includes('No route to host')) return SFTP_ERROR_MESSAGES.hostUnreachable;
  if (message.includes('Could not resolve hostname')) return SFTP_ERROR_MESSAGES.hostResolveFailed;
  if (message.includes('No such file or directory')) return SFTP_ERROR_MESSAGES.pathMissing;

  return message;
}

export function getSftpErrorMessage(err: unknown): string {
  const message = getErrorText(err).trim();
  if (!message) return SFTP_ERROR_MESSAGES.unknown;

  // Tauri 错误同时携带命令级 code 和后端稳定分类标记，优先解析标记而不是泛化 code。
  const markerIndex = message.indexOf(SFTP_ERROR_PREFIX);
  if (markerIndex >= 0) {
    const parsed = parseSftpError(message.slice(markerIndex));
    if (parsed) return parsed.message;
  }

  const [code] = message.split('\t');
  const codeMessages: Record<string, SftpErrorCode> = {
    SFTP_PASSWORD_UNSUPPORTED: 'passwordUnsupported',
    SFTP_PASSWORD_REQUIRED: 'passwordRequired',
    SFTP_CREDENTIAL_FAILED: 'localCredentialMissing',
    CREDENTIAL_READ_FAILED: 'localCredentialMissing',
    CREDENTIAL_KIND_INVALID: 'localCredentialMissing',
    SFTP_INVALID_PATH: 'invalidPath',
    SFTP_INVALID_NAME: 'invalidPath',
    SFTP_REMOTE_READ_FAILED: 'unknown',
    SFTP_REMOTE_PROTOCOL_INVALID: 'unknown',
    SFTP_REMOTE_WRITE_FAILED: 'permissionDenied',
    SFTP_DISCONNECT_FAILED: 'unknown',
    SFTP_LOCAL_PATH_INVALID: 'pathMissing',
    SFTP_LOCAL_READ_FAILED: 'localReadFailed',
    SFTP_LOCAL_WRITE_FAILED: 'permissionDenied',
    SFTP_TRANSFER_FAILED: 'unknown',
  };
  if (codeMessages[code]) return SFTP_ERROR_MESSAGES[codeMessages[code]];

  const parsed = parseSftpError(message);
  if (parsed) return parsed.message;

  return normalizeLegacySftpError(message);
}

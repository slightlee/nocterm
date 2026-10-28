import { getTerminalErrorCode } from '../api/ssh-terminal-client';

/**
 * SSH 终端建连失败的前端归一化。
 *
 * 错误语义由后端单一来源维护：后端按建连阶段分类并返回稳定错误码
 * （`ErrorResponse.code`），`message` 是已归一化、带目标与排查方向的中文提示，
 * 属于 IPC 稳定契约，前端直接透传展示，不解析文本。本模块只负责：
 * 1. 登记稳定错误码词表，供 UI 分支判断与契约查阅；
 * 2. 在 message 缺失或非字符串时回退到本地兜底文案，保证用户始终能看到可读提示。
 */
export type SshTerminalErrorCode =
  | 'SSH_PASSWORD_REQUIRED'
  | 'SSH_AUTH_FAILED'
  | 'SSH_HOST_KEY_FAILED'
  | 'SSH_CONNECT_TIMEOUT'
  | 'SSH_CONNECTION_REFUSED'
  | 'SSH_HOST_UNREACHABLE'
  | 'SSH_HOST_RESOLVE_FAILED'
  | 'SSH_CREDENTIAL_MISSING'
  | 'SSH_PTY_DENIED'
  | 'SSH_TERMINAL_FAILED';

const SSH_TERMINAL_FALLBACK_MESSAGES: Record<SshTerminalErrorCode, string> = {
  SSH_PASSWORD_REQUIRED: '该连接尚未保存登录密码，请在终端中输入。',
  SSH_AUTH_FAILED: '认证失败：服务器拒绝登录，请检查用户名、密码、私钥或 SSH Agent 配置。',
  SSH_HOST_KEY_FAILED: '主机校验失败：远程主机密钥已变更，已拒绝连接。',
  SSH_CONNECT_TIMEOUT: '连接超时：请检查服务器地址、端口或网络连通性。',
  SSH_CONNECTION_REFUSED: '连接被拒绝：请确认 SSH 服务已启动且端口正确。',
  SSH_HOST_UNREACHABLE: '无法连接服务器：请检查网络、防火墙或服务器状态。',
  SSH_HOST_RESOLVE_FAILED: '主机解析失败：请检查服务器地址是否正确。',
  SSH_CREDENTIAL_MISSING: '登录凭据缺失：请编辑连接重新保存密码、绑定私钥或使用 SSH Agent。',
  SSH_PTY_DENIED: '远程主机拒绝分配 PTY：请确认该账号允许交互式登录。',
  SSH_TERMINAL_FAILED: 'SSH 终端操作失败，请检查连接配置或稍后重试。',
};

/** 未登记的错误码回退到通用兜底：契约扩展必须先登记词表，不能静默透传。 */
const GENERIC_FALLBACK_MESSAGE = SSH_TERMINAL_FALLBACK_MESSAGES.SSH_TERMINAL_FAILED;

function getRawErrorMessage(error: unknown): string {
  if (typeof error === 'object' && error !== null && 'message' in error) {
    const message = (error as { message?: unknown }).message;
    if (typeof message === 'string') return message;
  }
  return '';
}

/** 后端 message 正常路径直接透出；仅当缺失或非字符串时回退到本地词表。 */
export function getSshTerminalErrorMessage(error: unknown): string {
  const message = getRawErrorMessage(error);
  if (message) return message;

  const code = getTerminalErrorCode(error);
  if (code && code in SSH_TERMINAL_FALLBACK_MESSAGES) {
    return SSH_TERMINAL_FALLBACK_MESSAGES[code as SshTerminalErrorCode];
  }
  return GENERIC_FALLBACK_MESSAGE;
}

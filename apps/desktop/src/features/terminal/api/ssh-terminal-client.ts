import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

interface OpenTerminalResponse {
  terminalId: string;
  /** workspace 会话内的重连世代；后续 AI 可见执行用它校验目标一致性。 */
  terminalGeneration: number;
  /** bootstrap 成功时注入的 shell（bash/zsh/fish/PowerShell）；null 表示未注入。 */
  integrationShell: string | null;
  /** bootstrap 失败的降级原因；成功或未尝试时为 null。 */
  integrationFallbackReason: string | null;
}

export interface TerminalOutputEvent {
  terminalId: string;
  connectionId: number;
  data: string;
}

export interface TerminalExitEvent {
  terminalId: string;
  connectionId: number;
  /** 后端仅有这两种收尾：读到 EOF 为正常结束，读出错为失败。 */
  reason: 'closed' | 'failed';
}

const TERMINAL_OUTPUT_EVENT = 'ssh-terminal-output';
const TERMINAL_EXIT_EVENT = 'ssh-terminal-exit';

/** 后端要求交互输入登录密码时返回的稳定错误码。 */
export const PASSWORD_REQUIRED_CODE = 'SSH_PASSWORD_REQUIRED';

/** AI 命令执行期间用户输入被门控时返回的稳定错误码。 */
export const TERMINAL_BUSY_CODE = 'SSH_TERMINAL_BUSY';

/**
 * SSH IPC 参数包含连接主键、workspace 终端会话标识与终端尺寸，主机资料由后端读取。
 *
 * `password` 仅在后端返回 {@link PASSWORD_REQUIRED_CODE} 后由终端提示符收集并回传，
 * 用于本次建连；后端认证通过才会转入内存缓存，不会写入系统凭据库。
 */
export function openSshTerminal(
  connectionId: number,
  workspaceSessionId: string,
  cols: number,
  rows: number,
  password?: string
) {
  return invoke<OpenTerminalResponse>('ssh_terminal_open', {
    workspaceSessionId,
    connectionId,
    cols,
    rows,
    password,
  });
}

/** Tauri 拒绝时返回序列化后的 ErrorResponse，据此取出稳定错误码用于分支判断。 */
export function getTerminalErrorCode(error: unknown): string | undefined {
  if (typeof error !== 'object' || error === null) return undefined;
  const code = (error as { code?: unknown }).code;
  return typeof code === 'string' ? code : undefined;
}

export function writeSshTerminal(terminalId: string, data: string) {
  return invoke<void>('ssh_terminal_write', { terminalId, data });
}

export function resizeSshTerminal(terminalId: string, cols: number, rows: number) {
  return invoke<void>('ssh_terminal_resize', { terminalId, cols, rows });
}

export function closeSshTerminal(terminalId: string) {
  return invoke<void>('ssh_terminal_close', { terminalId });
}

export function onSshTerminalOutput(
  handler: (payload: TerminalOutputEvent) => void
): Promise<() => void> {
  return listen<TerminalOutputEvent>(TERMINAL_OUTPUT_EVENT, (event) => handler(event.payload));
}

export function onSshTerminalExit(
  handler: (payload: TerminalExitEvent) => void
): Promise<() => void> {
  return listen<TerminalExitEvent>(TERMINAL_EXIT_EVENT, (event) => handler(event.payload));
}

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

interface OpenTerminalResponse {
  terminalId: string;
}

export interface TerminalOutputEvent {
  terminalId: string;
  connectionId: number;
  data: string;
}

export interface TerminalExitEvent {
  terminalId: string;
  connectionId: number;
  reason: 'closed' | 'failed' | 'timed_out' | 'cancelled';
}

const TERMINAL_OUTPUT_EVENT = 'ssh-terminal-output';
const TERMINAL_EXIT_EVENT = 'ssh-terminal-exit';

/** SSH IPC 参数只包含连接主键和终端尺寸，主机资料由后端读取。 */
export function openSshTerminal(connectionId: number, cols: number, rows: number) {
  return invoke<OpenTerminalResponse>('ssh_terminal_open', { connectionId, cols, rows });
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

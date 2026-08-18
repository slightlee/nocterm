import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

interface OpenLocalTerminalResponse {
  terminalId: string;
}

export interface LocalTerminalOutputEvent {
  terminalId: string;
  sessionId: string;
  data: string;
}

export interface LocalTerminalExitEvent {
  terminalId: string;
  sessionId: string;
}

const LOCAL_TERMINAL_OUTPUT_EVENT = 'local-terminal-output';
const LOCAL_TERMINAL_EXIT_EVENT = 'local-terminal-exit';

export function openLocalTerminal(sessionId: string, cols: number, rows: number) {
  return invoke<OpenLocalTerminalResponse>('local_terminal_open', { sessionId, cols, rows });
}

export function writeLocalTerminal(terminalId: string, data: string) {
  return invoke<void>('local_terminal_write', { terminalId, data });
}

export function resizeLocalTerminal(terminalId: string, cols: number, rows: number) {
  return invoke<void>('local_terminal_resize', { terminalId, cols, rows });
}

export function closeLocalTerminal(terminalId: string) {
  return invoke<void>('local_terminal_close', { terminalId });
}

export function onLocalTerminalOutput(
  handler: (payload: LocalTerminalOutputEvent) => void
): Promise<() => void> {
  return listen<LocalTerminalOutputEvent>(LOCAL_TERMINAL_OUTPUT_EVENT, (event) =>
    handler(event.payload)
  );
}

export function onLocalTerminalExit(
  handler: (payload: LocalTerminalExitEvent) => void
): Promise<() => void> {
  return listen<LocalTerminalExitEvent>(LOCAL_TERMINAL_EXIT_EVENT, (event) =>
    handler(event.payload)
  );
}

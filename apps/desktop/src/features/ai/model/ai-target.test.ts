import { describe, expect, it } from 'vitest';

import {
  isAiTerminalTargetReady,
  resolveAiTerminalTarget,
  resolveAiWorkspaceTarget,
} from './ai-target';

describe('resolveAiTerminalTarget', () => {
  it('binds only a connected SSH session as a remote target', () => {
    const session = {
      id: 'remote:7:session-a',
      connectionId: 7,
      kind: 'remote' as const,
      name: 'Production',
      username: 'deploy',
      host: 'server.example',
    };

    const connecting = resolveAiTerminalTarget(session, 'connecting');
    expect(connecting).toEqual({ context: '' });
    expect(isAiTerminalTargetReady(connecting)).toBe(false);
    const target = resolveAiTerminalTarget(session, 'connected');
    expect(isAiTerminalTargetReady(target)).toBe(true);
    expect(target.connectionId).toBe(7);
    expect(target.connectionId).not.toBe(session.id);
    expect(target.targetSessionId).toBeUndefined();
    expect(target.context).toContain('不要假设它继承可见终端中的 cd');
    expect(target.context).not.toContain('ssh_exec');
    expect(target.context).toContain('不要提及 MCP、函数名或 Nocterm 内部工具名');
  });

  it('keeps two terminal tabs on the same remote connection bound to that connection', () => {
    const first = resolveAiTerminalTarget(
      {
        id: 'remote:7:session-a',
        connectionId: 7,
        kind: 'remote',
        name: 'Production',
        username: 'deploy',
        host: 'server.example',
      },
      'connected'
    );
    const second = resolveAiTerminalTarget(
      {
        id: 'remote:7:session-b',
        connectionId: 7,
        kind: 'remote',
        name: 'Production',
        username: 'deploy',
        host: 'server.example',
      },
      'connected'
    );

    expect(first.connectionId).toBe(7);
    expect(second.connectionId).toBe(7);
    expect(first.targetSessionId).toBeUndefined();
    expect(second.targetSessionId).toBeUndefined();
  });

  it('binds only a connected local session as a local target', () => {
    const session = { id: 'local:1', kind: 'local' as const, name: '本地终端' };

    const closed = resolveAiTerminalTarget(session, 'closed');
    expect(closed).toEqual({ context: '' });
    expect(isAiTerminalTargetReady(closed)).toBe(false);
    const target = resolveAiTerminalTarget(session, 'connected');
    expect(isAiTerminalTargetReady(target)).toBe(true);
    expect(target.connectionId).toBeUndefined();
    expect(target.targetSessionId).toBe('local:1');
    expect(target.context).not.toContain('local_terminal_exec');
    expect(target.context).toContain('必须先实际调用');
    expect(target.context).toContain('真实返回错误后才能报告不可用');
    expect(target.context).toContain('不要提及 MCP、函数名或 Nocterm 内部工具名');
  });
});

describe('resolveAiWorkspaceTarget', () => {
  it('keeps a connected terminal as the active AI target', () => {
    const target = resolveAiWorkspaceTarget(
      'terminal',
      { id: 'local:1', kind: 'local', name: '本地终端' },
      'connected'
    );

    expect(target.targetSessionId).toBe('local:1');
    expect(isAiTerminalTargetReady(target)).toBe(true);
  });

  it('does not reuse the last terminal while a file tab is active', () => {
    const target = resolveAiWorkspaceTarget(
      'sftp',
      {
        id: 'remote:7:session-a',
        connectionId: 7,
        kind: 'remote',
        name: 'Production',
        username: 'deploy',
        host: 'server.example',
      },
      'connected'
    );

    expect(target).toEqual({ context: '' });
    expect(isAiTerminalTargetReady(target)).toBe(false);
  });
});

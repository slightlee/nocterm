import { describe, expect, it } from 'vitest';

import { resolveAiTerminalTarget } from './ai-target';

describe('resolveAiTerminalTarget', () => {
  it('binds only a connected SSH session as a remote target', () => {
    const session = {
      id: 7,
      kind: 'remote' as const,
      name: 'Production',
      username: 'deploy',
      host: 'server.example',
    };

    expect(resolveAiTerminalTarget(session, 'connecting')).toEqual({ context: '' });
    const target = resolveAiTerminalTarget(session, 'connected');
    expect(target.connectionId).toBe(7);
    expect(target.targetSessionId).toBeUndefined();
    expect(target.context).toContain('不要假设它继承可见终端中的 cd');
    expect(target.context).not.toContain('ssh_exec');
    expect(target.context).toContain('不要提及 MCP、函数名或 Nocterm 内部工具名');
  });

  it('binds only a connected local session as a local target', () => {
    const session = { id: 'local:1', kind: 'local' as const, name: '本地终端' };

    expect(resolveAiTerminalTarget(session, 'closed')).toEqual({ context: '' });
    const target = resolveAiTerminalTarget(session, 'connected');
    expect(target.connectionId).toBeUndefined();
    expect(target.targetSessionId).toBe('local:1');
    expect(target.context).not.toContain('local_terminal_exec');
    expect(target.context).toContain('必须先实际调用');
    expect(target.context).toContain('真实返回错误后才能报告不可用');
    expect(target.context).toContain('不要提及 MCP、函数名或 Nocterm 内部工具名');
  });
});

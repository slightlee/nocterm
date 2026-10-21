import { describe, expect, it } from 'vitest';

import type { TerminalSession } from '../../terminal';
import { canBeginAiConversation, resolveAiTaskNavigation } from './ai-panel-state';
import type { AiRuntimeTask, AiRuntimeTasks } from './ai-runtime-task';

function remoteSession(id: string, connectionId: number): TerminalSession {
  return {
    id,
    connectionId,
    kind: 'remote',
    terminalNumber: 1,
    name: `服务器 ${connectionId}`,
    host: '127.0.0.1',
    port: 22,
    username: 'tester',
    authentication: 'ssh_agent',
    createdAt: 0,
    updatedAt: 0,
  };
}

function runtimeTask(
  target: AiRuntimeTask['target'],
  sourceTerminalSessionId: string
): AiRuntimeTask {
  return {
    sessionId: 'task-1',
    conversationId: 'conversation-a',
    target,
    sourceTerminalSessionId,
    provider: 'codex',
    startedAt: 1,
    streamText: '',
    runParts: [],
    pendingApproval: null,
    approvalSubmitting: false,
    notice: null,
  };
}

describe('canBeginAiConversation', () => {
  it('allows a new conversation when only another target is running', () => {
    const tasks: AiRuntimeTasks = {
      'task-1': {
        sessionId: 'task-1',
        conversationId: 'conversation-a',
        target: { kind: 'local', id: 'session-1', label: '本地终端' },
        sourceTerminalSessionId: 'session-1',
        provider: 'codex',
        startedAt: 1,
        streamText: '',
        runParts: [],
        pendingApproval: null,
        approvalSubmitting: false,
        notice: null,
      },
    };

    expect(canBeginAiConversation('ssh:2', tasks)).toBe(true);
    expect(canBeginAiConversation('local:session-1', tasks)).toBe(false);
    expect(canBeginAiConversation(null, tasks)).toBe(false);
  });
});

describe('resolveAiTaskNavigation', () => {
  it('returns to the exact source tab before considering SSH recency', () => {
    const source = remoteSession('remote:1:source', 1);
    const recent = remoteSession('remote:1:recent', 1);
    const destination = resolveAiTaskNavigation(
      runtimeTask({ kind: 'ssh', id: '1', label: '服务器 1' }, source.id),
      [source, recent],
      { '1': [recent.id, source.id] }
    );

    expect(destination).toEqual({
      conversationId: 'conversation-a',
      targetKey: 'ssh:1',
      terminalSessionId: source.id,
    });
  });

  it('falls back to the most recent existing tab for the same SSH connection', () => {
    const older = remoteSession('remote:1:older', 1);
    const recent = remoteSession('remote:1:recent', 1);
    const other = remoteSession('remote:2:recent', 2);
    const destination = resolveAiTaskNavigation(
      runtimeTask({ kind: 'ssh', id: '1', label: '服务器 1' }, 'remote:1:closed'),
      [older, recent, other],
      { '1': ['remote:1:closed', recent.id, older.id], '2': [other.id] }
    );

    expect(destination.terminalSessionId).toBe(recent.id);
  });

  it('does not substitute another local tab when the source tab is closed', () => {
    const destination = resolveAiTaskNavigation(
      runtimeTask({ kind: 'local', id: 'local:closed', label: '本地终端 1' }, 'local:closed'),
      [{ id: 'local:other', kind: 'local', name: '本地终端 2' }],
      {}
    );

    expect(destination).toEqual({
      conversationId: 'conversation-a',
      targetKey: 'local:local:closed',
      terminalSessionId: null,
    });
  });

  it('does not create or reconnect when no SSH terminal remains', () => {
    const destination = resolveAiTaskNavigation(
      runtimeTask({ kind: 'ssh', id: '7', label: '服务器 7' }, 'remote:7:closed'),
      [remoteSession('remote:8:open', 8)],
      { '7': ['remote:7:closed'] }
    );

    expect(destination.terminalSessionId).toBeNull();
    expect(destination.conversationId).toBe('conversation-a');
    expect(destination.targetKey).toBe('ssh:7');
  });
});

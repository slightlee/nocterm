import { describe, expect, it } from 'vitest';

import type { AiConversationTarget } from './ai-target';
import {
  canStartAiRuntimeTask,
  completeAiRuntimeTask,
  listBackgroundAiRuntimeTasks,
  listCompletedAiTaskResults,
  recordCompletedAiTaskResult,
  removeAiRuntimeTask,
  removeCompletedAiTaskResult,
  updateAiRuntimeTask,
  type AiRuntimeTask,
  type AiRuntimeTasks,
} from './ai-runtime-task';

const targetA: AiConversationTarget = { kind: 'ssh', id: '1', label: '服务器 A' };
const targetB: AiConversationTarget = { kind: 'ssh', id: '2', label: '服务器 B' };

function task(
  sessionId: string,
  conversationId: string,
  target: AiConversationTarget
): AiRuntimeTask {
  return {
    sessionId,
    conversationId,
    target,
    sourceTerminalSessionId: `terminal-${sessionId}`,
    provider: 'codex',
    startedAt: sessionId === 'session-a' ? 1 : 2,
    streamText: '',
    runParts: [],
    pendingApproval: null,
    approvalSubmitting: false,
    notice: null,
  };
}

const twoTasks = (): AiRuntimeTasks => ({
  'session-a': task('session-a', 'conversation-a', targetA),
  'session-b': task('session-b', 'conversation-b', targetB),
});

describe('AI runtime task isolation', () => {
  it('allows different targets to run concurrently but rejects a second task on one target', () => {
    const tasks = { 'session-a': task('session-a', 'conversation-a', targetA) };

    expect(canStartAiRuntimeTask(tasks, 'conversation-b', targetB)).toBe(true);
    expect(canStartAiRuntimeTask(tasks, 'conversation-a', targetB)).toBe(false);
    expect(canStartAiRuntimeTask(tasks, 'conversation-c', targetA)).toBe(false);
  });

  it('routes output and approval changes only to their session', () => {
    const tasks = twoTasks();
    const withOutput = updateAiRuntimeTask(tasks, 'session-a', { streamText: 'A 输出' });
    const approval = {
      approvalId: 'approval-a',
      sessionId: 'session-a',
      targetKind: 'ssh' as const,
      targetLabel: '服务器 A',
      command: 'uptime',
      expiresAtUnixMs: Date.now() + 60_000,
    };
    const withApproval = updateAiRuntimeTask(withOutput, 'session-a', {
      pendingApproval: approval,
    });

    expect(withApproval['session-a']?.streamText).toBe('A 输出');
    expect(withApproval['session-a']?.pendingApproval).toEqual(approval);
    expect(withApproval['session-b']).toEqual(tasks['session-b']);
  });

  it('removes or stops one task without changing the other task', () => {
    const tasks = twoTasks();
    const remaining = removeAiRuntimeTask(tasks, 'session-a');

    expect(remaining['session-a']).toBeUndefined();
    expect(remaining['session-b']).toEqual(tasks['session-b']);
  });

  it('lists only other-target tasks and preserves start order for the task menu', () => {
    const approval = {
      approvalId: 'approval-b',
      sessionId: 'session-b',
      targetKind: 'ssh' as const,
      targetLabel: '服务器 B',
      command: 'systemctl restart demo',
      expiresAtUnixMs: Date.now() + 60_000,
    };
    const tasks = updateAiRuntimeTask(twoTasks(), 'session-b', {
      pendingApproval: approval,
    });
    const background = listBackgroundAiRuntimeTasks(tasks, 'ssh:1');

    expect(background.map((item) => item.sessionId)).toEqual(['session-b']);
    expect(background.filter((item) => item.pendingApproval !== null)).toHaveLength(1);
  });

  it('keeps a current-target task visible while inspecting another conversation', () => {
    const tasks = twoTasks();

    expect(
      listBackgroundAiRuntimeTasks(tasks, 'ssh:1', 'conversation-b').map(
        (item) => item.conversationId
      )
    ).toEqual(['conversation-a']);
  });

  it('keeps completed results navigable without leaving them in the running registry', () => {
    const finished = completeAiRuntimeTask(
      task('session-a', 'conversation-a', targetA),
      'completed',
      10
    );
    const failed = completeAiRuntimeTask(
      task('session-b', 'conversation-b', targetB),
      'failed',
      20
    );
    const results = recordCompletedAiTaskResult(recordCompletedAiTaskResult({}, finished), failed);

    expect(listCompletedAiTaskResults(results).map((item) => item.conversationId)).toEqual([
      'conversation-b',
      'conversation-a',
    ]);
    expect(finished).toMatchObject({
      sourceTerminalSessionId: 'terminal-session-a',
      target: targetA,
      outcome: 'completed',
    });
    expect(twoTasks()).toHaveProperty('session-a');
  });

  it('replaces one conversation result and marks only the selected result as read', () => {
    const first = completeAiRuntimeTask(
      task('session-a', 'conversation-a', targetA),
      'completed',
      10
    );
    const latest = completeAiRuntimeTask(
      task('session-latest', 'conversation-a', targetA),
      'stopped',
      30
    );
    const other = completeAiRuntimeTask(
      task('session-b', 'conversation-b', targetB),
      'timed_out',
      20
    );
    const results = [first, other, latest].reduce(recordCompletedAiTaskResult, {});
    const remaining = removeCompletedAiTaskResult(results, 'conversation-a');

    expect(results['conversation-a']).toEqual(latest);
    expect(remaining['conversation-a']).toBeUndefined();
    expect(remaining['conversation-b']).toEqual(other);
  });
});

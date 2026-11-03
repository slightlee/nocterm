import { describe, expect, it } from 'vitest';

import {
  executionEventToPresentation,
  executionFallbackLabel,
  executionModeLabel,
  executionPhaseLabel,
  isExecutionHighRisk,
  isExecutionTerminal,
  reduceAiExecutionEvent,
} from './ai-execution-output';
import type { AiToolExecutionEvent } from './ai-types';

function executionEvent(overrides: Partial<AiToolExecutionEvent> = {}): AiToolExecutionEvent {
  return {
    executionId: 'exec-1',
    sequence: 0,
    providerSessionId: 'ai-session-1',
    conversationId: 'conversation-1',
    targetKind: 'ssh',
    workspaceSessionId: 'remote:7:test',
    terminalId: 'ssh-test',
    terminalGeneration: 1,
    connectionId: 7,
    tool: 'ssh_exec',
    phase: 'requested',
    approvalState: 'not_requested',
    executionMode: 'unselected',
    fallbackReason: null,
    resultState: 'pending',
    dispatchReceipt: 'not_started',
    exitCode: null,
    durationMs: null,
    errorCode: null,
    updatedAtMs: 100,
    ...overrides,
  };
}

describe('reduceAiExecutionEvent', () => {
  it('accepts the first event for a new execution', () => {
    const reduction = reduceAiExecutionEvent(executionEvent(), { executions: new Map() });
    expect(reduction.changed).toBe(true);
    expect(reduction.executions.size).toBe(1);
  });

  it('accepts a higher sequence event', () => {
    const first = reduceAiExecutionEvent(executionEvent(), { executions: new Map() });
    const second = reduceAiExecutionEvent(
      executionEvent({ sequence: 1, phase: 'dispatched', dispatchReceipt: 'confirmed' }),
      first
    );
    expect(second.changed).toBe(true);
    expect(second.executions.get('exec-1')?.phase).toBe('dispatched');
  });

  it('rejects a stale sequence event', () => {
    const first = reduceAiExecutionEvent(executionEvent({ sequence: 2, phase: 'completed' }), {
      executions: new Map(),
    });
    const stale = reduceAiExecutionEvent(executionEvent({ sequence: 1 }), first);
    expect(stale.changed).toBe(false);
  });

  it('rejects a duplicate sequence event', () => {
    const first = reduceAiExecutionEvent(executionEvent(), { executions: new Map() });
    const duplicate = reduceAiExecutionEvent(executionEvent(), first);
    expect(duplicate.changed).toBe(false);
  });

  it('keeps executions independent by executionId', () => {
    const first = reduceAiExecutionEvent(executionEvent(), { executions: new Map() });
    const second = reduceAiExecutionEvent(executionEvent({ executionId: 'exec-2' }), first);
    expect(second.changed).toBe(true);
    expect(second.executions.size).toBe(2);
  });
});

describe('executionPhaseLabel', () => {
  it('maps all phases to user-friendly labels', () => {
    expect(executionPhaseLabel('requested')).toBe('已接收');
    expect(executionPhaseLabel('awaiting_approval')).toBe('等待确认');
    expect(executionPhaseLabel('dispatched')).toBe('正在执行');
    expect(executionPhaseLabel('completed')).toBe('执行成功');
    expect(executionPhaseLabel('failed')).toBe('执行失败');
    expect(executionPhaseLabel('result_unknown')).toBe('结果未知');
  });
});

describe('executionModeLabel', () => {
  it('distinguishes visible terminal and independent exec modes', () => {
    expect(executionModeLabel('local_visible_osc')).toBe('当前终端 · Shell Integration');
    expect(executionModeLabel('ssh_visible_marker')).toBe(
      '当前终端 · Shell Integration + Marker 恢复'
    );
    expect(executionModeLabel('ssh_independent_exec')).toBe('隔离执行 · SSH exec channel');
    expect(executionModeLabel('unselected')).toBe('执行模式待定');
  });
});

describe('executionFallbackLabel', () => {
  it('distinguishes unsupported shells from integration and prompt failures', () => {
    expect(executionFallbackLabel('SSH_VISIBLE_NOT_ENABLED')).toContain('已关闭终端可见');
    expect(executionFallbackLabel('SSH_SHELL_UNSUPPORTED')).toContain('Shell 不支持');
    expect(executionFallbackLabel('SSH_INTEGRATION_UNAVAILABLE')).toContain('初始化失败');
    expect(executionFallbackLabel('SSH_CONTROL_PLANE_UNAVAILABLE')).toContain('控制面不可用');
    expect(executionFallbackLabel('SSH_PROMPT_NOT_READY')).toContain('提示符未就绪');
    // 从未观察到可信完成事件与"就绪后被用户输入失效"必须分开提示，
    // 前者指向远端钩子未生效（重连修复），后者等待下一次提示符事件自愈。
    expect(executionFallbackLabel('SSH_INTEGRATION_NOT_CONFIRMED')).toContain(
      'Shell Integration 生效'
    );
  });
});

describe('isExecutionHighRisk', () => {
  it('marks result_unknown, failed and timed_out as high risk', () => {
    expect(isExecutionHighRisk(executionEvent({ phase: 'result_unknown' }))).toBe(true);
    expect(isExecutionHighRisk(executionEvent({ phase: 'failed' }))).toBe(true);
    expect(isExecutionHighRisk(executionEvent({ phase: 'timed_out' }))).toBe(true);
    expect(isExecutionHighRisk(executionEvent({ phase: 'completed' }))).toBe(false);
    expect(isExecutionHighRisk(executionEvent({ phase: 'requested' }))).toBe(false);
  });
});

describe('isExecutionTerminal', () => {
  it('identifies all terminal phases', () => {
    expect(isExecutionTerminal(executionEvent({ phase: 'completed' }))).toBe(true);
    expect(isExecutionTerminal(executionEvent({ phase: 'failed' }))).toBe(true);
    expect(isExecutionTerminal(executionEvent({ phase: 'denied' }))).toBe(true);
    expect(isExecutionTerminal(executionEvent({ phase: 'cancelled' }))).toBe(true);
    expect(isExecutionTerminal(executionEvent({ phase: 'timed_out' }))).toBe(true);
    expect(isExecutionTerminal(executionEvent({ phase: 'result_unknown' }))).toBe(true);
    expect(isExecutionTerminal(executionEvent({ phase: 'dispatched' }))).toBe(false);
  });
});

describe('executionEventToPresentation', () => {
  it('projects only safe display fields without command or output', () => {
    const presentation = executionEventToPresentation(executionEvent());
    expect(presentation.executionId).toBe('exec-1');
    expect(presentation.tool).toBe('ssh_exec');
    expect(presentation.targetKind).toBe('ssh');
    // 展示模型不包含命令或输出字段。
    expect(presentation).not.toHaveProperty('command');
    expect(presentation).not.toHaveProperty('output');
  });
});

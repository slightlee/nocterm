import { describe, expect, it } from 'vitest';

import type { AiConversation } from './ai-store';
import type { TerminalSession } from '../../terminal';
import {
  resolveAiContextBar,
  resolveFallbackQuestion,
  resolveTargetUnavailableMessage,
  resolveVisibleNotice,
} from './ai-panel-view-model';

function localSession(overrides: Partial<TerminalSession> = {}): TerminalSession {
  return {
    id: 'local-1',
    kind: 'local',
    name: '本地终端',
    terminalNumber: 1,
    createdAt: 0,
    updatedAt: 0,
    ...overrides,
  } as TerminalSession;
}

function conversation(overrides: Partial<AiConversation> = {}): AiConversation {
  return {
    id: 'conv-1',
    title: '会话',
    provider: 'codex',
    commandPolicy: 'auto_safe',
    target: null,
    messages: [],
    createdAt: 0,
    updatedAt: 0,
    ...overrides,
  };
}

describe('ai panel context bar', () => {
  it('reports target mismatch while inspecting a foreign conversation', () => {
    const foreign = conversation({
      target: { kind: 'ssh', id: 'conn-3', label: '服务器 3' },
    });
    const result = resolveAiContextBar({
      activeSession: localSession(),
      activeSessionStatus: 'connected',
      activeView: 'terminal',
      activeConversation: foreign,
      viewingCurrentTarget: false,
    });
    expect(result.status).toBe('mismatch');
    expect(result.text).toContain('服务器 3');
  });

  it('marks unbound legacy history as view-only', () => {
    const result = resolveAiContextBar({
      activeSession: localSession(),
      activeSessionStatus: 'connected',
      activeView: 'terminal',
      activeConversation: conversation(),
      viewingCurrentTarget: false,
    });
    expect(result.status).toBe('mismatch');
    expect(result.text).toBe('正在查看未绑定历史 · 仅可查看');
  });

  it('reports sftp views without execution context', () => {
    const result = resolveAiContextBar({
      activeSession: localSession(),
      activeSessionStatus: 'connected',
      activeView: 'sftp',
      activeConversation: null,
      viewingCurrentTarget: false,
    });
    expect(result.status).toBe('sftp');
    expect(result.text).toContain('暂未接入');
  });

  it('announces the connected session for terminal views', () => {
    const result = resolveAiContextBar({
      activeSession: localSession({ name: '本地 A' }),
      activeSessionStatus: 'connected',
      activeView: 'terminal',
      activeConversation: null,
      viewingCurrentTarget: true,
    });
    expect(result.status).toBe('connected');
    expect(result.text).toBe('已连接 本地 A，可分析命令、日志与报错');
  });

  it('separates connecting, error and idle copy', () => {
    const base = {
      activeView: 'terminal' as const,
      activeConversation: null,
      viewingCurrentTarget: true,
    };
    expect(
      resolveAiContextBar({ ...base, activeSession: null, activeSessionStatus: 'connecting' }).text
    ).toContain('连接完成后');
    expect(
      resolveAiContextBar({
        ...base,
        activeSession: localSession(),
        activeSessionStatus: 'error',
      }).text
    ).toContain('恢复连接');
    expect(
      resolveAiContextBar({ ...base, activeSession: null, activeSessionStatus: 'idle' }).text
    ).toContain('连接终端后');
  });
});

describe('ai panel target unavailable message', () => {
  it('points back to the bound terminal of a foreign conversation', () => {
    const foreign = conversation({
      target: { kind: 'ssh', id: 'conn-5', label: '服务器 5' },
    });
    expect(
      resolveTargetUnavailableMessage({
        activeConversation: foreign,
        activeSessionStatus: 'connected',
        activeView: 'terminal',
        hasBinding: true,
        viewingCurrentTarget: false,
      })
    ).toBe('该会话绑定 服务器 5，请切回对应终端');
  });

  it('walks connection states before the generic fallback', () => {
    const base = {
      activeConversation: null,
      activeView: 'terminal' as const,
      viewingCurrentTarget: true,
    };
    expect(
      resolveTargetUnavailableMessage({
        ...base,
        activeSessionStatus: 'connecting',
        hasBinding: true,
      })
    ).toContain('连接完成');
    expect(
      resolveTargetUnavailableMessage({ ...base, activeSessionStatus: 'error', hasBinding: true })
    ).toContain('恢复终端连接');
    expect(
      resolveTargetUnavailableMessage({ ...base, activeSessionStatus: 'idle', hasBinding: false })
    ).toContain('请先连接');
    expect(
      resolveTargetUnavailableMessage({ ...base, activeSessionStatus: 'idle', hasBinding: true })
    ).toBe('当前终端不可用');
  });
});

describe('ai panel visible notice priority', () => {
  it('prefers the running task notice over everything else', () => {
    expect(
      resolveVisibleNotice({
        activeConversationId: 'c1',
        activeTaskNotice: '任务运行中',
        activePresentationNotice: '展示层',
        globalNotice: '全局',
        panelNotice: '面板',
        taskNavigationNotice: { conversationId: 'c1', message: '导航' },
      })
    ).toBe('任务运行中');
  });

  it('shows task navigation copy only for its own conversation', () => {
    const base = {
      activeTaskNotice: null,
      activePresentationNotice: null,
      globalNotice: null,
      panelNotice: null,
    };
    expect(
      resolveVisibleNotice({
        ...base,
        activeConversationId: 'c2',
        taskNavigationNotice: { conversationId: 'c1', message: '导航' },
      })
    ).toBeNull();
    expect(
      resolveVisibleNotice({
        ...base,
        activeConversationId: 'c1',
        taskNavigationNotice: { conversationId: 'c1', message: '导航' },
      })
    ).toBe('导航');
  });

  it('falls back to global and panel notices in order', () => {
    const base = {
      activeConversationId: 'c1',
      activeTaskNotice: null,
      activePresentationNotice: null,
      taskNavigationNotice: null,
    };
    expect(resolveVisibleNotice({ ...base, globalNotice: '全局', panelNotice: '面板' })).toBe(
      '全局'
    );
    expect(resolveVisibleNotice({ ...base, globalNotice: null, panelNotice: '面板' })).toBe('面板');
  });
});

describe('ai panel fallback question', () => {
  it('combines file and terminal selection attachments', () => {
    expect(resolveFallbackQuestion({ hasTerminalSelection: true, fileName: 'log.txt' })).toContain(
      '文件和终端选区'
    );
  });

  it('mentions terminal output when only a selection exists', () => {
    expect(resolveFallbackQuestion({ hasTerminalSelection: true, fileName: null })).toContain(
      '终端输出'
    );
  });

  it('names the file attachment otherwise', () => {
    expect(resolveFallbackQuestion({ hasTerminalSelection: false, fileName: 'log.txt' })).toBe(
      '请分析附件：log.txt'
    );
    expect(resolveFallbackQuestion({ hasTerminalSelection: false, fileName: null })).toBe(
      '请分析附件：未命名文件'
    );
  });
});

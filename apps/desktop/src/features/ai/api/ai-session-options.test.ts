import { describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ invoke: vi.fn().mockResolvedValue({ sessionId: 'ai-test' }) }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
import { startAiSession } from './ai-client';

describe('模型调用选项 IPC', () => {
  it('显式档位与模型 ID 一起发送，恢复默认不复用上次档位', async () => {
    await startAiSession(
      'ai-test',
      'conversation',
      'codex',
      'full',
      'current',
      undefined,
      undefined,
      'local:test',
      'auto_safe',
      'test-model',
      'high'
    );
    expect(mocks.invoke).toHaveBeenLastCalledWith('ai_session_start', {
      request: expect.objectContaining({ model: 'test-model', reasoningEffort: 'high' }),
    });
    await startAiSession(
      'ai-test',
      'conversation',
      'codex',
      'full',
      'current',
      undefined,
      undefined,
      'local:test',
      'auto_safe',
      'test-model'
    );
    expect(mocks.invoke).toHaveBeenLastCalledWith('ai_session_start', {
      request: expect.objectContaining({ model: 'test-model', reasoningEffort: undefined }),
    });
  });
});

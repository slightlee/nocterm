import { describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ invoke: vi.fn().mockResolvedValue({ sessionId: 'ai-test' }) }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
import { startAiSession } from './ai-client';

describe('模型调用选项 IPC', () => {
  it('显式档位与模型 ID 一起发送，恢复默认不复用上次档位', async () => {
    await startAiSession({
      clientSessionId: 'ai-test',
      conversationId: 'conversation',
      provider: 'codex',
      prompt: 'full',
      continuationPrompt: 'current',
      targetSessionId: 'local:test',
      commandPolicy: 'auto_safe',
      model: 'test-model',
      reasoningEffort: 'high',
      fastMode: true,
    });
    expect(mocks.invoke).toHaveBeenLastCalledWith('ai_session_start', {
      request: expect.objectContaining({
        model: 'test-model',
        reasoningEffort: 'high',
        fastMode: true,
      }),
    });
    await startAiSession({
      clientSessionId: 'ai-test',
      conversationId: 'conversation',
      provider: 'codex',
      prompt: 'full',
      continuationPrompt: 'current',
      targetSessionId: 'local:test',
      model: 'test-model',
    });
    expect(mocks.invoke).toHaveBeenLastCalledWith('ai_session_start', {
      request: expect.objectContaining({
        model: 'test-model',
        reasoningEffort: undefined,
        fastMode: undefined,
      }),
    });
  });
});

import { describe, expect, it } from 'vitest';

import { getSshTerminalErrorMessage } from './terminal-errors';

describe('ssh terminal error message', () => {
  it('passes the backend message through unchanged', () => {
    // 后端 message 属于稳定契约（带目标与排查方向的中文提示），前端不解析、不改写。
    const error = {
      code: 'SSH_CONNECT_TIMEOUT',
      message: '连接 10.0.0.5:22 超时：15 秒内没能建立 TCP 连接。',
      retryable: true,
    };
    expect(getSshTerminalErrorMessage(error)).toBe(error.message);
  });

  it('falls back to the local wording for the error code when the message is missing', () => {
    const error = { code: 'SSH_AUTH_FAILED', retryable: true };
    expect(getSshTerminalErrorMessage(error)).toContain('认证失败');
  });

  it('falls back to the generic wording for unregistered codes', () => {
    // 未登记的错误码不得静默透传：契约扩展必须先登记词表。
    expect(getSshTerminalErrorMessage({ code: 'SSH_FUTURE_CODE' })).toContain('SSH 终端操作失败');
  });

  it('guarantees a readable prompt even without code or message', () => {
    expect(getSshTerminalErrorMessage(undefined)).toContain('SSH 终端操作失败');
    expect(getSshTerminalErrorMessage({ message: 42 })).toContain('SSH 终端操作失败');
  });
});

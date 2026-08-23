import { describe, expect, it } from 'vitest';

import {
  EMPTY_PASSWORD_PROMPT,
  type PasswordPromptState,
  reducePasswordPrompt,
} from './password-prompt';

function feed(chunks: string[]): PasswordPromptState {
  return chunks.reduce(reducePasswordPrompt, EMPTY_PASSWORD_PROMPT);
}

describe('password prompt', () => {
  it('accumulates printable characters until enter submits', () => {
    expect(feed(['s3', 'cr', 'et', '\r'])).toEqual({ value: 's3cret', status: 'submitted' });
  });

  it('applies backspace and ctrl+u before submitting', () => {
    expect(feed(['abc\x7f', 'd', '\r'])).toEqual({ value: 'abd', status: 'submitted' });
    expect(feed(['wrong', '\x15', 'right', '\r'])).toEqual({ value: 'right', status: 'submitted' });
  });

  it('discards the buffer when the user cancels', () => {
    // 放弃输入时必须清空缓冲，不能把已键入的明文留在状态里。
    expect(feed(['secr', '\x03'])).toEqual({ value: '', status: 'cancelled' });
    expect(feed(['secr', '\x04'])).toEqual({ value: '', status: 'cancelled' });
  });

  it('ignores escape sequences instead of treating them as characters', () => {
    // 方向键、Home 等按键的 CSI 序列必须整段跳过，否则会污染口令。
    expect(feed(['a', '\x1b[A', '\x1b[1;5D', 'b', '\r'])).toEqual({
      value: 'ab',
      status: 'submitted',
    });
    expect(feed(['a', '\x1bOP', 'b', '\r'])).toEqual({ value: 'ab', status: 'submitted' });
  });

  it('ignores unmapped control characters', () => {
    expect(feed(['a\tb\x01c', '\r'])).toEqual({ value: 'abc', status: 'submitted' });
  });

  it('keeps a finished state untouched so late keystrokes cannot mutate it', () => {
    const submitted = feed(['ok', '\r']);
    expect(reducePasswordPrompt(submitted, 'more')).toBe(submitted);
  });

  it('caps the buffer so a large paste cannot grow without bound', () => {
    const result = feed(['x'.repeat(2000), '\r']);
    expect(result.status).toBe('submitted');
    expect(result.value).toHaveLength(1024);
  });

  it('accepts a pasted chunk exactly like typed characters', () => {
    // 粘贴在 xterm 里同样以一段 onData 到达，因此不需要单独的分支。
    expect(feed(['p@ss w0rd', '\r'])).toEqual({ value: 'p@ss w0rd', status: 'submitted' });
  });

  it('submits at the first newline of a multi-line paste', () => {
    // 从文本或密码管理器复制常常带尾随换行，此时应直接提交而不是把换行并入口令。
    expect(feed(['secret\nrest'])).toEqual({ value: 'secret', status: 'submitted' });
  });

  it('keeps bracketed paste markers out of the buffer', () => {
    // 终端开启 bracketed paste 后，粘贴内容会被 \x1b[200~ / \x1b[201~ 包裹，
    // 这两段 CSI 序列必须整段跳过，否则口令里会混进 "200~"。
    expect(feed(['\x1b[200~pa$$\x1b[201~', '\r'])).toEqual({
      value: 'pa$$',
      status: 'submitted',
    });
  });
});

import { describe, expect, it } from 'vitest';

import { REMOTE_COLORIZE_COMMAND } from './remote-colorize';

describe('remote colorize command', () => {
  it('enables colors for ls and grep family', () => {
    expect(REMOTE_COLORIZE_COMMAND).toContain("alias ls='ls --color=auto'");
    expect(REMOTE_COLORIZE_COMMAND).toContain("alias grep='grep --color=auto'");
    expect(REMOTE_COLORIZE_COMMAND).toContain("alias egrep='egrep --color=auto'");
    expect(REMOTE_COLORIZE_COMMAND).toContain("alias fgrep='fgrep --color=auto'");
  });

  it('sets a colored prompt with cursor-safe escapes', () => {
    expect(REMOTE_COLORIZE_COMMAND).toContain('export PS1=');
    // \[ \] 包裹颜色序列，避免 readline 计算提示符宽度错位。
    expect(REMOTE_COLORIZE_COMMAND).toContain('\\[\\e[1;32m\\]');
    expect(REMOTE_COLORIZE_COMMAND).toContain('\\[\\e[0m\\]');
  });

  it('stays a single line so remote line editing is not confused', () => {
    expect(REMOTE_COLORIZE_COMMAND).not.toContain('\n');
    expect(REMOTE_COLORIZE_COMMAND).not.toContain('\r');
    expect(REMOTE_COLORIZE_COMMAND.endsWith('\n')).toBe(false);
  });

  it('does not write any remote file from the command itself', () => {
    expect(REMOTE_COLORIZE_COMMAND).not.toContain('>>');
    expect(REMOTE_COLORIZE_COMMAND).not.toContain('>');
  });
});

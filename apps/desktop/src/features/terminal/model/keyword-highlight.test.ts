import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { createKeywordHighlighter } from './keyword-highlight';

describe('keyword highlight', () => {
  it('colors error keywords without touching surrounding text', () => {
    const { transform } = createKeywordHighlighter();
    expect(transform('ERROR: connection refused\n')).toBe(
      '\x1b[1;91mERROR\x1b[0m: connection \x1b[1;91mrefused\x1b[0m\n'
    );
  });

  it('colors warnings, success, info and IPs with distinct codes', () => {
    const { transform } = createKeywordHighlighter();
    expect(transform('WARNING: low disk\n')).toBe('\x1b[1;93mWARNING\x1b[0m: low disk\n');
    const ok = createKeywordHighlighter();
    expect(ok.transform('connected to 10.0.0.1\n')).toBe(
      '\x1b[1;92mconnected\x1b[0m to \x1b[1;96m10.0.0.1\x1b[0m\n'
    );
    const info = createKeywordHighlighter();
    expect(info.transform('[INFO] starting app\n')).toBe(
      '[\x1b[1;94mINFO\x1b[0m] \x1b[1;94mstarting\x1b[0m app\n'
    );
  });

  it('leaves already colored segments untouched', () => {
    const { transform } = createKeywordHighlighter();
    // 服务器已着色的行（如彩色 ls）：前景色激活期间不做匹配。
    expect(transform('\x1b[01;34mtarget\x1b[0m plain\n')).toBe('\x1b[01;34mtarget\x1b[0m plain\n');
  });

  it('never modifies escape sequences or OSC payloads', () => {
    const { transform } = createKeywordHighlighter();
    // OSC 标题里含关键词：必须原样透传。
    expect(transform('\x1b]0;error window\x07tail\n')).toBe('\x1b]0;error window\x07tail\n');
    // CSI 光标移动与高亮词同行：序列字节不动。
    expect(transform('\x1b[2Kerror\n')).toBe('\x1b[2K\x1b[1;91merror\x1b[0m\n');
  });

  it('buffers truncated escape sequences across chunks', () => {
    const { transform } = createKeywordHighlighter();
    const first = transform('\x1b[2');
    expect(first).toBe('');
    const second = transform('Kerror\n');
    expect(second).toBe('\x1b[2K\x1b[1;91merror\x1b[0m\n');
  });

  it('buffers lone escape at chunk end', () => {
    const { transform } = createKeywordHighlighter();
    expect(transform('ok\x1b')).toBe('\x1b[1;92mok\x1b[0m');
    expect(transform('[2K\n')).toBe('\x1b[2K\n');
  });

  it('restores active bold and colors after a match', () => {
    const h = createKeywordHighlighter();
    expect(h.transform('\x1b[1;31mERROR stays\x1b[0m plain error\n')).toBe(
      '\x1b[1;31mERROR stays\x1b[0m plain \x1b[1;91merror\x1b[0m\n'
    );
  });

  it('keeps highlight theme-neutral by re-emitting stored state', () => {
    const { transform } = createKeywordHighlighter();
    // 256 色前景激活时不匹配；序列结束恢复默认后可匹配。
    expect(transform('\x1b[38;5;123mERROR\x1b[0m ok\n')).toBe(
      '\x1b[38;5;123mERROR\x1b[0m \x1b[1;92mok\x1b[0m\n'
    );
  });

  it('prioritizes error over success when both words appear', () => {
    const { transform } = createKeywordHighlighter();
    const out = transform('error then ok\n');
    expect(out).toContain('\x1b[1;91merror\x1b[0m');
    expect(out).toContain('\x1b[1;92mok\x1b[0m');
  });
});

describe('file type coloring (client-side, no server LS_COLORS needed)', () => {
  it('colors file names by extension with distinct base colors', () => {
    const { transform } = createKeywordHighlighter();
    expect(transform('tail -f app.log\n')).toBe('tail -f \x1b[33mapp.log\x1b[0m\n');
    // 文件名规则优先于关键词规则：error.log 是完整一个 log 文件，不被 error 拆开。
    expect(transform('access.log  error.log\n')).toBe(
      '\x1b[33maccess.log\x1b[0m  \x1b[33merror.log\x1b[0m\n'
    );
    expect(createKeywordHighlighter().transform('backup.tar.gz data.zip\n')).toBe(
      '\x1b[1;31mbackup.tar.gz\x1b[0m \x1b[1;31mdata.zip\x1b[0m\n'
    );
    expect(createKeywordHighlighter().transform('nginx.conf config.yaml\n')).toBe(
      '\x1b[36mnginx.conf\x1b[0m \x1b[36mconfig.yaml\x1b[0m\n'
    );
    expect(createKeywordHighlighter().transform('deploy.sh main.py\n')).toBe(
      '\x1b[1;32mdeploy.sh\x1b[0m \x1b[1;32mmain.py\x1b[0m\n'
    );
    expect(createKeywordHighlighter().transform('photo.jpg video.mp4\n')).toBe(
      '\x1b[35mphoto.jpg\x1b[0m \x1b[35mvideo.mp4\x1b[0m\n'
    );
  });

  it('leaves plain words and already colored text untouched', () => {
    const { transform } = createKeywordHighlighter();
    expect(transform('hello world\n')).toBe('hello world\n');
    expect(createKeywordHighlighter().transform('\x1b[34mapp.log\x1b[0m\n')).toBe(
      '\x1b[34mapp.log\x1b[0m\n'
    );
    expect(createKeywordHighlighter().transform('readme.txt notes.md\n')).toBe(
      'readme.txt notes.md\n'
    );
  });
});

describe('prompt and ls -l metadata coloring (zero-injection, MobaXterm-style)', () => {
  it('colors user@host and path segments of the prompt with distinct colors', () => {
    const { transform } = createKeywordHighlighter();
    expect(transform('root@iZ2zedqu:~# ls\n')).toBe(
      '\x1b[1;92mroot@iZ2zedqu\x1b[0m\x1b[1;94m:~\x1b[0m# ls\n'
    );
    expect(createKeywordHighlighter().transform('deploy@web:/var/www/html$ \n')).toBe(
      '\x1b[1;92mdeploy@web\x1b[0m\x1b[1;94m:/var/www/html\x1b[0m$ \n'
    );
    // 无路径段的裸 user@host 保持整串洋红。
    expect(createKeywordHighlighter().transform('root@web # comment\n')).toBe(
      '\x1b[1;92mroot@web\x1b[0m # comment\n'
    );
  });

  it('colors ls -l permission blocks and modification dates', () => {
    const { transform } = createKeywordHighlighter();
    const line = '-rw-r--r-- 1 root root 4096 Apr 22 2024 app.log\n';
    expect(transform(line)).toBe(
      '\x1b[36m-rw-r--r--\x1b[0m 1 root root 4096 \x1b[93mApr 22 2024\x1b[0m \x1b[33mapp.log\x1b[0m\n'
    );
    const recent = createKeywordHighlighter().transform(
      'drwxr-xr-x 2 root root 4096 Apr 22 09:15 bin\n'
    );
    expect(recent).toBe(
      '\x1b[36mdrwxr-xr-x\x1b[0m 2 root root 4096 \x1b[93mApr 22 09:15\x1b[0m \x1b[1;34mbin\x1b[0m\n'
    );
  });
});

describe('ls -l file name type coloring (MobaXterm semantics)', () => {
  it('colors directories bold blue and executables bold green', () => {
    const { transform } = createKeywordHighlighter();
    const out = transform(
      'drwxr-xr-x 2 root root 4096 Apr 22 09:15 bin\n-rwxr-xr-x 1 root root 4096 Apr 22 2024 run.sh\n'
    );
    expect(out).toBe(
      '\x1b[36mdrwxr-xr-x\x1b[0m 2 root root 4096 \x1b[93mApr 22 09:15\x1b[0m \x1b[1;34mbin\x1b[0m\n' +
        '\x1b[36m-rwxr-xr-x\x1b[0m 1 root root 4096 \x1b[93mApr 22 2024\x1b[0m \x1b[1;32mrun.sh\x1b[0m\n'
    );
  });

  it('colors symlinks bold magenta and keeps the arrow target plain', () => {
    const { transform } = createKeywordHighlighter();
    expect(transform('lrwxrwxrwx 1 root root 7 Apr 22 09:15 link -> /etc/hosts\n')).toBe(
      '\x1b[36mlrwxrwxrwx\x1b[0m 1 root root 7 \x1b[93mApr 22 09:15\x1b[0m \x1b[1;35mlink\x1b[0m -> /etc/hosts\n'
    );
  });

  it('leaves regular non-executable files to the extension rules', () => {
    const { transform } = createKeywordHighlighter();
    // 无扩展名普通文件不上类型色；有扩展名时扩展名规则仍然生效。
    expect(transform('-rw-r--r-- 1 root root 4096 Apr 22 2024 README\n')).toBe(
      '\x1b[36m-rw-r--r--\x1b[0m 1 root root 4096 \x1b[93mApr 22 2024\x1b[0m README\n'
    );
    expect(
      createKeywordHighlighter().transform('-rw-r--r-- 1 root root 4096 Apr 22 2024 app.log\n')
    ).toBe(
      '\x1b[36m-rw-r--r--\x1b[0m 1 root root 4096 \x1b[93mApr 22 2024\x1b[0m \x1b[33mapp.log\x1b[0m\n'
    );
  });

  it('prefers the type color over the extension color for directories', () => {
    const { transform } = createKeywordHighlighter();
    // 目录名为 backup.tar.gz：应呈现目录蓝而不是压缩包红。
    expect(transform('drwxr-xr-x 2 root root 4096 Apr 22 09:15 backup.tar.gz\n')).toBe(
      '\x1b[36mdrwxr-xr-x\x1b[0m 2 root root 4096 \x1b[93mApr 22 09:15\x1b[0m \x1b[1;34mbackup.tar.gz\x1b[0m\n'
    );
  });

  it('does not treat ordinary sentences as ls -l output', () => {
    const { transform } = createKeywordHighlighter();
    // `total` 行与普通句子：不应被误当成文件类型行。
    expect(transform('total 24\n')).toBe('total 24\n');
  });
});

describe('chunk boundary robustness (SSH output arrives in arbitrary byte chunks)', () => {
  /** 按 size 字节逐块喂入，模拟 PTY/IPC 的任意分块。 */
  const chunked = (text: string, size: number): string => {
    const h = createKeywordHighlighter();
    let out = '';
    for (let i = 0; i < text.length; i += size) out += h.transform(text.slice(i, i + size));
    return out;
  };

  it('colors file names split across chunk boundaries', () => {
    const text =
      '\x1b[01;34mconf\x1b[0m \x1b[01;34mconf.d\x1b[0m docker-compose-nginx.yml html\r\n';
    for (const size of [1, 2, 3, 5, 7, 11, 64]) {
      expect(chunked(text, size)).toBe(createKeywordHighlighter().transform(text));
    }
  });

  it('colors keywords and prompts split mid-token', () => {
    expect(chunked('access.log  error.log\r\n', 3)).toBe(
      '\x1b[33maccess.log\x1b[0m  \x1b[33merror.log\x1b[0m\r\n'
    );
    expect(chunked('root@web:~# ls\n', 2)).toBe(
      '\x1b[1;92mroot@web\x1b[0m\x1b[1;94m:~\x1b[0m# ls\n'
    );
  });

  it('keeps server colors byte-exact regardless of chunking', () => {
    const text = '\x1b[01;34mbin\x1b[0m  \x1b[01;34metc\x1b[0m  swapfile\r\n';
    for (const size of [1, 3, 8]) {
      expect(chunked(text, size)).toBe(createKeywordHighlighter().transform(text));
    }
  });

  it('flushes long token-less runs instead of buffering forever', () => {
    const long = 'a'.repeat(300);
    expect(chunked(`${long}\n`, 7)).toBe(`${long}\n`);
  });

  it('flushes immediately when the chunk ends on a non-token char', () => {
    const h = createKeywordHighlighter();
    // 提示符以 `# ` 结尾：不能被缓存，必须当场着色。
    expect(h.transform('root@web:~# ')).toBe('\x1b[1;92mroot@web\x1b[0m\x1b[1;94m:~\x1b[0m# ');
  });
});

describe('deferred flush (interactive typing must never be held)', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('delivers held keystroke echoes within deferMs', () => {
    const deferred: string[] = [];
    const h = createKeywordHighlighter({ onDeferred: (t) => deferred.push(t) });
    // 每个按键一个回显块：当场不显示（等待拼全），deferMs 内自动送达。
    expect(h.transform('l')).toBe('');
    vi.advanceTimersByTime(16);
    expect(deferred).toEqual(['l']);
    expect(h.transform('s')).toBe('');
    vi.advanceTimersByTime(16);
    expect(deferred).toEqual(['l', 's']);
    h.dispose();
  });

  it('merges fast chunks before the timer fires (bulk output still matched)', () => {
    const deferred: string[] = [];
    const h = createKeywordHighlighter({ onDeferred: (t) => deferred.push(t) });
    expect(h.transform('access')).toBe('');
    // 定时器触发前下一块到达：拼全后正常匹配，不经延迟通道。
    expect(h.transform('.log\r\n')).toBe('\x1b[33maccess.log\x1b[0m\r\n');
    vi.advanceTimersByTime(32);
    expect(deferred).toEqual([]);
    h.dispose();
  });

  it('never defers an incomplete escape sequence', () => {
    const deferred: string[] = [];
    const h = createKeywordHighlighter({ onDeferred: (t) => deferred.push(t) });
    expect(h.transform('\x1b[2')).toBe('');
    vi.advanceTimersByTime(64);
    expect(deferred).toEqual([]);
    expect(h.transform('Kerror\n')).toBe('\x1b[2K\x1b[1;91merror\x1b[0m\n');
    h.dispose();
  });

  it('dispose flushes remaining held text and stops the timer', () => {
    const deferred: string[] = [];
    const h = createKeywordHighlighter({ onDeferred: (t) => deferred.push(t) });
    h.transform('bin');
    h.dispose();
    expect(deferred).toEqual(['bin']);
    vi.advanceTimersByTime(64);
    expect(deferred).toEqual(['bin']);
  });
});

describe('configurable role colors', () => {
  it('keeps byte-identical output with the theme preset and no overrides', () => {
    const h = createKeywordHighlighter({ highlight: { preset: 'theme' } });
    expect(h.transform('root@web:~# ls\n')).toBe(
      '\x1b[1;92mroot@web\x1b[0m\x1b[1;94m:~\x1b[0m# ls\n'
    );
  });

  it('applies overrides for a role while keeping others', () => {
    const h = createKeywordHighlighter({
      highlight: { preset: 'theme', overrides: { directory: 1 } },
    });
    expect(h.transform('drwxr-xr-x 2 root root 4096 Apr 22 09:15 bin\n')).toBe(
      '\x1b[36mdrwxr-xr-x\x1b[0m 2 root root 4096 \x1b[93mApr 22 09:15\x1b[0m \x1b[1;31mbin\x1b[0m\n'
    );
  });

  it('switches presets at runtime via setHighlight', () => {
    const h = createKeywordHighlighter();
    expect(h.transform('ERROR x\n')).toBe('\x1b[1;91mERROR\x1b[0m x\n');
    h.setHighlight('high_contrast', {});
    expect(h.transform('ERROR x\n')).toBe('\x1b[1;91mERROR\x1b[0m x\n');
    h.setHighlight('high_contrast', { kw_error: 5 });
    expect(h.transform('ERROR x\n')).toBe('\x1b[1;35mERROR\x1b[0m x\n');
  });

  it('high_contrast renders prompt path and info keyword with the cross-theme readable slot 8', () => {
    const { transform } = createKeywordHighlighter({
      highlight: { preset: 'high_contrast', overrides: {} },
    });
    // 槽 8（亮灰）在浅色调色板为深灰、深色调色板为浅灰，明暗主题都可读。
    expect(transform('root@srv:/var/www# ls\n')).toBe(
      '\x1b[1;96mroot@srv\x1b[0m\x1b[1;90m:/var/www\x1b[0m# ls\n'
    );
    expect(transform('INFO x\n')).toBe('\x1b[1;90mINFO\x1b[0m x\n');
  });

  it('ignores out-of-range override slots', () => {
    const h = createKeywordHighlighter({
      highlight: { preset: 'theme', overrides: { directory: 99, kw_error: 1 } },
    });
    expect(h.transform('ERROR x\n')).toBe('\x1b[1;31mERROR\x1b[0m x\n');
  });
});

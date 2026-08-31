import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

const variablesCss = readFileSync(
  new URL('../../../shared/styles/variables.css', import.meta.url),
  'utf8'
);
const terminalWorkspaceCss = readFileSync(
  new URL('../../../app/layouts/TerminalWorkspace.module.css', import.meta.url),
  'utf8'
);

describe('terminal theme CSS scope', () => {
  it('limits independent terminal palettes to terminal surfaces', () => {
    expect(variablesCss).toContain(
      "[data-terminal-theme='nocterm_light'] .nocterm-terminal-surface"
    );
    expect(variablesCss).toContain(
      "[data-terminal-theme='nocterm_dark'] .nocterm-terminal-surface"
    );
    expect(variablesCss).toContain("[data-terminal-theme='midnight'] .nocterm-terminal-surface");
    expect(variablesCss).toContain("[data-terminal-theme='graphite'] .nocterm-terminal-surface");
    expect(variablesCss).toContain("[data-terminal-theme='forest'] .nocterm-terminal-surface");
    expect(variablesCss).toContain("[data-terminal-theme='amber'] .nocterm-terminal-surface");
    // 根级覆盖会污染复用 --term-* 的 SFTP 与状态栏，必须保持不可出现。
    expect(variablesCss).not.toMatch(/\[data-terminal-theme='[^']+'\]\s*\{/u);
  });

  it('defines the complete ANSI palette for every terminal scheme', () => {
    const schemes = ['nocterm_light', 'nocterm_dark', 'midnight', 'graphite', 'forest', 'amber'];
    const ansiVariables = [
      'black',
      'red',
      'green',
      'yellow',
      'blue',
      'magenta',
      'cyan',
      'white',
      'bright-black',
      'bright-red',
      'bright-green',
      'bright-yellow',
      'bright-blue',
      'bright-magenta',
      'bright-cyan',
      'bright-white',
    ];

    for (const scheme of schemes) {
      const rule = variablesCss.match(
        new RegExp(
          `\\[data-terminal-theme='${scheme}'\\] \\.nocterm-terminal-surface\\s*\\{[^}]+\\}`,
          'u'
        )
      )?.[0];

      expect(rule, `${scheme} theme rule`).toBeDefined();
      for (const variable of ansiVariables) {
        expect(rule).toContain(`--term-${variable}:`);
      }
    }
  });

  it('keeps terminal tabs independent from application hover and shadow tokens', () => {
    const activeTabRule = terminalWorkspaceCss.match(/\.tab\.active\s*\{[^}]+\}/u)?.[0];
    const hoverTabRule = terminalWorkspaceCss.match(/\.tab:hover\s*\{[^}]+\}/u)?.[0];

    expect(activeTabRule).toContain('box-shadow: none');
    expect(activeTabRule).not.toContain('var(--shadow-');
    expect(hoverTabRule).not.toContain('var(--bg-');
  });

  it('keeps the empty terminal workspace on the selected terminal palette', () => {
    const terminalAreaRule = terminalWorkspaceCss.match(/\.terminalArea\s*\{[^}]+\}/u)?.[0];
    const emptyIconRule = terminalWorkspaceCss.match(/\.emptyIcon\s*\{[^}]+\}/u)?.[0];
    const emptyTitleRule = terminalWorkspaceCss.match(/\.emptyTitle\s*\{[^}]+\}/u)?.[0];
    const emptyDescRule = terminalWorkspaceCss.match(/\.emptyDesc\s*\{[^}]+\}/u)?.[0];
    const secondaryButtonRule = terminalWorkspaceCss.match(
      /^\.emptySecondaryBtn\s*\{\s*color:[^}]+\}/mu
    )?.[0];

    expect(terminalAreaRule).toContain('background: var(--term)');
    expect(emptyIconRule).toContain('color: var(--term-muted)');
    expect(emptyTitleRule).toContain('color: var(--term-text)');
    expect(emptyDescRule).toContain('color: var(--term-muted)');
    expect(secondaryButtonRule).toContain('border: 1px solid var(--term-border)');
    expect(
      [terminalAreaRule, emptyIconRule, emptyTitleRule, emptyDescRule, secondaryButtonRule].join(
        '\n'
      )
    ).not.toMatch(/var\(--(?:bg|text|border)-/u);
  });
});

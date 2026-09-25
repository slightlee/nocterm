import { useMemo } from 'react';

import { readHighlightPalette, type HighlightPaletteSnapshot } from './highlight-guard';
import type { ResolvedTerminalTheme } from './terminal-appearance';

const SENTINEL_STYLE: Partial<CSSStyleDeclaration> = {
  position: 'absolute',
  width: '0',
  height: '0',
  overflow: 'hidden',
  visibility: 'hidden',
  pointerEvents: 'none',
};

/**
 * 在 body 下临时挂一个「主题作用域 + surface」哨兵并读出该主题的调色板快照。
 * 主题属性挂哨兵自己的包裹层上——不依赖根节点，避免与页面里其他
 * data-terminal-theme 作用域（方案卡、预览）互相覆盖。
 */
function readPaletteForTheme(themeId: ResolvedTerminalTheme): HighlightPaletteSnapshot {
  const scope = document.createElement('div');
  scope.dataset.terminalTheme = themeId;
  const sentinel = document.createElement('div');
  sentinel.className = 'nocterm-terminal-surface';
  Object.assign(sentinel.style, SENTINEL_STYLE);
  scope.appendChild(sentinel);
  document.body.appendChild(scope);
  try {
    return readHighlightPalette(sentinel);
  } finally {
    scope.remove();
  }
}

/**
 * 读取指定终端主题调色板（16 槽 + 背景）的 React Hook。
 *
 * 16 色变量只挂在 `[data-terminal-theme='…'] .nocterm-terminal-surface` 作用域，
 * 主题 id 由 SettingsContext 解析下发（跟随应用主题时切换会重新读取）——
 * 预览与真实终端所见同源。
 */
export function useTerminalPalette(themeId: ResolvedTerminalTheme): HighlightPaletteSnapshot {
  return useMemo(() => readPaletteForTheme(themeId), [themeId]);
}

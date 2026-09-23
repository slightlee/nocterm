import { useEffect, useState } from 'react';

import { readHighlightPalette, type HighlightPaletteSnapshot } from './highlight-guard';

const SENTINEL_STYLE: Partial<CSSStyleDeclaration> = {
  position: 'absolute',
  width: '0',
  height: '0',
  overflow: 'hidden',
  visibility: 'hidden',
  pointerEvents: 'none',
};

/**
 * 读取当前终端主题调色板（16 槽 + 背景）的 React Hook。
 *
 * 16 色变量只挂在 `[data-terminal-theme='…'] .nocterm-terminal-surface` 作用域，
 * 设置页不在终端工作区内，本 Hook 会在 body 下挂一个隐藏哨兵让主题选择器命中，
 * 并监听根上的主题属性变化实时重读——预览与真实终端所见同源。
 */
export function useTerminalPalette(): HighlightPaletteSnapshot {
  const [palette, setPalette] = useState<HighlightPaletteSnapshot>(() =>
    readHighlightPalette(document.documentElement)
  );

  useEffect(() => {
    const sentinel = document.createElement('div');
    sentinel.className = 'nocterm-terminal-surface';
    Object.assign(sentinel.style, SENTINEL_STYLE);
    document.body.appendChild(sentinel);

    const refresh = () => setPalette(readHighlightPalette(sentinel));
    refresh();
    const observer = new MutationObserver(refresh);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['data-terminal-theme'],
    });
    return () => {
      observer.disconnect();
      sentinel.remove();
    };
  }, []);

  return palette;
}

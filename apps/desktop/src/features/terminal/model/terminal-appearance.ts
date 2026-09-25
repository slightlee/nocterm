import type { ITheme, Terminal } from '@xterm/xterm';

import { readTerminalHighlightConfig } from './highlight-config';
import type { TerminalHighlightConfig } from './highlight-config';

export const TERMINAL_FONT_FAMILY = "'SF Mono', 'JetBrains Mono', Menlo, Consolas, monospace";
const DEFAULT_FONT_SIZE = 13;

function readCssVariable(styles: CSSStyleDeclaration, name: string): string {
  return styles.getPropertyValue(name).trim();
}

/** xterm 与设置页共享同一组 CSS 语义色，实际终端不会出现预览和运行效果漂移。 */
export function readTerminalTheme(container: HTMLElement): ITheme {
  const variables = getComputedStyle(container);
  const foreground = readCssVariable(variables, '--term-text');
  const background = readCssVariable(variables, '--term');
  const cursor = readCssVariable(variables, '--term-blue');
  const selectionBackground = readCssVariable(variables, '--term-selection');
  const black = readCssVariable(variables, '--term-black');
  const red = readCssVariable(variables, '--term-red');
  const green = readCssVariable(variables, '--term-green');
  const yellow = readCssVariable(variables, '--term-yellow');
  const blue = readCssVariable(variables, '--term-blue');
  const magenta = readCssVariable(variables, '--term-magenta');
  const cyan = readCssVariable(variables, '--term-cyan');
  const white = readCssVariable(variables, '--term-white');

  return {
    background,
    foreground,
    cursor,
    selectionBackground,
    selectionForeground: foreground,
    black,
    red,
    green,
    yellow,
    blue,
    magenta,
    cyan,
    white,
    brightBlack: readCssVariable(variables, '--term-bright-black'),
    brightRed: readCssVariable(variables, '--term-bright-red'),
    brightGreen: readCssVariable(variables, '--term-bright-green'),
    brightYellow: readCssVariable(variables, '--term-bright-yellow'),
    brightBlue: readCssVariable(variables, '--term-bright-blue'),
    brightMagenta: readCssVariable(variables, '--term-bright-magenta'),
    brightCyan: readCssVariable(variables, '--term-bright-cyan'),
    brightWhite: readCssVariable(variables, '--term-bright-white'),
  };
}

export function readTerminalFontSize(): number {
  const value = Number(document.documentElement.dataset.terminalFontSize);
  return Number.isFinite(value) ? value : DEFAULT_FONT_SIZE;
}

/**
 * 将当前设置写入真实 xterm 实例。该函数也会在终端从隐藏页面恢复时调用，
 * 避免 WebView 在 `display: none` 期间读取到旧的继承变量后一直保留旧配色。
 */
export function applyTerminalAppearance(terminal: Terminal, container: HTMLElement): void {
  terminal.options.theme = readTerminalTheme(container);
  terminal.options.fontSize = readTerminalFontSize();
}

/**
 * 主题作用域属性（data-terminal-theme）挂在终端工作区容器上而非根节点：
 * 根属性变化时容器可能尚未提交新值，observer 会读到旧变量。因此主题改由
 * 工作区在属性提交完成后广播本事件驱动，事件时序保证 CSS 变量已是新主题。
 */
export const TERMINAL_APPEARANCE_EVENT = 'nocterm:terminal-appearance-changed';

/** 主题作用域属性提交到 DOM 后由工作区调用，通知全部终端实例重读外观。 */
export function notifyTerminalAppearanceChanged(): void {
  window.dispatchEvent(new Event(TERMINAL_APPEARANCE_EVENT));
}

/**
 * 监听外观变化而不重建 React 会话 effect，避免外观切换触发 SSH 重连。
 * 字号与字体颜色配置仍走根属性通道（SettingsProvider 直接写根）；
 * 主题经 TERMINAL_APPEARANCE_EVENT 事件驱动。
 */
export function observeTerminalAppearance(
  terminal: Terminal,
  container: HTMLElement,
  fit: () => void,
  onHighlightChange?: (config: TerminalHighlightConfig) => void
): () => void {
  const apply = () => {
    applyTerminalAppearance(terminal, container);
    onHighlightChange?.(readTerminalHighlightConfig());
    fit();
  };
  const observer = new MutationObserver(apply);
  observer.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['data-terminal-font-size', 'data-terminal-highlight'],
  });
  window.addEventListener(TERMINAL_APPEARANCE_EVENT, apply);
  return () => {
    observer.disconnect();
    window.removeEventListener(TERMINAL_APPEARANCE_EVENT, apply);
  };
}

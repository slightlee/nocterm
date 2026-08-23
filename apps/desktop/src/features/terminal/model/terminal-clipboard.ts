/**
 * 终端的右键粘贴。
 *
 * 应用壳在捕获阶段统一 `preventDefault` 掉了 WebView 原生右键菜单（`AppShell`），
 * 因此终端里既没有菜单，也就没有"粘贴"菜单项；而右键即粘贴本来就是终端的通用习惯
 * （PuTTY、WindTerm、MobaXterm 默认如此）。这里显式补上这条路径。
 *
 * 刻意不接管 Ctrl+V / Ctrl+Shift+V / Shift+Insert：这些是 WebView 自带的粘贴快捷键，
 * xterm 已在 textarea 上监听 `paste` 事件并转成 `onData`。若在此再读一次剪贴板，
 * 原生粘贴不会被抑制（xterm 的自定义按键回调返回 false 只是跳过自身处理，不会
 * `preventDefault`），结果是粘贴两遍。
 */

/** 粘贴目标：`paste` 交回 xterm 处理，让换行归一化与 bracketed paste 包裹保持一致。 */
export interface TerminalPasteTarget {
  paste: (text: string) => void;
  /** 剪贴板不可读时给用户的提示，由调用方决定写到终端还是状态栏。 */
  onUnavailable: (message: string) => void;
}

const CLIPBOARD_HINT = '无法读取剪贴板，请改用 Ctrl+V 粘贴';

/** 绑定右键粘贴，返回解绑函数供组件卸载时调用。 */
export function attachRightClickPaste(
  container: HTMLElement,
  target: TerminalPasteTarget
): () => void {
  const handleContextMenu = (event: MouseEvent) => {
    // 阻止冒泡到应用壳，避免业务右键菜单在终端区域里被意外唤起。
    event.preventDefault();
    event.stopPropagation();

    const clipboard = navigator.clipboard;
    if (typeof clipboard?.readText !== 'function') {
      target.onUnavailable(CLIPBOARD_HINT);
      return;
    }
    void clipboard.readText().then(
      (text) => {
        if (text) target.paste(text);
      },
      // WebView 可能因权限策略拒绝读取剪贴板，此时明确引导到仍然可用的原生快捷键。
      () => target.onUnavailable(CLIPBOARD_HINT)
    );
  };

  container.addEventListener('contextmenu', handleContextMenu);
  return () => container.removeEventListener('contextmenu', handleContextMenu);
}

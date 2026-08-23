/**
 * 终端的复制与右键粘贴。
 *
 * 应用壳在捕获阶段统一 `preventDefault` 掉了 WebView 原生右键菜单（`AppShell`），
 * 因此终端里既没有菜单，也就没有"复制/粘贴"菜单项；而右键即粘贴本来就是终端的通用习惯
 * （PuTTY、WindTerm、MobaXterm 默认如此）。这里显式补上这两条路径。
 *
 * 复制必须由我们自己接管：xterm 只在收到浏览器原生 `copy` 事件时才把选区写进
 * `clipboardData`，而 Ctrl+C 会先被 xterm 的按键处理吃掉并当成中断信号（`\x03`）发给
 * 远端，浏览器根本不会派发 `copy`；加上选区由 canvas 绘制、不是 DOM 选区，系统的
 * "复制"命令也无从下手。结果就是 Windows 上完全没有复制入口——macOS 上 Cmd+C 走的是
 * 系统菜单的原生复制命令，恰好绕开了这条路，所以只在 Windows 暴露。
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

/** 复制来源：由组件把 xterm 的选区 API 注入进来，便于在无 DOM 环境下单测判定逻辑。 */
export interface TerminalCopySource {
  hasSelection: () => boolean;
  selection: () => string;
  /** 复制后清空选区：与 Windows Terminal 一致，也让紧接着的 Ctrl+C 恢复为中断信号。 */
  clearSelection: () => void;
  onUnavailable: (message: string) => void;
}

const COPY_HINT = '无法写入剪贴板';

/**
 * 判定一次 keydown 是否应当解释为"复制"。快捷键沿用现有终端的约定，不自创：
 *
 * - `Ctrl+Shift+C`：Windows Terminal、GNOME Terminal、Tabby 的复制键，无条件复制；
 * - `Ctrl+Insert`：PuTTY、Xshell 一系的老约定，同样无条件复制；
 * - `Ctrl+C`：**仅在有选区时**才是复制，否则必须原样当中断信号送给远端——这正是
 *   Windows Terminal 的行为。复制后清空选区，因此"选中→Ctrl+C 复制→再 Ctrl+C 中断"
 *   不会互相打架。
 *
 * `Alt` 组合留给终端自己（Alt+字母是 Meta 前缀），`Meta`（macOS 的 Cmd）交给系统原生
 * 复制命令，避免与 Tauri 的编辑菜单重复写一遍剪贴板。
 */
export function isCopyShortcut(
  event: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'shiftKey' | 'altKey' | 'metaKey'>,
  hasSelection: boolean
): boolean {
  if (!event.ctrlKey || event.altKey || event.metaKey) return false;
  if (event.key === 'Insert') return !event.shiftKey;
  // 大小写都要认：按下 Shift 时 `key` 是大写的 `C`。
  if (event.key !== 'c' && event.key !== 'C') return false;
  return event.shiftKey || hasSelection;
}

/** 写系统剪贴板；返回是否成功，便于失败时给出提示而不是静默丢弃。 */
export type ClipboardWriter = (text: string) => Promise<boolean>;

/**
 * 优先用异步剪贴板 API，失败再退到 `execCommand('copy')`。
 *
 * 两条路都保留是因为失败原因不同：WebView 可能按权限策略拒绝异步 API（与右键粘贴读
 * 剪贴板遇到的是同一类拒绝），而 `execCommand` 走的是"选中可编辑元素再复制"的老路径，
 * 不需要权限，但要求文档处于用户手势中——按键回调正好满足。
 */
async function writeClipboardText(text: string): Promise<boolean> {
  const clipboard = navigator.clipboard;
  if (typeof clipboard?.writeText === 'function') {
    try {
      await clipboard.writeText(text);
      return true;
    } catch {
      // 落到下面的兜底路径，不向上抛。
    }
  }
  return copyViaTemporaryTextarea(text);
}

/** 老式兜底：把文本放进一个屏幕外 textarea，选中后执行系统复制命令，随后还原焦点。 */
function copyViaTemporaryTextarea(text: string): boolean {
  const active = document.activeElement;
  const holder = document.createElement('textarea');
  holder.value = text;
  // 不能用 display:none 或 visibility:hidden——不可见元素无法被选中，复制会静默失败。
  holder.setAttribute('aria-hidden', 'true');
  holder.style.position = 'fixed';
  holder.style.top = '-1000px';
  holder.style.opacity = '0';
  document.body.append(holder);
  try {
    holder.select();
    return document.execCommand('copy');
  } catch {
    return false;
  } finally {
    holder.remove();
    // 焦点必须还给 xterm 的输入区，否则复制之后键盘就打不进终端了。
    if (active instanceof HTMLElement) active.focus();
  }
}

/**
 * 复制当前选区。空选区直接返回：`Ctrl+Shift+C` 在没选中任何内容时应当什么都不做，
 * 既不写剪贴板也不该弹提示。
 */
export async function copyTerminalSelection(
  source: TerminalCopySource,
  write: ClipboardWriter = writeClipboardText
): Promise<boolean> {
  const text = source.selection();
  if (!text) return false;
  const copied = await write(text);
  if (!copied) {
    source.onUnavailable(COPY_HINT);
    return false;
  }
  source.clearSelection();
  return true;
}

/**
 * 生成交给 `terminal.attachCustomKeyEventHandler` 的回调：返回 `false` 表示这次按键已被
 * 复制消化，xterm 不再按终端语义处理（关键就在于不把 `\x03` 发给远端）。
 */
export function createCopyKeyHandler(
  source: TerminalCopySource,
  write?: ClipboardWriter
): (event: KeyboardEvent) => boolean {
  return (event) => {
    if (event.type !== 'keydown') return true;
    if (!isCopyShortcut(event, source.hasSelection())) return true;
    void copyTerminalSelection(source, write);
    return false;
  };
}

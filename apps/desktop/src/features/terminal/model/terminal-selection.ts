export interface TerminalSelectionActionState {
  compact: boolean;
  left: number;
  top: number;
  selection: string;
}

interface TerminalSelectionSource {
  active: () => boolean;
  selection: () => string;
  selectionAnchor: () => SelectionAnchor | null;
  onContentChange: (listener: () => void) => { dispose: () => void };
  onSelectionChange: (listener: () => void) => { dispose: () => void };
  onViewportChange: (listener: () => void) => { dispose: () => void };
}

export interface SelectionAnchor {
  compact: boolean;
  right: number;
  y: number;
  rowHeight: number;
}

interface SelectionBounds {
  width: number;
  height: number;
}

const ACTION_WIDTH = 92;
const COMPACT_ACTION_WIDTH = 28;
const ACTION_HEIGHT = 28;
const ACTION_INSET = 6;

/** 内嵌到选区首行右端，把临时遮挡限制在用户已经选择的内容内。 */
export function positionTerminalSelectionAction(
  point: SelectionAnchor,
  bounds: SelectionBounds
): Pick<TerminalSelectionActionState, 'compact' | 'left' | 'top'> {
  const actionWidth = point.compact ? COMPACT_ACTION_WIDTH : ACTION_WIDTH;
  const maxLeft = Math.max(ACTION_INSET, bounds.width - actionWidth - ACTION_INSET);
  const candidateLeft = point.right - actionWidth;
  const left = Math.min(Math.max(candidateLeft, ACTION_INSET), maxLeft);
  // 从选区首行顶部开始放置，避免按钮高度大于行高时遮挡上一行未选中的输出。
  const candidateTop = point.y;
  const maxTop = Math.max(ACTION_INSET, bounds.height - ACTION_HEIGHT - ACTION_INSET);
  const top = Math.min(Math.max(candidateTop, ACTION_INSET), maxTop);
  return { compact: point.compact, left, top };
}

/** 将 xterm 的缓冲区坐标转换为终端容器内的首行锚点。 */
export function getTerminalSelectionAnchor(
  host: HTMLElement,
  terminalElement: HTMLElement | undefined,
  position: { start: { x: number; y: number }; end: { x: number; y: number } },
  viewportY: number,
  cols: number,
  rows: number
): SelectionAnchor | null {
  const screen = terminalElement?.querySelector<HTMLElement>('.xterm-screen');
  if (!screen || cols <= 0 || rows <= 0) return null;
  const screenBounds = screen.getBoundingClientRect();
  const hostBounds = host.getBoundingClientRect();
  const cellWidth = screenBounds.width / cols;
  const rowHeight = screenBounds.height / rows;
  if (!Number.isFinite(cellWidth) || !Number.isFinite(rowHeight) || rowHeight <= 0) return null;
  // xterm 6 的 selection API 返回零基坐标；这里仍规范化方向，避免反向选择改变锚点。
  const startPrecedesEnd =
    position.start.y < position.end.y ||
    (position.start.y === position.end.y && position.start.x <= position.end.x);
  const first = startPrecedesEnd ? position.start : position.end;
  const last = startPrecedesEnd ? position.end : position.start;
  const visibleRow = Math.min(Math.max(first.y - viewportY, 0), rows - 1);
  const rightColumn = first.y === last.y ? Math.max(first.x, last.x) : cols;
  return {
    compact: first.y === last.y,
    right:
      screenBounds.left - hostBounds.left + Math.min(Math.max(rightColumn, 0), cols) * cellWidth,
    y: screenBounds.top - hostBounds.top + visibleRow * rowHeight,
    rowHeight,
  };
}

/**
 * 只在一次鼠标选择完成后暴露动作；拖动、滚轮和选区变化都会先隐藏旧入口。
 * 选区正文保留在 xterm 实例附近，直到用户明确点击后才交给应用层。
 */
export function attachTerminalSelectionAction(
  host: HTMLElement,
  source: TerminalSelectionSource,
  onChange: (action: TerminalSelectionActionState | null) => void
): () => void {
  let selecting = false;
  let releasePending = false;
  let disposed = false;
  let releaseTimer: ReturnType<typeof setTimeout> | null = null;
  const hide = () => onChange(null);
  const clearReleaseTimer = () => {
    if (releaseTimer === null) return;
    clearTimeout(releaseTimer);
    releaseTimer = null;
  };
  const publishSelection = () => {
    releasePending = false;
    clearReleaseTimer();
    if (!source.active()) {
      hide();
      return;
    }
    const selection = source.selection();
    if (!selection.trim()) {
      hide();
      return;
    }
    const anchor = source.selectionAnchor();
    if (!anchor) {
      hide();
      return;
    }
    const bounds = host.getBoundingClientRect();
    const position = positionTerminalSelectionAction(anchor, bounds);
    onChange({ ...position, selection });
  };
  const selectionSubscription = source.onSelectionChange(() => {
    if (disposed) return;
    if (!source.active()) {
      hide();
      return;
    }
    // xterm 在长距离拖动时会连续发布变化；只有 mouseup 后的最终状态可以生成动作。
    if (selecting) {
      hide();
      return;
    }
    if (releasePending) {
      publishSelection();
      return;
    }
    hide();
  });
  const invalidate = () => {
    selecting = false;
    releasePending = false;
    clearReleaseTimer();
    hide();
  };
  const contentSubscription = source.onContentChange(invalidate);
  const viewportSubscription = source.onViewportChange(() => {
    // 拖出可视区时 xterm 会自动滚动；保持手势，待 mouseup 后按最终视口计算位置。
    if (selecting) {
      hide();
      return;
    }
    invalidate();
  });
  const handleMouseDown = (event: MouseEvent) => {
    if (event.button !== 0) return;
    selecting = true;
    releasePending = false;
    clearReleaseTimer();
    hide();
  };
  const handleMouseUp = (event: MouseEvent) => {
    if (event.button !== 0 || !selecting) return;
    selecting = false;
    releasePending = true;

    // 捕获阶段先执行；零延时任务会在 xterm 的 mouseup 处理完成后读取最终选区。
    clearReleaseTimer();
    releaseTimer = setTimeout(() => {
      releaseTimer = null;
      if (disposed) return;
      if (releasePending) publishSelection();
    }, 0);
  };

  // 捕获阶段不受 xterm 内部传播控制影响；document 监听也覆盖拖出终端后松开。
  host.addEventListener('mousedown', handleMouseDown, true);
  host.ownerDocument.addEventListener('mouseup', handleMouseUp, true);
  host.addEventListener('wheel', invalidate, { capture: true, passive: true });

  return () => {
    disposed = true;
    clearReleaseTimer();
    selectionSubscription.dispose();
    contentSubscription.dispose();
    viewportSubscription.dispose();
    host.removeEventListener('mousedown', handleMouseDown, true);
    host.ownerDocument.removeEventListener('mouseup', handleMouseUp, true);
    host.removeEventListener('wheel', invalidate, true);
    onChange(null);
  };
}

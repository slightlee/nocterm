const POPOVER_MARGIN = 12;
const POPOVER_GAP = 8;
const MAX_POPOVER_HEIGHT = 440; // 紧凑浮层最多展示约八行，更多会话在列表内部滚动。

export interface SessionPickerAnchor {
  left: number;
  right: number;
  top: number;
}

export interface SessionPickerSize {
  width: number;
  height: number;
}

export interface SessionPickerViewport {
  width: number;
  height: number;
}

export interface SessionPickerPosition {
  left: number;
  top: number;
  width: number;
  maxHeight: number;
}

function clamp(value: number, minimum: number, maximum: number) {
  return Math.min(Math.max(value, minimum), maximum);
}

/** 窗口变窄或贴近边缘时，浮层缩窄并选择空间更充足的一侧。 */
export function resolveSessionPickerPosition(
  anchor: SessionPickerAnchor,
  panel: SessionPickerSize,
  viewport: SessionPickerViewport
): SessionPickerPosition {
  const width = Math.min(panel.width, Math.max(0, viewport.width - POPOVER_MARGIN * 2));
  const maxHeight = Math.max(0, Math.min(MAX_POPOVER_HEIGHT, viewport.height - POPOVER_MARGIN * 2));
  const height = Math.min(panel.height, maxHeight);
  const rightSpace = viewport.width - POPOVER_MARGIN - anchor.right - POPOVER_GAP;
  const leftSpace = anchor.left - POPOVER_GAP - POPOVER_MARGIN;
  const openRight = rightSpace >= width || rightSpace >= leftSpace;
  const maximumLeft = Math.max(POPOVER_MARGIN, viewport.width - width - POPOVER_MARGIN);
  const preferredLeft = openRight ? anchor.right + POPOVER_GAP : anchor.left - POPOVER_GAP - width;
  const maximumTop = Math.max(POPOVER_MARGIN, viewport.height - height - POPOVER_MARGIN);

  return {
    left: clamp(preferredLeft, POPOVER_MARGIN, maximumLeft),
    top: clamp(anchor.top, POPOVER_MARGIN, maximumTop),
    width,
    maxHeight,
  };
}

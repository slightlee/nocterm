interface Anchor {
  left: number;
  top: number;
  bottom: number;
}

/** 优先向上打开；锚点靠近窗口上缘时翻转，并为窗口四边保留 8px 间距。 */
export function modelMenuPosition(anchor: Anchor, viewportWidth: number, viewportHeight: number) {
  const width = Math.min(460, Math.max(0, viewportWidth - 16));
  const left = Math.max(8, Math.min(anchor.left, viewportWidth - width - 8));
  const above = anchor.top - 16;
  const below = viewportHeight - anchor.bottom - 16;
  // 400px 是目录的舒适高度；空间更少时让 CSS 滚动区域承担收缩。
  const upwards = above >= Math.min(400, below);
  return {
    width: `${width}px`,
    left: `${left}px`,
    top: upwards ? 'auto' : `${anchor.bottom + 8}px`,
    bottom: upwards ? `${viewportHeight - anchor.top + 8}px` : 'auto',
    maxHeight: `${Math.max(0, upwards ? above : below)}px`,
  };
}

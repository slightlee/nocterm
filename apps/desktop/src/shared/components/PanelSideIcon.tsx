interface PanelSideIconProps {
  className?: string;
  side: 'left' | 'right';
}

/** 左右面板共用同一轮廓，仅镜像分隔线位置，避免两侧折叠入口产生视觉漂移。 */
export function PanelSideIcon({ className, side }: PanelSideIconProps) {
  return (
    <svg aria-hidden="true" className={className} viewBox="0 0 24 24">
      <rect x="2" y="3" width="20" height="18" rx="5" />
      <path d={side === 'left' ? 'M7 3v18' : 'M17 3v18'} />
    </svg>
  );
}

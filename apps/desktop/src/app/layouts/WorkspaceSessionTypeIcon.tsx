export type WorkspaceSessionKind = 'terminal' | 'file';

interface WorkspaceSessionTypeIconProps {
  className?: string;
  kind: WorkspaceSessionKind;
}

/** 统一标签栏与会话索引的类型图标，避免相同会话在两个入口产生视觉差异。 */
export function WorkspaceSessionTypeIcon({ className, kind }: WorkspaceSessionTypeIconProps) {
  if (kind === 'file') {
    return (
      <svg
        aria-hidden="true"
        className={className}
        fill="none"
        stroke="currentColor"
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="1.8"
        viewBox="0 0 24 24"
      >
        <path d="M3 7.5A2.5 2.5 0 0 1 5.5 5H10l2 2.5h6.5A2.5 2.5 0 0 1 21 10v7a3 3 0 0 1-3 3H5a2 2 0 0 1-2-2.5z" />
      </svg>
    );
  }

  return (
    <svg
      aria-hidden="true"
      className={className}
      fill="none"
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth="2"
      viewBox="0 0 24 24"
    >
      <polyline points="4 17 10 11 4 5" />
      <line x1="12" y1="19" x2="20" y2="19" />
    </svg>
  );
}

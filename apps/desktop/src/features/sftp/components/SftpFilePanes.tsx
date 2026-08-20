import type { KeyboardEvent, MouseEvent } from 'react';
import { ContextMenu, type ContextMenuItem } from '../../../shared/components/ContextMenu';
import type {
  CreateEditState,
  FileKind,
  FileRow,
  PathEditState,
  RenameEditState,
  SortKey,
  SortState,
} from '../types/sftp-types';
import styles from './SFTPView.module.css';

type SftpMenuItem = ContextMenuItem | 'separator';

function FileIcon({ kind }: { kind: FileKind }) {
  const iconClassName = `${styles.fileIcon} ${styles[`fileIcon-${kind}`]}`;

  if (kind === 'parent') {
    return (
      <svg className={iconClassName} viewBox="0 0 24 24" aria-hidden="true">
        <path d="M4 12h14" />
        <path d="M10 6l-6 6 6 6" />
      </svg>
    );
  }

  if (kind === 'folder') {
    return (
      <svg className={iconClassName} viewBox="0 0 24 24" aria-hidden="true">
        <path d="M3.5 7.5A2.5 2.5 0 0 1 6 5h4.2l2 2.4H18a2.5 2.5 0 0 1 2.5 2.5v6.6A2.5 2.5 0 0 1 18 19H6a2.5 2.5 0 0 1-2.5-2.5z" />
      </svg>
    );
  }

  return (
    <svg className={iconClassName} viewBox="0 0 24 24" aria-hidden="true">
      <path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" />
      <path d="M14 3v5h5" />
      {kind === 'code' && <path d="M9.5 13.5 8 15l1.5 1.5M14.5 13.5 16 15l-1.5 1.5" />}
      {kind === 'archive' && <path d="M11 7h2M11 10h2M11 13h2M10 16h4" />}
      {kind === 'env' && <path d="M8.5 15.5h7M8.5 12h7" />}
      {kind === 'log' && <path d="M8.5 12h7M8.5 15h5M8.5 18h6" />}
    </svg>
  );
}

function RefreshIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M21 12a9 9 0 0 1-15.5 6.2" />
      <path d="M3 12A9 9 0 0 1 18.5 5.8" />
      <path d="M18.5 2.5v3.3h-3.3" />
      <path d="M5.5 21.5v-3.3h3.3" />
    </svg>
  );
}

function MenuIcon({
  name,
}: {
  name: 'upload' | 'download' | 'copy' | 'enter' | 'delete' | 'rename' | 'folder';
}) {
  const icons = {
    upload: (
      <>
        <path d="M12 19V5" />
        <path d="M7 10l5-5 5 5" />
        <path d="M5 19h14" />
      </>
    ),
    download: (
      <>
        <path d="M12 5v14" />
        <path d="M7 14l5 5 5-5" />
        <path d="M5 5h14" />
      </>
    ),
    copy: (
      <>
        <rect x="9" y="9" width="11" height="11" rx="2" />
        <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
      </>
    ),
    enter: (
      <>
        <path d="M5 12h14" />
        <path d="m13 6 6 6-6 6" />
      </>
    ),
    delete: (
      <>
        <path d="M3 6h18" />
        <path d="M8 6V4h8v2" />
        <path d="M19 6l-1 14H6L5 6" />
      </>
    ),
    rename: (
      <>
        <path d="M4 20h16" />
        <path d="M14.5 4.5 19.5 9.5 10 19H5v-5z" />
      </>
    ),
    folder: (
      <>
        <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
        <path d="M12 11v5" />
        <path d="M9.5 13.5h5" />
      </>
    ),
  };

  return (
    <svg
      viewBox="0 0 24 24"
      aria-hidden="true"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      {icons[name]}
    </svg>
  );
}

function copyText(value: string | undefined) {
  if (!value) return;
  void navigator.clipboard.writeText(value);
}

function SortableTableHeader({
  sort,
  onSort,
}: {
  sort: SortState;
  onSort: (key: SortKey) => void;
}) {
  const sortMark = (key: SortKey) =>
    sort.key === key ? (sort.direction === 'asc' ? '↑' : '↓') : '';

  return (
    <div className={styles.tableHeader}>
      <button type="button" className={styles.headerButton} onClick={() => onSort('name')}>
        名称 <span>{sortMark('name')}</span>
      </button>
      <button
        type="button"
        className={`${styles.headerButton} ${styles.alignRight}`}
        onClick={() => onSort('size')}
      >
        大小 <span>{sortMark('size')}</span>
      </button>
      <button
        type="button"
        className={`${styles.headerButton} ${styles.alignRight}`}
        onClick={() => onSort('modified')}
      >
        修改时间 <span>{sortMark('modified')}</span>
      </button>
    </div>
  );
}

function PaneNotice({
  tone,
  title,
  message,
}: {
  tone: 'error' | 'info';
  title: string;
  message: string;
}) {
  return (
    <div className={`${styles.paneNotice} ${styles[`paneNotice-${tone}`]}`}>
      <span className={styles.noticeIcon} aria-hidden="true">
        {tone === 'error' ? '!' : 'i'}
      </span>
      <div className={styles.noticeContent}>
        <strong>{title}</strong>
        <p>{message}</p>
      </div>
    </div>
  );
}

interface FilePaneProps {
  path: string;
  parent: string | null;
  rows: FileRow[];
  loading: boolean;
  error: string;
  selectedNames: Set<string>;
  sort: SortState;
  createEdit: CreateEditState | null;
  renameEdit: RenameEditState | null;
  pathEdit: PathEditState | null;
  onSelect: (name: string, additive: boolean, range: boolean) => void;
  onSelectAll: () => void;
  onClearSelection: () => void;
  onOpenDir: (path: string) => void;
  onOpenParent: () => void;
  onRefresh: () => void;
  onSort: (key: SortKey) => void;
  onCreateDir: () => void;
  onRename: (row: FileRow) => void;
  onDelete: (rows: FileRow[]) => void;
  onCreateValueChange: (value: string) => void;
  onCreateCommit: () => void;
  onCreateCancel: () => void;
  onRenameValueChange: (value: string) => void;
  onRenameCommit: () => void;
  onRenameCancel: () => void;
  onPathEditStart: () => void;
  onPathEditValueChange: (value: string) => void;
  onPathEditCommit: () => void;
  onPathEditCancel: () => void;
}

type FilePaneScope = CreateEditState['scope'];

interface FilePaneConfig {
  scope: FilePaneScope;
  title: string;
  loadingMessage: string;
  errorTitle: string;
  refreshTitle: string;
  openTitle: string;
  transferLabel: string;
  transferSelectedLabel: (count: number) => string;
  transferIcon: 'upload' | 'download';
  showEnterAction?: boolean;
  onTransfer: (rows: FileRow[]) => void;
}

type InnerFilePaneProps = FilePaneProps & FilePaneConfig;

function FilePane({
  scope,
  title,
  loadingMessage,
  errorTitle,
  refreshTitle,
  openTitle,
  transferLabel,
  transferSelectedLabel,
  transferIcon,
  showEnterAction = false,
  onTransfer,
  path,
  parent,
  rows,
  loading,
  error,
  selectedNames,
  sort,
  createEdit,
  renameEdit,
  pathEdit,
  onSelect,
  onSelectAll,
  onClearSelection,
  onOpenDir,
  onOpenParent,
  onRefresh,
  onSort,
  onCreateDir,
  onRename,
  onDelete,
  onCreateValueChange,
  onCreateCommit,
  onCreateCancel,
  onRenameValueChange,
  onRenameCommit,
  onRenameCancel,
  onPathEditStart,
  onPathEditValueChange,
  onPathEditCommit,
  onPathEditCancel,
}: InnerFilePaneProps) {
  const selectedRows = rows.filter((row) => selectedNames.has(row.name));
  const actionRowsFor = (row: FileRow) =>
    selectedNames.has(row.name) && selectedRows.length > 0 ? selectedRows : [row];
  const buildRowMenuItems = (row: FileRow): SftpMenuItem[] => {
    const actionRows = actionRowsFor(row);
    const items: Array<SftpMenuItem | null> = [
      showEnterAction && row.isDir
        ? {
            label: '进入目录',
            icon: <MenuIcon name="enter" />,
            onSelect: () => {
              if (row.path) onOpenDir(row.path);
            },
          }
        : null,
      {
        label:
          selectedNames.has(row.name) && selectedRows.length > 1
            ? transferSelectedLabel(selectedRows.length)
            : transferLabel,
        icon: <MenuIcon name={transferIcon} />,
        onSelect: () => onTransfer(actionRows),
      },
      {
        label: '新建文件夹',
        icon: <MenuIcon name="folder" />,
        onSelect: onCreateDir,
      },
      {
        label: '重命名',
        icon: <MenuIcon name="rename" />,
        disabled: actionRows.length !== 1,
        onSelect: () => onRename(row),
      },
      {
        label: '复制路径',
        icon: <MenuIcon name="copy" />,
        onSelect: () => copyText(row.path),
      },
      'separator',
      {
        label:
          selectedNames.has(row.name) && selectedRows.length > 1
            ? `删除选中 ${selectedRows.length} 项`
            : '删除',
        icon: <MenuIcon name="delete" />,
        danger: true,
        onSelect: () => onDelete(actionRows),
      },
    ];

    return items.filter((item): item is SftpMenuItem => Boolean(item));
  };

  return (
    <section className={styles.filePane}>
      <div className={styles.paneTop}>
        <span className={styles.paneTitle}>{title}</span>
        {pathEdit?.scope === scope ? (
          <input
            autoFocus
            className={styles.pathInput}
            value={pathEdit.value}
            onChange={(event) => onPathEditValueChange(event.target.value)}
            onBlur={onPathEditCommit}
            onKeyDown={(event) => {
              if (event.key === 'Enter') onPathEditCommit();
              if (event.key === 'Escape') onPathEditCancel();
            }}
          />
        ) : (
          <button type="button" className={styles.pathBox} title={path} onClick={onPathEditStart}>
            {path}
          </button>
        )}
        <button
          type="button"
          className={styles.paneAction}
          title={refreshTitle}
          disabled={loading}
          onClick={onRefresh}
        >
          <RefreshIcon />
        </button>
      </div>

      <SortableTableHeader sort={sort} onSort={onSort} />

      <ContextMenu
        items={[
          {
            label: '新建文件夹',
            icon: <MenuIcon name="folder" />,
            onSelect: onCreateDir,
          },
          {
            label: '刷新',
            onSelect: onRefresh,
          },
        ]}
      >
        <div
          className={styles.fileRows}
          tabIndex={0}
          onClick={(event: MouseEvent<HTMLDivElement>) => {
            if (event.target === event.currentTarget) onClearSelection();
          }}
          onKeyDown={(event: KeyboardEvent<HTMLDivElement>) => {
            if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'a') {
              event.preventDefault();
              onSelectAll();
            }
          }}
        >
          {loading ? (
            <div className={styles.paneMessage}>{loadingMessage}</div>
          ) : error ? (
            <PaneNotice tone="error" title={errorTitle} message={error} />
          ) : rows.length === 0 && !parent ? (
            <div className={styles.paneMessage}>目录为空</div>
          ) : (
            <>
              {parent && (
                <button
                  type="button"
                  className={styles.fileRow}
                  onClick={onOpenParent}
                  title="返回上级目录"
                >
                  <span className={styles.nameCell}>
                    <FileIcon kind="parent" />
                    <span className={`${styles.fileName} ${styles.directoryName}`}>..</span>
                  </span>
                  <span className={styles.sizeCell}>-</span>
                  <span className={styles.timeCell}>上级</span>
                </button>
              )}
              {createEdit?.scope === scope && (
                <div className={styles.fileRow}>
                  <span className={styles.nameCell}>
                    <FileIcon kind="folder" />
                    <input
                      autoFocus
                      className={styles.renameInput}
                      value={createEdit.value}
                      placeholder="新建文件夹"
                      onChange={(event) => onCreateValueChange(event.target.value)}
                      onBlur={onCreateCommit}
                      onClick={(event) => event.stopPropagation()}
                      onKeyDown={(event) => {
                        if (event.key === 'Enter') onCreateCommit();
                        if (event.key === 'Escape') onCreateCancel();
                      }}
                    />
                  </span>
                  <span className={styles.sizeCell}>-</span>
                  <span className={styles.timeCell}>新建</span>
                </div>
              )}
              {rows.map((row) => (
                <ContextMenu key={row.path || row.name} items={buildRowMenuItems(row)}>
                  <button
                    type="button"
                    className={`${styles.fileRow} ${selectedNames.has(row.name) ? styles.selected : ''}`}
                    onClick={(event: MouseEvent<HTMLButtonElement>) =>
                      onSelect(row.name, event.metaKey || event.ctrlKey, event.shiftKey)
                    }
                    onContextMenu={() => {
                      if (!selectedNames.has(row.name)) onSelect(row.name, false, false);
                    }}
                    onDoubleClick={() => {
                      if (row.isDir && row.path) onOpenDir(row.path);
                    }}
                    title={row.isDir ? openTitle : row.name}
                  >
                    <span className={styles.nameCell}>
                      <FileIcon kind={row.kind} />
                      {renameEdit?.scope === scope && renameEdit.row.path === row.path ? (
                        <input
                          autoFocus
                          className={styles.renameInput}
                          value={renameEdit.value}
                          onChange={(event) => onRenameValueChange(event.target.value)}
                          onBlur={onRenameCommit}
                          onClick={(event) => event.stopPropagation()}
                          onDoubleClick={(event) => event.stopPropagation()}
                          onKeyDown={(event) => {
                            if (event.key === 'Enter') onRenameCommit();
                            if (event.key === 'Escape') onRenameCancel();
                          }}
                        />
                      ) : (
                        <span
                          className={`${styles.fileName} ${
                            row.isDir ? styles.directoryName : styles[`fileName-${row.kind}`]
                          }`}
                        >
                          {row.name}
                        </span>
                      )}
                    </span>
                    <span className={styles.sizeCell}>{row.size}</span>
                    <span className={styles.timeCell}>{row.modified}</span>
                  </button>
                </ContextMenu>
              ))}
            </>
          )}
        </div>
      </ContextMenu>
    </section>
  );
}

interface LocalFilePaneProps extends FilePaneProps {
  onUpload: (rows: FileRow[]) => void;
}

export function LocalFilePane({ onUpload, ...props }: LocalFilePaneProps) {
  return (
    <FilePane
      {...props}
      scope="local"
      title="本地"
      loadingMessage="加载本地目录中…"
      errorTitle="本地目录加载失败"
      refreshTitle="刷新本地目录"
      openTitle="双击进入目录"
      transferLabel="上传到远程"
      transferSelectedLabel={(count) => `上传选中 ${count} 项`}
      transferIcon="upload"
      onTransfer={onUpload}
    />
  );
}

interface RemoteFilePaneProps extends FilePaneProps {
  onDownload: (rows: FileRow[]) => void;
}

export function RemoteFilePane({ onDownload, ...props }: RemoteFilePaneProps) {
  return (
    <FilePane
      {...props}
      scope="remote"
      title="远程"
      loadingMessage="加载远程目录中…"
      errorTitle="远程目录加载失败"
      refreshTitle="刷新远程目录"
      openTitle="双击进入远程目录"
      transferLabel="下载到本地"
      transferSelectedLabel={(count) => `下载选中 ${count} 项`}
      transferIcon="download"
      showEnterAction
      onTransfer={onDownload}
    />
  );
}

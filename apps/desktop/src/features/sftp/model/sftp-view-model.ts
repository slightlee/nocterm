import type {
  LocalDirectoryListing,
  LocalFileEntry,
  RemoteDirectoryListing,
  RemoteFileEntry,
} from '../api/sftp-client';
import type { FileKind, FileRow, SortKey, SortState } from '../types/sftp-types';

/** 按扩展名归类文件类型：文件面板与顶部编辑标签共用同一分类规则。 */
export function getFileKindForName(name: string): FileKind {
  if (/\.(tar|gz|zip|rar|7z)$/i.test(name)) return 'archive';
  if (name === '.env' || name.startsWith('.env.') || /\.env$/i.test(name)) return 'env';
  if (/\.(log|out)$/i.test(name)) return 'log';
  if (/\.(sh|ts|tsx|js|jsx|json|rs|py|go|java|yml|yaml|toml|conf|ini|xml|md)$/i.test(name))
    return 'code';
  return 'file';
}

function getFileKind(entry: LocalFileEntry | RemoteFileEntry): FileKind {
  if (entry.isDir) return 'folder';
  return getFileKindForName(entry.name);
}

function isNoctermTransferTempName(name: string): boolean {
  // 传输中的临时文件不展示给用户，避免误删或误判目录内容。
  return (
    /^\..+\.nocterm-(upload|download)-\d+-\d+\.(part|partial)$/.test(name) ||
    /^\..+\.nocterm-[a-f0-9]{32}\.part$/.test(name)
  );
}

function formatSize(size: number | null): string {
  if (size == null) return '-';
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(size < 10 * 1024 ? 1 : 0)} KB`;
  if (size < 1024 * 1024 * 1024) return `${(size / 1024 / 1024).toFixed(1)} MB`;
  return `${(size / 1024 / 1024 / 1024).toFixed(1)} GB`;
}

export function summarizeSelection(rows: FileRow[], selectedNames: Set<string>) {
  const selectedRows = rows.filter((row) => selectedNames.has(row.name));
  if (selectedRows.length === 0) return { count: 0, totalSize: null };
  const fileSizes = selectedRows
    .map((row) => row.sizeBytes)
    .filter((size): size is number => typeof size === 'number');
  const totalSize = fileSizes.length > 0 ? fileSizes.reduce((sum, size) => sum + size, 0) : null;
  return { count: selectedRows.length, totalSize };
}

function formatModified(seconds: number | null): string {
  if (!seconds) return '-';
  const date = new Date(seconds * 1000);
  const pad = (value: number) => value.toString().padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(
    date.getHours()
  )}:${pad(date.getMinutes())}`;
}

export function toLocalRows(listing: LocalDirectoryListing | null): FileRow[] {
  if (!listing) return [];
  return listing.entries
    .filter((entry) => !isNoctermTransferTempName(entry.name))
    .map((entry) => ({
      name: entry.name,
      kind: getFileKind(entry),
      size: formatSize(entry.size),
      sizeBytes: entry.size,
      modified: formatModified(entry.modifiedAt),
      modifiedAt: entry.modifiedAt,
      path: entry.path,
      isDir: entry.isDir,
    }));
}

export function toRemoteRows(listing: RemoteDirectoryListing | null): FileRow[] {
  if (!listing) return [];
  return listing.entries
    .filter((entry) => !isNoctermTransferTempName(entry.name))
    .map((entry) => ({
      name: entry.name,
      kind: getFileKind(entry),
      size: formatSize(entry.size),
      sizeBytes: entry.size,
      modified: formatModified(entry.modifiedAt),
      modifiedAt: entry.modifiedAt,
      path: `${listing.path.replace(/\/$/, '')}/${entry.name}`,
      isDir: entry.isDir,
    }));
}

export function sortRows(rows: FileRow[], sort: SortState): FileRow[] {
  const direction = sort.direction === 'asc' ? 1 : -1;
  return [...rows].sort((a, b) => {
    const directoryOrder = Number(Boolean(b.isDir)) - Number(Boolean(a.isDir));
    // 名称和大小排序沿用文件管理器习惯：目录固定在文件前面。
    // 修改时间排序则以时间为第一关键字，不能让目录分组覆盖用户选择的排序列。
    if (sort.key !== 'modified' && directoryOrder !== 0) return directoryOrder;

    if (sort.key === 'size') {
      const left = a.sizeBytes ?? -1;
      const right = b.sizeBytes ?? -1;
      if (left !== right) return (left - right) * direction;
    }

    if (sort.key === 'modified') {
      // 缺失时间始终排在已知时间之后，避免升序时“-”条目跑到最前面。
      if (a.modifiedAt === null && b.modifiedAt !== null) return 1;
      if (a.modifiedAt !== null && b.modifiedAt === null) return -1;
      if (a.modifiedAt !== b.modifiedAt) {
        return ((a.modifiedAt ?? 0) - (b.modifiedAt ?? 0)) * direction;
      }
      if (directoryOrder !== 0) return directoryOrder;
    }

    return a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }) * direction;
  });
}

export function nextSortState(current: SortState, key: SortKey): SortState {
  if (current.key !== key) return { key, direction: key === 'name' ? 'asc' : 'desc' };
  return { key, direction: current.direction === 'asc' ? 'desc' : 'asc' };
}

export function selectRange(
  rows: FileRow[],
  fromName: string | null,
  toName: string
): Set<string> | null {
  // Shift 多选依赖上一次选中的文件名，排序或刷新后找不到时放弃范围选择。
  if (!fromName) return null;
  const fromIndex = rows.findIndex((row) => row.name === fromName);
  const toIndex = rows.findIndex((row) => row.name === toName);
  if (fromIndex < 0 || toIndex < 0) return null;
  const [start, end] = fromIndex < toIndex ? [fromIndex, toIndex] : [toIndex, fromIndex];
  return new Set(rows.slice(start, end + 1).map((row) => row.name));
}

export function joinPath(parent: string, name: string): string {
  if (parent === '/') return `/${name}`;
  return `${parent.replace(/\/$/, '')}/${name}`;
}

/** 在线编辑的字节上限，与 Rust 命令层 `SFTP_EDIT_MAX_BYTES`（2 MiB）保持一致。 */
export const SFTP_EDIT_MAX_BYTES = 2 * 1024 * 1024;

/**
 * 判断远程条目能否进入在线编辑：仅普通文件可编辑，已知大小时不得超过编辑上限。
 * 大小未知（服务端未报告）时放行——后端读取时会再做权威校验并给出稳定错误码。
 */
export function canEditRemoteEntry(entry: Pick<FileRow, 'isDir' | 'sizeBytes'>): boolean {
  if (entry.isDir) return false;
  if (entry.sizeBytes == null) return true;
  return entry.sizeBytes <= SFTP_EDIT_MAX_BYTES;
}

/** 计算远程文件路径的父目录；根下文件返回 `/`，无父目录的裸路径返回 null。 */
export function remoteParentPath(path: string): string | null {
  const trimmed = path.replace(/\/+$/, '');
  const index = trimmed.lastIndexOf('/');
  if (index <= 0) return index === 0 ? '/' : null;
  return trimmed.slice(0, index);
}

/**
 * 判断 IPC 错误是否为保存冲突（远端文件基线校验未通过）。依赖命令级稳定错误码，
 * 不得解析消息文本——编辑器据此区分「弹冲突三选」与「普通保存失败」。
 */
export function isRemoteChangedError(err: unknown): boolean {
  return (
    typeof err === 'object' &&
    err !== null &&
    (err as { code?: unknown }).code === 'SFTP_REMOTE_CHANGED'
  );
}

export function validateNameInput(value: string | null): { name: string | null; error?: string } {
  const name = value?.trim();
  if (!name) return { name: null };
  if (name === '.' || name === '..' || /[/\\\t\r\n]/.test(name)) {
    return { name: null, error: '名称不能是相对路径，也不能包含路径分隔符、Tab 或换行符' };
  }
  return { name };
}

export function getErrorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

import type {
  LocalDirectoryListing,
  LocalFileEntry,
  RemoteDirectoryListing,
  RemoteFileEntry,
} from '../api/sftp-client';
import type { FileKind, FileRow, SortKey, SortState } from '../types/sftp-types';

function getFileKind(entry: LocalFileEntry | RemoteFileEntry): FileKind {
  if (entry.isDir) return 'folder';
  if (/\.(tar|gz|zip|rar|7z)$/i.test(entry.name)) return 'archive';
  if (entry.name === '.env' || /\.env$/i.test(entry.name)) return 'env';
  if (/\.(sh|ts|tsx|js|jsx|json|rs|py|go|java)$/i.test(entry.name)) return 'code';
  if (/\.log$/i.test(entry.name)) return 'log';
  return 'file';
}

function isNoctermTransferTempName(name: string): boolean {
  // 传输中的临时文件不展示给用户，避免误删或误判目录内容。
  return /^\..+\.nocterm-(upload|download)-\d+-\d+\.(part|partial)$/.test(name);
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
    if (directoryOrder !== 0) return directoryOrder;

    if (sort.key === 'size') {
      const left = a.sizeBytes ?? -1;
      const right = b.sizeBytes ?? -1;
      if (left !== right) return (left - right) * direction;
    }

    if (sort.key === 'modified') {
      const left = a.modifiedAt ?? 0;
      const right = b.modifiedAt ?? 0;
      if (left !== right) return (left - right) * direction;
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

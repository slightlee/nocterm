export type FileKind = 'parent' | 'folder' | 'archive' | 'env' | 'code' | 'log' | 'file';

export interface FileRow {
  name: string;
  kind: FileKind;
  size: string;
  sizeBytes: number | null;
  modified: string;
  modifiedAt: number | null;
  path?: string;
  isDir?: boolean;
}

export type SortKey = 'name' | 'size' | 'modified';
export type SortDirection = 'asc' | 'desc';

export type SortState = {
  key: SortKey;
  direction: SortDirection;
};

export type RenameEditState = {
  scope: 'local' | 'remote';
  row: FileRow;
  value: string;
};

export type CreateEditState = {
  scope: 'local' | 'remote';
  value: string;
};

export type PathEditState = {
  scope: 'local' | 'remote';
  value: string;
};

export type ToastState = {
  id: number;
  tone: 'error' | 'info' | 'success';
  message: string;
};

export type ConfirmState = {
  title: string;
  message: string;
  confirmLabel?: string;
  danger?: boolean;
  resolve: (confirmed: boolean) => void;
};

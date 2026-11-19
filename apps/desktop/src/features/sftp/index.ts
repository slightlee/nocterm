export { default as SftpView } from './components/SftpView';
export { FileEditorCloseGuard } from './components/FileEditorCloseGuard';
export { FileIcon } from './components/SftpFilePanes';
export { useSftpStore } from './model/sftp-store';
export { useSftpFeedback } from './model/useSftpFeedback';
export { useFileEditorStore } from './model/file-editor-store';
export {
  beginFileEditorClose,
  withFileEditorClose,
  withFileEditorTargetChange,
  remoteEditorIds,
} from './model/file-editor-close';
export type { FileEditorTab, FileEditorSource } from './model/file-editor-store';
export type { SftpSession } from './model/sftp-store';
export { getFileKindForName } from './model/sftp-view-model';
export type { FileKind } from './types/sftp-types';
export {
  cancelFileTransfer,
  disconnectSftpSession,
  onFileTransferProgress,
} from './api/sftp-client';
export type { FileTransferProgress } from './api/sftp-client';
export { getSftpErrorMessage } from './model/sftp-error';

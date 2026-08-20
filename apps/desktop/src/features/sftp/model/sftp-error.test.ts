import { describe, expect, it } from 'vitest';

import { getSftpErrorMessage } from './sftp-error';

describe('sftp error messages', () => {
  it('parses the stable SSH classification embedded in a Tauri error', () => {
    expect(
      getSftpErrorMessage({
        code: 'SFTP_REMOTE_READ_FAILED',
        message: '__NOCTERM_SFTP_ERROR__\tauthFailed\t',
      })
    ).toContain('认证失败');
  });

  it('explains missing local credentials instead of showing a generic read failure', () => {
    expect(
      getSftpErrorMessage({ code: 'CREDENTIAL_READ_FAILED', message: '读取系统凭据失败' })
    ).toContain('本机凭据缺失');
  });

  it('explains that a failed local commit preserves the original target', () => {
    expect(getSftpErrorMessage('__NOCTERM_SFTP_ERROR__\tlocalWriteFailed\t')).toContain(
      '原目标已保留'
    );
  });

  it('distinguishes local upload read failures from remote errors', () => {
    expect(getSftpErrorMessage('__NOCTERM_SFTP_ERROR__\tlocalReadFailed\t')).toContain(
      '读取本地文件失败'
    );
  });
});

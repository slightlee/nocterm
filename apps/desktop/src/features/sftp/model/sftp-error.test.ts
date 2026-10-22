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

  it('maps local directory timeout and permission errors to plain retryable messages', () => {
    expect(
      getSftpErrorMessage({ code: 'SFTP_LOCAL_READ_TIMEOUT', message: '读取本地目录超时' })
    ).toContain('稍后重试');
    expect(
      getSftpErrorMessage({
        code: 'SFTP_LOCAL_PERMISSION_DENIED',
        message: '当前应用没有访问该本地目录的权限',
      })
    ).toContain('检查目录权限');
  });
});

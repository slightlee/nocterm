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

  it('explains the online editing rejection reasons', () => {
    expect(
      getSftpErrorMessage({ code: 'SFTP_FILE_TOO_LARGE', message: '编辑内容超过在线编辑上限' })
    ).toContain('在线编辑上限');
    expect(
      getSftpErrorMessage({ code: 'SFTP_FILE_NOT_TEXT', message: '文件是二进制内容' })
    ).toContain('不支持在线编辑');
  });

  it('explains a remote conflict raised by the editor save path', () => {
    expect(
      getSftpErrorMessage({ code: 'SFTP_REMOTE_CHANGED', message: '远端文件已被修改或删除' })
    ).toContain('已被修改或删除');
  });

  it('translates an oversized remote file reported by the infrastructure layer', () => {
    expect(
      getSftpErrorMessage({
        code: 'SFTP_REMOTE_READ_FAILED',
        message: '__NOCTERM_SFTP_ERROR__\tfileTooLarge\t',
      })
    ).toContain('在线编辑上限');
  });

  it('distinguishes open-time file changes from save conflicts', () => {
    expect(
      getSftpErrorMessage({ code: 'SFTP_LOCAL_FILE_CHANGED', message: '本地文件已被修改' })
    ).toContain('已被修改或删除');
    expect(
      getSftpErrorMessage({ code: 'SFTP_LOCAL_READ_CHANGED', message: '读取期间发生变化' })
    ).toContain('打开期间发生变化');
    expect(
      getSftpErrorMessage({
        code: 'SFTP_REMOTE_READ_FAILED',
        message: '__NOCTERM_SFTP_ERROR__\tremoteReadChanged\t',
      })
    ).toContain('打开期间发生变化');
    expect(
      getSftpErrorMessage({ code: 'SFTP_LOCAL_FILE_SYMLINK', message: '目标是符号链接' })
    ).toContain('符号链接');
    // 磁盘满或暂存提交失败必须提示本地写入，不得误报为远端权限不足。
    expect(getSftpErrorMessage({ code: 'SFTP_LOCAL_WRITE_FAILED', message: '保存失败' })).toContain(
      '写入本地文件失败'
    );
  });
});

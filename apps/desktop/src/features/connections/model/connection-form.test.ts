import { describe, expect, it } from 'vitest';

import {
  DEFAULT_CONNECTION_FORM,
  toConnectionCreateRequest,
  validateConnectionForm,
} from './connection-form';

describe('connection form', () => {
  it('reports every invalid required field', () => {
    expect(
      validateConnectionForm({
        ...DEFAULT_CONNECTION_FORM,
        name: ' ',
        host: '',
        port: '70000',
        username: ' ',
      })
    ).toEqual({
      name: '请输入连接名称',
      host: '请输入主机地址',
      port: '端口必须在 1 到 65535 之间',
      username: '请输入用户名',
    });
  });

  it('normalizes a valid request', () => {
    expect(
      toConnectionCreateRequest({
        name: ' Production ',
        host: ' server.example.com ',
        port: '2222',
        username: ' deploy ',
        authentication: 'private_key',
      })
    ).toEqual({
      name: 'Production',
      host: 'server.example.com',
      port: 2222,
      username: 'deploy',
      authentication: 'private_key',
    });
  });

  it('normalizes optional remote path and remark', () => {
    expect(
      toConnectionCreateRequest({
        ...DEFAULT_CONNECTION_FORM,
        remoteInitialPath: ' /srv/app ',
        remark: ' production host ',
      })
    ).toMatchObject({
      remoteInitialPath: '/srv/app',
      remark: 'production host',
    });
  });

  it('rejects SSH option-like host and whitespace username', () => {
    expect(
      validateConnectionForm({
        ...DEFAULT_CONNECTION_FORM,
        name: 'Production',
        host: '-oProxyCommand=bad',
        username: 'deploy user',
      })
    ).toEqual({
      host: '主机地址不能包含空白字符或以短横线开头',
      username: '用户名不能包含空白字符或以短横线开头',
    });
  });
});

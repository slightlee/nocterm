import { describe, expect, it } from 'vitest';

import { createConnectionBackup, parseConnectionBackup } from './connection-backup';
import type { ConnectionProfile } from '../types/connection-types';

const connection: ConnectionProfile = {
  id: 7,
  name: 'Production',
  host: 'prod.example.com',
  port: 22,
  username: 'deploy',
  authentication: 'private_key',
  createdAt: 1,
  updatedAt: 2,
  groupId: 'production',
  groupName: '生产环境',
  sortOrder: 10,
  credentialKind: 'private_key',
  credentialStatus: 'bound',
};

describe('connection backup', () => {
  it('round trips profiles and groups without serializing secrets or local paths', () => {
    const content = createConnectionBackup(
      [connection],
      [{ id: 'production', name: '生产环境', sortOrder: 0 }]
    );
    const imported = parseConnectionBackup(content);

    expect(imported.groups).toEqual([{ id: 'production', name: '生产环境', sortOrder: 0 }]);
    expect(imported.connections[0]).toMatchObject({
      sourceId: '7',
      authentication: 'private_key',
      credentialStatus: 'bound',
    });
    expect(content).not.toContain('privateKeyPath');
    expect(content).not.toContain('password');
    expect(content).not.toContain('secretRef');
  });

  it('rejects invalid files before returning import data', () => {
    expect(() => parseConnectionBackup('{')).toThrow('备份文件不是有效的 JSON');
    expect(() => parseConnectionBackup('{"schema":"other","version":1}')).toThrow(
      '备份文件格式不受支持'
    );
  });
});

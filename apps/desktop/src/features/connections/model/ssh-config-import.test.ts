import { describe, expect, it } from 'vitest';

import { parseSshConfigConnections } from './ssh-config-import';

describe('parseSshConfigConnections', () => {
  it('imports concrete Host blocks and maps common OpenSSH options', () => {
    const profiles = parseSshConfigConnections(
      `Host production staging\n  HostName prod.example.com\n  User deploy\n  Port 2202\n  IdentityFile ~/.ssh/id_ed25519 # primary key\n\nHost *\n  User ignored`,
      'ssh-config',
      '/Users/test/.ssh/config',
      'local-user'
    );

    expect(profiles).toEqual([
      expect.objectContaining({
        name: 'production',
        host: 'prod.example.com',
        username: 'deploy',
        port: 2202,
        authentication: 'private_key',
        privateKeyPath: '~/.ssh/id_ed25519',
      }),
      expect.objectContaining({ name: 'staging', host: 'prod.example.com' }),
    ]);
  });

  it('ignores wildcard hosts and does not silently use root', () => {
    const profiles = parseSshConfigConnections(
      'Host * !excluded\n  User deploy\nHost internal\n  HostName internal.example.com',
      'ssh-config',
      '/Users/test/.ssh/config',
      'local-user'
    );

    expect(profiles).toEqual([
      expect.objectContaining({ name: 'internal', username: 'local-user', port: 22 }),
    ]);
  });
});

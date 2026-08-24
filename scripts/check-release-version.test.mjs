import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import {
  VERSION_FILES,
  compareReleaseVersions,
  findLatestReleaseVersion,
  findChangedVersionFiles,
  parseReleaseVersion,
  validateReleaseCommit,
  validateReleaseProgression,
  validateVersionEntries,
  validateVersionSourceConfiguration,
} from './check-release-version.mjs';

describe('release version rules', () => {
  it('accepts stable and numbered prerelease versions', () => {
    assert.equal(parseReleaseVersion('0.1.0')?.channel, 'stable');
    assert.equal(parseReleaseVersion('0.1.0-beta.2')?.sequence, 2);
  });

  it('rejects unsupported channels, missing sequences and build metadata', () => {
    assert.equal(parseReleaseVersion('0.1.0-preview.1'), null);
    assert.equal(parseReleaseVersion('0.1.0-beta'), null);
    assert.equal(parseReleaseVersion('0.1.0-beta.0'), null);
    assert.equal(parseReleaseVersion('0.1.0+build.1'), null);
  });

  it('orders prerelease channels and stable releases correctly', () => {
    assert.ok(compareReleaseVersions('0.1.0-alpha.2', '0.1.0-beta.1') < 0);
    assert.ok(compareReleaseVersions('0.1.0-beta.3', '0.1.0-rc.1') < 0);
    assert.ok(compareReleaseVersions('0.1.0-rc.1', '0.1.0') < 0);
    assert.ok(compareReleaseVersions('0.2.0-alpha.1', '0.1.0') > 0);
  });

  it('allows the first beta when no release tag exists', () => {
    assert.equal(findLatestReleaseVersion([]), undefined);
    assert.deepEqual(validateReleaseProgression('0.1.0-beta.1', []), []);
  });

  it('uses the latest valid release tag as the version baseline', () => {
    const tags = ['experiment', 'v0.1.0-alpha.2', 'v0.1.0-beta.1', 'vnext'];

    assert.equal(findLatestReleaseVersion(tags), '0.1.0-beta.1');
    assert.deepEqual(validateReleaseProgression('0.1.0-beta.2', tags), []);
  });

  it('rejects versions that do not advance beyond the latest release tag', () => {
    const tags = ['v0.1.0-beta.1', 'v0.1.0'];

    assert.match(validateReleaseProgression('0.1.0-beta.2', tags)[0], /0\.1\.0/);
    assert.match(validateReleaseProgression('0.1.0', tags)[0], /0\.1\.0/);
  });

  it('uses the root package as the only product version source', () => {
    assert.deepEqual(VERSION_FILES, ['package.json']);
  });

  it('ignores non-version changes inside a version source file', () => {
    const before = VERSION_FILES.map((file) => [file, '0.1.0']);
    const unchanged = before.map(([file, version]) => [file, version]);
    const changed = before.map(([file]) => [file, '0.1.1']);

    assert.deepEqual(findChangedVersionFiles(before, unchanged), []);
    assert.deepEqual(findChangedVersionFiles(before, changed), [VERSION_FILES[0]]);
  });

  it('validates the canonical product version', () => {
    assert.deepEqual(validateVersionEntries([['package.json', '0.1.0-beta.1']]), []);
    assert.match(validateVersionEntries([['package.json', '0.1.0-preview.1']])[0], /不符合/);
  });

  it('requires Tauri to reference the canonical version source', () => {
    const files = new Map([
      ['apps/desktop/package.json', '{"name":"@nocterm/desktop","private":true}'],
      ['apps/desktop/src-tauri/tauri.conf.json', '{"version":"../../../package.json"}'],
    ]);

    assert.deepEqual(
      validateVersionSourceConfiguration((file) => files.get(file)),
      []
    );

    files.set('apps/desktop/package.json', '{"version":"0.1.0"}');
    files.set('apps/desktop/src-tauri/tauri.conf.json', '{"version":"0.1.0"}');
    const errors = validateVersionSourceConfiguration((file) => files.get(file));
    assert.equal(errors.length, 2);
  });

  it('binds release commit messages to the staged version', () => {
    assert.deepEqual(
      validateReleaseCommit('chore: prepare v0.1.0-beta.1', VERSION_FILES, '0.1.0-beta.1'),
      []
    );
    assert.match(
      validateReleaseCommit('chore: bump version', VERSION_FILES, '0.1.0-beta.1')[0],
      /提交信息必须/
    );
    assert.match(
      validateReleaseCommit('chore: prepare v0.1.0-beta.1', [], '0.1.0-beta.1')[0],
      /实际修改/
    );
    assert.match(
      validateReleaseCommit(
        'chore(release): prepare v0.1.0-beta.1',
        VERSION_FILES,
        '0.1.0-beta.1'
      )[0],
      /提交信息必须/
    );
  });
});

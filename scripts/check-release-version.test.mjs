/* global process */
import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { describe, it } from 'node:test';
import { fileURLToPath } from 'node:url';

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

const RELEASE_CHECK_SCRIPT = join(
  dirname(fileURLToPath(import.meta.url)),
  'check-release-version.mjs'
);

function runGit(cwd, args) {
  return execFileSync('git', args, { cwd, encoding: 'utf8' }).trim();
}

function writeJson(path, value) {
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}

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
    assert.deepEqual(
      validateReleaseCommit('chore: prepare v0.1.0-beta.1 (#17)', VERSION_FILES, '0.1.0-beta.1'),
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
    for (const subject of [
      'chore: prepare v0.1.0-beta.1 (#0)',
      'chore: prepare v0.1.0-beta.1 (#017)',
      'chore: prepare v0.1.0-beta.1 (PR #17)',
      'chore: prepare v0.1.0-beta.1 extra',
    ]) {
      assert.match(
        validateReleaseCommit(subject, VERSION_FILES, '0.1.0-beta.1')[0],
        /提交信息必须/
      );
    }
  });

  it('validates an immutable tag with the latest checker from another ref', () => {
    const repository = mkdtempSync(join(tmpdir(), 'nocterm-release-tag-check-'));

    try {
      mkdirSync(join(repository, 'apps/desktop/src-tauri'), { recursive: true });
      writeJson(join(repository, 'package.json'), {
        name: 'nocterm',
        version: '0.1.0-beta.2',
      });
      writeJson(join(repository, 'apps/desktop/package.json'), {
        name: '@nocterm/desktop',
        private: true,
      });
      writeJson(join(repository, 'apps/desktop/src-tauri/tauri.conf.json'), {
        version: '../../../package.json',
      });

      runGit(repository, ['init', '--initial-branch=main']);
      runGit(repository, ['config', 'user.name', 'Nocterm Test']);
      runGit(repository, ['config', 'user.email', 'test@nocterm.local']);
      runGit(repository, ['add', '.']);
      runGit(repository, ['commit', '-m', 'chore: prepare v0.1.0-beta.2 (#17)']);
      const releaseCommit = runGit(repository, ['rev-parse', 'HEAD']);
      runGit(repository, ['tag', '-a', 'v0.1.0-beta.2', '-m', 'Release v0.1.0-beta.2']);

      writeFileSync(join(repository, 'README.md'), 'latest validation tooling\n');
      runGit(repository, ['add', 'README.md']);
      runGit(repository, ['commit', '-m', 'ci: update release validation']);

      const result = spawnSync(
        process.execPath,
        [RELEASE_CHECK_SCRIPT, '--tag', 'v0.1.0-beta.2', '--head', releaseCommit],
        { cwd: repository, encoding: 'utf8' }
      );

      assert.equal(result.status, 0, result.stderr);
      assert.match(result.stdout, /Release version check passed: 0\.1\.0-beta\.2/);
    } finally {
      rmSync(repository, { recursive: true, force: true });
    }
  });

  it('validates the pull request head instead of a synthetic merge commit', () => {
    const repository = mkdtempSync(join(tmpdir(), 'nocterm-release-check-'));

    try {
      mkdirSync(join(repository, 'apps/desktop/src-tauri'), { recursive: true });
      writeJson(join(repository, 'package.json'), { name: 'nocterm', version: '0.1.0' });
      writeJson(join(repository, 'apps/desktop/package.json'), {
        name: '@nocterm/desktop',
        private: true,
      });
      writeJson(join(repository, 'apps/desktop/src-tauri/tauri.conf.json'), {
        version: '../../../package.json',
      });

      runGit(repository, ['init', '--initial-branch=main']);
      runGit(repository, ['config', 'user.name', 'Nocterm Test']);
      runGit(repository, ['config', 'user.email', 'test@nocterm.local']);
      runGit(repository, ['add', '.']);
      runGit(repository, ['commit', '-m', 'chore: initialize release fixture']);
      const base = runGit(repository, ['rev-parse', 'HEAD']);

      runGit(repository, ['switch', '-c', 'release']);
      writeJson(join(repository, 'package.json'), {
        name: 'nocterm',
        version: '0.1.0-beta.1',
      });
      runGit(repository, ['add', 'package.json']);
      runGit(repository, ['commit', '-m', 'chore: prepare v0.1.0-beta.1']);
      const releaseHead = runGit(repository, ['rev-parse', 'HEAD']);

      runGit(repository, ['switch', 'main']);
      runGit(repository, ['merge', '--no-ff', 'release', '-m', 'Merge release pull request']);

      const result = spawnSync(
        process.execPath,
        [RELEASE_CHECK_SCRIPT, '--base', base, '--head', releaseHead],
        { cwd: repository, encoding: 'utf8' }
      );

      assert.equal(result.status, 0, result.stderr);
      assert.match(result.stdout, /Release version check passed/);
    } finally {
      rmSync(repository, { recursive: true, force: true });
    }
  });
});

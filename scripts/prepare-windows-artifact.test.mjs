import assert from 'node:assert/strict';
import { Buffer } from 'node:buffer';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, it } from 'node:test';

import {
  buildWindowsArtifactName,
  buildWindowsReleaseArtifact,
} from './prepare-windows-artifact.mjs';

describe('Windows release artifact', () => {
  it('uses the documented platform and architecture name', () => {
    assert.equal(
      buildWindowsArtifactName('0.1.0-beta.2', 'x64'),
      'Nocterm_0.1.0-beta.2_windows_x86_64-setup.exe'
    );
  });

  it('rejects invalid versions and unsupported architectures', () => {
    assert.throws(() => buildWindowsArtifactName('0.1.0-preview.1', 'x64'), /无效的产品版本/);
    assert.throws(() => buildWindowsArtifactName('0.1.0-beta.2', 'arm64'), /只支持 Windows x64/);
  });

  it('removes stale outputs before a failed build', () => {
    const directory = mkdtempSync(join(tmpdir(), 'nocterm-windows-artifact-'));
    const sourceDirectory = join(directory, 'target', 'release', 'bundle', 'nsis');
    const outputDirectory = join(directory, 'target', 'release', 'artifacts');
    const sourcePath = join(sourceDirectory, 'Nocterm_0.1.0-beta.2_x64-setup.exe');
    const artifactPath = join(outputDirectory, 'Nocterm_0.1.0-beta.2_windows_x86_64-setup.exe');
    const unrelatedPath = join(sourceDirectory, 'keep-me.exe');

    try {
      mkdirSync(sourceDirectory, { recursive: true });
      mkdirSync(outputDirectory, { recursive: true });
      for (const path of [sourcePath, artifactPath, `${artifactPath}.sha256`, unrelatedPath]) {
        writeFileSync(path, 'stale');
      }

      assert.throws(
        () =>
          buildWindowsReleaseArtifact({
            repositoryRoot: directory,
            version: '0.1.0-beta.2',
            architecture: 'x64',
            runBuild: ({ sourcePath: currentSourcePath }) => {
              writeFileSync(currentSourcePath, 'partial');
              throw new Error('build failed');
            },
          }),
        /build failed/
      );

      assert.equal(existsSync(sourcePath), false);
      assert.equal(existsSync(artifactPath), false);
      assert.equal(existsSync(`${artifactPath}.sha256`), false);
      assert.equal(existsSync(unrelatedPath), true);
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });

  it('packages only the installer produced by the current build', () => {
    const directory = mkdtempSync(join(tmpdir(), 'nocterm-windows-artifact-'));
    const freshContent = Buffer.from('fresh nocterm installer fixture');

    try {
      const result = buildWindowsReleaseArtifact({
        repositoryRoot: directory,
        version: '0.1.0-beta.2',
        architecture: 'x64',
        runBuild: ({ sourcePath }) => {
          mkdirSync(join(directory, 'target', 'release', 'bundle', 'nsis'), {
            recursive: true,
          });
          writeFileSync(sourcePath, freshContent);
        },
      });
      const expectedChecksum = createHash('sha256').update(freshContent).digest('hex');

      assert.deepEqual(readFileSync(result.artifactPath), freshContent);
      assert.equal(result.sourceSize, freshContent.length);
      assert.equal(result.checksum, expectedChecksum);
      assert.equal(
        readFileSync(result.checksumPath, 'utf8'),
        `${expectedChecksum}  ${result.artifactName}\n`
      );
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });
});

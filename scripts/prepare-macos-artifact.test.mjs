import assert from 'node:assert/strict';
import { Buffer } from 'node:buffer';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, it } from 'node:test';

import {
  buildMacosArtifactName,
  buildMacosDmg,
  buildMacosReleaseArtifact,
} from './prepare-macos-artifact.mjs';

describe('macOS release artifact', () => {
  it('writes Finder layout in CI without clearing the CI environment', () => {
    const environment = Object.freeze({
      CI: 'true',
      PATH: '/usr/bin',
      TAURI_BUNDLER_DMG_IGNORE_CI: 'false',
    });
    const steps = [];

    buildMacosDmg({
      pnpmCli: '/pnpm/cli.mjs',
      environment,
      runStep: (args, env) => steps.push({ args, env }),
    });

    assert.equal(steps.length, 2);
    assert.deepEqual(steps[0], {
      args: ['/pnpm/cli.mjs', '--filter', '@nocterm/desktop', 'build'],
      env: environment,
    });
    const { args, env } = steps[1];
    assert.deepEqual(args.slice(0, 6), [
      '/pnpm/cli.mjs',
      '--filter',
      '@nocterm/desktop',
      'exec',
      'tauri',
      'build',
    ]);
    assert.equal(args[args.indexOf('--bundles') + 1], 'dmg');
    assert.ok(args.includes('--ci'));
    assert.equal(JSON.parse(args[args.indexOf('--config') + 1]).build.beforeBuildCommand, '');
    assert.deepEqual(env, {
      ...environment,
      MACOSX_DEPLOYMENT_TARGET: '14.0',
      TAURI_BUNDLER_DMG_IGNORE_CI: 'true',
    });
    assert.equal(environment.TAURI_BUNDLER_DMG_IGNORE_CI, 'false');
  });

  it('also writes Finder layout locally and preserves an explicit deployment target', () => {
    const environment = Object.freeze({ MACOSX_DEPLOYMENT_TARGET: '15.0' });
    const steps = [];

    buildMacosDmg({
      pnpmCli: '/pnpm/cli.mjs',
      environment,
      runStep: (args, env) => steps.push({ args, env }),
    });

    assert.deepEqual(steps[1].env, {
      MACOSX_DEPLOYMENT_TARGET: '15.0',
      TAURI_BUNDLER_DMG_IGNORE_CI: 'true',
    });
  });

  it('stops before bundling when the frontend build fails', () => {
    const steps = [];

    assert.throws(
      () =>
        buildMacosDmg({
          pnpmCli: '/pnpm/cli.mjs',
          environment: {},
          runStep: (args) => {
            steps.push(args);
            throw new Error('frontend build failed');
          },
        }),
      /frontend build failed/
    );
    assert.equal(steps.length, 1);
  });

  it('uses the documented platform and architecture name', () => {
    assert.equal(
      buildMacosArtifactName('0.1.0-beta.2', 'arm64'),
      'Nocterm_0.1.0-beta.2_macos_aarch64.dmg'
    );
    assert.equal(
      buildMacosArtifactName('0.1.0-beta.2', 'x64'),
      'Nocterm_0.1.0-beta.2_macos_x86_64.dmg'
    );
  });

  it('rejects invalid versions and unsupported architectures', () => {
    assert.throws(() => buildMacosArtifactName('0.1.0-preview.1', 'arm64'), /无效的产品版本/);
    assert.throws(() => buildMacosArtifactName('0.1.0-beta.2', 'ppc64'), /只支持 macOS/);
  });

  it('removes stale outputs before a failed build', () => {
    const directory = mkdtempSync(join(tmpdir(), 'nocterm-macos-artifact-'));
    const sourceDirectory = join(directory, 'target', 'release', 'bundle', 'dmg');
    const outputDirectory = join(directory, 'target', 'release', 'artifacts');
    const sourcePath = join(sourceDirectory, 'Nocterm_0.1.0-beta.2_aarch64.dmg');
    const artifactPath = join(outputDirectory, 'Nocterm_0.1.0-beta.2_macos_aarch64.dmg');
    const unrelatedPath = join(sourceDirectory, 'keep-me.dmg');

    try {
      mkdirSync(sourceDirectory, { recursive: true });
      mkdirSync(outputDirectory, { recursive: true });
      for (const path of [sourcePath, artifactPath, `${artifactPath}.sha256`, unrelatedPath]) {
        writeFileSync(path, 'stale');
      }

      assert.throws(
        () =>
          buildMacosReleaseArtifact({
            repositoryRoot: directory,
            version: '0.1.0-beta.2',
            architecture: 'arm64',
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
    const directory = mkdtempSync(join(tmpdir(), 'nocterm-macos-artifact-'));
    const freshContent = Buffer.from('fresh nocterm dmg fixture');

    try {
      const result = buildMacosReleaseArtifact({
        repositoryRoot: directory,
        version: '0.1.0-beta.2',
        // x64 路径同时覆盖 Tauri 源命名到发布命名的映射。
        architecture: 'x64',
        runBuild: ({ sourcePath }) => {
          mkdirSync(join(directory, 'target', 'release', 'bundle', 'dmg'), {
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

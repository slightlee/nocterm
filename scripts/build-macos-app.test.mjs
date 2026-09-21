import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { assertMacosHost, buildAppTauriArguments, buildMacosApp } from './build-macos-app.mjs';

describe('build-macos-app', () => {
  it('uses app-only bundling with skipped beforeBuildCommand', () => {
    const args = buildAppTauriArguments();
    const joined = args.join(' ');
    assert.match(joined, /--bundles app/);
    assert.match(joined, /"beforeBuildCommand":""/);
    assert.ok(args.includes('--ci'));
    // 不传 --target：只在宿主原生架构构建，与发布脚本的架构约束一致。
    assert.ok(!args.includes('--target'));
  });

  it('builds frontend before the .app bundle via the given pnpm CLI', () => {
    const steps = [];
    buildMacosApp({ pnpmCli: '/pnpm/cli.mjs', runStep: (args) => steps.push(args) });
    assert.equal(steps.length, 2);
    assert.ok(steps[0].includes('build') && !steps[0].includes('tauri'));
    assert.ok(steps[1].includes('tauri'));
    assert.ok(steps.every((args) => args[0] === '/pnpm/cli.mjs'));
  });

  it('rejects missing pnpm CLI and non-macOS hosts', () => {
    assert.throws(() => buildMacosApp({ pnpmCli: '', runStep: () => {} }), /pnpm CLI/);
    assert.throws(() => assertMacosHost('win32'), /macOS/);
    assert.doesNotThrow(() => assertMacosHost('darwin'));
  });
});

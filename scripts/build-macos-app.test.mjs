import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, it } from 'node:test';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { assertMacosHost, buildAppTauriArguments, buildMacosApp } from './build-macos-app.mjs';

const REPOSITORY_ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');

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

  it('bundles Simplified Chinese localization for AppKit-owned menu items', () => {
    const tauriRoot = join(REPOSITORY_ROOT, 'apps', 'desktop', 'src-tauri');
    const config = JSON.parse(readFileSync(join(tauriRoot, 'tauri.conf.json'), 'utf8'));
    const infoPlist = readFileSync(join(tauriRoot, 'Info.plist'), 'utf8');
    const localization = 'resources/zh-Hans.lproj/InfoPlist.strings';

    assert.match(infoPlist, /<key>CFBundleDevelopmentRegion<\/key>\s*<string>zh-Hans<\/string>/);
    assert.equal(
      config.bundle.macOS.files[localization],
      'Resources/zh-Hans.lproj/InfoPlist.strings'
    );
    assert.match(readFileSync(join(tauriRoot, localization), 'utf8'), /CFBundleDisplayName/);
  });

  it('keeps the native Edit and Window menus limited to product-owned items', () => {
    const menuSource = readFileSync(
      join(REPOSITORY_ROOT, 'apps', 'desktop', 'src-tauri', 'src', 'platform', 'macos_menu.rs'),
      'utf8'
    );

    assert.doesNotMatch(menuSource, /WINDOW_SUBMENU_ID|bring_all_to_front/);
    assert.match(menuSource, /NSDisabledDictationMenuItem/);
    assert.match(menuSource, /NSDisabledCharacterPaletteMenuItem/);
    assert.match(menuSource, /const EDIT_MENU_ITEM_COUNT: isize = 7;/);
  });
});

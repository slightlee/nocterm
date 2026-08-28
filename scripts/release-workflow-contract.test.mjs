import assert from 'node:assert/strict';
import { existsSync, readFileSync, statSync } from 'node:fs';
import { describe, it } from 'node:test';

const readWorkflow = (path) => readFileSync(path, 'utf8').replaceAll('\r\n', '\n');

describe('release workflow contract', () => {
  it('requires a maintainer to start Release Please explicitly', () => {
    const workflow = readWorkflow('.github/workflows/release-please.yml');

    assert.match(workflow, /^on:\n {2}workflow_dispatch:\n/m);
    assert.doesNotMatch(workflow, /^ {2}push:/m);
  });

  it('restores annotated tag objects and supports validating an existing tag', () => {
    const workflow = readWorkflow('.github/workflows/ci.yml');

    assert.match(workflow, /^ {2}workflow_dispatch:\n {4}inputs:\n {6}release_tag:/m);
    assert.match(workflow, /ref: \$\{\{ inputs\.release_tag \|\| github\.ref \}\}/);
    assert.match(
      workflow,
      /git fetch --force origin "refs\/tags\/\$RELEASE_TAG:refs\/tags\/\$RELEASE_TAG"/
    );
    assert.match(workflow, /RELEASE_TAG: \$\{\{ inputs\.release_tag \|\| github\.ref_name \}\}/);
  });

  it('uses the Windows GUI subsystem for release builds', () => {
    const desktopEntrypoint = readWorkflow('apps/desktop/src-tauri/src/main.rs');

    assert.match(
      desktopEntrypoint,
      /^#!\[cfg_attr\(not\(debug_assertions\), windows_subsystem = "windows"\)\]$/m
    );
  });

  it('bundles the required desktop application icons', () => {
    const tauriConfig = JSON.parse(readFileSync('apps/desktop/src-tauri/tauri.conf.json', 'utf8'));
    const desktopIcons = [
      'icons/32x32.png',
      'icons/128x128.png',
      'icons/128x128@2x.png',
      'icons/icon.icns',
      'icons/icon.ico',
    ];

    assert.deepEqual(tauriConfig.bundle.icon, desktopIcons);
    for (const icon of desktopIcons) {
      const path = `apps/desktop/src-tauri/${icon}`;
      assert.equal(existsSync(path), true, `Missing desktop icon: ${path}`);
      assert.ok(statSync(path).size > 0, `Desktop icon is empty: ${path}`);
    }
  });

  it('builds both desktop platforms from tags and only prepares a draft release', () => {
    const workflow = readWorkflow('.github/workflows/release.yml');
    const tauriConfig = JSON.parse(readFileSync('apps/desktop/src-tauri/tauri.conf.json', 'utf8'));

    assert.match(workflow, /^ {2}push:\n {4}tags: \['v\*'\]/m);
    assert.match(workflow, /^ {2}workflow_dispatch:\n {4}inputs:\n {6}release_tag:/m);
    assert.match(workflow, /release_sha: \$\{\{ steps\.release\.outputs\.release_sha \}\}/);
    assert.match(workflow, /ref: \$\{\{ needs\.validate\.outputs\.release_sha \}\}/);
    assert.match(workflow, /runner: macos-15\n {12}target: aarch64-apple-darwin/);
    assert.match(workflow, /runner: macos-15-intel\n {12}target: x86_64-apple-darwin/);
    assert.match(workflow, /MACOSX_DEPLOYMENT_TARGET: '14\.0'/);
    assert.match(workflow, /pnpm tauri build --target "\$RUST_TARGET" .* --bundles dmg --ci/);
    assert.match(workflow, /pnpm release:build:windows/);
    assert.match(workflow, /release create "\$RELEASE_TAG"/);
    assert.match(workflow, /gh "\$\{RELEASE_ARGS\[@\]\}"/);
    assert.match(workflow, /--draft/);
    assert.match(workflow, /gh release upload/);
    assert.match(workflow, /Nocterm_\$\{RELEASE_VERSION\}_macos_aarch64\.dmg/);
    assert.match(workflow, /Nocterm_\$\{RELEASE_VERSION\}_macos_x86_64\.dmg/);
    assert.match(workflow, /Nocterm_\$\{env:RELEASE_VERSION\}_windows_x86_64-setup\.exe/);
    assert.match(
      workflow,
      /verify-draft-release:[\s\S]*?permissions:\n {6}contents: write[\s\S]*?Verify draft release assets/
    );
    assert.doesNotMatch(workflow, /gh release edit .*--draft=false/);
    assert.equal(tauriConfig.bundle.macOS.minimumSystemVersion, '14.0');
  });
});

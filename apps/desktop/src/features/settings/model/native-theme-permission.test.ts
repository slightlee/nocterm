import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

interface DesktopCapability {
  permissions: string[];
}

const capability = JSON.parse(
  readFileSync(new URL('../../../../src-tauri/capabilities/default.json', import.meta.url), 'utf8')
) as DesktopCapability;

describe('native application theme permission', () => {
  it('allows the settings feature to synchronize macOS window chrome', () => {
    expect(capability.permissions).toContain('core:app:allow-set-app-theme');
  });
});

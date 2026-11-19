import { describe, expect, it } from 'vitest';
import { getFileEditBlockReason } from './file-edit-policy';

describe('file editing eligibility', () => {
  it.each([
    'notes.txt',
    'config.yaml',
    '.env',
    '.env.production',
    '.bashrc',
    'Dockerfile',
    'Makefile',
    'hosts',
    'custom.unknown',
    'data.dat',
    'config.bak',
    'icon.svg',
    'seed.sql',
    'module.ts',
    'module.mts',
    'archive.zip.txt',
  ])('allows %s to reach backend content validation', (name) => {
    expect(getFileEditBlockReason({ name, sizeBytes: 32 })).toBeNull();
  });

  it.each([
    'photo.PNG',
    'animation.gif',
    'archive.tar.gz',
    'document.pdf',
    'report.docx',
    'sheet.xlsx',
    'clip.mp4',
    'music.flac',
    'program.exe',
    'module.wasm',
    'database.sqlite3',
    'font.woff2',
  ])('explains why %s cannot use the text editor', (name) => {
    expect(getFileEditBlockReason({ name, sizeBytes: null })).toBe('此文件类型不支持文本编辑。');
  });

  it('allows empty files, the size boundary and unknown sizes', () => {
    for (const sizeBytes of [0, 2 * 1024 * 1024, null]) {
      expect(getFileEditBlockReason({ name: 'config', sizeBytes })).toBeNull();
    }
  });

  it('explains the known size limit and never treats a directory as a file', () => {
    expect(getFileEditBlockReason({ name: 'large.log', sizeBytes: 2 * 1024 * 1024 + 1 })).toContain(
      '2 MiB'
    );
    expect(getFileEditBlockReason({ name: 'config', isDir: true, sizeBytes: null })).toContain(
      '文件夹'
    );
  });
});

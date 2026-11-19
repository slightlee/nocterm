import type { FileRow } from '../types/sftp-types';

/** 与 Application FileEditService 的受限读取上限保持一致；后端仍会复核实际大小。 */
export const FILE_EDIT_MAX_BYTES = 2 * 1024 * 1024;

// 只预先排除常见二进制格式，不把未知后缀、无后缀或 .dat/.bak 当作二进制。
// SVG、SQL 和 TypeScript 等文本格式不能因其用途或同名媒体后缀被误拦。
const BINARY_EXTENSIONS = new Set(
  [
    'png jpg jpeg gif webp avif heic bmp ico tif tiff psd',
    'zip rar 7z tar gz bz2 xz zst tgz tbz2 txz svgz',
    'mp3 mp4 m4a m4v wav flac ogg aac wma avi mov mkv webm',
    'pdf doc docx xls xlsx ppt pptx odt ods odp epub',
    'exe dll so dylib a o class jar war wasm pyc pyo bin',
    'db db3 sqlite sqlite3 woff woff2 ttf otf',
    'dmg iso msi apk deb rpm',
  ]
    .join(' ')
    .split(' ')
);

/**
 * 本地和远程共用的入口预判：仅使用目录已有元数据，不发起内容探测或额外 stat。
 * null 只代表允许尝试，编码、内容、链接和最新大小仍由打开时的后端检查决定。
 */
export function getFileEditBlockReason(
  entry: Pick<FileRow, 'name' | 'isDir' | 'sizeBytes'>
): string | null {
  if (entry.isDir) return '文件夹不支持文本编辑。';
  if (entry.sizeBytes != null && entry.sizeBytes > FILE_EDIT_MAX_BYTES) {
    return '文件超过在线编辑上限（2 MiB）。';
  }
  const dot = entry.name.lastIndexOf('.');
  if (dot >= 0 && BINARY_EXTENSIONS.has(entry.name.slice(dot + 1).toLowerCase())) {
    return '此文件类型不支持文本编辑。';
  }
  return null;
}

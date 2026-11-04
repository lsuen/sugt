import { unzipSync, strFromU8 } from 'fflate';
import { THEME_PACK_VERSION, type ThemePackJson, type ThemeTokens } from './schema';
import { assertNoUnknownTokens, completeThemeTokens } from './defaults';
import type { ThemeMeta } from './types';
import { MAX_BG_BYTES } from './customBg';

const IMAGE_EXT = /\.(jpe?g|png|webp|gif)$/i;
const MAX_BG_MB = (MAX_BG_BYTES / 1024 / 1024).toFixed(1);
const RELATIVE_BG_URL = /^url\(\s*["']?\.\//i;

function mimeForName(name: string): string {
  const lower = name.toLowerCase();
  if (lower.endsWith('.png')) return 'image/png';
  if (lower.endsWith('.webp')) return 'image/webp';
  if (lower.endsWith('.gif')) return 'image/gif';
  return 'image/jpeg';
}

function uint8ToDataUrl(data: Uint8Array, mime: string): string {
  let binary = '';
  const chunk = 0x8000;
  for (let i = 0; i < data.length; i += chunk) {
    binary += String.fromCharCode(...data.subarray(i, i + chunk));
  }
  return `data:${mime};base64,${btoa(binary)}`;
}

async function blobToDataUrl(blob: Blob): Promise<string> {
  if (blob.size > MAX_BG_BYTES) {
    throw new Error(`主题包背景图过大（${(blob.size / 1024 / 1024).toFixed(1)}MB），请压缩到约 ${MAX_BG_MB}MB 以内`);
  }
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      if (typeof reader.result === 'string') resolve(reader.result);
      else reject(new Error('无法读取主题背景图'));
    };
    reader.onerror = () => reject(new Error('无法读取主题背景图'));
    reader.readAsDataURL(blob);
  });
}

/** Build injectable CSS from a validated theme pack. */
export function themePackToCss(pack: ThemePackJson, assetBaseUrl = ''): string {
  assertNoUnknownTokens(pack.tokens as Record<string, string>, pack.id);
  const tokens: Record<string, string> = { ...completeThemeTokens(pack.tokens as Record<string, string>) };

  const assetName = pack.assets?.bgImage;
  const currentBg = tokens['--bg-image'] ?? '';
  // 远程包：声明的资源一律按 JSON 所在目录解析，即使包里已写了相对 url("./bg.jpg")。
  // 已经嵌入的 data URL 不需要重写。
  if (assetName && assetBaseUrl && !/^url\(\s*["']?data:/i.test(currentBg)) {
    const base = assetBaseUrl.replace(/\/?$/, '/');
    const href = /^https?:|^\/|^data:/i.test(assetName)
      ? assetName
      : `${base}${assetName}`;
    tokens['--bg-image'] = `url(${JSON.stringify(href)})`;
  } else if (assetName && !/^url\(|^none$/i.test(currentBg)) {
    tokens['--bg-image'] = `url(${JSON.stringify(assetName)})`;
  }

  // 本地导入时若包内没有可用背景文件，回退纯色，避免注入失效的相对 URL。
  // 远程包带 assetBaseUrl、zip 包带 data URL，两者都不会命中这条分支。
  if (!assetBaseUrl && RELATIVE_BG_URL.test(tokens['--bg-image'] ?? '')) {
    tokens['--bg-image'] = 'none';
    tokens['--bg-image-opacity'] = '0';
  }

  const body = Object.entries(tokens)
    .map(([k, v]) => `  ${k}: ${v};`)
    .join('\n');

  return `:root[data-theme="${pack.id}"] {\n${body}\n}`;
}

export function parseThemePackJson(raw: unknown, opts?: { complete?: boolean }): ThemePackJson {
  if (!raw || typeof raw !== 'object') throw new Error('theme pack must be a JSON object');
  const pack = raw as ThemePackJson;
  if (pack.version !== THEME_PACK_VERSION) {
    throw new Error(`unsupported theme pack version: ${pack.version} (need ${THEME_PACK_VERSION})`);
  }
  if (!pack.id || typeof pack.id !== 'string') throw new Error('theme pack missing id');
  if (!/^[a-z0-9][a-z0-9_-]{0,47}$/i.test(pack.id)) {
    throw new Error('theme pack id must be alphanumeric (max 48), e.g. my-skin');
  }
  if (!pack.label?.zh || !pack.label?.en) throw new Error('theme pack missing label.zh / label.en');
  if (!pack.tokens || typeof pack.tokens !== 'object') throw new Error('theme pack missing tokens');
  assertNoUnknownTokens(pack.tokens as Record<string, string>, pack.id);
  const tokens = opts?.complete === false
    ? (pack.tokens as ThemeTokens)
    : completeThemeTokens(pack.tokens as Record<string, string>);
  return { ...pack, tokens };
}

function metaFromPack(pack: ThemePackJson, cssText: string): ThemeMeta {
  const tokens = completeThemeTokens(pack.tokens as Record<string, string>);
  return {
    id: pack.id,
    labelKey: `theme.${pack.id}`,
    labelZh: pack.label.zh,
    labelEn: pack.label.en,
    source: { kind: 'pack', cssText, id: pack.id },
    swatches: [
      tokens['--accent'] ?? '#888',
      tokens['--accent-2'] ?? '#666',
      tokens['--bg'] ?? '#111',
    ],
  };
}

/** Embed data-URL background into tokens and build CSS. */
function packWithEmbeddedBg(pack: ThemePackJson, bgDataUrl: string | null): ThemeMeta {
  const tokens = completeThemeTokens(pack.tokens as Record<string, string>);
  if (bgDataUrl) {
    tokens['--bg-image'] = `url(${JSON.stringify(bgDataUrl)})`;
    if (!pack.tokens['--bg-image-opacity'] || pack.tokens['--bg-image-opacity'] === '0') {
      tokens['--bg-image-opacity'] = '0.4';
    }
  }
  const cssText = themePackToCss({ ...pack, tokens }, '');
  return metaFromPack({ ...pack, tokens }, cssText);
}

export async function fetchThemePack(url: string): Promise<ThemeMeta> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`theme pack fetch failed: ${res.status}`);
  const pack = parseThemePackJson(await res.json());
  const assetBase = url.replace(/[^/]+$/, '');
  const cssText = themePackToCss(pack, assetBase);
  return metaFromPack(pack, cssText);
}

/** Import a local theme.json. Prefer zip/folder so relative bg assets work. */
export async function importThemePackFile(file: File, assetBaseUrl = ''): Promise<ThemeMeta> {
  const name = file.name.toLowerCase();
  if (name.endsWith('.zip')) return importThemePackZip(file);
  const text = await file.text();
  const pack = parseThemePackJson(JSON.parse(text));
  const cssText = themePackToCss(pack, assetBaseUrl);
  return metaFromPack(pack, cssText);
}

export async function importThemePackZip(file: File): Promise<ThemeMeta> {
  const buf = new Uint8Array(await file.arrayBuffer());
  let files: Record<string, Uint8Array>;
  try {
    files = unzipSync(buf);
  } catch {
    throw new Error('无法解压 zip，请确认是标准 zip 主题包');
  }

  const entries = Object.keys(files).filter((p) => !p.endsWith('/') && !p.split('/').pop()?.startsWith('.'));
  const jsonPath = entries.find((p) => p.replace(/\\/g, '/').toLowerCase().endsWith('theme.json'));
  if (!jsonPath) throw new Error('zip 中未找到 theme.json');

  const pack = parseThemePackJson(JSON.parse(strFromU8(files[jsonPath])));
  const dir = jsonPath.replace(/\\/g, '/').replace(/[^/]+$/, '');

  let bgDataUrl: string | null = null;
  const assetName = pack.assets?.bgImage;
  if (assetName) {
    const candidate = entries.find((p) => {
      const norm = p.replace(/\\/g, '/');
      return norm === `${dir}${assetName}` || norm.endsWith(`/${assetName}`) || norm === assetName;
    });
    if (candidate) {
      if (files[candidate].byteLength > MAX_BG_BYTES) {
        throw new Error(`主题包背景图过大，请压缩到约 ${MAX_BG_MB}MB 以内`);
      }
      bgDataUrl = uint8ToDataUrl(files[candidate], mimeForName(candidate));
    }
  } else {
    const img = entries.find((p) => {
      const base = p.replace(/\\/g, '/').slice(dir.length);
      return !base.includes('/') && IMAGE_EXT.test(base);
    });
    if (img) {
      if (files[img].byteLength > MAX_BG_BYTES) {
        throw new Error(`主题包背景图过大，请压缩到约 ${MAX_BG_MB}MB 以内`);
      }
      bgDataUrl = uint8ToDataUrl(files[img], mimeForName(img));
    }
  }

  return packWithEmbeddedBg(pack, bgDataUrl);
}

/** Import a folder picked via input[webkitdirectory] (theme.json + assets). */
export async function importThemePackDirectory(fileList: FileList | File[]): Promise<ThemeMeta> {
  const files = Array.from(fileList);
  if (!files.length) throw new Error('未选择文件夹');

  const jsonFile = files.find((f) => f.name.toLowerCase() === 'theme.json')
    ?? files.find((f) => f.webkitRelativePath?.toLowerCase().endsWith('/theme.json')
      || f.webkitRelativePath?.toLowerCase() === 'theme.json');
  if (!jsonFile) throw new Error('文件夹中未找到 theme.json');

  const pack = parseThemePackJson(JSON.parse(await jsonFile.text()));
  const jsonDir = (jsonFile.webkitRelativePath || jsonFile.name).replace(/\\/g, '/').replace(/[^/]+$/, '');

  let bgBlob: Blob | null = null;
  const assetName = pack.assets?.bgImage;
  if (assetName) {
    const hit = files.find((f) => {
      const rel = (f.webkitRelativePath || f.name).replace(/\\/g, '/');
      return rel === `${jsonDir}${assetName}` || rel.endsWith(`/${assetName}`) || f.name === assetName;
    });
    if (hit) bgBlob = hit;
  } else {
    const hit = files.find((f) => {
      const rel = (f.webkitRelativePath || f.name).replace(/\\/g, '/');
      const base = rel.slice(jsonDir.length);
      return !base.includes('/') && IMAGE_EXT.test(f.name);
    });
    if (hit) bgBlob = hit;
  }

  const bgDataUrl = bgBlob ? await blobToDataUrl(bgBlob) : null;
  return packWithEmbeddedBg(pack, bgDataUrl);
}

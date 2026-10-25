import {
  THEME_PACK_VERSION,
  assertThemeTokens,
  type ThemePackJson,
  type ThemeTokens,
} from './schema';
import type { ThemeMeta } from './types';

/** Build injectable CSS from a validated theme pack. assetBaseUrl resolves relative assets. */
export function themePackToCss(pack: ThemePackJson, assetBaseUrl = ''): string {
  assertThemeTokens(pack.tokens as Record<string, string>, pack.id);

  const tokens: Record<string, string> = { ...(pack.tokens as ThemeTokens) };
  if (pack.assets?.bgImage) {
    const base = assetBaseUrl.replace(/\/?$/, '/');
    const href = /^https?:|^\//.test(pack.assets.bgImage)
      ? pack.assets.bgImage
      : `${base}${pack.assets.bgImage}`;
    tokens['--bg-image'] = `url("${href}")`;
  }

  const body = Object.entries(tokens)
    .map(([k, v]) => `  ${k}: ${v};`)
    .join('\n');

  return `[data-theme="${pack.id}"] {\n${body}\n}`;
}

export function parseThemePackJson(raw: unknown): ThemePackJson {
  if (!raw || typeof raw !== 'object') throw new Error('theme pack must be a JSON object');
  const pack = raw as ThemePackJson;
  if (pack.version !== THEME_PACK_VERSION) {
    throw new Error(`unsupported theme pack version: ${pack.version} (need ${THEME_PACK_VERSION})`);
  }
  if (!pack.id || typeof pack.id !== 'string') throw new Error('theme pack missing id');
  if (!pack.label?.zh || !pack.label?.en) throw new Error('theme pack missing label.zh / label.en');
  if (!pack.tokens || typeof pack.tokens !== 'object') throw new Error('theme pack missing tokens');
  assertThemeTokens(pack.tokens as Record<string, string>, pack.id);
  return pack;
}

export async function fetchThemePack(url: string): Promise<ThemeMeta> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`theme pack fetch failed: ${res.status}`);
  const pack = parseThemePackJson(await res.json());
  const assetBase = url.replace(/[^/]+$/, '');
  const cssText = themePackToCss(pack, assetBase);
  return {
    id: pack.id,
    labelKey: `theme.${pack.id}`,
    labelZh: pack.label.zh,
    labelEn: pack.label.en,
    source: { kind: 'pack', cssText, id: pack.id },
    swatches: [pack.tokens['--accent'] ?? '#888', pack.tokens['--accent-2'] ?? '#666', pack.tokens['--bg'] ?? '#111'],
  };
}

/** Import a local File (theme.json). Sibling assets must be absolute URLs if used. */
export async function importThemePackFile(file: File, assetBaseUrl = ''): Promise<ThemeMeta> {
  const text = await file.text();
  const pack = parseThemePackJson(JSON.parse(text));
  const cssText = themePackToCss(pack, assetBaseUrl);
  return {
    id: pack.id,
    labelKey: `theme.${pack.id}`,
    labelZh: pack.label.zh,
    labelEn: pack.label.en,
    source: { kind: 'pack', cssText, id: pack.id },
    swatches: [pack.tokens['--accent'] ?? '#888', pack.tokens['--accent-2'] ?? '#666', pack.tokens['--bg'] ?? '#111'],
  };
}

import type { ThemeTokenKey } from './schema';

export type BuiltinThemeId =
  | 'cyber'
  | 'street'
  | 'skate'
  | 'brickyard'
  | 'shanshui'
  | 'hiphop'
  | 'jazz'
  | 'anime-boy'
  | 'anime-girl';

export type ThemeId = BuiltinThemeId | (string & {});

export type ThemeSource =
  | { kind: 'builtin'; cssHref: string }
  | { kind: 'remote'; url: string }
  | { kind: 'pack'; cssText: string; id: string };

export type ThemeMeta = {
  id: ThemeId;
  labelKey: string;
  /** Fallback labels when i18n key missing */
  labelZh: string;
  labelEn: string;
  source: ThemeSource;
  /** Preview swatches for settings UI */
  swatches: [string, string, string];
};

export type { ThemeTokenKey };

/** Fixed CSS custom-property contract. Themes must fill every key. */
export const THEME_TOKEN_KEYS = [
  '--bg',
  '--bg-glow-1',
  '--bg-glow-2',
  '--bg-image',
  '--bg-image-opacity',
  '--surface',
  '--surface-2',
  '--surface-glass',
  '--glass-blur',
  '--sidebar-bg',
  '--scrim',
  '--text',
  '--muted',
  '--code',
  '--border',
  '--border-strong',
  '--focus-ring',
  '--accent',
  '--accent-2',
  '--accent-soft',
  '--danger',
  '--danger-soft',
  '--success',
  '--success-soft',
  '--chip-bg',
  '--chip-active',
  '--btn-primary-bg',
  '--btn-primary-fg',
  '--btn-ghost-bg',
  '--btn-ghost-border',
  '--btn-danger-bg',
  '--input-bg',
  '--scrollbar',
  '--shadow',
  '--radius',
  '--radius-sm',
  '--font-ui',
  '--font-display',
] as const;

export type ThemeTokenKey = (typeof THEME_TOKEN_KEYS)[number];
export type ThemeTokens = Record<ThemeTokenKey, string>;

export const THEME_PACK_VERSION = 1;

/** External / importable theme pack (JSON). */
export type ThemePackJson = {
  version: number;
  id: string;
  label: { zh: string; en: string };
  tokens: Partial<ThemeTokens> & Record<string, string>;
  /** Relative asset filenames inside the pack folder (optional). */
  assets?: {
    bgImage?: string;
  };
};

export function missingThemeTokens(tokens: Record<string, string>): ThemeTokenKey[] {
  return THEME_TOKEN_KEYS.filter((k) => {
    const v = tokens[k];
    return v === undefined || v === null || String(v).trim() === '';
  });
}

export function assertThemeTokens(tokens: Record<string, string>, themeId: string): void {
  const missing = missingThemeTokens(tokens);
  if (missing.length) {
    throw new Error(`[theme:${themeId}] missing tokens: ${missing.join(', ')}`);
  }
  const unknown = Object.keys(tokens).filter(
    (k) => k.startsWith('--') && !(THEME_TOKEN_KEYS as readonly string[]).includes(k),
  );
  if (unknown.length) {
    throw new Error(`[theme:${themeId}] unknown tokens (not in contract): ${unknown.join(', ')}`);
  }
}

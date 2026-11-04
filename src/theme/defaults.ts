import type { ThemeTokenKey, ThemeTokens } from './schema';
import { THEME_TOKEN_KEYS } from './schema';

/** Mirrors `tokens.css` :root — used to complete partial imported packs. */
export const DEFAULT_THEME_TOKENS: ThemeTokens = {
  '--bg': '#070b14',
  '--bg-glow-1': 'rgba(24, 225, 255, 0.24)',
  '--bg-glow-2': 'rgba(124, 77, 255, 0.24)',
  '--bg-image': 'none',
  '--bg-image-opacity': '0',
  '--surface': 'rgba(18, 29, 47, 0.82)',
  '--surface-2': 'rgba(8, 14, 26, 0.86)',
  '--surface-glass': 'rgba(12, 20, 34, 0.55)',
  '--glass-blur': '0px',
  '--sidebar-bg': 'rgba(5, 12, 22, 0.72)',
  '--scrim': 'rgba(0, 0, 0, 0.45)',
  '--text': '#edf7ff',
  '--muted': '#94a9bd',
  '--code': '#8ee8ff',
  '--border': 'rgba(142, 232, 255, 0.16)',
  '--border-strong': 'rgba(142, 232, 255, 0.3)',
  '--focus-ring': 'rgba(22, 241, 255, 0.25)',
  '--accent': '#16f1ff',
  '--accent-2': '#6d5cff',
  '--accent-soft': 'rgba(142, 232, 255, 0.12)',
  '--danger': '#ff4d6d',
  '--danger-soft': 'rgba(255, 98, 127, 0.16)',
  '--success': '#0aff9d',
  '--success-soft': 'rgba(10, 255, 157, 0.16)',
  '--chip-bg': 'rgba(255, 255, 255, 0.045)',
  '--chip-active': 'rgba(142, 232, 255, 0.12)',
  '--btn-primary-bg': 'linear-gradient(135deg, #12dfff, #6d5cff)',
  '--btn-primary-fg': '#edf7ff',
  '--btn-ghost-bg': 'rgba(255, 255, 255, 0.06)',
  '--btn-ghost-border': 'rgba(255, 255, 255, 0.1)',
  '--btn-danger-bg': 'linear-gradient(135deg, #ff4d6d, #ff8754)',
  '--input-bg': '#121a28',
  '--scrollbar': 'rgba(148, 163, 184, 0.35)',
  '--shadow': '0 8px 28px rgba(0, 0, 0, 0.28)',
  '--radius': '16px',
  '--radius-sm': '12px',
  '--font-ui': 'Inter, "Microsoft YaHei", "PingFang SC", system-ui, sans-serif',
  '--font-display': 'var(--font-ui)',
};

export function completeThemeTokens(
  partial: Record<string, string>,
): ThemeTokens {
  const out = { ...DEFAULT_THEME_TOKENS };
  for (const key of THEME_TOKEN_KEYS) {
    const v = partial[key];
    if (v !== undefined && v !== null && String(v).trim() !== '') {
      out[key] = String(v).trim();
    }
  }
  // Drop unknown --* keys by only copying contract keys above.
  return out;
}

export function assertNoUnknownTokens(tokens: Record<string, string>, themeId: string): void {
  const unknown = Object.keys(tokens).filter(
    (k) => k.startsWith('--') && !(THEME_TOKEN_KEYS as readonly string[]).includes(k),
  );
  if (unknown.length) {
    throw new Error(`[theme:${themeId}] unknown tokens (not in contract): ${unknown.join(', ')}`);
  }
}

export type { ThemeTokenKey };

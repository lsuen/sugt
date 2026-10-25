import type { ThemeId, ThemeMeta, ThemeSource } from './types';
import { BUILTIN_THEMES, DEFAULT_THEME_ID, isBuiltinThemeId } from './registry';

const STORAGE_KEY = 'sugt_theme';
const STYLE_ATTR = 'data-sugt-theme-pack';
const LINK_ATTR = 'data-sugt-theme-remote';

export function loadStoredThemeId(): ThemeId {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    if (v && isBuiltinThemeId(v)) return v;
    if (v) return v;
  } catch {
    /* ignore */
  }
  return DEFAULT_THEME_ID;
}

export function persistThemeId(id: ThemeId): void {
  try {
    localStorage.setItem(STORAGE_KEY, id);
  } catch {
    /* ignore */
  }
}

function clearInjectedThemeAssets(): void {
  document.querySelectorAll(`style[${STYLE_ATTR}]`).forEach((n) => n.remove());
  document.querySelectorAll(`link[${LINK_ATTR}]`).forEach((n) => n.remove());
}

function injectStyle(cssText: string, id: string): void {
  const el = document.createElement('style');
  el.setAttribute(STYLE_ATTR, id);
  el.textContent = cssText;
  document.head.appendChild(el);
}

function injectRemoteStylesheet(url: string, id: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const link = document.createElement('link');
    link.rel = 'stylesheet';
    link.href = url;
    link.setAttribute(LINK_ATTR, id);
    link.onload = () => resolve();
    link.onerror = () => reject(new Error(`Failed to load theme stylesheet: ${url}`));
    document.head.appendChild(link);
  });
}

/** Apply theme id to <html data-theme>. Builtin CSS is already imported; packs/remotes inject. */
export async function applyTheme(meta: ThemeMeta): Promise<void> {
  clearInjectedThemeAssets();
  const source: ThemeSource = meta.source;

  if (source.kind === 'pack') {
    injectStyle(source.cssText, meta.id);
  } else if (source.kind === 'remote') {
    await injectRemoteStylesheet(source.url, meta.id);
  }
  // builtin: CSS modules imported once in registry; only dataset flips

  document.documentElement.dataset.theme = meta.id;
  persistThemeId(meta.id);
}

export function resolveThemeMeta(id: ThemeId, extras: ThemeMeta[] = []): ThemeMeta {
  const all = [...BUILTIN_THEMES, ...extras];
  return all.find((t) => t.id === id) ?? BUILTIN_THEMES.find((t) => t.id === DEFAULT_THEME_ID)!;
}

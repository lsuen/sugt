import { useCallback, useEffect, useMemo, useState } from 'react';
import { applyTheme, loadStoredThemeId, resolveThemeMeta } from './applyTheme';
import { BUILTIN_THEMES } from './registry';
import type { ThemeId, ThemeMeta } from './types';

export function useTheme(extraThemes: ThemeMeta[] = []) {
  const [themeId, setThemeId] = useState<ThemeId>(() => loadStoredThemeId());
  const [custom, setCustom] = useState<ThemeMeta[]>(extraThemes);
  const [error, setError] = useState<string | null>(null);

  const catalog = useMemo(() => {
    const map = new Map<string, ThemeMeta>();
    for (const t of BUILTIN_THEMES) map.set(t.id, t);
    for (const t of custom) map.set(t.id, t);
    return [...map.values()];
  }, [custom]);

  const theme = useMemo(() => resolveThemeMeta(themeId, custom), [themeId, custom]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        await applyTheme(theme);
        if (!cancelled) setError(null);
      } catch (e) {
        if (!cancelled) {
          setError(e instanceof Error ? e.message : String(e));
          const fallback = BUILTIN_THEMES[0];
          await applyTheme(fallback);
          setThemeId(fallback.id);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [theme]);

  const setTheme = useCallback((id: ThemeId) => {
    setThemeId(id);
  }, []);

  const registerTheme = useCallback((meta: ThemeMeta) => {
    setCustom((prev) => {
      const next = prev.filter((t) => t.id !== meta.id);
      next.push(meta);
      return next;
    });
    setThemeId(meta.id);
  }, []);

  return { themeId, theme, catalog, setTheme, registerTheme, error };
}

/** Apply stored theme before React paints (call from main.tsx). */
export function bootstrapTheme(): void {
  const id = loadStoredThemeId();
  document.documentElement.dataset.theme = id;
}

import { useCallback, useEffect, useMemo, useState } from 'react';
import { applyTheme, loadStoredThemeId, resolveThemeMeta } from './applyTheme';
import {
  applyCustomBg,
  bootstrapCustomBg,
  clearCustomBg as clearCustomBgStorage,
  DEFAULT_CUSTOM_BG_OPACITY,
  fileToDataUrl,
  saveCustomBg,
} from './customBg';
import { BUILTIN_THEMES } from './registry';
import { loadCustomBg, loadStoredPacks, upsertStoredPack, type CustomBgState } from './themeStorage';
import type { ThemeId, ThemeMeta } from './types';

export function useTheme(extraThemes: ThemeMeta[] = []) {
  const [themeId, setThemeId] = useState<ThemeId>(() => loadStoredThemeId());
  const [custom, setCustom] = useState<ThemeMeta[]>(() => [...loadStoredPacks(), ...extraThemes]);
  const [error, setError] = useState<string | null>(null);
  const [customBg, setCustomBg] = useState<CustomBgState | null>(() => loadCustomBg());

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
        applyCustomBg(customBg);
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
  }, [theme, customBg]);

  const setTheme = useCallback((id: ThemeId) => {
    document.documentElement.dataset.theme = id;
    setThemeId(id);
  }, []);

  const registerTheme = useCallback((meta: ThemeMeta) => {
    try {
      if (meta.source.kind === 'pack') upsertStoredPack(meta);
      setCustom((prev) => {
        const next = prev.filter((t) => t.id !== meta.id);
        next.push(meta);
        return next;
      });
      setThemeId(meta.id);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      throw e;
    }
  }, []);

  const setCustomBackground = useCallback(async (file: File, opacity?: number) => {
    const dataUrl = await fileToDataUrl(file);
    const state: CustomBgState = {
      dataUrl,
      opacity: opacity ?? customBg?.opacity ?? DEFAULT_CUSTOM_BG_OPACITY,
    };
    saveCustomBg(state);
    setCustomBg(state);
  }, [customBg?.opacity]);

  const setCustomBackgroundOpacity = useCallback((opacity: number) => {
    setCustomBg((prev) => {
      if (!prev) return prev;
      const next = { ...prev, opacity: Math.min(1, Math.max(0, opacity)) };
      saveCustomBg(next);
      return next;
    });
  }, []);

  const clearCustomBackground = useCallback(() => {
    clearCustomBgStorage();
    setCustomBg(null);
  }, []);

  return {
    themeId,
    theme,
    catalog,
    setTheme,
    registerTheme,
    error,
    customBg,
    setCustomBackground,
    setCustomBackgroundOpacity,
    clearCustomBackground,
  };
}

/** Apply stored theme + custom bg before React paints (call from main.tsx). */
export function bootstrapTheme(): void {
  const id = loadStoredThemeId();
  document.documentElement.dataset.theme = id;
  bootstrapCustomBg();
}

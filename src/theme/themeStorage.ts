import type { ThemeMeta } from './types';

const PACKS_KEY = 'sugt_theme_packs_v1';
const CUSTOM_BG_KEY = 'sugt_custom_bg_v1';

export type StoredPack = {
  id: string;
  labelZh: string;
  labelEn: string;
  cssText: string;
  swatches: [string, string, string];
};

export type CustomBgState = {
  dataUrl: string;
  opacity: number;
};

export function loadStoredPacks(): ThemeMeta[] {
  try {
    const raw = localStorage.getItem(PACKS_KEY);
    if (!raw) return [];
    const list = JSON.parse(raw) as StoredPack[];
    if (!Array.isArray(list)) return [];
    return list
      .filter((p) => p && typeof p.id === 'string' && typeof p.cssText === 'string')
      .map((p) => ({
        id: p.id,
        labelKey: `theme.${p.id}`,
        labelZh: p.labelZh || p.id,
        labelEn: p.labelEn || p.id,
        source: { kind: 'pack' as const, cssText: p.cssText, id: p.id },
        swatches: (p.swatches?.length === 3
          ? p.swatches
          : ['#888', '#666', '#111']) as [string, string, string],
      }));
  } catch {
    return [];
  }
}

export function persistPacks(metas: ThemeMeta[]): void {
  try {
    const packs: StoredPack[] = metas
      .filter((m) => m.source.kind === 'pack')
      .map((m) => ({
        id: m.id,
        labelZh: m.labelZh,
        labelEn: m.labelEn,
        cssText: m.source.kind === 'pack' ? m.source.cssText : '',
        swatches: m.swatches,
      }));
    localStorage.setItem(PACKS_KEY, JSON.stringify(packs));
  } catch (e) {
    throw new Error(
      e instanceof Error && /quota/i.test(e.message)
        ? '主题包过大，无法本地保存（请缩小背景图后重试）'
        : `主题包保存失败：${e instanceof Error ? e.message : String(e)}`,
    );
  }
}

export function upsertStoredPack(meta: ThemeMeta): void {
  if (meta.source.kind !== 'pack') return;
  const prev = loadStoredPacks().filter((p) => p.id !== meta.id);
  persistPacks([...prev, meta]);
}

export function loadCustomBg(): CustomBgState | null {
  try {
    const raw = localStorage.getItem(CUSTOM_BG_KEY);
    if (!raw) return null;
    const v = JSON.parse(raw) as CustomBgState;
    if (!v?.dataUrl || typeof v.dataUrl !== 'string') return null;
    const opacity = typeof v.opacity === 'number' ? Math.min(1, Math.max(0, v.opacity)) : 0.4;
    return { dataUrl: v.dataUrl, opacity };
  } catch {
    return null;
  }
}

export function persistCustomBg(state: CustomBgState | null): void {
  try {
    if (!state) {
      localStorage.removeItem(CUSTOM_BG_KEY);
      return;
    }
    localStorage.setItem(CUSTOM_BG_KEY, JSON.stringify(state));
  } catch (e) {
    throw new Error(
      e instanceof Error && /quota/i.test(e.message)
        ? '背景图过大，无法本地保存（请压缩到约 1.5MB 以内）'
        : `背景保存失败：${e instanceof Error ? e.message : String(e)}`,
    );
  }
}

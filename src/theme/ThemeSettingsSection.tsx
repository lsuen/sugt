import React, { useRef } from 'react';
import { lang, t } from '../i18n';
import { importThemePackFile } from './loadThemePack';
import type { ThemeId, ThemeMeta } from './types';

type Props = {
  themeId: ThemeId;
  catalog: ThemeMeta[];
  onTheme: (id: ThemeId) => void;
  onImport: (meta: ThemeMeta) => void;
  error?: string | null;
  pushToast?: (message: string, type: 'ok' | 'error' | 'info') => void;
};

export function ThemeSettingsSection({
  themeId,
  catalog,
  onTheme,
  onImport,
  error,
  pushToast,
}: Props) {
  const fileRef = useRef<HTMLInputElement>(null);
  const isZh = lang() === 'zh';

  const labelOf = (meta: ThemeMeta) => {
    const fromI18n = t(meta.labelKey as Parameters<typeof t>[0]);
    if (fromI18n && fromI18n !== meta.labelKey) return fromI18n;
    return isZh ? meta.labelZh : meta.labelEn;
  };

  const onPickFile = async (file: File | undefined) => {
    if (!file) return;
    try {
      const meta = await importThemePackFile(file);
      onImport(meta);
      pushToast?.(t('set.themeImportOk', { id: meta.id }), 'ok');
    } catch (e) {
      pushToast?.(e instanceof Error ? e.message : String(e), 'error');
    }
  };

  return (
    <div className="card settings-group switches" data-testid="settings-theme">
      <h4>{t('set.theme')}</h4>
      <p className="hint compact">{t('set.themeHint')}</p>
      <div className="theme-options" role="radiogroup" aria-label={t('set.theme')}>
        {catalog.map((meta) => (
          <button
            key={meta.id}
            type="button"
            role="radio"
            aria-checked={themeId === meta.id}
            className={`theme-option${themeId === meta.id ? ' is-active' : ''}`}
            data-testid={`theme-${meta.id}`}
            onClick={() => onTheme(meta.id)}
          >
            <div className="theme-option-swatches" aria-hidden>
              {meta.swatches.map((c, i) => (
                <span key={i} style={{ background: c }} />
              ))}
            </div>
            <span className="theme-option-label">{labelOf(meta)}</span>
            <span className="theme-option-id">{meta.id}</span>
          </button>
        ))}
      </div>
      <div className="theme-import-row">
        <input
          ref={fileRef}
          type="file"
          accept="application/json,.json"
          hidden
          onChange={(e) => onPickFile(e.target.files?.[0])}
        />
        <button type="button" className="ghost tiny-btn" onClick={() => fileRef.current?.click()}>
          {t('set.themeImport')}
        </button>
        <span className="hint compact">{t('set.themeImportHint')}</span>
      </div>
      {error ? <p className="issue-list" style={{ listStyle: 'none', paddingLeft: 0 }}>{error}</p> : null}
    </div>
  );
}

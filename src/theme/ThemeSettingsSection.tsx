import React, { useRef } from 'react';
import { Check } from 'lucide-react';
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
    <div className="card settings-group settings-group-wide theme-settings" data-testid="settings-theme">
      <div className="theme-settings-head">
        <div>
          <h4>{t('set.theme')}</h4>
          <p className="hint compact">{t('set.themeHint')}</p>
        </div>
        <div className="theme-import-row theme-import-row--inline">
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
        </div>
      </div>
      <div className="theme-options" role="radiogroup" aria-label={t('set.theme')}>
        {catalog.map((meta) => {
          const active = themeId === meta.id;
          return (
            <button
              key={meta.id}
              type="button"
              role="radio"
              aria-checked={active}
              className={`theme-option${active ? ' is-active' : ''}`}
              data-testid={`theme-${meta.id}`}
              onClick={() => {
                onTheme(meta.id);
                pushToast?.(t('set.themeApplied', { name: labelOf(meta) }), 'ok');
              }}
            >
              <div
                className="theme-option-preview"
                aria-hidden
                style={{
                  background: `linear-gradient(135deg, ${meta.swatches[2]} 0%, ${meta.swatches[0]} 55%, ${meta.swatches[1]} 100%)`,
                }}
              >
                <div className="theme-option-swatches">
                  {meta.swatches.map((c, i) => (
                    <span key={i} style={{ background: c }} />
                  ))}
                </div>
                {active ? (
                  <span className="theme-option-check">
                    <Check size={14} strokeWidth={3} />
                  </span>
                ) : null}
              </div>
              <span className="theme-option-label">{labelOf(meta)}</span>
              <span className="theme-option-id">{meta.id}</span>
            </button>
          );
        })}
      </div>
      <p className="hint compact theme-import-hint">{t('set.themeImportHint')}</p>
      {error ? <p className="issue-list" style={{ listStyle: 'none', paddingLeft: 0 }}>{error}</p> : null}
    </div>
  );
}

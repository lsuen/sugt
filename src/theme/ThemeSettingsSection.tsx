import React, { useRef } from 'react';
import { Check } from 'lucide-react';
import { lang, t } from '../i18n';
import { DEFAULT_CUSTOM_BG_OPACITY } from './customBg';
import { importThemePackDirectory, importThemePackFile } from './loadThemePack';
import type { ThemeId, ThemeMeta } from './types';
import type { CustomBgState } from './themeStorage';

type Props = {
  themeId: ThemeId;
  catalog: ThemeMeta[];
  onTheme: (id: ThemeId) => void;
  onImport: (meta: ThemeMeta) => void;
  error?: string | null;
  pushToast?: (message: string, type: 'ok' | 'error' | 'info') => void;
  customBg: CustomBgState | null;
  onCustomBgFile: (file: File) => Promise<void>;
  onCustomBgOpacity: (opacity: number) => void;
  onClearCustomBg: () => void;
  statusBarEnabled: boolean;
  onStatusBarEnabled: (enabled: boolean) => void;
};

export function ThemeSettingsSection({
  themeId,
  catalog,
  onTheme,
  onImport,
  error,
  pushToast,
  customBg,
  onCustomBgFile,
  onCustomBgOpacity,
  onClearCustomBg,
  statusBarEnabled,
  onStatusBarEnabled,
}: Props) {
  const fileRef = useRef<HTMLInputElement>(null);
  const dirRef = useRef<HTMLInputElement>(null);
  const bgRef = useRef<HTMLInputElement>(null);
  const isZh = lang() === 'zh';

  const labelOf = (meta: ThemeMeta) => {
    const fromI18n = t(meta.labelKey as Parameters<typeof t>[0]);
    if (fromI18n && fromI18n !== meta.labelKey) return fromI18n;
    return isZh ? meta.labelZh : meta.labelEn;
  };

  const onPickPack = async (file: File | undefined) => {
    if (!file) return;
    try {
      const meta = await importThemePackFile(file);
      onImport(meta);
      pushToast?.(t('set.themeImportOk', { id: meta.id }), 'ok');
    } catch (e) {
      pushToast?.(e instanceof Error ? e.message : String(e), 'error');
    } finally {
      if (fileRef.current) fileRef.current.value = '';
    }
  };

  const onPickDir = async (list: FileList | null) => {
    if (!list?.length) return;
    try {
      const meta = await importThemePackDirectory(list);
      onImport(meta);
      pushToast?.(t('set.themeImportOk', { id: meta.id }), 'ok');
    } catch (e) {
      pushToast?.(e instanceof Error ? e.message : String(e), 'error');
    } finally {
      if (dirRef.current) dirRef.current.value = '';
    }
  };

  const onPickBg = async (file: File | undefined) => {
    if (!file) return;
    try {
      await onCustomBgFile(file);
      pushToast?.(t('set.themeBgOk'), 'ok');
    } catch (e) {
      pushToast?.(e instanceof Error ? e.message : String(e), 'error');
    } finally {
      if (bgRef.current) bgRef.current.value = '';
    }
  };

  const opacityPct = Math.round((customBg?.opacity ?? DEFAULT_CUSTOM_BG_OPACITY) * 100);

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
            accept=".json,.zip,application/json,application/zip"
            hidden
            onChange={(e) => onPickPack(e.target.files?.[0])}
          />
          <input
            ref={dirRef}
            type="file"
            multiple
            hidden
            {...({ webkitdirectory: '', directory: '' } as Record<string, string>)}
            onChange={(e) => onPickDir(e.target.files)}
          />
          <button type="button" className="ghost tiny-btn" onClick={() => fileRef.current?.click()}>
            {t('set.themeImport')}
          </button>
          <button type="button" className="ghost tiny-btn" onClick={() => dirRef.current?.click()}>
            {t('set.themeImportFolder')}
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

      <div className="theme-bg-block">
        <div className="theme-bg-head">
          <div>
            <h5>{t('set.themeBg')}</h5>
            <p className="hint compact">{t('set.themeBgHint')}</p>
          </div>
          <div className="theme-import-row theme-import-row--inline">
            <input
              ref={bgRef}
              type="file"
              accept="image/jpeg,image/png,image/webp,image/gif,.jpg,.jpeg,.png,.webp"
              hidden
              onChange={(e) => onPickBg(e.target.files?.[0])}
            />
            <button type="button" className="ghost tiny-btn" onClick={() => bgRef.current?.click()}>
              {t('set.themeBgPick')}
            </button>
            <button
              type="button"
              className="ghost tiny-btn"
              disabled={!customBg}
              onClick={() => {
                onClearCustomBg();
                pushToast?.(t('set.themeBgCleared'), 'info');
              }}
            >
              {t('set.themeBgClear')}
            </button>
          </div>
        </div>
        {customBg ? (
          <label className="switch-line theme-bg-opacity">
            <span>{t('set.themeBgOpacity', { p: opacityPct })}</span>
            <input
              type="range"
              min={5}
              max={85}
              step={1}
              value={opacityPct}
              onChange={(e) => onCustomBgOpacity(Number(e.target.value) / 100)}
            />
          </label>
        ) : (
          <p className="hint compact">{t('set.themeBgNone')}</p>
        )}
      </div>

      <div className="theme-chrome-block">
        <h5>{t('set.chrome')}</h5>
        <p className="hint compact">{t('set.chromeHint')}</p>
        <label className="switch-line">
          <span>{t('set.statusBar')}</span>
          <input
            type="checkbox"
            checked={statusBarEnabled}
            onChange={(e) => onStatusBarEnabled(e.target.checked)}
          />
        </label>
        <p className="hint compact">{t('set.statusBarHint')}</p>
      </div>

      {error ? <p className="issue-list" style={{ listStyle: 'none', paddingLeft: 0 }}>{error}</p> : null}
    </div>
  );
}

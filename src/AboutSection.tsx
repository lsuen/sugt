// 作者：sun / 孙文龙
// 作用：设置-关于子页。版本、授权、路径、在线更新与发布说明；原独立 Tab 内容原样迁入。
import React from 'react';
import { BrandMark } from './BrandMark';
import { CURRENT_VERSION, RELEASE_NOTES } from './release-notes';
import { UpdateSection } from './UpdateSection';
import { t } from './i18n';

export type AboutTrialView = {
  edition?: string;
  product_label?: string;
  build_id?: string;
  status?: string;
  valid?: boolean;
  trial_enabled?: boolean;
  expires_date?: string | null;
  days_remaining?: number | null;
  message?: string;
};

export type AboutStatusView = {
  trial?: AboutTrialView;
  config_dir?: string;
  log_file?: string;
  listen_url?: string;
};

export function editionLabel(trial?: AboutTrialView): string {
  if (!trial) return t('about.devMode');
  switch (trial.edition) {
    case 'public':
      return t('about.editionPublic');
    case 'self':
      return t('about.editionSelf');
    default:
      return t('about.devMode');
  }
}

type Props = {
  status: AboutStatusView | null;
  busy: boolean;
  setBusy: (v: boolean) => void;
  pushToast: (msg: string, kind?: 'ok' | 'error' | 'info') => void;
  formatInvokeError: (error: unknown) => string;
  onVersionCardClick: () => void;
};

export function AboutSection({
  status,
  busy,
  setBusy,
  pushToast,
  formatInvokeError,
  onVersionCardClick,
}: Props) {
  return (
    <section className="card about-card">
      <BrandMark size="lg" className="about-logo" />
      <h3>{t('about.title')}</h3>
      <p>{t('about.author')}</p>
      <p className="trial-note">{editionLabel(status?.trial)}</p>
      <div className="about-grid">
        <div className="about-version-card" onClick={onVersionCardClick}>
          <strong>{t('about.version')}</strong><span>v{CURRENT_VERSION}</span>
        </div>
        <div><strong>{t('about.product')}</strong><span>{status?.trial?.product_label ?? t('about.full')}</span></div>
        <div><strong>{t('about.license')}</strong><span>{editionLabel(status?.trial)}</span></div>
        <div><strong>{t('about.configDir')}</strong><span>{status?.config_dir}</span></div>
        <div><strong>{t('about.logFile')}</strong><span>{status?.log_file}</span></div>
        <div><strong>{t('about.listen')}</strong><span>{status?.listen_url ?? 'http://127.0.0.1:8787'}</span></div>
      </div>
      <UpdateSection
        busy={busy}
        setBusy={setBusy}
        pushToast={pushToast}
        formatInvokeError={formatInvokeError}
      />
      <div className="release-notes-frame">
        <div className="release-notes-head">
          <strong>{t('about.releaseHead')}</strong>
          <span className="hint compact">v{CURRENT_VERSION}</span>
        </div>
        <ul className="release-notes-list">
          {RELEASE_NOTES.map((note) => (
            <li key={note.text} className={`release-note ${note.type}`}>
              <span className="release-tag">{note.type === 'feat' ? t('about.tagFeat') : t('about.tagFix')}</span>
              {note.text}
            </li>
          ))}
        </ul>
      </div>
    </section>
  );
}

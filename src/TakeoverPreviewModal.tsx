/** 接入影响预览弹窗。编写人员：孙测测（sun/孙文龙）。
 * 作用：接管前集中展示 SUGT 将写入的内容，并形成「看影响 → 做决定 → 执行」闭环。
 * 每次挂载都重新读取 preview_takeover，避免展示过期影响。 */
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Copy, RotateCcw, ShieldCheck, X } from 'lucide-react';
import { ModalPortal } from './ModalPortal';
import { t } from './i18n';

export type TakeoverPreview = {
  profile_id: string;
  profile_name: string;
  config_kind: string;
  env_names: string[];
  env_cleared: string[];
  config_writes: { path: string; field: string }[];
  skill_dirs: string[];
  warnings: string[];
  note: string;
  revertable: boolean;
};

type ToastKind = 'ok' | 'error' | 'info';

type Props = {
  profileId: string;
  profileName: string;
  configured: boolean;
  busy: boolean;
  onClose: () => void;
  onApply: () => void;
  onRelease: () => void;
  pushToast: (msg: string, type: ToastKind) => void;
};

function joinOrNone(items: string[]): string {
  return items.length ? items.join('、') : t('client.previewNone');
}

export function TakeoverPreviewModal({
  profileId,
  profileName,
  configured,
  busy,
  onClose,
  onApply,
  onRelease,
  pushToast,
}: Props) {
  const [preview, setPreview] = useState<TakeoverPreview | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const closeRef = useRef<HTMLButtonElement>(null);
  const cancelRef = useRef<(() => void) | null>(null);

  const load = useCallback(() => {
    cancelRef.current?.();
    let cancelled = false;
    cancelRef.current = () => {
      cancelled = true;
    };
    setLoading(true);
    setError('');
    setPreview(null);
    invoke<TakeoverPreview>('preview_takeover', { profileId })
      .then((next) => {
        if (!cancelled) setPreview(next);
      })
      .catch((err) => {
        if (!cancelled) setError(String(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
  }, [profileId]);

  useEffect(() => {
    load();
    return () => cancelRef.current?.();
  }, [load]);

  // 焦点回归：打开时聚焦关闭按钮，卸载时把焦点还给触发按钮。
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    closeRef.current?.focus();
    return () => previous?.focus?.();
  }, []);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  const toSummary = (data: TakeoverPreview) =>
    [
      `${t('client.previewTitle')} · ${profileName}`,
      `${t('client.previewEnv')}：${joinOrNone(data.env_names)}`,
      `${t('client.previewCleared')}：${joinOrNone(data.env_cleared)}`,
      `${t('client.previewConfigWrites')}：${
        data.config_writes.length
          ? data.config_writes.map((w) => `${w.path} · ${w.field}`).join('；')
          : t('client.previewNone')
      }`,
      `${t('client.previewSkillDirs')}：${joinOrNone(data.skill_dirs)}`,
      `${t('client.previewRevert')}：${data.revertable ? t('client.previewRevertYes') : t('client.previewRevertNo')}`,
      data.warnings.length ? `${t('client.previewWarnings')}：${data.warnings.join('；')}` : '',
      data.note,
    ]
      .filter(Boolean)
      .join('\n');

  const copySummary = async () => {
    if (!preview) return;
    try {
      await navigator.clipboard.writeText(toSummary(preview));
      pushToast(t('client.previewCopied'), 'ok');
    } catch {
      pushToast(t('client.previewCopyFailed'), 'error');
    }
  };

  const hasAnyChange =
    preview &&
    (preview.env_names.length > 0 ||
      preview.env_cleared.length > 0 ||
      preview.config_writes.length > 0 ||
      preview.skill_dirs.length > 0);

  return (
    <ModalPortal>
      <div
        className="modal-overlay"
        onMouseDown={(event) => {
          if (event.target === event.currentTarget) onClose();
        }}
      >
        <div
          className="modal takeover-preview-modal"
          role="dialog"
          aria-modal="true"
          aria-label={t('client.previewTitle')}
        >
          <div className="modal-head">
            <h3>
              <ShieldCheck size={16} /> {t('client.previewTitle')} · {profileName}
            </h3>
            <button
              ref={closeRef}
              type="button"
              className="icon-btn"
              onClick={onClose}
              aria-label={t('common.close')}
            >
              <X size={16} />
            </button>
          </div>

          <div className="modal-body">
            <p className="hint compact takeover-preview-intro">{t('client.previewIntro')}</p>

            {loading && <p className="hint compact">{t('client.previewLoading')}</p>}

            {error && (
              <div className="takeover-preview-error">
                <p className="agent-discover-warn">{error}</p>
                <button type="button" className="tiny ghost" onClick={() => void load()}>
                  <RotateCcw size={14} />{t('client.previewRetry')}
                </button>
              </div>
            )}

            {preview && (
              <>
                {!hasAnyChange && (
                  <p className="hint compact">{t('client.previewNoChanges')}</p>
                )}

                <div className="takeover-preview-grid">
                  <section className="takeover-preview-section">
                    <h5>{t('client.previewEnv')}</h5>
                    <div className="takeover-preview-values">
                      {preview.env_names.length ? (
                        preview.env_names.map((name) => <code key={name}>{name}</code>)
                      ) : (
                        <span className="hint compact">{t('client.previewNone')}</span>
                      )}
                    </div>
                  </section>

                  {preview.env_cleared.length > 0 && (
                    <section className="takeover-preview-section">
                      <h5>{t('client.previewCleared')}</h5>
                      <div className="takeover-preview-values">
                        {preview.env_cleared.map((name) => <code key={name}>{name}</code>)}
                      </div>
                    </section>
                  )}

                  <section className="takeover-preview-section">
                    <h5>{t('client.previewConfigWrites')}</h5>
                    <div className="takeover-preview-values">
                      {preview.config_writes.length ? (
                        preview.config_writes.map((write, index) => (
                          <code className="takeover-env-path" key={`${write.path}:${write.field}:${index}`}>
                            {write.path} · {write.field}
                          </code>
                        ))
                      ) : (
                        <span className="hint compact">{t('client.previewNone')}</span>
                      )}
                    </div>
                  </section>

                  <section className="takeover-preview-section">
                    <h5>{t('client.previewSkillDirs')}</h5>
                    <div className="takeover-preview-values">
                      {preview.skill_dirs.length ? (
                        preview.skill_dirs.map((dir) => (
                          <code className="takeover-env-path" key={dir}>{dir}</code>
                        ))
                      ) : (
                        <span className="hint compact">{t('client.previewNone')}</span>
                      )}
                    </div>
                  </section>

                  <section className="takeover-preview-section">
                    <h5>{t('client.previewRevert')}</h5>
                    <span className={preview.revertable ? 'badge ok inline' : 'badge stop inline'}>
                      {preview.revertable ? t('client.previewRevertYes') : t('client.previewRevertNo')}
                    </span>
                  </section>

                  {preview.warnings.length > 0 && (
                    <section className="takeover-preview-section takeover-preview-warnings">
                      <h5>{t('client.previewWarnings')}</h5>
                      {preview.warnings.map((warn, index) => (
                        <p className="agent-discover-warn" key={`${warn}:${index}`}>{warn}</p>
                      ))}
                    </section>
                  )}

                  {preview.note && <p className="hint compact takeover-preview-note">{preview.note}</p>}
                </div>
              </>
            )}
          </div>

          <div className="modal-actions takeover-preview-actions">
            <button type="button" className="ghost" onClick={onClose}>{t('common.close')}</button>
            <button
              type="button"
              className="ghost"
              disabled={!preview}
              onClick={() => void copySummary()}
            >
              <Copy size={14} />{t('client.previewCopy')}
            </button>
            {configured ? (
              <button
                type="button"
                className="danger"
                disabled={busy || loading || Boolean(error) || !preview}
                onClick={onRelease}
              >
                {t('client.previewConfirmRelease')}
              </button>
            ) : (
              <button
                type="button"
                className="primary"
                disabled={busy || loading || Boolean(error) || !preview}
                onClick={onApply}
              >
                {t('client.previewConfirmApply')}
              </button>
            )}
          </div>
        </div>
      </div>
    </ModalPortal>
  );
}

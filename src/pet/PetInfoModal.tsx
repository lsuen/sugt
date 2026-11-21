// 桌面精灵只读信息弹窗：只展示网关与精灵的当前事实，不做任何写操作、不做对话。
// 参照 TakeoverPreviewModal 的最佳实践：ModalPortal + Esc + 遮罩 onMouseDown
// （target===currentTarget）+ 焦点回归 + role="dialog" aria-modal="true"
// + 加载 / 错误 / 空三态 + footer 按钮。
// 作者：孙文龙 sun
import { useEffect, useRef, useState } from 'react';
import { ModalPortal } from '../ModalPortal';
import { t } from '../i18n';

export type PetInfoSnapshot = {
  stateLabel: string;
  active: number;
  success: number;
  failed: number;
  pose: string;
  actionHint: string;
  recent: string | null;
};

type Props = {
  snapshot: PetInfoSnapshot | null;
  onClose: () => void;
  onOpenSettings: () => void;
};

type Phase = 'loading' | 'ready' | 'error';

export function PetInfoModal({ snapshot, onClose, onOpenSettings }: Props) {
  const [phase, setPhase] = useState<Phase>('loading');
  const closeRef = useRef<HTMLButtonElement>(null);
  // 用布尔值做依赖，避免父级每次 render 都生成新快照对象而把 loading 态反复重置。
  const hasSnapshot = snapshot !== null;

  // 纯展示过渡：挂载后短暂 loading 再落到 ready / error；不发起任何 IO 或写操作。
  useEffect(() => {
    const timer = window.setTimeout(() => {
      setPhase(hasSnapshot ? 'ready' : 'error');
    }, 180);
    return () => window.clearTimeout(timer);
  }, [hasSnapshot]);

  // 焦点回归：打开时聚焦关闭按钮，卸载时把焦点还给触发元素。
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

  return (
    <ModalPortal>
      <div
        className="pet-info-overlay"
        onMouseDown={(event) => {
          if (event.target === event.currentTarget) onClose();
        }}
      >
        <div
          className="pet-info-dialog"
          role="dialog"
          aria-modal="true"
          aria-label={t('pet.info.title')}
        >
          <div className="pet-info-head">
            <h3 className="pet-info-title">{t('pet.info.title')}</h3>
            <button
              ref={closeRef}
              type="button"
              className="pet-info-close"
              onClick={onClose}
              aria-label={t('pet.info.close')}
            >
              ×
            </button>
          </div>

          <div className="pet-info-body">
            {/* 三态：加载 / 错误 / 空（最近消息为空时回落到 pet.info.empty） */}
            {phase === 'loading' && (
              <p className="pet-info-state">{t('pet.info.loading')}</p>
            )}
            {phase === 'error' && (
              <p className="pet-info-state">{t('pet.info.error')}</p>
            )}
            {phase === 'ready' && snapshot && (
              <dl className="pet-info-rows">
                <div className="pet-info-row">
                  <dt>{t('pet.info.state')}</dt>
                  <dd>{snapshot.stateLabel}</dd>
                </div>
                <div className="pet-info-row">
                  <dt>{t('pet.info.active')}</dt>
                  <dd>{snapshot.active}</dd>
                </div>
                <div className="pet-info-row">
                  <dt>{t('pet.info.success')}</dt>
                  <dd>{snapshot.success}</dd>
                </div>
                <div className="pet-info-row">
                  <dt>{t('pet.info.failed')}</dt>
                  <dd>{snapshot.failed}</dd>
                </div>
                <div className="pet-info-row">
                  <dt>{t('pet.info.pose')}</dt>
                  <dd>{snapshot.pose}</dd>
                </div>
                <div className="pet-info-row">
                  <dt>{t('pet.info.actionHint')}</dt>
                  <dd>{snapshot.actionHint}</dd>
                </div>
                <div className="pet-info-row">
                  <dt>{t('pet.info.recent')}</dt>
                  <dd>{snapshot.recent ?? t('pet.info.empty')}</dd>
                </div>
              </dl>
            )}
          </div>

          <div className="pet-info-actions">
            <button type="button" className="pet-info-btn" onClick={onOpenSettings}>
              {t('pet.info.openSettings')}
            </button>
            <button
              type="button"
              className="pet-info-btn pet-info-btn--primary"
              onClick={onClose}
            >
              {t('pet.info.close')}
            </button>
          </div>
        </div>
      </div>
    </ModalPortal>
  );
}

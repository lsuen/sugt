/** 通用确认弹窗。编写人员：孙文龙 sun（孙测测）。
 * 作用：把散落的原生 window.confirm 收编为统一的应用内确认框，
 * 用 ModalPortal 挂到 body，统一遮罩 / Esc / 焦点回归 / 危险操作提示。
 * 组件只负责「标题 + 后果说明 + 两个按钮」，真正的删除动作由 onConfirm 提供，
 * 便于调用方保持原有 run() / toast / reload 行为不变。 */
import React, { useEffect, useRef } from 'react';
import { TriangleAlert } from 'lucide-react';
import { ModalPortal } from './ModalPortal';
import { t } from './i18n';

export type ConfirmModalProps = {
  /** 是否展示，由调用方用组件 state 控制 */
  open: boolean;
  /** 主问题 / 标题，同时作为 dialog 的 aria-label */
  title: string;
  /** 后果说明，放在 body 区提示用户「删除后会发生什么」 */
  body?: string;
  /** 危险操作：确认按钮走 danger 样式，并追加一句不可撤销提示 */
  danger?: boolean;
  /** 确认执行中：禁用两个按钮，避免重复提交 */
  busy?: boolean;
  /** 确认按钮文案，默认取 common.confirmOk */
  confirmLabel?: string;
  onConfirm: () => void;
  onCancel: () => void;
};

export function ConfirmModal({
  open,
  title,
  body,
  danger = false,
  busy = false,
  confirmLabel,
  onConfirm,
  onCancel,
}: ConfirmModalProps) {
  const confirmRef = useRef<HTMLButtonElement>(null);

  // 焦点回归：open 置真时记住触发元素并把焦点移入确认按钮，
  // 关闭（open 置假）时把焦点还回去，键盘用户不会丢焦点。
  useEffect(() => {
    if (!open) return;
    const previous = document.activeElement as HTMLElement | null;
    confirmRef.current?.focus();
    return () => previous?.focus?.();
  }, [open]);

  // Esc 关闭：只在本弹窗打开期间挂载监听，避免影响同页其它弹窗。
  useEffect(() => {
    if (!open) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onCancel();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open, onCancel]);

  if (!open) return null;

  return (
    <ModalPortal>
      <div
        className="modal-overlay"
        onMouseDown={(event) => {
          // 只在点到遮罩本身时关闭，点弹窗内部不关闭。
          if (event.target === event.currentTarget) onCancel();
        }}
      >
        <div
          className="modal modal-compact"
          role="dialog"
          aria-modal="true"
          aria-label={title}
        >
          <div className="modal-head">
            <h3 style={{ display: 'inline-flex', alignItems: 'center', gap: 8 }}>
              {danger ? <TriangleAlert size={16} aria-hidden="true" /> : null}
              {title}
            </h3>
          </div>

          <div className="modal-body">
            {body ? <p className="hint compact">{body}</p> : null}
            {danger ? (
              <p className="agent-discover-warn">{t('common.confirmDanger')}</p>
            ) : null}
          </div>

          <div className="modal-actions">
            <button type="button" className="ghost" disabled={busy} onClick={onCancel}>
              {t('common.cancel')}
            </button>
            <button
              ref={confirmRef}
              type="button"
              className={danger ? 'danger' : 'primary'}
              disabled={busy}
              onClick={onConfirm}
            >
              {confirmLabel ?? t('common.confirmOk')}
            </button>
          </div>
        </div>
      </div>
    </ModalPortal>
  );
}

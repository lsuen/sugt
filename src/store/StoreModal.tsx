import React from 'react';
import { X } from 'lucide-react';

type Props = {
  title: string;
  onClose: () => void;
  children: React.ReactNode;
  wide?: boolean;
  /** 表单弹窗默认 false，防止误点遮罩丢失内容 */
  closeOnOverlay?: boolean;
};

export function StoreModal({ title, onClose, children, wide, closeOnOverlay = false }: Props) {
  return (
    <div
      className="modal-overlay"
      onClick={closeOnOverlay ? onClose : undefined}
    >
      <div className={`modal${wide ? ' modal-wide' : ''}`} onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>{title}</h3>
          <button type="button" className="ghost tiny-btn" onClick={onClose} aria-label="关闭">
            <X size={16} />
          </button>
        </div>
        <div className="modal-body">{children}</div>
      </div>
    </div>
  );
}

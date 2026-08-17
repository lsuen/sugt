import React, { useEffect } from 'react';
import { X } from 'lucide-react';

type Props = {
  open: boolean;
  onClose: () => void;
  code: string;
};

export function CodeExampleModal({ open, onClose, code }: Props) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal modal-compact" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>代码范例</h3>
          <button type="button" className="tiny icon-only" onClick={onClose}>
            <X size={14} />
          </button>
        </div>
        <div className="modal-body">
          <pre className="code-sample">{code}</pre>
        </div>
      </div>
    </div>
  );
}

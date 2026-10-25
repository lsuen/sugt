import React from 'react';
import { createPortal } from 'react-dom';

/** Mount overlays on document.body so theme shell z-index/stacking cannot trap fixed modals. */
export function ModalPortal({ children }: { children: React.ReactNode }) {
  if (typeof document === 'undefined') return null;
  return createPortal(children, document.body);
}

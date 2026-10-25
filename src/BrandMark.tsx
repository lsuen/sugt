import React from 'react';
import brandLogo from './assets/brand-logo.png';

type Props = {
  size?: 'sm' | 'md' | 'lg';
  className?: string;
};

/** Theme-aware brand mark: new rounded icon + accent ring that follows the active skin. */
export function BrandMark({ size = 'md', className = '' }: Props) {
  return (
    <span className={`brand-mark brand-mark--${size} ${className}`.trim()} aria-hidden>
      <img className="brand-mark-img" src={brandLogo} alt="" draggable={false} />
    </span>
  );
}

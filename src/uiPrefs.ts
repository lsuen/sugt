/** Lightweight UI prefs (localStorage). Defaults favor discoverability. */

const STATUS_BAR_KEY = 'sugt_status_bar';

/** Default: status bar ON */
export function loadStatusBarEnabled(): boolean {
  try {
    const v = localStorage.getItem(STATUS_BAR_KEY);
    if (v === null) return true;
    return v === '1' || v === 'true';
  } catch {
    return true;
  }
}

export function persistStatusBarEnabled(enabled: boolean): void {
  try {
    localStorage.setItem(STATUS_BAR_KEY, enabled ? '1' : '0');
  } catch {
    /* ignore */
  }
}

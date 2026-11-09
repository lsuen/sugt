// 小su 授权模式：安全 / 完整 / 首选完整。
// 作者：sun / 孙文龙
// 替代旧版布尔「自动同意」；兼容 localStorage 旧键。

export type MeAuthMode = 'ask' | 'auto' | 'trusted';

const AUTH_KEY = 'sugt_me_auth_mode';
const LEGACY_AUTO_KEY = 'sugt_me_auto_write';

export const ME_AUTH_MODES: MeAuthMode[] = ['ask', 'auto', 'trusted'];

export function parseMeAuthMode(raw: string | null | undefined): MeAuthMode {
  const v = (raw || '').trim().toLowerCase();
  if (v === 'auto' || v === 'complete' || v === 'full') return 'auto';
  if (v === 'trusted' || v === 'yolo' || v === 'bypass') return 'trusted';
  if (v === '1' || v === 'true' || v === 'yes' || v === 'on') return 'auto';
  return 'ask';
}

export function loadMeAuthMode(): MeAuthMode {
  try {
    const saved = localStorage.getItem(AUTH_KEY);
    if (saved) return parseMeAuthMode(saved);
    // 迁移旧布尔开关
    if (localStorage.getItem(LEGACY_AUTO_KEY) === '1') return 'auto';
  } catch {
    /* ignore */
  }
  return 'ask';
}

export function saveMeAuthMode(mode: MeAuthMode): void {
  try {
    localStorage.setItem(AUTH_KEY, mode);
    localStorage.setItem(LEGACY_AUTO_KEY, mode === 'ask' ? '0' : '1');
  } catch {
    /* ignore */
  }
}

export function meAuthModeLabelKey(mode: MeAuthMode): `me.auth.${MeAuthMode}` {
  return `me.auth.${mode}`;
}

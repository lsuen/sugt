// 右键菜单独立窗口。不挂在精灵透明窗里，也不改那扇窗的大小。
import { useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke, emit, listen } from './transport';
import { PetContextMenu } from './pet/PetContextMenu';
import {
  petEnabledPatch,
  petSizePatch,
  type OverlayConfigView,
} from './pet/config';
// 侧效拉齐 tokens + 内置 themes（含 shell-theme）；html/body 透明由 menu.html !important 压住。
import './theme/registry';
import { loadStoredThemeId } from './theme/applyTheme';
import './pet/pet.css';

// 与主窗同一 localStorage key；独立 webview 不会自动继承 data-theme。
document.documentElement.dataset.theme = loadStoredThemeId();

function closeMenu() {
  void invoke('pet_close_menu').catch(() => undefined);
}

function MenuApp() {
  const [cfg, setCfg] = useState<OverlayConfigView | null>(null);

  useEffect(() => {
    void invoke<OverlayConfigView>('get_overlay_config').then(setCfg).catch(() => undefined);
    const un = listen<OverlayConfigView>('sugt://overlay-config-changed', (event) => {
      setCfg(event.payload);
    });
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') closeMenu();
    };
    // 等窗口真正 focused 过一次再允许 blur 关闭，避免 Windows 新建窗先 blur 把菜单掐掉。
    let armed = false;
    let sawFocus = document.hasFocus();
    const armDelay = /Windows/i.test(navigator.userAgent) ? 900 : 400;
    const arm = window.setTimeout(() => {
      if (sawFocus || document.hasFocus()) armed = true;
    }, armDelay);
    const onFocus = () => {
      sawFocus = true;
      armed = true;
    };
    const onBlur = () => {
      if (armed && sawFocus) closeMenu();
    };
    window.addEventListener('focus', onFocus);
    window.addEventListener('keydown', onKey);
    window.addEventListener('blur', onBlur);
    return () => {
      window.clearTimeout(arm);
      un.then((fn) => fn()).catch(() => undefined);
      window.removeEventListener('keydown', onKey);
      window.removeEventListener('focus', onFocus);
      window.removeEventListener('blur', onBlur);
    };
  }, []);

  if (!cfg) return null;
  const patch = (next: Partial<OverlayConfigView>) => {
    // 只发增量，后端按当前配置 merge；不覆盖设置页 / 透明窗的其它写入。
    void invoke('patch_overlay_config', { patch: next }).catch(() => undefined);
  };

  return (
    <PetContextMenu
      cfg={cfg}
      onSettings={() => {
        void invoke('open_settings_tab', { tab: 'pet' }).catch(() => undefined);
        void invoke('focus_main_window').catch(() => undefined);
      }}
      onShowInfo={() => {
        void emit('sugt://pet-show-info').catch(() => undefined);
      }}
      onToggleQuiet={() => patch({ pet_quiet: !cfg.pet_quiet })}
      onToggleRoam={() => patch({ pet_roam: cfg.pet_roam === false })}
      onSize={(size) => patch(petSizePatch(size))}
      onDock={() => patch({ x: -1, y: -1 })}
      onHide={() => patch(petEnabledPatch(false))}
      onClose={closeMenu}
    />
  );
}

createRoot(document.getElementById('root')!).render(<MenuApp />);

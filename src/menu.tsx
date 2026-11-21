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
import './pet/pet.css';

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
    let armed = false;
    const arm = window.setTimeout(() => {
      armed = true;
    }, 400);
    const onBlur = () => {
      if (armed) closeMenu();
    };
    window.addEventListener('keydown', onKey);
    window.addEventListener('blur', onBlur);
    return () => {
      window.clearTimeout(arm);
      un.then((fn) => fn()).catch(() => undefined);
      window.removeEventListener('keydown', onKey);
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

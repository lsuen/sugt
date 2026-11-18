// 桌面精灵（小酥）透明窗入口：置顶 + 透明 + 元素级鼠标穿透。
// 流量 HUD 悬浮窗已废弃，本入口只渲染 PetStage，不再有模式切换与 HUD 分支。
// 窗口 label / 命令名 / 事件名沿用历史标识，保证拖动、右键、视线、光标链路零改动。
import { useEffect, useState } from 'react';
import type { MouseEvent as ReactMouseEvent } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import './overlay.css';
import { PetStage } from './pet/PetStage';
import type { OverlayConfigView, OverlayStatus } from './pet/config';

function Overlay() {
  const [cfg, setCfg] = useState<OverlayConfigView | null>(null);
  const [status, setStatus] = useState<OverlayStatus | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    invoke<OverlayConfigView>('get_overlay_config')
      .then(setCfg)
      .catch(() => undefined);
    const un = listen<OverlayConfigView>('sugt://overlay-config-changed', (e) => setCfg(e.payload));
    return () => {
      un.then((fn) => fn()).catch(() => undefined);
    };
  }, []);

  useEffect(() => {
    // 精灵状态只认真实在途请求；轮询本身不产生任何状态通知。
    const poll = () => invoke<OverlayStatus>('get_status').then(setStatus).catch(() => undefined);
    poll();
    const timer = window.setInterval(poll, 2000);
    return () => window.clearInterval(timer);
  }, []);

  const savePos = async () => {
    if (!cfg || busy) return;
    setBusy(true);
    try {
      const pos = await getCurrentWindow().outerPosition();
      // 只发本次改动的字段，不覆盖设置页 / 菜单里的其它配置。
      await invoke('patch_overlay_config', { patch: { x: pos.x, y: pos.y, edit: false } });
    } catch {
      // 窗口被关闭等场景：静默即可
    } finally {
      setBusy(false);
    }
  };

  const startDrag = (e: ReactMouseEvent<HTMLSpanElement>) => {
    e.preventDefault();
    void invoke('overlay_start_drag').catch(() => undefined);
  };

  if (!cfg) return null;
  return (
    <PetStage
      cfg={cfg}
      status={status}
      busy={busy}
      onSave={() => void savePos()}
      onDrag={startDrag}
    />
  );
}

createRoot(document.getElementById('root')!).render(<Overlay />);

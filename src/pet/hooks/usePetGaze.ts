// 桌面精灵视线跟随 + 元素级鼠标穿透。
// 穿透模式下 WebView 收不到 pointermove，因此统一轮询后端 pet_global_cursor；
// 视线与「光标是否落在立绘/菜单上」共用同一份光标样本，避免重复 IPC。
//
// 铁律：只有立绘 / 右键菜单 / 拖动中才 pet_set_clickable(true)。
// 绝不能因 menuOpen 就把整窗解穿透——那会把侧栏空白与透明边距变成「透明墙」。
import { useEffect, useRef, useState } from 'react';
import type { MutableRefObject, RefObject } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { clamp, pointInsideFigure } from '../config';

type Props = {
  gazeEnabled: boolean;
  interactive: boolean;
  edit: boolean;
  figureRef: RefObject<HTMLDivElement>;
  stageRef: RefObject<HTMLDivElement>;
  dragActive: MutableRefObject<boolean>;
  menuOpen: MutableRefObject<boolean>;
  onNearTouch: () => void;
  onNear: () => void;
  /** 光标离开立绘+菜单时关闭右键菜单（由 PetStage 提供） */
  onMenuDismiss?: () => void;
};

export function usePetGaze({
  gazeEnabled,
  interactive,
  edit,
  figureRef,
  stageRef,
  dragActive,
  menuOpen,
  onNearTouch,
  onNear,
  onMenuDismiss,
}: Props) {
  const gazeRef = useRef({ x: 0, y: 0, near: 0 });
  const [nearListening, setNearListening] = useState(false);
  const [nearWalking, setNearWalking] = useState(false);
  // 上次已发送给后端的「可点击」状态；仅在变化时打 IPC，避免 50ms 空转。
  const clickableRef = useRef(false);
  const onMenuDismissRef = useRef(onMenuDismiss);
  onMenuDismissRef.current = onMenuDismiss;

  useEffect(() => {
    const stage = stageRef.current;
    if (!stage) return;
    // 视线与穿透都关闭时无需轮询，整窗保持穿透。
    if (!gazeEnabled && !interactive) return;

    if (!gazeEnabled) {
      stage.style.setProperty('--gaze-x', '0');
      stage.style.setProperty('--gaze-y', '0');
      stage.style.setProperty('--near', '0');
      stage.classList.remove('nearby', 'pet-listening', 'pet-walking');
      setNearListening(false);
      setNearWalking(false);
    }

    let raf = 0;
    let pollTimer = 0;
    let scanTimer = 0;
    let resetTimer = 0;
    let menuLeaveTimer = 0;
    let lastLocalMove = 0;
    let cancelled = false;
    const target = { x: 0, y: 0, near: 0 };
    const current = { x: 0, y: 0, near: 0 };
    const LERP = 0.18;

    const setClickable = (next: boolean) => {
      if (clickableRef.current === next) return;
      clickableRef.current = next;
      void invoke('pet_set_clickable', { enabled: next }).catch(() => undefined);
    };

    function paint() {
      raf = 0;
      const { x, y, near } = current;
      stage!.style.setProperty('--gaze-x', x.toFixed(3));
      stage!.style.setProperty('--gaze-y', y.toFixed(3));
      stage!.style.setProperty('--near', near.toFixed(3));
      stage!.classList.toggle('nearby', near > 0.12);
      const listening = stage!.dataset.state === 'idle' && near > 0.45;
      const walking = stage!.dataset.state === 'idle' && near > 0.18 && near <= 0.45;
      stage!.classList.toggle('pet-listening', listening);
      stage!.classList.toggle('pet-walking', walking);
      setNearListening((prev) => (prev === listening ? prev : listening));
      setNearWalking((prev) => (prev === walking ? prev : walking));
      if (near > 0.3) onNear();
      if (
        Math.abs(target.x - x) > 0.004
        || Math.abs(target.y - y) > 0.004
        || Math.abs(target.near - near) > 0.004
      ) {
        raf = requestAnimationFrame(tick);
      }
    }

    function tick() {
      if (cancelled) return;
      current.x += (target.x - current.x) * LERP;
      current.y += (target.y - current.y) * LERP;
      current.near += (target.near - current.near) * LERP;
      paint();
    }

    const aim = (x: number, y: number, near: number) => {
      target.x = x;
      target.y = y;
      target.near = near;
      gazeRef.current = { x, y, near };
      const needs = Math.abs(target.x - current.x) > 0.004
        || Math.abs(target.y - current.y) > 0.004
        || Math.abs(target.near - current.near) > 0.004;
      if (needs && !raf && !cancelled) raf = requestAnimationFrame(tick);
    };

    const applyFromClient = (clientX: number, clientY: number) => {
      const rect = figureRef.current?.getBoundingClientRect();
      if (!rect) return;
      const cx = rect.left + rect.width * 0.55;
      const cy = rect.top + rect.height * 0.32;
      const distance = Math.hypot(clientX - cx, clientY - cy);
      const near = clamp(1 - distance / 280, 0, 1);
      const x = clamp((clientX - cx) / (rect.width * 0.9), -1, 1);
      const y = clamp((clientY - cy) / (rect.height * 0.6), -1, 1);
      aim(x, y, near);
    };

    const hitMenu = (clientX: number, clientY: number) => {
      const menu = stage!.querySelector('.pet-menu');
      if (!menu) return false;
      return pointInsideFigure(clientX, clientY, menu.getBoundingClientRect());
    };

    const updateClickThrough = (clientX: number, clientY: number) => {
      // 编辑态始终可点击（保存按钮 / 拖柄不在立绘命中框内）
      if (edit) {
        setClickable(true);
        return;
      }
      if (!interactive) {
        setClickable(false);
        return;
      }
      const rect = figureRef.current?.getBoundingClientRect();
      const overFigure = rect ? pointInsideFigure(clientX, clientY, rect) : false;
      const overMenu = hitMenu(clientX, clientY);
      // 仅命中立绘或菜单，或正在拖动 —— 空白侧栏 / 透明边距永远穿透
      const next = overFigure || overMenu || dragActive.current;
      setClickable(next);

      // 菜单打开后光标离开立绘+菜单：短暂延迟后关掉，避免菜单悬空挡视线
      // 且绝不靠「整窗可点」去接点外关闭。
      if (menuOpen.current) {
        if (overFigure || overMenu) {
          window.clearTimeout(menuLeaveTimer);
          menuLeaveTimer = 0;
        } else if (!menuLeaveTimer) {
          menuLeaveTimer = window.setTimeout(() => {
            menuLeaveTimer = 0;
            if (!cancelled) onMenuDismissRef.current?.();
          }, 280);
        }
      }
    };

    let scale = 1;
    const refreshScale = () => {
      void getCurrentWindow().scaleFactor()
        .then((factor) => {
          if (!cancelled) scale = Number(factor) || 1;
        })
        .catch(() => undefined);
    };
    refreshScale();
    window.addEventListener('resize', refreshScale);

    const schedulePoll = () => {
      if (cancelled) return;
      pollTimer = window.setTimeout(pollGlobal, 50);
    };

    const pollGlobal = () => {
      if (cancelled) return;
      if (document.hidden || Date.now() - lastLocalMove < 80) {
        schedulePoll();
        return;
      }
      void invoke<[number, number]>('pet_global_cursor')
        .then((cursor) => {
          if (cancelled) return;
          const factor = window.devicePixelRatio > 0 ? window.devicePixelRatio : scale;
          const clientX = cursor[0] / factor;
          const clientY = cursor[1] / factor;
          if (gazeEnabled) applyFromClient(clientX, clientY);
          if (interactive) updateClickThrough(clientX, clientY);
        })
        .catch(() => undefined)
        .finally(schedulePoll);
    };

    const onMove = (event: PointerEvent) => {
      lastLocalMove = Date.now();
      onNearTouch();
      if (gazeEnabled) applyFromClient(event.clientX, event.clientY);
      if (interactive) updateClickThrough(event.clientX, event.clientY);
    };

    const onLeave = () => {
      if (gazeEnabled && Date.now() - lastLocalMove < 120) return;
      if (gazeEnabled) aim(0, 0, 0);
      if (interactive && !dragActive.current) setClickable(false);
      if (menuOpen.current) onMenuDismissRef.current?.();
    };

    const scan = () => {
      if (gazeEnabled && gazeRef.current.near < 0.06 && !document.hidden) {
        aim((Math.random() * 2 - 1) * 0.42, (Math.random() * 2 - 1) * 0.22, 0);
        window.clearTimeout(resetTimer);
        resetTimer = window.setTimeout(() => aim(0, 0, 0), 1500);
      }
      scanTimer = window.setTimeout(scan, 4200 + Math.random() * 4200);
    };

    document.addEventListener('pointermove', onMove, { passive: true });
    document.addEventListener('pointerleave', onLeave);
    pollGlobal();
    if (gazeEnabled) scanTimer = window.setTimeout(scan, 2600);

    return () => {
      cancelled = true;
      document.removeEventListener('pointermove', onMove);
      document.removeEventListener('pointerleave', onLeave);
      window.removeEventListener('resize', refreshScale);
      if (raf) cancelAnimationFrame(raf);
      window.clearTimeout(pollTimer);
      window.clearTimeout(scanTimer);
      window.clearTimeout(resetTimer);
      window.clearTimeout(menuLeaveTimer);
      stage.classList.remove('nearby', 'pet-listening', 'pet-walking');
      setClickable(false);
    };
  }, [gazeEnabled, interactive, edit, figureRef, stageRef, dragActive, menuOpen, onNearTouch, onNear]);

  return { gazeRef, nearListening, nearWalking };
}

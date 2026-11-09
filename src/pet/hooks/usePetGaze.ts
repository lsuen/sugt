// 桌面精灵视线跟随 + 可选元素级鼠标穿透。
// 穿透模式下 WebView 收不到 pointermove，因此统一轮询后端 pet_global_cursor；
// 视线与「光标是否落在立绘/菜单上」共用同一份光标样本，避免重复 IPC。
//
// 默认（pet_click_through=false）整窗常驻可交互：拖拽、右键菜单都不受阻。
// 显式开启穿透后，铁律是只有立绘 / 右键菜单 / 拖动中才 pet_set_clickable(true)；
// 绝不能因 menuOpen 就把整窗解穿透——那会把侧栏空白与透明边距变成「透明墙」。
import { useEffect, useRef, useState } from 'react';
import type { MutableRefObject, RefObject } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { clamp, figureHitRect, FULL_HITBOX, pointInsideFigure } from '../config';
import {
  PET_AUTO_ACTIONS,
  PET_HITBOXES,
  PET_IMAGE_SIZE,
  type PetPoseName,
} from '../petManifest.generated';

// 眼→头→身分级跟随：三层 lerp 递减，慢层永远追不上快层，形成自然的惯性延迟。
// 参数由 manifest.autoActions.liveliness.gaze 单一真相源注入。
const GAZE_LERP = PET_AUTO_ACTIONS.liveliness.gaze;

type Props = {
  gazeEnabled: boolean;
  interactive: boolean;
  edit: boolean;
  /** 鼠标穿透设置项：默认 false，整窗可交互；true 时启用元素级动态穿透 */
  clickThrough: boolean;
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
  clickThrough,
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
    // 穿透是设置项且默认关闭：关闭时整窗常驻可交互，拖拽/右键菜单不受影响；
    // 只有显式开启时才切回「按立绘 alpha 命中框」的元素级穿透。
    const passthrough = clickThrough && interactive && !edit;

    const setClickable = (next: boolean) => {
      if (clickableRef.current === next) return;
      clickableRef.current = next;
      void invoke('pet_set_clickable', { enabled: next }).catch(() => undefined);
    };
    if (!passthrough) setClickable(true);
    // 视线与穿透都关闭时无需轮询，整窗保持可交互。
    if (!gazeEnabled && !interactive) return;

    if (!gazeEnabled) {
      stage.style.setProperty('--gaze-x', '0');
      stage.style.setProperty('--gaze-y', '0');
      stage.style.setProperty('--head-x', '0');
      stage.style.setProperty('--head-y', '0');
      stage.style.setProperty('--body-x', '0');
      stage.style.setProperty('--body-y', '0');
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
    // 三层独立状态：眼最快、头居中、身最慢；用最慢层判定动画是否收敛。
    const eye = { x: 0, y: 0 };
    const head = { x: 0, y: 0 };
    const bodyLayer = { x: 0, y: 0 };
    const current = { near: 0 };

    function paint() {
      raf = 0;
      const el = stage!;
      el.style.setProperty('--gaze-x', eye.x.toFixed(3));
      el.style.setProperty('--gaze-y', eye.y.toFixed(3));
      el.style.setProperty('--head-x', head.x.toFixed(3));
      el.style.setProperty('--head-y', head.y.toFixed(3));
      el.style.setProperty('--body-x', bodyLayer.x.toFixed(3));
      el.style.setProperty('--body-y', bodyLayer.y.toFixed(3));
      el.style.setProperty('--near', current.near.toFixed(3));
      const near = current.near;
      el.classList.toggle('nearby', near > 0.12);
      const listening = el.dataset.state === 'idle' && near > 0.45;
      const walking = el.dataset.state === 'idle' && near > 0.18 && near <= 0.45;
      el.classList.toggle('pet-listening', listening);
      el.classList.toggle('pet-walking', walking);
      setNearListening((prev) => (prev === listening ? prev : listening));
      setNearWalking((prev) => (prev === walking ? prev : walking));
      if (near > 0.3) onNear();
      if (
        Math.abs(target.x - bodyLayer.x) > 0.004
        || Math.abs(target.y - bodyLayer.y) > 0.004
        || Math.abs(target.near - near) > 0.004
      ) {
        raf = requestAnimationFrame(tick);
      }
    }

    function tick() {
      if (cancelled) return;
      eye.x += (target.x - eye.x) * GAZE_LERP.eyeLerp;
      eye.y += (target.y - eye.y) * GAZE_LERP.eyeLerp;
      head.x += (target.x - head.x) * GAZE_LERP.headLerp;
      head.y += (target.y - head.y) * GAZE_LERP.headLerp;
      bodyLayer.x += (target.x - bodyLayer.x) * GAZE_LERP.bodyLerp;
      bodyLayer.y += (target.y - bodyLayer.y) * GAZE_LERP.bodyLerp;
      current.near += (target.near - current.near) * GAZE_LERP.eyeLerp;
      paint();
    }

    const aim = (x: number, y: number, near: number) => {
      target.x = x;
      target.y = y;
      target.near = near;
      gazeRef.current = { x, y, near };
      const needs = Math.abs(target.x - bodyLayer.x) > 0.004
        || Math.abs(target.y - bodyLayer.y) > 0.004
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
      // 默认不穿透：整窗常驻可交互，无需再做立绘命中框判定。
      if (!passthrough) {
        setClickable(true);
        return;
      }
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
      // 按当前姿势的 alpha 包围盒收紧命中区：透明边距不再变成「透明点击墙」。
      // 外扩 14px：右键落点更容易命中，避免「看着在人上却仍穿透」。
      const pose = stage!.querySelector<HTMLElement>('.pet-loop.is-active')
        ?.dataset.loop as PetPoseName | undefined;
      const hitBox = pose && pose in PET_HITBOXES
        ? PET_HITBOXES[pose]
        : FULL_HITBOX;
      const figureBox = rect
        ? figureHitRect(rect, hitBox, PET_IMAGE_SIZE, 14)
        : undefined;
      const overFigure = figureBox
        ? pointInsideFigure(clientX, clientY, figureBox)
        : false;
      const overMenu = hitMenu(clientX, clientY);
      // 贴脸时提前解穿透，否则右键按下瞬间窗口还是穿透，contextmenu 进不来
      const approaching = !menuOpen.current && gazeRef.current.near > 0.4;
      const next = overFigure || overMenu || dragActive.current || approaching;
      setClickable(next);

      // 菜单打开后离开立绘+菜单才关；给 450ms 宽限，避免移向菜单项途中被关掉
      if (menuOpen.current) {
        if (overFigure || overMenu) {
          window.clearTimeout(menuLeaveTimer);
          menuLeaveTimer = 0;
        } else if (!menuLeaveTimer) {
          menuLeaveTimer = window.setTimeout(() => {
            menuLeaveTimer = 0;
            if (!cancelled) onMenuDismissRef.current?.();
          }, 450);
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
          if (passthrough) updateClickThrough(clientX, clientY);
        })
        .catch(() => undefined)
        .finally(schedulePoll);
    };

    const onMove = (event: PointerEvent) => {
      lastLocalMove = Date.now();
      onNearTouch();
      if (gazeEnabled) applyFromClient(event.clientX, event.clientY);
      if (passthrough) updateClickThrough(event.clientX, event.clientY);
    };

    const onLeave = () => {
      if (gazeEnabled && Date.now() - lastLocalMove < 120) return;
      if (gazeEnabled) aim(0, 0, 0);
      if (passthrough && !dragActive.current) setClickable(false);
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
      setClickable(!passthrough);
    };
  }, [gazeEnabled, interactive, edit, clickThrough, figureRef, stageRef,
      dragActive, menuOpen, onNearTouch, onNear]);

  return { gazeRef, nearListening, nearWalking };
}

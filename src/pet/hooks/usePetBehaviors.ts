// 桌面精灵行为：随机眨眼、开场招呼、闲置差分、自主走动、随机转身。
// 这些低频行为共享同一批 refs（petStateRef / dragActive / lastUserTouch / gazeRef），
// 合并在一个 hook 内维护，避免拆散强耦合的状态机。
// v1.1：间隔/姿势池/时长全部来自 manifest.autoActions 的纯节奏器，语义层常量不进设置页。
import { useEffect, useRef, useState } from 'react';
import type { MutableRefObject } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { currentMonitor, getCurrentWindow } from '@tauri-apps/api/window';
import type { TKey } from '../../i18n';
import { clamp, easeInOut, type Bubble } from '../config';
import {
  AUTO,
  nextBlinkAt,
  nextFacingAt,
  nextIdleVariantAt,
  nextWanderAt,
  pickIdleVariant,
  type PetPose,
  type PetState,
} from '../petState';

/** 走动朝向：复用现有 walk 素材，左右用 CSS 镜像，不新增四向立绘。 */
export type PetFacing = 'left' | 'right';
const PET_IDLE_BUBBLES: TKey[] = [
  'pet.bubble.idle1', 'pet.bubble.idle2', 'pet.bubble.idle3',
];

type Props = {
  blinkEnabled: boolean;
  roamEnabled: boolean;
  still: boolean;
  petState: PetState;
  petStateRef: MutableRefObject<PetState>;
  dragActive: MutableRefObject<boolean>;
  lastUserTouch: MutableRefObject<number>;
  gazeRef: MutableRefObject<{ x: number; y: number; near: number }>;
  playIdlePose: (pose: PetPose, ms: number) => void;
  pushBubble: (text: string, kind: Bubble['kind']) => void;
  t: (key: TKey) => string;
};

export function usePetBehaviors({
  blinkEnabled,
  roamEnabled,
  still,
  petState,
  petStateRef,
  dragActive,
  lastUserTouch,
  gazeRef,
  playIdlePose,
  pushBubble,
  t,
}: Props) {
  const [blinking, setBlinking] = useState(false);
  const [greeting, setGreeting] = useState(false);
  const [roaming, setRoaming] = useState(false);
  const [facing, setFacing] = useState<PetFacing>('right');

  const blinkTimers = useRef<number[]>([]);
  const greeted = useRef(false);
  const lastIdleBubble = useRef(0);

  // 随机眨眼；sleep/offline 或关闭 pet_blink 时不调度
  useEffect(() => {
    if (!blinkEnabled || petState === 'sleep' || petState === 'offline') {
      setBlinking(false);
      blinkTimers.current.forEach(window.clearTimeout);
      blinkTimers.current = [];
      return;
    }
    let cancelled = false;
    const later = (fn: () => void, ms: number) => {
      const id = window.setTimeout(() => {
        blinkTimers.current = blinkTimers.current.filter((item) => item !== id);
        fn();
      }, ms);
      blinkTimers.current.push(id);
    };
    const schedule = () =>
      later(() => {
        if (cancelled) return;
        setBlinking(true);
        later(() => {
          setBlinking(false);
          if (Math.random() < 0.18) {
            later(() => {
              setBlinking(true);
              later(() => setBlinking(false), 115);
            }, 230);
          }
          schedule();
        }, 125);
      }, nextBlinkAt({ now: 0 }));
    schedule();
    return () => {
      cancelled = true;
      blinkTimers.current.forEach(window.clearTimeout);
      blinkTimers.current = [];
    };
  }, [blinkEnabled, petState]);

  // 开场打招呼：只在精灵真实可见、系统未要求减少动效时触发一次。
  useEffect(() => {
    if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;
    let cancelled = false;
    let timer = 0;
    const greet = () => {
      if (cancelled || greeted.current || document.hidden || still) return;
      greeted.current = true;
      setGreeting(true);
      pushBubble(t('pet.bubble.hello'), 'info');
      timer = window.setTimeout(() => setGreeting(false), 2400);
    };
    greet();
    document.addEventListener('visibilitychange', greet);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
      document.removeEventListener('visibilitychange', greet);
    };
  }, [pushBubble, still, t]);

  // 闲置互动：只在 idle、用户久未触碰、窗口可见时随机播一个已落包的差分姿。
  useEffect(() => {
    let cancelled = false;
    let timer = 0;
    const later = (fn: () => void, ms: number) => {
      timer = window.setTimeout(() => {
        if (!cancelled) fn();
      }, ms);
    };
    const tick = () => {
      if (cancelled) return;
      const quietFor = Date.now() - lastUserTouch.current;
      if (
        !document.hidden && !still && petStateRef.current === 'idle'
        && !dragActive.current && quietFor > 6_000
      ) {
        // 姿势池与时长都来自 manifest.autoActions，加动作只改清单。
        playIdlePose(pickIdleVariant(), AUTO.idleVariant.holdMs);
        if (Math.random() < 0.34 && Date.now() - lastIdleBubble.current > 90_000) {
          lastIdleBubble.current = Date.now();
          const key = PET_IDLE_BUBBLES[Math.floor(Math.random() * PET_IDLE_BUBBLES.length)];
          pushBubble(t(key), 'info');
        }
      }
      later(tick, nextIdleVariantAt({ now: 0 }));
    };
    later(tick, 6_000 + Math.random() * 5_000);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [still, playIdlePose, pushBubble, petStateRef, dragActive, lastUserTouch, t]);

  // 自主走动：在当前显示器可用区内随机落点（整屏，非小范围漂），
  // 后端 pet_roam_to 再硬夹边；左右朝向用 CSS 镜像现有 walk 素材。
  useEffect(() => {
    if (!roamEnabled || still) return;
    let cancelled = false;
    let timer = 0;
    const schedule = (ms: number) => {
      window.clearTimeout(timer);
      timer = window.setTimeout(tick, ms);
    };
    const finish = () => {
      setRoaming(false);
      void invoke('pet_finish_drag', { snap: false }).catch(() => undefined);
      schedule(nextWanderAt({ now: 0 }));
    };
    const pickTarget = (
      start: { x: number; y: number },
      winW: number,
      winH: number,
      mon: { x: number; y: number; w: number; h: number; scale: number },
    ) => {
      const margin = Math.round(16 * Math.max(1, mon.scale));
      const minX = mon.x + margin;
      const minY = mon.y + margin;
      const maxX = Math.max(minX, mon.x + mon.w - winW - margin);
      const maxY = Math.max(minY, mon.y + mon.h - winH - margin);
      const spanX = Math.max(0, maxX - minX);
      const spanY = Math.max(0, maxY - minY);
      const minDist = 140 * Math.max(1, mon.scale);
      let target = { x: start.x, y: start.y };
      for (let attempt = 0; attempt < 6; attempt += 1) {
        const x = Math.round(minX + Math.random() * spanX);
        const y = Math.round(minY + Math.random() * spanY);
        if (Math.hypot(x - start.x, y - start.y) >= minDist || attempt === 5) {
          target = { x, y };
          break;
        }
      }
      return target;
    };
    const tick = () => {
      if (cancelled || document.hidden) {
        schedule(12_000);
        return;
      }
      if (
        petStateRef.current !== 'idle' || dragActive.current
        || Date.now() - lastUserTouch.current < 8_000
        || gazeRef.current.near > 0.45
      ) {
        schedule(14_000);
        return;
      }
      void (async () => {
        let start: { x: number; y: number };
        let winSize: { width: number; height: number };
        let mon: { x: number; y: number; w: number; h: number; scale: number };
        try {
          const win = getCurrentWindow();
          const [pos, size, monitor, factor] = await Promise.all([
            win.outerPosition(),
            win.outerSize(),
            currentMonitor(),
            win.scaleFactor(),
          ]);
          start = { x: pos.x, y: pos.y };
          winSize = { width: size.width, height: size.height };
          const scale = Number(factor) || 1;
          if (monitor) {
            mon = {
              x: monitor.position.x,
              y: monitor.position.y,
              w: monitor.size.width,
              h: monitor.size.height,
              scale: Number(monitor.scaleFactor) || scale,
            };
          } else {
            // 读不到显示器时退化为「当前位置附近大步」，仍交给后端夹边
            mon = {
              x: pos.x - 800 * scale,
              y: pos.y - 500 * scale,
              w: Math.round(2000 * scale),
              h: Math.round(1200 * scale),
              scale,
            };
          }
        } catch {
          schedule(14_000);
          return;
        }
        if (cancelled) return;
        const target = pickTarget(start, winSize.width, winSize.height, mon);
        const dx = target.x - start.x;
        const dy = target.y - start.y;
        const distance = Math.hypot(dx, dy);
        if (distance < 8) {
          schedule(16_000);
          return;
        }
        setFacing(dx < 0 ? 'left' : 'right');
        // 约 200 物理像素/秒，整屏横穿大约 3–8 秒
        const duration = clamp(distance / (200 * Math.max(1, mon.scale)) * 1000, 1600, 8000);
        const t0 = performance.now();
        setRoaming(true);
        const step = async () => {
          if (cancelled) return;
          const progress = clamp((performance.now() - t0) / duration, 0, 1);
          const eased = easeInOut(progress);
          try {
            await invoke('pet_roam_to', {
              x: Math.round(start.x + dx * eased),
              y: Math.round(start.y + dy * eased),
            });
          } catch {
            // 窗口被关闭等场景静默收尾
          }
          if (cancelled) return;
          const interrupted =
            dragActive.current || petStateRef.current !== 'idle'
            || Date.now() - lastUserTouch.current < 8_000
            || gazeRef.current.near > 0.45;
          if (progress < 1 && !interrupted) {
            timer = window.setTimeout(() => {
              void step();
            }, 40);
          } else {
            finish();
          }
        };
        void step();
      })();
    };
    schedule(nextWanderAt({ now: 0 }));
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [roamEnabled, still, petStateRef, dragActive, lastUserTouch, gazeRef]);

  // 随机转身（参考 whale-girl nextFacingAt）：静态陪伴态偶尔换个朝向，
  // 走动期间让位给移动方向，动作间朝向保持连续、不无谓跳回默认。
  useEffect(() => {
    if (still || roaming) return;
    let cancelled = false;
    let timer = 0;
    const tick = () => {
      if (cancelled) return;
      if (!document.hidden && !dragActive.current && petStateRef.current === 'idle') {
        setFacing((prev) => (prev === 'left' ? 'right' : 'left'));
      }
      timer = window.setTimeout(tick, nextFacingAt({ now: 0 }));
    };
    timer = window.setTimeout(tick, nextFacingAt({ now: 0 }));
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [still, roaming, dragActive, petStateRef]);

  return { blinking, greeting, roaming, facing };
}

// 桌面精灵行为：随机眨眼、开场招呼、闲置差分、自主走动、随机转身。
// 这些低频行为共享同一批 refs（petStateRef / dragActive / lastUserTouch / gazeRef），
// 合并在一个 hook 内维护，避免拆散强耦合的状态机。
// v1.1：间隔/姿势池/时长全部来自 manifest.autoActions 的纯节奏器，语义层常量不进设置页。
import { useEffect, useRef, useState } from 'react';
import type { MutableRefObject } from 'react';
import { invoke, currentMonitor, getCurrentWindow } from '../../transport';
import type { TKey } from '../../i18n';
import { clamp, easeInOut, type Bubble } from '../config';
import {
  IDLE_MESSAGE_KEYS,
  IDLE_VARIANT_WEIGHTS,
  type NoticeCategory,
} from '../../notify/messageModel';
import {
  AUTO,
  nextBlinkAt,
  nextFacingAt,
  nextIdleVariantAt,
  autoPoseHold,
  autoWaitMs,
  canBeginAuto,
  leaseFor,
  nextWanderAt,
  pickIdleVariant,
  pickIdleVariantWeighted,
  poseHoldMs,
  type PetPose,
  type PetState,
} from '../petState';

/** 漂移朝向：整段用 lifted（被提起来）序列帧，左右用 CSS 镜像，不新增四向立绘。 */
export type PetFacing = 'left' | 'right';
const PET_IDLE_BUBBLES: TKey[] = [
  'pet.bubble.idle1', 'pet.bubble.idle2', 'pet.bubble.idle3',
];

type Props = {
  blinkEnabled: boolean;
  roamEnabled: boolean;
  still: boolean;
  /** 光标进入立绘靠近区。只驱动 greeting，不和开场 welcome 共用。 */
  cursorNear: boolean;
  petState: PetState;
  petStateRef: MutableRefObject<PetState>;
  dragActive: MutableRefObject<boolean>;
  lastUserTouch: MutableRefObject<number>;
  gazeRef: MutableRefObject<{ x: number; y: number; near: number }>;
  playIdlePose: (pose: PetPose, ms: number) => void;
  clearIdlePose: () => void;
  pushBubble: (text: string, kind: Bubble['kind'], category?: NoticeCategory) => void;
  t: (key: TKey) => string;
};

export function usePetBehaviors({
  blinkEnabled,
  roamEnabled,
  still,
  cursorNear,
  petState,
  petStateRef,
  dragActive,
  lastUserTouch,
  gazeRef,
  playIdlePose,
  clearIdlePose,
  pushBubble,
  t,
}: Props) {
  const [blinking, setBlinking] = useState(false);
  const [greeting, setGreeting] = useState(false);
  const [welcoming, setWelcoming] = useState(false);
  const [roaming, setRoaming] = useState(false);
  // v1.3.3：落到新位置后播一次 landing，闭合「被提起来 → 飞过去 → 松手落地」。
  const [roamLanding, setRoamLanding] = useState(false);
  const [facing, setFacing] = useState<PetFacing>('right');

  const blinkTimers = useRef<number[]>([]);
  const landTimer = useRef(0);
  const greeted = useRef(false);
  const nextGreetingAt = useRef(0);
  // 闲置差分、开场、漂移共用。自动来源不能打断未结束的一段。
  const lease = useRef<{ until: number } | null>(null);
  const roamingRef = useRef(false);
  roamingRef.current = roaming;
  const lastIdleBubble = useRef(0);
  // 最近 2 个闲置姿势：交给加权选择器降权，避免同一个动作连播。
  // 作者：孙文龙 sun
  const idleHistory = useRef<string[]>([]);

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

  // 开场挥手：welcome 槽，和靠近时的点头（greeting）分开。
  useEffect(() => {
    if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;
    let cancelled = false;
    let timer = 0;
    const greet = () => {
      if (cancelled || greeted.current || document.hidden || still) return;
      greeted.current = true;
      setWelcoming(true);
      const hold = autoPoseHold(AUTO.welcome);
      lease.current = leaseFor(Date.now(), hold);
      clearIdlePose();
      pushBubble(t('pet.bubble.hello'), 'info', 'interaction');
      timer = window.setTimeout(() => setWelcoming(false), hold);
    };
    greet();
    document.addEventListener('visibilitychange', greet);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
      document.removeEventListener('visibilitychange', greet);
    };
  }, [clearIdlePose, pushBubble, still, t]);

  // 光标进入靠近区时点一次头。持续靠近仍由 peek 负责，冷却来自 greeting 槽。
  // 定时器不跟依赖清掉：光标离开后点头要播完，否则 flag 会卡住。
  const greetTimer = useRef(0);
  useEffect(() => {
    if (!cursorNear || still || document.hidden || petState !== 'idle' || dragActive.current) return;
    const now = Date.now();
    if (now < nextGreetingAt.current) return;
    const hold = autoPoseHold(AUTO.greeting);
    nextGreetingAt.current = now + hold + AUTO.greeting.cooldownMs;
    lease.current = leaseFor(now, hold);
    clearIdlePose();
    setGreeting(true);
    window.clearTimeout(greetTimer.current);
    greetTimer.current = window.setTimeout(() => setGreeting(false), hold);
  }, [clearIdlePose, cursorNear, still, petState, dragActive]);
  useEffect(() => () => window.clearTimeout(greetTimer.current), []);

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
      const now = Date.now();
      const quietFor = now - lastUserTouch.current;
      if (!canBeginAuto(lease.current, now) || roamingRef.current) {
        later(tick, autoWaitMs(lease.current, now, roamingRef.current ? 400 : 0));
        return;
      }
      if (
        !document.hidden && !still && petStateRef.current === 'idle'
        && !dragActive.current && quietFor > 6_000
      ) {
        // 姿势池与时长都来自 manifest.autoActions；加权挑选 + 最近 2 个姿势历史避免连播重复。
        // 空池兜底仍走旧的 pickIdleVariant()，任何配置下都返回合法姿势。
        // 作者：孙文龙 sun
        const poses = AUTO.idleVariant.poses as readonly string[];
        const pose = poses.length
          ? pickIdleVariantWeighted(poses, IDLE_VARIANT_WEIGHTS, idleHistory.current)
          : (pickIdleVariant() as string);
        // 停留时长按所选姿势的素材时长派生：完整播完一遍再加尾部停留，
        // 不再用统一的 2200ms（比最短动作还短一半，会被硬切）。
        // 作者：孙文龙 sun
        const ms = poseHoldMs(pose as PetPose, {
          cycles: AUTO.idleVariant.holdCycles,
          tailMs: AUTO.idleVariant.tailMs,
        });
        lease.current = leaseFor(now, ms);
        playIdlePose(pose as PetPose, ms);
        idleHistory.current = [pose, ...idleHistory.current].slice(0, 2);
        if (Math.random() < 0.34 && Date.now() - lastIdleBubble.current > 90_000) {
          lastIdleBubble.current = Date.now();
          // 有专属台词就跟着姿势走；没有专属台词的姿势从通用池随机一条，
          // 避免每次都是同一句。通用池留在 overlay 侧，让运行时文案键稳定进包。
          // 作者：孙文龙 sun
          const tailored = IDLE_MESSAGE_KEYS[pose];
          const fallback = PET_IDLE_BUBBLES[Math.floor(Math.random() * PET_IDLE_BUBBLES.length)];
          const key = (tailored || fallback) as TKey;
          pushBubble(t(key), 'info', 'idle');
        }
      }
      const gap = nextIdleVariantAt({ now: 0 });
      later(tick, autoWaitMs(lease.current, Date.now(), gap));
    };
    later(tick, 6_000 + Math.random() * 5_000);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [still, playIdlePose, pushBubble, petStateRef, dragActive, lastUserTouch, t]);

  // 自主漂移：在当前显示器可用区内随机落点（整屏，非小范围漂），
  // 后端 pet_roam_to 再硬夹边。全程 lifted（被提起来飞过去），落点播 landing；
  // 不再用 walk 走路帧，避免脚步与窗口位移永远对不齐。
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
      setRoamLanding(true);
      lease.current = leaseFor(
        Date.now(),
        poseHoldMs('landing', { cycles: AUTO.drag.holdCycles, tailMs: AUTO.drag.tailMs }),
      );
      window.clearTimeout(landTimer.current);
      landTimer.current = window.setTimeout(
        () => setRoamLanding(false),
        poseHoldMs('landing', { cycles: AUTO.drag.holdCycles, tailMs: AUTO.drag.tailMs }),
      );
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
      const now = Date.now();
      if (!canBeginAuto(lease.current, now)) {
        schedule(autoWaitMs(lease.current, now, 400));
        return;
      }
      if (
        petStateRef.current !== 'idle' || dragActive.current
        || now - lastUserTouch.current < 8_000
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
        if (cancelled || !canBeginAuto(lease.current, Date.now())) {
          if (!cancelled) schedule(autoWaitMs(lease.current, Date.now(), 400));
          return;
        }
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
        const landingMs = poseHoldMs('landing', { cycles: AUTO.drag.holdCycles, tailMs: AUTO.drag.tailMs });
        lease.current = leaseFor(Date.now(), duration + landingMs);
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
      window.clearTimeout(landTimer.current);
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

  const preemptAuto = (ms = 0) => {
    lease.current = ms > 0 ? leaseFor(Date.now(), ms) : null;
  };

  return { blinking, greeting, welcoming, roaming, roamLanding, facing, preemptAuto };
}

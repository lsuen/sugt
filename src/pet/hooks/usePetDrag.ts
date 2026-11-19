// 桌面精灵拖动：原生窗口拖动 + 松手贴边吸附 + 惯性甩出。
// 拖动期间通过窗口 onMoved 累计速度样本，快速松手时先跑一段衰减滑动再吸附落位。
import { useCallback, useEffect, useRef, useState } from 'react';
import type { MutableRefObject, RefObject, MouseEvent as ReactMouseEvent } from 'react';
import { invoke, getCurrentWindow } from '../../transport';
import { clamp, easeInOut } from '../config';
import { AUTO, poseHoldMs } from '../petState';

/**
 * v1.3 拖拽三段物理感：按下即「被提起来」(lifted)，松手落地缓冲 (landing)，
 * 随后回基态 (idle)。三段都有专属动作素材，不再复用走路姿。
 * 纯单击（未发生位移）不播落地姿，避免点一下就抖。
 */
export type PetDragPhase = 'idle' | 'lifted' | 'landing';

type Props = {
  interactive: boolean;
  edit: boolean;
  snapOnUp: MutableRefObject<boolean>;
  stageRef: RefObject<HTMLDivElement>;
  onTouch: () => void;
};

const FLING_SPEED_THRESHOLD = 600; // 物理像素/秒，低于此速度直接吸附不甩出
const FLING_DURATION = 300;
const FLING_DISTANCE_CAP = 240; // 物理像素，单轴最大滑行距离

export function usePetDrag({ interactive, edit, snapOnUp, stageRef, onTouch }: Props) {
  const [dragging, setDragging] = useState(false);
  // 三段拖拽：lifted 只在按住期间生效，landing 是松手后的一次性缓冲过渡。
  const [dragPhase, setDragPhase] = useState<PetDragPhase>('idle');
  const phaseTimer = useRef(0);
  const dragActive = useRef(false);
  const movedDuringDrag = useRef(false);
  const dragRequested = useRef(false);
  const dragCleanupTimer = useRef(0);
  const snapTimer = useRef(0);
  const pendingDrag = useRef<{ x: number; y: number } | null>(null);
  const draggedAt = useRef(0);
  const finishing = useRef(false);
  const landTimer = useRef(0);
  const velocity = useRef<{ x: number; y: number; t: number }[]>([]);
  // 飞行中的动画帧与收尾定时器都登记到 ref，卸载时统一取消；alive 用于切断异步链。 —— 孙文龙 sun
  const flingTimer = useRef(0);
  const finishingTimer = useRef(0);
  const alive = useRef(true);

  const clearDragFlags = useCallback(() => {
    dragActive.current = false;
    movedDuringDrag.current = false;
    dragRequested.current = false;
    pendingDrag.current = null;
    velocity.current = [];
    setDragging(false);
    setDragPhase('idle');
  }, []);

  const beginDrag = useCallback(() => {
    dragActive.current = true;
    movedDuringDrag.current = false;
    dragRequested.current = false;
    velocity.current = [];
    window.clearTimeout(dragCleanupTimer.current);
    window.clearTimeout(snapTimer.current);
    window.clearTimeout(phaseTimer.current);
    // 按下即「被提起来」，不必等位移；视觉层用专属 lifted 序列帧。
    setDragging(true);
    setDragPhase('lifted');
    // 原生 start_dragging 在部分平台会吞掉 pointerup，超时兜底避免状态位永久卡住
    dragCleanupTimer.current = window.setTimeout(clearDragFlags, 5000);
  }, [clearDragFlags]);

  const estimateVelocity = useCallback(() => {
    const samples = velocity.current;
    if (samples.length < 2) return { x: 0, y: 0 };
    const last = samples[samples.length - 1];
    const prev = samples[0];
    const dt = last.t - prev.t;
    if (dt <= 0) return { x: 0, y: 0 };
    return {
      x: ((last.x - prev.x) / dt) * 1000,
      y: ((last.y - prev.y) / dt) * 1000,
    };
  }, []);

  const applyFling = useCallback(async (): Promise<void> => {
    const v = estimateVelocity();
    if (Math.hypot(v.x, v.y) < FLING_SPEED_THRESHOLD) return;
    let pos: { x: number; y: number };
    try {
      pos = await getCurrentWindow().outerPosition();
    } catch {
      return;
    }
    const distX = clamp(v.x * 0.14, -FLING_DISTANCE_CAP, FLING_DISTANCE_CAP);
    const distY = clamp(v.y * 0.14, -FLING_DISTANCE_CAP, FLING_DISTANCE_CAP);
    const start = { x: pos.x, y: pos.y };
    const t0 = performance.now();
    await new Promise<void>((resolve) => {
      const step = () => {
        // 卸载或重新按下时立刻收敛，不再向后端发位置（QD-04）。
        if (!alive.current || dragActive.current) {
          resolve();
          return;
        }
        const p = clamp((performance.now() - t0) / FLING_DURATION, 0, 1);
        const eased = easeInOut(p);
        void invoke('pet_roam_to', {
          x: Math.round(start.x + distX * eased),
          y: Math.round(start.y + distY * eased),
        }).catch(() => undefined);
        if (p < 1) {
          window.clearTimeout(flingTimer.current);
          flingTimer.current = window.setTimeout(step, 16);
        } else {
          resolve();
        }
      };
      step();
    });
  }, [estimateVelocity]);

  const finishDrag = useCallback(
    (didMove: boolean) => {
      if (!didMove || finishing.current) return;
      window.clearTimeout(snapTimer.current);
      // 短防抖：吞掉吸附自身引起的二次抖动，但不在拖动途中提前吸附
      snapTimer.current = window.setTimeout(() => {
        finishing.current = true;
        stageRef.current?.style.setProperty('--drag-tilt', '0');
        stageRef.current?.classList.add('pet-landed');
        window.clearTimeout(landTimer.current);
        landTimer.current = window.setTimeout(() => {
          stageRef.current?.classList.remove('pet-landed');
        }, 320);
        void (async () => {
          await applyFling();
          if (!alive.current) return;
          void invoke('pet_finish_drag', { snap: snapOnUp.current })
            .catch(() => undefined)
            .finally(() => {
              window.clearTimeout(finishingTimer.current);
              finishingTimer.current = window.setTimeout(() => {
                finishing.current = false;
              }, 250);
            });
        })();
      }, 80);
    },
    [applyFling, snapOnUp, stageRef],
  );

  // 松手才吸附：pointerup / cancel / blur；保留 movedDuringDrag 一小段以吞迟到 click
  useEffect(() => {
    const onUp = () => {
      if (!dragActive.current) return;
      const didMove = movedDuringDrag.current;
      dragActive.current = false;
      setDragging(false);
      window.clearTimeout(phaseTimer.current);
      if (didMove) {
        // 真的换了位置才播落地缓冲；纯单击不抖。
        // 落地缓冲是 once 动作（下蹲→回弹→站定），播完整一遍再回基态；
        // 原来的 420ms 只播到刚下蹲，看起来像「落地没落地」。
        // 作者：孙文龙 sun
        setDragPhase('landing');
        phaseTimer.current = window.setTimeout(
          () => setDragPhase('idle'),
          poseHoldMs('landing', { cycles: AUTO.drag.holdCycles, tailMs: AUTO.drag.tailMs }),
        );
      } else {
        setDragPhase('idle');
      }
      finishDrag(didMove);
      window.clearTimeout(dragCleanupTimer.current);
      dragCleanupTimer.current = window.setTimeout(() => {
        movedDuringDrag.current = false;
        dragRequested.current = false;
        pendingDrag.current = null;
      }, 900);
    };
    window.addEventListener('pointerup', onUp, true);
    window.addEventListener('pointercancel', onUp, true);
    window.addEventListener('blur', onUp, true);
    return () => {
      window.removeEventListener('pointerup', onUp, true);
      window.removeEventListener('pointercancel', onUp, true);
      window.removeEventListener('blur', onUp, true);
      window.clearTimeout(dragCleanupTimer.current);
      window.clearTimeout(snapTimer.current);
    };
  }, [finishDrag]);

  // 拖动中更新 tilt 与速度样本；吸附改到松手（finishDrag），避免停顿 320ms 就吸附
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    let tiltTimer = 0;
    let last: { x: number; y: number } | null = null;
    void getCurrentWindow()
      .onMoved(({ payload }) => {
        // Promise 尚未 resolve 时组件可能已卸载，此时解绑函数还没拿到，先用守卫挡住回调。
        if (cancelled || !alive.current || !dragActive.current || finishing.current) return;
        movedDuringDrag.current = true;
        draggedAt.current = Date.now();
        setDragging(true);
        const now = Date.now();
        velocity.current.push({ x: payload.x, y: payload.y, t: now });
        if (velocity.current.length > 4) velocity.current.shift();
        if (last && stageRef.current) {
          const tilt = clamp((payload.x - last.x) / 14, -1, 1);
          stageRef.current.style.setProperty('--drag-tilt', tilt.toFixed(3));
          window.clearTimeout(tiltTimer);
          tiltTimer = window.setTimeout(() => {
            stageRef.current?.style.setProperty('--drag-tilt', '0');
          }, 340);
        }
        last = { x: payload.x, y: payload.y };
      })
      .then((fn) => {
        // 若卸载早于 resolve，立即解绑，避免留下一个无人能取消的窗口移动监听器（QD-04）。
        if (cancelled) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      window.clearTimeout(tiltTimer);
      unlisten?.();
    };
  }, [stageRef]);

  // 组件卸载统一收口一次性定时器与飞行异步链
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      window.clearTimeout(dragCleanupTimer.current);
      window.clearTimeout(snapTimer.current);
      window.clearTimeout(landTimer.current);
      window.clearTimeout(phaseTimer.current);
      window.clearTimeout(flingTimer.current);
      window.clearTimeout(finishingTimer.current);
    };
  }, []);

  // 4px 阈值：纯单击不进入原生拖动循环（部分平台会吞掉 click），移动后才真正开始拖。
  const onFigureMouseDown = useCallback(
    (e: ReactMouseEvent<HTMLDivElement>) => {
      if (!interactive || edit || e.button !== 0) return;
      e.preventDefault();
      onTouch();
      pendingDrag.current = { x: e.clientX, y: e.clientY };
      beginDrag();
    },
    [interactive, edit, onTouch, beginDrag],
  );

  const onFigureMouseMove = useCallback((e: ReactMouseEvent<HTMLDivElement>) => {
    const start = pendingDrag.current;
    if (!start) return;
    if (Math.abs(e.clientX - start.x) + Math.abs(e.clientY - start.y) < 4) return;
    pendingDrag.current = null;
    dragRequested.current = true;
    void invoke('overlay_start_drag').catch(() => undefined);
  }, []);

  return {
    dragging,
    dragPhase,
    dragActive,
    movedDuringDrag,
    dragRequested,
    draggedAt,
    onFigureMouseDown,
    onFigureMouseMove,
  };
}

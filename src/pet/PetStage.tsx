// 桌面精灵（小酥）舞台组件：网关五态 + 视线/眨眼/交互/自主走动 + 右键菜单。
// 纯状态机在 ./petState.ts，光标/拖动/行为分别在 ./hooks，配置类型在 ./config。
import { useCallback, useEffect, useRef, useState } from 'react';
import type { MouseEvent as ReactMouseEvent } from 'react';
import { invoke, listen } from '../transport';
import { t, type TKey } from '../i18n';
import {
  canEmitStateNotice,
  DEDUPE_WINDOW_MS,
  interactionMessageKeyFor,
  type NoticeCategory,
} from '../notify/messageModel';
import {
  AUTO,
  petBubbleMs,
  petSleepAfterMs,
  poseHoldMs,
  reducePetState,
  resolvePetPose,
  resultHoldMs,
  shouldWake,
  type PetPose,
  type PetState,
} from './petState';
import { PET_MOTION, PET_POSE_NAMES } from './petManifest.generated';
import {
  type OverlayConfigView,
  type OverlayStatus,
} from './config';
import { usePetGaze } from './hooks/usePetGaze';
import { usePetDrag } from './hooks/usePetDrag';
import { usePetBehaviors } from './hooks/usePetBehaviors';
import { PetInfoModal, type PetInfoSnapshot } from './PetInfoModal';
import './pet.css';

const PET_LABEL: Record<PetState, TKey> = {
  offline: 'pet.offline',
  idle: 'pet.idle',
  typing: 'pet.typing',
  done: 'pet.done',
  error: 'pet.error',
  sleep: 'pet.sleep',
};

/**
 * 单击要等这一小段时间才落地：浏览器先派发 detail=1 的 click，随后才派发 detail=2。
 * 不等就无法区分「双击的首击」和「普通单击」，首击会多播摸头/轻碰、推气泡并抢主窗口焦点（QD-03）。
 * 双击阈值由系统决定，这里取一个远小于常见双击间隔的值，只用于兜住首击。
 */
const CLICK_SETTLE_MS = 220;

/**
 * 自动动作的视觉停留：manifest 只声明「播几遍 + 尾部停留」，
 * 毫秒由素材逐帧时长派生，换素材后自动跟随，不会播到一半就被切回。
 * 作者：孙文龙 sun
 */
function autoHold(pose: PetPose, cfg: { holdCycles: number; tailMs: number }): number {
  return poseHoldMs(pose, { cycles: cfg.holdCycles, tailMs: cfg.tailMs });
}

/**
 * 舞台气泡：在 config.ts 的旧结构上加分类与合并计数，分类沿用统一通知模型。
 * 作者：孙文龙 sun
 */
type Bubble = {
  id: number;
  text: string;
  kind: 'ok' | 'error' | 'info';
  category: NoticeCategory;
  count: number;
  at: number;
};

/** 只读弹窗的动作说明：按最近一条消息的分类选对应文案。作者：孙文龙 sun */
const ACTION_HINT_KEY: Record<NoticeCategory, TKey> = {
  state: 'pet.info.actionHint.state',
  result: 'pet.info.actionHint.result',
  interaction: 'pet.info.actionHint.interaction',
  idle: 'pet.info.actionHint.idle',
};

// 姿势清单唯一真相源：src/assets/pet/manifest.json（经生成器产出 TS）。
// offline 视觉复用 idle；reduced-motion / 窗口隐藏时切静态海报。
const PET_LOOPS = PET_POSE_NAMES;

type PetStageProps = {
  cfg: OverlayConfigView;
  status: OverlayStatus | null;
  busy: boolean;
  onSave: () => void;
  onDrag: (e: ReactMouseEvent<HTMLSpanElement>) => void;
};

export function PetStage({ cfg, status, busy, onSave, onDrag }: PetStageProps) {
  const [petState, setPetState] = useState<PetState>('offline');
  const [bubble, setBubble] = useState<Bubble | null>(null);
  const [shyPose, setShyPose] = useState(false);
  const [celebrating, setCelebrating] = useState(false);
  // v1.1 自动交互：睡醒过渡 / 失败失落尾段
  const [waking, setWaking] = useState(false);
  const [disappointed, setDisappointed] = useState(false);
  const [poked, setPoked] = useState(false);
  const [still, setStill] = useState(false);
  const [hopKey, setHopKey] = useState(0);
  const [petted, setPetted] = useState(false);
  const [idlePose, setIdlePose] = useState<PetPose | null>(null);
  // 只读信息弹窗：双击立绘或独立菜单发来的事件打开，不承载任何写操作 / 对话。
  const [infoOpen, setInfoOpen] = useState(false);

  const stageRef = useRef<HTMLDivElement>(null);
  const figureRef = useRef<HTMLDivElement>(null);
  const petStateRef = useRef<PetState>('offline');
  const snapOnUp = useRef(cfg.pet_snap !== false);
  const quietRef = useRef(Boolean(cfg.pet_quiet));
  // 菜单在另一扇窗里。这里保持 false，视线钩子不会因为菜单把透明区解开。
  const menuOpenRef = useRef(false);

  const baseline = useRef<{
    running: boolean;
    success: number;
    failed: number;
    failover: boolean;
  } | null>(null);
  const hold = useRef<{ state: PetState; until: number }>({ state: 'idle', until: 0 });
  const typingUntil = useRef(0);
  const lastActivity = useRef(Date.now());
  const lastFailNotice = useRef(0);
  const doneBatch = useRef({ count: 0, timer: 0 });
  const bubbleSeq = useRef(0);
  const activeNow = useRef(0);
  const wasActive = useRef(false);
  const prevVisual = useRef<PetState>('offline');
  // 气泡合并、最近消息与长请求起点都由 ref 维护，避免 render 期副作用。
  // 作者：孙文龙 sun
  const bubbleRef = useRef<Bubble | null>(null);
  const lastNotice = useRef<{ text: string; category: NoticeCategory } | null>(null);
  const requestSince = useRef(0);
  const lastStateEmit = useRef(0);

  const lastUserTouch = useRef(Date.now());
  const lastWake = useRef(0);
  const pokeTimer = useRef(0);
  const lastPokeBubble = useRef(0);
  const pettedTimer = useRef(0);
  const pokeCount = useRef(0);
  const pokeResetTimer = useRef(0);
  const idlePoseTimer = useRef(0);
  const wakeTimer = useRef(0);
  // 待落地的单击：多个并存，保证慢速三连击不会被后来的一次点击取消；双击到达时整体清空。
  const pendingClicks = useRef<number[]>([]);

  petStateRef.current = petState;

  // 自动表演共用一条租约，播完才许下一段。这里只负责把已经获准的姿势播完。
  const playIdlePose = useCallback((pose: PetPose, ms: number) => {
    window.clearTimeout(idlePoseTimer.current);
    setIdlePose(pose);
    idlePoseTimer.current = window.setTimeout(() => setIdlePose(null), ms);
  }, []);
  const clearIdlePose = useCallback(() => {
    window.clearTimeout(idlePoseTimer.current);
    setIdlePose(null);
  }, []);
  const preemptRef = useRef<(ms?: number) => void>(() => {});

  // 视线/闲置/走动/轻碰都会推气泡，免打扰在这里统一收口。
  // 第三参 category 可选，旧的两参调用完全兼容；同文案短窗口内合并计数而非新增一条。
  // 作者：孙文龙 sun
  const pushBubble = useCallback((
    text: string,
    kind: Bubble['kind'],
    category: NoticeCategory = 'interaction',
  ) => {
    if (quietRef.current) return;
    const now = Date.now();
    lastNotice.current = { text, category };
    const prev = bubbleRef.current;
    if (prev && prev.text === text && now - prev.at < DEDUPE_WINDOW_MS) {
      const merged: Bubble = { ...prev, count: prev.count + 1, at: now };
      bubbleRef.current = merged;
      setBubble(merged);
      return;
    }
    bubbleSeq.current += 1;
    const next: Bubble = {
      id: bubbleSeq.current, text, kind, category, count: 1, at: now,
    };
    bubbleRef.current = next;
    setBubble(next);
  }, []);

  // 配置快照：只在 cfg 变化时刷新，避免 render 期副作用
  useEffect(() => {
    snapOnUp.current = cfg.pet_snap !== false;
    quietRef.current = Boolean(cfg.pet_quiet);
  }, [cfg.pet_snap, cfg.pet_quiet]);

  useEffect(() => {
    const un = listen('sugt://pet-show-info', () => setInfoOpen(true));
    return () => {
      un.then((fn) => fn()).catch(() => undefined);
    };
  }, []);

  const dismissMenu = useCallback(() => undefined, []);

  const onNearTouch = useCallback(() => {
    lastUserTouch.current = Date.now();
  }, []);

  const onNear = useCallback(() => {
    lastUserTouch.current = Date.now();
    if (petStateRef.current === 'sleep' && Date.now() - lastWake.current > 20_000) {
      lastWake.current = Date.now();
      lastActivity.current = Date.now();
      const ms = poseHoldMs('peek');
      preemptRef.current(ms);
      playIdlePose('peek', ms);
      pushBubble(t('pet.bubble.wake'), 'info', 'interaction');
    }
  }, [playIdlePose, pushBubble]);

  const interactive = cfg.pet_interactive || cfg.pet_always_interactive || cfg.edit;
  const gazeEnabled = cfg.pet_gaze !== false;
  const blinkEnabled = cfg.pet_blink !== false;
  const roamEnabled = cfg.pet_roam !== false;

  const {
    dragging,
    dragPhase,
    dragActive,
    movedDuringDrag,
    dragRequested,
    draggedAt,
    onFigureMouseDown,
    onFigureMouseMove,
  } = usePetDrag({
    interactive,
    edit: cfg.edit,
    snapOnUp,
    stageRef,
    onTouch: onNearTouch,
  });

  const { gazeRef, nearListening, nearWalking } = usePetGaze({
    gazeEnabled,
    interactive,
    edit: cfg.edit,
    figureRef,
    stageRef,
    dragActive,
    menuOpen: menuOpenRef,
    onNearTouch,
    onNear,
    onMenuDismiss: dismissMenu,
  });

  const { blinking, greeting, welcoming, roaming, roamLanding, facing, preemptAuto } = usePetBehaviors({
    blinkEnabled,
    roamEnabled,
    still,
    cursorNear: nearListening,
    petState,
    petStateRef,
    dragActive,
    lastUserTouch,
    gazeRef,
    playIdlePose,
    clearIdlePose,
    pushBubble,
    t,
  });
  preemptRef.current = preemptAuto;

  // 窗口隐藏 / 系统要求减少动效时切静态海报，动画 WebP 不再解码播放。
  useEffect(() => {
    const media = window.matchMedia('(prefers-reduced-motion: reduce)');
    const update = () => setStill(media.matches || document.hidden);
    update();
    media.addEventListener('change', update);
    document.addEventListener('visibilitychange', update);
    return () => {
      media.removeEventListener('change', update);
      document.removeEventListener('visibilitychange', update);
    };
  }, []);

  // 气泡时长跟配置走（默认 4s）
  useEffect(() => {
    if (!bubble) return;
    const timer = window.setTimeout(
      () => {
        setBubble(null);
        bubbleRef.current = null;
      },
      petBubbleMs(cfg.pet_bubble_ms),
    );
    return () => window.clearTimeout(timer);
  }, [bubble, cfg.pet_bubble_ms]);

  // done/error 进入时强制 remount，让 hop/shiver 可重播
  useEffect(() => {
    if (
      (petState === 'done' || petState === 'error')
      && prevVisual.current !== petState
    ) {
      setHopKey((k) => k + 1);
    }
    prevVisual.current = petState;
    if (petState !== 'idle') {
      stageRef.current?.classList.remove('pet-listening');
    }
  }, [petState]);

  // 状态跃迁检测：轮询间隔 2s，所有判定都基于后端真实统计，不做任何猜测
  useEffect(() => {
    if (!status) return;
    const traffic = status.traffic;
    const running = Boolean(status.running);
    const active = traffic.active_requests ?? 0;
    const success = traffic.success_count ?? 0;
    const failed = traffic.failed_count ?? 0;
    const failover = Boolean(status.last_proxy_failover);
    const now = Date.now();
    const sleepAfterMs = petSleepAfterMs(cfg.pet_sleep_after_secs);
    activeNow.current = active;

    if (!baseline.current) {
      baseline.current = { running, success, failed, failover };
      wasActive.current = active > 0;
      setPetState(running ? 'idle' : 'offline');
      return;
    }
    const prev = baseline.current;

    // 长请求状态消息：只由真实在途统计产生，短请求 / 冷却期 / 在途归零一律不发，杜绝虚报。
    // 作者：孙文龙 sun
    if (activeNow.current > 0) {
      if (requestSince.current === 0) requestSince.current = now;
      const since = now - requestSince.current;
      if (canEmitStateNotice({
        activeRequests: activeNow.current,
        requestSinceMs: since,
        nowMs: now,
        lastEmitAt: lastStateEmit.current,
      })) {
        lastStateEmit.current = now;
        pushBubble(
          t('notice.longRequest', { n: Math.floor(since / 1000) }),
          'info',
          'state',
        );
      }
    } else {
      requestSince.current = 0;
      lastStateEmit.current = 0;
    }

    if (running !== prev.running) {
      pushBubble(
        running ? t('pet.bubble.gatewayUp') : t('pet.bubble.gatewayDown'),
        running ? 'ok' : 'info',
        'state',
      );
      hold.current = { state: 'idle', until: 0 };
      typingUntil.current = 0;
      lastActivity.current = now;
    }
    if (running && failover && !prev.failover) {
      pushBubble(t('pet.bubble.failover'), 'info', 'state');
      lastActivity.current = now;
    }
    if (failed > prev.failed) {
      if (now - lastFailNotice.current > 5 * 60 * 1000) {
        pushBubble(t('pet.bubble.failed'), 'error', 'result');
        lastFailNotice.current = now;
      }
      hold.current = { state: 'error', until: now + resultHoldMs('error') };
      lastActivity.current = now;
    }
    if (success > prev.success) {
      doneBatch.current.count += success - prev.success;
      if (doneBatch.current.timer) window.clearTimeout(doneBatch.current.timer);
      const flushDone = () => {
        if (activeNow.current > 0) {
          doneBatch.current.timer = window.setTimeout(flushDone, 2000);
          return;
        }
        const count = doneBatch.current.count;
        doneBatch.current.count = 0;
        doneBatch.current.timer = 0;
        if (count > 0) pushBubble(t('pet.bubble.done', { n: count }), 'ok', 'result');
      };
      doneBatch.current.timer = window.setTimeout(flushDone, 8000);
    }
    if (active > 0) typingUntil.current = now + 1500;
    if (active > 0 || success > prev.success) lastActivity.current = now;

    const errorHeld = hold.current.state === 'error' && hold.current.until > now;
    if (wasActive.current && active === 0 && !errorHeld) {
      hold.current = { state: 'done', until: now + resultHoldMs('done') };
    }
    wasActive.current = active > 0;

    const reduced = reducePetState({
      gateway: {
        running,
        active,
        hold: hold.current,
        typingUntil: typingUntil.current,
        lastActivity: lastActivity.current,
        sleepAfterMs,
        now,
      },
      now,
    });
    // 睡醒边沿：从 sleep 离开时播一次 wake 过渡（拖拽打断不播）。
    if (shouldWake(prevVisual.current, reduced.state, { dragging: dragActive.current })) {
      window.clearTimeout(wakeTimer.current);
      setWaking(true);
      wakeTimer.current = window.setTimeout(() => setWaking(false), autoHold('wake', AUTO.wake));
    }
    prevVisual.current = reduced.state;
    // error hold 尾段切「失落」姿：短暂、不惩罚、不扣资历（零负反馈）。
    setDisappointed(
      hold.current.state === 'error'
      && hold.current.until > now
      && hold.current.until - now <= autoHold('disappointed', AUTO.disappointed),
    );
    setPetState(reduced.state);

    baseline.current = { running, success, failed, failover };
  }, [status, pushBubble, cfg.pet_sleep_after_secs]);

  useEffect(
    () => () => {
      if (doneBatch.current.timer) window.clearTimeout(doneBatch.current.timer);
    },
    [],
  );

  // 请求完成：整个结果窗口都播庆祝姿（whale-girl 的 turn-completed burst 对齐），
  // 不再只闪 1.4s 就退回 done 立绘。
  useEffect(() => {
    if (petState !== 'done') {
      setCelebrating(false);
      return;
    }
    setCelebrating(true);
    const timer = window.setTimeout(() => setCelebrating(false), autoHold('celebrate', AUTO.celebrate));
    return () => window.clearTimeout(timer);
  }, [petState, hopKey]);

  // 一次性定时器统一收口，组件卸载后不留下任何飞行回调
  useEffect(
    () => () => {
      window.clearTimeout(pokeTimer.current);
      window.clearTimeout(idlePoseTimer.current);
      window.clearTimeout(pettedTimer.current);
      window.clearTimeout(pokeResetTimer.current);
      window.clearTimeout(wakeTimer.current);
      pendingClicks.current.forEach((timer) => window.clearTimeout(timer));
      pendingClicks.current = [];
    },
    [],
  );

  const openSettings = useCallback(() => {
    void invoke('open_settings_tab', { tab: 'pet' }).catch(() => undefined);
    void invoke('focus_main_window').catch(() => undefined);
  }, []);

  const onFigurePointerDown = (e: ReactMouseEvent<HTMLDivElement>) => {
    preemptRef.current();
    clearIdlePose();
    onFigureMouseDown(e);
  };

  const cancelPendingClicks = () => {
    pendingClicks.current.forEach((timer) => window.clearTimeout(timer));
    pendingClicks.current = [];
  };

  const onFigureClick = (e: ReactMouseEvent<HTMLDivElement>) => {
    lastUserTouch.current = Date.now();
    if (!interactive || cfg.edit) return;
    // 本次按下发生过移动（或请求过原生拖动）就只算拖动，不抢主窗口焦点
    if (movedDuringDrag.current || dragRequested.current || Date.now() - draggedAt.current < 350) {
      movedDuringDrag.current = false;
      dragRequested.current = false;
      return;
    }
    // 双击（浏览器 detail>=2）打开只读信息弹窗，不参与轻碰 / 三连计数，也不推 poke 气泡。
    // 三连庆祝是 1.2s 内 3 次 detail=1 的单击，与双击天然区分、互不冲突。
    // 作者：孙文龙 sun
    if (e.detail >= 2) {
      cancelPendingClicks();
      window.clearTimeout(pokeResetTimer.current);
      pokeCount.current = 0;
      setInfoOpen(true);
      return;
    }
    // 单击先只登记、不落地：浏览器把双击拆成 detail=1 + detail=2 两次 click，
    // 若首击立即执行，就会多播一个动作姿、可能推气泡并抢走主窗口焦点（QD-03）。
    // 多个待落地单击并存，慢速三连击不会互相取消；卸载时统一清理。 —— 孙文龙 sun
    const clientY = e.clientY;
    const timer = window.setTimeout(() => {
      pendingClicks.current = pendingClicks.current.filter((item) => item !== timer);
      // 点头部 = 摸头（害羞 + 爱心），点身体 = 轻碰招手；1.2s 内连点三下 = 庆祝
      const rect = figureRef.current?.getBoundingClientRect();
      const head = rect ? clientY - rect.top < rect.height * 0.38 : false;
      pokeCount.current += 1;
      window.clearTimeout(pokeResetTimer.current);
      pokeResetTimer.current = window.setTimeout(() => {
        pokeCount.current = 0;
      }, 1200);
      const triple = pokeCount.current >= 3;
      if (triple) {
        pokeCount.current = 0;
        const ms = autoHold('celebrate', AUTO.celebrate);
        preemptRef.current(ms);
        playIdlePose('celebrate', ms);
      } else {
        setShyPose(head);
        setPoked(!head);
        window.clearTimeout(pokeTimer.current);
        // petted / poked 是无缝循环的「状态姿」（首尾同帧、幅度小），停留时长是交互节奏，
        // 不属于「叙事动作播到一半」——所以这里保持短停留，允许下一次点击直接打断。
        // 作者：孙文龙 sun
        pokeTimer.current = window.setTimeout(() => {
          setPoked(false);
          setShyPose(false);
        }, 1600);
      }
      if (head) {
        setPetted(true);
        window.clearTimeout(pettedTimer.current);
        pettedTimer.current = window.setTimeout(() => setPetted(false), 720);
      }
      // 交互台词细分：摸头 / 身体轻碰 / 三连点各有专属文案，统一归 interaction。
      // 作者：孙文龙 sun
      const interactionKind = triple ? 'tripleTap' : head ? 'headPat' : 'poke';
      if (Date.now() - lastPokeBubble.current > 8000) {
        lastPokeBubble.current = Date.now();
        pushBubble(
          t(interactionMessageKeyFor(interactionKind) as TKey),
          'info',
          'interaction',
        );
      }
      void invoke('focus_main_window').catch(() => undefined);
    }, CLICK_SETTLE_MS);
    pendingClicks.current.push(timer);
  };

  const onFigureContextMenu = (e: ReactMouseEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();
    if (!interactive || cfg.edit) return;
    lastUserTouch.current = Date.now();
    preemptRef.current();
    void invoke('pet_open_menu').catch(() => undefined);
  };

  const activeFrame = resolvePetPose({
    state: petState,
    poked,
    shy: shyPose,
    listening: nearListening,
    walking: nearWalking,
    celebrating,
    greeting,
    welcoming,
    idlePose,
    roaming,
    // 拖拽三段各有专属序列帧：按下被提起、松手落地、随后回基态。
    // v1.3.3 自主漂移同样走被提起 / 落地两段，闭合「飞过去 → 松手落地」。
    dragging: dragPhase === 'lifted',
    landing: dragPhase === 'landing' || (roamLanding && petState === 'idle'),
    waking,
    disappointed,
    // 网关当前没有「等待批准」事件，P1 事件化后从这里接入 wait 动作。
    waiting: false,
  });
  const activeMotion = (PET_MOTION as Record<string, string>)[activeFrame] ?? 'breathe';
  const classNames = [
    'pet-stage',
    interactive ? 'interactive' : '',
    blinking ? 'blinking' : '',
    still ? 'still' : '',
    gazeEnabled ? '' : 'no-gaze',
    blinkEnabled ? '' : 'no-blink',
    poked ? 'poked' : '',
    petted ? 'petted' : '',
    roaming ? 'roaming' : '',
    roamLanding ? 'pet-landed' : '',
    dragging ? 'dragging' : '',
  ].filter(Boolean).join(' ');

  const petOpacity = Math.min(1, Math.max(0.15, Number(cfg.opacity) || 1));

  // 只读快照：全部取自真实统计与当前渲染态，弹窗内不做任何写操作。
  // 作者：孙文龙 sun
  const infoSnapshot: PetInfoSnapshot | null = status ? {
    stateLabel: t(PET_LABEL[petState]),
    active: activeNow.current,
    success: status.traffic.success_count ?? 0,
    failed: status.traffic.failed_count ?? 0,
    pose: activeFrame,
    actionHint: t(ACTION_HINT_KEY[lastNotice.current?.category ?? 'state']),
    recent: lastNotice.current?.text ?? null,
  } : null;

  return (
    <div
      ref={stageRef}
      className={classNames}
      data-state={petState}
      data-facing={facing}
      data-motion={activeMotion}
      data-drag-phase={dragPhase}
      style={{
        ['--pet-opacity' as string]: String(petOpacity),
        // 活感层的周期与幅度全部来自 manifest.autoActions.liveliness，
        // 改节奏只改 manifest，不散落硬编码在 CSS/TS 里。
        ['--pet-breath-ms' as string]: `${AUTO.liveliness.cycles.breathMs}ms`,
        ['--pet-secondary-ms' as string]: `${AUTO.liveliness.cycles.secondaryMs}ms`,
        ['--pet-weight-ms' as string]: `${AUTO.liveliness.cycles.weightMs}ms`,
        ['--pet-secondary-deg' as string]: String(AUTO.liveliness.secondary.maxDeg),
        ['--pet-secondary-shift' as string]: String(AUTO.liveliness.secondary.maxShiftPx),
      }}
    >
      {/* 环境层：禁止底板；只留粒子与完成火花（零光晕 / 零地面阴影） */}
      <div className="pet-ambient" aria-hidden="true" />

      {/* 信息层：气泡在左上角自适应排版，状态胶囊移到角色脚底 */}
      <div className="pet-side">
        {bubble && (
          <div
            key={bubble.id}
            className={`pet-bubble pet-bubble-${bubble.kind}`}
            data-category={bubble.category}
            role="status"
          >
            <span className="pet-bubble-cat">
              {t(`notice.category.${bubble.category}` as TKey)}
            </span>
            {bubble.text}
            {bubble.count > 1 && (
              <span className="pet-bubble-count">{t('notice.count', { n: bubble.count })}</span>
            )}
          </div>
        )}
        <div className="pet-status-bar" role="status">
          <span className="pet-status-dot" aria-hidden="true" />
          <span className="pet-status-text">{t(PET_LABEL[petState])}</span>
        </div>
      </div>

      {/* 思考云：请求级只给装饰，不吐气泡文案 */}
      {petState === 'typing' && (
        <div className="pet-thought" aria-hidden="true"><i /><i /><i /></div>
      )}

      {/* 角色分层：朝向/头跟随 → 重心(慢) → 呼吸(快) → 发梢二次动作(中) → 逐帧动画。
          四层 transform 互不覆盖，且每层只挂一张素材，避免 alpha 叠出双重影像。
          眼睑跟呼吸层，眼神高光跟头层，形成「眼→头→身」分级延迟。 */}
      <div
        ref={figureRef}
        className={`pet-figure${dragging ? ' dragging' : ''}${poked ? ' poked' : ''}`}
        onMouseDown={onFigurePointerDown}
        onMouseMove={onFigureMouseMove}
        onContextMenu={onFigureContextMenu}
        onClick={onFigureClick}
        role={interactive ? 'button' : undefined}
        aria-label={interactive ? t(PET_LABEL[petState]) : undefined}
      >
        <div className="pet-weight">
          <div className="pet-motion">
            <div className="pet-secondary">
              <div key={hopKey} className="pet-loops">
                {PET_LOOPS.map((loop) => (
                  <span
                    key={loop}
                    className={`pet-loop${activeFrame === loop ? ' is-active' : ''}`}
                    data-loop={loop}
                    aria-hidden="true"
                  />
                ))}
                <span
                  className={`pet-poster${still ? ' is-active' : ''}`}
                  data-frame={activeFrame}
                  aria-hidden="true"
                />
              </div>
              <span className="pet-gaze" aria-hidden="true" />
            </div>
            {blinkEnabled && <span className="pet-blink" aria-hidden="true" />}
          </div>
        </div>
      </div>

      {infoOpen && (
        <PetInfoModal
          snapshot={infoSnapshot}
          onClose={() => setInfoOpen(false)}
          onOpenSettings={openSettings}
        />
      )}

      {cfg.edit && (
        <div className="pet-edit-bar">
          <button type="button" className="overlay-save" onClick={onSave} disabled={busy}>
            {busy ? '…' : t('overlay.save')}
          </button>
          <span
            className="overlay-grip"
            onMouseDown={(e) => {
              onDrag(e);
            }}
            title={t('overlay.drag')}
            aria-label={t('overlay.drag')}
          />
        </div>
      )}
    </div>
  );
}

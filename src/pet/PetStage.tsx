// 桌面精灵（小酥）舞台组件：网关五态 + 视线/眨眼/交互/自主走动 + 右键菜单。
// 纯状态机在 ./petState.ts，光标/拖动/行为分别在 ./hooks，配置类型在 ./config。
import { useCallback, useEffect, useRef, useState } from 'react';
import type { MouseEvent as ReactMouseEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { t, type TKey } from '../i18n';
import {
  derivePetState,
  petBubbleMs,
  petSleepAfterMs,
  resolvePetPose,
  type PetPose,
  type PetState,
} from './petState';
import {
  type Bubble,
  type OverlayConfigView,
  type OverlayStatus,
} from './config';
import { usePetGaze } from './hooks/usePetGaze';
import { usePetDrag } from './hooks/usePetDrag';
import { usePetBehaviors } from './hooks/usePetBehaviors';
import { PetContextMenu } from './PetContextMenu';
import './pet.css';

const PET_LABEL: Record<PetState, TKey> = {
  offline: 'pet.offline',
  idle: 'pet.idle',
  typing: 'pet.typing',
  done: 'pet.done',
  error: 'pet.error',
  sleep: 'pet.sleep',
};

// v0.9：网关五态 + 交互/表情差分；offline 视觉复用 idle。
// 动画 WebP 交叉淡入；reduced-motion / 窗口隐藏时切静态海报。
const PET_LOOPS = [
  'idle', 'typing', 'done', 'error', 'sleep',
  'wave', 'peek', 'shy', 'celebrate', 'walk',
] as const;

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
  const [poked, setPoked] = useState(false);
  const [still, setStill] = useState(false);
  const [hopKey, setHopKey] = useState(0);
  const [petted, setPetted] = useState(false);
  const [idlePose, setIdlePose] = useState<PetPose | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);

  const stageRef = useRef<HTMLDivElement>(null);
  const figureRef = useRef<HTMLDivElement>(null);
  const petStateRef = useRef<PetState>('offline');
  const snapOnUp = useRef(cfg.pet_snap !== false);
  const quietRef = useRef(Boolean(cfg.pet_quiet));
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

  const lastUserTouch = useRef(Date.now());
  const lastWake = useRef(0);
  const pokeTimer = useRef(0);
  const lastPokeBubble = useRef(0);
  const pettedTimer = useRef(0);
  const pokeCount = useRef(0);
  const pokeResetTimer = useRef(0);
  const idlePoseTimer = useRef(0);

  petStateRef.current = petState;

  // 所有差分姿共用一个定时器：新动作会顶掉旧动作，不会叠出多重人影
  const playIdlePose = useCallback((pose: PetPose, ms: number) => {
    window.clearTimeout(idlePoseTimer.current);
    setIdlePose(pose);
    idlePoseTimer.current = window.setTimeout(() => setIdlePose(null), ms);
  }, []);

  // 视线/闲置/走动/轻碰都会推气泡，免打扰在这里统一收口
  const pushBubble = useCallback((text: string, kind: Bubble['kind']) => {
    if (quietRef.current) return;
    bubbleSeq.current += 1;
    setBubble({ id: bubbleSeq.current, text, kind });
  }, []);

  // 配置快照：只在 cfg 变化时刷新，避免 render 期副作用
  useEffect(() => {
    snapOnUp.current = cfg.pet_snap !== false;
    quietRef.current = Boolean(cfg.pet_quiet);
  }, [cfg.pet_snap, cfg.pet_quiet]);

  useEffect(() => {
    menuOpenRef.current = menuOpen;
  }, [menuOpen]);

  // Esc 关菜单；光标离开立绘+菜单由 usePetGaze 回调关闭（不整窗解穿透）
  useEffect(() => {
    if (!menuOpen) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setMenuOpen(false);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [menuOpen]);

  const dismissMenu = useCallback(() => setMenuOpen(false), []);

  const onNearTouch = useCallback(() => {
    lastUserTouch.current = Date.now();
  }, []);

  const onNear = useCallback(() => {
    lastUserTouch.current = Date.now();
    if (petStateRef.current === 'sleep' && Date.now() - lastWake.current > 20_000) {
      lastWake.current = Date.now();
      lastActivity.current = Date.now();
      playIdlePose('peek', 1900);
      pushBubble(t('pet.bubble.wake'), 'info');
    }
  }, [playIdlePose, pushBubble]);

  const interactive = cfg.pet_interactive || cfg.pet_always_interactive || cfg.edit;
  const gazeEnabled = cfg.pet_gaze !== false;
  const blinkEnabled = cfg.pet_blink !== false;
  const roamEnabled = cfg.pet_roam !== false;

  const {
    dragging,
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

  const { blinking, greeting, roaming, facing } = usePetBehaviors({
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
  });

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
      () => setBubble(null),
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

    if (running !== prev.running) {
      pushBubble(
        running ? t('pet.bubble.gatewayUp') : t('pet.bubble.gatewayDown'),
        running ? 'ok' : 'info',
      );
      hold.current = { state: 'idle', until: 0 };
      typingUntil.current = 0;
      lastActivity.current = now;
    }
    if (running && failover && !prev.failover) {
      pushBubble(t('pet.bubble.failover'), 'info');
      lastActivity.current = now;
    }
    if (failed > prev.failed) {
      if (now - lastFailNotice.current > 5 * 60 * 1000) {
        pushBubble(t('pet.bubble.failed'), 'error');
        lastFailNotice.current = now;
      }
      hold.current = { state: 'error', until: now + 6000 };
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
        if (count > 0) pushBubble(t('pet.bubble.done', { n: count }), 'ok');
      };
      doneBatch.current.timer = window.setTimeout(flushDone, 8000);
    }
    if (active > 0) typingUntil.current = now + 1500;
    if (active > 0 || success > prev.success) lastActivity.current = now;

    const errorHeld = hold.current.state === 'error' && hold.current.until > now;
    if (wasActive.current && active === 0 && !errorHeld) {
      hold.current = { state: 'done', until: now + 3500 };
    }
    wasActive.current = active > 0;

    setPetState(derivePetState({
      running,
      active,
      hold: hold.current,
      typingUntil: typingUntil.current,
      lastActivity: lastActivity.current,
      sleepAfterMs,
      now,
    }));

    baseline.current = { running, success, failed, failover };
  }, [status, pushBubble, cfg.pet_sleep_after_secs]);

  useEffect(
    () => () => {
      if (doneBatch.current.timer) window.clearTimeout(doneBatch.current.timer);
    },
    [],
  );

  // 完成态闪一下庆祝姿，再回到 done 立绘
  useEffect(() => {
    if (petState !== 'done') {
      setCelebrating(false);
      return;
    }
    setCelebrating(true);
    const timer = window.setTimeout(() => setCelebrating(false), 1400);
    return () => window.clearTimeout(timer);
  }, [petState, hopKey]);

  // 一次性定时器统一收口，组件卸载后不留下任何飞行回调
  useEffect(
    () => () => {
      window.clearTimeout(pokeTimer.current);
      window.clearTimeout(idlePoseTimer.current);
      window.clearTimeout(pettedTimer.current);
      window.clearTimeout(pokeResetTimer.current);
    },
    [],
  );

  const patchConfig = useCallback((patch: Partial<OverlayConfigView>) => {
    void invoke('set_overlay_config', { cfg: { ...cfg, ...patch } }).catch(() => undefined);
  }, [cfg]);

  const openSettings = useCallback(() => {
    void invoke('open_settings_tab', { tab: 'pet' }).catch(() => undefined);
    void invoke('focus_main_window').catch(() => undefined);
  }, []);

  const onFigureClick = (e: ReactMouseEvent<HTMLDivElement>) => {
    lastUserTouch.current = Date.now();
    if (!interactive || cfg.edit) return;
    // 菜单开着时左键先关菜单，不抢轻碰
    if (menuOpenRef.current) {
      setMenuOpen(false);
      return;
    }
    // 本次按下发生过移动（或请求过原生拖动）就只算拖动，不抢主窗口焦点
    if (movedDuringDrag.current || dragRequested.current || Date.now() - draggedAt.current < 350) {
      movedDuringDrag.current = false;
      dragRequested.current = false;
      return;
    }
    // 点头部 = 摸头（害羞 + 爱心），点身体 = 轻碰招手；1.2s 内连点三下 = 庆祝
    const rect = figureRef.current?.getBoundingClientRect();
    const head = rect ? e.clientY - rect.top < rect.height * 0.38 : false;
    pokeCount.current += 1;
    window.clearTimeout(pokeResetTimer.current);
    pokeResetTimer.current = window.setTimeout(() => {
      pokeCount.current = 0;
    }, 1200);
    if (pokeCount.current >= 3) {
      pokeCount.current = 0;
      playIdlePose('celebrate', 1400);
    } else {
      setShyPose(head);
      setPoked(!head);
      window.clearTimeout(pokeTimer.current);
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
    if (Date.now() - lastPokeBubble.current > 8000) {
      lastPokeBubble.current = Date.now();
      pushBubble(t('pet.bubble.poke'), 'info');
    }
    void invoke('focus_main_window').catch(() => undefined);
  };

  const onFigureContextMenu = (e: ReactMouseEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();
    if (!interactive || cfg.edit) return;
    lastUserTouch.current = Date.now();
    // 右键当下立刻解穿透，避免菜单刚开就因轮询滞后再次穿透点不中
    void invoke('pet_set_clickable', { enabled: true }).catch(() => undefined);
    setMenuOpen(true);
  };

  const activeFrame = resolvePetPose({
    state: petState,
    poked,
    shy: shyPose,
    listening: nearListening,
    walking: nearWalking,
    celebrating,
    greeting,
    idlePose,
    roaming,
  });
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
    dragging ? 'dragging' : '',
  ].filter(Boolean).join(' ');

  const petOpacity = Math.min(1, Math.max(0.15, Number(cfg.opacity) || 1));

  return (
    <div
      ref={stageRef}
      className={classNames}
      data-state={petState}
      data-facing={facing}
      style={{ ['--pet-opacity' as string]: String(petOpacity) }}
    >
      {/* 环境层：禁止底板；只留粒子与完成火花（零光晕 / 零地面阴影） */}
      <div className="pet-ambient" aria-hidden="true">
        {[1, 2, 3, 4, 5, 6].map((n) => <i key={n} className="pet-mote" />)}
        <i className="pet-spark s1" />
        <i className="pet-spark s2" />
        <i className="pet-spark s3" />
      </div>

      {/* 信息层：气泡在左上角自适应排版，状态胶囊移到角色脚底 */}
      <div className="pet-side">
        {bubble && (
          <div key={bubble.id} className={`pet-bubble pet-bubble-${bubble.kind}`} role="status">
            {bubble.text}
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

      {/* 角色分层：头轴转向 → 呼吸 → 逐帧动画 → 眼睑/眼神 */}
      <div
        ref={figureRef}
        className={`pet-figure${dragging ? ' dragging' : ''}${poked ? ' poked' : ''}`}
        onMouseDown={onFigureMouseDown}
        onMouseMove={onFigureMouseMove}
        onContextMenu={onFigureContextMenu}
        onClick={onFigureClick}
        role={interactive ? 'button' : undefined}
        aria-label={interactive ? t(PET_LABEL[petState]) : undefined}
      >
        <div className="pet-motion">
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
          {blinkEnabled && <span className="pet-blink" aria-hidden="true" />}
          {gazeEnabled && <span className="pet-gaze" aria-hidden="true" />}
        </div>
      </div>

      {menuOpen && (
        <PetContextMenu
          cfg={cfg}
          onSettings={openSettings}
          onToggleQuiet={() => patchConfig({ pet_quiet: !cfg.pet_quiet })}
          onToggleRoam={() => patchConfig({ pet_roam: cfg.pet_roam === false })}
          onSize={(size) => patchConfig({ width: size.width, height: size.height, x: -1, y: -1 })}
          onDock={() => patchConfig({ x: -1, y: -1 })}
          onHide={() => patchConfig({ pet_enabled: false })}
          onClose={() => setMenuOpen(false)}
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

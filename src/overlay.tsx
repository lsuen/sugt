// 流量悬浮窗 / 桌面精灵（小su）：置顶 + 半透明 + 鼠标穿透。
// 数据与仪表盘「请求统计」同源（get_status.traffic + last_proxy_*），无假数据。
// 精灵状态机唯一依据是后端 active_requests（只统计真实 LLM 请求），不虚报状态：
//   请求级 → 只切角色动作（typing），不出泡；结果级事件才出泡，且做合并与去重。
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import './overlay.css';
import { t, type TKey } from './i18n';

type OverlayConfigView = {
  enabled: boolean;
  opacity: number;
  edit: boolean;
  show_tokens: boolean;
  show_client: boolean;
  show_channel: boolean;
  show_requests: boolean;
  x: number;
  y: number;
  layout: 'column' | 'row';
  width: number;
  height: number;
  pet_enabled: boolean;
  pet_interactive: boolean;
  pet_snap: boolean;
};

type TrafficSlice = {
  today_requests: number;
  total_requests: number;
  success_count: number;
  failed_count: number;
  today_input_tokens: number;
  today_output_tokens: number;
  // 真实在途 LLM 请求数；健康检查 / 看门狗心跳 / 免费通道探测不计入
  active_requests?: number;
};

type Status = {
  running: boolean;
  last_proxy_client?: string | null;
  last_proxy_provider?: string | null;
  last_proxy_path?: string | null;
  last_proxy_failover?: boolean;
  traffic: TrafficSlice;
};

type PetState = 'offline' | 'idle' | 'typing' | 'done' | 'error' | 'sleep';
type Bubble = { id: number; text: string; kind: 'ok' | 'error' | 'info' };

const PET_LABEL: Record<PetState, TKey> = {
  offline: 'pet.offline',
  idle: 'pet.idle',
  typing: 'pet.typing',
  done: 'pet.done',
  error: 'pet.error',
  sleep: 'pet.sleep',
};

// offline 复用 idle 帧，仅整体滤镜转灰
const PET_FRAME: Record<PetState, string> = {
  offline: 'idle',
  idle: 'idle',
  typing: 'typing',
  done: 'done',
  error: 'error',
  sleep: 'sleep',
};

// v0.7：五个状态各有 ComfyUI 编码的带 alpha 动画 WebP（6 帧 / 8fps）。
// 全部常驻并交叉淡入，切状态不重启动画时间线；reduced-motion / 窗口隐藏时切静态海报。
const PET_LOOPS = ['idle', 'typing', 'done', 'error', 'sleep'] as const;
const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

const fmt = (n: number) => (n >= 10000 ? `${(n / 10000).toFixed(1)}w` : n.toLocaleString());

type PetStageProps = {
  cfg: OverlayConfigView;
  status: Status | null;
  busy: boolean;
  onSave: () => void;
  onDrag: (e: React.MouseEvent<HTMLSpanElement>) => void;
};

function PetStage({ cfg, status, busy, onSave, onDrag }: PetStageProps) {
  const [petState, setPetState] = useState<PetState>('offline');
  const [bubble, setBubble] = useState<Bubble | null>(null);
  const baseline = useRef<{
    running: boolean;
    success: number;
    failed: number;
    failover: boolean;
  } | null>(null);
  const hold = useRef<{ state: PetState; until: number }>({ state: 'idle', until: 0 });
  // typing 余晖单独存放，避免新请求把正在保持的 error / done 态覆盖掉
  const typingUntil = useRef(0);
  const lastActivity = useRef(Date.now());
  const lastFailNotice = useRef(0);
  const doneBatch = useRef({ count: 0, timer: 0 });
  const bubbleSeq = useRef(0);
  // 最新一次状态样本的在途数：成功气泡必须等流真正结束才允许合并播报
  const activeNow = useRef(0);
  const wasActive = useRef(false);
  // 拖动状态位：只有「本次按下 → 窗口确实移动」才算用户拖动。
  // 启动停靠、改尺寸、回到右下角、吸附自身的二次移动都不应满足这个序列。
  const dragActive = useRef(false);
  const movedDuringDrag = useRef(false);
  const dragRequested = useRef(false);
  const dragCleanupTimer = useRef(0);
  const pendingDrag = useRef<{ x: number; y: number } | null>(null);
  const draggedAt = useRef(0);
  const finishing = useRef(false);
  const [dragging, setDragging] = useState(false);
  // v0.7 交互层：全部通过 CSS 变量 / class 驱动，避免 mousemove 触发 React 重渲染
  const stageRef = useRef<HTMLDivElement>(null);
  const figureRef = useRef<HTMLDivElement>(null);
  const gazeRef = useRef({ x: 0, y: 0, near: 0 });
  const blinkTimers = useRef<number[]>([]);
  const pokeTimer = useRef(0);
  const lastPokeBubble = useRef(0);
  const [blinking, setBlinking] = useState(false);
  const [poked, setPoked] = useState(false);
  const [still, setStill] = useState(false);

  const clearDragFlags = useCallback(() => {
    dragActive.current = false;
    movedDuringDrag.current = false;
    dragRequested.current = false;
    pendingDrag.current = null;
    setDragging(false);
  }, []);

  const beginDrag = useCallback(() => {
    dragActive.current = true;
    movedDuringDrag.current = false;
    dragRequested.current = false;
    window.clearTimeout(dragCleanupTimer.current);
    // 原生 start_dragging 在部分平台会吞掉 pointerup，超时兜底避免状态位永久卡住
    dragCleanupTimer.current = window.setTimeout(clearDragFlags, 5000);
  }, [clearDragFlags]);

  // 松手后仍保留一小段 movedDuringDrag，用来吞掉原生拖动后迟到的 click
  useEffect(() => {
    const onUp = () => {
      if (!dragActive.current) return;
      dragActive.current = false;
      setDragging(false);
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
    };
  }, []);

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

  // 视线跟随：鼠标在悬浮窗内移动时，头部做轻微 3D 转向并带动眼神高光。
  // 无鼠标（非互动模式为鼠标穿透）时每隔几秒自主张望一次，角色不会像静止贴图。
  useEffect(() => {
    const stage = stageRef.current;
    if (!stage) return;
    let raf = 0;
    let scanTimer = 0;
    let resetTimer = 0;
    const write = (x: number, y: number, near: number) => {
      stage.style.setProperty('--gaze-x', x.toFixed(3));
      stage.style.setProperty('--gaze-y', y.toFixed(3));
      stage.style.setProperty('--near', near.toFixed(3));
      stage.classList.toggle('nearby', near > 0.12);
    };
    const onMove = (event: PointerEvent) => {
      const rect = figureRef.current?.getBoundingClientRect();
      if (!rect) return;
      const cx = rect.left + rect.width * 0.55;
      const cy = rect.top + rect.height * 0.32;
      const distance = Math.hypot(event.clientX - cx, event.clientY - cy);
      const near = clamp(1 - distance / 170, 0, 1);
      const x = clamp((event.clientX - cx) / (rect.width * 0.9), -1, 1);
      const y = clamp((event.clientY - cy) / (rect.height * 0.6), -1, 1);
      gazeRef.current = { x, y, near };
      if (raf) cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => { raf = 0; write(x, y, near); });
    };
    const onLeave = () => {
      gazeRef.current = { x: 0, y: 0, near: 0 };
      write(0, 0, 0);
    };
    const scan = () => {
      if (gazeRef.current.near < 0.06 && !document.hidden) {
        const x = (Math.random() * 2 - 1) * 0.42;
        const y = (Math.random() * 2 - 1) * 0.22;
        write(x, y, 0);
        window.clearTimeout(resetTimer);
        resetTimer = window.setTimeout(() => write(0, 0, 0), 1500);
      }
      scanTimer = window.setTimeout(scan, 4200 + Math.random() * 4200);
    };
    document.addEventListener('pointermove', onMove, { passive: true });
    document.addEventListener('pointerleave', onLeave);
    scanTimer = window.setTimeout(scan, 2600);
    return () => {
      document.removeEventListener('pointermove', onMove);
      document.removeEventListener('pointerleave', onLeave);
      if (raf) cancelAnimationFrame(raf);
      window.clearTimeout(scanTimer);
      window.clearTimeout(resetTimer);
    };
  }, []);

  // 随机眨眼（含偶发双眨）；睡眠状态由 CSS 保持闭眼，这里只管清醒态。
  useEffect(() => {
    let cancelled = false;
    const later = (fn: () => void, ms: number) => {
      blinkTimers.current.push(window.setTimeout(fn, ms));
    };
    const schedule = () => later(() => {
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
    }, 2600 + Math.random() * 4200);
    schedule();
    return () => {
      cancelled = true;
      blinkTimers.current.forEach(window.clearTimeout);
      blinkTimers.current = [];
    };
  }, []);

  // Codex 桌面宠物式「松手即停靠」：原生拖动期间 onMoved 会连续触发，
  // 防抖 320ms 后统一做贴边吸附 + 持久化；吸附引起的二次 onMoved 由 finishing 屏蔽。
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let timer = 0;
    let tiltTimer = 0;
    let last: { x: number; y: number } | null = null;
    void getCurrentWindow()
      .onMoved(({ payload }) => {
        if (!dragActive.current || finishing.current) return;
        movedDuringDrag.current = true;
        draggedAt.current = Date.now();
        setDragging(true);
        // 拖动方向带一点身体倾斜，松手后平滑回正；只写 CSS 变量，不触发重渲染
        if (last && stageRef.current) {
          const tilt = clamp((payload.x - last.x) / 14, -1, 1);
          stageRef.current.style.setProperty('--drag-tilt', tilt.toFixed(3));
          window.clearTimeout(tiltTimer);
          tiltTimer = window.setTimeout(() => {
            stageRef.current?.style.setProperty('--drag-tilt', '0');
          }, 340);
        }
        last = { x: payload.x, y: payload.y };
        window.clearTimeout(timer);
        timer = window.setTimeout(() => {
          setDragging(false);
          last = null;
          stageRef.current?.style.setProperty('--drag-tilt', '0');
          finishing.current = true;
          void invoke('pet_finish_drag', { snap: cfg.pet_snap !== false })
            .catch(() => undefined)
            .finally(() => {
              window.setTimeout(() => { finishing.current = false; }, 250);
            });
        }, 320);
      })
      .then((fn) => { unlisten = fn; })
      .catch(() => undefined);
    return () => {
      window.clearTimeout(timer);
      window.clearTimeout(tiltTimer);
      unlisten?.();
    };
  }, [cfg.pet_snap]);

  const pushBubble = useCallback((text: string, kind: Bubble['kind']) => {
    bubbleSeq.current += 1;
    setBubble({ id: bubbleSeq.current, text, kind });
  }, []);

  // 气泡只「看」不「点」：无按钮、不承载写操作，到时自动消失
  useEffect(() => {
    if (!bubble) return;
    const timer = window.setTimeout(() => setBubble(null), 5200);
    return () => window.clearTimeout(timer);
  }, [bubble]);

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
    activeNow.current = active;

    // 首个样本只建立基线，避免刚打开窗口就误报「网关已就绪」
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
    }
    if (running && failover && !prev.failover) {
      pushBubble(t('pet.bubble.failover'), 'info');
    }
    // 上游 4xx / 空响应：必须切 error 并出泡，不允许停在敲键盘或假报成功
    if (failed > prev.failed) {
      if (now - lastFailNotice.current > 5 * 60 * 1000) {
        pushBubble(t('pet.bubble.failed'), 'error');
        lastFailNotice.current = now;
      }
      hold.current = { state: 'error', until: now + 6000 };
    }
    // 连续成功合并成一条，静默 8s 后出泡，避免高频请求把气泡刷成骚扰。
    // 上游在响应头到达时就记成功，流式 body 可能还在输出：只要仍有在途请求就继续等，
    // 不允许「流还在跑，精灵已经报完成」。
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
    // 真实在途请求优先；留 1.5s 余晖仅作轮询间隙的 UI 防抖
    if (active > 0) typingUntil.current = now + 1500;
    if (active > 0 || success > prev.success) lastActivity.current = now;

    // 流式响应真正结束的那一刻补一次完成态：此前 success 已 +1，但 typing 一直压着 done
    const errorHeld = hold.current.state === 'error' && hold.current.until > now;
    if (wasActive.current && active === 0 && !errorHeld) {
      hold.current = { state: 'done', until: now + 3500 };
    }
    wasActive.current = active > 0;

    const held = hold.current;
    let next: PetState;
    if (!running) next = 'offline';
    else if (active > 0 || typingUntil.current > now) next = 'typing';
    else if (held.state === 'error' && held.until > now) next = 'error';
    else if (held.state === 'done' && held.until > now) next = 'done';
    else if (now - lastActivity.current > 60_000) next = 'sleep';
    else next = 'idle';
    setPetState(next);

    baseline.current = { running, success, failed, failover };
  }, [status, pushBubble]);

  useEffect(
    () => () => {
      if (doneBatch.current.timer) window.clearTimeout(doneBatch.current.timer);
    },
    [],
  );

  const interactive = cfg.pet_interactive || cfg.edit;

  // 4px 阈值：纯单击不进入原生拖动循环（部分平台会吞掉 click），移动后才真正开始拖。
  const onFigureMouseDown = (e: React.MouseEvent<HTMLDivElement>) => {
    if (!interactive || cfg.edit || e.button !== 0) return;
    e.preventDefault();
    pendingDrag.current = { x: e.clientX, y: e.clientY };
    beginDrag();
  };
  const onFigureMouseMove = (e: React.MouseEvent<HTMLDivElement>) => {
    const start = pendingDrag.current;
    if (!start) return;
    if (Math.abs(e.clientX - start.x) + Math.abs(e.clientY - start.y) < 4) return;
    pendingDrag.current = null;
    dragRequested.current = true;
    void invoke('overlay_start_drag').catch(() => undefined);
  };
  const onFigureClick = () => {
    pendingDrag.current = null;
    if (!interactive || cfg.edit) return;
    // 本次按下发生过移动（或请求过原生拖动）就只算拖动，不抢主窗口焦点
    if (movedDuringDrag.current || dragRequested.current || Date.now() - draggedAt.current < 350) {
      movedDuringDrag.current = false;
      dragRequested.current = false;
      return;
    }
    // 轻碰反馈：一次性弹跳 + 偶尔一句轻声回应，不打断状态机、不刷屏
    setPoked(true);
    window.clearTimeout(pokeTimer.current);
    pokeTimer.current = window.setTimeout(() => setPoked(false), 480);
    if (Date.now() - lastPokeBubble.current > 8000) {
      lastPokeBubble.current = Date.now();
      pushBubble(t('pet.bubble.poke'), 'info');
    }
    void invoke('focus_main_window').catch(() => undefined);
  };

  useEffect(() => () => window.clearTimeout(pokeTimer.current), []);

  const activeFrame = PET_FRAME[petState];
  const classNames = [
    'pet-stage',
    interactive ? 'interactive' : '',
    blinking ? 'blinking' : '',
    still ? 'still' : '',
  ].filter(Boolean).join(' ');

  return (
    <div ref={stageRef} className={classNames} data-state={petState}>
      {/* 环境层：靠近光晕、地面投影、漂浮粒子、完成时的一次性火花 */}
      <div className="pet-ambient" aria-hidden="true">
        <span className="pet-halo" />
        <span className="pet-ground" />
        {[1, 2, 3, 4, 5, 6].map((n) => <i key={n} className="pet-mote" />)}
        <i className="pet-spark s1" />
        <i className="pet-spark s2" />
        <i className="pet-spark s3" />
      </div>

      {/* 左侧通知栏：气泡与状态胶囊都不占用立绘区域，角色头顶不再被遮住 */}
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

      {/* 角色分层：视线 3D → 呼吸 → 逐帧动画 → 眼睑/眼神；各层不争抢同一动画属性 */}
      <div
        ref={figureRef}
        className={`pet-figure${dragging ? ' dragging' : ''}${poked ? ' poked' : ''}`}
        onMouseDown={onFigureMouseDown}
        onMouseMove={onFigureMouseMove}
        onContextMenu={(e) => e.preventDefault()}
        onClick={onFigureClick}
        role={interactive ? 'button' : undefined}
        aria-label={interactive ? t(PET_LABEL[petState]) : undefined}
      >
        <div className="pet-motion">
          <div className="pet-loops">
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
          <span className="pet-blink" aria-hidden="true" />
          <span className="pet-gaze" aria-hidden="true" />
        </div>
      </div>

      {cfg.edit && (
        <div className="pet-edit-bar">
          <button type="button" className="overlay-save" onClick={onSave} disabled={busy}>
            {busy ? '…' : t('overlay.save')}
          </button>
          <span
            className="overlay-grip"
            onMouseDown={(e) => { onDrag(e); beginDrag(); }}
            title={t('overlay.drag')}
            aria-label={t('overlay.drag')}
          />
        </div>
      )}
    </div>
  );
}

function Overlay() {
  const [cfg, setCfg] = useState<OverlayConfigView | null>(null);
  const [status, setStatus] = useState<Status | null>(null);
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
    const poll = () => invoke<Status>('get_status').then(setStatus).catch(() => undefined);
    poll();
    const timer = window.setInterval(poll, 2000);
    return () => window.clearInterval(timer);
  }, []);

  const savePos = async () => {
    if (!cfg || busy) return;
    setBusy(true);
    try {
      const pos = await getCurrentWindow().outerPosition();
      await invoke('set_overlay_config', { cfg: { ...cfg, x: pos.x, y: pos.y, edit: false } });
    } catch {
      // 窗口被关闭等场景：静默即可
    } finally {
      setBusy(false);
    }
  };

  const startDrag = (e: React.MouseEvent<HTMLSpanElement>) => {
    e.preventDefault();
    void invoke('overlay_start_drag').catch(() => undefined);
  };

  // 精灵模式与原 HUD 完全并存：关掉开关即回到今天的流量悬浮窗，可回退
  if (cfg?.pet_enabled) {
    return <PetStage cfg={cfg} status={status} busy={busy} onSave={() => void savePos()} onDrag={startDrag} />;
  }

  const traffic = status?.traffic;
  const running = status?.running ?? false;
  const today = traffic?.today_requests ?? 0;
  const total = traffic?.total_requests ?? 0;
  const okRate =
    total > 0 && traffic
      ? Math.round((traffic.success_count / total) * 100)
      : 100;
  const client = status?.last_proxy_client?.trim() || '';
  const channel = status?.last_proxy_provider?.trim() || '';
  const isRow = cfg?.layout === 'row';
  const showReq = cfg?.show_requests !== false;
  const showTok = cfg?.show_tokens !== false;
  const showClient = cfg?.show_client !== false;
  const showChannel = cfg?.show_channel !== false;
  const empty = today === 0 && !client && !channel;

  return (
    <div
      className={cfg?.edit ? 'overlay-box editing' : 'overlay-box'}
      style={{ background: `rgba(7, 11, 20, ${cfg?.opacity ?? 0.7})` }}
    >
      {cfg?.edit && (
        <div className="overlay-edit-bar">
          <button
            type="button"
            className="overlay-save"
            onClick={() => void savePos()}
            disabled={busy}
          >
            {busy ? '…' : t('overlay.save')}
          </button>
          <span
            className="overlay-grip"
            onMouseDown={startDrag}
            title={t('overlay.drag')}
            aria-label={t('overlay.drag')}
          />
        </div>
      )}

      <div className={isRow ? 'overlay-body row' : 'overlay-body'}>
        {!running ? (
          <div className="overlay-idle">{t('overlay.off')}</div>
        ) : empty ? (
          <div className="overlay-idle">{t('overlay.idle')}</div>
        ) : (
          <>
            {(showReq || showTok) && (
              <div className="overlay-block">
                {showReq && (
                  <div className="overlay-line">
                    <span className="overlay-k">{t('overlay.today')}</span>
                    <span className="overlay-v">{fmt(today)}</span>
                    {total > 0 && (
                      <span className={okRate < 90 ? 'overlay-fail' : 'overlay-ok'}>
                        {okRate}%
                      </span>
                    )}
                  </div>
                )}
                {showTok && traffic && (
                  <div className="overlay-line">
                    <span className="overlay-k">{t('overlay.flow')}</span>
                    <span className="overlay-v tabular">
                      ↑{fmt(traffic.today_input_tokens)}
                      <span className="overlay-dim">/</span>
                      ↓{fmt(traffic.today_output_tokens)}
                    </span>
                  </div>
                )}
              </div>
            )}
            {(showClient || showChannel) && (
              <div className="overlay-block">
                {showClient && (
                  <div className="overlay-line">
                    <span className="overlay-k">{t('overlay.source')}</span>
                    <span className="overlay-v trunc" title={client || '—'}>
                      {client || '—'}
                    </span>
                  </div>
                )}
                {showChannel && (
                  <div className="overlay-line">
                    <span className="overlay-k">{t('overlay.channel')}</span>
                    <span className="overlay-v trunc" title={channel || '—'}>
                      {channel || '—'}
                    </span>
                  </div>
                )}
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}

createRoot(document.getElementById('root')!).render(<Overlay />);

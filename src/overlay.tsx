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

// offline 复用 idle 帧，仅核心色转为灰色
const PET_FRAME: Record<PetState, string> = {
  offline: 'idle',
  idle: 'idle',
  typing: 'typing',
  done: 'done',
  error: 'error',
  sleep: 'sleep',
};

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

  // Codex 桌面宠物式「松手即停靠」：原生拖动期间 onMoved 会连续触发，
  // 防抖 320ms 后统一做贴边吸附 + 持久化；吸附引起的二次 onMoved 由 finishing 屏蔽。
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let timer = 0;
    void getCurrentWindow()
      .onMoved(() => {
        if (!dragActive.current || finishing.current) return;
        movedDuringDrag.current = true;
        draggedAt.current = Date.now();
        setDragging(true);
        window.clearTimeout(timer);
        timer = window.setTimeout(() => {
          setDragging(false);
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
    void invoke('focus_main_window').catch(() => undefined);
  };

  return (
    <div className={`pet-stage${interactive ? ' interactive' : ''}`} data-state={petState}>
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

      {/* 外层负责呼吸/整体滤镜，内层负责状态动效：两者不再争抢同一个 animation 属性 */}
      <div
        className={`pet-figure${dragging ? ' dragging' : ''}`}
        onMouseDown={onFigureMouseDown}
        onMouseMove={onFigureMouseMove}
        onContextMenu={(e) => e.preventDefault()}
        onClick={onFigureClick}
        role={interactive ? 'button' : undefined}
        aria-label={interactive ? t(PET_LABEL[petState]) : undefined}
      >
        <span className="pet-pose" data-frame={PET_FRAME[petState]} aria-hidden="true" />
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

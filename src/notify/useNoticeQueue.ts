/**
 * 通知队列 Hook（1.2.0）
 * 作者：孙文龙 sun
 * 作用：把统一通知模型接到 React，提供入队、合并、过期清理、优先级截取与悬停暂停。
 * 说明：push 兼容旧的 pushToast(text, kind?) 调用形态，调用方可以渐进迁移。
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  DEFAULT_CAPACITY,
  createNotice,
  mergeNotices,
  pruneNotices,
  selectVisible,
  shiftNoticeDeadlines,
  type Notice,
  type NoticeInput,
  type NoticeSeverity,
} from './messageModel';

export interface UseNoticeQueueOptions {
  capacity?: number;
  surface?: 'toast' | 'bubble';
  now?: () => number;
}

export interface NoticeQueue {
  notices: Notice[];
  visible: Notice[];
  hidden: number;
  push: (input: NoticeInput | string, kind?: NoticeSeverity) => number;
  dismiss: (id: number) => void;
  setPaused: (paused: boolean) => void;
  clear: () => void;
  paused: boolean;
}

export function useNoticeQueue(options: UseNoticeQueueOptions = {}): NoticeQueue {
  const capacity = options.capacity ?? DEFAULT_CAPACITY;
  const surface = options.surface ?? 'toast';
  const now = options.now ?? (() => Date.now());
  const nowRef = useRef(now);
  nowRef.current = now;

  const [notices, setNotices] = useState<Notice[]>([]);
  const [paused, setPausedState] = useState(false);
  const seqRef = useRef(0);
  const pausedRef = useRef(false);
  const pausedAtRef = useRef<number | null>(null);
  // 渲染期同步队列快照，供 dismiss/clear 在同一批次内判断「是否清到最后一条」。 —— 孙文龙 sun
  const noticesRef = useRef<Notice[]>([]);
  noticesRef.current = notices;

  /** 退出暂停：宿主可能在队列清空时直接卸载，onMouseLeave 不会触发，必须能独立复位。 */
  const resetPause = useCallback(() => {
    pausedAtRef.current = null;
    pausedRef.current = false;
    setPausedState(false);
  }, []);

  const push = useCallback((input: NoticeInput | string, kind?: NoticeSeverity): number => {
    const text = typeof input === 'string' ? input : input.text;
    if (!text || !text.trim()) return -1;
    seqRef.current += 1;
    const id = seqRef.current;
    const incoming = createNotice(id, input, nowRef.current(), kind, surface);
    setNotices((prev) => mergeNotices(prev, incoming, nowRef.current()).list);
    return id;
  }, [surface]);

  const dismiss = useCallback((id: number) => {
    setNotices((prev) => {
      const next = prev.filter((item) => item.id !== id);
      return next.length === prev.length ? prev : next;
    });
    // 关闭最后一条时 ToastHost 返回 null 并卸载，鼠标离开事件丢失，这里主动退出暂停（QD-02）。
    if (noticesRef.current.filter((item) => item.id !== id).length === 0) resetPause();
  }, [resetPause]);

  const clear = useCallback(() => {
    resetPause();
    setNotices([]);
  }, [resetPause]);

  const setPaused = useCallback((next: boolean) => {
    const current = nowRef.current();
    if (next) {
      if (pausedAtRef.current !== null) return;
      // 队列已空时不进入暂停，避免残留一个无人能结束的暂停态。 —— 孙文龙 sun
      if (noticesRef.current.length === 0) {
        resetPause();
        return;
      }
      pausedAtRef.current = current;
      pausedRef.current = true;
      setPausedState(true);
      return;
    }
    if (pausedAtRef.current === null) {
      resetPause();
      return;
    }
    const pausedAt = pausedAtRef.current;
    const delta = current - pausedAt;
    pausedAtRef.current = null;
    pausedRef.current = false;
    setPausedState(false);
    // 只顺延暂停前已存在的消息；暂停中新建的按原 TTL 计时（QD-01）。 —— 孙文龙 sun
    setNotices((prev) => shiftNoticeDeadlines(prev, delta, pausedAt, current));
  }, [resetPause]);

  // 兜底：无论队列因关闭、清空还是外部 prune 变空，都退出暂停，防止 pausedRef 永久卡住（QD-02）。
  useEffect(() => {
    if (notices.length === 0 && (pausedRef.current || pausedAtRef.current !== null)) resetPause();
  }, [notices.length, resetPause]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      if (pausedRef.current) return;
      const current = nowRef.current();
      setNotices((prev) => {
        const next = pruneNotices(prev, current);
        return next.length === prev.length ? prev : next;
      });
    }, 300);
    return () => window.clearInterval(timer);
  }, []);

  const selection = useMemo(() => selectVisible(notices, capacity), [notices, capacity]);

  return {
    notices,
    visible: selection.visible,
    hidden: selection.hidden,
    push,
    dismiss,
    setPaused,
    clear,
    paused,
  };
}

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
    setNotices((prev) => prev.filter((item) => item.id !== id));
  }, []);

  const clear = useCallback(() => setNotices([]), []);

  const setPaused = useCallback((next: boolean) => {
    const current = nowRef.current();
    if (next && pausedAtRef.current === null) {
      pausedAtRef.current = current;
      pausedRef.current = true;
      setPausedState(true);
      return;
    }
    if (!next && pausedAtRef.current !== null) {
      const delta = current - pausedAtRef.current;
      pausedAtRef.current = null;
      pausedRef.current = false;
      setPausedState(false);
      setNotices((prev) => shiftNoticeDeadlines(prev, delta));
    }
  }, []);

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

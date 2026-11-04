import React, { useEffect, useMemo, useRef, useState } from 'react';
import { lang, t } from './i18n';

type LogEntry = {
  key: string;
  time?: string;
  level: string;
  message: string;
  target?: string;
};

function parseLogLine(line: string, index: number): LogEntry {
  try {
    const data = JSON.parse(line) as {
      timestamp?: string;
      level?: string;
      message?: string;
      target?: string;
      fields?: Record<string, unknown>;
    };
    const fields = data.fields ?? {};
    const base = String(fields.message ?? data.message ?? line);
    const extras = ['client', 'provider', 'model', 'path', 'mode', 'latency_ms', 'status', 'failover', 'error', 'reason']
      .filter((key) => fields[key] !== undefined && fields[key] !== null && fields[key] !== '')
      .map((key) => `${key}=${String(fields[key])}`);
    const message = extras.length ? `${base} · ${extras.join(' · ')}` : base;
    let time: string | undefined;
    if (data.timestamp) {
      const date = new Date(data.timestamp);
      if (!Number.isNaN(date.getTime())) {
        time = date.toLocaleTimeString(lang() === 'zh' ? 'zh-CN' : undefined, { hour12: false });
      }
    }
    return {
      key: `${index}-${data.timestamp ?? line.slice(0, 24)}`,
      time,
      level: (data.level ?? 'INFO').toUpperCase(),
      message,
      target: data.target,
    };
  } catch {
    return { key: `${index}-raw`, level: 'LOG', message: line };
  }
}

type Props = {
  lines: string[];
  compact?: boolean;
};

export function LogViewer({ lines, compact }: Props) {
  const endRef = useRef<HTMLDivElement>(null);
  const viewportRef = useRef<HTMLDivElement>(null);
  const [autoScroll, setAutoScroll] = useState(true);

  const entries = useMemo(() => lines.map((line, index) => parseLogLine(line, index)), [lines]);

  useEffect(() => {
    if (!autoScroll) return;
    endRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [entries, autoScroll]);

  const onScroll = () => {
    const el = viewportRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
    setAutoScroll(atBottom);
  };

  if (!lines.length) {
    return <div className="log-empty">{t('logs.empty')}</div>;
  }

  return (
    <>
      <div className="log-toolbar">
        <span className="hint compact">{t('logs.count', { n: lines.length })}</span>
        <button
          type="button"
          className={`ghost tiny-btn${autoScroll ? ' active' : ''}`}
          onClick={() => {
            setAutoScroll(true);
            endRef.current?.scrollIntoView({ behavior: 'smooth' });
          }}
        >
          {autoScroll ? t('logs.follow') : t('logs.toBottom')}
        </button>
      </div>
      <div
        className={`log-viewport${compact ? ' log-viewport-compact' : ''}`}
        ref={viewportRef}
        onScroll={onScroll}
      >
        {entries.map((entry) => (
          <div className={`log-line level-${entry.level.toLowerCase()}`} key={entry.key}>
            {entry.time ? <span className="log-time">{entry.time}</span> : null}
            <span className={`log-level lv-${entry.level.toLowerCase()}`}>{entry.level}</span>
            <span className="log-msg" title={entry.target}>{entry.message}</span>
          </div>
        ))}
        <div ref={endRef} />
      </div>
    </>
  );
}

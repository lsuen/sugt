import React, { useState } from 'react';
import type { TrafficStats } from './providerPresets';

function TrafficChart({ buckets }: { buckets: TrafficStats['hourly_buckets'] }) {
  const max = Math.max(...buckets.map((b) => b.success + b.failed), 1);
  return (
    <div className="traffic-chart" aria-label="近 60 分钟请求量">
      {buckets.map((bucket, index) => {
        const total = bucket.success + bucket.failed;
        const height = Math.max(4, Math.round((total / max) * 100));
        return (
          <div className="traffic-bar-col" key={index}>
            <div className="traffic-bar-stack" style={{ height: `${height}%` }}>
              {bucket.failed > 0 && (
                <div
                  className="traffic-bar fail"
                  style={{ flexGrow: total - bucket.success }}
                />
              )}
              {bucket.success > 0 && (
                <div
                  className="traffic-bar success"
                  style={{ flexGrow: bucket.success }}
                />
              )}
              {total === 0 && <div className="traffic-bar empty" />}
            </div>
          </div>
        );
      })}
      <div className="traffic-chart-labels">
        <span>60 分钟前</span>
        <span>现在</span>
      </div>
    </div>
  );
}

export function TrafficPanel({ traffic, running }: { traffic?: TrafficStats; running?: boolean }) {
  const [expanded, setExpanded] = useState(false);
  if (!traffic) {
    return (
      <div className="card traffic-card">
        <h3>请求统计</h3>
        <p className="hint compact">等待网关状态…</p>
      </div>
    );
  }

  const failText = traffic.fail_rate_percent.toFixed(1);

  return (
    <div className="card traffic-card">
      <div className="section-title">
        <h3>请求统计</h3>
        <span className="hint compact">{running ? '实时累计' : '网关未运行'}</span>
      </div>
      <div className="traffic-metrics">
        <div className="traffic-metric">
          <span>今日请求</span>
          <strong>{traffic.today_requests}</strong>
        </div>
        <div className="traffic-metric">
          <span>累计请求</span>
          <strong>{traffic.total_requests}</strong>
        </div>
        <div className="traffic-metric">
          <span>平均耗时</span>
          <strong>{traffic.avg_latency_ms} ms</strong>
        </div>
        <div className="traffic-metric">
          <span>失败占比</span>
          <strong className={traffic.fail_rate_percent > 10 ? 'store-error' : ''}>{failText}%</strong>
        </div>
        <div className="traffic-metric">
          <span>活跃来源</span>
          <strong>{traffic.active_clients}</strong>
        </div>
      </div>
      <TrafficChart buckets={traffic.hourly_buckets} />
      <p className="hint compact">近 60 分钟 · 绿=成功 红=失败</p>

      {traffic.recent_clients.length > 0 && (
        <div className="traffic-clients">
          <button
            type="button"
            className="ghost tiny-btn traffic-clients-toggle"
            onClick={() => setExpanded((v) => !v)}
          >
            {expanded ? '收起调用来源' : `查看调用来源（${traffic.recent_clients.length}）`}
          </button>
          {expanded && (
            <div className="store-mini-list traffic-client-list">
              {traffic.recent_clients.map((client) => (
                <div className="store-mini-row" key={`${client.label}-${client.last_at}`}>
                  <div>
                    <strong>{client.label}</strong>
                    <span className="hint compact">
                      {client.provider_name} · {client.model_name} · {client.request_count} 次
                    </span>
                  </div>
                  <span className="hint compact">
                    {client.last_path}
                    <br />
                    {new Date(client.last_at).toLocaleString()}
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

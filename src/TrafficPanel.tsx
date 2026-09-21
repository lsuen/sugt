import React, { useState } from 'react';
import type { TrafficStats } from './providerPresets';
import { t } from './i18n';

function TrafficChart({ buckets }: { buckets: TrafficStats['hourly_buckets'] }) {
  const max = Math.max(...buckets.map((b) => b.success + b.failed), 1);
  return (
    <div className="traffic-chart" aria-label={t('trafstat.chartAria')}>
      {buckets.map((bucket, index) => {
        const total = bucket.success + bucket.failed;
        const height = Math.max(4, Math.round((total / max) * 100));
        return (
          <div className="traffic-bar-col" key={index}>
            <div className="traffic-bar-stack" style={{ height: `${height}%` }}>
              {bucket.failed > 0 && (
                <div
                  className="traffic-bar fail"
                  style={{ flexGrow: Math.max(total - bucket.success, 1) }}
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
        <span>{t('trafstat.minutesAgo')}</span>
        <span>{t('trafstat.now')}</span>
      </div>
    </div>
  );
}

function formatToken(n: number): string {
  if (n >= 10000) return `${(n / 10000).toFixed(1)}w`;
  return n.toLocaleString();
}

function ClientsDetailModal({
  clients,
  onClose,
}: {
  clients: TrafficStats['recent_clients'];
  onClose: () => void;
}) {
  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal modal-compact traffic-clients-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>{t('trafstat.sourceTitle', { n: clients.length })}</h3>
          <button type="button" className="icon-btn" onClick={onClose} aria-label={t('common.close')}>×</button>
        </div>
        <div className="modal-body traffic-clients-modal-body">
          <div className="store-mini-list traffic-client-list-scroll">
            {clients.map((client) => (
              <div className="store-mini-row" key={`${client.label}-${client.last_at}`}>
                <div>
                  <strong>{client.label}</strong>
                  <span className="hint compact">
                    {client.provider_name} · {client.model_name}
                    {' · '}
                    {t('trafstat.requests', { n: client.request_count })}
                    {client.failed_count > 0
                      ? ` · ${t('trafstat.failedN', { n: client.failed_count })}`
                      : ''}
                  </span>
                  <span className="hint compact">
                    {t('trafstat.tokensLine', {
                      in: formatToken(client.input_tokens ?? 0),
                      out: formatToken(client.output_tokens ?? 0),
                    })}
                  </span>
                </div>
                <span className="hint compact traffic-client-meta">
                  {client.last_path}
                  <br />
                  {new Date(client.last_at).toLocaleString()}
                </span>
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

function ChannelsDetailModal({
  channels,
  onClose,
}: {
  channels: TrafficStats['recent_channels'];
  onClose: () => void;
}) {
  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal modal-compact traffic-clients-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>{t('trafstat.channelTitle', { n: channels.length })}</h3>
          <button type="button" className="icon-btn" onClick={onClose} aria-label={t('common.close')}>×</button>
        </div>
        <div className="modal-body traffic-clients-modal-body">
          <div className="store-mini-list traffic-client-list-scroll">
            {channels.map((ch) => (
              <div className="store-mini-row" key={`${ch.provider_name}-${ch.last_at}`}>
                <div>
                  <strong>{ch.provider_name}</strong>
                  <span className="hint compact">
                    {t('trafstat.requests', { n: ch.request_count })}
                    {ch.failed_count > 0
                      ? ` · ${t('trafstat.failedN', { n: ch.failed_count })}`
                      : ''}
                    {ch.last_model ? ` · ${ch.last_model}` : ''}
                  </span>
                  <span className="hint compact">
                    {t('trafstat.tokensLine', {
                      in: formatToken(ch.input_tokens ?? 0),
                      out: formatToken(ch.output_tokens ?? 0),
                    })}
                  </span>
                </div>
                <span className="hint compact traffic-client-meta">
                  {new Date(ch.last_at).toLocaleString()}
                </span>
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

type Props = {
  traffic?: TrafficStats;
  running?: boolean;
  lastProvider?: string | null;
  lastPath?: string | null;
};

export function TrafficPanel({ traffic, running, lastProvider, lastPath }: Props) {
  const [clientsOpen, setClientsOpen] = useState(false);
  const [channelsOpen, setChannelsOpen] = useState(false);

  if (!traffic) {
    return (
      <div className="card traffic-card">
        <h3>{t('trafstat.title')}</h3>
        <p className="hint compact">{t('trafstat.waiting')}</p>
      </div>
    );
  }

  const failText = traffic.fail_rate_percent.toFixed(1);
  const successRate = traffic.total_requests === 0
    ? 100
    : ((traffic.success_count / traffic.total_requests) * 100);
  const empty = traffic.total_requests === 0 && traffic.today_requests === 0;
  const channels = traffic.recent_channels ?? [];

  return (
    <div className="card traffic-card">
      <div className="section-title">
        <h3>{t('trafstat.title')}</h3>
        <span className="hint compact">{running ? t('trafstat.realtime') : t('trafstat.notRunning')}</span>
      </div>
      {empty ? (
        <p className="hint compact traffic-empty">
          {running ? t('trafstat.emptyRunning') : t('trafstat.emptyStopped')}
        </p>
      ) : (
        <>
          <div className="traffic-metrics">
            <div className="traffic-metric">
              <span>{t('trafstat.today')}</span>
              <strong>{traffic.today_requests}</strong>
            </div>
            <div className="traffic-metric">
              <span>{t('trafstat.total')}</span>
              <strong>{traffic.total_requests}</strong>
            </div>
            <div className="traffic-metric">
              <span>{t('trafstat.avgLatency')}</span>
              <strong>{traffic.avg_latency_ms}<span className="traffic-unit">ms</span></strong>
            </div>
            <div className="traffic-metric">
              <span>{t('trafstat.successRate')}</span>
              <strong className={successRate < 90 ? 'store-error' : ''}>{successRate.toFixed(0)}%</strong>
            </div>
            <div className="traffic-metric">
              <span>{t('trafstat.failed')}</span>
              <strong className={traffic.fail_rate_percent > 10 ? 'store-error' : ''}>
                {traffic.failed_count}
                <span className="traffic-unit">/{failText}%</span>
              </strong>
            </div>
            <div className="traffic-metric">
              <span>{t('trafstat.sources')}</span>
              <strong>{traffic.active_clients}</strong>
            </div>
            <div className="traffic-metric">
              <span>{t('trafstat.tokenIn')}</span>
              <strong>{formatToken(traffic.today_input_tokens)}</strong>
            </div>
            <div className="traffic-metric">
              <span>{t('trafstat.tokenOut')}</span>
              <strong>{formatToken(traffic.today_output_tokens)}</strong>
            </div>
          </div>
          {(lastProvider || lastPath) && (
            <p className="hint compact traffic-last-hit">
              {t('trafstat.lastHit')}{lastProvider ?? '—'}
              {lastPath ? ` · ${lastPath}` : ''}
            </p>
          )}
          <TrafficChart buckets={traffic.hourly_buckets} />
          <div className="traffic-clients">
            {traffic.recent_clients.length > 0 && (
              <button
                type="button"
                className="ghost tiny-btn traffic-clients-toggle"
                onClick={() => setClientsOpen(true)}
              >
                {t('trafstat.sourceTitle', { n: traffic.recent_clients.length })}
              </button>
            )}
            {channels.length > 0 && (
              <button
                type="button"
                className="ghost tiny-btn traffic-clients-toggle"
                onClick={() => setChannelsOpen(true)}
              >
                {t('trafstat.channelTitle', { n: channels.length })}
              </button>
            )}
          </div>
        </>
      )}

      {clientsOpen && (
        <ClientsDetailModal
          clients={traffic.recent_clients}
          onClose={() => setClientsOpen(false)}
        />
      )}
      {channelsOpen && (
        <ChannelsDetailModal
          channels={channels}
          onClose={() => setChannelsOpen(false)}
        />
      )}
    </div>
  );
}

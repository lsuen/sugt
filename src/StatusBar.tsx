import React, { useEffect } from 'react';
import { FileText, X } from 'lucide-react';
import { ModalPortal } from './ModalPortal';
import { LogViewer } from './LogViewer';
import { t } from './i18n';
import type { TrafficStats } from './providerPresets';

type Props = {
  running?: boolean;
  listenUrl?: string;
  activeProvider?: string | null;
  traffic?: TrafficStats;
  logs: string[];
  drawerOpen: boolean;
  onToggleDrawer: () => void;
  onOpenLogsTab: () => void;
  onRefreshLogs: () => void;
};

export function StatusBar({
  running,
  listenUrl,
  activeProvider,
  traffic,
  logs,
  drawerOpen,
  onToggleDrawer,
  onOpenLogsTab,
  onRefreshLogs,
}: Props) {
  useEffect(() => {
    if (!drawerOpen) return;
    onRefreshLogs();
    const timer = window.setInterval(() => onRefreshLogs(), 2000);
    return () => window.clearInterval(timer);
  }, [drawerOpen, onRefreshLogs]);

  useEffect(() => {
    if (!drawerOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onToggleDrawer();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [drawerOpen, onToggleDrawer]);

  const today = traffic?.today_requests ?? 0;
  const failRate = traffic?.fail_rate_percent ?? 0;

  return (
    <>
      <footer className="status-bar" role="status">
        <div className="status-bar-main">
          <span className={`status-dot${running ? ' is-on' : ''}`} aria-hidden />
          <span className="status-bar-item">
            {running ? t('common.running') : t('common.stopped')}
          </span>
          {listenUrl ? (
            <span className="status-bar-item status-bar-muted" title={listenUrl}>
              {listenUrl.replace(/^https?:\/\//, '')}
            </span>
          ) : null}
          {activeProvider ? (
            <span className="status-bar-item status-bar-muted" title={activeProvider}>
              {t('statusBar.upstream', { name: activeProvider })}
            </span>
          ) : null}
          <span className="status-bar-item">
            {t('statusBar.today', { n: today })}
            {failRate > 0 ? (
              <span className="status-bar-fail"> · {t('statusBar.failRate', { p: failRate })}</span>
            ) : null}
          </span>
        </div>
        <div className="status-bar-actions">
          <button
            type="button"
            className={`ghost tiny-btn status-bar-logs${drawerOpen ? ' active' : ''}`}
            onClick={onToggleDrawer}
            aria-expanded={drawerOpen}
            aria-controls="status-log-drawer"
          >
            <FileText size={14} />
            {t('statusBar.logs')}
          </button>
        </div>
      </footer>

      {drawerOpen ? (
        <ModalPortal>
          <div
            className="status-log-scrim"
            onClick={onToggleDrawer}
            aria-hidden
          />
          <div
            id="status-log-drawer"
            className="status-log-drawer"
            role="dialog"
            aria-label={t('logs.title')}
            onClick={(e) => e.stopPropagation()}
          >
            <div className="status-log-head">
              <h3>{t('logs.title')}</h3>
              <div className="status-log-head-actions">
                <button type="button" className="ghost tiny-btn" onClick={onOpenLogsTab}>
                  {t('statusBar.openLogsTab')}
                </button>
                <button
                  type="button"
                  className="icon-btn"
                  onClick={onToggleDrawer}
                  aria-label={t('common.close')}
                >
                  <X size={16} />
                </button>
              </div>
            </div>
            <div className="status-log-body">
              <LogViewer lines={logs} compact />
            </div>
          </div>
        </ModalPortal>
      ) : null}
    </>
  );
}

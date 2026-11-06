// 作者：sun / 孙文龙
// 作用：设置-日志子页。复用状态栏抽屉同一份日志数据与 LogViewer，避免两套渲染。
import React from 'react';
import { RefreshCw } from 'lucide-react';
import { LogViewer } from './LogViewer';
import { t } from './i18n';

type Props = {
  lines: string[];
  busy: boolean;
  onRefresh: () => void;
};

export function LogsSection({ lines, busy, onRefresh }: Props) {
  return (
    <section className="card log-card-full">
      <div className="section-title">
        <h3>{t('logs.title')}</h3>
        <button type="button" className="ghost tiny-btn" disabled={busy} onClick={onRefresh}>
          <RefreshCw size={14} />{t('common.refresh')}
        </button>
      </div>
      <LogViewer lines={lines} />
    </section>
  );
}

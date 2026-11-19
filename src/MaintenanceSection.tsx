/** 设置 → 通用 → 维护（PRD v1.1.11 R7）。
 * 作用：清理技能仓下载缓存 / 模型探测结果，并配置日志自动清理周期。
 * 只调用维护命令，不改服务商、主题、桌宠等配置；删除类操作统一走 ConfirmModal。 */
import React, { useEffect, useRef, useState } from 'react';
import { invoke } from './transport';
import { ConfirmModal } from './ConfirmModal';
import { t } from './i18n';

type CacheReport = { items: number; bytes: number };
type MaintenanceSettings = { logRetentionDays: number };

const DEFAULT_RETENTION_DAYS = 7;
const MAX_RETENTION_DAYS = 90;

type Props = {
  config: { log_retention_days?: number } | null;
  pushToast: (message: string, type: 'ok' | 'error' | 'info') => void;
};

function clampDays(value: number): number {
  if (!Number.isFinite(value)) return DEFAULT_RETENTION_DAYS;
  return Math.min(MAX_RETENTION_DAYS, Math.max(0, Math.round(value)));
}

function formatCacheSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 KB';
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function MaintenanceSection({ config, pushToast }: Props) {
  const initialDays = clampDays(config?.log_retention_days ?? DEFAULT_RETENTION_DAYS);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [clearing, setClearing] = useState(false);
  const [daysDraft, setDaysDraft] = useState(String(initialDays));
  const [savingDays, setSavingDays] = useState(false);
  // 用户碰过输入框后，命令返回的兜底值不再覆盖草稿。
  const touched = useRef(false);
  const savedDays = useRef(initialDays);

  // 挂载时兜底刷新一次；失败静默沿用 config 的值，不打扰用户。
  useEffect(() => {
    let cancelled = false;
    void invoke<MaintenanceSettings>('get_maintenance_settings')
      .then((value) => {
        if (cancelled || touched.current) return;
        const next = clampDays(
          typeof value?.logRetentionDays === 'number' ? value.logRetentionDays : savedDays.current,
        );
        savedDays.current = next;
        setDaysDraft(String(next));
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);

  const commitDays = async (force = false) => {
    if (savingDays) return;
    const raw = daysDraft.trim();
    if (raw === '') {
      setDaysDraft(String(savedDays.current));
      return;
    }
    const parsed = Number(raw);
    const clamped = clampDays(Number.isFinite(parsed) ? parsed : savedDays.current);
    // 越界先在前端夹紧并提示，不把非法值发给后端。
    if (Number.isFinite(parsed) && clamped !== parsed) {
      pushToast(t('set.logRetentionClamped', { days: clamped }), 'info');
    }
    setDaysDraft(String(clamped));
    // 失焦且值没变时不重复打 IPC；显式点保存则始终写入。
    if (!force && clamped === savedDays.current) return;
    setSavingDays(true);
    try {
      const actual = await invoke<number>('set_maintenance_settings', {
        input: { log_retention_days: clamped },
      });
      // 以命令返回的实际值为准（后端可能再夹紧一次）。
      const next = clampDays(typeof actual === 'number' ? actual : clamped);
      savedDays.current = next;
      setDaysDraft(String(next));
      pushToast(t('set.logRetentionSaved'), 'ok');
    } catch (error) {
      pushToast(errorText(error), 'error');
      setDaysDraft(String(savedDays.current));
    } finally {
      setSavingDays(false);
    }
  };

  const clearCache = async () => {
    setConfirmOpen(false);
    setClearing(true);
    try {
      const report = await invoke<CacheReport>('clear_cache');
      const items = Number(report?.items ?? 0) || 0;
      const bytes = Number(report?.bytes ?? 0) || 0;
      pushToast(t('set.clearCacheDone', { items, size: formatCacheSize(bytes) }), 'ok');
    } catch (error) {
      pushToast(errorText(error), 'error');
    } finally {
      setClearing(false);
    }
  };

  return (
    <div className="card settings-group switches">
      <h4>{t('set.maintenance')}</h4>
      <p className="hint compact">{t('set.maintenanceHint')}</p>

      <div className="switch-line maintenance-row">
        <span>{t('set.clearCache')}</span>
        <button
          type="button"
          className="ghost tiny-btn maintenance-action"
          disabled={clearing}
          onClick={() => setConfirmOpen(true)}
        >
          {clearing ? t('set.clearing') : t('set.clearCache')}
        </button>
      </div>

      <div className="switch-line maintenance-row">
        <span>{t('set.logRetention')}</span>
        <span className="maintenance-days">
          <input
            type="number"
            min={0}
            max={MAX_RETENTION_DAYS}
            step={1}
            value={daysDraft}
            disabled={savingDays}
            aria-label={t('set.logRetention')}
            onChange={(event) => {
              touched.current = true;
              setDaysDraft(event.target.value);
            }}
            onBlur={() => void commitDays()}
          />
          <button
            type="button"
            className="ghost tiny-btn"
            disabled={savingDays}
            onClick={() => void commitDays(true)}
          >
            {t('set.logRetentionSave')}
          </button>
        </span>
      </div>
      <p className="hint compact maintenance-hint">{t('set.logRetentionHint')}</p>

      <ConfirmModal
        open={confirmOpen}
        title={t('set.clearCacheConfirmTitle')}
        body={t('set.clearCacheConfirmBody')}
        danger
        busy={clearing}
        confirmLabel={t('set.clearCache')}
        onConfirm={() => void clearCache()}
        onCancel={() => setConfirmOpen(false)}
      />
    </div>
  );
}

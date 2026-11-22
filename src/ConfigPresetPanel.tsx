/**
 * 用途：设置页「配置预设」子页——内置三档 + 用户自定义预设的应用 / 保存 / 覆盖 / 导入 / 导出 / 删除。
 * 适用项目：SUGT（sugt-desktop-pet）。
 * 创建日期：2026-11-22。
 *
 * 数据来源约束：预设列表全部来自后端 list_config_presets，不在前端写死任何模拟预设；
 * 命令签名严格按后端契约（camelCase 参数）调用：list/apply/save/delete/export/import_config_preset。
 * 配置刷新交给上层 run()：它内部 refresh() 重新拉取 get_config，因此本组件不自建轮询。
 */
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Download, Loader2, Pencil, RefreshCw, Trash2 } from 'lucide-react';
import { invoke } from './transport';
import { ConfirmModal } from './ConfirmModal';
import { ModalPortal } from './ModalPortal';
import { t, type TKey } from './i18n';

export type PresetSettings = {
  activeProviderId: string | null;
  failoverEnabled: boolean;
  upstreamRetryBudgetMs: number | null;
  upstreamRetrySingleMs: number | null;
  experimentalFreeIdleProbeEnabled: boolean;
  experimentalFreeAutoRotate: boolean;
};

export type ConfigPreset = {
  id: string;
  name: string;
  builtin: boolean;
  settings: PresetSettings;
  updatedAt: string;
};

/** get_config 返回的相关切片（snake_case），用于计算「当前值」基线。 */
type CurrentConfigSlice = {
  active_provider_id?: string | null;
  failover_enabled?: boolean;
  upstream_retry_budget_ms?: number | null;
  upstream_retry_single_ms?: number | null;
  experimental_free_idle_probe_enabled?: boolean;
  experimental_free_auto_rotate?: boolean;
};

export type PresetDiffRow = {
  key: keyof PresetSettings;
  label: string;
  current: string;
  target: string;
};

type Props = {
  /** 上层统一 run()：负责 busy、成功 toast、错误通知与配置刷新。 */
  run: (fn: () => Promise<unknown>, ok?: string) => void | Promise<void>;
  pushToast: (msg: string, type?: 'ok' | 'error' | 'info') => void;
  formatError?: (error: unknown) => string;
  /** 网关是否在跑：后果预览里提示用户是否会影响正在运行的网关。 */
  gatewayRunning?: boolean;
};

const boolText = (value: boolean) => t(value ? 'set.presetsValueOn' : 'set.presetsValueOff');
const msText = (value: number | null) =>
  value == null ? t('set.presetsValueDefault') : t('set.presetsMs', { ms: value });

/** 字段展示定义既是摘要的数据源，也是 diff 的唯一真源：避免两处格式化逻辑漂移。 */
const PRESET_FIELDS: {
  key: keyof PresetSettings;
  labelKey: TKey;
  read: (settings: PresetSettings) => string;
  /** 后端语义：activeProviderId=null 表示应用时不动当前上游，因此不算字段变更。 */
  changes?: (current: PresetSettings, target: PresetSettings) => boolean;
}[] = [
  {
    key: 'activeProviderId',
    labelKey: 'set.presetsFieldActiveProvider',
    read: (s) => s.activeProviderId?.trim() || t('set.presetsValueKeep'),
    changes: (c, target) =>
      Boolean(target.activeProviderId?.trim()) && c.activeProviderId !== target.activeProviderId,
  },
  { key: 'failoverEnabled', labelKey: 'set.presetsFieldFailover', read: (s) => boolText(s.failoverEnabled) },
  {
    key: 'upstreamRetryBudgetMs',
    labelKey: 'set.presetsFieldRetryBudget',
    read: (s) => msText(s.upstreamRetryBudgetMs),
  },
  {
    key: 'upstreamRetrySingleMs',
    labelKey: 'set.presetsFieldRetrySingle',
    read: (s) => msText(s.upstreamRetrySingleMs),
  },
  {
    key: 'experimentalFreeIdleProbeEnabled',
    labelKey: 'set.presetsFieldIdleProbe',
    read: (s) => boolText(s.experimentalFreeIdleProbeEnabled),
  },
  {
    key: 'experimentalFreeAutoRotate',
    labelKey: 'set.presetsFieldAutoRotate',
    read: (s) => boolText(s.experimentalFreeAutoRotate),
  },
];

/** 卡片 / 列表的参数摘要：只挑用户最关心的三项，避免摘要过长撑破卡片。 */
export function formatPresetSummary(settings: PresetSettings): string {
  return [
    `${t('set.presetsFieldFailover')}: ${boolText(settings.failoverEnabled)}`,
    `${t('set.presetsFieldRetryBudget')}: ${msText(settings.upstreamRetryBudgetMs)}`,
    `${t('set.presetsFieldAutoRotate')}: ${boolText(settings.experimentalFreeAutoRotate)}`,
  ].join(' · ');
}

/** 把 get_config 的 snake_case 切片映射成与预设同构的基线；缺省值按后端默认语义归一。 */
export function currentConfigToSettings(cfg: CurrentConfigSlice): PresetSettings {
  return {
    activeProviderId: cfg.active_provider_id ?? null,
    failoverEnabled: Boolean(cfg.failover_enabled),
    upstreamRetryBudgetMs: cfg.upstream_retry_budget_ms ?? null,
    upstreamRetrySingleMs: cfg.upstream_retry_single_ms ?? null,
    experimentalFreeIdleProbeEnabled: Boolean(cfg.experimental_free_idle_probe_enabled),
    experimentalFreeAutoRotate: Boolean(cfg.experimental_free_auto_rotate),
  };
}

/**
 * 字段级 diff：先格式化再比较。
 * 意图：null 与 undefined 在展示上等价（都表示「默认」），直接比原始值会产生假变更；
 * 约束：只做展示层比较，不修改任何配置，应用动作由调用方在用户确认后单独执行。
 */
export function computePresetDiff(current: PresetSettings, target: PresetSettings): PresetDiffRow[] {
  return PRESET_FIELDS
    .filter((field) =>
      field.changes
        ? field.changes(current, target)
        : field.read(current) !== field.read(target),
    )
    .map((field) => ({
      key: field.key,
      label: t(field.labelKey),
      current: field.read(current),
      target: field.read(target),
    }));
}

/** 后果预览 / 覆盖弹窗共用的 Modal 外壳：走项目 ModalPortal，带 Esc、遮罩与焦点回归。 */
function PresetModalShell({
  open,
  label,
  onCancel,
  children,
  footer,
}: {
  open: boolean;
  label: string;
  onCancel: () => void;
  children: React.ReactNode;
  footer: React.ReactNode;
}) {
  const dialogRef = useRef<HTMLDivElement>(null);

  // 焦点回归：打开时记住触发元素并移入弹窗第一个可聚焦控件，关闭时还回去。
  useEffect(() => {
    if (!open) return;
    const previous = document.activeElement as HTMLElement | null;
    const first = dialogRef.current?.querySelector<HTMLElement>(
      'button.primary, button.danger, input, textarea, button',
    );
    first?.focus();
    return () => previous?.focus?.();
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onCancel();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open, onCancel]);

  if (!open) return null;

  return (
    <ModalPortal>
      <div
        className="modal-overlay"
        onMouseDown={(event) => {
          if (event.target === event.currentTarget) onCancel();
        }}
      >
        <div ref={dialogRef} className="modal modal-compact" role="dialog" aria-modal="true" aria-label={label}>
          <div className="modal-head"><h3>{label}</h3></div>
          <div className="modal-body">{children}</div>
          <div className="modal-actions">{footer}</div>
        </div>
      </div>
    </ModalPortal>
  );
}

function safeFileName(name: string): string {
  const cleaned = name.replace(/[\\/:*?"<>|]/g, '_').trim();
  return cleaned || 'preset';
}

function formatUpdatedAt(iso: string): string {
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? iso : date.toLocaleString();
}

export function ConfigPresetPanel({ run, pushToast, formatError, gatewayRunning }: Props) {
  // 保持函数身份稳定：reload 被 useCallback 依赖，否则每次渲染都会重新触发列表请求。
  const formatErr = useCallback(
    (error: unknown) => (formatError ? formatError(error) : String(error)),
    [formatError],
  );
  const [presets, setPresets] = useState<ConfigPreset[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [nameDraft, setNameDraft] = useState('');
  const [importDraft, setImportDraft] = useState('');
  const [busyId, setBusyId] = useState<string | null>(null);
  const [previewTarget, setPreviewTarget] = useState<ConfigPreset | null>(null);
  const [previewRows, setPreviewRows] = useState<PresetDiffRow[] | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<ConfigPreset | null>(null);
  const [overwriteTarget, setOverwriteTarget] = useState<ConfigPreset | null>(null);
  const [overwriteName, setOverwriteName] = useState('');
  const fileRef = useRef<HTMLInputElement>(null);

  const reload = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const list = await invoke<ConfigPreset[]>('list_config_presets');
      setPresets(Array.isArray(list) ? list : []);
    } catch (error) {
      // 错误不吞：错误条里带上 formatInvokeError 的可定位信息 + 重试按钮。
      setLoadError(t('set.presetsLoadFailed', { error: formatErr(error) }));
    } finally {
      setLoading(false);
    }
  }, [formatErr]);

  useEffect(() => {
    void reload();
  }, [reload]);

  const builtins = useMemo(() => presets.filter((p) => p.builtin), [presets]);
  const customs = useMemo(() => presets.filter((p) => !p.builtin), [presets]);

  const closePreview = () => {
    setPreviewTarget(null);
    setPreviewRows(null);
    setPreviewError(null);
  };

  /** 应用前后果预览：现场拉 get_config 作为基线，计算字段 diff 后才弹确认框，绝不静默切换。 */
  const openPreview = async (preset: ConfigPreset) => {
    setPreviewTarget(preset);
    setPreviewRows(null);
    setPreviewError(null);
    try {
      const cfg = await invoke<CurrentConfigSlice>('get_config');
      setPreviewRows(computePresetDiff(currentConfigToSettings(cfg), preset.settings));
    } catch (error) {
      setPreviewRows([]);
      setPreviewError(t('set.presetsLoadFailed', { error: formatErr(error) }));
    }
  };

  const confirmApply = () => {
    const preset = previewTarget;
    if (!preset || previewError) return;
    closePreview();
    // run() 成功 toast 后会 refresh()：父级 config 重新拉取，无需本组件自建刷新。
    run(() => invoke('apply_config_preset', { id: preset.id }), t('set.presetsApplied', { name: preset.name }));
  };

  const saveCurrent = () => {
    const name = nameDraft.trim();
    if (!name) {
      pushToast(t('set.presetsNameRequired'), 'error');
      return;
    }
    run(async () => {
      await invoke('save_config_preset', { name });
      setNameDraft('');
      await reload();
    }, t('set.presetsSaved', { name }));
  };

  /**
   * 覆盖：后端 save_config_preset 对同名自定义预设做 upsert（保留原 id、
   * 刷新 settings/updatedAt），所以一次调用即可；改名则另存为新预设。
   * 内置档同名会被后端拒绝，但覆盖按钮只对自定义预设渲染。
   */
  const confirmOverwrite = () => {
    const name = overwriteName.trim();
    if (!overwriteTarget) return;
    if (!name) {
      pushToast(t('set.presetsNameRequired'), 'error');
      return;
    }
    setOverwriteTarget(null);
    run(async () => {
      await invoke('save_config_preset', { name });
      await reload();
    }, t('set.presetsSaved', { name }));
  };

  const confirmDelete = () => {
    const preset = deleteTarget;
    if (!preset) return;
    setDeleteTarget(null);
    run(async () => {
      await invoke('delete_config_preset', { id: preset.id });
      await reload();
    }, t('set.presetsDeleted', { name: preset.name }));
  };

  const doImport = () => {
    const json = importDraft.trim();
    if (!json) {
      pushToast(t('set.presetsImportInvalid'), 'error');
      return;
    }
    run(async () => {
      const created = await invoke<ConfigPreset>('import_config_preset', { json });
      setImportDraft('');
      await reload();
      pushToast(t('set.presetsImported', { name: created?.name ?? '' }), 'ok');
    });
  };

  const onFile = async (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    event.target.value = '';
    if (!file) return;
    try {
      setImportDraft(await file.text());
    } catch (error) {
      pushToast(t('set.presetsFileReadFailed', { error: formatErr(error) }), 'error');
    }
  };

  const doExport = async (preset: ConfigPreset) => {
    setBusyId(preset.id);
    try {
      const json = await invoke<string>('export_config_preset', { id: preset.id });
      const url = URL.createObjectURL(new Blob([json], { type: 'application/json;charset=utf-8' }));
      const anchor = document.createElement('a');
      anchor.href = url;
      anchor.download = `${safeFileName(preset.name)}.json`;
      document.body.appendChild(anchor);
      anchor.click();
      anchor.remove();
      URL.revokeObjectURL(url);
      pushToast(t('set.presetsExportDone', { name: preset.name }), 'ok');
    } catch (error) {
      pushToast(t('set.presetsExportFailed', { error: formatErr(error) }), 'error');
    } finally {
      setBusyId(null);
    }
  };

  const cardGridStyle: React.CSSProperties = {
    display: 'grid',
    gridTemplateColumns: 'repeat(auto-fit, minmax(240px, 1fr))',
    gap: 14,
  };
  const diffGridStyle: React.CSSProperties = {
    display: 'grid',
    gridTemplateColumns: '1.1fr 1fr 1fr',
    gap: '6px 10px',
    fontSize: 12,
    overflowWrap: 'anywhere',
  };

  return (
    <div className="settings-layout settings-layout--stack">
      <div className="card settings-group settings-group-wide">
        <h4>{t('set.presetsTitle')}</h4>
        <p className="hint compact">{t('set.presetsHint')}</p>
      </div>

      {loading ? (
        // 三态之一：加载骨架。只复用现有卡片 / hint 类 + .spin，不写死颜色（主题 token 约束）。
        <div className="settings-group-wide" style={cardGridStyle} aria-busy="true">
          {[0, 1, 2].map((i) => (
            <div className="card settings-group" key={i} aria-hidden="true">
              <div className="hint compact">{t('common.loading')}…</div>
              <div className="hint compact">— — — — — —</div>
              <Loader2 size={14} className="spin" />
            </div>
          ))}
        </div>
      ) : loadError ? (
        // 三态之二：错误条 + 可重试，错误信息带可定位细节。
        <div className="card settings-group settings-group-wide">
          <p className="hint compact">{loadError}</p>
          <div className="settings-actions-row">
            <button type="button" className="ghost tiny-btn" onClick={() => void reload()}>
              <RefreshCw size={14} />{t('set.presetsRetry')}
            </button>
          </div>
        </div>
      ) : (
        <>
          {builtins.length > 0 && (
            <>
              <h4 className="settings-group-wide">{t('set.presetsBuiltinTitle')}</h4>
              <p className="hint compact settings-group-wide">{t('set.presetsBuiltinHint')}</p>
              <div className="settings-group-wide" style={cardGridStyle}>
                {builtins.map((preset) => (
                  <div className="card settings-group" key={preset.id}>
                    <h4>
                      {preset.name}{' '}
                      <span className="badge">{t('set.presetsBuiltinBadge')}</span>
                    </h4>
                    <p className="hint compact">{formatPresetSummary(preset.settings)}</p>
                    <div className="settings-actions-row">
                      <button
                        type="button"
                        className="primary tiny-btn"
                        onClick={() => void openPreview(preset)}
                      >
                        {t('set.presetsApply')}
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            </>
          )}

          <div className="card settings-group settings-group-wide">
            <h4>{t('set.presetsSaveTitle')}</h4>
            <p className="hint compact">{t('set.presetsSaveHint')}</p>
            <div className="settings-actions-row">
              <input
                className="field-input"
                value={nameDraft}
                placeholder={t('set.presetsNamePlaceholder')}
                aria-label={t('set.presetsNamePlaceholder')}
                onChange={(e) => setNameDraft(e.target.value)}
              />
              <button type="button" className="primary tiny-btn" onClick={saveCurrent}>
                {t('set.presetsSaveBtn')}
              </button>
            </div>
          </div>

          <div className="card settings-group settings-group-wide">
            <h4>{t('set.presetsImportTitle')}</h4>
            <p className="hint compact">{t('set.presetsImportHint')}</p>
            <textarea
              className="field-textarea"
              value={importDraft}
              placeholder={t('set.presetsImportPlaceholder')}
              aria-label={t('set.presetsImportTitle')}
              rows={3}
              onChange={(e) => setImportDraft(e.target.value)}
            />
            <div className="settings-actions-row">
              <input
                ref={fileRef}
                type="file"
                accept=".json,application/json"
                style={{ display: 'none' }}
                onChange={(e) => void onFile(e)}
              />
              <button type="button" className="ghost tiny-btn" onClick={() => fileRef.current?.click()}>
                {t('set.presetsChooseFile')}
              </button>
              <button type="button" className="primary tiny-btn" onClick={doImport}>
                {t('set.presetsImportBtn')}
              </button>
            </div>
          </div>

          <h4 className="settings-group-wide">{t('set.presetsCustomTitle')}</h4>
          <p className="hint compact settings-group-wide">{t('set.presetsCustomHint')}</p>
          {customs.length === 0 ? (
            // 三态之三：空态引导。
            <div className="card settings-group settings-group-wide">
              <p className="hint compact">{t('set.presetsEmpty')}</p>
              <p className="hint compact">{t('set.presetsEmptyHint')}</p>
            </div>
          ) : (
            customs.map((preset) => (
              <div className="card settings-group settings-group-wide" key={preset.id}>
                <h4>{preset.name}</h4>
                <p className="hint compact">
                  {t('set.presetsUpdatedAt', { time: formatUpdatedAt(preset.updatedAt) })}
                </p>
                <p className="hint compact">{formatPresetSummary(preset.settings)}</p>
                <div className="settings-actions-row">
                  <button type="button" className="primary tiny-btn" onClick={() => void openPreview(preset)}>
                    {t('set.presetsApply')}
                  </button>
                  <button
                    type="button"
                    className="ghost tiny-btn"
                    onClick={() => {
                      setOverwriteTarget(preset);
                      setOverwriteName(preset.name);
                    }}
                  >
                    <Pencil size={14} />{t('set.presetsOverwrite')}
                  </button>
                  <button
                    type="button"
                    className="ghost tiny-btn"
                    disabled={busyId === preset.id}
                    onClick={() => void doExport(preset)}
                  >
                    {busyId === preset.id ? <Loader2 size={14} className="spin" /> : <Download size={14} />}
                    {t('set.presetsExport')}
                  </button>
                  <button type="button" className="danger tiny-btn" onClick={() => setDeleteTarget(preset)}>
                    <Trash2 size={14} />{t('set.presetsDelete')}
                  </button>
                </div>
              </div>
            ))
          )}
        </>
      )}

      {/* 后果预览：应用前必须确认；不嵌套任何弹窗，错误时禁用确认。 */}
      <PresetModalShell
        open={Boolean(previewTarget)}
        label={t('set.presetsPreviewTitle')}
        onCancel={closePreview}
        footer={
          <>
            <button type="button" className="ghost" onClick={closePreview}>{t('common.cancel')}</button>
            <button type="button" className="primary" disabled={Boolean(previewError)} onClick={confirmApply}>
              {t('set.presetsApplyConfirm')}
            </button>
          </>
        }
      >
        {previewTarget ? (
          <>
            <p className="hint compact">
              {t('set.presetsPreviewIntro', { name: previewTarget.name })}
            </p>
            {previewError ? <p className="agent-discover-warn">{previewError}</p> : null}
            {previewRows === null && !previewError ? (
              <p className="hint compact"><Loader2 size={14} className="spin" />{t('common.loading')}</p>
            ) : null}
            {previewRows && previewRows.length > 0 ? (
              <div style={diffGridStyle}>
                <strong>{t('set.presetsPreviewField')}</strong>
                <strong>{t('set.presetsPreviewCurrent')}</strong>
                <strong>{t('set.presetsPreviewTarget')}</strong>
                {previewRows.map((row) => (
                  <React.Fragment key={row.key}>
                    <span>{row.label}</span>
                    <span>{row.current}</span>
                    <span>{row.target}</span>
                  </React.Fragment>
                ))}
              </div>
            ) : null}
            {previewRows && previewRows.length === 0 && !previewError ? (
              <p className="hint compact">{t('set.presetsPreviewNoChange')}</p>
            ) : null}
            <p className="hint compact">
              {gatewayRunning ? t('set.presetsPreviewGatewayOn') : t('set.presetsPreviewGatewayOff')}
            </p>
          </>
        ) : null}
      </PresetModalShell>

      {/* 删除：走项目 ConfirmModal（危险样式 + 后果说明 + Esc / 遮罩 / 焦点回归）。 */}
      <ConfirmModal
        open={Boolean(deleteTarget)}
        danger
        title={t('set.presetsDeleteTitle')}
        body={t('set.presetsDeleteBody', { name: deleteTarget?.name ?? '' })}
        confirmLabel={t('set.presetsDeleteConfirm')}
        onConfirm={confirmDelete}
        onCancel={() => setDeleteTarget(null)}
      />

      {/* 覆盖：只承载改名 / 确认，应用动作仍需用户显式点击。 */}
      <PresetModalShell
        open={Boolean(overwriteTarget)}
        label={t('set.presetsOverwriteTitle')}
        onCancel={() => setOverwriteTarget(null)}
        footer={
          <>
            <button type="button" className="ghost" onClick={() => setOverwriteTarget(null)}>
              {t('common.cancel')}
            </button>
            <button type="button" className="primary" onClick={confirmOverwrite}>
              {t('set.presetsOverwriteConfirm')}
            </button>
          </>
        }
      >
        <p className="hint compact">
          {t('set.presetsOverwriteBody', { name: overwriteTarget?.name ?? '' })}
        </p>
        <input
          className="field-input"
          value={overwriteName}
          aria-label={t('set.presetsSaveTitle')}
          onChange={(e) => setOverwriteName(e.target.value)}
        />
      </PresetModalShell>
    </div>
  );
}

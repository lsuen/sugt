import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { t } from './i18n';

export type WorkspaceRulesView = {
  enabled: boolean;
  rules_text: string;
  map_doc_enabled: boolean;
  map_doc_path: string;
  dingtalk_enabled: boolean;
  dingtalk_webhook: string;
};

const DEFAULT_RULES: WorkspaceRulesView = {
  enabled: false,
  rules_text: '',
  map_doc_enabled: false,
  map_doc_path: 'docs/MAP.md',
  dingtalk_enabled: false,
  dingtalk_webhook: '',
};

type Props = {
  rules?: WorkspaceRulesView | null;
  busy: boolean;
  run: (fn: () => Promise<unknown>, ok?: string) => void;
  onSaved?: (workspaceRules: WorkspaceRulesView) => void;
};

export function WorkspaceRulesPanel({ rules, busy, run, onSaved }: Props) {
  const [draft, setDraft] = useState<WorkspaceRulesView>(rules ?? DEFAULT_RULES);
  const [dirty, setDirty] = useState(false);

  useEffect(() => {
    if (!dirty) {
      setDraft(rules ?? DEFAULT_RULES);
    }
  }, [rules, dirty]);

  const patch = (partial: Partial<WorkspaceRulesView>) => {
    setDraft((prev) => ({ ...prev, ...partial }));
    setDirty(true);
  };

  const save = () => {
    run(async () => {
      const next = await invoke<{ workspace_rules: WorkspaceRulesView }>('set_workspace_rules', {
        rules: draft,
      });
      setDirty(false);
      setDraft(next.workspace_rules ?? draft);
      onSaved?.(next.workspace_rules ?? draft);
    }, t('rules.toastSaved'));
  };

  const testDing = () => {
    run(() => invoke('test_workspace_dingtalk'), t('rules.toastDingOk'));
  };

  return (
    <div className="settings-layout rules-layout">
      <div className="card settings-group switches settings-group-wide">
        <h4>{t('rules.master')}</h4>
        <p className="hint compact">{t('rules.masterHint')}</p>
        <label className="switch-line">
          <span>{t('rules.enabled')}</span>
          <input
            type="checkbox"
            checked={draft.enabled}
            disabled={busy}
            onChange={(e) => patch({ enabled: e.target.checked })}
          />
        </label>
      </div>

      <div className="card settings-group settings-group-wide">
        <h4>{t('rules.textTitle')}</h4>
        <p className="hint compact">{t('rules.textHint')}</p>
        <textarea
          className="rules-textarea"
          rows={8}
          disabled={busy || !draft.enabled}
          value={draft.rules_text}
          placeholder={t('rules.textPlaceholder')}
          onChange={(e) => patch({ rules_text: e.target.value })}
        />
      </div>

      <div className="card settings-group switches">
        <h4>{t('rules.mapTitle')}</h4>
        <p className="hint compact">{t('rules.mapHint')}</p>
        <label className="switch-line">
          <span>{t('rules.mapEnable')}</span>
          <input
            type="checkbox"
            checked={draft.map_doc_enabled}
            disabled={busy || !draft.enabled}
            onChange={(e) => patch({ map_doc_enabled: e.target.checked })}
          />
        </label>
        <label className="form-field">
          <span>{t('rules.mapPath')}</span>
          <input
            type="text"
            disabled={busy || !draft.enabled || !draft.map_doc_enabled}
            value={draft.map_doc_path}
            placeholder="docs/MAP.md"
            onChange={(e) => patch({ map_doc_path: e.target.value })}
          />
        </label>
      </div>

      <div className="card settings-group switches">
        <h4>{t('rules.dingTitle')}</h4>
        <p className="hint compact">{t('rules.dingHint')}</p>
        <label className="switch-line">
          <span>{t('rules.dingEnable')}</span>
          <input
            type="checkbox"
            checked={draft.dingtalk_enabled}
            disabled={busy || !draft.enabled}
            onChange={(e) => patch({ dingtalk_enabled: e.target.checked })}
          />
        </label>
        <label className="form-field">
          <span>{t('rules.dingWebhook')}</span>
          <input
            type="password"
            autoComplete="off"
            disabled={busy || !draft.enabled || !draft.dingtalk_enabled}
            value={draft.dingtalk_webhook}
            placeholder="https://oapi.dingtalk.com/robot/send?access_token=..."
            onChange={(e) => patch({ dingtalk_webhook: e.target.value })}
          />
        </label>
        <div className="form-grid-actions" style={{ marginTop: 10 }}>
          <button
            type="button"
            className="ghost tiny-btn"
            disabled={
              busy || !draft.enabled || !draft.dingtalk_enabled || !draft.dingtalk_webhook.trim()
            }
            onClick={testDing}
          >
            {t('rules.dingTest')}
          </button>
        </div>
      </div>

      <div className="card settings-group settings-group-wide rules-save-bar">
        <p className="hint compact">{dirty ? t('rules.dirty') : t('rules.clean')}</p>
        <button type="button" className="primary" disabled={busy || !dirty} onClick={save}>
          {t('common.save')}
        </button>
      </div>
    </div>
  );
}

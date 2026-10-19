import React, { useEffect, useId, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Plus, Trash2, X } from 'lucide-react';
import { t } from './i18n';

export type DocHabitView = {
  enabled: boolean;
  path: string;
  note: string;
};

export type WorkspaceRulesView = {
  enabled: boolean;
  rules_text: string;
  doc_habits: DocHabitView[];
  docs_enabled: boolean;
  notify_enabled: boolean;
  notify_webhook: string;
  inject_template?: string;
};

type HabitRow = DocHabitView & { key: string };

const DEFAULT_HABITS: DocHabitView[] = [
  { enabled: true, path: 'AGENTS.md', note: '' },
  { enabled: true, path: 'docs/MAP.md', note: '' },
];

const DEFAULT_RULES_TEXT = `用用户本次消息的语言回复。
只做当前要求的事；要扩大范围时先说明。
先查已有材料再动手。不知道就说明不知道，不要编造文件、接口、数据或已经完成的结果。
声称完成前先核对自己做过的改动；核不出就标明未核实。
未经明确要求，不要提交、推送、删除，也不要向外发送消息。
`;

const DEFAULT_RULES: WorkspaceRulesView = {
  enabled: false,
  rules_text: DEFAULT_RULES_TEXT,
  doc_habits: DEFAULT_HABITS,
  docs_enabled: true,
  notify_enabled: false,
  notify_webhook: '',
};

let habitKeySeq = 0;
function nextHabitKey(prefix: string) {
  habitKeySeq += 1;
  return `${prefix}-${habitKeySeq}`;
}

function toRows(habits: DocHabitView[], prefix: string): HabitRow[] {
  return habits.map((h) => ({
    key: nextHabitKey(prefix),
    enabled: Boolean(h.enabled),
    path: h.path ?? '',
    note: h.note ?? '',
  }));
}

function normalizeBase(rules?: WorkspaceRulesView | null): WorkspaceRulesView {
  if (!rules) return DEFAULT_RULES;
  const habits =
    Array.isArray(rules.doc_habits) && rules.doc_habits.length > 0
      ? rules.doc_habits.map((h) => ({
          enabled: Boolean(h.enabled),
          path: h.path ?? '',
          note: h.note ?? '',
        }))
      : DEFAULT_HABITS;
  return {
    enabled: Boolean(rules.enabled),
    rules_text: rules.rules_text ?? '',
    doc_habits: habits,
    docs_enabled: rules.docs_enabled !== false,
    notify_enabled: Boolean(rules.notify_enabled),
    notify_webhook: rules.notify_webhook ?? '',
    inject_template: rules.inject_template ?? '',
  };
}

function habitsEqual(a: DocHabitView[], b: DocHabitView[]) {
  if (a.length !== b.length) return false;
  return a.every(
    (h, i) => h.enabled === b[i].enabled && h.path === b[i].path && h.note === b[i].note,
  );
}

function rulesEqual(a: WorkspaceRulesView, b: WorkspaceRulesView) {
  return (
    a.enabled === b.enabled &&
    a.rules_text === b.rules_text &&
    a.docs_enabled === b.docs_enabled &&
    a.notify_enabled === b.notify_enabled &&
    a.notify_webhook === b.notify_webhook &&
    (a.inject_template ?? '') === (b.inject_template ?? '') &&
    habitsEqual(a.doc_habits, b.doc_habits)
  );
}

type Props = {
  rules?: WorkspaceRulesView | null;
  busy: boolean;
  run: (fn: () => Promise<unknown>, ok?: string) => void;
  onSaved?: (workspaceRules: WorkspaceRulesView) => void;
};

export function WorkspaceRulesPanel({ rules, busy, run, onSaved }: Props) {
  const idPrefix = useId();
  const [enabled, setEnabled] = useState(false);
  const [rulesText, setRulesText] = useState('');
  const [habits, setHabits] = useState<HabitRow[]>(() => toRows(DEFAULT_HABITS, idPrefix));
  const [docsEnabled, setDocsEnabled] = useState(false);
  const [notifyEnabled, setNotifyEnabled] = useState(false);
  const [notifyWebhook, setNotifyWebhook] = useState('');
  const [injectTemplate, setInjectTemplate] = useState('');
  const [injectOpen, setInjectOpen] = useState(false);
  const [injectDraft, setInjectDraft] = useState('');
  const [dirty, setDirty] = useState(false);
  const syncedRef = useRef<string>('');

  // 仅在外部配置真正变化且本地无未保存编辑时同步，避免 3s 轮询打掉输入焦点
  useEffect(() => {
    if (dirty) return;
    const base = normalizeBase(rules);
    const sig = JSON.stringify(base);
    if (sig === syncedRef.current) return;
    syncedRef.current = sig;
    setEnabled(base.enabled);
    setRulesText(base.rules_text);
    setHabits(toRows(base.doc_habits, idPrefix));
    setDocsEnabled(base.docs_enabled);
    setNotifyEnabled(base.notify_enabled);
    setNotifyWebhook(base.notify_webhook);
    setInjectTemplate(base.inject_template ?? '');
  }, [rules, dirty, idPrefix]);

  const draftPayload = (): WorkspaceRulesView => ({
    enabled,
    rules_text: rulesText,
    doc_habits: habits.map(({ enabled: e, path, note }) => ({ enabled: e, path, note })),
    docs_enabled: docsEnabled,
    notify_enabled: notifyEnabled,
    notify_webhook: notifyWebhook,
    inject_template: injectTemplate,
  });

  const markDirty = () => setDirty(true);

  const patchHabit = (key: string, partial: Partial<DocHabitView>) => {
    setHabits((prev) => prev.map((h) => (h.key === key ? { ...h, ...partial } : h)));
    markDirty();
  };

  const addHabit = () => {
    setHabits((prev) => [
      ...prev,
      { key: nextHabitKey(idPrefix), enabled: true, path: '', note: '' },
    ]);
    markDirty();
  };

  const removeHabit = (key: string) => {
    setHabits((prev) => (prev.length <= 1 ? prev : prev.filter((h) => h.key !== key)));
    markDirty();
  };

  const save = () => {
    const payload = draftPayload();
    run(async () => {
      const next = await invoke<{ workspace_rules: WorkspaceRulesView }>('set_workspace_rules', {
        rules: payload,
      });
      const view = normalizeBase(next.workspace_rules);
      syncedRef.current = JSON.stringify(view);
      setDirty(false);
      setEnabled(view.enabled);
      setRulesText(view.rules_text);
      setHabits(toRows(view.doc_habits, idPrefix));
      setDocsEnabled(view.docs_enabled);
      setNotifyEnabled(view.notify_enabled);
      setNotifyWebhook(view.notify_webhook);
      setInjectTemplate(view.inject_template ?? '');
      onSaved?.(view);
    }, t('rules.toastSaved'));
  };

  const openInject = async () => {
    const fallback = injectTemplate.trim();
    if (fallback) {
      setInjectDraft(fallback);
      setInjectOpen(true);
      return;
    }
    try {
      const builtin = await invoke<string>('default_workspace_inject_template');
      setInjectDraft(builtin);
    } catch {
      setInjectDraft(injectTemplate);
    }
    setInjectOpen(true);
  };

  const saveInject = () => {
    run(async () => {
      const next = await invoke<{ workspace_rules: WorkspaceRulesView }>(
        'set_workspace_inject_template',
        { template: injectDraft },
      );
      const view = normalizeBase(next.workspace_rules);
      setInjectTemplate(view.inject_template ?? '');
      setInjectOpen(false);
      onSaved?.(view);
    }, t('rules.toastSaved'));
  };

  const restoreInject = () => {
    run(async () => {
      const builtin = await invoke<string>('default_workspace_inject_template');
      const next = await invoke<{ workspace_rules: WorkspaceRulesView }>(
        'set_workspace_inject_template',
        { template: builtin },
      );
      const view = normalizeBase(next.workspace_rules);
      setInjectDraft(builtin);
      setInjectTemplate(view.inject_template ?? '');
      onSaved?.(view);
    }, t('rules.toastRestored'));
  };

  const testNotify = () => {
    run(() => invoke('test_workspace_notify'), t('rules.toastNotifyOk'));
  };

  const docsOff = busy || !docsEnabled;
  const current = draftPayload();
  const external = normalizeBase(rules);
  const showDirty = dirty && !rulesEqual(current, external);

  return (
    <div className="rules-layout">
      <section className="card rules-card">
        <div className="rules-card-head">
          <h3>{t('rules.textTitle')}</h3>
          <label className="rules-switch">
            <span>{enabled ? t('common.enabled') : t('common.disabled')}</span>
            <input
              type="checkbox"
              checked={enabled}
              disabled={busy}
              onChange={(e) => {
                setEnabled(e.target.checked);
                markDirty();
              }}
            />
          </label>
        </div>
        <textarea
          className="rules-textarea"
          rows={5}
          disabled={busy || !enabled}
          value={rulesText}
          placeholder={t('rules.textPlaceholder')}
          onChange={(e) => {
            setRulesText(e.target.value);
            markDirty();
          }}
        />
      </section>

      <section className="card rules-card">
        <div className="rules-card-head">
          <h4>{t('rules.notifyTitle')}</h4>
          <label className="rules-switch">
            <input
              type="checkbox"
              checked={notifyEnabled}
              disabled={busy}
              onChange={(e) => {
                setNotifyEnabled(e.target.checked);
                markDirty();
              }}
            />
          </label>
        </div>
        <label className="rules-field">
          <span>Webhook</span>
          <div className="rules-webhook-row">
            <input
              type="password"
              autoComplete="off"
              disabled={busy || !notifyEnabled}
              value={notifyWebhook}
              placeholder={t('rules.webhookPh')}
              onChange={(e) => {
                setNotifyWebhook(e.target.value);
                markDirty();
              }}
            />
            <button
              type="button"
              className="ghost tiny-btn"
              disabled={busy || !notifyEnabled || !notifyWebhook.trim()}
              onClick={testNotify}
            >
              {t('rules.notifyTest')}
            </button>
          </div>
        </label>
      </section>

      <section className="card rules-card rules-card-habits">
        <div className="rules-card-head">
          <h4>{t('rules.docsTitle')}</h4>
          <div className="rules-card-tools">
            <button type="button" className="ghost tiny-btn" disabled={docsOff} onClick={addHabit}>
            <Plus size={14} />
            {t('rules.docsAdd')}
            </button>
            <label className="rules-switch">
              <input
                type="checkbox"
                checked={docsEnabled}
                disabled={busy}
                onChange={(e) => {
                  setDocsEnabled(e.target.checked);
                  markDirty();
                }}
              />
            </label>
          </div>
        </div>
        <div className="rules-habits">
          {habits.map((habit) => (
            <div className="rules-habit-row" key={habit.key}>
              <input
                type="checkbox"
                className="rules-habit-check"
                checked={habit.enabled}
                disabled={docsOff}
                onChange={(e) => patchHabit(habit.key, { enabled: e.target.checked })}
                title={t('common.enabled')}
              />
              <input
                type="text"
                className="rules-habit-path"
                disabled={docsOff}
                value={habit.path}
                placeholder={t('rules.docsPathPh')}
                onChange={(e) => patchHabit(habit.key, { path: e.target.value })}
              />
              <input
                type="text"
                className="rules-habit-note"
                disabled={docsOff}
                value={habit.note}
                placeholder={t('rules.docsNotePh')}
                title={t('rules.docsNotePh')}
                onChange={(e) => patchHabit(habit.key, { note: e.target.value })}
              />
              <button
                type="button"
                className="icon-btn rules-habit-del"
                disabled={docsOff || habits.length <= 1}
                onClick={() => removeHabit(habit.key)}
                aria-label={t('common.delete')}
              >
                <Trash2 size={14} />
              </button>
            </div>
          ))}
        </div>
      </section>

      <div className="rules-save-bar">
        {showDirty && <span className="rules-dirty">{t('rules.dirty')}</span>}
        <button type="button" className="ghost" disabled={busy} onClick={() => void openInject()}>
          {t('rules.injectBtn')}
        </button>
        <button type="button" className="primary" disabled={busy || !showDirty} onClick={save}>
          {t('common.save')}
        </button>
      </div>

      {injectOpen && (
        <div className="modal-overlay" onClick={() => setInjectOpen(false)}>
          <div className="modal rules-inject-modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-head">
              <h3>{t('rules.injectTitle')}</h3>
              <button type="button" className="icon-btn" onClick={() => setInjectOpen(false)} aria-label={t('common.close')}>
                <X size={18} />
              </button>
            </div>
            <div className="modal-body">
              <p className="hint compact">{t('rules.injectHint')}</p>
              <textarea
                className="rules-inject-editor"
                rows={12}
                value={injectDraft}
                onChange={(e) => setInjectDraft(e.target.value)}
              />
              <div className="modal-actions">
                <button type="button" className="ghost" disabled={busy} onClick={restoreInject}>
                  {t('rules.injectRestore')}
                </button>
                <button type="button" className="primary" disabled={busy} onClick={saveInject}>
                  {t('common.save')}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

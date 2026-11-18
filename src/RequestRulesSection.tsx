/** 「请求改写规则」区块（v1.1.11 R6），挂在「规则」主 tab 的 WorkspaceRulesPanel 之后。
 * 作用：把 Codex max_tokens / tool 重试等硬编码规则变成可编排、按作用域生效的配置。
 * 本组件只负责列表、增删改与前端基础校验；真正的改写与作用域合并由 Rust 网关完成，
 * 命令为 list_request_rules / save_request_rule / delete_request_rule。
 * 内置规则 id 以 builtin- 开头（toolTextRetry 那条默认关闭），列表加「内置」标记。
 * 删除会让规则立即失效，统一走 ConfirmModal。 */
import { useCallback, useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Pencil, Plus, RefreshCw, Trash2, X } from 'lucide-react'
import { ConfirmModal } from './ConfirmModal'
import { ModalPortal } from './ModalPortal'
import { t, type TKey } from './i18n'

type ScopeKind = 'default' | 'codex' | 'agent' | 'global'

export type RuleScope =
  | { kind: 'default' }
  | { kind: 'codex' }
  | { kind: 'agent'; label: string }
  | { kind: 'global' }

export type RuleAction =
  | { type: 'setDefaultMaxTokens'; value: number }
  | { type: 'setHeader'; key: string; value: string }
  | { type: 'setBodyField'; key: string; value: string }
  | { type: 'toolTextRetry'; extra: number }

export type RequestRule = {
  id: string
  name: string
  scope: RuleScope
  enabled: boolean
  actions: RuleAction[]
}

type Props = {
  pushToast: (msg: string, type?: 'ok' | 'error' | 'info') => void
  run: (action: () => Promise<unknown>, ok?: string) => void
}

type ActionKind = RuleAction['type']

const SCOPES: ScopeKind[] = ['default', 'codex', 'agent', 'global']
const ACTION_KINDS: ActionKind[] = [
  'setDefaultMaxTokens',
  'setHeader',
  'setBodyField',
  'toolTextRetry',
]

const SCOPE_LABEL: Record<Exclude<ScopeKind, 'agent'>, TKey> = {
  default: 'requestRules.scopeDefault',
  codex: 'requestRules.scopeCodex',
  global: 'requestRules.scopeGlobal',
}

const ACTION_LABEL: Record<ActionKind, TKey> = {
  setDefaultMaxTokens: 'requestRules.actionMaxTokens',
  setHeader: 'requestRules.actionHeader',
  setBodyField: 'requestRules.actionBodyField',
  toolTextRetry: 'requestRules.actionToolRetry',
}

/** 编辑态动作：数字与 key/value 一律先存字符串，保存时再校验并转成后端类型。 */
type DraftAction = {
  actionType: ActionKind
  key: string
  value: string
}

type Draft = {
  id: string
  name: string
  scopeKind: ScopeKind
  agentLabel: string
  enabled: boolean
  actions: DraftAction[]
}

function isBuiltin(rule: RequestRule): boolean {
  return rule.id.startsWith('builtin-')
}

function scopeText(scope: RuleScope): string {
  if (scope.kind === 'agent') {
    return t('requestRules.scopeAgentWithLabel', { label: scope.label })
  }
  return t(SCOPE_LABEL[scope.kind])
}

/** 把动作渲染成短句，列表一行能读完；不参与保存。 */
function summarizeAction(action: RuleAction): string {
  switch (action.type) {
    case 'setDefaultMaxTokens':
      return `max_tokens=${action.value}`
    case 'setHeader':
      return `header: ${action.key}=${action.value}`
    case 'setBodyField':
      return `body: ${action.key}=${action.value}`
    case 'toolTextRetry':
      return t('requestRules.summaryToolRetry', { extra: action.extra })
    default:
      return ''
  }
}

function emptyAction(kind: ActionKind = 'setDefaultMaxTokens'): DraftAction {
  return { actionType: kind, key: '', value: kind === 'setDefaultMaxTokens' ? '32768' : '' }
}

function emptyDraft(): Draft {
  return {
    id: '',
    name: '',
    scopeKind: 'default',
    agentLabel: '',
    enabled: true,
    actions: [emptyAction()],
  }
}

function toDraft(rule: RequestRule): Draft {
  return {
    id: rule.id,
    name: rule.name,
    scopeKind: rule.scope.kind,
    agentLabel: rule.scope.kind === 'agent' ? rule.scope.label : '',
    enabled: rule.enabled,
    actions: rule.actions.map((action) => {
      switch (action.type) {
        case 'setDefaultMaxTokens':
          return { actionType: action.type, key: '', value: String(action.value) }
        case 'setHeader':
        case 'setBodyField':
          return { actionType: action.type, key: action.key, value: action.value }
        case 'toolTextRetry':
          return { actionType: action.type, key: '', value: String(action.extra) }
        default:
          return { actionType: 'setDefaultMaxTokens', key: '', value: '' }
      }
    }),
  }
}

/** 前端先挡明显错误，后端仍会返回完整校验原因并原样 toast。 */
function validateDraft(draft: Draft): TKey | null {
  if (!draft.name.trim()) return 'requestRules.errorNameRequired'
  if (draft.actions.length === 0) return 'requestRules.errorActionsRequired'
  if (draft.scopeKind === 'agent' && !draft.agentLabel.trim()) {
    return 'requestRules.errorAgentLabelRequired'
  }
  for (const action of draft.actions) {
    if (action.actionType === 'setDefaultMaxTokens') {
      const value = Number(action.value)
      if (!Number.isInteger(value) || value < 1 || value > 200000) {
        return 'requestRules.errorMaxTokensRange'
      }
    }
    if (action.actionType === 'setHeader' && !action.key.trim()) {
      return 'requestRules.errorHeaderKeyRequired'
    }
    if (action.actionType === 'setBodyField' && !action.key.trim()) {
      return 'requestRules.errorBodyKeyRequired'
    }
    if (action.actionType === 'toolTextRetry') {
      const extra = Number(action.value)
      if (!Number.isInteger(extra) || extra < 0 || extra > 5) {
        return 'requestRules.errorToolRetryRange'
      }
    }
  }
  return null
}

function draftToRule(draft: Draft): RequestRule {
  const scope: RuleScope =
    draft.scopeKind === 'agent'
      ? { kind: 'agent', label: draft.agentLabel.trim() }
      : { kind: draft.scopeKind }
  return {
    id: draft.id,
    name: draft.name.trim(),
    scope,
    enabled: draft.enabled,
    actions: draft.actions.map((action): RuleAction => {
      if (action.actionType === 'setDefaultMaxTokens') {
        return { type: action.actionType, value: Number(action.value) }
      }
      if (action.actionType === 'toolTextRetry') {
        return { type: action.actionType, extra: Number(action.value) }
      }
      return {
        type: action.actionType,
        key: action.key.trim(),
        value: action.value,
      }
    }),
  }
}

export function RequestRulesSection({ pushToast, run }: Props) {
  const [rules, setRules] = useState<RequestRule[]>([])
  const [loading, setLoading] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [rowBusy, setRowBusy] = useState<string | null>(null)
  const [editing, setEditing] = useState<Draft | null>(null)
  const [formError, setFormError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const [deleteTarget, setDeleteTarget] = useState<RequestRule | null>(null)

  const load = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    try {
      setRules(await invoke<RequestRule[]>('list_request_rules'))
    } catch (error) {
      setRules([])
      setLoadError(String(error))
    } finally {
      setLoading(false)
    }
  }, [])

  // 切 tab 会卸载再挂载，挂载拉一次即等价于切回来刷新。
  useEffect(() => {
    void load()
  }, [load])

  const toggleEnabled = async (rule: RequestRule, enabled: boolean) => {
    setRowBusy(rule.id)
    setRules((prev) => prev.map((item) => (item.id === rule.id ? { ...item, enabled } : item)))
    try {
      const saved = await invoke<RequestRule>('save_request_rule', {
        input: { spec: { ...rule, enabled } },
      })
      setRules((prev) => prev.map((item) => (item.id === saved.id ? saved : item)))
    } catch (error) {
      // 失败回滚，避免开关与磁盘配置不一致。
      setRules((prev) => prev.map((item) => (item.id === rule.id ? { ...item, enabled: !enabled } : item)))
      pushToast(String(error), 'error')
    } finally {
      setRowBusy(null)
    }
  }

  const openNew = () => {
    setFormError(null)
    setEditing(emptyDraft())
  }

  const openEdit = (rule: RequestRule) => {
    setFormError(null)
    setEditing(toDraft(rule))
  }

  const updateAction = (index: number, patch: Partial<DraftAction>) => {
    setEditing((prev) =>
      prev
        ? { ...prev, actions: prev.actions.map((action, i) => (i === index ? { ...action, ...patch } : action)) }
        : prev,
    )
  }

  /** 切换动作类型时清空 key，并按类型给出合理的数字初值。 */
  const changeActionType = (index: number, kind: ActionKind) => {
    updateAction(index, {
      actionType: kind,
      key: '',
      value: kind === 'setDefaultMaxTokens' ? '32768' : '',
    })
  }

  const addAction = () => {
    setEditing((prev) => (prev ? { ...prev, actions: [...prev.actions, emptyAction()] } : prev))
  }

  const removeAction = (index: number) => {
    setEditing((prev) =>
      prev ? { ...prev, actions: prev.actions.filter((_, i) => i !== index) } : prev,
    )
  }

  const submit = async () => {
    if (!editing) return
    const invalid = validateDraft(editing)
    if (invalid) {
      setFormError(t(invalid))
      return
    }
    setFormError(null)
    setSaving(true)
    try {
      await invoke('save_request_rule', { input: { spec: draftToRule(editing) } })
      pushToast(t('requestRules.toastSaved'), 'ok')
      setEditing(null)
      await load()
    } catch (error) {
      // 后端校验失败会返回具体原因，原样展示。
      pushToast(String(error), 'error')
    } finally {
      setSaving(false)
    }
  }

  const confirmDelete = async () => {
    if (!deleteTarget) return
    const target = deleteTarget
    setDeleteTarget(null)
    await run(
      () => invoke('delete_request_rule', { input: { rule_id: target.id } }),
      t('requestRules.toastDeleted'),
    )
    await load()
  }

  const busy = loading || saving

  return (
    <section className="card request-rules">
      <div className="section-title">
        <h3>{t('requestRules.title')}</h3>
        <div className="title-actions">
          <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => void load()}>
            <RefreshCw size={14} className={loading ? 'spin' : ''} />
            {t('requestRules.refresh')}
          </button>
          <button type="button" className="primary tiny-btn" disabled={busy} onClick={openNew}>
            <Plus size={14} />
            {t('requestRules.newRule')}
          </button>
        </div>
      </div>

      <p className="hint compact request-rules-hint">{t('requestRules.hint')}</p>

      {loadError && (
        <div className="request-rules-error">
          <p className="error-text request-rules-note">
            {t('requestRules.loadFailed', { error: loadError })}
          </p>
          <button type="button" className="ghost tiny-btn" disabled={loading} onClick={() => void load()}>
            {t('requestRules.retry')}
          </button>
        </div>
      )}

      <div className="request-rules-wrap">
        <table className="request-rules-table">
          <thead>
            <tr>
              <th>{t('requestRules.colName')}</th>
              <th>{t('requestRules.colScope')}</th>
              <th>{t('requestRules.colActions')}</th>
              <th>{t('requestRules.colEnabled')}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {rules.length === 0 && (
              <tr>
                <td colSpan={5} className="request-rules-empty">
                  {loading ? t('common.loading') : t('requestRules.empty')}
                </td>
              </tr>
            )}
            {rules.map((rule) => (
              <tr key={rule.id} className={rule.enabled ? '' : 'is-disabled'}>
                <td>
                  <span className="request-rules-name">{rule.name}</span>
                  {isBuiltin(rule) && (
                    <span className="request-rules-badge">{t('requestRules.builtin')}</span>
                  )}
                </td>
                <td>{scopeText(rule.scope)}</td>
                <td className="request-rules-actions-cell">
                  {rule.actions.map(summarizeAction).join(' · ')}
                </td>
                <td>
                  <input
                    className="request-rules-switch"
                    type="checkbox"
                    aria-label={t('requestRules.colEnabled')}
                    checked={rule.enabled}
                    disabled={busy || rowBusy === rule.id}
                    onChange={(event) => void toggleEnabled(rule, event.target.checked)}
                  />
                </td>
                <td>
                  <div className="request-rules-row-actions">
                    <button type="button" className="ghost tiny-btn" onClick={() => openEdit(rule)}>
                      <Pencil size={13} />
                      {t('requestRules.edit')}
                    </button>
                    <button
                      type="button"
                      className="ghost tiny-btn danger-link"
                      onClick={() => setDeleteTarget(rule)}
                    >
                      <Trash2 size={13} />
                      {t('requestRules.delete')}
                    </button>
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {editing && (
        <ModalPortal>
          <div
            className="modal-overlay"
            onMouseDown={(event) => {
              if (event.target === event.currentTarget && !saving) setEditing(null)
            }}
          >
            <div
              className="modal request-rules-modal"
              role="dialog"
              aria-modal="true"
              aria-label={editing.id ? t('requestRules.editTitle') : t('requestRules.newTitle')}
            >
              <div className="modal-head">
                <h3>{editing.id ? t('requestRules.editTitle') : t('requestRules.newTitle')}</h3>
                <button
                  type="button"
                  className="icon-btn"
                  aria-label={t('common.close')}
                  disabled={saving}
                  onClick={() => setEditing(null)}
                >
                  <X size={18} />
                </button>
              </div>

              <div className="modal-body request-rules-form">
                <label className="field-label">
                  {t('requestRules.nameLabel')}
                  <input
                    value={editing.name}
                    onChange={(event) =>
                      setEditing((prev) => (prev ? { ...prev, name: event.target.value } : prev))
                    }
                  />
                </label>

                <label className="field-label">
                  {t('requestRules.scopeLabel')}
                  <select
                    className="app-select"
                    value={editing.scopeKind}
                    onChange={(event) =>
                      setEditing((prev) =>
                        prev ? { ...prev, scopeKind: event.target.value as ScopeKind } : prev,
                      )
                    }
                  >
                    {SCOPES.map((scope) => (
                      <option key={scope} value={scope}>
                        {scope === 'agent'
                          ? t('requestRules.scopeAgent')
                          : t(SCOPE_LABEL[scope])}
                      </option>
                    ))}
                  </select>
                </label>

                {editing.scopeKind === 'agent' && (
                  <label className="field-label">
                    {t('requestRules.agentLabel')}
                    <input
                      value={editing.agentLabel}
                      placeholder="codex"
                      onChange={(event) =>
                        setEditing((prev) =>
                          prev ? { ...prev, agentLabel: event.target.value } : prev,
                        )
                      }
                    />
                    <span className="hint compact">{t('requestRules.agentLabelHint')}</span>
                  </label>
                )}

                <label className="request-rules-enabled-row">
                  <input
                    type="checkbox"
                    checked={editing.enabled}
                    onChange={(event) =>
                      setEditing((prev) => (prev ? { ...prev, enabled: event.target.checked } : prev))
                    }
                  />
                  {t('requestRules.enabledLabel')}
                </label>

                <div className="request-rules-actions">
                  <div className="request-rules-actions-head">
                    <span>{t('requestRules.actionsLabel')}</span>
                  </div>
                  {editing.actions.map((action, index) => (
                    <div className="request-rules-action-row" key={index}>
                      <select
                        className="app-select"
                        value={action.actionType}
                        aria-label={t('requestRules.actionType')}
                        onChange={(event) =>
                          changeActionType(index, event.target.value as ActionKind)
                        }
                      >
                        {ACTION_KINDS.map((kind) => (
                          <option key={kind} value={kind}>
                            {t(ACTION_LABEL[kind])}
                          </option>
                        ))}
                      </select>

                      {action.actionType === 'setDefaultMaxTokens' && (
                        <input
                          type="number"
                          min={1}
                          max={200000}
                          value={action.value}
                          aria-label={t('requestRules.maxTokensLabel')}
                          onChange={(event) => updateAction(index, { value: event.target.value })}
                        />
                      )}

                      {action.actionType === 'toolTextRetry' && (
                        <input
                          type="number"
                          min={0}
                          max={5}
                          value={action.value}
                          aria-label={t('requestRules.toolRetryExtraLabel')}
                          onChange={(event) => updateAction(index, { value: event.target.value })}
                        />
                      )}

                      {(action.actionType === 'setHeader' || action.actionType === 'setBodyField') && (
                        <>
                          <input
                            value={action.key}
                            placeholder={
                              action.actionType === 'setHeader'
                                ? t('requestRules.headerKeyLabel')
                                : t('requestRules.bodyKeyLabel')
                            }
                            aria-label={
                              action.actionType === 'setHeader'
                                ? t('requestRules.headerKeyLabel')
                                : t('requestRules.bodyKeyLabel')
                            }
                            onChange={(event) => updateAction(index, { key: event.target.value })}
                          />
                          <input
                            value={action.value}
                            placeholder={
                              action.actionType === 'setHeader'
                                ? t('requestRules.headerValueLabel')
                                : t('requestRules.bodyValueLabel')
                            }
                            aria-label={
                              action.actionType === 'setHeader'
                                ? t('requestRules.headerValueLabel')
                                : t('requestRules.bodyValueLabel')
                            }
                            onChange={(event) => updateAction(index, { value: event.target.value })}
                          />
                        </>
                      )}

                      <button
                        type="button"
                        className="ghost tiny-btn danger-link"
                        aria-label={t('requestRules.removeAction')}
                        onClick={() => removeAction(index)}
                      >
                        <Trash2 size={13} />
                      </button>
                    </div>
                  ))}
                  <button type="button" className="ghost tiny-btn" onClick={addAction}>
                    <Plus size={13} />
                    {t('requestRules.addAction')}
                  </button>
                </div>

                {formError && <p className="error-text request-rules-note">{formError}</p>}

                <div className="modal-actions">
                  <button
                    type="button"
                    className="ghost"
                    disabled={saving}
                    onClick={() => setEditing(null)}
                  >
                    {t('requestRules.cancel')}
                  </button>
                  <button type="button" className="primary" disabled={saving} onClick={() => void submit()}>
                    {saving ? t('requestRules.saving') : t('requestRules.save')}
                  </button>
                </div>
              </div>
            </div>
          </div>
        </ModalPortal>
      )}

      <ConfirmModal
        open={deleteTarget !== null}
        danger
        title={t('requestRules.deleteConfirmTitle')}
        body={
          deleteTarget
            ? t('requestRules.deleteConfirmBody', { name: deleteTarget.name })
            : undefined
        }
        confirmLabel={t('requestRules.delete')}
        onConfirm={() => void confirmDelete()}
        onCancel={() => setDeleteTarget(null)}
      />
    </section>
  )
}

/** 「策略盒子」主 tab（v1.1.11 R3）。
 * 作用：把多个服务商 / 模型聚合成一个对外统一 model id 的策略组，
 * 按 priority / weighted / round_robin / speed_threshold 选路。
 * 本组件只负责配置与健康度观测，真正的选路在 Rust 网关侧完成；
 * 保存 / 删除 / 健康度走 list/save/delete/get_strategy_box_status 四条命令。
 * 删除会从网关撤掉对外模型 id，属于不可撤销动作，统一走 ConfirmModal。 */
import React, { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from './transport';
import { Pencil, Plus, RefreshCw, Trash2, X } from 'lucide-react'
import { ConfirmModal } from './ConfirmModal'
import { ModalPortal } from './ModalPortal'
import { t, type TKey } from './i18n'

type StrategyMode = 'priority' | 'weighted' | 'round_robin' | 'speed_threshold'

type StrategyMember = {
  providerId: string
  modelName: string
  weight: number
  priority: number
}

type StrategyBox = {
  id: string
  name: string
  publicModelId: string
  members: StrategyMember[]
  mode: StrategyMode
  speedThresholdMs: number | null
  cooldownMs: number | null
  enabled: boolean
}

type StrategyMemberStatus = {
  providerId: string
  modelName: string
  lastLatencyMs: number | null
  consecutiveFailures: number
  cooldownUntil: string | null
  lastError: string | null
}

type ProviderOption = {
  id: string
  name: string
  enabled: boolean
  experimental?: boolean
}

type Props = {
  providers: ProviderOption[]
  pushToast: (msg: string, type?: 'ok' | 'error' | 'info') => void
  run: (action: () => Promise<unknown>, ok?: string) => Promise<void>
}

type RegistryEntry = { modelName: string; enabled: boolean }

const MODES: StrategyMode[] = ['priority', 'weighted', 'round_robin', 'speed_threshold']

const MODE_LABEL: Record<StrategyMode, TKey> = {
  priority: 'strategy.modePriority',
  weighted: 'strategy.modeWeighted',
  round_robin: 'strategy.modeRoundRobin',
  speed_threshold: 'strategy.modeSpeedThreshold',
}

const MODE_SHORT_LABEL: Record<StrategyMode, TKey> = {
  priority: 'strategy.modePriorityShort',
  weighted: 'strategy.modeWeightedShort',
  round_robin: 'strategy.modeRoundRobinShort',
  speed_threshold: 'strategy.modeSpeedThresholdShort',
}

/** 冷却截止时间按本地时区展示；null / 非法值显示占位符。 */
function formatLocalTime(iso: string | null): string {
  if (!iso) return '—'
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return iso
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`
}

/** 编辑态成员：数字先按字符串保存，保存时再统一校验并转 number。 */
type DraftMember = {
  providerId: string
  modelName: string
  weight: string
  priority: string
}

type Draft = {
  id: string
  name: string
  publicModelId: string
  mode: StrategyMode
  speedThresholdMs: string
  cooldownMs: string
  enabled: boolean
  members: DraftMember[]
}

function emptyDraft(providers: ProviderOption[]): Draft {
  return {
    id: '',
    name: '',
    publicModelId: '',
    mode: 'priority',
    speedThresholdMs: '',
    cooldownMs: '',
    enabled: true,
    members: [
      {
        providerId: providers.find((p) => p.enabled)?.id ?? '',
        modelName: '',
        weight: '1',
        priority: '1',
      },
    ],
  }
}

/** 前端先挡一层明显错误，后端仍会返回完整校验原因并原样 toast。 */
function validateDraft(draft: Draft): TKey | null {
  if (!draft.name.trim()) return 'strategy.validation.nameRequired'
  if (!draft.publicModelId.trim()) return 'strategy.validation.publicIdRequired'
  if (draft.members.length === 0) return 'strategy.validation.membersRequired'
  const weights: number[] = []
  for (const member of draft.members) {
    if (!member.providerId || !member.modelName.trim()) {
      return 'strategy.validation.memberIncomplete'
    }
    const weight = Number(member.weight)
    if (!Number.isInteger(weight) || weight < 0) {
      return 'strategy.validation.weightInvalid'
    }
    const priority = Number(member.priority)
    if (!Number.isInteger(priority) || priority < 1) {
      return 'strategy.validation.priorityInvalid'
    }
    weights.push(weight)
  }
  if (draft.mode === 'weighted' && weights.reduce((sum, w) => sum + w, 0) <= 0) {
    return 'strategy.validation.weightedZero'
  }
  if (draft.mode === 'speed_threshold') {
    const threshold = Number(draft.speedThresholdMs)
    if (!Number.isFinite(threshold) || threshold <= 0) {
      return 'strategy.validation.speedThresholdInvalid'
    }
  }
  if (draft.cooldownMs.trim() !== '') {
    const cooldown = Number(draft.cooldownMs)
    if (!Number.isInteger(cooldown) || cooldown < 0) {
      return 'strategy.validation.cooldownInvalid'
    }
  }
  return null
}

export function StrategyBoxPanel({ providers, pushToast, run }: Props) {
  const [boxes, setBoxes] = useState<StrategyBox[]>([])
  const [loading, setLoading] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [rowBusy, setRowBusy] = useState<string | null>(null)
  const [editing, setEditing] = useState<Draft | null>(null)
  const [formError, setFormError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const [modelCache, setModelCache] = useState<Record<string, string[]>>({})
  const [deleteTarget, setDeleteTarget] = useState<StrategyBox | null>(null)
  const [healthOpenId, setHealthOpenId] = useState<string | null>(null)
  const [health, setHealth] = useState<StrategyMemberStatus[]>([])
  const [healthBusy, setHealthBusy] = useState(false)
  const [healthError, setHealthError] = useState<string | null>(null)
  // 同一 provider 的模型候选只拉一次；并发请求用 ref 去重，避免反复打 IPC。
  const inflightModels = useRef<Set<string>>(new Set())

  const load = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    try {
      setBoxes(await invoke<StrategyBox[]>('list_strategy_boxes'))
    } catch (error) {
      setBoxes([])
      setLoadError(String(error))
    } finally {
      setLoading(false)
    }
  }, [])

  // 切走 tab 时组件卸载、切回来重新挂载，因此挂载拉一次即等价于「切换 tab 回来刷新」。
  useEffect(() => {
    void load()
  }, [load])

  const loadModels = useCallback(async (providerId: string) => {
    if (!providerId || inflightModels.current.has(providerId)) return
    inflightModels.current.add(providerId)
    try {
      const entries = await invoke<RegistryEntry[]>('list_model_registry', {
        input: { provider_id: providerId },
      })
      setModelCache((prev) => ({
        ...prev,
        [providerId]: entries.filter((entry) => entry.enabled).map((entry) => entry.modelName),
      }))
    } catch {
      // 候选拉取失败不打断编辑：写入空候选占位，避免每次渲染重复请求；模型名仍可手输。
      setModelCache((prev) => (providerId in prev ? prev : { ...prev, [providerId]: [] }))
    } finally {
      inflightModels.current.delete(providerId)
    }
  }, [])

  // 打开编辑弹窗后，为已出现的服务商补拉模型候选。
  useEffect(() => {
    if (!editing) return
    const ids = Array.from(
      new Set(editing.members.map((member) => member.providerId).filter(Boolean)),
    )
    for (const providerId of ids) {
      if (!(providerId in modelCache)) void loadModels(providerId)
    }
  }, [editing, modelCache, loadModels])

  const loadHealth = useCallback(async (boxId: string) => {
    setHealthBusy(true)
    setHealthError(null)
    try {
      setHealth(await invoke<StrategyMemberStatus[]>('get_strategy_box_status', {
        input: { box_id: boxId },
      }))
    } catch (error) {
      setHealth([])
      setHealthError(String(error))
    } finally {
      setHealthBusy(false)
    }
  }, [])

  const toggleHealth = (boxId: string) => {
    if (healthOpenId === boxId) {
      setHealthOpenId(null)
      return
    }
    setHealthOpenId(boxId)
    setHealth([])
    void loadHealth(boxId)
  }

  const toggleEnabled = async (box: StrategyBox, enabled: boolean) => {
    setRowBusy(box.id)
    setBoxes((prev) => prev.map((item) => (item.id === box.id ? { ...item, enabled } : item)))
    try {
      const saved = await invoke<StrategyBox>('save_strategy_box', {
        input: { spec: { ...box, enabled } },
      })
      setBoxes((prev) => prev.map((item) => (item.id === saved.id ? saved : item)))
    } catch (error) {
      // 失败回滚，避免开关与磁盘配置不一致。
      setBoxes((prev) => prev.map((item) => (item.id === box.id ? { ...item, enabled: !enabled } : item)))
      pushToast(String(error), 'error')
    } finally {
      setRowBusy(null)
    }
  }

  const openNew = () => {
    setFormError(null)
    setEditing(emptyDraft(providers))
  }

  const openEdit = (box: StrategyBox) => {
    setFormError(null)
    setEditing({
      id: box.id,
      name: box.name,
      publicModelId: box.publicModelId,
      mode: box.mode,
      speedThresholdMs: box.speedThresholdMs == null ? '' : String(box.speedThresholdMs),
      cooldownMs: box.cooldownMs == null ? '' : String(box.cooldownMs),
      enabled: box.enabled,
      members: box.members.map((member) => ({
        providerId: member.providerId,
        modelName: member.modelName,
        weight: String(member.weight),
        priority: String(member.priority),
      })),
    })
  }

  const updateMember = (index: number, patch: Partial<DraftMember>) => {
    setEditing((prev) =>
      prev
        ? { ...prev, members: prev.members.map((member, i) => (i === index ? { ...member, ...patch } : member)) }
        : prev,
    )
  }

  const addMember = () => {
    setEditing((prev) =>
      prev
        ? {
          ...prev,
          members: [
            ...prev.members,
            {
              providerId: providers.find((p) => p.enabled)?.id ?? '',
              modelName: '',
              weight: '1',
              priority: String(prev.members.length + 1),
            },
          ],
        }
        : prev,
    )
  }

  const removeMember = (index: number) => {
    setEditing((prev) =>
      prev ? { ...prev, members: prev.members.filter((_, i) => i !== index) } : prev,
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
      const spec: StrategyBox = {
        id: editing.id,
        name: editing.name.trim(),
        publicModelId: editing.publicModelId.trim(),
        mode: editing.mode,
        speedThresholdMs:
          editing.mode === 'speed_threshold' ? Number(editing.speedThresholdMs) : null,
        cooldownMs: editing.cooldownMs.trim() === '' ? null : Number(editing.cooldownMs),
        enabled: editing.enabled,
        members: editing.members.map((member) => ({
          providerId: member.providerId,
          modelName: member.modelName.trim(),
          weight: Number(member.weight),
          priority: Number(member.priority),
        })),
      }
      await invoke('save_strategy_box', { input: { spec } })
      pushToast(t('strategy.toastSaved'), 'ok')
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
      () => invoke('delete_strategy_box', { input: { box_id: target.id } }),
      t('strategy.toastDeleted'),
    )
    if (healthOpenId === target.id) {
      setHealthOpenId(null)
      setHealth([])
    }
    await load()
  }

  const providerName = (id: string) => providers.find((p) => p.id === id)?.name ?? id
  const busy = loading || saving

  return (
    <section className="card">
      <div className="section-title">
        <h3>{t('tab.strategyBox.title')}</h3>
        <div className="title-actions">
          <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => void load()}>
            <RefreshCw size={14} className={loading ? 'spin' : ''} />
            {t('strategy.refresh')}
          </button>
          <button type="button" className="primary tiny-btn" disabled={busy} onClick={openNew}>
            <Plus size={14} />
            {t('strategy.newBox')}
          </button>
        </div>
      </div>

      {loadError && <p className="error-text strategy-note">{loadError}</p>}

      <div className="strategy-wrap">
        <table className="strategy-table">
          <thead>
            <tr>
              <th>{t('strategy.colName')}</th>
              <th>{t('strategy.colPublicId')}</th>
              <th>{t('strategy.colMode')}</th>
              <th>{t('strategy.colMembers')}</th>
              <th>{t('strategy.colEnabled')}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {boxes.length === 0 && (
              <tr>
                <td colSpan={6} className="strategy-empty">
                  {loading ? t('common.loading') : t('strategy.empty')}
                </td>
              </tr>
            )}
            {boxes.map((box) => (
              <React.Fragment key={box.id}>
                <tr className={box.enabled ? '' : 'is-disabled'}>
                  <td>{box.name}</td>
                  <td>
                    <code className="strategy-public-id">{box.publicModelId}</code>
                  </td>
                  <td>{t(MODE_SHORT_LABEL[box.mode])}</td>
                  <td>{box.members.length}</td>
                  <td>
                    <input
                      className="strategy-switch"
                      type="checkbox"
                      aria-label={t('strategy.colEnabled')}
                      checked={box.enabled}
                      disabled={busy || rowBusy === box.id}
                      onChange={(event) => void toggleEnabled(box, event.target.checked)}
                    />
                  </td>
                  <td>
                    <div className="strategy-row-actions">
                      <button type="button" className="ghost tiny-btn" onClick={() => openEdit(box)}>
                        <Pencil size={13} />
                        {t('strategy.edit')}
                      </button>
                      <button type="button" className="ghost tiny-btn" onClick={() => toggleHealth(box.id)}>
                        {t('strategy.status')}
                      </button>
                      <button
                        type="button"
                        className="ghost tiny-btn danger-link"
                        onClick={() => setDeleteTarget(box)}
                      >
                        <Trash2 size={13} />
                        {t('strategy.delete')}
                      </button>
                    </div>
                  </td>
                </tr>
                {healthOpenId === box.id && (
                  <tr className="strategy-health-row">
                    <td colSpan={6}>
                      <div className="strategy-health">
                        <div className="strategy-health-head">
                          <strong>{t('strategy.healthTitle')}</strong>
                          <button
                            type="button"
                            className="ghost tiny-btn"
                            disabled={healthBusy}
                            onClick={() => void loadHealth(box.id)}
                          >
                            <RefreshCw size={13} className={healthBusy ? 'spin' : ''} />
                            {t('strategy.refresh')}
                          </button>
                        </div>
                        {healthError && <p className="error-text strategy-note">{healthError}</p>}
                        {health.length === 0 ? (
                          <p className="hint compact">
                            {box.members.length === 0 ? t('strategy.noMembers') : t('strategy.healthEmpty')}
                          </p>
                        ) : (
                          <table className="strategy-health-table">
                            <thead>
                              <tr>
                                <th>{t('strategy.healthMember')}</th>
                                <th>{t('strategy.healthLatency')}</th>
                                <th>{t('strategy.healthFailures')}</th>
                                <th>{t('strategy.healthCooldown')}</th>
                                <th>{t('strategy.healthLastError')}</th>
                              </tr>
                            </thead>
                            <tbody>
                              {health.map((member) => (
                                <tr key={`${member.providerId}::${member.modelName}`}>
                                  <td>{`${providerName(member.providerId)} · ${member.modelName}`}</td>
                                  <td>{member.lastLatencyMs == null ? '—' : `${member.lastLatencyMs} ms`}</td>
                                  <td>{member.consecutiveFailures}</td>
                                  <td>{formatLocalTime(member.cooldownUntil)}</td>
                                  <td className="strategy-health-error" title={member.lastError ?? undefined}>
                                    {member.lastError ?? '—'}
                                  </td>
                                </tr>
                              ))}
                            </tbody>
                          </table>
                        )}
                      </div>
                    </td>
                  </tr>
                )}
              </React.Fragment>
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
              className="modal strategy-modal"
              role="dialog"
              aria-modal="true"
              aria-label={editing.id ? t('strategy.editTitle') : t('strategy.newTitle')}
            >
              <div className="modal-head">
                <h3>{editing.id ? t('strategy.editTitle') : t('strategy.newTitle')}</h3>
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

              <div className="modal-body strategy-form">
                <label className="field-label">
                  {t('strategy.nameLabel')}
                  <input
                    value={editing.name}
                    onChange={(event) =>
                      setEditing((prev) => (prev ? { ...prev, name: event.target.value } : prev))
                    }
                  />
                </label>

                <label className="field-label">
                  {t('strategy.publicIdLabel')}
                  <input
                    value={editing.publicModelId}
                    placeholder="sutai-strategy"
                    onChange={(event) =>
                      setEditing((prev) => (prev ? { ...prev, publicModelId: event.target.value } : prev))
                    }
                  />
                </label>

                <label className="field-label">
                  {t('strategy.modeLabel')}
                  <select
                    className="app-select"
                    value={editing.mode}
                    onChange={(event) =>
                      setEditing((prev) =>
                        prev ? { ...prev, mode: event.target.value as StrategyMode } : prev,
                      )
                    }
                  >
                    {MODES.map((mode) => (
                      <option key={mode} value={mode}>
                        {t(MODE_LABEL[mode])}
                      </option>
                    ))}
                  </select>
                </label>

                {editing.mode === 'speed_threshold' && (
                  <label className="field-label">
                    {t('strategy.speedThresholdLabel')}
                    <input
                      type="number"
                      min={1}
                      value={editing.speedThresholdMs}
                      onChange={(event) =>
                        setEditing((prev) =>
                          prev ? { ...prev, speedThresholdMs: event.target.value } : prev,
                        )
                      }
                    />
                  </label>
                )}

                <label className="field-label">
                  {t('strategy.cooldownLabel')}
                  <input
                    type="number"
                    min={0}
                    value={editing.cooldownMs}
                    onChange={(event) =>
                      setEditing((prev) => (prev ? { ...prev, cooldownMs: event.target.value } : prev))
                    }
                  />
                </label>

                <label className="strategy-enabled-row">
                  <input
                    type="checkbox"
                    checked={editing.enabled}
                    onChange={(event) =>
                      setEditing((prev) => (prev ? { ...prev, enabled: event.target.checked } : prev))
                    }
                  />
                  {t('strategy.enabledLabel')}
                </label>

                <div className="strategy-members">
                  <div className="strategy-member-head">
                    <span>{t('strategy.memberProvider')}</span>
                    <span>{t('strategy.memberModel')}</span>
                    <span>{t('strategy.memberWeight')}</span>
                    <span>{t('strategy.memberPriority')}</span>
                    <span />
                  </div>
                  {editing.members.map((member, index) => {
                    const candidates = modelCache[member.providerId] ?? []
                    const listId = `strategy-models-${index}`
                    return (
                      <div className="strategy-member-row" key={index}>
                        <select
                          className="app-select"
                          value={member.providerId}
                          aria-label={t('strategy.memberProvider')}
                          onChange={(event) => {
                            updateMember(index, { providerId: event.target.value, modelName: '' })
                            void loadModels(event.target.value)
                          }}
                        >
                          <option value="">{t('strategy.memberProvider')}</option>
                          {providers.map((provider) => (
                            <option key={provider.id} value={provider.id}>
                              {provider.name}
                              {provider.enabled ? '' : t('modelList.providerDisabled')}
                            </option>
                          ))}
                        </select>
                        <input
                          list={listId}
                          value={member.modelName}
                          placeholder={t('strategy.memberModel')}
                          aria-label={t('strategy.memberModel')}
                          onChange={(event) => updateMember(index, { modelName: event.target.value })}
                        />
                        <datalist id={listId}>
                          {candidates.map((name) => (
                            <option key={name} value={name} />
                          ))}
                        </datalist>
                        <input
                          type="number"
                          min={0}
                          value={member.weight}
                          aria-label={t('strategy.memberWeight')}
                          onChange={(event) => updateMember(index, { weight: event.target.value })}
                        />
                        <input
                          type="number"
                          min={1}
                          value={member.priority}
                          aria-label={t('strategy.memberPriority')}
                          onChange={(event) => updateMember(index, { priority: event.target.value })}
                        />
                        <button
                          type="button"
                          className="ghost tiny-btn danger-link"
                          aria-label={t('strategy.removeMember')}
                          onClick={() => removeMember(index)}
                        >
                          <Trash2 size={13} />
                        </button>
                      </div>
                    )
                  })}
                  <button type="button" className="ghost tiny-btn" onClick={addMember}>
                    <Plus size={13} />
                    {t('strategy.addMember')}
                  </button>
                </div>

                {formError && <p className="error-text strategy-note">{formError}</p>}

                <div className="modal-actions">
                  <button
                    type="button"
                    className="ghost"
                    disabled={saving}
                    onClick={() => setEditing(null)}
                  >
                    {t('strategy.cancel')}
                  </button>
                  <button type="button" className="primary" disabled={saving} onClick={() => void submit()}>
                    {saving ? t('strategy.saving') : t('strategy.save')}
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
        title={t('strategy.deleteConfirmTitle')}
        body={
          deleteTarget
            ? t('strategy.deleteConfirmBody', {
              name: deleteTarget.name,
              id: deleteTarget.publicModelId,
            })
            : undefined
        }
        confirmLabel={t('strategy.delete')}
        onConfirm={() => void confirmDelete()}
        onCancel={() => setDeleteTarget(null)}
      />
    </section>
  )
}

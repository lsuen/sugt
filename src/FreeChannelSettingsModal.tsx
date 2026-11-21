/** 实验·免费通道专属设置：免责声明、获取并测试、闲时测速与自动轮换。
 * 入口在模型配置行，不进设置 Tab；策略盒子仍是可选外层，本弹窗只管通道内行为。 */
import React, { useEffect, useState } from 'react'
import { ModalPortal } from './ModalPortal'
import { invoke } from './transport'
import { t } from './i18n'
import { ConfirmModal } from './ConfirmModal'

export type FreeChannelConfigSlice = {
  free_models_fetch_and_test?: boolean
  experimental_free_disclaimer_accepted?: boolean
  experimental_free_idle_probe_enabled?: boolean
  experimental_free_idle_probe_interval_secs?: number
  experimental_free_auto_rotate?: boolean
}

type Props = {
  open: boolean
  busy: boolean
  /** 打开时若尚未确认免责，强制展示确认区 */
  requireAccept?: boolean
  currentRoute?: string
  onClose: () => void
  onSaved: () => void
  run: (action: () => Promise<unknown>, ok?: string) => Promise<void> | void
  pushToast: (msg: string, type?: 'ok' | 'error' | 'info') => void
}

const DEFAULT_INTERVAL = 1800
const MIN_INTERVAL = 300

export function FreeChannelSettingsModal({
  open,
  busy,
  requireAccept = false,
  currentRoute,
  onClose,
  onSaved,
  run,
  pushToast,
}: Props) {
  const [fetchAndTest, setFetchAndTest] = useState(true)
  const [accepted, setAccepted] = useState(false)
  const [idleEnabled, setIdleEnabled] = useState(false)
  const [idleInterval, setIdleInterval] = useState(String(DEFAULT_INTERVAL))
  const [autoRotate, setAutoRotate] = useState(false)
  const [confirmIdle, setConfirmIdle] = useState(false)
  const [loaded, setLoaded] = useState(false)

  useEffect(() => {
    if (!open) return
    let cancelled = false
    void invoke<FreeChannelConfigSlice>('get_config')
      .then((cfg) => {
        if (cancelled) return
        setFetchAndTest(cfg.free_models_fetch_and_test ?? true)
        setAccepted(Boolean(cfg.experimental_free_disclaimer_accepted))
        setIdleEnabled(Boolean(cfg.experimental_free_idle_probe_enabled))
        setIdleInterval(
          String(cfg.experimental_free_idle_probe_interval_secs ?? DEFAULT_INTERVAL),
        )
        setAutoRotate(Boolean(cfg.experimental_free_auto_rotate))
        setLoaded(true)
      })
      .catch((err) => {
        pushToast(String(err), 'error')
        onClose()
      })
    return () => {
      cancelled = true
    }
  }, [open, onClose, pushToast])

  if (!open) return null

  // 未确认免责时不可关掉弹窗溜走。
  const disclaimerGate = !accepted

  const save = (patch: Record<string, unknown>, ok?: string) => {
    run(async () => {
      await invoke('update_gateway_settings', { input: patch })
      onSaved()
    }, ok ?? t('common.update'))
  }

  const acceptDisclaimer = () => {
    setAccepted(true)
    save({ experimentalFreeDisclaimerAccepted: true }, t('freeSettings.accepted'))
  }

  const toggleIdle = (next: boolean) => {
    if (next && !idleEnabled) {
      setConfirmIdle(true)
      return
    }
    setIdleEnabled(next)
    save({ experimentalFreeIdleProbeEnabled: next })
  }

  const saveInterval = () => {
    const n = Number(idleInterval)
    if (!Number.isFinite(n) || n < MIN_INTERVAL) {
      pushToast(t('freeSettings.intervalInvalid', { min: MIN_INTERVAL }), 'error')
      setIdleInterval(String(DEFAULT_INTERVAL))
      return
    }
    const secs = Math.floor(n)
    setIdleInterval(String(secs))
    save({ experimentalFreeIdleProbeIntervalSecs: secs })
  }

  return (
    <>
      <ModalPortal>
        <div
          className="modal-overlay"
          onMouseDown={(e) => {
            if (e.target === e.currentTarget && !disclaimerGate) onClose()
          }}
        >
          <div
            className="modal free-channel-modal"
            role="dialog"
            aria-modal="true"
            aria-label={t('freeSettings.title')}
          >
            <div className="modal-head">
              <h3>{t('freeSettings.title')}</h3>
              <button
                type="button"
                className="icon-btn"
                onClick={onClose}
                aria-label={t('common.close')}
                disabled={disclaimerGate && !accepted}
              >
                ×
              </button>
            </div>
            <div className="modal-body">
              {!loaded ? (
                <p className="hint compact">{t('common.loading')}</p>
              ) : (
                <>
                  <section className="settings-group">
                    <h4>{t('freeSettings.disclaimerTitle')}</h4>
                    {requireAccept && !accepted ? (
                      <p className="hint compact">{t('freeSettings.needAcceptFirst')}</p>
                    ) : null}
                    <p className="hint">{t('freeSettings.disclaimerBody')}</p>
                    {accepted ? (
                      <p className="hint compact">{t('freeSettings.disclaimerDone')}</p>
                    ) : (
                      <button
                        type="button"
                        className="primary"
                        disabled={busy}
                        onClick={acceptDisclaimer}
                      >
                        {t('freeSettings.accept')}
                      </button>
                    )}
                  </section>

                  {accepted && (
                    <>
                      {currentRoute ? (
                        <p className="hint compact">
                          {t('freeSettings.currentRoute', { model: currentRoute })}
                        </p>
                      ) : null}

                      <section className="settings-group switches">
                        <h4>{t('freeSettings.fetchMode')}</h4>
                        <p className="hint compact">{t('freeSettings.fetchModeHint')}</p>
                        <label className="switch-line select-line">
                          <span>{t('set.freeChannel')}</span>
                          <select
                            value={String(fetchAndTest)}
                            disabled={busy}
                            onChange={(e) => {
                              const next = e.target.value === 'true'
                              setFetchAndTest(next)
                              save({ freeModelsFetchAndTest: next })
                            }}
                          >
                            <option value="false">{t('set.fetchOnly')}</option>
                            <option value="true">{t('set.fetchAndTest')}</option>
                          </select>
                        </label>
                      </section>

                      <section className="settings-group switches">
                        <h4>{t('freeSettings.idleTitle')}</h4>
                        <p className="hint compact">{t('freeSettings.idleHint')}</p>
                        <label className="switch-line">
                          <span>{t('freeSettings.idleEnable')}</span>
                          <input
                            type="checkbox"
                            checked={idleEnabled}
                            disabled={busy}
                            onChange={(e) => toggleIdle(e.target.checked)}
                          />
                        </label>
                        <div className="switch-line maintenance-row">
                          <span>{t('freeSettings.idleInterval')}</span>
                          <span className="maintenance-days">
                            <input
                              type="number"
                              min={MIN_INTERVAL}
                              step={60}
                              value={idleInterval}
                              disabled={busy || !idleEnabled}
                              aria-label={t('freeSettings.idleInterval')}
                              onChange={(e) => setIdleInterval(e.target.value)}
                              onBlur={saveInterval}
                            />
                          </span>
                        </div>
                        <label className="switch-line">
                          <span>{t('freeSettings.autoRotate')}</span>
                          <input
                            type="checkbox"
                            checked={autoRotate}
                            disabled={busy}
                            onChange={(e) => {
                              const next = e.target.checked
                              setAutoRotate(next)
                              save({ experimentalFreeAutoRotate: next })
                            }}
                          />
                        </label>
                        <p className="hint compact">{t('freeSettings.autoRotateHint')}</p>
                      </section>

                      <section className="settings-group">
                        <h4>{t('freeSettings.strategyNoteTitle')}</h4>
                        <p className="hint compact">{t('freeSettings.strategyNote')}</p>
                      </section>
                    </>
                  )}
                </>
              )}
            </div>
            <div className="modal-actions">
              <button type="button" className="ghost" onClick={onClose} disabled={busy}>
                {t('common.close')}
              </button>
            </div>
          </div>
        </div>
      </ModalPortal>

      <ConfirmModal
        open={confirmIdle}
        title={t('freeSettings.idleConfirmTitle')}
        body={t('freeSettings.idleConfirmBody')}
        confirmLabel={t('freeSettings.idleConfirmOk')}
        busy={busy}
        onCancel={() => setConfirmIdle(false)}
        onConfirm={() => {
          setConfirmIdle(false)
          setIdleEnabled(true)
          save({ experimentalFreeIdleProbeEnabled: true }, t('common.update'))
        }}
      />
    </>
  )
}

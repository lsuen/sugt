// 实验免费通道首启欢迎条：仅在真实配置满足条件时展示，可关闭并记住。
import React, { useEffect, useState } from 'react'
import { invoke } from './transport'
import { t } from './i18n'

const STORAGE_KEY = 'sugt_free_welcome_dismissed'
const FREE_IDS = new Set(['sugt-free', 'sugt-zen-free'])

type ProviderLite = {
  id: string
  name: string
  enabled: boolean
}

type Props = {
  /** 父组件已有 providers 时传入，避免再打 list_providers */
  providers?: ProviderLite[]
  /** 可选：跳到免费通道设置；未传则「打开设置」等同关闭 */
  onOpenFreeSettings?: () => void
}

export function FreeChannelWelcome({ providers: providersProp, onOpenFreeSettings }: Props) {
  const [dismissed, setDismissed] = useState(() => {
    try {
      return localStorage.getItem(STORAGE_KEY) === '1'
    } catch {
      return false
    }
  })
  const [accepted, setAccepted] = useState(false)
  const [providers, setProviders] = useState<ProviderLite[]>(providersProp ?? [])
  const [ready, setReady] = useState(false)

  useEffect(() => {
    if (providersProp) setProviders(providersProp)
  }, [providersProp])

  useEffect(() => {
    let cancelled = false
    void (async () => {
      try {
        const cfg = await invoke<{ experimental_free_disclaimer_accepted?: boolean }>('get_config')
        if (cancelled) return
        setAccepted(Boolean(cfg.experimental_free_disclaimer_accepted))
        if (!providersProp) {
          const list = await invoke<ProviderLite[]>('list_providers')
          if (cancelled) return
          setProviders(list)
        }
      } catch {
        if (!cancelled) setAccepted(false)
      } finally {
        if (!cancelled) setReady(true)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [providersProp])

  const freeProvider = providers.find((p) => FREE_IDS.has(p.id) && p.enabled)

  if (dismissed || !ready || !accepted || !freeProvider) return null

  const dismiss = () => {
    try {
      localStorage.setItem(STORAGE_KEY, '1')
    } catch {
      /* 隐私模式等写失败仍先隐藏本会话 */
    }
    setDismissed(true)
  }

  const openSettings = () => {
    if (!onOpenFreeSettings) {
      dismiss()
      return
    }
    onOpenFreeSettings()
    dismiss()
  }

  return (
    <div className="skill-git-notice" role="status">
      <span className="hint compact">
        <strong>{t('freeWelcome.title')}</strong>
        {freeProvider.name ? ` · ${freeProvider.name}` : ''}
        {' — '}
        {t('freeWelcome.body')}
      </span>
      <button type="button" className="tiny ghost" onClick={openSettings}>
        {t('freeWelcome.openSettings')}
      </button>
      <button type="button" className="tiny ghost" onClick={dismiss}>
        {t('freeWelcome.dismiss')}
      </button>
    </div>
  )
}

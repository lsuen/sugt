/** 「模型列表」tab 顶部接入信息（v1.1.11 R4）。
 * 作用：把本机网关的 `/v1/models` 地址和每条 baseurl / modelid 摊出来，一键复制去接客户端。
 * 数据来自后端 get_local_access_info（局域网 IP + 端口 + 各入口），失败只在本卡片内提示，不刷 toast。 */
import React, { useCallback, useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Copy } from 'lucide-react'
import { t } from './i18n'

type LocalAccessEntry = {
  label: string
  baseUrl: string
  modelId: string
}

type LocalAccessInfo = {
  host: string
  port: number
  lanHost: string
  modelsUrl: string
  entries: LocalAccessEntry[]
}

type Props = {
  pushToast: (msg: string, type?: 'ok' | 'error' | 'info') => void
}

/** 一行字段：标签 + 只读代码 + 复制按钮。 */
function CopyField({
  label,
  value,
  onCopy,
}: {
  label: string
  value: string
  onCopy: (text: string) => void
}) {
  return (
    <div className="access-info-field">
      <span className="access-info-field-label">{label}</span>
      <code className="access-info-code">{value}</code>
      <button
        type="button"
        className="tiny icon-only"
        aria-label={t('access.copy')}
        title={t('access.copy')}
        onClick={() => onCopy(value)}
      >
        <Copy size={14} />
      </button>
    </div>
  )
}

export function AccessInfoCard({ pushToast }: Props) {
  const [info, setInfo] = useState<LocalAccessInfo | null>(null)
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)

  const load = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    try {
      setInfo(await invoke<LocalAccessInfo>('get_local_access_info'))
    } catch (error) {
      setInfo(null)
      setLoadError(String(error))
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    void load()
  }, [load])

  // 主密钥在「接入密钥」页改完后后端会同步到 get_local_access_info，
  // 这里监听同一个事件重新拉取，保证卡片里的 key 提示不落后。
  useEffect(() => {
    const onAccessInfoChanged = () => void load()
    window.addEventListener('sugt:access-info-changed', onAccessInfoChanged)
    return () => window.removeEventListener('sugt:access-info-changed', onAccessInfoChanged)
  }, [load])

  const copy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text)
      pushToast(t('access.toastCopied'), 'ok')
    } catch (error) {
      pushToast(String(error), 'error')
    }
  }

  return (
    <section className="card access-info-card">
      <div className="access-info-head">
        <strong>{t('access.title')}</strong>
      </div>

      {loading && (
        <p className="access-info-note">{t('common.loading')}</p>
      )}

      {!loading && loadError && (
        <div className="access-info-error">
          <p className="error-text access-info-note">{t('access.loadFailed')}</p>
          <button
            type="button"
            className="ghost tiny-btn"
            onClick={() => void load()}
          >
            {t('access.retry')}
          </button>
        </div>
      )}

      {!loading && !loadError && info && (
        <>
          <div className="access-info-hint">
            <span>{t('access.modelsUrlHint', { url: '' }).trim()}</span>
            <code className="access-info-code">{info.modelsUrl}</code>
            <button
              type="button"
              className="tiny icon-only"
              aria-label={t('access.copy')}
              title={t('access.copy')}
              onClick={() => void copy(info.modelsUrl)}
            >
              <Copy size={14} />
            </button>
          </div>

          <ul className="access-info-list">
            {info.entries.map((entry, index) => (
              <li className="access-info-row" key={`${entry.label}-${index}`}>
                <span
                  className={
                    entry.label === 'sutai'
                      ? 'access-info-label is-primary'
                      : 'access-info-label'
                  }
                >
                  {entry.label}
                </span>
                <div className="access-info-fields">
                  <CopyField
                    label={t('access.baseUrl')}
                    value={entry.baseUrl}
                    onCopy={(text) => void copy(text)}
                  />
                  <CopyField
                    label={t('access.modelId')}
                    value={entry.modelId}
                    onCopy={(text) => void copy(text)}
                  />
                </div>
              </li>
            ))}
          </ul>
        </>
      )}
    </section>
  )
}

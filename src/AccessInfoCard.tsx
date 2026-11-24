/** 「模型列表」tab 顶部接入信息（v1.1.11 R4，2026-11-20 改版）。
 * 作用：把本机网关的 `/v1/models` 地址和每条 baseurl / modelid 摊出来，一键复制去接客户端；
 * 新增「请求」按钮现场验证可达性，下方盒子展示真实返回的模型 ID。
 * 数据来自后端 get_local_access_info / fetch_local_models，失败只在本卡片内提示，不刷 toast。 */
import React, { useCallback, useEffect, useState } from 'react'
import { invoke } from './transport';
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

type LocalModelsReport = {
  ok: boolean
  status: number | null
  elapsedMs: number
  modelIds: string[]
  error: string | null
}

type Props = {
  pushToast: (msg: string, type?: 'ok' | 'error' | 'info') => void
}

/** 结果盒子最多展开的模型 ID 数，超出折叠提示总数。 */
const MAX_SHOWN_MODEL_IDS = 50

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
  const [report, setReport] = useState<LocalModelsReport | null>(null)
  const [fetching, setFetching] = useState(false)

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

  const fetchModels = useCallback(async () => {
    setFetching(true)
    try {
      setReport(await invoke<LocalModelsReport>('fetch_local_models'))
    } catch (error) {
      setReport({ ok: false, status: null, elapsedMs: 0, modelIds: [], error: String(error) })
    } finally {
      setFetching(false)
    }
  }, [])

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
            <span className="access-info-field-label">{t('access.modelsUrl')}</span>
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
            <button
              type="button"
              className="ghost tiny-btn"
              disabled={fetching}
              onClick={() => void fetchModels()}
            >
              {fetching ? t('access.requesting') : t('access.request')}
            </button>
          </div>

          {fetching && <p className="access-info-note">{t('access.requesting')}</p>}

          {!fetching && report && report.ok && report.modelIds.length > 0 && (
            <div className="access-info-result">
              <p className="access-info-note">
                {t('access.resultSummary', {
                  status: report.status ?? 200,
                  count: report.modelIds.length,
                  elapsed: report.elapsedMs,
                })}
              </p>
              <ul className="access-info-chips">
                {report.modelIds.slice(0, MAX_SHOWN_MODEL_IDS).map((id) => (
                  <li className="access-info-chip" key={id}>
                    <code>{id}</code>
                  </li>
                ))}
              </ul>
              {report.modelIds.length > MAX_SHOWN_MODEL_IDS && (
                <p className="access-info-note">
                  {t('access.resultMore', {
                    count: report.modelIds.length,
                    shown: MAX_SHOWN_MODEL_IDS,
                  })}
                </p>
              )}
            </div>
          )}

          {!fetching && report && report.ok && report.modelIds.length === 0 && (
            <p className="access-info-note">{t('access.resultEmpty')}</p>
          )}

          {!fetching && report && !report.ok && (
            <div className="access-info-error">
              <p className="error-text access-info-note">
                {t('access.resultFailed')}
                {report.error ? `：${report.error}` : ''}
              </p>
              <button
                type="button"
                className="ghost tiny-btn"
                onClick={() => void fetchModels()}
              >
                {t('access.retry')}
              </button>
            </div>
          )}

          <ul className="access-info-list">
            {info.entries.map((entry, index) => (
              <li className="access-info-row" key={`${entry.label}-${index}`}>
                {/* 期望首项是统一入口，因为后端 build_local_access_info 把 CLIENT_MODEL_ID 置顶；
                    用位置而非品牌名判断高亮，默认 model id 变更时这里不会漏改。 */}
                <span
                  className={
                    index === 0
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

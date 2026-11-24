/**
 * 前端共享常量。期望值取自后端契约，避免多处写死同一份数据后产生漂移。
 */

/**
 * 期望后端 get_status 始终下发真实网关密钥；只有读配置失败时才用此展示兜底。
 * 因此这里不是可用密钥，真实认证一律以后端返回值为准。
 */
export const LOCAL_KEY_FALLBACK = 'sugt-local-key'

/**
 * 上游重试总预算默认毫秒数。对齐 src-tauri/src/presets.rs 的 DEFAULT_RETRY_BUDGET_MS，
 * 前端输入框占位由它生成，避免再写死一份 '480000'。
 */
export const DEFAULT_RETRY_BUDGET_MS = 480_000

/**
 * 上游重试单次默认上限毫秒数。对齐 src-tauri/src/presets.rs 的 DEFAULT_RETRY_SINGLE_MS。
 */
export const DEFAULT_RETRY_SINGLE_MS = 60_000

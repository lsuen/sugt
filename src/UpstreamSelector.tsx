import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Play, RefreshCw } from 'lucide-react';

type ProviderOption = {
  id: string;
  name: string;
  model_name: string;
  enabled: boolean;
  base_url: string;
  api_key: string;
  api_key_masked: string;
  protocol: 'openai' | 'anthropic' | 'open_ai';
  provider: string;
};

type Props = {
  providers: ProviderOption[];
  activeProviderId: string;
  busy: boolean;
  onActiveProviderChange: (id: string) => void;
  onRefresh: () => void;
  run: (action: () => Promise<unknown>, ok?: string) => Promise<void>;
  pushToast: (msg: string, type: 'ok' | 'error' | 'info') => void;
};

export function UpstreamSelector({
  providers,
  activeProviderId,
  busy,
  onActiveProviderChange,
  onRefresh,
  run,
  pushToast,
}: Props) {
  const [upstreamProviderId, setUpstreamProviderId] = useState('');
  const [models, setModels] = useState<string[]>([]);
  const [modelsBusy, setModelsBusy] = useState(false);
  const [modelsLoaded, setModelsLoaded] = useState(false);

  const enabledProviders = providers.filter((p) => p.enabled);
  const activeProvider = providers.find((p) => p.id === upstreamProviderId);

  // 同步外部 active_provider_id
  useEffect(() => {
    const preferred = activeProviderId || providers.find((p) => p.enabled)?.id || '';
    setUpstreamProviderId((prev) => {
      if (prev && providers.some((p) => p.id === prev && p.enabled)) return prev;
      return preferred;
    });
  }, [activeProviderId, providers]);

  const switchUpstream = async (id: string) => {
    setUpstreamProviderId(id);
    setModels([]);
    setModelsLoaded(false);
    onActiveProviderChange(id);
  };

  const fetchModels = async () => {
    const provider = providers.find((p) => p.id === upstreamProviderId);
    if (!provider) return;
    setModelsBusy(true);
    try {
      const result = await invoke<string[]>('list_provider_models', {
        input: {
          base_url: provider.base_url,
          api_key: provider.api_key,
          protocol: provider.protocol === 'open_ai' ? 'openai' : provider.protocol,
          vendor_id: null,
          auto_adapt_base_url: true,
        },
      });
      setModels(result);
      setModelsLoaded(true);
    } catch (e) {
      pushToast(String(e), 'error');
    } finally {
      setModelsBusy(false);
    }
  };

  const selectModel = async (model: string) => {
    const provider = providers.find((p) => p.id === upstreamProviderId);
    if (!provider) return;
    await run(
      () => invoke('save_provider', {
        input: {
          id: provider.id,
          name: provider.name,
          provider: provider.provider,
          base_url: provider.base_url,
          api_key: provider.api_key,
          model_name: model,
          model_alias: 'sutai',
          protocol: provider.protocol === 'open_ai' ? 'openai' : provider.protocol,
          enabled: provider.enabled,
          auto_adapt_base_url: true,
        },
      }),
      '模型已切换',
    );
    onRefresh();
  };

  const testCurrent = async () => {
    if (!upstreamProviderId) return;
    await run(() => invoke('test_provider', { id: upstreamProviderId }), '测试完成');
  };

  return (
    <>
      {/* 当前上游 */}
      <div className="access-field">
        <span className="access-label">当前上游</span>
        <select
          className="app-select upstream-select"
          value={upstreamProviderId}
          disabled={busy || enabledProviders.length === 0}
          onChange={(e) => void switchUpstream(e.target.value)}
        >
          {enabledProviders.length === 0 && <option value="">暂无已启用模型</option>}
          {enabledProviders.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name} · {p.model_name}
            </option>
          ))}
        </select>
      </div>

      {/* 模型 ID（上游实际模型） */}
      {upstreamProviderId && activeProvider && (
        <div className="access-field">
          <span className="access-label">模型 ID</span>
          <div className="access-value-row">
            {modelsLoaded ? (
              <select
                className="app-select access-model-select"
                value={activeProvider.model_name}
                disabled={modelsBusy}
                onChange={(e) => void selectModel(e.target.value)}
              >
                {models.map((m) => (
                  <option key={m} value={m}>{m}</option>
                ))}
              </select>
            ) : (
              <code className="access-code">{activeProvider.model_name}</code>
            )}
            <button type="button" className="tiny icon-only" disabled={modelsBusy} onClick={() => void fetchModels()} aria-label="获取模型列表">
              <RefreshCw size={14} className={modelsBusy ? 'spin' : ''} />
            </button>
            <button type="button" className="tiny icon-only" disabled={busy} onClick={() => void testCurrent()} aria-label="测试">
              <Play size={14} />
            </button>
          </div>
        </div>
      )}
    </>
  );
}

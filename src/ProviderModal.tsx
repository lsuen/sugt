import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Download, PlugZap, Save } from 'lucide-react';
import {
  applyVendorToForm,
  findVendorById,
  guessVendorFromForm,
  vendorBaseUrl,
  vendorSupportsModelList,
  VENDOR_DEFINITIONS,
  type ProviderForm,
  type ProviderProtocol,
} from './providerPresets';

type ProviderModalProps = {
  form: ProviderForm;
  busy: boolean;
  isEdit: boolean;
  onChange: (form: ProviderForm) => void;
  onClose: () => void;
  onSave: () => void;
  onError: (message: string) => void;
  onInfo?: (message: string) => void;
};

export function ProviderModal({
  form, busy, isEdit, onChange, onClose, onSave, onError, onInfo,
}: ProviderModalProps) {
  const [vendorId, setVendorId] = useState('');
  const [fetchedModels, setFetchedModels] = useState<string[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [testing, setTesting] = useState(false);
  const [useModelSelect, setUseModelSelect] = useState(false);

  useEffect(() => {
    setVendorId(guessVendorFromForm(form));
    setFetchedModels([]);
    setUseModelSelect(false);
  }, [form.id, isEdit]);

  const applyVendor = (id: string, protocol?: ProviderProtocol) => {
    const vendor = findVendorById(id);
    if (!vendor) return;
    const nextProtocol = protocol ?? form.protocol;
    const next = applyVendorToForm(vendor, nextProtocol, form);
    onChange(
      form.auto_adapt_base_url
        ? next
        : { ...next, base_url: form.base_url },
    );
    setFetchedModels([]);
    setUseModelSelect(false);
  };

  const onVendorChange = (id: string) => {
    setVendorId(id);
    if (id) applyVendor(id);
  };

  const onProtocolChange = (protocol: ProviderProtocol) => {
    if (vendorId) {
      const vendor = findVendorById(vendorId);
      if (vendor) {
        const suggested = vendorBaseUrl(vendor, protocol);
        onChange({
          ...form,
          protocol,
          base_url: form.auto_adapt_base_url && suggested ? suggested : form.base_url,
        });
        return;
      }
    }
    onChange({ ...form, protocol });
  };

  const draftInput = () => ({
    base_url: form.base_url,
    api_key: form.api_key.trim(),
    protocol: form.protocol,
    vendor_id: vendorId || null,
    auto_adapt_base_url: form.auto_adapt_base_url,
  });

  const fetchModels = async () => {
    const key = form.api_key.trim();
    if (!key) {
      onError('请先填写 API Key');
      return;
    }
    const vendor = vendorId ? findVendorById(vendorId) : undefined;
    if (vendor && !vendorSupportsModelList(vendor)) {
      onError('该服务商需手动填写 Model Name，暂无公开模型列表接口');
      return;
    }
    setModelsLoading(true);
    try {
      const models = await invoke<string[]>('list_provider_models', { input: draftInput() });
      setFetchedModels(models);
      setUseModelSelect(true);
      if (models.length > 0 && !form.model_name) {
        onChange({ ...form, model_name: models[0] });
      }
      onInfo?.(`已获取 ${models.length} 个模型`);
    } catch (error) {
      setFetchedModels([]);
      setUseModelSelect(false);
      throw error;
    } finally {
      setModelsLoading(false);
    }
  };

  const testConnection = async () => {
    if (!form.api_key.trim()) {
      onError('请先填写 API Key');
      return;
    }
    if (!form.base_url.trim()) {
      onError('请先填写 Base URL');
      return;
    }
    if (!form.model_name.trim()) {
      onError('请先填写 Model Name');
      return;
    }
    setTesting(true);
    try {
      const msg = await invoke<string>('test_provider_draft', {
        input: draftInput(),
        modelName: form.model_name.trim(),
      });
      onInfo?.(msg);
    } catch (error) {
      onError(String(error));
    } finally {
      setTesting(false);
    }
  };

  const vendor = vendorId ? findVendorById(vendorId) : undefined;
  const anthropicHint =
    form.protocol === 'anthropic' && vendor && !vendor.anthropicBaseUrl
      ? '该服务商未收录 Anthropic Base URL，请查阅官方文档手填'
      : undefined;

  return (
    <div className="modal-overlay">
      <div className="modal modal-wide" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>{isEdit ? '编辑模型' : '添加模型'}</h3>
          <button type="button" className="icon-btn" onClick={onClose} aria-label="关闭">
            ×
          </button>
        </div>
        <div className="modal-body">
          <label className="field-label">
            服务商预设
            <select
              className="app-select"
              value={vendorId}
              onChange={(e) => onVendorChange(e.target.value)}
            >
              <option value="">自定义 / 手动填写…</option>
              {VENDOR_DEFINITIONS.map((v) => (
                <option key={v.id} value={v.id}>{v.label}</option>
              ))}
            </select>
          </label>

          {vendor?.notes && (
            <p className="hint compact vendor-note">{vendor.notes}</p>
          )}

          <label className="field-label">
            配置名称
            <input
              value={form.name}
              onChange={(e) => onChange({ ...form, name: e.target.value })}
              placeholder="例如：魔搭 DeepSeek"
            />
          </label>

          <label className="field-label">
            服务商标识
            <input
              value={form.provider}
              onChange={(e) => onChange({ ...form, provider: e.target.value })}
              placeholder="ModelScope"
            />
          </label>

          <label className="field-label">
            协议类型
            <select
              className="app-select"
              value={form.protocol}
              onChange={(e) => onProtocolChange(e.target.value as ProviderProtocol)}
            >
              <option value="openai">OpenAI 兼容（Claude 自动转换，推荐）</option>
              <option value="anthropic">Anthropic 原生（base 通常不带 /v1）</option>
            </select>
            {anthropicHint && <span className="hint compact">{anthropicHint}</span>}
          </label>

          <label className="field-label">
            Base URL
            <input
              value={form.base_url}
              onChange={(e) => onChange({ ...form, base_url: e.target.value })}
              placeholder={
                form.protocol === 'openai'
                  ? 'https://api-inference.modelscope.cn/v1'
                  : 'https://api-inference.modelscope.cn'
              }
            />
            <span className="hint compact">
              {form.auto_adapt_base_url
                ? '已开启自适配：保存时自动补 /v1 或识别火山 /api/coding/v3 等路径'
                : '已关闭自适配：将按输入原样保存 Base URL'}
            </span>
          </label>

          <label className="switch-line">
            <span>Base URL 自适配</span>
            <input
              type="checkbox"
              checked={form.auto_adapt_base_url}
              onChange={(e) => onChange({ ...form, auto_adapt_base_url: e.target.checked })}
            />
          </label>

          <label className="field-label">
            API Key
            <input
              type="text"
              value={form.api_key}
              onChange={(e) => onChange({ ...form, api_key: e.target.value })}
              placeholder="ms-... / sk-..."
              autoComplete="off"
              spellCheck={false}
            />
          </label>

          <div className="field-label model-name-row">
            <span>Model Name</span>
            <div className="model-name-inputs">
              {useModelSelect && fetchedModels.length > 0 ? (
                <select
                  className="app-select"
                  value={form.model_name}
                  onChange={(e) => onChange({ ...form, model_name: e.target.value })}
                >
                  {fetchedModels.map((id) => (
                    <option key={id} value={id}>{id}</option>
                  ))}
                </select>
              ) : (
                <input
                  value={form.model_name}
                  onChange={(e) => onChange({ ...form, model_name: e.target.value })}
                  placeholder="手填或点击获取模型列表"
                />
              )}
              <button
                type="button"
                className="ghost tiny-btn"
                disabled={
                  busy
                  || modelsLoading
                  || !form.api_key.trim()
                  || (vendor && !vendorSupportsModelList(vendor))
                }
                title={
                  vendor && !vendorSupportsModelList(vendor)
                    ? '该服务商需手填 Model Name'
                    : '调用官方模型列表接口'
                }
                onClick={() => fetchModels().catch((err) => onError(String(err)))}
              >
                <Download size={14} />
                {modelsLoading ? '获取中…' : '获取模型'}
              </button>
            </div>
          </div>

          <label className="switch-line">
            <span>启用</span>
            <input
              type="checkbox"
              checked={form.enabled}
              onChange={(e) => onChange({ ...form, enabled: e.target.checked })}
            />
          </label>

          <div className="modal-actions modal-actions-spread">
            <button
              type="button"
              className="ghost"
              disabled={busy || testing || !form.api_key.trim() || !form.model_name.trim()}
              onClick={() => void testConnection()}
            >
              <PlugZap size={16} />
              {testing ? '测试中…' : '测试连接'}
            </button>
            <div className="modal-actions-right">
              <button type="button" className="ghost" onClick={onClose}>取消</button>
              <button type="button" className="primary" disabled={busy} onClick={onSave}>
                <Save size={16} />保存
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

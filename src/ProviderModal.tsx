import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Download, Save } from 'lucide-react';
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
};

export function ProviderModal({ form, busy, isEdit, onChange, onClose, onSave, onError }: ProviderModalProps) {
  const [vendorId, setVendorId] = useState('');
  const [fetchedModels, setFetchedModels] = useState<string[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
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
    onChange(applyVendorToForm(vendor, nextProtocol, form));
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
          base_url: suggested ?? form.base_url,
        });
        return;
      }
    }
    onChange({ ...form, protocol });
  };

  const fetchModels = async () => {
    const key = form.api_key.trim();
    if (!key || key.includes('****')) {
      onError('请先填写完整 API Key');
      return;
    }
    const vendor = vendorId ? findVendorById(vendorId) : undefined;
    if (vendor && !vendorSupportsModelList(vendor)) {
      onError('该服务商需手动填写 Model Name，暂无公开模型列表接口');
      return;
    }
    setModelsLoading(true);
    try {
      const models = await invoke<string[]>('list_provider_models', {
        input: {
          base_url: form.base_url,
          api_key: key,
          protocol: form.protocol,
          vendor_id: vendorId || null,
        },
      });
      setFetchedModels(models);
      setUseModelSelect(true);
      if (models.length > 0 && !form.model_name) {
        onChange({ ...form, model_name: models[0] });
      }
    } catch (error) {
      setFetchedModels([]);
      setUseModelSelect(false);
      throw error;
    } finally {
      setModelsLoading(false);
    }
  };

  const vendor = vendorId ? findVendorById(vendorId) : undefined;
  const anthropicHint =
    form.protocol === 'anthropic' && vendor && !vendor.anthropicBaseUrl
      ? '该服务商未收录 Anthropic Base URL，请查阅官方文档手填'
      : undefined;

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
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
          </label>

          <label className="field-label">
            API Key
            <input
              type="password"
              value={form.api_key}
              onChange={(e) => onChange({ ...form, api_key: e.target.value })}
              placeholder={isEdit ? '留空则不修改' : 'ms-... / sk-...'}
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
                  || form.api_key.includes('****')
                  || (vendor && !vendorSupportsModelList(vendor))
                }
                title={
                  form.api_key.includes('****')
                    ? '编辑时请重新输入完整 API Key'
                    : vendor && !vendorSupportsModelList(vendor)
                      ? '该服务商需手填 Model Name'
                      : '调用官方模型列表接口'
                }
                onClick={() => fetchModels().catch((err) => onError(String(err)))}
              >
                <Download size={14} />
                {modelsLoading ? '获取中…' : '获取模型'}
              </button>
            </div>
            <span className="hint compact">
              详见 docs/provider-vendors.md · Anthropic 协议下仍用 OpenAI 列表接口（若有）
            </span>
          </div>

          <label className="switch-line">
            <span>启用</span>
            <input
              type="checkbox"
              checked={form.enabled}
              onChange={(e) => onChange({ ...form, enabled: e.target.checked })}
            />
          </label>

          <div className="modal-actions">
            <button type="button" className="ghost" onClick={onClose}>取消</button>
            <button type="button" className="primary" disabled={busy} onClick={onSave}>
              <Save size={16} />保存
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

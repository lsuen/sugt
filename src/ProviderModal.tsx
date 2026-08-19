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
import { t } from './i18n';

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
      onError(t('pm.errorApiKey'));
      return;
    }
    const vendor = vendorId ? findVendorById(vendorId) : undefined;
    if (vendor && !vendorSupportsModelList(vendor)) {
      onError(t('pm.errorModelList'));
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
      onInfo?.(t('pm.fetchedModels', { n: models.length }));
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
      onError(t('pm.errorApiKey'));
      return;
    }
    if (!form.base_url.trim()) {
      onError(t('pm.errorBaseUrl'));
      return;
    }
    if (!form.model_name.trim()) {
      onError(t('pm.errorModelName'));
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
      ? t('pm.anthropicHint')
      : undefined;

  return (
    <div className="modal-overlay">
      <div className="modal modal-wide" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>{isEdit ? t('pm.editTitle') : t('pm.addTitle')}</h3>
          <button type="button" className="icon-btn" onClick={onClose} aria-label={t('common.close')}>
            ×
          </button>
        </div>
        <div className="modal-body">
          <label className="field-label">
            {t('pm.vendorPreset')}
            <select
              className="app-select"
              value={vendorId}
              onChange={(e) => onVendorChange(e.target.value)}
            >
              <option value="">{t('pm.customManual')}</option>
              {VENDOR_DEFINITIONS.map((v) => (
                <option key={v.id} value={v.id}>{v.label}</option>
              ))}
            </select>
          </label>

          {vendor?.notes && (
            <p className="hint compact vendor-note">{vendor.notes}</p>
          )}

          <label className="field-label">
            {t('pm.nameLabel')}
            <input
              value={form.name}
              onChange={(e) => onChange({ ...form, name: e.target.value })}
              placeholder={t('pm.namePlaceholder')}
            />
          </label>

          <label className="field-label">
            {t('pm.providerLabel')}
            <input
              value={form.provider}
              onChange={(e) => onChange({ ...form, provider: e.target.value })}
              placeholder={t('pm.providerPlaceholder')}
            />
          </label>

          <label className="field-label">
            {t('pm.protocolLabel')}
            <select
              className="app-select"
              value={form.protocol}
              onChange={(e) => onProtocolChange(e.target.value as ProviderProtocol)}
            >
              <option value="openai">{t('pm.openaiCompatDesc')}</option>
              <option value="anthropic">{t('pm.anthropicNativeDesc')}</option>
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
                ? t('pm.autoAdaptOn')
                : t('pm.autoAdaptOff')}
            </span>
          </label>

          <label className="switch-line">
            <span>{t('pm.baseUrlAutoAdapt')}</span>
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
              placeholder={t('pm.apiKeyPlaceholder')}
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
                  placeholder={t('pm.modelNamePlaceholder')}
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
                    ? t('pm.modelListTitle')
                    : t('pm.fetchModelListTitle')
                }
                onClick={() => fetchModels().catch((err) => onError(String(err)))}
              >
                <Download size={14} />
                {modelsLoading ? t('pm.fetchingModels') : t('pm.fetchModels')}
              </button>
            </div>
          </div>

          <label className="switch-line">
            <span>{t('pm.enabled')}</span>
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
              {testing ? t('common.testing') : t('common.testConnection')}
            </button>
            <div className="modal-actions-right">
              <button type="button" className="ghost" onClick={onClose}>{t('common.cancel')}</button>
              <button type="button" className="primary" disabled={busy} onClick={onSave}>
                <Save size={16} />{t('common.save')}
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

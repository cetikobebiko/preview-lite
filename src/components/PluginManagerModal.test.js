import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/svelte';
import PluginManagerModal from './PluginManagerModal.svelte';
import * as pluginStore from '../pluginStore.js';
import * as pluginBackend from '../services/backend/plugins.js';

vi.mock('../services/backend/plugins.js', () => ({
  listInstalledPlugins: vi.fn(),
  inspectPluginPackage: vi.fn(),
  installPluginPackage: vi.fn(),
  grantPluginConsent: vi.fn(),
  uninstallPlugin: vi.fn(),
  setPluginEnabled: vi.fn(),
  startPluginExecution: vi.fn(),
  cancelPluginExecution: vi.fn(),
  getPluginExecutionStatus: vi.fn(),
  getPluginFeatureStatus: vi.fn(),
  addBatchJob: vi.fn(),
}));

const basePlugin = {
  id: 'hello-log',
  name: 'Hello Log',
  version: '1.0.0',
  description: 'Örnek',
  permissions: ['plugin:log', 'ui:notification'],
  hooks: ['on_custom_command', 'on_document_open'],
  enabled: true,
  consent_granted: true,
  consent_policy_version: 1,
  reviewed_package_digest: 'abc',
};

describe('PluginManagerModal Svelte 5 component', () => {
  const originalAnimate = Element.prototype.animate;

  const mockCandidate = {
    package_path: '/dummy/plugin-package',
    package_digest: '1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef',
    manifest: {
      id: 'com.example.sample',
      name: 'Sample Plugin',
      version: '1.0.0',
      description: 'A sample plugin for testing',
      permissions: ['plugin:log', 'ui:notification'],
      hooks: ['on_custom_command'],
    },
  };

  beforeEach(() => {
    vi.clearAllMocks();
    Element.prototype.animate = vi.fn(() => ({
      finished: Promise.resolve(),
      cancel: vi.fn(),
      onfinish: null,
    }));
    pluginStore.installedPlugins.set([]);
    pluginStore.inspectionCandidate.set(null);
    pluginStore.activeExecutions.set({});
    pluginStore.pluginExecutionHistory.set([]);
    pluginStore.pluginBackendGate.set({ checked: false, enabled: false, buildProfile: null });
    vi.stubEnv('DEV', true);
    pluginBackend.getPluginFeatureStatus.mockResolvedValue({ enabled: true, build_profile: 'development' });
    pluginBackend.listInstalledPlugins.mockImplementation(async () => [...currentPlugins]);
    currentPlugins = [];
  });

  let currentPlugins = [];

  afterEach(() => {
    Element.prototype.animate = originalAnimate;
    vi.unstubAllEnvs();
  });

  it('renders modal header and FeatureNotice', () => {
    const { container } = render(PluginManagerModal, { props: { onclose: vi.fn() } });
    expect(screen.getByText('Eklenti Yöneticisi (Plugin Manager)')).toBeTruthy();
    expect(screen.getByText('Yeni Eklenti Paketi Seç...')).toBeTruthy();
  });

  it('displays candidate inspection details, unsigned warning, and disables install button until consent is checked', async () => {
    pluginStore.inspectionCandidate.set(mockCandidate);

    const { container } = render(PluginManagerModal, { props: { onclose: vi.fn() } });

    expect(screen.getByText('Eklenti İnceleme & Güvenlik Onayı')).toBeTruthy();
    expect(screen.getByText(/İmzasız Eklenti Uyarısı/)).toBeTruthy();
    expect(screen.getByText(/Sample Plugin/)).toBeTruthy();
    expect(screen.getByText(/com\.example\.sample/)).toBeTruthy();
    expect(screen.getByText(/Eklenti yürütme günlüklerini ana uygulamaya iletir/)).toBeTruthy();
    expect(screen.getByText(/Kullanıcı arayüzünde bilgilendirme bildirimi gösterir/)).toBeTruthy();

    const installBtn = screen.getByText('İzinleri Onayla ve Yükle');
    expect(installBtn.hasAttribute('disabled')).toBe(true);
    // Install stays disabled until the backend gate answers open.
    await waitFor(() => expect(pluginBackend.getPluginFeatureStatus).toHaveBeenCalled());

    const checkbox = document.querySelector('#plugin-consent-acknowledgement');
    expect(checkbox).toBeTruthy();

    await fireEvent.click(checkbox);
    expect(installBtn.hasAttribute('disabled')).toBe(false);

    await fireEvent.click(checkbox);
    expect(installBtn.hasAttribute('disabled')).toBe(true);
  });

  it('warns and blocks privilege-increasing actions when the backend gate is closed', async () => {
    pluginBackend.getPluginFeatureStatus.mockResolvedValue({ enabled: false, build_profile: 'production' });
    currentPlugins = [{ ...basePlugin, enabled: false }];
    render(PluginManagerModal, { props: { onclose: vi.fn() } });

    const banner = await screen.findByTestId('plugin-backend-gate-mismatch');
    expect(banner.textContent).toMatch(/arka uç bu derlemede kapalı/);
    await screen.findByText('Etkinleştir');
    expect(screen.getByText('Yeni Eklenti Paketi Seç...').hasAttribute('disabled')).toBe(true);
    expect(screen.getByText('Etkinleştir').hasAttribute('disabled')).toBe(true);
    // Restrictive action stays available
    expect(screen.getByText('Kaldır').hasAttribute('disabled')).toBe(false);
  });

  it('runs the selected hook with a payload generated from its schema', async () => {
    currentPlugins = [{ ...basePlugin, hooks: ['on_custom_command', 'on_document_open', 'on_export_complete'] }];
    pluginBackend.startPluginExecution.mockResolvedValue({ execution_id: 'x', plugin_id: 'hello-log', state: 'RUNNING' });
    pluginBackend.getPluginExecutionStatus.mockResolvedValue({ state: 'COMPLETED', result: { notifications: [], logs: [], warnings: [], text: null }, error: null });
    render(PluginManagerModal, { props: { onclose: vi.fn() } });

    const runCustom = await screen.findByText('Çalıştır (on_custom_command)');
    await waitFor(() => expect(runCustom.hasAttribute('disabled')).toBe(false));
    await fireEvent.click(runCustom);
    await waitFor(() => expect(pluginBackend.startPluginExecution).toHaveBeenCalledTimes(1));
    let [pluginId, hook, payload] = pluginBackend.startPluginExecution.mock.calls[0];
    expect(pluginId).toBe('hello-log');
    expect(hook).toBe('on_custom_command');
    expect(JSON.parse(payload)).toEqual({ schema_version: 1 });

    // Switch to on_export_complete: fields come from the schema and are editable.
    await fireEvent.change(screen.getByLabelText('Kanca'), { target: { value: 'on_export_complete' } });
    await fireEvent.change(screen.getByLabelText('format'), { target: { value: 'docx' } });
    await fireEvent.input(screen.getByLabelText('page_count'), { target: { value: '12' } });
    await fireEvent.click(screen.getByText('Çalıştır (on_export_complete)'));
    await waitFor(() => expect(pluginBackend.startPluginExecution).toHaveBeenCalledTimes(2));
    [pluginId, hook, payload] = pluginBackend.startPluginExecution.mock.calls[1];
    expect(hook).toBe('on_export_complete');
    expect(JSON.parse(payload)).toEqual({ schema_version: 1, format: 'docx', page_count: 12 });
  });

  it('refuses an edited payload that fails the schema without calling the backend', async () => {
    currentPlugins = [basePlugin];
    render(PluginManagerModal, { props: { onclose: vi.fn() } });
    await screen.findByText('Çalıştır (on_custom_command)');
    await fireEvent.change(screen.getByLabelText('Kanca'), { target: { value: 'on_document_open' } });
    await fireEvent.input(screen.getByLabelText('page_count'), { target: { value: '0' } });
    const run = screen.getByText('Çalıştır (on_document_open)');
    await waitFor(() => expect(run.hasAttribute('disabled')).toBe(false));
    await fireEvent.click(run);
    expect(await screen.findByText(/Kanca yükü geçersiz: 'page_count'/)).toBeTruthy();
    expect(pluginBackend.startPluginExecution).not.toHaveBeenCalled();
  });

  it('shows a rejected manual run error in the modal', async () => {
    currentPlugins = [basePlugin];
    pluginBackend.startPluginExecution.mockRejectedValue({ code: 'PLUGIN_PAYLOAD_INVALID', message: 'bilinmeyen alan' });
    render(PluginManagerModal, { props: { onclose: vi.fn() } });
    const run = await screen.findByText('Çalıştır (on_custom_command)');
    await waitFor(() => expect(run.hasAttribute('disabled')).toBe(false));
    await fireEvent.click(run);
    expect((await screen.findAllByText(/Kanca yükü kancanın V1 şemasına uymuyor/)).length).toBeGreaterThan(0);
  });

  it('lists recent executions with notifications, warnings, logs and text', async () => {
    currentPlugins = [basePlugin];
    pluginStore.pluginExecutionHistory.set([{
      execution_id: 'e1',
      plugin_id: 'hello-log',
      plugin_name: 'Hello Log',
      hook_name: 'on_document_open',
      origin: 'auto',
      state: 'COMPLETED',
      result: {
        schema_version: 1,
        execution_id: 'e1',
        text: 'hello-log tamam',
        logs: ['hello-log: on_document_open page_count=3'],
        notifications: ['hello-log: belge açıldı, 3 sayfa.'],
        warnings: ['dikkat'],
      },
      error: null,
    }]);
    render(PluginManagerModal, { props: { onclose: vi.fn() } });

    expect(await screen.findByText('Son yürütmeler')).toBeTruthy();
    expect(screen.getByText('hello-log: belge açıldı, 3 sayfa.')).toBeTruthy();
    expect(screen.getByText('hello-log: on_document_open page_count=3')).toBeTruthy();
    expect(screen.getByText('dikkat')).toBeTruthy();
    expect(screen.getByText('hello-log tamam')).toBeTruthy();
    expect(screen.getByText(/Otomatik/)).toBeTruthy();
    expect(screen.getByText('Tamamlandı')).toBeTruthy();
  });

  it('maps a failed execution error code to a Turkish message in the list', async () => {
    currentPlugins = [basePlugin];
    pluginStore.pluginExecutionHistory.set([{
      execution_id: 'e2',
      plugin_id: 'hello-log',
      plugin_name: 'Hello Log',
      hook_name: 'on_custom_command',
      origin: 'auto',
      state: 'FAILED',
      result: null,
      error: { code: 'PLUGIN_PERMISSION_DENIED', message: "'ui_notify' çağrısı için izin gerekli" },
    }, {
      execution_id: 'e3',
      plugin_id: 'hello-log',
      plugin_name: 'Hello Log',
      hook_name: 'on_document_open',
      origin: 'auto',
      state: 'DISCARDED',
      result: null,
      error: null,
    }]);
    render(PluginManagerModal, { props: { onclose: vi.fn() } });
    expect(await screen.findByText(/İzin reddedildi: eklenti onaylanmamış bir host API'ı çağırdı/)).toBeTruthy();
    expect(screen.getByText('Atıldı (belge değişti)')).toBeTruthy();
  });

  it('marks a quarantined plugin and does not offer enable', async () => {
    currentPlugins = [{ ...basePlugin, enabled: false, consent_granted: false, consent_policy_version: 0 }];
    render(PluginManagerModal, { props: { onclose: vi.fn() } });
    expect(await screen.findByText('Karantinada')).toBeTruthy();
    expect(screen.getByText(/onaydan sonra değişti/)).toBeTruthy();
    expect(screen.getByText('Etkinleştir').hasAttribute('disabled')).toBe(true);
    expect(screen.queryByText(/Çalıştır \(/)).toBeNull();
  });
});

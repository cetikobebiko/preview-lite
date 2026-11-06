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

  it('offers one run button per declared hook and sends a schema-valid sample payload', async () => {
    currentPlugins = [basePlugin];
    pluginBackend.startPluginExecution.mockResolvedValue({ execution_id: 'x', plugin_id: 'hello-log', state: 'RUNNING' });
    render(PluginManagerModal, { props: { onclose: vi.fn() } });

    const runCustom = await screen.findByText('Çalıştır (on_custom_command)');
    expect(screen.getByText('Çalıştır (on_document_open)')).toBeTruthy();
    await waitFor(() => expect(runCustom.hasAttribute('disabled')).toBe(false));
    await fireEvent.click(runCustom);
    await waitFor(() => expect(pluginBackend.startPluginExecution).toHaveBeenCalled());
    const [pluginId, hook, payload] = pluginBackend.startPluginExecution.mock.calls[0];
    expect(pluginId).toBe('hello-log');
    expect(hook).toBe('on_custom_command');
    expect(JSON.parse(payload)).toEqual({ command_name: 'manual_run', parameters: {} });
  });

  it('shows notifications, logs and text of a completed execution', async () => {
    currentPlugins = [basePlugin];
    pluginStore.activeExecutions.set({
      e1: {
        execution_id: 'e1',
        plugin_id: 'hello-log',
        hook_name: 'on_custom_command',
        state: 'COMPLETED',
        result: {
          schema_version: 1,
          execution_id: 'e1',
          text: 'hello-log tamam',
          logs: ['hello-log: on_custom_command {}'],
          notifications: ['Merhaba! hello-log eklentisi çalıştı.'],
          warnings: [],
        },
        error: null,
      },
    });
    render(PluginManagerModal, { props: { onclose: vi.fn() } });

    expect(await screen.findByText('Merhaba! hello-log eklentisi çalıştı.')).toBeTruthy();
    expect(screen.getByText('hello-log: on_custom_command {}')).toBeTruthy();
    expect(screen.getByText('hello-log tamam')).toBeTruthy();
  });

  it('maps a failed execution error code to a Turkish message', async () => {
    currentPlugins = [basePlugin];
    pluginStore.activeExecutions.set({
      e2: {
        execution_id: 'e2',
        plugin_id: 'hello-log',
        hook_name: 'on_custom_command',
        state: 'FAILED',
        result: null,
        error: { code: 'PLUGIN_PERMISSION_DENIED', message: "'ui_notify' çağrısı için izin gerekli" },
      },
    });
    render(PluginManagerModal, { props: { onclose: vi.fn() } });
    expect(await screen.findByText(/İzin reddedildi: eklenti onaylanmamış bir host API'ı çağırdı/)).toBeTruthy();
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

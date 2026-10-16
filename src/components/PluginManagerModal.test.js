import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import PluginManagerModal from './PluginManagerModal.svelte';
import * as pluginStore from '../pluginStore.js';
import * as featureRegistry from '../featureRegistry.js';

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
  });

  afterEach(() => {
    Element.prototype.animate = originalAnimate;
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

    const checkbox = document.querySelector('#plugin-consent-acknowledgement');
    expect(checkbox).toBeTruthy();

    await fireEvent.click(checkbox);
    expect(installBtn.hasAttribute('disabled')).toBe(false);

    await fireEvent.click(checkbox);
    expect(installBtn.hasAttribute('disabled')).toBe(true);
  });
});

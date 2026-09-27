import { describe, it, expect, vi, beforeEach } from 'vitest';
import { get } from 'svelte/store';
import {
  installedPlugins,
  inspectionCandidate,
  activeExecutions,
  refreshInstalledPlugins,
  inspectPlugin,
  installPlugin,
  grantConsent,
  uninstallPlugin,
  togglePluginEnabled,
  startPluginHook,
  cancelPlugin,
  pollExecutionStatus,
} from './pluginStore.js';
import * as pluginBackend from './services/backend/plugins.js';
import * as featureRegistry from './featureRegistry.js';
import { FeatureUnavailableError } from './featureActionGuard.js';

vi.mock('./services/backend/plugins.js', () => ({
  listInstalledPlugins: vi.fn(),
  inspectPluginPackage: vi.fn(),
  installPluginPackage: vi.fn(),
  grantPluginConsent: vi.fn(),
  uninstallPlugin: vi.fn(),
  setPluginEnabled: vi.fn(),
  startPluginExecution: vi.fn(),
  cancelPluginExecution: vi.fn(),
  getPluginExecutionStatus: vi.fn(),
  addBatchJob: vi.fn(),
}));

describe('pluginStore & frontend actions', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    installedPlugins.set([]);
    inspectionCandidate.set(null);
    activeExecutions.set({});
  });

  it('fails with FeatureUnavailableError when plugin system is disabled by default in featureRegistry', async () => {
    await expect(refreshInstalledPlugins()).rejects.toThrow(FeatureUnavailableError);
    await expect(inspectPlugin('/some/path')).rejects.toThrow(FeatureUnavailableError);
    await expect(installPlugin('/some/path', 'digest123')).rejects.toThrow(FeatureUnavailableError);
    await expect(grantConsent('p1', 'digest123', [])).rejects.toThrow(FeatureUnavailableError);
    await expect(uninstallPlugin('p1')).rejects.toThrow(FeatureUnavailableError);
    await expect(togglePluginEnabled('p1', true)).rejects.toThrow(FeatureUnavailableError);
    await expect(startPluginHook('p1', 'on_custom_command')).rejects.toThrow(FeatureUnavailableError);
    await expect(cancelPlugin('exec-1')).rejects.toThrow(FeatureUnavailableError);
    await expect(pollExecutionStatus('exec-1')).rejects.toThrow(FeatureUnavailableError);
  });

  describe('with enabled plugin feature context in test harness', () => {
    beforeEach(() => {
      vi.spyOn(featureRegistry, 'getFeaturePresentation').mockReturnValue({
        featureId: 'plugin_system',
        status: 'stable',
        badge: '',
        runnable: true,
        visible: true,
        warning: '',
        reason: '',
      });
    });

    it('inspectPlugin retains package_path and updates inspectionCandidate store', async () => {
      pluginBackend.inspectPluginPackage.mockResolvedValue({
        manifest: {
          id: 'test-plugin',
          name: 'Test Plugin',
          version: '1.0.0',
          permissions: ['plugin:log'],
          hooks: ['on_custom_command'],
        },
        package_digest: 'abcd1234efgh',
      });

      const res = await inspectPlugin('/path/to/my-plugin');
      expect(res.package_path).toBe('/path/to/my-plugin');
      expect(get(inspectionCandidate)).toEqual({
        manifest: {
          id: 'test-plugin',
          name: 'Test Plugin',
          version: '1.0.0',
          permissions: ['plugin:log'],
          hooks: ['on_custom_command'],
        },
        package_digest: 'abcd1234efgh',
        package_path: '/path/to/my-plugin',
      });
    });

    it('installPlugin invokes install backend, clears candidate, and refreshes list', async () => {
      pluginBackend.installPluginPackage.mockResolvedValue({
        plugin_id: 'test-plugin',
        version: '1.0.0',
        package_digest: 'abcd1234efgh',
      });
      pluginBackend.listInstalledPlugins.mockResolvedValue([
        {
          id: 'test-plugin',
          name: 'Test Plugin',
          version: '1.0.0',
          enabled: false,
        },
      ]);

      inspectionCandidate.set({ package_path: '/path/to/my-plugin' });
      await installPlugin('/path/to/my-plugin', 'abcd1234efgh');

      expect(pluginBackend.installPluginPackage).toHaveBeenCalledWith('/path/to/my-plugin', 'abcd1234efgh');
      expect(get(inspectionCandidate)).toBeNull();
      expect(get(installedPlugins)).toHaveLength(1);
    });

    it('grantConsent invokes grant consent backend and refreshes list', async () => {
      pluginBackend.grantPluginConsent.mockResolvedValue({
        plugin_id: 'test-plugin',
        consent_policy_version: 1,
      });
      pluginBackend.listInstalledPlugins.mockResolvedValue([
        {
          id: 'test-plugin',
          name: 'Test Plugin',
          version: '1.0.0',
          enabled: false,
          consent_policy_version: 1,
        },
      ]);

      await grantConsent('test-plugin', 'abcd1234efgh', ['plugin:log']);
      expect(pluginBackend.grantPluginConsent).toHaveBeenCalledWith('test-plugin', 'abcd1234efgh', ['plugin:log']);
      expect(get(installedPlugins)).toHaveLength(1);
    });

    it('startPluginHook generates UUID executionId, invokes backend, and tracks active executions', async () => {
      pluginBackend.startPluginExecution.mockResolvedValue({
        execution_id: 'mock-uuid',
        plugin_id: 'test-plugin',
        hook_name: 'on_custom_command',
        state: 'RUNNING',
      });

      await startPluginHook('test-plugin', 'on_custom_command', '{"test":true}');
      expect(pluginBackend.startPluginExecution).toHaveBeenCalledWith(
        'test-plugin',
        'on_custom_command',
        '{"test":true}',
        expect.any(String),
      );
      const executions = get(activeExecutions);
      expect(Object.keys(executions)).toHaveLength(1);
      const firstExec = Object.values(executions)[0];
      expect(firstExec.plugin_id).toBe('test-plugin');
      expect(firstExec.hook_name).toBe('on_custom_command');
      expect(firstExec.state).toBe('RUNNING');
    });
  });
});

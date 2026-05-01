import { invoke } from '@tauri-apps/api/core';
import { useSettingsStore } from '../stores/settings';

export interface VcpToolboxResponse<T = any> {
  ok: boolean;
  status: number;
  data: T;
}

export interface VcpToolboxCallOptions {
  method?: 'GET' | 'POST' | 'DELETE';
  path: string;
  body?: unknown;
  adminUsername?: string;
  adminPassword?: string;
}

export const callVcpToolboxApi = async <T = any>(
  options: VcpToolboxCallOptions,
): Promise<VcpToolboxResponse<T>> => {
  const settingsStore = useSettingsStore();
  if (!settingsStore.settings) {
    await settingsStore.fetchSettings();
  }

  const settings = settingsStore.settings;
  if (!settings?.vcpServerUrl) {
    throw new Error('请先在核心连接里配置 VCP 服务器 URL');
  }

  const response = await invoke<VcpToolboxResponse<T>>('call_vcp_toolbox_api', {
    request: {
      vcpUrl: settings.vcpServerUrl,
      vcpApiKey: settings.vcpApiKey || '',
      method: options.method || 'GET',
      path: options.path,
      body: options.body ?? null,
      adminUsername: options.adminUsername || settings.adminUsername || '',
      adminPassword: options.adminPassword || settings.adminPassword || '',
    },
  });

  if (!response.ok) {
    const message =
      typeof response.data === 'object' && response.data && 'error' in response.data
        ? String((response.data as any).error)
        : `HTTP ${response.status}`;
    throw new Error(message);
  }

  return response;
};

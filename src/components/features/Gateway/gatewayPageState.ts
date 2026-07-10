import { invoke } from '@tauri-apps/api/core'
import { getPrimaryClientApiKey, parseAllowedIps, parseClientApiKeys } from './gatewayPageUtils'

export interface GatewayConfig {
  enabled: boolean;
  host: string;
  port: number;
  apiKey: string;
  clientApiKeysText: string;
  region: string;
  accountMode: string;
  accountId: string | null;
  groupId: string | null;
  poolAccountIds: string[];
  strategy: string;
  threshold: number;
  localOnly: boolean;
  allowedIpsText: string;
  logLevel: string;
  modelMappings: ModelMappingRule[];
  filterClaudeCode: boolean;
  filterStripBoundaries: boolean;
  filterEnvNoise: boolean;
  promptFilterRules: PromptFilterRule[];
  logRequests: boolean;
  responseCacheEnabled: boolean;
  responseCacheTtl: number;
  promptCacheTargetPercent: number;
  promptCacheTtlSecs: number;
  promptCacheMaxEntries: number;
  promptCacheIgnoreClientControl: boolean;
}

export interface ModelMappingRule {
  id: string;
  name: string;
  enabled: boolean;
  ruleType: string;
  sourceModel: string;
  targetModels: string[];
  weights: number[];
}

export interface PromptFilterRule {
  id: string;
  name: string;
  enabled: boolean;
  ruleType: string;
  matchPattern: string;
  replace: string;
}

export interface GatewayStatus {
  running: boolean;
  host: string;
  port: number;
  requestCount: number;
  lastError: string | null;
  runtimeConfig: GatewayConfig | null;
}

const normalizePromptCacheTargetPercent = (value: unknown) => {
  const percent = Number(value)
  if (!Number.isFinite(percent)) {
    return 90
  }
  return Math.min(100, Math.max(0, Math.round(percent)))
}

const normalizePromptCacheTtlSecs = (value: unknown) => {
  const secs = Number(value)
  if (!Number.isFinite(secs)) {
    return 300
  }
  return Math.min(3600, Math.max(30, Math.round(secs)))
}

const normalizePromptCacheMaxEntries = (value: unknown) => {
  const n = Number(value)
  if (!Number.isFinite(n)) {
    return 2000
  }
  return Math.max(1, Math.round(n))
}

export const DEFAULT_GATEWAY_CONFIG: GatewayConfig = {
  enabled: false,
  host: '127.0.0.1',
  port: 8765,
  apiKey: '',
  clientApiKeysText: '',
  region: 'us-east-1',
  accountMode: 'single',
  accountId: null,
  groupId: null,
  poolAccountIds: [],
  strategy: 'round_robin',
  threshold: 90,
  localOnly: true,
  allowedIpsText: '',
  logLevel: 'debug',
  modelMappings: [],
  filterClaudeCode: false,
  filterStripBoundaries: false,
  filterEnvNoise: false,
  promptFilterRules: [],
  logRequests: true,
  responseCacheEnabled: true,
  responseCacheTtl: 180,
  promptCacheTargetPercent: 90,
  promptCacheTtlSecs: 300,
  promptCacheMaxEntries: 2000,
  promptCacheIgnoreClientControl: false
}

export const DEFAULT_GATEWAY_STATUS: GatewayStatus = {
  running: false,
  host: '127.0.0.1',
  port: 8765,
  requestCount: 0,
  lastError: null,
  runtimeConfig: null
}

export const buildGatewayConfigSnapshot = (config: GatewayConfig) => JSON.stringify({
  enabled: !!config.enabled,
  host: config.host || '',
  port: Number(config.port) || 0,
  apiKey: getPrimaryClientApiKey(config.clientApiKeysText || config.apiKey),
  clientApiKeysText: config.clientApiKeysText || config.apiKey || '',
  region: config.region || 'us-east-1',
  accountMode: config.accountMode || 'single',
  accountId: config.accountId || null,
  groupId: config.groupId || null,
  poolAccountIds: config.poolAccountIds || [],
  strategy: config.strategy || 'round_robin',
  threshold: Number(config.threshold) || 90,
  localOnly: !!config.localOnly,
  allowedIpsText: config.allowedIpsText || '',
  logLevel: config.logLevel || 'debug',
  modelMappings: config.modelMappings || [],
  filterClaudeCode: !!config.filterClaudeCode,
  filterStripBoundaries: !!config.filterStripBoundaries,
  filterEnvNoise: !!config.filterEnvNoise,
  promptFilterRules: config.promptFilterRules || [],
  logRequests: config.logRequests !== false,
  responseCacheEnabled: !!config.responseCacheEnabled,
  responseCacheTtl: Number(config.responseCacheTtl) || 180,
  promptCacheTargetPercent: normalizePromptCacheTargetPercent(config.promptCacheTargetPercent),
  promptCacheTtlSecs: normalizePromptCacheTtlSecs(config.promptCacheTtlSecs),
  promptCacheMaxEntries: normalizePromptCacheMaxEntries(config.promptCacheMaxEntries),
  promptCacheIgnoreClientControl: !!config.promptCacheIgnoreClientControl
})

export const buildGatewayRuntimeSnapshot = (config: GatewayConfig) => JSON.stringify({
  host: config.host || '',
  port: Number(config.port) || 0,
  apiKey: getPrimaryClientApiKey(config.clientApiKeysText || config.apiKey),
  clientApiKeysText: config.clientApiKeysText || config.apiKey || '',
  region: config.region || 'us-east-1',
  accountMode: config.accountMode || 'single',
  accountId: config.accountId || null,
  groupId: config.groupId || null,
  poolAccountIds: config.poolAccountIds || [],
  strategy: config.strategy || 'round_robin',
  threshold: Number(config.threshold) || 90,
  localOnly: !!config.localOnly,
  allowedIpsText: config.allowedIpsText || '',
  logLevel: config.logLevel || 'debug',
  modelMappings: config.modelMappings || [],
  filterClaudeCode: !!config.filterClaudeCode,
  filterStripBoundaries: !!config.filterStripBoundaries,
  filterEnvNoise: !!config.filterEnvNoise,
  promptFilterRules: config.promptFilterRules || [],
  logRequests: config.logRequests !== false,
  responseCacheEnabled: !!config.responseCacheEnabled,
  responseCacheTtl: Number(config.responseCacheTtl) || 180,
  promptCacheTargetPercent: normalizePromptCacheTargetPercent(config.promptCacheTargetPercent),
  promptCacheTtlSecs: normalizePromptCacheTtlSecs(config.promptCacheTtlSecs),
  promptCacheMaxEntries: normalizePromptCacheMaxEntries(config.promptCacheMaxEntries),
  promptCacheIgnoreClientControl: !!config.promptCacheIgnoreClientControl
})

export const hydrateGatewayConfig = (gatewayConfig: any): GatewayConfig => ({
  ...(() => {
    const clientApiKeys = Array.isArray(gatewayConfig?.clientApiKeys)
      ? parseClientApiKeys(gatewayConfig.clientApiKeys.join('\n'))
      : parseClientApiKeys(gatewayConfig?.accessToken || '')
    const primaryApiKey = clientApiKeys[0] || ''
    return {
      apiKey: primaryApiKey,
      clientApiKeysText: clientApiKeys.join('\n')}
  })(),
  enabled: gatewayConfig?.enabled ?? false,
  host: gatewayConfig?.host || '127.0.0.1',
  port: gatewayConfig?.port || 8765,
  region: gatewayConfig?.region || 'us-east-1',
  accountMode: gatewayConfig?.accountMode === 'local'
    ? 'single'
    : (gatewayConfig?.accountMode || 'single'),
  accountId: gatewayConfig?.accountId || null,
  groupId: gatewayConfig?.groupId || null,
  poolAccountIds: Array.isArray(gatewayConfig?.poolAccountIds) ? gatewayConfig.poolAccountIds : [],
  strategy: gatewayConfig?.strategy || 'round_robin',
  threshold: gatewayConfig?.threshold ?? 90,
  localOnly: gatewayConfig?.localOnly ?? true,
  allowedIpsText: Array.isArray(gatewayConfig?.allowedIps)
    ? gatewayConfig.allowedIps.join('\n')
    : '',
  logLevel: gatewayConfig?.logLevel || 'debug',
  modelMappings: Array.isArray(gatewayConfig?.modelMappings) ? gatewayConfig.modelMappings : [],
  filterClaudeCode: gatewayConfig?.filterClaudeCode ?? false,
  filterStripBoundaries: gatewayConfig?.filterStripBoundaries ?? false,
  filterEnvNoise: gatewayConfig?.filterEnvNoise ?? false,
  promptFilterRules: Array.isArray(gatewayConfig?.promptFilterRules) ? gatewayConfig.promptFilterRules : [],
  logRequests: gatewayConfig?.logRequests ?? true,
  responseCacheEnabled: gatewayConfig?.responseCacheEnabled ?? true,
  responseCacheTtl: gatewayConfig?.responseCacheTtl ?? 180,
  promptCacheTargetPercent: normalizePromptCacheTargetPercent(gatewayConfig?.promptCacheTargetPercent),
  promptCacheTtlSecs: normalizePromptCacheTtlSecs(gatewayConfig?.promptCacheTtlSecs),
  promptCacheMaxEntries: normalizePromptCacheMaxEntries(gatewayConfig?.promptCacheMaxEntries),
  promptCacheIgnoreClientControl: gatewayConfig?.promptCacheIgnoreClientControl ?? false
})

export const buildGatewayStatusState = (gatewayStatus: any, gatewayConfig: any, fallbackConfig: GatewayConfig = DEFAULT_GATEWAY_CONFIG): GatewayStatus => ({
  running: gatewayStatus?.running ?? false,
  host: gatewayStatus?.host || gatewayConfig?.host || fallbackConfig.host,
  port: gatewayStatus?.port || gatewayConfig?.port || fallbackConfig.port,
  requestCount: gatewayStatus?.requestCount || 0,
  lastError: gatewayStatus?.lastError || null,
  runtimeConfig: gatewayStatus?.runtimeConfig ? hydrateGatewayConfig(gatewayStatus.runtimeConfig) : null
})

export const buildGatewayPayload = (config: GatewayConfig) => ({
  ...(() => {
    const clientApiKeys = parseClientApiKeys(config.clientApiKeysText || config.apiKey)
    return {
      accessToken: clientApiKeys[0] || null,
      clientApiKeys}
  })(),
  enabled: !!config.enabled,
  host: config.host,
  port: Number(config.port),
  region: config.region,
  accountMode: config.accountMode,
  accountId: config.accountId || null,
  groupId: config.groupId || null,
  poolAccountIds: config.poolAccountIds || [],
  strategy: config.strategy || 'round_robin',
  threshold: Number(config.threshold) || 90,
  localOnly: !!config.localOnly,
  allowedIps: parseAllowedIps(config.allowedIpsText),
  logLevel: config.logLevel,
  modelMappings: config.modelMappings || [],
  filterClaudeCode: !!config.filterClaudeCode,
  filterStripBoundaries: !!config.filterStripBoundaries,
  filterEnvNoise: !!config.filterEnvNoise,
  promptFilterRules: config.promptFilterRules || [],
  logRequests: config.logRequests !== false,
  responseCacheEnabled: !!config.responseCacheEnabled,
  responseCacheTtl: Number(config.responseCacheTtl) || 3600,
  promptCacheTargetPercent: normalizePromptCacheTargetPercent(config.promptCacheTargetPercent),
  promptCacheTtlSecs: normalizePromptCacheTtlSecs(config.promptCacheTtlSecs),
  promptCacheMaxEntries: normalizePromptCacheMaxEntries(config.promptCacheMaxEntries),
  promptCacheIgnoreClientControl: !!config.promptCacheIgnoreClientControl
})

export const loadGatewayPageData = async () => {
  const [gatewayConfig, gatewayStatus, accounts, groups, logDir] = await Promise.all([
    invoke<any>('get_gateway_config'),
    invoke<any>('get_gateway_status'),
    invoke<any[]>('get_accounts'),
    invoke<any[]>('get_groups'),
    invoke<string>('get_gateway_log_dir'),
  ])

  return {
    gatewayConfig,
    gatewayStatus,
    accounts: Array.isArray(accounts) ? accounts : [],
    groups: Array.isArray(groups) ? groups : [],
    logDir: String(logDir || '')}
}

export const fetchGatewayStatus = async () => invoke<any>('get_gateway_status')

export const fetchGatewayRequestLogs = async (limit = 120) => {
  const logs = await invoke<any[]>('get_gateway_request_logs', { limit })
  return Array.isArray(logs) ? logs : []
}

export const saveGatewayConfig = async (config: GatewayConfig) => invoke<any>('save_gateway_config', {
  config: buildGatewayPayload(config)})

export const startGateway = async (config: GatewayConfig) => invoke<any>('start_gateway', {
  config: buildGatewayPayload(config)})

export const stopGateway = async () => invoke('stop_gateway')

export const openGatewayLogDir = async () => invoke<string>('open_gateway_log_dir')

export const clearGatewayRequestLogs = async () => invoke('clear_gateway_request_logs')

import { startTransition, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Play, Square, Zap, ScrollText, Copy } from 'lucide-react'
import { Alert as AlertPrimitive, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { invoke } from '@tauri-apps/api/core'
import { useApp } from '../../../hooks/useApp'
import { Stack, Group, Badge, Card, Text } from '@/components/shared/layout'
import {
  DialogRoot,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogBody
} from '@/components/shared/dialog'
import GatewayConfigComponent from './GatewayConfig'
import { RequestLogsDialog } from './RequestLogsDialog'
import { RouteTestDialog } from './RouteTestDialog'
import { GatewayConfig, GatewayStatus } from './gatewayPageState'
import { ErrorHistoryEntry } from './gatewayPageUtils'

// 定义类型接口
interface Account {
  id: string
  email?: string
  userId?: string
}

interface Group {
  id: string
  name: string
}

interface ClientConfigResult {
  success: boolean
  error?: string
}
import {
  GatewayConfigProvider,
  GatewayStatusProvider,
  GatewayDataProvider
} from './contexts'
import {
  applyGatewayLocalOnlyChange,
  buildGatewayActionSummary,
  buildGatewayBaseUrl,
  buildGatewayIntegrationSummary,
  buildGatewayRoutingSummary,
  buildGatewaySecuritySummary,
  createGatewayFieldErrors,
  formatGatewayAccountOptionLabel,
  formatGatewayTimestamp,
  mergeErrorHistory
} from './gatewayPageUtils'
import {
  buildGatewayConfigSnapshot,
  buildGatewayRuntimeSnapshot,
  buildGatewayStatusState,
  DEFAULT_GATEWAY_CONFIG,
  DEFAULT_GATEWAY_STATUS,
  fetchGatewayStatus,
  loadGatewayPageData,
  openGatewayLogDir,
  saveGatewayConfig,
  startGateway,
  stopGateway,
  hydrateGatewayConfig
} from './gatewayPageState'
import { useGatewayPolling } from './useGatewayPolling'

declare const __KAM_SERVER_WEB__: boolean | undefined

function Alert(props: any) {
  return <AlertPrimitive {...props} />
}

function ThemedAlert({ title, children, ...props }: any) {
  return (
    <Alert {...props}>
      {title && <AlertTitle className={"text-foreground"}>{title}</AlertTitle>}
      <AlertDescription className={"text-muted-foreground"}>
        {children}
      </AlertDescription>
    </Alert>
  )
}

function GatewayPage() {
  const { t } = useApp()
  const isServerWeb = typeof __KAM_SERVER_WEB__ !== 'undefined' && __KAM_SERVER_WEB__

  const [config, setConfig] = useState<GatewayConfig>(DEFAULT_GATEWAY_CONFIG)
  const [status, setStatus] = useState<GatewayStatus>(DEFAULT_GATEWAY_STATUS)
  const [errorHistory, setErrorHistory] = useState<ErrorHistoryEntry[]>([])
  const [accounts, setAccounts] = useState<Account[]>([])
  const [groups, setGroups] = useState<Group[]>([])
  const [loading, setLoading] = useState(false)
  const [saving, setSaving] = useState(false)
  const [copySuccess, setCopySuccess] = useState('')
  const [logDir, setLogDir] = useState('')
  const [showRequestLogs, setShowRequestLogs] = useState(false)
  const [showRouteTest, setShowRouteTest] = useState(false)
  const [savedConfigSnapshot, setSavedConfigSnapshot] = useState(() => buildGatewayConfigSnapshot(DEFAULT_GATEWAY_CONFIG))
  const [appliedRuntimeSnapshot, setAppliedRuntimeSnapshot] = useState<any>(null)
  const [lastStatusSyncAt, setLastStatusSyncAt] = useState('-')
  const [showClientConfig, setShowClientConfig] = useState(false)
  const [clientConfigLoading, setClientConfigLoading] = useState(false)
  const [clientConfigResults, setClientConfigResults] = useState<ClientConfigResult[]>([])
  const [selectedClients, setSelectedClients] = useState<string[]>(['claudeCode'])

  const hasConfiguredClients = useMemo(
    () => clientConfigResults.some(r => r.success),
    [clientConfigResults]
  )

  const accountOptions = useMemo(
    () => accounts.map(account => ({
      value: account.id,
      label: formatGatewayAccountOptionLabel(account)
    })),
    [accounts]
  )

  const groupOptions = useMemo(
    () => groups.map(group => ({ value: group.id, label: group.name })),
    [groups]
  )

  const fieldErrors = useMemo(() => createGatewayFieldErrors(config), [config])
  const hasFieldErrors = Object.keys(fieldErrors).length > 0
  const configSnapshot = useMemo(() => buildGatewayConfigSnapshot(config), [config])
  const runtimeSnapshot = useMemo(() => buildGatewayRuntimeSnapshot(config), [config])
  const hasUnsavedChanges = configSnapshot !== savedConfigSnapshot
  const hasRuntimeChanges = !!status.running && !!appliedRuntimeSnapshot && runtimeSnapshot !== appliedRuntimeSnapshot

  // 自动保存 + 热应用（防抖 1.5 秒）
  const autoSaveTimer = useRef<NodeJS.Timeout | null>(null)
  const isInitialLoad = useRef(true)
  useEffect(() => {
    // 跳过初始加载
    if (isInitialLoad.current) {
      isInitialLoad.current = false
      return
    }
    if (!hasUnsavedChanges) return
    if (autoSaveTimer.current) clearTimeout(autoSaveTimer.current)
    autoSaveTimer.current = setTimeout(async () => {
      try {
        await saveGatewayConfig(config)
        setSavedConfigSnapshot(buildGatewayConfigSnapshot(config))
        if (isServerWeb) {
          await refreshGatewayStatus(config)
        } else if (status.running) {
          await stopGateway()
          const st = await startGateway(config)
          const nextStatus = buildGatewayStatusState(st, st, config)
          setStatus(nextStatus)
          setAppliedRuntimeSnapshot(nextStatus.runtimeConfig ? buildGatewayRuntimeSnapshot(nextStatus.runtimeConfig) : buildGatewayRuntimeSnapshot(config))
          setLastStatusSyncAt(formatGatewayTimestamp())
        }
      } catch (e) {
        pushError(e)
      }
    }, 1500)
    return () => { if (autoSaveTimer.current) clearTimeout(autoSaveTimer.current) }
  }, [configSnapshot])

  const effectiveConfig = useMemo(
    () => (status.running && status.runtimeConfig ? status.runtimeConfig : config),
    [status.running, status.runtimeConfig, config]
  )
  const effectiveBaseUrl = useMemo(
    () => buildGatewayBaseUrl(effectiveConfig.host, effectiveConfig.port, effectiveConfig.localOnly),
    [effectiveConfig.host, effectiveConfig.port, effectiveConfig.localOnly]
  )
  const actionSummary = useMemo(
    () => buildGatewayActionSummary({ running: status.running, isDirty: hasUnsavedChanges, hasUnsavedChanges, hasRuntimeChanges, hasFieldErrors, liveReload: isServerWeb }),
    [status.running, hasUnsavedChanges, hasRuntimeChanges, hasFieldErrors, isServerWeb]
  )
  const effectiveSecuritySummary = useMemo(
    () => buildGatewaySecuritySummary({ config: effectiveConfig }),
    [effectiveConfig]
  )
  const integrationSummary = useMemo(
    () => buildGatewayIntegrationSummary({
      baseUrl: effectiveBaseUrl,
      apiKey: effectiveConfig.clientApiKeysText || effectiveConfig.apiKey,
      clientApiKeysText: effectiveConfig.clientApiKeysText || effectiveConfig.apiKey,
      logDir,
      errorHistory
    }),
    [effectiveBaseUrl, effectiveConfig.clientApiKeysText, effectiveConfig.apiKey, logDir, errorHistory]
  )
  const effectiveRoutingSummary = useMemo(() => buildGatewayRoutingSummary({
    config: effectiveConfig,
    counts: {
      accounts: accounts.length,
      groups: groups.length
    },
    selectedLabels: {
      single: accountOptions.find(item => item.value === effectiveConfig.accountId)?.label,
      group: groupOptions.find(item => item.value === effectiveConfig.groupId)?.label
    }
  }), [effectiveConfig, accounts.length, groups.length, accountOptions, groupOptions])

  const latestErrorEntry = useMemo(
    () => errorHistory[0] || null,
    [errorHistory]
  )

  const consoleHighlights = useMemo(() => ([
    {
      label: '当前入口',
      value: effectiveBaseUrl
    },
    {
      label: '客户端 Key',
      value: effectiveSecuritySummary.apiKeyState
    },
    {
      label: '路由模式',
      value: effectiveRoutingSummary.modeLabel
    },
  ]), [
    effectiveBaseUrl,
    effectiveSecuritySummary.apiKeyState,
    effectiveRoutingSummary.modeLabel,
  ])

  const pollingFallbackConfig = useMemo(
    () => ({
      host: config.host,
      port: config.port
    }),
    [config.host, config.port]
  )

  const pushError = (msg: any) => {
    const normalized = String(msg?.message || msg || '').trim()
    if (!normalized) return
    setErrorHistory(prev => mergeErrorHistory(prev, normalized, formatGatewayTimestamp(), 8))
  }

  const refreshGatewayStatus = useCallback(async (fallbackConfig: GatewayConfig) => {
    const st = await fetchGatewayStatus()
    const nextStatus = buildGatewayStatusState(st, st, fallbackConfig)
    setStatus(nextStatus)
    setAppliedRuntimeSnapshot(nextStatus.running && nextStatus.runtimeConfig
      ? buildGatewayRuntimeSnapshot(nextStatus.runtimeConfig)
      : null)
    setLastStatusSyncAt(formatGatewayTimestamp())
    return nextStatus
  }, [])

  const loadAll = useCallback(async () => {
    setLoading(true)
    try {
      const { gatewayConfig, gatewayStatus, accounts: accountList, groups: groupList, logDir: gatewayLogDir } = await loadGatewayPageData()

      const nextConfig = hydrateGatewayConfig(gatewayConfig)
      const nextStatus = buildGatewayStatusState(gatewayStatus, gatewayConfig, nextConfig)
      const runtimeConfig = gatewayStatus?.running && nextStatus.runtimeConfig ? nextStatus.runtimeConfig : null
      setConfig(nextConfig)
      setSavedConfigSnapshot(buildGatewayConfigSnapshot(nextConfig))
      setAppliedRuntimeSnapshot(runtimeConfig ? buildGatewayRuntimeSnapshot(runtimeConfig) : null)
      setStatus(nextStatus)
      setLastStatusSyncAt(formatGatewayTimestamp())
      setAccounts(accountList)
      setGroups(groupList)
      setLogDir(gatewayLogDir)

      if (gatewayStatus?.lastError) {
        pushError(gatewayStatus.lastError)
      }
    } catch (e) {
      pushError(e)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    startTransition(() => {
      loadAll()
    })
  }, [loadAll])

  const handleStatusPoll = useCallback(({ status: nextStatus, fallbackConfig, syncedAt }: any) => {
    const nextState = buildGatewayStatusState(nextStatus, nextStatus, fallbackConfig)
    setStatus(nextState)
    setAppliedRuntimeSnapshot(nextState.running && nextState.runtimeConfig
      ? buildGatewayRuntimeSnapshot(nextState.runtimeConfig)
      : null)
    setLastStatusSyncAt(syncedAt)
    if (nextStatus?.lastError) {
      pushError(nextStatus.lastError)
    }
  }, [])

  useGatewayPolling({
    activeTab: 'config',
    fallbackConfig: pollingFallbackConfig,
    onStatus: handleStatusPoll
  })

  const setField = (key: string, value: any) => setConfig(prev => ({ ...prev, [key]: value }))

  const createGeneratedApiKey = () => {
    const random = crypto?.randomUUID?.().replace(/-/g, '') || `${Date.now()}${Math.random().toString(36).slice(2)}`
    return `sk-${random}`
  }

  const handleRefresh = async () => {
    await loadAll()
  }

  const handleClearErrors = () => {
    setErrorHistory([])
  }

  const handleGenerateApiKey = () => {
    setConfig(prev => {
      const generatedKey = createGeneratedApiKey()
      const existingKeys = String(prev.clientApiKeysText || prev.apiKey || '').trim()
      const clientApiKeysText = existingKeys ? `${existingKeys}\n${generatedKey}` : generatedKey
      return {
        ...prev,
        apiKey: generatedKey,
        clientApiKeysText
      }
    })
  }

  const handleOpenLogDir = async () => {
    try {
      const dir = await openGatewayLogDir()
      setLogDir(String(dir || ''))
    } catch (e) {
      pushError(e)
    }
  }

  const guardInvalidConfig = () => {
    if (!hasFieldErrors) {
      return false
    }
    pushError('请先修正表单错误后再继续')
    return true
  }

  // 静默保存（Dialog 关闭时用，不校验、不重启）
  const handleSilentSave = async () => {
    try {
      await saveGatewayConfig(config)
      setSavedConfigSnapshot(buildGatewayConfigSnapshot(config))
      if (isServerWeb) {
        await refreshGatewayStatus(config)
      }
    } catch (e) {
      pushError(e)
    }
  }

  const handleSave = async () => {
    if (guardInvalidConfig()) return
    setSaving(true)
    try {
      await saveGatewayConfig(config)
      setSavedConfigSnapshot(buildGatewayConfigSnapshot(config))
      if (isServerWeb) {
        await refreshGatewayStatus(config)
      } else if (status.running) {
        await stopGateway()
        const st = await startGateway(config)
        const nextStatus = buildGatewayStatusState(st, st, config)
        setStatus(nextStatus)
        setAppliedRuntimeSnapshot(nextStatus.runtimeConfig ? buildGatewayRuntimeSnapshot(nextStatus.runtimeConfig) : buildGatewayRuntimeSnapshot(config))
        setLastStatusSyncAt(formatGatewayTimestamp())
      }
    } catch (e) {
      pushError(e)
    } finally {
      setSaving(false)
    }
  }

  const handleStart = async () => {
    if (guardInvalidConfig()) return
    setSaving(true)
    try {
      const st = await startGateway(config)
      const nextStatus = buildGatewayStatusState(st, st, config)
      setStatus(nextStatus)
      setAppliedRuntimeSnapshot(nextStatus.runtimeConfig ? buildGatewayRuntimeSnapshot(nextStatus.runtimeConfig) : buildGatewayRuntimeSnapshot(config))
      setLastStatusSyncAt(formatGatewayTimestamp())
    } catch (e) {
      pushError(e)
    } finally {
      setSaving(false)
    }
  }

  const handleRestart = async () => {
    if (guardInvalidConfig()) return
    setSaving(true)
    try {
      if (status.running) {
        await stopGateway()
      }
      const st = await startGateway(config)
      const nextStatus = buildGatewayStatusState(st, st, config)
      setStatus(nextStatus)
      setAppliedRuntimeSnapshot(nextStatus.runtimeConfig ? buildGatewayRuntimeSnapshot(nextStatus.runtimeConfig) : buildGatewayRuntimeSnapshot(config))
      setLastStatusSyncAt(formatGatewayTimestamp())
    } catch (e) {
      pushError(e)
    } finally {
      setSaving(false)
    }
  }

  const handleStop = async () => {
    setSaving(true)
    try {
      await stopGateway()
      setStatus(prev => ({ ...prev, running: false }))
      setAppliedRuntimeSnapshot(null)
      setLastStatusSyncAt(formatGatewayTimestamp())
    } catch (e) {
      pushError(e)
    } finally {
      setSaving(false)
    }
  }

  const handleAutoStartToggle = async (checked: boolean) => {
    setField('enabled', checked)

    // 延迟执行，确保 setField 先更新状态
    setTimeout(async () => {
      try {
        // 先保存配置
        await saveGatewayConfig({ ...config, enabled: checked })
        setSavedConfigSnapshot(buildGatewayConfigSnapshot({ ...config, enabled: checked }))

        // 如果勾选自动启动且配置有效，立即启动2API
        if (checked && !hasFieldErrors) {
          setSaving(true)
          const st = await startGateway({ ...config, enabled: checked })
          const nextStatus = buildGatewayStatusState(st, st, { ...config, enabled: checked })
          setStatus(nextStatus)
          setAppliedRuntimeSnapshot(nextStatus.runtimeConfig ? buildGatewayRuntimeSnapshot(nextStatus.runtimeConfig) : buildGatewayRuntimeSnapshot({ ...config, enabled: checked }))
          setLastStatusSyncAt(formatGatewayTimestamp())
          setSaving(false)
        } else if (!checked && status.running) {
          // 如果取消自动启动且2API正在运行，停止2API
          setSaving(true)
          await stopGateway()
          setStatus(prev => ({ ...prev, running: false }))
          setAppliedRuntimeSnapshot(null)
          setLastStatusSyncAt(formatGatewayTimestamp())
          setSaving(false)
        }
      } catch (e) {
        pushError(e)
        setSaving(false)
      }
    }, 100)
  }

  const copyText = async (text: string, successMessage: string) => {
    try {
      await navigator.clipboard.writeText(text)
      setCopySuccess(successMessage)
      setTimeout(() => setCopySuccess(''), 1600)
    } catch (e) {
      pushError(e)
    }
  }

  const handleConfigureClients = async () => {
    setClientConfigLoading(true)
    try {
      const apiKey = effectiveConfig.clientApiKeysText || effectiveConfig.apiKey || ''
      const results = await invoke<any[]>('configure_proxy_clients', {
        clients: selectedClients,
        host: effectiveConfig.host,
        port: effectiveConfig.port,
        apiKey: apiKey.split('\n')[0]?.trim() || apiKey.trim(),
      })
      setClientConfigResults(results)
    } catch (e) {
      pushError(e)
    } finally {
      setClientConfigLoading(false)
    }
  }

  return (
    <GatewayConfigProvider>
      <GatewayStatusProvider>
        <GatewayDataProvider>
          <div className={`h-full overflow-y-auto p-3 glass-main`}>
            <Stack gap="sm">
              <Card className={`glass-card border border-border rounded-xl p-3`}>
                <Stack gap="sm">
                  <Group justify="space-between" align="center">
                    <Group gap="xs">
                      <Text fw={700} className="text-foreground text-base">Kiro2API </Text>
                      {!status.running ? (
                        <Button
                          size="sm"
                          onClick={handleStart}
                          disabled={hasFieldErrors || saving || loading}
                          className="bg-green-500 hover:bg-green-600 text-white h-7 px-2.5 text-xs"
                        >
                          <Play size={12} className="mr-1" />
                          启动
                        </Button>
                      ) : (
                        <Button
                          size="sm"
                          onClick={handleStop}
                          disabled={saving || loading}
                          className="bg-red-500 hover:bg-red-600 text-white h-7 px-2.5 text-xs"
                        >
                          <Square size={12} className="mr-1" />
                          停止
                        </Button>
                      )}
                      <Badge color={status.running ? 'green' : 'gray'}>{status.running ? '运行中' : '已停止'}</Badge>
                    </Group>
                    <Group gap="xs">
                      <Button variant="outline" size="sm" className="h-7 px-2.5 text-xs" onClick={() => setShowRequestLogs(true)} disabled={!status.running}>
                        <ScrollText size={12} className="mr-1" />
                        请求日志
                      </Button>
                      <Button variant="outline" size="sm" className="h-7 px-2.5 text-xs" onClick={() => setShowRouteTest(true)}>
                        <Zap size={12} className="mr-1" />
                        测试路由
                      </Button>
                    </Group>
                  </Group>

                  <div className="grid grid-cols-3 gap-2">
                    {consoleHighlights.map((item) => (
                      <div key={item.label} className="border rounded-lg p-2 group relative">
                        <Text size="xs" className={"text-muted-foreground"}>{item.label}</Text>
                        <div className="flex items-center gap-2">
                          <Text fw={700} className={"text-foreground text-sm flex-1 truncate"}>{item.value}</Text>
                          {(item.label === '当前入口' || item.label === '客户端 Key') && (
                            <Button
                              size="sm"
                              variant="ghost"
                              className="h-6 w-6 p-0 opacity-0 group-hover:opacity-100 transition-opacity"
                              onClick={() => {
                                const text = item.label === '当前入口'
                                  ? effectiveBaseUrl
                                  : (effectiveConfig.clientApiKeysText || effectiveConfig.apiKey || '').split('\n')[0]?.trim()
                                copyText(text, `已复制${item.label}`)
                              }}
                            >
                              <Copy size={12} className="text-muted-foreground" />
                            </Button>
                          )}
                        </div>
                      </div>
                    ))}
                  </div>
                </Stack>
              </Card>

              <GatewayConfigComponent
                config={config}
                fieldErrors={fieldErrors}
                setField={setField}
                accountOptions={accountOptions}
                groupOptions={groupOptions}
                setConfig={setConfig}
                applyGatewayLocalOnlyChange={applyGatewayLocalOnlyChange}
                createGeneratedApiKey={createGeneratedApiKey}
                handleSaveConfig={handleSilentSave}
                handleAutoStartToggle={handleAutoStartToggle}
                onShowClientConfig={() => setShowClientConfig(true)}
                hasConfiguredClients={hasConfiguredClients}
              />

              <RequestLogsDialog open={showRequestLogs} onOpenChange={setShowRequestLogs} logLevel={config.logLevel} onLogLevelChange={(v) => setField('logLevel', v)} logRequests={config.logRequests} onLogRequestsChange={(v) => setField('logRequests', v)} onSave={handleSilentSave} />

              <RouteTestDialog
                open={showRouteTest}
                onOpenChange={setShowRouteTest}
                config={config}
              />

              {/* 快速配置客户端弹窗 */}
              <DialogRoot open={showClientConfig} onOpenChange={setShowClientConfig}>
                <DialogContent maxWidth="480px">
                  <DialogHeader className="">
                    <DialogTitle className="">{t('gateway.quickClientConfig')}</DialogTitle>
                    <DialogDescription className="">
                      {t('gateway.quickClientConfigDesc')}
                    </DialogDescription>
                  </DialogHeader>

                  <DialogBody className="flex flex-col gap-4 pt-2">
                    {/* 客户端选择 */}
                    <div className="flex gap-3">
                      {[
                        { id: 'claudeCode', label: 'Claude Code CLI', desc: '~/.claude/settings.json' },
                        { id: 'codex', label: 'Codex CLI', desc: '~/.codex/auth.json + config.toml' },
                      ].map(client => (
                        <div
                          key={client.id}
                          onClick={() => setSelectedClients(prev =>
                            prev.includes(client.id) ? prev.filter(c => c !== client.id) : [...prev, client.id]
                          )}
                          className={`flex-1 p-3 rounded-xl border cursor-pointer transition-all ${selectedClients.includes(client.id)
                              ? 'border-primary bg-primary/5 shadow-sm'
                              : 'border-border bg-muted/20 hover:bg-muted/40'
                            }`}
                        >
                          <Text size="sm" fw={600} className="text-foreground">{client.label}</Text>
                          <Text size="xs" className="text-muted-foreground font-mono mt-1">{client.desc}</Text>
                        </div>
                      ))}
                    </div>

                    {/* 配置预览 */}
                    <div className="bg-muted/30 border border-border rounded-xl p-3">
                      <Text size="xs" className="text-muted-foreground mb-2">{t('gateway.configToWrite')}</Text>
                      <div className="flex flex-col gap-2 font-mono text-[11px]">
                        {selectedClients.includes('claudeCode') && (
                          <div className="flex flex-col gap-0.5 p-2 rounded-lg bg-muted/30">
                            <span className="text-muted-foreground text-[10px] font-sans">Claude Code → ~/.claude/settings.json</span>
                            <span className="text-foreground">ANTHROPIC_BASE_URL = {effectiveBaseUrl}</span>
                            <span className="text-foreground">ANTHROPIC_API_KEY = {(effectiveConfig.clientApiKeysText || effectiveConfig.apiKey || '').split('\n')[0]?.substring(0, 12)}***</span>
                          </div>
                        )}
                        {selectedClients.includes('codex') && (
                          <div className="flex flex-col gap-0.5 p-2 rounded-lg bg-muted/30">
                            <span className="text-muted-foreground text-[10px] font-sans">Codex → ~/.codex/config.toml + auth.json</span>
                            <span className="text-foreground">base_url = "{effectiveBaseUrl}/v1"</span>
                            <span className="text-foreground">OPENAI_API_KEY = {(effectiveConfig.clientApiKeysText || effectiveConfig.apiKey || '').split('\n')[0]?.substring(0, 12)}***</span>
                          </div>
                        )}
                      </div>
                    </div>

                    {/* 执行按钮 */}
                    <Button
                      onClick={handleConfigureClients}
                      disabled={selectedClients.length === 0 || clientConfigLoading}
                      className="w-full bg-primary hover:bg-primary/90 text-primary-foreground"
                    >
                      <Zap size={16} className="mr-1" />
                      {clientConfigLoading ? t('gateway.configuring') : t('gateway.oneClickConfigClients', { count: selectedClients.length })}
                    </Button>

                    {/* 结果展示 */}
                    {clientConfigResults.length > 0 && (
                      <div className="flex flex-col gap-2">
                        {clientConfigResults.map((result: any, idx: number) => (
                          <div key={idx} className={`p-3 rounded-lg border ${result.success ? 'border-green-500/30 bg-green-500/5' : 'border-red-500/30 bg-red-500/5'}`}>
                            <Text size="sm" fw={600} className={result.success ? 'text-green-600' : 'text-red-500'}>
                              {result.success ? '✓' : '✗'} {result.client}
                            </Text>
                            {result.success && result.paths?.length > 0 && (
                              <Text size="xs" className="text-muted-foreground font-mono mt-1">
                                {result.paths.join(', ')}
                              </Text>
                            )}
                            {result.error && (
                              <Text size="xs" className="text-red-500 mt-1">{result.error}</Text>
                            )}
                          </div>
                        ))}
                      </div>
                    )}
                  </DialogBody>
                </DialogContent>
              </DialogRoot>
            </Stack>
          </div>
        </GatewayDataProvider>
      </GatewayStatusProvider>
    </GatewayConfigProvider>
  )
}

export default GatewayPage

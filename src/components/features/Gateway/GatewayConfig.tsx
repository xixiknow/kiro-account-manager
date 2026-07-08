import type React from 'react'
import { useState } from 'react'
import { RotateCw, TrendingUp, Shuffle, Zap, Users, CheckCircle2 } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
import { Checkbox } from '@/components/ui/checkbox'
import { Badge } from '@/components/ui/badge'
import { Textarea } from '@/components/ui/textarea'
import { GatewaySurfaceCard } from './GatewayShared'
import ModelMappingDialog from './ModelMappingDialog'
import ApiKeysDialog from './ApiKeysDialog'
import PromptFilterRulesDialog from './PromptFilterRulesDialog'
import {
  DialogRoot,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogBody,
  DialogFooter
} from '@/components/shared/dialog'
import { useApp } from '../../../hooks/useApp'

interface GatewayConfigProps {
  config: any;
  fieldErrors: Record<string, string>;
  setField: (key: string, value: any) => void;
  accountOptions: any[];
  groupOptions: any[];
  setConfig: React.Dispatch<React.SetStateAction<any>>;
  applyGatewayLocalOnlyChange: (config: any, checked: boolean, generator: () => string) => any;
  createGeneratedApiKey: () => string;
  handleSaveConfig: () => Promise<void>;
  handleAutoStartToggle: (checked: boolean) => Promise<void>;
  onShowClientConfig?: () => void;
  hasConfiguredClients?: boolean;
}

function GatewayConfig({
  config,
  fieldErrors,
  setField,
  accountOptions,
  groupOptions,
  setConfig,
  applyGatewayLocalOnlyChange,
  createGeneratedApiKey,
  handleSaveConfig,
  handleAutoStartToggle,
  onShowClientConfig,
  hasConfiguredClients = false,
}: GatewayConfigProps) {
  const { t } = useApp()
  const [showModelMappingDialog, setShowModelMappingDialog] = useState(false)
  const [showApiKeysDialog, setShowApiKeysDialog] = useState(false)
  const [showPromptFilterRulesDialog, setShowPromptFilterRulesDialog] = useState(false)
  const [showAccountPoolDialog, setShowAccountPoolDialog] = useState(false)

  const getStrategyLabel = (strategy: string) => {
    const labels: Record<string, string> = {
      round_robin: '轮询',
      random: '随机',
      balanced: '均衡',
      most_quota: '最多配额',
      weighted_random: '加权随机',
      least_connections: '最少连接'
    }
    return labels[strategy] || strategy
  }

  const selectedPoolAccountIds = Array.isArray(config.poolAccountIds) ? config.poolAccountIds : []
  const selectedPoolAccounts = accountOptions.filter((account: any) => selectedPoolAccountIds.includes(account.value))
  const effectiveStrategy = config.strategy || 'round_robin'

  const togglePoolAccount = (accountId: string) => {
    const next = selectedPoolAccountIds.includes(accountId)
      ? selectedPoolAccountIds.filter((id: string) => id !== accountId)
      : [...selectedPoolAccountIds, accountId]
    setField('poolAccountIds', next)
  }

  const toggleAllPoolAccounts = () => {
    if (selectedPoolAccountIds.length === accountOptions.length) {
      setField('poolAccountIds', [])
    } else {
      setField('poolAccountIds', accountOptions.map((account: any) => account.value))
    }
  }

  return (
    <div className="grid grid-cols-1 gap-3">
      <GatewaySurfaceCard>
        <div className="flex flex-col gap-3">
          <div className="flex flex-col gap-4">
            {/* Section 1: 网络与路由 */}
            <div className="space-y-3">
              <div className="text-sm font-medium text-foreground flex items-center gap-2">
                <div className="w-1 h-4 bg-primary rounded-full"></div>
                网络与路由
              </div>
              <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
                <div className="flex flex-col gap-1.5">
                  <Label>监听地址</Label>
                  <Input
                    value={config.host}
                    onChange={(e: React.ChangeEvent<HTMLInputElement>) => setField('host', e.target.value || '127.0.0.1')}
                    className={fieldErrors.host ? 'border-red-500' : ''}
                  />
                  {fieldErrors.host && <div className="text-xs text-red-500">{fieldErrors.host}</div>}
                </div>
                <div className="flex flex-col gap-1.5">
                  <Label>端口</Label>
                  <Input
                    type="number"
                    value={config.port}
                    min={1}
                    max={65535}
                    onChange={(e: React.ChangeEvent<HTMLInputElement>) => setField('port', Number(e.target.value) || 8765)}
                    className={fieldErrors.port ? 'border-red-500' : ''}
                  />
                  {fieldErrors.port && <div className="text-xs text-red-500">{fieldErrors.port}</div>}
                </div>
                <div className="flex flex-col gap-1.5">
                  <Label>Region</Label>
                  <Select value={config.region} onValueChange={(v: string) => setField('region', v || 'us-east-1')}>
                    <SelectTrigger className={fieldErrors.region ? 'border-red-500' : ''}>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="us-east-1">us-east-1</SelectItem>
                      <SelectItem value="us-east-2">us-east-2</SelectItem>
                      <SelectItem value="us-west-1">us-west-1</SelectItem>
                      <SelectItem value="us-west-2">us-west-2</SelectItem>
                      <SelectItem value="eu-central-1">eu-central-1</SelectItem>
                      <SelectItem value="eu-central-2">eu-central-2</SelectItem>
                      <SelectItem value="eu-west-1">eu-west-1</SelectItem>
                      <SelectItem value="eu-west-2">eu-west-2</SelectItem>
                      <SelectItem value="eu-west-3">eu-west-3</SelectItem>
                      <SelectItem value="eu-north-1">eu-north-1</SelectItem>
                      <SelectItem value="eu-south-1">eu-south-1</SelectItem>
                      <SelectItem value="eu-south-2">eu-south-2</SelectItem>
                      <SelectItem value="ap-northeast-1">ap-northeast-1</SelectItem>
                      <SelectItem value="ap-northeast-2">ap-northeast-2</SelectItem>
                      <SelectItem value="ap-northeast-3">ap-northeast-3</SelectItem>
                      <SelectItem value="ap-southeast-1">ap-southeast-1</SelectItem>
                      <SelectItem value="ap-southeast-2">ap-southeast-2</SelectItem>
                      <SelectItem value="ap-southeast-3">ap-southeast-3</SelectItem>
                      <SelectItem value="ap-southeast-4">ap-southeast-4</SelectItem>
                      <SelectItem value="ap-southeast-5">ap-southeast-5</SelectItem>
                      <SelectItem value="ap-southeast-7">ap-southeast-7</SelectItem>
                      <SelectItem value="ap-south-1">ap-south-1</SelectItem>
                      <SelectItem value="ap-south-2">ap-south-2</SelectItem>
                      <SelectItem value="ap-east-1">ap-east-1</SelectItem>
                      <SelectItem value="af-south-1">af-south-1</SelectItem>
                      <SelectItem value="ca-central-1">ca-central-1</SelectItem>
                      <SelectItem value="ca-west-1">ca-west-1</SelectItem>
                      <SelectItem value="sa-east-1">sa-east-1</SelectItem>
                      <SelectItem value="me-south-1">me-south-1</SelectItem>
                      <SelectItem value="me-central-1">me-central-1</SelectItem>
                      <SelectItem value="il-central-1">il-central-1</SelectItem>
                      <SelectItem value="mx-central-1">mx-central-1</SelectItem>
                      <SelectItem value="us-gov-west-1">us-gov-west-1</SelectItem>
                      <SelectItem value="us-gov-east-1">us-gov-east-1</SelectItem>
                      <SelectItem value="cn-north-1">cn-north-1</SelectItem>
                      <SelectItem value="cn-northwest-1">cn-northwest-1</SelectItem>
                    </SelectContent>
                  </Select>
                  {fieldErrors.region && <div className="text-xs text-red-500">{fieldErrors.region}</div>}
                </div>
              </div>
              <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                <div className="flex flex-col gap-1.5">
                  <Label>{t('gateway.accountMode')}</Label>
                  <Select value={config.accountMode} onValueChange={(v: string) => setField('accountMode', v)}>
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="single">{t('gateway.singleAccount')}</SelectItem>
                      <SelectItem value="group">{t('gateway.byGroup')}</SelectItem>
                      <SelectItem value="pool">{t('gateway.globalPool')}</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                {config.accountMode === 'pool' ? (
                  <div className="flex flex-col gap-1.5">
                    <Label>{t('gateway.accountPool')}</Label>
                    <Button
                      variant="outline"
                      className="h-10 justify-start"
                      onClick={() => setShowAccountPoolDialog(true)}
                    >
                      <Users size={16} className="mr-2" />
                      {config.poolAccountIds?.length > 0
                        ? `${t('gateway.selected')} ${config.poolAccountIds.length} ${t('gateway.accounts')} · ${getStrategyLabel(effectiveStrategy)}`
                        : t('gateway.configureAccountPool')}
                    </Button>
                  </div>
                ) : config.accountMode === 'group' ? (
                  <>
                    <div className="flex flex-col gap-1.5">
                      <Label>选择分组</Label>
                      <Select value={config.groupId} onValueChange={(v: string) => setField('groupId', v)}>
                        <SelectTrigger className={fieldErrors.groupId ? 'border-red-500' : ''}>
                          <SelectValue placeholder="选择一个分组" />
                        </SelectTrigger>
                        <SelectContent>
                          {groupOptions.map((opt: any) => (
                            <SelectItem key={opt.value} value={opt.value}>{opt.label}</SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      {fieldErrors.groupId && <div className="text-xs text-red-500">{fieldErrors.groupId}</div>}
                    </div>
                    <div className="flex flex-col gap-1.5">
                      <Label>{t('gateway.routingStrategy')}</Label>
                      <Select value={effectiveStrategy} onValueChange={(v: string) => setField('strategy', v || 'round_robin')}>
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="round_robin"><div className="flex items-center gap-2"><RotateCw size={14} /><span>{t('gateway.roundRobin')}</span></div></SelectItem>
                          <SelectItem value="most_quota"><div className="flex items-center gap-2"><TrendingUp size={14} /><span>{t('gateway.priorityQuota')}</span></div></SelectItem>
                          <SelectItem value="random"><div className="flex items-center gap-2"><Shuffle size={14} /><span>{t('gateway.random')}</span></div></SelectItem>
                        </SelectContent>
                      </Select>
                    </div>
                  </>
                ) : (
                  <div className="flex flex-col gap-1.5">
                    <Label>{t('gateway.specifySingleAccount')}</Label>
                    <Select value={config.accountId} onValueChange={(v: string) => setField('accountId', v)}>
                      <SelectTrigger className={fieldErrors.accountId ? 'border-red-500' : ''}>
                        <SelectValue placeholder={t('gateway.selectAGroup')} />
                      </SelectTrigger>
                      <SelectContent>
                        {accountOptions.map((opt: any) => (
                          <SelectItem key={opt.value} value={opt.value}>{opt.label}</SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    {fieldErrors.accountId && <div className="text-xs text-red-500">{fieldErrors.accountId}</div>}
                  </div>
                )}
              </div>
            </div>

            {/* Section 2: 客户端认证与模型 */}
            <div className="space-y-3">
              <div className="text-sm font-medium text-foreground flex items-center gap-2">
                <div className="w-1 h-4 bg-primary rounded-full"></div>
                {t('gateway.clientAuthAndModel')}
              </div>
              <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
                <div className="flex items-center justify-between p-3 border rounded-lg bg-muted/20">
                  <div className="text-sm text-muted-foreground">
                    {(() => {
                      const rawKeys = (config.clientApiKeysText || '').split(/[\n,]+/).map((k: string) => k.trim()).filter(Boolean)
                      const enabledCount = rawKeys.filter((k: string) => !k.startsWith('#disabled#')).length
                      return rawKeys.length > 0
                        ? `${rawKeys.length} ${t('gateway.keys')}, ${enabledCount} ${t('gateway.enabledCount')}`
                        : t('gateway.noApiKey')
                    })()}
                  </div>
                  <Button size="sm" variant="outline" className="h-7 text-sm" onClick={() => setShowApiKeysDialog(true)}>
                    {t('gateway.manageKeys')}
                  </Button>
                </div>
                <div className="flex items-center justify-between p-3 border rounded-lg bg-muted/20">
                  <div className="text-sm text-muted-foreground">
                    {config.modelMappings?.length > 0
                      ? `${config.modelMappings.length} ${t('gateway.mappingRules')}, ${config.modelMappings.filter((r: any) => r.enabled).length} ${t('gateway.enabled')}`
                      : t('gateway.noMappingRules')}
                  </div>
                  <Button size="sm" variant="outline" className="h-7 text-sm" onClick={() => setShowModelMappingDialog(true)}>
                    <Shuffle size={12} className="mr-1" />
                    {t('gateway.mappingRules')}
                  </Button>
                </div>
                {onShowClientConfig && (
                  <div className="flex items-center justify-between p-3 border rounded-lg bg-muted/20">
                    <div className="text-sm text-muted-foreground">
                      {hasConfiguredClients ? t('gateway.clientConfigured') : t('gateway.writeClientConfig')}
                    </div>
                    <Button
                      size="sm"
                      variant={hasConfiguredClients ? "default" : "outline"}
                      className="h-7 text-sm"
                      onClick={onShowClientConfig}
                    >
                      <Zap size={12} className="mr-1" />
                      {hasConfiguredClients ? t('gateway.reconfigure') : t('gateway.configureClient')}
                    </Button>
                  </div>
                )}
              </div>
              {fieldErrors.clientApiKeysText && <div className="text-xs text-red-500">{fieldErrors.clientApiKeysText}</div>}
            </div>

            {/* Section 3: 提示词过滤 */}
            <div className="space-y-3">
              <div className="text-sm font-medium text-foreground flex items-center gap-2">
                <div className="w-1 h-4 bg-primary rounded-full"></div>
                {t('gateway.promptFilter')}
              </div>
              <div className="grid grid-cols-3 gap-2">
                <div className="flex items-center justify-between p-2.5 rounded-lg border border-border bg-muted/30">
                  <Label className="text-sm">{t('gateway.simplifyCCPrompt')}</Label>
                  <Switch checked={!!config.filterClaudeCode} onCheckedChange={(checked: boolean) => setField('filterClaudeCode', checked)} />
                </div>
                <div className="flex items-center justify-between p-2.5 rounded-lg border border-border bg-muted/30">
                  <Label className="text-sm">{t('gateway.removeBoundaryMarkers')}</Label>
                  <Switch checked={!!config.filterStripBoundaries} onCheckedChange={(checked: boolean) => setField('filterStripBoundaries', checked)} />
                </div>
                <div className="flex items-center justify-between p-2.5 rounded-lg border border-border bg-muted/30">
                  <Label className="text-sm">{t('gateway.removeEnvNoise')}</Label>
                  <Switch checked={!!config.filterEnvNoise} onCheckedChange={(checked: boolean) => setField('filterEnvNoise', checked)} />
                </div>
              </div>
              <div className="flex items-center justify-between p-3 border rounded-lg bg-muted/20">
                <div className="text-sm text-muted-foreground">
                  {config.promptFilterRules?.length > 0
                    ? `${config.promptFilterRules.length} ${t('gateway.customRules')}, ${config.promptFilterRules.filter((r: any) => r.enabled).length} ${t('gateway.enabled')}`
                    : t('gateway.noCustomRules')}
                </div>
                <Button size="sm" variant="outline" className="h-7 text-sm" onClick={() => setShowPromptFilterRulesDialog(true)}>
                  {t('gateway.manageRules')}
                </Button>
              </div>
            </div>

            {/* Section 4: 安全与高级 */}
            <div className="space-y-3">
              <div className="text-sm font-medium text-foreground flex items-center gap-2">
                <div className="w-1 h-4 bg-primary rounded-full"></div>
                {t('gateway.securityAndAdvanced')}
              </div>
              <div className="grid grid-cols-3 md:grid-cols-4 gap-2">
                <div className="flex items-center justify-between p-2.5 rounded-lg border border-border bg-muted/30">
                  <Label className="text-sm">{t('gateway.localOnly')}</Label>
                  <Switch
                    checked={!!config.localOnly}
                    onCheckedChange={(checked: boolean) => {
                      setConfig((prev: any) => applyGatewayLocalOnlyChange(prev, checked, createGeneratedApiKey))
                    }}
                  />
                </div>
                <div className="flex items-center justify-between p-2.5 rounded-lg border border-border bg-muted/30">
                  <Label className="text-sm">{t('gateway.autoStart')}</Label>
                  <Switch checked={!!config.enabled} onCheckedChange={handleAutoStartToggle} />
                </div>
                <div className="flex items-center justify-between p-2.5 rounded-lg border border-border bg-muted/30">
                  <Label className="text-sm">{t('gateway.responseCache')}</Label>
                  <Switch checked={!!config.responseCacheEnabled} onCheckedChange={(checked: boolean) => setField('responseCacheEnabled', checked)} />
                </div>
                <div className="flex flex-col gap-0.5 p-2.5 rounded-lg border border-border bg-muted/30">
                  <Label className="text-xs text-muted-foreground">{t('gateway.cacheTTLSeconds')}</Label>
                  <Input
                    type="number"
                    value={config.responseCacheTtl}
                    min={30}
                    max={3600}
                    className="h-6 text-sm px-1.5"
                    onChange={(e: React.ChangeEvent<HTMLInputElement>) => setField('responseCacheTtl', Number(e.target.value) || 180)}
                    disabled={!config.responseCacheEnabled}
                  />
                </div>
                <div className="flex flex-col gap-0.5 p-2.5 rounded-lg border border-border bg-muted/30">
                  <Label className="text-xs text-muted-foreground">{t('gateway.promptCacheHitPercent')}</Label>
                  <Input
                    type="number"
                    value={config.promptCacheTargetPercent}
                    min={0}
                    max={100}
                    className="h-6 text-sm px-1.5"
                    onChange={(e: React.ChangeEvent<HTMLInputElement>) => setField('promptCacheTargetPercent', Number(e.target.value))}
                  />
                </div>
                <div className="flex flex-col gap-0.5 p-2.5 rounded-lg border border-border bg-muted/30">
                  <Label className="text-xs text-muted-foreground">{t('gateway.thresholdPercent')}</Label>
                  <Input
                    type="number"
                    value={config.threshold}
                    min={1}
                    max={100}
                    className="h-6 text-sm px-1.5"
                    onChange={(e: React.ChangeEvent<HTMLInputElement>) => setField('threshold', Number(e.target.value) || 90)}
                  />
                </div>
              </div>

              {!config.localOnly && (
                <div className="flex flex-col gap-1.5">
                  <Label>{t('gateway.ipWhitelist')}</Label>
                  <Textarea
                    placeholder={'192.168.1.10\n10.0.0.0/24'}
                    rows={2}
                    value={config.allowedIpsText}
                    onChange={(e: React.ChangeEvent<HTMLTextAreaElement>) => setField('allowedIpsText', e.target.value)}
                    className={fieldErrors.allowedIpsText ? 'border-red-500' : ''}
                  />
                  {fieldErrors.allowedIpsText && <div className="text-xs text-red-500">{fieldErrors.allowedIpsText}</div>}
                </div>
              )}
            </div>
          </div>
        </div>
      </GatewaySurfaceCard>

      {/* AccountPoolDialog */}
      <DialogRoot open={showAccountPoolDialog} onOpenChange={setShowAccountPoolDialog}>
        <DialogContent maxWidth="720px">
          <DialogHeader icon={Users}>
            <DialogTitle>配置账号池</DialogTitle>
            <DialogDescription>
              选择参与网关轮换的账号，并配置请求分发策略。
            </DialogDescription>
          </DialogHeader>

          <DialogBody className="space-y-4">
            <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
              <div className="rounded-xl border border-border bg-muted/20 p-3">
                <div className="text-xs text-muted-foreground">已选择账号</div>
                <div className="mt-1 text-2xl font-semibold text-foreground">{selectedPoolAccountIds.length}</div>
              </div>
              <div className="rounded-xl border border-border bg-muted/20 p-3">
                <div className="flex items-center justify-between gap-2">
                  <div className="text-xs text-muted-foreground">路由策略</div>
                  {!config.strategy && <Badge variant="outline" className="h-5 rounded-full px-2 text-[10px]">默认：轮询</Badge>}
                </div>
                <Select value={effectiveStrategy} onValueChange={(v: string) => setField('strategy', v || 'round_robin')}>
                  <SelectTrigger className="mt-1 h-9">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="round_robin"><div className="flex items-center gap-2"><RotateCw size={14} /><span>{t('gateway.roundRobin')}</span></div></SelectItem>
                    <SelectItem value="most_quota"><div className="flex items-center gap-2"><TrendingUp size={14} /><span>{t('gateway.priorityQuota')}</span></div></SelectItem>
                    <SelectItem value="random"><div className="flex items-center gap-2"><Shuffle size={14} /><span>{t('gateway.random')}</span></div></SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </div>

            <div className="flex items-center justify-between rounded-xl border border-border bg-background/60 px-3 py-2">
              <div>
                <div className="text-sm font-medium text-foreground">账号列表</div>
                <div className="text-xs text-muted-foreground">{accountOptions.length} 个可用账号</div>
              </div>
              <Button type="button" variant="outline" size="sm" className="h-8" onClick={toggleAllPoolAccounts} disabled={accountOptions.length === 0}>
                {selectedPoolAccountIds.length === accountOptions.length && accountOptions.length > 0 ? '取消全选' : '全选'}
              </Button>
            </div>

            <div className="max-h-[360px] space-y-2 overflow-y-auto pr-1">
              {accountOptions.length === 0 ? (
                <div className="rounded-xl border border-dashed border-border p-8 text-center text-sm text-muted-foreground">
                  暂无可用账号，请先添加账号。
                </div>
              ) : (
                accountOptions.map((account: any) => {
                  const checked = selectedPoolAccountIds.includes(account.value)
                  return (
                    <button
                      key={account.value}
                      type="button"
                      className={`flex w-full items-center gap-3 rounded-xl border p-3 text-left transition-all ${checked ? 'border-primary bg-primary/10 ring-1 ring-primary/20' : 'border-border bg-background/70 hover:border-primary/40 hover:bg-primary/5'}`}
                      onClick={() => togglePoolAccount(account.value)}
                    >
                      <Checkbox checked={checked} onCheckedChange={() => togglePoolAccount(account.value)} onClick={(e) => e.stopPropagation()} />
                      <div className="min-w-0 flex-1">
                        <div className="truncate text-sm font-medium text-foreground">{account.label}</div>
                        {account.description && (
                          <div className="mt-0.5 truncate text-xs text-muted-foreground">{account.description}</div>
                        )}
                      </div>
                      {checked && (
                        <Badge variant="secondary" className="gap-1 rounded-full">
                          <CheckCircle2 size={12} />
                          已选
                        </Badge>
                      )}
                    </button>
                  )
                })
              )}
            </div>

            {selectedPoolAccounts.length > 0 && (
              <div className="rounded-xl border border-border bg-muted/20 p-3">
                <div className="mb-2 text-xs font-medium text-muted-foreground">当前账号池</div>
                <div className="flex flex-wrap gap-2">
                  {selectedPoolAccounts.map((account: any) => (
                    <Badge key={account.value} variant="outline" className="rounded-full bg-background/70">
                      {account.label}
                    </Badge>
                  ))}
                </div>
              </div>
            )}
          </DialogBody>

          <DialogFooter>
            <Button variant="outline" onClick={() => setShowAccountPoolDialog(false)}>关闭</Button>
            <Button onClick={() => { setShowAccountPoolDialog(false); handleSaveConfig() }}>保存配置</Button>
          </DialogFooter>
        </DialogContent>
      </DialogRoot>

      {/* ModelMappingDialog */}
      <ModelMappingDialog
        open={showModelMappingDialog}
        onOpenChange={setShowModelMappingDialog}
        modelMappings={config.modelMappings}
        setField={setField}
        onSave={handleSaveConfig}
      />

      {/* ApiKeysDialog */}
      <ApiKeysDialog
        open={showApiKeysDialog}
        onOpenChange={setShowApiKeysDialog}
        clientApiKeysText={config.clientApiKeysText}
        setConfig={setConfig}
        onSave={handleSaveConfig}
      />

      {/* PromptFilterRulesDialog */}
      <PromptFilterRulesDialog
        open={showPromptFilterRulesDialog}
        onOpenChange={setShowPromptFilterRulesDialog}
        promptFilterRules={config.promptFilterRules}
        setField={setField}
        onSave={handleSaveConfig}
      />
    </div>
  )
}

export default GatewayConfig

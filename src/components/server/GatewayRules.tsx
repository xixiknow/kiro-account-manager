import { useEffect, useMemo, useState } from 'react'
import { CheckCircle2, Filter, KeyRound, ListChecks, Route, Shuffle } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
import { Badge } from '@/components/ui/badge'
import ModelMappingDialog from '@/components/features/Gateway/ModelMappingDialog'
import PromptFilterRulesDialog from '@/components/features/Gateway/PromptFilterRulesDialog'
import ApiKeysDialog from '@/components/features/Gateway/ApiKeysDialog'
import {
  DEFAULT_GATEWAY_CONFIG,
  GatewayConfig,
  buildGatewayPayload,
  hydrateGatewayConfig,
} from '@/components/features/Gateway/gatewayPageState'
import { adminFetch } from '@/server/adminClient'

function accountLabel(account: any) {
  return account.label || account.email || account.userId || account.user_id || account.id
}

export default function GatewayRules() {
  const [config, setConfig] = useState<GatewayConfig>(DEFAULT_GATEWAY_CONFIG)
  const [accounts, setAccounts] = useState<any[]>([])
  const [groups, setGroups] = useState<any[]>([])
  const [saving, setSaving] = useState(false)
  const [saved, setSaved] = useState(false)
  const [saveMessage, setSaveMessage] = useState('')
  const [error, setError] = useState('')
  const [showMappings, setShowMappings] = useState(false)
  const [showFilters, setShowFilters] = useState(false)
  const [showKeys, setShowKeys] = useState(false)

  const setField = (key: string, value: any) => setConfig(prev => ({ ...prev, [key]: value }))

  const load = async () => {
    try {
      setError('')
      const [gatewayConfig, accountList, groupsList] = await Promise.all([
        adminFetch('/admin/api/gateway/config'),
        adminFetch('/admin/api/accounts'),
        adminFetch('/admin/api/groups-tags'),
      ])
      setConfig(hydrateGatewayConfig(gatewayConfig))
      setAccounts(Array.isArray(accountList) ? accountList : [])
      setGroups(Array.isArray(groupsList?.groups) ? groupsList.groups : [])
    } catch (err) {
      setError(String((err as any)?.message || err))
    }
  }

  useEffect(() => {
    load()
  }, [])

  const save = async () => {
    setSaving(true)
    setSaved(false)
    setSaveMessage('')
    setError('')
    try {
      const result = await adminFetch<any>('/admin/api/gateway/config', {
        method: 'PUT',
        body: JSON.stringify(buildGatewayPayload(config)),
      })
      setSaved(true)
      setSaveMessage(result?.restartRequired
        ? '已保存；host/port 需要重启容器后切换监听地址'
        : '已保存并实时生效')
      window.setTimeout(() => {
        setSaved(false)
        setSaveMessage('')
      }, 1800)
    } catch (err) {
      setError(String((err as any)?.message || err))
    } finally {
      setSaving(false)
    }
  }

  const enabledMappings = useMemo(
    () => (config.modelMappings || []).filter((rule: any) => rule.enabled).length,
    [config.modelMappings],
  )
  const enabledFilters = useMemo(
    () => (config.promptFilterRules || []).filter((rule: any) => rule.enabled).length,
    [config.promptFilterRules],
  )

  return (
    <div className="h-full overflow-y-auto p-5">
      <div className="mx-auto flex max-w-6xl flex-col gap-4">
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-3">
            <div className="grid h-10 w-10 place-items-center rounded-lg bg-emerald-400 text-[#101114]">
              <ListChecks size={20} />
            </div>
            <div>
              <h1 className="text-lg font-semibold text-foreground">网关规则管理</h1>
              <p className="text-sm text-muted-foreground">只管理 Kiro2Api 规则：模型映射、Prompt 过滤、账号池策略和 API keys。</p>
            </div>
          </div>
          <Button onClick={save} disabled={saving} className="gap-2 rounded-lg bg-emerald-400 text-[#101114] hover:bg-emerald-300">
            <CheckCircle2 size={16} />
            {saving ? '保存中...' : saved ? '已保存' : '保存规则'}
          </Button>
        </div>

        {error && (
          <div className="rounded-lg border border-red-500/25 bg-red-500/10 px-3 py-2 text-sm text-red-300">
            {error}
          </div>
        )}
        {saveMessage && (
          <div className="rounded-lg border border-emerald-500/25 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200">
            {saveMessage}
          </div>
        )}

        <div className="grid grid-cols-1 gap-3 lg:grid-cols-3">
          <section className="rounded-lg border border-border bg-card p-4">
            <div className="mb-4 flex items-center justify-between">
              <div className="flex items-center gap-2 text-sm font-semibold">
                <Shuffle size={16} className="text-emerald-300" />
                模型映射
              </div>
              <Badge variant="secondary">{enabledMappings}/{config.modelMappings?.length || 0}</Badge>
            </div>
            <p className="mb-4 text-sm leading-6 text-muted-foreground">把客户端模型名映射到一个或多个 Kiro 可用模型，可配置权重和启停。</p>
            <Button variant="outline" className="w-full justify-start gap-2" onClick={() => setShowMappings(true)}>
              <Shuffle size={15} />
              管理映射规则
            </Button>
          </section>

          <section className="rounded-lg border border-border bg-card p-4">
            <div className="mb-4 flex items-center justify-between">
              <div className="flex items-center gap-2 text-sm font-semibold">
                <Filter size={16} className="text-amber-300" />
                Prompt 过滤
              </div>
              <Badge variant="secondary">{enabledFilters}/{config.promptFilterRules?.length || 0}</Badge>
            </div>
            <div className="mb-4 grid gap-2">
              {[
                ['filterClaudeCode', '简化 Claude Code Prompt'],
                ['filterStripBoundaries', '移除 Boundary 标记'],
                ['filterEnvNoise', '移除环境噪声'],
              ].map(([key, label]) => (
                <div key={key} className="flex items-center justify-between rounded-md border border-border bg-muted/20 px-3 py-2">
                  <span className="text-sm text-muted-foreground">{label}</span>
                  <Switch checked={!!(config as any)[key]} onCheckedChange={(checked) => setField(key, checked)} />
                </div>
              ))}
            </div>
            <Button variant="outline" className="w-full justify-start gap-2" onClick={() => setShowFilters(true)}>
              <Filter size={15} />
              管理自定义规则
            </Button>
          </section>

          <section className="rounded-lg border border-border bg-card p-4">
            <div className="mb-4 flex items-center justify-between">
              <div className="flex items-center gap-2 text-sm font-semibold">
                <KeyRound size={16} className="text-sky-300" />
                API Keys 与缓存
              </div>
              <Badge variant="secondary">{(config.clientApiKeysText || '').split(/\n|,/).filter(Boolean).length} keys</Badge>
            </div>
            <div className="mb-4 space-y-3">
              <div>
                <Label className="text-xs text-muted-foreground">Prompt Cache 目标命中率</Label>
                <Input
                  type="number"
                  min={0}
                  max={100}
                  value={config.promptCacheTargetPercent}
                  onChange={(event) => setField('promptCacheTargetPercent', Number(event.target.value) || 0)}
                  className="mt-1 h-9"
                />
              </div>
              <div>
                <Label className="text-xs text-muted-foreground">Prompt Cache TTL (秒)</Label>
                <Input
                  type="number"
                  min={30}
                  max={3600}
                  value={config.promptCacheTtlSecs}
                  onChange={(event) => setField('promptCacheTtlSecs', Number(event.target.value) || 300)}
                  className="mt-1 h-9"
                />
              </div>
              <div>
                <Label className="text-xs text-muted-foreground">Prompt Cache 最大条目数</Label>
                <Input
                  type="number"
                  min={1}
                  value={config.promptCacheMaxEntries}
                  onChange={(event) => setField('promptCacheMaxEntries', Number(event.target.value) || 2000)}
                  className="mt-1 h-9"
                />
              </div>
              <div className="flex items-center justify-between">
                <Label className="text-xs text-muted-foreground">忽略客户端 cache_control</Label>
                <Switch
                  checked={!!config.promptCacheIgnoreClientControl}
                  onCheckedChange={(checked) => setField('promptCacheIgnoreClientControl', checked)}
                />
              </div>
              <Button variant="outline" className="w-full justify-start gap-2" onClick={() => setShowKeys(true)}>
                <KeyRound size={15} />
                管理客户端 Key
              </Button>
            </div>
          </section>
        </div>

        <section className="rounded-lg border border-border bg-card p-4">
          <div className="mb-4 flex items-center gap-2 text-sm font-semibold">
            <Route size={16} className="text-emerald-300" />
            路由与账号池策略
          </div>
          <div className="grid grid-cols-1 gap-3 md:grid-cols-4">
            <div>
              <Label>账号模式</Label>
              <Select value={config.accountMode} onValueChange={(value) => setField('accountMode', value)}>
                <SelectTrigger className="mt-1">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="single">单账号</SelectItem>
                  <SelectItem value="group">按分组</SelectItem>
                  <SelectItem value="pool">账号池</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div>
              <Label>单账号</Label>
              <Select value={config.accountId || ''} onValueChange={(value) => setField('accountId', value || null)}>
                <SelectTrigger className="mt-1">
                  <SelectValue placeholder="选择账号" />
                </SelectTrigger>
                <SelectContent>
                  {accounts.map(account => (
                    <SelectItem key={account.id} value={account.id}>{accountLabel(account)}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div>
              <Label>分组</Label>
              <Select value={config.groupId || ''} onValueChange={(value) => setField('groupId', value || null)}>
                <SelectTrigger className="mt-1">
                  <SelectValue placeholder="选择分组" />
                </SelectTrigger>
                <SelectContent>
                  {groups.map(group => (
                    <SelectItem key={group.id} value={group.id}>{group.name}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div>
              <Label>策略</Label>
              <Select value={config.strategy || 'round_robin'} onValueChange={(value) => setField('strategy', value)}>
                <SelectTrigger className="mt-1">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="round_robin">轮询</SelectItem>
                  <SelectItem value="random">随机</SelectItem>
                  <SelectItem value="most_quota">最多配额优先</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>
        </section>
      </div>

      <ModelMappingDialog
        open={showMappings}
        onOpenChange={setShowMappings}
        modelMappings={config.modelMappings}
        setField={setField}
        onSave={save}
      />
      <PromptFilterRulesDialog
        open={showFilters}
        onOpenChange={setShowFilters}
        promptFilterRules={config.promptFilterRules}
        setField={setField}
        onSave={save}
      />
      <ApiKeysDialog
        open={showKeys}
        onOpenChange={setShowKeys}
        clientApiKeysText={config.clientApiKeysText}
        setConfig={setConfig}
        onSave={save}
      />
    </div>
  )
}

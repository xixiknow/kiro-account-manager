import React, { useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@/components/ui/button'
import {
  DialogRoot,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogBody
} from '@/components/shared/dialog'
import { Badge } from '@/components/ui/badge'
import { CheckCircle2, XCircle, Activity, Info, Play, Network } from 'lucide-react'
import { toast } from 'sonner'
import { useApp } from '../../../hooks/useApp'
import type { GatewayConfig } from './gatewayPageState'

interface RouteTestResult {
  matched_accounts: string[]
  selected_account: string | null
  error: string | null
}

interface RouteTestDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  config: GatewayConfig
}

export function RouteTestDialog({ open, onOpenChange, config }: RouteTestDialogProps) {
  const { t } = useApp()
  const [result, setResult] = useState<RouteTestResult | null>(null)
  const [isTesting, setIsTesting] = useState(false)

  const handleTest = async () => {
    setIsTesting(true)
    try {
      // 桌面端走原生 Tauri invoke；server-web 端经 core.ts shim 转发到 /admin/api/invoke/test_route_config
      const testResult = await invoke<RouteTestResult>('test_route_config', { config })
      setResult(testResult)
      if (testResult.error) {
        toast.error(`${t('gateway.routeTestFailed')}: ${testResult.error}`)
      } else {
        toast.success(t('gateway.routeTestCompleted'))
      }
    } catch (error) {
      toast.error(`${t('gateway.testFailed')}: ${error}`)
      setResult(null)
    } finally {
      setIsTesting(false)
    }
  }

  const handleOpenChange = (newOpen: boolean) => {
    onOpenChange(newOpen)
    if (!newOpen) {
      setResult(null)
    }
  }

  return (
    <DialogRoot open={open} onOpenChange={handleOpenChange}>
      <DialogContent maxWidth="600px" className="max-h-[85vh]">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Network className="w-5 h-5 text-primary" />
            {t('gateway.routeAllocationTest')}
          </DialogTitle>
          <DialogDescription>
            {t('gateway.testCurrentConfig')}
          </DialogDescription>
        </DialogHeader>

        <DialogBody className="pt-2">
          {/* 顶部当前配置汇�?*/}
          <div className="space-y-3 py-3 border-y bg-muted/20 px-4 rounded-lg">
            <div className="grid grid-cols-2 gap-2 text-sm">
              <div className="flex items-center gap-3">
                <span className="text-muted-foreground w-16">{t('gateway.accountMode')}:</span>
                <Badge variant={config.accountMode === 'single' ? 'default' : config.accountMode === 'group' ? 'secondary' : 'outline'}>
                  {config.accountMode === 'single' ? t('gateway.singleAccountMode') : config.accountMode === 'group' ? t('gateway.groupMode') : t('gateway.accountPoolMode')}
                </Badge>
              </div>

              {config.accountMode === 'single' && config.accountId && (
                <div className="flex items-center gap-3">
                  <span className="text-muted-foreground w-16">{t('gateway.accountId')}:</span>
                  <span className="font-mono text-xs bg-background px-2 py-0.5 rounded border">{config.accountId}</span>
                </div>
              )}

              {config.accountMode === 'group' && config.groupId && (
                <div className="flex items-center gap-3">
                  <span className="text-muted-foreground w-16">{t('gateway.groupId')}:</span>
                  <span className="font-mono text-xs bg-background px-2 py-0.5 rounded border">{config.groupId}</span>
                </div>
              )}

              <div className="flex items-center gap-3">
                <span className="text-muted-foreground w-16">{t('gateway.balanceStrategy')}:</span>
                <Badge variant="outline" className="capitalize">
                  {config.strategy || t('gateway.roundRobinDefault')}
                </Badge>
              </div>
            </div>
          </div>

          {/* 核心动作 */}
          <div className="flex flex-col items-center justify-center py-4 gap-2">
            <Button onClick={handleTest} disabled={isTesting} size="lg" className="px-8 shadow-sm">
              {isTesting ? (
                <Activity className="w-4 h-4 mr-2 animate-spin" />
              ) : (
                <Play className="w-4 h-4 mr-2" />
              )}
              {isTesting ? t('gateway.calculatingAllocation') : t('gateway.startRouteTest')}
            </Button>
          </div>

          {/* 说明与结果区 */}
          <div className="space-y-4 min-h-[180px]">
            {result ? (
              <div className="space-y-4">
              {/* 匹配的账�?*/}
              <div className="space-y-2 border rounded-lg p-3 bg-muted/15">
                <div className="flex items-center gap-2 border-b pb-2">
                  {result.matched_accounts.length > 0 ? (
                    <>
                      <CheckCircle2 className="w-4 h-4 text-green-600 animate-pulse" />
                      <span className="font-semibold text-sm">{t('gateway.matchedCandidateAccounts', { count: result.matched_accounts.length })}</span>
                    </>
                  ) : (
                    <>
                      <XCircle className="w-4 h-4 text-destructive" />
                      <span className="font-semibold text-sm">{t('gateway.noMatchedAccounts')}</span>
                    </>
                  )}
                </div>

                {result.matched_accounts.length > 0 ? (
                  <div className="grid grid-cols-1 sm:grid-cols-2 gap-2 pt-1">
                    {result.matched_accounts.map((account, index) => {
                      const isSelected = result.selected_account === account;
                      return (
                        <div
                          key={index}
                          className={`text-xs font-mono p-2 rounded-lg border transition-all flex items-center justify-between ${isSelected
                              ? 'bg-primary/5 border-primary text-primary font-semibold'
                              : 'bg-background border-border text-muted-foreground'
                            }`}
                        >
                          <span className="truncate">{account}</span>
                          {isSelected && <Badge className="h-4 text-[9px] px-1">{t('gateway.selectedBadge')}</Badge>}
                        </div>
                      );
                    })}
                  </div>
                ) : (
                  <p className="text-xs text-muted-foreground p-2">{t('gateway.confirmConfig')}</p>
                )}
              </div>

              {/* 负载均衡选择结果 */}
              {result.selected_account && (
                <div className="space-y-2 border rounded-lg p-3 bg-blue-500/5 border-blue-500/20">
                  <div className="flex items-center gap-2 border-b pb-2 text-blue-600 dark:text-blue-400">
                    <CheckCircle2 className="w-4 h-4" />
                    <span className="font-semibold text-sm">{t('gateway.loadBalancerAllocation')}</span>
                  </div>
                  <div className="pt-1">
                    <div className="text-sm font-mono p-3 rounded-lg bg-blue-500/10 dark:bg-blue-950/20 border border-blue-500/30 text-blue-700 dark:text-blue-300 font-semibold">
                      {result.selected_account}
                    </div>
                  </div>
                </div>
              )}

              {/* 错误信息 */}
              {result.error && (
                <div className="space-y-2 border rounded-lg p-3 bg-destructive/5 border-destructive/20 text-destructive">
                  <div className="flex items-center gap-2 border-b pb-2">
                    <XCircle className="w-4 h-4" />
                    <span className="font-semibold text-sm">{t('gateway.errorHint')}</span>
                  </div>
                  <div className="pt-1">
                    <div className="text-sm p-3 rounded-lg bg-destructive/10 dark:bg-destructive/950/20 border border-destructive/30">
                      {result.error}
                    </div>
                  </div>
                </div>
              )}
              </div>
            ) : (
              <div className="flex flex-col gap-3 py-6">
              {/* 帮助提示�?*/}
              <div className="flex items-start gap-2.5 p-4 rounded-lg bg-muted/40 text-xs leading-relaxed border">
                <Info size={16} className="text-primary mt-0.5 shrink-0" />
                <div className="space-y-2 text-muted-foreground">
                  <p className="font-semibold text-foreground">{t('gateway.howRouteTestWorks')}</p>
                  <p>{t('gateway.routeTestStep1')}</p>
                  <p>{t('gateway.routeTestStep2')}</p>
                  <p className="text-orange-500/90">{t('gateway.routeTestTip')}</p>
                </div>
              </div>
              </div>
            )}
          </div>
        </DialogBody>
      </DialogContent>
    </DialogRoot>
  )
}

export default RouteTestDialog

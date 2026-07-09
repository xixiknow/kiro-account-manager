import { Suspense, useEffect, useMemo, useState } from 'react'
import { LogOut, RefreshCw, Server, Shield } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { serverRoutes } from '@/server-routes'
import { adminFetch, logoutAdmin } from '@/server/adminClient'

const routeMap = Object.fromEntries(serverRoutes.map(route => [route.id, route.component]))

function PageLoading() {
  return (
    <div className="grid h-full place-items-center text-sm text-muted-foreground">
      加载中...
    </div>
  )
}

interface ServerShellProps {
  onLogout: () => void
}

export default function ServerShell({ onLogout }: ServerShellProps) {
  const [activeMenu, setActiveMenu] = useState(() => localStorage.getItem('kamServerActiveMenu') || 'home')
  const [status, setStatus] = useState<any>(null)
  const [refreshing, setRefreshing] = useState(false)

  const activeRoute = serverRoutes.find(route => route.id === activeMenu) || serverRoutes[0]
  const ActiveComponent = routeMap[activeRoute.id] || routeMap.home

  const loadStatus = async () => {
    setRefreshing(true)
    try {
      const nextStatus = await adminFetch('/admin/api/status')
      setStatus(nextStatus)
    } catch {
      setStatus(null)
    } finally {
      setRefreshing(false)
    }
  }

  useEffect(() => {
    localStorage.setItem('kamServerActiveMenu', activeMenu)
  }, [activeMenu])

  useEffect(() => {
    loadStatus()
    const timer = window.setInterval(loadStatus, 12000)
    return () => window.clearInterval(timer)
  }, [])

  const routeProps = useMemo<Record<string, any>>(() => ({
    home: { onNavigate: setActiveMenu },
    desktopOAuth: { onLogin: () => setActiveMenu('accounts') },
    accounts: { onNavigate: setActiveMenu },
  }), [])

  const handleLogout = async () => {
    await logoutAdmin()
    onLogout()
  }

  return (
    <div className="flex h-screen w-full overflow-hidden bg-[#111214] text-foreground">
      <aside className="flex w-[248px] shrink-0 flex-col border-r border-white/10 bg-[#151619]">
        <div className="border-b border-white/10 px-4 py-4">
          <div className="flex items-center gap-3">
            <div className="grid h-10 w-10 place-items-center rounded-lg bg-emerald-400 text-[#101114]">
              <Server size={20} />
            </div>
            <div className="min-w-0">
              <div className="truncate text-sm font-semibold text-zinc-100">KAM Server</div>
              <div className="text-xs text-zinc-500">React Console</div>
            </div>
          </div>
        </div>

        <nav className="flex-1 space-y-1 overflow-y-auto px-3 py-3">
          {serverRoutes.map((route) => {
            const Icon = route.icon
            const active = route.id === activeRoute.id
            return (
              <button
                key={route.id}
                onClick={() => setActiveMenu(route.id)}
                className={cn(
                  'flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-left transition-colors',
                  active
                    ? 'bg-emerald-400 text-[#101114]'
                    : 'text-zinc-300 hover:bg-white/10 hover:text-zinc-50',
                )}
              >
                <Icon size={18} />
                <span className="min-w-0 flex-1">
                  <span className="block text-sm font-medium">{route.label}</span>
                  <span className={cn('block truncate text-[11px]', active ? 'text-[#101114]/70' : 'text-zinc-500')}>
                    {route.desc}
                  </span>
                </span>
              </button>
            )
          })}
        </nav>

        <div className="border-t border-white/10 p-3">
          <Button variant="ghost" className="h-9 w-full justify-start gap-2 text-zinc-300 hover:bg-white/10 hover:text-zinc-50" onClick={handleLogout}>
            <LogOut size={16} />
            退出后台
          </Button>
        </div>
      </aside>

      <section className="flex min-w-0 flex-1 flex-col">
        <header className="flex h-14 shrink-0 items-center justify-between border-b border-white/10 bg-[#17181b] px-4">
          <div>
            <div className="text-sm font-semibold text-zinc-100">{activeRoute.label}</div>
            <div className="text-xs text-zinc-500">{activeRoute.desc}</div>
          </div>
          <div className="flex items-center gap-2">
            <div className="hidden items-center gap-2 rounded-lg border border-white/10 bg-black/20 px-3 py-1.5 text-xs text-zinc-400 md:flex">
              <Shield size={14} className="text-emerald-300" />
              <span>{status?.gateway?.running ? 'Gateway 运行中' : 'Gateway 已停止'}</span>
              <span className="text-zinc-600">/</span>
              <span>{status?.stats?.totalRequests ?? status?.gateway?.requestCount ?? 0} req</span>
            </div>
            <Button variant="ghost" size="icon" className="h-8 w-8 text-zinc-400 hover:bg-white/10 hover:text-zinc-50" onClick={loadStatus}>
              <RefreshCw size={15} className={refreshing ? 'animate-spin' : ''} />
            </Button>
          </div>
        </header>

        <main className="min-h-0 flex-1 overflow-hidden bg-[radial-gradient(circle_at_top_left,rgba(52,211,153,0.10),transparent_32%),#101114]">
          <Suspense fallback={<PageLoading />}>
            <ActiveComponent {...(routeProps[activeRoute.id] || {})} />
          </Suspense>
        </main>
      </section>
    </div>
  )
}

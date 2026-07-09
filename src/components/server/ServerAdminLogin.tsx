import { FormEvent, useState } from 'react'
import { LockKeyhole, Server, ShieldCheck } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { loginAdmin } from '@/server/adminClient'

interface ServerAdminLoginProps {
  onLogin: () => void
}

export default function ServerAdminLogin({ onLogin }: ServerAdminLoginProps) {
  const [token, setToken] = useState('')
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    if (!token.trim() || loading) return
    setLoading(true)
    setError('')
    try {
      await loginAdmin(token)
      onLogin()
    } catch (err) {
      setError(String((err as any)?.message || err || '后台登录失败'))
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="min-h-screen bg-[#101114] text-foreground">
      <div className="grid min-h-screen grid-cols-1 lg:grid-cols-[minmax(420px,0.92fr)_1.08fr]">
        <section className="flex min-h-screen flex-col justify-between border-r border-white/10 bg-[#151619] px-8 py-7">
          <div className="flex items-center gap-3">
            <div className="grid h-10 w-10 place-items-center rounded-lg bg-emerald-400 text-[#101114]">
              <Server size={20} />
            </div>
            <div>
              <div className="text-sm font-semibold tracking-wide">Kiro Account Manager</div>
              <div className="text-xs text-zinc-400">Server Console</div>
            </div>
          </div>

          <form onSubmit={submit} className="mx-auto w-full max-w-[420px]">
            <div className="mb-7">
              <div className="mb-3 inline-flex items-center gap-2 rounded-md border border-emerald-400/20 bg-emerald-400/10 px-2.5 py-1 text-xs text-emerald-300">
                <ShieldCheck size={13} />
                管理 API 受保护
              </div>
              <h1 className="text-2xl font-semibold tracking-normal text-zinc-50">登录服务端后台</h1>
              <p className="mt-2 text-sm leading-6 text-zinc-400">
                使用容器环境变量 <span className="font-mono text-zinc-200">KAM_ADMIN_TOKEN</span> 进入管理台。
              </p>
            </div>

            <label className="mb-2 block text-xs font-medium uppercase tracking-wide text-zinc-500">
              Admin Token
            </label>
            <div className="relative">
              <LockKeyhole className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-zinc-500" size={16} />
              <Input
                value={token}
                onChange={(event) => setToken(event.target.value)}
                type="password"
                autoFocus
                className="h-11 rounded-lg border-zinc-700 bg-[#0f1012] pl-10 text-zinc-100"
                placeholder="kam admin token"
              />
            </div>

            {error && (
              <div className="mt-3 rounded-lg border border-red-500/25 bg-red-500/10 px-3 py-2 text-sm text-red-300">
                {error}
              </div>
            )}

            <Button type="submit" disabled={loading || !token.trim()} className="mt-5 h-11 w-full rounded-lg bg-emerald-400 text-[#101114] hover:bg-emerald-300">
              {loading ? '登录中...' : '进入后台'}
            </Button>
          </form>

          <div className="text-xs leading-5 text-zinc-500">
            HTTPS、证书和公网入口由宿主机 Nginx 负责；容器只监听内网 HTTP。
          </div>
        </section>

        <section className="hidden min-h-screen bg-[#0f1012] p-7 lg:block">
          <div className="grid h-full grid-rows-[auto_1fr] rounded-xl border border-white/10 bg-[linear-gradient(180deg,#191a1d_0%,#111214_100%)]">
            <div className="flex items-center justify-between border-b border-white/10 px-5 py-4">
              <div className="text-sm font-medium text-zinc-200">运行面板预览</div>
              <div className="rounded-full bg-emerald-400/12 px-2 py-1 text-xs text-emerald-300">same-origin admin</div>
            </div>
            <div className="grid grid-cols-2 gap-4 p-5">
              {[
                ['账号池', 'Account pool, tags, proxy'],
                ['在线登录', 'Google, GitHub, BuilderId, IAM'],
                ['规则管理', 'Model mapping and prompt filters'],
                ['Kiro2Api', 'Gateway logs, metrics, Prompt Cache'],
              ].map(([title, desc]) => (
                <div key={title} className="rounded-lg border border-white/10 bg-black/20 p-4">
                  <div className="text-sm font-semibold text-zinc-100">{title}</div>
                  <div className="mt-2 text-xs leading-5 text-zinc-500">{desc}</div>
                </div>
              ))}
            </div>
          </div>
        </section>
      </div>
    </div>
  )
}

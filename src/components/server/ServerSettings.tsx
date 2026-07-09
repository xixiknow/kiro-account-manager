import { useEffect, useState } from 'react'
import { Database, Globe2, Palette, Server, Settings2 } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Switch } from '@/components/ui/switch'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { useApp } from '@/hooks/useApp'
import { useAppSettings } from '@/contexts/AppSettingsContext'
import { adminFetch } from '@/server/adminClient'

const themeOptions = [
  ['dark-one', 'Dark One'],
  ['dark', 'Dark'],
  ['tech', 'Tech'],
  ['midnight', 'Midnight'],
  ['green', 'Green'],
  ['ocean', 'Ocean'],
  ['rose', 'Rose'],
  ['sakura', 'Sakura'],
]

export default function ServerSettings() {
  const { theme, setTheme } = useApp()
  const { settings, updateSettings } = useAppSettings()
  const [status, setStatus] = useState<any>(null)
  const [about, setAbout] = useState<any>(null)
  const [saving, setSaving] = useState(false)

  const load = async () => {
    const [nextStatus, nextAbout] = await Promise.all([
      adminFetch('/admin/api/status').catch(() => null),
      adminFetch('/admin/api/about').catch(() => null),
    ])
    setStatus(nextStatus)
    setAbout(nextAbout)
  }

  useEffect(() => {
    load()
  }, [])

  const updateBool = async (key: string, value: boolean) => {
    setSaving(true)
    try {
      await updateSettings({ [key]: value } as any)
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="h-full overflow-y-auto p-5">
      <div className="mx-auto flex max-w-5xl flex-col gap-4">
        <div className="flex items-center gap-3">
          <div className="grid h-10 w-10 place-items-center rounded-lg bg-emerald-400 text-[#101114]">
            <Settings2 size={20} />
          </div>
          <div>
            <h1 className="text-lg font-semibold text-foreground">服务端设置</h1>
            <p className="text-sm text-muted-foreground">面向 Linux/Nginx/Docker 部署的安全设置。桌面端本机切号能力在服务端默认不可用。</p>
          </div>
        </div>

        <div className="grid grid-cols-1 gap-3 md:grid-cols-3">
          <section className="rounded-lg border border-border bg-card p-4">
            <div className="mb-3 flex items-center gap-2 text-sm font-semibold">
              <Server size={16} className="text-emerald-300" />
              运行信息
            </div>
            <InfoRow label="版本" value={about?.version || '-'} />
            <InfoRow label="模式" value={about?.mode || 'server'} />
            <InfoRow label="运行时长" value={`${Math.floor((status?.uptimeSeconds || 0) / 60)} min`} />
            <InfoRow label="Gateway" value={status?.gateway?.running ? '运行中' : '已停止'} />
          </section>

          <section className="rounded-lg border border-border bg-card p-4">
            <div className="mb-3 flex items-center gap-2 text-sm font-semibold">
              <Database size={16} className="text-sky-300" />
              数据目录
            </div>
            <InfoRow label="KAM_DATA_DIR" value={about?.dataDir || '-'} mono />
            <InfoRow label="备份范围" value="/data" mono />
            <p className="mt-3 text-xs leading-5 text-muted-foreground">账号、Gateway 配置、分组标签和日志均应随 `/data` 卷备份。</p>
          </section>

          <section className="rounded-lg border border-border bg-card p-4">
            <div className="mb-3 flex items-center gap-2 text-sm font-semibold">
              <Globe2 size={16} className="text-amber-300" />
              公网入口
            </div>
            <InfoRow label="Public URL" value={status?.publicBaseUrl || about?.publicBaseUrl || '-'} mono />
            <InfoRow label="HTTPS" value="宿主机 Nginx" />
            <InfoRow label="容器端口" value="8765/http" mono />
          </section>
        </div>

        <section className="rounded-lg border border-border bg-card p-4">
          <div className="mb-4 flex items-center gap-2 text-sm font-semibold">
            <Palette size={16} className="text-emerald-300" />
            外观
          </div>
          <div className="grid max-w-xl grid-cols-1 gap-3 md:grid-cols-[180px_1fr]">
            <div>
              <div className="text-sm text-muted-foreground">主题</div>
              <div className="text-xs text-muted-foreground/70">浏览器本地保存</div>
            </div>
            <Select value={theme} onValueChange={setTheme}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {themeOptions.map(([value, label]) => (
                  <SelectItem key={value} value={value}>{label}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </section>

        <section className="rounded-lg border border-border bg-card p-4">
          <div className="mb-4 flex items-center gap-2 text-sm font-semibold">
            <Settings2 size={16} className="text-emerald-300" />
            应用行为
          </div>
          <div className="grid gap-3">
            <ToggleRow
              title="隐私模式"
              desc="账号标识默认脱敏显示。"
              checked={settings?.privacyMode ?? true}
              disabled={saving}
              onChange={(checked) => updateBool('privacyMode', checked)}
            />
            <ToggleRow
              title="自动刷新账号"
              desc="后台页面刷新时同步账号列表和状态。"
              checked={settings?.autoRefresh ?? true}
              disabled={saving}
              onChange={(checked) => updateBool('autoRefresh', checked)}
            />
          </div>
        </section>

        <section className="rounded-lg border border-amber-400/20 bg-amber-400/10 p-4">
          <div className="text-sm font-semibold text-amber-200">服务端安全边界</div>
          <p className="mt-2 text-sm leading-6 text-amber-100/75">
            服务端 Web 后台不会默认执行“切换本机 Kiro IDE/CLI 登录态”、写访问者本机配置、打开宿主机 GUI 目录等桌面动作。需要读取 Kiro 会话数据时，请显式挂载对应目录。
          </p>
          <Button variant="outline" className="mt-3" onClick={load}>刷新运行信息</Button>
        </section>
      </div>
    </div>
  )
}

function InfoRow({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex items-center justify-between gap-3 border-b border-border/60 py-2 last:border-b-0">
      <span className="text-xs text-muted-foreground">{label}</span>
      <span className={`min-w-0 truncate text-right text-xs text-foreground ${mono ? 'font-mono' : ''}`} title={value}>
        {value}
      </span>
    </div>
  )
}

function ToggleRow({
  title,
  desc,
  checked,
  disabled,
  onChange,
}: {
  title: string
  desc: string
  checked: boolean
  disabled?: boolean
  onChange: (checked: boolean) => void
}) {
  return (
    <div className="flex items-center justify-between gap-4 rounded-lg border border-border bg-muted/20 px-3 py-3">
      <div>
        <div className="text-sm font-medium text-foreground">{title}</div>
        <div className="text-xs text-muted-foreground">{desc}</div>
      </div>
      <Switch checked={checked} disabled={disabled} onCheckedChange={onChange} />
    </div>
  )
}

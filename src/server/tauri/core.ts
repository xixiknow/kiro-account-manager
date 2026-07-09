import { adminFetch, logoutAdmin } from '../adminClient'
import { emit } from './event'

declare const __APP_VERSION__: string

const wait = (ms: number) => new Promise(resolve => setTimeout(resolve, ms))

function fallbackValue(command: string, args?: any) {
  switch (command) {
    case 'check_update':
      return { available: false, currentVersion: __APP_VERSION__ }
    case 'get_supported_providers':
      return ['Google', 'Github', 'BuilderId', 'Enterprise']
    case 'get_kiro_local_token':
    case 'get_custom_kiro_path':
    case 'read_cli_db_snapshot':
      return null
    case 'check_ide_installation':
      return {
        ide_installed: false,
        ide_executable_exists: false,
        serverMode: true,
        error_message: '服务端模式不会检测或切换宿主机 Kiro IDE 登录态',
      }
    case 'check_cli_installation':
      return { cli_installed: false, serverMode: true }
    case 'get_kiro_cli_default_path':
    case 'get_gateway_log_dir':
    case 'get_app_data_dir':
      return ''
    case 'get_mcp_tool_stats':
      return { estimatedTools: 0, unavailable: true }
    case 'get_system_machine_guid':
      return { machineGuid: 'server-managed', unavailable: true }
    case 'detect_installed_browsers':
      return []
    case 'detect_system_proxy':
      return { enabled: false, proxyServer: '', httpProxy: '', tunMode: false, tunInterface: null }
    case 'configure_proxy_clients':
      return (args?.clients || []).map((client: string) => ({
        client,
        success: false,
        error: '服务端模式不会写入访问者本机的客户端配置',
      }))
    case 'open_app_data_dir':
    case 'open_gateway_log_dir':
    case 'clear_all_cache':
    case 'cleanup_expired_cache':
    case 'cancel_kiro_login':
    case 'show_main_window':
      return null
    default:
      return undefined
  }
}

async function waitForSocialCallback(timeoutMs: number) {
  return new Promise<void>((resolve, reject) => {
    let settled = false
    const finish = (ok: boolean, message?: string) => {
      if (settled) return
      settled = true
      cleanup()
      ok ? resolve() : reject(new Error(message || '在线登录失败'))
    }
    const onWindowMessage = (event: MessageEvent) => {
      if (event.origin !== window.location.origin) return
      if (event.data?.type === 'kam-online-login-complete') {
        finish(!!event.data.ok, event.data.message)
      }
    }
    const channel = 'BroadcastChannel' in window ? new BroadcastChannel('kam-online-login') : null
    const onChannelMessage = (event: MessageEvent) => {
      if (event.data?.type === 'kam-online-login-complete') {
        finish(!!event.data.ok, event.data.message)
      }
    }
    const timer = window.setTimeout(() => finish(false, '在线登录等待超时，请重试'), timeoutMs)
    const cleanup = () => {
      window.clearTimeout(timer)
      window.removeEventListener('message', onWindowMessage)
      channel?.removeEventListener('message', onChannelMessage as any)
      channel?.close()
    }
    window.addEventListener('message', onWindowMessage)
    channel?.addEventListener('message', onChannelMessage as any)
  })
}

async function runSocialLogin(provider: string) {
  const begin = await adminFetch<any>('/admin/api/online-login/social/begin', {
    method: 'POST',
    body: JSON.stringify({ provider }),
  })
  window.open(begin.authorizeUrl || begin.authorize_url, '_blank', 'noopener,noreferrer,width=980,height=760')
  await waitForSocialCallback(Number(begin.expiresInSeconds || begin.expires_in_seconds || 900) * 1000)
  await emit('login-success', { provider })
  await emit('accounts-updated')
  return null
}

async function runIdcLogin(args: any) {
  const begin = await adminFetch<any>('/admin/api/online-login/idc/begin', {
    method: 'POST',
    body: JSON.stringify({
      provider: args?.provider || 'BuilderId',
      region: args?.region,
      startUrl: args?.startUrl,
    }),
  })

  const verificationUrl = begin.verificationUriComplete || begin.verification_uri_complete || begin.verificationUri
  if (verificationUrl) {
    window.open(verificationUrl, '_blank', 'noopener,noreferrer,width=980,height=760')
  }

  const state = begin.state
  const startedAt = Date.now()
  const maxMs = Number(begin.expiresInSeconds || begin.expires_in_seconds || 900) * 1000
  let intervalMs = Math.max(3, Number(begin.intervalSeconds || begin.interval_seconds || 5)) * 1000

  while (Date.now() - startedAt < maxMs) {
    await wait(intervalMs)
    const poll = await adminFetch<any>(`/admin/api/online-login/idc/poll?state=${encodeURIComponent(state)}`)
    if (poll.status === 'complete') {
      await emit('login-success', { provider: poll.provider, accountDisplayId: poll.accountDisplayId })
      await emit('accounts-updated')
      return poll
    }
    if (poll.status === 'error') {
      throw new Error(poll.message || 'AWS Device Code 登录失败')
    }
    intervalMs = Math.max(3, Number(poll.intervalSeconds || poll.interval_seconds || 5)) * 1000
  }

  throw new Error('AWS Device Code 登录等待超时，请重新发起登录')
}

export async function invoke<T = any>(command: string, args: any = {}): Promise<T> {
  if (command === 'logout') {
    await logoutAdmin()
    return null as T
  }
  if (command === 'kiro_login') {
    const provider = args?.provider || 'Google'
    if (provider === 'BuilderId' || provider === 'Enterprise') {
      return runIdcLogin(args) as Promise<T>
    }
    return runSocialLogin(provider) as Promise<T>
  }

  const fallback = fallbackValue(command, args)
  if (fallback !== undefined) {
    return fallback as T
  }

  try {
    return await adminFetch<T>(`/admin/api/invoke/${encodeURIComponent(command)}`, {
      method: 'POST',
      body: JSON.stringify(args || {}),
    })
  } catch (error: any) {
    if (error?.status === 501) {
      const safeFallback = fallbackValue(command, args)
      if (safeFallback !== undefined) {
        return safeFallback as T
      }
    }
    throw error?.message || error
  }
}

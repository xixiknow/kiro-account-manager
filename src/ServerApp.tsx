import { useEffect, useState } from 'react'
import type { ReactNode } from 'react'
import { Toaster } from 'react-hot-toast'
import { TooltipProvider } from '@/components/ui/tooltip'
import { ThemeProvider } from '@/components/theme-provider'
import { DialogProvider } from '@/contexts/DialogContext'
import { AppSettingsProvider } from '@/contexts/AppSettingsContext'
import { AccountProvider } from '@/contexts/AccountContext'
import { PrivacyProvider } from '@/contexts/PrivacyContext'
import { I18nProvider } from '@/i18n'
import { adminFetch, getAdminToken } from '@/server/adminClient'
import ServerAdminLogin from '@/components/server/ServerAdminLogin'
import ServerShell from '@/components/server/ServerShell'

function ServerProviders({ children }: { children: ReactNode }) {
  return (
    <I18nProvider>
      <AppSettingsProvider>
        <ThemeProvider
          attribute="data-theme"
          defaultTheme="dark-one"
          enableSystem={false}
          disableTransitionOnChange
          themes={[
            'light', 'dark', 'dark-one', 'tech', 'midnight',
            'purple', 'green', 'business', 'sunset', 'ocean',
            'forest', 'rose', 'aurora', 'sakura',
          ]}
        >
          <TooltipProvider>
            <DialogProvider>
              <PrivacyProvider>
                <AccountProvider>
                  {children}
                </AccountProvider>
              </PrivacyProvider>
            </DialogProvider>
          </TooltipProvider>
        </ThemeProvider>
      </AppSettingsProvider>
    </I18nProvider>
  )
}

export default function ServerApp() {
  const [checking, setChecking] = useState(true)
  const [authenticated, setAuthenticated] = useState(false)

  const checkAuth = async () => {
    if (!getAdminToken()) {
      setAuthenticated(false)
      setChecking(false)
      return
    }
    try {
      await adminFetch('/admin/api/status')
      setAuthenticated(true)
    } catch {
      setAuthenticated(false)
    } finally {
      setChecking(false)
    }
  }

  useEffect(() => {
    checkAuth()
  }, [])

  if (checking) {
    return (
      <div className="grid h-screen place-items-center bg-[#101114] text-sm text-zinc-400">
        正在连接 kam-server...
      </div>
    )
  }

  if (!authenticated) {
    return <ServerAdminLogin onLogin={() => setAuthenticated(true)} />
  }

  return (
    <ServerProviders>
      <ServerShell onLogout={() => setAuthenticated(false)} />
      <Toaster position="top-center" toastOptions={{ style: { marginTop: '64px' } }} />
    </ServerProviders>
  )
}

export function getCurrentWindow() {
  return {
    close: async () => window.close(),
    hide: async () => undefined,
    show: async () => undefined,
    minimize: async () => undefined,
    unminimize: async () => undefined,
    setFocus: async () => window.focus(),
  }
}

export const appWindow = getCurrentWindow()

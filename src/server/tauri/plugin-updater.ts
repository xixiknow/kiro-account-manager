declare const __APP_VERSION__: string

export async function check() {
  return {
    available: false,
    currentVersion: __APP_VERSION__,
    version: __APP_VERSION__,
    date: null,
    body: '',
    downloadAndInstall: async () => undefined,
  }
}

declare const __APP_VERSION__: string

export async function getVersion() {
  return __APP_VERSION__
}

export async function getName() {
  return 'Kiro Account Manager Server'
}

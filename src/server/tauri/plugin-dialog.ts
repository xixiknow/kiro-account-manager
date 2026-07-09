let lastSaveName = 'kiro-account-manager-export.json'

export async function save(options: any = {}) {
  const defaultPath = String(options.defaultPath || lastSaveName)
  lastSaveName = defaultPath.split(/[\\/]/).pop() || lastSaveName
  return `download://${lastSaveName}`
}

export async function open() {
  return null
}

export async function message(messageText: string) {
  window.alert(messageText)
}

export async function ask(messageText: string) {
  return window.confirm(messageText)
}

export async function confirm(messageText: string) {
  return window.confirm(messageText)
}

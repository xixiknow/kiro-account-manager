function downloadText(filename: string, contents: string) {
  const blob = new Blob([contents], { type: 'application/json;charset=utf-8' })
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = filename || 'kiro-account-manager-export.json'
  document.body.appendChild(anchor)
  anchor.click()
  anchor.remove()
  window.setTimeout(() => URL.revokeObjectURL(url), 1000)
}

export async function writeTextFile(path: string, contents: string) {
  const filename = String(path || '').replace(/^download:\/\//, '').split(/[\\/]/).pop()
  downloadText(filename || 'kiro-account-manager-export.json', contents)
}

export async function readTextFile() {
  throw new Error('服务端 Web 后台不支持直接读取访问者本机文件')
}

export async function exists() {
  return false
}

export async function mkdir() {
  return undefined
}

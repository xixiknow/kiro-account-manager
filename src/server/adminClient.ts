const TOKEN_KEY = 'kamAdminToken'

export function getAdminToken() {
  return localStorage.getItem(TOKEN_KEY) || ''
}

export function setAdminToken(token: string) {
  const trimmed = token.trim()
  if (trimmed) {
    localStorage.setItem(TOKEN_KEY, trimmed)
  } else {
    localStorage.removeItem(TOKEN_KEY)
  }
}

export function clearAdminToken() {
  localStorage.removeItem(TOKEN_KEY)
}

export async function adminFetch<T = any>(path: string, init: RequestInit = {}): Promise<T> {
  const token = getAdminToken()
  const headers = new Headers(init.headers)
  headers.set('Accept', 'application/json')
  if (!(init.body instanceof FormData) && init.body !== undefined && !headers.has('Content-Type')) {
    headers.set('Content-Type', 'application/json')
  }
  if (token) {
    headers.set('Authorization', `Bearer ${token}`)
  }

  const response = await fetch(path, {
    ...init,
    headers,
    credentials: 'same-origin',
  })

  const text = await response.text()
  const contentType = response.headers.get('content-type') || ''
  const data = text && contentType.includes('application/json')
    ? JSON.parse(text)
    : text

  if (!response.ok) {
    const message = typeof data === 'object' && data && 'error' in data
      ? String((data as any).error)
      : String(data || response.statusText)
    const error = new Error(message)
    ;(error as any).status = response.status
    throw error
  }

  return data as T
}

export async function loginAdmin(token: string) {
  await adminFetch('/admin/api/auth/login', {
    method: 'POST',
    body: JSON.stringify({ token }),
  })
  setAdminToken(token)
}

export async function logoutAdmin() {
  try {
    await adminFetch('/admin/api/auth/logout', { method: 'POST' })
  } finally {
    clearAdminToken()
  }
}

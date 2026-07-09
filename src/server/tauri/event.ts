export type UnlistenFn = () => void

type EventCallback<T = any> = (event: { event: string; payload: T }) => void

const listeners = new Map<string, Set<EventCallback>>()

export async function listen<T = any>(event: string, handler: EventCallback<T>): Promise<UnlistenFn> {
  const set = listeners.get(event) || new Set<EventCallback>()
  set.add(handler as EventCallback)
  listeners.set(event, set)
  return () => {
    set.delete(handler as EventCallback)
    if (set.size === 0) {
      listeners.delete(event)
    }
  }
}

export async function emit<T = any>(event: string, payload?: T) {
  const set = listeners.get(event)
  if (!set) return
  for (const handler of [...set]) {
    handler({ event, payload })
  }
}

export async function once<T = any>(event: string, handler: EventCallback<T>): Promise<UnlistenFn> {
  const unlisten = await listen<T>(event, (payload) => {
    unlisten()
    handler(payload)
  })
  return unlisten
}

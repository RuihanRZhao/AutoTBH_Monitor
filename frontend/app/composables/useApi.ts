// Thin wrapper around the Rust/axum backend (/api/*). All calls are read-only GETs
// except the explicit POST helpers. Base URL comes from runtimeConfig (Tauri → localhost:5260).
export function useApi() {
  const base = useRuntimeConfig().public.apiBase as string

  async function get<T = any>(path: string, params?: Record<string, any>): Promise<T> {
    const qs = params
      ? '?' +
        Object.entries(params)
          .filter(([, v]) => v !== undefined && v !== null && v !== '')
          .map(([k, v]) => `${k}=${encodeURIComponent(String(v))}`)
          .join('&')
      : ''
    return await $fetch<T>(`${base}${path}${qs}`, { retry: 0 })
  }

  async function post<T = any>(path: string, params?: Record<string, any>): Promise<T> {
    const qs = params
      ? '?' +
        Object.entries(params)
          .map(([k, v]) => `${k}=${encodeURIComponent(String(v))}`)
          .join('&')
      : ''
    return await $fetch<T>(`${base}${path}${qs}`, { method: 'POST', retry: 0 })
  }

  return { base, get, post }
}

// Format a *Cents value (main-unit × 100) with a currency symbol + decimals.
export function fmtMoney(cents: number | null | undefined, symbol = '', decimals = 2): string {
  if (cents == null) return '—'
  const v = cents / 100
  const s = decimals === 0 ? Math.round(v).toLocaleString() : v.toLocaleString(undefined, {
    minimumFractionDigits: decimals,
    maximumFractionDigits: decimals,
  })
  return symbol ? `${symbol}${s}` : s
}

import { useCallback, useEffect, useState } from 'react'
import type { UnlistenFn } from '@tauri-apps/api/event'
import {
  installApp,
  listApps,
  onAppState,
  startApp,
  stopApp,
  type AppStateEvent,
  type AppSummary,
} from '../api/apps'

export function useApps() {
  const [apps, setApps] = useState<AppSummary[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const refresh = useCallback(async () => {
    setError(null)
    try {
      setApps(await listApps())
    } catch (err) {
      setError(String(err))
    } finally {
      setLoading(false)
    }
  }, [])

  const handleAppState = useCallback((event: AppStateEvent) => {
    setApps((current) =>
      current.map((app) =>
        app.id === event.appId
          ? {
              ...app,
              state: event.state,
              version:
                event.version ??
                (event.state === 'not_installed' ? null : app.version),
              managed: event.managed,
            }
          : app,
      ),
    )
  }, [])

  useEffect(() => {
    let disposed = false
    let unlisten: UnlistenFn | undefined

    void (async () => {
      try {
        const next = await listApps()
        if (!disposed) {
          setApps(next)
          setError(null)
        }
      } catch (err) {
        if (!disposed) setError(String(err))
      } finally {
        if (!disposed) setLoading(false)
      }
    })()

    onAppState(handleAppState)
      .then((fn) => {
        if (disposed) fn()
        else unlisten = fn
      })
      .catch((err) => {
        if (!disposed) setError(String(err))
      })

    return () => {
      disposed = true
      unlisten?.()
    }
  }, [handleAppState])

  const run = useCallback(async (action: () => Promise<void>) => {
    setError(null)
    try {
      await action()
    } catch (err) {
      setError(String(err))
    }
  }, [])

  return {
    apps,
    loading,
    error,
    refresh,
    dismissError: () => setError(null),
    install: async (
      appId: string,
      methodId: string | null,
      allowAdvanced: boolean,
      allowElevation: boolean,
    ) => {
      setError(null)
      try {
        await installApp(appId, methodId, allowAdvanced, allowElevation)
      } catch (err) {
        setError(String(err))
        throw err
      }
    },
    start: (appId: string) => run(() => startApp(appId)),
    stop: (appId: string) => run(() => stopApp(appId)),
  }
}
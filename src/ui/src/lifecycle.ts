import type { AppSummary, LifecycleState } from './api/apps'

const STATE_LABEL: Record<LifecycleState, string> = {
  not_installed: 'Not installed',
  installing: 'Installing',
  stopped: 'Stopped',
  starting: 'Starting',
  running: 'Running',
  stopping: 'Stopping',
  failed: 'Failed',
  uninstalling: 'Uninstalling',
}

export function lifecycleLabel(
  app: Pick<AppSummary, 'state' | 'managed'>,
): string {
  if (app.state === 'running' && !app.managed) {
    return 'Running (external)'
  }
  return STATE_LABEL[app.state]
}
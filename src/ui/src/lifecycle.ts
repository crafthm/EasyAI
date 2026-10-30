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

export type PrimaryAction = {
  label: string
  disabled: boolean
}

export function primaryAction(
  app: Pick<AppSummary, 'state' | 'managed' | 'draft'>,
): PrimaryAction {
  if (app.draft) {
    return { label: 'No install method', disabled: true }
  }
  switch (app.state) {
    case 'not_installed':
      return { label: 'Install', disabled: false }
    case 'stopped':
      return { label: 'Start', disabled: false }
    case 'running':
      return app.managed
        ? { label: 'Stop', disabled: false }
        : { label: 'Running externally', disabled: true }
    case 'installing':
      return { label: 'Installing…', disabled: true }
    case 'starting':
      return { label: 'Starting…', disabled: true }
    case 'stopping':
      return { label: 'Stopping…', disabled: true }
    case 'uninstalling':
      return { label: 'Uninstalling…', disabled: true }
    case 'failed':
      return { label: 'Failed', disabled: true }
  }
}
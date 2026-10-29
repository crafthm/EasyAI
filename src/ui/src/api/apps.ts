import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

export type LifecycleState =
  | 'not_installed'
  | 'installing'
  | 'stopped'
  | 'starting'
  | 'running'
  | 'stopping'
  | 'failed'
  | 'uninstalling'

export type AppSummary = {
  id: string
  name: string
  description: string
  glyph: string
  color: string
  iconUrl: string | null
  state: LifecycleState
  version: string | null
  managed: boolean
  sourceUrl: string | null
  methodId: string | null
  methodKind: string | null
}

export type UsageApi = {
  base_url: string | null
  openai_base_url: string | null
  docs_url: string | null
}

export type UsageCli = {
  label: string
  command: string
}

export type Usage = {
  ui_url: string | null
  api: UsageApi | null
  docs_url: string | null
  cli: UsageCli[]
  notes: string | null
}

export type AppDetail = Omit<AppSummary, 'iconUrl'> & {
  installDir: string | null
  method: string | null
  healthOk: boolean
  provenanceJson: string | null
  bomPath: string | null
  usage: Usage | null
}

export type ExistingInstall = {
  methodId: string | null
  kind: string
  location: string
  version: string | null
  managed: boolean
}

export type InstallPlanRejection = {
  methodId: string
  reason: string
}

export type InstallPlan = {
  appId: string
  methodId: string
  methodKind: string
  tier: string
  score: number
  rationale: string[]
  rejected: InstallPlanRejection[]
  estimatedSize: number | null
  needsElevation: boolean
  advanced: boolean
  existing: ExistingInstall[]
  action: string
}

export type RecipeInstall = {
  kind: string
  repo: string
  asset: Record<string, string>
}

export type RecipeService = {
  command: string[]
  health_url: string
  env: Record<string, string>
}

export type RecipeUninstall = {
  keep_paths: string[]
}

export type Recipe = {
  id: string
  name: string
  description: string
  glyph: string
  color: string
  icon: string
  platforms: string[]
  install: RecipeInstall | null
  service: RecipeService | null
  uninstall: RecipeUninstall | null
}

export type MethodSettings = {
  order: string[]
  enabled: Record<string, boolean>
}

export type RecipePreview = {
  recipe: Recipe
  sourceUrl: string
  sha256: string
  platform: string
  supported: boolean
  conflict: boolean
  warnings: string[]
}

export type JobRecord = {
  id: number
  appId: string
  kind: string
  origin: string
  state: string
  error: string | null
  logPath: string
  createdAt: string
  finishedAt: string | null
}

export type AppStateEvent = {
  appId: string
  state: LifecycleState
  version: string | null
  managed: boolean
}

export type JobLogEvent = {
  jobId: number
  appId: string
  line: string
}

export type JobFinishedEvent = {
  jobId: number
  appId: string
  state: string
  error: string | null
}

export function listApps(): Promise<AppSummary[]> {
  return invoke('list_apps')
}

export function getAppDetail(appId: string): Promise<AppDetail> {
  return invoke('get_app_detail', { appId })
}

export function planInstall(
  appId: string,
  methodId: string | null,
  allowAdvanced: boolean,
  allowElevation: boolean,
): Promise<InstallPlan> {
  return invoke('plan_install', {
    appId,
    methodId,
    allowAdvanced,
    allowElevation,
  })
}

export function installApp(
  appId: string,
  methodId: string | null,
  allowAdvanced: boolean,
  allowElevation: boolean,
): Promise<void> {
  return invoke('install_app', {
    appId,
    methodId,
    allowAdvanced,
    allowElevation,
    origin: 'ui',
    confirmed: true,
  })
}

export function startApp(appId: string): Promise<void> {
  return invoke('start_app', { appId, origin: 'ui', confirmed: true })
}

export function stopApp(appId: string): Promise<void> {
  return invoke('stop_app', { appId, origin: 'ui', confirmed: true })
}

export function uninstallApp(appId: string): Promise<void> {
  return invoke('uninstall_app', { appId, origin: 'ui', confirmed: true })
}

export function listJobs(appId: string): Promise<JobRecord[]> {
  return invoke('list_jobs', { appId })
}

export function getJobLog(jobId: number): Promise<string> {
  return invoke('get_job_log', { jobId })
}

export function previewRecipe(url: string): Promise<RecipePreview> {
  return invoke('preview_recipe', { url })
}

export function importRecipe(url: string, sha256: string): Promise<void> {
  return invoke('import_recipe', {
    url,
    sha256,
    origin: 'ui',
    confirmed: true,
  })
}

export function removeRecipe(appId: string): Promise<void> {
  return invoke('remove_recipe', {
    appId,
    origin: 'ui',
    confirmed: true,
  })
}

export function openUrl(appId: string, url: string): Promise<void> {
  return invoke('open_url', {
    appId,
    url,
    origin: 'ui',
    confirmed: true,
  })
}

export function getMethodSettings(): Promise<MethodSettings> {
  return invoke('get_method_settings')
}

export function saveMethodSettings(settings: MethodSettings): Promise<void> {
  return invoke('save_method_settings', {
    settings,
    origin: 'ui',
    confirmed: true,
  })
}

export function onAppState(
  handler: (event: AppStateEvent) => void,
): Promise<UnlistenFn> {
  return listen<AppStateEvent>('app-state-changed', (event) =>
    handler(event.payload),
  )
}

export function onJobLog(
  handler: (event: JobLogEvent) => void,
): Promise<UnlistenFn> {
  return listen<JobLogEvent>('job-log', (event) => handler(event.payload))
}

export function onJobFinished(
  handler: (event: JobFinishedEvent) => void,
): Promise<UnlistenFn> {
  return listen<JobFinishedEvent>('job-finished', (event) =>
    handler(event.payload),
  )
}
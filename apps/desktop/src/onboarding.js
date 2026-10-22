export const ONBOARDING_STORAGE_KEY = 'rhn-link:onboarding:v1'
export const ONBOARDING_ROLES = ['clinician', 'implementer', 'developer']

export function readOnboardingPreference(storage) {
  try {
    const value = JSON.parse(storage.getItem(ONBOARDING_STORAGE_KEY) ?? 'null')
    if (value?.schemaVersion !== 1 || !ONBOARDING_ROLES.includes(value.role)) return null
    return { schemaVersion: 1, role: value.role }
  } catch {
    return null
  }
}

export function saveOnboardingPreference(storage, role) {
  if (!ONBOARDING_ROLES.includes(role)) return false
  try {
    storage.setItem(ONBOARDING_STORAGE_KEY, JSON.stringify({ schemaVersion: 1, role }))
    return true
  } catch {
    return false
  }
}

export function isOnboardingProjectReady(config) {
  return Boolean(
    config
    && typeof config.projectId === 'string'
    && config.projectId.trim()
    && typeof config.projectName === 'string'
    && config.projectName.trim()
    && typeof config.website === 'string'
    && config.website.trim(),
  )
}

export function validateOnboardingDraft(draft) {
  const projectName = draft?.projectName?.trim() ?? ''
  const projectId = draft?.projectId?.trim() ?? ''
  const environmentName = draft?.environmentName?.trim() ?? ''
  const businessUrl = draft?.businessUrl?.trim() ?? ''
  const tenantId = draft?.tenantId?.trim() ?? ''

  if (!projectName || projectName.length > 128 || /[\u0000-\u001f\u007f]/.test(projectName)) {
    return '请输入 1–128 个可显示字符的项目名称。'
  }
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(projectId)) {
    return '项目标识须为 1–64 位字母、数字、点、短横线或下划线，并以字母或数字开头。'
  }
  if (!environmentName || environmentName.length > 128 || /[\u0000-\u001f\u007f]/.test(environmentName)) {
    return '请输入 1–128 个可显示字符的环境名称。'
  }
  if (tenantId.length > 256 || /[\u0000-\u001f\u007f]/.test(tenantId)) {
    return '默认租户不能超过 256 个字符或包含控制字符。'
  }
  try {
    const url = new URL(businessUrl)
    if (url.protocol !== 'http:' && url.protocol !== 'https:') throw new Error('unsupported protocol')
    if (url.username || url.password) return '业务地址不能包含用户名或密码。'
  } catch {
    return '请输入完整的 HTTP 或 HTTPS 业务地址，例如 https://rhn.example.internal/。'
  }
  return ''
}

export function applyOnboardingDraft(config, draft) {
  const projectName = draft.projectName.trim()
  const projectId = draft.projectId.trim()
  const environmentName = draft.environmentName.trim()
  const businessUrl = draft.businessUrl.trim()
  const tenantId = draft.tenantId.trim()
  const environments = Array.isArray(config.environments) ? config.environments : []
  const matching = environments.find((environment) => (
    environment?.name?.trim() === environmentName || environment?.url?.trim() === businessUrl
  ))
  const selected = { ...(matching ?? {}), name: environmentName, url: businessUrl }
  const remaining = environments.filter((environment) => environment !== matching)

  return {
    ...config,
    projectName,
    projectId,
    tenantId,
    website: businessUrl,
    environments: [selected, ...remaining],
    allowSwitch: remaining.length > 0 ? Boolean(config.allowSwitch) : false,
  }
}

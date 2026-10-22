export type OnboardingRole = 'clinician' | 'implementer' | 'developer'

export type OnboardingPreference = {
  schemaVersion: 1
  role: OnboardingRole
}

export type OnboardingDraft = {
  projectName: string
  projectId: string
  environmentName: string
  businessUrl: string
  tenantId: string
}

export const ONBOARDING_STORAGE_KEY: string
export const ONBOARDING_ROLES: OnboardingRole[]
export function readOnboardingPreference(storage: Pick<Storage, 'getItem'>): OnboardingPreference | null
export function saveOnboardingPreference(storage: Pick<Storage, 'setItem'>, role: OnboardingRole): boolean
export function isOnboardingProjectReady(config: unknown): boolean
export function validateOnboardingDraft(draft: OnboardingDraft): string
export function applyOnboardingDraft<T extends Record<string, unknown>>(config: T, draft: OnboardingDraft): T

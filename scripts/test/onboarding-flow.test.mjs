import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

import {
  ONBOARDING_STORAGE_KEY,
  applyOnboardingDraft,
  isOnboardingProjectReady,
  readOnboardingPreference,
  saveOnboardingPreference,
  validateOnboardingDraft,
} from '../../apps/desktop/src/onboarding.js'

const validDraft = {
  projectName: '市一院 RHN',
  projectId: 'city-hospital-rhn',
  environmentName: '院内环境',
  businessUrl: 'http://10.17.5.57/',
  tenantId: 'hospital-01',
}

test('onboarding role preference is optional, versioned, and fails safely', () => {
  let value = null
  const storage = {
    getItem: (key) => key === ONBOARDING_STORAGE_KEY ? value : null,
    setItem: (key, next) => {
      assert.equal(key, ONBOARDING_STORAGE_KEY)
      value = next
    },
  }

  assert.equal(readOnboardingPreference(storage), null)
  assert.equal(saveOnboardingPreference(storage, 'clinician'), true)
  assert.deepEqual(readOnboardingPreference(storage), { schemaVersion: 1, role: 'clinician' })

  value = JSON.stringify({ schemaVersion: 2, role: 'developer' })
  assert.equal(readOnboardingPreference(storage), null)
  value = '{invalid'
  assert.equal(readOnboardingPreference(storage), null)
  assert.equal(saveOnboardingPreference({ setItem: () => { throw new Error('blocked') } }, 'developer'), false)
  assert.equal(saveOnboardingPreference(storage, 'administrator'), false)
})

test('project readiness requires identity and a business address', () => {
  assert.equal(isOnboardingProjectReady(validDraft), false)
  assert.equal(isOnboardingProjectReady({
    projectName: validDraft.projectName,
    projectId: validDraft.projectId,
    website: validDraft.businessUrl,
  }), true)
  assert.equal(isOnboardingProjectReady({ projectName: 'RHN', projectId: '', website: 'https://rhn.test/' }), false)
})

test('quick setup accepts authorized intranet HTTP addresses but rejects unsafe URL shapes', () => {
  assert.equal(validateOnboardingDraft(validDraft), '')
  assert.equal(validateOnboardingDraft({ ...validDraft, businessUrl: 'https://rhn.example.internal/app' }), '')
  assert.match(validateOnboardingDraft({ ...validDraft, projectId: 'bad id' }), /项目标识/)
  assert.match(validateOnboardingDraft({ ...validDraft, businessUrl: 'file:///tmp/rhn' }), /HTTP/)
  assert.match(validateOnboardingDraft({ ...validDraft, businessUrl: 'http://user:secret@10.0.0.1/' }), /用户名或密码/)
  assert.match(validateOnboardingDraft({ ...validDraft, projectName: '' }), /项目名称/)
})

test('quick setup preserves advanced settings and unrelated environments', () => {
  const matching = { name: '旧院内环境', url: 'http://10.17.5.57/', customHeader: 'preserved' }
  const config = {
    projectName: '',
    projectId: '',
    website: '',
    tenantId: '',
    environments: [matching, { name: '灾备环境', url: 'https://backup.example/' }],
    allowSwitch: true,
    trustedOrigins: ['https://trusted.example'],
    pluginCatalogUrl: 'https://plugins.example/catalog.json',
  }

  const result = applyOnboardingDraft(config, validDraft)
  assert.equal(result.projectName, validDraft.projectName)
  assert.equal(result.projectId, validDraft.projectId)
  assert.equal(result.website, validDraft.businessUrl)
  assert.equal(result.environments[0].name, validDraft.environmentName)
  assert.equal(result.environments[0].customHeader, 'preserved')
  assert.equal(result.environments[1].name, '灾备环境')
  assert.equal(result.allowSwitch, true)
  assert.deepEqual(result.trustedOrigins, config.trustedOrigins)
  assert.equal(result.pluginCatalogUrl, config.pluginCatalogUrl)
})

test('desktop composes onboarding from existing secured operations', async () => {
  const [app, wizard] = await Promise.all([
    readFile(new URL('../../apps/desktop/src/App.vue', import.meta.url), 'utf8'),
    readFile(new URL('../../apps/desktop/src/OnboardingWizard.vue', import.meta.url), 'utf8'),
  ])

  assert.match(app, /maybeOfferOnboarding\(config\.config\)/)
  assert.match(app, /applyOnboardingDraft\(snapshot\.value\.config, draft\)/)
  assert.match(app, /const saved = await saveConfig\(\)/)
  assert.match(app, /await inspectProjectBundle\(\)/)
  assert.match(app, /await openBusiness\(\)/)
  assert.match(app, /<strong>RHN LINK<\/strong><small>健域端联<\/small>/)
  assert.match(wizard, /id: 'clinician'/)
  assert.match(wizard, /id: 'implementer'/)
  assert.match(wizard, /id: 'developer'/)
  assert.match(wizard, /不改变账号权限，也不会跳过安全校验/)
  assert.match(wizard, /保存不会自动放开来源/)
  assert.doesNotMatch(wizard, /invoke\(/)
})

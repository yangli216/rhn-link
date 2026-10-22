<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import {
  readOnboardingPreference,
  saveOnboardingPreference,
  validateOnboardingDraft,
  type OnboardingDraft,
  type OnboardingRole,
} from './onboarding.js'

const props = defineProps<{
  open: boolean
  busy: boolean
  configured: boolean
  canOpenBusiness: boolean
  loadFailed: boolean
  initialProjectName: string
  initialProjectId: string
  initialBusinessUrl: string
  initialEnvironmentName: string
  initialTenantId: string
  actionError: string
}>()

const emit = defineEmits<{
  close: []
  save: [draft: OnboardingDraft, role: OnboardingRole]
  importProject: [role: OnboardingRole]
  openBusiness: [role: OnboardingRole]
  navigate: [section: 'overview' | 'native' | 'delivery', role: OnboardingRole]
}>()

const selectedRole = ref<OnboardingRole | null>(null)
const showRoleSelection = ref(true)
const attemptedSubmit = ref(false)
const draft = ref<OnboardingDraft>({
  projectName: '',
  projectId: '',
  environmentName: '',
  businessUrl: '',
  tenantId: '',
})

const roles: Array<{
  id: OnboardingRole
  marker: string
  title: string
  audience: string
  description: string
  outcome: string
}> = [
  {
    id: 'clinician',
    marker: '医',
    title: '医护人员',
    audience: '日常业务使用',
    description: '确认工作站状态并直接进入 RHN 业务，不展示插件和开发配置。',
    outcome: '目标：少判断、快进入',
  },
  {
    id: 'implementer',
    marker: '实',
    title: '实施人员',
    audience: '部署与现场交付',
    description: '导入已签项目包，或录入最小项目配置后继续做交付检查。',
    outcome: '目标：可检查、可交付',
  },
  {
    id: 'developer',
    marker: '开',
    title: '开发人员',
    audience: '接入与联调',
    description: '连接 RHN 开发地址，再进入本地能力映射与调试工作台。',
    outcome: '目标：快接入、易调试',
  },
]

const role = computed(() => roles.find((item) => item.id === selectedRole.value) ?? null)
const validationError = computed(() => validateOnboardingDraft(draft.value))
const stepLabel = computed(() => showRoleSelection.value ? '第 1 步，共 2 步' : '第 2 步，共 2 步')

function readStoredRole(): OnboardingRole | null {
  if (typeof window === 'undefined') return null
  return readOnboardingPreference(window.localStorage)?.role ?? null
}

function resetDraft() {
  draft.value = {
    projectName: props.initialProjectName,
    projectId: props.initialProjectId,
    environmentName: props.initialEnvironmentName || '院内环境',
    businessUrl: props.initialBusinessUrl,
    tenantId: props.initialTenantId,
  }
  attemptedSubmit.value = false
}

function chooseRole(nextRole: OnboardingRole) {
  selectedRole.value = nextRole
  showRoleSelection.value = false
  if (typeof window !== 'undefined') saveOnboardingPreference(window.localStorage, nextRole)
}

function backToRoles() {
  showRoleSelection.value = true
  attemptedSubmit.value = false
}

function submitSetup() {
  attemptedSubmit.value = true
  if (!selectedRole.value || validationError.value) return
  emit('save', { ...draft.value }, selectedRole.value)
}

function importProject() {
  if (!selectedRole.value) return
  emit('importProject', selectedRole.value)
}

function openBusiness() {
  if (!selectedRole.value) return
  emit('openBusiness', selectedRole.value)
}

function navigate(section: 'overview' | 'native' | 'delivery') {
  if (!selectedRole.value) return
  emit('navigate', section, selectedRole.value)
}

watch(
  () => props.open,
  (open) => {
    if (!open) return
    resetDraft()
    selectedRole.value = readStoredRole()
    showRoleSelection.value = selectedRole.value == null
  },
  { immediate: true },
)
</script>

<template>
  <Teleport to="body">
    <div v-if="open" class="onboarding-backdrop">
      <section
        class="onboarding-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="onboarding-title"
        aria-describedby="onboarding-description"
      >
        <header class="onboarding-header">
          <div>
            <p class="eyebrow">RHN LINK QUICK START</p>
            <h1 id="onboarding-title">首次使用向导</h1>
            <p id="onboarding-description">根据你的工作角色，只完成当前真正需要的设置。</p>
          </div>
          <button class="onboarding-dismiss" type="button" :disabled="busy" @click="emit('close')">稍后设置</button>
        </header>

        <div class="onboarding-progress" aria-live="polite">
          <span>{{ stepLabel }}</span>
          <i :class="{ complete: !showRoleSelection }" />
        </div>

        <div v-if="showRoleSelection" class="onboarding-role-step">
          <div class="onboarding-step-copy">
            <p class="eyebrow">CHOOSE YOUR ROLE</p>
            <h2>你主要如何使用这台工作站？</h2>
            <p>这只影响引导内容和默认入口，不改变账号权限，也不会跳过安全校验。</p>
          </div>
          <div class="onboarding-role-grid">
            <button
              v-for="item in roles"
              :key="item.id"
              type="button"
              :class="['onboarding-role-card', { selected: selectedRole === item.id }]"
              @click="chooseRole(item.id)"
            >
              <b>{{ item.marker }}</b>
              <span><small>{{ item.audience }}</small><strong>{{ item.title }}</strong></span>
              <p>{{ item.description }}</p>
              <em>{{ item.outcome }}</em>
            </button>
          </div>
        </div>

        <div v-else-if="selectedRole && role" class="onboarding-setup-step">
          <div class="onboarding-step-copy onboarding-role-summary">
            <span class="onboarding-role-marker">{{ role.marker }}</span>
            <div>
              <p class="eyebrow">{{ role.audience }}</p>
              <h2>{{ role.title }}快捷入口</h2>
              <p>{{ role.description }}</p>
            </div>
            <button type="button" :disabled="busy" @click="backToRoles">切换角色</button>
          </div>

          <div v-if="selectedRole === 'clinician'" class="onboarding-clinician-panel">
            <div :class="['onboarding-readiness', { ready: configured && !loadFailed }]">
              <span>{{ configured && !loadFailed ? '✓' : '!' }}</span>
              <div>
                <p class="eyebrow">WORKSTATION STATUS</p>
                <h3>{{ configured && !loadFailed ? '工作站已经可以使用' : loadFailed ? '暂时无法确认工作站状态' : '工作站尚未完成项目设置' }}</h3>
                <p v-if="configured && !loadFailed">项目与业务地址已经就绪，可以直接打开 RHN 业务系统。</p>
                <p v-else-if="loadFailed">请先让实施人员检查客户端运行状态和诊断日志，再继续业务操作。</p>
                <p v-else>请联系项目实施人员导入已签项目包。完成后重新打开本向导即可进入业务。</p>
              </div>
            </div>
            <div class="onboarding-clinician-actions">
              <button
                v-if="configured && !loadFailed"
                class="primary"
                type="button"
                :disabled="busy || !canOpenBusiness"
                @click="openBusiness"
              >
                {{ busy ? '正在启动…' : '进入 RHN 业务' }}
              </button>
              <button v-else type="button" :disabled="busy" @click="navigate('overview')">查看运行状态</button>
              <button v-if="!configured" type="button" :disabled="busy" @click="chooseRole('implementer')">转到实施设置</button>
            </div>
            <p class="onboarding-help">日常使用无需管理 DLL、COM 或插件；出现设备问题时，将“安全与诊断”中的脱敏诊断包交给实施人员。</p>
          </div>

          <template v-else>
            <section v-if="selectedRole === 'implementer'" class="onboarding-import-callout">
              <div>
                <p class="eyebrow">RECOMMENDED</p>
                <h3>导入已签项目包</h3>
                <p>一次带入项目配置、来源策略和本地能力清单，并保留正式的预检与确认步骤。</p>
              </div>
              <button class="primary" type="button" :disabled="busy || loadFailed" @click="importProject">选择项目包</button>
            </section>

            <div class="onboarding-divider"><span>{{ selectedRole === 'implementer' ? '或先完成最小配置' : '连接开发环境' }}</span></div>

            <form class="onboarding-form" @submit.prevent="submitSetup">
              <label>
                <span>项目名称 <b>必填</b></span>
                <input v-model="draft.projectName" type="text" maxlength="128" autocomplete="off" placeholder="例如：市一院 RHN" :disabled="busy">
              </label>
              <label>
                <span>项目标识 <b>必填</b></span>
                <input v-model="draft.projectId" type="text" maxlength="64" autocomplete="off" placeholder="例如：city-hospital-rhn" :disabled="busy">
              </label>
              <label>
                <span>环境名称 <b>必填</b></span>
                <input v-model="draft.environmentName" type="text" maxlength="128" autocomplete="off" placeholder="例如：院内环境" :disabled="busy">
              </label>
              <label class="onboarding-url-field">
                <span>RHN 业务地址 <b>必填</b></span>
                <input v-model="draft.businessUrl" type="url" autocomplete="off" :placeholder="selectedRole === 'developer' ? 'http://127.0.0.1:5173/' : 'http://10.0.0.10/'" :disabled="busy">
                <small>HTTP 内网地址可用，但仍须由签名来源策略明确授权；保存不会自动放开来源。</small>
              </label>
              <label>
                <span>默认租户 <i>可选</i></span>
                <input v-model="draft.tenantId" type="text" maxlength="256" autocomplete="off" placeholder="由 RHN 项目约定" :disabled="busy">
              </label>

              <p v-if="attemptedSubmit && validationError" class="onboarding-form-error" role="alert">{{ validationError }}</p>
              <p v-if="actionError" class="onboarding-form-error" role="alert">保存失败：{{ actionError }}</p>

              <footer class="onboarding-footer">
                <button type="button" :disabled="busy" @click="backToRoles">上一步</button>
                <button class="primary" type="submit" :disabled="busy || loadFailed">
                  {{ busy ? '正在保存…' : selectedRole === 'developer' ? '保存并配置本地能力' : '保存并进入交付检查' }}
                </button>
              </footer>
            </form>
          </template>
        </div>
      </section>
    </div>
  </Teleport>
</template>

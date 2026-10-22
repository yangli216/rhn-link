# RHN Link · 健域端联

[![Build and test](https://github.com/yangli216/rhn-link/actions/workflows/ci.yml/badge.svg)](https://github.com/yangli216/rhn-link/actions/workflows/ci.yml)

RHN Link 是 **健域智枢（Regional Health Nexus，RHN）** 的工作站本地能力扩展。它让 RHN Web 应用在受控边界内调用读卡器、打印机、扫码设备、CA、厂商原生组件和必要的操作系统能力，同时把插件、项目配置、签名、更新、诊断与部署验证纳入同一条交付链。

当前真实项目需求主要来自 Windows，因此现阶段优先完善 Windows 上的 DLL、COM/OCX、双架构宿主和安装交付。整体桥接协议、权限模型和插件治理不以 Windows 为产品边界；Linux（尤其国产信创桌面操作系统）和 macOS 会在出现明确项目需求后，按各自原生组件、设备驱动和发布体系补齐适配与验收。

RHN Link 不是独立 HIS，也不是通用浏览器或任意本机 HTTP 服务。普通业务用户仍在 RHN 中完成工作；RHN Link 只为当前业务来源提供经过签名策略授权的最小本地能力。

## 典型场景

- 门诊、收费和药房页面调用身份证、社保卡或医保设备。
- 处方、执行单、标签等受控打印，以及打印结果对账。
- 临床文档签署、CA 证书和密码设备接入。
- 扫码枪、签名板及厂商专用设备的 Windows DLL/COM 或其他平台原生组件封装。
- 院内 HTTP、隔离网或半离线环境中的项目安装、升级和恢复。
- 将既有 WebPlus/桌面本地 HTTP 调用迁移到类型化 Web Bridge。

## 架构

当前 Windows 主线实现如下：

```text
RHN business WebView
        │  typed Web Bridge
        ▼
Tauri command allowlist
        │
        ▼
Rust controller
        │  framed private IPC
        ├──────────────┐
        ▼              ▼
x64 plugin host   x86 plugin host
        │              │
        └──── DLL / COM / OCX / controlled EXE
```

Tauri 主进程不加载第三方 DLL 或 OCX。x86 与 x64 原生组件分别运行在对应架构的隔离宿主中；宿主故障不会直接拖垮桌面主进程。新架构内部通信不依赖 localhost HTTP。

Web Bridge、controller 契约、来源授权、插件签名和交付治理保持平台无关；原生宿主和插件入口按目标操作系统实现。Linux/macOS 不会模拟 Windows DLL/COM，而是接入各自可验证的动态库、系统 API、驱动或标准设备协议。

## 核心能力

### 本地能力运行

- Windows 主线同时支持 x86/x64 DLL、COM/OCX 和受控进程适配。
- 统一的超时、请求 ID、输入上限、并发限制和稳定错误契约。
- 打印、写卡等非幂等操作支持持久 operation ID、状态查询和人工对账，禁止不确定结果自动重放。
- 业务页面加载后自动完成 WebView → Rust 就绪握手，启动失败可进入诊断入口。

### 插件与开发接入

- 可视化发现、映射、调试和回归 DLL/COM 能力，无需为简单组件重新编译桌面程序。
- 从同一 API 清单生成类型化 TypeScript Client、tracked API、fixture 和 Web 接入包。
- 第三方复杂 ABI、线程模型或厂商内存管理通过专用 Rust 适配器隔离。
- 插件包包含 SemVer、Desktop 兼容范围、完整文件摘要和组织签名。

### 项目交付

- 将 RHN 业务来源、项目配置、签名插件和本地映射组成可预检的原子项目包。
- 支持安装、升级、回退、卸载和应用数据保留验证。
- 提供 Windows 交付工具包，用于离线诊断、材料预检、迁移审计和正式硬件矩阵。
- 生产切换通过迁移、插件矩阵和 Windows 包三类独立证据执行 Go/No-Go。

### 安全边界

- 远程页面不获得 Shell、任意文件系统或通用 IPC 权限。
- 每个 origin 只能调用签名来源策略精确授权的 service/method，不接受通配授权。
- 插件、来源策略、项目包、更新包和发布证据分别进入对应签名域。
- 原生组件只在独立宿主中加载；路径越界、内容漂移、位数不符和签名异常均失败关闭。
- 本地映射和回归参数进入持久能力基线，退出期间被替换后不会静默启用。

完整约束见 [产品定位与迭代准则](docs/product-direction.md) 和 [目标架构 ADR](docs/adr/0001-target-architecture.md)。

## 当前阶段

项目当前处于“Windows 工程链路基本完成、等待真实 RHN 项目试点”的阶段。合成测试已经覆盖双架构宿主、插件签名、类型化 SDK、安装升级与回退，但生产交付仍必须使用真实原生组件、代表性硬件、脱敏业务 HAR、组织密钥和上一正式安装包完成验证。

当前支持策略：

- **Windows x64**：默认桌面版本，同时携带 x86/x64 插件宿主。
- **Windows x86**：为确有 32 位系统或组件需求的项目提供完整安装包。
- **Windows 10/11**：主线验证平台；可选择包含 WebView2 的离线包或在线轻量包。
- **Windows 7**：不作为主线承诺；只有明确项目需求时建立冻结依赖和独立验证分支。
- **Linux**：保留 DEB/AppImage 预览构建；国产信创桌面系统是后续重点候选，出现项目需求后按 CPU 架构、WebView、`.so`/系统 API、外设驱动、签名和安装格式建立独立验证基线。
- **macOS**：保留 DMG 预览构建；出现明确业务或设备需求后补充 arm64/x64 原生适配、签名、公证与实机门禁。

跨平台支持指业务桥接契约和治理能力保持一致，不表示 Windows DLL/COM 插件可以直接在 Linux 或 macOS 运行。每个平台都需要使用其原生组件和真实硬件独立验收。

详见 [平台支持边界](docs/platform-support.md) 和 [当前完成度评估](docs/completion-assessment.md)。

## RHN Web 接入

业务前端通过 [`packages/web-bridge`](packages/web-bridge/README.md) 接入，不直接调用 Tauri 私有 API：

1. 为设备或厂商组件建立本地映射或签名插件。
2. 从插件 API 清单生成类型化 Web Client 和 fixture。
3. 在 RHN 前端测试中使用 fixture 验证业务流程。
4. 在发布策略中精确授权 RHN origin、service 和 method。
5. 在目标操作系统实机上执行插件黄金矩阵和业务页面就绪验证。

旧 WebPlus/本机 HTTP 调用的迁移方法见 [Web Bridge 迁移指南](docs/web-bridge-migration.md)。打印、写卡等流程应同时遵循 [持久调用与防重放](docs/tracked-invocations.md)。

## 开发环境

基础要求：

- Rust `1.91`（以仓库工具链文件为准）
- Node.js `22`
- Windows 原生开发需安装 MSVC x86/x64 工具链和 WebView2
- Linux/macOS 原生适配需准备目标发行版或系统版本、对应工具链及真实设备环境

安装前端依赖：

```bash
npm ci --prefix apps/desktop
npm ci --prefix packages/web-bridge
```

启动控制台前端：

```bash
npm run dev --prefix apps/desktop
```

启动 Tauri 开发程序：

```bash
npm run tauri --prefix apps/desktop -- dev
```

Linux/macOS 当前可以开发控制台、共享协议和平台无关工具，并生成预览包；进入生产支持前仍需补齐该平台的原生适配、安装签名、真实设备和升级回退验证。任何平台的开发机结果都不能替代目标环境实机验收。

## 本地验证

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked

npm run build --prefix apps/desktop
npm test --prefix packages/web-bridge
node --test scripts/test/*.test.mjs
```

Windows 上执行完整原生门禁：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/test-windows.ps1
```

GitHub Actions 默认验证共享 Rust/前端能力和当前优先级最高的 Windows x86/x64 原生链路，并生成 x64/x86 离线及在线轻量 NSIS 测试包。Linux/macOS 预览包在手动选择 `all` 时构建；后续形成实际交付需求后，再把对应原生与安装验收提升为默认门禁。

> CI 产物使用临时测试密钥且没有 Authenticode，名称包含 `unsigned`，仅用于试用和验证，不能作为生产安装包分发。

## 仓库结构

```text
apps/desktop/              Tauri 2 + Vue 3 桌面程序
packages/web-bridge/       RHN Web 类型化桥接 SDK
crates/webplus-*           原生协议、控制器、宿主与兼容层
crates/ssdev-*             配置、签名、发布和交付工具
examples/                  Windows 系统能力插件示例
scripts/                   Windows 构建、验收和供应链门禁
docs/                      架构、迁移、安全与交付文档
```

部分 crate、CLI 和插件扩展名仍保留历史 `ssdev` / `webplus` 技术标识，用于维持协议和迁移工具兼容；面向用户和 RHN 项目的产品名称统一为 **RHN Link（健域端联）**。

## 文档导航

| 角色/主题 | 文档 |
| --- | --- |
| 产品边界与迭代优先级 | [产品定位与迭代准则](docs/product-direction.md) |
| RHN 前端接入 | [Web Bridge 迁移指南](docs/web-bridge-migration.md) |
| DLL/COM 可视化配置与调试 | [本地映射工作台](docs/local-mapping-studio.md) |
| 插件开发、封包与黄金矩阵 | [插件发布流程](docs/plugin-release.md) |
| 插件签名与运行时信任 | [插件签名与生命周期](docs/plugin-signing.md) |
| 业务来源授权 | [来源策略](docs/origin-policy.md) |
| 项目配置与能力联合交付 | [项目部署包](docs/project-bundles.md) |
| Windows 安装与更新 | [应用更新](docs/app-updates.md) |
| 启动失败与现场诊断 | [诊断与日志](docs/diagnostics.md) |
| 实施/QA 离线工具 | [Windows 项目交付工具包](docs/windows-delivery-tools.md) |
| 真实项目试点输入 | [试点材料预检](docs/pilot-readiness.md) |
| 生产切换判定 | [Go/No-Go 证据](docs/cutover-evidence.md) |

Windows API 插件的完整参考实现见 [`examples/windows-system-plugin`](examples/windows-system-plugin/README.md)。

## 许可证

本项目使用专有许可证，详见各 Cargo/package manifest 中的许可证声明。未经授权不得用于生产分发。

# RHN 部署说明

本文记录 RHN（健域智枢）业务前端接入 RHN Link 时与通用部署不同的约定：登录方式、Web Bridge SDK 的获取方式和受控打印。通用的来源策略、项目部署包和 Windows 交付仍按各自文档执行。

## 登录：不使用 RHN Link SSO

RHN 在页面内完成登录，并使用 `RHN_LOGIN` Cookie 维持会话；它没有 `logon/ssoLogin` 端点，也不需要启动器把用户、角色、科室传给客户端。RHN 部署**不启用** RHN Link 的 SSO，无需改协议。

已核实的行为（`apps/desktop/src-tauri/src/sso.rs`、`lib.rs` 启动流程）：

- 只有命令行中出现 `--uid <值>` 才会发起 SSO。没有 `--uid` 时 `parse_arguments` 返回“未请求”，不会构造请求、不会联网，启动流程直接打开已配置的默认业务地址；重复启动（单实例转发）同样只聚焦已有业务窗口。
- 只传 `--rid` 或 `--deptId` 而没有 `--uid` 会被视为参数错误（`sso-arguments-invalid`）并显示控制台。因此 RHN 的快捷方式、开机启动项和院内启动器**不得**携带这三个参数。
- 业务窗口直接加载配置中的 `website`（如 `https://rhn.example.internal/`）。登录页与业务页同源时无需任何额外配置；登录过程若跳转到其他来源（例如独立认证服务），该来源要写入配置的 `trustedOrigins` 并由签名来源策略的 `navigationOrigins` 批准，它只获得导航能力，不获得桥接。
- Cookie 保存在客户端的 WebView 持久配置中，客户端重启后仍然有效；控制台“清理站点数据”会删除 Cookie 和登录状态并关闭业务窗口，相当于强制重新登录。

RHN 侧需要注意：

- 院内只能使用 HTTP 时，来源策略需显式 `allowInsecureHttp: true`，且 `RHN_LOGIN` 不能带 `Secure` 属性，否则 WebView 不会回传 Cookie。推荐使用 HTTPS 并设置 `Secure; HttpOnly; SameSite=Lax`。
- 业务窗口拒绝 `window.open` 和 `target=_blank` 新窗口；需要新窗口时使用 SDK 的 `openWindow`，外部帮助页使用 `openExternal`。
- 隐藏 iframe 调用 `print()` 在 WebView2 中仍会弹出系统打印预览，但无法指定打印机、校验内容或得到回执；受控打印改用 `rhn.printing`。

## Web Bridge SDK 的获取

SDK 目前仍是 `private: true` 的 `@rhn/link-web-bridge`，没有发布到任何 npm 仓库。主分支 CI 每次都会生成 `rhn-link-web-bridge-sdk` 制品（保留 14 天），内容固定为 `.tgz` 和 `rhn-link-web-bridge-sdk.json` 清单；构建过程已经在临时项目中完成离线安装、ESM 调用和类型检查（见 [`packages/web-bridge/README.md`](../packages/web-bridge/README.md)）。

### 当前建议：vendor 目录 + 锁定摘要

1. 从目标 rhn-link 提交对应的 Actions run 下载整个 `rhn-link-web-bridge-sdk` 目录；或在该提交的干净检出中运行：

   ```bash
   node scripts/web-bridge-package.mjs build --output /path/to/new/rhn-link-web-bridge-sdk
   node scripts/web-bridge-package.mjs verify --directory /path/to/new/rhn-link-web-bridge-sdk
   ```

2. 把目录原样放入 RHN 前端仓库，例如 `frontend/vendor/rhn-link-web-bridge/`，不要解包或改名。另写一行来源记录（rhn-link 提交、Actions run ID、清单 `sha256`、`sourceSha256`），与 tgz 一起评审。
3. 在 `package.json` 中引用归档：

   ```json
   { "dependencies": { "@rhn/link-web-bridge": "file:vendor/rhn-link-web-bridge/rhn-link-web-bridge-0.2.0.tgz" } }
   ```

   `npm install` 会把归档的 `integrity`（SHA-512）写入 `package-lock.json`，之后 `npm ci` 遇到被替换的 tgz 会失败，这就是依赖锁定。
4. RHN CI 再做一次独立核对：清单中的 `archive`、`bytes`、`sha256` 与目录内 tgz 一致，且 `sha256` 等于来源记录中的值。`web-bridge-package.mjs verify` 还会比较 rhn-link 当前源码摘要，因此只能在对应 rhn-link 提交中运行，RHN 仓库内用简单的 SHA-256 比对即可。
5. 升级 SDK 时整目录替换、重新 `npm install`、同一个变更中更新来源记录。CI 制品 14 天后过期，应在下载当天完成 vendor。

这一方式不需要任何新基础设施，但 tgz 本身没有组织签名；可信度来自代码评审、来源记录和锁文件摘要。

### 之后：内部 npm 仓库

院内或公司有私有 npm 仓库（Verdaccio、Nexus、GitLab Package Registry 等）后，由批准的版本治理变更移除 `private`、配置 `publishConfig.registry` 和发布签名，RHN 改为按精确版本安装。需要先确定仓库地址、发布权限和版本号规则（0.2.0 仍是开发期版本，同一版本号的不同构建内容可能不同）。

### 包名

自 0.2.0 起 SDK 包名为 `@rhn/link-web-bridge`（此前为 `@bsoft/ssdev-web-bridge`），CI 制品与清单为 `rhn-link-web-bridge-sdk` / `rhn-link-web-bridge-sdk.json`，归档名为 `rhn-link-web-bridge-<版本>.tgz`。导出 API、桥接协议版本和注入对象名 `window.ssdevDesktop` 都没有变化；注入对象名属于桥接协议，不随包名修改。

这是破坏性变更：插件生成器（`ssdev-plugin-tool client/web-kit`、本地映射工作台“TS”导出）生成的 `client.ts`、`fixtures.ts` 现在从 `@rhn/link-web-bridge` 导入，旧版生成文件和 Web 接入包摘要都必须重新生成并评审，不能与新 SDK 混用。RHN 当前直接检测 `window.ssdevDesktop`，接入 SDK 时直接使用新包名即可。

## 受控打印

契约、状态码和 Windows 验证项见 [RHN 受控 PDF 打印](rhn-printing.md)。部署时需要：

1. 安装与工作站架构匹配的 `rhn-printing-x64`（或 x86）签名插件，建议放进项目部署包；
2. 在签名来源策略中为 RHN 来源授权 `rhn.printing` 的 `listPrinters`、`printPdf`（见 [来源策略](origin-policy.md)），兼容模式下只需插件已签名并被当前来源覆盖；
3. 确认部署自检中持久调用账本可用，否则 `printPdf` 会失败关闭；
4. RHN 设备绑定中的 `deviceName` 应等于 `listPrinters` 返回的系统打印机名称。

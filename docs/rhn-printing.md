# RHN 受控 PDF 打印（`rhn.printing`）

RHN 的处方、执行单和标签由后端生成不可变 PDF，并带有 `jobId`、`outputId`、SHA-256 摘要和打印设备绑定。页面内隐藏 iframe 调用 `print()` 无法指定打印机、不能校验内容，也没有任何回执。`rhn.printing` 只补齐这一段：在业务页面已经拿到 PDF 与摘要后，由工作站校验同一份字节并提交到指定系统打印队列。

它不是通用打印服务：不接受 URL、本机路径、HTML、图片或打印机驱动参数，不做打印预览，不读取打印队列历史，也不承诺纸张已经输出。

## 放置位置

| 方案 | 结论 | 原因 |
| --- | --- | --- |
| 受控 EXE 适配（`mainType: exe`） | 不采用 | 进程适配器把每个参数作为命令行参数传递；Windows `CreateProcess` 命令行上限为 32,767 个 UTF-16 字符，100 KB PDF 的 Base64 已超出。即使改为临时文件，也会让 PDF 内容落盘并出现在其他进程可读的位置，结果只能通过 stdout 文本返回。 |
| 安装包内置插件 | 暂不采用 | 当前安装包没有“预装签名插件”的机制。新增该机制需要定义随客户端升级、卸载、回退和信任根的语义，属于平台核心扩张；打印行为与设备驱动强相关，也应能独立于 Desktop 版本发布和撤回。 |
| 独立 Rust DLL 适配器，按普通签名插件交付 | **采用** | 仓库内一方源码 [`crates/rhn-printing-plugin`](../crates/rhn-printing-plugin)，经通用 DLL ABI 在 x86/x64 隔离插件宿主中运行：打印机驱动在 GDI 渲染时会被加载进调用进程，放在宿主中才不会拖垮 Tauri 主进程。签名、`trackedInvocationRequired`、Desktop 兼容范围、黄金矩阵、仓库更新/撤回和项目部署包全部复用现有链路，不新增运行时协议。 |

Tauri 主进程和 controller 不直接实现打印。插件清单位于 [`plugins/rhn-printing`](../plugins/rhn-printing)，x86 与 x64 使用不同插件 ID、同一个 `rhn.printing` 服务，一台工作站只安装其中一个。

## 输入上限核对

| 边界 | 上限 | 来源 |
| --- | --- | --- |
| 单次调用参数 JSON | 8 MiB | `webplus_protocol::MAX_INVOKE_PARAMETERS_BYTES`，controller 在进入宿主前校验 |
| controller ↔ 宿主帧 | 16 MiB | `webplus_ipc::MAX_FRAME_BYTES` |
| 业务页 → Tauri 命令 | 无额外上限 | 注入桥直接把参数交给 `plugin_invoke_tracked` |
| 持久调用账本 | 只记录请求哈希 | 参数不落盘，大小不影响账本记录上限 |
| 插件自身 | 解码后 6 MiB、Base64 最多 8,388,608 字符 | `rhn-printing-plugin` 常量 |

典型 100 KB–2 MB 的 PDF 编码后约 134 KB–2.7 MB，可以直接作为 `documentBase64` 传递。解码后接近 6 MiB 时参数 JSON 会先触达 8 MiB 的 controller 上限，返回普通参数错误且保证未执行。

超出时的替代方案（当前不实现，按需求再评审）：

1. **首选**：RHN 后端按份或按页拆成多个不可变 PDF，每份独立 `outputId`、摘要和 operation ID。不改协议，每份都可以单独对账。
2. 分块暂存：增加 `beginDocument/appendChunk/printStaged` 一类方法，由插件在宿主内存或受控临时目录拼接，最终仍按完整摘要校验。需要新的状态清理和持久调用语义，是一个新契约。
3. 由工作站凭 URL 下载 PDF：不采用。会让本地组件获得 RHN 会话或令牌、新增网络信任边界，并使持久调用的请求哈希不再覆盖实际打印内容。

## 契约

服务 `rhn.printing`，两个方法都在 `ResData` 中返回通用 DLL ABI 的 `ReturnValue`（整数状态码）和 `result`（JSON 文本）。`ReturnValue = 0` 时 `JSON.parse(ResData.result)` 正好是下面的业务对象；非零时 `result` 为 `{"error":"<稳定码>"}`。

> **与最初约定的差异**：最初约定 `ResData` 直接等于 `{ printers: [...] }` / `{ submitted: true, systemJobId? }`。通用 DLL ABI 的输出只有整数和字符串，因此业务对象改为放在 `ResData.result` 的 JSON 文本中，并增加 `ReturnValue`。若希望 `ResData` 直接是结构化对象，需要先为通用 DLL ABI 增加 JSON 输出类型，这是平台级变更，需要单独决定。

### `listPrinters({})`

```json
{ "ResCode": 0, "ResData": { "ReturnValue": 0, "result": "{\"printers\":[{\"name\":\"处方打印机\",\"isDefault\":true}]}" } }
```

- 枚举本机和当前用户连接的打印机（`EnumPrintersW` `PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS`），保持系统枚举顺序并去重。
- `isDefault` 来自当前用户默认打印机；没有默认打印机时全部为 `false`。
- 不返回端口、驱动、服务器、共享名或队列状态。普通 `invokePlugin` 可调用。

### `printPdf({ printerName, documentBase64, sha256, copies, jobName })`

方法在签名 `api.json` 中声明 `trackedInvocationRequired: true`：只能通过 `invokePluginTracked` 调用，普通桥调用在进入宿主前以 `ResCode=-32004` 拒绝。

| 字段 | 约束 |
| --- | --- |
| `printerName` | 1–256 字符、无控制字符；必须与 `listPrinters` 返回的某个名称相同（Windows 打印机名不区分大小写） |
| `documentBase64` | 标准 Base64（含填充），解码后 1 字节–6 MiB，且以 `%PDF-` 开头 |
| `sha256` | 解码后 PDF 字节的 SHA-256，64 位**小写**十六进制 |
| `copies` | 1–99，在同一个打印作业内逐份排序输出 |
| `jobName` | 1–128 字符、无控制字符；作为系统打印队列中的文档名，建议使用 `outputId` 等不含患者信息的标识 |

处理顺序：校验字段 → 解码 → **校验摘要** → 校验 PDF 头 → 确认打印机存在 → 解析 PDF → `StartDoc` → 逐页渲染并提交 → `EndDoc`。摘要、文档和打印机不符都在创建打印作业前失败。

成功：

```json
{ "ResCode": 0, "ResData": { "ReturnValue": 0, "result": "{\"submitted\":true,\"systemJobId\":42}" } }
```

`submitted: true` 只表示整份文档已经交给 Windows 打印后台处理程序（`EndDoc` 成功），**不代表已经出纸**；缺纸、卡纸、脱机或驱动丢弃都可能发生在之后。`systemJobId` 是 `StartDoc` 返回的后台作业号，仅用于现场在打印队列中定位，系统重启或作业完成后会被复用，不能作为业务主键；该字段可能缺省。

### 稳定状态码

| `ReturnValue` | `error` | 是否已进入打印队列 | 业务处理 |
| --- | --- | --- | --- |
| 0 | — | 已提交 | 记录已提交；出纸仍需人工或后续回执确认 |
| 1001 | `invalid-request` | 否 | 修正参数后用**新** operation ID 重新发起 |
| 1002 | `document-digest-mismatch` | 否 | 视为内容被篡改或传输错误，重新从后端获取 PDF |
| 1003 | `document-invalid` | 否 | Base64、PDF 头、大小、页数（份数 × 页数最多 200 页）或 PDF 解析失败 |
| 1004 | `printer-not-found` | 否 | 打印设备绑定与本机不符，刷新打印机列表或修正绑定 |
| 1005 | `printer-unavailable` | 否 | 无法打开打印机或创建作业 |
| 1006 | `job-aborted` | **不确定** | 作业已创建但未完整提交，已请求取消；先到打印队列核对是否有残留或部分输出，再决定是否重打 |
| 1007 | `output-buffer-too-small` | 否（`listPrinters` 专用） | 打印机过多，联系实施人员 |
| 1008 | `internal-error` | **不确定** | 插件内部异常；按不确定结果对账 |
| 1050 | `unsupported-platform` | 否 | 当前只有 Windows 实现 |

`ResCode` 非零（宿主超时、崩溃、协议错误）以及 tracked 状态 `indeterminate` / `completedWithoutResult` 都按“可能已提交”处理，禁止自动重放，先核对打印队列和纸面结果。业务侧统一使用 SDK 的 `settleTrackedInvocation()`。

## Windows 实现

- PDF 解析与渲染使用系统自带的 `Windows.Data.Pdf`（Windows 10 及以上），不引入第三方 PDF 引擎；渲染在插件内新建的 MTA 线程中完成。
- 每页按打印机 `LOGPIXELSX/Y` 渲染，最高 300 DPI、单页最多 4,800 万像素，再用 GDI `StretchDIBits`（HALFTONE）输出到 `CreateDCW("WINSPOOL", printerName)` 设备上下文。
- 按 PDF 页面实际尺寸居中放置在物理纸张上；页面大于纸张时等比缩小。纸张、方向、纸盒和双面使用该打印机的系统默认设置，插件不修改 `DEVMODE`。不同标签/处方纸应在 RHN 设备绑定中对应不同的系统打印机（可以是同一物理设备的多个队列）。
- 以位图方式输出：清晰度满足处方、执行单和条码标签，但不保留矢量文字；超高精度或超大幅面文档不在本插件范围。
- 所有入口捕获 panic，返回 `1008`，不让异常跨越 FFI 终止宿主。

## 验证边界

macOS/Linux 开发机只能验证参数校验、摘要、版式计算、输出格式和 `api.json` 与通用 DLL ABI 的一致性，并对 Windows 目标执行 `cargo check`。以下内容必须在 Windows 实机验证：

- x86 与 x64 插件分别通过通用 DLL 映射和隔离宿主加载、Health 预检、`listPrinters`；
- 真实打印机（含网络连接打印机和标签机）上实际出纸、尺寸、居中、缩放和多份顺序；
- 摘要不符、不存在的打印机、损坏 PDF、超页数不会产生打印作业；
- 渲染中途断开打印机或驱动报错时返回 `1006` 并在队列中取消作业；
- 宿主超时（默认方法超时 120 秒）后 tracked 状态为 `indeterminate`；
- 黄金矩阵：除 `listPrinters` 和失败用例外，真实打印用例必须在矩阵中显式标为有副作用并由现场人员确认纸面结果。

## 来源授权示例

严格模式下，RHN 来源需在签名来源策略中精确授权两个方法：

```json
{
  "origin": "https://rhn.example.internal",
  "services": [
    { "serviceId": "rhn.printing", "methods": ["listPrinters", "printPdf"] }
  ]
}
```

完整示例见 [来源策略](origin-policy.md)。

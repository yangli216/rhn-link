# RHN 受控 PDF 打印插件

`rhn.printing` 服务的签名插件清单。契约、放置位置、输入上限和状态码见 [RHN 受控 PDF 打印](../../docs/rhn-printing.md)；源码在 [`crates/rhn-printing-plugin`](../../crates/rhn-printing-plugin)。

| 架构 | 插件 ID（建议） | 清单 |
| --- | --- | --- |
| x64 | `rhn-printing-x64` | [`api.x64.json`](api.x64.json) |
| x86 | `rhn-printing-x86` | [`api.x86.json`](api.x86.json) |

两者声明同一个 `rhn.printing` 服务，一台工作站只安装其中一个。`printPdf` 声明了 `trackedInvocationRequired: true`，业务页只能通过持久调用使用它。

## 构建与调试（Windows）

```powershell
cargo build --locked --release -p rhn-printing-plugin --target x86_64-pc-windows-msvc
```

输出 `target/x86_64-pc-windows-msvc/release/rhn_printing_plugin.dll`。可在控制台“DLL 动态映射与调试”中按对应 `api.json` 加载；`listPrinters` 无副作用，`printPdf` 会真实提交打印作业，只能在确认的测试打印机上调试。

仓库 Windows 门禁 `scripts/test-windows.ps1` 会对 x86/x64 分别构建，并通过真实 `webplus-native` DLL 映射执行 `listPrinters`、摘要不符和打印机不存在三条不会产生打印作业的用例。

## 签名发布

目录结构与 [Windows 系统能力插件示例](../../examples/windows-system-plugin/README.md) 相同：在新的源目录中放入对应架构的 `api.json` 和 `bin/rhn_printing_plugin.dll`，然后按 [插件发布流程](../../docs/plugin-release.md) 执行 `ssdev-plugin-tool source-check`、`prepare`（填写已验证的 `--desktop-version-requirement`）、组织 KMS/HSM 签名和 `finalize`。生成的黄金矩阵草稿中，真实打印用例必须由现场人员确认纸面结果后才能解除草稿状态。

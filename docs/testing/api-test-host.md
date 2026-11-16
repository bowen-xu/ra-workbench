# 公开 API 测试宿主

插件工程可固定 Workbench 版本，通过开发程序实际注册的 Tauri 命令验证协作行为。
宿主使用原有命令处理器、授权检查、托管服务、数据库和退出流程。入口仅编译进
启用 `development-data` 的 debug 程序，显式传入 `--workbench-api-test-host` 后启用。

## 构建与复用

在 Workbench 工程运行 `pnpm build:api-test-host`。该入口调用既有 `bundle:dev`
流程生成零插件开发应用，再封装成可复用的版本化测试产物。目前提供 macOS 产物。
相同源码、公开 SDK 和工具链输入会复用经过完整性校验的既有包。

`artifacts/api-test-host/latest.json` 指向不可变包目录。包内的 `api-test-host.json`
记录 Core 版本、协议版本、平台、架构、开发标识、归档文件及 SHA-256。
消费方固定产物键，校验清单中的版本、平台、架构及归档摘要，然后将归档解压到
自己的 `.tmp/` 目录。应用内已包含 Core 所需的运行资源。

公开客户端随 `pnpm build:preview-host` 产生的
`@research-assistant/workbench-preview` 包发布，子路径为 `/api-test-host`。
消费方安装该包的指定版本，使用已解压应用中的可执行文件。

```js
import {launchWorkbenchApiTestHost} from "@research-assistant/workbench-preview/api-test-host";

const host = await launchWorkbenchApiTestHost({
  executable: "/absolute/project/.tmp/Research Assistant Dev.app/Contents/MacOS/research-assistant",
  dataDirectory: "/absolute/project/.tmp/synthetic-api-data",
  expectedVersion: "0.8.0",
});
try {
  const result = await host.invoke("list_runtime_plugin_packages");
  if (result.packages.length !== 0) throw new Error("expected_zero_plugins");
  const page = await host.invoke("create_note_page", {draft: {title: "Synthetic note"}});
  await host.invoke("load_note_document", {pageId: page.page.id});
} finally {
  await host.close();
}
```

## 数据、插件与生命周期

调用方提供绝对路径的合成数据目录。默认使用独立空插件目录。插件自己的协作验收
通过 `pluginDirectory` 指定已构建的运行时插件目录，其版本与产物由插件或父仓选择。
宿主不会构建、发现或导入相邻工程的插件源码。

客户端通过既有开发凭据路径覆盖，将凭据位置固定在该合成目录的
`credentials.json`。测试使用的模型配置和合成凭据由调用方显式准备。

显式 API 测试模式中的 WebView 存储按规范化数据目录隔离。同一目录的冷重开复用
相同配置，以便验证界面恢复。macOS 使用命名 WebKit 数据存储，要求 macOS 14
及以上。应用数据与 WebView 存储均只应使用合成测试内容。

客户端保持异步请求编号关联，原生错误内容通过拒绝值保留。`close()` 等待已发请求
完成，关闭输入并等待 Core 原有后台退出流程。该出口验证后台状态收束，界面的
未保存编辑需要通过正式界面的保存与关闭流程单独验收。

## 接口与测试归属

`invoke` 的命令、参数和返回值沿用 Core 已公开的 Tauri 命令契约。JSON-RPC 只负责
测试传输与协议版本握手。生产 SDK 和插件能力声明仍是插件正式运行时的边界。
测试入口内保留 Tauri 调用身份，客户端通过标准输入输出通信。

Core 自身测试可验证内部实现。跨工程协作测试使用公开 SDK、Native 协议和这里
已有的命令入口。必要能力尚无公开契约时，先明确能力与权限，再补接口。

当前已有真实零插件程序的笔记创建、标题与正文保存、修订冲突、并发响应关联、
缺席对象错误和冷重开验收。真实界面已验证设置、共享聊天、从上下文资源链接打开
Core 笔记、正文编辑保存、设置与文档标签切换，以及冷重开后的标签和正文恢复。
测试使用原生产界面和公开资源链接入口。

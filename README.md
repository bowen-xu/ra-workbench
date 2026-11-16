# ra-workbench

核心工作台，版本从 0.8.0 开始。导航、标签、共享编辑／PDF、Agent loop、HBS、凭据、数据库、文件和受控进程由本工程拥有。

## 依赖与构建

```sh
node /path/to/sdk-artifacts/install-sdk.mjs /path/to/sdk-artifacts .
pnpm install --frozen-lockfile
pnpm test-dependencies:install -- /path/to/literature-test-artifacts /path/to/writing-test-artifacts
pnpm build
pnpm build:native
pnpm typecheck
pnpm test -- apps/web/src/core/runtime/plugins/viewRegistry.test.ts
```

SDK 公开依赖位于本工程 .dependencies/sdk/。现有 Native 回归使用文献和写作库的版本化测试支持产物，安装到 .dependencies/test-plugins/。Cargo 解析应用 manifest 时同时解析 dev-dependencies，测试支持产物也需要先准备。主仓库的统一安装和构建入口自动准备这些依赖。

包和锁文件由本工程维护。Web SDK 的 UI／Hook 声明绑定到本次 Core 构建的 /sdk/v1/。插件通过这些 facade 消费同一 React、文档、HBS 和 Agent 实例。Agent 执行机制位于 packages/agent-core，公开输入与纯编解码使用 SDK 的 agent-api。

build:native 编译 research-assistant 和 note-pdf-worker 两个运行程序，通过 artifacts/workbench-native/latest.json 定位公开产物，Cargo 缓存归本工程 .cache/cargo。安装包生成沿主仓库既有发布流程执行。

## 开发与回归

pnpm dev 提供 Core 前端。pnpm dev:desktop 和 pnpm bundle:dev 消费 RESEARCH_ASSISTANT_RUNTIME_PLUGIN_DIRECTORY 指定的预构建插件目录。完整产品的默认三插件组装由主仓库入口负责。

本工程拥有 Core 源码、单元测试、Agent／存储／解析运行包及其测试。`tests/ui/` 的通用 Core 浏览器验收通过 `pnpm test:ui` 执行。


插件 UI 测试可将 apps/web/dist 作为 RA_WORKBENCH_SDK_DIRECTORY，使用本次构建的公开同源 SDK。该测试入口消费 Core 构建产物。

## 公开预览宿主

build:preview-host 发布 @research-assistant/workbench-preview，复用正式工作台与 Core 组件。
各插件拥有自己的预览，调用宿主的 PreviewWorkbench 并提供 loadPlugins。
开发包包含编译后 JS、声明、原 CSS、笔记查询 Wasm、词典和静态资源，版本跟随 Workbench。
显式模块清单在 apps/web/previewHostSurface.json，产物在 artifacts/preview-host/，定向回归使用 pnpm test:preview-host。

## 构建缓存与产物

标准构建入口复用通过完整性校验的既有包。产物在本工程 `artifacts/<kind>/<内容哈希>/package/`，当前入口在 `artifacts/<kind>/latest.json`。回执随产物保存，`.cache/build` 保存索引，`.cache/logs` 保存日志，`.cache/work` 为完成后清理的隔离工作目录。清理编译缓存后仍可复用已保存的当前包。

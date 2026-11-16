# ra-workbench

核心工作台，版本从 0.8.0 开始。导航、标签、共享编辑／PDF、Agent loop、HBS、凭据、数据库、文件和受控进程由本工程拥有。

## 依赖与构建

以下命令在本工程根目录执行。先取得指定版本的 SDK 成品目录，其中包含
`sdk-artifacts.json` 和 `install-sdk.mjs`。

```sh
pnpm sdk:install /absolute/path/to/sdk-package
pnpm install --frozen-lockfile
pnpm build
pnpm build:native
```

SDK 公开依赖位于本工程 `.dependencies/sdk/`。Core 自身的安装、前后端构建和测试使用本工程源码、SDK 与工具依赖。固定 SQL、JSON 及有限策略响应是带有精确来源和摘要的自有测试输入。领域规则及单插件协作测试由各插件工程拥有。

显式真实包的 Core 接纳／事务集成用例由父仓选择并提供预构建包，通过原有限 Native 协议验证。其包目录和执行结果单独记录。

包和锁文件由本工程维护。Web SDK 的 UI／Hook 声明绑定到本次 Core 构建的 /sdk/v1/。插件通过这些 facade 消费同一 React、文档、HBS 和 Agent 实例。Agent 执行机制位于 packages/agent-core，公开输入与纯编解码使用 SDK 的 agent-api。

`pnpm build` 构建零插件前端，并将已验证的 Web 产物装配到 `apps/web/dist/`。
`pnpm build:native` 准备 Core 自有运行资源，编译 `research-assistant` 和
`note-pdf-worker` 两个程序。安装包生成沿主仓库既有发布流程执行。

## 开发与回归

```sh
pnpm dev
pnpm dev:desktop
```

`pnpm dev` 提供 Core 前端开发服务。`pnpm dev:desktop` 自动准备 Core 自有运行资源，
构建前端并启动零插件桌面工作台。显式设置 `RESEARCH_ASSISTANT_RUNTIME_PLUGIN_DIRECTORY`
时消费该目录的预构建插件包。完整产品的默认三插件组装由父仓库入口负责。

```sh
RESEARCH_ASSISTANT_RUNTIME_PLUGIN_DIRECTORY=/absolute/path/to/plugin-directory pnpm dev:desktop
pnpm bundle:dev
```

插件目录的直接子目录各保存一个完整插件包。`pnpm bundle:dev` 默认生成零插件
开发应用，路径为本工程 `.tmp/Research Assistant Dev.app`。

本工程拥有 Core 源码、单元测试、Agent／存储／解析运行包及其测试。`tests/ui/` 的通用 Core 浏览器验收通过 `pnpm test:ui` 执行。

```sh
pnpm typecheck
pnpm exec vitest run apps/web/src/core/runtime/plugins/viewRegistry.test.ts
pnpm test:native --lib runtime_plugins::tests
```

插件 UI 测试可将本工程 `apps/web/dist/` 的绝对路径作为
`RA_WORKBENCH_SDK_DIRECTORY`，使用本次构建的公开同源 SDK。

## 公开预览宿主

`pnpm build:preview-host` 生成 `@research-assistant/workbench-preview`，复用正式工作台与 Core 组件。
各插件拥有自己的预览，调用宿主的 `PreviewWorkbench` 并提供 `loadPlugins`。
开发包包含编译后 JS、声明、原 CSS、笔记查询 Wasm、词典和静态资源，版本跟随 Workbench。
显式模块清单在 `apps/web/previewHostSurface.json`，定向回归使用 `pnpm test:preview-host`。

```sh
pnpm build:preview-host
pnpm build:api-test-host
```

`pnpm build:api-test-host` 通过现有临时开发包流程提供零插件原生测试应用，发布到
`artifacts/api-test-host/`。指定版本的公开客户端从预览包 `/api-test-host` 子路径导入。
构建、消费和数据隔离约定见 [公开 API 测试宿主](docs/testing/api-test-host.md)。

## 构建缓存与产物

| 路径 | 内容 |
|---|---|
| `artifacts/workbench-web/latest.json` | 当前前端包，包内 `web/` 同时装配到 `apps/web/dist/` |
| `artifacts/workbench-native/latest.json` | 当前两个原生程序 |
| `artifacts/preview-host/latest.json` | 公开前端预览及测试宿主 |
| `artifacts/api-test-host/latest.json` | 可复用原生 API 测试应用 |
| `apps/desktop/*/target/`、`packages/*/target/` | 各 Core Rust 工程的编译缓存 |
| `.cache/build/` | 构建输入到已有包的索引 |
| `.cache/logs/` | 按次保存的构建日志 |
| `.cache/work/` | 隔离构建工作目录 |

标准构建入口校验并复用完整产物。`latest.json` 指向
`artifacts/<kind>/<内容哈希>/package/`，对应构建回执也保存在产物目录。
清理编译缓存后，同输入构建仍可复用已保存的包。

工程位于 Research Assistant 父仓库时，可在父仓库根目录运行统一清理入口。

```sh
pnpm clean:cache workbench --dry-run
pnpm clean:cache workbench
pnpm clean:cache workbench --kind logs
```

默认清理编译缓存并保留构建索引。`--kind logs` 选择构建日志，`--kind all`
还清理构建索引及残留隔离工作目录。索引清除后，构建工具回退核对当前
`latest.json` 对应的包，其他历史输入组合可能重新构建。清理范围与命令说明见
父仓库 README 的缓存章节。

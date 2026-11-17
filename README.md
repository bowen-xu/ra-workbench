# Research Assistant Workbench

TypeScript 与 Electron 迁移的工作台工程，当前已建立目录、包清单和严格 TypeScript 配置。

| 目录 | 职责 |
|---|---|
| `apps/desktop/src/main` | Electron 主进程、窗口和系统集成 |
| `apps/desktop/src/preload` | 类型化受控桥接 |
| `apps/web/src/app` | React 工作台装配与导航 |
| `apps/web/src/core` | 共享生产界面与前端运行资源 |
| `apps/chrome-connector/src` | Chrome 采集适配 |
| `services/core/src` | Node 后台应用服务与物理执行 |
| `services/docling/src` | TypeScript 本地解析服务 |
| `packages/agent-core/src` | 平台无关 Agent 生命周期与调度 |
| `packages/data-runtime/src` | 数据接纳、物化和恢复协调 |
| `packages/ui/src` | 可复用 UI |
| `tests/contracts` | Workbench 契约验收 |
| `tests/integration` | Workbench 组合与真实适配器验收 |

本工程是独立 Git submodule，拥有自己的 pnpm 工作区、包清单、锁文件及配置。
`tsconfig.node.json` 用于 Node／Electron 代码，`tsconfig.browser.json` 用于浏览器代码。
依赖与可执行入口随各能力迁移落地。跨工程消费采用版本化公开产物，领域插件由公开加载接口接入。

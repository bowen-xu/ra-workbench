# Workbench 与 SDK 的组合契约验收

本目录验证 Workbench 的公开实现与 SDK 类型、产物及相关插件消费者之间的兼容性。测试需要父工程检出 SDK 和相应插件，并按父工程入口准备 SDK 产物与 Web 依赖；它们不属于 Workbench 的独立构建或默认单元测试。通过父工程的 `integration` 契约测试入口显式运行。

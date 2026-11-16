# 工作流等待的受控响应

响应由文献 Native 的公开 API 对固定合成输入产生，来源和摘要记录于
`provenance.json`。Core 测试使用相同 DTO 验证数据库守卫释放、读集重查、候选
持久化和操作生命周期。模板与运行身份在夹具装配时绑定 Core 当前产生的原身份。
录制的任务、上下文、Docling、选区、页面与模板响应由 Native 的
`core_workflow_wait_contract` 公开 API 测试复验。

候选修订、延后审阅、人工 Claim 与派生视图的准备器使用测试显式给定的正向响应，
用于驱动 Core 持久化、并发和生命周期场景。相关领域校验由 Native 测试及真实包
场景负责。

上下文和结构块同时保留公开 wire 契约的 CBOR，Core 通过原 SDK 解码，保留表格
显式 `Some(null)` 和浮点值。JSON 视图用于查阅固定数据，二进制字段用于装配 DTO。

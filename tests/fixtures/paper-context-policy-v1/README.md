# 有限上下文策略响应契约

样本保存公开 CatalogInput 与 CatalogOutput，对应 Core 合成夹具的确定输入。
Core 测试按精确输入回放策略响应，验证权限、事务与进程生命周期。文献 Native
通过公开 catalog::dispatch 验证同值响应。来源实现、精确提交和摘要记录于
provenance.json。

增加输入场景时用该版本公开领域接口生成响应，同步核对两端样本并运行受影响回归。

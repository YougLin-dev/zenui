---
doc-id: EVID-<(VAL|PROBE)>-NN
layer: evidence
status: normative
date: YYYY-MM-DD
owner: 测试与 CI/发布负责人
sources:
  - <命令输出的保存位置、探针源文件路径>
replaces: []
---

# EVID-<VAL|PROBE>-NN <一句话：这次证明了什么>

- **类型**：`EVID-VAL` 真实执行记录 / `EVID-PROBE` 探针能力边界。
- **主机与环境**：OS 版本、CPU/GPU adapter、驱动、工具链版本、commit。缺任一项的记录不可跨机器引用。
- **命令**：
  ```sh
  <逐条可复跑命令，带完整 manifest-path>
  ```
- **原始结果**：退出码、通过/失败数、关键输出片段。粘贴原文，不改写。
- **能证明什么**：一句话，范围写死（例如"仅 parse-only，不含 lowering"）。
- **不证明什么**：必须写。跨平台、跨后端、跨输入法、跨 scale 的结论，一处没跑就不能声称。
- **升级路径**：要把它升到更高一级证据，还缺哪台机器/哪个夹具。

> 规则：一条记录只增不改。结论变了就新开一条，旧条目标 `supersededBy`；失败、编译不过、跑不起来同样是证据，必须记录。

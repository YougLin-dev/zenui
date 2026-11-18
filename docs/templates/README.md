---
doc-id: TEMPLATES-HOME
layer: governance
status: normative
date: 2026-11-18
owner: Crew 队长（架构）
sources:
  - docs/README.md §3–§7
replaces: []
---

# 模板：新增文档时复制这里，不要自己发明格式

四类模板对应四个可写层。每份新文档都从模板起手，删掉注释再填；填完必须：

1. 在 [../README.md](../README.md) §3.1 主题索引里登记一行（主题、文档、负责人、证据入口）；
2. 跑 `python3 scripts/check_docs.py` 通过；
3. 若结论推翻了旧条目，旧条目标 `WITHDRAWN` 或 `superseded` 并保留原文。

| 模板 | 用于 | 落到 |
|---|---|---|
| [adr.md](adr.md) | 一个不可逆的取舍：决定了什么、否掉了什么、撤销条件 | `decisions/` |
| [spec-entry.md](spec-entry.md) | 一条规范要求（MUST / MUST NOT / SHOULD） | `spec/` |
| [evidence-entry.md](evidence-entry.md) | 一次真实执行或一个探针的能力边界 | `evidence/` |
| [claim-row.md](claim-row.md) | 台账里的一行可判真假的断言 | `evidence/claim-register.md` |

## 不作为模板的两类

- **研究草稿**：本仓库不设 `research/` 层。未定的候选方案写成 `status: draft` 的 spec 条目或 `OPEN` 的 `CLAIM-*`，避免出现第二权威。
- **性能基线**：暂无 `bench/`。第一次产出可复跑数据时，按 `evidence-entry.md` 记录环境、命令、原始输出与对比基线，再决定是否单独建目录。

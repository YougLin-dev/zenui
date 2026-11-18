---
doc-id: TEMPLATE-CLAIM-ROW
layer: governance
status: normative
date: 2026-11-18
owner: 顶级研究员（疑难攻坚）
sources:
  - docs/README.md §6
replaces: []
---

# 台账行怎么写

`evidence/claim-register.md` 的每一行是一条**可判真假的单句断言**：

```markdown
| CLAIM-NNN | <单句断言，能被一条命令或一行源码证实/证伪> | <状态> | <证据> | <Gate> | <限制与备注> |
```

- **状态**：只用 `SPEC` / `PROBE` / `VERIFIED` / `WITHDRAWN` / `OPEN` 五词。
- **证据**：`EVID-VAL-*` > `EVID-PROBE-*` > `SRC path:line` > 外部官方文档 URL。`SRC` 只证明"代码此刻是这样"，不证明运行行为。`VERIFIED` / `PROBE` / `WITHDRAWN` 不许留空。
- **Gate**：本断言对应 `SPEC-GATE-*` 判据；无关则写 `—`。
- **限制与备注**：写清"只证明了哪一半"。范围没写死的断言，下游会当成全域结论引用。
- **推翻**：不删行、不改历史证据，把状态改为 `WITHDRAWN` 并在备注写明推翻依据。

新增后跑 `python3 scripts/check_docs.py`，它会把缺证据的行、悬空的 ID 标出来。

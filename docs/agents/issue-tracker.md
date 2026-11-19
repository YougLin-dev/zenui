# Issue tracker：GitHub

本仓库的 issue 与规格以 GitHub issue 的形式存放。所有操作使用 `gh` CLI。

## 约定

- **创建 issue**：`gh issue create --title "..." --body "..."`。多行正文使用 heredoc。
- **读取 issue**：`gh issue view <number> --json number,title,body,labels,comments`。
- **列出 issue**：`gh issue list --state open --json number,title,body,labels,comments --jq '[.[] | {number, title, body, labels: [.labels[].name], comments: [.comments[].body]}]'`，配合适当的 `--label` 与 `--state` 过滤。
- **把 issue 设为父 issue 的子 issue**：`gh issue create --parent <parent> ...`，或事后 `gh issue edit <parent> --add-sub-issue <child>`（需 `gh` 2.94+）。旧版 `gh`：`gh api --method POST repos/<owner>/<repo>/issues/<parent>/sub_issues -F sub_issue_id=<child-db-id>`（使用数据库 id，见下方 **阻塞**）。若不支持子 issue，在子 issue 正文顶部写 `Part of #<parent>`。
- **评论 issue**：`gh issue comment <number> --body "..."`。
- **添加 / 移除标签**：`gh issue edit <number> --add-label "..."` / `--remove-label "..."`。
- **关闭**：`gh issue close <number> --comment "..."`。

仓库从 `git remote -v` 推断；在克隆目录内运行 `gh` 时会自动推断。

## PR 作为 triage 入口

**PR 作为请求入口：否。**（若本仓库把外部 PR 当作功能请求，改为 `是`；`/triage` 会读取此标记。）

设为 `是` 后，PR 与 issue 走相同的标签和状态，使用对应的 `gh pr` 命令：

- **读取 PR**：`gh pr view <number> --comments`，diff 用 `gh pr diff <number>`。
- **列出待 triage 的外部 PR**：`gh api --paginate 'repos/{owner}/{repo}/pulls?state=open' --jq '.[] | select(.author_association | IN("OWNER","MEMBER","COLLABORATOR") | not) | {number, title, author: .user.login, author_association, labels: [.labels[].name]}'`。
- **评论 / 打标签 / 关闭**：`gh pr comment`、`gh pr edit --add-label`/`--remove-label`、`gh pr close`。

GitHub 的 issue 与 PR 共用编号空间，裸 `#42` 可能是两者之一：先用 `gh pr view 42` 解析，失败再回退 `gh issue view 42`。

## 当技能说“发布到 issue tracker”

创建一个 GitHub issue。

## 当技能说“获取相关工单”

按上文 **读取 issue** 执行。

## Wayfinding 操作

供 `/wayfinder` 使用。**地图** 是一个 issue，**子 issue** 是工单。

- **地图**：一个带 `wayfinder:map` 标签的 issue，承载 Notes / Decisions-so-far / Fog 正文。`gh issue create --label wayfinder:map`。
- **子工单**：作为地图的 GitHub 子 issue 关联（见 **把 issue 设为父 issue 的子 issue**）。未启用子 issue 的环境，把子 issue 加进地图正文的任务列表，并在子 issue 正文顶部写 `Part of #<map>`。标签：`wayfinder:<type>`（`research`/`prototype`/`grilling`/`task`）。被认领后，工单分配给执行的开发者。
- **阻塞**：使用 GitHub 原生 issue 依赖。用 `gh api --method POST repos/<owner>/<repo>/issues/<child>/dependencies/blocked_by -F issue_id=<blocker-db-id>` 添加边，`<blocker-db-id>` 是阻塞者的数字 **数据库 id**（`gh api repos/<owner>/<repo>/issues/<n> --jq .id`，不是 `#number` 或 `node_id`）。GitHub 以 `issue_dependencies_summary.blocked_by` 报告未关闭的阻塞者。不支持依赖的环境，回退为子 issue 正文顶部的 `Blocked by: #<n>, #<n>`。所有阻塞者关闭后工单才算解除阻塞。
- **前沿查询**：列出地图的未关闭子 issue，剔除有未关闭阻塞者（`issue_dependencies_summary.blocked_by > 0`，或 `Blocked by` 行内有未关闭 issue）或已有 assignee 的，按地图顺序取第一个。
- **认领**：`gh issue edit <n> --add-assignee @me`，会话的第一次写操作。
- **解决**：`gh issue comment <n> --body "<答案>"`，然后 `gh issue close <n>`，最后把上下文指针追加到地图的 Decisions-so-far。

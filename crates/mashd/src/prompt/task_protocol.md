## TaskGraph Protocol (JSONL + TaskNode)

**TaskGraph directory (store files under this folder):**
`{task_dir}`

当目标需要多步推进（尤其是并行子任务）时，使用 TaskGraph 管理执行。简单问题不必创建 TaskGraph。

TaskGraph 以 **JSONL** 保存，每行一个 TaskNode。不要在回复正文里粘贴完整文件内容；UI 会从该目录下最新文件读取并展示。

在每一轮需要 TaskGraph 时，先在该目录下生成一个新文件名（例如 `taskgraph_<timestamp>.jsonl`），并在本轮后续读写中始终使用同一文件。

---

### TaskNode Schema

每个 TaskNode 必须包含：

- `id`: string，唯一标识（如 `task_1`）
- `desc`: string，任务描述
- `depen`: string[]，依赖任务 ID 列表，可为空
- `type`: `"inline"` 或 `"subagent"`

其中：

- `inline`：由当前主会话直接执行（串行）。
- `subagent`：必须通过 **bash** 启动子代理执行，即运行  
  `mash agent "<任务描述>"`  
  如需附加规则：`mash agent "<任务描述>" --system "<附加指令>"`。

---

### 1. TaskGraphCreate

需要多步骤任务时，先确定本轮文件路径，再调用 `write` 创建 TaskGraph JSONL。

Example:
write({
  "path": "{task_dir}/taskgraph_1710000000.jsonl",
  "content": "{\"id\":\"task_1\",\"desc\":\"明确目标\",\"depen\":[],\"type\":\"inline\"}\n{\"id\":\"task_2\",\"desc\":\"并行调研方案A\",\"depen\":[\"task_1\"],\"type\":\"subagent\"}\n{\"id\":\"task_3\",\"desc\":\"并行调研方案B\",\"depen\":[\"task_1\"],\"type\":\"subagent\"}\n{\"id\":\"task_4\",\"desc\":\"汇总并决策\",\"depen\":[\"task_2\",\"task_3\"],\"type\":\"inline\"}\n"
})

---

### 2. TaskGraphUpdate

当拆分方案变化时，使用 `read` + `edit` 修改 TaskGraph（尽量一次 edit 完成多处变更）。

Example flow:
1) read({
  "path": "{task_dir}/taskgraph_1710000000.jsonl"
})
2) edit({
  "path": "{task_dir}/taskgraph_1710000000.jsonl",
  "edits": [
    {
      "oldText": "{\"id\":\"task_3\",\"desc\":\"并行调研方案B\",\"depen\":[\"task_1\"],\"type\":\"subagent\"}",
      "newText": "{\"id\":\"task_3\",\"desc\":\"并行调研方案B（缩小范围）\",\"depen\":[\"task_1\"],\"type\":\"subagent\"}"
    }
  ]
})

---

### 3. TaskGraphList

调用 `read` 查看当前 TaskGraph。

read({
  "path": "{task_dir}/taskgraph_1710000000.jsonl"
})

---

### 4. TaskGraphGet

调用 `read` 的 `offset/limit` 分段查看大图。

read({
  "path": "{task_dir}/taskgraph_1710000000.jsonl",
  "offset": 1,
  "limit": 80
})

---

### 执行约束

- 只在依赖满足时执行节点（DAG，无环）。
- 避免多个同时就绪的 `inline` 节点；主会话只能串行处理。
- 可并行工作优先建为 `subagent` 节点。
- 遇到 `subagent` 节点时，使用 bash 启动 `mash agent ...` 执行，并在拿到结果后继续推进后续节点。

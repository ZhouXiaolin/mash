# System Prompt 模块

System prompt 在编译期由多个 md 文件通过 `concat!(include_str!(...), ...)` 拼接，便于分块维护和扩展。

## 模块说明

| 文件 | 职责 |
|------|------|
| `identity.md` | **定位**：通用智能助手的 meta 执行协议（收集信息、规划分解、执行验证、可复现交付） |
| `tools_general.md` | **通用工具**：bash 的使用边界与常用操作范式（本地读写/搜索/脚本/网络请求） |
| `tools_specialized.md` | **特例化工具**：任务列表必须用四类 bash 任务；MCP 能力通过 bash + curl 调用，且无 tool-use 块 |
| `work_style.md` | **工作方式**：收集信息→规划→执行→验证→交付的通用流程 |
| `response_format.md` | **回复格式**：每轮开头的 Insight 块与可验证交付要求 |

## 组装顺序

在 `core/agent.rs` 中当前顺序为：

1. `identity.md`
2. `tools_general.md`
3. `tools_specialized.md`
4. `work_style.md`
5. `response_format.md`

运行时还会在整段 system prompt 后追加（在 `tui/mod.rs`）：

- MCP 工具列表（`format_mcp_tools_for_prompt`）
- 任务列表协议详情（`format_task_prompt` → `task_protocol.md`）

新增静态模块时，在 `SYSTEM_PROMPT` 的 `concat!()` 中追加对应 `include_str!("prompt/xxx.md")` 即可。

## 其他 prompt

- `task_protocol.md`：任务列表协议（TaskCreate/TaskUpdate/TaskList/TaskGet），由 `core/tasks.rs` 的 `format_task_prompt()` 引入并填入 `{path_str}`、`{perl_cmd}`。
- MCP 工具列表由 `core/mcp::format_mcp_tools_for_prompt()` 动态生成，无静态 md。

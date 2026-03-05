<h1 align="center">Mash</h1>

<p align="center">
  <strong>Bash is all you need.</strong>
</p>

<p align="center">
  一个极简的 Claude 风格 agent：只暴露一个 <code>bash</code> 工具，读文件、写文件、改文件、任务列表全部通过 bash 完成。
</p>

---

## 理念

- 不提供 `read` / `write` / `edit` 等一堆 API，模型只调用 `bash`，用 `cat`、`tee`、`sed`、`grep` 等完成一切。 
- 任务列表等「协议」写在 system prompt 里，用 4 个约定的 bash 调用（TaskCreate / TaskUpdate / TaskList / TaskGet）操作同一份 markdown 任务文件，UI 只负责监听文件变化并刷新。 
- 核心就是「发消息 → 收 tool_use → 执行 bash → 回填 tool_result」的循环，易于扩展和替换后端。 

---

## 配置

配置目录：**`~/.mash/settings.json`**

```json
{
  "model_provider": "deepseek",
  "model": "deepseek-chat",
  "model_providers": [
    {
      "name": "deepseek",
      "base_url": "https://api.deepseek.com/anthropic",
      "api_key": "sk-xxxxxxxxx"
    }
  ]
}
```

---

## 下一步规划

- **Agent team**：多 agent 协作（分工、接力、评审），仍保持「bash is all you need」的单一工具哲学，在编排层扩展。

---

## 对 MCP 的态度

核心不变：**bash is all you need**。Agent 在 API 层只有一个工具 `bash`，我们不把 MCP 作为独立 tool 暴露给模型。

TUI 启动时在本机起一个 HTTP 服务（默认 `127.0.0.1:31415`，可通过 `MCP_HTTP_PORT` 修改），唯一路由是 `POST /mcp/call`，请求体为 `{ "server", "tool", "arguments" }`，由我们内部转成 MCP 的 `tools/call` 并返回结果。同时把已连接的所有 MCP 工具格式化成一段prompt：工具名、简短描述、参数说明。模型不会收到任何 MCP 的 tool_use，只会被告诉：「需要这些能力时，用 bash 执行 curl 调用上述接口。」
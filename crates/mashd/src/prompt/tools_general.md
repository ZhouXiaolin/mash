## Tools

你可以使用四个 coding tools：

1. `read`
   - Schema:
     - `path: string`
     - `offset?: number`（1-indexed 起始行）
     - `limit?: number`（最多读取行数）
   - 描述：
     - 读取文件内容，支持文本和图片。
     - 文本默认截断到 200 行或 100KB；内容过大时，用 `offset/limit` 分段读取。
     - 图片返回 base64，并自动缩放到不超过 2000x2000。

2. `write`
   - Schema:
     - `path: string`
     - `content: string`
   - 描述：
     - 写入文件；不存在则创建，存在则覆盖；自动创建父目录。
     - 同一路径写操作会被串行化。

3. `edit`
   - Schema:
     - `path: string`
     - `edits: [{ oldText: string, newText: string }]`
   - 描述：
     - 对单文件做精确文本替换。
     - 所有 `oldText` 基于原始文件匹配（非增量）；每个 `oldText` 必须唯一且不重叠。
     - 多处修改应合并到同一次 `edit` 调用。
     - 返回 unified diff 和变更行号。

4. `bash`
   - Schema:
     - `command: string`
     - `timeout?: number`（秒）
   - 描述：
     - 在当前工作目录执行 bash 命令。
     - 输出截断到最后 200 行或 100KB；超出部分保存到临时文件。
     - 支持超时；非零退出码会返回错误。

### 使用策略

- 文件读取优先 `read`，文件写入优先 `write`，文本替换优先 `edit`。
- 只有在需要执行命令（构建、测试、搜索、脚本、外部程序）时才使用 `bash`。
- 完成代码改动后，优先用可复现命令验证（例如 check/test/build）。

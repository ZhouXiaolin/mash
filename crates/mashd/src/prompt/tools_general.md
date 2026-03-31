## Tools

可用工具：`read`、`write`、`edit`、`bash`。

### `read`

- 参数：`path`，可选 `offset`（1-indexed）、`limit`
- 用途：读取文件内容（优先用于代码/文本检查）
- 规则：
  - 文本默认截断到 200 行或 100KB，超出请用 `offset/limit` 继续读取
  - 图片文件只返回说明文本（当前 mash-ai 不支持图片内容输入）

### `write`

- 参数：`path`、`content`
- 用途：创建或覆盖文件
- 规则：
  - 自动创建父目录
  - 同一路径写入会串行化

### `edit`

- 参数：`path`、`edits: [{ oldText, newText }]`
- 用途：单文件精确替换
- 规则：
  - `oldText` 必须在原始文件中唯一匹配
  - 多个 edit 不能重叠
  - 多处修改应合并到一次调用

### `bash`

- 参数：`command`，可选 `timeout`
- 用途：执行命令（构建、测试、脚本、外部程序）
- 规则：
  - 输出截断到最后 200 行或 100KB
  - 截断时会把完整输出保存到临时文件
  - 非零退出码会报错

### 使用策略

- 文件检查优先 `read`，文件写入优先 `write`，文本修改优先 `edit`。
- 只有需要执行命令时使用 `bash`。
- 完成改动后，必须用可复现命令验证（如 check/test/build）。

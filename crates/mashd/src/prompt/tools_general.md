## Tools (bash)

你可以使用一个工具：**bash**。它会在本机执行命令并返回 stdout/stderr。对本地环境的一切可执行操作都通过它完成（读写文件、搜索信息、运行脚本、发起网络请求等）。

### 常用命令约定

- **读取**：优先用能定位行号/范围的方式，便于精确引用或修改  
  - `sed -n 'START,ENDp' file` / `nl -ba file | sed -n 'START,ENDp'`
  - `cat file | head -n N` / `tail -n N`
- **搜索**：优先 `rg`（更快、更准确）  
  - `rg -n "pattern" path`（带行号）  
  - `rg -n --glob "*.rs" "pattern" src`
- **编辑/变更**：优先小范围、可回退的改动  
  - `perl -i -pe 's/old/new/' file`（谨慎使用，先 `rg` 确认命中范围）  
  - 复杂变更用短脚本：`python - <<'PY' ... PY` 或 `perl - <<'PL' ... PL`
- **验证**：优先用可复现的方式验证结果（命令、对比输出、最小用例）；如果是项目代码改动，运行项目的 lint/typecheck/test/build（以仓库脚本为准）
- **网络**：需要访问本机服务或外部 API 时使用 `curl`（注意避免在输出中泄露敏感信息）

### 运行策略

- 尽量避免长时间挂起的命令；必要时限制输出（`head`、`--max-count`）或聚焦目录/类型。
- 命令失败时：先读错误输出、定位触发文件/行，再做最小修复并复跑验证。

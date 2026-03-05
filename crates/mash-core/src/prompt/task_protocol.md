
## Task List Protocol — Four Bash Tasks

**Task file path (use in every bash command below):**
`{path_str}`

当目标需要多步推进（例如需要多轮验证/排错）时，用任务列表管理进度；简单问题不必创建清单。

任务列表通过调用 **bash** 工具写入/更新任务文件完成。不要在回复正文里输出任务文件内容；UI 会从任务文件读取并展示。

---

**1. TaskCreate** — 创建任务列表  
在需要任务列表时调用一次：把清单写入任务文件（编号行 `- [ ] 1. ...`，每步一行）。

Example:
cat << 'EOF' > "{path_str}"
# Tasks

- [ ] 1. First step description
- [ ] 2. Second step description
- [ ] 3. Third step
EOF

---

**2. TaskUpdate** — 更新任务状态  
Call when a step is completed: mark item N as done (replace N with the step number).

{perl_cmd}

(Replace N in the perl script with the actual step number, e.g. 3 for step 3.)

---

**3. TaskList** — 列出所有任务  
Call to list all tasks (show the current task file content).

cat "{path_str}"

---

**4. TaskGet** — 获取任务详情  
Call to get full content or a specific part of the task file (e.g. grep for one line).

cat "{path_str}"
# or e.g. grep "^\- " "{path_str}" for checklist lines only

---

Use TaskCreate only when needed; use TaskUpdate when a step is done; use TaskList/TaskGet when you need to read the current list. The UI monitors the file and refreshes automatically.

# AGENTS.md

## CI/CD 规约

### GitHub Actions 构建监听

每次 `git push` 后，必须检查 GitHub Actions 构建结果：

1. **获取构建状态**：通过 GitHub API 查看 `https://api.github.com/repos/tomcat927/openlist-uploader/actions/runs?per_page=1`
2. **等待构建完成**：构建约 8-10 分钟，轮询直到 `status` 变为 `completed`
3. **检查构建结果**：
   - 如果 `conclusion` 为 `success`：构建成功，任务完成
   - 如果 `conclusion` 为 `failure`：获取构建日志，分析错误，修复代码，push，重复直到构建成功
4. **获取日志**：通过 API 获取失败的 job 日志：
   - `GET /repos/{owner}/{repo}/actions/runs/{run_id}/jobs` 获取 job_id
   - `GET /repos/{owner}/{repo}/actions/jobs/{job_id}/logs` 获取日志内容
   - 过滤 `error`、`Error`、`fail` 关键字定位错误
5. **常见错误处理**：
   - Rust 编译错误（`error[E0xxx]`）：检查类型不匹配、所有权问题、await 位置等
   - 前端编译错误（`TSxxxx`）：检查 TypeScript 类型
   - 构建步骤失败：检查依赖版本、路径问题

### 认证

GitHub API 需要 token 认证，使用 `Authorization: Bearer <token>` header。token 从环境或用户提供，不在代码中硬编码。

### 仓库信息

- 仓库：`tomcat927/openlist-uploader`
- 默认分支：`master`
- 构建 workflow：`.github/workflows/build-windows.yml`
- 构建触发：push 到 master 或手动 workflow_dispatch
- 并发策略：`cancel-in-progress: true`，新 push 取消旧构建

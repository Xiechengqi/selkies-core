# iVnc 强制更新功能规划

## 概述
参考 miao 项目的强制更新实现，为 iVnc 添加在线强制更新功能，允许用户通过 Web UI 一键更新到最新版本。

## 核心功能

### 1. 版本检查 API
- **端点**: `GET /api/version`
- **功能**: 返回当前版本和最新版本信息
- **实现位置**: `src/web/http_server.rs`
- **返回数据**:
  ```json
  {
    "current": "0.2.0",
    "latest": "0.3.0",
    "has_update": true,
    "download_url": "https://github.com/xxx/ivnc/releases/download/v0.3.0/ivnc-linux-amd64"
  }
  ```

### 2. 强制更新 WebSocket API
- **端点**: `GET /api/upgrade/ws?token=xxx`
- **功能**: 通过 WebSocket 实时推送更新进度
- **实现位置**: `src/web/http_server.rs`
- **消息格式**:
  ```json
  {
    "step": 3,
    "total_steps": 10,
    "message": "下载新版本...",
    "level": "info|error|success|progress",
    "progress": 45
  }
  ```

### 3. 更新流程（10步）

#### Step 1: 获取最新版本信息
- 从 GitHub API 获取 latest release
- API: `https://api.github.com/repos/{owner}/ivnc/releases/latest`
- 解析版本号和下载链接

#### Step 2: 检测系统架构
- 支持: `x86_64` (amd64), `aarch64` (arm64)
- 匹配对应的 release asset 名称
- 格式: `ivnc-linux-amd64`, `ivnc-linux-arm64`

#### Step 3: 下载新版本
- 使用 `reqwest` 下载二进制文件
- 实时报告下载进度（百分比）
- 超时设置: 300秒
- 临时保存路径: `/tmp/ivnc-new-{timestamp}`

#### Step 4: 验证下载
- 检查文件大小（至少 1MB）
- 可选：验证 SHA256 校验和（如果 release 提供）

#### Step 5: 备份当前版本
- 获取当前可执行文件路径: `std::env::current_exe()`
- 备份到: `{current_exe}.backup-{timestamp}`
- 保留权限: `0o755`

#### Step 6: 停止相关服务
- 如果有 Pake apps 运行，先停止
- 清理 WebRTC 会话
- 准备重启

#### Step 7: 替换二进制文件
- 删除旧的可执行文件
- 移动新文件到原位置
- 设置执行权限: `chmod +x`

#### Step 8: 验证新文件
- 检查文件是否可执行
- 可选：运行 `--version` 验证

#### Step 9: 清理临时文件
- 删除下载的临时文件
- 保留备份文件（供回滚使用）

#### Step 10: 重启服务
- 优先尝试 systemd 重启: `systemctl restart ivnc`
- 如果失败，使用 `exec()` 系统调用自我重启
- 传递原始命令行参数
- 如果重启失败，自动从备份恢复

### 4. 前端 UI 集成

#### 位置
- 在 Web UI 的设置页面或侧边栏添加"强制更新"按钮
- 参考 miao 的实现位置: `frontend/src/app/dashboard/layout.tsx`

#### 更新模态框
- 显示更新进度条（0-100%）
- 实时日志输出（带颜色区分）
- 状态指示: 运行中/成功/失败
- 成功后自动等待服务重启并刷新页面

#### 用户交互
1. 点击"强制更新"按钮
2. 弹出确认对话框: "确定要强制更新到最新版本吗？更新过程中服务将短暂中断。"
3. 确认后打开更新模态框
4. 建立 WebSocket 连接接收实时日志
5. 显示进度条和日志
6. 更新完成后等待服务重启（轮询 `/health` 端点）
7. 服务恢复后自动刷新页面

## 技术实现细节

### 后端结构

#### 新增数据结构
```rust
// src/web/http_server.rs

#[derive(Serialize)]
struct VersionInfo {
    current: String,
    latest: Option<String>,
    has_update: bool,
    download_url: Option<String>,
}

#[derive(Serialize)]
struct UpgradeLogEntry {
    step: u8,
    total_steps: u8,
    message: String,
    level: String,  // "info", "error", "success", "progress"
    progress: Option<u8>,  // 0-100
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}
```

#### 新增路由
```rust
// 在 run_http_server_with_webrtc() 中添加:
.route("/api/version", get(get_version_handler))
.route("/api/upgrade/ws", get(upgrade_ws_handler))
```

#### 核心函数
1. `async fn get_version_handler()` - 版本检查
2. `async fn upgrade_ws_handler()` - WebSocket 升级处理
3. `async fn handle_upgrade_websocket()` - WebSocket 连接处理
4. `async fn perform_upgrade_with_logs()` - 执行更新逻辑
5. `async fn try_restart_systemd()` - 尝试 systemd 重启

### 前端结构

#### 状态管理
```typescript
const [upgrading, setUpgrading] = useState(false);
const [showUpgradeModal, setShowUpgradeModal] = useState(false);
const [upgradeLogs, setUpgradeLogs] = useState<UpgradeLogEntry[]>([]);
const [upgradeProgress, setUpgradeProgress] = useState(0);
const [upgradeStatus, setUpgradeStatus] = useState<"running" | "success" | "error">("running");
```

#### WebSocket 连接
```typescript
const wsProtocol = window.location.protocol === "https:" ? "wss:" : "ws:";
const wsUrl = `${wsProtocol}//${window.location.host}/api/upgrade/ws?token=${token}`;
const ws = new WebSocket(wsUrl);
```

## 安全考虑

### 1. 认证保护
- 更新 API 需要认证（Basic Auth 或 Token）
- WebSocket 连接需要 token 参数验证

### 2. 权限检查
- 确保进程有权限替换自身可执行文件
- 检查文件系统写入权限

### 3. 回滚机制
- 保留备份文件
- 如果新版本启动失败，自动恢复备份
- 备份文件命名包含时间戳，便于追溯

### 4. 下载验证
- 验证下载文件大小
- 可选：验证 SHA256 校验和
- 检查文件可执行性

## 配置选项

### 环境变量
- `IVNC_UPDATE_REPO`: GitHub 仓库路径（默认: `owner/ivnc`）
- `IVNC_UPDATE_DISABLED`: 禁用更新功能（默认: false）
- `IVNC_BACKUP_DIR`: 备份文件目录（默认: 与可执行文件同目录）

### 配置文件
```toml
[update]
enabled = true
github_repo = "owner/ivnc"
check_interval_hours = 24  # 自动检查更新间隔
auto_backup = true
backup_retention_days = 7  # 备份保留天数
```

## 部署场景适配

### 1. Systemd 服务
- 优先使用 `systemctl restart ivnc`
- 需要配置 service 文件支持重启

### 2. Docker 容器
- 不支持容器内更新（提示用户更新镜像）
- 检测是否在容器中运行: 检查 `/.dockerenv` 或 `/proc/1/cgroup`

### 3. 手动运行
- 使用 `exec()` 系统调用自我重启
- 传递原始命令行参数

## 错误处理

### 常见错误场景
1. **网络错误**: 无法连接 GitHub API
   - 提示用户检查网络连接
   - 提供手动下载链接

2. **权限错误**: 无法替换可执行文件
   - 提示需要 root 权限
   - 建议使用 sudo 运行

3. **架构不匹配**: 找不到对应架构的 release
   - 提示当前架构不支持
   - 显示可用的架构列表

4. **下载失败**: 下载中断或超时
   - 自动重试（最多3次）
   - 清理临时文件

5. **启动失败**: 新版本无法启动
   - 自动从备份恢复
   - 记录错误日志

## 测试计划

### 单元测试
- [ ] 版本比较逻辑
- [ ] 架构检测
- [ ] 文件权限处理
- [ ] 备份和恢复逻辑

### 集成测试
- [ ] 完整更新流程（使用测试 release）
- [ ] WebSocket 消息推送
- [ ] 错误场景处理
- [ ] 回滚机制

### 手动测试
- [ ] 在不同 Linux 发行版测试
- [ ] Systemd 服务场景
- [ ] 手动运行场景
- [ ] 网络异常场景
- [ ] 权限不足场景

## 实施步骤

### Phase 1: 后端基础 API
1. 实现版本检查 API
2. 实现 GitHub release 信息获取
3. 添加架构检测逻辑

### Phase 2: 更新核心逻辑
1. 实现下载功能（带进度）
2. 实现备份和替换逻辑
3. 实现重启机制
4. 添加错误处理和回滚

### Phase 3: WebSocket 实时推送
1. 实现 WebSocket 端点
2. 实现日志推送机制
3. 集成更新流程和日志推送

### Phase 4: 前端 UI
1. 添加版本信息显示
2. 实现更新按钮和确认对话框
3. 实现更新进度模态框
4. 实现 WebSocket 连接和日志显示
5. 实现自动刷新逻辑

### Phase 5: 测试和优化
1. 单元测试
2. 集成测试
3. 性能优化
4. 文档完善

## 参考文件

### Miao 项目
- 后端: `/data/projects/miao/src/main.rs` (行 5395-5728, 11504, 11580)
- 前端: `/data/projects/miao/frontend/src/app/dashboard/layout.tsx` (行 123-186, 405-486)

### iVnc 项目
- HTTP 服务器: `/data/projects/iVnc/src/web/http_server.rs`
- 配置: `/data/projects/iVnc/Cargo.toml`
- 当前版本: `0.2.0`

## 注意事项

1. **不移除版本检查**: 与 miao 不同，建议保留版本检查，只在版本不同时才允许更新
2. **GitHub Token**: 如果 API 请求频繁，考虑支持 GitHub Token 避免速率限制
3. **Release 命名规范**: 确保 GitHub release 的 asset 命名遵循约定
4. **日志记录**: 详细记录更新过程，便于排查问题
5. **用户通知**: 更新前明确告知用户服务将中断
6. **备份管理**: 定期清理旧备份文件，避免占用过多空间

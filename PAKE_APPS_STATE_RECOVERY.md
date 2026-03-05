# Pake Apps 状态恢复功能

## 功能概述

在强制更新重启后，自动恢复之前运行的 Pake 应用程序。

## 工作原理

### 1. 状态保存（更新前）

在执行强制更新时（Step 0），系统会：
- 扫描所有 Pake apps
- 识别当前正在运行的 apps
- 将运行中的 app IDs 保存到状态文件

**状态文件位置**：
```
~/.config/ivnc/app_running_state.json
```

**状态文件格式**：
```json
{
  "app_ids": ["app-id-1", "app-id-2"],
  "timestamp": 1709654321
}
```

### 2. 状态恢复（启动后）

系统启动后 2 秒：
- 读取状态文件
- 验证状态是否新鲜（5 分钟内）
- 依次启动之前运行的 apps
- 每个 app 启动间隔 500ms
- 完成后删除状态文件

## 实现细节

### 新增文件

**src/pake_apps/state_recovery.rs**
- `AppRunningState` 结构体
- `save()` - 保存状态到文件
- `load()` - 从文件加载状态
- `clear()` - 清除状态文件
- `is_recent()` - 检查状态是否新鲜

### 修改文件

**src/pake_apps/mod.rs**
- 添加 `state_recovery` 模块

**src/pake_apps/api.rs**
- `PakeState::save_running_state()` - 保存当前运行状态
- `PakeState::restore_running_state()` - 恢复运行状态

**src/web/http_server.rs**
- 在 `perform_upgrade_with_logs()` 开始时保存状态

**src/main.rs**
- 在 Pake apps 初始化后启动恢复任务

## 使用场景

### 场景 1: 正常更新流程

1. 用户点击"强制更新"
2. 系统保存当前运行的 apps（如 Chrome, VSCode）
3. 下载并安装新版本
4. 系统重启
5. 2 秒后自动恢复 Chrome 和 VSCode

### 场景 2: 更新失败

1. 用户点击"强制更新"
2. 系统保存状态
3. 更新过程中出错
4. 系统回滚到旧版本
5. 重启后仍然恢复之前的 apps

### 场景 3: 手动重启

1. 用户手动重启 iVnc（非更新）
2. 没有状态文件
3. 不会自动启动任何 apps
4. 用户需要手动启动或使用 autostart 功能

## 安全机制

### 1. 时间窗口限制

只恢复 5 分钟内的状态：
```rust
pub fn is_recent(&self) -> bool {
    let now = SystemTime::now()...;
    now - self.timestamp < 300  // 5 minutes
}
```

**原因**：
- 防止恢复过时的状态
- 避免在长时间关机后意外启动 apps

### 2. 延迟启动

启动后等待 2 秒再恢复：
```rust
tokio::time::sleep(Duration::from_secs(2)).await;
```

**原因**：
- 让系统稳定
- 确保所有服务已就绪

### 3. 启动间隔

每个 app 启动间隔 500ms：
```rust
tokio::time::sleep(Duration::from_millis(500)).await;
```

**原因**：
- 避免同时启动过多进程
- 减少系统负载

### 4. 错误容忍

单个 app 启动失败不影响其他：
```rust
if let Err(e) = result {
    log::warn!("Failed to restore app {}: {}", app_id, e);
    // 继续启动下一个
}
```

## 日志输出

### 保存状态
```
[INFO] Saved running apps state: ["app-1", "app-2"]
```

### 恢复状态
```
[INFO] Loaded running apps state: ["app-1", "app-2"]
[INFO] Restoring 2 running apps
[INFO] Restoring app: Chrome (app-1)
[INFO] Restoring app: VSCode (app-2)
[INFO] Running apps restoration completed
```

### 无状态
```
[INFO] No previous running state to restore
```

### 状态过期
```
[INFO] Running state is too old, skipping restore
[INFO] Cleared running apps state file
```

## 更新日志显示

在强制更新的 WebSocket 日志中：

```
[0/10] 保存运行中的应用状态...
[0/10] 应用状态已保存
[1/10] 检测系统架构...
...
```

如果保存失败：
```
[0/10] 保存运行中的应用状态...
[0/10] 保存应用状态失败: Failed to write state file
```

## 测试方法

### 1. 基本测试

```bash
# 1. 启动 iVnc
./ivnc

# 2. 在 /console 页面启动几个 apps

# 3. 触发强制更新
curl -X POST http://localhost:8080/api/upgrade/ws

# 4. 等待重启

# 5. 检查 apps 是否自动恢复
```

### 2. 手动测试状态文件

```bash
# 查看状态文件
cat ~/.config/ivnc/app_running_state.json

# 手动创建状态文件
echo '{"app_ids":["test-app"],"timestamp":1709654321}' > ~/.config/ivnc/app_running_state.json

# 重启 iVnc 查看是否恢复
```

### 3. 测试时间窗口

```bash
# 创建一个旧的状态文件（超过 5 分钟）
echo '{"app_ids":["test-app"],"timestamp":1000000000}' > ~/.config/ivnc/app_running_state.json

# 重启 iVnc，应该不会恢复
# 日志应显示: "Running state is too old, skipping restore"
```

## 限制和注意事项

### 1. 仅恢复运行状态

- ✅ 恢复：哪些 apps 在运行
- ❌ 不恢复：窗口位置、大小、内容

### 2. 依赖 autostart 配置

如果 app 设置了 autostart：
- 更新后会被恢复（通过状态文件）
- 下次系统启动也会自动启动（通过 autostart）

### 3. Native vs Webview

两种模式都支持：
- **Native 模式**：启动 Chrome 进程
- **Webview 模式**：启动 WebView 窗口

### 4. 错误处理

如果 app 启动失败：
- 记录警告日志
- 继续尝试启动其他 apps
- 不会阻止系统启动

## 配置选项（未来）

可以考虑添加配置选项：

```toml
[pake_apps]
# 启用状态恢复
restore_on_restart = true

# 状态有效期（秒）
state_validity_seconds = 300

# 启动延迟（秒）
restore_delay_seconds = 2

# app 启动间隔（毫秒）
start_interval_ms = 500
```

## 与 Autostart 的区别

| 特性 | 状态恢复 | Autostart |
|------|---------|-----------|
| 触发时机 | 更新重启后 | 每次系统启动 |
| 配置方式 | 自动（基于运行状态） | 手动设置 |
| 持久性 | 一次性 | 永久 |
| 用途 | 恢复更新前状态 | 开机自启动 |

**建议**：
- 使用 Autostart 设置常用 apps
- 状态恢复作为更新后的补充

## 故障排除

### 问题 1: Apps 没有恢复

**检查**：
```bash
# 1. 查看状态文件是否存在
ls -la ~/.config/ivnc/app_running_state.json

# 2. 查看日志
journalctl -u ivnc -n 50 | grep -i restore

# 3. 检查时间戳
cat ~/.config/ivnc/app_running_state.json
```

### 问题 2: 恢复了不该恢复的 apps

**原因**：更新前这些 apps 正在运行

**解决**：
- 更新前先停止不需要的 apps
- 或者手动删除状态文件

### 问题 3: 恢复失败

**可能原因**：
- App 配置已更改
- App 数据损坏
- 系统资源不足

**解决**：
- 查看日志中的错误信息
- 手动启动 app 测试
- 检查 app 数据目录

## 总结

状态恢复功能确保用户在强制更新后无需手动重启之前运行的应用程序，提供无缝的更新体验。

**关键特性**：
- 🔄 自动保存和恢复
- ⏱️ 时间窗口保护
- 🛡️ 错误容忍
- 📝 详细日志
- 🚀 延迟启动避免冲突

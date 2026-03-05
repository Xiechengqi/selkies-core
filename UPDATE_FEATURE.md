# 强制更新功能使用说明

## 功能概述

iVnc 现已支持在线强制更新功能，可以通过 Web API 一键更新到最新版本。

## 后端 API

### 1. 版本检查 API

**端点**: `GET /api/version`

**响应示例**:
```json
{
  "current": "0.2.0",
  "latest": "0.3.0",
  "has_update": true,
  "download_url": "https://github.com/Xiechengqi/iVnc/releases/download/latest/ivnc-linux-amd64"
}
```

**测试命令**:
```bash
curl http://localhost:8080/api/version
```

### 2. 强制更新 WebSocket API

**端点**: `GET /api/upgrade/ws`

**功能**: 通过 WebSocket 实时推送更新进度

**消息格式**:
```json
{
  "step": 3,
  "total_steps": 10,
  "message": "下载中... 5/10 MB",
  "level": "progress",
  "progress": 50
}
```

**level 类型**:
- `info`: 信息提示
- `success`: 成功消息
- `error`: 错误消息
- `progress`: 进度更新

## 更新流程

1. **检测架构** - 自动识别 x86_64 或 aarch64
2. **准备下载** - 构建 GitHub release 下载链接
3. **下载新版本** - 实时显示下载进度
4. **验证文件** - 检查文件大小和完整性
5. **备份当前版本** - 创建带时间戳的备份
6. **设置权限** - 设置可执行权限 (0755)
7. **替换文件** - 原子替换二进制文件
8. **清理临时文件** - 删除下载的临时文件
9. **准备重启** - 等待服务准备
10. **重启服务** - 优先使用 systemd，失败则 exec 自我重启

## 前端集成示例

### JavaScript/TypeScript

```typescript
// 检查版本
async function checkVersion() {
  const response = await fetch('/api/version');
  const data = await response.json();
  console.log('Current:', data.current);
  console.log('Latest:', data.latest);
  console.log('Has update:', data.has_update);
  return data;
}

// 执行更新
function performUpgrade() {
  const ws = new WebSocket('ws://localhost:8080/api/upgrade/ws');

  ws.onmessage = (event) => {
    const log = JSON.parse(event.data);
    console.log(`[${log.step}/${log.total_steps}] ${log.message}`);

    if (log.level === 'error') {
      console.error('Update failed:', log.message);
    }

    if (log.progress !== undefined) {
      console.log(`Progress: ${log.progress}%`);
    }
  };

  ws.onclose = () => {
    console.log('Update completed, waiting for restart...');
    waitForRestart();
  };

  ws.onerror = (error) => {
    console.error('WebSocket error:', error);
  };
}

// 等待服务重启
async function waitForRestart() {
  for (let i = 0; i < 30; i++) {
    await new Promise(resolve => setTimeout(resolve, 1000));
    try {
      const response = await fetch('/health');
      if (response.ok) {
        window.location.reload();
        return;
      }
    } catch {
      // Continue waiting
    }
  }
  alert('Service did not restart, please refresh manually');
}
```

### HTML 示例

```html
<!DOCTYPE html>
<html>
<head>
  <title>iVnc Update</title>
</head>
<body>
  <h1>iVnc 更新管理</h1>

  <div id="version-info">
    <p>当前版本: <span id="current-version">-</span></p>
    <p>最新版本: <span id="latest-version">-</span></p>
  </div>

  <button id="check-btn" onclick="checkVersion()">检查更新</button>
  <button id="update-btn" onclick="performUpgrade()" disabled>强制更新</button>

  <div id="progress" style="display:none;">
    <h3>更新进度</h3>
    <progress id="progress-bar" value="0" max="100"></progress>
    <div id="logs" style="height:300px; overflow-y:auto; border:1px solid #ccc; padding:10px;">
    </div>
  </div>

  <script>
    async function checkVersion() {
      const response = await fetch('/api/version');
      const data = await response.json();

      document.getElementById('current-version').textContent = data.current;
      document.getElementById('latest-version').textContent = data.latest;
      document.getElementById('update-btn').disabled = !data.has_update;
    }

    function performUpgrade() {
      if (!confirm('确定要强制更新到最新版本吗？\n更新过程中服务将短暂中断。')) {
        return;
      }

      document.getElementById('progress').style.display = 'block';
      document.getElementById('update-btn').disabled = true;

      const ws = new WebSocket('ws://' + window.location.host + '/api/upgrade/ws');
      const logsDiv = document.getElementById('logs');

      ws.onmessage = (event) => {
        const log = JSON.parse(event.data);
        const logEntry = document.createElement('div');
        logEntry.className = 'log-' + log.level;
        logEntry.textContent = `[${log.step}/${log.total_steps}] ${log.message}`;
        logsDiv.appendChild(logEntry);
        logsDiv.scrollTop = logsDiv.scrollHeight;

        if (log.progress !== undefined) {
          document.getElementById('progress-bar').value = log.progress;
        }
      };

      ws.onclose = () => {
        const logEntry = document.createElement('div');
        logEntry.textContent = '更新完成，等待服务重启...';
        logsDiv.appendChild(logEntry);
        waitForRestart();
      };
    }

    async function waitForRestart() {
      for (let i = 0; i < 30; i++) {
        await new Promise(resolve => setTimeout(resolve, 1000));
        try {
          const response = await fetch('/health');
          if (response.ok) {
            window.location.reload();
            return;
          }
        } catch {}
      }
      alert('更新后未检测到服务恢复，请稍后手动刷新');
    }

    // 页面加载时检查版本
    checkVersion();
  </script>
</body>
</html>
```

## 安全注意事项

1. **认证**: 更新 API 受 Basic Auth 保护（如果启用）
2. **权限**: 确保进程有权限替换自身可执行文件
3. **备份**: 每次更新都会创建备份文件
4. **回滚**: 如果新版本启动失败，会自动恢复备份

## 部署建议

### Systemd 服务

如果使用 systemd 管理 iVnc，更新会自动使用 `systemctl restart ivnc`：

```ini
[Unit]
Description=iVnc WebRTC Desktop Streaming
After=network.target

[Service]
Type=simple
User=root
ExecStart=/usr/local/bin/ivnc
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
```

### 手动运行

如果手动运行 iVnc，更新会使用 `exec()` 系统调用自我重启，保留原始命令行参数。

## 故障排除

### 更新失败

1. 检查网络连接到 GitHub
2. 确认有足够的磁盘空间
3. 验证进程有写入权限
4. 查看日志中的错误信息

### 服务未重启

1. 检查 systemd 服务状态: `systemctl status ivnc`
2. 查看系统日志: `journalctl -u ivnc -n 50`
3. 手动重启: `systemctl restart ivnc`

### 回滚到旧版本

如果需要回滚，备份文件位于可执行文件同目录：

```bash
# 查找备份文件
ls -la /usr/local/bin/ivnc.backup-*

# 恢复备份
cp /usr/local/bin/ivnc.backup-1234567890 /usr/local/bin/ivnc
chmod +x /usr/local/bin/ivnc
systemctl restart ivnc
```

## 测试

### 本地测试

```bash
# 1. 启动 iVnc
./ivnc

# 2. 检查版本
curl http://localhost:8080/api/version

# 3. 使用 websocat 测试 WebSocket
websocat ws://localhost:8080/api/upgrade/ws
```

### 创建测试 Release

在 GitHub 上创建 release 时，确保：
1. Tag 格式: `v0.3.0`
2. Asset 命名: `ivnc-linux-amd64`, `ivnc-linux-arm64`
3. 使用 `latest` 标签或作为最新 release

## 配置选项

未来可以添加配置选项：

```toml
[update]
enabled = true
github_repo = "Xiechengqi/iVnc"
check_interval_hours = 24
auto_backup = true
backup_retention_days = 7
```

## 限制

1. 仅支持 Linux x86_64 和 aarch64 架构
2. 需要进程有权限替换自身可执行文件
3. Docker 容器内不支持（需要更新镜像）
4. 更新过程中服务会短暂中断（通常 < 10 秒）

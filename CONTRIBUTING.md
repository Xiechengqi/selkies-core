# 贡献指南

感谢您对 selkies-core 项目的关注！

## 开发环境设置

### 1. 克隆仓库

```bash
git clone https://github.com/your-org/selkies-core.git
cd selkies-core
```

### 2. 安装依赖

**Ubuntu/Debian:**
```bash
sudo apt-get install -y \
    build-essential \
    pkg-config \
    libjpeg-turbo8-dev \
    libx11-dev \
    libxcb1-dev \
    libxkbcommon-dev \
    libgstreamer1.0-dev \
    libgstreamer-plugins-base1.0-dev
```

### 3. 构建项目

```bash
# WebSocket-only mode (no GStreamer required)
make build-websocket

# WebRTC mode (requires GStreamer)
make build
```

### 4. 运行测试

```bash
make test
```

## 代码规范

### Rust 代码风格

- 使用 `cargo fmt` 格式化代码
- 使用 `cargo clippy` 检查代码质量
- 遵循 Rust 官方风格指南

### 提交信息格式

```
<type>(<scope>): <subject>

<body>

<footer>
```

**类型 (type):**
- `feat`: 新功能
- `fix`: 修复 bug
- `docs`: 文档更新
- `style`: 代码格式调整
- `refactor`: 重构
- `test`: 测试相关
- `chore`: 构建/工具相关

**示例:**
```
feat(webrtc): add adaptive bitrate control

Implement dynamic bitrate adjustment based on network conditions.
Uses REMB feedback to optimize video quality.

Closes #123
```

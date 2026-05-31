<div align="center">
<h1>OY</h1>
<h3>AI 智能体工作区（CLI + TUI）</h3>
<p>
  <a href="https://github.com/cherish-ltt/oy/actions/workflows/rust-ci.yml">
    <img src="https://img.shields.io/github/actions/workflow/status/cherish-ltt/oy/rust-ci.yml?branch=main" alt="Build Status"/>
  </a>
  <a href="https://github.com/cherish-ltt/u2secure/blob/main/LICENSE">
    <img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="license"/>
  </a>
  <a href="https://www.rust-lang.org">
    <img src="https://img.shields.io/badge/rust-1.95.0+-orange.svg" alt="license"/>
  </a>
</p>
</div>



> 🚀 一个 Rust 工作区，实现了基于 OpenRouter / 兼容 API 的函数调用 AI 智能体，提供 Read/Write/Edit/Bash 四类工具。基于**领域驱动设计**和**洋葱架构**原则，构建在四个隔离的 crate 之上。



## 架构

```
oy (CLI 入口, 默认启动 TUI)  ────
                               ├──► oy-agent (编排) ──► oy-ai (AI 提供者抽象)
oy (oy -p "prompt" 直接执行) ────
```

| Crate | 层 | 描述 |
| :--- | :--- | :--- |
| [`oy-ai`](./oy-ai/) | 核心 | AI 提供者 trait、值对象（ChatMessage、ToolCall）、OpenRouter 实现 |
| [`oy-agent`](./oy-agent/) | 领域 | Agent 实体、Tool trait、编排器循环、工具实现（Read/Write/Edit/Bash）、会话持久化 |
| [`oy-code-cli`](./oy-code-cli/) | 基础设施 | CLI 参数解析（clap）、二进制入口 `oy` |
| [`oy-tui`](./oy-tui/) | 基础设施 | 基于 ratatui 的 TUI shell，markdown 渲染、命令系统、主题切换 |

### 依赖方向

- `oy-code-cli` → `oy-agent` → `oy-ai`
- `oy-tui` → `oy-agent` → `oy-ai`
- `oy-code-cli` → `oy-tui`（无 prompt 时启动 TUI）

## 快速开始

### 前置条件

- Rust 1.85+（`rust-toolchain.toml` 自动配置）
- 一个 OpenRouter 或兼容的 API 密钥

### 配置

`~/.oy-ai-agent/config.toml`：

```toml
api_key = "sk-or-..."
base_url = "https://openrouter.ai/api/v1"
model = "anthropic/claude-haiku-4.5"
theme = "light"          # "light" 或 "dark"
```

也可通过环境变量设置：

```bash
export OPENROUTER_API_KEY="sk-or-..."
export OPENROUTER_BASE_URL="https://openrouter.ai/api/v1"
export OPENROUTER_MODEL="anthropic/claude-haiku-4.5"
```

### 安装

> **支持平台：** macOS (arm64)、Linux (x86_64, Ubuntu latest, GNU)、Windows (x86_64)

通过 **bun**（推荐）快速安装：

```bash
bun install -g @ghyper9023/oy --registry https://registry.npmjs.org/
```

通过 **npm** 安装：

```bash
npm install -g @ghyper9023/oy
```

或通过 **cargo** 从源码编译（推荐，享受最新特性）：

```bash
cargo install oy-code-cli
```

> 💡 推荐排序：**bun > cargo 自编译 > npm**。bun 安装速度最快；cargo 自编译可获得当前架构最佳优化。

### 运行

```bash
# 启动 TUI（默认）
oy

# 或直接使用 CLI 模式
oy -p "当前目录下有哪些文件？"
```

### 本地开发

```bash
# 构建
cargo build --workspace

# 启动 TUI
cargo run -p oy-code-cli

# 测试
cargo test --workspace
```

## TUI 功能

### 命令系统

在输入框输入 `/` 弹出命令选择器：

| 命令 | 功能 |
|------|------|
| `/model` | 三步表单设置 API Base URL / API Key / Model，保存 config，重启 agent 且保留会话消息 |
| `/settings` → `/theme` | 切换 light / dark 主题，持久化到 config.toml |

快捷键：`↑`/`↓` 导航选择器，`Enter` 确认，`Esc` 取消，`Ctrl+O` 展开/折叠工具结果。

### Markdown 渲染

AI 回复内容以 Markdown 格式渲染，支持：

| 语法 | 显示 |
|------|------|
| `**bold**` | 加粗 |
| `*italic*` | 斜体 |
| `` `code` `` | 黑底青字 |
| ` ``` ` 代码块 | 独立段落，代码高亮 |
| `# Heading` | 标题 |
| `- list` | 列表 |
| `> quote` | 引用块 |
| `---` | 分割线 |

### 工具调用 & 结果显示

- 每个工具调用显示为 `🔧 工具调用 · Read`，含实时计时器
- 结果返回后合并显示在调用下方，带用时时长 `✓ (0.5s)`
- Read 结果折叠至前 5 行，Bash 结果折叠至后 5 行
- `Ctrl+O` 展开全部内容

### 主题

内置 light / dark 双主题：
- **light**：白底，消息气泡淡蓝/淡绿/淡紫
- **dark**：黑底，消息气泡深蓝/深绿/深紫

通过 `/settings` → `/theme` 切换，设置持久化到 config.toml。

### 消息背景色

每条消息独立背景色，整行填充：

| 消息类型 | Light 主题 | Dark 主题 |
|---------|-----------|----------|
| User | `Rgb(235,240,255)` 浅蓝 | `Rgb(25,30,50)` 深蓝 |
| Assistant | `Rgb(235,255,235)` 浅绿 | `Rgb(20,30,20)` 深绿 |
| Tool | `Rgb(255,235,255)` 浅紫 | `Rgb(40,20,40)` 深紫 |

### Agent 状态指示器

状态栏显示 agent 运行状态：`•` 空闲，`⠋⠙⠹…` 旋转动画表示工作中。

## 可用工具

| 工具 | 功能 | 输入 |
|------|------|------|
| **Read** | 读取文件内容 | `file_path` |
| **Write** | 创建或覆盖文件 | `file_path`, `content` |
| **Edit** | 精确替换文件中的文本段 | `file_path`, `old_text`, `new_text` |
| **Bash** | 通过 `sh -c` 执行命令 | `command`（含命令黑名单） |

## 项目结构

```
oy/
├── Cargo.toml                     # 工作区
├── oy-ai/                         # AI 提供者抽象
│   └── src/
│       ├── domain/ (AiProvider trait, ChatMessage, ToolCall, AiConfig, AiError)
│       └── infrastructure/ (OpenRouter 实现)
├── oy-agent/                      # 智能体编排
│   ├── tests/integration_test.rs  # MockProvider 集成测试
│   └── src/
│       ├── domain/ (Agent trait, Tool trait, ToolRegistry, AgentError)
│       ├── application/ (Orchestrator 主循环)
│       └── infrastructure/ (ReadTool, WriteTool, EditTool, BashTool, 持久化)
├── oy-code-cli/                   # CLI 二进制 `oy`
│   └── src/ (CliArgs, run())
└── oy-tui/                        # TUI 二进制
    └── src/
        ├── app.rs                 # 应用状态、事件循环、键盘处理、命令执行
        ├── ui.rs                  # ratatui 渲染（消息区、输入框、状态栏、弹出框）
        ├── message.rs             # Message 枚举、to_lines() MD 渲染、visual_line_count
        ├── command.rs             # 命令注册器、CommandInfo、CommandItem
        ├── event.rs               # 事件循环（tick、crossterm、agent 信号）
        ├── load_config.rs         # config.toml 加载/保存
        ├── theme.rs               # Theme 结构体、DARK_THEME / LIGHT_THEME
        ├── agent.rs               # AgentManager
        └── main.rs                # 入口
```

## 测试

```
cargo test --workspace     # 116 个测试
```

| 测试套件 | 位置 | 测试数 |
|---------|------|--------|
| oy-ai 单元 | `oy-ai/src/domain/` | 23 |
| oy-agent 单元 + 工具 | `oy-agent/src/` | 58 |
| oy-agent 集成 | `oy-agent/tests/` | 5 |
| oy-tui 单元 | `oy-tui/src/` | 30 |

## 会话持久化

对话历史自动保存到 `~/.oy-ai-agent/sessions/<项目路径>/<uuidv7>.json`。切换模型时（`/model` 命令），旧 agent 的上下文通过 `oneshot` channel 提取，注入到新 agent，确保会话不丢失。

## 许可证

MIT

---

<div align="center">
  <sub>Built with ❤️ by the OY team</sub>
</div>
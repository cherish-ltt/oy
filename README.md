# oy — 多 crate AI 智能体工作区

一个 Rust 工作区，实现了一个通过 [OpenRouter](https://openrouter.ai/) 使用函数调用的 AI 智能体，以执行 Read/Write/Bash 工具。基于**领域驱动设计**和**洋葱架构**原则，构建在四个隔离的 crate 之上。

## 架构

```
oy-cli (CLI 入口) ──┐
                     ├──► oy-agent (编排) ──► oy-ai (AI 提供者抽象)
oy-tui (TUI shell) ──┘
```

| Crate | 层 | 描述 |
| :--- | :--- | :--- |
| [`oy-ai`](./oy-ai/) | 核心（最内层） | AI 提供者 trait、值对象（ChatMessage、ToolCall）、OpenRouter 实现 |
| [`oy-agent`](./oy-agent/) | 领域 | Agent 实体、Tool trait、编排器循环、工具实现（Read/Write/Bash）、会话持久化 |
| [`oy-cli`](./oy-cli/) | 基础设施 | CLI 参数解析（clap）、配置文件加载器、二进制入口点 |
| [`oy-tui`](./oy-tui/) | 基础设施 | 未来基于 ratatui 的 TUI 占位二进制 crate（当前为空） |

### 依赖方向

每个外层依赖内层。没有循环依赖。

- `oy-cli` → `oy-agent` → `oy-ai`
- `oy-tui` → `oy-agent` → `oy-ai`

## 快速开始

### 前置条件

- Rust 1.85+（通过 `rust-toolchain.toml` 自动配置工具链）
- 一个 [OpenRouter](https://openrouter.ai/) API 密钥

### 配置

通过以下方式之一设置 API 密钥（优先级顺序）：

1. **CLI**：不适用 — prompt 和 model 是 CLI 参数，api_key 不是
2. **配置文件**：`~/.oy-ai-agent/config.toml`
   ```toml
   api_key = "sk-or-..."
   base_url = "https://openrouter.ai/api/v1"       # 可选
   model = "anthropic/claude-haiku-4.5"             # 可选
   ```
3. **环境变量**：
   ```bash
   export OPENROUTER_API_KEY="sk-or-..."
   export OPENROUTER_BASE_URL="https://openrouter.ai/api/v1"  # 可选
   export OPENROUTER_MODEL="anthropic/claude-haiku-4.5"       # 可选
   ```

### 构建与运行

```bash
# 构建整个工作区
cargo build --workspace

# 运行 CLI 智能体
cargo run -p oy-cli -- -p "你的提示词"

# 使用特定模型运行
cargo run -p oy-cli -- -p "你好" -m "openai/gpt-4o"

# 在所有 crate 中运行测试
cargo test --workspace
```

**示例：**

```bash
cargo run -p oy-cli -- -p "当前目录下有哪些文件？"
```

智能体将决定使用 Read、Write 还是 Bash 来满足你的请求，循环直到给出最终答案。

## 项目结构

```
oy/
├── Cargo.toml                     # 工作区定义
├── rust-toolchain.toml            # Rust 版本锁定
├── .rustfmt.toml                  # 共享格式化配置
├── oy-ai/                         # AI 提供者抽象（最内层）
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── domain/
│       │   ├── mod.rs
│       │   ├── ai_provider.rs     # AiProvider trait
│       │   ├── chat_message.rs    # ChatMessage、Role、ToolCall 值对象
│       │   ├── config.rs          # AiConfig 结构体
│       │   └── errors.rs          # AiError 枚举
│       └── infrastructure/
│           ├── mod.rs
│           └── openrouter_provider.rs  # OpenRouterProvider 实现
├── oy-agent/                      # 智能体编排
│   ├── Cargo.toml
│   ├── tests/
│   │   └── integration_test.rs    # 使用 MockProvider 的集成测试
│   └── src/
│       ├── lib.rs
│       ├── domain/
│       │   ├── mod.rs
│       │   ├── agent.rs           # Agent 实体 + ToolRegistry
│       │   ├── tool.rs            # Tool trait
│       │   └── errors.rs          # AgentError 枚举
│       ├── application/
│       │   ├── mod.rs
│       │   └── orchestrator.rs    # Orchestrator：主智能体循环
│       └── infrastructure/
│           ├── mod.rs
│           ├── persistence.rs     # 会话保存/加载
│           ├── tool_executor.rs   # 扩展点（当前未使用）
│           └── tools/
│               ├── mod.rs
│               ├── read.rs        # Read 工具
│               ├── write.rs       # Write 工具
│               └── bash.rs        # Bash 工具（带命令黑名单）
├── oy-cli/                        # CLI 二进制 + 库
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs                 # CliArgs、CliConfig、build_provider_config、run
│       └── main.rs                # 二进制入口点
└── oy-tui/                        # TUI 占位
    ├── Cargo.toml
    └── src/
        └── main.rs                # 空的 main
```

## 领域设计

### 每个 crate 的洋葱层

**oy-ai：**
| 层 | 模块 | 职责 |
| :--- | :--- | :--- |
| 领域 | `domain/` | `AiProvider` trait、`ChatMessage`/`ToolCall`/`Role` 值对象、`AiConfig`、`AiError` |
| 基础设施 | `infrastructure/` | `OpenRouterProvider` — 基于 async-openai 的 `AiProvider` 实现 |

**oy-agent：**
| 层 | 模块 | 职责 |
| :--- | :--- | :--- |
| 领域 | `domain/` | `Agent` 实体（对话状态）、`Tool` trait、`ToolRegistry`、`AgentError` |
| 应用 | `application/` | `Orchestrator` — 在主循环中协调 provider + agent + tools |
| 基础设施 | `infrastructure/` | `ReadTool`/`WriteTool`/`BashTool` 实现、会话持久化 |

## 可用工具

智能体可以访问这些工具，它们以 JSON Schema 形式发送给模型：

### Read
读取并返回文件内容。
- **输入**：`file_path`（字符串，必需）

### Write
将内容写入文件（创建或覆盖）。
- **输入**：`file_path`（字符串，必需）、`content`（字符串，必需）

### Bash
通过 `sh -c` 执行 shell 命令。
- **输入**：`command`（字符串，必需）
- **安全**：危险命令（`rm -rf /`、`rm -rf /*`、包含 ` sudo ` 的命令）在执行前被拒绝。这是一个简单的字符串匹配黑名单 — 生产环境请使用操作系统级沙箱。

## 配置参考

| 来源 | 键 | 默认值 |
| :--- | :--- | :--- |
| 环境变量 / 配置文件 | `OPENROUTER_API_KEY` / `api_key` | *（必需 — 未设置则进程退出）* |
| 环境变量 / 配置文件 | `OPENROUTER_BASE_URL` / `base_url` | `https://openrouter.ai/api/v1` |
| CLI 参数 `-m` / 环境变量 / 配置文件 | `OPENROUTER_MODEL` / `model` | `anthropic/claude-haiku-4.5` |
| 硬编码 | 最大循环迭代次数 | `50` |

### 会话持久化

对话历史自动保存到 `~/.oy-ai-agent/sessions/<项目>/<uuidv7>.json`。每个会话文件包含完整的 `ChatMessage` 数组，序列化为 JSON。使用 UUID v7 以获得时间排序的文件名。

### 扩展点

- **新 AI 提供者**：实现 `AiProvider` trait（在 `oy-ai` 中），并在 CLI 中注册。
- **新工具**：实现 `Tool` trait（在 `oy-agent` 中），通过 `ToolRegistry::register()` 注册。
- **TUI**：`oy-tui` crate 已作为工作区成员就绪，依赖 `oy-agent` 以获得 `Orchestrator` 循环。

## 测试

```
cargo test --workspace
```

| 测试套件 | 位置 | 测试内容 |
| :--- | :--- | :--- |
| oy-ai 单元测试 | `oy-ai/src/domain/chat_message.rs` | ChatMessage 创建、序列化、ToolCall 线路格式 |
| oy-agent 单元测试 | `oy-agent/src/domain/agent.rs` | Agent 消息管理、ToolRegistry 操作 |
| oy-agent 工具测试 | `oy-agent/src/infrastructure/tools/` | Read 错误处理、Write 成功、Bash 黑名单 |
| oy-agent 集成测试 | `oy-agent/tests/integration_test.rs` | 使用 MockProvider 的 Orchestrator 循环（直接响应、工具调用、最大迭代次数） |

## 许可证

MIT

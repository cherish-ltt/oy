/// Sub-agent system types for CommanderAgent orchestration.
use std::{fmt, str::FromStr};

/// Types of sub-agents that CommanderAgent can create.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SubAgentType {
    Planner,
    Worker,
    Reviewer,
    GitHelper,
}

impl SubAgentType {
    pub fn max_rounds(&self) -> u32 {
        match self {
            Self::Planner => 50,
            Self::Worker => 100,
            Self::Reviewer => 75,
            Self::GitHelper => 15,
        }
    }

    pub fn system_prompt(&self) -> &'static str {
        match self {
            Self::Planner => PLANNER_SYSTEM_PROMPT,
            Self::Worker => WORKER_SYSTEM_PROMPT,
            Self::Reviewer => REVIEWER_SYSTEM_PROMPT,
            Self::GitHelper => GIT_HELPER_SYSTEM_PROMPT,
        }
    }
}

impl FromStr for SubAgentType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "planner" => Ok(Self::Planner),
            "worker" => Ok(Self::Worker),
            "reviewer" => Ok(Self::Reviewer),
            "git_helper" | "githelper" | "git" => Ok(Self::GitHelper),
            _ => Err(format!("无效的子代理类型: '{}'", s)),
        }
    }
}

impl fmt::Display for SubAgentType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Planner => write!(f, "Planner"),
            Self::Worker => write!(f, "Worker"),
            Self::Reviewer => write!(f, "Reviewer"),
            Self::GitHelper => write!(f, "GitHelper"),
        }
    }
}

/// Output from a sub-agent execution.
#[derive(Debug, Clone)]
pub struct SubAgentOutput {
    pub agent_type: SubAgentType,
    pub success: bool,
    pub summary: String,
    pub rounds_used: u32,
    pub error: Option<String>,
}

/// Status of a running sub-agent, used for UI progress reporting.
#[derive(Debug, Clone)]
pub enum SubAgentStatus {
    Pending,
    Running { round: u32, max_rounds: u32 },
    Completed(SubAgentOutput),
    Failed(String),
}

// ── System Prompts ─────────────────────────────────────────────

const PLANNER_SYSTEM_PROMPT: &str = r#"
你是一名 Planner（计划制定者），是 OY 子代理系统中的一部分。

## 角色定位
- 你只负责**制定计划**，不写代码，不执行文件操作。
- 你的输出是一份可执行的开发/测试计划。

## 约束
- 计划必须具体、可执行，包含文件路径和修改要点。
- 计划需要持久化到项目目录的 `.oy-agent-output/plans/{uuid-v7}-简短描述<10字.md`。
- 使用 Write 工具来创建计划文件。

## 工作方式
1. 阅读相关源码，理解当前结构和需求。
2. 制定详细的实施步骤。
3. 将计划写入 `.oy-agent-output/plans/` 目录。

## 最终输出模板
```
简述计划: <一句话描述>
plan文件名: .oy-agent-output/plans/<文件名>
```
"#;

const WORKER_SYSTEM_PROMPT: &str = r#"
你是一名 Worker（执行者），是 OY 子代理系统中的一部分。

## 角色定位
- 你严格按照 Planner 制定的计划实施，产出代码/结果。
- 你的职责是执行，而不是重新设计。

## 约束
- 生成的代码必须完整、可编译/可运行，禁止使用 TODO 占位符。
- 严格遵循计划中的文件路径和修改范围。

## 最终输出模板
```
完成了哪个计划文件: .oy-agent-output/plans/<文件名>
修改了什么:
- <文件1>: <修改摘要>
- <文件2>: <修改摘要>
简短总结: <实施心得>
```
"#;

const REVIEWER_SYSTEM_PROMPT: &str = r#"
你是一名 Reviewer（代码审查者），是 OY 子代理系统中的一部分。

## 角色定位
- 你审计 Worker 的产出，检查代码质量、正确性和完整性。
- 输出通过/不通过及改进建议。

## 约束
- 由 CommanderAgent 控制重试，你只需给出评审意见。
- 问题按优先级排序：严重 > 中度 > 轻度。
- 存在中度及中度以上问题，则review不通过。

## 最终输出模板
存在的问题/完美:
严重:
  - <问题描述>
中度:
  - <问题描述>
轻度:
  - <问题描述>
通过: 是/否
```
"#;

const GIT_HELPER_SYSTEM_PROMPT: &str = r#"
你是一名 GitHelper（Git 助手），是 OY 子代理系统中的一部分。

## 角色定位
- 你负责整理当前 git diff，给出有意义的 commit message，并提交 commit。

## 约束
- 使用 Bash 工具执行 `git add`、`git commit` 等操作。
- commit message 应清晰描述改动内容和原因，且符合标准 message 语句。

## 最终输出模板
```
提交的message: <有意义的 commit message>
```
"#;

/// CommanderAgent system prompt — the top-level orchestrator.
pub const COMMANDER_SYSTEM_PROMPT: &str = r#"
你是 CommanderAgent（指挥官代理），是 OY 系统的统筹规划指挥角色。

## 核心原则
- 你**不直接执行**文件操作、代码编写或 git 命令。
- 你的职责是：意图识别 → 任务拆分 → 调度子代理 → 结果汇总。
- 如对用户意图有哪怕 1% 的疑问，必须向用户确认，不要猜测。

## 工作流程
1. 将用户意图拆分为若干大小适中的子问题 (sub-issue 1/2/3...)
2. 对每个 sub-issue:
   a. 调用 `create_sub_agent(agent_type="planner", task="...")` 制定计划
   b. 调用 `create_sub_agent(agent_type="worker", task="...", context="plan文件路径")` 实施计划
   c. 调用 `create_sub_agent(agent_type="reviewer", task="...")` 审查产出
   d. 如审查通过，调用 `create_sub_agent(agent_type="git_helper", task="...")` 提交 commit
   e. 如审查不通过，重复步骤 b-d（最多重试 10 次）
3. 完成所有 sub-issue 后汇总结果

## 子问题拆分原则
- 独立、清晰、大小适中
- 不依赖或需要显式串行处理
- 具备可验证的完成标准

## 工具使用
你只有一个工具：`create_sub_agent`。通过 `agent_type` 参数指定子代理类型：
- `planner`：制定计划
- `worker`：执行计划
- `reviewer`：审查产出
- `git_helper`：提交 git commit

绝不要尝试调用其他工具（如 Read/Write/Bash），这些工具由子代理使用。

## 最终输出模板
```
任务目标: <原始用户需求>
任务总结:
  - sub-issue1: 完成/失败
  - sub-issue2: 完成/失败
  - ...
git记录:
  - <commit message 1>
  - <commit message 2>
  - ...
```
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sub_agent_type_from_str() {
        assert_eq!(SubAgentType::from_str("planner"), Ok(SubAgentType::Planner));
        assert_eq!(SubAgentType::from_str("Planner"), Ok(SubAgentType::Planner));
        assert_eq!(SubAgentType::from_str("worker"), Ok(SubAgentType::Worker));
        assert_eq!(
            SubAgentType::from_str("reviewer"),
            Ok(SubAgentType::Reviewer)
        );
        assert_eq!(
            SubAgentType::from_str("git_helper"),
            Ok(SubAgentType::GitHelper)
        );
        assert_eq!(
            SubAgentType::from_str("githelper"),
            Ok(SubAgentType::GitHelper)
        );
        assert_eq!(SubAgentType::from_str("git"), Ok(SubAgentType::GitHelper));
        assert_eq!(
            SubAgentType::from_str("unknown"),
            Err(String::from("无效的子代理类型: 'unknown'"))
        );
    }

    #[test]
    fn test_sub_agent_type_display() {
        assert_eq!(SubAgentType::Planner.to_string(), "Planner");
        assert_eq!(SubAgentType::Worker.to_string(), "Worker");
        assert_eq!(SubAgentType::Reviewer.to_string(), "Reviewer");
        assert_eq!(SubAgentType::GitHelper.to_string(), "GitHelper");
    }

    #[test]
    fn test_max_rounds() {
        assert_eq!(SubAgentType::Planner.max_rounds(), 50);
        assert_eq!(SubAgentType::Worker.max_rounds(), 100);
        assert_eq!(SubAgentType::Reviewer.max_rounds(), 75);
        assert_eq!(SubAgentType::GitHelper.max_rounds(), 15);
    }

    #[test]
    fn test_system_prompts_not_empty() {
        assert!(!SubAgentType::Planner.system_prompt().is_empty());
        assert!(!SubAgentType::Worker.system_prompt().is_empty());
        assert!(!SubAgentType::Reviewer.system_prompt().is_empty());
        assert!(!SubAgentType::GitHelper.system_prompt().is_empty());
    }

    #[test]
    fn test_commander_prompt_not_empty() {
        assert!(!COMMANDER_SYSTEM_PROMPT.is_empty());
    }

    #[test]
    fn test_sub_agent_output_display() {
        let output = SubAgentOutput {
            agent_type: SubAgentType::Planner,
            success: true,
            summary: "Plan created".into(),
            rounds_used: 3,
            error: None,
        };
        assert!(output.success);
        assert_eq!(output.summary, "Plan created");
    }
}

# Terra 文档重写检查清单

本文件是重写 PRD/TDD/Workflow 前的完整需求收集。
每一项都必须在文档中有对应的 requirement 和 test case。

---

## 来源 1: 对话中 Oliver 提出的设计要求

### 核心定位
- [ ] Terra 是 Agent OS，不是框架，不是 chatbot
- [ ] 受 Caging Engineering 启发
- [ ] CE 同时治理开发过程和产品行为（双重角色）
- [ ] 聚焦 coding/软件工程场景
- [ ] Token efficiency 是一等公民
- [ ] 从"一"开始（不从"零"），Agent 出厂有第一性原理

### Intelligence Core
- [ ] Universal Layer: Reasoning + Behavioral + Self-Diagnostic (L0 只读)
- [ ] Domain Modules: 可插拔、可组合 (coding/, algorithm/)
- [ ] 每个 module 注册 Perspectives, Rules, Verifiers
- [ ] 多视角观察能力（"横看成岭侧成峰"）
- [ ] IC 规则必须在运行时主动执行，不是死数据

### Language Compilation Layer
- [ ] JSP 式编译：用户自然语言 → Spec IR
- [ ] 5 个 pass: Lexical, Semantic Resolution, Terminology Normalization, Completeness Check, Ambiguity Detection
- [ ] 编译一次节省后续大量 token
- [ ] Spec IR 是所有下游操作的统一输入

### Multi-Agent 架构
- [ ] Main Agent: 最强推理模型，接收所有上下文，制定计划
- [ ] Worker Sub-agent: 执行具体编码/修改
- [ ] Explorer Sub-agent: 搜索/研究/信息收集
- [ ] Cage Sub-agent: 独立监控者，不介入业务，只检查规范性
- [ ] 任务闭环: Sub-agent → 总结 → Main Agent 审计
- [ ] 同一模型内多角色调用
- [ ] Provider 适配: 多模型→分配, 单模型→参数区分
- [ ] Main Agent 推理深度应当是最高的
- [ ] Agent 间通信用 MessagePack (不用自然语言)

### Memory Database
- [ ] 不是 MD 档案库，是完整数据库系统
- [ ] 三层存储: L0 Rust内嵌 + moka缓冲 + SQLite持久
- [ ] 作用域维度: S0 Global → S1 Project → S2 Session → S3 Task
- [ ] 语义维度: L0-L6 (7层)
- [ ] Rust 式安全: Ownership, Borrowing, Lifetime, No null, No dangling, GC
- [ ] 指针权重用 Attention 机制（动态计算，不静态标注）
- [ ] Multi-Head Attention: 每个 Domain Module 贡献观察视角
- [ ] MessagePack 存储 (binary source of truth)
- [ ] Markdown 只是展示层（通过程序渲染）
- [ ] 每项目独立 SQLite，互不干扰
- [ ] moka cache 缓冲层
- [ ] Bloom Filter 快速排除不存在的 key
- [ ] Lazy Loading（指针只存 metadata）
- [ ] Embedding 增量更新
- [ ] Copy-on-Write（晋升时延迟复制）
- [ ] SQLite WAL 模式

### Command Protocol
- [ ] ~15 条固定命令覆盖所有交互
- [ ] Agent 不写数据库代码
- [ ] Command Executor: 审计 → Cage 检查 → 执行 → 记录
- [ ] 可审计 + 可回放
- [ ] 用户和 Agent 用同一套命令协议

### Caging Layer (8 cage)
- [ ] Specification Cage → 任务规格完整性
- [ ] Knowledge Cage → 知识权威排序
- [ ] Memory Cage → 写入权限和晋升门控
- [ ] Action Cage → Sandbox + Tool Protocol
- [ ] Verification Cage → 外部验证
- [ ] Convergence Cage → 成功后停止探索
- [ ] Parametric Memory Cage → L6 晋升需人工审核
- [ ] Meta-Cage → 宪法和 cage 规则不可修改

### Sandbox
- [ ] 项目目录内：可读写删除新增
- [ ] 项目目录外：只读+可执行
- [ ] .terra/ 内 constitution 等：只读
- [ ] SandboxGuard 必须在 Orchestrator 中实际调用

### Sleep Consolidation
- [ ] 会话结束/空闲时触发
- [ ] 清理无用临时数据
- [ ] 具体操作抽象为通用原则
- [ ] 提名晋升候选 → 用户确认

### 压缩保护
- [ ] 70% 阈值触发主动外化
- [ ] accepted paths / plan / 关键决策写入 Memory Database
- [ ] 更新指针表后才允许压缩

### Artifact 版本化存储
- [ ] Plan/Spec/Test 存为版本化 MessagePack 文件
- [ ] 记忆系统只存指针引用
- [ ] 类似 Git 的 object store + refs

### Config 系统
- [ ] ~/.terra/config.toml (global)
- [ ] project/.terra/config.toml (project override)
- [ ] 环境变量 override (最高优先级)
- [ ] terra doctor 环境检测
- [ ] terra config show/set

### Skill 系统
- [ ] MANIFEST.yml 注册 capabilities
- [ ] MessagePack over stdio 通信
- [ ] Superpower Skill: 内建工程规范
- [ ] OfficeX 未来作为第一个外部 Skill

### 通信协议
- [ ] 人 ↔ Agent: 自然语言
- [ ] Agent ↔ Agent: MessagePack 二进制
- [ ] Agent ↔ Tool: MessagePack/JSON Schema
- [ ] 展示层: 程序渲染为 Markdown

### Audit Trail
- [ ] 顺序编号
- [ ] 持久化到 SQLite
- [ ] Markdown 渲染
- [ ] 并发安全
- [ ] 可回放

---

## 来源 2: 审计中发现的真实问题

- [ ] Orchestrator 绕过分层架构（直调 provider）
- [ ] Intelligence Core 是死数据（规则未执行）
- [ ] Cage 不是独立 Sub-agent（只是同步调用）
- [ ] Multi-Agent 是假的（只有单 agent 调 provider）
- [ ] MemSearch 命令无后端
- [ ] Artifact Store 表存在但代码未使用
- [ ] 晋升链缺乏层级验证（可向下晋升）
- [ ] Verification Cage 不接入真实验证器
- [ ] Command Executor 完全不存在
- [ ] 测试多为字段检查而非行为验证
- [ ] 没有端到端走完整 Caging 链的测试
- [ ] 没有并发 Memory 写入测试
- [ ] 没有 session 生命周期测试

---

## 来源 3: 待解决的技术问题

- [ ] Streaming: LLM 调用是 streaming 的，需要 stream processing
- [ ] 向量索引一致性: SQLite 和 FAISS 同步问题
- [ ] Cache 写入顺序: 关键写入 write-through
- [ ] Sleep Agent 误总结: 用户确认是最后防线
- [ ] Embedding 模型迁移: 换模型需重新 embedding
- [ ] 多 Agent 并发写入: 乐观锁/MVCC
- [ ] 本地 Embedding 加载延迟: daemon 模式
- [ ] Skill 版本兼容性: manifest_version 字段

---

## 来源 4: Oliver 的开发原则

- [ ] Goal-driven (PRD/TDD/BDD 先行)
- [ ] 优雅代码，不堆砌
- [ ] 只改目标内容，不随手改别的
- [ ] 严禁偷懒、推理不够深层、目标偏移
- [ ] TDD 和架构方案双重确保任务实现
- [ ] 不做 MVP 简化，做最完整方案
- [ ] 分层和解耦

# rg-tree-sitter-vim：精准语义跳转工具提案

## 1. 问题背景

### 1.1 现有痛点
- **gtags**：增量更新在十万行以上项目中越来越慢，且对现代语言支持不足
- **ctags**：只能跳定义，无法区分定义/调用/引用；跨文件搜索时结果嘈杂
- **LSP (clangd 等)**：精准但依赖 `compile_commands.json`，大项目初始化慢、内存占用高
- **fff**（Neovim）：仅支持 Neovim；definition classifier 是字节级启发式（前缀匹配），对多行定义、宏包裹等场景会误判；无 AST 语义分析

### 1.2 空白地带
Vim 生态（及更广范围）缺乏一个**轻量级、零配置、基于 AST 语义**的跨文件跳转工具：
- 比 ctags/gtags **准**（AST 语义，非文本匹配）
- 比 LSP **轻**（无需编译数据库，秒开）
- 比 fff **准**（tree-sitter 语义确认 vs 启发式前缀匹配）
- 比 ast-grep **易用**（symbol-driven 自动跳转 vs pattern-driven 手动写规则）

---

## 2. 方案核心

**内嵌搜索（grep-searcher）+ tree-sitter 语义仲裁，以 CLI / Daemon 双模式提供零依赖服务**

### 2.1 架构演进：从 Vim 插件到通用 CLI

早期思路是做一个完整的 Vim 插件（Rust 二进制 + VimScript 异步 job + quickfix 管理），但工程量大且 Vim 端占 70% 工作量。

**更务实的方向**：先做一个独立的命令行工具，通过 fzf/LeaderF/Telescope 等现有框架集成到 Vim。CLI 本身也可在 Shell 中独立使用。

```
┌─────────────────────────────────────────┐
│           rg-tree-sitter CLI            │
│  ┌──────────┐     ┌──────────────────┐  │
│  │  内嵌搜索  │────►│ tree-sitter 过滤  │  │
│  └──────────┘     └──────────────────┘  │
│                            │            │
│              ┌─────────────┘            │
│              ▼                          │
│  ┌──────────────────────────────────┐   │
│  │  LRU AST 缓存（daemon 模式下有效） │   │
│  └──────────────────────────────────┘   │
└─────────────────────────────────────────┘
              │
    ┌─────────┴─────────┐
    ▼                   ▼
fzf/Telescope      Vim quickfix
(交互式选择)        (cexpr 直接填充)
```

### 2.2 双模式设计

#### Mode A：CLI 模式（单次执行）

```bash
# 基础用法
rg-tree-sitter define process_data --lang cpp --dir /project

# 从 stdin 接收外部搜索结果（兼容 rg 输出，便于管道组合）
rg -n --column 'process_data' src/ | rg-tree-sitter filter --lang cpp
```

- 优点：零依赖、随处可用、无需守护进程管理
- 缺点：每次调用冷启动，AST 需重新 parse（延迟 100ms~1s 视候选数而定）

#### Mode B：Daemon 模式（推荐用于日常编辑）

```bash
# 启动守护进程（可放在 ~/.bashrc 或 Vim autocmd 中自动启动）
rg-tree-sitter daemon --socket /tmp/rg-ts.sock --dir /project

# CLI 通过 Unix socket 与守护进程通信
rg-tree-sitter define process_data --socket /tmp/rg-ts.sock
```

- 守护进程内维护 LRU AST 缓存和文件 watcher
- CLI 只做轻量 IPC，查询延迟降到毫秒级
- 多个编辑器实例可共享同一个 daemon

### 2.3 核心工作流程（以 daemon 为例）

```
用户按 gd
    │
    ▼
Vim 调用 rg-tree-sitter CLI（带 --socket 参数）
    │
    ▼
CLI 通过 Unix socket 向 daemon 发送请求
    │
    ▼
Daemon 内部：
    ├─ 检查 LRU AST 缓存（文件 mtime 未变 → 直接用内存 Tree）
    ├─ mtime 变了或无缓存 → tree-sitter parse() → 存缓存
    └─ 在 AST 中根据搜索器给的精确行列定位节点
        │
        ▼
        向上追溯祖先链（3~5 层）：
        ├─ function_definition / class_definition → "定义"
        ├─ call_expression → "调用"
        └─ comment / string → 丢弃
    │
    ▼
返回定义节点的起始位置（修正多行定义）
    │
    ▼
JSON 响应 → Vim 填入 quickfix 或传给 fzf
```

### 2.2 关键设计：位置修正

```cpp
// 场景：多行函数定义
void              // ← 定义真正从这里开始（第 1 行）
process_data(const Input& input)  // ← 搜索器匹配到这里（第 2 行）
{
    ...
}
```

tree-sitter AST：
```
function_definition [0:0 - 4:0]      // 节点范围从第 1 行开始
  type: primitive_type [0:0 - 0:4]   // void
  declarator: function_declarator [1:0 - 1:30]
    name: identifier [1:0 - 1:12]    // process_data（搜索器匹配点）
```

**策略**：从搜索器匹配点向上追溯到 `function_definition`，返回该节点的起始行列。

---

## 3. 技术架构

### 3.1 项目结构

```
rg-tree-sitter/
├── Cargo.toml                          # Rust workspace
├── crates/
│   ├── rg-tree-sitter-core/            # 核心库（内嵌搜索 + tree-sitter 过滤）
│   │   ├── src/
│   │   │   ├── lib.rs                  # 库入口
│   │   │   ├── searcher.rs             # 内嵌搜索器（grep-searcher + ignore）
│   │   │   ├── filter.rs               # tree-sitter 语义过滤核心
│   │   │   ├── cache.rs                # LRU AST 缓存（内存）
│   │   │   └── languages.rs            # 语言映射 + 定义节点类型表
│   │   └── Cargo.toml
│   │
│   ├── rg-tree-sitter-cli/             # CLI / Daemon 二进制
│   │   ├── src/
│   │   │   ├── main.rs                 # CLI 入口（clap）
│   │   │   ├── cli.rs                  # 单次执行模式
│   │   │   └── daemon.rs               # 守护进程 + Unix socket
│   │   └── Cargo.toml
│   │
│   └── rg-tree-sitter-vim/             # 可选：轻量 Vim 辅助脚本
│       ├── plugin/rg_tree_sitter.vim   # 命令注册 + daemon 生命周期管理
│       └── autoload/rg_tree_sitter.vim # daemon 启动/停止/状态查询
│
└── README.md
```

### 3.2 核心依赖

| 组件 | 用途 |
|------|------|
| `grep-searcher` + `grep-regex` | 内嵌跨文件正则搜索（ripgrep 核心库，.gitignore 过滤、并行、SIMD） |
| `ignore` | .gitignore / .ignore 解析与目录遍历 |
| `tree-sitter` crate | AST 解析核心（C 库的 Rust 绑定） |
| `tree-sitter-c` / `cpp` / `python` / `rust` / `go` / `javascript` / `typescript` / `java` | 各语言 parser（静态链接进二进制） |
| `lru` crate | AST 缓存淘汰策略 |
| `serde` + `serde_json` | IPC / CLI 输出序列化 |
| `clap` | CLI 参数解析 |
| `tokio` / `uds` (可选) | Daemon 异步 IPC（Unix domain socket） |

### 3.3 CLI 接口设计

```bash
# ========== 单次执行模式 ==========

# 定义跳转（最常用）
rg-tree-sitter define <symbol> --lang <lang> [--dir <dir>]

# 引用查找
rg-tree-sitter refs <symbol> --lang <lang> [--dir <dir>]

# 从 stdin 过滤外部搜索结果（兼容 rg 输出格式）
rg -n --column 'foo' | rg-tree-sitter filter --lang cpp

# 输出格式控制
rg-tree-sitter define foo --lang cpp --format json   # JSON 数组
rg-tree-sitter define foo --lang cpp --format plain  # file:line:col:text（默认）

# ========== Daemon 模式 ==========

# 启动守护进程
rg-tree-sitter daemon --socket /tmp/rg-ts.sock --dir /project [--watch]

# 向 daemon 查询（CLI 自动检测到 socket 就走 IPC）
rg-tree-sitter define foo --socket /tmp/rg-ts.sock --lang cpp

# 管理命令
rg-tree-sitter daemon-status --socket /tmp/rg-ts.sock
rg-tree-sitter daemon-stop --socket /tmp/rg-ts.sock
```

### 3.4 Vim 集成：利用现有框架

**不做原生 Vim 异步 job + quickfix 管理**，而是输出标准格式，由现有工具消费。

#### fzf.vim 集成（推荐）

```vim
" 定义跳转，带预览
command! -nargs=1 -complete=tag RgTsDefine
  \ call fzf#run(fzf#wrap({
  \   'source': 'rg-tree-sitter define ' . shellescape(<q-args>) . ' --lang ' . &filetype,
  \   'sink': function('s:rgts_jump'),
  \   'options': [
  \     '--preview', 'bat --color=always --highlight-line {2} {1}',
  \     '--delimiter', ':'
  \   ]
  \ }))

function! s:rgts_jump(item)
  let parts = split(a:item, ':')
  execute 'e ' . parts[0]
  call cursor(str2nr(parts[1]), str2nr(parts[2]))
endfunction

" 光标下单词跳转
nnoremap gd :RgTsDefine <C-R><C-W><CR>
```

#### 纯 Vim（无 fzf）

```vim
" 直接用 cexpr 填充 quickfix
command! RgTsDefine
  \ cexpr system('rg-tree-sitter define ' . expand('<cword>') . ' --lang ' . &filetype)
```

#### Daemon 生命周期（Vim 辅助脚本）

```vim
" 进入项目时自动启动 daemon
autocmd VimEnter * call rg_tree_sitter#ensure_daemon()

" 退出 Vim 时可选停止 daemon
autocmd VimLeavePre * call rg_tree_sitter#stop_daemon()
```

---

## 4. 与现有工具的对比

| 工具 | 精准度 | 速度 | 重量 | Vim 支持 | 核心机制 |
|------|--------|------|------|----------|----------|
| **ctags** | 低（文本匹配） | 快 | 轻 | ✅ | 正则提取符号 |
| **gtags** | 低（文本匹配） | 增量慢 | 中 | ✅ | 全局数据库 |
| **LSP (clangd)** | 极高（语义） | 初始化慢 | 重 | ✅ | AST + 类型系统 |
| **fff** | 中（启发式前缀） | 极快 | 中（常驻内存） | ❌（仅 Neovim） | 内存索引 + 前缀扫描 |
| **ast-grep** | 高（AST 模式） | 快 | 中 | ❌ | 结构化模式匹配 |
| **本方案 (CLI)** | 高（AST 语义） | 中等（冷启动） | 极轻 | ✅ | 内嵌搜索 + tree-sitter 按需过滤 |
| **本方案 (Daemon)** | 高（AST 语义） | 极快 | 中（常驻缓存） | ✅ | 内存 LRU + 文件 watcher |

### 4.1 本方案的独特定位

> **"给不想配 LSP 的 Vim user 一个秒开、零配置、AST 级精准的跳转方案"**

- 无需 `compile_commands.json`
- 无需启动 language server
- 无需全局预索引（按需解析）
- CLI 模式零常驻内存；Daemon 模式常驻但远轻于 LSP
- 既可嵌入编辑器，也可在 Shell 中独立使用

---

## 5. 待验证的风险与局限

### 5.1 tree-sitter 的语义局限
- tree-sitter 是**语法级**（syntactic）解析，非**语义级**（semantic）
- 无法做类型推断、重载决议、跨文件命名空间解析
- 因此**无法完全替代 LSP**，定位是 LSP 的轻量备选

### 5.2 首次查询延迟
- 搜索返回 50+ 个候选文件时，需 parse 50 个文件
- 无缓存时首次可能秒级，日常走缓存后毫秒级
- 需实际测试验证可接受性

### 5.3 Parser 版本兼容性
- tree-sitter parser 版本需与源码语法版本大致匹配
- 极端新语法可能 parser 不认识，导致 AST 不准确

### 5.4 跨文件引用查找的局限
- 可区分"调用"和"定义"，但"引用"包含变量赋值、导入等多种语义
- 可能需要更细粒度的节点类型映射表

---

## 6. MVP 建议范围

若决定投入，分两阶段推进：

### Phase 1：纯 CLI（一个周末验证核心）

目标：验证内嵌搜索 + tree-sitter 过滤的延迟和精准度是否可接受。

1. **只支持 2~3 种语言**：C/C++、Python
2. **只做定义跳转**：`rg-tree-sitter define <symbol> --lang <lang>`
3. **只做 plain 输出**：`file:line:col:text` 格式
4. **无缓存**：每次调用重新 parse，接受冷启动延迟
5. **Vim 集成**：一行 fzf 映射或 `cexpr` 即可

**验证标准**：
- 十万行 C++ 项目日常 `gd` 延迟 < 1s（CLI 冷启动可接受）
- 多行定义（`void\nfoo()`）跳转位置正确
- 宏包裹定义（`API_EXPORT\nvoid foo()`）不被误判为调用

### Phase 2：Daemon 模式（第二个周末加缓存）

目标：解决 CLI 冷启动延迟，达到日常毫秒级响应。

1. **Unix socket 守护进程**：`rg-tree-sitter daemon`
2. **LRU AST 缓存**：内存缓存最近解析的文件
3. **文件 watcher**（可选）：基于 `notify` crate 的增量更新
4. **Vim 辅助脚本**：daemon 自动启动/停止/状态查询

**验证标准**：
- 重复查询同一符号延迟 < 100ms
- 修改文件后再次查询，只重新 parse 变更文件

---

## 7. 参考资料

- [tree-sitter Rust crate](https://docs.rs/tree-sitter/latest/tree_sitter/)
- [tree-sitter-c / tree-sitter-cpp crates](https://crates.io/)
- [fff (dmtrKovalenko)](https://github.com/dmtrKovalenko/fff) — 内存索引 + 启发式定义识别的参考
- [ast-grep](https://ast-grep.github.io/) — AST 结构化搜索的参考
- [vim-gutentags](https://github.com/ludovicchabant/vim-gutentags) — Vim 异步 tags 管理的参考

---

## 8. 决策记录

| 时间 | 结论 |
|------|------|
| 2026-06-03 | 论证完成：Vim 生态确实存在内嵌搜索 + tree-sitter 语义跳转的空白；方案具备一定 side project 价值。尚未进入实现阶段。 |
| 2026-06-03 | 技术选型：放弃调用外部 rg，改为内嵌 `grep-searcher` + `ignore` crate，实现真正的零依赖单 binary。 |
| 2026-06-03 | 架构演进：从"完整 Vim 插件"降级为"通用 CLI 工具 + 可选 Daemon"，利用 fzf/LeaderF 等现有框架做 Vim 集成，大幅降低工程复杂度。 |

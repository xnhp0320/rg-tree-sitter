# rg-tree-sitter 下一阶段开发计划（Phase Next）

> 按收益/代码量比排序，依次推进。

---

## P0：初筛机制（parse 前过滤）

**现状痛点：**
`rg` 文本搜索返回 50 个候选文件 → 全部 parse AST → 发现 40 个是调用/注释/字符串 → 只返回 10 个定义。大量 parse 是浪费的。

**目标：**
在完整 AST parse 之前加入轻量级初筛，减少需要 parse 的文件数。

**实现路径：**

| 层级 | 策略 | 是否需 parse | 预期过滤率 |
|------|------|-------------|-----------|
| **L1 行级启发式** | 匹配行首是 `//`、`/*`、`"`、`'`，或行内明显在字符串/注释上下文 | 否 | ~30-40% |
| **L2 上下文关键字** | 匹配行前后几行无 `def`/`void`/`fn`/`class` 等定义关键字 | 否 | ~20-30% |
| **L3 tree-sitter query** | 用 query 只 match 定义节点，跳过完整遍历 | 是（但更快）| ~10-20% |
| **L4 完整 parse** | 现有逻辑，AST 语义分类 | 是 | — |

**第一步（L1）：**
在 `search_symbol` 返回 `TextMatch` 后、进入 `AstFilter` 之前，加入 `PreFilter`：

```rust
pub fn quick_filter(matches: &[TextMatch]) -> Vec<TextMatch> {
    matches.iter().filter(|m| {
        let line = m.text.trim();
        !line.starts_with("//") && !line.starts_with("/*") &&
        !line.starts_with("*") &&  // block comment continuation
        !is_inside_string_literal(line, m.column)
    }).cloned().collect()
}
```

**验证标准：**
- 十万行项目搜索常见 symbol，候选文件减少 ≥ 30%
- 不丢失任何真正的定义（假阴性 = 0）

---

## P1：tree-sitter parse 多线程并行化

**现状痛点：**
`AstFilter::filter()` 串行遍历候选文件，逐个 `get_or_parse()`。50 个文件 = 50 次串行 IO + CPU。

**目标：**
对未缓存的文件并行 parse，利用多核降低延迟。

**技术要点：**
- `tree_sitter::Parser` 不是 `Sync`（内部有 C 可变状态），但创建开销极小
- 每个线程独立创建 `Parser`，parse 完成后统一插入共享 `AstCache`
- `tree_sitter::Tree` 是引用计数结构，`Clone` 廉价

**实现路径：**

```rust
use rayon::prelude::*;

fn parse_batch(
    paths: &[PathBuf],
    lang: LanguageId,
    cache: &AstCache,
) {
    paths.par_iter().for_each(|path| {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang.to_tree_sitter_language()).unwrap();
        let source = std::fs::read_to_string(path).unwrap();
        let tree = parser.parse(&source, None).unwrap();
        cache.insert(path.clone(), tree, source);
    });
}
```

**集成点：**
在 `AstFilter::filter()` 开始之前，先提取「未缓存的 unique 文件路径列表」，并行 parse，再进入串行的语义分类循环。

**验证标准：**
- 50 个未缓存文件，8 核机器上 parse 耗时从 ~500ms 降到 ~100ms
- 线程安全无 data race

---

## P2：inotify 更高效的监控策略

**现状痛点：**
`notify` crate 监听整个项目目录的 `RecursiveMode::Recursive`，任何文件变动都触发 `mark_dirty`，包括 `.git/`、`target/`、日志、swap 文件等无关文件。

**目标：**
减少无效 watcher 事件，降低事件处理开销；大项目避免触及 `fs.inotify.max_user_watches` 上限。

**实现路径：**

1. **扩展名过滤：** watcher 回调中只处理源码扩展名
   ```rust
   fn is_source_file(path: &Path) -> bool {
       matches!(path.extension().and_then(|s| s.to_str()),
           Some("c" | "h" | "cpp" | "cc" | "py" | ...))
   }
   ```

2. **忽略目录过滤：** 跳过 `.git/`、`target/`、`node_modules/`、`build/` 等

3. **事件 debounce：** 文件保存时编辑器可能触发多次连续事件，批量处理
   ```rust
   // 收到事件后不立即 mark_dirty，而是放入队列
   // 100ms 内没有新事件时统一 flush
   ```

4. **大项目降级：** 当目录深度/文件数超过阈值时，改为不递归 watch，或按需 watch

**验证标准：**
- 无关文件变更不再触发 `mark_dirty`
- 10万行项目不触及 inotify watches 上限

---

## P3：其他已记录改进（后续择机）

- `daemon-status` 返回实际状态（缓存大小、监听目录等）
- `mark_dirty` 闭环：dirty set 应主动从缓存中驱逐，而非仅靠 mtime 检测
- CLI `--socket` 自动 fallback 本地模式
- `filter` IPC 重构：单独设计 `FilterRequest` 结构
- 扩展语言：Rust、Go、TypeScript、Java
- Vim plugin 目录：`plugin/` + `autoload/` 开箱即用脚本

---

## 推进顺序

```
P0 初筛机制 → P1 并行 parse → P2 inotify 优化 → P3 其他
```

P0 和 P1 是**延迟削减**的核心，P2 是**daemon 体验**优化，P3 是**功能完善**。

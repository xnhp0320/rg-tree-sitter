# rg-tree-sitter 后续改进点

## 已记录（来自实现者）

1. `daemon-status` 应返回实际状态（缓存大小、监听目录等）
2. `mark_dirty` 机制未闭环——dirty set 被写入但从未读取
3. CLI 带 `--socket` 时若 daemon 未启动应自动 fallback 到本地模式
4. `filter` IPC 实现 hack（stdin 全文塞进 `symbol` 字段）
5. 搜索缺少 AST 预过滤——全文本搜索后逐个 parse 所有候选文件
6. 扩展更多语言（Rust/Go/TypeScript/Java）
7. 补全 Vim plugin 目录（`plugin/` + `autoload/`）

## 已记录（来自用户）

### 8. 更高效的 inotify 监控策略

当前使用 `notify` crate（Linux 底层即 inotify），但策略较粗放：
- 监听整个项目目录的 **RecursiveMode::Recursive**
- 任何文件变更都触发 `mark_dirty`
- 未区分「相关文件扩展名」 vs 「无关文件」（如 `.git/`、build artifacts）

**改进方向：**
- 按语言扩展名过滤 watcher 事件，无关文件变更直接忽略
- 对于超大规模项目，考虑限制 watcher 的递归深度或仅 watch 源码子目录
- 批量处理 inotify 事件（debounce），避免保存文件时连续多次触发重新 parse

### 9. tree-sitter parse 多线程并行化

当前 `AstFilter::filter()` 是串行遍历 matches：
```rust
for m in matches {
    let parsed = self.get_or_parse(path); // 串行 IO + CPU
    ...
}
```

当搜索返回 50+ 个候选文件时，逐个串行 parse 是主要瓶颈。

**改进方向：**
- 将候选文件按「未缓存」分组，使用 `rayon` 或 `tokio::task::spawn_blocking` 并行 parse
- parse 是纯 CPU 密集型任务（tree-sitter C 库），适合多核并行
- 需注意 `tree_sitter::Parser` 不是 `Send`（部分底层实现限制），可能需要每个线程独立创建 Parser 实例

### 10. 初筛机制（parse 前过滤）

当前流程：文本搜索命中 → **全部文件 parse AST** → 语义过滤。

问题：很多文本匹配是明显无效的（注释、字符串、日志行、二进制文件），但仍被完整 parse。

**改进方向：**

| 层级 | 策略 | 精度 | 开销 |
|------|------|------|------|
| L1 行级启发式 | 匹配行包含 `//`、`/*`、`"`、`'` 前缀则跳过 | 低 | 极低 |
| L2 文本上下文 | 用简单正则判断是否在 `def`、`class`、`fn` 附近 | 中 | 低 |
| L3 tree-sitter query | 先用 query 扫描文件是否包含目标 symbol 的定义节点 | 高 | 中（比全 parse 快）|
| L4 完整 parse | 对 L1-L3 通过的文件做完整 AST parse | 极高 | 高 |

**推荐实现：**
- 默认开启 L1（几乎零开销）
- 对于 define 查询，L3 可用 tree-sitter query 如：
  ```scheme
  (function_definition
    declarator: (function_declarator
      name: (identifier) @name)) @def
  ```
  只 capture 定义节点的 name，比完整 parse 快得多

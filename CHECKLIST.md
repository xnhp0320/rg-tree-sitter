# rg-tree-sitter 改进点检查清单

> 对照 IMPROVEMENTS.md、phrase.md 和 PERFORMANCE.md 逐项核对。

---

## ✅ 已完成

### 核心性能优化

| # | 改进点 | 实现位置 | 验证方式 |
|---|--------|---------|---------|
| 1 | **P0 L1: 行级启发式初筛** — 过滤注释/字符串中的匹配 | `prefilter.rs` | 4 个单元测试通过 |
| 2 | **P0 L2: 上下文关键字置信度排序** — `split_by_confidence()` | `prefilter.rs` | `has_definition_keyword()` 测试通过 |
| 3 | **P1: 并行 tree-sitter parse** — `rayon` 并行 parse 未缓存文件 | `filter.rs:parse_batch()` | `test_parallel_parse_batch` 通过 |
| 4 | **P2: inotify 扩展名过滤** — watcher 只处理源码扩展名 | `daemon.rs:should_ignore_watcher_event()` | 手动验证 |
| 5 | **P2: inotify 目录过滤** — 跳过 `.git/`、`target/` 等 | `daemon.rs:should_ignore_watcher_event()` | 手动验证 |
| 6 | **mark_dirty 闭环** — dirty 时主动从缓存驱逐 | `engine.rs:mark_dirty()` + `cache.rs:remove()` | `test_cache_mtime_check` 覆盖 |
| 7 | **搜索 AST 预过滤** — `quick_filter()` 在 parse 前过滤 | `lib.rs`、`engine.rs` | 集成在 `find_definitions` 链路中 |
| 8 | **B: CLI `--socket` fallback** — daemon 未启动时自动回退本地模式 | `cli.rs` | 手动验证（非存在 socket） |
| 9 | **A: `daemon-status` 真实状态** — 返回缓存大小、watcher 状态 | `daemon.rs` | 手动验证 |
| 10 | **F: watcher debounce** — 100ms 批量 flush | `daemon.rs:run_watcher()` | 代码审查 |
| 11 | **AstCache LRU 性能修复** — 用 `lru` crate 替代手写 O(n) | `cache.rs` | kernel 测试 warm 4.5x 加速 |
| 12 | **Daemon 默认缓存 2048** — 从 128 提升 | `daemon.rs` | kernel 测试验证 |

### 基础功能

| # | 改进点 | 状态 |
|---|--------|------|
| 13 | **MVP CLI 模式** — define/refs/filter | ✅ 已完成 |
| 14 | **MVP Daemon 模式** — Unix socket + LRU 缓存 | ✅ 已完成 |
| 15 | **22 个单元测试** — languages/searcher/filter/cache/prefilter | ✅ 已完成 |
| 16 | **C/C++/Python 语言支持** | ✅ 已完成 |
| 17 | **多行定义修正** — `function_definition` 起始行回溯 | ✅ 已完成 |
| 18 | **调用/定义语义分类** — 祖先链遍历 | ✅ 已完成 |
| 19 | **Linux kernel 性能测试** — PERFORMANCE.md | ✅ 已完成 |

---

## ❌ 未完成

| # | 改进点 | 影响 | 优先级 |
|---|--------|------|--------|
| C | **`filter` IPC 重构** | 当前 stdin 全文塞进 `symbol` 字段，应设计 `FilterRequest` 结构 | 低 |
| D | **扩展更多语言** — Rust、Go、TypeScript、Java | 只需在 `languages.rs` 加映射和节点类型表 | 低 |
| E | **Vim plugin 目录** — `plugin/` + `autoload/` 开箱即用 | 文档已有示例代码，需落盘到实际文件 | 低 |
| G | **大项目 watcher 降级** — 当 `fs.inotify.max_user_watches` 不足时自动降级 | kernel 源码下 daemon + watch 可能崩溃 | 中 |
| H | **L3 tree-sitter query 初筛** — parse 前用 query 跳过无定义的文件 | 对 `printk`/`memcpy` 等高频 symbol 可跳过 90% 文件 | 高 |
| I | **限制搜索范围** — define 优先搜 .h 和核心目录 | 减少 `search_symbol` 遍历文件数 | 中 |

---

## 性能瓶颈记录（见 PERFORMANCE.md）

| 瓶颈 | 耗时 | 占比 | 解决方案 |
|------|------|------|---------|
| `search_symbol`（遍历 58K 文件） | ~350ms | 20% | 限制搜索范围（I） |
| `parse_batch` cold（parse 1,226 文件） | ~1,050ms | **60%** | L3 query 初筛（H）+ 增大缓存 |
| `parse_batch` warm（LRU 命中） | ~36ms | 2% | 已优化（`lru` crate O(1)） |
| `filter` 循环（AST 分类） | ~0ms | <1% | 无需优化 |

---

## 总计

- **已完成：19 项**
- **未完成：6 项**

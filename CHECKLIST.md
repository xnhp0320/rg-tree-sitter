# rg-tree-sitter 改进点检查清单

> 对照 IMPROVEMENTS.md 与 phrase.md 逐项核对，标注完成状态。

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

### 基础功能

| # | 改进点 | 状态 |
|---|--------|------|
| 8 | **MVP CLI 模式** — define/refs/filter | ✅ 已完成 |
| 9 | **MVP Daemon 模式** — Unix socket + LRU 缓存 | ✅ 已完成 |
| 10 | **17 个单元测试** — languages/searcher/filter/cache | ✅ 已完成 |
| 11 | **5 个新增测试** — prefilter + parallel parse | ✅ 已完成 |
| 12 | **C/C++/Python 语言支持** | ✅ 已完成 |
| 13 | **多行定义修正** — `function_definition` 起始行回溯 | ✅ 已完成 |
| 14 | **调用/定义语义分类** — 祖先链遍历 | ✅ 已完成 |

---

## ❌ 未完成

| # | 改进点 | 影响 | 优先级 |
|---|--------|------|--------|
| A | **`daemon-status` 返回实际状态** | 目前返回空 `matches`，应返回缓存大小、监听目录、watcher 状态 | 中 |
| B | **CLI `--socket` 自动 fallback 本地模式** | daemon 未启动时直接报错 `Connection refused`，应静默 fallback 到 CLI | 高 |
| C | **`filter` IPC 重构** | 当前 stdin 全文塞进 `symbol` 字段，应设计 `FilterRequest` 结构 | 低 |
| D | **扩展更多语言** — Rust、Go、TypeScript、Java | 只需在 `languages.rs` 加映射和节点类型表 | 低 |
| E | **Vim plugin 目录** — `plugin/` + `autoload/` 开箱即用 | 文档已有示例代码，需落盘到实际文件 | 低 |
| F | **事件 debounce** — 文件保存时编辑器可能连续触发多次 watcher 事件 | 当前每事件都处理，应批量 flush | 中 |
| G | **大项目 watcher 降级** — 当 `fs.inotify.max_user_watches` 不足时自动降级 | 当前递归 watch 全目录，可能触及上限 | 低 |

---

## 总计

- **已完成：14 项**
- **未完成：7 项**

### 建议下一步

按影响/代码量比排序：
1. **B（fallback）** — 约 10 行代码，解决 daemon 未启动时的用户体验问题
2. **A（daemon-status）** — 约 20 行代码，提升 daemon 可观测性
3. **F（debounce）** — 约 30 行代码，减少文件保存时的重复 parse

# rg-tree-sitter 性能测试报告

> 测试目标：Linux kernel 6.8（1.5GB，~58,130 个 .c/.h 文件）
>
> 测试时间：2026-06-04
>
> 测试硬件：aarch64-unknown-linux-gnu，11GB RAM

---

## 测试方法

使用 `rg-tree-sitter` 的 `SearchEngine` 直接调用（等效于 Daemon 模式内部逻辑），
测量 `find_definitions` 和 `find_references` 的 cold/warm 耗时。

- **cold**：首次查询，AST 缓存为空
- **warm**：重复查询，AST 已缓存（cache capacity = 2048）
- **search**：仅 `grep-searcher` 文本搜索耗时（遍历 58K 文件）

---

## 核心发现

### 1. 缓存是关键加速器

| Symbol | 匹配文件数 | Cold | Warm | 加速比 |
|--------|-----------|------|------|--------|
| `socket` | 1,226 | 1.75s | **386ms** | **4.5x** |
| `sk_alloc` | 282 | 427ms | 347ms | 1.2x |
| `ip_rcv` | 42 | 408ms | 342ms | 1.2x |

**结论**：当缓存容量 ≥ 匹配的唯一文件数时，warm 查询显著快于 cold。

### 2. 超常见 symbol 是性能陷阱

| Symbol | 匹配文件数 | Cold | Warm | 问题 |
|--------|-----------|------|------|------|
| `printk` | 37,195 | 7.1s | 11.0s | **warm 更慢** |
| `memcpy` | 28,749 | 16.8s | 18.8s | **warm 更慢** |
| `spin_lock` | 43,510 | 15.0s | 17.5s | **warm 更慢** |
| `kmalloc` | 6,895 | 7.7s | 12.6s | **warm 更慢** |

**原因**：
- `printk`/`memcpy`/`spin_lock` 等宏/函数在内核中使用极其广泛
- 搜索返回数万条匹配，分布在数万个文件中
- 即使 `quick_filter` 过滤掉 45% 的注释/字符串匹配，剩余文件数仍远超 2048 缓存容量
- LRU 频繁淘汰 → 大量 miss → 重新 parse → warm 比 cold 还慢

### 3. 时间分解（以 `socket` 为例）

| 阶段 | 耗时 | 占比 |
|------|------|------|
| `search_symbol`（grep-searcher 遍历 58K 文件） | ~350ms | 20% |
| `quick_filter`（L1 行级过滤） | ~1ms | <1% |
| `parse_batch`（并行 parse 1,226 文件）cold | ~1,050ms | **60%** |
| `parse_batch` warm（全部命中） | ~36ms | 2% |
| `filter` 循环（AST 语义分类） | ~0ms | <1% |
| **Total cold** | **~1.7s** | |
| **Total warm** | **~387ms** | |

**瓶颈**：`search_symbol`（350ms）和 `parse_batch`（cold 时 1,050ms）是主要耗时。

### 4. `quick_filter` 初筛效果

| Symbol | 原始匹配 | 过滤后 | 过滤率 |
|--------|---------|--------|--------|
| `socket` | 15,371 | 8,447 | **45%** |
| `printk` | 38,059 | 37,195 | 2% |
| `kmalloc` | 7,555 | 6,895 | 9% |

L1 初筛对注释/字符串较多的 symbol 效果显著，但对纯代码中的高频 symbol 帮助有限。

---

## 详细数据

### define 查询

| Symbol | Search | Cold | Warm | Results | Files |
|--------|--------|------|------|---------|-------|
| `socket` | 15,371 | 1.75s | **386ms** | 3,785 | 8,447 |
| `sys_socket` | 41 | 347ms | 345ms | 15 | 34 |
| `sock_create` | 220 | 366ms | 358ms | 65 | 191 |
| `sk_alloc` | 300 | 427ms | 347ms | 70 | 282 |
| `tcp_v4_connect` | 6 | 341ms | 338ms | 3 | 5 |
| `ip_rcv` | 44 | 408ms | 342ms | 22 | 42 |
| `kmalloc` | 7,555 | 7.7s | 12.6s | 278 | 6,895 |
| `spin_lock` | 44,037 | 15.0s | 17.5s | 320 | 43,510 |
| `memcpy` | 29,460 | 16.8s | 18.8s | 765 | 28,749 |
| `printk` | 38,059 | 7.1s | 11.0s | 1,603 | 37,195 |

### refs 查询

refs 查询的时间与 warm define 接近，因为都共享相同的 AST 缓存。

| Symbol | Refs 耗时 |
|--------|----------|
| `socket` | 386ms |
| `printk` | 11.4s |
| `memcpy` | 18.9s |

---

## 优化建议（按优先级）

### P0: 增大默认缓存容量

当前 daemon 默认 2048，对 `socket` 够用，但对 `printk` 不够。

```
建议：8192 或 16384（11GB RAM 足够容纳）
```

### P1: L3 tree-sitter query 初筛

在 `parse_batch` 之前，先用 tree-sitter query 扫描文件是否包含目标 symbol 的**定义节点**。如果文件中只有调用/引用，直接跳过 parse。

```scheme
; C/C++ 定义 query
(function_definition
  declarator: (function_declarator
    name: (identifier) @name))
```

预期收益：对 `printk` 这类高频 symbol，可跳过 90% 只包含调用的文件。

### P2: 限制 search_symbol 的搜索范围

当前 `search_symbol` 遍历所有 58K 文件。可以：
- 对 define 查询，优先搜索头文件（.h）和核心源码目录
- 对 refs 查询，排除 `tools/`、`Documentation/` 等目录

### P3: CLI vs Daemon 对比

| 模式 | socket cold | socket warm |
|------|-------------|-------------|
| CLI | ~1.85s | ~1.85s（无缓存） |
| Daemon | ~2.0s | **~386ms**（缓存命中） |

Daemon 模式对重复查询的加速是核心优势。

---

## 已知问题

1. **Daemon 在大项目下启动 watcher 可能崩溃**：Linux kernel 目录数庞大，`notify` crate 的递归 watch 可能触及 `fs.inotify.max_user_watches` 上限。当前 workaround：不带 `--watch` 启动 daemon。
2. **超常见 symbol 的 warm 查询反而更慢**：当匹配文件数远超缓存容量时，LRU 淘汰导致大量 re-parse。需配合 L3 query 初筛解决。

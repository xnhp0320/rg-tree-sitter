# rg-tree-sitter

轻量级、零配置、基于 AST 语义的跨文件符号跳转工具。

> 给不想配 LSP 的 Vim user 一个秒开、零配置、AST 级精准的跳转方案。

## 特性

- **AST 语义过滤**：基于 tree-sitter 区分定义/调用/引用，而非纯文本匹配
- **多行定义修正**：自动将函数名修正到 `function_definition` 起始位置
- **双模式设计**：
  - CLI 模式：零依赖、单次执行
  - Daemon 模式：LRU AST 缓存 + 文件监听，重复查询毫秒级响应
- **零配置**：无需 `compile_commands.json`，无需 language server

## 安装

```bash
cargo build --release
# 二进制位于 target/release/rg-tree-sitter
```

## 用法

### CLI 模式

```bash
# 查找定义
rg-tree-sitter define process_data --lang cpp --dir ./src

# 查找引用
rg-tree-sitter refs process_data --lang cpp --dir ./src

# 从 rg 输出过滤（兼容管道）
rg -n --column 'process_data' src/ | rg-tree-sitter filter --lang cpp

# JSON 输出
rg-tree-sitter define process_data --lang cpp --format json
```

### Daemon 模式（推荐日常编辑）

```bash
# 启动守护进程
rg-tree-sitter daemon --socket /tmp/rg-ts.sock --dir /project --watch

# 通过 socket 查询（自动使用 AST 缓存）
rg-tree-sitter define process_data --socket /tmp/rg-ts.sock --lang cpp

# 停止守护进程
rg-tree-sitter daemon-stop --socket /tmp/rg-ts.sock
```

## Vim 集成

### fzf.vim（推荐）

```vim
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

nnoremap gd :RgTsDefine <C-R><C-W><CR>
```

### 纯 Vim（无 fzf）

```vim
command! RgTsDefine
  \ cexpr system('rg-tree-sitter define ' . expand('<cword>') . ' --lang ' . &filetype)
```

## 支持语言

| 语言 | 标识符 | 文件扩展名 |
|------|--------|-----------|
| C    | `c`    | `.c`, `.h` |
| C++  | `cpp`  | `.cpp`, `.cc`, `.cxx`, `.hpp`, `.hh`, `.hxx` |
| Python | `python` | `.py` |

## 架构

```
rg-tree-sitter/
├── Cargo.toml
└── crates/
    ├── rg-tree-sitter-core/   # 核心库（搜索 + tree-sitter 过滤 + 缓存）
    └── rg-tree-sitter-cli/    # CLI / Daemon 二进制
```

## 实验结果与结论

### Linux Kernel 6.8 测试（C 语言，58K 文件）

| 符号 | rg 原始匹配 | tree-sitter define | tree-sitter refs |
|------|------------|-------------------|-----------------|
| `socket` | ~1,226 文件 | 过滤后精准定位定义 ✅ | 空（全部被归为 Definition）❌ |
| `printk` | ~37K 文件 | 过滤后结果正确 ✅ | 空 ❌ |
| `tcp_v4_connect` | 3 个文件 | 正确定义 + 位置修正 ✅ | 空 ❌ |

**C 语言下 `refs` 完全失效**：`declaration` 节点被归为 `Definition`，而 C 代码中几乎所有标识符出现都在 `declaration` 下，导致 Reference 类为空。

### Boost 1.86 测试（C++，35K 文件）

| 符号 | rg 原始匹配 | tree-sitter define | tree-sitter refs |
|------|------------|-------------------|-----------------|
| `shared_ptr` | 11,393 行 | 返回变量声明（`boost::shared_ptr<T> x`），非类定义 ❌ | 返回 `typedef` / `template class`，非真正引用 ❌ |
| `make_shared` | 1,232 行 | 返回**函数调用**（`auto p = make_shared<T>()`）错标为 Definition ❌ | 仅 2 行，几乎为空 ❌ |

**核心问题**：C++ 中变量初始化 `auto x = foo()` 在 AST 中属于 `declaration` 节点，被错误归类为 `Definition`。`define` 返回调用，`refs` 几乎为空。

### 结论

**tree-sitter 基于节点类型的粗粒度分类（declaration = definition）对 C/C++ 不成立。**

在 C/C++ 中，变量的声明、初始化、调用、引用经常落在同一 AST 节点类型下。仅凭 `function_definition` / `declaration` / `call_expression` 等节点类型做语义区分，**结果与纯 rg 几乎无异，且性能更慢**（rg 123ms vs tree-sitter 460ms）。

**本项目暂停。** 后续若需继续，需引入更细粒度的 AST 分析（如区分 identifier 是声明者还是被使用者），或转向其他语言（Python、Rust 等声明/调用区分更清晰的语言）。

---

## 局限

- **C/C++ 语义分类失效**：`declaration` 节点同时覆盖变量声明、初始化、参数列表，无法区分定义/引用
- tree-sitter 是**语法级**解析，非**语义级**（无类型推断、重载决议）
- 无法完全替代 LSP，定位是 LSP 的轻量备选
- 首次冷启动需 parse 候选文件（延迟视文件数而定）
- 对**函数式/动态语言**（Python、JavaScript）可能效果更好，未充分验证

## License

MIT

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

## 验证结果

- **多行定义修正**：`void\nprocess_data()` 跳转至 `void` 所在行 ✅
- **宏包裹定义**：`API_EXPORT\nvoid foo()` 不被误判为调用 ✅
- **Daemon 缓存**：重复查询延迟 < 10ms ✅
- **文件监听**：修改文件后行号自动更新 ✅

## 局限

- tree-sitter 是**语法级**解析，非**语义级**（无类型推断、重载决议）
- 无法完全替代 LSP，定位是 LSP 的轻量备选
- 首次冷启动需 parse 候选文件（延迟视文件数而定）

## License

MIT

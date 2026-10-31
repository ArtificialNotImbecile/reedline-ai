# reedline-ai

一个基于 [Reedline](https://github.com/nushell/reedline) 的 Rust AI 命令行补全实验：保留原生历史/编辑/菜单，优先本地即时补全，再用异步 AI 做 fallback；支持保留光标后文本的 fill-in-the-middle（FIM）插入。

A small Rust playground for asynchronous suffix and cursor-aware infill completion. **Not a replacement shell or command-execution agent.** The standalone prototype and opt-in native-shell adapters are separate entry points.

> 当前是可运行的 MVP，不是 shell 替代品。按 Enter 只显示接受的文本，不会执行命令。默认使用确定性的离线 mock，不联网，也不需要 API key。

## Native shell integration (opt-in)

The standalone demo is not your shell. The native adapters instead keep the existing shell editor, history and completion, and call the shared Rust backend only through explicit keys. They insert accepted text into the current line and never press Enter or execute it. The host shell still executes normally if you subsequently press Enter.

Start with the [install and mock-first quickstart](docs/install.md), [Bash adapter](docs/bash-adapter.md) and [versioned stdin protocol](docs/adapter-protocol.md). This is explicit request → preview → accept, **not asynchronous inline ghost text**. Nothing installs itself into a profile; the default backend remains offline mock. The protocol always gates warnings or incomplete risk reviews regardless of the standalone risk setting. PowerShell, Nushell and CMD+Clink packages require explicit `--experimental` extraction. See [PowerShell](docs/powershell-adapter.md), [Nushell](docs/nushell-adapter.md), and [CMD+Clink](docs/cmd-adapter.md) for actual runtime coverage and limits; Windows/Clink is not claimed tested.

### Bash 体验与本地历史

构建后运行 `bash scripts/try-bash.sh` 即可进入临时 Bash 体验环境；显式加
`--deepseek` 使用环境中的 `DEEPSEEK_API_KEY`（或 `REEDLINE_AI_API_KEY`）和
`deepseek-flash`。默认仍是离线 mock，不修改 profile。Bash 需 4.3+；macOS 自带
3.2 不具备所需编辑接口，安装新版 Bash 后启动器会查找 Homebrew 路径。

重复 **Ctrl-x Ctrl-g** 完成请求、预览、插入；**Ctrl-x Ctrl-k** 取消 AI 并保留输入。
中文预览直接可读，原有 Tab、Enter、历史搜索不变。Bash、PowerShell、Nushell
优先在各自已加载的最近 200 条历史中做本地前缀/后缀匹配，命中不调用模型。
不进行首次全量上传或跨 Shell 猜测历史路径。设置和兼容范围见
[安装指南](docs/install.md)；Windows PowerShell 5.1 的兼容实现仍待 Windows 实测。

## Shell-aware workflows and editable local memory

The standalone prototype now supports explicit target-shell context, exact configured workflow phrases such as `run CCTV`, and previewable whole-line replacements. It is still **not a shell plugin** and never executes commands. Native Reedline history and keys remain first priority; Tab previews complex replacements, Enter accepts only.

```sh
cargo run --locked -- suggest --shell cmd --buffer 'ls'
cargo run --locked -- suggest --shell bash --config examples/project-memory.synthetic.json --buffer 'run CCTV'
cargo run --locked -- --shell bash --config examples/project-memory.synthetic.json
```

All fixture paths are synthetic. JSON project memory is user-editable, with explicit global/project/local/CLI-file layers, strict local-only settings and management commands. Nothing is auto-discovered or learned from real histories. Empty/whitespace input sends no AI request; the 180 ms debounce is unchanged. HTTP completions can include explicitly supplied shell metadata, but still return insertion-only text; deterministic local replacements are separate.

See [configuration, CLI, supported shell grammar and limitations](docs/project-memory.md), [local command-risk review](docs/risk-review.md), and the [truthful implementation roadmap](docs/ROADMAP.md). Explicit [mobile text interaction](docs/mobile-mode.md) is available with `--mode mobile`; native-shell adapter work is documented below.

### Local advisory command-risk review

`--risk-policy warn` is the default. The local, bounded shell-aware review shows advisory warnings for generated suggestions and configured workflow previews, without an extra acceptance step or model call. `--risk-policy confirm` requires Tab → F2 → Enter for a candidate with warnings **or an incomplete review**. Enter before F2 does not insert that candidate and closes the native menu; press Tab to reopen it. Editing, navigation, or cancellation invalidates acknowledgment. Whole/word AI hint acceptance cannot bypass this gate; native history hints, history menus, and search are unchanged.

```sh
cargo run --locked -- --shell bash --risk-policy confirm
cargo run --locked -- --shell bash --risk-policy warn --complete 'git st'
cargo run --locked -- suggest --shell bash --risk-policy confirm --buffer 'git st'
```

`off` hides UI warnings and removes the extra gate, but JSON still includes `risk_policy`, `risk_review` (`warnings`, `incomplete`), and `additional_acceptance_required`. Only the explicit CLI option controls policy: no environment variable, project-memory layer, or global policy file can change it. Missing shell context is explicitly incomplete. This is lexical advisory review, not a complete parser, execution permission system, or safety guarantee. See [coverage, limits, and protocol semantics](docs/risk-review.md).

## Mobile terminal mode

Use `reedline-ai --mode mobile --shell bash` on the server you already access through SSH. Type text + Enter, choose a numbered preview, then `accept` (and `confirm` if the risk policy requires it), or `cancel`. `edit TEXT`, `suggest`, `help` and `quit` need no Ctrl/Alt/function keys. `--mode desktop` remains the default. Both modes only accept text, never execute it. Mobile uses the selected server shell, never detects a phone from SSH, and does not set up SSH/Tailscale. See [mobile commands, limits and validation](docs/mobile-mode.md).

## 快速开始

需要 Rust 工具链（[官方安装说明](https://www.rust-lang.org/tools/install)）。仓库固定 Rust 1.99.1 / Reedline 0.52.1；首次构建会下载依赖。

```sh
git clone https://github.com/ArtificialNotImbecile/reedline-ai.git
cd reedline-ai
cargo run --locked
```

离线体验：

1. 输入 `git st`，暂停片刻，看到灰字 `atus`。按 Ctrl-F 接受，再按 Enter，显示 `Accepted (not executed): git status`。
2. 输入 `git  --short`（`git` 后两个空格），将光标放到第一个空格之后。按 Tab 打开补全菜单，Enter 插入 `status`；原有 ` --short` 不变。再次 Enter 仅打印结果。
3. 输入 `echo "hello"`，将光标移到最后的引号前，Tab → Enter 插入 ` world`。

按键：

- Ctrl-F：接受行尾灰字；不在行尾时沿用 Reedline 的移动行为。
- Tab：打开 / 切换补全菜单。候选到达不会自动写入。
- Enter：菜单打开时接受候选；`--risk-policy confirm` 下有风险提示或审查不完整的生成候选需先按 F2。未确认时 Enter 不插入并关闭菜单，Tab 可重新打开。否则提交文本用于显示，不执行。
- Esc：关闭菜单。Ctrl-C：清空当前输入。Ctrl-D（空行时）或 `:quit`：退出。
- Up/Down、Ctrl-P/N：原生历史导航；Ctrl-R：原生反向搜索；Alt-F / Alt-Right：接受灰字的下一个词，不是整个候选。
- Ctrl-Z / Ctrl-G：原生撤销/重做；Shift 选择、剪切/粘贴仍沿用 Reedline 默认 Emacs 行为。Ctrl-X 额外打开原生历史菜单。
- 默认保留本应用最近 10,000 条内存历史，退出即失；不会读取 bash/zsh/fish 历史。含控制字符、换行或单次超过 8 KiB 的粘贴会整段拒绝，不会先显示再过滤。

## 保留原生体验，AI 只做增量增强

- 灰字首先调用 Reedline 自带的 `DefaultHinter`，直接命中的本地历史立即显示，不等待 debounce 或网络。没有可用本地匹配时才请求 AI。
- Tab 使用原生 `ColumnarMenu`；本地历史的前缀/后缀匹配先显示，最多 20 个。中间匹配检查最近 500 个前缀候选。AI 不重排、不覆盖已经显示的本地候选，选中项不会因网络返回而跳动。
- Ctrl-R、历史导航、原生历史菜单、Emacs 编辑、词级接受、撤销/重做和选择保留。安全适配只过滤危险输入并将已显示候选绑定到一次性输入快照；普通提示接受仍走原生 hint 事件，由 Reedline 自己检查光标、选择和菜单状态。confirm 策略下需要额外确认的 AI 候选转入菜单预览，不允许整条/词级提示绕过确认。
- 这是 Reedline 的编辑/历史组件，不是 Nushell：不会凭空获得 shell parser、命令执行、PATH 命令发现、文件路径/参数语义补全或 Nushell 的语言能力。可以在这里继续组合真正的本地 completer，当前本地补全来源是本应用历史。

### 本地历史与独立的上传开关

默认只保存在内存。需要跨会话保留时，显式指定一个专用于本应用的文件，不要指向真实 shell history：

```sh
cargo run --locked -- --history-file ./reedline-ai-history.txt
# 离线预览，文件只读，不创建/裁剪/改写，不调用模型：
cargo run --locked -- --history-file ./reedline-ai-history.txt \
  --history-preview 'git ' --history-exclude 'PRIVATE-PROJECT'
```

持久化采用原生 FileBackedHistory 的容量/同步语义，可能裁剪超过 10,000 条的旧记录；仅预览和 one-shot 读取不会触发这种写入。Unix 新文件及清空后重建文件以 0600 创建；已有文件权限不擅自修改，Windows 使用所在目录 ACL。历史文件要求普通文件，最大 128 MiB；只应使用可信、稳定、归自己所有的路径。原生后端退出时会自动同步，不提供抵御其他本地进程并发替换路径/符号链接的安全边界。不安全条目不会显示或发送。以空格开头的输入和 `:` 应用控制命令不入历史，但空格不是网络隐私开关：当前行仍可能发送。

只有另加 `--ai-history` 才会把筛选后的本应用历史交给已配置的模型；它与 `--backend http` 是独立开关。默认 HTTP 请求仍只有当前行，没有历史。`--history-exclude TEXT` 可重复，按大小写敏感子串排除历史，不改变本地历史，也不拦截当前行。交互输入 `:history-preview PREFIX` 可本地查看拟选历史；`:history-clear` 清空本地历史/持久化文件和 AI 缓存，已发送请求/供应商记录无法收回。预览包含原文，注意终端录屏与日志。

筛选只考察最近 500 条，混合相关与近期记录、精确去重，最多 50 条；单条超过 1 KiB 或含危险格式的记录不发送。默认历史预算 4096、整个请求预算 8192，`--request-budget 16384` 显式扩大总预算。**这些是保守 UTF-8 字节估算单位，不是实际 tokenizer 的 4k/8k/16k tokens。** 估算包含 JSON 转义、固定系统提示、256 协议预留和 128 输出-token 预留；当前行超预算拒绝，历史只选完整条目，不截断命令。通用供应商的 tokenizer/协议不同，所以这不是精确 token 上限保证；通常会比相同数字的实际 token 容量更保守。实际 tokens 以供应商 usage 或匹配 tokenizer 的实验结果为准。排除与字符检查都不是秘密检测器，开启上传前请审阅，无法保证历史里没有秘密。

无需交互终端也能验证：

```sh
cargo run --locked -- --complete 'git st'
# 核心字段：{"cursor":6,"insertion":"atus","result":"git status"}；另含风险审查元数据。
cargo run --locked -- --complete 'git  --short' --cursor 4
# 核心字段：{"cursor":4,"insertion":"status","result":"git status --short"}；另含风险审查元数据。
cargo run --locked -- --complete 'echo 世' --cursor 8
# 核心字段：{"cursor":8,"insertion":"界","result":"echo 世界"}；另含风险审查元数据。
```

`--cursor` 是 UTF-8 字节偏移，不是字符序号。越界或落在多字节字符内部会报错。

## 接入模型（明确选择后才联网）

支持 OpenAI-compatible **chat completions** 协议，endpoint 必须是完整路径。先配置你信任的本地或远端服务，再运行：

```sh
cargo run --locked -- --backend http \
  --endpoint http://127.0.0.1:8080/v1/chat/completions \
  --model YOUR_MODEL
```

远端必须使用 HTTPS。需要密钥时，通过环境变量 `REEDLINE_AI_API_KEY` 注入；也可用 `--api-key-env NAME` 指定另一个变量名。不要把密钥放进命令行参数、URL、源码或 issue。未设置密钥时不发送 Authorization，适用于无认证的本地服务。

默认忽略环境代理，只信任内置公共根证书。若你的云环境/企业网络要求代理，可明确添加 `--allow-env-proxy`，按需读取 `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY` / `NO_PROXY`（含小写形式）。若需要平台/企业 CA，再独立添加 `--trust-native-certs`，使用系统证书及 `SSL_CERT_FILE` / `SSL_CERT_DIR`；TLS 证书验证始终开启。代理 URL 和证书路径不会出现在错误报告里。只启用你信任的网络配置：代理可看到目标地址，受信任的 TLS 检查代理还可能看到正文和 Authorization。

该适配器把 `prefix` / `suffix` 作为独立 JSON 字段放进 user message，要求模型只返回中间插入文本。**这是 prompted infill，不等于模型原生 FIM token 支持**；不依赖供应商是否支持 `/completions` 的 `suffix` 参数。2026-10-23 对 DeepSeek 官方 `deepseek-flash` 做过 3 个合成样例烟测和 15 次合成上下文延迟测量，见 [验证记录](docs/VALIDATION.md) 与 [冷热连接/上下文延迟报告](docs/latency-benchmark.md)。这不代表其他模型/供应商或任意命令都已验证。此 MVP 不启动模型服务器。

### DeepSeek（显式 opt-in）

按 [DeepSeek 官方接口](https://api-docs.deepseek.com/api/create-chat-completion/) 配置：

```sh
cargo run --locked -- --backend http --provider deepseek \
  --endpoint https://api.deepseek.com/chat/completions \
  --model deepseek-flash
```

`--provider deepseek` 才会添加 `thinking: {"type":"disabled"}`，避免短补全消耗推理输出；默认 `generic` 不添加供应商专属字段，也不会仅凭模型名自动推断。网络模式仍须明确指定 endpoint/model，密钥仅从进程环境读取。`--report-usage` 可与 HTTP `--complete` 一起使用，在 JSON 中额外输出允许字段内的数字 token 用量，不输出原始响应。

如需一次性真实服务验收，先确认官方 [当前价格](https://api-docs.deepseek.com/zh-cn/quick_start/pricing/) 和费用授权，再将密钥安全注入本次进程的 `REEDLINE_AI_API_KEY`。不要把密钥写到仓库、命令行参数、终端历史或测试报告。

```sh
cargo build --locked --release
python3 scripts/deepseek_smoke.py --allow-billable-api --max-cost-cny YOUR_APPROVED_CAP
```

上述网络选项也可显式传给这个脚本：`--allow-env-proxy --trust-native-certs`。两项默认关闭，不会为了连通性自动降低 TLS 安全性。

这条命令会计费，**不属于本地 CI，也不会自动运行**。脚本固定官方 endpoint / `deepseek-flash`，只发 3 个合成 suffix/FIM/Unicode 样例；每个最多 128 输出 tokens、15 秒网络超时，没有重试。遇到失败立即停止，不显示响应正文。报告只包含转义后的样例补全文本、后缀保留检查、示例匹配情况、延迟、允许字段内的数字用量和费用上界估算，不执行任何生成文本。

费用估算使用 2026-10-23 官方峰值价格：未缓存输入 ¥2/百万 tokens、输出 ¥8/百万 tokens；不计入缓存/非高峰优惠。发送前为每次请求按整个 1,048,576-token 上下文加 128 输出预留 ¥2.098176，三次总预留 ¥6.294528，须小于参数指定的已授权上限。把 `YOUR_APPROVED_CAP` 替换成你为本次运行明确批准的人民币数字上限。这是基于所注明价格/模型限制的保守估算，不是供应商账单锁定；价格变化后须先更新脚本。每次调用脚本是一次新的付费运行，绝不能用循环自动重试。

```sh
cargo run --locked -- --help
# 可配置 --debounce-ms（默认 180）、--timeout-seconds（默认 8）、--no-color
```

## 隐私与安全边界

- 默认 mock 不发出网络请求。HTTP 模式会在你停止编辑约 180 ms 后自动发送**当前行的 prefix/suffix**，包括行内可能存在的敏感内容；请勿输入秘密。显式提供的 shell 类型、版本、命令/alias 信息也会加入请求。空白行不请求 AI。请求还包含固定系统提示、模型名及生成参数。
- 历史默认仅用于本地匹配；仅 `--ai-history` 允许筛选历史进入 prompt。不会自动采集工作目录、任意文件、剪贴板或环境变量；只读取显式选择的历史和项目配置文件。配置文件本身不上传，但接受工作流后展开的路径/命令成为当前输入，在明确启用的 HTTP 模式下可能随后发送。HTTP 模式读取指定 API key；显式网络选项可读取代理/CA 配置用于传输，不作为 prompt。
- 不记录请求正文、API key 或原始错误响应。失败信息经过脱敏。模型 API key 只用于该 endpoint 的 Authorization header，不保存到磁盘；这不是内存清零或防止本机管理员读取进程的保证。
- 禁止 HTTP 重定向和请求重试，禁止带凭据/query/fragment 的 endpoint URL。远端只允许 HTTPS，明文 HTTP 仅允许 loopback。代理和平台 CA 必须分别显式 opt-in。
- 当前行最多 8 KiB，AI 插入文本最多 1 KiB，响应体最多 64 KiB；模型输出最多 128 tokens。请求另受上面的保守总预算约束。本地历史提示不是模型输出，仍按安全的 8 KiB 单行边界处理。超限拒绝，不偷偷截断。终端控制字符、换行、部分不可见控制和 Markdown 代码围栏会被拒绝。
- 一个后台 worker：最多 1 个进行中的请求 + 1 个可替换等待项。没有自动重试。超时默认 8 秒；这不是总费用预算，持续使用远端服务仍可能计费。
- 缓存只在内存中，默认最多 32 项、60 秒有效；提交/取消时清空。过期结果不能覆盖新输入，也不会重新填入已清空的缓存。按输入顺序消费的一次性已显示候选，防止同批按键重复/过期接受灰字，同时保留“接受后继续输入”的顺序。已发出的请求无法收回；退出可能等待当前 HTTP 超时结束。
- **绝不执行命令**，但建议本身仍可能错误、重复后缀或具有破坏性。字符过滤和本地词法风险提示都不是完整的 shell 语义安全审查；无警告不代表安全，confirm 也不是执行权限层。复制到真正 shell 前请自行检查。
- 命令行参数、终端回显、shell 自身历史、崩溃转储及供应商日志不受本程序控制。不要把敏感内容放在 `--complete` 参数里。

## 架构

```text
Reedline buffer + UTF-8 cursor
           │
           ▼
      native history hint / local history menu
           │ no local match
           ▼
      explicit local workflow / shell-intent preview (Tab only)
           │ no local match
           ▼
      Context { prefix, suffix, optional history, optional shell }
           │
           ▼
   debounce / bounded cache / generation check
           │              one background worker
           └──────────────► MockBackend | HttpBackend
                                      │
                            output validation
                                      │
                             repaint + poll ready
                                      ▼
              end-of-line Hinter / FullBuffer completion menu
```

- `src/context.rs`：输入/输出限制、UTF-8 边界和无损插入。
- `src/engine.rs`：debounce、单 worker、队列合并、缓存、stale suppression、生命周期。
- `src/backend.rs`：离线 mock、HTTP 请求与脱敏协议错误。
- `src/editor.rs`：Reedline async completer / hinter / prompt，以及渲染前输入过滤和批量按键接受保护。
- `src/risk.rs` / `src/review_menu.rs`：有界本地风险提示、显式策略与菜单候选确认；不执行、不增加模型调用。
- `src/main.rs`：CLI、显式网络/历史上传选择、非执行 REPL、离线预览与 one-shot JSON 模式。
- `src/local_history.rs`：共享原生历史、安全过滤、可选持久化与只读预览。
- `src/history_context.rs`：显式历史筛选、去重/排除和可复现的保守预算。
- `src/shell.rs` / `src/structured_edit.rs`：显式 shell 元信息、保守渲染和完整 buffer 绑定的替换。
- `src/project_memory.rs` / `src/workflow.rs` / `src/commands.rs`：本地 JSON 分层管理、精确工作流解析和离线结构化 CLI。

传统插入式 FIM 必须把整个 buffer 传给 completer。菜单使用 `InputMode::FullBuffer` 和 `OutputMode::SuggestedSpan`，候选替换区间为 `[cursor, cursor)`。因此 legacy AI/FIM 仅插入，不覆盖后缀。新的确定性本地工作流候选使用显式替换 span，并由完整原始 buffer + 光标校验阻止过期接受。Reedline 目前把 hinter 灰字画在缓冲区末尾，故中间补全用菜单展示，不伪装成中间 ghost text。

## 开发与验证

默认在本机运行 CI，不消耗 GitHub Actions 额度。Linux / macOS 安装 Bash、Python 3 和固定 Rust 工具链后，一条命令执行格式检查、lint、debug/release 测试与构建、headless 示例和 AI/native 两套 PTY 回归（各跑 debug/release）：

```sh
bash scripts/ci-local.sh
```

脚本可从任意工作目录调用；遇到失败立即停止，不会触发 GitHub Actions 或请求远端模型。首次安装工具链/下载 Rust 依赖仍可能联网；HTTP 测试仅使用 loopback。它只验证当前操作系统，不模拟其他平台。

Windows 或需要逐项排查时，可在本机执行以下 Rust 检查（POSIX PTY 脚本不支持 Windows）：

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo test --locked --release --all-targets
cargo build --locked --release
cargo run --locked -- --complete "git st"
```

测试覆盖：原生历史/词级提示/导航/搜索/撤销、菜单与搜索模式切换、持久化/只读预览/清空、历史 opt-in HTTP payload、预算超限零连接、suffix/FIM、Unicode 光标、无效/超限输入、异步 debounce、缓存、过期结果、取消、候选接受边界、端到端本地 HTTP payload/auth、重定向拒绝、超时、脱敏错误与不安全输出。HTTP 测试只连接一次性 loopback 服务，不需要付费 API、密钥或模型。

GitHub Actions 仅保留手动 `workflow_dispatch`：push 和 pull request 不会自动运行。确有需要时，在仓库 Actions → CI (manual) → Run workflow 手动启动 Linux / macOS / Windows 检查；手动运行仍可能消耗额度。日常验证优先使用本地脚本。

交互式 terminal smoke test 与 headless 测试是不同层面；不要把单元测试通过等同于所有终端已兼容。

## 当前限制与后续方向

- 不是 bash/zsh/fish/Nushell 集成，也没有 shell parser、命令执行或执行权限层。
- 仅支持单行补全；没有 streaming、原生 FIM adapter、语义安全分类器或跨文件上下文。
- 网络请求无法主动中止，只能丢弃过期结果并等待超时；没有自动限流恢复或供应商费用预算。
- 后续可评估原生 FIM 后端、可取消异步传输、真实终端跨平台回归，以及 opt-in shell 集成。先保持“只建议、不执行”的边界。

## 许可

MIT，见 [LICENSE](LICENSE)。依赖归各自作者所有，见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。本项目不隶属于 Nushell 或 OpenAI。

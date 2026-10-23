# reedline-ai

一个基于 [Reedline](https://github.com/nushell/reedline) 的 Rust AI 命令行补全实验：异步行尾补全 + 保留光标后文本的 fill-in-the-middle（FIM）插入。

A small Rust playground for asynchronous suffix and cursor-aware infill completion. **Not a shell, shell plugin, or command-execution agent.**

> 当前是可运行的 MVP，不是 shell 替代品。按 Enter 只显示接受的文本，不会执行命令。默认使用确定性的离线 mock，不联网，也不需要 API key。

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
- Enter：菜单打开时接受候选；否则提交文本用于显示，不执行。
- Esc：关闭菜单。Ctrl-C：清空当前输入。Ctrl-D（空行时）或 `:quit`：退出。
- 采用 Emacs 编辑模式。历史存储关闭。含控制字符、换行或单次超过 8 KiB 的粘贴会整段拒绝，不会先显示再过滤。

无需交互终端也能验证：

```sh
cargo run --locked -- --complete 'git st'
# {"cursor":6,"insertion":"atus","result":"git status"}
cargo run --locked -- --complete 'git  --short' --cursor 4
# {"cursor":4,"insertion":"status","result":"git status --short"}
cargo run --locked -- --complete 'echo 世' --cursor 8
# {"cursor":8,"insertion":"界","result":"echo 世界"}
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

该适配器把 `prefix` / `suffix` 作为独立 JSON 字段放进 user message，要求模型只返回中间插入文本。**这是 prompted infill，不等于模型原生 FIM token 支持**；不依赖供应商是否支持 `/completions` 的 `suffix` 参数。2026-10-23 对 DeepSeek 官方 `deepseek-flash` 做过 3 个合成样例的有限真实 API 验证，见 [验证记录](docs/VALIDATION.md)。这不代表其他模型/供应商或任意命令都已验证。此 MVP 不启动模型服务器。

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

- 默认 mock 不发出网络请求。HTTP 模式会在你停止编辑约 180 ms 后自动发送**当前行的 prefix/suffix**，包括行内可能存在的敏感内容；请勿输入秘密。请求还包含固定系统提示、模型名及生成参数。
- 不将历史、工作目录、文件、剪贴板或环境变量加入模型上下文。默认仅主动读取指定 API key；显式网络选项还会读取代理/CA 配置用于传输和本地信任验证，不作为 prompt。Reedline 提供给 hinter 的 history/cwd 参数被忽略。
- 不记录请求正文、API key 或原始错误响应。失败信息经过脱敏。模型 API key 只用于该 endpoint 的 Authorization header，不保存到磁盘；这不是内存清零或防止本机管理员读取进程的保证。
- 禁止 HTTP 重定向和请求重试，禁止带凭据/query/fragment 的 endpoint URL。远端只允许 HTTPS，明文 HTTP 仅允许 loopback。代理和平台 CA 必须分别显式 opt-in。
- 每次最多 8 KiB 输入，1 KiB 插入文本，64 KiB 响应体；模型请求最多 128 tokens。超限拒绝，不偷偷截断。终端控制字符、换行、部分不可见控制和 Markdown 代码围栏会被拒绝。
- 一个后台 worker：最多 1 个进行中的请求 + 1 个可替换等待项。没有自动重试。超时默认 8 秒；这不是总费用预算，持续使用远端服务仍可能计费。
- 缓存只在内存中，默认最多 32 项、60 秒有效；提交/取消时清空。过期结果不能覆盖新输入，也不会重新填入已清空的缓存。按输入顺序消费的一次性已显示候选，防止同批按键重复/过期接受灰字，同时保留“接受后继续输入”的顺序。已发出的请求无法收回；退出可能等待当前 HTTP 超时结束。
- **绝不执行命令**，但建议本身仍可能错误、重复后缀或具有破坏性。字符过滤不是 shell 语义安全审查。复制到真正 shell 前请自行检查。
- 命令行参数、终端回显、shell 自身历史、崩溃转储及供应商日志不受本程序控制。不要把敏感内容放在 `--complete` 参数里。

## 架构

```text
Reedline buffer + UTF-8 cursor
           │
           ▼
      validated Context { prefix, suffix }
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
- `src/main.rs`：CLI、显式网络选择、非执行 REPL 与 one-shot JSON 模式。

真正的 FIM 必须把整个 buffer 传给 completer。菜单使用 `InputMode::FullBuffer` 和 `OutputMode::SuggestedSpan`，候选替换区间为 `[cursor, cursor)`。因此仅插入，不覆盖后缀。Reedline 目前把 hinter 灰字画在缓冲区末尾，故中间补全用菜单展示，不伪装成中间 ghost text。

## 开发与验证

默认在本机运行 CI，不消耗 GitHub Actions 额度。Linux / macOS 安装 Bash、Python 3 和固定 Rust 工具链后，一条命令执行格式检查、lint、debug/release 测试与构建、headless 示例和两套 PTY 回归：

```sh
bash scripts/ci-local.sh
```

脚本可从任意工作目录调用；遇到失败立即停止，不会触发 GitHub Actions 或请求远端模型。首次安装工具链/下载 Rust 依赖仍可能联网；HTTP 测试仅使用 loopback。它只验证当前操作系统，不模拟其他平台。

Windows 或需要逐项排查时，可在本机执行以下 Rust 检查（POSIX PTY 脚本不支持 Windows）：

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo test --locked --release
cargo build --locked --release
cargo run --locked -- --complete "git st"
```

测试覆盖：suffix/FIM、Unicode 光标、无效/超限输入、异步 debounce、缓存、过期结果、取消、候选接受边界、端到端本地 HTTP payload/auth、重定向拒绝、超时、脱敏错误与不安全输出。HTTP 测试只连接一次性 loopback 服务，不需要付费 API、密钥或模型。

GitHub Actions 仅保留手动 `workflow_dispatch`：push 和 pull request 不会自动运行。确有需要时，在仓库 Actions → CI (manual) → Run workflow 手动启动 Linux / macOS / Windows 检查；手动运行仍可能消耗额度。日常验证优先使用本地脚本。

交互式 terminal smoke test 与 headless 测试是不同层面；不要把单元测试通过等同于所有终端已兼容。

## 当前限制与后续方向

- 不是 bash/zsh/fish/Nushell 集成，也没有 shell parser、命令执行或执行权限层。
- 仅支持单行补全；没有 streaming、原生 FIM adapter、语义安全分类器或跨文件上下文。
- 网络请求无法主动中止，只能丢弃过期结果并等待超时；没有自动限流恢复或供应商费用预算。
- 后续可评估原生 FIM 后端、可取消异步传输、真实终端跨平台回归，以及 opt-in shell 集成。先保持“只建议、不执行”的边界。

## 许可

MIT，见 [LICENSE](LICENSE)。依赖归各自作者所有，见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。本项目不隶属于 Nushell 或 OpenAI。

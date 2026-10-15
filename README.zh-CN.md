# gbtui

**从 Grok Build 抽取的通用 Rust 终端 UI 运行时与组件库。**

[English](README.md) | **简体中文**

[快速开始](#5-分钟上手) · [主题](#主题系统) · [文档](#文档) · [贡献](CONTRIBUTING.md)

> 当前版本：0.1.0，早期开发阶段，API 尚未承诺稳定。需要 Rust 1.94.0（仓库已固定工具链）。

从 Grok Build TUI 抽取的 **终端 UI 运行时 + 组件库**。站在 **ratatui 0.29 + crossterm** 上，把已经打磨过的键鼠、悬停、按需动画和主题做成可复用层。

这不是 Grok，也不依赖任何 `xai-grok-*` crate。`sources/grok-build/` 只是只读参考，实现禁止改那棵树。

## 功能

- 即时模式绘制，统一键鼠事件、同帧 hover、遮挡和按需 tick。
- List、Picker、Choice、Form、Prompt、TextArea、弹窗与通用内容查看器。
- 语义主题、终端默认背景、自动明暗、低色适配及热切换。
- 可选 Markdown、代码/diff、原生链接与图片支持。

本项目不是 Grok Build 产品复刻，也不提供 LLM、工具执行或会话持久化。

## 5 分钟上手

通过固定版本的 Git tag 引用（需要 Rust 1.94 或更新版本）。本版本通过 GitHub 分发，尚未发布到 crates.io。

`Cargo.toml`：

```toml
[dependencies]
gbtui = { git = "https://github.com/m2eat/gbtui.git", tag = "v0.1.0" }
```

```rust
use gbtui::{key, ratatui};
use gbtui::{App, AppEvent, Button, FrameCtx, HitId, Outcome, Pointer, ThemeKind, run_blocking};

const OK: HitId = HitId::named("ok");

struct Hello;

impl App for Hello {
    type Msg = ();

    fn handle(&mut self, ev: AppEvent<()>) -> Outcome {
        match ev {
            AppEvent::Hit(id, Pointer::Down { .. }) if id == OK => Outcome::Quit,
            AppEvent::Key(k) if key!(Esc).matches(&k) || key!('q').matches(&k) => Outcome::Quit,
            AppEvent::Hit(_, Pointer::Move { .. }) | AppEvent::Miss(Pointer::Move { .. }) => {
                Outcome::Changed
            }
            _ => Outcome::Unchanged,
        }
    }

    fn draw(&mut self, f: &mut ratatui::Frame, cx: &mut FrameCtx) {
        Button::new("OK").id(OK).focused(true).render(f.area(), f.buffer_mut(), cx);
    }
}

fn main() -> std::io::Result<()> {
    run_blocking(Hello, ThemeKind::Auto)
}
```

`gbtui` 同时导出匹配版本的 `ratatui` 和 `crossterm`；上例只需声明 `gbtui` 依赖。

不要在应用里 `match MouseEventKind`。点击走 `AppEvent::Hit`，hover 由 `FrameCtx` 在同一帧算完。

```bash
cargo run -p studio     # 工作台：变高折行列表、过滤/滚动条、hover / 遮挡 / 按需 tick
cargo run -p settings   # 富 Choice/Form、固定操作区、自动明暗与颜色降级
cargo run -p chat       # inline Prompt：补全、历史、粘贴折叠、附件 metadata
cargo run -p picker     # 富行/分组、fuzzy/regex/substring；Enter 打印选择到 stderr
cargo run -p agent      # 会话阅读；Ctrl+O 打开通用 Markdown/代码查看器
cargo run -p gbtui --example media --features images  # 图片、原生链接、移动/遮挡/清理
cargo run -p gbtui --example themes --features theme-config  # 主题、状态、透明背景、对比度
```

`Link` 和富 Markdown 链接会按终端能力自动输出 OSC 8，不会主动打开浏览器。
图片组件默认不启用；需要时为 `gbtui` 开启 `images` feature。图片由宿主提供字节，
不在框架内读文件或下载；不支持动态图像协议的终端显示 `alt` 文本。
`media` 示例中 `i` 显隐图片、`m` 移动、`o` 遮挡、`q` 退出。

交互入口：

- `chat`：输入 `h` 查看带详情列、滚动条的补全；Tab/Enter 接受而不提交；↑/↓ 可视浏览历史，Ctrl+R smart-case 模糊搜索，Esc 恢复含附件的原草稿。长粘贴双击展开、Ctrl+Z 撤销；Ctrl+G 插入示例附件。
- `picker`：F2 切换匹配方式，F3 切换 Filter/Search；查询为空时 ←/→ 折叠/展开详情；点击组标题折叠，`n/N` 在 Search 模式导航命中。无结果时 Enter 不选择任何项。
- `agent`：Ctrl+F 搜索，`n/N` 定位命中并按需展开隐藏详情；`[]` 跳用户 turn，`{}` 跳 assistant response；Ctrl+E 折叠最新 thinking，Ctrl+Shift+E 切换全部详情；`/` 打开富命令面板。
- `agent` 的 Ctrl+O：通用查看器；`m/c` 切换 Markdown/代码，`r` 切换原文/渲染，`w` 切换折行，`/` 搜索、`n/N` 跳命中，`v` 与 Shift+方向键/Shift-click 选择范围。Ctrl+Shift+C 复制选中行，Ctrl+Alt+C/R/M 分别复制原文/渲染文本/元数据，代码复制不含行号栏。
- `settings`：`c` 打开富选项，导航到 `other` 后 Enter 进入编辑；`j/k`、空格、Unicode 都是文字，Enter 提交、Esc 丢弃编辑。Form 页演示宿主校验、多选和固定 Apply/Reset。
- `themes`：`1/2/3/4` 切换 Night/Day/Auto/外部主题，`b` 切换背景策略，`r` 重新加载，`i` 编辑输入，`j/k` 滚动语义角色；窄窗口用 `s` 切换预览状态。可追加 TOML 文件路径，格式见 `crates/gbtui/examples/aurora.toml`。
- `settings` 的主题保存/下拉确认、`studio` 的主题选择现在实际热切换；`agent` 用 F4 切换 Day/Night，同时刷新 Markdown、代码和 diff。
- `ThemeKind::{Night, Day, Auto}` 自动适配终端色深；只有 `Auto` 自动选择明暗。ANSI16 保留 chrome 状态区分，`NO_COLOR` 清除颜色。显式 `Theme` 不被探测或降级覆盖；宿主可自行调用 `Theme::adapted`。终端事实通过 `AppEvent::TerminalInfo` 投递。

附件、补全候选和历史条目由宿主提供；框架不扫描文件、不加载附件、不执行命令，也不持久化历史。

## 主题系统

`Theme` 是轻量调色板；推荐用 `ThemeDefinition` 定义策略和覆盖，由运行时解析为
`ResolvedTheme`。组件通过 `cx.style(ThemeRole, ThemeState)` 取样式，帧内查表不分配。

```rust
use gbtui::{BackgroundPolicy, ThemeColor, ThemeDefinition, ThemeKind};
use gbtui::ratatui::style::Color;

let theme = ThemeDefinition::from(ThemeKind::Auto)
    .with_background(BackgroundPolicy::TerminalDefault)
    .with_color(ThemeColor::Accent, Color::Rgb(74, 158, 170));
```

| 背景策略 | 行为 |
|---|---|
| `Solid` | 使用主题背景 |
| `TerminalDefault` | 画布、输入、代码和滚动条轨道使用终端默认背景；浮层和交互状态保留背景 |
| `Transparent` | 再将普通浮层、按钮背景交给终端；仍保留悬停/选择区分 |

“透明”是 `Color::Reset`，不是 alpha 混色；窗口不透明度、毛玻璃由终端控制。
前景仍保留主题颜色；显式样式覆盖也可以重新指定背景。

- `ThemeRegistry` 管理命名主题，内置 Night/Day/Auto；`with_color` 和 `with_style` 设置应用覆盖。
  优先级：语义默认 → 角色 Normal 覆盖 → 当前状态覆盖 → 组件局部 `Style::patch`。
- 状态优先级为 Disabled → Invalid → Selected → Focused → Hovered → Normal；
  无颜色时仍用下划线、粗体、反显和 dim 区分交互。
- `Auto` 使用环境覆盖 → OSC11 → OS → Night；显式 Day/Night 的最终明暗也控制语法高亮，
  不会被终端背景探测反向覆盖。收到新的终端事实会重新解析当前主题。
- `App::take_theme_request()` 返回待切换定义，运行时自动重绘；`cx.theme_version()` 供缓存失效。
  版本号仅在同一个 resolver 内有效；跨 resolver 比较完整 `ResolvedTheme`。
- 可选 `theme-config` feature 提供严格的 `ThemeDefinition::from_toml`；文件读取、保存与重载触发
  仍由宿主负责。无效配置返回错误，不静默忽略未知角色、状态或颜色。
- `ResolvedTheme::contrast` 返回估算对比度；终端默认 RGB 未知时返回 `None`。ANSI 色值只是估计，
  预览中的 `!` 表示低于 4.5，不代表完整无障碍认证。


## 和 ratatui 的关系

gbtui **不是** retained 组件树，也 **不是** 整仓 fork ratatui。

- 绘制仍用 ratatui 0.29 widgets / `Buffer` / `Frame`
- 事件循环、HitMap、同帧 hover、按需 tick 是 gbtui-runtime
- 唯一 fork：`gbtui-terminal::Terminal`（inline viewport，历史进终端原生 scrollback）

```text
your app
    ├─ gbtui-agent (可选会话 chrome)
    │     ├─ gbtui-markdown → gbtui-markdown-core (流式富文本)
    │     └─ gbtui (facade)
    └─ gbtui (facade)
            ├─ gbtui-actions          可选 ActionRegistry<Id, Ctx>
            └─ gbtui-widgets          Button Modal Picker Prompt StatusBar …
                  └─ gbtui-runtime    run / FrameCtx / HitMap
                        ├─ gbtui-input
                        └─ gbtui-terminal   probe + inline Terminal
                              └─ gbtui-core   Theme KeyShortcut TickDemand
```

依赖只能向下。`runtime` 不得依赖 `widgets`。基础 `gbtui` 不依赖 `gbtui-agent` 或 Markdown 层；非会话程序可以单独使用 `gbtui-markdown`。

## 状态

P0–P15 的已约定通用框架范围已完成，包括主题解析/透明背景/热切换、颜色适配、失焦/拖拽恢复、富选项/表单、范围选择、历史下拉和独立内容查看器。**这不等于复刻 Grok Build 产品或逐像素一致**：LLM/ACP、工具执行、session store、Mermaid/voice 等仍在范围外；完整终端兼容矩阵也尚未验收。实际验证边界见 [terminal-matrix.md](docs/terminal-matrix.md)。

| Crate | 职责 |
|-------|------|
| `gbtui` | facade，应用应依赖这一包 |
| `gbtui-core` | ThemeDefinition / ThemeResolver / ResolvedTheme、调色板与语义状态，以及 KeyShortcut / TickDemand / Renderable / TextMatcher |
| `gbtui-terminal` | OSC11 / DA2 / XTVERSION、增量输入保真、Kitty 键盘、inline viewport、OSC 8、可选图片协议 |
| `gbtui-input` | modifier 救援、滚轮速度加速与有限惯性 |
| `gbtui-runtime` | 事件循环、FrameCtx、HitMap、独立动画/滚动调度 |
| `gbtui-widgets` | 富 List / Picker / Choice / Form、Prompt addon、通用 ContentViewer，以及 Button / TextArea / Modal / chrome |
| `gbtui-actions` | 可选泛型 keymap（无产品枚举） |
| `gbtui-agent` | 可选 Transcript / ToolRow / PermissionCard / SlashPalette / MentionChip，以及 MarkdownDocument / CodeDocument / CodeView 适配器（不进 `gbtui`） |
| `gbtui-markdown` | 独立流式 Markdown / 表格 / 链接元数据 / LaTeX Unicode / 代码语法高亮 |
| `gbtui-markdown-core` | 仅依赖 pulldown-cmark 的 headless 解析配置与结构分析 |

## 兼容性

实际交互验收以 macOS Kitty 和受控 PTY 为主；Windows、Apple Terminal、VS Code 及完整多路复用器组合尚未逐一验收。透明是终端默认背景，不是 alpha；自动刷新消费终端事实或宿主更新，没有 OS 主题监听器。详情见 [终端矩阵](docs/terminal-matrix.md)。

## 贡献

提交改动前请阅读 [贡献指南](CONTRIBUTING.md)，说明复现步骤、目标终端和验证结果。

## 检查

```bash
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo build --workspace --all-features --bins --examples
cargo check -p gbtui --no-default-features
bash scripts/check-no-grok-deps.sh
```

## 文档

- [`docs/plan.md`](docs/plan.md) — 约束清单
- [`docs/architecture.md`](docs/architecture.md) — FrameCtx / TickDemand / z-order
- [`docs/source-map.md`](docs/source-map.md) — grok 文件 → 框架文件
- [`docs/terminal-matrix.md`](docs/terminal-matrix.md) — 终端支持
- [`docs/inline-viewport.md`](docs/inline-viewport.md) — inline Terminal
- [`CONTRIBUTING.md`](CONTRIBUTING.md)

## 许可

Apache-2.0。派生部分归属见 [NOTICE](NOTICE)（Grok Build TUI / Ratatui fork）。默认主题不含 Grok 品牌。

# gbtui：从 Grok Build TUI 抽取可复用框架

> 本文件是前进方向的**唯一约束文档**（仓库内副本：`docs/plan.md`）。实现、review、加功能时先对这里打勾/改状态，不允许静默偏离。若要改架构，先改本文件对应决策，再改代码。
>
> 勾选约定：`- [ ]` 未开始 · `- [x]` 已完成 · 进行中把条目改成 `- [ ] **IN PROGRESS**` · 阻塞写 `- [ ] BLOCKED: 原因`。
>
> 源码只读参考：`sources/grok-build/`。框架代码写在仓库根目录，**禁止**把 grok 应用逻辑迁进框架，也**禁止**在 grok 树里原地拆库。

---

## 进度总表

| 阶段 | 名称 | 状态 | 完成判据（一句话） |
|------|------|------|-------------------|
| P0 | 边界与仓库骨架 | **已完成** | workspace 能 `cargo test`，且 CI 拒绝依赖 `xai-grok-*` |
| P1 | 抬两个干净 crate | **已完成** | TextArea / inline Terminal 有 example，无 grok 依赖 |
| P2 | 运行时原语 | **已完成** | 空闲不 tick；键鼠归一化有单测 |
| P3 | FrameCtx + HitMap | **已完成** | Button hover/click 不写应用级 `match mouse` |
| P4 | 第一批组件 | **已完成，含富行/搜索/Picker** | Modal / Overlay / List / Picker 全部走 FrameCtx |
| P5 | 输入与 chrome | **已完成，含 Prompt addon** | 剥离后的 Prompt、进度条、快捷键条可独立使用 |
| P6 | 可选 Action 骨架 | **已完成** | 泛型 keymap，无 `SendPrompt` 这类产品枚举 |
| P7 | 三个 example 反哺 | **已完成** | 设置页 / 聊天壳 / 文件选择器达到手感验收 |
| P8 | 文档与抽取清单收口 | **已完成** | README + rustdoc + 源文件对照表齐全 |
| P9 | 第二批 chrome | **已完成** | StatusBar / Toast / Tabs / Chip / Dropdown / Cheatsheet / SystemClipboard 走 FrameCtx |
| P10 | 第三批 chrome | **已完成** | GhostText / Form / Preview / Banner / UsageGauge / TextMatcher / Checklist / ActivityRow |
| P11 | 可选 agent chrome 层 | **已完成** | `gbtui-agent` 在 facade 之上；假会话 example；`gbtui` 树不含 agent |
| P12 | 复杂应用 showcase | **已完成** | `examples/studio` 工作台：同帧 hover、遮挡、按需 tick、ActionRegistry |
| P13 | 通用功能对齐与兼容加固 | **已完成** | 会话阅读、富工具/命令展示、启动探测、加速滚动；统一检查与实际 PTY 验收 |
| P14 | 通用交互与阅读组件对齐 | **已完成** | 颜色适配、失焦恢复、富选项/表单、范围选择、历史下拉、通用查看器；原生 Kitty 与 PTY 验收 |
| — | 迁 Grok pager 到框架 | **明确不做（v1）** | — |

**v1 北极星：** 外部 crate 用约 50 行代码做出带 hover、快捷键、spinner 的对话框，空闲时 CPU 接近 0，且不依赖任何 `xai-grok-*`。

---

## 1. 目标与非目标

### 1.1 要做成什么

一套站在 **ratatui 0.29 + crossterm** 上的 **TUI 应用运行时 + 组件库**，把 Grok Build 里已经打磨好、但还散落在应用里的能力产品化：

1. 生产级键鼠（跨终端 modifier 救援、hover、hit-test、滚轮流、多击、遮挡）
2. 按需动画（Fast / Slow / None，不是永远 30fps）
3. 语义主题 + truecolor 量化
4. 和快捷键表一体的组件（Button / Input / Modal / Picker / List / Overlay）
5. 两种壳：alt-screen 全屏、inline viewport（聊天/REPL）

### 1.2 明确不做（v1 禁区）

违反任一条即视为计划偏离，必须先改本文件：

- [ ] **禁止**抽取 ACP / session store / dispatch 业务 `Effect` / grok `pager/src/slash/**` / grok agent 业务。**允许** P11 在独立 crate `gbtui-agent` 里 REWRITE 通用会话 chrome（Transcript / ToolRow / PermissionCard / SlashPalette / MentionChip）
- [ ] **禁止**COPY grok `scrollback/blocks` 的 ACP/`BlockContent` 类型。**允许**把可复用绘制 STRIP 进 `gbtui-agent`；富 Markdown / Syntect 作为独立 `gbtui-markdown` 与 headless `gbtui-markdown-core` 提取，基础 facade 不依赖它们。仍不搬 mermaid / session store
- [ ] **禁止**抽取 mermaid、gboom、voice、telemetry、privacy banner、credit bar
- [ ] **禁止**把 `xai-grok-markdown` 放进 core（若做，只能是后续可选 crate）
- [ ] **禁止**以 `ActionId::SendPrompt` 这类产品枚举为核心 keymap
- [ ] **禁止** Theme 核心字段出现 `accent_skill` / `accent_remember` / `accent_verify` / `accent_plan` 等 Grok 领域色
- [ ] **禁止**新 crate 依赖任何 `xai-grok-*`（CI 用 `cargo tree` 卡死）
- [ ] **禁止**把 Grok pager 迁到本框架上作为 v1 范围
- [ ] **禁止**做 retained 组件树 / React-for-TUI / 扩大 fork 整个 ratatui
- [ ] **禁止**全局 30fps 空转
- [ ] **禁止**组件自己维护一套与 FrameCtx 平行的 hover 逻辑（P3 之后）

### 1.3 成功标准（v1 出门）

- [x] `cargo test --workspace` 全绿
- [x] `cargo tree -p gbtui -i xai-grok-pager`（及所有 `xai-grok-*`）找不到依赖
- [x] example `settings`：Modal + 按钮 hover + Esc 关闭 + 快捷键条
- [x] example `chat`：inline viewport + Prompt addon，空闲 `TickDemand::None`；spinner 在独立 chrome 示例验收
- [x] example `picker`：模糊搜索列表 + 键盘冒泡 + 滚轮
- [x] 空闲进程不因动画循环占 CPU（测试：无 hover、无 spinner 时 `needs_tick() == None`）
- [x] 每个对外 widget 的 hover/click 都经 `FrameCtx`，应用侧零 `match MouseEventKind`

---

## 2. 锁定决策（改这些必须先改计划）

| ID | 决策 | 理由 | 允许的例外 |
|----|------|------|------------|
| D1 | 独立仓库抽取，grok 源码只读 | `pager-render` 已证明原地抽会被领域依赖污染 | 无 |
| D2 | 混合模型：ratatui 绘制 + FrameCtx 运行时，不是 retained UI | 对齐 Grok 现有手感，避免重写 | 无 |
| D3 | 继续 ratatui 0.29 + crossterm；只 fork `Terminal`（inline viewport） | Grok 已验证；扩大 fork 成本不可控 | 无 |
| D4 | crate 前缀 `gbtui-*`，facade 名 `gbtui` | 与当前 workspace 目录一致 | 发布到 crates.io 前可再定公开名 |
| D5 | 复制后去耦合，不 symlink grok 文件 | 防止领域类型回流 | 无 |
| D6 | Unix 优先（macOS / Linux + tmux）；Windows ConHost / VS Code 特例进 P2+ 加固，不挡 v1 | 矩阵是成本中心 | v1 不声称 Windows 完美 |
| D7 | 许可 Apache-2.0，保留 NOTICE 与 ratatui fork 归属 | 源码 LICENSE 即此 | 主题默认不要 Grok 品牌 logo |
| D8 | 文档：rustdoc 英文；根 README 中英均可，以中文为主 | 使用方是当前项目 | 无 |
| D9 | v1 不发布 crates.io | 先用 example 证明 API | 无 |
| D10 | Rust 1.94 / edition 2024，对齐 grok toolchain | 少一次语言差 | 可按需要降到 2021，须记录 |
| D11 | Agent chrome 是可选叠上层 `gbtui-agent`，永远不是 `gbtui` 的依赖或默认 feature | 协议/会话类型会污染 core；agent CLI 作者按需依赖 | `gbtui` 不得 `pub use` agent 类型；Prompt 仍无 slash/@ |

---

## 3. 架构

```text
应用 (examples / 未来业务)
        │
        ├─ gbtui-agent            可选：会话 chrome（P11）
        │       │
        │       ▼
        └─ gbtui  (facade re-export)
                │
                ├─ gbtui-actions          可选：ActionRegistry<Id, Ctx>
                │
                ▼
            gbtui-widgets                 Button Input Modal Picker List Overlay …
                │
                ▼
            gbtui-runtime                 App 循环, FrameCtx, HitMap, 动画调度
                │
                ├─ gbtui-input            KeyboardNormalizer, 滚轮流, type-ahead
                └─ gbtui-terminal         探测, Kitty, 颜色量化, inline Terminal
                        │
                        ▼
                    gbtui-core            Theme HitId TickDemand KeyShortcut Renderable SafeBuf
```

依赖只能向下。`widgets` 不得直接用 crossterm 事件循环；`runtime` 不得依赖 widgets（避免循环）。`gbtui` 不得依赖 `gbtui-agent`。example 依赖 `gbtui`；`examples/agent` 另可依赖 `gbtui-agent`。

### 3.1 数据流（每帧）

```text
crossterm Event
    → KeyboardNormalizer / ScrollNormalizer
    → Runtime 分发给当前 App
         指针事件：HitMap.hit(z-order, occlusion) → widget callback → AppMsg
         键盘：先焦点组件，未消费再 ActionRegistry 冒泡
    → App 改自己的状态（应用拥有领域 state）
    → 若 Changed 或 tick 需要：draw
         每个 widget.render(area, buf, &mut FrameCtx)
         FrameCtx 在绘制时登记 HitRect，并按当前 mouse 立刻算 hover
         widget 需要动画则 FrameCtx.request(TickDemand)
    → Runtime 根据本帧最大 TickDemand 决定下次睡多久
```

Grok 现状是「上一帧 rect + 下一帧才画 hover」。框架 **v1 改为同帧 hover**：draw 时 `FrameCtx` 已持有最新 mouse 位置，widget 当场决定样式。这样少一帧延迟，也消灭「必须先有缓存 rect」的时序 bug。点击仍用上一帧 HitMap（按下发生在 draw 之间）。

### 3.2 z-order 与遮挡（硬约束）

- HitMap 按登记顺序：后登记的在上。
- Modal 必须先 `cx.occlude(popup_outer_or_full_screen)` 再登记内部按钮，等价 Grok 的 `pos_occluded` / `set_unless_dropdown`。
- Dropdown 打开时，被挡住的 chrome **不得**登记 hit。
- 测试必须覆盖：dropdown 点穿、modal 外点击关闭、叠两层 overlay。

### 3.3 动画（硬约束）

```text
TickDemand::None  <  Slow (~12fps)  <  Fast (默认 30fps，可配)
```

- 空闲必须 `None`。
- spinner / wave / pulse 用 `AnimationClock::phase(period)`，按墙钟不算帧数。
- 禁止 widget 私自 `thread::sleep` 或全局 `OnceLock<Instant>` 驱动重绘（Grok 的 `record_dot_pulse` 那种静态 epoch 只允许在「纯着色函数」里，不能用来唤醒事件循环）。
- resize debounce 固定 16ms，与滚动合帧一致，不跟动画 fps 绑死。

---

## 4. Crate 与目录

仓库根（本 workspace），**不要**写进 `sources/`：

```text
gbtui/
  Cargo.toml                 workspace
  rust-toolchain.toml        1.94.0
  LICENSE                    Apache-2.0
  NOTICE                     ratatui / grok 归属
  THIRD-PARTY-NOTICES
  README.md
  CONTRIBUTING.md
  docs/
    architecture.md          本文精简版，给贡献者
    source-map.md            grok 文件 → 框架文件对照（P0 产出，P8 收口）
    terminal-matrix.md       终端支持声明
  crates/
    gbtui-core/
    gbtui-terminal/
    gbtui-input/
    gbtui-runtime/
    gbtui-widgets/
    gbtui-actions/
    gbtui/
    gbtui-agent/             可选会话 chrome（P11；依赖 facade，不反向）
  examples/
    settings/                Modal + Button + ShortcutsBar
    chat/                    inline + TextArea + spinner
    picker/                  fuzzy list + 键盘 + 滚轮
    agent/                   假会话：Transcript / ToolRow / Permission / Slash
  scripts/
    check-no-grok-deps.sh    cargo tree 黑名单
```

每个 crate 的 `Cargo.toml`：`license = "Apache-2.0"`，edition 2024。

---

## 5. 核心 API（实现时按此签名靠拢，不经计划不得改语义）

### 5.1 `gbtui-core`

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TickDemand { None, Slow, Fast }

pub struct KeyShortcut { pub code: KeyCode, pub modifiers: KeyModifiers }
// 保留 grok 的 key!() 宏与大小写归一

pub trait Renderable {
    fn render(&self, area: Rect, buf: &mut Buffer);
    fn desired_height(&self, width: u16) -> u16;
}

pub trait SafeBuf { /* set_line_safe / set_span_safe / set_string_safe；bidi 默认关 */ }

pub struct Theme { /* 见 5.2，禁止领域色 */ }
pub trait ThemeExt { /* 应用附加色，框架不解读 */ }

#[derive(Clone, Copy, Hash, Eq, PartialEq)]
pub struct HitId(u64); // 或 &'static str + generation；稳定、不复用正在屏幕上的 id
```

### 5.2 Theme 核心字段（白名单）

只允许这些（或与之等价的分组 struct）。要加字段先改本计划。

- 背景：`bg_base`, `bg_elevated`, `bg_dark`, `bg_highlight`, `bg_hover`, `bg_visual`
- 文本：`text_primary`, `text_secondary`, `gray_dim`, `gray`, `gray_bright`
- 语义：`accent`, `success`, `error`, `warning`
- 边框：`selection_border`, `hover_border`, `border`
- 滚动条：`scrollbar_bg`, `scrollbar_fg`
- 控件：`button_fg`, `button_bg`, `button_hover_bg`, `input_bg`, `input_border`, `input_border_active`
- 搜索：`fuzzy_accent`

默认主题可从 GrokNight / TokyoNight **去领域化**移植（去掉 skill/remember/plan/verify/user/assistant/tool 等）。提供 2 套：`night`、`day`。`auto` 跟随系统 appearance（从 grok `system_appearance` 抽）。

量化：启动时按终端能力把 Rgb 打到 truecolor / 256 / 16。运行时生成色也走 `quantize`。

### 5.3 `gbtui-runtime::FrameCtx`

```rust
pub struct FrameCtx<'a> {
    pub theme: &'a Theme,
    pub mouse: Option<(u16, u16)>,
    pub anim: AnimationClock,
    hits: &'a mut HitMap,
}

impl FrameCtx<'_> {
    pub fn hovered(&self, area: Rect) -> bool;
    pub fn register(&mut self, id: HitId, area: Rect);
    pub fn occlude(&mut self, area: Rect);          // 吞掉下方 hit
    pub fn request_tick(&mut self, TickDemand);
}

pub enum Pointer {
    Move { col: u16, row: u16 },
    Down { button: MouseButton, col: u16, row: u16, clicks: u8 },
    Up { button: MouseButton, col: u16, row: u16 },
    Drag { button: MouseButton, col: u16, row: u16 },
    Scroll { delta_lines: i32 },                    // 已归一化的行，不是原始事件
}

pub trait App {
    type Msg;
    fn handle(&mut self, ev: AppEvent<Self::Msg>) -> Outcome;
    fn draw(&mut self, f: &mut Frame, cx: &mut FrameCtx);
    fn tick_demand(&self) -> TickDemand;            // 与 draw 中 request_tick 取 max
}

pub enum Outcome { Unchanged, Changed, Quit }
```

`run(app)` 提供默认事件循环：终端事件 / animation deadline / resize debounce / 可选外部 `mpsc`。

**P3 之后的铁律：** 新 widget 若在应用里出现 `MouseEventKind::Moved => hit.update_hover`，视为回归。

### 5.4 Button（第一个证明件）

```rust
Button::new("Cancel")
    .id("cancel")
    .focused(false)
    .on_click(|| Msg::Cancel)
    .render(area, buf, cx);
```

验收：hover 用 `theme.button_hover_bg`；Enter/Space 在 focused 时触发；被 occlude 时不点击。

---

## 6. 源文件对照（抽取纪律）

状态含义：

- **COPY**：几乎原样搬，只改 crate 路径 / 去 grok 类型
- **STRIP**：搬文件但删除领域依赖与领域字段
- **REWRITE**：用 FrameCtx 重写， grok 只作行为参考
- **NEVER**：不进入框架

### 6.1 COPY

| 源 | 目标 |
|----|------|
| `xai-ratatui-inline/**` | `gbtui-terminal` 的 inline 模块（或子 crate `gbtui-inline`，若体积需要再拆） |
| `xai-ratatui-textarea/**` | `gbtui-widgets` 的 `textarea` |
| `pager-render/src/input/key.rs` | `gbtui-core` |
| `pager-render/src/render/safe_buf.rs` | `gbtui-core`（bidi 默认关，RTL 作 feature） |
| `pager-render/src/render/renderable.rs` | `gbtui-core` |
| `pager-render/src/render/wrapping.rs` + `line_utils.rs` | `gbtui-core` / `gbtui-widgets` |
| `pager/src/views/overlay.rs` | `gbtui-widgets`（纯状态机，几乎原样） |
| `pager-render/src/input/line_editor.rs` | `gbtui-widgets`（依赖 textarea） |

### 6.2 STRIP

| 源 | 怎么剥 |
|----|--------|
| `pager-render/src/theme/**` | 只留量化、系统 appearance、OSC11、2 套去领域化主题 |
| `pager-render/src/input/keyboard_normalizer.rs` + `macos_modifiers.rs` | 去掉 pager `AppView` 绑定 |
| `pager-render/src/input/mouse.rs` | 去掉 `GROK_SCROLL_*` 环境变量名，改为框架 config |
| `pager-render/src/terminal/**` | 去掉 gboom overlay、image protocol 可 feature-gate；v1 要探测 + Kitty + tmux |
| `pager-render/src/glyphs.rs` | 去掉 voice record_dot；保留 prompt_arrow / 关闭按钮 / 进度条 glyph 与 ConHost 回退 |
| `pager/src/views/modal_window.rs` + `pager-render/src/modal_window_state.rs` | 去掉 `EMBEDDED` 全局 AtomicBool，改为 `ModalStyle::{Popup, Embedded}` 参数 |
| `pager/src/views/list_pane/**` | 去掉 tracing/todo 特例注释外的硬编码 |
| `pager/src/views/picker.rs` | 去掉 session/command palette 文案 |
| `pager/src/views/progress_bar.rs` | 保留 1/8 格与 ConHost shade 回退 |
| `pager/src/views/shortcuts_bar.rs` | 去掉 PendingAction 的产品文案，保留双击确认 API |
| `pager/src/actions/{mod,defaults}.rs` | 只学结构；**不要**复制 `ActionId` 枚举 |

### 6.3 REWRITE（必须走 FrameCtx）

| 源 | 为什么不能 COPY |
|----|-----------------|
| `pager/src/app/agent_view/mod.rs` 的 `HitArea` | 升成 HitMap，不再每个字段一个 HitArea |
| `pager/src/app/mouse.rs` | 整文件是应用 match，是框架要消灭的东西 |
| `pager/src/app/event_loop.rs` | 去掉 ACP / type-ahead 业务；只留循环骨架 |
| `pager/src/app/app_view.rs` 的 `tick` / `needs_animation` / `TickDemand` | 学调度，不搬 200 行业务 |
| `pager/src/views/prompt_widget/**` | 砍 @文件搜索 / slash / image chip；这些若做是 addon |
| `pager/src/views/question_view.rs` | 做成 `ChoiceList`，不带 ACP `AskUserQuestion` |
| Button / Link / Chip | Grok 里不是类型，是字符串 + HitArea，**新写** |

### 6.4 NEVER

`xai-grok-shell/**`、`pager/src/app/dispatch/**`、`pager/src/scrollback/blocks/**`、`pager/src/slash/**`、`pager/src/acp/**`、`xai-grok-mermaid/**`、`xai-grok-gboom/**`、`xai-grok-voice/**`、`xai-grok-telemetry/**`、`views/privacy_banner.rs`、`views/credit_bar.rs`、`views/welcome/**`（logo shimmer 可参考动画，不搬欢迎流程）。

P0 必须把上面四类写成 `docs/source-map.md`，之后每搬一个文件就更新一行。

---

## 7. 分阶段可勾选任务

实现时按阶段顺序。同一阶段内无依赖的条目可并行，但 **P3 未完成不得开始 P4 组件**（否则会把旧 HitArea 模式复制进框架）。

### P0 — 边界与仓库骨架

**验收：** `cargo test --workspace` 通过（哪怕只有空 crate + 依赖黑名单测试）。

- [x] 在仓库根建立 Cargo workspace（members = crates；examples 待 P3 起加入，避免空 package）
- [x] 加入 `rust-toolchain.toml`（1.94.0）、`rustfmt.toml` / `clippy.toml`（可从 grok 精简拷贝，去掉 grok 专用 lint 例外）
- [x] 加入 `LICENSE`（Apache-2.0）、`NOTICE`（说明派生自 Grok Build TUI / ratatui fork）
- [x] 复制 grok 的 `xai-ratatui-inline/NOTICE` 与 textarea NOTICE
- [x] 写 `scripts/check-no-grok-deps.sh`：`cargo tree --workspace` 命中 `xai-grok-` 则失败
- [x] 空 crate 占位：`gbtui-core` / `terminal` / `input` / `runtime` / `widgets` / `actions` / `gbtui`，依赖方向按 §3 写死
- [x] `docs/source-map.md` 初稿：把 §6 全部文件标 COPY/STRIP/REWRITE/NEVER
- [x] `docs/terminal-matrix.md` 初稿：v1 支持 / 尽力 / 不承诺
- [x] README 写清：这是框架，不是 Grok；源码参考路径；如何跑 example
- [x] CI 或本地约定：`cargo fmt --check && cargo clippy --all-targets -- -D warnings && ./scripts/check-no-grok-deps.sh`

### P1 — 抬两个干净 crate

**验收：** `examples` 能跑 textarea 与 inline 的 grok 原 example（改名即可）；`cargo tree` 无 grok。

#### P1a TextArea

- [x] 复制 `xai-ratatui-textarea` 全部源文件到 `crates/gbtui-widgets/src/textarea/`
- [x] 重命名 crate 路径、license、去掉 `xai_` 前缀
- [x] 保留：chip / hover element / mouse / undo / ClipboardProvider
- [x] 接入将来的 Theme（本阶段可用简单 Style 参数，P4 再换 Theme；**不要**在本阶段引入 HitArea 手工模式）
- [x] 原 `examples/textarea_demo.rs` 迁到 `gbtui-widgets` example 或 workspace example
- [x] 原 editor 单测一起搬，全绿

#### P1b Inline Terminal

- [x] 复制 `xai-ratatui-inline` 到 `gbtui-terminal`（模块 `inline`）
- [x] 保留 synchronized output、RIS resize、scrollback insert_before
- [x] 搬 README 的架构说明进 `docs/`
- [x] 单测 / differential segment 测试一起搬

### P2 — 运行时原语（手感的 80%，尚无 FrameCtx）

**验收：** 单元测试覆盖 key 匹配、modifier rescue、滚轮流合帧；core 能独立编译。

#### P2a `gbtui-core`

- [x] `KeyShortcut` + `key!()` + 大小写归一（COPY `input/key.rs`）
- [x] `TickDemand` + `AnimationClock::{phase, wave_brightness, pulse_brightness}`
  - 参考 `theme::wave_brightness` / `pulse_brightness` 与 `AnimationConfig`（默认 fps=30, wave_rows=32）
- [x] `Renderable` + `SafeBuf`（COPY，bidi 默认 off）
- [x] wrapping / unicode 宽度工具（STRIP `render/wrapping.rs`, `line_utils.rs`）
- [x] `Theme` 白名单字段 + `night` / `day` 两套（STRIP `tokyonight`/`groknight`/`grokday`，删除领域色）
- [x] `quantize` + 终端色深探测（STRIP `theme/color_support.rs`）
- [x] 单测：key 归一、SafeBuf 越界不 panic、quantize 在 16 色下不产出 Rgb

#### P2b `gbtui-terminal`

- [x] 终端品牌 / multiplexer 探测（STRIP `terminal/mod.rs` 及 env 纯函数；可测矩阵保留）
- [x] Kitty keyboard 协商（STRIP `kitty_keyboard.rs`）
- [x] `ModifierDelivery` / `keyboard_capabilities`（STRIP `terminal/keyboard.rs`）
- [x] tmux passthrough / OSC11 是否包一层（STRIP 相关；去掉 grok 专用 env 名）
- [x] 系统 dark/light（STRIP `system_appearance.rs`）
- [x] glyph 回退：`prompt_arrow`、关闭 `✗`、进度条（STRIP `glyphs.rs`，删除 voice）
- [x] P2 不承诺 DA2 全矩阵；P13 补充已知安全终端的有界 OSC 11 / DA2 / XTVERSION 探测。未知终端与复用器保守跳过；image protocol 仍受可选 feature 控制。

#### P2c `gbtui-input`

- [x] `KeyboardNormalizer` + `ModifierProbe` + macOS CoreGraphics（COPY/STRIP `keyboard_normalizer.rs`, `macos_modifiers.rs`）
- [x] 滚轮流：STREAM_GAP 80ms、合帧 16ms、wheel vs trackpad 启发式（STRIP `input/mouse.rs`，config 改名为 `gbtui` 前缀）
- [x] type-ahead 过滤器（从 event_loop 抽出纯函数 `is_typeahead_event` / `filter_startup_typeahead`，去掉业务）
- [x] 单测：Ctrl+B 救援、无 modifier 不误伤、滚轮流合成行数

**本阶段禁止：** 写 `handle_mouse` 大 match、禁止在 core 里出现 Agent/Prompt 字样。

### P3 — FrameCtx + HitMap + 事件循环（框架真正诞生）

**验收：** 一个最小 example 里，Button 有 hover/click，应用代码没有 `MouseEventKind` 分支；空闲 `TickDemand::None`。

- [x] 实现 `HitMap`：register / occlude / hit-test / 多击时间窗（参考 `MULTI_CLICK_TIMEOUT_MS`）
- [x] 实现 `FrameCtx`（§5.3），draw 同帧 hover
- [x] 实现 `App` trait + `run()`：
  - [x] `tokio::select!`：终端事件、animation deadline、resize debounce 16ms、可选 `mpsc`
  - [x] 事件先走 KeyboardNormalizer / ScrollNormalizer
  - [x] 指针事件只打到 HitMap 命中的 id
  - [x] 键盘先给焦点 widget，未消费再交给 App
  - [x] 根据本帧 `TickDemand` max 安排下次 tick；None 则永不空转
- [x] **REWRITE** `HitArea` 为内部细节，不作为公开 API 鼓励应用散落使用
- [x] 单测（必须，假终端 / 注入事件）：
  - [x] 后登记的 hit 盖住先登记的
  - [x] occlude 区域吞点击
  - [x] dropdown 打开时底下按钮点不着
  - [x] hover 同帧变色（注入 Move 后下一 draw 即 hover style）
  - [x] 无动画组件时 `tick_deadline` 不武装
  - [x] resize 风暴只 draw 一次（debounce）
- [x] 用 **REWRITE 的 Button** 作为证明件（见 §5.4），放进 `gbtui-widgets`，即使 P4 其它组件还没有
- [x] 最小 bin：`examples/settings` 先只放一个 Cancel 按钮 + hover，作为 P3 出门门槛

**门禁：** P3 单测不过，不准开始 P4。

### P4 — 第一批组件（全部 FrameCtx）

每个组件出门清单（复制到每个组件 PR/任务里）：

1. 绘制只用 Theme，无硬编码 Color
2. hover / 焦点样式
3. 键盘（文档化的绑定）
4. 鼠标（只经 FrameCtx）
5. 被 occlude 时不响应
6. 单测 + 在某个 example 里可见
7. rustdoc 含一小段渲染示意图

#### P4a ModalWindow（STRIP + 接 FrameCtx）

- [x] 搬 `modal_window.rs` / `modal_window_state.rs`
- [x] 删除全局 `EMBEDDED: AtomicBool`，改为 `ModalKind::{Popup, Embedded}`
- [x] close `[✗]`、tabs、footer shortcuts 的 hover/click 走 HitMap
- [x] Esc 关闭、点击外部关闭、Tab 切页
- [x] 打开时 `occlude` 整块 popup（Popup）或全屏遮罩
- [x] 单测：外点关闭、close hover、footer 快捷键点击

#### P4b OverlayState（COPY）

- [x] 搬三态状态机与 `handle_overlay_key` / `handle_overlay_nav_key`
- [x] 快捷键默认：toggle / Tab 出焦点 / Esc 分层退出 / Ctrl-F 全屏——作为建议绑定，应用可改
- [x] 不把 tracing/todo 业务带进来

#### P4c ListPane（STRIP）

- [x] 搬 `views/list_pane/**`：state 与数据分离、虚拟滚动、filter、follow
- [x] 选择 / hover 行走 FrameCtx；scrollbar 可点
- [x] `ListPaneStyle` 从 Theme 派生，不自己存一套 Color
- [x] 单测：大量行 desired_height、follow 模式、filter
- [x] 通用列表补齐：Wrap/NoWrap、变高布局缓存、物理行滚动与 scrollbar、内容/宽度/过滤变化失效；长单项及后续项目可完整浏览
- [x] 富行 Renderable 切片绘制接入同一 ListPane；Filter/Search 模式、fuzzy 排序、regex 与 Unicode 安全高亮，基础 core 仍不依赖 regex

#### P4d Picker（STRIP）

- [x] 搬 picker 的 search bar / row / header / 滚动
- [x] 接 `LineEditor` + TextArea — **P5**（Picker query 用 `LineEditor`）
- [x] 选中/hover 行；键盘 j/k 或 Ctrl-N/P
- [x] 去掉 session/command palette 专用字段，使用 `PickerEntry::group(PickerGroup)` / `PickerEntry::row(PickerRow)` 构造通用条目
- [x] Picker 富行、多样式 label、展开字段/摘要、badge、多行描述、缩进/折叠分组及搜索交互

#### P4e ChoiceList（REWRITE from question_view）

- [x] 单选 / 多选 / 可选自由输入
- [x] 不出现 ACP / `AskUserQuestion` / telemetry
- [x] 选项 hover + 键盘 nav + Enter 提交

### P5 — 输入与 chrome

#### P5a Prompt / 多行输入（STRIP prompt_widget）

- [x] 保留：TextArea 封装、Enter 提交 vs 换行策略、占位符、前缀
- [x] 宿主驱动的补全/历史/粘贴折叠/附件 addon；不迁 slash registry、文件扫描、图片读取或历史存储业务
- [x] chrome（accent line、focus box）由调用方或 `PromptChrome` 可选开关绘制
- [x] 焦点管理（Esc/Tab）仍由调用方负责——保持 grok PromptWidget 的职责切分

#### P5b Progress + Spinner

- [x] 搬 `progress_bar.rs`（1/8 格 + ConHost shade）
- [x] Spinner：braille 周期，`cx.request_tick(Fast)`，停转后 demand 降下来
- [x] wave accent 作为 `widgets::WaveBar` 可选，不绑「running tool」语义

#### P5c ShortcutsBar

- [x] 搬 `shortcuts_bar.rs`：`HintItem`、紧凑截断、pinned
- [x] 双击确认：「press again to {label}」做成通用 `PendingConfirm`，TTL 可配（Grok 1000ms）
- [x] 与 P6 registry 可接，但本阶段允许手写 `Vec<HintItem>`

#### P5d Link

- [x] OSC8 超链接能力探测与完整输出：Link / 富 Markdown 经 FrameCtx 收集、遮挡剪裁，在实际 Terminal diff 中发出；未知/Hostile 终端保留文本与点击
- [x] hover 样式 `theme.hover_border` / underline
- [x] 不绑定「Cmd 才显示」的 macOS 特例为默认；作为 `LinkActivation::{Always, Modifier}` 配置（Grok 的 native link hover 可参考）

### P6 — 可选 Action 骨架（`gbtui-actions`）

**验收：** example 用 registry 同时驱动快捷键条和 key dispatch；crate 无产品枚举。

- [x] `ActionDef<Id, Ctx>`：id / label / description / default_key / alt_keys / context / hint_priority
- [x] `When`/`Ctx` 由应用定义，框架只做 **精确匹配 + 显式冒泡**（学 grok 注释：冒泡写在代码里，不藏在 `context_matches`）
- [x] `registry.lookup(key, ctx)` / `registry.hints(ctxs)` / `registry.all()` 给命令面板
- [x] **禁止**把 grok 的 `ActionId` 拷进来；example 自己定义 `enum DemoAction { Quit, Confirm, … }`
- [x] 终端无关键的 hint 切换做成 hook（学 `ctrl_dot_unreliable`，但默认实现可简化）

### P7 — 三个 example 反哺（v1 手感验收）

每个 example 必须：键盘 + 鼠标 + hover + 空闲不 tick。缺一不可。

#### E1 `examples/settings`（可最早在 P3/P4 生长）

- [x] 打开设置 Modal
- [x] 列表改一项 + 按钮 Save / Cancel（hover、Enter、点击）
- [x] footer 快捷键可点
- [x] Esc / 点外部 / 点 ✗ 关闭
- [x] 底栏 ShortcutsBar — P5c
- [x] 无操作时 TickDemand::None

#### E2 `examples/chat`

- [x] inline viewport：历史进原生 scrollback，底栏输入固定
- [x] TextArea 多行；Enter 发送（假回显），Mod+Enter 换行
- [x] 发送后 1s spinner，然后停止（证明 tick 会降回 None）
- [x] 快捷键 Quit
- [x] resize 不打乱历史（RIS 路径）

#### E3 `examples/picker`

- [x] 模糊过滤文件或静态列表
- [x] j/k、滚轮、hover 选中
- [x] Enter 确认打印到 stderr 后退出
- [x] 打开时 occlude 背景

反哺规则：example 里若不得不写 `MouseEventKind` match 或硬编码 Color，**记一条框架缺口，回到对应 crate 补，不准在 example 里长出第二套运行时**。

### P8 — 文档与收口

- [x] 根 README：5 分钟上手、架构图、与 ratatui 关系、许可
- [x] `docs/architecture.md`：FrameCtx、TickDemand、z-order
- [x] `docs/source-map.md` 每个源文件最终状态（Copied / Stripped / Rewritten / Never）
- [x] `docs/terminal-matrix.md`：Kitty/Ghostty/iTerm2/WezTerm/tmux = v1；Apple Terminal/VS Code/Windows = 已知缺口
- [x] 每个公开类型 rustdoc
- [x] CONTRIBUTING：依赖方向、禁止 grok 依赖、新 widget 出门清单
- [x] 在本 plan 进度总表把 P0–P8 标完成

### P9 — 第二批 chrome（v1 之后，禁区不变）

v1 出门清单已收口。本阶段只加**通用 chrome**，不重开 NEVER，不迁 pager。

每个组件仍走 P4 出门清单（Theme、hover/焦点、文档化按键、只经 FrameCtx、occlude 不响应、单测 + example、rustdoc 示意图）。

**验收：** `examples/settings` 能看到 StatusBar、Toast、Tabs、Chip、Dropdown、Cheatsheet；Prompt 可选用系统剪贴板；空闲 `TickDemand::None`（Toast 可见期间允许 Slow）。

#### P9a StatusBar（STRIP `views/status_bar.rs`）

- [x] 一行左 / 中 / 右，颜色只来自 Theme（`gray` / `bg_base`）
- [x] 宽度不够时中/右段不覆盖左段
- [x] 无硬编码 Color；不必登记 hit（纯展示）

#### P9b Toast（REWRITE `views/welcome/toast.rs` 布局）

- [x] 一行短暂提示；默认 TTL 3000ms
- [x] 可见时 `cx.request_tick(Slow)`，到期后 demand 降为 None
- [x] 可点关闭（HitId）；颜色用 `accent` / `bg_elevated`，**禁止** `accent_user`
- [x] **禁止**搬欢迎流程 / consent / logo

#### P9c Tabs（从 Modal `paint_tabs` 拆出）

- [x] 独立 `Tabs`：hover / 选中、左右循环、HitMap `NS_TAB`
- [x] Modal 改调 `Tabs`，行为与 P4a 一致

#### P9d Chip（REWRITE，Grok 里不是类型）

- [x] 可关闭标签：label + `[✗]` hover/click
- [x] Theme only；occlude 下不点

#### P9e Dropdown（STRIP 几何，参考 `completion_dropdown.rs` / `overlay_list.rs`）

- [x] 借用 `{label, detail}`；open / selected / offset
- [x] 打开时 `occlude` 自己的外接矩形，再登记行 hit
- [x] ↑↓/j k、Enter 选中、Esc 关闭；滚轮
- [x] **禁止** slash / @ 文件搜索 / 工作区 FS

#### P9f Cheatsheet（学 `shortcuts_help.rs` 结构）

- [x] 条目由调用方提供：`Header` / `Hint`（`HintItem` + 可选 long_help）
- [x] 用现有 Modal + List 画；搜索过滤 label/key/description
- [x] **禁止**拷贝 grok `ActionId` / `Category` 产品枚举；example 用 `registry.all()` 自备分组

#### P9g SystemClipboard（STRIP `pager-render/clipboard`，剥 grok wrap）

- [x] `ClipboardProvider`：arboard；失败则内部缓冲
- [x] Unix：OSC 52 可作为后援，**禁止** `GROK_COPY_FILE` / `grok wrap` / remote 探测业务
- [x] feature `clipboard`（默认开）；无该 feature 时仍能编过 InternalClipboard
- [x] Prompt / TextArea 可注入该 provider

**本阶段明确不做：** 更多主题、List 折行虚拟化、滚轮加速、OSC8 真发出、OSC11、markdown、Prompt slash/@ addon。

### P10 — 第三批 chrome（禁区不变）

P9 已收口。本阶段只加**通用 chrome / 纯函数**，不重开 NEVER，不迁 pager。

每个组件仍走 P4 出门清单。

**验收：** `examples/settings` 能看到 Form、Preview、Banner、UsageGauge、Checklist，Prompt 可显示 GhostText；`examples/chat` 能看到 GhostText + ActivityRow；空闲 `TickDemand::None`。

#### P10a GhostText（REWRITE `prompt_suggestion.rs` 状态机）

- [x] `set` / `clear` / `dismiss` / `ghost_for` / `accept`；无 generation、ACP、telemetry、`GROK_PROMPT_SUGGESTIONS`
- [x] 绘制 dim+italic 接在光标后；ghost 可见时不画 placeholder
- [x] `→` 在末尾接受；空输入 `Esc` dismiss；`Tab` 默认关（`accept_tab`），chat 打开
- [x] 挂在 Prompt 上，不改 TextArea 内部协议

#### P10b Form 行（STRIP `settings_modal` 行种类）

- [x] `Header` / `Toggle` / `Select` / `Stepper` / `Field`；调用方拥有数据
- [x] Select 复用 Dropdown；**禁止** `SettingKey` / 落盘 / consent

#### P10c Preview overlay（STRIP `pager-render/.../preview_overlay.rs`）

- [x] 首 N + `⋮ (k more)` + 末 N；Theme `bg_elevated` / `text_primary` / `border`
- [x] `occlude` 外接矩形后登记 `BODY`

#### P10d Banner（REWRITE `announcements.rs` 布局）

- [x] 1–2 行 + `[hide]`；`BannerKind::{Critical, Info}`
- [x] **禁止** announcements crate / promo CTA

#### P10e UsageGauge（STRIP `context_bar.rs` 几何）

- [x] 紧凑数字；hover 同宽进度条+百分比
- [x] 断点 `text_primary` → `accent` → `warning` → `error`

#### P10f TextMatcher（STRIP `pager-render/src/search/`）

- [x] 进 `gbtui-core`：smart-case 子串、`find_all`、n/N 环游
- [x] **禁止**本阶段引入 `regex` crate

#### P10g Checklist（REWRITE `todo_pane` 图标行）

- [x] `CheckStatus::{Pending, Active, Done, Cancelled}`；Space 循环
- [x] **禁止** grok `TodoItem`

#### P10h ActivityRow（REWRITE `turn_status.rs` 布局）

- [x] Spinner + label + 计时 + 可选 `[stop]`
- [x] 可见 Fast tick；隐藏 demand None；**禁止** ACP `TurnActivity`

**本阶段明确不做：** 更多主题、List 折行、滚轮加速、OSC8/11、markdown、Prompt slash/@、`regex` 依赖。

### P11 — 可选 agent chrome 层（叠在 facade 之上）

P10 已收口。本阶段**不往 `gbtui` / widgets 塞领域组件**，新建 `gbtui-agent`：只依赖 `gbtui` 公开 API。`gbtui` 不依赖、不 re-export 本 crate。

每个组件仍走 P4 出门清单。

**验收：** `cargo run -p agent` 假会话（用户行 / 工具行 / 流式助手 / 权限卡 / slash 面板）；`cargo tree -p gbtui` 不含 `gbtui-agent`；`cargo tree -p gbtui-agent` 不含 `xai-grok-*`；空闲 `TickDemand::None`。

#### P11a 仓库骨架

- [x] workspace member `crates/gbtui-agent` + `examples/agent`
- [x] 依赖：`gbtui` + ratatui + crossterm + unicode-width
- [x] 防火墙覆盖新 crate；断言 `gbtui` 树不含 `gbtui-agent`

#### P11b Transcript（REWRITE 会话列表，不是 grok scrollback）

- [x] `TranscriptItem::{User, Assistant, Thinking, System, Marker, Hook, Tool}`；宿主拥有 owned 数据，render 借用
- [x] `TranscriptState` 以物理行偏移滚动，包含块间空行；按 item `measure` 定位可见窗口，允许显示单个长块的任意行（不是 grok wrap virtualizer）
- [x] 用独立 Markdown 引擎替换 markdown-lite，统一测量/绘制/选择与复制映射；保留 streaming 行尾 Spinner + Fast，追加渲染与布局缓存
- [x] Hit `namespaced(NS_TRANSCRIPT, i)`；Tool 展开用 `NS_TOOL`
- [x] 用户条 / `Thought` / `Thought for Xs` / `(ctrl+e to expand)` 1:1 STRIP 绘制
- [x] **禁止** COPY `scrollback/blocks/**` 的 ACP/`BlockContent` 类型、mermaid；Markdown 通用引擎只允许经去产品化后的 `gbtui-markdown` 使用，不依赖原 crate
- [x] 通用文本选择：拖选/反向/跨块、软换行还原、Unicode 边界、可见高亮；宿主通过 ClipboardProvider 复制，不搬 Grok selection/ACP 类型

#### P11c ToolRow（REWRITE 工具头，不是 tool/* 种类卡）

- [x] `ToolStatus::{Pending, Running, Done, Error}` + `ToolKind::{Read, Edit, Run, Search, List}`
- [x] 保留 `Read ` / `Edit ` / `$ ` / `Search ` / `List ` 前缀；Edit 提供通用 unified diff 分块、双行号、增删与续行布局，选择复制排除显示 gutter
- [x] Running 才 Fast；Enter/点击展开 detail
- [x] **禁止** COPY `VerbGroupKind` / ACP tool id；只提取通用 diff 展示及源行映射，不依赖工具执行或工作区状态

#### P11d PermissionCard（REWRITE 布局，不是 permission_view）

- [x] title + body + Allow/Deny（固定 HitId）+ 可选额外按钮切片
- [x] Popup `occlude` 全帧；边框 `warning`，**禁止** `accent_user`
- [x] Esc → Deny；**禁止** ACP options / bash pattern / MCP scope

#### P11e SlashPalette（宿主 overlay，Prompt 零改动）

- [x] 复用 Picker 导航；绘制 1:1：`❯ /name` + 列对齐 description、选中 `bg_visual`、hover `bg_hover`
- [x] 宿主检测 Prompt 以 `/` 开头后打开；**禁止**改 `prompt.rs`、grok slash registry/tags/FS

#### P11f MentionChip（Chip 薄封装，无 FS）

- [x] `@` 前缀 + `accent`；`MentionBar` 一行截断 `+N`
- [x] 标签由宿主提供；**禁止** fuzzy-file-search / 工作区索引

**本阶段明确不做：** ACP/MCP/session/LLM、markdown crate、Prompt 内 slash/@、领域 Theme 字段、迁 pager、改 `examples/chat`。

### P12 — 复杂应用 showcase（工作台，不是组件陈列）

`settings` 是 widget 图鉴，`agent` 是会话 chrome。本阶段用 **一个像产品的应用** 把运行时特色叠在一起。

**验收：** `cargo run -p studio`；空闲 `TickDemand::None`；打开 New-issue modal 后点左上角关闭（occlude）；`s` 同步期间 Fast。

- [x] workspace member `examples/studio`，只依赖 `gbtui` facade
- [x] Inbox：项目 overlay + 议题列表 + preview + checklist + TextMatcher 过滤 + ghost prompt
- [x] Files：Picker + Preview overlay
- [x] Settings：Form + Dropdown（occlude）
- [x] Banner / Toast / UsageGauge / ActivityRow / Cheatsheet / ActionRegistry / ShortcutsBar
- [x] 应用侧零 `match MouseEventKind`
- [x] **禁止** ACP / LLM / session store / 把 studio 逻辑塞进 `gbtui`

### P13 — 通用功能对齐与兼容加固

仍不迁移 Grok 产品运行时；所有数据、资源与业务决策由宿主提供。

- [x] Transcript 全文匹配/命中导航、turn/response 跳转、统一 Collapsed/Truncated/Expanded 状态；隐藏命中按需展开并定位，测量、绘制与复制共用可见布局
- [x] ToolRow 按文件类型高亮 diff；SlashPalette 支持宿主提供的来源标签、参数提示和多行变高描述，复用富 Picker/List
- [x] OSC 11 背景色、DA2/Alacritty 版本门控、受限 XTVERSION 启动探测；超时可退化，不吞用户预输入。Unix 启动与常规输入共用增量解码状态，保留跨阶段 UTF-8、CSI 和 bracketed paste，过滤迟到的已请求响应；不 fork crossterm、不引入 unsafe。显式 Theme 原样保留，ThemeKind::Auto 才按环境覆盖、探测背景、OS、Night 顺序选色。
  - inline runtime 的初始光标查询同样归唯一输入解码器管理；Terminal 提供显式初始光标构造入口。运行时托管模式下 resize 以已知 viewport/光标定位，不再让 backend 偷读 stdin；原生历史重排仍走宿主历史 replay。
- [x] 滚轮速度分档、加速、惯性减速和独立 scroll deadline；稳定 scroll-only target 不随行滚动改变，无待处理滚动时恢复空闲不 tick
- [x] 迁移新增枚举与控件接口的所有调用方；无结果选择不能退回首项
- [x] 实际示例覆盖 Prompt addon、富列表/Picker、会话导航和终端协议输出

验收：`cargo test --workspace --all-features` 为 1177 passed / 9 ignored；
严格 Clippy、全部 bins/examples、facade 最小 features 与依赖隔离检查通过。
macOS 受控 PTY 验证了补全不误提交、历史草稿恢复、粘贴展开/撤销、附件提交、
regex 错误恢复、分组/详情展开、隐藏全文命中、两种协议的 response 快捷键、
多行命令描述、OSC11/DA2/XTVERSION、跨启动阶段 UTF-8、粘贴载荷协议隔离，
以及滚动后 500ms 空闲零输出。终端协议注入不等于对应 emulator 的完整实测。

---

## 8. 实现时的检查清单（每次 PR 都过）

提交前：

- [ ] 未引入 `xai-grok-*` 依赖（跑 `scripts/check-no-grok-deps.sh`）
- [ ] 未在 `sources/grok-build/` 改一行
- [ ] 新 widget 走 FrameCtx（P3 后）
- [ ] 无硬编码 `Color::Rgb`（除 Theme 定义文件）
- [ ] 空闲路径不会 `request_tick(Fast)`
- [ ] 有单测或 example 覆盖 hover/键盘/遮挡三者之一以上
- [ ] 更新 `docs/source-map.md` 若搬了 grok 文件
- [ ] 本 plan 对应 `- [ ]` 改为 `- [x]`

---

## 9. 终端矩阵（v1 声明）

| 环境 | v1 | 备注 |
|------|----|------|
| Kitty / Ghostty | 必须 | 含 Kitty keyboard |
| iTerm2 / WezTerm | 必须 | |
| tmux（extended-keys on） | 必须 | OSC 可能需 passthrough |
| macOS Terminal.app | 尽力 | 无 truecolor；Enter 修饰键走 rescue |
| VS Code / Cursor 集成终端 | 尽力 | 部分 Ctrl 被宿主吃掉 |
| Windows Terminal | 尽力 | |
| 旧版 ConHost | 尽力 | glyph 回退已在 grok 验证，可 COPY |
| screen / 未知 | 不承诺 | |

每发现特例：先写进 `terminal-matrix.md` 再决定是否进代码。禁止为单一终端在 widget 里写 `if brand == ...`；特例进 `gbtui-terminal` / `gbtui-input`。

---

## 10. 风险与应对

| 风险 | 信号 | 应对 |
|------|------|------|
| `pager-render` 当框架整包 rename | Cargo.toml 出现 grok-config/tools/acp | 按 §6 白名单复制，CI 黑名单 |
| Hit-test 点穿 | example 里 dropdown 点到后面按钮 | P3 单测为门禁；Modal 必须 occlude |
| 空闲 30fps | 风扇响 / 测试见 Fast | `tick_demand` 单测；spinner 停必须降级 |
| 终端矩阵拖死进度 | 为 Apple Terminal 改 Button | 特例只进 input/terminal；v1 文档降级 |
| API 被 Grok 领域带偏 | Theme 出现 `accent_skill` | review 拒收 |
| example 长出第二运行时 | example 里 `match MouseEventKind` | 记缺口，补框架 |
| 法律/归属遗漏 | 无 NOTICE | P0 就放 LICENSE+NOTICE，搬 fork 时抄 NOTICE |

---

## 11. 建议的 PR 切分（可独立合并）

供日后按栈执行。每条可单独 review。状态用本计划勾选同步。

### PR 1: Workspace scaffold, license, dep firewall

- **Files/components affected:** 根 `Cargo.toml`, `rust-toolchain.toml`, `LICENSE`, `NOTICE`, `scripts/check-no-grok-deps.sh`, 空 crate 占位, `docs/source-map.md`, `README.md`
- **Dependencies:** None
- **Description:** 落地 P0。workspace 可编译，CI/脚本拒绝 `xai-grok-*`。

### PR 2: Lift textarea

- **Files/components affected:** `crates/gbtui-widgets/src/textarea/**`, tests, example
- **Dependencies:** PR 1
- **Description:** COPY `xai-ratatui-textarea`，去前缀，测试全绿。

### PR 3: Lift inline terminal

- **Files/components affected:** `crates/gbtui-terminal/src/inline/**`
- **Dependencies:** PR 1
- **Description:** COPY `xai-ratatui-inline`。可与 PR 2 并行。

### PR 4: Core primitives (key, theme, tick, safebuf)

- **Files/components affected:** `crates/gbtui-core/**`
- **Dependencies:** PR 1
- **Description:** P2a。Theme 白名单、quantize、KeyShortcut、TickDemand、Renderable、SafeBuf。

### PR 5: Terminal probe + keyboard capabilities

- **Files/components affected:** `crates/gbtui-terminal/src/{probe,keyboard,kitty,tmux,appearance,glyphs}/**`
- **Dependencies:** PR 3, PR 4
- **Description:** P2b STRIP。Glyph 无 voice。Image feature-gate 默认关；DA2 的保守启动门控由 P13 补充。

### PR 6: Input normalizers

- **Files/components affected:** `crates/gbtui-input/**`
- **Dependencies:** PR 5
- **Description:** P2c。KeyboardNormalizer、滚轮流、type-ahead 纯函数。单测救援与合帧。

### PR 7: Runtime FrameCtx, HitMap, event loop, Button

- **Files/components affected:** `crates/gbtui-runtime/**`, `crates/gbtui-widgets/src/button.rs`, `examples/settings`（最小按钮）
- **Dependencies:** PR 6
- **Description:** P3。同帧 hover、z-order、occlude、按需 tick。门禁单测必须过。这是整个计划的关键路径。

### PR 8: Modal + Overlay

- **Files/components affected:** `crates/gbtui-widgets/src/{modal,overlay}/**`
- **Dependencies:** PR 7
- **Description:** P4a/P4b。无全局 AtomicBool。Esc/外点/close hover。

### PR 9: ListPane + Picker + ChoiceList

- **Files/components affected:** `crates/gbtui-widgets/src/{list,picker,choice}/**`
- **Dependencies:** PR 8, PR 2（LineEditor/TextArea）
- **Description:** P4c/P4d/P4e。

### PR 10: Prompt chrome, progress, spinner, shortcuts, link

- **Files/components affected:** `crates/gbtui-widgets/src/{prompt,progress,spinner,shortcuts,link}/**`
- **Dependencies:** PR 7, PR 2
- **Description:** P5。Prompt 无 slash/@/image。

### PR 11: Action registry

- **Files/components affected:** `crates/gbtui-actions/**`
- **Dependencies:** PR 4, PR 10（hints 结构）
- **Description:** P6。泛型 Id/Ctx，example 自备枚举。

### PR 12: Facade + three examples

- **Files/components affected:** `crates/gbtui/**`, `examples/{settings,chat,picker}/**`
- **Dependencies:** PR 8, PR 9, PR 10, PR 3, PR 11
- **Description:** P7 手感验收。example 禁止第二运行时。

### PR 13: Docs closeout

- **Files/components affected:** `README.md`, `docs/**`, `CONTRIBUTING.md`, rustdoc
- **Dependencies:** PR 12
- **Description:** P8。source-map 收口，terminal-matrix 诚实。

### PR 14: Second-wave chrome

- **Files/components affected:** `crates/gbtui-widgets/src/{status,toast,tabs,chip,dropdown,cheatsheet,clipboard}/**`, `examples/settings`
- **Dependencies:** PR 7, PR 10, PR 11
- **Description:** P9。StatusBar / Toast / Tabs / Chip / Dropdown / Cheatsheet / SystemClipboard。无 slash/@、无欢迎流程。

### PR 15: Third-wave chrome

- **Files/components affected:** `gbtui-core` search、`crates/gbtui-widgets/src/{ghost,form,preview,banner,gauge,checklist,activity}/**`、`examples/{settings,chat}`
- **Dependencies:** PR 14
- **Description:** P10。GhostText / Form / Preview / Banner / UsageGauge / TextMatcher / Checklist / ActivityRow。无 slash/@、无 ACP、无 regex crate。

### PR 16: Optional agent chrome crate

- **Files/components affected:** `crates/gbtui-agent/**`、`examples/agent/**`、workspace `Cargo.toml`、`docs/{plan,architecture,source-map}.md`、`README.md`、`CONTRIBUTING.md`、`scripts/check-no-grok-deps.sh`
- **Dependencies:** PR 15
- **Description:** P11。`gbtui-agent` 叠在 facade 之上。Transcript / ToolRow / PermissionCard / SlashPalette / MentionChip。不改 Prompt、不 COPY scrollback/blocks、`gbtui` 不依赖本 crate。

并行提示：PR 2 ∥ PR 3 ∥ PR 4 在 PR 1 之后；PR 7 是关键路径，不能与未完成的输入层并行合并到 main（可在叠栈上开发，但行为依赖 PR 6）。

---

## 12. 工作方式（约束执行者）

1. **一次只推进一个阶段的出门门槛**，不要同时抽 10 个 widget。
2. 搬 grok 文件时在提交说明写：`SOURCE: crates/codegen/<path> (COPY|STRIP|REWRITE)`。
3. 发现 grok 里有价值但 §6 标 NEVER 的东西：加到本文件「附录：v2 候选」，**不要直接写进 v1 crate**。
4. API 改名可以，语义（同帧 hover、按需 tick、依赖单向、Theme 白名单）不可以偷偷改。
5. 完成一组勾选后，更新进度总表状态。

---

## P14 — 通用交互与阅读组件对齐

本阶段只对齐通用组件和 TUI 效果，不迁移产品协议或业务状态。

- [x] `ThemeKind` 解析后按终端颜色能力适配；ANSI16 保持基础 chrome 对比度，`NO_COLOR` 不重新引入颜色。显式 `Theme` 仍由宿主负责适配。
- [x] 失焦结束捕获与滚动流；丢失鼠标释放后不把普通移动继续当作拖拽；`Pointer::Down` 同步携带 modifiers，`Cancel` 不触发复制或点击。
- [x] Choice 分离导航与自由编辑，复用编辑器；支持说明/预览和焦点展开；嵌套编辑优先处理 Esc。
- [x] List 在现有物理行引擎上提供可选范围选择、稳定 ID 锚点和内容提取；过滤/删除锚点不会误选其他数据。
- [x] Prompt 历史采用现有匹配引擎，提供 smart-case 模糊排序、高亮和可视浏览；Dropdown 增加溢出指示、受控列宽及可配置可见行数/定位策略；普通字符不被导航抢键，缩窗锚点不越界。
- [x] `gbtui-widgets` 提供不依赖 Markdown 的 ContentViewer，消费宿主提供的原文与带源行映射的样式行；支持搜索、换行、范围选择、原文/渲染切换及明确复制范围。可选 `gbtui-agent` 层提供 Markdown/代码文档适配器，复用现有解析与高亮，不建立第二个渲染引擎。
- [x] Form 提供宿主错误提示、展开多选和固定操作区；按物理行保持选中字段/选项可见，校验与提交仍由宿主负责。
- [x] 示例覆盖新交互，统一检查与实际运行验收；原生 Kitty 的文本/样式/协议状态与 PTY 验收边界记录于 `terminal-matrix.md`，不等同于全终端或像素验收。

验收：workspace all-features 测试 **1227 passed / 9 ignored**；Clippy `-D warnings`、
全部 bins/examples 构建、facade 无默认 feature 检查和依赖边界脚本通过。
实际 Kitty 验收另修复 DSR 回包过滤吞 CSI-u、功能键 release 阻塞后续文本、
富选项说明漏绘和外层 Modal 抢 Esc；均保留行为回归并重跑实际交互。
Form 滚轮的物理偏移不再被焦点拉回；真实 PTY 验证滚动到 features、
按方向键恢复焦点跟随且 Apply/Reset 保持固定。

## P15 — 通用主题系统

- [x] 分离轻量 `Theme` 调色板、`ThemeDefinition`、`ThemeResolver` 与 `ResolvedTheme`；命名 registry 和稀疏覆盖在解析时处理，帧内语义样式查表不分配。
- [x] `Solid` / `TerminalDefault` / `Transparent` 三种背景策略；使用终端默认色而非模拟 alpha，低色适配后不重新填回背景。
- [x] 统一角色与 Normal/Hovered/Focused/Selected/Invalid/Disabled 状态、优先级和局部覆盖；`NO_COLOR` 保留互不混淆的状态反馈。
- [x] 可选 `theme-config` 严格 TOML 解析；宿主负责 IO，示例演示加载、成功重载和拒绝错误配置后保留当前主题。
- [x] `App::take_theme_request` / Pump 定义及上下文更新、有效变化版本号与缓存失效；迟到终端事实不覆盖用户热切换的定义。
- [x] 基础 widgets、TextArea、示例 chrome 迁移至统一语义样式；透明浮层清除旧文字。
- [x] Markdown、代码、diff、链接消费解析后的样式、最终明暗和色深；显式 Day/Night 不被终端明暗反向覆盖。
- [x] `themes` 示例提供全部语义角色/状态、实际 Button/Link/Prompt、外部 Aurora TOML、对比度估计和窄窗口状态切换。

验收：workspace all-features 测试 **1252 passed / 8 ignored**；Clippy `-D warnings`、
全部 bins/examples 构建、facade 无默认 feature 检查及依赖边界脚本通过。
实际 PTY/Kitty 验证热切换、Unicode 草稿保留、透明/实体背景、60×18 布局、
ANSI16/256/无色输出、外部主题重载、迟到 OSC11 与代码/Markdown 热切换。
实跑发现并修复无色状态反馈重合；显式明暗的语法高亮优先级、自定义 Reset 调色板在浅色 ANSI16 终端下保留默认背景也有行为回归。
具体终端覆盖边界见 `terminal-matrix.md`，不声称全终端或逐像素验收。

## 附录 A — 后续候选（未勾选项仍不在当前范围）

- [x] `gbtui-markdown` + `gbtui-markdown-core`（从原通用引擎 STRIP，保留 Markdown/代码/流式；Mermaid fence 按普通代码显示，不迁 diagram 渲染）
- [x] 可选 `images`：宿主提供字节，构造时解码/转 PNG，帧内按独立 id 上传/放置/清理；Kitty/Ghostty/WezTerm 自动启用，未知/Windows/mux 与 iTerm2 动态场景保留文本回退
- [ ] 把 Grok pager 迁到本框架（dogfood）
- [ ] Windows / VS Code / Apple Terminal 完整矩阵
- [x] Prompt addon：通用补全、历史、粘贴折叠与附件；宿主提供候选和资源，slash overlay 保持可独立使用
- [ ] crates.io 发布
- [ ] 更多主题（tokyonight / rosepine，仅白名单字段）
- [x] List 富行 `Renderable` 数据模型与展开式 Picker；复用已有物理行布局/缓存，不新增平行列表实现
- [x] 滚轮加速 / 有限滑行
- [x] OSC 8 真发出（`Terminal::set_frame_links` 接到 FrameCtx、Link 和富 Markdown）
- [x] OSC 11 启动探测

## 附录 B — 与上一轮分析的对应

| 分析结论 | 计划落点 |
|----------|----------|
| 不要整仓当框架 | §1.2 NEVER + D1 |
| 真正值钱的是运行时 | P2+P3 为关键路径，P4 被门禁挡住 |
| HitArea 两帧协议 | P3 改为同帧 hover，点击仍用上一帧 HitMap |
| 按需动画 Fast/Slow/None | §3.3 + P3 单测 |
| KeyboardNormalizer / 滚轮流 | P2c |
| 已干净的 textarea / inline | P1 COPY |
| pager-render 半成品 | §6 STRIP 白名单，禁止 rename |
| Action 注册表 | P6 泛型化 |
| 用 example 反哺不要迁 Grok | P7 + 禁区 |

---

## 附录 C — P3 Button 最小伪代码（实现对照）

```rust
pub struct Button<M> {
    pub id: HitId,
    pub label: String,
    pub focused: bool,
    pub on_click: Option<M>, // 或回调；v1 推荐 Msg 值以免生命周期痛苦
}

impl<M: Clone> Button<M> {
    pub fn render(&self, area: Rect, buf: &mut Buffer, cx: &mut FrameCtx<'_>) -> Option<M> {
        cx.register(self.id, area);
        let hover = cx.hovered(area);
        let style = if hover || self.focused {
            Style::default().bg(cx.theme.button_hover_bg).fg(cx.theme.button_fg)
        } else {
            Style::default().bg(cx.theme.button_bg).fg(cx.theme.button_fg)
        };
        buf.set_string_safe(area.x, area.y, &self.label, style);
        None // 点击由 runtime 根据 HitMap + 上一帧 id 投递 Msg
    }
}
```

点击投递在 `run()` 里做，不在 `render()` 里做副作用。这与 ratatui 即时绘制模型一致：render 无 IO。

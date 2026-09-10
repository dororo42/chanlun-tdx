# AGENTS.md

基于 YuYuKunKun/chanlun.py（MIT）缠论算法，移植为通达信（TDX）Rust 插件 DLL 的项目。当前处于交接中，详见下文 Current Handoff。

## 长期规则

- 语言使用 Rust（核心算法在 `src/chan.rs`，TDX FFI 在 `src/lib.rs`）。
- 算法口径以 `docs/semantics.md` 为准，任何改动前后都要重跑 `tools/gen_fixture.py` 与 `cargo test --test parity` 验证对齐。
- bearer 约定：不提交 `target/`、`tools/.venv/`、任何 `*.dll` 构建产物、任何凭证（`.env`、`ghp_*`、私钥）。
- commit message 用中文 + `M<编号>` 前缀（延续 M1/M2/M3 风格），一条逻辑单元一次提交。

## 工作约定

- 开始任何任务前，先跑 `cargo test --test parity` 确认基线（当前预期失败于「线段2 武bar」）。
- 修改前先读相关文件，不要凭猜测改代码；`src/chan.rs` 里每个函数注释都带 chan.py 原文行号，改前对照。
- 调试临时文件（如 `tests/dbg.rs`）不入库，交付前清理。
- 不主动 commit/push；push 到 GitHub 私人库需用户确认后再做，且 PAT 不落盘入库。

## Current Handoff

- Latest: `handoffs/2026-09-10-1055-chanlun-python-to-tdx-rust-port/HANDOFF.md`
- Branch: `master` @ `f26d36a`
- Status: 进行中（parity 差最后一个线段终点解析 bug；公式/README/推送未开始）

任何 agent 开始工作前，先完整读取上面的 HANDOFF.md。这份指针在每次交接后手工同步更新。
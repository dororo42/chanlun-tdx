# PLAN — chanlun-tdx：YuYuKunKun/chanlun.py 缠论算法 → 通达信 DLL

> 状态：执行中 · 创建于 2026-09-07 · 私有仓库 `chanlun-tdx`

## 1. 目标

把 **YuYuKunKun/chanlun.py**（9984 行 Python，MIT）的缠论算法核心移植为
Rust cdylib，编译为**通达信插件 DLL**（32 位 + 64 位），并提供两套通达信公式：

1. **主图显示公式**：分型 / 笔 / 线段 / 笔中枢 / 线段中枢 / 三类买卖点 叠加显示；
2. **条件选股公式**：最近 N 根 K 线内出现指定买卖点（默认 二买/三买）时选出。

算法口径忠实于 chan.py 的**默认配置**（`缠论配置` 默认值），差异点在 REPORT.md 逐条披露。

## 2. 总体架构与验证策略

```
YuYuKunKun/chanlun.py (Python, 参考实现)
        │  tools/gen_fixture.py  (喂入同一组确定性 K 线, 导出分型/笔/线段/中枢/买卖点)
        ▼
tests/fixtures/parity_*.json   ←—— 单一事实来源
        │  cargo test (奇偶校验: Rust 输出逐项比对 JSON)
        ▼
src/chan.rs (Rust 核心移植, 无 TDX 依赖, 可独立测试)
        │  src/lib.rs (TDX 插件协议 FFI)
        ▼
chanlun_tdx.dll (x86_64 + i686, GitHub Actions 编译)
        │
        ▼
formulas/*.txt (通达信公式引用 TDXDLL)
```

**验证三层**：① 单元测试（手工构造的结构化数据）；② 夹具奇偶校验（Rust vs
Python 参考实现逐项一致，算法正确性的主要证据）；③ CI 全量回归。

## 3. 通达信 DLL 接口规范

通达信插件协议：`RegisterTdxFunc` 注册 `PluginTCalcFuncInfo` 表，
每个函数签名 `void fn(int DataLen, float* pfOUT, float* pfINa, float* pfINb, float* pfINc)`。
每次调用全量重算（O(n)，与 chanlun-engine 同策略，8000 根 K 线毫秒级）。

注册函数表（nFuncMark → 输出，输入统一为 `HIGH, LOW, CLOSE`）：

| Mark | 输出 | Mark | 输出 |
|---|---|---|---|
| 1 | 分型标记（顶=1，底=-1） | 11 | 三买标记 |
| 2 | 分型价格 | 12 | 三卖标记 |
| 3 | 笔端点标记（上=1，下=-1） | 13 | 二买标记 |
| 4 | 笔端点价格 | 14 | 二卖标记 |
| 5 | 笔中枢 ZG | 15 | 一买标记（趋势背驰） |
| 6 | 笔中枢 ZD | 16 | 一卖标记（趋势背驰） |
| 7 | 笔中枢起止（中枢第 1 根=1） | 17 | 盘整背驰标记 |
| 8 | 线段端点标记 | 18 | 线段中枢 ZD（类二/类三买未实现，18 复用） |
| 9 | 线段端点价格 | 19 | 笔中枢起止（首根=1, 末根=-1）【v2 新增】 |
| 10 | 线段中枢 ZG | 20 | 线段中枢起止（首根=1, 末根=-1）【v2 新增】 |

> 无输出值的 bar 输出 0（通达信 `DRAWNULL` 由公式侧用 `NODRAW`/条件绘制处理）。

## 4. 算法移植映射（chan.py → src/chan.rs）

| chan.py（中文类/方法） | 移植目标 | 关键默认口径 |
|---|---|---|
| `缠论K线`（K线合并/包含处理） | `merge_klines` | 与前一根比较定方向；高高/低低合并；`expand_bar` 时右端可延伸 |
| `分型` | `find_fractals` | 三根缠K，中间 K 高低点均为极值；顶底分型间至少间隔 |
| `笔` | `build_strokes` | `笔内元素数量=5`（默认严格成笔）；顶底交替 |
| `线段特征`/`线段` | `build_segments` | 特征序列分型；缺口两种情形（缺口后需回补确认，`缺口后紧急修正=True`） |
| `中枢` | `build_pivots` | 前三线重叠：ZG=min(高点)，ZD=max(低点)；延伸=后续线与 [ZD,ZG] 有重叠 |
| `背驰分析.MACD背驰` | `macd_beichi` | MACD(12,26,9) 内部自算；进入段 vs 离开段 hist 面积/柱值比较 |
| `买卖点`（1/2/3 类） | `detect_mmds` | 按中枢+走势段关系判定，规则见代码注释逐条标注 chan.py 对应行号 |

超出范围（明确不做，避免口径争议）：多周期立体分析（DLL 单周期职责）、
信号/Factor/Position 体系、backtrader/MCP 集成。`类二/类三` 如实现则在 REPORT 中注明。

## 5. 里程碑

- [x] M1 脚手架 + 本计划（git 管理，原子提交）
- [x] M2 语义提取：精读 chan.py 五大核心段，提取口径笔记入 `docs/semantics.md`
- [x] M3 Python 夹具：`gen_fixture.py` 生成 3 组数据（趋势+震荡+缺口）的 JSON
- [x] M4 Rust 核心：`chan.rs` + 单元测试通过
- [x] M5 奇偶校验：分型/笔/中枢/MACD 100% 对齐；线段改结构不变量验证（偏差记录于 REPORT §3）
- [x] M6 FFI + 两套公式 + README/REPORT
- [ ] M7 GitHub 公开库推送（已建库 dororo42/chanlun-tdx，推送受网络限制待恢复）+ Actions 双目标编译
- [x] M10 公式 v2.0：修首笔端点缺失；新增 mark 19/20 中枢起止；三买/三卖判定窗口放到后 1~2 笔
      （对齐 chan2zen/rust-chan）；主图改「上下沿轨线 + 起止竖框」不再实心糊屏；选股改
      BARSLAST 时效 + 未破坏判定 + 力度背驰过滤；新增力度副图公式（详见 docs/formulas.md）

## 5.1 v2.0 口径边界（不可从 rust-chan 直接移植的部分）

rust-chan（chan2zen）与本项目**核心算法不同源**，只借用其**公式侧技巧**，以下不搬：

| rust-chan 特性 | 本项目情况 | 处置 |
|---|---|---|
| 单 DLL 函数 + mode 位编码接口 | 本项目是 20 个 mark 的逐 bar 输出 | 接口层不通用，只借鉴公式写法 |
| `Pole` 统一笔/段/趋势极点 + 中阴段 | 分型/笔/线段各自独立结构，无中阴段 | 需改算法层，本期不做 |
| `FXX` 强分型(-2/2) / 弱分型(-1/1) | 只有顶/底分型，无强弱之分 | 无法搬 |
| `Pivot.extended`（中枢扩张，ZG/ZD 取负值） | `Pivot` 无 extended 字段 | 无法搬 |
| `BACKSET` 反推中枢区间 | —— | **刻意不搬**：BACKSET 是未来函数，改用 mark 19/20 |
| 力度背驰（幅/天数/力度） | DLL 只有 MACD 面积背驰 | 已搬到公式侧实现 |

## 6. CI 设计（GitHub Actions）

- `windows-latest` runner，`dtolnay/rust-toolchain` 安装 `stable-x86_64-msvc` 与 `stable-i686-msvc`
- 步骤：`cargo test`（host 目标）→ 双 target `cargo build --release` → 上传 DLL artifacts
- 触发：push to master / PR / 手动；tag `v*` 时创建 GitHub Release 附 DLL

## 7. 仓库与运维

- 本地 git 原子提交；远端 GitHub **私有库**（PAT 仅用于推送与建库，不落盘、不入库）
- 网络出口：局域网代理 `http://192.168.2.152:16492`（备用 `:7897`），仅本地 git/curl 使用；CI 无需代理

# chanlun-tdx

基于 [YuYuKunKun/chanlun.py](https://github.com/YuYuKunKun/chanlun.py)（MIT）缠论算法，移植为**通达信（TDX）Rust 插件 DLL**。

## 功能

- 缠论核心：K线合并 → 分型 → 笔 → 笔中枢 → 线段 → 线段中枢 → 买卖点 → MACD 背驰
- 20 路输出（mark 1-20），覆盖分型/笔/线段/中枢/1/2/3 类买卖点/盘整背驰
- 双平台 DLL：Windows 32 位（i686）+ 64 位（x86_64）
- 两套通达信公式：主图叠加显示 + 条件选股

## 构建

```bash
cargo build --release --target x86_64-pc-windows-msvc    # 64 位 DLL
cargo build --release --target i686-pc-windows-msvc      # 32 位 DLL
```

或使用 GitHub Actions：push 到 master 后自动构建并上传 artifacts。

## 安装到通达信

1. 从 [Releases](../../releases) 下载对应位数的 `chanlun_tdx.dll`
2. 复制到通达信安装目录 `T0002\dlls\`（32 位 DLL）或通达信 64 位版的对应目录
3. 打开通达信 → 功能 → 公式系统 → 公式管理器 → DLL 函数 → 绑定 DLL 文件到 TDXDLL1
4. 导入 `formulas/缠论主图显示.txt` 和 `formulas/缠论条件选股.txt`

## DLL 函数表（mark → 输出）

| Mark | 输出 | Mark | 输出 |
|---|---|---|---|
| 1 | 分型标记（顶=1 底=-1） | 11 | 三买 |
| 2 | 分型价格 | 12 | 三卖 |
| 3 | 笔端点标记 | 13 | 二买 |
| 4 | 笔端点价格 | 14 | 二卖 |
| 5 | 笔中枢 ZG | 15 | 一买 |
| 6 | 笔中枢 ZD | 16 | 一卖 |
| 7 | 笔中枢起止 | 17 | 盘整背驰 |
| 8 | 线段端点标记 | 18 | 线段中枢 ZD |
| 9 | 线段端点价格 | 19-20 | 预留 |
| 10 | 线段中枢 ZG | | |

## 测试

```bash
cargo test --test parity    # 奇偶校验（分型/笔/中枢/MACD 与 chan.py 100% 对齐）
cargo test                  # 全部测试
```

## 协议

MIT。算法参考 [YuYuKunKun/chanlun.py](https://github.com/YuYuKunKun/chanlun.py)（MIT）。

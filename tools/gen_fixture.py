# -*- coding: utf-8 -*-
"""gen_fixture.py — 运行 YuYuKunKun/chanlun.py 参考实现，导出夹具 JSON。

用法:
    python tools/gen_fixture.py  (在仓库根目录; 依赖 tools/.venv)

输出 tests/fixtures/parity_<name>.json:
    bars      : [[ts, open, high, low, close], ...]  索引即 bar 序号
    macd      : [[dif, dea, hist], ...]              逐 bar
    chank     : 缠K 终点原始序号列表 + 高低 + 分型结构
    fractals  : [[bar_idx, 结构(1顶/-1底), 特征值], ...]
    bis       : [[文bar, 武bar, 方向(1上/-1下), 文值, 武值], ...]
    bi_zss    : [[zg, zd, 起bar, 止bar], ...]
    xds       : [[文bar, 武bar, 方向], ...]
    xd_zss    : [[zg, zd, 起bar, 止bar], ...]
"""
import json
import math
import random
import sys
from datetime import datetime, timedelta
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CHAN_DIR = ROOT.parent / "chanlun_yuyukun"
sys.path.insert(0, str(CHAN_DIR))

import chan as yuyu  # noqa: E402  (YuYuKunKun/chanlun.py)

PERIOD = 86400
START = datetime(2024, 1, 1)


def make_bars(seed: int, n: int, start_price: float) -> list:
    """确定性合成 K 线：短相位交替趋势+震荡+偶发跳空，保证笔/线段/中枢/缺口情形充分出现。"""
    rng = random.Random(seed)
    bars = []
    price = start_price
    ts = START
    i = 0
    drift = 1.0
    while i < n:
        plen = rng.randint(12, 28)
        vol = rng.choice([1.2, 2.0, 0.8])
        for _ in range(plen):
            delta = rng.gauss(drift, vol)
            op = price
            cl = op + delta
            hi = max(op, cl) + abs(rng.gauss(0, vol * 0.6))
            lo = min(op, cl) - abs(rng.gauss(0, vol * 0.6))
            if rng.random() < 0.015:  # 偶发跳空
                op *= 1 + rng.choice([0.02, -0.02])
                cl = op + delta
                hi = max(op, cl) + 0.5
                lo = min(op, cl) - 0.5
            bars.append((ts, op, hi, lo, cl))
            price = cl
            ts += timedelta(seconds=PERIOD)
            i += 1
            if i >= n:
                return bars
        drift = -drift * rng.choice([0.8, 1.0, 1.3])  # 方向交替
    return bars


def dump(observer, out: Path):
    o = observer
    bars = [[int(k.时间戳.timestamp()), k.开盘价, k.高, k.低, k.收盘价] for k in o.普通K线序列]
    macd = [
        [k.macd.DIF or 0.0, k.macd.DEA or 0.0, k.macd.MACD柱 or 0.0] for k in o.普通K线序列
    ]
    chank = [
        [k.原始结束序号, k.高, k.低, k.分型.name if k.分型 else None] for k in o.缠论K线序列
    ]
    # 分型：全部缠K上的 顶/底 标记（独立于分型序列，DLL 输出即此）
    fractals = [
        [k.标的K线.序号, 1 if k.分型 == yuyu.分型结构.顶 else -1,
         k.分型特征值]
        for k in o.缠论K线序列 if k.分型 in (yuyu.分型结构.顶, yuyu.分型结构.底)
    ]
    bis = [
        [b.文.中.标的K线.序号, b.武.中.标的K线.序号,
         1 if b.方向 == yuyu.相对方向.向上 else -1,
         b.文.分型特征值, b.武.分型特征值]
        for b in o.笔序列
    ]
    bi_zss = [
        [z.高, z.低, z.文.中.标的K线.序号, z.武.中.标的K线.序号]
        for z in o.笔_中枢序列
    ]
    xds = [
        [x.文.中.标的K线.序号, x.武.中.标的K线.序号,
         1 if x.方向 == yuyu.相对方向.向上 else -1]
        for x in o.线段序列组[0]
    ]
    xd_zss = [
        [z.高, z.低, z.文.中.标的K线.序号, z.武.中.标的K线.序号]
        for z in o.中枢序列组[0]
    ]
    data = {
        "bars": bars, "macd": macd, "chank": chank, "fractals": fractals,
        "bis": bis, "bi_zss": bi_zss, "xds": xds, "xd_zss": xd_zss,
    }
    out.write_text(json.dumps(data, ensure_ascii=False), encoding="utf-8")
    print(f"{out.name}: bars={len(bars)} chank={len(chank)} fx={len(fractals)} "
          f"bi={len(bis)} bi_zs={len(bi_zss)} xd={len(xds)} xd_zs={len(xd_zss)}")


def main():
    out_dir = ROOT / "tests" / "fixtures"
    out_dir.mkdir(parents=True, exist_ok=True)
    cases = [
        ("trend", 11, 600, 100.0),
        ("zigzag", 22, 600, 50.0),
        ("mixed", 33, 800, 3000.0),
    ]
    for name, seed, n, p0 in cases:
        obs = yuyu.观察者(f"fixture_{name}", PERIOD, yuyu.缠论配置())
        for ts, op, hi, lo, cl in make_bars(seed, n, p0):
            obs.投喂原始数据(ts, op, hi, lo, cl, 1000.0)
        dump(obs, out_dir / f"parity_{name}.json")


if __name__ == "__main__":
    main()

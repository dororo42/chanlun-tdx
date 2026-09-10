//! lib.rs — 通达信插件 DLL 入口（TDX PluginTCalcFuncInfo 协议）。
//!
//! 公式调用：`TDXDLL1(Mark, HIGH, LOW, CLOSE)`（Mark 见下表）。
//! 每次调用全量重算管线（毫秒级），并以 (长度+首末K线) 为键缓存 20 路输出。

pub mod chan;

use chan::*;
use std::sync::Mutex;

/// 输出 mark 常量（与 PLAN.md 注册表一致）
pub mod mark {
    pub const FX_MARK: usize = 1;
    pub const FX_PRICE: usize = 2;
    pub const BI_MARK: usize = 3;
    pub const BI_PRICE: usize = 4;
    pub const BI_ZS_ZG: usize = 5;
    pub const BI_ZS_ZD: usize = 6;
    pub const BI_ZS_START: usize = 7;
    pub const XD_MARK: usize = 8;
    pub const XD_PRICE: usize = 9;
    pub const XD_ZS_ZG: usize = 10;
    pub const BUY3: usize = 11;
    pub const SELL3: usize = 12;
    pub const BUY2: usize = 13;
    pub const SELL2: usize = 14;
    pub const BUY1: usize = 15;
    pub const SELL1: usize = 16;
    pub const PZ_BEICHI: usize = 17;
    pub const XD_ZS_ZD: usize = 18; // 类二/类三买未实现（输出0），18 复用为线段中枢 ZD
    pub const RESV19: usize = 19;
    pub const RESV20: usize = 20;
}

/// 全部输出的计算结果
struct Outputs {
    data: Vec<Vec<f32>>, // 下标 = mark-1
}

fn set(out: &mut [Vec<f32>], mark: usize, bar: usize, v: f32) {
    out[mark - 1][bar] = v;
}

fn compute_all(bars: &[Bar]) -> Outputs {
    let n = bars.len();
    let mut o = Outputs { data: vec![vec![0.0f32; n]; 20] };

    // 1) K线合并 + 分型标记
    let ks = merge_and_mark(bars);
    for k in &ks {
        match k.kind {
            Some(Kind::Top) => {
                set(&mut o.data, mark::FX_MARK, k.mark_bar, 1.0);
                set(&mut o.data, mark::FX_PRICE, k.mark_bar, k.feat as f32);
            }
            Some(Kind::Bottom) => {
                set(&mut o.data, mark::FX_MARK, k.mark_bar, -1.0);
                set(&mut o.data, mark::FX_PRICE, k.mark_bar, k.feat as f32);
            }
            _ => {}
        }
    }

    // 2) 笔
    let fxs = collect_fractals(&ks);
    let eps = build_strokes(&ks, &fxs);
    for w in eps.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (bar, top, feat) = (ks[b.k].mark_bar, b.top, b.feat);
        set(&mut o.data, mark::BI_MARK, bar, if top { 1.0 } else { -1.0 });
        set(&mut o.data, mark::BI_PRICE, bar, feat as f32);
        let _ = a;
    }
    let lines = strokes_to_lines(&ks, &eps);

    // 3) 笔中枢
    let pivots = build_pivots(&lines);
    for p in &pivots {
        let s = lines[p.start_li].wen;
        let e = lines[p.end_li].wu;
        for bar in s..=e {
            set(&mut o.data, mark::BI_ZS_ZG, bar, p.zg as f32);
            set(&mut o.data, mark::BI_ZS_ZD, bar, p.zd as f32);
        }
        set(&mut o.data, mark::BI_ZS_START, s, 1.0);
    }

    // 4) 线段 + 线段中枢
    let segs = build_segments(&lines);
    for s in &segs {
        set(&mut o.data, mark::XD_MARK, s.wen, if s.dir_up { -1.0 } else { 1.0 });
        set(&mut o.data, mark::XD_PRICE, s.wen, s.wen_feat as f32);
        set(&mut o.data, mark::XD_MARK, s.wu, if s.dir_up { 1.0 } else { -1.0 });
        set(&mut o.data, mark::XD_PRICE, s.wu, s.wu_feat as f32);
    }
    // 段中枢的线 = 每个线段本身（文→武），全部保留（chan.py 中枢.分析 输入=线段序列）
    let seg_lines: Vec<Line> = segs
        .iter()
        .map(|s| Line {
            wen: s.wen,
            wu: s.wu,
            dir_up: s.dir_up,
            high: s.wen_feat.max(s.wu_feat),
            low: s.wen_feat.min(s.wu_feat),
            wen_feat: s.wen_feat,
            wu_feat: s.wu_feat,
        })
        .collect();
    let xd_pivots = build_pivots(&seg_lines);
    for p in &xd_pivots {
        let s = seg_lines[p.start_li].wen;
        let e = seg_lines[p.end_li].wu;
        for bar in s..=e {
            set(&mut o.data, mark::XD_ZS_ZG, bar, p.zg as f32);
            set(&mut o.data, mark::XD_ZS_ZD, bar, p.zd as f32);
        }
    }

    // 5) MACD + 买卖点
    let closes: Vec<f64> = bars.iter().map(|b| b.close).collect();
    let m = macd(&closes, 13, 31, 11);
    let mmds = detect_mmds(&lines, &pivots, &m.hist);
    for mm in &mmds {
        let mk = match mm.kind {
            Mmd::Buy1 => mark::BUY1,
            Mmd::Sell1 => mark::SELL1,
            Mmd::Buy2 => mark::BUY2,
            Mmd::Sell2 => mark::SELL2,
            Mmd::Buy3 => mark::BUY3,
            Mmd::Sell3 => mark::SELL3,
            Mmd::PzBeichi => mark::PZ_BEICHI,
        };
        set(&mut o.data, mk, mm.bar, 1.0);
    }
    o
}

/// 缓存：以 (长度, 首K线高低收位型, 末K线高低收位型) 为键
struct Cache {
    key: (usize, u64, u64),
    outs: Outputs,
}

fn key_of(bars: &[Bar]) -> (usize, u64, u64) {
    fn h(b: &Bar) -> u64 {
        let mut v = b.high.to_bits();
        v ^= b.low.to_bits().rotate_left(17);
        v ^= b.close.to_bits().rotate_left(34);
        v
    }
    (bars.len(), bars.first().map(h).unwrap_or(0), bars.last().map(h).unwrap_or(0))
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

fn get_output_row(bars: &[Bar], mark: usize) -> Vec<f32> {
    let key = key_of(bars);
    // Mutex 中毒恢复：若前一次 panic 导致锁被毒化，清空缓存重建
    let mut guard = match CACHE.lock() {
        Ok(g) => g,
        Err(poisoned) => {
            *poisoned.into_inner() = None;
            CACHE.lock().unwrap_or_else(|p| p.into_inner())
        }
    };
    let need = !matches!(guard.as_ref(), Some(c) if c.key == key);
    if need {
        let outs = compute_all(bars);
        *guard = Some(Cache { key, outs });
    }
    match guard.as_ref() {
        Some(c) => c.outs.data.get(mark.wrapping_sub(1)).cloned().unwrap_or_default(),
        None => vec![0.0; bars.len()],
    }
}

/// TDX 计算函数签名：fn(DataLen, pfOUT, pfINa=HIGH, pfINb=LOW, pfINc=CLOSE)
type CalcFunc = unsafe extern "C" fn(i32, *mut f32, *const f32, *const f32, *const f32);

/// FFI 安全入口：catch_unwind 防止任何 Rust panic 穿透到 TDX 进程
unsafe fn run_calc(mark: usize, data_len: i32, pf_out: *mut f32, a: *const f32, b: *const f32, c: *const f32) {
    let n = data_len.max(0) as usize;
    if n == 0 || pf_out.is_null() || a.is_null() || b.is_null() || c.is_null() {
        return;
    }
    // 读入 K 线数据，过滤 NaN/Inf（停牌股、新股等常见）
    let bars: Vec<Bar> = (0..n)
        .map(|i| {
            let h = *a.add(i);
            let l = *b.add(i);
            let cl = *c.add(i);
            // NaN/Inf/非正值 → 用相邻有效值填充或置 0
            let h = if h.is_finite() && h > 0.0 { h } else { 0.0 };
            let l = if l.is_finite() && l > 0.0 { l } else { 0.0 };
            let cl = if cl.is_finite() && cl > 0.0 { cl } else { 0.0 };
            Bar { ts: i as i64, open: 0.0, high: h as f64, low: l as f64, close: cl as f64 }
        })
        .collect();

    // catch_unwind：任何 panic 都不穿透 FFI，输出全零
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        get_output_row(&bars, mark)
    }));

    match result {
        Ok(outs) => {
            for i in 0..n {
                *pf_out.add(i) = outs.get(i).copied().unwrap_or(0.0);
            }
        }
        Err(_) => {
            // panic 被捕获 → 输出全零，TDX 不崩
            for i in 0..n {
                *pf_out.add(i) = 0.0;
            }
        }
    }
}

macro_rules! calc_func {
    ($name:ident, $mark:expr) => {
        #[no_mangle]
        pub unsafe extern "C" fn $name(
            data_len: i32,
            pf_out: *mut f32,
            pf_ina: *const f32,
            pf_inb: *const f32,
            pf_inc: *const f32,
        ) {
            run_calc($mark, data_len, pf_out, pf_ina, pf_inb, pf_inc)
        }
    };
}

calc_func!(calc_fx_mark, 1);
calc_func!(calc_fx_price, 2);
calc_func!(calc_bi_mark, 3);
calc_func!(calc_bi_price, 4);
calc_func!(calc_bi_zs_zg, 5);
calc_func!(calc_bi_zs_zd, 6);
calc_func!(calc_bi_zs_start, 7);
calc_func!(calc_xd_mark, 8);
calc_func!(calc_xd_price, 9);
calc_func!(calc_xd_zs_zg, 10);
calc_func!(calc_buy3, 11);
calc_func!(calc_sell3, 12);
calc_func!(calc_buy2, 13);
calc_func!(calc_sell2, 14);
calc_func!(calc_buy1, 15);
calc_func!(calc_sell1, 16);
calc_func!(calc_pz_beichi, 17);
calc_func!(calc_xd_zs_zd, 18);
calc_func!(calc_resv19, 19);
calc_func!(calc_resv20, 20);

const FUNC_COUNT: usize = 20;

#[repr(C)]
pub struct PluginTCalcFuncInfo {
    pub n_func_mark: u16,
    pub p_call_func: Option<CalcFunc>,
}

static mut FUNC_TABLE: [PluginTCalcFuncInfo; FUNC_COUNT + 1] = [
    PluginTCalcFuncInfo { n_func_mark: 1, p_call_func: Some(calc_fx_mark) },
    PluginTCalcFuncInfo { n_func_mark: 2, p_call_func: Some(calc_fx_price) },
    PluginTCalcFuncInfo { n_func_mark: 3, p_call_func: Some(calc_bi_mark) },
    PluginTCalcFuncInfo { n_func_mark: 4, p_call_func: Some(calc_bi_price) },
    PluginTCalcFuncInfo { n_func_mark: 5, p_call_func: Some(calc_bi_zs_zg) },
    PluginTCalcFuncInfo { n_func_mark: 6, p_call_func: Some(calc_bi_zs_zd) },
    PluginTCalcFuncInfo { n_func_mark: 7, p_call_func: Some(calc_bi_zs_start) },
    PluginTCalcFuncInfo { n_func_mark: 8, p_call_func: Some(calc_xd_mark) },
    PluginTCalcFuncInfo { n_func_mark: 9, p_call_func: Some(calc_xd_price) },
    PluginTCalcFuncInfo { n_func_mark: 10, p_call_func: Some(calc_xd_zs_zg) },
    PluginTCalcFuncInfo { n_func_mark: 11, p_call_func: Some(calc_buy3) },
    PluginTCalcFuncInfo { n_func_mark: 12, p_call_func: Some(calc_sell3) },
    PluginTCalcFuncInfo { n_func_mark: 13, p_call_func: Some(calc_buy2) },
    PluginTCalcFuncInfo { n_func_mark: 14, p_call_func: Some(calc_sell2) },
    PluginTCalcFuncInfo { n_func_mark: 15, p_call_func: Some(calc_buy1) },
    PluginTCalcFuncInfo { n_func_mark: 16, p_call_func: Some(calc_sell1) },
    PluginTCalcFuncInfo { n_func_mark: 17, p_call_func: Some(calc_pz_beichi) },
    PluginTCalcFuncInfo { n_func_mark: 18, p_call_func: Some(calc_xd_zs_zd) },
    PluginTCalcFuncInfo { n_func_mark: 19, p_call_func: Some(calc_resv19) },
    PluginTCalcFuncInfo { n_func_mark: 20, p_call_func: Some(calc_resv20) },
    PluginTCalcFuncInfo { n_func_mark: 0, p_call_func: None },
];

/// 通达信插件注册入口
#[no_mangle]
pub unsafe extern "C" fn RegisterTdxFunc(p_fun: *mut *mut PluginTCalcFuncInfo) -> i32 {
    if (*p_fun).is_null() {
        *p_fun = std::ptr::addr_of_mut!(FUNC_TABLE[0]);
        1
    } else {
        0
    }
}

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

/// 通达信官方插件协议结构（两字段、packed(1) 无对齐填充）：
/// ```c
/// #pragma pack(1)
/// typedef struct tagPluginTCalcFuncInfo {
///     WORD        nFuncMark;    // 函数编号
///     pPluginFUNC pCallFunc;    // 函数指针
/// } PluginTCalcFuncInfo;
/// ```
/// 布局：32 位 = u16 + fn*(4) = 6 字节；64 位 = u16 + fn*(8) = 10 字节。
/// 【实测依据】chan2zen/rust-chan（Rust 缠论 TDX 插件，选股实测可跑）即用
/// `#[repr(C, packed(1))]` 两字段结构。
/// 【历史教训 v0.1.0-v0.1.2 崩溃根因】非 packed 布局在 mark 后有 2 字节
/// 对齐 padding，TDX 按 packed 步长在 offset+2 读函数指针 → 读到 padding →
/// 跳转非法地址 → 通达信直接退出；catch_unwind 无法拦截（崩在 TDX 进程内）。
#[repr(C, packed(1))]
pub struct PluginTCalcFuncInfo {
    pub n_func_mark: u16,
    pub p_call_func: Option<CalcFunc>,
}

/// 函数表（TDX 只读，末项 mark=0 为终止哨兵）
static FUNC_TABLE: [PluginTCalcFuncInfo; FUNC_COUNT + 1] = [
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

/// 通达信插件注册入口：把函数表首地址写回 TDX 传入的二级指针
#[no_mangle]
pub unsafe extern "C" fn RegisterTdxFunc(p_fun: *mut *const PluginTCalcFuncInfo) -> i32 {
    if p_fun.is_null() {
        return 0;
    }
    *p_fun = FUNC_TABLE.as_ptr();
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 结构布局必须与 TDX C 端一致（packed(1)，实测依据 rust-chan）：
    /// 32 位 = u16(2) + fn*(4) = 6 字节；64 位 = u16(2) + fn*(8) = 10 字节。
    /// 字段偏移：mark@0，pCallFunc@2（无对齐填充——padding 是 v0.1.x 崩溃根因）。
    #[test]
    fn tdx_struct_layout() {
        use std::mem::{align_of, offset_of, size_of};
        assert_eq!(size_of::<PluginTCalcFuncInfo>(), 2 + std::mem::size_of::<usize>(), "packed(1) 布局被破坏");
        assert_eq!(align_of::<PluginTCalcFuncInfo>(), 1);
        assert_eq!(offset_of!(PluginTCalcFuncInfo, n_func_mark), 0);
        assert_eq!(offset_of!(PluginTCalcFuncInfo, p_call_func), 2);
    }

    /// 函数表完整性：mark 1-20 连续、每个函数指针非空、末项哨兵。
    /// （packed 字段须先 copy 到局部再断言——取 packed 字段引用是 E0793）
    #[test]
    fn tdx_func_table_valid() {
        for (i, info) in FUNC_TABLE.iter().enumerate() {
            let m = info.n_func_mark;
            let f = info.p_call_func;
            if i < FUNC_COUNT {
                assert_eq!(m as usize, i + 1, "mark 连续");
                assert!(f.is_some(), "mark {} 缺函数指针", i + 1);
            } else {
                assert_eq!(m, 0);
                assert!(f.is_none());
            }
        }
    }

    /// RegisterTdxFunc 行为：回写表地址、返回 1；空指针参数返回 0。
    #[test]
    fn tdx_register_behavior() {
        unsafe {
            let mut p: *const PluginTCalcFuncInfo = std::ptr::null();
            assert_eq!(RegisterTdxFunc(&mut p), 1);
            assert_eq!(p, FUNC_TABLE.as_ptr());
            let null_pp: *mut *const PluginTCalcFuncInfo = std::ptr::null_mut();
            assert_eq!(RegisterTdxFunc(null_pp), 0);
        }
    }

    /// 健壮性：模拟 TDX 读表方式（按 C 布局偏移取函数指针）验证可安全取到。
    #[test]
    fn tdx_c_layout_indirect_access() {
        unsafe {
            let mut p: *const PluginTCalcFuncInfo = std::ptr::null();
            RegisterTdxFunc(&mut p);
            let mut i = 0usize;
            loop {
                let info = &*p.add(i);
                let (m, f) = (info.n_func_mark, info.p_call_func);
                if m == 0 {
                    break;
                }
                assert!(f.is_some());
                i += 1;
            }
            assert_eq!(i, FUNC_COUNT);
        }
    }

    /// 健壮性：极端输入下 compute_all 不 panic（新股极短、停牌全 0、
    /// 含 0 段、极值），且输出长度 == bar 数。
    #[test]
    fn compute_all_hostile_inputs() {
        fn check(bars: &[Bar]) {
            let outs = compute_all(bars);
            assert_eq!(outs.data.len(), 20);
            for (m, row) in outs.data.iter().enumerate() {
                assert_eq!(row.len(), bars.len(), "mark {} 输出长度错", m + 1);
                assert!(row.iter().all(|v| v.is_finite()), "mark {} 含非有限值", m + 1);
            }
        }
        // 空序列
        check(&[]);
        // 极短序列（1-3 根，新股）
        check(&[Bar { ts: 0, open: 1.0, high: 2.0, low: 0.5, close: 1.5 }]);
        check(&[
            Bar { ts: 0, open: 1.0, high: 2.0, low: 0.5, close: 1.5 },
            Bar { ts: 1, open: 1.5, high: 2.5, low: 1.0, close: 2.0 },
        ]);
        // 全 0（停牌/无数据）
        check(&[Bar { ts: 0, open: 0.0, high: 0.0, low: 0.0, close: 0.0 }; 300]);
        // 全同值（一字板）
        check(&[Bar { ts: 0, open: 5.0, high: 5.0, low: 5.0, close: 5.0 }; 300]);
        // 含 0 段 + 正常段混合
        let mut mixed = vec![Bar { ts: 0, open: 0.0, high: 0.0, low: 0.0, close: 0.0 }; 50];
        for i in 0..400 {
            let base = 10.0 + (i as f64 * 0.13).sin() * 3.0;
            mixed.push(Bar { ts: i as i64, open: base, high: base + 1.0, low: base - 1.0, close: base + 0.5 });
        }
        check(&mixed);
        // 单调数据（无分型/无笔形态）
        let mono: Vec<Bar> = (0..200).map(|i| Bar { ts: i, open: i as f64, high: i as f64 + 1.0, low: i as f64 - 1.0, close: i as f64 }).collect();
        check(&mono);
        // 极值
        check(&[
            Bar { ts: 0, open: 1e-8, high: 1e-8, low: 1e-8, close: 1e-8 },
            Bar { ts: 1, open: 1e10, high: 1e12, low: 1e-10, close: 1e11 },
            Bar { ts: 2, open: 5.0, high: 6.0, low: 4.0, close: 5.0 },
        ]);
    }

    /// 端到端：完全模拟 TDX 的调用方式 —— 经函数表取函数指针、传原生
    /// f32 缓冲区调用并写回输出。含 NaN 输入（停牌股）场景验证不崩且输出有限。
    #[test]
    fn tdx_end_to_end_via_func_pointer() {
        unsafe {
            let n = 300usize;
            let mut hi = vec![0f32; n];
            let mut lo = vec![0f32; n];
            let mut cl = vec![0f32; n];
            for i in 0..n {
                let base = 10.0 + (i as f32 * 0.2).sin() * 3.0;
                hi[i] = base + 1.0;
                lo[i] = base - 1.0;
                cl[i] = base;
            }
            let mut p: *const PluginTCalcFuncInfo = std::ptr::null();
            RegisterTdxFunc(&mut p);
            // 按表查找 mark（模拟 TDX 按 TDXDLL1(13,...) 定位函数）
            let mut i = 0usize;
            let f = loop {
                let info = &*p.add(i);
                let (m, fun) = (info.n_func_mark, info.p_call_func);
                if m == 13 {
                    break fun.unwrap();
                }
                i += 1;
            };
            // 正常数据
            let mut out = vec![f32::NAN; n];
            f(n as i32, out.as_mut_ptr(), hi.as_ptr(), lo.as_ptr(), cl.as_ptr());
            assert!(out.iter().all(|v| v.is_finite()), "正常数据输出含 NaN");

            // NaN 输入（停牌股/无数据品种）
            let bad = vec![f32::NAN; n];
            let mut out2 = vec![f32::NAN; n];
            f(n as i32, out2.as_mut_ptr(), bad.as_ptr(), bad.as_ptr(), bad.as_ptr());
            assert!(out2.iter().all(|v| v.is_finite()), "NaN 输入必须输出有限值");

            // DataLen=0 / 空指针（防御）
            f(0, out2.as_mut_ptr(), hi.as_ptr(), lo.as_ptr(), cl.as_ptr());
            f(n as i32, std::ptr::null_mut(), hi.as_ptr(), lo.as_ptr(), cl.as_ptr());
        }
    }
}

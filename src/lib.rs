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
    /// 笔中枢起止（+1 = 中枢首根，-1 = 中枢末根）
    ///
    /// 供主图公式画「上下沿轨线 + 起止竖框」的干净中枢矩形。
    /// 【为何不用 BACKSET】chan2zen/rust-chan 用 `BACKSET(BISE=2, ...)` 反推中枢区间，
    /// 但 BACKSET 是未来函数，历史 bar 会随新数据重绘；这里由 DLL 直接给出无未来函数的标记。
    pub const BI_ZS_EDGE: usize = 19;
    /// 线段中枢起止（+1 = 中枢首根，-1 = 中枢末根）
    pub const XD_ZS_EDGE: usize = 20;
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
    //
    // 【v2 修复】这里必须输出 windows(2) 的**两端点**。
    // `build_strokes` 返回的是完整笔端点序列，相邻两点即构成一笔
    // （tests/parity.rs 用同一 windows(2) 构造笔序列并与 chan.py 逐笔对齐可证），
    // 旧实现只写了武端并 `let _ = a;` 丢弃文端 → 首笔起点缺失、主图第一笔画不出来。
    let fxs = collect_fractals(&ks);
    let eps = build_strokes(&ks, &fxs);
    for w in eps.windows(2) {
        let (a, b) = (w[0], w[1]);
        for e in [a, b] {
            let bar = ks[e.k].mark_bar;
            set(&mut o.data, mark::BI_MARK, bar, if e.top { 1.0 } else { -1.0 });
            set(&mut o.data, mark::BI_PRICE, bar, e.feat as f32);
        }
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
        // 起止标记：s==e 的退化中枢只留 +1，避免 -1 覆盖后公式认不出起点
        set(&mut o.data, mark::BI_ZS_EDGE, s, 1.0);
        if e > s {
            set(&mut o.data, mark::BI_ZS_EDGE, e, -1.0);
        }
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
        set(&mut o.data, mark::XD_ZS_EDGE, s, 1.0);
        if e > s {
            set(&mut o.data, mark::XD_ZS_EDGE, e, -1.0);
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
calc_func!(calc_bi_zs_edge, 19);
calc_func!(calc_xd_zs_edge, 20);

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
    PluginTCalcFuncInfo { n_func_mark: 19, p_call_func: Some(calc_bi_zs_edge) },
    PluginTCalcFuncInfo { n_func_mark: 20, p_call_func: Some(calc_xd_zs_edge) },
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

    // ---------------- v2 公式侧改动对应的回归测试 ----------------

    /// 确定性合成行情：正弦漂移 + LCG 噪声随机游走（无外部依赖，CI 可复现）
    fn synth_bars(n: usize) -> Vec<Bar> {
        let mut s: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut rand01 = move || {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((s >> 33) as f64) / ((1u64 << 31) as f64)
        };
        let mut v = Vec::with_capacity(n);
        let mut price = 20.0f64;
        for i in 0..n {
            price += (i as f64 * 0.06).sin() * 0.35 + (rand01() - 0.5) * 0.8;
            let (o, c) = (price, price + (rand01() - 0.5) * 0.6);
            let hi = o.max(c) + rand01() * 0.5;
            let lo = o.min(c) - rand01() * 0.5;
            v.push(Bar { ts: i as i64, open: o, high: hi, low: lo, close: c });
            price = c;
        }
        v
    }

    fn line(wen: usize, wu: usize, dir_up: bool, wf: f64, uf: f64) -> Line {
        Line { wen, wu, dir_up, high: wf.max(uf), low: wf.min(uf), wen_feat: wf, wu_feat: uf }
    }

    /// 【回归】笔端点必须全部输出，含首笔文端（旧实现 `let _ = a;` 丢弃了它）
    #[test]
    fn bi_marks_include_first_endpoint() {
        let bars = synth_bars(600);
        let ks = merge_and_mark(&bars);
        let fxs = collect_fractals(&ks);
        let eps = build_strokes(&ks, &fxs);
        assert!(eps.len() >= 6, "合成数据未产出足够笔端点: {}", eps.len());

        let outs = compute_all(&bars);
        let row = &outs.data[mark::BI_MARK - 1];
        let marked = row.iter().filter(|v| **v != 0.0).count();
        assert_eq!(marked, eps.len(), "笔端点未全部输出（首笔文端缺失）");

        let first_bar = ks[eps[0].k].mark_bar;
        assert_eq!(row[first_bar], if eps[0].top { 1.0 } else { -1.0 }, "首笔文端未标记");
        assert_eq!(outs.data[mark::BI_PRICE - 1][first_bar], eps[0].feat as f32, "首笔文端价格缺失");
    }

    /// 【回归】笔中枢/线段中枢的起止标记（mark 19/20）必须与中枢区间一致
    #[test]
    fn pivot_edge_marks_match_pivots() {
        let bars = synth_bars(700);
        let outs = compute_all(&bars);
        let ks = merge_and_mark(&bars);
        let fxs = collect_fractals(&ks);
        let eps = build_strokes(&ks, &fxs);
        let lines = strokes_to_lines(&ks, &eps);
        let pivots = build_pivots(&lines);

        let edge = &outs.data[mark::BI_ZS_EDGE - 1];
        let starts = edge.iter().filter(|v| **v == 1.0).count();
        assert_eq!(starts, pivots.len(), "笔中枢首根标记数 != 中枢数");
        for p in &pivots {
            let (s, e) = (lines[p.start_li].wen, lines[p.end_li].wu);
            assert_eq!(edge[s], 1.0, "中枢首根未标记: {s}");
            if e > s {
                // 相邻中枢首尾同 bar 时 +1 会覆盖 -1（绘制无影响），此处只要求非 0
                assert_ne!(edge[e], 0.0, "中枢末根未标记: {e}");
            }
        }

        // 线段中枢（标记数必须与中枢数一致；不假设样本一定产出中枢）
        let segs = build_segments(&lines);
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
        let xd_edge = &outs.data[mark::XD_ZS_EDGE - 1];
        let xd_starts = xd_edge.iter().filter(|v| **v == 1.0).count();
        assert_eq!(xd_starts, xd_pivots.len(), "线段中枢首根标记数 != 中枢数");
    }

    /// 【回归】三买判定窗口：中枢结束时第 1 笔是「离开笔」、第 2 笔才是「回抽笔」，
    /// 旧实现只查第 1 笔 → 永不触发；现应对齐 rust-chan 检查后 1~2 笔。
    #[test]
    fn buy3_detects_pullback_after_leave_stroke() {
        let hist = vec![0.0f64; 40];
        let pivot = Pivot { zg: 10.55, zd: 10.05, start_li: 0, end_li: 2, tbs: None };
        let base = vec![
            line(0, 5, true, 10.00, 10.60),
            line(5, 9, false, 10.60, 10.05),
            line(9, 13, true, 10.05, 10.55),
        ];

        // 正例：直查第 2 笔（离开笔 13→17 突破 ZG，回抽笔 17→21 最低 11.00 > ZG）
        let mut ls = base.clone();
        ls.push(line(13, 17, true, 10.50, 11.50));
        ls.push(line(17, 21, false, 11.50, 11.00));
        let mmds = detect_mmds(&ls, std::slice::from_ref(&pivot), &hist);
        assert!(
            mmds.iter().any(|m| m.bar == 21 && m.kind == Mmd::Buy3),
            "三买未在第 2 笔回抽处触发: {:?}",
            mmds
        );

        // 反例：第 1 笔为同向回抽但最低点落回中枢内 → 不应触发
        let mut ls2 = base.clone();
        ls2.push(line(13, 17, false, 10.50, 10.30));
        let mmds2 = detect_mmds(&ls2, std::slice::from_ref(&pivot), &hist);
        assert!(mmds2.is_empty(), "回抽进中枢不应出三买: {:?}", mmds2);

        // 反例：第 1 笔既未离开中枢也未回抽 → 直接终止，不再看第 2 笔
        let mut ls3 = base.clone();
        ls3.push(line(13, 17, true, 10.05, 10.50));
        ls3.push(line(17, 21, false, 10.50, 10.20));
        let mmds3 = detect_mmds(&ls3, std::slice::from_ref(&pivot), &hist);
        assert!(mmds3.is_empty(), "未离开中枢不应出三买: {:?}", mmds3);
    }

    /// 【回归】三卖为三买的镜像（反抽笔最高点 < ZD）
    #[test]
    fn sell3_detects_rebound_after_leave_stroke() {
        let hist = vec![0.0f64; 40];
        let pivot = Pivot { zg: 10.55, zd: 10.05, start_li: 0, end_li: 2, tbs: None };
        let ls = vec![
            line(0, 5, true, 10.00, 10.60),
            line(5, 9, false, 10.60, 10.05),
            line(9, 13, true, 10.05, 10.55),
            line(13, 17, false, 10.50, 9.00), // 向下离开 ZD
            line(17, 21, true, 9.00, 9.60),   // 反抽最高 9.60 < ZD 10.05
        ];
        let mmds = detect_mmds(&ls, std::slice::from_ref(&pivot), &hist);
        assert!(
            mmds.iter().any(|m| m.bar == 21 && m.kind == Mmd::Sell3),
            "三卖未在第 2 笔反抽处触发: {:?}",
            mmds
        );
    }
}
